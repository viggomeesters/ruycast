import AppKit
import SwiftUI
import Vision
import LauncherCore
import LauncherModels

/// Isolated capture and quick-settings flows. Never captures into the user's vault.
@MainActor func checkFeedback(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-feedback-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    var config=state.config;config.commands=featureFixture()
    for index in config.commands.indices where config.commands[index].id.hasPrefix("capture.") {config.commands[index].arguments=["%s",#"{"ok":true}"#]}
    try Files.save(config,to:state.configURL);try state.reload()
    let creationPasteboard=NSPasteboard(name:NSPasteboard.Name("rc-quicklink-"+UUID().uuidString))
    defer {creationPasteboard.clearContents()}
    var presented=0
    state.showPalette={mode in presented+=1;state.paletteVisible=true;state.form=nil;state.mode=mode}
    for kind in ["url","snippet"] {
        state.paletteVisible=false;state.form=nil
        state.newCommand(kind,pasteboard:creationPasteboard)
        try require(state.paletteVisible && state.form?.isNew==true && state.form?.command.kind==kind,"New command from closed launcher remained invisible.")
        let count=presented
        state.newCommand(kind,pasteboard:creationPasteboard)
        try require(presented==count && state.form?.command.kind==kind,"Creation re-presented an already open launcher.")
    }
    state.showPalette=nil;state.form=nil;state.paletteVisible=false
    let server=ControlServer(root:root);try server.start {req,reply in state.handle(req,sink:reply)};defer {server.stop()}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {let end=Date().addingTimeInterval(0.15);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait() throws {let end=Date().addingTimeInterval(8);while state.busy,Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))};try require(!state.busy && state.message.isEmpty,"Fixture save failed: \(state.message)")}
    func input(_ view:NSView)->CaptureInput? {if let match=view as? CaptureInput {return match};return view.subviews.lazy.compactMap {input($0)}.first}
    let window=NSWindow(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:.borderless,backing:.buffered,defer:false);window.isReleasedWhenClosed=false
    defer {window.close()}
    let host=NSHostingView(rootView:Palette(state:state));window.contentView=host;host.frame=NSRect(x:0,y:0,width:760,height:490)
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    func capture(_ name:String,view:NSView?=nil) throws {
        pump();let view=view ?? host;view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("No feedback preview")}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent(name+".png"))
        if let image=bitmap.cgImage {
            let request=VNRecognizeTextRequest();request.recognitionLevel = .accurate
            try VNImageRequestHandler(cgImage:image).perform([request])
            let text=(request.results ?? []).compactMap {$0.topCandidates(1).first?.string}.joined(separator:"\n")
            try text.write(to:directory.appendingPathComponent(name+"-copy.txt"),atomically:true,encoding:.utf8)
        }
    }
    // Only a named synthetic pasteboard is read; never the user's clipboard.
    let copied="https://example.test/copied?q=1#part",raw=" \n"+copied+" \n"
    creationPasteboard.declareTypes([.string],owner:nil);creationPasteboard.setString(raw,forType:.string)
    let clipboardRevision=creationPasteboard.changeCount,configBeforeCreation=try Data(contentsOf:state.configURL)
    state.newCommand("url",pasteboard:creationPasteboard)
    try require(state.form?.command.target==copied,"New Quicklink did not prefill the current clipboard.")
    let settingsForm=state.newCommandForm("url",pasteboard:creationPasteboard)
    try require(settingsForm.command.target==copied && settingsForm.isNew && !settingsForm.isRun,"Settings creation did not use the same clipboard prefill.")
    try require(state.newCommandForm("snippet",pasteboard:creationPasteboard).command.target.isEmpty && state.newCommandForm("script",pasteboard:creationPasteboard).command.target=="/usr/bin/true","Clipboard replaced non-quicklink content.")
    let oldTheme=state.config.theme
    for theme in ["light","dark"] {
        state.config.theme=theme
        for width in [760,600] {window.setContentSize(NSSize(width:width,height:490));host.frame=NSRect(x:0,y:0,width:width,height:490);try capture("quicklink-prefill-\(theme)-\(width)")}
        let navigation=ManagementNavigation();navigation.select(.quicklinks);navigation.form=settingsForm
        let settings=NSHostingView(rootView:ManagementView(state:state,navigation:navigation));settings.frame=NSRect(x:0,y:0,width:980,height:680)
        try capture("quicklink-settings-\(theme)",view:settings)
    }
    state.config.theme=oldTheme;window.setContentSize(NSSize(width:760,height:490));host.frame=NSRect(x:0,y:0,width:760,height:490)
    try require(creationPasteboard.changeCount==clipboardRevision && creationPasteboard.string(forType:.string)==raw && (try Data(contentsOf:state.configURL))==configBeforeCreation,"Prefill mutated clipboard or saved without confirmation.")
    var quicklinkDraft=state.form!.command;quicklinkDraft.title="Fixture Quicklink";quicklinkDraft.target="https://example.test/edited"
    state.form?.command=quicklinkDraft;creationPasteboard.setString("https://example.test/later",forType:.string);pump()
    try require(state.form?.command.target==quicklinkDraft.target,"A later clipboard change replaced user edits.")
    state.form=nil
    try require((try Data(contentsOf:state.configURL))==configBeforeCreation,"Cancelling a new quicklink saved it.")
    state.saveCommand(quicklinkDraft,isNew:true);try wait();try state.reload()
    try require(state.config.commands.first {$0.id==quicklinkDraft.id}?.target==quicklinkDraft.target,"Explicit quicklink save lost the edited target.")
    creationPasteboard.clearContents()
    try require(state.newCommandForm("url",pasteboard:creationPasteboard).command.target=="https://","Empty clipboard lost the existing URL fallback.")
    let binaryType=NSPasteboard.PasteboardType("org.example.quicklink-fixture")
    creationPasteboard.declareTypes([binaryType],owner:nil);creationPasteboard.setData(Data([0,1,2]),forType:binaryType)
    try require(state.newCommandForm("url",pasteboard:creationPasteboard).command.target=="https://","Non-text clipboard became an invalid URL.")
    state.form=nil;state.paletteVisible=false
    for id in ["capture.quick-note","capture.quick-task"] {
        state.begin(state.config.commands.first {$0.id==id}!);try require(state.inlineCommand?.id==id && state.form==nil,"Capture did not start inline");state.expandInlineInput();pump();host.layoutSubtreeIfNeeded();pump()
        guard let editor=input(host) else {throw LauncherError("Quick capture did not render multiline editor")}
        try require(window.firstResponder===editor,"Quick capture editor did not acquire native focus")
        editor.insertText("First line",replacementRange:editor.selectedRange());editor.keyDown(with:NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:.shift,timestamp:0,windowNumber:window.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!);editor.insertText("Second line Ω",replacementRange:editor.selectedRange())
        editor.delegate?.textDidChange?(Notification(name:NSText.didChangeNotification,object:editor))
        try require(editor.string=="First line\nSecond line Ω" && state.form?.isRun==true && !state.busy,"Shift-Return did not add a new line")
        for dark in [true,false] {host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);try capture("\(id)-\(dark ? "dark":"light")")}
        // Intercept the editor's production submit callback, avoiding any provider mutation.
        let submit=editor.onSubmit;var submits=0;editor.onSubmit={submits+=1}
        func key(_ flags:NSEvent.ModifierFlags) {editor.keyDown(with:NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:window.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!)}
        key([]);key(.command);try require(submits==2,"Enter/Command-Enter did not submit once")
        editor.setMarkedText("語",selectedRange:NSRange(location:1,length:0),replacementRange:editor.selectedRange());key([])
        try require(submits==2,"IME confirmation submitted capture")
        editor.unmarkText();editor.onSubmit=submit
        key([]);key([]);try wait()
        try require(state.counts[id]==1 && state.form==nil,"Enter did not complete once through the real runner or busy submission duplicated it")

        state.form=nil;pump()
    }
    state.query="Finder";state.selection=0
    let command=state.selectedCommand!
    try require(state.actionItems.contains(where:{$0.id=="set-alias"}) && state.actionItems.contains(where:{$0.id=="set-hotkey"}) && state.actionItems.contains(where:{$0.id=="disable"}),"Quick settings actions are missing")
    try require(!state.actionItems.contains(where:{$0.id=="create-link" || $0.id=="create-snippet"}),"Creation remains attached to selected command")
    state.actionsOpen=true;state.performMenuAction("set-alias")
    try require(state.form?.bindingsOnly==true && state.form?.command.id==command.id,"Quick settings lost selected command")
    host.appearance=NSAppearance(named:.darkAqua);try capture("quick-settings-dark")
    var edited=state.form!.command;edited.aliases=["finder-custom"];edited.hotkey="ctrl+alt+f8";var external=state.config
    let commandIndex=external.commands.firstIndex {$0.id==command.id}!
    external.commands[commandIndex].title="Finder External";external.commands[commandIndex].target="com.apple.Safari";external.commands[commandIndex].arguments=["external argument"]
    try Files.save(external,to:state.configURL);try state.reload()
    state.saveBindings(edited,original:command);try wait()
    let preserved=state.config.commands.first {$0.id==command.id}!
    try require(preserved.title=="Finder External" && preserved.target=="com.apple.Safari" && preserved.arguments==["external argument"],"Quick settings overwrote external definition changes")
    try require(state.config.resolve("finder-custom")?.hotkey=="ctrl+alt+f8","Quick settings alias/hotkey save failed")
    state.query="finder-custom";state.selection=0;state.actionsOpen=true;state.performMenuAction("disable");try wait()
    try require(state.config.commands.first {$0.id==command.id}?.enabled==false && state.searchCommands("finder-custom").isEmpty,"Disable did not persist or remained searchable")
    state.query="nothing-matches-zz"
    for kind in ["url","snippet"] {state.newCommand(kind,pasteboard:creationPasteboard);try require(state.form?.isNew==true && state.form?.command.id != command.id && state.form?.command.target != command.target,"Creation inherited selected command");state.form=nil}
    state.query="";try capture("global-create-dark")
    // Process actions use fixture effects, never a real termination or app launch.
    let services=try checkSystemServices()
    try require(services["ok"] as? Bool==true,"Process tools failed platform boundary checks")
    try require(!window.isVisible,"Feedback test opened a visible window")
    return ["ok":true,"quickNoteAndTaskMultiline":true,"nativeCaptureFocus":true,"enterSubmits":true,"shiftEnterAddsLine":true,"commandEnterSubmits":true,"imeCommitDoesNotSubmit":true,"enterCompletesThroughRunnerOnce":true,"quickAliasHotkeySave":true,"bindingSavePreservesExternalDefinition":true,"quickDisable":true,"clipboardQuicklinkPrefill":true,"clipboardUnchangedAndSaveExplicit":true,"settingsCreationUsesSamePrefill":true,"emptyAndNonTextClipboardFallback":true,"creationIndependentOfSelection":true,"processToolsVerifiedWithoutDesktopEffects":true,"visibleWindowOpened":false,"desktopInputSent":false]
}
