# ADR 0001 — Native modular core

Date: 27 October 2026. Status: accepted direction after initial cost experiments; implementation and integrated acceptance remain open.
Issue: https://github.com/viggomeesters/ruycast/issues/33.

## Problem and evidence

Ruycast0.7.16 is already a native Swift host with a statically linked Rust domain library and Rust CLI. The accepted performance work reduced model search cost by roughly34–36% versus0.7.13. This is software evidence, not physical display latency. A language rewrite has no measured speed advantage.

The current host concentrates catalog/search, clipboard coordination, configuration lifecycle, command routing and UI interaction in `AppState` (581 lines,29 published properties; inventory must be read against the pinned baseline). Views subscribe to the entire object. C symbols are declared manually with `@_silgen_name`; a signature can diverge from Rust without a shared C declaration. Compile-time architecture and concurrency ownership deserve stronger guarantees.

### Reference findings

- [Tinycast architecture](https://github.com/abue-ammar/tinycast/blob/main/docs/architecture.md), blob328b1caaaa3b8c4b45cbab7cdd37ac1af5659130: separates deterministic decisions, OS effects, feature state and views; uses property observation and explicit concurrency isolation. This informs boundaries; its implementation is not copied.
- [RustCast dependencies](https://github.com/MystikoLab/rustcast/blob/master/Cargo.toml), bloba1a3eb27b64f6b68334a66e9200b82bb03c5aacf: iced plus native Objective-C bindings. Its [README](https://github.com/MystikoLab/rustcast) reports retirement. This proves a Rust UI is possible, not that it is faster or why maintenance stopped.
- [Vicinae build](https://github.com/vicinaehq/vicinae/blob/main/CMakeLists.txt), blob19eaec1546fa2c57bc384adcd60ead9f9d0597ca: Qt/QML and optional TypeScript extension runtime. Its [scorer](https://github.com/vicinaehq/vicinae/blob/main/src/server/src/internal/fuzzy-scorer.hpp), blob177edabfee98776626205a693702b92a5b70667d, reuses storage and parallelizes large catalogs. Scope and deployment complexity differ; their threshold is not our budget.
- [Apple SwiftUI performance](https://developer.apple.com/documentation/Xcode/understanding-and-improving-swiftui-performance) recommends profiling view work and narrowing dependencies; Observation tracks accessed properties. This is a mechanism, not evidence of our frame-rate gain.
- [Swift migration guide](https://www.swift.org/migration/) supports incremental per-target concurrency migration. [Concurrency](https://docs.swift.org/swift-book/LanguageGuide/Concurrency.html) distinguishes actor isolation from simply starting asynchronous work; suspension can invalidate assumptions.
- [Rust FFI guidance](https://doc.rust-lang.org/nomicon/ffi.html) requires explicit ownership, layout and unwind rules. Process isolation and language safety do not repair a mismatched ABI.

Sources inspected27 October2026. External apps were neither installed nor benchmarked against Ruycast. Source blobs identify the actual reviewed documents; mutable URLs may advance.

The installed toolchain is Swift6.2.3, while the package selects tools/language compatibility5.9 and macOS14+. A complete-concurrency Swift5 build passes. An isolated Swift6-language probe fails with 41 unique source diagnostics across shared mutable services, main-isolated callbacks and transferred non-Sendable state. These are migration findings, not a claim of 41 observed runtime races. Incremental compiler isolation is therefore part of the migration rather than an unchecked flag change.

## Options

| Option | Benefit | Cost / evidence limit | Disposition |
| --- | --- | --- | --- |
| Native Swift/AppKit + static Rust | Retains working platform behavior and shared Rust CLI rules; avoids IPC on typing | Two languages require a checked ABI and clear ownership | Preferred; implement stronger boundaries |
| All Swift | Removes FFI and one toolchain | Rewrites working domain/CLI behavior; no measured speed benefit | Reject for this migration |
| Rust UI (iced/other) | Can share more types with the CLI | Rebuilds text input, focus, accessibility, windows and visual acceptance; no measured advantage | Reject without a concrete unmet platform/performance need |
| Always-on backend daemon | Fault/process separation and independent clients | IPC, supervision, cancellation, permissions and version compatibility; already have a bounded private control socket | Reject for normal command path; helper processes remain appropriate for heavy/untrusted effects |

## Performance mandate — 27 October amendment

The maintainer explicitly asks for F1-style attention to even small costs: fastest, smallest, most stable and best practical app. Therefore this preferred architecture remains falsifiable. Compare costs on production-shaped workloads, not language shares or maintenance arguments alone. Include tail latency, per-keystroke allocations/copies, avoidable view invalidation, main-thread I/O, polling/wakeups, executable size, retained memory and fault recovery. A change must not trade away matching correctness, accessibility, determinism or robustness for one attractive metric.

First experiments: decompose the existing typed search boundary and test reusable caller buffers against identical Rust ranking/output; investigate native row rendering against existing SwiftUI only if UI evidence makes it material. Fair paired optimized builds and parity checks are required. No prototype result may be relabeled physical latency. Full UI replacement remains available if measured gains justify its native-behavior migration and maintenance costs. Final acceptance must compare the selected implementation with the frozen0.7.16 baseline and explain rejected marginal optimizations.

## Cost experiment and selection

A paired optimized Swift prototype uses the actual0.7.16 Rust library and production `CommandSearchIndex` as control. Three variants separate output-buffer reuse from pre-aligned recency snapshots. Two independent process runs each alternate six rounds for343/5000/10000 commands;102 equality comparisons per size/run cover populated, empty and changed recency, Unicode and embedded NUL queries.

At343 commands, median run-p95 is0.0461→0.0272ms in the first run and0.0435→0.0254ms in the repeat when recency weights are aligned only on updates. At5000 commands it is0.6919→0.4170ms and0.7014→0.4086ms. Output-buffer reuse alone is neutral. These numbers describe this synthetic model workload; they are not total application or physical-screen speed gains. This prototype keeps Rust ranking/allocations and command materialization unchanged. Production migration must invalidate weights on every catalog/recency change and verify that path.

Decision: retain the native/static-kernel architecture, reduce the measured recurring Swift-side preparation first, and enforce ownership/observation/isolation in production. The evidence supports this reversible design and identifies one cost; it does not prove a mathematically optimal UI toolkit. Additional expensive-path experiments and rejected optimizations belong in the implementation cost ledger. A native row renderer remains a valid follow-up within this goal if actual UI measurements establish material benefit. No all-Swift/all-Rust speed hierarchy is claimed without equivalent implementations.

## Selected target ownership

1. Rust kernel: deterministic validation, shortcut grammar, indexed matching/ranking, retention and geometry; split by responsibility without changing behavior. No OS UI or profile reads in deterministic functions.
2. C ABI: checked header/module, explicit UTF-8 lengths, capacity/result/error contracts, owned handles and panic containment. Swift owns its buffers; handles have one confined owner. Keep JSON on cold control/config boundaries when appropriate.
3. Swift model: portable command DTOs, templates and input state; environment facts passed explicitly. Model code does not import AppKit/SwiftUI. Runtime file operations must have a distinct owner.
4. Native runtime: macOS adapters and bounded workers; one composition root owns lifecycle and teardown. Main isolation owns AppKit and observable presentation; expensive blocking work uses explicit workers. No actor-per-object ceremony.
5. Features: separate catalog/search, palette/session, clipboard, configuration and command orchestration ownership. Property-level observation avoids unrelated feature invalidation. Views render and forward user intents; CLI and GUI reach the same command routing authority.
6. Persistence: one serialized owner per mutable resource; immutable snapshots across queues, latest-request generation checks, bounded pending work and explicit durability/error handling. Keep versioned readable config and private local runtime files. Do not introduce SQLite or a helper process without a concrete requirement.

## Migration and acceptance

Expand checked boundaries, migrate actual shipped callers, then remove obsolete coupling. Every intermediate release must build and preserve the public CLI/config/wire behavior. Keep panel hosting identity, exact aliases, disabled-command guards, snippet focus/security, confirmations, process identity and rollback semantics.

Acceptance requires compiler/behavioral checks for the new boundaries, local Rust/Swift/CLI regression gates, search quality, interaction reconciliation and10000-cycle footprint budgets, reviewed public PRs, source release/checksums and tested installation with unchanged profile/signing/permissions. Physical compositor timing and battery claims require separate measurements; fixture timings never imply them.

The architecture is implemented only when the boundaries govern production paths and regression checks prove their behavior. Renaming folders, wrapper classes or an ADR alone do not fulfill Issue33.
