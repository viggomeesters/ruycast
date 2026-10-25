import Foundation

/// An ephemeral suffix buffer; never persisted or logged.
public struct SnippetMatcher {
    private var buffer=""
    public init() {}
    public mutating func reset() {buffer=""}
    public mutating func backspace() {if !buffer.isEmpty {buffer.removeLast()}}
    public mutating func type(_ text:String,commands:[Command])->Command? {
        guard !text.isEmpty else {reset();return nil}
        buffer=String((buffer+text).suffix(128))
        let found=commands.filter {command in
            guard command.isEnabled,command.kind=="snippet",let keyword=command.keyword,buffer.hasSuffix(keyword) else {return false}
            let prefix=buffer.dropLast(keyword.count)
            if let first=keyword.first,first.isLetter || first.isNumber,let previous=prefix.last,previous.isLetter || previous.isNumber {return false}
            return Templates.fields(command).allSatisfy {!$0.defaultValue.isEmpty}
        }.max {($0.keyword?.count ?? 0)<($1.keyword?.count ?? 0)}
        if found != nil {reset()}
        return found
    }
}

/// A complete edit plan, applied synchronously before consuming the trigger key.
public struct SnippetReplacement {
    public let eraseCount:Int
    public let text:String
    public let cursor:Int?
    public init(keyword:String,finalInput:String,text:String,cursor:Int?) {
        eraseCount=max(0,keyword.count-finalInput.count);self.text=text;self.cursor=cursor
    }
    public func apply(key:(UInt16)->Void,insert:(String)->Void) {
        for _ in 0..<eraseCount {key(51)}
        var chunk=""
        for scalar in text.unicodeScalars {
            let value=String(scalar)
            if chunk.utf16.count+value.utf16.count>16 {insert(chunk);chunk=""}
            chunk+=value
        }
        if !chunk.isEmpty {insert(chunk)}
        if let cursor {
            let steps=(text as NSString).substring(from:min(cursor,text.utf16.count)).count
            for _ in 0..<steps {key(123)}
        }
    }
}

/// Missing accessibility metadata is common in web-backed editors; it is not
/// evidence of secure entry. The OS secure-input flag always takes precedence.
public enum SnippetFocusPolicy {
    public static func allowsExpansion(secureInput:Bool,subrole:String?)->Bool {
        !secureInput && subrole != "AXSecureTextField"
    }
}
