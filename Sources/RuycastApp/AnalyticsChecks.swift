import AppKit
import SwiftUI
import LauncherCore

func checkAnalytics(_ outputPath:String) throws -> [String:Any] {
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
    let beforePause=store.snapshot
    try require(beforePause.totalAttempts==1000 && beforePause.totalSuccesses==800 && beforePause.totalFailures==200 && beforePause.priorSuccesses==5,"Concurrent counts or legacy import changed")
    try require(beforePause.events.count==200 && beforePause.timings["search.model"]?.samples.count==256 && beforePause.commands[AnalyticsStore.commandKey("fixture.0")]?.timing.samples.count==32,"Analytics buffers are unbounded")
    let tracked=store.recordAttempt("inflight",source:"hotkey");try store.configure(enabled:false,retentionDays:7)
    try require(store.recordAttempt("paused",source:"cli")==nil,"Paused analytics accepted an attempt")
    if let tracked {store.recordCompletion(tracked,success:true,ms:1)}
    try require(store.snapshot.totalAttempts==1001 && store.snapshot.totalSuccesses==801,"Pause lost an accepted in-flight completion")
    try store.configure(enabled:true,retentionDays:nil);store.recordError("config",code:secret);store.recordTiming(secret,ms:1)
    store.flushSynchronously();let reloaded=AnalyticsStore(root:root)
    try require(reloaded.snapshot.totalAttempts==1001 && reloaded.storageError==nil,"Analytics persistence did not round trip")
    let bytes=try Data(contentsOf:root.appendingPathComponent("analytics.json"));let text=String(decoding:bytes,as:UTF8.self)
    try require(!text.contains(secret),"Arbitrary diagnostic content was persisted")
    let sensitiveID="/Users/private/"+secret+"/https://private.example/token"
    let privateTicket=store.recordAttempt(sensitiveID,source:"cli")!;store.recordCompletion(privateTicket,success:false,ms:1);store.flushSynchronously()
    try require(!String(decoding:try Data(contentsOf:root.appendingPathComponent("analytics.json")),as:UTF8.self).contains(secret),"Command ID leaked a raw URL/path into analytics")
    let exported=try JSONSerialization.data(withJSONObject:store.exportSummary())
    try require(!String(decoding:exported,as:UTF8.self).contains("fixture.0") && !String(decoding:exported,as:UTF8.self).contains(root.path),"Diagnostic export leaked command identities or profile path")
    let permissions=try FileManager.default.attributesOfItem(atPath:root.appendingPathComponent("analytics.json").path)[.posixPermissions] as? NSNumber
    try require(permissions?.intValue==0o600,"Analytics file is not private")
    let bad=root.appendingPathComponent("bad");try FileManager.default.createDirectory(at:bad,withIntermediateDirectories:true)
    let badURL=bad.appendingPathComponent("analytics.json"),original=Data("broken personal file".utf8);try original.write(to:badURL)
    let broken=AnalyticsStore(root:bad);broken.recordAttempt("test",source:"cli");broken.flushSynchronously()
    try require(broken.storageError != nil && (try Data(contentsOf:badURL))==original,"Malformed analytics was overwritten automatically")
    broken.reset();broken.flushSynchronously();try require(broken.storageError==nil && AnalyticsStore(root:bad).storageError==nil,"Explicit reset did not recover malformed analytics")
    var old=store.snapshot;old.events=[DiagnosticEvent(date:Date().addingTimeInterval(-100*86400),category:"runtime",code:"interrupted")]
    let stale=root.appendingPathComponent("stale");try Files.save(old,to:stale.appendingPathComponent("analytics.json"))
    try require(AnalyticsStore(root:stale).snapshot.events.isEmpty,"Diagnostic retention kept old events")
    var invalid=old;invalid.totalAttempts=Int.max
    let overflow=root.appendingPathComponent("overflow");try Files.save(invalid,to:overflow.appendingPathComponent("analytics.json"))
    try require(AnalyticsStore(root:overflow).storageError != nil,"Counter overflow fixture was accepted")
    let fixture=root.appendingPathComponent("host"),state=try AppState(root:fixture,headless:true)
    state.config.commands=[Command(id:"fixture.success",title:"Open Daily Note",kind:"script",target:"/usr/bin/true"),Command(id:"fixture.failure",title:"Run Maintenance",kind:"script",target:"/usr/bin/false")]
    state.query=secret
    func execute(_ id:String,_ source:String,dryRun:Bool=false) throws -> [String:Any] {
        var reply:[String:Any]?
        state.execute(id,dryRun:dryRun,values:["input":secret],source:source) {reply=$0}
        try wait {reply != nil};return reply!
    }
    _=try execute("fixture.success","launcher");_=try execute("fixture.success","hotkey");_=try execute("fixture.failure","cli");_=try execute("fixture.success","cli",dryRun:true)
    try require(state.analytics.snapshot.totalAttempts==3 && state.analytics.snapshot.totalSuccesses==2 && state.analytics.snapshot.totalFailures==1,"Execution or dry-run instrumentation counted incorrectly")
    try require(state.analytics.snapshot.commands[AnalyticsStore.commandKey("fixture.success")]?.sources==["launcher":1,"hotkey":1],"Execution sources were lost")
    let server=ControlServer(root:fixture);try server.start {request,reply in state.handle(request,reply:reply)};defer {server.stop()}
    func cli(_ arguments:[String]) throws -> [String:Any] {
        let process=Process(),pipe=Pipe();process.executableURL=ruycastCLI();process.arguments=arguments;var env=ProcessInfo.processInfo.environment;env["RUYCAST_HOME"]=fixture.path;process.environment=env;process.standardOutput=pipe;process.standardError=FileHandle.nullDevice;try process.run()
        try wait {!process.isRunning};return try JSONSerialization.jsonObject(with:pipe.fileHandleForReading.readDataToEndOfFile()) as! [String:Any]
    }
    try require(try cli(["analytics"])["ok"] as? Bool==true,"Analytics CLI summary failed")
    try require(try cli(["analytics","pause"])["ok"] as? Bool==true && !state.analytics.snapshot.enabled,"Analytics CLI pause failed")
    _=try execute("fixture.success","cli");try require(state.analytics.snapshot.totalAttempts==3,"Paused execution was counted")
    try require(try cli(["analytics","resume"])["ok"] as? Bool==true,"Analytics CLI resume failed")
    let output=try cli(["analytics","export"]);try require(output["ok"] as? Bool==true,"Analytics CLI export failed")
    state.analytics.flushSynchronously()
    let contents=String(decoding:try Data(contentsOf:fixture.appendingPathComponent("analytics.json")),as:UTF8.self)
    try require(!contents.contains(secret),"Input/query body leaked through real execution logging")
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    for dark in [true,false] {for width in [980,680] {
        let view=NSHostingView(rootView:AnalyticsDashboard(store:state.analytics,commands:state.config.commands,theme:SettingsTheme(dark:dark)))
        view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:width,height:1400);pump();view.layoutSubtreeIfNeeded()
        let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds)!;view.cacheDisplay(in:view.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("analytics-\(dark ? "dark":"light")-\(width).png"))
    }}
    var routed=false;state.showAnalytics={routed=true};let palette=PaletteController(state:state);palette.show("analytics")
    try require(routed && !palette.panel.isVisible,"Dashboard did not use management window routing")
    let resetWithout=try cli(["analytics","reset"]);try require(resetWithout["ok"] as? Bool==false,"Analytics reset lacked confirmation")
    let configBefore=try Data(contentsOf:state.configURL)
    let preReset=state.analytics.recordAttempt("old-inflight",source:"cli")!
    _=try cli(["analytics","reset","--confirm"])
    state.analytics.recordCompletion(preReset,success:true,ms:1)
    try require(state.analytics.snapshot.totalAttempts==0 && state.analytics.snapshot.totalSuccesses==0 && state.analytics.snapshot.timings.isEmpty && state.analytics.snapshot.commands.isEmpty && state.analytics.snapshot.events.isEmpty && (try Data(contentsOf:state.configURL))==configBefore,"Reset changed configuration or retained metrics")
    return ["ok":true,"concurrentAttempts":1000,"legacySuccessesPreservedOnce":true,"boundedSamplesAndEvents":true,"pausePreservesInflightCompletion":true,"privateFile":true,"inputQueryAndIDsExcludedFromExport":true,"retention":true,"corruptDataPreservedUntilReset":true,"overflowRejected":true,"guiHotkeyCLIAndDryRun":true,"rustCLIControls":true,"resetPreservesConfig":true,"rawCommandIDsHashed":true,"resetDiscardsPriorInflight":true,"dashboardLightDarkMinWidthRenders":4,"desktopInputSent":false,"visibleWindowOpened":false]
}
