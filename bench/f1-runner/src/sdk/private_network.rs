//! Test-only Security leaves. No constructor, interpose section, Keychain call,
//! account lookup or network fallback. The SDK integrator owns actual imports.
//! This is a bounded Keychain/entropy contribution, not provider/Safari admission.
//! Its fixed module has no whole-provider scope; process/network admission is separate.
use super::owned::AliveLease;
use std::{
    ffi::{c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
pub type Ptr = *const c_void;
pub const DISABLED: i32 = -25291;
pub const PARAM: i32 = -50;
pub const NOT_FOUND: i32 = -25300;
pub const DUPLICATE: i32 = -25299;
pub const PROCESS_NOT_FOUND: i32 = -600;
/// AEDesc uses the two-byte packing specified by AEDataModel.h, including on
/// 64-bit macOS. The opaque storage is accessed only through AE data getters.
#[repr(C, packed(2))]
pub struct AddressDesc {
    pub descriptor_type: u32,
    pub data_handle: *mut c_void,
}
const _: () = assert!(std::mem::size_of::<AddressDesc>() == 4 + std::mem::size_of::<Ptr>());
const _: () = assert!(std::mem::align_of::<AddressDesc>() == 2);
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AEGetDescDataSize(desc: *const AddressDesc) -> isize;
    fn AEGetDescData(desc: *const AddressDesc, data: *mut c_void, maximum: isize) -> i16;
}
const MAX: usize = 12_000_000;
type Result<T> = std::result::Result<T, i32>;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDictionaryGetCount(dictionary: Ptr) -> isize;
    fn CFDictionaryGetValue(dictionary: Ptr, key: Ptr) -> Ptr;
    fn CFEqual(a: Ptr, b: Ptr) -> u8;
    fn CFDataGetLength(data: Ptr) -> isize;
    fn CFDataGetBytePtr(data: Ptr) -> *const u8;
    fn CFDataCreate(allocator: Ptr, bytes: *const u8, length: isize) -> Ptr;
    fn CFStringCreateWithBytes(
        allocator: Ptr,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ptr;
    fn CFRelease(value: Ptr);
    static kCFBooleanTrue: Ptr;
}
// This block has constants only; original function addresses below are never called.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: Ptr;
    static kSecClassGenericPassword: Ptr;
    static kSecAttrService: Ptr;
    static kSecAttrAccount: Ptr;
    static kSecReturnData: Ptr;
    static kSecMatchLimit: Ptr;
    static kSecMatchLimitOne: Ptr;
    static kSecValueData: Ptr;
    static kSecRandomDefault: Ptr;
}
struct Cf(Ptr);
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
use crate::workspace::objc::{bridge_swift_data, SwiftData};

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_storage_reader(_entry: usize, _error: *mut usize) -> SwiftData {
    // The fixed Swift closure has no context. Swift carries errors in x21.
    core::arch::naked_asm!(
        "stp x20, x21, [sp, #-32]!",
        "stp x19, x30, [sp, #16]",
        "mov x9, x0",
        "mov x19, x1",
        "mov x20, xzr",
        "mov x21, xzr",
        "blr x9",
        "str x21, [x19]",
        "ldp x19, x30, [sp, #16]",
        "ldp x20, x21, [sp], #32",
        "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_storage_writer(_entry: usize, _data: SwiftData, _error: *mut usize) {
    core::arch::naked_asm!(
        "stp x20, x21, [sp, #-32]!",
        "stp x19, x30, [sp, #16]",
        "mov x9, x0",
        "mov x0, x1",
        "mov x1, x2",
        "mov x19, x3",
        "mov x20, xzr",
        "mov x21, xzr",
        "blr x9",
        "str x21, [x19]",
        "ldp x19, x30, [sp, #16]",
        "ldp x20, x21, [sp], #32",
        "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_storage_entropy(_entry: usize, _count: usize, _error: *mut usize) -> usize {
    core::arch::naked_asm!(
        "stp x20, x21, [sp, #-32]!",
        "stp x19, x30, [sp, #16]",
        "mov x9, x0",
        "mov x0, x1",
        "mov x19, x2",
        "mov x20, xzr",
        "mov x21, xzr",
        "blr x9",
        "str x21, [x19]",
        "ldp x19, x30, [sp, #16]",
        "ldp x20, x21, [sp], #32",
        "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) unsafe fn verify_original_entropy(
    entry: usize, import: &super::ImportReplacement, context: &Context,
) -> std::result::Result<(), String> {
    #[link(name = "swiftCore")]
    unsafe extern "C" {
        fn swift_release();
        fn swift_errorRelease(error: usize);
    }
    {
        let state = context.state.lock().map_err(|_| "Reference entropy fixture lock poisoned")?;
        if state.offset != 0 || !state.entropy.iter().copied().eq(0..32) {
            return Err("Reference entropy fixture is not the expected unused stream".into());
        }
    }
    for (count, start) in [(4, 0_u8), (24, 4), (4, 28), (4, 32)] {
        context.check_pure().map_err(|_| "Reference entropy fixture retired")?;
        if entry.checked_add(0x3c5a10 - 0x1d5214) != Some(import.cell_address())
            || import.selected_address() != rcf1_security_random as *const () as usize
        {
            return Err("Reference entropy import changed before original call".into());
        }
        let mut error = 0;
        let storage = unsafe { call_storage_entropy(entry, count, &mut error) };
        if error != 0 {
            unsafe { swift_errorRelease(error) };
            if start == 32 {
                return Ok(());
            }
            return Err("Reference original entropy call threw before exhaustion".into());
        }
        if storage == 0 || !storage.is_multiple_of(std::mem::align_of::<usize>()) {
            return Err("Reference original entropy returned invalid Array storage".into());
        }
        // The pinned Swift function returns +1 native Array<UInt8> storage.
        let object = storage as *const u8;
        let length = unsafe { object.add(16).cast::<usize>().read() };
        let matches = length == count && unsafe { std::slice::from_raw_parts(object.add(32), count) }
            .iter().copied().eq(start..start + count as u8);
        let release: unsafe extern "C" fn(usize) = unsafe { std::mem::transmute(swift_release as *const ()) };
        unsafe { release(storage) };
        if start == 32 || !matches {
            return Err("Reference original entropy bytes or exhaustion differs".into());
        }
    }
    Err("Reference original entropy did not check exhaustion".into())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe fn destroy_storage_data(data: &mut SwiftData) {
    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        #[link_name = "$s10Foundation4DataVMa"]
        fn data_metadata(request: usize) -> *const *const c_void;
    }
    // The bridge and writer borrow the value. This releases the original +1.
    unsafe {
        let metadata = data_metadata(0);
        let witnesses = (*metadata.sub(1)).cast::<*const c_void>();
        let destroy: unsafe extern "C" fn(&mut SwiftData, *const *const c_void) =
            std::mem::transmute(*witnesses.add(1));
        destroy(data, metadata);
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) unsafe fn verify_original_write(
    entry: usize, imports: &[super::ImportReplacement], context: &Context, bytes: &[u8],
) -> std::result::Result<(), String> {
    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        #[link_name = "$s10Foundation4DataV36_unconditionallyBridgeFromObjectiveCyACSo6NSDataCSgFZ"]
        fn bridge_input(data: Ptr) -> SwiftData;
    }
    #[link(name = "swiftCore")]
    unsafe extern "C" {
        fn swift_errorRelease(error: usize);
    }
    objc2::rc::autoreleasepool(|_| unsafe {
        let data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize);
        if data.is_null() {
            return Err("Reference Keychain fixture Data allocation failed".into());
        }
        let data = Cf(data);
        let mut value = bridge_input(data.0);
        let result = (|| {
            context.check_pure().map_err(|_| "Reference Security fixture retired before original write")?;
            for (symbol, cell, selected) in [
                (c"SecItemUpdate", 0x3c5a08, rcf1_security_update as *const () as usize),
                (c"SecItemAdd", 0x3c59f8, rcf1_security_add as *const () as usize),
            ] {
                if !imports.iter().any(|row| row.symbol == symbol
                    && entry.checked_add(cell - 0x1d4fe0) == Some(row.cell_address())
                    && row.selected_address() == selected)
                {
                    return Err("Reference Keychain write import changed before original write".into());
                }
            }
            let mut error = 0;
            call_storage_writer(entry, SwiftData(value.0), &mut error);
            if error != 0 {
                swift_errorRelease(error);
                return Err("Reference original Keychain write threw".into());
            }
            Ok(())
        })();
        destroy_storage_data(&mut value);
        result
    })
}

/// Compare the actual returned Data to the private assembly fixture.
/// The caller must qualify the original entry and its private import first.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) unsafe fn verify_original_read(
    entry: usize,
    copy: &super::ImportReplacement,
    context: &Context,
    expected: Option<&[u8]>,
) -> std::result::Result<(), String> {
    #[link(name = "swiftCore")]
    unsafe extern "C" {
        fn swift_errorRelease(error: usize);
    }

    objc2::rc::autoreleasepool(|_| unsafe {
        let mut error = 0;
        context.check_pure().map_err(|_| "Reference Security fixture retired before original read")?;
        if entry.checked_add(0x3c5a00 - 0x1d4dc0) != Some(copy.cell_address())
            || copy.selected_address() != rcf1_security_copy as *const () as usize
        {
            return Err("Reference Keychain copy import changed before original read".into());
        }
        let mut data = call_storage_reader(entry, &mut error);
        if error != 0 {
            swift_errorRelease(error);
            return Err("Reference original Keychain read threw".into());
        }
        if data.0 == [0, 0xf000_0000_0000_0000] {
            return if expected.is_none() {
                Ok(())
            } else {
                Err("Reference original Keychain read returned no data".into())
            };
        }
        let output = bridge_swift_data(SwiftData(data.0));
        destroy_storage_data(&mut data);
        let output = objc2::rc::Retained::from_raw(output.cast::<objc2::runtime::AnyObject>())
            .ok_or("Reference original Keychain Data bridge returned no object")?;
        let expected = expected.ok_or("Reference original Keychain read found an unexpected entry")?;
        let length: usize = objc2::msg_send![&*output, length];
        let pointer: *const u8 = objc2::msg_send![&*output, bytes];
        if length != expected.len() || pointer.is_null()
            || std::slice::from_raw_parts(pointer, length) != expected
        {
            return Err("Reference original Keychain read differs from fixture".into());
        }
        Ok(())
    })
}
#[derive(Clone, Copy)]
struct Types {
    dictionary: usize,
    string: usize,
    data: usize,
}
#[derive(Clone, Copy)]
pub enum Operation {
    Read,
    Update,
    Add,
    Entropy,
}
impl Operation {
    fn index(self) -> usize {
        self as usize
    }
}
struct State {
    consent: Option<(i32, i32)>,
    #[cfg(all(rcf1_reference_sdk, not(test)))]
    consent_identity: Option<super::system_providers::ProcessIdentity>,
    history: Option<Vec<u8>>,
    entropy: Vec<u8>,
    offset: usize,
    faults: [Option<i32>; 4],
    calls: [u64; 4],
}
/// All bytes are explicit synthetic input. This type intentionally has no Debug
/// or serialization implementation; diagnostic output cannot print its contents.
pub struct Context {
    life: Arc<dyn AliveLease>,
    retired: AtomicBool,
    state: Mutex<State>,
    types: Types,
    module: std::sync::OnceLock<SecurityModule>,
    backend: std::sync::OnceLock<(Arc<crate::workspace::Workspace>,u64)>,
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[path = "workspace_web_protocol.rs"]
mod web_protocol;

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) use web_protocol::{assembly_cache_listing, qualify_assembly_cache_error};
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use web_protocol::{assembly_image_is_live, assembly_started as reference_website_assembly_started, take_assembly_object};

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn install_reference_website(
    admission: Arc<super::core_preentry::ReferenceAdmission>, context: &Arc<Context>,
) -> std::result::Result<super::ModuleAttestation, String> {
    let owner = context.module.get().ok_or("Reference website Security owner absent")?;
    owner.website.set(web_protocol::install(admission, context.clone())?)
        .map_err(|_| "Reference website installation replay")?;
    attestation()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn drain_reference_website(context: &Arc<Context>) -> std::result::Result<(), String> {
    web_protocol::close_and_drain(context)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn rollback_reference_website(context: &Arc<Context>) -> std::result::Result<(), String> {
    web_protocol::rollback(context)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn probe_reference_website(context: &Arc<Context>) -> std::result::Result<(), String> {
    web_protocol::probe_original_image(context)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn submit_reference_website_reply(route: usize, group: usize, block: usize, queue: usize) -> std::result::Result<bool, String> {
    web_protocol::submit_assembly_reply(route, group, block, queue)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_website_assembly_active() -> bool {
    web_protocol::assembly_active()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_website_uses_platform_main_queue(return_pc: usize) -> bool {
    web_protocol::uses_platform_main_queue(return_pc)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn report_reference_website_data_dealloc(class: &CStr) {
    web_protocol::report_assembly_data_dealloc(class);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn report_reference_website_dispatch_refusal(reason: &str) {
    web_protocol::report_assembly_dispatch_refusal(reason);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn report_reference_website_selector_refusal(class: &CStr, selector: &CStr) {
    web_protocol::report_assembly_selector_refusal(class, selector);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn qualify_reference_website_clock(
    admission: &super::core_preentry::ReferenceAdmission, return_pc: usize,
) -> std::result::Result<bool, String> {
    web_protocol::qualify_assembly_clock(admission, return_pc)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn qualify_reference_website_cache_directory(
    admission: &super::core_preentry::ReferenceAdmission,
) -> std::result::Result<(), String> {
    web_protocol::qualify_assembly_cache_directory(admission)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn report_reference_website_clock_refusal(return_pc: usize, reason: &str) {
    web_protocol::report_assembly_clock_refusal(return_pc, reason);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) unsafe fn allocate_reference_website_object(
    class: &objc2::runtime::AnyClass, caller: usize, frame: usize,
) -> std::result::Result<Option<*mut objc2::runtime::AnyObject>, String> {
    unsafe { web_protocol::allocate_source_object(class, caller, frame) }
}

struct SecurityModule {
    lease: super::owned::OwnedHandleLease,
    root_identity: (u64, u64),
    imports: Vec<super::ImportReplacement>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    website: std::sync::OnceLock<Vec<super::ObjcReplacement>>,
}
/// Own publication cleanup is deliberately independent from a poisoned lock,
/// dynamic guardian or further business admission.
struct PublicationGuard {
    context: Arc<Context>,
    complete: bool,
}
impl Drop for PublicationGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.context.retired.store(true, Ordering::Release);
            PUBLISHED.store(false, Ordering::Release);
        }
    }
}
impl Context {
    pub fn new(
        life: Arc<dyn AliveLease>,
        history: Option<Vec<u8>>,
        entropy: Vec<u8>,
    ) -> std::result::Result<Arc<Self>, String> {
        if !life.is_alive()
            || history.as_ref().is_some_and(|v| v.len() > MAX)
            || entropy.len() > MAX
        {
            return Err("invalid owned Security fixture".into());
        }
        life.before_effect()?;
        // Type registration is explicit setup before publication, never lazy
        // work in an unpublished C callback.
        let types = unsafe {
            Types {
                dictionary: CFDictionaryGetTypeID(),
                string: CFStringGetTypeID(),
                data: CFDataGetTypeID(),
            }
        };
        Ok(Arc::new(Self {
            life,
            retired: AtomicBool::new(false),
            types,
            module: std::sync::OnceLock::new(),
            backend: std::sync::OnceLock::new(),
            state: Mutex::new(State {
                consent: None,
                #[cfg(all(rcf1_reference_sdk, not(test)))]
                consent_identity: None,
                history,
                entropy,
                offset: 0,
                faults: [None; 4],
                calls: [0; 4],
            }),
        }))
    }
    fn live(&self) -> Result<()> {
        self.live_status().map_err(|_| DISABLED)
    }
    fn live_status(&self) -> std::result::Result<(), &'static str> {
        self.pure_status()?;
        // Dynamic lease callbacks may synchronously retire this same Context
        // while still returning true/Ok. Recheck only our own monotone flag
        // after each callback, including the last gate before an effect.
        let alive = match catch_unwind(AssertUnwindSafe(||self.life.is_alive())) {
            Ok(alive)=>alive,
            Err(payload)=>{
                self.retired.store(true,Ordering::Release);
                std::mem::forget(payload);
                return Err("Security guardian liveness panicked");
            }
        };
        if self.retired.load(Ordering::Acquire) || !alive {
            self.retired.store(true, Ordering::Release);
            return Err("Security guardian not live");
        }
        let admitted = match catch_unwind(AssertUnwindSafe(||self.life.before_effect())) {
            Ok(admitted)=>admitted,
            Err(payload)=>{
                self.retired.store(true,Ordering::Release);
                std::mem::forget(payload);
                return Err("Security guardian effect check panicked");
            }
        };
        if self.retired.load(Ordering::Acquire) || admitted.is_err() {
            self.retired.store(true, Ordering::Release);
            return Err("Security guardian refused effect");
        }
        Ok(())
    }
    /// Callback-free fence for use under State's lock. Dynamic guardians run
    /// before acquiring it or after releasing it, never inside the lock.
    fn check_pure(&self)->Result<()> {
        self.pure_status().map_err(|_| DISABLED)
    }
    fn pure_status(&self) -> std::result::Result<(), &'static str> {
        if self.retired.load(Ordering::Acquire) {return Err("Security context retired");}
        if let Some((model,generation))=self.backend.get() {
            model.generation_status(*generation)?;
        }
        Ok(())
    }
    pub fn fault(
        &self,
        operation: Operation,
        status: Option<i32>,
    ) -> std::result::Result<(), String> {
        self.live().map_err(|_| "retired Security fixture")?;
        if status == Some(0) {
            return Err("success is not an injected error".into());
        }
        let mut state=self.state.lock().map_err(|_| "Security fixture poisoned")?;
        self.check_pure().map_err(|_| "Security fixture retired during fault publication")?;
        state.faults[operation.index()] = status;
        Ok(())
    }
    /// Explicit owned fixture only: no process lookup, TCC request or consent
    /// persistence. Unknown PIDs cannot borrow this target's disposition.
    pub fn consent(&self, pid: i32, disposition: i32) -> std::result::Result<(), String> {
        if pid <= 0 || !matches!(disposition, 0 | -1743 | -1744 | PROCESS_NOT_FOUND) {
            return Err("invalid owned automation fixture".into());
        }
        self.live().map_err(|_| "retired automation fixture")?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "automation fixture poisoned")?;
        self.check_pure().map_err(|_| "retired automation fixture")?;
        state.consent = Some((pid, disposition));
        Ok(())
    }
    pub fn counts(&self) -> std::result::Result<[u64; 4], String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "Security fixture poisoned")?
            .calls)
    }
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn configure_backend_consent(
        &self, identity: Option<super::system_providers::ProcessIdentity>, disposition: i32,
    ) -> std::result::Result<(), String> {
        if !matches!(disposition, 0 | -1743 | -1744 | PROCESS_NOT_FOUND) {
            return Err("invalid Reference automation disposition".into());
        }
        self.live().map_err(|_| "Reference consent owner retired")?;
        let mut state = self.state.lock().map_err(|_| "Reference consent state poisoned")?;
        self.check_pure().map_err(|_| "Reference consent retired during publication")?;
        state.consent = identity.map(|identity| (identity.pid, disposition));
        state.consent_identity = identity;
        Ok(())
    }
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn bind_backend(&self,model:&Arc<crate::workspace::Workspace>)->std::result::Result<(),String> {
        self.live().map_err(|_|"Reference automation context retired")?;
        let lease=model.lease();
        if !Arc::ptr_eq(&lease.life,&self.life) {return Err("Reference backend guardian differs".into());}
        let generation=model.backend_generation()?;
        self.backend.set((model.clone(),generation)).map_err(|_|"Reference automation backend already bound")?;
        self.check_pure().map_err(|_|"Reference backend retired during binding".into())
    }
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn revoke_backend(&self) {self.retired.store(true,Ordering::Release);}
    fn state(&self, operation: Operation) -> Result<std::sync::MutexGuard<'_, State>> {
        self.live()?;
        let mut state = self.state.lock().map_err(|_| DISABLED)?;
        self.check_pure()?;
        let index = operation.index();
        state.calls[index] = state.calls[index].checked_add(1).ok_or(DISABLED)?;
        if let Some(status) = state.faults[index] {
            return Err(status);
        }
        Ok(state)
    }
}
static CURRENT: Mutex<Option<Arc<Context>>> = Mutex::new(None);
static PUBLISHED: AtomicBool = AtomicBool::new(false);
pub fn install(context: Arc<Context>) -> std::result::Result<(), String> {
    context.live().map_err(|_| "retired Security fixture")?;
    let mut current = CURRENT.lock().map_err(|_| "Security owner poisoned")?;
    if current.is_some() {
        return Err("Security owner already installed".into());
    }
    *current = Some(context);
    PUBLISHED.store(true, Ordering::Release);
    Ok(())
}
pub fn retire(expected: &Arc<Context>) -> std::result::Result<(), String> {
    {
        let current = CURRENT.lock().map_err(|_| "Security owner poisoned")?;
        if !current.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
            return Err("stale Security owner".into());
        }
    }
    // CURRENT never replaces a published owner. Release its mutex before
    // waiting for an in-flight State commit, including an output write.
    let state = expected.state.lock();
    PUBLISHED.store(false, Ordering::Release);
    expected.retired.store(true, Ordering::Release);
    // Retain the retired owner: this process's publication cannot be rearmed.
    state.map(|_| ()).map_err(|_| "Security state poisoned during retirement".into())
}
fn current() -> Result<Arc<Context>> {
    let context = published_context().map_err(|_| DISABLED)?;
    context.live()?;
    Ok(context)
}

fn published_context() -> std::result::Result<Arc<Context>, &'static str> {
    if !PUBLISHED.load(Ordering::Acquire) {
        return Err("Security module unpublished");
    }
    CURRENT
        .lock()
        .map_err(|_| "Security owner lock poisoned")?
        .clone()
        .ok_or("Security owner absent")
}

fn current_attested() -> std::result::Result<Arc<Context>, String> {
    let context = published_context()?;
    context.live_status().map_err(|reason| {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(site) = context.backend.get().and_then(|(model, _)| model.fault_site()) {
            let source = site.file().rsplit('/').next().unwrap_or("workspace");
            return format!("{reason} at {source} line {}", site.line());
        }
        reason.to_owned()
    })?;
    Ok(context)
}
fn status(body: impl FnOnce() -> Result<()>) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result.err().unwrap_or(0),
        Err(payload) => {
            // Revoke the owner too: another in-flight call may already have
            // passed CURRENT's publication check and must see a pure fence.
            if let Ok(current)=CURRENT.lock() {
                if let Some(context)=current.as_ref() {context.retired.store(true,Ordering::Release);}
            }
            PUBLISHED.store(false, Ordering::Release);
            std::mem::forget(payload);
            DISABLED
        }
    }
}
unsafe fn value(dictionary: Ptr, key: Ptr) -> Result<Ptr> {
    let result = unsafe { CFDictionaryGetValue(dictionary, key) };
    if result.is_null() {
        Err(PARAM)
    } else {
        Ok(result)
    }
}
unsafe fn dictionary(context: &Context, pointer: Ptr, count: isize) -> Result<()> {
    if pointer.is_null()
        || unsafe { CFGetTypeID(pointer) } != context.types.dictionary
        || unsafe { CFDictionaryGetCount(pointer) } != count
    {
        return Err(PARAM);
    }
    Ok(())
}
unsafe fn string(context: &Context, pointer: Ptr, expected: &[u8]) -> Result<()> {
    if pointer.is_null() || unsafe { CFGetTypeID(pointer) } != context.types.string {
        return Err(PARAM);
    }
    let text = unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            expected.as_ptr(),
            expected.len() as isize,
            0x08000100,
            0,
        )
    };
    if text.is_null() {
        return Err(DISABLED);
    }
    let text = Cf(text);
    if unsafe { CFEqual(pointer, text.0) } == 0 {
        return Err(PARAM);
    }
    Ok(())
}
unsafe fn identity(context: &Context, dictionary: Ptr) -> Result<()> {
    if unsafe { CFEqual(value(dictionary, kSecClass)?, kSecClassGenericPassword) } == 0 {
        return Err(PARAM);
    }
    unsafe {
        string(
            context,
            value(dictionary, kSecAttrService)?,
            b"viggo-raycast-utilities",
        )?;
    }
    unsafe {
        string(
            context,
            value(dictionary, kSecAttrAccount)?,
            b"password-history",
        )
    }
}
unsafe fn bytes(context: &Context, pointer: Ptr) -> Result<Vec<u8>> {
    if pointer.is_null() || unsafe { CFGetTypeID(pointer) } != context.types.data {
        return Err(PARAM);
    }
    let length = unsafe { CFDataGetLength(pointer) };
    if length < 0 || length as usize > MAX {
        return Err(PARAM);
    }
    if length == 0 {
        return Ok(vec![]);
    }
    let data = unsafe { CFDataGetBytePtr(pointer) };
    if data.is_null() {
        return Err(PARAM);
    }
    Ok(unsafe { std::slice::from_raw_parts(data, length as usize) }.to_vec())
}
/// # Safety
/// Non-null input objects must be genuine, lifetime-pinned immutable CF objects;
/// any non-null output must be writable. Unpublished inputs are never inspected.
#[no_mangle]
pub unsafe extern "C" fn rcf1_security_copy(query: Ptr, output: *mut Ptr) -> i32 {
    if !output.is_aligned() {
        return PARAM;
    }
    if !output.is_null() {
        unsafe { output.write(std::ptr::null()) };
    }
    status(|| {
        let context = current()?;
        if output.is_null() {
            return Err(PARAM);
        }
        unsafe {
            dictionary(&context, query, 5)?;
            identity(&context, query)?;
            if CFEqual(value(query, kSecReturnData)?, kCFBooleanTrue) == 0
                || CFEqual(value(query, kSecMatchLimit)?, kSecMatchLimitOne) == 0
            {
                return Err(PARAM);
            }
        }
        let state = context.state(Operation::Read)?;
        let history = state.history.as_ref().ok_or(NOT_FOUND)?;
        let data =
            unsafe { CFDataCreate(std::ptr::null(), history.as_ptr(), history.len() as isize) };
        if data.is_null() {
            return Err(DISABLED);
        }
        let data = Cf(data);
        drop(state);
        context.live()?;
        let _publication = context.state.lock().map_err(|_| DISABLED)?;
        context.check_pure()?;
        unsafe { output.write(data.0) };
        std::mem::forget(data);
        Ok(())
    })
}
/// # Safety
/// Inputs must satisfy the same genuine-CF-object contract as rcf1_security_copy.
#[no_mangle]
pub unsafe extern "C" fn rcf1_security_update(query: Ptr, changes: Ptr) -> i32 {
    status(|| {
        let context = current()?;
        unsafe {
            dictionary(&context, query, 3)?;
            identity(&context, query)?;
            dictionary(&context, changes, 1)?;
        }
        let data = unsafe { bytes(&context, value(changes, kSecValueData)?)? };
        let mut state = context.state(Operation::Update)?;
        if state.history.is_none() {
            return Err(NOT_FOUND);
        }
        context.check_pure()?;
        state.history = Some(data);
        Ok(())
    })
}
/// # Safety
/// Inputs/output must satisfy the contract of rcf1_security_copy. This exact
/// product add shape does not request a returned object and requires null output.
#[no_mangle]
pub unsafe extern "C" fn rcf1_security_add(item: Ptr, output: *mut Ptr) -> i32 {
    status(|| {
        let context = current()?;
        if !output.is_null() {
            return Err(PARAM);
        }
        unsafe {
            dictionary(&context, item, 4)?;
            identity(&context, item)?;
        }
        let data = unsafe { bytes(&context, value(item, kSecValueData)?)? };
        let mut state = context.state(Operation::Add)?;
        if state.history.is_some() {
            return Err(DUPLICATE);
        }
        context.check_pure()?;
        state.history = Some(data);
        Ok(())
    })
}
/// # Safety
/// For an admitted nonzero request `output` must be writable for `count` bytes.
/// No output is written on refusal, failure, exhausted fixture, or retired lease.
#[no_mangle]
pub unsafe extern "C" fn rcf1_security_random(
    random: Ptr,
    count: usize,
    output: *mut c_void,
) -> i32 {
    status(|| {
        let context = current()?;
        if random != unsafe { kSecRandomDefault } || !matches!(count, 4 | 24) || output.is_null() {
            return Err(PARAM);
        }
        let mut state = context.state(Operation::Entropy)?;
        let end = state.offset.checked_add(count).ok_or(PARAM)?;
        let data = state.entropy.get(state.offset..end).ok_or(DISABLED)?;
        context.check_pure()?;
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), output.cast(), count) };
        state.offset = end;
        Ok(())
    })
}
/// # Safety
/// An admitted nonnull target must be a live immutable AEDesc. No storage
/// handle is dereferenced, and unpublished/retired calls do not inspect target.
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
#[no_mangle]
pub unsafe extern "C" fn rcf1_automation_permission(
    target: *const AddressDesc,
    event_class: u32,
    event_id: u32,
    ask: u8,
) -> i32 {
    unsafe { automation_permission_checked(target, event_class, event_id, ask, 0) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn rcf1_automation_permission(
    _: *const AddressDesc, _: u32, _: u32, _: u8,
) -> i32 {
    core::arch::naked_asm!("mov x4, x30", "b {checked}", checked = sym automation_permission_checked);
}

unsafe extern "C" fn automation_permission_checked(
    target: *const AddressDesc,
    event_class: u32,
    event_id: u32,
    ask: u8,
    _caller: usize,
) -> i32 {
    status(|| {
        let context = current()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let stopped = {
            let (model, _) = context.backend.get().ok_or(DISABLED)?;
            if ask != 1 {
                return Err(PARAM);
            }
            super::reference_runtime::verify_safari_effect(model, _caller, crate::workspace::callbacks::SafariEffect::Consent).map_err(|_| DISABLED)?
        };
        let consent={
            let state=context.state.lock().map_err(|_| DISABLED)?;
            context.check_pure()?;
            state.consent.ok_or(DISABLED)?
        };
        let (expected_pid, disposition) = consent;
        if target.is_null()
            || event_class != u32::from_be_bytes(*b"****")
            || event_id != u32::from_be_bytes(*b"****")
            || ask > 1
        {
            return Err(PARAM);
        }
        let kind = unsafe { std::ptr::addr_of!((*target).descriptor_type).read_unaligned() };
        if kind != u32::from_be_bytes(*b"kpid") || unsafe { AEGetDescDataSize(target) } != 4 {
            return Err(PARAM);
        }
        let mut pid = 0_i32;
        if unsafe { AEGetDescData(target, (&mut pid as *mut i32).cast(), 4) } != 0 || pid <= 0 {
            return Err(PARAM);
        }
        // A callback can retire the same owner and still return success. No
        // permission disposition escapes after that final dynamic boundary.
        context.live()?;
        // A guardian can reconfigure consent. Recheck the exact disposition
        // after that callback, without invoking another callback under State.
        let state=context.state.lock().map_err(|_| DISABLED)?;
        context.check_pure()?;
        if state.consent!=Some(consent) {return Err(DISABLED);}
        let outcome = if pid == expected_pid {
            disposition
        } else {
            PROCESS_NOT_FOUND
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            let identity = state.consent_identity.filter(|identity| identity.pid == expected_pid)
                .ok_or(DISABLED)?;
            super::process_router::with_virtual_identity(identity, || {
                crate::workspace::callbacks::reference_safari_effect(crate::workspace::callbacks::SafariEffect::Consent, stopped, || Ok(()))
            }).map_err(|_| DISABLED)?;
        }
        if outcome == 0 {
            Ok(())
        } else {
            Err(outcome)
        }
    })
}
/// Fixed Security/automation permission imports. Originals are addresses only;
/// no real Keychain, entropy or permission operation is called by setup.
pub fn import_specs() -> Vec<super::ImportSpec> {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecItemCopyMatching(query: Ptr, output: *mut Ptr) -> i32;
        fn SecItemUpdate(query: Ptr, changes: Ptr) -> i32;
        fn SecItemAdd(item: Ptr, output: *mut Ptr) -> i32;
        fn SecRandomCopyBytes(random: Ptr, count: usize, output: *mut c_void) -> i32;
    }
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn AEDeterminePermissionToAutomateTarget(
            target: *const AddressDesc,
            event_class: u32,
            event_id: u32,
            ask: u8,
        ) -> i32;
    }
    [
        (
            c"SecItemCopyMatching",
            SecItemCopyMatching as *const () as usize,
        ),
        (c"SecItemUpdate", SecItemUpdate as *const () as usize),
        (c"SecItemAdd", SecItemAdd as *const () as usize),
        (
            c"SecRandomCopyBytes",
            SecRandomCopyBytes as *const () as usize,
        ),
        (
            c"AEDeterminePermissionToAutomateTarget",
            AEDeterminePermissionToAutomateTarget as *const () as usize,
        ),
    ]
    .into_iter()
    .map(|(symbol, original)| super::ImportSpec {
        symbol,
        original,
        replacement: replacement(symbol).expect("fixed Security import"),
    })
    .collect()
}
/// Five leaves do not prove the provider/network conjunction.
pub fn scopes() -> std::collections::BTreeSet<&'static str> {
    std::collections::BTreeSet::new()
}
pub fn source_roster() -> std::collections::BTreeSet<super::SourceCallsite> {
    use sha2::{Digest, Sha256};
    let sources = [
        (
            "rust-native/src/utility_storage.rs",
            include_bytes!("../../../../rust-native/src/utility_storage.rs").as_slice(),
            "owned Keychain history and exact entropy requests",
        ),
        (
            "rust-native/src/system_search/native.rs",
            include_bytes!("../../../../rust-native/src/system_search/native.rs").as_slice(),
            "owned automation permission; discovery/send remain separate",
        ),
    ]
    .into_iter()
    .map(|(file, bytes, operation)| super::SourceCallsite {
        file,
        source_sha256: format!("{:x}", Sha256::digest(bytes)),
        operation,
    })
    .collect::<std::collections::BTreeSet<_>>();
    #[cfg(rcf1_reference_sdk)]
    let sources = sources.into_iter().chain(reference_source_roster()).collect();
    sources
}
#[cfg(rcf1_reference_sdk)]
pub(crate) fn reference_source_roster()->std::collections::BTreeSet<super::SourceCallsite> {
    [
        ("bench/reference/0.8.26/Sources/RuycastApp/BrowserTabs.swift","607f1893da625280b343016653cfdfad33f81fde1ec0a55811cfd1043782ad2c"),
        ("bench/reference/0.8.26/Sources/RuycastApp/Utilities.swift","f41a861d759567171b184b49e20e70dad6a1fb6a653e7f0c161114cbab1cf49f"),
        ("bench/reference/0.8.26/Sources/RuycastApp/SystemServices.swift","c2ffbb73cc7723234992b1883bb582e68b175982a6a5991757cf47397e9935ad"),
    ].into_iter().map(|(file,hash)|super::SourceCallsite {
        file,source_sha256:hash.into(),operation:"Original owned Security/entropy and creator-thread borrowed AE permission inputs",
    }).chain(std::iter::once(super::SourceCallsite {
        file: "bench/reference/0.8.26/Sources/RuycastApp/WebsiteIcons.swift",
        source_sha256: "2708e4accf997b102fe831dd271fab41cf40aa6911a00454e6d0a4ad181aadc8".into(),
        operation: "Original website operation queue, ephemeral task and delegate",
    })).collect()
}
/// Permission uses a borrowed aeDesc only from the selected same-model NSAE
/// factory. NSAppleEventDescriptor.h declares NS_RETURNS_INNER_POINTER; the
/// fixed BrowserTabs caller keeps its local receiver alive on its creator queue.
/// This joins existing proofs; it grants no Safari discovery/send/whole scope.
#[cfg(rcf1_reference_sdk)]
pub(crate) fn verify_automation_workspace(model:&Arc<crate::workspace::Workspace>,
    factories:&crate::workspace::factory::PublishedWorkspaceFactories)->std::result::Result<(),String> {
    let context=current().map_err(|_|"Reference permission context absent/retired")?;
    let module=context.module.get().ok_or("Reference permission imports absent")?;
    let workspace=model.lease();
    if !factories.belongs_to(model)||!Arc::ptr_eq(&workspace.life,&context.life)
        ||!Arc::ptr_eq(&workspace.life,&module.lease.life)
        ||workspace.root.path()!=module.lease.root.path()
        ||root_identity(&workspace.root)?!=module.root_identity {
        return Err("Reference permission/workspace issuer differs".into());
    }
    let selected=crate::workspace::objc::selector_replacement(c"NSAppleEventDescriptor",c"aeDesc",false)
        .ok_or("Reference owned aeDesc leaf missing")?;
    if !factories.covers(c"NSAppleEventDescriptor",c"aeDesc",false,selected)? {
        return Err("Reference permission borrowed target factory unqualified".into());
    }
    context.live().map_err(|_|"Reference permission retired during factory check")?;
    if !PUBLISHED.load(Ordering::Acquire) {return Err("Reference permission unpublished during factory check".into());}
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
pub(crate) fn automation_workspace_context(model:&Arc<crate::workspace::Workspace>,
    factories:&crate::workspace::factory::PublishedWorkspaceFactories)->std::result::Result<Arc<Context>,String> {
    verify_automation_workspace(model,factories)?;
    current().map_err(|_|"Reference automation owner retired after qualification".into())
}
fn root_identity(root: &super::owned::OwnedRoot) -> std::result::Result<(u64, u64), String> {
    use std::os::fd::AsRawFd;
    let directory = root.checked(root.path())?;
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(directory.as_raw_fd(), &mut metadata) } != 0 {
        return Err("Security root descriptor metadata".into());
    }
    Ok((metadata.st_dev as u64, metadata.st_ino))
}
fn import_keys(
    rows: &[super::ImportReplacement],
) -> std::collections::BTreeSet<(&CStr, usize, usize, usize)> {
    rows.iter()
        .map(|row| {
            (
                row.symbol,
                row.original,
                row.replacement,
                row.cell_address(),
            )
        })
        .collect()
}
/// Check only the actual retained module, not a caller's scope/name assertion.
pub(crate) fn verify_attestation(
    module: &super::ModuleAttestation,
) -> std::result::Result<(), String> {
    if module.module != "private-security"
        || !module.scopes.is_empty()
        || module.callsites != source_roster()
        || module.workspace_factories.is_some()
        || module.handles.len() != 1
    {
        return Err("Security module envelope differs".into());
    }
    let context = current_attested()?;
    let owner = context
        .module
        .get()
        .ok_or("Security module was not source-installed")?;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let website = owner.website.get().map_or(&[][..], Vec::as_slice);
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    let website: &[super::ObjcReplacement] = &[];
    if module.objc.len() != website.len() || module.objc.iter().zip(website).any(|(row, expected)| {
        (row.class, row.selector, row.class_method, row.original, row.replacement)
            != (expected.class, expected.selector, expected.class_method, expected.original, expected.replacement)
    }) {
        return Err("Security website selected method roster differs".into());
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if !website.is_empty() {
        web_protocol::verify_installed(&context)
            .inspect_err(|_| context.retired.store(true, Ordering::Release))?;
    }
    let handle = &module.handles[0];
    if handle.namespace != "private-security"
        || !Arc::ptr_eq(&handle.life, &context.life)
        || !Arc::ptr_eq(&handle.life, &owner.lease.life)
        || handle.root.path() != owner.lease.root.path()
        || root_identity(&handle.root)? != owner.root_identity
        || root_identity(&owner.lease.root)? != owner.root_identity
        || module.imports.len() != owner.imports.len()
        || owner.imports.is_empty()
        || import_keys(&owner.imports).len() != owner.imports.len()
        || import_keys(&module.imports) != import_keys(&owner.imports)
    {
        return Err("Security module owner/import identity differs".into());
    }
    let names = owner
        .imports
        .iter()
        .map(|row| row.symbol)
        .collect::<std::collections::BTreeSet<_>>();
    if names.len() != 5
        || owner.imports.iter().any(|row| {
            row.original == 0
                || row.original == row.replacement
                || replacement(row.symbol) != Some(row.replacement)
                || row.selected_address() != row.replacement
        })
    {
        return Err("Security selected import roster differs".into());
    }
    handle.before_effect()?;
    context
        .live()
        .map_err(|_| "Security module retired during verification")?;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if !website.is_empty() {
        let verify_website = || -> std::result::Result<(), String> {
            web_protocol::verify_installed(&context)?;
            for leaf in website {
                if leaf.provenance.verify()? != leaf.replacement {
                    return Err("Security website selected implementation changed".into());
                }
            }
            Ok(())
        };
        verify_website().inspect_err(|_| context.retired.store(true, Ordering::Release))?;
    }
    // No dynamic callback follows this final actual-cell/liveness check.
    if context.retired.load(Ordering::Acquire)
        || !PUBLISHED.load(Ordering::Acquire)
        || owner
            .imports
            .iter()
            .any(|row| row.selected_address() != row.replacement)
    {
        context.retired.store(true, Ordering::Release);
        return Err("Security import changed during final guardian check".into());
    }
    Ok(())
}
pub(crate) fn attestation() -> std::result::Result<super::ModuleAttestation, String> {
    let context = current_attested()?;
    let owner = context
        .module
        .get()
        .ok_or("Security module was not source-installed")?;
    let module = super::ModuleAttestation {
        module: "private-security",
        scopes: scopes(),
        callsites: source_roster(),
        imports: owner.imports.clone(),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        objc: owner.website.get().cloned().unwrap_or_default(),
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        objc: vec![],
        handles: vec![owner.lease.clone()],
        workspace_factories: None,
    };
    verify_attestation(&module)?;
    Ok(module)
}
/// Bind the same guardian/root capability before the existing actual main-image
/// import transaction. Legacy install(context) cannot manufacture this binding.
/// # Safety
/// Exclusive source-qualified SDK setup has verified caller/image/source/ABI
/// and all five actual import cells. prewrite is a pure validator after dynamic
/// guardian callbacks. No original Security operation or fallback is permitted.
pub(crate) unsafe fn initialize_owned(
    context: Arc<Context>,
    lease: super::owned::OwnedHandleLease,
    prewrite: impl Fn() -> std::result::Result<(), String> + Send + Sync,
) -> std::result::Result<super::ModuleAttestation, String> {
    if context.module.get().is_some()
        || lease.namespace != "private-security"
        || !Arc::ptr_eq(&lease.life, &context.life)
    {
        return Err("Security setup context/guardian differs".into());
    }
    lease.before_effect()?;
    context.live().map_err(|_| "Security setup retired")?;
    let root_identity = root_identity(&lease.root)?;
    prewrite()?;
    if context.retired.load(Ordering::Acquire) {
        return Err("Security setup retired".into());
    }
    install(context.clone())?;
    let mut publication = PublicationGuard {
        context: context.clone(),
        complete: false,
    };
    struct InstallLease<'a, F> {
        context: &'a Context,
        lease: &'a super::owned::OwnedHandleLease,
        prewrite: &'a F,
    }
    impl<F: Fn() -> std::result::Result<(), String> + Send + Sync> AliveLease for InstallLease<'_, F> {
        fn is_alive(&self) -> bool {
            self.before_effect().is_ok()
        }
        fn before_effect(&self) -> std::result::Result<(), String> {
            self.lease.before_effect()?;
            self.context.live().map_err(|_| "Security setup retired")?;
            (self.prewrite)()?;
            if self.context.retired.load(Ordering::Acquire) || !PUBLISHED.load(Ordering::Acquire) {
                return Err("Security setup retired before import write".into());
            }
            Ok(())
        }
    }
    let guard = InstallLease {
        context: &context,
        lease: &lease,
        prewrite: &prewrite,
    };
    let imports = super::import_patch::install(&import_specs(), &guard)?;
    if context
        .module
        .set(SecurityModule {
            lease,
            root_identity,
            imports,
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            website: std::sync::OnceLock::new(),
        })
        .is_err()
    {
        return Err("Security module already bound".into());
    }
    let module = attestation()?;
    publication.complete = true;
    Ok(module)
}
pub fn replacement(symbol: &CStr) -> Option<usize> {
    match symbol.to_bytes() {
        b"SecItemCopyMatching" => Some(rcf1_security_copy as *const () as usize),
        b"SecItemUpdate" => Some(rcf1_security_update as *const () as usize),
        b"SecItemAdd" => Some(rcf1_security_add as *const () as usize),
        b"SecRandomCopyBytes" => Some(rcf1_security_random as *const () as usize),
        b"AEDeterminePermissionToAutomateTarget" => {
            Some(rcf1_automation_permission as *const () as usize)
        }
        _ => None,
    }
}

#[cfg(test)]
mod module_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Life;
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            true
        }
        fn before_effect(&self) -> std::result::Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn explicit_retirement_waits_for_the_last_security_commit() {
        let context = Context::new(Arc::new(Life), None, vec![1; 4]).unwrap();
        install(context.clone()).unwrap();
        let publication = context.state(Operation::Entropy).unwrap();
        std::thread::scope(|scope| {
            let (started, waiting) = std::sync::mpsc::channel();
            let (retired, done) = std::sync::mpsc::channel();
            let context = &context;
            scope.spawn(move || {
                started.send(()).unwrap();
                retired.send(retire(context)).unwrap();
            });
            waiting.recv().unwrap();
            let early = done.recv_timeout(std::time::Duration::from_millis(50));
            drop(publication);
            assert!(matches!(early, Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
            done.recv_timeout(std::time::Duration::from_secs(2)).unwrap().unwrap();
        });
        let mut output = [255u8; 4];
        assert_eq!(unsafe { rcf1_security_random(kSecRandomDefault, 4, output.as_mut_ptr().cast()) }, DISABLED);
        assert_eq!(output, [255; 4]);
    }
    #[test]
    fn backend_owner_retirement_revokes_security_before_target_reads_or_outputs() {
        let fixture=crate::workspace::tests::Fixture::new();
        let generation=fixture.model.backend_generation().unwrap();
        let context=Context::new(fixture.model.lease().life,None,vec![1;4]).unwrap();
        // Plain same-owner binding only; this is not a Reference admission or
        // factory grant. The production binder separately checks those proofs.
        assert!(context.backend.set((fixture.model.clone(),generation)).is_ok());
        context.consent(700001,0).unwrap();
        install(context.clone()).unwrap();
        fixture.model.retire().unwrap();
        let mut output=[255u8;4];
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,output.as_mut_ptr().cast())},DISABLED);
        assert_eq!(output,[255;4]);
        let wildcard=u32::from_be_bytes(*b"****");
        assert_eq!(unsafe{rcf1_automation_permission(std::ptr::dangling(),wildcard,wildcard,1)},DISABLED);
        assert!(context.consent(700001,0).is_err());
        retire(&context).unwrap();
    }
    #[test]
    fn security_guard_and_unexpected_status_panics_revoke_inflight_owner() {
        struct Payload(Arc<AtomicUsize>);
        impl Drop for Payload {fn drop(&mut self){self.0.fetch_add(1,Ordering::SeqCst);}}
        struct PanicLife {mode:AtomicUsize,drops:Arc<AtomicUsize>}
        impl AliveLease for PanicLife {
            fn is_alive(&self)->bool {
                if self.mode.load(Ordering::Acquire)==2 {std::panic::panic_any(Payload(self.drops.clone()));}true
            }
            fn before_effect(&self)->std::result::Result<(),String> {
                if self.mode.load(Ordering::Acquire)==1 {std::panic::panic_any(Payload(self.drops.clone()));}Ok(())
            }
        }
        let drops=Arc::new(AtomicUsize::new(0));
        for mode in [1,2] {
            let life=Arc::new(PanicLife{mode:AtomicUsize::new(0),drops:drops.clone()});
            let context=Context::new(life.clone(),None,vec![1;4]).unwrap();
            life.mode.store(mode,Ordering::Release);
            assert!(context.state(Operation::Entropy).is_err());
            assert!(context.retired.load(Ordering::Acquire));
            assert!(context.state.try_lock().is_ok(),"guard panic must not poison State");
            life.mode.store(0,Ordering::Release);
            assert!(context.consent(700001,0).is_err());
            assert_eq!(drops.load(Ordering::SeqCst),0,"unknown panic payload must not run Drop");
        }
        let context=Context::new(Arc::new(Life),None,vec![1;4]).unwrap();
        install(context.clone()).unwrap();
        assert_eq!(status(||std::panic::panic_any(Payload(drops.clone()))),DISABLED);
        assert!(context.check_pure().is_err(),"already in-flight calls need owner revocation");
        assert_eq!(drops.load(Ordering::SeqCst),0,"status must not run unknown payload Drop");
        let mut output=[255u8;4];
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,output.as_mut_ptr().cast())},DISABLED);
        assert_eq!(output,[255;4]);
        retire(&context).unwrap();
    }
    #[test]
    fn security_leaves_reenter_without_state_lock_and_preserve_consent_and_effect_fences() {
        struct Audit {
            context:Mutex<std::sync::Weak<Context>>,
            reconfigure:AtomicUsize,
            revoke:AtomicBool,
        }
        impl AliveLease for Audit {
            fn is_alive(&self)->bool {true}
            fn before_effect(&self)->std::result::Result<(),String> {
                let context=self.context.lock().unwrap().upgrade();
                if let Some(context)=context {
                    let mut state=context.state.try_lock().map_err(|_|"guardian invoked under Security State lock")?;
                    if self.reconfigure.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_sub(1)).unwrap_or(0)==1 {
                        state.consent=Some((700001,-1743));
                    }
                    if self.revoke.load(Ordering::Acquire) {context.retired.store(true,Ordering::Release);}
                }
                Ok(())
            }
        }
        #[link(name="CoreFoundation",kind="framework")]
        unsafe extern "C" {
            fn CFDictionaryCreate(allocator:Ptr,keys:*const Ptr,values:*const Ptr,count:isize,key_callbacks:Ptr,value_callbacks:Ptr)->Ptr;
            static kCFTypeDictionaryKeyCallBacks:u8;
            static kCFTypeDictionaryValueCallBacks:u8;
        }
        #[link(name="CoreServices",kind="framework")]
        unsafe extern "C" {
            fn AECreateDesc(kind:u32,data:Ptr,size:isize,out:*mut AddressDesc)->i16;
            fn AEDisposeDesc(value:*mut AddressDesc)->i16;
        }
        fn text(bytes:&[u8])->Cf {
            let value=unsafe{CFStringCreateWithBytes(std::ptr::null(),bytes.as_ptr(),bytes.len() as isize,0x08000100,0)};
            assert!(!value.is_null());Cf(value)
        }
        fn data(bytes:&[u8])->Cf {
            let value=unsafe{CFDataCreate(std::ptr::null(),bytes.as_ptr(),bytes.len() as isize)};
            assert!(!value.is_null());Cf(value)
        }
        fn dictionary(rows:&[(Ptr,Ptr)])->Cf {
            let keys=rows.iter().map(|row|row.0).collect::<Vec<_>>();
            let values=rows.iter().map(|row|row.1).collect::<Vec<_>>();
            let value=unsafe{CFDictionaryCreate(std::ptr::null(),keys.as_ptr(),values.as_ptr(),rows.len() as isize,
                std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast())};
            assert!(!value.is_null());Cf(value)
        }
        let audit=Arc::new(Audit{context:Mutex::new(std::sync::Weak::new()),reconfigure:AtomicUsize::new(0),revoke:AtomicBool::new(false)});
        let context=Context::new(audit.clone(),None,(0..28).collect()).unwrap();
        *audit.context.lock().unwrap()=Arc::downgrade(&context);
        context.consent(700001,0).unwrap();
        context.fault(Operation::Read,None).unwrap();
        install(context.clone()).unwrap();
        let (service,account,payload,next)=(text(b"viggo-raycast-utilities"),text(b"password-history"),data(b"synthetic-only"),data(b"updated-own"));
        let base=unsafe{[(kSecClass,kSecClassGenericPassword),(kSecAttrService,service.0),(kSecAttrAccount,account.0)]};
        let query=dictionary(&base);
        let mut rows=base.to_vec();rows.push(unsafe{(kSecValueData,payload.0)});let item=dictionary(&rows);
        rows=base.to_vec();rows.extend(unsafe{[(kSecReturnData,kCFBooleanTrue),(kSecMatchLimit,kSecMatchLimitOne)]});let read=dictionary(&rows);
        let changes=dictionary(&[unsafe{(kSecValueData,next.0)}]);
        assert_eq!(unsafe{rcf1_security_add(item.0,std::ptr::null_mut())},0);
        assert_eq!(unsafe{rcf1_security_add(item.0,std::ptr::null_mut())},DUPLICATE);
        let mut refused_add: Ptr = std::ptr::dangling();
        assert_eq!(unsafe { rcf1_security_add(item.0, &mut refused_add) }, PARAM);
        assert_eq!(refused_add, std::ptr::dangling());
        let mut misaligned_storage = [usize::MAX; 2];
        let misaligned = unsafe { misaligned_storage.as_mut_ptr().cast::<u8>().add(1).cast::<Ptr>() };
        assert_eq!(unsafe { rcf1_security_copy(read.0, misaligned) }, PARAM);
        assert_eq!(misaligned_storage, [usize::MAX; 2]);
        let mut output=std::ptr::null();
        assert_eq!(unsafe{rcf1_security_copy(read.0,&mut output)},0);
        let copied=Cf(output);assert_eq!(unsafe{bytes(&context,copied.0)}.unwrap(),b"synthetic-only");
        assert_eq!(unsafe{rcf1_security_update(query.0,changes.0)},0);
        assert_eq!(unsafe{rcf1_security_copy(read.0,&mut output)},0);
        let copied=Cf(output);assert_eq!(unsafe{bytes(&context,copied.0)}.unwrap(),b"updated-own");
        context.fault(Operation::Read,Some(PARAM)).unwrap();
        output=std::ptr::dangling();assert_eq!(unsafe{rcf1_security_copy(read.0,&mut output)},PARAM);assert!(output.is_null());
        let mut random=[255u8;4];
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,random.as_mut_ptr().cast())},0);assert_eq!(random,[0,1,2,3]);
        let mut remaining=[255u8;24];
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,24,remaining.as_mut_ptr().cast())},0);assert_eq!(remaining,std::array::from_fn(|i|i as u8+4));
        random=[255;4];assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,random.as_mut_ptr().cast())},DISABLED);assert_eq!(random,[255;4]);
        let pid=700001i32;let mut address=AddressDesc{descriptor_type:0,data_handle:std::ptr::null_mut()};
        assert_eq!(unsafe{AECreateDesc(u32::from_be_bytes(*b"kpid"),(&pid as *const i32).cast(),4,&mut address)},0);
        let wildcard=u32::from_be_bytes(*b"****");
        assert_eq!(unsafe{rcf1_automation_permission(&address,wildcard,wildcard,1)},0);
        assert_eq!(unsafe{rcf1_automation_permission(&address,wildcard,wildcard,2)},PARAM);
        let other_pid=700002i32;let mut other=AddressDesc{descriptor_type:0,data_handle:std::ptr::null_mut()};
        assert_eq!(unsafe{AECreateDesc(u32::from_be_bytes(*b"kpid"),(&other_pid as *const i32).cast(),4,&mut other)},0);
        assert_eq!(unsafe{rcf1_automation_permission(&other,wildcard,wildcard,1)},PROCESS_NOT_FOUND);
        assert_eq!(unsafe{AEDisposeDesc(&mut other)},0);
        audit.reconfigure.store(2,Ordering::Release);
        assert_eq!(unsafe{rcf1_automation_permission(&address,wildcard,wildcard,1)},DISABLED,"a changed disposition must not escape as stale success");
        assert_eq!(unsafe{rcf1_automation_permission(&address,wildcard,wildcard,1)},-1743);
        assert_eq!(unsafe{AEDisposeDesc(&mut address)},0);
        audit.revoke.store(true,Ordering::Release);
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,random.as_mut_ptr().cast())},DISABLED);assert_eq!(random,[255;4]);
        assert_eq!(unsafe{rcf1_automation_permission(std::ptr::dangling(),wildcard,wildcard,1)},DISABLED);
        audit.revoke.store(false,Ordering::Release);
        assert_eq!(unsafe{rcf1_security_random(kSecRandomDefault,4,random.as_mut_ptr().cast())},DISABLED,"retirement must remain sticky");
        retire(&context).unwrap();
    }
    #[test]
    fn security_module_specs_keep_five_distinct_leaves_and_no_whole_scope() {
        let specs = import_specs();
        assert_eq!(specs.len(), 5);
        let actual = specs
            .iter()
            .map(|row| row.symbol)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = [
            c"SecItemCopyMatching",
            c"SecItemUpdate",
            c"SecItemAdd",
            c"SecRandomCopyBytes",
            c"AEDeterminePermissionToAutomateTarget",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
        for row in specs {
            assert_ne!(row.original, 0);
            assert_ne!(row.original, row.replacement);
            assert_eq!(Some(row.replacement), replacement(row.symbol));
        }
        assert!(replacement(c"AESendMessage").is_none());
        assert!(replacement(c"SecItemDelete").is_none());
        assert!(scopes().is_empty());
        let sources = source_roster();
        assert_eq!(sources.len(), 2);
        assert!(sources
            .iter()
            .all(|source| source.source_sha256.len() == 64));
    }
    #[test]
    fn security_module_panicking_postpublication_guard_retires_without_import_effects() {
        use std::os::unix::fs::DirBuilderExt;
        let parent = std::path::PathBuf::from(
            std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT")
                .expect("explicit private fixture parent"),
        );
        assert!(parent.is_absolute());
        let root = parent.join(format!("security-panic-module-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let life: Arc<dyn AliveLease> = Arc::new(Life);
        let context = Context::new(life.clone(), None, vec![0; 24]).unwrap();
        let lease = super::super::owned::OwnedHandleLease {
            namespace: "private-security",
            root: super::super::owned::OwnedRoot::open(&root).unwrap(),
            life,
        };
        let calls = AtomicUsize::new(0);
        let result = catch_unwind(AssertUnwindSafe(|| unsafe {
            initialize_owned(context.clone(), lease, || {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(
                    !PUBLISHED.load(Ordering::Acquire),
                    "own postpublication prewrite panic"
                );
                Ok(())
            })
        }));
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(context.retired.load(Ordering::Acquire));
        assert!(!PUBLISHED.load(Ordering::Acquire));
        assert!(context.module.get().is_none());
        assert!(context
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|count| *count == 0));
        assert!(current().is_err());
        assert!(attestation().is_err());
        assert!(install(context.clone()).is_err());
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn security_module_foreign_guardian_fails_before_publication_or_prewrite() {
        let parent = std::path::PathBuf::from(
            std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT")
                .expect("explicit private fixture parent"),
        );
        assert!(parent.is_absolute());
        let root = parent.join(format!("security-module-{}", std::process::id()));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let retained_root = super::super::owned::OwnedRoot::open(&root).unwrap();
        let original_identity = root_identity(&retained_root).unwrap();
        let moved = root.with_extension("retained");
        std::fs::rename(&root, &moved).unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let replaced_root = super::super::owned::OwnedRoot::open(&root).unwrap();
        assert_eq!(root_identity(&retained_root).unwrap(), original_identity);
        assert_ne!(root_identity(&replaced_root).unwrap(), original_identity);
        let life: Arc<dyn AliveLease> = Arc::new(Life);
        let foreign: Arc<dyn AliveLease> = Arc::new(Life);
        let context = Context::new(life, None, vec![0; 24]).unwrap();
        let lease = super::super::owned::OwnedHandleLease {
            namespace: "private-security",
            root: super::super::owned::OwnedRoot::open(&root).unwrap(),
            life: foreign,
        };
        let calls = AtomicUsize::new(0);
        let result = unsafe {
            initialize_owned(context.clone(), lease, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        };
        assert!(
            matches!(result, Err(ref error) if error == "Security setup context/guardian differs")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(context.module.get().is_none());
        assert!(context
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|count| *count == 0));
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(moved).unwrap();
    }
}
