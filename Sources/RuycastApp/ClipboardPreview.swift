import AppKit
import SwiftUI
import ImageIO
import LauncherCore
import Darwin
import Combine

struct ClipboardImage {
    let image:CGImage
    let width:Int
    let height:Int
    let bytes:Int
}
extension ClipboardStore {
    func preview(_ clip:Clip) throws -> ClipboardImage? {
        let data:Data
        if clip.contentType=="image" {data=try previewImageData(clip)}
        else if clip.contentType=="files",let raw=clip.fileURLs?.first,let url=URL(string:raw),url.isFileURL, url.host.map({$0.isEmpty || $0=="localhost"}) ?? true {
            guard ["png","jpg","jpeg","gif","tiff","tif","heic","webp","bmp"].contains(url.pathExtension.lowercased()) else {return nil}
            let fd=open(url.path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);guard fd>=0 else {throw LauncherError("Image file is no longer available.")};defer {close(fd)}
            var info=stat(),volume=statfs()
            guard fstat(fd,&info)==0,(info.st_mode & S_IFMT)==S_IFREG,info.st_size>0,info.st_size<=30_000_000,fstatfs(fd,&volume)==0,volume.f_flags & UInt32(MNT_LOCAL) != 0 else {throw LauncherError("Image file cannot be previewed.")}
            var bytes=Data(count:Int(info.st_size))
            let length=bytes.count
            let count=bytes.withUnsafeMutableBytes {buffer -> Int in
                var offset=0
                while offset<length {let n=Darwin.read(fd,buffer.baseAddress!.advanced(by:offset),length-offset);if n<=0 {return -1};offset+=n};return offset
            }
            guard count==length else {throw LauncherError("Image file could not be read.")};data=bytes
        } else {return nil}
        guard let source=CGImageSourceCreateWithData(data as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),let properties=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],let width=properties[kCGImagePropertyPixelWidth] as? Int,let height=properties[kCGImagePropertyPixelHeight] as? Int,width>0,height>0,width<=32768,height<=32768,width<=64_000_000/height,
              let thumbnail=CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,kCGImageSourceThumbnailMaxPixelSize:1600,kCGImageSourceCreateThumbnailWithTransform:true,kCGImageSourceShouldCacheImmediately:true] as CFDictionary) else {throw LauncherError("Image preview is unavailable.")}
        return ClipboardImage(image:thumbnail,width:width,height:height,bytes:data.count)
    }
}
final class ClipboardPreviewModel:ObservableObject {
    @Published var asset:ClipboardImage?
    private(set) var assetID:String?
    var assetIdentity:String? {guard let asset,let assetID else {return nil};return "\(assetID):\(asset.width):\(asset.height):\(asset.bytes)"}
    @Published var error:String?
    @Published var loading=false
    private let queue:OperationQueue={let q=OperationQueue();q.maxConcurrentOperationCount=1;q.qualityOfService = .userInitiated;return q}()
    private var selectedID:String?
    private var generation=0
    private let decode:(Clip,ClipboardStore)throws->ClipboardImage?
    private let deliver:(@escaping ()->Void)->Void
    init(decode:@escaping (Clip,ClipboardStore)throws->ClipboardImage?={try $1.preview($0)},deliver:@escaping (@escaping ()->Void)->Void={DispatchQueue.main.async(execute:$0)}) {self.decode=decode;self.deliver=deliver}
    func load(_ clip:Clip?,store:ClipboardStore) {
        if clip==nil,selectedID==nil,asset==nil,error==nil,!loading {return}
        generation+=1;let version=generation,decode=decode,deliver=deliver
        queue.cancelAllOperations();selectedID=clip?.id;assetID=nil;asset=nil;error=nil;loading=false
        guard let clip,clip.contentType=="image" || clip.contentType=="files" else {return}
        loading=true;let job=BlockOperation()
        job.addExecutionBlock { [weak self,weak job] in
            guard job?.isCancelled==false else {return}
            let result=Result {try decode(clip,store)}
            guard job?.isCancelled==false else {return}
            deliver { [weak self] in
                guard let self,self.generation==version,self.selectedID==clip.id else {return};self.loading=false
                switch result {case .success(let asset):self.assetID=asset == nil ? nil:clip.id;self.asset=asset;case .failure(let error):self.error=error.localizedDescription}
            }
        }
        queue.addOperation(job)
    }
    deinit {queue.cancelAllOperations()}
}
/// Hidden command typing does not invalidate the clipboard subtree. Data
/// changes still invalidate it so deleted content cannot remain in previews.
final class ClipboardRenderGate:ObservableObject {
    weak var state:AppState?
    private var subscription:AnyCancellable?
    private var pending=false
    private var active:Bool
    private var dataRevision:Int
    private(set) var invalidations=0
    init(state:AppState) {
        self.state=state;active=state.mode=="clipboard";dataRevision=state.clipboardDataRevision
        subscription=state.objectWillChange.sink { [weak self] _ in
            guard let self,!self.pending else {return};self.pending=true
            DispatchQueue.main.async { [weak self] in self?.refresh() }
        }
    }
    private func refresh() {
        pending=false;guard let state else {return}
        let visible=state.mode=="clipboard",revision=state.clipboardDataRevision
        guard visible || visible != active || revision != dataRevision else {return}
        active=visible;dataRevision=revision;invalidations+=1;objectWillChange.send()
    }
}
struct ClipboardHistoryView:View {
    let state:AppState
    @StateObject private var updates:ClipboardRenderGate
    @StateObject private var preview=ClipboardPreviewModel()
    @State private var retainedSelectionID:String?
    @State private var imageMetadata:(id:String,width:Int,height:Int,bytes:Int)?
    init(state:AppState) {self.state=state;_updates=StateObject(wrappedValue:ClipboardRenderGate(state:state))}
    @Environment(\.colorScheme) private var scheme
    private var actualSelection:Clip? {state.mode=="clipboard" && !state.clipboardSearchBusy && state.filteredClips.indices.contains(state.selection) ? state.filteredClips[state.selection]:nil}
    private var selected:Clip? {actualSelection ?? retainedSelectionID.flatMap {id in state.clips.first(where:{$0.id==id})}}
    var body:some View {
        GeometryReader {geometry in
            let contentWidth=max(0,geometry.size.width-1)
            HStack(spacing:0) {
                ScrollViewReader {proxy in ScrollView {
                    LazyVStack(alignment:.leading,spacing:2) {
                        HStack {Text("Clipboard").font(.system(size:12,weight:.medium)).foregroundStyle(.secondary);Spacer();Button(state.paused ? "Resume":"Pause") {state.handle(["op":"clipboard.pause","paused":!state.paused]) {_ in}}.buttonStyle(.plain).font(.caption)}.padding(.horizontal,10).padding(.vertical,10)
                        ForEach(Array(state.filteredClips.enumerated()),id:\.element.id) {index,clip in
                            Button {state.selection=index} label:{
                                HStack(spacing:9) {
                                    Image(systemName:clip.contentType=="image" ? "photo":clip.contentType=="files" ? "doc":"doc.text").frame(width:20).foregroundStyle(.secondary)
                                    VStack(alignment:.leading,spacing:3) {
                                        Text(title(clip)).font(.system(size:13,weight:.medium)).lineLimit(2).multilineTextAlignment(.leading)
                                        Text(clip.date.formatted(date:.omitted,time:.shortened)).font(.system(size:11)).foregroundStyle(.secondary)
                                    };Spacer(minLength:0)
                                }.padding(.horizontal,9).padding(.vertical,9).frame(maxWidth:.infinity,alignment:.leading).background(index==state.selection ? Color.primary.opacity(0.09):.clear,in:RoundedRectangle(cornerRadius:7)).contentShape(Rectangle())
                            }.buttonStyle(.plain).id(clip.id)
                        }
                        if state.filteredClips.isEmpty {Text(state.clipboardSearchBusy ? "Searching…":state.query.isEmpty ? "Clipboard is empty":"No clipboard items found").foregroundStyle(.secondary).font(.caption).padding(24)}
                    }.padding(.horizontal,8).padding(.bottom,10)
                }.onChange(of:state.selection) {_,_ in if state.mode=="clipboard",let actualSelection {proxy.scrollTo(actualSelection.id)}}}.frame(width:contentWidth*0.4)
                Divider()
                detail(padding:min(24,contentWidth*0.04))
                    .disabled(actualSelection==nil).accessibilityHidden(actualSelection==nil)
                    .overlay {if actualSelection==nil {Text(state.clipboardSearchBusy ? "Searching…":"Select an item").foregroundStyle(.secondary).frame(maxWidth:.infinity,maxHeight:.infinity).background(SettingsTheme(dark:scheme == .dark,graphite:state.config.theme=="graphite").background)}}.frame(width:contentWidth*0.6,height:max(0,geometry.size.height),alignment:.topLeading).clipped()
            }
        }.onChange(of:actualSelection?.id,initial:true) {_,_ in
            if let actualSelection {retainedSelectionID=actualSelection.id;preview.load(actualSelection,store:state.clipboardStore)}
        }.onChange(of:preview.assetIdentity) {_,_ in
            if let asset=preview.asset,let assetID=preview.assetID,actualSelection?.id==assetID {imageMetadata=(assetID,asset.width,asset.height,asset.bytes)}
        }.onChange(of:state.mode) {_,mode in
            if mode != "clipboard" {preview.load(nil,store:state.clipboardStore)}
        }.onChange(of:state.clips.map(\.id)) {_,ids in
            if let retainedSelectionID,!ids.contains(retainedSelectionID) {self.retainedSelectionID=nil;imageMetadata=nil;preview.load(nil,store:state.clipboardStore)}
        }
    }
    @ViewBuilder private func detail(padding:CGFloat)->some View {
        if let clip=selected {
            VStack(alignment:.leading,spacing:14) {
                if let asset=preview.asset {
                    Image(nsImage:NSImage(cgImage:asset.image,size:NSSize(width:asset.image.width,height:asset.image.height))).renderingMode(.original).resizable().scaledToFit().frame(maxWidth:.infinity,maxHeight:.infinity).accessibilityLabel("Clipboard image preview")
                } else if preview.loading {ProgressView().controlSize(.small).frame(maxWidth:.infinity,maxHeight:.infinity)}
                else if clip.contentType=="image" {Label(preview.error ?? "Image preview unavailable",systemImage:"photo").foregroundStyle(.secondary).frame(maxWidth:.infinity,maxHeight:.infinity)}
                else if clip.contentType=="files" {
                    ScrollView {VStack(alignment:.leading,spacing:12) {ForEach(clip.fileURLs ?? [],id:\.self) {raw in if let url=URL(string:raw) {VStack(alignment:.leading,spacing:4) {Label(url.lastPathComponent,systemImage:"doc");Text(url.path).font(.system(size:12)).foregroundStyle(.secondary).textSelection(.enabled)}}}}.frame(maxWidth:.infinity,alignment:.leading)}
                    if let error=preview.error {Text(error).font(.caption).foregroundStyle(.secondary)}
                } else {
                    ScrollView {Text(clip.text).font(.system(size:14,design:.monospaced)).lineSpacing(3).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)}
                }
                Divider()
                VStack(spacing:7) {
                    info("Source",sourceName(clip.source))
                    info("Type",preview.asset != nil ? "Image":clip.contentType=="files" ? "Files":clip.contentType=="image" ? "Image":"Text")
                    if let asset=imageMetadata,asset.id==clip.id {
                        // Retain row structure across native mode switches, but never
                        // expose values belonging to a cancelled or failed decode.
                        let ready=preview.assetID==clip.id && preview.asset != nil
                        info("Dimensions",ready ? "\(asset.width) × \(asset.height)":"—")
                        info("Size",ready ? memoryLabel(UInt64(asset.bytes)):"—")
                    }
                    else if clip.contentType != "files" && clip.contentType != "image" {info("Characters",String(clip.text.count))}
                    info("Copied",clip.date.formatted(date:.abbreviated,time:.shortened))
                }.font(.system(size:12))
            }.padding(padding)
        } else {Text("Select an item").foregroundStyle(.secondary)}
    }
    private func info(_ label:String,_ value:String)->some View {HStack(alignment:.top,spacing:12) {Text(label).foregroundStyle(.secondary);Spacer();Text(value).multilineTextAlignment(.trailing).lineLimit(2).textSelection(.enabled)}}
    private func title(_ clip:Clip)->String {
        if clip.contentType=="files",let raw=clip.fileURLs?.first,let url=URL(string:raw) {return url.lastPathComponent+(clip.fileURLs!.count>1 ? " +\(clip.fileURLs!.count-1)":"")}
        return clip.contentType=="image" ? "Image":clip.text.replacingOccurrences(of:"\n",with:" ")
    }
    private func sourceName(_ id:String)->String {
        return ApplicationNames.shared.name(id)
    }
}
