import SwiftUI
import AppKit
import CryptoKit
import LauncherCore

struct CommandForm:Identifiable {
    let id=UUID()
    var command:Command
    var isRun:Bool
    var isNew=false
    var bindingsOnly=false
    var bindingBase:Command?
    var values:[String:String]=[:]
    var source="launcher"
}
struct CommandEditor:View {
    @ObservedObject var state:AppState
    @State var form:CommandForm
    var onBack:(()->Void)?=nil
    @FocusState private var focused:Bool
    var body:some View {
        VStack(alignment:.leading,spacing:16) {
            HStack {Button {if let onBack {onBack()} else {state.form=nil}} label:{Label("Back",systemImage:"chevron.left")}.buttonStyle(.plain).disabled(state.busy);Spacer();Text(form.command.title).font(.headline);Spacer()}
            ScrollView {
                VStack(alignment:.leading,spacing:14) {
                    if form.isRun {
                        ForEach(Templates.fields(form.command)) {field in
                            VStack(alignment:.leading,spacing:5) {
                                Text(field.label).font(.caption).foregroundStyle(.secondary)
                                if field.options.isEmpty {
                                    if ["capture.quick-note","capture.quick-task"].contains(form.command.id) || form.command.target=="utility-set-text" {CaptureTextEditor(text:Binding(get:{form.values[field.id] ?? field.defaultValue},set:{form.values[field.id]=$0}),placeholder:field.label,onSubmit:submit).frame(minHeight:240).background(Color.primary.opacity(0.035),in:RoundedRectangle(cornerRadius:7))} else {
                                        TextField(field.label,text:Binding(get:{form.values[field.id] ?? field.defaultValue},set:{form.values[field.id]=$0})).textFieldStyle(.roundedBorder).focused($focused)
                                    }
                                } else {
                                    Picker(field.label,selection:Binding(get:{form.values[field.id] ?? field.defaultValue},set:{form.values[field.id]=$0})) {ForEach(field.options,id:\.self) {Text($0).tag($0)}}.labelsHidden()
                                }
                            }
                        }
                    } else {
                        if !form.bindingsOnly {
                        field("Name",text:$form.command.title)
                        if form.command.kind=="snippet" {
                            Text("Text").font(.caption).foregroundStyle(.secondary)
                            TextEditor(text:$form.command.target).font(.system(size:13)).frame(minHeight:130).border(Color.secondary.opacity(0.2))
                            field("Keyword",text:Binding(get:{form.command.keyword ?? ""},set:{form.command.keyword=$0.isEmpty ? nil:$0}))
                        } else {field(form.command.kind=="url" ? "Link":"Target",text:$form.command.target)}
                        if form.command.kind=="url" || form.command.kind=="path" {
                            Picker("Open with",selection:Binding(get:{form.command.applicationID ?? ""},set:{form.command.applicationID=$0.isEmpty ? nil:$0})) {
                                Text("Default app").tag("")
                                ForEach(state.applications) {app in Text(app.title).tag(app.target)}
                            }
                        }
                        if form.command.kind=="script" {
                            VStack(alignment:.leading,spacing:8) {
                                HStack {Text("Arguments").font(.caption).foregroundStyle(.secondary);Spacer();Button("Add Argument") {form.command.arguments.append("")}.controlSize(.small)}
                                ForEach(form.command.arguments.indices,id:\.self) {index in
                                    HStack {
                                        TextField("Argument",text:$form.command.arguments[index],axis:.vertical).textFieldStyle(.roundedBorder)
                                        Button {form.command.arguments.remove(at:index)} label:{Image(systemName:"minus.circle")}.buttonStyle(.plain).help("Remove argument")
                                    }
                                }
                                HStack {Text("Timeout").font(.caption).foregroundStyle(.secondary);TextField("Seconds",value:Binding(get:{form.command.timeout ?? 30},set:{form.command.timeout=$0}),format:.number).textFieldStyle(.roundedBorder).frame(width:80);Text("seconds").font(.caption).foregroundStyle(.secondary)}
                            }
                        }
                        }
                        field("Aliases",text:Binding(get:{form.command.aliases.joined(separator:", ")},set:{form.command.aliases=$0.split(separator:",").map {$0.trimmingCharacters(in:.whitespaces)}.filter {!$0.isEmpty}}))
                        HStack {Text("Shortcut").font(.caption).foregroundStyle(.secondary);Spacer();ShortcutRecorder(state:state,value:form.command.hotkey,onSave:{form.command.hotkey=$0}).frame(width:170,height:28)}
                        if form.bindingsOnly {Toggle("Enabled",isOn:Binding(get:{form.command.isEnabled},set:{form.command.enabled=$0})).toggleStyle(.switch)}
                    }
                }.padding(.vertical,6)
            }
            if !state.message.isEmpty {Text(state.message).font(.caption).foregroundStyle(.orange).textSelection(.enabled)}
            HStack {if form.isRun && (form.command.id.hasPrefix("capture.") || form.command.target=="utility-set-text") {Text("⇧↵ New line").font(.caption).foregroundStyle(.secondary)};Spacer();if state.busy {ProgressView().controlSize(.small)};Button(form.isRun ? (form.command.id.hasPrefix("capture.") ? "Save":form.command.kind=="snippet" ? "Insert":form.command.kind=="url" ? "Open":"Run"):"Save") {submit()}.keyboardShortcut(.return,modifiers:.command).disabled(state.busy)}
        }.padding(22).onAppear {focused = !["capture.quick-note","capture.quick-task"].contains(form.command.id)}.onSubmit {submit()}
    }
    private func field(_ label:String,text:Binding<String>)->some View {VStack(alignment:.leading,spacing:5) {Text(label).font(.caption).foregroundStyle(.secondary);TextField(label,text:text).textFieldStyle(.roundedBorder)}}
    private func submit() {
        guard !state.busy else {return}
        state.message=""
        if form.isRun {
            var values=form.values
            for field in Templates.fields(form.command) where values[field.id]==nil {values[field.id]=field.defaultValue}
            state.perform(form.command.id,values:values,source:form.source)
        } else if form.bindingsOnly {state.saveBindings(form.command,original:form.bindingBase ?? form.command,isNew:form.isNew,onSuccess:onBack)}
        else {state.saveCommand(form.command,isNew:form.isNew,onSuccess:onBack)}
    }
}
extension AppState {
    var visibleCommands:[Command] {
        if mode=="processes" {return processOrder}
        switch mode {case "snippets":return results.filter {$0.kind=="snippet"};case "quicklinks":return results.filter {$0.kind=="url" || $0.kind=="path"};default:return results}
    }
    var selectedCommand:Command? {visibleCommands.indices.contains(selection) ? visibleCommands[selection]:nil}
    func moveSelection(_ delta:Int) {
        if actionsOpen {moveActionSelection(delta);return}
        let count=mode=="clipboard" ? filteredClips.count:visibleCommands.count
        selection=max(0,min(max(0,count-1),selection+delta))
    }
    func activateSelection() {
        if mode=="processes",NSApp.currentEvent?.isARepeat==true {return}
        if actionsOpen {activateActionSelection();return}
        if mode=="clipboard" {
            guard filteredClips.indices.contains(selection) else {return}
            do {try copyClip(filteredClips[selection].id,paste:true)} catch {message=error.localizedDescription}
        } else if let cmd=selectedCommand {begin(cmd)}
    }
    func begin(_ command:Command,source:String="launcher") {
        actionsOpen=false;message=""
        if !Templates.fields(command).isEmpty {form=CommandForm(command:command,isRun:true,source:source)}
        else {perform(command.id,source:source)}
    }
    func newCommand(_ kind:String) {
        actionsOpen=false;message=""
        form=CommandForm(command:Command(id:kind+"."+UUID().uuidString.lowercased(),title:kind=="snippet" ? "New Snippet":"New Quicklink",kind:kind,target:kind=="snippet" ? "":"https://"),isRun:false,isNew:true)
    }
    func editSelection() {guard let cmd=selectedCommand else {return};actionsOpen=false;message="";form=CommandForm(command:cmd,isRun:false)}
    func saveBindings(_ edited:Command,original:Command,isNew:Bool=false,onSuccess:(()->Void)?=nil) {
        guard var current=config.commands.first(where:{$0.id==original.id}) ?? (isNew ? original:nil) else {message="Command was removed. Reopen it before editing.";return}
        if edited.aliases != original.aliases {current.aliases=edited.aliases}
        if edited.hotkey != original.hotkey {current.hotkey=edited.hotkey}
        if edited.isEnabled != original.isEnabled {current.enabled=edited.enabled}
        saveCommand(current,isNew:isNew,onSuccess:onSuccess)
    }
    func saveCommand(_ original:Command,isNew:Bool=false,remove:Bool=false,onSuccess:(()->Void)?=nil) {
        guard !busy else {return}
        var command=original
        if !remove,command.kind=="url" || command.kind=="path" {
            let target=command.target.trimmingCharacters(in:.whitespacesAndNewlines)
            if target.hasPrefix("/") || target.hasPrefix("~/") {command.kind="path";command.target=target}
            else if let url=URL(string:target),url.isFileURL {command.kind="path";command.target=url.path}
            else {command.kind="url";command.target=target.contains(":") ? target:"https://"+target}
        }
        var next=config
        if remove {next.commands.removeAll {$0.id==command.id}}
        else if let i=next.commands.firstIndex(where:{$0.id==command.id}) {next.commands[i]=command}
        else {next.commands.append(command)}
        do {try next.validate()} catch {message=error.localizedDescription;return}
        busy=true
        let path=configURL,root=root
        let expected=configDigestForSave
        DispatchQueue.global(qos:.userInitiated).async { [weak self] in
            do {
                let before=try Data(contentsOf:path)
                // The expected hash is captured with the GUI snapshot on the main thread below.
                guard SHA256.hash(data:before).map({String(format:"%02x",$0)}).joined()==expected else {throw LauncherError("Configuration changed. Reload and try again.")}
                let temporary=root.appendingPathComponent(".editor-"+UUID().uuidString+".json")
                defer {try? FileManager.default.removeItem(at:temporary)}
                try Files.save(next.commands,to:temporary)
                let process=Process();process.executableURL=ruycastCLI();process.arguments=["config","set","commands","@"+temporary.path,"--expect-sha256",expected]
                var env=ProcessInfo.processInfo.environment;env["RUYCAST_HOME"]=root.path;process.environment=env
                let pipe=Pipe();process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
                try process.run();let data=pipe.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit()
                let result=try JSONSerialization.jsonObject(with:data) as? [String:Any]
                guard process.terminationStatus==0,result?["ok"] as? Bool==true else {throw LauncherError(result?["error"] as? String ?? "Could not save configuration.")}
                DispatchQueue.main.async {self?.busy=false;self?.form=nil;self?.actionsOpen=false;self?.showToast?(remove ? "Removed":"Saved",true);onSuccess?()}
            } catch {DispatchQueue.main.async {self?.busy=false;self?.message=error.localizedDescription}}
        }
    }
}
func ruycastCLI()->URL {
    let bundled=Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/ruycast")
    let source=URL(fileURLWithPath:#filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent(".build/rust/release/ruycast")
    return FileManager.default.fileExists(atPath:bundled.path) ? bundled:source
}

extension AppState {
    var systemCommands:[Command] {
        var builtins=[Command(id:"builtin.settings",title:"Ruycast Settings",kind:"builtin",target:"settings",aliases:["settings","preferences","manage","beheer","instellingen"]),Command(id:"builtin.dashboard",title:"Ruycast Dashboard",kind:"builtin",target:"analytics",aliases:["analytics","dashboard","activity"])]
        builtins.removeAll {candidate in config.commands.contains {$0.id==candidate.id}}
        return builtins
    }
    func resolveCommand(_ id:String)->Command? {
        config.resolve(id) ?? systemCommands.first {$0.id==id || $0.aliases.contains(id.lowercased())} ?? results.first {$0.id==id} ?? serviceCommands[id]
    }
    func setHyperEnabled(_ enabled:Bool) {setPreference("hyperKeyEnabled",enabled)}
    func setExpansionEnabled(_ enabled:Bool) {setPreference("snippetExpansionEnabled",enabled)}
    func setClipboardEnabled(_ enabled:Bool) {setPreference("clipboard.enabled",enabled)}
    func setPreference(_ key:String,_ enabled:Bool) {setSetting(key,enabled ? "true":"false")}
    func setSetting(_ key:String,_ json:String) {
        guard !busy else {return};busy=true;message="";let root=root,expected=configDigestForSave
        DispatchQueue.global(qos:.userInitiated).async { [weak self] in
            do {
                let process=Process();process.executableURL=ruycastCLI();process.arguments=["config","set",key,json,"--expect-sha256",expected]
                var env=ProcessInfo.processInfo.environment;env["RUYCAST_HOME"]=root.path;process.environment=env
                let pipe=Pipe();process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
                try process.run();let bytes=pipe.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit()
                let reply=try JSONSerialization.jsonObject(with:bytes) as? [String:Any]
                guard process.terminationStatus==0,reply?["ok"] as? Bool==true else {throw LauncherError(reply?["error"] as? String ?? "Could not save settings.")}
                DispatchQueue.main.async {self?.busy=false;self?.expander?.refresh()}
            } catch {DispatchQueue.main.async {self?.busy=false;self?.message=error.localizedDescription}}
        }
    }
}
