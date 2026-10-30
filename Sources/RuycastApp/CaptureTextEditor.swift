import AppKit
import SwiftUI

/// Native multiline capture with focus acquired only when the user opens the form.
struct CaptureTextEditor:NSViewRepresentable {
    @Binding var text:String
    var placeholder:String
    var onSubmit:()->Void
    func makeCoordinator()->Coordinator {Coordinator(self)}
    func makeNSView(context:Context)->NSScrollView {
        let scroll=NSScrollView(frame:NSRect(x:0,y:0,width:600,height:240));scroll.hasVerticalScroller=true;scroll.drawsBackground=false
        let editor=Self.makeInput()
        editor.delegate=context.coordinator;editor.setAccessibilityLabel(placeholder)
        editor.onSubmit={context.coordinator.parent.onSubmit()}
        scroll.documentView=editor
        return scroll
    }
    static func makeInput()->CaptureInput {
        let editor=CaptureInput(frame:NSRect(x:0,y:0,width:600,height:240));editor.allowsUndo=true;editor.isRichText=false;editor.isAutomaticQuoteSubstitutionEnabled=false;editor.isAutomaticDashSubstitutionEnabled=false
        editor.font = .systemFont(ofSize:14);editor.textColor = .labelColor;editor.drawsBackground=false
        editor.textContainerInset=NSSize(width:10,height:10);editor.isVerticallyResizable=true;editor.isHorizontallyResizable=false
        editor.autoresizingMask=[.width];editor.textContainer?.widthTracksTextView=true
        return editor
    }
    func updateNSView(_ scroll:NSScrollView,context:Context) {
        context.coordinator.parent=self
        if let editor=scroll.documentView as? CaptureInput,editor.string != text {editor.string=text}
    }
    final class Coordinator:NSObject,NSTextViewDelegate {
        var parent:CaptureTextEditor
        init(_ parent:CaptureTextEditor) {self.parent=parent}
        func textDidChange(_ notification:Notification) {if let editor=notification.object as? NSTextView {parent.text=editor.string}}
    }
}
final class CaptureInput:NSTextView {
    var onSubmit:(()->Void)?
    override func keyDown(with event:NSEvent) {
        if event.keyCode==36 || event.keyCode==76 {
            if hasMarkedText() {super.keyDown(with:event);return}
            let modifiers=event.modifierFlags.intersection([.command,.control,.option,.shift])
            if modifiers == .shift {super.insertNewline(nil);return}
            if modifiers.isEmpty || modifiers == .command {onSubmit?();return}
        }
        super.keyDown(with:event)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow();guard let window else {return}
        DispatchQueue.main.async { [weak self,weak window] in guard let self,let window,self.window===window else {return};window.makeFirstResponder(self)}
    }
}
