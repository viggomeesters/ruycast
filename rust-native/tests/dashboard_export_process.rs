//! The five original export workloads, with file-backed host diagnostics.
#![cfg(feature = "test-tools")]
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn shim() -> &'static Path {
    static SHIM: OnceLock<PathBuf> = OnceLock::new();
    SHIM.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("target/dashboard-export-shim.dylib");
        let output = Command::new("/usr/bin/clang")
            .args(["-Wall", "-Wextra", "-Werror", "-dynamiclib"])
            .arg(root.join("tests/dashboard_export_shim.c"))
            .arg("-o")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    })
}
fn private_file(path: &Path) -> File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap()
}
struct Host {
    base: PathBuf,
    child: Child,
}
impl Host {
    fn start() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("owned fixture parent required"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        assert_eq!(
            fs::read(parent.join(".ruycast-ui-fixture-parent")).unwrap(),
            b"owned-ui-fixture-parent-v1"
        );
        let base = parent.join(format!(
            "rde-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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
        fs::write(
            base.join("p/config.json"),
            include_bytes!("../../config/default.json"),
        )
        .unwrap();
        let binary = root.join("target/native-tools/debug/ruycast-native");
        let child = Command::new(&binary)
            .arg("--native-hidden")
            .env_clear()
            .env("HOME", &base)
            .env("RUYCAST_HOME", base.join("p"))
            .env("RUYCAST_APP_BINARY", &binary)
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                serde_json::to_string(&[base.join("Applications")]).unwrap(),
            )
            .env("DYLD_INSERT_LIBRARIES", shim())
            .env("RUYCAST_DASHBOARD_FIXTURE_HOME", &base)
            .stdin(Stdio::null())
            .stdout(private_file(&base.join("stdout")))
            .stderr(private_file(&base.join("stderr")))
            .spawn()
            .unwrap();
        let mut host = Self { base, child };
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(
                host.child.try_wait().unwrap().is_none(),
                "host exited; diagnostics at {}",
                host.base.display()
            );
            if host
                .rpc_timeout(json!({"op":"status"}), Duration::from_millis(300))
                .is_ok_and(|v| v["ok"] == true)
            {
                break;
            }
            assert!(Instant::now() < until, "native bridge did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        host.settled(|_| true);
        host.ok(json!({"op":"show","mode":"analytics"}));
        host.settled(|v| v["managementUI"]["tab"] == "dashboard");
        fs::create_dir(host.base.join("export")).unwrap();
        fs::set_permissions(host.base.join("export"), fs::Permissions::from_mode(0o755)).unwrap();
        host
    }
    fn rpc_timeout(&self, request: Value, timeout: Duration) -> Result<Value, String> {
        let mut stream =
            UnixStream::connect(self.base.join("p/control.sock")).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut stream, &request).map_err(|e| e.to_string())?;
        stream.write_all(b"\n").map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    }
    fn rpc(&self, request: Value) -> Value {
        self.rpc_timeout(request, Duration::from_secs(3)).unwrap()
    }
    fn ok(&self, request: Value) -> Value {
        let v = self.rpc(request);
        assert_eq!(v["ok"], true, "{v}");
        v["result"].clone()
    }
    fn probe(&self, action: &str, mut fields: Value) -> Value {
        fields["op"] = json!("native.test");
        fields["surface"] = json!("settings");
        fields["action"] = json!(action);
        self.ok(fields)
    }
    fn settled(&self, predicate: impl Fn(&Value) -> bool) {
        let until = Instant::now() + Duration::from_secs(7);
        loop {
            let v = self.ok(json!({"op":"status"}));
            if v["configurationUpdating"] != true
                && v["managementUI"]["revision"]
                    .as_u64()
                    .is_some_and(|r| r > 0)
                && v["managementUI"]["revision"] == v["managementUI"]["appliedRevision"]
                && predicate(&v)
            {
                return;
            }
            assert!(Instant::now() < until, "configuration did not settle: {v}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let v = self.probe("dashboard-read", json!({}));
            if predicate(&v) {
                return v;
            }
            assert!(Instant::now() < until, "{v}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn touch(&self, name: &str) {
        fs::write(self.base.join(name), b"").unwrap();
    }
    fn entered(&self, name: &str) {
        let until = Instant::now() + Duration::from_secs(3);
        while !self.base.join(name).exists() {
            assert!(Instant::now() < until, "missing actual syscall hold {name}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn destination(&self) -> PathBuf {
        self.base.join("export/diagnostics.json")
    }
    fn begin(&self) {
        self.probe("dashboard-control", json!({"index":2}));
        let v = self.probe("dashboard-read", json!({}));
        assert_eq!(v["fixtureDialog"], true);
        assert_eq!(self.probe("dashboard-dialog-complete", json!({"serial":v["dialogSerial"],"token":v["surface"]["token"],"accepted":true,"path":self.destination()}))["accepted"], true);
    }
    fn capture(&self, name: &str) {
        if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let v = self.probe("dashboard-screenshot", json!({}));
            let folder = PathBuf::from(folder);
            fs::create_dir_all(&folder).unwrap();
            fs::copy(
                self.base.join("p/native-dashboard-probe.png"),
                folder.join(format!("{name}.png")),
            )
            .unwrap();
            fs::write(
                folder.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&v).unwrap(),
            )
            .unwrap();
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // Release both real syscall holds before asking the owner to stop.
        for name in ["export-file-sync-release", "export-rename-release"] {
            let _ = fs::write(self.base.join(name), b"");
        }
        let _ = self.rpc_timeout(json!({"op":"quit"}), Duration::from_secs(3));
        let until = Instant::now() + Duration::from_secs(5);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = self.child.kill();
                    let reap_until = Instant::now() + Duration::from_secs(2);
                    loop {
                        match self.child.try_wait() {
                            Ok(Some(_)) => break,
                            Ok(None) if Instant::now() < reap_until => {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            _ => {
                                eprintln!("Owned host PID {} could not be reaped within the original 2s kill deadline", self.child.id());
                                break;
                            }
                        }
                    }
                    break None;
                }
            }
        };
        // Keep complete logs even on kill or panic; never replay into a blocked pipe.
        let mut retained = false;
        if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let folder = PathBuf::from(folder);
            let _ = fs::create_dir_all(&folder);
            let folder = folder.join(self.base.file_name().unwrap());
            if fs::DirBuilder::new().mode(0o700).create(&folder).is_ok() {
                retained = ["stdout", "stderr"]
                    .iter()
                    .all(|name| fs::copy(self.base.join(name), folder.join(name)).is_ok());
            }
        }
        let success = status.is_some_and(|s| s.success());
        if !success || std::thread::panicking() {
            eprintln!("Host diagnostics retained at {}", self.base.display());
            for name in ["stdout", "stderr"] {
                if let Ok(bytes) = fs::read(self.base.join(name)) {
                    eprintln!("{name}: {}", String::from_utf8_lossy(&bytes));
                }
            }
        } else if retained {
            let _ = fs::remove_dir_all(&self.base);
        }
        if !std::thread::panicking() {
            assert!(
                success,
                "host failed or exceeded original shutdown deadline"
            );
        }
    }
}

#[test]
fn positive_held_export_global_reset_order_and_parent_mode() {
    let h = Host::start();
    h.touch("hold-export-file-sync");
    h.begin();
    h.entered("export-file-sync-entered");
    assert_eq!(h.rpc(json!({"op":"status"}))["ok"], true);
    assert_eq!(
        h.rpc(json!({"op":"analytics.reset","confirmed":true}))["ok"],
        false
    );
    assert_eq!(
        h.wait(|v| v["surface"]["busy"] == true)["surface"]["busy"],
        true
    );
    h.capture("export-busy");
    h.touch("export-file-sync-release");
    h.wait(|v| v["surface"]["busy"] == false && v["surface"]["message"] == "Diagnostics exported");
    assert_eq!(
        fs::metadata(h.destination().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(h.destination()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let v: Value = serde_json::from_slice(&fs::read(h.destination()).unwrap()).unwrap();
    assert!(v.get("commands").is_none());
    assert_eq!(
        h.rpc(json!({"op":"analytics.reset","confirmed":true}))["ok"],
        true
    );
}
#[test]
fn file_sync_and_rename_failure_preserve_prior_destination() {
    for (fault, marker) in [
        ("fail-export-file-sync", "export-file-sync-failed"),
        ("fail-export-rename", "export-rename-failed"),
    ] {
        let h = Host::start();
        fs::write(h.destination(), b"prior").unwrap();
        h.touch(fault);
        h.begin();
        h.wait(|v| {
            v["surface"]["busy"] == false
                && v["surface"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Could not export")
        });
        assert!(h.base.join(marker).exists());
        assert_eq!(fs::read(h.destination()).unwrap(), b"prior");
        assert_eq!(
            fs::metadata(h.destination().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
#[test]
fn postrename_parent_sync_failure_reports_published_outcome() {
    let h = Host::start();
    fs::write(h.destination(), b"prior").unwrap();
    h.touch("fail-export-parent-sync");
    h.begin();
    let v = h.wait(|v| {
        v["surface"]["busy"] == false
            && v["surface"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("durability could not be confirmed")
    });
    assert!(h.base.join("export-parent-sync-failed").exists());
    assert!(
        serde_json::from_slice::<Value>(&fs::read(h.destination()).unwrap())
            .unwrap()
            .is_object()
    );
    assert!(v["surface"]["message"]
        .as_str()
        .unwrap()
        .contains("Check the file before retrying"));
    h.probe("dashboard-size", json!({"width":680,"height":480}));
    h.capture("export-published-warning");
}
fn close_and_reopen(h: &Host) {
    h.probe("window-close", json!({}));
    h.settled(|s| s["managementUI"]["visible"] == false);
    h.ok(json!({"op":"show","mode":"analytics"}));
    h.settled(|s| s["managementUI"]["visible"] == true);
}
#[test]
fn close_before_commit_revokes_destination_without_feedback_to_reopened_window() {
    let h = Host::start();
    fs::write(h.destination(), b"prior").unwrap();
    h.touch("hold-export-file-sync");
    h.begin();
    h.entered("export-file-sync-entered");
    close_and_reopen(&h);
    h.touch("export-file-sync-release");
    h.wait(|v| v["surface"]["busy"] == false);
    assert_eq!(fs::read(h.destination()).unwrap(), b"prior");
    assert_eq!(
        h.probe("dashboard-read", json!({}))["surface"]["message"],
        ""
    );
}
#[test]
fn commit_entered_before_close_keeps_published_outcome_and_retires_feedback() {
    let h = Host::start();
    fs::write(h.destination(), b"prior").unwrap();
    h.touch("hold-export-rename");
    h.begin();
    h.entered("export-rename-entered");
    close_and_reopen(&h);
    h.touch("export-rename-release");
    h.wait(|v| v["surface"]["busy"] == false);
    assert!(
        serde_json::from_slice::<Value>(&fs::read(h.destination()).unwrap())
            .unwrap()
            .is_object()
    );
    assert_eq!(
        h.probe("dashboard-read", json!({}))["surface"]["message"],
        ""
    );
}
