import AppKit
import Carbon
import LauncherCore
import LauncherModels

/// No key history leaves the transient matcher. Never observes secure entry.
@MainActor final class SnippetExpander {
    nonisolated static let generatedTag:Int64=0x5255535443415354
    weak var state:AppState?
    private var tap:CFMachPort?
    private var source:CFRunLoopSource?
    private var matcher=SnippetMatcher()
    private var lastPID:pid_t?
    private var poll:Timer?
    private var choice:SnippetChoiceController?
    init(state:AppState) {
        self.state=state
        poll=mainRunLoopTimer(every:2) { [weak self] in self?.refresh() }
        refresh()
    }
    isolated deinit {poll?.invalidate();stop()}
    func stop() {
        choice?.cancel(restore:false);choice=nil
        if let tap {CGEvent.tapEnable(tap:tap,enable:false);CFMachPortInvalidate(tap)}
        if let source {CFRunLoopRemoveSource(CFRunLoopGetMain(),source,.commonModes)}
        tap=nil;source=nil;matcher.reset()
    }
    func refresh() {
        guard let state else {stop();return}
        let reason:String
        if state.config.snippetExpansionEnabled != true {reason="Off"}
        else if NSRunningApplication.runningApplications(withBundleIdentifier:"com.raycast.macos").contains(where:{!$0.isTerminated}) {reason="Quit Raycast to expand keywords"}
        else if !state.windows.trusted {reason="Allow Accessibility to expand keywords"}
        else {reason=""}
        if !reason.isEmpty {stop();if state.expansionStatus != reason {state.expansionStatus=reason};return}
        if let tap,CGEvent.tapIsEnabled(tap:tap) {if state.expansionStatus != "On" {state.expansionStatus="On"};return}
        stop()
        let mask:CGEventMask=(1<<CGEventType.keyDown.rawValue)|(1<<CGEventType.leftMouseDown.rawValue)|(1<<CGEventType.rightMouseDown.rawValue)
        let callback:CGEventTapCallBack={_,type,event,context in
            guard let context else {return Unmanaged.passUnretained(event)}
            return Unmanaged<SnippetExpander>.fromOpaque(context).takeUnretainedValue().handle(type,event)
        }
        guard let port=CGEvent.tapCreate(tap:.cgSessionEventTap,place:.headInsertEventTap,options:.defaultTap,eventsOfInterest:mask,callback:callback,userInfo:Unmanaged.passUnretained(self).toOpaque()) else {state.expansionStatus="Allow Input Monitoring to expand keywords";return}
        tap=port;source=CFMachPortCreateRunLoopSource(kCFAllocatorDefault,port,0)
        CFRunLoopAddSource(CFRunLoopGetMain(),source,.commonModes);CGEvent.tapEnable(tap:port,enable:true)
        state.expansionStatus="On"
    }
    private func handle(_ type:CGEventType,_ event:CGEvent)->Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {choice?.cancel(restore:false);matcher.reset();if let tap {CGEvent.tapEnable(tap:tap,enable:true)};return Unmanaged.passUnretained(event)}
        if event.getIntegerValueField(.eventSourceUserData)==Self.generatedTag {return Unmanaged.passUnretained(event)}
        if let choice {
            choice.input(type,event)
            matcher.reset();return Unmanaged.passUnretained(event)
        }
        guard type == .keyDown,let state,!state.paletteVisible,state.windows.trusted,!IsSecureEventInputEnabled(),!event.flags.contains(.maskCommand),!event.flags.contains(.maskControl),!event.flags.contains(.maskAlternate),event.getIntegerValueField(.keyboardEventAutorepeat)==0,
            let active=NSWorkspace.shared.frontmostApplication,active.processIdentifier != getpid(),!state.config.clipboard.excludedApps.contains(active.bundleIdentifier ?? ""),!secureField() else {matcher.reset();return Unmanaged.passUnretained(event)}
        if lastPID != active.processIdentifier {matcher.reset();lastPID=active.processIdentifier}
        let code=event.getIntegerValueField(.keyboardEventKeycode)
        if code==51 {matcher.backspace();return Unmanaged.passUnretained(event)}
        if [123,124,125,126,115,116,117,119,121,53].contains(code) {matcher.reset();return Unmanaged.passUnretained(event)}
        var units=[UniChar](repeating:0,count:16),length=0
        event.keyboardGetUnicodeString(maxStringLength:16,actualStringLength:&length,unicodeString:&units)
        guard length>0 else {matcher.reset();return Unmanaged.passUnretained(event)}
        let text=String(utf16CodeUnits:units,count:length)
        guard !state.recordingShortcut else {matcher.reset();return Unmanaged.passUnretained(event)}
        guard let command=matcher.type(text,commands:state.config.commands),let keyword=command.keyword else {return Unmanaged.passUnretained(event)}
        let pid=active.processIdentifier
        if Templates.needsInteraction(command) {
            let controller=SnippetChoiceController(command:command,pid:pid,safe:{ [weak self,weak state] in
                guard let self,let state else {return false}
                return !state.paletteVisible && state.config.snippetExpansionEnabled==true && state.windows.trusted && !IsSecureEventInputEnabled() && !self.secureField() && !state.config.clipboard.excludedApps.contains(active.bundleIdentifier ?? "") && state.config.commands.contains(command)
            },apply:{ [weak state] replacement in
                let started=DispatchTime.now().uptimeNanoseconds,tracked=state?.analytics.recordAttempt(command.id,source:"snippet")
                let applied=SnippetClipboard.apply(replacement,mode:command.snippetOutput,board:.general,onWrite:{state?.ignoreOwnClipboardChange()},key:Self.key,paste:Self.paste,moveCursor:{ [weak self] in self?.moveCursor($0,pid:pid) },insert:Self.insert)
                if !applied {state?.showToast?("Could not copy snippet. Type the keyword again.",false)}
                if let tracked {state?.analytics.recordCompletion(tracked,success:applied,ms:AnalyticsStore.elapsed(started))}
            },failure:{ [weak state] in state?.showToast?($0,false) })
            choice=controller;controller.onFinish={ [weak self] in self?.choice=nil;self?.matcher.reset() }
            DispatchQueue.main.asyncAfter(deadline:.now()+0.03) { [weak controller] in controller?.present() }
            return Unmanaged.passUnretained(event)
        }
        // Render before swallowing the last key; unavailable templates leave typing unchanged.
        guard let rendered=try? Templates.render(command,values:[:],clipboard:command.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil) else {return Unmanaged.passUnretained(event)}
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier==pid,!IsSecureEventInputEnabled(),!secureField() else {return Unmanaged.passUnretained(event)}
        let replacement=SnippetReplacement(keyword:keyword,finalInput:text,text:rendered.0,cursor:rendered.1)
        let started=DispatchTime.now().uptimeNanoseconds,tracked=state.analytics.recordAttempt(command.id,source:"snippet")
        let applied=SnippetClipboard.apply(replacement,mode:command.snippetOutput,board:.general,onWrite:{state.ignoreOwnClipboardChange()},key:Self.key,paste:Self.paste,moveCursor:{ [weak self] in self?.moveCursor($0,pid:pid) },insert:Self.insert)
        if !applied {state.showToast?("Could not copy snippet.",false)}
        if let tracked {state.analytics.recordCompletion(tracked,success:applied,ms:AnalyticsStore.elapsed(started))}
        return applied ? nil:Unmanaged.passUnretained(event)
    }
    private func moveCursor(_ steps:Int,pid:pid_t) {
        DispatchQueue.main.asyncAfter(deadline:.now()+0.08) { [weak self] in
            guard let self,NSWorkspace.shared.frontmostApplication?.processIdentifier==pid,!IsSecureEventInputEnabled(),!self.secureField() else {return}
            for _ in 0..<steps {Self.key(123)}
        }
    }
    private static func paste() {for down in [true,false] {let event=CGEvent(keyboardEventSource:nil,virtualKey:9,keyDown:down);event?.flags = .maskCommand;event?.setIntegerValueField(.eventSourceUserData,value:generatedTag);event?.post(tap:.cghidEventTap)}}
    private static func insert(_ value:String) {
        let units=Array(value.utf16)
        for down in [true,false] {
            if let event=CGEvent(keyboardEventSource:nil,virtualKey:0,keyDown:down) {
                event.flags=[];event.keyboardSetUnicodeString(stringLength:units.count,unicodeString:units)
                event.setIntegerValueField(.eventSourceUserData,value:generatedTag);event.post(tap:.cghidEventTap)
            }
        }
    }
    private static func key(_ code:CGKeyCode) {for down in [true,false] {let event=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down);event?.flags=[];event?.setIntegerValueField(.eventSourceUserData,value:generatedTag);event?.post(tap:.cghidEventTap)}}
    private func secureField()->Bool {
        var element:CFTypeRef?
        let system=AXUIElementCreateSystemWide()
        // A slow/unavailable AX implementation must not stall the event tap.
        AXUIElementSetMessagingTimeout(system,0.01)
        let result=AXUIElementCopyAttributeValue(system,kAXFocusedUIElementAttribute as CFString,&element)
        guard AXIsProcessTrusted() else {return true}
        var subrole:CFTypeRef?
        if result == .success,let element,CFGetTypeID(element)==AXUIElementGetTypeID() {
            let focused=unsafeDowncast(element,to:AXUIElement.self)
            AXUIElementSetMessagingTimeout(focused,0.01)
            AXUIElementCopyAttributeValue(focused,kAXSubroleAttribute as CFString,&subrole)
        }
        return !SnippetFocusPolicy.allowsExpansion(secureInput:IsSecureEventInputEnabled(),subrole:subrole as? String)
    }
}
