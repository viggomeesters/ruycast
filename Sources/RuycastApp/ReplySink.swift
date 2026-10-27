import Foundation

/// Wire replies cross the worker boundary as owned bytes. UI callbacks retain
/// their main-actor contract, including immediate replies from main-thread intents.
struct ReplySink:Sendable {
    private let wire:(@Sendable (Data)->Void)?
    private let ui:(@MainActor @Sendable ([String:Any])->Void)?
    init(wire:@escaping @Sendable (Data)->Void) {self.wire=wire;ui=nil}
    init(ui:@escaping @MainActor @Sendable ([String:Any])->Void) {self.ui=ui;wire=nil}
    @MainActor func send(_ object:[String:Any]) {
        if let ui {ui(object)}
        else {sendFromWorker(object)}
    }
    nonisolated func sendFromWorker(_ object:[String:Any]) {
        // Bulk clipboard replies stay on their worker through encoding; the wire
        // path transfers only owned bytes and never decodes them on the main actor.
        let data=(try? JSONSerialization.data(withJSONObject:object,options:[.sortedKeys,.fragmentsAllowed])) ?? Data("{\"ok\":false,\"error\":\"Invalid response\"}".utf8)
        if let wire {wire(data)}
        else if let ui {DispatchQueue.main.async {
            let value=(try? JSONSerialization.jsonObject(with:data)) as? [String:Any] ?? ["ok":false,"error":"Invalid response"]
            ui(value)
        }}
    }
}
