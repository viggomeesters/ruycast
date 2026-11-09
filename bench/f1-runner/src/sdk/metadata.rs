//! Non-resolving Objective-C metadata only. No messages, method lookup caches,
//! class handler, instance allocation or original IMP calls. Snapshot identity
//! is not source/callback authority and cannot qualify an SDK scope by itself.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void, CStr},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
type Result<T> = std::result::Result<T, String>;
type Class = *const c_void;
type Method = *const c_void;
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_copyClassList(count: *mut u32) -> *mut Class;
    fn class_copyMethodList(class: Class, count: *mut u32) -> *mut Method;
    fn class_getSuperclass(class: Class) -> Class;
    fn object_getClass(object: Class) -> Class;
    fn class_getName(class: Class) -> *const c_char;
    fn sel_getName(selector: *const c_void) -> *const c_char;
    fn method_getName(method: Method) -> *const c_void;
    fn method_getImplementation(method: Method) -> *const c_void;
    fn method_getTypeEncoding(method: Method) -> *const c_char;
    fn free(p: *mut c_void);
}
#[repr(C)]
struct DlInfo {
    name: *const c_char,
    base: *const c_void,
    symbol: *const c_char,
    symbol_address: *const c_void,
}
unsafe extern "C" {
    fn dladdr(address: *const c_void, info: *mut DlInfo) -> i32;
}
struct List<T>(*mut T);
impl<T> Drop for List<T> {
    fn drop(&mut self) {
        unsafe { free(self.0.cast()) }
    }
}
unsafe fn with_text<T>(p: *const c_char, read: impl FnOnce(&[u8]) -> T) -> Result<T> {
    if p.is_null() {
        return Err("null runtime metadata string".into());
    }
    let length = unsafe { libc::strnlen(p, 4096) };
    if length == 4096 {
        return Err("runtime metadata string exceeds bound".into());
    }
    Ok(read(unsafe { std::slice::from_raw_parts(p.cast(), length) }))
}
unsafe fn text(p: *const c_char) -> Result<Vec<u8>> {
    unsafe { with_text(p, <[u8]>::to_vec) }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClassHandle(usize);
impl ClassHandle {
    pub fn address(self) -> usize {
        self.0
    }
}
pub fn class_name(class: ClassHandle) -> Result<Vec<u8>> {
    unsafe { text(class_getName(class.0 as Class)) }
}
pub fn loaded_classes() -> Result<Vec<ClassHandle>> {
    let mut n = 0;
    let p = unsafe { objc_copyClassList(&mut n) };
    let list = List(p);
    // The complete actual copy is validated on EVERY query. No roster survives
    // this call or crosses a metadata write, callback or business effect.
    unsafe { checked_roster(list.0, n) }
}
unsafe fn checked_roster(p: *const Class, n: u32) -> Result<Vec<ClassHandle>> {
    if p.is_null() || n > 100_000 {
        return Err("loaded class roster invalid".into());
    }
    let mut addresses = Vec::with_capacity(n as usize);
    for c in unsafe { std::slice::from_raw_parts(p, n as usize) } {
        if c.is_null() {
            return Err("nil/duplicate loaded class".into());
        }
        addresses.push(*c as usize);
    }
    addresses.sort_unstable();
    if addresses.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("nil/duplicate loaded class".into());
    }
    Ok(addresses.into_iter().map(ClassHandle).collect())
}
pub fn registered_class(address: usize) -> Result<ClassHandle> {
    let roster = loaded_classes()?;
    roster
        .binary_search_by_key(&address, |class| class.0)
        .map(|index| roster[index])
        .map_err(|_| "class address is not in actual registered roster".into())
}
pub fn loaded_class(name: &CStr) -> Result<ClassHandle> {
    let mut found = None;
    for c in loaded_classes()? {
        let matches = unsafe { with_text(class_getName(c.0 as Class), |actual| actual == name.to_bytes()) }?;
        if matches && found.replace(c).is_some() {
            return Err("duplicate class name".into());
        }
    }
    found.ok_or_else(|| "attested loaded class missing".into())
}

/// One pure synchronous verification's complete registered-address observation.
/// Membership/name resolution and Eq-only whole-six lifecycle observations are
/// shared only under the existing pure, exclusive/no-class-write premise. Raw
/// selected methods/code and every proof/retirement check remain fresh. No
/// instance of this type escapes its enclosing closure or crosses
/// a callback, metadata write, business effect or subsequent verification.
pub(crate) struct VerificationRoster {
    classes: std::cell::OnceCell<Result<Vec<ClassHandle>>>,
    names: std::cell::OnceCell<Vec<std::cell::OnceCell<Result<std::ops::Range<usize>>>>>,
    name_bytes: std::cell::RefCell<Vec<u8>>,
    named_classes: std::cell::RefCell<BTreeMap<Vec<u8>, Result<ClassHandle>>>,
    lifecycle_observations: std::cell::RefCell<Vec<(ClassHandle, Result<LifecycleSnapshot>)>>,
}
pub(crate) fn with_verification_roster<T>(
    verify: impl FnOnce(&VerificationRoster) -> Result<T>,
) -> Result<T> {
    let roster = VerificationRoster {
        classes: std::cell::OnceCell::new(),
        names: std::cell::OnceCell::new(),
        name_bytes: std::cell::RefCell::new(Vec::new()),
        named_classes: std::cell::RefCell::new(BTreeMap::new()),
        lifecycle_observations: std::cell::RefCell::new(Vec::new()),
    };
    verify(&roster)
}
impl VerificationRoster {
    // Eq-only observation, never a snapshot, SourceCapture or proof output.
    // First read remains at the old lifecycle-read position. Every caller still
    // validates membership and its own baseline, retired and sticky guards.
    fn lifecycle_matches(&self, class: ClassHandle, expected: &LifecycleSnapshot) -> Result<bool> {
        let mut observations = self.lifecycle_observations.borrow_mut();
        let index = match observations
            .iter()
            .position(|(observed, _)| *observed == class)
        {
            Some(index) => index,
            None => {
                observations.push((class, lifecycle(class)));
                observations.len() - 1
            }
        };
        observations[index]
            .1
            .as_ref()
            .map(|actual| actual == expected)
            .map_err(Clone::clone)
    }
    fn classes(&self) -> Result<&[ClassHandle]> {
        // Capture at the first old roster-read position, not ahead of existing
        // retired/faulted guards. A failure stays inside their sticky catch.
        self.classes
            .get_or_init(loaded_classes)
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }
    pub(crate) fn registered_class(&self, address: usize) -> Result<ClassHandle> {
        let classes = self.classes()?;
        classes
            .binary_search_by_key(&address, |class| class.0)
            .map(|index| classes[index])
            .map_err(|_| "class address is not in actual registered roster".into())
    }
    pub(crate) fn loaded_class(&self, name: &CStr) -> Result<ClassHandle> {
        if let Some(observed) = self.named_classes.borrow().get(name.to_bytes()) {
            return observed.clone();
        }
        let observed = self.find_loaded_class(name);
        self.named_classes.borrow_mut().insert(name.to_bytes().to_vec(), observed.clone());
        observed
    }

    fn find_loaded_class(&self, name: &CStr) -> Result<ClassHandle> {
        let classes = self.classes()?;
        let names = self
            .names
            .get_or_init(|| classes.iter().map(|_| std::cell::OnceCell::new()).collect());
        let mut found = None;
        for (&class, observed) in classes.iter().zip(names) {
            // Observe each name at its original loop position. In particular,
            // an earlier duplicate still refuses before any later bad name.
            let range = observed
                .get_or_init(|| unsafe {
                    with_text(class_getName(class.0 as Class), |name| self.copy_name(name))
                })
                .as_ref()
                .map_err(Clone::clone)?;
            if &self.name_bytes.borrow()[range.clone()] == name.to_bytes() && found.replace(class).is_some() {
                return Err("duplicate class name".into());
            }
        }
        found.ok_or_else(|| "attested loaded class missing".into())
    }

    // Names remain owned by the short-lived roster. Append into one buffer so
    // a full scan does not allocate a separate vector for every loaded class.
    fn copy_name(&self, name: &[u8]) -> std::ops::Range<usize> {
        let mut bytes = self.name_bytes.borrow_mut();
        let start = bytes.len();
        bytes.extend_from_slice(name);
        start..bytes.len()
    }
}
fn chain(mut c: usize) -> Result<Vec<usize>> {
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    while c != 0 {
        if out.len() >= 256 || !seen.insert(c) {
            return Err("invalid class ancestry".into());
        }
        out.push(c);
        c = unsafe { class_getSuperclass(c as Class) } as usize;
    }
    Ok(out)
}
pub fn is_descendant(class: ClassHandle, family: &CStr) -> Result<bool> {
    for c in chain(class.0)? {
        if unsafe { text(class_getName(c as Class)) }? == family.to_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Actual loaded-image/code identity. UUID, executable segment and image-relative
/// code offset are observed from the loaded Mach-O, never accepted from JSON.
/// This does not replace exact pinned source/image authority in full admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeIdentity {
    image: Vec<u8>,
    base: usize,
    offset: usize,
    uuid: [u8; 16],
    segment_start: usize,
    segment_end: usize,
}
impl CodeIdentity {
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn uuid(&self)->[u8;16] {self.uuid}
}
/// DATA-only qualification of the exact runtime witness selected by genuine
/// Foundation Notification metadata. No Swift call or value/ARC permit.
/// Swift RuntimeFunctions.def declares GenericDestroy as C_CC void(value*,
/// const Metadata*); the original pinned CTo supplies precisely x0/x1.
/// https://github.com/swiftlang/swift/blob/main/include/swift/Runtime/RuntimeFunctions.def
pub(crate) fn verify_notification_runtime_destroy(address:usize,observed:&CodeIdentity)->Result<()> {
    #[link(name="swiftCore")]
    unsafe extern "C" {
        #[link_name="swift_cvw_destroy"]
        fn NOTIFICATION_RUNTIME_DESTROY(value:*mut c_void,metadata:*const c_void);
    }
    let expected=NOTIFICATION_RUNTIME_DESTROY as *const () as usize;
    if address!=expected || observed.image()!=b"/usr/lib/swift/libswiftCore.dylib"
        || code_identity(address)?!=*observed {
        return Err("Notification destroy is not the exact selected Swift runtime export".into());
    }
    let mut info=DlInfo{name:ptr::null(),base:ptr::null(),symbol:ptr::null(),symbol_address:ptr::null()};
    if unsafe{dladdr(address as *const c_void,&mut info)}==0 || info.base as usize!=observed.base
        || info.symbol_address as usize!=address || info.symbol.is_null()
        || unsafe{text(info.symbol)}?!=b"swift_cvw_destroy" {
        return Err("Notification runtime destroy named entry/provider differs".into());
    }
    if code_identity(expected)?!=*observed{return Err("Notification runtime destroy changed during qualification".into());}
    Ok(())
}
pub fn code_identity(address: usize) -> Result<CodeIdentity> {
    if address == 0 {
        return Err("nil code address".into());
    }
    let mut d = DlInfo {
        name: ptr::null(),
        base: ptr::null(),
        symbol: ptr::null(),
        symbol_address: ptr::null(),
    };
    if unsafe { dladdr(address as *const c_void, &mut d) } == 0 || d.base.is_null() {
        return Err("code image missing".into());
    }
    let image = unsafe { text(d.name) }?;
    let base = d.base as usize;
    let b = d.base.cast::<u8>();
    let u32at = |n| unsafe { b.add(n).cast::<u32>().read_unaligned() };
    let u64at = |n| unsafe { b.add(n).cast::<u64>().read_unaligned() };
    if u32at(0) != 0xfeedfacf {
        return Err("code image is not Mach-O64".into());
    }
    let count = u32at(16) as usize;
    let bytes = u32at(20) as usize;
    if count > 65536 || bytes > 16 * 1024 * 1024 {
        return Err("Mach-O load command bounds".into());
    }
    let end = 32usize.checked_add(bytes).ok_or("Mach-O size overflow")?;
    let mut n = 32;
    let mut text_vm = None;
    let mut uuid = None;
    let mut segments = vec![];
    for _ in 0..count {
        if n + 8 > end {
            return Err("truncated load command".into());
        }
        let kind = u32at(n);
        let size = u32at(n + 4) as usize;
        if size < 8 || n.checked_add(size).is_none_or(|x| x > end) {
            return Err("load command range".into());
        }
        if kind == 0x19 {
            if size < 72 {
                return Err("short segment command".into());
            }
            let vm = u64at(n + 24) as usize;
            let len = u64at(n + 32) as usize;
            let prot = u32at(n + 60);
            let name = unsafe { std::slice::from_raw_parts(b.add(n + 8), 16) };
            if name == b"__TEXT\0\0\0\0\0\0\0\0\0\0" && text_vm.replace(vm).is_some() {
                return Err("duplicate text segment".into());
            }
            segments.push((vm, len, prot));
        }
        if kind == 0x1b {
            if size != 24 || uuid.is_some() {
                return Err("invalid image UUID".into());
            }
            let mut id = [0; 16];
            unsafe { ptr::copy_nonoverlapping(b.add(n + 8), id.as_mut_ptr(), 16) }
            uuid = Some(id);
        }
        n += size;
    }
    if n != end {
        return Err("load command inventory differs".into());
    }
    let text = text_vm.ok_or("no text segment")?;
    let mut selected = None;
    for (vm, len, prot) in segments {
        if prot & 4 == 0 {
            continue;
        }
        let start = if vm >= text {
            base.checked_add(vm - text)
        } else {
            base.checked_sub(text - vm)
        }
        .ok_or("code segment slide overflow")?;
        let end = start.checked_add(len).ok_or("code segment overflow")?;
        if address >= start && address < end && selected.replace((start, end)).is_some() {
            return Err("overlapping executable segments".into());
        }
    }
    let (start, end) = selected.ok_or("IMP outside executable image segments")?;
    Ok(CodeIdentity {
        image,
        base,
        offset: address.checked_sub(base).ok_or("IMP precedes image")?,
        uuid: uuid.ok_or("code image lacks UUID")?,
        segment_start: start,
        segment_end: end,
    })
}
// One fresh dladdr/header observation, local to one verification pass only.
struct ObservedCodeImage {
    image: Vec<u8>,
    base: usize,
    uuid: [u8; 16],
    executable: Vec<(usize, usize)>,
}
impl ObservedCodeImage {
    fn identity(&self, address: usize) -> Result<CodeIdentity> {
        let mut selected = None;
        for &(start, end) in &self.executable {
            if address >= start && address < end && selected.replace((start, end)).is_some() {
                return Err("overlapping executable segments".into());
            }
        }
        let (start, end) = selected.ok_or("IMP outside executable image segments")?;
        Ok(CodeIdentity {
            image: self.image.clone(),
            base: self.base,
            offset: address.checked_sub(self.base).ok_or("IMP precedes image")?,
            uuid: self.uuid,
            segment_start: start,
            segment_end: end,
        })
    }
}
fn observe_code_image(address: usize) -> Result<ObservedCodeImage> {
    if address == 0 {
        return Err("nil code address".into());
    }
    let mut d = DlInfo {
        name: ptr::null(),
        base: ptr::null(),
        symbol: ptr::null(),
        symbol_address: ptr::null(),
    };
    if unsafe { dladdr(address as *const c_void, &mut d) } == 0 || d.base.is_null() {
        return Err("code image missing".into());
    }
    let image = unsafe { text(d.name) }?;
    let base = d.base as usize;
    let b = d.base.cast::<u8>();
    let u32at = |n| unsafe { b.add(n).cast::<u32>().read_unaligned() };
    let u64at = |n| unsafe { b.add(n).cast::<u64>().read_unaligned() };
    if u32at(0) != 0xfeedfacf {
        return Err("code image is not Mach-O64".into());
    }
    let count = u32at(16) as usize;
    let bytes = u32at(20) as usize;
    if count > 65536 || bytes > 16 * 1024 * 1024 {
        return Err("Mach-O load command bounds".into());
    }
    let end = 32usize.checked_add(bytes).ok_or("Mach-O size overflow")?;
    let mut n = 32;
    let mut text_vm = None;
    let mut uuid = None;
    let mut segments = vec![];
    for _ in 0..count {
        if n + 8 > end {
            return Err("truncated load command".into());
        }
        let kind = u32at(n);
        let size = u32at(n + 4) as usize;
        if size < 8 || n.checked_add(size).is_none_or(|x| x > end) {
            return Err("load command range".into());
        }
        if kind == 0x19 {
            if size < 72 {
                return Err("short segment command".into());
            }
            let vm = u64at(n + 24) as usize;
            let len = u64at(n + 32) as usize;
            let prot = u32at(n + 60);
            let name = unsafe { std::slice::from_raw_parts(b.add(n + 8), 16) };
            if name == b"__TEXT\0\0\0\0\0\0\0\0\0\0" && text_vm.replace(vm).is_some() {
                return Err("duplicate text segment".into());
            }
            segments.push((vm, len, prot));
        }
        if kind == 0x1b {
            if size != 24 || uuid.is_some() {
                return Err("invalid image UUID".into());
            }
            let mut id = [0; 16];
            unsafe { ptr::copy_nonoverlapping(b.add(n + 8), id.as_mut_ptr(), 16) }
            uuid = Some(id);
        }
        n += size;
    }
    if n != end {
        return Err("load command inventory differs".into());
    }
    let text = text_vm.ok_or("no text segment")?;
    let mut executable = vec![];
    for (vm, len, prot) in segments {
        if prot & 4 == 0 {
            continue;
        }
        let start = if vm >= text {
            base.checked_add(vm - text)
        } else {
            base.checked_sub(text - vm)
        }
        .ok_or("code segment slide overflow")?;
        let end = start.checked_add(len).ok_or("code segment overflow")?;
        executable.push((start, end));
    }
    let observed = ObservedCodeImage {
        image,
        base,
        uuid: uuid.ok_or("code image lacks UUID")?,
        executable,
    };
    // Even the representative must belong to exactly one actual RX range.
    observed.identity(address)?;
    Ok(observed)
}

#[derive(Clone, Debug)]
pub struct MethodSnapshot {
    receiver: usize,
    dispatch: usize,
    owner: usize,
    selector: Vec<u8>,
    method: usize,
    implementation: usize,
    encoding: Vec<u8>,
    ancestry: Vec<(usize, Vec<u8>)>,
    code: CodeIdentity,
    source_capture: Option<Arc<SourceCapture>>,
}
impl PartialEq for MethodSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.receiver == other.receiver
            && self.dispatch == other.dispatch
            && self.owner == other.owner
            && self.selector == other.selector
            && self.method == other.method
            && self.implementation == other.implementation
            && self.encoding == other.encoding
            && self.ancestry == other.ancestry
            && self.code == other.code
    }
}
impl Eq for MethodSnapshot {}
impl MethodSnapshot {
    pub fn implementation(&self) -> usize {
        self.implementation
    }
    pub fn owner(&self) -> usize {
        self.owner
    }
    pub fn encoding(&self) -> &[u8] {
        &self.encoding
    }
    pub fn code(&self) -> &CodeIdentity {
        &self.code
    }
    pub fn method_address(&self) -> usize {
        self.method
    }
}
fn lookup(receiver: usize, dispatch: usize, selector: &CStr) -> Result<Option<MethodSnapshot>> {
    let ancestry = chain(dispatch)?;
    let names = ancestry
        .iter()
        .map(|&c| Ok((c, unsafe { text(class_getName(c as Class)) }?)))
        .collect::<Result<Vec<_>>>()?;
    for owner in ancestry {
        let mut count = 0;
        let p = unsafe { class_copyMethodList(owner as Class, &mut count) };
        let list = List(p);
        if count > 65536 || (count != 0 && p.is_null()) {
            return Err("method inventory bounds".into());
        }
        let mut found = None;
        if count != 0 {
            for &m in unsafe { std::slice::from_raw_parts(list.0, count as usize) } {
                if m.is_null() {
                    return Err("nil method metadata".into());
                }
                let name = unsafe { text(sel_getName(method_getName(m))) }?;
                if name == selector.to_bytes() {
                    if found.is_some() {
                        return Err("duplicate declared selector".into());
                    }
                    let implementation = unsafe { method_getImplementation(m) } as usize;
                    let encoding = unsafe { text(method_getTypeEncoding(m)) }?;
                    if encoding.is_empty() {
                        return Err("empty method ABI".into());
                    }
                    found = Some(MethodSnapshot {
                        receiver,
                        dispatch,
                        owner,
                        selector: name,
                        method: m as usize,
                        implementation,
                        encoding,
                        ancestry: names.clone(),
                        code: code_identity(implementation)?,
                        source_capture: None,
                    });
                }
            }
        }
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}
fn raw_method(
    class: ClassHandle,
    selector: &CStr,
    class_method: bool,
) -> Result<Option<MethodSnapshot>> {
    let dispatch = if class_method {
        (unsafe { object_getClass(class.0 as Class) }) as usize
    } else {
        class.0
    };
    if dispatch == 0 {
        return Err("nil metaclass".into());
    }
    lookup(class.0, dispatch, selector)
}
/// Observe exact current method equality only. This cannot issue a source
/// baseline, constructor authority or replacement provenance: no snapshot or
/// SourceCapture leaves this function. Both complete observations are fresh.
pub(crate) fn verify_current_method(
    class: ClassHandle,
    selector: &CStr,
    class_method: bool,
    expected: &MethodSnapshot,
) -> Result<()> {
    for _ in 0..2 {
        if raw_method(class, selector, class_method)?.as_ref() != Some(expected) {
            return Err("current method metadata differs from captured method".into());
        }
    }
    Ok(())
}

struct ObservedMethod {
    receiver: usize,
    dispatch: usize,
    owner: usize,
    selector: Vec<u8>,
    method: usize,
    implementation: usize,
    encoding: Vec<u8>,
    ancestry: Vec<(usize, Vec<u8>)>,
}
impl ObservedMethod {
    fn matches(&self, expected: &MethodSnapshot) -> bool {
        self.receiver == expected.receiver
            && self.dispatch == expected.dispatch
            && self.owner == expected.owner
            && self.selector == expected.selector
            && self.method == expected.method
            && self.implementation == expected.implementation
            && self.encoding == expected.encoding
            && self.ancestry == expected.ancestry
    }
}
// One pass's complete actual method-address inventory for a physical owner.
// Nil/name validation stays lazy at the original query-loop slot, so an earlier
// selected ABI or duplicate error still precedes a later malformed slot.
struct DeclaredMethods {
    addresses: Vec<usize>,
    names: Vec<std::cell::OnceCell<Result<Vec<u8>>>>,
}
impl DeclaredMethods {
    fn observe(owner: usize) -> Result<Self> {
        let mut count = 0;
        let p = unsafe { class_copyMethodList(owner as Class, &mut count) };
        let list = List(p);
        if count > 65536 || (count != 0 && p.is_null()) {
            return Err("method inventory bounds".into());
        }
        let addresses = if count == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(list.0, count as usize) }
                .iter()
                .map(|&method| method as usize)
                .collect()
        };
        let names = (0..count).map(|_| std::cell::OnceCell::new()).collect();
        Ok(Self { addresses, names })
    }
}
fn observe_method_fields(
    receiver: usize,
    dispatch: usize,
    selector: &CStr,
    owners: &mut std::collections::BTreeMap<usize, Result<DeclaredMethods>>,
) -> Result<Option<ObservedMethod>> {
    let ancestry = chain(dispatch)?;
    let names = ancestry
        .iter()
        .map(|&c| Ok((c, unsafe { text(class_getName(c as Class)) }?)))
        .collect::<Result<Vec<_>>>()?;
    for owner in ancestry {
        let inventory = owners
            .entry(owner)
            .or_insert_with(|| DeclaredMethods::observe(owner));
        let declared = inventory.as_ref().map_err(Clone::clone)?;
        let mut found = None;
        for (&address, observed_name) in declared.addresses.iter().zip(&declared.names) {
            if address == 0 {
                return Err("nil method metadata".into());
            }
            let m = address as Method;
            let name = observed_name
                .get_or_init(|| unsafe { text(sel_getName(method_getName(m))) })
                .as_ref()
                .map_err(Clone::clone)?;
            if name == selector.to_bytes() {
                if found.is_some() {
                    return Err("duplicate declared selector".into());
                }
                // Every selected method's actual IMP and ABI are still fresh
                // for this query; no matched baseline/proof result is cached.
                let implementation = unsafe { method_getImplementation(m) } as usize;
                let encoding = unsafe { text(method_getTypeEncoding(m)) }?;
                if encoding.is_empty() {
                    return Err("empty method ABI".into());
                }
                found = Some(ObservedMethod {
                    receiver,
                    dispatch,
                    owner,
                    selector: name.clone(),
                    method: address,
                    implementation,
                    encoding,
                    ancestry: names.clone(),
                });
            }
        }
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}
/// Two complete fresh passes. Images are observed once per actual image in
/// each pass, only AFTER every actual IMP and method field matches its pin.
/// No state crosses this call, callback, metadata write or business operation.
/// This is not atomic image-unload/ABA authority: the caller's existing sealed
/// source/exclusive/no-concurrent-unload contract remains mandatory.
pub(crate) fn verify_current_methods(
    checks: &[(ClassHandle, &CStr, bool, &MethodSnapshot)],
) -> Result<()> {
    if checks.is_empty() || checks.len() > 128 {
        return Err("current method verification batch bound".into());
    }
    let mut keys = BTreeSet::new();
    for (class, selector, meta, _) in checks {
        if !keys.insert((class.address(), selector.to_bytes(), meta)) {
            return Err("duplicate current method verification key".into());
        }
    }
    for _ in 0..2 {
        // Fresh for THIS pass only. No owner inventory/name may survive the
        // second-pass boundary, a callback, metadata write or another call.
        let mut owners = std::collections::BTreeMap::new();
        let mut methods = Vec::with_capacity(checks.len());
        for (class, selector, meta, expected) in checks {
            let dispatch = if *meta {
                (unsafe { object_getClass(class.0 as Class) }) as usize
            } else {
                class.0
            };
            if dispatch == 0 {
                return Err("nil metaclass".into());
            }
            let observed = observe_method_fields(class.0, dispatch, selector, &mut owners)?
                .ok_or("current method observation missing")?;
            if !observed.matches(expected) {
                return Err("current method metadata differs from captured method".into());
            }
            methods.push(observed);
        }
        let mut images = std::collections::BTreeMap::<usize, ObservedCodeImage>::new();
        for (observed, (_, _, _, expected)) in methods.iter().zip(checks) {
            let image = match images.entry(expected.code.base) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let current = observe_code_image(observed.implementation)?;
                    if current.base != expected.code.base {
                        return Err("current method image base changed".into());
                    }
                    entry.insert(current)
                }
            };
            // Every row derives its OWN selected RX segment and offset. Never
            // substitute the representative's segment for another address.
            if image.identity(observed.implementation)? != expected.code {
                return Err("current method complete code image identity changed".into());
            }
        }
    }
    Ok(())
}

/// A source baseline carries an actual whole-six capture from this call. Clones
/// share detected inherited-proof invalidation; callers cannot reconstruct it
/// from an old IMP or a receipt. Capture must precede ALL ancestor writes.
pub fn method(
    class: ClassHandle,
    selector: &CStr,
    class_method: bool,
) -> Result<Option<MethodSnapshot>> {
    let Some(mut snapshot) = raw_method(class, selector, class_method)? else {
        return Ok(None);
    };
    let whole = lifecycle(class)?;
    if raw_method(class, selector, class_method)?.as_ref() != Some(&snapshot) {
        return Err("source method changed during before capture".into());
    }
    snapshot.source_capture = Some(Arc::new(SourceCapture {
        whole,
        class_method,
        retired: AtomicBool::new(false),
    }));
    Ok(Some(snapshot))
}
#[derive(Debug)]
struct SourceCapture {
    whole: LifecycleSnapshot,
    class_method: bool,
    retired: AtomicBool,
}
/// Includes exact declaring ancestor, selected IMP and actual code-image identity
/// for initializer/resolver/C++/deallocation leaves, including explicit absence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleSnapshot(Vec<(&'static CStr, bool, Option<MethodSnapshot>)>);
pub fn lifecycle(class: ClassHandle) -> Result<LifecycleSnapshot> {
    let mut leaves = vec![];
    for (sel, meta) in [
        (c"initialize", true),
        (c"resolveInstanceMethod:", true),
        (c"resolveClassMethod:", true),
        (c".cxx_construct", false),
        (c".cxx_destruct", false),
        (c"dealloc", false),
    ] {
        leaves.push((sel, meta, raw_method(class, sel, meta)?))
    }
    Ok(LifecycleSnapshot(leaves))
}
/// Explicit source-owner plan for one known lifecycle leaf. Encoding is the
/// actual before ABI (or explicit source-reviewed ABI for a planned absence).
/// Metadata agreement is not constructor/source execution authority.
#[derive(Clone, Debug)]
pub struct LifecycleChange {
    pub selector: &'static CStr,
    pub class_method: bool,
    pub replacement: usize,
    pub encoding: Vec<u8>,
}
#[derive(Clone, Debug)]
struct CapturedLifecycleChange {
    change: LifecycleChange,
    before: Option<MethodSnapshot>,
    code: CodeIdentity,
}
/// Single-use actual before snapshot. No receipt can supply its fields.
/// The source owner must serialize these writes before class use, after parent
/// transactions finish. This is no runtime mutation lock or initializer grant.
pub struct LifecycleBatch {
    class: ClassHandle,
    before: LifecycleSnapshot,
    changes: Vec<CapturedLifecycleChange>,
}
#[derive(Debug)]
struct LifecycleBatchEvidence {
    class: ClassHandle,
    before: LifecycleSnapshot,
    final_snapshot: LifecycleSnapshot,
    changes: Vec<CapturedLifecycleChange>,
    retired: AtomicBool,
}
/// Immutable final metadata shared by all proofs of this exact transaction.
/// Once an actual mismatch is observed, restoring an IMP cannot rearm it.
#[derive(Clone, Debug)]
pub struct LifecycleBatchProof(Arc<LifecycleBatchEvidence>);
impl LifecycleBatch {
    pub fn capture(class: ClassHandle, changes: Vec<LifecycleChange>) -> Result<Self> {
        registered_class(class.address())?;
        if changes.is_empty() || changes.len() > 6 {
            return Err("invalid lifecycle batch bound".into());
        }
        let before = lifecycle(class)?;
        let mut keys = BTreeSet::new();
        let mut captured = vec![];
        for change in changes {
            if !keys.insert((change.selector.to_bytes(), change.class_method))
                || change.replacement == 0
                || change.encoding.is_empty()
                || change.encoding.contains(&0)
            {
                return Err("invalid/duplicate lifecycle batch leaf".into());
            }
            let prior = before
                .0
                .iter()
                .find(|(selector, meta, _)| {
                    *meta == change.class_method
                        && selector.to_bytes() == change.selector.to_bytes()
                })
                .ok_or("unknown lifecycle batch selector/meta")?
                .2
                .clone();
            if let Some(prior) = &prior {
                if prior.encoding != change.encoding || prior.implementation == change.replacement {
                    return Err("lifecycle batch ABI/no-op differs".into());
                }
            }
            let code = code_identity(change.replacement)?;
            captured.push(CapturedLifecycleChange {
                change,
                before: prior,
                code,
            });
        }
        Ok(Self {
            class,
            before,
            changes: captured,
        })
    }
    pub fn finish(self) -> Result<LifecycleBatchProof> {
        let final_snapshot = lifecycle(registered_class(self.class.address())?)?;
        for ((selector, meta, before), (after_selector, after_meta, after)) in
            self.before.0.iter().zip(&final_snapshot.0)
        {
            if selector != after_selector || meta != after_meta {
                return Err("lifecycle batch roster changed".into());
            }
            let planned = self.changes.iter().find(|c| {
                c.change.class_method == *meta
                    && c.change.selector.to_bytes() == selector.to_bytes()
            });
            if let Some(planned) = planned {
                let after = after.as_ref().ok_or("planned lifecycle leaf missing")?;
                if before != &planned.before
                    || after.receiver != self.class.address()
                    || after.owner != after.dispatch
                    || after.implementation != planned.change.replacement
                    || after.encoding != planned.change.encoding
                    || after.code != planned.code
                {
                    return Err("planned lifecycle metadata/ABI/code differs".into());
                }
            } else if before != after {
                return Err("undeclared lifecycle batch delta".into());
            }
        }
        let proof = LifecycleBatchProof(Arc::new(LifecycleBatchEvidence {
            class: self.class,
            before: self.before,
            final_snapshot,
            changes: self.changes,
            retired: AtomicBool::new(false),
        }));
        proof.verify()?;
        Ok(proof)
    }
}
impl LifecycleBatchProof {
    pub fn verify(&self) -> Result<()> {
        with_verification_roster(|roster| self.verify_with_roster(roster))
    }
    pub(crate) fn verify_with_roster(&self, roster: &VerificationRoster) -> Result<()> {
        if self.0.retired.load(Ordering::Acquire) {
            return Err("lifecycle batch permanently retired".into());
        }
        let matches = roster
            .registered_class(self.0.class.address())
            .and_then(|class| roster.lifecycle_matches(class, &self.0.final_snapshot));
        if matches != Ok(true) {
            self.0.retired.store(true, Ordering::Release);
            return Err("lifecycle batch actual metadata changed".into());
        }
        if self.0.retired.load(Ordering::Acquire) {
            return Err("lifecycle batch retired during verification".into());
        }
        Ok(())
    }
}
fn same_method_leaf(a: &MethodSnapshot, b: &MethodSnapshot) -> bool {
    a.owner == b.owner
        && a.selector == b.selector
        && a.method == b.method
        && a.implementation == b.implementation
        && a.encoding == b.encoding
        && a.code == b.code
}
// The child before-image must project the ancestor's ORIGINAL whole-six,
// including leaves whose parent replacement happened before this selector.
// A child-local original override is distinct and remains subject to the later
// before-to-final check; inherited leaves cannot use a mixed parent transaction.
fn original_lifecycle_projection(
    child: &LifecycleSnapshot,
    parent: &LifecycleSnapshot,
    receiver: ClassHandle,
) -> bool {
    child.0.len() == 6
        && parent.0.len() == 6
        && child.0.iter().zip(&parent.0).all(
            |((selector, meta, original), (parent_selector, parent_meta, ancestor))| {
                if selector != parent_selector || meta != parent_meta {
                    return false;
                }
                let dispatch = if *meta {
                    (unsafe { object_getClass(receiver.address() as Class) }) as usize
                } else {
                    receiver.address()
                };
                match (original, ancestor) {
                    (Some(original), _) if original.owner == dispatch => true,
                    (Some(original), Some(ancestor)) => same_method_leaf(original, ancestor),
                    (None, None) => true,
                    _ => false,
                }
            },
        )
}
fn lifecycle_from_ancestor(
    before: &LifecycleSnapshot,
    after: &LifecycleSnapshot,
    ancestor: &LifecycleSnapshot,
    receiver: ClassHandle,
) -> Result<bool> {
    if before.0.len() != 6 || after.0.len() != 6 || ancestor.0.len() != 6 {
        return Ok(false);
    }
    for (((selector, meta, old), (next, next_meta, new)), (parent, parent_meta, parent_method)) in
        before.0.iter().zip(&after.0).zip(&ancestor.0)
    {
        if selector != next || selector != parent || meta != next_meta || meta != parent_meta {
            return Ok(false);
        }
        if old == new {
            continue;
        }
        let (Some(new), Some(parent)) = (new, parent_method) else {
            return Ok(false);
        };
        let dispatch = if *meta {
            (unsafe { object_getClass(receiver.address() as Class) }) as usize
        } else {
            receiver.address()
        };
        if new.owner == dispatch || !same_method_leaf(new, parent) {
            return Ok(false);
        }
    }
    Ok(true)
}
#[derive(Clone, Debug)]
pub struct ReplacementProvenance {
    baseline: MethodSnapshot,
    before_write: MethodSnapshot,
    selected: MethodSnapshot,
    lifecycle: LifecycleSnapshot,
    inherited_selected_from: Option<Box<MethodSnapshot>>,
    class_method: bool,
    batch: Option<LifecycleBatchProof>,
    ancestor_batches: Vec<LifecycleBatchProof>,
    inherited_prior: Option<Box<Self>>,
    inherited_capture: Option<Arc<SourceCapture>>,
}
impl ReplacementProvenance {
    pub fn matches_key(&self, class: ClassHandle, selector: &CStr, class_method: bool) -> bool {
        self.selected.receiver == class.address()
            && self.selected.selector == selector.to_bytes()
            && self.class_method == class_method
    }
    pub fn bind(
        baseline: MethodSnapshot,
        before_write: MethodSnapshot,
        selected: MethodSnapshot,
        lifecycle_before: LifecycleSnapshot,
        class_method: bool,
        prior: Option<&Self>,
    ) -> Result<Self> {
        Self::bind_selected(
            baseline,
            before_write,
            selected,
            lifecycle_before,
            class_method,
            prior,
            false,
        )
    }
    /// Bind exactly one deliberate lifecycle-selector replacement. This is not
    /// initializer authority: code/source/factory admission remains separate.
    /// Every other lifecycle leaf must remain identical to its before snapshot.
    pub fn bind_lifecycle(
        baseline: MethodSnapshot,
        before_write: MethodSnapshot,
        selected: MethodSnapshot,
        lifecycle_before: LifecycleSnapshot,
        class_method: bool,
        prior: Option<&Self>,
    ) -> Result<Self> {
        Self::bind_selected(
            baseline,
            before_write,
            selected,
            lifecycle_before,
            class_method,
            prior,
            true,
        )
    }
    /// Bind only after all planned writes completed. Existing baseline/original
    /// and selected-ancestor validation is unchanged; every bound proof pins
    /// this transaction's same immutable final whole-lifecycle snapshot.
    pub fn bind_in_batch(
        baseline: MethodSnapshot,
        before_write: MethodSnapshot,
        selected: MethodSnapshot,
        class_method: bool,
        prior: Option<&Self>,
        batch: &LifecycleBatchProof,
    ) -> Result<Self> {
        batch.verify()?;
        if selected.receiver != batch.0.class.address() {
            return Err("foreign lifecycle batch class".into());
        }
        if let Some((_, _, before)) = batch.0.before.0.iter().find(|(selector, meta, _)| {
            *meta == class_method && selector.to_bytes() == selected.selector
        }) {
            let planned = batch
                .0
                .changes
                .iter()
                .find(|c| {
                    c.change.class_method == class_method
                        && c.change.selector.to_bytes() == selected.selector
                })
                .ok_or("lifecycle replacement absent from explicit batch plan")?;
            if before.as_ref() != Some(&before_write)
                || planned.before.as_ref() != Some(&before_write)
            {
                return Err("lifecycle batch before-write differs".into());
            }
            let final_method = batch
                .0
                .final_snapshot
                .0
                .iter()
                .find(|(selector, meta, _)| {
                    *meta == class_method && selector.to_bytes() == selected.selector
                })
                .and_then(|(_, _, m)| m.as_ref());
            if final_method != Some(&selected) {
                return Err("lifecycle batch selected method differs".into());
            }
        }
        let mut proof = Self::bind_selected(
            baseline,
            before_write,
            selected,
            batch.0.final_snapshot.clone(),
            class_method,
            prior,
            false,
        )?;
        proof.batch = Some(batch.clone());
        proof.verify()?;
        Ok(proof)
    }
    /// No mutation or no-op write: inherit the nearest DIRECT superclass's
    /// complete provenance. Source capture must precede every ancestor write.
    /// Optional child batch accounts only for its separate real local changes.
    pub fn inherit(
        baseline: MethodSnapshot,
        selected: MethodSnapshot,
        class_method: bool,
        prior: &Self,
        child_batch: Option<&LifecycleBatchProof>,
    ) -> Result<Self> {
        let capture = baseline
            .source_capture
            .clone()
            .ok_or("inherited baseline lacks actual source capture")?;
        if capture.retired.load(Ordering::Acquire) {
            return Err("inherited source capture permanently retired".into());
        }
        prior.verify()?;
        if class_method != prior.class_method
            || class_method != capture.class_method
            || baseline.receiver != selected.receiver
            || baseline.dispatch != selected.dispatch
            || baseline.selector != selected.selector
            || baseline.encoding != selected.encoding
            || baseline.ancestry != selected.ancestry
            || baseline.implementation == selected.implementation
            || selected.owner == selected.dispatch
            || selected.ancestry.get(1).map(|(a, _)| *a) != Some(prior.selected.dispatch)
            || !same_method_leaf(&baseline, &prior.baseline)
            || !same_method_leaf(&selected, &prior.selected)
        {
            return Err("inherited original/nearest-ancestor/selected provenance differs".into());
        }
        let receiver = registered_class(selected.receiver)?;
        let parent_capture = prior
            .baseline
            .source_capture
            .as_ref()
            .ok_or("ancestor original whole-six capture missing")?;
        if !original_lifecycle_projection(&capture.whole, &parent_capture.whole, receiver) {
            capture.retired.store(true, Ordering::Release);
            return Err("child capture does not precede all ancestor writes".into());
        }
        let selector = CStr::from_bytes_with_nul(&[selected.selector.as_slice(), &[0]].concat())
            .map_err(|_| "inherited selector invalid")?
            .to_owned();
        if raw_method(receiver, &selector, class_method)?.as_ref() != Some(&selected) {
            capture.retired.store(true, Ordering::Release);
            return Err("selected inherited method changed".into());
        }
        let whole = lifecycle(receiver)?;
        let before_local = if let Some(batch) = child_batch {
            batch.verify()?;
            if batch.0.class != receiver || batch.0.final_snapshot != whole {
                capture.retired.store(true, Ordering::Release);
                return Err("inherited child batch differs".into());
            }
            &batch.0.before
        } else {
            &whole
        };
        if !lifecycle_from_ancestor(&capture.whole, before_local, &prior.lifecycle, receiver)? {
            capture.retired.store(true, Ordering::Release);
            return Err("undeclared inherited child lifecycle change".into());
        }
        let proof = Self {
            baseline,
            before_write: selected.clone(),
            selected,
            lifecycle: whole,
            inherited_selected_from: Some(Box::new(prior.selected.clone())),
            class_method,
            batch: child_batch.cloned(),
            ancestor_batches: vec![],
            inherited_prior: Some(Box::new(prior.clone())),
            inherited_capture: Some(capture),
        };
        proof.verify()?;
        Ok(proof)
    }
    fn bind_selected(
        baseline: MethodSnapshot,
        before_write: MethodSnapshot,
        selected: MethodSnapshot,
        mut lifecycle_before: LifecycleSnapshot,
        class_method: bool,
        prior: Option<&Self>,
        lifecycle_replacement: bool,
    ) -> Result<Self> {
        if baseline.receiver != before_write.receiver
            || baseline.receiver != selected.receiver
            || baseline.dispatch != before_write.dispatch
            || baseline.dispatch != selected.dispatch
            || baseline.selector != before_write.selector
            || baseline.selector != selected.selector
            || baseline.encoding != before_write.encoding
            || baseline.encoding != selected.encoding
            || selected.owner != selected.dispatch
            || baseline.implementation == selected.implementation
        {
            return Err("replacement metadata/ABI differs".into());
        }
        let inherited_selected_from = if baseline != before_write {
            let prior =
                prior.ok_or("changed inherited method lacks selected ancestor provenance")?;
            prior.verify()?;
            if !before_write
                .ancestry
                .iter()
                .skip(1)
                .any(|(c, _)| *c == prior.selected.owner)
                || before_write.owner != prior.selected.owner
                || before_write.selector != prior.selected.selector
                || before_write.implementation != prior.selected.implementation
                || before_write.code != prior.selected.code
                || before_write.encoding != prior.selected.encoding
            {
                return Err("inherited selected ancestor differs".into());
            }
            Some(Box::new(prior.selected.clone()))
        } else {
            if prior.is_some() {
                return Err("unexpected ancestor provenance".into());
            }
            None
        };
        if lifecycle_replacement {
            let mut replaced = 0;
            for (selector, meta, snapshot) in &mut lifecycle_before.0 {
                if selector.to_bytes() == selected.selector && *meta == class_method {
                    if snapshot.as_ref() != Some(&before_write) {
                        return Err("lifecycle before-write provenance differs".into());
                    }
                    *snapshot = Some(selected.clone());
                    replaced += 1;
                }
            }
            if replaced != 1 {
                return Err("replacement is not exactly one lifecycle selector".into());
            }
        }
        if lifecycle(registered_class(baseline.receiver)?)? != lifecycle_before {
            return Err("initializer/resolver/C++/dealloc metadata changed".into());
        }
        let result = Self {
            baseline,
            before_write,
            selected,
            lifecycle: lifecycle_before,
            inherited_selected_from,
            class_method,
            batch: None,
            // A real child-local replacement still depends on the FULL nearest
            // inherited ancestor, including its sticky source-capture chain.
            inherited_prior: prior.map(|p| Box::new(p.clone())),
            inherited_capture: None,
            ancestor_batches: prior
                .map(|p| {
                    let mut batches = p.ancestor_batches.clone();
                    if let Some(batch) = &p.batch {
                        batches.push(batch.clone());
                    }
                    batches
                })
                .unwrap_or_default(),
        };
        result.verify()?;
        Ok(result)
    }
    pub fn verify(&self) -> Result<usize> {
        with_verification_roster(|roster| self.verify_with_roster(roster))
    }
    pub(crate) fn verify_with_roster(&self, roster: &VerificationRoster) -> Result<usize> {
        let result = (|| {
            if self
                .inherited_capture
                .as_ref()
                .is_some_and(|c| c.retired.load(Ordering::Acquire))
            {
                return Err("inherited source capture permanently retired".into());
            }
            if let Some(prior) = &self.inherited_prior {
                prior.verify_with_roster(roster)?;
            }
            for ancestor in &self.ancestor_batches {
                ancestor.verify_with_roster(roster)?;
            }
            if let Some(batch) = &self.batch {
                batch.verify_with_roster(roster)?;
            }
            let selected = self.verify_selected_with_roster(roster)?;
            if self
                .inherited_capture
                .as_ref()
                .is_some_and(|c| c.retired.load(Ordering::Acquire))
            {
                return Err("inherited source capture retired during verification".into());
            }
            if self
                .batch
                .as_ref()
                .is_some_and(|b| b.0.retired.load(Ordering::Acquire))
                || self
                    .ancestor_batches
                    .iter()
                    .any(|b| b.0.retired.load(Ordering::Acquire))
            {
                return Err("lifecycle batch retired during selected verification".into());
            }
            Ok(selected)
        })();
        if result.is_err() {
            if let Some(capture) = &self.inherited_capture {
                capture.retired.store(true, Ordering::Release);
            }
            if let Some(batch) = &self.batch {
                batch.0.retired.store(true, Ordering::Release);
            }
        }
        result
    }
    fn verify_selected_with_roster(&self, roster: &VerificationRoster) -> Result<usize> {
        let selector =
            CStr::from_bytes_with_nul(&[self.selected.selector.as_slice(), &[0]].concat())
                .map_err(|_| "invalid stored selector")?
                .to_owned();
        let receiver = roster.registered_class(self.selected.receiver)?;
        let now = raw_method(receiver, &selector, self.class_method)?
            .ok_or("selected metadata missing")?;
        if now != self.selected || !roster.lifecycle_matches(receiver, &self.lifecycle)? {
            let class = class_name(receiver)?;
            let kind = if self.class_method { '+' } else { '-' };
            let changed = if now != self.selected { "method" } else { "lifecycle" };
            return Err(format!("selected provenance changed ({changed}) {kind}[{} {}]",
                String::from_utf8_lossy(&class), selector.to_string_lossy()));
        }
        if self.baseline.implementation == self.selected.implementation
            || self.before_write.implementation == 0
        {
            return Err("invalid original replacement provenance".into());
        }
        if let Some(ancestor) = &self.inherited_selected_from {
            let actual = raw_method(
                roster.registered_class(ancestor.receiver)?,
                &selector,
                self.class_method,
            )?
            .ok_or("selected ancestor missing")?;
            if actual != **ancestor {
                return Err("selected ancestor retired/changed".into());
            }
        }
        Ok(now.implementation)
    }
    pub fn original(&self) -> usize {
        self.baseline.implementation
    }
    pub fn replacement(&self) -> usize {
        self.selected.implementation
    }
    pub fn declaring_owner(&self) -> usize {
        self.baseline.owner
    }
    pub fn selected_owner(&self) -> usize {
        self.selected.owner
    }
    pub fn inherited_selected_owner(&self) -> Option<usize> {
        self.inherited_selected_from.as_ref().map(|s| s.owner)
    }
}

/// Fixed NSObject.init machine contract, observed without calling it. DATA for
/// the class-local source issuer; no initializer/ARC permission or leaf waiver.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub struct ObjectRootInitializer {
    class:ClassHandle,method:MethodSnapshot,words:Vec<(usize,u32,CodeIdentity)>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ObjectRootInitializer {
    fn instruction(address:usize)->Result<(u32,CodeIdentity)> {
        let code=code_identity(address)?;
        if !address.is_multiple_of(4)||address.checked_add(4).is_none_or(|end|end>code.segment_end) {
            return Err("NSObject init instruction range differs".into());
        }
        let word=unsafe{(address as *const u32).read_volatile()};
        if code_identity(address)?!=code {return Err("NSObject init code image changed during read".into());}
        Ok((word,code))
    }
    pub fn observe()->Result<Self> {
        let class=loaded_class(c"NSObject")?;
        let method=method(class,c"init",false)?.ok_or("NSObject init absent")?;
        for selector in [c".cxx_construct",c".cxx_destruct"] {
            if self::method(class,selector,false)?.is_some(){return Err("NSObject root CXX contract unqualified".into());}
        }
        if method.owner()!=class.address()||crate::workspace::simple_abi(&method)?!=b"@@:"
            ||method.code().image()!=b"/usr/lib/libobjc.A.dylib" {
            return Err("NSObject init owner, ABI or runtime image differs".into());
        }
        let mut address=method.implementation();let mut words=Vec::new();let mut returned=false;
        // Transparent BTI/NOP/direct tails/RET only. None consume self's +1,
        // touch storage, call another routine or replace x0. No fallback.
        for _ in 0..4 {
            if words.iter().any(|(at,_,_)|*at==address) {return Err("NSObject init branch cycle".into());}
            let (word,code)=Self::instruction(address)?;
            if code.image!=method.code().image||code.base!=method.code().base||code.uuid!=method.code().uuid {
                return Err("NSObject init tail leaves exact runtime image".into());
            }
            words.push((address,word,code));
            if word==0xd65f03c0 {returned=true;break;}
            address=if word==0xd503245f||word==0xd503201f {
                address.checked_add(4).ok_or("NSObject init next instruction overflow")?
            } else if word&0xfc000000==0x14000000 {
                let delta=(((word&0x03ffffff) as i32)<<6)>>4;
                address.checked_add_signed(delta as isize).ok_or("NSObject init tail target overflow")?
            } else {return Err("NSObject init same-self nonconsuming contract unqualified".into());};
        }
        if !returned {return Err("NSObject init transparent return exceeds bound".into());}
        let value=Self{class,method,words};value.verify()?;Ok(value)
    }
    pub fn verify(&self)->Result<()> {
        verify_current_method(self.class,c"init",false,&self.method)?;
        for (address,expected,identity) in &self.words {
            let (word,current)=Self::instruction(*address)?;
            if word!=*expected||&current!=identity {return Err("NSObject init transparent body changed".into());}
        }
        verify_current_method(self.class,c"init",false,&self.method)
    }
    pub fn method(&self)->&MethodSnapshot {&self.method}
}

#[cfg(test)]
mod roster_tests {
    #[test]
    #[ignore="explicit observed Notification destroy system cohort; read-only code data, no Swift call"]
    fn observed_notification_destroy_code_data() {
        use super::*;
        let handle=unsafe{libc::dlopen(c"/usr/lib/swift/libswiftCore.dylib".as_ptr(),libc::RTLD_NOW|libc::RTLD_LOCAL)};
        assert!(!handle.is_null());
        let anchor=unsafe{libc::dlsym(handle,c"swift_retain".as_ptr())} as usize;
        assert_ne!(anchor,0);
        let image=code_identity(anchor).unwrap();
        let uuid=[131,121,79,179,222,155,61,35,171,94,44,29,93,48,241,52];
        assert_eq!(image.image(),b"/usr/lib/swift/libswiftCore.dylib");assert_eq!(image.uuid(),uuid);
        let selected=image.base.checked_add(0x52f10).unwrap();
        let code=code_identity(selected).unwrap();assert_eq!(code.uuid(),uuid);
        verify_notification_runtime_destroy(selected,&code).unwrap();
        assert!(verify_notification_runtime_destroy(anchor,&image).is_err());
        assert!(verify_notification_runtime_destroy(selected+4,&code_identity(selected+4).unwrap()).is_err());
        assert!(selected>=code.segment_start&&selected.checked_add(48).unwrap()<=code.segment_end);
        let mut info=DlInfo{name:std::ptr::null(),base:std::ptr::null(),symbol:std::ptr::null(),symbol_address:std::ptr::null()};
        assert_ne!(unsafe{dladdr(selected as *const c_void,&mut info)},0);
        let name=if info.symbol.is_null(){Vec::new()}else{unsafe{text(info.symbol)}.unwrap()};
        let words=(0..12).map(|index|unsafe{((selected+index*4) as *const u32).read_unaligned()}).collect::<Vec<_>>();
        println!("selected_symbol={} symbol_delta={} words={:08x?}",String::from_utf8_lossy(&name),selected.saturating_sub(info.symbol_address as usize),words);
        assert_eq!(code_identity(selected).unwrap(),code);
        unsafe{libc::dlclose(handle)};
        // No witness invocation, opaque value, original receiver or grant.
    }
    use super::*;
    // Exact prior 5b1 algorithm, retained solely as the equivalence oracle.
    unsafe fn previous(p: *const Class, n: u32) -> Result<Vec<ClassHandle>> {
        if p.is_null() || n > 100_000 {
            return Err("loaded class roster invalid".into());
        }
        let mut seen = BTreeSet::new();
        for c in unsafe { std::slice::from_raw_parts(p, n as usize) } {
            if c.is_null() || !seen.insert(*c as usize) {
                return Err("nil/duplicate loaded class".into());
            }
        }
        Ok(seen.into_iter().map(ClassHandle).collect())
    }
    #[test]
    fn complete_roster_equivalence_empty_bounds_nil_duplicates_and_order() {
        let storage = [1usize, 4, usize::MAX];
        let values = [
            ptr::null(),
            (&storage[0] as *const usize).cast(),
            (&storage[1] as *const usize).cast(),
            (&storage[2] as *const usize).cast(),
        ];
        // All 1,365 bounded lists, including empty, every nil position,
        // repeated values and all possible input permutations of these slots.
        for length in 0..=5 {
            for mut bits in 0..4usize.pow(length) {
                let raw: Vec<Class> = (0..length)
                    .map(|_| {
                        let v = values[bits % 4];
                        bits /= 4;
                        v
                    })
                    .collect();
                assert_eq!(
                    unsafe { checked_roster(raw.as_ptr(), raw.len() as u32) },
                    unsafe { previous(raw.as_ptr(), raw.len() as u32) }
                );
            }
        }
        assert_eq!(unsafe { checked_roster(ptr::null(), 0) }, unsafe {
            previous(ptr::null(), 0)
        });
        let unused = ptr::NonNull::<Class>::dangling().as_ptr();
        assert_eq!(unsafe { checked_roster(unused, 100_001) }, unsafe {
            previous(unused, 100_001)
        });
        // Exact accepted upper bound, retaining every unique slot and sorting
        // just as the prior BTreeSet did. No slot is dereferenced as a class.
        let storage: Vec<usize> = (0..100_000).collect();
        let raw: Vec<Class> = storage
            .iter()
            .rev()
            .map(|slot| (slot as *const usize).cast())
            .collect();
        let actual = unsafe { checked_roster(raw.as_ptr(), 100_000) }.unwrap();
        assert_eq!(actual, unsafe { previous(raw.as_ptr(), 100_000) }.unwrap());
        assert_eq!(actual.len(), 100_000);
        assert!(actual.windows(2).all(|p| p[0].0 < p[1].0));
    }
    #[test]
    fn actual_typed_class_membership_matches_the_complete_roster() {
        let class = loaded_class(c"NSObject").unwrap();
        let roster = loaded_classes().unwrap();
        assert!(roster.contains(&class));
        assert_eq!(registered_class(class.address()).unwrap(), class);
        assert_eq!(
            registered_class(0).unwrap_err(),
            "class address is not in actual registered roster"
        );
        assert!(roster.windows(2).all(|p| p[0].0 < p[1].0));
    }
    #[test]
    fn verification_roster_matches_full_membership_names_and_refreshes_next_scope() {
        use objc2::{runtime::ClassBuilder, ClassType};
        use objc2_foundation::NSObject;
        with_verification_roster(|roster| {
            assert!(roster.classes.get().is_none());
            assert!(roster.names.get().is_none());
            let root = loaded_class(c"NSObject")?;
            assert_eq!(
                roster.registered_class(root.address())?,
                registered_class(root.address())?
            );
            assert!(
                roster.names.get().is_none(),
                "membership must not read names early"
            );
            for address in [0, usize::MAX] {
                assert_eq!(roster.registered_class(address), registered_class(address));
            }
            for name in [
                c"NSObject",
                c"NSString",
                c"RuycastF1RosterDefinitelyMissing",
            ] {
                assert_eq!(roster.loaded_class(name), loaded_class(name));
                assert_eq!(roster.loaded_class(name), loaded_class(name));
            }
            assert!(roster.loaded_class(c"NSObjecT").is_err());
            assert!(roster.loaded_class(c"RuycastF1RosterNextScope").is_err());
            Ok(())
        })
        .unwrap();
        // Registration occurs only AFTER that observation has been dropped.
        // A new standalone scope must not retain its old class/name roster.
        let new = ClassBuilder::new(c"RuycastF1RosterNextScope", NSObject::class())
            .unwrap()
            .register();
        with_verification_roster(|roster| {
            let actual = roster.loaded_class(c"RuycastF1RosterNextScope")?;
            assert_eq!(actual.address(), new as *const _ as usize);
            assert_eq!(roster.registered_class(actual.address())?, actual);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn verification_roster_name_refusals_preserve_original_scan_order() {
        // Pure malformed-observation fixtures, never used for a grant, a class
        // write or method/source authority. Every handle itself is genuinely
        // in the complete current runtime roster; only observed name results
        // are poisoned to compare the original loop's refusal precedence.
        let classes = loaded_classes().unwrap();
        assert!(classes.len() >= 3);
        let old = |names: &[Result<Vec<u8>>], query: &CStr| -> Result<ClassHandle> {
            let mut found = None;
            for (&class, observed) in classes.iter().zip(names) {
                if observed.as_ref().map_err(Clone::clone)? == query.to_bytes()
                    && found.replace(class).is_some()
                {
                    return Err("duplicate class name".into());
                }
            }
            found.ok_or_else(|| "attested loaded class missing".into())
        };
        for early_error in [false, true] {
            let mut names = vec![Ok(b"other".to_vec()); classes.len()];
            names[0] = Ok(b"duplicate".to_vec());
            names[1] = if early_error {
                Err("null runtime metadata string".into())
            } else {
                Ok(b"duplicate".to_vec())
            };
            names[2] = Err("runtime metadata string exceeds bound".into());
            with_verification_roster(|roster| {
                roster.classes.set(Ok(classes.clone())).unwrap();
                roster
                    .names
                    .set(
                        names
                            .iter()
                            .map(|result| result.as_ref().map(|name| roster.copy_name(name)).map_err(Clone::clone))
                            .map(std::cell::OnceCell::from)
                            .collect(),
                    )
                    .unwrap();
                for query in [c"duplicate", c"missing", c"other"] {
                    assert_eq!(roster.loaded_class(query), old(&names, query));
                    assert_eq!(roster.loaded_class(query), old(&names, query));
                }
                Ok(())
            })
            .unwrap();
        }
        with_verification_roster(|roster| {
            roster
                .classes
                .set(Err("nil/duplicate loaded class".into()))
                .unwrap();
            assert_eq!(
                roster.registered_class(0).unwrap_err(),
                "nil/duplicate loaded class"
            );
            assert_eq!(
                roster.loaded_class(c"NSObject").unwrap_err(),
                "nil/duplicate loaded class"
            );
            assert!(roster.names.get().is_none());
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn verification_roster_lifecycle_compares_every_baseline_and_refreshes_next_scope() {
        use objc2::{
            runtime::{AnyClass, ClassBuilder, Imp, Sel},
            ClassType,
        };
        use objc2_foundation::NSObject;
        unsafe extern "C" fn first(_: *const AnyClass, _: Sel) {
            std::hint::black_box(1usize);
        }
        unsafe extern "C" fn second(_: *const AnyClass, _: Sel) {
            std::hint::black_box(2usize);
        }
        let mut builder =
            ClassBuilder::new(c"RuycastF1LifecycleObservation", NSObject::class()).unwrap();
        unsafe {
            builder.add_class_method(
                objc2::sel!(initialize),
                first as unsafe extern "C" fn(*const AnyClass, Sel),
            );
        }
        let class = builder.register();
        let handle = registered_class(class as *const _ as usize).unwrap();
        let expected = lifecycle(handle).unwrap();
        with_verification_roster(|roster| {
            assert!(roster.lifecycle_observations.borrow().is_empty());
            assert!(roster.lifecycle_matches(handle, &expected)?);
            assert_eq!(roster.lifecycle_observations.borrow().len(), 1);
            // Each different proof baseline is compared again; this is no
            // cached proof success and includes every whole-six slot/absence.
            for slot in 0..6 {
                for field in 0..3 {
                    let mut wrong = expected.clone();
                    match field {
                        0 => wrong.0[slot].0 = c"wrongLifecycleSelector",
                        1 => wrong.0[slot].1 = !wrong.0[slot].1,
                        _ => {
                            if let Some(method) = wrong.0[slot].2.as_mut() {
                                method.implementation ^= 1;
                            } else {
                                wrong.0[slot].2 = expected.0[0].2.clone();
                            }
                        }
                    }
                    assert_eq!(
                        roster.lifecycle_matches(handle, &wrong)?,
                        lifecycle(handle)? == wrong
                    );
                    assert!(!roster.lifecycle_matches(handle, &wrong)?);
                    assert!(roster.lifecycle_matches(handle, &expected)?);
                }
            }
            assert_eq!(roster.lifecycle_observations.borrow().len(), 1);
            Ok(())
        })
        .unwrap();
        let selected = expected.0[0].2.as_ref().unwrap();
        let method = unsafe { &*(selected.method_address() as *const objc2::runtime::Method) };
        // No context survives an own metadata write. A new scope must see it.
        let prior = unsafe {
            method.set_implementation(std::mem::transmute::<usize, Imp>(
                second as *const () as usize,
            ))
        };
        assert_eq!(prior as usize, selected.implementation());
        assert_ne!(second as *const () as usize, selected.implementation());
        with_verification_roster(|roster| {
            assert!(!roster.lifecycle_matches(handle, &expected)?);
            assert_eq!(
                roster.lifecycle_matches(handle, &expected)?,
                lifecycle(handle)? == expected
            );
            Ok(())
        })
        .unwrap();
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, Imp>(selected.implementation()));
        }
        with_verification_roster(|roster| {
            assert!(roster.lifecycle_matches(handle, &expected)?);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn verification_roster_lifecycle_errors_remain_errors_not_proof_success() {
        let class = loaded_class(c"NSObject").unwrap();
        let expected = lifecycle(class).unwrap();
        with_verification_roster(|roster| {
            // A pure malformed observation tests error propagation only. It
            // cannot mint a snapshot, a grant or a successful proof.
            roster
                .lifecycle_observations
                .borrow_mut()
                .push((class, Err("own lifecycle observation error".into())));
            for _ in 0..2 {
                assert_eq!(
                    roster.lifecycle_matches(class, &expected).unwrap_err(),
                    "own lifecycle observation error"
                );
            }
            Ok(())
        })
        .unwrap();
        with_verification_roster(|roster| {
            assert!(roster.lifecycle_matches(class, &expected)?);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn batch_owner_inventory_preserves_inheritance_absence_and_fresh_next_pass() {
        use objc2::{
            runtime::{AnyObject, ClassBuilder, Imp, Sel},
            ClassType,
        };
        use objc2_foundation::NSObject;
        unsafe extern "C" fn marker(_: *mut AnyObject, _: Sel) -> usize {
            31
        }
        let mut builder =
            ClassBuilder::new(c"RuycastF1BatchInventoryParent", NSObject::class()).unwrap();
        unsafe {
            builder.add_method(
                objc2::sel!(ownedInventoryMarker),
                marker as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
        }
        let parent = builder.register();
        let child = ClassBuilder::new(c"RuycastF1BatchInventoryChild", parent)
            .unwrap()
            .register();
        let handle = registered_class(child as *const _ as usize).unwrap();
        let expected = raw_method(handle, c"ownedInventoryMarker", false)
            .unwrap()
            .unwrap();
        assert_eq!(expected.owner, parent as *const _ as usize);
        let mut pass = std::collections::BTreeMap::new();
        for _ in 0..2 {
            let actual =
                observe_method_fields(handle.0, handle.0, c"ownedInventoryMarker", &mut pass)
                    .unwrap()
                    .unwrap();
            assert!(actual.matches(&expected));
            assert!(
                observe_method_fields(handle.0, handle.0, c"ownedInventoryMissing", &mut pass)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(pass.contains_key(&(parent as *const _ as usize)));
        assert!(pass.contains_key(&handle.0));
        drop(pass); // NO pass observation may cross this own metadata write.
        unsafe extern "C" {
            fn class_replaceMethod(
                class: *const objc2::runtime::AnyClass,
                selector: Sel,
                implementation: Imp,
                encoding: *const std::ffi::c_char,
            ) -> Option<Imp>;
        }
        let encoding = std::ffi::CString::new(expected.encoding()).unwrap();
        unsafe {
            class_replaceMethod(
                child,
                objc2::sel!(ownedInventoryMarker),
                std::mem::transmute::<usize, Imp>(expected.implementation()),
                encoding.as_ptr(),
            );
        }
        let mut next_pass = std::collections::BTreeMap::new();
        let actual =
            observe_method_fields(handle.0, handle.0, c"ownedInventoryMarker", &mut next_pass)
                .unwrap()
                .unwrap();
        assert_eq!(actual.owner, handle.0); // New closest owner, even with SAME IMP.
        assert!(!actual.matches(&expected));
        let changed = raw_method(handle, c"ownedInventoryMarker", false)
            .unwrap()
            .unwrap();
        assert!(actual.matches(&changed));
        assert!(
            verify_current_methods(&[(handle, c"ownedInventoryMarker", false, &expected)]).is_err()
        );
        verify_current_methods(&[(handle, c"ownedInventoryMarker", false, &changed)]).unwrap();
    }
    #[test]
    fn batch_inventory_refusal_order_keeps_duplicate_and_selected_abi_before_later_slots() {
        use objc2::{
            runtime::{AnyObject, ClassBuilder, Sel},
            ClassType,
        };
        use objc2_foundation::NSObject;
        unsafe extern "C" fn marker(_: *mut AnyObject, _: Sel) -> usize {
            32
        }
        let mut builder =
            ClassBuilder::new(c"RuycastF1BatchInventoryOrder", NSObject::class()).unwrap();
        unsafe {
            builder.add_method(
                objc2::sel!(ownedInventoryOrder),
                marker as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
        }
        let class = builder.register();
        let handle = registered_class(class as *const _ as usize).unwrap();
        let selected = raw_method(handle, c"ownedInventoryOrder", false)
            .unwrap()
            .unwrap();
        let names = (0..3)
            .map(|_| std::cell::OnceCell::new())
            .collect::<Vec<_>>();
        names[2]
            .set(Err("later invalid selector name".into()))
            .unwrap();
        let mut pass = std::collections::BTreeMap::new();
        // These malformed PRIVATE observations use a genuine own Method only;
        // no instance, snapshot/capture or constructor grant is fabricated.
        pass.insert(
            handle.0,
            Ok(DeclaredMethods {
                addresses: vec![selected.method_address(); 3],
                names,
            }),
        );
        let error = observe_method_fields(handle.0, handle.0, c"ownedInventoryOrder", &mut pass)
            .err()
            .unwrap();
        assert_eq!(error, "duplicate declared selector");
        assert_eq!(
            pass[&handle.0].as_ref().unwrap().names[2]
                .get()
                .unwrap()
                .as_ref()
                .unwrap_err(),
            "later invalid selector name"
        );
        drop(pass);
        let empty = ClassBuilder::new(c"RuycastF1BatchInventoryEmptyAbi", NSObject::class())
            .unwrap()
            .register();
        unsafe extern "C" {
            fn class_addMethod(
                class: *const std::ffi::c_void,
                selector: *const std::ffi::c_void,
                implementation: *const std::ffi::c_void,
                types: *const std::ffi::c_char,
            ) -> u8;
            fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
        }
        // An own, NEVER INVOKED runtime leaf gives an actual empty-ABI witness.
        assert!(unsafe {
            class_addMethod(
                (empty as *const objc2::runtime::AnyClass).cast(),
                sel_registerName(c"ownedInventoryEmptyAbi".as_ptr()),
                marker as *const () as *const std::ffi::c_void,
                c"".as_ptr(),
            ) != 0
        });
        let address = empty as *const _ as usize;
        let declared = DeclaredMethods::observe(address).unwrap();
        assert_eq!(declared.addresses.len(), 1);
        let raw = declared.addresses[0];
        let mut pass = std::collections::BTreeMap::new();
        pass.insert(
            address,
            Ok(DeclaredMethods {
                addresses: vec![raw, 0],
                names: (0..2).map(|_| std::cell::OnceCell::new()).collect(),
            }),
        );
        assert_eq!(
            observe_method_fields(address, address, c"ownedInventoryEmptyAbi", &mut pass)
                .err()
                .unwrap(),
            "empty method ABI"
        );
        assert!(
            pass[&address].as_ref().unwrap().names[1].get().is_none(),
            "later nil slot must not be inspected early"
        );
        assert_eq!(
            raw_method(
                registered_class(address).unwrap(),
                c"ownedInventoryEmptyAbi",
                false
            )
            .err()
            .unwrap(),
            "empty method ABI"
        );
    }
    #[test]
    fn current_observation_matches_full_snapshot_without_minting_source_authority() {
        use objc2::{
            runtime::{AnyObject, ClassBuilder, Imp, Sel},
            ClassType,
        };
        use objc2_foundation::NSObject;
        unsafe extern "C" fn first(_: *mut AnyObject, _: Sel) -> usize {
            1
        }
        unsafe extern "C" fn second(_: *mut AnyObject, _: Sel) -> usize {
            2
        }
        let mut builder =
            ClassBuilder::new(c"RuycastF1ObserveMethodParent", NSObject::class()).unwrap();
        unsafe {
            builder.add_method(
                objc2::sel!(ownedObservationMarker),
                first as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
        }
        let parent = builder.register();
        let child = ClassBuilder::new(c"RuycastF1ObserveMethodChild", parent)
            .unwrap()
            .register();
        let other = ClassBuilder::new(c"RuycastF1ObserveMethodOther", NSObject::class())
            .unwrap()
            .register();
        let handle = registered_class(child as *const _ as usize).unwrap();
        let expected = method(handle, c"ownedObservationMarker", false)
            .unwrap()
            .unwrap();
        let agrees = |candidate: &MethodSnapshot| {
            let old = method(handle, c"ownedObservationMarker", false)
                .unwrap()
                .as_ref()
                == Some(candidate);
            let new =
                verify_current_method(handle, c"ownedObservationMarker", false, candidate).is_ok();
            let batch =
                verify_current_methods(&[(handle, c"ownedObservationMarker", false, candidate)])
                    .is_ok();
            assert_eq!(old, new);
            assert_eq!(old, batch);
            old
        };
        assert!(agrees(&expected));
        let raw = raw_method(handle, c"ownedObservationMarker", false)
            .unwrap()
            .unwrap();
        assert!(raw.source_capture.is_none());
        assert!(expected.source_capture.is_some());
        assert!(agrees(&raw)); // equality is not source/constructor authority
        for field in 0..15 {
            let mut changed = expected.clone();
            match field {
                0 => changed.receiver ^= 1,
                1 => changed.dispatch ^= 1,
                2 => changed.owner ^= 1,
                3 => changed.selector.push(b'x'),
                4 => changed.method ^= 1,
                5 => changed.implementation ^= 1,
                6 => changed.encoding.push(b'x'),
                7 => changed.ancestry[0].0 ^= 1,
                8 => changed.ancestry[0].1.push(b'x'),
                9 => changed.code.image.push(b'x'),
                10 => changed.code.base ^= 1,
                11 => changed.code.offset ^= 1,
                12 => changed.code.uuid[0] ^= 1,
                13 => changed.code.segment_start ^= 1,
                _ => changed.code.segment_end ^= 1,
            }
            assert!(!agrees(&changed), "full field {field} must remain compared");
        }
        assert!(verify_current_method(handle, c"ownedMissingMarker", false, &expected).is_err());
        assert!(method(handle, c"ownedMissingMarker", false)
            .unwrap()
            .is_none());
        assert!(verify_current_method(handle, c"ownedObservationMarker", true, &expected).is_err());
        assert!(verify_current_method(
            registered_class(other as *const _ as usize).unwrap(),
            c"ownedObservationMarker",
            false,
            &expected
        )
        .is_err());
        let selected_method =
            unsafe { &*(expected.method_address() as *const objc2::runtime::Method) };
        let prior = unsafe {
            selected_method.set_implementation(std::mem::transmute::<usize, Imp>(
                second as *const () as usize,
            ))
        };
        assert_eq!(prior as usize, expected.implementation());
        assert!(!agrees(&expected));
        unsafe {
            selected_method
                .set_implementation(std::mem::transmute::<usize, Imp>(expected.implementation()));
        }
        assert!(agrees(&expected)); // observations themselves are not sticky grants
                                    // Same IMP and ABI in a NEW declaring owner is still a different method.
        unsafe extern "C" {
            fn class_replaceMethod(
                class: *const objc2::runtime::AnyClass,
                selector: Sel,
                implementation: Imp,
                encoding: *const std::ffi::c_char,
            ) -> Option<Imp>;
        }
        let encoding = std::ffi::CString::new(expected.encoding()).unwrap();
        unsafe {
            class_replaceMethod(
                child,
                objc2::sel!(ownedObservationMarker),
                std::mem::transmute::<usize, Imp>(expected.implementation()),
                encoding.as_ptr(),
            );
        }
        assert!(!agrees(&expected));
    }
    #[test]
    fn current_batch_images_are_fresh_complete_and_select_each_address_range() {
        use objc2::{
            runtime::{AnyObject, Sel},
            ClassType,
        };
        use objc2_foundation::NSObject;
        unsafe extern "C" fn local(_: *mut AnyObject, _: Sel) -> usize {
            3
        }
        let root = registered_class(NSObject::class() as *const _ as usize).unwrap();
        let retained = method(root, c"retain", false).unwrap().unwrap();
        let local_address = local as *const () as usize;
        for address in [local_address, retained.implementation()] {
            let old = code_identity(address).unwrap();
            let observed = observe_code_image(address).unwrap();
            assert_eq!(observed.identity(address).unwrap(), old);
            assert!(observed.identity(0).is_err());
            let mut changed = observed.identity(address).unwrap();
            changed.uuid[0] ^= 1;
            assert_ne!(observed.identity(address).unwrap(), changed);
            let mut ambiguous = ObservedCodeImage {
                image: observed.image.clone(),
                base: observed.base,
                uuid: observed.uuid,
                executable: observed.executable.clone(),
            };
            ambiguous
                .executable
                .push((old.segment_start, old.segment_end));
            assert!(ambiguous.identity(address).is_err());
            // The same observed image can have distinct RX ranges. Every
            // address must select its own range, never the representative's.
            let ranges = ObservedCodeImage {
                image: observed.image,
                base: 100,
                uuid: observed.uuid,
                executable: vec![(100, 110), (200, 220)],
            };
            let second = ranges.identity(205).unwrap();
            assert_eq!(
                (second.offset, second.segment_start, second.segment_end),
                (105, 200, 220)
            );
            assert!(ranges.identity(150).is_err());
        }
        assert!(verify_current_methods(&[]).is_err());
        let one = [(root, c"retain", false, &retained)];
        verify_current_methods(&one).unwrap();
        assert!(verify_current_methods(&[one[0], one[0]]).is_err());
        assert!(verify_current_methods(&vec![one[0]; 129]).is_err());
    }
}

// Fixed Foundation Notification metadata-accessor DATA observation. This calls
// no Swift entry and creates no metadata, value, callback or ARC permission.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub(crate) enum NotificationMetadataAddressPoint {Symbol,AfterValueWitness}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub(crate) struct FoundationNotificationAccessorReturn {
    address:usize,vn:usize,returned:usize,words:[u32;4],code:CodeIdentity,
    point:NotificationMetadataAddressPoint,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl FoundationNotificationAccessorReturn {
    pub(crate) fn observe()->Result<Self> {
        if crate::sdk::process_fork::is_child(){unsafe{libc::_exit(78)}}
        #[link(name="Foundation",kind="framework")]
        unsafe extern "C" {
            #[link_name="$s10Foundation12NotificationVMa"] fn NOTIFICATION_MA_LABEL();
            #[link_name="$s10Foundation12NotificationVN"] static NOTIFICATION_VN_LABEL:u8;
        }
        let address=NOTIFICATION_MA_LABEL as *const () as usize;
        let vn=std::ptr::addr_of!(NOTIFICATION_VN_LABEL) as usize;
        let code=code_identity(address)?;
        if code.image()!=b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation" {
            return Err("Notification metadata accessor has another system provider".into());
        }
        let mut info=DlInfo{name:ptr::null(),base:ptr::null(),symbol:ptr::null(),symbol_address:ptr::null()};
        if unsafe{dladdr(address as *const c_void,&mut info)}==0||info.base as usize!=code.base
            ||info.symbol_address as usize!=address||unsafe{text(info.symbol)}?!=b"$s10Foundation12NotificationVMa" {
            return Err("Notification accessor is not the exact named Foundation entry".into());
        }
        let mut words=[0u32;4];
        for (index,word) in words.iter_mut().enumerate() {
            let at=address.checked_add(index*4).ok_or("Notification accessor code range overflow")?;
            let (observed,member)=ObjectRootInitializer::instruction(at)?;
            if member.base!=code.base||member.uuid!=code.uuid||member.image!=code.image {
                return Err("Notification accessor leaves its actual system image".into());
            }
            *word=observed;
        }
        // The only supported case is a genuine four-instruction complete
        // constant MetadataResponse. Lazy caches/calls/branches are NOT inferred.
        if words[0]&0x9f00001f!=0x90000000||words[1]&0xff8003ff!=0x91000000
            ||!matches!(words[2],0xd2800001|0x52800001|0xaa1f03e1|0x2a1f03e1)||words[3]!=0xd65f03c0 {
            return Err("Actual Notification accessor lacks the finite constant-return proof".into());
        }
        let imm=((words[0]>>29)&3)|(((words[0]>>5)&0x7ffff)<<2);
        let signed=((imm as i32)<<11)>>11;
        let page=(address&!4095usize).checked_add_signed((signed as isize)<<12).ok_or("Notification accessor ADRP overflow")?;
        let shift=if words[1]&(1<<22)!=0 {12}else{0};
        let returned=page.checked_add((((words[1]>>10)&0xfff) as usize)<<shift).ok_or("Notification accessor ADD overflow")?;
        let point=if returned==vn {NotificationMetadataAddressPoint::Symbol}
            else if vn.checked_add(8)==Some(returned){NotificationMetadataAddressPoint::AfterValueWitness}
            else{return Err("Actual Notification accessor does not return its named metadata storage".into());};
        for (index,expected) in words.iter().enumerate() {
            let (current,member)=ObjectRootInitializer::instruction(address+index*4)?;
            if current!=*expected||member.base!=code.base||member.uuid!=code.uuid||member.image!=code.image {
                return Err("Notification accessor code changed during observation".into());
            }
        }
        if code_identity(address)?!=code{return Err("Notification accessor identity changed".into());}
        Ok(Self{address,vn,returned,words,code,point})
    }
    pub(crate) fn point(&self)->&NotificationMetadataAddressPoint {&self.point}
    pub(crate) fn returned_address(&self)->usize {self.returned}
    pub(crate) fn vn_address(&self)->usize {self.vn}
    pub(crate) fn code(&self)->&CodeIdentity {&self.code}
    pub(crate) fn address(&self)->usize {self.address}
    pub(crate) fn verify(&self)->Result<()> {
        if Self::observe()?!=*self{return Err("Notification accessor observation changed".into());}Ok(())
    }
}
