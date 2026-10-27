# Changelog

## 0.8.10 — 2026-10-27

- Recover direct title typos before weak subsequences: `fnider` now finds Finder even when a longer title also matches its letters. Any exact, prefix, word-start/initial or literal match preserves the existing complete result list.
- Prefer a corrected full single-word title over a corrected word inside a longer title. Keep recency within relevance tiers, stable ties, disabled filtering and literal execution IDs/arguments.

## 0.8.9 — 2026-10-27

- Recover a single title-word typo when normal search finds no matches, including adjacent transpositions such as `fnider`. Existing result lists and their ranking remain unchanged; IDs, aliases and argument text stay literal.
- Cover bounded Unicode/length behavior, ambiguous suggestions, exact alias priority, disabled filtering and native/CLI parity. No query history or dependency is added.

## 0.8.8 — 2026-10-27

- Keep private JSON, analytics exports and icon cache writes private before any content is written. Secure an empty staging directory, create a 0600 payload without inherited ACLs, then atomically replace the destination. Preserve selected folder permissions and old contents on pre-commit failure.
- Apply the private policy to CLI configuration writes and no-replace initialization/relocation, retaining synchronization and profile aliases. Secure the local profile before state reads or IPC.
- Remove inherited ACL access from the private clipboard asset directory before creating image data. Add real inherited-ACL, umask, symlink-target and failed-write regression coverage.

## 0.8.7 — 2026-10-27

- Preserve existing files, symlinks and active control endpoints during startup; recover only verified stale sockets. Remove only the socket identity created by this instance when stopping or rolling back.
- Reject linked or nonregular instance locks without blocking, clean up failed startup and return a failure exit status. Add eight real headless-process recovery tests with disposable profiles.

## 0.8.6 — 2026-10-27

- Skip foreground-app attribution lookups for unchanged clipboard polls, while preserving activation settling, capture cadence, excluded/concealed sources and snapshot validation.
- Add named-pasteboard regression coverage for app/pasteboard transitions, in-flight capture and paused/disabled polling. Keep the polling cost benchmark behind an opt-in compiler flag. No energy or battery improvement is claimed.

## 0.8.5 — 2026-10-27

- Keep build artifacts protected through all local test, Settings and performance consumers, including controller interruption. Concurrent builds wait instead of removing a running check's executable.
- Bind successful pipeline receipts to current source, test/export inputs and consumed binaries/modules; reject stale proof and remove old passes when a new attempt fails. Preserve all existing checks.

## 0.8.4 — 2026-10-27

- Size-optimize only the Rust CLI adapter while retaining speed optimization for the shared search kernel.

- Prepare stable title/catalog ordering once per immutable search index; avoid repeated Unicode-length scans and eager title comparisons per query. Preserve relevance, recency and tied-result order.
- Roll back instance locks and socket descriptors when control-server startup fails, allowing the same instance to retry without an explicit stop. Preserve active-owner socket handling.

## 0.8.3 — 2026-10-27

- Strip local/debug symbols from release executables after verifying identical code/data sections and UUIDs. Keep the matching host dSYM and unstripped binaries privately in `.build/symbols/`; bind compiler options, toolchains and prepared artifacts in the build receipt.
- Optimize native UI code for size while keeping pure search projection in the speed-optimized core. Preserve stable usage ordering, scope filtering and result limits.
- Use Rust thin LTO and one codegen unit, preserving unwind/panic containment.
- Let the headless host wake for maintenance once per minute instead of polling UI/clipboard work it cannot perform. Native UI polling cadence is unchanged.

## 0.8.2 — 2026-10-27

- Compile the entire native host in Swift 6 mode, with explicit MainActor UI ownership and checked Sendable worker boundaries. Building requires Swift 6.2 or later.
- Give interactive and CLI system searches separate cancellation sessions, with bounded pending work and fair scheduling.
- Keep search collection predicates independent of UI isolation using immutable Sendable snapshots, and skip the redundant ordinary-mode filtering pass.
- Keep bulk clipboard reply encoding off the main thread and retain listening descriptors until their accept worker exits.
- Fix the completed clipboard-filter benchmark after observable-state extraction; earlier completion timings from 0.8.1 are invalid. The separate enqueue metric remains valid.

## 0.8.1 — 2026-10-27

- Give palette, clipboard, settings and command catalog separate observable owners; SwiftUI tracks the properties each consumer reads.
- Keep clipboard query and selection independent from command typing/navigation, preserving persistent native hosts and selected clipboard identity.
- Remove the broad Combine invalidation gate and keep appearance-only changes from rebuilding the search catalog.
- Preserve command generation/cancellation rules and update menu health through explicit Observation dependencies.

## 0.8.0 — 2026-10-27

- Compile portable command models as a separate Swift 6 module and import the Rust ABI through a checked C header.
- Align recency scores only when catalog or persisted ranking changes; repeated typing reuses the immutable snapshot. Reset, reload and failed-save behavior are checked.
- Add real linked-ABI, architecture-boundary and production search invalidation checks. Keep per-query output buffers local after a prototype found no worthwhile reuse benefit.
- This is the first architecture migration step for Issue #33. Feature-state isolation and the complete native concurrency migration remain separate work; no total UI-speed or physical-frame claim.


## 0.7.16 — 2026-10-26

- Correct interaction diagnostics: schema 2 measures owner-scoped input-to-field reconciliation through the actual NSViewRepresentable update, independently of offscreen capture. Field teardown cancels pending spans.
- Rename the optional NSHostingView draw counter to host.drawCallback and explicitly exclude result drawing and physical display latency; normal layer-backed display may not invoke this callback.

## 0.7.15 — 2026-10-26

- Match title word starts, initials and subsequences. Exact command identifiers and aliases stay first, followed by exact titles. Recent successful launcher choices break ties within the same relevance tier.
- Keep at most 512 local choice timestamps, bounded to 90 days at load/save, with `ruycast search-ranking reset --confirm`. Queries and argument text are never stored in ranking metadata.

## 0.7.14

- Transfer indexed search results through a catalog-bounded native integer buffer, removing JSON serialization and decoding from each query. Existing matching, ordering, scope and limits remain unchanged.

## 0.7.13 — 2026-10-26

- Add bounded, privacy-safe interaction stage timings, Instruments signposts and a hidden native interaction benchmark.

## 0.7.12 — 2026-10-26

- Show snippet keywords, every alias and configured keyboard shortcuts directly in launcher results. Long bindings wrap without truncation.

## 0.7.11 — 2026-10-26 · Script replies and project choices

- Add searchable keyboard selection for script-provided choices, retaining failed selections and provider reports.
- Allow a local adapter update to be staged directly before signing, without entering public source.

## 0.7.10 — 2026-10-26

- Close the launcher immediately when opening an app; report launch failures and keep a reopened palette visible.

## 0.7.9 — 2026-10-26 · Snippet keyboard controls

- Switch snippet choices immediately with Up/Down or Left/Right after typing the keyword.
- Insert with Return or keypad Enter and cancel with Escape, without opening the dropdown first.
- Preserve free-text editing, IME composition and modifier shortcuts; suppress repeated insertion.

## 0.7.8 — 2026-10-26 · Natural dates

- Accept English and Dutch natural dates in existing ISO date script inputs, with a live date preview before Enter. Invalid or incomplete dates keep the input open.

## 0.7.7 — 2026-10-26 · Snippet choices

- Typing an imported snippet keyword with argument options opens a native choice menu.
- Confirm replaces the keyword in the original editor; Escape leaves it untouched.
- Recheck the app, available field metadata and secure-input policy before inserting.

## 0.7.6 — 2026-10-26 · Accessibility read recovery

- Retry temporary Accessibility read failures when selecting a window and reading its frame, so a transient response can recover within the same shortcut.
- Distinguish an unresponsive app from a genuinely unavailable window attribute; persistent and hard failures remain visible.

## 0.7.5 — 2026-10-26 · Window placement

- Reapply window size after moving so half-screen layouts fill the available height on the first shortcut.
- Retry transient Accessibility write failures with bounded frame verification; report persistent constraints accurately.

## 0.7.4 — 2026-10-26 · Command key settings

- Add a General Settings switch for separate Left and Right Command shortcuts. Off pauses retained sided shortcuts and records generic Command; existing configurations keep their behavior.
- Report missing Input Monitoring for keyboard features and offer its permission settings.

## 0.7.3 — 2026-10-25 · Command-side shortcuts

- Bind left and right Command independently, record and display their side, and keep generic Command shortcuts through “Use Either Command”.

## 0.7.2 — 2026-10-25 · Native lifecycle stability

- Keep command and clipboard hosting views attached across mode changes to avoid growing AppKit dependency bookkeeping.
- Skip hidden clipboard redraws during command typing while reconciling deletions and clear operations.
- Cancel image previews on mode exit and reject stale deliveries, including reopening the same item.
- Restore search focus after mode changes; strengthen native lifecycle and clipboard correctness checks.

## 0.7.1 — 2026-10-25 · Snippet expansion

- Expand snippet keywords in editors that do not expose their focused accessibility element, including Codex and Obsidian.
- Keep OS secure input, recognized password fields, permission and excluded-app guards; bound accessibility waits and recover disabled event taps.
- Render `{datetime}` with a written month and local 24-hour time; retain explicit numeric formats.

## 0.7.0 — 2026-10-25 · Launcher polish

- Enter one-field command input directly in the launcher; expand to a form when needed.
- Choose System, Light, Dark or Graphite appearance and hide the menu bar icon.
- New app and menu bar identity, with an attention indicator for runtime problems.
- Larger clipboard previews, a quieter command editor and hourly Dashboard activity.
- Open Settings, configuration and System Monitor from the launcher; assign Settings a shortcut.

## 0.6.3 — 2026-10-25 · First public source preview

- Retain command errors locally with trace IDs, timings, provider details and original exit status.
- Inspect recent failures with `ruycast logs`; keep private inputs and successful script output out of logs.
- Choose a persistent configuration folder through Settings or `ruycast config directory`, independently of private local history and runtime files.
- Preserve configuration bytes when changing folders, refuse existing destination configs, and keep the app’s live configuration observer connected to the selected folder.
- Publish a clean MIT-licensed source repository, source release, public-safe native visuals and issue/pull-request templates.

## 0.6.2

- Neutral first-run commands, portable source installation and optional personal integrations.
- Product-first documentation, native visual examples and a clean source export.
- Build receipts prevent installing binaries from changed or incomplete builds.

## 0.6.1 — Responsive clipboard and background work

- Ordinary command typing no longer scans clipboard history. Clipboard searches run on a coalescing background worker and discard stale results.
- Clipboard image conversion, retention and persistence run on one bounded worker; imports, deletion and settings changes preserve accepted captures.
- Cache process ordering, observe configuration changes without rereading unchanged files, and avoid unchanged startup/history/asset writes.
- Bound website-icon requests/retry bookkeeping and CLI connection workers; move bulk clipboard CLI reads off the main thread.
- Reproducible release benchmarks, hidden lifecycle fixtures and passive resource sampling. Measurement boundaries are documented in docs/PERFORMANCE.md.

## 0.6.0 — Local Dashboard and diagnostics

- Command use, sources, success/failure and recent latency summaries in a native Dashboard; prior successful uses are preserved separately.
- Bounded private local storage with pause, retention, reset and a summary-only diagnostic export. No query/input/URL/clipboard payloads or remote telemetry.
- Real runner and CLI checks, corrupt-data preservation, concurrent aggregation and native light/dark renders.

## 0.5.1 — Quick capture keyboard flow

- Enter saves Quick Note, Quick Task and text-capture forms; Shift–Enter adds a line. Command–Enter remains available.
- IME confirmation commits composed text before submission; busy saves ignore duplicate submissions.

## 0.5.0 — Processes, System Monitor and clipboard previews

- Kill Process displays native app icons, CPU, memory and PID with stable selection across sorting and refresh. Kill executes immediately; PID/start-time/owner checks remain enforced.
- Separate native System Monitor with six pages, bounded history and background sampling that stops when closed or minimized. Read-only monitor/processes JSON CLI operations.
- Clipboard text/image/local image-file previews and metadata, serial background decoding, and fixed copied timestamps.
- Native fixture checks cover direct Kill without real termination, selection changes, previews, monitor lifecycle, all six pages in both themes and CLI integration.

## 0.4.1 — Capture and binding feedback

- Quick Note/Quick Task open a focused native multiline editor; Return adds a line, Command–Return saves.
- Command–K adds Alias & Shortcut and Disable. Creation moves to a global footer menu and never inherits the selected command.
- Quick binding saves merge only changed fields, preserving concurrent definition edits and refusing resurrection of removed commands.
- Raycast RAYCFG3 JSON binding audit/repair maps stable extension identities and stages reassignment before collision checks. Existing content and approved modifier taps remain intact.
- Native checks cover editor focus/newlines, quick binding saves, disable, independent creation and process adapter behavior.

## 0.4.0 — Classic Settings

- Six icon tabs above a compact grouped command table; Commands opens first.
- Viggo navy/cream palette, inline aliases, focused shortcut recording and contextual editors, including script arguments.
- Disable commands without losing data; release their hotkeys and prevent search, execution and snippet expansion. App discovery cannot reintroduce disabled applications.
- Clipboard retention/exclusions and general shortcut controls use guarded CLI writes. Failed runtime activation rolls back an unchanged proposed configuration.
- Native offscreen light/dark and minimum-size checks exercise recorder cancellation, modifier taps, conflicts, CLI saves and independent navigation.

## 0.3.2 — Actions keyboard navigation

- Command–K gives Actions its own highlighted keyboard selection; arrows and Return stay inside the menu.
- Escape and Command–K close Actions and return focus to search; background results stay selected.
- Mouse and keyboard share actions, with stable payloads for removal confirmation.

## 0.3.1 — Shortcut badges and website icons

- Compact native keycaps, with one Hyper symbol and clear arrow/letter keys.
- Website favicon/touch-icon discovery and bounded background loading, with a persistent local cache and native fallbacks.
- Full quicklink paths, queries, credentials and cookies are excluded from origin icon requests.

## 0.3.0 — Personal Raycast transition

- Native physical Caps Hyper and solo modifier gestures, with permission/sleep recovery.
- Personal command migration: preserved snippet/link content, preferred apps, aliases and hotkeys.
- Existing native app and Obsidian task providers connected through a checked bridge.
- Native file, menu, window and process search; explicit process termination confirmation.
- Text/image/file clipboard storage and bounded history migration.
- Saved text, generators and histories, with existing Keychain password history preserved.
- Persistent certificate installation; update permission continuity checked separately.

## 0.2.3 — 2026-10-24

Persistent app-specific local certificate signing replaces ad-hoc releases. Pin bundle identifier and certificate in the designated requirement, preserve the private identity outside the repository, and reject missing/changed signing state before stopping the installed host. Native two-build proof and installer-failure tests prevent silent identity changes. Transitioning from the old ad-hoc identity requires one final OS enrollment.

## 0.2.2 — 2026-10-24

Fix stale rendered search/library rows by using stable command and clipboard IDs throughout list rendering and scrolling. Remove footer category tabs. Keep one launcher with searchable actions and a separate reusable Settings window for browsing/editing commands and controlling snippet expansion/clipboard history. Add persistent native rendering regression with OCR, including management navigation and editor views.

## 0.2.1 — 2026-10-24

Rename app, CLI, login identity and canonical profile/repository to Ruycast. Preserve private data and old CLI/environment/path aliases. Installer checks owned destinations, migrates under the exclusive profile lock, verifies data hashes and restores previous app/links/service if startup fails. Permissions are handled separately after the rename.

## 0.2.0 — 2026-10-24

Native field-editor search and arrow navigation, selected-item Command–K actions, input forms for existing Quick Note/Quick Task providers, editable quicklinks and snippets, complete library CLI operations, named quicklink/template arguments, and guarded automatic keyword expansion. Raycast import now supports enabled parameterized/custom-scheme links and all existing snippets without modifying Raycast. Config edits reject concurrent changes. Rust archive changes invalidate Swift artifacts to prevent stale links.

## 0.1.2 — 2026-10-24

Persistent normalized Rust search index; query-only bridging returns indices instead of retransmitting whole commands. Published search results are reused during navigation and unrelated renders. CLI search leaves palette state untouched. Config reload, discovered apps and usage ranking refresh cached results. Consistent neutral command glyphs with distinct halves, thirds, maximize/restore and display directions; genuine app icons retained.

## 0.1.1 — 2026-10-24

Real macOS application icons with cached lookup and missing-app fallback. Centered palette placement on the cursor display, including smaller and offset displays. Raycast-inspired compact neutral rows, inline alias badges and action kinds; commands/clipboard switching remains in the footer.

## 0.1.0 — 2026-10-24

Initial Rustcast MVP: Rust domain core and JSON CLI; native Mac launcher; configurable aliases/hotkeys and app/link/script actions; bounded text clipboard history; Accessibility window layouts; read-only Raycast export importer; silent login startup and local usage counters.
