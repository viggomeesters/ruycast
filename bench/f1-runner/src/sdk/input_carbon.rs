//! Owned Carbon leaves for the original Hotkeys startup/callback route.
//! No OS registration, ready export, constructor or scope admission. The common
//! integrator must independently bind every source import and callback image.
use super::owned::AliveLease;
use super::{core_preentry::{NativeInputAdmission, NativeInputCallbackWitness, NativeInputContextKind, NativeInputInstallation, NativeInputOwnerFamily, NativeInputOwnerTicket}, owned::ModuleContext};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, ThreadId},
};
pub type Ptr = *mut c_void;
pub type Handler = unsafe extern "C" fn(Ptr, Ptr, Ptr) -> i32;
pub const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
pub const HOTKEY: u32 = u32::from_be_bytes(*b"hkid");
pub const DIRECT: u32 = u32::from_be_bytes(*b"----");
const PARAM: i32 = -50;
const UNHANDLED: i32 = -9874;
const CONFLICT: i32 = -9878;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Id {
    pub signature: u32,
    pub id: u32,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EventType {
    pub class: u32,
    pub kind: u32,
}
#[derive(Clone, Copy)]
pub struct Event {
    pub class: u32,
    pub kind: u32,
    pub id: Id,
    pub status: i32,
    pub parameter_type: u32,
    pub size: usize,
}
struct Installed {
    token: Box<u8>,
    callback: Handler,
    user: usize,
    live: bool,
    owner: Option<Arc<NativeInputOwnerTicket>>,
}
struct Key {
    token: Box<u8>,
    code: u32,
    modifiers: u32,
    id: Id,
    live: bool,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    reference_handler: Option<usize>,
}
#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn verify_reference_startup_keys(keys: &[Key]) -> Result<(), String> {
    // The guarded producer writes ctrl+alt+space and ctrl+alt+v into the
    // private startup config. Hotkeys.update sorts the canonical names and
    // assigns IDs starting at 1, so Space is the launcher and V the clipboard.
    const CTRL_ALT: u32 = (1 << 12) | (1 << 11);
    let mut live = keys.iter().filter(|key| key.live);
    let (Some(first), Some(second), None) = (live.next(), live.next(), live.next()) else {
        return Err("Reference Carbon requires exactly two live startup hotkeys".into());
    };
    let mut bindings = [(first.code, first.id.id), (second.code, second.id.id)];
    bindings.sort_unstable();
    if bindings != [(9, 2), (49, 1)]
        || [first, second].into_iter().any(|key| {
            key.modifiers != CTRL_ALT || key.id.signature != u32::from_be_bytes(*b"RUCA")
        })
    {
        return Err("Reference Carbon startup hotkeys differ from the owned fixture".into());
    }
    Ok(())
}
struct State {
    handlers: Vec<Installed>,
    keys: Vec<Key>,
    events: BTreeMap<usize, Box<Event>>,
    pending_owner: Option<Arc<NativeInputOwnerTicket>>,
}
struct NativeBinding {
    context: ModuleContext,
    installation: NativeInputInstallation,
    callback: NativeInputCallbackWitness,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
struct ReferenceBinding {
    admission:Arc<super::core_preentry::ReferenceAdmission>,
    context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
    child:std::sync::OnceLock<Arc<crate::workspace::factory::ReferenceInputChildCustody>>,
    cleanup_only: AtomicBool,
    module:std::sync::OnceLock<(super::owned::OwnedHandleLease,Vec<super::ImportReplacement>)>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ReferenceBinding {
    fn code(&self)->Result<usize,String> {
        use sha2::Digest;
        self.context.verify_admission_binding(&self.admission,&self.model)?;
        self.context.verify_input_parent_source_members()?;
        let base=self.context.observe_app_delegate_offset_cells()?.main_header();
        // Only fixed code bytes and image membership are observed here.
        // Dynamic owner/model and lease checks stay outside this scope.
        unsafe {
            self.context.with_callback_source_observation(|| {
                for (offset, bytes, sha) in [
                    (0xf33bcusize, 496usize, "df314903b7fbfb11d27080163ff9a4f867feffdfc6c1e9f5a1b396d92fadf836"),
                    (0xf3658, 468, "f64b366875ea37d7313a0e41f8a7db6247c4668b88b4c3c09567cd977b5dd259"),
                    (0xf3880, 12, "4b0c11020dbb66805046d8ba887b4c51d8af2fb97542c5742669aa55cd311001"),
                    (0xf45e0, 36, "cea9ca514ebb99e7608854a2c71a1038be12b26240e46a9e69cb7a75b6f5ca41"),
                ] {
                    self.context.verify_main_code(
                        base.checked_add(offset).ok_or("Reference Carbon code overflow")?,
                        bytes,
                        sha,
                    )?;
                }
                // The real C entry captures x30 before the Rust prologue.
                for (entry, checked) in [
                    (add as *const () as usize, add_checked as *const () as usize),
                    (register as *const () as usize, register_checked as *const () as usize),
                ] {
                    self.context.verify_app_delegate_sdk_members(entry, checked)?;
                    let pc = entry.checked_add(12).ok_or("Carbon entry branch overflow")?;
                    let delta = checked as i128 - pc as i128;
                    if delta % 4 != 0 || !(-(1i128 << 27)..(1i128 << 27)).contains(&delta) {
                        return Err("Carbon helper outside direct branch range".into());
                    }
                    let words = [
                        0xa9bf7bfdu32, 0x910003fd, 0xaa1e03e6,
                        0x94000000 | (((delta / 4) as i64 as u32) & 0x03ffffff),
                        0xa8c17bfd, 0xd65f03c0,
                    ];
                    let bytes = words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
                    self.context.verify_sdk_code(
                        entry, bytes.len(), &format!("{:x}", sha2::Sha256::digest(&bytes)),
                    )?;
                }
                Ok(())
            })
        }?;
        if let Some((lease,imports))=&self.module.get() {
            lease.before_effect()?;
            for row in imports {if row.selected_address()!=row.replacement{return Err("Reference Carbon import changed".into());}}
        }
        Ok(base)
    }
    fn registration(&self,callback:Handler,return_pc:usize)->Result<(),String> {
        let base=self.code()?;
        if base.checked_add(0xf3544)!=Some(return_pc)||base.checked_add(0xf3880)!=Some(callback as usize) {
            return Err("Reference Carbon registration caller/callback differs".into());
        }
        self.context.verify_main_code(base.checked_add(0x2fb42c).ok_or("Reference Carbon stub offset overflow")?, 12,
            "952c900ea090343e6c9231e484dd2b432f120a541e48bfcacbee95146258475b")?;
        let (_, imports) = self.module.get().ok_or("Reference Carbon imports absent")?;
        let import = imports.iter().find(|row| row.symbol == c"InstallEventHandler")
            .ok_or("Reference Carbon handler import absent")?;
        if base.checked_add(0x3c4b48) != Some(import.cell_address())
            || import.selected_address() != import.replacement
        {
            return Err("Reference Carbon handler import differs".into());
        }
        Ok(())
    }
    fn owner(&self,user:usize)->Result<Arc<crate::workspace::factory::ReferenceInputChildCustody>,String> {
        self.code()?;
        let child=self.child.get().ok_or("Reference Carbon registration is pending owner publication")?.clone();
        child.verify_reference_binding(&self.admission,&self.model)?;
        if child.kind()!=crate::workspace::factory::ReferenceInputChildKind::Hotkeys||child.pointer_on_creator()?!=user {
            return Err("Reference Carbon userdata differs from genuine held Hotkeys".into());
        }
        child.verify_held_epoch()?;Ok(child)
    }
}
pub struct Context {
    life: Arc<dyn AliveLease>,
    thread: ThreadId,
    callbacks: BTreeSet<usize>,
    target: Box<u8>,
    retired: AtomicBool,
    state: Mutex<State>,
    native: Option<NativeBinding>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference:Option<ReferenceBinding>,
}
fn ptr(token: &u8) -> Ptr {
    std::ptr::from_ref(token).cast_mut().cast()
}
impl Context {
    /// Only exact fixture callback addresses are admitted by this contribution.
    /// The set is not source/image provenance and cannot admit a product graph.
    pub fn new(life: Arc<dyn AliveLease>, callbacks: BTreeSet<usize>) -> Result<Arc<Self>, String> {
        life.before_effect()?;
        if !life.is_alive()
            || callbacks.is_empty()
            || callbacks.len() > 64
            || callbacks.contains(&0)
        {
            return Err("invalid own Carbon lifecycle/callback roster".into());
        }
        Ok(Arc::new(Self {
            life,
            thread: thread::current().id(),
            callbacks,
            target: Box::new(0),
            retired: AtomicBool::new(false),
            state: Mutex::new(State {
                handlers: vec![],
                keys: vec![],
                events: BTreeMap::new(),
                pending_owner: None,
            }),
            native: None,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            reference:None,
        }))
    }
    /// Source-qualified setup is separate from the fixture address roster.
    /// The private owner publisher must still supply a live ticket before add.
    pub fn for_native_input(context: ModuleContext, admission: &Arc<NativeInputAdmission>) -> Result<Arc<Self>, String> {
        let callback = admission.carbon_pressed();
        callback.verify(&context)?;
        let handler = callback.carbon_handler()?;
        let installation = admission.reserve_installation(NativeInputContextKind::Carbon, &context)?;
        let mut owner = Self::new(context.life.clone(), BTreeSet::from([handler as usize]))?;
        Arc::get_mut(&mut owner).ok_or("Carbon candidate was shared")?.native = Some(NativeBinding { context, installation, callback });
        owner.check_candidate()?;
        Ok(owner)
    }
    fn check_candidate(&self) -> Result<(), String> {
        self.check_life()?;
        if let Some(native) = &self.native { native.installation.verify_candidate(&native.context)?; }
        self.check_pure()
    }
    fn check(&self) -> Result<(), String> {
        self.check_life()?;
        if let Some(native) = &self.native { native.installation.verify(&native.context)?; }
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&self.reference {reference.code()?;}
        self.check_pure()
    }
    fn check_pure(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("retired/wrong-thread own Carbon context".into());
        }
        Ok(())
    }
    fn check_life(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("retired/wrong-thread own Carbon context".into());
        }
        let alive = catch_unwind(AssertUnwindSafe(|| {
            self.life.before_effect().is_ok() && self.life.is_alive()
        }))
        .unwrap_or(false);
        if !alive {
            self.retired.store(true, Ordering::Release);
            return Err("own Carbon guardian retired".into());
        }
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("own Carbon authority changed during lifecycle check".into());
        }
        Ok(())
    }
    /// Synchronous own event. Callback executes outside state/owner locks; no
    /// timer, OS event target, runloop, scheduler or original callback fallback.
    pub fn emit(&self, event: Event) -> Result<i32, String> {
        self.emit_registered(event, None)
    }
    fn emit_registered(&self, event: Event, key_registration: Option<usize>) -> Result<i32, String> {
        self.check()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.reference.as_ref().is_some_and(|source| source.cleanup_only.load(Ordering::Acquire)) {
            return Err("Reference Carbon delivery closed for cleanup".into());
        }
        let verify_key = || -> Result<(), String> {
            if let Some(registration) = key_registration {
                let state = self.state.lock().map_err(|_| "Carbon state poisoned")?;
                if !state.keys.iter().any(|key| key.live
                    && ptr(&key.token) as usize == registration
                    && key.id.id == event.id.id && key.id.signature == event.id.signature)
                {
                    return Err("Carbon key registration retired before delivery".into());
                }
            }
            Ok(())
        };
        let (callback, user, owner, pointer, registration) = {
            let mut state = self.state.lock().map_err(|_| "Carbon state poisoned")?;
            let Some(handler) = state.handlers.iter().find(|h| h.live) else {
                return Ok(UNHANDLED);
            };
            let (callback, user, owner) = (handler.callback, handler.user, handler.owner.clone());
            let registration=ptr(&handler.token) as usize;
            if state.events.len() >= 32 {
                return Err("own Carbon callback depth exceeded".into());
            }
            let owned = Box::new(event);
            let pointer = std::ptr::from_ref(&*owned) as usize;
            state.events.insert(pointer, owned);
            (callback, user, owner, pointer, registration)
        };
        let result = (|| {
            self.check()?;
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            if let Some(reference)=&self.reference {
                let key_registration = key_registration.ok_or("Reference Carbon delivery requires a registered hotkey")?;
                let child=reference.owner(user)?;
                self.check()?;
                reference.owner(user)?;
                self.check_pure()?;
                // Last guard is pure and refers to the SAME never-reused
                // token/registration, not merely another live callback row.
                let live=|before_delivery: bool|->Result<(),String>{
                    if child.carbon_handler_on_creator()? != registration {
                        return Err("Reference Swift Hotkeys handler no longer matches the delivery".into());
                    }
                    self.check_pure()?;
                    child.verify_held_epoch()?;
                    let state=self.state.lock().map_err(|_|"Carbon state poisoned")?;
                    if !state.handlers.iter().any(|h|h.live&&ptr(&h.token) as usize==registration&&h.user==user&&h.callback as usize==callback as usize) {
                        return Err("Reference Carbon registration retired before delivery".into());
                    }
                    if before_delivery && (reference.cleanup_only.load(Ordering::Acquire)
                        || !state.keys.iter().any(|key| key.live
                            && ptr(&key.token) as usize == key_registration
                            && key.id.id == event.id.id && key.id.signature == event.id.signature
                            && key.reference_handler == Some(registration)))
                    {
                        return Err("Reference Carbon key or handler changed before delivery".into());
                    }
                    Ok(())
                };
                live(true)?;
                super::reference_runtime::verify_published_input_child(&reference.admission, &child)?;
                let value=unsafe{callback(std::ptr::null_mut(),pointer as Ptr,user as Ptr)};
                live(false)?;self.check()?;live(false)?;
                super::reference_runtime::verify_published_input_child(&reference.admission, &child)?;
                return Ok(value);
            }
            let _=registration;
            if let Some(native) = &self.native {
                let owner = owner.ok_or("Carbon Native callback has no owner ticket")?;
                if !owner.matches_callback(&native.callback) { return Err("Carbon owner/code issuer differs".into()); }
                let entry = owner.enter()?;
                if entry.userdata()? != user { return Err("Carbon userdata differs from live owner".into()); }
                let guard = native.callback.before_callback(&native.context)?;
                native.installation.pure_after()?;
                self.check_pure()?;
                // Reentry during the dynamic gate may revoke this registration.
                if entry.userdata()? != user { return Err("Carbon owner retired before callback".into()); }
                verify_key()?;
                let value = unsafe { callback(std::ptr::null_mut(), pointer as Ptr, user as Ptr) };
                // Keep the Native owner entry through the final dynamic check;
                // only pure cleanup may follow the source/lifetime postchecks.
                self.check()?;
                let source = guard.pure_after_callback();
                let lifetime = entry.pure_after_callback();
                native.installation.pure_after()?;
                self.check_pure()?;
                source?;
                lifetime?;
                Ok(value)
            } else {
                verify_key()?;
                Ok(unsafe { callback(std::ptr::null_mut(), pointer as Ptr, user as Ptr) })
            }
        })();
        self.state
            .lock()
            .map_err(|_| "Carbon state poisoned")?
            .events
            .remove(&pointer);
        // Reference delivery already paired its final dynamic check with
        // exact handler validation. Only event removal may follow that pair.
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if self.reference.is_some() {
            return result;
        }
        if self.native.is_none() { self.check()?; }
        result
    }
    pub fn emit_binding(&self, code: u32, modifiers: u32) -> Result<i32, String> {
        self.check()?;
        let id = self
            .state
            .lock()
            .map_err(|_| "Carbon state poisoned")?
            .keys
            .iter()
            .find(|k| k.live && k.code == code && k.modifiers == modifiers)
            .map(|k| (k.id, ptr(&k.token) as usize));
        match id {
            Some((id, registration)) => self.emit_registered(Event {
                class: KEYBOARD,
                kind: 5,
                id,
                status: 0,
                parameter_type: HOTKEY,
                size: 8,
            }, Some(registration)),
            None => Ok(UNHANDLED),
        }
    }
}
static CURRENT: Mutex<Option<Arc<Context>>> = Mutex::new(None);
static PUBLISHED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static PUBLICATION_TEST_LOCK: Mutex<()> = Mutex::new(());
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
static REFERENCE_ATTEMPT:AtomicBool=AtomicBool::new(false);
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_roster()->BTreeSet<super::SourceCallsite> {
    BTreeSet::from([super::SourceCallsite{
        file:"reference-0.8.26/Sources/RuycastApp/Hotkeys.swift",
        source_sha256:"c78041f9d41ba4c97cd2958b3f575b24079d86ce401c3065705835e1d17b8ac0".into(),
        operation:"Carbon constructor registration, owned events and retirement",
    }])
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) unsafe fn initialize_reference(
    admission:Arc<super::core_preentry::ReferenceAdmission>,
    context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
)->Result<(Arc<Context>,super::ModuleAttestation),String> {
    // The original top-level AppDelegate has one construction attempt per
    // process. Failed setup is terminal, not permission to recycle its tokens.
    if REFERENCE_ATTEMPT.swap(true,Ordering::AcqRel){return Err("Reference Carbon construction attempt already claimed".into());}
    let module=super::bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;
    admission.verify_input_context(&module)?;
    let reference=ReferenceBinding{admission,context,model,child:std::sync::OnceLock::new(),cleanup_only:AtomicBool::new(false),module:std::sync::OnceLock::new()};
    let base=reference.code()?;
    let candidate=Arc::new(Context{
        life:module.life.clone(),thread:thread::current().id(),callbacks:BTreeSet::from([base+0xf3880]),
        target:Box::new(0),retired:AtomicBool::new(false),
        state:Mutex::new(State{handlers:vec![],keys:vec![],events:BTreeMap::new(),pending_owner:None}),
        native:None,reference:Some(reference),
    });
    install(candidate.clone())?;
    struct Publication {context:Arc<Context>,complete:bool}
    impl Drop for Publication {fn drop(&mut self){if !self.complete{rollback_installation(&self.context);}}}
    let mut publication=Publication{context:candidate.clone(),complete:false};
    struct InstallGuard<'a>(&'a Context);
    impl AliveLease for InstallGuard<'_> {
        fn is_alive(&self)->bool{self.before_effect().is_ok()}
        fn before_effect(&self)->Result<(),String>{self.0.check()}
    }
    let lease=module.lease("reference-carbon")?;
    // Frozen Reference Main uses these six functions. The general/native
    // eight-function universe stays intact; the other two must be absent.
    let specs=import_specs();
    let role=[c"GetApplicationEventTarget",c"InstallEventHandler",c"RemoveEventHandler",
        c"RegisterEventHotKey",c"UnregisterEventHotKey",c"GetEventParameter"];
    let selected=specs.iter().filter(|spec|role.contains(&spec.symbol)).copied().collect::<Vec<_>>();
    if selected.len()!=role.len(){return Err("Reference Carbon fixed import role differs".into());}
    let imports=super::import_patch::install_exact_presence(&selected,&specs,&InstallGuard(&candidate))?;
    candidate.reference.as_ref().ok_or("Reference Carbon source absent")?.module.set((lease,imports))
        .map_err(|_|"Reference Carbon module already installed")?;
    let result=reference_attestation()?;
    publication.complete=true;Ok((candidate,result))
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_attestation()->Result<super::ModuleAttestation,String> {
    let context=CURRENT.lock().map_err(|_|"Carbon owner poisoned")?.clone().ok_or("Reference Carbon absent")?;
    let source=context.reference.as_ref().ok_or("Carbon context is not Reference source")?;
    let (lease,imports)=source.module.get().ok_or("Reference Carbon imports absent")?;
    let result=super::ModuleAttestation{module:"reference-carbon",scopes:BTreeSet::new(),callsites:reference_roster(),
        imports:imports.clone(),objc:vec![],handles:vec![lease.clone()],workspace_factories:None};
    verify_reference_attestation(&result)?;Ok(result)
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_attestation(module:&super::ModuleAttestation)->Result<(),String> {
    if module.module!="reference-carbon"||!module.scopes.is_empty()||module.callsites!=reference_roster()
        ||!module.objc.is_empty()||module.workspace_factories.is_some()||module.handles.len()!=1 {
        return Err("Reference Carbon module envelope differs".into());
    }
    let context=CURRENT.lock().map_err(|_|"Carbon owner poisoned")?.clone().ok_or("Reference Carbon absent")?;
    // Registry inspection on a worker carries no callback or event authority.
    if context.thread == thread::current().id() {
        context.check()?;
    }
    let source=context.reference.as_ref().ok_or("Reference Carbon source absent")?;
    let (lease,imports)=source.module.get().ok_or("Reference Carbon imports absent")?;
    if module.handles[0].namespace!=lease.namespace||module.handles[0].root.path()!=lease.root.path()
        ||!Arc::ptr_eq(&module.handles[0].life,&lease.life)||module.imports.len()!=imports.len() {
        return Err("Reference Carbon module issuer differs".into());
    }
    lease.before_effect()?;
    let mut symbols=BTreeSet::new();
    for row in &module.imports {
        let expected=imports.iter().find(|expected|expected.symbol==row.symbol).ok_or("Foreign Reference Carbon import")?;
        if !symbols.insert(row.symbol)||row.original!=expected.original||row.replacement!=expected.replacement
            ||row.cell_address()!=expected.cell_address()||row.selected_address()!=row.replacement {
            return Err("Reference Carbon import/source differs".into());
        }
    }
    let current = CURRENT.lock().map_err(|_|"Carbon owner poisoned")?;
    if !PUBLISHED.load(Ordering::Acquire)
        || context.retired.load(Ordering::Acquire)
        || !current.as_ref().is_some_and(|published| Arc::ptr_eq(published, &context))
    {
        return Err("Reference Carbon publication changed during attestation".into());
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn publish_reference_owner(
    admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>,
    child:Arc<crate::workspace::factory::ReferenceInputChildCustody>,
)->Result<(),String> {
    let context=CURRENT.lock().map_err(|_|"Carbon owner poisoned")?.clone().ok_or("Reference Carbon absent")?;
    let result=(|| {
        context.check()?;
        let source=context.reference.as_ref().ok_or("Carbon has no Reference binding")?;
        if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model)||source.child.get().is_some()
            ||child.kind()!=crate::workspace::factory::ReferenceInputChildKind::Hotkeys {
            return Err("Reference Carbon owner publication issuer/replay differs".into());
        }
        child.verify_reference_binding(admission,model)?;
        let user=child.pointer_on_creator()?;
        context.check()?;
        let held_handler = child.carbon_handler_on_creator()?;
        // No dynamic guard crosses the registration lock or follows this
        // publication. The held child is independent of the pending record.
        let state=context.state.lock().map_err(|_|"Carbon state poisoned")?;
        let live=state.handlers.iter().filter(|h|h.live).collect::<Vec<_>>();
        if live.len()!=1||live[0].user!=user||ptr(&live[0].token) as usize != held_handler
            ||!context.callbacks.contains(&(live[0].callback as usize)) {
            return Err("Reference Carbon pending handler or userdata differs from actual source child".into());
        }
        if state.keys.iter().any(|key| key.live && key.reference_handler != Some(held_handler)) {
            return Err("Reference Carbon startup key belongs to another handler".into());
        }
        verify_reference_startup_keys(&state.keys)?;
        context.check_pure()?;
        source.child.set(child).map_err(|_|"Reference Carbon child already published")?;
        Ok(())
    })();
    if result.is_err(){context.retired.store(true,Ordering::Release);}
    result
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn retire_reference(admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->Result<(),String> {
    let context=CURRENT.lock().map_err(|_|"Carbon owner poisoned")?.clone();
    match context {
        Some(context) => {
            let source=context.reference.as_ref().ok_or("Carbon teardown found another source family")?;
            if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model){return Err("Carbon teardown found another Reference owner".into());}
            retire(&context)
        }
        None=>Err("Reference Carbon teardown has no installed owner to drain".into()),
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) struct ReferenceCarbonCleanup {
    context: Arc<Context>,
    finished: bool,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl ReferenceCarbonCleanup {
    pub(crate) fn begin(
        admission: &Arc<super::core_preentry::ReferenceAdmission>,
        model: &Arc<crate::workspace::Workspace>,
    ) -> Result<Self, String> {
        super::reference_runtime::verify_terminal_input_cleanup(admission, model)?;
        let context = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?.clone()
            .ok_or("Reference Carbon cleanup has no installed owner")?;
        let source = context.reference.as_ref().ok_or("Carbon cleanup found another source family")?;
        if !Arc::ptr_eq(&source.admission, admission) || !Arc::ptr_eq(&source.model, model) {
            return Err("Carbon cleanup found another Reference owner".into());
        }
        context.check()?;
        {
            let current = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?;
            context.check_pure()?;
            let state = context.state.lock().map_err(|_| "Carbon state poisoned")?;
            if !PUBLISHED.load(Ordering::Acquire)
                || !current.as_ref().is_some_and(|installed| Arc::ptr_eq(installed, &context)) || !state.events.is_empty()
                || state.pending_owner.is_some() || source.child.get().is_none()
                || state.handlers.iter().filter(|handler| handler.live).count() != 1
                || source.cleanup_only.swap(true, Ordering::AcqRel)
            {
                return Err("Reference Carbon cleanup has no exclusive published handler".into());
            }
        }
        Ok(Self { context, finished: false })
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.context.check()?;
        {
            let current = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?;
            self.context.check_pure()?;
            if !PUBLISHED.load(Ordering::Acquire)
                || !current.as_ref().is_some_and(|context| Arc::ptr_eq(context, &self.context))
                || !self.context.reference.as_ref().is_some_and(|source| source.cleanup_only.load(Ordering::Acquire))
            {
                return Err("Reference Carbon cleanup owner changed".into());
            }
            let state = self.context.state.lock().map_err(|_| "Carbon state poisoned")?;
            if !state.events.is_empty() || state.handlers.iter().any(|handler| handler.live)
                || state.keys.iter().any(|key| key.live)
            {
                return Err("Original Hotkeys deinit left a live Carbon registration".into());
            }
        }
        retire(&self.context)?;
        self.finished = true;
        Ok(())
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl Drop for ReferenceCarbonCleanup {
    fn drop(&mut self) {
        if !self.finished {
            let _ = retire(&self.context);
        }
    }
}

pub fn install(context: Arc<Context>) -> Result<(), String> {
    context.check_candidate()?;
    {
        let mut slot = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?;
        if slot.is_some() { return Err("Carbon owner already installed".into()); }
        *slot = Some(context.clone());
    }
    if let Some(native) = &context.native {
        if let Err(error) = native.installation.commit_installed() {
            rollback_installation(&context);
            return Err(error);
        }
    }
    PUBLISHED.store(true, Ordering::Release);
    if let Err(error) = context.check() {
        rollback_installation(&context);
        return Err(error);
    }
    Ok(())
}
fn rollback_installation(expected: &Arc<Context>) {
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    if let Ok(mut slot) = CURRENT.lock() {
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
            PUBLISHED.store(false, Ordering::Release);
            *slot = None;
        }
    }
}
pub fn retire(expected: &Arc<Context>) -> Result<(), String> {
    let mut slot = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?;
    if !slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
        return Err("stale Carbon owner".into());
    }
    PUBLISHED.store(false, Ordering::Release);
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    *slot = None;
    drop(slot);
    let (owners, mut busy, unresolved) = {
        let mut state = expected.state.lock().map_err(|_| "Carbon state poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let unresolved = expected.reference.is_some()
            && (state.keys.iter().any(|key| key.live) || state.handlers.iter().any(|handler| handler.live));
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let unresolved = false;
        let mut owners = state.pending_owner.take().into_iter().collect::<Vec<_>>();
        for key in &mut state.keys {
            key.live = false;
        }
        for entry in &mut state.handlers {
            entry.live = false;
            owners.extend(entry.owner.take());
        }
        // A raw Carbon event remains borrowed by the current callback. Leave
        // its allocation in place until emit returns, but refuse clean drain.
        (owners, !state.events.is_empty(), unresolved)
    };
    for owner in owners { busy |= owner.revoke().is_err(); }
    if busy { return Err("Carbon callback busy; its owner must remain retained".into()); }
    if unresolved { return Err("Reference Carbon retired a live key or handler".into()); }
    Ok(())
}
fn native_owner(expected: &NativeInputInstallation) -> Result<Arc<Context>, String> {
    // Core checked the owner thread when obtaining the expected installation;
    // verify it again before consulting the existing process-global slot.
    expected.pure_after()?;
    let owner = CURRENT.lock().map_err(|_| "Carbon owner poisoned")?.as_ref().cloned()
        .ok_or("Native Carbon owner is not installed")?;
    let native = owner.native.as_ref().ok_or("Carbon fixture is not Native Input authority")?;
    if !native.installation.matches(expected) { return Err("Native Carbon installation differs".into()); }
    owner.check()?;
    expected.pure_after()?;
    if !CURRENT.lock().map_err(|_| "Carbon owner poisoned")?.as_ref().is_some_and(|c| Arc::ptr_eq(c, &owner)) {
        return Err("Native Carbon owner changed during verification".into());
    }
    Ok(owner)
}
pub(super) fn verify_owner_installation(expected: &NativeInputInstallation) -> Result<(), String> {
    native_owner(expected).map(|_| ())
}
pub(super) fn owner_ticket(expected: &NativeInputInstallation, slot: u64, generation: u64) -> Result<Arc<NativeInputOwnerTicket>, String> {
    let owner = native_owner(expected)?;
    let ticket = {
        let state = owner.state.lock().map_err(|_| "Carbon state poisoned")?;
        state.pending_owner.iter().chain(state.handlers.iter().filter_map(|h| h.owner.as_ref()))
            .find(|ticket| ticket.slot() == slot && ticket.generation() == generation)
            .cloned().ok_or("Native Carbon ticket key is not registered")?
    };
    owner.check()?;
    let native = owner.native.as_ref().ok_or("Native Carbon binding absent")?;
    if !ticket.matches_callback(&native.callback) { return Err("Native Carbon ticket issuer differs".into()); }
    expected.pure_after()?;
    Ok(ticket)
}
pub(super) fn publish_owner_ticket(expected: &NativeInputInstallation, ticket: Arc<NativeInputOwnerTicket>) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let native = owner.native.as_ref().ok_or("Native Carbon binding absent")?;
    if ticket.family() != NativeInputOwnerFamily::Carbon || !ticket.matches_callback(&native.callback) {
        return Err("Native Carbon ticket family/code differs".into());
    }
    let entry = ticket.enter_registration()?;
    if entry.userdata()? == 0 { return Err("Native Carbon ticket userdata absent".into()); }
    {
        let mut state = owner.state.lock().map_err(|_| "Carbon state poisoned")?;
        if state.pending_owner.is_some() || state.handlers.iter().any(|h| h.live) {
            return Err("Native Carbon owner already pending or registered".into());
        }
        state.pending_owner = Some(ticket.clone());
    }
    let result = owner.check().and_then(|_| entry.pure_after_callback());
    if result.is_err() {
        let _ = ticket.revoke();
        if let Ok(mut state) = owner.state.lock() {
            if state.pending_owner.as_ref().is_some_and(|p| Arc::ptr_eq(p, &ticket)) { state.pending_owner = None; }
        }
    }
    result
}
pub(super) fn revoke_owner_ticket(expected: &NativeInputInstallation, ticket: &NativeInputOwnerTicket) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let native = owner.native.as_ref().ok_or("Native Carbon binding absent")?;
    if !ticket.matches_callback(&native.callback) { return Err("Native Carbon ticket issuer differs".into()); }
    let matches = |candidate: &Arc<NativeInputOwnerTicket>| std::ptr::eq(&**candidate, ticket);
    let found = {
        let mut state = owner.state.lock().map_err(|_| "Carbon state poisoned")?;
        let mut found = state.pending_owner.as_ref().is_some_and(matches);
        for handler in &mut state.handlers {
            if handler.owner.as_ref().is_some_and(matches) { handler.live = false; found = true; }
        }
        found
    };
    if !found { return Err("Native Carbon owner ticket is not registered".into()); }
    // Retain the actual registration while busy so a later cleanup request
    // can find this same ticket, rather than reconstructing one from scalars.
    ticket.revoke()?;
    let mut state = owner.state.lock().map_err(|_| "Carbon state poisoned")?;
    if state.pending_owner.as_ref().is_some_and(matches) { state.pending_owner = None; }
    Ok(())
}
fn with<T>(f: impl FnOnce(&Context) -> Option<T>) -> Option<T> {
    with_context(false, f)
}
fn with_context<T>(cleanup: bool, f: impl FnOnce(&Context) -> Option<T>) -> Option<T> {
    // Before publication, do not take locks, inspect caller pointers, allocate
    // Rust state or call the lifecycle/ObjC/CF/platform boundary.
    if !PUBLISHED.load(Ordering::Acquire) {
        return None;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let context = CURRENT.lock().ok()?.as_ref()?.clone();
        context.check().ok()?;
        if !PUBLISHED.load(Ordering::Acquire)
            || !CURRENT
                .lock()
                .ok()?
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &context))
            || context.retired.load(Ordering::Acquire)
        {
            return None;
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if !cleanup && context.reference.as_ref().is_some_and(|source| source.cleanup_only.load(Ordering::Acquire)) {
            return None;
        }
        let _ = cleanup;
        f(&context)
    }))
    .ok()
    .flatten()
}
unsafe extern "C" fn target() -> Ptr {
    with(|c| Some(ptr(&c.target))).unwrap_or(std::ptr::null_mut())
}
#[cfg(target_arch="aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn add(
    _target:Ptr,_callback:Option<Handler>,_count:u32,_types:*const EventType,_user:Ptr,_out:*mut Ptr,
)->i32 {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!","mov x29,sp","mov x6,x30","bl {checked}","ldp x29,x30,[sp],#16","ret",checked=sym add_checked);
}
#[cfg(not(target_arch="aarch64"))]
unsafe extern "C" fn add(target:Ptr,callback:Option<Handler>,count:u32,types:*const EventType,user:Ptr,out:*mut Ptr)->i32 {
    unsafe{add_checked(target,callback,count,types,user,out,0)}
}
unsafe extern "C" fn add_checked(
    target: Ptr,
    callback: Option<Handler>,
    count: u32,
    types: *const EventType,
    user: Ptr,
    out: *mut Ptr,
    return_pc:usize,
) -> i32 {
    if out.is_null() || !out.is_aligned() {
        return PARAM;
    }
    unsafe {
        out.write(std::ptr::null_mut());
    }
    with(|c| {
        if target != ptr(&c.target) || count != 1 || types.is_null() || user.is_null() {
            return Some(PARAM);
        }
        let callback = callback?;
        if !c.callbacks.contains(&(callback as usize)) {
            return Some(PARAM);
        }
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&c.reference {reference.registration(callback,return_pc).ok()?;}
        let _=return_pc;
        c.check_pure().ok()?;
        let kind = unsafe { types.read_unaligned() };
        if kind.class != KEYBOARD || kind.kind != 5 {
            return Some(PARAM);
        }
        let mut state = c.state.lock().ok()?;
        if state.handlers.iter().any(|h| h.live) || state.handlers.len() >= 256 {
            return Some(PARAM);
        }
        let owner = if let Some(native) = &c.native {
            let owner = state.pending_owner.as_ref()?;
            if !owner.matches_callback(&native.callback) { return Some(PARAM); }
            let entry = owner.enter().ok()?;
            if entry.userdata().ok()? != user as usize { return Some(PARAM); }
            native.installation.pure_after().ok()?;
            entry.pure_after_callback().ok()?;
            Some(owner.clone())
        } else { None };
        let entry = Installed {
            token: Box::new(0),
            callback,
            user: user as usize,
            live: true,
            owner,
        };
        c.check_pure().ok()?;
        unsafe {
            out.write(ptr(&entry.token));
        }
        state.handlers.push(entry);
        state.pending_owner = None;
        Some(0)
    })
    .unwrap_or(PARAM)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn remove(_: Ptr) -> i32 {
    core::arch::naked_asm!(
        "mov x1, x30",
        "mov x2, x19",
        "b {checked}",
        checked = sym remove_checked,
    );
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn remove(handle: Ptr) -> i32 {
    unsafe { remove_checked(handle, 0, 0) }
}

unsafe extern "C" fn remove_checked(handle: Ptr, caller: usize, owner: usize) -> i32 {
    with_context(true, |c| {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(source) = &c.reference {
            let base = source.code().ok()?;
            if base.checked_add(0xf5d48) != Some(caller) || owner == 0 {
                return Some(PARAM);
            }
            for (offset, length, sha) in [
                (0xf5d30, 24, "eba7041c697ee09ad7e13b1521d50bea634bd583994bb6d66bc5d4fd0a1c448d"),
                (0x2fb480, 12, "a21dae79e902ecae573c1e3a38127b95be6a868bd9c74ee561ee5c4e910ab15b"),
            ] {
                source.context.verify_main_code(base.checked_add(offset)?, length, sha).ok()?;
            }
            let (_, imports) = source.module.get()?;
            let import = imports.iter().find(|row| row.symbol == c"RemoveEventHandler")?;
            if base.checked_add(0x3c4b60) != Some(import.cell_address())
                || import.selected_address() != import.replacement
            {
                return Some(PARAM);
            }
        }
        let _ = (caller, owner);
        let mut state = c.state.lock().ok()?;
        c.check_pure().ok()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() && state.keys.iter().any(|key| key.live) {
            return Some(PARAM);
        }
        let entry = state
            .handlers
            .iter_mut()
            .find(|h| (h.live || c.native.is_some()) && ptr(&h.token) == handle)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() && entry.user != owner {
            return Some(PARAM);
        }
        entry.live = false;
        if entry.owner.as_ref().is_some_and(|owner| owner.revoke().is_err()) {
            return Some(PARAM);
        }
        Some(0)
    })
    .unwrap_or(PARAM)
}
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn register(
    _code: u32, _modifiers: u32, _id: Id, _target: Ptr, _options: u32, _out: *mut Ptr,
) -> i32 {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!", "mov x29,sp", "mov x6,x30",
        "bl {checked}", "ldp x29,x30,[sp],#16", "ret", checked = sym register_checked);
}
#[cfg(not(target_arch = "aarch64"))]
unsafe extern "C" fn register(code: u32, modifiers: u32, id: Id, target: Ptr, options: u32, out: *mut Ptr) -> i32 {
    unsafe { register_checked(code, modifiers, id, target, options, out, 0) }
}
unsafe extern "C" fn register_checked(
    code: u32,
    modifiers: u32,
    id: Id,
    target: Ptr,
    options: u32,
    out: *mut Ptr,
    return_pc: usize,
) -> i32 {
    if out.is_null() || !out.is_aligned() {
        return PARAM;
    }
    unsafe {
        out.write(std::ptr::null_mut());
    }
    with(|c| {
        if target != ptr(&c.target) || options != 0 {
            return Some(PARAM);
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(reference) = &c.reference {
            // The frozen Hotkeys.update emits only these four Carbon flags.
            const MODIFIERS: u32 = (1 << 8) | (1 << 9) | (1 << 11) | (1 << 12);
            let base = reference.code().ok()?;
            if base.checked_add(0xf4604)? != return_pc || id.signature != u32::from_be_bytes(*b"RUCA")
                || id.id == 0 || modifiers & !MODIFIERS != 0
            {
                c.retired.store(true, Ordering::Release);
                return Some(PARAM);
            }
            reference.context.verify_main_code(base.checked_add(0xf45ec)?, 32,
                "8e591a21809aab0a6991018f60ae9feba76edb5454757ef9002db442eddb2f4c").ok()?;
            reference.context.verify_main_code(base.checked_add(0x2fb474)?, 12,
                "cfc08075e495c99561abcad6c980437a7a51b32ff880abdd80a61e7263858c30").ok()?;
            let (_, imports) = reference.module.get()?;
            let import = imports.iter().find(|row| row.symbol == c"RegisterEventHotKey")?;
            if base.checked_add(0x3c4b58) != Some(import.cell_address())
                || import.selected_address() != import.replacement
            {
                return Some(PARAM);
            }
        }
        let _ = return_pc;
        let mut state = c.state.lock().ok()?;
        c.check_pure().ok()?;
        if state
            .keys
            .iter()
            .any(|k| k.live && k.code == code && k.modifiers == modifiers)
        {
            return Some(CONFLICT);
        }
        if state.keys.len() >= 4096 {
            return Some(PARAM);
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let reference_handler = if c.reference.is_some() {
            let mut handlers = state.handlers.iter().filter(|entry| entry.live);
            let handler = handlers.next()?;
            if handlers.next().is_some() || handler.user == 0 {
                return Some(PARAM);
            }
            Some(ptr(&handler.token) as usize)
        } else {
            None
        };
        let key = Key {
            token: Box::new(0),
            code,
            modifiers,
            id,
            live: true,
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            reference_handler,
        };
        unsafe {
            out.write(ptr(&key.token));
        }
        state.keys.push(key);
        Some(0)
    })
    .unwrap_or(PARAM)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn unregister(_: Ptr) -> i32 {
    core::arch::naked_asm!(
        "mov x1, x30",
        "mov x2, x19",
        "b {checked}",
        checked = sym unregister_checked,
    );
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn unregister(handle: Ptr) -> i32 {
    unsafe { unregister_checked(handle, 0, 0) }
}

unsafe extern "C" fn unregister_checked(handle: Ptr, caller: usize, owner: usize) -> i32 {
    with_context(true, |c| {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let deinit = if let Some(source) = &c.reference {
            let base = source.code().ok()?;
            let (offset, sha) = match caller.checked_sub(base)? {
                0xf392c => (0xf391c, "1e6fc785d0132a5d44e15e2fa88dc1d76d98f4cb037b26671c9d92f4e42c4a18"),
                0xf4930 => (0xf4920, "3e89074f6ea81197d81e74a2c038e34c8e3f4784b3290458474780f024b2798d"),
                0xf4b34 => (0xf4b24, "ac99b6ae9718f4375631d72f85681cf3658b526098a72a255face7ce9091cc46"),
                0xf5d24 => (0xf5d14, "e68bdc31bcaa53910253abf19a9244777234c06c4972a020b6ec57f41d23a523"),
                _ => return Some(PARAM),
            };
            if source.cleanup_only.load(Ordering::Acquire) && offset != 0xf5d14 {
                return Some(PARAM);
            }
            source.context.verify_main_code(base.checked_add(offset)?, 16, sha).ok()?;
            if matches!(offset, 0xf391c | 0xf5d14) {
                source.context.verify_main_code(base.checked_add(0xf74e0)?, 16,
                    "13467e81e68d93358b31110f3731d35e294abcc310d85a3405db9aa6f004bb0f").ok()?;
            }
            source.context.verify_main_code(base.checked_add(0x2fb4bc)?, 12,
                "37e0de1f0cef34e3f5475a4f3266fbf906418968ae51cee2ad39e579a9e52b23").ok()?;
            let (_, imports) = source.module.get()?;
            let import = imports.iter().find(|row| row.symbol == c"UnregisterEventHotKey")?;
            if base.checked_add(0x3c4b68) != Some(import.cell_address())
                || import.selected_address() != import.replacement
            {
                return Some(PARAM);
            }
            offset == 0xf5d14
        } else {
            false
        };
        let _ = (caller, owner);
        let mut state = c.state.lock().ok()?;
        c.check_pure().ok()?;
        let index = state
            .keys
            .iter()
            .position(|k| k.live && ptr(&k.token) == handle)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() && !state.handlers.iter().any(|entry| {
            entry.live && state.keys[index].reference_handler == Some(ptr(&entry.token) as usize)
                && (!deinit || owner != 0 && entry.user == owner)
        }) {
            return Some(PARAM);
        }
        state.keys[index].live = false;
        Some(0)
    })
    .unwrap_or(PARAM)
}
unsafe extern "C" fn class(event: Ptr) -> u32 {
    with(|c| Some(c.state.lock().ok()?.events.get(&(event as usize))?.class)).unwrap_or(0)
}
unsafe extern "C" fn kind(event: Ptr) -> u32 {
    with(|c| Some(c.state.lock().ok()?.events.get(&(event as usize))?.kind)).unwrap_or(0)
}
unsafe extern "C" fn parameter(
    event: Ptr,
    name: u32,
    expected: u32,
    actual: *mut u32,
    capacity: usize,
    size: *mut usize,
    output: Ptr,
) -> i32 {
    with(|c| {
        if name != DIRECT || expected != HOTKEY || capacity < std::mem::size_of::<Id>() || output.is_null()
            || !actual.is_aligned() || !size.is_aligned()
        {
            return Some(PARAM);
        }
        let state = c.state.lock().ok()?;
        let value = state.events.get(&(event as usize))?;
        if value.status != 0 {
            return Some(value.status);
        }
        if value.parameter_type != expected || value.size != std::mem::size_of::<Id>() {
            return Some(PARAM);
        }
        unsafe {
            if !actual.is_null() {
                actual.write(value.parameter_type);
            }
            if !size.is_null() {
                size.write(value.size);
            }
            output.cast::<Id>().write_unaligned(value.id);
        }
        Some(0)
    })
    .unwrap_or(PARAM)
}
/// Exact implemented C leaves; the roster establishes no import-cell selection.
pub fn replacements() -> [(&'static CStr, usize); 8] {
    [
        (c"GetApplicationEventTarget", target as *const () as usize),
        (c"InstallEventHandler", add as *const () as usize),
        (c"RemoveEventHandler", remove as *const () as usize),
        (c"RegisterEventHotKey", register as *const () as usize),
        (c"UnregisterEventHotKey", unregister as *const () as usize),
        (c"GetEventParameter", parameter as *const () as usize),
        (c"GetEventClass", class as *const () as usize),
        (c"GetEventKind", kind as *const () as usize),
    ]
}
/// Fixed typed system entries, observed only; no original Carbon function is called.
pub fn import_specs() -> [super::ImportSpec; 8] {
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn GetApplicationEventTarget() -> Ptr;
        fn InstallEventHandler(target: Ptr, callback: Option<Handler>, count: u32, types: *const EventType, user: Ptr, out: *mut Ptr) -> i32;
        fn RemoveEventHandler(handle: Ptr) -> i32;
        fn RegisterEventHotKey(code: u32, modifiers: u32, id: Id, target: Ptr, options: u32, out: *mut Ptr) -> i32;
        fn UnregisterEventHotKey(handle: Ptr) -> i32;
        fn GetEventParameter(event: Ptr, name: u32, expected: u32, actual: *mut u32, capacity: usize, size: *mut usize, output: Ptr) -> i32;
        fn GetEventClass(event: Ptr) -> u32;
        fn GetEventKind(event: Ptr) -> u32;
    }
    let original = [GetApplicationEventTarget as *const () as usize, InstallEventHandler as *const () as usize,
        RemoveEventHandler as *const () as usize, RegisterEventHotKey as *const () as usize,
        UnregisterEventHotKey as *const () as usize, GetEventParameter as *const () as usize,
        GetEventClass as *const () as usize, GetEventKind as *const () as usize];
    let leaves = replacements();
    std::array::from_fn(|i| super::ImportSpec { symbol: leaves[i].0, original: original[i], replacement: leaves[i].1 })
}
/// Import-memory observation through the actual retained bootstrap issuer.
/// This is not callback/source authority, complete import coverage, Context
/// installation or permission to register an Input scope. The coordinator owns
/// the existing main-import transaction and the separate C callback/source and
/// registered-userdata lifetime proof before any callback can be invoked.
pub fn observe_owned_imports(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
) -> Result<(super::owned::OwnedHandleLease, Vec<super::ImportReplacement>), String> {
    let context = super::bootstrap::owned_module_context(life, descriptors)?;
    let lease = context.lease("input-carbon")?;
    let imports = super::observe_main_imports(&import_specs())?;
    let confirmed = super::bootstrap::owned_module_context(life, descriptors)?;
    if !Arc::ptr_eq(&lease.life, &confirmed.life) { return Err("Carbon import issuer changed".into()); }
    lease.before_effect()?;
    if imports.iter().any(|row| row.selected_address() != row.replacement) {
        return Err("Carbon selected import changed".into());
    }
    Ok((lease, imports))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Life(AtomicBool);
    struct PanickingLife {
        mode: AtomicUsize,
        hits: AtomicUsize,
    }
    impl AliveLease for PanickingLife {
        fn is_alive(&self) -> bool {
            if self.mode.load(Ordering::Acquire) == 2 {
                self.hits.fetch_add(1, Ordering::AcqRel);
                panic!("own is_alive regression panic");
            }
            true
        }
        fn before_effect(&self) -> Result<(), String> {
            if self.mode.load(Ordering::Acquire) == 1 {
                self.hits.fetch_add(1, Ordering::AcqRel);
                panic!("own before_effect regression panic");
            }
            Ok(())
        }
    }
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<(), String> {
            if self.is_alive() {
                Ok(())
            } else {
                Err("own retired test guardian".into())
            }
        }
    }
    unsafe extern "C" fn callback(_: Ptr, event: Ptr, user: Ptr) -> i32 {
        assert_eq!(unsafe { class(event) }, KEYBOARD);
        assert_eq!(unsafe { kind(event) }, 5);
        let mut id = Id {
            signature: 0,
            id: 0,
        };
        // Actual Swift callback supplies null actual/size; preserve that ABI.
        assert_eq!(
            unsafe {
                parameter(
                    event,
                    DIRECT,
                    HOTKEY,
                    std::ptr::null_mut(),
                    8,
                    std::ptr::null_mut(),
                    (&mut id as *mut Id).cast(),
                )
            },
            0
        );
        unsafe { &*user.cast::<AtomicUsize>() }.store(id.id as usize, Ordering::Release);
        0
    }
    unsafe extern "C" fn foreign(_: Ptr, _: Ptr, _: Ptr) -> i32 {
        panic!("unadmitted callback executed")
    }
    fn denied() {
        let bad = std::ptr::dangling_mut::<c_void>();
        let mut out = bad;
        assert!(unsafe { target() }.is_null());
        assert_eq!(
            unsafe { add(bad, Some(foreign), 1, bad.cast(), bad, &mut out) },
            PARAM
        );
        assert!(out.is_null());
        out = bad;
        assert_eq!(
            unsafe {
                register(
                    1,
                    1,
                    Id {
                        signature: 1,
                        id: 1,
                    },
                    bad,
                    0,
                    &mut out,
                )
            },
            PARAM
        );
        assert!(out.is_null());
        assert_eq!(unsafe { remove(bad) }, PARAM);
        assert_eq!(unsafe { unregister(bad) }, PARAM);
        assert_eq!(unsafe { class(bad) }, 0);
        assert_eq!(unsafe { kind(bad) }, 0);
        assert_eq!(
            unsafe { parameter(bad, DIRECT, HOTKEY, bad.cast(), 8, bad.cast(), bad) },
            PARAM
        );
    }
    #[test]
    fn reference_startup_requires_both_fixture_hotkeys() {
        let key = |code, id| Key {
            token: Box::new(0),
            code,
            modifiers: (1 << 12) | (1 << 11),
            id: Id { signature: u32::from_be_bytes(*b"RUCA"), id },
            live: true,
        };
        let mut keys = [key(49, 1), key(9, 2)];
        assert!(verify_reference_startup_keys(&keys).is_ok());
        assert!(verify_reference_startup_keys(&[key(9, 2), key(49, 1)]).is_ok());
        assert!(verify_reference_startup_keys(&[key(49, 2), key(9, 1)]).is_err());
        assert!(verify_reference_startup_keys(&[key(49, 3), key(9, 4)]).is_err());
        assert!(verify_reference_startup_keys(&keys[..1]).is_err());
        keys[1].id.id = 1;
        assert!(verify_reference_startup_keys(&keys).is_err());
        keys[1] = key(8, 2);
        assert!(verify_reference_startup_keys(&keys).is_err());
        keys[1] = key(9, 2);
        keys[1].live = false;
        assert!(verify_reference_startup_keys(&keys).is_err());
    }
    #[test]
    fn eight_leaves_preserve_startup_binding_callback_cleanup_and_denial() {
        let _publication = PUBLICATION_TEST_LOCK.lock().unwrap();
        assert_eq!(
            replacements()
                .iter()
                .map(|r| r.0)
                .collect::<BTreeSet<_>>()
                .len(),
            8
        );
        assert_eq!(std::mem::size_of::<Id>(), 8);
        assert_eq!(std::mem::size_of::<EventType>(), 8);
        denied();
        let life = Arc::new(Life(AtomicBool::new(true)));
        let context = Context::new(life.clone(), [callback as usize].into()).unwrap();
        install(context.clone()).unwrap();
        assert!(install(context.clone()).is_err());
        let target = unsafe { target() };
        let count = AtomicUsize::new(0);
        let user = std::ptr::from_ref(&count).cast_mut().cast();
        let event_type = EventType {
            class: KEYBOARD,
            kind: 5,
        };
        let mut handler = std::ptr::null_mut();
        assert_eq!(
            unsafe { add(target, Some(foreign), 1, &event_type, user, &mut handler) },
            PARAM
        );
        assert!(handler.is_null());
        assert_eq!(
            unsafe { add(target, Some(callback), 1, &event_type, user, &mut handler) },
            0
        );
        let mut second = std::ptr::null_mut();
        assert_eq!(
            unsafe { add(target, Some(callback), 1, &event_type, user, &mut second) },
            PARAM
        );
        assert!(second.is_null());
        let id = Id {
            signature: 0x52554341,
            id: 9,
        };
        let mut key = std::ptr::null_mut();
        assert_eq!(unsafe { register(40, 256, id, target, 0, &mut key) }, 0);
        let mut duplicate = key;
        assert_eq!(
            unsafe { register(40, 256, id, target, 0, &mut duplicate) },
            CONFLICT
        );
        assert!(duplicate.is_null());
        assert_eq!(context.emit_binding(40, 256).unwrap(), 0);
        assert_eq!(count.load(Ordering::Acquire), 9);
        assert_eq!(unsafe { unregister(key) }, 0);
        assert_eq!(unsafe { unregister(key) }, PARAM);
        assert_eq!(context.emit_binding(40, 256).unwrap(), UNHANDLED);
        let mut replacement = std::ptr::null_mut();
        assert_eq!(unsafe { register(40, 256, id, target, 0, &mut replacement) }, 0);
        assert_ne!(replacement, key);
        count.store(0, Ordering::Release);
        let pending = Event {
            class: KEYBOARD,
            kind: 5,
            id,
            status: 0,
            parameter_type: HOTKEY,
            size: 8,
        };
        assert!(context.emit_registered(pending, Some(key as usize)).is_err());
        assert_eq!(count.load(Ordering::Acquire), 0);
        let rejected = Box::new(Event { status: PARAM, ..pending });
        let pointer = std::ptr::from_ref(&*rejected) as usize;
        context.state.lock().unwrap().events.insert(pointer, rejected);
        let mut actual = 0;
        let mut size = 0;
        let mut output = [0xa5u8; 8];
        assert_eq!(unsafe { parameter(pointer as Ptr, DIRECT, HOTKEY, &mut actual, output.len(), &mut size, output.as_mut_ptr().cast()) }, PARAM);
        assert_eq!((actual, size, output), (0, 0, [0xa5; 8]));
        context.state.lock().unwrap().events.remove(&pointer);
        assert_eq!(context.emit_binding(40, 256).unwrap(), 0);
        assert_eq!(count.load(Ordering::Acquire), 9);
        assert_eq!(unsafe { unregister(replacement) }, 0);
        assert_eq!(unsafe { remove(handler) }, 0);
        assert_eq!(unsafe { remove(handler) }, PARAM);
        assert_eq!(
            context
                .emit(Event {
                    class: KEYBOARD,
                    kind: 5,
                    id,
                    status: 0,
                    parameter_type: HOTKEY,
                    size: 8
                })
                .unwrap(),
            UNHANDLED
        );
        // Wrong owner thread must not use the configured target or callback.
        std::thread::spawn(|| assert!(unsafe { super::target() }.is_null()))
            .join()
            .unwrap();
        life.0.store(false, Ordering::Release);
        denied();
        life.0.store(true, Ordering::Release);
        denied();
        retire(&context).unwrap();
        denied();
        unsafe extern "C" fn retire_inside_callback(_: Ptr, event: Ptr, user: Ptr) -> i32 {
            let context = CURRENT.lock().ok().and_then(|slot| slot.clone());
            let Some(context) = context else { return PARAM; };
            let refused = retire(&context).is_err();
            let retained = context.state.lock().ok().is_some_and(|state| {
                state.events.contains_key(&(event as usize)) && state.keys.iter().all(|key| !key.live)
            });
            unsafe { &*user.cast::<AtomicUsize>() }.store(usize::from(refused && retained), Ordering::Release);
            0
        }
        let context = Context::new(life.clone(), [retire_inside_callback as usize].into()).unwrap();
        install(context.clone()).unwrap();
        let target = unsafe { super::target() };
        assert_eq!(unsafe { add(target, Some(retire_inside_callback), 1, &event_type, user, &mut handler) }, 0);
        assert_eq!(unsafe { register(40, 256, id, target, 0, &mut key) }, 0);
        assert!(context.emit_binding(40, 256).is_err());
        assert_eq!(count.load(Ordering::Acquire), 1);
        assert!(context.state.lock().unwrap().events.is_empty());
        denied();
        // Each actual panic path gets its own newly published context. Resetting
        // the trait cannot reactivate that context after either kind of panic.
        for mode in [1, 2] {
            let life = Arc::new(PanickingLife {
                mode: AtomicUsize::new(0),
                hits: AtomicUsize::new(0),
            });
            let context = Context::new(life.clone(), [callback as usize].into()).unwrap();
            install(context.clone()).unwrap();
            assert!(!unsafe { super::target() }.is_null());
            life.mode.store(mode, Ordering::Release);
            assert!(unsafe { super::target() }.is_null());
            assert_eq!(life.hits.load(Ordering::Acquire), 1);
            life.mode.store(0, Ordering::Release);
            denied();
            assert!(context.emit_binding(40, 256).is_err());
            assert!(install(context.clone()).is_err());
            assert_eq!(life.hits.load(Ordering::Acquire), 1);
            retire(&context).unwrap();
            denied();
        }
    }
}

#[cfg(test)]
mod import_issuer_tests {
    use super::*;
    struct Unissued;
    impl AliveLease for Unissued {
        fn is_alive(&self) -> bool { panic!("An unissued input lease must never be queried"); }
        fn before_effect(&self) -> Result<(), String> { panic!("An unissued input lease must never grant effects"); }
    }
    #[test]
    fn fixed_imports_require_the_actual_bootstrap_issuer_before_observation() {
        let _publication = PUBLICATION_TEST_LOCK.lock().unwrap();
        let specs = import_specs();
        let leaves = replacements();
        assert_eq!(specs.len(), 8);
        assert_eq!(specs.iter().map(|s| s.symbol).collect::<std::collections::BTreeSet<_>>().len(), 8);
        for (spec, leaf) in specs.iter().zip(leaves) {
            assert_eq!(spec.symbol, leaf.0);
            assert_eq!(spec.replacement, leaf.1);
            assert!(spec.original != 0 && spec.replacement != 0 && spec.original != spec.replacement);
        }
        let life: Arc<dyn AliveLease> = Arc::new(Unissued);
        let descriptors = super::super::descriptors::DescriptorTable::new();
        let published = PUBLISHED.load(Ordering::Acquire);
        assert!(observe_owned_imports(&life, &descriptors).is_err());
        assert_eq!(PUBLISHED.load(Ordering::Acquire), published);
    }
}
