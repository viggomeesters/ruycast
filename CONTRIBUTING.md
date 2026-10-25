# Contributing

Ruycast combines a Rust domain library and CLI with a native SwiftUI/AppKit host. Start with the [architecture](docs/ARCHITECTURE.md) and [product contract](docs/vision.json).

## Build and check

Install the [build requirements](docs/INSTALLATION.md), then run:

```sh
make check
bash scripts/check-settings.sh
```

Run `bash scripts/check-performance.sh` when changing search, clipboard queues, icon work or lifecycle behavior. Use an isolated `RUYCAST_HOME` profile for experiments. Build output lives in `.build`; runtime profiles and personal data do not belong in Git.

## Scope and evidence

Keep each change focused on a user-visible outcome. Describe the trigger, resulting behavior and relevant validation in your pull request. Preserve CLI/configuration compatibility, atomic writes, permission boundaries and clear error feedback.

Use named pasteboards, hidden native views and fake process effects in tests. Never send global keys, move the contributor's windows or kill real processes as part of an automated check. Measure release builds and label model/offscreen timing separately from physical display latency.

All validation runs locally; this project does not use GitHub Actions.

## Public source

Keep credentials, exports, clipboard contents, personal integrations, local workflow evidence and private screenshots out of shared source and releases. Review the [security policy](SECURITY.md) before sharing diagnostics. Preserve third-party licenses and identify implementation references; do not copy code under an incompatible license.

## Issues and pull requests

Report reproducible bugs or suggest features through [GitHub Issues](https://github.com/viggomeesters/ruycast/issues). Include macOS version, source tag and the smallest reproducible example. Review logs for private details before sharing; do not attach personal profiles or clipboard history.

Fork the repository or create a focused branch, make the change and open a pull request against `main`. Describe the behavior before and after, and the local checks you ran. The maintainer reviews and merges changes; `main` is the stable source branch. For UI changes, include public-safe native screenshots. Use a separate configuration/runtime profile for tests and preserve signing and existing user data.
