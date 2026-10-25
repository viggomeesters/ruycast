# Public source review

This source preview contains a neutral configuration and a fresh public Git history. The launcher, Settings and documentation images use a disposable demonstration profile. No personal configuration, clipboard contents, customer information or signing identity is included.

## Privacy boundary

The public source is selected by `scripts/prepare-public.py`. Development workflow state, private adapters, build output, personal profiles and private git-history are forbidden in that export. The three PNG assets under `assets/` were visually inspected: they show demonstration commands and native views, with no personal paths or private data. Configuration and runtime folders are outside this repository.

The filename `Tests/test_config_directory.py` and fixtures mentioning a synced vault describe isolated tests; they contain no real user configuration. References to local signing and mocked credential patterns test security behavior, and are not keys or tokens. These are reviewed public-safe test and implementation references, not an exemption for credentials. Automated secret scans, sensitive filename checks and the clean git-history check are required; any new false positive needs review against actual content.

## Verified release boundary

The exported source builds and is tested with a disposable local profile. Native visual and interaction checks use hidden windows and do not send global keyboard events. No GitHub Actions are required. The release is source-only; distribution notarization and Intel hardware acceptance are not established. Physical display latency and long-duration memory acceptance remain documented measurement gaps in [Performance](PERFORMANCE.md).
