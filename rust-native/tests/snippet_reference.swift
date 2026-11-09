// Exact current production output/clipboard code is linked only into this
// test oracle. All board effects use an owned unique named pasteboard.
import AppKit
import LauncherCore
private struct Fixture:Decodable {
    let source:String
    let mode:String?
    let cursor:Int?
    let label:String?
    let url:String?
    let clipboard:Bool?
    let seed:Bool?
}
@main struct SnippetReference {
    @MainActor static func main() throws {
        let data=FileHandle.standardInput.readDataToEndOfFile()
        guard data.count<=1_048_576 else {throw NSError(domain:"Fixture input exceeds limit",code:1)}
        let fixtures=try JSONDecoder().decode([Fixture].self,from:data)
        guard fixtures.count<=2048 else {throw NSError(domain:"Too many fixture cases",code:1)}
        let cases:[[String:Any]]=fixtures.map {f in
            let payload=SnippetOutput.render(f.source,mode:f.mode,cursor:f.cursor)
            var clipboard:Any=NSNull()
            if f.clipboard==true {
                let board=NSPasteboard.withUniqueName()
                defer {board.clearContents();board.releaseGlobally()}
                if f.seed==true {
                    board.setString("Own previous text",forType:.string)
                    board.setString("<b>Own previous HTML</b>",forType:.html)
                }
                let before=board.changeCount
                let success=SnippetClipboard.write(payload,to:board)
                clipboard=["success":success,"changes":board.changeCount-before,"text":board.string(forType:.string) as Any? ?? NSNull(),"html":board.string(forType:.html) as Any? ?? NSNull(),"types":(board.types ?? []).map {$0.rawValue}.sorted(),"items":board.pasteboardItems?.count ?? 0] as [String:Any]
            }
            let steps=payload.cursor.flatMap {cursor in cursor>=0 ? (payload.text as NSString).substring(from:min(cursor,payload.text.utf16.count)).count:nil}
            return ["payload":["text":payload.text,"html":payload.html as Any? ?? NSNull(),"cursor":payload.cursor as Any? ?? NSNull()],"cursorSteps":steps as Any? ?? NSNull(),"links":SnippetOutput.links(f.source).map {link in ["range":["location":link.range.location,"length":link.range.length],"labelRange":["location":link.labelRange.location,"length":link.labelRange.length],"label":link.label,"url":link.url] as [String:Any]},"validURL":SnippetOutput.validURL(f.url ?? ""),"linkSource":SnippetOutput.linkSource(label:f.label ?? "",url:f.url ?? ""),"clipboard":clipboard]
        }
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject:["ok":true,"cases":cases],options:[.sortedKeys]))
    }
}
