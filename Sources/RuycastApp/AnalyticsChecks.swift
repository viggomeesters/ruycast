import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

func checkAnalytics(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-a-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {let end=Date().addingTimeInterval(0.05);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait(_ condition:()->Bool) throws {let end=Date().addingTimeInterval(8);while !condition(),Date()<end {pump()};try require(condition(),"Analytics fixture timed out")}
    let hourlyChecks=try checkHourlyAnalytics()
    let store=AnalyticsStore(root:root,saveDelay:60);store.importLegacy(["legacy":5]);store.importLegacy(["legacy":5])
    let secret="PRIVATE_INPUT_SENTINEL_DONT_LOG"
    DispatchQueue.concurrentPerform(iterations:8) {worker in
        for index in 0..<125 {let id="fixture.\(worker)";if let ticket=store.recordAttempt(id,source:"cli") {store.recordCompletion(ticket,success:index%5 != 0,ms:Double(index),code:"timeout")};store.recordTiming("search.model",ms:Double(index)/100)}
    }
    let beforePause=store.snapshot
    try require(beforePause.hours.values.reduce(0,{$0+$1.uses})==1000 && beforePause.hours.values.reduce(0,{$0+$1.successes})==800 && beforePause.hours.values.reduce(0,{$0+$1.failures})==200,"Concurrent hourly counts changed")
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
    return ["ok":true,"concurrentAttempts":1000,"legacySuccessesPreservedOnce":true,"boundedSamplesAndEvents":true,"pausePreservesInflightCompletion":true,"privateFile":true,"inputQueryAndIDsExcludedFromExport":true,"retention":true,"corruptDataPreservedUntilReset":true,"overflowRejected":true,"guiHotkeyCLIAndDryRun":true,"rustCLIControls":true,"resetPreservesConfig":true,"rawCommandIDsHashed":true,"resetDiscardsPriorInflight":true,"dashboardLightDarkMinWidthRenders":4,"hourlyAnalytics":hourlyChecks,"desktopInputSent":false,"visibleWindowOpened":false]
}

/// Controlled clocks exercise storage without waiting for real hour boundaries or changing system time.
func checkHourlyAnalytics() throws -> [String:Any] {
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-hourly-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    let iso=ISO8601DateFormatter()
    var clock=iso.date(from:"2026-10-25T00:35:00Z")!
    var amsterdam=Calendar(identifier:.gregorian);amsterdam.timeZone=TimeZone(identifier:"Europe/Amsterdam")!
    let store=AnalyticsStore(root:root,saveDelay:60,now:{clock},calendar:amsterdam)
    store.importLegacy(["earlier":7])
    let first=store.recordAttempt("first",source:"launcher")!,firstHour=AnalyticsStore.hourKey(clock)
    clock=iso.date(from:"2026-10-25T01:15:00Z")!
    store.recordCompletion(first,success:true,ms:3)
    let second=store.recordAttempt("second",source:"hotkey")!,secondHour=AnalyticsStore.hourKey(clock)
    store.recordCompletion(second,success:false,ms:2,code:"timeout")
    let d=store.snapshot
    try require(firstHour != secondHour && d.hours.count==2 && d.hours[firstHour]?.uses==1 && d.hours[firstHour]?.successes==1 && d.hours[secondHour]?.uses==1 && d.hours[secondHour]?.failures==1,"Hour boundaries or in-flight completion attribution changed")
    let formatter=DateFormatter();formatter.calendar=amsterdam;formatter.timeZone=amsterdam.timeZone;formatter.locale=Locale(identifier:"en_US_POSIX");formatter.dateFormat="HH:mm"
    try require(formatter.string(from:Date(timeIntervalSince1970:Double(firstHour)!))=="02:00" && formatter.string(from:Date(timeIntervalSince1970:Double(secondHour)!))=="02:00","Repeated local-hour fixture is invalid")
    let points=d.hourlyActivity(now:clock)
    try require(points.count==24 && Set(points.map(\.date)).count==24 && points.compactMap(\.activity).reduce(0,{$0+$1.uses})==2 && points.filter {$0.activity==nil}.count==22,"Last-24-hour bins merged DST hours or invented older recordings")
    var otherZone=amsterdam;otherZone.timeZone=TimeZone(identifier:"Asia/Kathmandu")!
    store.flushSynchronously()
    let reloaded=AnalyticsStore(root:root,now:{clock},calendar:otherZone)
    try require(reloaded.storageError==nil && reloaded.snapshot.hours[firstHour]?.successes==1 && reloaded.snapshot.hours[secondHour]?.failures==1,"Hourly data changed after reload or time-zone change")

    // Build an actual schema-1 payload without either new field.
    let legacyRoot=root.appendingPathComponent("legacy")
    var old=try JSONSerialization.jsonObject(with:Files.encode(d)) as! [String:Any]
    old["schema"]=1;old.removeValue(forKey:"hours");old.removeValue(forKey:"hourlyStartedAt")
    try FileManager.default.createDirectory(at:legacyRoot,withIntermediateDirectories:true)
    try JSONSerialization.data(withJSONObject:old).write(to:legacyRoot.appendingPathComponent("analytics.json"))
    let upgraded=AnalyticsStore(root:legacyRoot,saveDelay:60,now:{clock},calendar:amsterdam),legacy=upgraded.snapshot
    try require(upgraded.storageError==nil && legacy.schema==2 && legacy.totalAttempts==2 && legacy.totalSuccesses==1 && legacy.totalFailures==1 && legacy.priorSuccesses==7 && legacy.legacyImported && legacy.commands.count==d.commands.count && legacy.events.count==d.events.count && legacy.timings.keys.sorted()==d.timings.keys.sorted() && legacy.days.keys.sorted()==d.days.keys.sorted(),"Schema-1 upgrade lost existing analytics")
    try require(legacy.hours.isEmpty && legacy.hourlyStartedAt==clock && legacy.hourlyActivity(now:clock).dropLast().allSatisfy {$0.activity==nil},"Schema-1 upgrade fabricated historical hourly counts")
    _=upgraded.recordAttempt("new-hour",source:"cli");upgraded.flushSynchronously()
    let upgradedAgain=AnalyticsStore(root:legacyRoot,now:{clock},calendar:amsterdam)
    try require(upgradedAgain.storageError==nil && upgradedAgain.snapshot.totalAttempts==3 && upgradedAgain.snapshot.priorSuccesses==7 && upgradedAgain.snapshot.hours.values.reduce(0,{$0+$1.uses})==1 && upgradedAgain.snapshot.hourlyStartedAt==clock,"Schema-2 reload lost upgraded totals or hourly recording start")

    let retentionRoot=root.appendingPathComponent("retention")
    let retained=AnalyticsStore(root:retentionRoot,saveDelay:60,now:{clock},calendar:amsterdam)
    try retained.configure(enabled:nil,retentionDays:1)
    let beginning=clock
    for offset in 0..<26 {clock=beginning.addingTimeInterval(Double(offset)*3600);_=retained.recordAttempt("retention",source:"cli")}
    try require(retained.snapshot.totalAttempts==26 && retained.snapshot.hours.count==25 && retained.snapshot.hours[AnalyticsStore.hourKey(beginning)]==nil && retained.snapshot.hourlyActivity(now:clock).count==24,"Hourly retention failed to prune or removed all-time totals")
    retained.flushSynchronously()
    try require(AnalyticsStore(root:retentionRoot,now:{clock},calendar:amsterdam).snapshot.hours.count==25,"Pruned hours reappeared on reload")

    let boundedRoot=root.appendingPathComponent("bounded")
    var bounded=AnalyticsDocument();bounded.retentionDays=90;bounded.hourlyStartedAt=clock.addingTimeInterval(-2160*3600)
    for offset in 0...2160 {bounded.hours[AnalyticsStore.hourKey(clock.addingTimeInterval(-Double(offset)*3600))]=ActivityDay(uses:1)}
    try Files.save(bounded,to:boundedRoot.appendingPathComponent("analytics.json"))
    let boundedStore=AnalyticsStore(root:boundedRoot,saveDelay:60,now:{clock},calendar:amsterdam)
    try require(boundedStore.storageError==nil && boundedStore.snapshot.hours.count==2161,"Maximum hourly retention fixture was rejected")
    clock=clock.addingTimeInterval(3600);_=boundedStore.recordAttempt("boundary",source:"cli")
    try require(boundedStore.snapshot.hours.count==2161,"Hourly aggregate storage exceeded the 90-day bound")
    var invalid=boundedStore.snapshot;invalid.hours.removeValue(forKey:invalid.hours.keys.sorted().first!);invalid.hours["PRIVATE_QUERY_SENTINEL"]=ActivityDay(uses:1)
    let invalidRoot=root.appendingPathComponent("invalid");try Files.save(invalid,to:invalidRoot.appendingPathComponent("analytics.json"))
    let invalidBytes=try Data(contentsOf:invalidRoot.appendingPathComponent("analytics.json"))
    let rejected=AnalyticsStore(root:invalidRoot,now:{clock},calendar:amsterdam)
    rejected.recordError("runtime",code:"invalid");rejected.flushSynchronously()
    try require(rejected.storageError != nil && (try Data(contentsOf:invalidRoot.appendingPathComponent("analytics.json")))==invalidBytes,"Invalid hourly keys were accepted or existing data was overwritten")
    return ["schema1Preserved":true,"noBackfilledHours":true,"hourlyReload":true,"completionAttributedToAttemptHour":true,"last24ElapsedHours":true,"dstHoursDistinct":true,"timeZoneChangePreservesHours":true,"retentionPreservesTotals":true,"hourlyBucketsBounded":2161,"invalidHourlyDataPreserved":true]
}
