//! Owned workspace/native-shell boundary. No original SDK implementation is
//! invoked. Data/effect admission and callback generations are separate from
//! guardian liveness. Not registered with the incomplete whole-SDK registry.
use crate::sdk::owned::{ModuleContext, OwnedHandleLease, Result};
use std::{
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[path = "workspace_callbacks.rs"]
pub mod callbacks;
#[path = "workspace_cf_loop.rs"]
pub mod cf_loop;
#[path = "workspace_classes.rs"]
pub mod classes;
#[path = "workspace_delegate.rs"]
pub mod delegate;
#[path = "workspace_loop.rs"]
pub mod event_loop;
#[path = "workspace_factory.rs"]
pub mod factory;
#[path = "workspace_images.rs"]
pub mod images;
pub(crate) fn raw_method(
    class: &objc2::runtime::AnyClass,
    selector: objc2::runtime::Sel,
    class_method: bool,
) -> Result<Option<crate::sdk::metadata::MethodSnapshot>> {
    crate::sdk::metadata::method(
        crate::sdk::metadata::registered_class(class as *const objc2::runtime::AnyClass as usize)?,
        selector.name(),
        class_method,
    )
}
/// Strict subset used by scalar/delegate/control callbacks: void/BOOL/id/SEL,
/// optional Objective-C class quotes and numeric stack offsets. Never resolves
/// a method or asks NSMethodSignature/forwarding machinery for missing inputs.
pub(crate) fn simple_abi(method: &crate::sdk::metadata::MethodSnapshot) -> Result<Vec<u8>> {
    let bytes = method.encoding();
    let mut at = 0;
    let mut types = vec![];
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'-') {
            at += 1;
        }
        while at < bytes.len() && b"rnNoORV".contains(&bytes[at]) {
            at += 1;
        }
        if at == bytes.len() {
            break;
        }
        let t = bytes[at];
        at += 1;
        if !b"vBc@:#".contains(&t) {
            return Err("unsupported source callback ABI".into());
        }
        if t == b'@' && at < bytes.len() && bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() && bytes[at] != b'"' {
                at += 1;
            }
            if at == bytes.len() {
                return Err("unterminated callback ABI class name".into());
            }
            at += 1;
        }
        if at < bytes.len() && bytes[at] == b'?' {
            return Err("block is not a plain object callback parameter".into());
        }
        types.push(t);
    }
    Ok(types)
}
#[path = "workspace_notifications.rs"]
pub mod notifications;
#[path = "workspace_receiver.rs"]
pub mod receiver;
#[path = "workspace_roster.rs"]
mod roster;
pub fn source_roster() -> std::collections::BTreeSet<crate::sdk::SourceCallsite> {
    roster::CALLSITES
        .iter()
        .map(|(file, hash, operation)| crate::sdk::SourceCallsite {
            file,
            source_sha256: (*hash).into(),
            operation,
        })
        .chain(notifications::source_roster())
        .chain(crate::sdk::clipboard::source_roster())
        .collect()
}
/// Construction-callsite inputs only. This does not extend the ordinary
/// callback source roster or supply a CallbackCodeAuthority implementation.
pub(crate) fn source_allocation_callsites() -> std::collections::BTreeSet<crate::sdk::SourceCallsite> {
    let sources = crate::sdk::bootstrap::source_allocation_callsites();
    #[cfg(test)]
    let sources = sources.into_iter()
        .chain([receiver::source_construction_canary_callsite()])
        .collect();
    sources
}
pub fn scopes() -> std::collections::BTreeSet<&'static str> {
    [
        "workspace-apps-icons",
        "screens-mouse",
        "open-activate-focus",
    ]
    .into_iter()
    .chain(crate::sdk::clipboard::scopes())
    .collect()
}
#[path = "workspace_objc.rs"]
pub mod objc;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Rect {
    pub origin: Point,
    pub size: Point,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Handle {
    pub id: u64,
    pub generation: u64,
}
#[derive(Clone, Debug)]
pub struct Application {
    pub bundle: Option<String>,
    pub name: Option<String>,
    pub pid: i32,
    pub bundle_path: Option<PathBuf>,
    pub executable_path: Option<PathBuf>,
    pub icon_png: Vec<u8>,
    pub regular: bool,
    pub terminated: bool,
}
/// Reference-only display metadata. The physical bundle remains the directory
/// admitted by Workspace::new under its private applications root. This path
/// conveys no filesystem permission and performs no discovery or file access.
#[cfg(any(rcf1_reference_sdk,test))]
pub(crate) fn reference_bundle_metadata_path(app:&Application)->Option<PathBuf> {
    let path=app.bundle_path.as_ref()?;
    if !app.terminated&&app.bundle.as_deref()==Some("com.apple.Safari")
        &&path.file_name()==Some(std::ffi::OsStr::new("Safari.app")) {
        Some(PathBuf::from("/Applications/Safari.app"))
    } else {Some(path.clone())}
}
#[derive(Clone, Debug)]
pub struct Screen {
    pub frame: Rect,
    pub visible: Rect,
    pub scale: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Open(String),
    Activate(Handle),
    Window(Handle, &'static str),
    Run,
    Stop,
    Menu(Handle),
    Status(Handle, bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub id: u64,
    pub generation: u64,
    pub outcome: Outcome,
    pub action_completed: bool,
}
#[derive(Clone, Debug)]
struct Pending {
    generation: u64,
    outcome: Outcome,
    delivered: bool,
}
/// Closed owned-shell policy, not an observation of desktop window state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRole {
    Window,
    Panel,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub frame: Rect,
    pub visible: bool,
    pub key: bool,
    pub miniaturized: bool,
    pub main: bool,
    pub screen: Option<usize>,
    pub role: WindowRole,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowOrder {
    Front,
    Back,
    Out,
    Close,
    Key,
    Miniaturize,
    Deminiaturize,
}
#[derive(Clone, Copy, Debug)]
struct Window {
    snapshot: WindowSnapshot,
    owner: std::thread::ThreadId,
}
// Preserve existing read-only child-module field access without exposing the
// creator identity or granting mutable access to the public snapshot.
impl std::ops::Deref for Window {
    type Target = WindowSnapshot;
    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}
struct State {
    generation: u64,
    retired: bool,
    next: u64,
    apps: BTreeMap<Handle, Application>,
    screens: Vec<Screen>,
    mouse: Point,
    front: Option<Handle>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    ax_activation: Option<(Handle, crate::sdk::system_providers::ProcessIdentity, &'static AtomicBool)>,
    current: Option<Handle>,
    windows: BTreeMap<Handle, Window>,
    effects: Vec<Effect>,
    open_results: BTreeMap<String, Outcome>,
    pending: BTreeMap<u64, Pending>,
    notifications: Vec<(u64, String, u64)>,
    next_notification: u64,
    status: BTreeMap<Handle, bool>,
}
pub struct Workspace {
    context: ModuleContext,
    lease: OwnedHandleLease,
    faulted: AtomicBool,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fault_site: std::sync::OnceLock<&'static std::panic::Location<'static>>,
    state: Mutex<State>,
    bundle_icons: std::sync::OnceLock<BTreeMap<PathBuf, Arc<images::IconPixels>>>,
}
impl Workspace {
    pub fn new(
        context: ModuleContext,
        apps: Vec<Application>,
        screens: Vec<Screen>,
        mouse: Point,
    ) -> Result<Arc<Self>> {
        let lease = context.lease("workspace-native-shell")?;
        if screens.is_empty() || !mouse.x.is_finite() || !mouse.y.is_finite() {
            return Err("invalid owned screen/mouse fixture".into());
        }
        for s in &screens {
            if !valid_rect(s.frame)
                || !valid_rect(s.visible)
                || !s.scale.is_finite()
                || s.scale <= 0.0
            {
                return Err("invalid screen fixture".into());
            }
        }
        let mut owned = BTreeMap::new();
        let mut pids = std::collections::BTreeSet::new();
        for (index, app) in apps.into_iter().enumerate() {
            if app.pid <= 0
                || app
                    .bundle
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > 1024 || id.contains('\0'))
                || !pids.insert(app.pid)
                || app.icon_png.is_empty()
            {
                return Err("unowned or duplicate application fixture".into());
            }
            if let Some(path) = &app.bundle_path {
                if !context.applications.info(path)?.0 {
                    return Err("fixture bundle must be an owned directory".into());
                }
            }
            if let Some(path) = &app.executable_path {
                if context.applications.info(path)?.0 {
                    return Err("fixture executable metadata must be an owned regular file".into());
                }
            }
            owned.insert(
                Handle {
                    id: index as u64 + 1,
                    generation: 1,
                },
                app,
            );
        }
        let next = owned.len() as u64 + 1;
        Ok(Arc::new(Self {
            context,
            lease,
            faulted: AtomicBool::new(false),
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            fault_site: std::sync::OnceLock::new(),
            bundle_icons: std::sync::OnceLock::new(),
            state: Mutex::new(State {
                generation: 1,
                retired: false,
                next,
                apps: owned,
                screens,
                mouse,
                front: None,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                ax_activation: None,
                current: None,
                windows: BTreeMap::new(),
                effects: vec![],
                open_results: BTreeMap::new(),
                pending: BTreeMap::new(),
                notifications: vec![],
                next_notification: 1,
                status: BTreeMap::new(),
            }),
        }))
    }
    fn with<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        if self.faulted.load(Ordering::Acquire) {
            return Err("workspace authority retired after failure".into());
        }
        match catch_unwind(AssertUnwindSafe(|| self.lease.before_effect())) {
            Ok(Ok(())) => (),
            outcome => {
                self.fault();
                return match outcome {
                    Ok(Err(error)) => Err(error),
                    _ => Err("workspace guardian panicked; authority retired".into()),
                };
            }
        }
        match catch_unwind(AssertUnwindSafe(|| {
            let mut s = self.state.lock().map_err(|_| "workspace state poisoned")?;
            if s.retired {
                return Err("workspace generation retired".into());
            }
            f(&mut s)
        })) {
            Ok(result) => result,
            Err(_) => {
                self.fault();
                Err("workspace operation panicked; unknown outcome, no retry".into())
            }
        }
    }
    pub fn applications(&self) -> Result<Vec<(Handle, Application)>> {
        self.with(|s| Ok(s.apps.iter().map(|(h, a)| (*h, a.clone())).collect()))
    }
    pub fn application(&self, h: Handle) -> Result<Application> {
        self.with(|s| {
            s.apps
                .get(&h)
                .cloned()
                .ok_or("unknown/expired application".into())
        })
    }
    pub fn by_bundle(&self, bundle: &str) -> Result<Vec<Handle>> {
        self.with(|s| {
            Ok(s.apps
                .iter()
                .filter(|(_, a)| a.bundle.as_deref() == Some(bundle) && !a.terminated)
                .map(|(h, _)| *h)
                .collect())
        })
    }
    pub fn by_pid(&self, pid: i32) -> Result<Option<Handle>> {
        self.with(|s| {
            Ok(s.apps
                .iter()
                .find(|(_, a)| a.pid == pid && !a.terminated)
                .map(|(h, _)| *h))
        })
    }
    pub fn installed_application(&self, bundle: &str) -> Result<Option<Application>> {
        self.with(|s| {
            Ok(s.apps
                .values()
                .find(|a| a.bundle.as_deref() == Some(bundle) && a.bundle_path.is_some())
                .cloned())
        })
    }
    pub(crate) fn icon_for_owned_bundle(&self, path: &Path) -> Result<Arc<images::IconPixels>> {
        self.with(|state| {
            let mut matches = state.apps.values().filter(|app| {
                !app.terminated && app.bundle_path.as_deref() == Some(path)
            });
            matches.next().ok_or("owned application icon absent")?;
            if matches.next().is_some() {
                return Err("owned application icon path is ambiguous".into());
            }
            Ok(())
        })?;
        if !self.context.applications.info(path)?.0 {
            return Err("owned application icon path is not a bundle directory".into());
        }
        self.bundle_icons.get().and_then(|icons| icons.get(path)).cloned()
            .ok_or_else(|| "bundle icon was not decoded before factory installation".into())
    }
    /// Must run before Foundation class overlays. ImageIO can use original
    /// Foundation objects internally, so only plain pixels survive this step.
    #[cfg(any(test, rcf1_reference_sdk))]
    pub(crate) fn prepare_bundle_icons(&self) -> Result<()> {
        if self.bundle_icons.get().is_some() {
            return Err("bundle icons already prepared".into());
        }
        let apps = self.with(|state| Ok(state.apps.values().filter_map(|app| {
            app.bundle_path.clone().map(|path| (path, app.icon_png.clone()))
        }).collect::<Vec<_>>()))?;
        let mut icons = BTreeMap::new();
        for (path, png) in apps {
            if !self.context.applications.info(&path)?.0 {
                return Err("owned application icon path is not a bundle directory".into());
            }
            let image = images::decode_png(&png, self.lease())?;
            if icons.insert(path, image.icon_pixels()).is_some() {
                return Err("duplicate owned application icon path".into());
            }
        }
        self.with(|_| Ok(()))?;
        self.bundle_icons.set(icons).map_err(|_| "bundle icons already prepared".into())
    }
    pub fn frontmost(&self) -> Result<Option<Handle>> {
        self.with(|s| Ok(s.front))
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn with_reference_frontmost<T>(
        &self, pid: i32, consume_activation: bool, commit: impl FnOnce(&AtomicBool, &AtomicBool) -> T,
    ) -> Result<T> {
        self.with(|state| {
            let receipt = if consume_activation {
                state.ax_activation.take()
            } else {
                state.ax_activation
            };
            let (handle, identity, stopped) = receipt.ok_or("Reference AX action has no qualified activation")?;
            if self.faulted.load(Ordering::Acquire)
                || identity.pid != pid || state.front != Some(handle)
                || !state.apps.get(&handle)
                    .is_some_and(|app| !app.terminated && app.pid == pid)
            {
                return Err("Reference AX action target is not the frontmost application".into());
            }
            // Only the AX model's callback-free commit runs under this lock.
            crate::sdk::process_router::with_virtual_identity(identity, || Ok(commit(&self.faulted, stopped)))
        })
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn clear_reference_ax_activation(&self) -> Result<()> {
        self.with(|state| {
            state.ax_activation = None;
            Ok(())
        })
    }
    /// The coordinator supplies the synthetic self-app record. Its PID must be
    /// this owned process; std::process::id is not a desktop/process inventory.
    pub fn select_current_application(&self, handle: Handle) -> Result<()> {
        self.with(|s| {
            let app = s
                .apps
                .get(&handle)
                .ok_or("unknown current application fixture")?;
            if app.terminated || app.pid as u32 != std::process::id() {
                return Err("current application is not this owned process".into());
            }
            if s.current.is_some() {
                return Err("current application already selected".into());
            }
            s.current = Some(handle);
            Ok(())
        })
    }
    pub fn current_application(&self) -> Result<Option<Handle>> {
        self.with(|s| Ok(s.current))
    }
    pub fn screens(&self) -> Result<Vec<Screen>> {
        self.with(|s| Ok(s.screens.clone()))
    }
    pub fn mouse(&self) -> Result<Point> {
        self.with(|s| Ok(s.mouse))
    }
    pub fn activate(&self, h: Handle) -> Result<bool> {
        self.with(|state| Self::activate_in_state(state, h))
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn activate_reference_ax(
        &self, handle: Handle, identity: crate::sdk::system_providers::ProcessIdentity, stopped: &'static AtomicBool,
    ) -> Result<bool> {
        self.with(|state| {
            crate::sdk::process_router::with_virtual_identity(identity, || {
                if self.faulted.load(Ordering::Acquire) || stopped.load(Ordering::Acquire)
                    || state.apps.get(&handle).is_none_or(|app| app.pid != identity.pid)
                {
                    return Err("Reference AX activation target or runtime changed".into());
                }
                let activated = Self::activate_in_state(state, handle)?;
                state.ax_activation = Some((handle, identity, stopped));
                Ok(activated)
            })
        })
    }
    fn activate_in_state(state: &mut State, handle: Handle) -> Result<bool> {
        if state.apps.get(&handle).is_none_or(|app| app.terminated) {
            return Err("unknown/expired activation target".into());
        }
        state.front = Some(handle);
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        { state.ax_activation = None; }
        state.effects.push(Effect::Activate(handle));
        Ok(true)
    }
    pub fn set_open_result(&self, url: String, outcome: Outcome) -> Result<()> {
        self.qualify_url(&url)?;
        self.with(|s| {
            s.open_results.insert(url, outcome);
            Ok(())
        })
    }
    fn qualify_url(&self, url: &str) -> Result<()> {
        if let Some(path) = url.strip_prefix("file://") {
            self.context
                .home
                .info(&PathBuf::from(path))
                .or_else(|_| self.context.applications.info(&PathBuf::from(path)))?;
            return Ok(());
        }
        if url.starts_with("https://fixture.invalid/")
            || url.starts_with("obsidian://open?vault=OwnedFixture&file=")
        {
            return Ok(());
        }
        Err("unknown/unowned opener target".into())
    }
    pub fn open(&self, url: &str) -> Result<Outcome> {
        self.qualify_url(url)?;
        self.with(|s| {
            let outcome = *s
                .open_results
                .get(url)
                .ok_or("opener target lacks configured outcome")?;
            s.effects.push(Effect::Open(url.into()));
            Ok(outcome)
        })
    }
    pub fn begin_open(&self, url: &str) -> Result<Completion> {
        self.begin_open_with(url, |_| Ok(()))
    }
    // The owner queue may publish its already retained block in this commit.
    // The callback must only update Rust data and must not re-enter Workspace.
    fn begin_open_with(
        &self,
        url: &str,
        publish: impl FnOnce(&Completion) -> Result<()>,
    ) -> Result<Completion> {
        self.qualify_url(url)?;
        self.with(|s| {
            let outcome = *s.open_results.get(url).ok_or("opener target lacks configured outcome")?;
            Self::commit_open(s, url, outcome, publish)
        })
    }
    fn commit_open(
        state: &mut State, url: &str, outcome: Outcome,
        publish: impl FnOnce(&Completion) -> Result<()>,
    ) -> Result<Completion> {
        let id = state.next;
        let next = id.checked_add(1).ok_or("workspace ID exhausted")?;
        let generation = state.generation;
        let completion = Completion {
            id, generation, outcome, action_completed: outcome != Outcome::Failure,
        };
        if state.pending.contains_key(&id) {
            return Err("workspace completion ID already pending".into());
        }
        publish(&completion)?;
        state.next = next;
        state.effects.push(Effect::Open(url.into()));
        state.pending.insert(id, Pending { generation, outcome, delivered: false });
        Ok(completion)
    }
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    fn begin_owned_application_open(
        &self, path: &Path, url: &str, opens_url: bool,
        publish: impl FnOnce(Handle, &Completion) -> Result<()>,
    ) -> Result<Completion> {
        if !self.context.applications.info(path)?.0 {
            return Err("Reference open target is not an owned application directory".into());
        }
        if opens_url {
            self.qualify_url(url)?;
        }
        self.with(|state| {
            let mut matches = state.apps.iter().filter(|(_, app)| {
                !app.terminated && app.bundle_path.as_deref() == Some(path)
            });
            let (&handle, _) = matches.next().ok_or("Reference open target application absent")?;
            if matches.next().is_some() {
                return Err("Reference open target application ambiguous".into());
            }
            // The private application model is the launch provider for this
            // fixture. A configured failure or unknown outcome takes precedence.
            let configured = state.open_results.get(url).copied();
            let outcome = if opens_url {
                configured.ok_or("opener target lacks configured outcome")?
            } else {
                configured.unwrap_or(Outcome::Success)
            };
            Self::commit_open(state, url, outcome, |completion| publish(handle, completion))
        })
    }
    /// Exactly once. Unknown outcomes are never replayed; retirement rejects
    /// stale callbacks while retaining callback ownership in the integration.
    pub fn complete(&self, c: &Completion) -> Result<Completion> {
        self.complete_if(c, || Ok(true)).map(|_| c.clone())
    }

    // The predicate must be callback-free: it runs inside the state lock.
    fn complete_if(&self, c: &Completion, qualified: impl FnOnce() -> Result<bool>) -> Result<bool> {
        self.with(|s| {
            if self.faulted.load(Ordering::Acquire) {
                return Err("workspace authority retired before completion".into());
            }
            let p = s.pending.get_mut(&c.id).ok_or("unknown callback")?;
            if p.generation != c.generation
                || c.generation != s.generation
                || p.outcome != c.outcome
                || p.delivered
            {
                return Err("stale/duplicate/mismatched callback".into());
            }
            if !qualified()? {
                return Ok(false);
            }
            p.delivered = true;
            Ok(true)
        })
    }
    pub fn new_window(&self, frame: Rect) -> Result<Handle> {
        self.new_window_typed(frame, WindowRole::Window, None)
    }
    pub fn new_window_typed(
        &self,
        frame: Rect,
        role: WindowRole,
        screen: Option<usize>,
    ) -> Result<Handle> {
        if !valid_rect(frame) {
            return Err("invalid window frame".into());
        }
        self.with(|s| {
            if screen.is_some_and(|index| index >= s.screens.len()) {
                return Err("unknown owned window screen".into());
            }
            let h = Handle {
                id: s.next,
                generation: s.generation,
            };
            let next = s.next.checked_add(1).ok_or("workspace ID exhausted")?;
            let screen = window_screen(frame, &s.screens, screen)?;
            s.windows.insert(
                h,
                Window {
                    snapshot: WindowSnapshot {
                        frame,
                        visible: false,
                        key: false,
                        miniaturized: false,
                        main: false,
                        screen,
                        role,
                    },
                    owner: std::thread::current().id(),
                },
            );
            s.next = next;
            Ok(h)
        })
    }
    pub fn window_action(&self, h: Handle, action: &'static str) -> Result<()> {
        self.window_order(
            h,
            match action {
                "front" => WindowOrder::Front,
                "key" => WindowOrder::Key,
                "out" => WindowOrder::Out,
                "close" => WindowOrder::Close,
                _ => return Err("unknown window effect".into()),
            },
        )
    }
    /// Explicit deterministic owned policy: showing clears miniaturization;
    /// miniaturizing hides and resigns key/main; deminiaturizing never steals
    /// either. Panels can be key but cannot become main in this shell.
    pub fn window_order(&self, h: Handle, order: WindowOrder) -> Result<()> {
        self.window_order_with_resignation(h, order).map(|_| ())
    }
    fn window_order_with_resignation(&self, h: Handle, order: WindowOrder) -> Result<Option<Handle>> {
        self.with(|s| {
            let role = s.windows.get(&h).ok_or("unknown window")?.snapshot.role;
            let resigned = match order {
                WindowOrder::Key => s.windows.iter().find_map(|(&handle, window)| {
                    (handle != h && window.snapshot.key).then_some(handle)
                }),
                WindowOrder::Out | WindowOrder::Close | WindowOrder::Miniaturize => {
                    s.windows[&h].snapshot.key.then_some(h)
                }
                _ => None,
            };
            if order == WindowOrder::Key {
                for window in s.windows.values_mut() {
                    window.snapshot.key = false;
                    if role == WindowRole::Window {
                        window.snapshot.main = false;
                    }
                }
            }
            let w = &mut s.windows.get_mut(&h).ok_or("unknown window")?.snapshot;
            let label = match order {
                WindowOrder::Front => {
                    w.visible = true;
                    w.miniaturized = false;
                    "front"
                }
                WindowOrder::Back => {
                    w.visible = true;
                    w.miniaturized = false;
                    "back"
                }
                WindowOrder::Key => {
                    w.visible = true;
                    w.miniaturized = false;
                    w.key = true;
                    w.main = role == WindowRole::Window;
                    "key"
                }
                WindowOrder::Out | WindowOrder::Close => {
                    w.visible = false;
                    w.key = false;
                    w.main = false;
                    if order == WindowOrder::Close {
                        "close"
                    } else {
                        "out"
                    }
                }
                WindowOrder::Miniaturize => {
                    w.visible = false;
                    w.key = false;
                    w.main = false;
                    w.miniaturized = true;
                    "miniaturize"
                }
                WindowOrder::Deminiaturize => {
                    w.visible = true;
                    w.miniaturized = false;
                    "deminiaturize"
                }
            };
            s.effects.push(Effect::Window(h, label));
            Ok(resigned)
        })
    }
    /// Physical cleanup for an existing allocation, also after business
    /// retirement. This invokes no guardian or callback and mints no handle.
    pub(crate) fn retire_window_allocation(&self, h: Handle) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| "workspace state poisoned")?;
        let window = state.windows.get(&h).ok_or("unknown window allocation")?;
        if window.owner != std::thread::current().id() {
            return Err("window cleanup on foreign owner thread".into());
        }
        state
            .windows
            .remove(&h)
            .ok_or("window allocation disappeared")?;
        Ok(())
    }
    pub fn window_snapshot(&self, h: Handle) -> Result<WindowSnapshot> {
        self.with(|s| {
            s.windows
                .get(&h)
                .map(|w| w.snapshot)
                .ok_or("unknown window".into())
        })
    }
    pub fn window(&self, h: Handle) -> Result<(Rect, bool, bool)> {
        self.window_snapshot(h).map(|w| (w.frame, w.visible, w.key))
    }
    pub fn set_frame(&self, h: Handle, frame: Rect) -> Result<()> {
        self.with(|s| set_window_frame(s, h, frame))
    }
    pub fn set_window_origin(&self, h: Handle, origin: Point) -> Result<()> {
        self.with(|s| {
            let mut frame = s.windows.get(&h).ok_or("unknown window")?.snapshot.frame;
            frame.origin = origin;
            set_window_frame(s, h, frame)
        })
    }
    pub fn set_window_size(&self, h: Handle, size: Point) -> Result<()> {
        self.with(|s| {
            let mut frame = s.windows.get(&h).ok_or("unknown window")?.snapshot.frame;
            frame.size = size;
            set_window_frame(s, h, frame)
        })
    }
    /// Center within the owned visible rectangle. No AppKit optical placement,
    /// desktop, Space, backing-store or style-mask inference is performed.
    pub fn center_window(&self, h: Handle) -> Result<()> {
        self.with(|s| {
            let w = s.windows.get(&h).ok_or("unknown window")?.snapshot;
            let screen = w.screen.unwrap_or(0);
            let visible = s
                .screens
                .get(screen)
                .ok_or("unknown owned window screen")?
                .visible;
            let frame = Rect {
                origin: Point {
                    x: visible.origin.x + (visible.size.x - w.frame.size.x) / 2.0,
                    y: visible.origin.y + (visible.size.y - w.frame.size.y) / 2.0,
                },
                size: w.frame.size,
            };
            set_window_frame(s, h, frame)
        })
    }
    pub fn subscribe(&self, name: &str) -> Result<u64> {
        if ![
            "NSWorkspaceDidActivateApplicationNotification",
            "NSWorkspaceDidLaunchApplicationNotification",
            "NSWorkspaceDidTerminateApplicationNotification",
            "NSWorkspaceWillSleepNotification",
            "NSWorkspaceDidWakeNotification",
            "NSWorkspaceSessionDidBecomeActiveNotification",
            "NSWorkspaceSessionDidResignActiveNotification",
            "NSWindowDidResignKeyNotification",
            "NSWindowDidResizeNotification",
            "NSViewBoundsDidChangeNotification",
        ]
        .contains(&name)
        {
            return Err("unknown workspace notification".into());
        }
        self.with(|s| {
            let id = s.next_notification;
            s.next_notification = s
                .next_notification
                .checked_add(1)
                .ok_or("notification IDs exhausted")?;
            s.notifications.push((id, name.into(), s.generation));
            Ok(id)
        })
    }
    pub fn unsubscribe(&self, id: u64) -> Result<()> {
        self.with(|s| {
            let n = s.notifications.len();
            s.notifications.retain(|(i, _, _)| *i != id);
            if n == s.notifications.len() {
                return Err("unknown observer".into());
            }
            Ok(())
        })
    }
    // Roll back only an ID reserved by the hub and never made dispatchable.
    // Cleanup remains possible after a token factory faults the guardian.
    fn discard_pending_notification(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.notifications.retain(|(candidate, _, _)| *candidate != id);
        }
    }
    pub fn notification_recipients(&self, name: &str) -> Result<Vec<u64>> {
        self.with(|s| {
            Ok(s.notifications
                .iter()
                .filter(|(_, n, g)| n == name && *g == s.generation)
                .map(|(id, _, _)| *id)
                .collect())
        })
    }
    pub fn status_item(&self) -> Result<Handle> {
        self.with(|s| {
            let h = Handle {
                id: s.next,
                generation: s.generation,
            };
            s.next = s.next.checked_add(1).ok_or("workspace ID exhausted")?;
            s.status.insert(h, true);
            s.effects.push(Effect::Status(h, true));
            Ok(h)
        })
    }
    pub fn remove_status(&self, h: Handle) -> Result<()> {
        self.with(|s| {
            let v = s.status.get_mut(&h).ok_or("unknown status item")?;
            if !*v {
                return Err("retired status item".into());
            }
            *v = false;
            s.effects.push(Effect::Status(h, false));
            Ok(())
        })
    }
    pub fn effects(&self) -> Result<Vec<Effect>> {
        self.with(|s| Ok(s.effects.clone()))
    }
    pub fn retire(&self) -> Result<()> {
        let mut s = self.state.lock().map_err(|_| "workspace state poisoned")?;
        s.retired = true;
        s.generation = s
            .generation
            .checked_add(1)
            .ok_or("workspace generations exhausted")?;
        Ok(())
    }
    pub fn lease(&self) -> OwnedHandleLease {
        self.lease.clone()
    }
    /// Callback-free owner fence for backend data already bound to this model.
    /// It never invokes a guardian while another backend's state is locked.
    pub(crate) fn generation_status(&self, generation: u64) -> std::result::Result<(), &'static str> {
        let state=self.state.lock().map_err(|_|"workspace state poisoned")?;
        if self.faulted.load(Ordering::Acquire) { return Err("workspace faulted"); }
        if state.retired { return Err("workspace retired"); }
        if state.generation != generation { return Err("workspace generation changed"); }
        Ok(())
    }
    #[track_caller]
    fn fault(&self) {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let _ = self.fault_site.set(std::panic::Location::caller());
        self.faulted.store(true, Ordering::Release);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn fault_site(&self) -> Option<&'static std::panic::Location<'static>> {
        self.fault_site.get().copied()
    }
    pub(crate) fn backend_generation(&self)->Result<u64> {
        self.with(|state|Ok(state.generation))
    }
    /// Bind this exact model's private context to the sealed Reference issuer.
    /// This is a preparation/prewrite guard, not a class or callback grant.
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn verify_reference_context(&self,admission:&crate::sdk::core_preentry::ReferenceAdmission)->Result<()> {
        self.verify_reference_context_with(admission, || Ok(()))
    }

    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn verify_reference_clipboard_context(&self,admission:&crate::sdk::core_preentry::ReferenceAdmission)->Result<()> {
        self.verify_reference_context_with(admission, || admission.verify_clipboard_manifest_members())
    }

    #[cfg(all(rcf1_reference_sdk,not(test)))]
    fn verify_reference_context_with(&self,admission:&crate::sdk::core_preentry::ReferenceAdmission,verify_members:impl FnOnce()->Result<()>)->Result<()> {
        struct FaultOnFailure<'a> {model:&'a Workspace,complete:bool}
        impl Drop for FaultOnFailure<'_> {
            fn drop(&mut self) {if !self.complete {self.model.fault();}}
        }
        let mut failure=FaultOnFailure{model:self,complete:false};
        // Establish the true context/guardian before invoking this model's
        // dynamic lease. A caller-supplied foreign guardian is never called.
        admission.verify_input_context(&self.context)?;
        verify_members()?;
        let generation=self.with(|state|Ok(state.generation))?;
        admission.verify_input_context(&self.context)?;
        // No dynamic callback follows the final sealed source/retirement guard.
        let state=self.state.lock().map_err(|_|"Reference model state poisoned")?;
        if self.faulted.load(Ordering::Acquire) || state.retired || state.generation!=generation {
            return Err("Reference model retired or changed during context qualification".into());
        }
        failure.complete=true;
        Ok(())
    }
}
fn set_window_frame(s: &mut State, h: Handle, frame: Rect) -> Result<()> {
    if !valid_rect(frame) {
        return Err("invalid window geometry".into());
    }
    let previous = s.windows.get(&h).ok_or("unknown window")?.snapshot.screen;
    let screen = window_screen(frame, &s.screens, previous)?;
    let window = &mut s.windows.get_mut(&h).ok_or("unknown window")?.snapshot;
    window.frame = frame;
    window.screen = screen;
    Ok(())
}
/// Largest positive overlap among owned screens; preserve the current screen
/// on a tie. Entirely off-screen windows have no screen.
fn window_screen(
    frame: Rect,
    screens: &[Screen],
    previous: Option<usize>,
) -> Result<Option<usize>> {
    let area = |screen: &Screen| -> Result<f64> {
        let width = ((frame.origin.x + frame.size.x)
            .min(screen.frame.origin.x + screen.frame.size.x)
            - frame.origin.x.max(screen.frame.origin.x))
        .max(0.0);
        let height = ((frame.origin.y + frame.size.y)
            .min(screen.frame.origin.y + screen.frame.size.y)
            - frame.origin.y.max(screen.frame.origin.y))
        .max(0.0);
        let area = width * height;
        if !area.is_finite() || (width > 0.0 && height > 0.0 && area == 0.0) {
            return Err("owned screen overlap outside numeric range".into());
        }
        Ok(area)
    };
    let overlaps = screens.iter().map(area).collect::<Result<Vec<_>>>()?;
    let mut selected = previous.filter(|index| *index < overlaps.len() && overlaps[*index] > 0.0);
    let mut largest = selected.map_or(0.0, |index| overlaps[index]);
    for (index, overlap) in overlaps.into_iter().enumerate() {
        if overlap > largest {
            selected = Some(index);
            largest = overlap;
        }
    }
    Ok(selected)
}
fn valid_rect(r: Rect) -> bool {
    [r.origin.x, r.origin.y, r.size.x, r.size.y]
        .iter()
        .all(|v| v.is_finite())
        && r.size.x > 0.0
        && r.size.y > 0.0
        && (r.origin.x + r.size.x).is_finite()
        && (r.origin.y + r.size.y).is_finite()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::sdk::owned::{AliveLease, OwnedRoot};
    use std::{
        fs,
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
    };
    pub struct Life(pub AtomicBool, pub std::sync::atomic::AtomicUsize);
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            assert_ne!(self.1.load(Ordering::Acquire), 2, "own is_alive panic");
            self.0.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<()> {
            assert_ne!(self.1.load(Ordering::Acquire), 1, "own before_effect panic");
            if self.is_alive() {
                Ok(())
            } else {
                Err("owned guardian retired".into())
            }
        }
    }
    pub struct Fixture {
        pub root: PathBuf,
        pub life: Arc<Life>,
        pub model: Arc<Workspace>,
    }
    impl Fixture {
        pub fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = PathBuf::from(
                std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT")
                    .expect("explicit owned fixture parent required"),
            );
            assert!(parent.is_absolute());
            let root = parent.join(format!(
                "workspace-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            for dir in [
                "home",
                "cache",
                "temp",
                "apps",
                "apps/Owned.app",
                "apps/Owned.app/Contents",
            ] {
                fs::create_dir(root.join(dir)).unwrap();
            }
            fs::write(
                root.join("apps/Owned.app/Contents/owned"),
                b"not an executable; synthetic bytes",
            )
            .unwrap();
            let life = Arc::new(Life(
                AtomicBool::new(true),
                std::sync::atomic::AtomicUsize::new(0),
            ));
            let context = ModuleContext {
                home: OwnedRoot::open(&root.join("home")).unwrap(),
                cache: OwnedRoot::open(&root.join("cache")).unwrap(),
                temp: OwnedRoot::open(&root.join("temp")).unwrap(),
                applications: OwnedRoot::open(&root.join("apps")).unwrap(),
                life: life.clone(),
            };
            let rect = Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Point {
                    x: 1280.0,
                    y: 800.0,
                },
            };
            let app = Application {
                bundle: Some("dev.fixture.Owned".into()),
                name: Some("Owned synthetic app".into()),
                pid: 701234,
                bundle_path: Some(root.join("apps/Owned.app")),
                executable_path: Some(root.join("apps/Owned.app/Contents/owned")),
                icon_png: vec![1, 2, 3],
                regular: true,
                terminated: false,
            };
            let model = Workspace::new(
                context,
                vec![app],
                vec![Screen {
                    frame: rect,
                    visible: rect,
                    scale: 2.0,
                }],
                Point { x: 50.0, y: 60.0 },
            )
            .unwrap();
            Self { root, life, model }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.model.retire().unwrap();
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn owned_window_cleanup_is_creator_bound_and_survives_business_retirement() {
        let f = Fixture::new();
        let frame = f.model.screens().unwrap()[0].frame;
        let window = f.model.new_window(frame).unwrap();
        let keep = f
            .model
            .new_window_typed(frame, WindowRole::Panel, None)
            .unwrap();
        let stale = Handle {
            id: window.id,
            generation: window.generation + 1,
        };
        assert!(f.model.retire_window_allocation(stale).is_err());
        assert!(f
            .model
            .retire_window_allocation(Handle {
                id: u64::MAX,
                generation: window.generation
            })
            .is_err());
        let other = f.model.clone();
        assert!(
            std::thread::spawn(move || other.retire_window_allocation(window))
                .join()
                .unwrap()
                .is_err()
        );
        let before = f.model.window_snapshot(window).unwrap();
        let effects = f.model.effects().unwrap();
        f.model.retire().unwrap();
        f.model.faulted.store(true, Ordering::Release);
        // Any guardian callback now panics; physical cleanup must not call it.
        f.life.1.store(1, Ordering::Release);
        f.model.retire_window_allocation(window).unwrap();
        assert!(f.model.retire_window_allocation(window).is_err());
        let state = f.model.state.lock().unwrap();
        assert!(!state.windows.contains_key(&window));
        assert_eq!(state.windows.len(), 1);
        assert!(state.windows.contains_key(&keep));
        assert_eq!(state.generation, window.generation + 1);
        assert!(state.retired);
        assert_eq!(state.effects, effects);
        assert_eq!(before.frame, frame);
    }
    #[test]
    fn owned_window_roles_ordering_and_legacy_tuple_keep_one_key_and_main() {
        let f = Fixture::new();
        let frame = f.model.screens().unwrap()[0].frame;
        let first = f.model.new_window(frame).unwrap();
        let second = f
            .model
            .new_window_typed(frame, WindowRole::Window, Some(0))
            .unwrap();
        let panel = f
            .model
            .new_window_typed(frame, WindowRole::Panel, None)
            .unwrap();
        let initial = f.model.window_snapshot(panel).unwrap();
        assert_eq!(
            (
                initial.visible,
                initial.key,
                initial.main,
                initial.miniaturized
            ),
            (false, false, false, false)
        );
        f.model.window_action(first, "key").unwrap();
        f.model.window_order(panel, WindowOrder::Key).unwrap();
        assert!(f.model.window_snapshot(first).unwrap().main);
        assert!(!f.model.window_snapshot(first).unwrap().key);
        assert!(f.model.window_snapshot(panel).unwrap().key);
        assert!(!f.model.window_snapshot(panel).unwrap().main);
        f.model.window_order(second, WindowOrder::Key).unwrap();
        assert!(!f.model.window_snapshot(first).unwrap().main);
        assert!(!f.model.window_snapshot(panel).unwrap().key);
        assert_eq!(f.model.window(second).unwrap(), (frame, true, true));
        f.model
            .window_order(second, WindowOrder::Miniaturize)
            .unwrap();
        let mini = f.model.window_snapshot(second).unwrap();
        assert_eq!(
            (mini.visible, mini.key, mini.main, mini.miniaturized),
            (false, false, false, true)
        );
        f.model.window_order(panel, WindowOrder::Key).unwrap();
        f.model
            .window_order(second, WindowOrder::Deminiaturize)
            .unwrap();
        let restored = f.model.window_snapshot(second).unwrap();
        assert_eq!(
            (
                restored.visible,
                restored.key,
                restored.main,
                restored.miniaturized
            ),
            (true, false, false, false)
        );
        assert!(f.model.window_snapshot(panel).unwrap().key);
        f.model.window_order(second, WindowOrder::Back).unwrap();
        f.model.window_action(panel, "out").unwrap();
        f.model.window_order(second, WindowOrder::Close).unwrap();
        assert_eq!(f.model.window(second).unwrap(), (frame, false, false));
        assert!(!f.model.window_snapshot(panel).unwrap().key);
    }
    #[test]
    fn owned_window_geometry_screen_ties_and_center_are_fixture_bound() {
        let f = Fixture::new();
        f.model
            .with(|s| {
                s.screens = vec![
                    Screen {
                        frame: Rect {
                            origin: Point { x: 0.0, y: 0.0 },
                            size: Point { x: 100.0, y: 100.0 },
                        },
                        visible: Rect {
                            origin: Point { x: 0.0, y: 10.0 },
                            size: Point { x: 100.0, y: 80.0 },
                        },
                        scale: 1.0,
                    },
                    Screen {
                        frame: Rect {
                            origin: Point { x: 100.0, y: 0.0 },
                            size: Point { x: 100.0, y: 100.0 },
                        },
                        visible: Rect {
                            origin: Point { x: 100.0, y: 0.0 },
                            size: Point { x: 100.0, y: 100.0 },
                        },
                        scale: 2.0,
                    },
                ];
                Ok(())
            })
            .unwrap();
        let frame = Rect {
            origin: Point { x: 90.0, y: 20.0 },
            size: Point { x: 20.0, y: 20.0 },
        };
        let window = f
            .model
            .new_window_typed(frame, WindowRole::Window, Some(1))
            .unwrap();
        f.model.set_frame(window, frame).unwrap();
        assert_eq!(f.model.window_snapshot(window).unwrap().screen, Some(1));
        f.model
            .set_window_origin(window, Point { x: 5.0, y: 10.0 })
            .unwrap();
        assert_eq!(f.model.window_snapshot(window).unwrap().screen, Some(0));
        f.model.center_window(window).unwrap();
        assert_eq!(
            f.model.window_snapshot(window).unwrap().frame.origin,
            Point { x: 40.0, y: 40.0 }
        );
        f.model
            .set_window_size(window, Point { x: 40.0, y: 30.0 })
            .unwrap();
        f.model
            .set_window_origin(
                window,
                Point {
                    x: -200.0,
                    y: -200.0,
                },
            )
            .unwrap();
        assert_eq!(f.model.window_snapshot(window).unwrap().screen, None);
        f.model.center_window(window).unwrap();
        let centered = f.model.window_snapshot(window).unwrap();
        assert_eq!(centered.screen, Some(0));
        assert_eq!(centered.frame.origin, Point { x: 30.0, y: 35.0 });
        assert!(!centered.visible);
    }
    #[test]
    fn owned_window_invalid_geometry_handles_and_retirement_never_mutate() {
        let f = Fixture::new();
        let frame = f.model.screens().unwrap()[0].frame;
        let window = f.model.new_window(frame).unwrap();
        let before = f.model.window_snapshot(window).unwrap();
        let effects = f.model.effects().unwrap();
        assert!(f
            .model
            .new_window_typed(frame, WindowRole::Panel, Some(usize::MAX))
            .is_err());
        assert!(f
            .model
            .set_window_origin(
                window,
                Point {
                    x: f64::NAN,
                    y: 0.0
                }
            )
            .is_err());
        assert!(f
            .model
            .set_window_size(window, Point { x: 0.0, y: 1.0 })
            .is_err());
        assert!(f
            .model
            .set_frame(
                window,
                Rect {
                    origin: Point {
                        x: f64::MAX,
                        y: 0.0
                    },
                    size: Point {
                        x: f64::MAX,
                        y: 1.0
                    }
                }
            )
            .is_err());
        let stale = Handle {
            id: window.id,
            generation: window.generation + 1,
        };
        assert!(f.model.window_order(stale, WindowOrder::Key).is_err());
        assert!(f.model.center_window(stale).is_err());
        assert_eq!(f.model.window_snapshot(window).unwrap(), before);
        assert_eq!(f.model.effects().unwrap(), effects);
        f.model.retire().unwrap();
        assert!(f.model.window_order(window, WindowOrder::Key).is_err());
        assert!(f.model.set_frame(window, frame).is_err());
        assert!(f.model.window_snapshot(window).is_err());
        let state = f.model.state.lock().unwrap();
        assert_eq!(state.windows.get(&window).unwrap().snapshot, before);
        assert_eq!(state.effects, effects);
    }
    #[test]
    fn typed_fixture_never_looks_up_unknown_targets_or_replays_effects() {
        let f = Fixture::new();
        let h = f.model.by_bundle("dev.fixture.Owned").unwrap()[0];
        assert_eq!(f.model.by_pid(701234).unwrap(), Some(h));
        assert!(f.model.by_pid(1).unwrap().is_none());
        assert!(f.model.by_bundle("com.apple.Safari").unwrap().is_empty());
        assert!(f
            .model
            .activate(Handle {
                id: 999,
                generation: 1
            })
            .is_err());
        f.model.activate(h).unwrap();
        assert_eq!(f.model.frontmost().unwrap(), Some(h));
        let url = "obsidian://open?vault=OwnedFixture&file=Saved%20note";
        f.model
            .set_open_result(url.into(), Outcome::Failure)
            .unwrap();
        assert_eq!(f.model.open(url).unwrap(), Outcome::Failure);
        assert!(f.model.open("https://personal.invalid/").is_err());
        assert!(f
            .model
            .open("https://fixture.invalid/unconfigured")
            .is_err());
        let effects = f.model.effects().unwrap();
        assert_eq!(effects, vec![Effect::Activate(h), Effect::Open(url.into())]);
        let rect = f.model.screens().unwrap()[0].frame;
        let w = f.model.new_window(rect).unwrap();
        f.model.window_action(w, "key").unwrap();
        assert_eq!(f.model.window(w).unwrap(), (rect, true, true));
        f.model.window_action(w, "out").unwrap();
        assert_eq!(f.model.window(w).unwrap(), (rect, false, false));
        assert!(f.model.window_action(w, "unowned").is_err());
        let observer = f
            .model
            .subscribe("NSWorkspaceDidActivateApplicationNotification")
            .unwrap();
        assert_eq!(
            f.model
                .notification_recipients("NSWorkspaceDidActivateApplicationNotification")
                .unwrap(),
            vec![observer]
        );
        f.model.unsubscribe(observer).unwrap();
        assert!(f
            .model
            .notification_recipients("NSWorkspaceDidActivateApplicationNotification")
            .unwrap()
            .is_empty());
        f.life.0.store(false, Ordering::Release);
        assert!(f.model.screens().is_err());
        assert!(f.model.activate(h).is_err());
        assert!(f.model.open(url).is_err());
    }
    #[test]
    fn guardian_panics_permanently_retire_each_workspace_before_later_effects() {
        for mode in [1, 2] {
            let f = Fixture::new();
            assert!(f.model.screens().is_ok());
            f.life.1.store(mode, Ordering::Release);
            assert!(f.model.screens().is_err());
            f.life.1.store(0, Ordering::Release);
            assert!(f.model.screens().is_err());
            assert!(f.model.open("https://fixture.invalid/no-retry").is_err());
        }
    }
    #[test]
    fn callback_mutations_refuse_foreign_owner_before_retaining_or_dropping_blocks() {
        use objc2::runtime::AnyObject;
        use objc2_foundation::{NSError, NSNotification, NSString};
        let f = Fixture::new();
        let hub = Arc::new(notifications::Hub::new(f.model.clone()).unwrap());
        let replies = Arc::new(callbacks::Replies::new(f.model.clone()).unwrap());
        let called = Arc::new(AtomicU64::new(0));
        let count = called.clone();
        let block = block2::RcBlock::new(move |_: *mut AnyObject| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let name = "NSWorkspaceDidActivateApplicationNotification";
        let id = unsafe { hub.subscribe(name, (&*block as *const block2::Block<_>).cast_mut()) }
            .unwrap();
        drop(block);
        let foreign_hub = hub.clone();
        let foreign_replies = replies.clone();
        std::thread::spawn(move || {
            let block = block2::RcBlock::new(|_: *mut AnyObject| {});
            assert!(unsafe {
                foreign_hub.subscribe(name, (&*block as *const block2::Block<_>).cast_mut())
            }
            .is_err());
            assert!(foreign_hub.unsubscribe(id).is_err());
            let reply = block2::RcBlock::new(|_: *mut AnyObject, _: *mut NSError| {});
            assert!(unsafe {
                foreign_replies.enqueue(
                    "https://fixture.invalid/foreign",
                    (&*reply as *const block2::Block<_>).cast_mut(),
                )
            }
            .is_err());
            assert!(foreign_replies.drain().is_err());
        })
        .join()
        .unwrap();
        assert_eq!(f.model.notification_recipients(name).unwrap(), vec![id]);
        assert!(f.model.effects().unwrap().is_empty());
        assert_eq!(replies.pending().unwrap(), 0);
        let notice =
            unsafe { NSNotification::notificationWithName_object(&NSString::from_str(name), None) };
        let pointer = objc2::rc::Retained::as_ptr(&notice)
            .cast_mut()
            .cast::<AnyObject>();
        assert_eq!(unsafe { hub.emit(name, pointer) }.unwrap(), 1);
        assert_eq!(called.load(Ordering::SeqCst), 1);
        hub.unsubscribe(id).unwrap();
        assert!(f.model.notification_recipients(name).unwrap().is_empty());
        assert_eq!(unsafe { hub.emit(name, pointer) }.unwrap(), 0);
    }
    #[test]
    fn owned_notification_blocks_hold_data_and_unregister_before_late_delivery() {
        use objc2::runtime::AnyObject;
        use objc2_foundation::{NSNotification, NSString};
        let f = Fixture::new();
        let hub = Arc::new(notifications::Hub::new(f.model.clone()).unwrap());
        let called = Arc::new(AtomicU64::new(0));
        let removed = Arc::new(AtomicU64::new(0));
        let weak = Arc::downgrade(&hub);
        let target = removed.clone();
        let first_count = called.clone();
        let first = block2::RcBlock::new(move |notice: *mut AnyObject| {
            let notice = unsafe { &*notice.cast::<NSNotification>() };
            assert_eq!(
                notice.name().to_string(),
                "NSWorkspaceDidActivateApplicationNotification"
            );
            first_count.fetch_add(1, Ordering::SeqCst);
            weak.upgrade()
                .unwrap()
                .unsubscribe(target.load(Ordering::SeqCst))
                .unwrap();
        });
        let count = called.clone();
        let second = block2::RcBlock::new(move |_: *mut AnyObject| {
            count.fetch_add(100, Ordering::SeqCst);
        });
        let name = "NSWorkspaceDidActivateApplicationNotification";
        unsafe {
            hub.subscribe(name, (&*first as *const block2::Block<_>).cast_mut())
                .unwrap();
            let id = hub
                .subscribe(name, (&*second as *const block2::Block<_>).cast_mut())
                .unwrap();
            removed.store(id, Ordering::SeqCst);
        }
        drop(first);
        drop(second);
        let notice =
            unsafe { NSNotification::notificationWithName_object(&NSString::from_str(name), None) };
        let pointer = objc2::rc::Retained::as_ptr(&notice)
            .cast_mut()
            .cast::<AnyObject>();
        assert_eq!(unsafe { hub.emit(name, pointer) }.unwrap(), 1);
        assert_eq!(called.load(Ordering::SeqCst), 1);
        f.life.0.store(false, Ordering::Release);
        assert!(unsafe { hub.emit(name, pointer) }.is_err());
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn configured_failed_and_unknown_completions_never_report_false_success_or_retry() {
        use objc2::runtime::AnyObject;
        use objc2_foundation::NSError;
        let f = Fixture::new();
        let replies = callbacks::Replies::new(f.model.clone()).unwrap();
        let called = Arc::new(AtomicU64::new(0));
        let n = called.clone();
        let fail = block2::RcBlock::new(move |app: *mut AnyObject, error: *mut NSError| {
            assert!(app.is_null());
            assert!(!error.is_null());
            assert_eq!(
                unsafe { &*error }.domain().to_string(),
                "RuycastOwnedWorkspace"
            );
            n.fetch_add(1, Ordering::SeqCst);
        });
        f.model
            .set_open_result("https://fixture.invalid/fail".into(), Outcome::Failure)
            .unwrap();
        f.model
            .set_open_result("https://fixture.invalid/unknown".into(), Outcome::Unknown)
            .unwrap();
        unsafe {
            replies
                .enqueue(
                    "https://fixture.invalid/fail",
                    (&*fail as *const block2::Block<_>).cast_mut(),
                )
                .unwrap();
            replies
                .enqueue(
                    "https://fixture.invalid/unknown",
                    (&*fail as *const block2::Block<_>).cast_mut(),
                )
                .unwrap();
        }
        drop(fail);
        assert_eq!(replies.drain().unwrap(), 1);
        assert_eq!(replies.drain().unwrap(), 0);
        assert_eq!(replies.pending().unwrap(), 1);
        assert_eq!(called.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.model.effects().unwrap(),
            vec![
                Effect::Open("https://fixture.invalid/fail".into()),
                Effect::Open("https://fixture.invalid/unknown".into())
            ]
        );
    }
    #[test]
    fn refused_completion_publication_leaves_no_open_effect_or_pending_id() {
        let f = Fixture::new();
        let url = "https://fixture.invalid/open";
        f.model.set_open_result(url.into(), Outcome::Success).unwrap();
        let before = f.model.with(|state| Ok(state.next)).unwrap();
        let refused = f.model.begin_open_with(url, |_| Err("queue refused".into()));
        assert_eq!(refused, Err("queue refused".into()));
        assert!(f.model.effects().unwrap().is_empty());
        f.model.with(|state| {
            assert_eq!(state.next, before);
            assert!(state.pending.is_empty());
            Ok(())
        }).unwrap();
        let accepted = f.model.begin_open(url).unwrap();
        assert_eq!(accepted.id, before);
        assert!(!f.model.complete_if(&accepted, || Ok(false)).unwrap());
        assert_eq!(f.model.complete_if(&accepted, || Err("capture changed".into())), Err("capture changed".into()));
        assert_eq!(f.model.complete(&accepted).unwrap(), accepted);
        assert!(f.model.complete(&accepted).is_err());
    }
    #[test]
    fn open_url_with_owned_application_requires_configured_result() {
        let f = Fixture::new();
        let path = f.root.join("apps/Owned.app");
        let url = "https://fixture.invalid/document";
        let application = f.model.by_pid(701234).unwrap().unwrap();
        let refused = f.model.begin_owned_application_open(&path, url, true, |_, _| {
            panic!("unconfigured URL was published");
        });
        assert!(refused.is_err());
        assert!(f.model.effects().unwrap().is_empty());
        f.model.set_open_result(url.into(), Outcome::Failure).unwrap();
        let completion = f.model.begin_owned_application_open(&path, url, true, |handle, reply| {
            assert_eq!(handle, application);
            assert_eq!(reply.outcome, Outcome::Failure);
            Ok(())
        }).unwrap();
        assert_eq!(f.model.effects().unwrap(), vec![Effect::Open(url.into())]);
        assert_eq!(f.model.complete(&completion).unwrap(), completion);

        let launch_url = format!("file://{}", path.display());
        let launch = f.model.begin_owned_application_open(&path, &launch_url, false, |handle, _| {
            assert_eq!(handle, application);
            Ok(())
        }).unwrap();
        assert_eq!(launch.outcome, Outcome::Success);
    }
    #[test]
    fn retained_real_blocks_deliver_once_and_retirement_prevents_late_callback() {
        use objc2::runtime::AnyObject;
        use objc2_foundation::NSError;
        let f = Fixture::new();
        let replies = callbacks::Replies::new(f.model.clone()).unwrap();
        let called = Arc::new(AtomicU64::new(0));
        let c = called.clone();
        let block = block2::RcBlock::new(move |app: *mut AnyObject, error: *mut NSError| {
            assert!(!app.is_null());
            assert!(error.is_null());
            c.fetch_add(1, Ordering::SeqCst);
        });
        f.model
            .set_open_result("https://fixture.invalid/success".into(), Outcome::Success)
            .unwrap();
        unsafe {
            replies
                .enqueue(
                    "https://fixture.invalid/success",
                    (&*block as *const block2::Block<_>).cast_mut(),
                )
                .unwrap();
        }
        drop(block);
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert_eq!(replies.drain().unwrap(), 1);
        assert_eq!(replies.drain().unwrap(), 0);
        assert_eq!(called.load(Ordering::SeqCst), 1);
        let c = called.clone();
        let late = block2::RcBlock::new(move |_: *mut AnyObject, _: *mut NSError| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        unsafe {
            replies
                .enqueue(
                    "https://fixture.invalid/success",
                    (&*late as *const block2::Block<_>).cast_mut(),
                )
                .unwrap();
        }
        drop(late);
        f.life.0.store(false, Ordering::Release);
        assert!(replies.drain().is_err());
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }
}
