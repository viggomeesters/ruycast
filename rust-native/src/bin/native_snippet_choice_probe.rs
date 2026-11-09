//! Explicit own directory + complete clipboard-abortguard required. Never
//! permits window ordering, global input or app activation.
use serde_json::json;
use std::io::{self, Read};
fn main() {
    let result = (|| {
        let root = std::env::args()
            .nth(1)
            .ok_or("Own fixture directory required")?;
        let root = std::path::Path::new(&root);
        if std::fs::read(root.join(".ruycast-choice-fixture"))
            .ok()
            .as_deref()
            != Some(b"own-choice-v1")
        {
            return Err("Own fixture stamp required".to_owned());
        }
        let mut bytes = vec![];
        io::stdin()
            .take(16_777_217)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 16_777_216 {
            return Err("Own fixture too large".into());
        }
        let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        ruycast_native::snippet_choice_window::fixture(root, value)
    })()
    .unwrap_or_else(|error: String| json!({"ok":false,"error":error}));
    println!("{result}");
}
