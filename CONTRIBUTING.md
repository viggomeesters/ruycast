# Contributing

Saved native script commands can opt into invocation provenance by putting the literal `{invocation-provenance}` first in `arguments`. It is not passed to the script. The supervisor gives the script a `VIGGOBSIDIAN_INVOCATION_ID` UUID and `VIGGOBSIDIAN_INVOCATION_AT` UTC timestamp sampled at admission. In this mode any `{uuid}` argument uses that same UUID. Literal input cannot enable the mode. Persist both values if the script supports recovery; running the command again is a new action, not a replay.

Ruycast's production host, domain library, CLI and delivery policy are Rust. The native host uses AppKit through Rust bindings. The former Swift package and its test oracles are available in Git history. Current tests use Rust and pinned historical results. Start with the [architecture](docs/ARCHITECTURE.md), [product contract](docs/vision.json) and the [current source beta](https://github.com/viggomeesters/ruycast/releases/tag/v0.9.0-beta.17).

## Build and check

Install the [build requirements](docs/INSTALLATION.md). For a focused change, run its affected Rust test target and case with `cargo test`; use the existing test target's command in `rust-native/src/bin/ruycast-build/validation.rs` when it has a native fixture or timeout wrapper. Check disk headroom before producer builds:

```sh
cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- disk-status --repo "$PWD"
```

The full Swift-comparison/F1 campaign ended on 10 November 2026 and is not a contribution or release gate. The focused Rust measurements in [Performance](docs/PERFORMANCE.md) do not substitute for functional tests or physical desktop acceptance. Use an explicitly isolated `RUYCAST_HOME` profile for experiments. Build output lives in `.build`; runtime profiles and personal data do not belong in Git.

Builds, validation, packaging and installation share `.build/artifacts.lock` through the final synchronous consumer. Nested Rust tools inherit a descriptor plus checked root/device/inode/owner metadata. Concurrent entrypoints wait; a controller crash does not release a live child's lease. Never unlink the lock file. Invoke the locked native build directly with `cargo run --quiet --locked --offline --target-dir .build/delivery-tools --manifest-path rust-native/Cargo.toml --bin ruycast-build -- compat build-receipt --repo "$(pwd -P)" -- build`; add `--configuration release` after `build` when needed.

## Build artifact cleanup

Check free disk space and the size of project-owned build/cache directories before expensive builds and after each completed issue or task. Once its consumers and required evidence are finished, remove task-owned temporary exports, fixtures, failed partial outputs and reproducible build caches. Repeat the check when generated output grows materially or disk space becomes tight.

For focused Rust tests and crate builds, set `CARGO_INCREMENTAL=0` and reuse the existing `rust-native/target` directory; do not create a target directory per issue. Build-tool routes use the existing `.build/delivery-tools` directory. Keep outputs in `rust-native/target` when their tests or evidence readers name that path. Run `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- clean-build-caches --repo "$PWD"` to remove only named Cargo caches in this checkout while holding its artifact lease. It retains top-level binaries, reports, receipts and lock files. It can remove compiled test executables under `deps`, so run it only after their consumers and required evidence are finished; verify its reported removed directories and actual free space afterward.

Before large producer builds, run the direct Cargo `disk-status --repo "$PWD"` command shown above. It measures available bytes with `statvfs` and reports the 50,000,000,000-byte floor plus each operation's estimated peak. Product builds refuse before touching their receipt or outputs when headroom is below the floor plus the estimated peak; full native validation uses an 8 GB peak estimate, and the combined check/settings pipeline uses 11 GB before it removes its prior receipt or builds. The initial `cargo run` bootstrap for the build tool is small but happens before its Rust command can perform this check.

The `clean-build-caches --repo "$PWD"` Cargo route acquires this checkout's exclusive artifact lease and waits for cooperating consumers to finish. It removes only Cargo `build`, `deps`, `incremental` and `.fingerprint` directories inside the named Ruycast build target roots. It does not remove receipts, release symbols, preparation/recovery records, verification reports, SDKs, screenshots, lock files or worktree edits. Run the direct `disk-status --repo "$PWD"` Cargo command and inspect `git status --short` after cleanup. Run the command separately in each Ruycast worktree whose caches should be retired; each checkout has its own lease. Direct Cargo processes that bypass the repository build routes do not participate in the lease and must be stopped before cleanup.

The direct `ruycast-build compat build-receipt --repo "$PWD" -- check` command runs the check pipeline. Its source inventory now qualifies the actual empty Python unittest tree and the current `git-diff-check` source. This source qualification does not substitute for running the full pipeline, which remains subject to the disk headroom preflight. The separate `settings` inventory source-qualifies all 16 selector gates; that result does not mean the full Settings matrix ran or grant production acceptance. The `verify-*` entrypoints revalidate corresponding receipts; an inventory or native-only pass cannot replace full validation. Inspect the IDs with `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- gate-inventory --repo "$PWD" --pipeline check` (or `settings`). No historical gate is silently skipped.

For a focused change, run the affected Rust test targets with bounded child-process timeouts. Reuse evidence only while its relevant sources, dependencies, toolchain and artifacts are unchanged. Related migration tasks may share one reviewed PR and source export; rerun broader integration checks when shared behavior changes and at integration boundaries.

The complete native route runs the ordered leaf matrix, four hidden renders and reference/SDK bindings, including the clipboard-parser, Rust benchmark-driver contracts and temporal-lease cases. Use `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- validation-plan --repo "$PWD"` for the current matrix; its report is `rust-native/target/verification/report.json`. Run this broader route at relevant integration boundaries after checking disk headroom and completing a verified release build:

```sh
cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- disk-status --repo "$PWD"
CARGO_INCREMENTAL=0 cargo run --locked --offline --release --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- build --repo "$PWD"
CARGO_INCREMENTAL=0 cargo run --locked --offline --release --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- check-native --repo "$PWD"
```

The Core gate builds the Rust static library, links it to a Rust ABI probe, and runs the probe against ABI version, JSON call/free, and search-index create/search/free. Its receipt does not establish app UI, startup or desktop acceptance. The standalone pre-main Swift compile/link smoke is retired because it duplicated that check; the pre-main search crosswalk remains source-qualified in the gate inventory. The retired concurrency and task-date helpers map to existing Rust tests in the gate inventory. Core source/artifact receipts are under `.build/receipts`. These checks do not establish full product acceptance, signing or physical desktop behavior.

Production release builds use explicit host-target output paths and v4 receipts. A release receipt requires matching retained originals/dSYM, unchanged backed sections/UUID and the hashed release-preparation record. Debug receipts have `releasePreparation: null` and cannot authorize packaging or installation. V2/v3 build receipts and historical candidate 67 remain historical evidence. Failed release builds invalidate their previous build receipt.

Build, install, version, source export and compatibility check entrypoints use Rust tools. The Core receipt binds the Rust ABI probe and current source policy. The product version comes from `rust/src/product_version.rs`. Use Rust for executable source; keep parity fixtures as data.

The lease descriptor is inherited by synchronous consumers. If the controller is interrupted, a still-running child keeps the lease until it exits; another build can therefore keep waiting after cancellation. Avoid deleting `.build/artifacts.lock` or invoking the internal helpers directly. Separate checkouts have separate leases and can still contend for CPU during measurements.

## Scope and evidence

`ruycast-build compat build-receipt --repo ROOT -- build` preserves its debug default; append `--configuration release` to request release. The native Rust buildtool's `build --repo` keeps its existing release default. Both configurations use explicit target paths; only prepared release authority can enter packaging/install.

Keep each change focused on a user-visible outcome. Describe the trigger, resulting behavior and relevant validation in your pull request. Preserve CLI/configuration compatibility, atomic writes, permission boundaries and clear error feedback.

Use named pasteboards, hidden native views and complete owned SDK/process fixtures in tests. Never send global keys, move the contributor's windows or kill real processes as part of an automated check. Fixtures must replace every selected SDK effect before initialization; a hidden flag alone is not a safety boundary. The validator removes inherited profile/SDK overrides and supplies a fresh owned fixture parent (or an explicitly supplied absolute `RUYCAST_NATIVE_TEST_FIXTURE_DIR`). Measure release builds and label model/offscreen timing separately from physical display latency.

All validation runs locally; this project does not use GitHub Actions.

## Public source

Keep credentials, exports, clipboard contents, personal integrations, local workflow evidence and private screenshots out of shared source and releases. Review the [security policy](SECURITY.md) before sharing diagnostics. Preserve third-party licenses and identify implementation references; do not copy code under an incompatible license.

## Issues and pull requests

Report reproducible bugs or suggest features through [GitHub Issues](https://github.com/viggomeesters/ruycast/issues). Include macOS version, source tag and the smallest reproducible example. Review logs for private details before sharing; do not attach personal profiles or clipboard history.

Fork the repository or create a focused branch, make the change and open a pull request against `main`. Describe the behavior before and after, and the local checks you ran. The maintainer reviews and merges changes; `main` is the stable source branch. For UI changes, include public-safe native screenshots. Use a separate configuration/runtime profile for tests and preserve signing and existing user data.
