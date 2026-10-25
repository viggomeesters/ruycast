import Foundation
import LauncherCore

/// Deterministic synthetic profile. Never reads the user's clipboard or changes focus.
func searchFixture()->[Command] {
    (0..<256).map { i in
        Command(id:"item.\(i)",title:["Microsoft Teams","Open Documentation","Left Half","Project Dashboard"][i % 4]+" \(i)",kind:"url",target:"https://example.com/\(i)",aliases:["a\(i)"])
    }
}
func benchmarkSearch(root:URL) throws -> [String:Any] {
    let state=try AppState(root:root,headless:true)
    state.config.commands=searchFixture()
    let queries=["m","mi","mic","micro","microsoft","a42","left","project dashboard","missing",""]
    var search:[Double]=[],navigation:[Double]=[]
    var checksum=0
    for i in 0..<300 {
        let start=DispatchTime.now().uptimeNanoseconds
        state.query=queries[i % queries.count]
        checksum += state.results.count
        let end=DispatchTime.now().uptimeNanoseconds
        if i>=20 {search.append(Double(end-start)/1_000_000)}
        let navStart=DispatchTime.now().uptimeNanoseconds
        for _ in 0..<5 {checksum += state.results.count}
        if i>=20 {navigation.append(Double(DispatchTime.now().uptimeNanoseconds-navStart)/1_000_000/5)}
    }
    func stats(_ samples:[Double])->[String:Double] {let s=samples.sorted();return ["medianMS":s[s.count/2],"p95MS":s[Int(Double(s.count)*0.95)]]}
    return ["catalogCount":256,"samples":280,"queryAndResults":stats(search),"repeatedResultsRead":stats(navigation),"checksum":checksum,"scope":"in-process main-thread search/result access; synthetic catalog; excludes rendering, app opening and display latency"]
}

func checkSearch(root unusedRoot:URL) throws -> [String:Any] {
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("rustcast-search-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let state=try AppState(root:sandbox,headless:true)
    state.config.commands=searchFixture()
    let catalog=try CommandSearchIndex(commands:state.config.commands)
    let cases=[""," ","MICROSOFT","a42","item.11","project dashboard","example.com/42","missing","Ångström","a\0b"]
    for q in cases {
        guard catalog.search(q)==state.config.search(q) else {throw LauncherError("Indexed search parity failed.")}
    }
    state.query="left"
    let before=state.searchEvaluations,results=state.results
    for _ in 0..<1000 {_=state.results}
    guard before==state.searchEvaluations,!results.isEmpty else {throw LauncherError("Results reads repeated search.")}
    var changes=0
    let subscription=state.objectWillChange.sink {changes+=1}
    var reply:[String:Any]=[:]
    state.handle(["op":"search","query":"a42"]) {reply=$0}
    guard state.query=="left",state.results==results,changes==0,reply["ok"] as? Bool==true else {throw LauncherError("CLI search changed the palette.")}
    withExtendedLifetime(subscription) {}
    state.query=""
    state.counts["item.42"]=10
    guard state.results.first?.id=="item.42" else {throw LauncherError("Usage ranking is stale.")}
    state.query="new app"
    state.applications=[Command(id:"discovered.test",title:"New App",kind:"app",target:"org.test.app")]
    guard state.results.first?.id=="discovered.test" else {throw LauncherError("Discovered apps are stale.")}
    var changed=state.config
    changed.commands=[Command(id:"replacement",title:"New App",kind:"app",target:"org.test.app")]
    try Files.save(changed,to:state.configURL);try state.reload()
    guard state.results.map(\.id)==["replacement"] else {throw LauncherError("Reload or app deduplication failed.")}
    return ["ok":true,"parityQueries":cases.count,"cachedReads":1000,"cliSearchLeavesPaletteUntouched":true,"usageRankingRefreshes":true,"discoveryAndReloadRefresh":true]
}
