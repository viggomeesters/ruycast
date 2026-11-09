import AppKit
import ApplicationServices
import SwiftUI
import LauncherModels

struct SystemSearchResults:Sendable {
    let commands:[Command]
    var menus:[String:MenuActionInfo]=[:]
    var windows:[String:WindowActionInfo]=[:]
    var tabs:[String:BrowserTabInfo]=[:]
}

/// App-provided shortcuts are presentation data, never launcher registrations.
struct MenuShortcut:Codable,Equatable,Sendable {
    let keys:[String]
    let label:String
    init?(character:String?,glyph:Int?,virtualKey:Int?,modifiers:Int?) {
        guard let modifiers,(0...15).contains(modifiers) else {return nil}
        let glyphs:[Int:String]=[2:"⇥",3:"⇤",4:"⌤",9:"Space",10:"⌦",11:"↵",12:"↵",13:"↵",23:"⌫",27:"⎋",28:"Clear",98:"⇞",100:"←",101:"→",102:"↖",103:"Help",104:"↑",105:"↘",106:"↓",107:"⇟"]
        let special:[Int:String]=[36:"↵",48:"⇥",49:"Space",51:"⌫",53:"⎋",76:"⌤",115:"↖",116:"⇞",117:"⌦",119:"↘",121:"⇟",123:"←",124:"→",125:"↓",126:"↑",122:"F1",120:"F2",99:"F3",118:"F4",96:"F5",97:"F6",98:"F7",100:"F8",101:"F9",109:"F10",103:"F11",111:"F12",105:"F13",107:"F14",113:"F15",106:"F16",64:"F17",79:"F18",80:"F19",90:"F20"]
        let controls:[String:String]=["\r":"↵","\n":"↵","\t":"⇥","\u{0008}":"⌫","\u{007f}":"⌫","\u{001b}":"⎋"," ":"Space","\u{f700}":"↑","\u{f701}":"↓","\u{f702}":"←","\u{f703}":"→"]
        let key:String?
        if let glyph,let known=glyphs[glyph] {key=known}
        else if let glyph,(111...122).contains(glyph) {key="F\(glyph-110)"}
        else if let glyph,(135...137).contains(glyph) {key="F\(glyph-122)"}
        else if let character,!character.isEmpty,character.count==1 {
            if let known=controls[character] {key=known}
            else if let scalar=character.unicodeScalars.first,(0xf704...0xf726).contains(scalar.value) {key="F\(scalar.value-0xf703)"}
            else if !character.unicodeScalars.contains(where:{CharacterSet.controlCharacters.contains($0) || (0xe000...0xf8ff).contains($0.value)}) {let upper=character.uppercased();key=upper.count==1 ? upper:character}
            else {key=virtualKey.flatMap {special[$0]}}
        } else {key=virtualKey.flatMap {special[$0]}}
        guard let key else {return nil}
        var symbols=[String](),names=[String]()
        for (bit,symbol,name) in [(4,"⌃","Control"),(2,"⌥","Option"),(1,"⇧","Shift")] where modifiers & bit != 0 {symbols.append(symbol);names.append(name)}
        if modifiers & 8 == 0 {symbols.append("⌘");names.append("Command")}
        keys=symbols+[key]
        let spoken=["⇥":"Tab","⇤":"Back Tab","⌤":"Enter","⌦":"Forward Delete","↵":"Return","⌫":"Delete","⎋":"Escape","⇞":"Page Up","←":"Left Arrow","→":"Right Arrow","↖":"Home","↑":"Up Arrow","↘":"End","↓":"Down Arrow","⇟":"Page Down"]
        label=(names+[spoken[key] ?? key]).joined(separator:" + ")
    }
}
struct MenuActionInfo:Codable,Equatable,Sendable {
    let application:String
    let path:String
    let title:String
    let shortcut:MenuShortcut?
    let enabled:Bool
}
struct MenuAncestor {
    let element:AXUIElement
    let title:String
    let role:String
}
struct MenuShortcutBadge:View {
    let shortcut:MenuShortcut
    let application:String
    var body:some View {
        HStack(spacing:3) {
            ForEach(Array(shortcut.keys.enumerated()),id:\.offset) {_,key in
                Text(key).font(.system(size:12,weight:.medium,design:.rounded))
                    .frame(minWidth:19,minHeight:21).padding(.horizontal,key.count>1 ? 4:0)
                    .background(Color.primary.opacity(0.045),in:RoundedRectangle(cornerRadius:5))
                    .overlay(RoundedRectangle(cornerRadius:5).strokeBorder(Color.primary.opacity(0.15),lineWidth:0.7))
            }
        }.foregroundStyle(.secondary).fixedSize()
            .accessibilityElement(children:.ignore).accessibilityLabel("\(application) shortcut: \(shortcut.label)")
            .help("\(application) shortcut: \(shortcut.label)")
    }
}
