//! Concrete C leaves, no __interpose section. The sole SDK integrator must
//! patch and attest all actual source import cells before endpoint startup.
use super::{
    model::{self, Attribute, Context},
    opaque::{self, Payload, Ptr, Range, Token},
};
use std::{
    ffi::{c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
static CURRENT: Mutex<Option<Arc<Context>>> = Mutex::new(None);
// Negative fast path only. A true value never replaces the context/lease checks.
static PUBLISHED: AtomicBool = AtomicBool::new(false);
pub fn retire(expected: &Arc<Context>) -> std::result::Result<(), String> {
    let mut current = CURRENT.lock().map_err(|_| "AX owner poisoned")?;
    if !current.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
        return Err("stale AX fixture owner".into());
    }
    PUBLISHED.store(false, Ordering::Release);
    expected.retire();
    *current = None;
    Ok(())
}
pub fn install(context: Arc<Context>) -> std::result::Result<(), String> {
    if !context.live() {
        return Err("retired AX fixture".into());
    }
    let mut current = CURRENT.lock().map_err(|_| "AX owner poisoned")?;
    if current.is_some() {
        return Err("AX owner already installed".into());
    }
    opaque::types()?;
    *current = Some(context);
    PUBLISHED.store(true, Ordering::Release);
    Ok(())
}
pub fn replace(expected: &Arc<Context>, next: Arc<Context>) -> std::result::Result<(), String> {
    if !next.live() {
        return Err("retired next AX fixture".into());
    }
    let mut current = CURRENT.lock().map_err(|_| "AX owner poisoned")?;
    if !current.as_ref().is_some_and(|c| Arc::ptr_eq(c, expected)) {
        return Err("stale AX fixture owner".into());
    }
    if Arc::ptr_eq(expected, &next) {
        return Err("same AX fixture owner".into());
    }
    expected.retire();
    *current = Some(next);
    Ok(())
}
fn current() -> model::Result<Arc<Context>> {
    current_with_search(false)
}
fn current_for_search() -> model::Result<Arc<Context>> {
    current_with_search(true)
}
fn current_with_search(search: bool) -> model::Result<Arc<Context>> {
    if !PUBLISHED.load(Ordering::Acquire) {
        return Err(model::DISABLED);
    }
    // Clone before any dynamic guardian. Reentry may retire/replace CURRENT.
    let context=CURRENT.lock().map_err(|_|model::DISABLED)?.clone().ok_or(model::DISABLED)?;
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    if context.reference.as_ref().is_some_and(|binding| {
        binding.owner != std::thread::current().id() && !(search && binding.search_worker())
    }) {
        // An unauthorized caller must not retire the legitimate main owner.
        return Err(model::DISABLED);
    }
    #[cfg(not(all(rcf1_reference_sdk,not(test),target_arch="aarch64")))]
    let _ = search;
    if !context.live()||!PUBLISHED.load(Ordering::Acquire)
        ||!CURRENT.lock().map_err(|_|model::DISABLED)?.as_ref().is_some_and(|current|Arc::ptr_eq(current,&context)){
        return Err(model::DISABLED);
    }
    Ok(context)
}
fn status(f: impl FnOnce() -> model::Result<()>) -> i32 {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or(Err(model::DISABLED))
        .err()
        .unwrap_or(0)
}
fn pointer(f: impl FnOnce() -> model::Result<Token>) -> Ptr {
    catch_unwind(AssertUnwindSafe(f))
        .ok()
        .and_then(std::result::Result::ok)
        .map_or(std::ptr::null_mut(), |t| t.retained())
}
unsafe fn key(p: Ptr) -> model::Result<String> {
    unsafe { opaque::string(p) }.map_err(|_| model::ILLEGAL)
}
unsafe fn output(out: *mut Ptr, f: impl FnOnce() -> model::Result<Token>) -> i32 {
    if out.is_null() {
        return model::ILLEGAL;
    }
    unsafe {
        out.write(std::ptr::null_mut());
    }
    status(|| {
        let token = f()?;
        unsafe {
            out.write(token.retained());
        }
        Ok(())
    })
}
unsafe extern "C" fn trusted() -> u8 {
    catch_unwind(|| current_for_search().is_ok_and(|c| c.trusted()))
        .unwrap_or(false)
        .into()
}
unsafe extern "C" fn application(pid: i32) -> Ptr {
    pointer(|| current_for_search()?.application(pid))
}
unsafe extern "C" fn system() -> Ptr {
    pointer(|| current()?.system())
}
unsafe extern "C" fn element_type() -> usize {
    if current_for_search().is_err() {
        return 0;
    }
    opaque::cached_types().map_or(0, |t| t.0)
}
unsafe extern "C" fn value_type() -> usize {
    if current().is_err() {
        return 0;
    }
    opaque::cached_types().map_or(0, |t| t.1)
}
unsafe extern "C" fn pid(node: Ptr, out: *mut i32) -> i32 {
    if out.is_null() {
        return model::ILLEGAL;
    }
    unsafe {
        out.write(0);
    }
    status(|| {
        let pid = current_for_search()?.pid(node)?;
        unsafe {
            out.write(pid);
        }
        Ok(())
    })
}
unsafe extern "C" fn timeout(node: Ptr, seconds: f32) -> i32 {
    status(|| {
        // SystemAccessibility sets this bounded timeout before its reads.
        let context = if seconds == 0.12 { current_for_search()? } else { current()? };
        context.timeout(node, seconds)
    })
}
unsafe extern "C" fn attribute(node: Ptr, name: Ptr, out: *mut Ptr) -> i32 {
    unsafe { output(out, || current_for_search()?.attribute(node, &key(name)?)) }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn count(_: Ptr, _: Ptr, _: *mut isize) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym count_checked);
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn count(node: Ptr, name: Ptr, out: *mut isize) -> i32 {
    unsafe { count_checked(node, name, out, 0) }
}
unsafe extern "C" fn count_checked(node: Ptr, name: Ptr, out: *mut isize, caller: usize) -> i32 {
    if out.is_null() {
        return model::ILLEGAL;
    }
    unsafe {
        out.write(0);
    }
    status(|| {
        let context = current_for_search()?;
        let name = unsafe { key(name) }?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(binding) = &context.reference {
            binding.verify_list_source(caller, &name, None)?;
        }
        let _ = caller;
        let count = context.count(node, &name)?;
        unsafe {
            out.write(count);
        }
        Ok(())
    })
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn elements(_: Ptr, _: Ptr, _: isize, _: isize, _: *mut Ptr) -> i32 {
    core::arch::naked_asm!("mov x5, x30", "b {checked}", checked = sym elements_checked);
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn elements(node: Ptr, name: Ptr, index: isize, count: isize, out: *mut Ptr) -> i32 {
    unsafe { elements_checked(node, name, index, count, out, 0) }
}
unsafe extern "C" fn elements_checked(
    node: Ptr,
    name: Ptr,
    index: isize,
    count: isize,
    out: *mut Ptr,
    caller: usize,
) -> i32 {
    unsafe { output(out, || {
        let context = current_for_search()?;
        let name = key(name)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(binding) = &context.reference {
            binding.verify_list_source(caller, &name, Some((index, count)))?;
        }
        let _ = caller;
        context.elements(node, &name, index, count)
    }) }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn write(_: Ptr, _: Ptr, _: Ptr) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym write_checked);
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn write(node: Ptr, name: Ptr, value: Ptr) -> i32 {
    unsafe { write_checked(node, name, value, 0) }
}
unsafe extern "C" fn write_checked(node: Ptr, name: Ptr, value: Ptr, caller: usize) -> i32 {
    status(|| {
        let c = current()?;
        let name = unsafe { key(name) }?;
        let value = unsafe { decode(value) }?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(binding) = &c.reference {
            let base = binding.context.observe_app_delegate_offset_cells().map_err(|_| model::DISABLED)?.main_header();
            let restore = match (caller.checked_sub(base), name.as_str(), &value) {
                (Some(0x1c6810), "AXMinimized", Attribute::Boolean(false)) => true,
                (Some(0x1eb26c), "AXPosition", Attribute::Value(Payload::Point(_)))
                | (Some(0x1eb26c), "AXSize", Attribute::Value(Payload::Size(_))) => false,
                _ => return Err(model::ILLEGAL),
            };
            binding.verify_write_source(base, restore).map_err(|_| model::DISABLED)?;
            return c.write_inner(node, &name, value, restore);
        }
        let _ = caller;
        c.write(node, &name, value)
    })
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn action(_: Ptr, _: Ptr) -> i32 {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym action_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn action(node: Ptr, name: Ptr) -> i32 {
    unsafe { action_checked(node, name, 0) }
}

unsafe extern "C" fn action_checked(node: Ptr, name: Ptr, caller: usize) -> i32 {
    status(|| {
        let context = current()?;
        let name = unsafe { key(name) }?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(binding) = &context.reference {
            binding.verify_action_source(caller, &name).map_err(|_| model::DISABLED)?;
        }
        let _ = caller;
        context.action(node, &name)
    })
}
unsafe extern "C" fn value_kind(value: Ptr) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if current().is_err() {
            return 0;
        }
        opaque::payload(value).map_or(0, Payload::kind)
    }))
    .unwrap_or(0)
}
unsafe extern "C" fn value_create(kind: u32, input: *const c_void) -> Ptr {
    pointer(|| {
        current()?;
        if input.is_null() {
            return Err(model::ILLEGAL);
        }
        let value = match kind {
            1 => Payload::Point(unsafe { input.cast::<[f64; 2]>().read_unaligned() }),
            2 => Payload::Size(unsafe { input.cast::<[f64; 2]>().read_unaligned() }),
            4 => Payload::Range(unsafe { input.cast::<Range>().read_unaligned() }),
            _ => return Err(model::ILLEGAL),
        };
        if !model::valid_payload(value) {
            return Err(model::ILLEGAL);
        }
        Token::value(value).map_err(|_| model::ILLEGAL)
    })
}
unsafe extern "C" fn value_get(value: Ptr, kind: u32, out: Ptr) -> u8 {
    status(|| {
        current()?;
        if out.is_null() {
            return Err(model::ILLEGAL);
        }
        let p = opaque::payload(value).map_err(|_| model::ILLEGAL)?;
        if p.kind() != kind {
            return Err(model::ILLEGAL);
        }
        match p {
            Payload::Point(v) | Payload::Size(v) => unsafe {
                out.cast::<[f64; 2]>().write_unaligned(v)
            },
            Payload::Range(r) => unsafe { out.cast::<Range>().write_unaligned(r) },
        }
        Ok(())
    })
    .eq(&0)
    .into()
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ptr) -> u8;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: *const c_void, kind: isize, out: *mut c_void) -> u8;
    fn CFStringGetTypeID() -> usize;
}
unsafe fn decode(p: Ptr) -> model::Result<Attribute> {
    if p.is_null() {
        return Err(model::ILLEGAL);
    }
    if let Ok(v) = opaque::payload(p) {
        return Ok(Attribute::Value(v));
    }
    let id = unsafe { opaque::CFGetTypeID(p) };
    if id == unsafe { CFBooleanGetTypeID() } {
        return Ok(Attribute::Boolean(unsafe { CFBooleanGetValue(p) } != 0));
    }
    if id == unsafe { CFNumberGetTypeID() } {
        let mut n = 0_i64;
        if unsafe { CFNumberGetValue(p, 4, std::ptr::from_mut(&mut n).cast()) } == 0 {
            return Err(model::ILLEGAL);
        }
        return Ok(Attribute::Integer(n));
    }
    if id == unsafe { CFStringGetTypeID() } {
        return unsafe { opaque::string(p) }
            .map(Attribute::Text)
            .map_err(|_| model::ILLEGAL);
    }
    Err(model::ILLEGAL)
}
/// Exact implemented symbols. Neither this roster nor calling a replacement
/// directly establishes that a product import cell selects it.
pub fn replacements() -> [(&'static CStr, usize); 15] {
    [
        (c"AXIsProcessTrusted", trusted as *const () as usize),
        (
            c"AXUIElementCreateApplication",
            application as *const () as usize,
        ),
        (c"AXUIElementCreateSystemWide", system as *const () as usize),
        (c"AXUIElementGetTypeID", element_type as *const () as usize),
        (c"AXUIElementGetPid", pid as *const () as usize),
        (
            c"AXUIElementSetMessagingTimeout",
            timeout as *const () as usize,
        ),
        (
            c"AXUIElementCopyAttributeValue",
            attribute as *const () as usize,
        ),
        (
            c"AXUIElementGetAttributeValueCount",
            count as *const () as usize,
        ),
        (
            c"AXUIElementCopyAttributeValues",
            elements as *const () as usize,
        ),
        (c"AXUIElementSetAttributeValue", write as *const () as usize),
        (c"AXUIElementPerformAction", action as *const () as usize),
        (c"AXValueGetTypeID", value_type as *const () as usize),
        (c"AXValueGetType", value_kind as *const () as usize),
        (c"AXValueCreate", value_create as *const () as usize),
        (c"AXValueGetValue", value_get as *const () as usize),
    ]
}
/// Typed addresses only, never a real AX/permission operation. Reference Main
/// imports the exact14 subset below; the unused AXValueGetType must stay absent.
pub fn import_specs()->[crate::sdk::ImportSpec;15]{
    #[link(name="ApplicationServices",kind="framework")]
    unsafe extern "C"{
        fn AXIsProcessTrusted()->u8;
        fn AXUIElementCreateApplication(pid:i32)->Ptr;
        fn AXUIElementCreateSystemWide()->Ptr;
        fn AXUIElementGetTypeID()->usize;
        fn AXUIElementGetPid(node:Ptr,pid:*mut i32)->i32;
        fn AXUIElementSetMessagingTimeout(node:Ptr,seconds:f32)->i32;
        fn AXUIElementCopyAttributeValue(node:Ptr,key:Ptr,out:*mut Ptr)->i32;
        fn AXUIElementGetAttributeValueCount(node:Ptr,key:Ptr,out:*mut isize)->i32;
        fn AXUIElementCopyAttributeValues(node:Ptr,key:Ptr,index:isize,count:isize,out:*mut Ptr)->i32;
        fn AXUIElementSetAttributeValue(node:Ptr,key:Ptr,value:Ptr)->i32;
        fn AXUIElementPerformAction(node:Ptr,action:Ptr)->i32;
        fn AXValueGetTypeID()->usize;
        fn AXValueGetType(value:Ptr)->u32;
        fn AXValueCreate(kind:u32,input:*const c_void)->Ptr;
        fn AXValueGetValue(value:Ptr,kind:u32,out:Ptr)->u8;
    }
    let original=[AXIsProcessTrusted as *const () as usize,AXUIElementCreateApplication as *const () as usize,AXUIElementCreateSystemWide as *const () as usize,
        AXUIElementGetTypeID as *const () as usize,AXUIElementGetPid as *const () as usize,AXUIElementSetMessagingTimeout as *const () as usize,
        AXUIElementCopyAttributeValue as *const () as usize,AXUIElementGetAttributeValueCount as *const () as usize,AXUIElementCopyAttributeValues as *const () as usize,
        AXUIElementSetAttributeValue as *const () as usize,AXUIElementPerformAction as *const () as usize,AXValueGetTypeID as *const () as usize,
        AXValueGetType as *const () as usize,AXValueCreate as *const () as usize,AXValueGetValue as *const () as usize];
    let leaves=replacements();std::array::from_fn(|i|crate::sdk::ImportSpec{symbol:leaves[i].0,original:original[i],replacement:leaves[i].1})
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(super) struct ReferenceBinding{
    admission:Arc<crate::sdk::core_preentry::ReferenceAdmission>,context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,
    pub(super) model:Arc<crate::workspace::Workspace>,owner:std::thread::ThreadId,
    module:std::sync::OnceLock<(crate::sdk::owned::OwnedHandleLease,Vec<crate::sdk::ImportReplacement>)>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ReferenceBinding{
    fn verify_list_source(&self, caller: usize, name: &str, range: Option<(isize, isize)>) -> model::Result<()> {
        let limit = match name {
            "AXChildren" => 1200,
            "AXWindows" => 50,
            _ => return Err(model::ILLEGAL),
        };
        if range.is_some_and(|(index, count)| index != 0 || !(1..=limit).contains(&count)) {
            return Err(model::ILLEGAL);
        }
        let copy = range.is_some();
        let cell = self.admission.verify_reference_ax_list_call(caller, copy).map_err(|_| model::DISABLED)?;
        let symbol = if copy {
            c"AXUIElementCopyAttributeValues"
        } else {
            c"AXUIElementGetAttributeValueCount"
        };
        let (_, imports) = self.module.get().ok_or(model::DISABLED)?;
        let import = imports.iter().find(|row| row.symbol == symbol).ok_or(model::DISABLED)?;
        if import.cell_address() != cell || import.selected_address() != import.replacement {
            return Err(model::DISABLED);
        }
        Ok(())
    }
    fn verify_write_source(&self, base: usize, restore: bool) -> std::result::Result<(), String> {
        let at = |offset| base.checked_add(offset).ok_or("Reference AX write offset overflow");
        let (offset, length, hash) = if restore {
            (0x1c67b8, 88, "82732bc34f9cb46483467cb4775f0c0848068392b81bf48c6d00ce8d48cfdc38")
        } else {
            (0x1eb254, 28, "1a61e0bd0d50e57b3988e58361384e5fc8da75b52b6f8a7815dcfe359f3fb641")
        };
        self.context.verify_main_code(at(offset)?, length, hash)?;
        self.context.verify_main_code(at(0x2fb054)?, 12,
            "b7d6d989d65c5ed0afdf2f2bc67e95a34b7809880c950bf2dba2baf6692dfb4d")?;
        let (_, imports) = self.module.get().ok_or("Reference AX write imports absent")?;
        let import = imports.iter().find(|row| row.symbol == c"AXUIElementSetAttributeValue")
            .ok_or("Reference AX write import absent")?;
        if import.cell_address() != at(0x3c4b10)? || import.selected_address() != import.replacement {
            return Err("Reference AX write import differs".into());
        }
        Ok(())
    }
    fn verify_action_source(&self, caller: usize, name: &str) -> std::result::Result<(), String> {
        let base = self.context.observe_app_delegate_offset_cells()?.main_header();
        let (offset, hash) = match (caller.checked_sub(base), name) {
            (Some(0x1c6cd8), "AXPress") => (0x1c6cb4, "6e6058643c8c9eb09f0a299e08b9a5585df39a3abba1e549b5345485db889b16"),
            (Some(0x1c6d90), "AXRaise") => (0x1c6d6c, "44194e937cb05d4fb54d49f5a393b6390730475ef93b4e3eea43e0af754edcee"),
            _ => return Err("Reference AX action caller or selector differs".into()),
        };
        let at = |offset| base.checked_add(offset).ok_or("Reference AX action offset overflow");
        self.context.verify_main_code(at(offset)?, 40, hash)?;
        self.context.verify_main_code(at(0x2fb048)?, 12,
            "b77fed262e7f44e5d95f9fedaff5f145fee7d9511a2263cbb8b393a60dfddc85")?;
        let (_, imports) = self.module.get().ok_or("Reference AX action imports absent")?;
        let import = imports.iter().find(|row| row.symbol == c"AXUIElementPerformAction")
            .ok_or("Reference AX action import absent")?;
        if import.cell_address() != at(0x3c4b08)? || import.selected_address() != import.replacement {
            return Err("Reference AX action import differs".into());
        }
        Ok(())
    }
    fn search_worker(&self) -> bool {
        crate::workspace::callbacks::reference_live_search_dispatch_active(&self.admission)
    }
    pub(super) fn verify_search_worker(&self) -> std::result::Result<(), String> {
        if !self.search_worker() {
            return Err("Reference AX read outside an admitted search turn".into());
        }
        let (lease, imports) = self.module.get().ok_or("Reference AX worker imports absent")?;
        lease.before_effect()?;
        if imports.iter().any(|row| row.selected_address() != row.replacement) || !self.search_worker() {
            return Err("Reference AX worker import or turn changed".into());
        }
        Ok(())
    }
    pub(super) fn is_owner_thread(&self) -> bool {
        self.owner == std::thread::current().id()
    }
    fn verify(&self)->std::result::Result<(),String>{
        if self.owner!=std::thread::current().id(){return Err("Reference AX on foreign thread".into());}
        self.context.verify_admission_binding(&self.admission,&self.model)?;
        self.context.verify_input_parent_source_members()?;self.context.verify_original_compiler_text()?;
        let all=import_specs();let observed=crate::sdk::imports::observe_main_imports(&all)?;
        let expected=all.iter().filter(|row|row.symbol!=c"AXValueGetType").collect::<Vec<_>>();
        if observed.len()!=14||observed.iter().map(|row|row.symbol).collect::<std::collections::BTreeSet<_>>()
            !=expected.iter().map(|row|row.symbol).collect(){return Err("Reference AX14 imports/AXValueGetType absence changed".into());}
        for row in &observed{
            let spec=expected.iter().find(|s|s.symbol==row.symbol).ok_or("Foreign Reference AX import")?;
            // During the guarded transaction each cell is original or this
            // exact substitute. Published modules below require all substitutes.
            if row.selected_address()!=spec.original&&row.selected_address()!=spec.replacement{return Err("Reference AX import selects another owner".into());}
        }
        if let Some((lease,imports))=self.module.get(){lease.before_effect()?;for row in imports{if row.selected_address()!=row.replacement{return Err("Reference AX import changed".into());}}}
        Ok(())
    }
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl crate::sdk::owned::AliveLease for ReferenceBinding{
    fn is_alive(&self)->bool{self.owner==std::thread::current().id() || self.search_worker()}
    fn before_effect(&self)->std::result::Result<(),String>{
        if self.owner == std::thread::current().id() { self.verify() } else { self.verify_search_worker() }
    }
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
static REFERENCE_ATTEMPT:AtomicBool=AtomicBool::new(false);
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
fn reference_roster()->std::collections::BTreeSet<crate::sdk::SourceCallsite>{
    [
        ("reference-0.8.26/Sources/RuycastApp/HyperKey.swift","a60dff19c9ab210fd8dc3ae564f962a2708b1e32a2f69663ff2c48e6d70b1544"),
        ("reference-0.8.26/Sources/RuycastApp/SnippetExpander.swift","0982fd55b9406f9983d40803ada1f97d415251ca0e5908b888b27da7fd697b96"),
        ("reference-0.8.26/Sources/RuycastApp/SnippetChoice.swift","3d43c84d15dc76aaed1a38c5e7d4131232168fb0d0723c79e52deb1e447b9d69"),
        ("reference-0.8.26/Sources/RuycastApp/MenuActions.swift","8b0e725641d5e621b456848a7e695a6572d7f5c4ce5e1675e5e1f745dc1b7857"),
        ("reference-0.8.26/Sources/RuycastApp/SystemAccessibility.swift","ceb6320c8132040ab6fe32eb0989a86e11436f580683b7aef9c98aa7243305d2"),
        ("reference-0.8.26/Sources/RuycastApp/SystemServices.swift","c2ffbb73cc7723234992b1883bb582e68b175982a6a5991757cf47397e9935ad"),
        ("reference-0.8.26/Sources/RuycastApp/WindowManager.swift","01b6d2951b64698664e175694407e9df84faf8a08adb84966fd4a3db7f58fd87"),
    ].into_iter().map(|(file,sha)|crate::sdk::SourceCallsite{file,source_sha256:sha.into(),operation:"Owned Reference AX14 tree/geometry/value operations; no desktop AX service"}).collect()
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) unsafe fn initialize_reference(admission:Arc<crate::sdk::core_preentry::ReferenceAdmission>,source:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<crate::workspace::Workspace>)->std::result::Result<(Arc<Context>,crate::sdk::ModuleAttestation),String>{
    use std::collections::{BTreeMap,BTreeSet};
    if REFERENCE_ATTEMPT.swap(true,Ordering::AcqRel){return Err("Reference AX construction attempt already claimed".into());}
    let module=crate::sdk::bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;admission.verify_input_context(&module)?;
    let binding=Arc::new(ReferenceBinding{admission,context:source,model:model.clone(),owner:std::thread::current().id(),module:std::sync::OnceLock::new()});binding.verify()?;
    let apps=model.applications()?;let front=model.frontmost()?;let mut applications=BTreeMap::new();let mut specifications=Vec::new();let mut focused=None;
    for (index,(handle,app)) in apps.into_iter().filter(|(_,app)|!app.terminated).enumerate(){
        let id=u64::try_from(index).map_err(|_|"Reference AX app index width")?.checked_add(2).ok_or("Reference AX id overflow")?;
        if applications.insert(app.pid,id).is_some(){return Err("Reference AX duplicate synthetic PID".into());}
        if Some(handle)==front{focused=Some(id);}
        let mut attributes=BTreeMap::from([("AXRole".into(),Attribute::Text("AXApplication".into())),("AXWindows".into(),Attribute::Elements(vec![]))]);
        if app.pid == crate::sdk::system_providers::OWNED_EDITOR_PID {
            let window = u64::MAX;
            attributes.insert("AXWindows".into(),Attribute::Elements(vec![window]));
            attributes.insert("AXFocusedWindow".into(),Attribute::Element(window));
            specifications.push(model::NodeSpec {
                id: window,
                pid: app.pid,
                attributes: BTreeMap::from([
                    ("AXRole".into(), Attribute::Text("AXWindow".into())),
                    ("AXTitle".into(), Attribute::Text("Fixture Document".into())),
                    ("AXMinimized".into(), Attribute::Boolean(true)),
                    ("AXPosition".into(), Attribute::Value(Payload::Point([120.0, 100.0]))),
                    ("AXSize".into(), Attribute::Value(Payload::Size([800.0, 600.0]))),
                ]),
                writable: ["AXMinimized".into(), "AXPosition".into(), "AXSize".into()].into(),
                actions: ["AXRaise".into()].into(),
            });
            let menu_bar = u64::MAX - 1;
            let file_menu = u64::MAX - 2;
            let open_item = u64::MAX - 3;
            attributes.insert("AXMenuBar".into(), Attribute::Element(menu_bar));
            specifications.extend([
                model::NodeSpec {
                    id: menu_bar,
                    pid: app.pid,
                    attributes: BTreeMap::from([
                        ("AXRole".into(), Attribute::Text("AXMenuBar".into())),
                        ("AXChildren".into(), Attribute::Elements(vec![file_menu])),
                    ]),
                    writable: BTreeSet::new(),
                    actions: BTreeSet::new(),
                },
                model::NodeSpec {
                    id: file_menu,
                    pid: app.pid,
                    attributes: BTreeMap::from([
                        ("AXRole".into(), Attribute::Text("AXMenuBarItem".into())),
                        ("AXTitle".into(), Attribute::Text("File".into())),
                        ("AXEnabled".into(), Attribute::Boolean(true)),
                        ("AXChildren".into(), Attribute::Elements(vec![open_item])),
                    ]),
                    writable: BTreeSet::new(),
                    actions: BTreeSet::new(),
                },
                model::NodeSpec {
                    id: open_item,
                    pid: app.pid,
                    attributes: BTreeMap::from([
                        ("AXRole".into(), Attribute::Text("AXMenuItem".into())),
                        ("AXTitle".into(), Attribute::Text("Open Fixture".into())),
                        ("AXEnabled".into(), Attribute::Boolean(true)),
                        ("AXMenuItemCmdChar".into(), Attribute::Text("o".into())),
                        ("AXMenuItemCmdVirtualKey".into(), Attribute::Integer(31)),
                        ("AXMenuItemCmdModifiers".into(), Attribute::Integer(0)),
                        ("AXChildren".into(), Attribute::Elements(vec![])),
                    ]),
                    writable: BTreeSet::new(),
                    actions: ["AXPress".into()].into(),
                },
            ]);
        }
        specifications.push(model::NodeSpec{id,pid:app.pid,attributes,writable:BTreeSet::new(),actions:BTreeSet::new()});
    }
    let mut system_attributes=BTreeMap::from([("AXRole".into(),Attribute::Text("AXSystemWide".into()))]);
    if let Some(id)=focused{system_attributes.insert("AXFocusedApplication".into(),Attribute::Element(id));}
    specifications.push(model::NodeSpec{id:1,pid:0,attributes:system_attributes,writable:BTreeSet::new(),actions:BTreeSet::new()});
    // Unknown attributes fail closed. The editor owns the fixture window and menu.
    let mut candidate=Context::new(binding.clone(),1,applications,specifications)?;
    Arc::get_mut(&mut candidate).ok_or("Reference AX candidate unexpectedly shared")?.reference=Some(binding.clone());
    install(candidate.clone())?;
    struct Publication{context:Arc<Context>,complete:bool}
    impl Drop for Publication{fn drop(&mut self){if !self.complete{let _=retire(&self.context);}}}
    let mut publication=Publication{context:candidate.clone(),complete:false};
    struct Guard<'a>(&'a Context);impl crate::sdk::owned::AliveLease for Guard<'_>{
        fn is_alive(&self)->bool{self.0.live()}
        fn before_effect(&self)->std::result::Result<(),String>{if self.0.live(){Ok(())}else{Err("Reference AX retired during installation".into())}}
    }
    let specs=import_specs().into_iter().filter(|row|row.symbol!=c"AXValueGetType").collect::<Vec<_>>();
    let imports=crate::sdk::import_patch::install(&specs,&Guard(&candidate))?;let lease=module.lease("reference-accessibility")?;
    binding.module.set((lease,imports)).map_err(|_|"Reference AX module already installed")?;
    let (lease,imports)=binding.module.get().ok_or("Reference AX imports absent")?;
    let result=crate::sdk::ModuleAttestation{module:"reference-accessibility",scopes:BTreeSet::new(),callsites:reference_roster(),imports:imports.clone(),objc:vec![],handles:vec![lease.clone()],workspace_factories:None};
    verify_reference_attestation(&result)?;publication.complete=true;Ok((candidate,result))
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_editor(
    admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    workspace: &Arc<crate::workspace::Workspace>,
) -> std::result::Result<(), String> {
    let context = current().map_err(|_| "Reference editor AX owner absent")?;
    let binding = context.reference.as_ref().ok_or("Editor AX owner is not Reference")?;
    if !Arc::ptr_eq(&binding.admission, admission) || !Arc::ptr_eq(&binding.model, workspace) {
        return Err("Reference editor AX issuer differs".into());
    }
    let pid = crate::sdk::system_providers::OWNED_EDITOR_PID;
    let app = context.application(pid).map_err(|_| "Reference editor AX application absent")?;
    let window = context.attribute(app.ptr(), "AXFocusedWindow").map_err(|_| "Reference editor AX window absent")?;
    if context.count(app.ptr(), "AXWindows") != Ok(1) || context.pid(window.ptr()) != Ok(pid) {
        return Err("Reference editor AX window count or owner differs".into());
    }
    let title = context.attribute(window.ptr(), "AXTitle").map_err(|_| "Reference editor AX title absent")?;
    if unsafe { opaque::string(title.ptr()) }? != "Fixture Document" {
        return Err("Reference editor AX title differs".into());
    }
    let menu_bar = context.attribute(app.ptr(), "AXMenuBar").map_err(|_| "Reference editor AX menu bar absent")?;
    if context.pid(menu_bar.ptr()) != Ok(pid) || context.count(menu_bar.ptr(), "AXChildren") != Ok(1) {
        return Err("Reference editor AX menu bar owner or children differ".into());
    }
    crate::sdk::process_router::reference_workspace_identity(workspace, pid)?;
    binding.verify()
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn verify_reference_attestation(module:&crate::sdk::ModuleAttestation)->std::result::Result<(),String>{
    if module.module!="reference-accessibility"||!module.scopes.is_empty()||module.callsites!=reference_roster()||!module.objc.is_empty()
        ||module.workspace_factories.is_some()||module.handles.len()!=1{return Err("Reference AX module envelope differs".into());}
    // Registry verification observes the published module. It is also used
    // by Dispatch before a worker enters its source callback. It must not
    // request access to the AX tree on that worker.
    let context=CURRENT.lock().map_err(|_|"AX owner poisoned")?.clone().ok_or("Reference AX context absent")?;
    context.check_pure().map_err(|_|"Reference AX context retired")?;
    let binding=context.reference.as_ref().ok_or("AX context is not Reference")?;
    if binding.owner == std::thread::current().id() {
        binding.verify()?;
    }
    let (lease,imports)=binding.module.get().ok_or("Reference AX imports absent")?;
    if module.handles[0].namespace!=lease.namespace||module.handles[0].root.path()!=lease.root.path()||!Arc::ptr_eq(&module.handles[0].life,&lease.life)
        ||module.imports.len()!=imports.len(){return Err("Reference AX module issuer differs".into());}
    lease.before_effect()?;
    let mut symbols=std::collections::BTreeSet::new();for row in &module.imports{
        let expected=imports.iter().find(|expected|expected.symbol==row.symbol).ok_or("Foreign Reference AX import")?;
        if !symbols.insert(row.symbol)||row.original!=expected.original||row.replacement!=expected.replacement||row.cell_address()!=expected.cell_address()||row.selected_address()!=row.replacement{return Err("Reference AX import differs".into());}
    }
    context.check_pure().map_err(|_|"Reference AX context retired during attestation")?;
    if !PUBLISHED.load(Ordering::Acquire)
        || !CURRENT.lock().map_err(|_|"AX owner poisoned")?.as_ref().is_some_and(|published|Arc::ptr_eq(published,&context))
    {
        return Err("Reference AX publication changed during attestation".into());
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn retire_reference(admission:&Arc<crate::sdk::core_preentry::ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->std::result::Result<(),String>{
    let context=CURRENT.lock().map_err(|_|"AX owner poisoned")?.clone();
    let context = context.ok_or("Reference AX teardown has no installed owner to drain")?;
    let binding=context.reference.as_ref().ok_or("AX teardown found another source family")?;
    if !Arc::ptr_eq(&binding.admission,admission)||!Arc::ptr_eq(&binding.model,model){return Err("AX teardown found another Reference owner".into());}
    retire(&context)
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    struct Audit;
    impl crate::sdk::owned::AliveLease for Audit {
        fn is_alive(&self) -> bool { true }
        fn before_effect(&self) -> std::result::Result<(), String> {
            CURRENT.try_lock().map(|_| ()).map_err(|_| "guardian ran under AX publication lock".into())
        }
    }
    #[test]
    fn dynamic_guard_runs_outside_publication_lock_and_specs_match_leaves() {
        let context = Context::new(Arc::new(Audit), 1, std::collections::BTreeMap::new(), vec![model::NodeSpec {
            id: 1, pid: 0, attributes: std::collections::BTreeMap::new(),
            writable: std::collections::BTreeSet::new(), actions: std::collections::BTreeSet::new(),
        }]).unwrap();
        install(context.clone()).unwrap();
        assert!(Arc::ptr_eq(&current().unwrap(), &context));
        assert_eq!(unsafe { trusted() }, 1);
        let specs = import_specs();
        assert_eq!(specs.len(), 15);
        for (spec, (symbol, replacement)) in specs.iter().zip(replacements()) {
            assert_eq!(spec.symbol, symbol);
            assert_eq!(spec.replacement, replacement);
            assert_ne!(spec.original, 0);
            assert_ne!(spec.original, spec.replacement);
        }
        retire(&context).unwrap();
        assert!(current().is_err());
    }
}
