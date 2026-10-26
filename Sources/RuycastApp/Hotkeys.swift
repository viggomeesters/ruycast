import Carbon
import LauncherCore

final class Hotkeys {
    private var refs:[String:EventHotKeyRef]=[:]
    private var ids:[String:UInt32]=[:]
    private var routes:[UInt32:String]=[:]
    private var nextID:UInt32=1
    private var handler:EventHandlerRef?
    private let keyboard=HyperKey()
    var keyboardStatus:String {keyboard.status}
    var hyperStatus:String {keyboard.hyperStatus}
    var modifierStatus:String {keyboard.modifierStatus}
    func refreshKeyboard() {keyboard.refresh()}
    var onAction:((String)->Void)?
    var warnings:[String]=[]
    init() {
        keyboard.onAction={ [weak self] action in PerformanceTrace.shared.measure(.hotkeyHandler) {self?.onAction?(action)} }
        var event=EventTypeSpec(eventClass:OSType(kEventClassKeyboard),eventKind:UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(),{_,event,user in
            guard let event,let user else {return OSStatus(eventNotHandledErr)}
            let instance=Unmanaged<Hotkeys>.fromOpaque(user).takeUnretainedValue()
            var id=EventHotKeyID();GetEventParameter(event,EventParamName(kEventParamDirectObject),EventParamType(typeEventHotKeyID),nil,MemoryLayout<EventHotKeyID>.size,nil,&id)
            if let action=instance.routes[id.id] {PerformanceTrace.shared.measure(.hotkeyHandler) {instance.onAction?(action)}}
            return noErr
        },1,&event,Unmanaged.passUnretained(self).toOpaque(),&handler)
    }
    private var suspended=false
    private var latest:Configuration?
    func suspend() {
        suspended=true
        for ref in refs.values {UnregisterEventHotKey(ref)}
        refs=[:];ids=[:];routes=[:]
        keyboard.update(hyper:latest?.hyperKeyEnabled==true,bindings:[:])
    }
    func resume() throws {suspended=false;if let latest {try update(latest)}}
    private static func isSided(_ value:String)->Bool {value.lowercased().split(separator:"+").dropLast().contains(where:{$0=="left-cmd" || $0=="right-cmd"})}
    static func hasCommandBindings(in config:Configuration)->Bool {
        config.separatesCommandKeys && ([config.launcherHotkey,config.clipboardHotkey].contains(where:isSided) || config.commands.contains {$0.isEnabled && $0.hotkey.map(isSided)==true})
    }
    static func commandBindings(in config:Configuration)->[String:String] {
        guard config.separatesCommandKeys else {return [:]}
        var result:[String:String]=[:]
        let candidates=[(config.launcherHotkey,"@launcher"),(config.clipboardHotkey,"@clipboard")]+config.commands.filter(\.isEnabled).compactMap {command in command.hotkey.map {($0,command.id)}}
        for (value,action) in candidates {
            if isSided(value),let shortcut=try? Shortcut(value),shortcut.commandSide != nil {result[shortcut.canonical]=action}
        }
        return result
    }
    func update(_ config:Configuration) throws {
        if suspended {latest=config;keyboard.update(hyper:config.hyperKeyEnabled==true,bindings:[:]);return}
        var desired:[String:String]=[:]
        desired[(try Shortcut(config.launcherHotkey)).canonical]="@launcher"
        desired[(try Shortcut(config.clipboardHotkey)).canonical]="@clipboard"
        for c in config.commands where c.isEnabled {if let h=c.hotkey {desired[(try Shortcut(h)).canonical]=c.id}}
        var modifierBindings:[UInt32:String]=[:]
        let commandBindings=Self.commandBindings(in:config)
        for key in Array(desired.keys) {
            let shortcut=try Shortcut(key)
            if shortcut.commandSide != nil {desired.removeValue(forKey:key);continue}
            if shortcut.key=="left-control" || shortcut.key=="left-option" {
                let expected:Set<String>=shortcut.key=="left-control" ? ["ctrl"]:["alt"]
                guard shortcut.modifiers==expected else {throw LauncherError("Use Ctrl+Left-Control or Alt+Left-Option for modifier taps.")}
                modifierBindings[shortcut.key=="left-control" ? 0xe0:0xe2]=desired.removeValue(forKey:key)
            }
        }
        var staged:[String:EventHotKeyRef]=[:]
        var stagedIDs:[String:UInt32]=[:]
        warnings=[]
        // Register additions before releasing old keys. A conflict leaves the old set intact.
        for key in desired.keys.sorted() where refs[key]==nil {
            let shortcut=try Shortcut(key)
            var modifiers:UInt32=0
            for m in shortcut.modifiers {modifiers |= ["ctrl":UInt32(controlKey),"alt":UInt32(optionKey),"shift":UInt32(shiftKey),"cmd":UInt32(cmdKey)][m]!}
            let id=nextID;nextID+=1;var ref:EventHotKeyRef?
            let status=RegisterEventHotKey(Shortcut.keyCodes[shortcut.key]!,modifiers,EventHotKeyID(signature:0x52554341,id:id),GetApplicationEventTarget(),0,&ref)
            if status==noErr,let ref {staged[key]=ref;stagedIDs[key]=id}
            else {warnings.append("Hotkey \(key) is unavailable (\(status)); choose another binding.")}
        }
        if !warnings.isEmpty {for ref in staged.values {UnregisterEventHotKey(ref)};throw LauncherError(warnings.joined(separator:" "))}
        for key in Array(refs.keys) where desired[key]==nil {UnregisterEventHotKey(refs.removeValue(forKey:key)!);ids.removeValue(forKey:key)}
        refs.merge(staged) {_,new in new};ids.merge(stagedIDs) {_,new in new}
        routes=Dictionary(uniqueKeysWithValues:desired.compactMap {key,action in ids[key].map {($0,action)}})
        latest=config
        keyboard.update(hyper:config.hyperKeyEnabled==true,bindings:modifierBindings,commandBindings:commandBindings)
    }
    deinit {for ref in refs.values {UnregisterEventHotKey(ref)};if let handler {RemoveEventHandler(handler)}}
}
