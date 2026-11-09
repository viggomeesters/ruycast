import Foundation
import LauncherModels
import CRuycastCore

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
public struct Shortcut {
    public let modifiers:Set<String>;public let key:String
    public var commandSide:String? {modifiers.contains("right-cmd") ? "right-cmd":modifiers.contains("left-cmd") ? "left-cmd":nil}
    public var canonical:String {modifiers.sorted().joined(separator:"+")+"+"+key}
    public static let keyCodes:[String:UInt32]=["a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,"b":11,"q":12,"w":13,"e":14,"r":15,"y":16,"t":17,"1":18,"2":19,"3":20,"4":21,"6":22,"5":23,"9":25,"7":26,"8":28,"0":29,"]":30,"o":31,"u":32,"[":33,"i":34,"p":35,"l":37,"j":38,"k":40,"n":45,"m":46,"tab":48,"space":49,"return":36,"left":123,"right":124,"down":125,"up":126,"left-control":59,"left-option":58,"\'":39,";":41,"`":50,"delete":51,"backspace":51,"escape":53,",":43,".":47,"/":44,"-":27,"=":24,"\\":42,"f1":122,"f2":120,"f3":99,"f4":118,"f5":96,"f6":97,"f7":98,"f8":100,"f9":101,"f10":109,"f11":103,"f12":111]
    public init(_ value:String) throws {
        guard let canonical=try RustCore.call("shortcut",value) as? String else {throw LauncherError("Invalid Rust shortcut response.")}
        let parts=canonical.split(separator:"+").map(String.init);key=parts.last!;modifiers=Set(parts.dropLast())
    }
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
public extension Configuration {
    static var initial:Configuration {try! RustCore.decode(Configuration.self,op:"defaults",payload:[:])}
    func validate() throws {_=try RustCore.call("validate",RustCore.object(self))}
    func search(_ query:String)->[Command] {(try? RustCore.decode([Command].self,op:"search",payload:["commands":try RustCore.object(commands),"query":query])) ?? []}
}
public extension Frame {
    func layout(_ name:String) throws -> Frame {try RustCore.decode(Frame.self,op:"layout",payload:["frame":try RustCore.object(self),"name":name])}
}
