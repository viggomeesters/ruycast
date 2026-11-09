//! Fixed Reference SDK consumer. No constructor or loader lives in this module.
//! The explicit Main preentry calls the source-built SDK export; the genuine
//! application CF loop installs the main owner only after DidFinishLaunching.
use super::{core_preentry::ReferenceAdmission, owned::Result};
use ruycast_native::native_f1::reference::{Guard, ProcessProvider, Reply, SourceCodeReader};
use std::{ffi::CString, sync::Arc};

/// Actual sealed admission and its retained private roots/guardian/code images.
/// No header/receipt or independently reopened root constructs this issuer.
pub(crate) struct ReferenceReader {
    admission: Arc<ReferenceAdmission>,
}
impl ReferenceReader {
    pub(crate) fn published(admission: Arc<ReferenceAdmission>) -> Result<Arc<Self>> {
        admission.before_reference_publication()?;
        Ok(Arc::new(Self { admission }))
    }
}
// SAFETY: the sole constructor above retains the actual sealed admission. Its
// main/worker predicates requalify the exact source/images/guardian/Registry;
// its table getter establishes the complete RO216 and fixed25 typed RX slots
// BEFORE returning a pointer. MAIN and initial inserted SDK are process-lifetime
// images, with no dlopen/unload/replacement path in this source-built runtime.
// Sourcefalse/unknown/retired/busy states refuse; no default success exists.
unsafe impl SourceCodeReader for ReferenceReader {
    fn qualified_main_table(
        &self,
        guard: Guard,
        reply: Reply,
        provider: ProcessProvider,
    ) -> Result<usize> {
        self.admission.before_reference_publication()?;
        let sdk = self.admission.qualified_sdk_addresses()?;
        let expected = [
            ("guard", guard as *const () as usize),
            ("reply", reply as *const () as usize),
            ("process_provider", provider as *const () as usize),
        ];
        if sdk[1..4] != expected {
            return Err("Reference typed SDK guard/reply/provider differ from sealed slots".into());
        }
        // This fresh full check is LAST: no callback follows it before the
        // Native consumer copies the already-qualified immutable typed table.
        self.admission.qualified_table_address()
    }
    fn qualified_profile(&self) -> Result<CString> {
        CString::new(self.admission.profile()?.as_os_str().as_encoded_bytes())
            .map_err(|_| "Reference sealed profile contains NUL".into())
    }
    fn qualified_fixture_epoch(&self) -> Result<u64> {
        self.admission.before_reference_publication()?;
        let epoch = self.admission.fixture_epoch_unix_seconds()?;
        self.admission.before_reference_publication()?;
        Ok(epoch)
    }
    fn qualified_process_ranking(&self) -> Result<Vec<u8>> {
        // Core binds the one fixed profile leaf, actual owned descriptor,
        // bounded read and pre/post fresh admission plus identity checks.
        self.admission.process_ranking_bytes()
    }
    fn begin_field_input(&self,bridge:&native_f1::reference::Bridge,query:&str,epoch:u64)->Result<bool> {
        #[cfg(all(not(test),target_arch="aarch64"))]
        {
            self.before_main_call()?;
            let (root,state)=MAIN.with(|slot|->Result<_>{let slot=slot.try_borrow().map_err(|_|"Reference field custody slot busy")?;
                Ok((slot.root.clone().ok_or("Reference field root absent")?,slot.state.clone().ok_or("Reference field state absent")?))})?;
            let runtime=RUNTIME.get().ok_or("Reference field runtime absent")?;
            root.verify_reference_binding(&self.admission,&runtime.workspace)?;
            state.verify_reference_binding(&self.admission,&runtime.workspace)?;
            let host=bridge.content_view_for(self)?;
            crate::workspace::objc::verify_reference_mounted_palette_host(
                &runtime.workspace, host.as_ptr() as usize, &runtime.stopped,
            )?;
            // The owner attempts confirmed cancellation after failed entry.
            // Keep any published custody available for that cleanup.
            crate::workspace::receiver::app_delegate::constructor::begin_reference_field(
                root,state,host.as_ptr() as usize,query,epoch,&runtime.stopped,
            )
        }
        #[cfg(any(test,not(target_arch="aarch64")))]
        {let _=(bridge,query,epoch);Err("Original reference field unavailable on this target".into())}
    }
    fn verify_field_input(&self,bridge:&native_f1::reference::Bridge,epoch:u64)->Result<()> {
        #[cfg(all(not(test),target_arch="aarch64"))]
        {self.before_main_call()?;let host=bridge.content_view_for(self)?;
            let runtime = RUNTIME.get().ok_or("Reference field runtime absent")?;
            crate::workspace::objc::verify_reference_mounted_palette_host(
                &runtime.workspace, host.as_ptr() as usize, &runtime.stopped,
            )?;
            crate::workspace::receiver::app_delegate::constructor::verify_reference_field(host.as_ptr() as usize,epoch)}
        #[cfg(any(test,not(target_arch="aarch64")))]
        {let _=(bridge,epoch);Err("Original reference field unavailable on this target".into())}
    }
    #[cfg(all(not(test), target_arch = "aarch64"))]
    fn field_delegate_not_entered(&self, bridge: &native_f1::reference::Bridge, epoch: u64) -> Result<bool> {
        self.before_main_call()?;
        let host = bridge.content_view_for(self)?;
        crate::workspace::receiver::app_delegate::constructor::field_delegate_not_entered(
            host.as_ptr() as usize, epoch,
        )
    }
    fn retire_field_input(&self,bridge:&native_f1::reference::Bridge,epoch:u64,confirmed:bool)->Result<()> {
        #[cfg(all(not(test),target_arch="aarch64"))]
        {
            if unsafe{libc::pthread_main_np()}!=1{return Err("Reference field retirement off main thread".into());}
            if !confirmed{
                crate::workspace::receiver::app_delegate::constructor::quarantine_reference_field(epoch);
                return Err("Reference field quarantined after unconfirmed completion/cancellation".into());
            }
            self.before_main_call()?;
            let host = bridge.content_view_for(self)?;
            crate::workspace::receiver::app_delegate::constructor::retire_reference_field(host.as_ptr() as usize, epoch)
        }
        #[cfg(any(test,not(target_arch="aarch64")))]
        {let _=(bridge,epoch,confirmed);Err("Original reference field unavailable on this target".into())}
    }
    #[cfg(all(not(test), target_arch = "aarch64"))]
    unsafe fn release_borrowed_field_box(
        &self,
        object: std::ptr::NonNull<std::ffi::c_void>,
        drop_box: unsafe extern "C" fn(*mut std::ffi::c_void),
    ) -> Result<()> {
        let runtime = RUNTIME.get().ok_or("Reference field release runtime absent")?;
        if !std::ptr::eq(self, runtime.reader.as_ref()) || !runtime.field_probe
            || !runtime.stopped.load(Ordering::Acquire) || !runtime.worker_done.load(Ordering::Acquire)
        {
            return Err("Reference field release issuer/phase differs".into());
        }
        let state = MAIN.with(|slot| -> Result<_> {
            let slot = slot.try_borrow().map_err(|_| "Reference field release owner busy")?;
            if slot.entered || slot.owner.is_some() {
                return Err("Reference field release requires the removed owner".into());
            }
            slot.state.clone().ok_or_else(|| "Reference field release state absent".into())
        })?;
        self.before_main_call()?;
        let table = self.admission.qualified_table_address()?;
        // The immutable typed table has already been checked against Main's
        // source pins. Its fourth zero-based function slot is rcf1_drop.
        if unsafe { ((table + 16 + 4 * size_of::<usize>()) as *const usize).read() }
            != drop_box as *const () as usize
        {
            return Err("Reference field release function differs".into());
        }
        unsafe { state.verify_borrowed_reference_box(object.as_ptr() as usize)?; }
        crate::workspace::receiver::app_delegate::constructor::verify_reference_field_drained(true)?;
        native_f1::reference::verify_reply_contexts_drained()?;
        state.verify_held_epoch()?;
        FIELD_BOX_DROP.with(|permit| {
            if permit.get().is_some() {
                return Err("Reference field box release reentered".into());
            }
            let call_site = (drop_box as *const () as usize)
                .checked_add(0xb0).ok_or("Reference field drop return PC overflow")?;
            let box_address = object.as_ptr() as usize;
            permit.set(Some((call_site, box_address, false)));
            // Only this exact admitted foreign call runs with the one-shot
            // guard armed. Its body requests operation 2 before reading the box.
            unsafe { drop_box(object.as_ptr()) };
            if permit.replace(None) != Some((call_site, box_address, true)) {
                unsafe { libc::_exit(78) };
            }
            Ok(())
        })
    }
    fn before_main_call(&self) -> Result<()> {
        self.admission.before_reference_publication()
    }
    fn before_worker_callback(&self) -> Result<()> {
        // This separate predicate exposes neither MAIN table/delegate getters
        // nor an original Main-call permission to the worker.
        self.admission.before_reference_worker_callback()
    }
}

/// Construct the real existing bootstrap from precisely the admitted roots and
/// actual descriptor ownership. This is ONE module, never a fourteen-scope
/// registry/Ready shortcut; workspace/callback/source actors must follow.
#[cfg(not(test))]
pub(crate) fn initialize_bootstrap(
    admission: &Arc<ReferenceAdmission>,
    prepare_workspace: impl FnOnce() -> Result<Arc<Workspace>>,
) -> Result<(Arc<Workspace>, super::ModuleAttestation)> {
    let [home, cache, temp, applications] = admission.runtime_roots()?;
    let (uid, gid) = home.owner_ids()?;
    let account =
        super::bootstrap::Account::fixture(uid, gid, "f1-owned", "Owned Fixture", home.path())?;
    let context = super::bootstrap::Bootstrap::new(
        home,
        cache,
        temp,
        applications,
        account,
        admission.life(),
    )?;
    admission.initialize_bootstrap(context, prepare_workspace)
}

use crate::workspace::{
    event_loop::{Engine, Phase},
    Workspace,
};
use ruycast_native::native_f1::{
    self,
    reference::{Bridge, QualifiedFunctions, ReferenceOwner},
    ReadyBindings,
};
use std::{
    cell::RefCell,
    os::fd::BorrowedFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread::JoinHandle,
};

/// Results of the source-built actor/catalog assembler, not a receipt model.
/// Every module is reverified by the existing Registry before publication.
pub struct ReferenceActors {
    // First field: retirement precedes dropping actors on an unpublished path.
    setup: ReferenceSetup,
    pub workspace: Arc<Workspace>,
    pub registry: super::Registry,
    field_probe: bool,
}

/// Failure fence for the actual early Registry, not another issuer or lease.
struct ReferenceSetup {
    retired: Arc<AtomicBool>,
    published: bool,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    sources: Option<super::reference_source_actors::SetupFailure>,
}
impl Drop for ReferenceSetup {
    fn drop(&mut self) {
        if !self.published {
            self.retired.store(true, Ordering::Release);
        }
    }
}

#[cfg(all(not(test), target_arch = "aarch64"))]
fn retire_input_actors(admission: &Arc<ReferenceAdmission>, model: &Arc<Workspace>) -> Result<()> {
    [
        super::input_hid::retire_reference(admission, model),
        super::input_events::retire_reference(admission, model),
        super::input_permissions_caps::retire_reference(admission, model),
        super::accessibility::abi::retire_reference(admission, model),
    ].into_iter().collect()
}

/// Plain fixture values from the fixed source-built producer. They confer no
/// class, lifecycle, executable, callback or module authority.
pub struct ReferenceActorInputs {
    pub applications: Vec<crate::workspace::Application>,
    pub screens: Vec<crate::workspace::Screen>,
    pub mouse: crate::workspace::Point,
    pub history: Option<Vec<u8>>,
    pub entropy: Vec<u8>,
    pub browser_wire: Option<crate::workspace::objc::apple_events::Wire>,
    pub browser_consent: i32,
    pub open_fixture_url: Option<String>,
    pub field_probe: bool,
}

/// Actual installed source actors supplied by their existing SDK owners.
/// Module attestations are reverified by the SAME early Registry; the callback
/// authority must independently qualify the original Swift/generated lifecycle.
pub struct ReferenceSourceActors {
    #[cfg(all(not(test), target_arch = "aarch64"))]
    pub(super) rollback: super::reference_source_actors::SetupFailure,
    pub callback_catalog: Arc<crate::workspace::objc::application::SourceCallbackCatalog>,
    pub modules: Vec<super::ModuleAttestation>,
}

#[cfg(not(test))]
type ReferenceSourceMaker = fn(
    Arc<ReferenceAdmission>,
    Arc<Workspace>,
    &mut super::Registry,
    Option<Vec<u8>>,
    Vec<u8>,
) -> Result<ReferenceSourceActors>;

/// Fixed constructor pipeline for the existing explicit SDK entry. The private
/// export's two makers are ordinary compile-time Rust function paths, never
/// selected by a fixture flag, environment lookup or authority roster.
///
/// # Safety
/// Both makers and all effects/callbacks they install must have genuine exact
/// source/image/ABI/owned-root qualification. make_sources may bind actors to
/// the supplied actual early Registry but may neither publish nor register a
/// different Registry. It must supply actual remaining factory and provider
/// actors/attestations; it cannot manufacture the missing Swift issuer or copy
/// Native-only input authority into Reference. No original class is allocated
/// to discover its missing construction/lifecycle contract.
/// Each maker must qualify its own callback-safe partial-failure cleanup: the
/// caller's retirement fence cannot precede cleanup inside a failing maker.
#[cfg(not(test))]
pub unsafe fn assemble_reference(
    admission: Arc<ReferenceAdmission>,
    make_inputs: fn(&ReferenceAdmission) -> Result<ReferenceActorInputs>,
    make_sources: ReferenceSourceMaker,
) -> Result<ReferenceActors> {
    admission.before_reference_initialization()?;
    let inputs = make_inputs(&admission)?;
    admission.before_reference_initialization()?;
    // This is the one actual issuer/retirement domain for construction and
    // publication. Never rebuild it after actors have retained its identity.
    let mut registry = super::Registry::new_reference(admission.clone())?;
    let setup = ReferenceSetup {
        retired: registry.retired.clone(),
        published: false,
        #[cfg(target_arch = "aarch64")]
        sources: None,
    };
    let (workspace, bootstrap) = initialize_workspace_model(
        &admission,
        inputs.applications,
        inputs.screens,
        inputs.mouse,
    )?;
    registry.verify_reference_issuer(&admission)?;
    workspace.verify_reference_context(&admission)?;
    workspace.prepare_bundle_icons()?;
    #[cfg(target_arch = "aarch64")]
    let mut keychain_fixture = None;
    if let Some(url) = inputs.open_fixture_url {
        if inputs.field_probe {
            return Err("Reference Open fixture is unavailable in field mode".into());
        }
        workspace.set_open_result(url, crate::workspace::Outcome::Success)?;
        workspace.verify_reference_context(&admission)?;
        #[cfg(target_arch = "aarch64")]
        {
            keychain_fixture = Some(inputs.history.clone().ok_or("Reference assembly history fixture absent")?);
        }
    }

    let source = make_sources(
        admission.clone(),
        workspace.clone(),
        &mut registry,
        {
            #[cfg(target_arch = "aarch64")]
            if keychain_fixture.is_some() { None } else { inputs.history }
            #[cfg(not(target_arch = "aarch64"))]
            inputs.history
        },
        inputs.entropy,
    )?;
    // Transfer the original rollback owner before attempting workspace setup.
    #[cfg(target_arch = "aarch64")]
    let setup = {
        let mut setup = setup;
        setup.sources = Some(source.rollback);
        setup
    };
    registry.verify_reference_issuer(&admission)?;
    workspace.verify_reference_context(&admission)?;
    #[cfg(target_arch = "aarch64")]
    let website_fixture = keychain_fixture.is_some();
    #[cfg(target_arch = "aarch64")]
    if let Some(expected) = keychain_fixture {
        let security = source.modules.iter().find(|module| module.module == "private-security")
            .ok_or("Reference Security module absent before Keychain fixture")?;
        let owner = setup.sources.as_ref().ok_or("Reference provider rollback owner absent")?;
        owner.verify_keychain_fixture(&admission, security, &expected)?;
    }
    let workspace_module = unsafe {
        initialize_workspace_module(
            admission.clone(),
            workspace.clone(),
            source.callback_catalog,
        )
    }?;
    #[cfg(target_arch = "aarch64")]
    super::accessibility::abi::verify_reference_editor(&admission, &workspace)?;
    crate::workspace::objc::apple_events::initialize_reference_backend(
        &admission,&workspace,workspace_module.workspace_factories.as_ref().ok_or("Reference workspace factories absent before browser backend")?,
        inputs.browser_wire,inputs.browser_consent,
    )?;
    #[cfg(target_arch = "aarch64")]
    let modules = {
        // Workspace installation can select inherited lifecycle methods of
        // NSOperationQueue. Capture website provenance only after that write.
        let mut modules = source.modules;
        let security = modules.iter().position(|module| module.module == "private-security")
            .ok_or("Reference Security module absent before website install")?;
        let owner = setup.sources.as_ref().ok_or("Reference provider rollback owner absent")?;
        modules[security] = owner.install_website(admission.clone())?;
        if website_fixture {
            modules[security].verify()?;
            let monitor = modules.iter().find(|module| module.module == "process-monitor")
                .ok_or("Reference process monitor module absent before text fixture")?;
            super::system_providers::verify_original_monitor_sysctl(&admission, monitor)?;
            super::system_providers::verify_original_monitor_sample(&admission, monitor)?;
            let process_table = admission.verify_original_monitor_process_table(monitor);
            let process_timeout = super::system_providers::monitor_timeout::finish_assembly_probe();
            process_timeout.and(process_table)?;
            owner.probe_website()?;
            crate::workspace::objc::apple_events::probe_reference_safari_descriptors(&admission)?;
            crate::workspace::objc::probe_reference_safari_discovery(&admission, &workspace)?;
            modules[security].verify()?;
        }
        modules
    };
    #[cfg(not(target_arch = "aarch64"))]
    let modules = source.modules;
    for module in std::iter::once(bootstrap)
        .chain(modules)
        .chain(std::iter::once(workspace_module))
    {
        registry.verify_reference_issuer(&admission)?;
        if module
            .handles
            .iter()
            .any(|handle| !Arc::ptr_eq(&handle.life, &admission.life()))
        {
            return Err("Reference module guardian differs".into());
        }
        registry.register(module)?;
        registry.verify_reference_issuer(&admission)?;
    }
    registry.verify_reference_issuer(&admission)?;
    Ok(ReferenceActors {
        setup,
        workspace,
        registry,
        field_probe: inputs.field_probe,
    })
}

/// First actual assembly nodes. Fixture values are supplied by the fixed
/// source-built producer; this function performs no desktop/account discovery.
/// The context is borrowed from the bootstrap's real installed issuer rather
/// than reconstructed from paths. Further factory/catalog installation and all
/// remaining module attestations are still required before publication.
#[cfg(not(test))]
pub fn initialize_workspace_model(
    admission: &Arc<ReferenceAdmission>,
    applications: Vec<crate::workspace::Application>,
    screens: Vec<crate::workspace::Screen>,
    mouse: crate::workspace::Point,
) -> Result<(Arc<Workspace>, super::ModuleAttestation)> {
    admission.before_reference_initialization()?;
    initialize_bootstrap(admission, || {
        let context =
            super::bootstrap::owned_module_context(&admission.life(), &admission.descriptors())?;
        let model = Workspace::new(context, applications, screens, mouse)?;
        model.verify_reference_context(admission)?;
        // No receiver or arena exists until bootstrap finishes its selectors.
        unsafe { crate::workspace::objc::prepare_reference_data_factories(model.clone(), admission) }?;
        Ok(model)
    })
}

/// Connect the genuine source catalog and existing joint factory transaction.
/// This does not manufacture the missing independent Swift lifecycle issuer.
/// The available typed families are prepared through their actual owners.
/// Other required roles must already carry genuine published proofs before
/// install_workspace_checked can return a workspace attestation.
///
/// # Safety
/// The fixed source-built producer must retain the independent compiler/ABI/
/// generated ARC and lifecycle authority for exactly these original classes.
/// Neither a source roster nor the rows of runtime method addresses is such an
/// authority. All setup is exclusive and no original initializer is probed.
/// The producer additionally qualifies the complete immutable compiler/header/
/// image, ancestor and lifecycle pins of all SDK family transactions below.
#[cfg(not(test))]
pub unsafe fn initialize_workspace_module(
    admission: Arc<ReferenceAdmission>,
    model: Arc<Workspace>,
    catalog: Arc<crate::workspace::objc::application::SourceCallbackCatalog>,
) -> Result<super::ModuleAttestation> {
    admission.before_reference_initialization()?;
    admission.verify_clipboard_source_members()?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_open_completion_source(false)?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_reference_script_write_import(false)?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_native_edit_super_import(false)?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_reference_palette_panel_source()?;
    if !Arc::ptr_eq(&model.lease().life, &admission.life()) {
        return Err("Reference workspace catalog issuer differs".into());
    }
    model.verify_reference_context(&admission)?;
    catalog.verify_model(&model)?;
    let event_guard = admission.clone();
    let event_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_event_data_factory_checked(model.clone(), move || {
            event_model.verify_reference_context(&event_guard)
        })
    }?;
    let apple_event_guard = admission.clone();
    let apple_event_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_apple_event_factory_checked(model.clone(), move || {
            apple_event_model.verify_reference_context(&apple_event_guard)
        })
    }?;
    let task_guard = admission.clone();
    let task_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_task_factories_checked(model.clone(), move || {
            task_model.verify_reference_context(&task_guard)
        })
    }?;
    let factory_guard = admission.clone();
    let factory_model = model.clone();
    let _window_guard = unsafe {
        crate::workspace::objc::prepare_application_window_control_factories_checked(
            model.clone(),
            move || factory_model.verify_reference_context(&factory_guard),
        )
    }?;
    #[cfg(target_arch = "aarch64")]
    unsafe { crate::workspace::factory::prepare_reference_palette_construction(
        admission.clone(), model.clone(), &_window_guard,
    ) }?;
    #[cfg(target_arch = "aarch64")]
    unsafe { crate::workspace::factory::prepare_reference_snippet_panel_construction(
        admission.clone(), model.clone(), &_window_guard,
    ) }?;
    let running_screen_guard = admission.clone();
    let running_screen_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_running_and_screen_factories_checked(
            model.clone(),
            move || running_screen_model.verify_reference_context(&running_screen_guard),
        )
    }?;
    let notification_guard = admission.clone();
    let notification_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_notification_token_factory_checked(
            model.clone(),
            move || notification_model.verify_reference_context(&notification_guard),
        )
    }?;
    let center_guard = admission.clone();
    let center_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_notification_and_workspace_factories_checked(
            model.clone(),
            move || center_model.verify_reference_context(&center_guard),
        )
    }?;
    let payload_guard = admission.clone();
    let payload_model = model.clone();
    unsafe {
        crate::workspace::objc::prepare_notification_payload_factory_checked(
            model.clone(),
            move || payload_model.verify_reference_context(&payload_guard),
        )
    }?;
    let install_guard = admission.clone();
    let install_model = model.clone();
    let module = unsafe {
        crate::workspace::objc::install_workspace_checked(model.clone(), catalog, move || {
            install_model.verify_reference_clipboard_context(&install_guard)
        })
    }?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_open_completion_source(true)?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_reference_script_write_import(true)?;
    #[cfg(target_arch = "aarch64")]
    admission.verify_native_edit_super_import(true)?;
    model.verify_reference_context(&admission)?;
    Ok(module)
}

/// Install the two existing private provider contributions from this actual
/// bootstrap issuer. The process registry must already contain the producer's
/// exact source/artifact-qualified own executable/argv rules. Neither returned
/// module currently owns a complete SDK scope; no Scope/Ready bits are added.
///
/// # Safety
/// Exclusive source-qualified setup must bind the genuine five Security and
/// combined process/monitor imports, their ABI and compiled caller sources.
/// The fixed private producer, not arbitrary fixture JSON, supplies those code
/// proofs and the process rules. This function invokes no original provider.
#[cfg(not(test))]
pub unsafe fn initialize_provider_modules(
    admission: Arc<ReferenceAdmission>,
    history: Option<Vec<u8>>,
    entropy: Vec<u8>,
    processes: Arc<super::process_dispatch::Registry>,
) -> Result<(Vec<super::ModuleAttestation>, Arc<super::private_network::Context>)> {
    admission.before_reference_initialization()?;
    processes.matches_issuer(&admission.life(), &admission.descriptors())?;
    admission.verify_private_security_source_members()?;
    let security_guard = admission.clone();
    let (security, context) = unsafe {
        super::bootstrap::initialize_private_security(
            admission.life(),
            admission.descriptors(),
            history,
            entropy,
            move || security_guard.verify_private_security_source_members(),
        )
    }?;
    #[cfg(target_arch = "aarch64")]
    if let Err(error) = admission.verify_reference_utility_imports(&security.imports) {
        processes.retire();
        let _ = super::private_network::retire(&context);
        return Err(error);
    }
    let process_guard = admission.clone();
    let process = unsafe {
        super::bootstrap::initialize_reference_process_monitor(
            admission.clone(),
            processes.clone(),
            move || process_guard.before_reference_initialization(),
        )
    };
    match process {
        Ok(process) => {
            if let Err(error) = admission.before_reference_initialization() {
                processes.retire();
                let _ = super::private_network::retire(&context);
                return Err(error);
            }
            Ok((vec![security, process], context))
        }
        Err(error) => {
            processes.retire();
            let _ = super::private_network::retire(&context);
            Err(error)
        }
    }
}
struct Runtime {
    reader: Arc<ReferenceReader>,
    workspace: Arc<Workspace>,
    bindings: ReadyBindings,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    providers: super::reference_source_actors::RuntimeProviders,
    field_probe: bool,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    configuration_drain: std::sync::atomic::AtomicU8,
    field_result: Mutex<Option<Result<()>>>,
    stopped: Arc<AtomicBool>,
    teardown_started: AtomicBool,
    launched: AtomicBool,
    worker_done: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<Result<()>>>>,
}
static RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_published_input_child(
    admission: &Arc<ReferenceAdmission>,
    child: &Arc<crate::workspace::factory::ReferenceInputChildCustody>,
) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference input runtime absent")?;
    if !Arc::ptr_eq(admission, &runtime.reader.admission) {
        return Err("Reference input runtime differs".into());
    }
    MAIN.with(|slot| -> Result<()> {
        let slot = slot.try_borrow().map_err(|_| "Reference input custody busy")?;
        let published = slot.children.as_ref().ok_or("Reference input startup not committed")?;
        if !published.iter().any(|owner| Arc::ptr_eq(owner, child)) {
            return Err("Reference input child differs from committed startup".into());
        }
        Ok(())
    })?;
    if runtime.stopped.load(Ordering::Acquire) {
        return Err("Reference input runtime stopped".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_hyper_action_owner(admission: &Arc<ReferenceAdmission>, route: usize, context: usize) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference HyperKey action runtime absent")?;
    if !Arc::ptr_eq(admission, &runtime.reader.admission) {
        return Err("Reference HyperKey action runtime differs".into());
    }
    let child = MAIN.with(|slot| -> Result<_> {
        let slot = slot.try_borrow().map_err(|_| "Reference HyperKey custody busy")?;
        Ok(slot.children.as_ref().ok_or("Reference input startup not committed")?[2].clone())
    })?;
    child.verify_reference_binding(admission, &runtime.workspace)?;
    let keyboard = child.pointer_on_creator()?;
    unsafe { admission.verify_hyper_action_capture(route, context, keyboard) }?;
    child.verify_held_epoch()?;
    MAIN.with(|slot| {
        let slot = slot.try_borrow().map_err(|_| "Reference HyperKey custody busy")?;
        if !slot.children.as_ref().is_some_and(|children| Arc::ptr_eq(&children[2], &child)) {
            return Err("Reference HyperKey action owner changed".into());
        }
        Ok(())
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_live_snippet_choice(
    admission: &Arc<ReferenceAdmission>,
    model: &Arc<Workspace>,
    controller: usize,
) -> Result<()> {
    let expander = MAIN.with(|slot| -> Result<_> {
        let slot = slot.try_borrow().map_err(|_| "Reference input custody busy")?;
        Ok(slot.children.as_ref().ok_or("Reference input startup not committed")?[1].clone())
    })?;
    verify_published_input_child(admission, &expander)?;
    expander.verify_current_snippet_choice(admission, model, controller)?;
    verify_published_input_child(admission, &expander)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_terminal_input_cleanup(
    admission: &Arc<ReferenceAdmission>,
    model: &Arc<Workspace>,
) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference runtime absent at input cleanup")?;
    let engine = crate::workspace::objc::application::engine()?;
    engine.check_owner()?;
    if !Arc::ptr_eq(admission, &runtime.reader.admission)
        || !Arc::ptr_eq(model, &runtime.workspace)
        || !Arc::ptr_eq(engine.model(), model)
        || !runtime.teardown_started.load(Ordering::Acquire)
        || !runtime.stopped.load(Ordering::Acquire)
        || engine.phase()? != Phase::Terminated
    {
        return Err("Reference input cleanup requires its terminal stopped runtime".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn close_terminal_ingress(engine: &Engine) -> Result<()> {
    engine.check_owner()?;
    let runtime = RUNTIME.get().ok_or("Reference runtime absent at terminal process cancellation")?;
    if !Arc::ptr_eq(engine.model(), &runtime.workspace) {
        return Err("Reference terminal process owner differs".into());
    }
    let timer = MAIN.with(|slot| {
        slot.try_borrow().map_err(|_| "Reference terminal AppState custody busy")?
            .state.clone().ok_or("Reference terminal AppState custody absent")
    }).map_err(str::to_owned)
        .and_then(|state| state.invalidate_shutdown_poll_timer());
    let input_timers = MAIN.with(|slot| {
        slot.try_borrow().map_err(|_| "Reference terminal input custody busy")?
            .children.clone().ok_or("Reference terminal input custody absent")
    }).map_err(str::to_owned)
        .and_then(|children| stop_input_poll_timers(&children));
    let observer = (|| -> Result<()> {
        match runtime.configuration_drain.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => (),
            Err(2) => return Ok(()),
            Err(_) => return Err("Reference configuration drain incomplete or reentered".into()),
        }
        let wake = engine.wake_handle()?;
        let state = MAIN.with(|slot| {
            slot.try_borrow().map_err(|_| "Reference terminal AppState custody busy")?
                .state.clone().ok_or("Reference terminal AppState custody absent")
        }).map_err(str::to_owned)?;
        let queue = state.cancel_configuration_sources()?;
        crate::workspace::callbacks::enqueue_reference_configuration_drain(queue, wake)?;
        runtime.configuration_drain.store(2, Ordering::Release);
        Ok(())
    })();
    let jobs = runtime.providers.cancel_jobs();
    timer.and(input_timers).and(observer).and(jobs)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
fn stop_input_poll_timers(
    children: &[Arc<crate::workspace::factory::ReferenceInputChildCustody>; 3],
) -> Result<()> {
    let snippets = children[1].invalidate_input_poll_timer();
    let hyper = children[2].invalidate_input_poll_timer();
    snippets.and(hyper)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn worker_dispatch_admission() -> Result<(Arc<ReferenceAdmission>, &'static AtomicBool)> {
    let runtime = RUNTIME.get().ok_or("Reference monitor runtime absent")?;
    let admission = &runtime.reader.admission;
    if unsafe { libc::pthread_main_np() } == 1 {
        admission.before_reference_publication()?;
    } else {
        admission.before_reference_worker_callback()?;
    }
    Ok((admission.clone(), runtime.stopped.as_ref()))
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn configuration_worker_wake(
    admission: &Arc<ReferenceAdmission>,
) -> Result<Arc<crate::workspace::cf_loop::Wake>> {
    let runtime = RUNTIME.get().ok_or("Reference configuration runtime absent")?;
    if !Arc::ptr_eq(admission, &runtime.reader.admission) {
        return Err("Reference configuration worker issuer differs".into());
    }
    crate::workspace::objc::application::configuration_worker_wake(&runtime.workspace)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_safari_cache_owner(services: usize) -> Result<()> {
    let state = MAIN.with(|slot| -> Result<_> {
        let slot = slot.try_borrow().map_err(|_| "Reference Safari owner slot busy")?;
        slot.state.clone().ok_or_else(|| "Reference Safari AppState absent".into())
    })?;
    state.verify_safari_cache_owner(services)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_monitor_worker_queue(window: bool, queue: usize, captured: usize) -> Result<()> {
    let root = MAIN.with(|slot| {
        slot.try_borrow().map_err(|_| "Reference monitor owner slot busy")?
            .root.clone().ok_or("Reference monitor root absent")
    })?;
    root.verify_monitor_worker_queue(window, queue, captured)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn verify_startup_control_caller(model: &Arc<Workspace>, caller: usize, offsets: &[usize]) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference startup control runtime absent")?;
    if !Arc::ptr_eq(model, &runtime.workspace) || runtime.stopped.load(Ordering::Acquire) {
        return Err("Reference startup control owner differs".into());
    }
    runtime.reader.admission.verify_reference_startup_control_caller(caller, offsets)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn verify_safari_discovery(model: &Arc<Workspace>, caller: usize) -> Result<&'static AtomicBool> {
    let runtime = RUNTIME.get().ok_or("Reference Safari runtime absent")?;
    if !Arc::ptr_eq(model, &runtime.workspace) {
        return Err("Reference Safari discovery owner differs".into());
    }
    // Search, consent, and activation all enter and leave through a
    // counted Dispatch block with full admission at both boundaries.
    let admission = &runtime.reader.admission;
    if !crate::workspace::callbacks::reference_safari_discovery_worker(admission) {
        return Err("Reference Safari discovery has no qualified dispatch turn".into());
    }
    admission.verify_reference_safari_discovery(caller)?;
    if !crate::workspace::callbacks::reference_safari_discovery_worker(admission) {
        return Err("Reference Safari discovery worker changed during qualification".into());
    }
    Ok(runtime.stopped.as_ref())
}

#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn verify_safari_effect(
    model: &Arc<Workspace>, caller: usize, effect: crate::workspace::callbacks::SafariEffect,
) -> Result<&'static AtomicBool> {
    let runtime = RUNTIME.get().ok_or("Reference Safari runtime absent")?;
    if !Arc::ptr_eq(model, &runtime.workspace) {
        return Err("Reference Safari effect owner differs".into());
    }
    let stopped = runtime.stopped.as_ref();
    crate::workspace::callbacks::reference_safari_effect(effect, stopped, || Ok(()))?;
    runtime.reader.admission.verify_reference_safari_effect(caller, effect)?;
    Ok(stopped)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
fn open_application_owner(model: &Arc<Workspace>) -> Result<&'static Arc<Runtime>> {
    let runtime = RUNTIME.get().ok_or("Reference open runtime absent")?;
    if runtime.field_probe || !runtime.launched.load(Ordering::Acquire)
        || !Arc::ptr_eq(model, &runtime.workspace)
    {
        return Err("Reference Open requires the launched non-field runtime and its workspace".into());
    }
    let engine = crate::workspace::objc::application::engine()?;
    engine.check_owner()?;
    if !Arc::ptr_eq(engine.model(), model) || engine.phase()? != Phase::Running {
        return Err("Reference Open requires the running owner loop".into());
    }
    runtime.reader.admission.before_reference_publication()?;
    Ok(runtime)
}
#[cfg(all(not(test), target_arch = "aarch64"))]
fn open_application_state(runtime: &Runtime) -> Result<Arc<crate::workspace::factory::AppStateCustody>> {
    let state = MAIN.with(|slot| -> Result<_> {
        let slot = slot.try_borrow().map_err(|_| "Reference Open AppState custody busy")?;
        slot.state.clone().ok_or_else(|| "Reference Open AppState custody absent".into())
    })?;
    state.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
    Ok(state)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn open_application_admission(model: &Arc<Workspace>) -> Result<(Arc<ReferenceAdmission>, usize)> {
    let runtime = open_application_owner(model)?;
    if runtime.stopped.load(Ordering::Acquire) {
        return Err("Reference open wire stopped".into());
    }
    let owner = open_application_state(runtime)?.pointer_on_creator()?;
    if runtime.stopped.load(Ordering::Acquire) {
        return Err("Reference open wire stopped during owner qualification".into());
    }
    Ok((runtime.reader.admission.clone(), owner))
}
#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn open_application_cleanup_admission(
    model: &Arc<Workspace>,
) -> Result<(Arc<ReferenceAdmission>, &'static AtomicBool, Arc<crate::workspace::factory::AppStateCustody>)> {
    let runtime = open_application_owner(model)?;
    let engine = crate::workspace::objc::application::engine()?;
    let owner = open_application_state(runtime)?;
    if engine.phase()? != Phase::Running {
        return Err("Reference open cleanup owner loop stopped during qualification".into());
    }
    Ok((runtime.reader.admission.clone(), runtime.stopped.as_ref(), owner))
}
#[cfg(all(not(test), target_arch = "aarch64"))]
pub(crate) fn verify_current_open_application_state(
    owner: &Arc<crate::workspace::factory::AppStateCustody>,
    expected: usize,
    generation: u64,
) -> Result<()> {
    MAIN.with(|slot| -> Result<()> {
        let slot = slot.try_borrow().map_err(|_| "Reference Open AppState custody busy")?;
        if !slot.state.as_ref().is_some_and(|state| Arc::ptr_eq(state, owner)) {
            return Err("Reference Open AppState owner changed".into());
        }
        owner.verify_open_owner_slot(expected, generation)
    })
}

struct MainSlot {
    owner: Option<ReferenceOwner>,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    root: Option<Arc<crate::workspace::factory::InitializedAppDelegateRoot>>,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    state: Option<Arc<crate::workspace::factory::AppStateCustody>>,
    #[cfg(all(not(test), target_arch = "aarch64"))]
    children: Option<[Arc<crate::workspace::factory::ReferenceInputChildCustody>; 3]>,
    entered: bool,
}
impl MainSlot {
    const fn empty() -> Self {
        Self { owner: None,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        root: None,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        state: None,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        children: None, entered: false }
    }
}
thread_local! {
    static MAIN: RefCell<MainSlot> = const { RefCell::new(MainSlot::empty()) };
    #[cfg(all(not(test), target_arch = "aarch64"))]
    static STARTUP_CUSTODY: RefCell<StartupCustody> = const { RefCell::new(StartupCustody {
        root: None, state: None, children: [None, None, None],
    }) };
}

#[cfg(all(not(test), target_arch = "aarch64"))]
struct StartupCustody {
    root: Option<Arc<crate::workspace::factory::InitializedAppDelegateRoot>>,
    state: Option<Arc<crate::workspace::factory::AppStateCustody>>,
    children: [Option<Arc<crate::workspace::factory::ReferenceInputChildCustody>>; 3],
}

fn clock_resolution_ns() -> Result<u64> {
    let mut value: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_getres(libc::CLOCK_MONOTONIC, &mut value) } != 0
        || value.tv_sec < 0
        || !(0..1_000_000_000).contains(&value.tv_nsec)
    {
        return Err("Reference monotonic clock resolution unavailable".into());
    }
    u64::try_from(value.tv_sec)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000_000))
        .and_then(|seconds| seconds.checked_add(value.tv_nsec as u64))
        .filter(|value| *value > 0)
        .ok_or("Reference monotonic resolution overflow/zero".into())
}

/// Publish only actual assembled source modules from the SAME admission/model.
/// The caller has installed the source-prepared factories and actual NSApp
/// imports; no context, literal readiness mask or synthetic catalog substitutes.
pub fn install_assembled(
    admission: Arc<ReferenceAdmission>,
    mut actors: ReferenceActors,
) -> Result<()> {
    admission.before_reference_initialization()?;
    admission.verify_startup_profile()?;
    if RUNTIME.get().is_some() {
        return Err("Reference runtime publication replay".into());
    }
    actors.registry.verify_reference_issuer(&admission)?;
    let life = admission.life();
    if actors.registry.modules.values().any(|module| {
        module
            .handles
            .iter()
            .any(|handle| !Arc::ptr_eq(&handle.life, &life))
    }) {
        return Err("Reference module guardian differs".into());
    }
    let lease = actors.workspace.lease();
    if !Arc::ptr_eq(&life, &lease.life)
        || !Arc::ptr_eq(
            crate::workspace::objc::application::engine()?.model(),
            &actors.workspace,
        )
    {
        return Err("Reference workspace/engine/guardian differs from actual assembly".into());
    }
    // Check the actual complete Registry before its one-way publication, not
    // a module count or copied mask. Incomplete actors never start this runtime.
    if actors.registry.ready() != 0xF100002600003FFF {
        #[cfg(all(not(test), target_arch = "aarch64"))]
        actors.setup.sources.as_ref()
            .ok_or("Reference source setup owner absent during readiness refusal")?
            .runtime_providers()?.retire()
            .map_err(|error| format!("Reference provider rollback on readiness refusal failed: {error}"))?;
        let present: std::collections::BTreeSet<_> = actors.registry.modules.values()
            .flat_map(|module| module.scopes.iter().copied()).collect();
        let required: std::collections::BTreeSet<_> = super::SCOPES.into_iter().collect();
        let missing: Vec<_> = required.difference(&present).copied().collect();
        let unexpected: Vec<_> = present.difference(&required).copied().collect();
        return Err(format!(
            "Reference readiness refused; source routes qualified: {}; missing scopes: [{}]; unexpected scopes: [{}]",
            super::SOURCE_ROUTES_QUALIFIED, missing.join(", "), unexpected.join(", "),
        ));
    }
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let providers = actors.setup.sources.as_ref()
        .ok_or("Reference source setup owner absent")?.runtime_providers()?;
    super::publish_registry(actors.registry)?;
    admission.before_reference_publication()?;
    let (fixture, authority, sdk) = admission.ready_facts();
    let bindings = ReadyBindings {
        fixture_sha256: fixture.into(),
        authority_sha256: authority.into(),
        sdk_guard_sha256: sdk.into(),
        clock_resolution_ns: clock_resolution_ns()?,
    };
    // Validation only; no Ready packet is emitted until the genuine owner and
    // complete existing Host routes subsequently pass native_f1::serve.
    bindings.envelope()?;
    let reader = ReferenceReader::published(admission)?;
    RUNTIME
        .set(Arc::new(Runtime {
            reader,
            workspace: actors.workspace,
            #[cfg(all(not(test), target_arch = "aarch64"))]
            providers,
            bindings,
            field_probe: actors.field_probe,
            #[cfg(all(not(test), target_arch = "aarch64"))]
            configuration_drain: std::sync::atomic::AtomicU8::new(0),
            field_result: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
            teardown_started: AtomicBool::new(false),
            launched: AtomicBool::new(false),
            worker_done: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
        }))
        .map_err(|_| "Reference runtime already published".to_string())?;
    actors.setup.published = true;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    if let Some(sources) = &mut actors.setup.sources {
        sources.finish();
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_original_launch_profile(model: &Arc<Workspace>) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference original launch runtime absent")?;
    if !Arc::ptr_eq(model, &runtime.workspace)
        || runtime.launched.load(Ordering::Acquire)
        || runtime.stopped.load(Ordering::Acquire)
    {
        return Err("Reference original launch profile owner or phase differs".into());
    }
    runtime.reader.admission.verify_startup_profile()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn activate_ax_target(
    model: &Arc<Workspace>, handle: crate::workspace::Handle, caller: usize,
    selector: objc2::runtime::Sel, options: usize,
) -> Result<bool> {
    let runtime = RUNTIME.get().ok_or("Reference AX activation runtime absent")?;
    if options != 0 || unsafe { libc::pthread_main_np() } != 1
        || !Arc::ptr_eq(model, &runtime.workspace)
        || !runtime.launched.load(Ordering::Acquire)
    {
        return Err("Reference AX activation owner or options differ".into());
    }
    runtime.reader.admission.verify_reference_ax_activation(caller, selector)?;
    // Swift can return after activation without reaching AXPress/AXRaise.
    // A new qualified attempt must supersede any receipt left by that path.
    model.clear_reference_ax_activation()?;
    let pid = super::system_providers::OWNED_EDITOR_PID;
    if model.by_pid(pid)? != Some(handle) {
        return Err("Reference AX activation target differs".into());
    }
    let identity = super::process_router::reference_workspace_identity(model, pid)?;
    runtime.reader.admission.verify_reference_ax_activation(caller, selector)?;
    model.activate_reference_ax(handle, identity, &runtime.stopped)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
thread_local! {
    static FIELD_BOX_DROP: std::cell::Cell<Option<(usize, usize, bool)>> =
        const { std::cell::Cell::new(None) };
}

/// The arm64 entry observes the original Swift caller and its exact box
/// register. All other operations still use the existing qualified guard.
#[cfg(all(not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
pub extern "C" fn reference_guard(_operation: u32) -> i32 {
    core::arch::naked_asm!(
        "mov x1, x30",
        "mov x2, x19",
        "b {guard}",
        guard = sym reference_guard_impl,
    );
}

#[cfg(any(test, not(target_arch = "aarch64")))]
#[inline(never)]
pub extern "C" fn reference_guard(operation: u32) -> i32 {
    reference_guard_impl(operation, 0, 0)
}

extern "C" fn reference_guard_impl(operation: u32, caller: usize, box_address: usize) -> i32 {
    #[cfg(all(not(test), target_arch = "aarch64"))]
    if operation == 2 && FIELD_BOX_DROP.with(|permit| {
        if permit.get() != Some((caller, box_address, false)) {
            return false;
        }
        permit.set(Some((caller, box_address, true)));
        true
    }) {
        return 1;
    }
    let _ = (caller, box_address);
    let outcome = std::panic::catch_unwind(|| {
        let runtime = RUNTIME.get().ok_or("Reference runtime absent")?;
        // This private probe uses the borrowed AppDelegate box, the hosted
        // view, the trace RPC and a snapshot readback. Refuse the standalone
        // constructors and direct query/mode/selection setters at the guard.
        // Exact field-box release remains the one-shot branch above.
        if runtime.field_probe && !matches!(operation, 7..=11) {
            return Err("Reference field probe operation outside retained route".into());
        }
        if runtime.stopped.load(Ordering::Acquire) && operation != 2 {
            return Err("Reference runtime stopped".into());
        }
        if unsafe { libc::pthread_main_np() } != 1 {
            return if operation == 8 {
                runtime.reader.before_worker_callback()
            } else {
                Err("Reference main operation on worker".into())
            };
        }
        if !(1..=11).contains(&operation)
            || matches!(operation, 10 | 11) && !runtime.launched.load(Ordering::Acquire)
        {
            return Err("Reference operation/startup phase differs".into());
        }
        runtime.reader.before_main_call()?;
        if operation == 2 {
            // This shared operation carries no box/child identity and is also
            // used by the temporary headless process owner. It may not retire
            // application-wide input registrations during an active session.
            if !runtime.stopped.load(Ordering::Acquire) {
                return Err("Reference release requires the actual runtime stop phase".into());
            }
            // Native owner tickets and absence of registrations do not prove
            // the missing Swift child, queued-work and inflight retirement.
            // Returning zero here keeps the actual F1Box+issuer quarantined by
            // the existing qualified Bridge close/Drop path, before Swift drop.
            return Err("Reference Swift input owner retirement is unqualified".into());
        }
        Ok(())
    });
    match outcome {
        Ok(Ok(())) => 1,
        Ok(Err(_)) => 0,
        Err(payload) => {
            if let Some(runtime) = RUNTIME.get() {
                runtime.stopped.store(true, Ordering::Release);
            }
            std::mem::forget(payload);
            0
        }
    }
}

/// Called by the existing CF run path only AFTER Engine::launch successfully
/// invoked the actual selected Will/DidFinish callbacks, before its first turn.
pub(crate) fn after_launch(engine: Arc<Engine>) -> Result<()> {
    engine.check_owner()?;
    let runtime = RUNTIME
        .get()
        .ok_or("Reference runtime absent before actual launch")?;
    if !Arc::ptr_eq(engine.model(), &runtime.workspace) || engine.phase()? != Phase::Running {
        return Err("Reference did-finish engine/model differs".into());
    }
    runtime.reader.before_main_call()?;
    if runtime.launched.swap(true, Ordering::AcqRel) {
        #[cfg(all(not(test), target_arch = "aarch64"))]
        {
            let (root, state, children) = MAIN.with(|slot| -> Result<_> {
                let slot = slot.try_borrow().map_err(|_| "Reference owner busy")?;
                Ok((
                    slot.root.clone().ok_or("Reference initialized root absent on resume")?,
                    slot.state.clone().ok_or("Reference captured AppState absent on resume")?,
                    slot.children.clone().ok_or("Reference captured input children absent on resume")?,
                ))
            })?;
            root.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
            state.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
            for child in &children {
                child.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
            }
            root.verify_held_epoch()?;
            state.verify_held_epoch()?;
            for child in &children { child.verify_held_epoch()?; }
        }
        return MAIN.with(|slot| {
            if slot
                .try_borrow()
                .map_err(|_| "Reference owner busy")?
                .owner
                .is_none()
            {
                Err("Reference owner missing on resumed actual loop".into())
            } else {
                Ok(())
            }
        });
    }
    let wake_handle = engine.wake_handle()?;
    let wake: native_f1::reference::Wake = Arc::new(move || {
        let _ = wake_handle.signal();
    });
    // Transfer the existing whole-initialized receiver's physical +1. This
    // grants no AppState/child identity, input callback or drain authority.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let root = {
        let root = crate::workspace::factory::take_initialized_app_delegate_root(&runtime.workspace)?;
        root.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        STARTUP_CUSTODY.with(|slot| -> Result<()> {
            let mut slot = slot.try_borrow_mut().map_err(|_| "Reference owner slot busy")?;
            if slot.root.is_some() {
                return Err("Reference root custody replay".into());
            }
            // Retain cleanup custody before later startup checks can fail.
            slot.root = Some(root.clone());
            Ok(())
        })?;
        root.verify_started_control(&runtime.reader.admission)?;
        root
    };
    let binding = QualifiedFunctions::read(runtime.reader.clone(), reference_guard)?;
    let bridge = Bridge::borrow_qualified_delegate(binding)?;
    // Bridge setup may cross dynamic source boundaries. Recheck
    // this exact admission/model/root after them, before any slot publication.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    root.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
    // Capture through the qualified finite getter only after genuine launch.
    // None/replay/error refuses setup; custody is not child or input authority.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let state = root.capture_app_state()?.ok_or("Reference did-finish AppState absent")?;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    {
        state.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        STARTUP_CUSTODY.with(|slot| -> Result<()> {
            let mut slot = slot.try_borrow_mut().map_err(|_| "Reference owner slot busy")?;
            if slot.state.is_some() {
                return Err("Reference AppState custody replay".into());
            }
            slot.state = Some(state.clone());
            Ok(())
        })?;
        let borrowed = bridge.borrowed_delegate_box_for(runtime.reader.as_ref())?;
        unsafe { state.verify_borrowed_reference_box(borrowed.as_ptr() as usize) }?;
        state.verify_configuration_observer(true)?;
        root.verify_held_epoch()?;
        state.verify_held_epoch()?;
    }
    // Actual typed children are captured after did-finish only. Physical
    // custody does not bind callback userdata or admit input registrations.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let children = {
        use crate::workspace::factory::ReferenceInputChildKind;
        let retain_for_cleanup = |index: usize, child: &Arc<crate::workspace::factory::ReferenceInputChildCustody>| -> Result<()> {
            child.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
            STARTUP_CUSTODY.with(|slot| -> Result<()> {
                let mut slot = slot.try_borrow_mut().map_err(|_| "Reference startup custody busy")?;
                if slot.children[index].is_some() {
                    return Err("Reference input child custody replay".into());
                }
                slot.children[index] = Some(child.clone());
                Ok(())
            })
        };
        let hotkeys = state.capture_input_child(ReferenceInputChildKind::Hotkeys)?
            .ok_or("Reference initialized Hotkeys absent")?;
        retain_for_cleanup(0, &hotkeys)?;
        let keyboard = hotkeys.capture_keyboard()?;
        retain_for_cleanup(2, &keyboard)?;
        let expander = (|| -> Result<_> {
            let child = state.capture_input_child(ReferenceInputChildKind::SnippetExpander)?
                .ok_or("Reference initialized SnippetExpander absent")?;
            retain_for_cleanup(1, &child)?;
            Ok(child)
        })();
        // Capture the independent timer if available, but a failed capture
        // must still let the original HyperKey restore a partial Caps change.
        let caps_rollback = keyboard.rollback_hyper_without_tap(&runtime.reader.admission, &runtime.workspace);
        let expander = expander.map_err(|error| match &caps_rollback {
            Err(rollback) => format!("{error}. Reference Caps rollback failed ({rollback})"),
            Ok(_) => error,
        })?;
        if caps_rollback? {
            return Err("Reference HyperKey tap startup failed; original Caps connection restored and closed".into());
        }
        let children = [hotkeys, expander, keyboard];
        super::input_permissions_caps::verify_reference_startup_connection(
            &runtime.reader.admission, &runtime.workspace, &children[2],
        )?;
        super::input_carbon::publish_reference_owner(&runtime.reader.admission,&runtime.workspace,children[0].clone())?;
        super::input_hid::publish_reference_owner(&runtime.reader.admission,&runtime.workspace,children[2].clone())?;
        super::input_events::publish_reference_owner(&runtime.reader.admission,&runtime.workspace,children[2].clone())?;
        super::input_events::publish_reference_owner(&runtime.reader.admission,&runtime.workspace,children[1].clone())?;
        if children.iter().map(|child| child.kind()).ne([
            ReferenceInputChildKind::Hotkeys,
            ReferenceInputChildKind::SnippetExpander,
            ReferenceInputChildKind::HyperKey,
        ]) {
            return Err("Reference captured child roles differ".into());
        }
        for child in &children {
            child.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        }
        root.verify_held_epoch()?;
        state.verify_held_epoch()?;
        for child in &children { child.verify_held_epoch()?; }
        children
    };
    // Stdio shutdown and owner requests must observe the same stop signal.
    let (mut host, owner) = native_f1::reference::coordinator(
        bridge,
        runtime.bindings.clone(),
        wake.clone(),
        runtime.stopped.clone(),
        runtime.field_probe,
    )?;
    MAIN.with(|slot| -> Result<()> {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| "Reference owner slot busy")?;
        if slot.owner.is_some() || slot.entered {
            return Err("Reference owner publication replay".into());
        }
        #[cfg(all(not(test), target_arch = "aarch64"))]
        {
            if slot.root.is_some() || slot.state.is_some() || slot.children.is_some() {
                return Err("Reference root/AppState/children publication replay".into());
            }
            STARTUP_CUSTODY.with(|pending| -> Result<()> {
                let mut pending = pending.try_borrow_mut().map_err(|_| "Reference startup custody busy")?;
                if !pending.root.as_ref().is_some_and(|held| Arc::ptr_eq(held, &root))
                    || !pending.state.as_ref().is_some_and(|held| Arc::ptr_eq(held, &state))
                    || !pending.children.iter().zip(&children).all(|(held, child)| {
                        held.as_ref().is_some_and(|held| Arc::ptr_eq(held, child))
                    })
                {
                    return Err("Reference root/AppState/children custody changed".into());
                }
                slot.root = pending.root.take();
                slot.state = pending.state.take();
                slot.children = Some(children.clone());
                pending.children = [None, None, None];
                Ok(())
            })?;
        }
        slot.owner = Some(owner);
        Ok(())
    })?;
    let stopped = runtime.stopped.clone();
    let worker_done = runtime.worker_done.clone();
    let field_probe = runtime.field_probe;
    let worker_wake = wake.clone();
    // Only the first owner-thread launch installs a worker. Release the slot
    // lock before spawn because a spawn error drops the host and can wake us.
    if runtime.worker.lock().map_err(|_| "Reference worker slot poisoned")?.is_some() {
        return Err("Reference wire worker already installed".into());
    }
    let worker = std::thread::Builder::new()
        .name("f1-reference-wire".into())
        .spawn(move || {
            let result = (|| {
                // Core verified these exact exclusive stdio endpoints and keeps
                // their source/admission alive. Borrow, never adopt/regrant/close.
                let input = unsafe { BorrowedFd::borrow_raw(0) };
                let output = unsafe { BorrowedFd::borrow_raw(1) };
                let reader = native_f1::borrowed_stdio::Reader::new(input, stopped.clone())
                    .map_err(|e| e.to_string())?;
                let writer = native_f1::borrowed_stdio::Writer::new(output, stopped.clone())
                    .map_err(|e| e.to_string())?;
                let mut frames = native_f1::Frames::new(reader, writer);
                if field_probe {
                    use native_f1::Host;
                    use std::time::{Duration, Instant};
                    let field = (|| -> Result<()> {
                        host.verify_live()?;
                        if frames.read()? != serde_json::json!({"phase":"reference-field-probe"}) {
                            return Err("Reference field probe request differs".into());
                        }
                        // Each query returns only after its owner-bound trace
                        // completes and the retained field epoch is retired.
                        // Reconcile uses a changed query so stale state cannot
                        // satisfy its readback after the Input stage.
                        let title = "Reference Input Fixture";
                        for (stage, query) in [
                            (native_f1::QueryStage::Input, title),
                            (native_f1::QueryStage::Reconciliation, "Reference Input"),
                        ] {
                            let deadline = Instant::now() + Duration::from_secs(15);
                            host.query(stage, query, deadline)
                                .map_err(|error| format!("Reference {stage:?} field request: {error}"))?;
                            let snapshot = host.snapshot(deadline)?;
                            if snapshot["query"] != query {
                                return Err(format!("Reference {stage:?} did not reach the original query"));
                            }
                            let rows = snapshot["rows"].as_array()
                                .ok_or_else(|| format!("Reference {stage:?} returned no original result rows"))?;
                            if !rows.iter().any(|row| row["id"] == "fixture.input" && row["title"] == title) {
                                return Err(format!("Reference {stage:?} did not produce the original fixture row"));
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = field {
                        let detail = error.chars().take(512).collect::<String>();
                        let _ = frames.write(&serde_json::json!({
                            "phase":"reference-field-error","error":detail
                        }));
                        return Err(error);
                    }
                    return Ok(());
                }
                native_f1::serve(&mut host, &mut frames)
            })();
            drop(host);
            stopped.store(true, Ordering::Release);
            // No main-thread requests remain after this point. The owner
            // may join even if it consumes the wake before the thread exits.
            worker_done.store(true, Ordering::Release);
            worker_wake();
            result
        })
        .map_err(|e| format!("Reference owned wire worker: {e}"))?;
    match runtime.worker.lock() {
        Ok(mut slot) => {
            *slot = Some(worker);
            Ok(())
        }
        Err(poisoned) => {
            // Keep the join handle for after_loop even when publication must
            // report a failure.
            *poisoned.into_inner() = Some(worker);
            Err("Reference worker slot poisoned".into())
        }
    }
}

/// Take the Main owner out of its cell BEFORE any original/source callback.
/// The existing CF callback supplies reentrancy containment and error retirement.
pub(crate) fn poll_main() -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference runtime absent on CF turn")?;
    runtime.reader.before_main_call()?;
    if runtime.field_probe && runtime.stopped.load(Ordering::Acquire)
        && runtime.field_result.lock().map_err(|_| "Reference field result slot poisoned")?.is_some()
    {
        // The owner has finished. Events already ahead of Terminate may still
        // leave the engine Running for a turn, with no wire work to poll.
        return Ok(());
    }
    // Clone actual custody under a short cell borrow; verification and all
    // foreign owner callbacks run after releasing that borrow.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let (held_root, held_state, held_children) = {
        let (root, state, children) = MAIN.with(|slot| -> Result<_> {
            let slot = slot.try_borrow().map_err(|_| "Reference owner slot busy")?;
            Ok((
                slot.root.clone().ok_or("Reference initialized root absent on CF turn")?,
                slot.state.clone().ok_or("Reference captured AppState absent on CF turn")?,
                slot.children.clone().ok_or("Reference captured input children absent on CF turn")?,
            ))
        })?;
        root.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        state.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        for child in &children {
            child.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
        }
        root.verify_held_epoch()?;
        state.verify_held_epoch()?;
        for child in &children { child.verify_held_epoch()?; }
        (root, state, children)
    };
    let owner = MAIN.with(|slot| -> Result<Option<ReferenceOwner>> {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| "Reference owner slot busy")?;
        if slot.entered {
            return Ok(None);
        }
        let owner = slot.owner.take().ok_or("Reference main owner absent")?;
        slot.entered = true;
        Ok(Some(owner))
    })?;
    let mut field_termination_safe = false;
    if let Some(mut owner) = owner {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.poll()));
        field_termination_safe = owner.field_termination_safe();
        MAIN.with(|slot| -> Result<()> {
            let mut slot = slot
                .try_borrow_mut()
                .map_err(|_| "Reference owner restore busy")?;
            slot.owner = Some(owner);
            slot.entered = false;
            Ok(())
        })?;
        match result {
            Ok(result) => {
                result?;
                // A nested callback may change the parent while returning Ok.
                // Revalidate binding outside the cell; only pure custody tails
                // follow that final dynamic check before accepting the poll.
                #[cfg(all(not(test), target_arch = "aarch64"))]
                {
                    held_state.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
                    for child in &held_children {
                        child.verify_reference_binding(&runtime.reader.admission, &runtime.workspace)?;
                    }
                    held_root.verify_held_epoch()?;
                    held_state.verify_held_epoch()?;
                    for child in &held_children { child.verify_held_epoch()?; }
                }
            }
            Err(payload) => {
                runtime.stopped.store(true, Ordering::Release);
                std::mem::forget(payload);
                return Err("Reference main owner panicked; no retry".into());
            }
        }
    }
    let finished = {
        let mut worker = runtime
            .worker
            .lock()
            .map_err(|_| "Reference worker slot poisoned")?;
        if worker.as_ref().is_some_and(|handle| {
            runtime.worker_done.load(Ordering::Acquire) || handle.is_finished()
        }) {
            worker.take()
        } else {
            None
        }
    };
    if let Some(worker) = finished {
        return match worker.join() {
            Ok(result) if runtime.field_probe && field_termination_safe => {
                // A failed input can leave the field untouched or complete
                // owner-bound cancellation. The worker result stays Err.
                #[cfg(all(not(test), target_arch = "aarch64"))]
                let confirmed_cancel = result.is_err()
                    && crate::workspace::receiver::app_delegate::constructor::verify_reference_field_drained(true).is_ok();
                #[cfg(any(test, not(target_arch = "aarch64")))]
                let confirmed_cancel = false;
                if !confirmed_cancel {
                    if let Err(error) = &result {
                        return Err(error.clone());
                    }
                    #[cfg(all(not(test), target_arch = "aarch64"))]
                    crate::workspace::receiver::app_delegate::constructor::verify_reference_field_drained(false)?;
                }
                let owner = MAIN.with(|slot| -> Result<_> {
                    let mut slot = slot.try_borrow_mut().map_err(|_| "Reference field release owner busy")?;
                    if slot.entered {
                        return Err("Reference field release owner entered".into());
                    }
                    slot.owner.take().ok_or_else(|| "Reference field release owner absent".into())
                })?;
                owner.finish_borrowed_field_box()?;
                crate::workspace::objc::application::engine()?.enqueue(crate::workspace::event_loop::Event::Terminate)?;
                let mut slot = runtime.field_result.lock().map_err(|_| "Reference field result slot poisoned")?;
                if slot.is_some() {
                    return Err("Reference field result already recorded".into());
                }
                *slot = Some(result);
                Ok(())
            }
            Ok(Ok(())) if runtime.field_probe => Err("Reference field probe owner still pending or quarantined".into()),
            Ok(result) => result,
            Err(payload) => {
                std::mem::forget(payload);
                Err("Reference wire worker panicked; no retry".into())
            }
        };
    }
    Ok(())
}

/// The two source-built producer functions are fixed at compile time by the
/// existing macro pattern below, never chosen by an environment/JSON registry.
/// # Safety
/// The producer independently source-qualifies both makers and the actual
/// private ReferencePins. assemble must construct/install genuine source actors,
/// factories, catalog and all module attestations from THIS admission. It must
/// not publish a registry itself or call any Main-table pointer before complete
/// code/table qualification. SDK role is zero constructors, Main exactly one.
#[cfg(not(test))]
pub unsafe fn enter_source_built(
    pins: &super::core_preentry::ReferencePins<'_>,
    guardian: crate::guardian::Guard,
    table: usize,
    entry_address: usize,
    assemble: fn(Arc<ReferenceAdmission>) -> Result<ReferenceActors>,
) -> Result<()> {
    let (reply, provider) = native_f1::reference::callback_entrypoints();
    let sdk = [
        ("entry", entry_address),
        ("guard", reference_guard as *const () as usize),
        ("reply", reply as *const () as usize),
        ("process_provider", provider as *const () as usize),
        (
            "main_owner_perform",
            crate::workspace::cf_loop::perform_address(),
        ),
    ];
    let admission = Arc::new(super::core_preentry::enter_reference_with_guard(
        pins, guardian, table, sdk,
    )?);
    // This invokes only the compile-time source-built actor maker, never the
    // caller-supplied Main table. Missing real modules fail Registry admission.
    let actors = assemble(admission.clone())?;
    install_assembled(admission, actors)
}

/// Same fixed source-built maker/finalizer pattern as the Native initializer,
/// but this declares ONLY the explicit SDK entry: no SDK initializer is emitted.
#[macro_export]
macro_rules! reference_sdk_export {
    ($make_pins:path, $assemble:path) => {
        #[cfg(not(rcf1_reference_sdk))]
        compile_error!("Reference SDK export requires the fixed Reference role cfg");
        #[cfg(all(rcf1_reference_sdk, not(test)))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn ruycast_f1_reference_preentry_v1(
            table: *const ruycast_native::native_f1::reference::MainFunctionsV1,
        ) -> i32 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let guard = $crate::guardian::Guard::bootstrap()
                    .map_err(|_| "Reference guardian admission failed".to_string())?;
                let pins = $make_pins();
                unsafe {
                    $crate::sdk::reference_runtime::enter_source_built(
                        &pins,
                        guard,
                        table as usize,
                        ruycast_f1_reference_preentry_v1 as *const () as usize,
                        $assemble,
                    )
                }
            }));
            match outcome {
                Ok(Ok(())) => 1,
                Ok(Err(error)) => {
                    // Only the already owned child stdout is used. A bounded
                    // diagnostic is not an admission or Ready response.
                    let error=error.chars().take(512).collect::<String>();
                    let packet=format!("{}\n",serde_json::json!({"reference_assembly_error":error}));
                    let _=std::io::Write::write_all(&mut std::io::stdout(),packet.as_bytes());
                    0
                },
                Err(payload) => {
                    std::mem::forget(payload);
                    0
                }
            }
        }
    };
}

/// The existing application run path calls this before destroying its sole CF
/// source. Stop IO, drop the owner OUTSIDE its cell, then join OUTSIDE the worker
/// mutex. This is transport teardown, not proof that original async owners or
/// quarantined boxes drained; native_f1::Host::shutdown retains that obligation.
pub(crate) fn after_loop(engine: Arc<Engine>) -> Result<()> {
    engine.check_owner()?;
    MAIN.with(|slot| -> Result<()> {
        if slot.try_borrow().map_err(|_| "Reference teardown owner busy")?.entered {
            return Err("Reference teardown during active owner callback".into());
        }
        Ok(())
    })?;
    let runtime = RUNTIME
        .get()
        .ok_or("Reference runtime absent at loop teardown")?;
    if !Arc::ptr_eq(engine.model(), &runtime.workspace) {
        return Err("Reference teardown engine/model differs".into());
    }
    if runtime.teardown_started.swap(true, Ordering::AcqRel) {
        return Err("Reference teardown already started".into());
    }
    runtime.stopped.store(true, Ordering::Release);
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let carbon_cleanup = engine.phase().and_then(|phase| {
        if phase == Phase::Terminated {
            super::input_carbon::ReferenceCarbonCleanup::begin(&runtime.reader.admission, &runtime.workspace)
                .map(Some)
        } else {
            super::input_carbon::retire_reference(&runtime.reader.admission, &runtime.workspace).map(|()| None)
        }
    }).inspect_err(|_| {
        let _ = super::input_carbon::retire_reference(&runtime.reader.admission, &runtime.workspace);
    });
    // Close process-table admission before retirement and the wire join.
    // Work admitted before this boundary keeps its existing drain reservation.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let monitor_ingress = super::system_providers::monitor_timeout::close_process_table_ingress();
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let process_cancel = runtime.providers.cancel_jobs();
    // An unconfirmed retirement must fail teardown, but it must not leave
    // independent registrations armed or skip stopping and joining the wire.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let retirement_error = {
        let original_stop = (|| -> Result<()> {
            if engine.phase()? != Phase::Terminated {
                return Ok(());
            }
            let (expander, keyboard) = MAIN.with(|slot| -> Result<_> {
                let slot = slot.try_borrow().map_err(|_| "Reference input custody busy at stop")?;
                if slot.entered {
                    return Err("Reference input stop during active owner callback".into());
                }
                let children = slot.children.as_ref().ok_or("Reference input custody absent at stop")?;
                Ok((children[1].clone(), children[2].clone()))
            })?;
            let snippets = expander.invalidate_input_poll_timer().and_then(|()| {
                expander.stop_original_snippet_expander(&runtime.reader.admission, &runtime.workspace)
            });
            let hyper = keyboard.invalidate_input_poll_timer().and_then(|()| {
                keyboard.stop_original_hyper(&runtime.reader.admission, &runtime.workspace)
            });
            snippets.and(hyper)
        })();
        let retirement = retire_input_actors(&runtime.reader.admission, &runtime.workspace);
        original_stop.and(retirement).err()
    };
    #[cfg(any(test, not(target_arch = "aarch64")))]
    let retirement_error: Option<String> = None;
    // Remove owner and its actual root custody together before any owner
    // destruction can reenter teardown. No callback runs under this borrow.
    let retired = MAIN.with(|slot| -> Result<MainSlot> {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| "Reference teardown owner busy")?;
        if slot.entered {
            return Err("Reference teardown during active owner callback".into());
        }
        Ok(std::mem::replace(&mut *slot, MainSlot::empty()))
    })?;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let (retired, pending_children) = STARTUP_CUSTODY.with(|pending| -> Result<_> {
        let mut pending = pending.try_borrow_mut().map_err(|_| "Reference startup custody busy at teardown")?;
        let mut retired = retired;
        if retired.owner.is_none() {
            retired.root = pending.root.take();
            retired.state = pending.state.take();
        }
        Ok((retired, std::mem::take(&mut pending.children)))
    })?;
    let MainSlot {
        owner,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        root,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        state,
        #[cfg(all(not(test), target_arch = "aarch64"))]
        children,
        entered: _,
    } = retired;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let monitor_timer = root.as_ref().ok_or("Reference monitor root absent at teardown".into())
        .and_then(|root| root.stop_monitor());
    // Keep root, state and all three children through cleanup and worker join.
    // Existing Root Drop quarantines absent final ARC permission; removal from
    // this slot proves no original input/async-owner drain.
    drop(owner);
    let (worker, worker_slot_result) = match runtime.worker.lock() {
        Ok(mut slot) => (slot.take(), Ok(())),
        Err(poisoned) => {
            // Keep custody of the join handle and run the other shutdown
            // checks even when the slot can no longer prove normal teardown.
            (poisoned.into_inner().take(), Err("Reference worker slot poisoned".into()))
        }
    };
    let worker_result = if let Some(worker) = worker {
        match worker.join() {
            Ok(result) => result,
            Err(payload) => {
                std::mem::forget(payload);
                Err("Reference worker teardown panicked".into())
            }
        }
    } else {
        Ok(())
    };
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let startup_cleanup = (|| -> Result<()> {
        if !runtime.field_probe && runtime.configuration_drain.load(Ordering::Acquire) != 2 {
            // Nonterminal Stop and a failed launch do not own a terminal
            // observer drain receipt.
            return Ok(());
        }
        // Check each owner independently of field completion. In particular,
        // a failed field callback must not skip cleanup of the started watcher.
        let observer = state.as_ref()
            .ok_or("Reference configuration observer state absent at termination".to_owned())
            .and_then(|state| state.verify_configuration_observer(false));
        let server = root.as_ref()
            .ok_or("Reference AppDelegate absent at ControlServer termination".to_owned())
            .and_then(|root| root.verify_stopped_control());
        observer.and(server)
            .and_then(|()| runtime.reader.admission.verify_stopped_startup_descriptors())
    })();
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let input_timers = if let Some(children) = &children {
        stop_input_poll_timers(children)
    } else {
        let snippets = pending_children[1].as_ref().map_or(Ok(()), |child| child.invalidate_input_poll_timer());
        let hyper = pending_children[2].as_ref().map_or(Ok(()), |child| child.invalidate_input_poll_timer());
        snippets.and(hyper).and(Err("Reference input children absent at termination".into()))
    };
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let control_cleanup = runtime.reader.admission.verify_control_clients_drained();
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let window_cleanup = (|| -> Result<()> {
        if engine.phase()? != Phase::Terminated {
            return Ok(());
        }
        let field = crate::workspace::receiver::app_delegate::constructor::verify_reference_field_drained(true);
        let open = crate::workspace::callbacks::verify_reference_open_drained();
        let snippets = crate::workspace::objc::verify_reference_snippet_windows_drained(&runtime.workspace);
        // The mounted host may still be borrowed by an unfinished field turn.
        // Its independent cleanup must not depend on unrelated Open or Snippet
        // receipts, but still requires the original field drain.
        let palette = field.and_then(|()| crate::workspace::objc::retire_reference_palette_host(&runtime.workspace));
        palette.and(open).and(snippets)
    })();
    // The original termination and wire join precede provider retirement.
    // Evaluate this even after an input/worker error so those paths cannot
    // leave private process dispatch or Security ingress armed.
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let provider_cleanup = {
        let monitor = super::system_providers::monitor_timeout::drain();
        let image = crate::workspace::objc::controls::retire_reference_status_images(&runtime.workspace);
        let jobs = runtime.providers.verify_jobs_drained();
        let providers = runtime.providers.retire();
        monitor_ingress.and(monitor_timer).and(process_cancel).and(monitor).and(jobs).and(image).and(providers)
    };
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let open_cleanup = crate::workspace::callbacks::verify_reference_open_drained();
    #[cfg(all(not(test), target_arch = "aarch64"))]
    let input_release = if retirement_error.is_none() && input_timers.is_ok() {
        // Only the input prerequisites authorize these original destructors.
        // A failure in an unrelated, already evaluated cleanup must not
        // prevent the stopped input owners from releasing their registrations.
        let snippet = (|| -> Result<()> {
            if engine.phase()? == Phase::Terminated {
                let children = children.as_ref().ok_or("Reference input children absent at final release")?;
                children[1].detach_stopped_snippet_expander(&runtime.reader.admission, &runtime.workspace)?;
            }
            Ok(())
        })();
        let hotkeys = (|| -> Result<()> {
            if let Some(carbon) = carbon_cleanup? {
                let children = children.as_ref().ok_or("Reference input children absent at final release")?;
                children[0].detach_hotkeys_after_stop(&children[2])?;
                carbon.finish()?;
            }
            Ok(())
        })();
        snippet.and(hotkeys)
    } else {
        Ok(())
    };
    retirement_error.map_or(worker_slot_result.and(worker_result), Err)?;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    {
        provider_cleanup?;
        input_timers?;
        startup_cleanup?;
        control_cleanup?;
        window_cleanup?;
        open_cleanup?;
        input_release?;
    }
    Ok(())
}

/// Acknowledge the retained field result, its local cleanup and termination.
/// This grants neither campaign readiness nor full original owner drain.
pub(crate) fn report_field_probe_completion(engine: &Engine, phase: Phase) -> Result<()> {
    let runtime = RUNTIME.get().ok_or("Reference runtime absent after loop teardown")?;
    if !runtime.field_probe {
        return Ok(());
    }
    engine.check_owner()?;
    if phase != Phase::Terminated || !Arc::ptr_eq(engine.model(), &runtime.workspace)
        || !runtime.stopped.load(Ordering::Acquire)
    {
        return Err("Reference field probe did not complete through termination".into());
    }
    let outcome = runtime.field_result.lock().map_err(|_| "Reference field result slot poisoned")?
        .take().ok_or("Reference field result absent at termination")?;
    outcome?;
    runtime.reader.admission.verify_terminal_stdio()?;
    #[cfg(all(not(test), target_arch = "aarch64"))]
    crate::workspace::receiver::app_delegate::constructor::verify_reference_field_drained(false)?;
    native_f1::reference::verify_reply_contexts_drained()?;
    // The joined worker is no longer a writer. One short fixed frame fits
    // within PIPE_BUF. Use a bounded poll before writing to the owned pipe.
    let mut ready = libc::pollfd { fd: 1, events: libc::POLLOUT, revents: 0 };
    if unsafe { libc::poll(&mut ready, 1, 100) } != 1 || ready.revents != libc::POLLOUT {
        return Err("Reference field completion pipe is not writable".into());
    }
    let frame = b"{\"phase\":\"reference-field-complete\"}\n";
    if unsafe { libc::write(1, frame.as_ptr().cast(), frame.len()) } != frame.len() as isize {
        return Err("Reference field completion frame write failed".into());
    }
    Ok(())
}
