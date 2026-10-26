import Foundation

/// Applies and verifies a layout independently of the Accessibility transport.
public enum WindowPlacement {
    public static func matches(_ actual:Frame,_ target:Frame) -> Bool {
        abs(actual.x-target.x)<8 && abs(actual.y-target.y)<8 &&
        abs(actual.width-target.width)<8 && abs(actual.height-target.height)<8
    }

    public static func apply(_ target:Frame,read:() throws -> Frame,
                             resize:(Frame) throws -> Void,position:(Frame) throws -> Void,
                             pause:() -> Void = {Thread.sleep(forTimeInterval:0.02)}) throws -> Frame {
        for attempt in 0..<3 {
            // Shrink before crossing displays, then resize again at the new
            // origin: macOS may clamp a taller window at its previous position.
            try resize(target)
            try position(target)
            try resize(target)
            let actual=try read()
            if matches(actual,target) {return actual}
            if attempt<2 {pause()}
        }
        throw LauncherError("The app constrained or did not finish applying the window size or position. Try again.")
    }
}
