import AppKit
import SwiftUI
import LauncherCore

func checkInteractionPerformance() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let trace=PerformanceTrace.shared
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    trace.reset()
    for _ in 0..<350 {trace.measure(.searchModel) {}}
    let bounded=trace.snapshot()["stages"] as! [String:[String:Any]]
    try require(bounded["search.model"]?["count"] as? Int==350 && bounded["search.model"]?["retainedSamples"] as? Int==256,"Trace retention is not bounded")
    trace.inputChanged();trace.inputChanged();trace.cancelDisplay()
    let cancelled=trace.snapshot()["stages"] as! [String:[String:Any]]
    try require(cancelled["input.draw"]?["cancelled"] as? Int==2,"Superseded input was counted as completed")
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
    let input=SearchInput()
    let field=PaletteSearchField(text:Binding(get:{state.query},set:{state.query=$0}),placeholder:"Search",onMove:state.moveSelection,onSubmit:{},onCancel:{},onActions:{})
    let delegate=PaletteSearchField.Coordinator(field)
    let terms=["E","Ex","Example 4","Example 42",""]
    trace.reset()
    func count(_ stage:String)->Int {(trace.snapshot()["stages"] as! [String:[String:Any]])[stage]?["count"] as? Int ?? 0}
    for i in 0..<100 {
        let drawsBefore=count("native.draw")
        try autoreleasepool {
            input.stringValue=terms[i%terms.count]
            delegate.controlTextDidChange(Notification(name:NSControl.textDidChangeNotification,object:input))
            let end=Date().addingTimeInterval(0.002)
            while Date()<end {_=RunLoop.current.run(mode:.default,before:end)}
            view.layoutSubtreeIfNeeded();view.needsDisplay=true
            guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("No native render buffer")}
            view.cacheDisplay(in:view.bounds,to:bitmap)
            try require(count("input.draw")==i+1 && count("native.draw")>drawsBefore,"Input did not complete exactly one native draw interval in cycle \(i)")
        }
    }
    let result=trace.snapshot(),stages=result["stages"] as! [String:[String:Any]]
    try require(stages["input.model"]?["count"] as? Int==100,"Native field delegate was not measured")
    try require((stages["native.draw"]?["count"] as? Int ?? 0)>0 && (stages["input.draw"]?["count"] as? Int ?? 0)>0,"Native draw hook did not complete software input timings")
    try require(!palette.panel.isVisible,"Background benchmark opened a window")
    let bytes=try JSONSerialization.data(withJSONObject:result),text=String(decoding:bytes,as:UTF8.self)
    try require(!text.contains("Example") && !text.contains("fixture.") && !text.contains("example.com"),"Trace retained input or command content")
    return ["ok":true,"scope":"100 hidden native field/model/draw cycles; not physical hotkey or screen latency","desktopInputSent":false,"visibleWindowOpened":false,"boundedSamples":true,"launcherRecencySuccessOnly":true,"cancelledAndResetSpans":true,"trace":result]
}
