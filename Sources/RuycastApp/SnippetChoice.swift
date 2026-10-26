import AppKit
import SwiftUI
import LauncherCore

struct SnippetChoiceView:View {
    let command:Command
    let submit:([String:String])->Void
    let cancel:()->Void
    @State private var values:[String:String]
    init(command:Command,submit:@escaping ([String:String])->Void,cancel:@escaping ()->Void) {
        self.command=command;self.submit=submit;self.cancel=cancel
        _values=State(initialValue:Dictionary(uniqueKeysWithValues:Templates.fields(command).map {field in
            (field.id,field.options.isEmpty ? field.defaultValue:(field.options.contains(field.defaultValue) ? field.defaultValue:field.options.first ?? ""))
        }))
    }
    var body:some View {
        VStack(alignment:.leading,spacing:18) {
            Text(command.title).font(.system(size:17,weight:.semibold)).lineLimit(2)
            ScrollView {
                VStack(alignment:.leading,spacing:14) {
                    ForEach(Templates.fields(command)) {field in
                        VStack(alignment:.leading,spacing:6) {
                            Text(field.label).font(.system(size:12,weight:.medium)).foregroundStyle(.secondary)
                            if field.options.isEmpty {
                                TextField(field.label,text:Binding(get:{values[field.id] ?? ""},set:{values[field.id]=$0})).textFieldStyle(.roundedBorder)
                            } else {
                                Picker(field.label,selection:Binding(get:{values[field.id] ?? ""},set:{values[field.id]=$0})) {
                                    ForEach(field.options,id:\.self) {Text($0).tag($0)}
                                }.labelsHidden().frame(maxWidth:.infinity,alignment:.leading)
                            }
                        }
                    }
                }
            }
            HStack {
                Button("Cancel",action:cancel).keyboardShortcut(.cancelAction)
                Spacer()
                Button("Insert") {submit(values)}.keyboardShortcut(.defaultAction)
                    .disabled(Templates.fields(command).contains {(values[$0.id] ?? "").trimmingCharacters(in:.whitespacesAndNewlines).isEmpty})
            }
        }.padding(22).frame(width:456,height:min(440,130+CGFloat(Templates.fields(command).count)*70))
            .background(.regularMaterial,in:RoundedRectangle(cornerRadius:14))
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
        let view=SnippetChoiceView(command:request.command,submit:{ [weak self] in self?.confirm($0) },cancel:{ [weak self] in self?.cancel() })
        let size=NSSize(width:456,height:min(440,130+CGFloat(Templates.fields(request.command).count)*70))
        let panel=PalettePanel(contentRect:NSRect(origin:.zero,size:size),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
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
