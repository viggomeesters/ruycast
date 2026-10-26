import AppKit
import SwiftUI
import LauncherCore

// Shared native palette for the launcher’s management surfaces. Settings remains a dense work surface.
struct SettingsTheme {
    let dark:Bool
    var graphite=false
    var background:Color {graphite ? Color(white:0.15):dark ? Color(red:0.10,green:0.12,blue:0.17):Color(red:0.98,green:0.97,blue:0.94)}
    var ink:Color {dark ? Color(red:0.96,green:0.95,blue:0.91):Color(red:0.18,green:0.20,blue:0.25)}
    var muted:Color {dark ? Color(red:0.68,green:0.71,blue:0.77):Color(red:0.42,green:0.44,blue:0.48)}
    var surface:Color {ink.opacity(dark ? 0.045:0.035)}
    var accent:Color {dark ? Color(red:1,green:0.66,blue:0.35):Color(red:0.64,green:0.30,blue:0.08)}
}
enum SettingsTab:String,CaseIterable {
    case dashboard="Dashboard",general="General",commands="Commands",snippets="Snippets",quicklinks="Quicklinks",clipboard="Clipboard",advanced="Advanced"
    var icon:String {switch self {case .dashboard:return "chart.bar";case .general:return "slider.horizontal.3";case .commands:return "command";case .snippets:return "text.quote";case .quicklinks:return "link";case .clipboard:return "clipboard";case .advanced:return "gearshape.2"}}
}
final class ManagementNavigation:ObservableObject {
    @Published var query=""
    @Published var kind="commands"
    @Published var filter="all"
    @Published var selectedID:String?
    @Published var form:CommandForm?
    @Published var collapsed:Set<String>=[]
    var tab:SettingsTab {SettingsTab.allCases.first {$0.rawValue.lowercased()==kind} ?? .commands}
    func select(_ tab:SettingsTab) {kind=tab.rawValue.lowercased();query="";filter="all";selectedID=nil;form=nil}
    func commands(in config:Configuration)->[Command] {
        config.search(query).filter {c in
            (tab == .snippets ? c.kind=="snippet":tab == .quicklinks ? ["url","path"].contains(c.kind):true) &&
            (filter=="all" || (filter=="disabled" ? !c.isEnabled:c.kind==filter))
        }
    }
    static func group(_ c:Command)->String {
        switch c.kind {
        case "app":return "Applications"
        case "window":return "Window Management"
        case "snippet":return "Snippets"
        case "url","path":return "Quicklinks"
        default:
            let key=(c.id+" "+c.title+" "+c.target).lowercased()
            if key.contains("pomodoro") {return "Pomodoro"}
            if key.contains("dictate") {return "Dictate"}
            if key.contains("ai-usage") || key.contains("ai usage") {return "AI Usage"}
            if key.contains("task") {return "Tasks"}
            return c.kind=="builtin" ? "Ruycast":"Scripts & Actions"
        }
    }
}

struct ManagementView:View {
    @ObservedObject var state:AppState
    @ObservedObject var navigation:ManagementNavigation
    @Environment(\.colorScheme) private var scheme
    @State private var pendingRemove:Command?
    @State private var confirmRemove=false
    private var theme:SettingsTheme {SettingsTheme(dark:(state.preferredColorScheme ?? scheme) == .dark,graphite:state.config.theme=="graphite")}
    private var commands:[Command] {var catalog=state.config;catalog.commands += state.systemCommands;return navigation.commands(in:catalog)}
    var body:some View {
        VStack(spacing:0) {
            HStack(spacing:4) {
                ForEach(SettingsTab.allCases,id:\.self) {tab in
                    Button {state.endShortcutRecording();navigation.select(tab)} label:{
                        VStack(spacing:6) {
                            Image(systemName:tab.icon).font(.system(size:21,weight:.regular)).frame(height:25)
                            Text(tab.rawValue).font(.system(size:11,weight:navigation.tab==tab ? .semibold:.regular))
                        }.frame(width:84,height:61)
                            .foregroundStyle(navigation.tab==tab ? theme.accent:theme.muted)
                            .background(navigation.tab==tab ? theme.ink.opacity(0.07):.clear,in:RoundedRectangle(cornerRadius:9))
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityLabel(tab.rawValue).accessibilityAddTraits(navigation.tab==tab ? .isSelected:[])
                }
            }.frame(maxWidth:.infinity).padding(.vertical,10).background(theme.surface)
            Divider().overlay(theme.ink.opacity(0.06))
            if let form=navigation.form {
                CommandEditor(state:state,form:form,onBack:{navigation.form=nil}).id(form.id)
            } else {
                switch navigation.tab {
                case .dashboard:AnalyticsDashboard(store:state.analytics,commands:state.config.commands+state.systemCommands+state.applications,theme:theme)
                case .commands,.snippets,.quicklinks:commandLibrary
                case .general:general
                case .clipboard:ClipboardPreferences(state:state,theme:theme)
                case .advanced:advanced
                }
            }
            if let error=state.configError {feedback(error)}
            if let error=state.clipboardCleanupError {feedback(error)}
            if !state.message.isEmpty {feedback(state.message)}
        }.environment(\.colorScheme,state.preferredColorScheme ?? scheme).preferredColorScheme(state.preferredColorScheme).frame(minWidth:680,minHeight:480).foregroundStyle(theme.ink).tint(theme.accent).background(theme.background)
            .confirmationDialog("Remove \(pendingRemove?.title ?? "command")?",isPresented:$confirmRemove) {
                Button("Remove",role:.destructive) {if let command=pendingRemove {state.saveCommand(command,remove:true);pendingRemove=nil}}
                Button("Cancel",role:.cancel) {pendingRemove=nil}
            }
    }
    private func feedback(_ text:String)->some View {
        HStack {Image(systemName:"exclamationmark.circle");Text(text).textSelection(.enabled);Spacer();if state.busy {ProgressView().controlSize(.small)}}
            .font(.caption).foregroundStyle(theme.accent).padding(12).background(theme.surface)
    }
    private var commandLibrary:some View {
        VStack(spacing:0) {
            HStack(spacing:12) {
                HStack(spacing:7) {
                    Image(systemName:"magnifyingglass").foregroundStyle(theme.muted)
                    TextField("Search \(navigation.tab.rawValue.lowercased())…",text:$navigation.query).textFieldStyle(.plain).accessibilityLabel("Search commands")
                    if !navigation.query.isEmpty {Button {navigation.query=""} label:{Image(systemName:"xmark.circle.fill")}.buttonStyle(.plain).foregroundStyle(theme.muted).help("Clear search")}
                }.padding(9).background(theme.surface,in:RoundedRectangle(cornerRadius:7))
                if navigation.tab == .commands {
                    Picker("Filter",selection:$navigation.filter) {
                        Text("All Commands").tag("all");Text("Applications").tag("app");Text("Window Management").tag("window");Text("Scripts").tag("script");Text("Ruycast").tag("builtin");Text("Disabled").tag("disabled")
                    }.labelsHidden().frame(width:160)
                }
                Menu {
                    Button("Snippet") {create("snippet")};Button("Quicklink") {create("url")};Button("Shell Command") {create("script")}
                } label:{Label("Add",systemImage:"plus")}.fixedSize().disabled(state.busy)
            }.padding(18)
            if navigation.tab == .snippets {
                HStack {Toggle("Expand snippet keywords",isOn:Binding(get:{state.config.snippetExpansionEnabled==true},set:state.setExpansionEnabled)).toggleStyle(.switch).controlSize(.small);Spacer();permissionNotice}.padding(.horizontal,20).padding(.bottom,14).disabled(state.busy)
            }
            HStack(spacing:12) {
                Text("Name").frame(maxWidth:.infinity,alignment:.leading)
                Text("Alias").frame(width:130,alignment:.leading)
                Text("Shortcut").frame(width:156,alignment:.leading)
                Text("Enabled").frame(width:52)
                Color.clear.frame(width:22,height:1)
            }.font(.system(size:11,weight:.medium)).foregroundStyle(theme.muted).padding(.horizontal,24).padding(.vertical,9).background(theme.surface)
            Divider()
            ScrollView {
                LazyVStack(spacing:0,pinnedViews:[]) {
                    let groups=Dictionary(grouping:commands,by:ManagementNavigation.group)
                    ForEach(groups.keys.sorted(),id:\.self) {group in
                        Button {if !navigation.collapsed.insert(group).inserted {navigation.collapsed.remove(group)}} label:{
                            HStack(spacing:6) {Image(systemName:navigation.collapsed.contains(group) ? "chevron.right":"chevron.down").font(.system(size:9,weight:.semibold));Text(group);Text("\(groups[group]!.count)").foregroundStyle(theme.muted);Spacer()}
                                .font(.system(size:11,weight:.semibold)).foregroundStyle(theme.muted).padding(.horizontal,23).padding(.top,15).padding(.bottom,6).contentShape(Rectangle())
                        }.buttonStyle(.plain).accessibilityLabel("\(group), \(navigation.collapsed.contains(group) ? "collapsed":"expanded")")
                        if !navigation.collapsed.contains(group) || !navigation.query.isEmpty {
                            ForEach(groups[group]!) {command in
                                SettingsCommandRow(state:state,command:command,theme:theme,onEdit:{navigation.form=CommandForm(command:command,isRun:false)},onRemove:{pendingRemove=command;confirmRemove=true})
                            }
                        }
                    }
                    if commands.isEmpty {Text("No commands found").foregroundStyle(theme.muted).padding(40)}
                }.padding(.bottom,12)
            }
            Divider()
            HStack {Text("\(commands.count) \(navigation.tab.rawValue.lowercased())").font(.caption).foregroundStyle(theme.muted);Spacer();if state.busy {ProgressView().controlSize(.small)};Button("Open Configuration…") {NSWorkspace.shared.open(state.configURL)}.controlSize(.small)}.padding(.horizontal,20).padding(.vertical,12)
        }
    }
    private var general:some View {
        preferences("General") {
            preferenceRow("Open Launcher") {ShortcutRecorder(state:state,value:state.config.launcherHotkey,onSave:{if let value=$0 {state.setSetting("launcherHotkey",jsonString(value))}}).frame(width:180,height:28)}
            preferenceRow("Clipboard History") {ShortcutRecorder(state:state,value:state.config.clipboardHotkey,onSave:{if let value=$0 {state.setSetting("clipboardHotkey",jsonString(value))}}).frame(width:180,height:28)}
            preferenceRow("Caps Lock as Hyper") {Toggle("Caps Lock as Hyper",isOn:Binding(get:{state.config.hyperKeyEnabled==true},set:state.setHyperEnabled)).labelsHidden().toggleStyle(.switch)}
            preferenceRow("Separate Left and Right Command") {
                Toggle("Separate Left and Right Command",isOn:Binding(get:{state.config.separatesCommandKeys},set:{state.setPreference("separateCommandKeys",$0)})).labelsHidden().toggleStyle(.switch)
            }
            Text(state.config.separatesCommandKeys ? "Record L⌘ or R⌘ shortcuts to launch apps independently.":"L⌘ and R⌘ shortcuts are paused. New shortcuts use either Command key.").font(.caption).foregroundStyle(theme.muted).fixedSize(horizontal:false,vertical:true).padding(.bottom,14)
            preferenceRow("Theme") {Picker("Theme",selection:Binding(get:{state.config.theme ?? "system"},set:{state.setSetting("theme",jsonString($0))})) {Text("System").tag("system");Text("Light").tag("light");Text("Dark").tag("dark");Text("Graphite").tag("graphite")}.labelsHidden().frame(width:180)}
            preferenceRow("Menu Bar Icon") {Toggle("Show menu bar icon",isOn:Binding(get:{state.config.menuBarVisible ?? true},set:{state.setPreference("menuBarVisible",$0)})).labelsHidden().toggleStyle(.switch)}
            permissionNotice
        }
    }
    @ViewBuilder private var permissionNotice:some View {
        if !state.headless {
            if !state.windows.trusted {
                Button("Allow Accessibility…") {NSWorkspace.shared.open(URL(string:"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)}.font(.caption)
            }
            if (state.config.hyperKeyEnabled==true || state.config.snippetExpansionEnabled==true || state.config.separatesCommandKeys) && !CGPreflightListenEventAccess() {
                Button("Allow Input Monitoring…") {NSWorkspace.shared.open(URL(string:"x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")!)}.font(.caption)
            }
        }
    }
    private var advanced:some View {
        preferences("Advanced") {
            preferenceRow("Configuration") {Button("Open Configuration…") {NSWorkspace.shared.open(state.configURL)}}
            preferenceRow("Configuration Folder") {VStack(alignment:.trailing,spacing:6) {Text(state.configDirectory.path).font(.caption).foregroundStyle(theme.muted).lineLimit(2).truncationMode(.middle).textSelection(.enabled);Button("Choose Folder…",action:state.chooseConfigurationDirectory)}}
            preferenceRow("Reload") {Button("Reload Configuration") {do {try state.reload();state.message="";state.showToast?("Configuration reloaded",true)} catch {state.message=error.localizedDescription}}}
            preferenceRow("Permissions") {Button("Accessibility…") {NSWorkspace.shared.open(URL(string:"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)}}
            preferenceRow("Version") {Text("Ruycast 0.7.11").foregroundStyle(theme.muted)}
        }
    }
    private func preferences<Content:View>(_ title:String,@ViewBuilder content:()->Content)->some View {
        ScrollView {VStack(alignment:.leading,spacing:22) {
            Text(title).font(.system(size:21,weight:.bold,design:.rounded))
            VStack(alignment:.leading,spacing:0) {content()}.padding(.horizontal,18).background(theme.surface,in:RoundedRectangle(cornerRadius:10))
        }.padding(30).frame(maxWidth:650,alignment:.leading).frame(maxWidth:.infinity)}.disabled(state.busy)
    }
    private func preferenceRow<Content:View>(_ title:String,@ViewBuilder control:()->Content)->some View {
        HStack {Text(title);Spacer();control()}.frame(minHeight:55)
    }
    private func create(_ kind:String) {
        state.message=""
        let title=kind=="snippet" ? "New Snippet":kind=="script" ? "New Shell Command":"New Quicklink"
        navigation.form=CommandForm(command:Command(id:kind+"."+UUID().uuidString.lowercased(),title:title,kind:kind,target:kind=="snippet" ? "":kind=="script" ? "/usr/bin/true":"https://"),isRun:false,isNew:true)
    }
}
func jsonString(_ text:String)->String {String(data:try! JSONEncoder().encode(text),encoding:.utf8)!}

private struct SettingsCommandRow:View {
    @ObservedObject var state:AppState
    let command:Command
    let theme:SettingsTheme
    var onEdit:()->Void
    var onRemove:()->Void
    @State private var alias=""
    @State private var hover=false
    @FocusState private var aliasFocused:Bool
    var body:some View {
        HStack(spacing:12) {
            Button(action:onEdit) {
                HStack(spacing:10) {CommandIcon(command:command);VStack(alignment:.leading,spacing:2) {Text(command.title).lineLimit(1);if let keyword=command.keyword {Text(keyword).font(.caption).foregroundStyle(theme.muted)}};Spacer(minLength:0)}.contentShape(Rectangle())
            }.buttonStyle(.plain).frame(maxWidth:.infinity,alignment:.leading).opacity(command.isEnabled ? 1:0.5).help("Edit \(command.title)")
            TextField("Add alias",text:$alias).textFieldStyle(.plain).font(.system(size:12)).padding(.horizontal,7).frame(width:130,height:27).background(theme.surface,in:RoundedRectangle(cornerRadius:5)).focused($aliasFocused).onSubmit(saveAlias).onChange(of:aliasFocused) {_,focus in if !focus {saveAlias()}}.accessibilityLabel("Aliases for \(command.title)")
            ShortcutRecorder(state:state,value:command.hotkey,onSave:{var updated=command;updated.hotkey=$0;state.saveCommand(updated)}).frame(width:156,height:27).accessibilityLabel("Shortcut for \(command.title)")
            Toggle("Enable \(command.title)",isOn:Binding(get:{command.isEnabled},set:{var updated=command;updated.enabled=$0;state.saveCommand(updated)})).labelsHidden().toggleStyle(.checkbox).frame(width:52)
            Menu {Button("Edit…",action:onEdit);Divider();Button("Remove…",role:.destructive,action:onRemove)} label:{Image(systemName:"ellipsis")}.menuStyle(.borderlessButton).frame(width:22).help("Actions for \(command.title)")
        }.font(.system(size:13)).padding(.horizontal,12).padding(.vertical,5).background(hover ? theme.ink.opacity(0.05):.clear,in:RoundedRectangle(cornerRadius:6)).padding(.horizontal,12)
            .onHover {hover=$0}.onAppear {alias=command.aliases.joined(separator:", ")}.onChange(of:command.aliases) {_,values in if !aliasFocused {alias=values.joined(separator:", ")}}.disabled(state.busy)
    }
    private func saveAlias() {
        let values=alias.split(separator:",").map {$0.trimmingCharacters(in:.whitespaces)}.filter {!$0.isEmpty}
        guard values != command.aliases else {return}
        var updated=command;updated.aliases=values;state.saveCommand(updated)
    }
}

private struct ClipboardPreferences:View {
    @ObservedObject var state:AppState
    let theme:SettingsTheme
    @State private var limit=200
    @State private var days=30
    @State private var exclusions=""
    var body:some View {
        ScrollView {VStack(alignment:.leading,spacing:22) {
            Text("Clipboard").font(.system(size:21,weight:.bold,design:.rounded))
            VStack(alignment:.leading,spacing:18) {
                Toggle("Clipboard history",isOn:Binding(get:{state.config.clipboard.enabled},set:state.setClipboardEnabled)).toggleStyle(.switch)
                if state.paused {Button("Resume recording") {state.handle(["op":"clipboard.pause","paused":false]) {_ in}}}
                HStack {Text("History limit");Spacer();Stepper("\(limit) items",value:$limit,in:1...1000).frame(width:160)}
                HStack {Text("Keep history for");Spacer();Stepper("\(days) days",value:$days,in:1...365).frame(width:160)}
                VStack(alignment:.leading,spacing:8) {Text("Excluded apps");TextEditor(text:$exclusions).font(.system(size:12,design:.monospaced)).frame(height:110).scrollContentBackground(.hidden).padding(6).background(theme.surface,in:RoundedRectangle(cornerRadius:6));Text("One bundle identifier per line").font(.caption).foregroundStyle(theme.muted)}
                HStack {Text("\(state.clips.count) saved items").font(.caption).foregroundStyle(theme.muted);Spacer();Button("Save") {var clipboard=state.config.clipboard;clipboard.limit=limit;clipboard.retentionDays=days;clipboard.excludedApps=exclusions.split(whereSeparator:\.isNewline).map {String($0).trimmingCharacters(in:.whitespaces)}.filter {!$0.isEmpty};if let data=try? JSONEncoder().encode(clipboard) {state.setSetting("clipboard",String(decoding:data,as:UTF8.self))}}.disabled(!changed)}
            }.padding(20).background(theme.surface,in:RoundedRectangle(cornerRadius:10))
        }.padding(30).frame(maxWidth:650,alignment:.leading).frame(maxWidth:.infinity)}.disabled(state.busy).onAppear {load()}.onChange(of:state.config.clipboard) {old,_ in if limit==old.limit && days==old.retentionDays && exclusions==old.excludedApps.joined(separator:"\n") {load()}}
    }
    private var changed:Bool {limit != state.config.clipboard.limit || days != state.config.clipboard.retentionDays || exclusions != state.config.clipboard.excludedApps.joined(separator:"\n")}
    private func load() {limit=state.config.clipboard.limit;days=state.config.clipboard.retentionDays;exclusions=state.config.clipboard.excludedApps.joined(separator:"\n")}
}

final class ManagementController:NSObject,NSWindowDelegate {
    let state:AppState
    let navigation=ManagementNavigation()
    let window:NSWindow
    init(state:AppState) {
        self.state=state
        window=NSWindow(contentRect:NSRect(x:0,y:0,width:980,height:680),styleMask:[.titled,.closable,.miniaturizable,.resizable],backing:.buffered,defer:false)
        super.init()
        window.title="Ruycast Settings";window.minSize=NSSize(width:680,height:480)
        window.isReleasedWhenClosed=false;window.delegate=self
        window.contentView=NSHostingView(rootView:ManagementView(state:state,navigation:navigation).preferredColorScheme(state.preferredColorScheme))
        state.showManagement={ [weak self] in self?.show() }
        state.showAnalytics={ [weak self] in self?.navigation.select(.dashboard);self?.state.analytics.flush();self?.show()}
    }
    func show() {
        state.hidePalette?();state.message="";state.managementVisible=true
        if !window.isVisible {
            let mouse=NSEvent.mouseLocation
            if let frame=(NSScreen.screens.first {$0.frame.contains(mouse)} ?? NSScreen.main)?.visibleFrame {
                var target=window.frame;target.size.width=min(target.width,frame.width);target.size.height=min(target.height,frame.height)
                target.origin=NSPoint(x:frame.midX-target.width/2,y:frame.midY-target.height/2)
                window.setFrame(target,display:false)
            }
        }
        window.deminiaturize(nil);window.makeKeyAndOrderFront(nil);NSApp.activate(ignoringOtherApps:true)
    }
    func windowDidResignKey(_ notification:Notification) {state.endShortcutRecording();if let button=window.firstResponder as? ShortcutRecordButton {button.end()}}
    func windowWillClose(_ notification:Notification) {state.endShortcutRecording();state.managementVisible=false}
}
