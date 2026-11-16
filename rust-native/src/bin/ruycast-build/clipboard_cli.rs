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
        println!("ruycast-build import-clipboard [EXPORT] [--max-text-bytes INTEGER] [--apply] [--socket PATH]\nEXPORT defaults to ~/.local/state/raycast-export-trial-20261020/post-import-20261021-083502.json. Dry run reports counts only. --apply sends bounded batches through the host clipboard writer.");
        return Ok(());
    }
    let report = process(args)?;
    println!("{report}");
    Ok(())
}

fn process(args: Vec<OsString>) -> Result<Value, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| objc2_foundation::NSHomeDirectory().to_string().into());
    let options = parse_options(args, Some(&home))?;
    let raw = raycast_cli::input(&options.export)
        .map_err(|_| "Clipboard import input unavailable; source unchanged.")?;
    let parsed = clipboard_import::parse_export(&raw)
        .map_err(|_| "Invalid clipboard export JSON; source unchanged.")?;
    let normalized = clipboard_import::normalize_parsed(&parsed, options.maximum).map_err(error)?;
    let source_hash = format!("{:x}", Sha256::digest(&raw));
    let mut report = json!({
        "sourceSHA256": source_hash.clone(),
        "counts": normalized.counts,
        "candidates": normalized.items.len(),
        "applied": false,
        "policy": "Host settings determine retention, exclusions, size limits and history capacity.",
        "scope": "timezone-aware ISO normalization; references and image counts only"
    });
    if options.apply {
        let socket = match options.socket {
            Some(path) => path,
            None => default_socket_for(&home),
        };
        let (batch_count, host_counts) = apply_batches(&normalized.items, &source_hash, &socket)?;
        report["applied"] = json!(true);
        report["batches"] = json!(batch_count);
        report["hostCounts"] = host_counts;
    }
    Ok(report)
}

struct Options {
    export: PathBuf,
    maximum: i64,
    apply: bool,
    socket: Option<PathBuf>,
}

fn parse_options(args: Vec<OsString>, home: Option<&Path>) -> Result<Options, String> {
    let mut args = args.into_iter();
    let mut export = None;
    let mut maximum = 100_000;
    let mut apply = false;
    let mut socket = None;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && arg == "--apply" {
            apply = true;
            continue;
        }
        if !positional && arg == "--socket" {
            let value = args
                .next()
                .filter(|value| !value.as_encoded_bytes().starts_with(b"-"));
            socket = Some(PathBuf::from(value.ok_or("Socket path required.")?));
            continue;
        }
        if !positional && arg == "--max-text-bytes" {
            maximum = args
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse::<i64>().ok())
                .ok_or("Integer text limit required.")?;
            continue;
        }
        if !positional {
            if let Some(value) = arg
                .to_str()
                .and_then(|value| value.strip_prefix("--socket="))
            {
                socket = Some(PathBuf::from(value));
                continue;
            }
            if let Some(value) = arg
                .to_str()
                .and_then(|value| value.strip_prefix("--max-text-bytes="))
            {
                maximum = value
                    .parse::<i64>()
                    .map_err(|_| "Integer text limit required.")?;
                continue;
            }
            if arg.as_encoded_bytes().starts_with(b"-") && !raycast_cli::negative_number(&arg) {
                return Err("Unsupported clipboard import option; source unchanged.".into());
            }
        }
        if export.is_some() {
            return Err("Only one clipboard export path is supported.".into());
        }
        export = Some(PathBuf::from(arg));
    }
    let export = match export {
        Some(path) => path,
        None => default_export_for(home.ok_or("Home directory unavailable; specify EXPORT.")?),
    };
    Ok(Options {
        export,
        maximum,
        apply,
        socket,
    })
}

fn default_export_for(home: &Path) -> PathBuf {
    home.join(".local/state/raycast-export-trial-20261020/post-import-20261021-083502.json")
}

fn default_socket_for(home: &Path) -> PathBuf {
    home.join(".config/ruycast/control.sock")
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
    fn optional_export_matches_python_default_and_option_order() {
        let home = Path::new("/fixture/home");
        let options = parse_options(
            vec![
                "--apply".into(),
                "--max-text-bytes=-7".into(),
                "--socket=/fixture/control.sock".into(),
                "/fixture/export.json".into(),
            ],
            Some(home),
        )
        .unwrap();
        assert_eq!(options.export, PathBuf::from("/fixture/export.json"));
        assert_eq!(options.maximum, -7);
        assert!(options.apply);
        assert_eq!(options.socket, Some(PathBuf::from("/fixture/control.sock")));

        let defaulted = parse_options(vec!["--apply".into()], Some(home)).unwrap();
        assert_eq!(
            defaulted.export,
            PathBuf::from("/fixture/home/.local/state/raycast-export-trial-20261020/post-import-20261021-083502.json")
        );
        assert_eq!(
            default_export_for(home),
            defaulted.export,
            "default path remains home-relative"
        );
        assert_eq!(
            default_socket_for(home),
            PathBuf::from("/fixture/home/.config/ruycast/control.sock")
        );

        let explicit_without_home = parse_options(
            vec![
                "/fixture/export.json".into(),
                "--apply".into(),
                "--socket".into(),
                "/fixture/control.sock".into(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            explicit_without_home.socket,
            Some(PathBuf::from("/fixture/control.sock"))
        );

        let negative_path = parse_options(vec!["-1".into()], Some(home)).unwrap();
        assert_eq!(negative_path.export, PathBuf::from("-1"));
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
