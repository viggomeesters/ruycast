import Foundation

public struct LauncherError:Error,LocalizedError {
    public let message:String
    public init(_ message:String) {self.message=message}
    public var errorDescription:String? {message}
}
public struct Command:Codable,Sendable,Identifiable,Equatable {
    public var id:String;public var title:String;public var kind:String;public var target:String
    public var aliases:[String];public var arguments:[String];public var hotkey:String?;public var timeout:Double?;public var inputPrompt:String?;public var keyword:String?;public var applicationID:String?;public var snippetOutput:String?;public var enabled:Bool?;public var isEnabled:Bool {enabled != false}
    public init(id:String,title:String,kind:String,target:String,aliases:[String]=[],arguments:[String]=[],hotkey:String?=nil,timeout:Double?=nil,inputPrompt:String?=nil,keyword:String?=nil,applicationID:String?=nil,enabled:Bool?=nil,snippetOutput:String?=nil) {self.id=id;self.title=title;self.kind=kind;self.target=target;self.aliases=aliases;self.arguments=arguments;self.hotkey=hotkey;self.timeout=timeout;self.inputPrompt=inputPrompt;self.keyword=keyword;self.applicationID=applicationID;self.enabled=enabled;self.snippetOutput=snippetOutput}
    enum CodingKeys:String,CodingKey {case id,title,kind,target,aliases,arguments,hotkey,timeout,inputPrompt,keyword,applicationID,enabled,snippetOutput}
    public init(from decoder:Decoder) throws {
        let c=try decoder.container(keyedBy:CodingKeys.self)
        id=try c.decode(String.self,forKey:.id);title=try c.decode(String.self,forKey:.title);kind=try c.decode(String.self,forKey:.kind);target=try c.decode(String.self,forKey:.target)
        aliases=try c.decodeIfPresent([String].self,forKey:.aliases) ?? [];arguments=try c.decodeIfPresent([String].self,forKey:.arguments) ?? []
        hotkey=try c.decodeIfPresent(String.self,forKey:.hotkey);timeout=try c.decodeIfPresent(Double.self,forKey:.timeout);inputPrompt=try c.decodeIfPresent(String.self,forKey:.inputPrompt);keyword=try c.decodeIfPresent(String.self,forKey:.keyword);applicationID=try c.decodeIfPresent(String.self,forKey:.applicationID);enabled=try c.decodeIfPresent(Bool.self,forKey:.enabled);snippetOutput=try c.decodeIfPresent(String.self,forKey:.snippetOutput)
    }
}
public struct ClipboardSettings:Codable,Sendable,Equatable {
    public var enabled:Bool;public var limit:Int;public var maxTextBytes:Int;public var retentionDays:Int;public var excludedApps:[String];public var maxAssetBytes:Int?;public var maxTotalAssetBytes:Int?
    public init(enabled:Bool=true,limit:Int=200,maxTextBytes:Int=100_000,retentionDays:Int=30,maxAssetBytes:Int?=nil,maxTotalAssetBytes:Int?=nil,excludedApps:[String]=["com.apple.Passwords","com.agilebits.onepassword7","com.1password.1password"]) {self.enabled=enabled;self.limit=limit;self.maxTextBytes=maxTextBytes;self.retentionDays=retentionDays;self.excludedApps=excludedApps;self.maxAssetBytes=maxAssetBytes;self.maxTotalAssetBytes=maxTotalAssetBytes}
}
public struct Configuration:Codable,Sendable,Equatable {
    public var version:Int;public var launcherHotkey:String;public var clipboardHotkey:String;public var clipboard:ClipboardSettings;public var commands:[Command];public var snippetExpansionEnabled:Bool?;public var hyperKeyEnabled:Bool?;public var separateCommandKeys:Bool?;public var theme:String?;public var menuBarVisible:Bool?
    public static let layouts=["left-half","right-half","first-third","center-third","last-third","first-two-thirds","last-two-thirds","maximize","restore","next-screen","previous-screen"]
    public var separatesCommandKeys:Bool {separateCommandKeys ?? true}
    public func resolve(_ name:String)->Command? {commands.first {$0.isEnabled && ($0.id.lowercased()==name.lowercased() || $0.aliases.contains {$0.lowercased()==name.lowercased()})}}
}
public struct Clip:Codable,Sendable,Identifiable,Equatable {
    public var id:String;public var text:String;public var date:Date;public var source:String;public var contentType:String?;public var dataFile:String?;public var fileURLs:[String]?
    public init(text:String,source:String,date:Date=Date(),contentType:String?=nil,dataFile:String?=nil,fileURLs:[String]?=nil) {id=UUID().uuidString;self.text=text;self.source=source;self.contentType=contentType;self.dataFile=dataFile;self.fileURLs=fileURLs;self.date=Date(timeIntervalSince1970:date.timeIntervalSince1970.rounded(.down))}
}
public struct Frame:Codable,Sendable,Equatable {
    public var x:Double;public var y:Double;public var width:Double;public var height:Double
    public init(x:Double,y:Double,width:Double,height:Double) {self.x=x;self.y=y;self.width=width;self.height=height}
}
