# Architecture

Ruycast has a shared Rust domain library, a native Rust CLI and a Swift macOS host. The user selected this split explicitly on 24 October 2026; AI-enabled means configurable/operable by external agents through files and a background CLI.

| Layer | Responsibility |
|---|---|
| `rust/src/lib.rs` | Strict config validation, normalized shortcut syntax, search ranking, clipboard retention/deduplication and window geometry |
| `rust/src/bin/ruycast.rs` | JSON CLI, config locking/atomic editing, SHA-256 conflict checks, Unix-socket client and bounded process-group script execution |
| `Sources/LauncherCore/Core.swift` | Codable data transfer objects and small synchronous C-ABI adapter to the Rust static library |
| `Sources/RuycastApp/` | Native palette/menu, macOS app discovery/opening, pasteboard observation, global Carbon hotkeys, Accessibility window API, socket host and platform-owned persistence |

Both GUI and CLI use the same JSON action catalog. Config validation runs in Rust on both paths. Search and retention rules execute in Rust even when called from the native UI. The app links Rust statically; no separate model service, Node runtime or webview is required.

The C ABI accepts an operation and JSON payload and returns a heap-owned JSON response. Swift frees every returned pointer with `rustcast_free`. Panics are contained before crossing the ABI boundary. Platform adapters keep AppKit work on the main queue; socket I/O and subprocesses run in background queues. The control server admits at most16 simultaneous clients, bounds request reads and socket writes, and dispatches bulk clipboard reads from immutable snapshots off the main queue.

One host owns the profile through an OS file lock. The control socket is private (0600) inside a private profile directory (0700). Config and history files are readable by the user (0600); clipboard bodies never enter source Git. Runtime data is local and uses no vault writer.

Config changes are observed through a coalescing directory watcher (including atomic replacement), with a five-second metadata fallback, or explicitly reloaded. Unchanged files are not reread. Background observations carry validated snapshot bytes; stale queued deliveries are invalidated by explicit reloads. The host validates before adopting config; invalid edits retain the previous model. Shortcut updates stage additions before removing old registrations, so conflicts keep the previous set. The CLI serializes its own edits with flock, checks an optional expected SHA-256 and checks for manual changes before atomic replacement. An unrelated editor which does not honor the lock can still race the final check/rename; callers needing coordinated changes should use the CLI.

External app opening reports macOS acceptance; it does not claim that remote pages finished loading. Script success requires a zero exit status. Output is drained while retaining at most 64 KiB per stream. Scripts receive no shell interpolation and run in a fresh process group; the runner terminates the group at completion or timeout, including descendants retaining stdout/stderr. Window moves read the resulting frame and report app-enforced constraints. Clipboard paste verifies that the intended app activated before emitting Command–V.

## Native design

A compact keyboard-first palette with familiar launcher conventions: neutral gray/dark and white/light surfaces, 38-point command rows, genuine 24-point application icons, inline alias badges, right-aligned action kinds, system type and a short footer. The palette opens centered in the cursor display’s visibleFrame and clamps to smaller displays. Native icon lookup is cached by command kind/target and retains original icon colors. The native menu provides launcher, clipboard, configuration, reload and quit. Toasts do not activate a window or intercept input and disappear after five seconds. Search/palette opening is an explicit action; startup and reload do not steal focus.

Offscreen content renders validate layout only. Live global shortcuts, focus, Accessibility and window movement require a separate desktop interaction check. No such check is inferred from a build or process listing.

## Primary API references

The C boundary follows Rust's [FFI guidance](https://doc.rust-lang.org/nomicon/ffi.html). The script runner uses the stable Unix [CommandExt.process_group](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#method.process_group) API. Installed build toolchains: Rust 1.92.0 and Swift 6.2.3, targeting macOS 14+.

## Search hot path

The Mac host owns one immutable Rust `SearchIndex` through `CommandSearchIndex`. Catalog changes rebuild normalized ids, titles, aliases and searchable text once. Query bytes cross the C boundary with an explicit UTF-8 byte length, including embedded NULs; results are native-word indices written into a Swift-owned buffer bounded by the catalog size. Swift validates the returned count and indices. The index has an explicit free function; the legacy JSON-result ABI remains available for compatibility. The existing stateless Rust search uses the same ranking implementation.

`AppState` publishes materialized results when the query, configuration, discovered apps or relevant usage ranking changes. Selection, drawing and repeated result reads do not rerun search or serialize commands. Clipboard filtering runs only in clipboard mode on a bounded worker with one active and one latest pending request. Old publications are canceled; a pending query clears actionable old results. Normal command typing never scans history. CLI searches query the index directly without changing the visible query or results. Folder commands use the shared neutral glyph family; application rows retain native colored icons.

## Daily launcher flows (0.2)

The search field is a native NSTextField/field-editor delegate: edit notifications update the query synchronously, and movement/Return/Escape selectors reach shared selection actions. The panel handles Command–K; selected-item actions provide open/insert, copy, edit and remove, plus creation and library navigation. Memory-only forms request capture text/template arguments or edit commands. Capture adapters call the existing provider scripts; they do not create a new vault writer. Provider JSON is checked before success and partial daily-link failures remain visible. GUI Quick Note opens the saved note; background CLI capture only saves and returns its path.

Snippet expansion uses an ephemeral bounded matcher and a main-run-loop CGEvent tap. The final trigger key is consumed only after synchronous delivery setup and a second focus/security check; failed checks pass the original event through. Generated events carry a private tag and no modifiers. The shared edit executor removes the preceding keyword, emits Unicode in <=16 UTF-16-unit scalar-safe chunks and restores the template cursor. Automatic expansion leaves the clipboard unchanged. Secure entry, excluded apps, modified/repeat keys, pointer/cursor/app changes and the visible palette reset matching. Application frameworks can interpret injected events differently; native payload checks and offscreen editing do not prove compatibility with every external app.

`scripts/build-native.sh` invalidates Swift artifacts when the linked Rust archive hash changes. SwiftPM does not track this external static library as a source dependency; this prevents stale domain behavior from reaching either debug checks or release installation.

## Clipboard work and resource bounds

Pasteboard access stays on the main thread. A bounded immutable text/file/image snapshot crosses to one utility worker, which owns PNG conversion, Rust retention, history writes and asset cleanup. At most one capture is in flight; polling rechecks the latest clipboard change after it completes. This remains polling, not a guarantee to record every OS clipboard transition. Revision checks prevent stale publications. Imports, deletions and clipboard-setting changes read the latest persisted history inside the same queue transaction so an accepted capture cannot be overwritten by an older main-thread snapshot. Clear intentionally removes it. Cleanup failure after a successful save preserves and publishes the capture with an error.

Process ordering is materialized when a batch or sort choice changes, preserving PID/start-time identity. Native icon caches have at most512 entries and a30-second refresh window; missing apps can recover after installation. Website work has at most64 outstanding origins, four active fetches and1024 retry entries. Icon misses also notify views when slots become available. Preview decoding remains serial and bounded; no speculative larger preview cache was added.

See [performance measurements and limits](PERFORMANCE.md) for reproducible commands and the distinction between model timings, hidden native fixtures and physical desktop measurements.


### Search quality and local recency

The shared Rust index ranks exact IDs/aliases/keywords first, then exact titles, title prefixes, title word prefixes/initials, existing all-term substring matches, and title subsequences. Recency sorts only within the same relevance tier; title ordering remains the deterministic fallback. Empty queries retain the existing usage-count behavior.

`SearchRanking` keeps at most 512 command IDs and successful launcher-selection timestamps in a private `search-ranking.json`. Entries older than 90 days and invalid/future timestamps are removed at load/save. CLI runs, hotkeys, dry runs and failed actions do not train launcher ranking. The typed ranked ABI receives catalog-aligned weights; unknown/stale IDs cannot add commands to a search. Diagnostics expose only entry count and limits. This optional local metadata is included in installer preservation/backup checks. No query, argument, snippet or clipboard text enters it.


Interaction tracing scopes a pending input span to ObjectIdentifier of the native SearchInput. Its representable update completes the span after applying the current binding; dismantle cancels it. Other fields cannot complete/cancel that owner's span. NSHostingView.draw only records an optional callback counter, independent of reconciliation: live layer-backed drawing may bypass that callback. Hidden tests drive the actual retained field/delegate and wait for reconciliation before explicit captures. These are software boundaries, not display presentation.
