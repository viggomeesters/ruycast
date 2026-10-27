func XCTAssertEqual<T:Equatable>(_ a:T,_ b:T) {precondition(a==b,"Expected \(a) == \(b)")}
func XCTAssertEqual(_ a:Double,_ b:Double,accuracy:Double) {precondition(abs(a-b)<=accuracy)}
func XCTAssertThrowsError<T>(_ expression:@autoclosure () throws -> T) {do {_ = try expression();preconditionFailure("Expected an error")} catch {}}
import Foundation
import LauncherCore
import LauncherModels
struct CoreTests {
    func testEmptyQueryProjection() {
        let commands=["first","second","third","fourth"].map {Command(id:$0,title:$0,kind:"url",target:"https://example.com")}
        let usage=["second":4,"third":4,"unknown":100]
        XCTAssertEqual(CommandProjection.emptyQuery(commands,usage:usage,limit:0).map(\.id),["second","third","first","fourth"])
        XCTAssertEqual(CommandProjection.emptyQuery(commands,usage:usage,limit:2).map(\.id),["second","third"])
        XCTAssertEqual(CommandProjection.emptyQuery(commands,usage:usage,limit:-1).count,4)
        XCTAssertEqual(CommandProjection.emptyQuery(commands,usage:[:],limit:100),commands)
        XCTAssertEqual(CommandProjection.emptyQuery([],usage:usage,limit:100),[])
    }
    func testConfigRejectsAmbiguousAliasesAndHotkeys() throws {
        XCTAssertEqual(try Shortcut("right-cmd+a").commandSide,"right-cmd")
        XCTAssertEqual(try Shortcut("left-cmd+a").commandSide,"left-cmd")
        XCTAssertEqual(try Shortcut("cmd+a").commandSide,nil)
        XCTAssertThrowsError(try Shortcut("cmd+right-cmd+a"))
        var c=Configuration.initial;try c.validate()
        XCTAssertEqual(c.separatesCommandKeys,false)
        c.separateCommandKeys=nil;XCTAssertEqual(c.separatesCommandKeys,true)
        c.separateCommandKeys=false;XCTAssertEqual(c.separatesCommandKeys,false)
        c.separateCommandKeys=true;try c.validate();XCTAssertEqual(c.separatesCommandKeys,true)
        c.commands.append(Command(id:"collision",title:"Collision",kind:"path",target:"/",aliases:["FiNdEr"]))
        XCTAssertThrowsError(try c.validate())
        c=Configuration.initial;c.commands[0].hotkey="alt+ctrl+space";XCTAssertThrowsError(try c.validate())
        c=Configuration.initial;c.version=2;XCTAssertThrowsError(try c.validate())
    }
    func testExactAliasBeatsTitleAndSupportsMultipleTerms() throws {
        var c=Configuration.initial;c.commands.append(Command(id:"link",title:"Finder documentation",kind:"url",target:"https://example.com",aliases:["docs"]))
        XCTAssertEqual(c.search("finder").first?.id,"app.finder");XCTAssertEqual(c.search("finder documentation").first?.id,"link")
        XCTAssertEqual(c.resolve("DOCS")?.id,"link")
    }
    func testPersistentIndexRankingAndUnicode() throws {
        let commands=[
            Command(id:"z",title:"Beta",kind:"url",target:"https://example.com/one",aliases:["go"]),
            Command(id:"a",title:"Go Alpha",kind:"url",target:"https://example.com/two"),
            Command(id:"b",title:"Ångström Notes",kind:"url",target:"https://example.com/three",aliases:["nul\0alias"]),
            Command(id:"c",title:"Beta",kind:"url",target:"https://example.com/four")]
        let index=try CommandSearchIndex(commands:commands)
        XCTAssertEqual(index.search("go").map(\.id),["z","a","b"])
        XCTAssertEqual(index.search(" BETA ").map(\.id),["z","c"])
        XCTAssertEqual(index.search("ÅNGSTRÖM notes").map(\.id),["b"])
        XCTAssertEqual(index.search("nul\0alias").map(\.id),["b"])
        XCTAssertEqual(index.search("   "),commands)
        XCTAssertEqual(index.search("",recency:["z":999]),commands)
        XCTAssertEqual(index.search("does not exist"),[])
        var legacy=Configuration.initial;legacy.commands=commands
        for query in ["go"," BETA ","ÅNGSTRÖM notes","nul\0alias","   ","does not exist","example"] {
            XCTAssertEqual(index.search(query),legacy.search(query))
        }
        XCTAssertEqual(try CommandSearchIndex(commands:[]).search(""),[])
        for _ in 0..<100 {let temporary=try CommandSearchIndex(commands:commands);XCTAssertEqual(temporary.search("go").first?.id,"z")}
        let quality=try CommandSearchIndex(commands:[
            Command(id:"exact",title:"Zulu",kind:"url",target:"https://example.com",aliases:["mt"]),
            Command(id:"teams",title:"Microsoft Teams",kind:"url",target:"https://example.com"),
            Command(id:"recent",title:"Many Tabs",kind:"url",target:"https://example.com"),
            Command(id:"history",title:"Microsoft Teams History",kind:"url",target:"https://example.com")])
        XCTAssertEqual(quality.search("mt",recency:["recent":100]).prefix(3).map(\.id),["exact","recent","teams"])
        XCTAssertEqual(quality.search("mic tea").first?.id,"teams")
        XCTAssertEqual(quality.search("mft").first?.id,"teams")
        XCTAssertEqual(quality.search("Microsoft Teams",recency:["history":100]).first?.id,"teams")
        // Equal titles and weights retain catalog order, including large ties;
        // recency may reorder ties but may never outrank an exact alias.
        let ties=(0..<128).map {Command(id:"tie.\($0)",title:"Same Title",kind:"url",target:"https://example.com",aliases:$0==127 ? ["same"]:[])}
        let tied=try CommandSearchIndex(commands:ties)
        XCTAssertEqual(tied.search("title").map(\.id),ties.map(\.id))
        XCTAssertEqual(tied.search("same",recency:["tie.64":100]).prefix(3).map(\.id),["tie.127","tie.64","tie.0"])
        XCTAssertEqual(tied.search("s t").map(\.id),ties.map(\.id))
        let scalars=try CommandSearchIndex(commands:[
            Command(id:"scalar",title:"A Ω B",kind:"url",target:"https://example.com"),
            Command(id:"expanded",title:"I x \u{0307}",kind:"url",target:"https://example.com")])
        XCTAssertEqual(scalars.search("Ω").map(\.id),["scalar"])
        // Uppercase I-with-dot lowercases to two Unicode scalars in Rust.
        XCTAssertEqual(scalars.search("İ").map(\.id),["expanded"])
        let root=FileManager.default.temporaryDirectory.appendingPathComponent("ranking-"+UUID().uuidString)
        defer {try? FileManager.default.removeItem(at:root)}
        try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
        let now=Date(timeIntervalSince1970:1_800_000_000)
        var entries=Dictionary(uniqueKeysWithValues:(0..<600).map {("id.\($0)",now.timeIntervalSince1970-Double($0))})
        entries["expired"]=now.timeIntervalSince1970-SearchRanking.retention-1;entries["future"]=now.timeIntervalSince1970+61
        let path=root.appendingPathComponent("search-ranking.json")
        try JSONSerialization.data(withJSONObject:["schema":1,"entries":entries]).write(to:path)
        let ranking=SearchRanking(root:root,now:now)
        XCTAssertEqual(ranking.entries.count,512);XCTAssertEqual(ranking.entries["expired"],nil);XCTAssertEqual(ranking.entries["future"],nil)
        try ranking.recordSuccess("chosen",now:now)
        XCTAssertEqual(SearchRanking(root:root,now:now).entries["chosen"],now.timeIntervalSince1970)
        XCTAssertEqual((try FileManager.default.attributesOfItem(atPath:path.path))[.posixPermissions] as? Int,0o600)
        try ranking.reset();XCTAssertEqual(SearchRanking(root:root,now:now).entries.count,0)
    }
    func testSnippetFocusAndDateTime() throws {
        // AX-unavailable editors are not necessarily password fields.
        XCTAssertEqual(SnippetFocusPolicy.allowsExpansion(secureInput:false,subrole:nil),true)
        XCTAssertEqual(SnippetFocusPolicy.allowsExpansion(secureInput:false,subrole:"AXStandardWindow"),true)
        XCTAssertEqual(SnippetFocusPolicy.allowsExpansion(secureInput:true,subrole:nil),false)
        XCTAssertEqual(SnippetFocusPolicy.allowsExpansion(secureInput:false,subrole:"AXSecureTextField"),false)
        let command=Command(id:"now",title:"Now",kind:"snippet",target:"{datetime}",keyword:";;")
        var matcher=SnippetMatcher()
        XCTAssertEqual(matcher.type(";",commands:[command]),nil)
        XCTAssertEqual(matcher.type(";",commands:[command]),command)
        let date=Date(timeIntervalSince1970:1761420900)
        let rendered=try Templates.render(command,values:[:],now:date,locale:Locale(identifier:"en_GB"),timeZone:TimeZone(secondsFromGMT:0)!)
        XCTAssertEqual(rendered.0,"25 October 2025 19:35")
        let explicit=Command(id:"iso",title:"ISO",kind:"snippet",target:"{datetime:yyyy-MM-dd HH:mm}")
        XCTAssertEqual(try Templates.render(explicit,values:[:],now:date,locale:Locale(identifier:"en_GB"),timeZone:TimeZone(secondsFromGMT:0)!).0,"2025-10-25 19:35")
    }
    func testTemplatesAndSnippetMatching() throws {
        let link=Command(id:"link",title:"Maps",kind:"url",target:"https://example.com/search?q={query}&place={Place}")
        XCTAssertEqual(Templates.fields(link).map(\.id),["Query","Place"])
        let rendered=try Templates.render(link,values:["Query":"a&b #/Ω","Place":"New York"]).0
        let url=URLComponents(string:rendered)!
        XCTAssertEqual(url.queryItems?.first?.value,"a&b #/Ω")
        XCTAssertEqual(url.queryItems?.last?.value,"New York")
        XCTAssertThrowsError(try Templates.render(link,values:[:]))
        let snippet=Command(id:"s",title:"Greeting",kind:"snippet",target:#"Hi {clipboard}, {cursor}{argument name="Tone" options="Hello,Hi" default="Hello"}"#,keyword:";hi")
        let result=try Templates.render(snippet,values:[:],clipboard:"Ω")
        XCTAssertEqual(result.0,"Hi Ω, Hello")
        XCTAssertEqual((result.0 as NSString).substring(to:result.1!),"Hi Ω, ")
        XCTAssertThrowsError(try Templates.render(snippet,values:[:]))
        var matcher=SnippetMatcher()
        XCTAssertEqual(matcher.type(";h",commands:[snippet]),nil)
        XCTAssertEqual(matcher.type("i",commands:[snippet]),snippet)
        XCTAssertEqual(matcher.type("i",commands:[snippet]),nil)
        matcher.reset();_ = matcher.type(";hx",commands:[snippet]);matcher.backspace()
        XCTAssertEqual(matcher.type("i",commands:[snippet]),snippet)
        let word=Command(id:"w",title:"Word",kind:"snippet",target:"expanded",keyword:"abc")
        matcher.reset();XCTAssertEqual(matcher.type("xabc",commands:[word]),nil)
        matcher.reset();XCTAssertEqual(matcher.type("abc",commands:[word]),word)
        let interactive=Command(id:"i",title:"Input",kind:"snippet",target:#"{argument name="Name"}"#,keyword:";name")
        matcher.reset();XCTAssertEqual(matcher.type(";name",commands:[interactive]),nil)
        let longText=String(repeating:"Hello Ω 👨‍👩‍👧‍👦\n",count:100)
        var chunks:[String]=[]
        SnippetReplacement(keyword:";long",finalInput:"g",text:longText,cursor:nil).apply(key:{_ in},insert:{chunks.append($0)})
        XCTAssertEqual(chunks.joined(),longText)
        XCTAssertEqual(chunks.allSatisfy {$0.utf16.count<=16},true)
        var c=Configuration.initial;c.commands.append(word);c.commands.append(Command(id:"w2",title:"Word 2",kind:"snippet",target:"other",keyword:"abc"));XCTAssertThrowsError(try c.validate())
    }
    func testSnippetChoices() throws {
        let command=Command(id:"choice",title:"Contact",kind:"snippet",target:#"{argument name="Contact" options=" Alpha, Beta , , Alpha"}"#,keyword:";contact")
        XCTAssertEqual(Templates.fields(command).first?.options,["Alpha","Beta"])
        var matcher=SnippetMatcher()
        XCTAssertEqual(matcher.type(";contact",commands:[command]),command)
        XCTAssertEqual(Templates.needsInteraction(command),true)
        let defaulted=Command(id:"defaulted",title:"Contact",kind:"snippet",target:#"{argument name="Contact" options="Alpha,Beta" default="Beta"}"#,keyword:";default")
        XCTAssertEqual(Templates.needsInteraction(defaulted),true)
        XCTAssertEqual(try Templates.render(command,values:["Contact":"Beta"]).0,"Beta")
        let plain=Command(id:"plain",title:"Plain",kind:"snippet",target:"Hello",keyword:";plain")
        XCTAssertEqual(Templates.needsInteraction(plain),false)
        var keys:[UInt16]=[],inserted=""
        let confirmed=SnippetChoiceRequest(command:command,pid:42)
        try confirmed.replacement(values:["Contact":"Beta Ω"],pid:42,safe:true,unchanged:true).apply(key:{keys.append($0)},insert:{inserted+=$0})
        XCTAssertEqual(keys,Array(repeating:51,count:8));XCTAssertEqual(inserted,"Beta Ω")
        XCTAssertThrowsError(try confirmed.replacement(values:["Contact":"Alpha"],pid:42,safe:true,unchanged:true))
        let cancelled=SnippetChoiceRequest(command:command,pid:42);cancelled.cancel()
        XCTAssertThrowsError(try cancelled.replacement(values:["Contact":"Alpha"],pid:42,safe:true,unchanged:true))
        for (pid,safe,unchanged):(Int32,Bool,Bool) in [(43,true,true),(42,false,true),(42,true,false)] {
            let request=SnippetChoiceRequest(command:command,pid:42)
            XCTAssertThrowsError(try request.replacement(values:["Contact":"Alpha"],pid:pid,safe:safe,unchanged:unchanged))
        }
    }
    func testHistoryDeduplicatesBoundsAndExpires() {
        let now=Date();let s=ClipboardSettings(limit:2,maxTextBytes:20,retentionDays:1,excludedApps:["secret"])
        let a=Clip(text:"a",source:"app"),b=Clip(text:"b",source:"app")
        let items=History.insert(a,into:[b,a,Clip(text:"expired",source:"app",date:now.addingTimeInterval(-90000))],settings:s,now:now)
        XCTAssertEqual(items.map(\.text),["a","b"])
        XCTAssertEqual(History.insert(Clip(text:"password",source:"secret"),into:items,settings:s),items)
        XCTAssertEqual(History.insert(Clip(text:String(repeating:"x",count:21),source:"app"),into:items,settings:s),items)
        XCTAssertEqual(History.insert(Clip(text:"  ",source:"app"),into:items,settings:s),items)
    }
    func testWindowReadsRetryOnlyTemporaryFailures() throws {
        var calls=0;var waits:[Double]=[]
        let window:String=try WindowRead.perform("AXFocusedWindow",retryingStatus:-25204,read:{
            calls+=1;return calls<3 ? (-25204,nil):(0,"window-1")
        },pause:{waits.append($0)})
        XCTAssertEqual(window,"window-1");XCTAssertEqual(calls,3);XCTAssertEqual(waits,[0.02,0.04])

        // Exercise the same attribute-read boundary inside placement readback.
        let target=Frame(x:1720,y:0,width:1720,height:1400)
        calls=0;waits=[]
        let placed=try WindowPlacement.apply(target,read:{
            try WindowRead.perform("AXPosition",retryingStatus:-25204,read:{
                calls+=1;return calls<4 ? (-25204,nil):(0,target)
            },pause:{waits.append($0)})
        },resize:{_ in},position:{_ in},pause:{})
        XCTAssertEqual(placed,target);XCTAssertEqual(calls,4);XCTAssertEqual(waits,[0.02,0.04,0.08])

        calls=0;waits=[]
        do {
            let _:String=try WindowRead.perform("AXPosition",retryingStatus:-25204,read:{
                calls+=1;return (-25204,nil)
            },pause:{waits.append($0)})
            preconditionFailure("Persistent temporary error must fail")
        } catch {
            XCTAssertEqual(String(describing:error).contains("did not respond"),true)
        }
        XCTAssertEqual(calls,4);XCTAssertEqual(waits,[0.02,0.04,0.08])

        for status:Int32 in [-25205,-25212,-25202,0] {
            calls=0;waits=[]
            XCTAssertThrowsError(try WindowRead.perform("AXFocusedWindow",retryingStatus:-25204,read:{() -> (Int32,String?) in
                calls+=1;return (status,nil)
            },pause:{waits.append($0)}))
            XCTAssertEqual(calls,1);XCTAssertEqual(waits,[])
        }
        calls=0;waits=[]
        let immediate:Int=try WindowRead.perform("AXSize",retryingStatus:-25204,read:{calls+=1;return (0,42)},pause:{waits.append($0)})
        XCTAssertEqual(immediate,42);XCTAssertEqual(calls,1);XCTAssertEqual(waits,[])
    }
    func testWindowPlacementConvergesAfterClampingAndTransientFailure() throws {
        let target=Frame(x:1720,y:0,width:1720,height:1400)
        var current=Frame(x:100,y:400,width:900,height:600)
        var writes=0
        let actual=try WindowPlacement.apply(target,read:{current},resize:{ f in
            writes+=1
            // First request at the old position is clamped; its AX call may
            // report cannotComplete despite having changed the window.
            current.width=f.width;current.height=min(f.height,1400-current.y)
        },position:{ f in current.x=f.x;current.y=f.y },pause:{})
        XCTAssertEqual(actual,target);XCTAssertEqual(writes>=2,true)

        current=Frame(x:0,y:0,width:500,height:500);writes=0
        let retried=try WindowPlacement.apply(target,read:{current},resize:{ f in
            writes+=1
            if writes>2 {current.width=f.width;current.height=f.height}
        },position:{ f in current.x=f.x;current.y=f.y },pause:{})
        XCTAssertEqual(retried,target);XCTAssertEqual(writes,4)

        var attempts=0
        XCTAssertThrowsError(try WindowPlacement.apply(target,read:{Frame(x:0,y:0,width:800,height:600)},resize:{_ in attempts+=1},position:{_ in},pause:{}))
        XCTAssertEqual(attempts,6)
        attempts=0
        XCTAssertThrowsError(try WindowPlacement.apply(target,read:{current},resize:{_ in attempts+=1;throw LauncherError("Not resizable")},position:{_ in},pause:{}))
        XCTAssertEqual(attempts,1)
    }
    func testThirdsOnNegativeOriginDisplay() throws {
        let screen=Frame(x:-3440,y:-200,width:3440,height:1400)
        let r=try screen.layout("last-two-thirds")
        XCTAssertEqual(r.x,-3440+3440.0/3,accuracy:0.01);XCTAssertEqual(r.width,3440*2.0/3,accuracy:0.01)
        XCTAssertThrowsError(try screen.layout("other"))
    }
    func testScriptAndSettingsValidation() throws {
        var c=Configuration.initial;c.commands.append(Command(id:"s",title:"Script",kind:"script",target:"relative"));XCTAssertThrowsError(try c.validate())
        c=Configuration.initial;c.clipboard.limit=1001;XCTAssertThrowsError(try c.validate())
        XCTAssertThrowsError(try Shortcut("ctrl+ctrl+v"));XCTAssertThrowsError(try Shortcut("space"))
    }
    func testRoundTripAndPrivateStorage() throws {
        let root=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer {try? FileManager.default.removeItem(at:root)}
        let p=root.appendingPathComponent("config.json");try Files.save(Configuration.initial,to:p)
        XCTAssertEqual(try Files.config(p),Configuration.initial)
        XCTAssertEqual((try FileManager.default.attributesOfItem(atPath:p.path)[.posixPermissions] as? NSNumber)?.intValue,0o600)
    }
}

@main struct CheckRunner {static func main() throws {let probeArgs=Array(CommandLine.arguments.dropFirst());if probeArgs.count==2,probeArgs[0]=="--private-storage-probe" {do {try runPrivateStorageWriteProbe(root:URL(fileURLWithPath:probeArgs[1],isDirectory:true))} catch {FileHandle.standardError.write(Data("Private storage probe failed.\n".utf8));exit(1)};return};try checkPrivateFiles();try checkImportedABI();try checkPreparedRanking();try checkScriptChoices();checkTaskDateInput();let t=CoreTests();t.testEmptyQueryProjection();try t.testSnippetChoices();try t.testSnippetFocusAndDateTime();try t.testConfigRejectsAmbiguousAliasesAndHotkeys();try t.testExactAliasBeatsTitleAndSupportsMultipleTerms();try t.testPersistentIndexRankingAndUnicode();try t.testTemplatesAndSnippetMatching();t.testHistoryDeduplicatesBoundsAndExpires();try t.testWindowReadsRetryOnlyTemporaryFailures();try t.testWindowPlacementConvergesAfterClampingAndTransientFailure();try t.testThirdsOnNegativeOriginDisplay();try t.testScriptAndSettingsValidation();try t.testRoundTripAndPrivateStorage();print("Core behavior checks passed")
        let args=Array(CommandLine.arguments.dropFirst())
        if args.count==2,args[0]=="--library" {
            let config=try Files.config(URL(fileURLWithPath:args[1]));let index=try CommandSearchIndex(commands:config.commands)
            var rendered=0
            for command in config.commands where command.kind=="snippet" || command.kind=="url" {
                let values=Dictionary(uniqueKeysWithValues:Templates.fields(command).map {($0.id,$0.defaultValue.isEmpty ? ($0.options.first ?? "Fixture"):$0.defaultValue)})
                _=try Templates.render(command,values:values,clipboard:"Fixture",now:Date(timeIntervalSince1970:0));rendered+=1
                if let keyword=command.keyword {precondition(index.search(keyword).contains(where:{$0.id==command.id}),"Imported keyword is not searchable")}
            }
            print("Private library verified: \(config.commands.count) commands, \(rendered) link/snippet templates; no private text printed")
        }}}
