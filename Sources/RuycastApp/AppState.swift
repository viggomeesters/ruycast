import AppKit
import Combine
import CryptoKit
import LauncherCore

func jsonValue<T:Encodable>(_ value:T)->Any { (try? JSONSerialization.jsonObject(with:Files.encode(value))) ?? NSNull() }

final class AppState:ObservableObject {
    let root:URL
    @Published var configDirectory:URL
    let headless:Bool
    @Published var config:Configuration {didSet {rebuildCatalog()}}
    var clipboardDataRevision=0
    @Published var clips:[Clip]=[] {didSet {
        clipboardDataRevision+=1;clipboardWork.invalidate()
        if mode != "clipboard" {
            let current=Dictionary(clips.map {($0.id,$0)},uniquingKeysWith:{$1})
            filteredClips=filteredClips.compactMap {current[$0.id]}
        }
        refreshClips(preserveSelection:paletteVisible)
    }}
    @Published var query="" {didSet {if query != oldValue {captureInlineQuery();refreshResults();refreshClips()}}}
    @Published var mode="commands" {didSet {if mode != oldValue {inlineCommand=nil;inlineInput="";query="";selection=0;actionsOpen=false;form=nil;detail=nil;refreshResults();refreshClips();restoreSearchFocus?()}}}
    @Published var selection=0
    @Published var actionSelection=0
    @Published var pendingRemoval:Command?
    var restoreSearchFocus:(()->Void)?
    @Published var actionsOpen=false {didSet {
        if actionsOpen && !oldValue {actionSelection=0}
        if !actionsOpen && oldValue {restoreSearchFocus?()}
    }}
    @Published var inlineCommand:Command?
    var inlineSource="launcher"
    @Published var inlineInput=""
    @Published var form:CommandForm?
    @Published var confirmationCommand:Command?
    let commandLog:CommandLog
    let analytics:AnalyticsStore
    let systemServices:SystemServices
    let clipboardStore:ClipboardStore
    let clipboardWork=ClipboardWork()
    private(set) var clipboardWorkInFlight=false
    private var configObserver:ConfigurationObserver?
    private let clipboardSearch=ClipboardSearch()
    private(set) var clipboardSearchBusy=false
    lazy var utilities=Utilities(root:root)
    @Published var detail:String?
    private var resultGeneration=0
    var serviceCommands:[String:Command]=[:]
    var configurationSnapshot:Data {configBytes}
    var configDigestForSave:String {SHA256.hash(data:configBytes).map {String(format:"%02x",$0)}.joined()}
    @Published var message=""
    @Published var busy=false
    @Published var paused=false
    @Published var clipboardCleanupError:String?
    @Published var configError:String?
    @Published var applications:[Command]=[] {didSet {rebuildCatalog()}}
    var counts:[String:Int]=[:] {didSet {if query.isEmpty {refreshResults()}}}
    @Published private(set) var results:[Command]=[] {didSet {rebuildProcessOrder()}}
    private(set) var processOrder:[Command]=[]
    @Published private(set) var filteredClips:[Clip]=[]
    private var searchIndex:CommandSearchIndex?
    private(set) var searchEvaluations=0
    var showPalette:((String)->Void)?
    var showManagement:(()->Void)?
    var showAnalytics:(()->Void)?
    var showSystemMonitor:(()->Void)?
    @Published var processResources:[String:ProcessResource]=[:]
    @Published var processSort:ProcessSort = .cpu {didSet {rebuildProcessOrder()}}
    private var lastProcessRefresh=Date.distantPast
    private let monitorSampler=ResourceSampler()
    private let monitorWorker=DispatchQueue(label:"ruycast.monitor-cli",qos:.utility)
    var managementVisible=false
    var recordingShortcut=false
    var hidePalette:(()->Void)?
    var showToast:((String,Bool)->Void)?
    var paletteVisible=false
    var previousPID:pid_t?
    let windows=WindowManager()
    var hotkeys:Hotkeys?
    var expander:SnippetExpander?
    @Published var expansionStatus="Off"
    private var timer:Timer?
    private var changeCount=0
    private var configBytes:Data
    private var lastAttempt:Data
    private var observer:NSObjectProtocol?
    private var clipboardAttributionUncertain=false
    private var lastClipboardPID:pid_t?
    private var lastPruned=Date()
    var configURL:URL {configDirectory.appendingPathComponent("config.json")}
    init(root:URL,headless:Bool,systemServices:SystemServices=SystemServices()) throws {
        self.root=root;self.headless=headless;self.systemServices=systemServices;self.analytics=AnalyticsStore(root:root);self.commandLog=CommandLog(root:root);self.clipboardStore=ClipboardStore(root:root)
        let url=try ConfigurationLocation.initialize(root:root)
        configDirectory=url.deletingLastPathComponent()
        let initialBytes=try Data(contentsOf:url)
        config=try Files.config(data:initialBytes);configBytes=initialBytes;lastAttempt=initialBytes
        let history=root.appendingPathComponent("clipboard.json")
        if FileManager.default.fileExists(atPath:history.path) {clips=try Files.decode([Clip].self,data:Data(contentsOf:history))}
        let retained=try History.updateChecked(clips,clip:nil,settings:config.clipboard)
        if retained != clips {clips=retained;try Files.save(clips,to:history)}
        if let d=try? Data(contentsOf:root.appendingPathComponent("usage.json")) {counts=try Files.decode([String:Int].self,data:d)}
        paused=(try? Files.decode(Bool.self,data:Data(contentsOf:root.appendingPathComponent("paused.json")))) ?? false
        if !headless {
            changeCount=NSPasteboard.general.changeCount
            lastClipboardPID=NSWorkspace.shared.frontmostApplication?.processIdentifier
            hotkeys=Hotkeys();hotkeys?.onAction={ [weak self] action in
                guard let self,!self.recordingShortcut else {return}
                if action=="@launcher" {self.showPalette?("commands")}
                else if action=="@clipboard" {self.showPalette?("clipboard")}
                else if let command=self.config.resolve(action),!Templates.fields(command).isEmpty {
                    if !self.paletteVisible {self.showPalette?("commands")}
                    self.begin(command,source:"hotkey")
                } else {self.perform(action,source:"hotkey")}
            };do {try hotkeys?.update(config)} catch {configError=error.localizedDescription;analytics.recordError("config",code:"config-invalid")}
            observer=NSWorkspace.shared.notificationCenter.addObserver(forName:NSWorkspace.didActivateApplicationNotification,object:nil,queue:.main) { [weak self] note in
                self?.clipboardAttributionUncertain=true
                if let app=note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,app.processIdentifier != getpid() {self?.previousPID=app.processIdentifier}
            }
            discoverApps()
        }
        analytics.importLegacy(counts)
        rebuildCatalog();refreshClips()
        if !headless {expander=SnippetExpander(state:self)}
        observeConfiguration()
        timer=Timer.scheduledTimer(withTimeInterval:0.75,repeats:true) { [weak self] _ in self?.tick() }
    }
    func observeConfiguration() {
        let directory=configDirectory
        configObserver=ConfigurationObserver(root:directory,initial:configBytes) { [weak self] result in
            guard let self,self.configDirectory==directory else {return}
            do {let observed=try result.get();guard observed.bytes != self.lastAttempt || self.configError != nil else {return};self.lastAttempt=observed.bytes;try self.reload(observedData:observed.bytes,observedConfig:observed.config)}
            catch {self.configError=error.localizedDescription;self.analytics.recordError("config",code:"config-invalid")}
        }
    }
    private func rebuildCatalog() {
        let ids=Set(config.commands.filter {$0.kind=="app"}.map(\.target))
        let commands=config.commands.filter(\.isEnabled)+systemCommands+applications.filter {!ids.contains($0.target)}
        searchIndex=try? CommandSearchIndex(commands:commands)
        refreshResults()
    }
    func rememberServices(_ commands:[Command]) {if serviceCommands.count>2048 {serviceCommands=[:]};for command in commands {serviceCommands[command.id]=command}}
    func searchCommands(_ query:String,mode:String="commands",limit:Int=100)->[Command] {
        let found=(searchIndex?.search(query) ?? []).filter {mode=="snippets" ? $0.kind=="snippet":mode=="quicklinks" ? ($0.kind=="url" || $0.kind=="path"):true}
        if !query.isEmpty {return limit<=0 ? found:Array(found.prefix(limit))}
        return Array(found.enumerated().sorted {a,b in let x=counts[a.element.id] ?? 0,y=counts[b.element.id] ?? 0;return x==y ? a.offset<b.offset : x>y}.map(\.element).prefix(limit<=0 ? found.count:limit))
    }
    private func refreshResults(preserveSelection:Bool=false) {
        let started=DispatchTime.now().uptimeNanoseconds
        defer {if !["files","menus","windows","processes"].contains(mode) {analytics.recordTiming("search.model",ms:AnalyticsStore.elapsed(started))}}
        searchEvaluations+=1;if !preserveSelection {selection=0};resultGeneration+=1
        let generation=resultGeneration
        if mode=="clipboard" {systemServices.cancel();if !results.isEmpty {results=[]};return}
        if ["files","menus","windows","processes"].contains(mode) {
            if !preserveSelection {results=[]}
            if mode=="processes" {
                systemServices.cancel()
                systemServices.processDetails(query) { [weak self] result in
                    guard let self,self.resultGeneration==generation else {return}
                    switch result {
                    case .success(let rows):self.applySystemResults(rows.map { $0.0 },resources:Dictionary(uniqueKeysWithValues:rows.map { ($0.0.id,$0.1) }),preserveSelection:preserveSelection)
                    case .failure(let error):self.message=error.localizedDescription
                    }
                }
            } else {
                systemServices.results(mode:mode,query:query,previousPID:previousPID) { [weak self] result in
                    guard let self,self.resultGeneration==generation else {return}
                    switch result {case .success(let commands):self.applySystemResults(commands,resources:[:],preserveSelection:preserveSelection);case .failure(let error):self.message=error.localizedDescription}
                }
            }
        } else if mode.hasPrefix("utility-") && mode.hasSuffix("history") {
            systemServices.cancel();do {results=try utilities.commands(mode:mode,query:query)} catch {results=[];message=error.localizedDescription}
        } else {systemServices.cancel();results=searchCommands(query,mode:mode)}
    }
    private func refreshClips(preserveSelection:Bool=false) {
        let selectedID=preserveSelection && filteredClips.indices.contains(selection) ? filteredClips[selection].id:nil
        clipboardSearch.cancel();clipboardSearchBusy=false
        guard mode=="clipboard" else {return}
        if query.isEmpty {filteredClips=clips;if let selectedID {selection=filteredClips.firstIndex {$0.id==selectedID} ?? -1};return}
        let started=DispatchTime.now().uptimeNanoseconds
        clipboardSearchBusy=true;filteredClips=[]
        clipboardSearch.search(clips,query:query) { [weak self] next in
            guard let self,self.mode=="clipboard" else {return};self.filteredClips=next;self.clipboardSearchBusy=false
            if let selectedID {self.selection=next.firstIndex {$0.id==selectedID} ?? -1}
            self.analytics.recordTiming("search.clipboard",ms:AnalyticsStore.elapsed(started))
        }
    }
    private func discoverApps() {
        DispatchQueue.global(qos:.utility).async { [weak self] in
            var apps:[String:Command]=[:]
            for path in ["/Applications","/System/Applications",NSString(string:"~/Applications").expandingTildeInPath] {
                guard let enumerator=FileManager.default.enumerator(at:URL(fileURLWithPath:path),includingPropertiesForKeys:nil,options:[.skipsHiddenFiles]) else {continue}
                for case let url as URL in enumerator {
                    if url.pathExtension=="app" {
                        enumerator.skipDescendants()
                        guard let bundle=Bundle(url:url),let id=bundle.bundleIdentifier else {continue}
                        let title=(bundle.object(forInfoDictionaryKey:"CFBundleDisplayName") as? String) ?? (bundle.object(forInfoDictionaryKey:"CFBundleName") as? String) ?? url.deletingPathExtension().lastPathComponent
                        apps[id]=Command(id:"discovered."+id,title:title,kind:"app",target:id)
                    } else if enumerator.level>3 {enumerator.skipDescendants()}
                }
            }
            let sorted=apps.values.sorted {$0.title.localizedStandardCompare($1.title) == .orderedAscending}
            DispatchQueue.main.async {self?.applications=sorted}
        }
    }
    // The closure is the platform registration boundary; checks inject failures without global keys.
    func reload(observedData:Data?=nil,observedConfig:Configuration?=nil,registerHotkeys:((Configuration)throws->Void)?=nil) throws {
        let started=DispatchTime.now().uptimeNanoseconds
        defer {analytics.recordTiming("config.reload",ms:AnalyticsStore.elapsed(started))}
        let data=try observedData ?? Data(contentsOf:configURL)
        let candidate=try observedConfig ?? Files.config(data:data)
        var trimmed=clips
        let register=registerHotkeys ?? {try self.hotkeys?.update($0)}
        try register(candidate)
        do {if candidate.clipboard != config.clipboard {
            let fallback=clips
            trimmed=try clipboardWork.sync {
                let base=try self.persistedHistory(fallback:fallback)
                let next=try History.updateChecked(base,clip:nil,settings:candidate.clipboard)
                if next != base {try Files.save(next,to:self.root.appendingPathComponent("clipboard.json"))}
                return next
            }
        }}
        catch {
            let saveError=error
            do {try register(config)} catch {configError="Could not restore shortcuts: \(error.localizedDescription)"}
            throw saveError
        }
        config=candidate;configBytes=data;lastAttempt=data;if clips != trimmed {clips=trimmed};configError=nil;configObserver?.acknowledge(data)
        cleanupClipboardAssets()
    }
    private func persistedHistory(fallback:[Clip]) throws -> [Clip] {
        let url=root.appendingPathComponent("clipboard.json")
        return FileManager.default.fileExists(atPath:url.path) ? try Files.decode([Clip].self,data:Data(contentsOf:url)):fallback
    }
    private func cleanupClipboardAssets() {
        guard !clipboardWorkInFlight else {return}
        do {try clipboardWork.sync {try clipboardStore.pruneAssets(keeping:clips,settings:config.clipboard)};if clipboardCleanupError != nil {clipboardCleanupError=nil}}
        catch {clipboardCleanupError="Clipboard cleanup failed: \(error.localizedDescription) Reload configuration to retry."}
    }
    func applySystemResults(_ commands:[Command],resources:[String:ProcessResource],preserveSelection:Bool) {
        let selectedID=preserveSelection ? selectedCommand?.id:nil
        processResources=resources;results=commands;rememberServices(commands)
        if preserveSelection {selection=selectedID.flatMap {id in visibleCommands.firstIndex(where:{$0.id==id})} ?? -1}
    }
    func rebuildProcessOrder() {
        guard mode=="processes" else {processOrder=[];return}
        processOrder=results.sorted {a,b in
                guard let x=processResources[a.id],let y=processResources[b.id] else {return a.id<b.id}
                switch processSort {case .cpu:if x.cpuPercent != y.cpuPercent {return x.cpuPercent>y.cpuPercent};case .memory:if x.memoryBytes != y.memoryBytes {return x.memoryBytes>y.memoryBytes};case .name:break}
                return x.name==y.name ? x.id<y.id:x.name.localizedStandardCompare(y.name) == .orderedAscending
            }
    }
    func setProcessSort(_ sort:ProcessSort) {let id=selectedCommand?.id;processSort=sort;selection=id.flatMap {id in visibleCommands.firstIndex(where:{$0.id==id})} ?? -1}
    func refreshProcessResults() {refreshResults(preserveSelection:true)}
    private func tick() {
        if paletteVisible,mode=="processes",Date().timeIntervalSince(lastProcessRefresh)>2 {lastProcessRefresh=Date();refreshProcessResults()}
        if Date().timeIntervalSince(lastPruned)>60,!clipboardWorkInFlight {lastPruned=Date();_=processClipboardPayload(nil,source:"maintenance")}
        guard !headless,!paused,config.clipboard.enabled,!clipboardWorkInFlight else {return}
        let active=NSWorkspace.shared.frontmostApplication
        let uncertain=clipboardAttributionUncertain || active?.processIdentifier != lastClipboardPID
        clipboardAttributionUncertain=false;lastClipboardPID=active?.processIdentifier
        guard !clipboardWorkInFlight else {return}
        let pb=NSPasteboard.general
        guard changeCount != pb.changeCount else {return};changeCount=pb.changeCount
        guard !uncertain else {return}
        let skip=["org.nspasteboard.ConcealedType","org.nspasteboard.TransientType","org.nspasteboard.AutoGeneratedType"]
        guard !(pb.types ?? []).contains(where:{skip.contains($0.rawValue)}) else {return}
        let app=active?.bundleIdentifier ?? "unknown"
        guard !config.clipboard.excludedApps.contains(app) else {return}
        do {
            guard let payload=try clipboardStore.snapshot(from:pb,settings:config.clipboard),pb.changeCount==changeCount,NSWorkspace.shared.frontmostApplication?.processIdentifier==active?.processIdentifier else {return}
            _=processClipboardPayload(payload,source:app)
        } catch {message="Clipboard could not be saved: \(error.localizedDescription)";analytics.recordError("clipboard",code:"clipboard-io")}
    }
    @discardableResult func processClipboardPayload(_ payload:ClipboardPayload?,source:String,completion:(()->Void)?=nil)->Bool {
        guard !clipboardWorkInFlight else {return false};clipboardWorkInFlight=true
        let token=clipboardWork.revision,base=clips,settings=config.clipboard,store=clipboardStore,url=root.appendingPathComponent("clipboard.json"),capturedAt=Date(),started=DispatchTime.now().uptimeNanoseconds
        clipboardWork.perform { [weak self] in
            guard let self else {return}
            var cleanupError:String?
            let result=Result { () throws -> [Clip] in
                guard self.clipboardWork.current(token) else {return base}
                var clip=try payload.flatMap {try store.capture($0,source:source,settings:settings)}
                clip?.date=Date(timeIntervalSince1970:capturedAt.timeIntervalSince1970.rounded(.down))
                let next=try History.updateChecked(base,clip:clip,settings:settings)
                guard self.clipboardWork.current(token) else {return base}
                if next != base {try Files.save(next,to:url)}
                if self.clipboardWork.current(token) {do {try store.pruneAssets(keeping:next,settings:settings)} catch {cleanupError=error.localizedDescription}}
                return next
            }
            DispatchQueue.main.async {
                self.clipboardWorkInFlight=false
                guard self.clipboardWork.current(token) else {self.cleanupClipboardAssets();completion?();return}
                switch result {
                case .success(let next):
                    if next != self.clips {self.clips=next}
                    if let cleanupError {self.clipboardCleanupError="Clipboard cleanup failed: \(cleanupError) Reload configuration to retry."}
                    else if self.clipboardCleanupError != nil {self.clipboardCleanupError=nil}
                case .failure(let error):self.message="Clipboard could not be saved: \(error.localizedDescription)";self.analytics.recordError("clipboard",code:"clipboard-io")
                }
                if payload != nil {self.analytics.recordTiming("clipboard.capture",ms:AnalyticsStore.elapsed(started))};completion?()
            }
        }
        return true
    }
    func shutdown() {clipboardSearch.cancel();configObserver=nil;timer?.invalidate();clipboardWork.drain();analytics.flushSynchronously();commandLog.flush()}
    func configurationReads(_ completion:@escaping(Int)->Void) {if let configObserver {configObserver.reads(completion)} else {completion(0)}}
    func ignoreOwnClipboardChange() {changeCount=NSPasteboard.general.changeCount}
    func copyClip(_ id:String,paste:Bool=false) throws {
        guard let clip=clips.first(where:{$0.id==id}) else {throw LauncherError("Clipboard item not found.")}
        if paste,!windows.trusted {throw LauncherError("Allow Accessibility in System Settings before using Paste.")}
        let pb=NSPasteboard.general;try clipboardStore.copy(clip,to:pb);changeCount=pb.changeCount
        if paste {
            guard let pid=previousPID,let app=NSRunningApplication(processIdentifier:pid) else {throw LauncherError("Text copied. The previous app is no longer available.")}
            hidePalette?();app.activate(options:[])
            DispatchQueue.main.asyncAfter(deadline:.now()+0.15) {
                guard NSWorkspace.shared.frontmostApplication?.processIdentifier==pid else {self.showToast?("Copied · target app did not activate",false);return}
                for down in [true,false] {let event=CGEvent(keyboardEventSource:nil,virtualKey:9,keyDown:down);event?.flags = .maskCommand;event?.setIntegerValueField(.eventSourceUserData,value:HyperKey.generatedTag);event?.post(tap:.cghidEventTap)}
            }
        } else {hidePalette?();showToast?("Copied",true)}
    }
    func perform(_ id:String,values:[String:String]=[:],confirmed:Bool=false,source:String="launcher") {
        if let command=resolveCommand(id),systemServices.requiresConfirmation(target:command.target),!confirmed {confirmationCommand=command;return}
        guard !busy else {return};busy=true
        execute(id,dryRun:false,values:values,openResult:true,confirmed:confirmed,source:source) { [weak self] response in
            guard let self else {return};self.busy=false
            if response["ok"] as? Bool == true {
                if self.resolveCommand(id)?.kind != "builtin" && self.detail==nil {self.form=nil;self.hidePalette?();self.showToast?("Done",true)}
            } else {self.message=response["error"] as? String ?? "Action failed.";self.showToast?(self.message,false)}
        }
    }
    private func targetPID()->pid_t? {
        if let app=NSWorkspace.shared.frontmostApplication,app.processIdentifier != getpid() {return app.processIdentifier}
        return previousPID
    }
    func execute(_ id:String,dryRun:Bool,values:[String:String]=[:],openResult:Bool=false,confirmed:Bool=false,source:String="cli",completion:@escaping ([String:Any])->Void) {
        if config.commands.contains(where:{!$0.isEnabled && ($0.id.lowercased()==id.lowercased() || $0.aliases.contains(where:{$0.lowercased()==id.lowercased()}))}) {completion(["ok":false,"error":"Command is disabled: \(id)"]);return}
        guard var cmd=resolveCommand(id) ?? applications.first(where:{$0.id==id}) else {completion(["ok":false,"error":"Unknown command: \(id)"]);return}
        guard cmd.isEnabled,!config.commands.contains(where:{!$0.isEnabled && ($0.id==id || ($0.kind=="app" && cmd.kind=="app" && $0.target==cmd.target))}) else {completion(["ok":false,"error":"Command is disabled."]);return}
        if dryRun {completion(["ok":true,"result":jsonValue(cmd),"dryRun":true]);return}
        if let prompt=cmd.inputPrompt,(values["input"] ?? "").trimmingCharacters(in:.whitespacesAndNewlines).isEmpty {completion(["ok":false,"error":"Enter \(prompt)."]);return}
        cmd.arguments=cmd.arguments.map {$0=="{input}" ? values["input"] ?? "":$0=="{uuid}" ? UUID().uuidString:$0}
        let started=DispatchTime.now().uptimeNanoseconds,tracked=analytics.recordAttempt(cmd.id,source:source)
        let traceID=UUID().uuidString
        self.commandLog.record(CommandLogEntry(traceID:traceID,commandID:CommandLog.clean(cmd.id,limit:512),source:source,phase:"command.start"))
        let finish:([String:Any])->Void={ original in
            var result=original
            let ok=result["ok"] as? Bool==true,elapsed=AnalyticsStore.elapsed(started)
            let secrets=Array(values.values)
            var details:[String:String]=[:]
            if let raw=result["diagnostics"] as? [String:Any] {
                for key in ["provider","providerExitCode","providerStderr","exception","runnerError"] {
                    if let value=raw[key] {details[key]=CommandLog.clean(String(describing:value),secrets:secrets,limit:1000)}
                }
            }
            self.commandLog.record(CommandLogEntry(traceID:traceID,commandID:CommandLog.clean(cmd.id,limit:512),source:source,phase:"command.finish",success:ok,durationMS:elapsed,code:CommandLog.clean(result["code"] as? String ?? (ok ? "success":AnalyticsStore.failureCode(result["error"] as? String ?? "")),limit:80),message:ok ? nil:CommandLog.clean(result["error"] as? String ?? "Action failed",secrets:secrets),exitStatus:result["exitCode"] as? Int,applied:result["applied"] as? Bool ?? result["actionCompleted"] as? Bool,details:details.isEmpty ? nil:details))
            if !ok {result["diagnosticID"]=traceID}

            if let tracked {self.analytics.recordCompletion(tracked,success:result["ok"] as? Bool==true,ms:AnalyticsStore.elapsed(started),code:AnalyticsStore.failureCode(result["error"] as? String ?? ""),exitStatus:result["exitCode"] as? Int)}
            if result["ok"] as? Bool == true {
                self.counts[cmd.id,default:0]+=1
                do {try Files.save(self.counts,to:self.root.appendingPathComponent("usage.json"))}
                catch {completion(["ok":false,"error":"Action completed, but usage could not be saved: \(error.localizedDescription)","actionCompleted":true]);return}
            };completion(result)
        }
        do {
            switch cmd.kind {
            case "snippet":
                let rendered=try Templates.render(cmd,values:values,clipboard:cmd.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil)
                try pasteText(rendered.0,cursor:rendered.1)
                finish(["ok":true,"result":["state":"pasted"]])
            case "app":
                guard let url=NSWorkspace.shared.urlForApplication(withBundleIdentifier:cmd.target) else {throw LauncherError("App is not installed: \(cmd.target)")}
                NSWorkspace.shared.openApplication(at:url,configuration:NSWorkspace.OpenConfiguration()) {_,error in DispatchQueue.main.async {if let error {finish(["ok":false,"error":error.localizedDescription])} else {finish(["ok":true,"result":["command":cmd.id,"state":"opened"]])}}}
            case "url","path":
                let url:URL
                if cmd.kind=="url" {cmd.target=try Templates.render(cmd,values:values,clipboard:cmd.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil).0}
                if cmd.kind=="path" {let path=NSString(string:cmd.target).expandingTildeInPath;guard FileManager.default.fileExists(atPath:path) else {throw LauncherError("Path does not exist: \(path)")};url=URL(fileURLWithPath:path)} else {guard let parsed=URL(string:cmd.target) else {throw LauncherError("Invalid URL.")};url=parsed}
                if let applicationID=cmd.applicationID {
                    guard let appURL=NSWorkspace.shared.urlForApplication(withBundleIdentifier:applicationID) else {throw LauncherError("The selected app is not installed. Choose another app in Settings.")}
                    NSWorkspace.shared.open([url],withApplicationAt:appURL,configuration:NSWorkspace.OpenConfiguration()) {_,error in DispatchQueue.main.async {if let error {finish(["ok":false,"error":error.localizedDescription])} else {finish(["ok":true,"result":["state":"opened"]])}}}
                } else {guard NSWorkspace.shared.open(url) else {throw LauncherError("macOS could not open \(cmd.title).")};finish(["ok":true,"result":["command":cmd.id,"state":"opened"]])}
            case "window": let f=try windows.move(cmd.target,pid:targetPID());finish(["ok":true,"result":jsonValue(f)])
            case "builtin":
                if cmd.target=="reload-config" {try reload();showToast?("Configuration reloaded",true);finish(["ok":true,"result":["state":"reloaded"]]);return}
                if cmd.target=="open-config" {guard !headless else {throw LauncherError("Open Configuration requires the app interface.")};NSWorkspace.shared.open(configURL);finish(["ok":true,"result":["state":"opened"]]);return}
                if cmd.target.hasPrefix("utility-") || cmd.target.hasPrefix("utility.history:") {
                    if cmd.target.hasSuffix("history") {showPalette?(cmd.target);finish(["ok":true]);return}
                    switch try utilities.execute(target:cmd.target,input:values["input"]) {
                    case .saved(let text):hidePalette?();showToast?(text,true)
                    case .paste(let text):try pasteText(text)
                    case .copy(let text,let secret):
                        let board=NSPasteboard.general;board.clearContents()
                        if secret {_=board.setData(Data(),forType:NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))}
                        guard board.setString(text,forType:.string) else {throw LauncherError("Could not copy text.")}
                        ignoreOwnClipboardChange();hidePalette?();showToast?("Copied",true)
                    };finish(["ok":true]);return
                }
                if cmd.target=="system-monitor" {guard !headless else {throw LauncherError("The monitor window is unavailable in headless mode. Use ruycast monitor for data.")};hidePalette?();showSystemMonitor?();finish(["ok":true,"result":["state":"opened"]]);return}
                if cmd.target.hasPrefix("system.") || cmd.target=="lock-screen" {
                    let response=systemServices.execute(target:cmd.target,confirmed:confirmed,previousPID:targetPID())
                    if response["ok"] as? Bool==true {if cmd.target.hasPrefix("system.kill:") {systemServices.invalidateProcessCache();refreshProcessResults();showToast?("Killed "+cmd.title,true)} else {hidePalette?()}};finish(response);return
                }
                guard !headless else {throw LauncherError("The palette is unavailable in headless mode.")}
                if cmd.target=="analytics" {hidePalette?();showAnalytics?()}
                else if cmd.target=="settings" {hidePalette?();showManagement?()}
                else if cmd.target=="create-snippet" {newCommand("snippet")}
                else if cmd.target=="create-quicklink" {newCommand("url")}
                else {let modes=["search-files":"files","search-menu-items":"menus","switch-windows":"windows","kill-process":"processes"];showPalette?(modes[cmd.target] ?? cmd.target)}
                finish(["ok":true,"result":["state":"opened"]])
            case "script":runScript(cmd,openResult:openResult,completion:finish)
            default:throw LauncherError("Unsupported command kind.")
            }
        } catch {finish(["ok":false,"error":error.localizedDescription])}
    }
    private func runScript(_ cmd:Command,openResult:Bool,completion:@escaping ([String:Any])->Void) {
        DispatchQueue.global(qos:.userInitiated).async {
            let process=Process()
            let bundled=Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/ruycast")
            let source=URL(fileURLWithPath:#filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent(".build/rust/release/ruycast")
            process.executableURL=FileManager.default.fileExists(atPath:bundled.path) ? bundled:source
            process.arguments=["__run-script"]
            let input=Pipe(),out=Pipe();process.standardInput=input;process.standardOutput=out;process.standardError=FileHandle.nullDevice
            do {
                try process.run()
                try input.fileHandleForWriting.write(contentsOf:Files.encode(cmd));try input.fileHandleForWriting.close()
                // The trusted runner owns and closes its output independently of child scripts.
                let data=out.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit()
                guard let result=try JSONSerialization.jsonObject(with:data) as? [String:Any] else {throw LauncherError("Script runner returned an invalid response.")}
                DispatchQueue.main.async {
                    if cmd.arguments.contains(where:{$0.hasSuffix("personal_bridge.py")}) {
                        guard let value=(result["result"] as? [String:Any]) ?? result as [String:Any]?,let stdout=value["stdout"] as? String,let bytes=stdout.data(using:.utf8),let native=try? JSONSerialization.jsonObject(with:bytes) as? [String:Any] else {completion(["ok":false,"error":"The app returned an invalid response. Check its current state before retrying.","exitCode":result["exitCode"] as Any? ?? NSNull(),"diagnostics":["runnerError":String((result["stderr"] as? String ?? "").suffix(2000))]]);return}
                        var response=native
                        if let status=value["exitCode"] {response["exitCode"]=status}
                        if result["ok"] as? Bool==false,native["ok"] as? Bool==true {response["ok"]=false;response["error"]=result["error"] ?? "Provider process failed."}
                        if response["ok"] as? Bool==true,openResult,let text=response["detail"] as? String {self.showPalette?("report");self.detail=text}
                        completion(response)
                    } else if cmd.id.hasPrefix("capture."),result["ok"] as? Bool==true {
                        guard let value=result["result"] as? [String:Any],let stdout=value["stdout"] as? String,let bytes=stdout.data(using:.utf8),
                              let capture=try? JSONSerialization.jsonObject(with:bytes) as? [String:Any] else {completion(["ok":false,"error":"Capture returned an invalid response. Check the inbox before retrying."]);return}
                        guard capture["ok"] as? Bool==true else {completion(["ok":false,"error":capture["error"] as? String ?? "Capture failed."]);return}
                        if let error=capture["daily_error"] as? String,!error.isEmpty {completion(["ok":false,"error":"Saved · daily link failed: \(error)","actionCompleted":true]);return}
                        if openResult,!self.headless,cmd.id=="capture.quick-note",let path=capture["note_path"] as? String {
                            var parts=URLComponents();parts.scheme="obsidian";parts.host="open";parts.queryItems=[URLQueryItem(name:"path",value:path)]
                            guard let url=parts.url,NSWorkspace.shared.open(url) else {completion(["ok":false,"error":"Note saved · could not open Obsidian.","actionCompleted":true]);return}
                        }
                        completion(["ok":true,"result":capture])
                    } else {completion(result)}
                }
            } catch {DispatchQueue.main.async {completion(["ok":false,"error":error.localizedDescription])}}
        }
    }
    func handle(_ req:[String:Any],reply:@escaping ([String:Any])->Void) {
        let started=DispatchTime.now().uptimeNanoseconds
        defer {if !((req["op"] as? String ?? "")=="analytics" || (req["op"] as? String ?? "").hasPrefix("analytics.")) {analytics.recordTiming("cli.dispatch",ms:AnalyticsStore.elapsed(started))}}
        do {
            switch req["op"] as? String {
            case "logs":
                reply(["ok":true,"result":jsonValue(commandLog.entries(commandID:req["commandID"] as? String,errorsOnly:req["errorsOnly"] as? Bool ?? false,limit:req["limit"] as? Int ?? 50)),"path":commandLog.url.path,"storageError":commandLog.storageError as Any? ?? NSNull()])
            case "logs.path":reply(["ok":true,"result":["path":commandLog.url.path]])
            case "logs.clear":
                guard req["confirmed"] as? Bool==true else {throw LauncherError("Confirm clearing command logs.")}
                commandLog.clear();reply(["ok":commandLog.storageError==nil,"storageError":commandLog.storageError as Any? ?? NSNull()])
            case "analytics":reply(["ok":true,"result":jsonValue(analytics.snapshot),"storageError":analytics.storageError as Any? ?? NSNull()])
            case "analytics.export":reply(["ok":true,"result":analytics.exportSummary()])
            case "analytics.settings":
                try analytics.configure(enabled:req["enabled"] as? Bool,retentionDays:req["retentionDays"] as? Int)
                analytics.flush {reply(["ok":self.analytics.storageError==nil,"result":jsonValue(self.analytics.snapshot),"storageError":self.analytics.storageError as Any? ?? NSNull()])}
            case "analytics.reset":
                guard req["confirmed"] as? Bool==true else {throw LauncherError("Confirm resetting analytics.")}
                analytics.reset();analytics.flush {reply(["ok":self.analytics.storageError==nil,"result":jsonValue(self.analytics.snapshot)])}
            case "monitor":
                monitorWorker.async {let snapshot=self.monitorSampler.sample();DispatchQueue.main.async {reply(["ok":true,"result":jsonValue(snapshot)])}}
            case "processes":
                systemServices.processDetails(req["query"] as? String ?? "") {result in
                    switch result {case .success(let rows):reply(["ok":true,"result":rows.map {["command":jsonValue($0.0),"metrics":jsonValue($0.1)]}]);case .failure(let error):reply(["ok":false,"error":error.localizedDescription])}
                }
            case "status":
                var status:[String:Any]=["version":"0.7.6","name":"Ruycast","pid":getpid(),"profile":root.path,"configPath":configURL.path,"configDirectory":configDirectory.path,"headless":headless,"commands":config.commands.count,"discoveredApps":applications.count,"clipboardCount":clips.count,"snippetExpansion":expansionStatus,"keyboard":hotkeys?.keyboardStatus ?? "Off","hyper":hotkeys?.hyperStatus ?? "Off","modifierTaps":hotkeys?.modifierStatus ?? "Off","clipboardEnabled":config.clipboard.enabled,"clipboardPaused":paused,"accessibility":windows.trusted,"hotkeyWarnings":hotkeys?.warnings ?? [],"configError":configError as Any? ?? NSNull(),"clipboardCleanupError":clipboardCleanupError as Any? ?? NSNull()]
                status["inputMonitoring"]=CGPreflightListenEventAccess();status["separateCommandKeys"]=config.separatesCommandKeys;status["theme"]=config.theme ?? "system";status["menuBarVisible"]=config.menuBarVisible ?? true;status["health"]=runtimeHealthy ? "healthy":"needs-attention"
                reply(["ok":true,"result":status])
            case "search":
                let query=req["query"] as? String ?? "",scope=req["mode"] as? String ?? "commands"
                if ["files","menus","windows","processes"].contains(scope) {
                    systemServices.results(mode:scope,query:query,previousPID:targetPID()) {result in
                        switch result {case .success(let commands):self.rememberServices(commands);reply(["ok":true,"result":jsonValue(commands)]);case .failure(let error):reply(["ok":false,"error":error.localizedDescription])}
                    }
                } else if scope.hasPrefix("utility-") {let commands=try utilities.commands(mode:scope,query:query);rememberServices(commands);reply(["ok":true,"result":jsonValue(commands)])}
                else {reply(["ok":true,"result":jsonValue(searchCommands(query,mode:scope,limit:req["limit"] as? Int ?? 100))])}
            case "commands":reply(["ok":true,"result":jsonValue(config.commands)])
            case "execute":guard let id=req["id"] as? String else {throw LauncherError("Missing command id.")};execute(id,dryRun:req["dryRun"] as? Bool ?? false,values:req["values"] as? [String:String] ?? [:],confirmed:req["confirmed"] as? Bool ?? false,completion:reply)
            case "config.directory":
                guard let path=req["directory"] as? String else {throw LauncherError("Missing configuration folder.")}
                try relocateConfiguration(to:path);reply(["ok":true,"result":["directory":configDirectory.path,"path":configURL.path,"originalRetained":true]])
            case "reload":
                if let expected=req["configPath"] as? String,expected != configURL.path {throw LauncherError("Configuration folder differs from the running app. Restart Ruycast to use the selected folder.")}
                try reload();reply(["ok":true,"result":["commands":config.commands.count,"hotkeyWarnings":hotkeys?.warnings ?? []]])
            case "show":guard !headless else {throw LauncherError("The palette is unavailable in headless mode.")};showPalette?(req["mode"] as? String ?? "commands");reply(["ok":true,"result":["state":"opened"]])
            case "utilities.import":
                guard let profile=req["profile"] as? [String:Any] else {throw LauncherError("Missing utilities profile.")}
                reply(["ok":true,"result":try utilities.importProfile(profile)])
            case "clipboard.import":
                guard let raw=req["items"] as? [[String:Any]],raw.count<=100 else {throw LauncherError("Import at most 100 clipboard items per batch.")}
                let incoming=try Files.decode([Clip].self,data:JSONSerialization.data(withJSONObject:raw))
                let fallback=clips,settings=config.clipboard
                var accepted=0;var duplicated=0;var excluded=0
                let next=try clipboardWork.sync { () throws -> [Clip] in
                var next=try self.persistedHistory(fallback:fallback)
                for var clip in incoming.reversed() {
                    guard clip.id.hasPrefix("raycast-"),["text","files"].contains(clip.contentType ?? "text"),clip.dataFile==nil else {throw LauncherError("Invalid imported clipboard item.")}
                    if clip.source.hasPrefix("/"),let bundle=Bundle(path:clip.source)?.bundleIdentifier {clip.source=bundle}
                    if settings.excludedApps.contains(clip.source) {excluded+=1;continue}
                    if next.contains(where:{$0.id==clip.id || ($0.text==clip.text && $0.fileURLs==clip.fileURLs)}) {duplicated+=1;continue}
                    next.append(clip);accepted+=1
                }
                next.sort {$0.date>$1.date};next=try History.updateChecked(next,clip:nil,settings:settings)
                try Files.save(next,to:self.root.appendingPathComponent("clipboard.json"));return next
                };clips=next
                reply(["ok":true,"result":["accepted":accepted,"deduplicated":duplicated,"excluded":excluded,"retained":next.count]])
            case "clipboard.list":
                let q=req["query"] as? String ?? "",snapshot=clips
                DispatchQueue.global(qos:.userInitiated).async {autoreleasepool {reply(["ok":true,"result":jsonValue(snapshot.filter {q.isEmpty || $0.text.localizedCaseInsensitiveContains(q)})])}}
            case "clipboard.copy","clipboard.paste":guard let id=req["id"] as? String else {throw LauncherError("Missing clipboard id.")};try copyClip(id,paste:req["op"] as? String=="clipboard.paste");reply(["ok":true,"result":["state":"copied"]])
            case "clipboard.clear","clipboard.delete":
                let fallback=clips
                let next=try clipboardWork.sync { () throws -> [Clip] in
                    let base=try self.persistedHistory(fallback:fallback),next:[Clip]
                    if req["op"] as? String=="clipboard.delete" {guard let id=req["id"] as? String,base.contains(where:{$0.id==id}) else {throw LauncherError("Clipboard item not found.")};next=base.filter {$0.id != id}} else {next=[]}
                    try Files.save(next,to:self.root.appendingPathComponent("clipboard.json"));return next
                };clips=next;try clipboardWork.sync {try clipboardStore.pruneAssets(keeping:clips,settings:config.clipboard)};reply(["ok":true,"result":["count":clips.count]])
            case "clipboard.pause":let next=req["paused"] as? Bool ?? true;try Files.save(next,to:root.appendingPathComponent("paused.json"));paused=next;reply(["ok":true,"result":["paused":paused]])
            case "usage":reply(["ok":true,"result":counts])
            case "quit":reply(["ok":true,"result":["state":"stopping"]]);DispatchQueue.main.asyncAfter(deadline:.now()+0.25) {NSApp.terminate(nil)}
            default:throw LauncherError("Unknown operation.")
            }
        } catch {reply(["ok":false,"error":error.localizedDescription])}
    }
}
