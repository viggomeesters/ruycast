import Foundation
import LauncherCore
import LauncherModels

func checkScriptChoices() throws {
    let items=[ScriptChoice(id:"project",title:"Café project",subtitle:"EX · Projectbreed",value:"pinned-project"),ScriptChoice(id:"part",title:"Planning",subtitle:"EX / PLAN · Café project",value:"pinned-component")]
    let choices=ScriptChoices(title:"Choose",placeholder:"Search",items:items);try choices.validate()
    precondition(choices.filtered("cafe PLAN").map(\.id)==["part"])
    precondition(choices.filtered("unmatched").isEmpty)
    precondition(choices.filtered("")==items)
    do {try ScriptChoices(title:"Choose",placeholder:"Search",items:[items[0],items[0]]).validate();preconditionFailure("Duplicate choice accepted")} catch {}
    try ScriptChoices(title:"Choose",placeholder:"Search",items:[]).validate()
    print("Script choice search and validation checks passed")
}
