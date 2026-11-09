//! User-facing labels shared by the native launcher and its action surfaces.
use rustcast_core::Command;

/// Display-only icon authority. Never carries snippet bodies, script arguments,
/// shortcut payloads, full quicklink URLs, or clipboard content.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    Symbol(&'static str),
    Window(String),
    Application(String),
    Path(String),
    Website {
        origin: String,
        application: Option<String>,
    },
}
pub fn icon(command: &Command) -> Icon {
    if apple_shortcut(command) {
        return Icon::Symbol("square.stack.3d.up.fill");
    }
    for (prefix, symbol) in [
        ("system.tab:", "safari"),
        ("system.window:", "macwindow"),
        ("system.menu:", "menubar.rectangle"),
        ("system.kill:", "cpu"),
    ] {
        if command.target.starts_with(prefix) {
            return Icon::Symbol(symbol);
        }
    }
    match command.kind.as_str() {
        "app" => Icon::Application(command.target.clone()),
        "path" => Icon::Path(command.target.clone()),
        "window" if rustcast_core::LAYOUTS.contains(&command.target.as_str()) => {
            Icon::Window(command.target.clone())
        }
        "url" => website_origin(&command.target)
            .map(|origin| Icon::Website {
                origin,
                application: command.application_id.clone(),
            })
            .unwrap_or(Icon::Symbol("link")),
        "snippet" => Icon::Symbol("text.quote"),
        "script" => Icon::Symbol("terminal"),
        "builtin" => Icon::Symbol("doc.on.clipboard"),
        _ => Icon::Symbol("command"),
    }
}
pub fn website_origin(raw: &str) -> Option<String> {
    use objc2_foundation::{NSString, NSURL};
    let url = NSURL::URLWithString(&NSString::from_str(raw))?;
    let scheme = url.scheme()?.to_string().to_lowercase();
    let host = url.host()?.to_string().to_lowercase();
    if !["http", "https"].contains(&scheme.as_str())
        || host.is_empty()
        || host.contains(['{', '}', '%', ' '])
    {
        return None;
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    let port = url.port().map(|p| format!(":{}", p)).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}/"))
}
pub fn palette_frame(visible: [f64; 4], clipboard: bool) -> [f64; 4] {
    let [x, y, w, h] = visible;
    let inset = (w.min(h) / 12.).clamp(0., 24.);
    let width = (if clipboard { 1080_f64 } else { 760_f64 }).min((w - 2. * inset).max(1.));
    let height = (if clipboard { 700_f64 } else { 490_f64 }).min((h - 2. * inset).max(1.));
    [x + (w - width) / 2., y + (h - height) / 2., width, height]
}
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "snippet" => "Snippet",
        "app" => "Application",
        "url" => "Quicklink",
        "path" => "File or Folder",
        "window" => "Window",
        _ => "Command",
    }
}
pub fn action_title(command: &Command) -> &'static str {
    for (prefix, label) in [
        ("system.kill:", "Kill"),
        ("system.menu:", "Run"),
        ("system.tab:", "Switch to Tab"),
        ("system.window:", "Switch to Window"),
    ] {
        if command.target.starts_with(prefix) {
            return label;
        }
    }
    if apple_shortcut(command) {
        return "Run Shortcut";
    }
    if command.kind == "snippet" {
        return "Insert Snippet";
    }
    if command.id.starts_with("capture.") {
        return "Save…";
    }
    if command.kind == "script" || command.kind == "window" {
        "Run"
    } else {
        "Open"
    }
}
fn apple_shortcut(command: &Command) -> bool {
    let Some(id) = command.id.strip_prefix("apple-shortcut.") else {
        return false;
    };
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
        && command.kind == "script"
        && command.target == "/usr/bin/shortcuts"
        && command.arguments == ["run", id]
}
pub fn shortcut(value: &str) -> (String, String) {
    let lower = value.to_lowercase();
    let parts: Vec<_> = lower.split('+').collect();
    let key = parts.last().copied().unwrap_or("");
    let modifiers = &parts[..parts.len().saturating_sub(1)];
    let full = modifiers.len() == 4
        && ["ctrl", "alt", "shift", "cmd"]
            .iter()
            .all(|m| modifiers.contains(m));
    let solo = (key == "left-control" && modifiers == ["ctrl"])
        || (key == "left-option" && modifiers == ["alt"]);
    let mut symbols = Vec::new();
    let mut names = Vec::new();
    if !solo {
        if full {
            symbols.push("✦");
        } else {
            for (modifier, symbol, name) in [
                ("ctrl", "⌃", "Control"),
                ("alt", "⌥", "Option"),
                ("shift", "⇧", "Shift"),
                ("cmd", "⌘", "Command"),
                ("left-cmd", "L⌘", "Left Command"),
                ("right-cmd", "R⌘", "Right Command"),
            ] {
                if modifiers.contains(&modifier) {
                    symbols.push(symbol);
                    names.push(name);
                }
            }
        }
    }
    let special = match key {
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "return" => "↵",
        "space" => "␣",
        "tab" => "⇥",
        "escape" => "⎋",
        "delete" | "backspace" => "⌫",
        "left-control" => "⌃",
        "left-option" => "⌥",
        _ => "",
    };
    let key_symbol = if special.is_empty() {
        key.to_uppercase()
    } else {
        special.to_string()
    };
    let display = if symbols.is_empty() {
        key_symbol
    } else {
        format!("{} {key_symbol}", symbols.join(" "))
    };
    let label = if full {
        format!("Hyper + {key}")
    } else if solo {
        format!(
            "Left {}",
            if key == "left-control" {
                "Control"
            } else {
                "Option"
            }
        )
    } else {
        names.push(key);
        names.join(" + ")
    };
    (display, label)
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn original_compact_keycaps_and_sided_accessibility_labels() {
        for (input, display) in [
            ("shift+ctrl+alt+cmd+left", "✦ ←"),
            ("cmd+alt+n", "⌥ ⌘ N"),
            ("ctrl+left-control", "⌃"),
            ("ctrl+alt+delete", "⌃ ⌥ ⌫"),
            ("right-cmd+a", "R⌘ A"),
        ] {
            assert_eq!(shortcut(input).0, display, "{input}");
        }
        assert_eq!(shortcut("left-cmd+a").1, "Left Command + a");
    }
    #[test]
    fn icons_never_carry_nonvisual_payloads() {
        for kind in ["snippet", "script", "builtin"] {
            let command:Command=serde_json::from_value(json!({"id":"fixture","title":"Visible name","kind":kind,"target":"PRIVATE-PAYLOAD","arguments":["PRIVATE-ARGUMENT"],"aliases":[]})).unwrap();
            let icon = icon(&command);
            assert!(matches!(icon, Icon::Symbol(_)));
            assert!(!format!("{icon:?}").contains("PRIVATE"));
        }
        let command:Command=serde_json::from_value(json!({"id":"fixture","title":"Visible link","kind":"url","target":"https://user:secret@example.test/private?q=secret#private","aliases":[]})).unwrap();
        assert_eq!(
            icon(&command),
            Icon::Website {
                origin: "https://example.test/".into(),
                application: None
            }
        );
    }
    #[test]
    fn original_geometry_on_negative_and_small_screens() {
        assert_eq!(
            palette_frame([0., 0., 1440., 900.], false),
            [340., 205., 760., 490.]
        );
        assert_eq!(
            palette_frame([-1440., -900., 1440., 900.], true),
            [-1260., -800., 1080., 700.]
        );
        assert_eq!(
            palette_frame([0., 0., 640., 480.], true),
            [24., 24., 592., 432.]
        );
        assert_eq!(palette_frame([0., 0., 1., 1.], false), [0., 0., 1., 1.]);
    }
}
