//! Typed payloads in the existing common receiver arena. No second address
//! registry, original Foundation/Pasteboard dispatch, or admission flag.
use super::*;
use crate::sdk::clipboard;
use foundation::{FoundationValue, Text, Url};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FoundationRole {
    String,
    Data,
    Url,
    Bool,
    Array,
    Dictionary,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipboardRole {
    Board,
    Item,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Foundation(FoundationRole),
    Clipboard(ClipboardRole),
}
impl Role {
    fn name(self) -> &'static CStr {
        match self {
            Self::Foundation(FoundationRole::String) => c"NSString",
            Self::Foundation(FoundationRole::Data) => c"NSData",
            Self::Foundation(FoundationRole::Url) => c"NSURL",
            Self::Foundation(FoundationRole::Bool) => c"NSNumber",
            Self::Foundation(FoundationRole::Array) => c"NSArray",
            Self::Foundation(FoundationRole::Dictionary) => c"NSDictionary",
            Self::Clipboard(ClipboardRole::Board) => c"NSPasteboard",
            Self::Clipboard(ClipboardRole::Item) => c"NSPasteboardItem",
        }
    }
}
#[derive(Clone)]
enum Payload {
    Pending,
    Value(FoundationValue),
    Array(Vec<FoundationReference>),
    Dictionary(Vec<(FoundationReference, FoundationReference)>),
    StringDictionary(Vec<(FoundationReference, FoundationReference)>),
    ErrorDictionary(Vec<(FoundationReference, FoundationReference)>),
    Board(Arc<clipboard::Board>),
    Item(Arc<clipboard::Item>),
}
struct Record {
    model: Arc<Workspace>,
    key: Handle,
    owner: std::thread::ThreadId,
    live: AtomicBool,
    ready: AtomicBool,
    shareable: AtomicBool,
    serial: AtomicU64,
    role: Role,
    payload: Mutex<Payload>,
}
#[derive(Clone)]
pub(super) struct Identity(Arc<Record>);
impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Identity {}
pub(super) fn role(identity: &Identity) -> &'static CStr {
    identity.0.role.name()
}

pub(super) struct State {
    model: Arc<Workspace>,
    owner: std::thread::ThreadId,
    initializing_clipboard: AtomicBool,
    initializing_owner: AtomicBool,
    clipboard: Mutex<Option<Arc<clipboard::Clipboard>>>,
    general_owner: Mutex<Option<FoundationReference>>,
    file_key_owner: Mutex<Option<FoundationReference>>,
}
impl State {
    pub(super) fn new(model: &Arc<Workspace>) -> Result<Self> {
        Ok(Self {
            model: model.clone(),
            owner: std::thread::current().id(),
            initializing_clipboard: AtomicBool::new(false),
            initializing_owner: AtomicBool::new(false),
            clipboard: Mutex::new(None),
            general_owner: Mutex::new(None),
            file_key_owner: Mutex::new(None),
        })
    }
}
struct Flag<'a>(&'a AtomicBool);
impl<'a> Flag<'a> {
    fn take(flag: &'a AtomicBool) -> Result<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "reentrant typed data operation".to_string())?;
        Ok(Self(flag))
    }
}
impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
thread_local! {
    static CHECKING: Cell<bool> = const { Cell::new(false) };
    static CONSTRUCTING: Cell<bool> = const { Cell::new(false) };
    static LEAF_ACTIVE: Cell<bool> = const { Cell::new(false) };
}
struct ThreadFlag(&'static std::thread::LocalKey<Cell<bool>>);
impl ThreadFlag {
    fn take(key: &'static std::thread::LocalKey<Cell<bool>>) -> Result<Self> {
        let occupied = key
            .try_with(|value| value.replace(true))
            .map_err(|_| "typed data thread is exiting")?;
        if occupied {
            return Err("reentrant typed data operation".into());
        }
        Ok(Self(key))
    }
}
impl Drop for ThreadFlag {
    fn drop(&mut self) {
        let _ = self.0.try_with(|value| value.set(false));
    }
}
fn installed() -> Result<&'static Installed> {
    INSTALLED.get().ok_or("workspace absent".into())
}
fn pure_model(model: &Arc<Workspace>) -> Result<u64> {
    let installed = installed()?;
    if !Arc::ptr_eq(&installed.model, model)
        || !Arc::ptr_eq(&installed.typed_data.model, model)
        || model.faulted.load(Ordering::Acquire)
    {
        return Err("typed data context retired, foreign or unowned".into());
    }
    let state = model.state.lock().map_err(|_| "workspace state poisoned")?;
    if state.retired {
        return Err("typed data generation retired".into());
    }
    Ok(state.generation)
}
fn context() -> Result<&'static Installed> {
    let installed = installed()?;
    pure_model(&installed.model)?;
    let _checking = ThreadFlag::take(&CHECKING)?;
    installed.model.with(|_| Ok(()))?;
    pure_model(&installed.model)?;
    Ok(installed)
}
fn clipboard_owner() -> Result<()> {
    data_role_allowed(c"NSPasteboard")?;
    if installed()?.typed_data.owner != std::thread::current().id() {
        return Err("clipboard context belongs to its setup owner".into());
    }
    Ok(())
}
pub(super) fn foundation_entry_allowed() -> bool {
    let Some(installed) = INSTALLED.get() else {
        return false;
    };
    if !CHECKING.try_with(|value| !value.get()).unwrap_or(false)
        || !LEAF_ACTIVE.try_with(|value| !value.get()).unwrap_or(false)
        || !CONSTRUCTING.try_with(|value| !value.get()).unwrap_or(false)
        || installed.model.faulted.load(Ordering::Acquire)
    {
        return false;
    }
    installed
        .model
        .state
        .lock()
        .is_ok_and(|state| !state.retired)
}
pub(super) struct EntryGuard {
    _flag: ThreadFlag,
}
pub(super) fn enter_foundation_leaf() -> Result<EntryGuard> {
    if !foundation_entry_allowed() {
        return Err("typed data entry denied".into());
    }
    Ok(EntryGuard {
        _flag: ThreadFlag::take(&LEAF_ACTIVE)?,
    })
}
fn new_identity(role: Role, payload: Payload) -> Result<Identity> {
    data_role_allowed(role.name())?;
    let installed = context()?;
    let key = installed.model.with(|state| {
        let key = Handle {
            id: state.next,
            generation: state.generation,
        };
        state.next = state
            .next
            .checked_add(1)
            .ok_or("typed identities exhausted")?;
        Ok(key)
    })?;
    pure_model(&installed.model)?;
    Ok(Identity(Arc::new(Record {
        model: installed.model.clone(),
        key,
        owner: std::thread::current().id(),
        live: AtomicBool::new(true),
        ready: AtomicBool::new(false),
        shareable: AtomicBool::new(false),
        serial: AtomicU64::new(0),
        role,
        payload: Mutex::new(payload),
    })))
}
fn identity_at(pointer: *mut AnyObject) -> Result<Identity> {
    let identity = {
        let objects = installed()?
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {
            Some(Kind::Data(identity)) => identity.clone(),
            _ => return Err("unknown or wrong-kind typed receiver".into()),
        }
    };
    check(pointer, &identity)?;
    Ok(identity)
}
fn pure_check(pointer: *mut AnyObject, identity: &Identity) -> Result<()> {
    data_role_allowed(role(identity))?;
    let record = &identity.0;
    if !record.live.load(Ordering::Acquire)
        || !record.ready.load(Ordering::Acquire)
        || pure_model(&record.model)? != record.key.generation
    {
        return Err("typed receiver retired or generation expired".into());
    }
    if record.owner != std::thread::current().id()
        && !(record.shareable.load(Ordering::Acquire)
            && is_shareable(identity)?
            && super::super::factory::is_immutable_data(pointer, &record.model)?)
    {
        return Err("typed receiver is exclusive to its owner".into());
    }
    let serial = record.serial.load(Ordering::Acquire);
    if serial == 0 || super::super::factory::owned_reference_serial(pointer)? != serial {
        return Err("typed allocation serial expired".into());
    }
    check_arena_publication(pointer, identity)
}
fn check_arena_publication(pointer: *mut AnyObject, identity: &Identity) -> Result<()> {
    let objects = installed()?
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?;
    if !matches!(objects.get(&(pointer as usize)),Some(Kind::Data(current)) if current == identity)
    {
        return Err("typed receiver publication changed".into());
    }
    Ok(())
}
fn check(pointer: *mut AnyObject, identity: &Identity) -> Result<()> {
    pure_check(pointer, identity)?;
    {
        let _checking = ThreadFlag::take(&CHECKING)?;
        let physical = super::super::factory::receiver_identity(pointer)?;
        if !physical.belongs_to(&identity.0.model) {
            return Err("foreign typed allocation".into());
        }
        physical.verify()?;
    }
    pure_check(pointer, identity)
}
fn payload(pointer: *mut AnyObject) -> Result<Payload> {
    let identity = identity_at(pointer)?;
    let value = identity
        .0
        .payload
        .lock()
        .map_err(|_| "typed payload poisoned")?
        .clone();
    pure_check(pointer, &identity)?;
    Ok(value)
}
fn create(role: Role, payload: Payload) -> Result<*mut AnyObject> {
    let _constructing = ThreadFlag::take(&CONSTRUCTING)?;
    let identity = new_identity(role, payload)?;
    let pointer = object(Kind::Data(identity.clone()))?;
    let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?;
    identity.0.serial.store(
        super::super::factory::owned_reference_serial(pointer)?,
        Ordering::Release,
    );
    context()?;
    identity.0.ready.store(true, Ordering::Release);
    publish_shareability(pointer, &identity)?;
    pure_check(pointer, &identity)?;
    Ok(pending.into_raw())
}
fn publish_shareability(pointer: *mut AnyObject, identity: &Identity) -> Result<()> {
    if is_shareable(identity)? {
        // SAFETY: exact READY private payload proof above; the factory repeats
        // identity/role/serial/model and immutable_payload before policy change.
        unsafe {
            super::super::factory::publish_immutable_data(
                pointer,
                role(identity),
                &identity.0.model,
            )
        }?;
        identity.0.shareable.store(true, Ordering::Release);
    }
    Ok(())
}
/// Computed from the actual READY private payload. This grants no receiver
/// policy by itself; the common factory separately checks class/serial/model.
/// No callback or original Foundation method is invoked during this proof.
pub(super) fn is_shareable(identity: &Identity) -> Result<bool> {
    let record = &identity.0;
    if !record.live.load(Ordering::Acquire)
        || !record.ready.load(Ordering::Acquire)
        || !matches!(record.role, Role::Foundation(_))
        || pure_model(&record.model)? != record.key.generation
    {
        return Ok(false);
    }
    let payload = record
        .payload
        .lock()
        .map_err(|_| "typed payload poisoned")?
        .clone();
    match payload {
        Payload::Value(value) => Ok(record.role == Role::Foundation(value_role(&value))),
        Payload::Array(values) => {
            if record.role != Role::Foundation(FoundationRole::Array) {
                return Ok(false);
            }
            for value in values {
                if !immutable_child(&value, &record.model, record.key.generation)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Payload::Dictionary(values)
        | Payload::StringDictionary(values)
        | Payload::ErrorDictionary(values) => {
            if record.role != Role::Foundation(FoundationRole::Dictionary) {
                return Ok(false);
            }
            for (key, value) in values {
                if !immutable_child(&key, &record.model, record.key.generation)?
                    || !immutable_child(&value, &record.model, record.key.generation)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Payload::Pending | Payload::Board(_) | Payload::Item(_) => Ok(false),
    }
}
pub(super) fn immutable_payload(
    pointer: *mut AnyObject,
    identity: &Identity,
    model: &Arc<Workspace>,
) -> Result<bool> {
    if !Arc::ptr_eq(&identity.0.model, model)
        || identity.0.serial.load(Ordering::Acquire) == 0
        || super::super::factory::owned_reference_serial(pointer)?
            != identity.0.serial.load(Ordering::Acquire)
    {
        return Ok(false);
    }
    {
        let objects = installed()?
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        if !matches!(objects.get(&(pointer as usize)),Some(Kind::Data(current)) if current == identity)
        {
            return Ok(false);
        }
    }
    is_shareable(identity)
}
fn immutable_child(
    reference: &FoundationReference,
    model: &Arc<Workspace>,
    generation: u64,
) -> Result<bool> {
    if !Arc::ptr_eq(&reference.0.model, model) || reference.0.generation != generation {
        return Ok(false);
    }
    match &reference.0.held {
        // This enum alternative is constructed only after exact NSURL source/
        // proxy class qualification. Classes are not adopted ARC allocations.
        Held::UrlClass(_) => Ok(true),
        // UI/control/application children are always owner-only, even when
        // the surrounding array payload itself has become READY.
        Held::Arena(held, role) if *role == c"NSError" => {
            let pointer = held.pointer()?;
            Ok(super::super::factory::is_immutable_data(pointer, model)?
                && super::error_payload_for_factory(pointer, model)?)
        }
        Held::Arena(_, _) => Ok(false),
        Held::Object(reference) => {
            let pointer = reference.pointer()?;
            if !super::super::factory::is_immutable_data(pointer, model)? {
                return Ok(false);
            }
            let child = {
                let objects = installed()?
                    .objects
                    .lock()
                    .map_err(|_| "object arena poisoned")?;
                match objects.get(&(pointer as usize)) {
                    Some(Kind::Data(child)) => child.clone(),
                    _ => return Ok(false),
                }
            };
            Ok(Arc::ptr_eq(&child.0.model, model)
                && child.0.key.generation == generation
                && child.0.live.load(Ordering::Acquire)
                && child.0.ready.load(Ordering::Acquire)
                && child.0.shareable.load(Ordering::Acquire)
                && matches!(child.0.role, Role::Foundation(_)))
        }
    }
}
fn bind(
    receiver: *mut AnyObject,
    role: Role,
    prepare: impl FnOnce() -> Result<Payload>,
) -> Result<*mut AnyObject> {
    data_role_allowed(role.name())?;
    let installed = installed()?;
    let _constructing = ThreadFlag::take(&CONSTRUCTING)?;
    super::super::factory::claim_owned_role(
        receiver,
        role.name(),
        &installed.model,
        data_deallocator(role.name()) as *const () as usize,
    )?;
    // SAFETY: exact role/deallocator/model and atomic allocated claim succeeded;
    // only now is the incoming source initializer's +1 ours to consume.
    let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(receiver) }?;
    let identity = new_identity(role, Payload::Pending)?;
    {
        let mut objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        if objects.contains_key(&(receiver as usize)) {
            return Err("typed initializer replay".into());
        }
        objects.insert(receiver as usize, Kind::Data(identity.clone()));
    }
    let prepared = prepare()?;
    context()?;
    unsafe { super::super::factory::initialize(receiver, &installed.model) }?;
    identity.0.serial.store(
        super::super::factory::owned_reference_serial(receiver)?,
        Ordering::Release,
    );
    let old = {
        let mut payload = identity
            .0
            .payload
            .lock()
            .map_err(|_| "typed payload poisoned")?;
        std::mem::replace(&mut *payload, prepared)
    };
    drop(old);
    context()?;
    identity.0.ready.store(true, Ordering::Release);
    publish_shareability(receiver, &identity)?;
    pure_check(receiver, &identity)?;
    Ok(pending.into_raw())
}
pub(super) unsafe extern "C" fn dealloc(receiver: *mut AnyObject, _: Sel) {
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        super::super::factory::deallocate_owned(receiver, || {
            let removed = {
                let mut objects = installed()?
                    .objects
                    .lock()
                    .map_err(|_| "object arena poisoned")?;
                match objects.get(&(receiver as usize)) {
                    Some(Kind::Data(identity)) => {
                        identity.0.live.store(false, Ordering::Release);
                        objects.remove(&(receiver as usize))
                    }
                    None => None,
                    Some(_) => return Err("typed deallocation kind differs".into()),
                }
            };
            // Strong child ARC and any destructor run after the sole arena lock.
            drop(removed);
            Ok(())
        })
    }));
    if !matches!(&result, Ok(Ok(()))) {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if crate::sdk::private_network::reference_website_assembly_active()
            && !receiver.is_null()
            && result.as_ref().is_ok_and(|error| error.as_ref().is_err_and(|error| error == "unknown SDK allocation receiver"))
        {
            if let Some(class) = unsafe { objc2::ffi::object_getClass(receiver).as_ref() } {
                crate::sdk::private_network::report_reference_website_data_dealloc(class.name());
            }
        }
        if let Err(payload) = result {
            std::mem::forget(payload);
        }
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_DATA_DEALLOC: OnceLock<(usize, crate::sdk::metadata::CodeIdentity)> = OnceLock::new();

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
const WEBSITE_PLATFORM_DATA_ROLES: [&CStr; 4] = [c"NSString", c"NSArray", c"NSDictionary", c"NSData"];

fn data_deallocator(_role: &CStr) -> unsafe extern "C" fn(*mut AnyObject, Sel) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if WEBSITE_PLATFORM_DATA_ROLES.contains(&_role) {
        return reference_data_dealloc;
    }
    dealloc
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn select_reference_data_dealloc(class: &AnyClass)
    -> Result<unsafe extern "C" fn(*mut AnyObject, Sel)> {
    if !WEBSITE_PLATFORM_DATA_ROLES.contains(&class.name()) {
        return Ok(data_deallocator(class.name()));
    }
    let handle = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
    let original = crate::sdk::metadata::method(handle, c"dealloc", false)?
        .ok_or("Reference platform data deallocation absent")?;
    let root = crate::sdk::metadata::loaded_class(c"NSObject")?;
    if original.owner() != root.address() || original.encoding() != b"v16@0:8"
        || original.code().image() != b"/usr/lib/libobjc.A.dylib" || original.code().offset() != 0xb328
    {
        return Err("Reference platform data deallocation ABI differs".into());
    }
    let baseline = (original.implementation(), original.code().clone());
    if PLATFORM_DATA_DEALLOC.get_or_init(|| baseline.clone()) != &baseline {
        return Err("Reference platform data deallocation roots differ".into());
    }
    Ok(data_deallocator(class.name()))
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_data_dealloc(receiver: *mut AnyObject, selector: Sel) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<bool> {
        if receiver.is_null() || selector != objc2::sel!(dealloc) {
            return Err("Reference data deallocation receiver or selector differs".into());
        }
        if !crate::sdk::private_network::reference_website_assembly_started() {
            return Ok(false);
        }
        // The concrete Foundation types observed in the original WebsiteIcons
        // lifecycle reach these parent slots via their own dealloc/super call,
        // including pools released after the probe has closed its ingress.
        // SDK proxies keep using the sole allocation ledger and typed disposal.
        let concrete = unsafe { objc2::ffi::object_getClass(receiver).as_ref() }
            .ok_or("Reference data deallocation concrete class absent")?;
        let family = match concrete.name().to_bytes() {
            b"NSCheapMutableString" | b"NSPathStore2" => c"NSString",
            b"__NSArrayI" | b"__NSArrayI_Transfer" | b"__NSArrayM" | b"__NSSingleObjectArrayI" => c"NSArray",
            b"__NSDictionaryI" | b"__NSDictionaryM" | b"__NSFrozenDictionaryM" | b"__NSSingleEntryDictionaryI" => c"NSDictionary",
            b"Foundation.__NSSwiftData" | b"NSConcreteData" | b"OS_dispatch_data" | b"_NSInlineData" => c"NSData",
            _ => return Ok(false),
        };
        if AnyClass::get(concrete.name()) != Some(concrete) {
            return Err("Reference platform data class registration differs".into());
        }
        let mut ancestor = concrete.superclass();
        while let Some(class) = ancestor {
            if class.name() == family {
                break;
            }
            ancestor = class.superclass();
        }
        if ancestor.is_none() {
            return Err("Reference platform data class family differs".into());
        }
        let (original, identity) = PLATFORM_DATA_DEALLOC.get().ok_or("Reference platform data deallocation unbound")?;
        if crate::sdk::metadata::code_identity(*original)? != *identity {
            return Err("Reference platform data deallocation code differs".into());
        }
        let original: unsafe extern "C" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(*original) };
        unsafe { original(receiver, selector) };
        Ok(true)
    }));
    match result {
        Ok(Ok(true)) => (),
        Ok(Ok(false)) => unsafe { dealloc(receiver, selector) },
        _ => unsafe { libc::_exit(89) },
    }
}

pub(super) fn foundation_class(class: *mut AnyClass, role: FoundationRole) -> Result<()> {
    data_role_allowed(Role::Foundation(role).name())?;
    let installed = context()?;
    let _checking = ThreadFlag::take(&CHECKING)?;
    super::super::factory::class_receiver(class, Role::Foundation(role).name(), &installed.model)?;
    pure_model(&installed.model).map(|_| ())
}
pub(super) fn clipboard_class(class: *mut AnyClass, role: ClipboardRole) -> Result<()> {
    data_role_allowed(Role::Clipboard(role).name())?;
    clipboard_owner()?;
    let installed = context()?;
    let _checking = ThreadFlag::take(&CHECKING)?;
    super::super::factory::class_receiver(class, Role::Clipboard(role).name(), &installed.model)?;
    pure_model(&installed.model).map(|_| ())
}
fn value_role(value: &FoundationValue) -> FoundationRole {
    match value {
        FoundationValue::String(_) => FoundationRole::String,
        FoundationValue::Data(_) => FoundationRole::Data,
        FoundationValue::Url(_) => FoundationRole::Url,
        FoundationValue::Bool(_) => FoundationRole::Bool,
    }
}
pub(super) fn new_foundation_value(value: FoundationValue) -> Result<*mut AnyObject> {
    create(Role::Foundation(value_role(&value)), Payload::Value(value))
}
pub(super) fn foundation_value(pointer: *mut AnyObject) -> Result<FoundationValue> {
    match payload(pointer)? {
        Payload::Value(value) => Ok(value),
        _ => Err("not a scalar Foundation receiver".into()),
    }
}
pub(super) fn foundation_utf16(pointer: *mut AnyObject) -> Result<Vec<u16>> {
    match foundation_value(pointer)? {
        FoundationValue::String(value) => Ok(value.utf16().to_vec()),
        _ => Err("not qualified text".into()),
    }
}
pub(super) fn foundation_bytes(pointer: *mut AnyObject) -> Result<Vec<u8>> {
    match foundation_value(pointer)? {
        FoundationValue::Data(value) => Ok(value.bytes().to_vec()),
        _ => Err("not qualified bytes".into()),
    }
}
pub(super) fn foundation_url(pointer: *mut AnyObject) -> Result<Url> {
    match foundation_value(pointer)? {
        FoundationValue::Url(value) => Ok((*value).clone()),
        _ => Err("not qualified URL".into()),
    }
}
pub(super) fn bind_foundation_initializer(
    receiver: *mut AnyObject,
    role: FoundationRole,
    prepare: impl FnOnce() -> Result<FoundationValue>,
) -> Result<*mut AnyObject> {
    bind(receiver, Role::Foundation(role), || {
        let value = prepare()?;
        if value_role(&value) != role {
            return Err("initializer produced wrong scalar role".into());
        }
        Ok(Payload::Value(value))
    })
}
enum Held {
    Object(super::super::factory::RetainedReference),
    Arena(super::super::factory::RetainedReference, &'static CStr),
    UrlClass(usize),
}
struct Reference {
    model: Arc<Workspace>,
    generation: u64,
    held: Held,
}
#[derive(Clone)]
pub(super) struct FoundationReference(Arc<Reference>);
impl FoundationReference {
    pub(super) fn as_ptr(&self) -> Result<*mut AnyObject> {
        if pure_model(&self.0.model)? != self.0.generation {
            return Err("foundation reference context expired".into());
        }
        let pointer = match &self.0.held {
            Held::Object(reference) => {
                let pointer = reference.pointer()?;
                identity_at(pointer)?;
                pointer
            }
            Held::Arena(reference, role) => {
                let pointer = reference.pointer()?;
                if qualified_arena_reference_role(pointer)? != *role {
                    return Err("owner-only collection child role changed".into());
                }
                pointer
            }
            Held::UrlClass(pointer) => {
                foundation_class(*pointer as *mut AnyClass, FoundationRole::Url)?;
                *pointer as *mut AnyObject
            }
        };
        if pure_model(&self.0.model)? != self.0.generation {
            return Err("reference generation changed".into());
        }
        Ok(pointer)
    }
}
/// Final import-lease check of an already retained immutable NSString. This
/// path cannot invoke model.with, a guardian, or original/business callbacks.
pub(super) fn verify_constant_reference(
    reference: &FoundationReference,
    expected_utf16: &[u16],
) -> Result<usize> {
    let Held::Object(held) = &reference.0.held else {
        return Err("constant requires a retained owned String".into());
    };
    if pure_model(&reference.0.model)? != reference.0.generation {
        return Err("constant retained generation expired".into());
    }
    let (pointer, serial) = held.immutable_constant_identity(&reference.0.model)?;
    let identity = {
        let objects = installed()?
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {
            Some(Kind::Data(identity)) => identity.clone(),
            _ => return Err("constant has no sole-arena data identity".into()),
        }
    };
    let record = &identity.0;
    let check_constant = || -> Result<()> {
        data_role_allowed(role(&identity))?;
        if record.role != Role::Foundation(FoundationRole::String)
            || !record.live.load(Ordering::Acquire)
            || !record.ready.load(Ordering::Acquire)
            || !record.shareable.load(Ordering::Acquire)
            || !Arc::ptr_eq(&record.model, &reference.0.model)
            || record.key.generation != reference.0.generation
            || pure_model(&record.model)? != record.key.generation
            || serial == 0 || record.serial.load(Ordering::Acquire) != serial
        {
            return Err("constant is not a qualified immutable owned String".into());
        }
        check_arena_publication(pointer, &identity)
    };
    // The retained +1 and the exact immutable allocation serial cover this
    // callback-free payload read. Recheck the arena around it without asking
    // the same class grant to prove its cleanup roster again for each read.
    check_constant()?;
    let payload = identity
        .0
        .payload
        .lock()
        .map_err(|_| "typed payload poisoned")?
        .clone();
    match payload {
        Payload::Value(FoundationValue::String(text)) if text.utf16() == expected_utf16 => (),
        _ => return Err("constant String value differs from source observation".into()),
    }
    check_constant()?;
    Ok(pointer as usize)
}
pub(super) fn foundation_reference(pointer: *mut AnyObject) -> Result<FoundationReference> {
    let installed = context()?;
    let known_kind = {
        let objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
        objects.get(&(pointer as usize)).cloned()
    };
    let held = if matches!(known_kind, Some(Kind::Data(_))) {
        let identity = identity_at(pointer)?;
        let reference =
            super::super::factory::RetainedReference::acquire(pointer, &installed.model)?;
        pure_check(pointer, &identity)?;
        Held::Object(reference)
    } else if known_kind.is_some() {
        let role = qualified_arena_reference_role(pointer)?;
        let reference = super::super::factory::RetainedReference::acquire(pointer, &installed.model)?;
        if qualified_arena_reference_role(pointer)? != role {
            return Err("owner-only collection child changed during retain".into());
        }
        Held::Arena(reference, role)
    } else {
        foundation_class(pointer.cast(), FoundationRole::Url)?;
        Held::UrlClass(pointer as usize)
    };
    let reference = FoundationReference(Arc::new(Reference {
        model: installed.model.clone(),
        generation: pure_model(&installed.model)?,
        held,
    }));
    reference.as_ptr()?;
    Ok(reference)
}
/// # Safety
/// Consumes exactly one fresh caller-owned common-factory +1. Serial identity
/// alone is not a proof of ownership; callers must hold that actual reference.
pub(super) unsafe fn foundation_created_reference(
    pointer: *mut AnyObject,
) -> Result<FoundationReference> {
    let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?;
    let reference = foundation_reference(pending.as_ptr())?;
    drop(pending);
    Ok(reference)
}
fn validate_references(values: &[FoundationReference]) -> Result<()> {
    let installed = context()?;
    for value in values {
        if !Arc::ptr_eq(&value.0.model, &installed.model) {
            return Err("foreign collection child".into());
        }
        value.as_ptr()?;
    }
    pure_model(&installed.model).map(|_| ())
}
pub(super) fn new_foundation_array(values: Vec<FoundationReference>) -> Result<*mut AnyObject> {
    validate_references(&values)?;
    create(
        Role::Foundation(FoundationRole::Array),
        Payload::Array(values),
    )
}
pub(super) fn foundation_array_elements(
    pointer: *mut AnyObject,
) -> Result<Vec<FoundationReference>> {
    match payload(pointer)? {
        Payload::Array(values) => {
            validate_references(&values)?;
            Ok(values)
        }
        _ => Err("not a qualified array".into()),
    }
}
pub(super) fn bind_foundation_array_initializer(
    receiver: *mut AnyObject,
    prepare: impl FnOnce() -> Result<Vec<FoundationReference>>,
) -> Result<*mut AnyObject> {
    bind(receiver, Role::Foundation(FoundationRole::Array), || {
        let values = prepare()?;
        validate_references(&values)?;
        Ok(Payload::Array(values))
    })
}
fn validate_dictionary(values: &[(FoundationReference, FoundationReference)]) -> Result<()> {
    for (key, value) in values {
        foundation_utf16(key.as_ptr()?)?;
        if !matches!(foundation_value(value.as_ptr()?)?, FoundationValue::Bool(_)) {
            return Err("dictionary option value is not Boolean".into());
        }
    }
    Ok(())
}
fn insert_dictionary_string(
    result: &mut BTreeMap<String, String>,
    key: &[u16],
    value: &[u16],
) -> Result<()> {
    let key = String::from_utf16(key).map_err(|_| "dictionary key is not valid Unicode")?;
    let value = String::from_utf16(value).map_err(|_| "dictionary value is not valid Unicode")?;
    if result.contains_key(&key) {
        return Err("duplicate String dictionary key".into());
    }
    result.insert(key, value);
    Ok(())
}
fn decode_string_dictionary(
    values: &[(FoundationReference, FoundationReference)],
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for (key, value) in values {
        // Both reads traverse the sole arena, exact receiver identity and
        // retained-reference lifetime checks; no original dictionary dispatch.
        let key = foundation_utf16(key.as_ptr()?)?;
        let value = foundation_utf16(value.as_ptr()?)?;
        insert_dictionary_string(&mut result, &key, &value)?;
    }
    Ok(result)
}
pub(super) fn bind_foundation_dictionary_initializer(
    receiver: *mut AnyObject,
    prepare: impl FnOnce() -> Result<Vec<(FoundationReference, FoundationReference)>>,
) -> Result<*mut AnyObject> {
    bind(
        receiver,
        Role::Foundation(FoundationRole::Dictionary),
        || {
            let values = prepare()?;
            if let Some((_, first)) = values.first() {
                if matches!(
                    foundation_value(first.as_ptr()?)?,
                    FoundationValue::String(_)
                ) {
                    decode_string_dictionary(&values)?;
                    return Ok(Payload::StringDictionary(values));
                }
            }
            validate_dictionary(&values)?;
            Ok(Payload::Dictionary(values))
        },
    )
}
/// A String projection is separate from Boolean-only HTML options. An empty
/// generic NSDictionary has no conflicting value type and projects as empty.
pub(super) fn foundation_string_dictionary(
    pointer: *mut AnyObject,
) -> Result<BTreeMap<String, String>> {
    let identity = identity_at(pointer)?;
    let values = match payload(pointer)? {
        Payload::StringDictionary(values) => values,
        Payload::Dictionary(values) if values.is_empty() => values,
        _ => return Err("not a qualified String dictionary".into()),
    };
    let result = decode_string_dictionary(&values)?;
    pure_check(pointer, &identity)?;
    Ok(result)
}
fn string_dictionary_reference(value: &str) -> Result<FoundationReference> {
    let pointer = new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(
        value.as_bytes(),
    )?)))?;
    // Consume the exact fresh child +1 through the existing qualified guard.
    unsafe { foundation_created_reference(pointer) }
}
/// Returns one actual common factory +1, to be consumed by the caller.
pub(super) fn new_foundation_string_dictionary(
    values: &BTreeMap<String, String>,
) -> Result<*mut AnyObject> {
    context()?;
    let mut references = Vec::with_capacity(values.len());
    for (key, value) in values {
        references.push((
            string_dictionary_reference(key)?,
            string_dictionary_reference(value)?,
        ));
    }
    decode_string_dictionary(&references)?;
    create(
        Role::Foundation(FoundationRole::Dictionary),
        Payload::StringDictionary(references),
    )
}
fn validate_error_dictionary(values: &[(FoundationReference, FoundationReference)]) -> Result<()> {
    let mut keys = std::collections::BTreeSet::new();
    for (key, value) in values {
        let key = foundation_utf16(key.as_ptr()?)?;
        if !keys.insert(key) {
            return Err("duplicate NSError userInfo key".into());
        }
        let pointer = value.as_ptr()?;
        match kind(pointer)? {
            Kind::Data(_) => {
                foundation_utf16(pointer)?;
            }
            Kind::TaskPayload(_) if qualified_arena_reference_role(pointer)? == c"NSError" => {
                super::task_error_fields(pointer)?;
            }
            _ => return Err("NSError userInfo value is not qualified String or NSError".into()),
        }
    }
    Ok(())
}
/// Exact typed userInfo route. The environment and Boolean-options projections
/// never accept this payload, even when its actual dictionary class is shared.
pub(super) fn new_foundation_error_dictionary(
    values: Vec<(FoundationReference, FoundationReference)>,
) -> Result<*mut AnyObject> {
    validate_error_dictionary(&values)?;
    create(
        Role::Foundation(FoundationRole::Dictionary),
        Payload::ErrorDictionary(values),
    )
}
/// Shared selected NSDictionary methods may expose only validated typed rows.
pub(super) fn foundation_dictionary_entries(
    pointer: *mut AnyObject,
) -> Result<Vec<(FoundationReference, FoundationReference)>> {
    let identity = identity_at(pointer)?;
    let values = match payload(pointer)? {
        Payload::Dictionary(values) => {
            validate_dictionary(&values)?;
            values
        }
        Payload::StringDictionary(values) => {
            decode_string_dictionary(&values)?;
            values
        }
        Payload::ErrorDictionary(values) => {
            validate_error_dictionary(&values)?;
            values
        }
        _ => return Err("not a qualified dictionary".into()),
    };
    pure_check(pointer, &identity)?;
    Ok(values)
}
const FILE_ONLY_KEY: &str = "NSPasteboardURLReadingFileURLsOnlyKey";
pub(super) fn foundation_bool_for_key(
    pointer: *mut AnyObject,
    key: &[u16],
) -> Result<Option<bool>> {
    if key != FILE_ONLY_KEY.encode_utf16().collect::<Vec<_>>() {
        return Err("unqualified clipboard option key".into());
    }
    let Payload::Dictionary(values) = payload(pointer)? else {
        return Err("not a qualified options dictionary".into());
    };
    let mut result = None;
    for (candidate, value) in values {
        if foundation_utf16(candidate.as_ptr()?)? != key || result.is_some() {
            return Err("unknown or duplicate clipboard option".into());
        }
        let FoundationValue::Bool(value) = foundation_value(value.as_ptr()?)? else {
            return Err("clipboard option is not Boolean".into());
        };
        result = Some(value);
    }
    Ok(result)
}
pub(super) fn foundation_url_reading_file_only_key() -> Result<*mut AnyObject> {
    clipboard_owner()?;
    let installed = context()?;
    constant_owner(&installed.typed_data.file_key_owner, || {
        new_foundation_value(FoundationValue::String(Arc::new(Text::new(
            FILE_ONLY_KEY.encode_utf16().collect(),
        ))))
    })?
    .as_ptr()
}
fn constant_owner(
    slot: &Mutex<Option<FoundationReference>>,
    create: impl FnOnce() -> Result<*mut AnyObject>,
) -> Result<FoundationReference> {
    clipboard_owner()?;
    let installed = context()?;
    let current = slot.lock().map_err(|_| "constant owner poisoned")?.clone();
    if let Some(current) = current {
        current.as_ptr()?;
        return Ok(current);
    }
    let _initializing = Flag::take(&installed.typed_data.initializing_owner)?;
    let pointer = create()?;
    // SAFETY: create returns this call's fresh known common factory +1.
    let reference = unsafe { foundation_created_reference(pointer) }?;
    reference.as_ptr()?;
    pure_model(&installed.model)?;
    {
        let mut owner = slot.lock().map_err(|_| "constant owner poisoned")?;
        if owner.is_some() {
            return Err("reentrant constant owner publication".into());
        }
        *owner = Some(reference.clone());
    }
    reference.as_ptr()?;
    Ok(reference)
}
pub(super) fn foundation_is_url_class(pointer: *mut AnyObject) -> Result<bool> {
    let installed = context()?;
    let _checking = ThreadFlag::take(&CHECKING)?;
    let result =
        super::super::factory::class_role_matches(pointer.cast(), c"NSURL", &installed.model)?;
    pure_model(&installed.model)?;
    Ok(result)
}
fn autorelease(
    pointer: *mut AnyObject,
    wanted: impl FnOnce(Role) -> bool,
) -> Result<*mut AnyObject> {
    // SAFETY: private helper only receives a fresh +1 from our common factory.
    let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?;
    let identity = identity_at(pointer)?;
    if !wanted(identity.0.role) {
        return Err("wrong typed autorelease role".into());
    }
    let result = unsafe { super::super::factory::autorelease_owned(pointer) }?;
    pending.into_raw();
    Ok(result)
}
/// # Safety
/// Consumes exactly one actual caller-owned common factory +1, not a borrow.
pub(super) unsafe fn autorelease_foundation(pointer: *mut AnyObject) -> Result<*mut AnyObject> {
    autorelease(pointer, |role| matches!(role, Role::Foundation(_)))
}
/// # Safety
/// Consumes exactly one actual caller-owned common factory +1, not a borrow.
pub(super) unsafe fn autorelease_clipboard(pointer: *mut AnyObject) -> Result<*mut AnyObject> {
    autorelease(pointer, |role| matches!(role, Role::Clipboard(_)))
}
fn clipboard_context() -> Result<Arc<clipboard::Clipboard>> {
    clipboard_owner()?;
    let installed = context()?;
    let existing = installed
        .typed_data
        .clipboard
        .lock()
        .map_err(|_| "clipboard context poisoned")?
        .clone();
    if let Some(existing) = existing {
        return Ok(existing);
    }
    let _initializing = Flag::take(&installed.typed_data.initializing_clipboard)?;
    let generation = pure_model(&installed.model)?;
    let life = Arc::new(ClipboardLife {
        model: installed.model.clone(),
        generation,
        owner: installed.typed_data.owner,
    });
    let clipboard = Arc::new(clipboard::Clipboard::new(
        &format!("workspace-{generation}"),
        life,
    )?);
    context()?;
    let mut slot = installed
        .typed_data
        .clipboard
        .lock()
        .map_err(|_| "clipboard context poisoned")?;
    if slot.is_some() {
        return Err("reentrant clipboard context publication".into());
    }
    *slot = Some(clipboard.clone());
    Ok(clipboard)
}
/// Guardian liveness is necessary but does not itself preserve this workspace's
/// generation. The existing clipboard model receives the conjunction, not a
/// second permission system or an alternative receiver arena.
struct ClipboardLife {
    model: Arc<Workspace>,
    generation: u64,
    owner: std::thread::ThreadId,
}
impl ClipboardLife {
    fn check(&self) -> Result<()> {
        if self.owner != std::thread::current().id() || pure_model(&self.model)? != self.generation
        {
            return Err("clipboard workspace generation or owner expired".into());
        }
        Ok(())
    }
}
impl crate::sdk::owned::AliveLease for ClipboardLife {
    fn before_effect(&self) -> Result<()> {
        self.check()?;
        let _checking = ThreadFlag::take(&CHECKING)?;
        match catch_unwind(AssertUnwindSafe(|| self.model.context.life.before_effect())) {
            Ok(Ok(())) => self.check(),
            Ok(Err(error)) => {
                self.model.fault();
                Err(error)
            }
            Err(payload) => {
                self.model.fault();
                std::mem::forget(payload);
                Err("clipboard guardian panicked".into())
            }
        }
    }
    fn is_alive(&self) -> bool {
        if self.check().is_err() {
            return false;
        }
        let Ok(_checking) = ThreadFlag::take(&CHECKING) else {
            return false;
        };
        match catch_unwind(AssertUnwindSafe(|| self.model.context.life.is_alive())) {
            Ok(true) => self.check().is_ok(),
            outcome => {
                self.model.fault();
                if let Err(payload) = outcome {
                    std::mem::forget(payload);
                }
                false
            }
        }
    }
}
fn board_pointer(board: Arc<clipboard::Board>) -> Result<*mut AnyObject> {
    let installed = context()?;
    let candidates = {
        let objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        objects
            .iter()
            .filter_map(|(&pointer, kind)| match kind {
                Kind::Data(identity)
                    if identity.0.role == Role::Clipboard(ClipboardRole::Board)
                        && identity.0.live.load(Ordering::Acquire) =>
                {
                    Some((pointer, identity.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // All payload checks, guardian callbacks and retained-reference operations
    // are outside the sole arena lock. Poison is a hard failure, never a miss.
    for (pointer, identity) in candidates {
        check(pointer as *mut AnyObject, &identity)?;
        let Payload::Board(current) = payload(pointer as *mut AnyObject)? else {
            return Err("board record has mismatched payload".into());
        };
        if Arc::ptr_eq(&current, &board) {
            return unsafe { super::super::factory::retain_owned(pointer as *mut AnyObject) };
        }
    }
    create(Role::Clipboard(ClipboardRole::Board), Payload::Board(board))
}
pub(super) fn clipboard_general() -> Result<*mut AnyObject> {
    clipboard_owner()?;
    let installed = context()?;
    let owner = constant_owner(&installed.typed_data.general_owner, || {
        board_pointer(clipboard_context()?.general()?)
    })?;
    // The scoped singleton keeps its own +1; each factory caller gets another.
    unsafe { super::super::factory::retain_owned(owner.as_ptr()?) }
}
pub(super) fn clipboard_unique() -> Result<*mut AnyObject> {
    clipboard_owner()?;
    board_pointer(clipboard_context()?.unique()?)
}
pub(super) fn clipboard_named(name: &[u16]) -> Result<*mut AnyObject> {
    clipboard_owner()?;
    let installed = context()?;
    let candidates = {
        let objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        objects
            .iter()
            .filter_map(|(&pointer, kind)| match kind {
                Kind::Data(identity)
                    if identity.0.role == Role::Clipboard(ClipboardRole::Board)
                        && identity.0.live.load(Ordering::Acquire) =>
                {
                    Some((pointer, identity.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for (pointer, identity) in candidates {
        check(pointer as *mut AnyObject, &identity)?;
        if let Payload::Board(board) = payload(pointer as *mut AnyObject)? {
            if board.name()? == name {
                return unsafe { super::super::factory::retain_owned(pointer as *mut AnyObject) };
            }
        }
    }
    board_pointer(clipboard_context()?.new_named(name.to_vec())?)
}
pub(super) fn clipboard_board(pointer: *mut AnyObject) -> Result<Arc<clipboard::Board>> {
    clipboard_owner()?;
    match payload(pointer)? {
        Payload::Board(board) => Ok(board),
        _ => Err("not a qualified board".into()),
    }
}
pub(super) fn clipboard_item(pointer: *mut AnyObject) -> Result<Arc<clipboard::Item>> {
    clipboard_owner()?;
    match payload(pointer)? {
        Payload::Item(item) => Ok(item),
        _ => Err("not a qualified clipboard item".into()),
    }
}
pub(super) fn new_clipboard_item(item: Arc<clipboard::Item>) -> Result<*mut AnyObject> {
    clipboard_owner()?;
    if !clipboard_context()?.owns_item(&item) {
        return Err("foreign clipboard item model".into());
    }
    item.snapshot()?;
    create(Role::Clipboard(ClipboardRole::Item), Payload::Item(item))
}
pub(super) fn bind_clipboard_item_initializer(pointer: *mut AnyObject) -> Result<*mut AnyObject> {
    clipboard_owner()?;
    bind(pointer, Role::Clipboard(ClipboardRole::Item), || {
        Ok(Payload::Item(clipboard_context()?.item()?))
    })
}
pub(super) fn clipboard_write_object(pointer: *mut AnyObject) -> Result<clipboard::WriteObject> {
    clipboard_owner()?;
    match payload(pointer)? {
        Payload::Item(item) => Ok(clipboard::WriteObject::Item(item)),
        Payload::Value(FoundationValue::Url(url)) => Ok(clipboard::WriteObject::FileUrl(
            url.string().utf16().to_vec(),
        )),
        _ => Err("unsupported qualified clipboard write object".into()),
    }
}
pub(super) fn clipboard_release(pointer: *mut AnyObject) -> Result<()> {
    clipboard_owner()?;
    let identity = identity_at(pointer)?;
    clipboard_board(pointer)?.release()?;
    // This business retirement removes the name-cache candidate. Physical ARC
    // cleanup remains independently qualified after releaseGlobally.
    identity.0.live.store(false, Ordering::Release);
    Ok(())
}
pub(super) fn foundation_path_is_directory(path: &[u16]) -> Result<bool> {
    let installed = context()?;
    let path = std::path::PathBuf::from(
        String::from_utf16(path).map_err(|_| "invalid owned path Unicode")?,
    );
    if !path.is_absolute() {
        return Err("relative data path refused".into());
    }
    let apps = installed.model.applications()?;
    if path == std::path::Path::new("/System/Applications/Utilities/Activity Monitor.app") {
        let matches = apps
            .iter()
            .filter(|(_, app)| {
                app.bundle.as_deref() == Some("com.apple.ActivityMonitor")
                    && app.bundle_path.is_some()
            })
            .count();
        if matches != 1 {
            return Err("Activity Monitor semantic fixture is missing or ambiguous".into());
        }
        pure_model(&installed.model)?;
        return Ok(true);
    }
    let mut known = None;
    for (_, app) in apps {
        for (candidate, directory) in [(app.bundle_path, true), (app.executable_path, false)] {
            if candidate.as_ref() == Some(&path) {
                if known.is_some_and(|old| old != directory) {
                    return Err("ambiguous declared path kind".into());
                }
                known = Some(directory);
            }
        }
    }
    if let Some(directory) = known {
        pure_model(&installed.model)?;
        return Ok(directory);
    }
    for root in [
        &installed.model.context.home,
        &installed.model.context.cache,
        &installed.model.context.temp,
        &installed.model.context.applications,
    ] {
        if path.starts_with(root.path()) {
            let (directory, _) = root.info(&path)?;
            pure_model(&installed.model)?;
            return Ok(directory);
        }
    }
    Err("path is outside declared data capabilities".into())
}

#[cfg(test)]
mod string_dictionary_tests {
    use super::*;

    #[test]
    fn string_dictionary_utf16_preserves_unicode_and_refuses_invalid_rows() {
        let mut result = BTreeMap::new();
        insert_dictionary_string(
            &mut result,
            &"CAFÉ_🌿".encode_utf16().collect::<Vec<_>>(),
            &"line1\nvalue=two\0".encode_utf16().collect::<Vec<_>>(),
        )
        .unwrap();
        insert_dictionary_string(&mut result, &[], &[]).unwrap();
        assert_eq!(result.get("CAFÉ_🌿").unwrap(), "line1\nvalue=two\0");
        assert_eq!(result.get("").unwrap(), "");
        let before = result.clone();
        assert!(insert_dictionary_string(&mut result, &[0xD800], &[b'x' as u16]).is_err());
        assert!(insert_dictionary_string(&mut result, &[b'x' as u16], &[0xDC00]).is_err());
        assert_eq!(result, before);
    }

    #[test]
    fn string_dictionary_duplicate_refusal_preserves_the_first_value() {
        let mut result = BTreeMap::new();
        insert_dictionary_string(&mut result, &[b'A' as u16], &[b'1' as u16]).unwrap();
        assert!(insert_dictionary_string(&mut result, &[b'A' as u16], &[b'2' as u16]).is_err());
        assert_eq!(result.get("A").unwrap(), "1");
    }
}
