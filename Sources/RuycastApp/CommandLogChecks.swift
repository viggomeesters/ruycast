import Foundation
import LauncherCore
import LauncherModels

@MainActor func checkCommandLog() throws -> [String:Any] {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("rc-logs-"+String(UUID().uuidString.prefix(8)))
    try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);defer {try? FileManager.default.removeItem(at:root)}
    func require(_ value:Bool,_ message:String) throws {if !value {throw LauncherError(message)}}
    let store=CommandLog(root:root)
    let cleaned=CommandLog.clean("token=private-token Bearer private-bearer https://example.com/?secret=yes\nprivate input",secrets:["private input"])
    try require(!cleaned.contains("private-token") && !cleaned.contains("private-bearer") && !cleaned.contains("secret=yes") && !cleaned.contains("private input"),"Command diagnostic redaction failed")
    try require(CommandLog.clean(String(repeating:"🙂",count:1000),limit:512).utf8.count<=512,"Diagnostic Unicode bound failed")
    for index in 0..<600 {store.record(CommandLogEntry(traceID:String(index),commandID:"fixture",source:"cli",phase:"command.finish",success:index%2==0,durationMS:1,code:"fixture",message:cleaned))}
    store.flush()
    let loaded=CommandLog(root:root)
    try require(loaded.storageError==nil && loaded.entries(commandID:nil,errorsOnly:false,limit:500).count==500,"Command log bounds or reload failed")
    try require(loaded.entries(commandID:"fixture",errorsOnly:true,limit:3).count==3 && loaded.entries(commandID:"other",errorsOnly:false,limit:500).isEmpty,"Command log filters failed")
    let mode=try FileManager.default.attributesOfItem(atPath:store.url.path)[.posixPermissions] as? Int
    try require(mode==0o600,"Command log is not private")
    try FileManager.default.setAttributes([.posixPermissions:0o644],ofItemAtPath:store.url.path)
    let repaired=CommandLog(root:root)
    let restricted=try FileManager.default.attributesOfItem(atPath:store.url.path)[.posixPermissions] as? Int
    try require(repaired.storageError==nil && restricted==0o600,"Existing command log permissions were not restricted before reading")
    loaded.clear();try require(CommandLog(root:root).entries(commandID:nil,errorsOnly:false,limit:500).isEmpty,"Cleared diagnostic log reappeared")
    let bad=root.appendingPathComponent("bad");try FileManager.default.createDirectory(at:bad,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    let path=bad.appendingPathComponent("command-log.json"),original=Data("broken diagnostic file".utf8);try original.write(to:path)
    let corrupt=CommandLog(root:bad);corrupt.record(CommandLogEntry(traceID:"x",commandID:"fixture",source:"cli",phase:"command.start"));corrupt.flush()
    try require(corrupt.storageError != nil && (try Data(contentsOf:path))==original,"Invalid command log was overwritten")
    let linked=root.appendingPathComponent("linked");try FileManager.default.createDirectory(at:linked,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    try FileManager.default.createSymbolicLink(at:linked.appendingPathComponent("command-log.json"),withDestinationURL:path)
    let refused=CommandLog(root:linked);refused.record(CommandLogEntry(traceID:"x",commandID:"fixture",source:"cli",phase:"command.start"));refused.flush()
    try require(refused.storageError != nil && (try Data(contentsOf:path))==original,"Command log followed a symlink")
    let insecure=root.appendingPathComponent("insecure");try FileManager.default.createDirectory(at:insecure,withIntermediateDirectories:true,attributes:[.posixPermissions:0o755])
    let unsafe=CommandLog(root:insecure);unsafe.record(CommandLogEntry(traceID:"x",commandID:"fixture",source:"cli",phase:"command.start"));unsafe.flush()
    try require(unsafe.storageError != nil && !FileManager.default.fileExists(atPath:insecure.appendingPathComponent("command-log.json").path),"Diagnostics wrote into a nonprivate profile directory")
    return ["ok":true,"bounded500":true,"ownerOnly":true,"insecureModeRepaired":true,"reloadFiltersAndClear":true,"tokensURLsAndInputRedacted":true,"unicodeByteBound":true,"corruptAndSymlinkPreserved":true,"desktopEffects":false]
}
