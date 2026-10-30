# Release status

**[0.8.23 source preview](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.23)** — build locally on macOS 14 or later. This release contains the launcher, native Settings, CLI, clipboard, window management, snippets and configurable storage. No prebuilt notarized app is distributed.

| Item | Status |
|---|---|
| Repository | [viggomeesters/ruycast](https://github.com/viggomeesters/ruycast) · public · MIT |
| Release | [v0.8.23](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.23) · source preview |
| Source asset | `ruycast-0.8.23-source.tar.gz` with SHA-256 checksum |
| Installation | Clone or extract source, then run `make install` |
| Local signing | A persistent per-Mac signing identity is reused on updates |
| Public binary | No Developer ID notarization or prebuilt app download |
| Validation | Local Rust, Swift, Python and hidden-native checks; no GitHub Actions |

## Build this release

```sh
git clone --branch v0.8.23 https://github.com/viggomeesters/ruycast.git
cd ruycast
make check
make install
```

Existing configuration and permissions are preserved by the local installer. Configuration defaults to `~/.config/ruycast/config.json`; choose a different folder through Settings or `ruycast config directory /absolute/folder`. Private history, logs, sockets and signing material remain outside the source tree.

## Maintaining source releases

Use a branch and pull request for a focused change. Run the local checks in [Contributing](../CONTRIBUTING.md), inspect the native [visuals](VISUALS.md), update the changelog and review privacy before merging into `main`. Create tags only from reviewed public commits. Keep source archives and checksums attached to the matching GitHub release. This project does not use GitHub Actions.

`scripts/prepare-public.py` can produce an allowlisted source snapshot and manifest. It excludes development Git history, workflow state, profiles, build output and optional private adapters. Exporting does not publish anything. The public repository starts from a clean history; private development history must never be pushed to it.

## Downloadable app distribution

A prebuilt app release requires a maintained Developer ID identity, hardened runtime configuration, notarization/stapling, clean-machine installation and update tests. Preserve its bundle identity and designated requirement between releases. Local certificates are for source builds on your own Mac. No updater is shipped in this preview.

Local source builds can stage an explicitly supplied personal adapter with `RUYCAST_PERSONAL_BRIDGE=/absolute/path/to/adapter.py make install`. The adapter is copied directly into the staged app before signing; it is never copied into public source or source release assets. Without this override, the existing installed adapter is retained.

## Updating the product version

Change the literal in `Sources/LauncherCore/ProductVersion.swift`; status, analytics, Settings, installer metadata and public-source manifest derive from it. Keep the enum/literal source form: the Python reader rejects expressions, duplicate declarations and malformed versions. Update the current README/release examples and changelog, then regenerate the source manifest/archive and run the local gates. Historical release notes and the Rust library package version are independent.
