func XCTAssertEqual<T:Equatable>(_ a:T,_ b:T) {precondition(a==b,"Expected \(a) == \(b)")}
func XCTAssertEqual(_ a:Double,_ b:Double,accuracy:Double) {precondition(abs(a-b)<=accuracy)}
func XCTAssertThrowsError<T>(_ expression:@autoclosure () throws -> T) {do {_ = try expression();preconditionFailure("Expected an error")} catch {}}
import Foundation
import LauncherCore
import LauncherModels
struct CoreTests {
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
        XCTAssertEqual(index.search("does not exist"),[])
        var legacy=Configuration.initial;legacy.commands=commands
        for query in ["go"," BETA ","ÅNGSTRÖM notes","nul\0alias","   ","does not exist","example"] {
            XCTAssertEqual(index.search(query),legacy.search(query))
        }
        for _ in 0..<100 {let temporary=try CommandSearchIndex(commands:commands);XCTAssertEqual(temporary.search("go").first?.id,"z")}
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
}

@main struct CheckRunner {static func main() throws {let t=CoreTests();try t.testPersistentIndexRankingAndUnicode();try t.testWindowReadsRetryOnlyTemporaryFailures();print("Core behavior checks passed")
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
