//! Complete clipboard file/image paste tickets on the existing owned SDK fixture.
//! Uses hidden windows, a private named board and synthetic target/key events.
#![cfg(feature = "test-tools")]

use serde_json::{json, Value};
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Host {
    base: PathBuf,
    child: Option<Child>,
    cli: PathBuf,
}
impl Host {
    fn start(shim: &Path) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixtures =
            PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect(
                "set RUYCAST_NATIVE_TEST_FIXTURE_DIR to an existing owned fixture directory",
            ));
        assert!(fixtures.is_absolute() && fixtures.is_dir());
        let base = fixtures.join(format!("rcrp-{}", std::process::id()));
        assert!(!base.exists(), "refuse reusing a fixture");
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(base.join("p"))
            .unwrap();
        fs::create_dir(base.join("Applications")).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        fs::copy(
            root.join("../config/default.json"),
            base.join("p/config.json"),
        )
        .unwrap();
        let binary = env!("CARGO_BIN_EXE_ruycast-native");
        let cli = std::env::var_os("RUYCAST_RICH_PASTE_CLI")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("../rust/target/debug/ruycast"));
        assert!(cli.is_file(), "build the existing Rust CLI first");
        let mut host = Self {
            base,
            child: None,
            cli,
        };
        host.child = Some(
            Command::new(binary)
                .arg("--native-hidden")
                .env_clear()
                .env("HOME", &host.base)
                .env("RUYCAST_HOME", host.base.join("p"))
                .env("RUYCAST_APP_BINARY", binary)
                .env(
                    "RUYCAST_NATIVE_TEST_APP_ROOTS",
                    serde_json::to_string(&[host.base.join("Applications")]).unwrap(),
                )
                .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &host.base)
                .env("RUYCAST_NATIVE_TEST_CLIPBOARD", "named")
                .env("RUYCAST_NATIVE_TEST_SNIPPET", "1")
                .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1")
                .env("DYLD_INSERT_LIBRARIES", shim)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let until = Instant::now() + Duration::from_secs(8);
        while !host.base.join("p/control.sock").exists() {
            assert!(
                host.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "owned host exited"
            );
            assert!(Instant::now() < until, "owned host readiness timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
        host.wait(|s| {
            s["nativeUI"]["revision"].as_u64().is_some_and(|r| r > 0)
                && s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]
        });
        host
    }
    fn rpc(&self, value: Value) -> Value {
        let mut socket = UnixStream::connect(self.base.join("p/control.sock")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(&mut socket, &value).unwrap();
        socket.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn ok(&self, value: Value) -> Value {
        let response = self.rpc(value);
        assert_eq!(response["ok"], true, "{response}");
        response["result"].clone()
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) {
        let until = Instant::now() + Duration::from_secs(6);
        loop {
            let status = self.ok(json!({"op":"status"}));
            if predicate(&status) {
                return;
            }
            assert!(
                Instant::now() < until,
                "owned host did not settle: {status}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn trace(&self) -> Vec<Value> {
        fs::read_to_string(self.base.join("workspace-trace.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn items(&self) -> Value {
        self.ok(json!({"op":"clipboard.list"}))
    }
    fn hidden(&self) {
        let view = self.ok(json!({"op":"native.test","action":"read"}));
        for name in ["windowShown", "windowKey", "windowMain"] {
            assert_eq!(view[name], false, "{view}");
        }
    }
    fn show(&self) {
        self.ok(json!({"op":"show","mode":"clipboard"}));
        self.wait(|s| {
            s["nativeUI"]["visible"] == true
                && s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]
        });
        self.hidden();
    }
    fn paste(&self, id: &str) -> Value {
        let child = Command::new(&self.cli)
            .args(["clipboard", "paste", id])
            .env_clear()
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", self.base.join("p"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child_output::wait(child, Duration::from_secs(6));
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn stop(&mut self) {
        self.ok(json!({"op":"quit"}));
        let mut child = self.child.take().unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned host shutdown timeout");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.wait().unwrap().success());
        assert!(!self.base.join("p/control.sock").exists());
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if fs::read(self.base.join(".ruycast-test-fixture"))
            .ok()
            .as_deref()
            == Some(b"own-disposable-fixture-v1")
        {
            fs::remove_dir_all(&self.base).unwrap();
        }
    }
}

#[test]
fn file_and_image_paste_complete_owned_ticket_without_recapture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shim = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("rich-paste-effects.dylib");
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
        ])
        .arg(root.join("tests/clipboard_history_shim.m"))
        .arg("-o")
        .arg(&shim)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut host = Host::start(&shim);
    host.hidden();
    let poll = host
        .ok(json!({"op":"native.test","action":"clipboard-fixture","checkPerformancePoll":true}));
    assert_eq!(poll["ownedPerformancePoll"], true);
    for name in ["owned-a", "owned-b"] {
        fs::write(host.base.join(name), b"synthetic owned file").unwrap();
    }
    // Exact existing clipboard_history::tests 2x2 synthetic PNG fixture.
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112, 80, 248,
        15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ];
    let files: Vec<_> = ["owned-a", "owned-b"]
        .iter()
        .map(|name| format!("file://{}", host.base.join(name).display()))
        .collect();
    let mut image = Value::Null;
    for (count, kind, payload, item_count) in [
        (1, "files", json!({"Files":files}), 2),
        (2, "image", json!({"Image":png}), 1),
    ] {
        let fixture = host.ok(
            json!({"op":"native.test","action":"clipboard-fixture","payload":payload,"defer":true}),
        );
        assert_eq!(fixture["namedBoard"], true);
        host.wait(|s| s["clipboardCount"] == count);
        let items = host.items();
        let clip = &items[0];
        assert_eq!(clip["contentType"], kind);
        if kind == "image" {
            image = clip.clone();
        }
        host.show();
        let before = host.trace().len();
        let response = host.paste(clip["id"].as_str().unwrap());
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(response["result"]["state"], "copied");
        host.wait(|s| s["snippetRequestsRunning"] == 0 && s["snippetTransactionsPending"] == 0);
        let trace = host.trace();
        let trace = &trace[before..];
        let writes: Vec<_> = trace
            .iter()
            .filter(|v| v["phase"] == "snippet-write")
            .collect();
        assert_eq!(writes.len(), 1, "{trace:?}");
        assert_eq!(writes[0]["ok"], true);
        assert_eq!(writes[0]["items"], item_count);
        let posts: Vec<_> = trace
            .iter()
            .filter(|v| v["phase"] == "snippet-post")
            .collect();
        assert_eq!(posts.len(), 2, "{trace:?}");
        for (post, down) in posts.iter().zip([true, false]) {
            assert_eq!(post["key"], 9);
            assert_eq!(post["down"], down);
            assert_eq!(post["flags"], 1 << 20);
            assert_eq!(post["pid"], 4260);
        }
        assert_eq!(
            trace
                .iter()
                .filter(|v| v["phase"] == "snippet-clear")
                .count(),
            1
        );
        assert!(!trace.iter().any(|v| v["phase"]
            .as_str()
            .is_some_and(|p| p.starts_with("unexpected-general"))));
        // Two production 0.4s capture polls: own rich write must not become history.
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(host.items(), items);
        host.hidden();
    }
    // Loading/validating an invalid asset must fail before any board clear/event.
    fs::write(
        host.base
            .join("p")
            .join(image["dataFile"].as_str().unwrap()),
        b"invalid owned PNG",
    )
    .unwrap();
    let before = host.trace().len();
    let denied = host.rpc(json!({"op":"clipboard.paste","id":image["id"]}));
    assert_eq!(denied["ok"], false, "{denied}");
    assert!(denied["actionCompleted"].is_null());
    assert!(host.trace()[before..].iter().all(|v| ![
        "snippet-clear",
        "snippet-write",
        "snippet-post"
    ]
    .contains(&v["phase"].as_str().unwrap_or(""))));
    host.stop();
    println!(
        "{}",
        json!({"filePaste":true,"imagePaste":true,"exactBoardMatchBeforeSyntheticEvents":true,"ownWriteNotRecaptured":true,"invalidAssetNoMutation":true,"realExternalPasteTested":false})
    );
}
