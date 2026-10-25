import SwiftUI
import LauncherCore

/// A shared 24 pt canvas, neutral tint and consistent stroke for command glyphs.
struct CommandGlyph:View {
    let kind:String
    let target:String
    var body:some View {
        Group {
            if kind=="window" {windowGlyph}
            else {Image(systemName:["snippet":"text.quote","app":"app","url":"link","path":"folder","script":"terminal","builtin":"doc.on.clipboard"][kind] ?? "command")
                .font(.system(size:17,weight:.regular))}
        }.foregroundStyle(.secondary).frame(width:24,height:24).accessibilityHidden(true)
    }
    private var windowGlyph:some View {
        Canvas {context,_ in
            let stroke=StrokeStyle(lineWidth:1.4,lineCap:.round,lineJoin:.round)
            let bounds=CGRect(x:2.5,y:4.5,width:19,height:15)
            func line(_ points:[CGPoint]) {
                var p=Path();if let first=points.first {p.move(to:first);for point in points.dropFirst() {p.addLine(to:point)}}
                context.stroke(p,with:.foreground,style:stroke)
            }
            if target=="restore" {
                line([CGPoint(x:7.5,y:6),CGPoint(x:7.5,y:4.5),CGPoint(x:21.5,y:4.5),CGPoint(x:21.5,y:15.5),CGPoint(x:19,y:15.5)])
                let front=Path(roundedRect:CGRect(x:2.5,y:8.5,width:14,height:11),cornerRadius:2)
                context.stroke(front,with:.foreground,style:stroke)
            } else {
                context.stroke(Path(roundedRect:bounds,cornerRadius:2),with:.foreground,style:stroke)
                let regions:[String:(CGFloat,CGFloat)]=["left-half":(0,0.5),"right-half":(0.5,0.5),"first-third":(0,1/3),"center-third":(1/3,1/3),"last-third":(2/3,1/3),"first-two-thirds":(0,2/3),"last-two-thirds":(1/3,2/3)]
                if let (x,w)=regions[target] {
                    let inner=CGRect(x:bounds.minX+2,y:bounds.minY+2,width:bounds.width-4,height:bounds.height-4)
                    context.fill(Path(roundedRect:CGRect(x:inner.minX+inner.width*x,y:inner.minY,width:inner.width*w,height:inner.height),cornerRadius:0.8),with:.foreground)
                } else if target=="maximize" {
                    line([CGPoint(x:7,y:10),CGPoint(x:7,y:8),CGPoint(x:10,y:8)])
                    line([CGPoint(x:14,y:8),CGPoint(x:17,y:8),CGPoint(x:17,y:10)])
                    line([CGPoint(x:7,y:14),CGPoint(x:7,y:16),CGPoint(x:10,y:16)])
                    line([CGPoint(x:14,y:16),CGPoint(x:17,y:16),CGPoint(x:17,y:14)])
                } else if target=="next-screen" || target=="previous-screen" {
                    let right=target=="next-screen"
                    line([CGPoint(x:7,y:12),CGPoint(x:17,y:12)])
                    line([CGPoint(x:right ? 14:10,y:9),CGPoint(x:right ? 17:7,y:12),CGPoint(x:right ? 14:10,y:15)])
                    line([CGPoint(x:12,y:20),CGPoint(x:12,y:22)])
                    line([CGPoint(x:8,y:22),CGPoint(x:16,y:22)])
                }
            }
        }.frame(width:24,height:24)
    }
}
struct CommandIconSheet:View {
    let examples=Configuration.layouts.map { ("window",$0) } + [("url","Quicklink"),("path","Folder"),("script","Script"),("builtin","Clipboard"),("app","Missing app")]
    var body:some View {
        LazyVGrid(columns:Array(repeating:GridItem(.fixed(145)),count:4),spacing:22) {
            ForEach(examples.indices,id:\.self) {i in
                VStack(spacing:10) {CommandGlyph(kind:examples[i].0,target:examples[i].1);Text(examples[i].1).font(.system(size:11)).lineLimit(1)}.frame(height:55)
            }
        }.padding(24).frame(width:660,height:350).background(Color(nsColor:.windowBackgroundColor))
    }
}
