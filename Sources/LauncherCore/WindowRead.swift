import Foundation

/// Bounded Accessibility attribute reads; hard errors and absent values fail immediately.
public enum WindowRead {
    public static func perform<Value>(_ attribute:String,retryingStatus:Int32,
                                      read:() -> (status:Int32,value:Value?),
                                      pause:(TimeInterval) -> Void = {Thread.sleep(forTimeInterval:$0)}) throws -> Value {
        let delays:[TimeInterval]=[0.02,0.04,0.08]
        for attempt in 0...delays.count {
            let result=read()
            if result.status==0,let value=result.value {return value}
            guard result.status==retryingStatus else {
                throw LauncherError("Cannot read \(attribute) from this window (\(result.status)).")
            }
            guard attempt<delays.count else {
                throw LauncherError("The app did not respond while reading \(attribute). Try again.")
            }
            pause(delays[attempt])
        }
        throw LauncherError("Cannot read \(attribute) from this window.")
    }
}
