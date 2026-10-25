import Foundation
import LauncherCore

/// At most one running filter and one latest request. Never queues one job per keystroke.
final class ClipboardSearch {
    private let queue=DispatchQueue(label:"ruycast.clipboard-search",qos:.userInitiated,autoreleaseFrequency:.workItem)
    private let lock=NSLock()
    private var version=0
    private var running=false
    private var pending:(Int,[Clip],String,([Clip])->Void)?
    func cancel() {lock.lock();version+=1;pending=nil;lock.unlock()}
    func search(_ items:[Clip],query:String,completion:@escaping([Clip])->Void) {
        lock.lock();version+=1;pending=(version,items,query,completion);let start = !running;if start {running=true};lock.unlock()
        if start {queue.async { [weak self] in self?.run() }}
    }
    private func current(_ token:Int)->Bool {lock.lock();defer {lock.unlock()};return token==version}
    private func run() {
        while true {
            lock.lock();guard let (token,items,query,completion)=pending else {running=false;lock.unlock();return};pending=nil;lock.unlock()
            var result=[Clip]()
            for item in items {guard current(token) else {break};if item.text.localizedCaseInsensitiveContains(query) {result.append(item)}}
            if current(token) {DispatchQueue.main.async { [weak self] in if self?.current(token)==true {completion(result)}}}
        }
    }
}
