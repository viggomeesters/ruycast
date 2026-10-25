import Foundation

/// Transient physical state only; no typed text or keyboard history is retained.
public struct KeyboardState {
    public private(set) var held:[UInt64:Set<UInt32>]=[:]
    private var candidate:(device:UInt64,usage:UInt32,time:Double)?
    public init() {}
    public var usages:Set<UInt32> {held.values.reduce(into:Set<UInt32>()) {$0.formUnion($1)}}
    public var capsHeld:Bool {usages.contains(0x39)}
    public mutating func cancelTap() {candidate=nil}
    /// Returns a left modifier usage only for an isolated, short press/release.
    public mutating func input(device:UInt64,usage:UInt32,down:Bool,time:Double)->UInt32? {
        let already=held[device]?.contains(usage)==true
        guard already != down else {return nil}
        var action:UInt32?
        if down {
            let solo=usages.isEmpty
            candidate=nil
            if solo && [0xe0,0xe2].contains(usage) {candidate=(device,usage,time)}
            held[device,default:[]].insert(usage)
        } else {
            if let c=candidate,c.device==device,c.usage==usage,time>=c.time,time-c.time<=0.5 {action=usage}
            candidate=nil
            held[device]?.remove(usage)
            if held[device]?.isEmpty==true {held.removeValue(forKey:device)}
        }
        return action
    }
    public mutating func remove(device:UInt64) {held.removeValue(forKey:device);candidate=nil}
    public mutating func reset() {held=[:];candidate=nil}
}
