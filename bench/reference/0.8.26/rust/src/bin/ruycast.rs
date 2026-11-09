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
        unix::{fs::OpenOptionsExt, net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into()))
}
fn root() -> PathBuf {
    env::var_os("RUYCAST_HOME")
        .or_else(|| env::var_os("RUSTCAST_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/ruycast"))
}
fn absolute_directory(value: &str) -> Result<PathBuf> {
    let path = if value == "~" {
        home()
    } else if let Some(tail) = value.strip_prefix("~/") {
        home().join(tail)
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
    if let Ok(value) = env::var("RUYCAST_CONFIG_DIR") {
        if !value.is_empty() {
            return absolute_directory(&value);
        }
    }
    let selector = root().join("config-location.json");
    if !selector.exists() {
        return Ok(root());
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
    if env::var("RUYCAST_CONFIG_DIR").is_ok_and(|v| !v.is_empty()) {
        return Err(
            "RUYCAST_CONFIG_DIR controls this configuration. Change or unset it first.".into(),
        );
    }
    if root().join("control.sock").exists() {
        return rpc(json!({"op":"config.directory","directory":path}));
    }
    let output = Command::new(binary()?)
        .args(["--set-config-directory", path])
        .env("RUYCAST_HOME", root())
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
    fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}
fn parse(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}
fn atomically(p: &Path, bytes: &[u8]) -> Result<()> {
    private_storage::write(p, bytes, true)
}
fn rpc(payload: Value) -> Result<Value> {
    let mut s = UnixStream::connect(root().join("control.sock")).map_err(|e| {
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
fn binary() -> Result<PathBuf> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let candidates = [
        env::var_os("RUYCAST_APP_BINARY")
            .or_else(|| env::var_os("RUSTCAST_APP_BINARY"))
            .map(PathBuf::from),
        exe.parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("MacOS/RuycastApp")),
        Some(PathBuf::from(
            "/Applications/Ruycast.app/Contents/MacOS/RuycastApp",
        )),
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".build/swift/debug/RuycastApp"),
        ),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
        .ok_or("Ruycast app is not installed or built. Run bash scripts/install.sh.".into())
}
fn config_edit(args: &[String]) -> Result<Value> {
    let r = root();
    private_dir(&r)?;
    let lock = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(r.join("config.lock"))
        .map_err(|e| e.to_string())?;
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
    match action {
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
        Ok(json!({"ok":true,"result":{"path":path,"reload":"on_next_start"}}))
    }
}
fn run_script(value: Value) -> Result<Value> {
    let action: Action = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if action.kind != "script" {
        return Err("The script runner requires a script command.".into());
    }
    let path = action
        .target
        .strip_prefix("~/")
        .map(|s| home().join(s))
        .unwrap_or_else(|| PathBuf::from(&action.target));
    let mut child = Command::new(path)
        .args(&action.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let group = child.id() as i32;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    unsafe {
        libc::fcntl(out.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        libc::fcntl(err.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
    }
    let timeout = action.timeout.unwrap_or(30.);
    let start = Instant::now();
    let mut ended: Option<Instant> = None;
    let mut expired = false;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    let drain = |reader: &mut dyn Read, result: &mut Vec<u8>| {
        let mut b = [0u8; 16384];
        for _ in 0..8 {
            match reader.read(&mut b) {
                Ok(0) => break,
                Ok(n) => {
                    result.extend_from_slice(&b[..n.min(65536usize.saturating_sub(result.len()))])
                }
                Err(_) => break,
            }
        }
    };
    loop {
        drain(&mut out, &mut stdout);
        drain(&mut err, &mut stderr);
        if status.is_none() {
            status = child.try_wait().map_err(|e| e.to_string())?;
        }
        if ended.is_none() && start.elapsed().as_secs_f64() >= timeout {
            expired = true;
            ended = Some(Instant::now());
            unsafe {
                libc::kill(-group, libc::SIGTERM);
            }
        }
        if ended.is_none() && status.is_some() {
            ended = Some(Instant::now());
            unsafe {
                libc::kill(-group, libc::SIGTERM);
            }
        }
        if let Some(end) = ended {
            if end.elapsed() >= Duration::from_millis(100) {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
            if end.elapsed() >= Duration::from_millis(150) && status.is_some() {
                break;
            }
            if end.elapsed() > Duration::from_secs(1) {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
                let _ = child.kill();
                status = Some(child.wait().map_err(|e| e.to_string())?);
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
    drain(&mut out, &mut stdout);
    drain(&mut err, &mut stderr);
    let output = String::from_utf8_lossy(&stdout);
    let errors = String::from_utf8_lossy(&stderr);
    if expired {
        Ok(json!({"ok":false,"error":format!("Script timed out after {timeout} seconds.")}))
    } else if !status.ok_or("Script status unavailable")?.success() {
        Ok(
            json!({"ok":false,"error":format!("Script exited with status {}.",status.unwrap().code().unwrap_or(-1)),"exitCode":status.unwrap().code().unwrap_or(-1),"stdout":output,"stderr":errors}),
        )
    } else {
        Ok(json!({"ok":true,"result":{"exitCode":0,"stdout":output,"stderr":errors}}))
    }
}
fn execute(args: &[String]) -> Result<Value> {
    let name = args.first().map(String::as_str).unwrap_or("--help");
    match name {
        "--help" | "help" | "-h" => Ok(
            json!({"ok":true,"commands":["status","performance [summary|reset --confirm]","search-ranking [summary|reset --confirm]","logs [--command ID] [--errors] [--limit N] | path | clear --confirm","analytics [summary|export|pause|resume|retention DAYS|reset --confirm]","monitor","processes [query]","commands","search [query] [--mode MODE]","run ID [--dry-run] [--input JSON_OR_TEXT] [--confirm]","snippets list [query]|add JSON|remove ID","quicklinks list [query]|add JSON|remove ID","window LAYOUT","show [commands|clipboard|snippets|quicklinks|files|menus|windows|processes|settings|analytics]","usage","reload","start [--headless]","quit","config path|get|init|validate [PATH]|reload", "config directory [ABSOLUTE_PATH]","config set KEY JSON [--expect-sha256 SHA]","config replace @FILE --expect-sha256 SHA","config add JSON_OR_@FILE [--expect-sha256 SHA]","config remove ID [--expect-sha256 SHA]","clipboard list [query]|copy ID|paste ID|delete ID|clear|pause|resume"],"profile":root(),"configFormat":"Readable JSON, version 1"}),
        ),
        "__run-script" => {
            let mut data = String::new();
            io::stdin()
                .take(2 * 1024 * 1024)
                .read_to_string(&mut data)
                .map_err(|e| e.to_string())?;
            run_script(parse(data.as_bytes())?)
        }
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
                    private_dir(&root())?;
                    if !path.exists() {
                        if root().join("config-location.json").exists()
                            && env::var("RUYCAST_CONFIG_DIR").map_or(true, |v| v.is_empty())
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
            private_dir(&root())?;
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(root().join("launcher.log"))
                .map_err(|e| e.to_string())?;
            let mut cmd = Command::new(binary()?);
            cmd.env("RUYCAST_HOME", root())
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
        "run" => rpc(
            json!({"op":"execute","id":argument(args,1,"command id")?,"confirmed":args.iter().any(|s|s=="--confirm"),"dryRun":args.iter().any(|s|s=="--dry-run"),"values":if let Some(i)=args.iter().position(|s|s=="--input") {let input=argument(args,i+1,"input")?;parse(input.as_bytes()).ok().filter(|v|v.is_object()).unwrap_or_else(||json!({"input":input}))} else {json!({})}}),
        ),
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
        "clipboard" => {
            let action = argument(args, 1, "clipboard action")?;
            match action {
                "list" => rpc(
                    json!({"op":"clipboard.list","query":args.get(2).map(String::as_str).unwrap_or("")}),
                ),
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
