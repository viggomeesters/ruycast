import AppKit
import LauncherCore
import LauncherModels

/// Uses a unique named pasteboard and a disposable profile; never touches general.
func checkClipboardStore() throws -> [String:Any] {
    let fm=FileManager.default,root=fm.temporaryDirectory.appendingPathComponent("ruycast-clipboard-check-"+UUID().uuidString,isDirectory:true)
    let pb=NSPasteboard(name:NSPasteboard.Name("ruycast-fixture-"+UUID().uuidString))
    defer {pb.releaseGlobally();try? fm.removeItem(at:root)}
    let store=ClipboardStore(root:root)
    var settings=ClipboardSettings()
    func require(_ value:@autoclosure () throws ->Bool,_ message:String) throws {if try !value() {throw LauncherError(message)}}
    func rejected(_ body:() throws -> Void) throws {do {try body()} catch {return};throw LauncherError("Unsafe clipboard operation was accepted.")}
    pb.clearContents();pb.setString("Synthetic clipboard text Å",forType:.string)
    guard let text=try store.capture(from:pb,source:"fixture",settings:settings) else {throw LauncherError("Text capture failed.")}
    pb.clearContents();try store.copy(text,to:pb)
    try require(pb.string(forType:.string)==text.text,"Text roundtrip failed.")
    settings.maxTextBytes=1
    try require(try store.capture(from:pb,source:"fixture",settings:settings)==nil,"Text limit failed.")
    settings=ClipboardSettings()
    let fileURL=root.appendingPathComponent("synthetic missing file.txt")
    pb.clearContents();pb.writeObjects([fileURL as NSURL])
    guard let files=try store.capture(from:pb,source:"fixture",settings:settings) else {throw LauncherError("File capture failed.")}
    try require(files.contentType=="files" && files.fileURLs==[fileURL.absoluteString],"File references changed.")
    try store.copy(files,to:pb)
    let result=(pb.readObjects(forClasses:[NSURL.self],options:[.urlReadingFileURLsOnly:true]) as? [URL]) ?? []
    try require(result==[fileURL] && !fm.fileExists(atPath:fileURL.path),"File roundtrip copied source files.")
    guard let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:2,pixelsHigh:2,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:8,bitsPerPixel:32) else {throw LauncherError("Fixture image failed.")}
    if let bytes=bitmap.bitmapData {for i in 0..<4 {bytes[i*4]=128;bytes[i*4+1]=64;bytes[i*4+2]=32;bytes[i*4+3]=255}}
    // Re-encode after initializing all pixel bytes.
    guard let imageData=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Fixture PNG failed.")}
    pb.clearContents();pb.setData(imageData,forType:.png)
    guard let image=try store.capture(from:pb,source:"fixture",settings:settings),let path=image.dataFile else {throw LauncherError("Image capture failed.")}
    try require(image.contentType=="image","Image type missing.")
    try store.copy(image,to:pb)
    guard let copiedData=pb.data(forType:.png),let copied=NSBitmapImageRep(data:copiedData) else {throw LauncherError("Image roundtrip failed.")}
    guard let color=copied.colorAt(x:1,y:1)?.usingColorSpace(.deviceRGB),let original=NSBitmapImageRep(data:imageData)?.colorAt(x:1,y:1)?.usingColorSpace(.deviceRGB) else {throw LauncherError("Image pixels missing.")}
    try require(copied.pixelsWide==2 && copied.pixelsHigh==2 && abs(color.redComponent-original.redComponent)<0.005 && abs(color.greenComponent-original.greenComponent)<0.005 && abs(color.blueComponent-original.blueComponent)<0.005 && color.alphaComponent==1,"Image pixels changed.")
    let assets=root.appendingPathComponent("clipboard-assets"),asset=root.appendingPathComponent(path)
    let fileMode=(try fm.attributesOfItem(atPath:asset.path)[.posixPermissions] as? NSNumber)?.intValue
    let dirMode=(try fm.attributesOfItem(atPath:assets.path)[.posixPermissions] as? NSNumber)?.intValue
    try require(fileMode==0o600 && dirMode==0o700,"Clipboard asset permissions failed.")
    settings.maxAssetBytes=1
    try rejected {_=try store.capture(from:pb,source:"fixture",settings:settings)}
    settings=ClipboardSettings();settings.maxTotalAssetBytes=imageData.count
    try rejected {_=try store.capture(from:pb,source:"fixture",settings:settings)}
    settings=ClipboardSettings()
    pb.clearContents();pb.setData(Data("not an image".utf8),forType:.png)
    try rejected {_=try store.capture(from:pb,source:"fixture",settings:settings)}
    var invalidFiles=files;invalidFiles.fileURLs=["https://example.com/fixture"]
    try rejected {try store.copy(invalidFiles,to:pb)}
    var malicious=image;malicious.dataFile="clipboard-assets/../outside.png"
    try rejected {try store.copy(malicious,to:pb)}
    malicious.dataFile="/etc/passwd";try rejected {try store.copy(malicious,to:pb)}
    let unrelated=assets.appendingPathComponent("unrelated.txt"),outside=root.appendingPathComponent("outside.txt")
    try Data("fixture".utf8).write(to:unrelated);try Data("fixture".utf8).write(to:outside)
    let symlinkName=UUID().uuidString+".png",symlink=assets.appendingPathComponent(symlinkName)
    try fm.createSymbolicLink(atPath:symlink.path,withDestinationPath:outside.path)
    malicious.dataFile="clipboard-assets/"+symlinkName
    try rejected {try store.copy(malicious,to:pb)}
    try store.pruneAssets(keeping:[image,malicious],settings:settings)
    try require(fm.fileExists(atPath:asset.path),"Kept image was removed.")
    try store.pruneAssets(keeping:[],settings:settings)
    try require(!fm.fileExists(atPath:asset.path) && fm.fileExists(atPath:outside.path) && fm.fileExists(atPath:unrelated.path),"Asset cleanup escaped ownership.")
    try require((try? fm.destinationOfSymbolicLink(atPath:symlink.path))==outside.path,"Asset cleanup removed a symlink.")
    try fm.removeItem(at:symlink);try fm.removeItem(at:unrelated);try fm.removeItem(at:assets)
    try fm.createSymbolicLink(atPath:assets.path,withDestinationPath:root.path)
    try rejected {try store.pruneAssets(keeping:[],settings:settings)}
    return ["ok":true,"namedPasteboardOnly":true,"textImageFileRoundtrips":true,"privateAssetPermissions":true,"assetAndTotalLimits":true,"safePruningAndSymlinkRejection":true]
}
