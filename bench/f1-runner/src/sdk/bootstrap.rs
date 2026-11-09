//! Explicit fixture records. These functions never call a replaced original.
#[cfg(not(test))]
use super::observe_main_imports;
use super::{
    owned::{AliveLease, OwnedRoot, Result},
    SourceCallsite,
};
use objc2::{
    rc::Retained,
    runtime::{AnyObject, Bool, Sel},
    AnyThread, DefinedClass,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSEnumerator, NSError, NSNumber, NSObject, NSObjectProtocol, NSString,
    NSURL,
};
use std::{
    collections::BTreeSet,
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, OnceLock},
};
pub struct Account {
    pub uid: libc::uid_t,
    pub gid: libc::gid_t,
    pub name: CString,
    pub full_name: CString,
    pub home: CString,
    pub shell: CString,
}
impl Account {
    pub fn fixture(
        uid: libc::uid_t,
        gid: libc::gid_t,
        name: &str,
        full_name: &str,
        home: &Path,
    ) -> Result<Self> {
        let string = |v: &str| CString::new(v).map_err(|_| "NUL in account fixture".to_string());
        if name.is_empty() || name.contains('/') || !home.is_absolute() {
            return Err("invalid fixture account".into());
        }
        Ok(Self {
            uid,
            gid,
            name: string(name)?,
            full_name: string(full_name)?,
            home: string(home.to_str().ok_or("nonUTF8 home")?)?,
            shell: string("/owned/no-shell")?,
        })
    }
}
pub struct Bootstrap {
    pub home: OwnedRoot,
    pub cache: OwnedRoot,
    pub temp: OwnedRoot,
    pub applications: OwnedRoot,
    pub account: Account,
    pub life: Arc<dyn AliveLease>,
    descriptors: Option<Arc<super::descriptors::DescriptorTable>>,
    #[cfg(rcf1_reference_sdk)]
    reference_retired:std::sync::atomic::AtomicBool,
}
/// A per-operation input check for real native persistence. This is not a
/// blanket syscall permission: the endpoint must invoke it before EVERY
/// constructor/reload/relocation or watcher callback that resolves this path.
pub struct QualifiedProfile {
    paths: ruycast_native::profile::ProfilePaths,
    root: OwnedRoot,
}
impl QualifiedProfile {
    fn checked_paths(&self, context: &Bootstrap) -> Result<ruycast_native::profile::ProfilePaths> {
        context.before()?;
        context.owner(&self.paths.root)?.info(&self.paths.root)?;
        let selector = self
            .root
            .read_optional(&self.paths.root.join("config-location.json"), 1_000_000)?;
        let chosen = if let Some(value) = self.paths.config_override.as_deref() {
            context.expand(value)?
        } else if let Some(bytes) = selector {
            #[derive(serde::Deserialize)]
            struct Selection {
                directory: String,
            }
            let selected: Selection = serde_json::from_slice(&bytes)
                .map_err(|_| "invalid owned configuration selector")?;
            context.expand(&selected.directory)?
        } else {
            self.paths.root.clone()
        };
        let owner = context.owner(&chosen)?;
        if !owner.info(&chosen)?.0 {
            return Err("configuration folder must be an owned directory".into());
        }
        owner.read_optional(&chosen.join("config.json"), 12_000_000)?;
        Ok(self.paths.clone())
    }
    pub fn open(&self, context: &Bootstrap) -> Result<ruycast_native::profile::ConfigStore> {
        ruycast_native::profile::ConfigStore::open(self.checked_paths(context)?)
    }
    pub fn reload(
        &self,
        context: &Bootstrap,
        store: &mut ruycast_native::profile::ConfigStore,
    ) -> Result<bool> {
        self.checked_paths(context)?;
        store.reload()
    }
}
impl Bootstrap {
    pub fn new(
        home: OwnedRoot,
        cache: OwnedRoot,
        temp: OwnedRoot,
        applications: OwnedRoot,
        account: Account,
        life: Arc<dyn AliveLease>,
    ) -> Result<Self> {
        life.before_effect()?;
        if !life.is_alive() {
            return Err("bootstrap guardian not admitted".into());
        }
        if account.home.to_bytes() != home.path().as_os_str().as_encoded_bytes() {
            return Err("account/home mismatch".into());
        }
        for root in [&home, &cache, &temp, &applications] {
            if root.owner_ids()? != (account.uid, account.gid) {
                return Err("account fixture must match explicit owned descriptor identity".into());
            }
        }
        life.before_effect()?;
        Ok(Self {
            home,
            cache,
            temp,
            applications,
            account,
            life,
            descriptors: None,
            #[cfg(rcf1_reference_sdk)]
            reference_retired:std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn before(&self) -> Result<()> {
        #[cfg(rcf1_reference_sdk)]
        {
            self.reference_not_retired()?;
            let alive=self.life.is_alive();
            self.reference_not_retired()?;
            if !alive {return Err("guardian retired".into());}
            let result=self.life.before_effect();
            self.reference_not_retired()?;
            result
        }
        #[cfg(not(rcf1_reference_sdk))]
        {
            if !self.life.is_alive() {
                return Err("guardian retired".into());
            }
            self.life.before_effect()
        }
    }
    #[cfg(rcf1_reference_sdk)]
    fn reference_not_retired(&self)->Result<()> {
        if self.reference_retired.load(Ordering::Acquire) {return Err("Reference bootstrap retired".into());}
        Ok(())
    }
    pub fn qualify_profile(
        &self,
        root: &Path,
        config_override: Option<&str>,
    ) -> Result<QualifiedProfile> {
        self.before()?;
        if self.descriptors.is_some() {
            return Err("owned Host profiles must use the pinned Native C startup route".into());
        }
        self.owner(root)?.info(root)?;
        let profile = QualifiedProfile {
            paths: ruycast_native::profile::ProfilePaths::new(root.into(), self.home.path().into(), config_override.filter(|v| !v.is_empty()).map(str::to_owned)),
            root: OwnedRoot::open(root)?,
        };
        profile.checked_paths(self)?;
        Ok(profile)
    }
    pub fn owner(&self, path: &Path) -> Result<&OwnedRoot> {
        self.before()?;
        [&self.cache, &self.temp, &self.applications, &self.home]
            .into_iter()
            .find(|r| path.starts_with(r.path()))
            .ok_or_else(|| "unowned path rejected without query".into())
    }
    fn product_path(&self, path: &Path) -> Result<PathBuf> {
        if !path.is_absolute()
            || path
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err("unqualified product path".into());
        }
        for alias in [
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            self.home.path().join("Applications"),
        ] {
            if let Ok(tail) = path.strip_prefix(alias) {
                return Ok(self.applications.path().join(tail));
            }
        }
        self.owner(path)?;
        Ok(path.into())
    }
    pub fn expand(&self, text: &str) -> Result<PathBuf> {
        self.before()?;
        let value = if text == "~" {
            self.home.path().to_path_buf()
        } else if let Some(tail) = text.strip_prefix("~/") {
            self.home.path().join(tail)
        } else if let Some(named) = text.strip_prefix('~') {
            let (name, tail) = named.split_once('/').unwrap_or((named, ""));
            if name.as_bytes() != self.account.name.to_bytes() {
                return Err("unknown account; no lookup fallback".into());
            }
            self.home.path().join(tail)
        } else {
            PathBuf::from(text)
        };
        let root = self.owner(&value)?;
        if value.components().any(|v| {
            matches!(
                v,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        }) {
            return Err("parent component refused".into());
        }
        if !value.is_absolute() || !value.starts_with(root.path()) {
            return Err("unowned expanded path".into());
        }
        Ok(value)
    }
    fn directory(&self, kind: usize, domain: usize) -> Result<PathBuf> {
        self.before()?;
        match (kind, domain) {
            (13, 1) => Ok(self.cache.path().into()),
            (1, 1) => Ok(self.applications.path().into()),
            _ => Err("unowned search-path/domain".into()),
        }
    }
}
static CONTEXT: OnceLock<Bootstrap> = OnceLock::new();
static DENIED: AtomicU64 = AtomicU64::new(0);
#[cfg(rcf1_reference_sdk)]
struct ReferenceBootstrapPublication<'a> {
    context: &'a Bootstrap,
    workspace: Option<Arc<crate::workspace::Workspace>>,
    complete: bool,
}
#[cfg(rcf1_reference_sdk)]
impl Drop for ReferenceBootstrapPublication<'_>{
    fn drop(&mut self) {
        if !self.complete {
            self.context.reference_retired.store(true, Ordering::Release);
            if let Some(workspace) = &self.workspace {
                // A poisoned workspace already rejects every grant check.
                let _ = workspace.retire();
            }
        }
    }
}
pub fn install_context(context: Bootstrap) -> Result<()> {
    CONTEXT
        .set(context)
        .map_err(|_| "bootstrap context already installed".into())
}
pub fn rejected_calls() -> u64 {
    DENIED.load(Ordering::Relaxed)
}
/// Concrete pre-main module assembly. This observes the host's actual loader
/// cells itself; it does not accept receipt-supplied import addresses.
/// The Rust admission object must retain all required bootstrap imports, even
/// for a branch which does not normally call every account/home interface.
/// # Safety
/// Fresh owned process, guardian armed before entry, no original objects or
/// concurrent initializers/swizzlers. This is not permission to load a product.
#[cfg(not(test))]
pub unsafe fn initialize(context: Bootstrap) -> Result<super::ModuleAttestation> {
    context.before()?;
    let life = context.life.clone();
    let handles = [
        ("bootstrap.home", context.home.clone()),
        ("bootstrap.cache", context.cache.clone()),
        ("bootstrap.temp", context.temp.clone()),
        ("bootstrap.applications", context.applications.clone()),
    ]
    .into_iter()
    .map(|(namespace, root)| super::owned::OwnedHandleLease {
        namespace,
        root,
        life: life.clone(),
    })
    .collect();
    install_context(context)?;
    let imports = main_imports()?;
    let objc = unsafe { install_selectors() }?;
    let module = super::ModuleAttestation {
        module: "bootstrap",
        workspace_factories: None,
        scopes: scopes(),
        callsites: source_roster(),
        imports,
        objc,
        handles,
    };
    module.verify()?;
    Ok(module)
}
/// # Safety
/// Only the actual sealed full-Host constructor may call this after Pins,
/// guardian, main/SDK source and initializer qualification. No external Rust
/// context or plain C frame is an admission proof.
#[cfg(not(test))]
pub(crate) unsafe fn initialize_owned_host(
    mut context: Bootstrap,
    life: Arc<dyn AliveLease>,
    descriptors: Arc<super::descriptors::DescriptorTable>,
) -> Result<super::ModuleAttestation> {
    if !super::SOURCE_ROUTES_QUALIFIED
        || !Arc::ptr_eq(&context.life, &life)
        || context.descriptors.is_some()
    {
        return Err("owned Host context/guardian authority differs".into());
    }
    context.before()?;
    context.descriptors = Some(descriptors);
    unsafe { initialize(context) }
}
/// Constructor-free Reference setup uses the same retained roots/issuer, but
/// patches the actual main cells transactionally rather than relying on dyld.
/// # Safety
/// Only sealed ReferenceAdmission supplies prewrite after qualifying the fixed
/// source, actual images, private guardian and caller ABI. No original object
/// or concurrent setup may exist; all failures revoke this installed context.
/// Workspace preparation is limited to the owned model and constructor-free
/// data factory registration. It must not invoke any unpatched selector.
#[cfg(all(not(test),rcf1_reference_sdk))]
pub(crate) unsafe fn initialize_owned_reference(
    mut context:Bootstrap,
    life:Arc<dyn AliveLease>,
    descriptors:Arc<super::descriptors::DescriptorTable>,
    prewrite:impl Fn()->Result<()>+Send+Sync,
    prepare_workspace: impl FnOnce() -> Result<Arc<crate::workspace::Workspace>>,
    install_profile_lock: impl FnOnce(&dyn AliveLease) -> Result<Vec<super::ImportReplacement>>,
) -> Result<(Arc<crate::workspace::Workspace>, super::ModuleAttestation)> {
    if !Arc::ptr_eq(&context.life,&life)
        || context.descriptors.is_some() || CONTEXT.get().is_some() {
        return Err("Reference bootstrap context/issuer differs or was published".into());
    }
    context.before()?;
    prewrite()?;
    context.reference_not_retired()?;
    context.descriptors=Some(descriptors);
    install_context(context)?;
    let context=CONTEXT.get().ok_or("Reference bootstrap publication absent")?;
    let mut publication = ReferenceBootstrapPublication { context, workspace: None, complete: false };
    struct Guard<'a,F>{context:&'a Bootstrap,prewrite:&'a F}
    impl<F:Fn()->Result<()>+Send+Sync> AliveLease for Guard<'_,F>{
        fn is_alive(&self)->bool{self.before_effect().is_ok()}
        fn before_effect(&self)->Result<()> {
            self.context.before()?;
            (self.prewrite)()?;
            self.context.reference_not_retired()
        }
    }
    let guard=Guard{context,prewrite:&prewrite};
    let specs=INTERPOSE.iter().zip(required_imports()).map(|(entry,symbol)|super::ImportSpec {
        symbol,original:entry.original as usize,replacement:entry.replacement as usize,
    }).collect::<Vec<_>>();
    let role=reference_required_imports();
    let selected=specs.iter().filter(|spec|role.contains(&spec.symbol)).copied().collect::<Vec<_>>();
    let mut imports=super::import_patch::install_exact_presence(&selected,&specs,&guard)?;
    // The Reference data factory changes NSString lifecycle metadata. Finish
    // that constructor-free transaction before capturing selector provenance
    // for the complete loaded NSString family.
    let workspace = prepare_workspace()?;
    publication.workspace = Some(workspace.clone());
    guard.before_effect()?;
    imports.extend(install_profile_lock(&guard)?);
    let objc=unsafe{install_selectors_checked(&||guard.before_effect())}?;
    let handles=[("bootstrap.home",&context.home),("bootstrap.cache",&context.cache),
        ("bootstrap.temp",&context.temp),("bootstrap.applications",&context.applications)]
        .into_iter().map(|(namespace,root)|super::owned::OwnedHandleLease{namespace,root:root.clone(),life:life.clone()}).collect();
    let module=super::ModuleAttestation{module:"bootstrap",workspace_factories:None,scopes:scopes(),callsites:source_roster(),imports,objc,handles};
    module.verify()?;
    guard.before_effect()?;
    publication.complete=true;
    Ok((workspace, module))
}
/// Borrow the existing full-Host capability issuer; never allocate a second
/// DescriptorTable or discover another root through the environment.
pub(crate) fn owned_module_context(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
) -> Result<super::owned::ModuleContext> {
    let context = CONTEXT.get().ok_or("owned Host bootstrap absent")?;
    if !Arc::ptr_eq(&context.life, life)
        || context.descriptors.as_ref().is_none_or(|actual| !Arc::ptr_eq(actual, descriptors)) {
        return Err("owned module guardian/descriptor issuer differs".into());
    }
    context.before()?;
    Ok(super::owned::ModuleContext {
        home: context.home.clone(), cache: context.cache.clone(),
        temp: context.temp.clone(), applications: context.applications.clone(),
        life: context.life.clone(),
    })
}
/// Install only the private Security leaves from explicit owned fixture bytes.
/// The returned exact Context Arc is retained for its existing retire API.
/// # Safety
/// The sealed Host coordinator has independently qualified this caller, the
/// five actual imports and their ABI/source/images. prewrite is a pure final
/// validator; neither this leaf module nor its empty scopes admit providers,
/// network, Safari discovery/send or complete Source14.
pub unsafe fn initialize_private_security(
    life: Arc<dyn AliveLease>,
    descriptors: Arc<super::descriptors::DescriptorTable>,
    history: Option<Vec<u8>>,
    entropy: Vec<u8>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<(super::ModuleAttestation, Arc<super::private_network::Context>)> {
    let context = owned_module_context(&life, &descriptors)?;
    let lease = context.lease("private-security")?;
    prewrite()?;
    let security = super::private_network::Context::new(life, history, entropy)?;
    let module = unsafe { super::private_network::initialize_owned(security.clone(), lease, prewrite) }?;
    Ok((module, security))
}
/// Install the existing Process/monitor owner through the same full-Host
/// guardian and DescriptorTable. No rule is discovered or manufactured here.
/// # Safety
/// The sealed source-qualified coordinator has filled registry only with exact
/// pinned own Rust-fake executable/argument rules and retained all code/lease
/// proofs. prewrite is its final pure source/import validator. The router checks
/// this same Bootstrap issuer and the actual unique combined import roster.
pub unsafe fn initialize_process_monitor(
    life: Arc<dyn AliveLease>,
    descriptors: Arc<super::descriptors::DescriptorTable>,
    registry: Arc<super::process_dispatch::Registry>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<super::ModuleAttestation> {
    unsafe { super::process_router::initialize_owned(life, descriptors, registry, prewrite) }
}
/// Install the Reference role's Process/monitor imports from its sealed issuer.
/// This does not substitute the Native role's main image or import cells.
/// # Safety
/// The caller retains the actual Reference admission and exact pinned process
/// rules. The router derives its guardian/descriptors from that admission and
/// checks the Reference import/absence roster before and after its transaction.
#[cfg(all(not(test), rcf1_reference_sdk))]
pub unsafe fn initialize_reference_process_monitor(
    admission: Arc<super::core_preentry::ReferenceAdmission>,
    registry: Arc<super::process_dispatch::Registry>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<super::ModuleAttestation> {
    admission.before_reference_initialization()?;
    unsafe { super::process_router::initialize_reference(admission, registry, prewrite) }
}
pub(crate) fn publish_profile_provider(
    life: &Arc<dyn AliveLease>,
    descriptors: &Arc<super::descriptors::DescriptorTable>,
    profile: &Path,
    config_override: Option<&str>,
) -> Result<(*const ruycast_native::profile_io_abi::Api, PathBuf)> {
    if !super::SOURCE_ROUTES_QUALIFIED {
        return Err("owned Host source routes are not qualified".into());
    }
    let context = CONTEXT.get().ok_or("owned Host bootstrap absent")?;
    if !Arc::ptr_eq(&context.life, life)
        || context
            .descriptors
            .as_ref()
            .is_none_or(|d| !Arc::ptr_eq(d, descriptors))
    {
        return Err("owned Host descriptor/guardian issuer differs".into());
    }
    context.before()?;
    if !profile.is_absolute()
        || profile.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || profile
            .to_str()
            .is_none_or(|s| s.is_empty() || s.len() > 8192 || s.contains('\0'))
        || context
            .home
            .path()
            .to_str()
            .is_none_or(|s| s.len() > 8192 || s.contains('\0'))
    {
        return Err("invalid owned Host profile/home path".into());
    }
    context.owner(profile)?;
    if let Some(value) = config_override {
        if value.len() > 8192 || value.contains('\0') {
            return Err("invalid owned Host override".into());
        }
        if !value.is_empty() {
            context.expand(value)?;
        }
    }
    let roots = [
        context.home.clone(),
        context.cache.clone(),
        context.temp.clone(),
        context.applications.clone(),
    ]
    .into_iter()
    .collect();
    let broker = super::profile_io::Broker::new(roots, life.clone(), descriptors.clone())?;
    super::profile_io::publish(broker)?;
    let api = super::profile_io::rcf1_profile_io_v1();
    if api.is_null() {
        super::profile_io::retire();
        return Err("owned profile provider refused".into());
    }
    Ok((api, context.home.path().into()))
}
fn with<T>(f: impl FnOnce(&Bootstrap) -> Result<T>) -> Option<T> {
    let result = CONTEXT
        .get()
        .ok_or_else(|| "bootstrap not configured".into())
        .and_then(|s| {
            s.before()?;
            f(s)
        });
    match result {
        Ok(v) => Some(v),
        Err(_) => {
            DENIED.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}
fn string(text: &str) -> *mut AnyObject {
    Retained::autorelease_return(NSString::from_str(text)).cast()
}
fn url(path: &Path) -> *mut AnyObject {
    path.to_str().map_or(ptr::null_mut(), |p| {
        Retained::autorelease_return(NSURL::fileURLWithPath(&NSString::from_str(p))).cast()
    })
}
unsafe fn text(pointer: *mut AnyObject) -> Option<String> {
    unsafe { pointer.cast::<NSString>().as_ref() }.map(|s| s.to_string())
}
unsafe fn path(pointer: *mut AnyObject, is_url: bool) -> Option<PathBuf> {
    let p = if is_url {
        let u = unsafe { pointer.cast::<NSURL>().as_ref() }?;
        if !u.isFileURL() {
            return None;
        }
        u.path().map(|p| PathBuf::from(p.to_string()))
    } else {
        unsafe { text(pointer) }.map(PathBuf::from)
    }?;
    with(|s| s.product_path(&p))
}
unsafe fn fail(error: *mut *mut NSError) {
    if !error.is_null() {
        let value = NSError::errorWithDomain_code_userInfo(
            &NSString::from_str("RuycastOwnedSDK"),
            libc::EPERM as isize,
            None,
        );
        unsafe { *error = Retained::autorelease_return(value) };
    }
}
unsafe fn fail_at(error: *mut *mut NSError, _path: &Path) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if !error.is_null() {
        let owned_error = super::private_network::qualify_assembly_cache_error(_path).and_then(|qualified| {
            if qualified {
                crate::workspace::objc::reference_file_fixture_error().map(Some)
            } else {
                Ok(None)
            }
        });
        match owned_error {
            Ok(Some(pointer)) => {
                let Some(value) = (unsafe { Retained::from_raw(pointer.cast::<NSError>()) }) else {
                    unsafe { libc::_exit(78) }
                };
                unsafe { *error = Retained::autorelease_return(value) };
                return;
            }
            Ok(None) => {}
            Err(_) => {
                super::private_network::report_reference_website_dispatch_refusal(
                    "Reference website cache error construction refused",
                );
                unsafe { libc::_exit(78) };
            }
        }
    }
    unsafe { fail(error) };
}
/// Exact main-image import specification; observing/installing the actual cell
/// and proving the complete source constructor/ARC roster remain mandatory.
/// This is not a dyld interpose entry or permission to call an original allocator.
pub(crate) fn source_allocation_import_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"objc_alloc",
        original: objc2::ffi::objc_alloc as *const () as usize,
        replacement: rcf1_objc_alloc as *const () as usize,
    }
}
/// Actual objc2 0.6.5 fast-path ABI is Class -> id, with no zone argument.
/// It must be observed/installed as its own exact main-image import cell.
pub(crate) fn source_allocation_with_zone_import_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol:c"objc_allocWithZone",
        original:objc2::ffi::objc_allocWithZone as *const () as usize,
        replacement:rcf1_objc_alloc_with_zone as *const () as usize,
    }
}
/// # Safety
/// Same source/class/ARC/+1/initializer qualification as rcf1_objc_alloc is
/// mandatory. This distinct import entry shares that exact guarded allocator;
/// it never calls an original allocator or records a second allocation.
#[no_mangle]
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
pub unsafe extern "C" fn rcf1_objc_alloc_with_zone(class:*const objc2::runtime::AnyClass)->*mut AnyObject {
    unsafe {rcf1_objc_alloc(class)}
}

#[no_mangle]
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
pub unsafe extern "C" fn rcf1_objc_alloc_with_zone(
    _class: *const objc2::runtime::AnyClass,
) -> *mut AnyObject {
    core::arch::naked_asm!("mov x1, x30", "mov x2, x29", "mov w3, w25", "mov x4, x19", "mov x5, x24",
        "b {checked}", checked = sym allocate_source);
}
/// Allocate through an already published exact class grant only. The same +1
/// is recorded by grant.allocate before the real source caller sets its ivars;
/// no second allocation observer or permissive original fallback is used.
/// # Safety
/// Installed only by the independently qualified main-image import transaction.
/// Actual source classes still require full source/CXX/ARC/initializer proofs,
/// and their selected super-init must consume the same allocation's token.
#[no_mangle]
pub unsafe extern "C" fn rcf1_objc_alloc(
    class: *const objc2::runtime::AnyClass,
) -> *mut AnyObject {
    unsafe { allocate_source(class, 0, 0, -1, 0, 0) }
}

unsafe extern "C" fn allocate_source(
    class: *const objc2::runtime::AnyClass,
    _zone_caller: usize,
    _zone_frame: usize,
    _zone_fd: i32,
    _zone_swift_self: usize,
    _zone_model_x24: usize,
) -> *mut AnyObject {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let edit_singleton = crate::workspace::factory::is_reference_edit_alloc_caller(_zone_caller);
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let lock_route = unsafe {
        super::core_preentry::ReferenceAdmission::reference_lock_route(_zone_caller, _zone_frame)
    };
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let configuration_read = super::core_preentry::ReferenceAdmission::is_configuration_read_allocator(_zone_caller);
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let search_field = super::core_preentry::ReferenceAdmission::is_search_field_allocator(_zone_caller);
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let palette_host = super::core_preentry::ReferenceAdmission::is_palette_host_allocator(_zone_caller);
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let palette_panel = super::core_preentry::ReferenceAdmission::is_palette_panel_allocator(_zone_caller);
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let snippet_panel = super::core_preentry::ReferenceAdmission::is_snippet_panel_allocator(_zone_caller);
    if class.is_null() {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if edit_singleton || lock_route.is_some() || configuration_read || search_field || palette_host || palette_panel || snippet_panel {
            unsafe { libc::_exit(95) }
        }
        return ptr::null_mut();
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // No raw class reference may be formed before actual runtime membership.
        let known = super::metadata::registered_class(class as usize)?;
        let class = unsafe { &*(known.address() as *const objc2::runtime::AnyClass) };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            if snippet_panel != (class.name() == c"RuycastF1Reference.SnippetChoicePanel") {
                return Err("Reference SnippetChoicePanel allocator class/caller mismatch".into());
            }
            if snippet_panel {
                let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                let model = admission.verify_reference_snippet_panel_allocator(
                    _zone_caller, known.address(), _zone_swift_self, _zone_model_x24,
                )?;
                if stopped.load(Ordering::Acquire) {
                    return Err("Reference SnippetChoicePanel allocation ingress closed".into());
                }
                let pointer = unsafe { crate::workspace::factory::allocate(class) }?;
                let grant = crate::workspace::factory::source_allocation_grant(pointer)?;
                if let Err(error) = grant.bind_snippet_panel_model(pointer, model) {
                    unsafe { grant.cleanup_source_partial(pointer) }?;
                    return Err(error);
                }
                if stopped.load(Ordering::Acquire) {
                    unsafe { grant.cleanup_source_partial(pointer) }?;
                    return Err("Reference SnippetChoicePanel stopped before allocation handoff".into());
                }
                return Ok(pointer);
            }
            if palette_panel != (class.name() == c"RuycastF1Reference.PalettePanel") {
                return Err("Reference PalettePanel allocator class/caller mismatch".into());
            }
            if palette_panel {
                if unsafe { libc::pthread_main_np() } != 1 || super::process_fork::is_child() {
                    return Err("Reference PalettePanel allocation off its owner".into());
                }
                let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                admission.verify_reference_palette_panel_source()?;
                if stopped.load(Ordering::Acquire) {
                    return Err("Reference PalettePanel allocation ingress closed".into());
                }
                let pointer = unsafe { crate::workspace::factory::allocate(class) }?;
                let grant = crate::workspace::factory::source_allocation_grant(pointer)?;
                if !grant.is_reference_palette_construction() || grant.class() != class {
                    return Err("Reference PalettePanel allocation owner changed".into());
                }
                if stopped.load(Ordering::Acquire) {
                    unsafe { grant.cleanup_source_partial(pointer) }?;
                    return Err("Reference PalettePanel allocation stopped before handoff".into());
                }
                return Ok(pointer);
            }
            if palette_host {
                if unsafe { libc::pthread_main_np() } != 1 {
                    return Err("Reference palette host allocation off main thread".into());
                }
                let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                let snippet = super::core_preentry::ReferenceAdmission::is_snippet_host_allocator(_zone_caller);
                let panel = admission.verify_palette_host_panel(_zone_caller, _zone_swift_self, class, false, _zone_model_x24)?;
                if stopped.load(Ordering::Acquire) {
                    return Err("Reference palette host allocation ingress closed".into());
                }
                return unsafe { crate::workspace::objc::allocate_reference_palette_host(
                    panel, class, _zone_swift_self, snippet, stopped, || {
                        if admission.verify_palette_host_panel(_zone_caller, _zone_swift_self, class, false, _zone_model_x24)? != panel {
                            return Err("Reference palette host allocation owner changed".into());
                        }
                        Ok(())
                    },
                ) };
            }
            if search_field {
                if unsafe { libc::pthread_main_np() } != 1 {
                    return Err("Reference search allocation off main thread".into());
                }
                let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                let initializer = admission.verify_search_field_allocator(_zone_caller, known.address())?;
                if stopped.load(Ordering::Acquire) {
                    return Err("Reference search allocation ingress closed".into());
                }
                let object = unsafe { objc2::rc::Retained::from_raw(objc2::ffi::objc_allocWithZone(class)) }
                    .ok_or("Reference search allocation absent")?;
                if !std::ptr::eq(object.class(), class)
                    || !admission.verify_search_field_allocator(_zone_caller, known.address())
                        .is_ok_and(|selected| selected == initializer)
                    || stopped.load(Ordering::Acquire)
                {
                    std::mem::forget(object);
                    return Err("Reference search allocation class or owner changed".into());
                }
                return Ok(objc2::rc::Retained::into_raw(object));
            }
            if configuration_read {
                if unsafe { libc::pthread_main_np() } == 1 {
                    return Err("Reference configuration file handle on main thread".into());
                }
                let (admission, _) = super::reference_runtime::worker_dispatch_admission()?;
                let (concrete, initializer) = admission.verify_configuration_read_allocator(_zone_caller, known.address(), _zone_fd, None)?;
                let object = unsafe { objc2::rc::Retained::from_raw(objc2::ffi::objc_allocWithZone(class)) }
                    .ok_or("Reference configuration file handle allocation absent")?;
                if object.class() as *const _ as usize != concrete
                    || !admission.verify_configuration_read_allocator(
                        _zone_caller, known.address(), _zone_fd, Some(&initializer),
                    ).is_ok_and(|(selected, _)| selected == concrete)
                {
                    std::mem::forget(object);
                    return Err("Reference configuration file handle class or descriptor changed".into());
                }
                return Ok(objc2::rc::Retained::into_raw(object));
            }
            if let Some(route) = lock_route {
                use super::core_preentry::ReferenceLockRoute;
                if (unsafe { libc::pthread_main_np() } == 1) != (route == ReferenceLockRoute::Main) {
                    return Err("Reference lock allocator on wrong thread".into());
                }
                let (admission, stopped) = if let Some(probe) =
                    super::system_providers::take_monitor_construction_admission(_zone_caller)
                {
                    probe.before_reference_initialization()?;
                    (probe, None)
                } else {
                    let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                    (admission, Some(stopped))
                };
                unsafe { admission.verify_reference_lock_caller(_zone_caller, _zone_frame, known.address(), route) }?;
                // The already admitted client and Safari search worker still
                // need their local lock to reach the cancellation path.
                if route == ReferenceLockRoute::Main && stopped.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                    return Err("Reference SystemServices lock ingress closed".into());
                }
                let object = unsafe { objc2::rc::Retained::from_raw(objc2::ffi::objc_allocWithZone(class)) }
                    .ok_or("Reference SystemServices lock allocation absent")?;
                if !std::ptr::eq(object.class(), class)
                    || unsafe { admission.verify_reference_lock_caller(_zone_caller, _zone_frame, known.address(), route) }.is_err()
                    || (route == ReferenceLockRoute::Main && stopped.is_some_and(|flag| flag.load(Ordering::Acquire)))
                {
                    std::mem::forget(object);
                    return Err("Reference SystemServices lock class or owner changed".into());
                }
                return Ok(objc2::rc::Retained::into_raw(object));
            }
            if let Some(receiver) = unsafe {
                super::private_network::allocate_reference_website_object(class, _zone_caller, _zone_frame)
            }? {
                return Ok(receiver);
            }
            let edit_class = class.name() == c"RuycastF1Reference.NativeEditCommands";
            if edit_singleton != edit_class {
                return Err("Reference Edit allocator class/caller mismatch".into());
            }
            if edit_singleton {
                crate::workspace::factory::verify_reference_edit_alloc_caller(_zone_caller)?;
            }
        }
        // allocate itself verifies the original model/grant and records the +1.
        unsafe { crate::workspace::factory::allocate(class) }
    }));
    match outcome {
        Ok(Ok(receiver)) => receiver,
        failure => {
            if let Err(payload) = failure {
                std::mem::forget(payload);
            }
            DENIED.fetch_add(1, Ordering::Relaxed);
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if edit_singleton || lock_route.is_some() || configuration_read || search_field || palette_host || palette_panel || snippet_panel {
                // The pinned Swift caller immediately writes the window ivar
                // or expects an initialized non-optional lock.
                unsafe { libc::_exit(95) }
            }
            ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "C" fn rcf1_NSHomeDirectory() -> *mut AnyObject {
    with(|s| Ok(string(s.home.path().to_str().ok_or("home encoding")?))).unwrap_or(ptr::null_mut())
}
#[no_mangle]
pub extern "C" fn rcf1_NSTemporaryDirectory() -> *mut AnyObject {
    with(|s| Ok(string(&format!("{}/", s.temp.path().display())))).unwrap_or(ptr::null_mut())
}
#[no_mangle]
pub extern "C" fn rcf1_NSUserName() -> *mut AnyObject {
    with(|s| {
        Ok(string(
            s.account.name.to_str().map_err(|_| "name encoding")?,
        ))
    })
    .unwrap_or(ptr::null_mut())
}
#[no_mangle]
pub extern "C" fn rcf1_NSFullUserName() -> *mut AnyObject {
    with(|s| {
        Ok(string(
            s.account.full_name.to_str().map_err(|_| "name encoding")?,
        ))
    })
    .unwrap_or(ptr::null_mut())
}
#[no_mangle]
pub extern "C" fn rcf1_getuid() -> libc::uid_t {
    with(|s| Ok(s.account.uid)).unwrap_or(libc::uid_t::MAX)
}
#[no_mangle]
pub extern "C" fn rcf1_geteuid() -> libc::uid_t {
    rcf1_getuid()
}
#[no_mangle]
pub extern "C" fn rcf1_getgid() -> libc::gid_t {
    with(|s| Ok(s.account.gid)).unwrap_or(libc::gid_t::MAX)
}
#[no_mangle]
pub extern "C" fn rcf1_getegid() -> libc::gid_t {
    rcf1_getgid()
}
/// # Safety
/// `name` is nil or a valid NSString, as in Foundation's original C ABI.
#[no_mangle]
pub unsafe extern "C" fn rcf1_NSHomeDirectoryForUser(name: *mut AnyObject) -> *mut AnyObject {
    let name = unsafe { text(name) };
    with(|s| {
        if name
            .as_deref()
            .is_none_or(|n| n.as_bytes() != s.account.name.to_bytes())
        {
            return Err("unknown account".into());
        }
        Ok(string(s.home.path().to_str().ok_or("home encoding")?))
    })
    .unwrap_or(ptr::null_mut())
}
#[no_mangle]
pub extern "C" fn rcf1_NSSearchPathForDirectoriesInDomains(
    kind: usize,
    domain: usize,
    _expand: Bool,
) -> *mut AnyObject {
    with(|s| {
        let p = s.directory(kind, domain)?;
        Ok(
            Retained::autorelease_return(NSArray::from_retained_slice(&[NSString::from_str(
                p.to_str().ok_or("path encoding")?,
            )]))
            .cast(),
        )
    })
    .unwrap_or(ptr::null_mut())
}
fn copy_account(
    account: &Account,
    pw: *mut libc::passwd,
    buffer: *mut libc::c_char,
    len: usize,
    result: *mut *mut libc::passwd,
) -> i32 {
    if pw.is_null() || buffer.is_null() || result.is_null() {
        return libc::EINVAL;
    }
    unsafe { *result = ptr::null_mut() };
    let fields = [
        account.name.as_bytes_with_nul(),
        c"*".to_bytes_with_nul(),
        account.full_name.as_bytes_with_nul(),
        account.home.as_bytes_with_nul(),
        account.shell.as_bytes_with_nul(),
        c"".to_bytes_with_nul(),
    ];
    let needed: usize = fields.iter().map(|s| s.len()).sum();
    if needed > len {
        return libc::ERANGE;
    }
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut offset = 0;
    let mut pointers = vec![];
    for field in fields {
        let p = unsafe { buffer.add(offset) };
        unsafe { ptr::copy_nonoverlapping(field.as_ptr().cast(), p, field.len()) };
        pointers.push(p);
        offset += field.len();
    }
    record.pw_name = pointers[0];
    record.pw_passwd = pointers[1];
    record.pw_gecos = pointers[2];
    record.pw_dir = pointers[3];
    record.pw_shell = pointers[4];
    record.pw_class = pointers[5];
    record.pw_uid = account.uid;
    record.pw_gid = account.gid;
    unsafe {
        *pw = record;
        *result = pw
    };
    0
}
/// # Safety
/// Output pointers and buffer follow the public getpwuid_r ABI.
#[no_mangle]
pub unsafe extern "C" fn rcf1_getpwuid_r(
    uid: libc::uid_t,
    pw: *mut libc::passwd,
    buffer: *mut libc::c_char,
    len: usize,
    result: *mut *mut libc::passwd,
) -> i32 {
    if !result.is_null() {
        unsafe { *result = ptr::null_mut() };
    }
    with(|s| {
        if uid != s.account.uid {
            return Err("unknown uid".into());
        }
        Ok(copy_account(&s.account, pw, buffer, len, result))
    })
    .unwrap_or(libc::ENOENT)
}
/// # Safety
/// Inputs/outputs follow the public getpwnam_r ABI; name is NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn rcf1_getpwnam_r(
    name: *const libc::c_char,
    pw: *mut libc::passwd,
    buffer: *mut libc::c_char,
    len: usize,
    result: *mut *mut libc::passwd,
) -> i32 {
    if !result.is_null() {
        unsafe { *result = ptr::null_mut() };
    }
    if name.is_null() {
        return libc::EINVAL;
    }
    let name = unsafe { CStr::from_ptr(name) };
    with(|s| {
        if name != s.account.name.as_c_str() {
            return Err("unknown name".into());
        }
        Ok(copy_account(&s.account, pw, buffer, len, result))
    })
    .unwrap_or(libc::ENOENT)
}
thread_local! {static PASSWD:std::cell::RefCell<(libc::passwd,[libc::c_char;4096])>=const {std::cell::RefCell::new((unsafe{std::mem::zeroed()},[0;4096]))};}
#[no_mangle]
pub extern "C" fn rcf1_getpwuid(uid: libc::uid_t) -> *mut libc::passwd {
    PASSWD.with(|v| {
        let mut v = v.borrow_mut();
        let (pw, buffer) = &mut *v;
        let mut result = ptr::null_mut();
        let code =
            unsafe { rcf1_getpwuid_r(uid, pw, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if code != 0 {
            unsafe { *libc::__error() = code };
        }
        result
    })
}
/// # Safety
/// Name is a NUL-terminated account-name string per getpwnam.
#[no_mangle]
pub unsafe extern "C" fn rcf1_getpwnam(name: *const libc::c_char) -> *mut libc::passwd {
    PASSWD.with(|v| {
        let mut v = v.borrow_mut();
        let (pw, buffer) = &mut *v;
        let mut result = ptr::null_mut();
        let code =
            unsafe { rcf1_getpwnam_r(name, pw, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if code != 0 {
            unsafe { *libc::__error() = code };
        }
        result
    })
}
unsafe extern "C" fn home(_this: *mut AnyObject, _sel: Sel) -> *mut AnyObject {
    with(|s| Ok(url(s.home.path()))).unwrap_or(ptr::null_mut())
}
unsafe extern "C" fn temp(_this: *mut AnyObject, _sel: Sel) -> *mut AnyObject {
    with(|s| Ok(url(s.temp.path()))).unwrap_or(ptr::null_mut())
}
unsafe extern "C" fn user_home(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
) -> *mut AnyObject {
    let name = unsafe { text(name) };
    with(|s| {
        if name
            .as_deref()
            .is_none_or(|n| n.as_bytes() != s.account.name.to_bytes())
        {
            return Err("unknown account".into());
        }
        Ok(url(s.home.path()))
    })
    .unwrap_or(ptr::null_mut())
}
unsafe extern "C" fn directories(
    _this: *mut AnyObject,
    _sel: Sel,
    kind: usize,
    domain: usize,
) -> *mut AnyObject {
    with(|s| {
        let p = s.directory(kind, domain)?;
        let value = NSURL::fileURLWithPath(&NSString::from_str(p.to_str().ok_or("path encoding")?));
        Ok(Retained::autorelease_return(NSArray::from_retained_slice(&[value])).cast())
    })
    .unwrap_or(ptr::null_mut())
}
unsafe extern "C" fn expand(this: *mut AnyObject, _sel: Sel) -> *mut AnyObject {
    let Some(value) = (unsafe { text(this) }) else {
        return ptr::null_mut();
    };
    with(|s| {
        let p = s.expand(&value)?;
        Ok(string(p.to_str().ok_or("path encoding")?))
    })
    .unwrap_or_else(|| {
        string(
            if !value.starts_with('/')
                || Path::new(&value)
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir))
            {
                &value
            } else {
                "__ruycast_unowned_path_rejected__"
            },
        )
    })
}
unsafe extern "C" fn exists(_this: *mut AnyObject, _sel: Sel, name: *mut AnyObject) -> Bool {
    let Some(path) = (unsafe { path(name, false) }) else {
        return Bool::NO;
    };
    Bool::new(with(|s| s.owner(&path)?.info(&path)).is_some())
}
unsafe extern "C" fn exists_directory(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    is_dir: *mut Bool,
) -> Bool {
    if !is_dir.is_null() {
        unsafe { *is_dir = Bool::NO };
    }
    let Some(path) = (unsafe { path(name, false) }) else {
        return Bool::NO;
    };
    let result = with(|s| s.owner(&path)?.info(&path));
    if let Some((directory, _)) = result {
        if !is_dir.is_null() {
            unsafe { *is_dir = Bool::new(directory) };
        }
        Bool::YES
    } else {
        Bool::NO
    }
}
unsafe fn list(
    name: *mut AnyObject,
    is_url: bool,
    options: usize,
    error: *mut *mut NSError,
) -> *mut AnyObject {
    let Some(p) = (unsafe { path(name, is_url) }) else {
        unsafe { fail(error) };
        return ptr::null_mut();
    };
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if is_url {
        match super::private_network::assembly_cache_listing(&p, options) {
            Ok(Some(urls)) => return Retained::autorelease_return(urls).cast(),
            Ok(None) => {}
            Err(_) => {
                super::private_network::report_reference_website_dispatch_refusal(
                    "Reference website cache listing refused",
                );
                unsafe { libc::_exit(78) };
            }
        }
    }
    let result = with(|s| s.owner(&p)?.list(&p));
    let Some(entries) = result else {
        unsafe { fail(error) };
        return ptr::null_mut();
    };
    if is_url {
        let values: Vec<_> = entries
            .into_iter()
            .filter(|e| options & 4 == 0 || !e.name.starts_with('.'))
            .map(|e| {
                NSURL::fileURLWithPath(&NSString::from_str(
                    p.join(e.name).to_str().expect("UTF8 fixture"),
                ))
            })
            .collect();
        Retained::autorelease_return(NSArray::from_retained_slice(&values)).cast()
    } else {
        let values: Vec<_> = entries
            .into_iter()
            .map(|e| NSString::from_str(&e.name))
            .collect();
        Retained::autorelease_return(NSArray::from_retained_slice(&values)).cast()
    }
}
unsafe extern "C" fn contents_path(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    error: *mut *mut NSError,
) -> *mut AnyObject {
    unsafe { list(name, false, 0, error) }
}
unsafe extern "C" fn contents_url(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    _keys: *mut AnyObject,
    options: usize,
    error: *mut *mut NSError,
) -> *mut AnyObject {
    unsafe { list(name, true, options, error) }
}
unsafe fn mkdir(
    name: *mut AnyObject,
    is_url: bool,
    recursive: Bool,
    error: *mut *mut NSError,
) -> Bool {
    let result = unsafe { path(name, is_url) }
        .and_then(|p| with(|s| s.owner(&p)?.mkdir(&p, recursive.as_bool())));
    if result.is_some() {
        Bool::YES
    } else {
        unsafe { fail(error) };
        Bool::NO
    }
}
unsafe extern "C" fn mkdir_path(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    recursive: Bool,
    _attributes: *mut AnyObject,
    error: *mut *mut NSError,
) -> Bool {
    unsafe { mkdir(name, false, recursive, error) }
}
unsafe extern "C" fn mkdir_url(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    recursive: Bool,
    _attributes: *mut AnyObject,
    error: *mut *mut NSError,
) -> Bool {
    unsafe { mkdir(name, true, recursive, error) }
}
unsafe fn remove(name: *mut AnyObject, is_url: bool, error: *mut *mut NSError) -> Bool {
    let Some(path) = (unsafe { path(name, is_url) }) else {
        unsafe { fail(error) };
        return Bool::NO;
    };
    if with(|s| s.owner(&path)?.remove(&path)).is_some() {
        Bool::YES
    } else {
        unsafe { fail_at(error, &path) };
        Bool::NO
    }
}
unsafe extern "C" fn remove_path(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    error: *mut *mut NSError,
) -> Bool {
    unsafe { remove(name, false, error) }
}
unsafe extern "C" fn remove_url(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    error: *mut *mut NSError,
) -> Bool {
    unsafe { remove(name, true, error) }
}
struct Enumeration {
    queue: std::collections::VecDeque<(PathBuf, usize)>,
    current_level: usize,
    strings: bool,
    base: PathBuf,
}
objc2::define_class!(
 #[unsafe(super=NSEnumerator)] #[thread_kind=AnyThread]
 #[ivars=std::sync::Mutex<Enumeration>]
 struct OwnedEnumerator;
 unsafe impl NSObjectProtocol for OwnedEnumerator {}
 impl OwnedEnumerator{
  #[unsafe(method(nextObject))]
  fn next(&self)->*mut AnyObject{let get=||->Option<Retained<NSObject>>{let mut state=self.ivars().lock().ok()?;let(path,level)=state.queue.pop_front()?;if with(|s|s.owner(&path)?.info(&path)).is_none(){state.queue.clear();return None}state.current_level=level;if state.strings{Some(NSString::from_str(path.strip_prefix(&state.base).ok()?.to_str()?).into_super())}else{Some(NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?)).into_super())}};get().map_or(ptr::null_mut(),|v|Retained::autorelease_return(v).cast())}
  #[unsafe(method(level))]fn level(&self)->usize{self.ivars().lock().map_or(0,|s|s.current_level)}
  #[unsafe(method(skipDescendants))]fn skip(&self){if let Ok(mut s)=self.ivars().lock(){while s.queue.front().is_some_and(|(_,level)|*level>s.current_level){s.queue.pop_front();}}}
  #[unsafe(method(skipDescendents))]fn old_skip(&self){if let Ok(mut s)=self.ivars().lock(){while s.queue.front().is_some_and(|(_,level)|*level>s.current_level){s.queue.pop_front();}}}
 }
);
fn tree(
    s: &Bootstrap,
    p: &Path,
    options: usize,
    depth: usize,
    output: &mut std::collections::VecDeque<(PathBuf, usize)>,
) -> Result<()> {
    if depth > 128 || output.len() > 100_000 {
        return Err("owned enumeration bound exceeded".into());
    }
    for entry in s.owner(p)?.list(p)? {
        if options & 4 != 0 && entry.name.starts_with('.') {
            continue;
        }
        let child = p.join(&entry.name);
        output.push_back((child.clone(), depth));
        if entry.directory
            && options & 1 == 0
            && !(options & 2 != 0 && entry.name.ends_with(".app"))
        {
            tree(s, &child, options, depth + 1, output)?;
        }
    }
    Ok(())
}
unsafe fn enumerate(name: *mut AnyObject, is_url: bool, options: usize) -> *mut AnyObject {
    let Some(p) = (unsafe { path(name, is_url) }) else {
        return ptr::null_mut();
    };
    let result = with(|s| {
        let mut queue = std::collections::VecDeque::new();
        tree(s, &p, options, 1, &mut queue)?;
        Ok(queue)
    });
    let Some(queue) = result else {
        return ptr::null_mut();
    };
    let allocated = OwnedEnumerator::alloc().set_ivars(std::sync::Mutex::new(Enumeration {
        queue,
        current_level: 0,
        strings: !is_url,
        base: p,
    }));
    let object: Retained<OwnedEnumerator> = unsafe { objc2::msg_send![super(allocated), init] };
    Retained::autorelease_return(object).cast()
}
unsafe extern "C" fn enumerate_path(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
) -> *mut AnyObject {
    unsafe { enumerate(name, false, 0) }
}
unsafe extern "C" fn enumerate_url(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    _keys: *mut AnyObject,
    options: usize,
    _handler: *mut AnyObject,
) -> *mut AnyObject {
    unsafe { enumerate(name, true, options) }
}
unsafe extern "C" fn directory(
    _this: *mut AnyObject,
    _sel: Sel,
    kind: usize,
    domain: usize,
    appropriate: *mut AnyObject,
    _create: Bool,
    error: *mut *mut NSError,
) -> *mut AnyObject {
    if !appropriate.is_null() {
        unsafe { fail(error) };
        return ptr::null_mut();
    }
    let value = with(|s| Ok(url(&s.directory(kind, domain)?)));
    if value.is_none() {
        unsafe { fail(error) }
    }
    value.unwrap_or(ptr::null_mut())
}
unsafe extern "C" fn attributes(
    _this: *mut AnyObject,
    _sel: Sel,
    name: *mut AnyObject,
    error: *mut *mut NSError,
) -> *mut AnyObject {
    let Some(p) = (unsafe { path(name, false) }) else {
        unsafe { fail(error) };
        return ptr::null_mut();
    };
    let Some(metadata) = with(|s| s.owner(&p)?.metadata(&p)) else {
        unsafe { fail_at(error, &p) };
        return ptr::null_mut();
    };
    let kind = NSString::from_str(if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
        "NSFileTypeDirectory"
    } else {
        "NSFileTypeRegular"
    });
    let size = NSNumber::numberWithUnsignedLongLong(metadata.st_size.max(0) as u64);
    let epoch = metadata.st_mtime as f64 + metadata.st_mtime_nsec as f64 / 1_000_000_000.0;
    let modified: Retained<AnyObject> = unsafe {
        objc2::msg_send![objc2::class!(NSDate), dateWithTimeIntervalSince1970: epoch]
    };
    let keys = [
        NSString::from_str("NSFileType"),
        NSString::from_str("NSFileSize"),
        NSString::from_str("NSFileModificationDate"),
    ];
    let dict = NSDictionary::from_slices(
        &[&*keys[0], &*keys[1], &*keys[2]],
        &[&*kind as &AnyObject, &*size as &AnyObject, &*modified as &AnyObject],
    );
    Retained::autorelease_return(dict).cast()
}
pub fn scopes() -> BTreeSet<&'static str> {
    [
        "profile-home",
        "account-lookup",
        "filesystem-owned",
        "startup-before-initializers",
    ]
    .into_iter()
    .collect()
}
/// Exact source caller anchor, separate from profile/startup operations. The
/// caller's compiled class/lifecycle/code proof remains independently required.
pub(crate) fn source_allocation_callsites() -> BTreeSet<SourceCallsite> {
    use sha2::{Digest, Sha256};
    let sources:BTreeSet<SourceCallsite>=[SourceCallsite {
        file: "rust-native/src/native.rs",
        source_sha256: format!("{:x}", Sha256::digest(include_bytes!("../../../../rust-native/src/native.rs"))),
        operation: "DefinedClass alloc/set_ivars/selected super-init; same source receiver",
    }].into_iter().collect();
    #[cfg(rcf1_reference_sdk)]
    let sources=sources.into_iter().chain([SourceCallsite {
        file:"Sources/RuycastApp/main.swift",
        source_sha256:format!("{:x}",Sha256::digest(include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/main.swift"))),
        operation:"AppDelegate() at main219: objc_allocWithZone/selected init; same Reference source receiver",
    }, crate::workspace::factory::reference_edit_construction_callsite(),
       crate::workspace::factory::reference_palette_construction_callsite(),
       crate::workspace::factory::reference_snippet_panel_construction_callsite()]).collect();
    sources
}
pub fn source_roster() -> BTreeSet<SourceCallsite> {
    use sha2::{Digest, Sha256};
    [
        (
            "Sources/RuycastApp/main.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/main.swift").as_slice(),
            "defaultProfilePath/NSString tilde",
        ),
        (
            "Sources/RuycastApp/WebsiteIcons.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/WebsiteIcons.swift").as_slice(),
            "init FileManager cache; prune enumeration",
        ),
        (
            "Sources/RuycastApp/AppState.swift",
            include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/AppState.swift").as_slice(),
            "discoverApps enumeration; path tilde",
        ),
        (
            "rust-native/src/main.rs",
            include_bytes!("../../../../rust-native/src/main.rs").as_slice(),
            "profile NSHomeDirectory",
        ),
        (
            "rust-native/src/profile.rs",
            include_bytes!("../../../../rust-native/src/profile.rs").as_slice(),
            "named_home getpwnam_r",
        ),
        (
            "rust-native/src/profile_io_abi.rs",
            include_bytes!("../../../../rust-native/src/profile_io_abi.rs").as_slice(),
            "HostBinding and typed profile request/reply ABI",
        ),
        (
            "rust-native/src/profile_io.rs",
            include_bytes!("../../../../rust-native/src/profile_io.rs").as_slice(),
            "retained Binding read/stat/write/lock/watch; retired IO refuses",
        ),
        (
            "rust-native/src/observe.rs",
            include_bytes!("../../../../rust-native/src/observe.rs").as_slice(),
            "Attached configuration observer stamps/rebind/poll/wake",
        ),
        (
            "rust-native/src/owned_host_entry.rs",
            include_bytes!("../../../../rust-native/src/owned_host_entry.rs").as_slice(),
            "independent image/table/path grant before with_pinned_io",
        ),
        (
            "rust-native/src/native.rs",
            include_bytes!("../../../../rust-native/src/native.rs").as_slice(),
            "run_owned retains bound profile paths in ordinary native graph",
        ),
        (
            "rust-native/src/runtime.rs",
            include_bytes!("../../../../rust-native/src/runtime.rs").as_slice(),
            "owned_channel consumes qualified Host paths for ConfigStore/observer",
        ),
        (
            "rust/src/bin/ruycast.rs",
            include_bytes!("../../../../rust/src/bin/ruycast.rs").as_slice(),
            "bootstrap getpwuid_r",
        ),
        (
            "rust-native/src/native_rows.rs",
            include_bytes!("../../../../rust-native/src/native_rows.rs").as_slice(),
            "cache NSSearchPathForDirectoriesInDomains",
        ),
        (
            "rust-native/src/applications.rs",
            include_bytes!("../../../../rust-native/src/applications.rs").as_slice(),
            "scan FileManager enumeration",
        ),
    ]
    .into_iter()
    .map(|(file, bytes, operation)| SourceCallsite {
        file,
        source_sha256: format!("{:x}", Sha256::digest(bytes)),
        operation,
    })
    .chain(source_allocation_callsites())
    .collect()
}
pub fn required_imports() -> Vec<&'static CStr> {
    vec![
        c"NSHomeDirectory",
        c"NSHomeDirectoryForUser",
        c"NSTemporaryDirectory",
        c"NSUserName",
        c"NSFullUserName",
        c"NSSearchPathForDirectoriesInDomains",
        c"getpwuid",
        c"getpwnam",
        c"getpwuid_r",
        c"getpwnam_r",
        c"getuid",
        c"geteuid",
        c"getgid",
        c"getegid",
    ]
}
/// Exact C imports of the immutable full Reference Main (748fd989...).
/// The other eleven names remain in the checked universe and must be absent;
/// selectors and all replacement implementations retain their full checks.
pub(crate) fn reference_required_imports()->Vec<&'static CStr> {
    vec![c"NSHomeDirectory",c"getpwuid_r",c"getuid"]
}
#[cfg(rcf1_reference_sdk)]
pub(crate) const REFERENCE_PROFILE_LOCK_IMPORT: &CStr =
    c"$s6Darwin4openys5Int32VSPys4Int8VG_ADs6UInt16VtF";
#[cfg(rcf1_reference_sdk)]
pub(crate) const REFERENCE_PROFILE_READ_IMPORT: &CStr =
    c"$s6Darwin4openys5Int32VSPys4Int8VG_ADtF";

#[cfg(rcf1_reference_sdk)]
pub(crate) fn reference_attested_imports() -> Vec<&'static CStr> {
    let mut imports = reference_required_imports();
    imports.extend([REFERENCE_PROFILE_LOCK_IMPORT, REFERENCE_PROFILE_READ_IMPORT, c"lstat", c"unlink", c"bind", c"connect", c"fchmodat", c"listen", c"shutdown", c"socket", c"accept", c"close"]);
    imports
}
fn methods() -> Vec<(&'static CStr, &'static CStr, usize)> {
    vec![
        (
            c"NSFileManager",
            c"homeDirectoryForCurrentUser",
            home as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"homeDirectoryForUser:",
            user_home as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"temporaryDirectory",
            temp as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"URLsForDirectory:inDomains:",
            directories as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"fileExistsAtPath:",
            exists as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"fileExistsAtPath:isDirectory:",
            exists_directory as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"contentsOfDirectoryAtPath:error:",
            contents_path as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"contentsOfDirectoryAtURL:includingPropertiesForKeys:options:error:",
            contents_url as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"createDirectoryAtPath:withIntermediateDirectories:attributes:error:",
            mkdir_path as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"createDirectoryAtURL:withIntermediateDirectories:attributes:error:",
            mkdir_url as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"removeItemAtPath:error:",
            remove_path as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"removeItemAtURL:error:",
            remove_url as *const () as usize,
        ),
        (
            c"NSString",
            c"stringByExpandingTildeInPath",
            expand as *const () as usize,
        ),
    ]
}
type MethodSpec = (&'static CStr, &'static CStr, usize);
unsafe extern "C" {
    fn class_replaceMethod(
        class: *const objc2::runtime::AnyClass,
        selector: Sel,
        imp: objc2::runtime::Imp,
        types: *const libc::c_char,
    ) -> Option<objc2::runtime::Imp>;
}
fn class_family(family: &CStr) -> Result<Vec<&'static objc2::runtime::AnyClass>> {
    let mut result = vec![];
    for class in super::metadata::loaded_classes()? {
        if super::metadata::is_descendant(class, family)? {
            // Actual registered class from the raw roster; no resolving lookup.
            result.push(unsafe { &*(class.address() as *const objc2::runtime::AnyClass) });
        }
    }
    if result.is_empty() {
        return Err("NSString family unavailable".into());
    }
    Ok(result)
}
fn complete_methods() -> Result<Vec<MethodSpec>> {
    let mut m = methods();
    m.retain(|(class, _, _)| *class != c"NSString");
    for class in class_family(c"NSString")? {
        if super::metadata::method(
            super::metadata::registered_class(class as *const _ as usize)?,
            c"stringByExpandingTildeInPath",
            false,
        )?
        .is_none()
        {
            return Err("NSString subclass lacks tilde selector".into());
        }
        let name = unsafe { CStr::from_ptr(class.name().as_ptr()) };
        m.push((
            name,
            c"stringByExpandingTildeInPath",
            expand as *const () as usize,
        ));
    }
    m.extend([
        (
            c"NSFileManager",
            c"enumeratorAtPath:",
            enumerate_path as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"enumeratorAtURL:includingPropertiesForKeys:options:errorHandler:",
            enumerate_url as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"URLForDirectory:inDomain:appropriateForURL:create:error:",
            directory as *const () as usize,
        ),
        (
            c"NSFileManager",
            c"attributesOfItemAtPath:error:",
            attributes as *const () as usize,
        ),
    ]);
    let base: Vec<_> = m
        .iter()
        .filter(|(c, _, _)| *c == c"NSFileManager")
        .copied()
        .collect();
    for class in class_family(c"NSFileManager")? {
        if class.name() != c"NSFileManager" {
            let name = unsafe { CStr::from_ptr(class.name().as_ptr()) };
            for (_, selector, replacement) in &base {
                m.push((name, *selector, *replacement));
            }
        }
    }
    Ok(m)
}
pub fn required_selectors() -> Result<Vec<(&'static CStr, &'static CStr, bool)>> {
    Ok(complete_methods()?
        .into_iter()
        .map(|(c, s, _)| (c, s, false))
        .collect())
}
pub fn selector_replacement(class: &CStr, selector: &CStr, class_method: bool) -> Option<usize> {
    if class_method {
        return None;
    }
    complete_methods()
        .ok()?
        .into_iter()
        .find_map(|(c, s, p)| (c == class && s == selector).then_some(p))
}
/// Installs methods without calling any previous implementation. Must run in a
/// fresh owned fake/guarded endpoint before it creates any original object.
/// # Safety
/// Process-wide exclusive pre-initialization admission; no concurrent swizzler
/// and no live original object or active Foundation operation may race this.
pub unsafe fn install_selectors() -> Result<Vec<super::ObjcReplacement>> {
    unsafe{install_selectors_checked(&||Ok(()))}
}
unsafe fn install_selectors_checked(prewrite:&impl Fn()->Result<()>) -> Result<Vec<super::ObjcReplacement>> {
    if with(|s| s.before()).is_none() {
        return Err("selector admission requires owned context and live guardian".into());
    }
    let mut output: Vec<super::ObjcReplacement> = vec![];
    let mut plan = vec![];
    for (class, selector, replacement) in complete_methods()? {
        let c = super::metadata::loaded_class(class)?;
        let before =
            super::metadata::method(c, selector, false)?.ok_or("bootstrap selector unavailable")?;
        if before.implementation() == replacement {
            return Err("selector already owned before bootstrap admission".into());
        }
        let lifecycle = super::metadata::lifecycle(c)?;
        plan.push((class, selector, replacement, c, before, lifecycle));
    }
    for (class, selector, replacement, c, before, lifecycle) in plan {
        let original = before.implementation();
        let at_write =
            super::metadata::method(c, selector, false)?.ok_or("selector disappeared")?;
        if super::metadata::lifecycle(c)? != lifecycle {
            return Err("lifecycle changed before selector mutation".into());
        }
        let prior = if at_write != before {
            Some(
                &output
                    .iter()
                    .find(|r| {
                        r.selector == selector
                            && r.provenance.selected_owner() == at_write.owner()
                            && r.replacement == at_write.implementation()
                    })
                    .ok_or("inherited selector changed without owned ancestor")?
                    .provenance,
            )
        } else {
            None
        };
        let encoding =
            std::ffi::CString::new(before.encoding()).map_err(|_| "invalid ABI encoding")?;
        prewrite()?;
        #[cfg(rcf1_reference_sdk)]
        {
            // The dynamic Source/guardian boundary may reenter metadata setup
            // while returning Ok. Never overwrite its changed IMP/lifecycle.
            if super::metadata::method(c,selector,false)?.as_ref()!=Some(&at_write)
                || super::metadata::lifecycle(c)?!=lifecycle {
                return Err("Reference selector changed during prewrite qualification".into());
            }
            CONTEXT.get().ok_or("Reference bootstrap absent before selector write")?.reference_not_retired()?;
        }
        let previous = unsafe {
            class_replaceMethod(
                c.address() as *const objc2::runtime::AnyClass,
                Sel::register(selector),
                std::mem::transmute::<usize, objc2::runtime::Imp>(replacement),
                encoding.as_ptr(),
            )
        };
        if previous.is_some_and(|p| p as usize != at_write.implementation()) {
            return Err("selector changed during admission".into());
        }
        let after =
            super::metadata::method(c, selector, false)?.ok_or("selected selector disappeared")?;
        let provenance = super::metadata::ReplacementProvenance::bind(
            before, at_write, after, lifecycle, false, prior,
        )?;
        output.push(super::ObjcReplacement {
            class,
            selector,
            class_method: false,
            original,
            replacement,
            provenance,
        });
    }
    prewrite()?;
    Ok(output)
}

// dyld's documented two-pointer interposition ABI; the originals are addresses
// only and are never called. Reference retains these exact import specs as
// ordinary data, without loader interposition; it must qualify real main cells.
#[cfg(not(test))]
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "NSHomeDirectory"]
    fn original_home();
    #[link_name = "NSHomeDirectoryForUser"]
    fn original_user_home();
    #[link_name = "NSTemporaryDirectory"]
    fn original_temp();
    #[link_name = "NSUserName"]
    fn original_username();
    #[link_name = "NSFullUserName"]
    fn original_fullname();
    #[link_name = "NSSearchPathForDirectoriesInDomains"]
    fn original_search();
}
#[cfg(not(test))]
#[repr(C)]
struct Interpose {
    replacement: *const (),
    original: *const (),
}
#[cfg(not(test))]
unsafe impl Sync for Interpose {}
#[cfg(not(test))]
#[used]
#[cfg_attr(not(rcf1_reference_sdk),link_section = "__DATA,__interpose")]
static INTERPOSE: [Interpose; 14] = [
    Interpose {
        replacement: rcf1_NSHomeDirectory as *const (),
        original: original_home as *const (),
    },
    Interpose {
        replacement: rcf1_NSHomeDirectoryForUser as *const (),
        original: original_user_home as *const (),
    },
    Interpose {
        replacement: rcf1_NSTemporaryDirectory as *const (),
        original: original_temp as *const (),
    },
    Interpose {
        replacement: rcf1_NSUserName as *const (),
        original: original_username as *const (),
    },
    Interpose {
        replacement: rcf1_NSFullUserName as *const (),
        original: original_fullname as *const (),
    },
    Interpose {
        replacement: rcf1_NSSearchPathForDirectoriesInDomains as *const (),
        original: original_search as *const (),
    },
    Interpose {
        replacement: rcf1_getpwuid as *const (),
        original: libc::getpwuid as *const (),
    },
    Interpose {
        replacement: rcf1_getpwnam as *const (),
        original: libc::getpwnam as *const (),
    },
    Interpose {
        replacement: rcf1_getpwuid_r as *const (),
        original: libc::getpwuid_r as *const (),
    },
    Interpose {
        replacement: rcf1_getpwnam_r as *const (),
        original: libc::getpwnam_r as *const (),
    },
    Interpose {
        replacement: rcf1_getuid as *const (),
        original: libc::getuid as *const (),
    },
    Interpose {
        replacement: rcf1_geteuid as *const (),
        original: libc::geteuid as *const (),
    },
    Interpose {
        replacement: rcf1_getgid as *const (),
        original: libc::getgid as *const (),
    },
    Interpose {
        replacement: rcf1_getegid as *const (),
        original: libc::getegid as *const (),
    },
];

/// Only known replacement addresses can be attached to import cells. An
/// endpoint must additionally prove the actual loader cell/source-image link.
pub fn replacement(symbol: &CStr) -> Option<usize> {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == REFERENCE_PROFILE_LOCK_IMPORT {
        return Some(super::core_preentry::reference_profile_lock_open as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == REFERENCE_PROFILE_READ_IMPORT {
        return Some(super::core_preentry::reference_profile_read_open as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"lstat" {
        return Some(super::core_preentry::reference_file_lstat as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"unlink" {
        return Some(super::core_preentry::reference_file_unlink as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"bind" {
        return Some(super::core_preentry::reference_control_bind as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"connect" {
        return Some(super::core_preentry::reference_control_connect as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"fchmodat" {
        return Some(super::core_preentry::reference_control_chmod as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"listen" {
        return Some(super::core_preentry::reference_control_listen as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"shutdown" {
        return Some(super::core_preentry::reference_control_shutdown as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"socket" {
        return Some(super::core_preentry::reference_control_new_socket as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"accept" {
        return Some(super::core_preentry::reference_control_accept as *const () as usize);
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if symbol == c"close" {
        return Some(super::core_preentry::reference_control_close as *const () as usize);
    }
    let methods: [(&CStr, usize); 14] = [
        (
            c"NSHomeDirectory",
            rcf1_NSHomeDirectory as *const () as usize,
        ),
        (
            c"NSHomeDirectoryForUser",
            rcf1_NSHomeDirectoryForUser as *const () as usize,
        ),
        (
            c"NSTemporaryDirectory",
            rcf1_NSTemporaryDirectory as *const () as usize,
        ),
        (c"NSUserName", rcf1_NSUserName as *const () as usize),
        (c"NSFullUserName", rcf1_NSFullUserName as *const () as usize),
        (
            c"NSSearchPathForDirectoriesInDomains",
            rcf1_NSSearchPathForDirectoriesInDomains as *const () as usize,
        ),
        (c"getpwuid", rcf1_getpwuid as *const () as usize),
        (c"getpwnam", rcf1_getpwnam as *const () as usize),
        (c"getpwuid_r", rcf1_getpwuid_r as *const () as usize),
        (c"getpwnam_r", rcf1_getpwnam_r as *const () as usize),
        (c"getuid", rcf1_getuid as *const () as usize),
        (c"geteuid", rcf1_geteuid as *const () as usize),
        (c"getgid", rcf1_getgid as *const () as usize),
        (c"getegid", rcf1_getegid as *const () as usize),
    ];
    methods
        .into_iter()
        .find_map(|(name, p)| if name == symbol { Some(p) } else { None })
}

/// Inspect real non-lazy/lazy import cells of this process's main image. The
/// source-owned fake calls this BEFORE any test import. Unsupported loader
/// layouts fail closed. It never invokes the original imported function.
#[cfg(not(test))]
pub fn main_imports() -> Result<Vec<super::ImportReplacement>> {
    let specs: Vec<_> = required_imports()
        .into_iter()
        .enumerate()
        .map(|(i, symbol)| super::ImportSpec {
            symbol,
            original: INTERPOSE[i].original as usize,
            replacement: INTERPOSE[i].replacement as usize,
        })
        .collect();
    observe_main_imports(&specs)
}
/// Query-only identity export used by a pure Rust fake before it may invoke
/// one named replacement. It is not a ready/SDK acceptance operation.
/// # Safety
/// `name` is a valid NUL-terminated C symbol name.
#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn rcf1_main_import_selected(name: *const libc::c_char) -> usize {
    if name.is_null() {
        return 0;
    }
    let wanted = unsafe { CStr::from_ptr(name) };
    main_imports()
        .map_err(|error| eprintln!("owned import admission refused: {error}"))
        .ok()
        .and_then(|m| m.into_iter().find(|i| i.symbol == wanted))
        .map_or(0, |i| i.selected_address())
}

#[cfg(test)]
mod tests {
    use super::super::owned::OwnedHandleLease;
    use super::super::{ImportReplacement, ModuleAttestation, ObjcReplacement, Registry};
    use super::*;
    use objc2::ClassType;
    #[cfg(rcf1_reference_sdk)]
    #[test]
    fn reference_setup_failure_and_unwind_retire_the_actual_context_without_callbacks() {
        let fixture=crate::workspace::tests::Fixture::new();
        let make=|| {
            let home=OwnedRoot::open(&fixture.root.join("home")).unwrap();
            let (uid,gid)=home.owner_ids().unwrap();
            let account=Account::fixture(uid,gid,"owned","Owned",home.path()).unwrap();
            Bootstrap::new(home,OwnedRoot::open(&fixture.root.join("cache")).unwrap(),
                OwnedRoot::open(&fixture.root.join("temp")).unwrap(),OwnedRoot::open(&fixture.root.join("apps")).unwrap(),
                account,fixture.life.clone()).unwrap()
        };
        let failed=make();
        {
            let _guard = ReferenceBootstrapPublication {
                context: &failed,
                workspace: Some(fixture.model.clone()),
                complete: false,
            };
            fixture.model.check_generation_pure(1).unwrap();
        }
        assert!(failed.before().is_err());
        assert!(fixture.model.check_generation_pure(1).is_err());
        let unwound=make();
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard=ReferenceBootstrapPublication{context:&unwound,workspace:None,complete:false};
            panic!("owned Reference setup unwind");
        }));
        assert!(result.is_err());assert!(unwound.before().is_err());
        let complete=make();
        {let _guard=ReferenceBootstrapPublication{context:&complete,workspace:None,complete:true};}
        complete.before().unwrap();
        // These real context flags are tested without publishing CONTEXT,
        // patching imports/selectors, or qualifying a Source14 admission.
    }
    #[test]
    fn source_allocator_import_is_exact_and_unknown_classes_never_allocate() {
        let abi: unsafe extern "C" fn(*const objc2::runtime::AnyClass) -> *mut AnyObject =
            rcf1_objc_alloc;
        let spec = source_allocation_import_spec();
        assert_eq!(spec.symbol, c"objc_alloc");
        assert_eq!(spec.original, objc2::ffi::objc_alloc as *const () as usize);
        assert_eq!(spec.replacement, abi as *const () as usize);
        assert_ne!(spec.original, spec.replacement);
        assert_ne!(spec.original, 0);
        let before = rejected_calls();
        assert!(unsafe { abi(ptr::null()) }.is_null());
        // This address is never dereferenced: runtime class membership fails.
        assert!(unsafe { abi(usize::MAX as *const objc2::runtime::AnyClass) }.is_null());
        assert!(rejected_calls() > before);
        let zone_abi:unsafe extern "C" fn(*const objc2::runtime::AnyClass)->*mut AnyObject=rcf1_objc_alloc_with_zone;
        let original_zone:unsafe extern "C-unwind" fn(*const objc2::runtime::AnyClass)->*mut AnyObject=objc2::ffi::objc_allocWithZone;
        let zone=source_allocation_with_zone_import_spec();
        assert_eq!(zone.symbol,c"objc_allocWithZone");
        assert_eq!(zone.original,original_zone as *const () as usize);
        assert_eq!(zone.replacement,zone_abi as *const () as usize);
        assert_ne!(zone.original,zone.replacement);
        assert!(unsafe{zone_abi(ptr::null())}.is_null());
        assert!(unsafe{zone_abi(usize::MAX as *const objc2::runtime::AnyClass)}.is_null());
    }
    use std::sync::atomic::AtomicBool;
    struct Life(AtomicBool);
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<()> {
            if self.is_alive() {
                Ok(())
            } else {
                Err("own dead lease".into())
            }
        }
    }
    fn fixture() -> &'static Path {
        static ROOT: OnceLock<PathBuf> = OnceLock::new();
        ROOT.get_or_init(|| {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("explicit own fixture parent required"));
            assert!(parent.is_absolute());
            assert_eq!(std::fs::canonicalize(&parent).unwrap(), parent);
            let metadata = std::fs::symlink_metadata(&parent).unwrap();
            assert!(metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o700);
            let root = parent.join(format!("rc-bootstrap-units-{}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            for d in [
                "home",
                "cache",
                "temp",
                "apps/A.app/Contents",
                "apps/B.app/Contents",
            ] {
                std::fs::create_dir_all(root.join(d)).unwrap();
            }
            std::fs::write(root.join("home/config.json"), b"owned configuration").unwrap();
            std::fs::write(
                root.join("apps/A.app/Contents/Info.plist"),
                b"owned fake metadata",
            )
            .unwrap();
            let live: Arc<dyn AliveLease> = Arc::new(Life(AtomicBool::new(true)));
            let context = Bootstrap::new(
                OwnedRoot::open(&root.join("home")).unwrap(),
                OwnedRoot::open(&root.join("cache")).unwrap(),
                OwnedRoot::open(&root.join("temp")).unwrap(),
                OwnedRoot::open(&root.join("apps")).unwrap(),
                Account::fixture(
                    OwnedRoot::open(&root.join("home"))
                        .unwrap()
                        .owner_ids()
                        .unwrap()
                        .0,
                    OwnedRoot::open(&root.join("home"))
                        .unwrap()
                        .owner_ids()
                        .unwrap()
                        .1,
                    "owned-fixture",
                    "Owned Fixture",
                    &root.join("home"),
                )
                .unwrap(),
                live,
            )
            .unwrap();
            install_context(context).unwrap();
            root
        })
        .as_path()
    }
    fn context() -> &'static Bootstrap {
        fixture();
        CONTEXT.get().unwrap()
    }
    fn selector_records() -> &'static Vec<ObjcReplacement> {
        static RECORDS: OnceLock<Vec<ObjcReplacement>> = OnceLock::new();
        context();
        RECORDS.get_or_init(|| unsafe { install_selectors().unwrap() })
    }
    #[test]
    fn actual_foundation_tilde_classcluster_selection_and_negative_named_home() {
        let s = context();
        objc2::rc::autoreleasepool(|_| unsafe {
            let records = selector_records();
            assert!(records.len() >= 17);
            for record in records {
                assert_eq!(record.selected().unwrap(), record.replacement);
            }
            for value in ["~/x", "~owned-fixture/x"] {
                let input = NSString::from_str(value);
                let output: *mut NSString = objc2::msg_send![&*input, stringByExpandingTildeInPath];
                assert!(!output.is_null());
                assert_eq!(
                    (*output).to_string(),
                    s.home.path().join("x").to_str().unwrap()
                );
                assert!(records.iter().any(|r| r.class == input.class().name()
                    && r.selector == c"stringByExpandingTildeInPath"));
            }
            let unknown = NSString::from_str("~__NO_REAL_ACCOUNT_FALLBACK__/x");
            let output: *mut NSString = objc2::msg_send![&*unknown, stringByExpandingTildeInPath];
            assert_eq!((*output).to_string(), unknown.to_string());
            let relative = NSString::from_str("relative/x");
            let output: *mut NSString = objc2::msg_send![&*relative, stringByExpandingTildeInPath];
            assert_eq!((*output).to_string(), relative.to_string());
        });
    }
    #[test]
    fn native_persistence_source_route_requires_qualified_selector_before_open_and_reload() {
        let s = context();
        let directory = s.temp.path().join("native-profile");
        s.temp.mkdir(&directory, false).unwrap();
        let profile = s.qualify_profile(&directory, None).unwrap();
        let mut store = profile.open(s).unwrap();
        assert!(store.snapshot_is_current(None).unwrap());
        let value: serde_json::Value = serde_json::from_slice(store.snapshot()).unwrap();
        store.save(&value).unwrap();
        assert!(store.snapshot_is_current(None).unwrap());
        let root = OwnedRoot::open(&directory).unwrap();
        let file = directory.join("config-location.json");
        root.create_file(
            &file,
            br#"{"directory":"/__NO_PRODUCT_FS_FALLBACK__/canary"}"#,
        )
        .unwrap();
        let revision = store.revision;
        assert!(profile
            .reload(s, &mut store)
            .unwrap_err()
            .contains("unowned"));
        assert_eq!(store.revision, revision);
        root.remove(&file).unwrap();
        std::os::unix::fs::symlink(directory.join("config.json"), &file).unwrap();
        assert!(profile.reload(s, &mut store).is_err());
        std::fs::remove_file(file).unwrap();
        assert!(s
            .qualify_profile(&directory, Some("~__UNKNOWN_ACCOUNT__/outside"))
            .is_err());
        assert!(s.qualify_profile(&directory, Some("../outside")).is_err());
        assert!(s
            .qualify_profile(&directory, Some("/__NO_PRODUCT_FS_FALLBACK__"))
            .is_err());
        s.temp.remove(&directory).unwrap();
    }
    #[test]
    fn explicit_home_cache_temp_and_named_tilde_without_environment_changes() {
        let s = context();
        objc2::rc::autoreleasepool(|_| unsafe {
            assert_eq!(
                text(rcf1_NSHomeDirectory()).unwrap(),
                s.home.path().to_str().unwrap()
            );
            assert_eq!(text(rcf1_NSUserName()).unwrap(), "owned-fixture");
            assert_eq!(text(rcf1_NSFullUserName()).unwrap(), "Owned Fixture");
            assert_eq!(
                text(rcf1_NSTemporaryDirectory()).unwrap(),
                format!("{}/", s.temp.path().display())
            );
            assert_eq!(
                s.expand("~owned-fixture/config.json").unwrap(),
                s.home.path().join("config.json")
            );
            assert!(s.expand("~__unknown_canary__/file").is_err());
            assert!(s.expand("~/../temp/file").is_err());
            assert!(s.directory(13, 2).is_err());
            assert!(s.directory(9999, 1).is_err());
            let dirs = rcf1_NSSearchPathForDirectoriesInDomains(13, 1, Bool::YES);
            assert!(!dirs.is_null());
            let values = &*dirs.cast::<NSArray<NSString>>();
            assert_eq!(values.len(), 1);
            assert_eq!(
                values.objectAtIndex(0).to_string(),
                s.cache.path().to_str().unwrap()
            );
        });
    }
    #[test]
    fn account_reentrant_storage_small_buffer_unknown_and_nonreentrant_lifetime() {
        let uid = context().account.uid;
        unsafe {
            let mut pw = std::mem::zeroed();
            let mut buffer = [0_i8; 4096];
            let mut found = ptr::null_mut();
            assert_eq!(
                rcf1_getpwuid_r(uid, &mut pw, buffer.as_mut_ptr(), buffer.len(), &mut found),
                0
            );
            assert_eq!(found, &mut pw as *mut _);
            assert_eq!(CStr::from_ptr(pw.pw_name), c"owned-fixture");
            assert_eq!(pw.pw_uid, uid);
            assert!(
                pw.pw_dir >= buffer.as_mut_ptr()
                    && pw.pw_dir < buffer.as_mut_ptr().add(buffer.len())
            );
            assert_eq!(
                rcf1_getpwnam_r(
                    c"owned-fixture".as_ptr(),
                    &mut pw,
                    buffer.as_mut_ptr(),
                    1,
                    &mut found
                ),
                libc::ERANGE
            );
            assert!(found.is_null());
            assert_eq!(
                rcf1_getpwuid_r(0, &mut pw, buffer.as_mut_ptr(), buffer.len(), &mut found),
                libc::ENOENT
            );
            assert!(found.is_null());
            assert_eq!(
                rcf1_getpwnam_r(
                    c"__NO_REAL_ACCOUNT_QUERY__".as_ptr(),
                    &mut pw,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found
                ),
                libc::ENOENT
            );
            assert!(found.is_null());
            let p = rcf1_getpwnam(c"owned-fixture".as_ptr());
            assert!(!p.is_null());
            assert_eq!(CStr::from_ptr((*p).pw_name), c"owned-fixture");
            assert!(rcf1_getpwuid(0).is_null());
        }
    }
    #[test]
    fn descriptor_namespace_parents_symlinks_hardlinks_and_repeat_enumeration() {
        let s = context();
        let path = s.temp.path().join("descriptor-case");
        s.temp.mkdir(&path, false).unwrap();
        std::fs::write(path.join("a"), b"owned").unwrap();
        let own = OwnedRoot::open(&path).unwrap();
        assert_eq!(own.list(&path).unwrap().len(), 1);
        assert_eq!(own.list(&path).unwrap().len(), 1);
        assert!(own.checked(&path.join("../home/config.json")).is_err());
        assert!(own
            .checked(Path::new("/__OWNED_UNKNOWN_NAMESPACE_NO_QUERY__/file"))
            .is_err());
        std::os::unix::fs::symlink(s.home.path(), path.join("symlink")).unwrap();
        assert!(own.checked(&path.join("symlink/config.json")).is_err());
        assert!(OwnedRoot::open(&path.join("symlink")).is_err());
        assert!(own.list(&path).is_err());
        std::fs::remove_file(path.join("symlink")).unwrap();
        std::fs::hard_link(path.join("a"), path.join("hard")).unwrap();
        assert!(own.checked(&path.join("a")).is_err());
        std::fs::remove_file(path.join("hard")).unwrap();
        assert!(own.mkdir(&path, false).is_err());
        own.remove(&path.join("a")).unwrap();
        assert!(own.list(&path).unwrap().is_empty());
        s.temp.remove(&path).unwrap();
    }
    #[test]
    fn filemanager_replacements_read_only_owned_fake_fields_and_mutate_owned_tree() {
        let s = context();
        objc2::rc::autoreleasepool(|_| unsafe {
            let p = NSString::from_str(s.home.path().join("config.json").to_str().unwrap());
            let raw = Retained::as_ptr(&p).cast_mut().cast();
            assert!(exists(ptr::null_mut(), objc2::sel!(fileExistsAtPath:), raw).as_bool());
            let unknown = NSString::from_str("/__NO_REAL_FILE_QUERY__/canary");
            assert!(!exists(
                ptr::null_mut(),
                objc2::sel!(fileExistsAtPath:),
                Retained::as_ptr(&unknown).cast_mut().cast()
            )
            .as_bool());
            let mut error = ptr::null_mut();
            let attrs = attributes(
                ptr::null_mut(),
                objc2::sel!(attributesOfItemAtPath:error:),
                raw,
                &mut error,
            );
            assert!(!attrs.is_null());
            assert!(error.is_null());
            let cache_file = s.cache.path().join("website-metadata.miss");
            let file = std::fs::File::create(&cache_file).unwrap();
            let modified = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_125);
            file.set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
            let cache_path = NSString::from_str(cache_file.to_str().unwrap());
            let cache_attributes = attributes(
                ptr::null_mut(),
                objc2::sel!(attributesOfItemAtPath:error:),
                Retained::as_ptr(&cache_path).cast_mut().cast(),
                &mut error,
            );
            assert!(!cache_attributes.is_null() && error.is_null());
            let key = NSString::from_str("NSFileModificationDate");
            let date: *mut AnyObject = objc2::msg_send![cache_attributes, objectForKey: &*key];
            assert!(!date.is_null());
            let epoch: f64 = objc2::msg_send![date, timeIntervalSince1970];
            assert_eq!(epoch, 1_700_000_000.125);
            std::fs::remove_file(cache_file).unwrap();
            let made = NSString::from_str(s.temp.path().join("fm/new").to_str().unwrap());
            let made_ptr = Retained::as_ptr(&made).cast_mut().cast();
            assert!(mkdir_path(
                ptr::null_mut(),
                objc2::sel!(createDirectoryAtPath:withIntermediateDirectories:attributes:error:),
                made_ptr,
                Bool::YES,
                ptr::null_mut(),
                &mut error
            )
            .as_bool());
            assert!(remove_path(
                ptr::null_mut(),
                objc2::sel!(removeItemAtPath:error:),
                made_ptr,
                &mut error
            )
            .as_bool());
            assert!(!remove_path(
                ptr::null_mut(),
                objc2::sel!(removeItemAtPath:error:),
                Retained::as_ptr(&unknown).cast_mut().cast(),
                &mut error
            )
            .as_bool());
            assert!(!error.is_null());
        });
    }
    #[test]
    fn owned_enumerator_real_objc_next_level_skip_and_virtual_application_roots() {
        let s = context();
        objc2::rc::autoreleasepool(|_| unsafe {
            let root = NSString::from_str("/Applications");
            let u = NSURL::fileURLWithPath(&root);
            let e = enumerate_url(
                ptr::null_mut(),
                objc2::sel!(enumeratorAtURL:includingPropertiesForKeys:options:errorHandler:),
                Retained::as_ptr(&u).cast_mut().cast(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
            );
            assert!(!e.is_null());
            let items = &*e.cast::<NSEnumerator<NSURL>>();
            let first = items.nextObject().unwrap();
            assert_eq!(
                first.path().unwrap().to_string(),
                s.applications.path().join("A.app").to_str().unwrap()
            );
            let level: usize = objc2::msg_send![e, level];
            assert_eq!(level, 1);
            let _: () = objc2::msg_send![e, skipDescendants];
            let second = items.nextObject().unwrap();
            assert_eq!(
                second.path().unwrap().to_string(),
                s.applications.path().join("B.app").to_str().unwrap()
            );
            let _: () = objc2::msg_send![e, skipDescendants];
            assert!(items.nextObject().is_none());
        });
    }
    #[test]
    fn complete_owned_fake_leaf_roster_refuses_unqualified_routes_duplicate_partial_and_stale() {
        let s = context();
        let names = required_imports();
        let mut cells: Vec<_> = names
            .iter()
            .map(|n| Box::new(replacement(n).unwrap()))
            .collect();
        let imports = names
            .into_iter()
            .zip(&cells)
            .map(|(name, cell)| {
                unsafe {
                    ImportReplacement::from_import_cell(
                        name,
                        (**cell).wrapping_add(4),
                        **cell,
                        &**cell,
                    )
                }
                .unwrap()
            })
            .collect();
        let module = ModuleAttestation {
            module: "bootstrap",
            workspace_factories: None,
            scopes: scopes(),
            callsites: source_roster(),
            imports,
            objc: selector_records().clone(),
            handles: vec![OwnedHandleLease {
                namespace: "fixture.bootstrap",
                root: s.home.clone(),
                life: s.life.clone(),
            }],
        };
        let mut registry = Registry::new(s.life.clone());
        assert!(registry
            .register(module.clone())
            .unwrap_err()
            .contains("not fully wired"));
        // Private owned state only to exercise duplicate/changed-cell policy;
        // this does not bypass the real registration/admission gate.
        registry.modules.insert("bootstrap", module.clone());
        assert_eq!(registry.modules.len(), 1);
        assert_eq!(registry.ready(), 0);
        assert!(!registry.retired.load(Ordering::Acquire));
        assert!(registry
            .register(module.clone())
            .unwrap_err()
            .contains("duplicate"));
        let mut missing = module.clone();
        missing.imports.pop();
        assert!(Registry::new(s.life.clone()).register(missing).is_err());
        let mut forged = module.clone();
        forged.scopes = super::super::SCOPES.into_iter().collect();
        assert!(Registry::new(s.life.clone()).register(forged).is_err());
        let mut missing_selector = module;
        missing_selector.objc.pop();
        assert!(Registry::new(s.life.clone())
            .register(missing_selector)
            .is_err());
        let original = *cells[0];
        *cells[0] = 1;
        assert_eq!(registry.ready(), 0);
        assert!(registry.retired.load(Ordering::Acquire));
        *cells[0] = original;
        assert_eq!(registry.ready(), 0);
    }
    #[test]
    fn selected_import_address_not_a_boolean_and_missing_module_never_ready() {
        let s = context();
        let replacement = rcf1_getpwuid as *const () as usize;
        let selected = replacement;
        let binding =
            unsafe { ImportReplacement::from_import_cell(c"getpwuid", 1, replacement, &selected) }
                .unwrap();
        assert_eq!(binding.selected_address(), replacement);
        let wrong = 2;
        let changed =
            unsafe { ImportReplacement::from_import_cell(c"getpwuid", 1, replacement, &wrong) }
                .unwrap();
        let module = ModuleAttestation {
            module: "bootstrap",
            workspace_factories: None,
            scopes: scopes(),
            callsites: source_roster(),
            imports: vec![changed],
            objc: vec![],
            handles: vec![OwnedHandleLease {
                namespace: "fixture.home",
                root: s.home.clone(),
                life: s.life.clone(),
            }],
        };
        assert!(module
            .verify()
            .unwrap_err()
            .contains("selected import differs"));
        let live = Arc::new(Life(AtomicBool::new(true)));
        let registry = Registry::new(live.clone());
        assert_eq!(registry.ready(), 0);
        live.0.store(false, Ordering::Release);
        assert_eq!(registry.ready(), 0);
        assert!(registry.retired.load(Ordering::Acquire));
        live.0.store(true, Ordering::Release);
        assert_eq!(registry.ready(), 0);
        assert_eq!(super::super::ruycast_f1_preentry_ready_v1(), 0);
    }
    #[test]
    fn own_fake_selector_actual_runtime_imp_must_match_before_call() {
        let _s = context();
        static ORIGINAL: AtomicU64 = AtomicU64::new(0);
        unsafe extern "C" fn original(_: *mut AnyObject, _: Sel) -> usize {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
            1
        }
        unsafe extern "C" fn replaced(_: *mut AnyObject, _: Sel) -> usize {
            77
        }
        let mut builder =
            objc2::runtime::ClassBuilder::new(c"RuycastF1OwnedIdentityProbe", NSObject::class())
                .unwrap();
        unsafe {
            builder.add_method(
                objc2::sel!(ownedIdentityProbe),
                original as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            )
        };
        let class = builder.register();
        let pointer = super::super::metadata::registered_class(class as *const _ as usize).unwrap();
        let before = super::super::metadata::method(pointer, c"ownedIdentityProbe", false)
            .unwrap()
            .unwrap();
        let lifecycle = super::super::metadata::lifecycle(pointer).unwrap();
        unsafe {
            class_replaceMethod(
                class,
                objc2::sel!(ownedIdentityProbe),
                std::mem::transmute::<usize, objc2::runtime::Imp>(replaced as *const () as usize),
                c"Q@:".as_ptr(),
            );
        }
        let after = super::super::metadata::method(pointer, c"ownedIdentityProbe", false)
            .unwrap()
            .unwrap();
        let provenance = super::super::metadata::ReplacementProvenance::bind(
            before.clone(),
            before,
            after,
            lifecycle,
            false,
            None,
        )
        .unwrap();
        let record = ObjcReplacement {
            class: c"RuycastF1OwnedIdentityProbe",
            selector: c"ownedIdentityProbe",
            class_method: false,
            original: original as *const () as usize,
            replacement: replaced as *const () as usize,
            provenance,
        };
        assert_eq!(record.selected().unwrap(), record.replacement);
        let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
        let value: usize = unsafe { objc2::msg_send![&*object, ownedIdentityProbe] };
        assert_eq!(value, 77);
        assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
    }
}
