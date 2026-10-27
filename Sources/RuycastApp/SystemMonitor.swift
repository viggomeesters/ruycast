import AppKit
import SwiftUI
import Charts

@MainActor final class SystemMonitorModel:ObservableObject {
    @Published var snapshot:MonitorSnapshot?
    @Published var history:[MonitorSnapshot]=[]
    private(set) var active=false
    private var timer:Timer?
    private var inFlight=false
    private var generation=0
    private let worker=DispatchQueue(label:"ruycast.monitor",qos:.utility)
    private let sampler=ResourceSampler()
    func start() {
        guard !active else {return};active=true;generation+=1;refresh()
        timer=mainRunLoopTimer(every:2) {[weak self] in self?.refresh()}
    }
    func stop() {active=false;generation+=1;timer?.invalidate();timer=nil}
    func refresh() {
        guard active,!inFlight else {return};inFlight=true;let version=generation
        let sampler=sampler
        worker.async { [weak self] in
            let result=sampler.sample()
            DispatchQueue.main.async { [weak self] in
                guard let self else {return}
                self.inFlight=false
                guard self.active,self.generation==version else {return}
                self.snapshot=result;self.history=Array((self.history+[result]).suffix(60))
            }
        }
    }
    isolated deinit {timer?.invalidate()}
}
enum MonitorTab:String,CaseIterable {
    case overview="System Info",cpu="CPU",memory="Memory",disk="Macintosh HD",power="Power",network="Network"
    var icon:String {switch self {case .overview:return "info.circle";case .cpu:return "cpu";case .memory:return "memorychip";case .disk:return "internaldrive";case .power:return "battery.100";case .network:return "network"}}
}
struct SystemMonitorView:View {
    @ObservedObject var model:SystemMonitorModel
    @State var tab:MonitorTab = .overview
    @Environment(\.colorScheme) private var scheme
    private var theme:SettingsTheme {SettingsTheme(dark:scheme == .dark)}
    private func percent(_ value:Double?)->String {value.map {String(format:"%.1f%%",$0)} ?? "—"}
    var body:some View {
        VStack(spacing:0) {
            HStack(spacing:10) {Image(systemName:"waveform.path.ecg").foregroundStyle(theme.accent);Text("System Monitor").font(.system(size:17,weight:.bold,design:.rounded));Spacer();if let snapshot=model.snapshot {Text(snapshot.sampledAt.formatted(date:.omitted,time:.standard)).font(.caption).foregroundStyle(theme.muted)}}.padding(20)
            Divider()
            HStack(spacing:0) {
                VStack(spacing:5) {
                    ForEach(MonitorTab.allCases,id:\.self) {item in
                        Button {tab=item} label:{HStack(spacing:10) {Image(systemName:item.icon).frame(width:21);Text(item.rawValue);Spacer();badge(item).font(.caption.monospacedDigit()).foregroundStyle(theme.muted)}.padding(11).background(tab==item ? theme.ink.opacity(0.08):.clear,in:RoundedRectangle(cornerRadius:8)).contentShape(Rectangle())}.buttonStyle(.plain).accessibilityAddTraits(tab==item ? .isSelected:[])
                    }
                    Spacer()
                }.padding(12).frame(width:225)
                Divider()
                ScrollView {
                    VStack(alignment:.leading,spacing:22) {
                        Text(tab.rawValue).font(.system(size:23,weight:.bold,design:.rounded))
                        if let s=model.snapshot {details(s)} else {ProgressView().controlSize(.small)}
                    }.padding(24).frame(maxWidth:.infinity,alignment:.leading)
                }
            }
            Divider()
            HStack {Text("Ruycast").font(.caption).foregroundStyle(theme.muted);Spacer();Button("Open Activity Monitor") {NSWorkspace.shared.open(URL(fileURLWithPath:"/System/Applications/Utilities/Activity Monitor.app"))}.controlSize(.small)}.padding(14)
        }.foregroundStyle(theme.ink).background(theme.background).tint(theme.accent).frame(minWidth:740,minHeight:500)
    }
    @ViewBuilder private func badge(_ item:MonitorTab)->some View {
        if let s=model.snapshot {
            switch item {
            case .cpu:Text(percent(s.cpuPercent))
            case .memory:Text(s.memoryTotal>0 ? "\(Int(Double(s.memoryUsed)*100/Double(s.memoryTotal)))%":"—")
            case .disk:Text(s.diskTotal>0 ? "\(Int(Double(s.diskTotal-s.diskAvailable)*100/Double(s.diskTotal)))%":"—")
            case .power:Text(s.batteryPercent.map {"\($0)%"} ?? "—")
            default:EmptyView()
            }
        }
    }
    @ViewBuilder private func details(_ s:MonitorSnapshot)->some View {
        switch tab {
        case .overview:
            rows([("Uptime",uptime(s.uptime)),("Processes",String(s.processCount)),("macOS",s.os),("Hostname",s.hostname),("Model",s.model),("Chip",s.chip),("CPU Cores",String(s.cpuCores)),("Memory",memoryLabel(s.memoryTotal))])
        case .cpu:
            metric(percent(s.cpuPercent),"CPU Usage")
            graph { $0.cpuPercent }
            rows([("Chip",s.chip),("Cores",String(s.cpuCores)),("Idle",s.cpuPercent.map {percent(100-$0)} ?? "Collecting…")])
        case .memory:
            metric(memoryLabel(s.memoryUsed),"Memory Used")
            graph {Double($0.memoryUsed)/1_073_741_824}
            rows([("Physical Memory",memoryLabel(s.memoryTotal)),("Available",memoryLabel(s.memoryTotal-s.memoryUsed)),("Wired",memoryLabel(s.memoryWired)),("Compressed",memoryLabel(s.memoryCompressed)),("Swap Used",memoryLabel(s.swapUsed))])
        case .disk:
            metric(memoryLabel(s.diskAvailable),"Available")
            ProgressView(value:Double(s.diskTotal-s.diskAvailable),total:Double(max(1,s.diskTotal)))
            rows([("Capacity",memoryLabel(s.diskTotal)),("Used",memoryLabel(s.diskTotal-s.diskAvailable)),("Available",memoryLabel(s.diskAvailable))])
        case .power:
            metric(s.batteryPercent.map {"\($0)%"} ?? "No Battery","Battery")
            rows([("Power Source",s.powerSource ?? "Unavailable"),("Charging",s.charging.map {$0 ? "Yes":"No"} ?? "—")])
        case .network:
            metric(s.downloadBytesPerSecond.map {memoryLabel(UInt64(max(0,$0)))+"/s"} ?? "—","Download")
            graph {$0.downloadBytesPerSecond.map {$0/1024}}
            rows([("Upload",s.uploadBytesPerSecond.map {memoryLabel(UInt64(max(0,$0)))+"/s"} ?? "—"),("Received",memoryLabel(s.networkReceived)),("Sent",memoryLabel(s.networkSent)),("Interfaces",s.interfaces.joined(separator:", "))])
        }
    }
    private func metric(_ value:String,_ label:String)->some View {VStack(alignment:.leading,spacing:5) {Text(value).font(.system(size:32,weight:.semibold,design:.rounded)).monospacedDigit();Text(label).font(.caption).foregroundStyle(theme.muted)}}
    private func graph(_ value:@escaping(MonitorSnapshot)->Double?)->some View {
        Chart(Array(model.history.enumerated()),id:\.offset) {_,sample in if let y=value(sample) {LineMark(x:.value("Time",sample.sampledAt),y:.value("Usage",y)).foregroundStyle(theme.accent)}}.chartXAxis(.hidden).frame(height:150).accessibilityLabel("Recent \(tab.rawValue) usage")
    }
    private func rows(_ values:[(String,String)])->some View {
        VStack(spacing:0) {ForEach(values.indices,id:\.self) {i in HStack(alignment:.top,spacing:20) {Text(values[i].0).foregroundStyle(theme.muted);Spacer();Text(values[i].1).multilineTextAlignment(.trailing).textSelection(.enabled)}.font(.system(size:13)).padding(12).background(i%2==0 ? theme.surface:.clear)}}.clipShape(RoundedRectangle(cornerRadius:8))
    }
    private func uptime(_ seconds:TimeInterval)->String {let hours=Int(seconds)/3600;return hours>=24 ? "\(hours/24) days, \(hours%24) hours":"\(hours) hours"}
}
@MainActor final class SystemMonitorController:NSObject,NSWindowDelegate {
    let model=SystemMonitorModel()
    let window:NSWindow
    init(state:AppState) {
        window=NSWindow(contentRect:NSRect(x:0,y:0,width:900,height:640),styleMask:[.titled,.closable,.miniaturizable,.resizable],backing:.buffered,defer:false)
        super.init();window.title="System Monitor";window.minSize=NSSize(width:740,height:500);window.isReleasedWhenClosed=false;window.delegate=self
        window.contentView=NSHostingView(rootView:SystemMonitorView(model:model))
        state.showSystemMonitor={ [weak self] in self?.show() }
    }
    func show() {if !window.isVisible {window.center()};window.deminiaturize(nil);window.makeKeyAndOrderFront(nil);NSApp.activate(ignoringOtherApps:true);model.start()}
    func windowWillClose(_ notification:Notification) {model.stop()}
    func windowDidMiniaturize(_ notification:Notification) {model.stop()}
    func windowDidDeminiaturize(_ notification:Notification) {model.start()}
}
