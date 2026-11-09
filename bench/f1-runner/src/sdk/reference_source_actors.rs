//! Fixed Reference source-actor producer with the actual class-local callback
//! qualifier. Constructor authority is never widened to callback/ARC permission.
use super::{core_preentry::ReferenceAdmission, owned::Result, reference_runtime::ReferenceSourceActors};
use crate::workspace::{delegate::CallbackCodeAuthority, factory, Workspace};
use objc2::runtime::AnyClass;
use std::{ffi::CStr, sync::{Arc, atomic::{AtomicBool, Ordering}}};

const APP_DELEGATE_CALLBACKS: [&CStr; 10] = [
    c"applicationDidFinishLaunching:", c"applicationWillTerminate:",
    c"openLauncher", c"openClipboard", c"openDashboard", c"openSettings",
    c"openMonitor", c"openConfig", c"reload", c"quit",
];

pub(super) struct SetupFailure {
    retired: Arc<AtomicBool>,
    processes: Option<Arc<super::process_dispatch::Registry>>,
    security: Option<Arc<super::private_network::Context>>,
    _callbacks: Option<Arc<dyn CallbackCodeAuthority>>,
    carbon:Option<Arc<super::input_carbon::Context>>,
    hid:Option<Arc<super::input_hid::Context>>,
    events:Option<std::rc::Rc<super::input_events::Context>>,
    input_prerequisites:Option<Arc<super::input_permissions_caps::Context>>,
    accessibility:Option<Arc<super::accessibility::model::Context>>,
    complete: bool,
}
/// The exact process and Security owners created during Reference assembly.
pub(super) struct RuntimeProviders {
    processes: Arc<super::process_dispatch::Registry>,
    security: Arc<super::private_network::Context>,
}
impl RuntimeProviders {
    #[cfg(target_arch = "aarch64")]
    pub(super) fn cancel_jobs(&self) -> Result<()> {
        super::process_abi::cancel_reference_jobs(&self.processes)
    }

    #[cfg(target_arch = "aarch64")]
    pub(super) fn verify_jobs_drained(&self) -> Result<()> {
        super::process_abi::verify_reference_jobs_drained(&self.processes)
    }

    pub(super) fn retire(&self) -> Result<()> {
        self.processes.retire();
        let website = super::private_network::drain_reference_website(&self.security);
        let security = super::private_network::retire(&self.security);
        website.and(security)
    }
}
impl SetupFailure {
    fn retire(&self) {
        // Pure monotone flags first, before any fallible cleanup or actor drop.
        self.retired.store(true, Ordering::Release);
        if let Some(processes) = &self.processes { processes.retire(); }
        if let Some(security) = &self.security {
            let _ = super::private_network::retire(security);
            let _ = super::private_network::rollback_reference_website(security);
        }
        if let Some(carbon)=&self.carbon {let _=super::input_carbon::retire(carbon);}
        if let Some(hid)=&self.hid {let _=super::input_hid::retire(hid);}
        if let Some(events)=&self.events{let _=super::input_events::retire(events);}
        if let Some(input)=&self.input_prerequisites{let _=super::input_permissions_caps::retire(input);}
        if let Some(ax)=&self.accessibility{let _=super::accessibility::abi::retire(ax);}
    }
    fn checked<T>(&self, result: Result<T>) -> Result<T> {
        result.inspect_err(|_| self.retire())
    }

    pub(super) fn runtime_providers(&self) -> Result<RuntimeProviders> {
        Ok(RuntimeProviders {
            processes: self.processes.clone().ok_or("Reference process owner absent")?,
            security: self.security.clone().ok_or("Reference Security owner absent")?,
        })
    }

    pub(super) fn install_website(&self, admission: Arc<ReferenceAdmission>) -> Result<super::ModuleAttestation> {
        let security = self.security.as_ref().ok_or("Reference Security rollback owner absent")?;
        super::private_network::install_reference_website(admission, security)
    }

    pub(super) fn probe_website(&self) -> Result<()> {
        let security = self.security.as_ref().ok_or("Reference Security probe owner absent")?;
        super::private_network::probe_reference_website(security)
    }

    pub(super) fn verify_keychain_fixture(
        &self,
        admission: &ReferenceAdmission,
        module: &super::ModuleAttestation,
        expected: &[u8],
    ) -> Result<()> {
        self.checked((|| {
            let security = self.security.as_ref().ok_or("Reference Security rollback owner absent")?;
            module.verify()?;
            admission.verify_reference_utility_imports(&module.imports)?;
            let before = security.counts()?;
            let entry = admission.reference_keychain_reader()?;
            let copy = module.imports.iter().find(|row| row.symbol == c"SecItemCopyMatching")
                .ok_or("Reference Keychain copy import absent")?;
            // Both the complete original Swift body and the selected private
            // Security import have been verified in this pre-entry owner.
            unsafe { super::private_network::verify_original_read(entry, copy, security, None) }?;
            let writer = admission.reference_keychain_writer()?;
            let alternate = b"owned synthetic Keychain update";
            if expected == alternate {
                return Err("Reference Keychain update fixture must differ".into());
            }
            for bytes in [expected, alternate.as_slice(), expected] {
                admission.verify_reference_utility_imports(&module.imports)?;
                unsafe {
                    super::private_network::verify_original_write(writer, &module.imports, security, bytes)?;
                    super::private_network::verify_original_read(entry, copy, security, Some(bytes))?;
                }
            }
            admission.verify_reference_utility_imports(&module.imports)?;
            let random = module.imports.iter().find(|row| row.symbol == c"SecRandomCopyBytes")
                .ok_or("Reference entropy import absent")?;
            let entropy = admission.reference_utility_entropy()?;
            unsafe { super::private_network::verify_original_entropy(entropy, random, security) }?;
            let after = security.counts()?;
            if before[0].checked_add(4) != Some(after[0])
                || before[1].checked_add(3) != Some(after[1])
                || before[2].checked_add(1) != Some(after[2])
                || before[3].checked_add(4) != Some(after[3])
            {
                return Err("Reference original Keychain or entropy effects differ".into());
            }
            module.verify()?;
            admission.verify_reference_utility_imports(&module.imports)
        })())
    }

    pub(super) fn finish(&mut self) {
        self.complete = true;
    }
}
impl Drop for SetupFailure {
    fn drop(&mut self) { if !self.complete { self.retire(); } }
}

/// Produce the original AppDelegate construction contribution, its independently
/// qualified callback rows, and the two real private provider contributions.
/// The supplied registry is the assembler's sole early Reference registry.
///
/// The concrete class-local callback qualifier is built here from the SAME
/// context Arc as the constructor. Its real secondary/weak/value-disposal
/// checks remain closed where original source facts are not yet qualified;
/// no external authority, constructor fallback or Ready flag fills that gap.
/// The optional same-build fixture admits exact discovery and one fixture UUID.
///
/// # Safety
/// Exclusive Reference pre-entry, before an original AppDelegate exists. All
/// selected SDK provider/factory ABI and failure cleanup must be source-qualified.
/// Every Result failure retires before propagation. This guard is not a proof
/// of arbitrary nested unwind cleanup: provider initializers must themselves
/// contain unknown failures before their own cleanup. This function allocates no original Swift receiver and runs
/// no original callback, getter, destructor or reference-counting entry.
pub unsafe fn make_reference_sources(
    admission: Arc<ReferenceAdmission>,
    model: Arc<Workspace>,
    registry: &mut super::Registry,
    history: Option<Vec<u8>>,
    entropy: Vec<u8>,
    shortcuts: Option<(Arc<super::process_dispatch::PinnedFake>, std::path::PathBuf)>,
) -> Result<ReferenceSourceActors> {
    // Arm the actual Registry root before even the first fallible prelude.
    // Retain the concrete authority here so dropping a temporary catalog during
    // unwind cannot finalize that authority before this guard's retirement.
    let mut failure = SetupFailure {
        retired: registry.retired.clone(), processes: None, security: None,
        _callbacks: None, carbon:None,hid:None,events:None,input_prerequisites:None,accessibility:None,complete: false,
    };
    failure.checked(registry.verify_reference_issuer(&admission))?;
    failure.checked(model.verify_reference_context(&admission))?;
    let mut processes = failure.checked(super::process_dispatch::Registry::new(
        admission.life(), admission.descriptors(),
    ))?;
    if let Some((fake, script_runner)) = shortcuts {
        for (arguments, mode) in [
            (["list", "--show-identifiers"], "discover"),
            (["run", super::process_router::REFERENCE_SHORTCUT_ID], "run"),
        ] {
            failure.checked(processes.register(
                std::path::Path::new("/usr/bin/shortcuts"),
                &arguments.map(str::to_owned),
                super::process_dispatch::Rule { fake: fake.clone(), fake_args: vec![mode.into()] },
            ))?;
        }
        failure.checked(processes.register(
            std::path::Path::new("/bin/ps"),
            &super::process_router::REFERENCE_PS_ARGS.map(str::to_owned),
            super::process_dispatch::Rule { fake: fake.clone(), fake_args: vec!["monitor-ps".into()] },
        ))?;
        failure.checked(processes.register(
            std::path::Path::new("/usr/bin/mdfind"),
            &super::process_router::reference_file_search_args(),
            super::process_dispatch::Rule { fake: fake.clone(), fake_args: vec!["find-fixture".into()] },
        ))?;
        failure.checked(processes.register(
            &script_runner,
            &["__run-script".into()],
            super::process_dispatch::Rule { fake, fake_args: vec!["script".into()] },
        ))?;
    }
    let processes = Arc::new(processes);
    failure.processes = Some(processes.clone());
    failure.checked(unsafe { factory::prepare_reference_edit_construction(admission.clone(), model.clone()) })?;
    let context = failure.checked(admission.app_delegate_context(model.clone()))?;
    let context = Arc::new(context);
    let class_preparation=failure.checked(unsafe {
        factory::prepare_app_delegate_class(context.clone(),model.clone())
    })?;
    let callbacks = failure.checked(factory::prepare_app_delegate_callback_authority(
        context.clone(), model.clone(),
    ))?;
    failure._callbacks = Some(callbacks.clone());
    let offsets = failure.checked(context.observe_app_delegate_offset_cells())?;
    let handle = failure.checked(super::metadata::loaded_class(c"RuycastF1Reference.AppDelegate"))?;
    if offsets.main_header().checked_add(0x3eb088) != Some(handle.address()) {
        failure.retire();
        return Err("Reference AppDelegate class belongs to another actual Main".into());
    }
    let class: &'static AnyClass = unsafe { &*(handle.address() as *const AnyClass) };
    let source = context.callback_source().clone();
    let rows = vec![(class, source, APP_DELEGATE_CALLBACKS.to_vec())];
    // Qualify the full callback/ARC closure BEFORE publishing any construction
    // grant. The catalog observes entries and lifecycle only; it invokes none.
    let catalog = failure.checked(unsafe {
        crate::workspace::objc::application::SourceCallbackCatalog::qualify(
            model.clone(), callbacks, rows,
        )
    })?;
    failure.checked(context.verify_model(&model))?;
    failure.checked(registry.verify_reference_issuer(&admission))?;
    let prepared = failure.checked(unsafe {
        factory::prepare_app_delegate_constructor(context.clone(), model.clone(),class_preparation)
    })?;
    failure.checked(prepared.grant().verify())?;
    // Requalify AFTER the selected constructor has been installed, using the
    // same independently qualified callback catalog, not its ctor-only sibling.
    let mut selected_methods = Vec::new();
    for selector in APP_DELEGATE_CALLBACKS {
        let selected = failure.checked(crate::workspace::raw_method(
            class, objc2::runtime::Sel::register(selector), false,
        ))?.ok_or_else(|| {
            failure.retire(); "Reference AppDelegate callback disappeared".to_string()
        })?;
        selected_methods.push((selector, selected.implementation()));
    }
    failure.checked(catalog.verify_methods(class, &selected_methods, context.callback_source()))?;
    failure.checked(catalog.verify_cleanup(class, context.callback_source()))?;
    let (input_context,input)=failure.checked(unsafe{super::input_permissions_caps::initialize_reference(admission.clone(),context.clone(),model.clone())})?;
    failure.input_prerequisites=Some(input_context);
    let (ax_context,ax)=failure.checked(unsafe{super::accessibility::abi::initialize_reference(admission.clone(),context.clone(),model.clone())})?;
    failure.accessibility=Some(ax_context);
    let (carbon_context,carbon)=failure.checked(unsafe{super::input_carbon::initialize_reference(admission.clone(),context.clone(),model.clone())})?;
    failure.carbon=Some(carbon_context);
    let (hid_context,hid)=failure.checked(unsafe{super::input_hid::initialize_reference(admission.clone(),context.clone(),model.clone())})?;
    failure.hid=Some(hid_context);
    let (events_context,events)=failure.checked(unsafe{super::input_events::initialize_reference(admission.clone(),context.clone(),model.clone())})?;
    failure.events=Some(events_context);
    let (mut modules, security) = failure.checked(unsafe {
        super::reference_runtime::initialize_provider_modules(
            admission.clone(), history, entropy, processes,
        )
    })?;
    failure.security = Some(security);
    modules.push(carbon);
    modules.push(hid);
    modules.push(events);
    modules.push(input);
    modules.push(ax);
    for module in &modules {
        failure.checked(module.verify())?;
        failure.checked(registry.verify_reference_issuer(&admission))?;
    }
    failure.checked(context.verify_model(&model))?;
    failure.checked(prepared.grant().verify())?;
    failure.checked(registry.verify_reference_issuer(&admission))?;
    // Setup success is not publication or Ready. The existing assembler owns
    // registry registration, complete workspace installation and final gate.
    Ok(ReferenceSourceActors {
        callback_catalog: catalog, modules, rollback: failure,
    })
}
