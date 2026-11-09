// Exact production SnippetChoiceModel; effects are only own counters. Targets
// containing clipboard demand are deliberately excluded by the fixture boundary.
import AppKit
import LauncherModels
import LauncherCore
@main struct ChoiceReference {
    @MainActor static func main() throws {
        let cases=try JSONSerialization.jsonObject(with:FileHandle.standardInput.readDataToEndOfFile()) as! [[[String:Any]]]
        var output:[[[String:Any]]]=[]
        for ops in cases {
            var model:SnippetChoiceModel?,submitted=0,cancelled=0,steps:[[String:Any]]=[]
            for op in ops {
                if op["op"] as? String == "equal" {
                    let left=try JSONDecoder().decode(Command.self,from:JSONSerialization.data(withJSONObject:op["left"]!))
                    let right=try JSONDecoder().decode(Command.self,from:JSONSerialization.data(withJSONObject:op["right"]!))
                    steps.append(["equal":left==right]);continue
                }
                var handled:Any=NSNull()
                switch op["op"] as! String {
                case "start":
                    let command=try JSONDecoder().decode(Command.self,from:JSONSerialization.data(withJSONObject:op["command"]!))
                    precondition(!command.target.contains("{clipboard}"))
                    submitted=0;cancelled=0
                    model=SnippetChoiceModel(command:command,submit:{_ in submitted+=1},cancel:{cancelled+=1})
                case "set":let i=op["field"] as? Int ?? 0;if model!.fields.indices.contains(i) {let id=model!.fields[i].id;model!.activeField=id;model!.values[id]=op["value"] as? String ?? ""}
                case "focus":let i=op["field"] as? Int ?? 0;if model!.fields.indices.contains(i) {model!.activeField=model!.fields[i].id}
                case "key":
                    let event=NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:NSEvent.ModifierFlags(rawValue:op["flags"] as? UInt ?? 0),timestamp:0,windowNumber:0,context:nil,characters:"",charactersIgnoringModifiers:"",isARepeat:op["repeat"] as? Bool ?? false,keyCode:op["code"] as? UInt16 ?? 36)!
                    handled=model!.handle(event,editingText:op["editingText"] as? Bool ?? false,composing:op["composing"] as? Bool ?? false)
                case "submit":model!.submit()
                case "cancel":model!.cancel()
                case "read":break
                default:fatalError("Unknown own choice action")
                }
                steps.append(["fields":model!.fields.map {["id":$0.id,"label":$0.label,"defaultValue":$0.defaultValue,"options":$0.options] as [String:Any]},"values":model!.values,"active":model!.activeField as Any? ?? NSNull(),"canSubmit":model!.canSubmit,"finished":submitted+cancelled>0,"submitted":submitted,"cancelled":cancelled,"handled":handled])
            };output.append(steps)
        }
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject:["ok":true,"cases":output],options:[.sortedKeys]))
    }
}
