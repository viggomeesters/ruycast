import Foundation

/// One synchronous lock owns the Sendable value through closure-scoped access.
/// Callers must keep closures short and never reenter this box.
final class LockedValue<Value:Sendable>:@unchecked Sendable {
    private let lock=NSLock()
    private var value:Value
    init(_ value:Value) {self.value=value}
    func read<Result>(_ body:(Value)throws->Result) rethrows->Result {
        lock.lock();defer {lock.unlock()};return try body(value)
    }
    func update<Result>(_ body:(inout Value)throws->Result) rethrows->Result {
        lock.lock();defer {lock.unlock()};return try body(&value)
    }
}
