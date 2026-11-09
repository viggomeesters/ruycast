import AppKit
import SwiftUI
import ApplicationServices
import LauncherCore
import LauncherModels

/// Real handles for nonexistent PIDs; a lock owns every simulated AX/process read.
private final class WindowFixture:@unchecked Sendable {
    private struct Node {var owner:pid_t;var attributes:[String:CFTypeRef];var members:[AXUIElement]=[];var unreadable=false}
    private let lock=NSLock()
    private var nodes:[CFHashCode:Node]=[:]
    private var apps:[WindowSearchApplication]=[]
    private var handles:[pid_t:AXUIElement]=[:]
    private var identities:[pid_t:SystemServices.Identity]=[:]
    private var next:pid_t=840000
    private var clock:TimeInterval=0
    private var step:TimeInterval=0.000001
    private var reads=0,lists=0,maxLimit=0
    func addApp(_ name:String,_ pid:pid_t)->AXUIElement {
        lock.lock();defer {lock.unlock()};next+=1
        let handle=AXUIElementCreateApplication(next)
        nodes[CFHash(handle)]=Node(owner:pid,attributes:[kAXRoleAttribute:kAXApplicationRole as CFString]);handles[pid]=handle
        identities[pid]=SystemServices.Identity(pid:pid,uid:getuid(),seconds:11,micros:22)
        apps.append(WindowSearchApplication(pid:pid,name:name));return handle
    }
    func addWindow(_ app:AXUIElement,_ title:String,_ minimized:Bool?=false)->AXUIElement {
        lock.lock();defer {lock.unlock()};next+=1
        let handle=AXUIElementCreateApplication(next),owner=nodes[CFHash(app)]!.owner
        var values:[String:CFTypeRef]=[kAXRoleAttribute:kAXWindowRole as CFString,kAXTitleAttribute:title as CFString]
        if let minimized {values[kAXMinimizedAttribute]=minimized ? kCFBooleanTrue:kCFBooleanFalse}
        nodes[CFHash(handle)]=Node(owner:owner,attributes:values);nodes[CFHash(app)]!.members.append(handle);return handle
    }
    func applications()->[WindowSearchApplication] {lock.lock();defer {lock.unlock()};return apps}
    func application(_ pid:pid_t)->AXUIElement {lock.lock();defer {lock.unlock()};return handles[pid]!}
    func identity(_ pid:pid_t)->SystemServices.Identity? {lock.lock();defer {lock.unlock()};return identities[pid]}
    func setIdentity(_ pid:pid_t,_ value:SystemServices.Identity?) {lock.lock();defer {lock.unlock()};identities[pid]=value}
    func owner(_ element:AXUIElement)->pid_t? {lock.lock();defer {lock.unlock()};return nodes[CFHash(element)]?.owner}
    func setOwner(_ element:AXUIElement,_ pid:pid_t) {lock.lock();defer {lock.unlock()};nodes[CFHash(element)]?.owner=pid}
    func read(_ element:AXUIElement,_ key:String)->CFTypeRef? {lock.lock();defer {lock.unlock()};reads+=1;guard let node=nodes[CFHash(element)],!node.unreadable else {return nil};return node.attributes[key]}
    func members(_ element:AXUIElement,_ limit:Int)->[AXUIElement]? {lock.lock();defer {lock.unlock()};lists+=1;maxLimit=max(maxLimit,limit);guard let node=nodes[CFHash(element)],!node.unreadable else {return nil};return Array(node.members.prefix(limit))}
    func setMembers(_ app:AXUIElement,_ members:[AXUIElement]) {lock.lock();defer {lock.unlock()};nodes[CFHash(app)]?.members=members}
    func set(_ element:AXUIElement,_ key:String,_ value:CFTypeRef?) {lock.lock();defer {lock.unlock()};nodes[CFHash(element)]?.attributes[key]=value}
    func unreadable(_ element:AXUIElement,_ value:Bool) {lock.lock();defer {lock.unlock()};nodes[CFHash(element)]?.unreadable=value}
    func time()->TimeInterval {lock.lock();defer {lock.unlock()};clock+=step;return clock}
    func timeStep(_ value:TimeInterval) {lock.lock();defer {lock.unlock()};step=value}
    var counters:(reads:Int,lists:Int,maxLimit:Int) {lock.lock();defer {lock.unlock()};return (reads,lists,maxLimit)}
}

@MainActor func checkWindowSearch(_ output:String?=nil) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-windows-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    func wait(_ ready:()->Bool) throws {let end=Date().addingTimeInterval(4);while !ready(),Date()<end {pump()};try require(ready(),"Window fixture timed out.")}
    let fixture=WindowFixture(),trusted=LockedValue(true),pid:pid_t=730001
    let app=fixture.addApp("Editor Fixture",pid),otherApp=fixture.addApp("Browser Fixture",pid+1)
    let first=fixture.addWindow(app,"Project Notes"),duplicate=fixture.addWindow(app,"Project Notes"),minimized=fixture.addWindow(app,"Draft",true),unknown=fixture.addWindow(app,"Inspector",nil),untitled=fixture.addWindow(otherApp,"",nil)
    let identity=fixture.identity(pid)!,all=[first,duplicate,minimized,unknown]
    var activated=[pid_t](),raised=[CFHashCode](),restored=[CFHashCode](),afterActivate:(()->Void)?,afterRestore:(()->Void)?
    let restoreStatus=LockedValue(AXError.success),raiseStatus=LockedValue(AXError.success)
    var effects=SystemServices.Effects()
    effects.trusted={trusted.read {$0}};effects.identity={fixture.identity($0)}
    effects.windowApplications={fixture.applications()};effects.windowApplication={fixture.application($0)}
    effects.windowAttribute={fixture.read($0,$1)};effects.windowList={fixture.members($0,$1)}
    effects.windowElementPID={fixture.owner($0)};effects.windowClock={fixture.time()}
    effects.activate={activated.append($0);afterActivate?();return true}
    effects.restoreWindow={element in restored.append(CFHash(element));if restoreStatus.read({$0}) == .success {fixture.set(element,kAXMinimizedAttribute,kCFBooleanFalse)};afterRestore?();return restoreStatus.read {$0}}
    effects.raise={raised.append(CFHash($0));return raiseStatus.read {$0}}
    let service=SystemServices(effects:effects)
    func search(_ query:String="",using searcher:SystemServices?=nil) throws -> SystemSearchResults {
        var result:Result<SystemSearchResults,Error>?
        (searcher ?? service).search(mode:"windows",query:query,previousPID:999,session:SystemSearchSession()) {result=$0}
        try wait {result != nil};return try result!.get()
    }
    let snapshot=try search()
    try require(snapshot.commands.count==5 && snapshot.windows.count==5 && raised.isEmpty && activated.isEmpty && restored.isEmpty,"Window enumeration changed the desktop or lost windows.")
    try require(snapshot.commands.contains {$0.title=="Untitled Window"} && snapshot.windows.values.filter {$0.minimized==nil}.count==2,"Unsupported minimized state was invented or untitled windows were lost.")
    try require(try search("editor").commands.count==4 && search("project").commands.count==2,"Window app/title search failed.")
    let targets=snapshot.commands.filter {$0.title=="Project Notes"};try require(targets.count==2 && targets[0].id != targets[1].id,"Duplicate window titles share an action token.")
    let target=targets[0].target
    let newer=try search("project");try require(newer.commands.allSatisfy {$0.id != target} && snapshot.windows[target]?.application=="Editor Fixture","A later search retargeted an old window snapshot.")
    fixture.setMembers(app,[duplicate])
    try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty && raised.isEmpty,"A closed window was replaced by a same-title window.")
    fixture.setMembers(app,all);fixture.set(first,kAXRoleAttribute,kAXButtonRole as CFString)
    try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty,"A non-window was activated.")
    fixture.set(first,kAXRoleAttribute,kAXWindowRole as CFString);fixture.setOwner(first,pid+1)
    try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty,"A foreign-process AX element was activated.")
    fixture.setOwner(first,pid);fixture.unreadable(app,true)
    try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty,"Unreadable app membership was accepted.")
    fixture.unreadable(app,false);fixture.setIdentity(pid,SystemServices.Identity(pid:pid,uid:getuid(),seconds:12,micros:22))
    try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty,"A reused PID was activated.")
    fixture.setIdentity(pid,identity);afterActivate={fixture.setMembers(app,[duplicate])}
    try require(service.execute(target:target)["ok"] as? Bool==false && raised.isEmpty,"A window closed during activation was raised.")
    afterActivate=nil;fixture.setMembers(app,all);activated=[]
    afterActivate={fixture.set(first,kAXTitleAttribute,"Changed" as CFString)}
    try require(service.execute(target:target)["ok"] as? Bool==false && raised.isEmpty,"A changed window title was raised.")
    afterActivate=nil;fixture.set(first,kAXTitleAttribute,"Project Notes" as CFString);activated=[]
    trusted.update {$0=false};try require(service.execute(target:target)["ok"] as? Bool==false && activated.isEmpty,"Window permission was bypassed.")
    trusted.update {$0=true}
    afterActivate={trusted.update {$0=false}}
    try require(service.execute(target:target)["ok"] as? Bool==false && raised.isEmpty,"Permission revoked during activation was ignored.")
    afterActivate={fixture.setIdentity(pid,nil)};trusted.update {$0=true}
    try require(service.execute(target:target)["ok"] as? Bool==false && raised.isEmpty,"App exit during activation was ignored.")
    afterActivate=nil;fixture.setIdentity(pid,identity);activated=[]
    try require(service.execute(target:target,previousPID:1)["ok"] as? Bool==true && raised==[CFHash(first)] && activated==[pid] && restored.isEmpty,"Cross-app switching did not use its captured window.")
    try require(service.execute(target:targets[1].target)["ok"] as? Bool==true && raised.last==CFHash(duplicate),"Duplicate title switched to the wrong window.")
    let mini=snapshot.commands.first {$0.title=="Draft"}!,inspector=snapshot.commands.first {$0.title=="Inspector"}!
    raised=[];restored=[];restoreStatus.update {$0 = .failure}
    try require(service.execute(target:mini.target)["ok"] as? Bool==false && raised.isEmpty,"Failed restore still raised a minimized window.")
    restoreStatus.update {$0 = .success};afterRestore={fixture.set(minimized,kAXMinimizedAttribute,kCFBooleanTrue)}
    try require(service.execute(target:mini.target)["ok"] as? Bool==false && raised.isEmpty,"Restore status was trusted without minimized readback.")
    afterRestore={fixture.setMembers(app,[first,duplicate,unknown])}
    try require(service.execute(target:mini.target)["ok"] as? Bool==false && raised.isEmpty,"A closed restored window was raised.")
    afterRestore=nil;fixture.setMembers(app,all);fixture.set(minimized,kAXMinimizedAttribute,kCFBooleanTrue);raised=[];restored=[]
    try require(service.execute(target:mini.target)["ok"] as? Bool==true && raised==[CFHash(minimized)] && restored==[CFHash(minimized)],"Minimized window was not restored then raised.")
    raised=[];restored=[]
    try require(service.execute(target:inspector.target)["ok"] as? Bool==true && raised==[CFHash(unknown)] && restored.isEmpty,"Unknown minimized state caused a restore write.")
    raiseStatus.update {$0 = .cannotComplete}
    try require(service.execute(target:target)["ok"] as? Bool==false,"Failed raise was reported as success.")
    raiseStatus.update {$0 = .success};raised=[];activated=[]
    let state=try AppState(root:root,headless:true,systemServices:service)
    state.foregroundPID={nil};state.previousPID=1
    let retainedCounts=["fixture.persisted":7];state.counts=retainedCounts;try Files.save(retainedCounts,to:root.appendingPathComponent("usage.json"))
    let usage=try Data(contentsOf:root.appendingPathComponent("usage.json"));try state.searchRanking.recordSuccess("fixture.persisted")
    let ranking=state.searchRanking.entries,rankingBytes=try Data(contentsOf:root.appendingPathComponent("search-ranking.json"))
    try require(state.searchCommands("windows").filter {$0.target=="switch-windows"}.count==1,"Window search has no unique discoverable entry point.")
    var reply:[String:Any]?
    state.handle(["op":"search","mode":"windows","query":"Project"]) {reply=$0};try wait {reply != nil}
    let rows=reply?["result"] as? [[String:Any]] ?? []
    try require(rows.count==2 && (rows[0]["window"] as? [String:Any])?["application"] as? String=="Editor Fixture" && rows[0]["hotkey"]==nil,"Window CLI lost snapshot metadata or invented a binding.")
    reply=nil;state.handle(["op":"execute","id":rows[0]["id"] as! String]) {reply=$0};try wait {reply != nil}
    try require(reply?["ok"] as? Bool==true && raised==[CFHash(first)],"Window CLI did not raise its selected captured handle.")
    try require(state.counts==retainedCounts && Data(contentsOf:root.appendingPathComponent("usage.json"))==usage,"Window UUID was persisted as a reusable command.")
    try require(state.analytics.snapshot.commands.count==1 && state.analytics.snapshot.commands[AnalyticsStore.commandKey("system.window")] != nil,"Window UUID displaced per-command analytics.")
    let large=WindowFixture()
    for i in 0..<205 {let a=large.addApp("Large \(i)",740000+pid_t(i));for j in 0..<55 {_=large.addWindow(a,"Window \(j)")}}
    var bounded=effects;bounded.windowApplications={large.applications()};bounded.windowApplication={large.application($0)};bounded.identity={large.identity($0)};bounded.windowAttribute={large.read($0,$1)};bounded.windowList={large.members($0,$1)};bounded.windowElementPID={large.owner($0)};bounded.windowClock={large.time()}
    let boundedService=SystemServices(effects:bounded)
    try require(try search(using:boundedService).commands.count==100 && large.counters.maxLimit==50,"Window result/native range cap failed.")
    let before=large.counters
    try require(try search("not-present",using:boundedService).commands.isEmpty && large.counters.lists-before.lists<=200,"Window app traversal cap failed.")
    large.timeStep(1);let readsBefore=large.counters.reads
    try require(try search(using:boundedService).commands.isEmpty && large.counters.reads-readsBefore<=2,"Window monotonic deadline continued AX reads.")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        state.mode="commands";let controller=PaletteController(state:state);state.showToast=nil;state.paletteVisible=true
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        state.mode="windows";try wait {state.results.count==5};controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        func field(_ view:NSView)->SearchInput? {if let result=view as? SearchInput {return result};return view.subviews.compactMap {field($0)}.first}
        guard let host=controller.panel.contentView,let input=field(host) else {throw LauncherError("Window search field missing.")}
        try require(input.accessibilityLabel()=="Search windows…","Window native search label remained stale.")
        state.applySystemResults(snapshot.commands,resources:[:],preserveSelection:false,windows:snapshot.windows);state.selection=0
        controller.restoreSearchFocus();for _ in 0..<5 {pump()}
        guard let editor=controller.panel.firstResponder as? NSTextView else {throw LauncherError("Window native field editor did not acquire focus.")}
        raised=[];let event=NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:controller.panel.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!
        editor.keyDown(with:event);try wait {!state.busy}
        try require(raised==[CFHash(first)] && !controller.panel.isVisible,"Native Return did not switch to its captured fixture window.")
        try require(state.counts==retainedCounts && state.searchRanking.entries==ranking && Data(contentsOf:root.appendingPathComponent("search-ranking.json"))==rankingBytes,"Native window Return changed durable usage/recency.")
        state.refreshNativeResults();try require(state.windowSearchBusy,"Window refresh has no loading state.")
        state.cancelNativeSearch();try require(!state.windowSearchBusy,"Window cancellation retained loading.")
        state.refreshNativeResults();try wait {state.results.count==5 && !state.windowSearchBusy}
        state.query="missing window";try wait {state.results.isEmpty && !state.windowSearchBusy}
        trusted.update {$0=false};state.refreshNativeResults();try wait {!state.message.isEmpty}
        let denied=state.message;try require(denied.contains("System Settings") && denied.contains("Accessibility"),"Window denial lost its recovery path.")
        trusted.update {$0=true};state.query="";state.refreshNativeResults();try wait {state.results.count==5 && state.message.isEmpty}
        for dark in [false,true] {for width in [600,760] {for variant in ["normal","minimized","long","empty","denied","loading"] {
            var commands=snapshot.commands,windows=snapshot.windows
            if variant=="long" {commands[0].title="Project Notes with a Very Long Document Name and Unsaved Changes";windows[commands[0].id]=WindowActionInfo(application:"A Long Application Name",title:commands[0].title,minimized:false)}
            if ["empty","denied","loading"].contains(variant) {commands=[];windows=[:]}
            state.message=variant=="denied" ? denied:"";state.paletteSession.nativeSearchBusy=variant=="loading"
            state.applySystemResults(commands,resources:[:],preserveSelection:false,windows:windows);state.selection=variant=="minimized" ? 2:0
            state.config.theme=dark ? "dark":"light";controller.panel.setContentSize(NSSize(width:width,height:490));controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
            guard let view=controller.panel.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Window render missing.")}
            view.cacheDisplay(in:view.bounds,to:bitmap);guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Window PNG missing.")}
            try data.write(to:directory.appendingPathComponent("windows-\(dark ? "dark":"light")-\(width)-\(variant).png"))
        }}}
        controller.hide();try require(!controller.panel.isVisible,"Window fixture made its panel visible.")
    }
    return ["ok":true,"realDesktopEffects":false,"windowAppAndTitleSearch":true,"immutableMetadataAndDuplicateTargets":true,"closedWindowAndPIDReuseRejected":true,"prePostActivationAndPostRestoreValidation":true,"minimizedOptionalRestoreVerified":true,"boundedNativeRangesAndDeadline":true,"cliAndNativeReturnParity":output != nil,"noPersistentWindowUUIDs":true,"permissionRecoveryAndCancellation":true,"untitledFixture":CFHash(untitled)]
}
