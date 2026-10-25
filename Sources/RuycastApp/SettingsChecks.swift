import AppKit
import SwiftUI
import Vision
import CryptoKit
import LauncherCore

/// Production controls in an invisible window, with a disposable CLI-backed profile.
func checkSettings(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-settings-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    var config=state.config
    config.commands=featureFixture()+Configuration.layouts.map {Command(id:"window."+$0,title:$0.replacingOccurrences(of:"-",with:" ").capitalized,kind:"window",target:$0)}
    config.commands[0].hotkey="ctrl+alt+f8"
    try Files.save(config,to:state.configURL);try state.reload()
    let server=ControlServer(root:root);try server.start {request,reply in state.handle(request,reply:reply)};defer {server.stop()}
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    func wait() throws {
        let end=Date().addingTimeInterval(8)
        while state.busy,Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
        try require(!state.busy && state.message.isEmpty,"Settings save failed: \(state.message)")
    }
    let controller=ManagementController(state:state),nav=controller.navigation,window=controller.window
    defer {window.close()}
    try require(nav.tab == .commands && !window.isVisible,"Settings must start in Commands without opening a window.")
    let button=ShortcutRecordButton(title:"",target:nil,action:nil)
    let recordingWindow=NSWindow(contentRect:NSRect(x:0,y:0,width:200,height:60),styleMask:.borderless,backing:.buffered,defer:false)
    recordingWindow.isReleasedWhenClosed=false;recordingWindow.contentView=button;defer {recordingWindow.close()}
    button.configure(state.config.commands[0].hotkey)
    button.start={state.beginShortcutRecording()};button.stop={state.endShortcutRecording()}
    button.save={value in var command=state.config.commands[0];command.hotkey=value;state.saveCommand(command)}
    func event(_ key:UInt16,_ flags:NSEvent.ModifierFlags=[],type:NSEvent.EventType = .keyDown)->NSEvent {NSEvent.keyEvent(with:type,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:recordingWindow.windowNumber,context:nil,characters:"",charactersIgnoringModifiers:"",isARepeat:false,keyCode:key)!}
    button.beginRecording();try require(button.recording && state.recordingShortcut,"Recorder did not acquire native focus.")
    button.keyDown(with:event(123,[.control,.option,.shift,.command]));try wait()
    try require(!state.recordingShortcut && state.config.commands[0].hotkey=="ctrl+alt+shift+cmd+left","Hyper arrow did not save through CLI.")
    let previous=state.config.commands[0]
    button.configure(previous.hotkey);button.beginRecording();button.keyDown(with:event(53))
    try require(!button.recording && !state.recordingShortcut && state.config.commands[0]==previous,"Escape changed binding or left recording active.")
    button.beginRecording();_=recordingWindow.makeFirstResponder(nil)
    try require(!button.recording && !state.recordingShortcut,"Focus loss did not end recording.")
    button.beginRecording();button.flagsChanged(with:event(59,.control,type:.flagsChanged));button.flagsChanged(with:event(59,[],type:.flagsChanged));try wait()
    try require(state.config.commands[0].hotkey=="ctrl+left-control","Left Control tap recording failed.")
    button.beginRecording();button.keyDown(with:event(51));try wait()
    try require(state.config.commands[0].hotkey==nil,"Delete did not clear recorded shortcut.")
    button.beginRecording();try require(button.performKeyEquivalent(with:event(40,.command)),"Command key equivalent escaped recorder.");try wait()
    try require(state.config.commands[0].hotkey=="cmd+k","Command shortcut did not persist.")
    // Drive the production recorder and filter without posting global input.
    for (mask,binding) in [(CommandShortcutFilter.rightMask,"right-cmd+k"),(CommandShortcutFilter.leftMask,"left-cmd+k")] {
        button.beginRecording()
        button.keyDown(with:event(40,NSEvent.ModifierFlags(rawValue:NSEvent.ModifierFlags.command.rawValue | UInt(mask))))
        try wait()
        try require(state.config.commands[0].hotkey==binding,"Command side was lost during save/reload.")
    }
    button.configure("right-cmd+k");button.useEitherCommand();try wait()
    try require(state.config.commands[0].hotkey=="cmd+k","Either Command action did not preserve a generic shortcut.")
    var filter=CommandShortcutFilter()
    filter.bindings=try [("right-cmd+k","right"),("left-cmd+k","left"),("shift+right-cmd+k","shifted")].map {CommandShortcutFilter.Binding(shortcut:try Shortcut($0.0),action:$0.1)}
    let right=CGEventFlags(rawValue:CGEventFlags.maskCommand.rawValue | CommandShortcutFilter.rightMask)
    let left=CGEventFlags(rawValue:CGEventFlags.maskCommand.rawValue | CommandShortcutFilter.leftMask)
    try require(filter.handle(.keyDown,key:40,flags:right).action=="right","Right Command did not route.")
    try require(filter.handle(.keyDown,key:40,flags:right,repeatKey:true).action==nil,"Repeat launched the app again.")
    try require(filter.handle(.keyUp,key:40,flags:[]).consumed,"Key release leaked after Command release.")
    try require(filter.handle(.keyDown,key:40,flags:left).action=="left","Left Command did not route separately.")
    _=filter.handle(.keyUp,key:40,flags:left)
    try require(!filter.handle(.keyDown,key:40,flags:.maskCommand).consumed,"Generic Command was captured.")
    try require(!filter.handle(.keyDown,key:40,flags:right.union(left)).consumed,"Both Command keys matched a single-side shortcut.")
    try require(!filter.handle(.keyDown,key:41,flags:right).consumed,"Unbound key was swallowed.")
    try require(!filter.handle(.keyDown,key:40,flags:right.union(.maskAlternate)).consumed,"Extra modifier matched.")
    try require(filter.handle(.keyDown,key:40,flags:right.union(.maskShift)).action=="shifted","Additional modifier binding failed.")
    filter.reset()
    try require(!filter.handle(.keyUp,key:40,flags:[]).consumed,"Stopped filter retained key state.")
    filter.bindings=[]
    try require(!filter.handle(.keyDown,key:40,flags:right).consumed,"Disabled bindings remained active.")
    var changed=state.config.commands[0];changed.aliases=["myfinder","files-home"];changed.enabled=false
    state.saveCommand(changed);try wait()
    try require(!state.searchCommands("myfinder").contains(where:{$0.id==changed.id}) && nav.commands(in:state.config).contains(changed),"Disabled command disappeared from Settings or remained in launcher.")
    var reply:[String:Any]=[:];state.execute(changed.id,dryRun:true) {reply=$0}
    try require(reply["ok"] as? Bool==false && (reply["error"] as? String ?? "").contains("disabled"),"Disabled command remained executable or hid its disabled state.")
    state.applications=[Command(id:"discovered.com.apple.finder",title:"Finder",kind:"app",target:"com.apple.finder")]
    try require(!state.searchCommands("Finder").contains(where:{$0.target=="com.apple.finder"}),"App discovery reintroduced disabled app.")
    state.execute("discovered.com.apple.finder",dryRun:true) {reply=$0}
    try require(reply["ok"] as? Bool==false,"Discovery ID bypassed disabled app guard.")
    var second=state.config.commands[1];second.hotkey=changed.hotkey;state.saveCommand(second);try wait()
    changed.enabled=true;state.message="";state.saveCommand(changed)
    try require(!state.busy && state.message.contains("Duplicate hotkey"),"Re-enabling conflicting shortcut was accepted.")
    state.message="";second.hotkey=nil;state.saveCommand(second);try wait()
    state.saveCommand(changed);try wait()
    try require(state.config.resolve("myfinder")?.id==changed.id,"Re-enabling did not preserve aliases.")
    var snippet=state.config.commands.first {$0.kind=="snippet"}!
    snippet.enabled=false;state.saveCommand(snippet);try wait()
    var matcher=SnippetMatcher();try require(matcher.type(snippet.keyword!,commands:state.config.commands)==nil,"Disabled snippet expanded.")
    state.query="obsidian";nav.query="myfinder"
    try require(nav.commands(in:state.config).map(\.id)==[changed.id] && state.query=="obsidian","Management search is stale or affects launcher search.")
    nav.select(.snippets);try require(nav.commands(in:state.config).map(\.id)==[snippet.id],"Snippet tab missed disabled snippet.")
    state.setSetting("clipboard.retentionDays","14");try wait()
    try require(state.config.clipboard.retentionDays==14,"Clipboard preferences failed to reload.")
    // Rejection must not commit pruning from a candidate that never activates.
    state.clips=(0..<4).map {Clip(text:"history \($0)",source:"fixture")}
    let historyURL=root.appendingPathComponent("clipboard.json")
    try Files.save(state.clips,to:historyURL)
    let beforeHistory=try Data(contentsOf:historyURL),beforeConfig=state.config,beforeBytes=try Data(contentsOf:state.configURL)
    var rejected=beforeConfig;rejected.clipboard.limit=1
    try Files.save(rejected,to:state.configURL)
    var didReject=false
    do {try state.reload(registerHotkeys:{_ in throw LauncherError("fixture hotkey conflict")})} catch {didReject=true}
    try require(didReject && state.clips.count==4 && state.config==beforeConfig,"Rejected activation changed in-memory history/configuration.")
    let afterHistory=try Data(contentsOf:historyURL)
    try require(afterHistory==beforeHistory,"Rejected activation permanently pruned history.")
    // A clipboard write failure after registration must restore the old binding set.
    try FileManager.default.removeItem(at:historyURL);try FileManager.default.createDirectory(at:historyURL,withIntermediateDirectories:true)
    var registrations:[Configuration]=[]
    do {try state.reload(registerHotkeys:{registrations.append($0)})} catch {}
    try require(registrations==[rejected,beforeConfig] && state.config==beforeConfig,"Clipboard write failure did not restore old shortcuts.")
    try FileManager.default.removeItem(at:historyURL);try beforeHistory.write(to:historyURL);try beforeBytes.write(to:state.configURL);try state.reload()
    // Retention applies to owned image files even when recording is disabled/paused.
    let assets=root.appendingPathComponent("clipboard-assets")
    try FileManager.default.createDirectory(at:assets,withIntermediateDirectories:true)
    let assetName=UUID().uuidString+".png",assetURL=assets.appendingPathComponent(assetName)
    let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:1,pixelsHigh:1,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:4,bitsPerPixel:32)!
    bitmap.setColor(NSColor.orange,atX:0,y:0)
    try bitmap.representation(using:.png,properties:[:])!.write(to:assetURL)
    var imageClip=Clip(text:"Image",source:"fixture");imageClip.contentType="image";imageClip.dataFile="clipboard-assets/"+assetName
    state.clips=[Clip(text:"Keep",source:"fixture"),imageClip];try Files.save(state.clips,to:historyURL)
    var imageConfig=state.config;imageConfig.clipboard.limit=1;imageConfig.clipboard.enabled=false
    try Files.save(imageConfig,to:state.configURL)
    do {try state.reload(registerHotkeys:{_ in throw LauncherError("image retention rejection")})} catch {}
    try require(FileManager.default.fileExists(atPath:assetURL.path) && state.clips.count==2,"Rejected retention edit deleted image asset.")
    state.paused=true;try state.reload()
    try require(!FileManager.default.fileExists(atPath:assetURL.path) && state.clips.count==1,"Committed retention edit left removed image asset.")
    // Asset cleanup errors are visible/retriable after activation, never a rollback
    // after deleting some of the old assets.
    try FileManager.default.removeItem(at:assets);try Data("unavailable folder".utf8).write(to:assets)
    try state.reload()
    try require(state.clipboardCleanupError != nil && state.config.clipboard.limit==1,"Cleanup failure was hidden or rolled back committed configuration.")
    try FileManager.default.removeItem(at:assets);try state.reload()
    try require(state.clipboardCleanupError==nil,"Clipboard cleanup did not recover after retry.")
    // An editor can replace the file during platform registration. The runtime
    // must retain the digest of the exact candidate bytes, not those later bytes.
    let snapshotBytes=try Data(contentsOf:state.configURL)
    var externalConfig=state.config
    externalConfig.commands.append(Command(id:"external.link",title:"External Edit",kind:"url",target:"https://example.com/external"))
    let externalBytes=try Files.encode(externalConfig)
    try state.reload(registerHotkeys:{_ in try externalBytes.write(to:state.configURL,options:.atomic)})
    let snapshotConfig=try Files.config(data:snapshotBytes)
    let expectedDigest=SHA256.hash(data:snapshotBytes).map {String(format:"%02x",$0)}.joined()
    try require(state.config==snapshotConfig && state.configDigestForSave==expectedDigest,"Runtime digest was not based on its exact snapshot bytes.")
    var snapshotCommand=state.config.commands[0];snapshotCommand.title="Stale Save"
    state.saveCommand(snapshotCommand)
    let saveDeadline=Date().addingTimeInterval(8)
    while state.busy,Date()<saveDeadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    let keptExternal=try Data(contentsOf:state.configURL)
    try require(!state.busy && state.message.contains("changed") && keptExternal==externalBytes,"Stale runtime snapshot overwrote a concurrent editor.")
    state.message="";try state.reload()
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    guard let host=window.contentView else {throw LauncherError("Missing Settings host")}
    var renders=0
    for dark in [true,false] {
        window.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);host.appearance=window.appearance
        for tab in SettingsTab.allCases {
            nav.select(tab)
            window.setContentSize(NSSize(width:980,height:680))
            try capture("settings-\(dark ? "dark":"light")-\(tab.rawValue.lowercased())",required:tab == .commands ? ["Name","Alias","Shortcut","Enabled","Finder"]:[tab.rawValue])
        }
        nav.select(.commands);window.setContentSize(NSSize(width:680,height:480))
        try capture("settings-\(dark ? "dark":"light")-small",required:["Alias","Shortcut","Enabled"])
    }
    func capture(_ name:String,required:[String]) throws {
        let end=Date().addingTimeInterval(0.12)
        while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
        host.layoutSubtreeIfNeeded()
        guard let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("Could not render Settings")};host.cacheDisplay(in:host.bounds,to:bitmap)
        guard let image=bitmap.cgImage,let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Settings image missing")}
        try data.write(to:directory.appendingPathComponent(name+".png"))
        let request=VNRecognizeTextRequest();request.recognitionLevel = .accurate;try VNImageRequestHandler(cgImage:image).perform([request])
        let text=(request.results ?? []).compactMap {$0.topCandidates(1).first?.string}.joined(separator:" ")
        try require(required.allSatisfy {text.contains($0)},"Missing Settings labels in \(name): \(text)")
        renders+=1
    }
    try require(!window.isVisible && !recordingWindow.isVisible,"Test opened a visible window.")
    return ["ok":true,"sevenTabs":true,"defaultCommands":true,"recorderHyperArrows":true,"recorderModifierTap":true,"recorderClearCancelFocusLoss":true,"commandKeyEquivalentCaptured":true,"settingsSearchIndependent":true,"disabledSearchExecutionAndSnippets":true,"discoveryCannotBypassDisabledApp":true,"aliasesPreservedOnReenable":true,"reenableHotkeyConflictRejected":true,"cliBackedSaves":true,"runtimeSnapshotDigestRejectsConcurrentEdit":true,"rejectedReloadPreservesClipboard":true,"failedClipboardWriteRestoresHotkeys":true,"retentionPrunesOwnedImageAssets":true,"assetCleanupFailureVisibleAndRetriable":true,"lightDarkRenders":renders,"minimumWidth":680,"visibleWindowOpened":false,"desktopInputSent":false]
}
