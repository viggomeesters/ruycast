import AppKit
import SwiftUI
import Darwin
import LauncherCore

/// Offscreen CPU chart with OS CPU counters independent of ResourceSampler.
func benchmarkMonitor(activeSeconds:Double=1800,idleSeconds:Double=60)throws->[String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let model=SystemMonitorModel(),view=NSHostingView(rootView:SystemMonitorView(model:model,tab:.cpu))
    view.frame=NSRect(x:0,y:0,width:900,height:640)
    func cpu()->Double {var usage=rusage();getrusage(RUSAGE_SELF,&usage);return Double(usage.ru_utime.tv_sec+usage.ru_stime.tv_sec)+Double(usage.ru_utime.tv_usec+usage.ru_stime.tv_usec)/1_000_000}
    func footprint()->UInt64 {
        var info=task_vm_info_data_t(),count=mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size/MemoryLayout<Int32>.size)
        let status=withUnsafeMutablePointer(to:&info) {p in p.withMemoryRebound(to:integer_t.self,capacity:Int(count)) {task_info(mach_task_self_,task_flavor_t(TASK_VM_INFO),$0,&count)}}
        return status==KERN_SUCCESS ? info.phys_footprint:0
    }
    func interval(_ name:String,_ seconds:Double,render:Bool)->[String:Any] {
        fputs("monitor phase=\(name) durationS=\(seconds)\n",stderr)
        let began=DispatchTime.now().uptimeNanoseconds,initialCPU=cpu(),initialFootprint=footprint()
        var maxFootprint=initialFootprint,frames=0,lastFrame:Date?,lastProgress=0
        while Double(DispatchTime.now().uptimeNanoseconds-began)/1_000_000_000<seconds {
            autoreleasepool {
                _=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.05))
                if render,let date=model.snapshot?.sampledAt,date != lastFrame {
                    lastFrame=date;view.layoutSubtreeIfNeeded()
                    if let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) {view.cacheDisplay(in:view.bounds,to:bitmap);frames+=1}
                }
            }
            maxFootprint=max(maxFootprint,footprint())
            let elapsed=Int(Double(DispatchTime.now().uptimeNanoseconds-began)/1_000_000_000)
            if elapsed/60>lastProgress/60 {fputs("monitor phase=\(name) elapsedS=\(elapsed) frames=\(frames)\n",stderr);lastProgress=elapsed}
        }
        let duration=Double(DispatchTime.now().uptimeNanoseconds-began)/1_000_000_000
        return ["durationS":duration,"cpuPercentOneCore":100*(cpu()-initialCPU)/max(0.001,duration),"footprintStartMiB":Double(initialFootprint)/1048576,"footprintEndMiB":Double(footprint())/1048576,"footprintMaxMiB":Double(maxFootprint)/1048576,"renderedFrames":frames,"historyCount":model.history.count]
    }
    let before=interval("idle-before",idleSeconds,render:false)
    model.start();let active=interval("cpu-chart",activeSeconds,render:true);model.stop()
    let last=model.snapshot?.sampledAt,after=interval("idle-after",idleSeconds,render:false)
    guard !model.active,model.history.count<=60,model.snapshot?.sampledAt==last else {throw LauncherError("Stopped monitor continued sampling")}
    let extra=(active["cpuPercentOneCore"] as! Double)-((before["cpuPercentOneCore"] as! Double)+(after["cpuPercentOneCore"] as! Double))/2
    return ["ok":true,"scope":"Isolated offscreen CPU chart at native 2-second sample interval, getrusage CPU and TASK_VM_INFO footprint; excludes display presentation, battery and physical energy","idleBefore":before,"active":active,"idleAfter":after,"extraCPUPercentOneCore":extra,"samplingStopsAfterClose":true,"desktopEffects":false]
}
