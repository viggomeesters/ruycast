import Foundation
import LauncherModels

/// Snippet sources stay editable Markdown; only explicit rich modes interpret links.
public enum SnippetOutput {
    public static let modes=["plain","teams","html"]
    public struct Link:Sendable {
        public let range:NSRange
        public let labelRange:NSRange
        public let label:String
        public let url:String
    }
    public struct Payload:Sendable {
        public let text:String
        public let html:String?
        public let cursor:Int?
    }
    public static func links(_ source:String)->[Link] {
        guard let regex=try? NSRegularExpression(pattern:#"\[((?:\\.|[^\]\r\n\\])*)\]\(((?:[^\s()]+|\([^\s()]*\))*)\)"#) else {return []}
        let ns=source as NSString
        return regex.matches(in:source,range:NSRange(location:0,length:ns.length)).compactMap {m in
            let url=ns.substring(with:m.range(at:2))
            guard validURL(url) else {return nil}
            return Link(range:m.range,labelRange:m.range(at:1),label:decodeLabel(ns.substring(with:m.range(at:1))),url:url)
        }
    }
    public static func validURL(_ value:String)->Bool {
        guard !value.contains(where:{$0.isWhitespace || $0.isNewline}),let u=URL(string:value),["https","http","mailto"].contains(u.scheme?.lowercased() ?? "") else {return false}
        return u.scheme?.lowercased()=="mailto" ? !String(value.dropFirst(7)).isEmpty:!(u.host ?? "").isEmpty
    }
    private static func decodeLabel(_ value:String)->String {
        value.replacingOccurrences(of:#"\\([\[\]\\])"#,with:"$1",options:.regularExpression)
    }
    public static func linkSource(label:String,url:String)->String {
        let escaped=label.replacingOccurrences(of:"\\",with:"\\\\").replacingOccurrences(of:"[",with:#"\["#).replacingOccurrences(of:"]",with:#"\]"#)
        let destination=url.replacingOccurrences(of:"(",with:"%28").replacingOccurrences(of:")",with:"%29")
        return "["+escaped+"]("+destination+")"
    }
    private static func escape(_ value:String)->String {value.replacingOccurrences(of:"&",with:"&amp;").replacingOccurrences(of:"<",with:"&lt;").replacingOccurrences(of:">",with:"&gt;").replacingOccurrences(of:"\"",with:"&quot;")}
    public static func render(_ source:String,mode:String?,cursor:Int?=nil)->Payload {
        guard mode=="teams" || mode=="html" else {return Payload(text:source,html:nil,cursor:cursor)}
        let ns=source as NSString,found=links(source)
        var plain="",rich="",position=0,mappedCursor:Int?
        func append(_ value:String) {plain+=value;rich+=escape(value)}
        for link in found {
            let before=ns.substring(with:NSRange(location:position,length:link.range.location-position))
            if let cursor,mappedCursor==nil,cursor<=link.range.location {mappedCursor=plain.utf16.count+max(0,cursor-position)}
            append(before)
            if let cursor,mappedCursor==nil,cursor<NSMaxRange(link.range) {let rawLabel=ns.substring(with:link.labelRange) as NSString
                let rawCount=max(0,min(rawLabel.length,cursor-link.labelRange.location))
                mappedCursor=plain.utf16.count+decodeLabel(rawLabel.substring(to:rawCount)).utf16.count}
            plain+=link.label;rich+="<a href=\""+escape(link.url)+"\">"+escape(link.label)+"</a>"
            position=NSMaxRange(link.range)
        }
        if let cursor,mappedCursor==nil {mappedCursor=plain.utf16.count+max(0,min(cursor,ns.length)-position)}
        append(ns.substring(from:position))
        // Explicit rows preserve blank lines in editors that discard CSS margins.
        let lines=rich.components(separatedBy:"\n")
        let body:String
        if mode=="teams" {
            body="<table role=\"presentation\" border=\"0\" cellspacing=\"0\" cellpadding=\"0\"><tbody>"+lines.map {"<tr><td>"+($0.isEmpty ? "&nbsp;":$0)+"</td></tr>"}.joined()+"</tbody></table>"
        } else {body="<div>"+lines.joined(separator:"<br>")+"</div>"}
        return Payload(text:plain,html:"<html><head><meta charset=\"utf-8\"></head><body>"+body+"</body></html>",cursor:mappedCursor)
    }
}
