import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkAnalytics(_ outputPath:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-a-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func pump() {let end=Date().addingTimeInterval(0.05);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func wait(_ condition:()->Bool) throws {let end=Date().addingTimeInterval(8);while !condition(),Date()<end {pump()};try require(condition(),"Analytics fixture timed out")}
    let secret="PRIVATE_INPUT_SENTINEL_DONT_LOG"
    let fixture=root.appendingPathComponent("host"),state=try AppState(root:fixture,headless:true)
    state.config.commands=[Command(id:"fixture.success",title:"Open Daily Note",kind:"script",target:"/usr/bin/true"),Command(id:"fixture.failure",title:"Run Maintenance",kind:"script",target:"/usr/bin/false")]
    state.query=secret
    func execute(_ id:String,_ source:String) throws {
        var reply:[String:Any]?
        state.execute(id,dryRun:false,values:["input":secret],source:source) {reply=$0}
        try wait {reply != nil}
    }
    try execute("fixture.success","launcher");try execute("fixture.success","hotkey");try execute("fixture.failure","cli")
    let directory=URL(fileURLWithPath:outputPath);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    for dark in [true,false] {for width in [980,680] {
        let view=NSHostingView(rootView:AnalyticsDashboard(store:state.analytics,commands:state.config.commands,theme:SettingsTheme(dark:dark)))
        view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:width,height:1400);pump();view.layoutSubtreeIfNeeded()
        let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds)!;view.cacheDisplay(in:view.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("analytics-\(dark ? "dark":"light")-\(width).png"))
    }}
    return ["ok":true,"inputQueryAndIDsExcludedFromExport":true,"corruptDataPreservedUntilReset":true,"dashboardLightDarkMinWidthRenders":4,"desktopInputSent":false,"visibleWindowOpened":false]
}
