// Test-only wrapper. The Python producer compiles literal current production
// KeyboardState, CommandShortcutFilter and Hyper pure function bodies.
import Foundation
import CoreGraphics
import LauncherCore

@main struct HIDStateReference {
    static func u(_ op:[String:Any],_ key:String)->UInt64 {(op[key] as! NSNumber).uint64Value}
    static func time(_ value:Any)->Double {
        if let text=value as? String {return ["nan":Double.nan,"inf":Double.infinity,"-inf":-Double.infinity][text]!}
        return (value as! NSNumber).doubleValue
    }
    @MainActor static func main() throws {
        let data=FileHandle.standardInput.readDataToEndOfFile()
        guard data.count<=16_777_216 else {throw NSError(domain:"Own state fixture too large",code:1)}
        let cases=try JSONSerialization.jsonObject(with:data) as! [[[String:Any]]]
        guard cases.count<=4096,cases.allSatisfy({$0.count<=16384}) else {throw NSError(domain:"Too many fixture events",code:1)}
        var result:[[[String:Any]]]=[]
        for input in cases {
            var state=KeyboardState(),filter=CommandShortcutFilter()
            var steps:[[String:Any]]=[]
            for op in input {
                var tap:UInt32?,action:String?,consumed=false,flags:UInt64?
                var readiness:String?
                switch op["op"] as! String {
                case "input":tap=state.input(device:u(op,"device"),usage:UInt32(u(op,"usage")),down:op["down"] as! Bool,time:time(op["time"]!))
                case "remove":state.remove(device:u(op,"device"))
                case "reset":state.reset();filter.reset()
                case "filter-reset":filter.reset()
                case "cancel":state.cancelTap()
                case "bindings":filter.bindings=try (op["bindings"] as! [[String:String]]).map {CommandShortcutFilter.Binding(shortcut:try Shortcut($0["shortcut"]!),action:$0["action"]!)}
                case "event":
                    let type=CGEventType(rawValue:UInt32(u(op,"kind"))) ?? .null
                    let decision=filter.handle(type,key:UInt16(u(op,"key")),flags:CGEventFlags(rawValue:u(op,"flags")),repeatKey:op["repeat"] as? Bool ?? false)
                    action=decision.action;consumed=decision.consumed
                case "rewrite":flags=FlagReference.rewrittenFlags(CGEventFlags(rawValue:u(op,"flags")),held:op["held"] as! Bool,tag:(op["tag"] as! NSNumber).int64Value).rawValue
                case "readiness":
                    ReadinessPredicates.facts=op
                    let reference=ReadinessReference(hyperEnabled:op["hyper"] as! Bool,bindings:op["modifiers"] as! Bool ? [224:"test"]:[:],commandBindings:op["sided"] as! Bool ? ["right-cmd+k":"test"]:[:],sleeping:op["sleeping"] as! Bool)
                    let reason=reference.reason();readiness=reason.isEmpty ? nil:reason
                default:throw NSError(domain:"Unknown own event",code:1)
                }
                let held:[Any]=state.held.keys.sorted().map {[$0,state.held[$0]!.sorted()] as [Any]}
                steps.append(["tap":tap.map{NSNumber(value:$0)} ?? NSNull(),"action":action as Any? ?? NSNull(),"consumed":consumed,"flags":flags.map{NSNumber(value:$0)} ?? NSNull(),"readiness":readiness as Any? ?? NSNull(),"caps":state.capsHeld,"modifiers":PhysicalModifierReference.flags(state).rawValue,"usages":state.usages.sorted(),"held":held])
            }
            result.append(steps)
        }
        let out=try JSONSerialization.data(withJSONObject:["ok":true,"cases":result],options:[.sortedKeys])
        FileHandle.standardOutput.write(out)
    }
}
