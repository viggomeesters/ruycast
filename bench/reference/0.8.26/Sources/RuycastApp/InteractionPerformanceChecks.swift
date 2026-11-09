import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkInteractionPerformance() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let trace=PerformanceTrace.shared
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    trace.reset()
    for _ in 0..<350 {trace.measure(.searchModel) {}}
    let bounded=trace.snapshot()["stages"] as! [String:[String:Any]]
    try require(bounded["search.model"]?["count"] as? Int==350 && bounded["search.model"]?["retainedSamples"] as? Int==256,"Trace retention is not bounded")
    let other=NSObject(),owner=ObjectIdentifier(trace)
    trace.inputChanged(fieldID:owner)
    trace.didReconcileField(fieldID:ObjectIdentifier(other));trace.cancelInput(fieldID:ObjectIdentifier(other))
    try require(trace.snapshot()["activeIntervals"] as? Int==1,"Another field completed or cancelled pending input")
    trace.inputChanged(fieldID:owner);trace.cancelInput()
    let cancelled=trace.snapshot()["stages"] as! [String:[String:Any]]
    try require(cancelled["input.reconcile"]?["cancelled"] as? Int==2,"Superseded input was counted as completed")
    let stale=trace.begin(.openReady);trace.reset();let fresh=trace.begin(.openReady);trace.finish(stale)
    try require(trace.snapshot()["activeIntervals"] as? Int==1,"Reset reused a stale interval identity")
    trace.finish(fresh)
    let overflow=(0..<100).map {_ in trace.begin(.inputModel)}
    try require(trace.snapshot()["activeIntervals"] as? Int==32,"In-flight trace state is unbounded")
    overflow.forEach {trace.finish($0,cancelled:true)};trace.reset()
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("trace-"+String(UUID().uuidString.prefix(8)))
    let state=try AppState(root:root,headless:true)
    defer {state.shutdown();try? FileManager.default.removeItem(at:root)}
    state.config.commands=[Command(id:"fixture.success",title:"Success Fixture",kind:"script",target:"/usr/bin/true",aliases:["ranking-exact"]),Command(id:"fixture.failure",title:"Failure Fixture",kind:"script",target:"/usr/bin/false")]
    func executeFixture(_ id:String,source:String,dryRun:Bool=false) throws -> Bool {
        var response:[String:Any]?
        state.execute(id,dryRun:dryRun,values:["input":"private ranking sentinel"],source:source) {response=$0}
        let deadline=Date().addingTimeInterval(5)
        while response==nil && Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.001))}
        guard let response else {throw LauncherError("Ranking fixture timed out")}
        return response["ok"] as? Bool==true
    }
    try require(try executeFixture("fixture.success",source:"cli"),"CLI success fixture failed")
    try require(try !executeFixture("fixture.failure",source:"launcher"),"Failure fixture unexpectedly succeeded")
    try require(try executeFixture("fixture.success",source:"launcher",dryRun:true),"Dry-run fixture failed")
    try require(state.searchRanking.entries.isEmpty,"CLI, failure or dry-run changed launcher recency")
    try require(try executeFixture("fixture.success",source:"launcher"),"Launcher success fixture failed")
    try require(state.searchRanking.entries.count==1 && state.searchRanking.entries["fixture.success"] != nil,"Successful launcher choice was not retained")
    let rankingBytes=try Data(contentsOf:root.appendingPathComponent("search-ranking.json"))
    try require(!String(decoding:rankingBytes,as:UTF8.self).contains("sentinel"),"Ranking retained argument text")
    state.config.commands[0].enabled=false
    try require(!state.searchCommands("ranking-exact",limit:0).contains(where:{$0.id=="fixture.success"}),"Disabled command appeared in ranking")
    state.config.commands=[]
    try require(!state.searchCommands("ranking-exact",limit:0).contains(where:{$0.id=="fixture.success"}),"Removed command appeared in ranking")
    try state.searchRanking.reset()
    state.config.commands=(0..<500).map {Command(id:"fixture.\($0)",title:"Example \($0)",kind:"url",target:"https://example.com/\($0)")}
    let palette=PaletteController(state:state);defer {palette.panel.close()}
    guard let view=palette.panel.contentView else {throw LauncherError("No native palette host")}
    view.frame=NSRect(x:0,y:0,width:760,height:490)
    func field(_ view:NSView)->SearchInput? {if let input=view as? SearchInput {return input};return view.subviews.lazy.compactMap {field($0)}.first}
    view.layoutSubtreeIfNeeded()
    let mountDeadline=Date().addingTimeInterval(1)
    while field(view)==nil && Date()<mountDeadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.001))}
    guard let input=field(view),let delegate=input.delegate as? PaletteSearchField.Coordinator else {throw LauncherError("No retained production search field/delegate")}
    let removedField=SearchInput()
    trace.inputChanged(fieldID:ObjectIdentifier(removedField))
    PaletteSearchField.dismantleNSView(removedField,coordinator:delegate)
    try require(trace.snapshot()["activeIntervals"] as? Int==0,"Removed field retained a pending input span")
    let terms=["E","Ex","Example 4","Example 42",""]
    trace.reset()
    func count(_ stage:String)->Int {(trace.snapshot()["stages"] as! [String:[String:Any]])[stage]?["count"] as? Int ?? 0}
    for i in 0..<100 {
        let drawsBefore=count("host.drawCallback")
        try autoreleasepool {
            input.stringValue=terms[i%terms.count]
            delegate.controlTextDidChange(Notification(name:NSControl.textDidChangeNotification,object:input))
            // Reconciliation must happen through the real representable update,
            // before any offscreen cacheDisplay can force a host draw callback.
            let end=Date().addingTimeInterval(1)
            while count("input.reconcile")<i+1 && Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.001))}
            try require(count("input.reconcile")==i+1,"Retained field did not reconcile before capture in cycle \(i)")
            view.layoutSubtreeIfNeeded();view.needsDisplay=true
            guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("No native render buffer")}
            view.cacheDisplay(in:view.bounds,to:bitmap)
            try require(count("input.reconcile")==i+1 && count("host.drawCallback")>drawsBefore,"Input reconciliation or explicit host capture failed in cycle \(i)")
        }
    }
    let result=trace.snapshot(),stages=result["stages"] as! [String:[String:Any]]
    try require(stages["input.model"]?["count"] as? Int==100,"Native field delegate was not measured")
    try require((stages["host.drawCallback"]?["count"] as? Int ?? 0)>0 && (stages["input.reconcile"]?["count"] as? Int ?? 0)>0,"Field reconciliation or explicit host draw callback was not measured")
    try require(!palette.panel.isVisible,"Background benchmark opened a window")
    let bytes=try JSONSerialization.data(withJSONObject:result),text=String(decoding:bytes,as:UTF8.self)
    try require(!text.contains("Example") && !text.contains("fixture.") && !text.contains("example.com"),"Trace retained input or command content")
    return ["ok":true,"scope":"100 retained native field/delegate/reconciliation cycles before offscreen captures; not result drawing or physical latency","desktopInputSent":false,"visibleWindowOpened":false,"boundedSamples":true,"launcherRecencySuccessOnly":true,"reconciliationBeforeCapture":true,"fieldOwnerAndDismantle":true,"cancelledAndResetSpans":true,"trace":result]
}
