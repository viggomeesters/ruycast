# Install Ruycast from source

This guide uses the current code on `main`. The documented setup is for **Apple silicon Macs running macOS 14 or later**. Intel installation has yet to be verified. You can build and sign the app locally with your own Mac account. The [release page](https://github.com/viggomeesters/ruycast/releases) contains tagged source archives and checksums.

## 1. Set up your Mac

- Install the Apple Command Line Tools with `xcode-select --install` if `xcode-select -p` reports no active developer tools. Finish the installation before continuing.
- Install Rust with [rustup](https://rustup.rs/). The checkout selects Rust 1.92.0 through `rust-toolchain.toml`.
- Install [Homebrew](https://brew.sh/) in its default Apple silicon location, then run `brew install openssl@3`. The local installer uses `/opt/homebrew/opt/openssl@3/bin/openssl` to create its initial signing certificate.
- Allow at least **38 GB of free space** for the release build's disk check, and an internet connection to fetch the Rust toolchain and dependencies.

## 2. Clone, build and install

Run these commands from Terminal as the logged-in user. If you already use Ruycast, keep a backup of your current app, profile and local signing directory before updating. The installer also stores backups when it replaces a matching local installation.

```sh
git clone https://github.com/viggomeesters/ruycast.git
cd ruycast
cargo fetch --locked --manifest-path rust/Cargo.toml
cargo fetch --locked --manifest-path rust-native/Cargo.toml
CARGO_INCREMENTAL=0 cargo run --locked --offline --release \
  --manifest-path rust-native/Cargo.toml --target-dir .build/installer-tools \
  --bin ruycast-install -- install --repo "$(pwd -P)" --build
"$HOME/.local/bin/ruycast" status
```

The two fetch commands fill Cargo's cache for the locked offline build. The installer builds Ruycast, creates a persistent local signing identity on first use, installs the app and the CLI, and starts the app via a per-user login service. A new user gets the bundled default configuration.

## 3. Open the launcher and enable what you use

Press **Control–Option–Space**, type an app name, and press Return. If the shortcut does not open the launcher, open Ruycast from Finder and check **Settings → General** for the configured shortcut.

- Enable **Accessibility** under **System Settings → Privacy & Security** for window management and pasting into other apps.
- Enable **Input Monitoring** for keyboard features such as Hyper and automatic snippet expansion, then turn on the features you want in Ruycast Settings.
- Add `~/.local/bin` to your shell's PATH to run `ruycast` without its full path.

[Getting started](GETTING-STARTED.md) · [Configuration and CLI](CONFIG.md)

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

Updates retain the certificate/designated requirement and authored configuration. Keep the signing directory; replacing it or switching to Developer ID can require renewed permission approval. The transaction verifies release authority before stopping/replacing the app, holds the same artifact lease through its final consumer, and attempts restoration after a failed update. The Apple silicon maintainer cutover and rollback were verified in [#100](https://github.com/viggomeesters/ruycast/issues/100) and [#133](https://github.com/viggomeesters/ruycast/issues/133).

V4 release authority binds explicit native target paths, both prepared binaries and retained matching dSYM/originals under `.build/full-rust-release/symbols/`. Code/data sections and UUID remain identical after stripping. Debug and historical v3 receipts cannot authorize install. Keep local diagnostics for crash investigation; they are not bundled or uploaded.

Local signing supports source builds on your own Mac. See the [release guide](RELEASING.md) for distribution requirements.
