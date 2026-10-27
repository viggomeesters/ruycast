import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

struct PaletteConfirmation {let command:Command;let removing:Bool}

struct Palette:View {
    @ObservedObject var state:AppState
    @Environment(\.colorScheme) private var colorScheme
    private var pendingConfirmation:PaletteConfirmation? {
        if let command=state.pendingRemoval {return PaletteConfirmation(command:command,removing:true)}
        return state.confirmationCommand.map {PaletteConfirmation(command:$0,removing:false)}
    }
    private var isClipboard:Bool {state.mode=="clipboard"}
    private var surface:Color {SettingsTheme(dark:(state.preferredColorScheme ?? colorScheme) == .dark,graphite:state.config.theme=="graphite").background}
    private var highlight:Color {Color.primary.opacity(colorScheme == .dark ? 0.085:0.065)}

    var body:some View {
        Group {
            if let form=state.form {CommandEditor(state:state,form:form).id(form.id)}
            else if let session=state.scriptChoices {ScriptChoiceView(state:state,session:session)}
            else if let detail=state.detail {
                VStack(alignment:.leading,spacing:12) {Button("Back") {state.detail=nil;state.mode="commands"}.buttonStyle(.plain);ScrollView {Text(detail).font(.system(size:13,design:.monospaced)).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)}}.padding(20)
            } else {content}
        }.frame(maxWidth:.infinity,maxHeight:.infinity).background(surface)
            .environment(\.colorScheme,state.preferredColorScheme ?? colorScheme)
            .preferredColorScheme(state.preferredColorScheme)
            .clipShape(RoundedRectangle(cornerRadius:18))
            .overlay(RoundedRectangle(cornerRadius:18).strokeBorder(Color.primary.opacity(0.12),lineWidth:1))
    }
    @ViewBuilder private var content:some View {
        VStack(spacing:0) {
            HStack(spacing:12) {
                Image(systemName:isClipboard ? "doc.on.clipboard":"command")
                    .font(.system(size:18,weight:.medium)).foregroundStyle(.tertiary).frame(width:22)
                if let command=state.inlineCommand {Text(command.title).font(.system(size:12,weight:.semibold)).lineLimit(1).padding(6).background(Color.primary.opacity(0.08),in:RoundedRectangle(cornerRadius:5))}
                PaletteSearchField(text:Binding(get:{state.inlineCommand==nil ? state.query:state.inlineInput},set:{if state.inlineCommand==nil {state.query=$0} else {state.inlineInput=$0}}),placeholder:state.inlineCommand.map {TaskDateInput.supports($0) ? "Datum, bijv. morgen of in 3 days":Templates.fields($0).first?.label ?? "Input"} ?? (isClipboard ? "Search clipboard…":state.mode=="processes" ? "Filter processes…":"Search for apps or commands…"),onMove:state.moveSelection,onSubmit:state.activateSelection,onCancel:state.cancelPaletteInput,onActions:{state.actionsOpen.toggle()},onTab:state.beginInlineSelection,onExpand:state.expandInlineInput).frame(height:28)
            }.padding(.horizontal,20).frame(height:62)

            if let preview=state.inlineDatePreview,state.inlineCommand.map(TaskDateInput.supports)==true {
                HStack(spacing:8) {
                    Image(systemName:preview.isValid ? "calendar":"calendar.badge.questionmark")
                    Text(preview.label)
                    Spacer()
                }.font(.system(size:13)).foregroundStyle(preview.isValid ? Color.primary:Color.secondary)
                    .padding(.horizontal,20).padding(.vertical,10)
                    .accessibilityElement(children:.combine)
            }
            PersistentLauncherResults(state:state)
            if let error=state.configError {
                Text("Config: \(error)").font(.caption).foregroundStyle(.red).padding(.horizontal,18).padding(.vertical,8)
            }
            if !state.message.isEmpty {
                Text(state.message).font(.caption).foregroundStyle(.orange).textSelection(.enabled).padding(.horizontal,18).padding(.vertical,8)
            }
            Divider().opacity(0.5)
            HStack(spacing:12) {
                if state.mode != "commands" {
                    Button {state.mode="commands"} label:{Label("Back",systemImage:"chevron.left")}.buttonStyle(.plain)
                }
                Spacer()
                if state.busy {ProgressView().controlSize(.small)}
                if isClipboard {
                    Button("Copy ⌘↵") {copy()}.keyboardShortcut(.return,modifiers:.command).buttonStyle(.plain)
                    Text("Paste ↵").foregroundStyle(.secondary)
                } else {Text(state.mode=="processes" ? "Kill ↵":"Open ↵").foregroundStyle(.secondary)}
                Button("Actions ⌘K") {state.actionsOpen.toggle()}.buttonStyle(.plain).keyboardShortcut("k",modifiers:.command)
                    .popover(isPresented:$state.actionsOpen,arrowEdge:.bottom) {PaletteActions(state:state)}
                Button {
                    if state.mode=="processes" {state.refreshProcessResults();return}
                    state.handle(["op":"reload"]) {r in
                        state.message=r["ok"] as? Bool == true ? "":r["error"] as? String ?? "Reload failed."
                    }
                } label:{Image(systemName:"arrow.clockwise").foregroundStyle(.secondary)}
                    .buttonStyle(.plain).help(state.mode=="processes" ? "Refresh processes":"Reload configuration").accessibilityLabel(state.mode=="processes" ? "Refresh processes":"Reload configuration")
            }.font(.system(size:11)).padding(.horizontal,16).frame(height:38)
        }
        .frame(maxWidth:.infinity,maxHeight:.infinity)
        .background(surface)
        .clipShape(RoundedRectangle(cornerRadius:18))
        .overlay(RoundedRectangle(cornerRadius:18).strokeBorder(Color.primary.opacity(0.12),lineWidth:1))
        .confirmationDialog(pendingConfirmation?.removing==true ? "Remove this command?":"Quit this process immediately? Unsaved changes may be lost.",isPresented:Binding(get:{pendingConfirmation != nil},set:{if !$0 {state.pendingRemoval=nil;state.confirmationCommand=nil}}),presenting:pendingConfirmation) { request in
            Button(request.removing ? "Remove":"Force Quit",role:.destructive) {
                state.pendingRemoval=nil;state.confirmationCommand=nil
                if request.removing {state.saveCommand(request.command,remove:true)} else {state.perform(request.command.id,confirmed:true)}
            }
            Button("Cancel",role:.cancel) {state.pendingRemoval=nil;state.confirmationCommand=nil}
        }
        .onExitCommand {state.cancelPaletteInput()}
    }
    @ViewBuilder private func commandIcon(_ cmd:Command)->some View {
        CommandIcon(command:cmd)
    }
    private func kindLabel(_ kind:String)->String {
        ["snippet":"Snippet","app":"Application","url":"Quicklink","path":"File or Folder","script":"Command","window":"Window","builtin":"Command"][kind] ?? "Command"
    }
    private func empty(_ title:String,symbol:String)->some View {
        VStack(spacing:12) {Image(systemName:symbol).font(.system(size:27));Text(title)}
            .foregroundStyle(.secondary).frame(maxWidth:.infinity,minHeight:170)
    }
    private func copy() {
        guard state.filteredClips.indices.contains(state.selection) else {return}
        do {try state.copyClip(state.filteredClips[state.selection].id)} catch {state.message=error.localizedDescription}
    }
    private func togglePause() {
        state.handle(["op":"clipboard.pause","paused":!state.paused]) {r in
            if r["ok"] as? Bool != true {state.message=r["error"] as? String ?? "Could not pause."}
        }
    }
}

final class PalettePanel:NSPanel {
    var actions:(()->Void)?
    override var canBecomeKey:Bool {true}
    override var canBecomeMain:Bool {false}
    override func performKeyEquivalent(with event:NSEvent)->Bool {
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,event.charactersIgnoringModifiers?.lowercased()=="k",let actions {actions();return true}
        return super.performKeyEquivalent(with:event)
    }
}
final class PaletteController {
    let state:AppState
    let panel:PalettePanel
    private var outside:Any?
    private var local:Any?
    private var toast:NSPanel?
    private var toastTimer:Timer?
    private var openTrace:PerformanceTrace.Span?
    init(state:AppState) {
        self.state=state
        panel=PalettePanel(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
        panel.actions={ [weak state] in guard state?.form==nil else {return};state?.actionsOpen.toggle() }
        panel.title="Ruycast";panel.isOpaque=false;panel.backgroundColor = .clear;panel.hasShadow=true
        panel.isMovableByWindowBackground=true
        panel.standardWindowButton(.closeButton)?.isHidden=true;panel.standardWindowButton(.miniaturizeButton)?.isHidden=true;panel.standardWindowButton(.zoomButton)?.isHidden=true
        panel.level = .floating;panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary];panel.isReleasedWhenClosed=false
        panel.contentView=TracedPaletteHost(rootView:Palette(state:state));panel.hidesOnDeactivate=false
        state.restoreSearchFocus={ [weak self] in self?.restoreSearchFocus() }
        state.showPalette={ [weak self] mode in self?.show(mode) };state.hidePalette={ [weak self] in self?.hide() };state.showToast={ [weak self] text,success in self?.showToast(text,success) }
    }
    func show(_ mode:String) {
        if mode=="analytics" {hide();state.showAnalytics?();return}
        if mode=="settings" {hide();state.showManagement?();return}
        if mode=="system-monitor" || mode=="monitor" {hide();state.showSystemMonitor?();return}
        if state.paletteVisible,state.mode==mode {hide();return}
        if let openTrace {PerformanceTrace.shared.finish(openTrace,cancelled:true)}
        openTrace=PerformanceTrace.shared.begin(.openReady)
        let trace=openTrace!
        if let app=NSWorkspace.shared.frontmostApplication,app.processIdentifier != getpid() {state.previousPID=app.processIdentifier}
        state.inlineCommand=nil;state.inlineInput="";state.query="";state.message="";state.mode=mode;state.detail=nil;state.scriptChoices=nil;state.form=nil;state.actionsOpen=false;state.paletteVisible=true
        let mouse=NSEvent.mouseLocation
        let screen=NSScreen.screens.first {$0.frame.contains(mouse)} ?? NSScreen.main
        if let frame=screen?.visibleFrame {panel.setFrame(PaletteGeometry.frame(in:frame,mode:mode),display:false)}
        panel.makeKeyAndOrderFront(nil)
        DispatchQueue.main.async { [weak self] in
            guard let self,self.panel.isVisible,self.openTrace?.id==trace.id else {return}
            defer {self.openTrace=nil}
            func field(_ view:NSView)->SearchInput? {if let input=view as? SearchInput {return input};return view.subviews.lazy.compactMap {field($0)}.first}
            if let view=self.panel.contentView,let input=field(view),self.panel.makeFirstResponder(input) {PerformanceTrace.shared.finish(trace)} else {PerformanceTrace.shared.finish(trace,cancelled:true)}
        }
        if outside==nil {outside=NSEvent.addGlobalMonitorForEvents(matching:[.leftMouseDown,.rightMouseDown]) {[weak self] _ in self?.hide()}}
        if local==nil {local=NSEvent.addLocalMonitorForEvents(matching:[.leftMouseDown,.rightMouseDown]) {[weak self] event in if event.window !== self?.panel,!(event.window?.contentView.map {ActionMenuKeyInput.contains(in:$0)} ?? false) {self?.hide()};return event}}
    }
    func restoreSearchFocus() {
        DispatchQueue.main.async { [weak self] in
            guard let self,self.state.paletteVisible,!self.state.actionsOpen,self.state.form==nil,self.state.pendingRemoval==nil,self.state.confirmationCommand==nil else {return}
            func field(_ view:NSView)->SearchInput? {if let input=view as? SearchInput {return input};return view.subviews.lazy.compactMap {field($0)}.first}
            if let view=self.panel.contentView,let input=field(view) {self.panel.makeFirstResponder(input)}
        }
    }
    func hide() {if let openTrace {PerformanceTrace.shared.finish(openTrace,cancelled:true);self.openTrace=nil};PerformanceTrace.shared.cancelInput();panel.orderOut(nil);state.paletteVisible=false;state.actionsOpen=false;if let outside {NSEvent.removeMonitor(outside);self.outside=nil};if let local {NSEvent.removeMonitor(local);self.local=nil}}
    func showToast(_ text:String,_ success:Bool) {
        toastTimer?.invalidate();toast?.orderOut(nil)
        let p=NSPanel(contentRect:NSRect(x:0,y:0,width:340,height:65),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
        p.isOpaque=false;p.backgroundColor = .clear;p.level = .statusBar;p.ignoresMouseEvents=true;p.hasShadow=true
        p.contentView=NSHostingView(rootView:HStack(spacing:10) {Image(systemName:success ? "checkmark.circle.fill":"exclamationmark.circle.fill").foregroundStyle(success ? Color.green:Color.orange);Text(text).font(.system(size:13,weight:.medium)).lineLimit(2);Spacer()}.padding(16).frame(width:340,height:65).background(.regularMaterial,in:RoundedRectangle(cornerRadius:12)))
        if let f=(NSScreen.screens.first {$0.frame.contains(NSEvent.mouseLocation)} ?? NSScreen.main)?.visibleFrame {p.setFrameOrigin(NSPoint(x:f.maxX-360,y:f.maxY-85))}
        p.orderFrontRegardless();toast=p
        toastTimer=Timer.scheduledTimer(withTimeInterval:5,repeats:false) {[weak self] _ in self?.toast?.orderOut(nil);self?.toast=nil}
    }
}

final class TracedPaletteHost<Content:View>:NSHostingView<Content> {
    override func draw(_ dirtyRect:NSRect) {
        PerformanceTrace.shared.measure(.hostDraw) {super.draw(dirtyRect)}
    }
}
