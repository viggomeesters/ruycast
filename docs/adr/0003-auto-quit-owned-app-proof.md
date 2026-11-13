# Auto Quit owned native application proof

Status: bounded native integration proof, 13 November 2026. Refs #371 and #141.

The clock and admission tests previously had no actual AppKit application at
which to direct a normal quit. The new `auto-quit-owned-app` test-tools example
is an owned Rust/AppKit application with no windows, documents, media/capture
APIs, plugins or child processes. It never activates. Its three normal modes return
`NSTerminateNow`, `NSTerminateCancel` and `NSTerminateLater` respectively.
The deferred mode models a pending save decision; no physical save panel is shown.

The unit-test-only bridge launches only that owned executable in a fresh .app
bundle and isolated absolute directory. It pins its directly spawned Child PID,
UID and start timestamp, bundle identifier and exact executable path. The live
snapshot checks the process lifetime before and after native observations.
Immediately before `NSRunningApplication.terminate`, it reacquires those facts,
foreground state and the current supplied preferences, then invokes the same
Tracker/Guard policy. Only a qualified exact owned process admits. A timer
candidate can make at most one request, including after refusal or deferment.
Foreground state comes from synchronous public `GetFrontProcess` and
`GetProcessPID` calls, with errors treated as Unknown. These APIs are deprecated
but available in the installed SDK; they are used only for this bounded test
bridge. Mutable NSRunningApplication properties such as `isActive` retain
values until the next main-runloop turn and are not used for this final check.
A successful send is separately checked against delegate receipt and successful
process exit; it is never presented as proof of successful termination by itself.

The qualification and bridge compile only under `cfg(test)`. Default and
`test-tools` product libraries still have no admitting Decision variant. This
adds no production allowlist, setting, environment bypass, polling or app
inventory. The not-connected editor status remains accurate. Production live
sleep/wake/foreground/safety observations and qualified capture/helper coverage
are still missing under #371. General applications remain blocked. #141 and
#89 remain open; this proof does not ship an operational Auto Quit feature.

## Focused proof

Build the example with a 90-second deadline. Run only the ignored native proof
with `RUYCAST_AUTO_QUIT_FIXTURE_BIN` pointing to that built absolute executable
and `RUYCAST_AUTO_QUIT_FIXTURE_DIR` pointing to a fresh owned absolute directory.
The normal unit-test suite does not launch a native application automatically.

```sh
perl -e 'alarm shift; exec @ARGV' 90 cargo build --locked --offline \
  --manifest-path rust-native/Cargo.toml --features test-tools \
  --example auto-quit-owned-app
# Supply the two explicit absolute environment paths described above.
perl -e 'alarm shift; exec @ARGV' 90 cargo test --locked --offline \
  --manifest-path rust-native/Cargo.toml --lib auto_quit \
  -- --include-ignored --nocapture
```

On macOS 26.6.2 (25G83), all 16 selected clock, preference, admission and native
proof tests passed. The native group used simulated monotonic timer expiry,
then actual native identity/foreground reads and normal quit requests. Before
effect it proved disabled preference, stale process candidate and synthetic
foreground reset block the request. The foreground transition is synthetic to
avoid taking focus; a real foreground transition was not induced. Accepted
quit exited, cancellation stayed alive and deferred termination stayed alive.
The two latter modes received exactly one delegate request. A fourth owned
mode exits with status 37 immediately after writing its quit receipt; the proof
requires this to be recognized as a non-success exit. A receipt plus a vanished
process therefore cannot satisfy the accepted-quit assertion. Cleanup is the
owned child's separate stdin shutdown; a 20-second self-exit watchdog bounds
orphaned fixtures. No force-kill or other application control is used.

Initial fixture starts failed before native proof: asserting the return value
of setting an already-selected activation policy was too strict. Checking the
actual Accessory policy fixed that. A later initial quit request returned
false during direct launch startup. Readiness now comes from the running main
event loop, after AppKit setup, rather than merely after constructing the app.
The final native run passed with that handshake. These failures remain failed
attempts, not passing acceptance. Local raw runner logs and per-child receipts
are retained outside public source; this document records the safe outcome.

Public API contract:
[Apple NSRunningApplication.terminate](https://developer.apple.com/documentation/appkit/nsrunningapplication/terminate()).
Return true means the request was sent; observation of termination remains
necessary. No resource saving, recording detection, real save-panel interaction,
full product acceptance, installation or release is claimed.

Validation also passed the exhaustive product-admission integration test with
both default features and `test-tools`, focused Clippy for the product library,
admission target and owned-app example with warnings denied, rustfmt, the
Rust-only checker and diff check. An additional broad Clippy `--tests` run
fails on existing clone-to-single-element-slice lints in the earlier timer and
safety unit tests and existing generic-borrow lints in `private_results.rs`.
Those unrelated checks were not weakened or reported as passing.
