import AppKit
import ImageIO
import UniformTypeIdentifiers
import LauncherCore
import LauncherModels
import Darwin

/// Owns only PNG assets in the profile. File clipboard items retain references.
enum ClipboardPayload:Sendable {case files([URL]);case image(Data);case text(String)}
final class ClipboardStore:Sendable {
    let root:URL
    init(root:URL) {self.root=root}
    func history(fallback:[Clip]) throws -> [Clip] {
        let url=root.appendingPathComponent("clipboard.json")
        return FileManager.default.fileExists(atPath:url.path) ? try Files.decode([Clip].self,data:Data(contentsOf:url)):fallback
    }
    private var assets:URL {root.appendingPathComponent("clipboard-assets",isDirectory:true)}
    private func directory() throws -> Int32 {
        try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        if mkdir(assets.path,0o700) != 0 && errno != EEXIST {throw LauncherError("Clipboard asset directory unavailable.")}
        let fd=open(assets.path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)
        guard fd >= 0 else {throw LauncherError("Unsafe clipboard asset directory.")}
        var ready=false;defer {if !ready {close(fd)}}
        var mode=stat()
        guard fstat(fd,&mode)==0,mode.st_uid==getuid() else {throw LauncherError("Clipboard asset permissions unavailable.")}
        if mode.st_mode & 0o777 != 0o700 {guard fchmod(fd,0o700)==0 else {throw LauncherError("Clipboard asset permissions unavailable.")}}
        // No new asset may inherit a readable ACL, even from an existing folder.
        guard let acl=acl_init(0) else {throw LauncherError("Clipboard asset permissions unavailable.")}
        defer {acl_free(UnsafeMutableRawPointer(acl))}
        guard acl_set_fd_np(fd,acl,ACL_TYPE_EXTENDED)==0 else {throw LauncherError("Clipboard asset permissions unavailable.")}
        if let current=acl_get_fd_np(fd,ACL_TYPE_EXTENDED) {
            defer {acl_free(UnsafeMutableRawPointer(current))};var entry:acl_entry_t?
            guard acl_valid(current)==0,acl_get_entry(current,Int32(ACL_FIRST_ENTRY.rawValue),&entry) != 0 else {throw LauncherError("Clipboard asset permissions unavailable.")}
        } else if errno != ENOENT {throw LauncherError("Clipboard asset permissions unavailable.")}
        guard fstat(fd,&mode)==0,(mode.st_mode & S_IFMT)==S_IFDIR,mode.st_uid==getuid(),mode.st_mode & 0o777==0o700 else {throw LauncherError("Clipboard asset permissions unavailable.")}
        ready=true;return fd
    }
    private func filename(_ path:String?) throws -> String {
        guard let path=path else {throw LauncherError("Clipboard image is missing.")}
        let parts=path.split(separator:"/",omittingEmptySubsequences:false)
        guard parts.count==2,parts[0]=="clipboard-assets",parts[1].hasSuffix(".png"),UUID(uuidString:String(parts[1].dropLast(4))) != nil else {throw LauncherError("Unsafe clipboard asset path.")}
        return String(parts[1])
    }
    private func size(_ name:String,fd:Int32)->Int? {
        var s=stat()
        guard fstatat(fd,name,&s,AT_SYMLINK_NOFOLLOW)==0,(s.st_mode & S_IFMT)==S_IFREG,s.st_uid==getuid(),s.st_nlink==1,s.st_size>=0 else {return nil}
        return Int(s.st_size)
    }
    private func ownedNames()->[String] {
        ((try? FileManager.default.contentsOfDirectory(atPath:assets.path)) ?? []).filter {(try? filename("clipboard-assets/"+$0)) != nil}
    }
    private func readImage(_ path:String?) throws -> Data {
        let name=try filename(path),fd=try directory();defer {close(fd)}
        let file=openat(fd,name,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);guard file>=0 else {throw LauncherError("Clipboard image is unavailable.")};defer {close(file)}
        var st=stat();guard fstat(file,&st)==0,(st.st_mode & S_IFMT)==S_IFREG,st.st_uid==getuid(),st.st_nlink==1,st.st_size>0,st.st_size<=100_000_000 else {throw LauncherError("Invalid clipboard image asset.")}
        var data=Data(count:Int(st.st_size))
        let count=data.count
        let readCount=data.withUnsafeMutableBytes {bytes -> Int in
            var n=0
            while n<count {let r=Darwin.read(file,bytes.baseAddress!.advanced(by:n),count-n);if r<=0 {return -1};n+=r};return n
        }
        guard readCount==count else {throw LauncherError("Clipboard image could not be read.")};return data
    }
    func previewImageData(_ clip:Clip) throws -> Data {try readImage(clip.dataFile)}
    private func png(_ input:Data,maxBytes:Int) throws -> Data {
        guard input.count<=maxBytes,input.count>0,let source=CGImageSourceCreateWithData(input as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),let props=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],let w=props[kCGImagePropertyPixelWidth] as? Int,let h=props[kCGImagePropertyPixelHeight] as? Int,w>0,h>0,w<=8192,h<=8192,w<=16_000_000/h,let image=CGImageSourceCreateImageAtIndex(source,0,[kCGImageSourceShouldCache:false] as CFDictionary) else {throw LauncherError("Clipboard image exceeds the supported size or is invalid.")}
        let data=NSMutableData();guard let dest=CGImageDestinationCreateWithData(data,UTType.png.identifier as CFString,1,nil) else {throw LauncherError("Clipboard image conversion unavailable.")}
        CGImageDestinationAddImage(dest,image,nil)
        guard CGImageDestinationFinalize(dest),data.length<=maxBytes else {throw LauncherError("Clipboard image exceeds its storage limit.")};return data as Data
    }
    func capture(from pb:NSPasteboard,source:String,settings:ClipboardSettings) throws -> Clip? {
        guard let payload=try snapshot(from:pb,settings:settings) else {return nil}
        return try capture(payload,source:source,settings:settings)
    }
    func snapshot(from pb:NSPasteboard,settings:ClipboardSettings) throws -> ClipboardPayload? {
        let urls=(pb.readObjects(forClasses:[NSURL.self],options:[.urlReadingFileURLsOnly:true]) as? [URL]) ?? []
        if !urls.isEmpty {
            guard urls.count<=1000,urls.allSatisfy({$0.isFileURL && $0.absoluteString.utf8.count<=16_384}) else {throw LauncherError("Clipboard file list exceeds the supported size.")}
            return .files(urls)
        }
        if let data=pb.data(forType:.png) ?? pb.data(forType:.tiff) {
            guard !data.isEmpty,data.count<=max(0,settings.maxAssetBytes ?? 15_000_000) else {throw LauncherError("Clipboard image exceeds its storage limit.")}
            return .image(data)
        }
        guard let text=pb.string(forType:.string),!text.isEmpty,text.utf8.count<=settings.maxTextBytes else {return nil};return .text(text)
    }
    func capture(_ payload:ClipboardPayload,source:String,settings:ClipboardSettings) throws -> Clip? {
        if case .files(let urls)=payload {
            var clip=Clip(text:"Files: "+urls.map(\.lastPathComponent).joined(separator:", "),source:source)
            guard clip.text.utf8.count<=settings.maxTextBytes else {throw LauncherError("Clipboard file list exceeds the text limit.")}
            clip.contentType="files";clip.fileURLs=urls.map(\.absoluteString);return clip
        }
        if case .text(let text)=payload {guard !text.isEmpty,text.utf8.count<=settings.maxTextBytes else {return nil};return Clip(text:text,source:source)}
        guard case .image(let data)=payload else {return nil}
        return try captureImage(data,source:source,settings:settings)
    }
    private func captureImage(_ data:Data,source:String,settings:ClipboardSettings) throws -> Clip? {
            let maxBytes=max(0,settings.maxAssetBytes ?? 15_000_000),totalMax=max(0,settings.maxTotalAssetBytes ?? 100_000_000)
            let image=try png(data,maxBytes:maxBytes),fd=try directory();defer {close(fd)}
            var total=0
            for size in ownedNames().compactMap({size($0,fd:fd)}) {
                guard size<=totalMax-total else {throw LauncherError("Clipboard image storage is full.")};total+=size
            }
            guard image.count<=totalMax,total<=totalMax-image.count else {throw LauncherError("Clipboard image storage is full.")}
            let name=UUID().uuidString+".png",file=openat(fd,name,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0o600)
            guard file>=0 else {throw LauncherError("Clipboard image could not be stored.")}
            var completed=false;defer {close(file);if !completed {_=unlinkat(fd,name,0)}}
            let written=image.withUnsafeBytes {bytes -> Bool in
                var offset=0
                while offset<image.count {let n=Darwin.write(file,bytes.baseAddress!.advanced(by:offset),image.count-offset);if n<=0 {return false};offset+=n};return true
            }
            guard written,fsync(file)==0 else {throw LauncherError("Clipboard image could not be stored.")};completed=true
            var clip=Clip(text:"Image",source:source);clip.contentType="image";clip.dataFile="clipboard-assets/"+name;return clip
    }
    func copy(_ clip:Clip,to pb:NSPasteboard) throws {
        switch clip.contentType ?? "text" {
        case "image":
            let data=try png(readImage(clip.dataFile),maxBytes:100_000_000)
            pb.clearContents();guard pb.setData(data,forType:.png) else {throw LauncherError("Clipboard image could not be copied.")}
        case "files":
            guard let values=clip.fileURLs,!values.isEmpty,values.count<=1000 else {throw LauncherError("Invalid clipboard file list.")}
            let urls=values.compactMap(URL.init(string:));guard urls.count==values.count,urls.allSatisfy({$0.isFileURL && $0.host.map({$0.isEmpty || $0=="localhost"}) ?? true}) else {throw LauncherError("Invalid clipboard file reference.")}
            pb.clearContents();guard pb.writeObjects(urls as [NSURL]) else {throw LauncherError("Clipboard files could not be copied.")}
        case "text":pb.clearContents();guard pb.setString(clip.text,forType:.string) else {throw LauncherError("Clipboard text could not be copied.")}
        default:throw LauncherError("Unsupported clipboard item type.")
        }
    }
    func pruneAssets(keeping:[Clip],settings:ClipboardSettings) throws {
        var existing=stat();if lstat(assets.path,&existing) != 0 && errno==ENOENT {return}
        let fd=try directory();defer {close(fd)}
        let keep=Set(keeping.compactMap {try? filename($0.dataFile)})
        for name in ownedNames() where !keep.contains(name) && size(name,fd:fd) != nil {
            guard unlinkat(fd,name,0)==0 else {throw LauncherError("Unused clipboard image could not be removed.")}
        }
    }
}
