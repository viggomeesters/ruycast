//! Explicit opt-in native proof. Compiled only into the unit-test executable;
//! neither a product feature nor an environment-variable production bypass.
use super::safety::{owned_fixture_guard, Decision, Guard, Helpers, Signal, Snapshot};
use super::*;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::NSRunningApplication;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};
const BUNDLE: &str = "test.ruycast.OwnedAutoQuit";
static OWNED_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
// Public Process Manager reads avoid NSRunningApplication's documented cache
// until the next main-runloop turn. No activation or application enumeration.
#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(process: *mut ProcessSerialNumber) -> i16;
    fn GetProcessPID(process: *const ProcessSerialNumber, pid: *mut i32) -> i32;
}
fn foreground_pid() -> Option<i32> {
    let mut process = ProcessSerialNumber { high: 0, low: 0 };
    let mut pid = 0;
    if unsafe { GetFrontProcess(&mut process) } != 0
        || unsafe { GetProcessPID(&process, &mut pid) } != 0
        || pid <= 1
    {
        return None;
    }
    Some(pid)
}

struct Owned {
    child: Child,
    identity: Identity,
    app: Retained<NSRunningApplication>,
    executable: PathBuf,
    receipt: PathBuf,
    attempted: Vec<Due>,
}
impl Owned {
    fn launch(root: &Path, binary: &Path, mode: &str) -> Self {
        let directory = root.join(format!("{}-{mode}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let contents = directory.join("OwnedAutoQuit.app/Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        let executable = contents.join("MacOS/owned-auto-quit");
        fs::copy(binary, &executable).unwrap();
        fs::write(contents.join("Info.plist"),format!(r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{BUNDLE}</string><key>CFBundleExecutable</key><string>owned-auto-quit</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#)).unwrap();
        let receipt = directory.join("receipt.txt");
        let mut child = Command::new(&executable)
            .arg(mode)
            .arg(&receipt)
            .env_remove("DYLD_INSERT_LIBRARIES")
            .env_remove("DYLD_LIBRARY_PATH")
            .stdin(Stdio::piped())
            .stdout(fs::File::create(directory.join("stdout.log")).unwrap())
            .stderr(fs::File::create(directory.join("stderr.log")).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if fs::read_to_string(&receipt).is_ok_and(|v| v == "ready\n") {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "owned app exited before readiness"
            );
            assert!(Instant::now() < deadline, "owned app readiness timeout");
            thread::sleep(Duration::from_millis(10));
        }
        let pid = child.id() as i32;
        let identity = crate::system_search::native::identity(pid).unwrap();
        assert_eq!(identity.uid, unsafe { libc::getuid() });
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid).unwrap();
        Self {
            child,
            identity,
            app,
            executable,
            receipt,
            attempted: vec![],
        }
    }
    fn snapshot(&self, now: Duration) -> Option<Snapshot> {
        if crate::system_search::native::identity(self.identity.pid) != Some(self.identity)
            || self.app.isTerminated()
            || self.app.processIdentifier() != self.identity.pid
            || self.app.bundleIdentifier()?.to_string() != BUNDLE
            || self.app.executableURL()?.path()?.to_string() != self.executable.to_str()?
        {
            return None;
        }
        let activity = match foreground_pid() {
            Some(pid) if pid == self.identity.pid => Activity::Foreground,
            Some(_) => Activity::Background,
            None => Activity::Unknown,
        };
        if crate::system_search::native::identity(self.identity.pid) != Some(self.identity) {
            return None;
        }
        Some(Snapshot {
            application: Observation {
                application: BUNDLE.into(),
                process: self.identity,
                activity,
            },
            sampled_at: now,
            // Qualified solely by this owned executable: no capture/media APIs,
            // no helper process creation, no document loading or plugins.
            running: Signal::Inactive,
            input: Signal::Inactive,
            output: Signal::Inactive,
            recording: Signal::Inactive,
            helpers: Helpers::Complete,
        })
    }
    fn request(
        &mut self,
        tracker: &Tracker,
        guard: &Guard,
        due: &Due,
        now: Duration,
        preferences: &BTreeMap<String, AutoQuitSettings>,
    ) -> bool {
        if self.attempted.contains(due) {
            return false;
        }
        // No cached snapshot is an effect authority. Reacquire live exact-PID
        // identity, executable/bundle and foreground status immediately here.
        let Some(snapshot) = self.snapshot(now) else {
            return false;
        };
        if guard.assess(tracker, due, now, preferences, Some(&snapshot)) != Decision::OwnedFixture {
            return false;
        }
        self.attempted.push(due.clone());
        // Normal public AppKit request; true is not proof of application exit.
        self.app.terminate()
    }
    fn receipt_has_quit(&self) -> bool {
        fs::read_to_string(&self.receipt).unwrap().contains("quit:")
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(mut input) = self.child.stdin.take() {
            let _ = input.write_all(b"x");
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        // Child has its own finite watchdog; never escalate to force-kill.
    }
}
#[test]
#[ignore = "owned native app proof requires explicit compiled fixture and isolated absolute directory"]
fn native_owned_app_accepts_refuses_and_defers_normal_auto_quit() {
    let _serial = OWNED_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert!(std::env::var_os("DYLD_INSERT_LIBRARIES").is_none());
    // Test threads are not main; only the child owns AppKit application state.
    assert!(MainThreadMarker::new().is_none());
    let root = PathBuf::from(
        std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_DIR").expect("explicit owned directory"),
    );
    let binary = PathBuf::from(
        std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_BIN").expect("explicit compiled fixture"),
    );
    assert!(root.is_absolute() && binary.is_absolute());
    fs::create_dir_all(&root).unwrap();
    for mode in ["accept", "refuse", "defer", "crash"] {
        let mut owned = Owned::launch(&root, &binary, mode);
        let prefs = BTreeMap::from([(
            BUNDLE.into(),
            AutoQuitSettings {
                allow_unverified_activity: false,
                enabled: true,
                minutes: 1,
            },
        )]);
        let mut tracker = Tracker::default();
        let snapshot = owned
            .snapshot(Duration::ZERO)
            .expect("live owned process binding");
        assert_eq!(snapshot.application.activity, Activity::Background);
        let guard = owned_fixture_guard(&snapshot.application);
        tracker.observe(
            Duration::ZERO,
            std::slice::from_ref(&snapshot.application),
            &prefs,
        );
        let now = Duration::from_secs(60);
        let snapshot = owned.snapshot(now).unwrap();
        let assessments = tracker.observe_guarded(
            &guard,
            now,
            std::slice::from_ref(&snapshot.application),
            &prefs,
            std::slice::from_ref(&snapshot),
        );
        assert_eq!(assessments.len(), 1);
        assert_eq!(assessments[0].decision, Decision::OwnedFixture);
        let mut due = assessments[0].candidate.clone();
        let mut disabled = prefs.clone();
        disabled.get_mut(BUNDLE).unwrap().enabled = false;
        assert!(!owned.request(&tracker, &guard, &due, now, &disabled));
        assert!(!owned.receipt_has_quit());
        let mut stale = due.clone();
        stale.process.seconds += 1;
        assert!(!owned.request(&tracker, &guard, &stale, now, &prefs));
        // Synthetic foreground transition on the real owned process, without
        // activating any window or taking focus. It retires the old deadline.
        let mut foreground = snapshot.application.clone();
        foreground.activity = Activity::Foreground;
        tracker.observe(now, &[foreground], &prefs);
        assert!(!owned.request(&tracker, &guard, &due, now, &prefs));
        assert!(tracker
            .observe(now, std::slice::from_ref(&snapshot.application), &prefs)
            .is_empty());
        let now = now + Duration::from_secs(60);
        due = tracker
            .observe(now, std::slice::from_ref(&snapshot.application), &prefs)
            .pop()
            .unwrap();
        assert!(!owned.receipt_has_quit());
        assert!(owned.request(&tracker, &guard, &due, now, &prefs));
        assert!(
            !owned.request(&tracker, &guard, &due, now, &prefs),
            "never repeat a refused/deferred attempt"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while !owned.receipt_has_quit() {
            assert!(
                Instant::now() < deadline,
                "normal quit never reached delegate"
            );
            thread::sleep(Duration::from_millis(10));
        }
        if mode == "accept" || mode == "crash" {
            let exit = loop {
                if let Some(exit) = owned.child.try_wait().unwrap() {
                    break exit;
                }
                assert!(Instant::now() < deadline, "accepted app did not exit");
                thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(
                exit.success(),
                mode == "accept",
                "receipt alone cannot prove clean termination: {exit}"
            );
            assert!(!owned.request(&tracker, &guard, &due, now, &prefs));
            assert!(
                owned.snapshot(now).is_none(),
                "dead process cannot be observed again"
            );
        } else {
            thread::sleep(Duration::from_millis(150));
            assert!(
                owned.child.try_wait().unwrap().is_none(),
                "refusal/deferred save decision must survive"
            );
            assert_eq!(
                fs::read_to_string(&owned.receipt).unwrap().lines().count(),
                2
            );
        }
        let outcome = match mode {
            "accept" => "clean-exit",
            "refuse" => "refused-alive",
            "defer" => "deferred-alive",
            "crash" => "rejected-nonzero-exit",
            _ => unreachable!(),
        };
        println!("owned-app {mode}: normal quit received; outcome={outcome}");
    }
}

#[test]
#[ignore = "explicit owned native runtime proof; requires compiled fixture and isolated paths"]
fn product_runtime_quits_only_the_consented_owned_app() {
    let _serial = OWNED_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use super::runtime::{Adapter, Preferences, Runtime};
    use crate::profile::{ConfigStore, ProfilePaths};
    struct Clocked {
        native: super::native::Native,
        now: Duration,
    }
    impl Adapter for Clocked {
        fn now(&self) -> Duration {
            self.now
        }
        fn preferences(&mut self) -> Result<Preferences, String> {
            self.native.preferences()
        }
        fn applications(&mut self, p: &Preferences) -> Vec<Observation> {
            self.native.applications(p)
        }
        fn snapshot(&mut self, due: &Due) -> Option<Snapshot> {
            let mut value = self.native.snapshot(due)?;
            value.sampled_at = self.now;
            Some(value)
        }
        fn request(&mut self, due: &Due, p: &AutoQuitSettings) -> bool {
            self.native.request(due, p)
        }
    }
    let root = PathBuf::from(std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_DIR").unwrap())
        .join("product-runtime");
    fs::create_dir_all(&root).unwrap();
    let binary = PathBuf::from(std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_BIN").unwrap());
    for mode in ["accept", "refuse", "defer"] {
        let mut owned = Owned::launch(&root, &binary, mode);
        let profile = root.join(format!("profile-{}-{mode}", std::process::id()));
        let paths = ProfilePaths::new(profile.clone(), profile.clone(), None);
        let mut store = ConfigStore::open(paths.clone()).unwrap();
        let mut config = rustcast_core::default_config();
        config["autoQuit"] = serde_json::json!({BUNDLE:{"enabled":true,"minutes":1}});
        store.save(&config).unwrap();
        let mut adapter = Clocked {
            native: super::native::Native::owned_test(
                paths,
                owned.identity,
                owned.executable.clone(),
            ),
            now: Duration::ZERO,
        };
        let mut runtime = Runtime::default();
        assert_eq!(runtime.tick(&mut adapter).unwrap(), 0);
        adapter.now = Duration::from_secs(60);
        assert_eq!(
            runtime.tick(&mut adapter).unwrap(),
            0,
            "enabled alone is not consent"
        );
        assert!(!owned.receipt_has_quit());
        config["autoQuit"][BUNDLE]["allowUnverifiedActivity"] = serde_json::json!(true);
        store.save(&config).unwrap();
        assert_eq!(
            runtime.tick(&mut adapter).unwrap(),
            0,
            "consent starts a fresh interval"
        );
        adapter.now = Duration::from_secs(120);
        if mode == "accept" {
            let effect_paths = store.paths.clone();
            adapter.native.before_effect(move || {
                let mut writer = ConfigStore::open_existing(effect_paths.clone()).unwrap();
                let mut disabled: serde_json::Value =
                    serde_json::from_slice(writer.snapshot()).unwrap();
                disabled["autoQuit"][BUNDLE]["enabled"] = serde_json::json!(false);
                writer.save(&disabled).unwrap();
            });
            assert_eq!(
                runtime.tick(&mut adapter).unwrap(),
                0,
                "disable published during final sensor queries must win"
            );
            assert!(!owned.receipt_has_quit());
            assert_eq!(runtime.tick(&mut adapter).unwrap(), 0);
            store.reload().unwrap();
            store.save(&config).unwrap();
            assert_eq!(runtime.tick(&mut adapter).unwrap(), 0);
            adapter.now = Duration::from_secs(180);
        }
        let result = runtime.tick(&mut adapter).unwrap();
        let preferences = adapter.preferences().unwrap();
        let observations = adapter.applications(&preferences);
        assert_eq!(
            result, 1,
            "consented native runtime must send normal quit: observations={observations:?}"
        );
        assert_eq!(
            runtime.tick(&mut adapter).unwrap(),
            0,
            "no repeat after refusal or deferred save"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while !owned.receipt_has_quit() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        if mode == "accept" {
            let exit = loop {
                if let Some(exit) = owned.child.try_wait().unwrap() {
                    break exit;
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            };
            assert!(exit.success());
        } else {
            thread::sleep(Duration::from_millis(100));
            assert!(owned.child.try_wait().unwrap().is_none());
        }
        println!("product runtime {mode}: consent required, fresh timer, normal request, refusal preserved");
    }
}

#[test]
#[ignore = "explicit one-minute real monotonic/native product proof against owned child only"]
fn real_minute_product_runtime_normally_quits_owned_child() {
    let _serial = OWNED_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use crate::profile::{ConfigStore, ProfilePaths};
    let root = PathBuf::from(std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_DIR").unwrap())
        .join("real-minute");
    fs::create_dir_all(&root).unwrap();
    let binary = PathBuf::from(std::env::var_os("RUYCAST_AUTO_QUIT_FIXTURE_BIN").unwrap());
    let mut owned = Owned::launch(&root, &binary, "accept");
    let profile = root.join(format!("profile-{}", std::process::id()));
    let paths = ProfilePaths::new(profile.clone(), profile, None);
    let mut store = ConfigStore::open(paths.clone()).unwrap();
    let mut config = rustcast_core::default_config();
    config["autoQuit"] =
        serde_json::json!({BUNDLE:{"enabled":true,"minutes":1,"allowUnverifiedActivity":true}});
    store.save(&config).unwrap();
    let mut native =
        super::native::Native::owned_test(paths, owned.identity, owned.executable.clone());
    let mut runtime = super::runtime::Runtime::default();
    let start = Instant::now();
    assert_eq!(runtime.tick(&mut native).unwrap(), 0);
    loop {
        thread::sleep(Duration::from_secs(2));
        let sent = runtime.tick(&mut native).unwrap();
        if start.elapsed() < Duration::from_secs(60) {
            assert_eq!(sent, 0);
            assert!(!owned.receipt_has_quit());
        }
        if sent == 1 {
            assert!(start.elapsed() >= Duration::from_secs(60));
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(66),
            "real native timer did not send normal quit"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let exit = loop {
        if let Some(exit) = owned.child.try_wait().unwrap() {
            break exit;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert!(owned.receipt_has_quit());
    assert!(exit.success());
    println!(
        "real product timer: normal owned-child quit after {:.2}s, clean exit",
        start.elapsed().as_secs_f64()
    );
}
