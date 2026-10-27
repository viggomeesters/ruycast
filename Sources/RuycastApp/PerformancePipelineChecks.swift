import AppKit
import Observation
import Darwin
import LauncherCore
import LauncherModels

@MainActor func checkPerformancePipeline() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-q-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func wait(_ condition:()->Bool,line:Int=#line) throws {let end=Date().addingTimeInterval(8);while !condition(),Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.002))};try require(condition(),"Pipeline fixture timed out at line \(line)")}
    let state=try AppState(root:root,headless:true);defer {state.shutdown()}
    func request(_ value:[String:Any]) throws -> [String:Any] {var response:[String:Any]?;state.handle(value) {response=$0};try wait {response != nil};try require(response!["ok"] as? Bool==true,"Pipeline operation failed: \(response!)");return response!}
    func disk()->[Clip] {(try? Files.decode([Clip].self,data:Data(contentsOf:root.appendingPathComponent("clipboard.json")))) ?? []}
    func capture(_ text:String,publish:Bool=true) throws {
        try require(state.processClipboardPayload(.text(text),source:"fixture"),"Capture not accepted")
        try require(!state.processClipboardPayload(.text("must not enqueue"),source:"fixture"),"Capture queue is unbounded")
        state.clipboardWork.drain() // Disk complete, publication deliberately not pumped yet.
        if publish {try wait {!state.clipboardWorkInFlight}}
    }
#if DEBUG || RUYCAST_POLL_BENCHMARK
    // Exercise the real production polling boundary with an owned pasteboard.
    let pollRoot=root.appendingPathComponent("poll")
    let pollState=try AppState(root:pollRoot,headless:true);defer {pollState.shutdown()}
    let board=NSPasteboard(name:NSPasteboard.Name("ruycast-poll-"+UUID().uuidString));defer {board.releaseGlobally()}
    var appReads=0
    func application()->NSRunningApplication? {appReads+=1;return .current}
    board.clearContents();board.setString("initial",forType:.string)
    pollState.pollClipboard(from:board,activeApplication:application) // Unknown initial owner is discarded.
    try require(!pollState.clipboardWorkInFlight,"Uncertain initial owner was captured")
    appReads=0
    for _ in 0..<1000 {pollState.pollClipboard(from:board,activeApplication:application)}
    try require(appReads==0,"Unchanged clipboard polls still consult application attribution")
    board.clearContents();board.setString("stable owner",forType:.string)
    pollState.pollClipboard(from:board,activeApplication:application);try wait {!pollState.clipboardWorkInFlight}
    try require(pollState.clips.first?.text=="stable owner","Stable owner capture was lost")
    pollState.clipboardApplicationActivated(getpid())
    appReads=0;pollState.pollClipboard(from:board,activeApplication:application)
    try require(appReads==1,"Activation uncertainty did not settle on an unchanged poll")
    pollState.pollClipboard(from:board,activeApplication:application)
    try require(appReads==1,"Settled attribution still repeats application work")
    board.clearContents();board.setString("transition",forType:.string);pollState.clipboardApplicationActivated(getpid())
    pollState.pollClipboard(from:board,activeApplication:application)
    try require(!pollState.clipboardWorkInFlight && !pollState.clips.contains {$0.text=="transition"},"Uncertain app transition captured clipboard")
    board.clearContents();board.setString("settled again",forType:.string)
    pollState.pollClipboard(from:board,activeApplication:application);try wait {!pollState.clipboardWorkInFlight}
    try require(pollState.clips.first?.text=="settled again","Capture did not resume after app transition")
    board.clearContents();board.setString("concealed",forType:.string);board.setData(Data(),forType:NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
    pollState.pollClipboard(from:board,activeApplication:application)
    try require(!pollState.clipboardWorkInFlight,"Concealed clipboard was captured")
    pollState.config.clipboard.excludedApps=[NSRunningApplication.current.bundleIdentifier ?? "unknown"]
    board.clearContents();board.setString("excluded",forType:.string);pollState.pollClipboard(from:board,activeApplication:application)
    try require(!pollState.clipboardWorkInFlight,"Excluded source was captured")
    pollState.config.clipboard.excludedApps=[]
    board.clearContents();board.setString("before provider mutation",forType:.string)
    var mutationReads=0
    pollState.pollClipboard(from:board) {
        mutationReads+=1
        if mutationReads==1 {board.clearContents();board.setString("after provider mutation",forType:.string)}
        return .current
    }
    try require(!pollState.clipboardWorkInFlight,"Pasteboard mutation during attribution was accepted")
    pollState.pollClipboard(from:board,activeApplication:application);try wait {!pollState.clipboardWorkInFlight}
    try require(pollState.clips.first?.text=="after provider mutation","Next poll lost the changed pasteboard count")
    try require(pollState.processClipboardPayload(.text("already in flight"),source:"fixture"),"In-flight poll fixture could not start")
    board.clearContents();board.setString("after in flight",forType:.string)
    appReads=0;pollState.pollClipboard(from:board,activeApplication:application)
    try require(appReads==0,"In-flight poll consulted attribution or queued another capture")
    try wait {!pollState.clipboardWorkInFlight}
    try require(pollState.clips.first?.text=="already in flight","Poll replaced the accepted in-flight capture")
    pollState.pollClipboard(from:board,activeApplication:application);try wait {!pollState.clipboardWorkInFlight}
    try require(pollState.clips.first?.text=="after in flight","Polling did not resume after the in-flight capture")
    pollState.paused=true
    appReads=0;pollState.pollClipboard(from:board,activeApplication:application)
    try require(appReads==0,"Paused poll consulted application")
    pollState.paused=false;pollState.config.clipboard.enabled=false
    pollState.pollClipboard(from:board,activeApplication:application)
    try require(appReads==0,"Disabled poll consulted application")
    pollState.config.clipboard.enabled=true
    board.clearContents();board.setString("owner changed during capture",forType:.string)
    var captureReads=0
    pollState.pollClipboard(from:board) {captureReads+=1;return captureReads==1 ? .current:nil}
    try require(captureReads==2 && !pollState.clipboardWorkInFlight,"Source change during snapshot was accepted")
#endif
    try capture("one")
    let one=state.clips[0]
    try capture("capture before import",publish:false)
    var imported=Clip(text:"imported",source:"fixture");imported.id="raycast-fixture"
    _=try request(["op":"clipboard.import","items":[jsonValue(imported)]])
    try wait {!state.clipboardWorkInFlight}
    try require(Set(state.clips.map(\.text))==Set(["one","capture before import","imported"]) && disk()==state.clips,"Import overwrote an in-flight capture")
    try capture("capture before delete",publish:false)
    _=try request(["op":"clipboard.delete","id":one.id]);try wait {!state.clipboardWorkInFlight}
    try require(state.clips.contains {$0.text=="capture before delete"} && !state.clips.contains {$0.id==one.id} && disk()==state.clips,"Delete overwrote an in-flight capture")
    try capture("capture before reload",publish:false)
    var config=state.config;config.clipboard.limit=100
    try Files.save(config,to:state.configURL);try state.reload();try wait {!state.clipboardWorkInFlight}
    try require(state.clips.contains {$0.text=="capture before reload"} && disk()==state.clips,"Reload lost accepted capture")
    try capture("capture before clear",publish:false)
    _=try request(["op":"clipboard.clear"]);try wait {!state.clipboardWorkInFlight}
    try require(state.clips.isEmpty && disk().isEmpty,"Clear resurrected an in-flight capture")
    try require(!FileManager.default.fileExists(atPath:root.appendingPathComponent("clipboard-assets").path),"Text maintenance created image storage")
    state.clips=(0..<200).map {i in Clip(text:"Fixture \(i): "+String(repeating:"Über CAFÉ Ω ",count:600),source:"fixture")}
    state.mode="clipboard";try require(state.filteredClips==state.clips,"Empty query did not populate clipboard")
    state.query="CAFÉ";try require(state.filteredClips.isEmpty,"Old query results remained actionable")
    try wait {!state.clipboardSearchBusy};let reference=state.clips.filter {$0.text.localizedCaseInsensitiveContains("CAFÉ")}
    try require(state.filteredClips==reference,"Unicode clipboard filter semantics changed")
    for i in 0..<1000 {state.query="missing \(i)"};state.query="Fixture 42:"
    try wait {!state.clipboardSearchBusy};try require(state.filteredClips.count==1 && state.filteredClips[0].text.hasPrefix("Fixture 42:"),"Stale clipboard search won")
    state.paletteVisible=true;state.selection=0;let selected=state.filteredClips[0].id
    state.clips.insert(Clip(text:"Fixture 42: new",source:"fixture"),at:0)
    try wait {!state.clipboardSearchBusy};try require(state.filteredClips[state.selection].id==selected,"Capture moved the selected clipboard identity")
    let retained=state.filteredClips
    state.mode="commands";state.query="terminal"
    let settleGate=Date().addingTimeInterval(0.05);while Date()<settleGate {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    let clipboardChanges=LockedValue(0)
    withObservationTracking {_=state.filteredClips;_=state.clips;_=state.clipboardSelection;_=state.clipboardQuery;_=state.paused;_=state.clipboardSearchBusy} onChange:{clipboardChanges.update {$0+=1}}
    for i in 0..<1000 {state.query="terminal \(i)"}
    let settleTyping=Date().addingTimeInterval(0.05);while Date()<settleTyping {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    try require(state.filteredClips==retained && !state.clipboardSearchBusy && clipboardChanges.read {$0}==0,"Hidden clipboard redrew or searched while command typing")
    state.clips.removeAll {$0.id==selected}
    try require(clipboardChanges.read {$0}==1,"Clipboard data change did not invalidate its tracked consumer")
    try require(!state.filteredClips.contains {$0.id==selected},"Deleted item remained in retained clipboard snapshot")
    let preservedClips=state.clips
    state.clips=[];try require(state.filteredClips.isEmpty,"Hidden clipboard clear retained deleted content")
    state.clips=preservedClips
    state.mode="clipboard";try require(state.filteredClips==state.clips,"Mode re-entry left clipboard empty")
    state.mode="commands";state.paletteVisible=false
    var observed=state.config;observed.commands.append(Command(id:"fixture.atomic",title:"Atomic update",kind:"url",target:"https://example.com"))
    try Files.save(observed,to:state.configURL);try wait {state.config.commands.contains {$0.id=="fixture.atomic"}}
    observed.commands.append(Command(id:"fixture.inplace",title:"In-place update",kind:"url",target:"https://example.com/inplace"))
    let handle=try FileHandle(forWritingTo:state.configURL);try handle.truncate(atOffset:0);try handle.write(contentsOf:Files.encode(observed));try handle.close()
    try wait {state.config.commands.contains {$0.id=="fixture.inplace"}}
    let valid=try Data(contentsOf:state.configURL)
    try Data("not json".utf8).write(to:state.configURL,options:.atomic);try wait {state.configError != nil}
    try require(state.config.commands.contains {$0.id=="fixture.atomic"},"Invalid config replaced working runtime")
    try valid.write(to:state.configURL,options:.atomic);try wait {state.configError==nil}
    let settle=Date().addingTimeInterval(0.25);while Date()<settle {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    var count:Int?;state.configurationReads {count=$0};try wait {count != nil};let before=count!
    for i in 0..<20 {try Data("\(i)".utf8).write(to:root.appendingPathComponent("unrelated.json"),options:.atomic)}
    let end=Date().addingTimeInterval(0.25);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    count=nil;state.configurationReads {count=$0};try wait {count != nil}
    try require(count==before,"Unrelated profile writes reread configuration bytes")
    let readsAfter=count!
    // A private queue's accepted image capture must round trip without touching the system clipboard.
    _=try request(["op":"clipboard.clear"])
    let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:64,pixelsHigh:64,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:256,bitsPerPixel:32)!
    memset(bitmap.bitmapData!,100,64*64*4);let png=bitmap.representation(using:.png,properties:[:])!
    try require(state.processClipboardPayload(.image(png),source:"fixture"),"Image not accepted");try wait {!state.clipboardWorkInFlight}
    try require(state.clips.count==1 && state.clips[0].contentType=="image" && disk()==state.clips && (try? state.clipboardStore.previewImageData(state.clips[0])) != nil,"Background image did not persist")
    let restarted=try AppState(root:root,headless:true);defer {restarted.shutdown()}
    try require(restarted.clips==state.clips,"Background capture did not survive restart")
    _=try request(["op":"clipboard.clear"])
    let assets=root.appendingPathComponent("clipboard-assets")
    try FileManager.default.removeItem(at:assets);try Data("unavailable folder".utf8).write(to:assets)
    try capture("saved despite cleanup failure")
    try require(state.clips.first?.text=="saved despite cleanup failure" && disk()==state.clips && state.clipboardCleanupError != nil,"Cleanup failure lost a persisted capture or hid its error")
    let result:[String:Any]=["ok":true,"cleanupFailurePreservesCapture":true,"captureQueueBounded":true,"importDeleteReloadPreserveAcceptedCapture":true,"clearPreventsResurrection":true,"imageRoundTripAndRestart":true,"unicodeFilterParity":true,"rapidQueries":1000,"selectedIdentityPreserved":true,"atomicConfigReloadAndInvalidRecovery":true,"unrelatedConfigReads":readsAfter-before,"desktopEffects":false]
#if DEBUG || RUYCAST_POLL_BENCHMARK
    return result.merging(["unchangedClipboardPollsSkipAttribution":true,"clipboardAttributionTransitionsPreserved":true]) {_,new in new}
#else
    return result
#endif
}

#if RUYCAST_POLL_BENCHMARK
/// Matched polling work on a named pasteboard, never desktop input or user content.
/// Tight-loop CPU cost is separate from the unchanged 0.75-second capture timer.
@MainActor func benchmarkClipboardPoll(ticks:Int,active:Bool) throws -> [String:Any] {
    guard ProcessInfo.processInfo.environment["RUYCAST_CONFIG_DIR"]==nil else {throw LauncherError("Unset RUYCAST_CONFIG_DIR before running the isolated polling fixture.")}
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-poll-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true);defer {state.shutdown()}
    state.config.clipboard.limit=20
    let board=NSPasteboard(name:NSPasteboard.Name("ruycast-poll-bench-"+UUID().uuidString));defer {board.releaseGlobally()}
    var reads=0,pids=Set<pid_t>()
    func application()->NSRunningApplication? {
        reads+=1;let app=NSWorkspace.shared.frontmostApplication
        if let pid=app?.processIdentifier {pids.insert(pid)}
        return app
    }
    func settle() throws {
        let end=Date().addingTimeInterval(5)
        while state.clipboardWorkInFlight,Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.0002))}
        if state.clipboardWorkInFlight {throw LauncherError("Polling fixture capture timed out")}
    }
    board.clearContents();board.setString("warm",forType:.string)
    state.pollClipboard(from:board,activeApplication:application)
    board.clearContents();board.setString("settled",forType:.string)
    state.pollClipboard(from:board,activeApplication:application);try settle();reads=0;pids=[]
    func cpu()->Double {var usage=rusage();getrusage(RUSAGE_SELF,&usage);return Double(usage.ru_utime.tv_sec+usage.ru_stime.tv_sec)+Double(usage.ru_utime.tv_usec+usage.ru_stime.tv_usec)/1_000_000}
    let began=DispatchTime.now().uptimeNanoseconds,initialCPU=cpu()
    for i in 0..<ticks {try autoreleasepool {
        if active,i%4==0 {board.clearContents();board.setString("Fixture \(i%64)",forType:.string)}
        state.pollClipboard(from:board,activeApplication:application)
        try settle()
    }}
    let elapsed=Double(DispatchTime.now().uptimeNanoseconds-began)/1_000_000_000,usedCPU=cpu()-initialCPU
    return ["ok":true,"schema":1,"mode":active ? "changed-every-fourth-poll":"unchanged","ticks":ticks,"applicationReads":reads,"distinctForegroundPIDs":pids.count,"elapsedSeconds":elapsed,"cpuSeconds":usedCPU,"retainedClips":state.clips.count,"scope":"Tight-loop named-pasteboard polling cost; ordinary timer unchanged; no general clipboard, hotkeys, visible UI, physical energy or battery claim"]
}
#endif
