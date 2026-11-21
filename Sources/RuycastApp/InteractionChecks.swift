import AppKit
import SwiftUI
import LauncherCore
import LauncherModels
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
    try checkAppLaunchDismissal()
    try checkSnippetChoiceKeyboard()
    return ["appLaunchDismissal":true,"snippetChoiceKeyboard":true,"ok":true,"nativeTextFieldFilters":true,"nativeArrowDelegateSelects":true,"returnAndEscape":true,"quickNoteInputForm":true,"quicklinkEditor":true,"snippetsAndQuicklinksNavigation":true,"settingsSaveThroughCLIAndReload":true,"settingsCommandRouting":true,"editorSavesThroughCLIAndReloads":true,"concurrentConfigEditPreserved":true,"liveDesktopInputSent":false]
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
