// Links exact current production models/templates/matcher. No native key effects.
import Foundation
import LauncherModels
import LauncherCore
private func effects(_ r:SnippetReplacement)->[String:Any] {
    var operations:[[String:Any]]=[]
    r.apply(key:{operations.append(["key":Int($0)])},insert:{operations.append(["insert":$0])})
    return ["eraseCount":r.eraseCount,"text":r.text,"cursor":r.cursor.map{$0 as Any} ?? NSNull(),"operations":operations]
}
@main struct KeywordReference {
    static func main() throws {
        let data=FileHandle.standardInput.readDataToEndOfFile()
        guard data.count<=16_777_216 else {throw LauncherError("Own fixture too large")}
        let cases=try JSONSerialization.jsonObject(with:data) as! [[[String:Any]]]
        var output:[[[String:Any]]]=[]
        for fixture in cases {
            var matcher=SnippetMatcher(),commands:[Command]=[],choice:SnippetChoiceRequest?
            var results:[[String:Any]]=[]
            for op in fixture {
                let result:[String:Any]
                switch op["op"] as! String {
                case "catalog":commands=try JSONDecoder().decode([Command].self,from:JSONSerialization.data(withJSONObject:op["commands"]!));result=[:]
                case "type":result=["match":matcher.type(op["text"] as! String,commands:commands).map{$0.id as Any} ?? NSNull()]
                case "backspace":matcher.backspace();result=[:]
                case "reset":matcher.reset();result=[:]
                case "replace":result=effects(SnippetReplacement(keyword:op["keyword"] as! String,finalInput:op["finalInput"] as! String,text:op["text"] as! String,cursor:op["cursor"] as? Int))
                case "focus":result=["allowed":SnippetFocusPolicy.allowsExpansion(secureInput:op["secure"] as? Bool ?? false,subrole:op["subrole"] as? String)]
                case "classify-all":
                    var bits=[UInt8](repeating:0,count:(0x110000+7)/8),count=0
                    for value in UInt32(0)..<0x110000 {
                        if let scalar=Unicode.Scalar(value){let c=Character(String(scalar));if c.isLetter || c.isNumber {bits[Int(value)/8] |= 1 << UInt8(value%8);count+=1}}
                    }
                    result=["bits":bits.map{String(format:"%02x",$0)}.joined(),"count":count]
                case "choice-start":choice=SnippetChoiceRequest(command:try JSONDecoder().decode(Command.self,from:JSONSerialization.data(withJSONObject:op["command"]!)),pid:op["pid"] as! Int32);result=["active":choice!.active]
                case "choice-cancel":choice!.cancel();result=["active":choice!.active]
                case "choice":
                    var values:[String:String]=[:];for pair in op["values"] as? [[String]] ?? [] {values[pair[0]]=pair[1]}
                    do {let r=try choice!.replacement(values:values,pid:op["pid"] as! Int32,safe:op["safe"] as? Bool ?? false,unchanged:op["unchanged"] as? Bool ?? false,clipboard:op["clipboard"] as? String);result=["ok":true,"active":choice!.active,"replacement":effects(r)]}
                    catch {result=["ok":false,"active":choice!.active,"error":error.localizedDescription]}
                default:throw LauncherError("Unknown own keyword operation")
                }
                results.append(result)
            };output.append(results)
        }
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject:["ok":true,"cases":output],options:[.sortedKeys]))
    }
}
