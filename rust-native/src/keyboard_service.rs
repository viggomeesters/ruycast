//! One outstanding main-thread registration transaction. Pending polls are
//! nonblocking; blocking config staging is exclusively for a background writer.
use crate::{
    config_effects,
    hotkeys::{Plan, Platform, Registry},
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    mpsc, Arc, Mutex,
};
type Wake = Arc<dyn Fn() + Send + Sync>;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HookHealth {
    pub generation: u64,
    pub status: String,
    pub hyper: String,
    pub modifier_taps: String,
    pub sided: String,
    pub recovering: bool,
}
impl Default for HookHealth {
    fn default() -> Self {
        Self {
            generation: 0,
            status: "Off".into(),
            hyper: "Off".into(),
            modifier_taps: "Off".into(),
            sided: "Off".into(),
            recovering: false,
        }
    }
}
struct Shared {
    analytics: Mutex<Option<crate::analytics::Client>>,
    keyword_stage_epoch: AtomicU64,
    external_activation: AtomicU64,
    keyword: Mutex<Option<(u64, Arc<crate::keyword_capture::Prepared>)>>,
    keyword_confirmed: AtomicU64,
    keyword_status: Mutex<(u64, String)>,
    paused: AtomicBool,
    hook_health: Mutex<HookHealth>,
    hook_epoch: AtomicU64,
    hook_available: AtomicBool,
    hook_forwarded: AtomicU64,
    dropped: AtomicU64,
    recorders: AtomicU64,
    recording_epoch: AtomicU64,
    recorder_ready: AtomicBool,
    callback_gate: Mutex<Option<Arc<AtomicBool>>>,
    error: Mutex<Option<String>>,
    profile: Mutex<Option<crate::control::ProfileLease>>,
    running: AtomicBool,
    leased: Arc<AtomicBool>,
    accepted: AtomicU64,
    wake: Mutex<Option<Wake>>,
}
impl Shared {
    fn gate(&self, enabled: bool) {
        if let Some(gate) = self
            .callback_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            gate.store(enabled, Ordering::Release);
        }
    }
    fn invalidate_recording(&self) {
        if self
            .recording_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.running.store(false, Ordering::Release);
            self.gate(false);
        }
    }
    fn notify(&self) {
        let wake = self.wake.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}
/// Immediate logical suppression; physical main-thread retirement is a separate
/// acknowledgement. Distinct window leases cannot resume each other's keys.
pub struct RecorderLease(Arc<Shared>);
impl RecorderLease {
    pub fn ready(&self) -> Result<bool, String> {
        if !self.0.running.load(Ordering::Acquire) {
            return Err("Keyboard service stopped.".into());
        }
        if let Some(error) = self
            .0
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Err(error);
        }
        Ok(self.0.recorder_ready.load(Ordering::Acquire))
    }
}
impl Drop for RecorderLease {
    fn drop(&mut self) {
        self.0.invalidate_recording();
        if self.0.recorders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.recorder_ready.store(false, Ordering::Release);
        }
        self.0.notify();
    }
}
struct Lease(Arc<AtomicBool>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Decision {
    phase: AtomicU8,
    shared: Arc<Shared>,
}
impl Decision {
    fn cancel(&self) {
        let _ = self
            .phase
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
        self.shared.notify();
    }
}
struct Request {
    _lease: Lease,
    plan: Plan,
    keyword: Option<Arc<crate::keyword_capture::Prepared>>,
    decision: Arc<Decision>,
    reply: mpsc::SyncSender<Result<u64, String>>,
    complete: mpsc::SyncSender<Result<config_effects::Receipt, String>>,
    generation: Option<u64>,
}
#[derive(Clone)]
pub struct Client {
    sender: mpsc::SyncSender<Request>,
    shared: Arc<Shared>,
}
pub struct Pending {
    reply: mpsc::Receiver<Result<u64, String>>,
    decision: Option<Arc<Decision>>,
    complete: Option<mpsc::Receiver<Result<config_effects::Receipt, String>>>,
}
pub struct Stage {
    decision: Arc<Decision>,
    generation: u64,
    complete: Option<mpsc::Receiver<Result<config_effects::Receipt, String>>>,
}
pub struct Activation {
    reply: mpsc::Receiver<Result<config_effects::Receipt, String>>,
    shared: Arc<Shared>,
}
impl Activation {
    pub fn poll(&mut self) -> Option<Result<config_effects::Receipt, String>> {
        match self.reply.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Keyboard activation stopped.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => {
                if self.shared.running.load(Ordering::Acquire) {
                    None
                } else {
                    Some(Err("Keyboard activation stopped.".into()))
                }
            }
        }
    }
}
impl Stage {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn ready(&self) -> Result<(), String> {
        if self.decision.shared.running.load(Ordering::Acquire)
            && self.decision.phase.load(Ordering::Acquire) == 0
        {
            Ok(())
        } else {
            Err("Keyboard stage stopped or was cancelled.".into())
        }
    }
    pub fn request_commit(&mut self) -> Result<Activation, String> {
        self.ready()?;
        if self.decision.shared.running.load(Ordering::Acquire)
            && self
                .decision
                .phase
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            self.decision.shared.notify();
            Ok(Activation {
                reply: self
                    .complete
                    .take()
                    .expect("uncommitted activation receiver"),
                shared: self.decision.shared.clone(),
            })
        } else {
            Err("Keyboard stage stopped or was cancelled before activation.".into())
        }
    }
}
struct BackgroundStage(Stage);
struct LiveStage {
    inner: BackgroundStage,
    live: crate::execution::Live,
}
impl config_effects::Stage for LiveStage {
    fn ready(&self) -> Result<(), String> {
        if !(self.live)() {
            Err("Configuration update was cancelled before publication.".into())
        } else {
            self.inner.ready()
        }
    }
    // Publication may have happened: UI retirement cannot retract its global result.
    fn commit(&mut self) -> Result<config_effects::Receipt, String> {
        self.inner.commit()
    }
}
impl config_effects::Stage for BackgroundStage {
    fn ready(&self) -> Result<(), String> {
        self.0.ready()
    }
    fn commit(&mut self) -> Result<config_effects::Receipt, String> {
        let mut activation = self.0.request_commit()?;
        loop {
            if let Some(result) = activation.poll() {
                return result;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(20));
        }
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        self.decision.cancel();
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(decision) = &self.decision {
            decision.cancel();
        }
    }
}
impl Pending {
    pub fn poll(&mut self) -> Option<Result<Stage, String>> {
        let decision = self.decision.as_ref()?;
        if !decision.shared.running.load(Ordering::Acquire) {
            decision.cancel();
            return Some(Err("Keyboard service stopped.".into()));
        }
        match self.reply.try_recv() {
            Ok(Ok(generation)) => Some(Ok(Stage {
                decision: self.decision.take().expect("live pending decision"),
                generation,
                complete: self.complete.take(),
            })),
            Ok(Err(error)) => {
                decision.cancel();
                self.decision.take();
                Some(Err(error))
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                decision.cancel();
                self.decision.take();
                Some(Err("Keyboard service stopped.".into()))
            }
        }
    }
}
pub struct Server {
    receiver: mpsc::Receiver<Request>,
    shared: Arc<Shared>,
    active: Option<Request>,
}
pub fn channel() -> (Client, Server) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let shared = Arc::new(Shared {
        analytics: Mutex::new(None),
        keyword_stage_epoch: AtomicU64::new(0),
        external_activation: AtomicU64::new(0),
        keyword: Mutex::new(None),
        keyword_confirmed: AtomicU64::new(0),
        keyword_status: Mutex::new((0, "Off".into())),
        paused: AtomicBool::new(false),
        hook_health: Mutex::new(HookHealth::default()),
        hook_epoch: AtomicU64::new(0),
        hook_available: AtomicBool::new(false),
        hook_forwarded: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        recorders: AtomicU64::new(0),
        recording_epoch: AtomicU64::new(0),
        recorder_ready: AtomicBool::new(false),
        callback_gate: Mutex::new(None),
        error: Mutex::new(None),
        profile: Mutex::new(None),
        running: AtomicBool::new(true),
        leased: Arc::new(AtomicBool::new(false)),
        accepted: AtomicU64::new(0),
        wake: Mutex::new(None),
    });
    (
        Client {
            sender,
            shared: shared.clone(),
        },
        Server {
            receiver,
            shared,
            active: None,
        },
    )
}
impl Client {
    pub fn attach_analytics(&self, client: crate::analytics::Client) {
        *self
            .shared
            .analytics
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(client);
    }
    pub fn analytics(&self) -> Option<crate::analytics::Client> {
        self.shared
            .analytics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn keyword_stage_epoch(&self) -> u64 {
        self.shared.keyword_stage_epoch.load(Ordering::Acquire)
    }
    pub fn running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }
    pub fn paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }
    pub fn set_paused(&self, value: bool) {
        if self.shared.paused.swap(value, Ordering::AcqRel) != value {
            self.shared.invalidate_recording();
            self.shared.notify();
        }
    }
    pub fn hook_epoch(&self) -> u64 {
        self.shared.hook_epoch.load(Ordering::Acquire)
    }
    pub fn invalidate_hook_delivery(&self) {
        self.shared.hook_available.store(false, Ordering::Release);
        if self
            .shared
            .hook_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.stop();
        }
        self.shared.notify();
    }
    pub fn hooks_available(&self) -> bool {
        self.running() && !self.paused() && self.shared.hook_available.load(Ordering::Acquire)
    }
    pub fn wake_main(&self) {
        self.shared.notify();
    }
    pub fn hook_health(&self) -> HookHealth {
        let mut health = self
            .shared
            .hook_health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        // Read logical gates without entering a native owner. Status RPCs remain
        // truthful even while main is held inside an SDK teardown call.
        let replacement = if !self.running() {
            Some("Off")
        } else if self.paused() {
            Some("Paused")
        } else if !self.shared.hook_available.load(Ordering::Acquire) {
            Some("Recovering")
        } else {
            None
        };
        if let Some(replacement) = replacement {
            if health.status == "On" {
                health.status = replacement.into();
                health.recovering = true;
            }
            for value in [
                &mut health.hyper,
                &mut health.modifier_taps,
                &mut health.sided,
            ] {
                if value == "On" {
                    *value = replacement.into();
                }
            }
        }
        if self.recording() {
            health.modifier_taps = "Off".into();
            health.sided = "Off".into();
        }
        health
    }
    pub fn note_hook_forwarded(&self) {
        self.shared.hook_forwarded.fetch_add(1, Ordering::Relaxed);
    }
    pub fn hook_forwarded(&self) -> u64 {
        self.shared.hook_forwarded.load(Ordering::Relaxed)
    }
    pub fn publish_hook_health(&self, health: HookHealth) {
        self.shared.hook_available.store(
            health.generation != 0 && health.status == "On",
            Ordering::Release,
        );
        *self
            .shared
            .hook_health
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = health;
    }
    pub fn attach_callback_gate(&self, gate: Arc<AtomicBool>) {
        gate.store(false, Ordering::Release);
        *self
            .shared
            .callback_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(gate);
    }
    pub fn note_dropped(&self) {
        self.shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
    pub fn recording_epoch(&self) -> u64 {
        self.shared.recording_epoch.load(Ordering::Acquire)
    }
    pub fn recording(&self) -> bool {
        self.shared.recorders.load(Ordering::Acquire) != 0
    }
    pub fn error(&self) -> Option<String> {
        self.shared
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn recorder(&self) -> Result<RecorderLease, String> {
        if !self.shared.running.load(Ordering::Acquire) {
            return Err("Keyboard service stopped.".into());
        }
        self.shared.gate(false);
        self.shared.invalidate_recording();
        let before = self
            .shared
            .recorders
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "Recorder capacity exhausted.".to_string())?;
        if before == 0 {
            self.shared.recorder_ready.store(false, Ordering::Release);
        }
        *self.shared.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.shared.notify();
        Ok(RecorderLease(self.shared.clone()))
    }
    pub fn retain_profile(&self, lease: crate::control::ProfileLease) {
        *self
            .shared
            .profile
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(lease);
    }
    pub fn pending(&self) -> bool {
        self.shared.leased.load(Ordering::Acquire)
    }
    pub fn attach_wake(&self, wake: Wake) {
        *self.shared.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
        self.shared.notify();
    }
    pub fn accepted_generation(&self) -> u64 {
        if self.shared.running.load(Ordering::Acquire) {
            self.shared.accepted.load(Ordering::Acquire)
        } else {
            0
        }
    }
    pub fn stop(&self) {
        self.shared.gate(false);
        self.shared.recorder_ready.store(false, Ordering::Release);
        self.shared.running.store(false, Ordering::Release);
        self.shared.accepted.store(0, Ordering::Release);
        self.shared.notify();
    }
    pub fn external_activation_epoch(&self) -> u64 {
        self.shared.external_activation.load(Ordering::Acquire)
    }
    pub fn note_external_activation(&self) -> Option<u64> {
        let result = self.shared.external_activation.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |n| n.checked_add(1),
        );
        if result.is_err() {
            self.stop();
        }
        self.shared.notify();
        result.ok().map(|previous| previous + 1)
    }
    pub fn keyword_generation(&self) -> u64 {
        let confirmed = self.shared.keyword_confirmed.load(Ordering::Acquire);
        if confirmed != 0 && !self.pending() && confirmed == self.accepted_generation() {
            confirmed
        } else {
            0
        }
    }
    /// Called only by the serialized owner after adopting the writer receipt.
    pub fn confirm_keywords(&self, generation: u64) {
        self.shared.keyword_confirmed.store(
            if !self.pending() && generation == self.accepted_generation() {
                generation
            } else {
                0
            },
            Ordering::Release,
        );
        self.shared.notify();
    }
    pub fn keyword_prepared(&self) -> Option<(u64, Arc<crate::keyword_capture::Prepared>)> {
        let generation = self.keyword_generation();
        let value = self
            .shared
            .keyword
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        value.filter(|(g, _)| {
            generation != 0 && *g == generation && self.keyword_generation() == generation
        })
    }
    pub fn keyword_status(&self) -> String {
        if !self.running() {
            "Off".into()
        } else if self.keyword_generation() == 0 {
            let (_, status) = self
                .shared
                .keyword_status
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if !self.paused() && !self.recording() && status == "Configuration pending" {
                status
            } else {
                "Off".into()
            }
        } else if self.paused() {
            "Paused".into()
        } else if self.recording() {
            "Off".into()
        } else {
            let (generation, status) = self
                .shared
                .keyword_status
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if generation == self.keyword_generation() {
                status
            } else {
                "Recovering".into()
            }
        }
    }
    pub fn publish_keyword_status(&self, generation: u64, status: String) {
        *self
            .shared
            .keyword_status
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = (generation, status);
    }
    pub fn prepare(&self, plan: Plan) -> Result<Pending, String> {
        self.prepare_with_keywords(plan, None)
    }
    fn prepare_with_keywords(
        &self,
        plan: Plan,
        keyword: Option<Arc<crate::keyword_capture::Prepared>>,
    ) -> Result<Pending, String> {
        if !self.shared.running.load(Ordering::Acquire) {
            return Err("Keyboard service stopped.".into());
        }
        self.shared
            .leased
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Keyboard configuration is busy.".to_string())?;
        // Revoke leases before publishing pending. A failed stage can restore
        // the same generation, but cannot restore this monotone episode.
        if self
            .shared
            .keyword_stage_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.shared.leased.store(false, Ordering::Release);
            self.stop();
            return Err("Keyboard configuration epoch exhausted.".into());
        }
        self.shared.keyword_confirmed.store(0, Ordering::Release);
        let decision = Arc::new(Decision {
            phase: AtomicU8::new(0),
            shared: self.shared.clone(),
        });
        let (reply, receive) = mpsc::sync_channel(1);
        let (complete, completed) = mpsc::sync_channel(1);
        let request = Request {
            _lease: Lease(self.shared.leased.clone()),
            plan,
            keyword,
            decision: decision.clone(),
            reply,
            complete,
            generation: None,
        };
        self.sender
            .try_send(request)
            .map_err(|_| "Keyboard service stopped or busy.".to_string())?;
        self.shared.notify();
        Ok(Pending {
            reply: receive,
            decision: Some(decision),
            complete: Some(completed),
        })
    }
    /// Use on a background writer, never the serialized UI/control owner.
    pub fn background_factory(&self) -> config_effects::Factory {
        self.background_factory_with_live(Arc::new(|| true))
    }
    pub fn background_factory_with_live(
        &self,
        live: crate::execution::Live,
    ) -> config_effects::Factory {
        let client = self.clone();
        Arc::new(move |config| {
            if !live() {
                return Err("Configuration update was cancelled.".into());
            }
            let prepared = Arc::new(crate::keyword_capture::Prepared::new(config)?);
            let mut pending =
                client.prepare_with_keywords(Plan::from_config(config)?, Some(prepared))?;
            loop {
                if !live() {
                    return Err("Configuration update was cancelled.".into());
                }
                if let Some(result) = pending.poll() {
                    return result.map(|stage| {
                        Box::new(LiveStage {
                            inner: BackgroundStage(stage),
                            live: live.clone(),
                        }) as Box<dyn config_effects::Stage>
                    });
                }
                std::thread::park_timeout(std::time::Duration::from_millis(20));
            }
        })
    }
}
impl Server {
    fn sync_recorders<P: Platform>(&self, registry: &mut Registry<P>) {
        if self.shared.recorders.load(Ordering::Acquire) != 0 {
            self.shared.gate(false);
            registry.suspend();
            let clean = registry.suspension_clean();
            self.shared.recorder_ready.store(clean, Ordering::Release);
            *self.shared.error.lock().unwrap_or_else(|e|e.into_inner()) = (!clean)
                .then(|| "Keyboard shortcuts could not be suspended. Reload configuration before recording.".into());
        } else {
            self.shared.recorder_ready.store(false, Ordering::Release);
            let result = registry.resume_if(|| {
                self.shared.running.load(Ordering::Acquire)
                    && self.shared.recorders.load(Ordering::Acquire) == 0
            });
            let ready = result.is_ok()
                && self.shared.running.load(Ordering::Acquire)
                && self.shared.recorders.load(Ordering::Acquire) == 0;
            self.shared.gate(ready);
            *self.shared.error.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
        }
    }
    pub fn pump<P: Platform>(&mut self, registry: &mut Registry<P>) {
        if !self.shared.running.load(Ordering::Acquire) {
            self.shared.gate(false);
            if let Some(request) = self.active.take() {
                request.decision.cancel();
            }
            registry.rollback();
            registry.suspend();
            return;
        }
        self.sync_recorders(registry);
        if let Some(request) = self.active.take() {
            match request.decision.phase.load(Ordering::Acquire) {
                1 => {
                    registry.commit();
                    self.sync_recorders(registry);
                    let active = self.shared.running.load(Ordering::Acquire);
                    *self
                        .shared
                        .keyword
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = if active {
                        request.keyword.as_ref().map(|value| {
                            (
                                request.generation.expect("prepared generation"),
                                value.clone(),
                            )
                        })
                    } else {
                        None
                    };
                    self.shared.accepted.store(
                        if active {
                            request.generation.expect("prepared main generation")
                        } else {
                            0
                        },
                        Ordering::Release,
                    );
                    let result = if !active {
                        Err("Keyboard activation stopped after publication.".into())
                    } else {
                        Ok(config_effects::Receipt {
                            generation: request.generation,
                            warning: (!registry.warnings().is_empty()).then(|| {
                                format!(
                                    "Keyboard bindings active; cleanup pending: {}",
                                    registry.warnings().join(" ")
                                )
                            }),
                        })
                    };
                    let complete = request.complete.clone();
                    drop(request); // Physical retirement precedes the writer's completion ACK.
                    let _ = complete.try_send(result);
                }
                2 => registry.rollback(),
                _ => {
                    self.active = Some(request);
                    return;
                }
            }
        }
        let Ok(mut request) = self.receiver.try_recv() else {
            return;
        };
        if request.decision.phase.load(Ordering::Acquire) != 0 {
            return;
        }
        let recording_at_entry = self.shared.recorders.load(Ordering::Acquire) != 0;
        let recording_epoch = self.shared.recording_epoch.load(Ordering::Acquire);
        let result = registry.stage_if(std::mem::take(&mut request.plan), || {
            request.decision.phase.load(Ordering::Acquire) == 0
                && self.shared.running.load(Ordering::Acquire)
                && (recording_at_entry
                    || self.shared.recording_epoch.load(Ordering::Acquire) == recording_epoch)
        });
        // A nested SDK call may have started a recorder. Retire physical keys
        // before publishing prepare/activation readiness or reopening the gate.
        self.sync_recorders(registry);
        if request.decision.phase.load(Ordering::Acquire) != 0
            || !self.shared.running.load(Ordering::Acquire)
        {
            registry.rollback();
            return;
        }
        let success = result.is_ok();
        request.generation = result.as_ref().ok().copied();
        if request.reply.try_send(result).is_err() {
            registry.rollback();
            return;
        }
        if success {
            self.active = Some(request);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shared.gate(false);
        self.shared.running.store(false, Ordering::Release);
        self.shared.accepted.store(0, Ordering::Release);
        if let Some(request) = self.active.take() {
            request.decision.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn health_projects_immediate_logical_gates_without_waiting_for_main() {
        let (client, _server) = channel();
        let on = HookHealth {
            generation: 1,
            status: "On".into(),
            hyper: "On".into(),
            modifier_taps: "On".into(),
            sided: "On".into(),
            recovering: false,
        };
        client.publish_hook_health(on.clone());
        assert_eq!(client.hook_health(), on);
        let first = client.recorder().unwrap();
        let second = client.recorder().unwrap();
        assert_eq!(client.hook_health().hyper, "On");
        assert_eq!(client.hook_health().modifier_taps, "Off");
        drop(first);
        assert_eq!(client.hook_health().sided, "Off");
        drop(second);
        assert_eq!(client.hook_health(), on);
        client.invalidate_hook_delivery();
        assert_eq!(client.hook_health().status, "Recovering");
        assert_eq!(client.hook_health().hyper, "Recovering");
        client.publish_hook_health(on.clone());
        client.set_paused(true);
        assert_eq!(client.hook_health().status, "Paused");
        client.set_paused(false);
        client.stop();
        assert_eq!(client.hook_health().status, "Off");
        assert_eq!(client.hook_health().sided, "Off");
    }
    #[test]
    fn two_recorder_leases_require_physical_retirement_and_cannot_resume_each_other() {
        let (client, mut server) = channel();
        let gate = Arc::new(AtomicBool::new(true));
        client.attach_callback_gate(gate.clone());
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count: count.clone(),
        });
        let mut pending = client.prepare(plan()).unwrap();
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        let generation = stage.generation();
        let mut activation = stage.request_commit().unwrap();
        server.pump(&mut registry);
        assert_eq!(
            activation.poll().unwrap().unwrap().generation,
            Some(generation)
        );
        let registered = count.load(Ordering::Acquire);
        assert!(registered > 0);
        assert!(gate.load(Ordering::Acquire));
        let epoch = client.recording_epoch();
        let first = client.recorder().unwrap();
        assert!(!gate.load(Ordering::Acquire));
        assert!(!first.ready().unwrap());
        assert_eq!(count.load(Ordering::Acquire), registered);
        server.pump(&mut registry);
        assert!(first.ready().unwrap());
        assert_eq!(count.load(Ordering::Acquire), 0);
        let second = client.recorder().unwrap();
        assert!(second.ready().unwrap());
        drop(first);
        server.pump(&mut registry);
        assert!(client.recording());
        assert!(!gate.load(Ordering::Acquire));
        assert!(second.ready().unwrap());
        drop(second);
        server.pump(&mut registry);
        assert!(!client.recording());
        assert!(gate.load(Ordering::Acquire));
        assert_eq!(count.load(Ordering::Acquire), registered);
        assert_eq!(client.accepted_generation(), generation);
        assert_ne!(client.recording_epoch(), epoch);
    }
    #[test]
    fn nested_recorder_during_sdk_stage_gates_immediately_and_waits_for_actual_cleanup() {
        struct Reentrant {
            inner: Fake,
            client: Client,
            lease: Arc<Mutex<Option<RecorderLease>>>,
            gate: Arc<AtomicBool>,
        }
        impl Platform for Reentrant {
            type Handle = u64;
            fn register(&mut self, a: u32, b: u32, id: u32) -> Result<u64, i32> {
                let handle = self.inner.register(a, b, id)?;
                if self.lease.lock().unwrap().is_none() {
                    let lease = self.client.recorder().unwrap();
                    assert!(!self.gate.load(Ordering::Acquire));
                    assert!(!lease.ready().unwrap());
                    *self.lease.lock().unwrap() = Some(lease);
                }
                Ok(handle)
            }
            fn unregister(&mut self, h: u64) -> Result<(), (u64, i32)> {
                self.inner.unregister(h)
            }
            fn routes(&mut self, r: BTreeMap<u32, crate::hotkeys::Press>) {
                self.inner.routes(r)
            }
            fn quarantine(&mut self, h: u64) {
                self.inner.quarantine(h)
            }
        }
        let (client, mut server) = channel();
        let gate = Arc::new(AtomicBool::new(true));
        client.attach_callback_gate(gate.clone());
        let count = Arc::new(AtomicU64::new(0));
        let lease = Arc::new(Mutex::new(None));
        let mut registry = Registry::new(Reentrant {
            inner: Fake {
                entered: None,
                release: None,
                count: count.clone(),
            },
            client: client.clone(),
            lease: lease.clone(),
            gate: gate.clone(),
        });
        let mut pending = client.prepare(plan()).unwrap();
        server.pump(&mut registry);
        let outcome = pending.poll().unwrap();
        assert!(outcome.is_err() || outcome.as_ref().is_ok_and(|s| s.ready().is_ok()));
        assert_eq!(count.load(Ordering::Acquire), 0);
        assert!(!gate.load(Ordering::Acquire));
        assert!(lease.lock().unwrap().as_ref().unwrap().ready().unwrap());
        client.stop();
        assert!(lease.lock().unwrap().as_ref().unwrap().ready().is_err());
    }
    #[test]
    fn cleanup_failure_acknowledges_active_generation_with_warning_instead_of_false_activation_error(
    ) {
        struct Fault {
            inner: Fake,
            fail: Arc<AtomicBool>,
        }
        impl Platform for Fault {
            type Handle = u64;
            fn register(&mut self, c: u32, m: u32, id: u32) -> Result<u64, i32> {
                self.inner.register(c, m, id)
            }
            fn unregister(&mut self, h: u64) -> Result<(), (u64, i32)> {
                if self.fail.load(Ordering::Acquire) {
                    Err((h, -50))
                } else {
                    self.inner.unregister(h)
                }
            }
            fn routes(&mut self, r: BTreeMap<u32, crate::hotkeys::Press>) {
                self.inner.routes(r);
            }
            fn quarantine(&mut self, h: u64) {
                self.inner.quarantine(h);
            }
        }
        let (client, mut server) = channel();
        let fail = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fault {
            inner: Fake {
                entered: None,
                release: None,
                count: count.clone(),
            },
            fail: fail.clone(),
        });
        let mut pending = client.prepare(plan()).unwrap();
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        let mut ack = stage.request_commit().unwrap();
        server.pump(&mut registry);
        assert!(ack.poll().unwrap().unwrap().warning.is_none());
        fail.store(true, Ordering::Release);
        let mut pending = client.prepare(Plan::default()).unwrap();
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        let generation = stage.generation();
        let mut ack = stage.request_commit().unwrap();
        server.pump(&mut registry);
        let receipt = ack.poll().unwrap().unwrap();
        assert!(receipt
            .warning
            .unwrap()
            .contains("bindings active; cleanup pending"));
        assert_eq!(client.accepted_generation(), generation);
        assert!(registry.current().carbon.is_empty());
        assert!(count.load(Ordering::Acquire) > 0);
        fail.store(false, Ordering::Release);
        drop(registry);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
    #[test]
    fn background_factory_never_reports_commit_before_native_activation_acknowledgement() {
        let (client, mut server) = channel();
        let (wake, receive) = mpsc::channel();
        client.attach_wake(Arc::new(move || {
            let _ = wake.send(());
        }));
        receive.recv().unwrap();
        let factory = client.background_factory();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let config = rustcast_core::validate(&rustcast_core::default_config()).unwrap();
            let mut stage = factory(&config).unwrap();
            done.send(stage.commit()).unwrap();
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count,
        });
        server.pump(&mut registry);
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert_eq!(client.accepted_generation(), 0);
        server.pump(&mut registry);
        assert!(result
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .is_ok());
        worker.join().unwrap();
        assert!(client.accepted_generation() > 0);
    }
    struct Fake {
        entered: Option<mpsc::Sender<()>>,
        release: Option<mpsc::Receiver<()>>,
        count: Arc<AtomicU64>,
    }
    impl Platform for Fake {
        type Handle = u64;
        fn register(&mut self, _: u32, _: u32, id: u32) -> Result<u64, i32> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.release.take().unwrap().recv().unwrap();
            }
            self.count.fetch_add(1, Ordering::AcqRel);
            Ok(u64::from(id))
        }
        fn unregister(&mut self, _: u64) -> Result<(), (u64, i32)> {
            self.count.fetch_sub(1, Ordering::AcqRel);
            Ok(())
        }
        fn routes(&mut self, _: BTreeMap<u32, crate::hotkeys::Press>) {}
        fn quarantine(&mut self, _: u64) {
            panic!("Own fake cannot fail release");
        }
    }
    fn plan() -> Plan {
        Plan::from_config(&rustcast_core::validate(&rustcast_core::default_config()).unwrap())
            .unwrap()
    }
    #[test]
    fn request_lease_remains_owned_while_native_stage_is_blocked_after_cancellation() {
        let (client, mut server) = channel();
        let pending = client.prepare(plan()).unwrap();
        let (entered, receive) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let count = Arc::new(AtomicU64::new(0));
        let n = count.clone();
        let main = std::thread::spawn(move || {
            let mut registry = Registry::new(Fake {
                entered: Some(entered),
                release: Some(wait),
                count: n,
            });
            server.pump(&mut registry);
            assert!(registry.current().carbon.is_empty());
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        drop(pending);
        assert!(client.prepare(plan()).is_err());
        assert!(client.shared.leased.load(Ordering::Acquire));
        release.send(()).unwrap();
        main.join().unwrap();
        assert!(!client.shared.leased.load(Ordering::Acquire));
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
    #[test]
    fn ready_cancel_and_service_stop_cannot_silently_commit() {
        let (client, mut server) = channel();
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count: count.clone(),
        });
        let mut pending = client.prepare(plan()).unwrap();
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        assert_eq!(client.accepted_generation(), 0);
        assert!(registry.current().carbon.is_empty());
        client.stop();
        assert!(stage.ready().is_err());
        assert!(stage.request_commit().is_err());
        server.pump(&mut registry);
        assert_eq!(client.accepted_generation(), 0);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
    #[test]
    fn keyword_payload_needs_both_native_commit_and_exact_owner_adoption() {
        let (client, mut server) = channel();
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count,
        });
        let config = rustcast_core::validate(&rustcast_core::default_config()).unwrap();
        let prepared = Arc::new(crate::keyword_capture::Prepared::new(&config).unwrap());
        let mut pending = client
            .prepare_with_keywords(plan(), Some(prepared.clone()))
            .unwrap();
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        let generation = stage.generation();
        let mut ack = stage.request_commit().unwrap();
        client.confirm_keywords(generation);
        assert!(client.keyword_prepared().is_none());
        server.pump(&mut registry);
        assert!(ack.poll().unwrap().is_ok());
        assert!(client.keyword_prepared().is_none());
        // Production owner only confirms after actual adoption; an earlier
        // confirmation of an unaccepted stage must not activate it later.
        client.confirm_keywords(0);
        assert!(client.keyword_prepared().is_none());
        client.confirm_keywords(generation + 1);
        assert!(client.keyword_prepared().is_none());
        client.confirm_keywords(generation);
        let (actual, payload) = client.keyword_prepared().unwrap();
        assert_eq!(actual, generation);
        assert!(Arc::ptr_eq(&prepared, &payload));
        client.stop();
        assert!(client.keyword_prepared().is_none());
    }
    #[test]
    fn keyword_stage_epoch_survives_pending_rollback_to_same_confirmed_generation() {
        let (client, mut server) = channel();
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count: Arc::new(AtomicU64::new(0)),
        });
        client.shared.accepted.store(7, Ordering::Release);
        client.confirm_keywords(7);
        let original_epoch = client.keyword_stage_epoch();
        assert_eq!(client.keyword_generation(), 7);
        let pending = client.prepare(plan()).unwrap();
        assert_eq!(client.keyword_generation(), 0);
        assert!(client.keyword_stage_epoch() > original_epoch);
        drop(pending);
        server.pump(&mut registry);
        client.confirm_keywords(7);
        assert_eq!(client.keyword_generation(), 7);
        assert!(client.keyword_stage_epoch() > original_epoch);
    }
    #[test]
    fn activation_sequence_is_exact_and_exhaustion_stops_without_wrapping() {
        let (client, _server) = channel();
        assert_eq!(client.note_external_activation(), Some(1));
        assert_eq!(client.note_external_activation(), Some(2));
        client
            .shared
            .external_activation
            .store(u64::MAX - 1, Ordering::Release);
        assert_eq!(client.note_external_activation(), Some(u64::MAX));
        assert_eq!(client.note_external_activation(), None);
        assert!(!client.running());
        assert_eq!(client.external_activation_epoch(), u64::MAX);
        assert_eq!(client.note_external_activation(), None);
        assert_eq!(client.external_activation_epoch(), u64::MAX);
    }
    #[test]
    fn exhausted_keyword_stage_epoch_stops_without_leaking_admission_lease() {
        let (client, _server) = channel();
        client
            .shared
            .keyword_stage_epoch
            .store(u64::MAX, Ordering::Release);
        assert!(client.prepare(plan()).is_err());
        assert!(!client.running());
        assert!(!client.pending());
        assert_eq!(client.keyword_generation(), 0);
    }
    #[test]
    fn nonblocking_prepare_commits_exact_generation_and_drop_rolls_back() {
        let (client, mut server) = channel();
        let count = Arc::new(AtomicU64::new(0));
        let mut registry = Registry::new(Fake {
            entered: None,
            release: None,
            count: count.clone(),
        });
        let mut pending = client.prepare(plan()).unwrap();
        assert!(pending.poll().is_none());
        server.pump(&mut registry);
        let mut stage = pending.poll().unwrap().unwrap();
        assert!(stage.ready().is_ok());
        let generation = stage.generation();
        let mut activated = stage.request_commit().unwrap();
        assert!(activated.poll().is_none());
        assert_eq!(client.accepted_generation(), 0);
        server.pump(&mut registry);
        assert!(activated.poll().unwrap().is_ok());
        assert_eq!(client.accepted_generation(), generation);
        assert_eq!(registry.current(), &plan());
        drop(stage);
        let mut pending = client.prepare(Plan::default()).unwrap();
        server.pump(&mut registry);
        drop(pending.poll().unwrap().unwrap());
        server.pump(&mut registry);
        assert_eq!(registry.current(), &plan());
        assert_eq!(client.accepted_generation(), generation);
        drop(registry);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
}
