import Foundation
import Darwin
import LauncherCore

func checkPrivateFiles() throws {
    let root=FileManager.default.temporaryDirectory.appendingPathComponent("ruycast-private-"+UUID().uuidString)
    try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o755])
    defer {try? FileManager.default.removeItem(at:root)}
    let target=root.appendingPathComponent("private.json"),other=root.appendingPathComponent("other.json")
    try Files.save(["old":"preserved"],to:other)
    let original=try Data(contentsOf:other)
    try FileManager.default.createSymbolicLink(at:target,withDestinationURL:other)
    try Files.save(["new":"replacement"],to:target)
    let retained=try Data(contentsOf:other);precondition(retained==original,"Saving must preserve a symlink target")
    let values=try target.resourceValues(forKeys:[.isSymbolicLinkKey]);precondition(values.isSymbolicLink==false)
    let permissions=try FileManager.default.attributesOfItem(atPath:target.path)[.posixPermissions] as? NSNumber;precondition(permissions?.intValue==0o600)
    let blocked=root.appendingPathComponent("directory.json")
    try FileManager.default.createDirectory(at:blocked,withIntermediateDirectories:false)
    let child=blocked.appendingPathComponent("keep");try Data("existing".utf8).write(to:child)
    do {try Files.save(["unexpected":"replacement"],to:blocked);preconditionFailure("A directory must not be replaced")} catch {}
    let kept=try Data(contentsOf:child);precondition(kept==Data("existing".utf8))
    let exclusive=root.appendingPathComponent("exclusive.json")
    let first=Data("first synthetic payload".utf8)
    try Files.createPrivate(first,to:exclusive)
    do {try Files.createPrivate(Data("must not replace".utf8),to:exclusive);preconditionFailure("Create-only must preserve existing data")} catch {}
    let exclusiveBytes=try Data(contentsOf:exclusive);precondition(exclusiveBytes==first)
    let exclusiveLink=root.appendingPathComponent("exclusive-link.json")
    try FileManager.default.createSymbolicLink(at:exclusiveLink,withDestinationURL:exclusive)
    do {try Files.createPrivate(Data("must not follow".utf8),to:exclusiveLink);preconditionFailure("Create-only must preserve symlinks")} catch {}
    let linkValues=try exclusiveLink.resourceValues(forKeys:[.isSymbolicLinkKey]);precondition(linkValues.isSymbolicLink==true)
    let originalExclusive=try Data(contentsOf:exclusive);precondition(originalExclusive==first)
    let names=try FileManager.default.contentsOfDirectory(atPath:root.path)
    precondition(Set(names)==["private.json","other.json","directory.json","exclusive.json","exclusive-link.json"],"Failed writes must remove their own temporary file")
}

// Executed only by the Python observer in a disposable directory. This fixture
// calls the production writer; it does not duplicate its implementation.
func runPrivateStorageWriteProbe(root:URL) throws {
    let body=String(repeating:"synthetic-private-payload-",count:160000)
    for iteration in 0..<20 {try Files.save(["iteration":String(iteration),"body":body],to:root.appendingPathComponent("private.json"))}
}
