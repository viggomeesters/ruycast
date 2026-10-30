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
                try require(state.selectedCommand?.id==command.id,"Primary action fixture selection changed")
                try capture(Palette(state:state),"action-\(command.kind)-\(command.id)-\(theme)-\(width)",NSSize(width:width,height:490))
            }
            state.begin(actionFixtures[3]);state.inlineInput="Example note"
            try capture(Palette(state:state),"action-inline-save-\(theme)-\(width)",NSSize(width:width,height:490),required:["Save"],absent:["Open"])
            let completedBefore=state.counts
            state.busy=true;state.activateSelection()
            try require(state.counts==completedBefore && state.inlineInput=="Example note","Busy primary action changed or submitted input")
            try capture(Palette(state:state),"action-inline-busy-\(theme)-\(width)",NSSize(width:width,height:490),required:["Save"],absent:["Open"])
            state.busy=false
            state.cancelInlineInput();state.query="___no-result___"
            try require(state.selectedCommand==nil,"Empty action fixture returned a result")
            try capture(Palette(state:state),"action-empty-\(theme)-\(width)",NSSize(width:width,height:490),absent:["Open"])
        }
    }
    state.mode="processes";state.systemServices.cancel()
    let process=Command(id:"system.kill:fixture",title:"Fixture Process",kind:"builtin",target:"system.kill:fixture")
    state.applySystemResults([process],resources:[:],preserveSelection:false);state.selection=0
    for theme in ["light","dark"] {
        state.config.theme=theme
        try capture(Palette(state:state),"action-process-"+theme,NSSize(width:600,height:490),required:["Kill"])
        try require(state.selectedCommand?.id==process.id,"Synthetic process selection changed during rendering")
    }
    state.mode="commands";state.query=""
#endif
    state.inlineSource="hotkey";state.selection=4;state.query="echo-inline Café & spaces"
    try require(state.inlineCommand?.id=="fixture.inline" && state.inlineInput=="Café & spaces" && state.form==nil && state.inlineSource=="launcher" && state.selection==0,"Alias argument did not enter inline mode")
    try capture(Palette(state:state),"inline-query",NSSize(width:760,height:490),required:["Inline Input","Café"])
    state.activateSelection();state.activateSelection()
    let end=Date().addingTimeInterval(8);while state.busy,Date()<end {pump(0.02)}
    try require(!state.busy && state.message.isEmpty && state.counts["fixture.inline"]==1,"Inline Enter failed or duplicated execution")
    state.cancelPaletteInput();try require(state.inlineCommand==nil && state.query=="echo-inline","Escape did not restore search")
    state.begin(state.config.resolve("echo-inline")!);state.inlineInput="First line";state.expandInlineInput()
    try require(state.inlineCommand==nil && state.form?.values["input"]=="First line","Expanded input lost text")
    state.form=nil;state.query=""
    let script=Command(id:"fixture.edit",title:"Example Action",kind:"script",target:"/bin/sh",aliases:["example"],arguments:["-c","printf sample"])
    try capture(CommandEditor(state:state,form:CommandForm(command:script,isRun:false)),"editor-closed",NSSize(width:760,height:490),required:["Name","Advanced"],absent:["Arguments","Timeout","Executable"])
    try capture(CommandEditor(state:state,form:CommandForm(command:script,isRun:false),advancedExpanded:true),"editor-expanded",NSSize(width:760,height:620),required:["Advanced","Arguments","Timeout"])
    state.clips=[Clip(text:String(repeating:"Readable clipboard text. ",count:80),source:"Fixture")];state.mode="clipboard"
    try capture(Palette(state:state),"clipboard-large",NSSize(width:1080,height:700),required:["Readable clipboard text"])
    try capture(Palette(state:state),"clipboard-small",NSSize(width:600,height:400))
    let cancellation=try checkClipboardPreviewCancellation(state.clipboardStore)
    state.clips=[]
    try capture(Palette(state:state),"clipboard-cleared",NSSize(width:1080,height:700),required:["Select an item"],absent:["Readable clipboard text"])
    let screen=NSRect(x:-1920,y:0,width:1920,height:1080),normal=PaletteGeometry.frame(in:screen),clip=PaletteGeometry.frame(in:screen,mode:"clipboard")
    try require(clip.width>normal.width && clip.height>normal.height && screen.contains(clip),"Clipboard geometry did not grow safely")
    try require(AppBrand.menuIcon().isTemplate && !AppBrand.menuIcon(healthy:false).isTemplate,"Health glyph states are missing")
    try require(!AppState.keyboardHealth(hyperEnabled:true,snippetsEnabled:false,trusted:true,raycastRunning:true) && !AppState.keyboardHealth(hyperEnabled:false,snippetsEnabled:true,trusted:false,raycastRunning:false) && AppState.keyboardHealth(hyperEnabled:false,snippetsEnabled:false,trusted:false,raycastRunning:true),"Keyboard health misses conflicts or disabled preferences")
    state.configError="Fixture configuration failure";try require(!state.runtimeHealthy,"Runtime health ignored config failure");state.configError=nil
    try require(["builtin.settings","builtin.open-config","builtin.reload-config","builtin.system-monitor"].allSatisfy{id in state.systemCommands.contains{$0.id==id}},"Launcher management actions missing")
    var catalog=state.config;catalog.commands.append(Command(id:"existing.monitor",title:"My Monitor",kind:"builtin",target:"system-monitor"));state.config=catalog
    try require(!state.systemCommands.contains {$0.target=="system-monitor"},"Duplicate built-in target in catalog")
    state.mode="commands";state.query=""
    let dateCommand=Command(id:"fixture.date",title:"Task Deadline",kind:"script",target:"/bin/echo",arguments:["{input}"],inputPrompt:"Deadline (YYYY-MM-DD of clear)")
    state.config.commands.append(dateCommand);state.begin(dateCommand);state.inlineInput="morgen"
    try require(state.inlineDatePreview?.isValid==true,"Natural date preview missing")
    let frozen=state.inlineDatePreview!
    try capture(Palette(state:state),"date-valid",NSSize(width:760,height:480),required:[frozen.label],absent:["YYYY-MM-DD"])
    state.inlineInput="in 3";try require(state.inlineDatePreview?.isValid==false,"Partial date accepted")
    state.submitInlineInput();try require(!state.busy && state.inlineCommand?.id==dateCommand.id,"Invalid date executed or discarded input")
    try capture(Palette(state:state),"date-invalid",NSSize(width:760,height:480),required:["Datum niet herkend"])
    state.message="";state.inlineInput="clear";try capture(Palette(state:state),"date-clear",NSSize(width:760,height:480),required:["Datum wissen"])
    let savedDate=root.appendingPathComponent("date-result.txt")
    var dateWriter=dateCommand;dateWriter.id="fixture.date-write";dateWriter.target="/bin/sh"
    dateWriter.arguments=["-c","printf '%s' \"$1\" > \"$2\"","date-preview","{input}",savedDate.path]
    state.config.commands.append(dateWriter);state.cancelInlineInput();state.begin(dateWriter);state.inlineInput="morgen"
    let frozenDate=TaskDateInput.resolve("morgen",now:Date(timeIntervalSince1970:946728000))
    state.inlineDatePreview=frozenDate;state.submitInlineInput();state.submitInlineInput()
    let dateEnd=Date().addingTimeInterval(8);while state.busy,Date()<dateEnd {pump(0.02)}
    try require(!state.busy && state.message.isEmpty && state.counts[dateWriter.id]==1,"Date Enter failed or ran twice")
    try require(try String(contentsOf:savedDate,encoding:.utf8)==frozenDate.value,"Enter did not send the displayed date")
    state.cancelInlineInput();state.message=""
    state.mode="snippets";state.query="";state.config.commands=[
        Command(id:"fixture.greet",title:"Greeting",kind:"snippet",target:"Hello",aliases:["hello","greeting","salutation"],hotkey:"cmd+shift+g",keyword:";greetsnippet"),
        Command(id:"fixture.long",title:"Longer bindings",kind:"snippet",target:"Example",aliases:["first-alias","second-alias","third-alias","fourth-alias","final-alias"],keyword:";longsnippet"),
        Command(id:"fixture.none",title:"Unbound snippet",kind:"snippet",target:"Example")]
    for theme in ["light","dark"] {
        state.config.theme=theme
        try capture(Palette(state:state),"snippet-bindings-"+theme,NSSize(width:760,height:490),required:["greetsnippet","Aliases","hello","greeting","salutation","final-alias","Unbound snippet"],absent:["Keyword"])
        try capture(Palette(state:state),"snippet-bindings-narrow-"+theme,NSSize(width:600,height:490),required:["salutation","final-alias","Unbound snippet"],absent:["Keyword"])
    }
    state.query="salutation"
    try require(state.visibleCommands.map(\.id)==["fixture.greet"],"Alias search lost the snippet")
    state.query="longsnippet"
    try require(state.selectedCommand?.id=="fixture.long","Keyword search lost the snippet")
    state.query="";state.selection=0;state.moveSelection(1)
    try require(state.selectedCommand?.id=="fixture.long","Arrow selection changed after rendering bindings")
    return ["snippetBindingsVisible":true,"snippetAliasesAndKeywordSearch":true,"ok":true,"themes":4,"createRemoved":true,"consumerEditorDisclosure":true,"inlineAliasAndEnter":true,"inlineSubmitOnce":true,"expandedInputPreserved":true,"clipboardLargeAndSmall":true,"clearedPreviewContentRemoved":true,"previewCancellation":cancellation,"healthStates":true,"managementBuiltins":true,"desktopInputSent":false,"visibleWindowOpened":false]
}
