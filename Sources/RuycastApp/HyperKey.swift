import AppKit
import Carbon
import IOKit.hid
import IOKit.hidsystem
import LauncherCore

/// Owns one nonexclusive HID observer and one event filter. Never seizes a keyboard
/// or installs persistent mappings. Raw usage 0x39 provides real Caps down AND up;
/// flagsChanged alone reports the Caps lock latch, not physical release.
final class HyperKey {
    static let generatedTag:Int64=0x52554859504552
    static let hyper:CGEventFlags=[.maskControl,.maskAlternate,.maskShift,.maskCommand]
    private var manager:IOHIDManager?
    private var capsConnection:io_connect_t=0
    private var originalCaps=false
    private var tap:CFMachPort?
    private var source:CFRunLoopSource?
    private var timer:Timer?
    private var observers:[NSObjectProtocol]=[]
    private var state=KeyboardState()
    private var hyperEnabled=false
    private var bindings:[UInt32:String]=[:]
    private var commandBindings:[String:String]=[:]
    private var commandFilter=CommandShortcutFilter()
    private var sleeping=false
    private(set) var status="Off"
    var onAction:((String)->Void)?
    var hyperStatus:String {hyperEnabled ? status:"Off"}
    var modifierStatus:String {bindings.isEmpty && commandBindings.isEmpty ? "Off":status}
    init() {
        let center=NSWorkspace.shared.notificationCenter
        for name in [NSWorkspace.willSleepNotification,NSWorkspace.sessionDidResignActiveNotification] {
            observers.append(center.addObserver(forName:name,object:nil,queue:.main) {[weak self] _ in self?.sleeping=true;self?.stop();self?.status="Paused"})
        }
        for name in [NSWorkspace.didWakeNotification,NSWorkspace.sessionDidBecomeActiveNotification,NSWorkspace.didLaunchApplicationNotification,NSWorkspace.didTerminateApplicationNotification] {
            observers.append(center.addObserver(forName:name,object:nil,queue:.main) {[weak self] _ in self?.sleeping=false;self?.refresh()})
        }
        timer=Timer.scheduledTimer(withTimeInterval:2,repeats:true) {[weak self] _ in self?.refresh()}
    }
    deinit {timer?.invalidate();for observer in observers {NSWorkspace.shared.notificationCenter.removeObserver(observer)};stop()}
    func update(hyper:Bool,bindings:[UInt32:String],commandBindings:[String:String]=[:]) {
        if hyperEnabled != hyper || self.bindings != bindings || self.commandBindings != commandBindings {stop()}
        hyperEnabled=hyper;self.bindings=bindings;self.commandBindings=commandBindings
        commandFilter.bindings=commandBindings.compactMap {key,action in
            guard let shortcut=try? Shortcut(key) else {return nil}
            return CommandShortcutFilter.Binding(shortcut:shortcut,action:action)
        }
        refresh()
    }
    func refresh() {
        let reason:String
        if !hyperEnabled && bindings.isEmpty && commandBindings.isEmpty {reason="Off"}
        else if sleeping {reason="Paused"}
        else if !NSRunningApplication.runningApplications(withBundleIdentifier:"com.raycast.macos").isEmpty {reason="Quit Raycast to use keyboard shortcuts"}
        else if IsSecureEventInputEnabled() {reason="Paused during secure input"}
        else if !AXIsProcessTrusted() {reason="Allow Accessibility in System Settings"}
        else if !CGPreflightListenEventAccess() {reason="Allow Input Monitoring in System Settings"}
        else {reason=""}
        if !reason.isEmpty {stop();status=reason;return}
        if let tap,CGEvent.tapIsEnabled(tap:tap) {status="On";return}
        stop()
        if hyperEnabled {
            let service=IOServiceGetMatchingService(kIOMainPortDefault,IOServiceMatching("IOHIDSystem"))
            guard service != 0 else {status="Caps Lock control unavailable";return}
            let opened=IOServiceOpen(service,mach_task_self_,UInt32(kIOHIDParamConnectType),&capsConnection)
            IOObjectRelease(service)
            guard opened==KERN_SUCCESS,IOHIDGetModifierLockState(capsConnection,Int32(kIOHIDCapsLockState),&originalCaps)==KERN_SUCCESS,
                  IOHIDSetModifierLockState(capsConnection,Int32(kIOHIDCapsLockState),false)==KERN_SUCCESS else {
                if capsConnection != 0 {IOServiceClose(capsConnection);capsConnection=0}
                status="Caps Lock control unavailable; check Accessibility";return
            }
        }
        let mask:CGEventMask=[CGEventType.keyDown,.keyUp,.flagsChanged,.leftMouseDown,.rightMouseDown,.otherMouseDown,.scrollWheel].reduce(0) {$0 | (1 << $1.rawValue)}
        guard let port=CGEvent.tapCreate(tap:.cgSessionEventTap,place:.headInsertEventTap,options:.defaultTap,eventsOfInterest:mask,callback:{_,type,event,context in
            guard let context else {return Unmanaged.passUnretained(event)}
            return Unmanaged<HyperKey>.fromOpaque(context).takeUnretainedValue().filter(type,event)
        },userInfo:Unmanaged.passUnretained(self).toOpaque()) else {status="Keyboard filter unavailable; check Accessibility";return}
        tap=port;source=CFMachPortCreateRunLoopSource(kCFAllocatorDefault,port,0)
        CFRunLoopAddSource(CFRunLoopGetMain(),source,.commonModes)
        let hid=IOHIDManagerCreate(kCFAllocatorDefault,IOOptionBits(kIOHIDOptionsTypeNone))
        IOHIDManagerSetDeviceMatching(hid,[kIOHIDDeviceUsagePageKey:1,kIOHIDDeviceUsageKey:6] as CFDictionary)
        IOHIDManagerSetInputValueMatching(hid,[kIOHIDElementUsagePageKey:7] as CFDictionary)
        IOHIDManagerRegisterInputValueCallback(hid,{context,result,_,value in
            guard result==kIOReturnSuccess,let context else {return}
            Unmanaged<HyperKey>.fromOpaque(context).takeUnretainedValue().input(value)
        },Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerRegisterDeviceRemovalCallback(hid,{context,_,_,device in
            guard let context else {return}
            let instance=Unmanaged<HyperKey>.fromOpaque(context).takeUnretainedValue()
            let was=instance.state.capsHeld
            instance.state.remove(device:instance.identity(device))
            if was != instance.state.capsHeld && instance.hyperEnabled {instance.emitModifiers()}
        },Unmanaged.passUnretained(self).toOpaque())
        IOHIDManagerScheduleWithRunLoop(hid,CFRunLoopGetMain(),CFRunLoopMode.commonModes.rawValue)
        manager=hid
        guard IOHIDManagerOpen(hid,IOOptionBits(kIOHIDOptionsTypeNone))==kIOReturnSuccess else {stop();status="Keyboard input unavailable; check Input Monitoring";return}
        CGEvent.tapEnable(tap:port,enable:true);status="On"
    }
    private func identity(_ device:IOHIDDevice)->UInt64 {UInt64(UInt(bitPattern:Unmanaged.passUnretained(device).toOpaque()))}
    private func input(_ value:IOHIDValue) {
        let element=IOHIDValueGetElement(value)
        guard IOHIDElementGetUsagePage(element)==7 else {return}
        let device=IOHIDElementGetDevice(element)
        let usage=IOHIDElementGetUsage(element)
        guard usage>=4,usage<=0xe7 else {return}
        let was=state.capsHeld
        let action=state.input(device:identity(device),usage:usage,down:IOHIDValueGetIntegerValue(value) != 0,time:ProcessInfo.processInfo.systemUptime)
        if hyperEnabled && was != state.capsHeld {
            guard clearCapsLock() else {stop();status="Caps Lock control interrupted; retrying";return}
            emitModifiers()
        }
        if let action,let route=bindings[action],!IsSecureEventInputEnabled() {
            DispatchQueue.main.async {[weak self] in
                guard let self,self.status=="On",self.bindings[action]==route else {return}
                self.onAction?(route)
            }
        }
    }
    private var physicalModifiers:CGEventFlags {
        var flags=CGEventFlags()
        for usage in state.usages {
            let deviceMasks:[UInt32:UInt64]=[0xe0:0x1,0xe1:0x2,0xe2:0x20,0xe3:0x8,0xe4:0x2000,0xe5:0x4,0xe6:0x40,0xe7:0x10]
            if let mask=deviceMasks[usage] {flags.formUnion(CGEventFlags(rawValue:mask))}
            switch usage {
            case 0xe0,0xe4:flags.insert(.maskControl)
            case 0xe1,0xe5:flags.insert(.maskShift)
            case 0xe2,0xe6:flags.insert(.maskAlternate)
            case 0xe3,0xe7:flags.insert(.maskCommand)
            default:break
            }
        }
        return flags
    }
    private func clearCapsLock()->Bool {
        capsConnection != 0 && IOHIDSetModifierLockState(capsConnection,Int32(kIOHIDCapsLockState),false)==KERN_SUCCESS
    }
    static func generated(_ tag:Int64)->Bool {tag==generatedTag || tag==SnippetExpander.generatedTag}
    static func rewrittenFlags(_ original:CGEventFlags,held:Bool,tag:Int64=0)->CGEventFlags {
        if generated(tag) {return original}
        var flags=original
        flags.remove(.maskAlphaShift)
        if held {flags.formUnion(hyper)}
        return flags
    }
    private func emitModifiers(flags:CGEventFlags?=nil) {
        guard let event=CGEvent(keyboardEventSource:CGEventSource(stateID:.privateState),virtualKey:57,keyDown:state.capsHeld) else {return}
        event.type = .flagsChanged
        event.flags=flags ?? physicalModifiers.union(state.capsHeld ? Self.hyper:[])
        event.setIntegerValueField(.eventSourceUserData,value:Self.generatedTag)
        event.post(tap:.cgSessionEventTap)
    }
    private func filter(_ type:CGEventType,_ event:CGEvent)->Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {stop();status="Keyboard filter interrupted; retrying";return Unmanaged.passUnretained(event)}
        if Self.generated(event.getIntegerValueField(.eventSourceUserData)) {return Unmanaged.passUnretained(event)}
        if [.leftMouseDown,.rightMouseDown,.otherMouseDown,.scrollWheel].contains(type) {state.cancelTap()}
        // A Quartz keystroke also cancels a tap if it arrived before its raw HID report.
        if type == .keyDown {state.cancelTap()}
        if IsSecureEventInputEnabled() {stop();status="Paused during secure input";return Unmanaged.passUnretained(event)}
        if hyperEnabled {
            if type == .flagsChanged && event.getIntegerValueField(.keyboardEventKeycode)==57 {
                guard clearCapsLock() else {stop();status="Caps Lock control interrupted; retrying";return Unmanaged.passUnretained(event)}
                return nil
            }
            // Do not reconstruct ordinary event flags: that would strip rcmd's
            // right-Command remapping. Only add Hyper while physically held.
            event.flags=Self.rewrittenFlags(event.flags,held:state.capsHeld)
        }
        let result=commandFilter.handle(type,key:UInt16(event.getIntegerValueField(.keyboardEventKeycode)),flags:event.flags,repeatKey:event.getIntegerValueField(.keyboardEventAutorepeat) != 0)
        if let action=result.action {
            DispatchQueue.main.async {[weak self] in
                guard let self,self.status=="On",!IsSecureEventInputEnabled(),self.commandBindings.values.contains(action) else {return}
                self.onAction?(action)
            }
        }
        return result.consumed ? nil:Unmanaged.passUnretained(event)
    }
    func stop() {
        let was=state.capsHeld,physical=physicalModifiers
        state.reset();commandFilter.reset()
        if was && hyperEnabled {emitModifiers(flags:physical)}
        if let manager {IOHIDManagerClose(manager,0);IOHIDManagerUnscheduleFromRunLoop(manager,CFRunLoopGetMain(),CFRunLoopMode.commonModes.rawValue)}
        manager=nil
        if let tap {CGEvent.tapEnable(tap:tap,enable:false);CFMachPortInvalidate(tap)}
        if let source {CFRunLoopRemoveSource(CFRunLoopGetMain(),source,.commonModes)}
        tap=nil;source=nil
        if capsConnection != 0 {
            _=IOHIDSetModifierLockState(capsConnection,Int32(kIOHIDCapsLockState),originalCaps)
            IOServiceClose(capsConnection);capsConnection=0
        }
    }
}

/// Session event flags contain Apple's device-specific Command bits. Match them
/// here before Carbon dispatch; never rewrite flags on unmatched keystrokes.
struct CommandShortcutFilter {
    static let leftMask=UInt64(NX_DEVICELCMDKEYMASK),rightMask=UInt64(NX_DEVICERCMDKEYMASK)
    struct Binding {
        let key:UInt16
        let flags:CGEventFlags
        let side:UInt64
        let action:String
        init(shortcut:Shortcut,action:String) {
            key=UInt16(Shortcut.keyCodes[shortcut.key]!)
            side=shortcut.commandSide=="right-cmd" ? CommandShortcutFilter.rightMask:CommandShortcutFilter.leftMask
            var value:CGEventFlags=[.maskCommand]
            for (name,flag) in [("ctrl",CGEventFlags.maskControl),("alt",.maskAlternate),("shift",.maskShift)] where shortcut.modifiers.contains(name) {value.insert(flag)}
            flags=value;self.action=action
        }
    }
    var bindings:[Binding]=[]
    private var swallowed=Set<UInt16>()
    mutating func reset() {swallowed.removeAll()}
    mutating func handle(_ type:CGEventType,key:UInt16,flags:CGEventFlags,repeatKey:Bool=false)->(consumed:Bool,action:String?) {
        if type == .keyUp {return (swallowed.remove(key) != nil,nil)}
        guard type == .keyDown else {return (false,nil)}
        if swallowed.contains(key) {return (true,nil)}
        let side=flags.rawValue & (Self.leftMask | Self.rightMask)
        let modifiers=flags.intersection([.maskCommand,.maskControl,.maskAlternate,.maskShift,.maskSecondaryFn])
        guard !repeatKey,let binding=bindings.first(where:{$0.key==key && $0.side==side && $0.flags==modifiers}) else {return (false,nil)}
        swallowed.insert(key)
        return (true,binding.action)
    }
}
