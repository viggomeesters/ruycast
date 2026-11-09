//! Own data only; no AppKit application, clipboard, hooks or target effects.
use ruycast_native::{
    snippet_choice::{Action, Key, Model, Prepared},
    templates::Context,
};
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    sync::Arc,
};
fn main() {
    let result =
        objc2::rc::autoreleasepool(|_| run()).unwrap_or_else(|e| json!({"ok":false,"error":e}));
    println!("{result}");
}
fn run() -> Result<Value, String> {
    let mut input = vec![];
    io::stdin()
        .take(16_777_217)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > 16_777_216 {
        return Err("Own choice input too large".into());
    }
    let cases: Vec<Vec<Value>> = serde_json::from_slice(&input).map_err(|e| e.to_string())?;
    let mut results = vec![];
    for ops in cases {
        let mut model: Option<Model> = None;
        let mut submit = 0;
        let mut cancel = 0;
        let mut steps = vec![];
        for op in ops {
            if op["op"].as_str() == Some("policy") {
                steps.push(ruycast_native::snippet_choice_request::fixture(&op)?);
                continue;
            }
            if op["op"].as_str() == Some("equal") {
                let left = serde_json::from_value(op["left"].clone()).map_err(|e| e.to_string())?;
                let right =
                    serde_json::from_value(op["right"].clone()).map_err(|e| e.to_string())?;
                steps.push(json!({"equal":ruycast_native::command_identity::Identity::new(&left)==ruycast_native::command_identity::Identity::new(&right)}));
                continue;
            }
            let mut handled = None;
            match op["op"].as_str() {
                Some("start") => {
                    let command =
                        serde_json::from_value(op["command"].clone()).map_err(|e| e.to_string())?;
                    model = Some(Model::new(Arc::new(Prepared::new(command)?)));
                    submit = 0;
                    cancel = 0;
                }
                Some("set") => {
                    let m = model.as_mut().ok_or("Model missing")?;
                    m.set(
                        op["field"].as_u64().unwrap_or(0) as usize,
                        op["value"].as_str().unwrap_or("").into(),
                    );
                }
                Some("focus") => model
                    .as_mut()
                    .ok_or("Model missing")?
                    .focus(op["field"].as_u64().unwrap_or(0) as usize),
                Some("key") => {
                    let m = model.as_mut().ok_or("Model missing")?;
                    let a = m.handle(Key {
                        code: op["code"].as_u64().unwrap_or(36) as u16,
                        modifiers: op["flags"].as_u64().unwrap_or(0),
                        repeat: op["repeat"].as_bool().unwrap_or(false),
                        editing_text: op["editingText"].as_bool().unwrap_or(false),
                        composing: op["composing"].as_bool().unwrap_or(false),
                    });
                    handled = Some(a != Action::Unhandled);
                    match a {
                        Action::Submit => {
                            if m.submit(&Context::current(None)).ok().flatten().is_some() {
                                submit += 1;
                            }
                        }
                        Action::Cancel => {
                            if m.cancel() {
                                cancel += 1;
                            }
                        }
                        _ => {}
                    }
                }
                Some("submit") => {
                    if model
                        .as_mut()
                        .ok_or("Model missing")?
                        .submit(&Context::current(None))
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        submit += 1;
                    }
                }
                Some("cancel") => {
                    if model.as_mut().ok_or("Model missing")?.cancel() {
                        cancel += 1;
                    }
                }
                Some("read") => {}
                _ => return Err("Unknown own choice action".into()),
            }
            let m = model.as_ref().ok_or("Model missing")?;
            let values = m
                .prepared()
                .fields()
                .iter()
                .zip(m.values())
                .map(|(f, v)| (f.id.clone(), json!(v)))
                .collect::<serde_json::Map<_, _>>();
            steps.push(json!({"fields":m.prepared().fields(),"values":values,"active":m.active().map(|i|&m.prepared().fields()[i].id),"canSubmit":m.can_submit(),"finished":m.finished(),"submitted":submit,"cancelled":cancel,"handled":handled}));
        }
        results.push(steps);
    }
    Ok(json!({"ok":true,"cases":results}))
}
