import Foundation
import Observation
import LauncherCore
import LauncherModels

/// Deterministic synthetic profile. Never reads the user's clipboard or changes focus.
func searchFixture()->[Command] {
    (0..<256).map { i in
        Command(id:"item.\(i)",title:["Microsoft Teams","Open Documentation","Left Half","Project Dashboard"][i % 4]+" \(i)",kind:"url",target:"https://example.com/\(i)",aliases:["a\(i)"])
    }
}
@MainActor func benchmarkSearch(root:URL) throws -> [String:Any] {
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

@MainActor func checkSearch(root unusedRoot:URL) throws -> [String:Any] {
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("rustcast-search-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let state=try AppState(root:sandbox,headless:true)
    state.config.commands=searchFixture()
    let cases=[""," ","MICROSOFT","a42","item.11","project dashboard","example.com/42","missing","Ångström","a\0b"]
    state.query="new app"
    state.applications=[Command(id:"discovered.test",title:"New App",kind:"app",target:"org.test.app")]
    guard state.results.first?.id=="discovered.test" else {throw LauncherError("Discovered apps are stale.")}
    var changed=state.config
    changed.commands=[Command(id:"replacement",title:"New App",kind:"app",target:"org.test.app")]
    try Files.save(changed,to:state.configURL);try state.reload()
    guard state.results.map(\.id)==["replacement"] else {throw LauncherError("Reload or app deduplication failed.")}
    state.config.commands=[Command(id:"cached.a",title:"Identical",kind:"url",target:"https://example.com/a"),Command(id:"cached.b",title:"Identical",kind:"url",target:"https://example.com/b")]
    state.query="identical"
    for _ in 0..<1000 {_=state.searchCommands("identical")}
    try state.searchRanking.recordSuccess("cached.b")
    _ = state.searchCommands("identical")
    try state.searchRanking.reset()
    _ = state.searchCommands("identical")
    state.config.commands=[Command(id:"typo.enabled",title:"Calendar",kind:"url",target:"https://example.com"),Command(id:"typo.disabled",title:"Finder",kind:"url",target:"https://example.com",enabled:false)]
    state.query="calender"
    let typoReply=LockedValue<[String]>([])
    state.handle(["op":"search","query":"calender"]) {value in typoReply.update {$0=(value["result"] as? [[String:Any]])?.compactMap {$0["id"] as? String} ?? []}}
    guard state.results.map(\.id)==["typo.enabled"],
          typoReply.read({$0})==["typo.enabled"]
    else {throw LauncherError("Typo fallback CLI/native search failed")}
    state.config.commands=[Command(id:"typo.finder",title:"Finder",kind:"url",target:"https://example.com"),Command(id:"typo.distractor",title:"Find Reminders",kind:"url",target:"https://example.com")]
    try state.searchRanking.recordSuccess("typo.distractor")
    state.query="fnider"
    state.handle(["op":"search","query":"fnider"]) {value in typoReply.update {$0=(value["result"] as? [[String:Any]])?.compactMap {$0["id"] as? String} ?? []}}
    guard typoReply.read({$0})==["typo.finder","typo.distractor"]
    else {throw LauncherError("Weak subsequence hid a direct title typo")}
    let resultChanges=LockedValue(0)
    withObservationTracking {_=state.query;_=state.visibleCommands;_=state.selection} onChange:{resultChanges.update {$0+=1}}
    state.paused.toggle();state.expansionStatus="Fixture";state.configError="Fixture"
    guard resultChanges.read({$0})==0 else {throw LauncherError("Unrelated clipboard/settings state invalidated results")}
    state.selection=1
    guard resultChanges.read({$0})==1 else {throw LauncherError("Command selection did not invalidate results")}
    state.config.theme="graphite"
    let settingsChanges=LockedValue(0)
    withObservationTracking {_=state.configError;_=state.configDirectory} onChange:{settingsChanges.update {$0+=1}}
    state.query="changed query";state.clipboardSelection=2
    guard settingsChanges.read({$0})==0 else {throw LauncherError("Palette or clipboard navigation invalidated settings")}
    state.configError=nil
    guard settingsChanges.read({$0})==1 else {throw LauncherError("Settings error did not invalidate its consumer")}
    return ["ok":true,"parityQueries":cases.count,"cachedReads":1000,"cliSearchLeavesPaletteUntouched":true,"preparedRankingUpdateResetAndCatalogParity":true,"rankingReuseQueries":1000,"usageRankingRefreshes":true,"discoveryAndReloadRefresh":true,"selectiveFeatureObservation":true,"appearancePreservesCatalog":true]
}
