//! A single binding owns the two overlapping process symbols. Monitor PIDs
//! are synthetic rows; physical PIDs are exclusively registered own children.
//! Unknown and ambiguous IDs never fall through to an original OS provider.
use super::{
    descriptors::DescriptorTable,
    owned::{AliveLease, OwnedHandleLease, OwnedRoot},
    ModuleAttestation, SourceCallsite,
};
use super::{owned::Result, process_abi as jobs, system_providers as monitor, ImportSpec};
use std::sync::{Arc, OnceLock};
use std::{collections::BTreeSet, ffi::c_void};

pub(crate) struct ProcessModule {
    pub(crate) registry: Arc<super::process_dispatch::Registry>,
    descriptors: Arc<DescriptorTable>,
    handles: Vec<OwnedHandleLease>,
    imports: OnceLock<Vec<super::ImportReplacement>>,
    process_info: OnceLock<Vec<super::ObjcReplacement>>,
    reference: Option<Arc<super::core_preentry::ReferenceAdmission>>,
}

#[derive(Clone, Copy)]
enum MainImportRole { Native, Reference }
const REFERENCE_MAIN_IMPORTS: [&std::ffi::CStr;25] = [
    c"posix_spawnp",c"posix_spawn_file_actions_init",c"posix_spawn_file_actions_destroy",
    c"posix_spawn_file_actions_adddup2",c"pipe",c"kill",c"fork",c"execvp",c"open",c"waitpid",c"proc_pidinfo",
    c"sysctlbyname",c"proc_listallpids",c"proc_pidpath",c"mach_host_self",c"mach_port_deallocate",c"getpagesize",
    c"host_statistics",c"host_statistics64",c"statfs",c"sysctl",c"if_indextoname",
    c"IOPSCopyPowerSourcesInfo",c"IOPSCopyPowerSourcesList",c"IOPSGetPowerSourceDescription",
];
const REFERENCE_MAIN_ABSENT: [&std::ffi::CStr;9] = [
    c"posix_spawn",c"posix_spawn_file_actions_addclose",c"posix_spawn_file_actions_addopen",
    c"posix_spawn_file_actions_addinherit_np",c"posix_spawn_file_actions_addchdir_np",
    c"posix_spawn_file_actions_addfchdir_np",c"execve",c"waitid",c"proc_listpids",
];
impl ProcessModule {
    fn role(&self)->MainImportRole {
        if self.reference.is_some() {MainImportRole::Reference} else {MainImportRole::Native}
    }
    fn verify_role_issuer(&self,life:&Arc<dyn AliveLease>)->Result<()> {
        if let Some(admission)=&self.reference {
            if !Arc::ptr_eq(life,&admission.life()) || !Arc::ptr_eq(&self.descriptors,&admission.descriptors()) {
                return Err("process Reference role issuer differs".into());
            }
        }
        Ok(())
    }
}
fn role_universe(role: MainImportRole) -> Result<Vec<ImportSpec>> {
    let universe=import_specs()?;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if matches!(role, MainImportRole::Reference) {
        let mut universe = universe;
        let row = universe.iter_mut().find(|row| row.symbol == c"proc_pidinfo")
            .ok_or("Reference BSD identity import missing from process universe")?;
        row.replacement = reference_pidinfo_entry as *const () as usize;
        let row = universe.iter_mut().find(|row| row.symbol == c"proc_pidpath")
            .ok_or("Reference process path import missing from process universe")?;
        row.replacement = reference_pidpath_entry as *const () as usize;
        let row = universe.iter_mut().find(|row| row.symbol == c"proc_listallpids")
            .ok_or("Reference process count import missing from process universe")?;
        row.replacement = reference_pidcount_entry as *const () as usize;
        return Ok(universe);
    }
    let _ = role;
    Ok(universe)
}

fn role_specs(role:MainImportRole)->Result<Vec<ImportSpec>> {
    let universe=role_universe(role)?;
    if universe.len()!=34 {return Err("process known import universe differs".into());}
    match role {
        MainImportRole::Native=>Ok(universe),
        MainImportRole::Reference=>{
            let present=REFERENCE_MAIN_IMPORTS.into_iter().collect::<BTreeSet<_>>();
            let absent=REFERENCE_MAIN_ABSENT.into_iter().collect::<BTreeSet<_>>();
            let all=universe.iter().map(|row|row.symbol).collect::<BTreeSet<_>>();
            if present.len()!=25 || absent.len()!=9 || !present.is_disjoint(&absent)
                || present.union(&absent).copied().collect::<BTreeSet<_>>()!=all {
                return Err("process fixed Reference role specification differs".into());
            }
            REFERENCE_MAIN_IMPORTS.into_iter().map(|symbol|universe.iter()
                .find(|row|row.symbol==symbol).copied().ok_or_else(||"process Reference leaf absent from source roster".into())).collect()
        },
    }
}

struct InstallRetirement<'a> {
    registry: &'a super::process_dispatch::Registry,
    accepted: bool,
}
impl Drop for InstallRetirement<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            // No dynamic callback, lock, allocation or destructor dispatch.
            self.registry.retire();
        }
    }
}

pub const REFERENCE_SHORTCUT_ID: &str = "00000000-0000-4000-8000-000000000001";
pub const REFERENCE_SCRIPT_RUNNER: &str = "bench/reference/0.8.26/.build/rust/release/ruycast";
pub const REFERENCE_PS_ARGS: [&str; 2] = ["-axo", "pid=,pcpu=,rss="];
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_file_search_args() -> [String; 2] {
    ["-0".into(), ruycast_native::file_search::predicate("Fixture")]
}
pub(super) fn is_reference_monitor_command(path: &std::path::Path, arguments: &[String]) -> bool {
    path == std::path::Path::new("/bin/ps") && arguments == REFERENCE_PS_ARGS
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceTaskRoute {
    Discovery,
    Script,
    ProcessTable,
    FileSearch,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_shortcuts_setter(
    return_pc: usize, arguments_for: Option<ReferenceTaskRoute>,
) -> Result<(ReferenceTaskRoute, std::path::PathBuf, impl Fn() -> Result<()>)> {
    let fixture = monitor::installed_fixture()?;
    let owner = fixture.module.get().ok_or("Reference process module absent")?;
    let admission = owner.reference.as_ref().ok_or("Shortcuts Task has no Reference issuer")?;
    if !Arc::ptr_eq(&owner.registry, &jobs::installed_context()?.registry) {
        return Err("Reference Shortcuts Task registry differs".into());
    }
    owner.registry.before_effect()?;
    let (route, path, check) = admission.verify_shortcuts_setter_caller(return_pc, arguments_for)?;
    owner.registry.not_retired()?;
    Ok((route, path, move || {
        check()?;
        owner.registry.not_retired()
    }))
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_script_write(return_pc: usize, witness: usize) -> Result<impl Fn() -> Result<()>> {
    let fixture = monitor::installed_fixture()?;
    let owner = fixture.module.get().ok_or("Reference process module absent")?;
    let admission = owner.reference.as_ref().ok_or("Script write has no Reference issuer")?;
    if !Arc::ptr_eq(&owner.registry, &jobs::installed_context()?.registry) {
        return Err("Reference script write registry differs".into());
    }
    owner.registry.before_effect()?;
    let check = admission.verify_reference_script_write(return_pc, witness)?;
    owner.registry.not_retired()?;
    Ok(move || {
        check()?;
        owner.registry.not_retired()
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_monitor_environment(return_pc: usize) -> Result<impl Fn() -> Result<()>> {
    let fixture = monitor::installed_fixture()?;
    let owner = fixture.module.get().ok_or("Reference process module absent")?;
    let admission = owner.reference.as_ref().ok_or("Monitor Task has no Reference issuer")?;
    if !Arc::ptr_eq(&owner.registry, &jobs::installed_context()?.registry) {
        return Err("Reference monitor Task registry differs".into());
    }
    owner.registry.before_effect()?;
    let current = admission.verify_reference_monitor_environment(return_pc)?;
    owner.registry.not_retired()?;
    Ok(move || {
        current()?;
        owner.registry.not_retired()
    })
}

pub fn scopes() -> BTreeSet<&'static str> {
    // Main C cells alone do not qualify Foundation Process/NSTask, CF/IOPS
    // object provenance. Shortcuts failures remain outcomes of its exact
    // executable/argv rule; this source route performs no AppleEvent consent call.
    BTreeSet::new()
}

pub fn source_roster() -> BTreeSet<SourceCallsite> {
    use sha2::{Digest, Sha256};
    [
        (
            "Sources/RuycastApp/AppleShortcuts.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/AppleShortcuts.swift")
                .as_slice(),
            "Process runner, bounded stdout/stderr and owned timeout termination",
        ),
        (
            "Sources/RuycastApp/AppState.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/AppState.swift")
                .as_slice(),
            "runScriptProcess exact executable and argv",
        ),
        (
            "Sources/RuycastApp/SystemServices.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/SystemServices.swift")
                .as_slice(),
            "files collector, process identity, terminate and sleep subprocess",
        ),
        (
            "Sources/RuycastApp/SystemSearchSession.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/SystemSearchSession.swift")
                .as_slice(),
            "Ticket launch/detach and cross-thread owned Process cancellation",
        ),
        (
            "Sources/RuycastApp/ResourceSampler.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/ResourceSampler.swift")
                .as_slice(),
            "owned host/process/network/battery inventory and ps collector",
        ),
        (
            "Sources/RuycastApp/CommandEditor.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/CommandEditor.swift")
                .as_slice(),
            "config editor CLI child and sole wait",
        ),
        (
            "rust-native/src/os_command.rs",
            include_bytes!("../../../../rust-native/src/os_command.rs").as_slice(),
            "bounded command spawn, cancellation and sole caller wait",
        ),
        (
            "rust-native/src/supervision.rs",
            include_bytes!("../../../../rust-native/src/supervision.rs").as_slice(),
            "owned CLI supervision pipe, exact child executable and cancellation",
        ),
        (
            "rust-native/src/system_search/mod.rs",
            include_bytes!("../../../../rust-native/src/system_search/mod.rs").as_slice(),
            "Ticket deadline/live admission and Shortcuts provider interface",
        ),
        (
            "rust-native/src/system_search/apple_shortcuts.rs",
            include_bytes!("../../../../rust-native/src/system_search/apple_shortcuts.rs")
                .as_slice(),
            "Shortcuts list/run argv, bounded private pipes and own child cleanup",
        ),
        (
            "rust-native/src/resource_sampler.rs",
            include_bytes!("../../../../rust-native/src/resource_sampler.rs").as_slice(),
            "Native sampler ProcessInfo, fixed monitor imports and CF battery projection",
        ),
        (
            "rust-native/src/processes.rs",
            include_bytes!("../../../../rust-native/src/processes.rs").as_slice(),
            "process identity path and selected termination",
        ),
        (
            "rust-native/src/system_search/native.rs",
            include_bytes!("../../../../rust-native/src/system_search/native.rs").as_slice(),
            "process identity admission before provider operations",
        ),
    ]
    .into_iter()
    .map(|(file, bytes, operation)| SourceCallsite {
        file,
        source_sha256: format!("{:x}", Sha256::digest(bytes)),
        operation,
    })
    .collect()
}

fn root_identity(root: &OwnedRoot) -> Result<(u64, u64)> {
    use std::os::fd::AsRawFd;
    let fd = root.checked(root.path())?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0 {
        return Err("process owned root metadata".into());
    }
    Ok((stat.st_dev as u64, stat.st_ino))
}

fn import_keys(
    rows: &[super::ImportReplacement],
) -> BTreeSet<(&std::ffi::CStr, usize, usize, usize)> {
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

pub(crate) fn verify_attestation(module: &ModuleAttestation) -> Result<()> {
    let fixture = monitor::installed_fixture()?;
    let owner = fixture
        .module
        .get()
        .ok_or("process module was not source-installed")?;
    let jobs = jobs::installed_context()?;
    let actual = super::bootstrap::owned_module_context(&jobs.life, &owner.descriptors)?;
    verify_binding(module, fixture, jobs, &actual)
}

fn verify_binding(
    module: &ModuleAttestation,
    fixture: &Arc<monitor::Fixture>,
    jobs: &jobs::Context,
    actual: &super::owned::ModuleContext,
) -> Result<()> {
    if module.module != "process-monitor"
        || module.scopes != scopes()
        || module.callsites != source_roster()
        || module.workspace_factories.is_some()
        || module.handles.len() != 4
    {
        return Err("process module envelope differs".into());
    }
    let owner = fixture
        .module
        .get()
        .ok_or("process module was not source-installed")?;
    let expected = owner.process_info.get().map(Vec::as_slice).unwrap_or_default();
    if expected.len() != if owner.reference.is_some() { 4 } else { 0 }
        || module.objc.len() != expected.len()
        || module.objc.iter().zip(expected).any(|(actual, expected)| {
            (actual.class, actual.selector, actual.class_method, actual.original, actual.replacement)
                != (expected.class, expected.selector, expected.class_method, expected.original, expected.replacement)
        })
    {
        return Err("process module ProcessInfo selectors differ".into());
    }
    if owner.handles.len() != 4 || jobs.owner_uid != actual.home.owner_ids()?.0 {
        return Err("process owned roots/account differs".into());
    }
    if !Arc::ptr_eq(&owner.registry, &jobs.registry)
        || !Arc::ptr_eq(&jobs.life, &fixture.context.life)
        || !Arc::ptr_eq(&jobs.life, &actual.life)
    {
        return Err("process installed Registry/fixture identity differs".into());
    }
    owner
        .registry
        .matches_issuer(&jobs.life, &owner.descriptors)?;
    owner.verify_role_issuer(&jobs.life)?;
    for ((((handle, retained), root), fixture_root), namespace) in module
        .handles
        .iter()
        .zip(&owner.handles)
        .zip([
            &actual.home,
            &actual.cache,
            &actual.temp,
            &actual.applications,
        ])
        .zip([
            &fixture.context.home,
            &fixture.context.cache,
            &fixture.context.temp,
            &fixture.context.applications,
        ])
        .zip([
            "process-monitor.home",
            "process-monitor.cache",
            "process-monitor.temp",
            "process-monitor.applications",
        ])
    {
        if handle.namespace != namespace
            || retained.namespace != namespace
            || !Arc::ptr_eq(&handle.life, &jobs.life)
            || !Arc::ptr_eq(&retained.life, &jobs.life)
            || handle.root.path() != root.path()
            || retained.root.path() != root.path()
            || fixture_root.path() != root.path()
            || root_identity(&handle.root)? != root_identity(root)?
            || root_identity(fixture_root)? != root_identity(root)?
            || root_identity(&retained.root)? != root_identity(root)?
        {
            return Err("process retained root/guardian differs".into());
        }
        handle.before_effect()?;
    }
    let imports = owner.imports.get().ok_or("process imports unpublished")?;
    if imports.is_empty()
        || imports.len() != module.imports.len()
        || import_keys(imports).len() != imports.len()
        || import_keys(imports) != import_keys(&module.imports)
    {
        return Err("process import-cell identity differs".into());
    }
    verify_imports(owner.role(),imports)?;
    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    if owner.reference.is_some() {
        monitor::verify_reference_power(fixture)?;
    }
    owner.registry.before_effect()?;
    // This full-universe read is AFTER the last dynamic guard. A formerly
    // absent Reference symbol must not silently become an unpatched cell.
    if verify_imports(owner.role(),imports).is_err() {
        owner.registry.retire();
        return Err("process import changed during guardian check".into());
    }
    if owner.reference.is_some() {
        // Pure actual clock cells/source/code only, after the final dynamic
        // guardian. Native's fixed34 route has no Swift Date constructor.
        monitor::verify_reference_clock(fixture)?;
        #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
        monitor::monitor_timeout::verify(fixture)?;
    }
    owner.registry.not_retired()
}

fn verify_imports(role:MainImportRole,imports: &[super::ImportReplacement]) -> Result<()> {
    let specs = role_specs(role)?;
    let expected = specs.iter().map(|s| s.symbol).collect::<BTreeSet<_>>();
    let observed=super::observe_main_imports(&role_universe(role)?)?;
    if expected.len()!=match role {MainImportRole::Native=>34,MainImportRole::Reference=>25}
        || imports.iter().map(|i| i.symbol).collect::<BTreeSet<_>>() != expected
        || observed.iter().map(|i|i.symbol).collect::<BTreeSet<_>>()!=expected
        || import_keys(&observed)!=import_keys(imports)
        || observed.len()!=imports.len()
        || imports.iter().any(|row| {
            !specs.iter().any(|spec| {
                spec.symbol == row.symbol
                    && spec.original == row.original
                    && spec.replacement == row.replacement
            }) || row.selected_address() != row.replacement
        })
    {
        return Err("process selected import roster differs".into());
    }
    Ok(())
}

pub(crate) fn attestation() -> Result<ModuleAttestation> {
    let fixture = monitor::installed_fixture()?;
    let owner = fixture
        .module
        .get()
        .ok_or("process module was not source-installed")?;
    let module = ModuleAttestation {
        module: "process-monitor",
        scopes: scopes(),
        callsites: source_roster(),
        imports: owner
            .imports
            .get()
            .ok_or("process imports unpublished")?
            .clone(),
        objc: owner.process_info.get().cloned().unwrap_or_default(),
        handles: owner.handles.clone(),
        workspace_factories: None,
    };
    verify_attestation(&module)?;
    Ok(module)
}

/// Install the fixed combined module using the actual full-Host issuer.
/// # Safety
/// The sealed coordinator has qualified this source/image/ABI and every actual
/// main-image import cell. Registry rules are already source-pinned owned fakes;
/// no arbitrary executable or original provider is admitted. prewrite must be
/// a pure final source validator, with no guarded allocation or callback.
pub(crate) unsafe fn initialize_owned(
    life: Arc<dyn AliveLease>,
    descriptors: Arc<DescriptorTable>,
    registry: Arc<super::process_dispatch::Registry>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<ModuleAttestation> {
    unsafe {initialize_for_role(life,descriptors,registry,prewrite,None)}
}

/// Separate sealed Reference role; there is no caller-supplied role flag or
/// JSON roster. The admitted Main/source/compiler profile must freshly match
/// the fixed25/absent9 contract before this initializer can succeed.
#[cfg(all(not(test), rcf1_reference_sdk))]
pub(crate) unsafe fn initialize_reference(
    admission: Arc<super::core_preentry::ReferenceAdmission>,
    registry: Arc<super::process_dispatch::Registry>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<ModuleAttestation> {
    admission.before_reference_initialization()?;
    admission.verify_process_source_members()?;
    let life=admission.life();
    let descriptors=admission.descriptors();
    unsafe {initialize_for_role(life,descriptors,registry,prewrite,Some(admission))}
}

unsafe fn initialize_for_role(
    life: Arc<dyn AliveLease>,
    descriptors: Arc<DescriptorTable>,
    registry: Arc<super::process_dispatch::Registry>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
    reference: Option<Arc<super::core_preentry::ReferenceAdmission>>,
) -> Result<ModuleAttestation> {
    registry.matches_issuer(&life, &descriptors)?;
    let role=if reference.is_some() {MainImportRole::Reference} else {MainImportRole::Native};
    let specs=role_specs(role)?;
    let universe=role_universe(role)?;
    let observed=super::observe_main_imports(&universe)?;
    if observed.iter().map(|row|row.symbol).collect::<BTreeSet<_>>()
        !=specs.iter().map(|row|row.symbol).collect::<BTreeSet<_>>() {
        return Err("process Main role presence/absence differs before publication".into());
    }
    let context = super::bootstrap::owned_module_context(&life, &descriptors)?;
    let handles = [
        ("process-monitor.home", context.home.clone()),
        ("process-monitor.cache", context.cache.clone()),
        ("process-monitor.temp", context.temp.clone()),
        ("process-monitor.applications", context.applications.clone()),
    ]
    .into_iter()
    .map(|(namespace, root)| OwnedHandleLease {
        namespace,
        root,
        life: life.clone(),
    })
    .collect();
    let owner_uid = context.home.owner_ids()?.0;
    let fixture = Arc::new(match reference.as_ref() {
        Some(admission)=>monitor::Fixture::new_at_epoch(context,admission.fixture_epoch_unix_seconds()?)?,
        None=>monitor::Fixture::new(context)?,
    });
    fixture
        .module
        .set(ProcessModule {
            registry: registry.clone(),
            descriptors,
            handles,
            imports: OnceLock::new(),
            process_info: OnceLock::new(),
            reference,
        })
        .map_err(|_| "process module already bound")?;
    prewrite()?;
    jobs::install(jobs::Context {
        registry: registry.clone(),
        life,
        owner_uid,
    })?;
    let mut retirement = InstallRetirement {
        registry: &registry,
        accepted: false,
    };
    if let Err(error) = fixture.clone().install() {
        registry.retire();
        return Err(error);
    }
    struct Guard<'a, F> {
        registry: &'a super::process_dispatch::Registry,
        prewrite: &'a F,
    }
    impl<F: Fn() -> Result<()> + Send + Sync> AliveLease for Guard<'_, F> {
        fn is_alive(&self) -> bool {
            self.before_effect().is_ok()
        }
        fn before_effect(&self) -> Result<()> {
            self.registry.before_effect()?;
            (self.prewrite)()?;
            self.registry.not_retired()
        }
    }
    let result = (|| {
        let imports = super::import_patch::install_exact_presence(
            &specs,
            &universe,
            &Guard {
                registry: &registry,
                prewrite: &prewrite,
            },
        )?;
        fixture
            .module
            .get()
            .ok_or("process module disappeared")?
            .imports
            .set(imports)
            .map_err(|_| "process imports already published")?;
        if let Some(admission)=fixture.module.get().and_then(|module|module.reference.clone()) {
            #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
            {
                monitor::monitor_timeout::install(&fixture, admission.clone(), &Guard{registry:&registry,prewrite:&prewrite})?;
                monitor::install_reference_clock(&fixture,admission.clone(),&Guard{registry:&registry,prewrite:&prewrite})?;
                monitor::prepare_reference_power(&fixture)?;
                let methods = monitor::monitor_process_info::install(&fixture, admission)?;
                fixture.module.get().ok_or("Reference process module absent")?.process_info
                    .set(methods).map_err(|_| "Reference ProcessInfo attestation already installed")?;
            }
            #[cfg(not(all(not(test), rcf1_reference_sdk, target_arch = "aarch64")))]
            {let _=admission;return Err("Reference Swift clock ABI is not compiled for this role".into());}
        }
        attestation()
    })();
    retirement.accepted = result.is_ok();
    result
}
fn denied() -> i32 {
    unsafe { *libc::__error() = libc::EPERM };
    -1
}
pub(crate) unsafe extern "C" fn kill(pid: i32, signal: i32) -> i32 {
    jobs::parent_boundary(-1, || {
        if !jobs::routes_live() {
            return denied();
        }
        let Some(key) = pid.checked_abs().filter(|key| *key > 1) else {
            return denied();
        };
        match (jobs::registered_job(key), monitor::contains_virtual(key)) {
            (true, false) => unsafe { jobs::kill(pid, signal) },
            (false, true) if pid > 1 => unsafe { monitor::owned_virtual_kill(pid, signal) },
            _ => denied(),
        }
    })
}
pub(crate) unsafe extern "C" fn proc_pidinfo(
    pid: i32,
    flavor: i32,
    arg: u64,
    out: *mut c_void,
    capacity: i32,
) -> i32 {
    jobs::parent_boundary(-1, || {
        if !jobs::routes_live() || pid <= 1 {
            return denied();
        }
        match (jobs::registered_job(pid), monitor::contains_virtual(pid)) {
            (true, false) => unsafe { jobs::job_proc_pidinfo(pid, flavor, arg, out, capacity) },
            (false, true) => unsafe {
                monitor::owned_proc_pidinfo(pid, flavor, arg, out, capacity)
            },
            _ => denied(),
        }
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn reference_pidinfo_entry(_: i32, _: i32, _: u64, _: *mut c_void, _: i32) -> i32 {
    core::arch::naked_asm!("mov x5, x30", "b {checked}", checked = sym reference_pidinfo_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_pidinfo_checked(
    pid: i32, flavor: i32, arg: u64, out: *mut c_void, capacity: i32, caller: usize,
) -> i32 {
    jobs::parent_boundary(-1, || {
        if flavor != libc::PROC_PIDTBSDINFO || arg != 0
            || capacity != std::mem::size_of::<libc::proc_bsdinfo>() as i32
            || out.is_null() || !out.cast::<libc::proc_bsdinfo>().is_aligned()
            || verify_reference_process_import(caller, c"proc_pidinfo").is_err()
        {
            return denied();
        }
        unsafe { proc_pidinfo(pid, flavor, arg, out, capacity) }
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn reference_pidpath_entry(_: i32, _: *mut c_void, _: u32) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym reference_pidpath_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_pidpath_checked(pid: i32, out: *mut c_void, capacity: u32, caller: usize) -> i32 {
    jobs::parent_boundary(-1, || {
        if pid <= 1 || out.is_null() || capacity != 4096
            || verify_reference_process_import(caller, c"proc_pidpath").is_err()
        {
            return denied();
        }
        unsafe { monitor::owned_proc_pidpath(pid, out, capacity) }
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn verify_reference_process_import(caller: usize, symbol: &std::ffi::CStr) -> Result<()> {
    let fixture = monitor::installed_fixture()?;
    let module = fixture.module.get().ok_or("Reference process module absent")?;
    let admission = module.reference.as_ref().ok_or("Reference process issuer absent")?;
    let cell = admission.verify_reference_process_read(caller, symbol)?;
    let imports = module.imports.get().ok_or("Reference process imports absent")?;
    let import = imports.iter().find(|row| row.symbol == symbol).ok_or("Reference process import absent")?;
    if import.cell_address() != cell || import.selected_address() != import.replacement {
        return Err("Reference process import differs".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn reference_pidcount_entry(_: *mut c_void, _: i32) -> i32 {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym reference_pidcount_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_pidcount_checked(buffer: *mut c_void, capacity: i32, caller: usize) -> i32 {
    jobs::parent_boundary(-1, || {
        if !buffer.is_null() || capacity != 0
            || verify_reference_process_import(caller, c"proc_listallpids").is_err()
        {
            return denied();
        }
        unsafe { monitor::owned_proc_listallpids(buffer, capacity) }
    })
}
#[cfg(all(rcf1_reference_sdk, not(test)))]
pub const REFERENCE_SAFARI_PID: i32 = 730002;

#[cfg(all(rcf1_reference_sdk, not(test)))]
pub(crate) fn register_reference_browser_process(
    admission: &Arc<super::core_preentry::ReferenceAdmission>,
    model: &Arc<crate::workspace::Workspace>,
    pid: i32,
) -> Result<()> {
    if pid != REFERENCE_SAFARI_PID {
        return Err("Reference Safari process is outside the fixed fixture".into());
    }
    model.verify_reference_context(admission)?;
    let fixture = monitor::installed_fixture()?;
    let module = fixture.module.get().ok_or("Reference Safari process module absent")?;
    if !module.reference.as_ref().is_some_and(|owner| Arc::ptr_eq(owner, admission)) {
        return Err("Reference Safari process module issuer differs".into());
    }
    monitor::verify_same_fixture(fixture)?;
    let uid = fixture.context.home.owner_ids()?.0;
    let row = monitor::ProcessRow {
        pid,
        uid,
        started_seconds: 33,
        started_micros: 44,
        path: "/owned/Safari fixture".into(),
        cpu: 0.,
        rss_kib: 0,
    };
    jobs::with_no_registered_job(pid, || {
        module.registry.not_retired()?;
        let mut rows = fixture.processes.lock().map_err(|_| "monitor rows poisoned")?;
        if rows.contains_key(&pid) {
            return Err("Reference Safari process registration replay".into());
        }
        rows.insert(pid, row);
        Ok(())
    })
}

/// Capture the identity using the SAME combined C route original SystemServices
/// calls. All real role/root guards run outside the final data locks.
#[cfg(all(rcf1_reference_sdk,not(test)))]
pub(crate) fn reference_browser_identity(model:&Arc<crate::workspace::Workspace>,pid:i32)->Result<monitor::ProcessIdentity> {
    let fixture=monitor::installed_fixture()?;
    let owner=fixture.module.get().ok_or("Reference process module absent")?;
    let admission=owner.reference.as_ref().ok_or("browser identity has no actual Reference issuer")?;
    admission.verify_private_security_source_members()?;
    reference_workspace_identity(model,pid)
}

#[cfg(all(rcf1_reference_sdk,not(test)))]
pub(crate) fn reference_workspace_identity(model:&Arc<crate::workspace::Workspace>,pid:i32)->Result<monitor::ProcessIdentity> {
    let fixture=monitor::installed_fixture()?.clone();
    let owner=fixture.module.get().ok_or("Reference process module absent")?;
    let admission=owner.reference.as_ref().ok_or("workspace identity has no actual Reference issuer")?;
    model.verify_reference_context(admission)?;
    admission.verify_input_context(&fixture.context)?;
    monitor::verify_same_fixture(&fixture)?;
    let lease=model.lease();
    if !Arc::ptr_eq(&lease.life,&fixture.context.life)||lease.root.path()!=fixture.context.home.path()
        ||root_identity(&lease.root)?!=root_identity(&fixture.context.home)? {
        return Err("process/workspace root issuer differs".into());
    }
    let expected=fixture.processes.lock().map_err(|_|"monitor rows poisoned")?
        .get(&pid).ok_or("workspace PID has no synthetic process row")?.identity();
    if expected.pid!=pid||expected.pid<=1||expected.uid!=lease.root.owner_ids()?.0||expected.micros>=1_000_000 {
        return Err("workspace synthetic identity owner/shape differs".into());
    }
    let mut info:libc::proc_bsdinfo=unsafe{std::mem::zeroed()};
    let size=std::mem::size_of::<libc::proc_bsdinfo>();
    let actual=unsafe{proc_pidinfo(pid,libc::PROC_PIDTBSDINFO,0,(&mut info as *mut libc::proc_bsdinfo).cast(),size as i32)};
    monitor::verify_same_fixture(&fixture)?;
    model.verify_reference_context(admission)?;
    with_virtual_identity(expected,|| {
        if actual!=size as i32||info.pbi_pid!=pid as u32||info.pbi_uid!=expected.uid
            ||info.pbi_start_tvsec!=expected.seconds||info.pbi_start_tvusec!=expected.micros {
            return Err("routed workspace BSD identity differs".into());
        }
        Ok(expected)
    })
}
/// Short callback-free commit: model -> Wire -> jobs -> virtual rows. The row
/// and unambiguous router classification stay locked through the effect.
pub(crate) fn with_virtual_identity<T>(identity:monitor::ProcessIdentity,commit:impl FnOnce()->Result<T>)->Result<T> {
    jobs::with_no_registered_job(identity.pid,||monitor::with_process_identity(identity,commit))
}
pub fn import_specs() -> Result<Vec<ImportSpec>> {
    let mut out = jobs::import_specs();
    let mut replaced = BTreeSet::new();
    for spec in &mut out {
        if spec.symbol == c"kill" {
            spec.replacement = kill as *const () as usize;
            replaced.insert(spec.symbol);
        } else if spec.symbol == c"proc_pidinfo" {
            spec.replacement = proc_pidinfo as *const () as usize;
            replaced.insert(spec.symbol);
        }
    }
    if replaced.len() != 2 {
        return Err("missing process router binding".into());
    }
    for spec in monitor::import_specs() {
        if spec.symbol == c"proc_pidinfo" {
            let shared = out
                .iter()
                .find(|s| s.symbol == spec.symbol)
                .ok_or("missing shared process original")?;
            if shared.original != spec.original {
                return Err("process original identity conflict".into());
            }
        } else {
            out.push(spec);
        }
    }
    let mut names = BTreeSet::new();
    for spec in &out {
        if !names.insert(spec.symbol) {
            return Err("duplicate process import authority".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod module_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn browser_identity_uses_combined_c_route_and_locked_stable_fields() {
        let workspace=crate::workspace::tests::Fixture::new();
        let context=super::super::owned::ModuleContext {
            home:OwnedRoot::open(&workspace.root.join("home")).unwrap(),
            cache:OwnedRoot::open(&workspace.root.join("cache")).unwrap(),
            temp:OwnedRoot::open(&workspace.root.join("temp")).unwrap(),
            applications:OwnedRoot::open(&workspace.root.join("apps")).unwrap(),
            life:workspace.life.clone(),
        };
        let registry=Arc::new(super::super::process_dispatch::Registry::new(context.life.clone(),DescriptorTable::new()).unwrap());
        jobs::install(jobs::Context{registry:registry.clone(),life:context.life.clone(),owner_uid:context.home.owner_ids().unwrap().0}).unwrap();
        let fixture=Arc::new(monitor::Fixture::new(context).unwrap());
        fixture.clone().install().unwrap();
        let expected=fixture.processes.lock().unwrap().get(&730001).unwrap().identity();
        assert_eq!(unsafe { monitor::owned_proc_listallpids(std::ptr::null_mut(), 0) }, 1);
        let mut listed = [-1i32; 2];
        assert_eq!(unsafe { monitor::owned_proc_listallpids(listed.as_mut_ptr().cast(), 8) }, 1);
        assert_eq!(listed, [expected.pid, -1]);
        let mut info:libc::proc_bsdinfo=unsafe{std::mem::zeroed()};
        let size=std::mem::size_of::<libc::proc_bsdinfo>();
        assert_eq!(unsafe{proc_pidinfo(expected.pid,libc::PROC_PIDTBSDINFO,0,(&mut info as *mut libc::proc_bsdinfo).cast(),size as i32)},size as i32);
        assert_eq!((info.pbi_pid,info.pbi_uid,info.pbi_start_tvsec,info.pbi_start_tvusec),
            (expected.pid as u32,expected.uid,expected.seconds,expected.micros));
        let mut path = [0xa5u8; 64];
        let expected_path = b"/owned/Owned Editor\0";
        assert_eq!(unsafe { monitor::owned_proc_pidpath(expected.pid, path.as_mut_ptr().cast(), (expected_path.len() - 1) as u32) }, -1);
        assert_eq!(path, [0xa5; 64]);
        assert_eq!(unsafe { monitor::owned_proc_pidpath(expected.pid, path.as_mut_ptr().cast(), path.len() as u32) }, (expected_path.len() - 1) as i32);
        assert_eq!(&path[..expected_path.len()], expected_path);
        let mut output=vec![0xa5u8;size];
        assert_eq!(unsafe{proc_pidinfo(730002,libc::PROC_PIDTBSDINFO,0,output.as_mut_ptr().cast(),size as i32)},-1);
        assert!(output.iter().all(|byte|*byte==0xa5));
        assert_eq!(unsafe{proc_pidinfo(expected.pid,libc::PROC_PIDTBSDINFO,0,output.as_mut_ptr().cast(),(size-1) as i32)},-1);
        assert!(output.iter().all(|byte|*byte==0xa5));
        with_virtual_identity(expected,||{
            assert!(fixture.processes.try_lock().is_err(),"row classification must stay locked through commit");Ok(())
        }).unwrap();
        {let mut rows=fixture.processes.lock().unwrap();let row=rows.get_mut(&expected.pid).unwrap();row.cpu+=1.;row.rss_kib+=1;}
        assert!(with_virtual_identity(expected,||Ok(())).is_ok(),"CPU/RSS changes are not identity changes");
        fixture.processes.lock().unwrap().get_mut(&expected.pid).unwrap().started_micros+=1;
        let mut committed=false;
        assert!(with_virtual_identity(expected,||{committed=true;Ok(())}).is_err());assert!(!committed);
        assert_eq!(unsafe { kill(expected.pid, 0) }, 0);
        assert_eq!(unsafe { kill(expected.pid, libc::SIGTERM) }, -1);
        assert!(fixture.events.lock().unwrap().is_empty());
        let current = fixture.processes.lock().unwrap()[&expected.pid].clone();
        assert_eq!(unsafe { kill(expected.pid, libc::SIGKILL) }, 0);
        assert_eq!(unsafe { monitor::owned_proc_listallpids(std::ptr::null_mut(), 0) }, 0);
        path.fill(0xa5);
        assert_eq!(unsafe { monitor::owned_proc_pidpath(expected.pid, path.as_mut_ptr().cast(), path.len() as u32) }, -1);
        assert_eq!(path, [0xa5; 64]);
        assert_eq!(&*fixture.events.lock().unwrap(), &[format!("virtual.force-quit:{}", expected.pid)]);
        assert!(with_virtual_identity(expected,||Ok(())).is_err());
        assert_eq!(unsafe { kill(expected.pid, libc::SIGKILL) }, -1);
        fixture.processes.lock().unwrap().insert(current.pid, current);
        registry.retire();
        assert_eq!(unsafe { kill(expected.pid, libc::SIGKILL) }, -1);
        assert!(fixture.processes.lock().unwrap().contains_key(&expected.pid));
        assert_eq!(fixture.events.lock().unwrap().len(), 1);
        assert_eq!(unsafe{proc_pidinfo(expected.pid,libc::PROC_PIDTBSDINFO,0,output.as_mut_ptr().cast(),size as i32)},-1);
        assert!(output.iter().all(|byte|*byte==0xa5));
    }

    struct Life(AtomicBool);
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<()> {
            if self.is_alive() {
                Ok(())
            } else {
                Err("own guardian retired".into())
            }
        }
    }
    fn life() -> Arc<dyn AliveLease> {
        Arc::new(Life(AtomicBool::new(true)))
    }

    #[test]
    fn process_module_owns_exact_combined_imports_without_alias_authority() {
        let specs = import_specs().unwrap();
        assert_eq!(specs.len(), 34);
        assert_eq!(
            specs
                .iter()
                .map(|s| s.symbol)
                .collect::<BTreeSet<_>>()
                .len(),
            34
        );
        for symbol in [c"kill", c"proc_pidinfo"] {
            assert_eq!(specs.iter().filter(|s| s.symbol == symbol).count(), 1);
        }
        assert_eq!(
            specs
                .iter()
                .find(|s| s.symbol == c"kill")
                .unwrap()
                .replacement,
            kill as *const () as usize
        );
        assert_eq!(
            specs
                .iter()
                .find(|s| s.symbol == c"proc_pidinfo")
                .unwrap()
                .replacement,
            proc_pidinfo as *const () as usize
        );
        assert!(specs
            .iter()
            .all(|s| s.original != 0 && s.replacement != 0 && s.original != s.replacement));
        let reference=role_specs(MainImportRole::Reference).unwrap();
        assert_eq!(reference.len(),25);
        assert!(REFERENCE_MAIN_ABSENT.iter().all(|symbol|!reference.iter().any(|row|row.symbol==*symbol)));
        assert!(reference.iter().all(|row|specs.iter().any(|native|
            native.symbol==row.symbol&&native.original==row.original&&native.replacement==row.replacement)));
        assert_eq!(role_specs(MainImportRole::Native).unwrap().len(),34);
        assert!(!specs.iter().any(|s| [
            c"SecItemCopyMatching",
            c"AEDeterminePermissionToAutomateTarget"
        ]
        .contains(&s.symbol)));
        let sources = source_roster();
        assert_eq!(sources.len(), 13);
        for file in [
            "Sources/RuycastApp/SystemSearchSession.swift",
            "rust-native/src/system_search/mod.rs",
            "rust-native/src/resource_sampler.rs",
        ] {
            assert_eq!(sources.iter().filter(|row| row.file == file).count(), 1);
        }
        assert!(scopes().is_empty());
        assert!(source_roster()
            .iter()
            .all(|row| row.source_sha256.len() == 64));
    }

    #[test]
    fn process_module_requires_same_guardian_and_descriptor_issuer() {
        let life = life();
        let descriptors = DescriptorTable::new();
        let registry =
            super::super::process_dispatch::Registry::new(life.clone(), descriptors.clone())
                .unwrap();
        assert!(registry
            .matches_issuer(&life.clone(), &descriptors.clone())
            .is_ok());
        assert!(registry
            .matches_issuer(&self::life(), &descriptors)
            .is_err());
        assert!(registry
            .matches_issuer(&life, &DescriptorTable::new())
            .is_err());
        registry.retire();
        assert!(registry.matches_issuer(&life, &descriptors).is_err());
        assert!(registry.pipe().is_err());
        assert!(registry.null(libc::O_RDONLY).is_err());
    }

    #[test]
    fn process_module_retired_guardian_and_unwind_revoke_new_effects() {
        let held = Arc::new(Life(AtomicBool::new(true)));
        let life: Arc<dyn AliveLease> = held.clone();
        let descriptors = DescriptorTable::new();
        let registry =
            super::super::process_dispatch::Registry::new(life.clone(), descriptors.clone())
                .unwrap();
        assert!(registry.before_effect().is_ok());
        held.0.store(false, Ordering::Release);
        assert!(registry.matches_issuer(&life, &descriptors).is_err());
        assert!(registry.pipe().is_err());

        let registry =
            super::super::process_dispatch::Registry::new(self::life(), DescriptorTable::new())
                .unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _publication = InstallRetirement {
                registry: &registry,
                accepted: false,
            };
            panic!("own post-publication validator panic");
        }));
        assert!(panic.is_err());
        assert!(registry.before_effect().is_err());
        assert!(registry.pipe().is_err());

        let registry =
            super::super::process_dispatch::Registry::new(self::life(), DescriptorTable::new())
                .unwrap();
        {
            let _accepted = InstallRetirement {
                registry: &registry,
                accepted: true,
            };
        }
        assert!(registry.before_effect().is_ok());
    }

    #[test]
    fn process_module_reentrant_final_guardian_cannot_restore_retired_authority() {
        struct Reentrant(
            std::sync::Mutex<Option<std::sync::Weak<super::super::process_dispatch::Registry>>>,
        );
        impl AliveLease for Reentrant {
            fn is_alive(&self) -> bool {
                if let Some(registry) = self
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|weak| weak.upgrade())
                {
                    registry.retire();
                }
                true
            }
            fn before_effect(&self) -> Result<()> {
                Ok(())
            }
        }
        let owner = Arc::new(Reentrant(std::sync::Mutex::new(None)));
        let registry = Arc::new(
            super::super::process_dispatch::Registry::new(owner.clone(), DescriptorTable::new())
                .unwrap(),
        );
        *owner.0.lock().unwrap() = Some(Arc::downgrade(&registry));
        assert!(registry.before_effect().is_err());
        assert!(registry.not_retired().is_err());
        assert!(registry.pipe().is_err());
    }

    #[test]
    fn process_module_retirement_preserves_held_descriptor_cleanup() {
        use std::os::fd::AsRawFd;
        let registry =
            super::super::process_dispatch::Registry::new(life(), DescriptorTable::new()).unwrap();
        let (read, write) = registry.pipe().unwrap();
        let reader = read.as_raw_fd();
        let writer = write.as_raw_fd();
        registry.retire();
        assert!(registry.pipe().is_err());
        assert!(unsafe { libc::fcntl(reader, libc::F_GETFD) } >= 0);
        assert!(unsafe { libc::fcntl(writer, libc::F_GETFD) } >= 0);
        // OwnedFd remains the sole closer after business admission retires.
        drop(read);
        drop(write);
        assert_eq!(unsafe { libc::fcntl(reader, libc::F_GETFD) }, -1);
        assert_eq!(unsafe { libc::fcntl(writer, libc::F_GETFD) }, -1);
    }

    #[test]
    fn process_module_observes_real_unpatched_main_cells_without_writes() {
        let specs = import_specs().unwrap();
        let observed = super::super::observe_main_imports(&specs).unwrap();
        // The test executable is never patched or admitted by this test. Its
        // genuine observed cells are originals, or required cells are absent.
        assert!(verify_imports(MainImportRole::Native,&observed).is_err());
        // Choosing the declared Reference shape cannot turn genuine original
        // main cells into owned replacements or admit an unpatched module.
        assert!(verify_imports(MainImportRole::Reference,&observed).is_err());
        assert!(observed
            .iter()
            .all(|row| row.selected_address() != row.replacement));
        assert!(verify_imports(MainImportRole::Native,&[]).is_err());
    }
}
