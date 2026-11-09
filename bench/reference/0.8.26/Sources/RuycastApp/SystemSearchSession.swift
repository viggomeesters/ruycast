import Foundation

/// One in-flight request and one replaceable latest request per consumer.
/// Each turn rejoins the serial queue so sustained typing cannot starve CLI work.
final class SystemSearchSession:Sendable {
    struct Ticket:Sendable {
        let session:SystemSearchSession
        let revision:Int
        var current:Bool {session.state.read {$0.revision==revision}}
        func launch(_ process:Process)throws->Bool {
            try session.state.update {value in
                guard value.revision==revision else {return false}
                value.process=process
                do {try process.run();return true}
                catch {value.process=nil;throw error}
            }
        }
        func detach(_ process:Process) {
            session.state.update {if $0.process===process {$0.process=nil}}
        }
    }
    private struct State:Sendable {
        var revision=0
        var running=false
        var pending:(Int,@Sendable (Ticket)->Void)?
        var process:Process?
    }
    private let state=LockedValue(State())
    var pendingCount:Int {state.read {$0.pending==nil ? 0:1}}
    func cancel() {
        let process=state.update {value in
            value.revision+=1;value.pending=nil
            let process=value.process;value.process=nil;return process
        }
        if let process,process.isRunning {process.terminate()}
    }
    func submit(on queue:DispatchQueue,work:@escaping @Sendable (Ticket)->Void) {
        let (start,process)=state.update {value in
            value.revision+=1;value.pending=(value.revision,work)
            let process=value.process;value.process=nil
            let start = !value.running;value.running=true;return (start,process)
        }
        if let process,process.isRunning {process.terminate()}
        if start {queue.async {self.turn(on:queue)}}
    }
    private func turn(on queue:DispatchQueue) {
        let next=state.update {value in
            let next=value.pending;value.pending=nil
            if next==nil {value.running=false}
            return next
        }
        guard let (revision,work)=next else {return}
        let ticket=Ticket(session:self,revision:revision)
        if ticket.current {work(ticket)}
        let again=state.update {value in
            if value.pending != nil {return true}
            value.running=false;return false
        }
        if again {queue.async {self.turn(on:queue)}}
    }
}
