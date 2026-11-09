// Test-only ABI to immutable 0.8.26 types. Never part of Package.swift or a product payload.
// Workloads, fixtures, clocks, statistics, SDK substitution and safety authority live in Rust.
import AppKit
import LauncherCore
import LauncherModels

private typealias F1Guard = @convention(c) (UInt32) -> Int32
private typealias F1Reply = @convention(c) (UnsafeMutableRawPointer?, UnsafeRawPointer?, Int) -> Void
@MainActor private var f1Guard: F1Guard?

@MainActor private func f1Require(_ operation: UInt32) {
    precondition(Thread.isMainThread && f1Guard?(operation) == 1,
                 "Complete Rust-owned pre-entry SDK guard required before legacy object access")
}
@MainActor private final class F1Box {
    let state: AppState
    let palette: PaletteController?
    let ownsLifecycle: Bool
    init(_ root: URL, _ headless: Bool, _ nativeView: Bool) throws {
        state = try AppState(root: root, headless: headless)
        palette = nativeView ? PaletteController(state: state) : nil
        ownsLifecycle = true
    }
    init(_ root: URL, _ headless: Bool, _ nativeView: Bool, systemServices: SystemServices) throws {
        state = try AppState(root: root, headless: headless, systemServices: systemServices)
        palette = nativeView ? PaletteController(state: state) : nil
        ownsLifecycle = true
    }
    init(borrowing state: AppState, palette: PaletteController?) {
        self.state = state
        self.palette = palette
        ownsLifecycle = false
    }
}
@MainActor private func f1Box(_ pointer: UnsafeMutableRawPointer) -> F1Box {
    Unmanaged<F1Box>.fromOpaque(pointer).takeUnretainedValue()
}

@_cdecl("rcf1_install_guard") @MainActor
func rcf1InstallGuard(_ guardFunction: (@convention(c) (UInt32) -> Int32)?) {
    precondition(Thread.isMainThread)
    f1Guard = guardFunction
}

@_cdecl("rcf1_create") @MainActor
func rcf1Create(_ root: UnsafePointer<CChar>, _ headless: Int32, _ nativeView: Int32) -> UnsafeMutableRawPointer? {
    f1Require(1)
    do {
        return Unmanaged.passRetained(try F1Box(URL(fileURLWithPath: String(cString: root)), headless != 0, nativeView != 0)).toOpaque()
    } catch { return nil }
}

@_cdecl("rcf1_drop") @MainActor
func rcf1Drop(_ pointer: UnsafeMutableRawPointer) {
    f1Require(2)
    if f1Box(pointer).ownsLifecycle { f1Box(pointer).state.shutdown() }
    Unmanaged<F1Box>.fromOpaque(pointer).release()
}

@_cdecl("rcf1_query") @MainActor
func rcf1Query(_ pointer: UnsafeMutableRawPointer, _ query: UnsafeRawPointer, _ count: Int) -> Int32 {
    f1Require(3)
    guard count >= 0, count <= 16 * 1024 * 1024,
          let value = String(data: Data(bytes: query, count: count), encoding: .utf8) else { return 0 }
    f1Box(pointer).state.query = value
    return 1
}

@_cdecl("rcf1_mode") @MainActor
func rcf1Mode(_ pointer: UnsafeMutableRawPointer, _ mode: UnsafeRawPointer, _ count: Int) -> Int32 {
    f1Require(4)
    guard count >= 0, count <= 16 * 1024 * 1024,
          let value = String(data: Data(bytes: mode, count: count), encoding: .utf8) else { return 0 }
    f1Box(pointer).state.mode = value
    return 1
}

@_cdecl("rcf1_select") @MainActor
func rcf1Select(_ pointer: UnsafeMutableRawPointer, _ selection: Int) {
    f1Require(5)
    f1Box(pointer).state.selection = selection
}

@_cdecl("rcf1_actions") @MainActor
func rcf1Actions(_ pointer: UnsafeMutableRawPointer, _ opened: Int32) {
    f1Require(6)
    f1Box(pointer).state.actionsOpen = opened != 0
}

@_cdecl("rcf1_snapshot") @MainActor
func rcf1Snapshot(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
    f1Require(7)
    let state = f1Box(pointer).state
    guard let data = try? JSONSerialization.data(withJSONObject: [
        "rows": jsonValue(state.visibleCommands), "selection": state.selection,
        "actionsOpen": state.actionsOpen, "config": jsonValue(state.config),
        "filteredClips": jsonValue(state.filteredClips), "query": state.query,
        "mode": state.mode, "message": state.message, "busy": state.busy,
        "clipboardSearchBusy": state.clipboardSearchBusy,
        "paletteVisible": state.paletteVisible
    ], options: [.sortedKeys]) else { return nil }
    return Unmanaged.passRetained(data as NSData).toOpaque()
}

@_cdecl("rcf1_call") @MainActor
func rcf1Call(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeRawPointer, _ count: Int,
             _ reply: @escaping @convention(c) (UnsafeMutableRawPointer?, UnsafeRawPointer?, Int) -> Void,
             _ context: UnsafeMutableRawPointer?) -> Int32 {
    f1Require(8)
    guard count >= 0, count <= 16 * 1024 * 1024,
          let request = try? JSONSerialization.jsonObject(with: Data(bytes: bytes, count: count)) as? [String: Any]
    else { return 0 }
    // Rust owns the reply context through completion; callbacks copy transient bytes synchronously.
    let contextAddress = UInt(bitPattern: context)
    f1Box(pointer).state.handle(request) { response in
        f1Require(9)
        guard let data = try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]) else {
            reply(UnsafeMutableRawPointer(bitPattern: contextAddress), nil, 0); return
        }
        data.withUnsafeBytes { buffer in reply(UnsafeMutableRawPointer(bitPattern: contextAddress), buffer.baseAddress, buffer.count) }
    }
    return 1
}

@_cdecl("rcf1_content_view") @MainActor
func rcf1ContentView(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
    f1Require(10)
    guard let view = f1Box(pointer).palette?.panel.contentView else { return nil }
    return Unmanaged.passUnretained(view).toOpaque()
}

#if F1_COMPLETE_HOST
@_cdecl("rcf1_borrow_delegate") @MainActor
func rcf1BorrowDelegate() -> UnsafeMutableRawPointer? {
    f1Require(11)
    guard let delegate = NSApplication.shared.delegate as? AppDelegate,
          let state = delegate.state else { return nil }
    return Unmanaged.passRetained(F1Box(borrowing: state, palette: delegate.palette)).toOpaque()
}
#endif

// Derived counts only: same snapshot-read guard, no timing or workload logic.
@_cdecl("rcf1_result_count") @MainActor
func rcf1ResultCount(_ pointer: UnsafeMutableRawPointer) -> Int {
    f1Require(7)
    return f1Box(pointer).state.results.count
}

@_cdecl("rcf1_visible_count") @MainActor
func rcf1VisibleCount(_ pointer: UnsafeMutableRawPointer) -> Int {
    f1Require(7)
    return f1Box(pointer).state.visibleCommands.count
}

@_cdecl("rcf1_selected_id_count") @MainActor
func rcf1SelectedIDCount(_ pointer: UnsafeMutableRawPointer) -> Int {
    f1Require(7)
    return f1Box(pointer).state.selectedCommand?.id.count ?? 0
}

@_cdecl("rcf1_filtered_count") @MainActor
func rcf1FilteredCount(_ pointer: UnsafeMutableRawPointer) -> Int {
    f1Require(7)
    return f1Box(pointer).state.filteredClips.count
}

@_cdecl("rcf1_config_data") @MainActor
func rcf1ConfigData(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
    f1Require(7)
    guard let value = try? Data(contentsOf: f1Box(pointer).state.configURL) else { return nil }
    return Unmanaged.passRetained(value as NSData).toOpaque()
}

@_cdecl("rcf1_encode_clips") @MainActor
func rcf1EncodeClips(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
    f1Require(7)
    guard let value = try? Files.encode(f1Box(pointer).state.clips) else { return nil }
    return Unmanaged.passRetained(value as NSData).toOpaque()
}

@_cdecl("rcf1_record_timing") @MainActor
func rcf1RecordTiming(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeRawPointer,
                      _ count: Int, _ milliseconds: Double) -> Int32 {
    f1Require(8)
    guard count >= 0, count <= 16 * 1024 * 1024,
          let value = String(data: Data(bytes: bytes, count: count), encoding: .utf8) else { return 0 }
    f1Box(pointer).state.analytics.recordTiming(value, ms: milliseconds)
    return 1
}

@_cdecl("rcf1_record_recency") @MainActor
func rcf1RecordRecency(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeRawPointer,
                       _ count: Int, _ epoch: Double) -> Int32 {
    f1Require(8)
    guard count >= 0, count <= 16 * 1024 * 1024, epoch.isFinite,
          let value = String(data: Data(bytes: bytes, count: count), encoding: .utf8) else { return 0 }
    do {
        try f1Box(pointer).state.searchRanking.recordSuccess(value, now: Date(timeIntervalSince1970: epoch))
        return 1
    } catch { return 0 }
}

// Exact process-owner ABI forwarding; Rust supplies all fixture/workload logic.
@_cdecl("rcf1_apply_system_results") @MainActor
func rcf1ApplySystemResults(_ pointer: UnsafeMutableRawPointer,
                           _ commandsBytes: UnsafeRawPointer?, _ commandsCount: Int,
                           _ resourcesBytes: UnsafeRawPointer?, _ resourcesCount: Int,
                           _ preserveSelection: Int32) -> Int32 {
    f1Require(8)
    guard commandsCount >= 0, commandsCount <= 16 * 1024 * 1024,
          resourcesCount >= 0, resourcesCount <= 16 * 1024 * 1024,
          let commandsBytes, let resourcesBytes,
          let commands = try? JSONDecoder().decode([Command].self, from: Data(bytes: commandsBytes, count: commandsCount)),
          let resources = try? JSONDecoder().decode([String: ProcessResource].self, from: Data(bytes: resourcesBytes, count: resourcesCount))
    else { return 0 }
    f1Box(pointer).state.applySystemResults(commands, resources: resources, preserveSelection: preserveSelection != 0)
    return 1
}

@_cdecl("rcf1_process_sort") @MainActor
func rcf1ProcessSort(_ pointer: UnsafeMutableRawPointer) -> Int32 {
    f1Require(7)
    switch f1Box(pointer).state.processSort {
    case .cpu: return 0
    case .memory: return 1
    case .name: return 2
    }
}

@_cdecl("rcf1_set_process_sort") @MainActor
func rcf1SetProcessSort(_ pointer: UnsafeMutableRawPointer, _ value: Int32) -> Int32 {
    f1Require(8)
    let sort: ProcessSort
    switch value {
    case 0: sort = .cpu
    case 1: sort = .memory
    case 2: sort = .name
    default: return 0
    }
    f1Box(pointer).state.setProcessSort(sort)
    return 1
}

@_cdecl("rcf1_process_snapshot") @MainActor
func rcf1ProcessSnapshot(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
    f1Require(7)
    let state = f1Box(pointer).state
    guard let data = try? JSONSerialization.data(withJSONObject: [
        "results": jsonValue(state.results), "visibleCommands": jsonValue(state.visibleCommands),
        "processResources": jsonValue(state.processResources), "processSort": state.processSort.rawValue,
        "selection": state.selection, "selectedID": state.selectedCommand?.id as Any? ?? NSNull()
    ], options: [.sortedKeys]) else { return nil }
    return Unmanaged.passRetained(data as NSData).toOpaque()
}

@_cdecl("rcf1_create_with_process_provider") @MainActor
func rcf1CreateWithProcessProvider(_ root: UnsafePointer<CChar>, _ headless: Int32, _ nativeView: Int32,
                                  _ provider: (@convention(c) (UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer?)?,
                                  _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    f1Require(1)
    guard let provider else { return nil }
    let contextAddress = UInt(bitPattern: context)
    var effects = SystemServices.Effects()
    effects.processes = {
        guard let pointer = provider(UnsafeMutableRawPointer(bitPattern: contextAddress)) else {
            throw LauncherError("F1 process provider refused.")
        }
        let data = Unmanaged<NSData>.fromOpaque(pointer).takeRetainedValue()
        guard data.length <= 16 * 1024 * 1024 else { throw LauncherError("F1 process provider exceeds the ABI limit.") }
        return try JSONDecoder().decode([ProcessResource].self, from: data as Data)
    }
    do {
        return Unmanaged.passRetained(try F1Box(URL(fileURLWithPath: String(cString: root)), headless != 0,
                                               nativeView != 0, systemServices: SystemServices(effects: effects))).toOpaque()
    } catch { return nil }
}

@_cdecl("rcf1_cancel_system_search") @MainActor
func rcf1CancelSystemSearch(_ pointer: UnsafeMutableRawPointer) {
    f1Require(8)
    f1Box(pointer).state.systemServices.cancel()
}

@_cdecl("rcf1_set_commands") @MainActor
func rcf1SetCommands(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeRawPointer, _ count: Int) -> Int32 {
    f1Require(8)
    guard count >= 0, count <= 16 * 1024 * 1024,
          let commands = try? JSONDecoder().decode([Command].self, from: Data(bytes: bytes, count: count))
    else { return 0 }
    f1Box(pointer).state.config.commands = commands
    return 1
}
