//! Own main-thread CG input effects. No real CG tap, event source, event posting,
//! CFMachPort, input query or loop handler. Only the source factory uses genuine
//! process-local CFRunLoopSource; exact common-loop ownership stays external.
use super::owned::AliveLease;
use super::{core_preentry::{NativeInputAdmission, NativeInputCallbackWitness, NativeInputContextKind, NativeInputInstallation, NativeInputOwnerFamily, NativeInputOwnerTicket}, owned::ModuleContext};
use crate::workspace::cf_loop::OwnedSource;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::{c_char, c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::NonNull,
    rc::{Rc, Weak},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, ThreadId},
};
pub type Ptr = *mut c_void;
pub type Callback = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> Ptr;
const MAX: usize = 65536;
#[repr(C)]
struct RuntimeClass {
    version: isize,
    name: *const c_char,
    init: usize,
    copy: usize,
    finalize: Option<unsafe extern "C" fn(Ptr)>,
    equal: usize,
    hash: usize,
    formatting: usize,
    debug: usize,
    reclaim: usize,
    refcount: usize,
    alignment: usize,
}
// Process-static descriptors, identical reviewed CFRuntime-v0 layout. Object
// storage/layout is opaque; mutable event data lives only in owner-thread Rust.
unsafe impl Sync for RuntimeClass {}
const fn class(name: *const c_char) -> RuntimeClass {
    RuntimeClass {
        version: 0,
        name,
        init: 0,
        copy: 0,
        finalize: Some(finalize),
        equal: 0,
        hash: 0,
        formatting: 0,
        debug: 0,
        reclaim: 0,
        refcount: 0,
        alignment: 0,
    }
}
static SOURCE: RuntimeClass = class(c"RuycastF1OwnedCGSource".as_ptr());
static EVENT: RuntimeClass = class(c"RuycastF1OwnedCGEvent".as_ptr());
static PORT: RuntimeClass = class(c"RuycastF1OwnedCGPort".as_ptr());
#[repr(C)]
struct SourceContext {
    version: isize,
    info: Ptr,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    description: usize,
    equal: usize,
    hash: usize,
    schedule: Option<unsafe extern "C" fn(Ptr, Ptr, *const c_void)>,
    cancel: Option<unsafe extern "C" fn(Ptr, Ptr, *const c_void)>,
    perform: Option<unsafe extern "C" fn(Ptr)>,
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn _CFRuntimeRegisterClass(class: *const RuntimeClass) -> usize;
    fn _CFRuntimeGetClassWithTypeID(id: usize) -> *const RuntimeClass;
    fn _CFRuntimeCreateInstance(
        allocator: Ptr,
        id: usize,
        extra: isize,
        category: *const u8,
    ) -> Ptr;
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CFRunLoopSourceCreate(
        allocator: *const c_void,
        order: isize,
        context: *mut SourceContext,
    ) -> Ptr;
    fn CFRunLoopSourceSignal(source: Ptr);
    fn CFRunLoopSourceInvalidate(source: Ptr);
    fn CFRunLoopWakeUp(loop_: Ptr);
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch="aarch64")))]
    fn CFRunLoopContainsSource(loop_: Ptr, source: Ptr, mode: *const c_void) -> u8;
}
static TYPES: OnceLock<Result<[usize; 3], String>> = OnceLock::new();
fn types() -> Result<[usize; 3], String> {
    TYPES
        .get_or_init(|| {
            let mut ids = [0; 3];
            for (i, class) in [&raw const SOURCE, &raw const EVENT, &raw const PORT]
                .into_iter()
                .enumerate()
            {
                let id = unsafe { _CFRuntimeRegisterClass(class) };
                if id == 0
                    || ids.contains(&id)
                    || unsafe { _CFRuntimeGetClassWithTypeID(id) } != class
                {
                    return Err("own CG CF class identity refused".into());
                }
                ids[i] = id;
            }
            Ok(ids)
        })
        .clone()
}
fn cached() -> Option<[usize; 3]> {
    TYPES.get().and_then(|r| r.as_ref().ok().copied())
}
// Deliberately !Send/!Sync: all admitted original CG/native-paste/system-action
// routes are main owned. No unsafe Send wrapper is put around OwnedSource.
struct Token(NonNull<c_void>);
impl Token {
    fn ptr(&self) -> Ptr {
        self.0.as_ptr()
    }
    fn retained(&self) -> Ptr {
        unsafe { CFRetain(self.ptr()).cast_mut() }
    }
    unsafe fn retain(pointer: Ptr) -> Option<Self> {
        NonNull::new(unsafe { CFRetain(pointer).cast_mut() }).map(Self)
    }
}
impl Clone for Token {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.ptr());
        }
        Self(self.0)
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr());
        }
    }
}
#[derive(Clone, PartialEq, Eq, Default)]
pub struct EventData {
    pub kind: u32,
    pub flags: u64,
    pub key: i64,
    pub repeat: i64,
    pub tag: i64,
    pub text: Vec<u16>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Posted {
    pub location: u32,
    pub event: EventData,
}
struct Event {
    data: RefCell<EventData>,
    _source: Option<Token>,
}
struct PortState {
    valid: bool,
    enabled: bool,
    scheduled: bool,
    enabled_after_schedule: bool,
}
impl PortState {
    fn invalidate(&mut self) {
        self.valid = false;
        self.enabled = false;
        self.scheduled = false;
        self.enabled_after_schedule = false;
    }
}
struct Port {
    state: RefCell<PortState>,
    callback: Callback,
    user: usize,
    mask: u64,
    sources: RefCell<Vec<Weak<Info>>>,
    native_owner: Option<(Arc<NativeInputOwnerTicket>, NativeInputCallbackWitness)>,
}
impl Port {
    fn accepts_kind(&self, kind: u32, force: bool) -> bool {
        if force {
            kind >= 0xfffffffe
        } else {
            kind < 64 && self.mask & (1u64 << kind) != 0
        }
    }
}
enum Payload {
    Source(i32),
    Event(Rc<Event>),
    Port(Rc<Port>),
}
struct Entry {
    owner: u64,
    payload: Payload,
}
thread_local! {static ENTRIES:RefCell<BTreeMap<usize,Entry>>=const{RefCell::new(BTreeMap::new())};static CURRENT:RefCell<Option<Rc<Context>>>=const{RefCell::new(None)};}
// Panic payloads are opaque user values: even dropping one may panic again.
// Foreign finalization must contain both the owned destructor and that payload.
fn retire_after_cleanup_panic(nonce: Option<u64>) {
    let _ = CURRENT.try_with(|slot| {
        if let Ok(slot) = slot.try_borrow() {
            if let Some(owner) = slot.as_ref().filter(|c| Some(c.nonce) == nonce) {
                owner.retired.store(true, Ordering::Release);
            }
        }
    });
}
unsafe extern "C" fn finalize(pointer: Ptr) {
    let mut nonce = None;
    let result = catch_unwind(AssertUnwindSafe(|| {
        let removed = ENTRIES
            .try_with(|e| e.try_borrow_mut().ok()?.remove(&(pointer as usize)))
            .ok()
            .flatten();
        nonce = removed.as_ref().map(|entry| entry.owner);
        drop(removed);
    }));
    if let Err(payload) = result {
        retire_after_cleanup_panic(nonce);
        std::mem::forget(payload);
    }
}
fn token(kind: usize, owner: u64, payload: Payload) -> Result<Token, String> {
    let ids = cached().ok_or("own CG types absent")?;
    let pointer = NonNull::new(unsafe {
        _CFRuntimeCreateInstance(std::ptr::null_mut(), ids[kind], 0, std::ptr::null())
    })
    .ok_or("own CG CF allocation failed")?;
    let object = Token(pointer);
    if unsafe { CFGetTypeID(object.ptr()) } != ids[kind] {
        return Err("own CG CF allocation identity differs".into());
    }
    ENTRIES.with(|e| -> Result<(), String> {
        e.try_borrow_mut()
            .map_err(|_| "CG metadata borrowed")?
            .insert(object.ptr() as usize, Entry { owner, payload });
        Ok(())
    })?;
    Ok(object)
}
struct Pending {
    event: Token,
    targets: VecDeque<Weak<Info>>,
    force: bool,
    source_owner: Option<Arc<NativeInputOwnerTicket>>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference_source:Option<(Rc<Port>,Arc<crate::workspace::factory::ReferenceInputChildCustody>)>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    external_mouse: bool,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl Pending {
    fn reference_source_live(&self)->Result<(),String>{
        if let Some((port,child))=&self.reference_source{
            child.verify_held_epoch()?;
            let state=port.state.try_borrow().map_err(|_|"Reference CG source port borrowed")?;
            if !state.valid||!state.scheduled{return Err("Reference CG queued source retired".into());}
        }
        Ok(())
    }
}
struct Info {
    context: Rc<Context>,
    port: Token,
    model: Rc<Port>,
    source: Cell<usize>,
    pending: RefCell<VecDeque<Pending>>,
    entered: Cell<bool>,
    deferred: Cell<bool>,
}
pub struct Context {
    life: Arc<dyn AliveLease>,
    thread: ThreadId,
    nonce: u64,
    owner: Rc<RefCell<OwnedSource>>,
    requested: (usize, usize),
    callback: (usize, usize),
    codes: BTreeSet<usize>,
    retired: Arc<AtomicBool>,
    active_callbacks: Cell<usize>,
    sources: RefCell<BTreeMap<usize, Weak<Info>>>,
    taps: RefCell<Vec<Weak<Port>>>,
    posts: RefCell<Vec<Posted>>,
    last: RefCell<Option<&'static str>>,
    native: Option<NativeBinding>,
    pending_owners: RefCell<[Option<Arc<NativeInputOwnerTicket>>; 2]>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference:Option<ReferenceBinding>,
}
struct NativeBinding {
    context: ModuleContext,
    installation: NativeInputInstallation,
    hid: NativeInputCallbackWitness,
    keyword: NativeInputCallbackWitness,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
struct ReferenceBinding {
    admission:Arc<super::core_preentry::ReferenceAdmission>,
    context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
    children:[OnceLock<Arc<crate::workspace::factory::ReferenceInputChildCustody>>;2],
    module:OnceLock<Arc<ReferencePublication>>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
struct ReferencePublication {
    nonce: u64,
    thread: ThreadId,
    retired: Arc<AtomicBool>,
    lease: super::owned::OwnedHandleLease,
    imports: Vec<super::ImportReplacement>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
static REFERENCE_PUBLICATION: OnceLock<Arc<ReferencePublication>> = OnceLock::new();
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ReferenceBinding {
    fn code(&self)->Result<usize,String> {
        use sha2::Digest;
        self.context.verify_admission_binding(&self.admission,&self.model)?;
        self.context.verify_input_parent_source_members()?;
        let base=self.context.observe_app_delegate_offset_cells()?.main_header();
        for (offset,len,sha) in [
            (0xf8170usize,1708usize,"8e4721bb651f2968220304504ef6d779a083df4f73aad43fd96c0958eb871e80"),
            (0xf9348,196,"59a8044c1b69a3498d0204bdd133838fcb106706ef084b9bf1a247eca2c4743a"),
            (0xf990c,72,"e2aa223bb9ee81e4bd7ad3f85abcf492bc0540f8c33ee23e446911c894d7a3c1"),
            (0x1adc18,1088,"a76ac351402c0e882deb585f7145c4bf2dacdc268397b74e62fbb402b39c7920"),
            (0x1ae548,72,"cbe0034689e038dce301d834bc748e9f6047d98ca5475d2fcb6178b8da35d737"),
            (0x1af5a0,72,"03963f81f360cbb87366f86072b2d3ebbf0f42abb105dae48faa772d0a886cde"),
        ] {self.context.verify_main_code(base.checked_add(offset).ok_or("Reference CG code overflow")?,len,sha)?;}
        let entry=tap_create as *const () as usize;let checked=tap_create_checked as *const () as usize;
        self.context.verify_app_delegate_sdk_members(entry,checked)?;
        let delta=checked as i128-entry.checked_add(12).ok_or("CG entry overflow")? as i128;
        if delta%4!=0||!(-(1i128<<27)..(1i128<<27)).contains(&delta){return Err("CG helper outside direct branch range".into());}
        let words=[0xa9bf7bfdu32,0x910003fd,0xaa1e03e6,0x94000000|(((delta/4) as i64 as u32)&0x03ffffff),0xa8c17bfd,0xd65f03c0];
        let bytes=words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        self.context.verify_sdk_code(entry,bytes.len(),&format!("{:x}",sha2::Sha256::digest(&bytes)))?;
        if let Some(module)=self.module.get(){
            module.lease.before_effect()?;
            for row in &module.imports {if row.selected_address()!=row.replacement{return Err("Reference CG import changed".into());}}
        }
        Ok(base)
    }
    fn role(&self,callback:Callback,caller:Option<usize>)->Result<usize,String> {
        let base=self.code()?;
        for (index,(entry,return_pc)) in [(0xf990cusize,0xf8574usize),(0x1af5a0,0x1adf74)].into_iter().enumerate(){
            if base.checked_add(entry)==Some(callback as usize)
                &&caller.is_none_or(|pc|base.checked_add(return_pc)==Some(pc)){return Ok(index);}
        }
        Err("Reference CG caller/callback differs".into())
    }
    fn owner(&self,callback:Callback,user:usize)->Result<Arc<crate::workspace::factory::ReferenceInputChildCustody>,String> {
        let role=self.role(callback,None)?;
        let child=self.children[role].get().ok_or("Reference CG callback pending owner publication")?.clone();
        child.verify_reference_binding(&self.admission,&self.model)?;
        if child.pointer_on_creator()?!=user{return Err("Reference CG userdata differs from genuine held child".into());}
        child.verify_held_epoch()?;Ok(child)
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
static ACTIVE: AtomicU64 = AtomicU64::new(0);
// The existing selected nonce has an owner-thread stamp so imported frontdoors
// can reject a foreign thread before consulting its thread-local CURRENT.
static ACTIVE_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);
static PUBLISHED: AtomicBool = AtomicBool::new(false);
impl Context {
    pub fn new(
        life: Arc<dyn AliveLease>,
        owner: Rc<RefCell<OwnedSource>>,
        codes: BTreeSet<usize>,
    ) -> Result<Rc<Self>, String> {
        life.before_effect()?;
        if !life.is_alive() || codes.is_empty() || codes.len() > 64 || codes.contains(&0) {
            return Err("invalid own CG lifecycle/callbacks".into());
        }
        let (requested, callback) = {
            let owner = owner
                .try_borrow()
                .map_err(|_| "own source anchor borrowed")?;
            (
                owner.guarded_add_remove_identities()?,
                owner.guarded_default_callback_identities()?,
            )
        };
        if requested.0 != callback.0 || callback.0 == 0 || callback.1 == 0 {
            return Err("own CG loop identities disagree".into());
        }
        types()?;
        let nonce = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "CG owner identities exhausted")?
            .checked_add(1)
            .ok_or("CG owner identity overflow")?;
        Ok(Rc::new(Self {
            life,
            thread: thread::current().id(),
            nonce,
            owner,
            requested,
            callback,
            codes,
            retired: Arc::new(AtomicBool::new(false)),
            active_callbacks: Cell::new(0),
            sources: RefCell::new(BTreeMap::new()),
            taps: RefCell::new(vec![]),
            posts: RefCell::new(vec![]),
            last: RefCell::new(None),
            native: None,
            pending_owners: RefCell::new([None, None]),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            reference:None,
        }))
    }
    pub fn for_native_input(context: ModuleContext, admission: &Arc<NativeInputAdmission>, source: Rc<RefCell<OwnedSource>>) -> Result<Rc<Self>, String> {
        let hid = admission.hid_filter();
        let keyword = admission.keyword_filter();
        hid.verify(&context)?;
        keyword.verify(&context)?;
        let codes = BTreeSet::from([hid.event_handler()? as usize, keyword.event_handler()? as usize]);
        let installation = admission.reserve_installation(NativeInputContextKind::Events, &context)?;
        let mut owner = Self::new(context.life.clone(), source, codes)?;
        Rc::get_mut(&mut owner).ok_or("CG candidate was shared")?.native = Some(NativeBinding { context, installation, hid, keyword });
        owner.local_guard()?;
        if let Some(native) = &owner.native { native.installation.verify_candidate(&native.context)?; }
        Ok(owner)
    }
    fn local_guard(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("retired/foreign-thread CG context".into());
        }
        let live = catch_unwind(AssertUnwindSafe(|| {
            self.life.before_effect().is_ok() && self.life.is_alive()
        }))
        .unwrap_or(false);
        if !live {
            self.retired.store(true, Ordering::Release);
            return Err("own CG lifecycle retired".into());
        }
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("own CG authority changed during guard".into());
        }
        Ok(())
    }
    fn guard(&self) -> Result<(), String> {
        if let Some(native) = &self.native { native.installation.pure_after()?; }
        self.local_guard()?;
        if let Some(native) = &self.native { native.installation.verify(&native.context)?; }
        let result = (|| {
            let owner = self
                .owner
                .try_borrow()
                .map_err(|_| "own CG anchor currently borrowed")?;
            owner.verify_default_callback_identities(self.callback.0, self.callback.1)?;
            if owner.guarded_add_remove_identities()? != self.requested {
                return Err("own CG anchor identity changed".into());
            }
            Ok(())
        })();
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("own CG authority changed during anchor guard".into());
        }
        result
    }
    fn error(&self, message: &'static str) {
        if let Ok(mut last) = self.last.try_borrow_mut() {
            *last = Some(message);
        }
    }
    pub fn take_error(&self) -> Result<Option<&'static str>, String> {
        self.guard()?;
        Ok(self
            .last
            .try_borrow_mut()
            .map_err(|_| "CG error borrowed")?
            .take())
    }
    pub fn drain_posts(&self) -> Result<Vec<Posted>, String> {
        self.guard()?;
        Ok(std::mem::take(
            &mut *self
                .posts
                .try_borrow_mut()
                .map_err(|_| "CG posts borrowed")?,
        ))
    }
    pub fn metadata_count(&self) -> Result<usize, String> {
        self.guard()?;
        ENTRIES.with(|e| {
            Ok(e.try_borrow()
                .map_err(|_| "CG metadata borrowed")?
                .values()
                .filter(|e| e.owner == self.nonce)
                .count())
        })
    }
    fn event(&self, pointer: Ptr) -> Option<Rc<Event>> {
        ENTRIES.with(|e| {
            let data = e.try_borrow().ok()?;
            let e = data.get(&(pointer as usize))?;
            if e.owner != self.nonce {
                return None;
            }
            if let Payload::Event(event) = &e.payload {
                Some(event.clone())
            } else {
                None
            }
        })
    }
    fn port(&self, pointer: Ptr) -> Option<Rc<Port>> {
        ENTRIES.with(|e| {
            let data = e.try_borrow().ok()?;
            let e = data.get(&(pointer as usize))?;
            if e.owner != self.nonce {
                return None;
            }
            if let Payload::Port(port) = &e.payload {
                Some(port.clone())
            } else {
                None
            }
        })
    }
    pub fn incoming(&self, data: EventData) -> Result<Ptr, String> {
        self.guard()?;
        if data.text.len() > MAX || self.metadata_count()? > 8190 {
            return Err("own CG event bound".into());
        }
        let object = token(
            1,
            self.nonce,
            Payload::Event(Rc::new(Event {
                data: RefCell::new(data),
                _source: None,
            })),
        )?;
        Ok(object.retained())
    }
    /// # Safety
    /// `event` and optional target port are live owner-thread own factory results.
    /// Root pins every returned source through its supported OwnedSource slot.
    pub unsafe fn inject(&self, event: Ptr, target: Option<Ptr>) -> Result<(), String> {
        unsafe { self.inject_routed(event, target, true) }
    }

    unsafe fn inject_routed(&self, event: Ptr, target: Option<Ptr>, _external: bool) -> Result<(), String> {
        self.guard()?;
        if !is_current(self) {
            return Err("own CG context is not selected".into());
        }
        let e = self.event(event).ok_or("unknown own CG event")?;
        let kind = e.data.try_borrow().map_err(|_| "CG event borrowed")?.kind;
        let force = kind >= 0xfffffffe;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.reference.is_some() && force && target.is_none() {
            return Err("Reference CG disable notification requires its exact target tap".into());
        }
        let taps = self
            .taps
            .try_borrow()
            .map_err(|_| "CG taps borrowed")?
            .clone();
        let mut targets = VecDeque::new();
        for tap in taps.into_iter().filter_map(|p| p.upgrade()) {
            if let Some(target) = target {
                if !self.port(target).is_some_and(|p| Rc::ptr_eq(&p, &tap)) {
                    continue;
                }
            }
            let s = tap.state.try_borrow().map_err(|_| "CG port borrowed")?;
            if !s.valid
                || !s.scheduled
                || (!s.enabled && !force)
                || !tap.accepts_kind(kind, force)
            {
                continue;
            }
            if force {
                drop(s);
                tap.state.borrow_mut().enabled = false;
            }
            let sources = tap
                .sources
                .try_borrow()
                .map_err(|_| "CG sources borrowed")?;
            if let Some(info) = sources.iter().find_map(Weak::upgrade) {
                targets.push_back(Rc::downgrade(&info));
            }
        }
        let event = unsafe { Token::retain(event) }.ok_or("own CG event retain failed")?;
        enqueue(
            self,
            Pending {
                event,
                targets,
                force,
                source_owner: None,
                #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
                reference_source:None,
                #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
                external_mouse: _external && target.is_none() && matches!(kind, 1 | 3),
            },
        )
    }
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
static REFERENCE_ATTEMPT:AtomicBool=AtomicBool::new(false);
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_roster()->BTreeSet<super::SourceCallsite>{
    [("HyperKey.swift","a60dff19c9ab210fd8dc3ae564f962a2708b1e32a2f69663ff2c48e6d70b1544"),
        ("SnippetExpander.swift","0982fd55b9406f9983d40803ada1f97d415251ca0e5908b888b27da7fd697b96")]
        .into_iter().map(|(file,sha)|super::SourceCallsite{
            file:if file=="HyperKey.swift"{"reference-0.8.26/Sources/RuycastApp/HyperKey.swift"}else{"reference-0.8.26/Sources/RuycastApp/SnippetExpander.swift"},
            source_sha256:sha.into(),operation:"CG tap registration, owned event/source and retirement",
        }).collect()
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_current()->Result<Rc<Context>,String>{
    let context=CURRENT.with(|s|s.try_borrow().ok().and_then(|s|s.clone())).ok_or("Reference CG context absent")?;
    if !is_current(&context)||context.reference.is_none(){return Err("Reference CG context is not selected".into());}
    Ok(context)
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) unsafe fn initialize_reference(
    admission:Arc<super::core_preentry::ReferenceAdmission>,context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
)->Result<(Rc<Context>,super::ModuleAttestation),String>{
    if REFERENCE_ATTEMPT.swap(true,Ordering::AcqRel){return Err("Reference CG construction attempt already claimed".into());}
    let module=super::bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;
    admission.verify_input_context(&module)?;
    let reference=ReferenceBinding{admission,context,model:model.clone(),children:std::array::from_fn(|_|OnceLock::new()),module:OnceLock::new()};
    let base=reference.code()?;
    let anchor=OwnedSource::current(model,crate::workspace::cf_loop::SourceMode::Common)?;
    let requested=anchor.guarded_add_remove_identities()?;let callback=anchor.guarded_default_callback_identities()?;
    let main=objc2_core_foundation::CFRunLoop::main().ok_or("Reference main CF loop absent")?;
    if requested.0!=&*main as *const _ as usize||callback.0!=requested.0{return Err("Reference CG anchor is not main loop".into());}
    types()?;
    let nonce=NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_add(1))
        .map_err(|_|"Reference CG identities exhausted")?.checked_add(1).ok_or("Reference CG identity overflow")?;
    let candidate=Rc::new(Context{
        life:module.life.clone(),thread:thread::current().id(),nonce,owner:Rc::new(RefCell::new(anchor)),requested,callback,
        codes:BTreeSet::from([base+0xf990c,base+0x1af5a0]),retired: Arc::new(AtomicBool::new(false)),
        active_callbacks:Cell::new(0),sources:RefCell::new(BTreeMap::new()),
        taps:RefCell::new(vec![]),posts:RefCell::new(vec![]),last:RefCell::new(None),native:None,
        pending_owners:RefCell::new([None,None]),reference:Some(reference),
    });
    install(candidate.clone())?;
    struct Publication{context:Rc<Context>,complete:bool}
    impl Drop for Publication{fn drop(&mut self){if !self.complete{rollback_installation(&self.context);}}}
    let mut publication=Publication{context:candidate.clone(),complete:false};
    // The importer requires a Send+Sync guardian. It carries no Rc or borrowed
    // CF identity across threads; only the exact creator TLS can qualify it.
    struct InstallGuard{nonce:u64,thread:ThreadId,life:Arc<dyn AliveLease>}
    impl AliveLease for InstallGuard{
        fn is_alive(&self)->bool{self.before_effect().is_ok()}
        fn before_effect(&self)->Result<(),String>{
            if self.thread!=thread::current().id(){return Err("Foreign-thread Reference CG import installation".into());}
            self.life.before_effect()?;
            let owner=reference_current()?;
            if owner.nonce!=self.nonce{return Err("Reference CG import owner changed".into());}
            owner.guard()
        }
    }
    let lease=module.lease("reference-events")?;
    // Reference Main has fifteen actual imports in the twenty-function
    // universe. Require the other five to be absent, without optional lookup.
    let specs=import_specs();
    let role=[c"CGEventTapCreate",c"CGEventTapEnable",c"CGEventTapIsEnabled",
        c"CFMachPortInvalidate",c"CFMachPortCreateRunLoopSource",c"CGEventSourceCreate",
        c"CGEventCreateKeyboardEvent",c"CGEventGetFlags",c"CGEventSetFlags",
        c"CGEventGetIntegerValueField",c"CGEventSetIntegerValueField",c"CGEventSetType",
        c"CGEventKeyboardGetUnicodeString",c"CGEventKeyboardSetUnicodeString",c"CGEventPost"];
    let selected=specs.iter().filter(|spec|role.contains(&spec.symbol)).copied().collect::<Vec<_>>();
    if selected.len()!=role.len(){return Err("Reference CG fixed import role differs".into());}
    let imports=super::import_patch::install_exact_presence(&selected,&specs,&InstallGuard{nonce,thread:candidate.thread,life:module.life.clone()})?;
    let selected = Arc::new(ReferencePublication {
        nonce, thread: candidate.thread, retired: candidate.retired.clone(), lease, imports,
    });
    candidate.reference.as_ref().ok_or("Reference CG source absent")?.module.set(selected.clone()).map_err(|_|"Reference CG module already installed")?;
    REFERENCE_PUBLICATION.set(selected.clone()).map_err(|_|"Reference CG attestation already published")?;
    let result=super::ModuleAttestation{module:"reference-events",scopes:BTreeSet::new(),callsites:reference_roster(),
        imports:selected.imports.clone(),objc:vec![],handles:vec![selected.lease.clone()],workspace_factories:None};
    verify_reference_attestation(&result)?;
    publication.complete=true;Ok((candidate,result))
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_attestation(module:&super::ModuleAttestation)->Result<(),String>{
    if module.module!="reference-events"||!module.scopes.is_empty()||module.callsites!=reference_roster()
        ||!module.objc.is_empty()||module.workspace_factories.is_some()||module.handles.len()!=1{return Err("Reference CG module envelope differs".into());}
    // Registry observers share the import record and retirement flag only.
    // The event context and all callback state stay in the owner's TLS.
    let selected = REFERENCE_PUBLICATION.get().ok_or("Reference CG imports absent")?;
    if selected.thread == thread::current().id() {
        let context = reference_current()?;
        context.guard()?;
        let owned = context.reference.as_ref().and_then(|source| source.module.get())
            .ok_or("Reference CG owner has no import record")?;
        if !Arc::ptr_eq(owned, selected) {
            return Err("Reference CG owner import record differs".into());
        }
    }
    let lease = &selected.lease;
    let imports = &selected.imports;
    if module.handles[0].namespace!=lease.namespace||module.handles[0].root.path()!=lease.root.path()
        ||!Arc::ptr_eq(&module.handles[0].life,&lease.life)||module.imports.len()!=imports.len(){return Err("Reference CG module issuer differs".into());}
    lease.before_effect()?;
    let mut symbols=BTreeSet::new();
    for row in &module.imports{
        let expected=imports.iter().find(|r|r.symbol==row.symbol).ok_or("Foreign Reference CG import")?;
        if !symbols.insert(row.symbol)||row.original!=expected.original||row.replacement!=expected.replacement
            ||row.cell_address()!=expected.cell_address()||row.selected_address()!=row.replacement{return Err("Reference CG import/source differs".into());}
    }
    if !PUBLISHED.load(Ordering::Acquire)
        || ACTIVE.load(Ordering::Acquire) != selected.nonce
        || selected.retired.load(Ordering::Acquire)
    {
        return Err("Reference CG publication changed during attestation".into());
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn publish_reference_owner(admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>,
    child:Arc<crate::workspace::factory::ReferenceInputChildCustody>)->Result<(),String>{
    use crate::workspace::factory::ReferenceInputChildKind;
    let context=reference_current()?;
    let result=(||{
        context.guard()?;
        let source=context.reference.as_ref().ok_or("Reference CG source absent")?;
        let role=match child.kind(){ReferenceInputChildKind::HyperKey=>0,ReferenceInputChildKind::SnippetExpander=>1,_=>return Err("Reference CG child role differs".into())};
        if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model)||source.children[role].get().is_some(){return Err("Reference CG publication issuer/replay differs".into());}
        child.verify_reference_binding(admission,model)?;let user=child.pointer_on_creator()?;
        // Copy weak port rows before dynamic source checks. No source callback
        // can run with a taps/model RefCell borrow held.
        let ports=context.taps.try_borrow().map_err(|_|"CG taps borrowed")?.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        let mut matching = Vec::new();
        for port in &ports {
            if source.role(port.callback,None)? == role {
                if port.user != user {
                    return Err("Reference CG pending userdata differs from actual child".into());
                }
                matching.push(port);
            }
        }
        context.guard()?;
        let (held_tap, held_source) = child.event_tap_on_creator()?;
        if !is_current(&context){return Err("Reference CG retired during publication".into());}
        let current=context.taps.try_borrow().map_err(|_|"CG taps borrowed")?.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        if current.len()!=ports.len()||current.iter().zip(&ports).any(|(current,prior)|!Rc::ptr_eq(current,prior)){
            return Err("Reference CG pending registrations changed during publication".into());
        }
        // Hyper and expansion are both enabled in the private launch
        // fixture. A retained but invalidated port cannot prove registration.
        let mut live_ports = 0;
        for port in matching {
            let state = port.state.try_borrow().map_err(|_| "Reference CG pending port borrowed")?;
            if state.valid && state.enabled && state.scheduled && state.enabled_after_schedule {
                let sources = port.sources.try_borrow().map_err(|_| "Reference CG pending sources borrowed")?;
                let mut live = sources.iter().filter_map(Weak::upgrade);
                if !live.next().is_some_and(|info| {
                    info.port.ptr() as usize == held_tap
                        && info.source.get() == held_source
                        && source_is_attached(&info)
                }) || live.next().is_some() {
                    return Err("Reference CG scheduled port or source differs from the retained child".into());
                }
                live_ports += 1;
            }
        }
        if live_ports != 1 {
            return Err("Reference CG requires exactly one enabled registration for this child".into());
        }
        // Only pure identity/epoch checks follow the last dynamic guard. A
        // reentrant newly registered tap cannot escape userdata validation.
        source.children[role].set(child).map_err(|_|"Reference CG child already published")?;
        Ok(())
    })();
    if result.is_err(){context.retired.store(true, Ordering::Release);}
    result
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn observe_reference_tap_stop(
    pointer: usize, source: usize, owner: usize,
) -> Result<impl FnOnce() -> Result<(), String>, String> {
    use objc2_core_foundation::{CFRetained, CFRunLoopSource, CFType};
    let context = reference_current()?;
    context.guard()?;
    let port = context.port(pointer as Ptr).ok_or("Reference HyperKey tap is not owned")?;
    let info = {
        let state = port.state.try_borrow().map_err(|_| "Reference HyperKey tap state borrowed")?;
        let sources = port.sources.try_borrow().map_err(|_| "Reference HyperKey sources borrowed")?;
        let mut live = sources.iter().filter_map(Weak::upgrade);
        let info = live.next().ok_or("Reference HyperKey source is absent")?;
        if context.active_callbacks.get() != 0 || info.entered.get()
            || port.user != owner || !state.valid || !state.enabled || !state.scheduled
            || live.next().is_some() || info.source.get() != source || !source_is_attached(&info)
        {
            return Err("Reference HyperKey tap or source differs before stop".into());
        }
        info
    };
    let held = unsafe { &*(source as *const CFType) }.downcast_ref::<CFRunLoopSource>()
        .ok_or("Reference HyperKey source type differs")?;
    let held = unsafe { CFRetained::retain(std::ptr::NonNull::from(held)) };
    Ok(move || {
        let state = port.state.try_borrow().map_err(|_| "Reference HyperKey tap state borrowed")?;
        let (run_loop, mode) = info.context.requested;
        if state.valid || state.enabled || state.scheduled || info.entered.get()
            || !info.pending.try_borrow().map_err(|_| "Reference HyperKey pending work borrowed")?.is_empty()
            || unsafe { CFRunLoopContainsSource(run_loop as Ptr,
                CFRetained::as_ptr(&held).as_ptr().cast(), mode as *const c_void) } != 0
        {
            return Err("Reference HyperKey tap did not invalidate and drain".into());
        }
        Ok(())
    })
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_keyboard_taps_stopped(
    child: &crate::workspace::factory::ReferenceInputChildCustody, owner: usize,
    before_publication: bool,
) -> Result<(), String> {
    // The caller has just checked live child custody. Keep this drain
    // observation callback-free so it can follow the final guardian.
    let context = reference_current()?;
    let reference = context.reference.as_ref().ok_or("Reference CG source absent")?;
    let same_owner = match reference.children[0].get() {
        Some(held) => !before_publication && std::ptr::eq(held.as_ref(), child),
        None => before_publication,
    };
    if !same_owner
        || context.active_callbacks.get() != 0
    {
        return Err("Reference CG stopped owner differs or is delivering".into());
    }
    let taps = context.taps.try_borrow().map_err(|_| "Reference CG taps borrowed")?;
    for port in taps.iter().filter_map(Weak::upgrade).filter(|port| port.user == owner) {
        let state = port.state.try_borrow().map_err(|_| "Reference CG tap state borrowed")?;
        if state.valid || state.enabled || state.scheduled {
            return Err("Reference HyperKey still has a live CG tap".into());
        }
    }
    let sources = context.sources.try_borrow().map_err(|_| "Reference CG sources borrowed")?;
    for info in sources.values().filter_map(Weak::upgrade).filter(|info| info.model.user == owner) {
        if info.entered.get() || source_is_attached(&info)
            || !info.pending.try_borrow().map_err(|_| "Reference CG pending work borrowed")?.is_empty()
        {
            return Err("Reference HyperKey still has attached or pending CG work".into());
        }
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn retire_reference(admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->Result<(),String>{
    let context = CURRENT.with(|slot| {
        slot.try_borrow().map(|current| current.clone())
            .map_err(|_| "Reference CG teardown owner slot borrowed")
    })?;
    let context = context.ok_or("Reference CG teardown has no installed owner to drain")?;
    let source=context.reference.as_ref().ok_or("CG teardown found another source family")?;
    if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model){return Err("CG teardown found another Reference owner".into());}
    retire(&context)
}
pub fn install(context: Rc<Context>) -> Result<(), String> {
    if let Some(native) = &context.native {
        context.local_guard()?;
        native.installation.verify_candidate(&native.context)?;
    } else { context.guard()?; }
    ACTIVE
        .compare_exchange(0, context.nonce, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "CG owner already installed")?;
    let placed = CURRENT.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| "CG owner borrowed")?;
        if slot.is_some() {
            return Err("CG owner already selected");
        }
        *slot = Some(context.clone());
        Ok(())
    });
    if placed.is_err() {
        let _ = ACTIVE.compare_exchange(context.nonce, 0, Ordering::AcqRel, Ordering::Acquire);
        if let Some(native) = &context.native { native.installation.retire(); }
        return Err("CG owner installation refused".into());
    }
    *ACTIVE_THREAD.lock().map_err(|_| "CG owner thread poisoned")? = Some(context.thread);
    if let Some(native) = &context.native {
        if let Err(error) = native.installation.commit_installed() { rollback_installation(&context); return Err(error); }
    }
    PUBLISHED.store(true, Ordering::Release);
    if let Err(error) = context.guard() { rollback_installation(&context); return Err(error); }
    Ok(())
}
fn rollback_installation(expected: &Rc<Context>) {
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    CURRENT.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if slot.as_ref().is_some_and(|c| Rc::ptr_eq(c, expected)) { PUBLISHED.store(false, Ordering::Release); *slot = None; }
        }
    });
    if ACTIVE.compare_exchange(expected.nonce, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
        if let Ok(mut thread) = ACTIVE_THREAD.lock() { *thread = None; }
    }
}
pub fn retire(expected: &Rc<Context>) -> Result<(), String> {
    if expected.thread != thread::current().id() {
        return Err("foreign-thread CG retirement".into());
    }
    let selected = CURRENT.with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().cloned()));
    if !selected.is_some_and(|c| Rc::ptr_eq(&c, expected)) {
        return Err("stale CG owner".into());
    }
    PUBLISHED.store(false, Ordering::Release);
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    ACTIVE.store(0, Ordering::Release);
    let thread_result = match ACTIVE_THREAD.lock() {
        Ok(mut owner) => {
            *owner = None;
            Ok(())
        }
        Err(poisoned) => {
            *poisoned.into_inner() = None;
            Err("CG owner thread poisoned".to_owned())
        }
    };
    let old = CURRENT.with(|s| s.borrow_mut().take());
    drop(old);
    // A partially initialized tap may never have acquired a run-loop source.
    // Retire its state even if the original caller still retains the port.
    let taps = expected.taps.borrow_mut().drain(..).filter_map(|tap| tap.upgrade()).collect::<Vec<_>>();
    let infos = expected
        .sources
        .borrow()
        .values()
        .filter_map(Weak::upgrade)
        .collect::<Vec<_>>();
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let unresolved = expected.reference.is_some() && (taps.iter().any(|tap| {
        let state = tap.state.borrow();
        state.valid || state.enabled || state.scheduled
    }) || infos.iter().any(|info| source_is_attached(info) || !info.pending.borrow().is_empty()));
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    let unresolved = false;
    for tap in taps {
        tap.state.borrow_mut().invalidate();
    }
    // An executing source can already have disappeared from the weak map.
    let mut busy = expected.active_callbacks.get() != 0;
    for info in infos {
        invalidate_info(&info);
    }
    let owners = expected.pending_owners.borrow_mut().iter_mut().filter_map(Option::take).collect::<Vec<_>>();
    for owner in owners { busy |= owner.revoke().is_err(); }
    thread_result?;
    if busy { return Err("CG callback busy; its owner must remain retained".into()); }
    if unresolved { return Err("Reference CG retired a live tap, attached source or pending event".into()); }
    Ok(())
}
fn is_current(context: &Context) -> bool {
    context.thread == thread::current().id() && PUBLISHED.load(Ordering::Acquire)
        && ACTIVE.load(Ordering::Acquire) == context.nonce
        && !context.retired.load(Ordering::Acquire)
        && CURRENT.with(|s| {
            s.try_borrow()
                .ok()
                .is_some_and(|s| s.as_ref().is_some_and(|c| std::ptr::eq(&**c, context)))
        })
}
fn native_owner(expected: &NativeInputInstallation) -> Result<Rc<Context>, String> {
    expected.pure_after()?; // Includes owner-thread check BEFORE TLS lookup.
    let owner = CURRENT.with(|slot| slot.try_borrow().ok()?.as_ref().cloned()).ok_or("Native CG owner absent")?;
    let native = owner.native.as_ref().ok_or("CG fixture is not Native Input authority")?;
    if !native.installation.matches(expected) { return Err("Native CG installation differs".into()); }
    owner.guard()?;
    expected.pure_after()?;
    if !is_current(&owner) { return Err("Native CG owner changed during verification".into()); }
    Ok(owner)
}
fn owner_index(family: NativeInputOwnerFamily) -> Result<usize, String> {
    match family { NativeInputOwnerFamily::Hid => Ok(0), NativeInputOwnerFamily::Keyword => Ok(1), _ => Err("Native CG owner family differs".into()) }
}
pub(super) fn verify_owner_installation(expected: &NativeInputInstallation) -> Result<(), String> {
    native_owner(expected).map(|_| ())
}
pub(super) fn owner_ticket(expected: &NativeInputInstallation, slot: u64, generation: u64) -> Result<Arc<NativeInputOwnerTicket>, String> {
    let owner = native_owner(expected)?;
    let ticket = owner.pending_owners.try_borrow().map_err(|_| "CG owner tickets borrowed")?.iter().flatten()
        .find(|ticket| ticket.slot() == slot && ticket.generation() == generation).cloned().ok_or("Native CG ticket key absent")?;
    owner.guard()?;
    expected.pure_after()?;
    Ok(ticket)
}
pub(super) fn publish_owner_ticket(expected: &NativeInputInstallation, ticket: Arc<NativeInputOwnerTicket>) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let index = owner_index(ticket.family())?;
    let native = owner.native.as_ref().ok_or("Native CG binding absent")?;
    let witness = if index == 0 { &native.hid } else { &native.keyword };
    if !ticket.matches_callback(witness) { return Err("Native CG ticket/code issuer differs".into()); }
    let entry = ticket.enter_registration()?;
    if entry.userdata()? == 0 { return Err("Native CG userdata absent".into()); }
    {
        let mut slots = owner.pending_owners.try_borrow_mut().map_err(|_| "CG owner tickets borrowed")?;
        if slots[index].is_some() { return Err("Native CG owner already published".into()); }
        slots[index] = Some(ticket.clone());
    }
    let result = owner.guard().and_then(|_| entry.pure_after_callback());
    if result.is_err() {
        let _ = ticket.revoke();
        if let Ok(mut slots) = owner.pending_owners.try_borrow_mut() {
            if slots[index].as_ref().is_some_and(|t| Arc::ptr_eq(t, &ticket)) { slots[index] = None; }
        }
    }
    result
}
pub(super) fn revoke_owner_ticket(expected: &NativeInputInstallation, ticket: &NativeInputOwnerTicket) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let index = owner_index(ticket.family())?;
    let actual = owner.pending_owners.try_borrow().map_err(|_| "CG owner tickets borrowed")?[index].clone().ok_or("Native CG owner absent")?;
    if !std::ptr::eq(&*actual, ticket) { return Err("Native CG ticket identity differs".into()); }
    ticket.revoke()?;
    let mut slots = owner.pending_owners.try_borrow_mut().map_err(|_| "CG owner tickets borrowed")?;
    if slots[index].as_ref().is_some_and(|t| std::ptr::eq(&**t, ticket)) { slots[index] = None; }
    Ok(())
}
fn with<T>(f: impl FnOnce(&Rc<Context>) -> Option<T>) -> Option<T> {
    if !PUBLISHED.load(Ordering::Acquire) {
        return None;
    }
    if ACTIVE_THREAD.lock().ok()?.as_ref() != Some(&thread::current().id()) { return None; }
    let context = CURRENT.with(|s| s.try_borrow().ok()?.as_ref().cloned())?;
    if context.retired.load(Ordering::Acquire) || context.thread != thread::current().id() || !is_current(&context) {
        return None;
    }
    match catch_unwind(AssertUnwindSafe(|| {
        context.guard().ok()?;
        if !is_current(&context) {
            return None;
        }
        f(&context)
    })) {
        Ok(value) => value,
        Err(_) => {
            context.retired.store(true, Ordering::Release);
            None
        }
    }
}

fn invalidate_info(info: &Info) {
    let pending = {
        let mut s = info.model.state.borrow_mut();
        s.invalidate();
        std::mem::take(&mut *info.pending.borrow_mut())
    };
    drop(pending);
    if info.source.get() != 0 {
        unsafe {
            CFRunLoopSourceInvalidate(info.source.get() as Ptr);
        }
    }
}
fn enqueue(context: &Context, mut pending: Pending) -> Result<(), String> {
    context.guard()?;
    if !is_current(context) {
        return Err("CG authority changed before enqueue".into());
    }
    if pending.source_owner.as_ref().is_some_and(|owner| owner.retired()) {
        return Err("Native CG queued source owner retired".into());
    }
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    pending.reference_source_live()?;
    let event = context.event(pending.event.ptr()).ok_or("CG pending event absent")?;
    let kind = event.data.try_borrow().map_err(|_| "CG pending event borrowed")?.kind;
    while let Some(target) = pending.targets.pop_front() {
        let Some(info) = target.upgrade() else {
            continue;
        };
        let s = info
            .model
            .state
            .try_borrow()
            .map_err(|_| "CG port borrowed")?;
        if !s.valid || !s.scheduled || (!s.enabled && !pending.force)
            || !info.model.accepts_kind(kind, pending.force)
        {
            continue;
        }
        if info.model.native_owner.as_ref().is_some_and(|(owner, _)| owner.retired()) { continue; }
        drop(s);
        if info.pending.borrow().len() >= 4096 {
            return Err("own CG pending bound".into());
        }
        let source_owner = pending.source_owner.clone();
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        let reference_source=pending.reference_source.clone();
        info.pending.borrow_mut().push_back(pending);
        unsafe {
            CFRunLoopSourceSignal(info.source.get() as Ptr);
            CFRunLoopWakeUp(context.callback.0 as Ptr);
        }
        if let Some(native) = &context.native { native.installation.pure_after()?; }
        if source_owner.as_ref().is_some_and(|owner| owner.retired()) {
            return Err("Native CG owner retired during queue signal".into());
        }
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some((port,child))=reference_source{
            child.verify_held_epoch()?;
            let state=port.state.try_borrow().map_err(|_|"Reference CG source port borrowed")?;
            if !state.valid||!state.scheduled{return Err("Reference CG source retired during queue signal".into());}
        }
        return Ok(());
    }
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    if pending.external_mouse && matches!(kind, 1 | 3) {
        if let Some(reference) = &context.reference {
            let flags = event.data.try_borrow().map_err(|_| "CG pending event borrowed")?.flags;
            crate::workspace::objc::deliver_reference_external_mouse(&reference.model, &reference.admission, kind, flags)?;
        }
    }
    Ok(())
}
unsafe extern "C" fn info_retain(pointer: *const c_void) -> *const c_void {
    unsafe {
        Rc::increment_strong_count(pointer.cast::<Info>());
    }
    pointer
}
unsafe extern "C" fn info_release(pointer: *const c_void) {
    // The CF source owns this live Info reference until this callback returns.
    let info = unsafe { &*pointer.cast::<Info>() };
    let nonce = info.context.nonce;
    let result = catch_unwind(AssertUnwindSafe(|| {
        let info = unsafe { Rc::from_raw(pointer.cast::<Info>()) };
        // A temporary Rust owner of Info can outlive its CF source.
        let source = info.source.replace(0);
        if source != 0 {
            info.context.sources.borrow_mut().remove(&source);
            info.model.sources.borrow_mut().retain(|entry| !std::ptr::eq(entry.as_ptr(), Rc::as_ptr(&info)));
        }
    }));
    if let Err(payload) = result {
        retire_after_cleanup_panic(Some(nonce));
        std::mem::forget(payload);
    }
}
#[cfg(any(test, all(rcf1_reference_sdk, target_arch="aarch64")))]
fn source_is_attached(info: &Info) -> bool {
    let source = info.source.get();
    let (run_loop, mode) = info.context.requested;
    source != 0 && unsafe { CFRunLoopContainsSource(run_loop as Ptr, source as Ptr, mode as *const c_void) != 0 }
}
unsafe extern "C" fn scheduled(pointer: Ptr, loop_: Ptr, mode: *const c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let info = unsafe { &*pointer.cast::<Info>() };
        let valid = info.context.local_guard().is_ok()
            && is_current(&info.context)
            && (loop_ as usize, mode as usize) == info.context.callback;
        if !valid {
            info.context.error("unqualified CG source schedule");
            return;
        }
        let mut s = info.model.state.borrow_mut();
        if s.valid {
            s.scheduled = true;
        }
    }));
}
unsafe extern "C" fn cancelled(pointer: Ptr, loop_: Ptr, mode: *const c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let info = unsafe { &*pointer.cast::<Info>() };
        if info.context.thread != thread::current().id() {
            return;
        }
        if (loop_ as usize, mode as usize) != info.context.callback {
            info.context.error("unqualified CG source cancel")
        }
        let mut state = info.model.state.borrow_mut();
        state.scheduled = false;
        state.enabled_after_schedule = false;
    }));
}
unsafe extern "C" fn perform(pointer: Ptr) {
    let on_owner = ACTIVE_THREAD.lock().ok().is_some_and(|owner| {
        *owner == Some(thread::current().id())
    });
    if !on_owner || pointer.is_null() {
        return;
    }
    // Reentrant retirement may invalidate the source and release CF's
    // context reference. Keep the exact Info alive through this turn.
    let info = unsafe {
        Rc::increment_strong_count(pointer.cast::<Info>());
        Rc::from_raw(pointer.cast::<Info>())
    };
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| perform_owned(&info))) {
        info.context.retired.store(true, Ordering::Release);
        info.context.error("own CG source callback panicked");
        std::mem::forget(payload);
    }
    let nonce = info.context.nonce;
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(info))) {
        retire_after_cleanup_panic(Some(nonce));
        std::mem::forget(payload);
    }
}
fn perform_owned(info: &Info) {
    if info.entered.replace(true) {
        info.deferred.set(true);
        return;
    }
    struct Entry<'a> {
        entered: &'a Cell<bool>,
        active_callbacks: &'a Cell<usize>,
    }
    impl Drop for Entry<'_> {
        fn drop(&mut self) {
            self.entered.set(false);
            self.active_callbacks.set(self.active_callbacks.get() - 1);
        }
    }
    let active_callbacks = &info.context.active_callbacks;
    active_callbacks.set(active_callbacks.get() + 1);
    let _entry = Entry { entered: &info.entered, active_callbacks };
    let result = (|| -> Result<(), String> {
        info.context.guard()?;
        if !is_current(&info.context) {
            return Err("retired CG source dispatch".into());
        }
        let pending = info.pending.borrow_mut().pop_front();
        if let Some(mut pending) = pending {
            if pending.source_owner.as_ref().is_some_and(|owner| owner.retired()) {
                return Err("Native CG queued source authorization retired".into());
            }
            let valid = {
                let s = info.model.state.borrow();
                s.valid && s.scheduled && (s.enabled || pending.force)
            };
            if valid {
                let event = info
                    .context
                    .event(pending.event.ptr())
                    .ok_or("CG pending event absent")?;
                let delivery_kind = || -> Result<u32, String> {
                    let kind = event.data.try_borrow().map_err(|_| "CG pending event borrowed")?.kind;
                    if !info.model.accepts_kind(kind, pending.force) {
                        return Err("CG event kind no longer matches the selected tap".into());
                    }
                    Ok(kind)
                };
                #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
                if let Some(reference)=&info.context.reference {
                    pending.reference_source_live()?;
                    let child=reference.owner(info.model.callback,info.model.user)?;
                    let live=||->Result<(),String>{
                        if child.event_tap_on_creator()? != (info.port.ptr() as usize, info.source.get()) {
                            return Err("Reference Swift event tap or source no longer matches the delivery".into());
                        }
                        pending.reference_source_live()?;
                        child.verify_held_epoch()?;
                        if !is_current(&info.context){return Err("Reference CG owner retired".into());}
                        if !source_is_attached(info) {
                            return Err("Reference CG source detached before delivery".into());
                        }
                        let state=info.model.state.try_borrow().map_err(|_|"CG port borrowed")?;
                        if !state.valid||!state.scheduled||(!state.enabled&& !pending.force){return Err("Reference CG port retired".into());}
                        Ok(())
                    };
                    info.context.guard()?;
                    reference.owner(info.model.callback,info.model.user)?;
                    live()?;
                    let kind = delivery_kind()?;
                    let interrupted_keyboard = pending.force
                        && child.kind() == crate::workspace::factory::ReferenceInputChildKind::HyperKey;
                    let held_source = if interrupted_keyboard {
                        use objc2_core_foundation::{CFRunLoopSource, CFRetained, CFType};
                        let source = unsafe { &*(info.source.get() as *const CFType) }
                            .downcast_ref::<CFRunLoopSource>().ok_or("Reference interrupted CG source type differs")?;
                        Some(unsafe { CFRetained::retain(std::ptr::NonNull::from(source)) })
                    } else {
                        None
                    };
                    super::reference_runtime::verify_published_input_child(&reference.admission, &child)?;
                    let output=unsafe{(info.model.callback)(info.port.ptr(),kind,pending.event.ptr(),info.model.user as Ptr)};
                    if let Some(source) = held_source {
                        // The original HyperKey callback stops this exact tap
                        // on either forced disable notification.
                        info.context.guard()?;
                        child.verify_keyboard_tap_cleared()?;
                        let (run_loop, mode) = info.context.requested;
                        let attached = unsafe { CFRunLoopContainsSource(run_loop as Ptr,
                            objc2_core_foundation::CFRetained::as_ptr(&source).as_ptr().cast(),
                            mode as *const c_void) } != 0;
                        {
                            let state = info.model.state.try_borrow().map_err(|_| "CG port borrowed")?;
                            if output != pending.event.ptr() || state.valid || state.enabled || state.scheduled
                                || attached || !info.pending.try_borrow().map_err(|_| "CG pending work borrowed")?.is_empty()
                            {
                                return Err("Reference interrupted HyperKey tap did not stop".into());
                            }
                        }
                        info.deferred.set(false);
                        drop(source);
                        drop(pending);
                        drop(event);
                        if !is_current(&info.context) {
                            return Err("Reference CG retired during interrupted tap cleanup".into());
                        }
                        child.verify_held_epoch()?;
                        super::reference_runtime::verify_published_input_child(&reference.admission, &child)?;
                        return Ok(());
                    }
                    info.context.guard()?;live()?;
                    super::reference_runtime::verify_published_input_child(&reference.admission, &child)?;
                    if !output.is_null(){
                        if output!=pending.event.ptr(){return Err("Unqualified Reference CG replacement event".into());}
                        pending.force=false;
                        pending.reference_source=Some((info.model.clone(),child.clone()));
                        enqueue(&info.context,pending)?;
                    }else{drop(pending);}
                    drop(event);
                    if !is_current(&info.context){return Err("Reference CG retired during queue cleanup".into());}
                    if info.deferred.replace(false)||!info.pending.borrow().is_empty(){unsafe{
                        CFRunLoopSourceSignal(info.source.get() as Ptr);CFRunLoopWakeUp(info.context.callback.0 as Ptr);
                    }}
                    child.verify_held_epoch()?;
                    if !is_current(&info.context){return Err("Reference CG retired during queue signal".into());}
                    return Ok(());
                }
                if let Some(native) = &info.context.native {
                    let (owner, witness) = info.model.native_owner.as_ref().ok_or("Native CG port has no owner ticket")?;
                    if !owner.matches_callback(witness) { return Err("Native CG callback owner/code differs".into()); }
                    let entry = owner.enter()?;
                    if entry.userdata()? != info.model.user { return Err("Native CG userdata differs from live owner".into()); }
                    let guard = witness.before_callback(&native.context)?;
                    native.installation.pure_after()?;
                    if !is_current(&info.context) { return Err("Native CG owner retired before callback".into()); }
                    {
                        let state = info.model.state.try_borrow().map_err(|_| "CG port borrowed")?;
                        if !state.valid || !state.scheduled || (!state.enabled && !pending.force) {
                            return Err("Native CG port retired before callback".into());
                        }
                    }
                    if pending.source_owner.as_ref().is_some_and(|source| source.retired()) {
                        return Err("Native CG queued source retired during callback gate".into());
                    }
                    if entry.userdata()? != info.model.user { return Err("Native CG userdata retired before callback".into()); }
                    let kind = delivery_kind()?;
                    let output = unsafe { (info.model.callback)(info.port.ptr(), kind, pending.event.ptr(), info.model.user as Ptr) };
                    info.context.guard()?;
                    guard.pure_after_callback()?;
                    entry.pure_after_callback()?;
                    native.installation.pure_after()?;
                    if !is_current(&info.context) { return Err("Native CG authority retired during callback".into()); }
                    if pending.source_owner.as_ref().is_some_and(|source| source.retired()) {
                        return Err("Native CG queued source retired during callback".into());
                    }
                    if !output.is_null() {
                        if output != pending.event.ptr() { return Err("unqualified Native CG replacement event ownership".into()); }
                        pending.force = false;
                        pending.source_owner = Some(owner.clone());
                        enqueue(&info.context, pending)?;
                    } else { drop(pending); }
                    drop(event);
                    if !info.context.retired.load(Ordering::Acquire) && (info.deferred.replace(false) || !info.pending.borrow().is_empty()) {
                        unsafe { CFRunLoopSourceSignal(info.source.get() as Ptr); CFRunLoopWakeUp(info.context.callback.0 as Ptr); }
                    }
                    // All guardian, CF cleanup, queue and signal callbacks are
                    // complete before releasing the Native userdata entry.
                    guard.pure_after_callback()?;
                    entry.pure_after_callback()?;
                    native.installation.pure_after()?;
                    if !is_current(&info.context) { return Err("Native CG authority retired after queue signal".into()); }
                    return Ok(());
                }
                let kind = delivery_kind()?;
                let output = unsafe {
                    (info.model.callback)(
                        info.port.ptr(),
                        kind,
                        pending.event.ptr(),
                        info.model.user as Ptr,
                    )
                };
                info.context.guard()?;
                if !is_current(&info.context) {
                    return Err("CG authority retired during callback".into());
                }
                if !output.is_null() {
                    if output != pending.event.ptr() {
                        return Err("unqualified CG replacement event ownership".into());
                    }
                    pending.force = false;
                    enqueue(&info.context, pending)?;
                }
            }
        }
        Ok(())
    })();
    if result.is_err() {
        info.context.error("own CG dispatch refused; no retry")
    }
    if info.context.native.is_none() && !info.context.retired.load(Ordering::Acquire)
        && (info.deferred.replace(false) || !info.pending.borrow().is_empty())
    {
        unsafe {
            CFRunLoopSourceSignal(info.source.get() as Ptr);
            CFRunLoopWakeUp(info.context.callback.0 as Ptr);
        }
    }
}
unsafe extern "C" fn tap_create_checked(
    location: u32,
    place: u32,
    options: u32,
    mask: u64,
    callback: Option<Callback>,
    user: Ptr,
    return_pc:usize,
) -> Ptr {
    with(|c| {
        let callback = callback?;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&c.reference{
            let role=match reference.role(callback,Some(return_pc)){Ok(role)=>role,Err(_)=>{c.retired.store(true, Ordering::Release);return None;}};
            // HyperKey listens to down/up/flags, three mouse buttons and
            // scroll. SnippetExpander listens to key down and two buttons.
            const HYPER_MASK: u64 = (1 << 10) | (1 << 11) | (1 << 12) | (1 << 1)
                | (1 << 3) | (1 << 25) | (1 << 22);
            const SNIPPET_MASK: u64 = (1 << 10) | (1 << 1) | (1 << 3);
            let expected_mask = [HYPER_MASK, SNIPPET_MASK][role];
            if (location, place, options, mask) != (1, 0, 0, expected_mask) || user.is_null() {
                c.retired.store(true, Ordering::Release);
                return None;
            }
            if reference.children[role].get().is_some()&&reference.owner(callback,user as usize).is_err(){c.retired.store(true, Ordering::Release);return None;}
            if !is_current(c){return None;}
        }
        let _=return_pc;
        if location != 1
            || place > 1
            || options != 0
            || mask == 0
            || !c.codes.contains(&(callback as usize))
            || c.metadata_count().ok()? > 8190
        {
            return None;
        }
        let native_owner = if let Some(native) = &c.native {
            let (index, witness) = if callback as usize == native.hid.event_handler().ok()? as usize {
                (0, native.hid.clone())
            } else if callback as usize == native.keyword.event_handler().ok()? as usize {
                (1, native.keyword.clone())
            } else { return None; };
            let ticket = c.pending_owners.try_borrow().ok()?[index].as_ref()?.clone();
            if !ticket.matches_callback(&witness) { return None; }
            let entry = ticket.enter().ok()?;
            if entry.userdata().ok()? != user as usize { return None; }
            native.installation.pure_after().ok()?;
            entry.pure_after_callback().ok()?;
            Some((ticket, witness))
        } else { None };
        if !is_current(c){return None;}
        let port = Rc::new(Port {
            state: RefCell::new(PortState {
                valid: true,
                enabled: true,
                scheduled: false,
                enabled_after_schedule: false,
            }),
            callback,
            user: user as usize,
            mask,
            sources: RefCell::new(vec![]),
            native_owner,
        });
        let object = token(2, c.nonce, Payload::Port(port.clone())).ok()?;
        // Own CF allocation may have invoked runtime callbacks. Publication
        // follows a fresh pure selected-owner check, without a dynamic guard.
        if !is_current(c){return None;}
        let mut taps = c.taps.borrow_mut();
        taps.retain(|p| p.strong_count() != 0);
        if place == 0 {
            taps.insert(0, Rc::downgrade(&port));
        } else {
            taps.push(Rc::downgrade(&port));
        }
        Some(object.retained())
    })
    .unwrap_or(std::ptr::null_mut())
}
#[cfg(target_arch="aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn tap_create(_location:u32,_place:u32,_options:u32,_mask:u64,_callback:Option<Callback>,_user:Ptr)->Ptr {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!","mov x29,sp","mov x6,x30","bl {checked}","ldp x29,x30,[sp],#16","ret",checked=sym tap_create_checked);
}
#[cfg(not(target_arch="aarch64"))]
unsafe extern "C" fn tap_create(location:u32,place:u32,options:u32,mask:u64,callback:Option<Callback>,user:Ptr)->Ptr {
    unsafe{tap_create_checked(location,place,options,mask,callback,user,0)}
}
unsafe extern "C" fn tap_enable(port: Ptr, enable: bool) {
    let _ = with(|c| {
        let p = c.port(port)?;
        let mut s = p.state.try_borrow_mut().ok()?;
        if s.valid {
            s.enabled = enable;
            s.enabled_after_schedule = enable && s.scheduled;
        }
        Some(())
    });
}
unsafe extern "C" fn tap_enabled(port: Ptr) -> bool {
    with(|c| {
        let p = c.port(port)?;
        let s = p.state.try_borrow().ok()?;
        Some(s.valid && s.enabled)
    })
    .unwrap_or(false)
}
unsafe extern "C" fn port_valid(port: Ptr) -> u8 {
    with(|c| Some(c.port(port)?.state.try_borrow().ok()?.valid.into())).unwrap_or(0)
}
unsafe extern "C" fn port_invalidate(port: Ptr) {
    let _ = with(|c| {
        let p = c.port(port)?;
        p.state.borrow_mut().invalidate();
        let sources = p
            .sources
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for info in sources {
            invalidate_info(&info);
        }
        Some(())
    });
}
unsafe extern "C" fn source_create(allocator: *const c_void, port: Ptr, order: isize) -> Ptr {
    with(|c| {
        if !allocator.is_null() || order != 0 {
            return None;
        }
        let model = c.port(port)?;
        if !model.state.borrow().valid {
            return None;
        }
        if let Some(info) = model.sources.borrow().iter().filter_map(Weak::upgrade).find(|info| info.source.get() != 0) {
            return Some(unsafe { CFRetain(info.source.get() as Ptr).cast_mut() });
        }
        let port = unsafe { Token::retain(port) }?;
        let info = Rc::new(Info {
            context: c.clone(),
            port,
            model: model.clone(),
            source: Cell::new(0),
            pending: RefCell::new(VecDeque::new()),
            entered: Cell::new(false),
            deferred: Cell::new(false),
        });
        let raw = Rc::into_raw(info);
        let mut context = SourceContext {
            version: 0,
            info: raw.cast_mut().cast(),
            retain: Some(info_retain),
            release: Some(info_release),
            description: 0,
            equal: 0,
            hash: 0,
            schedule: Some(scheduled),
            cancel: Some(cancelled),
            perform: Some(perform),
        };
        let source = unsafe { CFRunLoopSourceCreate(std::ptr::null(), 0, &mut context) };
        let info = unsafe { Rc::from_raw(raw) };
        if source.is_null() {
            return None;
        }
        info.source.set(source as usize);
        c.sources
            .borrow_mut()
            .insert(source as usize, Rc::downgrade(&info));
        model.sources.borrow_mut().push(Rc::downgrade(&info));
        Some(source)
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn event_source(state: i32) -> Ptr {
    with(|c| {
        if !matches!(state, -1..=1) {
            return None;
        }
        Some(token(0, c.nonce, Payload::Source(state)).ok()?.retained())
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn keyboard(source: Ptr, key: u16, down: bool) -> Ptr {
    with(|c| {
        let source = if source.is_null() {
            None
        } else {
            let valid = ENTRIES.with(|e| {
                e.try_borrow().ok().is_some_and(|e| {
                    e.get(&(source as usize)).is_some_and(|e| {
                        e.owner == c.nonce && matches!(e.payload, Payload::Source(-1..=1))
                    })
                })
            });
            if !valid {
                return None;
            }
            Some(unsafe { Token::retain(source) }?)
        };
        let event = EventData {
            kind: if down { 10 } else { 11 },
            key: key.into(),
            ..EventData::default()
        };
        Some(
            token(
                1,
                c.nonce,
                Payload::Event(Rc::new(Event {
                    data: RefCell::new(event),
                    _source: source,
                })),
            )
            .ok()?
            .retained(),
        )
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn flags(event: Ptr) -> u64 {
    with(|c| Some(c.event(event)?.data.try_borrow().ok()?.flags)).unwrap_or(0)
}
unsafe extern "C" fn set_flags(event: Ptr, value: u64) {
    let _ = with(|c| {
        c.event(event)?.data.try_borrow_mut().ok()?.flags = value;
        Some(())
    });
}
unsafe extern "C" fn field(event: Ptr, field: u32) -> i64 {
    with(|c| {
        let event = c.event(event)?;
        let data = event.data.try_borrow().ok()?;
        Some(match field {
            8 => data.repeat,
            9 => data.key,
            42 => data.tag,
            _ => 0,
        })
    })
    .unwrap_or(0)
}
unsafe extern "C" fn set_field(event: Ptr, field: u32, value: i64) {
    let _ = with(|c| {
        let event = c.event(event)?;
        let mut data = event.data.try_borrow_mut().ok()?;
        match field {
            8 => data.repeat = value,
            9 => data.key = value,
            42 => data.tag = value,
            _ => c.error("unqualified own CG integer field"),
        };
        Some(())
    });
}
unsafe extern "C" fn kind(event: Ptr) -> u32 {
    with(|c| Some(c.event(event)?.data.try_borrow().ok()?.kind)).unwrap_or(0)
}
unsafe extern "C" fn set_kind(event: Ptr, value: u32) {
    let _ = with(|c| {
        c.event(event)?.data.try_borrow_mut().ok()?.kind = value;
        Some(())
    });
}
unsafe extern "C" fn unicode(event: Ptr, max: usize, actual: *mut usize, out: *mut u16) {
    let _ = with(|c| {
        let event = c.event(event)?;
        let data = event.data.try_borrow().ok()?;
        if !actual.is_null() {
            unsafe {
                actual.write(data.text.len());
            }
        }
        if max != 0 && !out.is_null() {
            let count = max.min(data.text.len());
            unsafe {
                std::ptr::copy_nonoverlapping(data.text.as_ptr(), out, count);
            }
        }
        Some(())
    });
}
unsafe extern "C" fn set_unicode(event: Ptr, count: usize, input: *const u16) {
    let _ = with(|c| {
        if count > MAX || (count > 0 && input.is_null()) {
            c.error("own CG unicode bound");
            return None;
        }
        let event = c.event(event)?;
        let text = if count == 0 {
            vec![]
        } else {
            unsafe { std::slice::from_raw_parts(input, count) }.to_vec()
        };
        event.data.try_borrow_mut().ok()?.text = text;
        Some(())
    });
}
unsafe extern "C" fn post(location: u32, event: Ptr) {
    let _ = with(|c| {
        if location > 1 {
            c.error("unqualified own CG post location");
            return None;
        }
        let e = c.event(event)?;
        let data = e.data.try_borrow().ok()?.clone();
        if c.posts.borrow().len() >= 4096 {
            c.error("own CG post bound");
            return None;
        }
        c.posts.borrow_mut().push(Posted {
            location,
            event: data.clone(),
        });
        let snapshot = c.incoming(data).ok()?;
        let object = Token(NonNull::new(snapshot)?);
        let result = unsafe { c.inject_routed(object.ptr(), None, false) };
        if result.is_err() {
            c.error("own CG post enqueue refused")
        }
        Some(())
    });
}
fn type_(index: usize) -> usize {
    with(|_| Some(cached()?[index])).unwrap_or(0)
}
unsafe extern "C" fn source_type() -> usize {
    type_(0)
}
unsafe extern "C" fn event_type() -> usize {
    type_(1)
}
unsafe extern "C" fn port_type() -> usize {
    type_(2)
}
pub fn replacements() -> [(&'static CStr, usize); 20] {
    [
        (c"CGEventTapCreate", tap_create as *const () as usize),
        (c"CGEventTapEnable", tap_enable as *const () as usize),
        (c"CGEventTapIsEnabled", tap_enabled as *const () as usize),
        (
            c"CFMachPortInvalidate",
            port_invalidate as *const () as usize,
        ),
        (c"CFMachPortIsValid", port_valid as *const () as usize),
        (
            c"CFMachPortCreateRunLoopSource",
            source_create as *const () as usize,
        ),
        (c"CGEventSourceCreate", event_source as *const () as usize),
        (
            c"CGEventCreateKeyboardEvent",
            keyboard as *const () as usize,
        ),
        (c"CGEventGetFlags", flags as *const () as usize),
        (c"CGEventSetFlags", set_flags as *const () as usize),
        (c"CGEventGetIntegerValueField", field as *const () as usize),
        (
            c"CGEventSetIntegerValueField",
            set_field as *const () as usize,
        ),
        (c"CGEventGetType", kind as *const () as usize),
        (c"CGEventSetType", set_kind as *const () as usize),
        (
            c"CGEventKeyboardGetUnicodeString",
            unicode as *const () as usize,
        ),
        (
            c"CGEventKeyboardSetUnicodeString",
            set_unicode as *const () as usize,
        ),
        (c"CGEventPost", post as *const () as usize),
        (c"CGEventSourceGetTypeID", source_type as *const () as usize),
        (c"CGEventGetTypeID", event_type as *const () as usize),
        (c"CFMachPortGetTypeID", port_type as *const () as usize),
    ]
}
/// Fixed typed system entries; these addresses never post/create a real event
/// or tap. Only the existing source-qualified import transaction may select them.
pub fn import_specs() -> [super::ImportSpec; 20] {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventTapCreate(location: u32, place: u32, options: u32, mask: u64, callback: Option<Callback>, user: Ptr) -> Ptr;
        fn CGEventTapEnable(port: Ptr, enable: bool);
        fn CGEventTapIsEnabled(port: Ptr) -> bool;
        fn CGEventSourceCreate(state: i32) -> Ptr;
        fn CGEventCreateKeyboardEvent(source: Ptr, key: u16, down: bool) -> Ptr;
        fn CGEventGetFlags(event: Ptr) -> u64;
        fn CGEventSetFlags(event: Ptr, flags: u64);
        fn CGEventGetIntegerValueField(event: Ptr, field: u32) -> i64;
        fn CGEventSetIntegerValueField(event: Ptr, field: u32, value: i64);
        fn CGEventGetType(event: Ptr) -> u32;
        fn CGEventSetType(event: Ptr, kind: u32);
        fn CGEventKeyboardGetUnicodeString(event: Ptr, capacity: usize, actual: *mut usize, output: *mut u16);
        fn CGEventKeyboardSetUnicodeString(event: Ptr, count: usize, input: *const u16);
        fn CGEventPost(location: u32, event: Ptr);
        fn CGEventSourceGetTypeID() -> usize;
        fn CGEventGetTypeID() -> usize;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFMachPortInvalidate(port: Ptr);
        fn CFMachPortIsValid(port: Ptr) -> u8;
        fn CFMachPortCreateRunLoopSource(allocator: *const c_void, port: Ptr, order: isize) -> Ptr;
        fn CFMachPortGetTypeID() -> usize;
    }
    let original = [CGEventTapCreate as *const () as usize, CGEventTapEnable as *const () as usize,
        CGEventTapIsEnabled as *const () as usize, CFMachPortInvalidate as *const () as usize,
        CFMachPortIsValid as *const () as usize, CFMachPortCreateRunLoopSource as *const () as usize,
        CGEventSourceCreate as *const () as usize, CGEventCreateKeyboardEvent as *const () as usize,
        CGEventGetFlags as *const () as usize, CGEventSetFlags as *const () as usize,
        CGEventGetIntegerValueField as *const () as usize, CGEventSetIntegerValueField as *const () as usize,
        CGEventGetType as *const () as usize, CGEventSetType as *const () as usize,
        CGEventKeyboardGetUnicodeString as *const () as usize, CGEventKeyboardSetUnicodeString as *const () as usize,
        CGEventPost as *const () as usize, CGEventSourceGetTypeID as *const () as usize,
        CGEventGetTypeID as *const () as usize, CFMachPortGetTypeID as *const () as usize];
    let leaves = replacements();
    std::array::from_fn(|i| super::ImportSpec { symbol: leaves[i].0, original: original[i], replacement: leaves[i].1 })
}
/// Read-only selected import cells through the genuine issuer, not callback,
/// event/window, source-code or scope authority. No Context installation, OS
/// Post, actual tap or CG-to-NSEvent pointer conversion occurs here.
pub fn observe_owned_imports(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
) -> Result<(super::owned::OwnedHandleLease, Vec<super::ImportReplacement>), String> {
    let context = super::bootstrap::owned_module_context(life, descriptors)?;
    let lease = context.lease("input-events")?;
    let imports = super::observe_main_imports(&import_specs())?;
    let confirmed = super::bootstrap::owned_module_context(life, descriptors)?;
    if !Arc::ptr_eq(&lease.life, &confirmed.life) { return Err("CG import issuer changed".into()); }
    lease.before_effect()?;
    if imports.iter().any(|row| row.selected_address() != row.replacement) {
        return Err("CG selected import changed".into());
    }
    Ok((lease, imports))
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
        let specs = import_specs();
        let leaves = replacements();
        assert_eq!(specs.len(), 20);
        assert_eq!(specs.iter().map(|s| s.symbol).collect::<std::collections::BTreeSet<_>>().len(), 20);
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
    #[test]
    fn six_argument_c_entry_and_owned_cf_sources_forward_then_retire() {
        // Attach only to this test thread's loop. Drive perform explicitly so
        // the test never pumps desktop or unrelated run-loop events.
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRunLoopAddSource(loop_: Ptr, source: Ptr, mode: *const c_void);
        }
        let fixture=crate::workspace::tests::Fixture::new();
        let anchor=Rc::new(RefCell::new(OwnedSource::current(fixture.model.clone(),crate::workspace::cf_loop::SourceMode::Common).unwrap()));
        unsafe extern "C" fn callback(port:Ptr,kind:u32,event:Ptr,user:Ptr)->Ptr{
            let observed=unsafe{&*(user as *const AtomicU64)};
            if kind==10&&unsafe{CFGetTypeID(port)}==cached().unwrap()[2]
                &&unsafe{CFGetTypeID(event)}==cached().unwrap()[1]{observed.fetch_add(1,Ordering::AcqRel);}
            event
        }
        let context=Context::new(fixture.life.clone(),anchor,BTreeSet::from([callback as *const () as usize])).unwrap();
        install(context.clone()).unwrap();
        let first=AtomicU64::new(0);let second=AtomicU64::new(0);
        let event=context.incoming(EventData{kind:10,flags:0,key:57,repeat:0,tag:0,text:vec![97]}).unwrap();
        let a=unsafe{tap_create(1,0,0,1<<10,Some(callback),std::ptr::from_ref(&first).cast_mut().cast())};
        let b=unsafe{tap_create(1,1,0,1<<10,Some(callback),std::ptr::from_ref(&second).cast_mut().cast())};
        assert!(!a.is_null()&&!b.is_null());
        assert!(unsafe{tap_create(0,0,0,1<<10,Some(callback),std::ptr::null_mut())}.is_null());
        let sources=[unsafe{source_create(std::ptr::null(),a,0)},unsafe{source_create(std::ptr::null(),b,0)}];
        assert!(sources.iter().all(|source|!source.is_null()));
        let infos=sources.map(|source|context.sources.borrow().get(&(source as usize)).unwrap().upgrade().unwrap());
        assert!(infos.iter().all(|info| !source_is_attached(info)));
        for source in sources {
            unsafe { CFRunLoopAddSource(context.requested.0 as Ptr, source, context.requested.1 as *const c_void); }
        }
        assert!(infos.iter().all(|info| source_is_attached(info) && info.model.state.borrow().scheduled));
        assert!(infos.iter().all(|info| !info.model.state.borrow().enabled_after_schedule));
        for port in [a, b] {
            unsafe { tap_enable(port, true) };
        }
        assert!(infos.iter().all(|info| info.model.state.borrow().enabled_after_schedule));
        unsafe{context.inject(event,None).unwrap();}
        for info in &infos{unsafe{perform(Rc::as_ptr(info).cast_mut().cast());}}
        assert_eq!(first.load(Ordering::Acquire),1);
        assert_eq!(second.load(Ordering::Acquire),1);
        assert!(context.take_error().unwrap().is_none());
        assert!(infos.iter().all(|info|info.pending.borrow().is_empty()));
        // A mutable event may change after source scheduling. Neither tap
        // registered for flagsChanged, so the queued keyDown cannot dispatch.
        unsafe { context.inject(event, None).unwrap(); set_kind(event, 12); }
        for info in &infos { unsafe { perform(Rc::as_ptr(info).cast_mut().cast()); } }
        assert_eq!(first.load(Ordering::Acquire), 1);
        assert_eq!(second.load(Ordering::Acquire), 1);
        assert!(context.take_error().unwrap().is_some());
        unsafe { set_kind(event, 10); }
        unsafe{context.inject(event,None).unwrap();}
        assert_eq!(infos[0].pending.borrow().len(),1);
        unsafe{port_invalidate(a);port_invalidate(b);}
        assert!(infos.iter().all(|info| !source_is_attached(info)));
        assert!(infos.iter().all(|info| !info.model.state.borrow().enabled_after_schedule));
        // Late delivery from an invalidated source cannot invoke either user.
        for info in &infos{unsafe{perform(Rc::as_ptr(info).cast_mut().cast());}}
        assert_eq!(first.load(Ordering::Acquire),1);
        assert_eq!(second.load(Ordering::Acquire),1);
        unsafe{for source in sources{CFRelease(source);}CFRelease(event);CFRelease(a);CFRelease(b);}
        assert!(infos.iter().all(|info| info.source.get() == 0));
        assert!(context.sources.borrow().is_empty());
        assert!(infos.iter().all(|info| info.model.sources.borrow().is_empty()));
        drop(infos);
        assert_eq!(context.metadata_count().unwrap(),0);
        let unmounted = unsafe { tap_create(1, 0, 0, 1 << 10, Some(callback),
            std::ptr::from_ref(&first).cast_mut().cast()) };
        let model = context.port(unmounted).expect("owned tap without a source");
        assert!(model.state.borrow().valid);
        retire(&context).unwrap();
        assert!(!model.state.borrow().valid);
        assert!(!model.state.borrow().enabled);
        assert!(context.taps.borrow().is_empty());
        unsafe { CFRelease(unmounted); }
        assert!(context.guard().is_err());
        assert!(unsafe{tap_create(1,0,0,1<<10,Some(callback),std::ptr::null_mut())}.is_null());

        struct RetirementProbe {
            source: Cell<Ptr>,
            busy_and_retained: AtomicBool,
        }
        unsafe extern "C" fn retire_inside_tap(port: Ptr, _: u32, event: Ptr, user: Ptr) -> Ptr {
            let probe = unsafe { &*user.cast::<RetirementProbe>() };
            unsafe {
                port_invalidate(port);
                CFRelease(probe.source.replace(std::ptr::null_mut()));
            }
            let owner = CURRENT.with(|slot| slot.borrow().clone());
            let busy = owner.is_some_and(|owner| owner.sources.borrow().is_empty() && retire(&owner).is_err());
            let retained = cached().is_some_and(|ids| unsafe {
                CFGetTypeID(port) == ids[2] && CFGetTypeID(event) == ids[1]
            });
            probe.busy_and_retained.store(busy && retained, Ordering::Release);
            event
        }
        let anchor = Rc::new(RefCell::new(OwnedSource::current(
            fixture.model.clone(), crate::workspace::cf_loop::SourceMode::Default,
        ).unwrap()));
        let context = Context::new(fixture.life.clone(), anchor,
            BTreeSet::from([retire_inside_tap as *const () as usize])).unwrap();
        install(context.clone()).unwrap();
        let probe = RetirementProbe {
            source: Cell::new(std::ptr::null_mut()),
            busy_and_retained: AtomicBool::new(false),
        };
        let event = context.incoming(EventData {
            kind: 10, flags: 0, key: 57, repeat: 0, tag: 0, text: vec![],
        }).unwrap();
        let port = unsafe { tap_create(1, 0, 0, 1 << 10, Some(retire_inside_tap),
            std::ptr::from_ref(&probe).cast_mut().cast()) };
        let source = unsafe { source_create(std::ptr::null(), port, 0) };
        assert!(!source.is_null());
        probe.source.set(source);
        let info = context.sources.borrow().get(&(source as usize)).unwrap().upgrade().unwrap();
        unsafe {
            scheduled(Rc::as_ptr(&info).cast_mut().cast(), context.callback.0 as Ptr, context.callback.1 as *const c_void);
            context.inject(event, None).unwrap();
            perform(Rc::as_ptr(&info).cast_mut().cast());
        }
        assert!(probe.busy_and_retained.load(Ordering::Acquire));
        assert_eq!(context.active_callbacks.get(), 0);
        assert!(!info.entered.get());
        assert!(info.pending.borrow().is_empty());
        assert!(!info.model.state.borrow().valid);
        unsafe { CFRelease(port); CFRelease(event); }
    }
}
