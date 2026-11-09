use ruycast_native::analytics::{Calendar, Writer};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
fn main() {
    let result = (|| -> Result<Value, String> {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        let root = PathBuf::from(v["root"].as_str().ok_or("Own root required")?);
        if !root.join(".ruycast-analytics-fixture").is_file() {
            return Err("Own analytics fixture required".into());
        }
        let clock = Arc::new(AtomicU64::new(
            v["now"].as_f64().unwrap_or(1_790_000_000.).to_bits(),
        ));
        let time = clock.clone();
        let writer = Writer::open_with(
            &root,
            None,
            Arc::new(move || f64::from_bits(time.load(Ordering::Acquire))),
            Calendar::named(
                v["calendar"].as_str().unwrap_or("gregorian"),
                v["zone"].as_str().unwrap_or("UTC"),
            ),
            Duration::from_secs(60),
        )?;
        let client = writer.client();
        let mut tickets = HashMap::new();
        let mut results = vec![];
        for a in v["actions"].as_array().ok_or("Actions required")? {
            if let Some(now) = a["now"].as_f64() {
                clock.store(now.to_bits(), Ordering::Release);
            }
            match a["op"].as_str().unwrap_or("") {
                "attempt" => {
                    let ticket = client.attempt(
                        a["id"].as_str().unwrap_or("own"),
                        a["source"].as_str().unwrap_or("cli"),
                    );
                    if let Some(ticket) = ticket {
                        tickets.insert(a["ticket"].as_str().unwrap_or("t").to_string(), ticket);
                    }
                }
                "complete" => {
                    if let Some(ticket) = tickets.remove(a["ticket"].as_str().unwrap_or("t")) {
                        client.complete(
                            ticket,
                            a["ok"].as_bool().unwrap_or(false),
                            a["ms"].as_f64().unwrap_or(0.),
                            a["code"].as_str().unwrap_or("action-failed"),
                            a["exit"].as_i64(),
                        );
                    }
                }
                "timing" => client.timing(
                    a["name"].as_str().unwrap_or(""),
                    a["ms"].as_f64().unwrap_or(-1.),
                ),
                "error" => client.error(
                    a["category"].as_str().unwrap_or(""),
                    a["code"].as_str().unwrap_or(""),
                ),
                "configure" => {
                    client.configure(ruycast_native::analytics::request_bool(&a["enabled"]), ruycast_native::analytics::request_int(&a["retentionDays"]))?
                }
                "reset" => client.reset(),
                "import" => client.import_legacy(
                    &serde_json::from_value(a["counts"].clone()).map_err(|e| e.to_string())?,
                ),
                "snapshot" => results.push(
                    json!({"document":client.snapshot(),"storageError":client.storage_error()}),
                ),
                "export" => results.push(client.snapshot().export("fixture")),
                "dashboard" => {
                    let commands:Vec<rustcast_core::Command>=serde_json::from_value(a["commands"].clone()).map_err(|e|e.to_string())?;
                    let (document,calendar,error,_)=client.dashboard_snapshot();
                    results.push(serde_json::to_value(ruycast_native::analytics_dashboard::Surface::build(&document,&calendar,&commands,f64::from_bits(clock.load(Ordering::Acquire)),1,error)).map_err(|e|e.to_string())?);
                }
                "hourly" => results.push(json!(client
                    .snapshot()
                    .hourly_activity(f64::from_bits(clock.load(Ordering::Acquire))))),
                "casts" => results.push(json!({"bool":ruycast_native::analytics::request_bool(&a["value"]),"int":ruycast_native::analytics::request_int(&a["value"])})),
                "flush" => client.flush(),
                _ => return Err("Unknown action".into()),
            }
        }
        drop(writer);
        Ok(
            json!({"ok":true,"results":results,"document":client.snapshot(),"storageError":client.storage_error()}),
        )
    })();
    println!(
        "{}",
        result.unwrap_or_else(|e| json!({"ok":false,"error":e}))
    );
}
