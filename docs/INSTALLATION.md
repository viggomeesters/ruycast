# Installation

Build and install Ruycast locally:

```sh
make install
```

The build uses Cargo's locked dependencies and Swift Package Manager. The installer creates a persistent local signing identity on the first install, installs the app, preserves any existing profile and registers a per-user login service. Repeated installs reuse that identity.

## Requirements

- macOS 14 or later.
- Apple's command-line developer tools (`xcode-select --install`) and Swift 6 or later (the portable model target uses Swift 6 language mode; verified with Swift 6.2.3).
- Rust/Cargo; development was verified with Rust 1.92.
- Python 3.10 or later.
- OpenSSL for initial local certificate creation. The signing helper uses Homebrew OpenSSL when available and otherwise the system tool.

Apple silicon is the verified build platform. Intel builds have not been tested. Full Xcode is optional for builds; Instruments profiling requires it.

## What is installed

| Item | Default location |
|---|---|
| App | `/Applications/Ruycast.app`, or `~/Applications` when the shared Applications folder is not writable |
| Command | `~/.local/bin/ruycast` |
| Local runtime profile | `~/.config/ruycast` |
| Configuration | `config.json` in the profile by default; change with `ruycast config directory /absolute/folder` |
| Signing identity | `~/.local/share/ruycast/signing` |
| Login service | `~/Library/LaunchAgents/nl.viggomeesters.ruycast.plist` |
| Update backups | `~/.local/state/ruycast/install-backups` |

Add `~/.local/bin` to your shell's PATH if needed. Run `ruycast status` to inspect the running app. `ruycast quit` stops it; `ruycast start` starts it again.

The saved configuration folder is preserved on updates and used by the installed app and CLI. Only `config.json` moves; the installer also backs up and verifies that selected file. Set `RUYCAST_CONFIG_DIR` for an explicit configuration override. Set `RUYCAST_HOME` for a separate runtime profile. The CLI and host must use the same value. Short paths are required because macOS Unix sockets have a path-length limit.

## Permissions

Add Ruycast in System Settings → Privacy & Security → Accessibility to enable window management, text delivery and Hyper. Some keyboard features may also require Input Monitoring. Search and clipboard copying work without Accessibility.

Updates reuse the local certificate and designated requirement. Keep the signing directory when updating; the installer refuses silent identity replacement. Restoring a different certificate or switching to a Developer ID distribution can require renewed macOS permission approval. Ruycast never modifies the macOS privacy database.

## Updates and signing

Run `make install` from the updated source. Existing authored configuration is preserved, and an installation failure restores the previous app/registration where possible. The installer verifies that release binaries match their build receipt before replacing the app.

Local signing is for building on your own Mac. It is not Developer ID notarization for downloaded public binaries. See the [release guide](RELEASING.md) for distribution status and requirements.
