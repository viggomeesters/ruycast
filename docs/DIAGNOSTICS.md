# Command diagnostics

Inspect recent failures after an error toast disappears:

```sh
ruycast logs --errors --limit 20
ruycast logs --command script.my-action --errors
ruycast logs path
```

The Rust runtime records a redacted command identifier, source, start/completion, elapsed time, fixed outcome code and an available script exit status. Failed responses include a `diagnosticID`; matching log entries use `traceID`. The operational log does not persist provider stderr or actionable error excerpts; inspect the current error response when needed.

The owner-only `command-log.json` file lives in your profile. It is bounded to 500 entries, a seven-day window at pruning and a byte budget below 2 MB. Invalid existing data is preserved and reported rather than silently overwritten.

## Privacy

Command diagnostics are separate from aggregate analytics. Payloads, stdout/stderr, capture input, command arguments, queries and clipboard bodies are not persisted in this log. Command IDs are bounded and redact known input values, the home path, URLs and common credential patterns. Current error responses can still contain provider-authored context, so review them before sharing. The analytics export does not include this operational log.

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
