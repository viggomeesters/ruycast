# Auto Quit with explicit per-app acknowledgement

Decision, 13 November 2026: the maintainer explicitly chose “Per app bewust
toestaan, met waarschuwing” when recording/helper activity cannot be reliably
identified. This supersedes the absolute production default-deny choice in ADR
0002 for explicitly acknowledged apps only. Unknown activity remains unknown.
No existing enabled preference gains acknowledgement automatically.

`allowUnverifiedActivity` is a separate optional boolean, default false. The
application editor displays “May interrupt recordings or helper audio.” beside
“Allow quitting when activity is unknown”. It persists independently of enabled
and duration. Enabling alone does not authorize quitting. Known active audio
and recording, failed/unavailable reads, stale/ambiguous identity and foreground
state continue to block. Unknown main-process audio (no process object), camera,
screen and helper coverage may be accepted only by that separate acknowledgement.

The production monitor is main-owned, every two seconds, and queries only
configured enabled bundle IDs. Activation notifications reset the specific
app's interval even if it returned to background between samples. Missing or
malformed activation data resets all. Both sleep/wake edges and session lock/
unlock edges reset all intervals; pausing reasons are independent. Notification
ingress invalidates an in-flight observation via a checked epoch. Hidden test
hosts create no live monitor or workspace observers.

The serial runtime obtains current validated preferences, observes complete
process lifetimes and background state, then checks fresh preferences and
safety snapshots for due candidates. The native effect path rereads config,
process lifetime, foreground and audio again. After these sensor queries it
acquires the existing nonblocking configuration writer/relocation lock, reads
current config again and sends the normal request while holding that same
lock. A disable published during the queries wins; a busy writer blocks. The
owned native regression publishes disable immediately before final admission
and verifies that no quit reaches the child. Normal public AppKit termination
preserves application refusal and save prompts. Each timer interval consumes
at most one request, including a failed send or a pending save decision.

Foreground reads use public GetFrontProcess (OSErr/i16) and GetProcessPID
(OSStatus/i32). These APIs are deprecated but available in the current SDK.
They avoid NSRunningApplication's cache until a main-runloop turn; failure is
Unknown and cannot authorize a quit. Core Audio PID translation and generic-running/input/output
flags are checked for property size, PID binding and boolean values. Errors
block rather than substituting inactivity. No app inventory, capture permission,
private API, helper-kill route, force-kill or save confirmation is introduced.

Owned native acceptance includes both the production Native/Runtime route and
an actual monotonic one-minute wait, not only simulated deadlines: clean normal
quit after 60.87 seconds. Enabled without acknowledgement made no request;
adding acknowledgement began a new full interval. Cancellation and deferred
termination remained alive and received no repeated request. The earlier owned
bridge also rejects a nonzero exit after a quit receipt. These native tests are
explicit ignored tests; the test adapter binds its direct Child lifetime and
exact executable, and serializes fixture use. It cannot affect a different
process sharing the synthetic bundle ID. The fixture self-exit watchdog is now
90 seconds to accommodate the real minute; normal stdin cleanup is separate.
The first concurrent attempt before this extra test binding selected two owned
fixtures and failed its exact-one assertion. It affected only owned fixtures;
that failed result is not counted as acceptance.

Coverage remains bounded: main-process positive audio is detectable, generic
camera/screen/helper absence is not proved. No real user app was closed in tests,
no physical foreground switch was induced and no visible save panel was opened.
The real-minute result is not a RAM/compute benchmark. Local raw logs and owned
receipts remain outside public source. Installation and release require the
separate reviewed delivery chain.
