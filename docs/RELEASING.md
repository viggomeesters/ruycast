# Release status

## Rust beta source preview

The beta is a separate macOS app alongside Ruycast 0.8.26. It uses the
`nl.viggomeesters.ruycast.preview` bundle ID and stores data under
`~/Library/Application Support/Ruycast Rust Preview`. It starts with its own
configuration and history. Set a distinct launcher shortcut when both apps
are running. Review the clipboard setting before starting the beta if you
share this Mac.

Verify the SHA-256 of the source archive against the checksum attached to the
same GitHub prerelease. From the extracted source, build the beta host with:

```sh
CARGO_INCREMENTAL=0 cargo build --locked --release --manifest-path rust-native/Cargo.toml --features local-preview --bin ruycast-native
```

The prerelease supplies source. Use a bundle built from the verified source.
Stage the preview app separately from the
installed app, with the preview bundle ID, the host at
`Contents/MacOS/RuycastApp`, the CLI at `Contents/Resources/ruycast` and a
strictly higher numeric `CFBundleVersion`. Update an already enrolled preview with:

```sh
make install-preview APP="/absolute/path/to/staged-preview.app"
```

The command uses the existing local Ruycast signing identity from
`~/.local/share/ruycast/signing`. It verifies the old and new bundles against
the same preview requirement before stopping the running app. It restarts the
preview and checks that Accessibility and Input Monitoring rights that were
enabled before the update are still enabled. It retains the prior signed app
for rollback. The preview must be running and enrolled with the local
certificate before this update command can be used. The production installer
below uses the stable app identity.

If the original keychain cannot be unlocked, first preserve it and check for a
usable backup. To explicitly replace only the preview signing identity, run:

```sh
CARGO_INCREMENTAL=0 cargo run --locked --release --manifest-path rust-native/Cargo.toml --bin ruycast-install -- install-preview-reenroll "/absolute/path/to/staged-preview.app"
```

This verifies the existing preview against the original pinned certificate,
creates or reuses a separate persistent signer under
`~/.local/share/ruycast-preview/signing`, and retains the prior signed bundle for
rollback. It preserves the original keychain, stable app and preview profile.
Accessibility and Input Monitoring may need to be granted manually in macOS
System Settings because the preview's signing identity changes. The command
reports the actual permission and health state without claiming grants were retained.
The replacement host must provide the `storageAndKeyboardHealthy` status field;
independent storage or keyboard errors still cause rollback.
Future ordinary `make install-preview` updates reuse the separate preview signer
and again require existing permissions to remain enabled. If signing recovery
fails before replacement, the original preview stays installed; rerun the
explicit recovery command to reuse the prepared dedicated signer.

F1 comparisons and Developer ID notarization are pending.

## Current Rust candidate

The complete native rewrite remains a development candidate. V4 Rust build receipts and unsigned candidate receipts keep `productionAccepted: false`, `installedOrPublished: false` and no signing/notarization claim. Historical candidate 67 and v2/v3 receipts are preserved evidence, not new installation authority. No new release, public push, installation or publication follows implicitly from these tools.

Production build, version, diagnostic preparation and packaging policy are Rust. Compatibility Python/shell entrypoints forward arguments to an explicitly host-targeted, locked offline Cargo tool. `rust/src/product_version.rs` is the sole product-version literal. The Swift tree is a historical oracle and not a production fallback.

Export an explicit reviewed public source snapshot first and verify its clean archive before compilation. The accepted source-export policy excludes `.go`, profiles, signing material, private adapters and Actions. After building, packaging permits `.build` outputs but rebinds every PUBLIC-SOURCE.v2 file/mode and the entire production inventory. Unreviewed new production files, source changes, private checkouts, stale/pre-v4/debug receipts and existing outputs are refused.

```sh
bash scripts/build-native.sh -c release
python3 scripts/build_receipt.py verify
python3 scripts/package-app.py --out "$PWD/.dist/new-candidate" --build-number "$RUYCAST_BUILD_NUMBER"
python3 scripts/package-app.py --verify "$PWD/.dist/new-candidate"
```

Choose a fresh output and an explicit positive build number greater than the previous distributed build. Release preparation retains matching original binaries/dSYM privately, verifies code/data sections and UUID, and binds the preparation record into v4. The app contains only host, CLI, optional icon and fixed plist, including the Safari Automation purpose. It contains no profile, signing material, runtime adapters or diagnostics.

Complete functional/native/accessibility/F1, safe installation/readback and clean public-build acceptance remain separate gates. The full F1 comparison needs independently reviewed runner/authority pins and the preserved complete dataset; historical Swift timings or a small model benchmark do not satisfy it. Developer ID/notarization is deferred until the maintainer has the required account/identity. Do not run raw historical Swift-host checks without complete pre-entry SDK confinement.

### Updating the Rust product version

Change the single `CURRENT` semantic-version literal in `rust/src/product_version.rs`. The Rust build tool checks that source value against the compiled host and CLI; installer and source-export authority use it too. `python3 scripts/product_version.py` dispatches to that Rust authority. Its retained Python import function is a historical Swift-version test oracle.

Update current release examples and the changelog, rebuild the tools, then regenerate and verify the public source snapshot. Build and verify the prepared release pair from that reviewed clean snapshot before running the required local gates. The Cargo package versions and historical release notes are separate from the product version. Keep the frozen Swift reference unchanged.

## Historical 0.8.25 source preview

The remaining release/distribution notes predate the rewrite. They record the previous source preview and pending distribution requirements; their old build paths/recipes are not the current Rust build authority.

**[0.8.25 source preview](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.25)** — build locally on macOS 14 or later. This release contains the launcher, native Settings, CLI, clipboard, window management, snippets and configurable storage. No prebuilt notarized app is distributed.

| Item | Status |
|---|---|
| Repository | [viggomeesters/ruycast](https://github.com/viggomeesters/ruycast) · public · MIT |
| Release | [v0.8.25](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.25) · source preview |
| Source asset | `ruycast-0.8.25-source.tar.gz` with SHA-256 checksum |
| Installation | Clone or extract source, then run `make install` |
| Local signing | A persistent per-Mac signing identity is reused on updates |
| Public binary | No Developer ID notarization or prebuilt app download |
| Validation | Local Rust, Swift, Python and hidden-native checks; no GitHub Actions |

## Build this release

```sh
git clone --branch v0.8.25 https://github.com/viggomeesters/ruycast.git
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

### Candidate preparation (issue #1, not yet a binary release)

Use the reviewed **public** checkout, never a private development checkout or an installed personal app. `package-app.py` checks the public manifest and release-build receipt, copies only the app executable, CLI and icon, and writes an explicitly non-publishable candidate receipt. It does not install, sign, upload, publish, copy configuration or include personal adapters. Existing output is preserved.

```sh
make check
bash scripts/build-native.sh -c release
python3 scripts/package-app.py --out .dist/public-candidate --build-number 26
```

Choose a fresh candidate directory and a numeric build greater than the previous distributed build. The manifest supplies the product version. Candidate preparation does **not** prove Developer ID signing, notarization, supported architectures or clean-Mac acceptance.

### Distribution prerequisites

- A valid **Developer ID Application** identity with its private key in the maintainer's keychain. Inspect with `security find-identity -v -p codesigning`. A local/self-signed identity is insufficient.
- The intended Apple Developer team and a working `notarytool` keychain profile. Provision credentials interactively with `xcrun notarytool store-credentials`; never put passwords, keys or keychain databases in scripts, issues, archives or Git.
- A clean macOS test environment and an upgrade fixture. Record exact macOS versions and CPU architectures actually tested. Apple silicon development builds do not establish Intel support.

These are open acceptance gates for issue #1, not completed checks. See Apple's [notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) and [custom workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).

### Signing and notarization procedure (pending live validation)

Only continue once the prerequisites exist. Set `RUYCAST_SIGNING_IDENTITY` to the Developer ID certificate SHA-1, `RUYCAST_TEAM_ID` to its team, and `RUYCAST_NOTARY_PROFILE` to the credential profile name. Work on the staged copy; do not sign over `/Applications/Ruycast.app`.

```sh
set -euo pipefail
: "${RUYCAST_SIGNING_IDENTITY:?Developer ID identity required}"
: "${RUYCAST_TEAM_ID:?Developer team required}"
: "${RUYCAST_NOTARY_PROFILE:?Notary keychain profile required}"
RUYCAST_CANDIDATE=".dist/public-candidate"
RUYCAST_APP="$RUYCAST_CANDIDATE/Ruycast.app"
# A candidate may enter this procedure once, including after an interrupted run.
mkdir "$RUYCAST_CANDIDATE/.distribution-run"
set -C
for RUYCAST_OUTPUT in notary-upload.zip notary-result.json Ruycast-macOS.zip Ruycast-macOS.zip.sha256 release-receipt.json; do
  if [ -e "$RUYCAST_CANDIDATE/$RUYCAST_OUTPUT" ] || [ -L "$RUYCAST_CANDIDATE/$RUYCAST_OUTPUT" ]; then
    printf 'Existing artifact preserved: %s\n' "$RUYCAST_OUTPUT" >&2
    exit 1
  fi
done
python3 scripts/package-app.py --verify "$RUYCAST_CANDIDATE"
codesign --force --options runtime --timestamp --sign "$RUYCAST_SIGNING_IDENTITY" \
  "$RUYCAST_APP/Contents/Resources/ruycast"
codesign --force --options runtime --timestamp --sign "$RUYCAST_SIGNING_IDENTITY" "$RUYCAST_APP"
codesign --verify --deep --strict -R "=anchor apple generic and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$RUYCAST_TEAM_ID\"" "$RUYCAST_APP"
codesign --display --verbose=4 "$RUYCAST_APP"
lipo -archs "$RUYCAST_APP/Contents/MacOS/RuycastApp"
lipo -archs "$RUYCAST_APP/Contents/Resources/ruycast"
ditto -c -k --keepParent "$RUYCAST_APP" "$RUYCAST_CANDIDATE/notary-upload.zip"
xcrun notarytool submit "$RUYCAST_CANDIDATE/notary-upload.zip" \
  --keychain-profile "$RUYCAST_NOTARY_PROFILE" --wait --output-format json \
  > "$RUYCAST_CANDIDATE/notary-result.json"
python3 - "$RUYCAST_CANDIDATE/notary-result.json" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
if result.get('status') != 'Accepted':
    raise SystemExit('Notarization not accepted; inspect the submission log.')
print(result['id'])
PY
# Download and inspect that submission's notarytool log, including warnings.
xcrun stapler staple "$RUYCAST_APP"
xcrun stapler validate "$RUYCAST_APP"
spctl --assess --type execute --verbose=2 "$RUYCAST_APP"
ditto -c -k --keepParent "$RUYCAST_APP" "$RUYCAST_CANDIDATE/Ruycast-macOS.zip"
(cd "$RUYCAST_CANDIDATE" && shasum -a 256 Ruycast-macOS.zip > Ruycast-macOS.zip.sha256)
python3 - "$RUYCAST_CANDIDATE" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root = pathlib.Path(sys.argv[1])
record = {'unsignedCandidate': json.loads((root/'candidate.json').read_text()),
          'notarization': json.loads((root/'notary-result.json').read_text()),
          'zipSHA256': hashlib.sha256((root/'Ruycast-macOS.zip').read_bytes()).hexdigest(),
          'publicCommit': subprocess.check_output(['git','rev-parse','HEAD'], text=True).strip(),
          'signature': subprocess.check_output(['codesign','--display','--verbose=4',str(root/'Ruycast.app')], stderr=subprocess.STDOUT, text=True)}
target = root/'release-receipt.json'
with target.open('x') as output:
    json.dump(record, output, indent=2)
PY
```

The final ZIP must be created **after** stapling. Keep the separate final receipt with the source/build binding, signature, notarization ID and ZIP hash. The original upload ZIP and unsigned `candidate.json` are not release assets. Before retrying, use a fresh candidate instead of silently overwriting any ZIP or receipt. No signing/notarization command above has been validated with distribution credentials yet.

### Required acceptance before publication

Test the downloaded final ZIP on a clean Mac with Gatekeeper and quarantine intact, including an offline launch after stapling. Verify first launch, Accessibility/Input Monitoring enrollment, launcher/app activation, clipboard, snippets, window management and login startup with one silent instance. Record exact OS/architecture, signature/team, notarization ID, final checksum and test results. A successful `spctl` assessment on the build host alone is insufficient.

Test upgrades with preexisting configuration/history and permissions. Preserve `nl.viggomeesters.ruycast` and the distribution team's designated requirement in subsequent releases. The first transition from a local certificate to Developer ID changes the signing requirement and may need one new permission enrollment; do not promise zero prompts. The source installer deliberately refuses an unexpected signing identity: it must not overwrite a distribution installation. Prepare and validate that transition separately before changing the maintainer's daily app.

Only after all gates pass: finish the linked PR review, merge, publish versioned ZIP/checksum and matching source assets from that public commit, download them again, verify hashes/signature/stapling, and update the original issue/note. Until then keep issue #1 open and describe the current release as source-only. No GitHub Actions or updater is introduced.

Local source builds can stage an explicitly supplied personal adapter with `RUYCAST_PERSONAL_BRIDGE=/absolute/path/to/adapter.py make install`. The adapter is copied directly into the staged app before signing; it is never copied into public source or source release assets. Without this override, the existing installed adapter is retained.

## Historical Swift product-version procedure

Change the literal in `Sources/LauncherCore/ProductVersion.swift`; status, analytics, Settings, installer metadata and public-source manifest derive from it. Keep the enum/literal source form: the Python reader rejects expressions, duplicate declarations and malformed versions. Update the current README/release examples and changelog, then regenerate the source manifest/archive and run the local gates. Historical release notes and the Rust library package version are independent.
