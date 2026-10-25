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

Use `scripts/benchmark-host.py` for an isolated headless profile with500 commands and200 text fixtures. It measures30 process starts,1000 real CLI calls per operation, and active/idle intervals. Arguments `--active-seconds 1800 --idle-seconds 3600` select a90-minute stress run; the shorter600/600-second run is the recorded initial soak. Headless readiness excludes hotkeys, app discovery and visible presentation.

## Measurement contract

Release comparisons use500,1000,5000 and10000 commands, fixed queries/seed,200 approximately8kB clipboard items,1000 synthetic processes, a960k-pixel image and a native Finder icon. Paired baseline/current runs alternate on the same Apple M4 Mac with16GiB RAM, on mains power. Counts and timings are fixtures, not personal usage data.

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

## Initial measured round — 25 October 2026

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

## Background acceptance follow-up — 25 October 2026

Repeating the old 0.7.0 native fixture retained **34.88 MiB** across 10,000 mode/search/action cycles. macOS heap and allocation-history samples identified growing `NSKeyValueDependency` / `NSKeyValueDependencyContext` bookkeeping through AppKit control/view attachment and SwiftUI display-list updates. A custom button style and a replacement text view did not solve the problem and were discarded.

The launcher now creates its native command and clipboard hosting containers once, keeps them attached, and switches visibility. Hidden command typing does not invalidate the clipboard subtree; history changes still reconcile deleted items. Leaving clipboard mode cancels image work, generation tokens reject delayed results, and preview ownership includes the clip identity. Metadata from cancelled or failed decodes is not presented as current.

The final-logic release fixture settled at **+1.52 MiB** over 10,000 cycles (42.74 → 44.25 MiB). Checkpoints at 1,000/5,000/10,000 cycles stayed within the 10 MiB gate. Another earlier persistent-container run settled below its starting footprint; allocator variation is why the gate uses a ceiling rather than requiring negative growth. The fixture also verifies container identity, a hidden panel and 1,000 monitor start/stop cancellation cycles. These are same-data hidden native results, not physical launcher-open timing or a universal leak claim.

An isolated offscreen CPU chart ran for 30 minutes at its ordinary two-second interval, rendering 901 frames. Independent `getrusage` counters measured 0.679% of one core while active and **0.535 percentage points extra CPU** versus the mean of one-minute idle windows before and after. Peak physical footprint was 19.08 MiB; history stayed at 60 samples and no new sample arrived after stop. This does not measure display presentation, battery power or external pasteboard providers.

The 30-minute active / 60-minute idle host soak and final-logic idle observation are still running; their completed results will be recorded before this round is accepted.

Reproduce the bounded native gate with `bash scripts/check-performance.sh`. Longer background probes are opt-in:

```sh
.build/swift/release/RuycastApp --benchmark-monitor 1800 60
python3 scripts/benchmark-host.py --host .build/swift/release/RuycastApp \
  --cli .build/rust/release/ruycast --probe .build/perf-probe \
  --out .build/performance/host-90min.json --active-seconds 1800 --idle-seconds 3600
```

Use frozen copies of binaries for long probes. The host script strips external profile/config overrides, asserts its isolated fixture profile and checks that the host file did not change during measurement. The probe never opens the launcher, installs global input hooks or uses the general clipboard. Do not substitute these numbers for the deliberately excluded physical latency, battery and Raycast practice checks.
