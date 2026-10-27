import AppKit
import ApplicationServices
import CryptoKit
import Darwin
import LauncherCore
import LauncherModels

/// Ephemeral native targets. Nothing from Accessibility or the process table is persisted.
final class SystemServices {
    struct Identity: Codable, Equatable { let pid: pid_t; let uid: uid_t; let seconds: UInt64; let micros: UInt64 }
    struct Effects {
        var processes: () throws -> [ProcessResource] = ResourceSampler.processTable
        var identity: (pid_t) -> Identity? = SystemServices.identity
        var trusted: () -> Bool = { AXIsProcessTrusted() }
        var terminate: (pid_t) -> Int32 = { Darwin.kill($0, SIGKILL) }
        var activate: (pid_t) -> Bool = { NSRunningApplication(processIdentifier:$0)?.activate(options:[]) ?? false }
        var press: (AXUIElement) -> AXError = { AXUIElementPerformAction($0,kAXPressAction as CFString) }
        var raise: (AXUIElement) -> AXError = { element in
            _ = AXUIElementSetAttributeValue(element,kAXMinimizedAttribute as CFString,kCFBooleanFalse)
            return AXUIElementPerformAction(element,kAXRaiseAction as CFString)
        }
        var monitor: () -> Bool = {
            let url=URL(fileURLWithPath:"/System/Applications/Utilities/Activity Monitor.app")
            return NSWorkspace.shared.open(url)
        }
        var lock: () throws -> Void = {
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
    struct Target { let kind:String; let identity:Identity; let element:AXUIElement? }
    private let effects:Effects
    private let lock=NSLock()
    private var generation=0
    private var resources:[String:ProcessResource]=[:]
    private var processCache:[ProcessResource]=[]
    private var processCacheDate=Date.distantPast
    private var fileProcess:Process?
    private var targets:[String:Target]=[:]
    private var targetOrder=[String]()
    private let worker=DispatchQueue(label:"ruycast.system-search",qos:.userInitiated)
    init(effects:Effects=Effects()) {self.effects=effects}

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
    func cancel() {
        lock.lock();generation+=1;let old=fileProcess;fileProcess=nil;lock.unlock()
        if let old,old.isRunning {old.terminate()}
    }
    func results(mode:String,query:String,previousPID:pid_t?,completion:@escaping(Result<[Command],Error>)->Void) {
        cancel();lock.lock();let version=generation;lock.unlock()
        worker.async { [weak self] in
            guard let self,self.current(version) else {return}
            let result=Result { () throws -> [Command] in
                switch mode {
                case "files":return try self.files(query,version:version)
                case "menus":return try self.menus(query,pid:previousPID,version:version)
                case "windows":return try self.windows(query,version:version)
                case "processes":return try self.processes(query,version:version)
                default:throw LauncherError("Unknown system search.")
                }
            }
            DispatchQueue.main.async {if self.current(version) {completion(result)}}
        }
    }
    private func current(_ value:Int)->Bool {lock.lock();defer {lock.unlock()};return generation==value}
    @discardableResult func remember(kind:String,identity:Identity,element:AXUIElement?,key:String)->String {
        let token=Self.token("\(kind):\(identity.pid):\(identity.seconds):\(identity.micros):\(key)")
        lock.lock();defer {lock.unlock()}
        if targets[token]==nil {
            if targetOrder.count>=2048 {targets.removeValue(forKey:targetOrder.removeFirst())}
            targetOrder.append(token)
        }
        targets[token]=Target(kind:kind,identity:identity,element:element)
        return "system.\(kind):\(token)"
    }
    func requiresConfirmation(target:String)->Bool {false}
    func execute(target:String,confirmed:Bool=false,previousPID:pid_t?=nil)->[String:Any] {
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
                guard effects.activate(stored.identity.pid) else {throw LauncherError("Could not activate the original app.")}
                guard effects.press(element) == .success else {throw LauncherError("This menu item is no longer available.")}
            } else if stored.kind=="window" {
                guard effects.raise(element) == .success,effects.activate(stored.identity.pid) else {throw LauncherError("Could not switch to this window.")}
            } else {throw LauncherError("Unknown system action.")}
            return ["ok":true]
        } catch {return ["ok":false,"error":error.localizedDescription]}
    }
    private func files(_ query:String,version:Int) throws -> [Command] {
        let query=String(query.trimmingCharacters(in:.whitespacesAndNewlines).prefix(512))
        guard !query.isEmpty else {return []}
        guard !query.unicodeScalars.contains(where:CharacterSet.controlCharacters.contains) else {throw LauncherError("Enter a file name without control characters.")}
        let process=Process(),pipe=Pipe()
        process.executableURL=URL(fileURLWithPath:"/usr/bin/mdfind");process.arguments=["-0",Self.filePredicate(query)]
        process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
        lock.lock()
        guard generation==version else {lock.unlock();return []}
        fileProcess=process
        do {try process.run()} catch {fileProcess=nil;lock.unlock();throw error}
        lock.unlock()
        let deadline=DispatchWorkItem {if process.isRunning {process.terminate()}}
        DispatchQueue.global().asyncAfter(deadline:.now()+3,execute:deadline)
        defer {deadline.cancel();if process.isRunning {process.terminate()};try? pipe.fileHandleForReading.close();lock.lock();if fileProcess===process {fileProcess=nil};lock.unlock()}
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
    private func value(_ element:AXUIElement,_ key:String)->CFTypeRef? {var result:CFTypeRef?;guard AXUIElementCopyAttributeValue(element,key as CFString,&result) == .success else {return nil};return result}
    private func children(_ element:AXUIElement,_ key:String=kAXChildrenAttribute)->[AXUIElement] {value(element,key) as? [AXUIElement] ?? []}
    private func text(_ element:AXUIElement,_ key:String=kAXTitleAttribute)->String {value(element,key) as? String ?? ""}
    private func matches(_ title:String,_ query:String)->Bool {query.isEmpty || title.localizedCaseInsensitiveContains(query)}
    private func menus(_ query:String,pid:pid_t?,version:Int) throws -> [Command] {
        guard effects.trusted() else {throw LauncherError("Allow Accessibility access to search app menus.")}
        guard let pid,pid != getpid(),let identity=effects.identity(pid) else {throw LauncherError("Open an app before searching its menu.")}
        let app=AXUIElementCreateApplication(pid);AXUIElementSetMessagingTimeout(app,0.25)
        guard let raw=value(app,kAXMenuBarAttribute),CFGetTypeID(raw)==AXUIElementGetTypeID() else {throw LauncherError("This app does not expose a menu bar.")}
        let menu=unsafeBitCast(raw,to:AXUIElement.self),deadline=Date().addingTimeInterval(2)
        var results=[Command](),visited=0
        func visit(_ node:AXUIElement,_ path:[String],_ depth:Int) {
            guard depth<12,visited<1200,Date()<deadline,current(version),results.count<100 else {return};visited+=1
            let title=text(node),path=title.isEmpty ? path:path+[title]
            let kids=children(node)
            if !title.isEmpty,kids.isEmpty,text(node,kAXRoleAttribute)==kAXMenuItemRole,(value(node,kAXEnabledAttribute) as? Bool ?? true) {
                let label=path.joined(separator:" › ")
                if matches(label,query) {let target=remember(kind:"menu",identity:identity,element:node,key:path.joined(separator:"/"));results.append(Command(id:target,title:label,kind:"builtin",target:target))}
            }
            for child in kids {visit(child,path,depth+1)}
        }
        visit(menu,[],0);return results
    }
    private func windows(_ query:String,version:Int) throws -> [Command] {
        guard effects.trusted() else {throw LauncherError("Allow Accessibility access to switch windows.")}
        let deadline=Date().addingTimeInterval(3);var results=[Command]()
        for app in NSWorkspace.shared.runningApplications where app.processIdentifier != getpid() && app.activationPolicy == .regular {
            guard current(version),Date()<deadline,results.count<100 else {break}
            guard let identity=effects.identity(app.processIdentifier) else {continue}
            let element=AXUIElementCreateApplication(app.processIdentifier);AXUIElementSetMessagingTimeout(element,0.15)
            for (index,window) in children(element,kAXWindowsAttribute).prefix(50).enumerated() {
                let title=text(window);guard !title.isEmpty else {continue}
                let label="\(title) — \(app.localizedName ?? "App")"
                guard matches(label,query) else {continue}
                let target=remember(kind:"window",identity:identity,element:window,key:"\(index):\(CFHash(window))")
                results.append(Command(id:target,title:label,kind:"builtin",target:target));if results.count==100 {break}
            }
        }
        return results
    }
    func invalidateProcessCache() {worker.async { [weak self] in self?.processCacheDate=Date.distantPast}}
    var processResources:[String:ProcessResource] {lock.lock();defer {lock.unlock()};return resources}
    func processDetails(_ query:String,completion:@escaping(Result<[(Command,ProcessResource)],Error>)->Void) {
        worker.async { [weak self] in
            guard let self else {return}
            let result=Result { () throws -> [(Command,ProcessResource)] in
                let commands=try self.processes(query,version:nil)
                let resources=self.processResources
                return commands.compactMap {command in resources[command.id].map {(command,$0)}}
            }
            DispatchQueue.main.async {completion(result)}
        }
    }
    private func processes(_ query:String,version:Int?) throws -> [Command] {
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
