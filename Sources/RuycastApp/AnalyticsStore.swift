import Foundation
import Combine
import CryptoKit
import Darwin
import LauncherCore

struct TimingMetric:Codable {
    var count=0
    var totalMS=0.0
    var maxMS=0.0
    var samples:[Double]=[]
    var p95MS:Double? {let sorted=samples.sorted();return sorted.isEmpty ? nil:sorted[min(sorted.count-1,Int(Double(sorted.count-1)*0.95))]}
    mutating func add(_ ms:Double,limit:Int=256) {guard ms.isFinite,ms>=0 else {return};count+=1;totalMS+=ms;maxMS=max(maxMS,ms);samples.append(ms);if samples.count>limit {samples.removeFirst(samples.count-limit)}}
}
struct CommandActivity:Codable {
    var attempts=0
    var successes=0
    var failures=0
    var priorSuccesses=0
    var lastUsed=Date.distantPast
    var sources:[String:Int]=[:]
    var timing=TimingMetric()
    var uses:Int {attempts+priorSuccesses}
}
struct DiagnosticEvent:Codable,Identifiable {
    var id=UUID().uuidString
    var date=Date()
    var category:String
    var commandID:String?
    var code:String
    var exitStatus:Int?
}
struct ActivityDay:Codable {var uses=0;var successes=0;var failures=0}
struct AnalyticsDocument:Codable {
    var schema=2
    var enabled=true
    var retentionDays=30
    var startedAt=Date()
    var updatedAt=Date()
    var legacyImported=false
    var commands:[String:CommandActivity]=[:]
    var timings:[String:TimingMetric]=[:]
    var events:[DiagnosticEvent]=[]
    var days:[String:ActivityDay]=[:]
    var hours:[String:ActivityDay]=[:]
    var hourlyStartedAt:Date?=Date()
    var totalAttempts=0
    var totalSuccesses=0
    var totalFailures=0
    var priorSuccesses=0
    var omittedCommands=0
    init() {}
    enum CodingKeys:String,CodingKey {
        case schema,enabled,retentionDays,startedAt,updatedAt,legacyImported,commands,timings,events,days,hours,hourlyStartedAt,totalAttempts,totalSuccesses,totalFailures,priorSuccesses,omittedCommands
    }
    init(from decoder:Decoder) throws {
        let c=try decoder.container(keyedBy:CodingKeys.self)
        schema=try c.decode(Int.self,forKey:.schema)
        enabled=try c.decode(Bool.self,forKey:.enabled)
        retentionDays=try c.decode(Int.self,forKey:.retentionDays)
        startedAt=try c.decode(Date.self,forKey:.startedAt)
        updatedAt=try c.decode(Date.self,forKey:.updatedAt)
        legacyImported=try c.decode(Bool.self,forKey:.legacyImported)
        commands=try c.decode([String:CommandActivity].self,forKey:.commands)
        timings=try c.decode([String:TimingMetric].self,forKey:.timings)
        events=try c.decode([DiagnosticEvent].self,forKey:.events)
        days=try c.decode([String:ActivityDay].self,forKey:.days)
        hours=try c.decodeIfPresent([String:ActivityDay].self,forKey:.hours) ?? [:]
        hourlyStartedAt=try c.decodeIfPresent(Date.self,forKey:.hourlyStartedAt)
        totalAttempts=try c.decode(Int.self,forKey:.totalAttempts)
        totalSuccesses=try c.decode(Int.self,forKey:.totalSuccesses)
        totalFailures=try c.decode(Int.self,forKey:.totalFailures)
        priorSuccesses=try c.decode(Int.self,forKey:.priorSuccesses)
        omittedCommands=try c.decode(Int.self,forKey:.omittedCommands)
    }
}
/// UTC epoch hours retain their identity across time-zone changes and repeated DST hours.
struct HourlyActivity:Identifiable {
    let date:Date
    let activity:ActivityDay?
    var id:Date {date}
}
extension AnalyticsDocument {
    func hourlyActivity(now:Date=Date())->[HourlyActivity] {
        let last=AnalyticsStore.hourStart(now)
        return (0..<24).reversed().map {offset in
            let date=last.addingTimeInterval(-Double(offset)*3600)
            let recorded=hourlyStartedAt.map {date.addingTimeInterval(3600)>$0} ?? false
            return HourlyActivity(date:date,activity:recorded ? (hours[AnalyticsStore.hourKey(date)] ?? ActivityDay()):nil)
        }
    }
}
/// Small in-memory counters under one lock. One coalesced writer, never one queued job per event.
/// No arbitrary text, query, input, URL, target, arguments, clipboard body or exception messages.
struct AnalyticsAttempt {let key:String;let epoch:Int;let hour:String}
final class AnalyticsStore:ObservableObject {
    private let lock=NSLock()
    private let writer=DispatchQueue(label:"ruycast.analytics-writer",qos:.utility)
    private let url:URL
    private var data=AnalyticsDocument()
    private var scheduled=false
    private var persistAllowed=true
    private var epoch=0
    private var revision=0
    private var savedRevision=0
    private var failure:String?
    private var lastHourPrune:String?
    private var lastHourRetention=0
    private let saveDelay:Double
    private let now:()->Date
    private let calendar:Calendar
    static let timingNames:Set<String>=["search.model","search.clipboard","launcher.open.model","process.query","clipboard.capture","clipboard.preview","clipboard.save","config.read","config.reload","command.execute","cli.dispatch","monitor.sample","startup.model","process.sort","icons.lookup"]
    init(root:URL,saveDelay:Double=3,now:@escaping()->Date=Date.init,calendar:Calendar=Calendar.current) {
        url=root.appendingPathComponent("analytics.json");self.saveDelay=saveDelay;self.now=now;self.calendar=calendar
        data.startedAt=now();data.updatedAt=now();data.hourlyStartedAt=now()
        if FileManager.default.fileExists(atPath:url.path) {
            do {
                let fd=open(url.path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);guard fd>=0 else {throw LauncherError("Analytics unavailable")};defer {close(fd)}
                var info=stat();guard fstat(fd,&info)==0,(info.st_mode & S_IFMT)==S_IFREG,info.st_uid==getuid(),info.st_nlink==1,info.st_size>0,info.st_size<=8_000_000 else {throw LauncherError("Invalid analytics file")}
                let bytes=try FileHandle(fileDescriptor:fd,closeOnDealloc:false).read(upToCount:8_000_001) ?? Data()
                guard bytes.count<=8_000_000 else {throw LauncherError("Analytics size limit")}
                data=try Files.decode(AnalyticsDocument.self,data:bytes)
                guard [1,2].contains(data.schema),(data.schema==1 || data.hourlyStartedAt != nil),(1...90).contains(data.retentionDays),data.commands.count<=2048,data.timings.count<=Self.timingNames.count,data.events.count<=200,data.days.count<=91,data.hours.count<=2161,
                    data.timings.allSatisfy({Self.timingNames.contains($0.key) && $0.value.samples.count<=256 && $0.value.samples.allSatisfy {$0.isFinite && $0>=0}}),data.commands.values.allSatisfy({$0.timing.samples.count<=32}),Self.valid(data) else {throw LauncherError("Invalid analytics bounds")}
                data.schema=2
                if data.hourlyStartedAt==nil {data.hourlyStartedAt=now()}
                prune()
            } catch {data=AnalyticsDocument();persistAllowed=false;failure="Analytics data could not be read. Reset analytics to start again."}
        }
    }
    static func commandKey(_ id:String)->String {SHA256.hash(data:Data(("ruycast.command:"+id).utf8)).map {String(format:"%02x",$0)}.joined()}
    private static func validKey(_ value:String)->Bool {value.utf8.count==64 && value.utf8.allSatisfy {(48...57).contains($0) || (97...102).contains($0)}}
    private static func valid(_ d:AnalyticsDocument)->Bool {
        func count(_ n:Int)->Bool {(0...1_000_000_000_000).contains(n)}
        func timing(_ m:TimingMetric)->Bool {count(m.count) && m.totalMS.isFinite && m.totalMS>=0 && m.maxMS.isFinite && m.maxMS>=0 && m.samples.allSatisfy {$0.isFinite && $0>=0}}
        return d.hours.allSatisfy {key,row in
            guard let seconds=Int64(key),seconds>=0,seconds<=253402300800,seconds%3600==0,key==String(seconds) else {return false}
            return [row.uses,row.successes,row.failures].allSatisfy(count)
        } && (d.hourlyStartedAt?.timeIntervalSince1970.isFinite ?? true) && [d.totalAttempts,d.totalSuccesses,d.totalFailures,d.priorSuccesses,d.omittedCommands].allSatisfy(count) && d.timings.values.allSatisfy(timing) && d.commands.allSatisfy {id,row in
            validKey(id) && [row.attempts,row.successes,row.failures,row.priorSuccesses].allSatisfy(count) && timing(row.timing) && row.sources.allSatisfy {["launcher","hotkey","cli","snippet"].contains($0.key) && count($0.value)}
        } && d.events.allSatisfy {["command","config","clipboard","keyboard","runtime"].contains($0.category) && safeCode($0.code)==$0.code && ($0.commandID.map(validKey) ?? true) && ($0.exitStatus.map {(-1...255).contains($0)} ?? true)} && d.days.allSatisfy {$0.key.count==10 && [$0.value.uses,$0.value.successes,$0.value.failures].allSatisfy(count)}
    }
    var snapshot:AnalyticsDocument {lock.lock();defer {lock.unlock()};return data}
    var storageError:String? {lock.lock();defer {lock.unlock()};return failure}
    static func elapsed(_ started:UInt64)->Double {Double(DispatchTime.now().uptimeNanoseconds-started)/1_000_000}
    static func hourStart(_ date:Date)->Date {Date(timeIntervalSince1970:floor(date.timeIntervalSince1970/3600)*3600)}
    static func hourKey(_ date:Date)->String {String(Int64(hourStart(date).timeIntervalSince1970))}
    private func dayKey(_ date:Date)->String {let c=calendar.dateComponents([.year,.month,.day],from:date);return String(format:"%04d-%02d-%02d",c.year!,c.month!,c.day!)}
    private func prune() {
        let cutoff=calendar.date(byAdding:.day,value:-data.retentionDays,to:now()) ?? .distantPast
        data.events=Array(data.events.filter {$0.date>=cutoff}.suffix(200))
        let key=dayKey(cutoff);data.days=data.days.filter {$0.key>=key}
        let date=now(),hour=Self.hourKey(date)
        if lastHourPrune != hour || lastHourRetention != data.retentionDays {
            let first=Self.hourStart(date.addingTimeInterval(-Double(data.retentionDays)*86400)).timeIntervalSince1970,last=Self.hourStart(date).timeIntervalSince1970
            data.hours=data.hours.filter {guard let seconds=Double($0.key) else {return false};return seconds>=first && seconds<=last}
            lastHourPrune=hour;lastHourRetention=data.retentionDays
        }
    }
    @discardableResult private func mutate(_ action:(inout AnalyticsDocument)->Void,evenWhenPaused:Bool=false)->Bool {
        lock.lock()
        guard data.enabled || evenWhenPaused else {lock.unlock();return false}
        action(&data);revision+=1;data.updatedAt=now();prune()
        let schedule = !scheduled && persistAllowed
        if schedule {scheduled=true};lock.unlock()
        if schedule {writer.asyncAfter(deadline:.now()+saveDelay) { [weak self] in self?.save() }}
        return true
    }
    func importLegacy(_ counts:[String:Int]) {
        mutate({d in guard !d.legacyImported else {return};d.legacyImported=true
            d.priorSuccesses=counts.values.reduce(0) {min(1_000_000_000_000,$0+min(max(0,$1),1_000_000_000))}
            var represented=0
            for (id,count) in counts.sorted(by:{$0.value>$1.value}).prefix(2048) where count>0 {let key=Self.commandKey(id);var row=d.commands[key] ?? CommandActivity();row.priorSuccesses=min(count,1_000_000_000);d.commands[key]=row;represented+=row.priorSuccesses}
            d.omittedCommands+=max(0,d.priorSuccesses-represented)
        },evenWhenPaused:true)
    }
    func recordTiming(_ name:String,ms:Double) {guard Self.timingNames.contains(name),ms.isFinite,ms>=0 else {return};mutate {$0.timings[name,default:TimingMetric()].add(ms)}}
    @discardableResult func recordAttempt(_ id:String,source:String)->AnalyticsAttempt? {
        let key=Self.commandKey(id),source=["launcher","hotkey","cli","snippet"].contains(source) ? source:"cli"
        var ticket:AnalyticsAttempt?
        mutate {d in let date=self.now(),hour=Self.hourKey(date);ticket=AnalyticsAttempt(key:key,epoch:self.epoch,hour:hour);d.totalAttempts+=1;let day=self.dayKey(date);d.days[day,default:ActivityDay()].uses+=1;d.hours[hour,default:ActivityDay()].uses+=1
            guard d.commands[key] != nil || d.commands.count<2048 else {d.omittedCommands+=1;return}
            var row=d.commands[key] ?? CommandActivity();row.attempts+=1;row.lastUsed=date;row.sources[source,default:0]+=1;d.commands[key]=row
        }
        return ticket
    }
    func recordCompletion(_ ticket:AnalyticsAttempt,success:Bool,ms:Double,code:String="action-failed",exitStatus:Int?=nil) {
        mutate({d in
            guard ticket.epoch==self.epoch else {return}
            if success {d.totalSuccesses+=1;d.days[self.dayKey(self.now()),default:ActivityDay()].successes+=1} else {d.totalFailures+=1;d.days[self.dayKey(self.now()),default:ActivityDay()].failures+=1}
            if var hour=d.hours[ticket.hour] {if success {hour.successes+=1} else {hour.failures+=1};d.hours[ticket.hour]=hour}
            if var row=d.commands[ticket.key] {if success {row.successes+=1} else {row.failures+=1};row.timing.add(ms,limit:32);d.commands[ticket.key]=row}
            d.timings["command.execute",default:TimingMetric()].add(ms)
            if !success {d.events.append(DiagnosticEvent(date:self.now(),category:"command",commandID:ticket.key,code:Self.safeCode(code),exitStatus:exitStatus.flatMap {(-1...255).contains($0) ? $0:nil}))}
        },evenWhenPaused:true)
    }
    static func safeCode(_ value:String)->String {["action-failed","permission","timeout","unavailable","invalid","io","config-invalid","clipboard-io","interrupted","script-exit"].contains(value) ? value:"action-failed"}
    static func failureCode(_ message:String)->String {
        let text=message.lowercased()
        if text.contains("permission") || text.contains("accessibility") || text.contains("not permitted") {return "permission"}
        if text.contains("script exited") {return "script-exit"}
        if text.contains("timed out") || text.contains("timeout") {return "timeout"}
        if text.contains("unavailable") || text.contains("not found") || text.contains("missing") {return "unavailable"}
        if text.contains("invalid") || text.contains("disabled") {return "invalid"}
        return "action-failed"
    }
    func recordError(_ category:String,code:String) {guard ["config","clipboard","keyboard","runtime"].contains(category) else {return};mutate {$0.events.append(DiagnosticEvent(date:self.now(),category:category,code:Self.safeCode(code)))}}
    func configure(enabled:Bool?,retentionDays:Int?) throws {
        if let retentionDays,!(1...90).contains(retentionDays) {throw LauncherError("Choose 1–90 days.")}
        mutate({d in if let enabled {d.enabled=enabled};if let retentionDays {d.retentionDays=retentionDays}},evenWhenPaused:true)
    }
    func reset() {
        lock.lock();epoch+=1;let enabled=data.enabled,days=data.retentionDays;data=AnalyticsDocument();data.startedAt=now();data.updatedAt=now();data.hourlyStartedAt=now();data.enabled=enabled;data.retentionDays=days;data.legacyImported=true;persistAllowed=true;failure=nil;lock.unlock()
        mutate({_ in},evenWhenPaused:true)
    }
    func exportSummary()->[String:Any] {
        let d=snapshot
        return ["schema":1,"version":"0.7.14","uses":d.totalAttempts+d.priorSuccesses,"attempts":d.totalAttempts,"successes":d.totalSuccesses,"failures":d.totalFailures,"legacySuccesses":d.priorSuccesses,"timings":jsonValue(d.timings),"errors":Dictionary(grouping:d.events,by:{$0.category+":"+$0.code}).mapValues(\.count),"privacy":"No command IDs, input, queries, URLs, arguments or clipboard contents"]
    }
    private func save() {
        lock.lock();scheduled=false
        guard persistAllowed,revision != savedRevision else {lock.unlock();return}
        let snapshot=data,version=revision;lock.unlock()
        do {
            let encoded=try Files.encode(snapshot);guard encoded.count<=8_000_000 else {throw LauncherError("Analytics storage limit")}
            try FileManager.default.createDirectory(at:url.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
            try encoded.write(to:url,options:.atomic);try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:url.path)
            lock.lock();savedRevision=version;failure=nil;lock.unlock()
        } catch {lock.lock();failure="Analytics could not be saved. Check available disk space and folder permissions.";lock.unlock()}
        DispatchQueue.main.async { [weak self] in self?.objectWillChange.send() }
    }
    func flush(_ completion:@escaping()->Void={}) {writer.async { [weak self] in self?.save();DispatchQueue.main.async(execute:completion)}}
    func flushSynchronously() {writer.sync {save()}}
}
