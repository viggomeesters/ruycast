import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkScriptChoicesUI(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let output=URL(fileURLWithPath:outputPath,isDirectory:true)
    try FileManager.default.createDirectory(at:output,withIntermediateDirectories:true)
    let root=output.appendingPathComponent("fixture-profile"),state=try AppState(root:root,headless:true)
    defer {state.shutdown();try? FileManager.default.removeItem(at:root)}
    func require(_ ok:Bool,_ message:String)throws {if !ok {throw LauncherError(message)}}
    func pump(_ seconds:Double=0.12) {let end=Date().addingTimeInterval(seconds);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait() throws {let end=Date().addingTimeInterval(8);while state.busy,Date()<end {pump(0.02)};try require(!state.busy,"Fixture script did not finish")}
    func image(_ name:String,width:Double=760) throws {
        let view=NSHostingView(rootView:Palette(state:state));view.frame=NSRect(x:0,y:0,width:width,height:480);pump();view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("No choice preview")};view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("No choice pixels")};try png.write(to:output.appendingPathComponent(name+".png"))
    }
    let items=[ScriptChoice(id:"project",title:"Example Project",subtitle:"EX · Projectbreed",value:"pinned-project"),ScriptChoice(id:"part",title:"Planning",subtitle:"EX / PLAN · Example Project",value:"pinned-component")]
    let choices=ScriptChoices(title:"Project Koppelen",placeholder:"Zoek project, onderdeel of code…",items:items)
    let provider=root.appendingPathComponent("personal_bridge.py"),saved=root.appendingPathComponent("applied.txt"),stale=root.appendingPathComponent("stale.flag")
    let data=String(decoding:try Files.encode(choices),as:UTF8.self)
    let quoted=String(data:data.data(using:.utf8)!,encoding:.utf8)!
    let python="""
    import sys,json,pathlib
    choices=json.loads(\(String(data:try JSONSerialization.data(withJSONObject:quoted,options:.fragmentsAllowed),encoding:.utf8)!))
    selected=sys.argv[1]
    if not selected: print(json.dumps({'ok':True,'choices':choices}))
    elif pathlib.Path(sys.argv[3]).exists(): print(json.dumps({'ok':False,'error':'De note is intussen gewijzigd. Lees opnieuw.'}))
    else:
        pathlib.Path(sys.argv[2]).write_text(selected)
        print(json.dumps({'ok':True}))
    """
    try python.write(to:provider,atomically:true,encoding:.utf8)
    let cmd=Command(id:"fixture.project-picker",title:"Project Koppelen",kind:"script",target:"/usr/bin/python3",arguments:[provider.path,"{input}",saved.path,stale.path])
    state.config.commands.append(cmd);state.begin(cmd);try wait()
    try require(state.scriptChoices?.choices==choices && !FileManager.default.fileExists(atPath:saved.path),"Choice load wrote data or lost choices")
    state.config.theme="light";try image("project-light");state.config.theme="dark";try image("project-dark");try image("project-narrow",width:480)
    state.busy=true;try image("project-loading");state.busy=false
    let coordinator=PaletteSearchField(text:.constant(""),placeholder:"Search",onMove:state.moveScriptChoice,onSubmit:state.submitScriptChoice,onCancel:state.cancelScriptChoices,onActions:{}).makeCoordinator()
    _=coordinator.control(NSTextField(),textView:NSTextView(),doCommandBy:NSSelectorFromString("moveDown:"))
    try require(state.scriptChoiceSelection==1,"Down arrow did not change choice")
    state.scriptChoiceQuery="EX PLAN";try require(state.filteredScriptChoices.map(\.id)==["part"] && state.scriptChoiceSelection==0,"Search did not find component or reset selection")
    state.scriptChoiceQuery="nothing";state.submitScriptChoice();try require(!FileManager.default.fileExists(atPath:saved.path),"Empty search ran a command");try image("project-empty")
    state.scriptChoiceQuery="PLAN";try "stale".write(to:stale,atomically:true,encoding:.utf8)
    _=coordinator.control(NSTextField(),textView:NSTextView(),doCommandBy:NSSelectorFromString("insertNewline:"));try wait()
    try require(state.scriptChoices != nil && state.message.contains("gewijzigd") && !FileManager.default.fileExists(atPath:saved.path),"Stale selection was applied or dismissed")
    try image("project-stale");try FileManager.default.removeItem(at:stale);state.reloadScriptChoices();try wait();try require(state.message.isEmpty && state.scriptChoiceQuery.isEmpty,"Reload did not refresh choices")
    state.scriptChoiceQuery="PLAN";state.submitScriptChoice();state.submitScriptChoice();try wait()
    try require(try String(contentsOf:saved,encoding:.utf8)=="pinned-component" && state.scriptChoices==nil,"Enter did not submit selected pinned value")
    state.begin(cmd);try wait();state.cancelScriptChoices();try require(state.scriptChoices==nil,"Escape did not leave choices")
    state.scriptChoices=ScriptChoiceSession(commandID:cmd.id,choices:ScriptChoices(title:"Project Koppelen",placeholder:"Zoek…",items:[]),source:"launcher")
    try image("project-no-choices");state.cancelScriptChoices()
    try "import json\nprint(json.dumps({'ok':False,'error':'Mailverwerking vraagt aandacht','detail':'APPLE MAIL VRAAGT AANDACHT\\nArchivering vraagt aandacht'}))".write(to:provider,atomically:true,encoding:.utf8)
    state.begin(cmd);try wait()
    try require(state.detail?.contains("Archivering vraagt aandacht")==true && state.message.contains("Mailverwerking"),"Failure report was discarded or shown as success")
    try image("mail-partial-failure")
    state.detail=nil;state.message=""
    let generic=root.appendingPathComponent("choices.py");try python.write(to:generic,atomically:true,encoding:.utf8)
    let genericCommand=Command(id:"fixture.generic-picker",title:"Generic Choices",kind:"script",target:"/usr/bin/python3",arguments:[generic.path,"{input}",saved.path,stale.path])
    state.config.commands.append(genericCommand);state.begin(genericCommand);try wait()
    try require(state.scriptChoices?.commandID==genericCommand.id,"Generic script choices were not presented")
    state.scriptChoiceQuery="PLAN";state.submitScriptChoice();try wait();try require(state.scriptChoices==nil,"Generic script selection did not finish")
    return ["ok":true,"search":true,"keyboard":true,"staleRefused":true,"reload":true,"selectedValueSubmitted":true,"cancel":true,"emptySearchNoWrite":true,"desktopInputSent":false,"visibleWindowOpened":false]
}
