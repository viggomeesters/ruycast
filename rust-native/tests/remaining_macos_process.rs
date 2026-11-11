//! Real control sockets and owned hidden AppKit surfaces. No external query,
//! native lock/kill/global key, personal index or user window is exercised.
#![cfg(feature = "test-tools")]
use serde_json::{json, Value};
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};
fn native() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn shim() -> &'static Path {
    static SHIM: OnceLock<PathBuf> = OnceLock::new();
    SHIM.get_or_init(|| {
        if let Some(path) = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM") {
            let path = PathBuf::from(path);
            assert!(path.is_absolute() && path.is_file());
            return path;
        }
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                native()
                    .parent()
                    .unwrap()
                    .join(".build/full-rust-release/native")
            });
        let path = target.join("release/examples/libworkspace_fixture.dylib");
        assert!(
            path.is_absolute() && path.is_file(),
            "Build --features test-tools --example workspace-fixture first"
        );
        path
    })
}
fn cli() -> &'static Path {
    static CLI: OnceLock<PathBuf> = OnceLock::new();
    CLI.get_or_init(|| {
        if let Some(path) = std::env::var_os("RUYCAST_TEST_CLI") {
            let path = PathBuf::from(path);
            assert!(path.is_absolute() && path.is_file());
            return path;
        }
        let target = native().join("target/remaining-cli");
        let output = Command::new("cargo")
            .args(["build", "--locked", "--bin", "ruycast", "--manifest-path"])
            .arg(native().parent().unwrap().join("rust/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        target.join("debug/ruycast")
    })
}
struct Host {
    child: Child,
    base: PathBuf,
    profile: PathBuf,
    socket: PathBuf,
}
impl Host {
    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(cli())
            .args(args)
            .env_clear()
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", &self.profile)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "CLI returned invalid JSON: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }
    fn new(gui: bool, theme: &str) -> Self {
        Self::with_config(gui, theme, |_| {})
    }
    fn with_config(gui: bool, theme: &str, configure: impl FnOnce(&mut Value)) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| native().join("target"));
        fs::create_dir_all(&parent).unwrap();
        let base = parent.join(format!(
            "mac-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let profile = base.join("p");
        fs::create_dir(&profile).unwrap();
        let applications = base.join("Applications");
        fs::create_dir(&applications).unwrap();
        fs::write(base.join("Fixture Document.txt"), b"Owned file").unwrap();
        fs::create_dir(base.join("Fixture Folder")).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        let mut config: Value = serde_json::from_slice(
            &fs::read(native().parent().unwrap().join("config/default.json")).unwrap(),
        )
        .unwrap();
        config["theme"] = json!(theme);
        configure(&mut config);
        fs::write(
            profile.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let binary = env!("CARGO_BIN_EXE_ruycast-native");
        let child = Command::new(binary)
            .arg(if gui {
                "--native-hidden"
            } else {
                "--headless-test"
            })
            .env_clear()
            .env("HOME", &base)
            .env("RUYCAST_HOME", &profile)
            .env("RUYCAST_APP_BINARY", binary)
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                serde_json::to_string(&vec![&applications]).unwrap(),
            )
            .env("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH", "owned")
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD", "named")
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &base)
            .env("DYLD_INSERT_LIBRARIES", shim())
            .stdout(Stdio::from(
                fs::File::create(base.join("host.stdout")).unwrap(),
            ))
            .stderr(Stdio::from(
                fs::File::create(base.join("host.stderr")).unwrap(),
            ))
            .spawn()
            .unwrap();
        let socket = profile.join("control.sock");
        let mut host = Self {
            child,
            base,
            profile,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if host.try_rpc(json!({"op":"status"})).is_ok() {
                return host;
            }
            let status = host.child.try_wait().unwrap();
            assert!(
                status.is_none(),
                "Owned host failed ({status:?}): {}\n{}",
                fs::read_to_string(host.base.join("host.stderr")).unwrap(),
                fs::read_to_string(host.base.join("host.stdout")).unwrap()
            );
            assert!(Instant::now() < deadline, "Owned host did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn try_rpc(&self, value: Value) -> Result<Value, String> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(&mut stream, &value).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    }
    fn rpc(&self, value: Value) -> Value {
        self.try_rpc(value).unwrap()
    }
    fn ok(&self, value: Value) -> Value {
        let result = self.rpc(value);
        assert_eq!(result["ok"], true, "{result}");
        result
    }
    fn probe(&self, name: &str) -> Value {
        self.ok(json!({"op":"native.test","action":name}))
    }
    fn wait_probe(&self, name: &str, accept: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let value = self.rpc(json!({"op":"native.test","action":name}));
            if value["ok"] == true && accept(&value["result"]) {
                return value["result"].clone();
            }
            assert!(
                Instant::now() < deadline,
                "Owned probe {name} not settled: {value}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn capture(&self, name: &str) -> Value {
        let result = self.probe("monitor-screenshot");
        let out = native().join("target/remaining-macos-evidence");
        fs::create_dir_all(&out).unwrap();
        fs::copy(
            self.profile.join("native-monitor-probe.png"),
            out.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        result
    }
    fn capture_palette(&self, name: &str) {
        let read = self.probe("screenshot");
        let out = native().join("target/remaining-macos-evidence");
        fs::create_dir_all(&out).unwrap();
        fs::copy(
            self.profile.join("native-probe.png"),
            out.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&read).unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.try_rpc(json!({"op":"quit"}));
        let until = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().ok().flatten().is_none() {
            if Instant::now() > until {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.wait();
        let out = native().join("target/remaining-macos-evidence");
        let _ = fs::create_dir_all(&out);
        let _ = fs::copy(
            self.base.join("host.stderr"),
            out.join(format!(
                "{}.stderr",
                self.base.file_name().unwrap().to_string_lossy()
            )),
        );
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn hidden_startup_cli_routes_are_owned_before_the_first_show() {
    let host = Host::with_config(true, "light", |config| {
        config["commands"].as_array_mut().unwrap().push(
            json!({"id":"owned.lock","title":"Owned Lock","kind":"builtin","target":"lock-screen"}),
        );
    });
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    let before = host.ok(json!({"op":"system.fixture","action":"bridge-audit"}));
    assert_eq!(before["result"]["required"], true);
    assert_eq!(before["result"]["enabled"], true);
    assert_eq!(before["result"]["requests"], json!({}));
    let placed = host.cli(&["window", "left-half"]);
    assert_eq!(placed["ok"], true, "{placed}");
    assert_eq!(placed["result"]["width"], 600.);
    assert_eq!(placed["result"]["height"], 800.);
    let restored = host.cli(&["window", "restore"]);
    assert_eq!(restored["ok"], true);
    assert_eq!(restored["result"]["x"], 40.);
    assert_eq!(host.cli(&["run", "builtin.open-config"])["ok"], true);
    assert_eq!(host.cli(&["run", "owned.lock"])["ok"], true);
    let probe = host.ok(json!({"op":"system.fixture","action":"bridge-startup-probe"}));
    assert_eq!(probe["result"]["foregroundAbsent"], true);
    assert_eq!(probe["result"]["screens"][0]["width"], 1200.);
    let audit = &probe["result"]["audit"];
    assert_eq!(audit["blocked"], 0);
    assert_eq!(audit["requests"]["screens"], 3);
    assert_eq!(audit["requests"]["foreground"], 1);
    for key in ["configuration", "lock", "activity-monitor"] {
        assert_eq!(audit["requests"][key], 1);
    }
    let read = host.probe("read");
    assert_eq!(read["result"]["windowShown"], false);
    assert_eq!(read["result"]["windowKey"], false);
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    let out = native().join("target/repair-evidence");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("pre-show-startup.json"),serde_json::to_vec_pretty(&json!({"before":before,"placed":placed,"restored":restored,"probe":probe,"read":read,"realDesktopEffects":false})).unwrap()).unwrap();
}
#[test]
fn real_cli_window_files_processes_and_monitor_data_use_owned_backends() {
    let host = Host::new(false, "light");
    let placed = host.ok(json!({"op":"execute","id":"window.left-half"}));
    assert_eq!(placed["result"]["width"], 600.);
    let restored = host.ok(json!({"op":"execute","id":"window.restore"}));
    assert_eq!(restored["result"]["x"], 40.);
    assert_eq!(restored["result"]["width"], 600.);
    let files = host.ok(json!({"op":"search","mode":"files","query":"Fixture"}));
    assert_eq!(files["result"].as_array().unwrap().len(), 2);
    assert_eq!(files["result"][0]["kind"], "path");
    let dry = host.ok(json!({"op":"execute","id":files["result"][0]["id"],"dryRun":true}));
    assert_eq!(
        dry["result"]["target"],
        host.base.join("Fixture Document.txt").to_str().unwrap()
    );
    let processes = host.ok(json!({"op":"processes"}));
    let process = &processes["result"][0];
    assert_eq!(process["metrics"]["identity"]["pid"], 730011);
    assert_eq!(process["metrics"]["cpuPercent"], 20.);
    host.ok(json!({"op":"execute","id":process["command"]["id"]}));
    let processes = host.ok(json!({"op":"processes"}));
    assert_eq!(processes["result"].as_array().unwrap().len(), 1);
    let sample = host.ok(json!({"op":"monitor"}));
    assert_eq!(sample["result"]["hostname"], "owned.invalid");
    assert_eq!(sample["result"]["batteryPercent"], 75);
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"builtin.system-monitor"}))["ok"],
        false
    );
    let logs = host.ok(json!({"op":"logs"}));
    let text = logs.to_string();
    assert!(!text.contains("Owned Process"));
    assert!(!text.contains("system.kill:"));
}
#[test]
fn real_read_only_resource_sample_satisfies_original_bounds_without_serializing_metadata() {
    // Read the original native metric route once. Never serialize, print or
    // persist hostname, hardware identifiers, interface names or sample values.
    let snapshot = ruycast_native::resource_sampler::Sampler::default()
        .sample(&ruycast_native::resource_sampler::Native);
    assert!(snapshot.memory_total > 0);
    assert!(snapshot.disk_total > 0);
    assert!(snapshot.disk_available <= snapshot.disk_total);
    assert!(snapshot.process_count > 0);
}
#[test]
fn actual_cli_monitor_and_process_query_use_the_owned_running_host() {
    let host = Host::new(false, "light");
    let binary = PathBuf::from(
        std::env::var_os("RUYCAST_TEST_CLI").expect("explicit current owned CLI required"),
    );
    assert!(binary.is_absolute());
    assert!(fs::symlink_metadata(&binary).unwrap().is_file());
    for args in [vec!["monitor"], vec!["processes", "Owned Process"]] {
        let child = Command::new(&binary)
            .args(&args)
            .env_clear()
            .env("HOME", &host.base)
            .env("RUYCAST_HOME", &host.profile)
            .env("RUYCAST_APP_BINARY", env!("CARGO_BIN_EXE_ruycast-native"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child_output::wait(child, Duration::from_secs(5));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        if args[0] == "monitor" {
            assert_eq!(reply["result"]["hostname"], "owned.invalid");
            assert_eq!(reply["result"]["batteryPercent"], 75);
        } else {
            let rows = reply["result"].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row["metrics"]["identity"]["pid"]
                .as_i64()
                .is_some_and(|pid| [730011, 730012].contains(&pid))));
        }
    }
    assert_eq!(
        host.ok(json!({"op":"system.fixture","action":"read"}))["result"]["effects"],
        json!([])
    );
}
#[test]
fn vanished_native_process_selection_cannot_kill_the_remaining_owned_row() {
    let host = Host::new(true, "light");
    host.ok(json!({"op":"show","mode":"processes"}));
    host.wait_probe("read", |v| v["rows"] == 2 && v["selectedRow"] == 0);
    let rows = host.ok(json!({"op":"processes"}));
    // A separate owned actor removes the selected process; the next Return must
    // not fall through to the remaining row at the same visible index.
    host.ok(json!({"op":"execute","id":rows["result"][0]["command"]["id"]}));
    let read = host.wait_probe("read", |v| v["rows"] == 1);
    assert_eq!(read["selectedRow"], -1);
    let before = host.ok(json!({"op":"system.fixture","action":"read"}));
    assert_eq!(before["result"]["effects"], json!(["terminate-owned"]));
    host.ok(json!({"op":"native.test","action":"key","key":"return"}));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        host.ok(json!({"op":"system.fixture","action":"read"}))["result"]["effects"],
        before["result"]["effects"]
    );
    assert_eq!(
        host.ok(json!({"op":"processes"}))["result"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let read = host.probe("read");
    assert_eq!(read["result"]["windowShown"], false);
    assert_eq!(read["result"]["windowKey"], false);
}
#[test]
fn native_monitor_six_tabs_light_dark_minimum_wide_and_no_ordering() {
    for theme in ["light", "dark"] {
        let host = Host::new(true, theme);
        host.ok(json!({"op":"execute","id":"builtin.system-monitor"}));
        host.wait_probe("monitor-read", |v| {
            v["history"].as_u64().is_some_and(|n| n >= 3)
        });
        for (width, height) in [(740., 500.), (1100., 760.)] {
            host.ok(
                json!({"op":"native.test","action":"monitor-size","width":width,"height":height}),
            );
            for (index, tab) in [
                "System Info",
                "CPU",
                "Memory",
                "Macintosh HD",
                "Power",
                "Network",
            ]
            .into_iter()
            .enumerate()
            {
                host.ok(json!({"op":"native.test","action":"monitor-control","index":index}));
                let read = host.wait_probe("monitor-read", |v| v["tab"] == tab);
                assert_eq!(read["windowShown"], false);
                assert_eq!(read["windowKey"], false);
                assert_eq!(read["orderingAllowed"], false);
                let capture = host.capture(&format!("monitor-{theme}-{}-{index}", width as u32));
                assert!(capture["result"]["screenText"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == tab));
            }
        }
        host.ok(json!({"op":"native.test","action":"monitor-size","width":740.,"height":500.}));
        for state in ["loading", "error", "long", "no-battery"] {
            let read =
                host.ok(json!({"op":"native.test","action":"monitor-render-state","state":state}));
            let out = native().join("target/remaining-macos-evidence");
            let name = format!("monitor-{theme}-{state}");
            fs::copy(
                host.profile.join("native-monitor-probe.png"),
                out.join(format!("{name}.png")),
            )
            .unwrap();
            fs::write(
                out.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&read).unwrap(),
            )
            .unwrap();
            assert_eq!(read["result"]["windowShown"], false);
        }
    }
}
#[test]
fn native_process_sort_refresh_keeps_identity_and_kill_keeps_palette_open() {
    let host = Host::new(true, "light");
    let initial = host.ok(json!({"op":"processes"}));
    let identity = initial["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["metrics"]["identity"]["pid"] == 730011)
        .unwrap()["metrics"]["identity"]
        .clone();
    host.ok(
        json!({"op":"system.fixture","action":"process-cpu","identity":identity,"cpuPercent":49.5}),
    );
    // Complete fixture setup through the real provider's ordinary cache TTL
    // before opening the palette whose selected process is under test.
    let setup_until=Instant::now()+Duration::from_secs(5);
    loop {
        let staged=host.ok(json!({"op":"processes"}));
        if staged["result"].as_array().unwrap().iter().any(|row|row["metrics"]["identity"]==identity && row["metrics"]["cpuPercent"]==49.5) {break;}
        assert!(Instant::now()<setup_until,"Owned CPU sample setup did not reach provider");
        std::thread::sleep(Duration::from_millis(10));
    }
    host.ok(json!({"op":"show","mode":"processes"}));
    let before = host.wait_probe("read", |v| {
        v["rows"] == 2
            && v["screenText"].as_array().is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str().is_some_and(|s| s.contains("49.5% CPU")))
            })
    });
    assert_eq!(before["selectedRow"], 0);
    let selected = host.ok(json!({"op":"status"}))["result"]["nativeUI"]["selectedId"].clone();
    assert!(selected.is_string());
    let samples_before = host.ok(json!({"op":"processes"}))["result"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        samples_before
            .iter()
            .find(|row| row["metrics"]["identity"] == identity)
            .unwrap()["metrics"]["cpuPercent"],
        49.5
    );
    host.capture_palette("processes-light-cpu");
    host.ok(json!({"op":"native.test","action":"process-sort","index":1}));
    host.wait_probe("read", |v| v["selectedRow"] == 1);
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
        selected
    );
    host.capture_palette("processes-light-memory");
    // Mutate only the existing owned transport sample. Let the normal cache TTL
    // and two-second owner refresh publish it; never inject a UI/result snapshot.
    host.ok(
        json!({"op":"system.fixture","action":"process-cpu","identity":identity,"cpuPercent":0.1}),
    );
    let refreshed = host.wait_probe("read", |v| {
        v["screenText"].as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str().is_some_and(|s| s.contains("0.1% CPU")))
        })
    });
    assert_eq!(refreshed["selectedRow"], 1);
    assert_eq!(refreshed["windowShown"], false);
    assert_eq!(refreshed["windowKey"], false);
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
        selected
    );
    let samples_after = host.ok(json!({"op":"processes"}))["result"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(samples_after.len(), samples_before.len());
    for before in samples_before {
        let current = samples_after
            .iter()
            .find(|row| row["metrics"]["identity"] == before["metrics"]["identity"])
            .unwrap();
        let mut expected = before["metrics"].clone();
        if expected["identity"] == identity {
            expected["cpuPercent"] = json!(0.1);
        }
        assert_eq!(current["metrics"], expected);
    }
    assert_eq!(
        host.ok(json!({"op":"system.fixture","action":"read"}))["result"]["effects"],
        json!([])
    );
    host.capture_palette("processes-light-memory-refreshed");
    host.ok(json!({"op":"native.test","action":"key","key":"return"}));
    let after = host.wait_probe("read", |v| v["rows"] == 1);
    assert_eq!(after["windowShown"], false);
    let status = host.ok(json!({"op":"status"}));
    assert_eq!(status["result"]["nativeUI"]["visible"], true);
}
#[test]
fn native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes() {
    for theme in ["light", "dark"] {
        let host = Host::new(true, theme);
        host.ok(json!({"op":"show","mode":"processes"}));
        host.wait_probe("read", |v| v["rows"] == 2);
        host.ok(
            json!({"op":"native.test","action":"key-event","code":40,"flags":1u64<<20,"chars":"k"}),
        );
        let menu = host.wait_probe("read", |v| v["actionsVisible"] == true);
        let screen = menu["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        for value in [
            "Owned Process 2".to_owned(),
            "730011".into(),
            "20.0% CPU".into(),
            ruycast_native::resource_sampler::memory_label(4096),
        ] {
            assert!(
                screen.contains(&value),
                "Owned process metric not rendered: {value}; own screen: {screen}"
            );
        }
        let labels = menu["actionLabels"].as_array().unwrap();
        assert!(labels
            .first()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("Kill"));
        for label in labels {
            let label = label.as_str().unwrap();
            assert!(
                !["Set Alias", "Set Hotkey", "Disable"]
                    .iter()
                    .any(|prefix| label.starts_with(prefix)),
                "{label}"
            );
        }
        host.capture_palette(&format!("process-actions-{theme}"));
        host.ok(json!({"op":"native.test","action":"key-event","code":53,"flags":0,"chars":""}));
        host.wait_probe("read", |v| v["actionsVisible"] == false);
        host.ok(json!({"op":"native.test","action":"key","key":"return"}));
        let after = host.wait_probe("read", |v| v["rows"] == 1);
        assert_eq!(
            host.ok(json!({"op":"system.fixture","action":"read"}))["result"]["effects"],
            json!(["terminate-owned"])
        );
        assert_eq!(after["formVisible"], false);
        assert_eq!(after["windowShown"], false);
        assert_eq!(after["windowKey"], false);
    }
}
#[test]
fn native_clipboard_two_color_image_file_pixels_and_idle_text_are_preserved_in_both_themes() {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace};
    use objc2_foundation::{NSData, NSDictionary, NSString, NSURL};
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), 240, 140, 8, 4, true, false,
            NSDeviceRGBColorSpace, 960, 32,
        )
    }.unwrap();
    let pointer = bitmap.bitmapData();
    assert!(!pointer.is_null());
    let pixels = unsafe { std::slice::from_raw_parts_mut(pointer, 140 * 960) };
    for y in 0..140 {
        for x in 0..240 {
            pixels[y * 960 + x * 4..y * 960 + x * 4 + 4].copy_from_slice(if x < 120 {
                &[255, 128, 0, 255]
            } else {
                &[0, 102, 255, 255]
            });
        }
    }
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .unwrap()
    .to_vec();
    for theme in ["light", "dark"] {
        let host = Host::new(true, theme);
        let wait_count = |count| {
            let end = Instant::now() + Duration::from_secs(5);
            while host.ok(json!({"op":"clipboard.list"}))["result"]
                .as_array()
                .unwrap()
                .len()
                != count
            {
                assert!(
                    Instant::now() < end,
                    "Owned clipboard capture did not settle"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let text = "Clipboard preview keeps the full text.\nA second line with Ω and /owned/notes.";
        let reply = host
            .ok(json!({"op":"native.test","action":"clipboard-fixture","payload":{"Text":text}}));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["namedBoard"], true, "{reply}");
        wait_count(1);
        host.ok(json!({"op":"show","mode":"clipboard"}));
        let read = host.wait_probe("read", |v| {
            v["screenText"].as_array().is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str().is_some_and(|value| value == text))
            })
        });
        assert!(read["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("Characters"))));
        // Match the original selected-text fixture's 0.2-second layout settle.
        std::thread::sleep(Duration::from_millis(200));
        let first = host.probe("screenshot");
        assert_eq!(first["ok"], true, "{first}");
        assert_eq!(first["result"]["windowShown"], false, "{first}");
        assert_eq!(first["result"]["windowKey"], false, "{first}");
        let before = fs::read(host.profile.join("native-probe.png")).unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let second = host.probe("screenshot");
        assert_eq!(second["ok"], true, "{second}");
        assert_eq!(second["result"]["windowShown"], false, "{second}");
        assert_eq!(second["result"]["windowKey"], false, "{second}");
        let after = fs::read(host.profile.join("native-probe.png")).unwrap();
        let out = native().join("target/remaining-macos-evidence");
        fs::create_dir_all(&out).unwrap();
        for (name, bytes, read) in [("before", &before, &first), ("after", &after, &second)] {
            fs::write(out.join(format!("rich-{theme}-idle-{name}.png")), bytes).unwrap();
            fs::write(
                out.join(format!("rich-{theme}-idle-{name}.json")),
                serde_json::to_vec_pretty(read).unwrap(),
            )
            .unwrap();
        }
        assert!(
            before == after,
            "Idle clipboard text render changed: {theme}"
        );
        host.ok(json!({"op":"native.test","action":"clipboard-fixture","payload":{"Image":png}}));
        wait_count(2);
        let file = host.base.join("owned-preview.png");
        fs::write(&file, &png).unwrap();
        let url = NSURL::fileURLWithPath(&NSString::from_str(file.to_str().unwrap()))
            .absoluteString()
            .unwrap()
            .to_string();
        for (query, name) in [("Image", "image"), ("owned-preview.png", "image-file")] {
            if name == "image-file" {
                host.ok(json!({"op":"native.test","action":"clipboard-fixture","payload":{"Files":[url]}}));
                wait_count(3);
            }
            host.ok(json!({"op":"show","mode":"clipboard","query":query}));
            host.wait_probe("read", |v| {
                v["screenText"].as_array().is_some_and(|values| {
                    values.iter().any(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| value.contains("240 × 140"))
                    })
                })
            });
            let read = host.probe("screenshot");
            assert_eq!(read["ok"], true, "{read}");
            assert_eq!(read["result"]["windowShown"], false, "{read}");
            assert_eq!(read["result"]["windowKey"], false, "{read}");
            let bytes = fs::read(host.profile.join("native-probe.png")).unwrap();
            let normalized = ruycast_native::clipboard_image::preview(&bytes).unwrap();
            let bitmap =
                NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&normalized.png)).unwrap();
            let mut colored = 0;
            for y in (0..bitmap.pixelsHigh()).step_by(4) {
                for x in (bitmap.pixelsWide() * 2 / 5..bitmap.pixelsWide()).step_by(4) {
                    let color = bitmap.colorAtX_y(x, y).unwrap();
                    if color.alphaComponent() > 0.9
                        && ((color.redComponent() > 0.8 && color.blueComponent() < 0.2)
                            || (color.blueComponent() > 0.8 && color.redComponent() < 0.2))
                    {
                        colored += 1;
                    }
                }
            }
            assert!(
                colored > 300,
                "Rendered owned {name} pixels absent in {theme}: {colored}"
            );
            let out = native().join("target/remaining-macos-evidence");
            fs::create_dir_all(&out).unwrap();
            fs::write(out.join(format!("rich-{theme}-{name}.png")), bytes).unwrap();
            fs::write(
                out.join(format!("rich-{theme}-{name}.json")),
                serde_json::to_vec_pretty(&read).unwrap(),
            )
            .unwrap();
        }
    }
}
#[test]
fn native_monitor_delegate_pause_resume_close_and_reopen_use_actual_callback_route() {
    let host = Host::new(true, "dark");
    assert_eq!(
        host.rpc(json!({"op":"native.test","action":"monitor-read"}))["ok"],
        false
    );
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    host.ok(json!({"op":"execute","id":"builtin.system-monitor"}));
    let initial = host.wait_probe("monitor-read", |v| {
        v["history"].as_u64().is_some_and(|n| n >= 1)
    });
    assert_eq!(
        host.ok(json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    let token = initial["token"].as_u64().unwrap();
    host.ok(json!({"op":"native.test","action":"monitor-control","index":5}));
    host.wait_probe("monitor-read", |v| v["tab"] == "Network");
    host.ok(json!({"op":"native.test","action":"monitor-lifecycle","event":"pause"}));
    let paused = host.wait_probe("monitor-read", |v| v["active"] == false);
    let count = paused["history"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(host.probe("monitor-read")["result"]["history"], count);
    host.ok(json!({"op":"native.test","action":"monitor-lifecycle","event":"resume"}));
    host.wait_probe("monitor-read", |v| {
        v["active"] == true && v["history"].as_u64().is_some_and(|n| n > count)
    });
    host.ok(json!({"op":"native.test","action":"monitor-lifecycle","event":"close"}));
    let closed = host.wait_probe("monitor-read", |v| {
        v["shown"] == false && v["active"] == false
    });
    std::thread::sleep(Duration::from_millis(2100));
    let still_closed = host.probe("monitor-read");
    assert_eq!(still_closed["result"]["history"], closed["history"]);
    assert_eq!(still_closed["result"]["active"], false);
    host.ok(json!({"op":"execute","id":"builtin.system-monitor"}));
    let reopened = host.wait_probe("monitor-read", |v| {
        v["shown"] == true && v["token"].as_u64().is_some_and(|n| n > token)
    });
    assert_eq!(reopened["tab"], "Network");
    assert_eq!(reopened["windowShown"], false);
    assert_eq!(reopened["windowKey"], false);
}
#[test]
fn native_file_open_uses_owned_workspace_and_cli_menu_run_requires_original_identity() {
    let host = Host::new(true, "light");
    host.ok(json!({"op":"show","mode":"files","query":"Fixture Document"}));
    host.wait_probe("read", |v| v["rows"] == 1);
    host.capture_palette("files-light");
    host.ok(json!({"op":"native.test","action":"key","key":"return"}));
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let trace = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap_or_default();
        if trace.contains("Fixture%20Document.txt") {
            break;
        }
        assert!(
            Instant::now() < until,
            "Owned file open was not dispatched: {trace}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let menu = host.cli(&["search", "Owned", "--mode", "menus"]);
    assert_eq!(menu["ok"], true, "{menu}");
    let id = menu["result"][0]["id"].as_str().unwrap();
    let identity = menu["foreground"].to_string();
    let missing = host.cli(&["run", id]);
    assert_eq!(missing["ok"], false);
    let malformed = host.cli(&["run", id, "--foreground", "{\"pid\":730001}"]);
    assert_eq!(malformed["ok"], false);
    let run = host.cli(&["run", id, "--foreground", &identity]);
    assert_eq!(run["ok"], true, "{run}");
    let logs = host.ok(json!({"op":"logs"})).to_string();
    assert!(!logs.contains("Fixture Document"));
    assert!(!logs.contains("system.file."));
    assert!(!logs.contains("system.menu:"));
}
#[test]
fn direct_native_sdk_geometry_and_workspace_effects_have_complete_owned_substitutes() {
    let host = Host::new(false, "light");
    let system = native().join("target/remaining-system-sdk.dylib");
    let output = Command::new("cc")
        .args([
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fobjc-arc",
            "-fblocks",
            "-dynamiclib",
            "-framework",
            "AppKit",
            "-framework",
            "ApplicationServices",
            "-framework",
            "Carbon",
        ])
        .arg(native().join("tests/system_search_sdk_shim.m"))
        .arg("-o")
        .arg(&system)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe = env!("CARGO_BIN_EXE_macos-effects-probe");
    for (mode, libraries) in [
        ("workspace", shim().display().to_string()),
        (
            "geometry",
            format!("{}:{}", shim().display(), system.display()),
        ),
    ] {
        let output = Command::new(probe)
            .arg(mode)
            .env_clear()
            .env("HOME", &host.base)
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &host.base)
            .env("RUYCAST_SYSTEM_SDK_FIXTURE_HOME", &host.base)
            .env("DYLD_INSERT_LIBRARIES", libraries)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["realDesktopEffects"], false);
        let out = native().join("target/remaining-macos-evidence");
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join(format!("native-sdk-{mode}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }
    let output = Command::new(probe)
        .arg("geometry")
        .env_clear()
        .env("HOME", &host.base)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["ok"],
        false
    );
}
#[test]
fn builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly() {
    let host = Host::new(false, "light");
    let reloaded = host.ok(json!({"op":"execute","id":"builtin.reload-config"}));
    assert_eq!(reloaded["result"]["state"], "reloaded");
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"builtin.settings"}))["ok"],
        false
    );
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"builtin.open-config"}))["ok"],
        false
    );
    let usage = host.ok(json!({"op":"usage"}));
    assert_eq!(usage["result"]["builtin.reload-config"], 1);
}
#[test]
fn custom_window_inputs_unicode_alias_and_disabled_override_use_original_command_authority() {
    let host = Host::with_config(false, "light", |config| {
        let commands = config["commands"].as_array_mut().unwrap();
        commands
            .iter_mut()
            .find(|c| c["id"] == "window.left-half")
            .unwrap()["enabled"] = json!(false);
        commands.push(json!({"id":"owned.geometry","title":"Owned Geometry","kind":"window","target":"right-half","aliases":["géométrie"],"inputPrompt":"Workspace"}));
    });
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"WINDOW.LEFT-HALF"}))["ok"],
        false
    );
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"GÉOMÉTRIE"}))["ok"],
        false
    );
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"GÉOMÉTRIE","values":{"input":"  "}}))["ok"],
        false
    );
    let placed = host.ok(json!({"op":"execute","id":"GÉOMÉTRIE","values":{"input":"owned"}}));
    assert_eq!(placed["result"]["x"], 600.);
    let restored = host.ok(json!({"op":"execute","id":"window.restore"}));
    assert_eq!(restored["result"]["x"], 40.);
}

#[test]
fn native_prompted_window_and_builtin_submit_reach_owned_effect_once() {
    // Both actual inline Return and expanded Cmd-Return must get past the
    // script-only prepare gate. Host installs the complete owned services and
    // workspace shim before startup; none of these are desktop key events.
    for expanded in [false, true] {
        for kind in ["window", "builtin"] {
            let host = Host::with_config(true, "light", |config| {
                config["commands"].as_array_mut().unwrap().push(json!({
                    "id":"owned.prompt-dispatch", "title":"Owned Prompt Dispatch",
                    "aliases":["qzpromptdispatch"], "kind":kind,
                    "target":if kind == "window" {"right-half"} else {"reload-config"},
                    "inputPrompt":"Owned workspace"
                }));
            });
            let probe = |action: &str, extra: Value| {
                let mut request = extra;
                request["op"] = json!("native.test");
                request["action"] = json!(action);
                host.ok(request)
            };
            let key = |flags| {
                probe("key-event", json!({"code":36,"chars":"\r","flags":flags}));
            };
            let audit = host.ok(json!({"op":"system.fixture","action":"bridge-audit"}));
            assert_eq!(audit["result"]["required"], true);
            assert_eq!(audit["result"]["enabled"], true);
            host.ok(json!({"op":"show","mode":"commands","query":"qzpromptdispatch"}));
            host.wait_probe("read", |v| {
                v["rows"] == 1 && v["modelQuery"] == "qzpromptdispatch"
            });
            key(0);
            let opened = host.wait_probe("read", |v| v["editorId"] == "owned.prompt-dispatch");
            let token = opened["editorToken"].as_u64().expect("Actual editor token");
            assert_eq!(opened["runInputInline"], true);
            if expanded {
                key(1 << 20);
                host.wait_probe("read", |v| {
                    v["editorToken"] == token && v["runInputInline"] == false
                });
            }
            let typing = if expanded {
                "run-input-type"
            } else {
                "run-inline-type"
            };
            let reading = if expanded {
                "run-input-read"
            } else {
                "run-inline-read"
            };
            let submit_flags = if expanded { 1 << 20 } else { 0 };
            let before = host.ok(json!({"op":"system.fixture"}));
            let before_usage = host.ok(json!({"op":"usage"}));
            probe(
                typing,
                json!({"text":"\u{0085}\u{00a0}\u{2003}\u{2028}\u{3000}"}),
            );
            key(submit_flags);
            let rejected = host.wait_probe("read", |v| {
                v["editorToken"] == token && v.to_string().contains("Enter Owned workspace.")
            });
            assert!(!rejected
                .to_string()
                .contains("unavailable in the development build"));
            assert_eq!(
                host.ok(json!({"op":"system.fixture"}))["result"]["effects"],
                before["result"]["effects"]
            );
            assert_eq!(host.ok(json!({"op":"usage"})), before_usage);
            let literal = "  Owned Ω {input} $(literal) ";
            probe(typing, json!({"text":literal}));
            let entered = probe(reading, json!({}));
            assert!(
                entered["result"]["text"]
                    .as_str()
                    .unwrap()
                    .ends_with(literal),
                "{entered}"
            );
            key(submit_flags);
            let closed = host.wait_probe("read", |v| v["editorToken"].is_null());
            assert!(
                !closed
                    .to_string()
                    .contains("unavailable in the development build"),
                "{closed}"
            );
            assert_eq!(closed["windowShown"], false);
            assert_eq!(closed["windowKey"], false);
            let usage = host.ok(json!({"op":"usage"}));
            assert_eq!(usage["result"]["owned.prompt-dispatch"], 1, "{usage}");
            let effects = host.ok(json!({"op":"system.fixture"}));
            assert_eq!(
                effects["result"]["effects"],
                if kind == "window" {
                    json!(["resize", "position", "resize"])
                } else {
                    json!([])
                }
            );
            let after_audit = host.ok(json!({"op":"system.fixture","action":"bridge-audit"}));
            assert_eq!(after_audit["result"]["blocked"], 0);

            // Reopen, submit from the existing local timer, then revoke the
            // editor while the serialized owner is held. The queued submission
            // must not run a second effect or create a second usage record.
            host.ok(json!({"op":"show","mode":"commands","query":"qzpromptdispatch"}));
            host.wait_probe("read", |v| v["rows"] == 1 && v["editorToken"].is_null());
            key(0);
            let reopened = host.wait_probe("read", |v| v["editorId"] == "owned.prompt-dispatch");
            assert_ne!(reopened["editorToken"], token);
            probe("run-inline-type", json!({"text":"Owned cancelled input"}));
            probe("schedule-input-submit", json!({}));
            let socket = host.socket.clone();
            let held = std::thread::spawn(move || {
                let mut stream = UnixStream::connect(socket).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                serde_json::to_writer(
                    &mut stream,
                    &json!({"op":"native.delay","executionBarrier":true,"holdOwner":true}),
                )
                .unwrap();
                stream.write_all(b"\n").unwrap();
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).unwrap();
                serde_json::from_str::<Value>(&line).unwrap()
            });
            let wait_file = |name: &str| {
                let until = Instant::now() + Duration::from_secs(3);
                while !host.profile.join(name).exists() {
                    if Instant::now() >= until {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                true
            };
            let started = wait_file("owner-delay-started");
            let cancelled = wait_file("input-submit-fired");
            let released = fs::write(
                host.profile.join("owner-delay-release"),
                b"release after editor cancellation",
            );
            let response = held.join().expect("Owned barrier thread reaped");
            released.unwrap();
            assert!(
                started && cancelled,
                "Owned submit/cancellation barrier did not complete"
            );
            assert_eq!(response["ok"], true);
            host.wait_probe("read", |v| v["editorToken"].is_null());
            assert_eq!(host.ok(json!({"op":"usage"})), usage);
            assert_eq!(
                host.ok(json!({"op":"system.fixture"}))["result"]["effects"],
                effects["result"]["effects"]
            );
        }
    }
}
#[test]
fn configured_default_navigation_builtins_reach_the_native_interface() {
    let host = Host::new(true, "light");
    let mut replies = vec![];
    for id in [
        "clipboard",
        "builtin.snippets",
        "builtin.quicklinks",
        "builtin.create-snippet",
        "builtin.create-quicklink",
    ] {
        replies.push((id, host.rpc(json!({"op":"execute","id":id}))));
    }
    assert!(
        replies.iter().all(|(_, reply)| reply["ok"] == true),
        "Existing default builtins failed: {replies:?}"
    );
    for mode in ["snippets", "quicklinks"] {
        host.ok(json!({"op":"show","mode":mode}));
    }
}

#[test]
fn scoped_launcher_native_results_execute_and_reset_without_settings_navigation() {
    let host = Host::with_config(true, "light", |config| {
        let commands = config["commands"].as_array_mut().unwrap();
        for i in 0..130 {
            commands.push(json!({"id":format!("owned.noise.{i}"),"title":format!("qzscopeowned Noise {i}"),"kind":"script","target":"/usr/bin/true"}));
        }
        commands.push(json!({"id":"owned.scope-snippet","title":"qzscopeowned snippet","kind":"snippet","target":"Owned text"}));
        commands.push(json!({"id":"owned.scope-link","title":"qzscopeowned link","kind":"url","target":"ruycast-fixture://success/owned-scope"}));
        commands.push(json!({"id":"owned.scope-disabled","title":"qzscopeowned disabled","kind":"url","target":"https://example.com/disabled","enabled":false}));
    });
    for (mode, id) in [
        ("snippets", "owned.scope-snippet"),
        ("quicklinks", "owned.scope-link"),
    ] {
        host.ok(json!({"op":"show","mode":mode,"query":"qzscopeowned"}));
        let view = host.wait_probe("read", |v| {
            v["rows"] == 1 && v["modelQuery"] == "qzscopeowned"
        });
        assert!(view["editorToken"].is_null());
        let status = host.ok(json!({"op":"status"}));
        assert_eq!(status["result"]["nativeUI"]["selectedId"], id);
        assert!(status["result"]["nativeUI"]["tab"].is_null());
        assert_eq!(status["result"]["nativeUI"]["systemSearchBusy"], false);
        host.capture_palette(&format!("navigation-{mode}"));
        let copy = ["screenText", "accessibilityText"]
            .iter()
            .flat_map(|key| {
                view[*key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|text| text.as_str().unwrap())
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            native()
                .join("target/remaining-macos-evidence")
                .join(format!("navigation-{mode}-copy.txt")),
            copy,
        )
        .unwrap();
        assert!(view["accessibilityText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text
                .as_str()
                .is_some_and(|text| text.contains(if mode == "snippets" {
                    "Search snippets"
                } else {
                    "Search quicklinks"
                }))));
        host.ok(json!({"op":"native.test","action":"launcher-back"}));
        host.wait_probe("read", |v| {
            v["modelQuery"] == "" && !v["screenText"].as_array().unwrap().contains(&json!("Back"))
        });
    }
    host.ok(json!({"op":"show","mode":"quicklinks","query":"NoMatchingOwnedScope"}));
    host.wait_probe("read", |v| v["rows"] == 0);
    host.ok(json!({"op":"native.test","action":"launcher-back"}));
    host.wait_probe("read", |v| {
        v["modelQuery"] == "" && v["rows"].as_u64().is_some_and(|n| n > 0)
    });
    host.ok(json!({"op":"show","mode":"quicklinks","query":"qzscopeowned"}));
    host.wait_probe("read", |v| v["rows"] == 1);
    host.ok(json!({"op":"native.test","action":"key","key":"return"}));
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let trace = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap_or_default();
        if trace
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|event| {
                event["phase"] == "open-default"
                    && event["url"] == "ruycast-fixture://success/owned-scope"
            })
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "Scoped quicklink Return did not execute: {trace}; read={}; status={}; logs={}",
            host.probe("read"),
            host.ok(json!({"op":"status"})),
            host.ok(json!({"op":"logs"}))
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    host.ok(json!({"op":"show","mode":"commands","query":"qzscopeowned"}));
    host.wait_probe("read", |v| v["rows"] == 100);
    let read = host.probe("read");
    assert_eq!(read["result"]["windowShown"], false);
    assert_eq!(read["result"]["windowKey"], false);
}

#[test]
fn navigation_builtins_dry_run_and_headless_admission_have_no_effect() {
    let host = Host::new(false, "light");
    let usage = host.ok(json!({"op":"usage"}));
    for id in [
        "clipboard",
        "builtin.snippets",
        "builtin.quicklinks",
        "builtin.create-snippet",
        "builtin.create-quicklink",
    ] {
        assert_eq!(
            host.ok(json!({"op":"execute","id":id,"dryRun":true}))["dryRun"],
            true
        );
        let rejected = host.rpc(json!({"op":"execute","id":id}));
        assert_eq!(rejected["ok"], false);
        assert!(
            rejected["error"].as_str().unwrap().contains("headless"),
            "{rejected}"
        );
    }
    assert_eq!(host.ok(json!({"op":"usage"})), usage);
}

#[test]
fn navigation_builtin_overrides_alias_disabled_and_native_prompt_use_existing_owners() {
    let host = Host::with_config(true, "light", |config| {
        let commands = config["commands"].as_array_mut().unwrap();
        commands.push(json!({"id":"owned.nav-prompt","title":"qznavprompt","kind":"builtin","target":"snippets","inputPrompt":"Owned navigation"}));
        commands.push(json!({"id":"owned.nav-disabled","title":"Disabled nav","kind":"builtin","target":"quicklinks","enabled":false}));
        commands.push(json!({"id":"owned.nav-alias","title":"Alias nav","kind":"builtin","target":"quicklinks","aliases":["résumé-nav"]}));
    });
    assert_eq!(
        host.rpc(json!({"op":"execute","id":"owned.nav-disabled"}))["ok"],
        false
    );
    host.ok(json!({"op":"execute","id":"résumé-nav"}));
    host.ok(json!({"op":"show","mode":"commands","query":"qznavprompt"}));
    host.wait_probe("read", |v| v["rows"] == 1);
    host.ok(json!({"op":"native.test","action":"key-event","code":36,"chars":"\r","flags":0}));
    let opened = host.wait_probe("read", |v| v["editorId"] == "owned.nav-prompt");
    let token = opened["editorToken"].as_u64().unwrap();
    host.ok(json!({"op":"native.test","action":"run-inline-type","text":"\u{2003}\u{3000}"}));
    host.ok(json!({"op":"native.test","action":"key-event","code":36,"chars":"\r","flags":0}));
    host.wait_probe("read", |v| {
        v["editorToken"] == token && v.to_string().contains("Enter Owned navigation.")
    });
    host.ok(json!({"op":"native.test","action":"run-inline-type","text":"Owned literal"}));
    host.ok(json!({"op":"native.test","action":"key-event","code":36,"chars":"\r","flags":0}));
    host.wait_probe("read", |v| {
        v["editorToken"].is_null() && v["modelQuery"] == ""
    });
    let usage = host.ok(json!({"op":"usage"}));
    assert_eq!(usage["result"]["owned.nav-prompt"], 1);
    assert_eq!(usage["result"]["owned.nav-alias"], 1);
    assert!(usage["result"]["owned.nav-disabled"].is_null());
    assert_eq!(host.probe("read")["result"]["windowShown"], false);
}

#[test]
fn navigation_usage_save_failure_reports_completed_action_without_retry() {
    let host = Host::new(true, "light");
    let usage = host.profile.join("usage.json");
    if usage.exists() {
        fs::remove_file(&usage).unwrap();
    }
    fs::create_dir(&usage).unwrap();
    let reply = host.rpc(json!({"op":"execute","id":"builtin.quicklinks"}));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["actionCompleted"], true, "{reply}");
    assert!(reply["error"]
        .as_str()
        .unwrap()
        .contains("usage could not be saved"));
    host.wait_probe("read", |v| {
        v["accessibilityText"]
            .as_array()
            .unwrap()
            .contains(&json!("Search quicklinks…"))
    });
    let logs = host.ok(json!({"op":"logs"}));
    assert_eq!(
        logs["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["commandID"] == "builtin.quicklinks"
                && entry["phase"] == "command.finish"
                && entry["success"] == true)
            .count(),
        1
    );
    assert!(usage.is_dir());
    assert_eq!(host.probe("read")["result"]["windowShown"], false);
}

#[test]
fn scoped_cli_search_uses_configuration_catalog_in_clipboard_and_utility_modes() {
    let host = Host::with_config(true, "light", |config| {
        config["commands"].as_array_mut().unwrap().extend([
            json!({"id":"owned.catalog-snippet","title":"qzindependent snippet","kind":"snippet","target":"Owned configured text"}),
            json!({"id":"owned.catalog-link","title":"qzindependent link","kind":"url","target":"ruycast-fixture://success/catalog"}),
            json!({"id":"owned.catalog-disabled","title":"qzindependent disabled","kind":"snippet","target":"Disabled","enabled":false}),
        ]);
    });
    host.ok(json!({"op":"clipboard.import","items":[{
        "id":"raycast-owned-catalog","text":"qzindependent clipboard canary","source":"fixture",
        "date":ruycast_native::clipboard_history::now()
    }]}));
    for mode in [
        "commands",
        "clipboard",
        "utility-text-history",
        "quicklinks",
    ] {
        host.ok(json!({"op":"show","mode":mode}));
        for (scope, expected) in [
            ("snippets", "owned.catalog-snippet"),
            ("quicklinks", "owned.catalog-link"),
        ] {
            let result =
                host.ok(json!({"op":"search","mode":scope,"query":"qzindependent","limit":0}));
            let ids = result["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                ids,
                [expected],
                "CLI scope {scope} changed with visible mode {mode}: {result}"
            );
        }
    }
    assert_eq!(host.probe("read")["result"]["windowShown"], false);
}
