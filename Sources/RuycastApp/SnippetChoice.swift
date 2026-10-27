import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

/// Owns both visible selection and keyboard submission; no focus in a popup is required.
final class SnippetChoiceModel:ObservableObject {
    let command:Command
    let fields:[TemplateField]
    @Published var values:[String:String]
    var activeField:String?
    private var finished=false
    private let onSubmit:([String:String])->Void
    private let onCancel:()->Void
    init(command:Command,submit:@escaping ([String:String])->Void,cancel:@escaping ()->Void) {
        self.command=command;fields=Templates.fields(command);onSubmit=submit;onCancel=cancel
        values=Dictionary(uniqueKeysWithValues:fields.map {field in
            (field.id,field.options.isEmpty ? field.defaultValue:(field.options.contains(field.defaultValue) ? field.defaultValue:field.options.first ?? ""))
        });activeField=fields.first?.id
    }
    var canSubmit:Bool {!fields.contains {(values[$0.id] ?? "").trimmingCharacters(in:.whitespacesAndNewlines).isEmpty}}
    func submit() {
        guard !finished,canSubmit,(try? Templates.render(command,values:values,clipboard:command.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil)) != nil else {return}
        finished=true;onSubmit(values)
    }
    func cancel() {guard !finished else {return};finished=true;onCancel()}
    func handle(_ event:NSEvent,editingText:Bool,composing:Bool=false)->Bool {
        guard event.type == .keyDown,event.modifierFlags.intersection([.command,.control,.option,.shift]).isEmpty else {return false}
        if composing {return false}
        switch event.keyCode {
        case 36,76:if !event.isARepeat {submit()};return true
        case 53:cancel();return true
        case 123,124,125,126:
            guard !editingText,let field=fields.first(where:{$0.id==activeField}),!field.options.isEmpty else {return false}
            guard !finished else {return true}
            let index=field.options.firstIndex(of:values[field.id] ?? "") ?? 0
            let delta=event.keyCode==123 || event.keyCode==126 ? -1:1
            values[field.id]=field.options[max(0,min(field.options.count-1,index+delta))]
            return true
        default:return false
        }
    }
}

/// Intercepts keys before a dropdown, button or field editor can consume them.
final class SnippetChoicePanel:NSPanel {
    let model:SnippetChoiceModel
    init(model:SnippetChoiceModel) {
        self.model=model
        super.init(contentRect:NSRect(x:0,y:0,width:456,height:min(440,130+CGFloat(model.fields.count)*70)),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
        isReleasedWhenClosed=false
    }
    override var canBecomeKey:Bool {true}
    override var canBecomeMain:Bool {false}
    private func route(_ event:NSEvent)->Bool {
        let editor=firstResponder as? NSTextView
        return model.handle(event,editingText:editor != nil,composing:editor?.hasMarkedText() ?? false)
    }
    override func sendEvent(_ event:NSEvent) {if !route(event) {super.sendEvent(event)}}
    override func performKeyEquivalent(with event:NSEvent)->Bool {route(event) || super.performKeyEquivalent(with:event)}
    override func keyDown(with event:NSEvent) {if !route(event) {super.keyDown(with:event)}}
}

struct SnippetChoiceView:View {
    @ObservedObject var model:SnippetChoiceModel
    @FocusState private var focusedField:String?
    init(model:SnippetChoiceModel) {self.model=model}
    init(command:Command,submit:@escaping ([String:String])->Void,cancel:@escaping ()->Void) {
        model=SnippetChoiceModel(command:command,submit:submit,cancel:cancel)
    }
    private func binding(_ field:TemplateField)->Binding<String> {
        Binding(get:{model.values[field.id] ?? ""},set:{model.activeField=field.id;model.values[field.id]=$0})
    }
    var body:some View {
        VStack(alignment:.leading,spacing:18) {
            Text(model.command.title).font(.system(size:17,weight:.semibold)).lineLimit(2)
            ScrollView {
                VStack(alignment:.leading,spacing:14) {
                    ForEach(model.fields) {field in
                        VStack(alignment:.leading,spacing:6) {
                            Text(field.label).font(.system(size:12,weight:.medium)).foregroundStyle(.secondary)
                            if field.options.isEmpty {
                                TextField(field.label,text:binding(field)).textFieldStyle(.roundedBorder).focused($focusedField,equals:field.id)
                            } else {
                                Picker(field.label,selection:binding(field)) {
                                    ForEach(field.options,id:\.self) {Text($0).tag($0)}
                                }.labelsHidden().focused($focusedField,equals:field.id).frame(maxWidth:.infinity,alignment:.leading)
                            }
                        }
                    }
                }
            }
            HStack {
                Button("Cancel") {model.cancel()}.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Insert") {model.submit()}.keyboardShortcut(.defaultAction).disabled(!model.canSubmit)
            }
        }.padding(22).frame(width:456,height:min(440,130+CGFloat(model.fields.count)*70))
            .background(.regularMaterial,in:RoundedRectangle(cornerRadius:14))
            .onChange(of:focusedField) {_,field in if let field {model.activeField=field}}
    }
}

/// Only presents after the triggering key has reached the editor. The keyword
/// stays in place until confirmation, so dismissing never needs a rollback.
final class SnippetChoiceController {
    let request:SnippetChoiceRequest
    private var panel:NSPanel?
    private var activation:NSObjectProtocol?
    private var click:Any?
    private var resign:NSObjectProtocol?
    private var element:AXUIElement?
    private var range:CFRange?
    private var value:String?
    private var ready=false
    private var returning=false
    var onFinish:(()->Void)?
    let apply:(SnippetReplacement)->Void
    let safe:()->Bool
    let failure:(String)->Void
    init(command:Command,pid:pid_t,safe:@escaping ()->Bool,apply:@escaping (SnippetReplacement)->Void,failure:@escaping (String)->Void) {
        request=SnippetChoiceRequest(command:command,pid:pid);self.safe=safe;self.apply=apply;self.failure=failure
        activation=NSWorkspace.shared.notificationCenter.addObserver(forName:NSWorkspace.didActivateApplicationNotification,object:nil,queue:.main) { [weak self] note in
            guard let self,let app=note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else {return}
            let pid=app.processIdentifier
            if pid != getpid() && !(self.returning && pid==self.request.pid) {self.cancel(restore:false)}
        }
    }
    deinit {if let activation {NSWorkspace.shared.notificationCenter.removeObserver(activation)};if let click {NSEvent.removeMonitor(click)};if let resign {NotificationCenter.default.removeObserver(resign)}}
    func input(_ type:CGEventType,_ event:CGEvent) {
        if type == .keyDown {
            if panel?.isKeyWindow != true || returning {cancel(restore:false)}
        } else if (type == .leftMouseDown || type == .rightMouseDown) && (!ready || returning) {cancel(restore:false)}
    }
    func present() {
        guard request.active,NSWorkspace.shared.frontmostApplication?.processIdentifier==request.pid,safe() else {cancel(restore:false);return}
        element=focusedElement()
        if let element {
            var raw:CFTypeRef?
            if AXUIElementCopyAttributeValue(element,kAXSelectedTextRangeAttribute as CFString,&raw) == .success,let raw,CFGetTypeID(raw)==AXValueGetTypeID() {
                var selection=CFRange();if AXValueGetValue(unsafeBitCast(raw,to:AXValue.self),.cfRange,&selection) {range=selection}
            }
            raw=nil;AXUIElementCopyAttributeValue(element,kAXValueAttribute as CFString,&raw);value=raw as? String
            if let value,let range,range.length != 0 || range.location<0 || range.location>value.utf16.count || !(value as NSString).substring(to:range.location).hasSuffix(request.command.keyword ?? "") {cancel(restore:false);return}
        }
        ready=true
        let model=SnippetChoiceModel(command:request.command,submit:{ [weak self] in self?.confirm($0) },cancel:{ [weak self] in self?.cancel() })
        let size=NSSize(width:456,height:min(440,130+CGFloat(Templates.fields(request.command).count)*70))
        let panel=SnippetChoicePanel(model:model)
        let view=SnippetChoiceView(model:model)
        panel.isReleasedWhenClosed=false;panel.isOpaque=false;panel.backgroundColor = .clear;panel.hasShadow=true;panel.level = .floating
        panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary];panel.contentView=NSHostingView(rootView:view)
        let screen=NSScreen.screens.first {$0.frame.contains(NSEvent.mouseLocation)} ?? NSScreen.main
        if let f=screen?.visibleFrame {panel.setFrameOrigin(NSPoint(x:f.midX-size.width/2,y:f.maxY-size.height-100))}
        self.panel=panel;panel.makeKeyAndOrderFront(nil)
        click=NSEvent.addGlobalMonitorForEvents(matching:[.leftMouseDown,.rightMouseDown]) { [weak self] _ in self?.cancel(restore:false) }
        resign=NotificationCenter.default.addObserver(forName:NSWindow.didResignKeyNotification,object:panel,queue:.main) { [weak self] _ in
            guard let self,!self.returning else {return};self.cancel(restore:false)
        }
    }
    func cancel(restore:Bool=true) {
        request.cancel();if let resign {NotificationCenter.default.removeObserver(resign);self.resign=nil};panel?.orderOut(nil);panel=nil
        if restore,ready,NSWorkspace.shared.frontmostApplication?.processIdentifier==getpid() {NSRunningApplication(processIdentifier:request.pid)?.activate(options:[])}
        onFinish?()
    }
    private func focusedElement()->AXUIElement? {
        let app=AXUIElementCreateApplication(request.pid);AXUIElementSetMessagingTimeout(app,0.01)
        var raw:CFTypeRef?
        guard AXUIElementCopyAttributeValue(app,kAXFocusedUIElementAttribute as CFString,&raw) == .success,let raw,CFGetTypeID(raw)==AXUIElementGetTypeID() else {return nil}
        let element=unsafeBitCast(raw,to:AXUIElement.self);AXUIElementSetMessagingTimeout(element,0.01);return element
    }
    private func unchanged()->Bool {
        guard let element else {return true} // AX-unavailable web editors use the event/activation guards.
        guard let current=focusedElement(),CFEqual(element,current) else {return false}
        var raw:CFTypeRef?
        if let range {
            guard AXUIElementCopyAttributeValue(current,kAXSelectedTextRangeAttribute as CFString,&raw) == .success,let raw,CFGetTypeID(raw)==AXValueGetTypeID() else {return false}
            var now=CFRange();guard AXValueGetValue(unsafeBitCast(raw,to:AXValue.self),.cfRange,&now),now.location==range.location,now.length==range.length else {return false}
        }
        if let value {
            raw=nil;guard AXUIElementCopyAttributeValue(current,kAXValueAttribute as CFString,&raw) == .success,raw as? String==value else {return false}
        }
        return true
    }
    func confirm(_ values:[String:String]) {
        guard request.active,ready,!returning else {return}
        // Render while the form is still available; malformed input must not
        // delete a trigger or close the only place where it can be corrected.
        guard (try? Templates.render(request.command,values:values,clipboard:request.command.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil)) != nil else {return}
        returning=true;panel?.orderOut(nil)
        guard let app=NSRunningApplication(processIdentifier:request.pid),app.activate(options:[]) else {failure("The app is unavailable. Type the keyword again.");cancel(restore:false);return}
        finish(values,attempt:0)
    }
    private func finish(_ values:[String:String],attempt:Int) {
        DispatchQueue.main.asyncAfter(deadline:.now()+0.03) { [weak self] in
            guard let self,self.request.active else {return}
            let pid=NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
            if pid==getpid(),attempt<15 {self.finish(values,attempt:attempt+1);return}
            do {
                let replacement=try self.request.replacement(values:values,pid:pid,safe:self.safe(),unchanged:self.unchanged(),clipboard:self.request.command.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil)
                self.apply(replacement)
            } catch {self.failure("The text field changed. Type the keyword again.")}
            self.cancel(restore:false)
        }
    }
}
