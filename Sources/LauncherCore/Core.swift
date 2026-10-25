import Foundation

@_silgen_name("rustcast_call") private func rustcast_call(_ op:UnsafePointer<CChar>,_ payload:UnsafePointer<CChar>)->UnsafeMutablePointer<CChar>?
@_silgen_name("rustcast_free") private func rustcast_free(_ pointer:UnsafeMutablePointer<CChar>)
public struct LauncherError:Error,LocalizedError {
    public let message:String
    public init(_ message:String) {self.message=message}
    public var errorDescription:String? {message}
}
public enum RustCore {
    public static func call(_ op:String,_ payload:Any) throws -> Any {
        let data=try JSONSerialization.data(withJSONObject:payload,options:[.fragmentsAllowed,.sortedKeys])
        let text=String(decoding:data,as:UTF8.self)
        guard let pointer=op.withCString({a in text.withCString {b in rustcast_call(a,b)}}) else {throw LauncherError("Rust core returned no response.")}
        defer {rustcast_free(pointer)}
        guard let bytes=String(cString:pointer).data(using:.utf8),let response=try JSONSerialization.jsonObject(with:bytes) as? [String:Any] else {throw LauncherError("Invalid Rust core response.")}
        guard response["ok"] as? Bool==true else {throw LauncherError(response["error"] as? String ?? "Rust core failed.")}
        return response["result"] ?? NSNull()
    }
    public static func decode<T:Decodable>(_ type:T.Type,op:String,payload:Any) throws -> T {
        let value=try call(op,payload)
        return try Files.decode(type,data:JSONSerialization.data(withJSONObject:value,options:.fragmentsAllowed))
    }
    public static func object<T:Encodable>(_ value:T) throws -> Any {try JSONSerialization.jsonObject(with:Files.encode(value),options:.fragmentsAllowed)}
}
public struct Command:Codable,Identifiable,Equatable {
    public var id:String;public var title:String;public var kind:String;public var target:String
    public var aliases:[String];public var arguments:[String];public var hotkey:String?;public var timeout:Double?;public var inputPrompt:String?;public var keyword:String?;public var applicationID:String?;public var enabled:Bool?;public var isEnabled:Bool {enabled != false}
    public init(id:String,title:String,kind:String,target:String,aliases:[String]=[],arguments:[String]=[],hotkey:String?=nil,timeout:Double?=nil,inputPrompt:String?=nil,keyword:String?=nil,applicationID:String?=nil,enabled:Bool?=nil) {self.id=id;self.title=title;self.kind=kind;self.target=target;self.aliases=aliases;self.arguments=arguments;self.hotkey=hotkey;self.timeout=timeout;self.inputPrompt=inputPrompt;self.keyword=keyword;self.applicationID=applicationID;self.enabled=enabled}
    enum CodingKeys:String,CodingKey {case id,title,kind,target,aliases,arguments,hotkey,timeout,inputPrompt,keyword,applicationID,enabled}
    public init(from decoder:Decoder) throws {
        let c=try decoder.container(keyedBy:CodingKeys.self)
        id=try c.decode(String.self,forKey:.id);title=try c.decode(String.self,forKey:.title);kind=try c.decode(String.self,forKey:.kind);target=try c.decode(String.self,forKey:.target)
        aliases=try c.decodeIfPresent([String].self,forKey:.aliases) ?? [];arguments=try c.decodeIfPresent([String].self,forKey:.arguments) ?? []
        hotkey=try c.decodeIfPresent(String.self,forKey:.hotkey);timeout=try c.decodeIfPresent(Double.self,forKey:.timeout);inputPrompt=try c.decodeIfPresent(String.self,forKey:.inputPrompt);keyword=try c.decodeIfPresent(String.self,forKey:.keyword);applicationID=try c.decodeIfPresent(String.self,forKey:.applicationID);enabled=try c.decodeIfPresent(Bool.self,forKey:.enabled)
    }
}
public struct ClipboardSettings:Codable,Equatable {
    public var enabled:Bool;public var limit:Int;public var maxTextBytes:Int;public var retentionDays:Int;public var excludedApps:[String];public var maxAssetBytes:Int?;public var maxTotalAssetBytes:Int?
    public init(enabled:Bool=true,limit:Int=200,maxTextBytes:Int=100_000,retentionDays:Int=30,maxAssetBytes:Int?=nil,maxTotalAssetBytes:Int?=nil,excludedApps:[String]=["com.apple.Passwords","com.agilebits.onepassword7","com.1password.1password"]) {self.enabled=enabled;self.limit=limit;self.maxTextBytes=maxTextBytes;self.retentionDays=retentionDays;self.excludedApps=excludedApps;self.maxAssetBytes=maxAssetBytes;self.maxTotalAssetBytes=maxTotalAssetBytes}
}
public struct Configuration:Codable,Equatable {
    public var version:Int;public var launcherHotkey:String;public var clipboardHotkey:String;public var clipboard:ClipboardSettings;public var commands:[Command];public var snippetExpansionEnabled:Bool?;public var hyperKeyEnabled:Bool?;public var theme:String?;public var menuBarVisible:Bool?
    public static let layouts=["left-half","right-half","first-third","center-third","last-third","first-two-thirds","last-two-thirds","maximize","restore","next-screen","previous-screen"]
    public static var initial:Configuration {try! RustCore.decode(Configuration.self,op:"defaults",payload:[:])}
    public func validate() throws {_=try RustCore.call("validate",RustCore.object(self))}
    public func search(_ query:String)->[Command] {(try? RustCore.decode([Command].self,op:"search",payload:["commands":try RustCore.object(commands),"query":query])) ?? []}
    public func resolve(_ name:String)->Command? {commands.first {$0.isEnabled && ($0.id.lowercased()==name.lowercased() || $0.aliases.contains {$0.lowercased()==name.lowercased()})}}
}
public struct Shortcut {
    public let modifiers:Set<String>;public let key:String
    public var canonical:String {modifiers.sorted().joined(separator:"+")+"+"+key}
    public static let keyCodes:[String:UInt32]=["a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,"b":11,"q":12,"w":13,"e":14,"r":15,"y":16,"t":17,"1":18,"2":19,"3":20,"4":21,"6":22,"5":23,"9":25,"7":26,"8":28,"0":29,"]":30,"o":31,"u":32,"[":33,"i":34,"p":35,"l":37,"j":38,"k":40,"n":45,"m":46,"tab":48,"space":49,"return":36,"left":123,"right":124,"down":125,"up":126,"left-control":59,"left-option":58,"\'":39,";":41,"`":50,"delete":51,"backspace":51,"escape":53,",":43,".":47,"/":44,"-":27,"=":24,"\\":42,"f1":122,"f2":120,"f3":99,"f4":118,"f5":96,"f6":97,"f7":98,"f8":100,"f9":101,"f10":109,"f11":103,"f12":111]
    public init(_ value:String) throws {
        guard let canonical=try RustCore.call("shortcut",value) as? String else {throw LauncherError("Invalid Rust shortcut response.")}
        let parts=canonical.split(separator:"+").map(String.init);key=parts.last!;modifiers=Set(parts.dropLast())
    }
}
public struct Clip:Codable,Identifiable,Equatable {
    public var id:String;public var text:String;public var date:Date;public var source:String;public var contentType:String?;public var dataFile:String?;public var fileURLs:[String]?
    public init(text:String,source:String,date:Date=Date(),contentType:String?=nil,dataFile:String?=nil,fileURLs:[String]?=nil) {id=UUID().uuidString;self.text=text;self.source=source;self.contentType=contentType;self.dataFile=dataFile;self.fileURLs=fileURLs;self.date=Date(timeIntervalSince1970:date.timeIntervalSince1970.rounded(.down))}
}
public enum History {
    public static func insert(_ clip:Clip,into items:[Clip],settings:ClipboardSettings,now:Date=Date())->[Clip] {apply(items,clip:clip,settings:settings,now:now)}
    public static func prune(_ items:[Clip],settings:ClipboardSettings,now:Date=Date())->[Clip] {apply(items,clip:nil,settings:settings,now:now)}
    public static func updateChecked(_ items:[Clip],clip:Clip?=nil,settings:ClipboardSettings,now:Date=Date()) throws -> [Clip] {
        let payload:[String:Any]=["items":try RustCore.object(items),"clip":try RustCore.object(clip),"settings":try RustCore.object(settings),"now":ISO8601DateFormatter().string(from:now)]
        return try RustCore.decode([Clip].self,op:"history",payload:payload)
    }
    private static func apply(_ items:[Clip],clip:Clip?,settings:ClipboardSettings,now:Date)->[Clip] {(try? updateChecked(items,clip:clip,settings:settings,now:now)) ?? []}
}
public struct Frame:Codable,Equatable {
    public var x:Double;public var y:Double;public var width:Double;public var height:Double
    public init(x:Double,y:Double,width:Double,height:Double) {self.x=x;self.y=y;self.width=width;self.height=height}
    public func layout(_ name:String) throws -> Frame {try RustCore.decode(Frame.self,op:"layout",payload:["frame":try RustCore.object(self),"name":name])}
}
public enum Files {
    public static func encode<T:Encodable>(_ value:T) throws -> Data {let e=JSONEncoder();e.outputFormatting=[.prettyPrinted,.sortedKeys,.withoutEscapingSlashes];e.dateEncodingStrategy = .iso8601;return try e.encode(value)}
    public static func decode<T:Decodable>(_ type:T.Type,data:Data) throws -> T {let d=JSONDecoder();d.dateDecodingStrategy = .iso8601;return try d.decode(type,from:data)}
    public static func config(_ path:URL) throws -> Configuration {try config(data:Data(contentsOf:path))}
    public static func config(data:Data) throws -> Configuration {let raw=try JSONSerialization.jsonObject(with:data);return try RustCore.decode(Configuration.self,op:"validate",payload:raw)}
    public static func save<T:Encodable>(_ value:T,to url:URL) throws {try FileManager.default.createDirectory(at:url.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);try encode(value).write(to:url,options:.atomic);try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:url.path)}
}

@_silgen_name("rustcast_index_new") private func rustcast_index_new(_ catalog:UnsafePointer<CChar>)->OpaquePointer?
@_silgen_name("rustcast_index_search") private func rustcast_index_search(_ index:OpaquePointer,_ query:UnsafePointer<UInt8>,_ length:Int)->UnsafeMutablePointer<CChar>?
@_silgen_name("rustcast_index_free") private func rustcast_index_free(_ index:OpaquePointer)

/// Main-thread owned immutable catalog. Only query bytes and result indices cross the ABI.
public final class CommandSearchIndex {
    public let commands:[Command]
    private let index:OpaquePointer
    public init(commands:[Command]) throws {
        self.commands=commands
        let bytes=try JSONEncoder().encode(commands)
        guard let pointer=String(decoding:bytes,as:UTF8.self).withCString({rustcast_index_new($0)}) else {throw LauncherError("Could not index commands.")}
        index=pointer
    }
    deinit {rustcast_index_free(index)}
    public func search(_ query:String)->[Command] {
        let bytes=Array(query.utf8)+[0]
        guard let result=bytes.withUnsafeBufferPointer({rustcast_index_search(index,$0.baseAddress!,bytes.count-1)}) else {return []}
        defer {rustcast_free(result)}
        guard let ids=try? JSONDecoder().decode([Int].self,from:Data(String(cString:result).utf8)),ids.allSatisfy({commands.indices.contains($0)}) else {return []}
        return ids.map {commands[$0]}
    }
}
