import Foundation
import Security
import CryptoKit
import LauncherCore
import LauncherModels

/// Effects are handled by the host, keeping clipboard and desktop input out of storage logic.
enum UtilityEffect {
    case saved(String)
    case paste(String)
    case copy(String,secret:Bool)
}

final class Utilities {
    struct Entry:Codable,Equatable {var value:String;var created:String}
    struct Profile:Codable {var savedText:String?;var textHistory:[Entry]=[];var usernameHistory:[Entry]=[]}
    struct Storage {
        var readPasswords:() throws -> Data? = Utilities.readPasswords
        var writePasswords:(Data) throws -> Void = Utilities.writePasswords
        var random:(Int) throws -> [UInt8] = {count in
            var bytes=[UInt8](repeating:0,count:count)
            guard SecRandomCopyBytes(kSecRandomDefault,count,&bytes)==errSecSuccess else {throw LauncherError("Could not generate secure random data.")}
            return bytes
        }
        var now:() -> Date = Date.init
    }
    private let root:URL
    private let storage:Storage
    private let lock=NSLock()
    private var selected:[String:(String,Entry)]=[:]
    init(root:URL,storage:Storage=Storage()) {self.root=root;self.storage=storage}
    private static let keychainService="viggo-raycast-utilities"
    private static let keychainAccount="password-history"
    private static var keychainQuery:[String:Any] {[kSecClass as String:kSecClassGenericPassword,kSecAttrService as String:keychainService,kSecAttrAccount as String:keychainAccount]}
    private static func readPasswords() throws -> Data? {
        var query=keychainQuery;query[kSecReturnData as String]=true;query[kSecMatchLimit as String]=kSecMatchLimitOne
        var result:CFTypeRef?
        let status=SecItemCopyMatching(query as CFDictionary,&result)
        if status==errSecItemNotFound {return nil}
        guard status==errSecSuccess,let data=result as? Data else {throw LauncherError("Could not read password history from Keychain (\(status)).")}
        return data
    }
    private static func writePasswords(_ data:Data) throws {
        let changes=[kSecValueData as String:data]
        var status=SecItemUpdate(keychainQuery as CFDictionary,changes as CFDictionary)
        if status==errSecItemNotFound {
            var item=keychainQuery;item[kSecValueData as String]=data
            status=SecItemAdd(item as CFDictionary,nil)
        }
        guard status==errSecSuccess else {throw LauncherError("Could not save password history in Keychain (\(status)).")}
    }
    private func profileURL() throws -> URL {
        let manager=FileManager.default
        if !manager.fileExists(atPath:root.path) {try manager.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])}
        let attributes=try manager.attributesOfItem(atPath:root.path)
        guard attributes[.type] as? FileAttributeType == .typeDirectory else {throw LauncherError("Utilities storage must be a private folder.")}
        let url=root.appendingPathComponent("utilities.json")
        if let attributes=try? manager.attributesOfItem(atPath:url.path) {
            guard attributes[.type] as? FileAttributeType == .typeRegular else {throw LauncherError("Utilities storage must be a regular file.")}
            guard (attributes[.size] as? NSNumber)?.intValue ?? 0 <= 12_000_000 else {throw LauncherError("Utilities history is too large to load.")}
        }
        return url
    }
    private func readProfile() throws -> Profile {
        let url=try profileURL()
        guard FileManager.default.fileExists(atPath:url.path) else {return Profile()}
        do {return try JSONDecoder().decode(Profile.self,from:Data(contentsOf:url))}
        catch {throw LauncherError("Utilities history could not be read. Keep the file and restore a valid copy.")}
    }
    private func saveProfile(_ profile:Profile) throws {try Files.save(profile,to:profileURL())}
    private func passwordEntries() throws -> [Entry] {
        guard let bytes=try storage.readPasswords() else {return []}
        guard bytes.count<=12_000_000 else {throw LauncherError("Password history is too large to load.")}
        do {return try JSONDecoder().decode([Entry].self,from:bytes)}
        catch {throw LauncherError("Password history has an invalid format. The Keychain entry was preserved.")}
    }
    private func created()->String {ISO8601DateFormatter().string(from:storage.now())}
    static func preview(_ text:String)->String {
        let compact=text.split(whereSeparator:{$0.isWhitespace}).joined(separator:" ")
        return compact.isEmpty ? "(empty)":String(compact.prefix(100))+(compact.count>100 ? "…":"")
    }
    func commands(mode:String,query:String) throws -> [Command] {
        lock.lock();defer {lock.unlock()}
        let kind:String,entries:[Entry]
        switch mode {
        case "utility-text-history","text-history":kind="text";entries=try readProfile().textHistory
        case "utility-password-history","password-history":kind="password";entries=try passwordEntries()
        case "utility-username-history","username-history":kind="username";entries=try readProfile().usernameHistory
        default:return []
        }
        selected=selected.filter {$0.value.0 != kind}
        return entries.prefix(kind=="password" ? 50:100).enumerated().compactMap {index,entry in
            let title=kind=="password" ? "Password · \(entry.created)":Self.preview(entry.value)
            guard query.isEmpty || title.localizedCaseInsensitiveContains(query) else {return nil}
            // Secret values never enter command payloads, aliases, IDs, or logs.
            let key="\(kind):\(index):\(entry.created)"
            let token=SHA256.hash(data:Data(key.utf8)).map {String(format:"%02x",$0)}.joined()
            selected[token]=(kind,entry)
            return Command(id:"utility.history."+token,title:title,kind:"builtin",target:"utility.history:"+token)
        }
    }
    func importProfile(_ value:[String:Any]) throws -> [String:Int] {
        lock.lock();defer {lock.unlock()}
        guard Set(value.keys).isSubset(of:["savedText","textHistory","usernameHistory"]) else {throw LauncherError("Unsupported utilities import field.")}
        let incoming=try JSONDecoder().decode(Profile.self,from:JSONSerialization.data(withJSONObject:value))
        guard (incoming.savedText?.utf8.count ?? 0)<=100_000,incoming.textHistory.count<=100,incoming.usernameHistory.count<=100,
              (incoming.textHistory+incoming.usernameHistory).allSatisfy({$0.value.utf8.count<=100_000 && ISO8601DateFormatter().date(from:$0.created) != nil}) else {throw LauncherError("Invalid utilities import.")}
        var profile=try readProfile();var added=0
        if profile.savedText==nil {profile.savedText=incoming.savedText}
        for item in incoming.textHistory where !profile.textHistory.contains(where:{$0.value==item.value}) {profile.textHistory.append(item);added+=1}
        for item in incoming.usernameHistory where !profile.usernameHistory.contains(where:{$0.value==item.value}) {profile.usernameHistory.append(item);added+=1}
        profile.textHistory=Array(profile.textHistory.prefix(100));profile.usernameHistory=Array(profile.usernameHistory.prefix(100));try saveProfile(profile)
        return ["added":added,"textHistory":profile.textHistory.count,"usernameHistory":profile.usernameHistory.count]
    }
    func execute(target:String,input:String?=nil) throws -> UtilityEffect {
        lock.lock();defer {lock.unlock()}
        switch target {
        case "utility-set-text":
            guard let input else {throw LauncherError("Enter the text to save.")}
            guard input.utf8.count<=100_000 else {throw LauncherError("Saved text must be smaller than 100 KB.")}
            var profile=try readProfile();profile.savedText=input
            profile.textHistory.removeAll {$0.value==input}
            profile.textHistory.insert(Entry(value:input,created:created()),at:0)
            profile.textHistory=Array(profile.textHistory.prefix(100));try saveProfile(profile)
            return .saved("Text saved")
        case "utility-paste-text":
            guard let text=try readProfile().savedText,!text.isEmpty else {throw LauncherError("Save text with Set Text first.")}
            return .paste(text)
        case "utility-password":
            let bytes=try storage.random(24)
            guard bytes.count==24 else {throw LauncherError("Could not generate a password.")}
            let value=Data(bytes).base64EncodedString().replacingOccurrences(of:"+",with:"-").replacingOccurrences(of:"/",with:"_").replacingOccurrences(of:"=",with:"")
            let entries=Array(([Entry(value:value,created:created())]+(try passwordEntries())).prefix(50))
            try storage.writePasswords(JSONEncoder().encode(entries))
            return .copy(value,secret:true)
        case "utility-username":
            let bytes=try storage.random(4),words=["amber","atlas","cedar","luna","moss","north","raven","river","solar","violet"]
            guard bytes.count==4 else {throw LauncherError("Could not generate a username.")}
            let value="\(words[Int(bytes[0]) % words.count])-\(words[Int(bytes[1]) % words.count])-\(String(format:"%02x%02x",bytes[2],bytes[3]))"
            var profile=try readProfile();profile.usernameHistory.insert(Entry(value:value,created:created()),at:0)
            profile.usernameHistory=Array(profile.usernameHistory.prefix(100));try saveProfile(profile)
            return .copy(value,secret:false)
        default:
            guard target.hasPrefix("utility.history:"),let entry=selected[String(target.dropFirst("utility.history:".count))] else {throw LauncherError("This history item expired. Search again.")}
            if entry.0=="text" {var profile=try readProfile();profile.savedText=entry.1.value;try saveProfile(profile);return .saved("Active text changed")}
            return .copy(entry.1.value,secret:entry.0=="password")
        }
    }
}
