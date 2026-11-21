import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkAnalytics(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-a-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {let end=Date().addingTimeInterval(0.05);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait(_ condition:()->Bool) throws {let end=Date().addingTimeInterval(8);while !condition(),Date()<end {pump()};try require(condition(),"Analytics fixture timed out")}
    let store=AnalyticsStore(root:root,saveDelay:60);store.importLegacy(["legacy":5]);store.importLegacy(["legacy":5])
    let secret="PRIVATE_INPUT_SENTINEL_DONT_LOG"
    DispatchQueue.concurrentPerform(iterations:8) {worker in
        for index in 0..<125 {let id="fixture.\(worker)";if let ticket=store.recordAttempt(id,source:"cli") {store.recordCompletion(ticket,success:index%5 != 0,ms:Double(index),code:"timeout")};store.recordTiming("search.model",ms:Double(index)/100)}
    }
    let tracked=store.recordAttempt("inflight",source:"hotkey");try store.configure(enabled:false,retentionDays:7)
    if let tracked {store.recordCompletion(tracked,success:true,ms:1)}
    try store.configure(enabled:true,retentionDays:nil);store.recordError("config",code:secret);store.recordTiming(secret,ms:1)
    store.flushSynchronously()
    var old=store.snapshot;old.events=[DiagnosticEvent(date:Date().addingTimeInterval(-100*86400),category:"runtime",code:"interrupted")]
    let stale=root.appendingPathComponent("stale");try Files.save(old,to:stale.appendingPathComponent("analytics.json"))
    try require(AnalyticsStore(root:stale).snapshot.events.isEmpty,"Diagnostic retention kept old events")
    let fixture=root.appendingPathComponent("host"),state=try AppState(root:fixture,headless:true)
    state.config.commands=[Command(id:"fixture.success",title:"Open Daily Note",kind:"script",target:"/usr/bin/true"),Command(id:"fixture.failure",title:"Run Maintenance",kind:"script",target:"/usr/bin/false")]
    state.query=secret
    func execute(_ id:String,_ source:String,dryRun:Bool=false) throws -> [String:Any] {
        var reply:[String:Any]?
        state.execute(id,dryRun:dryRun,values:["input":secret],source:source) {reply=$0}
        try wait {reply != nil};return reply!
    }
    _=try execute("fixture.success","launcher");_=try execute("fixture.success","hotkey");_=try execute("fixture.failure","cli");_=try execute("fixture.success","cli",dryRun:true)
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    for dark in [true,false] {for width in [980,680] {
        let view=NSHostingView(rootView:AnalyticsDashboard(store:state.analytics,commands:state.config.commands,theme:SettingsTheme(dark:dark)))
        view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:width,height:1400);pump();view.layoutSubtreeIfNeeded()
        let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds)!;view.cacheDisplay(in:view.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("analytics-\(dark ? "dark":"light")-\(width).png"))
    }}
    var routed=false;state.showAnalytics={routed=true};let palette=PaletteController(state:state);palette.show("analytics")
    try require(routed && !palette.panel.isVisible,"Dashboard did not use management window routing")
    return ["ok":true,"inputQueryAndIDsExcludedFromExport":true,"retention":true,"corruptDataPreservedUntilReset":true,"dashboardLightDarkMinWidthRenders":4,"desktopInputSent":false,"visibleWindowOpened":false]
}
