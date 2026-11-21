import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func benchmarkPerformance()->[String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-p-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    var checksum=0
    func measure(_ count:Int,_ block:() throws->Void) rethrows->[String:Any] {
        for _ in 0..<10 {try autoreleasepool {try block()}}
        var values=[Double]();values.reserveCapacity(count)
        for _ in 0..<count {let start=DispatchTime.now().uptimeNanoseconds;try autoreleasepool {try block()};values.append(AnalyticsStore.elapsed(start))}
        let s=values.sorted()
        return ["samples":s.count,"medianMS":s[s.count/2],"p95MS":s[min(s.count-1,Int(Double(s.count-1)*0.95))],"p99MS":s.count>=1000 ? s[min(s.count-1,Int(Double(s.count-1)*0.99))] as Any:NSNull(),"maxMS":s.last!]
    }
    do {
        let state=try AppState(root:root,headless:true)
        var queries=[String:Any](),sequence=0
        let terms=["m","mi","mic","microsoft","alias42","left","project dashboard","missing Ω","","notes"]
        for size in [500,1000,5000,10000] {
            state.config.commands=(0..<size).map {i in Command(id:"fixture.\(i)",title:["Microsoft Teams","Open Notes","Left Half","Project Dashboard"][i%4]+" \(i)",kind:"url",target:"https://example.com/\(i)",aliases:["alias\(i)"])}
            queries[String(size)]=measure(1000) {sequence+=1;state.query=terms[sequence%terms.count];checksum+=state.results.count}
        }
        let catalogRead=measure(1000) {checksum+=state.results.count}
        for i in 0..<512 {try state.searchRanking.recordSuccess("fixture.\(i)")}
        let rankedSearch=measure(1000) {checksum+=state.searchCommands("pro das").count}
        state.mode="processes"
        var commands=[Command](),details=[String:ProcessResource]()
        for i in 0..<1000 {
            let id="fixture.process.\(i)";commands.append(Command(id:id,title:"Process \(i)",kind:"builtin",target:id))
            details[id]=ProcessResource(identity:.init(pid:pid_t(200000+i),uid:getuid(),seconds:UInt64(i),micros:0),name:"Process \(i)",path:"/usr/bin/true",cpuPercent:Double(i%97),memoryBytes:UInt64(i)*1024)
        }
        state.applySystemResults(commands,resources:details,preserveSelection:false)
        let processReads=measure(1000) {checksum+=state.visibleCommands.count;checksum+=state.selectedCommand?.id.count ?? 0}
        let processSort=measure(100) {state.setProcessSort(state.processSort == .cpu ? .memory:.cpu);checksum+=state.selectedCommand?.id.count ?? 0}
        let settings=ClipboardSettings(enabled:true,limit:200,maxTextBytes:100000,retentionDays:30,excludedApps:[])
        let history=(0..<200).map {i in Clip(text:"Fixture \(i): "+String(repeating:"native clipboard Ω ",count:400),source:"fixture",date:Date().addingTimeInterval(Double(-i)))}
        state.mode="commands";state.clips=history
        let commandSearchWithHistory=measure(200) {sequence+=1;state.query=terms[sequence%terms.count];checksum+=state.results.count}
        state.mode="clipboard"
        // Version 2 reads the observable state directly; earlier Mirror-based waits
        // stopped working when clipboardSearchBusy became a forwarding property.
        func waitForClipboard() throws {
            let deadline=Date().addingTimeInterval(5)
            while state.clipboardSearchBusy {
                guard Date()<deadline else {throw NSError(domain:"RuycastPerformance",code:1,userInfo:[NSLocalizedDescriptionKey:"Clipboard filter did not publish within five seconds"])}
                _=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.0002))
            }
        }
        let clipboardSearchEnqueue=measure(200) {sequence+=1;state.query=terms[sequence%terms.count];checksum+=state.filteredClips.count}
        try waitForClipboard()
        let clipboardSearch=try measure(200) {sequence+=1;state.query=terms[sequence%terms.count];try waitForClipboard();checksum+=state.filteredClips.count}
        let prune=measure(20) {checksum+=History.prune(history,settings:settings).count}
        let historyEncode=measure(20) {checksum+=(try? Files.encode(history).count) ?? 0}
        try Files.save(state.config,to:state.configURL)
        let configRead=measure(1000) {checksum+=(try? Data(contentsOf:state.configURL).count) ?? 0}
        let analytics=measure(1000) {state.analytics.recordTiming("search.model",ms:0.1)}
        let iconCommand=Command(id:"fixture.finder",title:"Finder",kind:"app",target:"com.apple.finder")
        let iconWarm=measure(1000) {checksum+=CommandIcons.shared.image(for:iconCommand)==nil ? 0:1}
        let pb=NSPasteboard(name:NSPasteboard.Name("rc-p-"+UUID().uuidString));defer {pb.releaseGlobally()}
        let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:1200,pixelsHigh:800,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:4800,bitsPerPixel:32)!
        var seed:UInt32=42
        for i in 0..<(1200*800) {seed=seed &* 1664525 &+ 1013904223;let b=bitmap.bitmapData!.advanced(by:i*4);b[0]=UInt8(truncatingIfNeeded:seed);b[1]=UInt8(truncatingIfNeeded:seed>>8);b[2]=UInt8(truncatingIfNeeded:seed>>16);b[3]=255}
        let png=bitmap.representation(using:.png,properties:[:])!;pb.setData(png,forType:.png)
        var clip:Clip?
        let imageCapture=measure(6) {clip=try? state.clipboardStore.capture(from:pb,source:"fixture",settings:settings);if let clip {try? state.clipboardStore.pruneAssets(keeping:[clip],settings:settings);checksum+=clip.id.count}}
        let imagePreview=measure(20) {if let clip,let image=try? state.clipboardStore.preview(clip) {checksum+=image.width}}
        let data=try Files.encode(history);checksum+=data.count
        state.analytics.flushSynchronously()
        #if DEBUG
        let optimized=false
        #else
        let optimized=true
        #endif
        return ["ok":true,"clipboardCompletionMeasurementVersion":2,"releaseBuild":optimized,"scope":"in-process model/main-thread work, deterministic fixtures; excludes physical keys, rendering/presentation and external app opening","system":["macOS":ProcessInfo.processInfo.operatingSystemVersionString,"chip":ResourceSampler.text("machdep.cpu.brand_string"),"cores":ProcessInfo.processInfo.processorCount],"queries":queries,"searchWith512RecentChoices10000":rankedSearch,"cachedResultReads":catalogRead,"processResultReads1000":processReads,"processSort1000":processSort,"commandSearchWith200x8kBHistory":commandSearchWithHistory,"clipboardSearch200x8kB":clipboardSearch,"clipboardSearchEnqueue200x8kB":clipboardSearchEnqueue,"clipboardPrune200x8kB":prune,"clipboardEncode200x8kB":historyEncode,"configurationRawRead":configRead,"analyticsTimingRecord":analytics,"nativeIconWarm":iconWarm,"imageCapture960kPixels":imageCapture,"imagePreview960kPixels":imagePreview,"checksum":checksum]
    } catch {return ["ok":false,"error":error.localizedDescription]}
}
