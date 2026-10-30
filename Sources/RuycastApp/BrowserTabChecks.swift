import AppKit
import Carbon
import SwiftUI
import LauncherCore
import LauncherModels

private final class SafariWire:@unchecked Sendable {
    struct State:Sendable {
        var tabs=[BrowserTabs.Tab(window:101,index:1,title:"Project Notes",url:"https://example.test/notes"),BrowserTabs.Tab(window:101,index:2,title:"Project Notes",url:"https://example.test/other")]
        var identity=SystemServices.Identity(pid:4242,uid:getuid(),seconds:10,micros:0)
        var running=true,selected:Int32=1,sets=0,raises=0,activates=0,sends=0,consents=0
        var status:Int32=0,clock:TimeInterval=0,mutate=""
    }
    let state=LockedValue(State())
    var effects:BrowserTabs.Effects {
        var e=BrowserTabs.Effects()
        e.browser={self.state.read {$0.running ? BrowserTabs.Browser(identity:$0.identity):nil}}
        e.clock={self.state.read {$0.clock}}
        e.consent={_ in self.state.update {$0.consents+=1};return noErr}
        e.activate={_,_,eligible in guard eligible() else {return false};return self.state.update {s in s.activates+=1;if s.mutate=="activate" {s.tabs[1]=BrowserTabs.Tab(window:101,index:2,title:"Changed",url:s.tabs[1].url)};return true}}
        e.send={pid,event,flags,timeout in try self.send(pid,event,flags,timeout)}
        return e
    }
    private func send(_ pid:pid_t,_ event:NSAppleEventDescriptor,_ flags:UInt,_ timeout:TimeInterval)throws->NSAppleEventDescriptor {
        let c=BrowserTabs.code
        guard pid==4242,flags==BrowserTabs.flags,flags & UInt(kAEDoNotPromptForUserConsent) != 0,timeout>0,timeout<=8,
              event.attributeDescriptor(forKeyword:keyEventClassAttr)?.typeCodeValue==c("core"),
              event.attributeDescriptor(forKeyword:keyAddressAttr)?.descriptorType==typeKernelProcessID,
              let ref=event.paramDescriptor(forKeyword:c("----")),let selector=ref.forKeyword(c("seld")),
              let parent=ref.forKeyword(c("from")) else {throw LauncherError("Fixture transport flags/address/object invalid.")}
        let key=selector.typeCodeValue,eventID=event.attributeDescriptor(forKeyword:keyEventIDAttr)?.typeCodeValue
        let current=parent.forKeyword(c("seld"))?.typeCodeValue==c("cTab")
        let reply=NSAppleEventDescriptor(eventClass:c("aevt"),eventID:c("ansr"),targetDescriptor:nil,returnID:0,transactionID:0)
        func list(_ values:[NSAppleEventDescriptor])->NSAppleEventDescriptor {let out=NSAppleEventDescriptor.list();for (i,v) in values.enumerated() {out.insert(v,at:i+1)};return out}
        try state.update {s in
            s.sends+=1
            if s.status != 0 {reply.setParam(NSAppleEventDescriptor(int32:s.status),forKeyword:c("errn"));return}
            if s.mutate=="timeout" {s.clock=9}
            if eventID==c("setd") {
                guard let value=event.paramDescriptor(forKeyword:c("data")) else {throw LauncherError("Fixture set lacks data.")}
                if key==c("cTab") {
                    guard value.forKeyword(c("want"))?.typeCodeValue==c("bTab"),let index=value.forKeyword(c("seld"))?.int32Value else {throw LauncherError("Fixture attempted navigation instead of existing tab.")}
                    s.sets+=1;s.selected=index
                    if s.mutate=="set" {s.tabs[1]=BrowserTabs.Tab(window:101,index:2,title:"Changed",url:s.tabs[1].url)}
                } else if key==c("pidx") {s.raises+=1;if s.mutate=="raise" {s.tabs[1]=BrowserTabs.Tab(window:101,index:2,title:"Changed",url:s.tabs[1].url)}}
                else {throw LauncherError("Fixture unexpected Safari write.")}
            } else if eventID==c("getd") {
                let value:NSAppleEventDescriptor
                if key==c("ID  ") {value=list([NSAppleEventDescriptor(int32:101)])}
                else if current {
                    guard let tab=s.tabs.first(where:{$0.index==s.selected}) else {throw LauncherError("Closed fixture tab.")}
                    if key==c("pidx") {value=NSAppleEventDescriptor(int32:tab.index)}
                    else if key==c("pnam") {value=NSAppleEventDescriptor(string:tab.title)}
                    else if key==c("pURL") {value=NSAppleEventDescriptor(string:tab.url)}
                    else {throw LauncherError("Unexpected selected property.")}
                } else if key==c("pnam") {value=list(s.tabs.map {NSAppleEventDescriptor(string:$0.title)})}
                else if key==c("pURL") {value=list(s.tabs.map {NSAppleEventDescriptor(string:$0.url)})}
                else {throw LauncherError("Unexpected Safari read.")}
                reply.setParam(value,forKeyword:c("----"))
            } else {throw LauncherError("Unexpected Safari event.")}
        }
        return reply
    }
}

@MainActor func checkBrowserTabs(_ output:String?=nil)throws->[String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-tabs-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ condition:Bool,_ message:String)throws {if !condition {throw LauncherError(message)}}
    func pump() {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    func wait(_ condition:()->Bool)throws {let end=Date().addingTimeInterval(5);while !condition(),Date()<end {pump()};try require(condition(),"Safari fixture timed out.")}
    let activationResult=LockedValue<Bool?>(nil),activationEligible=LockedValue(false)
    let activationDeadline=ProcessInfo.processInfo.systemUptime+0.03
    DispatchQueue.global().async {
        let result=BrowserTabs.activate(4242,activationDeadline) {activationEligible.update {$0=true};return false}
        activationResult.update {$0=result}
    }
    // Keep this fixture's main queue occupied until the request has expired.
    Thread.sleep(forTimeInterval:0.06)
    try wait {activationResult.read {$0 != nil}}
    for _ in 0..<4 {pump()}
    try require(activationResult.read({$0})==false && !activationEligible.read({$0}),"Expired queued activation reached its effect.")
    let wire=SafariWire();var effects=SystemServices.Effects();effects.browserTabs=wire.effects
    let service=SystemServices(effects:effects),state=try AppState(root:root,headless:true,systemServices:service)
    var snapshot:SystemSearchResults?,error:Error?
    func search(_ query:String="")throws->SystemSearchResults {
        snapshot=nil;error=nil;service.search(mode:"tabs",query:query,previousPID:nil,session:SystemSearchSession()) {result in switch result {case .success(let value):snapshot=value;case .failure(let failure):error=failure}}
        try wait {snapshot != nil || error != nil};if let error {throw error};return snapshot!
    }
    let rows=try search();try require(rows.commands.count==2 && rows.commands[0].id != rows.commands[1].id && rows.tabs.count==2,"Safari discovery lost duplicate titles or URLs.")
    try require(rows.commands.allSatisfy({!$0.id.contains("example") && $0.target==$0.id}),"Safari metadata leaked into target.")
    let sends=wire.state.read {$0.sends}
    try require(try search("OTHER").commands.map(\.id)==[rows.commands[1].id] && wire.state.read({$0.sends})==sends,"Safari address filter or short cache failed.")
    var cliReply:[String:Any]?
    state.handle(["op":"search","mode":"tabs","query":"other"]) {cliReply=$0}
    try wait {cliReply != nil}
    let cliRows=cliReply?["result"] as? [[String:Any]] ?? []
    try require(cliReply?["ok"] as? Bool==true && cliRows.count==1 && (cliRows[0]["tab"] as? [String:Any])?["url"] as? String=="https://example.test/other","Safari CLI search/metadata missing.")
    try require(state.searchCommands("tabbladen").filter {$0.target=="search-safari-tabs"}.count==1,"Safari entry point missing/duplicated.")
    state.rememberServices(rows.commands)
    var reply:[String:Any]?
    func execute(_ command:Command)throws->[String:Any] {reply=nil;state.execute(command.id,dryRun:false) {reply=$0};try wait {reply != nil};return reply!}
    try require(try execute(rows.commands[1])["ok"] as? Bool==true && wire.state.read({$0.selected})==2,"Safari CLI selection did not reach exact captured slot.")
    try require(state.counts[rows.commands[1].id]==nil,"Safari opaque target was persisted in usage.")
    for mutation in ["close","move","title","duplicate","restart","set","raise","activate","timeout","denied"] {
        wire.state.update {$0=SafariWire.State()};service.invalidateBrowserCache();let captured=try search()
        wire.state.update {s in
            s.mutate=mutation
            switch mutation {
            case "close":s.tabs.removeLast()
            case "move":s.tabs.swapAt(0,1);s.tabs=s.tabs.enumerated().map {BrowserTabs.Tab(window:101,index:Int32($0.offset+1),title:$0.element.title,url:$0.element.url)}
            case "title":s.tabs[1]=BrowserTabs.Tab(window:101,index:2,title:"Changed",url:s.tabs[1].url)
            case "duplicate":s.tabs[1]=BrowserTabs.Tab(window:101,index:2,title:s.tabs[0].title,url:s.tabs[0].url)
            case "restart":s.identity=SystemServices.Identity(pid:4242,uid:getuid(),seconds:11,micros:0)
            case "denied":s.status = -1743
            default:break
            }
        }
        state.rememberServices(captured.commands)
        let response=try execute(captured.commands[1]),after=wire.state.read {$0}
        try require(response["ok"] as? Bool==false,"Safari mutation falsely succeeded: "+mutation)
        if !["set","raise","activate"].contains(mutation) {try require(after.sets==0 && after.activates==0,"Safari stale/ambiguous result caused a write: "+mutation)}
        if mutation=="set" {try require(after.raises==0 && after.activates==0,"Safari selection mismatch caused foreground effects.")}
        if mutation=="raise" {try require(after.activates==0,"Safari raised mismatch activated app.")}
    }
    wire.state.update {$0=SafariWire.State()};service.invalidateBrowserCache();let clean=try search();state.rememberServices(clean.commands)
    wire.state.update {$0.clock=31}
    try require(try execute(clean.commands[0])["ok"] as? Bool==false,"Safari expired target succeeded.")
    wire.state.update {$0=SafariWire.State();$0.running=false};service.invalidateBrowserCache()
    do {_=try search();throw LauncherError("Missing Safari succeeded.")} catch {try require(error.localizedDescription.contains("Open Safari"),"Missing Safari error missing.")}
    wire.state.update {$0=SafariWire.State();$0.tabs=[]};service.invalidateBrowserCache();try require(try search().commands.isEmpty,"Empty Safari failed.")
    wire.state.update {$0=SafariWire.State();$0.tabs=Array(repeating:$0.tabs[0],count:513)};service.invalidateBrowserCache()
    do {_=try search();throw LauncherError("Safari cap failed.")} catch {try require(error.localizedDescription.contains("512"),"Safari tab cap error missing.")}
    wire.state.update {$0=SafariWire.State()};service.invalidateBrowserCache();let normal=try search()
    state.mode="tabs";try wait {!state.tabSearchBusy && state.results.count==2}
    try require(wire.state.read({$0.consents})==0,"Background search/CLI requested consent.")
    var consent=false;service.requestBrowserConsent {consent=true};try wait {consent};try require(wire.state.read({$0.consents})==1,"Explicit consent seam missing.")
    // Cancelling a queued consent must prevent the OS request; a completed old
    // request must not refresh a newly opened palette session.
    let consentQueue=DispatchQueue(label:"safari.fixture.consent"),gate=DispatchSemaphore(value:0),drained=LockedValue(false)
    consentQueue.async {gate.wait()}
    var consentEffects=wire.effects;consentEffects.consentQueue=consentQueue
    let consentService=BrowserTabs(effects:consentEffects),oldCallback=LockedValue(false)
    let priorConsent=wire.state.read {$0.consents}
    consentService.requestConsent {oldCallback.update {$0=true}}
    consentService.cancelConsent();gate.signal();consentQueue.async {drained.update {$0=true}}
    try wait {drained.read {$0}}
    try require(wire.state.read({$0.consents})==priorConsent && !oldCallback.read({$0}),"Queued cancelled consent still requested permission.")
    let entered=LockedValue(false),release=DispatchSemaphore(value:0)
    consentEffects.consent={_ in entered.update {$0=true};release.wait();return noErr}
    let lateService=BrowserTabs(effects:consentEffects)
    lateService.requestConsent {oldCallback.update {$0=true}};try wait {entered.read {$0}}
    lateService.cancelConsent();release.signal();drained.update {$0=false};consentQueue.async {drained.update {$0=true}}
    try wait {drained.read {$0}};for _ in 0..<5 {pump()}
    try require(!oldCallback.read({$0}),"Cancelled old consent refreshed a later session.")
    state.commandLog.flush();let logs=try String(contentsOf:state.commandLog.url,encoding:.utf8)
    try require(!logs.contains("example.test") && !logs.contains("Project Notes"),"Safari title/address reached persisted log.")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        let controller=PaletteController(state:state);state.paletteVisible=true;state.showToast=nil
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        func field(_ view:NSView)->SearchInput? {if let value=view as? SearchInput {return value};return view.subviews.compactMap {field($0)}.first}
        guard let host=controller.panel.contentView,let input=field(host) else {throw LauncherError("Safari search field missing.")}
        try require(input.accessibilityLabel()=="Search Safari tabs…","Safari accessible search name stale.")
        state.selection=1;controller.restoreSearchFocus();for _ in 0..<5 {pump()}
        guard let editor=controller.panel.firstResponder as? NSTextView else {throw LauncherError("Safari search editor missing.")}
        let before=wire.state.read {$0.activates}
        editor.keyDown(with:NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:controller.panel.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!)
        try wait {!state.busy};try require(wire.state.read({$0.activates})==before+1 && !controller.panel.isVisible,"Safari native Return lost selection or hid cancellation killed execution.")
        for dark in [false,true] {for width in [600,760] {for variant in ["normal","duplicate","long","empty","error","loading"] {
            var rows=normal.commands,metadata=normal.tabs
            if variant=="long" {rows[0].title="A Very Long Safari Tab Title for Project Notes and a Daily Workspace Review";metadata[rows[0].id]=BrowserTabInfo(url:"https://example.test/projects/a-long-address-for-workspace-notes-and-daily-review")}
            if ["empty","error","loading"].contains(variant) {rows=[]}
            state.message=variant=="error" ? BrowserTabs.permission:"";state.paletteSession.nativeSearchBusy=variant=="loading"
            state.applySystemResults(rows,resources:[:],preserveSelection:false,tabs:metadata);state.selection=0
            state.config.theme=dark ? "dark":"light";controller.panel.setContentSize(NSSize(width:width,height:490));controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
            guard let view=controller.panel.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Safari render missing.")}
            view.cacheDisplay(in:view.bounds,to:bitmap);guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Safari PNG missing.")}
            try data.write(to:directory.appendingPathComponent("tabs-\(dark ? "dark":"light")-\(width)-\(variant).png"))
        }}}
        controller.hide();try require(!controller.panel.isVisible,"Safari fixture showed its panel.")
    }
    let main=try checkMainBrowserTabs(output.map {$0+"/main"})
    return ["mainSearch":main,"ok":true,"actualNativeAETransportShapeAndNoPromptFlags":true,"capturedSnapshotGuardAndAmbiguousDuplicateRejection":true,"staleAndPostSelectionMutationsRejected":true,"cliAndNativeReturn":output != nil,"boundedCacheTargetsAndMetadata":true,"noPrivateMetadataInLogs":true,"realSafariActivation":false,"immutableTabIdentity":false,"atomicSelection":false]
}

@MainActor private func checkMainBrowserTabs(_ output:String?)throws->[String:Any] {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-main-tabs-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ condition:Bool,_ message:String)throws {if !condition {throw LauncherError(message)}}
    func pump() {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.004))}
    func wait(_ condition:()->Bool)throws {let end=Date().addingTimeInterval(4);while !condition(),Date()<end {pump()};try require(condition(),"Main tab fixture timed out.")}
    let wire=SafariWire(),hold=LockedValue<DispatchSemaphore?>(nil),entered=LockedValue(false),consentHold=LockedValue<DispatchSemaphore?>(nil),consentEntered=LockedValue(false)
    var effects=SystemServices.Effects();effects.mainTabBudget=0.3;effects.browserTabs=wire.effects
    effects.browserTabs.clock={ProcessInfo.processInfo.systemUptime}
    let send=effects.browserTabs.send
    effects.browserTabs.send={pid,event,flags,timeout in
        let gate=hold.update {gate -> DispatchSemaphore? in let result=gate;gate=nil;return result}
        if let gate {entered.update {$0=true};gate.wait()}
        return try send(pid,event,flags,timeout)
    }
    let consent=effects.browserTabs.consent
    effects.browserTabs.consent={pid in
        let gate=consentHold.update {value -> DispatchSemaphore? in let result=value;value=nil;return result}
        if let gate {consentEntered.update {$0=true};gate.wait()}
        return consent(pid)
    }
    let service=SystemServices(effects:effects),state=try AppState(root:root,headless:true,systemServices:service)
    state.config.commands=[Command(id:"fixture.notes",title:"Project Notes",kind:"builtin",target:"settings"),Command(id:"fixture.local",title:"Local Command",kind:"builtin",target:"reload-config")];try Files.save(state.config,to:state.configURL);try state.reload()
    try require(wire.state.read({$0.sends})==0,"Main tabs polled during startup.")
    state.mainTabSearchEnabled=true;state.paletteVisible=true;state.refreshMainSearch()
    try require(state.results.contains(where:{$0.id=="fixture.local"}) && !state.results.first!.target.hasPrefix("system.tab:"),"Local search waited for Safari.")
    try wait {state.results.first?.target.hasPrefix("system.tab:root.")==true}
    try require(state.results.contains(where:{$0.id=="fixture.local"}) && state.browserTabInfo.count==2,"Main merge lost local commands or tab metadata.")
    state.query="other";try wait {state.results.first?.target.hasPrefix("system.tab:root.")==true}
    try require(state.browserTabInfo[state.results[0].id]?.url=="https://example.test/other","Main address matching failed.")
    var reply:[String:Any]?
    state.handle(["op":"search","query":"other","limit":1]) {reply=$0};try wait {reply != nil}
    let cliRows=reply?["result"] as? [[String:Any]] ?? []
    try require(cliRows.count==1 && (cliRows[0]["tab"] as? [String:Any])?["url"] as? String=="https://example.test/other","Main CLI lost tab matching/metadata/limit.")
    reply=nil
    state.execute(state.results[0].id,dryRun:false) {reply=$0};try wait {reply != nil}
    try require(reply?["ok"] as? Bool==true && wire.state.read({$0.selected})==2,"Root target was sent to the wrong store.")
    // Root and dedicated targets have separate stores and never execution fallback.
    var dedicated:SystemSearchResults?
    service.search(mode:"tabs",query:"",previousPID:nil,session:SystemSearchSession()) {result in dedicated=try? result.get()}
    try wait {dedicated != nil};try require(dedicated!.commands.allSatisfy {!$0.id.hasPrefix("system.tab:root.")},"Dedicated namespace changed.")
    state.rememberServices(dedicated!.commands);reply=nil
    state.execute(dedicated!.commands[0].id,dryRun:false) {reply=$0};try wait {reply != nil}
    try require(reply?["ok"] as? Bool==true && wire.state.read({$0.selected})==1,"Dedicated target went to root store.")
    func pause()throws->DispatchSemaphore {
        entered.update {$0=false};let gate=DispatchSemaphore(value:0);hold.update {$0=gate}
        service.invalidateBrowserCache();state.query="";state.refreshMainSearch();try wait {entered.read {$0}};return gate
    }
    var gate=try pause();state.selection=state.results.firstIndex {$0.id=="fixture.local"}!
    gate.signal();try wait {state.results.first?.target.hasPrefix("system.tab:root.")==true}
    try require(state.selectedCommand?.id=="fixture.local","Late tabs moved the navigated selection.")
    gate=try pause();state.selection=0;let actionID=state.selectedCommand?.id;state.actionsOpen=true;gate.signal()
    for _ in 0..<30 {pump()}
    try require(state.selectedCommand?.id==actionID && !state.results.first!.target.hasPrefix("system.tab:"),"Late tabs retargeted Actions.")
    state.actionsOpen=false
    gate=try pause();state.paletteVisible=false;state.cancelNativeSearch();gate.signal()
    for _ in 0..<30 {pump()}
    try require(!state.results.first!.target.hasPrefix("system.tab:"),"Hidden palette received tabs.")
    state.paletteVisible=true;state.refreshMainSearch();try wait {state.results.first?.target.hasPrefix("system.tab:root.")==true}
    gate=try pause();state.mode="snippets";gate.signal();for _ in 0..<30 {pump()}
    try require(state.mode=="snippets" && !state.results.contains(where:{$0.target.hasPrefix("system.tab:")}),"Old tab query crossed a mode change.")
    state.mode="commands";state.query="";state.refreshMainSearch();try wait {state.results.first?.target.hasPrefix("system.tab:root.")==true}
    // A blocked provider cannot hold the main CLI response past its root budget.
    gate=try pause();var firstCallbacks=0,secondCallbacks=0
    let started=ProcessInfo.processInfo.systemUptime;reply=nil
    state.handle(["op":"search","query":"Local"]) {reply=$0;secondCallbacks+=1}
    try wait {reply != nil}
    try require(ProcessInfo.processInfo.systemUptime-started<0.9 && reply?["ok"] as? Bool==true && (reply?["result"] as? [[String:Any]])?.contains(where:{$0["id"] as? String=="fixture.local"})==true,"Busy root worker blocked CLI local results.")
    gate.signal();for _ in 0..<30 {pump()};try require(secondCallbacks==1,"Late provider delivered CLI twice.")
    // Superseding a query must not let its old timer cancel the latest consumer.
    service.invalidateBrowserCache();entered.update {$0=false};gate=DispatchSemaphore(value:0);hold.update {$0=gate}
    service.mainTabs(query:"notes") {_ in firstCallbacks+=1};try wait {entered.read {$0}}
    service.mainTabs(query:"other") {snapshot in if snapshot.commands.count==1 {secondCallbacks+=1}}
    gate.signal();try wait {secondCallbacks==2};for _ in 0..<85 {pump()}
    try require(firstCallbacks==1 && secondCallbacks==2,"Old timer cancelled or duplicated a newer query.")
    wire.state.update {$0.status = -1743};service.invalidateBrowserCache();state.refreshMainSearch();for _ in 0..<30 {pump()}
    try require(state.results.contains(where:{$0.id=="fixture.local"}) && state.message.isEmpty,"Permission denial broke main search.")
    let failedReads=wire.state.read {$0.sends};state.query="Local";for _ in 0..<20 {pump()}
    try require(wire.state.read({$0.sends})==failedReads,"Denied root reads were not throttled.")
    wire.state.update {$0.status=0;$0.running=false};service.invalidateBrowserCache();state.query="";state.refreshMainSearch();for _ in 0..<20 {pump()}
    try require(state.results.contains(where:{$0.id=="fixture.local"}) && state.message.isEmpty,"Missing Safari broke main search.")
    wire.state.update {s in s=SafariWire.State();s.tabs=(1...60).map {BrowserTabs.Tab(window:101,index:Int32($0),title:"Tab \($0)",url:"https://example.test/\($0)")}}
    service.invalidateBrowserCache();state.refreshMainSearch();try wait {state.browserTabInfo.count==5}
    state.query="Tab";try wait {state.browserTabInfo.count==50}
    try require(state.results.filter {$0.target.hasPrefix("system.tab:")}.count==50,"Root result cap changed.")
    try require(wire.state.read({$0.consents})==0,"Main discovery or CLI requested consent.")
    state.query="";try wait {!state.mainSearchBusy}
    consentEntered.update {$0=false};gate=DispatchSemaphore(value:0);consentHold.update {$0=gate}
    state.requestMainTabAccess();try wait {consentEntered.read {$0}}
    state.selection=state.results.firstIndex {$0.id=="fixture.local"}!
    let consentSelection=state.selectedCommand?.id,consentReads=wire.state.read {$0.sends}
    gate.signal();try wait {wire.state.read({$0.sends})>consentReads && !state.mainSearchBusy}
    try require(state.selectedCommand?.id==consentSelection,"Late consent refresh reset a navigated row.")
    consentEntered.update {$0=false};gate=DispatchSemaphore(value:0);consentHold.update {$0=gate}
    state.requestMainTabAccess();try wait {consentEntered.read {$0}}
    state.actionsOpen=true;let consentAction=state.selectedCommand?.id,actionReads=wire.state.read {$0.sends}
    gate.signal();try wait {wire.state.read {$0.consents}==2};for _ in 0..<20 {pump()}
    try require(state.selectedCommand?.id==consentAction && wire.state.read({$0.sends})==actionReads,"Late consent retargeted Actions.")
    state.actionsOpen=false
    // Render only synthetic data; all ordinary headless fixtures remain local-only.
    wire.state.update {$0=SafariWire.State()};service.invalidateBrowserCache();state.refreshMainSearch();try wait {state.browserTabInfo.count==2}
    state.selection=0;consentEntered.update {$0=false};gate=DispatchSemaphore(value:0);consentHold.update {$0=gate}
    let defaultReads=wire.state.read {$0.sends}
    state.requestMainTabAccess();try wait {consentEntered.read {$0}};gate.signal()
    try wait {wire.state.read({$0.sends})>defaultReads && !state.mainSearchBusy}
    try require(state.selection==0 && state.selectedCommand?.target.hasPrefix("system.tab:root.")==true,"Consent left the untouched default tab unselected.")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        let controller=PaletteController(state:state);state.paletteVisible=true;state.showToast=nil
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        let tabs=state.results.filter {$0.target.hasPrefix("system.tab:")},metadata=state.browserTabInfo,locals=state.results.filter {!$0.target.hasPrefix("system.tab:")}
        func field(_ view:NSView)->SearchInput? {if let value=view as? SearchInput {return value};return view.subviews.compactMap {field($0)}.first}
        guard let host=controller.panel.contentView,let input=field(host) else {throw LauncherError("Main native search field missing.")}
        try require(input.accessibilityLabel()=="Search apps, tabs or commands…","Main accessible search name stale.")
        state.selection=0;controller.restoreSearchFocus();for _ in 0..<5 {pump()}
        guard let editor=controller.panel.firstResponder as? NSTextView else {throw LauncherError("Main native editor missing.")}
        let activations=wire.state.read {$0.activates}
        editor.keyDown(with:NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:controller.panel.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!)
        try wait {wire.state.read({$0.activates})==activations+1 && !state.busy}
        try require(!state.paletteVisible && !controller.panel.isVisible,"Main Return did not hide or execute exactly once.")
        state.mainTabSearchEnabled=false
        for dark in [false,true] {for width in [600,760] {for variant in ["normal","typed","long","no-tabs","loading","empty"] {
            state.query=variant=="typed" ? "Project":(["loading","empty"].contains(variant) ? "unmatched-address":"")
            var rows=tabs,info=metadata
            let matchingLocals=["loading","empty"].contains(variant) ? []:(variant=="typed" ? locals.filter {$0.title.contains("Project")}:locals)
            if variant=="long",!rows.isEmpty {rows[0].title="A Very Long Safari Tab Title for Project Notes and a Daily Workspace Review";info[rows[0].id]=BrowserTabInfo(url:"https://example.test/projects/a-long-address-for-workspace-notes-and-daily-review")}
            if ["no-tabs","loading","empty"].contains(variant) {rows=[];info=[:]}
            state.paletteSession.nativeSearchBusy=variant=="loading"
            state.applySystemResults(AppState.mergeMainResults(local:matchingLocals,tabs:rows,limit:100),resources:[:],preserveSelection:false,tabs:info);state.selection=0
            state.config.theme=dark ? "dark":"light";controller.panel.setContentSize(NSSize(width:width,height:490));controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
            guard let view=controller.panel.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Root tab render missing.")}
            view.cacheDisplay(in:view.bounds,to:bitmap);guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Root PNG missing.")}
            try data.write(to:directory.appendingPathComponent("main-\(dark ? "dark":"light")-\(width)-\(variant).png"))
        }}}
        controller.hide();try require(!controller.panel.isVisible,"Root fixture showed a visible panel.")
    }
    state.commandLog.flush()
    let log=try String(contentsOf:state.commandLog.url,encoding:.utf8);try require(!log.contains("example.test") && !log.contains("Project Notes"),"Main tab metadata reached logs.")
    return ["ok":true,"localResultsImmediate":true,"rootAndDedicatedOwners":true,"selectionAndActionsStable":true,"hideModeAndStaleQueries":true,"busyWorkerDeadlineAndOnceOnly":true,"failureThrottleAndFallback":true,"caps5And50":true,"explicitConsentOnly":true,"delayedConsentSelectionStable":true,"mainNativeReturn":output != nil,"realSafariReadOrActivation":false]
}
