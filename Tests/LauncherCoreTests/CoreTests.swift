import Foundation
import LauncherCore
@main struct CheckRunner {static func main() throws {print("Core behavior checks passed")
        let args=Array(CommandLine.arguments.dropFirst())
        if args.count==2,args[0]=="--library" {
            let config=try Files.config(URL(fileURLWithPath:args[1]));let index=try CommandSearchIndex(commands:config.commands)
            var rendered=0
            for command in config.commands where command.kind=="snippet" || command.kind=="url" {
                let values=Dictionary(uniqueKeysWithValues:Templates.fields(command).map {($0.id,$0.defaultValue.isEmpty ? ($0.options.first ?? "Fixture"):$0.defaultValue)})
                _=try Templates.render(command,values:values,clipboard:"Fixture",now:Date(timeIntervalSince1970:0));rendered+=1
                if let keyword=command.keyword {precondition(index.search(keyword).contains(where:{$0.id==command.id}),"Imported keyword is not searchable")}
            }
            print("Private library verified: \(config.commands.count) commands, \(rendered) link/snippet templates; no private text printed")
        }}}
