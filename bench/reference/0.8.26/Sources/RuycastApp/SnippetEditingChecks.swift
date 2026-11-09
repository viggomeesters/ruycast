import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkSnippetEditing(_ directory:URL) throws -> [String:Any] {
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    let source="Hoi 👋,\n\nSee [Guide](https://example.com/a_(b)?x=1&y=2)\n\nGoodbye\n"
    try require(SnippetLineRuler.lineStarts("😀\n\n")==[0,3,4],"Logical UTF16 line starts wrong")
    var draft=SnippetLinkDraft.make(source:source,selection:NSRange(location:20,length:0))
    try require(draft.label=="Guide","Caret did not select existing link")
    draft.label="New guide";draft.url="https://example.com/changed?a=1&b=2"
    let updated=draft.applying(to:source)
    try require(updated?.0.contains("[New guide](https://example.com/changed?a=1&b=2)")==true,"Link edit lost source")
    try require(draft.applying(to:source+"changed")==nil,"Stale link form overwrote text")
    draft.url="javascript:alert(1)";try require(draft.applying(to:source)==nil,"Unsafe link accepted")
    let board=NSPasteboard(name:NSPasteboard.Name("ruycast-snippet-fixture-"+UUID().uuidString));defer {board.releaseGlobally()}
    var events=[String]()
    let replacement=SnippetReplacement(keyword:";test",finalInput:"t",text:source,cursor:nil)
    for mode in ["teams","html"] {
        events=[]
        let ok=SnippetClipboard.apply(replacement,mode:mode,board:board,onWrite:{events.append("write")},key:{events.append("key:\($0)")},paste:{events.append("paste")},moveCursor:{events.append("cursor:\($0)")},insert:{_ in events.append("unicode")})
        try require(ok && events.first=="write" && events.last=="paste" && events.filter {$0=="key:51"}.count==4 && !events.contains("unicode"),"Rich keyword route is not one paste after preparation")
        try require(board.pasteboardItems?.count==1 && board.string(forType:.html)?.contains("href=")==true && board.string(forType:.string)=="Hoi 👋,\n\nSee Guide\n\nGoodbye\n","Named clipboard lost links or newlines")
        let parsed=try NSAttributedString(data:Data(board.string(forType:.html)!.utf8),options:[.documentType:NSAttributedString.DocumentType.html],documentAttributes:nil)
        var links=0;parsed.enumerateAttribute(.link,in:NSRange(location:0,length:parsed.length)) {value,_,_ in if value != nil {links+=1}}
        try require(links==1 && parsed.string.contains("Guide"),"HTML hyperlink could not be imported")
        if mode=="teams" {
            try require(!board.string(forType:.html)!.lowercased().contains("<table"),"Teams clipboard still contains a table")
            var hasTextTable=false
            parsed.enumerateAttribute(.paragraphStyle,in:NSRange(location:0,length:parsed.length)) {value,_,_ in
                if let style=value as? NSParagraphStyle,!style.textBlocks.isEmpty {hasTextTable=true}
            }
            try require(!hasTextTable && parsed.string == board.string(forType:.string)!+"\n","Teams import contains table blocks or lost blank paragraphs")
        }
    }
    for sample in ["","a","a\n","\n","\n\n","a\n\n","a\n \t\nb"] {
        let payload=SnippetOutput.render(sample,mode:"teams")
        let parsed=try NSAttributedString(data:Data(payload.html!.utf8),options:[.documentType:NSAttributedString.DocumentType.html],documentAttributes:nil)
        let visible=sample.components(separatedBy:"\n").map {$0.trimmingCharacters(in:.whitespaces).isEmpty ? "":$0}.joined(separator:"\n")
        try require(parsed.string==visible+"\n","Teams paragraph import lost a leading, trailing or consecutive blank line")
        parsed.enumerateAttribute(.paragraphStyle,in:NSRange(location:0,length:parsed.length)) {value,_,_ in
            if let style=value as? NSParagraphStyle {precondition(style.textBlocks.isEmpty,"Teams output imported as a table")}
        }
    }
    events=[]
    try require(SnippetClipboard.apply(replacement,mode:nil,board:board,onWrite:{events.append("write")},key:{_ in},paste:{events.append("paste")},moveCursor:{events.append("cursor:\($0)")},insert:{_ in events.append("unicode")}) && events.contains("unicode") && !events.contains("write"),"Legacy keyword behavior changed")
    // A real native editor verifies wrapping, line gutter, edit, selection and undo offscreen.
    let window=NSWindow(contentRect:NSRect(x:0,y:0,width:620,height:680),styleMask:.borderless,backing:.buffered,defer:false);window.isReleasedWhenClosed=false;defer {window.close()}
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("snippet-fixture-"+UUID().uuidString);defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    var command=Command(id:"fixture.snippet",title:"Setup message",kind:"snippet",target:source,keyword:";setup",snippetOutput:"teams")
    var renders=0
    func pump() {let end=Date().addingTimeInterval(0.15);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    for dark in [false,true] {for width:CGFloat in [520,760] {
        window.setContentSize(NSSize(width:width,height:680));command.snippetOutput=dark ? "html":"teams"
        let host=NSHostingView(rootView:CommandEditor(state:state,form:CommandForm(command:command,isRun:false)).background(SettingsTheme(dark:dark).background).environment(\.colorScheme,dark ? .dark:.light));host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);window.contentView=host;host.frame=NSRect(x:0,y:0,width:width,height:680);pump();host.layoutSubtreeIfNeeded()
        let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds)!;host.cacheDisplay(in:host.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("snippet-\(dark ? "dark":"light")-\(Int(width)).png"));renders+=1
        func find(_ view:NSView)->NSTextView? {if let e=view as? NSTextView,e.enclosingScrollView?.verticalRulerView is SnippetLineRuler {return e};for v in view.subviews {if let e=find(v) {return e}};return nil}
        guard let editor=find(host) else {throw LauncherError("Snippet editor is absent")}
        try require(editor.string==source && editor.enclosingScrollView?.rulersVisible==true,"Editor source or line gutter missing")
        let ruler=editor.enclosingScrollView!.verticalRulerView!
        try require(SnippetLineRuler.lineStarts(editor.string).count==6 && ruler.ruleThickness>=44,"Gutter lost trailing blank lines")
        editor.setSelectedRange(NSRange(location:0,length:0));editor.insertText("Changed ",replacementRange:NSRange(location:0,length:0));pump()
        try require(editor.string.hasPrefix("Changed "),"Native typing failed")
        editor.undoManager?.undo();pump();try require(editor.string==source,"Native undo failed")
        let long=String(repeating:"wrapped word ",count:40)+"\n"+(1...70).map {"Line \($0)"}.joined(separator:"\n")+"\n"
        editor.insertText(long,replacementRange:NSRange(location:0,length:editor.string.utf16.count));pump()
        let layout=editor.layoutManager!;layout.ensureLayout(for:editor.textContainer!)
        let second=SnippetLineRuler.lineStarts(editor.string)[1]
        let secondY=layout.lineFragmentRect(forGlyphAt:layout.glyphIndexForCharacter(at:second),effectiveRange:nil).minY
        try require(secondY>50,"Long logical line did not wrap")
        editor.scrollRangeToVisible(NSRange(location:long.utf16.count-1,length:0));pump()
        let scrolledRuler=editor.enclosingScrollView!.verticalRulerView as! SnippetLineRuler
        let markers=scrolledRuler.markers(in:scrolledRuler.bounds)
        try require(markers.contains(where:{$0.number>50}) && !markers.contains(where:{$0.number==1}),"Gutter did not follow scroll position")
        let scrollBitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds)!;host.cacheDisplay(in:host.bounds,to:scrollBitmap)
        try scrollBitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("snippet-scroll-\(dark ? "dark":"light")-\(Int(width)).png"))

    }}
    let linkHost=NSHostingView(rootView:SnippetLinkForm(draft:SnippetLinkDraft.make(source:source,selection:NSRange(location:20,length:0)),save:{_ in},cancel:{}).background(SettingsTheme(dark:false).background));linkHost.appearance=NSAppearance(named:.aqua);window.contentView=linkHost;window.setContentSize(NSSize(width:420,height:250));linkHost.frame=NSRect(x:0,y:0,width:420,height:250);pump()
    let bitmap=linkHost.bitmapImageRepForCachingDisplay(in:linkHost.bounds)!;linkHost.cacheDisplay(in:linkHost.bounds,to:bitmap);try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("snippet-link.png"))
    try require(!window.isVisible,"Snippet test opened a visible window")
    return ["renders":renders*2+1,"richNamedClipboard":true,"legacyUnicode":true,"nativeUndo":true,"linkEditingAndStaleProtection":true,"visibleWindowOpened":false,"desktopInputSent":false]
}
