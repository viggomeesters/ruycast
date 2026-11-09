//! Concrete owned IOHID substitution. No real HID/device/runloop operations,
//! ready export, constructor, registry admission or original-function fallback.
use super::owned::AliveLease;
use super::{core_preentry::{NativeInputAdmission, NativeInputCallbackWitness, NativeInputContextKind, NativeInputInstallation, NativeInputOwnerFamily, NativeInputOwnerTicket}, owned::ModuleContext};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, ThreadId},
};
pub type Ptr = *mut c_void;
pub type Callback = unsafe extern "C" fn(Ptr, i32, Ptr, Ptr);
pub const ERROR: i32 = -536870206;
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
// Already-reviewed CFRuntime version0 ABI. Descriptors are immutable static
// data; no CF object/base-layout fields are read or modified.
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
static MANAGER: RuntimeClass = class(c"RuycastF1OwnedHIDManager".as_ptr());
static DEVICE: RuntimeClass = class(c"RuycastF1OwnedHIDDevice".as_ptr());
static ELEMENT: RuntimeClass = class(c"RuycastF1OwnedHIDElement".as_ptr());
static VALUE: RuntimeClass = class(c"RuycastF1OwnedHIDValue".as_ptr());
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
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetCount(value: *const c_void) -> isize;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(
        allocator: Ptr,
        text: *const c_char,
        encoding: u32,
    ) -> Ptr;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: *const c_void, kind: isize, out: *mut c_void) -> u8;
}
static TYPES: OnceLock<Result<[usize; 4], String>> = OnceLock::new();
fn types() -> Result<[usize; 4], String> {
    TYPES
        .get_or_init(|| {
            let classes = [
                &raw const MANAGER,
                &raw const DEVICE,
                &raw const ELEMENT,
                &raw const VALUE,
            ];
            let mut result = [0; 4];
            for (i, class) in classes.into_iter().enumerate() {
                let id = unsafe { _CFRuntimeRegisterClass(class) };
                if id == 0
                    || unsafe { _CFRuntimeGetClassWithTypeID(id) } != class
                    || result.contains(&id)
                {
                    return Err("own HID CF registration refused".into());
                }
                result[i] = id;
            }
            Ok(result)
        })
        .clone()
}
fn cached() -> Option<[usize; 4]> {
    TYPES.get().and_then(|r| r.as_ref().ok().copied())
}
struct Token(NonNull<c_void>);
// Genuine immutable CF identity/refcount only. Mutable manager state is
// separately locked, and each operation requires the owner's actual thread.
unsafe impl Send for Token {}
unsafe impl Sync for Token {}
impl Token {
    fn ptr(&self) -> Ptr {
        self.0.as_ptr()
    }
    fn retained(&self) -> Ptr {
        unsafe { CFRetain(self.ptr()).cast_mut() }
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
enum Payload {
    Manager(Arc<Manager>),
    Device,
    Element {
        device: Token,
        page: u32,
        usage: u32,
    },
    Value {
        element: Token,
        integer: isize,
    },
}
struct Entry {
    owner: u64,
    payload: Payload,
}
static ENTRIES: OnceLock<Mutex<BTreeMap<usize, Entry>>> = OnceLock::new();
fn entries() -> &'static Mutex<BTreeMap<usize, Entry>> {
    ENTRIES.get_or_init(Mutex::default)
}
unsafe extern "C" fn finalize(pointer: Ptr) {
    // Release nested CF tokens only AFTER releasing the metadata lock. Values
    // retain elements, elements retain devices; manager payload owns its device.
    let removed = ENTRIES
        .get()
        .and_then(|entries| entries.lock().ok()?.remove(&(pointer as usize)));
    drop(removed);
}
fn token(kind: usize, owner: u64, payload: Payload) -> Result<Token, String> {
    let ids = cached().ok_or("own HID classes absent")?;
    let pointer = NonNull::new(unsafe {
        _CFRuntimeCreateInstance(std::ptr::null_mut(), ids[kind], 0, std::ptr::null())
    })
    .ok_or("own HID allocation failed")?;
    let token = Token(pointer);
    if unsafe { CFGetTypeID(token.ptr()) } != ids[kind] {
        return Err("own HID identity differs".into());
    }
    entries()
        .lock()
        .map_err(|_| "HID metadata poisoned")?
        .insert(token.ptr() as usize, Entry { owner, payload });
    Ok(token)
}
#[derive(Clone)]
struct Registered {
    callback: Callback,
    user: usize,
    owner: Option<Arc<NativeInputOwnerTicket>>,
    generation: u64,
}
struct State {
    open: bool,
    scheduled: bool,
    device_match: bool,
    input_valid: bool,
    input_page: Option<u32>,
    present: bool,
    input: Option<Registered>,
    remove: Option<Registered>,
    input_generation: u64,
    remove_generation: u64,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl State {
    fn reference_keyboard_ready(&self) -> bool {
        self.device_match && self.input_valid && self.input_page == Some(7)
            && self.scheduled && self.input.is_some() && self.remove.is_some()
    }
}
struct Manager {
    device: Token,
    state: Mutex<State>,
}
#[derive(Clone, Copy)]
pub struct Loop {
    pub runloop: usize,
    pub mode: usize,
}
struct Faults {
    open: Option<i32>,
    close: Option<i32>,
    last: Option<i32>,
}
struct NativeBinding {
    context: ModuleContext,
    installation: NativeInputInstallation,
    input: NativeInputCallbackWitness,
    removed: NativeInputCallbackWitness,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
struct ReferenceBinding {
    admission: Arc<super::core_preentry::ReferenceAdmission>,
    context: Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model: Arc<crate::workspace::Workspace>,
    child: OnceLock<Arc<crate::workspace::factory::ReferenceInputChildCustody>>,
    module: OnceLock<(super::owned::OwnedHandleLease,Vec<super::ImportReplacement>)>,
    // Borrowed identity of CF's process-owned main loop. Never dereferenced;
    // the typed getter is reacquired on the creator thread when qualifying it.
    main_loop: usize,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ReferenceBinding {
    fn code(&self)->Result<usize,String> {
        use sha2::Digest;
        self.context.verify_admission_binding(&self.admission,&self.model)?;
        self.context.verify_input_parent_source_members()?;
        let main=objc2_core_foundation::CFRunLoop::main().ok_or("Reference main CF loop absent")?;
        if &*main as *const _ as usize!=self.main_loop{return Err("Reference HID main CF loop changed".into());}
        let base=self.context.observe_app_delegate_offset_cells()?.main_header();
        for (offset,bytes,sha) in [
            (0xf8170usize,1708usize,"8e4721bb651f2968220304504ef6d779a083df4f73aad43fd96c0958eb871e80"),
            (0xf9954,188,"6ab555c44969f4b0d530965e8d9fa0b07a561cb1d80e454d2650c148eace6434"),
            (0xf9eb0,72,"a4fdcb00d613fdd2dd06cf07b9e0a1761ca68539353fe35b1fe33e5ef7a5a5e1"),
            (0xf9ef8,316,"4cfbaca67b0ec82cf17abc1d1c4b0cc9d7678859e7e1713ab5492cf7ae783a63"),
            (0xfa138,52,"9a9d76dd7547160eb9e1206061575e6427eb2841fb3058e3e614660a20f76aa7"),
        ] {self.context.verify_main_code(base.checked_add(offset).ok_or("Reference HID code overflow")?,bytes,sha)?;}
        for (entry,checked) in [(register_input as *const () as usize,register_input_checked as *const () as usize),
            (register_remove as *const () as usize,register_remove_checked as *const () as usize)] {
            self.context.verify_app_delegate_sdk_members(entry,checked)?;
            let delta=checked as i128-entry.checked_add(12).ok_or("HID entry overflow")? as i128;
            if delta%4!=0||!(-(1i128<<27)..(1i128<<27)).contains(&delta){return Err("HID helper outside direct branch range".into());}
            // Three C args x0..x2; the original return address becomes x3.
            let words=[0xa9bf7bfdu32,0x910003fd,0xaa1e03e3,0x94000000|(((delta/4) as i64 as u32)&0x03ffffff),0xa8c17bfd,0xd65f03c0];
            let bytes=words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
            self.context.verify_sdk_code(entry,bytes.len(),&format!("{:x}",sha2::Sha256::digest(&bytes)))?;
        }
        if let Some((lease,imports))=self.module.get() {
            lease.before_effect()?;
            for row in imports {if row.selected_address()!=row.replacement{return Err("Reference HID import changed".into());}}
        }
        Ok(base)
    }
    fn registration(&self,callback:Option<Callback>,return_pc:usize,input:bool)->Result<(),String> {
        let base=self.code()?;
        let (caller,entry)=if input{(0xf86c4,0xf9eb0)}else{(0xf86d8,0xfa138)};
        if base.checked_add(caller)!=Some(return_pc)||callback.map(|f|f as usize)!=base.checked_add(entry) {
            return Err("Reference HID registration caller/callback differs".into());
        }
        Ok(())
    }
    fn owner(&self,user:usize)->Result<Arc<crate::workspace::factory::ReferenceInputChildCustody>,String> {
        self.code()?;
        let child=self.child.get().ok_or("Reference HID is pending owner publication")?.clone();
        child.verify_reference_binding(&self.admission,&self.model)?;
        if child.kind()!=crate::workspace::factory::ReferenceInputChildKind::HyperKey||child.pointer_on_creator()?!=user {
            return Err("Reference HID userdata differs from genuine held HyperKey".into());
        }
        child.verify_held_epoch()?;Ok(child)
    }
}
pub struct Context {
    life: Arc<dyn AliveLease>,
    thread: ThreadId,
    nonce: u64,
    loop_: Loop,
    input_codes: BTreeSet<usize>,
    remove_codes: BTreeSet<usize>,
    retired: AtomicBool,
    in_flight: AtomicUsize,
    faults: Mutex<Faults>,
    native: Option<NativeBinding>,
    pending_owner: Mutex<Option<Arc<NativeInputOwnerTicket>>>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference: Option<ReferenceBinding>,
}
static NEXT_OWNER: AtomicU64 = AtomicU64::new(0);
impl Context {
    pub fn new(
        life: Arc<dyn AliveLease>,
        loop_: Loop,
        input_codes: BTreeSet<usize>,
        remove_codes: BTreeSet<usize>,
    ) -> Result<Arc<Self>, String> {
        life.before_effect()?;
        if !life.is_alive()
            || loop_.runloop == 0
            || loop_.mode == 0
            || input_codes.is_empty()
            || remove_codes.is_empty()
            || input_codes.len() > 64
            || remove_codes.len() > 64
            || input_codes.contains(&0)
            || remove_codes.contains(&0)
        {
            return Err("invalid own HID context/callback/loop".into());
        }
        types()?;
        let _ = entries();
        let nonce = NEXT_OWNER
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "own HID owner identities exhausted")?
            .checked_add(1)
            .ok_or("own HID owner overflow")?;
        Ok(Arc::new(Self {
            life,
            thread: thread::current().id(),
            nonce,
            loop_,
            input_codes,
            remove_codes,
            retired: AtomicBool::new(false),
            in_flight: AtomicUsize::new(0),
            faults: Mutex::new(Faults {
                open: None,
                close: None,
                last: None,
            }),
            native: None,
            pending_owner: Mutex::new(None),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            reference:None,
        }))
    }
    pub fn for_native_input(context: ModuleContext, admission: &Arc<NativeInputAdmission>, loop_: Loop) -> Result<Arc<Self>, String> {
        let input = admission.hid_input();
        let removed = admission.hid_removed();
        input.verify(&context)?;
        removed.verify(&context)?;
        let input_code = input.hid_handler()?;
        let remove_code = removed.hid_handler()?;
        let installation = admission.reserve_installation(NativeInputContextKind::Hid, &context)?;
        let mut owner = Self::new(context.life.clone(), loop_, BTreeSet::from([input_code as usize]), BTreeSet::from([remove_code as usize]))?;
        Arc::get_mut(&mut owner).ok_or("HID candidate was shared")?.native = Some(NativeBinding { context, installation, input, removed });
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
        self.check_pure()
    }
    fn check_pure(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("retired/wrong-thread own HID context".into());
        }
        Ok(())
    }
    fn check_life(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("retired/wrong-thread own HID context".into());
        }
        let live = catch_unwind(AssertUnwindSafe(|| {
            self.life.before_effect().is_ok() && self.life.is_alive()
        }))
        .unwrap_or(false);
        if !live {
            self.retired.store(true, Ordering::Release);
            return Err("own HID guardian retired".into());
        }
        // A dynamic lifecycle callback can retire this owner and still return
        // success. Do not admit any work on the pre-callback observation.
        if self.retired.load(Ordering::Acquire) || self.thread != thread::current().id() {
            return Err("own HID authority changed during lifecycle check".into());
        }
        Ok(())
    }
    fn manager(&self, pointer: Ptr) -> Option<Arc<Manager>> {
        let data = entries().lock().ok()?;
        let entry = data.get(&(pointer as usize))?;
        if entry.owner != self.nonce {
            return None;
        }
        if let Payload::Manager(manager) = &entry.payload {
            Some(manager.clone())
        } else {
            None
        }
    }
    fn error(&self) {
        if let Ok(mut f) = self.faults.lock() {
            f.last = Some(ERROR);
        }
    }
    pub fn faults(&self, open: Option<i32>, close: Option<i32>) -> Result<(), String> {
        self.check()?;
        if open == Some(0) || close == Some(0) {
            return Err("success is not an HID fault".into());
        }
        let mut f = self.faults.lock().map_err(|_| "HID faults poisoned")?;
        f.open = open;
        f.close = close;
        Ok(())
    }
    pub fn last_error(&self) -> Result<Option<i32>, String> {
        self.check()?;
        Ok(self
            .faults
            .lock()
            .map_err(|_| "HID faults poisoned")?
            .last
            .take())
    }
    /// Synchronous own incoming value. All callback/query/drop reentry happens
    /// without either metadata or manager-state lock. Keep the sender CF object
    /// alive even if the callback releases the caller's last manager reference.
    /// # Safety
    /// `manager` must remain a live own CF manager while the call acquires its
    /// sender retain. Only a pinned, owner-thread factory result is admissible.
    pub unsafe fn emit(
        &self,
        manager: Ptr,
        page: u32,
        usage: u32,
        integer: isize,
    ) -> Result<(), String> {
        self.check()?;
        let m = self.manager(manager).ok_or("unknown own HID manager")?;
        let sender = Token(
            NonNull::new(unsafe { CFRetain(manager).cast_mut() }).ok_or("own HID sender absent")?,
        );
        let callback = {
            let s = m.state.lock().map_err(|_| "HID state poisoned")?;
            if !s.open || !s.scheduled || !s.device_match || !s.input_valid || !s.present {
                return Err("own HID route inactive".into());
            }
            if s.input_page.is_some_and(|p| p != page) {
                return Ok(());
            }
            s.input.clone().ok_or("own HID callback absent")?
        };
        if self.metadata_count()? > 8190 {
            return Err("own HID value metadata bound exceeded".into());
        }
        let element = token(
            2,
            self.nonce,
            Payload::Element {
                device: m.device.clone(),
                page,
                usage,
            },
        )?;
        let value = token(3, self.nonce, Payload::Value { element, integer })?;
        self.call_registered(m, callback, Some(page), sender, value)
    }
    /// # Safety
    /// Same pinned own CF manager contract as `emit`.
    pub unsafe fn remove_device(&self, manager: Ptr) -> Result<(), String> {
        self.check()?;
        let m = self.manager(manager).ok_or("unknown own HID manager")?;
        let sender = Token(
            NonNull::new(unsafe { CFRetain(manager).cast_mut() }).ok_or("own HID sender absent")?,
        );
        let callback = {
            let s = m.state.lock().map_err(|_| "HID state poisoned")?;
            if !s.open || !s.scheduled || !s.present {
                return Err("own HID removal route inactive".into());
            }
            s.remove.clone().ok_or("own HID removal callback absent")?
        };
        let device = m.device.clone();
        self.call_registered(m, callback, None, sender, device)
    }
    fn registration_current(
        &self, manager: &Manager, callback: &Registered, input: bool, delivery_page: Option<u32>,
    ) -> Result<(), String> {
        let state = manager.state.lock().map_err(|_| "HID state poisoned")?;
        Self::check_registration(&state, callback, input, delivery_page)
    }
    fn check_registration(
        state: &State, callback: &Registered, input: bool, delivery_page: Option<u32>,
    ) -> Result<(), String> {
        let current = if input { &state.input } else { &state.remove };
        if !state.open || !state.scheduled || current.as_ref().is_none_or(|c| c.generation != callback.generation) {
            return Err("HID callback registration retired or replaced".into());
        }
        if let Some(page) = delivery_page {
            if !state.present || !state.device_match || !state.input_valid
                || state.input_page.is_some_and(|matched| matched != page)
            {
                return Err("HID input route changed before delivery".into());
            }
        }
        Ok(())
    }
    fn begin_removal(
        manager: &Manager, callback: &Registered, before_commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut state = manager.state.lock().map_err(|_| "HID state poisoned")?;
        Self::check_registration(&state, callback, false, None)?;
        if !state.present {
            return Err("HID device already removed".into());
        }
        // Only a callback-free final fence can run under the state lock.
        before_commit()?;
        state.present = false;
        Ok(())
    }
    fn call_registered(&self, manager: Arc<Manager>, callback: Registered, page: Option<u32>, sender: Token, value: Token) -> Result<(), String> {
        let input = page.is_some();
        self.in_flight.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < 32).then(|| count + 1)
        }).map_err(|_| "own HID callback depth exceeded")?;
        struct Delivery<'a>(&'a AtomicUsize);
        impl Drop for Delivery<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _delivery = Delivery(&self.in_flight);
        // Drop borrowed callback resources before releasing the activity guard,
        // including error paths during post-callback source checks.
        let (manager, callback, sender, value) = (manager, callback, sender, value);
        self.check()?;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&self.reference {
            let child=reference.owner(callback.user)?;
            let sender_address = sender.ptr() as usize;
            let live = |delivery_page| -> Result<(), String> {
                if child.hid_manager_on_creator()? != sender_address {
                    return Err("Reference Swift HyperKey manager no longer matches the delivery".into());
                }
                self.check_pure()?;
                child.verify_held_epoch()?;
                if !self.manager(sender_address as Ptr)
                    .is_some_and(|current| Arc::ptr_eq(&current, &manager))
                {
                    return Err("Reference HID sender no longer names the same manager".into());
                }
                self.registration_current(&manager, &callback, input, delivery_page)?;
                super::reference_runtime::verify_published_input_child(&reference.admission, &child)
            };
            self.check()?;
            reference.owner(callback.user)?;
            let caps = if input { child.caps_connection_on_creator()? } else { 0 };
            live(page)?;
            if !input {
                Self::begin_removal(&manager, &callback, || {
                    self.check_pure()?;
                    super::reference_runtime::verify_published_input_child(&reference.admission, &child)
                })?;
            }
            unsafe{(callback.callback)(callback.user as Ptr,0,sender.ptr(),value.ptr());}
            let stopped_during_input = input && caps != 0 && child.hid_manager_slot_on_creator()? == 0;
            let after = || -> Result<(), String> {
                if !stopped_during_input {
                    return live(None);
                }
                // HyperKey stops the actual input path after a failed Caps
                // write. Require completed teardown of the same resources.
                child.verify_hyper_resources_cleared()?;
                super::input_events::verify_reference_keyboard_taps_stopped(&child, callback.user, false)?;
                super::input_permissions_caps::verify_reference_closed_caps_connection(
                    &reference.admission, &reference.model, Some(caps),
                )?;
                self.check_pure()?;
                {
                    let state = manager.state.lock().map_err(|_| "HID manager state poisoned")?;
                    if state.open || state.scheduled {
                        return Err("Reference HID callback did not close and unschedule its manager".into());
                    }
                }
                super::reference_runtime::verify_published_input_child(&reference.admission, &child)
            };
            after()?;
            // Keep original userdata custody through dynamic own CF finalizers
            // and final guardian checks, with no lock across original code.
            drop(value);drop(sender);
            self.check()?;
            after()?;
            return Ok(());
        }
        if let Some(native) = &self.native {
            let witness = if input { &native.input } else { &native.removed };
            let owner = callback.owner.as_ref().ok_or("Native HID callback has no owner ticket")?;
            if !owner.matches_callback(witness) { return Err("Native HID callback owner/code differs".into()); }
            let entry = owner.enter()?;
            if entry.userdata()? != callback.user { return Err("Native HID userdata differs from live owner".into()); }
            let guard = witness.before_callback(&native.context);
            let result = (|| {
                guard.as_ref().map_err(Clone::clone)?;
                native.installation.pure_after()?;
                self.check_pure()?;
                if entry.userdata()? != callback.user { return Err("Native HID owner retired before callback".into()); }
                if input {
                    self.registration_current(&manager, &callback, input, page)?;
                } else {
                    Self::begin_removal(&manager, &callback, || self.check_pure())?;
                }
                unsafe { (callback.callback)(callback.user as Ptr, 0, sender.ptr(), value.ptr()); }
                self.registration_current(&manager, &callback, input, None)
            })();
            // Own CF finalization is dynamic too; retain Native userdata until
            // it and the final guardian verification have both completed.
            drop(value);
            drop(sender);
            drop(manager);
            let checked = if result.is_ok() { self.check() } else { Ok(()) };
            let source = guard.as_ref().map_or(Ok(()), |guard| guard.pure_after_callback());
            let lifetime = entry.pure_after_callback();
            result?;
            checked?;
            native.installation.pure_after()?;
            self.check_pure()?;
            source?;
            lifetime
        } else {
            if input {
                self.registration_current(&manager, &callback, input, page)?;
            } else {
                Self::begin_removal(&manager, &callback, || self.check_pure())?;
            }
            unsafe { (callback.callback)(callback.user as Ptr, 0, sender.ptr(), value.ptr()); }
            self.check()
        }
    }
    pub fn metadata_count(&self) -> Result<usize, String> {
        self.check()?;
        Ok(entries()
            .lock()
            .map_err(|_| "HID metadata poisoned")?
            .values()
            .filter(|e| e.owner == self.nonce)
            .count())
    }
}
static CURRENT: Mutex<Option<Arc<Context>>> = Mutex::new(None);
static PUBLISHED: AtomicBool = AtomicBool::new(false);
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
static REFERENCE_ATTEMPT:AtomicBool=AtomicBool::new(false);
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_roster()->BTreeSet<super::SourceCallsite> {
    BTreeSet::from([super::SourceCallsite{
        file:"reference-0.8.26/Sources/RuycastApp/HyperKey.swift",
        source_sha256:"a60dff19c9ab210fd8dc3ae564f962a2708b1e32a2f69663ff2c48e6d70b1544".into(),
        operation:"HID input and device-removal registration, owned values and retirement",
    }])
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) unsafe fn initialize_reference(
    admission:Arc<super::core_preentry::ReferenceAdmission>,
    context:Arc<super::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>,
)->Result<(Arc<Context>,super::ModuleAttestation),String> {
    if REFERENCE_ATTEMPT.swap(true,Ordering::AcqRel){return Err("Reference HID construction attempt already claimed".into());}
    let module=super::bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;
    admission.verify_input_context(&module)?;
    let main_loop=objc2_core_foundation::CFRunLoop::main().ok_or("Reference main CF loop absent")?;
    let mode=unsafe{objc2_core_foundation::kCFRunLoopCommonModes}.ok_or("Reference common CF mode absent")?;
    let loop_=Loop{runloop:&*main_loop as *const _ as usize,mode:mode as *const _ as usize};
    let reference=ReferenceBinding{admission,context,model,child:OnceLock::new(),module:OnceLock::new(),main_loop:loop_.runloop};
    let base=reference.code()?;
    types()?;let _=entries();
    let nonce=NEXT_OWNER.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_add(1))
        .map_err(|_|"Reference HID identities exhausted")?.checked_add(1).ok_or("Reference HID identity overflow")?;
    let candidate=Arc::new(Context{
        life:module.life.clone(),thread:thread::current().id(),nonce,loop_,
        input_codes:BTreeSet::from([base+0xf9eb0]),remove_codes:BTreeSet::from([base+0xfa138]),
        retired:AtomicBool::new(false),in_flight:AtomicUsize::new(0),faults:Mutex::new(Faults{open:None,close:None,last:None}),
        native:None,pending_owner:Mutex::new(None),reference:Some(reference),
    });
    install(candidate.clone())?;
    struct Publication{context:Arc<Context>,complete:bool}
    impl Drop for Publication{fn drop(&mut self){if !self.complete{rollback_installation(&self.context);}}}
    let mut publication=Publication{context:candidate.clone(),complete:false};
    struct InstallGuard<'a>(&'a Context);
    impl AliveLease for InstallGuard<'_>{
        fn is_alive(&self)->bool{self.before_effect().is_ok()}
        fn before_effect(&self)->Result<(),String>{self.0.check()}
    }
    let lease=module.lease("reference-hid")?;
    // Exact retained Reference role; no synthetic type-ID anchors are added.
    // All four general/native type-ID imports must remain absent from Main.
    let specs=import_specs();
    let role=[c"IOHIDManagerCreate",c"IOHIDManagerOpen",c"IOHIDManagerClose",
        c"IOHIDManagerSetDeviceMatching",c"IOHIDManagerSetInputValueMatching",
        c"IOHIDManagerRegisterInputValueCallback",c"IOHIDManagerRegisterDeviceRemovalCallback",
        c"IOHIDManagerScheduleWithRunLoop",c"IOHIDManagerUnscheduleFromRunLoop",
        c"IOHIDValueGetElement",c"IOHIDValueGetIntegerValue",c"IOHIDElementGetUsagePage",
        c"IOHIDElementGetUsage",c"IOHIDElementGetDevice"];
    let selected=specs.iter().filter(|spec|role.contains(&spec.symbol)).copied().collect::<Vec<_>>();
    if selected.len()!=role.len(){return Err("Reference HID fixed import role differs".into());}
    let imports=super::import_patch::install_exact_presence(&selected,&specs,&InstallGuard(&candidate))?;
    candidate.reference.as_ref().ok_or("Reference HID source absent")?.module.set((lease,imports))
        .map_err(|_|"Reference HID module already installed")?;
    let result=reference_attestation()?;
    publication.complete=true;Ok((candidate,result))
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_attestation()->Result<super::ModuleAttestation,String> {
    let context=CURRENT.lock().map_err(|_|"HID owner poisoned")?.clone().ok_or("Reference HID absent")?;
    let source=context.reference.as_ref().ok_or("HID context is not Reference source")?;
    let (lease,imports)=source.module.get().ok_or("Reference HID imports absent")?;
    let result=super::ModuleAttestation{module:"reference-hid",scopes:BTreeSet::new(),callsites:reference_roster(),
        imports:imports.clone(),objc:vec![],handles:vec![lease.clone()],workspace_factories:None};
    verify_reference_attestation(&result)?;Ok(result)
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_attestation(module:&super::ModuleAttestation)->Result<(),String> {
    if module.module!="reference-hid"||!module.scopes.is_empty()||module.callsites!=reference_roster()
        ||!module.objc.is_empty()||module.workspace_factories.is_some()||module.handles.len()!=1 {
        return Err("Reference HID module envelope differs".into());
    }
    let context=CURRENT.lock().map_err(|_|"HID owner poisoned")?.clone().ok_or("Reference HID absent")?;
    // Registry inspection on a worker carries no callback or event authority.
    if context.thread == thread::current().id() {
        context.check()?;
    }
    let source=context.reference.as_ref().ok_or("Reference HID source absent")?;
    let (lease,imports)=source.module.get().ok_or("Reference HID imports absent")?;
    if context.loop_.runloop!=source.main_loop
        ||module.handles[0].namespace!=lease.namespace||module.handles[0].root.path()!=lease.root.path()
        ||!Arc::ptr_eq(&module.handles[0].life,&lease.life)||module.imports.len()!=imports.len(){return Err("Reference HID module issuer differs".into());}
    lease.before_effect()?;
    let mut symbols=BTreeSet::new();
    for row in &module.imports {
        let expected=imports.iter().find(|expected|expected.symbol==row.symbol).ok_or("Foreign Reference HID import")?;
        if !symbols.insert(row.symbol)||row.original!=expected.original||row.replacement!=expected.replacement
            ||row.cell_address()!=expected.cell_address()||row.selected_address()!=row.replacement {
            return Err("Reference HID import/source differs".into());
        }
    }
    let current = CURRENT.lock().map_err(|_|"HID owner poisoned")?;
    if !PUBLISHED.load(Ordering::Acquire)
        || context.retired.load(Ordering::Acquire)
        || !current.as_ref().is_some_and(|published| Arc::ptr_eq(published, &context))
    {
        return Err("Reference HID publication changed during attestation".into());
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn publish_reference_owner(
    admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>,
    child:Arc<crate::workspace::factory::ReferenceInputChildCustody>,
)->Result<(),String> {
    let context=CURRENT.lock().map_err(|_|"HID owner poisoned")?.clone().ok_or("Reference HID absent")?;
    let result=(|| {
        context.check()?;
        let source=context.reference.as_ref().ok_or("HID has no Reference binding")?;
        if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model)||source.child.get().is_some()
            ||child.kind()!=crate::workspace::factory::ReferenceInputChildKind::HyperKey {
            return Err("Reference HID publication issuer/replay differs".into());
        }
        child.verify_reference_binding(admission,model)?;
        let user=child.pointer_on_creator()?;
        context.check()?;
        let held_manager = child.hid_manager_on_creator()?;
        // The private Reference launch enables Hyper. Require both source
        // registrations on the same open manager before admitting its owner.
        // Hold only owned metadata/manager locks, never a dynamic source guard.
        let data=entries().lock().map_err(|_|"HID metadata poisoned")?;
        let mut live_managers = 0;
        for (&address, entry) in data.iter().filter(|(_, entry)| entry.owner == context.nonce) {
            if let Payload::Manager(manager)=&entry.payload {
                let state=manager.state.lock().map_err(|_|"HID state poisoned")?;
                for (callback,codes) in [(&state.input,&context.input_codes),(&state.remove,&context.remove_codes)] {
                    if let Some(callback)=callback {
                        if callback.user!=user||!codes.contains(&(callback.callback as usize)) {
                            return Err("Reference HID pending userdata differs from actual child".into());
                        }
                    }
                }
                if state.open {
                    if address != held_manager {
                        return Err("Reference HID live manager differs from HyperKey's retained manager".into());
                    }
                    if !state.reference_keyboard_ready() {
                        return Err("Reference HID live manager lacks exact keyboard matching or scheduled registrations".into());
                    }
                    live_managers += 1;
                }
            }
        }
        if live_managers != 1 {
            return Err("Reference HID requires exactly one registered live manager".into());
        }
        drop(data);
        let current = CURRENT.lock().map_err(|_| "HID owner poisoned")?;
        if !current.as_ref().is_some_and(|selected| Arc::ptr_eq(selected, &context)) {
            return Err("Reference HID owner changed during publication".into());
        }
        context.check_pure()?;
        let published = source.child.set(child);
        drop(current);
        published.map_err(|_|"Reference HID child already published")?;
        Ok(())
    })();
    if result.is_err(){context.retired.store(true,Ordering::Release);}
    result
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn observe_reference_manager_stop(pointer: usize, owner: usize) -> Result<impl FnOnce() -> Result<(), String>, String> {
    let manager = with(|context| {
        if context.in_flight.load(Ordering::Acquire) != 0 {
            return None;
        }
        context.manager(pointer as Ptr)
    })
        .ok_or("Reference HyperKey manager is not owned")?;
    {
        let state = manager.state.lock().map_err(|_| "HID manager state poisoned")?;
        if !state.open || !state.reference_keyboard_ready()
            || !state.input.as_ref().is_some_and(|callback| callback.user == owner)
            || !state.remove.as_ref().is_some_and(|callback| callback.user == owner)
        {
            return Err("Reference HyperKey manager is not live before stop".into());
        }
    }
    Ok(move || {
        let state = manager.state.lock().map_err(|_| "HID manager state poisoned")?;
        if state.open || state.scheduled {
            return Err("Reference HyperKey manager did not close and unschedule".into());
        }
        Ok(())
    })
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_manager_stopped(
    child: &crate::workspace::factory::ReferenceInputChildCustody,
    before_publication: bool,
) -> Result<(), String> {
    let current = CURRENT.lock().map_err(|_| "HID owner poisoned")?;
    let context = current.as_ref().ok_or("Reference HID owner absent")?;
    let source = context.reference.as_ref().ok_or("Reference HID source absent")?;
    context.check_pure()?;
    let same_owner = match source.child.get() {
        Some(held) => !before_publication && std::ptr::eq(held.as_ref(), child),
        None => before_publication,
    };
    if !same_owner
        || context.in_flight.load(Ordering::Acquire) != 0
    {
        return Err("Reference HID stopped owner differs or is delivering".into());
    }
    let data = entries().lock().map_err(|_| "HID metadata poisoned")?;
    for entry in data.values().filter(|entry| entry.owner == context.nonce) {
        if let Payload::Manager(manager) = &entry.payload {
            let state = manager.state.lock().map_err(|_| "HID manager state poisoned")?;
            if state.open || state.scheduled {
                return Err("Reference HyperKey still has an open or scheduled HID manager".into());
            }
        }
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn retire_reference(admission:&Arc<super::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->Result<(),String> {
    let context=CURRENT.lock().map_err(|_|"HID owner poisoned")?.clone();
    match context {
        Some(context)=>{
            let source=context.reference.as_ref().ok_or("HID teardown found another source family")?;
            if !Arc::ptr_eq(&source.admission,admission)||!Arc::ptr_eq(&source.model,model){return Err("HID teardown found another Reference owner".into());}
            retire(&context)
        }
        None=>Err("Reference HID teardown has no installed owner to drain".into()),
    }
}
pub fn install(context: Arc<Context>) -> Result<(), String> {
    context.check_candidate()?;
    {
        let mut slot = CURRENT.lock().map_err(|_| "HID owner poisoned")?;
        if slot.is_some() { return Err("HID owner already installed".into()); }
        *slot = Some(context.clone());
    }
    if let Some(native) = &context.native {
        if let Err(error) = native.installation.commit_installed() { rollback_installation(&context); return Err(error); }
    }
    PUBLISHED.store(true, Ordering::Release);
    if let Err(error) = context.check() { rollback_installation(&context); return Err(error); }
    Ok(())
}
fn rollback_installation(expected: &Arc<Context>) {
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    if let Ok(mut slot) = CURRENT.lock() {
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) { PUBLISHED.store(false, Ordering::Release); *slot = None; }
    }
}
pub fn retire(expected: &Arc<Context>) -> Result<(), String> {
    let mut slot = CURRENT.lock().map_err(|_| "HID owner poisoned")?;
    if !slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
        return Err("stale HID owner".into());
    }
    PUBLISHED.store(false, Ordering::Release);
    expected.retired.store(true, Ordering::Release);
    if let Some(native) = &expected.native { native.installation.retire(); }
    *slot = None;
    drop(slot);
    let managers = entries().lock().map(|data| {
        data.values().filter_map(|entry| match &entry.payload {
            Payload::Manager(manager) if entry.owner == expected.nonce => Some(manager.clone()),
            _ => None,
        }).collect::<Vec<_>>()
    }).map_err(|_| "HID metadata poisoned".to_owned());
    let manager_result = managers.and_then(|managers| {
        let mut result = Ok(());
        for manager in managers {
            match manager.state.lock() {
                Ok(mut state) => {
                    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                    if expected.reference.is_some() && (state.open || state.scheduled) {
                        result = Err("Reference HID retired an open or scheduled manager".into());
                    }
                    state.open = false;
                    state.scheduled = false;
                    let callbacks = (state.input.take(), state.remove.take());
                    drop(state);
                    drop(callbacks);
                }
                Err(_) => result = Err("HID manager state poisoned".into()),
            }
        }
        result
    });
    let ticket_result = expected.pending_owner.lock()
        .map(|owner| owner.clone())
        .map_err(|_| "HID owner ticket poisoned".to_owned())
        .and_then(|owner| owner.map_or(Ok(()), |owner| owner.revoke()));
    manager_result.and(ticket_result)?;
    if expected.in_flight.load(Ordering::Acquire) != 0 {
        return Err("HID callback busy; its owner must remain retained".into());
    }
    Ok(())
}
fn native_owner(expected: &NativeInputInstallation) -> Result<Arc<Context>, String> {
    expected.pure_after()?;
    let owner = CURRENT.lock().map_err(|_| "HID owner poisoned")?.as_ref().cloned().ok_or("Native HID owner absent")?;
    let native = owner.native.as_ref().ok_or("HID fixture is not Native Input authority")?;
    if !native.installation.matches(expected) { return Err("Native HID installation differs".into()); }
    owner.check()?;
    expected.pure_after()?;
    if !CURRENT.lock().map_err(|_| "HID owner poisoned")?.as_ref().is_some_and(|c| Arc::ptr_eq(c, &owner)) {
        return Err("Native HID owner changed during verification".into());
    }
    Ok(owner)
}
pub(super) fn verify_owner_installation(expected: &NativeInputInstallation) -> Result<(), String> {
    native_owner(expected).map(|_| ())
}
pub(super) fn owner_ticket(expected: &NativeInputInstallation, slot: u64, generation: u64) -> Result<Arc<NativeInputOwnerTicket>, String> {
    let owner = native_owner(expected)?;
    let ticket = owner.pending_owner.lock().map_err(|_| "HID owner ticket poisoned")?.as_ref()
        .filter(|ticket| ticket.slot() == slot && ticket.generation() == generation).cloned().ok_or("Native HID ticket key absent")?;
    owner.check()?;
    expected.pure_after()?;
    Ok(ticket)
}
pub(super) fn publish_owner_ticket(expected: &NativeInputInstallation, ticket: Arc<NativeInputOwnerTicket>) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let native = owner.native.as_ref().ok_or("Native HID binding absent")?;
    if ticket.family() != NativeInputOwnerFamily::Hid || !ticket.matches_callback(&native.input) || !ticket.matches_callback(&native.removed) {
        return Err("Native HID ticket family/code differs".into());
    }
    let entry = ticket.enter_registration()?;
    if entry.userdata()? == 0 { return Err("Native HID userdata absent".into()); }
    {
        let mut pending = owner.pending_owner.lock().map_err(|_| "HID owner ticket poisoned")?;
        if pending.is_some() { return Err("Native HID owner already published".into()); }
        *pending = Some(ticket.clone());
    }
    let result = owner.check().and_then(|_| entry.pure_after_callback());
    if result.is_err() {
        let _ = ticket.revoke();
        if let Ok(mut pending) = owner.pending_owner.lock() {
            if pending.as_ref().is_some_and(|p| Arc::ptr_eq(p, &ticket)) { *pending = None; }
        }
    }
    result
}
pub(super) fn revoke_owner_ticket(expected: &NativeInputInstallation, ticket: &NativeInputOwnerTicket) -> Result<(), String> {
    let owner = native_owner(expected)?;
    let pending = owner.pending_owner.lock().map_err(|_| "HID owner ticket poisoned")?.as_ref().cloned().ok_or("Native HID ticket absent")?;
    if !std::ptr::eq(&*pending, ticket) { return Err("Native HID ticket identity differs".into()); }
    ticket.revoke()?;
    let mut slot = owner.pending_owner.lock().map_err(|_| "HID owner ticket poisoned")?;
    if slot.as_ref().is_some_and(|p| std::ptr::eq(&**p, ticket)) { *slot = None; }
    Ok(())
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
unsafe extern "C" fn create(allocator: *const c_void, options: u32) -> Ptr {
    with(|c| {
        if !allocator.is_null() || options != 0 {
            return None;
        }
        if c.metadata_count().ok()? >= 8192 {
            return None;
        }
        #[cfg(not(all(rcf1_reference_sdk,not(test),target_arch="aarch64")))]
        let device_match = true;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        let device_match = c.reference.is_none();
        let device = token(1, c.nonce, Payload::Device).ok()?;
        let manager = Arc::new(Manager {
            device,
            state: Mutex::new(State {
                open: false,
                scheduled: false,
                device_match,
                input_valid: true,
                input_page: None,
                present: true,
                input: None,
                remove: None,
            input_generation: 0,
            remove_generation: 0,
            }),
        });
        let object = token(0, c.nonce, Payload::Manager(manager)).ok()?;
        Some(object.retained())
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn open(manager: Ptr, options: u32) -> i32 {
    with(|c| {
        if options != 0 {
            return Some(ERROR);
        }
        let m = c.manager(manager)?;
        if let Some(e) = c.faults.lock().ok()?.open {
            return Some(e);
        }
        let mut state = m.state.lock().ok()?;
        c.check_pure().ok()?;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if c.reference.is_some() && !state.reference_keyboard_ready() {
            c.retired.store(true, Ordering::Release);
            return Some(ERROR);
        }
        state.open = true;
        Some(0)
    })
    .unwrap_or(ERROR)
}
unsafe extern "C" fn close(manager: Ptr, options: u32) -> i32 {
    with(|c| {
        if options != 0 {
            return Some(ERROR);
        }
        let m = c.manager(manager)?;
        if let Some(e) = c.faults.lock().ok()?.close {
            return Some(e);
        }
        m.state.lock().ok()?.open = false;
        Some(0)
    })
    .unwrap_or(ERROR)
}
fn number(dictionary: *const c_void, key: &CStr) -> Option<u32> {
    let text = unsafe { CFStringCreateWithCString(std::ptr::null_mut(), key.as_ptr(), 0x08000100) };
    if text.is_null() {
        return None;
    }
    let value = unsafe { CFDictionaryGetValue(dictionary, text) };
    unsafe {
        CFRelease(text);
    }
    let mut number = 0i64;
    if value.is_null()
        || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() }
        || unsafe { CFNumberGetValue(value, 4, std::ptr::from_mut(&mut number).cast()) } == 0
    {
        return None;
    }
    u32::try_from(number).ok()
}
fn select(manager: Ptr, dictionary: *const c_void, input: bool) {
    let _ = with(|c| {
        let m = c.manager(manager)?;
        let valid = dictionary.is_null()
            || unsafe { CFGetTypeID(dictionary) } == unsafe { CFDictionaryGetTypeID() };
        if input {
            let page = if dictionary.is_null() {
                Some(None)
            } else if valid && unsafe { CFDictionaryGetCount(dictionary) } == 1 {
                number(dictionary, c"UsagePage").map(Some)
            } else {
                None
            };
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            if c.reference.is_some() && page != Some(Some(7)) {
                c.retired.store(true, Ordering::Release);
                return None;
            }
            if page.is_none() {
                c.error();
            }
            let mut s = m.state.lock().ok()?;
            c.check_pure().ok()?;
            s.input_valid = page.is_some();
            if let Some(page) = page {
                s.input_page = page;
            }
        } else {
            let matches = dictionary.is_null()
                || (valid
                    && (unsafe { CFDictionaryGetCount(dictionary) }) == 2
                    && number(dictionary, c"DeviceUsagePage") == Some(1)
                    && number(dictionary, c"DeviceUsage") == Some(6));
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            if c.reference.is_some() && (dictionary.is_null() || !matches) {
                c.retired.store(true, Ordering::Release);
                return None;
            }
            if !valid {
                c.error();
            }
            let mut state = m.state.lock().ok()?;
            c.check_pure().ok()?;
            state.device_match = matches;
        }
        Some(())
    });
}
unsafe extern "C" fn device_matching(manager: Ptr, dictionary: *const c_void) {
    select(manager, dictionary, false)
}
unsafe extern "C" fn input_matching(manager: Ptr, dictionary: *const c_void) {
    select(manager, dictionary, true)
}
fn register(manager: Ptr, callback: Option<Callback>, user: Ptr, input: bool, return_pc:usize) {
    let _ = with(|c| {
        let m = c.manager(manager)?;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(reference)=&c.reference {
            if reference.registration(callback,return_pc,input).is_err(){c.retired.store(true,Ordering::Release);c.error();return None;}
            if reference.child.get().is_some()&&reference.owner(user as usize).is_err(){c.retired.store(true,Ordering::Release);c.error();return None;}
            // Source qualification may reenter retirement. No raw userdata is
            // read or retained while pending; publication binds the real child.
            c.check_pure().ok()?;
        }
        let _=return_pc;
        let allowed = match callback {
            None => true,
            Some(callback) => {
                if input {
                    c.input_codes.contains(&(callback as usize))
                } else {
                    c.remove_codes.contains(&(callback as usize))
                }
            }
        };
        let owner = if allowed && callback.is_some() {
            if let Some(native) = &c.native {
            let owner = c.pending_owner.lock().ok()?.as_ref()?.clone();
            let witness = if input { &native.input } else { &native.removed };
            if !owner.matches_callback(witness) { c.error(); return None; }
            let entry = owner.enter().ok()?;
            if entry.userdata().ok()? != user as usize { c.error(); return None; }
            native.installation.pure_after().ok()?;
            entry.pure_after_callback().ok()?;
            Some(owner)
            } else { None }
        } else { None };
        let mut s = m.state.lock().ok()?;
        c.check_pure().ok()?;
        let generation = if input { &mut s.input_generation } else { &mut s.remove_generation };
        *generation = generation.checked_add(1)?;
        let registered = if allowed {
            callback.map(|callback| Registered { callback, user: user as usize, owner, generation: *generation })
        } else {
            c.error();
            None
        };
        if input {
            s.input = registered
        } else {
            s.remove = registered
        }
        Some(())
    });
}
unsafe extern "C" fn register_input_checked(manager: Ptr, callback: Option<Callback>, user: Ptr,return_pc:usize) {
    register(manager, callback, user, true,return_pc)
}
unsafe extern "C" fn register_remove_checked(manager: Ptr, callback: Option<Callback>, user: Ptr,return_pc:usize) {
    register(manager, callback, user, false,return_pc)
}
#[cfg(target_arch="aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn register_input(_manager:Ptr,_callback:Option<Callback>,_user:Ptr) {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!","mov x29,sp","mov x3,x30","bl {checked}","ldp x29,x30,[sp],#16","ret",checked=sym register_input_checked);
}
#[cfg(target_arch="aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn register_remove(_manager:Ptr,_callback:Option<Callback>,_user:Ptr) {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!","mov x29,sp","mov x3,x30","bl {checked}","ldp x29,x30,[sp],#16","ret",checked=sym register_remove_checked);
}
#[cfg(not(target_arch="aarch64"))]
unsafe extern "C" fn register_input(manager:Ptr,callback:Option<Callback>,user:Ptr){unsafe{register_input_checked(manager,callback,user,0)}}
#[cfg(not(target_arch="aarch64"))]
unsafe extern "C" fn register_remove(manager:Ptr,callback:Option<Callback>,user:Ptr){unsafe{register_remove_checked(manager,callback,user,0)}}
fn schedule_(manager: Ptr, runloop: Ptr, mode: *const c_void, scheduled: bool) {
    let _ = with(|c| {
        let m = c.manager(manager)?;
        if runloop as usize != c.loop_.runloop || mode as usize != c.loop_.mode {
            c.error();
            return None;
        }
        let mut state = m.state.lock().ok()?;
        c.check_pure().ok()?;
        state.scheduled = scheduled;
        Some(())
    });
}
unsafe extern "C" fn schedule(manager: Ptr, runloop: Ptr, mode: *const c_void) {
    schedule_(manager, runloop, mode, true)
}
unsafe extern "C" fn unschedule(manager: Ptr, runloop: Ptr, mode: *const c_void) {
    schedule_(manager, runloop, mode, false)
}
fn query<T>(pointer: Ptr, f: impl FnOnce(&Payload) -> Option<T>) -> Option<T> {
    with(|c| {
        let data = entries().lock().ok()?;
        let entry = data.get(&(pointer as usize))?;
        if entry.owner != c.nonce {
            return None;
        }
        f(&entry.payload)
    })
}
unsafe extern "C" fn element(value: Ptr) -> Ptr {
    query(value, |p| {
        if let Payload::Value { element, .. } = p {
            Some(element.ptr())
        } else {
            None
        }
    })
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn integer(value: Ptr) -> isize {
    query(value, |p| {
        if let Payload::Value { integer, .. } = p {
            Some(*integer)
        } else {
            None
        }
    })
    .unwrap_or(0)
}
unsafe extern "C" fn usage_page(element: Ptr) -> u32 {
    query(element, |p| {
        if let Payload::Element { page, .. } = p {
            Some(*page)
        } else {
            None
        }
    })
    .unwrap_or(0)
}
unsafe extern "C" fn usage(element: Ptr) -> u32 {
    query(element, |p| {
        if let Payload::Element { usage, .. } = p {
            Some(*usage)
        } else {
            None
        }
    })
    .unwrap_or(0)
}
unsafe extern "C" fn device(element: Ptr) -> Ptr {
    query(element, |p| {
        if let Payload::Element { device, .. } = p {
            Some(device.ptr())
        } else {
            None
        }
    })
    .unwrap_or(std::ptr::null_mut())
}
fn type_(kind: usize) -> usize {
    with(|_| Some(cached()?[kind])).unwrap_or(0)
}
unsafe extern "C" fn manager_type() -> usize {
    type_(0)
}
unsafe extern "C" fn device_type() -> usize {
    type_(1)
}
unsafe extern "C" fn element_type() -> usize {
    type_(2)
}
unsafe extern "C" fn value_type() -> usize {
    type_(3)
}
pub fn replacements() -> [(&'static CStr, usize); 18] {
    [
        (c"IOHIDManagerCreate", create as *const () as usize),
        (c"IOHIDManagerOpen", open as *const () as usize),
        (c"IOHIDManagerClose", close as *const () as usize),
        (
            c"IOHIDManagerSetDeviceMatching",
            device_matching as *const () as usize,
        ),
        (
            c"IOHIDManagerSetInputValueMatching",
            input_matching as *const () as usize,
        ),
        (
            c"IOHIDManagerRegisterInputValueCallback",
            register_input as *const () as usize,
        ),
        (
            c"IOHIDManagerRegisterDeviceRemovalCallback",
            register_remove as *const () as usize,
        ),
        (
            c"IOHIDManagerScheduleWithRunLoop",
            schedule as *const () as usize,
        ),
        (
            c"IOHIDManagerUnscheduleFromRunLoop",
            unschedule as *const () as usize,
        ),
        (c"IOHIDValueGetElement", element as *const () as usize),
        (c"IOHIDValueGetIntegerValue", integer as *const () as usize),
        (
            c"IOHIDElementGetUsagePage",
            usage_page as *const () as usize,
        ),
        (c"IOHIDElementGetUsage", usage as *const () as usize),
        (c"IOHIDElementGetDevice", device as *const () as usize),
        (c"IOHIDManagerGetTypeID", manager_type as *const () as usize),
        (c"IOHIDDeviceGetTypeID", device_type as *const () as usize),
        (c"IOHIDElementGetTypeID", element_type as *const () as usize),
        (c"IOHIDValueGetTypeID", value_type as *const () as usize),
    ]
}
/// Fixed typed system entries. No HID manager/device operation is performed.
pub fn import_specs() -> [super::ImportSpec; 18] {
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> Ptr;
        fn IOHIDManagerOpen(manager: Ptr, options: u32) -> i32;
        fn IOHIDManagerClose(manager: Ptr, options: u32) -> i32;
        fn IOHIDManagerSetDeviceMatching(manager: Ptr, dictionary: *const c_void);
        fn IOHIDManagerSetInputValueMatching(manager: Ptr, dictionary: *const c_void);
        fn IOHIDManagerRegisterInputValueCallback(manager: Ptr, callback: Option<Callback>, user: Ptr);
        fn IOHIDManagerRegisterDeviceRemovalCallback(manager: Ptr, callback: Option<Callback>, user: Ptr);
        fn IOHIDManagerScheduleWithRunLoop(manager: Ptr, runloop: Ptr, mode: *const c_void);
        fn IOHIDManagerUnscheduleFromRunLoop(manager: Ptr, runloop: Ptr, mode: *const c_void);
        fn IOHIDValueGetElement(value: Ptr) -> Ptr;
        fn IOHIDValueGetIntegerValue(value: Ptr) -> isize;
        fn IOHIDElementGetUsagePage(element: Ptr) -> u32;
        fn IOHIDElementGetUsage(element: Ptr) -> u32;
        fn IOHIDElementGetDevice(element: Ptr) -> Ptr;
        fn IOHIDManagerGetTypeID() -> usize;
        fn IOHIDDeviceGetTypeID() -> usize;
        fn IOHIDElementGetTypeID() -> usize;
        fn IOHIDValueGetTypeID() -> usize;
    }
    let original = [IOHIDManagerCreate as *const () as usize, IOHIDManagerOpen as *const () as usize,
        IOHIDManagerClose as *const () as usize, IOHIDManagerSetDeviceMatching as *const () as usize,
        IOHIDManagerSetInputValueMatching as *const () as usize, IOHIDManagerRegisterInputValueCallback as *const () as usize,
        IOHIDManagerRegisterDeviceRemovalCallback as *const () as usize, IOHIDManagerScheduleWithRunLoop as *const () as usize,
        IOHIDManagerUnscheduleFromRunLoop as *const () as usize, IOHIDValueGetElement as *const () as usize,
        IOHIDValueGetIntegerValue as *const () as usize, IOHIDElementGetUsagePage as *const () as usize,
        IOHIDElementGetUsage as *const () as usize, IOHIDElementGetDevice as *const () as usize,
        IOHIDManagerGetTypeID as *const () as usize, IOHIDDeviceGetTypeID as *const () as usize,
        IOHIDElementGetTypeID as *const () as usize, IOHIDValueGetTypeID as *const () as usize];
    let leaves = replacements();
    std::array::from_fn(|i| super::ImportSpec { symbol: leaves[i].0, original: original[i], replacement: leaves[i].1 })
}
/// Actual import observation with the genuine bootstrap lease. Does not create
/// callback/source authority, select a loop, install a Context or grant a scope.
/// The caller still needs its exact input/removal callback and userdata lifetime
/// proof; the returned rows are not evidence of complete source coverage.
pub fn observe_owned_imports(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
) -> Result<(super::owned::OwnedHandleLease, Vec<super::ImportReplacement>), String> {
    let context = super::bootstrap::owned_module_context(life, descriptors)?;
    let lease = context.lease("input-hid")?;
    let imports = super::observe_main_imports(&import_specs())?;
    let confirmed = super::bootstrap::owned_module_context(life, descriptors)?;
    if !Arc::ptr_eq(&lease.life, &confirmed.life) { return Err("HID import issuer changed".into()); }
    lease.before_effect()?;
    if imports.iter().any(|row| row.selected_address() != row.replacement) {
        return Err("HID selected import changed".into());
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
        assert_eq!(specs.len(), 18);
        assert_eq!(specs.iter().map(|s| s.symbol).collect::<std::collections::BTreeSet<_>>().len(), 18);
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
    fn c_registration_entries_deliver_both_owned_hid_routes_and_retire() {
        struct Live;
        impl AliveLease for Live {
            fn is_alive(&self)->bool{true}
            fn before_effect(&self)->Result<(),String>{Ok(())}
        }
        struct Observed{input:AtomicU64,removed:AtomicU64,bad:AtomicBool}
        unsafe extern "C" fn incoming(user:Ptr,result:i32,sender:Ptr,value:Ptr) {
            let seen=unsafe{&*(user as *const Observed)};
            let element=unsafe{element(value)};
            let valid=result==0 && unsafe{CFGetTypeID(sender)}==cached().unwrap()[0]
                && unsafe{usage_page(element)}==7 && unsafe{usage(element)}==57
                && unsafe{integer(value)}==1;
            if !valid{seen.bad.store(true,Ordering::Release);}
            seen.input.fetch_add(1,Ordering::AcqRel);
        }
        unsafe extern "C" fn removed(user:Ptr,result:i32,sender:Ptr,device:Ptr) {
            let seen=unsafe{&*(user as *const Observed)};
            if result!=0||unsafe{CFGetTypeID(sender)}!=cached().unwrap()[0]
                ||unsafe{CFGetTypeID(device)}!=cached().unwrap()[1]{seen.bad.store(true,Ordering::Release);}
            seen.removed.fetch_add(1,Ordering::AcqRel);
        }
        let context=Context::new(Arc::new(Live),Loop{runloop:1,mode:2},
            BTreeSet::from([incoming as *const () as usize]),BTreeSet::from([removed as *const () as usize])).unwrap();
        install(context.clone()).unwrap();
        let observed=Box::new(Observed{input:AtomicU64::new(0),removed:AtomicU64::new(0),bad:AtomicBool::new(false)});
        let user=std::ptr::from_ref(&*observed).cast_mut().cast();
        let manager=unsafe{create(std::ptr::null(),0)};
        assert!(!manager.is_null());
        unsafe{
            register_input(manager,Some(incoming),user);
            assert!(context.emit(manager,7,57,1).is_err());
            schedule(manager,1usize as Ptr,2usize as *const c_void);
            assert_eq!(open(manager,0),0);
            assert_eq!(context.remove_device(manager), Err("own HID removal callback absent".into()));
            context.emit(manager,7,57,1).unwrap();
            register_remove(manager,Some(removed),user);
            let owned = context.manager(manager).unwrap();
            let pending = owned.state.lock().unwrap().input.clone().unwrap();
            let element = token(2, context.nonce, Payload::Element {
                device: owned.device.clone(), page: 7, usage: 57,
            }).unwrap();
            let pending_value = token(3, context.nonce, Payload::Value { element, integer: 1 }).unwrap();
            let pending_sender = Token(NonNull::new(CFRetain(manager).cast_mut()).unwrap());
            context.remove_device(manager).unwrap();
            assert_eq!(
                context.call_registered(owned, pending, Some(7), pending_sender, pending_value),
                Err("HID input route changed before delivery".into()),
            );
            assert!(context.emit(manager,7,57,1).is_err());
            assert!(context.remove_device(manager).is_err());
            assert_eq!(close(manager,0),0);
            unschedule(manager,1usize as Ptr,2usize as *const c_void);
            schedule(manager,1usize as Ptr,2usize as *const c_void);
            assert_eq!(open(manager,0),0);
        }
        // Retiring the owner closes live registrations even while the caller
        // still holds the original manager token.
        let live_manager = context.manager(manager).unwrap();
        retire(&context).unwrap();
        unsafe {
            assert_ne!(open(manager, 0), 0);
            schedule(manager, 1usize as Ptr, 2usize as *const c_void);
        }
        {
            let state = live_manager.state.lock().unwrap();
            assert!(!state.open && !state.scheduled);
            assert!(state.input.is_none() && state.remove.is_none());
        }
        assert!(unsafe { context.emit(manager, 7, 57, 1) }.is_err());
        drop(live_manager);
        unsafe { CFRelease(manager); }
        assert_eq!(observed.input.load(Ordering::Acquire),1);
        assert_eq!(observed.removed.load(Ordering::Acquire),1);
        assert!(!observed.bad.load(Ordering::Acquire));
        assert!(!entries().lock().unwrap().values().any(|entry| entry.owner == context.nonce));
        assert!(context.check().is_err());
        assert!(unsafe{create(std::ptr::null(),0)}.is_null());

        unsafe extern "C" fn retire_inside_callback(user: Ptr, _: i32, sender: Ptr, value: Ptr) {
            let selected = CURRENT.lock().ok().and_then(|slot| slot.clone());
            let Some(context) = selected else { return; };
            let busy = retire(&context).is_err();
            let retained = cached().is_some_and(|ids| unsafe {
                CFGetTypeID(sender) == ids[0] && CFGetTypeID(value) == ids[3]
            });
            unsafe { &*user.cast::<AtomicBool>() }.store(busy && retained, Ordering::Release);
        }
        let context = Context::new(Arc::new(Live), Loop { runloop: 1, mode: 2 },
            BTreeSet::from([retire_inside_callback as *const () as usize]),
            BTreeSet::from([removed as *const () as usize])).unwrap();
        install(context.clone()).unwrap();
        let retired_busy = AtomicBool::new(false);
        let manager = unsafe { create(std::ptr::null(), 0) };
        assert!(!manager.is_null());
        unsafe {
            register_input(manager, Some(retire_inside_callback), std::ptr::from_ref(&retired_busy).cast_mut().cast());
            schedule(manager, 1usize as Ptr, 2usize as *const c_void);
            assert_eq!(open(manager, 0), 0);
            assert!(context.emit(manager, 7, 57, 1).is_err());
            CFRelease(manager);
        }
        assert!(retired_busy.load(Ordering::Acquire));
        assert_eq!(context.in_flight.load(Ordering::Acquire), 0);
        assert!(!entries().lock().unwrap().values().any(|entry| entry.owner == context.nonce));
    }
}
