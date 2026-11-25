//! Isolated scripted provider; never accesses an operational vault.
use serde_json::json;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let id = args.get(3).cloned().unwrap_or_default();
    let token = std::env::var("VIGGOBSIDIAN_DIAGNOSTIC_ID").unwrap_or_default();
    let action = args.get(2).map(String::as_str).unwrap_or("");
    let body = json!({"ok":false,"error":"Synthetic error at 00-inbox/demo/note.md","detail":"Synthetic error at 00-inbox/demo/note.md","failureContext":{"schema":"ruycast.work-failure-context.v1","invocationID":token,"scope":"work","relativePath":"00-inbox/demo/note.md"}});
    assert_eq!(id, body["failureContext"]["invocationID"]);
    println!("{body}");
    if action != "convert-note" {
        std::process::exit(1);
    }
}
