import AppKit
import ApplicationServices
import LauncherCore

final class WindowManager {
    private var saved:[String:Frame]=[:]
    var trusted:Bool { AXIsProcessTrusted() }
    private func attribute(_ element:AXUIElement,_ key:String) throws -> CFTypeRef {
        try WindowRead.perform(key,retryingStatus:AXError.cannotComplete.rawValue,read:{
            var value:CFTypeRef?
            let status=AXUIElementCopyAttributeValue(element,key as CFString,&value)
            return (status.rawValue,value)
        })
    }
    private func frame(_ w:AXUIElement) throws -> Frame {
        let p=try attribute(w,kAXPositionAttribute) as! AXValue
        let s=try attribute(w,kAXSizeAttribute) as! AXValue
        var point=CGPoint.zero;var size=CGSize.zero
        guard AXValueGetValue(p,.cgPoint,&point),AXValueGetValue(s,.cgSize,&size) else { throw LauncherError("Cannot read the window frame.") }
        return Frame(x:point.x,y:point.y,width:size.width,height:size.height)
    }
    private func apply(_ f:Frame,to w:AXUIElement) throws -> Frame {
        func write(_ key:String,_ value:AXValue) throws {
            let status=AXUIElementSetAttributeValue(w,key as CFString,value)
            // cannotComplete can be transient or arrive after a partial change.
            // Accept only verified geometry, never the status code alone.
            guard status == .success || status == .cannotComplete else {
                throw LauncherError("This window does not allow changing \(key) (\(status.rawValue)).")
            }
        }
        return try WindowPlacement.apply(f,read:{try self.frame(w)},resize:{ target in
            var size=CGSize(width:target.width,height:target.height)
            try write(kAXSizeAttribute,AXValueCreate(.cgSize,&size)!)
        },position:{ target in
            var point=CGPoint(x:target.x,y:target.y)
            try write(kAXPositionAttribute,AXValueCreate(.cgPoint,&point)!)
        })
    }
    func move(_ layout:String,pid:pid_t?) throws -> Frame {
        guard trusted else { throw LauncherError("Allow Ruycast in System Settings → Privacy & Security → Accessibility, then retry.") }
        guard let pid else { throw LauncherError("Select a window first.") }
        let app=AXUIElementCreateApplication(pid)
        let window=try attribute(app,kAXFocusedWindowAttribute) as! AXUIElement
        let old=try frame(window)
        let screens=NSScreen.screens
        guard let primary=screens.first else { throw LauncherError("No screen available.") }
        let top=primary.frame.maxY
        func available(_ s:NSScreen)->Frame { let f=s.visibleFrame;return Frame(x:f.minX,y:top-f.maxY,width:f.width,height:f.height) }
        let oldRect=CGRect(x:old.x,y:old.y,width:old.width,height:old.height)
        let index=screens.indices.max { a,b in
            let af=available(screens[a]),bf=available(screens[b])
            let ar=oldRect.intersection(CGRect(x:af.x,y:af.y,width:af.width,height:af.height)),br=oldRect.intersection(CGRect(x:bf.x,y:bf.y,width:bf.width,height:bf.height))
            return (ar.isNull ? 0:ar.width*ar.height)<(br.isNull ? 0:br.width*br.height)
        } ?? 0
        let key="\(pid)-\(CFHash(window))"
        let dest:Frame
        if layout == "restore" { guard let f=saved[key] else { throw LauncherError("No saved frame for this window yet.") };dest=f }
        else if layout == "next-screen" || layout == "previous-screen" {
            guard screens.count>1 else { throw LauncherError("Only one screen is connected.") }
            let n=(index+(layout=="next-screen" ? 1:screens.count-1))%screens.count
            let a=available(screens[index]),b=available(screens[n])
            let width=min(old.width,b.width),height=min(old.height,b.height)
            dest=Frame(x:min(max(b.x+(old.x-a.x)/a.width*b.width,b.x),b.x+b.width-width),y:min(max(b.y+(old.y-a.y)/a.height*b.height,b.y),b.y+b.height-height),width:width,height:height)
        } else { dest=try available(screens[index]).layout(layout) }
        if layout != "restore",saved[key]==nil { if saved.count>500 {saved.removeAll()};saved[key]=old }
        let actual=try apply(dest,to:window)
        if layout=="restore" {saved.removeValue(forKey:key)}
        return actual
    }
}
