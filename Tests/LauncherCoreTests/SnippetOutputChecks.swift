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
    precondition(SnippetOutput.render(source,mode:nil).text==source && SnippetOutput.render(source,mode:nil).html==nil)
    precondition(SnippetOutput.links("[line1\nline2](https://example.com)").isEmpty)
    let unsafe="<img src=x> [X](javascript:alert(1)) [Y](file:///etc/passwd)"
    let escaped=SnippetOutput.render(unsafe,mode:"html").html!
    precondition(!escaped.contains("<img") && !escaped.contains("<a "))
    let labeled="a[😀](https://example.com)b"
    for (sourceOffset,expectedOffset) in [(0,0),(1,1),(labeled.utf16.count-1,3),(labeled.utf16.count,4)] {
        precondition(SnippetOutput.render(labeled,mode:"html",cursor:sourceOffset).cursor==expectedOffset)
    }
    let triple="[Folder](https://example.com/folder)\n[Guide](https://example.com/guide?a=1&b=2)\n[Starter](https://example.com/starter.zip)"
    precondition(SnippetOutput.render(triple,mode:"teams").html!.components(separatedBy:"<a href=").count==4)
    let choice=Command(id:"choice.rich",title:"Rich choice",kind:"snippet",target:"Hi {argument name=\"Person\" options=\"A,B\"}\n\n[Guide](https://example.com)",keyword:";choice",snippetOutput:"teams")
    let request=SnippetChoiceRequest(command:choice,pid:123)
    let replacement=try request.replacement(values:["Person":"B"],pid:123,safe:true,unchanged:true)
    precondition(replacement.text.hasPrefix("Hi B\n\n") && SnippetOutput.render(replacement.text,mode:choice.snippetOutput).html!.contains("<a href="))
    let original=SnippetOutput.linkSource(label:"[Guide]",url:"https://example.com/?a=1&b=2")
    precondition(SnippetOutput.links(original).first?.label=="[Guide]")
    let cursorSource="a[hi](https://example.com)b"
    precondition(SnippetOutput.render(cursorSource,mode:"html",cursor:3).cursor==2)
    for label in [#"a\]b"#,#"a\b"#,"[brackets]","😀"] {
        let encoded=SnippetOutput.linkSource(label:label,url:"https://example.com/a)b")
        precondition(SnippetOutput.links(encoded).count==1 && SnippetOutput.render(encoded,mode:"teams").text==label)
        precondition(SnippetOutput.links(encoded).first?.url=="https://example.com/a%29b")
    }
    for mode in SnippetOutput.modes {
        let c=Command(id:"fixture.rich",title:"Rich",kind:"snippet",target:source,snippetOutput:mode)
        let copy=try JSONDecoder().decode(Command.self,from:JSONEncoder().encode(c))
        precondition(copy==c)
        var config=Configuration.initial;config.commands=[c];try config.validate()
    }
    var bad=Configuration.initial;bad.commands=[Command(id:"bad",title:"Bad",kind:"snippet",target:"x",snippetOutput:"other")]
    do {try bad.validate();preconditionFailure("Unknown output accepted")} catch {}
    print("Snippet output checks passed")
}
