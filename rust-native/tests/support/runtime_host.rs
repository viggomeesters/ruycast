//! Owned headless host, explicit executables and absolute-deadline Unix RPC.
#[path = "child_output.rs"]
mod child_output;
#[path = "status_rpc.rs"]
pub mod status_rpc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::DirBuilderExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
pub fn default_config() -> Value {
    serde_json::from_str(include_str!("../../../config/default.json")).unwrap()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn bind_artifact(path: &Path, label: &str) -> (String, String) {
    let root = root();
    let bytes = fs::read(path).unwrap();
    let digest = hash(&bytes);
    let destination = if path.starts_with(&root) {
        path.to_owned()
    } else {
        let parent = root.join("rust-native/target/runtime-proof-artifacts");
        fs::create_dir_all(&parent).unwrap();
        let target = parent.join(format!("{label}-{digest}"));
        if !target.exists() {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let temporary = parent.join(format!(
                "copy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .unwrap();
            struct Pending(PathBuf);
            impl Drop for Pending {
                fn drop(&mut self) {
                    let _ = fs::remove_file(&self.0);
                }
            }
            let pending = Pending(temporary.clone());
            file.write_all(&bytes).unwrap();
            drop(file);
            let published = fs::hard_link(&temporary, &target);
            drop(pending);
            if let Err(error) = published {
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            }
        }
        target
    };
    assert_eq!(hash(&fs::read(&destination).unwrap()), digest);
    (
        destination
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        digest,
    )
}
// Each fixture consumer selects only its own guarded entry point.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Entry {
    HeadlessTest,
    HiddenTest,
}
#[derive(Clone)]
pub struct Config {
    pub host: PathBuf,
    pub cli: PathBuf,
}
impl Config {
    pub fn current() -> Self {
        let host = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
        let cli = PathBuf::from(
            std::env::var_os("RUYCAST_TEST_CLI")
                .expect("Set RUYCAST_TEST_CLI to the source-built Rust CLI"),
        );
        for path in [&host, &cli] {
            assert!(
                path.is_absolute() && path.is_file(),
                "Actual absolute Rust executable required: {}",
                path.display()
            );
        }
        Self { host, cli }
    }
}
pub struct OwnedDir(pub PathBuf);
impl OwnedDir {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Set an owned absolute fixture parent"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        let path = parent.join(format!(
            "rpc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Drain {
    stop: Arc<AtomicBool>,
    result: mpsc::Receiver<io::Result<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
}
impl Drain {
    fn new(mut pipe: impl Read + AsRawFd + Send + 'static) -> Self {
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0
        );
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let (tx, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            let value = (|| {
                let mut output = Vec::new();
                let mut buffer = [0; 8192];
                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) => return Ok(output),
                        Ok(count) => {
                            if output.len() + count > 1024 * 1024 {
                                return Err(io::Error::other("Owned host output exceeds 1 MiB"));
                            }
                            output.extend_from_slice(&buffer[..count]);
                            continue;
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e),
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Ok(output);
                    }
                    let mut descriptor = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let polled = unsafe { libc::poll(&mut descriptor, 1, 50) };
                    if polled < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
            })();
            let _ = tx.send(value);
        });
        Self {
            stop,
            result,
            worker: Some(worker),
        }
    }
    fn finish(mut self) -> io::Result<Vec<u8>> {
        self.stop.store(true, Ordering::Release);
        let result = self
            .result
            .recv_timeout(Duration::from_secs(1))
            .map_err(io::Error::other)?;
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| io::Error::other("Owned output drain panicked"))?;
        result
    }
}
pub struct Host {
    pub base: PathBuf,
    pub profile: PathBuf,
    pub config: PathBuf,
    pub applications: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub settings: Config,
    pub process: Option<Child>,
    drains: Option<(Drain, Drain)>,
    owned: OwnedDir,
}
impl Host {
    pub fn new(settings: Config, start: bool) -> Self {
        let owned = OwnedDir::new();
        let base = owned.0.clone();
        let profile = base.join("p");
        fs::DirBuilder::new().mode(0o700).create(&profile).unwrap();
        let config = profile.join("config.json");
        fs::write(&config, serde_json::to_vec(&default_config()).unwrap()).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            "own-disposable-fixture-v1",
        )
        .unwrap();
        let applications = base.join("Applications");
        fs::create_dir(&applications).unwrap();
        let tmp = base.join("tmp");
        fs::create_dir(&tmp).unwrap();
        let mut env = std::env::vars_os()
            .filter(|(k, _)| {
                let k = k.to_string_lossy();
                !k.starts_with("RUYCAST_") && !k.starts_with("RUSTCAST_") && !k.starts_with("DYLD_")
            })
            .collect::<BTreeMap<_, _>>();
        for (k, v) in [
            ("RUYCAST_HOME", profile.as_os_str().to_owned()),
            ("HOME", base.as_os_str().to_owned()),
            ("TMPDIR", tmp.as_os_str().to_owned()),
            ("RUYCAST_APP_BINARY", settings.host.as_os_str().to_owned()),
        ] {
            env.insert(k.into(), v);
        }
        let mut host = Self {
            base,
            profile,
            config,
            applications,
            env,
            settings,
            process: None,
            drains: None,
            owned,
        };
        host.env.insert(
            "RUYCAST_NATIVE_TEST_APP_ROOTS".into(),
            serde_json::to_string(&[&host.applications]).unwrap().into(),
        );
        if start {
            host.start();
        }
        host
    }
    pub fn command(&self, executable: &Path) -> Command {
        let mut c = Command::new(executable);
        c.env_clear()
            .envs(&self.env)
            .current_dir(&self.base)
            .stdin(Stdio::null());
        c
    }
    pub fn socket(&self) -> PathBuf {
        self.profile.join("control.sock")
    }
    pub fn start(&mut self) {
        self.start_entry(Entry::HeadlessTest);
    }
    pub fn start_entry(&mut self, entry: Entry) {
        let (argument, timeout) = match entry {
            Entry::HeadlessTest => ("--headless-test", Duration::from_secs(6)),
            Entry::HiddenTest => ("--native-hidden", Duration::from_secs(8)),
        };
        assert!(self.process.is_none());
        let mut child = self
            .command(&self.settings.host)
            .arg(argument)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        // Track the child before allocating drain threads, so unwinding a
        // partial startup still kills and reaps this exact owned process.
        self.process = Some(child);
        let out = Drain::new(stdout);
        let err = Drain::new(stderr);
        self.drains = Some((out, err));
        let socket = self.socket();
        status_rpc::wait(&socket, timeout, || {
            self.process.as_mut().unwrap().try_wait().unwrap().is_none()
        });
    }
    pub fn rpc(&self, request: &Value) -> Value {
        self.rpc_timeout(request, Duration::from_secs(3))
    }
    pub fn rpc_timeout(&self, request: &Value, timeout: Duration) -> Value {
        status_rpc::request(&self.socket(), request, Instant::now() + timeout).unwrap()
    }
    pub fn connect(&self, timeout: Duration) -> UnixStream {
        let stream = status_rpc::connect(&self.socket(), Instant::now() + timeout).unwrap();
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        stream.set_write_timeout(Some(timeout)).unwrap();
        stream
    }
    pub fn cli_output(&self, args: &[&str]) -> Output {
        run(
            self.command(&self.settings.cli).args(args),
            Duration::from_secs(6),
        )
    }
    pub fn cli(&self, args: &[&str]) -> Value {
        let output = self.cli_output(args);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    pub fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "Owned configuration did not reach expected state"
            );
            let value = self.rpc_timeout(
                &json!({"op":"status"}),
                remaining.min(Duration::from_secs(3)),
            )["result"]
                .clone();
            if predicate(&value) {
                return value;
            }
            thread::sleep(
                Duration::from_millis(30).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    pub fn replace(&self, value: &Value, path: Option<&Path>) {
        self.replace_bytes(&serde_json::to_vec(value).unwrap(), path);
    }
    pub fn replace_bytes(&self, bytes: &[u8], path: Option<&Path>) {
        let path = path.unwrap_or(&self.config);
        assert!(path.starts_with(&self.base));
        let temporary = path.with_extension("candidate");
        fs::write(&temporary, bytes).unwrap();
        fs::rename(temporary, path).unwrap();
    }
    pub fn wait_exit(&mut self, timeout: Duration, kill_fallback: bool) -> Output {
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                assert!(
                    kill_fallback,
                    "Owned host did not exit normally before deadline"
                );
                let child = self.process.as_mut().unwrap();
                child.kill().unwrap();
                break child.wait().unwrap();
            }
            thread::sleep(Duration::from_millis(5));
        };
        self.process.take();
        let (out, err) = self.drains.take().unwrap();
        Output {
            status,
            stdout: out.finish().unwrap(),
            stderr: err.finish().unwrap(),
        }
    }
    pub fn stop(&mut self, graceful: bool) -> Option<Output> {
        self.process.as_ref()?;
        if self.process.as_mut().unwrap().try_wait().unwrap().is_none()
            && (!graceful
                || status_rpc::request(
                    &self.socket(),
                    &json!({"op":"quit"}),
                    Instant::now() + Duration::from_secs(3),
                )
                .is_err())
        {
            let _ = self.process.as_mut().unwrap().kill();
        }
        let output = self.wait_exit(Duration::from_secs(5), true);
        if graceful {
            assert!(
                output.status.success(),
                "Own host failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Some(output)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // Always reap the still-owned child, even while unwinding an assertion.
        if thread::panicking() {
            if let Some(child) = &mut self.process {
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            if let Some((out, err)) = self.drains.take() {
                let _ = out.finish();
                let _ = err.finish();
            }
        } else {
            self.stop(true);
        }
        // The field owns only its exclusively created directory.
        let _ = &self.owned;
    }
}
pub fn run(command: &mut Command, timeout: Duration) -> Output {
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child_output::wait(child, timeout)
}
