import Foundation
import LauncherModels
import LauncherCore
import CRuycastCore

func checkImportedABI() throws {
    precondition(rustcast_abi_version()==1)
    // The C-imported declarations call the actual linked Rust archive.
    let commands=[Command(id:"first",title:"Same",kind:"url",target:"https://example.com"),Command(id:"second",title:"Same",kind:"url",target:"https://example.com")]
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
}
