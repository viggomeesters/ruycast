import SwiftUI

struct ShortcutPresentation {
    let keys:[String]
    let label:String
    init(_ value:String) {
        let parts=value.lowercased().split(separator:"+").map(String.init)
        let key=parts.last ?? "",mods=Set(parts.dropLast())
        let symbols=["ctrl":"⌃","alt":"⌥","shift":"⇧","cmd":"⌘"]
        let names=["ctrl":"Control","alt":"Option","shift":"Shift","cmd":"Command"]
        let special=["left":"←","right":"→","up":"↑","down":"↓","return":"↵","space":"␣","tab":"⇥","escape":"⎋","delete":"⌫","backspace":"⌫","left-control":"⌃","left-option":"⌥"]
        let full=mods==Set(["ctrl","alt","shift","cmd"])
        let solo=(key=="left-control" && mods==["ctrl"]) || (key=="left-option" && mods==["alt"])
        keys=(solo ? []:full ? ["✦"]:["ctrl","alt","shift","cmd"].filter {mods.contains($0)}.compactMap {symbols[$0]}) + [special[key] ?? key.uppercased()]
        label=full ? "Hyper + \(key)":solo ? "Left \(key=="left-control" ? "Control":"Option")":(["ctrl","alt","shift","cmd"].filter {mods.contains($0)}.compactMap {names[$0]}+[key]).joined(separator:" + ")
    }
}
struct ShortcutBadge:View {
    let value:String
    var body:some View {
        let display=ShortcutPresentation(value)
        HStack(spacing:3) {
            ForEach(Array(display.keys.enumerated()),id:\.offset) {_,key in
                Text(key).font(.system(size:12,weight:.medium,design:.rounded))
                    .frame(minWidth:19,minHeight:21)
                    .padding(.horizontal,key.count>1 ? 4:0)
                    .background(Color.primary.opacity(0.045),in:RoundedRectangle(cornerRadius:5))
                    .overlay(RoundedRectangle(cornerRadius:5).strokeBorder(Color.primary.opacity(0.15),lineWidth:0.7))
            }
        }.foregroundStyle(.secondary).fixedSize().help(display.label)
            .accessibilityElement(children:.ignore).accessibilityLabel(display.label)
    }
}
