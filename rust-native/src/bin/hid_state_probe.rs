//! Data-only fixture for transient physical state, never installs a hook.
use ruycast_native::keyboard_state::{
    rewritten_flags, Binding, CommandFilter, Environment, Event, Physical,
};
use serde_json::{json, Value};
use std::io::{self, Read};
fn run() -> Result<Value, String> {
    let mut bytes = vec![];
    io::stdin()
        .take(16_777_217)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err("Own state fixture exceeds input limit".into());
    }
    let cases: Vec<Vec<Value>> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if cases.len() > 4096 || cases.iter().any(|c| c.len() > 16384) {
        return Err("Too many own state fixture events".into());
    }
    let number = |v: &Value, k: &str| v[k].as_u64().ok_or_else(|| format!("Missing {k}"));
    let mut results = vec![];
    for case in cases {
        let mut state = Physical::default();
        let mut filter = CommandFilter::default();
        let mut steps = vec![];
        for op in case {
            let mut tap = None;
            let mut action = None;
            let mut consumed = false;
            let mut flags = None;
            let mut readiness = None;
            match op["op"].as_str() {
                Some("input") => {
                    let time = match op["time"].as_str() {
                        Some("nan") => f64::NAN,
                        Some("inf") => f64::INFINITY,
                        Some("-inf") => f64::NEG_INFINITY,
                        _ => op["time"].as_f64().ok_or("Missing time")?,
                    };
                    tap = state.input(
                        number(&op, "device")?,
                        u32::try_from(number(&op, "usage")?).map_err(|_| "Invalid usage")?,
                        op["down"].as_bool().ok_or("Missing down")?,
                        time,
                    );
                }
                Some("remove") => state.remove(number(&op, "device")?),
                Some("reset") => {
                    state.reset();
                    filter.reset();
                }
                Some("filter-reset") => filter.reset(),
                Some("cancel") => state.cancel_tap(),
                Some("bindings") => {
                    let bindings = op["bindings"]
                        .as_array()
                        .ok_or("Missing bindings")?
                        .iter()
                        .map(|b| {
                            Binding::parse(
                                b["shortcut"].as_str().ok_or("Missing shortcut")?,
                                b["action"].as_str().ok_or("Missing action")?.into(),
                            )
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    filter.set_bindings(bindings);
                }
                Some("event") => {
                    let kind = match number(&op, "kind")? {
                        10 => Event::Down,
                        11 => Event::Up,
                        _ => Event::Other,
                    };
                    let decision = filter.handle(
                        kind,
                        u16::try_from(number(&op, "key")?).map_err(|_| "Invalid code")?,
                        number(&op, "flags")?,
                        op["repeat"].as_bool().unwrap_or(false),
                    );
                    action = decision.action;
                    consumed = decision.consumed;
                }
                Some("rewrite") => {
                    flags = Some(rewritten_flags(
                        number(&op, "flags")?,
                        op["held"].as_bool().ok_or("Missing held")?,
                        op["tag"].as_i64().ok_or("Missing tag")?,
                    ))
                }
                Some("readiness") => {
                    let b = |k: &str| op[k].as_bool().ok_or_else(|| format!("Missing {k}"));
                    readiness = Environment {
                        sleeping: b("sleeping")?,
                        raycast_running: b("raycast")?,
                        secure_input: b("secure")?,
                        accessibility: b("accessibility")?,
                        input_monitoring: b("monitoring")?,
                    }
                    .blocked(b("hyper")?, b("modifiers")?, b("sided")?);
                }
                _ => return Err("Unknown own state event".into()),
            }
            let held = state
                .held()
                .iter()
                .map(|(device, usages)| json!([device, usages]))
                .collect::<Vec<_>>();
            steps.push(json!({"tap":tap,"action":action,"consumed":consumed,"flags":flags,"readiness":readiness,"caps":state.caps_held(),"modifiers":state.modifiers(),"usages":state.usages(),"held":held}));
        }
        results.push(steps);
    }
    Ok(json!({"ok":true,"cases":results}))
}
fn main() {
    match run() {
        Ok(v) => println!("{v}"),
        Err(e) => {
            println!("{}", json!({"ok":false,"error":e}));
            std::process::exit(1);
        }
    }
}
