import AppKit
import SwiftUI
import LauncherCore

func defaultProfilePath()->String {NSString(string:"~/.config/ruycast").expandingTildeInPath}
let args=Array(CommandLine.arguments.dropFirst())
let root=URL(fileURLWithPath:ProcessInfo.processInfo.environment["RUYCAST_HOME"] ?? ProcessInfo.processInfo.environment["RUSTCAST_HOME"] ?? defaultProfilePath())
func output(_ value:[String:Any]) {if let data=try? JSONSerialization.data(withJSONObject:value,options:[.sortedKeys]),let s=String(data:data,encoding:.utf8) {print(s)}}

if args.first=="--render-public" {
    do {guard args.count==2 else {throw LauncherError("Expected assets directory")};output(try renderPublicPreview(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-command-log" {
    do {output(try checkCommandLog());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if ["--check-keyboard","--check-system","--check-clipboard","--check-utilities"].contains(args.first ?? "") {
    do {let result = try args.first=="--check-keyboard" ? checkKeyboard():args.first=="--check-system" ? checkSystemServices():args.first=="--check-utilities" ? checkUtilities():checkClipboardStore();output(result);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-analytics" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkAnalytics(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-rich-resources" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkRichResources(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-feedback" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkFeedback(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-settings" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkSettings(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-actions-menu" {
    do {output(try checkActionsMenu(output:args.count>1 ? args[1]:nil));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-visual-assets" {
    do {output(try checkVisualAssets());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--render-website-icons" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory.")};output(try renderWebsiteIcons(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--render-visual-assets" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory.")};try renderVisualAssets(args[1]);output(["ok":true]);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--validate-config" {
    do {guard args.count==2 else {throw LauncherError("Expected --validate-config PATH.")};let config=try Files.config(URL(fileURLWithPath:args[1]));output(["ok":true,"commands":config.commands.count]);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--set-config-directory" {
    do {guard args.count==2 else {throw LauncherError("Expected a configuration folder path.")};let directory=try ConfigurationLocation.relocate(root:root,to:args[1]);output(["ok":true,"result":["directory":directory.path,"path":directory.appendingPathComponent("config.json").path,"originalRetained":true]]);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--init-config" {
    do {let p=try ConfigurationLocation.initialize(root:root);output(["ok":true,"path":p.path]);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--benchmark-lifecycle" {do {output(try benchmarkLifecycle(blocks:args.count>1 ? max(1,min(100,Int(args[1]) ?? 10)):10,render:!args.contains("model")));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
if args.first=="--check-performance-pipeline" {do {output(try checkPerformancePipeline());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
if args.first=="--benchmark-performance" {let result=benchmarkPerformance();output(result);exit(result["ok"] as? Bool==true ? 0:1)}
if args.first=="--benchmark-search" {
    do {output(try benchmarkSearch(root:root));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-render-updates" {
    do {guard args.count==2 else {throw LauncherError("Expected render output directory.")};output(try checkRenderedUpdates(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-interactions" {
    do {output(try checkInteractions());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--render-features" {
    do {guard args.count==4 else {throw LauncherError("Expected --render-features OUTPUT light|dark MODE.")};try renderFeatures(args[1],theme:args[2],mode:args[3]);exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-search" {
    do {output(try checkSearch(root:root));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--render-icons" {
    do {
        guard args.count==3 else {throw LauncherError("Expected --render-icons OUTPUT light|dark.")}
        _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
        let view=NSHostingView(rootView:CommandIconSheet())
        view.appearance=NSAppearance(named:args[2]=="dark" ? .darkAqua:.aqua)
        view.frame=NSRect(x:0,y:0,width:660,height:350);view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Could not render icons.")}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode icons.")}
        try data.write(to:URL(fileURLWithPath:args[1]));output(["ok":true]);exit(0)
    } catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-palette" {
    _=NSApplication.shared
    NSApp.setActivationPolicy(.prohibited)
    let displays=[NSRect(x:0,y:0,width:3440,height:1400),NSRect(x:1920,y:0,width:1920,height:1080),NSRect(x:-1920,y:-180,width:1920,height:1080),NSRect(x:40,y:20,width:640,height:400),NSRect(x:0,y:0,width:300,height:200)]
    let centered=displays.allSatisfy {screen in
        let frame=PaletteGeometry.frame(in:screen)
        return screen.contains(frame) && abs(frame.midX-screen.midX)<0.01 && abs(frame.midY-screen.midY)<0.01
    }
    let finder=Command(id:"finder",title:"Finder",kind:"app",target:"com.apple.finder")
    let icon=CommandIcons.shared.image(for:finder)
    let nativeIcon=icon != nil && icon?.tiffRepresentation != nil
    let cached=icon === CommandIcons.shared.image(for:finder)
    let missing=CommandIcons.shared.image(for:Command(id:"missing",title:"Missing",kind:"app",target:"invalid.rustcast.missing"))==nil
    let glyphs=Configuration.layouts.compactMap {layout -> Data? in
        let view=NSHostingView(rootView:CommandGlyph(kind:"window",target:layout))
        view.appearance=NSAppearance(named:.darkAqua);view.frame=NSRect(x:0,y:0,width:24,height:24);view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {return nil}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        return bitmap.representation(using:.png,properties:[:])
    }
    let distinctGlyphs=glyphs.count==Configuration.layouts.count && Set(glyphs).count==Configuration.layouts.count
    let passed=centered && nativeIcon && cached && missing && distinctGlyphs
    output(["ok":passed,"centeredDisplayCases":displays.count,"centeredAndContained":centered,"finderNativeIcon":nativeIcon,"iconCached":cached,"missingAppFallback":missing,"distinctWindowGlyphs":distinctGlyphs,"windowGlyphCount":glyphs.count])
    exit(passed ? 0:1)
}
if args.first=="--render" {
    do {
        guard args.count>=3 else {throw LauncherError("Expected --render OUTPUT light|dark [WIDTH HEIGHT].")}
        _=NSApplication.shared
        NSApp.setActivationPolicy(.prohibited)
        let state=try AppState(root:root,headless:true)
        let width=args.count>3 ? Double(args[3]) ?? 760:760,height=args.count>4 ? Double(args[4]) ?? 490:490
        let view=NSHostingView(rootView:Palette(state:state));view.appearance=NSAppearance(named:args[2]=="dark" ? .darkAqua:.aqua)
        view.frame=NSRect(x:0,y:0,width:width,height:height);view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Could not render palette.")}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode preview.")}
        try data.write(to:URL(fileURLWithPath:args[1]));exit(0)
    } catch {fputs(error.localizedDescription+"\n",stderr);exit(1)}
}
final class AppDelegate:NSObject,NSApplicationDelegate {
    var state:AppState?
    var server:ControlServer?
    var palette:PaletteController?
    var management:ManagementController?
    var monitor:SystemMonitorController?
    var statusItem:NSStatusItem?
    func applicationDidFinishLaunching(_ notification:Notification) {
        do {
            // Claim the profile before starting any observers or clipboard work.
            let control=ControlServer(root:root)
            try control.start { [weak self] req,reply in guard let state=self?.state else {reply(["ok":false,"error":"Starting up; retry."]);return};state.handle(req,reply:reply) }
            server=control
            let s=try AppState(root:root,headless:args.contains("--headless"));state=s
            if !s.headless {
                palette=PaletteController(state:s)
                management=ManagementController(state:s)
                monitor=SystemMonitorController(state:s)
                let item=NSStatusBar.system.statusItem(withLength:NSStatusItem.squareLength)
                item.button?.image=NSImage(systemSymbolName:"command",accessibilityDescription:"Ruycast")
                item.button?.image?.isTemplate=true;item.button?.toolTip="Ruycast";statusItem=item
                let menu=NSMenu()
                for (title,action) in [("Open Launcher",#selector(openLauncher)),("Clipboard History",#selector(openClipboard)),("Dashboard",#selector(openDashboard)),("Settings…",#selector(openSettings)),("System Monitor",#selector(openMonitor)),("Open Configuration",#selector(openConfig)),("Reload Configuration",#selector(reload)),("Quit",#selector(quit))] {let mi=NSMenuItem(title:title,action:action,keyEquivalent:"");mi.target=self;menu.addItem(mi)}
                item.menu=menu
            }
        } catch {fputs(error.localizedDescription+"\n",stderr);NSApp.terminate(nil)}
    }
    @objc func openLauncher() {palette?.show("commands")}
    @objc func openClipboard() {palette?.show("clipboard")}
    @objc func openDashboard() {state?.showAnalytics?()}
    @objc func openSettings() {management?.show()}
    @objc func openMonitor() {palette?.hide();monitor?.show()}
    @objc func openConfig() {if let state {NSWorkspace.shared.open(state.configURL)}}
    @objc func reload() {do {try state?.reload();palette?.showToast("Configuration reloaded",true)} catch {palette?.showToast(error.localizedDescription,false)}}
    @objc func quit() {NSApp.terminate(nil)}
    func applicationWillTerminate(_ notification:Notification) {state?.shutdown();server?.stop();server=nil}
}
let app=NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate=AppDelegate();app.delegate=delegate;app.run()
