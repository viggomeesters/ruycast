import AppKit
import Carbon
import LauncherCore
import LauncherModels

struct BrowserTabInfo:Codable,Sendable,Equatable {let url:String}

/// Safari offers window IDs, but no immutable tab ID. These are short-lived
/// position/title/URL snapshots. Identical replacement and get/set races remain
/// indistinguishable; never navigate or search for a replacement after mismatch.
final class BrowserTabs:@unchecked Sendable {
    struct Browser:Sendable,Equatable {let identity:SystemServices.Identity}
    struct Tab:Sendable,Equatable {let window:Int32;let index:Int32;let title:String;let url:String}
    struct Effects:Sendable {
        var browser:@Sendable ()->Browser? = {
            guard let app=NSRunningApplication.runningApplications(withBundleIdentifier:"com.apple.Safari").first,
                  !app.isTerminated,let url=app.bundleURL,
                  ["/Applications/Safari.app","/System/Applications/Safari.app","/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"].contains(url.path),
                  let identity=SystemServices.identity(app.processIdentifier),identity.uid==getuid() else {return nil}
            return Browser(identity:identity)
        }
        var clock:@Sendable ()->TimeInterval = {ProcessInfo.processInfo.systemUptime}
        var send:@Sendable (pid_t,NSAppleEventDescriptor,UInt,TimeInterval)throws->NSAppleEventDescriptor = {_,event,flags,timeout in
            try event.sendEvent(options:NSAppleEventDescriptor.SendOptions(rawValue:flags),timeout:timeout)
        }
        var activate:@Sendable (pid_t,TimeInterval,@escaping @Sendable ()->Bool)->Bool = BrowserTabs.activate
        var consentQueue=DispatchQueue(label:"ruycast.safari-consent",qos:.userInitiated)
        var consent:@Sendable (pid_t)->OSStatus = {pid in
            let address=NSAppleEventDescriptor(processIdentifier:pid)
            return AEDeterminePermissionToAutomateTarget(address.aeDesc,typeWildCard,typeWildCard,true)
        }
    }
    /// Never synchronously wait for the main queue. A queued activation expires
    /// before it can touch Safari; an already submitted OS activation is not revoked.
    static func activate(_ pid:pid_t,_ deadline:TimeInterval,_ eligible:@escaping @Sendable ()->Bool)->Bool {
        struct Activation:Sendable {var cancelled=false;var result:Bool?}
        let request=LockedValue(Activation())
        DispatchQueue.main.async {
            guard !request.read({$0.cancelled}),ProcessInfo.processInfo.systemUptime<deadline,eligible() else {request.update {$0.result=false};return}
            let ok=NSRunningApplication(processIdentifier:pid)?.activate(options:[]) ?? false
            request.update {$0.result=ok}
        }
        while ProcessInfo.processInfo.systemUptime<deadline {
            if let result=request.read({$0.result}) {return result}
            Thread.sleep(forTimeInterval:0.002)
        }
        request.update {$0.cancelled=true};return false
    }
    static let flags=UInt(kAEWaitReply|kAENeverInteract|kAEDontRecord|kAEDoNotPromptForUserConsent)
    static let expired="This tab changed or closed. Search Safari tabs again."
    static let permission="Allow Safari access in System Settings → Privacy & Security → Automation, then refresh."
    static func code(_ s:String)->OSType {s.utf8.reduce(0){($0<<8)|UInt32($1)}}
    static func all()->NSAppleEventDescriptor {
        var value=code("all ")
        return withUnsafeBytes(of:&value) {NSAppleEventDescriptor(descriptorType:code("abso"),data:Data($0))!}
    }
    static func object(_ cls:String,_ container:NSAppleEventDescriptor = .null(),_ form:String="indx",_ key:NSAppleEventDescriptor)->NSAppleEventDescriptor {
        let r=NSAppleEventDescriptor.record()
        r.setDescriptor(NSAppleEventDescriptor(typeCode:code(cls)),forKeyword:code("want"))
        r.setDescriptor(container,forKeyword:code("from"))
        r.setDescriptor(NSAppleEventDescriptor(enumCode:code(form)),forKeyword:code("form"))
        r.setDescriptor(key,forKeyword:code("seld"))
        return r.coerce(toDescriptorType:code("obj "))!
    }
    static func property(_ name:String,_ from:NSAppleEventDescriptor)->NSAppleEventDescriptor {object("prop",from,"prop",NSAppleEventDescriptor(typeCode:code(name)))}
    private struct Target {let browser:Browser;let tab:Tab;let time:TimeInterval}
    private let effects:Effects
    private let namespace:String
    private let targets=LockedValue<[String:Target]>([:])
    private struct ConsentState:Sendable {var epoch=0;var pending=false}
    private let consentState=LockedValue(ConsentState())
    // Cache belongs exclusively to SystemServices' serial worker.
    private var cache:(Browser,TimeInterval,[Command],[String:BrowserTabInfo])?
    init(effects:Effects=Effects(),namespace:String="") {self.effects=effects;self.namespace=namespace}
    func invalidate() {cache=nil}
    /// Only an explicit interactive entry calls this. The OS consent wait may be
    /// unbounded; it is separate from data deadlines and never holds the main queue.
    func requestConsent(completion:@escaping @MainActor @Sendable ()->Void) {
        let epoch=consentState.update {value -> Int? in
            guard !value.pending else {return nil};value.pending=true;return value.epoch
        }
        guard let epoch else {return}
        effects.consentQueue.async {
            defer {self.consentState.update {$0.pending=false}}
            guard self.consentState.read({$0.epoch==epoch}),let browser=self.effects.browser(),self.consentState.read({$0.epoch==epoch}) else {return}
            _=self.effects.consent(browser.identity.pid)
            DispatchQueue.main.async {if self.consentState.read({$0.epoch==epoch}) {completion()}}
        }
    }
    func cancelConsent() {consentState.update {$0.epoch+=1}}
    private func check(_ browser:Browser,_ ticket:SystemSearchSession.Ticket,_ deadline:TimeInterval)throws {
        guard ticket.current else {throw LauncherError("Safari search cancelled.")}
        guard effects.clock()<deadline else {throw LauncherError("Safari did not respond in time. Refresh to retry.")}
        guard effects.browser()==browser else {throw LauncherError("Safari closed or restarted. Open Safari, then search again.")}
    }
    private func send(_ id:String,_ browser:Browser,_ ticket:SystemSearchSession.Ticket,_ deadline:TimeInterval,_ ref:NSAppleEventDescriptor,value:NSAppleEventDescriptor?=nil)throws->NSAppleEventDescriptor {
        try check(browser,ticket,deadline)
        let event=NSAppleEventDescriptor(eventClass:Self.code("core"),eventID:Self.code(id),targetDescriptor:NSAppleEventDescriptor(processIdentifier:browser.identity.pid),returnID:AEReturnID(kAutoGenerateReturnID),transactionID:AETransactionID(kAnyTransactionID))
        event.setParam(ref,forKeyword:Self.code("----"));if let value {event.setParam(value,forKeyword:Self.code("data"))}
        let reply:NSAppleEventDescriptor
        do {reply=try effects.send(browser.identity.pid,event,Self.flags,max(0.001,deadline-effects.clock()))}
        catch {
            let status=(error as NSError).code
            throw LauncherError([-1743,-1744].contains(status) ? Self.permission:"Could not read Safari tabs. Refresh to retry.")
        }
        try check(browser,ticket,deadline)
        // The OS allocates the reply before this check; this is a retention cap.
        guard reply.data.count<=2_097_152 else {throw LauncherError("Safari tab list is too large to load.")}
        if let error=reply.paramDescriptor(forKeyword:Self.code("errn")),error.int32Value != 0 {
            throw LauncherError([-1743,-1744].contains(error.int32Value) ? Self.permission:Self.expired)
        }
        return reply.paramDescriptor(forKeyword:Self.code("----")) ?? .null()
    }
    private func window(_ id:Int32)->NSAppleEventDescriptor {Self.object("cwin",.null(),"ID  ",NSAppleEventDescriptor(int32:id))}
    private func strings(_ value:NSAppleEventDescriptor)throws->[String] {
        guard value.descriptorType==typeAEList,value.numberOfItems<=512 else {throw LauncherError("Too many Safari tabs to load. The limit is 512.")}
        var result=[String]()
        for index in 0..<value.numberOfItems {
            guard let item=value.atIndex(index+1),let text=item.stringValue,text.utf8.count<=8192 else {throw LauncherError("Could not read Safari tab metadata. Refresh to retry.")}
            result.append(text)
        }
        return result
    }
    private func tabs(_ id:Int32,_ browser:Browser,_ ticket:SystemSearchSession.Ticket,_ deadline:TimeInterval)throws->[Tab] {
        let ref=Self.object("bTab",window(id),"indx",Self.all())
        func read(_ key:String)throws->[String] {try strings(send("getd",browser,ticket,deadline,Self.property(key,ref)))}
        let titles=try read("pnam"),urls=try read("pURL")
        guard titles.count==urls.count,try read("pnam")==titles,try read("pURL")==urls else {throw LauncherError(Self.expired)}
        return titles.indices.map {Tab(window:id,index:Int32($0+1),title:titles[$0],url:urls[$0])}
    }
    func search(_ query:String,_ ticket:SystemSearchSession.Ticket,deadline requestedDeadline:TimeInterval?=nil)throws->SystemSearchResults {
        guard let browser=effects.browser() else {throw LauncherError("Open Safari to search its tabs.")}
        let now=effects.clock(),deadline=min(requestedDeadline ?? effects.clock()+8,effects.clock()+8)
        try check(browser,ticket,deadline)
        if let (previous,time,commands,metadata)=cache,previous==browser,now-time<1 {return try filter(commands,metadata,query,ticket,deadline)}
        let ref=Self.property("ID  ",Self.object("cwin",.null(),"indx",Self.all()))
        let ids=try send("getd",browser,ticket,deadline,ref)
        guard ids.descriptorType==typeAEList,ids.numberOfItems<=64 else {throw LauncherError("Too many Safari windows to load. The limit is 64.")}
        var rows=[Tab](),seen=Set<Int32>(),bytes=0
        for index in 0..<ids.numberOfItems {
            guard let value=ids.atIndex(index+1),value.descriptorType==typeSInt32 else {throw LauncherError(Self.expired)}
            let id=value.int32Value;guard seen.insert(id).inserted else {throw LauncherError(Self.expired)}
            let next=try tabs(id,browser,ticket,deadline);rows+=next
            bytes+=next.reduce(0){$0+$1.title.utf8.count+$1.url.utf8.count}
            guard rows.count<=512,bytes<=2_097_152 else {throw LauncherError("Too many Safari tabs to load. The limit is 512.")}
        }
        try check(browser,ticket,deadline)
        var commands=[Command](),metadata=[String:BrowserTabInfo](),nextTargets=[String:Target]()
        for tab in rows {
            let token="system.tab:"+namespace+UUID().uuidString.lowercased()
            commands.append(Command(id:token,title:tab.title.isEmpty ? (tab.url.isEmpty ? "Untitled Tab":tab.url):tab.title,kind:"builtin",target:token))
            metadata[token]=BrowserTabInfo(url:tab.url);nextTargets[token]=Target(browser:browser,tab:tab,time:now)
        }
        targets.update {$0=nextTargets};cache=(browser,now,commands,metadata)
        return try filter(commands,metadata,query,ticket,deadline)
    }
    private func filter(_ commands:[Command],_ metadata:[String:BrowserTabInfo],_ query:String,_ ticket:SystemSearchSession.Ticket,_ deadline:TimeInterval)throws->SystemSearchResults {
        let words=query.split(whereSeparator:{$0.isWhitespace}).map(String.init)
        var matches=[Command]()
        for command in commands {
            let text=command.title+" "+(metadata[command.id]?.url ?? "")
            var found=true
            for word in words {
                guard ticket.current,effects.clock()<deadline else {throw LauncherError("Safari search cancelled or timed out. Refresh to retry.")}
                if text.range(of:word,options:[.caseInsensitive,.diacriticInsensitive])==nil {found=false;break}
            }
            if found {matches.append(command)}
        }
        guard ticket.current,effects.clock()<deadline else {throw LauncherError("Safari search cancelled or timed out. Refresh to retry.")}
        return SystemSearchResults(commands:matches,tabs:metadata)
    }
    func execute(_ token:String,_ ticket:SystemSearchSession.Ticket)throws {
        guard let target=targets.read({$0[token]}),effects.clock()-target.time<30 else {throw LauncherError(Self.expired)}
        let browser=target.browser,tab=target.tab,deadline=effects.clock()+8
        func validate()throws {
            try check(browser,ticket,deadline)
            let current=try tabs(tab.window,browser,ticket,deadline)
            guard current.contains(tab) else {throw LauncherError(Self.expired)}
            guard current.filter({$0.title==tab.title && $0.url==tab.url}).count==1 else {throw LauncherError("These tabs have the same title and address. Select one in Safari.")}
        }
        func selected()throws {
            try check(browser,ticket,deadline)
            let current=Self.property("cTab",window(tab.window))
            let index=try send("getd",browser,ticket,deadline,Self.property("pidx",current))
            let title=try send("getd",browser,ticket,deadline,Self.property("pnam",current))
            let url=try send("getd",browser,ticket,deadline,Self.property("pURL",current))
            guard index.int32Value==tab.index,title.stringValue==tab.title,url.stringValue==tab.url else {throw LauncherError(Self.expired)}
            try validate()
        }
        try validate()
        _=try send("setd",browser,ticket,deadline,Self.property("cTab",window(tab.window)),value:Self.object("bTab",window(tab.window),"indx",NSAppleEventDescriptor(int32:tab.index)))
        try selected() // No foreground effect after a detected selection mismatch.
        _=try send("setd",browser,ticket,deadline,Self.property("pidx",window(tab.window)),value:NSAppleEventDescriptor(int32:1))
        try selected()
        guard effects.activate(browser.identity.pid,deadline,{ticket.current && self.effects.clock()<deadline && self.effects.browser()==browser}) else {throw LauncherError("Could not activate Safari.")}
        try selected()
    }
}
