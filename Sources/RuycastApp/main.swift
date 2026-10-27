import AppKit
import SwiftUI
import Combine
import Observation
import LauncherCore
import LauncherModels

func defaultProfilePath()->String {NSString(string:"~/.config/ruycast").expandingTildeInPath}
let args=Array(CommandLine.arguments.dropFirst())
let root=URL(fileURLWithPath:ProcessInfo.processInfo.environment["RUYCAST_HOME"] ?? ProcessInfo.processInfo.environment["RUSTCAST_HOME"] ?? defaultProfilePath())
func output(_ value:[String:Any]) {if let data=try? JSONSerialization.data(withJSONObject:value,options:[.sortedKeys]),let s=String(data:data,encoding:.utf8) {print(s)}}

if args.first=="--benchmark-clipboard-poll" {
#if RUYCAST_POLL_BENCHMARK
    do {let ticks=args.count>1 ? Int(args[1]):10000;guard let ticks,ticks>0,ticks<=100000 else {throw LauncherError("Expected 1–100000 polls")};output(try benchmarkClipboardPoll(ticks:ticks,active:args.contains("active")));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
#else
    output(["ok":false,"error":"Build with -Xswiftc -DRUYCAST_POLL_BENCHMARK to run this fixture."]);exit(1)
#endif
}
if args.first=="--check-concurrency" {do {output(try checkConcurrencyOwnership());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
if args.first=="--benchmark-interactions" {do {output(try checkInteractionPerformance());exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
if args.first=="--check-script-choices" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkScriptChoicesUI(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--check-polish" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try checkLauncherPolish(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
if args.first=="--render-brand" {
    do {guard args.count==2 else {throw LauncherError("Expected output directory")};output(try renderBrandAssets(args[1]));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}
}
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
if args.first=="--benchmark-monitor" {do {output(try benchmarkMonitor(activeSeconds:args.count>1 ? max(1,min(3600,Double(args[1]) ?? 1800)):1800,idleSeconds:args.count>2 ? max(1,min(600,Double(args[2]) ?? 60)):60));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
if args.first=="--benchmark-lifecycle" {do {output(try benchmarkLifecycle(blocks:args.count>1 ? max(1,min(100,Int(args[1]) ?? 10)):10,render:!args.contains("model"),component:["search","preview","label"].first(where:args.contains) ?? "palette"));exit(0)} catch {output(["ok":false,"error":error.localizedDescription]);exit(1)}}
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
@MainActor final class AppDelegate:NSObject,NSApplicationDelegate {
    var state:AppState?
    var server:ControlServer?
    var palette:PaletteController?
    var management:ManagementController?
    var monitor:SystemMonitorController?
    var statusItem:NSStatusItem?
    var subscriptions:Set<AnyCancellable>=[]
    func applicationDidFinishLaunching(_ notification:Notification) {
        do {
            // Claim the profile before starting any observers or clipboard work.
            let control=ControlServer(root:root)
            try control.start { [weak self] req,sink in guard let state=self?.state else {sink.send(["ok":false,"error":"Starting up; retry."]);return};state.handle(req,sink:sink) }
            server=control
            let s=try AppState(root:root,headless:args.contains("--headless"));state=s
            if !s.headless {
                palette=PaletteController(state:s)
                management=ManagementController(state:s)
                monitor=SystemMonitorController(state:s)
                updateMenuBar()
                observeMenuBarState()
                for name in [NSWorkspace.didLaunchApplicationNotification,NSWorkspace.didTerminateApplicationNotification] {NSWorkspace.shared.notificationCenter.publisher(for:name).receive(on:DispatchQueue.main).sink { [weak self] _ in self?.updateMenuBar() }.store(in:&subscriptions)}
                s.analytics.objectWillChange.debounce(for:.seconds(2),scheduler:DispatchQueue.main).sink { [weak self] _ in self?.updateMenuBar() }.store(in:&subscriptions)

            }
        } catch {fputs(error.localizedDescription+"\n",stderr);NSApp.terminate(nil)}
    }
    func observeMenuBarState() {
        guard let state else {return}
        withObservationTracking {
            _=state.config;_=state.configError;_=state.clipboardCleanupError
        } onChange: { [weak self] in
            DispatchQueue.main.async { [weak self] in self?.updateMenuBar();self?.observeMenuBarState() }
        }
    }
    func updateMenuBar() {
        guard let state else {return}
        guard state.config.menuBarVisible ?? true else {if let statusItem {NSStatusBar.system.removeStatusItem(statusItem)};statusItem=nil;return}
        if statusItem==nil {
            let item=NSStatusBar.system.statusItem(withLength:NSStatusItem.squareLength),menu=NSMenu()
            for (title,action) in [("Open Launcher",#selector(openLauncher)),("Settings…",#selector(openSettings)),("Quit",#selector(quit))] {let row=NSMenuItem(title:title,action:action,keyEquivalent:"");row.target=self;menu.addItem(row)}
            item.menu=menu;statusItem=item
        }
        let healthy=state.runtimeHealthy
        statusItem?.button?.image=AppBrand.menuIcon(healthy:healthy)
        statusItem?.button?.toolTip=healthy ? "Ruycast · Running":"Ruycast · Needs attention"
        statusItem?.button?.setAccessibilityLabel(healthy ? "Ruycast, running":"Ruycast, needs attention")
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
