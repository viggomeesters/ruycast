import AppKit
import SwiftUI
import Darwin
import LauncherCore

/// Hidden native view/model cycles. Never orders a window front or installs global input hooks.
func benchmarkLifecycle(blocks:Int=10,render:Bool=true,component:String="palette") throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-l-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true);defer {state.shutdown()}
    state.config.commands=(0..<500).map {Command(id:"fixture.\($0)",title:"Open Project \($0)",kind:"builtin",target:"clipboard")}
    state.clips=(0..<200).map {Clip(text:"Fixture \($0) Ω",source:"fixture")}
    let palette=render ? PaletteController(state:state):nil;defer {palette?.panel.close()}
    if component=="search" {palette?.panel.contentView=NSHostingView(rootView:LifecycleSearchProbe(state:state))}
    if component=="preview" {palette?.panel.contentView=NSHostingView(rootView:ClipboardHistoryView(state:state))}
    if component=="label" {palette?.panel.contentView=NSHostingView(rootView:LifecycleLabelProbe(state:state))}
    func pump(_ seconds:Double=0.002) {let end=Date().addingTimeInterval(seconds);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.0002))}}
    func footprint()->UInt64 {
        var info=task_vm_info_data_t(),count=mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size/MemoryLayout<Int32>.size)
        let status=withUnsafeMutablePointer(to:&info) {p in p.withMemoryRebound(to:integer_t.self,capacity:Int(count)) {task_info(mach_task_self_,task_flavor_t(TASK_VM_INFO),$0,&count)}}
        return status==KERN_SUCCESS ? info.phys_footprint:0
    }
    func block(_ cycles:Int) throws {
        for i in 0..<cycles {try autoreleasepool {
            state.mode="commands";state.query="Project \(i%10)";state.actionsOpen=true;state.moveSelection(1);state.actionsOpen=false
            state.mode="clipboard";state.query="Fixture \(i%10)"
            let deadline=Date().addingTimeInterval(5);while state.clipboardSearchBusy,Date()<deadline {pump()}
            guard !state.clipboardSearchBusy else {throw LauncherError("Lifecycle filter timed out")}
            state.mode="commands";state.query="";palette?.panel.contentView?.layoutSubtreeIfNeeded()
        }}
    }
    func drawFrame() {autoreleasepool {
        guard let view=palette?.panel.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {return}
        view.cacheDisplay(in:view.bounds,to:bitmap)
    }}
    try block(100);pump(1);drawFrame()
    func container(_ view:NSView)->LauncherResultsContainer? {
        if let value=view as? LauncherResultsContainer {return value}
        return view.subviews.lazy.compactMap {container($0)}.first
    }
    let retainedContainer=palette?.panel.contentView.flatMap {container($0)}
    if render,component=="palette",retainedContainer==nil {throw LauncherError("Persistent results container missing")}
    var samples:[[String:Any]]=[["cycles":0,"footprintBytes":footprint()]]
    for i in 1...blocks {try block(100);pump(0.2);drawFrame();samples.append(["cycles":i*100,"footprintBytes":footprint()]);if i%10==0 {fputs("lifecycle cycles=\(i*100) footprintMiB=\(Double(footprint())/1048576)\n",stderr)}}
    if render,component=="palette",let view=palette?.panel.contentView {
        guard container(view) === retainedContainer else {throw LauncherError("Mode switching replaced native results container")}
    }
    pump(2);let end=footprint(),start=samples[0]["footprintBytes"] as! UInt64
    let monitor=SystemMonitorModel()
    for _ in 0..<1000 {monitor.start();monitor.stop()}
    pump(0.2);guard !monitor.active,monitor.history.isEmpty,palette?.panel.isVisible != true else {throw LauncherError("Closed monitor sampled or hidden palette became visible")}
    let beforeRelief=footprint();let released=malloc_zone_pressure_relief(nil,0);pump(0.2);let afterRelief=footprint()
    return ["ok":true,"nativeView":render,"component":component,"diagnosticAllocatorReliefBytes":released,"footprintBeforeRelief":beforeRelief,"footprintAfterRelief":afterRelief,"scope":"Hidden native view/model cycles after100warmup;1000monitor start/stop cancellation cycles; excludes display/focus/eventtaps","cycles":blocks*100,"warmupCycles":100,"startFootprintBytes":start,"endFootprintBytes":end,"growthMiB":Double(Int64(end)-Int64(start))/1048576,"samples":samples,"monitorClosedAfterCancellationStorm":true,"persistentNativeContainers":render && component=="palette","desktopEffects":false]
}

private struct LifecycleSearchProbe:View {
    @ObservedObject var state:AppState
    var body:some View {PaletteSearchField(text:$state.query,placeholder:"Search",onMove:{_ in},onSubmit:{},onCancel:{},onActions:{})}
}
private struct LifecycleLabelProbe:View {
    @ObservedObject var state:AppState
    var body:some View {Text(state.query).textSelection(.enabled)}
}
