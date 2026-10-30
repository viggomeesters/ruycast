import AppKit
import LauncherCore
import LauncherModels

// Real panel key dispatch with a private pasteboard; never sends desktop keys.
@MainActor func checkNativeEditCommands() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func key(_ character:String,_ flags:NSEvent.ModifierFlags = .command)->NSEvent {
        NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:0,context:nil,characters:flags.contains(.shift) ? character.uppercased():character,charactersIgnoringModifiers:flags.contains(.shift) ? character.uppercased():character,isARepeat:false,keyCode:["v":9,"a":0,"z":6,"c":8,"x":7,"k":40,",":43][character] ?? 0)!
    }
    let board=NSPasteboard(name:NSPasteboard.Name("ruycast-edit-check-"+UUID().uuidString))
    defer {board.clearContents()}
    let panel=PalettePanel(contentRect:NSRect(x:0,y:0,width:400,height:240),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
    let editor=EditCheckTextView(frame:NSRect(x:0,y:0,width:400,height:240));editor.board=board;editor.allowsUndo=true;panel.contentView=editor
    try require(panel.makeFirstResponder(editor),"Text responder did not acquire offscreen focus")
    board.setString("café 🦀\n\nTwo  spaces",forType:.string)
    try require(panel.performKeyEquivalent(with:key("v")),"Cmd+V did not route to the native text responder")
    try require(editor.string=="café 🦀\n\nTwo  spaces" && editor.pastes==1,"Paste did not preserve exact text")
    try require(panel.performKeyEquivalent(with:key("a")),"Cmd+A not handled")
    try require(editor.selectedRange().length==editor.string.utf16.count,"Select All lost text range")
    try require(panel.performKeyEquivalent(with:key("c")),"Cmd+C not handled")
    try require(board.string(forType:.string)==editor.string,"Copy lost selected text")
    try require(panel.performKeyEquivalent(with:key("x")),"Cmd+X not handled")
    try require(editor.string.isEmpty,"Cut did not remove selection")
    try require(panel.performKeyEquivalent(with:key("v",[.command,.capsLock])),"Caps Lock blocked Cmd+V")
    try require(!panel.performKeyEquivalent(with:key("v",[.command,.option])),"Modified paste was swallowed")
    let capture=CaptureTextEditor.makeInput()
    try require(capture.allowsUndo && capture.textStorage != nil && capture.textContainer != nil,"Capture input lost native text storage or undo")
    panel.contentView=capture;try require(panel.makeFirstResponder(capture),"Capture focus missing")
    capture.string="Before";capture.setSelectedRange(NSRange(location:0,length:6))
    guard let undo=capture.undoManager else {throw LauncherError("Capture undo manager missing")}
    undo.removeAllActions();capture.breakUndoCoalescing();undo.groupsByEvent=false;undo.beginUndoGrouping();capture.insertText("After",replacementRange:capture.selectedRange());undo.endUndoGrouping()
    try require(undo.canUndo,"Native typing did not register undo")
    try require(panel.performKeyEquivalent(with:key("z")) && capture.string=="Before","Cmd+Z did not undo native typing")
    try require(panel.performKeyEquivalent(with:key("z",[.command,.shift])) && capture.string=="After","Shift+Cmd+Z did not redo native typing")
    let field=NSTextField(string:"Search field");panel.contentView=field
    try require(panel.makeFirstResponder(field),"Search field focus missing")
    guard let fieldEditor=panel.firstResponder as? NSTextView else {throw LauncherError("Native field editor missing")}
    try require(panel.performKeyEquivalent(with:key("a")) && fieldEditor.selectedRange().length==12,"Search field editor did not select all")
    let recorder=ShortcutRecordButton(frame:.zero);panel.contentView=recorder
    var recorded:String?;recorder.save={recorded=$0}
    for character in ["v","a","z"] {
        recorder.beginRecording()
        try require(panel.performKeyEquivalent(with:key(character)) && recorded=="cmd+"+character,"Recorder lost edit key to text editing")
    }
    let originalMenu=NSApp.mainMenu;defer {NSApp.mainMenu=originalMenu}
    NativeEditCommands.installMenu();NativeEditCommands.installMenu()
    let menus=NSApp.mainMenu!.items.compactMap(\.submenu).filter {$0.identifier==NSUserInterfaceItemIdentifier("native-edit")}
    try require(menus.count==1,"Edit menu missing or duplicated")
    for (keyName,selector) in [("v","paste:"),("x","cut:"),("c","copy:"),("a","selectAll:"),("z","undo:")] {
        try require(menus[0].items.contains {$0.keyEquivalent==keyName && $0.action.map(NSStringFromSelector)==selector},"Standard edit menu command missing")
    }
    try require(menus[0].items.contains {$0.keyEquivalent=="Z" && $0.keyEquivalentModifierMask==[.command,.shift] && $0.action.map(NSStringFromSelector)=="redo:"},"Redo menu command missing")
    let formWindow=NSWindow(contentRect:NSRect(x:0,y:0,width:400,height:240),styleMask:[.titled],backing:.buffered,defer:false)
    let formEditor=EditCheckTextView(frame:NSRect(x:0,y:0,width:400,height:240));formEditor.board=board;formEditor.allowsUndo=true
    formWindow.contentView=formEditor;_=formWindow.makeFirstResponder(formEditor)
    let target=NativeEditCommands(window:{formWindow});let editMenu=NativeEditCommands.menu(target:target)
    board.clearContents();board.setString("Ordinary form 🦀",forType:.string)
    try require(editMenu.performKeyEquivalent(with:key("v")) && formEditor.string=="Ordinary form 🦀","Ordinary window menu paste did not dispatch")
    try require(editMenu.performKeyEquivalent(with:key("a")) && formEditor.selectedRange().length==formEditor.string.utf16.count,"Ordinary window menu Select All failed")
    try require(editMenu.performKeyEquivalent(with:key("c")) && board.string(forType:.string)==formEditor.string,"Ordinary window menu Copy failed")
    try require(editMenu.performKeyEquivalent(with:key("x")) && formEditor.string.isEmpty,"Ordinary window menu Cut failed")
    let formCapture=CaptureTextEditor.makeInput();formWindow.contentView=formCapture;_=formWindow.makeFirstResponder(formCapture)
    formCapture.string="Before";formCapture.setSelectedRange(NSRange(location:0,length:6))
    guard let formUndo=formCapture.undoManager else {throw LauncherError("Form undo manager missing")}
    formUndo.removeAllActions();formCapture.breakUndoCoalescing();formUndo.groupsByEvent=false;formUndo.beginUndoGrouping();formCapture.insertText("After",replacementRange:formCapture.selectedRange());formUndo.endUndoGrouping()
    try require(editMenu.performKeyEquivalent(with:key("z")) && formCapture.string=="Before","Ordinary window menu Undo failed")
    // The hidden check has no application event loop to refresh menu validation.
    editMenu.update()
    let redoHandled=editMenu.performKeyEquivalent(with:key("z",[.command,.shift]))
    try require(redoHandled && formCapture.string=="After","Ordinary window menu Redo failed: handled=\(redoHandled), text=\(formCapture.string), canRedo=\(formUndo.canRedo)")
    let formField=NSTextField(string:"Form field");formWindow.contentView=formField;_=formWindow.makeFirstResponder(formField)
    guard let formFieldEditor=formWindow.firstResponder as? NSTextView else {throw LauncherError("Form native field editor missing")}
    try require(editMenu.performKeyEquivalent(with:key("a")) && formFieldEditor.selectedRange().length==10,"Ordinary form field Select All failed")
    let button=NSButton(title:"Button",target:nil,action:nil);formWindow.contentView=button;_=formWindow.makeFirstResponder(button)
    let pasteItem=editMenu.items.first {$0.action.map(NSStringFromSelector)=="paste:"}!
    try require(!target.validateMenuItem(pasteItem),"Editing stayed enabled without a text responder")
    try require(!formWindow.isVisible,"Form check displayed a window")
    var actions=0;panel.actions={actions+=1};panel.contentView=editor;_=panel.makeFirstResponder(editor)
    try require(panel.performKeyEquivalent(with:key("k")) && actions==1,"Cmd+K regression")
    var bindings=0;panel.itemBindingKey={_ in bindings+=1;return true}
    try require(panel.performKeyEquivalent(with:key(",",[.command,.option])) && bindings==1,"Item binding regression")
    try require(!panel.isVisible,"Native edit check displayed a window")
    return ["ok":true,"nativePanelEditDispatch":true,"exactPrivatePasteboardText":true,"captureUndoRedo":true,"fieldEditorSelectAll":true,"recorderPriority":true,"standardEditMenu":true,"ordinaryWindowEditDispatch":true,"menuLabels":menus[0].items.filter {!$0.isSeparatorItem}.map(\.title),"existingActions":true,"desktopInputSent":false,"visibleWindowOpened":false]
}
private final class EditCheckTextView:NSTextView {
    var board:NSPasteboard!;var pastes=0
    override func paste(_ sender:Any?) {pastes+=1;insertText(board.string(forType:.string) ?? "",replacementRange:selectedRange())}
    override func copy(_ sender:Any?) {board.clearContents();board.setString((string as NSString).substring(with:selectedRange()),forType:.string)}
    override func cut(_ sender:Any?) {copy(sender);insertText("",replacementRange:selectedRange())}
}
