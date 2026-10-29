import AppKit
import SwiftUI
import LauncherCore
import LauncherModels

/// Keep native descendants attached to one window. Replacing hosting subtrees
/// on every mode switch grows AppKit NSCell dependency bookkeeping on macOS 26.
struct PersistentLauncherResults:NSViewRepresentable {
    let state:AppState
    let clipboardVisible:Bool
    init(state:AppState) {self.state=state;clipboardVisible=state.mode=="clipboard"}
    @Environment(\.colorScheme) private var scheme
    func makeNSView(context:Context)->LauncherResultsContainer {LauncherResultsContainer(state:state)}
    func updateNSView(_ view:LauncherResultsContainer,context:Context) {
        let hidden=clipboardVisible
        if view.commands.isHidden != hidden {view.commands.isHidden=hidden}
        if view.clipboard.isHidden == hidden {view.clipboard.isHidden = !hidden}
        let name:NSAppearance.Name=scheme == .dark ? .darkAqua:.aqua
        if view.appearance?.name != name {view.appearance=NSAppearance(named:name)}
    }
}
final class LauncherResultsContainer:NSView {
    let commands:NSHostingView<LauncherCommandResults>
    let clipboard:NSHostingView<ClipboardHistoryView>
    init(state:AppState) {
        commands=NSHostingView(rootView:LauncherCommandResults(state:state));clipboard=NSHostingView(rootView:ClipboardHistoryView(state:state))
        super.init(frame:NSRect(x:0,y:0,width:760,height:390))
        for host:NSView in [commands,clipboard] {host.frame=bounds;host.autoresizingMask=[.width,.height];addSubview(host)}
        commands.isHidden=state.mode=="clipboard";clipboard.isHidden=state.mode != "clipboard"
    }
    required init?(coder:NSCoder) {fatalError("init(coder:) has not been implemented")}
}
struct LauncherCommandResults:View {
    let state:AppState
    @Environment(\.colorScheme) private var scheme
    private var highlight:Color {Color.primary.opacity(scheme == .dark ? 0.085:0.065)}
    private func kindLabel(_ kind:String)->String {["snippet":"Snippet","app":"Application","url":"Quicklink","path":"File or Folder","script":"Command","window":"Window","builtin":"Command"][kind] ?? "Command"}
    private func snippetBindings(_ command:Command)->String {
        var parts:[String]=[]
        if let keyword=command.keyword,!keyword.isEmpty {parts.append("Keyword "+keyword)}
        if !command.aliases.isEmpty {parts.append("Aliases "+command.aliases.joined(separator:", "))}
        return parts.joined(separator:" · ")
    }
    var body:some View {
        let commands=state.visibleCommands
        let duplicateShortcutTitles=state.mode=="shortcuts" ? Set(Dictionary(grouping:commands,by: \.title).filter {$0.value.count>1}.keys):Set<String>()
                ScrollViewReader {proxy in ScrollView {
                    LazyVStack(alignment:.leading,spacing:2) {
                        HStack {
                            Text(state.mode=="processes" ? "Processes · \(commands.count)":state.mode=="menus" ? "Menu Actions":state.mode=="windows" ? "Windows":state.mode=="shortcuts" ? "Apple Shortcuts":state.query.isEmpty ? "Suggestions":"Results").font(.system(size:11,weight:.medium)).foregroundStyle(.secondary)
                            Spacer()
                            if state.mode=="processes" {Picker("Sort processes",selection:Binding(get:{state.processSort},set:{state.setProcessSort($0)})) {ForEach(ProcessSort.allCases,id:\.self) {Text($0.rawValue).tag($0)}}.labelsHidden().frame(width:135)}
                        }.padding(.horizontal,8).padding(.top,6).padding(.bottom,8)
                            if commands.isEmpty && (state.mode != "shortcuts" || state.message.isEmpty) {Text(state.mode=="menus" ? (state.menuSearchBusy ? "Searching menu…":"No menu actions found"):state.mode=="windows" ? (state.windowSearchBusy ? "Searching windows…":"No windows found"):state.mode=="shortcuts" ? (state.shortcutSearchBusy ? "Loading shortcuts…":"No shortcuts found"):"No commands found").foregroundStyle(.secondary).padding(24)}
                            ForEach(Array(commands.enumerated()),id:\.element.id) { index,cmd in
                                Button {state.selection=index;state.activateSelection()} label:{
                                    HStack(spacing:12) {
                                        if state.mode=="processes",let process=state.processResources[cmd.id] {ProcessIcon(resource:process)}
                                        else if cmd.target.hasPrefix("system.window:") {Image(systemName:"macwindow").font(.system(size:17)).foregroundStyle(.secondary).frame(width:24,height:24).accessibilityHidden(true)}
                                        else if cmd.target.hasPrefix("system.menu:") {Image(systemName:"menubar.rectangle").font(.system(size:17)).foregroundStyle(.secondary).frame(width:24,height:24).accessibilityHidden(true)}
                                        else if AppleShortcuts.recognizes(cmd) {Image(systemName:"square.stack.3d.up.fill").font(.system(size:17)).foregroundStyle(.secondary).frame(width:24,height:24).accessibilityHidden(true)}
                                        else {CommandIcon(command:cmd)}
                                        VStack(alignment:.leading,spacing:4) {
                                          HStack(spacing:8) {
                                            Text(cmd.title).font(.system(size:13,weight:.semibold)).lineLimit(1).layoutPriority(2)
                                            if state.mode=="processes",let process=state.processResources[cmd.id] {Text(String(process.id)).font(.system(size:11)).foregroundStyle(.tertiary)}
                                            if cmd.kind != "snippet",let alias=cmd.aliases.first {
                                                Text(alias).font(.system(size:11,weight:.medium)).foregroundStyle(.secondary)
                                                    .padding(.horizontal,5).padding(.vertical,2)
                                                    .background(Color.primary.opacity(0.08),in:RoundedRectangle(cornerRadius:4))
                                                    .lineLimit(1).frame(maxWidth:120,alignment:.leading).fixedSize(horizontal:false,vertical:true)
                                            }
                                          }
                                          if duplicateShortcutTitles.contains(cmd.title),AppleShortcuts.recognizes(cmd) {Text(String(cmd.id.suffix(8))).font(.system(size:11)).monospaced().foregroundStyle(.secondary)}
                                          if let menu=state.menuActions[cmd.id] {
                                              Text(menu.application+(menu.path.isEmpty ? "":" · "+menu.path)).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1)
                                          }
                                          if let window=state.windowActions[cmd.id] {
                                              Text(window.application).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1)
                                          }
                                          if cmd.kind=="snippet" {
                                              let bindings=snippetBindings(cmd)
                                              if !bindings.isEmpty {
                                                  Text(bindings).font(.system(size:11)).foregroundStyle(.secondary)
                                                      .fixedSize(horizontal:false,vertical:true)
                                              }
                                          }
                                        }.frame(maxWidth:.infinity,alignment:.leading)
                                        Spacer(minLength:10)
                                        if let menu=state.menuActions[cmd.id],let shortcut=menu.shortcut {
                                            MenuShortcutBadge(shortcut:shortcut,application:menu.application)
                                        } else if let key=cmd.hotkey {
                                            ShortcutBadge(value:key)
                                        }
                                        if state.mode=="processes",let process=state.processResources[cmd.id] {
                                            HStack(spacing:5) {Image(systemName:"cpu");Text(String(format:"%.1f%%",process.cpuPercent)).frame(width:52,alignment:.trailing);Image(systemName:"memorychip");Text(memoryLabel(process.memoryBytes)).frame(width:68,alignment:.trailing)}.font(.system(size:12)).monospacedDigit().foregroundStyle(.secondary)
                                        } else if cmd.target.hasPrefix("system.window:") {
                                            if state.windowActions[cmd.id]?.minimized==true {Text("Minimized").font(.system(size:12)).foregroundStyle(.secondary).fixedSize()}
                                        } else if cmd.target.hasPrefix("system.menu:") {
                                            if !cmd.isEnabled {Text("Unavailable").font(.system(size:12)).foregroundStyle(.secondary).fixedSize()}
                                        } else if !AppleShortcuts.recognizes(cmd) {Text(kindLabel(cmd.kind)).font(.system(size:12)).foregroundStyle(.secondary).fixedSize()}
                                    }.padding(.horizontal,10).padding(.vertical,8).frame(minHeight:38)
                                        .frame(maxWidth:.infinity,alignment:.leading)
                                        .background(index==state.selection ? highlight:Color.clear,in:RoundedRectangle(cornerRadius:8))
                                        .contentShape(Rectangle())
                                }.buttonStyle(.plain).disabled(!cmd.isEnabled).id(cmd.id)
                            }

                    }.padding(.horizontal,8).padding(.bottom,12)
                }.scrollIndicators(.hidden).onChange(of:state.selection) {_,index in if commands.indices.contains(index) {proxy.scrollTo(commands[index].id)}}}
    }
}
