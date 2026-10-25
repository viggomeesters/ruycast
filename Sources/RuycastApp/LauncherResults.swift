import AppKit
import SwiftUI
import LauncherCore

/// Keep native descendants attached to one window. Replacing hosting subtrees
/// on every mode switch grows AppKit NSCell dependency bookkeeping on macOS 26.
struct PersistentLauncherResults:NSViewRepresentable {
    @ObservedObject var state:AppState
    @Environment(\.colorScheme) private var scheme
    func makeNSView(context:Context)->LauncherResultsContainer {LauncherResultsContainer(state:state)}
    func updateNSView(_ view:LauncherResultsContainer,context:Context) {
        let hidden=state.mode=="clipboard"
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
    @ObservedObject var state:AppState
    @Environment(\.colorScheme) private var scheme
    private var highlight:Color {Color.primary.opacity(scheme == .dark ? 0.085:0.065)}
    private func kindLabel(_ kind:String)->String {["snippet":"Snippet","app":"Application","url":"Quicklink","path":"File or Folder","script":"Command","window":"Window","builtin":"Command"][kind] ?? "Command"}
    var body:some View {
        let commands=state.visibleCommands
                ScrollViewReader {proxy in ScrollView {
                    LazyVStack(alignment:.leading,spacing:2) {
                        HStack {
                            Text(state.mode=="processes" ? "Processes · \(commands.count)":state.query.isEmpty ? "Suggestions":"Results").font(.system(size:11,weight:.medium)).foregroundStyle(.secondary)
                            Spacer()
                            if state.mode=="processes" {Picker("Sort processes",selection:Binding(get:{state.processSort},set:state.setProcessSort)) {ForEach(ProcessSort.allCases,id:\.self) {Text($0.rawValue).tag($0)}}.labelsHidden().frame(width:135)}
                        }.padding(.horizontal,8).padding(.top,6).padding(.bottom,8)
                            if commands.isEmpty {Text("No commands found").foregroundStyle(.secondary).padding(24)}
                            ForEach(Array(commands.enumerated()),id:\.element.id) { index,cmd in
                                Button {state.selection=index;state.activateSelection()} label:{
                                    HStack(spacing:12) {
                                        if let process=state.processResources[cmd.id],state.mode=="processes" {ProcessIcon(resource:process)} else {CommandIcon(command:cmd)}
                                        HStack(spacing:8) {
                                            Text(cmd.title).font(.system(size:13,weight:.semibold)).lineLimit(1).layoutPriority(2)
                                            if state.mode=="processes",let process=state.processResources[cmd.id] {Text(String(process.id)).font(.system(size:11)).foregroundStyle(.tertiary)}
                                            if let alias=cmd.aliases.first {
                                                Text(alias).font(.system(size:11,weight:.medium)).foregroundStyle(.secondary)
                                                    .padding(.horizontal,5).padding(.vertical,2)
                                                    .background(Color.primary.opacity(0.08),in:RoundedRectangle(cornerRadius:4))
                                                    .lineLimit(1).frame(maxWidth:120,alignment:.leading).fixedSize(horizontal:false,vertical:true)
                                            }
                                        }
                                        Spacer(minLength:10)
                                        if let key=cmd.hotkey {
                                            ShortcutBadge(value:key)
                                        }
                                        if let process=state.processResources[cmd.id],state.mode=="processes" {
                                            HStack(spacing:5) {Image(systemName:"cpu");Text(String(format:"%.1f%%",process.cpuPercent)).frame(width:52,alignment:.trailing);Image(systemName:"memorychip");Text(memoryLabel(process.memoryBytes)).frame(width:68,alignment:.trailing)}.font(.system(size:12)).monospacedDigit().foregroundStyle(.secondary)
                                        } else {Text(kindLabel(cmd.kind)).font(.system(size:12)).foregroundStyle(.secondary).fixedSize()}
                                    }.padding(.horizontal,10).frame(height:38)
                                        .frame(maxWidth:.infinity,alignment:.leading)
                                        .background(index==state.selection ? highlight:Color.clear,in:RoundedRectangle(cornerRadius:8))
                                        .contentShape(Rectangle())
                                }.buttonStyle(.plain).id(cmd.id)
                            }

                    }.padding(.horizontal,8).padding(.bottom,12)
                }.scrollIndicators(.hidden).onChange(of:state.selection) {_,index in if commands.indices.contains(index) {proxy.scrollTo(commands[index].id)}}}
    }
}
