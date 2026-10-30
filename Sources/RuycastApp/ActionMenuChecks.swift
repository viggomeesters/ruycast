import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

/// Dispatches only to our invisible native views; no global input or visible window.
@MainActor func checkActionsMenu(output:String?=nil) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-actions-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    state.config.commands=featureFixture();state.selection=4
    let controller=PaletteController(state:state)
    state.paletteVisible=true
    controller.panel.contentView?.layoutSubtreeIfNeeded()
    let popup=NSWindow(contentRect:NSRect(x:0,y:0,width:256,height:340),styleMask:.borderless,backing:.buffered,defer:false)
    popup.isReleasedWhenClosed=false
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func event(_ key:UInt16,flags:NSEvent.ModifierFlags=[],characters:String="")->NSEvent {NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:popup.windowNumber,context:nil,characters:characters,charactersIgnoringModifiers:characters,isARepeat:false,keyCode:key)!}
    func pump() {for _ in 0..<10 {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}}
    let original=state.selectedCommand!
    try require(state.actionItems.contains(where:{$0.id=="set-alias"}) && state.actionItems.contains(where:{$0.id=="set-hotkey"}),"Separate alias and hotkey actions are missing.")
    try require(controller.panel.performKeyEquivalent(with:event(40,flags:.command,characters:"k")) && state.actionsOpen && state.actionSelection==0,"Command K did not open at first action.")
    let host=NSHostingView(rootView:PaletteActions(state:state).frame(maxWidth:.infinity,maxHeight:.infinity).background(Color(nsColor:.windowBackgroundColor)));host.frame=NSRect(x:0,y:0,width:256,height:340);popup.contentView=host;host.layoutSubtreeIfNeeded();pump()
    guard let input=popup.firstResponder as? ActionMenuKeyInput else {throw LauncherError("Actions native input did not acquire first responder automatically.")}
    try require(ActionMenuKeyInput.contains(in:host),"Actions window was not recognized.")
    input.keyDown(with:event(125));input.keyDown(with:event(125))
    try require(state.actionSelection==2 && state.selection==4 && state.selectedCommand?.id==original.id,"Arrows moved underlying results.")
    input.keyDown(with:event(126))
    try require(state.actionSelection==1,"Up did not move within Actions.")
    for _ in 0..<20 {input.keyDown(with:event(125))}
    try require(state.actionSelection==state.actionItems.count-1,"Down did not clamp to last action.")
    for _ in 0..<20 {input.keyDown(with:event(126))}
    try require(state.actionSelection==0,"Up did not clamp to first action.")
    // Even if the field editor retains input during popover construction, it routes to Actions.
    let search=PaletteSearchField(text:Binding(get:{state.query},set:{state.query=$0}),placeholder:"Search",onMove:state.moveSelection,onSubmit:state.activateSelection,onCancel:state.cancelPaletteInput,onActions:{state.actionsOpen.toggle()})
    let delegate=PaletteSearchField.Coordinator(search)
    _=delegate.control(SearchInput(),textView:NSTextView(),doCommandBy:NSSelectorFromString("moveDown:"))
    try require(state.actionSelection==1 && state.selection==4,"Field-editor fallback moved results.")
    state.actionSelection=state.actionItems.firstIndex(where:{$0.id=="edit"})!
    input.keyDown(with:event(36))
    try require(!state.actionsOpen && state.form?.command.id==original.id && state.form?.isRun==false,"Return did not execute highlighted Edit.")
    state.form=nil;state.actionsOpen=true;input.keyDown(with:event(53));pump()
    try require(!state.actionsOpen && state.paletteVisible && state.query.isEmpty && state.selection==4,"Escape closed palette or changed search state.")
    let responder=controller.panel.firstResponder
    let fieldEditor=(responder as? NSTextView)?.delegate as? SearchInput
    try require(responder is SearchInput || fieldEditor != nil,"Search focus was not restored.")
    state.actionsOpen=true
    try require(input.performKeyEquivalent(with:event(40,flags:.command,characters:"k")) && !state.actionsOpen,"Command K did not close Actions.")
    state.actionsOpen=true;state.actionSelection=state.actionItems.firstIndex(where:{$0.id=="remove"})!;state.activateActionSelection()
    state.query="Finder"
    try require(state.pendingRemoval?.id==original.id,"Removal lost its original command after dismissal.")
    state.pendingRemoval=nil;state.query="";state.selection=4
    state.actionsOpen=true;state.newCommand("snippet")
    try require(state.form?.isNew==true && state.form?.command.kind=="snippet","Mouse/keyboard shared action did not open editor.")
    state.form=nil;state.clips=[Clip(text:"fixture",source:"fixture")];state.mode="clipboard";state.actionsOpen=true
    try require(state.actionItems.prefix(3).map(\.id)==["paste","copy-clip","delete-clip"],"Clipboard action catalog changed.")
    state.actionSelection=2;input.keyDown(with:event(36))
    try require(state.clips.isEmpty && !state.actionsOpen,"Clipboard menu Return did not delete selected fixture item.")
    state.actionsOpen=false;state.mode="commands";pump()
    let restored=controller.panel.firstResponder
    try require(restored is SearchInput || ((restored as? NSTextView)?.delegate is SearchInput),"Mode switch did not restore search focus")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        state.mode="commands";state.selection=4;state.actionsOpen=true
        for dark in [false,true] {
            host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua)
            for selected in [0,2] {
                state.actionSelection=selected;pump();host.layoutSubtreeIfNeeded()
                guard let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("Actions preview unavailable.")};host.cacheDisplay(in:host.bounds,to:bitmap)
                try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("actions-\(dark ? "dark":"light")-\(selected).png"))
            }
        }
    }
    state.paletteVisible=false;state.actionsOpen=false
    try checkItemBindingEditing(output:output)
    return ["ok":true,"commandKOpensAtFirstAction":true,"nativeMenuFirstResponder":true,"arrowsStayInMenu":true,"modeSwitchRestoresSearchFocus":true,"fieldEditorFallback":true,"returnExecutesHighlightedAction":true,"escapeRestoresSearchFocus":true,"commandKTogglesClosed":true,"clipboardActions":true,"removalPayloadPreserved":true,"snippetFooterAccessiblePressAndHotkeySave":true,"directItemAliasAndHotkey":true,"aliasNativeFocusAndEnterSave":true,"recorderAutoFocusAndAccessibleName":true,"recordedPaletteShortcutsNotIntercepted":true,"cancelRepeatClearAndConflictRetry":true,"desktopInputSent":false,"visibleWindowOpened":false]
}

/// Exercise the real hidden panel, field editor, recorder and CLI-backed config writer.
@MainActor private func checkItemBindingEditing(output:String?) throws {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-item-bindings-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    defer {state.endShortcutRecording();state.shutdown()}
    state.config.commands=featureFixture();try Files.save(state.config,to:state.configURL);try state.reload()
    let controller=PaletteController(state:state),panel=controller.panel
    state.paletteVisible=true;state.query="Finder";state.selection=0
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {for _ in 0..<15 {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))};panel.contentView?.layoutSubtreeIfNeeded()}
    func event(_ code:UInt16,_ flags:NSEvent.ModifierFlags=[],repeatKey:Bool=false)->NSEvent {NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:panel.windowNumber,context:nil,characters:"",charactersIgnoringModifiers:"",isARepeat:repeatKey,keyCode:code)!}
    func waitForSave() throws {
        let limit=Date().addingTimeInterval(5)
        while state.busy && Date()<limit {pump()}
        try require(!state.busy && state.message.isEmpty && state.form==nil,"Direct binding save failed: \(state.message)")
        try state.reload();pump()
    }
    func recorder(_ view:NSView)->ShortcutRecordButton? {if let button=view as? ShortcutRecordButton {return button};return view.subviews.lazy.compactMap {recorder($0)}.first}
    func recordButton() throws -> ShortcutRecordButton {
        pump()
        guard let host=panel.contentView,let button=recorder(host) else {throw LauncherError("Direct hotkey editor did not render its native recorder.")}
        try require(button.recording && state.recordingShortcut && panel.firstResponder===button,"Recorder did not start and acquire focus automatically.")
        try require(button.accessibilityLabel()=="Press shortcut","Recorder accessible name is missing.")
        return button
    }
    let command=state.selectedCommand!,id=command.id
    let before=try Data(contentsOf:state.configURL)
    try require(!panel.performKeyEquivalent(with:event(43,.command)),"Plain Command-comma was captured by an item action.")
    try require(panel.performKeyEquivalent(with:event(43,[.command,.option])) && state.form?.bindingField == .alias,"Direct alias shortcut did not target selected command.")
    pump()
    guard let editor=panel.firstResponder as? NSTextView else {throw LauncherError("Alias field did not automatically acquire native editing focus.")}
    try require(editor.string==command.aliases.joined(separator:", "),"Alias input did not preserve existing aliases.")
    editor.selectAll(nil);editor.insertText("finder-a, finder-b",replacementRange:editor.selectedRange())
    editor.doCommand(by:NSSelectorFromString("insertNewline:"));try waitForSave()
    let aliasSaved=state.config.commands.first {$0.id==id}!
    try require(aliasSaved.aliases==["finder-a","finder-b"] && aliasSaved.hotkey==command.hotkey && aliasSaved.target==command.target,"Alias Enter save changed other fields or lost multiple aliases.")
    state.query="finder-a";state.selection=0
    try require(panel.performKeyEquivalent(with:event(43,[.command,.shift])) && state.form?.bindingField == .hotkey,"Direct hotkey shortcut did not open recording.")
    var button=try recordButton()
    let afterAliases=try Data(contentsOf:state.configURL)
    _=panel.performKeyEquivalent(with:event(43,[.command,.shift],repeatKey:true))
    try require(button.recording && !state.busy && (try Data(contentsOf:state.configURL))==afterAliases,"Held opener was captured or saved.")
    _=panel.performKeyEquivalent(with:event(53));pump()
    try require(state.form==nil && !state.recordingShortcut && (try Data(contentsOf:state.configURL))==afterAliases,"Escape wrote a hotkey or left recording active.")
    // Menu routing uses the selected command; it must not open full Settings.
    state.actionsOpen=true
    try require(state.handleActionMenuKey(event(43,[.command,.shift])),"Hotkey chord did not route from Actions.")
    button=try recordButton()
    _=panel.performKeyEquivalent(with:event(40,.command));try waitForSave()
    try require(state.config.commands.first {$0.id==id}?.hotkey=="cmd+k" && !state.actionsOpen && !state.recordingShortcut,"Panel intercepted Command-K while recording.")
    // The opener itself and Command-Return are valid recorder payloads too.
    for (code,flags,value) in [(UInt16(43),NSEvent.ModifierFlags([.command,.shift]),"shift+cmd+,"),(UInt16(36),NSEvent.ModifierFlags.command,"cmd+return")] {
        state.query="finder-a";state.selection=0
        state.performMenuAction("set-hotkey");_=try recordButton()
        _=panel.performKeyEquivalent(with:event(code,flags));try waitForSave()
        try require(state.config.commands.first {$0.id==id}?.hotkey==value,"Palette intercepted a recorded editor shortcut.")
    }
    // Duplicate combinations stay visible and retryable; no config mutation.
    state.query="finder-a";state.selection=0;state.performMenuAction("set-hotkey");button=try recordButton()
    let beforeConflict=try Data(contentsOf:state.configURL)
    _=panel.performKeyEquivalent(with:event(49,[.control,.option]));pump()
    try require(state.form != nil && !state.message.isEmpty && !state.recordingShortcut && (try Data(contentsOf:state.configURL))==beforeConflict,"Conflicting shortcut was saved or its error was hidden.")
    button.beginRecording();_=panel.performKeyEquivalent(with:event(51));try waitForSave()
    try require(state.config.commands.first {$0.id==id}?.hotkey==nil,"Delete did not clear the selected command's shortcut.")
    // Escape from alias input leaves its pending text and all bindings uncommitted.
    state.query="finder-a";state.selection=0;state.performMenuAction("set-alias");pump()
    guard let cancelledAlias=panel.firstResponder as? NSTextView else {throw LauncherError("Alias retry did not focus")}
    let beforeAliasCancel=try Data(contentsOf:state.configURL)
    cancelledAlias.selectAll(nil);cancelledAlias.insertText("discard-this",replacementRange:cancelledAlias.selectedRange())
    cancelledAlias.doCommand(by:NSSelectorFromString("cancelOperation:"));pump()
    try require(state.form==nil && (try Data(contentsOf:state.configURL))==beforeAliasCancel,"Alias Escape wrote pending text.")
    // Closing the palette must end both active and queued automatic recording.
    for pending in [false,true] {
        state.paletteVisible=true;state.performMenuAction("set-hotkey")
        var queued:ShortcutRecordButton?
        var starts=0
        if pending {
            let pendingForm=state.form!.id
            let control=ShortcutRecordButton(title:"",target:nil,action:nil)
            control.recordOnAppear=true;control.canStartAutomatically={state.paletteVisible && state.form?.id==pendingForm}
            control.start={starts+=1;state.beginShortcutRecording()};control.stop={state.endShortcutRecording()}
            panel.contentView!.addSubview(control);queued=control
            try require(control.window===panel && !control.recording && starts==0,"Queued native recorder control did not attach before the pump.")
        } else {
            button=try recordButton()
            // Control: the former orderOut-only sequence leaves the attached recorder active.
            panel.orderOut(nil);state.paletteVisible=false;pump()
            try require(button.recording && state.recordingShortcut,"Legacy orderOut control did not reproduce retained recording.")
        }
        let beforeHide=try Data(contentsOf:state.configURL)
        controller.hide();pump()
        let attached=panel.contentView.flatMap {recorder($0)}
        try require(!state.recordingShortcut && attached?.recording != true && queued?.recording != true && starts==0 && (try Data(contentsOf:state.configURL))==beforeHide,"Palette hide leaked active or queued recording.")
        queued?.removeFromSuperview()
    }
    state.paletteVisible=true
    // The visible snippet footer action uses the existing recorder and writer.
    state.mode="snippets";state.query="Greeting";state.selection=0;pump()
    guard let snippet=state.selectedCommand,snippet.kind=="snippet" else {throw LauncherError("Snippet fixture was not selected.")}
    func snippetHotkeyButton(_ view:NSView)->NSButton? {
        if let button=view as? NSButton,button.title=="Set Hotkey…" {return button}
        return view.subviews.lazy.compactMap {snippetHotkeyButton($0)}.first
    }
    func pressSnippetHotkey() throws {
        pump()
        guard let view=panel.contentView,let button=snippetHotkeyButton(view) else {throw LauncherError("Visible snippet Set Hotkey button is missing.")}
        try require(button.accessibilityLabel()=="Set Hotkey…" && button.isEnabled,"Snippet hotkey action is not accessible or enabled.")
        button.performClick(nil);pump()
    }
    let beforeSnippet=try Data(contentsOf:state.configURL)
    try pressSnippetHotkey()
    _=try recordButton()
    try require(state.form?.command.id==snippet.id && state.form?.bindingField == .hotkey,"Snippet footer opened the wrong editor.")
    _=panel.performKeyEquivalent(with:event(53));pump()
    try require(state.form==nil && (try Data(contentsOf:state.configURL))==beforeSnippet,"Snippet hotkey cancellation changed configuration.")
    try pressSnippetHotkey()
    _=try recordButton();_=panel.performKeyEquivalent(with:event(101,[.control,.option]));try waitForSave()
    let boundSnippet=state.config.commands.first {$0.id==snippet.id}!
    try require(boundSnippet.hotkey=="ctrl+alt+f9" && boundSnippet.target==snippet.target && boundSnippet.keyword==snippet.keyword && boundSnippet.aliases==snippet.aliases,"Snippet hotkey save changed snippet content or invocation fields.")
    state.mode="commands";state.query="finder-a";state.selection=0;pump()
    // A newly discovered app is materialized with its target intact, using the same writer.
    let discovered=Command(id:"app.fixture-binding",title:"Discovered Fixture App",kind:"app",target:"org.example.fixture-binding")
    state.applications=[discovered];state.query=discovered.title;state.selection=0
    state.performMenuAction("set-hotkey");_=try recordButton()
    try require(state.form?.isNew==true,"Discovered app was not marked new.")
    _=panel.performKeyEquivalent(with:event(20,[.command,.option]));try waitForSave()
    try require(state.config.commands.first {$0.id==discovered.id}?.target==discovered.target && state.config.commands.first {$0.id==discovered.id}?.hotkey=="alt+cmd+3","Discovered app binding lost its identity or target.")
    // A pre-existing editor and unavailable selections cannot be replaced by item chords.
    state.query="finder-a";state.selection=0;state.performMenuAction("edit")
    let formID=state.form?.id
    try require(!state.handleItemBindingKey(event(43,[.command,.option])) && state.form?.id==formID,"Item shortcut overwrote an existing form.")
    state.form=nil;state.query="no-matches-zz"
    try require(!state.handleItemBindingKey(event(43,[.command,.shift])),"Item shortcut accepted an empty selection.")
    state.query="finder-a";state.selection=0;state.pendingRemoval=command
    try require(!state.handleItemBindingKey(event(43,[.command,.shift])),"Item shortcut replaced an active confirmation.")
    state.pendingRemoval=nil;state.busy=true
    try require(!state.handleItemBindingKey(event(43,[.command,.option])),"Item shortcut replaced an in-flight save.")
    state.busy=false
    state.mode="clipboard";state.clips=[Clip(text:"fixture",source:"fixture")];state.selection=0
    try require(!state.handleItemBindingKey(event(43,[.command,.option])),"Clipboard item accepted command bindings.")
    try require(!panel.isVisible && before != (try Data(contentsOf:state.configURL)),"Binding checks opened a visible window or never exercised persistence.")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        state.mode="commands";state.query="finder-a";state.selection=0
        for dark in [false,true] {for width in [600,760] {
            state.config.theme=dark ? "dark":"light"
            for field in ["set-alias","set-hotkey"] {
                state.form=nil;state.performMenuAction(field);pump()
                guard let view=panel.contentView else {throw LauncherError("No binding preview")}
                panel.setContentSize(NSSize(width:width,height:490));view.layoutSubtreeIfNeeded();pump()
                guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds),let data:Data = {view.cacheDisplay(in:view.bounds,to:bitmap);return bitmap.representation(using:.png,properties:[:])}() else {throw LauncherError("No binding pixels")}
                try data.write(to:directory.appendingPathComponent("\(field)-\(dark ? "dark":"light")-\(width).png"))
            }
        }}
    }
    state.form=nil;state.paletteVisible=false;pump()
    try require(!state.recordingShortcut,"Editor teardown left recording active.")
}
