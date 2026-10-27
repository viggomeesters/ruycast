import Foundation
import LauncherModels

public enum Files {
    public static func encode<T:Encodable>(_ value:T) throws -> Data {let e=JSONEncoder();e.outputFormatting=[.prettyPrinted,.sortedKeys,.withoutEscapingSlashes];e.dateEncodingStrategy = .iso8601;return try e.encode(value)}
    public static func decode<T:Decodable>(_ type:T.Type,data:Data) throws -> T {let d=JSONDecoder();d.dateDecodingStrategy = .iso8601;return try d.decode(type,from:data)}
    public static func config(_ path:URL) throws -> Configuration {try config(data:Data(contentsOf:path))}
    public static func config(data:Data) throws -> Configuration {let raw=try JSONSerialization.jsonObject(with:data);return try RustCore.decode(Configuration.self,op:"validate",payload:raw)}
    public static func save<T:Encodable>(_ value:T,to url:URL) throws {try FileManager.default.createDirectory(at:url.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);try encode(value).write(to:url,options:.atomic);try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:url.path)}
}

