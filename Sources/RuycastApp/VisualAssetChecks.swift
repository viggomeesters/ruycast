import AppKit
import SwiftUI
import LauncherCore

func checkVisualAssets() throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    try require(ShortcutPresentation("shift+ctrl+alt+cmd+left").keys==["✦","←"],"Hyper is not compact.")
    try require(ShortcutPresentation("cmd+alt+n").keys==["⌥","⌘","N"],"Modifier order or glyphs changed.")
    try require(ShortcutPresentation("ctrl+left-control").keys==["⌃"],"Solo modifier is duplicated.")
    try require(ShortcutPresentation("ctrl+alt+delete").keys==["⌃","⌥","⌫"],"Delete key is unreadable.")
    try require(ShortcutPresentation("right-cmd+a").keys==["R⌘","A"],"Right Command badge lost its side.")
    try require(ShortcutPresentation("left-cmd+a").label=="Left Command + a","Left Command accessibility label lost its side.")
    try require(WebsiteIcons.origin("https://user:secret@example.com:8443/private/path?q=secret#fragment")?.absoluteString=="https://example.com:8443/","Quicklink secrets reached icon request.")
    try require(WebsiteIcons.origin("obsidian://open?path=private")==nil && WebsiteIcons.origin("https://{query}.example.com")==nil,"Unsupported origin accepted.")
    let origin=URL(string:"https://example.com/")!
    let html="<link href='/brand.png' sizes='128x128' rel='apple-touch-icon'><LINK REL=icon HREF=\"https://cdn.example.com/icon.png?v=1&amp;x=2\"><link rel=icon href='data:bad'>"
    try require(WebsiteIcons.candidates(html:html,origin:origin).map(\.absoluteString)==["https://example.com/brand.png","https://cdn.example.com/icon.png?v=1&x=2"],"Icon discovery failed.")
    try require(WebsiteIcons.png(Data(repeating:0,count:1_048_577))==nil && WebsiteIcons.png(Data("not an image".utf8))==nil,"Invalid/oversize image accepted.")
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("ruycast-icons-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:2,pixelsHigh:2,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:8,bitsPerPixel:32)!
    if let bytes=bitmap.bitmapData {for i in 0..<16 {bytes[i]=255}}
    let png=bitmap.representation(using:.png,properties:[:])!
    let counter=IconRequestCounter()
    let store=WebsiteIcons(cache:root,load:{url in counter.increment();Thread.sleep(forTimeInterval:0.05);return png})
    let start=Date();_=store.image(for:"https://example.com/private?secret=one");_=store.image(for:"https://example.com/another?secret=two")
    try require(Date().timeIntervalSince(start)<0.04,"Icon fetch blocks the main thread.")
    let deadline=Date().addingTimeInterval(3)
    while store.image(for:origin.absoluteString)==nil,Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    try require(store.image(for:origin.absoluteString) != nil && counter.value==1 && store.revision>0,"Async deduplication/publication failed.")
    let cached=WebsiteIcons(cache:root,load:{_ in counter.increment();return nil})
    _=cached.image(for:origin.absoluteString)
    while cached.image(for:origin.absoluteString)==nil,Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    try require(cached.image(for:origin.absoluteString) != nil && counter.value==1,"Disk cache did not survive a new store.")
    let file=root.appendingPathComponent("native-icon.txt")
    try Data("fixture".utf8).write(to:file)
    let native=CommandIcons(lifetime:0),fileCommand=Command(id:"fixture.file",title:"Fixture",kind:"path",target:file.path)
    try require(native.image(for:fileCommand) != nil,"Native file icon missing")
    try FileManager.default.removeItem(at:file)
    try require(native.image(for:fileCommand)==nil,"Expired native icon concealed deleted file")
    try Data("fixture".utf8).write(to:file)
    try require(native.image(for:fileCommand) != nil,"Native icon did not recover after recreation")
    var metadataCalls=0,clock=Date();let names=ApplicationNames(now:{clock},resolve:{id in metadataCalls+=1;return id=="found" ? "Found App":nil})
    for _ in 0..<1000 {_=names.name("found");_=names.name("missing")}
    try require(metadataCalls==2 && names.name("found")=="Found App" && names.name("missing")=="missing","Source labels repeat native bundle lookups")
    clock=clock.addingTimeInterval(31);_=names.name("missing");try require(metadataCalls==3,"Source label metadata never refreshes")
    let boundedCount=IconRequestCounter()
    let bounded=WebsiteIcons(cache:root.appendingPathComponent("offline"),load:{_ in boundedCount.increment();return nil})
    for i in 0..<2000 {_=bounded.image(for:"https://fixture-\(i).example.invalid/")}
    let settle=Date().addingTimeInterval(0.5);while Date()<settle {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    try require(boundedCount.value>0 && boundedCount.value<=64,"Icon flood queued unbounded origin fetches")
    return ["ok":true,"boundedOriginFlood":true,"nativeIconInvalidation":true,"sourceNameLookupCoalescedAndRefreshed":true,"compactHyperAndNativeKeycaps":true,"originOnlyRequests":true,"htmlIconDiscovery":true,"invalidImageBounds":true,"nonBlockingAndDeduplicated":true,"observableIconArrival":true,"persistentCache":true,"desktopEffects":false]
}
private final class IconRequestCounter {
    private let lock=NSLock();private var count=0
    func increment() {lock.lock();count+=1;lock.unlock()}
    var value:Int {lock.lock();defer {lock.unlock()};return count}
}
func renderVisualAssets(_ directory:String) throws {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let output=URL(fileURLWithPath:directory);try FileManager.default.createDirectory(at:output,withIntermediateDirectories:true)
    for dark in [false,true] {
        let view=NSHostingView(rootView:VStack(alignment:.leading,spacing:14) {
            HStack {Text("Left Half");Spacer();ShortcutBadge(value:"ctrl+alt+shift+cmd+left")}
            HStack {Text("Quick Note");Spacer();ShortcutBadge(value:"ctrl+alt+shift+cmd+n")}
            HStack {Text("Clipboard History");Spacer();ShortcutBadge(value:"ctrl+alt+shift+cmd+v")}
            HStack {Text("Force Quit");Spacer();ShortcutBadge(value:"ctrl+alt+delete")}
            HStack {Text("Dictate Voice");Spacer();ShortcutBadge(value:"ctrl+left-control")}
            HStack {Text("Safari");Spacer();ShortcutBadge(value:"right-cmd+s")}
            HStack {Text("Finder");Spacer();ShortcutBadge(value:"left-cmd+f")}
            HStack {Text("App Settings");Spacer();ShortcutBadge(value:"cmd+,")}
        }.font(.system(size:13,weight:.medium)).padding(22).frame(width:420,height:340).background(Color(nsColor:.windowBackgroundColor)))
        view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:420,height:340);view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Could not render shortcut preview.")};view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode shortcut preview.")}
        try png.write(to:output.appendingPathComponent(dark ? "shortcuts-dark.png":"shortcuts-light.png"))
    }
}

func renderWebsiteIcons(_ directory:String) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let examples=[("GitHub","https://github.com/private?never-sent=1"),("YouTube","https://www.youtube.com/feed/subscriptions"),("Notion","https://www.notion.so/workspace")]
    let store=WebsiteIcons.shared,deadline=Date().addingTimeInterval(30)
    for example in examples {_=store.image(for:example.1)}
    while examples.contains(where:{store.image(for:$0.1)==nil}),Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.05))}
    let loaded=examples.filter {store.image(for:$0.1) != nil}.count
    let output=URL(fileURLWithPath:directory);try FileManager.default.createDirectory(at:output,withIntermediateDirectories:true)
    for dark in [false,true] {
        let view=NSHostingView(rootView:VStack(spacing:16) {
            ForEach(examples.indices,id:\.self) {i in
                HStack(spacing:12) {CommandIcon(command:Command(id:"site.\(i)",title:examples[i].0,kind:"url",target:examples[i].1));Text(examples[i].0).font(.system(size:13,weight:.semibold));Spacer();ShortcutBadge(value:"ctrl+alt+shift+cmd+\(i+1)")}
            }
        }.padding(22).frame(width:420,height:180).background(Color(nsColor:.windowBackgroundColor)))
        view.appearance=NSAppearance(named:dark ? .darkAqua:.aqua);view.frame=NSRect(x:0,y:0,width:420,height:180);view.layoutSubtreeIfNeeded()
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Could not render website icons.")};view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode website icons.")};try data.write(to:output.appendingPathComponent(dark ? "sites-dark.png":"sites-light.png"))
    }
    return ["ok":loaded==examples.count,"loaded":loaded,"requested":examples.count,"originOnly":true]
}
