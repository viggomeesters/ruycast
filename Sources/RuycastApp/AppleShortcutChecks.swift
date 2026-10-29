import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

@MainActor func checkAppleShortcuts(_ output:String?=nil)throws->[String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-shortcuts-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String)throws {if !value {throw LauncherError(message)}}
    func pump() {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}
    func wait(_ ready:()->Bool)throws {let end=Date().addingTimeInterval(5);while !ready(),Date()<end {pump()};try require(ready(),"Shortcuts fixture timed out.")}
    let a="11111111-1111-4111-8111-111111111111",b="22222222-2222-4222-8222-222222222222",c="33333333-3333-4333-8333-333333333333"
    let text="Focus (\(a))\nFocus (\(b))\nCafé 🌿 $(ignored) (\(c.uppercased()))\n"
    let commands=try AppleShortcuts.parse(Data(text.utf8))
    try require(commands.count==3 && commands[0].id != commands[1].id && commands.allSatisfy(AppleShortcuts.recognizes),"Shortcut UUID parsing lost duplicate or Unicode names.")
    try require(commands[2].arguments==["run",c] && commands[2].title=="Café 🌿 $(ignored)","Display name became executable arguments.")
    for bad in ["Missing identifier",text+"Focus (\(a))\n","Multiline\nFocus (\(a))\n","Focus (not-a-uuid)"] {
        do {_=try AppleShortcuts.parse(Data(bad.utf8));throw LauncherError("Malformed discovery succeeded.")}
        catch {try require(error.localizedDescription != "Malformed discovery succeeded.","Malformed discovery succeeded.")}
    }
    try require(try AppleShortcuts.parse(Data()).isEmpty,"Valid empty discovery failed.")
    let oversized=(0...1000).map {_ in "Item (\(UUID().uuidString))"}.joined(separator:"\n")
    do {_=try AppleShortcuts.parse(Data(oversized.utf8));throw LauncherError("Discovery cap failed.")} catch {try require(error.localizedDescription.contains("1,000"),"Discovery cap failed.")}
    for mutation in [Command(id:commands[0].id,title:"Wrong",kind:"script",target:"/bin/echo",arguments:["run",a]),Command(id:commands[0].id,title:"Wrong",kind:"script",target:"/usr/bin/shortcuts",arguments:["run","Focus"])] {
        try require(!AppleShortcuts.recognizes(mutation),"Shortcut route accepted a mismatched execution tuple.")
    }
    // Disposable child probes never invoke Shortcuts or execute user workflows.
    func child(_ args:[String],timeout:TimeInterval=1,limit:Int=65_536,cancel:Bool=false)throws->Result<Data,Error> {
        let session=SystemSearchSession(),result=LockedValue<Result<Data,Error>?>(nil)
        session.submit(on:DispatchQueue(label:"shortcut.child-fixture")) {ticket in
            result.update {$0=Result {try AppleShortcuts.list(ticket,executable:"/bin/sh",arguments:args,timeout:timeout,byteLimit:limit)}}
        }
        if cancel {Thread.sleep(forTimeInterval:0.03);session.cancel()}
        try wait {result.read {$0 != nil}};return result.read {$0!}
    }
    let childData=try child(["-c","printf '%s' \"$1\"","fixture",text]).get()
    try require(String(data:childData,encoding:.utf8)==text,"Bounded discovery child lost output.")
    for (args,budget,limit) in [(["-c","exec /bin/sleep 10"],0.05,65_536),(["-c","while :; do printf 'xxxxxxxxxxxxxxxx'; done"],1.0,128),(["-c","while :; do printf 'xxxxxxxxxxxxxxxx' >&2; done"],1.0,128),(["-c","exit 1"],1.0,65_536),(["-c","/bin/sleep 1 & exit 0"],0.05,65_536)] {
        let start=ProcessInfo.processInfo.systemUptime
        switch try child(args,timeout:budget,limit:limit) {case .success:throw LauncherError("Discovery child pressure/deadline failed.");case .failure:break}
        try require(ProcessInfo.processInfo.systemUptime-start<1,"Discovery cleanup exceeded its bound.")
    }
    switch try child(["-c","exec /bin/sleep 10"],cancel:true) {case .success:throw LauncherError("Cancelled list succeeded.");case .failure:break}
    let calls=LockedValue(0),clock=LockedValue<TimeInterval>(0),failure=LockedValue(false),delay=LockedValue(false)
    var effects=SystemServices.Effects()
    effects.shortcutClock={clock.read {$0}}
    effects.shortcuts={ticket in
        calls.update {$0+=1}
        while delay.read({$0}),ticket.current {Thread.sleep(forTimeInterval:0.002)}
        if failure.read({$0}) {throw LauncherError("Could not load shortcuts. Check Shortcuts, then refresh.")}
        return commands
    }
    let service=SystemServices(effects:effects),state=try AppState(root:root,headless:true,systemServices:service)
    state.foregroundPID={nil}
    try require(state.searchCommands("opdrachten").filter {$0.target=="apple-shortcuts"}.count==1,"Shortcut search has no unique entry point.")
    var reply:[String:Any]?
    func query(_ value:String="")throws->[[String:Any]] {
        reply=nil;state.handle(["op":"search","mode":"shortcuts","query":value]) {reply=$0};try wait {reply != nil}
        try require(reply?["ok"] as? Bool==true,"Shortcut CLI search failed.");return reply?["result"] as? [[String:Any]] ?? []
    }
    try require(try query().count==3 && query("café").count==1 && calls.read {$0}==1,"Discovery cache/query failed.")
    var configured=commands[0];configured.enabled=true;configured.title="Personal Focus";configured.aliases=["work"];configured.hotkey="cmd+ctrl+f"
    state.config.commands.append(configured)
    try require(try query("work").first?["hotkey"] as? String==configured.hotkey,"Alias/hotkey override disappeared from discovery.")
    state.config.commands[ state.config.commands.count-1 ].enabled=false
    try require(try query().count==2 && query("work").isEmpty,"Disabled shortcut was resurrected.")
    state.config.commands[ state.config.commands.count-1 ].enabled=true
    try Files.save(state.config,to:state.configURL)
    let restarted=try AppState(root:root,headless:true,systemServices:service)
    try require(restarted.resolveCommand("work")==configured && restarted.serviceCommands.isEmpty,"Saved shortcut depends on an old discovery cache.")
    let executions=LockedValue<[Command]>([]),payload=LockedValue<Data>(try JSONSerialization.data(withJSONObject:["ok":true,"result":["exitCode":0,"stdout":"{\"ok\":true,\"detail\":\"PRIVATE-SHORTCUT-OUTPUT\",\"choices\":{}}","stderr":"PRIVATE-SHORTCUT-ERROR"]]))
    state.scriptExecution={command in executions.update {$0.append(command)};return payload.read {$0}}
    var hides=0;state.hidePalette={hides+=1}
    func execute(_ id:String,source:String="cli")throws->[String:Any] {
        reply=nil;state.execute(id,dryRun:false,openResult:true,source:source) {reply=$0};try wait {reply != nil};return reply!
    }
    try require(try execute("work")["ok"] as? Bool==true && hides==0 && executions.read {$0.last?.arguments}==["run",a] && state.detail==nil && state.scriptChoices==nil,"Shortcut CLI executed by name, hid palette, or exposed its output.")
    try require(try execute(commands[1].id,source:"launcher")["ok"] as? Bool==true && hides==1 && state.counts[commands[1].id]==1,"Launcher shortcut lost hide/usage behavior.")
    for error in ["PRIVATE-SHORTCUT-ERROR","Script timed out after 0.1 seconds."] {
        payload.update {$0=try! JSONSerialization.data(withJSONObject:["ok":false,"error":error,"stderr":"PRIVATE-SHORTCUT-ERROR","stdout":"PRIVATE-SHORTCUT-OUTPUT"])}
        let response=try execute("work")
        try require(response["ok"] as? Bool==false && (response["error"] as? String)==(error.hasPrefix("Script timed") ? AppleShortcuts.timeoutFailure:AppleShortcuts.failure),"Shortcut failure leaked data or claimed cancellation.")
    }
    payload.update {$0=Data("PRIVATE-SHORTCUT-OUTPUT".utf8)}
    try require(try execute("work")["error"] as? String==AppleShortcuts.failure,"Invalid runner data leaked through catch.")
    state.scriptExecution={_ in throw LauncherError("PRIVATE-SHORTCUT-ERROR")}
    try require(try execute("work")["error"] as? String==AppleShortcuts.failure,"Thrown shortcut error leaked.")
    state.commandLog.flush()
    try require(!state.commandLog.entries(commandID:nil,errorsOnly:false,limit:500).isEmpty && state.commandLog.storageError==nil,"Shortcut log privacy assertion has no entries.")
    let savedLog=try String(contentsOf:state.commandLog.url,encoding:.utf8)
    try require(!savedLog.contains("PRIVATE-SHORTCUT"),"Shortcut output reached persisted logs.")
    let log=String(decoding:try Files.encode(state.commandLog.entries(commandID:nil,errorsOnly:false,limit:500)),as:UTF8.self)
    try require(!log.contains("PRIVATE-SHORTCUT"),"Shortcut output reached command logs.")
    // Ordinary scripts retain their pre-existing UI-envelope behavior.
    let ordinary=Command(id:"fixture.ordinary",title:"Ordinary",kind:"script",target:"/bin/echo")
    state.config.commands.append(ordinary)
    state.scriptExecution={_ in try JSONSerialization.data(withJSONObject:["ok":true,"result":["stdout":"{\"ok\":true,\"detail\":\"Ordinary report\"}"]])}
    try require(try execute(ordinary.id)["ok"] as? Bool==true && state.detail=="Ordinary report","Ordinary script UI behavior changed.")
    state.detail=nil
    clock.update {$0=31};_=try query();try require(calls.read {$0}==2,"Discovery cache never expired.")
    state.mode="shortcuts";try wait {!state.shortcutSearchBusy && state.results.count==3}
    failure.update {$0=true};state.refreshNativeResults();try wait {!state.message.isEmpty && !state.shortcutSearchBusy}
    failure.update {$0=false};state.refreshNativeResults();try wait {state.message.isEmpty && !state.shortcutSearchBusy && state.results.count==3}
    delay.update {$0=true};clock.update {$0=62};state.refreshNativeResults();try require(state.shortcutSearchBusy,"Shortcut loading state missing.")
    state.cancelNativeSearch();delay.update {$0=false};try require(!state.shortcutSearchBusy,"Shortcut cancellation retained loading.")
    state.refreshNativeResults();try wait {!state.shortcutSearchBusy && state.results.count==3}
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        let controller=PaletteController(state:state);state.showToast=nil;state.paletteVisible=true
        state.scriptExecution={command in executions.update {$0.append(command)};return try JSONSerialization.data(withJSONObject:["ok":true])}
        controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
        func field(_ view:NSView)->SearchInput? {if let result=view as? SearchInput {return result};return view.subviews.compactMap {field($0)}.first}
        guard let host=controller.panel.contentView,let input=field(host) else {throw LauncherError("Shortcut native search field missing.")}
        try require(input.accessibilityLabel()=="Search Apple Shortcuts…","Shortcut search accessibility name stale.")
        state.selection=0;controller.restoreSearchFocus();for _ in 0..<5 {pump()}
        guard let editor=controller.panel.firstResponder as? NSTextView else {throw LauncherError("Shortcut native editor missing.")}
        let before=executions.read {$0.count}
        editor.keyDown(with:NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:controller.panel.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!)
        try wait {!state.busy};try require(executions.read {$0.count}==before+1 && !controller.panel.isVisible,"Shortcut Return did not execute exactly once in a hidden fixture panel.")
        for dark in [false,true] {for width in [600,760] {for variant in ["normal","duplicate","long","empty","error","loading"] {
            var rows=commands;rows[2].title="Daily Notes"
            if variant=="long" {rows[1].title="A Very Long Shortcut Name for a Daily Focus Routine and Workspace Setup"}
            if variant=="duplicate" {rows=Array(rows.suffix(2));rows[0].title="Focus";rows[1].title="Focus"}
            if ["empty","error","loading"].contains(variant) {rows=[]}
            state.message=variant=="error" ? "Could not load shortcuts. Check Shortcuts, then refresh.":"";state.paletteSession.nativeSearchBusy=variant=="loading"
            state.applySystemResults(rows,resources:[:],preserveSelection:false);state.selection=0
            state.config.theme=dark ? "dark":"light";controller.panel.setContentSize(NSSize(width:width,height:490));controller.panel.contentView?.layoutSubtreeIfNeeded();for _ in 0..<12 {pump()}
            guard let view=controller.panel.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw LauncherError("Shortcut render missing.")}
            view.cacheDisplay(in:view.bounds,to:bitmap);guard let data=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Shortcut PNG missing.")}
            try data.write(to:directory.appendingPathComponent("shortcuts-\(dark ? "dark":"light")-\(width)-\(variant).png"))
        }}}
        controller.hide();try require(!controller.panel.isVisible,"Shortcut fixture made its panel visible.")
    }
    return ["ok":true,"realWorkflowExecution":false,"realDesktopEffects":false,"uuidIdentityAndDuplicateNames":true,"boundedChildPipesTimeoutAndCancellation":true,"cacheExpiryAndRecovery":true,"configurationOverridesAndRestart":true,"privateOutputDiscardedBeforeUIAndLogs":true,"ordinaryScriptEnvelopePreserved":true,"cliAndNativeReturnParity":output != nil,"nativeWorkflowUIAccepted":false]
}
