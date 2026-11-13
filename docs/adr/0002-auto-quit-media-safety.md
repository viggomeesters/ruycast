# Auto Quit: media and recording admission

Research scope: [#140](https://github.com/viggomeesters/ruycast/issues/140),
13 November 2026. This is a bounded platform experiment and first-integration
policy. It is not implemented runtime protection or approval to quit an app.

## Decision

Core Audio process flags can contribute a positive audio-IO blocking signal.
They cannot establish that an application is safe to quit. Unknown, unavailable,
failed, stale or incomplete media/recording information must block automatic
termination. Zero audio flags also do not prove absence of recording.

An application exception is required: general media, meeting, recording and
unqualified multi-process applications remain excluded. The enabled Auto Quit
preference is not a safety override. First effect integration may qualify only
an owned synthetic test application with verified non-recording/non-media
capabilities and a complete process-lifetime binding, in isolated test mode.
That qualification must never become a production bypass for unknown status.

[Implementation #371](https://github.com/viggomeesters/ruycast/issues/371)
must connect this admission decision and its focused tests before
[#141](https://github.com/viggomeesters/ruycast/issues/141) may use it. The existing
not-connected editor status remains accurate. Timer/configuration work is not
blocked; general automatic application termination remains blocked.

## One public platform route

The system object's
[`kAudioHardwarePropertyTranslatePIDToProcessObject`](https://developer.apple.com/documentation/coreaudio/kaudiohardwarepropertytranslatepidtoprocessobject)
translates a specific PID into a Core Audio process object. Its public SDK
contract returns an unknown object when that PID has no process object, rather
than necessarily returning an error. The experiment queried only its own PID;
it did not read the process list or another application's metadata.

The SDK describes
[`kAudioProcessPropertyIsRunning`](https://developer.apple.com/documentation/coreaudio/kaudioprocesspropertyisrunning),
[`kAudioProcessPropertyIsRunningInput`](https://developer.apple.com/documentation/coreaudio/kaudioprocesspropertyisrunninginput)
and
[`kAudioProcessPropertyIsRunningOutput`](https://developer.apple.com/documentation/coreaudio/kaudioprocesspropertyisrunningoutput)
as UInt32 IO-state flags. Input/output flags describe running IO with an active
input/output stream; they do not describe a particular document, recording
session or permission. Running IO may exist without an active stream. The probe
checks the process object's PID before and after flag reads and validates sizes
and boolean values. A future product adapter still needs full process-lifetime
revalidation, runtime property-support checks and current safeguard admission.
This experiment does not qualify every supported deployment OS.

The descriptions above were read in Apple's installed SDK headers as well as
the linked Apple symbol pages (whose public Markdown pages expose declarations
but little explanatory detail). Exact inspected local sources:

- `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/CoreAudio.framework/Versions/A/Headers/AudioHardware.h`,
  SHA-256 `910e8e0fa8da940bdafdbeb15cfb928bdbde1c36229234072fb82da161c63c25`.
- `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/AudioToolbox.framework/Versions/A/Headers/AudioQueue.h`,
  SHA-256 `7237430f6d85b02c8ce40cfc96bdef87f975cfbc5c086f5a8b483cc6177f1721`.
- `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/CoreAudioTypes.framework/Versions/A/Headers/CoreAudioBaseTypes.h`
  for the PCM format layout.

## Owned experiment and observed limit

The Rust-only, test-tools-gated example
`rust-native/tests/fixtures/auto_quit_audio_probe.rs` links the existing system
CoreAudio/AudioToolbox frameworks without adding a package. It creates one
[`AudioQueueNewOutput`](https://developer.apple.com/documentation/audiotoolbox/audioqueuenewoutput(_:_:_:_:_:_:_:))
queue in its own process and enqueues three one-second initialized Float32
stereo-zero buffers. It reads no audio file, microphone, screen or camera and
does not request capture permissions, change volume or manipulate another app.
The normal path checks the explicit Stop result. Its RAII owner also invokes
Stop and Dispose, including on errors, but ignores those cleanup return values.
Successful disposal was therefore not verified. No native UI is created. This
is a real silent-output experiment, not an API mock or a measured
resource-saving claim.

Command, with a 90-second outer deadline and two-second flag-change deadlines:

```sh
perl -e 'alarm shift; exec @ARGV' 90 cargo run --locked --offline \
  --features test-tools --manifest-path rust-native/Cargo.toml \
  --example auto-quit-audio-probe
```

Observed on macOS 26.6.2, build 25G83:

| Control | Observation | Meaning |
| --- | --- | --- |
| Initial attempt: prepared queue, before explicit Start | Expected output=0 did not occur within two seconds; returned known=true, running=1, input=0, output=1 | Queue preparation is not a valid negative IO control on this host. |
| Corrected attempt: own queue after explicit Start | The positive-output checkpoint succeeded (known=true, output=1), inferred from reaching the subsequent stop-control failure | The route recognized own silent output IO. It does not prove audible playback. |
| Corrected attempt: after checked Stop and an unchecked Dispose call | Within two seconds, still known=true, running=1, input=0, output=1; probe exited 1 | No qualified transition to idle was established. Cleanup failure was not excluded, so this does not prove continued IO after successful disposal. Do not substitute a longer wait or treat this as safe idle. |

The corrected run's terminal result was transcribed from the agent tool history:

```json
{"error":"Output flag did not become 0: {\"input\":0,\"known\":true,\"output\":1,\"running\":1}","ok":false}
```

The original native-run stdout/stderr was not separately saved to a local file.
This quotation is a transcript, not an original machine receipt. Its evidence
limit remains explicit; no additional native run was made to reconstruct one.

The retained probe source for the corrected run has SHA-256
`c1c61593f1f6a5210f028a7a2edafd41e619ab7bb8126a41abb806b76230dac3`.
The first attempt was corrected only by recording prepared state without a
false idle expectation and moving the stopped measurement after the unchecked
disposal call. The ignored disposal status prevents a successful-cleanup claim.
There were no further timing/API experiments. The research result is the
limited positive signal plus a failed negative control, not a passing detector.

## Unsupported conclusions

No input stream was opened; positive microphone-capture detection was not
qualified. Camera-only recording, screen recording, silent video, recording to
memory/disk, remote playback and activity in helper processes were not tested
and are not covered by these flags. Absence of the main PID's Core Audio object,
zero flags, API errors or an unbound helper must remain unknown, never safe.
The flag reads are not atomic with an application's changing state. Their
successful return cannot replace a final live process and policy check.

The focused example passed rustfmt and Clippy with warnings denied. Its two
native executions failed the specified negative controls, as retained above.
No production adapter, privacy permission, external app, installation, release,
benchmark or GitHub Action was involved.
