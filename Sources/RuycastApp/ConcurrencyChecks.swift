import Foundation
import LauncherModels

@MainActor func checkConcurrencyOwnership()throws->[String:Any] {
    func require(_ value:Bool,_ message:String)throws {if !value {throw LauncherError(message)}}
    func wait(_ condition:()->Bool)throws {
        let end=Date().addingTimeInterval(4)
        while !condition(),Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.002))}
        try require(condition(),"Concurrency fixture timed out")
    }
    let count=LockedValue(0)
    DispatchQueue.concurrentPerform(iterations:1000) {_ in count.update {$0+=1}}
    try require(count.read {$0}==1000,"Concurrent updates lost state")

    let queue=DispatchQueue(label:"ruycast.concurrency-fixture")
    let session=SystemSearchSession(),independent=SystemSearchSession()
    let started=DispatchSemaphore(value:0),release=DispatchSemaphore(value:0)
    let order=LockedValue<[Int]>([])
    session.submit(on:queue) {_ in
        order.update {$0.append(0)};started.signal();_=release.wait(timeout:.now()+3)
    }
    try require(started.wait(timeout:.now()+3) == .success,"Worker did not start")
    for index in 1...1000 {session.submit(on:queue) {ticket in if ticket.current {order.update {$0.append(index)}}}}
    try require(session.pendingCount==1,"Typing queued more than the latest request")
    independent.submit(on:queue) {_ in order.update {$0.append(2000)}}
    release.signal()
    try wait {order.read {$0.count}==3}
    try require(order.read {$0}==[0,2000,1000],"Latest work starved another consumer or stale work ran")

    let service=SystemServices()
    var wireSearch=false
    service.results(mode:"files",query:"",previousPID:nil,session:SystemSearchSession()) {result in
        if case .success(let rows)=result {wireSearch=rows.isEmpty}
    }
    service.cancel()
    try wait {wireSearch}

    var uiReply=false
    ReplySink(ui:{value in uiReply=value["ok"] as? Bool==true}).send(["ok":true])
    try require(uiReply,"Main-actor intent reply became asynchronous")
    let bytes=LockedValue((data:Data(),onMain:true)),done=DispatchSemaphore(value:0)
    let sink=ReplySink(wire:{data in bytes.update {$0=(data,Thread.isMainThread)};done.signal()})
    queue.async {sink.sendFromWorker(["ok":true,"result":Array(repeating:String(repeating:"x",count:8192),count:200)])}
    try require(done.wait(timeout:.now()+3) == .success,"Worker reply was not delivered")
    let result=bytes.read {$0}
    let decoded=try JSONSerialization.jsonObject(with:result.data) as? [String:Any]
    try require(!result.onMain && (decoded?["result"] as? [String])?.count==200,"Bulk wire reply moved to main or lost data")
    return ["ok":true,"concurrentLockedUpdates":1000,"replacedUIRequests":1000,"maxPendingUIRequests":1,"independentConsumersFair":true,"uiCancellationPreservesCLIRequest":true,"mainIntentReplySynchronous":true,"bulkWireReplyEncodedOffMain":true,"desktopEffects":false]
}
