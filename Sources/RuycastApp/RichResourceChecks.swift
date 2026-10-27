import AppKit
import SwiftUI
import Vision
import LauncherCore
import LauncherModels

/// Fixture processes and a named pasteboard only: never kills real processes or reads general.
@MainActor func checkRichResources(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-rich-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump(_ duration:Double=0.12) {let end=Date().addingTimeInterval(duration);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait(_ condition:()->Bool) throws {let end=Date().addingTimeInterval(5);while !condition(),Date()<end {pump(0.01)};try require(condition(),"Native resource fixture timed out")}
    let samplesBox=LockedValue([ProcessResource(identity:.init(pid:123410,uid:getuid(),seconds:10,micros:0),name:"Syncthing",path:"/usr/bin/true",cpuPercent:49.5,memoryBytes:163*1_048_576),ProcessResource(identity:.init(pid:123411,uid:getuid(),seconds:11,micros:0),name:"Obsidian",path:"/Applications/Obsidian.app/Contents/MacOS/Obsidian",cpuPercent:23.9,memoryBytes:370*1_048_576),ProcessResource(identity:.init(pid:123412,uid:getuid(),seconds:12,micros:0),name:"Safari",path:"/System/Applications/Safari.app/Contents/MacOS/Safari",cpuPercent:9.8,memoryBytes:64*1_048_576)])
    var samples:[ProcessResource] {get {samplesBox.read {$0}} set {samplesBox.update {$0=newValue}}}
    var killed=[pid_t]();var effects=SystemServices.Effects()
    effects.processes={samplesBox.read {$0}};effects.identity={pid in samplesBox.read {$0.first {$0.id==pid}?.identity}};effects.terminate={pid in killed.append(pid);samples.removeAll {$0.id==pid};return 0}
    let service=SystemServices(effects:effects),state=try AppState(root:root,headless:true,systemServices:service)
    state.mode="processes";try wait {state.results.count==3}
    let original=state.selectedCommand!.id
    state.setProcessSort(.memory);try require(state.selectedCommand?.id==original && state.visibleCommands.first?.title=="Obsidian","Sorting changed selected identity or memory order")
    var metrics=state.processResources
    let current=metrics[original]!
    metrics[original]=ProcessResource(identity:current.identity,name:current.name,path:current.path,cpuPercent:0.1,memoryBytes:current.memoryBytes)
    state.applySystemResults(state.results,resources:metrics,preserveSelection:true)
    try require(state.selectedCommand?.id==original,"Refresh moved process selection")
    let without=state.results.filter {$0.id != original}
    state.applySystemResults(without,resources:metrics,preserveSelection:true)
    state.activateSelection();try require(state.selection == -1 && killed.isEmpty,"Disappearing selection activated another process")
    state.query="Obsidian";try wait {state.results.count==1}
    try require(state.actionItems.first?.title=="Kill" && !state.actionItems.contains(where:{$0.id=="bindings" || $0.id=="disable"}),"Process actions are mislabeled or persist ephemeral targets")
    state.activateSelection();try wait {!state.busy && state.results.isEmpty}
    try require(killed==[123411] && state.confirmationCommand==nil,"Direct Kill did not execute once without a confirmation")
    // Restore samples for visual checks, never an actual PID.
    samples.append(ProcessResource(identity:.init(pid:123411,uid:getuid(),seconds:11,micros:0),name:"Obsidian",path:"/Applications/Obsidian.app/Contents/MacOS/Obsidian",cpuPercent:23.9,memoryBytes:370*1_048_576))
    service.invalidateProcessCache();state.query="";state.setProcessSort(.cpu);try wait {state.results.count==3};state.selection=0
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    let window=NSWindow(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:.borderless,backing:.buffered,defer:false);window.isReleasedWhenClosed=false;defer {window.close()}
    let host=NSHostingView(rootView:Palette(state:state));window.contentView=host;host.frame=NSRect(x:0,y:0,width:760,height:490)
    var renders=0
    @discardableResult func capture(_ view:NSView,_ name:String,words:[String]=[])->Data {
        pump();view.layoutSubtreeIfNeeded()
        let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds)!;view.cacheDisplay(in:view.bounds,to:bitmap)
        let data=bitmap.representation(using:.png,properties:[:])!
        try! data.write(to:directory.appendingPathComponent(name+".png"))
        if name.hasPrefix("clipboard-"),(name.hasSuffix("-1") || name.hasSuffix("-2")) {
            // Nested native hosts can return an extended/float display bitmap.
            // Normalize to RGBA8 rather than relying on NSBitmapImageRep.colorAt.
            let width=bitmap.pixelsWide,height=bitmap.pixelsHigh
            var pixels=[UInt8](repeating:0,count:width*height*4),colored=0
            pixels.withUnsafeMutableBytes {buffer in
                let context=CGContext(data:buffer.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
                context.draw(bitmap.cgImage!,in:CGRect(x:0,y:0,width:width,height:height))
            }
            for y in stride(from:0,to:height,by:4) {for x in stride(from:width*2/5,to:width,by:4) {
                let i=(y*width+x)*4
                if pixels[i+3]>229 && ((pixels[i]>204 && pixels[i+2]<51) || (pixels[i+2]>204 && pixels[i]<51)) {colored+=1}
            }}
            precondition(colored>300,"Clipboard image pixels are absent from the rendered view: \(name)")
        }
        if !words.isEmpty {
            let request=VNRecognizeTextRequest();request.recognitionLevel = .accurate;try! VNImageRequestHandler(cgImage:bitmap.cgImage!).perform([request])
            let text=(request.results ?? []).compactMap {$0.topCandidates(1).first?.string}.joined(separator:" ")
            precondition(words.allSatisfy {text.localizedCaseInsensitiveContains($0)},"Missing rendered labels in \(name): \(text)")
        }
        renders+=1;return data
    }
    for dark in [true,false] {host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);capture(host,"processes-\(dark ? "dark":"light")",words:["Obsidian","49.5","163","Kill"])}
    let pb=NSPasteboard(name:NSPasteboard.Name("rc-rich-fixture-"+UUID().uuidString));defer {pb.releaseGlobally()}
    let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:240,pixelsHigh:140,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:960,bitsPerPixel:32)!
    for y in 0..<140 {for x in 0..<240 {let offset=y*960+x*4;let bytes=bitmap.bitmapData!;bytes[offset]=x<120 ? 255:0;bytes[offset+1]=x<120 ? 128:102;bytes[offset+2]=x<120 ? 0:255;bytes[offset+3]=255}}
    let png=bitmap.representation(using:.png,properties:[:])!
    pb.setData(png,forType:.png)
    let image=try state.clipboardStore.capture(from:pb,source:"md.obsidian",settings:state.config.clipboard)!
    let file=root.appendingPathComponent("clipboard-preview.png");try png.write(to:file)
    var fileClip=Clip(text:"Files: clipboard-preview.png",source:"md.obsidian");fileClip.contentType="files";fileClip.fileURLs=[file.absoluteString]
    let text=Clip(text:"Clipboard preview keeps the full text.\nA second line with Ω and a path /Users/example/notes.",source:"md.obsidian")
    state.clips=[text,image,fileClip];state.mode="clipboard";state.selection=0
    for dark in [true,false] {
        host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua)
        for index in [0,1,2] {state.selection=index;pump(0.2);capture(host,"clipboard-\(dark ? "dark":"light")-\(index)",words:index==0 ? ["Characters","full text"]:["Dimensions","240","140"])}
    }
    state.selection=0;pump(0.2)
    let first=capture(host,"clipboard-static-before");pump(1.2);let second=capture(host,"clipboard-static-after")
    try require(first==second,"Clipboard timestamps still animate or update while idle")
    var forged=image;forged.dataFile="../outside.png"
    var rejected=false;do {_=try state.clipboardStore.preview(forged)} catch {rejected=true}
    try require(rejected,"Clipboard preview accepted an unsafe owned-asset path")
    let decoded=try state.clipboardStore.preview(image),decodedFile=try state.clipboardStore.preview(fileClip)
    if let decoded {let pixels=NSBitmapImageRep(cgImage:decoded.image);try require((pixels.colorAt(x:60,y:70)?.alphaComponent ?? 0)>0.9 && pixels.colorAt(x:decoded.image.width/4,y:decoded.image.height/2) != pixels.colorAt(x:decoded.image.width*3/4,y:decoded.image.height/2),"Image preview lost its actual pixels")}
    try require(decoded?.width==240 && decoded?.height==140 && decodedFile?.width==240,"Image or image-file preview failed")
    let monitor=SystemMonitorModel();monitor.start();try wait {monitor.snapshot != nil};monitor.stop();let samplesAfterStop=monitor.history.count;pump(2.1)
    try require(!monitor.active && monitor.history.count==samplesAfterStop,"Monitor kept sampling while closed")
    let actual=ResourceSampler().sample()
    try require(actual.memoryTotal>0 && actual.diskTotal>0 && actual.diskAvailable<=actual.diskTotal && actual.processCount>0,"Native system sampling did not return real values")
    var fixture=actual;fixture.cpuPercent=42.5;fixture.memoryTotal=16*1_073_741_824;fixture.memoryUsed=10*1_073_741_824;fixture.memoryWired=2*1_073_741_824;fixture.memoryCompressed=512*1_048_576
    fixture.batteryPercent=90;fixture.charging=true;fixture.powerSource="AC Power";fixture.downloadBytesPerSecond=1024*420;fixture.uploadBytesPerSecond=1024*32
    monitor.snapshot=fixture;monitor.history=(0..<30).map {index in var value=fixture;value.sampledAt=fixture.sampledAt.addingTimeInterval(Double(index-29)*2);value.cpuPercent=Double(20+index);return value}
    for dark in [true,false] {for tab in MonitorTab.allCases {
        let view=NSHostingView(rootView:SystemMonitorView(model:monitor,tab:tab));view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:900,height:640)
        capture(view,"monitor-\(dark ? "dark":"light")-\(tab.rawValue.replacingOccurrences(of:" ",with:"-"))",words:[tab.rawValue])
    }}
    let controller=SystemMonitorController(state:state);defer {controller.window.close()}
    try require(!controller.window.isVisible && !controller.model.active,"Monitor window opened or sampled automatically")
    let palette=PaletteController(state:state);var routed=false;state.showSystemMonitor={routed=true};palette.show("system-monitor")
    try require(routed && !palette.panel.isVisible,"System Monitor did not use its own window route")
    let server=ControlServer(root:root);try server.start {request,reply in state.handle(request,sink:reply)};defer {server.stop()}
    func cli(_ arguments:[String]) throws -> [String:Any] {
        let process=Process(),pipe=Pipe();process.executableURL=ruycastCLI();process.arguments=arguments;var env=ProcessInfo.processInfo.environment;env["RUYCAST_HOME"]=root.path;process.environment=env
        process.standardOutput=pipe;process.standardError=FileHandle.nullDevice;try process.run()
        let end=Date().addingTimeInterval(5);while process.isRunning,Date()<end {pump(0.01)}
        guard !process.isRunning else {process.terminate();throw LauncherError("CLI resource check timed out")}
        return try JSONSerialization.jsonObject(with:pipe.fileHandleForReading.readDataToEndOfFile()) as! [String:Any]
    }
    let monitorReply=try cli(["monitor"]),processReply=try cli(["processes","Syncthing"])
    try require(monitorReply["ok"] as? Bool==true && processReply["ok"] as? Bool==true,"Read-only resource CLI failed")
    var batches=[[(Command,ProcessResource)]]()
    service.processDetails("Syncthing") {if case .success(let rows)=$0 {batches.append(rows)}}
    service.processDetails("Safari") {if case .success(let rows)=$0 {batches.append(rows)}}
    try wait {batches.count==2}
    try require(batches[0].first?.1.name=="Syncthing" && batches[1].first?.1.name=="Safari","Concurrent process queries mixed resource metadata")
    var largeEffects=SystemServices.Effects()
    largeEffects.processes={(0..<2100).map {index in ProcessResource(identity:.init(pid:pid_t(200000+index),uid:getuid(),seconds:UInt64(index),micros:0),name:"Fixture \(index)",path:"/usr/bin/true",cpuPercent:Double(index),memoryBytes:1)}}
    let largeService=SystemServices(effects:largeEffects);var bounded:[(Command,ProcessResource)]?
    largeService.processDetails("") {if case .success(let rows)=$0 {bounded=rows}}
    try wait {bounded != nil}
    try require(bounded?.count==1000 && bounded?.first?.1.cpuPercent==2099,"Large process lists were not bounded before target registration")
    try require(!window.isVisible,"Resource checks opened a visible window")
    return ["ok":true,"directKillNoConfirmation":true,"fakeTerminationOnly":true,"sortAndRefreshPreserveProcessIdentity":true,"vanishedSelectionCannotKillNext":true,"cpuRamPidAndIcons":true,"clipboardTextImageAndImageFilePreview":true,"clipboardStaticWhileIdle":true,"unsafeAssetPathRejected":true,"monitorSeparateWindowRoute":true,"monitorStopsSamplingWhenClosed":true,"realReadOnlySystemSampling":true,"monitorAndProcessCLI":true,"lightDarkRenders":renders,"visibleWindowOpened":false,"desktopInputSent":false]
}
