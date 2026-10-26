import SwiftUI
import AppKit
import LauncherCore

extension AppState {
    static func keyboardHealth(hyperEnabled:Bool,snippetsEnabled:Bool,trusted:Bool,raycastRunning:Bool,commandShortcutsEnabled:Bool=false,inputMonitoring:Bool=true)->Bool {
        !(hyperEnabled || snippetsEnabled || commandShortcutsEnabled) || (trusted && inputMonitoring && !raycastRunning)
    }
    var runtimeHealthy:Bool {
        configError==nil && clipboardCleanupError==nil && analytics.storageError==nil && (hotkeys?.warnings.isEmpty ?? true) &&
        (headless || Self.keyboardHealth(hyperEnabled:config.hyperKeyEnabled==true,snippetsEnabled:config.snippetExpansionEnabled==true,trusted:windows.trusted,raycastRunning:!NSRunningApplication.runningApplications(withBundleIdentifier:"com.raycast.macos").isEmpty,commandShortcutsEnabled:Hotkeys.hasCommandBindings(in:config),inputMonitoring:CGPreflightListenEventAccess()))
    }
    var preferredColorScheme:ColorScheme? {
        switch config.theme ?? "system" {case "light":return .light;case "dark","graphite":return .dark;default:return nil}
    }
    func captureInlineQuery() {
        guard mode=="commands",inlineCommand==nil,let separator=query.firstIndex(where:{$0.isWhitespace}) else {return}
        let alias=String(query[..<separator]),tail=String(query[query.index(after:separator)...])
        guard let command=config.resolve(alias),Templates.fields(command).count==1,Templates.fields(command)[0].options.isEmpty else {return}
        inlineSource="launcher";selection=0;inlineCommand=command;inlineInput=tail
    }
    func beginInlineSelection() {
        guard inlineCommand==nil,let command=selectedCommand,Templates.fields(command).count==1,Templates.fields(command)[0].options.isEmpty else {return}
        begin(command)
    }
    func expandInlineInput() {
        guard let command=inlineCommand,let field=Templates.fields(command).first else {return}
        let values=[field.id:inlineInput],source=inlineSource
        inlineCommand=nil;inlineInput=""
        form=CommandForm(command:command,isRun:true,values:values,source:source)
    }
    func cancelInlineInput() {
        inlineCommand=nil;inlineInput=""
        if let first=query.split(whereSeparator:{$0.isWhitespace}).first,config.resolve(String(first)) != nil {query=String(first)}
        restoreSearchFocus?()
    }
    func submitInlineInput() {
        guard !busy,let command=inlineCommand,let field=Templates.fields(command).first else {return}
        let values=[field.id:inlineInput]
        perform(command.id,values:values,source:inlineSource)
    }
}
