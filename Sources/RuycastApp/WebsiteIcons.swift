import AppKit
import SwiftUI
import LauncherCore
import Combine
import CryptoKit
import ImageIO

/// Requests only the origin, never quicklink paths, queries, credentials or cookies.
final class WebsiteIcons:ObservableObject {
    static let shared=WebsiteIcons()
    @Published private(set) var revision=0
    private let cache:URL
    private let queue:OperationQueue
    private let load:(URL)->Data?
    private let images=NSCache<NSString,NSImage>()
    private var pending=Set<String>()
    private var retryAfter:[String:Date]=[:]
    init(cache:URL=FileManager.default.urls(for:.cachesDirectory,in:.userDomainMask)[0].appendingPathComponent("nl.viggomeesters.ruycast/website-icons"),load:@escaping(URL)->Data?=WebsiteIcons.fetch) {
        self.cache=cache;self.load=load;queue=OperationQueue();queue.name="ruycast.website-icons";queue.maxConcurrentOperationCount=4;queue.qualityOfService = .utility;images.countLimit=512
    }
    static func origin(_ target:String)->URL? {
        guard var parts=URLComponents(string:target),let scheme=parts.scheme?.lowercased(),["https","http"].contains(scheme),let host=parts.host,!host.isEmpty,!host.contains(where:{"{}% ".contains($0)}) else {return nil}
        parts.scheme=scheme;parts.host=host.lowercased();parts.user=nil;parts.password=nil;parts.path="/";parts.query=nil;parts.fragment=nil
        return parts.url
    }
    static func cacheKey(_ origin:URL)->String {SHA256.hash(data:Data(origin.absoluteString.utf8)).map {String(format:"%02x",$0)}.joined()}
    func image(for target:String)->NSImage? {
        guard let origin=Self.origin(target) else {return nil}
        let key=Self.cacheKey(origin)
        if let image=images.object(forKey:key as NSString) {return image}
        guard pending.count<64,!pending.contains(key),(retryAfter[key] ?? .distantPast)<Date() else {return nil}
        pending.insert(key)
        let cache=cache,load=load
        queue.addOperation { [weak self] in
            let file=cache.appendingPathComponent(key+".png"),miss=cache.appendingPathComponent(key+".miss"),fm=FileManager.default
            let attrs=try? fm.attributesOfItem(atPath:file.path)
            let age=Date().timeIntervalSince(attrs?[.modificationDate] as? Date ?? .distantPast)
            var data:Data?
            if (attrs?[.size] as? NSNumber)?.intValue ?? Int.max <= 131072 {data=(try? Data(contentsOf:file)).flatMap(Self.png)}
            let missDate=(try? fm.attributesOfItem(atPath:miss.path)[.modificationDate]) as? Date
            if data != nil {let cached=data;DispatchQueue.main.async {self?.publish(cached,key:key,finished:false)}}
            if age>7*86400 && (missDate.map {Date().timeIntervalSince($0)>3600} ?? true) {
                if let fresh=load(origin).flatMap(Self.png) {
                    data=fresh
                    try? fm.createDirectory(at:cache,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
                    try? fresh.write(to:file,options:.atomic);try? fm.setAttributes([.posixPermissions:0o600],ofItemAtPath:file.path);try? fm.removeItem(at:miss)
                } else {
                    try? fm.createDirectory(at:cache,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);try? Data().write(to:miss);try? fm.setAttributes([.posixPermissions:0o600],ofItemAtPath:miss.path)
                }
                Self.prune(cache)
            }
            let result=data;DispatchQueue.main.async {self?.publish(result,key:key,finished:true)}
        }
        return nil
    }
    private func publish(_ data:Data?,key:String,finished:Bool) {
        if finished {
            pending.remove(key);retryAfter[key]=Date().addingTimeInterval(3600)
            if retryAfter.count>1024,let oldest=retryAfter.min(by:{$0.value<$1.value})?.key {retryAfter.removeValue(forKey:oldest)}
        }
        if let data,let image=NSImage(data:data) {image.isTemplate=false;images.setObject(image,forKey:key as NSString);revision+=1}
        else if finished {revision+=1} // Free slots must also refresh skipped visible requests after misses.
    }
    private static func prune(_ cache:URL) {
        guard let urls=try? FileManager.default.contentsOfDirectory(at:cache,includingPropertiesForKeys:[.contentModificationDateKey],options:.skipsHiddenFiles),urls.count>1024 else {return}
        let ordered=urls.filter {["png","miss"].contains($0.pathExtension)}.sorted {((try? $0.resourceValues(forKeys:[.contentModificationDateKey]).contentModificationDate) ?? .distantPast)<((try? $1.resourceValues(forKeys:[.contentModificationDateKey]).contentModificationDate) ?? .distantPast)}
        for url in ordered.prefix(max(0,ordered.count-1024)) {try? FileManager.default.removeItem(at:url)}
    }
    static func png(_ bytes:Data)->Data? {
        guard bytes.count<=1_048_576,let source=CGImageSourceCreateWithData(bytes as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),let props=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],let w=props[kCGImagePropertyPixelWidth] as? Int,let h=props[kCGImagePropertyPixelHeight] as? Int,w>0,h>0,w<=8192,h<=8192,
              let cg=CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,kCGImageSourceThumbnailMaxPixelSize:64,kCGImageSourceShouldCacheImmediately:true] as CFDictionary) else {return nil}
        return NSBitmapImageRep(cgImage:cg).representation(using:.png,properties:[:])
    }
    static func candidates(html:String,origin:URL)->[URL] {
        let links=try! NSRegularExpression(pattern:"<link\\b[^>]*>",options:.caseInsensitive)
        let attrs=try! NSRegularExpression(pattern:#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#,options:.caseInsensitive)
        var found:[URL]=[]
        for match in links.matches(in:html,range:NSRange(html.startIndex...,in:html)) {
            guard let range=Range(match.range,in:html) else {continue};let tag=String(html[range]);var values:[String:String]=[:]
            for attr in attrs.matches(in:tag,range:NSRange(tag.startIndex...,in:tag)) {
                guard let key=Range(attr.range(at:1),in:tag),let value=(2...4).compactMap({Range(attr.range(at:$0),in:tag)}).first else {continue};values[tag[key].lowercased()]=String(tag[value])
            }
            let rel=(values["rel"] ?? "").lowercased().split(whereSeparator:{$0.isWhitespace})
            guard rel.contains("icon") || rel.contains("apple-touch-icon"),let href=values["href"],let url=URL(string:href.replacingOccurrences(of:"&amp;",with:"&"),relativeTo:origin)?.absoluteURL,["http","https"].contains(url.scheme?.lowercased() ?? ""),url.user==nil,url.password==nil else {continue}
            if !found.contains(url) {found.append(url)}
            if found.count==6 {break}
        }
        return found
    }
    static func fetch(_ origin:URL)->Data? {
        if let data=BoundedWebRequest.get(origin.appendingPathComponent("favicon.ico")),png(data) != nil {return data}
        if let data=BoundedWebRequest.get(origin),let html=String(data:data,encoding:.utf8) {
            for url in candidates(html:html,origin:origin) {if let data=BoundedWebRequest.get(url),png(data) != nil {return data}}
        }
        for name in ["apple-touch-icon.png","favicon.png"] {if let data=BoundedWebRequest.get(origin.appendingPathComponent(name)),png(data) != nil {return data}}
        return nil
    }
}
private final class BoundedWebRequest:NSObject,URLSessionDataDelegate,URLSessionTaskDelegate {
    private var bytes=Data();private var valid=false
    private let done=DispatchSemaphore(value:0)
    static func get(_ url:URL)->Data? {
        let handler=BoundedWebRequest(),config=URLSessionConfiguration.ephemeral
        config.httpShouldSetCookies=false;config.httpCookieStorage=nil;config.urlCredentialStorage=nil;config.urlCache=nil;config.timeoutIntervalForRequest=3;config.timeoutIntervalForResource=4
        let session=URLSession(configuration:config,delegate:handler,delegateQueue:nil)
        var request=URLRequest(url:url);request.setValue("Ruycast/0.3 website icon",forHTTPHeaderField:"User-Agent")
        session.dataTask(with:request).resume()
        let completed=handler.done.wait(timeout:.now()+5) == .success
        session.invalidateAndCancel()
        // Delegate state is read only after completion; a timed-out request has no usable result.
        return completed && handler.valid ? handler.bytes:nil
    }
    func urlSession(_ session:URLSession,dataTask:URLSessionDataTask,didReceive response:URLResponse,completionHandler:@escaping(URLSession.ResponseDisposition)->Void) {
        valid=(response as? HTTPURLResponse).map {(200..<300).contains($0.statusCode)} ?? false
        if response.expectedContentLength>1_048_576 {valid=false}
        completionHandler(valid ? .allow:.cancel)
    }
    func urlSession(_ session:URLSession,dataTask:URLSessionDataTask,didReceive data:Data) {
        if bytes.count+data.count>1_048_576 {valid=false;dataTask.cancel()} else {bytes.append(data)}
    }
    func urlSession(_ session:URLSession,task:URLSessionTask,didCompleteWithError error:Error?) {if error != nil {valid=false};done.signal()}
    func urlSession(_ session:URLSession,task:URLSessionTask,willPerformHTTPRedirection response:HTTPURLResponse,newRequest request:URLRequest,completionHandler:@escaping(URLRequest?)->Void) {
        guard let url=request.url,["http","https"].contains(url.scheme ?? ""),url.user==nil,url.password==nil else {completionHandler(nil);return};completionHandler(request)
    }
}
struct CommandIcon:View {
    let command:Command
    @ObservedObject private var websites=WebsiteIcons.shared
    var body:some View {
        Group {
            if command.kind=="url",let image=websites.image(for:command.target) {Image(nsImage:image).resizable().interpolation(.high).scaledToFit().padding(2).background(Color.white,in:RoundedRectangle(cornerRadius:5))}
            else if command.kind=="url",let app=command.applicationID,let image=CommandIcons.shared.image(for:Command(id:"icon."+app,title:"",kind:"app",target:app)) {Image(nsImage:image).resizable().interpolation(.high).scaledToFit()}
            else if ["app","path"].contains(command.kind),let image=CommandIcons.shared.image(for:command) {Image(nsImage:image).resizable().interpolation(.high).scaledToFit()}
            else {CommandGlyph(kind:command.kind,target:command.target)}
        }.frame(width:24,height:24).accessibilityHidden(true)
    }
}
