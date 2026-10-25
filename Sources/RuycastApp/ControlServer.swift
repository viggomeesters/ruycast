import Foundation
import Darwin
import LauncherCore

final class ControlServer {
    private var fd:Int32 = -1
    private var lockFD:Int32 = -1
    private let clients=DispatchSemaphore(value:16)
    private let socketURL:URL
    private var handler:(([String:Any],@escaping ([String:Any])->Void)->Void)?
    init(root:URL) { socketURL=root.appendingPathComponent("control.sock") }
    func start(handler:@escaping ([String:Any],@escaping ([String:Any])->Void)->Void) throws {
        let root=socketURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        try FileManager.default.setAttributes([.posixPermissions:0o700],ofItemAtPath:root.path)
        lockFD=open(root.appendingPathComponent("instance.lock").path,O_CREAT|O_RDWR,0o600)
        guard lockFD>=0,flock(lockFD,LOCK_EX|LOCK_NB)==0 else { throw LauncherError("Ruycast is already running for this profile.") }
        self.handler=handler
        fd=socket(AF_UNIX,SOCK_STREAM,0)
        guard fd>=0 else { throw LauncherError("Cannot create control socket.") }
        var address=sockaddr_un();address.sun_family=sa_family_t(AF_UNIX)
        let bytes=Array(socketURL.path.utf8CString)
        guard bytes.count<=MemoryLayout.size(ofValue:address.sun_path) else { throw LauncherError("Profile path is too long for a Unix socket.") }
        withUnsafeMutableBytes(of:&address.sun_path) { raw in raw.copyBytes(from:bytes.map{UInt8(bitPattern:$0)}) }
        unlink(socketURL.path)
        let status=withUnsafePointer(to:&address) { p in p.withMemoryRebound(to:sockaddr.self,capacity:1) { Darwin.bind(fd,$0,socklen_t(MemoryLayout<sockaddr_un>.size)) } }
        guard status==0,listen(fd,16)==0 else { throw LauncherError("Cannot bind control socket: \(String(cString:strerror(errno))).") }
        chmod(socketURL.path,0o600)
        let descriptor=fd
        DispatchQueue.global(qos:.userInitiated).async { [weak self] in
            while let self {
                let client=accept(descriptor,nil,nil)
                if client<0 {break}
                guard self.clients.wait(timeout:.now()) == .success else {Darwin.close(client);continue}
                DispatchQueue.global(qos:.userInitiated).async {defer {self.clients.signal()};autoreleasepool {self.receive(client)}}
            }
        }
    }
    private func receive(_ client:Int32) {
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
        let response=ResponseBox()
        DispatchQueue.main.async { [weak self] in
            self?.handler?(request) { result in response.value=result;done.signal() }
        }
        guard done.wait(timeout:.now()+310) == .success else {send(["ok":false,"error":"Request timed out."],client);return}
        send(response.value,client)
    }
    private func send(_ result:[String:Any],_ client:Int32) {
        guard var data=try? JSONSerialization.data(withJSONObject:result,options:[.sortedKeys,.fragmentsAllowed]) else {return}
        data.append(10)
        data.withUnsafeBytes { raw in
            var offset=0
            while offset<raw.count {let n=write(client,raw.baseAddress!.advanced(by:offset),raw.count-offset);if n<=0 {break};offset+=n}
        }
    }
    func stop() {
        if fd>=0 {
            let descriptor=fd;fd = -1
            shutdown(descriptor,SHUT_RDWR)
            Darwin.close(descriptor)
            unlink(socketURL.path)
        }
        if lockFD>=0 {Darwin.close(lockFD);lockFD = -1}
        handler=nil
    }
    deinit {stop()}
}
private final class ResponseBox {
    private let lock=NSLock();private var stored:[String:Any]=[:]
    var value:[String:Any] {get {lock.lock();defer {lock.unlock()};return stored} set {lock.lock();stored=newValue;lock.unlock()}}
}
