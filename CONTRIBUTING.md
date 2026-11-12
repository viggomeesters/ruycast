# Contributing

Ruycast's production host, domain library, CLI and delivery policy are Rust. The native host uses AppKit through Rust bindings. Frozen Swift sources are historical parity oracles, not a production host or fallback. Start with the [architecture](docs/ARCHITECTURE.md) and [product contract](docs/vision.json).

## Build and check

Install the [build requirements](docs/INSTALLATION.md), then run:

```sh
make check
bash scripts/check-settings.sh
```

Use complete F1 evidence when changing search, clipboard queues, icon work or lifecycle behavior. `bash scripts/check-performance.sh` requires an independently reviewed runner/authority and the complete preserved dataset; see [Performance](docs/PERFORMANCE.md). It does not collect data, run the installed app or silently substitute a smaller model benchmark. Use an explicitly isolated `RUYCAST_HOME` profile for experiments. Build output lives in `.build`; runtime profiles and personal data do not belong in Git.

Builds, validation, packaging and installation share `.build/artifacts.lock` through the final synchronous consumer. Nested Rust tools inherit a descriptor plus checked root/device/inode/owner metadata. Concurrent entrypoints wait; a controller crash does not release a live child's lease. Never unlink the lock file. The old `build-native-locked.sh` name is a compatibility dispatch to the same Rust lease, not a separate build recipe.

The full `check` entrypoint still fails closed on the original Python-discovery gate. The `settings` inventory has no unmapped gates, but requires the complete current native validation and qualified settings cases. Their `verify-*` entrypoints revalidate the corresponding receipts; an inventory or native-only pass cannot replace full validation. Inspect the IDs with `cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --bin ruycast-build -- gate-inventory --repo "$PWD" --pipeline check` (or `settings`). No historical gate is silently skipped.

For a focused change, run the affected Rust test targets with bounded child-process timeouts. Reuse evidence only while its relevant sources, dependencies, toolchain and artifacts are unchanged. Related migration tasks may share one reviewed PR and source export; rerun broader integration checks when shared behavior changes and at integration boundaries.

The complete native route runs the ordered leaf matrix, four hidden renders and reference/SDK bindings, including the mandatory clipboard-parser, benchmark-contract and temporal-lease cases. Use `ruycast-build validation-plan --repo "$PWD"` for the current matrix; its report is `rust-native/target/verification/report.json`. Run it after a verified release build:

```sh
make build
cargo run --quiet --locked --offline --target-dir rust-native/target/native-tools --manifest-path rust-native/Cargo.toml --bin ruycast-build -- check-native --repo "$PWD"
```

Three old contracts have actual qualified oracles: LauncherCoreChecks, pre-main search and pre-main concurrency. Their source/artifact receipts are under `.build/receipts`. Full F1 verification has a separate receipt and `verify-performance` revalidates the preserved dataset with the reviewed runner. None is a full check, signature or physical desktop acceptance.

Production release builds use explicit host-target output paths and v4 receipts. A release receipt requires matching retained originals/dSYM, unchanged backed sections/UUID and the hashed release-preparation record. Debug receipts have `releasePreparation: null` and cannot authorize packaging or installation. V2/v3 build receipts and historical candidate 67 remain historical evidence. Failed release builds invalidate their previous build receipt.

Python/shell production entrypoints only forward to Rust. A few Python import functions remain unchanged for historical whitebox tests; they are not production configuration, version, locking, packaging or signing authority. The actual product version is the Rust `rust/src/product_version.rs` literal. New implementation and tests belong in Rust. Existing leaf validators and historical test oracles may be maintained only within their declared boundary.

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
