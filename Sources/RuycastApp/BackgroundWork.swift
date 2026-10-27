import Foundation
import Darwin
import LauncherCore
import LauncherModels

/// Serializes history/asset writes and invalidates stale capture publications.
// Version is locked; work executes on the serial queue. No unlocked mutable state.
final class ClipboardWork:Sendable {
    let queue=DispatchQueue(label:"ruycast.clipboard-work",qos:.utility,autoreleaseFrequency:.workItem)
    private let version=LockedValue(0)
    var revision:Int {version.read {$0}}
    func invalidate() {version.update {$0+=1}}
    func current(_ candidate:Int)->Bool {revision==candidate}
    func perform(_ work:@escaping @Sendable ()->Void) {queue.async(execute:work)}
    func sync<T:Sendable>(_ work:@Sendable ()throws->T) rethrows->T {let result=try queue.sync(execute:work);invalidate();return result}
    func drain() {queue.sync {}}
}

/// Watches the directory so atomic replacements do not detach the observer.
/// Coalesces events, reads bytes only when the config file's identity/metadata changed.
struct ObservedConfiguration:Sendable {let bytes:Data;let config:Configuration}
// Fingerprint/bytes/readCount/pending are worker-confined; delivery is locked.
// source/fallback are created before publication and cancelled only at teardown.
final class ConfigurationObserver:@unchecked Sendable {
    private let worker=DispatchQueue(label:"ruycast.config-observer",qos:.utility,autoreleaseFrequency:.workItem)
    private var source:DispatchSourceFileSystemObject?
    private var fallback:DispatchSourceTimer?
    private let lock=NSLock()
    private var delivery=0
    private var pending=false
    private var fingerprint:String?
    private var lastBytes:Data?
    private let root:URL
    private let callback:@MainActor @Sendable (Result<ObservedConfiguration,Error>)->Void
    private var readCount=0
    init(root:URL,initial:Data,callback:@escaping @MainActor @Sendable (Result<ObservedConfiguration,Error>)->Void) {
        self.root=root;self.callback=callback;self.lastBytes=initial;fingerprint=Self.stamp(root.appendingPathComponent("config.json"))
        let fd=open(root.path,O_EVTONLY|O_CLOEXEC)
        if fd>=0 {
            let source=DispatchSource.makeFileSystemObjectSource(fileDescriptor:fd,eventMask:[.write,.rename,.delete,.attrib,.revoke],queue:worker)
            source.setEventHandler { [weak self] in self?.schedule() };source.setCancelHandler {close(fd)};self.source=source;source.resume()
        }
        let timer=DispatchSource.makeTimerSource(queue:worker);timer.schedule(deadline:.now()+5,repeating:5,leeway:.milliseconds(500));timer.setEventHandler { [weak self] in self?.check() };timer.resume();fallback=timer
    }
    private static func stamp(_ url:URL)->String? {
        var s=stat();guard lstat(url.path,&s)==0,(s.st_mode & S_IFMT)==S_IFREG else {return nil}
        return "\(s.st_dev):\(s.st_ino):\(s.st_size):\(s.st_mtimespec.tv_sec):\(s.st_mtimespec.tv_nsec):\(s.st_ctimespec.tv_sec):\(s.st_ctimespec.tv_nsec)"
    }
    private func schedule() {guard !pending else {return};pending=true;worker.asyncAfter(deadline:.now()+0.08) { [weak self] in self?.pending=false;self?.check()}}
    func requestCheck() {worker.async { [weak self] in self?.schedule() }}
    private func check() {
        let url=root.appendingPathComponent("config.json")
        guard let stamp=Self.stamp(url),stamp != fingerprint else {return}
        let result=Result { () throws -> ObservedConfiguration in
            let fd=open(url.path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC|O_NONBLOCK);guard fd>=0 else {throw LauncherError("Configuration is unavailable.")};defer {close(fd)}
            var info=stat();guard fstat(fd,&info)==0,(info.st_mode & S_IFMT)==S_IFREG,info.st_size>=0,info.st_size<=32_000_000 else {throw LauncherError("Configuration exceeds the supported size.")}
            let bytes=try FileHandle(fileDescriptor:fd,closeOnDealloc:false).read(upToCount:32_000_001) ?? Data()
            guard bytes.count<=32_000_000 else {throw LauncherError("Configuration exceeds the supported size.")}
            return ObservedConfiguration(bytes:bytes,config:try Files.config(data:bytes))
        }
        guard Self.stamp(url)==stamp else {schedule();return}
        fingerprint=stamp;readCount+=1
        switch result {case .success(let observed):guard observed.bytes != lastBytes else {return};lastBytes=observed.bytes;case .failure:lastBytes=nil}
        lock.lock();delivery+=1;let token=delivery;lock.unlock()
        DispatchQueue.main.async { [weak self] in guard let self else {return};self.lock.lock();let current=self.delivery==token;self.lock.unlock();if current {self.callback(result)} }
    }
    func acknowledge(_ bytes:Data) {
        lock.lock();delivery+=1;lock.unlock()
        worker.async { [weak self] in guard let self else {return};self.lastBytes=bytes;self.fingerprint=nil;self.schedule()}
    }
    func reads(_ completion:@escaping @MainActor @Sendable (Int)->Void) {worker.async { [weak self] in let count=self?.readCount ?? 0;DispatchQueue.main.async {completion(count)}}}
    deinit {source?.cancel();fallback?.cancel()}
}
