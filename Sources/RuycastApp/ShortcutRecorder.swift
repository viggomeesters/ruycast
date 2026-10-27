import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

extension AppState {
    func beginShortcutRecording() {recordingShortcut=true;hotkeys?.suspend()}
    func endShortcutRecording() {
        guard recordingShortcut else {return};recordingShortcut=false
        do {try hotkeys?.resume()} catch {configError=error.localizedDescription}
    }
}

/// Receives only events directed to this focused control. No global listener.
final class ShortcutRecordButton:NSButton {
    var start:(()->Void)?
    var stop:(()->Void)?
    var save:((String?)->Void)?
    var value:String?
    var separateCommandKeys=true
    private(set) var recording=false
    private var solo:UInt16?
    override var acceptsFirstResponder:Bool {true}
    func configure(_ value:String?) {
        self.value=value
        if !recording {title=value.map {ShortcutPresentation($0).keys.joined(separator:" ")} ?? "Record Shortcut"}
        toolTip=recording ? "Press shortcut · Escape to cancel · Delete to clear":value.map {ShortcutPresentation($0).label} ?? "Record Shortcut"
        setAccessibilityLabel(recording ? "Press shortcut":value.map {ShortcutPresentation($0).label} ?? "Record Shortcut")
    }
    @objc func beginRecording() {
        if recording {end();return}
        guard window?.makeFirstResponder(self)==true else {return}
        recording=true;solo=nil;start?();title="Press shortcut…";toolTip="Escape to cancel · Delete to clear"
    }
    func end() {guard recording else {return};recording=false;solo=nil;stop?();configure(value)}
    override func resignFirstResponder()->Bool {end();return super.resignFirstResponder()}
    override func viewWillMove(toWindow newWindow:NSWindow?) {if newWindow==nil {end()};super.viewWillMove(toWindow:newWindow)}
    override func performKeyEquivalent(with event:NSEvent)->Bool {
        if recording,window?.firstResponder===self {keyDown(with:event);return true}
        return super.performKeyEquivalent(with:event)
    }
    static func shortcut(for event:NSEvent,separateCommandKeys:Bool=true)->String? {
        let mods:[(NSEvent.ModifierFlags,String)]=[(.control,"ctrl"),(.option,"alt"),(.shift,"shift"),(.command,"cmd")]
        var names=mods.filter {event.modifierFlags.contains($0.0)}.map(\.1)
        let side=event.modifierFlags.rawValue & UInt(CommandShortcutFilter.leftMask | CommandShortcutFilter.rightMask)
        // Keep the existing compact Hyper binding when all four modifiers are held.
        if separateCommandKeys,names.count != 4,let index=names.firstIndex(of:"cmd") {
            if side==CommandShortcutFilter.rightMask {names[index]="right-cmd"}
            else if side==CommandShortcutFilter.leftMask {names[index]="left-cmd"}
        }
        guard !names.isEmpty,let key=Shortcut.keyCodes.filter({!["left-control","left-option","backspace"].contains($0.key)}).first(where:{$0.value==UInt32(event.keyCode)})?.key else {return nil}
        return names.joined(separator:"+")+"+"+key
    }
    override func keyDown(with event:NSEvent) {
        guard recording else {super.keyDown(with:event);return}
        solo=nil
        if event.keyCode==53 {end();return}
        if event.keyCode==51,event.modifierFlags.intersection([.control,.option,.shift,.command]).isEmpty {end();save?(nil);return}
        guard let value=Self.shortcut(for:event,separateCommandKeys:separateCommandKeys) else {NSSound.beep();return}
        end();save?(value)
    }
    override func menu(for event:NSEvent)->NSMenu? {
        guard !recording,let value,(try? Shortcut(value).commandSide) != nil else {return nil}
        let menu=NSMenu()
        let item=NSMenuItem(title:"Use Either Command",action:#selector(useEitherCommand),keyEquivalent:"")
        item.target=self;menu.addItem(item);return menu
    }
    @objc func useEitherCommand() {
        guard !recording,let value,let shortcut=try? Shortcut(value),let side=shortcut.commandSide else {return}
        save?(value.replacingOccurrences(of:side,with:"cmd"))
    }
    override func flagsChanged(with event:NSEvent) {
        guard recording else {super.flagsChanged(with:event);return}
        let flags=event.modifierFlags.intersection([.control,.option,.shift,.command])
        if (event.keyCode==59 && flags == .control) || (event.keyCode==58 && flags == .option) {solo=event.keyCode}
        else if flags.isEmpty,let code=solo {end();save?(code==59 ? "ctrl+left-control":"alt+left-option")}
        else {solo=nil}
    }
}
struct ShortcutRecorder:NSViewRepresentable {
    let state:AppState
    var value:String?
    var onSave:(String?)->Void
    func makeNSView(context:Context)->ShortcutRecordButton {
        let button=ShortcutRecordButton(title:"",target:nil,action:nil)
        button.target=button;button.action=#selector(ShortcutRecordButton.beginRecording)
        button.bezelStyle = .rounded;button.font = .systemFont(ofSize:12,weight:.medium)
        button.setContentCompressionResistancePriority(.defaultLow,for:.horizontal)
        return button
    }
    func updateNSView(_ button:ShortcutRecordButton,context:Context) {
        button.separateCommandKeys=state.config.separatesCommandKeys;button.configure(value);button.start={state.beginShortcutRecording()};button.stop={state.endShortcutRecording()};button.save=onSave
        button.isEnabled = !state.busy
    }
    static func dismantleNSView(_ button:ShortcutRecordButton,coordinator:()) {button.end()}
}
