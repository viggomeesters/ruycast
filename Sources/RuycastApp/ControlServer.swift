import Foundation
import Darwin
import LauncherCore
import LauncherModels

@MainActor final class ControlServer {
    typealias Handler = @MainActor @Sendable ([String:Any],ReplySink)->Void
    private var lockFD:Int32 = -1
    private var listener:SocketListener?
    private struct SocketIdentity:Equatable {let device:dev_t;let inode:ino_t}
    private var ownedSocket:SocketIdentity?
    private let clients=DispatchSemaphore(value:16)
    private let socketURL:URL
    init(root:URL) { socketURL=root.appendingPathComponent("control.sock") }
    func start(handler:@escaping Handler) throws {
        guard listener==nil,lockFD<0 else {throw LauncherError("Control server is already started.")}
        let root=socketURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        try FileManager.default.setAttributes([.posixPermissions:0o700],ofItemAtPath:root.path)
        let lockFD=open(root.appendingPathComponent("instance.lock").path,O_CREAT|O_RDWR|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC,0o600)
        guard lockFD>=0 else {throw LauncherError("Cannot open instance lock.")}
        var committed=false
        defer {if !committed {Darwin.close(lockFD)}}
        var lockInfo=stat()
        guard fstat(lockFD,&lockInfo)==0,(lockInfo.st_mode & S_IFMT)==S_IFREG,lockInfo.st_uid==getuid() else {throw LauncherError("Instance lock must be an owned regular file.")}
        guard flock(lockFD,LOCK_EX|LOCK_NB)==0 else {throw LauncherError("Ruycast is already running for this profile.")}
        let fd=socket(AF_UNIX,SOCK_STREAM,0)
        guard fd>=0 else { throw LauncherError("Cannot create control socket.") }
        var boundIdentity:SocketIdentity?
        defer {if !committed {Darwin.close(fd);if let boundIdentity {removeSocket(matching:boundIdentity)}}}
        var address=sockaddr_un();address.sun_family=sa_family_t(AF_UNIX)
        let bytes=Array(socketURL.path.utf8CString)
        guard bytes.count<=MemoryLayout.size(ofValue:address.sun_path) else { throw LauncherError("Profile path is too long for a Unix socket.") }
        withUnsafeMutableBytes(of:&address.sun_path) { raw in raw.copyBytes(from:bytes.map{UInt8(bitPattern:$0)}) }
        try removeStaleSocket(address:&address)
        let status=withUnsafePointer(to:&address) { p in p.withMemoryRebound(to:sockaddr.self,capacity:1) { Darwin.bind(fd,$0,socklen_t(MemoryLayout<sockaddr_un>.size)) } }
        guard status==0 else {throw LauncherError("Cannot bind control socket: \(String(cString:strerror(errno))).")}
        guard let identity=socketIdentityAtPath() else {throw LauncherError("Control socket changed during startup.")}
        boundIdentity=identity
        guard listen(fd,16)==0 else {throw LauncherError("Cannot listen on control socket: \(String(cString:strerror(errno))).")}
        guard socketIdentityAtPath()==identity,fchmodat(AT_FDCWD,socketURL.path,0o600,AT_SYMLINK_NOFOLLOW)==0 else {throw LauncherError("Cannot secure control socket.")}
        let listener=SocketListener(fd),clients=clients
        self.listener=listener;self.lockFD=lockFD;ownedSocket=identity;committed=true
        let descriptor=listener.descriptor
        DispatchQueue.global(qos:.userInitiated).async {
            while listener.running {
                let client=accept(descriptor,nil,nil)
                if client<0 {break}
                guard listener.running else {Darwin.close(client);break}
                guard clients.wait(timeout:.now()) == .success else {Darwin.close(client);continue}
                DispatchQueue.global(qos:.userInitiated).async {defer {clients.signal()};autoreleasepool {Self.receive(client,handler:handler)}}
            }
        }
    }
    private func socketIdentityAtPath()->SocketIdentity? {
        var info=stat()
        guard lstat(socketURL.path,&info)==0,(info.st_mode & S_IFMT)==S_IFSOCK,info.st_uid==getuid() else {return nil}
        return SocketIdentity(device:info.st_dev,inode:info.st_ino)
    }
    private func removeSocket(matching identity:SocketIdentity) {
        // Preserve paths replaced after startup. The profile lock coordinates
        // Ruycast peers; this is not an atomic guard against hostile same-UID edits.
        if socketIdentityAtPath()==identity {unlink(socketURL.path)}
    }
    private func removeStaleSocket(address:inout sockaddr_un) throws {
        guard let identity=socketIdentityAtPath() else {return} // bind preserves non-socket paths.
        let probe=socket(AF_UNIX,SOCK_STREAM,0)
        guard probe>=0 else {throw LauncherError("Cannot inspect existing control socket.")}
        defer {Darwin.close(probe)}
        let flags=fcntl(probe,F_GETFL)
        guard flags>=0,fcntl(probe,F_SETFL,flags|O_NONBLOCK)==0 else {throw LauncherError("Cannot inspect existing control socket.")}
        let status=withUnsafePointer(to:&address) {p in p.withMemoryRebound(to:sockaddr.self,capacity:1) {Darwin.connect(probe,$0,socklen_t(MemoryLayout<sockaddr_un>.size))}}
        let code=errno
        guard status<0,code==ECONNREFUSED || code==ENOENT else {throw LauncherError("Control socket is active or cannot be verified stale.")}
        removeSocket(matching:identity)
    }
    private nonisolated static func receive(_ client:Int32,handler:@escaping Handler) {
        defer {Darwin.close(client)}
        var timeout=timeval(tv_sec:3,tv_usec:0)
        setsockopt(client,SOL_SOCKET,SO_RCVTIMEO,&timeout,socklen_t(MemoryLayout.size(ofValue:timeout)))
        setsockopt(client,SOL_SOCKET,SO_SNDTIMEO,&timeout,socklen_t(MemoryLayout.size(ofValue:timeout)))
        var noSignal:Int32=1;setsockopt(client,SOL_SOCKET,SO_NOSIGPIPE,&noSignal,socklen_t(MemoryLayout.size(ofValue:noSignal)))
        var data=Data();var buffer=[UInt8](repeating:0,count:4096)
        while data.count<1_048_576 {
            let count=read(client,&buffer,buffer.count)
            if count<=0 {return}
            data.append(contentsOf:buffer.prefix(count))
            if data.contains(10) {break}
        }
        let request:[String:Any]
        do {
            guard data.count<1_048_576,data.last==10,let obj=try JSONSerialization.jsonObject(with:data) as? [String:Any] else {throw LauncherError("Expected one JSON object followed by newline (max 1 MiB).")};request=obj
        } catch {send(["ok":false,"error":error.localizedDescription],client);return}
        let done=DispatchSemaphore(value:0)
        let response=LockedValue(Data())
        let sink=ReplySink(wire:{data in response.update {$0=data};done.signal()})
        DispatchQueue.main.async {handler(request,sink)}
        guard done.wait(timeout:.now()+310) == .success else {send(["ok":false,"error":"Request timed out."],client);return}
        sendData(response.read {$0},client)
    }
    private nonisolated static func send(_ result:[String:Any],_ client:Int32) {
        guard let data=try? JSONSerialization.data(withJSONObject:result,options:[.sortedKeys,.fragmentsAllowed]) else {return}
        sendData(data,client)
    }
    private nonisolated static func sendData(_ body:Data,_ client:Int32) {
        var data=body
        data.append(10)
        data.withUnsafeBytes { raw in
            var offset=0
            while offset<raw.count {let n=write(client,raw.baseAddress!.advanced(by:offset),raw.count-offset);if n<=0 {break};offset+=n}
        }
    }
    func stop() {
        if let listener {listener.stop();self.listener=nil;if let ownedSocket {removeSocket(matching:ownedSocket)};ownedSocket=nil}
        if lockFD>=0 {Darwin.close(lockFD);lockFD = -1}
    }
    isolated deinit {stop()}
}

/// Shutdown wakes accept; the descriptor closes only after its worker releases
/// the lease, so a stopped listener cannot accidentally accept on a reused fd.
private final class SocketListener:Sendable {
    let descriptor:Int32
    private let active=LockedValue(true)
    init(_ descriptor:Int32) {self.descriptor=descriptor}
    var running:Bool {active.read {$0}}
    func stop() {
        let wasActive=active.update {let old=$0;$0=false;return old}
        if wasActive {shutdown(descriptor,SHUT_RDWR)}
    }
    deinit {Darwin.close(descriptor)}
}
