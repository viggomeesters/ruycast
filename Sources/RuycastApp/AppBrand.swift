import AppKit
import LauncherCore

/// Ruycast's own cut-ray R. The same geometry serves the Dock and menu bar;
/// no font or SF Symbol is needed to reproduce the identity.
enum AppBrand {
    private static let navy=NSColor(srgbRed:0.075,green:0.102,blue:0.157,alpha:1)
    private static let cream=NSColor(srgbRed:0.976,green:0.953,blue:0.902,alpha:1)
    private static let warm=NSColor(srgbRed:1,green:0.663,blue:0.353,alpha:1)

    private static let warning=NSColor(srgbRed:0.78,green:0.37,blue:0.10,alpha:1)

    static func menuIcon(healthy:Bool=true)->NSImage {
        let image=NSImage(size:NSSize(width:18,height:18),flipped:false) { rect in
            let color=healthy ? NSColor.black:warning
            drawMark(in:rect.insetBy(dx:1,dy:1),color:color,accent:nil)
            return true
        }
        image.isTemplate=healthy
        image.accessibilityDescription=healthy ? "Ruycast":"Ruycast needs attention"
        return image
    }

    /// All coordinates use a 100-unit square, with a shared optical center.
    /// The open triangular counter and diagonal leg suggest a cursor/ray.
    private static func drawMark(in rect:NSRect,color:NSColor,accent:NSColor?) {
        NSGraphicsContext.saveGraphicsState();defer {NSGraphicsContext.restoreGraphicsState()}
        let transform=NSAffineTransform()
        transform.translateX(by:rect.minX,yBy:rect.minY)
        transform.scaleX(by:rect.width/100,yBy:rect.height/100)
        transform.concat()
        let bowl=NSBezierPath()
        bowl.move(to:NSPoint(x:17,y:15))
        for point in [NSPoint(x:17,y:85),NSPoint(x:58,y:85),NSPoint(x:83,y:61),NSPoint(x:59,y:38),NSPoint(x:34,y:38),NSPoint(x:34,y:15)] {bowl.line(to:point)}
        bowl.close()
        let counter=NSBezierPath()
        counter.move(to:NSPoint(x:34,y:69))
        counter.line(to:NSPoint(x:53,y:69));counter.line(to:NSPoint(x:62,y:61))
        counter.line(to:NSPoint(x:53,y:53));counter.line(to:NSPoint(x:34,y:53));counter.close()
        bowl.append(counter);bowl.windingRule = .evenOdd
        color.setFill();bowl.fill()
        let ray=NSBezierPath()
        ray.move(to:NSPoint(x:39,y:43));ray.line(to:NSPoint(x:59,y:43))
        ray.line(to:NSPoint(x:85,y:15));ray.line(to:NSPoint(x:62,y:15));ray.close()
        (accent ?? color).setFill();ray.fill()
    }

    private static func drawAppIcon(in rect:NSRect) {
        let tile=rect.insetBy(dx:rect.width*0.055,dy:rect.height*0.055)
        navy.setFill();NSBezierPath(roundedRect:tile,xRadius:rect.width*0.205,yRadius:rect.height*0.205).fill()
        drawMark(in:rect.insetBy(dx:rect.width*0.205,dy:rect.height*0.205),color:cream,accent:warm)
    }

    private static func png(size:Int,draw:(NSRect)->Void) throws -> Data {
        guard let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:size,pixelsHigh:size,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0),let context=NSGraphicsContext(bitmapImageRep:bitmap) else {throw LauncherError("Could not draw the app icon.")}
        bitmap.size=NSSize(width:size,height:size)
        NSGraphicsContext.saveGraphicsState();NSGraphicsContext.current=context
        context.imageInterpolation = .high
        NSColor.clear.setFill();NSRect(x:0,y:0,width:size,height:size).fill(using:.copy)
        draw(NSRect(x:0,y:0,width:size,height:size))
        NSGraphicsContext.restoreGraphicsState()
        guard let result=bitmap.representation(using:.png,properties:[:]) else {throw LauncherError("Could not encode the app icon.")}
        return result
    }

    static func renderAssets(_ output:String) throws -> [String:Any] {
        let directory=URL(fileURLWithPath:output,isDirectory:true).appendingPathComponent("assets",isDirectory:true)
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        // Keep bounded intermediates inside the caller's explicit output root.
        // The installer can use a vault .tmp output; nothing goes to /tmp.
        let working=directory.appendingPathComponent(".brand-"+UUID().uuidString,isDirectory:true)
        let iconset=working.appendingPathComponent("AppIcon.iconset",isDirectory:true)
        try FileManager.default.createDirectory(at:iconset,withIntermediateDirectories:true)
        defer {try? FileManager.default.removeItem(at:working)}
        for points in [16,32,128,256,512] {
            for scale in [1,2] {
                let name="icon_\(points)x\(points)"+(scale==2 ? "@2x":"")+".png"
                try png(size:points*scale,draw:drawAppIcon).write(to:iconset.appendingPathComponent(name),options:.atomic)
            }
        }
        let generated=working.appendingPathComponent("AppIcon.icns")
        let process=Process();process.executableURL=URL(fileURLWithPath:"/usr/bin/iconutil")
        process.arguments=["-c","icns",iconset.path,"-o",generated.path]
        process.standardOutput=FileHandle.nullDevice;process.standardError=FileHandle.nullDevice
        try process.run();process.waitUntilExit()
        guard process.terminationStatus==0 else {throw LauncherError("Could not package the app icon. Run the brand export again.")}
        let icns=try Data(contentsOf:generated)
        guard icns.count>8,icns.prefix(4)==Data("icns".utf8) else {throw LauncherError("The app icon package is invalid.")}
        try icns.write(to:directory.appendingPathComponent("AppIcon.icns"),options:.atomic)
        try png(size:1024,draw:drawAppIcon).write(to:directory.appendingPathComponent("app-icon.png"),options:.atomic)
        try png(size:1024) {rect in
            cream.setFill();rect.fill()
            drawAppIcon(in:NSRect(x:256,y:300,width:512,height:512))
            // Actual 18-point menu glyphs at 1x and Retina on both surfaces.
            for (i,dark) in [false,true].enumerated() {
                let bar=NSRect(x:208,y:180-i*64,width:608,height:48)
                (dark ? navy:NSColor.white).setFill();NSBezierPath(roundedRect:bar,xRadius:12,yRadius:12).fill()
                for (j,healthy) in [true,false].enumerated() {
                    let color=healthy ? (dark ? cream:navy):warning
                    drawMark(in:NSRect(x:CGFloat(300+j*280),y:bar.minY+15,width:18,height:18),color:color,accent:nil)
                    drawMark(in:NSRect(x:CGFloat(350+j*280),y:bar.minY+6,width:36,height:36),color:color,accent:nil)
                }
            }
        }.write(to:directory.appendingPathComponent("brand-preview.png"),options:.atomic)
        return ["ok":true,"assets":["assets/AppIcon.icns","assets/app-icon.png","assets/brand-preview.png"],"iconRepresentations":10,"menuIconPoints":18,"healthyMenuIconTemplate":true,"visibleWindowOpened":false,"desktopInputSent":false]
    }
}

func renderBrandAssets(_ output:String) throws -> [String:Any] {try AppBrand.renderAssets(output)}
