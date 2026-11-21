import Foundation
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
