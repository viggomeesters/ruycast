//! Rust dry-run I/O. Apply remains explicitly refused until its CAS writer lands.
use super::*;
use objc2_foundation::{NSBundle, NSHomeDirectory, NSString, NSURL};
use ruycast_native::raycast_import::{self, Application, Options as Mapping};
use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    time::Duration,
};
const LIMIT: u64 = 256 * 1024 * 1024;
const HELP:&str="ruycast-build import-raycast EXPORT [--config FILE] [--cli PATH] [--include-snippets] [--include-templates]\nDry run only; configuration is never written.";
struct Options {
    export: PathBuf,
    config: Option<PathBuf>,
    cli: PathBuf,
    mapping: Mapping,
    apply: bool,
}
pub(super) struct Failure {
    pub code: i32,
    pub message: String,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self { code: 1, message }
    }
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| NSHomeDirectory().to_string().into())
}
fn parse(args: Vec<OsString>) -> std::result::Result<Option<Options>, String> {
    let names = [
        "--config",
        "--cli",
        "--apply",
        "--include-snippets",
        "--include-templates",
        "--help",
    ];
    let mut export = None;
    let mut config = None;
    let mut cli = home().join(".local/bin/ruycast");
    let mut mapping = Mapping::default();
    let mut apply = false;
    let mut args = args.into_iter();
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional && arg == OsStr::new("--") {
            positional = true;
            continue;
        }
        if !positional && (arg == OsStr::new("-h") || arg == OsStr::new("--help")) {
            return Ok(None);
        }
        if !positional && arg.as_encoded_bytes().starts_with(b"-") {
            let text = arg.to_str().ok_or("Unrecognized non-UTF8 option.")?;
            let (key, inline) = text
                .split_once('=')
                .map_or((text, None), |(k, v)| (k, Some(OsString::from(v))));
            let matches = names
                .iter()
                .filter(|name| **name == key || name.starts_with(key))
                .copied()
                .collect::<Vec<_>>();
            let key = if names.contains(&key) {
                key
            } else if matches.len() == 1 {
                matches[0]
            } else {
                return Err(format!("Unrecognized or ambiguous option: {key}"));
            };
            match key {
                "--config" | "--cli" => {
                    let value = if let Some(v) = inline {
                        v
                    } else {
                        let v = args
                            .next()
                            .ok_or_else(|| format!("{key} requires a path."))?;
                        if v.as_encoded_bytes().starts_with(b"-") {
                            return Err(format!("{key} requires a path."));
                        }
                        v
                    };
                    if key == "--config" {
                        config = Some(value.into())
                    } else {
                        cli = value.into()
                    }
                }
                _ => {
                    if inline.is_some() {
                        return Err(format!("{key} does not take a value."));
                    }
                    match key {
                        "--apply" => apply = true,
                        "--include-snippets" => mapping.include_snippets = true,
                        "--include-templates" => mapping.include_templates = true,
                        "--help" => return Ok(None),
                        _ => unreachable!(),
                    }
                }
            }
        } else if export.replace(PathBuf::from(arg)).is_some() {
            return Err("Expected one export path.".into());
        }
    }
    Ok(Some(Options {
        export: export.ok_or("An export path is required.")?,
        config,
        cli,
        mapping,
        apply,
    }))
}
fn input(path: &Path) -> Result<Vec<u8>> {
    // User-selected config aliases remain supported, but nonregular/torn inputs fail closed.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(error)?;
    let before = file.metadata().map_err(error)?;
    if !before.is_file() || before.len() > LIMIT {
        return Err("Import input must be a bounded regular file.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let after = file.metadata().map_err(error)?;
    let named = fs::metadata(path).map_err(error)?;
    if bytes.len() as u64 != before.len()
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
        || named.dev() != before.dev()
        || named.ino() != before.ino()
    {
        return Err("Import input changed while reading.".into());
    }
    Ok(bytes)
}
fn active(cli: &Path) -> Result<PathBuf> {
    let mut command = Command::new(cli);
    command.args(["config", "path"]);
    let capture = process::capture(command, Duration::from_secs(10), 1024 * 1024)?;
    if !capture.status.success() {
        return Err("Active configuration CLI failed.".into());
    }
    let value: Value = serde_json::from_slice(&capture.stdout).map_err(error)?;
    if value["ok"] != true {
        return Err("Cannot resolve the active configuration.".into());
    }
    Ok(PathBuf::from(
        value["result"]
            .as_str()
            .ok_or("CLI configuration path is not a string.")?,
    ))
}
fn applications(data: &Value, dirs: &[PathBuf]) -> Result<BTreeMap<String, Application>> {
    let mut apps = BTreeMap::new();
    let mut roots = BTreeMap::new();
    for row in data["builtin_package_rootSearch"]["rootSearch"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(key) = row["key"].as_str() {
            roots.insert(key, row);
        }
    }
    let wanted = roots
        .into_iter()
        .filter_map(|(id, row)| {
            let enabled = row.get("alias").is_some_and(|value| match value {
                Value::Null => false,
                Value::Bool(v) => *v,
                Value::Number(v) => v.as_f64() != Some(0.0),
                Value::String(v) => !v.is_empty(),
                Value::Array(v) => !v.is_empty(),
                Value::Object(v) => !v.is_empty(),
            });
            (row["type"] == "systemApp" && enabled).then_some(id)
        })
        .collect::<std::collections::BTreeSet<_>>();
    if wanted.is_empty() {
        return Ok(apps);
    }
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension() != Some(OsStr::new("app")) {
                continue;
            }
            let Some(name) = path.to_str() else { continue };
            let url = NSURL::fileURLWithPath(&NSString::from_str(name));
            let Some(bundle) = NSBundle::bundleWithURL(&url) else {
                continue;
            };
            let Some(info) = bundle.infoDictionary() else {
                continue;
            };
            let Some(id) = info
                .objectForKey(&NSString::from_str("CFBundleIdentifier"))
                .and_then(|v| v.downcast::<NSString>().ok())
            else {
                continue;
            };
            if !wanted.contains(id.to_string().as_str()) {
                continue;
            }
            let mut metadata = serde_json::Map::new();
            for key in ["CFBundleDisplayName", "CFBundleName"] {
                if let Some(value) = info.objectForKey(&NSString::from_str(key)) {
                    let value = value
                        .downcast::<NSString>()
                        .map_err(|_| "Invalid application title metadata.".to_string())?;
                    metadata.insert(key.into(), Value::String(value.to_string()));
                }
            }
            apps.insert(
                id.to_string(),
                Application {
                    path,
                    metadata: Value::Object(metadata),
                },
            );
        }
    }
    Ok(apps)
}
fn execute(options: Options) -> Result<Value> {
    if options.apply {
        return Err(
            "Apply is unavailable in this dry-run command; no configuration changed.".into(),
        );
    }
    let path = options.config.map_or_else(|| active(&options.cli), Ok)?;
    let data: Value = serde_json::from_slice(&input(&options.export)?).map_err(error)?;
    let config: Value = serde_json::from_slice(&input(&path)?).map_err(error)?;
    let dirs = [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices"),
        home().join("Applications"),
    ];
    let apps = applications(&data, &dirs)?;
    Ok(raycast_import::prepare(&config, &data, &apps, &options.mapping)?.summary(false))
}
pub(super) fn run(args: Vec<OsString>) -> std::result::Result<(), Failure> {
    let options = parse(args).map_err(|message| Failure { code: 2, message })?;
    let Some(options) = options else {
        println!("{HELP}");
        return Ok(());
    };
    println!("{}", execute(options)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_preserves_defaults_last_option_and_flag_failures() {
        let p = parse(vec![
            "export".into(),
            "--cli=first".into(),
            "--cli".into(),
            "second".into(),
            "--include-snippets".into(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(p.cli, PathBuf::from("second"));
        assert!(p.mapping.include_snippets);
        assert!(parse(vec!["--unknown".into()]).is_err());
        assert!(parse(vec!["export".into(), "--apply=yes".into()]).is_err());
        assert!(parse(vec![]).is_err());
        assert!(parse(vec!["--help".into()]).unwrap().is_none());
    }
    #[test]
    fn apply_refuses_before_reading_or_running_any_caller() {
        let options = Options {
            export: "/not/read".into(),
            config: None,
            cli: "/not/executed".into(),
            mapping: Mapping::default(),
            apply: true,
        };
        assert!(execute(options)
            .unwrap_err()
            .contains("no configuration changed"));
    }
    #[test]
    fn unrelated_nonstring_plist_title_does_not_abort_requested_discovery() {
        let fixture = crate::tests::Fixture::new();
        let directory = fixture.0.join("Applications");
        fs::create_dir(&directory).unwrap();
        for (name, id, title) in [
            ("Wanted", "owned.wanted", "<string>Wanted</string>"),
            ("Other", "owned.other", "<integer>42</integer>"),
        ] {
            let contents = directory.join(format!("{name}.app/Contents"));
            fs::create_dir_all(contents.join("MacOS")).unwrap();
            fs::write(contents.join("Info.plist"), format!("<?xml version=\"1.0\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleDisplayName</key>{title}</dict></plist>")).unwrap();
        }
        let data = serde_json::json!({"builtin_package_rootSearch":{"rootSearch":[{"key":"owned.wanted","type":"systemApp","alias":"w"}]}});
        let found = applications(&data, &[directory]).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found["owned.wanted"].metadata["CFBundleDisplayName"],
            "Wanted"
        );
    }
    #[test]
    #[ignore = "Explicit source-built importer and CLI artifact bindings required"]
    fn actual_dry_run_uses_active_cli_and_preserves_private_files() {
        let fixture = crate::tests::Fixture::new();
        let tool = PathBuf::from(std::env::var_os("RUYCAST_IMPORT_TOOL").unwrap());
        let cli = PathBuf::from(std::env::var_os("RUYCAST_IMPORT_CLI").unwrap());
        let native = PathBuf::from(std::env::var_os("RUYCAST_IMPORT_NATIVE").unwrap());
        assert_eq!(
            digest(&tool).unwrap(),
            std::env::var("RUYCAST_IMPORT_SHA256").unwrap()
        );
        let profile = fixture.0.join("profile");
        fs::create_dir(&profile).unwrap();
        let original = include_bytes!("../../../../config/default.json");
        fs::write(profile.join("config.json"), original).unwrap();
        let selected = fixture.0.join("Synced vault with spaces/config");
        let invoke = |executable: &Path, args: &[OsString]| {
            let mut command = Command::new(executable);
            command
                .args(args)
                .env_clear()
                .current_dir(&fixture.0)
                .env("HOME", &fixture.0)
                .env("RUYCAST_HOME", &profile)
                .env("RUYCAST_APP_BINARY", &native);
            process::capture(command, Duration::from_secs(30), 1024 * 1024).unwrap()
        };
        let relocated = invoke(
            &cli,
            &[
                "config".into(),
                "directory".into(),
                selected.clone().into_os_string(),
            ],
        );
        assert!(relocated.status.success());
        let added = invoke(&cli, &["config".into(), "add".into(), serde_json::json!({"id":"fixture.portable","title":"Portable","kind":"url","target":"https://example.com"}).to_string().into()]);
        assert!(added.status.success());
        let export = fixture.0.join("raycast-export.json");
        let data = serde_json::json!({"builtin_package_rootSearch":{"rootSearch":[]},"raycast_user_activity":{"activityInfo":[]},"builtin_package_quicklinks":{"quicklinks":[]}}).to_string();
        fs::write(&export, &data).unwrap();
        let selected_bytes = fs::read(selected.join("config.json")).unwrap();
        let result = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.clone().into_os_string(),
                "--cli".into(),
                cli.clone().into_os_string(),
            ],
        );
        assert_eq!(
            result.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        let initial: Value = serde_json::from_slice(original).unwrap();
        assert_eq!(
            summary["totalCommands"],
            initial["commands"].as_array().unwrap().len() + 1
        );
        assert_eq!(summary["applied"], false);
        assert_eq!(summary["hotkeysImported"], 0);
        let explicit = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.clone().into_os_string(),
                "--config".into(),
                selected.join("config.json").into_os_string(),
                "--cli".into(),
                "/not/executed".into(),
            ],
        );
        assert_eq!(explicit.status.code(), Some(0));
        assert_eq!(
            serde_json::from_slice::<Value>(&explicit.stdout).unwrap(),
            summary
        );
        let refused = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.clone().into_os_string(),
                "--apply".into(),
                "--cli".into(),
                "/not/executed".into(),
            ],
        );
        assert_eq!(refused.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&refused.stderr).contains("no configuration changed"));
        assert_eq!(
            fs::read(selected.join("config.json")).unwrap(),
            selected_bytes
        );
        assert_eq!(fs::read(profile.join("config.json")).unwrap(), original);
        assert_eq!(fs::read_to_string(export).unwrap(), data);
    }
}
