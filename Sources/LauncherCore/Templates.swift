import Foundation

public struct TemplateField:Identifiable,Equatable {
    public let id:String
    public let label:String
    public let defaultValue:String
    public let options:[String]
}
public enum Templates {
    private static let pattern=try! NSRegularExpression(pattern:#"\{([^{}]+)\}"#)
    private static func attribute(_ name:String,in text:String)->String? {
        let r=try! NSRegularExpression(pattern:name + #"="([^"]*)""#)
        guard let m=r.firstMatch(in:text,range:NSRange(text.startIndex...,in:text)),let range=Range(m.range(at:1),in:text) else {return nil}
        return String(text[range])
    }
    public static func needsInteraction(_ command:Command)->Bool {
        fields(command).contains {!$0.options.isEmpty || $0.defaultValue.isEmpty}
    }
    public static func fields(_ command:Command)->[TemplateField] {
        var fields:[TemplateField]=[]
        if let prompt=command.inputPrompt {fields.append(TemplateField(id:"input",label:prompt,defaultValue:"",options:[]))}
        guard command.kind=="url" || command.kind=="snippet" else {return fields}
        for match in pattern.matches(in:command.target,range:NSRange(command.target.startIndex...,in:command.target)) {
            guard let range=Range(match.range(at:1),in:command.target) else {continue}
            let token=String(command.target[range])
            if ["clipboard","cursor","date","time","datetime","day"].contains(token) || token.hasPrefix("date ") || token.hasPrefix("datetime:") {continue}
            let name=attribute("name",in:token) ?? (token=="query" || token=="argument" ? "Query":token)
            if !fields.contains(where:{$0.id==name}) {fields.append(TemplateField(id:name,label:name,defaultValue:attribute("default",in:token) ?? "",options:(attribute("options",in:token) ?? "").split(separator:",").map {$0.trimmingCharacters(in:.whitespacesAndNewlines)}.filter {!$0.isEmpty}.reduce(into:[String]()) {if !$0.contains($1) {$0.append($1)}}))}
        }
        return fields
    }
    public static func render(_ command:Command,values:[String:String],clipboard:String?=nil,now:Date=Date(),locale:Locale = .current,timeZone:TimeZone = .current) throws -> (String,Int?) {
        var text=command.target
        var cursor:Int?
        for match in pattern.matches(in:text,range:NSRange(text.startIndex...,in:text)).reversed() {
            guard let tokenRange=Range(match.range(at:1),in:text),let fullRange=Range(match.range,in:text) else {continue}
            let token=String(text[tokenRange]);let replacement:String
            switch token {
            case "cursor":replacement=""
            case "clipboard":guard let clipboard else {throw LauncherError("Clipboard text is unavailable.")};replacement=clipboard
            case "date","time","datetime","day":
                let f=DateFormatter();f.locale=locale;f.timeZone=timeZone;f.dateFormat=["date":"yyyy-MM-dd","time":"HH:mm","datetime":"d MMMM yyyy HH:mm","day":"EEEE"][token]!;replacement=f.string(from:now)
            default:
                if token.hasPrefix("date ") || token.hasPrefix("datetime:") {
                    let f=DateFormatter();f.locale=locale;f.timeZone=timeZone;f.dateFormat=attribute("format",in:token) ?? String(token.dropFirst("datetime:".count));replacement=f.string(from:now)
                } else {
                    let name=attribute("name",in:token) ?? (token=="query" || token=="argument" ? "Query":token)
                    let value=values[name] ?? attribute("default",in:token) ?? ""
                    guard !value.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty else {throw LauncherError("Enter \(name).")}
                    if command.kind=="url" {replacement=value.addingPercentEncoding(withAllowedCharacters:.alphanumerics) ?? ""} else {replacement=value}
                }
            }
            // Work right-to-left; shift an already observed cursor when text before it changes.
            if let c=cursor {cursor=c+replacement.utf16.count-match.range.length}
            if token=="cursor" {cursor=match.range.location}
            text.replaceSubrange(fullRange,with:replacement)
        }
        return (text,cursor)
    }
}
