// Test-only oracle compiled with the exact current production Templates.swift
// and LauncherModels sources. Never linked/shipped in the Rust host.
import Foundation
import LauncherModels

private struct Fixture:Decodable {
    let command:Command
    let values:[[String]]?
    let clipboard:String?
    let now:Double
    let locale:String?
    let timeZone:String?
}
@main struct TemplateReference {
    static func main() throws {
        let data=FileHandle.standardInput.readDataToEndOfFile()
        guard data.count<=1_048_576 else {throw LauncherError("Fixture input exceeds limit.")}
        let fixtures=try JSONDecoder().decode([Fixture].self,from:data)
        guard fixtures.count<=2048 else {throw LauncherError("Too many fixture cases.")}
        let output:[[String:Any]]=try fixtures.map {fixture in
            var values:[String:String]=[:]
            for pair in fixture.values ?? [] {
                guard pair.count==2 else {throw LauncherError("Invalid fixture value pair.")}
                values[pair[0]]=pair[1]
            }
            let fields=Templates.fields(fixture.command)
            let rendered:[String:Any]
            do {
                let result=try Templates.render(fixture.command,values:values,clipboard:fixture.clipboard,now:Date(timeIntervalSince1970:fixture.now),locale:fixture.locale.map {Locale(identifier:$0)} ?? .current,timeZone:fixture.timeZone.flatMap {TimeZone(identifier:$0)} ?? .current)
                rendered=["ok":true,"text":result.0,"cursor":result.1.map {$0 as Any} ?? NSNull()]
            } catch {rendered=["ok":false,"error":error.localizedDescription]}
            return ["ok":true,"fields":fields.map {["id":$0.id,"label":$0.label,"defaultValue":$0.defaultValue,"options":$0.options] as [String:Any]},"needsInteraction":Templates.needsInteraction(fixture.command),"needsClipboard":fixture.command.target.contains("{clipboard}"),"render":rendered]
        }
        let result=try JSONSerialization.data(withJSONObject:["ok":true,"cases":output,"pureTemplateFixture":true],options:[.sortedKeys])
        FileHandle.standardOutput.write(result)
    }
}
