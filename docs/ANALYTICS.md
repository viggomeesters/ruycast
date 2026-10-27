# Local analytics

Open **Ruycast Dashboard** from the launcher or the Dashboard tab in Settings. It shows command uses, successes/failures, hourly and daily activity, command latency and recent errors. All records stay in the local profile, in `analytics.json` (0600), with no telemetry service.

```sh
ruycast analytics                         # local command-level data
ruycast analytics export                  # privacy-reduced summary as JSON
ruycast analytics pause
ruycast analytics resume
ruycast analytics retention 7             # recent daily totals/errors; 1–90 days
ruycast analytics reset --confirm          # analytics only
ruycast show analytics
```

## What is counted

A use is an executable action attempt after command/input validation, including attempts that fail. Opening an input form is not yet execution; dry runs and searches are not command uses. Launcher, hotkey, CLI and automatic snippet delivery use the same local collector. Snippet success means event delivery was initiated, not that every external editor accepted the expansion. In-flight actions finish counting if collection is paused after they began.

Existing `usage.json` successful counts are imported once as prior successes. Their failure rates and timings are unknown; they are not fabricated. Existing usage ranking remains intact. Lifetime aggregate counts stay until reset. Recent daily totals and diagnostic events use the configured retention period; the list keeps at most 200 errors. Top-level totals remain exact when the per-command table reaches its 2,048-entry limit; the dashboard reports additional uses separately.

## Timings and errors

Monotonic intervals measure search model work, clipboard filtering and command acceptance/completion through the platform adapter. CLI dispatch is time spent dispatching a request on the host, not the full round-trip. Values are milliseconds. Runtime p95 uses the most recent 32 observations per command and 256 per measurement; totals and maximums are lifetime aggregates. These timings do not measure physical display presentation or page load.

Errors retain a timestamp, category, opaque command key and a fixed reason code; script failures may include their numeric exit status. Full exception messages, stdout/stderr, targets, arguments, URLs, queries, typed input and clipboard content are excluded. Unknown exception text becomes a fixed generic code. The error list is diagnostic context; the action still shows its specific failure in the existing UI.

## Storage and controls

Persisted analytics and saved summary exports use the private writer described in [Security and data storage](../SECURITY.md#private-writes-and-recovery): payload files start private, including under inherited ACLs. The explicit export still contains the same reduced fields.

Counters update in memory under a short lock. One background writer coalesces persistence, normally within three seconds, and flushes at clean exit. A forced termination may lose the most recent unflushed records. No analytics failure blocks an action. A malformed existing file is preserved; **Reset Analytics** explicitly replaces it. A failed write is visible in Dashboard and retried on the next update or Refresh.

Storage is bounded to 8 MB, 2,048 command summaries, a fixed set of timing names and bounded samples. Command identifiers are stored as stable opaque SHA-256 keys and resolved to titles from the current catalog only in Dashboard. The key is lowercase hex SHA-256 of UTF-8 `ruycast.command:` followed by the canonical command ID, so local agents can map the current catalog without storing raw IDs. No raw IDs, arbitrary metric names or diagnostic text are accepted. Pausing stops new collection; it does not delete existing data. Reset discards pre-reset in-flight analytics completions and preserves commands, shortcuts and clipboard history. `analytics.json` is separate from the shared action configuration and available through the CLI.

The diagnostic export omits command IDs, titles, dates of individual events, profile paths and daily activity. It contains aggregate counts, fixed timing names and error categories/codes. Exporting is explicit and local; Ruycast does not upload it. Inspect any file before sharing it externally.

## Development checks

`RuycastApp --check-analytics OUTPUT` checks concurrent aggregation, import-once, limits, pause/in-flight behavior, retention, corrupt/overflow files, private file permissions, real script/CLI instrumentation, payload exclusion, reset isolation and four native light/dark/minimum-width renders. It uses an isolated profile and never injects desktop input.

## Hourly activity

The last 24 hours use UTC hour buckets with local time labels, including repeated daylight-saving hours. An action outcome is counted in the hour its attempt began. Existing schema-1 totals migrate without changes; hourly recording starts at migration, and earlier hours remain unavailable. Hourly buckets follow the configured retention period and are capped at 2,161 entries. They are excluded from summary exports.
