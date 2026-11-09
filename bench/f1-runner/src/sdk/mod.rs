//! The source roster and actual address probes are executable admission rules.
//! No JSON receipt, count, environment flag or ready marker can register a module.
pub mod bootstrap;
pub mod descriptors;
pub mod imports;
pub mod metadata;
pub mod owned;
pub mod clipboard;
pub mod accessibility;
pub mod input_carbon;
pub mod input_permissions_caps;
pub mod input_hid;
pub mod input_events;
pub mod private_network;
pub mod profile_io;
pub mod process_dispatch;
pub mod process_abi;
pub mod process_fork;
pub mod core_preentry;
// The Reference SDK has one explicit entry and no SDK constructor.
#[cfg(rcf1_reference_sdk)]
pub mod reference_runtime;
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub mod reference_source_actors;
pub mod system_providers;
pub mod process_router;
pub mod task_model;
pub use system_providers::import_patch;
pub use imports::observe_main_imports;
pub use imports::{ImportReplacement, ImportSpec};
use owned::{AliveLease, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CStr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex, OnceLock},
};
pub const SCOPES: [&str; 14] = [
    "profile-home",
    "account-lookup",
    "clipboard",
    "workspace-apps-icons",
    "windows-menus-accessibility",
    "screens-mouse",
    "hotkeys-input-permissions",
    "safari-appleevents",
    "shortcuts-processes",
    "monitor-host-process-network-battery",
    "provider-network-keychain",
    "open-activate-focus",
    "filesystem-owned",
    "startup-before-initializers",
];
// Concrete missing integration, not a receipt option: the complete endpoint
// must route native observe/config-location resolution and every mutation/
// reload through typed input qualification before original reads/writes.
pub const SOURCE_ROUTES_QUALIFIED: bool = false;
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SourceCallsite {
    pub file: &'static str,
    pub source_sha256: String,
    pub operation: &'static str,
}
#[derive(Clone, Debug)]
pub struct ObjcReplacement {
    pub class: &'static CStr,
    pub selector: &'static CStr,
    pub class_method: bool,
    pub original: usize,
    pub replacement: usize,
    pub provenance: metadata::ReplacementProvenance,
}
impl ObjcReplacement {
    #[cfg(test)]
    fn selected(&self) -> Result<usize> {
        metadata::with_verification_roster(|roster| self.selected_with_roster(roster))
    }

    fn selected_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<usize> {
        let class = roster.loaded_class(self.class)?;
        let selected = metadata::method(class, self.selector, self.class_method)?
            .ok_or("attested selector missing")?;
        if !self
            .provenance
            .matches_key(class, self.selector, self.class_method)
            || selected.implementation() != self.replacement
            || self.provenance.original() != self.original
            || self.provenance.replacement() != self.replacement
            || self.provenance.selected_owner() != selected.owner()
        {
            return Err("attested raw metadata identity differs".into());
        }
        self.provenance.verify_with_roster(roster)
    }
}
#[derive(Clone)]
pub struct ModuleAttestation {
    pub module: &'static str,
    pub scopes: BTreeSet<&'static str>,
    pub callsites: BTreeSet<SourceCallsite>,
    pub imports: Vec<ImportReplacement>,
    pub objc: Vec<ObjcReplacement>,
    pub handles: Vec<owned::OwnedHandleLease>,
    /// Existing published SDK factories carry their typed constructor,
    /// owned-declaration and memory proofs without fabricating original IMPs.
    pub workspace_factories: Option<crate::workspace::factory::PublishedWorkspaceFactories>,
}
impl ModuleAttestation {
    pub(crate) fn verify(&self) -> Result<()> {
        if self.callsites.is_empty() || self.handles.is_empty() {
            return Err("missing source roster/owned leases".into());
        }
        for h in &self.handles {
            h.before_effect()?;
        }
        match (&self.workspace_factories, self.module) {
            (Some(_), "workspace") => crate::workspace::objc::verify_attestation(self)?,
            (None, "bootstrap") => (),
            (None, "private-security") => private_network::verify_attestation(self)?,
            (None, "process-monitor") => process_router::verify_attestation(self)?,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            (None, "reference-carbon") => input_carbon::verify_reference_attestation(self)?,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            (None, "reference-hid") => input_hid::verify_reference_attestation(self)?,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            (None, "reference-events") => input_events::verify_reference_attestation(self)?,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            (None, "reference-input-prerequisites") => input_permissions_caps::verify_reference_attestation(self)?,
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            (None, "reference-accessibility") => accessibility::abi::verify_reference_attestation(self)?,
            _ => return Err("module/factory attestation kind differs".into()),
        }
        for i in &self.imports {
            if i.selected_address() != i.replacement {
                return Err(format!("selected import differs: {:?}", i.symbol));
            }
        }
        // Only this final metadata-only pass shares the class roster. The
        // module-specific validators above can invoke guards and retain their
        // own fresh observations.
        metadata::with_verification_roster(|roster| {
            for i in &self.objc {
                if i.original == 0
                    || i.replacement == 0
                    || i.original == i.replacement
                    || i.selected_with_roster(roster)? != i.replacement
                {
                    return Err(format!("selected IMP differs: {:?}", i.selector));
                }
            }
            Ok(())
        })
    }
}
pub struct Registry {
    modules: BTreeMap<&'static str, ModuleAttestation>,
    life: Arc<dyn AliveLease>,
    retired: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    #[cfg(rcf1_reference_sdk)]
    reference_admission:Option<Arc<core_preentry::ReferenceAdmission>>,
}
/// A lease on this actual Registry's monotone retirement bit. Only a virgin
/// Registry and the genuine Host admission can issue it; no JSON/bool constructor.
pub struct NativeInputRegistryToken {
    retired: Arc<AtomicBool>,
    life: Arc<dyn AliveLease>,
    descriptors: Arc<descriptors::DescriptorTable>,
    owner: std::thread::ThreadId,
}
impl Registry {
    pub fn new(life: Arc<dyn AliveLease>) -> Self {
        Self {
            modules: BTreeMap::new(),
            life,
            retired: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            #[cfg(rcf1_reference_sdk)]
            reference_admission:None,
        }
    }
    /// The Reference actor assembler creates this ONE registry before any
    /// actors, and moves that same value into publication. This binds an actual
    /// sealed role, not a Native Input token or a callback/source authority.
    #[cfg(rcf1_reference_sdk)]
    pub fn new_reference(admission:Arc<core_preentry::ReferenceAdmission>)->Result<Self> {
        admission.before_reference_initialization()?;
        let mut registry=Self::new(admission.life());
        admission.bind_registry_retirement(&registry.retired)?;
        registry.reference_admission=Some(admission.clone());
        registry.verify_reference_issuer(&admission)?;
        Ok(registry)
    }
    /// Setup/publication check only. Invoke outside any Registry mutex because
    /// sealed admission validation includes real dynamic guardian boundaries.
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_reference_issuer(&self,admission:&Arc<core_preentry::ReferenceAdmission>)->Result<()> {
        if !self.reference_admission.as_ref().is_some_and(|actual|Arc::ptr_eq(actual,admission))
            || !Arc::ptr_eq(&self.life,&admission.life()) || self.retired.load(Ordering::Acquire)
            || READY_RETIRED.load(Ordering::Acquire) {
            return Err("Reference actors require the same sealed Registry/admission issuer".into());
        }
        admission.before_reference_initialization()?;
        if self.retired.load(Ordering::Acquire) || READY_RETIRED.load(Ordering::Acquire) {
            return Err("Reference Registry retired during setup verification".into());
        }
        Ok(())
    }
    /// Borrow the actual installed Bootstrap context from this same early
    /// Reference Registry. This prepares roots/ownership only; Swift callback
    /// code, userdata lifetime and registration authority remain separate.
    #[cfg(rcf1_reference_sdk)]
    pub fn reference_input_context(&self,admission:&Arc<core_preentry::ReferenceAdmission>)->Result<owned::ModuleContext> {
        struct RetireOnFailure<'a> {retired:&'a AtomicBool,complete:bool}
        impl Drop for RetireOnFailure<'_> {
            fn drop(&mut self) {if !self.complete {self.retired.store(true,Ordering::Release);}}
        }
        let mut failure=RetireOnFailure{retired:&self.retired,complete:false};
        self.verify_reference_issuer(admission)?;
        let context=bootstrap::owned_module_context(&admission.life(),&admission.descriptors())?;
        if self.retired.load(Ordering::Acquire) || READY_RETIRED.load(Ordering::Acquire) {
            return Err("Reference input Registry retired while borrowing Bootstrap context".into());
        }
        admission.verify_input_context(&context)?;
        if self.retired.load(Ordering::Acquire) || READY_RETIRED.load(Ordering::Acquire) {
            return Err("Reference input Registry retired during context verification".into());
        }
        failure.complete=true;
        Ok(context)
    }
    pub fn native_input_retirement(
        &mut self,
        admission: &core_preentry::HostAdmission,
    ) -> Result<NativeInputRegistryToken> {
        #[cfg(rcf1_reference_sdk)]
        if self.reference_admission.is_some() {return Err("Reference Registry cannot issue a Native Input lease".into());}
        admission.verify_native_input_issuer()?;
        if !Arc::ptr_eq(&self.life,&admission.life()) || !self.modules.is_empty()
            || self.retired.load(Ordering::Acquire) || Arc::get_mut(&mut self.retired).is_none()
            || REGISTRY.get().is_some() || READY_RETIRED.load(Ordering::Acquire) {
            return Err("Native Input issuer requires the same virgin live Registry".into());
        }
        Ok(NativeInputRegistryToken {
            retired:self.retired.clone(),life:self.life.clone(),descriptors:admission.descriptors(),
            owner:std::thread::current().id(),
        })
    }
    pub fn register(&mut self, module: ModuleAttestation) -> Result<()> {
        #[cfg(rcf1_reference_sdk)]
        struct ReferenceRegistrationFailure { retired:Arc<AtomicBool>,complete:bool }
        #[cfg(rcf1_reference_sdk)]
        impl Drop for ReferenceRegistrationFailure {
            fn drop(&mut self) {if !self.complete {self.retired.store(true,Ordering::Release);}}
        }
        #[cfg(rcf1_reference_sdk)]
        let reference=self.reference_admission.clone();
        #[cfg(rcf1_reference_sdk)]
        let mut failure=reference.as_ref().map(|_|ReferenceRegistrationFailure{retired:self.retired.clone(),complete:false});
        #[cfg(rcf1_reference_sdk)]
        if let Some(admission)=&reference {self.verify_reference_issuer(admission)?;}
        #[cfg(rcf1_reference_sdk)]
        let sealed_reference = reference.is_some();
        #[cfg(not(rcf1_reference_sdk))]
        let sealed_reference = false;
        if !sealed_reference {
            module.verify()?;
        }
        // Module ownership is fixed by source; typed owned factory proofs are
        // not ordinary replacements and never relax original!=0 above.
        match module.module {
            "bootstrap" => {
                if module.workspace_factories.is_some()
                    || module.scopes != bootstrap::scopes()
                    || module.callsites != bootstrap::source_roster() {
                    return Err("unknown/incomplete bootstrap source module".into());
                }
        let required=bootstrap::required_imports();
        #[cfg(rcf1_reference_sdk)]
        let required=if reference.is_some(){bootstrap::reference_attested_imports()}else{required};
        let expected: BTreeSet<_> = required
            .into_iter()
            .map(|n| n.to_bytes())
            .collect();
        let actual: BTreeSet<_> = module.imports.iter().map(|i| i.symbol.to_bytes()).collect();
        if actual != expected
            || module
                .imports
                .iter()
                .map(|i| (i.symbol, i.cell_address()))
                .collect::<BTreeSet<_>>()
                .len()
                != module.imports.len()
            || module
                .imports
                .iter()
                .any(|i| bootstrap::replacement(i.symbol) != Some(i.replacement))
        {
            return Err("missing/duplicate/incorrect bootstrap import leaf".into());
        }
        let expected: BTreeSet<_> = bootstrap::required_selectors()?.into_iter().collect();
        let actual: BTreeSet<_> = module
            .objc
            .iter()
            .map(|i| (i.class, i.selector, i.class_method))
            .collect();
        if actual != expected
            || actual.len() != module.objc.len()
            || module.objc.iter().any(|i| {
                bootstrap::selector_replacement(i.class, i.selector, i.class_method)
                    != Some(i.replacement)
            })
        {
            return Err("missing/duplicate/incorrect bootstrap selector leaf".into());
        }
            }
            "workspace" => {
                if module.scopes != crate::workspace::scopes()
                    || module.callsites != crate::workspace::source_roster() {
                    return Err("incomplete workspace source module".into());
                }
            }
            "private-security" | "process-monitor" => (),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            "reference-carbon" | "reference-hid" | "reference-events"
            | "reference-input-prerequisites" | "reference-accessibility" => (),
            _ => return Err("unknown/incomplete source module".into()),
        }
        if self.modules.contains_key(module.module)
            || self.modules.values().any(|m| {
                !m.scopes.is_disjoint(&module.scopes)
                    || m.imports
                        .iter()
                        .any(|i| module.imports.iter().any(|j| i.symbol == j.symbol))
                    || m.objc.iter().any(|i| {
                        module.objc.iter().any(|j| {
                            (i.class, i.selector, i.class_method)
                                == (j.class, j.selector, j.class_method)
                        })
                    })
            })
        {
            return Err("duplicate scope/API ownership".into());
        }
        if !SOURCE_ROUTES_QUALIFIED {
            // Only the actual sealed Reference assembler may collect modules
            // before readiness. A plain Registry, even in that build, cannot.
            #[cfg(not(rcf1_reference_sdk))]
            return Err("bootstrap endpoint/watcher input qualification is not fully wired; no SDK scope admitted".into());
            #[cfg(rcf1_reference_sdk)]
            if reference.is_none() {
                return Err("bootstrap endpoint/watcher input qualification is not fully wired; no SDK scope admitted".into());
            }
        }
        #[cfg(rcf1_reference_sdk)]
        if let Some(admission)=&reference {
            self.verify_reference_issuer(admission)?;
            module.verify()?;
            // Final callback-free fence before insertion. Neither module
            // collection nor its generation publishes the Registry/Ready.
            if self.retired.load(Ordering::Acquire)||READY_RETIRED.load(Ordering::Acquire) {
                return Err("Reference Registry retired during module assembly".into());
            }
        }
        let next=self.generation.load(Ordering::Acquire).checked_add(1).ok_or("Registry generation exhausted")?;
        self.modules.insert(module.module, module);
        self.generation.store(next,Ordering::Release);
        #[cfg(rcf1_reference_sdk)]
        if let Some(failure)=&mut failure {failure.complete=true;}
        Ok(())
    }
    pub fn ready(&self) -> u64 {
        if !SOURCE_ROUTES_QUALIFIED || self.retired.load(Ordering::Acquire) {return 0;}
        self.ready_snapshot().ready()
    }
    fn ready_snapshot(&self)->ReadySnapshot {
        ReadySnapshot {
            modules:self.modules.values().cloned().collect(),life:self.life.clone(),retired:self.retired.clone(),
            generation:self.generation.clone(),expected_generation:self.generation.load(Ordering::Acquire),
        }
    }
}
/// One verification only: copied under the Registry lock, then all dynamic
/// guardian/module callbacks run after that lock is released. Existing proofs
/// retain their actual model/descriptors/identity; this is no cache or grant.
struct ReadySnapshot {
    modules:Vec<ModuleAttestation>,
    life:Arc<dyn AliveLease>,
    retired:Arc<AtomicBool>,
    generation:Arc<AtomicU64>,
    expected_generation:u64,
}
thread_local! {
    static READY_ENTERED: std::cell::RefCell<Option<Arc<AtomicBool>>> = const {std::cell::RefCell::new(None)};
}
struct ReadyEntry { retired:Arc<AtomicBool>, completed:bool }
impl ReadyEntry {
    fn enter(retired:&Arc<AtomicBool>)->Option<Self> {
        READY_ENTERED.with(|slot| {
            let mut slot=slot.borrow_mut();
            if let Some(active)=slot.as_ref() {
                active.store(true,Ordering::Release);
                retired.store(true,Ordering::Release);
                return None;
            }
            *slot=Some(retired.clone());
            Some(Self{retired:retired.clone(),completed:false})
        })
    }
}
impl Drop for ReadyEntry {
    fn drop(&mut self) {
        if !self.completed {self.retired.store(true,Ordering::Release);}
        READY_ENTERED.with(|slot| {
            let mut slot=slot.borrow_mut();
            if slot.as_ref().is_some_and(|active|Arc::ptr_eq(active,&self.retired)) {slot.take();}
            else {self.retired.store(true,Ordering::Release);}
        });
    }
}
impl ReadySnapshot {
    fn current(&self)->bool {
        if self.generation.load(Ordering::Acquire)!=self.expected_generation {self.retired.store(true,Ordering::Release);}
        !self.retired.load(Ordering::Acquire) && !READY_RETIRED.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire)==self.expected_generation
    }
    fn checked_callback(&self,callback:impl FnOnce()->Result<()>)->Result<()> {
        if !self.current() {return Err("Registry snapshot retired before verifier".into());}
        callback()?;
        if !self.current() {return Err("Registry snapshot retired during verifier".into());}
        Ok(())
    }
    fn callback_check(&self,verify:impl FnOnce()->Result<bool>)->bool {
        if self.retired.load(Ordering::Acquire) {return false;}
        if self.generation.load(Ordering::Acquire)!=self.expected_generation {self.retired.store(true,Ordering::Release);return false;}
        let Some(mut entered)=ReadyEntry::enter(&self.retired) else{return false;};
        let result=verify();
        if result.is_err() {self.retired.store(true,Ordering::Release);}
        if self.generation.load(Ordering::Acquire)!=self.expected_generation {self.retired.store(true,Ordering::Release);}
        entered.completed=true;
        drop(entered);
        result.unwrap_or(false) && !self.retired.load(Ordering::Acquire)
            && !READY_RETIRED.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire)==self.expected_generation
    }
    fn ready(&self)->u64 {
        if !SOURCE_ROUTES_QUALIFIED {
            return 0;
        }
        let verified=self.callback_check(|| {
            self.checked_callback(||if self.life.is_alive(){Ok(())}else{Err("Registry guardian retired".into())})?;
            self.checked_callback(||self.life.before_effect())?;
            let scopes:BTreeSet<_>=self.modules.iter().flat_map(|m|m.scopes.iter().copied()).collect();
            for module in &self.modules {self.checked_callback(||module.verify())?;}
            Ok(scopes==SCOPES.into_iter().collect())
        });
        if !verified || !SOURCE_ROUTES_QUALIFIED {
            return 0;
        }
        0xF100002600003FFF
    }
}

#[cfg(test)]
mod ready_snapshot_tests {
    use super::*;
    struct Live;
    impl AliveLease for Live {fn is_alive(&self)->bool{true}fn before_effect(&self)->Result<()>{Ok(())}}
    fn own()->Arc<Mutex<Registry>> {Arc::new(Mutex::new(Registry::new(Arc::new(Live))))}
    #[test]
    fn snapshot_releases_actual_registry_mutex_before_dynamic_verifier_and_reentry_retires_outer_ok() {
        let registry=own();
        let snapshot=registry.lock().unwrap().ready_snapshot();
        let called=AtomicBool::new(false);
        let accepted=snapshot.callback_check(|| {
            let nested=registry.try_lock().expect("actual Registry mutex must be released").ready_snapshot();
            assert!(!nested.callback_check(||{called.store(true,Ordering::Release);Ok(true)}));
            Ok(true) // Nested refusal cannot be hidden by outer success.
        });
        assert!(!accepted);assert!(!called.load(Ordering::Acquire));
        assert!(registry.lock().unwrap().retired.load(Ordering::Acquire));
        assert_eq!(registry.lock().unwrap().ready(),0); // No Sourceflag flip.
    }
    #[test]
    fn changed_attestation_generation_and_unwinding_verifier_cannot_reanimate_snapshot() {
        let registry=own();
        let snapshot=registry.lock().unwrap().ready_snapshot();
        assert!(!snapshot.callback_check(|| {
            // The actual checked generation used by register never rolls back,
            // even if a caller replaces then restores an attestation's contents.
            registry.try_lock().unwrap().generation.fetch_add(2,Ordering::AcqRel);
            Ok(true)
        }));
        assert!(registry.lock().unwrap().retired.load(Ordering::Acquire));
        let other=own();let snapshot=other.lock().unwrap().ready_snapshot();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(||snapshot.callback_check(||panic!("owned verifier panic")))).is_err());
        assert!(other.lock().unwrap().retired.load(Ordering::Acquire));
        // Unwind releases the exact TLS lease; an unrelated fresh actual
        // Registry can verify its helper, without claiming fourteen scopes.
        let fresh=own();let snapshot=fresh.lock().unwrap().ready_snapshot();
        assert!(snapshot.callback_check(||Ok(true)));
        assert_eq!(fresh.lock().unwrap().ready(),0);
    }
    #[test]
    fn nested_refusal_returning_ok_stops_before_the_next_dynamic_verifier() {
        let registry=own();let snapshot=registry.lock().unwrap().ready_snapshot();
        let second=AtomicBool::new(false);
        let first=||->Result<()> {
            let nested=registry.try_lock().unwrap().ready_snapshot();
            assert!(!nested.callback_check(||Ok(true)));
            Ok(())
        };
        let next=||->Result<()> {second.store(true,Ordering::Release);Ok(())};
        let callbacks:[&dyn Fn()->Result<()>;2]=[&first,&next];
        assert!(!snapshot.callback_check(|| {
            for callback in callbacks {snapshot.checked_callback(callback)?;}
            Ok(true)
        }));
        assert!(!second.load(Ordering::Acquire));
        assert!(registry.lock().unwrap().retired.load(Ordering::Acquire));
        assert_eq!(registry.lock().unwrap().ready(),0);
    }
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static READY_RETIRED: AtomicBool = AtomicBool::new(false);
pub fn publish_registry(registry: Registry) -> Result<()> {
    REGISTRY
        .set(Mutex::new(registry))
        .map_err(|_| "registry already published".into())
}
#[no_mangle]
pub extern "C" fn ruycast_f1_preentry_ready_v1() -> u64 {
    if !SOURCE_ROUTES_QUALIFIED || READY_RETIRED.load(Ordering::Acquire) {
        return 0;
    }
    match std::panic::catch_unwind(|| {
        REGISTRY
            .get()
            .and_then(|r| r.lock().ok().map(|registry|registry.ready_snapshot()))
            .map_or(0, |snapshot| snapshot.ready())
    }) {
        Ok(ready) => ready,
        Err(_) => {
            READY_RETIRED.store(true, Ordering::Release);
            0
        }
    }
}

#[cfg(test)]
mod readiness_boundary_tests {
    use super::*;
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! { static COUNT_THIS_THREAD: Cell<bool> = const { Cell::new(false) }; }
    static ALLOCATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct Counter;
    fn allocated() {
        if COUNT_THIS_THREAD.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
    }
    unsafe impl GlobalAlloc for Counter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            allocated();
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            allocated();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            allocated();
            unsafe { System.realloc(pointer, layout, size) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: Counter = Counter;
    static CALLBACKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct OwnPanicLease;
    impl AliveLease for OwnPanicLease {
        fn is_alive(&self) -> bool {
            CALLBACKS.fetch_add(1, Ordering::SeqCst);
            panic!("unqualified readiness called guardian");
        }
        fn before_effect(&self) -> Result<()> {
            CALLBACKS.fetch_add(1, Ordering::SeqCst);
            panic!("unqualified readiness called guardian");
        }
    }
    #[test]
    fn unqualified_ready_refuses_before_dynamic_guardian_or_registry_lock() {
        let registry = Registry::new(Arc::new(OwnPanicLease));
        assert_eq!(registry.ready(), 0);
        publish_registry(registry).unwrap();
        // Deliberately hold this own registry lock: unqualified C admission may
        // not consult it or invoke its dynamic guardian at all.
        let _held = REGISTRY.get().unwrap().lock().unwrap();
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        COUNT_THIS_THREAD.with(|count| count.set(true));
        let native = _held.ready();
        let cabi = ruycast_f1_preentry_ready_v1();
        COUNT_THIS_THREAD.with(|count| count.set(false));
        assert_eq!(native, 0);
        assert_eq!(cabi, 0);
        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), before);
        assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod raw_metadata_attestation_tests {
    use super::*;
    use std::{
        ffi::{c_char, c_void},
        sync::atomic::AtomicUsize,
    };
    static INITIALIZERS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn initialize(_: *const c_void, _: *const c_void) {
        INITIALIZERS.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn before(_: *const c_void, _: *const c_void) -> usize {
        1
    }
    unsafe extern "C" fn after(_: *const c_void, _: *const c_void) -> usize {
        2
    }
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_allocateClassPair(
            parent: *const c_void,
            name: *const c_char,
            extra: usize,
        ) -> *mut c_void;
        fn objc_registerClassPair(class: *const c_void);
        fn object_getClass(object: *const c_void) -> *const c_void;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn class_addMethod(
            class: *const c_void,
            selector: *const c_void,
            imp: *const c_void,
            types: *const c_char,
        ) -> u8;
        #[link_name = "class_replaceMethod"]
        fn replace(
            class: *const objc2::runtime::AnyClass,
            selector: objc2::runtime::Sel,
            imp: objc2::runtime::Imp,
            types: *const c_char,
        ) -> Option<objc2::runtime::Imp>;
    }
    #[test]
    fn own_raw_metadata_attestation_selection_is_nonresolving_and_fail_closed() {
        let base = metadata::loaded_class(c"NSObject").unwrap();
        let p = unsafe {
            objc_allocateClassPair(
                base.address() as *const c_void,
                c"RuycastRawMetadataAttestation".as_ptr(),
                0,
            )
        };
        assert!(!p.is_null());
        let meta = unsafe { object_getClass(p) };
        assert_ne!(
            unsafe {
                class_addMethod(
                    meta,
                    sel_registerName(c"initialize".as_ptr()),
                    initialize as *const () as *const c_void,
                    c"v@:".as_ptr(),
                )
            },
            0
        );
        assert_ne!(
            unsafe {
                class_addMethod(
                    p,
                    sel_registerName(c"ownAttested".as_ptr()),
                    before as *const () as *const c_void,
                    c"Q@:".as_ptr(),
                )
            },
            0
        );
        unsafe { objc_registerClassPair(p) };
        let child =
            unsafe { objc_allocateClassPair(p, c"RuycastRawMetadataAttestationChild".as_ptr(), 0) };
        assert!(!child.is_null());
        unsafe { objc_registerClassPair(child) };
        let class = metadata::registered_class(p as usize).unwrap();
        let baseline = metadata::method(class, c"ownAttested", false)
            .unwrap()
            .unwrap();
        let lifecycle = metadata::lifecycle(class).unwrap();
        unsafe {
            replace(
                p.cast(),
                objc2::sel!(ownAttested),
                std::mem::transmute::<usize, objc2::runtime::Imp>(after as *const () as usize),
                c"Q@:".as_ptr(),
            );
        }
        let selected = metadata::method(class, c"ownAttested", false)
            .unwrap()
            .unwrap();
        let provenance = metadata::ReplacementProvenance::bind(
            baseline.clone(),
            baseline,
            selected,
            lifecycle,
            false,
            None,
        )
        .unwrap();
        let mut record = ObjcReplacement {
            class: c"RuycastRawMetadataAttestation",
            selector: c"ownAttested",
            class_method: false,
            original: before as *const () as usize,
            replacement: after as *const () as usize,
            provenance,
        };
        assert_eq!(record.selected().unwrap(), record.replacement);
        record.class = c"RuycastRawMetadataAttestationChild";
        assert!(record.selected().is_err());
        record.class = c"RuycastRawMetadataAttestation";
        record.selector = c"missingOwnedAttestation";
        assert!(record.selected().is_err());
        record.selector = c"ownAttested";
        record.class = c"MissingOwnedAttestationClass";
        assert!(record.selected().is_err());
        record.class = c"RuycastRawMetadataAttestation";
        record.class_method = true;
        assert!(record.selected().is_err());
        assert_eq!(INITIALIZERS.load(Ordering::SeqCst), 0);
        struct Life;
        impl AliveLease for Life {
            fn is_alive(&self) -> bool {
                true
            }
            fn before_effect(&self) -> Result<()> {
                Ok(())
            }
        }
        assert_eq!(Registry::new(Arc::new(Life)).ready(), 0);
    }
}
