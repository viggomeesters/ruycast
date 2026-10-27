import AppKit
import SwiftUI
import LauncherCore
import LauncherModels
struct ProcessIcon:View {
    let resource:ProcessResource
    var body:some View {
        let parts=NSString(string:resource.path).pathComponents
        let path=parts.firstIndex(where:{$0.hasSuffix(".app")}).map {NSString.path(withComponents:Array(parts.prefix($0+1)))} ?? resource.path
        let command=Command(id:"process-icon",title:resource.name,kind:"path",target:path)
        Group {if let icon=CommandIcons.shared.image(for:command) {Image(nsImage:icon).resizable().interpolation(.high)} else {Image(systemName:"terminal").foregroundStyle(.secondary)}}.frame(width:22,height:22)
            .help("PID \(resource.id)")
    }
}
