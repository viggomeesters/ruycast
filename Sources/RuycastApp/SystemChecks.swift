import AppKit
import ApplicationServices
import LauncherCore

/// Uses fake process identities and injected effects; never changes the desktop.
func checkSystemServices() throws -> [String:Any] {
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    let identity=SystemServices.Identity(pid:123456,uid:getuid(),seconds:11,micros:22)
    var current: SystemServices.Identity?=identity
    var killed=[pid_t](),pressed=0,raised=0,activated=0,monitored=0,locked=0,trusted=true
    var effects=SystemServices.Effects()
    effects.identity={_ in current};effects.trusted={trusted}
    effects.terminate={killed.append($0);return 0}
    effects.activate={_ in activated+=1;return true}
    effects.press={_ in pressed+=1;return .success}
    effects.raise={_ in raised+=1;return .success}
    effects.monitor={monitored+=1;return true}
    effects.lock={locked+=1}
    let service=SystemServices(effects:effects)
    let kill=service.remember(kind:"kill",identity:identity,element:nil,key:"fixture")
    try require(!service.requiresConfirmation(target:kill),"Kill still requires confirmation.")
    try require(service.execute(target:kill)["ok"] as? Bool==true && killed==[identity.pid],"Direct Kill did not terminate the verified fixture.")
    killed=[]
    current=SystemServices.Identity(pid:identity.pid,uid:identity.uid,seconds:12,micros:22)
    try require(service.execute(target:kill,confirmed:true)["ok"] as? Bool==false && killed.isEmpty,"A reused process ID was terminated.")
    current=identity
    try require(service.execute(target:kill,confirmed:true)["ok"] as? Bool==true && killed==[identity.pid],"Confirmed force quit did not use the verified process.")
    let own=SystemServices.Identity(pid:getpid(),uid:getuid(),seconds:1,micros:1)
    current=own;let ownTarget=service.remember(kind:"kill",identity:own,element:nil,key:"self")
    try require(service.execute(target:ownTarget,confirmed:true)["ok"] as? Bool==false && killed.count==1,"The launcher could terminate itself.")
    let foreign=SystemServices.Identity(pid:123457,uid:getuid()+1,seconds:1,micros:1)
    current=foreign;let foreignTarget=service.remember(kind:"kill",identity:foreign,element:nil,key:"foreign")
    try require(service.execute(target:foreignTarget,confirmed:true)["ok"] as? Bool==false && killed.count==1,"Another user's process could be terminated.")
    current=identity
    let element=AXUIElementCreateApplication(identity.pid)
    let menu=service.remember(kind:"menu",identity:identity,element:element,key:"File/Open")
    try require(menu==service.remember(kind:"menu",identity:identity,element:element,key:"File/Open"),"Menu identity was unstable.")
    try require(service.execute(target:menu,previousPID:42)["ok"] as? Bool==false && pressed==0 && activated==0,"Menu execution lost its original app.")
    trusted=false
    try require(service.execute(target:menu,previousPID:identity.pid)["ok"] as? Bool==false && pressed==0,"Menu permission was bypassed.")
    trusted=true
    try require(service.execute(target:menu,previousPID:identity.pid)["ok"] as? Bool==true && pressed==1 && activated==1,"Menu action was not routed to its original app.")
    let window=service.remember(kind:"window",identity:identity,element:element,key:"window")
    try require(raised==0,"Window enumeration raised a window.")
    try require(service.execute(target:window)["ok"] as? Bool==true && raised==1 && activated==2,"Window action did not raise and activate.")
    try require(service.execute(target:kill.replacingOccurrences(of:"system.kill:",with:"system.menu:"),confirmed:true)["ok"] as? Bool==false,"A token could be used for another action.")
    try require(service.execute(target:"system.window:missing")["ok"] as? Bool==false,"An expired token was accepted.")
    try require(monitored==0 && locked==0,"A system action ran during search.")
    try require(service.execute(target:"system-monitor")["ok"] as? Bool==true && monitored==1,"Monitor action did not execute.")
    try require(service.execute(target:"lock-screen")["ok"] as? Bool==true && locked==1,"Lock action did not execute.")
    let predicate=SystemServices.filePredicate("a\" || true || \\\u{0022}*?")
    try require(predicate=="kMDItemFSName == \"*a\\\" || true || \\\\\\\"\\*\\?*\"cd","File search did not escape the Spotlight literal.")
    var delivered=false,stale=false,onMain=false
    service.results(mode:"invalid",query:"",previousPID:nil) {_ in stale=true}
    service.cancel()
    service.results(mode:"files",query:"",previousPID:nil) {result in
        delivered=true;onMain=Thread.isMainThread
        if case .success(let rows)=result {onMain = onMain && rows.isEmpty} else {onMain=false}
    }
    let deadline=Date().addingTimeInterval(2)
    while !delivered,Date()<deadline {_=RunLoop.current.run(mode:.default,before:Date().addingTimeInterval(0.01))}
    try require(delivered && onMain && !stale,"Stale results or background callbacks reached the consumer.")
    return ["ok":true,"directKillWithoutConfirmation":true,"pidReuseRejected":true,"selfAndForeignProcessesProtected":true,"originalMenuAppPreserved":true,"accessibilityRequired":true,"opaqueTargetsValidated":true,"explicitWindowAction":true,"spotlightLiteralEscaped":true,"staleCallbacksCancelled":true,"callbacksOnMainThread":true,"realDesktopEffects":false]
}
