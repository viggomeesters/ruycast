import Foundation
import LauncherCore
import LauncherModels

func checkUtilities() throws -> [String:Any] {
    func require(_ condition:Bool,_ message:String) throws {if !condition {throw LauncherError(message)}}
    let sandbox=FileManager.default.temporaryDirectory.appendingPathComponent("ruycast-utilities-"+UUID().uuidString)
    defer {try? FileManager.default.removeItem(at:sandbox)}
    let old=Utilities.Entry(value:"fixture-old-password",created:"2000-01-01T00:00:00Z")
    var keychain:Data?=try JSONEncoder().encode([old]),writes=0
    var storage=Utilities.Storage()
    storage.readPasswords={keychain};storage.writePasswords={keychain=$0;writes+=1}
    storage.random={count in Array((0..<count).map {UInt8($0)})}
    storage.now={Date(timeIntervalSince1970:1_700_000_000)}
    let utilities=Utilities(root:sandbox,storage:storage)
    do {_=try utilities.execute(target:"utility-paste-text");throw LauncherError("Missing text was accepted.")} catch let error as LauncherError {try require(error.message.contains("Set Text"),"Missing text was not explained.")}
    _=try utilities.execute(target:"utility-set-text",input:"first\nline")
    _=try utilities.execute(target:"utility-set-text",input:"second")
    _=try utilities.execute(target:"utility-set-text",input:"first\nline")
    let text=try utilities.commands(mode:"text-history",query:"")
    try require(text.count==2 && text[0].title=="first line","Saved text history was not deduplicated.")
    _=try utilities.execute(target:text[1].target)
    if case .paste(let value)=try utilities.execute(target:"utility-paste-text") {try require(value=="second","Selecting active text failed.")} else {throw LauncherError("Saved text did not produce a paste effect.")}
    let reopened=Utilities(root:sandbox,storage:storage)
    if case .paste(let value)=try reopened.execute(target:"utility-paste-text") {try require(value=="second","Saved text did not survive reopening.")}
    let attributes=try FileManager.default.attributesOfItem(atPath:sandbox.appendingPathComponent("utilities.json").path)
    try require((attributes[.posixPermissions] as? NSNumber)?.intValue==0o600,"Utilities history permissions are not private.")
    if case .copy(let value,let secret)=try utilities.execute(target:"utility-password") {try require(value=="AAECAwQFBgcICQoLDA0ODxAREhMUFRYX" && secret,"Password generation parity or secret marking failed.")} else {throw LauncherError("Password did not produce a copy effect.")}
    let passwords=try JSONDecoder().decode([Utilities.Entry].self,from:keychain!)
    try require(passwords.count==2 && passwords[1]==old && writes==1,"Existing Keychain history was not preserved.")
    let commands=try utilities.commands(mode:"password-history",query:"")
    let payload=String(decoding:try JSONEncoder().encode(commands),as:UTF8.self)
    try require(!payload.contains(old.value) && !payload.contains(passwords[0].value),"A password leaked into a command payload.")
    if case .copy(let value,let secret)=try utilities.execute(target:commands[1].target) {try require(value==old.value && secret,"Password history did not copy as concealed text.")}
    if case .copy(let value,let secret)=try utilities.execute(target:"utility-username") {try require(value=="amber-atlas-0203" && !secret,"Username generator parity failed.")}
    try require(try utilities.commands(mode:"username-history",query:"amber").count==1,"Username history search failed.")
    keychain=Data("malformed".utf8)
    do {_=try utilities.execute(target:"utility-password");throw LauncherError("Malformed history was overwritten.")} catch let error as LauncherError {try require(error.message.contains("preserved") && writes==1,"Malformed Keychain history was not preserved.")}
    let stamp="2026-10-25T00:00:00Z"
    let imported=try utilities.importProfile(["savedText":"older imported text","textHistory":[["value":"imported history","created":stamp]],"usernameHistory":[]])
    try require(imported["added"]==1,"Utilities import did not merge history.")
    try require(try utilities.importProfile(["textHistory":[["value":"imported history","created":stamp]],"usernameHistory":[]])["added"]==0,"Utilities import replay duplicated data.")
    if case .paste(let value)=try utilities.execute(target:"utility-paste-text") {try require(value != "older imported text","Import replaced active native text.")}
    let disk=try Data(contentsOf:sandbox.appendingPathComponent("utilities.json"))
    try require(!String(decoding:disk,as:UTF8.self).contains("password"),"Passwords reached the profile file.")
    return ["ok":true,"savedTextPersists":true,"textHistoryDeduplicated":true,"activeTextRestored":true,"privateFileMode":true,"passwordGeneratorParity":true,"usernameGeneratorParity":true,"existingKeychainHistoryPreserved":true,"malformedKeychainPreserved":true,"secretsAbsentFromCommandsAndProfile":true,"concealedCopyEffect":true,"realKeychainOrClipboardChanged":false]
}
