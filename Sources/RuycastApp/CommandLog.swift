import Foundation
import Darwin
import LauncherCore

struct CommandLogEntry:Codable {
    var date=Date()
    var traceID:String
    var commandID:String
    var source:String
    var phase:String
    var success:Bool?
    var durationMS:Double?
    var code:String?
    var message:String?
    var exitStatus:Int?
    var applied:Bool?
    var details:[String:String]?
}
struct CommandLogDocument:Codable {var schema=1;var entries:[CommandLogEntry]=[]}
/// Operational errors are separate from payload-free aggregate analytics.
/// No successful stdout, command arguments, capture text or clipboard contents are stored.
final class CommandLog {
    let url:URL
    private let lock=NSLock()
    private let writer=DispatchQueue(label:"ruycast.command-log",qos:.utility,autoreleaseFrequency:.workItem)
    private var data=CommandLogDocument()
    private var scheduled=false
    private var revision=0
    private var savedRevision=0
    private var writable=true
    private var failure:String?
    init(root:URL) {
        url=root.appendingPathComponent("command-log.json")
        do {try Self.checkDirectory(root)} catch {writable=false;failure="Command logging requires an owner-only profile folder (0700).";return}
        if FileManager.default.fileExists(atPath:url.path) || (try? url.resourceValues(forKeys:[.isSymbolicLinkKey]).isSymbolicLink)==true {
            do {
                let fd=open(url.path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC|O_NONBLOCK);guard fd>=0 else {throw LauncherError("Unavailable command log")};defer {close(fd)}
                var info=stat();guard fstat(fd,&info)==0,(info.st_mode & S_IFMT)==S_IFREG,info.st_uid==getuid(),info.st_nlink==1,info.st_size>0,info.st_size<=2_000_000 else {throw LauncherError("Invalid command log")}
                guard fchmod(fd,0o600)==0 else {throw LauncherError("Could not restrict command log permissions")}
                let bytes=try FileHandle(fileDescriptor:fd,closeOnDealloc:false).read(upToCount:2_000_001) ?? Data()
                data=try Files.decode(CommandLogDocument.self,data:bytes)
                guard bytes.count<=2_000_000,data.schema==1,data.entries.count<=500,data.entries.allSatisfy({$0.commandID.utf8.count<=512 && ($0.message?.utf8.count ?? 0)<=4096 && ($0.details?.count ?? 0)<=6}) else {throw LauncherError("Invalid command log bounds")}
                prune()
            } catch {data=CommandLogDocument();writable=false;failure="Command log could not be read. Use ruycast logs clear --confirm to reset it."}
        }
    }
    private static func checkDirectory(_ root:URL) throws {
        if !FileManager.default.fileExists(atPath:root.path) {try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])}
        let fd=open(root.path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);guard fd>=0 else {throw LauncherError("Unsafe profile directory")};defer {close(fd)}
        var info=stat();guard fstat(fd,&info)==0,info.st_uid==getuid(),info.st_mode & 0o077==0 else {throw LauncherError("Profile directory must be owner-only")}
    }
    static func clean(_ value:String,secrets:[String]=[],limit:Int=1800)->String {
        var s=String(value.prefix(16384))
        for secret in secrets.filter({!$0.isEmpty}).sorted(by:{$0.count>$1.count}) {s=s.replacingOccurrences(of:secret,with:"[redacted]")}
        s=s.replacingOccurrences(of:NSHomeDirectory(),with:"~")
        for (pattern,replacement) in [
            ("(?i)\\b(?:https?|obsidian)://[^\\s\\\"<>]+","[url]"),
            ("(?i)\\bBearer\\s+[^\\s\\\",;]+","Bearer [redacted]"),
            ("(?i)\\b(password|passwd|token|secret|api[_-]?key|authorization)\\b[\\\"']?\\s*[:=]\\s*[\\\"']?[^\\s\\\"',;]+","$1=[redacted]")
        ] {s=s.replacingOccurrences(of:pattern,with:replacement,options:.regularExpression)}
        s=s.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) || $0=="\n" || $0=="\t" }.map(String.init).joined()
        var remaining=limit,output=""
        for character in s {let bytes=character.utf8.count;guard bytes<=remaining else {break};output.append(character);remaining-=bytes}
        return output
    }
    var storageError:String? {lock.lock();defer {lock.unlock()};return failure}
    func entries(commandID:String?,errorsOnly:Bool,limit:Int)->[CommandLogEntry] {
        lock.lock();defer {lock.unlock()}
        return Array(data.entries.reversed().filter {(commandID==nil || $0.commandID==commandID) && (!errorsOnly || $0.success==false)}.prefix(max(1,min(500,limit))))
    }
    private func prune() {
        data.entries=Array(data.entries.filter {$0.date>=Date().addingTimeInterval(-7*86400)}.suffix(500))
        // A byte budget as well as a count budget, including unusually large error messages.
        func cost(_ entry:CommandLogEntry)->Int {
            let strings=[entry.traceID,entry.commandID,entry.source,entry.phase,entry.code ?? "",entry.message ?? ""]+Array((entry.details ?? [:]).keys)+Array((entry.details ?? [:]).values)
            return 512+strings.reduce(0) {$0+$1.utf8.count*2}
        }
        var bytes=data.entries.reduce(0) {$0+cost($1)}
        while data.entries.count>1,bytes>1_500_000 {bytes-=cost(data.entries.removeFirst())}
    }
    func record(_ entry:CommandLogEntry) {
        lock.lock();data.entries.append(entry);prune();revision+=1
        let enqueue = !scheduled && writable;if enqueue {scheduled=true};lock.unlock()
        if enqueue {writer.asyncAfter(deadline:.now()+0.5) { [weak self] in self?.save() }}
    }
    private func save() {
        lock.lock();scheduled=false
        guard writable,revision != savedRevision else {lock.unlock();return}
        let snapshot=data,version=revision;lock.unlock()
        do {try Self.checkDirectory(url.deletingLastPathComponent());try Files.save(snapshot,to:url);lock.lock();savedRevision=version;failure=nil;lock.unlock()}
        catch {lock.lock();failure="Command log could not be saved. Check disk space and profile permissions.";lock.unlock()}
    }
    func clear() {
        writer.sync {
            lock.lock();data=CommandLogDocument();revision+=1;writable=true;lock.unlock();save()
        }
    }
    func flush() {writer.sync {save()}}
}
