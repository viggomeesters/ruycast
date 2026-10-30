import AppKit

/// Standard editing stays in AppKit and the local text responder's undo manager.
@MainActor final class NativeEditCommands:NSObject,NSMenuItemValidation {
    static let shared=NativeEditCommands()
    private let window:()->NSWindow?
    init(window:@escaping ()->NSWindow? = {NSApp.keyWindow}) {self.window=window;super.init()}
    static func perform(_ event:NSEvent,responder:NSResponder?)->Bool {
        guard event.type == .keyDown,let editor=responder as? NSTextView else {return false}
        let flags=event.modifierFlags.intersection([.command,.shift,.control,.option])
        let key=event.charactersIgnoringModifiers?.lowercased()
        if key=="z",flags == .command || flags == [.command,.shift] {
            guard editor.isEditable else {return false}
            if flags.contains(.shift) {if editor.undoManager?.canRedo==true {editor.undoManager?.redo()}}
            else if editor.undoManager?.canUndo==true {editor.undoManager?.undo()}
            return true
        }
        guard flags == .command else {return false}
        let action:Selector
        switch key {
        case "v":guard editor.isEditable else {return false};action=#selector(NSText.paste(_:))
        case "x":guard editor.isEditable else {return false};action=#selector(NSText.cut(_:))
        case "c":action=#selector(NSText.copy(_:))
        case "a":action=#selector(NSText.selectAll(_:))
        default:return false
        }
        return editor.tryToPerform(action,with:nil)
    }
    static func menu(target:NativeEditCommands)->NSMenu {
        let edit=NSMenu(title:"Edit");edit.identifier=NSUserInterfaceItemIdentifier("native-edit")
        for (title,action,key,shift) in [
            ("Undo",#selector(undo(_:)),"z",false),("Redo",#selector(redo(_:)),"Z",true),
            ("Cut",#selector(cut(_:)),"x",false),("Copy",#selector(copy(_:)),"c",false),
            ("Paste",#selector(paste(_:)),"v",false),("Select All",#selector(selectAll(_:)),"a",false)
        ] {
            if title=="Cut" {edit.addItem(.separator())}
            let item=NSMenuItem(title:title,action:action,keyEquivalent:key)
            item.keyEquivalentModifierMask=shift ? [.command,.shift]:.command;item.target=target;edit.addItem(item)
        }
        return edit
    }
    static func installMenu() {
        let main=NSApp.mainMenu ?? NSMenu()
        if main.items.contains(where:{$0.submenu?.identifier==NSUserInterfaceItemIdentifier("native-edit")}) {return}
        if main.items.isEmpty {
            let app=NSMenu(title:"Ruycast");app.addItem(withTitle:"Quit Ruycast",action:#selector(NSApplication.terminate(_:)),keyEquivalent:"q")
            let item=NSMenuItem(title:"Ruycast",action:nil,keyEquivalent:"");item.submenu=app;main.addItem(item)
        }
        let item=NSMenuItem(title:"Edit",action:nil,keyEquivalent:"");item.submenu=menu(target:shared);main.addItem(item);NSApp.mainMenu=main
    }
    private var editor:NSTextView? {window()?.firstResponder as? NSTextView}
    @objc func undo(_ sender:Any?) {if editor?.isEditable==true,editor?.undoManager?.canUndo==true {editor?.undoManager?.undo()}}
    @objc func redo(_ sender:Any?) {if editor?.isEditable==true,editor?.undoManager?.canRedo==true {editor?.undoManager?.redo()}}
    @objc func paste(_ sender:Any?) {if editor?.isEditable==true {editor?.paste(sender)}}
    @objc func cut(_ sender:Any?) {if editor?.isEditable==true {editor?.cut(sender)}}
    @objc func copy(_ sender:Any?) {editor?.copy(sender)}
    @objc func selectAll(_ sender:Any?) {editor?.selectAll(sender)}
    func validateMenuItem(_ item:NSMenuItem)->Bool {
        guard let editor else {return false}
        switch item.action.map(NSStringFromSelector) {
        case "undo:":return editor.isEditable && editor.undoManager?.canUndo==true
        case "redo:":return editor.isEditable && editor.undoManager?.canRedo==true
        case "cut:":return editor.isEditable && editor.selectedRange().length>0
        case "copy:":return editor.selectedRange().length>0
        case "paste:":return editor.isEditable
        case "selectAll:":return editor.isSelectable
        default:return false
        }
    }
}
