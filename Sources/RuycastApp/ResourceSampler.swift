import Foundation
import Darwin
import IOKit.ps
import LauncherCore
import LauncherModels

struct ProcessResource:Codable,Identifiable {
    let identity:SystemServices.Identity
    let name:String
    let path:String
    let cpuPercent:Double
    let memoryBytes:UInt64
    var id:pid_t {identity.pid}
}
enum ProcessSort:String,CaseIterable {case cpu="CPU Usage",memory="Memory",name="Name"}
struct MonitorSnapshot:Codable {
    var sampledAt=Date()
    var cpuPercent:Double?
    var cpuCores=ProcessInfo.processInfo.processorCount
    var chip=""
    var model=""
    var os=ProcessInfo.processInfo.operatingSystemVersionString
    var hostname=ProcessInfo.processInfo.hostName
    var uptime=ProcessInfo.processInfo.systemUptime
    var processCount=0
    var memoryTotal:UInt64=0
    var memoryUsed:UInt64=0
    var memoryWired:UInt64=0
    var memoryCompressed:UInt64=0
    var swapUsed:UInt64=0
    var diskTotal:UInt64=0
    var diskAvailable:UInt64=0
    var batteryPercent:Int?
    var charging:Bool?
    var powerSource:String?
    var networkReceived:UInt64=0
    var networkSent:UInt64=0
    var downloadBytesPerSecond:Double?
    var uploadBytesPerSecond:Double?
    var interfaces:[String]=[]
}
/// Read-only sampling; called from serial background queues, never during a SwiftUI body.
final class ResourceSampler {
    private var previousCPU:[UInt64]?
    private var previousNetwork:(Date,UInt64,UInt64)?
    static func text(_ name:String)->String {
        var count=0;guard sysctlbyname(name,nil,&count,nil,0)==0,count>0,count<4096 else {return "Unavailable"}
        var bytes=[CChar](repeating:0,count:count);guard sysctlbyname(name,&bytes,&count,nil,0)==0 else {return "Unavailable"};return String(cString:bytes)
    }
    static func value<T>(_ name:String,_ fallback:T)->T {var value=fallback;var size=MemoryLayout<T>.size;guard sysctlbyname(name,&value,&size,nil,0)==0 else {return fallback};return value}
    func sample()->MonitorSnapshot {
        var result=MonitorSnapshot();result.chip=Self.text("machdep.cpu.brand_string");result.model=Self.text("hw.model")
        result.processCount=max(0,Int(proc_listallpids(nil,0)))
        result.memoryTotal=Self.value("hw.memsize",UInt64(0))
        let host=mach_host_self();defer {mach_port_deallocate(mach_task_self_,host)}
        var cpu=host_cpu_load_info(),cpuCount=mach_msg_type_number_t(MemoryLayout<host_cpu_load_info>.size/MemoryLayout<integer_t>.size)
        let cpuStatus=withUnsafeMutablePointer(to:&cpu) {$0.withMemoryRebound(to:integer_t.self,capacity:Int(cpuCount)) {host_statistics(host,HOST_CPU_LOAD_INFO,$0,&cpuCount)}}
        if cpuStatus==KERN_SUCCESS {
            let ticks=withUnsafeBytes(of:cpu.cpu_ticks) {Array($0.bindMemory(to:UInt32.self)).map(UInt64.init)}
            if let previousCPU,previousCPU.count==ticks.count {
                let deltas:[UInt64]=zip(ticks,previousCPU).map {current,previous in
                    if current>=previous {return current-previous}
                    return (UInt64(UInt32.max)-previous)+current+1
                }
                let total=deltas.reduce(0,+)
                if total>0 {result.cpuPercent=100*Double(total-deltas[Int(CPU_STATE_IDLE)])/Double(total)}
            }
            previousCPU=ticks
        }
        var vm=vm_statistics64(),vmCount=mach_msg_type_number_t(MemoryLayout<vm_statistics64>.size/MemoryLayout<integer_t>.size)
        let vmStatus=withUnsafeMutablePointer(to:&vm) {$0.withMemoryRebound(to:integer_t.self,capacity:Int(vmCount)) {host_statistics64(host,HOST_VM_INFO64,$0,&vmCount)}}
        if vmStatus==KERN_SUCCESS {
            let page=UInt64(vm_page_size)
            result.memoryWired=UInt64(vm.wire_count)*page;result.memoryCompressed=UInt64(vm.compressor_page_count)*page
            result.memoryUsed=min(result.memoryTotal,UInt64(vm.active_count)*page+result.memoryWired+result.memoryCompressed)
        }
        result.swapUsed=Self.value("vm.swapusage",xsw_usage()).xsu_used
        var disk=statfs()
        if statfs("/",&disk)==0 {result.diskTotal=UInt64(disk.f_blocks)*UInt64(disk.f_bsize);result.diskAvailable=UInt64(disk.f_bavail)*UInt64(disk.f_bsize)}
        if let info=IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),let sources=IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] {
            for source in sources {
                guard let description=IOPSGetPowerSourceDescription(info,source)?.takeUnretainedValue() as? [String:Any],let current=description[kIOPSCurrentCapacityKey] as? Int,let maximum=description[kIOPSMaxCapacityKey] as? Int,maximum>0 else {continue}
                result.batteryPercent=min(100,max(0,current*100/maximum));result.charging=description[kIOPSIsChargingKey] as? Bool
                result.powerSource=description[kIOPSPowerSourceStateKey] as? String;break
            }
        }
        let network=Self.networkCounters();result.networkReceived=network.received;result.networkSent=network.sent;result.interfaces=network.names
        if let (date,received,sent)=previousNetwork,result.sampledAt>date {
            let seconds=result.sampledAt.timeIntervalSince(date)
            if network.received>=received && network.sent>=sent {result.downloadBytesPerSecond=Double(network.received-received)/seconds;result.uploadBytesPerSecond=Double(network.sent-sent)/seconds}
        }
        previousNetwork=(result.sampledAt,network.received,network.sent)
        return result
    }
    private static func networkCounters()->(received:UInt64,sent:UInt64,names:[String]) {
        var mib:[Int32]=[CTL_NET,PF_ROUTE,0,0,NET_RT_IFLIST2,0],size=0
        guard sysctl(&mib,6,nil,&size,nil,0)==0,size>0,size<2_000_000 else {return (0,0,[])}
        var data=Data(count:size)
        let status=data.withUnsafeMutableBytes {sysctl(&mib,6,$0.baseAddress,&size,nil,0)}
        guard status==0 else {return (0,0,[])}
        var received:UInt64=0,sent:UInt64=0,names=[String](),offset=0
        data.withUnsafeBytes {buffer in
            while offset+MemoryLayout<if_msghdr2>.size<=size {
                let row=buffer.loadUnaligned(fromByteOffset:offset,as:if_msghdr2.self)
                let length=Int(row.ifm_msglen);guard length>0,offset+length<=size else {break}
                if row.ifm_type==RTM_IFINFO2,row.ifm_flags & IFF_UP != 0,row.ifm_flags & IFF_LOOPBACK==0 {
                    var name=[CChar](repeating:0,count:Int(IFNAMSIZ))
                    if if_indextoname(UInt32(row.ifm_index),&name) != nil {
                        let interface=String(cString:name)
                        // Physical en* interfaces avoid counting VPN/tunnel traffic twice.
                        if interface.hasPrefix("en") {received &+= row.ifm_data.ifi_ibytes;sent &+= row.ifm_data.ifi_obytes;names.append(interface)}
                    }
                }
                offset+=length
            }
        }
        return (received,sent,names.sorted())
    }
    static func processTable() throws -> [ProcessResource] {
        let process=Process(),pipe=Pipe();process.executableURL=URL(fileURLWithPath:"/bin/ps")
        process.arguments=["-axo","pid=,pcpu=,rss="];process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
        var env=ProcessInfo.processInfo.environment;env["LC_ALL"]="C";process.environment=env
        do {try process.run()} catch {throw LauncherError("Could not read running processes.")}
        let timeout=DispatchWorkItem {if process.isRunning {process.terminate()}}
        DispatchQueue.global().asyncAfter(deadline:.now()+2,execute:timeout);defer {timeout.cancel()}
        let data=pipe.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit()
        guard process.terminationStatus==0,data.count<2_000_000 else {throw LauncherError("Could not read running processes.")}
        return String(decoding:data,as:UTF8.self).split(whereSeparator:\.isNewline).compactMap {line in
            let fields=line.split(whereSeparator:\.isWhitespace)
            guard fields.count==3,let pid=pid_t(fields[0]),pid>1,pid != getpid(),let cpu=Double(fields[1]),cpu.isFinite,cpu>=0,let rss=UInt64(fields[2]),rss<=UInt64.max/1024,
                let identity=SystemServices.identity(pid),identity.uid==getuid() else {return nil}
            var path=[CChar](repeating:0,count:4096)
            guard proc_pidpath(pid,&path,UInt32(path.count))>0,SystemServices.identity(pid)==identity else {return nil}
            let executable=String(cString:path)
            return ProcessResource(identity:identity,name:URL(fileURLWithPath:executable).lastPathComponent,path:executable,cpuPercent:cpu,memoryBytes:rss*1024)
        }
    }
}
func memoryLabel(_ bytes:UInt64)->String {ByteCountFormatter.string(fromByteCount:Int64(min(UInt64(Int64.max),bytes)),countStyle:.memory)}
