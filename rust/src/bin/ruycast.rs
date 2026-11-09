#[path = "../private_storage.rs"]
mod private_storage;
use rustcast_core::{default_config, validate, Command as Action, LAYOUTS};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::{MetadataExt, OpenOptionsExt}, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
type Result<T> = std::result::Result<T, String>;
fn selected_home(value:Option<std::ffi::OsString>,fallback:impl FnOnce()->Result<PathBuf>)->Result<PathBuf>{
    value.filter(|v|!v.is_empty()).map(PathBuf::from).map(Ok).unwrap_or_else(fallback)
}
fn account_home()->Result<PathBuf>{
    use std::os::unix::ffi::OsStrExt;
    let mut size=16384;
    loop {
        let mut buffer=vec![0u8;size];let mut record=std::mem::MaybeUninit::<libc::passwd>::uninit();let mut found=std::ptr::null_mut();
        let code=unsafe{libc::getpwuid_r(libc::getuid(),record.as_mut_ptr(),buffer.as_mut_ptr().cast(),buffer.len(),&mut found)};
        if code==libc::ERANGE&&size<1024*1024 {size*=2;continue;}
        if code!=0||found.is_null(){return Err("Cannot determine the home directory for the default Ruycast profile.".into());}
        let record=unsafe{record.assume_init()};if record.pw_dir.is_null(){return Err("The account home directory is unavailable.".into());}
        let bytes=unsafe{std::ffi::CStr::from_ptr(record.pw_dir)}.to_bytes();
        let path=PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
        return if path.is_absolute(){Ok(path)}else{Err("The account home directory must be absolute.".into())};
    }
}
fn home() -> Result<PathBuf> { selected_home(env::var_os("HOME"),account_home) }
fn root() -> Result<PathBuf> {
    let path = env::var_os("RUYCAST_HOME")
        .or_else(|| env::var_os("RUSTCAST_HOME"))
        .map(|v|Ok(PathBuf::from(v)))
        .unwrap_or_else(|| home().map(|p|p.join(".config/ruycast")))?;
    absolute_profile(&path, &env::current_dir().map_err(|e| e.to_string())?)
}
fn absolute_profile(path: &Path, cwd: &Path) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().is_empty() || path.to_str().is_none() || path.as_os_str().as_bytes().contains(&0) {
        return Err("Choose a nonempty UTF-8 profile path without NUL characters.".into());
    }
    let path = if path.is_absolute() {path.to_owned()} else {cwd.join(path)};
    if !path.is_absolute() || path.components().any(|p| p == std::path::Component::ParentDir) {
        return Err("Choose an absolute profile path without '..' components.".into());
    }
    Ok(path.components().collect())
}
fn config_override() -> Result<Option<String>> {
    env::var_os("RUYCAST_CONFIG_DIR")
        .or_else(|| env::var_os("RUSTCAST_CONFIG_DIR"))
        .map(|value| value.into_string().map_err(|_| "Choose a UTF-8 configuration folder path.".into()))
        .transpose()
}
fn absolute_directory(value: &str) -> Result<PathBuf> {
    let path = if value == "~" {
        home()?
    } else if let Some(tail) = value.strip_prefix("~/") {
        home()?.join(tail)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() || value.contains(['\n', '\r']) {
        return Err("Choose an absolute configuration folder path.".into());
    }
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("Choose a configuration folder without '..' components.".into());
    }
    Ok(path.components().collect())
}
fn config_directory() -> Result<PathBuf> {
    let root = root()?;
    if let Some(value) = config_override()? {
        if !value.is_empty() {
            return absolute_directory(&value);
        }
    }
    let selector = root.join("config-location.json");
    if !selector.exists() {
        return Ok(root);
    }
    let value = parse(&read(&selector)?)?;
    absolute_directory(
        value["directory"]
            .as_str()
            .ok_or("Invalid configuration location; fix config-location.json.")?,
    )
}
fn relocate_config(path: &str) -> Result<Value> {
    absolute_directory(path)?;
    if config_override()?.is_some_and(|v| !v.is_empty()) {
        return Err(
            "RUYCAST_CONFIG_DIR controls this configuration. Change or unset it first.".into(),
        );
    }
    if root()?.join("control.sock").exists() {
        return rpc(json!({"op":"config.directory","directory":path}));
    }
    let output = Command::new(binary()?)
        .args(["--set-config-directory", path])
        .env("RUYCAST_HOME", root()?)
        .output()
        .map_err(|e| e.to_string())?;
    let value = parse(&output.stdout).map_err(|_| {
        format!(
            "Could not change configuration folder: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    Ok(value)
}
fn private_dir(p: &Path) -> Result<()> {
    private_storage::secure_directory(p)
}
fn read(p: &Path) -> Result<Vec<u8>> {
    let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK|libc::O_CLOEXEC)
        .open(p).map_err(|e|format!("{}: {e}",p.display()))?;
    if !file.metadata().map_err(|e|e.to_string())?.is_file(){return Err("Expected a regular configuration file.".into());}
    let mut bytes=Vec::new();file.read_to_end(&mut bytes).map_err(|e|e.to_string())?;Ok(bytes)
}
fn parse(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}
fn atomically(p: &Path, bytes: &[u8]) -> Result<()> {
    private_storage::write(p, bytes, true)
}
fn rpc(payload: Value) -> Result<Value> {
    let mut s = UnixStream::connect(root()?.join("control.sock")).map_err(|e| {
        format!("Ruycast is not running or its socket is unavailable ({e}). Run: ruycast start")
    })?;
    s.set_read_timeout(Some(Duration::from_secs(312)))
        .map_err(|e| e.to_string())?;
    writeln!(s, "{payload}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s)
        .take(256 * 1024 * 1024)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if !line.ends_with('\n') {
        return Err("Ruycast disconnected or its response exceeded 256 MiB.".into());
    }
    parse(line.as_bytes())
}
fn argument<'a>(args: &'a [String], i: usize, name: &str) -> Result<&'a str> {
    args.get(i)
        .map(String::as_str)
        .ok_or_else(|| format!("Missing {name}. Run ruycast --help."))
}
// Count a conservative lower bound of compact JSON while serde consumes the
// original bytes. This bounds strings and Value nodes before AST allocation;
// whitespace and numeric spelling must not invalidate compactable exports.
struct ImportReader<R> { input:R, remaining:usize, string:bool, escape:bool, unicode:u8, primitive:bool }
impl<R:Read> ImportReader<R> {
    fn new(input:R,remaining:usize)->Self {Self{input,remaining,string:false,escape:false,unicode:0,primitive:false}}
}
impl<R:Read> Read for ImportReader<R> {
    fn read(&mut self,buffer:&mut [u8])->io::Result<usize> {
        let count=self.input.read(buffer)?;
        for &byte in &buffer[..count] {
            let charge=if self.string {
                if self.unicode>0 {self.unicode-=1;false}
                else if self.escape {self.escape=false;if byte==b'u'{self.unicode=4;}true}
                else if byte==b'\\' {self.escape=true;false}
                else {if byte==b'"'{self.string=false;}true}
            }else if byte.is_ascii_whitespace(){self.primitive=false;false}
            else if byte==b'"'{self.string=true;self.primitive=false;true}
            else if b"{}[]:,".contains(&byte){self.primitive=false;true}
            else {let first=!self.primitive;self.primitive=true;first};
            if charge {self.remaining=self.remaining.checked_sub(1).ok_or_else(||io::Error::other("Clipboard import exceeds the 1 MiB local request limit before allocation."))?;}
        }
        Ok(count)
    }
}
fn clipboard_import_request(raw: &str) -> Result<Value> {
    // The existing transport accepts 1 MiB requests and at most 256 MiB
    // replies/exports. Raw pretty/escaped JSON is not the compact wire frame.
    const EXPORT_BYTES: u64 = 256 * 1024 * 1024;
    const REQUEST_BYTES: usize = 1_048_576;
    let items = if let Some(path) = raw.strip_prefix('@') {
        let mut file = OpenOptions::new().read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path).map_err(|e|format!("Cannot open clipboard import JSON: {e}"))?;
        let before = file.metadata().map_err(|e|e.to_string())?;
        if !before.is_file() { return Err("Clipboard import requires a regular JSON file.".into()); }
        if before.len() > EXPORT_BYTES { return Err("Clipboard import file exceeds the existing 256 MiB export limit.".into()); }
        let mut bounded = BufReader::new((&mut file).take(EXPORT_BYTES + 1));
        let value = serde_json::from_reader(ImportReader::new(&mut bounded,REQUEST_BYTES-1)).map_err(|e|e.to_string())?;
        if bounded.get_ref().limit() == 0 { return Err("Clipboard import file grew beyond 256 MiB while reading.".into()); }
        let after = file.metadata().map_err(|e|e.to_string())?;
        let current = fs::symlink_metadata(path).map_err(|e|e.to_string())?;
        let identity = |m:&fs::Metadata| (m.dev(),m.ino(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
        if !current.is_file() || identity(&before) != identity(&after) || identity(&before) != identity(&current) {
            return Err("Clipboard import file changed while reading; nothing imported.".into());
        }
        value
    } else { parse(raw.as_bytes())? };
    let mut request = json!({"op":"clipboard.import"});
    request["items"] = items;
    // Count serialized bytes without allocating another potentially huge JSON
    // string, reserving the newline required by the existing local protocol.
    struct WireBudget(usize);
    impl Write for WireBudget {
        fn write(&mut self, bytes:&[u8])->io::Result<usize> {
            self.0=self.0.checked_sub(bytes.len()).ok_or_else(||io::Error::other("Clipboard import exceeds the 1 MiB local request limit."))?;
            Ok(bytes.len())
        }
        fn flush(&mut self)->io::Result<()> { Ok(()) }
    }
    serde_json::to_writer(WireBudget(REQUEST_BYTES - 1),&request).map_err(|e|e.to_string())?;
    Ok(request)
}
fn binary_candidates(exe:&Path,explicit:Option<PathBuf>,source:&Path)->Vec<PathBuf>{
    let target=if cfg!(target_arch="aarch64"){"aarch64-apple-darwin"}else{"x86_64-apple-darwin"};
    [
        explicit,
        exe.parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("MacOS/RuycastApp")),
        Some(PathBuf::from(
            "/Applications/Ruycast.app/Contents/MacOS/RuycastApp",
        )),
        Some(source.join(format!(".build/full-rust-release/native/{target}/release/ruycast-native"))),
        Some(source.join("rust-native/target/release/ruycast-native")),
        Some(source.join("rust-native/target/debug/ruycast-native")),
    ].into_iter().flatten().collect()
}
fn binary() -> Result<PathBuf> {
    let exe=env::current_exe().map_err(|e|e.to_string())?;
    let source=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("Rust source root unavailable.")?;
    binary_candidates(&exe,env::var_os("RUYCAST_APP_BINARY").or_else(||env::var_os("RUSTCAST_APP_BINARY")).map(PathBuf::from),source)
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("Ruycast app is not installed or built. Build the Rust host or install the matching Ruycast app.".into())
}
fn import_commands(config: &mut Value, source: &Value, without_hotkeys: bool) -> Result<Value> {
    validate(config)?;
    let source_commands = source["commands"].as_array().ok_or("Source commands must be an array.")?;
    let mut imported = 0;
    let mut duplicates = 0;
    let mut omitted_hotkeys = 0;
    let mut restored_hotkeys = 0;
    let mut conflicts = Vec::new();
    let mut unavailable_scripts = Vec::new();
    for command in source_commands {
        let kind = command["kind"].as_str().unwrap_or_default();
        if !matches!(kind, "url" | "path" | "script") {
            continue;
        }
        let mut candidate = command.clone();
        let omitted = without_hotkeys
            && candidate.as_object_mut().and_then(|fields| fields.remove("hotkey")).is_some_and(|key| {
                if key.is_null() { return false; }
                candidate["importedHotkey"] = key;
                true
            });
        if omitted && !candidate["importedHotkey"].as_str().is_some_and(|key| rustcast_core::shortcut(key).is_ok()) {
            conflicts.push(json!({"id":command["id"],"reason":"Invalid source shortcut."}));
            continue;
        }
        let existing = config["commands"].as_array_mut().ok_or("Commands must be an array.")?;
        if existing.contains(&candidate) {
            duplicates += 1;
            continue;
        }
        if omitted {
            let mut previous = candidate.clone();
            previous.as_object_mut().expect("imported command object").remove("importedHotkey");
            if let Some(entry) = existing.iter_mut().find(|entry| **entry == previous) {
                *entry = candidate;
                restored_hotkeys += 1;
                continue;
            }
        }
        existing.push(candidate);
        if let Err(reason) = validate(config) {
            config["commands"].as_array_mut().expect("validated command array").pop();
            conflicts.push(json!({"id":command["id"],"reason":reason}));
            continue;
        }
        imported += 1;
        omitted_hotkeys += usize::from(omitted);
        if kind == "script" {
            let target = command["target"].as_str().expect("validated script target");
            if !absolute_directory(target).is_ok_and(|path| path.is_file()) {
                unavailable_scripts.push(command["id"].clone());
            }
        }
    }
    Ok(json!({"imported":imported,"duplicates":duplicates,"conflicts":conflicts,
        "omittedHotkeys":omitted_hotkeys,"restoredHotkeys":restored_hotkeys,"unavailableScripts":unavailable_scripts}))
}

#[test]
fn imports_quicklinks_and_scripts_without_overwriting_existing_commands() {
    let mut config = default_config();
    let source = json!({"commands":[
        {"id":"fixture.link","title":"Link","kind":"url","target":"https://example.invalid"},
        {"id":"fixture.script","title":"Script","kind":"script","target":"/usr/bin/true",
            "arguments":["fixture"],"aliases":["fixture-test"],"hotkey":"alt+shift+f8","enabled":false},
        {"id":"fixture.link","title":"Collision","kind":"path","target":"/"},
        {"id":"fixture.app","title":"App","kind":"app","target":"example.app"}
    ]});
    let initial = config["commands"].as_array().unwrap().len();
    let report = import_commands(&mut config, &source, true).unwrap();
    assert_eq!(report["imported"], 2);
    assert_eq!(report["omittedHotkeys"], 1);
    assert_eq!(report["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(config["commands"][initial], source["commands"][0]);
    let mut script = source["commands"][1].clone();
    script.as_object_mut().unwrap().remove("hotkey");
    script["importedHotkey"] = source["commands"][1]["hotkey"].clone();
    assert_eq!(config["commands"][initial + 1], script);
    let repeat = import_commands(&mut config, &source, true).unwrap();
    assert_eq!(repeat["imported"], 0);
    assert_eq!(repeat["duplicates"], 2);
    config["commands"][initial + 1].as_object_mut().unwrap().remove("importedHotkey");
    let upgrade = import_commands(&mut config, &source, true).unwrap();
    assert_eq!(upgrade["restoredHotkeys"], 1);
    assert_eq!(config["commands"][initial + 1], script);
    config["commands"][initial + 1].as_object_mut().unwrap().remove("importedHotkey");
    let mut malformed = source.clone();
    malformed["commands"][1]["hotkey"] = json!("not a shortcut");
    let report = import_commands(&mut config, &malformed, true).unwrap();
    assert_eq!(report["restoredHotkeys"], 0);
    assert!(config["commands"][initial + 1].get("importedHotkey").is_none());
}

fn activate_imported_hotkeys(config: &mut Value) -> Result<usize> {
    let commands = config["commands"].as_array_mut().ok_or("Commands must be an array.")?;
    let mut activated = 0;
    for command in commands {
        let fields = command.as_object_mut().ok_or("Command must be an object.")?;
        let Some(imported) = fields.remove("importedHotkey").filter(|key| !key.is_null()) else {
            continue;
        };
        if fields.get("hotkey").is_some_and(|key| !key.is_null() && *key != imported) {
            return Err("An imported shortcut would replace a different configured shortcut.".into());
        }
        fields.insert("hotkey".into(), imported);
        activated += 1;
    }
    validate(config)?;
    Ok(activated)
}

#[test]
fn activation_validates_imported_shortcut_conflicts() {
    let mut config = default_config();
    let launcher = config["launcherHotkey"].clone();
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"fixture.activation","title":"Activation","kind":"script",
        "target":"/usr/bin/true","importedHotkey":launcher
    }));
    let index = config["commands"].as_array().unwrap().len() - 1;
    assert!(activate_imported_hotkeys(&mut config.clone()).is_err());
    config["commands"][index]["importedHotkey"] = json!("ctrl+shift+alt+cmd+f8");
    assert_eq!(activate_imported_hotkeys(&mut config).unwrap(), 1);
    assert_eq!(config["commands"][index]["hotkey"], "ctrl+shift+alt+cmd+f8");
    assert!(config["commands"][index].get("importedHotkey").is_none());
}

fn config_edit(args: &[String]) -> Result<Value> {
    let r = root()?;
    private_dir(&r)?;
    let lock = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(r.join("config.lock"))
        .map_err(|e| format!("Cannot open a safe configuration lock: {e}"))?;
    let lock_metadata = lock.metadata().map_err(|e|e.to_string())?;
    if !lock_metadata.is_file() || lock_metadata.uid() != unsafe {libc::getuid()} || lock_metadata.nlink() != 1 {
        return Err("Configuration lock must be an owned regular file without hard links.".into());
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err("Cannot lock configuration.".into());
    }
    let path = config_directory()?.join("config.json");
    let original = read(&path)?;
    let mut value = parse(&original)?;
    if let Some(i) = args.iter().position(|s| s == "--expect-sha256") {
        let expected = argument(args, i + 1, "sha256")?;
        if expected != format!("{:x}", Sha256::digest(&original)) {
            return Err(
                "Config changed since it was read. Read config get and retry with its sha256."
                    .into(),
            );
        }
    }
    let action = argument(args, 1, "config action")?;
    let load = |s: &str| -> Result<Value> {
        if let Some(path) = s.strip_prefix('@') {
            parse(&read(Path::new(path))?)
        } else {
            parse(s.as_bytes())
        }
    };
    let mut import = None;
    match action {
        "activate-imported-hotkeys" => {
            let options = &args[2..];
            let dry_run = options == ["--dry-run"];
            if !dry_run && !(options.len() == 2 && options[0] == "--expect-sha256") {
                return Err("Use config activate-imported-hotkeys --dry-run or --expect-sha256 SHA.".into());
            }
            let count = activate_imported_hotkeys(&mut value)?;
            if dry_run || count == 0 {
                return Ok(json!({"ok":true,"bindings":count,"written":false}));
            }
        }
        "import-commands" => {
            let source = Path::new(argument(args, 2, "source configuration path")?);
            let options = &args[3..];
            if options.iter().any(|s| !matches!(s.as_str(), "--dry-run" | "--without-hotkeys")) {
                return Err("Use config import-commands SOURCE [--dry-run] [--without-hotkeys].".into());
            }
            let report = import_commands(&mut value, &parse(&read(source)?)?, options.iter().any(|s| s == "--without-hotkeys"))?;
            if options.iter().any(|s| s == "--dry-run") || (report["imported"] == 0 && report["restoredHotkeys"] == 0) {
                return Ok(json!({"ok":true,"import":report,"written":false}));
            }
            import = Some(report);
        }
        "replace" => {
            if !args.iter().any(|s| s == "--expect-sha256") {
                return Err("Config replace requires --expect-sha256.".into());
            }
            value = load(argument(args, 2, "configuration JSON")?)?;
        }
        "set" => {
            let key = argument(args, 2, "setting")?;
            let keys = key.split('.').collect::<Vec<_>>();
            let mut target = &mut value;
            for part in &keys[..keys.len() - 1] {
                target = target
                    .get_mut(*part)
                    .ok_or_else(|| format!("Unknown config setting: {key}"))?
            }
            if ["snippetExpansionEnabled", "hyperKeyEnabled"].contains(&key)
                && target.get(key).is_none()
            {
                target[key] = json!(false);
            }
            if key == "theme" && target.get(key).is_none() {
                target[key] = json!("system");
            }
            if key == "separateCommandKeys" && target.get(key).is_none() {
                target[key] = json!(true);
            }
            if key == "menuBarVisible" && target.get(key).is_none() {
                target[key] = json!(true);
            }
            let final_key = keys.last().unwrap();
            let slot = target
                .get_mut(*final_key)
                .ok_or_else(|| format!("Unknown config setting: {key}"))?;
            *slot = load(argument(args, 3, "JSON value")?)?;
        }
        "add" => value["commands"]
            .as_array_mut()
            .ok_or("commands must be an array")?
            .push(load(argument(args, 2, "command JSON")?)?),
        "remove" => {
            let id = argument(args, 2, "command id")?;
            let commands = value["commands"]
                .as_array_mut()
                .ok_or("commands must be an array")?;
            let old = commands.len();
            commands.retain(|c| c["id"].as_str() != Some(id));
            if old == commands.len() {
                return Err(format!("Unknown command: {id}"));
            }
        }
        _ => return Err("Unknown config action.".into()),
    }
    validate(&value)?;
    if read(&path)? != original {
        return Err("Config changed during validation; nothing overwritten.".into());
    }
    let mut bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    atomically(&path, &bytes)?;
    // Keep other CLI writers serialized until runtime activation succeeds.
    if r.join("control.sock").exists() {
        let activation = rpc(json!({"op":"reload","configPath":path}));
        if activation.as_ref().is_ok_and(|v| v["ok"] == true) {
            let mut result = activation?;
            result["configPath"] = json!(path);
            if let Some(report) = import {
                result["import"] = report;
                result["written"] = json!(true);
            }
            Ok(result)
        } else {
            // An external editor is outside this lock: never roll back its newer bytes.
            let restored = read(&path)? == bytes;
            if restored {
                atomically(&path, &original)?;
            }
            let recovery = if restored {
                rpc(json!({"op":"reload","configPath":path})).ok()
            } else {
                None
            };
            let message = match activation {
                Ok(v) => v["error"]
                    .as_str()
                    .unwrap_or("Runtime rejected configuration")
                    .to_owned(),
                Err(e) => e,
            };
            Ok(
                json!({"ok":false,"error":format!("{}. {}",message,if restored {"Previous configuration restored."} else {"Configuration changed externally; reload and try again."}),"rolledBack":restored,"runtimeRecovered":recovery.is_some_and(|v|v["ok"]==true)}),
            )
        }
    } else {
        let mut result = json!({"ok":true,"result":{"path":path,"reload":"on_next_start"}});
        if let Some(report) = import {
            result["import"] = report;
            result["written"] = json!(true);
        }
        Ok(result)
    }
}
fn run_script(value: Value) -> Result<Value> {
    let action: Action = serde_json::from_value(value).map_err(|e| e.to_string())?;
    rustcast_core::script::run(&action, &home()?, None, || false).map(|result| result.response())
}

/// Internal two-line protocol. Keeping stdin open is the owner's lifeline;
/// its EOF cancels the independently owned script group even after host death.
fn supervised_script() -> Result<Value> {
    use std::os::unix::ffi::OsStringExt;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        command: Action,
        home_bytes: Vec<u8>,
    }
    let flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(0, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("Could not prepare script supervision.".into());
    }
    {
        let mut output = io::stdout().lock();
        writeln!(
            output,
            "{}",
            json!({"scriptSupervisor":rustcast_core::script::SUPERVISOR_VERSION,"ready":true})
        )
        .map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
    let receive = || -> Result<Request> {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 16384];
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("Script runner input timed out.".into());
            }
            let count = unsafe { libc::read(0, buffer.as_mut_ptr().cast(), buffer.len()) };
            if count == 0 {
                return Err("Script runner input ended before a complete command.".into());
            }
            if count < 0 {
                let error = io::Error::last_os_error();
                if !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    return Err("Script runner input failed.".into());
                }
                rustcast_core::script::wait_pipe_readiness(
                    &mut [libc::pollfd {
                        fd: 0,
                        events: libc::POLLIN,
                        revents: 0,
                    }],
                    Some(deadline),
                )
                .map_err(|_| "Script runner input failed.".to_string())?;
                continue;
            }
            let chunk = &buffer[..count as usize];
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("Script runner input exceeds the supported size.".into());
            }
            if let Some(newline) = chunk.iter().position(|b| *b == b'\n') {
                if newline + 1 != chunk.len() {
                    return Err("Unexpected script runner input.".into());
                }
                bytes.extend_from_slice(&chunk[..newline]);
                let request: Request =
                    serde_json::from_slice(&bytes).map_err(|_| "Invalid script runner input.")?;
                if request.home_bytes.len() > 4096 {
                    return Err("Invalid script home path.".into());
                }
                if request.home_bytes.first() != Some(&b'/') || request.home_bytes.contains(&0) {
                    return Err("Invalid script home path.".into());
                }
                return Ok(request);
            }
            bytes.extend_from_slice(chunk);
        }
    };
    let (response, capture) = match receive() {
        Err(error) => (json!({"ok":false,"error":error}), None),
        Ok(request) => {
            let home = PathBuf::from(std::ffi::OsString::from_vec(request.home_bytes));
            rustcast_core::script::run(&request.command, &home, None, || {
                let mut byte = 0u8;
                for _ in 0..4 {
                    let count = unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) };
                    if count >= 0 {
                        return true;
                    } // EOF or extra data cancels.
                    match io::Error::last_os_error().kind() {
                        io::ErrorKind::Interrupted => continue,
                        io::ErrorKind::WouldBlock => return false,
                        _ => return true,
                    }
                }
                true
            })
            .map(|r| (r.response(), Some(r.capture)))
            .unwrap_or_else(|error| (json!({"ok":false,"error":error}), None))
        }
    };
    // main prints this final envelope exactly once. A valid script failure is
    // a successful protocol exchange, independent of the script's exit code.
    let mut envelope = json!({"ok":true,"scriptSupervisor":rustcast_core::script::SUPERVISOR_VERSION,"capture":capture});
    envelope["response"] = response;
    Ok(envelope)
}

fn execute(args: &[String]) -> Result<Value> {
    let name = args.first().map(String::as_str).unwrap_or("--help");
    match name {
        "--version" | "version" if args.len() == 1 => Ok(json!({"ok":true,"result":{"name":"Ruycast","version":rustcast_core::product_version::CURRENT,"implementation":"rust","component":"cli"}})),
        "--help" | "help" | "-h" => Ok(
            json!({"ok":true,"commands":["status","performance [summary|reset --confirm]","search-ranking [summary|reset --confirm]","logs [--command ID] [--errors] [--limit N] | path | clear --confirm","analytics [summary|export|pause|resume|retention DAYS|reset --confirm]","monitor","processes [query]","commands","search [query] [--mode MODE]","run ID [--dry-run] [--input JSON_OR_TEXT] [--confirm] [--foreground ORIGINAL_JSON]","snippets list [query]|add JSON|remove ID","quicklinks list [query]|add JSON|remove ID","window LAYOUT","show [commands|clipboard|snippets|quicklinks|files|menus|windows|processes|settings|analytics]","usage","reload","start [--headless]","quit","config path|get|init|validate [PATH]|reload", "config directory [ABSOLUTE_PATH]","config set KEY JSON [--expect-sha256 SHA]","config replace @FILE --expect-sha256 SHA","config add JSON_OR_@FILE [--expect-sha256 SHA]","config remove ID [--expect-sha256 SHA]","clipboard list [query]|export [query]|import JSON_OR_@FILE|copy ID|paste ID|delete ID|clear|pause|resume"],"profile":root()?,"configFormat":"Readable JSON, version 1"}),
        ).map(|mut value|{value["commands"].as_array_mut().expect("help command list").extend([
            json!("utilities import JSON_OR_@FILE"),
            json!("config import-commands SOURCE [--dry-run] [--without-hotkeys]"),
            json!("config activate-imported-hotkeys --dry-run | --expect-sha256 SHA"),
        ]);value}),
        "__run-script" => {
            let mut data = String::new();
            io::stdin()
                .take(2 * 1024 * 1024)
                .read_to_string(&mut data)
                .map_err(|e| e.to_string())?;
            run_script(parse(data.as_bytes())?)
        }
        "__run-script-supervised" if args.len() == 1 => supervised_script(),
        "config" => {
            let action = argument(args, 1, "config action")?;
            let path = config_directory()?.join("config.json");
            match action {
                "path" => Ok(json!({"ok":true,"result":path})),
                "directory" => match args.get(2) {
                    Some(path) => relocate_config(path),
                    None => Ok(json!({"ok":true,"result":config_directory()?})),
                },
                "get" => {
                    let data = read(&path)?;
                    Ok(
                        json!({"ok":true,"result":parse(&data)?,"sha256":format!("{:x}",Sha256::digest(&data))}),
                    )
                }
                "init" => {
                    private_dir(&root()?)?;
                    if !path.exists() {
                        if root()?.join("config-location.json").exists()
                            && config_override()?.is_none_or(|v| v.is_empty())
                        {
                            return Err("Selected configuration is unavailable. Restore the folder or fix config-location.json.".into());
                        }
                        private_storage::write(
                            &path,
                            serde_json::to_string_pretty(&default_config())
                                .unwrap()
                                .as_bytes(),
                            false,
                        )?;
                    }
                    Ok(json!({"ok":true,"path":path}))
                }
                "validate" => {
                    let path = args.get(2).map(PathBuf::from).unwrap_or(path);
                    let c = validate(&parse(&read(&path)?)?)?;
                    Ok(json!({"ok":true,"commands":c.commands.len()}))
                }
                "reload" => rpc(json!({"op":"reload"})),
                _ => config_edit(args),
            }
        }
        "start" => {
            if let Ok(v) = rpc(json!({"op":"status"})) {
                return Ok(v);
            }
            private_dir(&root()?)?;
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)
                .open(root()?.join("launcher.log"))
                .map_err(|e| e.to_string())?;
            let metadata=log.metadata().map_err(|e|e.to_string())?;
            if !metadata.is_file()||metadata.uid()!=unsafe{libc::getuid()}||metadata.nlink()!=1 {
                return Err("Launcher log must be an owned regular file without hard links.".into());
            }
            let mut cmd = Command::new(binary()?);
            cmd.env("RUYCAST_HOME", root()?)
                .stdin(Stdio::null())
                .stdout(log.try_clone().map_err(|e| e.to_string())?)
                .stderr(log);
            if args.iter().any(|a| a == "--headless") {
                cmd.arg("--headless");
            }
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            for _ in 0..60 {
                if let Ok(v) = rpc(json!({"op":"status"})) {
                    if v["ok"] == true {
                        return Ok(v);
                    }
                };
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err("Ruycast could not start; inspect the profile's launcher.log.".into())
        }
        "run" => {
            let mut request=json!({"op":"execute","id":argument(args,1,"command id")?,"confirmed":args.iter().any(|s|s=="--confirm"),"dryRun":args.iter().any(|s|s=="--dry-run"),"values":if let Some(i)=args.iter().position(|s|s=="--input") {let input=argument(args,i+1,"input")?;parse(input.as_bytes()).ok().filter(|v|v.is_object()).unwrap_or_else(||json!({"input":input}))} else {json!({})}});
            if let Some(i)=args.iter().position(|s|s=="--foreground"){
                let foreground=parse(argument(args,i+1,"original foreground JSON")?.as_bytes())?;
                let valid=foreground.as_object().is_some_and(|fields|fields.len()==4&&fields.keys().all(|k|["pid","uid","seconds","micros"].contains(&k.as_str())))&&foreground["pid"].as_i64().is_some_and(|pid|(2..=i32::MAX as i64).contains(&pid))&&foreground["uid"].as_u64().is_some_and(|uid|uid<=u32::MAX as u64)&&foreground["seconds"].as_u64().is_some()&&foreground["micros"].as_u64().is_some_and(|n|n<1_000_000);
                if !valid{return Err("Expected original foreground {pid,uid,seconds,micros} from menu search.".into());}
                request["foreground"]=foreground;
            }
            rpc(request)
        },
        "window" => {
            let layout = argument(args, 1, "layout")?;
            if !LAYOUTS.contains(&layout) {
                return Err(format!("Unknown layout: {layout}"));
            }
            rpc(json!({"op":"execute","id":format!("window.{layout}")}))
        }
        "show" => rpc(
            json!({"op":"show","mode":args.get(1).map(|s|s.trim_start_matches("--")).unwrap_or("commands")}),
        ),
        "snippets" | "quicklinks" => {
            let action = argument(args, 1, "list|add|remove")?;
            match action {
                "list" => {
                    let mut result = rpc(
                        json!({"op":"search","query":args.get(2).map(String::as_str).unwrap_or("")}),
                    )?;
                    if let Some(items) = result["result"].as_array_mut() {
                        items.retain(|c| {
                            if name == "snippets" {
                                c["kind"] == "snippet"
                            } else {
                                c["kind"] == "url" || c["kind"] == "path"
                            }
                        });
                    }
                    Ok(result)
                }
                "add" | "remove" => config_edit(
                    &[vec!["config".into(), action.into()], args[2..].to_vec()].concat(),
                ),
                _ => Err("Expected list, add or remove.".into()),
            }
        }
        "search" => {
            let mode = args
                .iter()
                .position(|s| s == "--mode")
                .map(|i| argument(args, i + 1, "search mode"))
                .transpose()?
                .unwrap_or("commands");
            let query = args
                .get(1)
                .filter(|s| s.as_str() != "--mode")
                .map(String::as_str)
                .unwrap_or("");
            rpc(json!({"op":"search","query":query,"mode":mode}))
        }
        "utilities" => {
            if args.len()!=3 || args[1]!="import" { return Err("Use utilities import JSON_OR_@FILE".into()); }
            let profile=if let Some(path)=args[2].strip_prefix('@'){
                let file=OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK|libc::O_CLOEXEC).open(path).map_err(|e|e.to_string())?;
                let info=file.metadata().map_err(|e|e.to_string())?;
                if !info.is_file()||info.len()>1_048_576{return Err("Utilities import must be a regular JSON file no larger than 1 MiB.".into());}
                let mut bytes=vec![];file.take(1_048_577).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                if bytes.len()>1_048_576{return Err("Utilities import exceeds 1 MiB.".into());}parse(&bytes)?
            }else{parse(args[2].as_bytes())?};
            if !profile.is_object(){return Err("Expected a Utilities profile object.".into());}
            let request=json!({"op":"utilities.import","profile":profile});
            if request.to_string().len()+1>1_048_576{return Err("Utilities import exceeds the 1 MiB local request limit.".into());}
            rpc(request)
        }
        "clipboard" => {
            let action = argument(args, 1, "clipboard action")?;
            match action {
                "list" => rpc(
                    json!({"op":"clipboard.list","query":args.get(2).map(String::as_str).unwrap_or("")}),
                ),
                "export" => rpc(json!({"op":"clipboard.export","query":args.get(2).map(String::as_str).unwrap_or("")})),
                "import" => rpc(clipboard_import_request(argument(args,2,"clipboard items JSON or @FILE")?)?),
                "copy" | "paste" | "delete" => rpc(
                    json!({"op":format!("clipboard.{action}"),"id":argument(args,2,"clipboard id")?}),
                ),
                "clear" => rpc(json!({"op":"clipboard.clear"})),
                "pause" | "resume" => rpc(json!({"op":"clipboard.pause","paused":action=="pause"})),
                _ => Err("Unknown clipboard action.".into()),
            }
        }
        "logs" => {
            if args.get(1).map(String::as_str) == Some("path") {
                return rpc(json!({"op":"logs.path"}));
            }
            if args.get(1).map(String::as_str) == Some("clear") {
                if args.len() != 3 || args.get(2).map(String::as_str) != Some("--confirm") {
                    return Err("Use logs clear --confirm".into());
                }
                return rpc(json!({"op":"logs.clear","confirmed":true}));
            }
            let mut request = json!({"op":"logs"});
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--command" => {
                        i += 1;
                        request["commandID"] = json!(args.get(i).ok_or("Expected command ID")?);
                    }
                    "--errors" => {
                        request["errorsOnly"] = json!(true);
                    }
                    "--limit" => {
                        i += 1;
                        let limit = args
                            .get(i)
                            .ok_or("Expected log limit")?
                            .parse::<u32>()
                            .map_err(|_| "Expected limit1–500")?;
                        if !(1..=500).contains(&limit) {
                            return Err("Expected limit1–500".into());
                        }
                        request["limit"] = json!(limit);
                    }
                    _ => return Err("Use logs [--command ID] [--errors] [--limit1–500]".into()),
                };
                i += 1;
            }
            rpc(request)
        }
        "performance" => match args.get(1).map(String::as_str) {
            None | Some("summary") if args.len() <= 2 => rpc(json!({"op":"performance"})),
            Some("reset") if args.len() == 3 && args[2] == "--confirm" => {
                rpc(json!({"op":"performance.reset"}))
            }
            _ => Err("Use performance [summary|reset --confirm]".into()),
        },
        "search-ranking" => match args.get(1).map(String::as_str) {
            None | Some("summary") if args.len() <= 2 => rpc(json!({"op":"search-ranking"})),
            Some("reset") if args.len() == 3 && args[2] == "--confirm" => {
                rpc(json!({"op":"search-ranking.reset","confirmed":true}))
            }
            _ => Err("Use search-ranking [summary|reset --confirm]".into()),
        },
        "analytics" => match args.get(1).map(String::as_str) {
            None | Some("summary") => rpc(json!({"op":"analytics"})),
            Some("export") => rpc(json!({"op":"analytics.export"})),
            Some("pause") => rpc(json!({"op":"analytics.settings","enabled":false})),
            Some("resume") => rpc(json!({"op":"analytics.settings","enabled":true})),
            Some("retention") => {
                let days = args
                    .get(2)
                    .ok_or("Expected retention days")?
                    .parse::<u32>()
                    .map_err(|_| "Expected 1–90 days")?;
                if !(1..=90).contains(&days) {
                    return Err("Expected 1–90 days".into());
                };
                rpc(json!({"op":"analytics.settings","retentionDays":days}))
            }
            Some("reset") if args.get(2).map(String::as_str) == Some("--confirm") => {
                rpc(json!({"op":"analytics.reset","confirmed":true}))
            }
            _ => Err(
                "Use: analytics [summary|export|pause|resume|retention DAYS|reset --confirm]"
                    .into(),
            ),
        },
        "monitor" => rpc(json!({"op":"monitor"})),
        "processes" => {
            rpc(json!({"op":"processes","query":args.get(1).map(|s|s.as_str()).unwrap_or("")}))
        }
        "status" | "commands" | "usage" | "reload" | "quit" => rpc(json!({"op":name})),
        _ => Err(format!("Unknown command: {name}. Run ruycast --help.")),
    }
}
fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let result = execute(&args).unwrap_or_else(|e| json!({"ok":false,"error":e}));
    println!("{result}");
    if result["ok"] != true {
        std::process::exit(1)
    }
}
#[cfg(test)]mod startup_discovery_tests{
    use super::*;
    fn bound_test_binary(path_key:&str,sha_key:&str)->(PathBuf,String) {
        let path=PathBuf::from(env::var_os(path_key).unwrap_or_else(||panic!("Builder must supply {path_key}")));
        let sha=env::var(sha_key).unwrap_or_else(|_|panic!("Builder must supply {sha_key}"));
        assert!(path.is_absolute()&&fs::symlink_metadata(&path).unwrap().is_file(),"Builder-bound binary must be an absolute regular file");
        assert!(sha.len()==64&&sha.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)),"Expected builder-bound lowercase SHA256");
        assert_test_binary(&path,&sha);(path,sha)
    }
    fn assert_test_binary(path:&Path,sha:&str) {
        assert_eq!(format!("{:x}",Sha256::digest(fs::read(path).unwrap())),sha,"Builder-bound binary changed");
    }
    #[test]
    fn clipboard_import_refuses_owned_fifo_without_waiting_for_a_writer() {
        use std::os::unix::{ffi::OsStrExt, fs::DirBuilderExt};
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild { fn drop(&mut self) { if self.0.try_wait().ok().flatten().is_none() { let _ = self.0.kill(); } let _ = self.0.wait(); } }
        let parent=PathBuf::from(env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Explicit owned fixture parent required"));
        assert!(parent.is_absolute() && fs::symlink_metadata(&parent).unwrap().is_dir());
        let path=parent.join(format!("cli-fifo-{}",std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let owned=Cleanup(path);
        let fifo=owned.0.join("clipboard.json");
        let raw=std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(raw.as_ptr(),0o600)},0);
        let binary=PathBuf::from(env::var_os("RUYCAST_TEST_CLI").expect("Explicit freshly built validation CLI required"));
        assert!(binary.is_absolute());
        let mut child=OwnedChild(Command::new(binary).args(["clipboard","import",&format!("@{}",fifo.display())])
            .env_clear().env("HOME",&owned.0).env("RUYCAST_HOME",owned.0.join("p")).env("PATH","/usr/bin:/bin")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
        let deadline=std::time::Instant::now()+Duration::from_secs(3);
        let status=loop {
            if let Some(status)=child.0.try_wait().unwrap(){break Some(status);}
            if std::time::Instant::now()>=deadline{break None;}
            thread::sleep(Duration::from_millis(5));
        };
        assert!(status.is_some(),"Clipboard @FIFO import blocked waiting for a writer before any RPC; owned child is killed/reaped by guard");
        assert_eq!(status.unwrap().code(),Some(1));
        let mut stdout=String::new();child.0.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
        let value:Value=serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["ok"],false);
        assert!(value["error"].as_str().unwrap().contains("regular"),"Must reject the input type before an unrelated missing-runtime error: {value}");
        assert!(!owned.0.join("p").exists(),"Failed input must not create a profile");
    }
    #[test]
    fn clipboard_import_bounds_ast_before_allocation_without_rewriting_json() {
        let bytes=format!("[{}null]","null,".repeat(1000)).into_bytes();
        let mut input=ImportReader::new(std::io::Cursor::new(&bytes),64);
        let error=serde_json::from_reader::<_,Value>(&mut input).unwrap_err();
        assert!(error.to_string().contains("before allocation"));
        assert!(input.input.position()<(bytes.len() as u64),"Reject while consuming, before a complete oversized AST");
        let number=format!("1.{}","0".repeat(10_000));
        assert_eq!(serde_json::from_reader::<_,Value>(ImportReader::new(number.as_bytes(),8)).unwrap(),json!(1.0));
        for text in [r#""\\\\\\\"[]{}""#,r#""\u00c5\ud83d\ude00""#] {
            assert_eq!(serde_json::from_reader::<_,Value>(ImportReader::new(text.as_bytes(),64)).unwrap(),parse(text.as_bytes()).unwrap());
        }
    }
    #[test]
    fn offline_cli_config_lock_refuses_link_without_creating_an_external_file() {
        use std::os::unix::fs::DirBuilderExt;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
        let parent=PathBuf::from(env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Owned fixture parent required"));
        assert!(parent.is_absolute());
        let base=parent.join(format!("cli-lock-{}",std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();let owned=Cleanup(base);
        fs::DirBuilder::new().mode(0o700).create(owned.0.join("p")).unwrap();
        let config=serde_json::to_vec(&default_config()).unwrap();fs::write(owned.0.join("p/config.json"),&config).unwrap();
        let outside=owned.0.join("must-not-create.lock");
        std::os::unix::fs::symlink(&outside,owned.0.join("p/config.lock")).unwrap();
        let cli=PathBuf::from(env::var_os("RUYCAST_TEST_CLI").expect("Fresh validation CLI required"));assert!(cli.is_absolute());
        let output=Command::new(cli).args(["config","set","theme","\"dark\""]).env_clear()
            .env("HOME",&owned.0).env("RUYCAST_HOME",owned.0.join("p")).env("PATH","/usr/bin:/bin")
            .stdin(Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(),Some(1));assert!(output.stderr.is_empty());
        assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap()["ok"],false);
        assert!(!outside.exists());assert_eq!(fs::read(owned.0.join("p/config.json")).unwrap(),config);
    }
    #[test]
    fn clipboard_import_keeps_large_pretty_exports_and_checks_compact_wire_and_input_types() {
        use std::os::unix::fs::{DirBuilderExt,PermissionsExt,symlink};
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
        let parent=PathBuf::from(env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Explicit owned fixture parent required"));
        assert!(parent.is_absolute());
        let path=parent.join(format!("cli-import-{}",std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();let owned=Cleanup(path);
        let items=json!([{"id":"raycast-owned","text":"Owned Å","source":"owned","date":"2026-11-01T00:00:00Z"}]);
        let expected=json!({"op":"clipboard.import","items":items});
        assert_eq!(clipboard_import_request(&items.to_string()).unwrap(),expected,"Full JSON argv contract");
        let file=owned.0.join("pretty.json");
        let pretty=format!("{}{}", " ".repeat(1_048_577),serde_json::to_string_pretty(&items).unwrap().replace('Å',"\\u00c5"));
        fs::write(&file,pretty).unwrap();
        fs::set_permissions(&file,fs::Permissions::from_mode(0o400)).unwrap();
        assert!(fs::metadata(&file).unwrap().len()>1_048_576);
        assert_eq!(clipboard_import_request(&format!("@{}",file.display())).unwrap(),expected,"Raw whitespace/escapes above 1 MiB must not reject a valid compact request");
        let link=owned.0.join("linked.json");symlink(&file,&link).unwrap();
        assert!(clipboard_import_request(&format!("@{}",link.display())).is_err());
        assert!(clipboard_import_request(&format!("@{}",owned.0.display())).unwrap_err().contains("regular"));
        let oversized=owned.0.join("oversized.json");
        let sparse=fs::File::create(&oversized).unwrap();sparse.set_len(256*1024*1024+1).unwrap();
        assert!(clipboard_import_request(&format!("@{}",oversized.display())).unwrap_err().contains("256 MiB"));
        let large=json!([{"text":"x".repeat(1_048_576)}]);
        assert!(clipboard_import_request(&large.to_string()).unwrap_err().contains("1 MiB"));
        assert!(clipboard_import_request("[1 2]").is_err(),"Streaming must retain JSON token boundaries");
        assert!(!owned.0.join("p").exists());
    }
    #[test]
    #[ignore = "Requires the mandatory late CLI parity matrix route with fresh builder-bound CLI and fixture binaries"]
    fn current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects() {
        use std::os::unix::fs::DirBuilderExt;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild { fn drop(&mut self) { if self.0.try_wait().ok().flatten().is_none() { let _=self.0.kill(); } let _=self.0.wait(); } }
        let parent=PathBuf::from(env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Explicit owned fixture parent required"));
        assert!(parent.is_absolute());
        let base=parent.join(format!("cli-roundtrip-{}",std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();let owned=Cleanup(base);
        fs::write(owned.0.join(".ruycast-test-fixture"),b"own-disposable-fixture-v1").unwrap();
        let apps=owned.0.join("Applications");fs::create_dir(&apps).unwrap();
        let (cli,cli_sha)=bound_test_binary("RUYCAST_CLI_PARITY_BINARY","RUYCAST_CLI_PARITY_BINARY_SHA256");
        let (host,host_sha)=bound_test_binary("RUYCAST_CLI_PARITY_HOST","RUYCAST_CLI_PARITY_HOST_SHA256");
        let run=|args:&[&str]| {
            let mut child=OwnedChild(Command::new(&cli).args(args).env_clear()
                .env("HOME",&owned.0).env("RUYCAST_HOME",owned.0.join("p")).env("PATH","/usr/bin:/bin")
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
            let deadline=std::time::Instant::now()+Duration::from_secs(8);
            let status=loop {
                if let Some(status)=child.0.try_wait().unwrap(){break status;}
                assert!(std::time::Instant::now()<deadline,"Owned CLI operation did not finish: {args:?}");
                thread::sleep(Duration::from_millis(5));
            };
            let mut stdout=String::new();child.0.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
            let mut stderr=String::new();child.0.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
            assert!(stderr.is_empty(),"CLI contract writes envelopes to stdout: {stderr}");
            assert_eq!(stdout.lines().count(),1,"One complete JSON envelope per command");
            let value:Value=serde_json::from_str(&stdout).unwrap();
            assert_eq!(status.code(),Some(if value["ok"]==true {0}else{1}),"JSON/exit contract: {value}");
            value
        };
        assert_eq!(run(&["config","init"])["ok"],true);
        assert_eq!(run(&["config","set","theme","\"light\""])["ok"],true,"Healthy offline lock/edit path");
        assert_eq!(run(&["config","validate"])["ok"],true);
        let mut config=default_config();
        config["commands"]=json!([{"id":"owned.cli","title":"Owned CLI command","kind":"script","target":host,"arguments":["fake","echo",owned.0.join("must-not-run"),"owned"],"aliases":["owned-alias"]}]);
        fs::write(owned.0.join("p/config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
        let mut server=OwnedChild(Command::new(&host).args(["host",owned.0.to_str().unwrap()]).env_clear()
            .env("HOME",&owned.0).env("RUYCAST_NATIVE_TEST_APP_ROOTS",json!([apps]).to_string()).env("PATH","/usr/bin:/bin")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        while !owned.0.join("p/control.sock").exists(){assert!(server.0.try_wait().unwrap().is_none(),"Owned headless fixture failed to start");assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
        let status=run(&["status"]);assert_eq!(status["ok"],true);assert_eq!(status["result"]["headless"],true);
        for key in ["version","name","profile","configPath","configDirectory","snippetExpansion","keyboard","hyper","modifierTaps","theme","health"] {
            assert!(status["result"][key].is_string(),"Original status string field missing: {key}: {status}");
        }
        for key in ["headless","clipboardEnabled","clipboardPaused","accessibility","inputMonitoring","separateCommandKeys","menuBarVisible"] {
            assert!(status["result"][key].is_boolean(),"Original status boolean field missing: {key}: {status}");
        }
        for key in ["pid","commands","discoveredApps","clipboardCount"] {
            assert!(status["result"][key].as_u64().is_some(),"Original status count field missing: {key}: {status}");
        }
        assert!(status["result"]["hotkeyWarnings"].is_array());
        for key in ["configError","clipboardCleanupError"] { assert!(status["result"][key].is_null()||status["result"][key].is_string()); }
        assert!(["healthy","needs-attention"].iter().any(|s|status["result"]["health"]==*s));
        assert_eq!(status["result"]["profile"],owned.0.join("p").to_string_lossy().as_ref());
        assert_eq!(run(&["config","set","theme","\"dark\""])["ok"],true,"Healthy live activation still uses the one owner");
        let expected_ranking = json!({"entries":0,"limit":512,"retentionDays":90});
        for args in [vec!["search-ranking"], vec!["search-ranking", "summary"]] {
            let summary = run(&args);
            assert_eq!(summary["ok"], true);
            assert_eq!(summary["result"], expected_ranking);
        }
        assert_eq!(run(&["search-ranking", "reset"])["ok"], false);
        assert_eq!(run(&["search-ranking", "summary", "unexpected"])["ok"], false);
        let ranking_reset = run(&["search-ranking", "reset", "--confirm"]);
        assert_eq!(ranking_reset["ok"], true);
        assert_eq!(ranking_reset["result"]["entries"], 0);
        assert_eq!(run(&["run","owned-alias","--dry-run"])["dryRun"],true);
        assert!(!owned.0.join("must-not-run").exists());assert_eq!(run(&["usage"])["result"],json!({}));
        let snippet=json!({"id":"snippet.test","title":"Greeting","kind":"snippet","target":"Hello {cursor}","keyword":";hello"});
        assert_eq!(run(&["snippets","add",&snippet.to_string()])["ok"],true);
        let snippets=run(&["snippets","list"]);
        assert_eq!(snippets["result"].as_array().unwrap().iter().map(|row|row["id"].as_str().unwrap()).collect::<Vec<_>>(),vec!["snippet.test"]);
        assert_eq!(run(&["search",";hello"])["result"][0]["id"],"snippet.test");
        let link=json!({"id":"link.test","title":"Find","kind":"url","target":"https://example.com/?q={query}","aliases":["find-test"]});
        assert_eq!(run(&["quicklinks","add",&link.to_string()])["ok"],true);
        assert_eq!(run(&["quicklinks","list","find-test"])["result"][0]["target"],link["target"]);
        let input_marker=owned.0.join("literal-input-runs");
        let input=json!({"id":"test.input","title":"Input","kind":"script","target":host,"arguments":["fake","echo",input_marker,"{input}"],"inputPrompt":"Text"});
        assert_eq!(run(&["config","add",&input.to_string()])["ok"],true);
        assert_eq!(run(&["run","test.input"])["ok"],false);
        assert!(!input_marker.exists());
        let literal="literal $(do not run)\nΩ";
        assert_eq!(run(&["run","test.input","--input",&json!({"input":literal}).to_string()])["result"]["stdout"],literal);
        assert_eq!(run(&["run","test.input","--input","123"])["result"]["stdout"],"123");
        assert_eq!(fs::read_to_string(&input_marker).unwrap(),"run\nrun\n");
        assert_eq!(run(&["snippets","remove","snippet.test"])["ok"],true);
        assert_eq!(run(&["snippets","list"])["result"],json!([]));
        assert_eq!(run(&["config","set","snippetExpansionEnabled","true"])["ok"],true);
        assert_eq!(run(&["config","get"])["result"]["snippetExpansionEnabled"],true);
        // Original CLI conjuncts use the real frontend and the same owned
        // headless owner; none executes a snippet, URL, app or hotkey.
        {
            let rich=json!({"id":"test.rich-snippet","title":"Rich fixture","kind":"snippet","target":"Hello\n\n[Guide](https://example.com/?a=1&b=2)","keyword":";fixture-rich","snippetOutput":"teams"});
            assert_eq!(run(&["config","add",&rich.to_string()])["ok"],true);
            for unrelated_save in [false,true] {
                if unrelated_save {assert_eq!(run(&["config","set","theme","\"light\""])["ok"],true);}
                let current=run(&["config","get"]);
                let saved=current["result"]["commands"].as_array().unwrap().iter().find(|c|c["id"]==rich["id"]).unwrap();
                assert_eq!(saved["snippetOutput"],"teams");
                assert_eq!(saved["target"],rich["target"]);
            }
            let before=run(&["config","get"])["sha256"].clone();
            let mut invalid=rich.clone();invalid["id"]=json!("test.bad-output");invalid["keyword"]=json!(";bad-output");invalid["snippetOutput"]=json!("unknown");
            assert_eq!(run(&["config","add",&invalid.to_string()])["ok"],false);
            let mut wrong_kind=rich.clone();wrong_kind["id"]=json!("test.bad-kind");wrong_kind["kind"]=json!("url");wrong_kind["target"]=json!("https://example.com/");wrong_kind.as_object_mut().unwrap().remove("keyword");
            assert_eq!(run(&["config","add",&wrong_kind.to_string()])["ok"],false);
            assert_eq!(run(&["config","get"])["sha256"],before,"Rejected snippet output/kind must not publish configuration");
            assert_eq!(run(&["config","remove","test.rich-snippet"])["ok"],true);
        }
        {
            let snapshot=run(&["config","get"]);
            let mut candidate=snapshot["result"].clone();candidate["hyperKeyEnabled"]=json!(true);
            let chosen=json!({"id":"chosen-app","title":"Chosen app","kind":"url","target":"https://example.com/","applicationID":"com.apple.Safari","hotkey":"ctrl+alt+delete"});
            candidate["commands"].as_array_mut().unwrap().push(chosen.clone());
            let file=owned.0.join("cas-candidate.json");fs::write(&file,serde_json::to_vec(&candidate).unwrap()).unwrap();
            let input=format!("@{}",file.display());
            assert_eq!(run(&["config","replace",&input])["ok"],false);
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",&"0".repeat(64)])["ok"],false);
            assert_eq!(run(&["config","get"])["sha256"],snapshot["sha256"]);
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",snapshot["sha256"].as_str().unwrap()])["ok"],true);
            let published=run(&["config","get"]);
            assert_eq!(published["result"]["hyperKeyEnabled"],true);
            let saved=published["result"]["commands"].as_array().unwrap().iter().find(|c|c["id"]=="chosen-app").unwrap();
            for key in ["id","title","kind","target","applicationID","hotkey"] {assert_eq!(saved[key],chosen[key]);}
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",snapshot["sha256"].as_str().unwrap()])["ok"],false);
            assert_eq!(run(&["config","get"])["sha256"],published["sha256"],"A stale CAS must preserve the newer complete configuration");
            fs::write(&file,serde_json::to_vec(&snapshot["result"]).unwrap()).unwrap();
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",published["sha256"].as_str().unwrap()])["ok"],true);
        }
        {
            let snapshot=run(&["config","get"]);
            let mut legacy=snapshot["result"].clone();legacy.as_object_mut().unwrap().remove("separateCommandKeys");
            let file=owned.0.join("legacy-command-keys.json");fs::write(&file,serde_json::to_vec(&legacy).unwrap()).unwrap();
            let input=format!("@{}",file.display());
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",snapshot["sha256"].as_str().unwrap()])["ok"],true);
            assert_eq!(run(&["status"])["result"]["separateCommandKeys"],true,"Absent legacy setting must retain its original default");
            for enabled in [false,true,false] {
                assert_eq!(run(&["config","set","separateCommandKeys",&enabled.to_string()])["ok"],true);
                assert_eq!(run(&["config","get"])["result"]["separateCommandKeys"],enabled);
                assert_eq!(run(&["status"])["result"]["separateCommandKeys"],enabled);
            }
            let current=run(&["config","get"]);
            assert_eq!(run(&["config","set","separateCommandKeys","\"invalid\""])["ok"],false);
            assert_eq!(run(&["config","get"])["sha256"],current["sha256"]);
            assert_eq!(run(&["status"])["result"]["separateCommandKeys"],false);
            fs::write(&file,serde_json::to_vec(&snapshot["result"]).unwrap()).unwrap();
            assert_eq!(run(&["config","replace",&input,"--expect-sha256",current["sha256"].as_str().unwrap()])["ok"],true);
        }
        {
            let snapshot=run(&["config","get"]);
            std::thread::scope(|scope| {
                let first=scope.spawn(||run(&["config","set","clipboard.limit","77"]));
                let second=scope.spawn(||run(&["config","set","clipboard.retentionDays","12"]));
                assert_eq!(first.join().unwrap()["ok"],true,"The first concurrent CLI edit must succeed");
                assert_eq!(second.join().unwrap()["ok"],true,"The second concurrent CLI edit must succeed");
            });
            let current=run(&["config","get"]);
            assert_eq!(current["result"]["clipboard"]["limit"],77);
            assert_eq!(current["result"]["clipboard"]["retentionDays"],12);
            let file=owned.0.join("concurrent-restore.json");fs::write(&file,serde_json::to_vec(&snapshot["result"]).unwrap()).unwrap();
            assert_eq!(run(&["config","replace",&format!("@{}",file.display()),"--expect-sha256",current["sha256"].as_str().unwrap()])["ok"],true);
        }
        {
            let command=json!({"id":"test.added","title":"Added","kind":"path","target":"/","aliases":["added"]});
            let usage=run(&["usage"])["result"].clone();
            assert_eq!(run(&["config","add",&command.to_string()])["ok"],true);
            let dry=run(&["run","added","--dry-run"]);
            assert_eq!(dry["ok"],true);assert_eq!(dry["dryRun"],true);assert_eq!(dry["result"]["id"],"test.added");
            let before=run(&["config","get"])["sha256"].clone();
            assert_eq!(run(&["config","add",&command.to_string()])["ok"],false);
            assert_eq!(run(&["config","get"])["sha256"],before);
            assert_eq!(run(&["config","remove","test.added"])["ok"],true);
            assert_eq!(run(&["run","added","--dry-run"])["ok"],false);
            assert_eq!(run(&["usage"])["result"],usage,"Alias dry run must not count an execution");
        }
        {
            let file=owned.0.join("p/config.json");let before=fs::read(&file).unwrap();
            let mut invalid:Value=serde_json::from_slice(&before).unwrap();invalid["misspelledSetting"]=json!(true);
            fs::write(&file,serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert_eq!(run(&["config","validate"])["ok"],false);
            assert_eq!(run(&["reload"])["ok"],false);
            assert_eq!(run(&["search","owned-alias"])["result"][0]["id"],"owned.cli","An invalid external field must preserve the last-good runtime catalog");
            assert_eq!(run(&["status"])["result"]["headless"],true);
            fs::write(&file,&before).unwrap();assert_eq!(run(&["reload"])["ok"],true);
        }
        {
            use std::{io::BufRead,os::unix::net::UnixStream};
            // The original unknown-operation case is a valid wire request,
            // distinct from CLI syntax or malformed-JSON refusal.
            let mut stream=UnixStream::connect(owned.0.join("p/control.sock")).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
            stream.write_all(b"{\"op\":\"missing\"}\n").unwrap();
            let mut line=String::new();
            std::io::BufReader::new(stream.take(8193)).read_line(&mut line).unwrap();
            assert!(line.len()<=8192&&line.ends_with('\n'));
            let response:Value=serde_json::from_str(&line).unwrap();
            assert_eq!(response["ok"],false);assert!(response["error"].is_string());
        }
        let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let items=json!([{"id":"raycast-cli-owned","text":"Owned CLI Å","source":"owned","date":chrono::DateTime::from_timestamp(now,0).unwrap().to_rfc3339()}]);
        let file=owned.0.join("pretty-export.json");
        fs::write(&file,format!("{}{}"," ".repeat(1_048_577),serde_json::to_string_pretty(&items).unwrap())).unwrap();
        assert_eq!(run(&["clipboard","import",&format!("@{}",file.display())])["result"]["accepted"],1);
        assert_eq!(run(&["clipboard","export"])["result"],items);
        assert_eq!(run(&["clipboard","import",&items.to_string()])["result"]["deduplicated"],1);
        assert_eq!(run(&["clipboard","list","cli å"])["result"],items);
        assert_eq!(run(&["clipboard","delete","raycast-cli-owned"])["result"]["count"],0);
        assert_eq!(run(&["clipboard","clear"])["ok"],true);
        assert_eq!(run(&["run","no-such-owned-command"])["ok"],false);
        assert_eq!(run(&["quit"])["ok"],true);
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        loop {if let Some(status)=server.0.try_wait().unwrap(){assert!(status.success());break;}assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
        assert!(!owned.0.join("p/control.sock").exists());
        let configuration=owned.0.join("p/config.json");
        let before=fs::read(&configuration).unwrap();
        let lock=owned.0.join("p/config.lock");
        if lock.exists(){assert!(fs::symlink_metadata(&lock).unwrap().is_file());fs::remove_file(&lock).unwrap();}
        let outside=owned.0.join("must-not-create-outside-profile.lock");
        std::os::unix::fs::symlink(&outside,&lock).unwrap();
        let refused=run(&["config","set","theme","\"dark\""]);
        assert!(!outside.exists(),"A linked config.lock created a file outside its profile: {refused}");
        assert_eq!(refused["ok"],false);
        assert_eq!(fs::read(configuration).unwrap(),before,"Unsafe locking must fail before config publication");
        fs::remove_file(&lock).unwrap();
        let relocated=owned.0.join("Synced vault with spaces/.appdata/ruycast");
        let original_configuration=fs::read(owned.0.join("p/config.json")).unwrap();
        let original_history=fs::read(owned.0.join("p/clipboard.json")).unwrap();
        assert_eq!(run(&["config","directory",relocated.to_str().unwrap()])["ok"],true);
        assert_eq!(fs::read(relocated.join("config.json")).unwrap(),original_configuration);
        assert_eq!(fs::read(owned.0.join("p/config.json")).unwrap(),original_configuration);
        let selector=fs::read(owned.0.join("p/config-location.json")).unwrap();
        for _ in 0..2 {
            let mut restarted=OwnedChild(Command::new(&host).args(["host",owned.0.to_str().unwrap()]).env_clear()
                .env("HOME",&owned.0).env("RUYCAST_NATIVE_TEST_APP_ROOTS",json!([apps]).to_string()).env("PATH","/usr/bin:/bin")
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
            let deadline=std::time::Instant::now()+Duration::from_secs(5);
            while !owned.0.join("p/control.sock").exists(){assert!(restarted.0.try_wait().unwrap().is_none());assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
            let restarted_status=run(&["status"]);
            assert_eq!(restarted_status["ok"],true);
            assert_eq!(restarted_status["result"]["pid"],restarted.0.id());
            assert_eq!(restarted_status["result"]["configPath"],relocated.join("config.json").to_string_lossy().as_ref());
            assert_eq!(restarted_status["result"]["profile"],owned.0.join("p").to_string_lossy().as_ref());
            assert_eq!(run(&["quit"])["ok"],true);
            let deadline=std::time::Instant::now()+Duration::from_secs(5);
            loop {if let Some(status)=restarted.0.try_wait().unwrap(){assert!(status.success());break;}assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
            assert!(!owned.0.join("p/control.sock").exists());
            assert_eq!(fs::read(owned.0.join("p/config-location.json")).unwrap(),selector);
            assert_eq!(fs::read(owned.0.join("p/config.json")).unwrap(),original_configuration);
            assert_eq!(fs::read(owned.0.join("p/clipboard.json")).unwrap(),original_history);
            assert_eq!(fs::read(relocated.join("config.json")).unwrap(),original_configuration);
        }
        assert_test_binary(&cli,&cli_sha);assert_test_binary(&host,&host_sha);
    }
    #[test]
    #[ignore = "Requires the mandatory late CLI parity matrix route with fresh builder-bound CLI and native binaries"]
    fn current_cli_starts_one_real_headless_owner_and_quits_without_capture_or_input() {
        use std::os::unix::{fs::DirBuilderExt, process::CommandExt};
        struct OwnedGroup { base: PathBuf, group: std::cell::Cell<Option<i32>> }
        impl Drop for OwnedGroup {
            fn drop(&mut self) {
                if self.base.join("p/control.sock").exists() {
                    if let Some(group)=self.group.get() { unsafe { libc::kill(-group,libc::SIGKILL); } }
                }
                let _=fs::remove_dir_all(&self.base);
            }
        }
        let parent=PathBuf::from(env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Owned fixture parent required"));
        let (cli,cli_sha)=bound_test_binary("RUYCAST_CLI_PARITY_BINARY","RUYCAST_CLI_PARITY_BINARY_SHA256");
        let (host,host_sha)=bound_test_binary("RUYCAST_CLI_PARITY_NATIVE","RUYCAST_CLI_PARITY_NATIVE_SHA256");
        assert!(parent.is_absolute());
        let base=parent.join(format!("cli-start-{}",std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        let owned=OwnedGroup{base,group:std::cell::Cell::new(None)};
        fs::create_dir(owned.base.join("Applications")).unwrap();
        fs::DirBuilder::new().mode(0o700).create(owned.base.join("p")).unwrap();
        let mut config=default_config();
        config["commands"]=json!([]);config["clipboard"]["enabled"]=json!(false);
        config["snippetExpansionEnabled"]=json!(false);config["hyperKeyEnabled"]=json!(false);
        validate(&config).unwrap();
        let bytes=serde_json::to_vec(&config).unwrap();fs::write(owned.base.join("p/config.json"),&bytes).unwrap();
        let run=|args:&[&str],group:&mut Option<i32>| {
            let mut command=Command::new(&cli);
            command.args(args).env_clear().env("HOME",&owned.base).env("RUYCAST_HOME",owned.base.join("p"))
                .env("RUYCAST_APP_BINARY",&host).env("PATH","/usr/bin:/bin")
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            if group.is_none(){command.process_group(0);}
            let mut child=command.spawn().unwrap();
            if group.is_none(){*group=Some(i32::try_from(child.id()).unwrap());}
            owned.group.set(*group);
            let deadline=std::time::Instant::now()+Duration::from_secs(8);
            let status=loop {
                if let Some(status)=child.try_wait().unwrap(){break status;}
                if std::time::Instant::now()>=deadline {let _=child.kill();let _=child.wait();panic!("Owned CLI start/control deadline");}
                thread::sleep(Duration::from_millis(5));
            };
            let mut stdout=String::new();child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
            let mut stderr=String::new();child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();assert!(stderr.is_empty());
            assert!(status.success());assert_eq!(stdout.lines().count(),1);
            let response:Value=serde_json::from_str(&stdout).unwrap();assert_eq!(response["ok"],true);response
        };
        let mut group=None;
        let first=run(&["start","--headless"],&mut group);
        let pid=first["result"]["pid"].as_u64().unwrap();assert!(pid>1);
        let again=run(&["start","--headless"],&mut group);assert_eq!(again["result"]["pid"],pid);
        let status=run(&["status"],&mut group);
        assert_eq!(status["result"]["pid"],pid);assert_eq!(status["result"]["headless"],true);
        assert_eq!(status["result"]["profile"],owned.base.join("p").to_string_lossy().as_ref());
        assert_eq!(status["result"]["commands"],0);assert_eq!(status["result"]["clipboardEnabled"],false);
        assert_eq!(status["result"]["clipboardCaptureActive"],false);
        assert_eq!(fs::read(owned.base.join("p/config.json")).unwrap(),bytes);
        run(&["quit"],&mut group);
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        while owned.base.join("p/control.sock").exists(){assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
        assert_eq!(fs::read(owned.base.join("p/config.json")).unwrap(),bytes);
        assert_test_binary(&cli,&cli_sha);assert_test_binary(&host,&host_sha);
    }
    #[test]fn explicit_profile_validation_matches_native_planning(){
        for value in ["","../escape","/owned/../escape","bad\0path"] {assert!(absolute_profile(Path::new(value),Path::new("/owned/cwd")).is_err());}
        use std::os::unix::ffi::OsStringExt;
        assert!(absolute_profile(Path::new(&std::ffi::OsString::from_vec(vec![0xff])),Path::new("/owned/cwd")).is_err());
        assert_eq!(absolute_profile(Path::new("p"),Path::new("/owned/cwd")).unwrap(),Path::new("/owned/cwd/p"));
    }
    #[test]fn missing_or_empty_home_uses_only_injected_account_home(){
        assert_eq!(selected_home(Some("/owned/Å".into()),||panic!("unexpected account query")).unwrap(),Path::new("/owned/Å"));
        for value in [None,Some(std::ffi::OsString::new())]{assert_eq!(selected_home(value,||Ok("/owned/account".into())).unwrap().join(".config/ruycast"),Path::new("/owned/account/.config/ruycast"));}
        assert!(selected_home(None,||Err("unavailable".into())).is_err());
    }
    #[test]fn packaged_cli_prefers_override_then_same_bundle_without_swift_fallback(){
        let items=binary_candidates(Path::new("/Owned/Ruycast.app/Contents/Resources/ruycast"),Some("/Owned/explicit".into()),Path::new("/Owned/source"));
        assert_eq!(items[0],Path::new("/Owned/explicit"));assert_eq!(items[1],Path::new("/Owned/Ruycast.app/Contents/MacOS/RuycastApp"));
        assert_eq!(items[2],Path::new("/Applications/Ruycast.app/Contents/MacOS/RuycastApp"));
        assert!(items.iter().all(|p|!p.to_string_lossy().contains("/swift/")));
        assert!(items.iter().any(|p|p.ends_with("rust-native/target/debug/ruycast-native")));
        assert!(items.iter().any(|p|p.to_string_lossy().contains("/native/")&&p.ends_with("release/ruycast-native")));
    }
}
