//! Independent helper death through the actual headless host and Rust heartbeat.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    time::{Duration, Instant},
};
struct ManagedHost(Host);
impl std::ops::Deref for ManagedHost {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.0
    }
}
impl std::ops::DerefMut for ManagedHost {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.0
    }
}
impl Drop for ManagedHost {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.stop(true)));
        }
    }
}
struct PausedHost<'a> {
    host: &'a Host,
    resumed: bool,
}
impl<'a> PausedHost<'a> {
    fn new(host: &'a Host) -> Self {
        let pid = host.process.as_ref().unwrap().id() as libc::pid_t;
        assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
        Self {
            host,
            resumed: false,
        }
    }
    fn resume(mut self) {
        let pid = self.host.process.as_ref().unwrap().id() as libc::pid_t;
        assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
        self.resumed = true;
    }
}
impl Drop for PausedHost<'_> {
    fn drop(&mut self) {
        if !self.resumed {
            let _ = unsafe {
                libc::kill(
                    self.host.process.as_ref().unwrap().id() as libc::pid_t,
                    libc::SIGCONT,
                )
            };
        }
    }
}
fn wait_file(path: &Path) -> Value {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if path.exists() {
            return serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("own script did not start");
}
#[test]
fn unexpected_helper_death_reports_unknown_without_retry() {
    let mut host = ManagedHost(Host::new(Config::current(), false));
    fs::write(
        host.base.join(".ruycast-supervision-fixture"),
        "owned-supervision-helper-v1",
    )
    .unwrap();
    let command = json!({"id":"script.life0","title":"Lifecycle fixture0","kind":"script","target":env!("CARGO_BIN_EXE_supervision-probe"),"arguments":["__supervision-heartbeat",host.base,"0"],"timeout":5});
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(command);
    host.replace(&config, None);
    host.start();
    let mut stream = host.connect(Duration::from_secs(3));
    stream
        .write_all(b"{\"op\":\"execute\",\"id\":\"script.life0\"}\n")
        .unwrap();
    let initial = wait_file(&host.base.join("0"));
    let helper = libc::pid_t::try_from(initial["helper"].as_i64().unwrap()).unwrap();
    assert!(helper > 0);
    assert_ne!(helper, host.process.as_ref().unwrap().id() as libc::pid_t);
    // The owning parent is stopped first; its unreaped helper PID cannot be recycled.
    let paused = PausedHost::new(&host);
    assert_eq!(unsafe { libc::kill(helper, libc::SIGKILL) }, 0);
    paused.resume();
    let mut line = String::new();
    BufReader::new(&mut stream).read_line(&mut line).unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("before retrying"));
    let usage = host.rpc(&json!({"op":"usage"}));
    assert_eq!(usage["result"], json!({}));
    std::thread::sleep(Duration::from_millis(1300));
    let last: Value = serde_json::from_slice(&fs::read(host.base.join("0")).unwrap()).unwrap();
    assert_eq!(last.get("pid").unwrap(), initial.get("pid").unwrap());
    let pid = libc::pid_t::try_from(initial["pid"].as_i64().unwrap()).unwrap();
    assert!(pid > 0);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = std::path::PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("supervision-host-helper-death.json"),serde_json::to_vec_pretty(&json!({"initial":initial,"last":last,"response":result,"usage":usage,"killZeroErrno":libc::ESRCH})).unwrap()).unwrap();
    }
}
