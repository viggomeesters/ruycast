//! Pure own-input fixture. Never observes real input or emits an OS event.
use rustcast_core::Command;
use ruycast_native::{
    keyword_matcher::{self, Catalog, ChoiceRequest, Matcher, Replacement},
    templates::Context,
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    io::{self, Read},
};
fn effects(replacement: &Replacement) -> Value {
    let operations = RefCell::new(vec![]);
    replacement.apply(
        |key| operations.borrow_mut().push(json!({"key":key})),
        |text| operations.borrow_mut().push(json!({"insert":text})),
    );
    json!({"eraseCount":replacement.erase_count(),"text":replacement.text(),"cursor":replacement.cursor(),"operations":operations.into_inner()})
}
fn run() -> Result<Value, String> {
    keyword_matcher::prepare_callback_thread();
    let mut bytes = vec![];
    io::stdin()
        .take(16_777_217)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err("Own keyword fixture too large".into());
    }
    let cases: Vec<Vec<Value>> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if cases.len() > 4096 || cases.iter().any(|c| c.len() > 16384) {
        return Err("Too many own keyword operations".into());
    }
    let string = |op: &Value, name: &str| {
        op[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Missing {name}"))
    };
    let mut output = vec![];
    for case in cases {
        let mut matcher = Matcher::default();
        let mut catalog = Catalog::new(&[])?;
        let mut choice: Option<ChoiceRequest> = None;
        let mut results = vec![];
        for op in case {
            let result = match op["op"].as_str() {
                Some("catalog") => {
                    let commands: Vec<Command> = serde_json::from_value(op["commands"].clone())
                        .map_err(|e| e.to_string())?;
                    catalog = Catalog::new(&commands)?;
                    json!({})
                }
                Some("type") => {
                    json!({"match":matcher.type_text(&string(&op,"text")?,&catalog).map(|c|c.id.clone())})
                }
                Some("backspace") => {
                    matcher.backspace();
                    json!({})
                }
                Some("reset") => {
                    matcher.reset();
                    json!({})
                }
                Some("replace") => effects(&Replacement::new(
                    &string(&op, "keyword")?,
                    &string(&op, "finalInput")?,
                    string(&op, "text")?,
                    op["cursor"].as_u64().map(|n| n as usize),
                )),
                Some("focus") => {
                    json!({"allowed":keyword_matcher::allows_expansion(op["secure"].as_bool().unwrap_or(false),op["subrole"].as_str())})
                }
                Some("classify-all") => {
                    let mut bits = vec![0u8; 0x110000_usize.div_ceil(8)];
                    let mut count = 0;
                    for value in 0..0x110000u32 {
                        if let Some(c) = char::from_u32(value) {
                            let mut buf = [0; 4];
                            if keyword_matcher::word_character(c.encode_utf8(&mut buf)) {
                                bits[value as usize / 8] |= 1 << (value % 8);
                                count += 1;
                            }
                        }
                    }
                    json!({"bits":bits.iter().map(|b|format!("{b:02x}")).collect::<String>(),"count":count})
                }
                Some("choice-start") => {
                    choice = Some(ChoiceRequest::new(
                        serde_json::from_value(op["command"].clone()).map_err(|e| e.to_string())?,
                        op["pid"].as_i64().ok_or("Missing pid")? as i32,
                    ));
                    json!({"active":true})
                }
                Some("choice-cancel") => {
                    let choice = choice.as_mut().ok_or("Choice missing")?;
                    choice.cancel();
                    json!({"active":choice.active()})
                }
                Some("choice") => {
                    let choice = choice.as_mut().ok_or("Choice missing")?;
                    let values: Vec<(String, String)> = serde_json::from_value(
                        op["values"]
                            .as_array()
                            .cloned()
                            .map(Value::Array)
                            .unwrap_or(json!([])),
                    )
                    .map_err(|e| e.to_string())?;
                    let context = Context {
                        clipboard: op["clipboard"].as_str(),
                        now: 0.,
                        locale: Some("en_US_POSIX"),
                        time_zone: Some("UTC"),
                    };
                    let result = choice.replacement(
                        &values,
                        op["pid"].as_i64().ok_or("Missing pid")? as i32,
                        op["safe"].as_bool().unwrap_or(false),
                        op["unchanged"].as_bool().unwrap_or(false),
                        &context,
                    );
                    match result {
                        Ok(r) => {
                            json!({"ok":true,"active":choice.active(),"replacement":effects(&r)})
                        }
                        Err(e) => json!({"ok":false,"active":choice.active(),"error":e}),
                    }
                }
                _ => return Err("Unknown own keyword operation".into()),
            };
            results.push(result);
        }
        output.push(results);
    }
    Ok(json!({"ok":true,"cases":output}))
}
fn main() {
    let result = objc2::rc::autoreleasepool(|_| run())
        .unwrap_or_else(|error| json!({"ok":false,"error":error}));
    println!("{result}");
}
