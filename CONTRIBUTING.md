# Contributing

Ruycast's production host, domain library, CLI and delivery policy are Rust. The native host uses AppKit through Rust bindings. Frozen Swift sources are historical parity oracles, not a production host or fallback. Start with the [architecture](docs/ARCHITECTURE.md), [product contract](docs/vision.json) and the [current source beta](https://github.com/viggomeesters/ruycast/releases/tag/v0.9.0-beta.17).

## Build and check

Install the [build requirements](docs/INSTALLATION.md). For a focused change, run its affected Rust test target and case with `cargo test`; use the existing test target's command in `rust-native/src/bin/ruycast-build/validation.rs` when it has a native fixture or timeout wrapper. Check disk headroom before producer builds:

```sh
make disk-status
```

The full Swift-comparison/F1 campaign ended on 10 November 2026 and is not a contribution or release gate. The focused Rust measurements in [Performance](docs/PERFORMANCE.md) do not substitute for functional tests or physical desktop acceptance. Use an explicitly isolated `RUYCAST_HOME` profile for experiments. Build output lives in `.build`; runtime profiles and personal data do not belong in Git.

Builds, validation, packaging and installation share `.build/artifacts.lock` through the final synchronous consumer. Nested Rust tools inherit a descriptor plus checked root/device/inode/owner metadata. Concurrent entrypoints wait; a controller crash does not release a live child's lease. Never unlink the lock file. The old `build-native-locked.sh` name is a compatibility dispatch to the same Rust lease, not a separate build recipe.

## Build artifact cleanup

Check free disk space and the size of project-owned build/cache directories before expensive builds and after each completed issue or task. Once its consumers and required evidence are finished, remove task-owned temporary exports, fixtures, failed partial outputs and reproducible build caches. Repeat the check when generated output grows materially or disk space becomes tight.

Before large producer builds, check `make disk-status`. It measures available bytes with `statvfs` and reports the 50,000,000,000-byte floor plus each operation's estimated peak. Product builds refuse before touching their receipt or outputs when headroom is below the floor plus the estimated peak; full native validation uses an 8 GB peak estimate, and the combined check/settings pipeline uses 11 GB before it removes its prior receipt or builds. The initial `cargo run` bootstrap for the build tool is small but happens before its Rust command can perform this check.

`make clean-build-caches` acquires this checkout's exclusive artifact lease and waits for cooperating consumers to finish. It removes only Cargo `build`, `deps`, `incremental` and `.fingerprint` directories inside the named Ruycast build target roots. It does not remove receipts, release symbols, preparation/recovery records, verification reports, SDKs, screenshots, lock files or worktree edits. Run `make disk-status` and inspect `git status --short` after cleanup. Run the command separately in each Ruycast worktree whose caches should be retired; each checkout has its own lease. Direct Cargo processes that bypass the repository build routes do not participate in the lease and must be stopped before cleanup.

The legacy `make check` route still fails closed on the original Python-discovery gate; it does not prove Rust completion. The `settings` inventory has no unmapped gates, but requires the complete current native validation and qualified settings cases. Their `verify-*` entrypoints revalidate the corresponding receipts; an inventory or native-only pass cannot replace full validation. Inspect the IDs with `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- gate-inventory --repo "$PWD" --pipeline check` (or `settings`). No historical gate is silently skipped.

For a focused change, run the affected Rust test targets with bounded child-process timeouts. Reuse evidence only while its relevant sources, dependencies, toolchain and artifacts are unchanged. Related migration tasks may share one reviewed PR and source export; rerun broader integration checks when shared behavior changes and at integration boundaries.

The complete native route runs the ordered leaf matrix, four hidden renders and reference/SDK bindings, including the clipboard-parser, Rust benchmark-driver contracts and temporal-lease cases. Use `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- validation-plan --repo "$PWD"` for the current matrix; its report is `rust-native/target/verification/report.json`. Run this broader route at relevant integration boundaries after checking disk headroom and completing a verified release build:

```sh
make disk-status
CARGO_INCREMENTAL=0 cargo run --locked --offline --release --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- build --repo "$PWD"
CARGO_INCREMENTAL=0 cargo run --locked --offline --release --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- check-native --repo "$PWD"
```

Three old contracts have actual qualified oracles: LauncherCoreChecks, pre-main search and pre-main concurrency. Their source/artifact receipts are under `.build/receipts`. These checks do not establish full product acceptance, signing or physical desktop behavior.

Production release builds use explicit host-target output paths and v4 receipts. A release receipt requires matching retained originals/dSYM, unchanged backed sections/UUID and the hashed release-preparation record. Debug receipts have `releasePreparation: null` and cannot authorize packaging or installation. V2/v3 build receipts and historical candidate 67 remain historical evidence. Failed release builds invalidate their previous build receipt.

Build, install, version and source-export routes are available directly through Rust commands; compatibility wrappers and the legacy check pipeline still include Python and shell code. A few Python import functions remain for historical whitebox tests, and the full `check` route still has an unmapped discovery gate. The actual product version is the Rust `rust/src/product_version.rs` literal. New implementation and tests belong in Rust. Existing leaf validators and historical test oracles may be maintained only within their declared boundary.

The lease descriptor is inherited by synchronous consumers. If the controller is interrupted, a still-running child keeps the lease until it exits; another build can therefore keep waiting after cancellation. Avoid deleting `.build/artifacts.lock` or invoking the internal helpers directly. Separate checkouts have separate leases and can still contend for CPU during measurements.

## Scope and evidence

`build-native.sh`/`build_receipt.py build` preserve their debug default; `make build` explicitly requests release. The native Rust buildtool's `build --repo` keeps its existing release default. Both configurations use explicit target paths; only prepared release authority can enter packaging/install.

Keep each change focused on a user-visible outcome. Describe the trigger, resulting behavior and relevant validation in your pull request. Preserve CLI/configuration compatibility, atomic writes, permission boundaries and clear error feedback.

Use named pasteboards, hidden native views and complete owned SDK/process fixtures in tests. Never send global keys, move the contributor's windows or kill real processes as part of an automated check. Fixtures must replace every selected SDK effect before initialization; a hidden flag alone is not a safety boundary. The validator removes inherited profile/SDK overrides and supplies a fresh owned fixture parent (or an explicitly supplied absolute `RUYCAST_NATIVE_TEST_FIXTURE_DIR`). Measure release builds and label model/offscreen timing separately from physical display latency.

All validation runs locally; this project does not use GitHub Actions.

## Public source

Keep credentials, exports, clipboard contents, personal integrations, local workflow evidence and private screenshots out of shared source and releases. Review the [security policy](SECURITY.md) before sharing diagnostics. Preserve third-party licenses and identify implementation references; do not copy code under an incompatible license.

## Issues and pull requests

Report reproducible bugs or suggest features through [GitHub Issues](https://github.com/viggomeesters/ruycast/issues). Include macOS version, source tag and the smallest reproducible example. Review logs for private details before sharing; do not attach personal profiles or clipboard history.

Fork the repository or create a focused branch, make the change and open a pull request against `main`. Describe the behavior before and after, and the local checks you ran. The maintainer reviews and merges changes; `main` is the stable source branch. For UI changes, include public-safe native screenshots. Use a separate configuration/runtime profile for tests and preserve signing and existing user data.
