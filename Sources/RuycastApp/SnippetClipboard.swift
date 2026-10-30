import AppKit
import LauncherCore

@MainActor enum SnippetClipboard {
    @discardableResult static func write(_ payload:SnippetOutput.Payload,to board:NSPasteboard)->Bool {
        let item=NSPasteboardItem()
        guard item.setString(payload.text,forType:.string) else {return false}
        if let html=payload.html {guard item.setString(html,forType:.html) else {return false}}
        board.clearContents();return board.writeObjects([item])
    }
    /// Prepare the complete rich item before deleting any trigger text.
    @discardableResult static func apply(_ replacement:SnippetReplacement,mode:String?,board:NSPasteboard,onWrite:()->Void,key:(UInt16)->Void,paste:()->Void,moveCursor:(Int)->Void,insert:(String)->Void)->Bool {
        guard mode=="teams" || mode=="html" else {replacement.apply(key:key,insert:insert);return true}
        let payload=SnippetOutput.render(replacement.text,mode:mode,cursor:replacement.cursor)
        guard write(payload,to:board) else {return false}
        onWrite()
        for _ in 0..<replacement.eraseCount {key(51)}
        paste()
        if let cursor=payload.cursor {
            let steps=(payload.text as NSString).substring(from:min(cursor,payload.text.utf16.count)).count
            moveCursor(min(steps,10000))
        }
        return true
    }
}
