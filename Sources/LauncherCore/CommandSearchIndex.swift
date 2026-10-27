import Foundation
import LauncherModels
import CRuycastCore

/// Owns one immutable Rust catalog. All per-call buffers are caller-owned.
/// Prepared ranking snapshots retain this owner so borrowed handles cannot outlive it.
public final class CommandSearchIndex {
    public let commands:[Command]
    private let index:OpaquePointer
    public init(commands:[Command]) throws {
        guard rustcast_abi_version()==1 else {throw LauncherError("Unsupported Rust core ABI.")}
        self.commands=commands
        let bytes=try JSONEncoder().encode(commands)
        guard let pointer=String(decoding:bytes,as:UTF8.self).withCString({rustcast_index_new($0)}) else {throw LauncherError("Could not index commands.")}
        index=pointer
    }
    deinit {rustcast_index_free(index)}
    /// Cold/update boundary. Resolve ID weights once for this exact catalog.
    public func prepared(recency:[String:Double])->PreparedCommandSearch {
        PreparedCommandSearch(owner:self,weights:recency.isEmpty ? []:commands.map {recency[$0.id] ?? 0})
    }
    /// Compatibility/stateless entry. Interactive callers retain a prepared snapshot.
    public func search(_ query:String,recency:[String:Double]=[:])->[Command] {
        prepared(recency:recency).search(query)
    }
    fileprivate func search(_ query:String,weights:[Double])->[Command] {
        guard !commands.isEmpty else {return []}
        let bytes=Array(query.utf8)+[0]
        var ids=[Int](repeating:0,count:commands.count)
        let current=query.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty ? []:weights
        let count=ids.withUnsafeMutableBufferPointer {output in
            current.withUnsafeBufferPointer {weights in
                bytes.withUnsafeBufferPointer {rustcast_index_search_ranked_into(index,$0.baseAddress!,bytes.count-1,weights.baseAddress,weights.count,output.baseAddress,output.count)}
            }
        }
        guard count>=0,count<=ids.count,ids.prefix(count).allSatisfy({commands.indices.contains($0)}) else {return []}
        return ids.prefix(count).map {commands[$0]}
    }
}

/// Immutable ranking aligned to one retained catalog. No mutable shared scratch buffer.
public struct PreparedCommandSearch {
    fileprivate let owner:CommandSearchIndex
    fileprivate let weights:[Double]
    public func search(_ query:String)->[Command] {owner.search(query,weights:weights)}
}
