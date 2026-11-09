//! Actual owned NSApplication receiver/run/delegate route. The real Cocoa run
//! method, menu publication and activation are never called. Selected original
//! delegate callbacks require the common code authority and complete SDK gate.
use super::super::{
    delegate::{CallbackCodeAuthority, ObjectiveC},
    event_loop::{Callback, Engine, Event},
};
use super::*;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};
/// Common coordinator supplies actual source/class/selector/code provenance;
/// qualification is supplied by the concrete source-bound catalog below.
/// # Safety
/// Bind all present lifecycle and allowed control selectors to independently
/// verified method entries/source/images including trampolines and late classes.
pub unsafe trait CallbackCatalog: CallbackCodeAuthority {
    fn class_binding(
        &self,
        class: &AnyClass,
    ) -> Result<(crate::sdk::SourceCallsite, Vec<(&'static CStr, usize)>)>;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn verify_published_class(
        &self, model: &Arc<Workspace>, class: &AnyClass, source: &crate::sdk::SourceCallsite,
    ) -> Result<()>;
}
struct SourceCallbackClass {
    class:&'static AnyClass,
    source:crate::sdk::SourceCallsite,
    methods:Vec<(&'static CStr,crate::sdk::metadata::MethodSnapshot)>,
}
/// Immutable projection of independently qualified compiler-owned callbacks.
/// This catalog does not turn runtime addresses or source roster rows into a
/// code authority: its supplied authority must qualify every selected entry and
/// the complete generated lifecycle before construction can succeed.
pub struct SourceCallbackCatalog {
    model:Arc<Workspace>,
    owner:std::thread::ThreadId,
    generation:u64,
    authority:Arc<dyn CallbackCodeAuthority>,
    classes:Vec<SourceCallbackClass>,
    retired:std::sync::atomic::AtomicBool,
    business_retired:std::sync::atomic::AtomicBool,
}
impl SourceCallbackCatalog {
    #[cfg(all(rcf1_reference_sdk, not(test)))]
    pub(crate) fn verify_model(&self, model: &Arc<Workspace>) -> Result<()> {
        if !Arc::ptr_eq(&self.model, model) {
            return Err("source callback catalog model differs".into());
        }
        self.business_check()
    }

    /// # Safety
    /// `authority` must bind actual compiled source, C/unwind ABI, generated
    /// lifecycle/ARC and image custody for these exact original classes. A
    /// method address list or a source file hash alone cannot meet that contract.
    /// No original callback is invoked to discover or qualify its implementation.
    pub unsafe fn qualify(
        model:Arc<Workspace>,authority:Arc<dyn CallbackCodeAuthority>,
        rows:Vec<(&'static AnyClass,crate::sdk::SourceCallsite,Vec<&'static CStr>)>,
    )->Result<Arc<Self>> {
        model.lease().before_effect()?;
        if rows.is_empty() { return Err("source callback catalog empty".into()); }
        let generation=model.state.lock().map_err(|_|"workspace state poisoned")?.generation;
        let roster=super::super::source_roster();
        let mut classes=Vec::new();
        let mut seen=std::collections::BTreeSet::new();
        for (class,source,selectors) in rows {
            if !seen.insert(class as *const AnyClass as usize)||!roster.contains(&source)||selectors.is_empty() {
                return Err("source callback class/source roster differs".into());
            }
            let mut methods=Vec::new();
            let mut names=std::collections::BTreeSet::new();
            for selector in selectors {
                if !names.insert(selector) { return Err("source callback selector duplicated".into()); }
                let observed=super::super::raw_method(class,Sel::register(selector),false)?
                    .ok_or("source callback entry absent")?;
                methods.push((selector,observed));
            }
            let selected = methods.iter().map(|(selector, method)| (*selector, method.implementation()))
                .collect::<Vec<_>>();
            authority.verify_methods(class, &selected, &source)?;
            for (selector, observed) in &methods {
                let after=super::super::raw_method(class,Sel::register(selector),false)?
                    .ok_or("source callback disappeared during qualification")?;
                if &after!=observed { return Err("source callback changed during qualification".into()); }
            }
            authority.verify_cleanup(class,&source)?;
            classes.push(SourceCallbackClass{class,source,methods});
        }
        let result=Arc::new(Self{model,owner:std::thread::current().id(),generation,authority,classes,
            retired:std::sync::atomic::AtomicBool::new(false),
            business_retired:std::sync::atomic::AtomicBool::new(false)});
        result.business_check()?;
        Ok(result)
    }
    fn pure_business(&self)->Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("source callback catalog on foreign owner thread".into());
        }
        self.pure_publication()
    }
    fn pure_publication(&self)->Result<()> {
        let state=self.model.state.lock().map_err(|_|"workspace state poisoned")?;
        if self.retired.load(std::sync::atomic::Ordering::Acquire)
            ||self.business_retired.load(std::sync::atomic::Ordering::Acquire)
            ||state.retired||state.generation!=self.generation
            ||self.model.faulted.load(std::sync::atomic::Ordering::Acquire) {
            return Err("source callback catalog epoch retired".into());
        }
        Ok(())
    }
    fn metadata_check(&self)->Result<()> {
        if self.retired.load(std::sync::atomic::Ordering::Acquire) { return Err("source callback catalog retired".into()); }
        for entry in &self.classes {
            for (selector,observed) in &entry.methods {
                if self.qualification(super::super::raw_method(entry.class,Sel::register(selector),false))?.as_ref()!=Some(observed) {
                    self.retired.store(true,std::sync::atomic::Ordering::Release);
                    return Err("source callback catalog method changed".into());
                }
            }
        }
        Ok(())
    }
    fn business_check(&self)->Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("source callback catalog on foreign owner thread".into());
        }
        let result=(||{
            self.pure_business()?;
            self.model.lease().before_effect()?;
            self.metadata_check()?;
            self.pure_business()
        })();
        // A nested guardian refusal cannot be erased by a later outer Ok.
        // Physical cleanup retains its separate code/identity qualification.
        if result.is_err() { self.business_retired.store(true,std::sync::atomic::Ordering::Release); }
        result
    }
    fn qualification<T>(&self,result:Result<T>)->Result<T> {
        if result.is_err() { self.retired.store(true,std::sync::atomic::Ordering::Release); }
        result
    }
    fn entry(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<&SourceCallbackClass> {
        self.classes.iter().find(|entry|entry.class==class&&entry.source==*source)
            .ok_or("source callback class binding differs".into())
    }
}
// The only constructor first invokes the independent unsafe code authority;
// every dynamic qualification is followed by fresh metadata and epoch checks.
unsafe impl CallbackCatalog for SourceCallbackCatalog {
    fn class_binding(&self,class:&AnyClass)->Result<(crate::sdk::SourceCallsite,Vec<(&'static CStr,usize)>)> {
        self.business_check()?;
        let entry=self.classes.iter().find(|entry|entry.class==class).ok_or("source callback class absent")?;
        Ok((entry.source.clone(),entry.methods.iter().map(|(name,method)|(*name,method.implementation())).collect()))
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn verify_published_class(
        &self, model: &Arc<Workspace>, class: &AnyClass, source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.model, model) || self.entry(class, source)?.methods.is_empty() {
            return Err("published callback class/model differs".into());
        }
        self.pure_publication()?;
        self.model.lease().before_effect()?;
        self.metadata_check()?;
        self.pure_publication()
    }
}
unsafe impl CallbackCodeAuthority for SourceCallbackCatalog {
    fn verify_method(&self,class:&AnyClass,selector:Sel,imp:usize,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.business_check()?;
        let entry=self.entry(class,source)?;
        if !entry.methods.iter().any(|(name,method)|Sel::register(name)==selector&&method.implementation()==imp) {
            return Err("source callback method not selected".into());
        }
        self.qualification(self.authority.verify_method(class,selector,imp,source))?;
        self.business_check()
    }
    fn verify_methods(&self, class: &AnyClass, methods: &[(&CStr, usize)],
        source: &crate::sdk::SourceCallsite) -> Result<()> {
        self.business_check()?;
        let entry = self.entry(class, source)?;
        if methods.is_empty() || methods.iter().any(|(selector, implementation)| {
            !entry.methods.iter().any(|(name, method)| name == selector
                && method.implementation() == *implementation)
        }) {
            return Err("source callback method roster not selected".into());
        }
        self.qualification(self.authority.verify_methods(class, methods, source))?;
        self.business_check()
    }
    fn verify_sender(&self,class:&AnyClass,selector:Sel,imp:usize,source:&crate::sdk::SourceCallsite,sender:&AnyClass)->Result<()> {
        self.verify_method(class,selector,imp,source)?;
        self.qualification(self.authority.verify_sender(class,selector,imp,source,sender))?;
        self.business_check()
    }
    fn verify_cleanup(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.entry(class,source)?;
        self.metadata_check()?;
        self.qualification(self.authority.verify_cleanup(class,source))?;
        self.metadata_check()
    }
    fn verify_receiver_cleanup(&self,receiver:&super::super::receiver::ReceiverIdentity,source:&crate::sdk::SourceCallsite)->Result<()> {
        if !receiver.belongs_to(&self.model) { return Err("foreign source callback receiver".into()); }
        self.entry(receiver.class(),source)?;
        self.metadata_check()?;
        self.qualification(self.authority.verify_receiver_cleanup(receiver,source))?;
        self.metadata_check()
    }
    fn verify_source_construction(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        if !self.classes.iter().any(|entry|entry.class==class)
            ||!super::super::source_allocation_callsites().contains(source) {
            return Err("source construction callsite not selected".into());
        }
        self.business_check()?;
        self.qualification(self.authority.verify_source_construction(class,source))?;
        self.business_check()
    }
    fn verify_source_partial_cleanup(&self,receiver:&super::super::receiver::SourceConstructionIdentity<'_>,source:&crate::sdk::SourceCallsite)->Result<()> {
        if !receiver.belongs_to(&self.model) { return Err("foreign source partial allocation".into()); }
        if !self.classes.iter().any(|entry|entry.class==receiver.class())
            ||!super::super::source_allocation_callsites().contains(source) {
            return Err("partial cleanup source callsite not selected".into());
        }
        receiver.verify_allocation()?;
        self.metadata_check()?;
        self.qualification(self.authority.verify_source_partial_cleanup(receiver,source))?;
        receiver.verify_allocation()?;
        self.metadata_check()
    }
}
struct Properties {
    policy: isize,
    active: bool,
}
struct Application {
    model: Arc<Workspace>,
    engine: Arc<Engine>,
    properties: Mutex<Properties>,
    catalog: Mutex<Option<Arc<dyn CallbackCatalog>>>,
    owner: std::thread::ThreadId,
    retired: std::sync::atomic::AtomicBool,
    generation: u64,
    delegate_epoch: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    fixtures: Mutex<BTreeMap<usize, Arc<dyn Callback>>>,
}
enum ApplicationReference {
    Qualified(super::super::factory::OwnedReceiver),
    #[cfg(test)]
    Fixture(Retained<AnyObject>),
}
impl ApplicationReference {
    unsafe fn adopt(pointer: *mut AnyObject) -> Result<Self> {
        #[cfg(test)]
        if INSTALLED.get().is_some_and(|s| s.fixture_classes) {
            return unsafe { Retained::from_raw(pointer) }.map(Self::Fixture)
                .ok_or("nil application fixture reference".into());
        }
        Ok(Self::Qualified(unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?))
    }
    fn as_ptr(&self) -> *mut AnyObject {
        match self {
            Self::Qualified(owner) => owner.as_ptr(),
            #[cfg(test)]
            Self::Fixture(owner) => Retained::as_ptr(owner).cast_mut(),
        }
    }
    unsafe fn retain(pointer: *mut AnyObject) -> Result<Self> {
        #[cfg(test)]
        if INSTALLED.get().is_some_and(|s| s.fixture_classes) {
            return unsafe { Retained::retain(pointer) }.map(Self::Fixture)
                .ok_or("nil application fixture retain".into());
        }
        let retained = unsafe { super::super::factory::retain_owned(pointer) }?;
        unsafe { Self::adopt(retained) }
    }
    unsafe fn autorelease(self) -> Result<*mut AnyObject> {
        match self {
            Self::Qualified(owner) => {
                let returned = unsafe { super::super::factory::autorelease_owned(owner.as_ptr()) }?;
                owner.into_raw();
                Ok(returned)
            }
            #[cfg(test)]
            Self::Fixture(owner) => Ok(Retained::autorelease_return(owner)),
        }
    }
}
// These Objective-C owners are confined to the established application thread;
// no unsafe Send/Sync wrapper turns UI ownership into cross-thread authority.
#[derive(Default)]
struct ApplicationReferences {
    delegate: Option<std::rc::Rc<DelegateReference>>,
    menu: Option<ApplicationReference>,
    shell: Option<ApplicationReference>,
}
thread_local! {
    static REFERENCES: std::cell::RefCell<ApplicationReferences> =
        std::cell::RefCell::new(ApplicationReferences::default());
}
static APP: OnceLock<Application> = OnceLock::new();
// Written exactly once during exclusive SDK setup, before original entry or
// threads can read the substituted NSApp data symbol. Never mutated afterwards.
static mut NSAPP_VALUE: *mut AnyObject = std::ptr::null_mut();
pub(super) fn initialize(model: Arc<Workspace>) -> Result<()> {
    let engine = Engine::new(model.clone())?;
    let generation = model.with(|state| Ok(state.generation))?;
    APP.set(Application {
        model,
        engine,
        owner: std::thread::current().id(),
        retired: std::sync::atomic::AtomicBool::new(false),
        generation,
        delegate_epoch: std::sync::atomic::AtomicU64::new(0),
        properties: Mutex::new(Properties {
            policy: 2,
            active: false,
        }),
        catalog: Mutex::new(None),
        #[cfg(test)]
        fixtures: Mutex::new(BTreeMap::new()),
    })
    .map_err(|_| "application shell already configured")?;
    let shell = object(Kind::ApplicationShell)?;
    let owned = unsafe { ApplicationReference::adopt(shell) }?;
    REFERENCES.with(|references| references.borrow_mut().shell = Some(owned));
    unsafe { NSAPP_VALUE = shell };
    Ok(())
}
fn pure_state(state: &Application) -> Result<()> {
    if state.owner != std::thread::current().id() {
        return Err("application shell on foreign owner thread".into());
    }
    pure_publication(state)
}
fn pure_publication(state: &Application) -> Result<()> {
    if state.retired.load(std::sync::atomic::Ordering::Acquire)
        || state.model.faulted.load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("application shell retired".into());
    }
    let model = state.model.state.lock().map_err(|_| "workspace state poisoned")?;
    if model.retired || model.generation != state.generation {
        return Err("application shell generation expired".into());
    }
    Ok(())
}
#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
pub(super) fn verify_attested_shell(model: &Arc<Workspace>) -> Result<()> {
    let state = APP.get().ok_or("application shell absent")?;
    if !Arc::ptr_eq(&state.model, model) || !Arc::ptr_eq(state.engine.model(), model) {
        return Err("application shell and workspace publication differ".into());
    }
    pure_publication(state)?;
    model.with(|_| Ok(()))?;
    pure_publication(state)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn configuration_worker_wake(model: &Arc<Workspace>) -> Result<Arc<super::super::cf_loop::Wake>> {
    verify_attested_shell(model)?;
    let state = APP.get().ok_or("application shell absent")?;
    let wake = state.engine.wake_handle()?;
    pure_publication(state)?;
    Ok(wake)
}
fn state() -> Result<&'static Application> {
    let state = APP.get().ok_or("application shell absent")?;
    pure_state(state)?;
    state.model.with(|_| Ok(()))?;
    pure_state(state)?;
    Ok(state)
}
/// Owner-thread endpoint teardown. Retirement precedes callback/ARC cleanup;
/// cleanup never depends on reacquiring live business admission.
pub fn retire() -> Result<()> {
    let state = APP.get().ok_or("application shell absent")?;
    if state.owner != std::thread::current().id() {
        return Err("application teardown on foreign owner thread".into());
    }
    if state.retired.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Ok(());
    }
    unsafe { NSAPP_VALUE = std::ptr::null_mut() };
    let engine_result = state.engine.retire();
    let owned = REFERENCES.with(|references| std::mem::take(&mut *references.borrow_mut()));
    // Release ARC owners outside the TLS borrow, so selected cleanup may reenter
    // the already-retired boundary without a RefCell borrow panic.
    drop(owned);
    engine_result
}
fn receiver(p: *mut AnyObject) -> Result<&'static Application> {
    if kind(p)? != Kind::ApplicationShell {
        return Err("unknown application receiver".into());
    }
    state()
}
pub fn engine() -> Result<Arc<Engine>> {
    Ok(state()?.engine.clone())
}
pub fn configure_catalog(catalog: Arc<dyn CallbackCatalog>) -> Result<()> {
    let mut selected = state()?
        .catalog
        .lock()
        .map_err(|_| "callback catalog poisoned")?;
    if selected.is_some() {
        return Err("callback catalog already selected".into());
    }
    *selected = Some(catalog);
    Ok(())
}
pub fn data_import_replacement(symbol: &CStr) -> Option<usize> {
    (symbol == c"NSApp").then_some((&raw const NSAPP_VALUE) as usize)
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
pub(super) fn callback(
    target: *mut AnyObject,
    application: *mut AnyObject,
) -> Result<Arc<dyn Callback>> {
    #[cfg(test)]
    let s = state()?;
    #[cfg(test)]
    if let Some(callback) = s
        .fixtures
        .lock()
        .map_err(|_| "fixture callback registry poisoned")?
        .get(&(target as usize))
        .cloned()
    {
        return Ok(callback);
    }
    Ok(selected_callback(target, application)? as Arc<dyn Callback>)
}
pub(super) fn selected_callback(
    target: *mut AnyObject,
    application: *mut AnyObject,
) -> Result<Arc<ObjectiveC>> {
    selected_callback_binding(target, application, None).map(|binding| binding.0)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn selected_control_callback(
    target: &super::super::receiver::ReceiverIdentity,
    application: *mut AnyObject,
) -> Result<Arc<dyn Callback>> {
    let (callback, _, _) = selected_callback_binding(
        target.receiver() as *mut AnyObject, application, Some(target),
    )?;
    Ok(callback)
}
fn selected_callback_binding(
    target: *mut AnyObject,
    application: *mut AnyObject,
    expected: Option<&super::super::receiver::ReceiverIdentity>,
) -> Result<(Arc<ObjectiveC>, crate::sdk::SourceCallsite, Arc<dyn CallbackCatalog>)> {
    let s = state()?;
    if target.is_null() {
        return Err("null callback target".into());
    }
    if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
        return Err("complete SDK not admitted before original callback catalog".into());
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let catalog = s
            .catalog
            .lock()
            .map_err(|_| "callback catalog poisoned")?
            .clone()
            .ok_or("required bound-code callback catalog absent")?;
        // Only an actual initialized allocation ledger entry can supply the
        // class. No raw target.class() discovery on an unknown/dead receiver.
        let target_identity = super::super::factory::receiver_identity(target)?;
        if let Some(expected) = expected {
            expected.verify_pure()?;
            if expected.receiver() != target_identity.receiver()
                || expected.allocation_serial() != target_identity.allocation_serial()
            {
                return Err("control callback target allocation changed".into());
            }
        }
        let application_identity = super::super::factory::receiver_identity(application)?;
        if !target_identity.belongs_to(&s.model) || !application_identity.belongs_to(&s.model) {
            return Err("callback receiver belongs to another application model".into());
        }
        application_identity.verify_sdk_cleanup()?;
        let (source, methods) = catalog.class_binding(target_identity.class())?;
        target_identity.verify_pure()?;
        application_identity.verify_pure()?;
        let authority: Arc<dyn CallbackCodeAuthority> = catalog.clone();
        let selected = Arc::new(unsafe {
            ObjectiveC::from_original(
                s.model.clone(),
                target,
                application,
                &methods,
                &source,
                authority,
            )
        }?);
        if let Some(expected) = expected {
            expected.verify_pure()?;
        }
        Ok((selected, source, catalog))
    }));
    match outcome {
        Ok(result) => result,
        Err(payload) => {
            std::mem::forget(payload);
            s.model
                .faulted
                .store(true, std::sync::atomic::Ordering::Release);
            Err("original callback catalog panicked; authority retired".into())
        }
    }
}
fn fatal() -> ! {
    unsafe { libc::_exit(78) }
}
unsafe fn shared_body(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    if state().is_err() {
        return std::ptr::null_mut();
    }
    // The sole owner remains in REFERENCES; sharedApplication is a +0 getter.
    unsafe { NSAPP_VALUE }
}
enum DelegateReference {
    Qualified {
        weak: objc2::rc::Weak<AnyObject>,
        class: &'static AnyClass,
        identity: super::super::receiver::ReceiverIdentity,
        source: crate::sdk::SourceCallsite,
        catalog: Arc<dyn CallbackCatalog>,
        epoch: u64,
    },
    // Old explicitly registered own callbacks are not evidence of production
    // weak-reference qualification. This branch performs no ObjC weak operation.
    #[cfg(test)]
    Fixture(usize),
}
// A successful weak load owns one real +1. Errors must not run an ordinary
// Retained destructor after the object/source identity became unqualified.
struct WeakPin {
    value: Option<Retained<AnyObject>>,
    identity: super::super::receiver::ReceiverIdentity,
    source: crate::sdk::SourceCallsite,
    catalog: Arc<dyn CallbackCatalog>,
}
impl WeakPin {
    fn pointer(&self) -> *mut AnyObject {
        self.value.as_ref().map_or(std::ptr::null_mut(), |value| Retained::as_ptr(value).cast_mut())
    }
    fn qualify_cleanup(&self) -> Result<()> {
        self.identity.verify_allocation()?;
        self.catalog.verify_cleanup(self.identity.class(), &self.source)?;
        self.identity.verify_allocation()?;
        self.catalog.verify_receiver_cleanup(&self.identity, &self.source)?;
        self.identity.verify_allocation()?;
        if self.pointer() as usize != self.identity.receiver() {
            return Err("weak load returned another receiver".into());
        }
        // The observed physical allocation and the qualified weak +1 permit
        // this live-object class read; no dead raw-target discovery is used.
        if unsafe { &*self.pointer() }.class() != self.identity.class() {
            return Err("weak target class changed".into());
        }
        Ok(())
    }
}
impl Drop for WeakPin {
    fn drop(&mut self) {
        if self.value.is_none() { return; }
        let qualified = catch_unwind(AssertUnwindSafe(|| self.qualify_cleanup()));
        let value = self.value.take(); // release obligation before foreign code
        match qualified {
            Ok(Ok(())) => drop(value),
            other => {
                if let Err(payload) = other { std::mem::forget(payload); }
                // This is the actual already-acquired +1, not a fabricated
                // ownership count or permission to release another receiver.
                std::mem::forget(value);
                if let Some(state) = APP.get() { state.model.fault(); }
            }
        }
    }
}
struct WeakDelegate {
    application: usize,
    epoch: u64,
}
fn next_delegate_epoch(state: &Application) -> Result<u64> {
    state.delegate_epoch.fetch_update(
        std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire,
        |epoch| epoch.checked_add(1),
    ).map(|previous| previous + 1).map_err(|_| "application delegate epoch exhausted".into())
}
fn load_delegate() -> Result<Option<WeakPin>> {
    let state = state()?;
    // Rc clones only the weak-slot lifetime, not the Objective-C target. No
    // dynamic code, retain or weak operation crosses a TLS RefCell borrow.
    let reference = REFERENCES.with(|refs| refs.borrow().delegate.clone());
    let Some(reference) = reference else { return Ok(None); };
    match &*reference {
        DelegateReference::Qualified { weak, class, identity, source, catalog, epoch } => {
            identity.verify()?;
            catalog.verify_cleanup(class, source)?;
            identity.verify_pure()?;
            catalog.verify_receiver_cleanup(identity, source)?;
            identity.verify_pure()?;
            pure_state(state)?;
            if state.delegate_epoch.load(std::sync::atomic::Ordering::Acquire) != *epoch {
                return Err("application delegate changed during qualification".into());
            }
            let Some(value) = weak.load() else { return Ok(None); };
            let retained = WeakPin { value: Some(value), identity: identity.clone(), source: source.clone(), catalog: catalog.clone() };
            retained.qualify_cleanup()?;
            identity.verify_pure()?;
            pure_state(state)?;
            if state.delegate_epoch.load(std::sync::atomic::Ordering::Acquire) != *epoch {
                return Err("application delegate changed during weak load".into());
            }
            Ok(Some(retained))
        }
        #[cfg(test)]
        DelegateReference::Fixture(_) => Err("fixture delegate is not a qualified weak reference".into()),
    }
}
impl WeakDelegate {
    fn selected(&self) -> Result<Option<Arc<dyn Callback>>> {
        let state = state()?;
        if state.delegate_epoch.load(std::sync::atomic::Ordering::Acquire) != self.epoch {
            return Err("stale application delegate callback".into());
        }
        let Some(target) = load_delegate()? else { return Ok(None); };
        let selected = selected_callback(target.pointer(), self.application as *mut AnyObject)?;
        pure_state(state)?;
        if state.delegate_epoch.load(std::sync::atomic::Ordering::Acquire) != self.epoch {
            return Err("delegate changed during callback selection".into());
        }
        Ok(Some(selected))
    }
}
impl Callback for WeakDelegate {
    fn lifecycle(&self, event: super::super::event_loop::Lifecycle) -> Result<()> {
        if let Some(selected) = self.selected()? { selected.lifecycle(event)?; }
        Ok(())
    }
    fn action(&self, selector: &str) -> Result<()> {
        if let Some(selected) = self.selected()? { selected.action(selector)?; }
        Ok(())
    }
}
unsafe fn set_delegate_body(p: *mut AnyObject, _: Sel, target: *mut AnyObject) {
    let result = (|| -> Result<()> {
        let state = receiver(p)?;
        if target.is_null() {
            next_delegate_epoch(state)?;
            state.engine.clear_delegate()?;
            let previous = REFERENCES.with(|refs| refs.borrow_mut().delegate.take());
            drop(previous);
            return Ok(());
        }
        #[cfg(test)]
        if state.fixtures.lock().map_err(|_| "fixture callback registry poisoned")?.contains_key(&(target as usize)) {
            state.engine.set_delegate(callback(target, p)?)?;
            let previous = REFERENCES.with(|refs| refs.borrow_mut().delegate.replace(std::rc::Rc::new(DelegateReference::Fixture(target as usize))));
            drop(previous);
            return Ok(());
        }
        let (selected, source, catalog) = selected_callback_binding(target, p, None)?;
        let class = selected.class_identity();
        let identity = super::super::factory::receiver_identity(target)?;
        if !identity.original_source() || !identity.belongs_to(&state.model) || identity.class() != class {
            return Err("original weak target allocation/source role differs".into());
        }
        identity.verify()?;
        catalog.verify_cleanup(class, &source)?;
        identity.verify_pure()?;
        catalog.verify_receiver_cleanup(&identity, &source)?;
        identity.verify_pure()?;
        pure_state(state)?;
        let weak = objc2::rc::Weak::new(unsafe { &*target });
        identity.verify_pure()?;
        pure_state(state)?;
        let epoch = next_delegate_epoch(state)?;
        let reference = std::rc::Rc::new(DelegateReference::Qualified { weak, class, identity, source, catalog, epoch });
        let previous = REFERENCES.with(|refs| refs.borrow_mut().delegate.replace(reference));
        let engine_result = state.engine.set_delegate(Arc::new(WeakDelegate { application: p as usize, epoch }));
        if let Err(error) = engine_result {
            next_delegate_epoch(state)?;
            let failed = REFERENCES.with(|refs| std::mem::replace(&mut refs.borrow_mut().delegate, previous));
            drop(failed);
            return Err(error);
        }
        drop(previous);
        drop(selected); // no persistent strong target in Engine/WeakDelegate
        Ok(())
    })();
    if result.is_err() { fatal(); }
}
unsafe fn delegate_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        let application = receiver(p)?;
        #[cfg(test)]
        if let Some(pointer) = REFERENCES.with(|refs| match refs.borrow().delegate.as_deref() {
            Some(DelegateReference::Fixture(pointer)) => Some(*pointer), _ => None,
        }) { return Ok(pointer as *mut AnyObject); }
        let Some(pin) = load_delegate()? else { return Ok(std::ptr::null_mut()); };
        pin.qualify_cleanup()?;
        let retained = unsafe { Retained::retain(pin.pointer()) }.ok_or("delegate getter retain failed")?;
        // Handoff remains contingent on a fresh object-scoped cleanup proof.
        let returned = WeakPin { value: Some(retained), identity: pin.identity.clone(), source: pin.source.clone(), catalog: pin.catalog.clone() };
        returned.qualify_cleanup()?;
        pure_state(application)?;
        let mut returned = returned;
        let value = returned.value.take().ok_or("delegate getter ownership absent")?;
        Ok(Retained::autorelease_return(value))
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn run_body(p: *mut AnyObject, _: Sel) {
    let result = receiver(p).and_then(|s| super::super::cf_loop::run(s.engine.clone()));
    if result.is_err() {
        fatal();
    }
}
unsafe fn stop_body(p: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if receiver(p)
        .and_then(|s| s.engine.enqueue(Event::Stop))
        .is_err()
    {
        fatal();
    }
}
unsafe fn terminate_body(p: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if receiver(p)
        .and_then(|s| s.engine.enqueue(Event::Terminate))
        .is_err()
    {
        fatal();
    }
}
unsafe fn set_policy_body(p: *mut AnyObject, _: Sel, policy: isize) -> Bool {
    Bool::new(
        (|| -> Result<()> {
            if !(0..=2).contains(&policy) {
                return Err("unknown activation policy".into());
            }
            receiver(p)?
                .properties
                .lock()
                .map_err(|_| "application properties poisoned")?
                .policy = policy;
            Ok(())
        })()
        .is_ok(),
    )
}
unsafe fn policy_body(p: *mut AnyObject, _: Sel) -> isize {
    if let Ok(Kind::Application(h)) = kind(p) {
        return model()
            .and_then(|m| m.application(h))
            .map_or(2, |a| if a.regular { 0 } else { 1 });
    }
    receiver(p)
        .and_then(|s| {
            Ok(s.properties
                .lock()
                .map_err(|_| "application properties poisoned")?
                .policy)
        })
        .unwrap_or(2)
}
unsafe fn activate_body(p: *mut AnyObject, _: Sel) {
    if let Ok(s) = receiver(p) {
        if let Ok(mut properties) = s.properties.lock() {
            properties.active = true;
        }
    }
}
unsafe fn activate_flag_body(p: *mut AnyObject, s: Sel, _flag: Bool) {
    unsafe { activate(p, s) }
}
unsafe fn deactivate_body(p: *mut AnyObject, _: Sel) {
    if let Ok(s) = receiver(p) {
        if let Ok(mut properties) = s.properties.lock() {
            properties.active = false;
        }
    }
}
unsafe fn active_body(p: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(
        receiver(p)
            .and_then(|s| {
                Ok(s.properties
                    .lock()
                    .map_err(|_| "application properties poisoned")?
                    .active)
            })
            .unwrap_or(false),
    )
}
unsafe fn menu_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        receiver(p)?;
        let pointer = REFERENCES.with(|references| references.borrow().menu.as_ref().map(ApplicationReference::as_ptr));
        let retained = pointer.map(|p| unsafe { ApplicationReference::retain(p) }).transpose()?;
        state()?;
        match retained {
            Some(owned) => unsafe { owned.autorelease() },
            None => Ok(std::ptr::null_mut()),
        }
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn set_menu_body(p: *mut AnyObject, _: Sel, menu: *mut AnyObject) {
    let result = (|| -> Result<()> {
        receiver(p)?;
        if !menu.is_null() && !matches!(kind(menu)?, Kind::Menu(_)) {
            return Err("unowned main menu".into());
        }
        let retained = if menu.is_null() { None } else {
            Some(unsafe { ApplicationReference::retain(menu) }?)
        };
        state()?;
        let previous = REFERENCES.with(|references| {
            std::mem::replace(&mut references.borrow_mut().menu, retained)
        });
        drop(previous);
        Ok(())
    })();
    if result.is_err() {
        fatal();
    }
}
// Window identity comes exclusively from the existing model/receiver arena.
fn window_handles(key_only: bool) -> Result<Vec<Handle>> {
    state()?.model.with(|s| {
        Ok(s.windows.iter().filter(|(_, w)| !key_only || w.key)
            .map(|(h, _)| *h).collect())
    })
}
unsafe fn key_window_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        receiver(p)?;
        match window_handles(true)?.first() {
            Some(h) => object(Kind::Window(*h)),
            None => Ok(std::ptr::null_mut()),
        }
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn main_window_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        let state = receiver(p)?;
        let h = state.model.with(|model| {
            Ok(model.windows.iter().find(|(_, window)| window.main).map(|(handle, _)| *handle))
        })?;
        let result = h.map(|handle| object(Kind::Window(handle))).transpose()?
            .unwrap_or(std::ptr::null_mut());
        pure_state(state)?;
        Ok(result)
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn windows_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        receiver(p)?;
        array(window_handles(false)?.into_iter().map(Kind::Window))
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn current_event_body(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        let state = receiver(p)?;
        let Some(event) = state.engine.current_event()? else { return Ok(std::ptr::null_mut()); };
        let pointer = event.borrowed_pointer()?;
        state.engine.verify_current_event(&event)?;
        pure_state(state)?;
        Ok(pointer)
    })().unwrap_or(std::ptr::null_mut())
}
unsafe fn post_event_body(p: *mut AnyObject, _: Sel, event: *mut AnyObject, at_start: Bool) {
    let result = (|| -> Result<()> {
        let state = receiver(p)?;
        let event = event_handle(event)?;
        if !event.belongs_to(&state.model) { return Err("foreign application event".into()); }
        let held = EventLease::acquire(&event)?;
        state.engine.post_native_event(held, at_start.as_bool())?;
        pure_state(state)
    })();
    if result.is_err() { fatal(); }
}
// A Rust panic must never escape an Objective-C IMP's C boundary. Retirement
// uses only the already-installed model's atomic flag, not another callback.
macro_rules! application_boundary {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty => $body:ident, $fallback:expr;)*) => {
        $(pub(super) unsafe extern "C" fn $name($($arg: $ty),*) -> $ret {
            match catch_unwind(AssertUnwindSafe(|| unsafe { $body($($arg),*) })) {
                Ok(value) => value,
                Err(payload) => {
                    std::mem::forget(payload);
                    if let Some(state) = APP.get() {
                        state.model.fault();
                    }
                    $fallback
                }
            }
        })*
    };
}
application_boundary! {
    fn shared(p: *mut AnyObject, s: Sel) -> *mut AnyObject => shared_body, std::ptr::null_mut();
    fn set_delegate(p: *mut AnyObject, s: Sel, target: *mut AnyObject) -> () => set_delegate_body, ();
    fn delegate(p: *mut AnyObject, s: Sel) -> *mut AnyObject => delegate_body, std::ptr::null_mut();
    fn run(p: *mut AnyObject, s: Sel) -> () => run_body, ();
    fn stop(p: *mut AnyObject, s: Sel, sender: *mut AnyObject) -> () => stop_body, ();
    fn terminate(p: *mut AnyObject, s: Sel, sender: *mut AnyObject) -> () => terminate_body, ();
    fn set_policy(p: *mut AnyObject, s: Sel, policy: isize) -> Bool => set_policy_body, Bool::new(false);
    fn policy(p: *mut AnyObject, s: Sel) -> isize => policy_body, 2;
    fn activate(p: *mut AnyObject, s: Sel) -> () => activate_body, ();
    fn activate_flag(p: *mut AnyObject, s: Sel, flag: Bool) -> () => activate_flag_body, ();
    fn deactivate(p: *mut AnyObject, s: Sel) -> () => deactivate_body, ();
    fn active(p: *mut AnyObject, s: Sel) -> Bool => active_body, Bool::new(false);
    fn menu(p: *mut AnyObject, s: Sel) -> *mut AnyObject => menu_body, std::ptr::null_mut();
    fn set_menu(p: *mut AnyObject, s: Sel, menu: *mut AnyObject) -> () => set_menu_body, ();
    fn key_window(p: *mut AnyObject, s: Sel) -> *mut AnyObject => key_window_body, std::ptr::null_mut();
    fn main_window(p: *mut AnyObject, s: Sel) -> *mut AnyObject => main_window_body, std::ptr::null_mut();
    fn windows(p: *mut AnyObject, s: Sel) -> *mut AnyObject => windows_body, std::ptr::null_mut();
    fn current_event(p: *mut AnyObject, s: Sel) -> *mut AnyObject => current_event_body, std::ptr::null_mut();
    fn post_event(p: *mut AnyObject, s: Sel, event: *mut AnyObject, at_start: Bool) -> () => post_event_body, ();
}
macro_rules! application_roster {
    ($consumer:ident) => { $consumer! {
        (true,c"sharedApplication",shared,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"setDelegate:",set_delegate,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (false,c"delegate",delegate,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"run",run,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (false,c"stop:",stop,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (false,c"terminate:",terminate,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (false,c"setActivationPolicy:",set_policy,unsafe extern "C" fn(*mut AnyObject,Sel,isize)->Bool),
        (false,c"activationPolicy",policy,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        (false,c"activate",activate,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (false,c"deactivate",deactivate,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (false,c"activateIgnoringOtherApps:",activate_flag,unsafe extern "C" fn(*mut AnyObject,Sel,Bool)),
        (false,c"isActive",active,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (false,c"mainMenu",menu,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"setMainMenu:",set_menu,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (false,c"keyWindow",key_window,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"mainWindow",main_window,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"windows",windows,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"currentEvent",current_event,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"postEvent:atStart:",post_event,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,Bool))
    }};
}
fn signature<F: objc2::runtime::MethodImplementation>(_: F) -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut signature = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
    signature.extend_from_slice(F::Arguments::ENCODINGS);
    signature
}
#[cfg(test)]
macro_rules! add_application_method {
    ($builder:ident,true,$selector:expr,$function:ident,$ty:ty) => {};
    ($builder:ident,false,$selector:expr,$function:ident,$ty:ty) => {
        unsafe { $builder.add_method(Sel::register($selector), $function as $ty) }
    };
}
macro_rules! export_application_roster {
    ($(($meta:tt,$selector:expr,$function:ident,$ty:ty)),*$(,)?) => {
        pub fn replacements() -> Vec<(&'static CStr,&'static CStr,bool,usize)> {
            vec![$((c"NSApplication",$selector,$meta,$function as *const () as usize)),*]
        }
        pub fn signatures() -> Vec<(&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
            vec![$(($selector,$meta,signature($function as $ty))),*]
        }
        #[cfg(test)]
        pub(super) fn add_methods(builder: &mut objc2::runtime::ClassBuilder) {
            $(add_application_method!(builder,$meta,$selector,$function,$ty);)*
        }
    };
}
application_roster!(export_application_roster);

#[cfg(test)]
pub(super) fn own_fixture_delegate(
    target: *mut AnyObject,
    callback: Arc<dyn Callback>,
) -> Result<()> {
    state()?
        .fixtures
        .lock()
        .map_err(|_| "fixture callback registry poisoned")?
        .insert(target as usize, callback);
    Ok(())
}

#[cfg(test)]
pub(super) fn prove_fixture_application(model: Arc<Workspace>) {
    use crate::workspace::event_loop::Phase;
    use objc2::runtime::ClassBuilder;
    use objc2_foundation::NSNotification;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ORDER: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn will(_: *mut AnyObject, _: Sel, n: *mut AnyObject) {
        assert!(unsafe { &*n.cast::<NSNotification>() }.object().is_some());
        assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 0);
    }
    unsafe extern "C" fn did(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
        assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 1);
    }
    unsafe extern "C" fn toggle(_: *mut AnyObject, _: Sel) {
        assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 2);
    }
    unsafe extern "C" fn end(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
        assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 3);
    }
    let mut b = ClassBuilder::new(c"RuycastF1OwnedApplicationDelegate", NSObject::class()).unwrap();
    unsafe {
        b.add_method(
            objc2::sel!(applicationWillFinishLaunching:),
            will as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(applicationDidFinishLaunching:),
            did as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(ownedToggle),
            toggle as unsafe extern "C" fn(*mut AnyObject, Sel),
        );
        b.add_method(
            objc2::sel!(applicationWillTerminate:),
            end as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }
    let class = b.register();
    let target: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
    let p = Retained::as_ptr(&target).cast_mut().cast::<AnyObject>();
    let expected = [
        (
            c"applicationWillFinishLaunching:",
            will as *const () as usize,
        ),
        (c"applicationDidFinishLaunching:", did as *const () as usize),
        (c"ownedToggle", toggle as *const () as usize),
        (c"applicationWillTerminate:", end as *const () as usize),
    ];
    let selected = Arc::new(unsafe { ObjectiveC::owned_fixture(model, p, &expected) }.unwrap());
    own_fixture_delegate(p, selected.clone()).unwrap();
    let shell = unsafe { shared(std::ptr::null_mut(), objc2::sel!(sharedApplication)) };
    assert!(!shell.is_null());
    assert_eq!(unsafe { NSAPP_VALUE }, shell);
    let shell_model = &state().unwrap().model;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            verify_attested_shell(shell_model).unwrap();
            assert!(engine().is_err());
        }).join().unwrap();
    });
    unsafe {
        let _: () = objc2::msg_send![&*shell,setDelegate:&*target];
    }
    let current: *mut AnyObject = unsafe { objc2::msg_send![&*shell, delegate] };
    assert_eq!(current, p);
    let accepted: Bool = unsafe { objc2::msg_send![&*shell,setActivationPolicy:1isize] };
    assert!(accepted.as_bool());
    let policy: isize = unsafe { objc2::msg_send![&*shell, activationPolicy] };
    assert_eq!(policy, 1);
    let accepted: Bool = unsafe { objc2::msg_send![&*shell,setActivationPolicy:999isize] };
    assert!(!accepted.as_bool());
    unsafe {
        let _: () = objc2::msg_send![&*shell,activateIgnoringOtherApps:true];
    }
    let active: Bool = unsafe { objc2::msg_send![&*shell, isActive] };
    assert!(active.as_bool());
    unsafe {
        let _: () = objc2::msg_send![&*shell, deactivate];
    }
    let active: Bool = unsafe { objc2::msg_send![&*shell, isActive] };
    assert!(!active.as_bool());
    let menu = object(Kind::Menu(Handle {
        id: 900,
        generation: 1,
    }))
    .unwrap();
    unsafe {
        let _: () = objc2::msg_send![&*shell,setMainMenu:&*menu];
    }
    let actual: *mut AnyObject = unsafe { objc2::msg_send![&*shell, mainMenu] };
    assert_eq!(actual, menu);
    let engine = engine().unwrap();
    let handle = Handle {
        id: 901,
        generation: 1,
    };
    engine.register_target(handle, selected).unwrap();
    engine
        .enqueue(Event::Action {
            target: handle,
            selector: "ownedToggle".into(),
        })
        .unwrap();
    unsafe {
        let _: () = objc2::msg_send![&*shell,stop:std::ptr::null::<AnyObject>()];
        let _: () = objc2::msg_send![&*shell, run];
    }
    assert_eq!(engine.phase().unwrap(), Phase::Stopped);
    assert_eq!(ORDER.load(Ordering::SeqCst), 3);
    unsafe {
        let _: () = objc2::msg_send![&*shell,terminate:std::ptr::null::<AnyObject>()];
        let _: () = objc2::msg_send![&*shell, run];
    }
    assert_eq!(engine.phase().unwrap(), Phase::Terminated);
    assert_eq!(ORDER.load(Ordering::SeqCst), 4);
    unsafe {
        let _: () = objc2::msg_send![&*shell,setDelegate:std::ptr::null::<AnyObject>()];
    }
    struct PrematureCatalog(Arc<std::sync::atomic::AtomicUsize>);
    unsafe impl CallbackCodeAuthority for PrematureCatalog {
        fn verify_sender(
            &self,
            _: &AnyClass,
            _: Sel,
            _: usize,
            _: &crate::sdk::SourceCallsite,
            _: &AnyClass,
        ) -> Result<()> {
            Err("own catalog must remain uncalled before admission".into())
        }
        fn verify_method(
            &self,
            _: &AnyClass,
            _: Sel,
            _: usize,
            _: &crate::sdk::SourceCallsite,
        ) -> Result<()> {
            Err("own catalog must remain uncalled before admission".into())
        }
    }
    unsafe impl CallbackCatalog for PrematureCatalog {
        fn class_binding(
            &self,
            _: &AnyClass,
        ) -> Result<(crate::sdk::SourceCallsite, Vec<(&'static CStr, usize)>)> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("own catalog invoked before complete SDK admission");
        }
    }
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    configure_catalog(Arc::new(PrematureCatalog(calls.clone()))).unwrap();
    let unknown = NSObject::new();
    let error = callback(Retained::as_ptr(&unknown).cast_mut().cast(), shell)
        .err()
        .unwrap();
    assert!(error.contains("before original callback catalog"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
