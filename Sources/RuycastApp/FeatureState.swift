import Foundation
import Observation
import LauncherCore
import LauncherModels

@Observable final class LauncherSettingsState {
    var configDirectory:URL
    var config:Configuration
    var configError:String?
    var expansionStatus:String = "Off"
    init(configDirectory:URL,config:Configuration) {self.configDirectory=configDirectory;self.config=config}
}

@Observable final class PaletteSession {
    var query:String = ""
    var mode:String = "commands"
    var actionSelection:Int = 0
    var pendingRemoval:Command?
    var actionsOpen:Bool = false
    var inlineCommand:Command?
    var inlineInput:String = ""
    var inlineDatePreview:TaskDateInput?
    var form:CommandForm?
    var confirmationCommand:Command?
    var scriptChoices:ScriptChoiceSession?
    var scriptChoiceQuery:String = ""
    var scriptChoiceSelection:Int = 0
    var detail:String?
    var message:String = ""
    var busy:Bool = false
    var selection=0
}

@Observable final class ClipboardState {
    var clips:[Clip] = []
    var clipboardDataRevision:Int = 0
    var filteredClips:[Clip] = []
    var clipboardSearchBusy:Bool = false
    var paused:Bool = false
    var clipboardCleanupError:String?
    var selection=0
    var query=""
}

@Observable final class CatalogState {
    var applications:[Command] = []
    var results:[Command] = []
    var processOrder:[Command] = []
    var processResources:[String:ProcessResource] = [:]
    var processSort:ProcessSort = .cpu
    @ObservationIgnored private var index:CommandSearchIndex?
    @ObservationIgnored private var prepared:PreparedCommandSearch?
    @ObservationIgnored private var rankingRevision:UInt64?
    @ObservationIgnored private(set) var preparationCount=0

    func replace(commands:[Command]) {
        index=try? CommandSearchIndex(commands:commands)
        prepared=nil;rankingRevision=nil
    }
    func search(_ query:String,ranking:SearchRanking)->[Command] {
        if rankingRevision != ranking.revision {
            prepared=index?.prepared(recency:ranking.entries)
            preparationCount+=1;rankingRevision=ranking.revision
        }
        return prepared?.search(query) ?? []
    }
}
