//! Read-only export normalization. No host, socket or history writer.
use super::{error, raycast_cli};
use ruycast_native::clipboard_import;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{ffi::OsString, path::PathBuf};

pub(super) fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.len() == 1 && args[0] == "--help" {
        println!("ruycast-build import-clipboard EXPORT [--max-text-bytes INTEGER]\nRead-only normalization with RFC3339 dates. Files remain references; images are counted without reading pixels. Apply is not migrated.");
        return Ok(());
    }
    let mut args = args.into_iter();
    let export = args
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("Export path required.")?;
    let mut maximum = 100_000;
    while let Some(arg) = args.next() {
        if arg != "--max-text-bytes" {
            return Err("Unsupported clipboard import option; source unchanged.".into());
        }
        maximum = args
            .next()
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or("Integer text limit required.")?;
    }
    let raw = raycast_cli::input(&PathBuf::from(export))
        .map_err(|_| "Clipboard import input unavailable; source unchanged.")?;
    let data: Value = serde_json::from_slice(&raw)
        .map_err(|_| "Invalid clipboard export JSON; source unchanged.")?;
    let normalized = clipboard_import::normalize(&data, maximum).map_err(error)?;
    println!(
        "{}",
        json!({"sourceSHA256":format!("{:x}",Sha256::digest(&raw)),"counts":normalized.counts,"candidates":normalized.items.len(),"applied":false,"scope":"RFC3339 normalization; references and image counts only"})
    );
    Ok(())
}
