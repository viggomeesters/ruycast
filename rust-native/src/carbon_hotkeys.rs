//! Main-thread Carbon ownership. Complete fixture mode never falls back to
//! actual global registration. Native events carry IDs only, no input history.
use crate::hotkeys::{Platform, Press};
use objc2::MainThreadMarker;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::c_void,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
const SIGNATURE: u32 = 0x52554341;
const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const HOTKEY: u32 = u32::from_be_bytes(*b"hkid");
const DIRECT: u32 = u32::from_be_bytes(*b"----");
const NOT_HANDLED: i32 = -9874;
#[repr(C)]
#[derive(Clone, Copy)]
struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HotkeyId {
    pub signature: u32,
    pub id: u32,
}
type Handler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;
type Target = unsafe extern "C" fn() -> *mut c_void;
type Install = unsafe extern "C" fn(
    *mut c_void,
    Handler,
    u32,
    *const EventType,
    *mut c_void,
    *mut *mut c_void,
) -> i32;
type Remove = unsafe extern "C" fn(*mut c_void) -> i32;
type Register = unsafe extern "C" fn(u32, u32, HotkeyId, *mut c_void, u32, *mut *mut c_void) -> i32;
type Parameter =
    unsafe extern "C" fn(*mut c_void, u32, u32, *mut u32, usize, *mut usize, *mut c_void) -> i32;
type EventPart = unsafe extern "C" fn(*mut c_void) -> u32;
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        count: u32,
        types: *const EventType,
        data: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RemoveEventHandler(handler: *mut c_void) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: HotkeyId,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(key: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        out_kind: *mut u32,
        capacity: usize,
        out_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
    fn GetEventClass(event: *mut c_void) -> u32;
    fn GetEventKind(event: *mut c_void) -> u32;
}
#[derive(Clone, Copy)]
struct Api {
    target: Target,
    install: Install,
    remove: Remove,
    register: Register,
    unregister: Remove,
    parameter: Parameter,
    class: EventPart,
    kind: EventPart,
}
impl Api {
    fn load(fixture: bool) -> Result<Self, String> {
        if !fixture {
            return Ok(Self {
                target: GetApplicationEventTarget,
                install: InstallEventHandler,
                remove: RemoveEventHandler,
                register: RegisterEventHotKey,
                unregister: UnregisterEventHotKey,
                parameter: GetEventParameter,
                class: GetEventClass,
                kind: GetEventKind,
            });
        }
        #[cfg(feature = "test-tools")]
        unsafe {
            let ready = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_keyboard_fixture_ready".as_ptr(),
            );
            let names = [
                c"ruycast_keyboard_fixture_target",
                c"ruycast_keyboard_fixture_install",
                c"ruycast_keyboard_fixture_remove",
                c"ruycast_keyboard_fixture_register",
                c"ruycast_keyboard_fixture_unregister",
                c"ruycast_keyboard_fixture_parameter",
                c"ruycast_keyboard_fixture_class",
                c"ruycast_keyboard_fixture_kind",
            ];
            let pointers = names.map(|name| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()));
            if ready.is_null() || pointers.iter().any(|p| p.is_null()) {
                return Err("Complete keyboard fixture is unavailable.".into());
            }
            let ready: unsafe extern "C" fn() -> u64 = std::mem::transmute(ready);
            if ready() != 0x52554b4559533031 {
                return Err("Complete keyboard fixture is unavailable.".into());
            }
            // Every function is mandatory; exact C SDK ABI, no production fallback.
            Ok(Self {
                target: std::mem::transmute::<*mut c_void, Target>(pointers[0]),
                install: std::mem::transmute::<*mut c_void, Install>(pointers[1]),
                remove: std::mem::transmute::<*mut c_void, Remove>(pointers[2]),
                register: std::mem::transmute::<*mut c_void, Register>(pointers[3]),
                unregister: std::mem::transmute::<*mut c_void, Remove>(pointers[4]),
                parameter: std::mem::transmute::<*mut c_void, Parameter>(pointers[5]),
                class: std::mem::transmute::<*mut c_void, EventPart>(pointers[6]),
                kind: std::mem::transmute::<*mut c_void, EventPart>(pointers[7]),
            })
        }
        #[cfg(not(feature = "test-tools"))]
        Err("Keyboard fixtures require test-tools.".into())
    }
}
pub(crate) struct Callback {
    api: Api,
    routes: Mutex<BTreeMap<u32, Press>>,
    enabled: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    alive: AtomicBool,
    sink: Arc<dyn Fn(Press) + Send + Sync>,
    #[cfg(feature = "test-tools")]
    source_ticket: std::cell::Cell<Option<crate::owned_host_entry::NativeInputSourceTicket>>,
}
#[cfg(feature = "test-tools")]
impl Callback {
    fn revoke_source_ticket(&self) -> bool {
        let Some(mut ticket) = self.source_ticket.take() else { return true; };
        if ticket.revoke().is_ok() { true } else { self.source_ticket.set(Some(ticket)); false }
    }
}
/// Actual typed entrypoint for the fixed Input5 compiler/source join.
/// The pointer alone grants neither callback nor userdata authority.
#[cfg(feature = "test-tools")]
pub const fn callback_entrypoints() -> unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 {
    pressed
}
unsafe extern "C" fn pressed(_call: *mut c_void, event: *mut c_void, data: *mut c_void) -> i32 {
    if data.is_null() || event.is_null() || MainThreadMarker::new().is_none() {
        return NOT_HANDLED;
    }
    // Installed handler owns this stable allocation until successful removal.
    let state = unsafe { &*data.cast::<Callback>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !state.alive.load(Ordering::Acquire)
            || !state.enabled.load(Ordering::Acquire)
            || state.failed.load(Ordering::Acquire)
        {
            return NOT_HANDLED;
        }
        let api = state.api;
        if unsafe { (api.class)(event) } != KEYBOARD || unsafe { (api.kind)(event) } != 5 {
            return NOT_HANDLED;
        }
        let mut id = HotkeyId::default();
        let mut kind = 0;
        let mut size = 0;
        let status = unsafe {
            (api.parameter)(
                event,
                DIRECT,
                HOTKEY,
                &mut kind,
                std::mem::size_of::<HotkeyId>(),
                &mut size,
                (&mut id as *mut HotkeyId).cast(),
            )
        };
        if status != 0
            || size != std::mem::size_of::<HotkeyId>()
            || kind != HOTKEY
            || id.signature != SIGNATURE
        {
            return NOT_HANDLED;
        }
        let route = state
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id.id)
            .cloned();
        if let Some(route) = route {
            // No mutex/registry borrow spans a downstream callback.
            (state.sink)(route);
            0
        } else {
            NOT_HANDLED
        }
    }));
    result.unwrap_or_else(|_| {
        state.failed.store(true, Ordering::Release);
        state.enabled.store(false, Ordering::Release);
        NOT_HANDLED
    })
}
/// Opaque, non-cloneable native reference; successful unregister consumes it.
pub struct Handle(NonNull<c_void>);
#[derive(Default)]
struct Quarantine {
    // Carbon retains the exact userdata address; moving callbacks into the
    // vector's relocatable backing storage would invalidate that pointer.
    #[allow(clippy::vec_box)]
    callbacks: Vec<Box<Callback>>,
    handles: Vec<(Api, Handle)>,
}
thread_local! {static QUARANTINE:RefCell<Quarantine>=RefCell::default();}
fn finish_callback_removal(callback: Box<Callback>, removal_uncertain: bool) {
    callback.enabled.store(false, Ordering::Release);
    callback.alive.store(false, Ordering::Release);
    #[cfg(feature = "test-tools")]
    let removal_uncertain = {
        let revoked = callback.revoke_source_ticket();
        removal_uncertain || !revoked
    };
    if removal_uncertain {
        QUARANTINE.with(|q| q.borrow_mut().callbacks.push(callback));
    }
}
pub struct Carbon {
    _main: MainThreadMarker,
    api: Api,
    target: NonNull<c_void>,
    handler: NonNull<c_void>,
    callback: Option<Box<Callback>>,
    orphans: Vec<Handle>,
}
impl Carbon {
    pub fn new(
        main: MainThreadMarker,
        fixture: bool,
        sink: Arc<dyn Fn(Press) + Send + Sync>,
    ) -> Result<Self, String> {
        if QUARANTINE.with(|q| {
            let q = q.borrow();
            !q.callbacks.is_empty() || !q.handles.is_empty()
        }) {
            return Err("A previous hotkey cleanup is incomplete. Restart Ruycast before registering again.".into());
        }
        let api = Api::load(fixture)?;
        let target = NonNull::new(unsafe { (api.target)() })
            .ok_or("Keyboard event target is unavailable.")?;
        let mut callback = Box::new(Callback {
            api,
            routes: Mutex::new(BTreeMap::new()),
            enabled: Arc::new(AtomicBool::new(true)),
            failed: Arc::new(AtomicBool::new(false)),
            alive: AtomicBool::new(true),
            sink,
            #[cfg(feature = "test-tools")]
            source_ticket: std::cell::Cell::new(None),
        });
        #[cfg(feature = "test-tools")]
        {
            let (owner, ticket) = crate::owned_host_entry::NativeInputSourceTicket::for_carbon(callback);
            callback = owner;
            match ticket {
                Ok(ticket) => callback.source_ticket.set(ticket),
                Err(error) => {
                    callback.enabled.store(false, Ordering::Release);
                    callback.alive.store(false, Ordering::Release);
                    QUARANTINE.with(|q| q.borrow_mut().callbacks.push(callback));
                    return Err(error);
                }
            }
        }
        let mut handler = std::ptr::null_mut();
        let kind = EventType {
            class: KEYBOARD,
            kind: 5,
        };
        let status = unsafe {
            (api.install)(
                target.as_ptr(),
                pressed,
                1,
                &kind,
                (&mut *callback as *mut Callback).cast(),
                &mut handler,
            )
        };
        if status != 0 || handler.is_null() {
            // A malformed partial SDK success may still retain callback data.
            callback.enabled.store(false, Ordering::Release);
            callback.alive.store(false, Ordering::Release);
            let removal_uncertain = (handler.is_null() && status == 0)
                || (!handler.is_null() && unsafe { (api.remove)(handler) } != 0);
            finish_callback_removal(callback, removal_uncertain);
            return Err(format!("Could not install hotkey handler ({status})."));
        }
        Ok(Self {
            _main: main,
            api,
            target,
            handler: NonNull::new(handler).expect("checked handler"),
            callback: Some(callback),
            orphans: vec![],
        })
    }
    pub fn callback_gate(&self) -> Arc<AtomicBool> {
        self.callback
            .as_ref()
            .expect("live callback")
            .enabled
            .clone()
    }
    pub fn failure_flag(&self) -> Arc<AtomicBool> {
        self.callback
            .as_ref()
            .expect("live callback")
            .failed
            .clone()
    }
    pub fn callback_failed(&self) -> bool {
        self.callback
            .as_ref()
            .is_some_and(|c| c.failed.load(Ordering::Acquire))
    }
    pub fn orphan_count(&self) -> usize {
        self.orphans.len()
    }
    fn retry_orphans(&mut self) {
        for handle in std::mem::take(&mut self.orphans) {
            if unsafe { (self.api.unregister)(handle.0.as_ptr()) } != 0 {
                self.orphans.push(handle);
            }
        }
    }
}
impl Platform for Carbon {
    type Handle = Handle;
    fn cleanup(&mut self) -> bool {
        self.retry_orphans();
        self.orphans.is_empty()
    }
    fn register(&mut self, code: u32, modifiers: u32, id: u32) -> Result<Handle, i32> {
        self.retry_orphans();
        if !self.orphans.is_empty() {
            return Err(-50);
        }
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            (self.api.register)(
                code,
                modifiers,
                HotkeyId {
                    signature: SIGNATURE,
                    id,
                },
                self.target.as_ptr(),
                0,
                &mut handle,
            )
        };
        if status == 0 {
            NonNull::new(handle).map(Handle).ok_or(-50)
        } else {
            // Standard Carbon failure returns no reference. Retain correctness
            // for a provider returning an owned reference alongside its error.
            if !handle.is_null() && unsafe { (self.api.unregister)(handle) } != 0 {
                self.orphans.push(Handle(
                    NonNull::new(handle).expect("non-null partial reference"),
                ));
            }
            Err(status)
        }
    }
    fn unregister(&mut self, handle: Handle) -> Result<(), (Handle, i32)> {
        let status = unsafe { (self.api.unregister)(handle.0.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err((handle, status))
        }
    }
    fn quarantine(&mut self, handle: Handle) {
        self.orphans.push(handle);
    }
    fn routes(&mut self, routes: BTreeMap<u32, Press>) {
        *self
            .callback
            .as_ref()
            .expect("live callback")
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = routes;
    }
}
impl Drop for Carbon {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback.enabled.store(false, Ordering::Release);
            callback.alive.store(false, Ordering::Release);
            let removal_uncertain = unsafe { (self.api.remove)(self.handler.as_ptr()) } != 0;
            finish_callback_removal(callback, removal_uncertain);
        }
        self.retry_orphans();
        for handle in std::mem::take(&mut self.orphans) {
            QUARANTINE.with(|q| q.borrow_mut().handles.push((self.api, handle)));
        }
    }
}

#[cfg(all(test, feature = "test-tools"))]
mod source_owner_tests {
    use super::*;
    use crate::profile_io_abi as wire;
    struct Backend {
        key: Box<[u64; 2]>, busy: bool, reenter: bool, shape_ok: bool,
        published: usize, revoked: usize, returned: usize, held_during_reentry: bool,
        userdata: usize, drops: Arc<std::sync::atomic::AtomicUsize>,
    }
    thread_local! { static BACKEND: RefCell<Option<Backend>> = const { RefCell::new(None) }; }
    unsafe extern "C" fn bind(_: wire::Bytes) -> wire::Reply { wire::Reply::denied() }
    extern "C" fn release(_: wire::Handle) -> i32 { wire::DENIED }
    extern "C" fn returned(owner: wire::Handle, token: u64) -> i32 {
        BACKEND.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut() else { return wire::DENIED; };
            if owner != (wire::Handle { slot: 17, generation: 1 }) || token != 41 { state.shape_ok = false; return wire::DENIED; }
            state.returned += 1;
            wire::OK
        })
    }
    unsafe extern "C" fn request(owner: wire::Handle, input: wire::Request) -> wire::Reply {
        BACKEND.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut() else { return wire::Reply::denied(); };
            if owner != (wire::Handle { slot: 17, generation: 1 }) || input.flags != 0 || input.limit != 0
                || !input.path.pointer.is_null() || input.path.length != 0 || input.expected_device != 0 || input.expected_inode != 0
                || input.data.pointer.is_null() || input.data.length != 48 || !matches!(input.operation, 0x1001 | 0x1002) {
                state.shape_ok = false; return wire::Reply::denied();
            }
            // The test owns the actual boxed Callback and the synchronous48B
            // request for this whole call. No Box/Rc is adopted from userdata.
            let bytes = unsafe { input.data.pointer.cast::<[u8; 48]>().read_unaligned() };
            let word = |at| u32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
            let wide = |at| u64::from_ne_bytes(bytes[at..at + 8].try_into().unwrap());
            if word(0) != 1 || word(4) != 48 || word(8) != 1 || word(12) != 1 || wide(24) != 1 {
                state.shape_ok = false; return wire::Reply::denied();
            }
            let mut reply = wire::Reply::denied();
            if input.operation == 0x1001 {
                if wide(32) != 0 || wide(40) != 0 { state.shape_ok = false; return reply; }
                state.published += 1;
                state.userdata = wide(16) as usize;
                if state.reenter {
                    state.held_during_reentry = state.drops.load(Ordering::Acquire) == 0;
                    let callback = unsafe { &*(state.userdata as *const Callback) };
                    callback.enabled.store(false, Ordering::Release);
                    callback.alive.store(false, Ordering::Release);
                    return reply;
                }
                reply.status = wire::OK;
                reply.buffer = wire::Buffer { token: 41, pointer: state.key.as_ptr().cast(), length: 16 };
            } else {
                if wide(16) as usize != state.userdata || wide(32) != 1 || wide(40) != 41 { state.shape_ok = false; return reply; }
                state.revoked += 1;
                if !state.busy { reply.status = wire::OK; }
            }
            reply
        })
    }
    static API: wire::Api = wire::Api { version: 1, bytes: 56, reserved: [0; 2], bind, request, release_handle: release, return_buffer: returned };
    struct Probe(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Probe { fn drop(&mut self) { self.0.fetch_add(1, Ordering::AcqRel); } }
    fn owner(drops: &Arc<std::sync::atomic::AtomicUsize>) -> Box<Callback> {
        let probe = Probe(drops.clone());
        Box::new(Callback {
            // Addresses only: none of these original Carbon functions is called.
            api: Api { target: GetApplicationEventTarget, install: InstallEventHandler, remove: RemoveEventHandler,
                register: RegisterEventHotKey, unregister: UnregisterEventHotKey, parameter: GetEventParameter,
                class: GetEventClass, kind: GetEventKind },
            routes: Mutex::new(BTreeMap::new()), enabled: Arc::new(AtomicBool::new(true)), failed: Arc::new(AtomicBool::new(false)),
            alive: AtomicBool::new(true), sink: Arc::new(move |_| { std::hint::black_box(&probe); }),
            source_ticket: std::cell::Cell::new(None),
        })
    }
    fn backend(drops: Arc<std::sync::atomic::AtomicUsize>, busy: bool, reenter: bool) {
        BACKEND.with(|state| *state.borrow_mut() = Some(Backend { key: Box::new([1, 41]), busy, reenter, shape_ok: true,
            published: 0, revoked: 0, returned: 0, held_during_reentry: false, userdata: 0, drops }));
        assert!(QUARANTINE.with(|q| q.borrow().callbacks.is_empty()));
    }
    #[test]
    fn actual_box_survives_busy_revoke_until_exact_key_cleanup() {
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        backend(drops.clone(), true, false);
        let boxed = owner(&drops);
        let address = std::ptr::from_ref(&*boxed) as usize;
        let (boxed, ticket) = crate::owned_host_entry::NativeInputSourceTicket::fixture_carbon_owner(boxed, &API);
        boxed.source_ticket.set(ticket.unwrap());
        finish_callback_removal(boxed, false);
        assert_eq!(drops.load(Ordering::Acquire), 0);
        let boxed = QUARANTINE.with(|q| q.borrow_mut().callbacks.pop().unwrap());
        assert_eq!(std::ptr::from_ref(&*boxed) as usize, address);
        assert!(!boxed.alive.load(Ordering::Acquire));
        BACKEND.with(|state| state.borrow_mut().as_mut().unwrap().busy = false);
        finish_callback_removal(boxed, false);
        assert_eq!(drops.load(Ordering::Acquire), 1);
        BACKEND.with(|state| { let state = state.borrow(); let state = state.as_ref().unwrap();
            assert!(state.shape_ok); assert_eq!((state.published, state.revoked, state.returned), (1, 2, 1)); });
        assert!(QUARANTINE.with(|q| q.borrow().callbacks.is_empty()));
    }
    #[test]
    fn publish_reentry_keeps_actual_box_until_uncertain_owner_is_quarantined() {
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        backend(drops.clone(), false, true);
        let boxed = owner(&drops);
        let address = std::ptr::from_ref(&*boxed) as usize;
        let (boxed, ticket) = crate::owned_host_entry::NativeInputSourceTicket::fixture_carbon_owner(boxed, &API);
        assert!(ticket.is_err());
        assert!(!boxed.alive.load(Ordering::Acquire));
        finish_callback_removal(boxed, true);
        assert_eq!(drops.load(Ordering::Acquire), 0);
        let boxed = QUARANTINE.with(|q| q.borrow_mut().callbacks.pop().unwrap());
        assert_eq!(std::ptr::from_ref(&*boxed) as usize, address);
        BACKEND.with(|state| { let state = state.borrow(); let state = state.as_ref().unwrap();
            assert!(state.shape_ok && state.held_during_reentry); assert_eq!((state.published, state.revoked, state.returned), (1, 0, 0)); });
        // This controlled mock never registered a C handler; only its known
        // test owner can now release the retained box after preserving proof.
        drop(boxed);
        assert_eq!(drops.load(Ordering::Acquire), 1);
    }
}
