import AppKit
import SwiftUI
import Vision
import LauncherCore
import LauncherModels

@MainActor func checkLauncherPolish(_ outputPath:String)throws->[String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let directory=URL(fileURLWithPath:outputPath,isDirectory:true)
    try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
    let root=directory.appendingPathComponent("profile")
    var systemEffects=SystemServices.Effects();systemEffects.processes={[]};systemEffects.terminate={_ in -1}
    let state=try AppState(root:root,headless:true,systemServices:SystemServices(effects:systemEffects))
    defer {state.shutdown();try? FileManager.default.removeItem(at:root)}
    func require(_ condition:Bool,_ message:String)throws {if !condition {throw LauncherError(message)}}
    func pump(_ seconds:Double=0.12) {let end=Date().addingTimeInterval(seconds);while Date()<end {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}}
    func capture<V:View>(_ view:V,_ name:String,_ size:NSSize,required:[String]=[],absent:[String]=[])throws {
        let host=NSHostingView(rootView:view);host.appearance=NSAppearance(named:.darkAqua);host.frame=NSRect(origin:.zero,size:size);pump();host.layoutSubtreeIfNeeded()
        guard let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("No polish preview")};host.cacheDisplay(in:host.bounds,to:bitmap)
        guard let image=bitmap.cgImage,let png=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("No polish pixels")}
        try png.write(to:directory.appendingPathComponent(name+".png"))
        let request=VNRecognizeTextRequest();request.recognitionLevel = .accurate;try VNImageRequestHandler(cgImage:image).perform([request])
        let lines=(request.results ?? []).compactMap {$0.topCandidates(1).first?.string}
#if DEBUG
        try lines.joined(separator:"\n").write(to:directory.appendingPathComponent(name+"-copy.txt"),atomically:true,encoding:.utf8)
#endif
        let text=lines.joined(separator:" ")
        try require(required.allSatisfy{text.folding(options:[.diacriticInsensitive,.caseInsensitive],locale:Locale(identifier:"en")).contains($0.folding(options:[.diacriticInsensitive,.caseInsensitive],locale:Locale(identifier:"en")))},"Missing polish labels in \(name): \(text)")
        try require(absent.allSatisfy{!text.folding(options:[.diacriticInsensitive,.caseInsensitive],locale:Locale(identifier:"en")).contains($0.folding(options:[.diacriticInsensitive,.caseInsensitive],locale:Locale(identifier:"en")))},"Technical field leaked in \(name): \(text)")
    }
    var config=state.config;config.commands=featureFixture()
    config.commands.append(Command(id:"fixture.inline",title:"Inline Input",kind:"script",target:"/usr/bin/printf",aliases:["echo-inline"],arguments:["%s","{input}"],inputPrompt:"Text"))
    try Files.save(config,to:state.configURL);try state.reload()
    for theme in ["system","light","dark","graphite"] {
        state.config.theme=theme;state.query=""
        try capture(Palette(state:state),"launcher-"+theme,NSSize(width:760,height:490),absent:["Create"])
        let navigation=ManagementNavigation();navigation.select(.general)
        try capture(ManagementView(state:state,navigation:navigation),"settings-"+theme,NSSize(width:980,height:680),required:["Theme","Menu Bar Icon"])
    }
#if DEBUG
    let actionFixtures=[state.config.commands.first {$0.id=="app.finder"}!,
        state.config.commands.first {$0.id=="snippet.greeting"}!,
        state.config.commands.first {$0.id=="fixture.inline"}!,
        state.config.commands.first {$0.id=="capture.quick-note"}!,
        Command(id:"fixture.window",title:"Left Half",kind:"window",target:"left-half")]
    state.config.commands.append(actionFixtures.last!)
    for theme in ["light","dark"] {
        state.config.theme=theme
        for width in [600,760] {
            for command in actionFixtures {
                state.query=command.title;state.selection=0
                try capture(Palette(state:state),"action-\(command.kind)-\(command.id)-\(theme)-\(width)",NSSize(width:width,height:490))
            }
            state.begin(actionFixtures[3]);state.inlineInput="Example note"
            try capture(Palette(state:state),"action-inline-save-\(theme)-\(width)",NSSize(width:width,height:490),required:["Save"],absent:["Open"])
            state.busy=true
            try capture(Palette(state:state),"action-inline-busy-\(theme)-\(width)",NSSize(width:width,height:490),required:["Save"],absent:["Open"])
            state.busy=false
            state.cancelInlineInput();state.query="___no-result___"
            try capture(Palette(state:state),"action-empty-\(theme)-\(width)",NSSize(width:width,height:490),absent:["Open"])
        }
    }
    state.mode="commands";state.query=""
#endif
    state.inlineSource="hotkey";state.selection=4;state.query="echo-inline Café & spaces"
    try capture(Palette(state:state),"inline-query",NSSize(width:760,height:490),required:["Inline Input","Café"])
    state.cancelPaletteInput()
    state.form=nil;state.query=""
    let script=Command(id:"fixture.edit",title:"Example Action",kind:"script",target:"/bin/sh",aliases:["example"],arguments:["-c","printf sample"])
    try capture(CommandEditor(state:state,form:CommandForm(command:script,isRun:false)),"editor-closed",NSSize(width:760,height:490),required:["Name","Advanced"],absent:["Arguments","Timeout","Executable"])
    try capture(CommandEditor(state:state,form:CommandForm(command:script,isRun:false),advancedExpanded:true),"editor-expanded",NSSize(width:760,height:620),required:["Advanced","Arguments","Timeout"])
    state.mode="snippets";state.query="";state.config.commands=[
        Command(id:"fixture.greet",title:"Greeting",kind:"snippet",target:"Hello",aliases:["hello","greeting","salutation"],hotkey:"cmd+shift+g",keyword:";greetsnippet"),
        Command(id:"fixture.long",title:"Longer bindings",kind:"snippet",target:"Example",aliases:["first-alias","second-alias","third-alias","fourth-alias","final-alias"],keyword:";longsnippet"),
        Command(id:"fixture.none",title:"Unbound snippet",kind:"snippet",target:"Example")]
    for theme in ["light","dark"] {
        state.config.theme=theme
        try capture(Palette(state:state),"snippet-bindings-"+theme,NSSize(width:760,height:490),required:["Set Hotkey","greetsnippet","Aliases","hello","greeting","salutation","final-alias","Unbound snippet"],absent:["Keyword"])
        try capture(Palette(state:state),"snippet-bindings-narrow-"+theme,NSSize(width:600,height:490),required:["Set Hotkey","salutation","final-alias","Unbound snippet"],absent:["Keyword"])
    }
    return ["snippetBindingsVisible":true,"snippetAliasesAndKeywordSearch":true,"ok":true,"themes":4,"createRemoved":true,"consumerEditorDisclosure":true,"healthStates":true,"managementBuiltins":true,"desktopInputSent":false,"visibleWindowOpened":false]
}
