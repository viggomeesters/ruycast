import AppKit
import SwiftUI
import LauncherCore

struct SnippetTextEditor:NSViewRepresentable {
    @Binding var text:String
    @Binding var selection:NSRange
    @Environment(\.colorScheme) private var scheme
    func makeCoordinator()->Coordinator {Coordinator(self)}
    func makeNSView(context:Context)->NSScrollView {
        let scroll=NSScrollView(frame:NSRect(x:0,y:0,width:560,height:230))
        scroll.hasVerticalScroller=true;scroll.drawsBackground=false;scroll.clipsToBounds=true
        let editor=NSTextView(frame:NSRect(x:0,y:0,width:510,height:230))
        editor.isRichText=false;editor.font = .monospacedSystemFont(ofSize:13,weight:.regular);editor.textColor = .labelColor;editor.drawsBackground=false
        editor.isAutomaticQuoteSubstitutionEnabled=false;editor.isAutomaticDashSubstitutionEnabled=false;editor.isAutomaticTextReplacementEnabled=false
        editor.allowsUndo=true;editor.isVerticallyResizable=true;editor.isHorizontallyResizable=false;editor.autoresizingMask=[.width]
        editor.textContainer?.widthTracksTextView=true;editor.textContainerInset=NSSize(width:8,height:10)
        editor.delegate=context.coordinator;editor.setAccessibilityLabel("Snippet text")
        scroll.documentView=editor;scroll.hasVerticalRuler=true;scroll.rulersVisible=true
        scroll.verticalRulerView=SnippetLineRuler(textView:editor,scrollView:scroll)
        return scroll
    }
    func updateNSView(_ scroll:NSScrollView,context:Context) {
        context.coordinator.parent=self
        scroll.appearance=NSAppearance(named:scheme == .dark ? .darkAqua:.aqua)
        guard let editor=scroll.documentView as? NSTextView else {return}
        context.coordinator.updating=true
        if editor.string != text {
            // replaceCharacters keeps the native undo stack when a link form edits source.
            let old=editor.string
            if editor.shouldChangeText(in:NSRange(location:0,length:old.utf16.count),replacementString:text) {
                editor.replaceCharacters(in:NSRange(location:0,length:old.utf16.count),with:text);editor.didChangeText()
            }
        }
        let safe=NSRange(location:min(selection.location,text.utf16.count),length:min(selection.length,max(0,text.utf16.count-selection.location)))
        if editor.selectedRange() != safe {editor.setSelectedRange(safe)}
        context.coordinator.updating=false;scroll.verticalRulerView?.needsDisplay=true
    }
    final class Coordinator:NSObject,NSTextViewDelegate {
        var parent:SnippetTextEditor;var updating=false
        init(_ parent:SnippetTextEditor) {self.parent=parent}
        func textDidChange(_ n:Notification) {guard !updating,let e=n.object as? NSTextView else {return};parent.text=e.string;parent.selection=e.selectedRange();e.enclosingScrollView?.verticalRulerView?.needsDisplay=true}
        func textViewDidChangeSelection(_ n:Notification) {guard !updating,let e=n.object as? NSTextView else {return};if parent.selection != e.selectedRange() {parent.selection=e.selectedRange()}}
    }
}
final class SnippetLineRuler:NSRulerView {
    weak var textView:NSTextView?
    init(textView:NSTextView,scrollView:NSScrollView) {self.textView=textView;super.init(scrollView:scrollView,orientation:.verticalRuler);clientView=textView;ruleThickness=44}
    required init(coder:NSCoder) {fatalError("init(coder:) is not supported")}
    static func lineStarts(_ source:String)->[Int] {
        let ns=source as NSString;var starts=[0]
        for i in 0..<ns.length where ns.character(at:i)==10 {starts.append(i+1)}
        return starts
    }
    func markers(in rect:NSRect)->[(number:Int,y:CGFloat)] {
        guard let editor=textView,let layout=editor.layoutManager,let container=editor.textContainer else {return []}
        layout.ensureLayout(for:container)
        return Self.lineStarts(editor.string).enumerated().compactMap {index,offset in
            let fragment:NSRect
            if offset==editor.string.utf16.count {fragment=layout.extraLineFragmentRect.isEmpty ? NSRect(x:0,y:0,width:0,height:16):layout.extraLineFragmentRect}
            else {fragment=layout.lineFragmentRect(forGlyphAt:layout.glyphIndexForCharacter(at:offset),effectiveRange:nil)}
            let y=editor.convert(NSPoint(x:0,y:fragment.minY+editor.textContainerOrigin.y),to:self).y
            guard y+fragment.height>=rect.minY,y<=rect.maxY else {return nil}
            return (index+1,y)
        }
    }
    override func drawHashMarksAndLabels(in rect:NSRect) {
        guard let editor=textView else {return}
        let width=max(44,CGFloat(String(Self.lineStarts(editor.string).count).count)*8+20)
        if ruleThickness != width {ruleThickness=width}
        NSGraphicsContext.saveGraphicsState();defer {NSGraphicsContext.restoreGraphicsState()};NSBezierPath(rect:bounds).addClip()
        NSColor.controlBackgroundColor.setFill();bounds.fill()
        NSColor.separatorColor.setFill();NSRect(x:bounds.maxX-1,y:bounds.minY,width:1,height:bounds.height).fill()
        let attributes:[NSAttributedString.Key:Any]=[.font:NSFont.monospacedDigitSystemFont(ofSize:11,weight:.regular),.foregroundColor:NSColor.secondaryLabelColor]
        for marker in markers(in:rect) {
            let label=String(marker.number) as NSString
            label.draw(at:NSPoint(x:ruleThickness-label.size(withAttributes:attributes).width-10,y:marker.y+2),withAttributes:attributes)
        }
    }

}

struct SnippetLinkDraft:Identifiable {
    let id=UUID();let source:String;let range:NSRange
    var label:String;var url:String
    static func make(source:String,selection:NSRange)->Self {
        if let link=SnippetOutput.links(source).first(where: {NSIntersectionRange($0.range,selection).length>0 || (selection.length==0 && selection.location >= $0.range.location && selection.location<NSMaxRange($0.range))}) {return Self(source:source,range:link.range,label:link.label,url:link.url)}
        let range=NSRange(location:min(selection.location,source.utf16.count),length:min(selection.length,max(0,source.utf16.count-selection.location)))
        return Self(source:source,range:range,label:(source as NSString).substring(with:range),url:"")
    }
    func applying(to current:String)->(String,NSRange)? {
        guard current==source,!label.isEmpty,!label.contains(where:{$0.isNewline}),SnippetOutput.validURL(url) else {return nil}
        let replacement=SnippetOutput.linkSource(label:label,url:url)
        return ((source as NSString).replacingCharacters(in:range,with:replacement),NSRange(location:range.location+replacement.utf16.count,length:0))
    }
}
struct SnippetLinkForm:View {
    @State var draft:SnippetLinkDraft
    let save:(SnippetLinkDraft)->Void
    let cancel:()->Void
    var body:some View {
        VStack(alignment:.leading,spacing:16) {
            Text("Link").font(.headline)
            VStack(alignment:.leading,spacing:5) {Text("Text");TextField("Text",text:$draft.label)}
            VStack(alignment:.leading,spacing:5) {Text("URL");TextField("https://",text:$draft.url)}
            HStack {Button("Cancel",action:cancel).keyboardShortcut(.cancelAction);Spacer();Button("Save") {save(draft)}.keyboardShortcut(.defaultAction).disabled(draft.applying(to:draft.source)==nil)}
        }.textFieldStyle(.roundedBorder).padding(22).frame(width:420)
            .onSubmit {if draft.applying(to:draft.source) != nil {save(draft)}}.submitScope()
    }
}
