import Foundation
import LauncherModels
import LauncherCore
import CRuycastCore

func checkImportedABI() throws {
    precondition(rustcast_abi_version()==1)
    // The C-imported declarations call the actual linked Rust archive.
    guard let error=rustcast_call(nil,nil) else {throw LauncherError("Missing ABI error response")}
    let rejected=try JSONSerialization.jsonObject(with:Data(String(cString:error).utf8)) as? [String:Any]
    rustcast_free(error)
    precondition(rejected?["ok"] as? Bool==false)
    rustcast_free(nil);rustcast_index_free(nil)
    let commands=[Command(id:"first",title:"Same",kind:"url",target:"https://example.com"),Command(id:"second",title:"Same",kind:"url",target:"https://example.com")]
    let json=String(decoding:try Files.encode(commands),as:UTF8.self)
    guard let index=json.withCString({rustcast_index_new($0)}) else {throw LauncherError("ABI catalog creation failed")}
    defer {rustcast_index_free(index)}
    var output=[Int](repeating:777,count:4)
    let query=Array("same".utf8)
    let count=output.withUnsafeMutableBufferPointer {out in query.withUnsafeBufferPointer {q in rustcast_index_search_ranked_into(index,q.baseAddress,query.count,nil,0,out.baseAddress!.advanced(by:1),1)}}
    precondition(count==1 && output==[777,0,777,777],"ABI output length/canary")
    let invalid:[UInt8]=[0xff]
    precondition(invalid.withUnsafeBufferPointer {rustcast_index_search_ranked_into(index,$0.baseAddress,1,nil,0,nil,0)} == -1)
    precondition(query.withUnsafeBufferPointer {rustcast_index_search_ranked_into(nil,$0.baseAddress,query.count,nil,0,nil,0)} == -1)
    precondition(rustcast_index_search_ranked_into(index,nil,1,nil,0,nil,0) == -1)
    let badWeights=[-1.0,0.0]
    precondition(query.withUnsafeBufferPointer {q in badWeights.withUnsafeBufferPointer {w in rustcast_index_search_ranked_into(index,q.baseAddress,query.count,w.baseAddress,w.count,nil,0)}} == -1)
    let prepared=try CommandSearchIndex(commands:commands).prepared(recency:["second":5])
    // The snapshot retains its catalog owner even after the local creator dies.
    precondition(prepared.search("same").map(\.id)==["second","first"])
    precondition(prepared.search("").map(\.id)==["first","second"])
}

func checkPreparedRanking() throws {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("prepared-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    let ranking=SearchRanking(root:root)
    let index=try CommandSearchIndex(commands:[Command(id:"a",title:"Same",kind:"url",target:"https://example.com"),Command(id:"b",title:"Same",kind:"url",target:"https://example.com")])
    let original=index.prepared(recency:ranking.entries),initial=ranking.revision
    try ranking.recordSuccess("b")
    precondition(ranking.revision != initial)
    let trained=index.prepared(recency:ranking.entries)
    precondition(original.search("same").first?.id=="a" && trained.search("same").first?.id=="b")
    let beforeReset=ranking.revision;try ranking.reset();precondition(ranking.revision != beforeReset)
    precondition(index.prepared(recency:ranking.entries).search("same").first?.id=="a")
    let replacement=try CommandSearchIndex(commands:[Command(id:"new",title:"Same",kind:"url",target:"https://example.com")])
    precondition(replacement.prepared(recency:["b":100]).search("same").map(\.id)==["new"])
    // Failed durable update must not change revision or active ranking.
    try FileManager.default.removeItem(at:root);try Data("not a directory".utf8).write(to:root)
    let revision=ranking.revision,entries=ranking.entries
    do {try ranking.recordSuccess("b");preconditionFailure("Expected persistence failure")} catch {}
    precondition(ranking.revision==revision && ranking.entries==entries)
}
