import AppKit
import LauncherCore
import LauncherModels

/// Native lookups stay out of repeated renders; short-lived entries recover after app/file changes.
@MainActor final class CommandIcons {
    static let shared=CommandIcons()
    private final class Entry:NSObject {let image:NSImage;let date:Date;init(_ image:NSImage,_ date:Date) {self.image=image;self.date=date}}
    private let images=NSCache<NSString,Entry>()
    private let missingApps=NSCache<NSString,NSDate>()
    private let lifetime:TimeInterval
    init(lifetime:TimeInterval=30) {self.lifetime=lifetime;images.countLimit=512;missingApps.countLimit=512}

    func image(for command:Command)->NSImage? {
        guard command.kind=="app" || command.kind=="path" else {return nil}
        let key="\(command.kind):\(command.target)" as NSString,now=Date()
        if let cached=images.object(forKey:key),now.timeIntervalSince(cached.date)<lifetime {return cached.image}
        let path:String
        if command.kind=="app" {
            if let missing=missingApps.object(forKey:key),now.timeIntervalSince(missing as Date)<lifetime {return nil}
            guard let url=NSWorkspace.shared.urlForApplication(withBundleIdentifier:command.target) else {
                images.removeObject(forKey:key);missingApps.setObject(now as NSDate,forKey:key);return nil
            }
            path=url.path
        } else {path=NSString(string:command.target).expandingTildeInPath}
        guard FileManager.default.fileExists(atPath:path) else {images.removeObject(forKey:key);return nil}
        let image=NSWorkspace.shared.icon(forFile:path)
        missingApps.removeObject(forKey:key);images.setObject(Entry(image,now),forKey:key)
        return image
    }
}

/// Source labels are metadata, not per-frame work. Cache successful and missing bundle lookups.
@MainActor final class ApplicationNames {
    static let shared=ApplicationNames()
    private final class Entry:NSObject {let name:String;let date:Date;init(_ name:String,_ date:Date) {self.name=name;self.date=date}}
    private let cache=NSCache<NSString,Entry>()
    private let now:()->Date
    private let resolve:@MainActor (String)->String?
    init(now:@escaping()->Date=Date.init,resolve:@escaping @MainActor (String)->String?=ApplicationNames.resolve) {self.now=now;self.resolve=resolve;cache.countLimit=512}
    private static func resolve(_ id:String)->String? {
        guard let url=NSWorkspace.shared.urlForApplication(withBundleIdentifier:id),let bundle=Bundle(url:url) else {return nil}
        return bundle.object(forInfoDictionaryKey:"CFBundleDisplayName") as? String ?? bundle.object(forInfoDictionaryKey:"CFBundleName") as? String
    }
    func name(_ id:String)->String {
        let date=now(),key=id as NSString
        if let entry=cache.object(forKey:key),date.timeIntervalSince(entry.date)<30 {return entry.name}
        let name=resolve(id) ?? id;cache.setObject(Entry(name,date),forKey:key);return name
    }
}
