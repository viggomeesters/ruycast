import AppKit
import ApplicationServices

struct WindowSearchApplication:Sendable {
    let pid:pid_t
    let name:String
    static func running()->[WindowSearchApplication] {
        NSWorkspace.shared.runningApplications.filter {$0.processIdentifier != getpid() && $0.activationPolicy == .regular}.map {WindowSearchApplication(pid:$0.processIdentifier,name:$0.localizedName ?? "App")}
    }
}
struct WindowActionInfo:Codable,Equatable,Sendable {
    let application:String
    let title:String
    // AXMinimized is optional for windows that cannot be minimized.
    let minimized:Bool?
}
