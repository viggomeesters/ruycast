# Configuration and control

Configuration defaults to `~/.config/ruycast/config.json`. Choose another folder in **Settings → Advanced → Configuration Folder**, or use the background CLI:

```sh
ruycast config directory                          # Show the active folder
ruycast config directory "$HOME/Documents/Ruycast" # Copy configuration and select this folder
ruycast config path                               # Show the active config.json
```

The folder choice persists in the machine-local `~/.config/ruycast/config-location.json`. CLI commands, Finder launches, login startup and future installations use it automatically. A running host switches without restarting; edits at the new location still reload automatically. An existing destination `config.json` is never overwritten. The previous file remains at its original location as a recovery copy, and is no longer active. An unavailable selected folder reports an error instead of silently creating a fresh configuration.

For an explicit environment override, set `RUYCAST_CONFIG_DIR` to an absolute folder path for both the app and CLI. It takes precedence over the saved choice; unset it before changing folders through Settings or `config directory`. For a separate *runtime profile* in tests, use `RUYCAST_HOME`; its own `config-location.json` and runtime files remain isolated. Precedence: `RUYCAST_CONFIG_DIR` → the profile’s saved directory → the profile itself. `RUSTCAST_HOME` remains a compatible explicit runtime override. Unrelated RustCast profiles are never selected automatically.

If you edit the machine-local selector directly, restart the host to apply that location. Prefer Settings or `config directory` for a live switch.

Only `config.json` moves. Clipboard history, image assets, analytics, utility state, sockets, locks, logs and the directory selector stay in the local runtime profile. A custom synced folder may expose the scripts, snippets and URLs in your configuration to your sync provider and other synced devices. A new Mac still needs its own app installation, folder selection, permissions and any external scripts. Selecting a folder does not publish it or add it to the source repository.


The shipped version-1 example is `config/default.json`. Unknown fields, invalid types, duplicate IDs/aliases and duplicate/invalid shortcuts fail validation. Configured IDs and aliases are case-insensitive. Commands are data, so an agent can add or remove them without rebuilding the app. Source is available for changing the UI or adding a new kind.

| Field | Meaning |
|---|---|
| `version` | Must be 1 |
| `launcherHotkey`, `clipboardHotkey` | `ctrl+alt+space`, `ctrl+alt+v`; combinations of ctrl/alt/shift/cmd and a supported physical key |
| `clipboard.enabled` | Capture enabled; the persisted runtime pause flag can additionally pause capture |
| `clipboard.limit` | 1–1000 text items; default 200 |
| `clipboard.maxTextBytes` | Per-item UTF-8 limit, 1–1000000; default 100000 |
| `clipboard.retentionDays` | 1–365 days; default 30 |
| `clipboard.excludedApps` | Bundle IDs excluded while in the foreground; concealed/transient items and uncertain app-switch intervals are also skipped |
| `commands` | Ordered command array; usage ranks the empty query, exact aliases rank first for typed searches |

A command requires `id`, `title`, `kind` and `target`; optional `aliases`, `arguments`, `hotkey` and `timeout` customize it.

| Kind | Target |
|---|---|
| `app` | Installed bundle identifier, e.g. `com.apple.Safari` |
| `url` | Absolute URL/deeplink, including custom app schemes; javascript/data URLs are rejected |
| `path` | Absolute or `~/` file/folder path |
| `script` | Absolute or `~/` executable; literal `arguments`, timeout 0.1–300 seconds (default 30) |
| `window` | `left-half`, `right-half`, `first-third`, `center-third`, `last-third`, `first-two-thirds`, `last-two-thirds`, `maximize`, `restore`, `next-screen`, `previous-screen` |
| `builtin` | `clipboard` |

Configured shortcuts are physical macOS keycodes (US key-position naming). An initial shortcut conflict is visible in `status.hotkeyWarnings`; choose another shortcut. A reload conflict keeps the existing bindings and reports an error.

## CLI

All successful commands return `{"ok":true,...}` and exit 0. Failures return `{"ok":false,"error":"..."}` and exit 1. `--help` lists supported forms. Config commands work with the host stopped; execution/history commands require it.

- `status`: process/profile/version, commands/discovered apps, clip count and state, Accessibility, config and hotkey errors. No clipboard bodies.
- `commands`, `search QUERY`: action catalog and ranked matches.
- `run ID_OR_ALIAS [--dry-run]`: execute or inspect; dry run neither executes nor increments usage.
- `window LAYOUT`: execute the configured `window.LAYOUT` action.
- `show [--clipboard]`: explicit UI open; `reload` validates and applies config in the background.
- `clipboard list [QUERY]`, `copy ID`, `paste ID`, `delete ID`, `clear`, `pause`, `resume`.
- `usage`: successful action counts collected since Rustcast started recording them; historical Raycast evidence is separate.
- `config directory [ABSOLUTE_PATH]`: inspect or change the portable configuration folder.
- `config path|get|init|validate [PATH]|reload|set KEY JSON|add JSON|remove ID`.
- `start [--headless]`, `quit`: explicit lifecycle. Headless mode uses an isolated profile in tests, registers no hotkeys and captures no clipboard.

For direct local integrations the profile contains `control.sock`. Send one JSON object terminated by newline, max 1 MiB; the reply is one JSON object terminated by newline. Supported operations are visible in `AppState.handle`; the CLI is the maintained interface. No network listener is started.

## Local files

`config.json` in the selected directory owns settings; `clipboard.json` holds text history; `usage.json` holds action counts; `paused.json` holds the capture pause preference. `control.sock`, `instance.lock`, `config.lock` and `launcher.log` are runtime files. Keep runtime/history files outside configuration Git. `RUYCAST_HOME` selects a separate local runtime profile; the production LaunchAgent pins that profile explicitly.

Do not start a second profile casually: each non-headless profile registers its own hotkeys and observes the same system clipboard. The instance lock guarantees one host per profile, not a machine-wide singleton across deliberately separate profiles.

## Snippets, inputs and quicklinks (0.2)

`kind: "snippet"` uses `target` for its text and an optional `keyword` for automatic expansion. Keywords are unique, case-sensitive, at most 64 characters, without whitespace. Names and keywords are searchable. Snippet text is excluded from the normalized search haystack. `snippetExpansionEnabled` defaults to false; only this explicit flag enables a keystroke tap. Rustcast suspends expansion while Raycast runs, in secure entry, recognized password fields, excluded apps, or while its palette is visible. It requires Accessibility and possibly Input Monitoring. The Snippets tab shows the precise activation reason and a settings button. It never grants permission or quits Raycast automatically.

URL quicklinks support `{query}`, `{argument}`, named `{Place}` and `{argument name="Name" options="One,Two" default="One"}` inputs. Values are percent-encoded individually. Snippets support named arguments, `{clipboard}`, `{cursor}`, `{date}`, `{time}`, `{day}`, `{datetime}` and `{date format="yyyyMMdd-HHmm"}`. `{datetime}` renders the current local date and time as `25 October 2025 19:35`, with the month in your system language. Use `{datetime:yyyy-MM-dd HH:mm}` for the previous numeric format. The existing imported token forms are covered. Automatic expansion uses defaults for argument fields; snippets needing a human value remain available in the launcher and prompt there.

Script commands can declare `inputPrompt` and include exact `{input}` or `{uuid}` argument tokens. Tokens substitute whole argv elements, with no shell expansion. `ruycast run ID --input '123'` preserves literal text; a JSON object supplies named fields, e.g. `--input '{"Query":"coffee & tea"}'`. Missing required values fail before execution. `--dry-run` resolves the definition without executing it.

Use `ruycast snippets list [query]`, `ruycast quicklinks list [query]`, `... add JSON`, `... remove ID`, and the existing SHA-guarded `config` commands. List commands return the complete filtered library. GUI editors use the same locked CLI with an expected SHA; conflicts preserve external edits.

## Appearance and launcher input

Settings → General offers System, Light, Dark and Graphite themes and a Menu Bar Icon toggle. CLI equivalents are `ruycast config set theme '"graphite"'` and `ruycast config set menuBarVisible false`. Old configurations default to the system theme and a visible icon.

Search Settings, Open Configuration, Reload Configuration or System Monitor in the launcher. Settings also appears in the Commands library, where Alias & Shortcut assigns a hotkey. The menu bar contains only Open Launcher, Settings and Quit; its orange icon indicates a reported runtime problem.

Commands with one free-text input stay in the launcher: select one and press Return or Tab, or type its exact alias followed by a space and input. Return runs it; Escape returns to search. Shift–Return or Actions → Expand Input opens the larger editor and preserves the text. Commands with multiple fields or options retain their forms. Technical script fields are under Advanced when editing a command.

Missing focused-field accessibility metadata (for example in Electron or web-backed editors) does not by itself disable snippets. OS secure input and recognized password fields still block expansion; Accessibility permission and excluded-app settings remain required. Unknown fields cannot provide the extra password-subrole check, so use excluded apps for editors that do not expose their password fields or enable macOS secure input.

## Left and right Command

Enable **Settings → General → Separate Left and Right Command** to bind the
Command keys independently. Then assign `right-cmd+s` or `left-cmd+s`, or record a shortcut while holding the desired Command key. Its
badge shows `R⌘` or `L⌘`. Right-click a recorded sided shortcut and choose
**Use Either Command** for a generic binding. Hyper recording stays generic.

Turning the switch off pauses saved `left-cmd`/`right-cmd` bindings without
removing them; recording then produces generic `cmd` shortcuts. Turning it on
reactivates them. The CLI equivalent is `ruycast config set separateCommandKeys true`
(or `false`). `ruycast status` reports the effective setting and Input Monitoring.
New profiles start off; older files without this field keep their prior enabled
behavior, so existing sided bindings continue working.

Existing `cmd+s` bindings accept either Command key. A matching sided shortcut
takes priority over a generic binding for the same key. Unbound keys keep their
normal behavior. Both Command keys together do not match a single-side binding;
key repeat launches the app once per press. Extra modifiers must match exactly.

Sided shortcuts require Accessibility and Input Monitoring; keyboard status
appears in Settings. They pause during secure input and while Raycast is running.
Quit another shortcut manager (such as rcmd) before using overlapping bindings.
Configure and test your replacement bindings before removing it. Ruycast opens
or activates configured apps; it does not import rcmd bindings or assign letters.


### Snippet choices

A keyword can open a choice menu using the existing Raycast-compatible syntax:

```json
{"id":"snippet.contact","title":"Choose a contact","kind":"snippet","keyword":";contact","target":"{argument name=\"Contact\" options=\"Example A, Example B, Example C\"}"}
```

Type the keyword, use Up/Left for the previous option or Down/Right for the next, then press Return (or keypad Enter) to insert it. The arrow keys work immediately without opening the dropdown; selection stops at the first/last option. Mouse selection remains available. In forms with multiple fields, Tab changes focus and arrows keep their normal behavior in text fields. Escape cancels and leaves the keyword unchanged. Choices are trimmed, empty entries removed and duplicates collapsed. A choice with a default still asks before inserting. Existing imported templates and configuration remain usable without conversion.

The choice is tied to the original app. Switching apps, clicking outside or typing into the editor while the menu is pending cancels it. When Accessibility exposes the field, selection and text, they are checked again before replacement. Editors without that metadata use the app and input guards; a background programmatic field change in such an editor cannot be verified. Secure input, known password fields and excluded apps remain blocked. The menu stays local and choice values are not logged.

### Natural dates for script inputs

Script commands whose `inputPrompt` includes the existing `YYYY-MM-DD` hint accept natural dates in the launcher. The launcher shows a live calendar date and passes that exact ISO value to the script on Enter. Examples: `tomorrow`, `morgen`, `next month`, `in 3 days`, `over 3 dagen`, `vrijdag`, `5 november` and `2026-11-05`. `clear` or `wissen` removes a date. Partial or unrecognized dates keep the input open without running the command. Relative dates use the Europe/Amsterdam calendar; “next month” means the same day next month, clipped to the last day when needed. Weekdays select the next occurrence.

### Interactive script choices

A script adapter can return `{"ok":true,"choices":{"title":"Choose","placeholder":"Search…","items":[{"id":"one","title":"Example","subtitle":"Group","value":"pinned-value"}]}}`. Ruycast opens a searchable keyboard list. Enter runs the same command with the selected opaque value as `{input}`; Escape leaves without running a selection. Failed selections keep the list and offer Reload. Scripts own validation and must reject stale context before changing data. A failed response can include `detail` to retain its report alongside the error.

Interaction durations are available through `ruycast performance`; `performance reset --confirm` clears the bounded in-memory samples. These contain no command IDs, queries or clipboard contents and are not persisted.


### Search ranking

Search supports title word starts (`mic tea` → Microsoft Teams), initials (`mt`) and ordered title subsequences (`mft`). Exact IDs, aliases and snippet keywords keep first priority, followed by exact titles. With no exact, prefix, word-start/initial or literal substring match, one alphanumeric query term of 4–64 Unicode scalars can recover a substituted, inserted, deleted or adjacent-transposed character in a title word. A corrected full single-word title precedes a corrected word inside a longer title; both precede weak ordered subsequences (`fnider` → Finder before Find Reminders). Any stronger existing match preserves the whole established result list. Queries containing punctuation or multiple terms, and shorter/longer tokens, keep ordinary matching. Execution IDs, aliases and argument text remain literal. Recent successful choices from the launcher break ties only within the same relevance tier. Existing empty-query usage ordering remains available.

`ruycast search-ranking` shows the local entry count and bounds; `ruycast search-ranking reset --confirm` clears these choice timestamps. This does not reset existing usage counters. At most 512 IDs/timestamps are stored in the runtime profile's private `search-ranking.json`; 90-day retention is applied at load/save. CLI/hotkey execution, dry runs and failed actions do not add entries. Query and argument text are never recorded here. The configuration file does not change.


Interaction diagnostics use schema 2 from 0.7.16: `input.reconcile` ends when SwiftUI applies the binding to the same native field; `host.drawCallback` counts an optional hosting draw override that normal layer-backed display may skip. Neither is result rendering or physical screen latency. The old input.draw/native.draw stages are retired.

### Set an item's alias or hotkey

Select a result and press **Command–Option–comma** for **Set Alias…**, or **Command–Shift–comma** for **Set Hotkey…**. Both actions also appear in the Actions menu with their shortcuts. Alias input is focused immediately; Return saves the comma-separated aliases. Hotkey recording starts immediately and saves the combination you press. Escape cancels; Delete while recording clears the hotkey. Existing validation and conflict errors apply; other command fields are preserved.


### App menu actions

Run **Search Menu Actions** (alias `menu-actions`), or your existing menu-search command, to search the app you were using before opening Ruycast. An existing configured command targeting `search-menu-items` is reused. Results show the action, app name, menu path and the shortcut exposed by macOS. Unavailable actions remain visible; Return runs an available action. Refresh reloads the current app menu.

These shortcuts belong to the source app and are shown separately from Ruycast bindings. They do not register global hotkeys or change command configuration. Menu discovery depends on the source app’s Accessibility support; it does not enumerate every macOS shortcut or detect all conflicts.

`ruycast search --mode menus` returns temporary command results with a `menu` object containing `application`, `path`, `title`, `enabled` and an optional `shortcut` (`keys` and accessible `label`). `ruycast run ID` executes that captured result while its source app and menu identity remain valid. Rerun the search after a result expires or changes. Menu text and shortcuts are not persisted; temporary result IDs do not enter usage/recency storage, and analytics groups menu actions under one category.


### Open windows

Run **Search Windows** (alias `windows`), or an existing command targeting `switch-windows`, to find windows by title or application name. Results show the window title and app, with **Minimized** when macOS exposes that state. Return switches to the selected captured window; minimized windows are restored first. Refresh reloads open windows. Existing window layouts and bindings are preserved.

`ruycast search --mode windows` returns temporary results with a `window` object containing `application`, `title` and optional `minimized`. `ruycast run ID` switches to that captured window. Closed, changed, unreadable or expired targets require a new search; a same-title window is never used as a fallback. Temporary IDs do not enter persistent usage/recency; analytics groups switching under one category.

Accessibility permission is required. Search considers up to 200 regular applications, 50 exposed windows per app and 100 results within a cooperative two-second traversal budget, with per-read AX timeouts. Availability across Spaces and keyboard focus depend on the source app and macOS; universal cross-Space/focus behavior is not promised. Windows without readable identity/title are skipped; an empty but readable title appears as **Untitled Window**.


### Safari tabs

Opening the main launcher with your configured shortcut shows up to five open Safari tabs first, followed by the existing suggestions. Typing matches tab titles/addresses alongside apps and commands, with at most 50 matching tabs and 100 combined UI results. Local matches appear immediately; optional tab results arrive asynchronously. A navigated selection, Actions menu or editor is not retargeted by late tab or permission results. The first explicit launcher hotkey/menu entry can request Safari Automation permission; typing, background CLI search and generic CLI `show` do not request it.

The main tab group has a separate worker/store and a one-second budget measured from request time, including queue delay. When that budget expires or Safari/access is unavailable, ordinary results remain usable. Data sends use the remaining native timeout; callback delivery depends on the main run loop and is not hard real-time. Failed provider requests are throttled for one second. Hiding the launcher or changing query/mode cancels stale discovery; no startup or idle polling is added. Permission requests still use one shared controller. The two short-lived stores together retain at most 1,024 tab snapshots and 4 MiB of title/address UTF-8 content; per-response allocation remains an OS responsibility.

`ruycast search QUERY` through the normal running app uses the same tabs-first merge and `tab.url` metadata; existing `--limit` semantics remain. A `--headless` host keeps its existing local-only main search so isolated profiles/benchmarks do not read personal tabs implicitly. Explicit `ruycast search --mode tabs` remains available there. The dedicated search below retains its longer budget and useful recovery errors.

Run **Search Safari Tabs** (aliases `tabs`, `safari-tabs`, `tabbladen`) to search existing open tabs by title or address. Results show the address; Return switches to that tab. Refresh reloads Safari. Safari must already be running. The first interactive entry can request macOS Automation permission for Safari. Background/CLI data requests never prompt; if access is denied, allow Safari under Ruycast in System Settings → Privacy & Security → Automation and refresh.

CLI: `ruycast search --mode tabs` returns temporary results with a `tab.url` field; `ruycast run ID` selects the captured result. Discovery is on demand, caches for one second, and retains at most 512 tabs from 64 windows with an eight-second data deadline, 8 KiB per metadata string and 2 MiB retained response/metadata limits. The OS allocates AppleEvent replies before their size is checked. Targets expire after 30 seconds or a newer snapshot; they are not saved as alias/hotkey commands. Tab titles and addresses stay in memory; temporary IDs do not enter usage/recency, and analytics uses one category. No history, private browser files or page scripts are read.

Safari has no immutable tab ID in its automation interface. Ruycast checks the captured process lifetime, window ID, position, title and address before selection, then verifies the selected tab before bringing Safari forward and afterwards. Known closed/moved/changed targets and same-window tabs with identical title and address require a new search or manual selection in Safari; there is no name/address fallback, navigation, new tab or automatic retry. A tab closed and identically recreated at the same position cannot be distinguished, and checks plus selection are not atomic. A timeout or mismatch after selection does not undo a change Safari already processed. Cancelling discovery prevents new reads and ignores late results; it cannot revoke processed browser actions or dismiss an already displayed OS permission dialog. Physical focus/Spaces behavior and actual Safari tab activation remain outside the automated fixture acceptance.


### Apple Shortcuts

Run **Search Apple Shortcuts** (aliases `apple-shortcuts`, `opdrachten`) to list existing macOS shortcuts. Type to filter by name or a saved alias; Return runs the selected shortcut by its fixed UUID. Names do not become shell commands. Use **Set Alias…** or **Set Hotkey…** from its Actions menu to save bindings; disabling a shortcut in Ruycast preserves the original in Shortcuts.

Discovery runs on demand, caches successful lists for 30 seconds, and Refresh reloads them. The list is bounded to 1,000 shortcuts and 64 KiB per output stream, with an eight-second deadline. Unsupported list formatting, including multiline names, gives a recovery error instead of partial results. Equal names remain separate UUID targets. No shortcut is run while searching.

A run uses the existing bounded script runner, with a default five-minute timeout that can be changed through Edit. Shortcuts may display their own input or permission dialogs after you run them. Ruycast discards workflow output; it cannot interpret that output as a report or another action. A timeout ends Ruycast's wait, but does not prove that the underlying workflow or its effects stopped. Check Shortcuts before retrying. No workflow is retried automatically.

CLI: search with `ruycast search --mode shortcuts`, then execute the returned command ID. Saved aliases and hotkeys work after restart without discovery. Removing a saved binding command restores automatic discovery of that original shortcut; it does not delete the shortcut from macOS.


## Snippet output

Snippet commands accept optional `snippetOutput`: `plain` (the default), `teams`, or `html`. The editor exposes these as Plain text, Teams and Mail (HTML). Rich modes interpret Markdown links such as `[Guide](https://example.com)` and place HTML plus a plain text fallback on the clipboard. Teams uses a presentation table with one row per source line, including blank lines; mail uses HTML line breaks. Text and link attributes are escaped, and links support HTTPS, HTTP and mailto. Raw HTML is treated as text. The source, keyword, aliases and hotkey remain editable. Choose Link… with a selection or a caret inside an existing link to add or edit its text and URL.

The receiving app controls paste handling. Rich output requires normal paste into a rich-text composer; plain-text paste intentionally drops formatting. App import fixtures verify links and spacing; live client behavior can differ.
