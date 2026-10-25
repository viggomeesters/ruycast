# Getting started

Press **Control–Option–Space**, search for an app and press Return. Ruycast discovers installed apps and displays their native icons.

## Make a command yours

Open Settings from the menu bar or launcher. Choose a command, add an alias or record a shortcut. Command–K in the launcher opens actions for the selected item; arrow keys move inside that menu and Escape returns to search.

Add a quicklink, snippet or script in Settings. Quicklinks can request values such as a search query. Script arguments are passed literally to the executable.

Configuration changes are saved to `~/.config/ruycast/config.json` by default. Choose another folder in Settings → Advanced or use `ruycast config directory /absolute/folder`; the app and CLI remember it. Clipboard history and logs stay local. The same commands are available through the CLI:

```sh
ruycast status
ruycast commands
ruycast config get
ruycast search safari
ruycast run safari --dry-run
```

## Clipboard

Press **Control–Option–V**. Search or select an item; Return pastes into the previous app, and Command–Return copies. Image and local image-file previews appear beside the list. Command–K provides copy, paste and delete actions.

Choose history limits, retention and excluded apps in Settings. Pause recording from the clipboard view. A clipboard watcher samples changes; it does not promise to capture every transition during rapid copying or app switches.

## Hyper and snippets

Enable Caps Lock as Hyper in Settings, then record a command shortcut with Caps held. Shortcuts appear as a compact Hyper symbol plus the letter or arrow.

Create a snippet with a keyword, then enable automatic expansion. Keyword expansion uses Accessibility, resets when focus/security conditions change and pauses while Raycast is running to avoid duplicate input. Snippet text can include a cursor marker and supported template fields; see the [configuration reference](CONFIG.md).

## Input forms

A command with input fields opens a focused form. In multiline capture/text forms, Return saves/runs and Shift–Return adds a line. Command–Return also submits. Confirming composed text with an input method commits the text before submission.

## Dashboard and system monitor

Open Dashboard from search or the menu bar for local command usage, errors and recent timing summaries. Pause collection, reset it or export an aggregate diagnostic summary there.

System Monitor opens a separate native window for CPU, memory, disk, power and network information. Sampling stops when the monitor closes or minimizes.

[Full CLI/configuration reference](CONFIG.md) · [Security](../SECURITY.md)
