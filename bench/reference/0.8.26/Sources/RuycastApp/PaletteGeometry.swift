import AppKit

/// AppKit coordinates, including displays left of/below the primary screen.
enum PaletteGeometry {
    static let preferredSize=NSSize(width:760,height:490)
    static let clipboardPreferredSize=NSSize(width:1080,height:700)
    static func frame(in visibleFrame:NSRect,mode:String="commands")->NSRect {
        let preferred=mode=="clipboard" ? clipboardPreferredSize:preferredSize
        let inset=min(24,max(0,min(visibleFrame.width,visibleFrame.height)/12))
        let width=min(preferred.width,max(1,visibleFrame.width-2*inset))
        let height=min(preferred.height,max(1,visibleFrame.height-2*inset))
        return NSRect(x:visibleFrame.midX-width/2,y:visibleFrame.midY-height/2,width:width,height:height)
    }
}
