import AppKit
import SwiftUI

/// Use the field editor's native delegate rather than SwiftUI key handlers,
/// which an editing TextField consumes before its parent sees them.
struct PaletteSearchField:NSViewRepresentable {
    @Binding var text:String
    var placeholder:String
    var onMove:(Int)->Void
    var onSubmit:()->Void
    var onCancel:()->Void
    var onActions:()->Void
    func makeCoordinator()->Coordinator {Coordinator(self)}
    func makeNSView(context:Context)->SearchInput {
        let field=SearchInput();field.isBordered=false;field.drawsBackground=false
        field.focusRingType = .none;field.font = .systemFont(ofSize:18)
        field.textColor = .labelColor;field.delegate=context.coordinator
        field.setAccessibilityLabel(placeholder);field.actions=onActions
        field.setContentCompressionResistancePriority(.defaultLow,for:.horizontal)
        return field
    }
    func updateNSView(_ field:SearchInput,context:Context) {
        context.coordinator.parent=self;field.placeholderString=placeholder;field.actions=onActions
        if field.stringValue != text {field.stringValue=text}
    }
    final class Coordinator:NSObject,NSTextFieldDelegate {
        var parent:PaletteSearchField
        init(_ parent:PaletteSearchField) {self.parent=parent}
        func controlTextDidChange(_ notification:Notification) {
            guard let field=notification.object as? NSTextField else {return}
            parent.text=field.stringValue
        }
        func control(_ control:NSControl,textView:NSTextView,doCommandBy selector:Selector)->Bool {
            switch NSStringFromSelector(selector) {
            case "moveDown:":parent.onMove(1)
            case "moveUp:":parent.onMove(-1)
            case "insertNewline:":parent.onSubmit()
            case "cancelOperation:":parent.onCancel()
            default:return false
            }
            return true
        }
    }
}
final class SearchInput:NSTextField {
    var actions:(()->Void)?
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window else {return}
        DispatchQueue.main.async { [weak self,weak window] in guard let self,let window,window.isVisible else {return};window.makeFirstResponder(self)}
    }
    override func performKeyEquivalent(with event:NSEvent)->Bool {
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,event.charactersIgnoringModifiers?.lowercased()=="k" {actions?();return true}
        return super.performKeyEquivalent(with:event)
    }
}
