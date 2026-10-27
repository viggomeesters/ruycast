import Foundation
import CoreGraphics
import LauncherCore
import LauncherModels

/// Pure deterministic state checks. No tap, HID manager, Carbon registration,
/// permission prompt, synthetic key post or application window is created.
func checkKeyboard() throws -> [String:Any] {
    var state=KeyboardState()
    var effects:[String]=[]
    func input(_ usage:UInt32,_ down:Bool,_ time:Double,device:UInt64=1) {
        let before=state.capsHeld
        if let action=state.input(device:device,usage:usage,down:down,time:time) {effects.append("tap:\(action)")}
        if before != state.capsHeld {effects.append(state.capsHeld ? "hyper:down":"hyper:up")}
    }
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    input(0x39,true,0);input(0x39,true,0.01);input(4,true,0.02);input(4,false,0.03);input(0x39,false,0.04)
    try require(effects==["hyper:down","hyper:up"] && state.usages.isEmpty,"Hyper did not balance physical down/up.")
    effects=[];input(0xe0,true,1);input(0xe0,false,1.1);input(0xe2,true,2);input(0xe2,false,2.1)
    try require(effects==["tap:224","tap:226"],"Isolated left modifier taps failed.")
    effects=[];input(0xe0,true,3);input(4,true,3.1);input(4,false,3.15);input(0xe0,false,3.2)
    input(0xe2,true,4);input(0xe1,true,4.1);input(0xe1,false,4.2);input(0xe2,false,4.3)
    input(0xe0,true,5);input(0xe0,false,6)
    input(0xe7,true,7);input(0xe0,true,7.1);input(0xe0,false,7.2);input(0xe7,false,7.3)
    try require(effects.isEmpty,"Chord, held modifier or right-Command generated a solo tap.")
    input(0xe2,true,8);state.cancelTap();input(0xe2,false,8.1)
    try require(effects.isEmpty,"Mouse or Quartz cancellation did not cancel a tap.")
    input(0x39,true,9);input(0x39,true,9.1,device:2);input(0x39,false,9.2)
    try require(state.capsHeld,"Releasing one keyboard released the other keyboard's Hyper.")
    state.remove(device:2)
    try require(!state.capsHeld,"Unplugging keyboard left Hyper held.")
    input(0x39,true,10);input(0xe0,true,10.1);state.reset()
    try require(!state.capsHeld && state.usages.isEmpty,"Sleep or provider teardown left held state.")
    input(0xe0,false,11)
    try require(state.usages.isEmpty,"Stray release created held state.")
    let rightCommand=CGEventFlags(rawValue:CGEventFlags.maskCommand.rawValue | 0x10)
    try require(HyperKey.rewrittenFlags(rightCommand,held:false)==rightCommand,"Ordinary right-Command flags changed.")
    let combined=HyperKey.rewrittenFlags(rightCommand.union(.maskAlphaShift),held:true)
    try require(combined.contains(HyperKey.hyper) && combined.rawValue & 0x10 != 0 && !combined.contains(.maskAlphaShift),"Hyper flag rewrite changed unowned device flags.")
    for tag in [HyperKey.generatedTag,SnippetExpander.generatedTag] {
        try require(HyperKey.rewrittenFlags([.maskCommand,.maskControl],held:true,tag:tag)==[.maskCommand,.maskControl],"Generated lock or paste gained Hyper modifiers.")
        try require(HyperKey.rewrittenFlags([],held:true,tag:tag).isEmpty,"Generated snippet insertion gained physical modifiers.")
    }
    return ["generatedEventsKeepOwnModifiers":true,"ok":true,"physicalCapsEdges":true,"duplicateReportsIgnored":true,"soloModifierRelease":true,"chordsAndLongHoldsCancelled":true,"rightCommandFlagsPreserved":true,"multipleKeyboards":true,"deviceRemovalAndSleepReset":true,"eventTapsInstalled":false,"desktopInputSent":false,"hardwareAcceptance":"required"]
}
