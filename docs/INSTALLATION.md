# Installation

The Rust build and installer are rewrite candidates. Isolated filesystem/process evidence does not establish complete desktop, installation or release acceptance. Developer ID/notarization is deferred. Building or verifying a candidate does not install it.

After the combined candidate is accepted for your use, build and install locally:

```sh
make install
```

`install.sh` forwards `--build` to Rust. `install.py` without that flag consumes an existing verified release build. Production build/install policy is Rust; Python/shell only dispatch. No Swift host/compiler/helper is used for production. The local signer prepares one persistent identity when absent and refuses silent rotation or ad-hoc fallback. The transaction preserves the profile and uses a per-user login service.

## Requirements

- macOS 14 or later.
- Apple's command-line developer tools, including the linker, `dsymutil`, `strip` and `codesign`.
- Rust/Cargo 1.92 or later, with locked dependencies cached for offline builds.
- Python 3.10 or later for compatibility wrappers and existing validators. Explicit historical Swift parity oracles require Swift 6.2; the production build does not.
- OpenSSL for initial local certificate preparation. The signer prefers Homebrew OpenSSL when available and otherwise the system tool.

Development builds have been checked on Apple silicon. This does not establish clean-Mac or physical desktop acceptance of the complete rewrite; Intel execution is untested. Full Xcode is optional for builds; Instruments profiling requires it.

## Locations and configuration

| Item | Default location |
|---|---|
| App | `/Applications/Ruycast.app`, otherwise `~/Applications` when the shared folder is not writable |
| Command | `~/.local/bin/ruycast` |
| Runtime profile | `~/.config/ruycast` |
| Configuration | `config.json` in the profile, or the selected configuration directory |
| Local signing identity | `~/.local/share/ruycast/signing` |
| Login service | `~/Library/LaunchAgents/nl.viggomeesters.ruycast.plist` |
| Update backups | `~/.local/state/ruycast/install-backups` |

Add `~/.local/bin` to PATH if needed. `ruycast status` inspects the running owner; `ruycast quit` stops it and `ruycast start` requests startup. Choose another configuration folder with `ruycast config directory /absolute/folder`. The selected file is shared by GUI and CLI and preserved on updates; runtime history stays in the profile.

`RUYCAST_HOME`, `RUYCAST_CONFIG_DIR` and `RUYCAST_APP_DIR` have their existing `RUSTCAST_*` aliases and priority. Empty explicit roots are refused. Use the same authority for CLI and host. Short profile paths are required by the macOS Unix-socket length limit.

## Installer and signer commands

The Rust installer accepts `install [--build] [--repo ABSOLUTE_ROOT]`. Signer management exposes `prepare`, `signing-status`, and `signing prepare|status|sign`. Explicit `--signing-root` or `--root` selects an isolated signing directory. Status reads the existing pinned authority; it does not silently provision a replacement. Keep signing material outside source exports.

`verify-download --archive ABSOLUTE_FILE --sha256 TRUSTED_SHA256` verifies the supplied archive bytes. A checksum alone is not reviewed source, extraction or installation authority. Archive-to-source binding remains a separate delivery gate.

## Permissions and updates

Enable Accessibility in System Settings → Privacy & Security for window management, text delivery and Hyper. Some keyboard features also require Input Monitoring. Search and clipboard copying work without Accessibility. Ruycast does not edit the macOS privacy database.

Updates retain the certificate/designated requirement and authored configuration. Keep the signing directory; replacing it or switching to Developer ID can require renewed permission approval. The transaction verifies release authority before stopping/replacing the app, holds the same artifact lease through its final consumer, and attempts restoration after a failed update. These contracts still require combined real installation/readback acceptance.

V4 release authority binds explicit native target paths, both prepared binaries and retained matching dSYM/originals under `.build/full-rust-release/symbols/`. Code/data sections and UUID remain identical after stripping. Debug and historical v3 receipts cannot authorize install. Keep local diagnostics for crash investigation; they are not bundled or uploaded.

Local signing supports source builds on your own Mac. It is not Developer ID notarization for public downloads. See [release status](RELEASING.md).
