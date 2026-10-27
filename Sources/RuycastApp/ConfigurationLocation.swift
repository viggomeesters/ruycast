import AppKit
import Darwin
import LauncherCore
import LauncherModels

/// Machine-local selector; only config.json belongs in the portable directory.
enum ConfigurationLocation {
    struct Selection:Codable {let directory:String}
    static var override:String? {
        guard let value=ProcessInfo.processInfo.environment["RUYCAST_CONFIG_DIR"],!value.isEmpty else {return nil}
        return value
    }
    static func absolute(_ path:String)throws->URL {
        let expanded=NSString(string:path).expandingTildeInPath
        guard expanded.hasPrefix("/"),!expanded.contains("\n"),!expanded.contains("\r") else {throw LauncherError("Choose an absolute configuration folder path.")}
        guard !expanded.split(separator:"/").contains("..") else {throw LauncherError("Choose a configuration folder without '..' components.")}
        return URL(fileURLWithPath:expanded,isDirectory:true).standardizedFileURL
    }
    static func directory(root:URL)throws->URL {
        if let override {return try absolute(override)}
        let selector=root.appendingPathComponent("config-location.json")
        guard FileManager.default.fileExists(atPath:selector.path) else {return root}
        do {
            let selected=try Files.decode(Selection.self,data:Data(contentsOf:selector))
            return try absolute(selected.directory)
        } catch {throw LauncherError("Configuration location is invalid. Fix \(selector.path).")}
    }
    static func configURL(root:URL)throws->URL {try directory(root:root).appendingPathComponent("config.json")}
    static func initialize(root:URL)throws->URL {
        try Files.securePrivateDirectory(root)
        let url=try configURL(root:root)
        if !FileManager.default.fileExists(atPath:url.path) {
            guard override != nil || !FileManager.default.fileExists(atPath:root.appendingPathComponent("config-location.json").path) else {throw LauncherError("Configuration is unavailable at \(url.path). Restore the folder or fix config-location.json.")}
            try Files.createPrivate(Files.encode(Configuration.initial),to:url)
        }
        return url
    }
    static func relocate(root:URL,to path:String,expected:Data?=nil)throws->URL {
        guard override==nil else {throw LauncherError("RUYCAST_CONFIG_DIR controls this configuration. Change or unset it before choosing another folder.")}
        try Files.securePrivateDirectory(root)
        let destination=try absolute(path)
        // Native and CLI writers use this same local lock. Never block the main
        // queue while a CLI writer is waiting for its reload RPC.
        let lock=open(root.appendingPathComponent("config.lock").path,O_CREAT|O_RDWR|O_CLOEXEC|O_NOFOLLOW,0o600)
        guard lock>=0 else {throw LauncherError("Could not lock configuration.")};defer {close(lock)}
        guard flock(lock,LOCK_EX|LOCK_NB)==0 else {throw LauncherError("Configuration is being saved. Try again.")};defer {flock(lock,LOCK_UN)}
        let source=try configURL(root:root)
        if destination.resolvingSymlinksInPath()==source.deletingLastPathComponent().resolvingSymlinksInPath() {return destination}
        let bytes=try Data(contentsOf:source);_=try Files.config(data:bytes)
        if let expected,expected != bytes {throw LauncherError("Configuration changed. Reload and try again.")}
        try FileManager.default.createDirectory(at:destination,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        let target=destination.appendingPathComponent("config.json")
        do {try Files.createPrivate(bytes,to:target)} catch {throw LauncherError("The destination already contains a configuration or is not writable. Choose an empty folder.")}
        do {
            guard try Data(contentsOf:source)==bytes else {throw LauncherError("Configuration changed during the move. Try again.")}
            try Files.save(Selection(directory:destination.path),to:root.appendingPathComponent("config-location.json"))
        } catch {
            // Remove only the file created here, preserving any external edit.
            if (try? Data(contentsOf:target))==bytes {try? FileManager.default.removeItem(at:target)}
            throw error
        }
        // Keep the original bytes as a recovery copy; no local history moves.
        return destination
    }
}

extension AppState {
    func relocateConfiguration(to path:String)throws {
        guard !busy else {throw LauncherError("Wait until configuration has finished saving.")}
        let destination=try ConfigurationLocation.relocate(root:root,to:path,expected:configurationSnapshot)
        configDirectory=destination
        observeConfiguration()
    }
    func chooseConfigurationDirectory() {
        let panel=NSOpenPanel();panel.canChooseDirectories=true;panel.canChooseFiles=false;panel.canCreateDirectories=true;panel.allowsMultipleSelection=false
        panel.message="Choose a folder for your configuration.";panel.prompt="Choose";panel.directoryURL=configDirectory
        panel.begin { [weak self] response in
            guard response == .OK,let url=panel.url,let self else {return}
            do {try self.relocateConfiguration(to:url.path);self.message="";self.showToast?("Configuration folder updated",true)}
            catch {self.message=error.localizedDescription}
        }
    }
}
