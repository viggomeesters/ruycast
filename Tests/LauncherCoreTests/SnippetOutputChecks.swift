import Foundation
import LauncherCore
import LauncherModels

func checkSnippetOutput() throws {
    let source="Hi 👋\n\nSee [Guide & tips](https://example.com/a_(b)?x=1&y=2)\nGoodbye\n"
    let expected="Hi 👋\n\nSee Guide & tips\nGoodbye\n"
    for mode in ["teams","html"] {
        let value=SnippetOutput.render(source,mode:mode,cursor:source.utf16.count)
        precondition(value.text==expected && value.cursor==expected.utf16.count)
        precondition(value.html!.contains("href=\"https://example.com/a_(b)?x=1&amp;y=2\""))
        precondition(value.html!.contains("Guide &amp; tips</a>") && !value.html!.contains("[Guide"))
        if mode=="teams" {precondition(!value.html!.contains("<table") && !value.html!.contains("<td") && value.html!.components(separatedBy:"<p style=\"margin:0\">").count==6 && value.html!.contains("<p style=\"margin:0\"><br></p>"),"Teams snippets must be normal paragraphs, never a visible table")}
        else {precondition(value.html!.contains("Hi 👋<br><br>See"))}
    }
    precondition(SnippetOutput.links("[line1\nline2](https://example.com)").isEmpty)
    let labeled="a[😀](https://example.com)b"
    for (sourceOffset,expectedOffset) in [(0,0),(1,1),(labeled.utf16.count-1,3),(labeled.utf16.count,4)] {
        precondition(SnippetOutput.render(labeled,mode:"html",cursor:sourceOffset).cursor==expectedOffset)
    }
    let cursorSource="a[hi](https://example.com)b"
    precondition(SnippetOutput.render(cursorSource,mode:"html",cursor:3).cursor==2)
    print("Snippet output checks passed")
}
