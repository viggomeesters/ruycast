import AppKit
import SwiftUI
import LauncherCore

/// Dispatches only to our invisible native views; no global input or visible window.
func checkActionsMenu(output:String?=nil) throws -> [String:Any] {
    _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-actions-"+String(UUID().uuidString.prefix(8)))
    defer {try? FileManager.default.removeItem(at:root)}
    let state=try AppState(root:root,headless:true)
    state.config.commands=featureFixture();state.selection=4
    let controller=PaletteController(state:state)
    state.paletteVisible=true
    controller.panel.contentView?.layoutSubtreeIfNeeded()
    let popup=NSWindow(contentRect:NSRect(x:0,y:0,width:256,height:340),styleMask:.borderless,backing:.buffered,defer:false)
    popup.isReleasedWhenClosed=false
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    func event(_ key:UInt16,flags:NSEvent.ModifierFlags=[],characters:String="")->NSEvent {NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:0,windowNumber:popup.windowNumber,context:nil,characters:characters,charactersIgnoringModifiers:characters,isARepeat:false,keyCode:key)!}
    func pump() {for _ in 0..<10 {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.005))}}
    let original=state.selectedCommand!
    try require(controller.panel.performKeyEquivalent(with:event(40,flags:.command,characters:"k")) && state.actionsOpen && state.actionSelection==0,"Command K did not open at first action.")
    let host=NSHostingView(rootView:PaletteActions(state:state).frame(maxWidth:.infinity,maxHeight:.infinity).background(Color(nsColor:.windowBackgroundColor)));host.frame=NSRect(x:0,y:0,width:256,height:340);popup.contentView=host;host.layoutSubtreeIfNeeded();pump()
    guard let input=popup.firstResponder as? ActionMenuKeyInput else {throw LauncherError("Actions native input did not acquire first responder automatically.")}
    try require(ActionMenuKeyInput.contains(in:host),"Actions window was not recognized.")
    input.keyDown(with:event(125));input.keyDown(with:event(125))
    try require(state.actionSelection==2 && state.selection==4 && state.selectedCommand?.id==original.id,"Arrows moved underlying results.")
    input.keyDown(with:event(126))
    try require(state.actionSelection==1,"Up did not move within Actions.")
    for _ in 0..<20 {input.keyDown(with:event(125))}
    try require(state.actionSelection==state.actionItems.count-1,"Down did not clamp to last action.")
    for _ in 0..<20 {input.keyDown(with:event(126))}
    try require(state.actionSelection==0,"Up did not clamp to first action.")
    // Even if the field editor retains input during popover construction, it routes to Actions.
    let search=PaletteSearchField(text:Binding(get:{state.query},set:{state.query=$0}),placeholder:"Search",onMove:state.moveSelection,onSubmit:state.activateSelection,onCancel:state.cancelPaletteInput,onActions:{state.actionsOpen.toggle()})
    let delegate=PaletteSearchField.Coordinator(search)
    _=delegate.control(SearchInput(),textView:NSTextView(),doCommandBy:NSSelectorFromString("moveDown:"))
    try require(state.actionSelection==1 && state.selection==4,"Field-editor fallback moved results.")
    input.keyDown(with:event(125)) // Edit
    input.keyDown(with:event(36))
    try require(!state.actionsOpen && state.form?.command.id==original.id && state.form?.isRun==false,"Return did not execute highlighted Edit.")
    state.form=nil;state.actionsOpen=true;input.keyDown(with:event(53));pump()
    try require(!state.actionsOpen && state.paletteVisible && state.query.isEmpty && state.selection==4,"Escape closed palette or changed search state.")
    let responder=controller.panel.firstResponder
    let fieldEditor=(responder as? NSTextView)?.delegate as? SearchInput
    try require(responder is SearchInput || fieldEditor != nil,"Search focus was not restored.")
    state.actionsOpen=true
    try require(input.performKeyEquivalent(with:event(40,flags:.command,characters:"k")) && !state.actionsOpen,"Command K did not close Actions.")
    state.actionsOpen=true;state.actionSelection=state.actionItems.firstIndex(where:{$0.id=="remove"})!;state.activateActionSelection()
    state.query="Finder"
    try require(state.pendingRemoval?.id==original.id,"Removal lost its original command after dismissal.")
    state.pendingRemoval=nil;state.query="";state.selection=4
    state.actionsOpen=true;state.newCommand("snippet")
    try require(state.form?.isNew==true && state.form?.command.kind=="snippet","Mouse/keyboard shared action did not open editor.")
    state.form=nil;state.clips=[Clip(text:"fixture",source:"fixture")];state.mode="clipboard";state.actionsOpen=true
    try require(state.actionItems.prefix(3).map(\.id)==["paste","copy-clip","delete-clip"],"Clipboard action catalog changed.")
    state.actionSelection=2;input.keyDown(with:event(36))
    try require(state.clips.isEmpty && !state.actionsOpen,"Clipboard menu Return did not delete selected fixture item.")
    if let output {
        let directory=URL(fileURLWithPath:output);try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        state.mode="commands";state.selection=4;state.actionsOpen=true
        for dark in [false,true] {
            host.appearance=NSAppearance(named:dark ? .darkAqua:.aqua)
            for selected in [0,2] {
                state.actionSelection=selected;pump();host.layoutSubtreeIfNeeded()
                guard let bitmap=host.bitmapImageRepForCachingDisplay(in:host.bounds) else {throw LauncherError("Actions preview unavailable.")};host.cacheDisplay(in:host.bounds,to:bitmap)
                try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("actions-\(dark ? "dark":"light")-\(selected).png"))
            }
        }
    }
    state.paletteVisible=false;state.actionsOpen=false
    return ["ok":true,"commandKOpensAtFirstAction":true,"nativeMenuFirstResponder":true,"arrowsStayInMenu":true,"fieldEditorFallback":true,"returnExecutesHighlightedAction":true,"escapeRestoresSearchFocus":true,"commandKTogglesClosed":true,"clipboardActions":true,"removalPayloadPreserved":true,"desktopInputSent":false,"visibleWindowOpened":false]
}
