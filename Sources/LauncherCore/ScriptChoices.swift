import Foundation
import LauncherModels

public struct ScriptChoice: Codable, Identifiable, Equatable {
    public let id: String
    public let title: String
    public let subtitle: String
    public let value: String
    public init(id:String,title:String,subtitle:String,value:String) {self.id=id;self.title=title;self.subtitle=subtitle;self.value=value}
}
public struct ScriptChoices: Codable, Equatable {
    public let title: String
    public let placeholder: String
    public let items: [ScriptChoice]
    public init(title:String,placeholder:String,items:[ScriptChoice]) {self.title=title;self.placeholder=placeholder;self.items=items}
    public func validate() throws {
        guard items.count<=2000,Set(items.map(\.id)).count==items.count,
              items.allSatisfy({!$0.id.isEmpty && !$0.title.isEmpty && $0.value.utf8.count<=65536}) else {throw LauncherError("The command returned invalid choices. Reload and try again.")}
    }
    public func filtered(_ query:String)->[ScriptChoice] {
        let terms=query.folding(options:[.caseInsensitive,.diacriticInsensitive],locale:.current).split(whereSeparator:{$0.isWhitespace})
        return items.filter {item in let haystack=(item.title+" "+item.subtitle).folding(options:[.caseInsensitive,.diacriticInsensitive],locale:.current);return terms.allSatisfy {haystack.contains($0)}}
    }
}
