//! Test-tools-only constructor handoff. A borrowed C frame is transport, never
//! authority. Only the independently compile-pinned private pipe can grant the
//! exact source/image/path set; ordinary startup must not consult this module.
//! The dedicated owned Host fixtures use explicit UTF-8 paths. This JSON
//! transport makes no claim about non-UTF-8 production profile paths.
use crate::{profile::ProfilePaths, profile_io_abi as abi};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    mem::{align_of, size_of},
    path::{Component, Path},
    sync::{Arc, Mutex, TryLockError, atomic::{AtomicU64, Ordering}},
};
type Result<T> = std::result::Result<T, String>;
/// Fixed Input5 transport in the actual Native executable. Source, ABI,
/// image/body and userdata-lifetime admission remain the independent issuer's job.
#[repr(C)]
pub struct NativeInputCallbacksV1 {
    pub magic: u64,
    pub abi: u32,
    pub count: u32,
    pub carbon_pressed: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub hid_input: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void),
    pub hid_removed: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void),
    pub hid_filter: unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
    pub keyword_filter: unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
}
const _: [(); 56] = [(); size_of::<NativeInputCallbacksV1>()];
const _: [(); 8] = [(); align_of::<NativeInputCallbacksV1>()];
const ROLE: &str = "full-owned-host-v1";
const READY: u64 = 0xF100002600003FFF;
const MAX_GRANT: usize = 16_384;
const MAX_FILE: usize = 128 * 1024 * 1024;
const FUNCTIONS: [&str; 6] = [
    "bind",
    "request",
    "release_handle",
    "return_buffer",
    "getter",
    "ready",
];
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    device: u64,
    inode: u64,
    kind: u32,
    access: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    fd: i32,
    fingerprint: Fingerprint,
    nonce_sha256: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    path: String,
    sha256: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Function {
    name: String,
    image_offset: usize,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    role: String,
    nonce: String,
    owner_uid: u32,
    endpoint: Input,
    sdk: Input,
    source_manifest: Input,
    native_callback_roster: Input,
    source_set_sha256: String,
    fixture: Input,
    fixture_sha256: String,
    authority_sha256: String,
    api_header: Input,
    api_offset: usize,
    initializer_offset: usize,
    function_roster: Vec<Function>,
    profile: String,
    home: String,
    config_override: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sources {
    sources: Vec<Input>,
}
enum Phase {
    Empty(Option<Box<Grant>>),
    Bound(Box<QualifiedOwnedHost>),
    Consumed(Arc<NativeInputPublisher>),
}
/// Affine admission to the owned production host. Only a fully qualified
/// constructor binding can mint this value; caller-supplied paths cannot.
/// This proves host admission, not readiness of individual F1 measurements.
pub struct QualifiedOwnedHost {
    paths: ProfilePaths,
    fixture: Input,
    owner_uid: u32,
    fixture_sha256: String,
    authority_sha256: String,
    sdk_sha256: String,
    input_publisher: Arc<NativeInputPublisher>,
}
struct NativeInputPublisher {
    api: &'static abi::Api,
    capability: abi::Handle,
    owner: std::thread::ThreadId,
    next_epoch: AtomicU64,
}
#[repr(C)]
struct InputOwnerNotice {
    version: u32, bytes: u32, role: u32, ticket_kind: u32,
    userdata: usize, owner_epoch: u64, ticket_slot: u64, ticket_generation: u64,
}
const _: [(); 48] = [(); size_of::<InputOwnerNotice>()];
/// Native-local metadata only. The Box/Rc owner must explicitly revoke before
/// freeing userdata, and retain it in the existing quarantine on any error.
pub(crate) struct NativeInputSourceTicket {
    publisher: Arc<NativeInputPublisher>,
    role: u32,
    userdata: usize,
    epoch: u64,
    key: [u64; 2],
    revoked: bool,
}
impl NativeInputPublisher {
    fn request(&self, notice: &InputOwnerNotice, revoke: bool) -> Result<[u64; 2]> {
        if self.owner != std::thread::current().id() { return Err("Native Input publisher requires its actual owner thread.".into()); }
        let bytes = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(notice).cast::<u8>(), size_of::<InputOwnerNotice>()) };
        let reply = unsafe { (self.api.request)(self.capability, abi::Request {
            operation: if revoke { 0x1002 } else { 0x1001 }, flags: 0,
            path: abi::Bytes { pointer: std::ptr::null(), length: 0 }, data: abi::Bytes::borrowed(bytes), limit: 0,
            expected_device: 0, expected_inode: 0,
        }) };
        let result = (|| {
            if reply.status != abi::OK || reply.reserved != 0 || reply.handle != abi::Handle::default()
                || reply.metadata.reserved != 0 {
                return Err("Native Input owner request refused.".into());
            }
            if revoke {
                if reply.buffer.token != 0 || !reply.buffer.pointer.is_null() || reply.buffer.length != 0 {
                    return Err("Native Input revoke returned unexpected data.".into());
                }
                return Ok([notice.ticket_slot, notice.ticket_generation]);
            }
            if reply.buffer.token == 0 || reply.buffer.pointer.is_null() || reply.buffer.length != 16 {
                return Err("Native Input owner key storage differs.".into());
            }
            // As in the existing profile IO client, the qualified SDK owns
            // readable immutable borrowed bytes until return_buffer. Its Vec
            // storage is heap memory, not a readonly image/code declaration.
            let key = unsafe { reply.buffer.pointer.cast::<[u64; 2]>().read_unaligned() };
            if key[0] != u64::from(notice.role) || key[1] == 0 { return Err("Native Input owner key differs.".into()); }
            Ok(key)
        })();
        if reply.buffer.token != 0 && (self.api.return_buffer)(self.capability, reply.buffer.token) != abi::OK {
            return Err("Native Input owner reply cleanup refused.".into());
        }
        result
    }
}
impl NativeInputSourceTicket {
    fn publish(role: u32, userdata: usize) -> Result<Option<Self>> {
        if option_env!("RUYCAST_F1_NATIVE_GRANT_PIN").is_none() { return Ok(None); }
        let publisher = {
            let state = STATE.try_lock().map_err(|_| "Native Input publisher binding unavailable.")?;
            let Phase::Consumed(publisher) = &*state else { return Err("Native Input requires the consumed genuine Host binding.".into()); };
            publisher.clone()
        };
        Self::publish_from(publisher, role, userdata).map(Some)
    }
    fn publish_from(publisher: Arc<NativeInputPublisher>, role: u32, userdata: usize) -> Result<Self> {
        let epoch = publisher.next_epoch.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "Native Input owner epoch exhausted.")?;
        if epoch == 0 || userdata == 0 { return Err("Native Input owner identity is empty.".into()); }
        let notice = InputOwnerNotice { version: 1, bytes: 48, role, ticket_kind: role, userdata,
            owner_epoch: epoch, ticket_slot: 0, ticket_generation: 0 };
        let key = publisher.request(&notice, false)?;
        Ok(Self { publisher, role, userdata, epoch, key, revoked: false })
    }
    #[cfg(test)]
    pub(crate) fn fixture_carbon_owner(owner: Box<crate::carbon_hotkeys::Callback>, api: &'static abi::Api)
        -> (Box<crate::carbon_hotkeys::Callback>, Result<Option<Self>>) {
        // Typed actual Box custody; this owned mock transport is test-only and
        // is never a Host/Source grant or a production publisher constructor.
        let publisher = Arc::new(NativeInputPublisher { api, capability: abi::Handle { slot: 17, generation: 1 },
            owner: std::thread::current().id(), next_epoch: AtomicU64::new(1) });
        let result = Self::publish_from(publisher, 1, std::ptr::from_ref(&*owner) as usize).map(Some);
        (owner, result)
    }
    pub(crate) fn for_carbon(owner: Box<crate::carbon_hotkeys::Callback>) -> (Box<crate::carbon_hotkeys::Callback>, Result<Option<Self>>) {
        let result = Self::publish(1, std::ptr::from_ref(&*owner) as usize);
        (owner, result)
    }
    pub(crate) fn for_hid(owner: std::rc::Rc<crate::hid_service::Callback>) -> (std::rc::Rc<crate::hid_service::Callback>, Result<Option<Self>>) {
        let result = Self::publish(2, std::rc::Rc::as_ptr(&owner) as usize);
        (owner, result)
    }
    pub(crate) fn for_keyword(owner: std::rc::Rc<crate::keyword_capture::Callback>) -> (std::rc::Rc<crate::keyword_capture::Callback>, Result<Option<Self>>) {
        let result = Self::publish(3, std::rc::Rc::as_ptr(&owner) as usize);
        (owner, result)
    }
    pub(crate) fn revoke(&mut self) -> Result<()> {
        if self.revoked { return Ok(()); }
        self.publisher.request(&InputOwnerNotice { version: 1, bytes: 48, role: self.role, ticket_kind: self.role,
            userdata: self.userdata, owner_epoch: self.epoch, ticket_slot: self.key[0], ticket_generation: self.key[1] }, true)?;
        self.revoked = true;
        Ok(())
    }
}
impl QualifiedOwnedHost {
    /// Borrow already-qualified paths while the affine admission is retained.
    pub fn paths(&self) -> &ProfilePaths {
        &self.paths
    }
    /// Immutable comparison bindings from the independent private grant.
    /// These facts do not by themselves admit any endpoint or measurement.
    pub fn ready_facts(&self) -> (&str, &str, &str) {
        (
            &self.fixture_sha256,
            &self.authority_sha256,
            &self.sdk_sha256,
        )
    }
    /// Read the epoch only from the independent grant's still-exact fixture
    /// manifest. A transported timestamp or ambient environment is not custody.
    pub fn fixture_epoch_unix_seconds(&self) -> Result<u64> {
        fixture_epoch(&self.fixture, &self.fixture_sha256, self.owner_uid, &System::load()?)
    }
    /// Consume the admission when constructing the production service graph.
    pub fn into_paths(self) -> ProfilePaths {
        self.paths
    }
}
static STATE: Mutex<Phase> = Mutex::new(Phase::Empty(None));
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fixture_epoch(input: &Input, expected: &str, owner_uid: u32, sys: &System) -> Result<u64> {
    if input.sha256 != expected || !hash_valid(expected) {
        return Err("Owned Host fixture comparison binding differs.".into());
    }
    #[derive(Deserialize)]
    struct Epoch { schema: String, epoch_unix_seconds: u64 }
    let manifest: Epoch = serde_json::from_slice(&sys.input(input, 2 * 1024 * 1024, owner_uid)?)
        .map_err(|_| "Owned Host fixture manifest schema differs.")?;
    if manifest.schema != "ruycast.f1.fixture.v1" {
        return Err("Owned Host fixture manifest schema differs.".into());
    }
    Ok(manifest.epoch_unix_seconds)
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn path(value: &str) -> Result<&Path> {
    let p = Path::new(value);
    if value.len() > 32_768
        || value.contains('\0')
        || !p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err("Invalid pinned owned Host path.".into());
    }
    Ok(p)
}
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
    fn dladdr(address: *const c_void, info: *mut DlInfo) -> i32;
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(index: u32) -> *const u8;
    fn _dyld_get_image_name(index: u32) -> *const c_char;
    fn _NSGetMachExecuteHeader() -> *const u8;
    static mach_task_self_: u32;
    fn mach_vm_region_recurse(
        task: u32,
        address: *mut u64,
        size: *mut u64,
        depth: *mut u32,
        info: *mut i32,
        count: *mut u32,
    ) -> i32;
}
#[repr(C)]
struct DlInfo {
    name: *const c_char,
    base: *const c_void,
    symbol: *const c_char,
    symbol_address: *const c_void,
}
// vm_region_submap_short_info_64, SDK mach/vm_region.h (pack4, count12).
#[repr(C, packed(4))]
struct Region {
    protection: i32,
    max_protection: i32,
    inheritance: i32,
    offset: u64,
    user_tag: u32,
    ref_count: u32,
    shadow_depth: u16,
    external_pager: u8,
    share_mode: u8,
    is_submap: u32,
    behavior: i32,
    object_id: u32,
    user_wired_count: u16,
    flags: u16,
}

struct Image {
    base: usize,
    kind: u32,
    segments: Vec<(usize, usize, u32)>,
    initializer_sections: Vec<(usize, usize)>,
    alternative_initializers: bool,
}
impl Image {
    // Only dyld-owned actual loaded headers reach this unsafe reader. The
    // bounded byte parser is shared with synthetic data-only regression tests.
    unsafe fn loaded(base: *const u8) -> Result<Self> {
        if base.is_null() {
            return Err("Loaded Host image missing.".into());
        }
        let header = unsafe { std::slice::from_raw_parts(base, 32) };
        let length = 32usize
            .checked_add(u32::from_le_bytes(
                header[20..24]
                    .try_into()
                    .map_err(|_| "Host command size.")?,
            ) as usize)
            .ok_or("Host command size overflow.")?;
        if length > 16 * 1024 * 1024 {
            return Err("Host load command bounds.".into());
        }
        Self::commands(base as usize, unsafe {
            std::slice::from_raw_parts(base, length)
        })
    }
    fn commands(base: usize, bytes: &[u8]) -> Result<Self> {
        let u32at = |at: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                bytes
                    .get(at..at.checked_add(4).ok_or("Host offset overflow.")?)
                    .ok_or("Truncated Host command.")?
                    .try_into()
                    .map_err(|_| "Host word.")?,
            ))
        };
        let u64at = |at: usize| -> Result<usize> {
            usize::try_from(u64::from_le_bytes(
                bytes
                    .get(at..at.checked_add(8).ok_or("Host offset overflow.")?)
                    .ok_or("Truncated Host command.")?
                    .try_into()
                    .map_err(|_| "Host word.")?,
            ))
            .map_err(|_| "Host address overflow.".into())
        };
        if bytes.len() < 32 || u32at(0)? != 0xfeedfacf {
            return Err("Host image is not Mach-O64.".into());
        }
        let count = u32at(16)? as usize;
        let end = 32usize
            .checked_add(u32at(20)? as usize)
            .ok_or("Host command overflow.")?;
        if count > 65_536 || end > 16 * 1024 * 1024 || end != bytes.len() {
            return Err("Host load command bounds.".into());
        }
        let mut at = 32usize;
        let mut text = None;
        let mut segments = vec![];
        let mut initializers = vec![];
        let mut alternative_initializers = false;
        for _ in 0..count {
            if at.checked_add(8).is_none_or(|n| n > end) {
                return Err("Truncated Host load command.".into());
            }
            let command = u32at(at)?;
            let size = u32at(at + 4)? as usize;
            let next = at.checked_add(size).ok_or("Host command overflow.")?;
            if size < 8 || !size.is_multiple_of(8) || next > end {
                return Err("Host load command bounds.".into());
            }
            if matches!(command, 0x11 | 0x1a) {
                alternative_initializers = true; // LC_ROUTINES / LC_ROUTINES_64
            }
            if command == 0x19 {
                if size < 72 {
                    return Err("Short Host segment.".into());
                }
                let sections = u32at(at + 64)? as usize;
                if sections > 65_536
                    || sections.checked_mul(80).and_then(|n| n.checked_add(72)) != Some(size)
                {
                    return Err("Host section roster bounds.".into());
                }
                let vm = u64at(at + 24)?;
                let length = u64at(at + 32)?;
                let vm_end = vm.checked_add(length).ok_or("Host segment overflow.")?;
                if &bytes[at + 8..at + 24] == b"__TEXT\0\0\0\0\0\0\0\0\0\0"
                    && text.replace(vm).is_some()
                {
                    return Err("Duplicate Host text segment.".into());
                }
                segments.push((vm, length, u32at(at + 60)?));
                for index in 0..sections {
                    let section = at + 72 + index * 80;
                    let name = &bytes[section..section + 16];
                    let address = u64at(section + 32)?;
                    let length = u64at(section + 40)?;
                    let section_end = address
                        .checked_add(length)
                        .ok_or("Host section overflow.")?;
                    if address < vm || section_end > vm_end {
                        return Err("Host section outside containing segment.".into());
                    }
                    let kind = u32at(section + 64)? & 0xff;
                    let mod_init = name == b"__mod_init_func\0";
                    if mod_init && kind != 0x9 {
                        alternative_initializers = true;
                    }
                    if kind == 0x9 && length != 0 {
                        initializers.push((address, length));
                    }
                    if length != 0
                        && (matches!(kind, 0x15 | 0x16)
                            || name == b"__objc_nlclslist"
                            || name == b"__objc_nlcatlist")
                    {
                        alternative_initializers = true;
                    }
                }
            }
            at = next;
        }
        if at != end {
            return Err("Host load command inventory differs.".into());
        }
        let text = text.ok_or("Host text segment missing.")?;
        let slide = |vm: usize| -> Result<usize> {
            if vm >= text {
                base.checked_add(vm - text)
            } else {
                base.checked_sub(text - vm)
            }
            .ok_or_else(|| "Host segment slide overflow.".into())
        };
        let mut mapped_segments = vec![];
        for (vm, length, protection) in segments {
            let start = slide(vm)?;
            mapped_segments.push((
                start,
                start
                    .checked_add(length)
                    .ok_or("Host segment end overflow.")?,
                protection,
            ));
        }
        let initializer_sections = initializers
            .into_iter()
            .map(|(vm, length)| {
                let start = slide(vm)?;
                start
                    .checked_add(length)
                    .ok_or("Host initializer range overflow.")?;
                Ok((start, length))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            base,
            kind: u32at(12)?,
            segments: mapped_segments,
            initializer_sections,
            alternative_initializers,
        })
    }
    fn initializer_roster(
        &self,
        expected: Option<usize>,
        read: impl FnOnce(usize) -> Result<usize>,
    ) -> Result<()> {
        if self.alternative_initializers {
            return Err("Owned Host alternate initializer route is forbidden.".into());
        }
        let Some(offset) = expected else {
            return if self.initializer_sections.is_empty() {
                Ok(())
            } else {
                Err("Native Host must have no initializers.".into())
            };
        };
        let [(entry, length)] = self.initializer_sections.as_slice() else {
            return Err("SDK must have exactly one initializer entry.".into());
        };
        if *length != size_of::<usize>()
            || !entry.is_multiple_of(align_of::<usize>())
            || !self.contains(*entry, *length, false)
        {
            return Err("SDK initializer entry storage differs.".into());
        }
        let target = self
            .base
            .checked_add(offset)
            .ok_or("SDK initializer offset overflow.")?;
        if !self.contains(target, 1, true) || read(*entry)? != target {
            return Err("SDK initializer differs from private pinned code.".into());
        }
        Ok(())
    }
    fn contains(&self, address: usize, bytes: usize, executable: bool) -> bool {
        let Some(end) = address.checked_add(bytes) else {
            return false;
        };
        let matching = self
            .segments
            .iter()
            .filter(|(a, b, p)| {
                address >= *a && end <= *b && p & 1 != 0 && (p & 4 != 0) == executable
            })
            .count();
        matching == 1
    }
    fn code(&self, offset: usize) -> Result<usize> {
        let address = self
            .base
            .checked_add(offset)
            .ok_or("Host code offset overflow.")?;
        if !self.contains(address, 1, true) {
            return Err("Host function outside pinned executable image.".into());
        }
        mapped(address, 1, true)?;
        Ok(address)
    }
}
fn loaded_image(name: &str) -> Result<Image> {
    let mut found = None;
    let count = unsafe { _dyld_image_count() };
    if count > 65_536 {
        return Err("Loaded image inventory bound.".into());
    }
    for index in 0..count {
        let p = unsafe { _dyld_get_image_name(index) };
        if !p.is_null() && unsafe { CStr::from_ptr(p) }.to_bytes() == name.as_bytes() {
            if found.is_some() {
                return Err("Duplicate pinned Host image.".into());
            }
            found = Some(unsafe { Image::loaded(_dyld_get_image_header(index)) }?);
        }
    }
    found.ok_or_else(|| "Pinned Host image not loaded.".into())
}
fn mapped(address: usize, bytes: usize, executable: bool) -> Result<()> {
    let end = address
        .checked_add(bytes)
        .ok_or("Host mapping size overflow.")? as u64;
    let mut depth = 0u32;
    for _ in 0..16 {
        let mut start = address as u64;
        let mut size = 0;
        let mut info = std::mem::MaybeUninit::<Region>::zeroed();
        let mut count = (size_of::<Region>() / 4) as u32;
        let status = unsafe {
            mach_vm_region_recurse(
                mach_task_self_,
                &mut start,
                &mut size,
                &mut depth,
                info.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if status != 0
            || count as usize != size_of::<Region>() / 4
            || start > address as u64
            || start.checked_add(size).is_none_or(|n| n < end)
        {
            return Err("Host mapping does not cover the complete qualified range.".into());
        }
        let info = unsafe { info.assume_init() };
        if info.is_submap != 0 {
            depth = depth.checked_add(1).ok_or("Host submap depth overflow.")?;
            continue;
        }
        let protection = info.protection;
        if protection & 1 == 0 || protection & 2 != 0 || (protection & 4 != 0) != executable {
            return Err("Host leaf mapping protections differ from qualified data/code.".into());
        }
        return Ok(());
    }
    Err("Host mapping nesting bound.".into())
}
// These direct libSystem entry points deliberately bypass Native's replaceable
// filesystem imports. No provider getter is used to discover or bless them.
struct LoaderHandle(*mut c_void);
impl Drop for LoaderHandle {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.0);
        }
    }
}
fn qualify_infrastructure() -> Result<()> {
    // This finite loader/introspection set is source-rostered as unpatched.
    // Its actual runtime addresses must also remain in the system loader/kernel,
    // before any replaceable filesystem entry is resolved or invoked.
    let loader = ["/usr/lib/dyld", "/usr/lib/system/libdyld.dylib"];
    let kernel = ["/usr/lib/system/libsystem_kernel.dylib"];
    let crt = ["/usr/lib/system/libsystem_c.dylib"];
    let locations: &[(usize, &[&str])] = &[
        (dladdr as *const () as usize, &loader),
        (dlopen as *const () as usize, &loader),
        (dlsym as *const () as usize, &loader),
        (dlclose as *const () as usize, &loader),
        (_dyld_image_count as *const () as usize, &loader),
        (_dyld_get_image_header as *const () as usize, &loader),
        (_dyld_get_image_name as *const () as usize, &loader),
        (_NSGetMachExecuteHeader as *const () as usize, &crt),
        (mach_vm_region_recurse as *const () as usize, &kernel),
    ];
    for (address, owners) in locations {
        let mut info = DlInfo {
            name: std::ptr::null(),
            base: std::ptr::null(),
            symbol: std::ptr::null(),
            symbol_address: std::ptr::null(),
        };
        if unsafe { dladdr(*address as *const c_void, &mut info) } == 0
            || info.name.is_null()
            || !owners
                .iter()
                .any(|owner| unsafe { CStr::from_ptr(info.name) }.to_bytes() == owner.as_bytes())
        {
            return Err("Owned Host prequalification infrastructure is not system-owned.".into());
        }
        // dladdr supplies the actual system image header, never a frame header.
        let image = unsafe { Image::loaded(info.base.cast()) }?;
        if !image.contains(*address, 1, true) {
            return Err("Owned Host infrastructure outside system executable code.".into());
        }
    }
    for (address, _) in locations {
        mapped(*address, 1, true)?;
    }
    Ok(())
}
struct System {
    openat: unsafe extern "C" fn(i32, *const c_char, i32, ...) -> i32,
    close: unsafe extern "C" fn(i32) -> i32,
    fstat: unsafe extern "C" fn(i32, *mut libc::stat) -> i32,
    fcntl: unsafe extern "C" fn(i32, i32, ...) -> i32,
    read: unsafe extern "C" fn(i32, *mut c_void, usize) -> isize,
}
impl System {
    fn load() -> Result<Self> {
        qualify_infrastructure()?;
        let handle = unsafe { dlopen(c"/usr/lib/libSystem.B.dylib".as_ptr(), 2 | 4 | 0x10) };
        if handle.is_null() {
            return Err("Qualified system entry unavailable.".into());
        }
        let _retained = LoaderHandle(handle);
        let resolve = |name: &CStr| -> Result<*mut c_void> {
            let p = unsafe { dlsym(handle, name.as_ptr()) };
            let mut info = DlInfo {
                name: std::ptr::null(),
                base: std::ptr::null(),
                symbol: std::ptr::null(),
                symbol_address: std::ptr::null(),
            };
            if p.is_null()
                || unsafe { dladdr(p, &mut info) } == 0
                || info.name.is_null()
                || unsafe { CStr::from_ptr(info.name) }.to_bytes()
                    != b"/usr/lib/system/libsystem_kernel.dylib"
            {
                return Err("System entry did not resolve to the actual kernel image.".into());
            }
            let image = loaded_image("/usr/lib/system/libsystem_kernel.dylib")?;
            if info.base as usize != image.base || !image.contains(p as usize, 1, true) {
                return Err("System entry outside qualified kernel code.".into());
            }
            mapped(p as usize, 1, true)?;
            Ok(p)
        };
        Ok(Self {
            openat: unsafe {
                std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, *const c_char, i32, ...) -> i32,
                >(resolve(c"openat")?)
            },
            close: unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(i32) -> i32>(resolve(
                    c"close",
                )?)
            },
            fstat: unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(i32, *mut libc::stat) -> i32>(
                    resolve(if cfg!(target_arch = "x86_64") {
                        c"fstat$INODE64"
                    } else {
                        c"fstat"
                    })?,
                )
            },
            fcntl: unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(i32, i32, ...) -> i32>(
                    resolve(c"fcntl")?,
                )
            },
            read: unsafe {
                std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, *mut c_void, usize) -> isize,
                >(resolve(c"read")?)
            },
        })
    }
    fn stat(&self, fd: i32) -> Result<libc::stat> {
        let mut stat = unsafe { std::mem::zeroed() };
        if unsafe { (self.fstat)(fd, &mut stat) } != 0 {
            return Err("Owned Host descriptor unavailable.".into());
        }
        Ok(stat)
    }
    fn fingerprint(&self, fd: i32) -> Result<Fingerprint> {
        let s = self.stat(fd)?;
        let flags = unsafe { (self.fcntl)(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err("Owned Host descriptor flags unavailable.".into());
        }
        Ok(Fingerprint {
            device: s.st_dev as u64,
            inode: s.st_ino,
            kind: u32::from(s.st_mode & libc::S_IFMT),
            access: flags & libc::O_ACCMODE,
        })
    }
    fn read_all(&self, fd: i32, max: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![];
        loop {
            let mut block = [0; 8192];
            let count = unsafe { (self.read)(fd, block.as_mut_ptr().cast(), block.len()) };
            if count < 0 {
                return Err("Owned Host immutable input incomplete or unavailable.".into());
            }
            if count == 0 {
                return Ok(bytes);
            }
            if bytes.len() + count as usize > max {
                return Err("Owned Host input size bound.".into());
            }
            bytes.extend_from_slice(&block[..count as usize]);
        }
    }
    fn input(&self, input: &Input, max: usize, owner_uid: u32) -> Result<Vec<u8>> {
        path(&input.path)?;
        if !hash_valid(&input.sha256) {
            return Err("Missing owned Host input hash.".into());
        }
        let mut directory = Fd {
            sys: self,
            fd: unsafe {
                (self.openat)(
                    libc::AT_FDCWD,
                    c"/".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            },
        };
        if directory.fd < 0 {
            return Err("Owned Host input root unavailable.".into());
        }
        let components = Path::new(&input.path)
            .components()
            .filter_map(|c| {
                if let Component::Normal(s) = c {
                    Some(s)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if components.is_empty() {
            return Err("Owned Host input must be a file.".into());
        }
        for (index, component) in components.iter().enumerate() {
            let name = CString::new(component.as_encoded_bytes())
                .map_err(|_| "Owned Host input encoding.")?;
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if index + 1 < components.len() {
                    libc::O_DIRECTORY
                } else {
                    0
                };
            let next = Fd {
                sys: self,
                fd: unsafe { (self.openat)(directory.fd, name.as_ptr(), flags) },
            };
            if next.fd < 0 {
                return Err("Owned Host input component unavailable.".into());
            }
            directory = next;
        }
        let before = self.stat(directory.fd)?;
        if before.st_mode & libc::S_IFMT != libc::S_IFREG
            || before.st_nlink != 1
            || before.st_uid != owner_uid
            || before.st_mode & 0o022 != 0
            || before.st_size < 0
            || before.st_size as usize > max
        {
            return Err("Owned Host input is not a private regular bounded file.".into());
        }
        let bytes = self.read_all(directory.fd, max)?;
        let after = self.stat(directory.fd)?;
        if before.st_dev != after.st_dev
            || before.st_ino != after.st_ino
            || before.st_size != after.st_size
            || before.st_mtime != after.st_mtime
            || before.st_mtime_nsec != after.st_mtime_nsec
            || before.st_ctime != after.st_ctime
            || before.st_ctime_nsec != after.st_ctime_nsec
            || bytes.len() != before.st_size as usize
            || digest(&bytes) != input.sha256
        {
            return Err("Owned Host pinned input changed.".into());
        }
        Ok(bytes)
    }
}
struct Fd<'a> {
    sys: &'a System,
    fd: i32,
}
impl Drop for Fd<'_> {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                (self.sys.close)(self.fd);
            }
        }
    }
}
fn read_grant(pin: &Pin, sys: &System, locator: &str) -> Result<Grant> {
    if pin.fd <= 2
        || locator != pin.fd.to_string()
        || !hash_valid(&pin.nonce_sha256)
        || pin.fingerprint.kind != u32::from(libc::S_IFIFO)
        || pin.fingerprint.access != libc::O_RDONLY
        || sys.fingerprint(pin.fd)? != pin.fingerprint
    {
        return Err("Native grant is not the exact compiled readcap.".into());
    }
    let descriptor = Fd { sys, fd: pin.fd };
    let flags = unsafe { (sys.fcntl)(descriptor.fd, libc::F_GETFL) };
    if flags < 0
        || unsafe { (sys.fcntl)(descriptor.fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
    {
        return Err("Native grant readcap setup failed.".into());
    }
    let bytes = sys.read_all(descriptor.fd, MAX_GRANT + 4)?;
    if bytes.len() < 4 {
        return Err("Native grant frame incomplete.".into());
    }
    let length =
        u32::from_ne_bytes(bytes[..4].try_into().map_err(|_| "Native grant length.")?) as usize;
    if length > MAX_GRANT || bytes.len() != length + 4 {
        return Err("Native grant framing differs.".into());
    }
    let grant: Grant =
        serde_json::from_slice(&bytes[4..]).map_err(|_| "Native grant schema differs.")?;
    if grant.role != ROLE
        || digest(grant.nonce.as_bytes()) != pin.nonce_sha256
        || grant.owner_uid != sys.stat(descriptor.fd)?.st_uid
    {
        return Err("Native independent role/nonce differs.".into());
    }
    Ok(grant)
}
fn qualify(grant: &Grant, sys: &System) -> Result<(Image, Image, BTreeMap<String, usize>)> {
    if !hash_valid(&grant.fixture_sha256) || !hash_valid(&grant.authority_sha256) {
        return Err("Owned Host comparison bindings invalid.".into());
    }
    fixture_epoch(&grant.fixture, &grant.fixture_sha256, grant.owner_uid, sys)?;
    path(&grant.profile)?;
    path(&grant.home)?;
    if let Some(value) = &grant.config_override {
        if !value.is_empty() {
            path(value)?;
        }
    }
    if grant.endpoint.path == grant.sdk.path {
        return Err("Native and SDK must be separate images.".into());
    }
    sys.input(&grant.endpoint, MAX_FILE, grant.owner_uid)?;
    sys.input(&grant.sdk, MAX_FILE, grant.owner_uid)?;
    let callbacks = crate::native_callback_roster::Declaration::decode(&sys.input(
        &grant.native_callback_roster,
        crate::native_callback_roster::MAX_BYTES,
        grant.owner_uid,
    )?)?;
    if callbacks.native.path != grant.endpoint.path
        || callbacks.native.sha256 != grant.endpoint.sha256
        || callbacks.native_source_manifest.path != grant.source_manifest.path
        || callbacks.native_source_manifest.sha256 != grant.source_manifest.sha256
    {
        return Err("Owned Host callback declaration image/source bindings differ.".into());
    }
    // The independently pinned sidecar declares its complete file inputs. This
    // verifies bytes only; labels and offsets never establish code provenance.
    for binding in callbacks.bindings() {
        sys.input(
            &Input {
                path: binding.path.clone(),
                sha256: binding.sha256.clone(),
            },
            MAX_FILE,
            grant.owner_uid,
        )?;
    }
    let sources: Sources = serde_json::from_slice(&sys.input(
        &grant.source_manifest,
        2 * 1024 * 1024,
        grant.owner_uid,
    )?)
    .map_err(|_| "Owned Host source manifest schema.")?;
    if sources.sources.is_empty()
        || sources.sources.len() > 4096
        || !hash_valid(&grant.source_set_sha256)
    {
        return Err("Owned Host source roster bounds.".into());
    }
    let mut previous: Option<&str> = None;
    let mut set = Sha256::new();
    let mut header = false;
    for source in &sources.sources {
        if previous.is_some_and(|p| p >= source.path.as_str()) {
            return Err("Owned Host sources must be uniquely ordered.".into());
        }
        sys.input(source, MAX_FILE, grant.owner_uid)?;
        set.update(source.path.as_bytes());
        set.update([0]);
        set.update(source.sha256.as_bytes());
        set.update(b"\n");
        if source.path == grant.api_header.path && source.sha256 == grant.api_header.sha256 {
            header = true;
        }
        previous = Some(&source.path);
    }
    if !header || format!("{:x}", set.finalize()) != grant.source_set_sha256 {
        return Err("Owned Host source/header closure differs.".into());
    }
    let args = std::env::args_os().collect::<Vec<_>>();
    if args.len() != 2
        || args[0].as_encoded_bytes() != grant.endpoint.path.as_bytes()
        || args[1] != "__f1-owned-host"
    {
        return Err("Native owned Host arguments differ.".into());
    }
    let main = loaded_image(&grant.endpoint.path)?;
    let sdk = loaded_image(&grant.sdk.path)?;
    if main.kind != 2
        || main.base != unsafe { _NSGetMachExecuteHeader() } as usize
        || sdk.kind != 6
        || sdk.base == main.base
    {
        return Err("Actual Native/SDK loaded image types differ.".into());
    }
    main.initializer_roster(None, |_| Err("Native initializer must not be read.".into()))?;
    for ty in &callbacks.types {
        for pin in std::iter::once(&ty.registration)
            .chain(ty.methods.iter().map(|method| &method.entry))
            .chain(ty.drop_glue.iter())
        {
            let offset = usize::try_from(pin.image_offset)
                .map_err(|_| "Owned Host callback offset exceeds address space.")?;
            // Executable containment is necessary, but is not compiler/type or
            // transitive cleanup authority. No callback is invoked here.
            main.code(offset)?;
        }
    }
    sdk.initializer_roster(Some(grant.initializer_offset), |entry| {
        mapped(entry, size_of::<usize>(), false)?;
        let target = unsafe { (entry as *const usize).read() };
        mapped(target, 1, true)?;
        Ok(target)
    })?;
    let mut functions = BTreeMap::new();
    for function in &grant.function_roster {
        if !FUNCTIONS.contains(&function.name.as_str())
            || functions
                .insert(function.name.clone(), sdk.code(function.image_offset)?)
                .is_some()
        {
            return Err("Owned Host function roster differs.".into());
        }
    }
    if functions.len() != FUNCTIONS.len() {
        return Err("Owned Host function roster incomplete.".into());
    }
    Ok((main, sdk, functions))
}
unsafe fn frame_paths(bytes: abi::Bytes) -> Result<String> {
    if bytes.length == 0 {
        return Ok(String::new());
    }
    if bytes.pointer.is_null() || bytes.length > 32_768 {
        return Err("Owned Host borrowed path bounds.".into());
    }
    // Caller contract: this entire borrowed range remains readable/immutable
    // until return. Length/version are not a pointer validator or a grant.
    let value =
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(bytes.pointer, bytes.length) })
            .map_err(|_| "Owned Host path UTF8.")?;
    if value.contains('\0') {
        return Err("Owned Host path contains NUL.".into());
    }
    Ok(value.to_owned())
}
unsafe fn frame_abi_prefix(frame: *const abi::HostBinding) -> Result<()> {
    if frame.is_null() || !(frame as usize).is_multiple_of(align_of::<abi::HostBinding>()) {
        return Err("Owned Host frame missing/unaligned.".into());
    }
    // Read only the common eight-byte prefix. An old104-byte frame must be
    // rejected before forming a reference to the new120-byte object.
    let prefix = unsafe { frame.cast::<[u32; 2]>().read_unaligned() };
    if prefix != [abi::HOST_BINDING_VERSION, size_of::<abi::HostBinding>() as u32] {
        return Err("Owned Host frame version/size differs.".into());
    }
    Ok(())
}
unsafe fn bind_frame(
    frame: *const abi::HostBinding,
    grant: &Grant,
    main: &Image,
    sdk: &Image,
    functions: &BTreeMap<String, usize>,
) -> Result<QualifiedOwnedHost> {
    unsafe { frame_abi_prefix(frame) }?;
    // Explicit unsafe C caller storage contract, not trust in these fields.
    let f = unsafe { &*frame };
    if f.version != abi::HOST_BINDING_VERSION
        || f.bytes as usize != size_of::<abi::HostBinding>()
        || f.reserved != 0
        || f.flags & !abi::HOST_HAS_CONFIG_OVERRIDE != 0
        || f.main_header != main.base
        || f.sdk_header != sdk.base
        || f.input_publisher.contains(&0)
    {
        return Err("Owned Host frame ABI/image mismatch.".into());
    }
    let profile = unsafe { frame_paths(f.profile) }?;
    let home = unsafe { frame_paths(f.home) }?;
    let config = if f.flags & abi::HOST_HAS_CONFIG_OVERRIDE != 0 {
        Some(unsafe { frame_paths(f.config_override) }?)
    } else {
        if !f.config_override.pointer.is_null() || f.config_override.length != 0 {
            return Err("Owned Host absent override carries bytes.".into());
        }
        None
    };
    if profile != grant.profile || home != grant.home || config != grant.config_override {
        return Err("Owned Host paths outside independent grant.".into());
    }
    let address = sdk
        .base
        .checked_add(grant.api_offset)
        .ok_or("Owned Host table offset overflow.")?;
    // Qualify the full table storage and every raw function word before forming
    // a typed Api reference; getters/ready have not yet been invoked.
    if f.api as usize != address
        || !address.is_multiple_of(align_of::<abi::Api>())
        || !sdk.contains(address, size_of::<abi::Api>(), false)
        || f.getter != functions["getter"]
        || f.ready != functions["ready"]
    {
        return Err("Owned Host table/getter location differs.".into());
    }
    mapped(address, size_of::<abi::Api>(), false)?;
    let raw = unsafe { std::slice::from_raw_parts(address as *const u8, size_of::<abi::Api>()) };
    let word = |offset| {
        usize::from_ne_bytes(
            raw[offset..offset + size_of::<usize>()]
                .try_into()
                .expect("fixed ABI word"),
        )
    };
    for (index, name) in FUNCTIONS[..4].iter().enumerate() {
        if word(std::mem::offset_of!(abi::Api, bind) + index * size_of::<usize>())
            != functions[*name]
        {
            return Err("Owned Host table function differs from independent roster.".into());
        }
    }
    let version = u32::from_ne_bytes(raw[..4].try_into().map_err(|_| "Owned Host ABI version.")?);
    let bytes = u32::from_ne_bytes(raw[4..8].try_into().map_err(|_| "Owned Host ABI size.")?);
    if version != abi::VERSION || bytes as usize != size_of::<abi::Api>() || raw[8..24] != [0; 16] {
        return Err("Owned Host table ABI differs.".into());
    }
    let ready = unsafe { std::mem::transmute::<usize, extern "C" fn() -> u64>(functions["ready"]) };
    let getter = unsafe {
        std::mem::transmute::<usize, unsafe extern "C" fn() -> *const abi::Api>(functions["getter"])
    };
    if ready() != READY || unsafe { getter() } != f.api {
        return Err("Qualified owned Host provider not fully admitted/live.".into());
    }
    let paths =
        unsafe { ProfilePaths::new(profile.into(), home.into(), config).with_pinned_io(&*f.api) }?;
    Ok(QualifiedOwnedHost {
        paths,
        fixture: grant.fixture.clone(),
        owner_uid: grant.owner_uid,
        fixture_sha256: grant.fixture_sha256.clone(),
        authority_sha256: grant.authority_sha256.clone(),
        sdk_sha256: grant.sdk.sha256.clone(),
        input_publisher: Arc::new(NativeInputPublisher {
            api: unsafe { &*f.api }, capability: abi::Handle { slot: f.input_publisher[0], generation: f.input_publisher[1] },
            owner: std::thread::current().id(), next_epoch: AtomicU64::new(1),
        }),
    })
}
unsafe fn receive(pin: Option<&str>, frame: *const abi::HostBinding) -> Result<()> {
    // This branch must precede even frame alignment/access and every provider
    // operation. An ordinary test-tools artifact has no Host grant at all.
    let pin: Pin = serde_json::from_str(pin.ok_or("Native Host compile pin missing.")?)
        .map_err(|_| "Native Host compile pin schema.")?;
    let mut state = match STATE.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => return Err("Native Host bind already in progress.".into()),
        Err(TryLockError::Poisoned(_)) => return Err("Native Host bind state unavailable.".into()),
    };
    let Phase::Empty(cached) = &mut *state else {
        return Err("Native Host already bound or consumed.".into());
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let sys = System::load()?;
        if cached.is_none() {
            let locator =
                std::env::var("F1_NATIVE_GRANT_FD").map_err(|_| "Native grant locator missing.")?;
            *cached = Some(Box::new(read_grant(&pin, &sys, &locator)?));
        }
        let grant = cached
            .as_ref()
            .ok_or("Native Host independent grant missing.")?;
        if std::env::var("F1_NATIVE_GRANT_FD").ok().as_deref() != Some(pin.fd.to_string().as_str())
        {
            return Err("Native grant locator changed.".into());
        }
        let (main, sdk, functions) = qualify(grant, &sys)?;
        unsafe { bind_frame(frame, grant, &main, &sdk, &functions) }
    }));
    match result {
        Ok(Ok(paths)) => {
            *state = Phase::Bound(Box::new(paths));
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(payload) => {
            std::mem::forget(payload);
            Err("Native Host bind panicked and was refused.".into())
        }
    }
}
/// # Safety
/// The separately source-qualified constructor must supply an aligned readable
/// HostBinding and immutable readable borrowed byte ranges for the whole call.
/// This storage contract grants no authority. Missing independent compile pins
/// refuse before any frame access. Qualified SDK callbacks must never unwind.
#[no_mangle]
pub unsafe extern "C" fn ruycast_owned_host_bind_v1(frame: *const abi::HostBinding) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        receive(option_env!("RUYCAST_F1_NATIVE_GRANT_PIN"), frame)
    })) {
        Ok(Ok(())) => 0,
        Ok(Err(_)) => 2,
        Err(payload) => {
            std::mem::forget(payload);
            2
        }
    }
}
/// Main consumes exactly one completed constructor binding before any ordinary
/// paths/config/home initialization; missing or duplicate consumption refuses.
pub fn take_bound_paths() -> Result<ProfilePaths> {
    take_bound_host().map(QualifiedOwnedHost::into_paths)
}
/// Consume the one-shot binding without erasing its admission provenance.
pub fn take_bound_host() -> Result<QualifiedOwnedHost> {
    let mut state = STATE
        .try_lock()
        .map_err(|_| "Native Host binding unavailable.")?;
    if !matches!(*state, Phase::Bound(_)) {
        return Err("Native Host constructor did not bind.".into());
    }
    let Phase::Bound(paths) = std::mem::replace(&mut *state, Phase::Empty(None)) else {
        unreachable!()
    };
    *state = Phase::Consumed(paths.input_publisher.clone());
    Ok(*paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    static STATE_TESTS: Mutex<()> = Mutex::new(());
    #[test]
    fn old_host_frame_refuses_using_only_its_common_prefix() {
        let mut old = [0u64; 13]; // Exactly104bytes; never form &HostBinding.
        unsafe { old.as_mut_ptr().cast::<[u32; 2]>().write([1, 104]); }
        assert!(unsafe { frame_abi_prefix(old.as_ptr().cast()) }.is_err());
        unsafe { old.as_mut_ptr().cast::<[u32; 2]>().write([2, 104]); }
        assert!(unsafe { frame_abi_prefix(old.as_ptr().cast()) }.is_err());
        let mut current = [0u64; 15];
        unsafe { current.as_mut_ptr().cast::<[u32; 2]>().write([2, 120]); }
        assert!(unsafe { frame_abi_prefix(current.as_ptr().cast()) }.is_ok());
        unsafe { current.as_mut_ptr().cast::<[u32; 2]>().write([1, 120]); }
        assert!(unsafe { frame_abi_prefix(current.as_ptr().cast()) }.is_err());
    }
    #[test]
    fn fixture_epoch_requires_exact_pinned_owned_manifest_and_fresh_bytes() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let parent = std::path::PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"),
        );
        let directory = parent.join(format!("host-fixture-epoch-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
        }
        let _cleanup = Cleanup(directory.clone());
        let file = directory.join("fixture.json");
        let good = br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":946728000,"files":{}}"#;
        std::fs::write(&file, good).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let owner = unsafe { libc::geteuid() };
        let sys = System::load().unwrap();
        let input = Input { path: file.to_str().unwrap().into(), sha256: digest(good) };
        assert_eq!(fixture_epoch(&input, &input.sha256, owner, &sys).unwrap(), 946728000);
        assert!(fixture_epoch(&input, &"0".repeat(64), owner, &sys).is_err());
        assert!(fixture_epoch(&input, &input.sha256, owner ^ 1, &sys).is_err());
        std::fs::write(&file, br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":946728001}"#).unwrap();
        assert!(fixture_epoch(&input, &input.sha256, owner, &sys).is_err(), "a later changed epoch must not acquire the original grant");
        for bad in [
            br#"{"schema":"wrong","epoch_unix_seconds":1}"#.as_slice(),
            br#"{"schema":"ruycast.f1.fixture.v1"}"#,
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":-1}"#,
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1.5}"#,
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1,"epoch_unix_seconds":2}"#,
        ] {
            std::fs::write(&file, bad).unwrap();
            let invalid = Input { path: input.path.clone(), sha256: digest(bad) };
            assert!(fixture_epoch(&invalid, &invalid.sha256, owner, &sys).is_err());
        }
        let oversized = vec![b' '; 2 * 1024 * 1024 + 1];
        std::fs::write(&file, &oversized).unwrap();
        let invalid = Input { path: input.path.clone(), sha256: digest(&oversized) };
        assert!(fixture_epoch(&invalid, &invalid.sha256, owner, &sys).is_err());
        std::fs::write(&file, good).unwrap();
        let alias = directory.join("alias.json");
        std::os::unix::fs::symlink(&file, &alias).unwrap();
        let linked = Input { path: alias.to_str().unwrap().into(), sha256: input.sha256.clone() };
        assert!(fixture_epoch(&linked, &linked.sha256, owner, &sys).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::hard_link(&file, &alias).unwrap();
        assert!(fixture_epoch(&input, &input.sha256, owner, &sys).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(fixture_epoch(&input, &input.sha256, owner, &sys).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fixture_epoch(&input, &input.sha256, owner, &sys).unwrap(), 946728000);
    }
    fn synthetic_image(sections: &[(&[u8], usize, usize, u32)], extra: &[u32]) -> Vec<u8> {
        fn word(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn wide(bytes: &mut [u8], at: usize, value: usize) {
            bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        let mut text = vec![0; 72];
        word(&mut text, 0, 0x19);
        word(&mut text, 4, 72);
        text[8..14].copy_from_slice(b"__TEXT");
        wide(&mut text, 24, 0x1000);
        wide(&mut text, 32, 0x1000);
        word(&mut text, 60, 5);
        let mut data = vec![0; 72 + 80 * sections.len()];
        word(&mut data, 0, 0x19);
        let len = data.len() as u32;
        word(&mut data, 4, len);
        data[8..20].copy_from_slice(b"__DATA_CONST");
        wide(&mut data, 24, 0x2000);
        wide(&mut data, 32, 0x1000);
        word(&mut data, 60, 1);
        word(&mut data, 64, sections.len() as u32);
        for (index, (name, address, length, kind)) in sections.iter().enumerate() {
            let at = 72 + 80 * index;
            data[at..at + name.len()].copy_from_slice(name);
            wide(&mut data, at + 32, *address);
            wide(&mut data, at + 40, *length);
            word(&mut data, at + 64, *kind);
        }
        let mut bytes = vec![0; 32];
        word(&mut bytes, 0, 0xfeedfacf);
        word(&mut bytes, 12, 6);
        word(&mut bytes, 16, 2 + extra.len() as u32);
        bytes.extend(text);
        bytes.extend(data);
        for command in extra {
            let size = match command {
                0x11 => 40usize,
                0x1a => 72,
                0x80000034 => 16,
                _ => 8,
            };
            bytes.extend(command.to_le_bytes());
            bytes.extend((size as u32).to_le_bytes());
            bytes.resize(bytes.len() + size - 8, 0);
        }
        let size = bytes.len() as u32 - 32;
        word(&mut bytes, 20, size);
        bytes
    }
    #[test]
    fn initializer_roles_require_zero_main_and_one_exact_sdk_entry() {
        let none = Image::commands(0x100000, &synthetic_image(&[], &[])).unwrap();
        assert!(
            none.initializer_roster(None, |_| panic!("main must not read an entry"))
                .is_ok()
        );
        assert!(
            none.initializer_roster(Some(0x100), |_| panic!("missing SDK entry"))
                .is_err()
        );
        let sdk = Image::commands(
            0x100000,
            &synthetic_image(&[(b"__mod_init_func", 0x2000, 8, 9)], &[]),
        )
        .unwrap();
        assert!(
            sdk.initializer_roster(None, |_| panic!("forbidden main entry"))
                .is_err()
        );
        let calls = std::cell::Cell::new(0);
        assert!(
            sdk.initializer_roster(Some(0x100), |entry| {
                calls.set(calls.get() + 1);
                assert_eq!(entry, 0x101000);
                Ok(0x100100)
            })
            .is_ok()
        );
        assert_eq!(calls.get(), 1);
        assert!(
            sdk.initializer_roster(Some(0x100), |_| Ok(0x100200))
                .is_err()
        );
        assert!(
            sdk.initializer_roster(Some(0x1000), |_| panic!("non-RX target"))
                .is_err()
        );
        assert!(
            sdk.initializer_roster(Some(usize::MAX), |_| panic!("overflow target"))
                .is_err()
        );
    }
    #[test]
    fn initializer_alternatives_and_entry_roster_refuse_before_pointer_read() {
        for command in [0x11, 0x1a] {
            let image = Image::commands(0x100000, &synthetic_image(&[], &[command])).unwrap();
            assert!(
                image
                    .initializer_roster(None, |_| panic!("LC_ROUTINES"))
                    .is_err()
            );
        }
        for (name, kind) in [
            (b"__objc_nlclslist".as_slice(), 0),
            (b"__objc_nlcatlist".as_slice(), 0),
            (b"__thread_init".as_slice(), 0x15),
            (b"__init_offsets".as_slice(), 0x16),
        ] {
            let image =
                Image::commands(0x100000, &synthetic_image(&[(name, 0x2000, 8, kind)], &[]))
                    .unwrap();
            assert!(
                image
                    .initializer_roster(None, |_| panic!("alternate initializer"))
                    .is_err()
            );
            assert!(
                image
                    .initializer_roster(Some(0x100), |_| panic!("alternate initializer"))
                    .is_err()
            );
        }
        for sections in [
            vec![(b"__mod_init_func".as_slice(), 0x2000, 16, 9)],
            vec![(b"__mod_init_func".as_slice(), 0x2001, 8, 9)],
            vec![
                (b"__mod_init_func".as_slice(), 0x2000, 8, 9),
                (b"other".as_slice(), 0x2010, 8, 9),
            ],
        ] {
            let image = Image::commands(0x100000, &synthetic_image(&sections, &[])).unwrap();
            assert!(
                image
                    .initializer_roster(Some(0x100), |_| panic!("invalid initializer roster"))
                    .is_err()
            );
        }
        let renamed =
            Image::commands(0x100000, &synthetic_image(&[(b"other", 0x2000, 8, 9)], &[])).unwrap();
        assert!(
            renamed
                .initializer_roster(None, |_| panic!("typed initializer under another name"))
                .is_err()
        );
        let wrong_type = Image::commands(
            0x100000,
            &synthetic_image(&[(b"__mod_init_func", 0x2000, 8, 0)], &[]),
        )
        .unwrap();
        assert!(
            wrong_type
                .initializer_roster(None, |_| panic!("wrong type"))
                .is_err()
        );
        // Chained fixups alone are not an initializer or a blanket refusal.
        let image = Image::commands(0x100000, &synthetic_image(&[], &[0x80000034])).unwrap();
        assert!(
            image
                .initializer_roster(None, |_| panic!("no initializer"))
                .is_ok()
        );
    }
    #[test]
    fn initializer_command_parser_bounds_sections_and_address_arithmetic() {
        let baseline = synthetic_image(&[(b"__mod_init_func", 0x2000, 8, 9)], &[]);
        for mutation in 0..6 {
            let mut bytes = baseline.clone();
            match mutation {
                0 => bytes[32 + 72 + 64..32 + 72 + 68].copy_from_slice(&u32::MAX.to_le_bytes()),
                1 => bytes[32 + 72 + 72 + 32..32 + 72 + 72 + 40]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                2 => bytes[32 + 72 + 72 + 40..32 + 72 + 72 + 48]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                3 => bytes[32 + 4..32 + 8].copy_from_slice(&7u32.to_le_bytes()),
                4 => {
                    bytes.pop();
                }
                _ => bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes()),
            }
            assert!(
                Image::commands(0x100000, &bytes).is_err(),
                "mutation {mutation}"
            );
        }
        assert!(Image::commands(usize::MAX, &baseline).is_err());
    }
    #[test]
    fn missing_compile_pin_refuses_before_invalid_frame_access() {
        let _serial = STATE_TESTS.lock().unwrap();
        let invalid = std::ptr::dangling::<abi::HostBinding>();
        assert!(unsafe { receive(None, invalid) }.is_err());
        assert!(matches!(*STATE.try_lock().unwrap(), Phase::Empty(_)));
    }
    #[test]
    fn concurrent_entry_refuses_without_frame_access_or_latch_poison() {
        let _serial = STATE_TESTS.lock().unwrap();
        let guard = STATE.try_lock().unwrap();
        let pin = r#"{"fd":4,"fingerprint":{"device":1,"inode":2,"kind":4096,"access":0},"nonce_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        assert!(unsafe { receive(Some(pin), std::ptr::dangling::<abi::HostBinding>()) }.is_err());
        assert!(matches!(*guard, Phase::Empty(_)));
        drop(guard);
        assert!(STATE.try_lock().is_ok());
    }
    #[test]
    fn own_fifo_requires_exact_identity_nonce_owner_and_complete_eof_frame() {
        let sys = System::load().unwrap_or_else(|error| panic!("{error}"));
        for case in 0..7 {
            let mut pair = [-1; 2];
            assert_eq!(unsafe { libc::pipe(pair.as_mut_ptr()) }, 0);
            assert!(pair[0] > 2 && pair[1] > 2);
            let owner = sys.stat(pair[0]).unwrap().st_uid;
            let body=serde_json::to_vec(&serde_json::json!({
                "role":ROLE,"nonce":if case==1 {"wrong"} else {"owned-fixture-nonce"},
                "owner_uid":if case==3 {owner^1} else {owner},
                "endpoint":{"path":"/owned/host","sha256":"a"},
                "sdk":{"path":"/owned/sdk","sha256":"b"},
                "source_manifest":{"path":"/owned/sources","sha256":"c"},
                "native_callback_roster":{"path":"/owned/callbacks","sha256":"f"},
                "source_set_sha256":"d","fixture":{"path":"/owned/fixture.json","sha256":"1".repeat(64)},"fixture_sha256":"1".repeat(64),"authority_sha256":"2".repeat(64),
                "api_header":{"path":"/owned/header","sha256":"e"},
                "api_offset":8,"initializer_offset":16,"function_roster":[],"profile":"/owned/profile","home":"/owned/home", "config_override":null
            })).unwrap();
            let mut wire = (body.len() as u32).to_ne_bytes().to_vec();
            wire.extend(body);
            if case == 2 {
                wire.push(1);
            }
            assert_eq!(
                unsafe { libc::write(pair[1], wire.as_ptr().cast(), wire.len()) },
                wire.len() as isize
            );
            if case != 5 {
                assert_eq!(unsafe { (sys.close)(pair[1]) }, 0);
            }
            let mut pin = Pin {
                fd: pair[0],
                fingerprint: sys.fingerprint(pair[0]).unwrap(),
                nonce_sha256: digest(b"owned-fixture-nonce"),
            };
            if case == 4 {
                pin.fingerprint.inode ^= 1;
            }
            let locator = if case == 6 {
                (pair[0] + 1).to_string()
            } else {
                pair[0].to_string()
            };
            let result = read_grant(&pin, &sys, &locator);
            assert_eq!(result.is_ok(), case == 0, "case {case}");
            if case == 4 || case == 6 {
                // An unqualified cap was never taken; the known test owner closes it.
                assert!(sys.stat(pair[0]).is_ok());
                assert_eq!(unsafe { (sys.close)(pair[0]) }, 0);
            } else {
                assert_ne!(
                    sys.fingerprint(pair[0]).ok().as_ref(),
                    Some(&pin.fingerprint),
                    "readcap leaked in case {case}"
                );
            }
            if case == 5 {
                assert_eq!(unsafe { (sys.close)(pair[1]) }, 0);
            }
        }
    }
    #[test]
    fn prequalification_resolves_only_already_loaded_system_code() {
        System::load().unwrap_or_else(|error| panic!("{error}"));
    }
    #[test]
    fn unknown_provider_refuses_before_code_or_table_calls() {
        assert!(loaded_image("/unqualified/ruycast-provider-never-loaded.dylib").is_err());
        let image = Image {
            base: 0x1000,
            kind: 6,
            segments: vec![(0x1000, 0x2000, 1)],
            initializer_sections: vec![],
            alternative_initializers: false,
        };
        assert!(image.code(0x10).is_err());
        assert!(!image.contains(usize::MAX, 64, false));
    }
    #[test]
    fn grant_schema_preserves_override_absent_versus_empty() {
        assert!(serde_json::from_str::<Pin>(r#"{"fd":4,"fingerprint":{"device":1,"inode":2,"kind":4096,"access":0},"nonce_sha256":"x","extra":1}"#).is_err());
        assert!(path("relative").is_err());
        assert!(path("/owned/../other").is_err());
        let mut object = serde_json::json!({
            "role": ROLE, "nonce": "fixture", "owner_uid": 501, "endpoint": {"path":"/owned/host","sha256":"a"},
            "sdk": {"path":"/owned/sdk","sha256":"b"}, "source_manifest": {"path":"/owned/source","sha256":"c"},
            "native_callback_roster":{"path":"/owned/callbacks","sha256":"f"},
            "source_set_sha256":"d", "fixture":{"path":"/owned/fixture.json","sha256":"1".repeat(64)}, "fixture_sha256":"1".repeat(64), "authority_sha256":"2".repeat(64),
            "api_header":{"path":"/owned/header","sha256":"e"},
            "api_offset": 8, "initializer_offset": 16, "function_roster": [], "profile":"/owned/profile", "home":"/owned/home"
        });
        let mut missing_initializer = object.clone();
        missing_initializer
            .as_object_mut()
            .unwrap()
            .remove("initializer_offset");
        assert!(serde_json::from_value::<Grant>(missing_initializer).is_err());
        for key in [
            "fixture",
            "fixture_sha256",
            "authority_sha256",
            "native_callback_roster",
        ] {
            let mut missing_binding = object.clone();
            missing_binding.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<Grant>(missing_binding).is_err());
        }
        let absent: Grant = serde_json::from_value(object.clone()).unwrap();
        object["config_override"] = serde_json::json!("");
        let empty: Grant = serde_json::from_value(object).unwrap();
        assert_eq!(absent.config_override, None);
        assert_eq!(empty.config_override.as_deref(), Some(""));
        assert_eq!(
            unsafe {
                frame_paths(abi::Bytes {
                    pointer: std::ptr::null(),
                    length: 0,
                })
            }
            .unwrap(),
            ""
        );
        assert!(
            unsafe {
                frame_paths(abi::Bytes {
                    pointer: std::ptr::null(),
                    length: 1,
                })
            }
            .is_err()
        );
    }
}
