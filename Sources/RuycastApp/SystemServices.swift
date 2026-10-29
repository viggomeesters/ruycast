import AppKit
import ApplicationServices
import CryptoKit
import Darwin
import LauncherCore
import LauncherModels

/// Ephemeral native targets. Nothing from Accessibility or the process table is persisted.
// Generation/targets/resources/process ownership use lock; cache uses worker only.
// Immutable Effects has compiler-checked worker callbacks and main-actor effects.
final class SystemServices:@unchecked Sendable {
    struct Identity: Codable, Equatable, Sendable { let pid: pid_t; let uid: uid_t; let seconds: UInt64; let micros: UInt64 }
    struct Effects:Sendable {
        var browserTabs=BrowserTabs.Effects()
        var shortcuts: @Sendable (SystemSearchSession.Ticket) throws -> [Command] = AppleShortcuts.discover
        var shortcutClock: @Sendable () -> TimeInterval = {ProcessInfo.processInfo.systemUptime}
        var processes: @Sendable () throws -> [ProcessResource] = ResourceSampler.processTable
        var identity: @Sendable (pid_t) -> Identity? = SystemServices.identity
        var trusted: @Sendable () -> Bool = { AXIsProcessTrusted() }
        var windowApplications: @Sendable () -> [WindowSearchApplication] = WindowSearchApplication.running
        var windowApplication: @Sendable (pid_t) -> AXUIElement = {AXUIElementCreateApplication($0)}
        var windowAttribute: @Sendable (AXUIElement,String) -> CFTypeRef? = SystemAccessibility.value
        var windowList: @Sendable (AXUIElement,Int) -> [AXUIElement]? = {SystemAccessibility.elements($0,kAXWindowsAttribute,$1)}
        var windowElementPID: @Sendable (AXUIElement) -> pid_t? = SystemAccessibility.pid
        var windowClock: @Sendable () -> TimeInterval = {ProcessInfo.processInfo.systemUptime}
        var restoreWindow: @MainActor @Sendable (AXUIElement) -> AXError = {element in
            AXUIElementSetMessagingTimeout(element,0.12)
            return AXUIElementSetAttributeValue(element,kAXMinimizedAttribute as CFString,kCFBooleanFalse)
        }
        var menuApplication: @Sendable (pid_t) -> AXUIElement = {AXUIElementCreateApplication($0)}
        var menuAttribute: @Sendable (AXUIElement,String) -> CFTypeRef? = SystemAccessibility.value
        var menuChildren: @Sendable (AXUIElement,Int) -> [AXUIElement]? = SystemAccessibility.children
        var menuApplicationName: @Sendable (pid_t) -> String = {NSRunningApplication(processIdentifier:$0)?.localizedName ?? "App"}
        var menuClock: @Sendable () -> TimeInterval = {ProcessInfo.processInfo.systemUptime}
        var menuElementPID: @Sendable (AXUIElement) -> pid_t? = SystemAccessibility.pid
        var terminate: @MainActor @Sendable (pid_t) -> Int32 = { Darwin.kill($0, SIGKILL) }
        var activate: @MainActor @Sendable (pid_t) -> Bool = { NSRunningApplication(processIdentifier:$0)?.activate(options:[]) ?? false }
        var press: @MainActor @Sendable (AXUIElement) -> AXError = { AXUIElementPerformAction($0,kAXPressAction as CFString) }
        var raise: @MainActor @Sendable (AXUIElement) -> AXError = { element in
            AXUIElementSetMessagingTimeout(element,0.12)
            return AXUIElementPerformAction(element,kAXRaiseAction as CFString)
        }
        var monitor: @MainActor @Sendable () -> Bool = {
            let url=URL(fileURLWithPath:"/System/Applications/Utilities/Activity Monitor.app")
            return NSWorkspace.shared.open(url)
        }
        var lock: @MainActor @Sendable () throws -> Void = {
            // CGSession is absent on current macOS. The supported lock shortcut is the fallback.
            let path="/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession"
            if FileManager.default.isExecutableFile(atPath:path) {
                let process=Process();process.executableURL=URL(fileURLWithPath:path);process.arguments=["-suspend"]
                try process.run();return
            }
            guard AXIsProcessTrusted() else {throw LauncherError("Allow Accessibility access to lock the screen.")}
            guard let down=CGEvent(keyboardEventSource:nil,virtualKey:12,keyDown:true),let up=CGEvent(keyboardEventSource:nil,virtualKey:12,keyDown:false) else {throw LauncherError("Could not lock the screen.")}
            down.flags=[.maskCommand,.maskControl];up.flags=down.flags
            down.setIntegerValueField(.eventSourceUserData,value:HyperKey.generatedTag);up.setIntegerValueField(.eventSourceUserData,value:HyperKey.generatedTag)
            down.post(tap:.cghidEventTap);up.post(tap:.cghidEventTap)
        }
    }
    struct Target { let kind:String; let identity:Identity; let element:AXUIElement?; let menu:MenuActionInfo?;let menuAncestors:[MenuAncestor];let window:WindowActionInfo? }
    private let effects:Effects
    private let browserTabs:BrowserTabs
    private let lock=NSLock()
    let interactiveSession=SystemSearchSession()
    private var resources:[String:ProcessResource]=[:]
    private var processCache:[ProcessResource]=[]
    private var shortcutCache:[Command]?
    private var shortcutCacheTime:TimeInterval=0
    private var processCacheDate=Date.distantPast
    private var targets:[String:Target]=[:]
    private var targetOrder=[String]()
    private let worker=DispatchQueue(label:"ruycast.system-search",qos:.userInitiated)
    init(effects:Effects=Effects()) {self.effects=effects;self.browserTabs=BrowserTabs(effects:effects.browserTabs)}

    static func identity(_ pid:pid_t)->Identity? {
        var info=proc_bsdinfo()
        let size=MemoryLayout<proc_bsdinfo>.size
        guard proc_pidinfo(pid,PROC_PIDTBSDINFO,0,&info,Int32(size))==Int32(size) else {return nil}
        return Identity(pid:pid,uid:info.pbi_uid,seconds:info.pbi_start_tvsec,micros:info.pbi_start_tvusec)
    }
    static func token(_ text:String)->String {SHA256.hash(data:Data(text.utf8)).map {String(format:"%02x",$0)}.joined()}
    static func filePredicate(_ query:String)->String {
        let escaped=query.replacingOccurrences(of:"\\",with:"\\\\").replacingOccurrences(of:"\"",with:"\\\"").replacingOccurrences(of:"*",with:"\\*").replacingOccurrences(of:"?",with:"\\?")
        return "kMDItemFSName == \"*\(escaped)*\"cd"
    }
    func cancel() {interactiveSession.cancel()}
    func results(mode:String,query:String,previousPID:pid_t?,session:SystemSearchSession?=nil,completion:@escaping @MainActor @Sendable (Result<[Command],Error>)->Void) {
        search(mode:mode,query:query,previousPID:previousPID,session:session) {completion($0.map(\.commands))}
    }
    func search(mode:String,query:String,previousPID:pid_t?,session:SystemSearchSession?=nil,completion:@escaping @MainActor @Sendable (Result<SystemSearchResults,Error>)->Void) {
        (session ?? interactiveSession).submit(on:worker) { [weak self] version in
            guard let self,version.current else {return}
            let result=Result { () throws -> SystemSearchResults in
                switch mode {
                case "files":return SystemSearchResults(commands:try self.files(query,version:version))
                case "menus":return try self.menus(query,pid:previousPID,version:version)
                case "windows":return try self.windows(query,version:version)
                case "tabs":return try self.browserTabs.search(query,version)
                case "shortcuts":return SystemSearchResults(commands:try self.shortcuts(version:version))
                case "processes":return SystemSearchResults(commands:try self.processes(query,version:version))
                default:throw LauncherError("Unknown system search.")
                }
            }
            DispatchQueue.main.async {if version.current {completion(result)}}
        }
    }
    func cancelBrowserConsent() {browserTabs.cancelConsent()}
    func invalidateBrowserCache() {worker.async {self.browserTabs.invalidate()}}
    func requestBrowserConsent(completion:@escaping @MainActor @Sendable ()->Void) {browserTabs.requestConsent(completion:completion)}
    func executeTab(_ target:String,completion:@escaping @MainActor @Sendable (Result<Void,Error>)->Void) {
        // Execution owns its session: hiding the palette cancels discovery only.
        let session=SystemSearchSession()
        session.submit(on:worker) {ticket in
            let result=Result {try self.browserTabs.execute(target,ticket)}
            DispatchQueue.main.async {completion(result)}
        }
    }
    private func current(_ version:SystemSearchSession.Ticket)->Bool {version.current}
    @discardableResult func remember(kind:String,identity:Identity,element:AXUIElement?,key:String,menu:MenuActionInfo?=nil,ancestors:[MenuAncestor]=[],window:WindowActionInfo?=nil)->String {
        let token=Self.token("\(kind):\(identity.pid):\(identity.seconds):\(identity.micros):\(key)")
        lock.lock();defer {lock.unlock()}
        if targets[token]==nil {
            if targetOrder.count>=2048 {targets.removeValue(forKey:targetOrder.removeFirst())}
            targetOrder.append(token)
        }
        targets[token]=Target(kind:kind,identity:identity,element:element,menu:menu,menuAncestors:ancestors,window:window)
        return "system.\(kind):\(token)"
    }
    func requiresConfirmation(target:String)->Bool {false}
    @MainActor func execute(target:String,confirmed:Bool=false,previousPID:pid_t?=nil)->[String:Any] {
        do {
            if target=="system-monitor" {guard effects.monitor() else {throw LauncherError("Could not open Activity Monitor.")};return ["ok":true]}
            if target=="lock-screen" {try effects.lock();return ["ok":true]}
            let parts=target.split(separator:":",maxSplits:1).map(String.init)
            guard parts.count==2 else {throw LauncherError("This result has expired. Search again.")}
            lock.lock();let stored=targets[parts[1]];lock.unlock()
            guard let stored,parts[0]=="system.\(stored.kind)",effects.identity(stored.identity.pid)==stored.identity else {throw LauncherError("This result has expired. Search again.")}
            if stored.kind=="kill" {
                guard stored.identity.uid==getuid(),stored.identity.pid != getpid(),stored.identity.pid>1 else {throw LauncherError("This process cannot be force quit.")}
                guard effects.terminate(stored.identity.pid)==0 else {throw LauncherError("Could not force quit the process.")}
                return ["ok":true]
            }
            guard effects.trusted() else {throw LauncherError("Allow Accessibility access in System Settings.")}
            guard let element=stored.element else {throw LauncherError("This result has expired. Search again.")}
            if stored.kind=="menu" {
                guard previousPID==stored.identity.pid else {throw LauncherError("The original app changed. Search its menu again.")}
                guard stored.menu?.enabled==true,!stored.menuAncestors.isEmpty else {throw LauncherError("This menu item is unavailable. Search its menu again.")}
                let deadline=effects.menuClock()+2
                func read(_ node:AXUIElement,_ key:String)->CFTypeRef? {effects.menuClock()<deadline ? effects.menuAttribute(node,key):nil}
                func validate() throws {
                    guard effects.trusted(),effects.identity(stored.identity.pid)==stored.identity else {throw LauncherError("The original app changed. Search its menu again.")}
                    for ancestor in stored.menuAncestors {
                        guard effects.menuElementPID(ancestor.element)==stored.identity.pid,
                              read(ancestor.element,kAXRoleAttribute) as? String==ancestor.role,
                              read(ancestor.element,kAXTitleAttribute) as? String==ancestor.title,
                              ancestor.role==kAXMenuBarItemRole ? SystemAccessibility.boolean(read(ancestor.element,kAXEnabledAttribute)) != false:SystemAccessibility.boolean(read(ancestor.element,kAXEnabledAttribute))==true else {throw LauncherError("This menu item changed or is unavailable. Search its menu again.")}
                    }
                    guard effects.menuClock()<deadline,effects.menuChildren(element,1)?.isEmpty==true,effects.identity(stored.identity.pid)==stored.identity else {throw LauncherError("This menu item changed or is unavailable. Search its menu again.")}
                }
                try validate()
                guard effects.activate(stored.identity.pid) else {throw LauncherError("Could not activate the original app.")}
                // Activation can rebuild a dynamic menu. Validate after that boundary too.
                try validate()
                guard effects.press(element) == .success else {throw LauncherError("This menu item is no longer available.")}
            } else if stored.kind=="window" {
                guard let captured=stored.window else {throw LauncherError("This window has expired. Search again.")}
                let deadline=effects.windowClock()+2,app=effects.windowApplication(stored.identity.pid)
                func validate()->(valid:Bool,minimized:Bool?) {
                    guard effects.windowClock()<deadline,effects.trusted(),effects.identity(stored.identity.pid)==stored.identity,
                          effects.windowElementPID(element)==stored.identity.pid,effects.windowElementPID(app)==stored.identity.pid,
                          effects.windowAttribute(element,kAXRoleAttribute) as? String==kAXWindowRole,
                          effects.windowClock()<deadline,effects.windowAttribute(element,kAXTitleAttribute) as? String==captured.title,
                          effects.windowClock()<deadline,let members=effects.windowList(app,50),members.contains(where:{CFEqual($0,element)}),
                          effects.windowClock()<deadline,effects.identity(stored.identity.pid)==stored.identity else {return (false,nil)}
                    return (true,SystemAccessibility.boolean(effects.windowAttribute(element,kAXMinimizedAttribute)))
                }
                guard validate().valid else {throw LauncherError("This window changed or closed. Search again.")}
                guard effects.activate(stored.identity.pid) else {throw LauncherError("Could not activate this window’s app.")}
                let current=validate()
                guard current.valid else {throw LauncherError("This window changed or closed. Search again.")}
                if current.minimized==true {
                    guard effects.restoreWindow(element) == .success else {throw LauncherError("Could not restore this window.")}
                    let restored=validate()
                    guard restored.valid,restored.minimized==false else {throw LauncherError("This window could not be restored. Search again.")}
                }
                guard effects.windowClock()<deadline,effects.trusted(),effects.identity(stored.identity.pid)==stored.identity,
                      effects.raise(element) == .success else {throw LauncherError("Could not switch to this window.")}
            } else {throw LauncherError("Unknown system action.")}
            return ["ok":true]
        } catch {return ["ok":false,"error":error.localizedDescription]}
    }
    private func files(_ query:String,version:SystemSearchSession.Ticket) throws -> [Command] {
        let query=String(query.trimmingCharacters(in:.whitespacesAndNewlines).prefix(512))
        guard !query.isEmpty else {return []}
        guard !query.unicodeScalars.contains(where:CharacterSet.controlCharacters.contains) else {throw LauncherError("Enter a file name without control characters.")}
        let process=Process(),pipe=Pipe()
        process.executableURL=URL(fileURLWithPath:"/usr/bin/mdfind");process.arguments=["-0",Self.filePredicate(query)]
        process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
        guard try version.launch(process) else {return []}
        let deadline=DispatchWorkItem {if process.isRunning {process.terminate()}}
        DispatchQueue.global().asyncAfter(deadline:.now()+3,execute:deadline)
        defer {deadline.cancel();if process.isRunning {process.terminate()};try? pipe.fileHandleForReading.close();version.detach(process)}
        var pending=Data(),paths=[String](),bytes=0
        while current(version),paths.count<100,bytes<1_048_576 {
            guard let data=try pipe.fileHandleForReading.read(upToCount:16384),!data.isEmpty else {break}
            bytes+=data.count;pending.append(data)
            while let end=pending.firstIndex(of:0) {
                let value=String(decoding:pending[..<end],as:UTF8.self);pending.removeSubrange(...end)
                if value.hasPrefix("/"),!paths.contains(value) {paths.append(value)}
                if paths.count==100 {break}
            }
        }
        return paths.prefix(100).map {Command(id:"system.file."+Self.token($0),title:URL(fileURLWithPath:$0).lastPathComponent,kind:"path",target:$0,aliases:[$0])}
    }
    private func matches(_ title:String,_ query:String)->Bool {query.isEmpty || title.localizedCaseInsensitiveContains(query)}
    private func menus(_ query:String,pid:pid_t?,version:SystemSearchSession.Ticket) throws -> SystemSearchResults {
        guard effects.trusted() else {throw LauncherError("Allow Ruycast in System Settings → Privacy & Security → Accessibility to search app menus.")}
        guard let pid,pid != getpid(),let identity=effects.identity(pid) else {throw LauncherError("Open an app before searching its menu.")}
        let deadline=effects.menuClock()+2,app=effects.menuApplication(pid)
        guard let raw=effects.menuAttribute(app,kAXMenuBarAttribute),CFGetTypeID(raw)==AXUIElementGetTypeID() else {throw LauncherError("This app does not expose a menu bar.")}
        let menu=unsafeDowncast(raw,to:AXUIElement.self),name=effects.menuApplicationName(pid)
        var results=[Command](),metadata=[String:MenuActionInfo](),visited=0
        func available()->Bool {current(version) && effects.menuClock()<deadline && visited<1200 && results.count<100}
        func read(_ node:AXUIElement,_ key:String)->CFTypeRef? {available() ? effects.menuAttribute(node,key):nil}
        func visit(_ node:AXUIElement,_ ancestors:[MenuAncestor],_ parentEnabled:Bool,_ depth:Int) {
            guard depth<12,available(),effects.menuElementPID(node)==pid else {return};visited+=1
            let role=read(node,kAXRoleAttribute) as? String ?? "",title=read(node,kAXTitleAttribute) as? String ?? ""
            let named = !title.isEmpty && (role==kAXMenuItemRole || role==kAXMenuBarItemRole)
            let path=named ? ancestors+[MenuAncestor(element:node,title:title,role:role)]:ancestors
            let enabled=parentEnabled && (role==kAXMenuItemRole ? SystemAccessibility.boolean(read(node,kAXEnabledAttribute))==true:SystemAccessibility.boolean(read(node,kAXEnabledAttribute)) != false)
            guard available() else {return}
            guard let kids=effects.menuChildren(node,1200-visited) else {return}
            if named,kids.isEmpty,role==kAXMenuItemRole {
                let breadcrumb=path.dropLast().map(\.title).joined(separator:" › ")
                if matches(name+" "+breadcrumb+" "+title,query) {
                    let shortcut=MenuShortcut(character:read(node,kAXMenuItemCmdCharAttribute) as? String,glyph:SystemAccessibility.integer(read(node,kAXMenuItemCmdGlyphAttribute)),virtualKey:SystemAccessibility.integer(read(node,kAXMenuItemCmdVirtualKeyAttribute)),modifiers:SystemAccessibility.integer(read(node,kAXMenuItemCmdModifiersAttribute)))
                    guard available() else {return}
                    let info=MenuActionInfo(application:name,path:breadcrumb,title:title,shortcut:shortcut,enabled:enabled)
                    // A new scan cannot retarget an already returned opaque action.
                    let target=remember(kind:"menu",identity:identity,element:node,key:UUID().uuidString,menu:info,ancestors:path)
                    metadata[target]=info;results.append(Command(id:target,title:title,kind:"builtin",target:target,enabled:enabled))
                }
            }
            for child in kids {guard available() else {break};visit(child,path,enabled,depth+1)}
        }
        visit(menu,[],true,0)
        guard effects.identity(pid)==identity else {throw LauncherError("The original app changed. Search its menu again.")}
        return SystemSearchResults(commands:results,menus:metadata)
    }
    private func windows(_ query:String,version:SystemSearchSession.Ticket) throws -> SystemSearchResults {
        guard effects.trusted() else {throw LauncherError("Allow Ruycast in System Settings → Privacy & Security → Accessibility to search windows.")}
        let deadline=effects.windowClock()+2;var commands=[Command](),metadata=[String:WindowActionInfo]()
        func available()->Bool {current(version) && effects.windowClock()<deadline && commands.count<100}
        for app in effects.windowApplications().prefix(200) {
            guard available() else {break}
            guard app.pid != getpid(),app.pid>1,let identity=effects.identity(app.pid),identity.uid==getuid() else {continue}
            let application=effects.windowApplication(app.pid)
            guard available(),effects.windowElementPID(application)==app.pid,let windows=effects.windowList(application,50) else {continue}
            for window in windows {
                guard available() else {break}
                guard effects.windowElementPID(window)==app.pid,effects.windowAttribute(window,kAXRoleAttribute) as? String==kAXWindowRole,
                      available(),let title=effects.windowAttribute(window,kAXTitleAttribute) as? String else {continue}
                guard matches(app.name+" "+title,query),available(),effects.identity(app.pid)==identity else {continue}
                let minimized=SystemAccessibility.boolean(effects.windowAttribute(window,kAXMinimizedAttribute))
                guard available() else {break}
                let info=WindowActionInfo(application:app.name,title:title,minimized:minimized)
                let target=remember(kind:"window",identity:identity,element:window,key:UUID().uuidString,window:info)
                metadata[target]=info;commands.append(Command(id:target,title:title.isEmpty ? "Untitled Window":title,kind:"builtin",target:target))
            }
        }
        return SystemSearchResults(commands:commands,windows:metadata)
    }
    // Query filtering follows configuration overrides in AppState, including aliases.
    private func shortcuts(version:SystemSearchSession.Ticket) throws -> [Command] {
        if let shortcutCache,effects.shortcutClock()-shortcutCacheTime<30 {return shortcutCache}
        let commands=try effects.shortcuts(version)
        guard version.current else {throw LauncherError("Shortcut search cancelled.")}
        shortcutCache=commands;shortcutCacheTime=effects.shortcutClock();return commands
    }
    func invalidateShortcutCache() {worker.async { [weak self] in self?.shortcutCache=nil}}
    func invalidateProcessCache() {worker.async { [weak self] in self?.processCacheDate=Date.distantPast}}
    var processResources:[String:ProcessResource] {lock.lock();defer {lock.unlock()};return resources}
    func processDetails(_ query:String,session:SystemSearchSession?=nil,completion:@escaping @MainActor @Sendable (Result<[(Command,ProcessResource)],Error>)->Void) {
        (session ?? SystemSearchSession()).submit(on:worker) { [weak self] version in
            guard let self,version.current else {return}
            let result=Result { () throws -> [(Command,ProcessResource)] in
                let commands=try self.processes(query,version:version)
                let resources=self.processResources
                return commands.compactMap {command in resources[command.id].map {(command,$0)}}
            }
            DispatchQueue.main.async {if version.current {completion(result)}}
        }
    }
    private func processes(_ query:String,version:SystemSearchSession.Ticket?) throws -> [Command] {
        if Date().timeIntervalSince(processCacheDate)>1.5 {processCache=try effects.processes();processCacheDate=Date()}
        var result=[Command](),details=[String:ProcessResource]()
        for sample in processCache.filter({matches($0.name,query) || String($0.id).contains(query)}).sorted(by: {$0.cpuPercent>$1.cpuPercent}).prefix(1000) {
            if let version,!current(version) {return []}
            guard matches(sample.name,query) || String(sample.id).contains(query) else {continue}
            let target=remember(kind:"kill",identity:sample.identity,element:nil,key:String(sample.id))
            result.append(Command(id:target,title:sample.name,kind:"builtin",target:target));details[target]=sample
        }
        lock.lock();resources=details;lock.unlock()
        return result.sorted {(details[$0.id]?.cpuPercent ?? 0)>(details[$1.id]?.cpuPercent ?? 0)}
    }
}
