import Foundation
import os

// Software stages only. No input, command identifiers or clipboard data is recorded.
final class PerformanceTrace {
    enum Stage:String,CaseIterable {
        case searchModel="search.model",inputModel="input.model",inputReconcile="input.reconcile",openReady="open.ready",hostDraw="host.drawCallback",hotkeyHandler="hotkey.handler"
        var signpostName:StaticString {
            switch self {
            case .searchModel:return "Search model"
            case .inputModel:return "Input model"
            case .inputReconcile:return "Input to field reconciliation"
            case .openReady:return "Palette ready"
            case .hostDraw:return "Host draw callback"
            case .hotkeyHandler:return "Hotkey handler"
            }
        }
    }
    struct Span {let id:UInt64;let stage:Stage}
    private struct Entry {let start:UInt64;let state:OSSignpostIntervalState;let stage:Stage}
    private struct Metric {var total=0;var cancelled=0;var samples:[Double]=[]}
    static let shared=PerformanceTrace()
    private let lock=NSRecursiveLock()
    private let signposter=OSSignposter(subsystem:"nl.viggomeesters.ruycast",category:"Interaction")
    private var sequence:UInt64=0,active:[UInt64:Entry]=[:],metrics:[Stage:Metric]=[:]
    private var pendingInput:(span:Span,fieldID:ObjectIdentifier)?
    func begin(_ stage:Stage)->Span {
        lock.lock();defer {lock.unlock()}
        // Event bursts/cancelled UI work cannot retain unbounded interval state.
        if active.count>=32,let oldest=active.keys.min(),let entry=active[oldest] {finish(Span(id:oldest,stage:entry.stage),cancelled:true)}
        sequence &+= 1
        active[sequence]=Entry(start:DispatchTime.now().uptimeNanoseconds,state:signposter.beginInterval(stage.signpostName,id:signposter.makeSignpostID()),stage:stage)
        return Span(id:sequence,stage:stage)
    }
    func finish(_ span:Span,cancelled:Bool=false) {
        lock.lock();defer {lock.unlock()}
        guard let entry=active.removeValue(forKey:span.id) else {return}
        signposter.endInterval(entry.stage.signpostName,entry.state)
        if cancelled {metrics[entry.stage,default:Metric()].cancelled+=1;return}
        var value=metrics[entry.stage,default:Metric()];value.total+=1
        value.samples.append(Double(DispatchTime.now().uptimeNanoseconds-entry.start)/1_000_000)
        if value.samples.count>256 {value.samples.removeFirst(value.samples.count-256)}
        metrics[entry.stage]=value
    }
    func measure<T>(_ stage:Stage,_ work:()->T)->T {let span=begin(stage);defer {finish(span)};return work()}
    func inputChanged(fieldID:ObjectIdentifier) {
        lock.lock();defer {lock.unlock()}
        if let pendingInput {finish(pendingInput.span,cancelled:true)}
        pendingInput=(begin(.inputReconcile),fieldID)
    }
    func didReconcileField(fieldID:ObjectIdentifier) {
        lock.lock();defer {lock.unlock()}
        guard let pending=pendingInput,pending.fieldID==fieldID else {return}
        pendingInput=nil;finish(pending.span)
    }
    func cancelInput(fieldID:ObjectIdentifier?=nil) {
        lock.lock();defer {lock.unlock()}
        guard let pending=pendingInput,fieldID==nil || fieldID==pending.fieldID else {return}
        pendingInput=nil;finish(pending.span,cancelled:true)
    }
    func reset() {
        lock.lock();defer {lock.unlock()}
        for (_,entry) in active {signposter.endInterval(entry.stage.signpostName,entry.state)}
        active=[:];metrics=[:];pendingInput=nil
    }
    func snapshot()->[String:Any] {
        lock.lock();defer {lock.unlock()}
        var stages=[String:Any]()
        for stage in Stage.allCases {
            let metric=metrics[stage,default:Metric()],sorted=metric.samples.sorted()
            func percentile(_ p:Double)->Any {sorted.isEmpty ? NSNull():sorted[Int(Double(sorted.count-1)*p)] as Any}
            stages[stage.rawValue]=["count":metric.total,"retainedSamples":sorted.count,"cancelled":metric.cancelled,"p50MS":percentile(0.5),"p95MS":percentile(0.95),"maxMS":sorted.last as Any? ?? NSNull()]
        }
        return ["schema":2,"scope":"software stages; field reconciliation is not result drawing or physical presentation; host.drawCallback may be absent for layer-backed display; hotkey timing begins at handler delivery","sampleLimitPerStage":256,"activeIntervals":active.count,"stages":stages]
    }
}
