import AppKit
import SwiftUI
import ApplicationServices
import LauncherCore
import LauncherModels

/// Real AX handles for nonexistent fixture PIDs; all reads/effects stay in this map.
private final class MenuFixture:@unchecked Sendable {
    private struct Node {var attributes:[String:CFTypeRef];var children:[AXUIElement];var unreadable=false}
    private let lock=NSLock()
    private var nodes:[CFHashCode:Node]=[:]
    private var nextPID:pid_t=700000
    private var appName="TextEdit Fixture"
    private var clock:TimeInterval=0
    private var clockStep:TimeInterval=0.00001
    private var reads=0
    private var appElement:AXUIElement?
    func add(_ title:String,_ role:String,_ enabled:Bool?=true,_ children:[AXUIElement]=[],_ extra:[String:CFTypeRef]=[:])->AXUIElement {
        lock.lock();defer {lock.unlock()};nextPID+=1
        let element=AXUIElementCreateApplication(nextPID)
        var values=extra;values[kAXTitleAttribute]=title as CFString;values[kAXRoleAttribute]=role as CFString
        if let enabled {values[kAXEnabledAttribute]=enabled ? kCFBooleanTrue:kCFBooleanFalse}
        nodes[CFHash(element)]=Node(attributes:values,children:children)
        if role==kAXApplicationRole {appElement=element}
        return element
    }
    func set(_ element:AXUIElement,_ key:String,_ value:CFTypeRef?) {lock.lock();defer {lock.unlock()};nodes[CFHash(element)]?.attributes[key]=value}
    func read(_ element:AXUIElement,_ key:String)->CFTypeRef? {lock.lock();defer {lock.unlock()};reads+=1;return nodes[CFHash(element)]?.attributes[key]}
    func children(_ element:AXUIElement,_ limit:Int)->[AXUIElement]? {lock.lock();defer {lock.unlock()};guard let node=nodes[CFHash(element)],!node.unreadable else {return nil};return Array(node.children.prefix(limit))}
    func unreadable(_ element:AXUIElement,_ value:Bool) {lock.lock();defer {lock.unlock()};nodes[CFHash(element)]?.unreadable=value}
    func name()->String {lock.lock();defer {lock.unlock()};return appName}
    func application()->AXUIElement {lock.lock();defer {lock.unlock()};return appElement!}
    func rename(_ name:String) {lock.lock();defer {lock.unlock()};appName=name}
    func time()->TimeInterval {lock.lock();defer {lock.unlock()};clock+=clockStep;return clock}
    func step(_ value:TimeInterval) {lock.lock();defer {lock.unlock()};clockStep=value}
    var readCount:Int {lock.lock();defer {lock.unlock()};return reads}
}

/// Isolated menu fixtures and hidden native views; never opens another app's menu.
@MainActor func checkMenuActions(_ output:String?=nil) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-menus-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    func pump() {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    func wait(_ condition:()->Bool) throws {let end=Date().addingTimeInterval(4);while !condition(),Date()<end {pump()};try require(condition(),"Menu fixture timed out.")}
    try require(MenuShortcut(character:"o",glyph:nil,virtualKey:31,modifiers:0)?.keys==["⌘","O"],"AX default Command modifier lost.")
    try require(MenuShortcut(character:"+",glyph:nil,virtualKey:nil,modifiers:7)?.keys==["⌃","⌥","⇧","⌘","+"],"Literal plus or four app modifiers were collapsed into launcher Hyper.")
    try require(MenuShortcut(character:"X",glyph:nil,virtualKey:nil,modifiers:13)?.keys==["⌃","⇧","X"],"AX NoCommand modifier ignored.")
    try require(MenuShortcut(character:nil,glyph:100,virtualKey:nil,modifiers:0)?.label=="Command + Left Arrow","Menu glyph did not decode.")
    try require(MenuShortcut(character:nil,glyph:122,virtualKey:nil,modifiers:8)?.keys==["F12"],"Function glyph did not decode.")
    try require(MenuShortcut(character:nil,glyph:nil,virtualKey:36,modifiers:0)?.keys==["⌘","↵"],"Return virtual key did not decode.")
    try require(MenuShortcut(character:nil,glyph:nil,virtualKey:80,modifiers:0)?.keys==["⌘","F19"],"Function virtual key did not decode.")
    try require(MenuShortcut(character:"é",glyph:nil,virtualKey:0,modifiers:0)?.keys==["⌘","É"],"Layout-specific key was replaced with physical US letter.")
    try require(MenuShortcut(character:"ß",glyph:nil,virtualKey:nil,modifiers:0)?.keys==["⌘","ß"] && MenuShortcut(character:"\u{f700}",glyph:nil,virtualKey:nil,modifiers:0)?.keys==["⌘","↑"],"Unicode menu keys were expanded or lost.")
    try require(SystemAccessibility.integer(kCFBooleanFalse)==nil,"A boolean menu attribute was treated as a modifier mask.")
    try require(SystemAccessibility.boolean(NSNull())==nil,"An unknown AX enabled value was accepted.")
    try require(MenuShortcut(character:"",glyph:0,virtualKey:0,modifiers:0)==nil && MenuShortcut(character:"X",glyph:nil,virtualKey:nil,modifiers:nil)==nil && MenuShortcut(character:"X",glyph:nil,virtualKey:nil,modifiers:16)==nil,"Missing key or modifiers invented a shortcut.")
    let fixture=MenuFixture()
    let open=fixture.add("Open…",kAXMenuItemRole,true,[],[kAXMenuItemCmdCharAttribute:"O" as CFString,kAXMenuItemCmdModifiersAttribute:NSNumber(value:0)])
    let duplicate=fixture.add("Open…",kAXMenuItemRole,true)
    let save=fixture.add("Save",kAXMenuItemRole,false)
    let unknown=fixture.add("Unknown State",kAXMenuItemRole,nil)
    let unreadable=fixture.add("Unreadable Submenu",kAXMenuItemRole,true,[open]);fixture.unreadable(unreadable,true)
    let container=fixture.add("File",kAXMenuRole,true,[open,duplicate,save,unknown,unreadable])
    let file=fixture.add("File",kAXMenuBarItemRole,true,[container])
    let bar=fixture.add("",kAXMenuBarRole,true,[file])
    _=fixture.add("Fixture",kAXApplicationRole,true,[],[kAXMenuBarAttribute:bar])
    let identity=SystemServices.Identity(pid:123456,uid:getuid(),seconds:11,micros:22)
    let current=LockedValue<SystemServices.Identity?>(identity),trusted=LockedValue(true)
    let foreignElement=LockedValue<CFHashCode?>(nil)
    var pressed=[CFHashCode](),activated=0,afterActivate:(()->Void)?
    var effects=SystemServices.Effects()
    effects.identity={pid in current.read {$0?.pid==pid ? $0:nil}};effects.trusted={trusted.read {$0}}
    effects.menuApplication={_ in fixture.application()};effects.menuAttribute={fixture.read($0,$1)};effects.menuChildren={fixture.children($0,$1)};effects.menuApplicationName={_ in fixture.name()};effects.menuClock={fixture.time()}
    effects.menuElementPID={element in foreignElement.read {$0==CFHash(element) ? 999:identity.pid}}
    effects.activate={_ in activated+=1;afterActivate?();return true};effects.press={pressed.append(CFHash($0));return .success}
    let service=SystemServices(effects:effects)
    func search(_ query:String="",using provider:SystemServices?=nil) throws -> SystemSearchResults {
        var received:Result<SystemSearchResults,Error>?
        (provider ?? service).search(mode:"menus",query:query,previousPID:identity.pid,session:SystemSearchSession()) {received=$0}
        try wait {received != nil};return try received!.get()
    }
    let snapshot=try search()
    try require(snapshot.commands.count==4 && snapshot.menus.count==4,"Disabled/unknown items disappeared or unreadable submenu became an action.")
    let opens=snapshot.commands.filter {$0.title=="Open…"}
    try require(opens.count==2 && opens[0].id != opens[1].id,"Duplicate menu paths share an executable token.")
    let first=opens[0],second=opens[1],disabled=snapshot.commands.first {$0.title=="Save"}!
    try require(first.hotkey==nil && first.aliases.isEmpty && snapshot.menus[first.id]?.path=="File" && snapshot.menus[first.id]?.application=="TextEdit Fixture" && snapshot.menus[first.id]?.shortcut?.keys==["⌘","O"],"Menu origin/shortcut is missing or pollutes launcher bindings.")
    try require(snapshot.commands.first {$0.title=="Unknown State"}?.isEnabled==false && !disabled.isEnabled,"Unknown or disabled AX state is executable.")
    fixture.rename("Renamed Fixture")
    let later=try search("Renamed Fixture")
    try require(later.commands.count==4 && snapshot.menus[first.id]?.application=="TextEdit Fixture" && later.menus.values.allSatisfy {$0.application=="Renamed Fixture"},"Interleaved searches changed an earlier metadata snapshot.")
    try require(Set(later.commands.map(\.id)).isDisjoint(with:Set(snapshot.commands.map(\.id))),"A later scan retargeted an earlier action token.")
    try require(try search("File Open").commands.count==2,"Combined menu path/title search failed.")
    try require(try search("Open").commands.count==2,"Title search failed.")
    try require(service.execute(target:disabled.target,previousPID:identity.pid)["ok"] as? Bool==false && activated==0 && pressed.isEmpty,"Disabled menu performed a desktop effect.")
    try require(service.execute(target:first.target,previousPID:42)["ok"] as? Bool==false && activated==0,"Original app identity was ignored.")
    afterActivate={fixture.set(open,kAXEnabledAttribute,kCFBooleanFalse)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Menu disabled during activation was pressed.")
    fixture.set(open,kAXEnabledAttribute,kCFBooleanTrue)
    afterActivate={fixture.set(open,kAXTitleAttribute,"Other Action" as CFString)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Menu renamed during activation was pressed.")
    fixture.set(open,kAXTitleAttribute,"Open…" as CFString)
    afterActivate={fixture.set(file,kAXTitleAttribute,"Other Menu" as CFString)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Changed ancestor path was pressed.")
    fixture.set(file,kAXTitleAttribute,"File" as CFString)
    afterActivate={fixture.set(file,kAXEnabledAttribute,kCFBooleanFalse)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"A menu disabled during activation was pressed.")
    fixture.set(file,kAXEnabledAttribute,kCFBooleanTrue)
    afterActivate={fixture.set(open,kAXRoleAttribute,kAXButtonRole as CFString)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Changed menu role was pressed.")
    fixture.set(open,kAXRoleAttribute,kAXMenuItemRole as CFString)
    afterActivate={fixture.unreadable(open,true)}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Unreadable menu state was pressed.")
    fixture.unreadable(open,false)
    afterActivate={foreignElement.update {$0=CFHash(open)}}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"A menu element from another process was pressed.")
    foreignElement.update {$0=nil}
    afterActivate={current.update {$0=nil}}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"App exit during activation was ignored.")
    current.update {$0=identity};afterActivate=nil
    trusted.update {$0=false}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==false && pressed.isEmpty,"Accessibility permission was bypassed.")
    trusted.update {$0=true}
    try require(service.execute(target:first.target,previousPID:identity.pid)["ok"] as? Bool==true && pressed==[CFHash(open)],"Original menu action did not press its captured element.")
    try require(service.execute(target:second.target,previousPID:identity.pid)["ok"] as? Bool==true && pressed==[CFHash(open),CFHash(duplicate)],"Duplicate title executed the wrong captured element.")
    pressed=[]
    let state=try AppState(root:root,headless:true,systemServices:service)
    let retainedCounts=["fixture.persisted":7]
    state.counts=retainedCounts;try Files.save(retainedCounts,to:root.appendingPathComponent("usage.json"))
    let retainedUsageBytes=try Data(contentsOf:root.appendingPathComponent("usage.json"))
    try state.searchRanking.recordSuccess("fixture.persisted")
    let retainedRanking=state.searchRanking.entries,retainedRankingBytes=try Data(contentsOf:root.appendingPathComponent("search-ranking.json"))
    try require(state.searchCommands("menu-actions").contains {$0.target=="search-menu-items"},"Menu search has no discoverable launcher entry point.")
    state.previousPID=identity.pid;state.foregroundPID={nil};state.mode="menus"
    try wait {!state.results.isEmpty}
    state.applySystemResults(snapshot.commands,resources:[:],preserveSelection:false,menus:snapshot.menus)
    state.selection=snapshot.commands.firstIndex {$0.id==disabled.id}!
    try require(state.selectedCommand?.id==disabled.id && !state.actionItems.contains(where:{$0.id=="open" || $0.id=="set-hotkey" || $0.id=="set-alias"}),"An unavailable app menu item still offers an executable action.")
    state.activateSelection();try wait {!state.busy}
    try require(pressed.isEmpty,"Return ran a disabled menu item.")
    var reply:[String:Any]?
    state.handle(["op":"execute","id":disabled.id]) {reply=$0};try wait {reply != nil}
    try require(reply?["ok"] as? Bool==false && pressed.isEmpty,"CLI ran a disabled menu item.")
    reply=nil;state.handle(["op":"search","mode":"menus","query":"Open"]) {reply=$0};try wait {reply != nil}
    let rows=reply?["result"] as? [[String:Any]] ?? []
    try require(rows.count==2 && (rows[0]["menu"] as? [String:Any])?["application"] as? String=="Renamed Fixture" && rows[0]["hotkey"]==nil,"CLI menu search lost its origin metadata or published a launcher binding.")
    let executableID=rows[0]["id"] as! String
    reply=nil;state.handle(["op":"execute","id":executableID]) {reply=$0};try wait {reply != nil}
    try require(reply?["ok"] as? Bool==true && pressed==[CFHash(open)],"CLI search result did not execute its exact menu item.")
    try require(state.counts==retainedCounts && Data(contentsOf:root.appendingPathComponent("usage.json"))==retainedUsageBytes,"A temporary menu token was persisted as a reusable command.")
    try require(state.analytics.snapshot.commands.count==1 && state.analytics.snapshot.commands[AnalyticsStore.commandKey("system.menu")] != nil,"Ephemeral menu actions displaced per-command analytics.")
    // Adversarial size and a deterministic monotonic deadline, without sleeping.
    let large=MenuFixture()
    let leaves=(0..<1400).map {large.add("Item \($0)",kAXMenuItemRole)}
    let largeBar=large.add("",kAXMenuBarRole,true,leaves)
    _=large.add("Fixture",kAXApplicationRole,true,[],[kAXMenuBarAttribute:largeBar])
    var bounded=effects;bounded.menuApplication={_ in large.application()};bounded.menuAttribute={large.read($0,$1)};bounded.menuChildren={large.children($0,$1)};bounded.menuClock={large.time()}
    let boundedService=SystemServices(effects:bounded)
    let capped=try search("Item",using:boundedService)
    try require(capped.commands.count==100 && capped.menus.count==100,"Menu result cap failed.")
    let readsBefore=large.readCount
    try require(try search("not-present",using:boundedService).commands.isEmpty && large.readCount-readsBefore<6500,"Menu traversal node cap failed.")
    large.step(1)
    let deadlineReads=large.readCount
    try require(try search(using:boundedService).commands.isEmpty && large.readCount-deadlineReads<=2,"Expired menu deadline continued native reads.")
    large.step(0.00001)
    pressed=[]
    state.message="";state.applySystemResults(snapshot.commands,resources:[:],preserveSelection:false,menus:snapshot.menus);state.selection=0
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        state.mode="commands"
        let controller=PaletteController(state:state);state.showToast=nil;state.paletteVisible=true
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        state.mode="menus";try wait {state.results.count==4}
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        func searchField(_ view:NSView)->SearchInput? {if let field=view as? SearchInput {return field};return view.subviews.compactMap {searchField($0)}.first}
        guard let host=controller.panel.contentView,let field=searchField(host) else {throw LauncherError("Menu native search field missing.")}
        try require(field.accessibilityLabel()=="Search app menu…","Menu mode kept the previous native accessible search name.")
        state.applySystemResults(snapshot.commands,resources:[:],preserveSelection:false,menus:snapshot.menus)
        controller.restoreSearchFocus();for _ in 0..<5 {pump()}
        guard let editor=controller.panel.firstResponder as? NSTextView else {throw LauncherError("Menu native field editor did not acquire focus.")}
        func enter()->NSEvent {NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:controller.panel.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!}
        state.selection=snapshot.commands.firstIndex {$0.id==disabled.id}!
        editor.keyDown(with:enter());try wait {!state.busy};try require(pressed.isEmpty,"Native Return executed a disabled menu item.")
        state.selection=0;state.message="";editor.keyDown(with:enter());try wait {!state.busy}
        try require(pressed==[CFHash(open)] && !controller.panel.isVisible,"Native Return failed to run its menu fixture without desktop focus.")
        try require(state.counts==retainedCounts,"Native menu Return persisted an ephemeral usage entry.")
        try require(state.searchRanking.entries==retainedRanking && Data(contentsOf:root.appendingPathComponent("search-ranking.json"))==retainedRankingBytes,"Native menu Return displaced durable command recency.")
        fixture.rename("Refreshed Fixture");state.refreshMenuResults()
        try require(state.menuSearchBusy,"A pending menu refresh reports no loading state.")
        try wait {state.menuActions.values.first?.application=="Refreshed Fixture"}
        try require(!state.menuSearchBusy,"Completed menu refresh stays busy.")
        state.refreshMenuResults();state.cancelMenuSearch()
        try require(!state.menuSearchBusy,"Closing menu search retains its loading state.")
        state.refreshMenuResults();try wait {state.results.count==4}
        try require(state.query.isEmpty && state.results.count==4,"Explicit menu refresh changed the query or kept stale origin.")
        state.query="missing action";try wait {state.results.isEmpty}
        trusted.update {$0=false};state.refreshMenuResults();try wait {!state.message.isEmpty}
        let permissionError=state.message
        try require(permissionError.contains("System Settings") && permissionError.contains("Accessibility") && state.results.isEmpty,"Menu permission failure lost its concrete recovery.")
        trusted.update {$0=true};state.query="";state.refreshMenuResults();try wait {state.results.count==4}
        try require(state.message.isEmpty,"Successful menu refresh retained an old permission error.")
        state.applySystemResults(snapshot.commands,resources:[:],preserveSelection:false,menus:snapshot.menus)
        state.message="";state.paletteVisible=true
        for dark in [false,true] {for width in [600,760] {for variant in ["enabled","disabled","long","empty","denied","loading"] {
            var commands=snapshot.commands,menus=snapshot.menus
            if variant=="long" {
                commands[0].title="Export Presentation with All Linked Resources…"
                menus[commands[0].id]=MenuActionInfo(application:"A Long Application Name",path:"File › Export › Presentation and Sharing",title:commands[0].title,shortcut:MenuShortcut(character:"+",glyph:nil,virtualKey:nil,modifiers:7),enabled:true)
            }
            if ["empty","denied","loading"].contains(variant) {commands=[];menus=[:]}
            state.message=variant=="denied" ? permissionError:""
            state.paletteSession.nativeSearchBusy=variant=="loading"
            state.applySystemResults(commands,resources:[:],preserveSelection:false,menus:menus)
            state.config.theme=dark ? "dark":"light";state.selection=variant=="disabled" ? snapshot.commands.firstIndex {$0.id==disabled.id}!:0
            controller.panel.setContentSize(NSSize(width:width,height:490));controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
            guard let host=controller.panel.contentView,let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("Menu render unavailable.")}
            host.cacheDisplay(in:host.bounds,to:bitmap);guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Menu render unavailable.")}
            try data.write(to:directory.appendingPathComponent("menus-\(dark ? "dark":"light")-\(width)-\(variant).png"))
        }}}
        controller.hide();try require(!controller.panel.isVisible,"Menu check made its panel visible.")
    }
    return ["ok":true,"injectedAXMenuEnumeration":true,"shortcutCharactersGlyphsModifiers":true,"disabledAndUnknownStateFailClosed":true,"distinctDuplicateTargets":true,"postActivationRevalidation":true,"originalAppIdentity":true,"immutableSearchMetadata":true,"cliSearchAndExecution":true,"disabledReturnAndCLI":true,"boundedTraversalAndDeadline":true,"noLauncherBindingsChanged":true,"nativeSearchFieldReturnAndRecovery":output != nil,"realDesktopEffects":false,"fixtureAttributeReads":fixture.readCount]
}
