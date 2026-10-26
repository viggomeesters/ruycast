import AppKit
import SwiftUI
import LauncherCore

struct ScriptChoiceSession {let commandID:String;let choices:ScriptChoices;let source:String}

extension AppState {
    var filteredScriptChoices:[ScriptChoice] {scriptChoices?.choices.filtered(scriptChoiceQuery) ?? []}
    func moveScriptChoice(_ delta:Int) {scriptChoiceSelection=max(0,min(max(0,filteredScriptChoices.count-1),scriptChoiceSelection+delta))}
    func submitScriptChoice() {
        guard !busy,let session=scriptChoices,filteredScriptChoices.indices.contains(scriptChoiceSelection) else {return}
        perform(session.commandID,values:["input":filteredScriptChoices[scriptChoiceSelection].value],source:session.source)
    }
    func cancelScriptChoices() {
        guard !busy else {return};scriptChoices=nil;scriptChoiceQuery="";scriptChoiceSelection=0;message="";mode="commands";restoreSearchFocus?()
    }
    func reloadScriptChoices() {
        guard !busy,let session=scriptChoices else {return};message="";perform(session.commandID,source:session.source)
    }
}

struct ScriptChoiceView:View {
    @ObservedObject var state:AppState
    let session:ScriptChoiceSession
    var body:some View {
        VStack(spacing:0) {
            HStack(spacing:12) {
                Button {state.cancelScriptChoices()} label:{Image(systemName:"chevron.left")}.buttonStyle(.plain).accessibilityLabel("Terug")
                PaletteSearchField(text:$state.scriptChoiceQuery,placeholder:session.choices.placeholder,onMove:state.moveScriptChoice,onSubmit:state.submitScriptChoice,onCancel:state.cancelScriptChoices,onActions:{})
            }.padding(.horizontal,20).frame(height:62)
            Divider().opacity(0.5)
            ScrollViewReader {proxy in
                ScrollView {
                    LazyVStack(spacing:3) {
                        if state.filteredScriptChoices.isEmpty {Text("Geen resultaten").foregroundStyle(.secondary).padding(24)}
                        ForEach(Array(state.filteredScriptChoices.enumerated()),id:\.element.id) {index,item in
                            Button {state.scriptChoiceSelection=index;state.submitScriptChoice()} label:{
                                HStack(spacing:12) {
                                    Image(systemName:"folder").foregroundStyle(.secondary).frame(width:22)
                                    VStack(alignment:.leading,spacing:3) {Text(item.title).lineLimit(1);Text(item.subtitle).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1)}
                                    Spacer()
                                }.padding(.horizontal,14).padding(.vertical,10).frame(maxWidth:.infinity,alignment:.leading)
                                    .background(state.scriptChoiceSelection==index ? Color.primary.opacity(0.09):.clear,in:RoundedRectangle(cornerRadius:8))
                            }.buttonStyle(.plain).disabled(state.busy).id(item.id).accessibilityLabel(item.title+", "+item.subtitle)
                        }
                    }.padding(8)
                }.onChange(of:state.scriptChoiceSelection) {index in if state.filteredScriptChoices.indices.contains(index) {proxy.scrollTo(state.filteredScriptChoices[index].id,anchor:.center)}}
            }
            if !state.message.isEmpty {Text(state.message).font(.caption).foregroundStyle(.orange).padding(.horizontal,16).padding(.vertical,8)}
            Divider().opacity(0.5)
            HStack {
                Text(session.choices.title).foregroundStyle(.secondary)
                Spacer()
                if state.busy {ProgressView().controlSize(.small)}
                Button("Opnieuw lezen") {state.reloadScriptChoices()}.buttonStyle(.plain).disabled(state.busy)
                Text("Kies ↵").foregroundStyle(.secondary)
            }.font(.system(size:11)).padding(.horizontal,16).frame(height:38)
        }.onExitCommand {state.cancelScriptChoices()}
    }
}
