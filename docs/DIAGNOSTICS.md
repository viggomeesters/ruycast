# Command diagnostics

Inspect recent failures after an error toast disappears:

```sh
ruycast logs --errors --limit 20
ruycast logs --command script.my-action --errors
ruycast logs path
```

The Rust runtime records a redacted command identifier, source, start/completion, elapsed time, fixed outcome code and an available script exit status. Failed responses include a `diagnosticID`; matching log entries use `traceID`. The operational log does not persist provider stderr or actionable error excerpts; inspect the current error response when needed.

The owner-only `command-log.json` file lives in your profile. It is bounded to 500 entries, a seven-day window at pruning and a byte budget below 2 MB. Invalid existing data is preserved and reported rather than silently overwritten.

When a pending recorded native action loses its execution context, the local log uses the `interrupted` code and a bounded `cancellationReason` for an observed scope, supersession, dismissal, configuration or shutdown change. `multiple` means several changes were observed, and `unknown` means no single cause was established. These reasons do not prove whether an external effect occurred. Aggregate analytics uses the existing `interrupted` category; interruption remains a failed outcome in historical success/failure totals.

`ruycast performance summary` reports bounded, in-memory software timings since the host started or the measurements were reset. For supervised scripts, `script.helperSpawn` covers the helper spawn call; `script.helperReady` waits for its ready message; `script.dispatch` sends the validated request; `script.response` waits from dispatch until the helper response ends; and `script.finalize` waits for the helper exit and validates its reply. Response time includes provider startup, execution, interaction and cancellation; it is not provider startup alone. Failed or cancelled attempts contribute elapsed samples for stages they entered. Cleanup after an early response error is outside these timings. These stage aggregates contain no command identity, arguments or contents and are not physical input-to-display measurements.

Launcher timings distinguish the first visible search-opening callback in a process from later ones, including layout, window ordering, activation and focus. Field focus, key-window/active-Space checks, parked-window checks and later focus fallback are measured separately. Parked fallback covers restoring the parked surface; the subsequent order call is part of `open.keyWindow`, not that fallback measurement. A reset clears samples but does not restart the process or its first-opening marker. Percentiles use nearest-rank over retained samples: with two observations p95 is the larger one. Sample counts remain essential when interpreting small sets.

The analytics dashboard also uses nearest-rank p95. It retains up to 32 recent duration samples per command and 256 per aggregate measurement. A p95 from a small sample is descriptive of those retained observations, not a reliable prediction.

## Privacy

Command diagnostics are separate from aggregate analytics. Payloads, stdout/stderr, capture input, command arguments, queries and clipboard bodies are not persisted in this log. Command IDs are bounded and redact known input values, the home path, URLs and common credential patterns. Current error responses can still contain provider-authored context, so review them before sharing. The analytics export does not include this operational log.

For a failed Work task-router action, a verified provider can attach the original Work-relative Markdown path as `affectedFile` in the local command log. The path is tied to that invocation, never to a later active editor. It uses the same retention as other command-log entries and is not added to aggregate analytics or its exports. If the source was not verified, the field is absent. Source text and selections are not included.

Clear the log explicitly:

```sh
ruycast logs clear --confirm
```

Clearing logs leaves configuration, history and analytics intact. A command still running can record its completion afterward.

## Investigate an action

First read existing logs. A dry run (`ruycast run ID --dry-run`) inspects the definition without executing it; it does not validate the receiving app or its current document. Reproduce document-changing actions only with the intended note open or an isolated fixture. Do not repeatedly retry an action that reports an uncertain or partially applied result.

Errors from before command logging was installed cannot be reconstructed from aggregate failure counts. A successful read-only preflight also does not prove that a later write succeeds.

## Release diagnostics

Rust release preparation retains matching unstripped binaries and dSYM under `.build/full-rust-release/symbols/`. Its receipt binds the UUID and every backed code/data section before and after stripping. V4 build verification also checks those retained diagnostic hashes. Keep them for local crash investigation; candidate apps and public source archives do not include them. A failed or interrupted preparation must recover its recorded known pair before another compiler invocation; unknown changed outputs are preserved and refused.

## Startup and control socket recovery

A failed startup exits with a nonzero status after releasing its own resources. Inspect the startup error before retrying. The profile's `instance.lock` must be a regular file owned by the current user; a symlink, FIFO or other unexpected type is rejected.

Ruycast preserves an existing regular file or symlink at `control.sock`. It also preserves an existing socket if a nonblocking connection succeeds or its stale state cannot be established. An owned socket that refuses connections is recovered automatically. When stopping or rolling back, Ruycast removes the socket only while its filesystem identity still matches the one created at startup.

If startup reports an occupied or unexpected path, first determine which process or file owns it. Stop that process deliberately, or preserve the unexpected file and resolve its location before retrying. Do not blindly delete the profile or its control files: they may belong to a running instance or contain unrelated data. These checks coordinate normal Ruycast peers through the private profile and instance lock; they do not provide atomic protection against hostile concurrent namespace changes by another process running as the same user.
## Work status admission

For a compatible, verified Work status writer, a native status action may ask
the host for admission after its source is bound and before it publishes a
task binding or editor request. Closing the launcher before admission cancels
the action; after admission, closing it detaches the stale UI while the writer
finishes. Timeout and host shutdown still stop the writer, and an incomplete
post-admission result is reported as unknown. A private-vault status route
declines admission and keeps its prior behavior. Status actions started from an
existing result surface, CLI calls, older writers and other operations retain
their prior cancellation behavior. No automatic replay is performed.
The local `script.writerCapability` timing measures the compatibility probe;
it is separate from helper startup and provider execution.
An explicit Work error after admission can also follow a partial write; inspect
the Work writer journal and recover if needed before any manual retry.

The compatibility probe and pre-exec identity checks detect ordinary writer
replacement, but are not an atomic guarantee against a malicious same-user
process or a replacement between the final path check and exec. Install and
verify the compatible writer before updating the paired Ruycast host/helper.
