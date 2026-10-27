# Performance

Ruycast uses a cached Rust search index, materialized command/process results, background clipboard work and bounded icon requests. Local measurements are model/native-fixture measurements; they do not establish physical keyboard-to-display latency or a comparison against Raycast.

## Reproduce locally

```sh
bash scripts/check-performance.sh
```

This builds release binaries, calibrates CPU counter units against `getrusage`, checks clipboard/configuration races, runs fixed search/clipboard/process fixtures, and measures 1,000 hidden native view/model cycles. Output defaults to `.build/performance`; set `RUYCAST_CHECK_OUTPUT` for another destination.

The deterministic gate checks model search at500/5000 commands, process result reads, clipboard query enqueue cost and retained footprint after1000 cycles. These are absolute local regression budgets, not end-to-end desktop guarantees. It never orders windows front, sends global keys, reads your clipboard or invokes real process termination.

To sample a running host without interacting with it:

```sh
clang -O2 -Wall -Wextra scripts/perf-probe.c -o .build/perf-probe
.build/perf-probe --self-check
python3 scripts/sample-performance.py --pid PID --seconds 600 \
  --probe .build/perf-probe --out .build/performance/host.json --label local-host
```

Use `scripts/benchmark-host.py` for an isolated headless profile with500 commands and 200 text fixtures. It measures30 process starts,1000 real CLI calls per operation, and active/idle intervals. Arguments `--active-seconds 1800 --idle-seconds 3600` select a90-minute stress run; the shorter600/600-second run is the recorded initial soak. Headless readiness excludes hotkeys, app discovery and visible presentation.

## Measurement contract

Release comparisons use500,1000,5000 and 10000 commands, fixed queries/seed,200 approximately8kB clipboard items,1000 synthetic processes, a960k-pixel image and a native Finder icon. Paired baseline/current runs alternate on the same Apple M4 Mac with16GiB RAM, on mains power. Counts and timings are fixtures, not personal usage data.

Durations use a monotonic clock. Per-stage results report sample count, median, p95, maximum, and p99 only for at least1000 observations. Clipboard publication timing waits for the completed asynchronous filter; enqueue timing is reported separately. Image/preview runs have fewer samples and are exploratory. OS caches are not forcibly flushed. Builds and aggressive UI stress are excluded from the paired timing window.

Process CPU counters are Mach-time values in [Apple's XNU implementation](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/task.c). The probe converts them with `mach_timebase_info` using a128-bit intermediate. Its four-thread calibration compares against `getrusage`; the observed125/3 timebase must not be mistaken for1:1 nanoseconds. Physical footprint, resident bytes, wakeups and disk counters remain separate metrics.

## Changes that matter

- Ordinary command typing no longer filters clipboard history. Clipboard queries use one active and one latest pending background filter, preserving Unicode matching and discarding stale publications.
- PNG conversion, Rust retention, history writes and asset cleanup run on one clipboard worker. Only immutable pasteboard snapshots cross from the main thread. Imports/deletes/settings changes transact against the latest saved history; clear prevents old captures reappearing.
- Process ordering is rebuilt when data or sort changes, rather than during every result/selection read. PID/start-time selection checks remain intact.
- A coalescing directory watcher and metadata fallback replace repeated full config reads. Unrelated profile writes do not reread config bytes. Invalid edits retain the working configuration.
- Native icons and source labels have bounded caches and short refresh windows. Website work is limited to64 outstanding origins/four active fetches, with1024 retry entries. A failed fetch releases capacity and refreshes waiting visible rows.
- CLI workers are capped at16 clients with read/write timeouts. Bulk clipboard filtering/encoding runs from an immutable snapshot off the main thread.

Named-pasteboard and isolated-profile checks cover accepted capture followed by import, delete, settings reload and clear before main-thread publication; restart; image persistence; cleanup failure;1000 rapidly changing queries; Unicode parity; selected identity; atomic/in-place config edits; invalid recovery; and unchanged-file reads. Monitor cancellation checks use1000 start/stop cycles and the ordinary resource suite checks sampling stops when closed.

## Known measurement limits

The current 10,000-cycle native gate checks settled growth and the 1,000/5,000/10,000 checkpoints against a 10 MiB ceiling. It is a bounded regression test, not a proof of leak freedom. The earlier roughly 39 MiB result belongs to the initial round below; the follow-up identifies and repairs the native container lifecycle involved.

Full Xcode/Instruments was unavailable on the measurement host. Physical hotkey/focus/frame latency, Energy traces, secure-entry/sleep/wake behavior, battery-versus-mains comparison and a separate-session Raycast comparison remain unmeasured. This follow-up was explicitly limited to background measurements. Hidden native rendering and headless readiness do not establish physical presentation latency. Pasteboard providers, icon misses and external apps can introduce platform-owned delays.

See Apple's [Instruments tutorials](https://developer.apple.com/tutorials/instruments) and [hang investigation guide](https://developer.apple.com/documentation/xcode/understanding-hangs-in-your-app) for the full desktop profiling route.

## Initial measured round — 0.6.1, 25 October 2026

Three alternating release pairs on one M4 Mac reduced command typing with10,000 commands plus200 long clipboard entries from p95 **15.1–15.4 ms to4.0–4.1 ms**. Reading1000 process results fell from about10.9 ms to0.0011 ms by reusing materialized order. Clipboard enqueue work is below0.01 ms; completed filtering still takes about14 ms on the worker. Moving work is not eliminating it.

| Proposed budget | Assessment | Evidence / boundary |
|---|---|---|
| Warm open | not measured | Physical hotkey → presented frame was not measured. |
| Typing/arrows | not measured | Model and hidden-native behavior covered; presentation latency and main-thread traces remain open. |
| Model search | achieved | 500 commands p95 ~0.23 ms; 5000 ~2.1 ms, three paired release runs. |
| Cold host + hotkey ready | not measured | 30 headless starts cover RPC readiness only, not hotkeys/discovery. |
| Clipboard preview | not measured | Decode fixture ~15 ms; selection-to-visible and repeated-image cache budgets unmeasured. |
| Idle CPU/wakeups | achieved within observed scope | Headless600 s:0.068% of one core,1.54 interrupt wakeups/s. Installed0.6.1 personal profile600 s:0.097%,2.08/s. |
| Idle I/O | partially measured | Unchanged config/history tests pass. OS counters include delayed analytics flush; no claim of zero process I/O. |
| Personal-profile footprint | achieved within passive observation | Installed0.6.1,343commands:55.7→55.9 MiB, max56.0 MiB over600 s; no writes observed. |
| Long-lived growth | not achieved | 1000-cycle gate passes; 10000 hidden-native cycles retain ~38.9 MiB. Model-only5000-cycle growth ~0.23 MiB. |
| System Monitor overhead | not measured | Cancellation/lifecycle tested; isolated extra CPU and30-minute visible-open run remain open. |
| Warm CLI | achieved | 1000 status and search RPC subprocess samples within50 ms p95; noisy runs do not support a relative speed claim. |
| Correctness | achieved within tested scope | Isolated capture/config races, stale searches, selected identity, bounded resources and fake process effects pass. Live secure-entry/sleep/wake not remeasured. |

The first investigation and improvement round is complete. The full desktop acceptance is open: the native stress growth and unmeasured rows above are explicit follow-up work, rather than successful measurements.

The installed0.6.1 performance build retained the same binary throughout its600-second passive sample. The later0.6.2 source preview adds neutral defaults and packaging provenance; it is not a second full desktop timing run. Passive observation does not prove that every timer or external clipboard provider is idle.

## Background acceptance follow-up — 0.7.2, 25 October 2026

Repeating the old 0.7.0 native fixture retained **34.88 MiB** across 10,000 mode/search/action cycles. macOS heap and allocation-history samples identified growing `NSKeyValueDependency` / `NSKeyValueDependencyContext` bookkeeping through AppKit control/view attachment and SwiftUI display-list updates. A custom button style and a replacement text view did not solve the problem and were discarded.

The launcher now creates its native command and clipboard hosting containers once, keeps them attached, and switches visibility. Hidden command typing does not invalidate the clipboard subtree; history changes still reconcile deleted items. Leaving clipboard mode cancels image work, generation tokens reject delayed results, and preview ownership includes the clip identity. Metadata from cancelled or failed decodes is not presented as current.

The final-logic release fixture settled at **+1.52 MiB** over 10,000 cycles (42.74 → 44.25 MiB). Checkpoints at 1,000/5,000/10,000 cycles stayed within the 10 MiB gate. An independent build of the exported public source repeated the gate at **+5.34 MiB**; both final-logic builds passed every checkpoint. Another earlier persistent-container run settled below its starting footprint; allocator variation is why the gate uses a ceiling rather than requiring negative growth. The fixture also verifies container identity, a hidden panel and 1,000 monitor start/stop cancellation cycles. These are same-data hidden native results, not physical launcher-open timing or a universal leak claim.

An isolated offscreen CPU chart ran for 30 minutes at its ordinary two-second interval, rendering 901 frames. Independent `getrusage` counters measured 0.679% of one core while active and **0.535 percentage points extra CPU** versus the mean of one-minute idle windows before and after. Peak physical footprint was 19.08 MiB; history stayed at 60 samples and no new sample arrived after stop. This does not measure display presentation, battery power or external pasteboard providers.

The compatible frozen headless base completed **30 minutes active + 60 minutes idle**: 31,131 active requests, then 0.086% of one core and 1.542 interrupt wakeups/s while idle. Its idle footprint peaked at 41.80 MiB and ended at 31.75 MiB; it did not remain at one fixed size. This binary was frozen before the native lifecycle and snippet fixes, so it is a headless reference run rather than an exact final-app soak. A separate final-logic build completed two active minutes and **60 idle minutes**: idle 0.087% of one core, 1.540 interrupt wakeups/s, maximum 43.56 MiB footprint and 43.06 → 40.72 MiB start/end. This frozen fixture still carries the 0.7.1 label; every production source matches 0.7.2 after normalizing only the three version literals. Both host and CLI hashes were independently verified unchanged. No native UI logic differs.

The exact 0.7.2 executable completed two active minutes and ten idle minutes: idle 0.075% of one core, 1.548 interrupt wakeups/s, maximum 41.93 MiB footprint. Across 30 cold starts, headless RPC readiness was p95 121.9 ms; 1,000 subprocess samples per operation gave warm status/search p95 3.18/3.07 ms. These are absolute budget observations, not an alternating paired CLI speed comparison. Delayed analytics flushes and OS I/O are included; no zero-I/O claim is made.

A separate ten-minute passive observation of the installed 0.7.2 personal profile (343 commands, keyboard/snippet services enabled) averaged 0.226% of one core and 2.643 interrupt wakeups/s. Footprint moved from 57.63 to 58.00 MiB, maximum 58.05 MiB. Ordinary user activity was allowed; this is not an enforced idle session. The executable and PID were unchanged throughout. The host was an M4 with 16 GB; a mid-run environment observation reported battery power and no recorded thermal warning, without establishing a controlled battery comparison.

The selected background acceptance is complete. Physical hotkey/focus/frame latency, Energy/battery and a live Raycast comparison remain unmeasured by explicit scope choice; they are not successful measurements.

Reproduce the bounded native gate with `bash scripts/check-performance.sh`. Longer background probes are opt-in:

```sh
.build/swift/release/RuycastApp --benchmark-monitor 1800 60
python3 scripts/benchmark-host.py --host .build/swift/release/RuycastApp \
  --cli .build/rust/release/ruycast --probe .build/perf-probe \
  --out .build/performance/host-90min.json --active-seconds 1800 --idle-seconds 3600
```

Use frozen copies of binaries for long probes. The host script strips external profile/config overrides, asserts its isolated fixture profile and checks that the host file did not change during measurement. The probe never opens the launcher, installs global input hooks or uses the general clipboard. Do not substitute these numbers for the deliberately excluded physical latency, battery and Raycast practice checks.

## Interaction stages (0.7.13)

`ruycast performance` reports bounded in-memory software timings (last256 durations per fixed stage, count, cancellations, p50/p95/max). `ruycast performance reset --confirm` clears them. No query text, command identifiers, snippet bodies or clipboard data is recorded or written. Signposts use only fixed stage names in `nl.viggomeesters.ruycast/Interaction` and can be inspected with Instruments.

- `hotkey.handler`: handler delivery to callback return; excludes hardware/OS delivery before the handler.
- `open.ready`: show request through the queued native focus request; excludes compositor/display presentation. Closing or superseding an open cancels the interval.
- `input.model`: native text-field delegate through synchronous model publication.
- `search.model`: the shared Rust index call and Swift result conversion/filtering.
- `host.drawCallback`: optional palette hosting-view draw override. Layer-backed SwiftUI display may not invoke it; zero is not evidence that no display occurred. Offscreen cacheDisplay fixtures invoke this hook.
- `input.reconcile`: latest native input through application of its current binding in that same field's NSViewRepresentable.updateNSView. Other fields cannot complete it; superseding input, hide or field teardown cancels it. This is field reconciliation, not result drawing, compositor work or physical presentation.

`RuycastApp --benchmark-interactions` drives100 hidden updates through the actual retained search field and its production delegate. Each update must reconcile through SwiftUI before any explicit cacheDisplay. Separate capture checks invoke host.drawCallback. It checks owner identity, teardown, privacy, cancellation, reset identities and bounded retention. It never makes a window visible or emits desktop input. Rendering software stages is distinct from physical key-to-screen latency; a controlled visible acceptance slot is still needed for that claim. The existing model and lifecycle gates remain unchanged.


## 0.7.14: typed search transfer

The indexed Swift path now receives native integer indices in a caller-owned, catalog-bounded buffer, avoiding JSON encoding in Rust and string/data/JSON decoding in Swift on every query. Matching, ordering, scope filtering and CLI limits are unchanged; the buffer is local to the synchronous call.

Six alternating release runs on the same Apple M4 Mac (baseline, candidate, candidate, baseline, baseline, candidate), without concurrent agent builds, produced these medians of per-run model p95:

| Commands | 0.7.13 baseline | 0.7.14 candidate | Reduction |
| --- | --- | --- | --- |
| 500 | 0.237 ms | 0.148 ms | 37.5% |
| 1,000 | 0.456 ms | 0.278 ms | 39.0% |
| 5,000 | 2.206 ms | 1.350 ms | 38.8% |
| 10,000 | 4.382 ms | 2.768 ms | 36.8% |

These are deterministic model fixtures with 1,000 samples per catalog per run, including empty queries. They establish a local model-cost reduction, not physical keyboard-to-display speed or a comparison with another launcher. The frozen baseline binary SHA-256 is `759699fe3081d8f23728f10beae8695de79c86a1cc30eaa84a97979cec7cdb0a`; candidate `89783a47474cf7ed5fc88e380ed4fc89483b935b2a077b54a3725d11f8944cca`.


## 0.7.15: search quality acceptance

Word-prefix, initials and subsequence matching share the existing Rust index and typed result boundary. Exact alias/ID priority is preserved even when a lower relevance match was used recently; exact titles also precede title prefixes. Tests cover these cases, Unicode/NUL parity, bounded metadata/retention/reset and hidden host execution proving only successful launcher selections train recency. Disabled and removed commands remain absent.

The model benchmark retains the original catalog/query matrix and adds a separate 10,000-command search with 512 recent choices. The original P2 figures compare 0.7.13 against 0.7.14; they must not be presented as a speed claim for the later, broader matching algorithm. Physical hotkey-to-display acceptance remains separate from these software fixtures.


## 0.7.16: correct the live measurement boundary

The visible acceptance attempt exposed a measurement gap: input.model recorded events while the NSHostingView draw override remained unused. The original 0.7.13–0.7.15 `input.draw`/`native.draw` counters therefore do not prove a live draw or result-presentation boundary. Schema 2 replaces them with the precisely limited stages above. The input endpoint now runs after the actual field binding is applied in updateNSView; it is independent of offscreen captures. Field teardown cancels pending work before an object address can be reused.

The launcher window was observed through accessibility, but the computer-use bridge repeatedly timed out or reported no available window. Visible typing/arrows/close acceptance is still open; no desktop keys were posted by the agent. This is separate from the successful hidden delegate/reconciliation tests and model/lifecycle budgets.


### Clipboard measurement correction (0.8.2)

The original completion wait inspected stored properties through Mirror. In 0.8.1, clipboardSearchBusy became a computed forwarding property, so that wait no longer waited. Earlier `clipboardSearch200x8kB` results for that architecture must not be interpreted as completed-filter latency. The independent `clipboardSearchEnqueue200x8kB` metric remains valid. Measurement version2 reads the state directly and fails if publication exceeds five seconds. Do not compare completed-filter results across these harness versions; retain enqueue comparisons separately.

## Size and idle cost experiments (0.8.3)

The release decision compares ordinary and trained search tails, executable bytes, new-process readiness, real CLI status/search, and headless CPU/wakeups/footprint/I/O. Exact compiler arguments, toolchains, environment overrides and prepared executable hashes are recorded in the build receipt. Source-build artifacts and locally signed installed artifacts are compared separately.

The initial matrix used two reversed-order runs per variant with the same corrected completion harness: default optimization, symbol stripping, Swift-wide `-Osize`, Rust thin LTO, and their combination. Removing debug and local symbol entries reduced the unchanged 0.8.2 host from 8,010,296 to 3,867,992 bytes while all 37 backed code/data sections stayed identical. The original executable and matching dSYM are retained privately for diagnostics.

Swift-wide size optimization was rejected: ordinary 10k-command model p95 increased from 2.2195415 to 2.4931665ms in that exploratory matrix. The selected arrangement keeps portable models/core speed-optimized and makes only the native UI target size-optimized. Pure empty-query projection preserves stable usage-count ties, filters/limits and catalog order. An empty query can retain the immutable catalog directly; with no usage history, projection skips sorting. Rust thin LTO uses one codegen unit and explicitly keeps unwind/panic containment.

The release preparation handles independently cached Cargo/SwiftPM outputs per artifact, preserving the original diagnostics of an unchanged executable. A durable preparation intent restores an interrupted pair before compilation; known hashes, matching UUIDs/dSYM and code/data-section parity are required. Real Mach-O fixture tests cover only-host/only-CLI relinks, second-replacement failure, rollback/retry and recovery without the in-memory handler. Build and install share the exclusive artifact lock.

Final confirmation uses three alternating model runs per variant. Host observations use ABBA order, 30 new-process starts and 1000 actual CLI samples per run, plus explicitly bounded 30-second active/idle windows. File caches are not flushed: process-cold readiness is not a disk-cold claim. The headless fixture has 500 commands and 200 clipboard records, no visible UI, global input, app discovery or general pasteboard access. Short counter windows do not establish long-session idle behavior or physical energy use. See the [0.8.3 release measurements](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.3) for final results and tradeoffs.


## Remaining search and CLI costs (0.8.4)

The immutable Rust index prepares a stable numeric title/catalog order once. Queries compare this order only after relevance and recency tie, and determine the lowercased Unicode scalar-length threshold once rather than for every entry. The query sorter remains stable. Exactly equal titles retain catalog order; recency still precedes title order. No match classes, scope, limits or UI input handling changed.

Six alternating frozen release runs (baseline, candidate, candidate, baseline, baseline, candidate), 1,000 observations per catalog/run on the same Apple M4, produced these medians of per-run model p95. This is the complete Swift/Rust model fixture, separate from the narrower kernel experiment.

| Commands | 0.8.3 p95 | 0.8.4 p95 | Reduction |
| --- | --- | --- | --- |
| 500 | 0.068625 ms | 0.066666 ms | 2.9% |
| 1,000 | 0.124708 ms | 0.118291 ms | 5.1% |
| 5,000 | 0.603083 ms | 0.566375 ms | 6.1% |
| 10,000 | 1.205500 ms | 1.122500 ms | 6.9% |

The 10k per-run p95 ranges were 1.189166–1.257791 ms and 1.122500–1.131208 ms. With 512 recent choices, median per-run p95 fell from 0.566334 to 0.483250 ms (14.7%). The smallest catalogs overlap between runs, so the small deltas there have weaker evidence. All six fixture checksums matched. A separate differential fixture compared 1,200 complete ordered results across 12 catalogs, 600 entries each, four recency profiles and 25 Unicode/whitespace/alias/keyword queries. Permanent checks cover large title ties, recency, exact-alias precedence and Unicode lowercase expansion.

The tradeoff is one native word per indexed entry: 8 bytes on supported 64-bit Macs, or 80,000 bytes at 10k, plus temporary construction sorting storage. Forty alternating kernel-construction observations per variant gave median baseline/candidate 0.154854/0.164187 ms at 343 entries and 4.504500/4.898375 ms at 10k. This is a cold catalog/update cost, not a cost on every keystroke, and not a claim about whole-process resident memory.

Only the CLI adapter is built with `cargo rustc --bin ruycast -- -C opt-level=s`; the shared library retains the ordinary speed profile, thin LTO, one codegen unit and unwind containment. Prepared executables total 4,153,168 → 4,125,328 bytes (27,840 bytes / 0.67% smaller): host 3,379,608 → 3,396,408; CLI 773,560 → 728,920. These are source-build bytes before installation signing, with behavioral check code included.

Rejected experiments: replacing the hot stable sort with an unstable sort and explicit index tiebreaker increased host bytes and did not improve the full model's 10k tail. Optimizing the entire root Rust package for size made the selected kernel p95 0.458521 → 0.626979 ms (36.7% slower, six alternating runs); the CLI-only boundary avoids that shared-kernel regression. No new dependency or runtime cache was added.

Control-server startup now commits lock/socket ownership only after bind and listen succeed. Failure closes local descriptors and releases the instance lock; it does not leave a half-started object that requires stop before retry. Real Unix-socket checks cover lock contention, preservation of the active owner's socket, a bind failure on an occupied directory path, and subsequent same-instance recovery without explicit stop. The targeted fixture fails against the old code and passes against the candidate. Arbitrary preexisting-path handling and active-client cancellation are unchanged; this does not claim every possible OS socket failure was injected.

Four ABBA headless runs compared the frozen 0.8.3 pair with the selected 0.8.4 host and compact CLI. Each run used 30 fresh process starts, 1,000 real status subprocesses, 1,000 real search subprocesses, 15 active seconds and 30 idle seconds. Cold-ready p95 ranges overlapped: baseline 139.820–141.153 ms, candidate 135.551–141.904 ms. Status p95 ranged 10.177–12.674 versus 12.751–16.093 ms; search p95 2.243–2.905 versus 2.571–3.053 ms, all below the existing 50 ms budget. These noisy ranges do not establish a universal CLI speedup. A same-host, alternating-per-call CLI-only comparison separately checks the compiler boundary.

Idle interrupt wakeups stayed about 0.300/s in both variants. Idle CPU ranged 0.005–0.014% versus 0.008–0.009% of one core. End footprint was 14.860–15.360 versus 15.594–16.454 MiB; no resident-memory saving is claimed. Short windows, normal OS caches and process-cold readiness are not long-session idle, disk-cold, physical presentation or energy measurements. No global keys, visible window, general pasteboard, personal configuration or real process termination were used by these fixtures.

The final compiler-only check kept one canonical candidate host running and alternated speed/size CLI order on every call, with 1,000 retained observations per operation/variant. Status median/p95 was 9.409979/10.580250 ms versus 9.413646/10.652917 ms; search 2.118313/2.386750 versus 2.122542/2.378084 ms. These differences are within ordinary variation, supporting the size boundary without a CLI latency improvement claim. Canonical status maxima were 18.641/28.549 ms. An earlier prototype check observed 59.771/95.159 ms maxima, so p95 compliance is not a guarantee that every call stays below 50 ms.

The frozen ABBA CLI prototype and canonical build have identical hashes for all 14 backed Mach-O code/data sections. Their linker/signing metadata and UUID differ; the canonical executable is 8 bytes smaller. Full-model observations use the exact canonical host; the final compiler-only CLI comparison uses the exact canonical CLI. The prototype and canonical hashes remain separately recorded, and the source-build byte totals above use canonical filenames.

Final local validation passed Rust and Swift core/concurrency checks and 85 Python tests (one optional private-adapter test skipped in public source). The release-native 10,000-cycle gate retained +0.8125 MiB, with every required checkpoint below the 10 MiB ceiling. Hidden Settings checks rendered 20 light/dark states, preserved configuration/routing/race invariants, and confirmed no visible window or desktop input. These fixtures establish their bounded regression coverage, not live physical display latency or all possible OS failures.

## Build/check artifact ownership (0.8.5)

Public build, test, Settings and performance entrypoints now hold one checkout-local artifact lease through their final synchronous consumer. Previously, compilation released the lease before tests used the debug executable; a concurrent build could remove it. The regression fixture reproduces this failure against the old scripts. The retained descriptor also protects live consumers after SIGKILL or SIGINT of the Python controller. A failed build does not start consumers and releases its lease. Every previous Rust, Swift, Python and native budget check remains in place.

Successful pipeline receipts record separate stage durations, lock-wait time, input hashes and consumed executable/module hashes. A source/test/export-input or artifact edit invalidates revalidation; a failed new attempt removes the previous pass. This is a local consistency check, not a tamper-proof attestation. Settings checks now build the current debug host first rather than assuming an existing binary. Internal locked helpers are implementation entrypoints; use the public wrappers.

One fresh-output release build per variant in the same disposable extracted 0.8.4 source took 83.08 seconds before and 83.92 seconds with the candidate orchestration. Three subsequent cached builds took 3.434/1.697/1.663 seconds before and 1.724/1.558/1.514 seconds after. Toolchain/OS caches were not flushed. The candidate measurement preceded the final expanded receipt-input coverage and version-only 0.8.5 literals. These observations describe build cost; they do not establish a relative speedup. The accepted benefit is avoiding invalid/racy checks and repeated recovery work.

Eight disposable subprocess tests cover competing builds, live consumers surviving controller crash/interruption, failed builds, edits during checks, stale receipts after source/module/binary changes, failed-attempt cleanup, and all three verification wrappers. These are real kernel leases with stub compilers; full native integration remains separately required. A lease serializes one checkout only. Concurrent work in other checkouts can still distort CPU measurements.

## Recurring clipboard attribution work (0.8.6)

The ordinary clipboard timer remains at0.75seconds. When its pasteboard count is unchanged and no app-activation uncertainty needs settling, it now returns before resolving the foreground application. Activation still forces an ordinary settling poll; a changed payload still validates its source and count after the snapshot. Paused, disabled and in-flight capture, concealed/transient/generated types, excluded apps and a source switch during capture retain their guards. A missed activation notification remains conservative: a different observed PID discards that changed payload rather than assigning it to the wrong app.

The named-pasteboard regression first failed against the extracted original polling body at1,000 unchanged polls. The candidate covers stable capture, activation with and without a new payload, resume after settling, pasteboard mutation during attribution followed by the next successful poll, source mutation during snapshot, excluded/concealed content and polling while a prior capture is still in flight. Fixtures install no global keyboard hooks, read no general clipboard, and open no visible window.

Six frozen release runs alternated baseline, candidate, candidate, baseline, baseline, candidate on the same M4. The baseline keeps the original poll body behind the same named-pasteboard/provider seam and benchmark driver. These are tight-loop cost observations, with three runs per variant; they do not measure native idle sessions or physical energy.

| Workload | Original app lookups | Candidate app lookups | Original median CPU | Candidate median CPU |
| --- | --- | --- | --- | --- |
| 10,000 unchanged polls | 10,000 | 0 | 7.325 ms | 6.377 ms |
| 1,000 polls, changed every fourth poll | 1,250 | 500 | 166.722 ms | 167.196 ms |

The unchanged difference is only0.0948microseconds per poll. Original CPU ranges were7.255–9.245ms; candidate6.337–7.042ms. Active ranges overlap (164.386–170.755ms versus164.465–167.401ms), so no active CPU improvement is established. Foreground-app property calls are not proven IPC requests. No whole-app CPU, wakeup, energy or battery saving follows from eliminating them.

The cost driver is opt-in at compile time. The new regression body is compiled in debug builds and in this explicit benchmark build; ordinary releases keep the original pipeline checks and omit the new regression result fields. The task requires debug Settings checks as well as ordinary integration checks. Production polling logic is identical in all configurations:

```sh
bash scripts/build-native.sh -c release -Xswiftc -DRUYCAST_POLL_BENCHMARK
.build/swift/release/RuycastApp --benchmark-clipboard-poll 10000
.build/swift/release/RuycastApp --benchmark-clipboard-poll 1000 active
```

An ordinary build rejects that fixture flag before starting the app. The opt-in fixture rejects an ambient `RUYCAST_CONFIG_DIR` before creating state, keeping the fixture profile isolated. Rebuild without the define for installation. Keep frozen copies for alternating experiments. The original prototype's source and executable hashes remain separate from the default0.8.6 candidate; the production poll logic is unchanged after the six-run experiment.

The recurring-work inventory retained the5-second configuration metadata fallback with500ms leeway,2-second keyboard/snippet recovery checks,60-second history/assets maintenance, the active-only2-second monitor, and coalesced event-driven analytics/log writers. Slowing capture or removing recovery checks was rejected. Physical power sampling requires unavailable administrator credentials in this session; CPU, wakeups and I/O remain separate observed metrics, with no energy claim.

Four additional frozen headless runs used ABBA order, the same CLI,500 commands/200 text fixtures,10 new-process starts,200 observations per CLI operation,15seconds active and60seconds idle. This host mode does not run the native clipboard poll; it checks the common background path rather than proving the targeted optimization's benefit. Active CPU ranges overlapped:18.04–18.92% baseline and17.95–20.01% candidate of one core. Idle CPU likewise overlapped:0.093–0.127% and0.091–0.119%. Interrupt wakeups stayed about0.267/s in all four runs; each idle window wrote8,192 disk bytes. One first baseline read69,632 bytes; the others read0, with ordinary OS caches left intact. No I/O reduction is claimed.

New-process RPC readiness p95 ranged133.4–142.6ms versus136.1–138.2ms. Status p95 ranged14.29–19.91ms versus12.21–12.79ms; search2.91–3.34ms across all runs. These short noisy host observations do not establish a causal CLI speedup; the native polling gate is not executed in headless mode.200samples are insufficient for the fixture's p99 reporting. Individual status maxima can exceed50ms even while the existing p95 budget passes.

A separate600-second passive observation of installed0.8.5build45 used121 OS counter samples:0.1016% of one CPU core,2.076 interrupt wakeups/s,54.24→54.61MiB footprint,2,113,536 disk bytes read and65,536 written. The PID and installed executable remained unchanged. Ordinary user and development activity was allowed, so this is contextual baseline evidence, not an enforced idle or paired causal energy comparison.

The diagnostic split was measured before acceptance. Prepared source-build host bytes are3,396,408 for0.8.5,3,412,952 for the0.8.6opt-in benchmark build, and3,396,424 for the ordinary0.8.6build: the ordinary host is16bytes larger, while the optional diagnostic body adds16,528bytes. The first unsplit ordinary prototype was3,412,856bytes and was rejected as the default. These are executable file bytes before local installation signing, not resident-memory measurements. The ordinary binary rejects the cost-driver flag without starting the app; the opt-in binary runs the new regression guards and rejects an external config-directory override. All old release pipeline checks run in the ordinary binary; new guard result fields appear only when those guards actually ran.

## 0.8.9: bounded typo recovery

Search keeps its existing relevance tiers and full ordering. Only when these rules return no matches, one alphanumeric query term of4–64 Unicode scalars may match a title word within one insertion, deletion, substitution or adjacent transposition. Exact IDs, aliases, arguments, disabled filtering and all ordinary matches retain their existing behavior. A weak existing match also suppresses typo recovery. No new index field, package or query history is added.

A pre-candidate26-intent synthetic fixture improves from19/26 intended top results to26/26; seven previously absent typo queries are recovered. An extended set of38 cases passes, including near negatives. These expected intents are based on public command titles; they do not measure a personal query distribution or daily typing speed. Native Core checks cover short/long bounds, Unicode/combining marks, ambiguity, exact resolution and recency; the native search check also covers disabled filtering and CLI/native parity. An independent generated edit-neighborhood test compares14,641 ordered pairs.

Six frozen release-core runs in BCCBBC order use170 synthetic/public-default commands,2329 queries and two recency snapshots. All4438 previously nonempty complete ordered rows among4658 rows remain identical.192 previously empty rows gain suggestions,28 remain empty; all six runs are deterministic. At343 repeated-fixture commands, median-of-run-medians for `fnider` is0.011583→0.014625ms and an unmatched `zzzzzz`0.009542→0.012375ms. At10000 commands these are0.327916→0.417792ms and0.273916→0.368083ms. Each query/size/run retains200 observations. This additional scan is a measured cost, approximately0.003ms at343 commands/~0.09ms at10k, not a latency improvement. Ordinary `finder` medians are0.011291→0.011417ms and0.324208→0.325125ms; empty-query differences are within0.000042ms. These are kernel/model observations, excluding physical input and display.

Six whole-app release model runs keep equal checksums. Median p95 at10000 is1.151792→1.160166ms (ranges1.130125–1.157959 vs1.152333–1.162750); with512 recent choices,0.483500→0.523583ms (ranges0.482541–0.494875 vs0.479542–0.532042). These figures do not support a speedup. An additional nine-run baseline/A/B comparison tests isolating miss-only code out of line; its p95 ranges overlap widely, including baseline recent-choice0.549–1.525583ms, A0.480250–2.112209ms and B0.513292–0.680958ms. No causal advantage is established; the extra helper/annotation is discarded. The simpler candidate remains. Every run passes existing model checks; this variability is retained, not hidden by selecting a favorable run.

Prepared ordinary source-build host size is3414664→3414696bytes (+32 bytes); CLI file size remains729080 bytes. File bytes, model latency and resident memory are distinct measures. Final native lifecycle, source review, publication and installation remain separate acceptance gates. No physical hotkey, energy or battery improvement is inferred.
