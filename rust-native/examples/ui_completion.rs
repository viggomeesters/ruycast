//! Owned native process checks. Never opens a window or a personal profile.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
// Matrix artifacts are data selected by the source-bound validation builder,
// not an independent SDK or execution grant.
fn ui_validation_file(root: &Path, kind: &str) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let value = std::env::var_os(format!("RUYCAST_UI_VALIDATION_{kind}"));
    let expected = std::env::var_os(format!("RUYCAST_UI_VALIDATION_{kind}_SHA"));
    let (value, expected) = match (value, expected) {
        (None, None) => return None,
        (Some(value), Some(expected)) => (value, expected),
        _ => panic!("Incomplete UI validation artifact pair"),
    };
    let expected = expected.to_str().expect("UI validation hash must be UTF8");
    assert!(expected.len()==64 && expected.bytes().all(|b|b.is_ascii_hexdigit()), "Invalid UI validation hash");
    let (tree,leaf) = match kind {
        "HOST" => ("validation-native","ruycast-native"),
        "FIXTURE" => ("validation-build","private-results-fixture"),
        _ => panic!("Unknown UI validation artifact role"),
    };
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "UI validation artifact must be absolute");
    let prefix = root.join("rust-native/target").join(tree);
    let parts = path.strip_prefix(&prefix).expect("UI validation artifact escaped its role target").components().collect::<Vec<_>>();
    assert!(parts.len()==3 && matches!(parts[0],std::path::Component::Normal(target) if target=="aarch64-apple-darwin"||target=="x86_64-apple-darwin") && parts[1].as_os_str()=="debug" && parts[2].as_os_str()==leaf, "UI validation target/artifact identity differs");
    let metadata = fs::symlink_metadata(&path).expect("UI validation artifact missing");
    assert!(metadata.is_file() && metadata.uid()==unsafe{libc::geteuid()}, "UI validation artifact must be an own regular file");
    assert_eq!(fs::canonicalize(&path).unwrap(),path,"UI validation artifact path must be canonical without symlinks");
    assert_eq!(format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),expected.to_ascii_lowercase(),"UI validation artifact hash differs");
    Some(path)
}
fn ui_validation_output(root: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let value = std::env::var_os("RUYCAST_UI_VALIDATION_EVIDENCE");
    let matrix = std::env::var_os("RUYCAST_UI_VALIDATION_HOST").is_some();
    if !matrix {
        assert!(value.is_none() && std::env::var_os("RUYCAST_UI_VALIDATION_FIXTURE").is_none() && std::env::var_os("RUYCAST_UI_VALIDATION_FIXTURE_SHA").is_none(),"UI validation output/fixture requires its Host pair");
        return None;
    }
    let path = PathBuf::from(value.expect("UI validation evidence directory required"));
    assert!(path.is_absolute(),"UI validation evidence must be absolute");
    let prefix = root.join("rust-native/target/verification");
    let parts=path.strip_prefix(prefix).expect("UI validation evidence escaped verification").components().collect::<Vec<_>>();
    assert!(parts.len()==2 && matches!(parts[0],std::path::Component::Normal(stage) if stage.to_str().is_some_and(|s|!s.is_empty()&&s.len()<=128&&s.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.')))) && parts[1].as_os_str()=="frames","UI validation evidence stage differs");
    for directory in [path.parent().unwrap(),path.as_path()] {
        let metadata=fs::symlink_metadata(directory).expect("UI validation evidence directory missing");
        assert!(metadata.is_dir()&&metadata.uid()==unsafe{libc::geteuid()}&&metadata.mode()&0o077==0,"UI validation evidence requires own private directories");
        assert_eq!(fs::canonicalize(directory).unwrap(),directory,"UI validation evidence directory must be canonical");
    }
    Some(path)
}
fn ui_validation_date_fixture(root: &Path) -> PathBuf {
    match ui_validation_file(root,"FIXTURE") {
        Some(path) => {assert!(std::env::var_os("RUYCAST_UI_VALIDATION_HOST").is_some(),"UI validation fixture requires Host pair");path},
        None => {
            assert!(std::env::var_os("RUYCAST_UI_VALIDATION_HOST").is_none(),"Date matrix requires its matching validation fixture");
            root.join("rust-native/target/native-tools/debug/private-results-fixture")
        }
    }
}
struct Host {
    child: Child,
    base: PathBuf,
    profile: PathBuf,
    out: PathBuf,
}
impl Host {
    fn new(root: &Path, theme: &str) -> Self {
        Self::with_clipboard(root, theme, None, false)
    }
    fn with_clipboard(root: &Path, theme: &str, clipboard: Option<&str>, shared_board: bool) -> Self {
        Self::with_inputs(root, theme, clipboard, shared_board, false)
    }
    fn with_inputs(root: &Path, theme: &str, clipboard: Option<&str>, shared_board: bool, keyboard: bool) -> Self {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Set RUYCAST_NATIVE_TEST_FIXTURE_DIR to an existing marked fixture parent"),
        );
        assert!(parent.is_absolute(), "Fixture parent must be absolute");
        assert!(
            fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir()),
            "Fixture parent must be an existing directory, not a symlink"
        );
        assert!(
            fs::symlink_metadata(parent.join(".ruycast-ui-fixture-parent"))
                .is_ok_and(|m| m.is_file()),
            "Fixture parent must have its own regular .ruycast-ui-fixture-parent marker"
        );
        let base = parent.join(format!(
            "u{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros()
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            "own-disposable-fixture-v1",
        )
        .unwrap();
        let profile = base.join("p");
        fs::create_dir(&profile).unwrap();
        let icons = profile.join("icon-fixtures");
        fs::create_dir(&icons).unwrap();
        let png = [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112,
            80, 248, 15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69,
            78, 68, 174, 66, 96, 130,
        ];
        for key in [
            "app:dev.fixture.synthetic",
            "path:/synthetic/folder",
            "website:https://example.invalid/",
        ] {
            let key = format!("{:x}", Sha256::digest(key.as_bytes()));
            fs::write(icons.join(format!("{key}.png")), png).unwrap();
            fs::write(icons.join(format!("{key}.name")), "Synthetic app").unwrap();
        }
        fs::create_dir(base.join("Applications")).unwrap();
        if std::env::args().nth(1).as_deref()==Some("--actions-menu-only") {
            // Real discovery reads only this synthetic bundle's Info.plist;
            // there is no executable, launch or installed-app inventory.
            let contents=base.join("Applications/Discovered Action Binding.app/Contents");
            fs::create_dir_all(&contents).unwrap();
            fs::write(contents.join("Info.plist"),b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.action-binding</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleDisplayName</key><string>Discovered Action Binding</string></dict></plist>").unwrap();
        }
        let mut config: Value =
            serde_json::from_str(include_str!("../../config/default.json")).unwrap();
        config["theme"] = json!(theme);
        config["commands"] = json!([
         {"id":"snippet.fixture","title":"Synthetic snippet — Unicode 日本語","kind":"snippet","target":"Original text","aliases":["fixture"],"keyword":";fixture","hotkey":"left-cmd+shift+f12"},
         {"id":"script.fixture","title":"Synthetic script","kind":"script","target":"/usr/bin/true","arguments":["first\nsecond","literal $() argument"],"aliases":[]},
         {"id":"url.fixture","title":"Synthetic quicklink","kind":"url","target":"https://example.invalid/private?q=secret","aliases":["site"],"hotkey":"ctrl+alt+shift+cmd+k"},
         {"id":"path.fixture","title":"Synthetic folder","kind":"path","target":"/synthetic/folder","aliases":[]},
         {"id":"app.fixture","title":"Synthetic app","kind":"app","target":"dev.fixture.synthetic","aliases":[]},
         {"id":"window.fixture","title":"Left half","kind":"window","target":"left-half","aliases":[]},
         {"id":"url.inputs","title":"Synthetic inputs","kind":"url","target":"https://example.invalid/{first name=\"First\"}/{second name=\"Second\"}","aliases":[]}
        ]);
        if keyboard {
            config["commands"].as_array_mut().unwrap().push(json!({"id":"script.keyboard-error","title":"Owned failing command","kind":"script","target":"/usr/bin/false","arguments":[],"aliases":[],"hotkey":"cmd+a"}));
        }
        fs::write(
            profile.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let binary = if let Some(path) = ui_validation_file(root,"HOST") { let _ = ui_validation_file(root,"FIXTURE"); path } else if std::env::args().nth(1).as_deref() == Some("--polish-bindings-only") {
            let path = PathBuf::from(std::env::var_os("RUYCAST_POLISH_BINDINGS_HOST").expect("Explicit qualified Polish fixture host required"));
            assert!(path.is_absolute(), "Polish fixture host must be absolute");
            assert_eq!(format!("{:x}", Sha256::digest(fs::read(&path).unwrap())), "185dd5388c4ce3c57e4254e56e949233fda066742189b8d89888e82fa8e42f84", "Polish fixture host changed");
            path
        } else if std::env::args().nth(1).as_deref() == Some("--polish-original-only") {
            let path=PathBuf::from(std::env::var_os("RUYCAST_POLISH_ORIGINAL_HOST").expect("Explicit qualified current Polish host required"));
            assert!(path.is_absolute(), "Polish host must be absolute");
            assert_eq!(format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),"1987db672c1a9770a0ccec48c5f85039c325ab72b6725ba184904f71f39635a4","Current Polish host changed");
            path
        } else if matches!(std::env::args().nth(1).as_deref(),Some("--feedback-actions-only"|"--interaction-trace-only")) {
            let path=PathBuf::from(std::env::var_os("RUYCAST_INTERACTION_TRACE_HOST").expect("Explicit qualified integrated Trace host required"));
            assert!(path.is_absolute(), "Integrated Trace host must be absolute");
            assert_eq!(format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),"755cbc6fd948e469994648cdda1cbc128cd9da5ec80e7270d24077c65d6db47f","Integrated Trace host changed");
            path
        } else if std::env::args().nth(1).as_deref() == Some("--item-binding-original-only") {
            let path=PathBuf::from(std::env::var_os("RUYCAST_ITEM_BINDING_HOST").expect("Explicit qualified item-binding host required"));
            assert!(path.is_absolute(),"Item-binding host must be absolute");
            assert_eq!(format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),"862d47e6585ef4df50ed9c1b21060b62300d156450e26ceb206e29fbb0c096d0","Item-binding host changed");
            path
        } else if std::env::args().nth(1).as_deref() == Some("--palette-lifecycle-only") {
            let path=PathBuf::from(std::env::var_os("RUYCAST_PALETTE_LIFECYCLE_HOST").expect("Explicit qualified palette lifecycle host required"));
            assert!(path.is_absolute(),"Palette lifecycle host must be absolute");
            assert_eq!(format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),"45c6fe79c3d0435df20a9afcecec6bf25580957b3534857df895232f2cb03445","Palette lifecycle host changed");
            path
        } else if std::env::args().nth(1).as_deref() == Some("--task-date-only") {
            let path = PathBuf::from(std::env::var_os("RUYCAST_TASK_DATE_HOST").expect("Explicit qualified Date fixture host required"));
            assert!(path.is_absolute(), "Date fixture host must be absolute");
            assert_eq!(format!("{:x}", Sha256::digest(fs::read(&path).unwrap())), "185dd5388c4ce3c57e4254e56e949233fda066742189b8d89888e82fa8e42f84", "Date fixture host changed");
            path
        } else {
            root.join("rust-native/target/native-tools/debug/ruycast-native")
        };
        let out = ui_validation_output(root).unwrap_or_else(||root.join("rust-native/target/ui-evidence"));
        fs::create_dir_all(&out).unwrap();
        let mut command = Command::new(&binary);
        command
            .arg("--native-hidden")
            .env_clear()
            .env("RUYCAST_HOME", &profile)
            .env("RUYCAST_CONFIG_DIR", &profile)
            .env("HOME", &base)
            .env("RUYCAST_APP_BINARY", &binary)
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                json!([base.join("Applications")]).to_string(),
            )
            .env(
                "DYLD_INSERT_LIBRARIES",
                root.join("rust-native/target/clipboard-history-effects.dylib"),
            )
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &base)
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD", "named")
            .env("RUYCAST_NATIVE_TEST_SNIPPET", "1")
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1")
            .env("RUYCAST_NATIVE_TEST_ICON_FIXTURES", "owned")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if let Some(text) = clipboard { command.env("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT", text); }
        if shared_board {
            command.env("RUYCAST_NATIVE_TEST_CREATION_BOARD", "shared");
            command.env("RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER", &base);
        }
        if keyboard {
            use std::os::unix::fs::MetadataExt;
            let sdk=PathBuf::from(std::env::var_os("RUYCAST_TEST_KEYBOARD_SDK").expect("Qualified owned keyboard fixture required"));
            let sha=std::env::var("RUYCAST_TEST_KEYBOARD_SHA").expect("Owned keyboard fixture hash required");
            assert_eq!(sdk,root.join("rust-native/target/verification/native-profile/host-sdks/NativeKeyboard.dylib"));
            let metadata=fs::symlink_metadata(&sdk).unwrap();
            assert!(metadata.is_file()&&metadata.uid()==unsafe{libc::geteuid()}&&metadata.nlink()==1);
            assert_eq!(fs::canonicalize(&sdk).unwrap(),sdk);
            assert_eq!(format!("{:x}",Sha256::digest(fs::read(&sdk).unwrap())),sha);
            fs::write(base.join(".ruycast-keyboard-fixture"),"own-keyboard-fixture-v1").unwrap();
            command.env("DYLD_INSERT_LIBRARIES",&sdk).env("RUYCAST_NATIVE_TEST_KEYBOARD","1")
                .env("RUYCAST_KEYBOARD_FIXTURE_HOME",&base).env("RUYCAST_KEYWORD_FIXTURE_HOME",&base)
                .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD","trace")
                .env_remove("RUYCAST_NATIVE_TEST_CLIPBOARD").env_remove("RUYCAST_NATIVE_TEST_SNIPPET");
        }
        let child = command.spawn()
            .unwrap();
        let h = Self {
            child,
            base,
            profile,
            out,
        };
        let end = Instant::now() + Duration::from_secs(8);
        while !h.profile.join("control.sock").exists() {
            assert!(Instant::now() < end, "Own host did not start");
            thread::sleep(Duration::from_millis(20));
        }
        h.wait(|s| s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]);
        h
    }
    fn rpc(&self, value: Value) -> Value {
        let mut stream = UnixStream::connect(self.profile.join("control.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        writeln!(stream, "{value}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["ok"], true, "{value}");
        value["result"].clone()
    }
    #[track_caller]
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(7);
        loop {
            let s = self.rpc(json!({"op":"status"}));
            if s["configurationUpdating"] != true && predicate(&s) {
                return s;
            }
            assert!(Instant::now() < end, "State timeout: {s}; native read: {}",self.probe(false,"read",json!({})));
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn probe(&self, settings: bool, action: &str, extra: Value) -> Value {
        let mut r = json!({"op":"native.test","action":action});
        if settings {
            r["surface"] = json!("settings");
        }
        if let Some(extra) = extra.as_object() {
            for (k, v) in extra {
                r[k] = v.clone();
            }
        }
        self.rpc(r)
    }
    fn settled(&self) {
        self.wait(|s| {
            s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
                && s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]
        });
    }
    fn capture(&self, settings: bool, name: &str) {
        self.settled();
        let v = self.probe(settings, "screenshot", json!({}));
        assert_eq!(v["windowShown"], false);
        assert_eq!(v["windowKey"], false);
        assert_eq!(v["windowMain"], false);
        fs::copy(
            self.profile.join(if settings {
                "native-settings-probe.png"
            } else {
                "native-probe.png"
            }),
            self.out.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            self.out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap();
    }
    fn edit(&self, id: &str) {
        self.rpc(json!({"op":"show","mode":"settings","query":id}));
        self.wait(|s| {
            s["managementUI"]["selectedId"] == id
                && self.probe(true, "read", json!({}))["input"] == id
        });
        self.probe(
            true,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        self.settled();
        let v = self.probe(true, "read", json!({}));
        let index = v["actionLabels"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v.as_str().is_some_and(|s| s.starts_with("Edit")))
            .expect("Edit action");
        self.probe(true, "action-button", json!({"index":index}));
        self.wait(|_| {
            let v = self.probe(true, "read", json!({}));
            v["formVisible"] == true && v["editorId"] == id
        });
        assert_eq!(self.probe(true, "read", json!({}))["formVisible"], true);
    }
    fn prefs(&self) {
        self.rpc(json!({"op":"show","mode":"settings"}));
        self.settled();
        self.probe(true, "clipboard-preferences", json!({}));
        self.wait(|s| {
            s["managementUI"]["tab"] == "clipboard"
                && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
        });
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // These branches explicitly check successful normal quit before Drop.
        // All existing branches retain their original quit/cleanup path.
        if !matches!(std::env::args().nth(1).as_deref(),Some("--task-date-only"|"--interaction-trace-only"|"--palette-lifecycle-only"|"--item-binding-original-only"))
            || self.child.try_wait().ok().flatten().is_none()
        {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.rpc(json!({"op":"quit"}))
            }));
        }
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() > end {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    if std::env::args().nth(1).as_deref() == Some("--actions-menu-only") && std::env::args().any(|arg|arg=="--controlled-alias-gap"||arg=="--controlled-alias-gap-ack") {
        controlled_alias_gap(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--task-date-only") {
        task_date(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--item-binding-original-only") {
        item_binding_original(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--palette-lifecycle-only") {
        palette_lifecycle(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--interaction-trace-only") {
        interaction_trace(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--polish-original-only") {
        polish_original(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--polish-bindings-only") {
        polish_bindings(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--settings-only") {
        settings_parity(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--render-updates-only") {
        render_updates(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--actions-menu-only") {
        actions_menu(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--feedback-actions-only") {
        feedback_actions(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--snippet-hotkey-only") {
        snippet_hotkey(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--snippet-links-only") {
        snippet_links(&root);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--script-advanced-only") {
        script_advanced(&root);
        return;
    }
    for theme in ["light", "dark"] {
        let h = Host::new(&root, theme);
        for id in ["snippet.fixture", "script.fixture", "url.fixture"] {
            h.edit(id);
            let before = h.probe(true, "editor-layout", json!({}));
            if id == "snippet.fixture" {
                h.probe(true, "snippet-type", json!({"text":" Local undo value"}));
                assert_eq!(h.probe(true, "editor-mark", json!({}))["marked"], true);
            }
            for (w, height) in [(680, 480), (980, 680), (680, 480)] {
                h.probe(true, "dashboard-size", json!({"width":w,"height":height}));
                let after = h.probe(true, "editor-layout", json!({}));
                assert_eq!(before["textIdentity"], after["textIdentity"]);
                assert_eq!(
                    before["fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v["identity"].clone())
                        .collect::<Vec<_>>(),
                    after["fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v["identity"].clone())
                        .collect::<Vec<_>>()
                );
                let dw = after["document"][2].as_f64().unwrap();
                for field in after["fields"].as_array().unwrap() {
                    let f = &field["frame"];
                    assert!(
                        f[0].as_f64().unwrap() + f[2].as_f64().unwrap() <= dw + 1.,
                        "Field clips horizontally: {field}"
                    );
                }
                if id == "snippet.fixture" {
                    assert_eq!(after["marked"], true);
                    assert_eq!(after["undo"], true);
                }
                h.probe(true, "command-scroll", json!({"y":0.}));
                h.capture(true, &format!("editor-{id}-{theme}-{w}-{height}"));
                h.probe(true, "command-scroll", json!({"y":2000.}));
                h.capture(true, &format!("editor-bottom-{id}-{theme}-{w}-{height}"));
                let record = h.probe(true, "editor-record", json!({}));
                let label = after["fields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| f["text"] == "Shortcut")
                    .unwrap();
                assert_eq!(
                    record["frame"][1].as_f64().unwrap(),
                    label["frame"][1].as_f64().unwrap() + 24.
                );
                assert_eq!(record["inDocument"], true);
                h.probe(
                    true,
                    "command-scroll",
                    json!({"y":record["frame"][1].as_f64().unwrap()-40.}),
                );
                let record = h.probe(true, "editor-record", json!({}));
                assert_eq!(record["hit"], true, "{record}");
                assert_eq!(record["visibleCenter"], true, "{record}");
                fs::write(
                    h.out
                        .join(format!("recorder-{id}-{theme}-{w}-{height}.json")),
                    serde_json::to_vec_pretty(&record).unwrap(),
                )
                .unwrap();
                h.capture(true, &format!("editor-recorder-{id}-{theme}-{w}-{height}"));
            }
            if id == "snippet.fixture" {
                h.probe(true, "editor-unmark", json!({}));
            }
            h.probe(true, "editor-record", json!({"click":true}));
            h.wait(|_| h.probe(true, "editor-record", json!({}))["recording"] == true);
            h.probe(true, "cancel-editor", json!({}));
            h.settled();
        }
        h.rpc(json!({"op":"show","mode":"settings","query":"snippet.fixture"}));
        h.settled();
        h.probe(true, "inline-alias", json!({"id":"snippet.fixture"}));
        h.settled();
        let binding = h.probe(true, "inline-mark", json!({"text":"日本"}));
        assert_eq!(binding["marked"], true);
        for (w, height) in [(680, 480), (980, 680)] {
            h.probe(true, "dashboard-size", json!({"width":w,"height":height}));
            let v = h.probe(true, "inline-marked-read", json!({}));
            assert_eq!(v["marked"], true);
            assert_eq!(v["editorToken"], binding["editorToken"]);
            assert_eq!(
                h.probe(true, "inline-control-read", json!({}))["hitEditor"],
                true
            );
            h.capture(true, &format!("inline-ime-{theme}-{w}-{height}"));
        }
        h.probe(true, "inline-unmark", json!({}));
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        h.probe(true, "inline-hotkey", json!({"id":"snippet.fixture"}));
        h.wait(|_| h.probe(true, "read", json!({}))["recording"] == true);
        let identity = h.probe(true, "editor-record", json!({}))["identity"].clone();
        for (w, height) in [(680, 480), (980, 680)] {
            h.probe(true, "dashboard-size", json!({"width":w,"height":height}));
            let record = h.probe(true, "editor-record", json!({}));
            assert_eq!(record["identity"], identity);
            assert_eq!(record["inDocument"], false);
            assert_eq!(record["hit"], true, "{record}");
            assert_eq!(record["recording"], true);
            h.capture(true, &format!("binding-recorder-{theme}-{w}-{height}"));
        }
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        h.rpc(json!({"op":"show","query":"url.inputs"}));
        h.wait(|s| {
            s["nativeUI"]["selectedId"] == "url.inputs"
                && h.probe(false, "read", json!({}))["input"] == "url.inputs"
        });
        h.probe(
            false,
            "key-event",
            json!({"code":36,"flags":0,"chars":"\r"}),
        );
        h.wait(|_| h.probe(false, "template-read", json!({}))["fields"] == 2);
        h.probe(false, "template-mark", json!({"index":0,"text":"日本"}));
        let form = h.probe(false, "template-read", json!({}));
        assert_eq!(form["marked"], true);
        for (w, height) in [(680, 480), (980, 680)] {
            h.probe(false, "dashboard-size", json!({"width":w,"height":height}));
            let v = h.probe(false, "template-read", json!({}));
            assert_eq!(v["marked"], true);
            assert_eq!(v["token"], form["token"]);
            h.capture(false, &format!("template-ime-{theme}-{w}-{height}"));
        }
        h.probe(false, "template-unmark", json!({}));
        h.probe(false, "cancel-editor", json!({}));
        h.settled();
        h.prefs();
        for (w, height) in [(680, 480), (980, 680)] {
            h.probe(true, "dashboard-size", json!({"width":w,"height":height}));
            let v = h.probe(true, "clipboard-preferences-screenshot", json!({}));
            assert_eq!(v["controls"][0]["role"], "AXCheckBox");
            assert_eq!(v["controls"][0]["value"], "1");
            assert_eq!(v["controls"][1]["role"], "AXTextField");
            assert_eq!(v["controls"][3]["role"], "AXTextArea");
            fs::copy(
                h.profile.join("native-clipboard-preferences-probe.png"),
                h.out
                    .join(format!("clipboard-preferences-{theme}-{w}-{height}.png")),
            )
            .unwrap();
            fs::write(
                h.out
                    .join(format!("clipboard-preferences-{theme}-{w}-{height}.json")),
                serde_json::to_vec_pretty(&v).unwrap(),
            )
            .unwrap();
            h.capture(
                true,
                &format!("clipboard-preferences-viewport-{theme}-{w}-{height}"),
            );
            h.probe(
                true,
                "clipboard-preferences-probe",
                json!({"operation":"scroll","y":2000.}),
            );
            let v = h.probe(true, "clipboard-preferences-probe", json!({}));
            assert!(
                v["saveVisibleRect"][3].as_f64().unwrap() > 0.,
                "Save unreachable: {v}"
            );
            h.capture(
                true,
                &format!("clipboard-preferences-bottom-{theme}-{w}-{height}"),
            );
        }
        h.probe(
            true,
            "clipboard-preferences-probe",
            json!({"operation":"retain"}),
        );
        h.rpc(json!({"op":"clipboard.pause","paused":true}));
        h.settled();
        h.capture(true, &format!("clipboard-preferences-paused-{theme}"));
        h.probe(true, "window-close", json!({}));
        h.settled();
        h.prefs();
        let before = fs::read(h.profile.join("config.json")).unwrap();
        h.probe(
            true,
            "clipboard-preferences-probe",
            json!({"operation":"stale"}),
        );
        h.settled();
        assert_eq!(before, fs::read(h.profile.join("config.json")).unwrap());
        assert_eq!(h.rpc(json!({"op":"status"}))["clipboardPaused"], true);
        h.probe(
            true,
            "clipboard-preferences-probe",
            json!({"operation":"resume"}),
        );
        h.wait(|s| s["clipboardPaused"] == false);
        let before = fs::read(h.profile.join("config.json")).unwrap();
        h.probe(
            true,
            "clipboard-preferences-save",
            json!({"limit":0,"enabled":false}),
        );
        h.wait(|_| {
            h.probe(true, "clipboard-preferences-probe", json!({}))["text"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s.as_str().is_some_and(|s| s.contains("Clipboard bounds:")))
        });
        assert_eq!(before, fs::read(h.profile.join("config.json")).unwrap());
        h.capture(true, &format!("clipboard-preferences-error-{theme}"));
        assert_eq!(
            h.probe(true, "clipboard-preferences-probe", json!({}))["controls"][0]["value"],
            "0"
        );
        h.probe(
            true,
            "clipboard-preferences-save",
            json!({"limit":12,"retentionDays":7,"enabled":true,"excludedApps":["dev.fixture.private"]}),
        );
        h.wait(|_| {
            serde_json::from_slice::<Value>(&fs::read(h.profile.join("config.json")).unwrap())
                .unwrap()["clipboard"]["limit"]
                == 12
        });
        h.rpc(json!({"op":"show","query":""}));
        h.settled();
        h.capture(false, &format!("launcher-rows-{theme}"));
        let now = ruycast_native::clipboard_history::now();
        h.rpc(json!({"op":"clipboard.import","items":[{"id":"raycast-synthetic-text","text":"Synthetic row text","source":"dev.fixture.synthetic","date":now},{"id":"raycast-synthetic-files","text":"Files: synthetic.txt","source":"dev.fixture.synthetic","date":now,"contentType":"files","fileURLs":["file:///synthetic/synthetic.txt"]}]}));
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.settled();
        thread::sleep(Duration::from_millis(100));
        h.capture(false, &format!("clipboard-rows-{theme}"));
        let read = h.probe(false, "read", json!({}));
        assert!(read["screenText"].as_array().unwrap().iter().any(|s| s
            .as_str()
            .is_some_and(|s| s.contains("Source  Synthetic app"))));
        h.probe(
            false,
            "clipboard-retained-control",
            json!({"operation":"retain"}),
        );
        h.probe(
            false,
            "clipboard-retained-control",
            json!({"operation":"back"}),
        );
        h.settled();
        assert!(!h.probe(false, "read", json!({}))["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str() == Some("Source  Synthetic app")));
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.settled();
        let paused = h.rpc(json!({"op":"status"}))["clipboardPaused"].clone();
        h.probe(
            false,
            "clipboard-retained-control",
            json!({"operation":"stale"}),
        );
        h.settled();
        assert_eq!(paused, h.rpc(json!({"op":"status"}))["clipboardPaused"]);
        for visible in [
            json!([-1440., -900., 1280., 720.]),
            json!([0., 0., 640., 480.]),
        ] {
            let v = h.probe(
                false,
                "palette-geometry",
                json!({"visible":visible,"clipboard":true}),
            );
            let f = &v["frame"];
            assert!(f[2].as_f64().unwrap() <= visible[2].as_f64().unwrap());
            assert!(f[3].as_f64().unwrap() <= visible[3].as_f64().unwrap());
            h.capture(false, &format!("clipboard-clamped-{theme}-{}", visible[2]));
        }
        h.probe(
            false,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        h.settled();
        let v = h.probe(false, "read", json!({}));
        assert_eq!(
            v["actionLabels"],
            json!(["Paste", "Copy", "Delete", "Settings"])
        );
        h.capture(false, &format!("clipboard-menu-{theme}"));
        h.probe(false, "action-button", json!({"index":3}));
        h.wait(|s| s["managementUI"]["visible"] == true && s["nativeUI"]["visible"] == false);
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.settled();
        h.probe(
            false,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        h.settled();
        h.probe(false, "retain-action-button", json!({"index":3}));
        h.probe(
            false,
            "key-event",
            json!({"code":53,"flags":0,"chars":"\u{1b}"}),
        );
        h.settled();
        h.probe(false, "old-input-button", json!({}));
        h.settled();
        assert_eq!(h.rpc(json!({"op":"status"}))["nativeUI"]["visible"], true);
        h.probe(
            false,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        h.settled();
        for _ in 0..3 {
            h.probe(false, "key-event", json!({"code":125,"flags":0,"chars":""}));
        }
        h.settled();
        h.probe(
            false,
            "key-event",
            json!({"code":36,"flags":0,"chars":"\r"}),
        );
        h.wait(|s| s["managementUI"]["visible"] == true && s["nativeUI"]["visible"] == false);
        h.rpc(json!({"op":"clipboard.clear"}));
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.settled();
        h.probe(
            false,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        h.settled();
        assert_eq!(
            h.probe(false, "read", json!({}))["actionLabels"],
            json!(["Settings"])
        );
        h.capture(false, &format!("clipboard-empty-menu-{theme}"));
        h.probe(false, "retain-action-button", json!({"index":0}));
        h.probe(false, "action-button", json!({"index":0}));
        h.wait(|s| s["managementUI"]["visible"] == true && s["nativeUI"]["visible"] == false);
        // The old native menu button must not operate on the reopened screen.
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.settled();
        h.probe(false, "old-input-button", json!({}));
        h.settled();
        assert_eq!(h.rpc(json!({"op":"status"}))["nativeUI"]["visible"], true);
        h.probe(
            false,
            "key-event",
            json!({"code":40,"flags":1u64<<20,"chars":"k"}),
        );
        h.settled();
        h.probe(false, "old-input-button", json!({}));
        h.settled();
        assert_eq!(h.rpc(json!({"op":"status"}))["nativeUI"]["visible"], true);
        h.probe(
            false,
            "key-event",
            json!({"code":36,"flags":0,"chars":"\r"}),
        );
        h.wait(|s| s["managementUI"]["visible"] == true && s["nativeUI"]["visible"] == false);
    }
    println!(
        "{}",
        json!({"ok":true,"themes":2,"editorResizeCases":18,"recorderPositionAndHitCases":18,"recorderClicks":6,"bindingRecorderResizeCases":4,"emptyAndFullSettingsClickAndReturn":true,"staleMenuRevoked":true,"inlineIMECases":4,"templateIMECases":4,"preferenceSizes":4,"nativeAXRolesAndValues":true,"retainedControlsRevoked":true,"realSettingsSave":true,"physicalInput":false})
    );
}
fn task_date(root: &Path) {
    let fixture = ui_validation_date_fixture(root);
    assert!(fixture.is_file(), "Build the existing owned Rust process fixture");
    let mut captures = 0;
    for theme in ["light", "dark"] {
        let mut h = Host::new(root, theme);
        let marker = h.base.join("date-runs.txt");
        let mut config: Value = serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        config["commands"] = json!([{"id":"date.fixture","title":"Task Deadline","kind":"script","target":fixture,"arguments":["fake","date-detail",marker,"{input}"],"inputPrompt":"Deadline YYYY-MM-DD","aliases":["deadline"]}]);
        fs::write(h.profile.join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        h.rpc(json!({"op":"reload"}));
        h.settled();
        let key = |code: u16, flags: u64, chars: &str| { h.probe(false,"key-event",json!({"code":code,"flags":flags,"chars":chars})); };
        let type_value = |inline: bool, text: &str| {
            key(0, 1 << 20, "a");
            h.probe(false,if inline {"run-inline-type"}else{"run-input-type"},json!({"text":text}));
            h.settled();
            let v = h.probe(false,if inline {"run-inline-read"}else{"run-input-read"},json!({}));
            assert_eq!(v["text"],text,"Actual native typing did not replace the date input: {v}");
        };
        let visible_date = |view: &Value| {
            view["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).any(|label| {
                ["maandag ","dinsdag ","woensdag ","donderdag ","vrijdag ","zaterdag ","zondag "].iter().any(|day|label.starts_with(*day))
                    && label.split_whitespace().last().is_some_and(|year|year.len()==4&&year.bytes().all(|b|b.is_ascii_digit()))
            })
        };
        let capture = |name: &str, present: &[&str], absent: &[&str]| {
            h.wait(|_| {
                let v=h.probe(false,"read",json!({}));
                let text=v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>();
                present.iter().all(|s|text.iter().any(|t|t.contains(*s))) && absent.iter().all(|s|text.iter().all(|t|!t.contains(*s)))
                    && (!["valid","valid-after-error","expanded-valid"].contains(&name) || visible_date(&v))
            });
            h.capture(false,&format!("task-date-{name}-{theme}"));
            let v=h.probe(false,"read",json!({}));
            for flag in ["windowShown","windowKey","windowMain"] { assert_eq!(v[flag],false); }
            assert!(!marker.exists(),"Date preview, validation or expansion executed a process");
            v
        };
        let require_visible_date = |view: &Value| {
            assert!(visible_date(view),"Actual native natural-date preview is absent: {view}");
        };
        h.rpc(json!({"op":"show","mode":"commands","query":"deadline"}));
        h.settled();
        key(36,0,"\r");
        h.wait(|_| h.probe(false,"read",json!({}))["runInputInline"]==true);
        type_value(true,"morgen");
        require_visible_date(&capture("valid",&["Task Deadline"],&["YYYY-MM-DD","Datum niet herkend"]));
        type_value(true,"in 3");
        let invalid = capture("invalid",&["Datum niet herkend"],&["YYYY-MM-DD"]);
        key(36,0,"\r");
        h.settled();
        let retained=h.probe(false,"read",json!({}));
        assert_eq!(retained["editorToken"],invalid["editorToken"]);
        assert_eq!(retained["runInputInline"],true);
        assert!(!marker.exists());
        assert_eq!(h.rpc(json!({"op":"usage"})),json!({}));
        type_value(true,"morgen");
        require_visible_date(&capture("valid-after-error",&["Task Deadline"],&["Datum niet herkend","YYYY-MM-DD"]));
        type_value(true,"clear");
        capture("clear",&["Datum wissen"],&["YYYY-MM-DD"]);
        key(36,1 << 20,"\r");
        h.wait(|_| {let v=h.probe(false,"read",json!({}));v["formVisible"]==true&&v["runInputInline"]==false});
        type_value(false,"in 3");
        let invalid=capture("expanded-invalid",&["Datum","Datum niet herkend"],&["YYYY-MM-DD"]);
        assert!(invalid["screenText"].as_array().unwrap().iter().any(|text|text=="Datum"), "Actual expanded field label must be Datum, separately from the error label: {invalid}");
        assert_eq!(invalid["saveEnabled"],false);
        type_value(false,"morgen");
        let valid=capture("expanded-valid",&["Datum"],&["Datum niet herkend","YYYY-MM-DD"]);
        assert!(valid["screenText"].as_array().unwrap().iter().any(|text|text=="Datum"), "Actual expanded field label must remain Datum: {valid}");
        require_visible_date(&valid);
        assert_eq!(valid["saveEnabled"],true);
        h.probe(false,"cancel-editor",json!({}));
        h.settled();
        assert!(!marker.exists());
        h.rpc(json!({"op":"show","mode":"commands","query":"deadline"}));
        h.settled();key(36,0,"\r");
        h.wait(|_|h.probe(false,"read",json!({}))["runInputInline"]==true);
        type_value(true,"morgen");
        let token=h.probe(false,"read",json!({}))["editorToken"].as_u64().unwrap();
        let frozen=h.rpc(json!({"op":"native.test","action":"date-reference","token":token,"now":946728000.}));
        assert_eq!(frozen["input"],"morgen");
        assert_eq!(frozen["value"],"2000-01-02");
        h.wait(|_|h.probe(false,"read",json!({}))["editorToken"]==frozen["token"]);
        capture("frozen",&[frozen["label"].as_str().unwrap()],&["YYYY-MM-DD"]);
        fs::write(h.out.join(format!("task-date-clock-{theme}.json")),serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
        key(36,0,"\r");key(36,0,"\r");
        h.wait(|s|s["scriptsRunning"]==0&&h.probe(false,"read",json!({}))["resultPanel"]["detail"]==frozen["value"]);
        let result=h.probe(false,"read",json!({}));
        assert_eq!(result["resultPanel"]["detail"],frozen["value"],"Actual Rust child stdout detail must equal the displayed frozen date");
        assert_eq!(fs::read(&marker).unwrap(),b"run\n","Double Return executed the real child twice");
        assert_eq!(h.rpc(json!({"op":"usage"}))["date.fixture"],1);
        h.capture(false,&format!("task-date-result-{theme}"));
        let before_quit=h.rpc(json!({"op":"logs"}));
        fs::write(h.out.join(format!("task-date-execution-{theme}.json")),serde_json::to_vec_pretty(&json!({"frozen":frozen,"result":result,"logs":before_quit,"marker":"run\\n","usage":h.rpc(json!({"op":"usage"}))})).unwrap()).unwrap();
        captures += 8;
        h.rpc(json!({"op":"quit"}));
        let deadline=Instant::now()+Duration::from_secs(3);
        loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success(),"Owned Date host did not quit normally");break;}assert!(Instant::now()<deadline,"Owned Date host did not stop");thread::sleep(Duration::from_millis(5));}
        assert_eq!(fs::read(&marker).unwrap(),b"run\n");
        assert!(!h.profile.join("control.sock").exists());
    }
    println!("{}",json!({"ok":true,"taskDate":true,"themes":2,"captures":captures,"nativeTyping":true,"invalidRefused":true,"frozenValue":"2000-01-02","doubleSubmitOnce":true,"normalQuit":true,"visibleWindowOpened":false,"physicalInput":false}));
}

fn render_updates(root: &Path) {
    let mut captures = 0;
    for theme in ["light", "dark"] {
        let h = Host::new(root, theme);
        let mut config: Value = serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        config["commands"] = json!([
            {"id":"render.finder","title":"Finder","kind":"app","target":"dev.fixture.finder","aliases":[]},
            {"id":"render.obsidian","title":"Obsidian","kind":"app","target":"dev.fixture.obsidian","aliases":[]},
            {"id":"render.greeting","title":"Greeting","kind":"snippet","target":"Synthetic greeting","keyword":";greeting","aliases":[]},
            {"id":"render.maps","title":"Search Google Maps","kind":"url","target":"https://example.invalid/maps","aliases":["gmaps"]}
        ]);
        let replace = |value: &Value| {
            let next = h.profile.join("render-config.candidate");
            fs::write(&next, serde_json::to_vec(value).unwrap()).unwrap();
            fs::rename(next, h.profile.join("config.json")).unwrap();
        };
        let watch = |config: &mut Value, watched: &str| {
            let changed = config["theme"] != watched;
            let revision = h.rpc(json!({"op":"status"}))["configurationRevision"].as_u64().unwrap();
            config["theme"] = json!(watched);
            replace(config);
            h.wait(|s| s["theme"] == watched && (!changed || s["configurationRevision"].as_u64().unwrap() > revision));
            h.settled();
        };
        replace(&config);
        h.rpc(json!({"op":"reload"}));
        h.settled();
        assert_ne!(h.rpc(json!({"op":"status"}))["managementUI"]["visible"], true);
        let frame = |settings: bool, name: &str, present: &[&str], absent: &[&str]| {
            h.wait(|_| {
                let v = h.probe(settings, "read", json!({}));
                let text = v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>();
                present.iter().all(|s| text.iter().any(|t| t.contains(*s)))
                    && absent.iter().all(|s| text.iter().all(|t| !t.contains(*s)))
            });
            h.capture(settings, &format!("render-{name}-{theme}"));
            let v = h.probe(settings, "read", json!({}));
            let text = v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>();
            for required in present { assert!(text.iter().any(|t|t.contains(*required)), "Missing actual {name} label {required}: {v}"); }
            for excluded in absent { assert!(text.iter().all(|t|!t.contains(*excluded)), "Stale actual {name} label {excluded}: {v}"); }
            for key in ["windowShown", "windowKey", "windowMain"] { assert_eq!(v[key], false, "{name}: {v}"); }
            v
        };
        h.rpc(json!({"op":"show","mode":"commands","query":""}));
        frame(false, "initial", &["Finder"], &[]);
        for query in ["o", "ob", "obs", "obsidian"] { h.probe(false, "query", json!({"query":query})); }
        let filtered = frame(false, "filtered", &["Obsidian"], &["Finder"]);
        assert_eq!(filtered["modelQuery"], "obsidian");
        assert_eq!(filtered["input"], "obsidian");
        h.probe(false, "query", json!({"query":"missing zzxx"}));
        let empty = frame(false, "empty", &["No commands found"], &["Obsidian"]);
        assert_eq!(empty["rows"], 0);
        assert!(empty["selectedId"].is_null());
        h.rpc(json!({"op":"show","mode":"snippets","query":""}));
        frame(false, "snippets", &["Greeting"], &["Finder"]);
        h.rpc(json!({"op":"show","mode":"commands","query":""}));
        let restored = frame(false, "restored", &["Finder"], &[]);
        let before_image = fs::read(h.out.join(format!("render-restored-{theme}.png"))).unwrap();
        h.probe(false, "key", json!({"key":"down"}));
        h.wait(|_| h.probe(false,"read",json!({}))["selectedRow"] != restored["selectedRow"]);
        let moved = frame(false, "arrow-selection", &["Finder"], &[]);
        assert_ne!(moved["selectedId"], restored["selectedId"]);
        assert_ne!(fs::read(h.out.join(format!("render-arrow-selection-{theme}.png"))).unwrap(), before_image, "Actual arrow selection did not redraw");
        h.rpc(json!({"op":"show","mode":"commands","query":"obsidian"}));
        h.settled();
        watch(&mut config, "dark");
        h.rpc(json!({"op":"show","mode":"settings","query":""}));
        h.probe(true, "library-tab", json!({"tab":"commands"}));
        let management = frame(true, "settings-dark", &["General", "Finder"], &[]);
        assert!(management["appearance"].as_str().unwrap().contains("DarkAqua"));
        h.probe(true, "query", json!({"query":"gmaps"}));
        frame(true, "settings-search", &["Search Google Maps"], &["Finder"]);
        assert_eq!(h.probe(false,"read",json!({}))["modelQuery"], "obsidian");
        h.probe(true, "library-tab", json!({"tab":"snippets"}));
        frame(true, "settings-snippets", &["Greeting"], &["Finder"]);
        h.edit("render.greeting");
        frame(true, "settings-editor", &["Keyword"], &[]);
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        watch(&mut config, "light");
        h.probe(true, "dashboard-size", json!({"width":680,"height":480}));
        let management = frame(true, "settings-light-small", &["Greeting"], &["Finder"]);
        assert!(!management["appearance"].as_str().unwrap().contains("DarkAqua"));
        let geometry = h.probe(false, "palette-geometry", json!({"visible":[-20000.,-20000.,688.,448.],"clipboard":false}));
        assert_eq!(geometry["frame"][2], 640.);
        assert_eq!(geometry["frame"][3], 400.);
        frame(false, "launcher-light-small", &["Obsidian"], &["Finder"]);
        let png = fs::read(h.out.join(format!("render-launcher-light-small-{theme}.png"))).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        // NSPanel sizes are points; the cached bitmap uses the display's backing scale.
        assert!(width >= 640 && height >= 400);
        assert_eq!(u64::from(width) * 400, u64::from(height) * 640);
        captures += 12;

        // Genuine watcher publication, without an explicit reload or appearance setter.
        for watched in ["dark", "light", "graphite", "system"] {
            watch(&mut config, watched);
            for settings in [false,true] {
                let v = h.probe(settings, "read", json!({}));
                if watched == "system" { assert!(v["explicitAppearance"].is_null(), "{v}"); }
                else { assert_eq!(v["appearance"].as_str().unwrap().contains("DarkAqua"), watched != "light", "{v}"); }
                for key in ["windowShown", "windowKey", "windowMain"] { assert_eq!(v[key], false); }
            }
        }
        let revision = h.rpc(json!({"op":"status"}))["configurationRevision"].as_u64().unwrap();
        config.as_object_mut().unwrap().remove("theme");
        replace(&config);
        h.wait(|s| s["configurationRevision"].as_u64().unwrap() > revision && s["theme"] == "system");
        h.settled();
        for settings in [false,true] { assert!(h.probe(settings,"read",json!({}))["explicitAppearance"].is_null()); }
        h.probe(false, "stale-query", json!({"query":"finder"}));
        h.wait(|s| s["nativeUI"]["selectedId"] == "render.finder");
        let pending = frame(false, "pending-query", &["Finder"], &["Obsidian"]);
        assert_eq!(pending["input"], "finder");
        assert_eq!(pending["modelQuery"], "finder");
        h.probe(false, "ime", json!({}));
        h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["windowShown"],false);
        captures += 1;
    }
    println!("{}", json!({"ok":true,"renderUpdates":true,"themes":2,"originalFramesPerTheme":12,"captures":captures,"watchedTheme":true,"pendingQuery":true,"selectionImageChanged":true,"independentQueries":true,"visibleWindowOpened":false,"physicalInput":false}));
}

fn settings_parity(root: &Path) {
    let mut renders = 0;
    let mut themes = 0;
    for theme in ["light", "dark"] {
        let h = Host::new(root, theme);
        let config = || -> Value {
            serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap()
        };
        h.rpc(json!({"op":"show","mode":"commands","query":"obsidian"}));
        h.settled();
        let launcher = h.probe(false, "read", json!({}));
        assert_eq!(launcher["modelQuery"], "obsidian");
        h.rpc(json!({"op":"show","mode":"settings"}));
        h.settled();
        assert_eq!(h.probe(true, "library-read", json!({}))["tab"], "commands");
        let nav = |tab: &str| {
            h.probe(true, "library-tab", json!({"tab":tab}));
            h.wait(|s| s["managementUI"]["tab"] == tab);
            h.settled();
        };
        let mut disabled = config();
        disabled["commands"][0]["enabled"] = json!(false);
        disabled["commands"][0]["aliases"] = json!(["myfinder", "files-home"]);
        disabled["separateCommandKeys"] = json!(true);
        fs::write(h.profile.join("config.json"), serde_json::to_vec(&disabled).unwrap()).unwrap();
        h.rpc(json!({"op":"reload"}));
        h.settled();
        h.probe(true, "query", json!({"query":"myfinder"}));
        h.settled();
        let library = h.probe(true, "library-read", json!({}));
        assert!(library["rows"].as_array().unwrap().iter().any(|r| r["id"] == "snippet.fixture" && r["enabled"] == false));
        assert_eq!(library["rows"].as_array().unwrap().iter().filter(|r| r["group"] == false).map(|r| r["id"].clone()).collect::<Vec<_>>(), vec![json!("snippet.fixture")]);
        assert_eq!(h.probe(false, "read", json!({}))["modelQuery"], launcher["modelQuery"]);
        nav("snippets");
        let rows = h.probe(true, "library-read", json!({}));
        assert_eq!(rows["rows"].as_array().unwrap().iter().filter(|r| r["group"] == false).map(|r| r["id"].clone()).collect::<Vec<_>>(), vec![json!("snippet.fixture")]);
        assert_eq!(rows["rows"].as_array().unwrap().iter().find(|r| r["id"] == "snippet.fixture").unwrap()["enabled"], false);
        // Exercise both actual recorder dispatch boundaries with literal original keys.
        nav("general");
        for (code, flags, chars, expected) in [
            (123, (1u64 << 18) | (1u64 << 19) | (1u64 << 17) | (1u64 << 20), "", "ctrl+alt+shift+cmd+left"),
            (40, 1u64 << 20, "k", "cmd+k"),
        ] {
            h.probe(true, "settings-record", json!({"index":0}));
            h.wait(|_| h.probe(true, "settings-read", json!({}))["recording"][0] == true);
            h.probe(true, "key-event", json!({"code":code,"flags":flags,"chars":chars}));
            h.wait(|_| config()["launcherHotkey"] == expected && h.probe(true, "settings-read", json!({}))["surface"]["busy"] == false);
            assert_eq!(h.probe(true, "settings-read", json!({}))["recording"][0], false);
            fs::write(h.out.join(format!("settings-parity-recorder-{theme}-{code}.json")), serde_json::to_vec_pretty(&json!({"code":code,"flags":flags,"chars":chars,"expected":expected,"configuration":config(),"control":h.probe(true,"settings-read",json!({}))})).unwrap()).unwrap();
        }
        for (tab, label) in [("dashboard", "Dashboard"), ("general", "General"), ("commands", "Commands"), ("snippets", "Snippets"), ("quicklinks", "Quicklinks"), ("clipboard", "Clipboard"), ("advanced", "Advanced")] {
            nav(tab);
            h.probe(true, "dashboard-size", json!({"width":980,"height":680}));
            let v = h.probe(true, "read", json!({}));
            let text = v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
            assert!(text.contains(label), "Missing {tab} title: {text}");
            if tab == "commands" {
                for required in ["Name", "Alias", "Shortcut", "Enabled", "Synthetic app"] {
                    assert!(text.contains(required), "Missing Commands label {required}: {text}");
                }
            }
            if tab == "general" {
                assert!(h.probe(true, "settings-read", json!({}))["text"].as_array().unwrap().iter().any(|s| s == "Separate Left and Right Command"));
            }
            h.capture(true, &format!("settings-parity-{theme}-{tab}-980-680"));
            renders += 1;
        }
        for tab in ["general", "commands"] {
            nav(tab);
            h.probe(true, "dashboard-size", json!({"width":680,"height":480}));
            let v = h.probe(true, "read", json!({}));
            let text = v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
            for required in if tab == "general" { vec!["Separate Left and Right Command"] } else { vec!["Alias", "Shortcut", "Enabled"] } {
                assert!(text.contains(required), "Missing small {tab} label {required}: {text}");
            }
            h.capture(true, &format!("settings-parity-{theme}-{tab}-680-480"));
            renders += 1;
        }
        nav("general");
        let before = config();
        if before["separateCommandKeys"] != false {
            h.probe(true, "settings-control", json!({"index":1}));
            h.wait(|_| config()["separateCommandKeys"] == false && h.probe(true, "settings-read", json!({}))["surface"]["busy"] == false);
        }
        let text = h.probe(true, "settings-read", json!({}))["text"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Separate Left and Right Command") && text.contains("paused"), "{text}");
        h.capture(true, &format!("settings-parity-{theme}-general-off-680-480"));
        renders += 1;
        assert_eq!(config()["commands"], disabled["commands"]);
        themes += 1;
    }
    println!("{}", json!({"ok":true,"settings":true,"themes":themes,"tabsPerTheme":7,"renders":renders,"hyperLeftAndCommandK":true,"disabledSnippetListed":true,"independentManagementQuery":true,"visibleWindowOpened":false,"physicalInput":false}));
}

fn script_advanced(root: &Path) {
    for theme in ["light", "dark"] {
        let h = Host::new(root, theme);
        let before_config = fs::read(h.profile.join("config.json")).unwrap();
        h.edit("script.fixture");
        h.capture(true, &format!("script-advanced-initial-{theme}"));
        let screen = h.probe(true, "read", json!({}));
        assert!(
            !screen["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Target" || v == "Arguments" || v == "Timeout in seconds"),
            "Script implementation details must start closed: {screen}"
        );
        let closed = h.probe(true, "editor-layout", json!({}));
        assert_eq!(closed["advanced"], false);
        for label in ["Target", "Arguments", "Timeout in seconds"] {
            assert!(closed["fields"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["text"] == label || v["label"] == label)
                .all(|v| v["hidden"] == true));
        }
        h.probe(true, "command-scroll", json!({"y":2000.}));
        h.capture(true, &format!("script-advanced-closed-{theme}"));
        let value = h.probe(true, "command-read", json!({}));
        let open = h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        assert_eq!(open["advanced"], true);
        assert!(open["document"][3].as_f64().unwrap() > closed["document"][3].as_f64().unwrap());
        for field in open["fields"].as_array().unwrap() {
            assert_eq!(field["hidden"], false);
        }
        h.probe(true, "command-scroll", json!({"y":2000.}));
        h.capture(true, &format!("script-advanced-open-{theme}"));
        let closed_again = h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        assert_eq!(closed_again["advanced"], false);
        let reopened = h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        assert_eq!(open["fields"], reopened["fields"]);
        assert_eq!(value, h.probe(true, "command-read", json!({})));
        for (width, height) in [(680, 480), (980, 680)] {
            h.probe(
                true,
                "dashboard-size",
                json!({"width":width,"height":height}),
            );
            let resized = h.probe(true, "editor-layout", json!({}));
            for field in resized["fields"].as_array().unwrap() {
                assert!(
                    field["frame"][0].as_f64().unwrap() + field["frame"][2].as_f64().unwrap()
                        <= resized["document"][2].as_f64().unwrap() + 1.
                );
            }
            h.probe(true, "command-scroll", json!({"y":2000.}));
            h.capture(true, &format!("script-advanced-open-{theme}-{width}"));
            h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
            h.capture(true, &format!("script-advanced-closed-{theme}-{width}"));
            h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
            assert_eq!(value, h.probe(true, "command-read", json!({})));
        }
        h.probe(
            true,
            "editor-text-command",
            json!({"argument":0,"marked":true}),
        );
        h.probe(
            true,
            "key-event",
            json!({"code":0,"flags":1u64<<20,"chars":"a"}),
        );
        h.probe(true, "key-event", json!({"code":12,"chars":"q"}));
        assert_eq!(
            h.probe(true, "command-read", json!({}))["value"]["arguments"][0],
            "q"
        );
        let typed = h.probe(true, "editor-layout", json!({}))["arguments"].clone();
        assert_eq!(typed[0]["undo"], true);
        h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        assert_eq!(
            h.probe(true, "editor-layout", json!({}))["arguments"],
            typed
        );
        assert_eq!(
            h.probe(true, "command-read", json!({}))["value"]["arguments"][0],
            "q"
        );
        h.probe(
            true,
            "command-input",
            json!({"key":"timeout","text":"17.5"}),
        );
        let draft = h.probe(true, "command-read", json!({}));
        h.probe(true, "add-argument", json!({}));
        assert_eq!(h.probe(true, "editor-layout", json!({}))["advanced"], true);
        h.probe(true, "remove-argument", json!({"index":2}));
        assert_eq!(h.probe(true, "editor-layout", json!({}))["advanced"], true);
        assert_eq!(draft, h.probe(true, "command-read", json!({})));
        h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        assert_eq!(draft, h.probe(true, "command-read", json!({})));
        assert_eq!(
            fs::read(h.profile.join("config.json")).unwrap(),
            before_config
        );
        h.probe(true, "editor-layout", json!({"advanced":"retain"}));
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        assert_eq!(
            fs::read(h.profile.join("config.json")).unwrap(),
            before_config
        );
        h.edit("script.fixture");
        let reset = h.probe(true, "editor-layout", json!({"advanced":"stale"}));
        assert_eq!(reset["advanced"], false);
        assert_eq!(reset["staleAdvancedRetired"], true);
        h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        h.probe(
            true,
            "command-argument",
            json!({"index":0,"text":"Saved\nliteral"}),
        );
        h.probe(true, "editor-layout", json!({"advanced":"toggle"}));
        h.probe(true, "save-editor", json!({}));
        h.wait(|_| h.probe(true, "read", json!({}))["formVisible"] == false);
        let saved: Value =
            serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        assert_eq!(saved["commands"][1]["arguments"][0], "Saved\nliteral");
        for id in ["url.fixture", "snippet.fixture"] {
            h.edit(id);
            assert_eq!(
                h.probe(true, "editor-layout", json!({}))["advanced"],
                Value::Null
            );
            h.probe(true, "cancel-editor", json!({}));
            h.settled();
        }
        println!("{theme}: script-only Advanced closed/open/value/cancel/save/retired-control pass; hidden native window");
    }
}

fn snippet_links(root: &Path) {
    let mut results = vec![];
    for theme in ["light", "dark"] {
        let h = Host::new(root, theme);
        let config = fs::read(h.profile.join("config.json")).unwrap();
        h.edit("snippet.fixture");
        let wrapped = format!("{}\nlast\n", "Long wrapped text ".repeat(80));
        h.probe(
            true,
            "command-input",
            json!({"key":"target","text":wrapped}),
        );
        let top = h.probe(
            true,
            "snippet-link",
            json!({"command":"scroll","location":0}),
        );
        assert_eq!(
            top["visibleMarkers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v[0].clone())
                .collect::<Vec<_>>(),
            vec![json!(1)]
        );
        let bottom = h.probe(
            true,
            "snippet-link",
            json!({"command":"scroll","location":wrapped.encode_utf16().count()}),
        );
        assert!(
            bottom["visibleMarkers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v[0] == 3),
            "{bottom}"
        );
        h.capture(true, &format!("snippet-link-ruler-scroll-{theme}"));
        h.probe(
            true,
            "command-input",
            json!({"key":"target","text":"😀 one\nsecond\n"}),
        );
        h.probe(
            true,
            "snippet-link",
            json!({"command":"scroll","location":0}),
        );
        let read = h.probe(true, "snippet-link", json!({"command":"read"}));
        fs::write(
            h.out.join(format!("snippet-ruler-{theme}.json")),
            serde_json::to_vec_pretty(&read).unwrap(),
        )
        .unwrap();
        assert_eq!(read["lineStarts"], json!([0, 7, 14]));
        assert_eq!(read["rulerVisible"], true);
        assert_eq!(
            read["visibleMarkers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v[0].clone())
                .collect::<Vec<_>>(),
            vec![json!(1), json!(2), json!(3)]
        );
        h.probe(
            true,
            "snippet-link",
            json!({"command":"selection","location":3,"length":3}),
        );
        let open = h.probe(true, "snippet-link", json!({"command":"open"}));
        assert_eq!(open["open"], true);
        assert_eq!(open["label"], "one");
        assert_eq!(open["saveEnabled"], false);
        assert_eq!(open["parentSaveHidden"], true);
        assert_eq!(open["parentSaveEnabled"], false);
        assert_eq!(open["parentFieldsLocked"], true);
        assert_eq!(open["parentTabsLocked"], true);
        assert_eq!(open["parentCancelEnabled"], false);
        assert_eq!(open["parentRecordEnabled"], false);
        h.capture(true, &format!("snippet-link-open-{theme}"));
        for label in ["", "one\ntwo"] {
            let invalid = h.probe(
                true,
                "snippet-link",
                json!({"command":"fill","label":label,"url":"https://example.com"}),
            );
            assert_eq!(invalid["saveEnabled"], false);
            assert_eq!(invalid["source"], "😀 one\nsecond\n");
        }
        let invalid = h.probe(
            true,
            "snippet-link",
            json!({"command":"fill","label":"Guide","url":"file:///private"}),
        );
        assert_eq!(invalid["saveEnabled"], false);
        h.probe(
            true,
            "key-event",
            json!({"code":36,"flags":1u64<<20,"chars":"\r"}),
        );
        let invalid = h.probe(true, "snippet-link", json!({"command":"read"}));
        assert_eq!(invalid["open"], true);
        assert!(!invalid["error"].as_str().unwrap().is_empty());
        let ready = h.probe(
            true,
            "snippet-link",
            json!({"command":"fill","label":"Guide","url":"https://example.com"}),
        );
        assert_eq!(ready["saveEnabled"], true);
        assert_eq!(ready["error"], "");
        // The hidden/disabled parent action cannot commit or finish this form;
        // the existing owner shortcut intentionally submits the active Link.
        h.probe(true, "save-editor", json!({}));
        assert_eq!(
            h.probe(true, "snippet-link", json!({"command":"read"}))["open"],
            true
        );
        h.probe(
            true,
            "key-event",
            json!({"code":36,"flags":1u64<<20,"chars":"\r"}),
        );
        let saved = h.probe(true, "snippet-link", json!({"command":"read"}));
        let expected = "😀 [Guide](https://example.com)\nsecond\n";
        assert_eq!(saved["source"], expected);
        assert_eq!(saved["open"], false);
        assert_eq!(saved["canUndo"], true);
        assert_eq!(saved["parentFieldsEnabled"], true);
        assert_eq!(saved["parentTabsEnabled"], true);
        assert_eq!(saved["parentSaveHidden"], false);
        assert_eq!(saved["parentSaveEnabled"], true);
        assert_eq!(saved["parentCancelEnabled"], true);
        assert_eq!(saved["parentRecordEnabled"], true);
        assert_eq!(
            saved["selection"],
            json!([expected.encode_utf16().count() - 8, 0])
        );
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(), config);
        assert_eq!(h.probe(true, "read", json!({}))["formVisible"], true);
        h.capture(true, &format!("snippet-link-saved-{theme}"));
        let undone = h.probe(true, "snippet-link", json!({"command":"undo"}));
        assert_eq!(undone["source"], "😀 one\nsecond\n");
        let redone = h.probe(true, "snippet-link", json!({"command":"redo"}));
        assert_eq!(redone["source"], expected);
        h.probe(
            true,
            "snippet-link",
            json!({"command":"selection","location":5,"length":0}),
        );
        let edit = h.probe(true, "snippet-link", json!({"command":"open"}));
        assert_eq!(edit["label"], "Guide");
        assert_eq!(edit["url"], "https://example.com");
        for (width, height) in [(680, 480), (980, 680)] {
            h.probe(
                true,
                "dashboard-size",
                json!({"width":width,"height":height}),
            );
            let resized = h.probe(true, "snippet-link", json!({"command":"read"}));
            assert_eq!(resized["open"], true);
            assert_eq!(resized["source"], expected);
            h.capture(true, &format!("snippet-link-form-{theme}-{width}-{height}"));
        }
        h.probe(
            true,
            "snippet-link",
            json!({"command":"fill","label":"Cancelled","url":"https://example.com/cancel"}),
        );
        h.probe(
            true,
            "key-event",
            json!({"code":53,"flags":0,"chars":"\u{1b}"}),
        );
        let cancelled = h.probe(true, "snippet-link", json!({"command":"read"}));
        assert_eq!(cancelled["open"], false);
        assert_eq!(cancelled["source"], expected);
        assert_eq!(cancelled["selection"], json!([5, 0]));
        assert_eq!(cancelled["parentFieldsEnabled"], true);
        assert_eq!(cancelled["parentTabsEnabled"], true);
        assert_eq!(cancelled["parentSaveHidden"], false);
        assert_eq!(cancelled["parentSaveEnabled"], true);
        assert_eq!(h.probe(true, "read", json!({}))["formVisible"], true);
        h.probe(true, "snippet-link", json!({"command":"open"}));
        h.probe(
            true,
            "snippet-link",
            json!({"command":"fill","label":"Updated","url":"https://example.com/updated"}),
        );
        h.probe(
            true,
            "key-event",
            json!({"code":36,"flags":1u64<<20,"chars":"\r"}),
        );
        let updated = h.probe(true, "snippet-link", json!({"command":"read"}));
        assert_eq!(updated["open"], false);
        assert!(updated["source"]
            .as_str()
            .unwrap()
            .contains("[Updated](https://example.com/updated)"));
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(), config);
        h.probe(
            true,
            "snippet-link",
            json!({"command":"selection","location":4,"length":0}),
        );
        h.probe(true, "snippet-link", json!({"command":"open"}));
        h.probe(
            true,
            "command-input",
            json!({"key":"target","text":"External owned edit"}),
        );
        let stale = h.probe(
            true,
            "snippet-link",
            json!({"command":"invoke-save","label":"Stale","url":"https://example.com/stale"}),
        );
        assert_eq!(stale["source"], "External owned edit");
        assert_eq!(stale["open"], true);
        assert_eq!(stale["saveEnabled"], false);
        assert!(!stale["error"].as_str().unwrap().is_empty());
        h.capture(true, &format!("snippet-link-stale-{theme}"));
        h.probe(true, "snippet-link", json!({"command":"cancel"}));
        h.probe(true, "snippet-link", json!({"command":"retain"}));
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(), config);
        h.edit("snippet.fixture");
        let stale = h.probe(true, "snippet-link", json!({"command":"stale"}));
        assert_eq!(stale["targetsRetired"], true);
        assert_eq!(stale["source"], "Original text");
        let read = h.probe(true, "snippet-link", json!({"command":"read"}));
        assert_eq!(read["open"], false);
        // Parent navigation is modal too; after explicit child cancellation,
        // the normal Back owner retires these controls before the next editor.
        h.probe(true, "snippet-link", json!({"command":"open"}));
        h.probe(true, "snippet-link", json!({"command":"retain"}));
        h.probe(true, "library-tab", json!({"tab":"general"}));
        h.settled();
        let still_open = h.probe(true, "snippet-link", json!({"command":"read"}));
        assert_eq!(still_open["open"], true);
        assert_eq!(still_open["parentTabsLocked"], true);
        assert_eq!(
            h.rpc(json!({"op":"status"}))["managementUI"]["tab"],
            "commands"
        );
        h.probe(true, "snippet-link", json!({"command":"cancel"}));
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        h.edit("snippet.fixture");
        let retired = h.probe(true, "snippet-link", json!({"command":"stale"}));
        assert_eq!(retired["targetsRetired"], true);
        assert_eq!(retired["source"], "Original text");
        assert_eq!(
            h.probe(true, "snippet-link", json!({"command":"read"}))["sourceVisible"],
            true
        );
        h.probe(true, "cancel-editor", json!({}));
        h.settled();
        // The normal owner Save still persists after the link form has finished.
        h.edit("snippet.fixture");
        h.probe(
            true,
            "snippet-link",
            json!({"command":"selection","location":0,"length":8}),
        );
        h.probe(true, "snippet-link", json!({"command":"open"}));
        h.probe(
            true,
            "snippet-link",
            json!({"command":"save","label":"Persisted","url":"https://example.com/save"}),
        );
        h.probe(true, "save-editor", json!({}));
        h.wait(|_| {
            let cfg: Value =
                serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
            cfg["commands"][0]["target"]
                .as_str()
                .is_some_and(|s| s.contains("[Persisted](https://example.com/save)"))
        });
        results.push(json!({"theme":theme,"nativeLinkCreateEditCancel":true,"logicalRuler":true,"undoRedo":true,"staleSourceRefused":true,"oldControlsRetired":true,"parentSaveAndCancel":true,"nativeRender":true}));
    }
    let mut copy = std::collections::BTreeSet::new();
    for entry in fs::read_dir(root.join("rust-native/target/ui-evidence")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if !name.starts_with("snippet-link-")
            || path.extension().and_then(|s| s.to_str()) != Some("json")
        {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for key in ["screenText", "accessibilityText"] {
            if let Some(lines) = value[key].as_array() {
                for line in lines {
                    if let Some(text) = line.as_str().filter(|s| !s.is_empty()) {
                        copy.insert(text.replace('\n', " "));
                    }
                }
            }
        }
    }
    fs::write(
        root.join("rust-native/target/ui-evidence/snippet-link-copy.txt"),
        copy.into_iter().collect::<Vec<_>>().join("\n"),
    )
    .unwrap();
    println!(
        "{}",
        json!({"ok":true,"snippetLinkCases":results,"physicalInput":false})
    );
}

fn snippet_hotkey(root:&Path) {
    let mut themes=0usize;
    for theme in ["light","dark"] {
        let h=Host::new(root,theme);
        let show=|query:&str| {
            h.rpc(json!({"op":"show","mode":"snippets","query":query}));
            h.wait(|status|status["nativeUI"]["selectedId"]=="snippet.fixture");
            h.settled();
        };
        show("Synthetic snippet");
        let read=h.probe(false,"read",json!({}));
        assert!(read["screenText"].as_array().unwrap().iter().any(|s|s=="Set Hotkey…"),"Visible snippet footer button missing: {read}");
        let footer=h.probe(false,"snippet-hotkey",json!({}));
        assert_eq!(footer["present"],true);assert_eq!(footer["hidden"],false);
        assert_eq!(footer["enabled"],true);assert_eq!(footer["title"],"Set Hotkey…");
        assert_eq!(footer["label"],"Set Hotkey…");assert_eq!(footer["role"],"AXButton");assert_eq!(footer["hit"],true);
        let before=fs::read(h.profile.join("config.json")).unwrap();
        let config:Value=serde_json::from_slice(&before).unwrap();
        let snippet=config["commands"].as_array().unwrap().iter().find(|c|c["id"]=="snippet.fixture").unwrap().clone();
        assert_eq!(footer["keyEquivalent"],"");
        for (visible,width) in [(648.,600.),(1440.,760.)] {
            let frame=h.probe(false,"palette-geometry",json!({"visible":[-20000.,-20000.,visible,900.],"clipboard":false}));
            assert_eq!(frame["frame"][2],width);
            let current=h.probe(false,"snippet-hotkey",json!({}));
            assert_eq!(current["identity"],footer["identity"]);assert_eq!(current["hit"],true);
            assert!(current["frame"][0].as_f64().unwrap()+current["frame"][2].as_f64().unwrap()<width);
            h.capture(false,&format!("snippet-hotkey-footer-{theme}-{}",width as u32));
        }
        h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        h.probe(false,"snippet-hotkey",json!({"operation":"click"}));
        h.wait(|_| {let v=h.probe(false,"read",json!({}));v["editorId"]=="snippet.fixture" && v["recording"]==true});
        h.capture(false,&format!("snippet-hotkey-record-{theme}"));
        h.probe(false,"key-event",json!({"code":53,"flags":0,"chars":""}));
        h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==false);
        h.settled();assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
        let fresh=h.probe(false,"snippet-hotkey",json!({}));assert_ne!(fresh["identity"],footer["identity"]);
        h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        show("fixture");
        assert_ne!(h.probe(false,"snippet-hotkey",json!({}))["identity"],fresh["identity"]);
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        let scoped=h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic snippet"}));
        h.wait(|status|status["nativeUI"]["selectedId"]=="snippet.fixture");h.settled();
        let global=h.probe(false,"snippet-hotkey",json!({}));
        assert_eq!(global["present"],true);assert_eq!(global["enabled"],true);assert_eq!(global["hit"],true);
        assert_eq!(global["frame"][0],24.);assert_ne!(global["identity"],scoped["identity"]);
        h.capture(false,&format!("snippet-hotkey-global-{theme}"));
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic quicklink"}));
        h.wait(|status|status["nativeUI"]["selectedId"]=="url.fixture");h.settled();
        assert_eq!(h.probe(false,"snippet-hotkey",json!({}))["present"],false);
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        show("Synthetic snippet");
        h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        h.probe(false,"hide-palette",json!({}));h.settled();
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        show("Synthetic snippet");
        let old=h.probe(false,"snippet-hotkey",json!({"operation":"retain"}));
        let mut changed:Value=serde_json::from_slice(&before).unwrap();
        changed["commands"][1]["title"]=json!("Changed own script title");
        fs::write(h.profile.join("config.json"),serde_json::to_vec(&changed).unwrap()).unwrap();
        h.rpc(json!({"op":"reload"}));h.settled();
        assert_ne!(h.probe(false,"snippet-hotkey",json!({}))["identity"],old["identity"]);
        h.probe(false,"snippet-hotkey",json!({"operation":"stale"}));h.settled();
        assert_eq!(h.probe(false,"read",json!({}))["formVisible"],false);
        h.probe(false,"snippet-hotkey",json!({"operation":"click"}));
        h.wait(|_|h.probe(false,"read",json!({}))["recording"]==true);
        h.probe(false,"key-event",json!({"code":101,"flags":(1u64<<18)|(1u64<<19),"chars":""}));
        h.wait(|_|{let value:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
            value["commands"].as_array().unwrap().iter().any(|c|c["id"]=="snippet.fixture" && c["hotkey"]=="ctrl+alt+f9")});
        h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==false);h.settled();
        let saved:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        let bound=saved["commands"].as_array().unwrap().iter().find(|c|c["id"]=="snippet.fixture").unwrap();
        for (key,value) in snippet.as_object().unwrap(){if key!="hotkey"{assert_eq!(&bound[key],value,"Snippet field changed: {key}");}}
        assert_eq!(bound["hotkey"],"ctrl+alt+f9");
        h.capture(false,&format!("snippet-hotkey-saved-{theme}"));
        h.rpc(json!({"op":"show","mode":"clipboard"}));h.settled();
        assert_eq!(h.probe(false,"snippet-hotkey",json!({}))["present"],false);
        themes+=1;
    }
    println!("{}",json!({"ok":true,"snippetHotkey":true,"themes":themes,"nativeSnippetFooter":true,"accessibleButton":true,"cancelPreservesConfig":true,"savePreservesSnippetFields":true,"retiredControlsRefused":true,"physicalInput":false}));
}

fn alias_ack_catalog_matches(model: u64, published: u64, first_advanced: u64, before: u64) -> bool {
    model == published && model >= first_advanced && model > before
}

// Publish only complete marker bytes, without replacing any existing final.
fn publish_alias_gap_release(home: &Path) -> std::io::Result<()> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let directory=fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(home)?;
    let staged=home.join("alias-main-gap-release.candidate");
    let mut file=fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&staged)?;
    let result=(|| {
        file.write_all(b"release\n")?;
        file.sync_all()?;
        let status=unsafe{libc::renameatx_np(directory.as_raw_fd(),c"alias-main-gap-release.candidate".as_ptr(),directory.as_raw_fd(),c"alias-main-gap-release".as_ptr(),libc::RENAME_EXCL)};
        if status!=0 {return Err(std::io::Error::last_os_error());}
        Ok(())
    })();
    if result.is_err() {let _=fs::remove_file(staged);}
    result
}

#[cfg(test)]
mod alias_release_tests {
    use super::*;
    #[test]
    fn latest_catalog_ack_accepts_later_publication_but_not_stale_or_missing_advance() {
        assert!(alias_ack_catalog_matches(13,13,12,9));
        assert!(alias_ack_catalog_matches(12,12,12,9));
        assert!(!alias_ack_catalog_matches(12,13,12,9));
        assert!(!alias_ack_catalog_matches(9,9,9,9));
        assert!(!alias_ack_catalog_matches(11,11,12,9));
    }
    #[test]
    fn atomic_release_is_complete_and_never_overwrites() {
        use std::os::unix::fs::MetadataExt;
        let parent=PathBuf::from(std::env::var_os("RUYCAST_ALIAS_RELEASE_TEST_PARENT").expect("Explicit private own test parent required"));
        let meta=fs::symlink_metadata(&parent).unwrap();
        assert!(meta.is_dir()&&meta.uid()==unsafe{libc::geteuid()}&&meta.mode()&0o077==0);
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let home=parent.join(format!("release-{}",std::process::id()));
        fs::create_dir(&home).unwrap();fs::set_permissions(&home,fs::Permissions::from_mode(0o700)).unwrap();
        let result=std::panic::catch_unwind(|| {
            let staged=home.join("alias-main-gap-release.candidate");let final_path=home.join("alias-main-gap-release");
            fs::write(&staged,b"partial").unwrap();
            assert!(publish_alias_gap_release(&home).is_err());
            assert!(!final_path.exists());assert_eq!(fs::read(&staged).unwrap(),b"partial");
            fs::remove_file(&staged).unwrap();
            publish_alias_gap_release(&home).unwrap();
            assert_eq!(fs::read(&final_path).unwrap(),b"release\n");assert!(!staged.exists());
            assert_eq!(fs::metadata(&final_path).unwrap().mode()&0o777,0o600);
            fs::write(&final_path,b"existing").unwrap();
            assert_eq!(publish_alias_gap_release(&home).unwrap_err().raw_os_error(),Some(libc::EEXIST));
            assert_eq!(fs::read(&final_path).unwrap(),b"existing");assert!(!staged.exists());
        });
        fs::remove_dir_all(home).unwrap();if let Err(error)=result {std::panic::resume_unwind(error);}
    }
}

fn controlled_alias_gap(root: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    let acknowledged=std::env::args().any(|arg|arg=="--controlled-alias-gap-ack");
    let h=Host::new(root,"light");
    let query="Discovered Action Binding";
    h.rpc(json!({"op":"show","mode":"commands","query":query}));
    h.settled();
    h.wait(|_| {
        let v=h.probe(false,"read",json!({}));
        v["selectedRow"]==0 && v["selectedId"]=="discovered.dev.fixture.action-binding" && v["responderSearch"]==true
    });
    let before=h.probe(false,"read",json!({}));
    assert_eq!(before["editorId"],Value::Null);
    let status_before=h.rpc(json!({"op":"status"}));
    assert_eq!(status_before["lastNamedRefusal"],Value::Null,"A fresh controlled run must not reuse an earlier refusal");
    let armed=h.probe(false,if acknowledged{"alias-main-gap-ack-arm"}else{"alias-main-gap-arm"},json!({}));
    // A normal repeated show creates a real newer M1, not a forced row/model.
    h.rpc(json!({"op":"show","mode":"commands","query":query}));
    let wait_file=|name:&str| {
        let deadline=Instant::now()+Duration::from_secs(4);
        loop {
            if let Ok(bytes)=fs::read(h.base.join(name)) {
                assert!(bytes.len()<=4096,"Owned gap scalar bound");
                if let Ok(value)=serde_json::from_slice::<Value>(&bytes) {break value;}
            }
            assert!(Instant::now()<deadline,"Owned gap witness missing: {name}");
            thread::sleep(Duration::from_millis(2));
        }
    };
    let entered=wait_file("alias-main-gap-entered.json");
    assert!(entered["modelRevision"].as_u64().unwrap()>armed["appliedRevision"].as_u64().unwrap());
    assert_eq!(entered["modelRevision"],entered["appliedRevision"]);
    let mut config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
    config["commands"][0]["title"]=json!("Unrelated owned catalog revision");
    let candidate=h.profile.join("alias-gap-config.candidate");
    let mut file=fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&candidate).unwrap();
    file.write_all(&serde_json::to_vec(&config).unwrap()).unwrap();file.sync_all().unwrap();drop(file);
    fs::rename(candidate,h.profile.join("config.json")).unwrap();
    // The real profile watcher/serialized reload must adopt and publish M2.
    // Its config revision is witnessed inside the held main callback; merely
    // seeing configurationBusy=false or a catalog tick would not prove this.
    let advanced=wait_file("alias-main-gap-advanced.json");
    assert!(advanced["publishedConfig"].as_u64().unwrap()>entered["modelConfig"].as_u64().unwrap());
    assert!(advanced["publishedCatalog"].as_u64().unwrap()>entered["modelCatalog"].as_u64().unwrap());
    assert_eq!(advanced["modelRevision"],entered["modelRevision"]);
    assert_eq!(advanced["appliedRevision"],entered["appliedRevision"]);
    assert_eq!(advanced["sameSelectedId"],true);assert_eq!(advanced["sameQuery"],true);
    let config_before_alias=fs::read(h.profile.join("config.json")).unwrap();
    let socket=h.profile.join("control.sock");
    let alias=thread::spawn(move || {
        let mut stream=UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        writeln!(stream,"{}",json!({"op":"native.test","action":"key-event","code":43,"flags":(1u64<<20)|(1u64<<19),"chars":","})).unwrap();
        let mut line=String::new();BufReader::new(stream).read_line(&mut line).unwrap();
        let value:Value=serde_json::from_str(&line).unwrap();assert_eq!(value["ok"],true,"{value}");value
    });
    let queued=wait_file("alias-main-gap-queued.json");
    assert_eq!(queued["modelRevision"],entered["modelRevision"]);
    publish_alias_gap_release(&h.base).unwrap();
    let event=alias.join().unwrap();
    let taken=wait_file("alias-main-gap-taken.json");
    assert_eq!(taken["requests"],1);assert_eq!(taken["aliasRequests"],1);
    assert_eq!(taken["beforeDispatch"]["modelRevision"],entered["modelRevision"]);
    assert_eq!(taken["beforeDispatch"]["appliedRevision"],entered["appliedRevision"]);
    let dispatch=wait_file("alias-main-gap-dispatch.json");
    assert_eq!(dispatch["acknowledged"],acknowledged);
    let status=if acknowledged {
        assert_eq!(dispatch["beforeInput"]["modelRevision"],dispatch["beforeInput"]["appliedRevision"]);
        assert_eq!(dispatch["beforeInput"]["modelRevision"],dispatch["beforeInput"]["publishedRevision"]);
        assert!(alias_ack_catalog_matches(dispatch["beforeInput"]["modelCatalog"].as_u64().unwrap(),dispatch["beforeInput"]["publishedCatalog"].as_u64().unwrap(),advanced["publishedCatalog"].as_u64().unwrap(),entered["modelCatalog"].as_u64().unwrap()));
        assert_eq!(dispatch["beforeInput"]["modelConfig"],advanced["publishedConfig"]);
        assert_eq!(dispatch["beforeInput"]["modelConfig"],1);
        assert_eq!(dispatch["beforeInput"]["sameSelectedId"],true);
        assert_eq!(dispatch["beforeInput"]["sameQuery"],true);
        h.wait(|_|h.probe(false,"read",json!({}))["editorId"]==before["selectedId"])
    }else{
        assert_eq!(dispatch["beforeInput"]["modelRevision"],entered["modelRevision"]);
        let status=h.wait(|s|s["lastNamedRefusal"].is_object());
        let refusal=&status["lastNamedRefusal"];
        assert_eq!(refusal["alias"],true);assert_eq!(refusal["sameSelectedId"],true);
        assert_eq!(refusal["targetCatalog"],entered["modelCatalog"]);
        assert_eq!(refusal["targetEpoch"],entered["modelEpoch"]);
        assert_eq!(refusal["currentEpoch"],entered["modelEpoch"]);
        assert!(refusal["currentCatalog"].as_u64().unwrap()>refusal["targetCatalog"].as_u64().unwrap());
        status
    };
    let after=h.probe(false,"read",json!({}));
    if acknowledged {
        fs::write(h.out.join("actions-controlled-gap-ack-observation.json"),serde_json::to_vec_pretty(&json!({"after":after,"event":event,"status":status,"configurationUnchangedByAlias":fs::read(h.profile.join("config.json")).unwrap()==config_before_alias})).unwrap()).unwrap();
    }
    for packet in [&before,&after] {for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(packet[flag],false);}}
    if acknowledged {
        assert_eq!(after["editorId"],before["selectedId"]);
        assert_eq!(status["lastNamedRefusal"],Value::Null);
        assert_eq!(after["formVisible"],true);
        assert_eq!(after["inlineVisible"],false);
        assert!(after["screenText"].as_array().unwrap().iter().any(|text|text=="Alias"));
    }else{assert_eq!(after["editorId"],Value::Null);}
    assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),config_before_alias,"Opening Alias must not write configuration");
    let ended=wait_file("alias-main-gap-ended.json");assert_eq!(ended["released"],true);
    let evidence=json!({"scope":"one controlled after-apply before-probes interleave only; not the cause of earlier uncontrolled failures","armed":armed,"entered":entered,"advanced":advanced,"queued":queued,"taken":taken,"ended":ended,"dispatch":dispatch,"acknowledged":acknowledged,"before":before,"event":event,"after":after,"status":status});
    fs::write(h.out.join(if acknowledged{"actions-controlled-gap-ack.json"}else{"actions-controlled-gap-stale.json"}),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("{}",json!({"ok":true,"controlledAliasGap":true,"aliasProbes":1,"acknowledged":acknowledged,"catalogRefusalObserved":!acknowledged,"sameIdAliasOpened":acknowledged,"configurationUnchangedByAlias":true,"visibleWindowOpened":false,"physicalInput":false,"productionFixApplied":false}));
}

fn actions_menu(root: &Path) {
    let mut themes=0;
    for theme in ["light","dark"] {
        let h=Host::with_clipboard(root,theme,None,true);
        let read=|| {
            let value=h.probe(false,"read",json!({}));
            for key in ["windowShown","windowKey","windowMain"] {assert_eq!(value[key],false,"{value}");}
            value
        };
        let key=|code:u16,flags:u64,chars:&str| {
            h.probe(false,"key-event",json!({"code":code,"flags":flags,"chars":chars}));
            h.settled();
        };
        h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic"}));h.settled();
        assert!(read()["rows"].as_u64().unwrap()>4);
        h.probe(false,"select",json!({"row":4}));h.settled();
        let original=read();let selected=original["selectedId"].clone();
        assert_eq!(original["selectedRow"],4);
        key(40,1<<20,"k");
        let menu=read();assert_eq!(menu["actionsVisible"],true);
        assert_eq!(menu["responderMenu"],true);assert_eq!(menu["actionSelection"],0);
        assert!(menu["actionLabels"].as_array().unwrap().iter().any(|v|v.as_str().unwrap().starts_with("Set Alias…")));
        assert!(menu["actionLabels"].as_array().unwrap().iter().any(|v|v.as_str().unwrap().starts_with("Set Hotkey…")));
        key(125,0,"");key(125,0,"");assert_eq!(read()["actionSelection"],2);
        key(126,0,"");assert_eq!(read()["actionSelection"],1);
        for _ in 0..20 {key(125,0,"");}
        let bottom=read();assert_eq!(bottom["actionSelection"].as_u64().unwrap() as usize,bottom["actionLabels"].as_array().unwrap().len()-1);
        for _ in 0..20 {key(126,0,"");}
        assert_eq!(read()["actionSelection"],0);
        assert_eq!(read()["selectedId"],selected);assert_eq!(read()["selectedRow"],4);
        // Existing burst invokes the real search field's current NSTextView
        // delegate, even while Actions owns the panel's first responder.
        h.probe(false,"burst",json!({"query":"Synthetic","count":1}));h.settled();
        let fallback=read();assert_eq!(fallback["actionSelection"],1);
        assert_eq!(fallback["selectedId"],selected);assert_eq!(fallback["selectedRow"],4);
        key(53,0,"");let escaped=read();assert_eq!(escaped["actionsVisible"],false);
        assert_eq!(escaped["responderSearch"],true);assert_eq!(escaped["input"],"Synthetic");
        assert_eq!(escaped["selectedId"],selected);assert_eq!(escaped["selectedRow"],4);
        assert_eq!(h.rpc(json!({"op":"status"}))["nativeUI"]["visible"],true);
        key(40,1<<20,"k");key(40,1<<20,"k");assert_eq!(read()["actionsVisible"],false);
        key(40,1<<20,"k");let menu=read();
        let edit=menu["actionLabels"].as_array().unwrap().iter().position(|v|v.as_str().unwrap().starts_with("Edit")).unwrap();
        for _ in 0..edit {key(125,0,"");}key(36,0,"\r");
        h.wait(|_|read()["editorId"]==selected);assert_eq!(read()["actionsVisible"],false);
        let edit_token=read()["editorToken"].clone();
        key(43,(1<<20)|(1<<19),",");assert_eq!(read()["editorToken"],edit_token);
        h.probe(false,"cancel-editor",json!({}));h.settled();
        key(40,1<<20,"k");let menu=read();
        let remove=menu["actionLabels"].as_array().unwrap().iter().position(|v|v.as_str().unwrap().starts_with("Remove")).unwrap();
        h.probe(false,"action-button",json!({"index":remove}));h.wait(|_|read()["formVisible"]==true);
        assert_eq!(read()["editorId"],selected);let before=fs::read(h.profile.join("config.json")).unwrap();
        let confirmation_token=read()["editorToken"].clone();
        key(43,(1<<20)|(1<<17),",");assert_eq!(read()["editorToken"],confirmation_token);
        assert_eq!(read()["recording"],false);
        h.probe(false,"query",json!({"query":"unrelated changed query"}));h.settled();
        // Unlike Swift's directly mutable pendingRemoval, the Rust modal
        // rejects search edits. A fresh ordinary show retires that modal.
        assert_eq!(read()["input"],"unrelated changed query");assert_eq!(read()["modelQuery"],"Synthetic");assert_eq!(read()["editorId"],selected);
        assert_eq!(read()["editorToken"],confirmation_token);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
        h.rpc(json!({"op":"show","mode":"commands","query":"unrelated changed query"}));h.settled();
        assert_eq!(read()["modelQuery"],"unrelated changed query");assert_eq!(read()["formVisible"],false);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
        h.rpc(json!({"op":"show","mode":"commands","query":"no-matches-owned-action"}));h.settled();
        assert_eq!(read()["rows"],0);key(43,(1<<20)|(1<<17),",");assert_eq!(read()["formVisible"],false);
        h.rpc(json!({"op":"show","mode":"commands","query":""}));h.settled();
        key(40,1<<20,"k");let menu=read();
        let create=menu["actionLabels"].as_array().unwrap().iter().position(|v|v=="New Snippet…").unwrap();
        h.probe(false,"action-button",json!({"index":create}));h.wait(|_|read()["formVisible"]==true);
        let draft=h.probe(false,"command-read",json!({}))["value"].clone();
        assert_eq!(draft["kind"],"snippet");assert_ne!(draft["id"],selected);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
        h.probe(false,"cancel-editor",json!({}));h.settled();
        h.rpc(json!({"op":"clipboard.clear"}));
        h.rpc(json!({"op":"clipboard.import","items":[{"id":"raycast-actions-parity","text":"Owned menu fixture","source":"fixture","date":ruycast_native::clipboard_history::now()}]}));
        h.rpc(json!({"op":"show","mode":"clipboard"}));h.settled();
        h.wait(|_|{let v=read();v["rows"]==1&&v["formVisible"]==false&&v["selectedId"].as_str().is_some_and(|id|id.starts_with("clipboard:"))});
        key(43,(1<<20)|(1<<19),",");let clipboard_binding=read();assert_eq!(clipboard_binding["formVisible"],false,"Clipboard binding must remain unavailable: {clipboard_binding}");
        key(40,1<<20,"k");assert_eq!(read()["actionLabels"].as_array().unwrap()[..3],json!(["Paste","Copy","Delete"]).as_array().unwrap()[..]);
        key(125,0,"");key(125,0,"");key(36,0,"\r");
        h.wait(|_|read()["rows"]==0);assert_eq!(read()["actionsVisible"],false);
        assert_eq!(serde_json::from_slice::<Value>(&fs::read(h.profile.join("clipboard.json")).unwrap()).unwrap(),json!([]));
        h.rpc(json!({"op":"show","mode":"commands","query":""}));
        h.wait(|_|read()["responderSearch"]==true);
        h.wait(|status|status["appDiscoveryPending"]==false);
        let config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        assert!(!config["commands"].as_array().unwrap().iter().any(|c|c["target"]=="dev.fixture.action-binding"));
        h.rpc(json!({"op":"show","mode":"commands","query":"Discovered Action Binding"}));
        h.wait(|_|read()["selectedId"]=="discovered.dev.fixture.action-binding");
        key(43,(1<<20)|(1<<17),",");
        h.wait(|_|read()["recording"]==true);
        assert_eq!(read()["editorId"],"discovered.dev.fixture.action-binding");
        fs::write(h.base.join("hold-config-adoption"),b"own").unwrap();
        h.probe(false,"key-event",json!({"code":20,"flags":(1u64<<20)|(1u64<<19),"chars":"3"}));
        let end=Instant::now()+Duration::from_secs(3);
        while !h.base.join("config-adoption-entered").is_file(){assert!(Instant::now()<end,"Own binding adoption barrier not entered");thread::sleep(Duration::from_millis(2));}
        assert_eq!(h.rpc(json!({"op":"status"}))["configurationUpdating"],true);
        let busy_token=read()["editorToken"].clone();let published=fs::read(h.profile.join("config.json")).unwrap();
        h.probe(false,"key-event",json!({"code":43,"flags":(1u64<<20)|(1u64<<19),"chars":","}));
        assert_eq!(read()["editorToken"],busy_token);assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),published);
        fs::write(h.base.join("config-adoption-release"),b"own").unwrap();
        h.wait(|_|read()["formVisible"]==false);
        for name in ["hold-config-adoption","config-adoption-entered","config-adoption-release"] {fs::remove_file(h.base.join(name)).unwrap();}
        let config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        let discovered=config["commands"].as_array().unwrap().iter().find(|c|c["id"]=="discovered.dev.fixture.action-binding").unwrap();
        assert_eq!(discovered["kind"],"app");assert_eq!(discovered["target"],"dev.fixture.action-binding");
        assert_eq!(discovered["title"],"Discovered Action Binding");assert_eq!(discovered["hotkey"],"alt+cmd+3");
        // The original binding check records palette shortcuts themselves;
        // successful generic recorder input does not prove these exact keys.
        key(43,(1<<20)|(1<<17),",");h.wait(|_|read()["recording"]==true);
        let before_repeat=fs::read(h.profile.join("config.json")).unwrap();
        h.probe(false,"key-event",json!({"code":43,"flags":(1u64<<20)|(1u64<<17),"chars":",","repeat":true}));h.settled();
        assert_eq!(read()["recording"],true);assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before_repeat);
        key(53,0,"");h.wait(|_|read()["formVisible"]==false);
        assert_eq!(read()["recording"],false);assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before_repeat);
        for (code,flags,chars,value) in [(40,1u64<<20,"k","cmd+k"),(43,(1u64<<20)|(1u64<<17),",","shift+cmd+,"),(36,1u64<<20,"\r","cmd+return")] {
            key(40,1<<20,"k");
            h.wait(|_|read()["actionsVisible"]==true);h.settled();
            key(43,(1<<20)|(1<<17),",");
            h.wait(|_|read()["recording"]==true);key(code,flags,chars);
            h.wait(|_|read()["formVisible"]==false);
            let saved:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
            let row=saved["commands"].as_array().unwrap().iter().find(|c|c["id"]=="discovered.dev.fixture.action-binding").unwrap();
            assert_eq!(row["target"],"dev.fixture.action-binding");assert_eq!(row["hotkey"],value);
            assert_eq!(read()["actionsVisible"],false);assert_eq!(read()["recording"],false);
        }
        key(43,(1<<20)|(1<<17),",");h.wait(|_|read()["recording"]==true);
        let before_conflict=fs::read(h.profile.join("config.json")).unwrap();
        key(49,(1<<18)|(1<<19)," ");
        h.wait(|_|{let v=read();v["formVisible"]==true&&v["recording"]==false&&v["screenText"].as_array().unwrap().iter().any(|s|s.as_str().unwrap().to_lowercase().contains("duplicate hotkey"))});
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before_conflict);
        h.probe(false,"record",json!({}));h.wait(|_|read()["recording"]==true);key(51,0,"");
        h.wait(|_|read()["formVisible"]==false);
        let saved:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        let row=saved["commands"].as_array().unwrap().iter().find(|c|c["id"]=="discovered.dev.fixture.action-binding").unwrap();
        assert!(row.get("hotkey").is_none());assert_eq!(row["target"],"dev.fixture.action-binding");
        key(43,(1<<20)|(1<<19),",");h.wait(|_|read()["formVisible"]==true);
        h.probe(false,"alias-input",json!({"text":""}));
        // alias-type refuses unless the real alias field owns its NSTextView.
        h.probe(false,"alias-type",json!({"text":"owned-action-a, owned-action-b"}));
        key(36,0,"\r");h.wait(|_|read()["formVisible"]==false);
        let aliased:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        let row=aliased["commands"].as_array().unwrap().iter().find(|c|c["id"]=="discovered.dev.fixture.action-binding").unwrap();
        assert_eq!(row["aliases"],json!(["owned-action-a","owned-action-b"]));
        assert_eq!(row["target"],"dev.fixture.action-binding");assert!(row.get("hotkey").is_none());
        let before_opener=read();
        let event_opener=h.probe(false,"key-event",json!({"code":43,"flags":(1u64<<20)|(1u64<<19),"chars":","}));
        h.settled();let after_opener=read();
        fs::write(h.out.join(format!("actions-alias-opener-{theme}.json")),serde_json::to_vec_pretty(&json!({"theme":theme,"before":before_opener,"event":event_opener,"after":after_opener,"status":h.rpc(json!({"op":"status"}))})).unwrap()).unwrap();
        h.wait(|_|read()["formVisible"]==true);
        let before_alias_cancel=fs::read(h.profile.join("config.json")).unwrap();
        h.probe(false,"alias-input",json!({"text":"discard-owned-alias"}));key(53,0,"");
        h.wait(|_|read()["formVisible"]==false);assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before_alias_cancel);
        themes+=1;
    }
    println!("{}",json!({"ok":true,"actionsMenu":true,"themes":themes,"nativeMenuFirstResponder":true,"fieldEditorFallback":true,"selectedRowFourPreserved":true,"clampedMenuArrows":true,"returnEditsOriginal":true,"escapeAndModeRestoreSearch":true,"commandKTogglesClosed":true,"removalQueryBoundaryPreservesIdentity":true,"newSnippetDraft":true,"clipboardDelete":true,"discoveredAppMaterializedWithTarget":true,"recordedPaletteShortcutPayloads":true,"repeatCancelConflictClear":true,"physicalInput":false}));
}

fn feedback_actions(root: &Path) {
    let mut themes = 0;
    for theme in ["light", "dark"] {
        let h = Host::with_clipboard(root, theme, None, true);
        let key = |code: u16, flags: u64, chars: &str| {
            h.probe(false, "key-event", json!({"code":code,"flags":flags,"chars":chars}));
            thread::sleep(Duration::from_millis(25));
            h.settled();
        };
        h.rpc(json!({"op":"show","mode":"commands","query":""}));
        h.wait(|_|h.probe(false,"read",json!({}))["input"]=="");
        key(0,0,"Synthetic");
        h.wait(|_|h.probe(false,"read",json!({}))["input"]=="Synthetic");
        let selected=h.probe(false,"read",json!({}));
        assert!(selected["rows"].as_u64().unwrap()>=4);
        let id=selected["selectedId"].clone();
        let row=selected["selectedRow"].clone();
        key(40,1<<20,"k");
        let menu=h.probe(false,"read",json!({}));
        assert_eq!(menu["actionsVisible"],true);
        assert_eq!(menu["responderMenu"],true);
        assert_eq!(menu["actionSelection"],0);
        for control in menu["menuControls"].as_array().unwrap() {assert_eq!(control["hit"],true);assert_eq!(control["inside"],true);}
        h.capture(false,&format!("actions-first-{theme}"));
        key(125,0,"");key(125,0,"");
        assert_eq!(h.probe(false,"read",json!({}))["actionSelection"],2);
        h.capture(false,&format!("actions-third-{theme}"));
        key(126,0,"");
        assert_eq!(h.probe(false,"read",json!({}))["actionSelection"],1);
        for _ in 0..20 {key(125,0,"");}
        let bottom=h.probe(false,"read",json!({}));
        assert_eq!(bottom["actionSelection"].as_u64().unwrap() as usize,bottom["actionLabels"].as_array().unwrap().len()-1);
        for _ in 0..20 {key(126,0,"");}
        assert_eq!(h.probe(false,"read",json!({}))["actionSelection"],0);
        // Existing probe invokes the actual search-field delegate callback.
        h.probe(false,"burst",json!({"query":"Synthetic","count":1}));
        h.settled();
        let fallback=h.probe(false,"read",json!({}));
        assert_eq!(fallback["actionSelection"],1);
        assert_eq!(fallback["selectedId"],id);
        assert_eq!(fallback["selectedRow"],row);
        key(53,0,"");
        let closed=h.probe(false,"read",json!({}));
        assert_eq!(closed["actionsVisible"],false);
        assert_eq!(closed["responderSearch"],true);
        assert_eq!(closed["input"],"Synthetic");
        assert_eq!(closed["selectedId"],id);
        assert_eq!(closed["selectedRow"],row);
        assert_eq!(h.rpc(json!({"op":"status"}))["nativeUI"]["visible"],true);
        key(40,1<<20,"k");key(40,1<<20,"k");
        let toggled=h.probe(false,"read",json!({}));
        assert_eq!(toggled["actionsVisible"],false);
        assert_eq!(toggled["responderSearch"],true);
        key(40,1<<20,"k");
        let menu=h.probe(false,"read",json!({}));
        let edit=menu["actionLabels"].as_array().unwrap().iter().position(|v|v.as_str().is_some_and(|s|s.starts_with("Edit"))).unwrap();
        for _ in 0..edit {key(125,0,"");}
        key(36,0,"\r");
        h.wait(|_|h.probe(false,"read",json!({}))["editorId"]==id);
        assert_eq!(h.probe(false,"read",json!({}))["recording"],false);
        h.probe(false,"cancel-editor",json!({}));h.settled();

        // A saved snippet has eleven command actions; provider actions use their own context.
        key(0,1<<20,"a");key(51,0,"");
        h.wait(|_|h.probe(false,"read",json!({}))["selectedId"]=="snippet.fixture");
        key(40,1<<20,"k");
        let largest=h.probe(false,"read",json!({}));
        assert_eq!(largest["actionLabels"],json!(["Insert Snippet","Copy Template","Edit","Set Alias…   ⌥ ⌘ ,","Set Hotkey…   ⇧ ⌘ ,","Disable","Remove","Settings","New Snippet…","New Quicklink…","New Shell Command…"]));
        for _ in 0..20 {key(125,0,"");}
        assert_eq!(h.probe(false,"read",json!({}))["actionSelection"],10);
        for width in [600.,760.] {
            h.probe(false,"dashboard-size",json!({"width":width,"height":490.}));
            let controls=h.probe(false,"read",json!({}))["menuControls"].clone();
            assert_eq!(controls.as_array().unwrap().len(),11);
            for control in controls.as_array().unwrap() {assert_eq!(control["hit"],true);assert_eq!(control["inside"],true);}
            h.capture(false,&format!("actions-largest-{theme}-{width}"));
        }
        // Exercise Refresh only in an eligible empty MainTabs context, using owned providers.
        // Never invoke consent; this no longer claims a thirteen-control positive case.
        key(53,0,"");
        h.rpc(json!({"op":"show","mode":"commands","query":"owned-feedback-no-match-7d09"}));
        h.wait(|_| {let view=h.probe(false,"read",json!({}));view["rows"]==0&&view["selectedId"].is_null()&&view["input"]=="owned-feedback-no-match-7d09"});
        key(40,1<<20,"k");
        let empty=h.probe(false,"read",json!({}));
        assert_eq!(empty["actionLabels"],json!(["Settings","New Snippet…","New Quicklink…","New Shell Command…","Refresh","Allow Safari Access…"]));
        assert_eq!(empty["menuControls"].as_array().unwrap().len(),6);
        for control in empty["menuControls"].as_array().unwrap() {assert_eq!(control["hit"],true);assert_eq!(control["inside"],true);}
        h.capture(false,&format!("actions-empty-provider-{theme}"));
        h.probe(false,"action-button",json!({"index":4}));
        h.wait(|_|h.probe(false,"read",json!({}))["actionsVisible"]==false);
        h.rpc(json!({"op":"clipboard.clear"}));
        let now=ruycast_native::clipboard_history::now();
        h.rpc(json!({"op":"clipboard.import","items":[{"id":"raycast-owned-actions","text":"Own actions fixture","source":"fixture","date":now}]}));
        h.rpc(json!({"op":"show","mode":"clipboard"}));
        h.wait(|_|h.probe(false,"read",json!({}))["rows"]==1);
        key(40,1<<20,"k");
        assert_eq!(h.probe(false,"read",json!({}))["actionLabels"],json!(["Paste","Copy","Delete","Settings"]));
        key(125,0,"");key(125,0,"");key(36,0,"\r");
        h.wait(|_|h.probe(false,"read",json!({}))["rows"]==0);
        assert_eq!(h.probe(false,"read",json!({}))["actionsVisible"],false);
        h.rpc(json!({"op":"show","mode":"commands","query":""}));
        h.wait(|_|h.probe(false,"read",json!({}))["responderSearch"]==true);

        // Both production creation entry points consume the genuine own named-board seed.
        for settings in [false,true] {
            let raw=" \nhttps://example.test/copied?q=1#part \n";
            h.probe(false,"clipboard-fixture",json!({"payload":{"Text":raw},"own":true}));
            let board_before=h.probe(false,"clipboard-fixture",json!({"creationBoard":"read"}));
            assert_eq!(board_before["text"],raw);
            h.rpc(json!({"op":"show","mode":if settings {"settings"} else {"commands"},"query":"no-matches-zz"}));h.settled();
            let before=fs::read(h.profile.join("config.json")).unwrap();
            if settings {h.probe(true,"library-add",json!({"kind":"url"}));}
            else {
                key(40,1<<20,"k");
                let menu=h.probe(false,"read",json!({}));
                let index=menu["actionLabels"].as_array().unwrap().iter().position(|v|v=="New Quicklink…").unwrap();
                h.probe(false,"action-button",json!({"index":index}));
            }
            h.wait(|_|h.probe(settings,"read",json!({}))["formVisible"]==true);
            let draft=h.probe(settings,"command-read",json!({}))["value"].clone();
            assert_eq!(draft["kind"],"url");
            assert_eq!(draft["target"],"https://example.test/copied?q=1#part");
            assert!(!["snippet.fixture","url.fixture"].contains(&draft["id"].as_str().unwrap()));
            assert!(h.base.join("creation-prefill-read").is_file());
            assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
            assert_eq!(h.probe(false,"clipboard-fixture",json!({"creationBoard":"read"})),board_before);
            for width in [600.,760.] {
                h.probe(settings,"dashboard-size",json!({"width":width,"height":490.}));
                h.capture(settings,&format!("quicklink-prefill-{}-{theme}-{width}",if settings {"settings"} else {"palette"}));
            }
            h.probe(settings,"command-input",json!({"key":"title","text":"Edited own quicklink"}));
            h.probe(settings,"command-input",json!({"key":"target","text":"https://example.test/edited"}));
            h.probe(false,"clipboard-fixture",json!({"payload":{"Text":"https://example.test/later"},"own":true}));
            let board_later=h.probe(false,"clipboard-fixture",json!({"creationBoard":"read"}));
            assert_eq!(board_later["name"],board_before["name"]);
            assert!(board_later["count"].as_i64().unwrap()>board_before["count"].as_i64().unwrap());
            assert_eq!(board_later["text"],"https://example.test/later");
            h.settled();
            assert_eq!(h.probe(settings,"command-read",json!({}))["value"]["target"],"https://example.test/edited");
            h.probe(settings,"cancel-editor",json!({}));h.settled();
            assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
            for (kind,label,expected) in [("snippet","New Snippet…",""),("script","New Shell Command…","/usr/bin/true")] {
                if settings {h.probe(true,"library-add",json!({"kind":kind}));}
                else {key(40,1<<20,"k");let m=h.probe(false,"read",json!({}));let index=m["actionLabels"].as_array().unwrap().iter().position(|v|v==label).unwrap();h.probe(false,"action-button",json!({"index":index}));}
                h.wait(|_|h.probe(settings,"read",json!({}))["formVisible"]==true);
                assert_eq!(h.probe(settings,"command-read",json!({}))["value"]["target"],expected);
                h.probe(settings,"cancel-editor",json!({}));h.settled();
                assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
            }
            assert_eq!(h.probe(false,"clipboard-fixture",json!({"creationBoard":"read"})),board_later);
            for operation in ["clear","nontext"] {
                let fallback_board=h.probe(false,"clipboard-fixture",json!({"creationBoard":operation}));
                assert_eq!(fallback_board["name"],board_before["name"]);
                assert!(fallback_board["text"].is_null());
                if operation=="nontext" {assert_eq!(fallback_board["nontextData"],json!([0,1,2]));assert_eq!(fallback_board["types"],json!(["org.example.quicklink-fixture"]));}
                else {assert_eq!(fallback_board["types"],json!([]));}
                if settings {h.probe(true,"library-add",json!({"kind":"url"}));}
                else {key(40,1<<20,"k");let m=h.probe(false,"read",json!({}));let index=m["actionLabels"].as_array().unwrap().iter().position(|v|v=="New Quicklink…").unwrap();h.probe(false,"action-button",json!({"index":index}));}
                h.wait(|_|h.probe(settings,"read",json!({}))["formVisible"]==true);
                assert_eq!(h.probe(settings,"command-read",json!({}))["value"]["target"],"https://");
                assert_eq!(h.probe(false,"clipboard-fixture",json!({"creationBoard":"read"})),fallback_board);
                h.probe(settings,"cancel-editor",json!({}));h.settled();
                assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),before);
            }
            // New draft is separately confirmed through the same actual entry.
            if settings {h.probe(true,"library-add",json!({"kind":"url"}));}
            else {key(40,1<<20,"k");let m=h.probe(false,"read",json!({}));let i=m["actionLabels"].as_array().unwrap().iter().position(|v|v=="New Quicklink…").unwrap();h.probe(false,"action-button",json!({"index":i}));}
            h.wait(|_|h.probe(settings,"read",json!({}))["formVisible"]==true);
            let new_id=h.probe(settings,"command-read",json!({}))["value"]["id"].clone();
            h.probe(settings,"command-input",json!({"key":"title","text":"Saved own quicklink"}));
            h.probe(settings,"command-input",json!({"key":"target","text":"https://example.test/edited"}));
            h.probe(settings,"save-editor",json!({}));
            h.wait(|_|h.probe(settings,"read",json!({}))["formVisible"]==false);
            let config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
            assert_eq!(config["commands"].as_array().unwrap().iter().find(|c|c["id"]==new_id).unwrap()["target"],"https://example.test/edited");
        }
        use_either_command(&h, theme);
        closed_creation(&h, theme);
        themes+=1;
    }
    println!("{}",json!({"ok":true,"feedbackActions":true,"themes":themes,"nativeMenuResponder":true,"fieldEditorFallback":true,"multirowClamps":true,"allOriginalMenuControlsReachable":true,"emptyMainTabsRefresh":true,"clipboardDeleteAndModeFocus":true,"searchFocusRestored":true,"namedBoardQuicklinkBothEntries":true,"explicitCancelSave":true,"useEitherCommand":true,"eitherCommandSettingsBothTargets":true,"eitherCommandDraftCancelSave":true,"eitherCommandStaleAndBusyRefused":true,"sameBoardChangeCountAndRawPreserved":true,"closedCreationBothKindsWithoutRepeatPresentation":true,"laterBoardMutationPreservesEditedDraft":true,"emptyAndNontextBothEntries":true,"physicalInput":false}));
}

fn use_either_command(h: &Host, theme: &str) {
    let config = || -> Value { serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap() };
    let replace = |key: &str, value: &str| {
        let mut next=config();next[key]=json!(value);next["separateCommandKeys"]=json!(true);
        fs::write(h.profile.join("config.json"),serde_json::to_vec(&next).unwrap()).unwrap();
        h.rpc(json!({"op":"reload"}));h.settled();
    };
    let general = || {
        h.rpc(json!({"op":"show","mode":"settings"}));h.settled();
        h.probe(true,"library-tab",json!({"tab":"general"}));
        h.wait(|_|{let v=h.probe(true,"settings-read",json!({}));v["surface"]["tab"]=="general" && v["surface"]["busy"]==false});
    };
    let menu = |record: &str, held: bool, invoke: bool, force: bool| {
        h.probe(true,"recorder-menu",json!({"record":record,"held":held,"invoke":invoke,"force":force}))
    };
    let enabled = |value: &Value, expected: bool| {
        assert_eq!(value["menuItems"],json!([{"title":"Use Either Command","enabled":expected}]),"{value}");
    };
    for (record,key,letter,index) in [("launcher","launcherHotkey","l",0),("clipboard","clipboardHotkey","u",1)] {
        for side in ["left-cmd","right-cmd"] {
            eprintln!("either positive {theme}/{record}/{side}");
            let sided=format!("alt+{side}+{letter}");let either=format!("alt+cmd+{letter}");
            replace(key,&sided);general();
            h.wait(|_|h.probe(true,"settings-read",json!({}))["surface"]["configuration"][key]==sided);
            let before=config();let opened=menu(record,false,false,false);
            enabled(&opened,true);assert_eq!(opened["value"],sided);
            menu(record,true,true,false);
            h.wait(|_|config()[key]==either && h.probe(true,"settings-read",json!({}))["surface"]["busy"]==false);
            let mut expected=before;expected[key]=json!(either);assert_eq!(config(),expected);
            assert_eq!(menu(record,true,false,false)["value"],either);
            let bytes=fs::read(h.profile.join("config.json")).unwrap();
            enabled(&menu(record,true,true,true),false);h.settled();
            assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
            assert_eq!(menu(record,false,false,false)["menuItems"],json!([]));
            h.capture(true,&format!("either-command-{record}-{side}-{theme}"));
        }
        // Held real NSMenu action owners must reject all retired contexts.
        for retirement in ["configuration","navigation","hide","recording"] {
            eprintln!("either retirement {theme}/{record}/{retirement}");
            let sided=format!("alt+left-cmd+{letter}");replace(key,&sided);general();
            enabled(&menu(record,false,false,false),true);
            match retirement {
                "configuration"=>{replace(key,&format!("alt+right-cmd+{letter}"));},
                "navigation"=>{h.probe(true,"library-tab",json!({"tab":"advanced"}));h.wait(|_|h.probe(true,"settings-read",json!({}))["surface"]["tab"]=="advanced");},
                "hide"=>{h.probe(true,"window-close",json!({}));h.wait(|s|s["managementUI"]["visible"]==false);},
                "recording"=>{h.probe(true,"settings-record",json!({"index":index}));h.wait(|_|h.probe(true,"settings-read",json!({}))["recording"][index]==true);},
                _=>unreachable!(),
            }
            let bytes=fs::read(h.profile.join("config.json")).unwrap();
            enabled(&menu(record,true,true,true),false);h.settled();
            assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes,"{record}/{retirement}");
            if retirement=="recording" {
                assert_eq!(menu(record,false,false,false)["menuItems"],json!([]));
                h.probe(true,"key-event",json!({"code":53,"flags":0,"chars":""}));
                h.wait(|_|h.probe(true,"settings-read",json!({}))["recording"][index]==false);
                assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
            }
        }
    }
    // Hold a real unrelated Clipboard preference write after disk publication
    // and before owner adoption. The retained unused Launcher action cannot write.
    eprintln!("either busy {theme}");
    replace("launcherHotkey","alt+left-cmd+l");replace("clipboardHotkey","alt+right-cmd+u");general();
    enabled(&menu("launcher",false,false,false),true);
    let before=config();
    for name in ["config-adoption-entered","config-adoption-release"] {let _=fs::remove_file(h.base.join(name));}
    fs::write(h.base.join("hold-config-adoption"),b"own").unwrap();
    h.probe(true,"settings-record",json!({"index":1}));
    h.wait(|_|h.probe(true,"settings-read",json!({}))["recording"][1]==true);
    h.probe(true,"key-event",json!({"code":11,"flags":(1u64<<19)|(1u64<<20),"chars":"b"}));
    let end=Instant::now()+Duration::from_secs(3);
    while !h.base.join("config-adoption-entered").is_file() {assert!(Instant::now()<end,"Own adoption barrier not entered");thread::sleep(Duration::from_millis(2));}
    assert_eq!(h.rpc(json!({"op":"status"}))["configurationUpdating"],true);
    let mut expected=before;expected["clipboardHotkey"]=json!("alt+cmd+b");
    assert_eq!(config(),expected);
    let bytes=fs::read(h.profile.join("config.json")).unwrap();
    enabled(&menu("launcher",true,true,true),false);
    assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
    fs::write(h.base.join("config-adoption-release"),b"own").unwrap();
    h.wait(|s|s["configurationUpdating"]==false);
    assert_eq!(config(),expected);
    for name in ["hold-config-adoption","config-adoption-entered","config-adoption-release"] {fs::remove_file(h.base.join(name)).unwrap();}

    // Full command Edit changes only the visible draft until explicit Save.
    for save in [false,true] {
        eprintln!("either draft {theme}/save={save}");
        let mut original=config();
        let snippet=original["commands"].as_array_mut().unwrap().iter_mut().find(|c|c["id"]=="snippet.fixture").unwrap();
        snippet["hotkey"]=json!("alt+right-cmd+f12");
        fs::write(h.profile.join("config.json"),serde_json::to_vec(&original).unwrap()).unwrap();h.rpc(json!({"op":"reload"}));h.settled();
        h.probe(true,"library-tab",json!({"tab":"commands"}));h.settled();
        h.edit("snippet.fixture");h.probe(true,"command-scroll",json!({"y":2000.}));
        let record=h.probe(true,"editor-record",json!({}));assert_eq!(record["hit"],true);assert_eq!(record["visibleCenter"],true);
        let bytes=fs::read(h.profile.join("config.json")).unwrap();
        enabled(&menu("inline",false,false,false),true);
        menu("inline",true,true,false);
        assert_eq!(h.probe(true,"command-read",json!({}))["value"]["hotkey"],"alt+cmd+f12");
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
        enabled(&menu("inline",true,true,true),false);
        h.capture(true,&format!("either-command-inline-{}-{theme}",if save {"save"} else {"cancel"}));
        if save {
            h.probe(true,"save-editor",json!({}));h.wait(|_|h.probe(true,"read",json!({}))["formVisible"]==false);
            original["commands"].as_array_mut().unwrap().iter_mut().find(|c|c["id"]=="snippet.fixture").unwrap()["hotkey"]=json!("alt+cmd+f12");
            assert_eq!(config(),original);
        } else {
            h.probe(true,"cancel-editor",json!({}));h.settled();assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
            enabled(&menu("inline",true,true,true),false);assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
        }
    }
    // The existing Hotkey editor stays open after genuine focus loss ends its
    // automatic recording; the same native action then uses its existing writer.
    eprintln!("either hotkey {theme}");
    let mut before=config();before["commands"].as_array_mut().unwrap().iter_mut().find(|c|c["id"]=="snippet.fixture").unwrap()["hotkey"]=json!("alt+left-cmd+f12");
    fs::write(h.profile.join("config.json"),serde_json::to_vec(&before).unwrap()).unwrap();h.rpc(json!({"op":"reload"}));h.settled();
    h.rpc(json!({"op":"show","mode":"snippets","query":"Synthetic snippet"}));h.settled();
    h.probe(false,"snippet-hotkey",json!({"operation":"click"}));
    h.wait(|_|h.probe(false,"read",json!({}))["recording"]==true);
    h.probe(false,"unfocus",json!({}));h.wait(|_|h.probe(false,"read",json!({}))["recording"]==false);
    let opened=h.probe(false,"recorder-menu",json!({"record":"inline"}));enabled(&opened,true);
    h.probe(false,"recorder-menu",json!({"record":"inline","held":true,"invoke":true}));
    h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==false);
    before["commands"].as_array_mut().unwrap().iter_mut().find(|c|c["id"]=="snippet.fixture").unwrap()["hotkey"]=json!("alt+cmd+f12");
    assert_eq!(config(),before);
    let bytes=fs::read(h.profile.join("config.json")).unwrap();
    enabled(&h.probe(false,"recorder-menu",json!({"record":"inline","held":true,"invoke":true,"force":true})),false);
    assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),bytes);
}

fn polish_bindings(root: &Path) {
    let mut captures = 0;
    for theme in ["light", "dark"] {
        let h = Host::new(root, theme);
        let mut config: Value =
            serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        config["commands"] = json!([
            {"id":"fixture.greet","title":"Greeting","kind":"snippet","target":"Hello","aliases":["hello","greeting","salutation"],"hotkey":"cmd+shift+g","keyword":";greetsnippet"},
            {"id":"fixture.long","title":"Longer bindings","kind":"snippet","target":"Example","aliases":["first-alias","second-alias","third-alias","fourth-alias","final-alias"],"keyword":";longsnippet"},
            {"id":"fixture.none","title":"Unbound snippet","kind":"snippet","target":"Example"}
        ]);
        fs::write(
            h.profile.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        h.rpc(json!({"op":"reload"}));
        h.settled();
        let bytes = fs::read(h.profile.join("config.json")).unwrap();
        for width in [600, 760] {
            h.rpc(json!({"op":"show","mode":"snippets","query":""}));
            h.settled();
            let geometry = h.probe(
                false,
                "palette-geometry",
                json!({"visible":[-20000.,-20000.,width as f64 + 48.,900.],"clipboard":false}),
            );
            assert_eq!(geometry["frame"][2], width as f64);
            h.settled();
            let view = h.probe(false, "read", json!({}));
            assert_eq!(view["rows"], 3);
            assert_eq!(view["selectedId"], "fixture.greet");
            let text = view["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            for required in [
                "Greeting",
                "Longer bindings",
                "Unbound snippet",
                "Aliases",
                ";greetsnippet",
                ";longsnippet",
                "hello",
                "greeting",
                "salutation",
                "first-alias",
                "second-alias",
                "third-alias",
                "fourth-alias",
                "final-alias",
            ] {
                assert!(
                    text.contains(required),
                    "missing original snippet binding {required}: {view}"
                );
            }
            assert!(
                !text.contains("Keyword"),
                "technical Keyword label leaked: {view}"
            );
            let footer = h.probe(false, "snippet-hotkey", json!({}));
            assert_eq!(footer["present"], true);
            assert_eq!(footer["enabled"], true);
            assert_eq!(footer["title"], "Set Hotkey…");
            h.capture(false, &format!("polish-bindings-{theme}-{width}"));
            let png = fs::read(h.out.join(format!("polish-bindings-{theme}-{width}.png"))).unwrap();
            let pixels = u32::from_be_bytes(png[16..20].try_into().unwrap());
            assert!(pixels == width || pixels == width * 2, "Wrong snippet binding capture width: {pixels}");
            captures += 1;
            h.rpc(json!({"op":"show","mode":"snippets","query":"salutation"}));
            h.settled();
            let found = h.probe(false, "read", json!({}));
            assert_eq!(found["rows"], 1);
            assert_eq!(found["selectedId"], "fixture.greet");
            h.rpc(json!({"op":"show","mode":"snippets","query":"longsnippet"}));
            h.settled();
            assert_eq!(
                h.probe(false, "read", json!({}))["selectedId"],
                "fixture.long"
            );
            h.rpc(json!({"op":"show","mode":"snippets","query":""}));
            h.settled();
            h.probe(
                false,
                "key-event",
                json!({"code":125,"chars":"\u{f701}","flags":0}),
            );
            h.wait(|s| s["nativeUI"]["selectedId"] == "fixture.long");
            h.settled();
            assert_eq!(h.probe(false, "read", json!({}))["selectedRow"], 1);
            assert_eq!(fs::read(h.profile.join("config.json")).unwrap(), bytes);
        }
    }
    println!(
        "{}",
        json!({"ok":true,"polishBindings":true,"themes":2,"captures":captures,"aliasAndKeywordSearch":true,"arrowSelection":true,"physicalInput":false,"visibleWindowOpened":false})
    );
}

// Original Polish and filled Feedback routes, using only the existing owned host.
fn polish_original(root: &Path) {
    let fixture=ui_validation_date_fixture(root);
    assert!(fixture.is_file());
    let mut captures=0usize;
    let mut receipts=Vec::new();
    for theme in ["system","light","dark","graphite"] {
        let mut h=Host::new(root,theme);
        let marker=h.base.join("polish-runs.txt");
        let alias_marker=h.base.join("alias-runs.txt");
        let mut config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
        config["clipboard"]["enabled"]=json!(false);
        config["commands"]=json!([
            {"id":"app.fixture","title":"Synthetic app","kind":"app","target":"dev.fixture.synthetic","aliases":[]},
            {"id":"snippet.fixture","title":"Synthetic snippet","kind":"snippet","target":"Hello","aliases":[]},
            {"id":"script.fixture","title":"Inline Input","kind":"script","target":fixture,"arguments":["fake","wait",alias_marker,"{input}"],"inputPrompt":"Text","aliases":["echo-inline"]},
            {"id":"capture.quick-note","title":"Quick Note","kind":"script","target":fixture,"arguments":["fake","wait",marker,"{\"ok\":true}","{input}"],"inputPrompt":"Note","aliases":["polish-note"]},
            {"id":"capture.quick-task","title":"Quick Task","kind":"script","target":fixture,"arguments":["fake","write",marker,"{\"ok\":true}","{input}"],"inputPrompt":"Task","aliases":["polish-task"]},
            {"id":"window.fixture","title":"Left Half","kind":"window","target":"left-half","aliases":[]}
        ]);
        fs::write(h.profile.join("config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
        h.rpc(json!({"op":"reload"}));h.settled();
        h.rpc(json!({"op":"clipboard.pause","paused":true}));
        assert_eq!(h.rpc(json!({"op":"status"}))["clipboardPaused"],true);
        let key=|code:u16,flags:u64,chars:&str| {h.probe(false,"key-event",json!({"code":code,"flags":flags,"chars":chars}));};
        let text=|v:&Value|v["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
        let capture=|settings:bool,name:&str,present:&[&str],absent:&[&str]| {
            h.settled();let view=h.probe(settings,"read",json!({}));let actual=text(&view);
            for expected in present {assert!(actual.contains(expected),"Missing {expected} in {name}: {actual}");}
            for rejected in absent {assert!(!actual.contains(rejected),"Unexpected {rejected} in {name}: {actual}");}
            for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(view[flag],false);}
            h.capture(settings,name);
            fs::write(h.out.join(format!("{name}-read.json")),serde_json::to_vec_pretty(&view).unwrap()).unwrap();
            view
        };
        h.rpc(json!({"op":"show","mode":"commands","query":""}));h.settled();
        h.probe(false,"dashboard-size",json!({"width":760,"height":490}));
        capture(false,&format!("polish-launcher-{theme}"),&[],&["Create"]);captures+=1;
        h.rpc(json!({"op":"show","mode":"settings"}));h.probe(true,"library-tab",json!({"tab":"general"}));
        h.wait(|s|s["managementUI"]["tab"]=="general");
        h.probe(true,"dashboard-size",json!({"width":980,"height":680}));
        capture(true,&format!("polish-general-{theme}"),&["Theme","Menu Bar Icon"],&[]);captures+=1;
        if ["light","dark"].contains(&theme) {
            h.probe(true,"window-close",json!({}));
            for width in [600,760] {
                for (id,title,action) in [("app.fixture","Synthetic app","Open"),("snippet.fixture","Synthetic snippet","Insert Snippet"),("script.fixture","Inline Input","Run"),("capture.quick-note","Quick Note","Save"),("window.fixture","Left Half","Run")] {
                    h.rpc(json!({"op":"show","mode":"commands","query":title}));h.wait(|s|s["nativeUI"]["selectedId"]==id);
                    h.probe(false,"dashboard-size",json!({"width":width,"height":490}));
                    capture(false,&format!("polish-context-{id}-{theme}-{width}"),&[title,action],&[]);captures+=1;
                }
            }
            // Filled multiline Note and Task, before any submission or IME.
            for (id,query) in [("capture.quick-note","polish-note"),("capture.quick-task","polish-task")] {
                h.rpc(json!({"op":"show","mode":"commands","query":query}));h.settled();key(36,0,"\r");
                h.wait(|_|h.probe(false,"read",json!({}))["runInputInline"]==true);
                key(36,1<<20,"\r");h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==true);
                h.probe(false,"run-input-type",json!({"text":"First line"}));key(36,1<<17,"\r");
                h.probe(false,"run-input-type",json!({"text":"Second line Ω"}));h.settled();
                let filled=h.probe(false,"run-input-read",json!({}));
                assert_eq!(filled["text"],"First line\nSecond line Ω");assert_eq!(filled["focused"],true);
                assert_eq!(h.rpc(json!({"op":"status"}))["scriptsRunning"],0);assert!(!marker.exists());
                h.probe(false,"dashboard-size",json!({"width":760,"height":490}));
                capture(false,&format!("polish-filled-{id}-{theme}"),&["Save","⇧↵ New line"],&[]);captures+=1;
                h.probe(false,"cancel-editor",json!({}));h.settled();
            }
            h.rpc(json!({"op":"show","mode":"commands","query":"polish-note"}));h.settled();key(36,0,"\r");
            h.wait(|_|h.probe(false,"read",json!({}))["runInputInline"]==true);
            h.probe(false,"run-inline-type",json!({"text":"Example note"}));
            for width in [600,760] {
                h.probe(false,"dashboard-size",json!({"width":width,"height":490}));
                capture(false,&format!("polish-capture-idle-{theme}-{width}"),&["Save"],&["Open","Run ↵"]);captures+=1;
            }
            key(36,0,"\r");h.wait(|s|s["scriptsRunning"]==1&&fs::read(&marker).is_ok_and(|b|b==b"run\n"));
            key(36,0,"\r");
            assert_eq!(h.probe(false,"read",json!({}))["input"],"Example note","Busy Return changed capture input");
            for width in [600,760] {
                assert_eq!(h.rpc(json!({"op":"status"}))["scriptsRunning"],1,"Busy capture already completed");
                h.probe(false,"dashboard-size",json!({"width":width,"height":490}));
                capture(false,&format!("polish-capture-busy-{theme}-{width}"),&["Save"],&["Open"]);captures+=1;
                assert_eq!(h.probe(false,"read",json!({}))["input"],"Example note","Busy render changed capture input");
            }
            h.wait(|s|s["scriptsRunning"]==0);assert_eq!(fs::read(&marker).unwrap(),b"run\n");
            assert_eq!(h.rpc(json!({"op":"usage"}))["capture.quick-note"],1);
            key(53,0,"");
            for width in [600,760] {
                h.rpc(json!({"op":"show","mode":"commands","query":"___no-result___"}));h.settled();
                assert!(h.probe(false,"read",json!({}))["selectedId"].is_null());
                h.probe(false,"dashboard-size",json!({"width":width,"height":490}));
                capture(false,&format!("polish-empty-{theme}-{width}"),&[],&["Open"]);captures+=1;
            }
            h.rpc(json!({"op":"show","mode":"commands","query":"echo-inline Café & spaces"}));
            h.wait(|_|h.probe(false,"read",json!({}))["runInputInline"]==true);
            let inline=h.probe(false,"read",json!({}));assert_eq!(inline["selectedRow"],0);assert_eq!(inline["formVisible"],false);
            assert_eq!(h.probe(false,"run-inline-read",json!({}))["text"],"Café & spaces");
            h.probe(false,"dashboard-size",json!({"width":760,"height":490}));
            capture(false,&format!("polish-inline-query-{theme}"),&["Inline Input","Café & spaces","Run"],&[]);captures+=1;
            key(36,0,"\r");h.wait(|s|s["scriptsRunning"]==1&&fs::read(&alias_marker).is_ok_and(|b|b==b"run\n"));key(36,0,"\r");
            h.wait(|s|s["scriptsRunning"]==0);assert_eq!(fs::read(&alias_marker).unwrap(),b"run\n");
            assert_eq!(h.rpc(json!({"op":"usage"}))["script.fixture"],1);
            key(53,0,"");h.settled();assert_eq!(h.probe(false,"read",json!({}))["modelQuery"],"echo-inline");
            h.rpc(json!({"op":"show","mode":"commands","query":"echo-inline"}));h.settled();key(36,0,"\r");
            h.wait(|_|h.probe(false,"read",json!({}))["runInputInline"]==true);
            h.probe(false,"run-inline-type",json!({"text":"First line"}));let token=h.probe(false,"read",json!({}))["editorToken"].clone();
            key(36,1<<20,"\r");h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==true);
            assert_eq!(h.probe(false,"run-input-read",json!({}))["text"],"First line");
            assert_eq!(h.probe(false,"read",json!({}))["editorToken"],token);
            h.probe(false,"cancel-editor",json!({}));h.settled();assert_eq!(fs::read(&alias_marker).unwrap(),b"run\n");
            let long="Readable clipboard text. ".repeat(80);
            h.rpc(json!({"op":"clipboard.import","items":[{"id":"raycast-polish-owned-text","text":long,"source":"Fixture","date":ruycast_native::clipboard_history::now()}]}));
            h.rpc(json!({"op":"show","mode":"clipboard"}));h.wait(|s|s["clipboardCount"]==1);
            for (width,height,label) in [(1080,700,"large"),(600,400,"small")] {
                let f=h.probe(false,"palette-geometry",json!({"visible":[-20000.,-20000.,width+48,height+48],"clipboard":true}));
                assert_eq!(f["frame"][2].as_f64().unwrap(),f64::from(width));assert_eq!(f["frame"][3].as_f64().unwrap(),f64::from(height));
                capture(false,&format!("polish-clipboard-{label}-{theme}"),&["Readable clipboard text"],&[]);captures+=1;
            }
            h.rpc(json!({"op":"clipboard.clear"}));h.wait(|s|s["clipboardCount"]==0);
            h.probe(false,"palette-geometry",json!({"visible":[-20000.,-20000.,1128.,748.],"clipboard":true}));
            capture(false,&format!("polish-clipboard-cleared-{theme}"),&["Select an item"],&["Readable clipboard text"]);captures+=1;
            let normal=h.probe(false,"palette-geometry",json!({"visible":[-1920.,0.,1920.,1080.],"clipboard":false}));
            let clip=h.probe(false,"palette-geometry",json!({"visible":[-1920.,0.,1920.,1080.],"clipboard":true}));
            let n=normal["frame"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect::<Vec<_>>();
            let c=clip["frame"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect::<Vec<_>>();
            assert!(c[2]>n[2]&&c[3]>n[3]&&c[0]>=-1920.&&c[1]>=0.&&c[0]+c[2]<=0.&&c[1]+c[3]<=1080.);
            let logs=h.rpc(json!({"op":"logs"}));
            for id in ["capture.quick-note","script.fixture"] {
                let entries=logs.as_array().unwrap().iter().filter(|entry|entry["commandID"]==id).collect::<Vec<_>>();
                assert_eq!(entries.len(),2,"Expected one actual start/finish for {id}: {logs}");
                assert_eq!(entries.iter().filter(|entry|entry["phase"]=="command.start").count(),1);
                assert_eq!(entries.iter().filter(|entry|entry["phase"]=="command.finish"&&entry["success"]==true).count(),1);
                assert_eq!(entries[0]["traceID"],entries[1]["traceID"]);
                assert!(entries.iter().all(|entry|entry["source"]=="launcher"));
            }
            receipts.push(json!({"theme":theme,"captureMarker":fs::read_to_string(&marker).unwrap(),"aliasMarker":fs::read_to_string(&alias_marker).unwrap(),"usage":h.rpc(json!({"op":"usage"})),"logs":h.rpc(json!({"op":"logs"})),"normalFrame":normal,"clipboardFrame":clip}));
        }
        assert_eq!(serde_json::from_slice::<Value>(&fs::read(h.profile.join("config.json")).unwrap()).unwrap(),config);
        if ["light","dark"].contains(&theme) {
            // The saved binding edit owns only aliases/hotkey, never the
            // simultaneously updated external definition (Feedback115).
            for (label,alias) in [("Set Alias",true),("Set Hotkey",false)] {
                let current:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
                let current_title=current["commands"].as_array().unwrap().iter().find(|command|command["id"]=="app.fixture").unwrap()["title"].clone();
                h.rpc(json!({"op":"show","mode":"commands","query":current_title}));
                h.wait(|status|status["nativeUI"]["selectedId"]=="app.fixture");
                key(40,1<<20,"k");h.wait(|_|h.probe(false,"read",json!({}))["actionsVisible"]==true);
                let menu=h.probe(false,"read",json!({}));
                let index=menu["actionLabels"].as_array().unwrap().iter().position(|value|value.as_str().is_some_and(|title|title.starts_with(label))).unwrap();
                let opened=h.probe(false,"action-button",json!({"index":index}));
                h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==true);
                let before=h.probe(false,"read",json!({}));assert_eq!(before["editorId"],"app.fixture");
                if alias {h.probe(false,"alias-input",json!({"text":"preserved-external-alias"}));}
                else {h.wait(|_|h.probe(false,"read",json!({}))["recording"]==true);}
                let mut external:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
                let command=external["commands"].as_array_mut().unwrap().iter_mut().find(|command|command["id"]=="app.fixture").unwrap();
                command["title"]=json!(if alias {"External Alias App"}else{"External Hotkey App"});
                command["target"]=json!(if alias {"dev.fixture.external-one"}else{"dev.fixture.external-two"});
                command["arguments"]=json!(["literal Ω", "$(never-execute)", if alias {"alias"}else{"hotkey"}]);
                let unrelated=external["commands"].as_array().unwrap().iter().filter(|command|command["id"]!="app.fixture").cloned().collect::<Vec<_>>();
                fs::write(h.profile.join("config.json"),serde_json::to_vec(&external).unwrap()).unwrap();
                h.wait(|status|status["configurationUpdating"]!=true && h.rpc(json!({"op":"commands"})).as_array().unwrap().iter().any(|command|command["id"]=="app.fixture"&&command["target"]==external["commands"][0]["target"]));
                let reloaded=h.rpc(json!({"op":"reload"}));h.settled();
                let adopted=h.probe(false,"read",json!({}));assert_eq!(adopted["editorId"],"app.fixture");
                let packet=h.probe(false,"key-event",if alias {json!({"code":36,"flags":0,"chars":"\r"})}else{json!({"code":100,"flags":(1u64<<18)|(1u64<<19),"chars":""})});
                h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==false);
                let saved:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
                let mut expected=external.clone();let command=expected["commands"].as_array_mut().unwrap().iter_mut().find(|command|command["id"]=="app.fixture").unwrap();
                if alias {command["aliases"]=json!(["preserved-external-alias"]);}else{command["hotkey"]=json!("ctrl+alt+f8");}
                assert_eq!(saved,expected,"Binding overwrote the external command definition or unrelated configuration");
                assert_eq!(saved["commands"].as_array().unwrap().iter().filter(|command|command["id"]!="app.fixture").cloned().collect::<Vec<_>>(),unrelated);
                let commands=h.rpc(json!({"op":"commands"}));
                assert!(commands.as_array().unwrap().iter().any(|command|command["id"]=="app.fixture"&&command["target"]==expected["commands"][0]["target"]));
                assert_eq!(fs::read(&marker).unwrap(),b"run\n");assert_eq!(fs::read(&alias_marker).unwrap(),b"run\n");
                fs::write(h.out.join(format!("polish-binding-{}-{theme}.json",if alias {"alias"}else{"hotkey"})),serde_json::to_vec_pretty(&json!({"menu":menu,"opened":opened,"before":before,"external":external,"reload":reloaded,"adopted":adopted,"savePacket":packet,"saved":saved,"commands":commands})).unwrap()).unwrap();
            }
        }
        let quit=h.rpc(json!({"op":"quit"}));let end=Instant::now()+Duration::from_secs(3);
        loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<end,"Polish host failed normal quit");thread::sleep(Duration::from_millis(5));}
        assert!(!h.profile.join("control.sock").exists());
        fs::write(h.out.join(format!("polish-quit-{theme}.json")),serde_json::to_vec_pretty(&quit).unwrap()).unwrap();
    }
    fs::write(ui_validation_output(root).unwrap_or_else(||root.join("rust-native/target/ui-evidence")).join("polish-original-receipts.json"),serde_json::to_vec_pretty(&receipts).unwrap()).unwrap();
    println!("{}",json!({"ok":true,"polishOriginal":true,"themes":4,"contextThemes":2,"captures":captures,"filledCaptureFrames":4,"doubleSubmitOnce":true,"clipboardClear":true,"geometry":true,"normalQuit":true,"healthIconQualified":false,"physicalInput":false,"visibleWindowOpened":false}));
}

fn closed_creation(h: &Host, theme: &str) {
    let mut configuration:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
    configuration["commands"].as_array_mut().unwrap().extend([
        json!({"id":"fixture.create-snippet","title":"Owned Create Snippet","kind":"builtin","target":"create-snippet","aliases":[]}),
        json!({"id":"fixture.create-quicklink","title":"Owned Create Quicklink","kind":"builtin","target":"create-quicklink","aliases":[]}),
    ]);
    fs::write(h.profile.join("config.json"),serde_json::to_vec(&configuration).unwrap()).unwrap();
    h.wait(|status|status["configurationUpdating"]!=true&&h.rpc(json!({"op":"commands"})).as_array().unwrap().iter().any(|command|command["id"]=="fixture.create-quicklink"));
    h.settled();
    let baseline=fs::read(h.profile.join("config.json")).unwrap();
    let stage_total=|snapshot:&Value| {
        let stage=&snapshot["stages"]["open.ready"];
        stage["count"].as_u64().unwrap()+stage["cancelled"].as_u64().unwrap()
    };
    for (id,kind,target) in [("fixture.create-snippet","snippet",""),("fixture.create-quicklink","url","https://")] {
        h.probe(true,"window-close",json!({}));
        h.probe(false,"hide-palette",json!({}));
        h.wait(|status|status["nativeUI"]["visible"]==false&&status["managementUI"]["visible"]==false);
        let reset=h.rpc(json!({"op":"performance.reset"}));
        assert_eq!(reset["activeIntervals"],0);assert_eq!(stage_total(&reset),0);
        let first_packet=h.rpc(json!({"op":"execute","id":id}));
        assert_eq!(first_packet["state"],"opened");
        h.wait(|status|status["nativeUI"]["visible"]==true&&status["nativeUI"]["creationPreparing"]!=true&&h.probe(false,"read",json!({}))["formVisible"]==true);
        h.settled();
        let first=h.probe(false,"read",json!({}));
        let first_command=h.probe(false,"command-read",json!({}));
        assert_eq!(first_command["value"]["kind"],kind);assert_eq!(first_command["value"]["target"],target);
        let first_id=first_command["value"]["id"].as_str().unwrap();
        assert!(!configuration["commands"].as_array().unwrap().iter().any(|command|command["id"]==first_id));
        h.wait(|_|{let value=h.rpc(json!({"op":"performance"}));value["activeIntervals"]==0&&stage_total(&value)==1});
        let first_trace=h.rpc(json!({"op":"performance"}));
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
        let repeat_packet=h.rpc(json!({"op":"execute","id":id}));
        assert_eq!(repeat_packet["state"],"opened");
        h.wait(|status|status["nativeUI"]["visible"]==true&&status["nativeUI"]["creationPreparing"]!=true&&{let value=h.probe(false,"read",json!({}));value["formVisible"]==true&&value["editorToken"]!=first["editorToken"]});
        h.settled();
        let repeated=h.probe(false,"read",json!({}));
        let repeated_command=h.probe(false,"command-read",json!({}));
        assert_ne!(repeated_command["value"]["id"],first_command["value"]["id"]);
        assert_eq!(repeated_command["value"]["kind"],kind);assert_eq!(repeated_command["value"]["target"],target);
        assert!(!configuration["commands"].as_array().unwrap().iter().any(|command|command["id"]==repeated_command["value"]["id"]));
        let repeated_trace=h.rpc(json!({"op":"performance"}));
        assert_eq!(repeated_trace["activeIntervals"],0);
        assert_eq!(stage_total(&repeated_trace),stage_total(&first_trace),"Repeated New re-presented the logical palette");
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
        let status=h.rpc(json!({"op":"status"}));
        for field in ["scriptsRunning","openRequestsRunning","snippetRequestsRunning"] {assert_eq!(status[field],0);}
        for view in [&first,&repeated] {for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(view[flag],false);}}
        // Public builtin dispatch itself legitimately records two CLI actions;
        // neither newly-created draft nor an external command is executed.
        assert_eq!(h.rpc(json!({"op":"usage"}))[id],2);
        let receipt=json!({"theme":theme,"kind":kind,"closedReset":reset,"firstPacket":first_packet,"first":first,"firstCommand":first_command,"firstTrace":first_trace,"repeatPacket":repeat_packet,"repeated":repeated,"repeatedCommand":repeated_command,"repeatedTrace":repeated_trace,"status":status,"configurationUnchanged":true});
        fs::write(h.out.join(format!("feedback-closed-new-{kind}-{theme}-receipt.json")),serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        h.capture(false,&format!("feedback-closed-new-{kind}-{theme}"));
        h.probe(false,"cancel-editor",json!({}));h.settled();
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
    }
}

fn interaction_trace(root: &Path) {
    let mut h=Host::new(root,"dark");
    let host_path=ui_validation_file(root,"HOST").unwrap_or_else(||PathBuf::from(std::env::var_os("RUYCAST_INTERACTION_TRACE_HOST").expect("Qualified Trace host required")));
    let host_bytes=fs::read(&host_path).unwrap();
    let host_sha=format!("{:x}",Sha256::digest(&host_bytes));
    // Diagnostic identity only. Use the existing builder's bounded thin-Mach-O
    // LC_UUID algorithm; no new loader, process query or execution authority.
    let word=|offset:usize|u32::from_le_bytes(host_bytes.get(offset..offset.checked_add(4).unwrap()).unwrap().try_into().unwrap());
    assert_eq!(word(0),0xfeed_facf);assert_eq!(word(12),2);
    let end=32usize.checked_add(word(20) as usize).unwrap();assert!(end<=host_bytes.len());
    let count=word(16) as usize;assert!(count<=(end-32)/8);
    let mut offset=32usize;let mut uuid=None;
    for _ in 0..count {
        let kind=word(offset);let size=word(offset+4) as usize;
        let next=offset.checked_add(size).unwrap();assert!(size>=8&&size.is_multiple_of(8)&&next<=end);
        if kind==0x1b {
            assert_eq!(size,24);assert!(uuid.is_none());
            let hex=host_bytes[offset+8..next].iter().map(|byte|format!("{byte:02X}")).collect::<String>();
            uuid=Some(format!("{}-{}-{}-{}-{}",&hex[..8],&hex[8..12],&hex[12..16],&hex[16..20],&hex[20..]));
        }
        offset=next;
    }
    assert_eq!(offset,end);let host_uuid=uuid.expect("Qualified image has no UUID");
    let host_pid=h.child.id();
    let started=SystemTime::now();
    let started_ms=u64::try_from(started.duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
    let started_utc=chrono::DateTime::<chrono::Utc>::from(started).to_rfc3339_opts(chrono::SecondsFormat::Millis,true);
    fs::write(h.out.join("interaction-trace-host-start.json"),serde_json::to_vec_pretty(&json!({"pid":host_pid,"path":host_path,"sha256":host_sha,"imageUUID":host_uuid,"startedUnixMS":started_ms,"startedUTC":started_utc})).unwrap()).unwrap();
    let mut config:Value=serde_json::from_slice(&fs::read(h.profile.join("config.json")).unwrap()).unwrap();
    config["clipboard"]["enabled"]=json!(false);
    config["commands"]=json!((0..500).map(|index|json!({"id":format!("fixture.{index}"),"title":format!("Example {index}"),"kind":"url","target":format!("https://example.invalid/{index}"),"aliases":[]})).collect::<Vec<_>>());
    fs::write(h.profile.join("config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
    h.wait(|status|status["configurationUpdating"]!=true&&h.rpc(json!({"op":"commands"})).as_array().unwrap().len()==500);
    h.settled();h.rpc(json!({"op":"clipboard.pause","paused":true}));
    h.rpc(json!({"op":"show","mode":"commands","query":""}));
    h.wait(|status|status["nativeUI"]["visible"]==true);
    h.settled();h.probe(false,"dashboard-size",json!({"width":760,"height":490}));
    let baseline=fs::read(h.profile.join("config.json")).unwrap();
    let initial=h.probe(false,"read",json!({}));
    for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(initial[flag],false);}
    let reset=h.rpc(json!({"op":"performance.reset"}));
    assert_eq!(reset["schema"],2);assert_eq!(reset["sampleLimitPerStage"],256);assert_eq!(reset["activeIntervals"],0);
    for stage in reset["stages"].as_object().unwrap().values() {assert_eq!(stage["count"],0);assert_eq!(stage["cancelled"],0);assert_eq!(stage["retainedSamples"],0);}
    let terms=["E","Ex","Example 4","Example 42",""];
    let mut cycles=Vec::new();
    for index in 0..100u64 {
        let query=terms[index as usize%terms.len()];
        let before=h.rpc(json!({"op":"performance"}));
        let draws_before=before["stages"]["host.drawCallback"]["count"].as_u64().unwrap();
        // This existing probe changes the retained NSTextField and dispatches
        // its real NSControlTextDidChange notification to the Controller.
        h.probe(false,"query",json!({"query":query}));
        let deadline=Instant::now()+Duration::from_secs(1);
        let reconciled=loop {
            let snapshot=h.rpc(json!({"op":"performance"}));
            let count=snapshot["stages"]["input.reconcile"]["count"].as_u64().unwrap();
            assert!(count<=index+1,"More than one field reconciliation in cycle {index}: {snapshot}");
            if count==index+1 {break snapshot;}
            assert!(Instant::now()<deadline,"Field did not reconcile before capture in cycle {index}: {snapshot}");
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(reconciled["stages"]["input.model"]["count"],index+1);
        let view=h.probe(false,"read",json!({}));
        assert_eq!(view["input"],query);assert_eq!(view["modelQuery"],query);
        for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(view[flag],false);}
        // No offscreen capture occurs until the public metric above proves
        // this cycle's genuine field reconciliation has already completed.
        h.capture(false,&format!("interaction-trace-cycle-{index:03}"));
        let after=h.rpc(json!({"op":"performance"}));
        assert_eq!(after["stages"]["input.reconcile"]["count"],index+1);
        assert_eq!(after["stages"]["input.model"]["count"],index+1);
        assert!(after["stages"]["host.drawCallback"]["count"].as_u64().unwrap()>draws_before,"Actual host capture did not draw in cycle {index}: {after}");
        cycles.push(json!({"cycle":index,"before":before,"reconciledBeforeCapture":reconciled,"afterCapture":after,"nativeQueryMatched":true,"hidden":true}));
    }
    let measured=h.rpc(json!({"op":"performance"}));
    assert_eq!(measured["stages"]["input.model"]["count"],100);
    assert_eq!(measured["stages"]["input.reconcile"]["count"],100);
    assert!(measured["stages"]["search.model"]["count"].as_u64().unwrap()>=100);
    assert!(measured["stages"]["host.drawCallback"]["count"].as_u64().unwrap()>=100);
    assert_eq!(measured["activeIntervals"],0);
    for stage in measured["stages"].as_object().unwrap().values() {
        assert!(stage["retainedSamples"].as_u64().unwrap()<=256);
        for field in ["p50MS","p95MS","maxMS"] {
            if !stage[field].is_null() {let value=stage[field].as_f64().unwrap();assert!(value.is_finite()&&value>=0.);}
        }
    }
    let encoded=serde_json::to_string(&measured).unwrap();
    for rejected in ["Example","fixture.","example.invalid",h.base.to_str().unwrap()] {assert!(!encoded.contains(rejected),"Trace retained input or command payload: {encoded}");}
    // A retained field callback after closing must retire its pending search
    // interval rather than manufacture a successful visible reconciliation.
    h.probe(false,"hide-palette",json!({}));h.wait(|status|status["nativeUI"]["visible"]==false);
    h.rpc(json!({"op":"performance.reset"}));
    h.probe(false,"query",json!({"query":"OWN_CLOSED_QUERY_SENTINEL"}));
    let deadline=Instant::now()+Duration::from_secs(1);
    let cancelled=loop {
        let value=h.rpc(json!({"op":"performance"}));
        if value["stages"]["input.reconcile"]["cancelled"]==1 {break value;}
        assert!(Instant::now()<deadline,"Closed field did not retire its trace: {value}");thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(cancelled["activeIntervals"],0);assert_eq!(cancelled["stages"]["input.reconcile"]["count"],0);
    assert!(!serde_json::to_string(&cancelled).unwrap().contains("OWN_CLOSED_QUERY_SENTINEL"));
    let cleared=h.rpc(json!({"op":"performance.reset"}));
    assert_eq!(cleared["activeIntervals"],0);
    for stage in cleared["stages"].as_object().unwrap().values() {for field in ["count","cancelled","retainedSamples"] {assert_eq!(stage[field],0);}for field in ["p50MS","p95MS","maxMS"] {assert!(stage[field].is_null());}}
    assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
    assert_eq!(h.rpc(json!({"op":"usage"})),json!({}));assert_eq!(h.rpc(json!({"op":"logs"})),json!([]));
    let final_view=h.probe(false,"read",json!({}));for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(final_view[flag],false);}
    let quit=h.rpc(json!({"op":"quit"}));let deadline=Instant::now()+Duration::from_secs(3);
    loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<deadline,"Trace host did not quit normally");thread::sleep(Duration::from_millis(5));}
    assert!(!h.profile.join("control.sock").exists());
    let ended=SystemTime::now();
    let ended_ms=u64::try_from(ended.duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
    let ended_utc=chrono::DateTime::<chrono::Utc>::from(ended).to_rfc3339_opts(chrono::SecondsFormat::Millis,true);
    assert_eq!(format!("{:x}",Sha256::digest(fs::read(&host_path).unwrap())),host_sha);
    let receipt=json!({"host":{"pid":host_pid,"path":host_path,"sha256":host_sha,"imageUUID":host_uuid,"startedUnixMS":started_ms,"startedUTC":started_utc,"endedUnixMS":ended_ms,"endedUTC":ended_utc},"cycles":cycles,"measured":measured,"closedFieldCancelled":cancelled,"reset":cleared,"finalView":final_view,"quit":quit,"normalExit":true,"socketGone":true});
    fs::write(h.out.join("interaction-trace-receipt.json"),serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    println!("{}",json!({"ok":true,"interactionTrace":true,"cycles":100,"catalogCommands":500,"reconciliationBeforeCapture":true,"actualCaptureDraws":true,"closedFieldCancelled":true,"publicReset":true,"tracePayloadPrivate":true,"normalQuit":true,"physicalInput":false,"visibleWindowOpened":false,"scope":"100 retained native field/delegate/reconciliation cycles before offscreen captures; not result drawing or physical latency"}));
}


// Palette.swift 147, 175–176, 193: own native properties and direct
// callbacks only. Hidden hosts never install a global/local OS monitor.
fn palette_lifecycle(root: &Path) {
    for theme in ["light", "dark"] {
        let mut h=Host::new(root,theme);
        let baseline=fs::read(h.profile.join("config.json")).unwrap();
        let mut receipts=Vec::new();
        let check=|view:&Value,armed:bool| {
            assert_eq!(view["panelLevel"],3);
            assert_eq!(view["collectionBehavior"],257);
            assert_eq!(view["movableByWindowBackground"],true);
            assert_eq!(view["panelOpaque"],false);
            assert_eq!(view["panelHasShadow"],true);
            assert_eq!(view["releasedWhenClosed"],false);
            assert_eq!(view["panelBackgroundAlpha"].as_f64(),Some(0.0));
            assert_eq!(view["outsideArmed"],armed);
            assert_eq!(view["outsideGlobalInstalled"],false);
            assert_eq!(view["outsideLocalInstalled"],false);
            for flag in ["windowShown","windowKey","windowMain"] {assert_eq!(view[flag],false);}
        };
        for route in ["pending-query","local-own","local-action-menu","retired","global-left","global-right","local-other"] {
            h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic"}));
            h.wait(|status|status["nativeUI"]["visible"]==true);
            h.settled();
            let before=h.probe(false,"read",json!({}));check(&before,true);
            let callback=h.probe(false,"outside-click",json!({"route":route}));
            fs::write(h.out.join(format!("palette-callback-{route}-{theme}.json")),serde_json::to_vec_pretty(&callback).unwrap()).unwrap();
            if route.starts_with("local-") {assert_eq!(callback["callbackReturnedSameEvent"],true);}
            if route=="pending-query" {
                assert!(callback["queryEpochAfter"].as_u64().unwrap()>callback["queryEpochBefore"].as_u64().unwrap());
                assert_eq!(callback["viewEpochAfter"],callback["viewEpochBefore"]);
                assert_eq!(callback["visibleAfter"],false,"Pending query must not defeat outside dismissal: {callback}");
            }
            let remains=matches!(route,"local-own"|"local-action-menu"|"retired");
            h.wait(|status|status["nativeUI"]["visible"]==remains);
            h.settled();
            let after=h.probe(false,"read",json!({}));check(&after,remains);
            if route=="retired" {
                assert_ne!(callback["beforeGeneration"],callback["afterGeneration"]);
                assert_eq!(after["outsideGeneration"],callback["afterGeneration"]);
            }
            let fresh_after_retired=if route=="retired" {
                let fresh=h.probe(false,"outside-click",json!({"route":"global-left"}));
                h.wait(|status|status["nativeUI"]["visible"]==false);h.settled();
                check(&h.probe(false,"read",json!({})),false);
                Some(fresh)
            } else {None};
            receipts.push(json!({"route":route,"before":before,"callback":callback,"after":after,"freshAfterRetired":fresh_after_retired}));
            h.probe(false,"hide-palette",json!({}));
            h.wait(|status|status["nativeUI"]["visible"]==false);
            h.settled();
            let hidden=h.probe(false,"read",json!({}));check(&hidden,false);
            h.probe(false,"hide-palette",json!({}));h.settled();
            let repeated=h.probe(false,"read",json!({}));check(&repeated,false);
            assert_eq!(hidden["outsideGeneration"],repeated["outsideGeneration"]);
        }
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
        assert_eq!(h.rpc(json!({"op":"usage"})),json!({}));
        assert_eq!(h.rpc(json!({"op":"logs"})),json!([]));
        h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic"}));
        h.wait(|status|status["nativeUI"]["visible"]==true);h.settled();
        let before_quit=h.probe(false,"read",json!({}));check(&before_quit,true);
        h.capture(false,&format!("palette-lifecycle-{theme}"));
        let quit=h.rpc(json!({"op":"quit"}));let deadline=Instant::now()+Duration::from_secs(3);
        loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<deadline,"Lifecycle host failed normal quit");thread::sleep(Duration::from_millis(5));}
        assert!(!h.profile.join("control.sock").exists());
        fs::write(h.out.join(format!("palette-lifecycle-{theme}-receipts.json")),serde_json::to_vec_pretty(&json!({"theme":theme,"routes":receipts,"armedBeforeQuit":before_quit,"quit":quit,"normalExit":true,"socketGone":true})).unwrap()).unwrap();
    }
    println!("{}",json!({"ok":true,"paletteLifecycle":true,"themes":2,"callbackRoutes":7,"nativeFloatingAndSpaces":true,"retiredCallbackRefused":true,"normalQuit":true,"physicalInput":false,"visibleWindowOpened":false,"actualOSMonitorRegistration":false}));
}


// The four residual ActionMenuChecks premises only; accepted menu/footer,
// persistence and payload tests remain separate and are not rerun here.
fn item_binding_original(root:&Path) {
    for theme in ["light","dark"] {
        let mut h=Host::new(root,theme);
        let baseline=fs::read(h.profile.join("config.json")).unwrap();
        let show_app=|| {
            let previous=h.rpc(json!({"op":"status"}))["nativeUI"]["revision"].as_u64().unwrap();
            h.rpc(json!({"op":"show","mode":"commands","query":"Synthetic app"}));
            h.wait(|status| {
                let ui=&status["nativeUI"];
                ui["visible"]==true && ui["revision"].as_u64().is_some_and(|revision|revision>previous)
                    && ui["revision"]==ui["appliedRevision"]
                    && h.probe(false,"read",json!({}))["selectedId"]=="app.fixture"
            });h.settled();
        };
        show_app();
        // Real Named intent and attached persistent native control; only
        // main/Surface.apply is held by the current turn, not the worker.
        let queued=h.probe(false,"queued-recorder-hide",json!({}));
        fs::write(h.out.join(format!("item-binding-queued-{theme}.json")),serde_json::to_vec_pretty(&queued).unwrap()).unwrap();
        assert_eq!(queued["phase"],"pre-native-main-apply");assert_eq!(queued["targetId"],"app.fixture");
        assert_eq!(queued["attachedBefore"],true);assert_eq!(queued["attachedAfterQueue"],true);
        assert_eq!(queued["nativeEditorAfterQueue"],false);
        assert!(queued["intentQueued"].as_u64().unwrap()>queued["intentBefore"].as_u64().unwrap());
        assert!(queued["routingQueued"].as_u64().unwrap()>queued["routingBefore"].as_u64().unwrap());
        assert_eq!(queued["dismissalQueued"],queued["dismissalBefore"]);
        assert!(queued["dismissalAfterHide"].as_u64().unwrap()>queued["dismissalBefore"].as_u64().unwrap());
        for field in ["startsBefore","startsAfterQueue","startsAfterHide"] {assert_eq!(queued[field],0);}
        let owner_revision=queued["ownerRevisionBefore"].as_u64().unwrap();
        let drained=|view:&Value| {
            let pending=&view["recorderPendingInputs"];
            pending["overflow"]==false && ["query","actions","close","surface_cancel","save","preview","transition","native_probes"].iter().all(|key|pending[*key]==0)
        };
        let acknowledged=h.wait(|status| {
            let ui=&status["nativeUI"];
            ui["revision"].as_u64().is_some_and(|revision|revision>owner_revision)
                && ui["revision"]==ui["appliedRevision"] && ui["visible"]==false && ui["editorOpen"]==false
                && drained(&h.probe(false,"read",json!({})))
        });
        let after_queue=h.probe(false,"read",json!({}));
        assert!(drained(&after_queue));assert_eq!(after_queue["recorderAcquireStarts"],0);
        assert_eq!(after_queue["recording"],false);assert_eq!(after_queue["formVisible"],false);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
        fs::write(h.out.join(format!("item-binding-queued-{theme}-ack.json")),serde_json::to_vec_pretty(&json!({"queued":queued,"ownerAcknowledged":acknowledged,"after":after_queue})).unwrap()).unwrap();
        show_app();
        let plain=h.probe(false,"key-event",json!({"code":43,"flags":1u64<<20,"chars":","}));
        assert_eq!(plain["keyEventHandled"],false);
        assert_eq!(plain["formVisible"],false);
        assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
        let mut fields=Vec::new();
        for width in [600.,760.] {
            for (label,flags,alias) in [("alias",(1u64<<20)|(1u64<<19),true),("hotkey",(1u64<<20)|(1u64<<17),false)] {
                show_app();
                h.probe(false,"key-event",json!({"code":43,"flags":flags,"chars":","}));
                h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==true);
                h.probe(false,"dashboard-size",json!({"width":width,"height":490.}));h.settled();
                let before=h.probe(false,"read",json!({}));
                assert_eq!(before["editorId"],"app.fixture");
                if alias {
                    // Genuine native field-editor ownership is required by
                    // alias-type; no direct model text mutation.
                    h.probe(false,"alias-type",json!({"text":"owned-first, owned-second"}));
                    assert_eq!(h.probe(false,"read",json!({}))["aliasValue"],"owned-first, owned-second");
                } else {
                    h.wait(|_|h.probe(false,"read",json!({}))["recording"]==true);
                    let recorder=h.probe(false,"read",json!({}));
                    assert_eq!(recorder["recorderResponder"],true);
                    assert_eq!(recorder["recorderAccessibilityLabel"],"Press shortcut");
                }
                h.capture(false,&format!("item-binding-{label}-{theme}-{}",width as u32));
                let rendered=h.probe(false,"read",json!({}));
                let active_hide=!alias && width as u32==600;
                if active_hide {
                    assert_eq!(rendered["recording"],true);
                    h.probe(false,"hide-palette",json!({}));
                    h.wait(|status|status["nativeUI"]["visible"]==false);
                } else {h.probe(false,"key-event",json!({"code":53,"flags":0,"chars":""}));}
                h.wait(|_|h.probe(false,"read",json!({}))["formVisible"]==false);
                let cancelled=h.probe(false,"read",json!({}));assert_eq!(cancelled["recording"],false);
                assert_eq!(fs::read(h.profile.join("config.json")).unwrap(),baseline);
                fields.push(json!({"field":label,"width":width,"height":490.,"before":before,"rendered":rendered,"activeHide":active_hide,"cancelled":cancelled}));
            }
        }
        assert_eq!(h.rpc(json!({"op":"usage"})),json!({}));assert_eq!(h.rpc(json!({"op":"logs"})),json!([]));
        let quit=h.rpc(json!({"op":"quit"}));let deadline=Instant::now()+Duration::from_secs(3);
        loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<deadline,"Binding host failed normal quit");thread::sleep(Duration::from_millis(5));}
        assert!(!h.profile.join("control.sock").exists());
        fs::write(h.out.join(format!("item-binding-original-{theme}-receipts.json")),serde_json::to_vec_pretty(&json!({"theme":theme,"queuedBeforeMainApply":queued,"queuedOwnerAcknowledged":acknowledged,"queuedAfter":after_queue,"plainCommandComma":plain,"fields":fields,"quit":quit,"normalExit":true,"socketGone":true})).unwrap()).unwrap();
        keyboard_copy_original(root,theme);
    }
    println!("{}",json!({"ok":true,"itemBindingOriginal":true,"themes":2,"fieldCaptures":8,"queuedBeforeMainApplyStopped":true,"plainCommandCommaNotConsumed":true,"genuineRecorderResponderAndAX":true,"clipboardHotkeyOriginal":true,"hotkeyFailureNotice":true,"keyboardCopyCaptures":4,"normalQuit":true,"physicalInput":false,"visibleWindowOpened":false}));
}
fn keyboard_copy_original(root:&Path,theme:&str) {
    let mut h=Host::with_inputs(root,theme,None,false,true);
    h.wait(|s|s["keyboardGeneration"].as_u64().is_some_and(|g|g>0));
    assert_eq!(h.probe(false,"keyboard-press",json!({"code":9,"modifiers":6144}))["statuses"],json!([0]));
    h.wait(|s|s["nativeUI"]["visible"]==true&&h.probe(false,"read",json!({}))["accessibilityText"].as_array().unwrap().iter().any(|v|v=="Search clipboard"));
    h.settled();
    let clipboard=h.probe(false,"read",json!({}));
    // These labels are set only by the actual Model::clipboard branch.
    for text in ["Clipboard","Clipboard is empty","Copy ⌘↵ · Paste ↵"] {
        assert!(clipboard["screenText"].as_array().unwrap().iter().any(|v|v==text),"Missing clipboard-mode text {text}: {clipboard}");
    }
    assert!(clipboard["accessibilityText"].as_array().unwrap().iter().any(|v|v=="Search clipboard"));
    h.capture(false,&format!("clipboard-hotkey-{theme}"));
    assert_eq!(h.probe(false,"keyboard-press",json!({"code":0,"modifiers":256}))["statuses"],json!([0]));
    let deadline=Instant::now()+Duration::from_secs(3);
    let notice=loop {
        let value=h.probe(false,"notice-read",json!({}));
        if value["present"]==true {break value;}
        assert!(Instant::now()<deadline,"Own failed-command notice missing: {value}");
        thread::sleep(Duration::from_millis(5));
    };
    fs::write(h.out.join(format!("keyboard-copy-{theme}-proof.json")),serde_json::to_vec_pretty(&json!({"clipboard":clipboard,"notice":notice})).unwrap()).unwrap();
    assert_eq!(notice["text"],"Script exited with status 1.");assert_eq!(notice["success"],false);
    let rendered=h.probe(false,"notice-screenshot",json!({}));
    for key in ["windowShown","canBecomeKey"] {assert_eq!(rendered[key],false);}
    assert_eq!(rendered["nonactivating"],true);assert_eq!(rendered["ignoresMouse"],true);
    fs::copy(h.profile.join("native-notice-probe.png"),h.out.join(format!("script-error-{theme}.png"))).unwrap();
    fs::write(h.out.join(format!("script-error-{theme}.json")),serde_json::to_vec_pretty(&rendered).unwrap()).unwrap();
    let quit=h.rpc(json!({"op":"quit"}));let deadline=Instant::now()+Duration::from_secs(3);
    loop {if let Some(status)=h.child.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<deadline,"Keyboard host failed normal quit");thread::sleep(Duration::from_millis(5));}
    assert!(!h.profile.join("control.sock").exists());
    fs::write(h.out.join(format!("keyboard-copy-{theme}-quit.json")),serde_json::to_vec_pretty(&json!({"quit":quit,"normalExit":true,"socketGone":true})).unwrap()).unwrap();
}
