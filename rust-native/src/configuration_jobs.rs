//! One physical background writer. The serialized owner keeps its last accepted
//! snapshot and harvests data-only receipts; no SDK wait occurs on that owner.
use crate::{
    control::{HostEvent, ProfileLease},
    execution::Live,
    profile::ConfigStore,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc, Mutex,
};
/// Shared storage truth survives owner delivery, transport expiry and shutdown.
/// Entry linearizes with retirement; an entered OS write is conservatively
/// uncertain until its actual write_outcome returns.
#[derive(Default)]
pub struct Publication(Mutex<PublicationState>);
#[derive(Default)]
struct PublicationState {
    retired: bool,
    writing: bool,
    published: bool,
    retention_writing: bool,
    retention_published: bool,
    retention_warning: Option<String>,
}
impl Publication {
    pub(crate) fn enter_retention(&self, live: impl FnOnce() -> bool) -> Result<(), String> {
        let mut state = self.0.lock().unwrap_or_else(|p|p.into_inner());
        if state.retired || !live() { return Err("Configuration request ended before clipboard publication.".into()); }
        state.retention_writing = true;
        Ok(())
    }
    pub(crate) fn complete_retention(&self, published: bool, warning: Option<String>) {
        let mut state = self.0.lock().unwrap_or_else(|p|p.into_inner());
        state.retention_published |= published;
        state.retention_writing = false;
        // Completion is storage truth, even after transport/actor retirement.
        if published { state.retention_warning = warning; }
    }
    pub fn enter(&self, live: impl FnOnce() -> bool) -> Result<(), String> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.retired || !live() {
            return Err("Configuration update was cancelled before publication.".into());
        }
        state.writing = true;
        Ok(())
    }
    pub fn complete(&self, published: bool) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.published |= published;
        state.writing = false;
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        serde_json::json!({"published":state.published,"outcomeUncertain":state.writing})
    }
    pub fn retire(&self) -> serde_json::Value {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.retired = true;
        let mut response = serde_json::json!({"ok":false,
            "error":if state.writing || state.published {
                "Configuration may have been saved. Reload it before trying again."
            } else { "Configuration request ended. Reload configuration to confirm its state." },
            "configurationWrite":{"published":state.published,"outcomeUncertain":state.writing},
            "configurationReconciliationRequired":true});
        if state.retention_writing || state.retention_published || state.retention_warning.is_some() {
            response["clipboardWrite"] = serde_json::json!({"published":state.retention_published,"outcomeUncertain":state.retention_writing});
            response["clipboardWarning"] = serde_json::json!(state.retention_warning);
            response["clipboardReconciliationRequired"] = serde_json::json!(true);
        }
        response
    }
}
pub struct Completion<T> {
    pub store: ConfigStore,
    pub result: Result<T, String>,
    pub published: bool,
    pub uncertain: bool,
    /// Physical clipboard publication truth, independent of delivery/cancellation.
    pub retention: Option<crate::profile::SaveReceipt>,
}
struct Job<T> {
    result: Arc<Mutex<Option<Completion<T>>>>,
    thread: std::thread::JoinHandle<()>,
}
pub struct Writer<T> {
    job: Option<Job<T>>,
    stop: Arc<AtomicBool>,
    events: SyncSender<HostEvent>,
    profile: ProfileLease,
    keyboard: Option<crate::keyboard_service::Client>,
    retention_pending: Arc<AtomicBool>,
    capture_busy: Option<Arc<AtomicBool>>,
}
struct NoopStage(Live);
struct RetentionStage {
    inner: Box<dyn crate::config_effects::Stage>,
    root: std::path::PathBuf,
    settings: rustcast_core::ClipboardSettings,
    live: Live,
    receipt: Arc<Mutex<Option<crate::profile::SaveReceipt>>>,
    pending: Arc<AtomicBool>,
    capture_busy: Option<Arc<AtomicBool>>,
}
impl crate::config_effects::Stage for RetentionStage {
    fn ready(&self) -> Result<(), String> { self.inner.ready() }
    fn before_adopt(&mut self, publication: Option<Arc<Publication>>) -> Result<(), String> {
        self.inner.before_adopt(publication.clone())?;
        let was_pending = self.pending.swap(true, Ordering::SeqCst);
        // Admission handshake: a previously accepted capture finishes on its
        // ordinary owner. No storage/main lock is held while this worker waits.
        while self.capture_busy.as_ref().is_some_and(|p|p.load(Ordering::SeqCst)) {
            if !(self.live)() {
                self.pending.store(was_pending, Ordering::SeqCst);
                return Err("Configuration update was cancelled while waiting for clipboard capture.".into());
            }
            std::thread::park_timeout(std::time::Duration::from_millis(2));
        }
        let receipt = match crate::clipboard_history::Store::retain_metadata(&self.root, &self.settings, || (self.live)(), publication) {
            Ok(receipt) => receipt,
            Err(error) => { self.pending.store(was_pending, Ordering::SeqCst); return Err(error); }
        };
        // Never discard a returned post-rename warning because the transport or
        // actor retired during the physical write.
        *self.receipt.lock().unwrap_or_else(|e| e.into_inner()) = Some(receipt);
        Ok(())
    }
    fn commit(&mut self) -> Result<crate::config_effects::Receipt, String> { self.inner.commit() }
}
impl crate::config_effects::Stage for NoopStage {
    fn ready(&self) -> Result<(), String> {
        if (self.0)() {
            Ok(())
        } else {
            Err("Configuration update was cancelled before publication.".into())
        }
    }
    fn commit(&mut self) -> Result<crate::config_effects::Receipt, String> {
        Ok(Default::default())
    }
}
impl<T: Send + 'static> Writer<T> {
    pub fn new(
        profile: ProfileLease,
        events: SyncSender<HostEvent>,
        keyboard: Option<crate::keyboard_service::Client>,
    ) -> Self {
        if let Some(keyboard) = &keyboard {
            keyboard.retain_profile(profile.clone());
        }
        Self {
            job: None,
            stop: Arc::new(AtomicBool::new(false)),
            events,
            profile,
            keyboard,
            retention_pending: Arc::new(AtomicBool::new(false)),
            capture_busy: None,
        }
    }
    pub(crate) fn retention_phase(&self) -> Arc<AtomicBool> { self.retention_pending.clone() }
    pub(crate) fn bind_capture(&mut self, port: &crate::clipboard_history::CapturePort) {
        assert!(self.job.is_none());
        if let Some(previous) = &self.capture_busy { assert!(Arc::ptr_eq(previous, &port.busy)); }
        port.bind_retention(self.retention_phase());
        self.capture_busy = Some(port.busy.clone());
    }
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.keyboard.as_ref().is_some_and(|k| k.pending())
    }
    pub fn active(&self) -> bool {
        self.job.is_some()
    }
    pub fn has_keyboard(&self) -> bool {
        self.keyboard.is_some()
    }
    pub fn start(
        &mut self,
        base: &ConfigStore,
        live: Live,
        force: bool,
        work: impl FnOnce(&mut ConfigStore) -> Result<T, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("Configuration writer stopped.".into());
        }
        if self.busy() {
            return Err("Configuration is being updated. Try again.".into());
        }
        #[cfg(feature = "test-tools")]
        let adoption_barrier = fixture_adoption_barrier(&base.paths)?;
        #[cfg(feature = "test-tools")]
        let barrier_stop = self.stop.clone();
        let stopped = self.stop.clone();
        let check: Live = Arc::new(move || !stopped.load(Ordering::Acquire) && live());
        let mut store = base.actor_copy();
        let before = store.publication_state().0;
        let platform_factory = if let Some(keyboard) = &self.keyboard {
            keyboard.background_factory_with_live(check.clone())
        } else {
            Arc::new({
                let check = check.clone();
                move |_: &rustcast_core::Config| {
                    Ok(Box::new(NoopStage(check.clone())) as Box<dyn crate::config_effects::Stage>)
                }
            })
        };
        let retention = Arc::new(Mutex::new(None));
        let root = base.paths.root.clone();
        let retained = retention.clone();
        let stage_live = check.clone();
        let pending = self.retention_pending.clone();
        let capture_busy = self.capture_busy.clone();
        let factory: crate::config_effects::Factory = Arc::new(move |candidate| {
            Ok(Box::new(RetentionStage {
                inner: platform_factory(candidate)?, root: root.clone(),
                settings: candidate.clipboard.clone(), live: stage_live.clone(),
                receipt: retained.clone(),
                pending: pending.clone(),
                capture_busy: capture_busy.clone(),
            }))
        });
        store.configure_writer_effects(factory, force, check.clone());
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        let events = self.events.clone();
        let lease = self.profile.clone();
        let thread = std::thread::Builder::new()
            .name("ruycast-config-writer".into())
            .spawn(move || {
                let _lease = lease;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if !check() {
                        return Err("Configuration update was cancelled.".into());
                    }
                    work(&mut store)
                }))
                .unwrap_or_else(|_| {
                    Err(
                        "Configuration writer failed. Reload configuration before trying again."
                            .into(),
                    )
                });
                #[cfg(feature = "test-tools")]
                let outcome = if let Some(root) = adoption_barrier {
                    if outcome.is_ok() {
                        wait_fixture_adoption(&root, &barrier_stop).and(outcome)
                    } else {
                        outcome
                    }
                } else {
                    outcome
                };
                let (writes, uncertain) = store.publication_state();
                *output.lock().unwrap_or_else(|e| e.into_inner()) = Some(Completion {
                    store: store.actor_copy(),
                    result: outcome,
                    published: writes != before,
                    uncertain,
                    retention: retention.lock().unwrap_or_else(|e| e.into_inner()).take(),
                });
                let _ = events.try_send(HostEvent::UiWake);
            })
            .map_err(|e| e.to_string())?;
        self.job = Some(Job { result, thread });
        Ok(())
    }
    pub fn take(&mut self) -> Option<Completion<T>> {
        if !self.job.as_ref()?.thread.is_finished() {
            return None;
        }
        let job = self.job.take().expect("finished writer");
        let _ = job.thread.join();
        let output = job.result.lock().unwrap_or_else(|e| e.into_inner()).take();
        output
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(keyboard) = &self.keyboard {
            keyboard.stop();
        }
    }
}
#[cfg(feature = "test-tools")]
fn fixture_adoption_barrier(
    paths: &crate::profile::ProfilePaths,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(value) = std::env::var_os("RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER") else {
        return Ok(None);
    };
    let root = std::path::PathBuf::from(value)
        .canonicalize()
        .map_err(|_| "Own config adoption fixture required")?;
    let home = paths
        .home
        .canonicalize()
        .map_err(|_| "Own config adoption profile required")?;
    let profile = paths
        .root
        .canonicalize()
        .map_err(|_| "Own config adoption profile required")?;
    if home != root
        || profile.parent() != Some(root.as_path())
        || std::fs::read(root.join(".ruycast-test-fixture"))
            .ok()
            .as_deref()
            != Some(b"own-disposable-fixture-v1")
    {
        return Err("Own config adoption fixture required".into());
    }
    Ok(root.join("hold-config-adoption").exists().then_some(root))
}
#[cfg(feature = "test-tools")]
fn wait_fixture_adoption(root: &std::path::Path, stopped: &AtomicBool) -> Result<(), String> {
    std::fs::write(root.join("config-adoption-entered"), b"entered").map_err(|e| e.to_string())?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.join("config-adoption-release").exists() {
        if stopped.load(Ordering::Acquire) || std::time::Instant::now() >= until {
            return Err("Own config adoption fixture ended".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
}
impl<T> Drop for Writer<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(keyboard) = &self.keyboard {
            keyboard.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlServer, Limits};
    use std::{
        path::PathBuf,
        sync::mpsc,
        sync::atomic::AtomicU64,
        time::{Duration, Instant},
    };
    #[test]
    fn save_write_refusal_never_enters_retention_or_prunes_history_assets() {
        use std::os::unix::fs::PermissionsExt;
        struct ReadOnly {
            root: PathBuf,
            calls: Arc<AtomicU64>,
        }
        impl crate::config_effects::Stage for ReadOnly {
            fn ready(&self) -> Result<(), String> {
                std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o500)).unwrap();
                Ok(())
            }
            fn before_adopt(&mut self, _: Option<Arc<Publication>>) -> Result<(), String> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            fn commit(&mut self) -> Result<crate::config_effects::Receipt, String> { panic!("unpublished SAVE must not commit") }
        }
        impl Drop for ReadOnly {
            fn drop(&mut self) { std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700)).unwrap(); }
        }
        let temp = Temp::new();
        let mut config = temp.store();
        let root = config.paths.root.clone();
        let asset = root.join("clipboard-assets/11111111-1111-4111-8111-111111111111.png");
        std::fs::create_dir(asset.parent().unwrap()).unwrap();
        let png = [137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,2,0,0,0,2,8,6,0,0,0,114,182,13,36,0,0,0,17,73,68,65,84,120,156,99,104,112,80,248,15,194,12,48,6,0,64,180,7,125,193,185,247,76,0,0,0,0,73,69,78,68,174,66,96,130];
        std::fs::write(&asset, png).unwrap();
        let clips: Vec<rustcast_core::Clip> = serde_json::from_value(serde_json::json!([
            {"id":"raycast-keep","text":"Keep","date":crate::clipboard_history::now(),"source":"fixture"},
            {"id":"raycast-image","text":"Image","date":crate::clipboard_history::now(),"source":"fixture","contentType":"image","dataFile":"clipboard-assets/11111111-1111-4111-8111-111111111111.png"}
        ])).unwrap();
        let history = serde_json::to_vec_pretty(&clips).unwrap();
        std::fs::write(root.join("clipboard.json"), &history).unwrap();
        let before = config.snapshot().to_vec();
        let calls = Arc::new(AtomicU64::new(0));
        let invoked = calls.clone();
        let receipts = Arc::new(Mutex::new(None));
        let output = receipts.clone();
        let stage_root = root.clone();
        config.configure_writer_effects(Arc::new(move |candidate| Ok(Box::new(RetentionStage {
            inner: Box::new(ReadOnly { root: stage_root.clone(), calls: invoked.clone() }),
            root: stage_root.clone(), settings: candidate.clipboard.clone(), live: Arc::new(||true), receipt: output.clone(), pending: Arc::new(AtomicBool::new(false)), capture_busy: None,
        }))), false, Arc::new(||true));
        let mut candidate: serde_json::Value = serde_json::from_slice(&before).unwrap();
        candidate["clipboard"]["limit"] = serde_json::json!(1);
        let error = config.save(&candidate).unwrap_err();
        assert!(error.to_lowercase().contains("permission denied"), "actual config write must fail: {error}");
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(receipts.lock().unwrap().is_none());
        assert_eq!(config.snapshot(), before);
        assert_eq!(std::fs::read(&config.file).unwrap(), before);
        assert_eq!(std::fs::read(root.join("clipboard.json")).unwrap(), history);
        assert_eq!(std::fs::read(asset).unwrap(), png);
    }
    #[test]
    fn retention_publication_retirement_preserves_independent_storage_warning() {
        let publication = Arc::new(Publication::default());
        publication.enter_retention(||true).unwrap();
        let timed = publication.retire();
        assert_eq!(timed["clipboardWrite"], serde_json::json!({"published":false,"outcomeUncertain":true}));
        assert_eq!(timed["configurationWrite"], serde_json::json!({"published":false,"outcomeUncertain":false}));
        assert!(publication.enter_retention(||true).is_err());
        // Physical completion wins over delivery retirement, including warning.
        publication.complete_retention(true, Some("own directory-sync warning".into()));
        let completed = publication.retire();
        assert_eq!(completed["clipboardWrite"], serde_json::json!({"published":true,"outcomeUncertain":false}));
        assert_eq!(completed["clipboardWarning"], "own directory-sync warning");
        assert_eq!(completed["configurationWrite"], timed["configurationWrite"]);
        let retry = Publication::default();
        retry.enter_retention(||true).unwrap();
        retry.complete_retention(true, Some("own earlier published warning".into()));
        retry.enter_retention(||true).unwrap();
        retry.complete_retention(false, None);
        assert_eq!(retry.retire()["clipboardWarning"], "own earlier published warning");
    }
    #[test]
    fn actual_retention_postrename_warning_survives_retirement_and_stage_drop() {
        use crate::config_effects::Stage as _;
        use std::{io::Write, os::unix::fs::PermissionsExt, process::{Command, Stdio}};
        const CHILD: &str = "RUYCAST_RETENTION_WARNING_ROOT";
        const NAME: &str = "configuration_jobs::tests::actual_retention_postrename_warning_survives_retirement_and_stage_drop";
        if let Some(base) = std::env::var_os(CHILD).map(PathBuf::from) {
            assert!(base.is_absolute());
            assert_eq!(std::fs::read(base.join(".ruycast-test-fixture")).unwrap(), b"own-disposable-fixture-v1");
            let root = base.join("p");
            std::fs::create_dir(&root).unwrap();
            let assets = root.join("clipboard-assets");
            std::fs::create_dir(&assets).unwrap();
            let asset = assets.join("11111111-1111-4111-8111-111111111111.png");
            std::fs::write(&asset, b"owned retained asset sentinel").unwrap();
            let clips: Vec<rustcast_core::Clip> = serde_json::from_value(serde_json::json!([
                {"id":"raycast-keep","text":"Keep","date":crate::clipboard_history::now(),"source":"fixture"},
                {"id":"raycast-image","text":"Image","date":crate::clipboard_history::now(),"source":"fixture","contentType":"image","dataFile":"clipboard-assets/11111111-1111-4111-8111-111111111111.png"}
            ])).unwrap();
            std::fs::write(root.join("clipboard.json"), serde_json::to_vec_pretty(&clips).unwrap()).unwrap();
            let publication = Arc::new(Publication::default());
            let pin = publication.clone();
            let receipt = Arc::new(Mutex::new(None));
            let output = receipt.clone();
            let pending = Arc::new(AtomicBool::new(false));
            let mut settings = rustcast_core::validate(&rustcast_core::default_config()).unwrap().clipboard;
            settings.limit = 1;
            let stage = RetentionStage { inner: Box::new(NoopStage(Arc::new(||true))), root: root.clone(), settings, live: Arc::new(||true), receipt: output, pending: pending.clone(), capture_busy: None };
            let worker = std::thread::spawn(move || { let mut stage = stage; stage.before_adopt(Some(pin)).unwrap(); });
            let until = Instant::now() + Duration::from_secs(2);
            while !base.join("entered").exists() {
                assert!(!worker.is_finished());
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("clipboard.json")).unwrap()).unwrap();
            assert_eq!(disk.as_array().unwrap().len(), 1, "rename must already have published metadata");
            assert!(asset.exists());
            let timed = publication.retire();
            assert_eq!(timed["clipboardWrite"]["outcomeUncertain"], true);
            assert_eq!(timed["configurationWrite"]["published"], false);
            std::fs::write(base.join("release"), b"own release").unwrap();
            worker.join().unwrap();
            assert!(!receipt.lock().unwrap().as_ref().unwrap().durable);
            assert!(receipt.lock().unwrap().as_ref().unwrap().warning.is_some());
            let completed = publication.retire();
            assert_eq!(completed["clipboardWrite"], serde_json::json!({"published":true,"outcomeUncertain":false}));
            assert!(completed["clipboardWarning"].is_string());
            assert_eq!(completed["configurationWrite"]["published"], false);
            assert!(pending.load(Ordering::Acquire));
            assert!(asset.exists(), "uncertain metadata must never authorize deleting assets");
            assert!(publication.enter_retention(||true).is_err());
            std::fs::write(base.join("passed"), b"actual-postrename-warning-retained").unwrap();
            return;
        }
        let temp = Temp::new();
        std::fs::set_permissions(&temp.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(temp.0.join(".ruycast-test-fixture"), b"own-disposable-fixture-v1").unwrap();
        let shim = crate::test_fixtures::fcntl(&temp.0.join("fixture-build.log"));
        let log = temp.0.join("child.log");
        let file = std::fs::File::create(&log).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap()).env_clear().args(["--exact",NAME,"--nocapture","--test-threads=1"])
            .env(CHILD, &temp.0).env("HOME", &temp.0).env("TMPDIR", &temp.0).env("DYLD_INSERT_LIBRARIES", &shim)
            .env("RUYCAST_TEST_FSYNC_DIRECTORY", temp.0.join("p")).env("RUYCAST_TEST_FSYNC_MARKER", temp.0.join("entered")).env("RUYCAST_TEST_FSYNC_RELEASE", temp.0.join("release"))
            .stdout(Stdio::from(file.try_clone().unwrap())).stderr(Stdio::from(file)).spawn().unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() { break status; }
            if Instant::now() >= until { child.kill().unwrap(); child.wait().unwrap(); panic!("own warning child timed out: {}", std::fs::read_to_string(&log).unwrap()); }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "own warning child {status}: {}", std::fs::read_to_string(log).unwrap());
        assert_eq!(std::fs::read(temp.0.join("passed")).unwrap(), b"actual-postrename-warning-retained");
        writeln!(std::io::stdout(), "actual metadata post-rename warning survived retirement and stage drop").unwrap();
    }
    #[test]
    fn retention_owner_rebases_capture_and_defers_assets_until_actual_adoption() {
        let temp = Temp::new();
        let config = temp.store();
        let root = config.paths.root.clone();
        let old = config.current().clipboard.clone();
        let (tx, _rx) = mpsc::sync_channel(8);
        let mut control = ControlServer::start(&root, tx.clone(), Limits::default()).unwrap();
        let mut writer = Writer::new(control.profile_lease(), tx, None);
        let mut history = crate::clipboard_history::Store::open_with_retention(&root, &old, writer.retention_phase()).unwrap();
        let png = vec![137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,2,0,0,0,2,8,6,0,0,0,114,182,13,36,0,0,0,17,73,68,65,84,120,156,99,104,112,80,248,15,194,12,48,6,0,64,180,7,125,193,185,247,76,0,0,0,0,73,69,78,68,174,66,96,130];
        history.capture(crate::clipboard_history::Payload::Image(png.clone()), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
        let asset = root.join(history.items[0].data_file.as_ref().unwrap());
        history.capture(crate::clipboard_history::Payload::Text("candidate keeps this".into()), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
        let mut candidate: serde_json::Value = serde_json::from_slice(config.snapshot()).unwrap();
        candidate["clipboard"]["limit"] = serde_json::json!(1);
        std::fs::write(&config.file, serde_json::to_vec(&candidate).unwrap()).unwrap();
        let (entered, observed) = mpsc::sync_channel(1);
        let (release, held) = mpsc::sync_channel(1);
        writer.start(&config, Arc::new(||true), false, move |actor| {
            assert!(actor.reload()?);
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        }).unwrap();
        observed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(writer.retention_phase().load(Ordering::Acquire));
        assert!(asset.exists(), "metadata-only preparation must not prune");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(root.join("clipboard.json")).unwrap()).unwrap().as_array().unwrap().len(), 1);
        // This real capture is accepted by the same owner between worker
        // publication and owner adoption, with the still-old retention policy.
        history.capture(crate::clipboard_history::Payload::Text("accepted while adoption held".into()), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
        assert_eq!(history.items.len(), 2);
        assert!(asset.exists(), "concurrent capture must defer candidate-orphan cleanup");
        // Replacement owners bind the identical phase before open's maintenance.
        history = crate::clipboard_history::Store::open_with_retention(&root, &old, writer.retention_phase()).unwrap();
        assert_eq!(history.items.len(), 2);
        assert!(asset.exists(), "new owner must not prune while adoption is pending");
        let older = history.items.iter().find(|c|c.text=="candidate keeps this").unwrap().id.clone();
        assert!(history.delete(Some(&older), &old, ||true).unwrap().durable);
        assert!(history.cleanup_error.as_deref().is_some_and(|e|e.contains("pending configuration activation")));
        assert!(asset.exists(), "explicit delete must report deferred asset cleanup");
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let completed = loop {
            if let Some(completed) = writer.take() { break completed; }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(completed.result.is_ok());
        assert!(completed.retention.as_ref().unwrap().durable);
        assert!(asset.exists());
        writer.retention_phase().store(false, Ordering::Release);
        history.reconcile_settings(&completed.store.current().clipboard, false, ||true).unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].text, "accepted while adoption held");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(root.join("clipboard.json")).unwrap()).unwrap(), serde_json::to_value(&history.items).unwrap());
        assert!(!asset.exists());
        history.capture(crate::clipboard_history::Payload::Image(png), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
        let cleared_asset = root.join(history.items[0].data_file.as_ref().unwrap());
        writer.retention_phase().store(true, Ordering::Release);
        assert!(history.delete(None, &old, ||true).unwrap().durable);
        assert!(history.items.is_empty());
        assert!(cleared_asset.exists());
        assert!(history.cleanup_error.as_deref().is_some_and(|e|e.contains("pending configuration activation")));
        writer.retention_phase().store(false, Ordering::Release);
        history.reconcile_settings(&completed.store.current().clipboard, false, ||true).unwrap();
        assert!(history.cleanup_error.is_none());
        assert!(!cleared_asset.exists());
        control.stop();
    }
    #[test]
    fn capture_retention_handshake_preserves_both_orders_and_stops_without_owner_lock() {
        for mode in 0..3 {
            let temp = Temp::new();
            let config = temp.store();
            let old = config.current().clipboard.clone();
            let (tx, _rx) = mpsc::sync_channel(8);
            let mut control = ControlServer::start(&config.paths.root, tx.clone(), Limits::default()).unwrap();
            let mut writer = Writer::new(control.profile_lease(), tx, None);
            let port = crate::clipboard_history::CapturePort::default();
            writer.bind_capture(&port);
            let mut history = crate::clipboard_history::Store::open_with_retention(&config.paths.root, &old, writer.retention_phase()).unwrap();
            for text in ["owned older", "owned newer"] {
                history.capture(crate::clipboard_history::Payload::Text(text.into()), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
            }
            let before = std::fs::read(config.paths.root.join("clipboard.json")).unwrap();
            let accepted = if mode == 1 { None } else { Some(port.acquire().unwrap()) };
            let mut candidate: serde_json::Value = serde_json::from_slice(config.snapshot()).unwrap();
            candidate["clipboard"]["limit"] = serde_json::json!(1);
            std::fs::write(&config.file, serde_json::to_vec(&candidate).unwrap()).unwrap();
            writer.start(&config, Arc::new(||true), false, |actor| actor.reload()).unwrap();
            let until = Instant::now() + Duration::from_secs(3);
            while !writer.retention_phase().load(Ordering::SeqCst) {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(port.acquire().is_none(), "retention-first must refuse new admission");
            if mode != 1 {
                assert!(writer.take().is_none(), "capture-first must defer metadata without a storage lock");
                assert_eq!(std::fs::read(config.paths.root.join("clipboard.json")).unwrap(), before);
            }
            if mode == 2 {
                writer.stop();
                let completed = harvest(&mut writer);
                assert!(completed.result.is_err());
                assert!(completed.retention.is_none());
                assert!(!writer.retention_phase().load(Ordering::SeqCst));
                assert_eq!(std::fs::read(config.paths.root.join("clipboard.json")).unwrap(), before);
                assert_eq!(completed.store.snapshot(), config.snapshot());
                assert!(port.acquire().is_none(), "stop must not revoke the accepted capture lease");
                drop(accepted);
                assert!(port.acquire().is_some());
            } else {
                if mode == 0 {
                    history.capture(crate::clipboard_history::Payload::Text("accepted before retention".into()), "fixture".into(), crate::clipboard_history::now(), &old, ||true).unwrap();
                }
                drop(accepted);
                let completed = harvest(&mut writer);
                assert!(completed.result.unwrap());
                assert!(completed.retention.is_some());
                assert!(port.acquire().is_none(), "completed worker still waits for owner adoption");
                writer.retention_phase().store(false, Ordering::SeqCst);
                history.reconcile_settings(&completed.store.current().clipboard, false, ||true).unwrap();
                assert_eq!(history.items.len(), 1);
                if mode == 0 { assert_eq!(history.items[0].text, "accepted before retention"); }
                assert_eq!(std::fs::read(config.paths.root.join("clipboard.json")).unwrap(), serde_json::to_vec_pretty(&history.items).unwrap());
                assert!(port.acquire().is_some());
            }
            control.stop();
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").map(PathBuf::from)
                .unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
                .join(format!(
                    "cj{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn store(&self) -> ConfigStore {
            ConfigStore::open(crate::profile::ProfilePaths::new(self.0.join("p"), self.0.clone(), None))
            .unwrap()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn harvest<T: Send + 'static>(writer: &mut Writer<T>) -> Completion<T> {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(value) = writer.take() {
                return value;
            }
            assert!(Instant::now() < end, "writer did not retire");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn publication_retirement_is_conservative_and_preserves_prior_writes() {
        let publication = Publication::default();
        publication.enter(|| true).unwrap();
        assert_eq!(
            publication.retire()["configurationWrite"],
            serde_json::json!({"published":false,"outcomeUncertain":true})
        );
        publication.complete(true);
        assert_eq!(
            publication.snapshot(),
            serde_json::json!({"published":true,"outcomeUncertain":false})
        );
        assert!(publication.enter(|| true).is_err());
        let multi = Publication::default();
        multi.enter(|| true).unwrap();
        multi.complete(true);
        multi.enter(|| true).unwrap();
        multi.complete(false);
        assert_eq!(
            multi.snapshot(),
            serde_json::json!({"published":true,"outcomeUncertain":false})
        );
        multi.enter(|| true).unwrap();
        assert_eq!(
            multi.retire()["configurationWrite"],
            serde_json::json!({"published":true,"outcomeUncertain":true})
        );
    }
    #[test]
    fn stopped_writer_retires_but_held_main_sdk_keeps_profile_locked() {
        use crate::hotkeys::{Platform, Registry};
        struct Held {
            entered: Option<std::sync::mpsc::Sender<()>>,
            release: std::sync::mpsc::Receiver<()>,
            called: Arc<AtomicBool>,
        }
        impl Platform for Held {
            type Handle = u32;
            fn register(&mut self, _: u32, _: u32, id: u32) -> Result<u32, i32> {
                if let Some(entered) = self.entered.take() {
                    self.called.store(true, Ordering::Release);
                    entered.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                Ok(id)
            }
            fn unregister(&mut self, _: u32) -> Result<(), (u32, i32)> {
                Ok(())
            }
            fn routes(&mut self, _: std::collections::BTreeMap<u32, crate::hotkeys::Press>) {}
            fn quarantine(&mut self, _: u32) {
                panic!("own fixture cannot fail cleanup");
            }
        }
        let temp = Temp::new();
        let store = temp.store();
        let before = store.snapshot().to_vec();
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        let mut control =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        let (client, mut service) = crate::keyboard_service::channel();
        let mut writer: Writer<()> =
            Writer::new(control.profile_lease(), tx.clone(), Some(client.clone()));
        writer
            .start(&store, Arc::new(|| true), false, |store| {
                let mut value: serde_json::Value =
                    serde_json::from_slice(store.snapshot()).unwrap();
                value["theme"] = "light".into();
                store.save(&value)?;
                Ok(())
            })
            .unwrap();
        let (entered, wait) = std::sync::mpsc::channel();
        let (release, hold) = std::sync::mpsc::channel();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !client.pending() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let main = std::thread::spawn(move || {
            let called = Arc::new(AtomicBool::new(false));
            let mut registry = Registry::new(Held {
                entered: Some(entered),
                release: hold,
                called: called.clone(),
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !called.load(Ordering::Acquire) {
                service.pump(&mut registry);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(writer.busy());
        assert_eq!(std::fs::read(&store.file).unwrap(), before);
        writer.stop();
        let completion = harvest(&mut writer);
        assert!(completion.result.is_err());
        assert!(!completion.published);
        control.stop();
        drop(writer);
        drop(client);
        assert!(ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).is_err());
        release.send(()).unwrap();
        main.join().unwrap();
        let mut next = ControlServer::start(&store.paths.root, tx, Limits::default()).unwrap();
        next.stop();
        assert_eq!(std::fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn bounded_writer_keeps_owner_snapshot_and_harvests_even_with_full_wake_queue() {
        let temp = Temp::new();
        let store = temp.store();
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let server =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        tx.try_send(HostEvent::UiWake).unwrap();
        let mut writer = Writer::new(server.profile_lease(), tx, None);
        let (entered, wait) = std::sync::mpsc::channel();
        let (release, hold) = std::sync::mpsc::channel();
        let before = store.snapshot().to_vec();
        writer
            .start(&store, Arc::new(|| true), false, move |store| {
                entered.send(()).unwrap();
                hold.recv().unwrap();
                let mut value: serde_json::Value =
                    serde_json::from_slice(store.snapshot()).unwrap();
                value["theme"] = "light".into();
                store.save(&value)?;
                Ok(42)
            })
            .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(writer.busy());
        assert!(writer.take().is_none());
        assert!(writer
            .start(&store, Arc::new(|| true), false, |_| Ok(99))
            .is_err());
        assert_eq!(store.snapshot(), before);
        release.send(()).unwrap();
        let result = harvest(&mut writer);
        assert_eq!(result.result.unwrap(), 42);
        assert!(result.published);
        assert!(!result.uncertain);
        assert_eq!(result.store.current().theme.as_deref(), Some("light"));
        assert_eq!(store.snapshot(), before);
        assert!(!writer.busy());
    }
    #[test]
    fn stopped_detached_writer_keeps_only_profile_lock_until_real_thread_exit() {
        let temp = Temp::new();
        let store = temp.store();
        let before = store.snapshot().to_vec();
        let file = store.file.clone();
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        let mut server =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        let mut writer = Writer::new(server.profile_lease(), tx.clone(), None);
        let (entered, wait) = std::sync::mpsc::channel();
        let (release, hold) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        writer
            .start(&store, Arc::new(|| true), false, move |store| {
                entered.send(()).unwrap();
                hold.recv().unwrap();
                let mut value: serde_json::Value =
                    serde_json::from_slice(store.snapshot()).unwrap();
                value["theme"] = "light".into();
                assert!(store.save(&value).is_err());
                done.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        let at = Instant::now();
        server.stop();
        drop(writer);
        assert!(at.elapsed() < Duration::from_millis(500));
        assert!(!store.paths.root.join("control.sock").exists());
        assert!(ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).is_err());
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(mut next) =
                ControlServer::start(&store.paths.root, tx.clone(), Limits::default())
            {
                next.stop();
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(std::fs::read(file).unwrap(), before);
    }
    #[test]
    fn published_save_survives_late_cancellation_and_error_without_leaking_job_authority() {
        let temp = Temp::new();
        let store = temp.store();
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        let server =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        let mut writer: Writer<()> = Writer::new(server.profile_lease(), tx, None);
        let live = Arc::new(AtomicBool::new(true));
        let guard = live.clone();
        writer
            .start(
                &store,
                Arc::new(move || guard.load(Ordering::Acquire)),
                false,
                move |store| {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(store.snapshot()).unwrap();
                    value["theme"] = "light".into();
                    store.save(&value)?;
                    live.store(false, Ordering::Release);
                    Err("A saved; the follow-up is no longer allowed".into())
                },
            )
            .unwrap();
        let mut result = harvest(&mut writer);
        assert!(result.result.is_err());
        assert!(result.published);
        assert!(!result.uncertain);
        assert_eq!(result.store.current().theme.as_deref(), Some("light"));
        // The owner's accepted copy cannot retain the now-false UI job guard.
        let mut value: serde_json::Value = serde_json::from_slice(result.store.snapshot()).unwrap();
        value["theme"] = "dark".into();
        result.store.save(&value).unwrap();
    }
    #[test]
    fn cancelled_relocation_cannot_publish_selector_or_leave_its_destination_copy() {
        let temp = Temp::new();
        let store = temp.store();
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        let server =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        let mut writer = Writer::new(server.profile_lease(), tx, None);
        let live = Arc::new(AtomicBool::new(true));
        let guard = live.clone();
        let destination = temp.0.join("moved");
        let folder = destination.clone();
        writer
            .start(
                &store,
                Arc::new(move || guard.load(Ordering::Acquire)),
                false,
                move |store| {
                    live.store(false, Ordering::Release);
                    store.relocate(folder.to_str().unwrap())
                },
            )
            .unwrap();
        let result = harvest(&mut writer);
        assert!(result.result.is_err());
        assert!(!result.published);
        assert!(!result.uncertain);
        assert!(!store.paths.root.join("config-location.json").exists());
        assert!(!destination.join("config.json").exists());
        assert_eq!(result.store.file, store.file);
    }
    #[test]
    fn panic_after_publication_retains_saved_data_for_global_reconciliation() {
        let temp = Temp::new();
        let store = temp.store();
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        let server =
            ControlServer::start(&store.paths.root, tx.clone(), Limits::default()).unwrap();
        let mut writer: Writer<()> = Writer::new(server.profile_lease(), tx, None);
        writer
            .start(&store, Arc::new(|| true), false, |store| {
                let mut value: serde_json::Value =
                    serde_json::from_slice(store.snapshot()).unwrap();
                value["theme"] = "light".into();
                store.save(&value)?;
                panic!("own writer fixture after rename");
            })
            .unwrap();
        let result = harvest(&mut writer);
        assert!(result.result.is_err());
        assert!(result.published);
        assert_eq!(result.store.current().theme.as_deref(), Some("light"));
    }
}
