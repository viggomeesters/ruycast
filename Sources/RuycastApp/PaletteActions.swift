import SwiftUI
import AppKit
import LauncherCore

struct PaletteActionItem:Identifiable {
    let id:String
    let title:String
    let symbol:String
    var section:Int=0
}
extension AppState {
    var actionItems:[PaletteActionItem] {
        var items:[PaletteActionItem]=[]
        if mode=="clipboard",filteredClips.indices.contains(selection) {
            items += [PaletteActionItem(id:"paste",title:"Paste",symbol:"doc.on.clipboard"),PaletteActionItem(id:"copy-clip",title:"Copy",symbol:"doc.on.doc"),PaletteActionItem(id:"delete-clip",title:"Delete",symbol:"trash")]
        } else if let command=selectedCommand {
            items.append(PaletteActionItem(id:"open",title:command.target.hasPrefix("system.kill:") ? "Kill":command.kind=="snippet" ? "Insert Snippet":command.id.hasPrefix("capture.") ? "Save…":command.kind=="script" || command.kind=="window" ? "Run":"Open",symbol:"return"))
            if ["snippet","url","path"].contains(command.kind) {items.append(PaletteActionItem(id:"copy-template",title:command.kind=="snippet" ? "Copy Template":"Copy Link",symbol:"doc.on.doc"))}
            if !command.id.hasPrefix("system.") {
            items.append(PaletteActionItem(id:"edit",title:"Edit",symbol:"pencil"))
            items.append(PaletteActionItem(id:"bindings",title:"Alias & Shortcut…",symbol:"slider.horizontal.3"))
            items.append(PaletteActionItem(id:"disable",title:"Disable",symbol:"pause.circle"))
            if config.commands.contains(where:{$0.id==command.id}) {items.append(PaletteActionItem(id:"remove",title:"Remove",symbol:"trash"))}
            }
        }
        items.append(PaletteActionItem(id:"settings",title:"Settings",symbol:"gearshape",section:2))
        return items
    }
    func moveActionSelection(_ delta:Int) {actionSelection=max(0,min(max(0,actionItems.count-1),actionSelection+delta))}
    func activateActionSelection() {let items=actionItems;guard items.indices.contains(actionSelection) else {return};performMenuAction(items[actionSelection].id)}
    func cancelPaletteInput() {if actionsOpen {actionsOpen=false} else {hidePalette?()}}
    func performMenuAction(_ id:String) {
        guard actionItems.contains(where:{$0.id==id}) else {return}
        let command=selectedCommand
        let clip=filteredClips.indices.contains(selection) ? filteredClips[selection]:nil
        actionsOpen=false
        switch id {
        case "paste":if let clip {do {try copyClip(clip.id,paste:true)} catch {message=error.localizedDescription}}
        case "copy-clip":if let clip {do {try copyClip(clip.id)} catch {message=error.localizedDescription}}
        case "delete-clip":if let clip {handle(["op":"clipboard.delete","id":clip.id]) {_ in}}
        case "open":if let command {begin(command)}
        case "copy-template":if let command {copyText(command.target)}
        case "edit":if let command {message="";form=CommandForm(command:command,isRun:false)}
        case "bindings":if let command {message="";var edit=CommandForm(command:command,isRun:false);edit.bindingsOnly=true;edit.bindingBase=command;edit.isNew = !config.commands.contains(where:{$0.id==command.id});form=edit}
        case "disable":if var command {command.enabled=false;saveCommand(command)}
        case "remove":pendingRemoval=command
        case "settings":hidePalette?();showManagement?()
        default:break
        }
    }
}
struct PaletteActions:View {
    @ObservedObject var state:AppState
    var body:some View {
        let items=state.actionItems
        VStack(alignment:.leading,spacing:3) {
            if state.mode != "clipboard",let command=state.selectedCommand {Text(command.title).font(.caption).foregroundStyle(.secondary).lineLimit(1).padding(8)}
            ForEach(Array(items.enumerated()),id:\.element.id) {index,item in
                if index>0,item.section != items[index-1].section {Divider().padding(.vertical,4)}
                Button {state.actionSelection=index;state.performMenuAction(item.id)} label:{
                    Label(item.title,systemImage:item.symbol).font(.system(size:13)).frame(maxWidth:.infinity,alignment:.leading).padding(.horizontal,8).padding(.vertical,6)
                        .background(index==state.actionSelection ? Color.primary.opacity(0.09):Color.clear,in:RoundedRectangle(cornerRadius:5))
                }.buttonStyle(.plain).accessibilityAddTraits(index==state.actionSelection ? .isSelected:[])
            }
        }.padding(8).frame(width:240)
            .background(ActionMenuKeys(state:state).frame(width:0,height:0))
            .onDisappear {state.restoreSearchFocus?()}
    }
}
struct ActionMenuKeys:NSViewRepresentable {
    let state:AppState
    func makeNSView(context:Context)->ActionMenuKeyInput {let view=ActionMenuKeyInput();view.route={ [weak state] event in state?.handleActionMenuKey(event) ?? false };return view}
    func updateNSView(_ view:ActionMenuKeyInput,context:Context) {view.route={ [weak state] event in state?.handleActionMenuKey(event) ?? false }}
}
final class ActionMenuKeyInput:NSView {
    var route:((NSEvent)->Bool)?
    override var acceptsFirstResponder:Bool {true}
    override func viewDidMoveToWindow() {super.viewDidMoveToWindow();guard let window else {return};DispatchQueue.main.async { [weak self,weak window] in if let self,let window {window.makeFirstResponder(self)}}}
    override func keyDown(with event:NSEvent) {if route?(event) != true {super.keyDown(with:event)}}
    override func performKeyEquivalent(with event:NSEvent)->Bool {route?(event)==true || super.performKeyEquivalent(with:event)}
    static func contains(in view:NSView)->Bool {view is ActionMenuKeyInput || view.subviews.contains(where:{contains(in:$0)})}
}
extension AppState {
    func handleActionMenuKey(_ event:NSEvent)->Bool {
        guard actionsOpen else {return false}
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,event.charactersIgnoringModifiers?.lowercased()=="k" {actionsOpen=false;return true}
        guard event.modifierFlags.intersection([.command,.control,.option]).isEmpty else {return false}
        switch event.keyCode {
        case 125:moveActionSelection(1)
        case 126:moveActionSelection(-1)
        case 36,76:activateActionSelection()
        case 53:actionsOpen=false
        case 48:moveActionSelection(event.modifierFlags.contains(.shift) ? -1:1)
        default:return false
        };return true
    }
}

extension AppState {
    func copyText(_ text:String) {
        let board=NSPasteboard.general;board.clearContents()
        if board.setString(text,forType:.string) {ignoreOwnClipboardChange();showToast?("Copied",true)}
        else {message="Could not copy text."}
    }
    func pasteText(_ text:String,cursor:Int?=nil) throws {
        guard windows.trusted else {throw LauncherError("Allow Ruycast in System Settings → Privacy & Security → Accessibility to insert text.")}
        guard !headless,let pid=previousPID,let app=NSRunningApplication(processIdentifier:pid) else {throw LauncherError("Choose an app to insert the snippet into.")}
        let board=NSPasteboard.general;board.clearContents()
        guard board.setString(text,forType:.string) else {throw LauncherError("Could not copy text.")}
        ignoreOwnClipboardChange();hidePalette?();app.activate(options:[])
        DispatchQueue.main.asyncAfter(deadline:.now()+0.15) { [weak self] in
            guard NSWorkspace.shared.frontmostApplication?.processIdentifier==pid else {self?.showToast?("Copied · target app did not activate",false);return}
            for down in [true,false] {let event=CGEvent(keyboardEventSource:nil,virtualKey:9,keyDown:down);event?.flags = .maskCommand;event?.setIntegerValueField(.eventSourceUserData,value:HyperKey.generatedTag);event?.post(tap:.cghidEventTap)}
            if let cursor {
                let steps=(text as NSString).substring(from:min(cursor,text.utf16.count)).count
                DispatchQueue.main.asyncAfter(deadline:.now()+0.08) {
                    guard NSWorkspace.shared.frontmostApplication?.processIdentifier==pid else {return}
                    for _ in 0..<min(steps,10000) {for down in [true,false] {let event=CGEvent(keyboardEventSource:nil,virtualKey:123,keyDown:down);event?.flags=[];event?.setIntegerValueField(.eventSourceUserData,value:HyperKey.generatedTag);event?.post(tap:.cghidEventTap)}}
                }
            }
        }
    }
}
