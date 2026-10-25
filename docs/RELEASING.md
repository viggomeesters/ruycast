# Release status

**[0.7.3 source preview](https://github.com/viggomeesters/ruycast/releases/tag/v0.7.3)** — build locally on macOS 14 or later. This release contains the launcher, native Settings, CLI, clipboard, window management, snippets and configurable storage. No prebuilt notarized app is distributed.

| Item | Status |
|---|---|
| Repository | [viggomeesters/ruycast](https://github.com/viggomeesters/ruycast) · public · MIT |
| Release | [v0.7.3](https://github.com/viggomeesters/ruycast/releases/tag/v0.7.3) · source preview |
| Source asset | `ruycast-0.7.3-source.tar.gz` with SHA-256 checksum |
| Installation | Clone or extract source, then run `make install` |
| Local signing | A persistent per-Mac signing identity is reused on updates |
| Public binary | No Developer ID notarization or prebuilt app download |
| Validation | Local Rust, Swift, Python and hidden-native checks; no GitHub Actions |

## Build this release

```sh
git clone --branch v0.7.3 https://github.com/viggomeesters/ruycast.git
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
