import AppKit
import ApplicationServices
import LauncherCore

final class WindowManager {
    private var saved:[String:Frame]=[:]
    var trusted:Bool { AXIsProcessTrusted() }
    private func attribute(_ element:AXUIElement,_ key:String) throws -> CFTypeRef {
        var value:CFTypeRef?
        let status=AXUIElementCopyAttributeValue(element,key as CFString,&value)
        guard status == .success,let value else { throw LauncherError("This window does not support \(key) (\(status.rawValue)).") };return value
    }
    private func frame(_ w:AXUIElement) throws -> Frame {
        let p=try attribute(w,kAXPositionAttribute) as! AXValue
        let s=try attribute(w,kAXSizeAttribute) as! AXValue
        var point=CGPoint.zero;var size=CGSize.zero
        guard AXValueGetValue(p,.cgPoint,&point),AXValueGetValue(s,.cgSize,&size) else { throw LauncherError("Cannot read the window frame.") }
        return Frame(x:point.x,y:point.y,width:size.width,height:size.height)
    }
    private func apply(_ f:Frame,to w:AXUIElement) throws {
        var point=CGPoint(x:f.x,y:f.y);var size=CGSize(width:f.width,height:f.height)
        // Size first allows a window to cross from a large screen to a smaller one.
        let s=AXUIElementSetAttributeValue(w,kAXSizeAttribute as CFString,AXValueCreate(.cgSize,&size)!)
        let p=AXUIElementSetAttributeValue(w,kAXPositionAttribute as CFString,AXValueCreate(.cgPoint,&point)!)
        guard s == .success,p == .success else { throw LauncherError("This window cannot be moved or resized (\(p.rawValue)/\(s.rawValue)).") }
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
        try apply(dest,to:window)
        let actual=try frame(window)
        guard abs(actual.x-dest.x)<8,abs(actual.y-dest.y)<8,abs(actual.width-dest.width)<8,abs(actual.height-dest.height)<8 else { throw LauncherError("The app constrained the window size or position; the requested layout was not fully applied.") }
        if layout=="restore" {saved.removeValue(forKey:key)}
        return actual
    }
}
