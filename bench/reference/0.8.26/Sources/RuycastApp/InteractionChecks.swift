import AppKit
import SwiftUI
import LauncherCore
import LauncherModels
import Vision

func featureFixture()->[Command] {
    [Command(id:"app.finder",title:"Finder",kind:"app",target:"com.apple.finder",aliases:["finder"]),
     Command(id:"app.obsidian",title:"Obsidian",kind:"app",target:"md.obsidian",aliases:["obsidian"]),
     Command(id:"capture.quick-note",title:"Quick Note",kind:"script",target:"/usr/bin/printf",arguments:["%s","{input}"],inputPrompt:"Note"),
     Command(id:"capture.quick-task",title:"Quick Task",kind:"script",target:"/usr/bin/printf",arguments:["%s","{input}"],inputPrompt:"Task"),
     Command(id:"link.search",title:"Search Google Maps",kind:"url",target:"https://maps.google.com/?q={query}",aliases:["gmaps"]),
     Command(id:"snippet.greeting",title:"Greeting",kind:"snippet",target:"Hi {cursor},\nThanks for your message.",aliases:["hello","greeting","salutation"],hotkey:"cmd+shift+g",keyword:";hello")]
}
@MainActor func checkInteractions() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("rc-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let state=try AppState(root:sandbox,headless:true);state.config.commands=featureFixture()
    var submitted=false,cancelled=false
    let field=PaletteSearchField(text:Binding(get:{state.query},set:{state.query=$0}),placeholder:"Search",onMove:state.moveSelection,onSubmit:{submitted=true},onCancel:{cancelled=true},onActions:{state.actionsOpen.toggle()})
    let coordinator=PaletteSearchField.Coordinator(field),input=SearchInput(),editor=NSTextView()
    input.stringValue="obsidian";coordinator.controlTextDidChange(Notification(name:NSControl.textDidChangeNotification,object:input))
    guard state.query=="obsidian",state.visibleCommands.map(\.id)==["app.obsidian"] else {throw LauncherError("Native text input did not filter results.")}
    input.stringValue="";coordinator.controlTextDidChange(Notification(name:NSControl.textDidChangeNotification,object:input))
    for _ in 0..<2 {_=coordinator.control(input,textView:editor,doCommandBy:NSSelectorFromString("moveDown:"))}
    guard state.selection==2,state.selectedCommand?.id=="capture.quick-note" else {throw LauncherError("Down arrow did not select Quick Note.")}
    _=coordinator.control(input,textView:editor,doCommandBy:NSSelectorFromString("moveUp:"))
    guard state.selection==1 else {throw LauncherError("Up arrow did not move selection.")}
    _=coordinator.control(input,textView:editor,doCommandBy:NSSelectorFromString("insertNewline:"))
    _=coordinator.control(input,textView:editor,doCommandBy:NSSelectorFromString("cancelOperation:"))
    guard submitted,cancelled else {throw LauncherError("Return/Escape did not reach the field delegate.")}
    let panel=PalettePanel(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:[.borderless],backing:.buffered,defer:false)
    panel.isReleasedWhenClosed=false
    panel.actions={state.actionsOpen.toggle()}
    let event=NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:.command,timestamp:0,windowNumber:panel.windowNumber,context:nil,characters:"k",charactersIgnoringModifiers:"k",isARepeat:false,keyCode:40)!
    guard panel.performKeyEquivalent(with:event),state.actionsOpen else {throw LauncherError("Command K did not open actions.")}
    state.query="quick note";state.begin(state.selectedCommand!);state.expandInlineInput()
    guard state.form?.command.id=="capture.quick-note",state.form?.isRun==true else {throw LauncherError("Quick Note did not request input.")}
    state.form=nil;state.newCommand("url",pasteboard:NSPasteboard(name:NSPasteboard.Name("rc-create-fixture-"+UUID().uuidString)))
    guard state.form?.isNew==true,state.form?.command.kind=="url" else {throw LauncherError("Quicklink editor did not open.")}
    state.form=nil;state.mode="snippets"
    guard state.visibleCommands.map(\.id)==["snippet.greeting"] else {throw LauncherError("Snippet navigation failed.")}
    state.mode="quicklinks";state.query="gmaps"
    guard state.selectedCommand?.id=="link.search" else {throw LauncherError("Quicklink alias search failed.")}
    let all=state.searchCommands("",mode:"snippets",limit:0)
    guard all.count==1 else {throw LauncherError("Filtered list failed.")}
    let server=ControlServer(root:sandbox)
    try server.start {request,reply in state.handle(request,sink:reply)}
    defer {server.stop()}
    state.mode="commands"
    let created=Command(id:"saved.link",title:"Saved Quicklink",kind:"url",target:"https://example.com/?q={query}",aliases:["saved"])
    var saveClosedEditor=false
    state.saveCommand(created,isNew:true,onSuccess:{saveClosedEditor=true})
    let deadline=Date().addingTimeInterval(8)
    while state.busy,Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    guard !state.busy,saveClosedEditor,state.config.resolve("saved")?.id=="saved.link",state.message.isEmpty else {throw LauncherError("Native editor save/reload failed: \(state.message)")}
    func waitForSave() throws {
        let deadline=Date().addingTimeInterval(8)
        while state.busy,Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
        guard !state.busy,state.message.isEmpty else {throw LauncherError("Settings save failed: \(state.message)")}
    }
    state.setExpansionEnabled(true);try waitForSave()
    guard state.config.snippetExpansionEnabled==true else {throw LauncherError("Expansion setting did not reload.")}
    state.setClipboardEnabled(false);try waitForSave()
    guard !state.config.clipboard.enabled else {throw LauncherError("Clipboard setting did not reload.")}
    state.setClipboardEnabled(true);try waitForSave()
    let palette=PaletteController(state:state)
    var settingsOpened=false;state.showManagement={settingsOpened=true}
    palette.show("settings")
    guard settingsOpened,!palette.panel.isVisible,state.resolveCommand("settings")?.target=="settings" else {throw LauncherError("Settings routing failed.")}
    // A concurrent manual edit must survive an editor save based on an old snapshot.
    let configPath=sandbox.appendingPathComponent("config.json"),external=try Data(contentsOf:configPath)+Data("\n".utf8)
    try external.write(to:configPath)
    state.saveCommand(Command(id:"blocked.link",title:"Blocked",kind:"url",target:"https://example.com"),isNew:true)
    let nextDeadline=Date().addingTimeInterval(8)
    while state.busy,Date()<nextDeadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    guard try Data(contentsOf:configPath)==external,state.message.contains("changed") else {throw LauncherError("Concurrent edit was overwritten.")}
    // The production replacement executor edits a real offscreen text editor;
    // no CGEvent is posted and no app receives input during this check.
    let document=NSTextView();document.string="prefix ;hell";document.setSelectedRange(NSRange(location:document.string.utf16.count,length:0))
    let plan=SnippetReplacement(keyword:";hello",finalInput:"o",text:"Hi Ω!",cursor:3)
    plan.apply(key:{code in if code==51 {document.deleteBackward(nil)} else if code==123 {document.moveLeft(nil)}},insert:{document.insertText($0,replacementRange:document.selectedRange())})
    guard document.string=="prefix Hi Ω!",document.selectedRange().location=="prefix Hi ".utf16.count else {throw LauncherError("Snippet replacement or cursor placement failed.")}
    // Exercise the reported two-key datetime trigger through the shared matcher,
    // focus policy, renderer and edit executor in a hidden real editor.
    let now=Command(id:"snippet.now",title:"Now",kind:"snippet",target:"{datetime}",keyword:";;")
    var matcher=SnippetMatcher()
    guard SnippetFocusPolicy.allowsExpansion(secureInput:false,subrole:nil),
        matcher.type(";",commands:[now])==nil,matcher.type(";",commands:[now])==now else {throw LauncherError("Unavailable AX metadata blocked a normal snippet.")}
    let rendered=try Templates.render(now,values:[:],now:Date(timeIntervalSince1970:1761420900),locale:Locale(identifier:"en_GB"),timeZone:TimeZone(secondsFromGMT:0)!)
    document.string="prefix ;";document.setSelectedRange(NSRange(location:document.string.utf16.count,length:0))
    SnippetReplacement(keyword:";;",finalInput:";",text:rendered.0,cursor:rendered.1).apply(key:{code in if code==51 {document.deleteBackward(nil)}},insert:{document.insertText($0,replacementRange:document.selectedRange())})
    guard document.string=="prefix 25 October 2025 19:35",
        !SnippetFocusPolicy.allowsExpansion(secureInput:true,subrole:nil),
        !SnippetFocusPolicy.allowsExpansion(secureInput:false,subrole:kAXSecureTextFieldSubrole) else {throw LauncherError("Datetime expansion or secure-entry guard failed.")}
    var roundTrip="",eventPayloadValid=true
    let longText=String(repeating:"Line Ω 👨‍👩‍👧‍👦\n",count:40)
    SnippetReplacement(keyword:";test",finalInput:"t",text:longText,cursor:nil).apply(key:{_ in},insert:{chunk in
        let units=Array(chunk.utf16)
        guard let event=CGEvent(keyboardEventSource:nil,virtualKey:0,keyDown:true) else {eventPayloadValid=false;return}
        event.keyboardSetUnicodeString(stringLength:units.count,unicodeString:units)
        var outputUnits=[UniChar](repeating:0,count:32),length=0
        event.keyboardGetUnicodeString(maxStringLength:32,actualStringLength:&length,unicodeString:&outputUnits)
        let decoded=String(utf16CodeUnits:outputUnits,count:length)
        eventPayloadValid = eventPayloadValid && decoded==chunk && length<=16
        roundTrip+=decoded
    })
    guard eventPayloadValid,roundTrip==longText else {throw LauncherError("CGEvent Unicode payload changed or truncated.")}
    let choiceCommand=Command(id:"choice",title:"Choose a contact",kind:"snippet",target:#"{argument name="Contact" options="Example A, Example B"}{cursor} Ω"#,keyword:";contact")
    let choiceFields=Templates.fields(choiceCommand)
    guard choiceFields.first?.options == ["Example A","Example B"] else {throw LauncherError("Choice options were not normalized.")}
    document.string="prefix ;contact";document.setSelectedRange(NSRange(location:document.string.utf16.count,length:0))
    let cancelledChoice=SnippetChoiceRequest(command:choiceCommand,pid:42);cancelledChoice.cancel()
    guard !cancelledChoice.active,document.string=="prefix ;contact" else {throw LauncherError("Cancellation changed the editor.")}
    let chosen=SnippetChoiceRequest(command:choiceCommand,pid:42)
    let replacement=try chosen.replacement(values:["Contact":"Example B"],pid:42,safe:true,unchanged:true)
    replacement.apply(key:{code in if code==51 {document.deleteBackward(nil)} else if code==123 {document.moveLeft(nil)}},insert:{document.insertText($0,replacementRange:document.selectedRange())})
    guard document.string=="prefix Example B Ω",document.selectedRange().location=="prefix Example B".utf16.count else {throw LauncherError("Choice insertion or cursor position failed.")}
    try checkAppLaunchDismissal()
    try checkSnippetChoiceKeyboard()
    panel.close()
    return ["appLaunchDismissal":true,"snippetChoiceKeyboard":true,"snippetChoiceConfirmationCancellation":true,"ok":true,"nativeTextFieldFilters":true,"nativeArrowDelegateSelects":true,"returnAndEscape":true,"commandKOpensActions":true,"quickNoteInputForm":true,"quicklinkEditor":true,"snippetsAndQuicklinksNavigation":true,"offscreenSnippetReplacement":true,"datetimeKeywordWithUnavailableAX":true,"secureSnippetGuards":true,"nativeUnicodeEventPayloadRoundTrip":true,"settingsSaveThroughCLIAndReload":true,"settingsCommandRouting":true,"editorSavesThroughCLIAndReloads":true,"concurrentConfigEditPreserved":true,"liveDesktopInputSent":false]
}
@MainActor func renderFeatures(_ path:String,theme:String,mode:String) throws {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("rustcast-feature-render-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let state=try AppState(root:sandbox,headless:true);state.config.commands=featureFixture()
    if mode=="search" {state.query="obsidian"}
    if mode=="note-form" {state.begin(state.config.commands[2])}
    if mode=="snippet-editor" {state.newCommand("snippet");state.form?.command.title="Greeting";state.form?.command.target="Hi {cursor},\nThanks for your message.";state.form?.command.keyword=";hello"}
    if mode=="quicklink-form" {state.begin(state.config.commands[4])}
    if mode=="snippets" {state.mode="snippets"}
    let choice=Command(id:"snippet.choice",title:"Choose a contact",kind:"snippet",target:#"{argument name="Contact" options="Example A, Example B, Example C"}"#,keyword:";contact")
    let root:AnyView=mode=="snippet-choice" ? AnyView(SnippetChoiceView(command:choice,submit:{_ in},cancel:{})):mode=="actions" ? AnyView(PaletteActions(state:state).background(Color(nsColor:.windowBackgroundColor))):AnyView(Palette(state:state))
    let view=NSHostingView(rootView:root);view.appearance=NSAppearance(named:theme=="dark" ? .darkAqua:.aqua)
    view.frame=NSRect(x:0,y:0,width:mode=="snippet-choice" ? 456:mode=="actions" ? 256:760,height:mode=="snippet-choice" ? 200:mode=="actions" ? 380:490);view.layoutSubtreeIfNeeded()
    guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Could not render features.")}
    view.cacheDisplay(in:view.bounds,to:bitmap)
    guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode features.")}
    try data.write(to:URL(fileURLWithPath:path))
}

// Exercise one retained hosting tree, not only the model or a fresh screenshot.
// Vision reads actual rendered labels; no production window or desktop input is used.
@MainActor func checkRenderedUpdates(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("rc-render-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let state=try AppState(root:sandbox,headless:true);state.config.commands=featureFixture()
    let host=NSHostingView(rootView:Palette(state:state));host.appearance=NSAppearance(named:.darkAqua)
    let panel=NSPanel(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
    panel.isReleasedWhenClosed=false;panel.contentView=host
    host.frame=NSRect(x:0,y:0,width:760,height:490)
    defer {panel.close()}
    var capturedHost:NSView=host
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    func capture(_ name:String) throws -> [String] {
        let deadline=Date().addingTimeInterval(0.15)
        while Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
        capturedHost.layoutSubtreeIfNeeded()
        guard let bitmap=capturedHost.bitmapImageRepForCachingDisplay(in:capturedHost.bounds) else {throw LauncherError("No render bitmap.")}
        capturedHost.cacheDisplay(in:capturedHost.bounds,to:bitmap)
        guard let data=bitmap.representation(using:.png,properties:[:]),let cg=bitmap.cgImage else {throw LauncherError("No render image.")}
        try data.write(to:directory.appendingPathComponent(name+".png"))
        let request=VNRecognizeTextRequest();request.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage:cg).perform([request])
        return (request.results ?? []).compactMap {$0.topCandidates(1).first?.string}
    }
    let initial=try capture("initial")
    guard initial.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Initial view did not render rows: \(initial)")}
    func searchField(_ view:NSView)->SearchInput? {if let field=view as? SearchInput {return field};return view.subviews.lazy.compactMap {searchField($0)}.first}
    guard let input=searchField(host) else {throw LauncherError("Rendered search field missing.")}
    for query in ["o","ob","obs","obsidian"] {
        input.stringValue=query
        input.delegate?.controlTextDidChange?(Notification(name:NSControl.textDidChangeNotification,object:input))
    }
    guard state.query=="obsidian" else {throw LauncherError("Hosting field delegate did not write binding.")}
    let filtered=try capture("filtered")
    guard filtered.contains(where:{$0.contains("Obsidian")}),!filtered.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Rendered search rows are stale: \(filtered)")}
    state.query="missing zzxx"
    let empty=try capture("empty")
    guard empty.contains(where:{$0.contains("No commands found")}),!empty.contains(where:{$0.contains("Obsidian")}) else {throw LauncherError("Rendered empty search is stale: \(empty)")}
    state.query="";state.mode="snippets"
    let snippets=try capture("snippets")
    guard snippets.contains(where:{$0.contains("Greeting")}),!snippets.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Rendered snippets are stale: \(snippets)")}
    state.mode="commands"
    let restored=try capture("restored")
    guard restored.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Clearing search did not restore rows.")}
    let beforeMove=try Data(contentsOf:directory.appendingPathComponent("restored.png"))
    state.moveSelection(1);_=try capture("arrow-selection")
    guard try Data(contentsOf:directory.appendingPathComponent("arrow-selection.png")) != beforeMove else {throw LauncherError("Arrow selection did not redraw.")}
    let controller=ManagementController(state:state)
    guard !controller.window.isVisible,let management=controller.window.contentView else {throw LauncherError("Management opened automatically.")}
    defer {controller.window.close()}
    capturedHost=management
    controller.window.appearance=NSAppearance(named:.darkAqua);management.appearance=NSAppearance(named:.darkAqua)
    state.query="obsidian"
    let settings=try capture("settings-dark")
    guard settings.contains(where:{$0.contains("General")}),settings.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Settings did not render: \(settings)")}
    controller.navigation.query="gmaps"
    let managedSearch=try capture("settings-search")
    guard managedSearch.contains(where:{$0.contains("Search Google Maps")}),!managedSearch.contains(where:{$0.contains("Finder")}),state.query=="obsidian" else {throw LauncherError("Management search stale or changed launcher query: \(managedSearch)")}
    controller.navigation.query="";controller.navigation.kind="snippets"
    let managedSnippets=try capture("settings-snippets")
    guard managedSnippets.contains(where:{$0.contains("Greeting")}),!managedSnippets.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Management library stale: \(managedSnippets)")}
    controller.navigation.form=CommandForm(command:state.config.commands.last!,isRun:false)
    let editor=try capture("settings-editor")
    guard editor.contains(where:{$0.contains("Keyword")}) else {throw LauncherError("Management editor missing: \(editor)")}
    controller.navigation.form=nil;controller.window.appearance=NSAppearance(named:.aqua);management.appearance=NSAppearance(named:.aqua)
    controller.window.setContentSize(NSSize(width:680,height:480))
    _=try capture("settings-light-small")
    panel.contentView=host;capturedHost=host;host.appearance=NSAppearance(named:.aqua)
    state.query="obsidian";panel.setContentSize(NSSize(width:640,height:400))
    let small=try capture("launcher-light-small")
    guard small.contains(where:{$0.contains("Obsidian")}),!small.contains(where:{$0.contains("Finder")}) else {throw LauncherError("Small launcher results stale.")}
    return ["ok":true,"persistentHostingView":true,"renderedTextChecks":12,"nativeHostingFieldDelegate":true,"separateManagementNavigation":true,"managementDoesNotOpenAutomatically":true,"desktopInputSent":false,"visibleWindowOpened":false]
}

// Route real AppKit key events through the same hidden panel as production.
@MainActor func checkSnippetChoiceKeyboard() throws {
    let command=Command(id:"choice.keys",title:"Choose a contact",kind:"snippet",target:#"{argument name="Contact" options="Alpha, Beta, Gamma"}"#,keyword:";contact")
    var submissions:[[String:String]]=[],cancellations=0,throwawaySubmission=false
    let model=SnippetChoiceModel(command:command,submit:{submissions.append($0)},cancel:{cancellations+=1})
    let panel=SnippetChoicePanel(model:model);panel.contentView=NSHostingView(rootView:SnippetChoiceView(model:model));panel.contentView?.layoutSubtreeIfNeeded();defer {panel.close()}
    func key(_ code:UInt16,_ flags:NSEvent.ModifierFlags=[],repeatKey:Bool=false)->NSEvent {
        NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:panel.windowNumber,context:nil,characters:"",charactersIgnoringModifiers:"",isARepeat:repeatKey,keyCode:code)!
    }
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    try require(model.values["Contact"]=="Alpha","Choice did not start on the first option")
    panel.sendEvent(key(125));try require(model.values["Contact"]=="Beta","Down did not switch immediately")
    panel.sendEvent(key(124));try require(model.values["Contact"]=="Gamma","Right did not switch immediately")
    panel.sendEvent(key(126));try require(model.values["Contact"]=="Beta","Up did not switch immediately")
    panel.sendEvent(key(123));try require(model.values["Contact"]=="Alpha","Left did not switch immediately")
    panel.sendEvent(key(123));try require(model.values["Contact"]=="Alpha","Choice escaped its lower bound")
    try require(!model.handle(key(125,.command),editingText:false),"Command-arrow was intercepted")
    try require(!model.handle(key(125),editingText:true),"Text editing arrow was intercepted")
    try require(!model.handle(key(36),editingText:true,composing:true),"IME Return was intercepted")
    panel.sendEvent(key(125,[],repeatKey:true));try require(model.values["Contact"]=="Beta","Repeated arrow failed")
    try require(panel.performKeyEquivalent(with:key(36)),"Plain Return did not route through the panel")
    panel.sendEvent(key(36));model.submit()
    try require(submissions.count==1 && submissions[0]["Contact"]=="Beta","Return did not insert the selected option exactly once")
    let cancelled=SnippetChoiceModel(command:command,submit:{_ in throwawaySubmission=true},cancel:{cancellations+=1})
    let cancelPanel=SnippetChoicePanel(model:cancelled);defer {cancelPanel.close()}
    cancelPanel.sendEvent(key(53));cancelPanel.sendEvent(key(36))
    try require(cancellations==1 && !throwawaySubmission,"Escape did not cancel without inserting")
    let keypad=SnippetChoiceModel(command:command,submit:{submissions.append($0)},cancel:{})
    let keypadPanel=SnippetChoicePanel(model:keypad);defer {keypadPanel.close()}
    keypadPanel.sendEvent(key(76,[],repeatKey:true));try require(submissions.count==1,"Held Enter submitted unexpectedly")
    keypadPanel.sendEvent(key(76));try require(submissions.count==2,"Keypad Enter did not insert")
}

// No real application starts and no visible window is opened.
@MainActor func checkAppLaunchDismissal() throws {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("launch-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    defer {state.shutdown()}
    state.config.commands=[Command(id:"fixture.app",title:"Fixture App",kind:"app",target:"test.fixture")]
    var hidden=0,launched=0,completion:((Error?)->Void)?,toasts:[Bool]=[]
    state.hidePalette={hidden+=1;state.paletteVisible=false}
    state.showToast={_,ok in toasts.append(ok)}
    state.applicationURL={_ in URL(fileURLWithPath:"/Fixture.app")}
    state.openApplication={_,reply in
        guard hidden==1,!state.paletteVisible else {fatalError("Palette still visible when app launch started")}
        launched+=1;completion=reply
    }
    state.paletteVisible=true
    state.perform("fixture.app")
    guard hidden==1,launched==1,state.busy else {throw LauncherError("App did not dismiss before its delayed launch")}
    state.perform("fixture.app")
    guard launched==1 else {throw LauncherError("Repeated Enter started the app twice")}
    state.paletteVisible=true // User has reopened the launcher while target is loading.
    let commands=state.config.commands;state.config.commands=[]
    completion?(nil)
    state.config.commands=commands
    guard state.paletteVisible,hidden==1,!state.busy else {throw LauncherError("Delayed app completion hid the reopened palette")}
    state.applicationURL={_ in nil};state.message=""
    state.perform("fixture.app")
    guard hidden==1,!state.message.isEmpty,toasts.last==false else {throw LauncherError("Missing app lost its error or hid the palette")}
    state.applicationURL={_ in URL(fileURLWithPath:"/Fixture.app")}
    state.openApplication={_,reply in launched+=1;completion=reply}
    state.perform("fixture.app");completion?(LauncherError("Fixture launch failed"))
    guard hidden==2,state.message=="Fixture launch failed",toasts.last==false else {throw LauncherError("Asynchronous app launch failure was not reported")}
    var replied=false
    state.execute("fixture.app",dryRun:true) {_ in replied=true}
    guard replied,hidden==2,launched==2 else {throw LauncherError("Dry run launched an application")}
    state.execute("fixture.app",dryRun:false) {_ in replied=true};completion?(nil)
    guard hidden==2,launched==3 else {throw LauncherError("CLI app launch dismissed the interface")}
}
