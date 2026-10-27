import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

/// Public imagery uses the production views and a disposable demonstration profile.
func renderPublicPreview(_ path:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-public-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true);defer {state.shutdown()}
    var commands=Configuration.initial.commands
    commands.insert(Command(id:"snippet.greeting",title:"Friendly Greeting",kind:"snippet",target:"Hello {cursor},\nThanks for getting in touch.",aliases:["hello"],keyword:";hello"),at:3)
    commands.insert(Command(id:"script.date",title:"Today's Date",kind:"script",target:"/bin/date",aliases:["today"],arguments:["+%Y-%m-%d"]),at:4)
    if let i=commands.firstIndex(where:{$0.id=="window.left-half"}) {commands[i].hotkey="ctrl+alt+shift+cmd+left"}
    state.config.commands=commands;state.query=""
    let nav=ManagementNavigation();nav.select(.commands)
    let directory=URL(fileURLWithPath:path);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    func render<V:View>(_ view:V,size:NSSize,name:String) throws -> NSImage {
        let host=NSHostingView(rootView:view);host.appearance=NSAppearance(named:.darkAqua)
        let window=NSWindow(contentRect:NSRect(origin:.zero,size:size),styleMask:.borderless,backing:.buffered,defer:false)
        window.isReleasedWhenClosed=false;window.appearance=host.appearance;window.contentView=host;defer {window.close()}
        host.frame=NSRect(origin:.zero,size:size)
        let end=Date().addingTimeInterval(0.25);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
        host.layoutSubtreeIfNeeded()
        guard !window.isVisible,let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("Public render unexpectedly visible or unavailable")}
        host.cacheDisplay(in:host.bounds,to:bitmap)
        guard let data=bitmap.representation(using:.png,properties:[:]),let image=NSImage(data:data) else {throw LauncherError("Public render encoding failed")}
        try data.write(to:directory.appendingPathComponent(name+".png"));return image
    }
    let launcher=try render(Palette(state:state),size:NSSize(width:760,height:490),name:"launcher")
    let settings=try render(ManagementView(state:state,navigation:nav),size:NSSize(width:980,height:680),name:"settings")
    let hero=NSImage(size:NSSize(width:1800,height:1200));hero.lockFocus()
    NSColor(calibratedRed:0.08,green:0.10,blue:0.15,alpha:1).setFill();NSRect(x:0,y:0,width:1800,height:1200).fill()
    let cream=NSColor(calibratedRed:0.97,green:0.95,blue:0.90,alpha:1)
    ("Ruycast" as NSString).draw(at:NSPoint(x:80,y:1050),withAttributes:[.font:NSFont.systemFont(ofSize:76,weight:.bold),.foregroundColor:cream])
    ("Your Mac. Your shortcuts." as NSString).draw(at:NSPoint(x:83,y:995),withAttributes:[.font:NSFont.systemFont(ofSize:32,weight:.medium),.foregroundColor:cream.withAlphaComponent(0.75)])
    launcher.draw(in:NSRect(x:80,y:440,width:760,height:490))
    settings.draw(in:NSRect(x:870,y:220,width:850,height:590))
    ("Native on macOS  ·  Rust + Swift  ·  CLI & readable configuration" as NSString).draw(at:NSPoint(x:83,y:95),withAttributes:[.font:NSFont.systemFont(ofSize:25,weight:.medium),.foregroundColor:cream.withAlphaComponent(0.65)])
    hero.unlockFocus()
    guard let tiff=hero.tiffRepresentation,let bitmap=NSBitmapImageRep(data:tiff),let png=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Hero encoding failed")}
    try png.write(to:directory.appendingPathComponent("hero.png"))
    return ["ok":true,"productionViews":true,"demonstrationProfile":true,"visibleWindowOpened":false,"desktopInputSent":false,"assets":["hero.png","launcher.png","settings.png"]]
}
