//! Clipboard export normalization and optional import through the host writer.
use super::{error, raycast_cli};
use ruycast_native::clipboard_import;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.len() == 1 && args[0] == "--help" {
        println!("ruycast-build import-clipboard EXPORT [--max-text-bytes INTEGER] [--apply] [--socket PATH]\nDry run reports counts only. --apply sends bounded batches through the host clipboard writer.");
        return Ok(());
    }
    let report = process(args)?;
    println!("{report}");
    Ok(())
}

fn process(args: Vec<OsString>) -> Result<Value, String> {
    let mut args = args.into_iter();
    let export = args
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("Export path required.")?;
    let mut maximum = 100_000;
    let mut apply = false;
    let mut socket = None;
    while let Some(arg) = args.next() {
        if arg == "--apply" {
            apply = true;
            continue;
        }
        if arg == "--socket" {
            socket = Some(PathBuf::from(args.next().ok_or("Socket path required.")?));
            continue;
        }
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
    let parsed = clipboard_import::parse_export(&raw)
        .map_err(|_| "Invalid clipboard export JSON; source unchanged.")?;
    let normalized = clipboard_import::normalize_parsed(&parsed, maximum).map_err(error)?;
    let source_hash = format!("{:x}", Sha256::digest(&raw));
    let mut report = json!({
        "sourceSHA256": source_hash.clone(),
        "counts": normalized.counts,
        "candidates": normalized.items.len(),
        "applied": false,
        "policy": "Host settings determine retention, exclusions, size limits and history capacity.",
        "scope": "timezone-aware ISO normalization; references and image counts only"
    });
    if apply {
        let socket = match socket {
            Some(path) => path,
            None => default_socket()?,
        };
        let (batch_count, host_counts) = apply_batches(&normalized.items, &source_hash, &socket)?;
        report["applied"] = json!(true);
        report["batches"] = json!(batch_count);
        report["hostCounts"] = host_counts;
    }
    Ok(report)
}

fn default_socket() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config/ruycast/control.sock"))
        .ok_or_else(|| "Home directory unavailable; specify --socket.".into())
}

fn apply_batches(
    items: &[Value],
    source_hash: &str,
    socket: &Path,
) -> Result<(usize, Value), String> {
    let requests = clipboard_import::batches(items, source_hash).map_err(error)?;
    let mut host_counts = BTreeMap::<String, i64>::new();
    for request in &requests {
        let result = clipboard_import::send_request(socket, request).map_err(error)?;
        let counts = result
            .as_object()
            .ok_or("Ruycast clipboard import returned invalid host counts.")?;
        for (key, value) in counts {
            let Some(value) = value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            else {
                continue;
            };
            if key == "retained" {
                host_counts.insert(key.clone(), value);
            } else {
                let total = host_counts.entry(key.clone()).or_default();
                *total = total
                    .checked_add(value)
                    .ok_or("Ruycast clipboard import host counts overflowed.")?;
            }
        }
    }
    Ok((requests.len(), json!(host_counts)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::{fs::OpenOptionsExt, net::UnixListener},
        thread,
    };

    struct TestPath(PathBuf);

    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn test_path(extension: &str) -> TestPath {
        let root = std::env::var_os("RUYCAST_CLIPBOARD_SOCKET_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&root).unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        TestPath(root.join(format!("rc{}-{nonce}.{extension}", std::process::id())))
    }

    fn export(rows: usize) -> Vec<u8> {
        let rows: Vec<_> = (0..rows)
            .map(|index| {
                json!({
                    "category":"text",
                    "text":format!("synthetic-{index}"),
                    "applicationPath":"/Applications/Fixture.app",
                    "createdAt":"2026-10-25T00:00:00Z"
                })
            })
            .collect();
        serde_json::to_vec(&json!({
            "builtin_package_clipboardHistory":{"clipboardHistoryRecords":rows}
        }))
        .unwrap()
    }

    fn write_export(bytes: &[u8]) -> TestPath {
        let path = test_path("json");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path.0)
            .unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn dry_run_never_connects_and_keeps_the_export_unchanged() {
        let bytes = export(1);
        let source = write_export(&bytes);
        let missing_socket = test_path("missing.sock");
        let report = process(vec![
            source.0.clone().into_os_string(),
            "--socket".into(),
            missing_socket.0.clone().into_os_string(),
        ])
        .unwrap();
        assert_eq!(report["applied"], false);
        assert_eq!(report["candidates"], 1);
        assert_eq!(fs::read(&source.0).unwrap(), bytes);
    }

    #[test]
    fn apply_sends_bounded_batches_and_aggregates_host_counts() {
        let bytes = export(101);
        let source = write_export(&bytes);
        let socket = test_path("sock");
        let listener = UnixListener::bind(&socket.0).unwrap();
        let server = thread::spawn(move || {
            for (index, expected_items) in [100, 1].into_iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                assert_eq!(request["op"], "clipboard.import");
                assert_eq!(request["items"].as_array().unwrap().len(), expected_items);
                assert_eq!(request["sourceSHA256"].as_str().unwrap().len(), 64);
                assert_eq!(request["batchID"].as_str().unwrap().len(), 64);
                let reply = json!({"ok":true,"result":{
                    "accepted":if index == 0 { 3 } else { 2 },
                    "retained":if index == 0 { 50 } else { 1 },
                    "ignored":true
                }});
                stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
            }
        });
        let report = process(vec![
            source.0.clone().into_os_string(),
            "--apply".into(),
            "--socket".into(),
            socket.0.clone().into_os_string(),
        ])
        .unwrap();
        server.join().unwrap();
        assert_eq!(report["applied"], true);
        assert_eq!(report["batches"], 2);
        assert_eq!(report["hostCounts"], json!({"accepted":5,"retained":1}));
        assert_eq!(fs::read(&source.0).unwrap(), bytes);
    }
}
