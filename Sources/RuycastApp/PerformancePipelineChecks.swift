import AppKit
import LauncherCore
import LauncherModels

func checkPerformancePipeline() throws -> [String:Any] {
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
    let gate=ClipboardRenderGate(state:state)
    state.mode="commands";state.query="terminal"
    let settleGate=Date().addingTimeInterval(0.05);while Date()<settleGate {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    let beforeGate=gate.invalidations
    for i in 0..<1000 {state.query="terminal \(i)"}
    let settleTyping=Date().addingTimeInterval(0.05);while Date()<settleTyping {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    try require(state.filteredClips==retained && !state.clipboardSearchBusy && gate.invalidations==beforeGate,"Hidden clipboard redrew or searched while command typing")
    state.clips.removeAll {$0.id==selected}
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
    return ["ok":true,"cleanupFailurePreservesCapture":true,"captureQueueBounded":true,"importDeleteReloadPreserveAcceptedCapture":true,"clearPreventsResurrection":true,"imageRoundTripAndRestart":true,"unicodeFilterParity":true,"rapidQueries":1000,"selectedIdentityPreserved":true,"atomicConfigReloadAndInvalidRecovery":true,"unrelatedConfigReads":readsAfter-before,"desktopEffects":false]
}
