//! Rust importer I/O. Publication goes through the existing CLI's CAS writer.
use super::*;
use objc2_foundation::{
    NSArray, NSBundle, NSCharacterSet, NSHomeDirectory, NSJSONSerialization, NSJSONWritingOptions,
    NSString, NSURL,
};
use ruycast_native::raycast_import::{self, Application, Options as Mapping};
use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::PermissionsExt,
        },
    },
    time::Duration,
};
const LIMIT: u64 = 256 * 1024 * 1024;
const HELP:&str="ruycast-build import-raycast EXPORT [--config FILE] [--cli PATH] [--apply] [--include-snippets] [--include-templates]\nWithout --apply, configuration is never written. Apply targets the CLI active configuration; the expected SHA checks bytes, not path identity.";
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
fn negative_number(value: &OsStr) -> bool {
    let Some(value) = value.to_str().and_then(|s| s.strip_prefix('-')) else {
        return false;
    };
    let digits = NSCharacterSet::decimalDigitCharacterSet();
    let decimal = |text: &str| text.chars().all(|c| digits.longCharacterIsMember(c as u32));
    if let Some((whole, fraction)) = value.split_once('.') {
        decimal(whole) && !fraction.is_empty() && decimal(fraction)
    } else {
        !value.is_empty() && decimal(value)
    }
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
        if !positional
            && arg != OsStr::new("-")
            && !negative_number(&arg)
            && arg.as_encoded_bytes().starts_with(b"-")
        {
            let bytes = arg.as_bytes();
            let (key, inline) = bytes
                .iter()
                .position(|b| *b == b'=')
                .map_or((bytes, None), |i| {
                    (
                        &bytes[..i],
                        Some(OsString::from_vec(bytes[i + 1..].to_vec())),
                    )
                });
            let key = std::str::from_utf8(key).map_err(|_| "Unrecognized non-UTF8 option.")?;
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
                        if v != OsStr::new("-")
                            && !negative_number(&v)
                            && v.as_encoded_bytes().starts_with(b"-")
                        {
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
    // Leave cleanup margin for the original ten-second importer caller.
    let capture = process::capture(command, Duration::from_secs(8), 1024 * 1024)?;
    std::io::stderr()
        .write_all(&capture.stderr)
        .map_err(error)?;
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
            let mut non_json = std::collections::BTreeSet::new();
            for key in ["CFBundleDisplayName", "CFBundleName"] {
                if let Some(value) = info.objectForKey(&NSString::from_str(key)) {
                    let wrapper = NSArray::from_slice(&[&*value]);
                    // This is an immutable plist object inside a known NSArray.
                    // Non-JSON values (date/data) count in dry run but cannot be applied.
                    let valid = unsafe { NSJSONSerialization::isValidJSONObject(&wrapper) };
                    let converted = if valid {
                        let bytes = unsafe {
                            NSJSONSerialization::dataWithJSONObject_options_error(
                                &wrapper,
                                NSJSONWritingOptions::empty(),
                            )
                        }
                        .map_err(error)?;
                        let mut values: Vec<Value> =
                            serde_json::from_slice(&bytes.to_vec()).map_err(error)?;
                        values.pop().unwrap()
                    } else {
                        non_json.insert(key);
                        Value::Null
                    };
                    metadata.insert(key.into(), converted);
                }
            }
            let title = if metadata.contains_key("CFBundleDisplayName") {
                "CFBundleDisplayName"
            } else {
                "CFBundleName"
            };
            apps.insert(
                id.to_string(),
                Application {
                    path,
                    non_json_title: non_json.contains(title),
                    metadata: Value::Object(metadata),
                },
            );
        }
    }
    Ok(apps)
}
struct PrivateCommands {
    path: PathBuf,
    device: u64,
    inode: u64,
    removed: bool,
}
impl PrivateCommands {
    fn create(parent: &Path, commands: &[Value]) -> Result<Self> {
        let mut template = std::ffi::CString::new(
            parent
                .join(".raycast-import-XXXXXX.json")
                .as_os_str()
                .as_bytes(),
        )
        .map_err(error)?
        .into_bytes_with_nul();
        let descriptor = unsafe { libc::mkstemps(template.as_mut_ptr().cast(), 5) };
        if descriptor < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        template.pop();
        let metadata = file.metadata().map_err(error)?;
        let mut temporary = Self {
            path: PathBuf::from(OsString::from_vec(template)),
            device: metadata.dev(),
            inode: metadata.ino(),
            removed: false,
        };
        let result = (|| {
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(error(std::io::Error::last_os_error()));
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(error)?;
            file.write_all(&serde_json::to_vec(commands).map_err(error)?)
                .map_err(error)
        })();
        drop(file);
        if let Err(failure) = result {
            let cleanup = temporary.remove();
            return Err(match cleanup {
                Ok(()) => failure,
                Err(cleanup) => format!("{failure}; {cleanup}"),
            });
        }
        Ok(temporary)
    }
    fn owned(&self) -> bool {
        fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.is_file() && m.dev() == self.device && m.ino() == self.inode)
    }
    fn remove(&mut self) -> Result<()> {
        if !self.owned() {
            return Err("Private import staging file changed; preserved.".into());
        }
        fs::remove_file(&self.path).map_err(error)?;
        self.removed = true;
        Ok(())
    }
}
impl Drop for PrivateCommands {
    fn drop(&mut self) {
        if !self.removed && self.owned() {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn publish(cli: &Path, config: &Path, original: &[u8], commands: &[Value]) -> Result<()> {
    let mut temporary =
        PrivateCommands::create(config.parent().unwrap_or(Path::new(".")), commands)?;
    let mut reference = OsString::from("@");
    reference.push(&temporary.path);
    let mut command = Command::new(cli);
    command.args([
        OsString::from("config"),
        "set".into(),
        "commands".into(),
        reference,
        "--expect-sha256".into(),
        format!("{:x}", Sha256::digest(original)).into(),
    ]);
    let outcome =
        process::capture(command, Duration::from_secs(10), 1024 * 1024).and_then(|output| {
            if output.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        });
    let cleanup = temporary.remove();
    match (outcome, cleanup) {
        (Ok(()), result) => result,
        (Err(failure), Ok(())) => Err(failure),
        (Err(failure), Err(cleanup)) => Err(format!("{failure}; {cleanup}")),
    }
}
fn execute(options: Options) -> Result<Value> {
    let path = options.config.map_or_else(|| active(&options.cli), Ok)?;
    let data: Value = serde_json::from_slice(&input(&options.export)?).map_err(error)?;
    let original = input(&path)?;
    let config: Value = serde_json::from_slice(&original).map_err(error)?;
    let dirs = [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices"),
        home().join("Applications"),
    ];
    let apps = applications(&data, &dirs)?;
    let prepared = raycast_import::prepare(&config, &data, &apps, &options.mapping)?;
    if options.apply {
        if prepared.non_json_title {
            return Err("Application title is not JSON serializable.".into());
        }
        publish(&options.cli, &path, &original, &prepared.commands)?;
    }
    Ok(prepared.summary(options.apply))
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
    fn parser_keeps_unicode_decimal_paths_os_bytes_and_argparse_boundaries() {
        for value in ["-", "-1", "-.2", "-١", "-١.٢"] {
            assert_eq!(
                parse(vec![value.into()]).unwrap().unwrap().export,
                PathBuf::from(value)
            );
            assert_eq!(
                parse(vec!["export".into(), "--config".into(), value.into()])
                    .unwrap()
                    .unwrap()
                    .config,
                Some(PathBuf::from(value))
            );
        }
        let bytes = b"--config=owned-\xff.json".to_vec();
        let parsed = parse(vec!["export".into(), OsString::from_vec(bytes)])
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed.config.unwrap().as_os_str().as_bytes(),
            b"owned-\xff.json"
        );
        assert_eq!(
            parse(vec!["export".into()]).unwrap().unwrap().cli,
            home().join(".local/bin/ruycast")
        );
        assert_eq!(
            parse(vec!["--".into(), "--literal".into()])
                .unwrap()
                .unwrap()
                .export,
            PathBuf::from("--literal")
        );
        assert_eq!(
            parse(vec!["export".into(), "--con=file".into()])
                .unwrap()
                .unwrap()
                .config,
            Some(PathBuf::from("file"))
        );
        assert!(parse(vec!["export".into(), "--include".into()]).is_err());
        assert!(parse(vec!["export".into(), "--config".into(), "--apply".into()]).is_err());
        assert!(parse(vec!["export".into(), "-Ⅳ".into()]).is_err());
    }
    #[test]
    fn private_commands_are_0600_cleaned_and_replacements_are_preserved() {
        let fixture = crate::tests::Fixture::new();
        let commands = [serde_json::json!({"private":"owned fixture"})];
        let temporary = PrivateCommands::create(&fixture.0, &commands).unwrap();
        let path = temporary.path.clone();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            serde_json::json!(commands)
        );
        drop(temporary);
        assert!(!path.exists());
        let mut temporary = PrivateCommands::create(&fixture.0, &commands).unwrap();
        let path = temporary.path.clone();
        let retained_inode = File::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"external replacement").unwrap();
        assert!(temporary.remove().is_err());
        drop(temporary);
        assert_eq!(fs::read(&path).unwrap(), b"external replacement");
        drop(retained_inode);
        let config = fixture.0.join("config.json");
        fs::write(&config, b"{}").unwrap();
        assert!(publish(Path::new("/not/executed"), &config, b"{}", &commands).is_err());
        assert_eq!(fs::read(config).unwrap(), b"{}");
        assert_eq!(
            fs::read_dir(&fixture.0)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".raycast-import-"))
                .count(),
            1
        );
    }
    #[test]
    fn unrelated_nonstring_plist_title_does_not_abort_requested_discovery() {
        let fixture = crate::tests::Fixture::new();
        let directory = fixture.0.join("Applications");
        fs::create_dir(&directory).unwrap();
        for (name, id, title) in [
            ("Wanted", "owned.wanted", "<string>Wanted</string>"),
            ("Other", "owned.other", "<integer>42</integer>"),
            ("Date", "owned.date", "<date>2024-01-01T00:00:00Z</date>"),
            (
                "Unused",
                "owned.unused",
                "<string>Good</string><key>CFBundleName</key><date>2024-01-01T00:00:00Z</date>",
            ),
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
        let data = serde_json::json!({"builtin_package_rootSearch":{"rootSearch":[{"key":"owned.other","type":"systemApp","alias":"o"},{"key":"owned.date","type":"systemApp","alias":"d"},{"key":"owned.unused","type":"systemApp","alias":"u"}]},"raycast_user_activity":{"activityInfo":[]},"builtin_package_quicklinks":{"quicklinks":[]}});
        let found = applications(&data, &[fixture.0.join("Applications")]).unwrap();
        assert_eq!(found["owned.other"].metadata["CFBundleDisplayName"], 42);
        assert!(found["owned.date"].non_json_title);
        assert!(!found["owned.unused"].non_json_title);
        let prepared = raycast_import::prepare(
            &serde_json::json!({"commands":[]}),
            &data,
            &found,
            &Mapping::default(),
        )
        .unwrap();
        assert_eq!(prepared.counts["apps"], 3);
        assert!(prepared.non_json_title);
        let existing = serde_json::json!({"commands":[{"id":"existing","kind":"app","target":"owned.date","title":"Existing"}]});
        assert!(
            !raycast_import::prepare(&existing, &data, &found, &Mapping::default())
                .unwrap()
                .non_json_title
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
        assert!(!refused.stderr.is_empty());
        assert_eq!(
            fs::read(selected.join("config.json")).unwrap(),
            selected_bytes
        );
        assert_eq!(fs::read(profile.join("config.json")).unwrap(), original);
        assert_eq!(fs::read_to_string(&export).unwrap(), data);
        let stale = fixture.0.join("stale-config.json");
        fs::write(&stale, &selected_bytes).unwrap();
        let mut incoming: Value = serde_json::from_str(&data).unwrap();
        incoming["builtin_package_quicklinks"]["quicklinks"] = serde_json::json!([{"uuid":"OWNED","name":"Owned import","url":"https://example.com/imported"}]);
        fs::write(&export, serde_json::to_vec(&incoming).unwrap()).unwrap();
        let applied = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.clone().into_os_string(),
                "--apply".into(),
                "--cli".into(),
                cli.clone().into_os_string(),
            ],
        );
        assert_eq!(
            applied.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
        let report: Value = serde_json::from_slice(&applied.stdout).unwrap();
        assert_eq!(report["applied"], true);
        assert_eq!(report["imported"]["quicklinks"], 1);
        let published = fs::read(selected.join("config.json")).unwrap();
        let config: Value = serde_json::from_slice(&published).unwrap();
        assert!(config["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "link.owned"));
        // Preserve and expose the legacy byte-CAS boundary: another active
        // path with identical bytes is not distinguished by expected SHA.
        let alternate = fixture.0.join("alternate-active");
        fs::create_dir(&alternate).unwrap();
        fs::write(alternate.join("config.json"), &published).unwrap();
        fs::write(
            profile.join("config-location.json"),
            serde_json::to_vec(&serde_json::json!({"directory":alternate})).unwrap(),
        )
        .unwrap();
        incoming["builtin_package_quicklinks"]["quicklinks"] = serde_json::json!([{"uuid":"BOUNDARY","name":"Same bytes boundary","url":"https://example.com/boundary"}]);
        fs::write(&export, serde_json::to_vec(&incoming).unwrap()).unwrap();
        let same_bytes = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.clone().into_os_string(),
                "--apply".into(),
                "--config".into(),
                selected.join("config.json").into_os_string(),
                "--cli".into(),
                cli.clone().into_os_string(),
            ],
        );
        assert_eq!(same_bytes.status.code(), Some(0));
        assert_eq!(fs::read(selected.join("config.json")).unwrap(), published);
        let alternate_bytes = fs::read(alternate.join("config.json")).unwrap();
        assert!(
            serde_json::from_slice::<Value>(&alternate_bytes).unwrap()["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == "link.boundary")
        );
        let stale_attempt = invoke(
            &tool,
            &[
                "import-raycast".into(),
                export.into_os_string(),
                "--apply".into(),
                "--config".into(),
                stale.clone().into_os_string(),
                "--cli".into(),
                cli.into_os_string(),
            ],
        );
        assert_eq!(stale_attempt.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&stale_attempt.stderr).contains("changed"));
        assert_eq!(fs::read(selected.join("config.json")).unwrap(), published);
        assert_eq!(
            fs::read(alternate.join("config.json")).unwrap(),
            alternate_bytes
        );
        assert_eq!(fs::read(stale).unwrap(), selected_bytes);
        assert_eq!(fs::read(profile.join("config.json")).unwrap(), original);
        for directory in [&selected, &alternate, &fixture.0] {
            assert!(!fs::read_dir(directory).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".raycast-import-")));
        }
    }
}
