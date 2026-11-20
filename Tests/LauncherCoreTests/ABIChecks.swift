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
