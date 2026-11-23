# Public source review

The public source contains a neutral configuration and a separate public Git history. The launcher, Settings and documentation images use a disposable demonstration profile. No personal configuration, clipboard contents, customer information or signing identity is included.

## Privacy boundary

The public source is selected by the Rust `ruycast-source export` command and its explicit allowlist. Development workflow state, private adapters, build output, personal profiles and private git-history are forbidden in that export. The original public screenshots were visually inspected: they show demonstration commands and native views, with no personal paths or private data. Configuration and runtime folders are outside this repository.

The config-directory fixtures in `rust-native/tests/startup_config_repair.rs` mentioning a synced vault describe isolated tests; they contain no real user configuration. References to local signing and mocked credential patterns test security behavior, and are not keys or tokens. These are reviewed public-safe test and implementation references, not an exemption for credentials. Automated secret scans, sensitive filename checks and the clean git-history check are required; any new false positive needs review against actual content.

## Verified release boundary

The exported source builds and is tested with a disposable local profile. Native visual and interaction checks use hidden windows and do not send global keyboard events. No GitHub Actions are required. Public notarized distribution is deferred. Intel installation remains unverified. Physical display latency and long-duration memory acceptance remain documented measurement gaps in [Performance](PERFORMANCE.md).
