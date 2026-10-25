import AppKit
import SwiftUI
import Charts
import LauncherCore

struct AnalyticsDashboard:View {
    @ObservedObject var store:AnalyticsStore
    let commands:[Command]
    let theme:SettingsTheme
    @State private var confirmingReset=false
    @State private var exportMessage=""
    private var d:AnalyticsDocument {store.snapshot}
    private var names:[String:String] {Dictionary(commands.map {(AnalyticsStore.commandKey($0.id),$0.title)},uniquingKeysWith:{$1})}
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:22) {
                HStack {Text("Dashboard").font(.system(size:22,weight:.bold,design:.rounded));Spacer();Button {store.flush()} label:{Label("Refresh",systemImage:"arrow.clockwise")}.controlSize(.small)}
                HStack(spacing:12) {
                    stat("Uses",String(d.totalAttempts+d.priorSuccesses))
                    stat("Successful",String(d.totalSuccesses+d.priorSuccesses))
                    stat("Failed",String(d.totalFailures))
                }
                TimelineView(.periodic(from:.now,by:60)) {context in
                    hourlyChart(now:context.date)
                }
                if !d.days.isEmpty {
                    VStack(alignment:.leading,spacing:12) {
                        Text("Recorded daily activity").font(.headline)
                        Chart(dayKeys,id:\.self) {day in if let recorded=d.days[day] {BarMark(x:.value("Day",day),y:.value("Uses",recorded.uses)).foregroundStyle(theme.accent)}}.chartXScale(domain:dayKeys).frame(height:110)
                    }.padding(16).background(theme.surface,in:RoundedRectangle(cornerRadius:9))
                }
                VStack(alignment:.leading,spacing:10) {
                    Text("Commands").font(.headline)
                    HStack {Text("Name");Spacer();Text("Uses").frame(width:65,alignment:.trailing);Text("Failed").frame(width:65,alignment:.trailing);Text("p95").frame(width:90,alignment:.trailing)}.font(.caption).foregroundStyle(theme.muted)
                    let rows=d.commands.sorted {$0.value.uses==$1.value.uses ? $0.key<$1.key:$0.value.uses>$1.value.uses}
                    if rows.isEmpty {Text("Run a command to see its activity.").font(.caption).foregroundStyle(theme.muted)}
                    ForEach(Array(rows.prefix(30)),id:\.key) {id,row in
                        HStack {Text(names[id] ?? "Removed command · \(id.prefix(6))").lineLimit(1);Spacer();Text(String(row.uses)).frame(width:65,alignment:.trailing);Text(String(row.failures)).frame(width:65,alignment:.trailing);Text(ms(row.timing.p95MS)).frame(width:90,alignment:.trailing)}.font(.system(size:12)).monospacedDigit().padding(.vertical,4)
                    }
                    if d.omittedCommands>0 {Text("Additional commands: \(d.omittedCommands) uses").font(.caption).foregroundStyle(theme.muted)}
                }
                VStack(alignment:.leading,spacing:10) {
                    Text("Recent timings").font(.headline)
                    if d.timings.isEmpty {Text("No timings recorded").font(.caption).foregroundStyle(theme.muted)}
                    ForEach(d.timings.keys.sorted(),id:\.self) {key in
                        HStack {Text(title(key));Spacer();Text(ms(d.timings[key]?.p95MS)).monospacedDigit();Text("p95").foregroundStyle(theme.muted)}.font(.system(size:12)).padding(.vertical,3)
                    }
                    Text("p95: latest 32 samples per command, 256 per measurement.").font(.caption).foregroundStyle(theme.muted)
                }.padding(16).background(theme.surface,in:RoundedRectangle(cornerRadius:9))
                VStack(alignment:.leading,spacing:10) {
                    Text("Recent errors").font(.headline)
                    if d.events.isEmpty {Text("No errors recorded").font(.caption).foregroundStyle(theme.muted)}
                    ForEach(Array(d.events.reversed().prefix(20))) {event in
                        HStack(alignment:.top) {
                            Image(systemName:"exclamationmark.circle").foregroundStyle(theme.accent)
                            VStack(alignment:.leading,spacing:3) {Text(event.commandID.flatMap {names[$0]} ?? event.category.capitalized);Text(event.exitStatus.map {"Script exited with status \($0)"} ?? reason(event.code)).font(.caption).foregroundStyle(theme.muted)}
                            Spacer();Text(event.date.formatted(date:.abbreviated,time:.shortened)).font(.caption).foregroundStyle(theme.muted)
                        }.font(.system(size:12)).padding(.vertical,3)
                    }
                }
                VStack(spacing:16) {
                    Toggle("Local analytics",isOn:Binding(get:{d.enabled},set:{value in try? store.configure(enabled:value,retentionDays:nil);store.flush()})).toggleStyle(.switch)
                    HStack {Text("Recent activity and errors");Spacer();Picker("Retention",selection:Binding(get:{d.retentionDays},set:{value in try? store.configure(enabled:nil,retentionDays:value);store.flush()})) {if ![7,30,90].contains(d.retentionDays) {Text("\(d.retentionDays) days").tag(d.retentionDays)};Text("7 days").tag(7);Text("30 days").tag(30);Text("90 days").tag(90)}.labelsHidden().frame(width:120)}
                    HStack {Button("Export Diagnostics…",action:export);Spacer();Button("Reset Analytics…",role:.destructive) {confirmingReset=true}}.controlSize(.small)
                    if d.priorSuccesses>0 {Text("Includes \(d.priorSuccesses) earlier successful uses. Earlier failures and timings are unknown.").font(.caption).foregroundStyle(theme.muted).frame(maxWidth:.infinity,alignment:.leading)}
                    Text("Stored on this Mac. No input or clipboard contents are logged.").font(.caption).foregroundStyle(theme.muted).frame(maxWidth:.infinity,alignment:.leading)
                    if let error=store.storageError {Text(error).font(.caption).foregroundStyle(theme.accent)}
                    if !exportMessage.isEmpty {Text(exportMessage).font(.caption).foregroundStyle(theme.muted)}
                }.font(.system(size:12)).padding(16).background(theme.surface,in:RoundedRectangle(cornerRadius:9))
            }.padding(26)
        }.foregroundStyle(theme.ink).background(theme.background).tint(theme.accent).confirmationDialog("Reset Analytics?",isPresented:$confirmingReset) {
            Button("Reset Analytics",role:.destructive) {store.reset();store.flush()}
            Button("Cancel",role:.cancel) {}
        } message:{Text("Deletes analytics only. Commands and clipboard history are kept.")}
    }
    private func hourlyChart(now:Date)->some View {
        let document=d,points=document.hourlyActivity(now:now)
        let start=points.first!.date,end=points.last!.date.addingTimeInterval(3600)
        return VStack(alignment:.leading,spacing:12) {
            HStack {
                Text("Last 24 hours").font(.headline)
                Spacer()
                Text("\(points.compactMap { $0.activity?.uses }.reduce(0,+)) uses").font(.caption).foregroundStyle(theme.muted).monospacedDigit()
            }
            Chart(points) {point in
                if let activity=point.activity {
                    BarMark(x:.value("Hour",point.date),y:.value("Uses",activity.uses),width:.ratio(0.72))
                        .foregroundStyle(theme.accent)
                        .accessibilityLabel(Text(hourDescription(point.date)))
                        .accessibilityValue(Text("\(activity.uses) uses"))
                }
            }
            .chartXScale(domain:start...end)
            .chartXAxis {AxisMarks(values:.stride(by:.hour,count:4)) {value in
                AxisGridLine();AxisTick();AxisValueLabel(format:.dateTime.hour().minute())
            }}
            .chartYAxis {AxisMarks(position:.leading,values:.automatic(desiredCount:4))}
            .frame(height:150)
            HStack(alignment:.top) {
                Text("Per hour · current hour included").font(.caption).foregroundStyle(theme.muted)
                Spacer()
                if let started=document.hourlyStartedAt,started>start {
                    Text("Recording since \(started.formatted(date:.abbreviated,time:.shortened))").font(.caption).foregroundStyle(theme.muted)
                }
            }
        }.padding(16).background(theme.surface,in:RoundedRectangle(cornerRadius:9))
    }
    private func hourDescription(_ date:Date)->String {
        let formatter=DateFormatter();formatter.locale=Locale.current;formatter.dateFormat="MMM d, HH:mm z"
        return formatter.string(from:date)
    }
    private var dayKeys:[String] {
        let calendar=Calendar.current,today=calendar.startOfDay(for:Date()),start=calendar.startOfDay(for:d.startedAt)
        let formatter=DateFormatter();formatter.calendar=calendar;formatter.locale=Locale(identifier:"en_US_POSIX");formatter.dateFormat="yyyy-MM-dd"
        return (0..<d.retentionDays).reversed().compactMap {offset in guard let date=calendar.date(byAdding:.day,value:-offset,to:today),date>=start else {return nil};return formatter.string(from:date)}
    }
    private func stat(_ label:String,_ value:String)->some View {VStack(alignment:.leading,spacing:6) {Text(value).font(.system(size:27,weight:.semibold,design:.rounded)).monospacedDigit();Text(label).font(.caption).foregroundStyle(theme.muted)}.frame(maxWidth:.infinity,alignment:.leading).padding(16).background(theme.surface,in:RoundedRectangle(cornerRadius:9))}
    private func ms(_ value:Double?)->String {value.map {String(format:$0<1 ? "%.2f ms":"%.1f ms",$0)} ?? "—"}
    private func title(_ key:String)->String {["search.model":"Search","search.clipboard":"Clipboard search","command.execute":"Commands","cli.dispatch":"CLI dispatch","launcher.open.model":"Open launcher","clipboard.preview":"Image preview","clipboard.capture":"Clipboard capture","clipboard.save":"Clipboard storage","process.query":"Processes","config.read":"Configuration read","config.reload":"Reload configuration","monitor.sample":"System Monitor","process.sort":"Process sorting","icons.lookup":"App icons","startup.model":"Startup" ][key] ?? key}
    private func reason(_ code:String)->String {switch code {case "permission":return "Permission required";case "timeout":return "Timed out";case "unavailable":return "Item or app unavailable";case "invalid","config-invalid":return "Invalid configuration or command";case "clipboard-io":return "Clipboard storage failed";case "interrupted":return "Operation interrupted";default:return "Action failed"}}
    private func export() {
        let panel=NSSavePanel();panel.nameFieldStringValue="ruycast-diagnostics.json";panel.allowedContentTypes=[.json]
        panel.begin {response in guard response == .OK,let url=panel.url else {return};do {let bytes=try JSONSerialization.data(withJSONObject:store.exportSummary(),options:[.prettyPrinted,.sortedKeys]);try bytes.write(to:url,options:.atomic);exportMessage="Diagnostics exported"} catch {exportMessage="Could not export diagnostics. Choose a writable folder."}}
    }
}
