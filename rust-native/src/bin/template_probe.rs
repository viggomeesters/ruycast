//! Test-only pure template evaluation; no UI, clipboard reads or OS effects.
use rustcast_core::Command;
use ruycast_native::templates::{Context, Limits, Plan};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, Read};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    command: Command,
    #[serde(default)]
    values: Vec<(String, String)>,
    clipboard: Option<String>,
    now: f64,
    locale: Option<String>,
    time_zone: Option<String>,
    limits: Option<Limits>,
}
fn run() -> Result<serde_json::Value, String> {
    let mut bytes = vec![];
    io::stdin()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("Fixture input exceeds limit.".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if cases.len() > 2048 {
        return Err("Too many fixture cases.".into());
    }
    let output: Vec<_> = cases.into_iter().map(|case| {
        let context = Context { clipboard: case.clipboard.as_deref(), now: case.now, locale: case.locale.as_deref(), time_zone: case.time_zone.as_deref() };
        match Plan::parse(&case.command, case.limits.unwrap_or_else(Limits::application)) {
            Err(error) => json!({"ok":false,"error":error}),
            Ok(plan) => {
                let rendered = match plan.render(&case.values, &context) {
                    Ok((text, cursor)) => json!({"ok":true,"text":text,"cursor":cursor}),
                    Err(error) => json!({"ok":false,"error":error}),
                };
                json!({"ok":true,"fields":plan.fields(),"needsInteraction":plan.needs_interaction(),"needsClipboard":ruycast_native::templates::needs_clipboard(&case.command.target),"render":rendered})
            }
        }
    }).collect();
    Ok(json!({"ok":true,"cases":output,"pureTemplateFixture":true}))
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
