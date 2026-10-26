import Foundation

/// Local successful launcher choices only. Never stores queries or argument text.
public final class SearchRanking {
    public static let limit=512
    public static let retention:TimeInterval=90*86400
    private struct State:Codable {var schema=1;var entries:[String:Double]}
    private let url:URL
    public private(set) var entries:[String:Double]=[:]
    public init(root:URL,now:Date=Date()) {
        url=root.appendingPathComponent("search-ranking.json")
        if let attributes=try? FileManager.default.attributesOfItem(atPath:url.path),
           attributes[.type] as? FileAttributeType == .typeRegular,
           let size=attributes[.size] as? NSNumber,size.intValue<=1_000_000,
           let bytes=try? Data(contentsOf:url),let state=try? JSONDecoder().decode(State.self,from:bytes),state.schema==1 {
            entries=Self.bounded(state.entries,now:now)
        }
    }
    private static func bounded(_ entries:[String:Double],now:Date)->[String:Double] {
        let time=now.timeIntervalSince1970
        let valid=entries.filter {!$0.key.isEmpty && $0.key.utf8.count<=512 && $0.value.isFinite && $0.value>=0 && $0.value>time-retention && $0.value<=time+60}
        return Dictionary(uniqueKeysWithValues:valid.sorted {a,b in a.value==b.value ? a.key<b.key:a.value>b.value}.prefix(limit).map {($0.key,$0.value)})
    }
    public func recordSuccess(_ id:String,now:Date=Date()) throws {
        guard !id.isEmpty,id.utf8.count<=512 else {return}
        var next=entries;next[id]=now.timeIntervalSince1970;next=Self.bounded(next,now:now)
        try Files.save(State(entries:next),to:url);entries=next
    }
    public func reset() throws {try Files.save(State(entries:[:]),to:url);entries=[:]}
    public var summary:[String:Int] {["entries":entries.count,"limit":Self.limit,"retentionDays":90]}
}
