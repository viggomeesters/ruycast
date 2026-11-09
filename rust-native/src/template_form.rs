//! Worker-prepared immutable field metadata. One worker and one replaceable
//! desired request per window; no AppKit handles, pasteboard reads or effects.
use crate::{
    control::HostEvent,
    templates::{Field, Plan},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc, Condvar, Mutex,
};
use std::thread::JoinHandle;

#[derive(Clone, Debug)]
pub struct Fields(
    pub Arc<Vec<Field>>,
    Arc<Vec<std::collections::HashMap<String, usize>>>,
);
impl Fields {
    pub fn new(fields: Vec<Field>) -> Self {
        let indices = fields
            .iter()
            .map(|field| {
                field
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, text)| {
                        let key = objc2::rc::autoreleasepool(|_| {
                            objc2_foundation::NSString::from_str(text)
                                .precomposedStringWithCanonicalMapping()
                                .to_string()
                        });
                        (key, i)
                    })
                    .collect()
            })
            .collect();
        Self(Arc::new(fields), Arc::new(indices))
    }
    pub fn option_index(&self, field: usize, canonical_key: &str) -> Option<usize> {
        self.1.get(field)?.get(canonical_key).copied()
    }
}
impl PartialEq for Fields {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
struct Shared {
    stop: AtomicBool,
    state: Mutex<State>,
    wake: Condvar,
}
#[derive(Default)]
struct State {
    generation: u64,
    active: bool,
    desired: Option<(u64, rustcast_core::Command)>,
    ready: Option<(u64, Result<Fields, String>)>,
}
pub struct Preparation {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    events: SyncSender<HostEvent>,
    #[cfg(feature = "test-tools")]
    fixture: Option<std::path::PathBuf>,
}
impl Preparation {
    #[cfg(feature = "test-tools")]
    pub(crate) fn owned_pending(&self) -> serde_json::Value {
        let state=self.shared.state.lock().unwrap_or_else(|p|p.into_inner());
        serde_json::json!({"active":state.active,"desired":state.desired.is_some(),"ready":state.ready.is_some(),"worker_retained":self.worker.is_some()})
    }
    pub fn new(events: SyncSender<HostEvent>) -> Self {
        Self {
            shared: Arc::new(Shared {
                stop: AtomicBool::new(false),
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
            }),
            worker: None,
            events,
            #[cfg(feature = "test-tools")]
            fixture: None,
        }
    }
    #[cfg(feature = "test-tools")]
    pub(crate) fn fixture(mut self, home: std::path::PathBuf) -> Self {
        self.fixture = Some(home);
        self
    }
    pub fn request(&mut self, command: rustcast_core::Command) -> Result<u64, String> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let _ = self.worker.take().expect("finished worker").join();
        }
        if self.worker.is_none() {
            let shared = self.shared.clone();
            let events = self.events.clone();
            #[cfg(feature = "test-tools")]
            let fixture = self.fixture.clone();
            self.worker = Some(
                std::thread::Builder::new()
                    .name("ruycast-template-fields".into())
                    .spawn(move || loop {
                        let (generation, command) = {
                            let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
                            while state.desired.is_none() && !shared.stop.load(Ordering::Acquire) {
                                state = shared.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                            }
                            if shared.stop.load(Ordering::Acquire) {
                                break;
                            }
                            state.desired.take().expect("desired field request")
                        };
                        let result = std::panic::catch_unwind(|| {
                            #[cfg(feature = "test-tools")]
                            if let Some(home) = &fixture {
                                fixture_gate(home, &shared, generation)?;
                            }
                            Plan::parse(&command, crate::templates::Limits::application())
                                .map(|plan| Fields::new(plan.fields().to_vec()))
                        })
                        .unwrap_or_else(|_| Err("Template fields could not be prepared.".into()));
                        let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
                        if shared.stop.load(Ordering::Acquire) {
                            break;
                        }
                        if state.active && state.generation == generation {
                            state.ready = Some((generation, result));
                            drop(state);
                            let _ = events.try_send(HostEvent::UiWake);
                        }
                    })
                    .map_err(|_| "Template preparation could not start.".to_string())?,
            );
        }
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or("Template sequence exhausted.")?;
        let generation = state.generation;
        state.active = true;
        state.ready = None;
        state.desired = Some((generation, command));
        self.shared.wake.notify_one();
        Ok(generation)
    }
    pub fn take(&self) -> Option<(u64, Result<Fields, String>)> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .ready
            .take()
    }
    pub fn cancel(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.generation = state.generation.saturating_add(1);
        state.active = false;
        state.desired = None;
        state.ready = None;
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        {
            let _state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
            self.shared.stop.store(true, Ordering::Release);
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take().filter(|w| w.is_finished()) {
            let _ = worker.join();
        }
    }
}

#[cfg(feature = "test-tools")]
fn explicit_fixture_boundary(home: &std::path::Path, parent: &std::path::Path) -> bool {
    use std::{io::Read, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    let uid = unsafe { libc::geteuid() };
    let private_dir = |path: &std::path::Path| {
        path.is_absolute()
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o777 == 0o700)
            && path.canonicalize().ok().as_deref() == Some(path)
    };
    let marker = |path: &std::path::Path, expected: &[u8], private: bool| {
        let Ok(file) = std::fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path) else { return false; };
        let Ok(metadata) = file.metadata() else { return false; };
        if !metadata.is_file() || metadata.uid() != uid || metadata.nlink() != 1
            || metadata.len() != expected.len() as u64
            || metadata.mode() & (if private { 0o077 } else { 0o022 }) != 0 {
            return false;
        }
        let mut bytes = Vec::new();
        file.take(expected.len() as u64 + 1).read_to_end(&mut bytes).is_ok() && bytes == expected
    };
    home.parent() == Some(parent) && private_dir(parent) && private_dir(home)
        && marker(&parent.join(".ruycast-ui-fixture-parent"), b"owned-ui-fixture-parent-v1", true)
        && marker(&home.join(".ruycast-test-fixture"), b"own-disposable-fixture-v1", false)
}

#[cfg(feature = "test-tools")]
fn fixture_gate(home: &std::path::Path, shared: &Shared, generation: u64) -> Result<(), String> {
    if std::env::var("RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER").as_deref() != Ok("1") {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let legacy = home.starts_with(root)
        && home.canonicalize().ok().as_deref() == Some(home)
        && std::fs::read(home.join(".ruycast-test-fixture"))
            .ok().as_deref() == Some(b"own-disposable-fixture-v1");
    let explicit = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .is_some_and(|parent| explicit_fixture_boundary(home, std::path::Path::new(&parent)));
    if !legacy && !explicit
    {
        return Err("Template fixture boundary is unavailable.".into());
    }
    std::fs::write(home.join("template-fields-entered"), generation.to_string())
        .map_err(|e| e.to_string())?;
    while !home.join("template-fields-release").exists() {
        if shared.stop.load(Ordering::Acquire) {
            return Err("Template preparation stopped.".into());
        }
        {
            let state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.active || state.generation != generation {
                return Err("Template preparation retired.".into());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
}

/// Native snapshots contain each prepared field exactly once. Reject forged
/// names and resource overflow; the render worker validates options before handing values to execution.
pub fn values(fields: &Fields, supplied: &[(String, String)]) -> Result<serde_json::Value, String> {
    if fields.0.len() != supplied.len() {
        return Err("Template fields changed. Reopen the command.".into());
    }
    let mut bytes = 0usize;
    let mut result = serde_json::Map::new();
    for (field, (id, value)) in fields.0.iter().zip(supplied) {
        if &field.id != id {
            return Err("Template fields changed. Reopen the command.".into());
        }
        bytes = bytes
            .checked_add(id.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or("Input exceeds the supported size.")?;
        if bytes > isize::MAX as usize {
            return Err("Input exceeds the supported size.".into());
        }
        result.insert(id.clone(), serde_json::Value::String(value.clone()));
    }
    Ok(serde_json::Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-tools")]
    #[test]
    fn explicit_private_fixture_parent_reaches_barrier_without_target_ancestry() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if std::env::var_os("RUYCAST_TEMPLATE_BOUNDARY_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["template_form::tests::explicit_private_fixture_parent_reaches_barrier_without_target_ancestry", "--exact", "--nocapture"])
                .env("RUYCAST_TEMPLATE_BOUNDARY_CHILD", "1")
                .env("RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER", "1")
                .output().unwrap();
            assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let parent = std::path::PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned fixture parent"),
        );
        assert_eq!(std::env::var("RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER").as_deref(), Ok("1"));
        let home = parent.join(format!("template-boundary-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
        }
        let _cleanup = Cleanup(home.clone());
        let marker = home.join(".ruycast-test-fixture");
        std::fs::write(&marker, b"own-disposable-fixture-v1").unwrap();
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(home.join("template-fields-release"), b"release").unwrap();
        let shared = Shared {
            stop: AtomicBool::new(false),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        };
        let result = fixture_gate(&home, &shared, 7);
        assert!(result.is_ok(), "explicit owned boundary refused: {result:?}");
        assert_eq!(std::fs::read(home.join("template-fields-entered")).unwrap(), b"7");

        let scope = home.join("scope");
        std::fs::DirBuilder::new().mode(0o700).create(&scope).unwrap();
        let child = scope.join("child");
        std::fs::DirBuilder::new().mode(0o700).create(&child).unwrap();
        let parent_marker = scope.join(".ruycast-ui-fixture-parent");
        let child_marker = child.join(".ruycast-test-fixture");
        std::fs::write(&parent_marker, b"owned-ui-fixture-parent-v1").unwrap();
        std::fs::set_permissions(&parent_marker, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&child_marker, b"own-disposable-fixture-v1").unwrap();
        assert!(explicit_fixture_boundary(&child, &scope));
        assert!(!explicit_fixture_boundary(&child, &home), "wrong selected root");
        assert!(!explicit_fixture_boundary(&scope, &scope), "parent itself is not a child");
        std::fs::remove_file(&parent_marker).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "missing parent marker");
        std::fs::write(&parent_marker, b"owned-ui-fixture-parent-v1\n").unwrap();
        std::fs::set_permissions(&parent_marker, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "marker newline must not pass");
        std::fs::write(&parent_marker, b"owned-ui-fixture-parent-v1").unwrap();
        for directory in [&scope, &child] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(!explicit_fixture_boundary(&child, &scope), "nonprivate directory");
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::set_permissions(&parent_marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "nonprivate parent marker");
        std::fs::set_permissions(&parent_marker, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&child_marker, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "writable child marker");
        std::fs::set_permissions(&child_marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        let linked = scope.join("linked-marker");
        std::fs::hard_link(&child_marker, &linked).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "multiply linked marker");
        std::fs::remove_file(&linked).unwrap();
        std::fs::rename(&child_marker, &linked).unwrap();
        std::os::unix::fs::symlink(&linked, &child_marker).unwrap();
        assert!(!explicit_fixture_boundary(&child, &scope), "symlink marker");
        std::fs::remove_file(&child_marker).unwrap();
        std::fs::rename(&linked, &child_marker).unwrap();
        let alias = home.join("scope-alias");
        std::os::unix::fs::symlink(&scope, &alias).unwrap();
        assert!(!explicit_fixture_boundary(&alias.join("child"), &alias), "symlink ancestor");
        assert!(explicit_fixture_boundary(&child, &scope), "restored owned boundary");

        let legacy = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target")
            .join(format!("template-legacy-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&legacy).unwrap();
        let _legacy_cleanup = Cleanup(legacy.clone());
        std::fs::write(legacy.join(".ruycast-test-fixture"), b"own-disposable-fixture-v1").unwrap();
        std::fs::write(legacy.join("template-fields-release"), b"release").unwrap();
        assert!(fixture_gate(&legacy, &shared, 8).is_ok(), "legacy target route preserved");
        assert_eq!(std::fs::read(legacy.join("template-fields-entered")).unwrap(), b"8");
    }
    fn command(target: &str) -> rustcast_core::Command {
        serde_json::from_value(serde_json::json!({
            "id":"fixture", "title":"Fixture", "kind":"url", "target":target
        }))
        .unwrap()
    }
    #[test]
    fn only_latest_request_can_publish_and_cancel_retires_ready_metadata() {
        let (events, _) = std::sync::mpsc::sync_channel(1);
        let mut preparation = Preparation::new(events);
        preparation
            .request(command("https://example.invalid/{old}"))
            .unwrap();
        let latest = preparation
            .request(command("https://example.invalid/{current}"))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let fields = loop {
            if let Some((generation, fields)) = preparation.take() {
                assert_eq!(generation, latest);
                break fields.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "metadata worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!(fields.0[0].id, "current");
        preparation.cancel();
        assert!(preparation.take().is_none());
    }
    #[test]
    fn snapshot_schema_budget_and_canonical_option_lookup_are_bound_to_metadata() {
        let fields = Fields::new(vec![Field {
            id: "place".into(),
            label: "Place".into(),
            default_value: String::new(),
            options: vec!["cafe\u{301}".into(), "Other".into()],
        }]);
        assert_eq!(fields.option_index(0, "café"), Some(0));
        assert_eq!(fields.option_index(1, "café"), None);
        assert!(values(&fields, &[]).is_err());
        assert!(values(&fields, &[("forged".into(), "x".into())]).is_err());
        assert!(values(&fields, &[("place".into(), "x".repeat(1_048_576))]).is_ok());
        assert_eq!(
            values(&fields, &[("place".into(), "cafe\u{301}".into())]).unwrap()["place"],
            "cafe\u{301}"
        );
    }
    #[test]
    fn shutdown_wakes_an_idle_metadata_worker() {
        let (events, _) = std::sync::mpsc::sync_channel(1);
        let mut preparation = Preparation::new(events);
        preparation
            .request(command("https://example.invalid"))
            .unwrap();
        let shared = preparation.shared.clone();
        let worker = preparation.worker.take().unwrap();
        drop(preparation);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !worker.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not observe shutdown"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        worker.join().unwrap();
        assert!(shared.stop.load(Ordering::Acquire));
    }
}
