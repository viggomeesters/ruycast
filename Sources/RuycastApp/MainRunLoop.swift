import Foundation

/// Creation is main-actor isolated, so the scheduled timer belongs to the main
/// run loop. Its legacy Foundation callback executes inline on that same loop.
@MainActor func mainRunLoopTimer(every interval:TimeInterval,repeats:Bool=true,action:@escaping @MainActor @Sendable ()->Void)->Timer {
    Timer.scheduledTimer(withTimeInterval:interval,repeats:repeats) {_ in
        MainActor.assumeIsolated {action()}
    }
}
