# Ruycast

A native keyboard launcher for your Mac. Open apps, run your commands and make it yours through a CLI and readable files.

![Ruycast's native launcher and Settings](assets/hero.png)

**Source preview · macOS 14+ · Rust + Swift**

## Your everyday shortcuts

- **Launch anything.** Apps, websites, folders and scripts, with your own aliases and hotkeys.
- **Type less.** Snippets with automatic keyword expansion, parameterized quicklinks and Caps Lock as Hyper.
- **Keep useful copies.** Search text, images and file references in clipboard history, with previews and retention controls.
- **Find app actions.** Search the active app’s menus, see their shortcuts and run available actions.
- **Find Safari tabs.** See open tabs first when you open the launcher, or search by title or address alongside apps and commands.
- **Find open windows.** Search by document or app name and switch to the selected existing window.
- **Arrange your windows.** Halves, thirds, maximize, restore and move between displays.
- **Stay in control.** Native Settings, a system monitor, and a local Dashboard for usage, errors and timings.
- **Work with your AI tools.** Coding agents can inspect status, edit configuration and run actions in the background. Configuration lives in files you can read and version.

## Install

Build from source with macOS command-line developer tools, Rust, Swift and Python 3.10+ installed:

```sh
git clone https://github.com/viggomeesters/ruycast.git
cd ruycast
make install
```

This installs `Ruycast.app`, adds `ruycast` to `~/.local/bin` and starts the menu bar app at login. The installer preserves your profile and reuses a stable local signing identity on updates.

Allow Ruycast in **System Settings → Privacy & Security → Accessibility** for window management, Hyper, snippet expansion and pasting into other apps. Enable Hyper and snippet expansion in Settings when you want them.

[Build requirements and signing](docs/INSTALLATION.md) · [Getting started](docs/GETTING-STARTED.md)

## Usage

Choose your theme and menu bar visibility in Settings → General. Enable **Separate Left and Right Command** there to assign different app shortcuts to `L⌘` and `R⌘`.

Press **Control–Option–Space**, type an app name or alias, then Return. Open clipboard history with **Control–Option–V**. Command–K opens actions; arrow keys navigate them.

For a quicklink with one input, type its alias followed by a space and your query, then Return.

Create quicklinks in Settings or through the CLI:

```sh
ruycast config add '{"id":"link.project-docs","title":"Project Docs","kind":"url","target":"https://example.com/docs","aliases":["projectdocs"]}'
ruycast search projectdocs
ruycast run projectdocs --dry-run
ruycast run projectdocs
```

Your configuration defaults to `~/.config/ruycast/config.json`. Choose another folder in Settings → Advanced or with `ruycast config directory /absolute/folder`. Your choice persists across app updates; clipboard history and runtime files stay local. File edits reload automatically; an invalid edit keeps the last working configuration.

[Configuration and CLI reference](docs/CONFIG.md) · [Architecture](docs/ARCHITECTURE.md)

## Privacy and security

Clipboard history and analytics stay on your Mac. Analytics records counts, durations and fixed error categories; it excludes typed queries, clipboard contents and script input. Pause or reset it in Dashboard, and export an aggregate summary when needed.

Website icons contact the relevant websites and use a local cache. Only add scripts and command definitions you trust.

[Security and data storage](SECURITY.md) · [Analytics](docs/ANALYTICS.md) · [Command errors](docs/DIAGNOSTICS.md)

## Development

```sh
make check
bash scripts/check-settings.sh
bash scripts/check-performance.sh
```

Validation runs locally. Native checks use hidden windows, isolated profiles and fake destructive effects. They do not operate your desktop.

[Contributing](CONTRIBUTING.md) · [Performance](docs/PERFORMANCE.md) · [Product contract](docs/vision.json)

## Releases

[Download the 0.8.19 source release](https://github.com/viggomeesters/ruycast/releases/tag/v0.8.19) or clone this repository and build locally. This source preview has no prebuilt notarized app. See the [release status](docs/RELEASING.md) for supported installation and distribution details.

[Changelog](CHANGELOG.md)

## License

[MIT](LICENSE). Ruycast is an independent project. It is not affiliated with Raycast.
