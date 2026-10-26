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

Type the keyword, select a value and press Return to insert it. Escape cancels and leaves the keyword unchanged. Choices are trimmed, empty entries removed and duplicates collapsed. A choice with a default still asks before inserting. Existing imported templates and configuration remain usable without conversion.

The choice is tied to the original app. Switching apps, clicking outside or typing into the editor while the menu is pending cancels it. When Accessibility exposes the field, selection and text, they are checked again before replacement. Editors without that metadata use the app and input guards; a background programmatic field change in such an editor cannot be verified. Secure input, known password fields and excluded apps remain blocked. The menu stays local and choice values are not logged.
