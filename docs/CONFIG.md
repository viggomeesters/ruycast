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

`kind: "snippet"` uses `target` for its text and an optional `keyword` for automatic expansion. Keywords are unique, case-sensitive, at most 64 characters, without whitespace. Names and keywords are searchable. Snippet text is excluded from the normalized search haystack. `snippetExpansionEnabled` defaults to false; only this explicit flag enables a keystroke tap. Rustcast suspends expansion while Raycast runs, in secure entry, unknown focused fields, excluded apps, or while its palette is visible. It requires Accessibility and possibly Input Monitoring. The Snippets tab shows the precise activation reason and a settings button. It never grants permission or quits Raycast automatically.

URL quicklinks support `{query}`, `{argument}`, named `{Place}` and `{argument name="Name" options="One,Two" default="One"}` inputs. Values are percent-encoded individually. Snippets support named arguments, `{clipboard}`, `{cursor}`, `{date}`, `{time}`, `{day}`, `{datetime}` and `{date format="yyyyMMdd-HHmm"}`. The existing imported token forms are covered. Automatic expansion uses defaults for argument fields; snippets needing a human value remain available in the launcher and prompt there.

Script commands can declare `inputPrompt` and include exact `{input}` or `{uuid}` argument tokens. Tokens substitute whole argv elements, with no shell expansion. `ruycast run ID --input '123'` preserves literal text; a JSON object supplies named fields, e.g. `--input '{"Query":"coffee & tea"}'`. Missing required values fail before execution. `--dry-run` resolves the definition without executing it.

Use `ruycast snippets list [query]`, `ruycast quicklinks list [query]`, `... add JSON`, `... remove ID`, and the existing SHA-guarded `config` commands. List commands return the complete filtered library. GUI editors use the same locked CLI with an expected SHA; conflicts preserve external edits.
