// Native Swift pasteboard bridge plus the production CommandEditor.swift trim
// expression. Only an explicit own named board and attached fault provider.
import AppKit
import Darwin

func symbol<T>(_ name:String,_ type:T.Type)->T {
    guard let p=dlsym(UnsafeMutableRawPointer(bitPattern:-2),name) else {fatalError("Complete owned clipboard provider required")}
    return unsafeBitCast(p,to:type)
}
let guardBoard=symbol("ruycast_owned_board_general_guard",(@convention(c)()->Bool).self)
let attach=symbol("ruycast_owned_board_attach",(@convention(c)(UnsafeRawPointer?,Int32)->Bool).self)
let detach=symbol("ruycast_owned_board_detach",(@convention(c)()->Void).self)
precondition(guardBoard() && CommandLine.arguments.count==3)
let reader=CommandLine.arguments[1],mode=Int32(CommandLine.arguments[2])!
precondition(["read","prefill"].contains(reader) && (0...17).contains(mode))
autoreleasepool {
    let board=NSPasteboard.withUniqueName()
    defer {detach();board.clearContents();board.releaseGlobally()}
    precondition(board.setString("OWN ORIGINAL",forType:.string))
    precondition(attach(Unmanaged.passUnretained(board).toOpaque(),mode))
    var value=board.string(forType:.string)
    if reader=="prefill" {
        value=value?.trimmingCharacters(in:.whitespacesAndNewlines)
        if value?.isEmpty==true {value=nil}
    }
    detach()
    // Observe actual Swift String UTF8 semantics. JSONSerialization otherwise
    // rebridges a lazy foreign String to the original malformed NSString and
    // rejects it, rather than observing its repaired Swift character stream.
    let observed=value.map {String(decoding:Array($0.utf8),as:UTF8.self)}
    let output:[String:Any]=["ok":true,"value":observed as Any? ?? NSNull(),"restored":board.string(forType:.string)=="OWN ORIGINAL","generalBoardAbortGuard":true]
    let data=try! JSONSerialization.data(withJSONObject:output,options:[.sortedKeys])
    FileHandle.standardOutput.write(data)
}
