//! Exact owned secure-input/listen-permission/Caps prerequisites. No device,
//! real permission query, Mach port operation or global input action. No ready
//! export/registry admission; import/source qualification remains external.
use super::owned::AliveLease;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread::{self, ThreadId},
};
pub type Ptr = *mut c_void;
pub const ERROR: i32 = -536870206;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(allocator: Ptr, string: *const c_char, encoding: u32) -> Ptr;
    fn CFDictionaryCreateMutable(
        allocator: Ptr,
        capacity: isize,
        keys: *const u8,
        values: *const u8,
    ) -> Ptr;
    fn CFDictionarySetValue(dictionary: Ptr, key: Ptr, value: Ptr);
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}
struct Cf(Ptr);
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
fn matching_dictionary() -> Option<Cf> {
    let key = unsafe {
        CFStringCreateWithCString(
            std::ptr::null_mut(),
            c"IOProviderClass".as_ptr(),
            0x08000100,
        )
    };
    if key.is_null() {
        return None;
    }
    let key = Cf(key);
    let value = unsafe {
        CFStringCreateWithCString(std::ptr::null_mut(), c"IOHIDSystem".as_ptr(), 0x08000100)
    };
    if value.is_null() {
        return None;
    }
    let value = Cf(value);
    let dictionary = unsafe {
        CFDictionaryCreateMutable(
            std::ptr::null_mut(),
            0,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    };
    if dictionary.is_null() {
        return None;
    }
    unsafe {
        CFDictionarySetValue(dictionary, key.0, value.0);
    }
    Some(Cf(dictionary))
}
#[derive(Clone, Copy)]
pub enum Operation {
    Matching,
    Lookup,
    Open,
    Release,
    Close,
    Get,
    Set,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CapsConnection {
    Open,
    Read(bool),
    Disabled(bool),
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    Restored(bool),
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
impl CapsConnection {
    fn restore(&mut self, value: bool) -> bool {
        if matches!(*self, Self::Disabled(original) if original == value) {
            *self = Self::Restored(value);
            true
        } else {
            false
        }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum CapsLifecycle {
    Disabled(u32),
    ClosedAfterRestore(u32),
}

struct State {
    secure: bool,
    monitoring: bool,
    caps: bool,
    matching: BTreeSet<usize>,
    services: BTreeSet<u32>,
    connections: BTreeMap<u32, CapsConnection>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    caps_lifecycle: Option<CapsLifecycle>,
    faults: [Option<i32>; 7],
}

impl State {
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    fn has_started_caps_connection(&self, connection: u32) -> bool {
        self.connections.len() == 1
            && matches!(self.connections.get(&connection), Some(CapsConnection::Disabled(_)))
            && self.services.is_empty()
            && self.matching.is_empty()
            && !self.caps
    }
}
pub struct Context {
    life: Arc<dyn AliveLease>,
    owner: ThreadId,
    task: u32,
    retired: AtomicBool,
    state: Mutex<State>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference:Option<ReferenceBinding>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
struct ReferenceBinding {
    admission:Arc<super::core_preentry::ReferenceAdmission>,
    context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
    main_base: usize,
    task_import:super::imports::UnpatchedImportObservation,
    module:std::sync::OnceLock<(super::owned::OwnedHandleLease,Vec<super::ImportReplacement>)>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ReferenceBinding {
    fn verify(&self)->Result<(),String>{
        self.context.verify_admission_binding(&self.admission,&self.model)?;
        // These fixed source observations run synchronously without invoking
        // a guardian or returning an effect permit between them.
        unsafe { self.context.with_callback_source_observation(|| {
            self.context.verify_input_parent_source_members()?;
            self.context.verify_original_compiler_text()
        })?; }
        self.task_import.verify_unpatched()?;
        if let Some((lease,imports))=self.module.get(){
            lease.before_effect()?;
            for row in imports{if row.selected_address()!=row.replacement{return Err("Reference input prerequisite import changed".into());}}
        }
        Ok(())
    }
}
impl Context {
    pub fn new(
        life: Arc<dyn AliveLease>,
        task: u32,
        secure: bool,
        monitoring: bool,
        caps: bool,
    ) -> Result<Arc<Self>, String> {
        life.before_effect()?;
        if !life.is_alive() || task == 0 {
            return Err("invalid own input/Caps lifecycle/task".into());
        }
        Ok(Arc::new(Self {
            life,
            owner: thread::current().id(),
            task,
            retired: AtomicBool::new(false),
            state: Mutex::new(State {
                secure,
                monitoring,
                caps,
                matching: BTreeSet::new(),
                services: BTreeSet::new(),
                connections: BTreeMap::new(),
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                caps_lifecycle: None,
                faults: [None; 7],
            }),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            reference:None,
        }))
    }
    fn check(&self) -> Result<(), i32> {
        self.check_pure()?;
        let alive = catch_unwind(AssertUnwindSafe(|| {
            self.life.before_effect().is_ok() && self.life.is_alive()
        }))
        .unwrap_or(false);
        if !alive {
            self.retired.store(true, Ordering::Release);
            return Err(ERROR);
        }
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&self.reference{
            if reference.verify().is_err(){self.retired.store(true,Ordering::Release);return Err(ERROR);}
        }
        self.check_pure()
    }
    fn check_pure(&self)->Result<(),i32>{
        if self.retired.load(Ordering::Acquire)||self.owner!=thread::current().id(){Err(ERROR)}else{Ok(())}
    }
    pub fn permissions(&self, secure: bool, monitoring: bool) -> Result<(), i32> {
        self.check()?;
        let mut s = self.state.lock().map_err(|_| ERROR)?;
        self.check_pure()?;
        s.secure = secure;
        s.monitoring = monitoring;
        Ok(())
    }
    pub fn fault(&self, operation: Operation, status: Option<i32>) -> Result<(), i32> {
        self.check()?;
        if status == Some(0) {
            return Err(ERROR);
        }
        let mut state=self.state.lock().map_err(|_|ERROR)?;
        self.check_pure()?;
        state.faults[operation as usize]=status;
        Ok(())
    }
    pub fn counts(&self) -> Result<(usize, usize, usize), i32> {
        self.check()?;
        let s = self.state.lock().map_err(|_| ERROR)?;
        Ok((s.matching.len(), s.services.len(), s.connections.len()))
    }
    fn state(&self, op: Operation) -> Result<std::sync::MutexGuard<'_, State>, i32> {
        self.check()?;
        self.state_after_check(op)
    }
    fn state_after_check(&self, op: Operation) -> Result<std::sync::MutexGuard<'_, State>, i32> {
        let state = self.state.lock().map_err(|_| ERROR)?;
        self.check_pure()?;
        if let Some(e) = state.faults[op as usize] {
            return Err(e);
        }
        Ok(state)
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        // These are own extra CF retains only, never service/device resources.
        // All dictionary mutation/lookup is owner-thread checked. CF reference
        // counting itself is thread safe even if the last Arc drops elsewhere.
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        for address in &state.matching {
            unsafe {
                CFRelease(*address as Ptr);
            }
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
fn reference_roster()->BTreeSet<super::SourceCallsite>{
    [
        ("reference-0.8.26/Sources/RuycastApp/AppState.swift","ffba512c44134bf74005bd6cc409c1c92d09b4a13ee9709ca60a7b6e895abd14"),
        ("reference-0.8.26/Sources/RuycastApp/HyperKey.swift","a60dff19c9ab210fd8dc3ae564f962a2708b1e32a2f69663ff2c48e6d70b1544"),
        ("reference-0.8.26/Sources/RuycastApp/SnippetExpander.swift","0982fd55b9406f9983d40803ada1f97d415251ca0e5908b888b27da7fd697b96"),
        ("reference-0.8.26/Sources/RuycastApp/Appearance.swift","34ed301465269449351ac544fd899902fdfe72c150d577859acc1a90c783bc93"),
        ("reference-0.8.26/Sources/RuycastApp/Management.swift","3057068c7ff081e4f551f46690fb5f29f5e3b1d7f63b55d5667fcdfbebf2d42f"),
    ].into_iter().map(|(file,sha)|super::SourceCallsite{file,source_sha256:sha.into(),operation:"Private secure-input/listen/Caps prerequisites; no real permission or device access"}).collect()
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) unsafe fn initialize_reference(
    admission:Arc<super::core_preentry::ReferenceAdmission>,context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
)->Result<(Arc<Context>,super::ModuleAttestation),String>{
    if REFERENCE_ATTEMPT.swap(true,Ordering::AcqRel){return Err("Reference input prerequisite construction attempt already claimed".into());}
    let module=super::bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;
    admission.verify_input_context(&module)?;
    unsafe extern "C"{static mach_task_self_:u32;}
    let task_address=std::ptr::addr_of!(mach_task_self_) as usize;
    let observed=super::imports::observe_unpatched_main_imports(&[(c"mach_task_self_",task_address)])?;
    if observed.len()!=1||observed[0].symbol!=c"mach_task_self_"{return Err("Reference input prerequisite task import differs".into());}
    let task=unsafe{std::ptr::read_volatile(task_address as *const u32)};
    if task==0{return Err("Reference input prerequisite task identity absent".into());}
    let main_base = context.observe_app_delegate_offset_cells()?.main_header();
    let reference=ReferenceBinding{admission,context,model,main_base,task_import:observed[0].clone(),module:std::sync::OnceLock::new()};
    reference.verify()?;
    // Fixed synthetic run state. These booleans neither query nor describe
    // Viggo's actual permissions/secure entry/Caps Lock. All device handles
    // below are private process-unique integers and never enter IOKit.
    let candidate=Arc::new(Context{life:module.life.clone(),owner:thread::current().id(),task,retired:AtomicBool::new(false),
        state:Mutex::new(State{secure:false,monitoring:true,caps:false,matching:BTreeSet::new(),services:BTreeSet::new(),connections:BTreeMap::new(),caps_lifecycle:None,faults:[None;7]}),reference:Some(reference)});
    install(candidate.clone())?;
    struct Publication{context:Arc<Context>,complete:bool}
    impl Drop for Publication{fn drop(&mut self){if !self.complete{let _=retire(&self.context);}}}
    let mut publication=Publication{context:candidate.clone(),complete:false};
    struct Guard<'a>(&'a Context);
    impl AliveLease for Guard<'_>{
        fn is_alive(&self)->bool{self.before_effect().is_ok()}
        fn before_effect(&self)->Result<(),String>{self.0.check().map_err(|_|"Reference input prerequisite retired".into())}
    }
    let lease=module.lease("reference-input-prerequisites")?;
    let imports=super::import_patch::install(&import_specs(),&Guard(&candidate))?;
    candidate.reference.as_ref().ok_or("Reference input prerequisite source absent")?.module.set((lease,imports)).map_err(|_|"Reference input prerequisite module already installed")?;
    let source=candidate.reference.as_ref().ok_or("Reference input prerequisite source absent")?;
    let (lease,imports)=source.module.get().ok_or("Reference input prerequisite imports absent")?;
    let result=super::ModuleAttestation{module:"reference-input-prerequisites",scopes:BTreeSet::new(),callsites:reference_roster(),
        imports:imports.clone(),objc:vec![],handles:vec![lease.clone()],workspace_factories:None};
    verify_reference_attestation(&result)?;
    publication.complete=true;Ok((candidate,result))
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_attestation(module:&super::ModuleAttestation)->Result<(),String>{
    if module.module!="reference-input-prerequisites"||!module.scopes.is_empty()||module.callsites!=reference_roster()
        ||!module.objc.is_empty()||module.workspace_factories.is_some()||module.handles.len()!=1{return Err("Reference input prerequisite envelope differs".into());}
    let context=CURRENT.lock().map_err(|_|"Input prerequisite owner poisoned")?.clone().ok_or("Reference input prerequisite absent")?;
    if context.owner == std::thread::current().id() {
        context.check().map_err(|_|"Reference input prerequisite retired")?;
    }
    let source=context.reference.as_ref().ok_or("Reference input prerequisite source absent")?;
    let (lease,imports)=source.module.get().ok_or("Reference input prerequisite imports absent")?;
    if module.handles[0].namespace!=lease.namespace||module.handles[0].root.path()!=lease.root.path()
        ||!Arc::ptr_eq(&module.handles[0].life,&lease.life)||module.imports.len()!=imports.len(){return Err("Reference input prerequisite issuer differs".into());}
    lease.before_effect()?;
    source.task_import.verify_unpatched()?;
    let mut symbols=BTreeSet::new();
    for row in &module.imports{
        let expected=imports.iter().find(|r|r.symbol==row.symbol).ok_or("Foreign Reference input prerequisite import")?;
        if !symbols.insert(row.symbol)||row.original!=expected.original||row.replacement!=expected.replacement||row.cell_address()!=expected.cell_address()||row.selected_address()!=row.replacement{
            return Err("Reference input prerequisite import differs".into());
        }
    }
    let current = CURRENT.lock().map_err(|_|"Input prerequisite owner poisoned")?;
    if !PUBLISHED.load(Ordering::Acquire)
        || context.retired.load(Ordering::Acquire)
        || !current.as_ref().is_some_and(|published| Arc::ptr_eq(published, &context))
    {
        return Err("Reference input prerequisite publication changed during attestation".into());
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn retire_reference(admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->Result<(),String>{
    let context=CURRENT.lock().map_err(|_|"Input prerequisite owner poisoned")?.clone();
    let context = context.ok_or("Reference input prerequisite teardown has no installed owner to drain")?;
    let source=context.reference.as_ref().ok_or("Input prerequisite teardown found another source family")?;
    if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model){return Err("Input prerequisite teardown found another Reference owner".into());}
    retire(&context)
}

#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_startup_connection(
    admission: &Arc<super::core_preentry::ReferenceAdmission>,
    model: &Arc<crate::workspace::Workspace>,
    keyboard: &crate::workspace::factory::ReferenceInputChildCustody,
) -> Result<(), String> {
    let context = CURRENT.lock().map_err(|_| "Input prerequisite owner poisoned")?
        .clone().ok_or("Reference input prerequisite absent")?;
    let source = context.reference.as_ref().ok_or("Reference input prerequisite source absent")?;
    if !Arc::ptr_eq(&source.admission, admission) || !Arc::ptr_eq(&source.model, model) {
        return Err("Reference Caps connection issuer differs".into());
    }
    context.check().map_err(|_| "Reference input prerequisite retired")?;
    keyboard.verify_reference_binding(admission, model)?;
    let connection = keyboard.caps_connection_on_creator()?;
    let state = context.state.lock().map_err(|_| "Reference Caps state poisoned")?;
    context.check_pure().map_err(|_| "Reference Caps owner retired during startup")?;
    if !state.has_started_caps_connection(connection)
        || state.caps_lifecycle != Some(CapsLifecycle::Disabled(connection))
    {
        return Err("Reference HyperKey did not read and disable Caps on its sole live connection".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_closed_caps_connection(
    admission: &Arc<super::core_preentry::ReferenceAdmission>,
    model: &Arc<crate::workspace::Workspace>,
    connection: Option<u32>,
) -> Result<(), String> {
    let current = CURRENT.lock().map_err(|_| "Reference Caps owner poisoned")?;
    let context = current.as_ref().ok_or("Reference Caps owner absent")?;
    let source = context.reference.as_ref().ok_or("Reference Caps source absent")?;
    let state = context.state.lock().map_err(|_| "Reference Caps state poisoned")?;
    if !PUBLISHED.load(Ordering::Acquire) || context.check_pure().is_err()
        || !Arc::ptr_eq(&source.admission, admission) || !Arc::ptr_eq(&source.model, model)
        || !state.connections.is_empty()
        || !matches!(state.caps_lifecycle, Some(CapsLifecycle::ClosedAfterRestore(closed))
            if connection.is_none_or(|expected| closed == expected))
    {
        return Err("Reference HyperKey did not close its Caps connection".into());
    }
    Ok(())
}
// Both service and connection handles are process-unique. Retiring an owner
// cannot make a stale numeric capability refer to a later owner's object.
static NEXT_HANDLE: AtomicU32 = AtomicU32::new(0x61000000);
fn next_handle() -> Option<u32> {
    NEXT_HANDLE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .ok()
        .and_then(|n| n.checked_add(1))
}
pub fn install(context: Arc<Context>) -> Result<(), String> {
    context.check().map_err(|_| "retired own input context")?;
    let mut slot = CURRENT.lock().map_err(|_| "input owner poisoned")?;
    if slot.is_some() {
        return Err("input owner already installed".into());
    }
    *slot = Some(context);
    PUBLISHED.store(true, Ordering::Release);
    Ok(())
}
pub fn retire(expected: &Arc<Context>) -> Result<(), String> {
    let mut slot = CURRENT.lock().map_err(|_| "input owner poisoned")?;
    if !slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
        return Err("stale own input owner".into());
    }
    PUBLISHED.store(false, Ordering::Release);
    expected.retired.store(true, Ordering::Release);
    *slot = None;
    drop(slot);
    let (matching, unresolved) = {
        let mut state = expected.state.lock().map_err(|_| "input state poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let unresolved = expected.reference.is_some()
            && (!state.matching.is_empty() || !state.services.is_empty()
                || !state.connections.is_empty()
                || matches!(state.caps_lifecycle, Some(CapsLifecycle::Disabled(_))));
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let unresolved = false;
        state.services.clear();
        state.connections.clear();
        (std::mem::take(&mut state.matching), unresolved)
    };
    // Only the registry's extra retain belongs to us. Swift still owns the
    // reference returned by IOServiceMatching if lookup never consumed it.
    for address in matching {
        unsafe { CFRelease(address as Ptr); }
    }
    if unresolved {
        Err("Reference Caps retired with an unresolved matching, service or connection".into())
    } else {
        Ok(())
    }
}
fn with<T>(f: impl FnOnce(&Context) -> Option<T>) -> Option<T> {
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
        f(&context)
    }))
    .ok()
    .flatten()
}
unsafe extern "C" fn secure() -> u8 {
    with(|c| {let state=c.state.lock().ok()?;c.check_pure().ok()?;Some(state.secure.into())}).unwrap_or(1)
}
unsafe extern "C" fn monitoring() -> bool {
    with(|c| {let state=c.state.lock().ok()?;c.check_pure().ok()?;Some(state.monitoring)}).unwrap_or(false)
}
unsafe extern "C" fn matching(name: *const c_char) -> Ptr {
    with(|c| {
        if name.is_null() || unsafe { CStr::from_ptr(name) } != c"IOHIDSystem" {
            return None;
        }
        let mut s = c.state(Operation::Matching).ok()?;
        if s.matching.len() >= 256 {
            return None;
        }
        let dictionary = matching_dictionary()?;
        c.check_pure().ok()?;
        // Return one caller-owned retain; independently retain only while this
        // matching capability can be consumed by the owned lookup leaf.
        unsafe {
            CFRetain(dictionary.0);
        }
        s.matching.insert(dictionary.0 as usize);
        let pointer = dictionary.0;
        std::mem::forget(dictionary);
        Some(pointer)
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn lookup(main: u32, dictionary: Ptr) -> u32 {
    with(|c| {
        {
            let mut state = c.state.lock().ok()?;
            c.check_pure().ok()?;
            if !state.matching.remove(&(dictionary as usize)) {
                return None;
            }
        }
        // The real API consumes the caller's matching reference on all results.
        // Drop the separate own capability retain as well, exactly once.
        unsafe {
            CFRelease(dictionary);
            CFRelease(dictionary);
        }
        let mut s = c.state.lock().ok()?;
        c.check_pure().ok()?;
        if main != 0 || s.services.len() >= 4096 || s.faults[Operation::Lookup as usize].is_some() {
            return None;
        }
        let id = next_handle()?;
        s.services.insert(id);
        c.check_pure().ok()?;
        Some(id)
    })
    .unwrap_or(0)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn open(_: u32, _: u32, _: u32, _: *mut u32) -> i32 {
    core::arch::naked_asm!("mov x4, x30", "b {checked}", checked = sym open_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn open(service: u32, task: u32, kind: u32, out: *mut u32) -> i32 {
    if out.is_null() {
        return ERROR;
    }
    unsafe { out.write(0); }
    unsafe { open_checked(service, task, kind, out, 0) }
}

unsafe extern "C" fn open_checked(service: u32, task: u32, kind: u32, out: *mut u32, caller: usize) -> i32 {
    if out.is_null() || !out.is_aligned() {
        return ERROR;
    }
    with(|c| {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(source) = &c.reference {
            if source.main_base.checked_add(0xf83f4) != Some(caller) {
                return Some(ERROR);
            }
            source.context.verify_main_code(source.main_base.checked_add(0xf83e0)?, 20,
                "0109d9e98613cf6a5e738a2c3b677e858db191d6113f9eb0846ba914a5860dce").ok()?;
            source.context.verify_main_code(source.main_base.checked_add(0x2fb420)?, 12,
                "b6971be98143df4be7ff09b5587de0a4a5f8aa74ac32cf5f4bea8dc6e4a34afe").ok()?;
        }
        let _ = caller;
        if task != c.task || kind != 1 {
            return Some(ERROR);
        }
        let mut s = match c.state(Operation::Open) {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        if !s.services.contains(&service) || s.connections.len() >= 4096 {
            return Some(ERROR);
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() && (!s.connections.is_empty()
            || matches!(s.caps_lifecycle, Some(CapsLifecycle::Disabled(_))))
        {
            return Some(ERROR);
        }
        let id = next_handle()?;
        s.connections.insert(id, CapsConnection::Open);
        unsafe {
            out.write(id);
        }
        Some(0)
    })
    .unwrap_or(ERROR)
}
unsafe extern "C" fn release(service: u32) -> i32 {
    with(|c| {
        let mut s = match c.state(Operation::Release) {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        Some(if s.services.remove(&service) {
            0
        } else {
            ERROR
        })
    })
    .unwrap_or(ERROR)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn close(_: u32) -> i32 {
    core::arch::naked_asm!("mov x1, x30", "b {checked}", checked = sym close_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn close(connection: u32) -> i32 {
    unsafe { close_checked(connection, 0) }
}

unsafe extern "C" fn close_checked(connection: u32, caller: usize) -> i32 {
    with(|c| {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let stop = if let Some(source) = &c.reference {
            let base = source.main_base;
            let (offset, length, sha) = match caller.checked_sub(base)? {
                0xf7fc8 => (0xf7fc0, 12, "7c42010189af379256f618d8d5cae01efdf755f418cfcc235bfa05c6eed890b9"),
                0xf8458 => (0xf844c, 16, "3677b4b22798b9fb4bf6086894e44e28086098623244125c5731836ae3305960"),
                _ => return Some(ERROR),
            };
            source.context.verify_main_code(base.checked_add(offset)?, length, sha).ok()?;
            source.context.verify_main_code(base.checked_add(0x2fb3fc)?, 12,
                "427f351a5273470f179fe9e1a1d7092dc031edc47df72c10bdbe47c8bfcf3d04").ok()?;
            let (_, imports) = source.module.get()?;
            let import = imports.iter().find(|row| row.symbol == c"IOServiceClose")?;
            if base.checked_add(0x3c5960) != Some(import.cell_address())
                || import.selected_address() != import.replacement
            {
                return Some(ERROR);
            }
            offset == 0xf7fc0
        } else {
            false
        };
        let _ = caller;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let state = if c.reference.is_some() {
            c.state_after_check(Operation::Close)
        } else {
            c.state(Operation::Close)
        };
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let state = c.state(Operation::Close);
        let mut s = match state {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() {
            if stop {
                if s.connections.get(&connection) != Some(&CapsConnection::Restored(s.caps))
                    || s.caps_lifecycle != Some(CapsLifecycle::Disabled(connection))
                {
                    return Some(ERROR);
                }
                s.caps_lifecycle = Some(CapsLifecycle::ClosedAfterRestore(connection));
            } else if !matches!(s.connections.get(&connection), Some(CapsConnection::Open | CapsConnection::Read(_))) {
                return Some(ERROR);
            }
        }
        Some(if s.connections.remove(&connection).is_some() {
            0
        } else {
            ERROR
        })
    })
    .unwrap_or(ERROR)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn get(_: u32, _: i32, _: *mut bool) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym get_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn get(connection: u32, selector: i32, out: *mut bool) -> i32 {
    unsafe { get_checked(connection, selector, out, 0) }
}

unsafe extern "C" fn get_checked(connection: u32, selector: i32, out: *mut bool, caller: usize) -> i32 {
    with(|c| {
        if selector != 1 || out.is_null() {
            return Some(ERROR);
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(source) = &c.reference {
            if source.main_base.checked_add(0xf842c) != Some(caller) {
                return Some(ERROR);
            }
            source.context.verify_main_code(source.main_base.checked_add(0xf841c)?, 20,
                "d6ea1d86696dea90b83f226b8606b080e0a1afaf69516572763c06edc39f60f8").ok()?;
            source.context.verify_main_code(source.main_base.checked_add(0x2fb330)?, 12,
                "5bee84a82cd6f3d09a5bb92c5a9dbc96e36293720d5853e69fc574aac8b0c0e2").ok()?;
        }
        let _ = caller;
        let mut s = match c.state(Operation::Get) {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        let original = s.caps;
        let Some(phase) = s.connections.get_mut(&connection) else {
            return Some(ERROR);
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if c.reference.is_some() && *phase != CapsConnection::Open {
            return Some(ERROR);
        }
        if *phase == CapsConnection::Open {
            *phase = CapsConnection::Read(original);
        }
        unsafe {
            out.write(s.caps);
        }
        Some(0)
    })
    .unwrap_or(ERROR)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set(_: u32, _: i32, _: bool) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn set(connection: u32, selector: i32, value: bool) -> i32 {
    unsafe { set_checked(connection, selector, value, 0) }
}

unsafe extern "C" fn set_checked(connection: u32, selector: i32, value: bool, caller: usize) -> i32 {
    with(|c| {
        if selector != 1 {
            return Some(ERROR);
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let reference_call = if let Some(source) = &c.reference {
            let base = source.main_base;
            let call = caller.checked_sub(base)?;
            let (offset, length, sha) = match call {
                0xf7fc0 => (0xf7fb0, 16, "1850966293ec07b7edda1b09a767a87fa981c955c9f525e597d7c06b6598a21a"),
                0xf8444 => (0xf8438, 16, "3c9735a14d59a3498f9f4bb827adaf6188d6dc5d782b8fdc7e6a6833ea97bc27"),
                0xfa18c => (0xfa180, 20, "e8ec76c401b946b90e525ab3b222f71fc231fc10821d7e5533f4ce542e8160f3"),
                _ => return Some(ERROR),
            };
            source.context.verify_main_code(base.checked_add(offset)?, length, sha).ok()?;
            if call != 0xf7fc0 {
                source.context.verify_main_code(base.checked_add(0xfc418)?, 12,
                    "a6d8b172c8201660170a04ae09132fb417d0d10e2e0e672600309f0e5afb3d0d").ok()?;
            }
            source.context.verify_main_code(base.checked_add(0x2fb3a8)?, 12,
                "efd398805a2215a4b308cd4cae153c6dcc623f45660c27cdf0fd129b082fa231").ok()?;
            let (_, imports) = source.module.get()?;
            let import = imports.iter().find(|row| row.symbol == c"IOHIDSetModifierLockState")?;
            if base.checked_add(0x3c5928) != Some(import.cell_address())
                || import.selected_address() != import.replacement
            {
                return Some(ERROR);
            }
            Some(call)
        } else {
            None
        };
        let _ = caller;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let state = if reference_call.is_some() { c.state_after_check(Operation::Set) } else { c.state(Operation::Set) };
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let state = c.state(Operation::Set);
        let mut s = match state {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(call) = reference_call {
            let active = s.caps_lifecycle == Some(CapsLifecycle::Disabled(connection));
            let phase = s.connections.get(&connection).copied();
            let allowed = match call {
                0xf7fc0 => active && matches!(phase, Some(CapsConnection::Disabled(_))),
                0xf8444 => !value && matches!(phase, Some(CapsConnection::Read(_)))
                    && !matches!(s.caps_lifecycle, Some(CapsLifecycle::Disabled(_))),
                0xfa18c => !value && active && matches!(phase, Some(CapsConnection::Disabled(_))),
                _ => false,
            };
            if !allowed {
                return Some(ERROR);
            }
        }
        let Some(phase) = s.connections.get_mut(&connection) else {
            return Some(ERROR);
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if reference_call == Some(0xf7fc0) {
            if !phase.restore(value) {
                return Some(ERROR);
            }
            s.caps = value;
            return Some(0);
        }
        if value {
            *phase = CapsConnection::Open;
        } else if let CapsConnection::Read(original) = *phase {
            *phase = CapsConnection::Disabled(original);
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if c.reference.is_some() {
                s.caps_lifecycle = Some(CapsLifecycle::Disabled(connection));
            }
        }
        s.caps = value;
        Some(0)
    })
    .unwrap_or(ERROR)
}
pub fn replacements() -> [(&'static CStr, usize); 9] {
    [
        (c"IsSecureEventInputEnabled", secure as *const () as usize),
        (
            c"CGPreflightListenEventAccess",
            monitoring as *const () as usize,
        ),
        (c"IOServiceMatching", matching as *const () as usize),
        (c"IOServiceGetMatchingService", lookup as *const () as usize),
        (c"IOServiceOpen", open as *const () as usize),
        (c"IOObjectRelease", release as *const () as usize),
        (c"IOServiceClose", close as *const () as usize),
        (c"IOHIDGetModifierLockState", get as *const () as usize),
        (c"IOHIDSetModifierLockState", set as *const () as usize),
    ]
}
/// Fixed typed system entries; taking their addresses performs no permission,
/// secure-input, device or Caps operation.
pub fn import_specs() -> [super::ImportSpec; 9] {
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" { fn IsSecureEventInputEnabled() -> u8; }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" { fn CGPreflightListenEventAccess() -> bool; }
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> Ptr;
        fn IOServiceGetMatchingService(main: u32, dictionary: Ptr) -> u32;
        fn IOServiceOpen(service: u32, task: u32, kind: u32, out: *mut u32) -> i32;
        fn IOObjectRelease(service: u32) -> i32;
        fn IOServiceClose(connection: u32) -> i32;
        fn IOHIDGetModifierLockState(connection: u32, selector: i32, out: *mut bool) -> i32;
        fn IOHIDSetModifierLockState(connection: u32, selector: i32, value: bool) -> i32;
    }
    let original = [IsSecureEventInputEnabled as *const () as usize, CGPreflightListenEventAccess as *const () as usize,
        IOServiceMatching as *const () as usize, IOServiceGetMatchingService as *const () as usize,
        IOServiceOpen as *const () as usize, IOObjectRelease as *const () as usize,
        IOServiceClose as *const () as usize, IOHIDGetModifierLockState as *const () as usize,
        IOHIDSetModifierLockState as *const () as usize];
    let leaves = replacements();
    std::array::from_fn(|i| super::ImportSpec { symbol: leaves[i].0, original: original[i], replacement: leaves[i].1 })
}
/// Actual import cells and the existing issuer only. No permission/scope,
/// source-code or callback authority is created, and no Context is installed.
pub fn observe_owned_imports(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
) -> Result<(super::owned::OwnedHandleLease, Vec<super::ImportReplacement>), String> {
    let context = super::bootstrap::owned_module_context(life, descriptors)?;
    let lease = context.lease("input-permissions-caps")?;
    let imports = super::observe_main_imports(&import_specs())?;
    let confirmed = super::bootstrap::owned_module_context(life, descriptors)?;
    if !Arc::ptr_eq(&lease.life, &confirmed.life) { return Err("Input/Caps import issuer changed".into()); }
    lease.before_effect()?;
    if imports.iter().any(|row| row.selected_address() != row.replacement) {
        return Err("Input/Caps selected import changed".into());
    }
    Ok((lease, imports))
}

#[cfg(test)]
mod import_issuer_tests {
    #[test]
    fn reference_stop_restores_only_the_captured_caps_value_once() {
        use super::CapsConnection;
        for original in [false, true] {
            let mut open = CapsConnection::Open;
            assert!(!open.restore(original));
            let mut connection = CapsConnection::Disabled(original);
            assert!(!connection.restore(!original));
            assert!(connection.restore(original));
            assert!(matches!(connection, CapsConnection::Restored(value) if value == original));
            assert!(!connection.restore(original));
        }
    }
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
        assert_eq!(specs.len(), 9);
        assert_eq!(specs.iter().map(|s| s.symbol).collect::<std::collections::BTreeSet<_>>().len(), 9);
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
    fn private_prerequisites_consume_matching_close_handles_and_fence_reentry(){
        let _publication = PUBLICATION_TEST_LOCK.lock().unwrap();
        use std::sync::{Weak,atomic::AtomicUsize};
        unsafe extern "C"{fn CFGetRetainCount(value:*const c_void)->isize;}
        struct Audit{context:Mutex<Weak<Context>>,calls:AtomicUsize,retire_at:AtomicUsize,under_lock:AtomicBool}
        impl AliveLease for Audit{
            fn is_alive(&self)->bool{true}
            fn before_effect(&self)->Result<(),String>{
                let call=self.calls.fetch_add(1,Ordering::AcqRel)+1;
                let context=self.context.lock().unwrap().upgrade();
                if let Some(context)=context{
                    if context.state.try_lock().is_err(){self.under_lock.store(true,Ordering::Release);}
                    if call==self.retire_at.load(Ordering::Acquire){context.retired.store(true,Ordering::Release);}
                }
                Ok(())
            }
        }
        let audit=Arc::new(Audit{context:Mutex::new(Weak::new()),calls:AtomicUsize::new(0),retire_at:AtomicUsize::new(usize::MAX),under_lock:AtomicBool::new(false)});
        let context=Context::new(audit.clone(),7,false,true,false).unwrap();
        *audit.context.lock().unwrap()=Arc::downgrade(&context);
        install(context.clone()).unwrap();
        unsafe{
            assert_eq!(secure(),0);assert!(monitoring());
            context.permissions(true,false).unwrap();assert_eq!(secure(),1);assert!(!monitoring());
            context.permissions(false,true).unwrap();
            let dictionary=matching(c"IOHIDSystem".as_ptr());assert!(!dictionary.is_null());
            assert_eq!(lookup(1,dictionary),0);assert_eq!(context.counts().unwrap(),(0,0,0));
            let dictionary=matching(c"IOHIDSystem".as_ptr());assert!(!dictionary.is_null());
            CFRetain(dictionary);let before=CFGetRetainCount(dictionary);
            let service=lookup(0,dictionary);assert!(service>=0x61000000);
            assert_eq!(CFGetRetainCount(dictionary),before-2);
            CFRelease(dictionary); // only this test's independent sentinel +1
            assert_eq!(lookup(0,dictionary),0);
            let mut connection=99;assert_eq!(open(service,8,1,&mut connection),ERROR);assert_eq!(connection,0);
            assert_eq!(open(service,7,1,&mut connection),0);assert_ne!(connection,service);
            assert_eq!(release(service),0);assert_eq!(release(service),ERROR);
            let started = |handle| context.state.lock().unwrap().has_started_caps_connection(handle);
            assert!(!started(connection));
            assert_eq!(set(connection,1,false),0);
            assert!(!started(connection));
            let mut caps = true;
            context.fault(Operation::Get, Some(ERROR)).unwrap();
            assert_eq!(get(connection,1,&mut caps),ERROR);
            context.fault(Operation::Get, None).unwrap();
            assert_eq!(set(connection,1,false),0);
            assert!(!started(connection));
            assert_eq!(get(connection,1,&mut caps),0);assert!(!caps);
            context.fault(Operation::Set, Some(ERROR)).unwrap();
            assert_eq!(set(connection,1,false),ERROR);
            context.fault(Operation::Set, None).unwrap();
            assert!(!started(connection));
            assert_eq!(set(connection,1,false),0);
            assert!(started(connection));
            assert!(!started(service));
            assert_eq!(set(connection,1,true),0);assert_eq!(get(connection,1,&mut caps),0);assert!(caps);
            assert!(!started(connection));
            assert_eq!(close(connection),0);assert_eq!(close(connection),ERROR);
            assert_eq!(context.counts().unwrap(),(0,0,0));
            assert!(!audit.under_lock.load(Ordering::Acquire));
            let dictionary=matching(c"IOHIDSystem".as_ptr());let service=lookup(0,dictionary);
            audit.calls.store(0,Ordering::Release);audit.retire_at.store(2,Ordering::Release);
            connection=99;assert_eq!(open(service,7,1,&mut connection),ERROR);assert_eq!(connection,0);
            assert!(context.state.lock().unwrap().connections.is_empty());
            assert!(!audit.under_lock.load(Ordering::Acquire));
            audit.retire_at.store(usize::MAX, Ordering::Release);
            // Install a fresh owner for the positive retirement case. The
            // previous owner already refused its final guard.
            retire(&context).unwrap();
            let context = Context::new(audit.clone(), 7, false, true, false).unwrap();
            *audit.context.lock().unwrap() = Arc::downgrade(&context);
            install(context.clone()).unwrap();
            let service = lookup(0, matching(c"IOHIDSystem".as_ptr()));
            assert_eq!(open(service, 7, 1, &mut connection), 0);
            let unconsumed = matching(c"IOHIDSystem".as_ptr());
            assert!(!unconsumed.is_null());
            let before = CFGetRetainCount(unconsumed);
            assert_eq!(context.counts().unwrap(), (1, 1, 1));
            retire(&context).unwrap();
            assert_eq!(CFGetRetainCount(unconsumed), before - 1);
            CFRelease(unconsumed); // caller's original retain
            let state = context.state.lock().unwrap();
            assert!(state.matching.is_empty() && state.services.is_empty() && state.connections.is_empty());
            drop(state);
            assert_eq!(secure(),1);assert!(!monitoring());assert!(matching(c"IOHIDSystem".as_ptr()).is_null());
            assert_eq!(get(connection,1,&mut caps),ERROR);assert_eq!(set(connection,1,false),ERROR);
        }
    }
}
