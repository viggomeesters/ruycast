//! Type-preserving owned subclass construction. Runtime metadata only; never
//! calls an AppKit initializer or previous IMP. Whole-image +load coverage is a
//! distinct mandatory bootstrap proof, not supplied by this class selection.
use super::*;
use crate::sdk::{metadata, ObjcReplacement};
use objc2::{
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, Imp, Method, Sel},
    ClassType,
};
use objc2_foundation::NSObject;
use std::{
    collections::BTreeSet,
    ffi::CStr,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};
fn raw(class: &AnyClass, selector: Sel, meta: bool) -> Result<Option<metadata::MethodSnapshot>> {
    metadata::method(
        metadata::registered_class(class as *const AnyClass as usize)?,
        selector.name(),
        meta,
    )
}
/// Constructor mutation provenance is deliberately distinct from the common
/// ordinary-leaf invariant that lifecycle remains unchanged. All snapshot and
/// lifecycle-delta authority stays in the common metadata implementation.
#[derive(Clone)]
pub struct ConstructorBinding {
    pub class: &'static CStr,
    pub selector: &'static CStr,
    pub class_method: bool,
    pub original: usize,
    pub replacement: usize,
    provenance: metadata::ReplacementProvenance,
    faulted: Arc<AtomicBool>,
}
impl ConstructorBinding {
    pub fn verify(&self) -> Result<()> {
        metadata::with_verification_roster(|roster| self.verify_with_roster(roster))
    }
    pub(super) fn verify_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        if self.faulted.load(Ordering::Acquire) {
            return Err("constructor authority retired after failure".into());
        }
        self.verify_metadata_with_roster(roster)
    }
    /// Cleanup may outlive business admission. This verifies only immutable
    /// method/lifecycle provenance and never authorizes allocation or callbacks.
    pub(super) fn verify_metadata_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        let h = roster.loaded_class(self.class)?;
        if !self
            .provenance
            .matches_key(h, self.selector, self.class_method)
            || self.provenance.original() != self.original
            || self.provenance.replacement() != self.replacement
            || self.provenance.verify_with_roster(roster)? != self.replacement
        {
            return Err("constructor method/lifecycle provenance changed".into());
        }
        Ok(())
    }
}
/// Explicit owned memory declaration profile, never an ordinary replacement.
/// Same-IMP/new-owner slots are represented by their actual before/selected
/// metadata, with the NSObject primitive and full constructor provenance.
#[derive(Clone)]
pub(crate) struct OwnedMemoryArcBinding(Arc<OwnedMemoryArcInner>);
struct OwnedMemoryArcInner {
    class: &'static CStr,
    parent: super::factory::FoundationMemoryParent,
    plan: super::factory::FoundationMemoryPlan,
    before: Vec<(&'static CStr, metadata::MethodSnapshot)>,
    selected: Vec<(&'static CStr, metadata::MethodSnapshot)>,
    lifecycle: metadata::LifecycleSnapshot,
    constructors: Vec<ConstructorBinding>,
    model: Arc<Workspace>,
    generation: u64,
    retired: AtomicBool,
}
impl OwnedMemoryArcBinding {
    pub(crate) fn class(&self) -> &'static CStr {
        self.0.class
    }
    pub(crate) fn original(&self) -> usize {
        self.0.parent.methods[0].1.implementation()
    }
    pub(crate) fn replacement(&self) -> usize {
        self.0.selected[0].1.implementation()
    }
    pub(crate) fn belongs_to(&self, model: &Arc<Workspace>) -> bool {
        Arc::ptr_eq(&self.0.model, model)
    }
    pub(crate) fn root_matches(&self, root: &super::factory::RootDeallocator) -> bool {
        &self.0.plan.root == root
    }
    pub(crate) fn verify(&self) -> Result<()> {
        if self.0.model.faulted.load(Ordering::Acquire) {
            return Err("owned memory business authority retired".into());
        }
        {
            let state = self
                .0
                .model
                .state
                .lock()
                .map_err(|_| "workspace state poisoned")?;
            if state.retired || state.generation != self.0.generation {
                return Err("owned memory generation retired".into());
            }
        }
        self.verify_metadata()
    }
    pub(crate) fn verify_metadata(&self) -> Result<()> {
        if self.0.retired.load(Ordering::Acquire) {
            return Err("owned memory metadata permanently retired".into());
        }
        let result = metadata::with_verification_roster(|roster| {
            self.0.plan.verify_arc_sources()?;
            let class = roster.loaded_class(self.0.class)?;
            let actual = unsafe { &*(class.address() as *const AnyClass) };
            if actual.superclass().map(|p| p as *const AnyClass as usize)
                != Some(self.0.parent.class.address())
                || metadata::lifecycle(class)? != self.0.lifecycle
            {
                return Err("owned memory class/lifecycle changed".into());
            }
            if self.0.selected.len() != super::factory::MEMORY_SELECTORS.len()
                || self.0.before.len() != self.0.selected.len()
            {
                return Err("owned memory declaration roster differs".into());
            }
            for (((selector, selected), (before_selector, before)), (root_selector, root)) in self
                .0
                .selected
                .iter()
                .zip(&self.0.before)
                .zip(&self.0.plan.roots)
            {
                if selector != before_selector || selector != root_selector {
                    return Err("owned memory selected declaration changed".into());
                }
                // The former method() result was used only for Eq, discarding
                // its new source capture. Keep both complete current raw reads;
                // the used class and constructor whole-life proofs stay fresh.
                metadata::verify_current_method(class, selector, false, selected)?;
                if selected.owner() != class.address()
                    || selected.implementation() != root.implementation()
                    || selected.encoding()
                        != if before.owner() == class.address() {
                            before.encoding()
                        } else {
                            root.encoding()
                        }
                    || selected.code() != root.code()
                {
                    return Err("owned memory selected declaration changed".into());
                }
                super::factory::verify_memory_abi(selector, selected)?;
            }
            for constructor in &self.0.constructors {
                constructor.verify_metadata_with_roster(roster)?;
            }
            Ok(())
        });
        if result.is_err() {
            self.0.retired.store(true, Ordering::Release);
            self.0.model.fault();
        }
        result
    }
}
unsafe extern "C" {
    fn sel_registerName(name: *const libc::c_char) -> *const std::ffi::c_void;
    fn class_addMethod(
        class: *const std::ffi::c_void,
        selector: *const std::ffi::c_void,
        imp: *const std::ffi::c_void,
        types: *const libc::c_char,
    ) -> u8;
    fn class_replaceMethod(
        class: *const AnyClass,
        selector: Sel,
        imp: Imp,
        types: *const libc::c_char,
    ) -> Option<Imp>;
    #[link_name = "method_getTypeEncoding"]
    fn raw_type_encoding(method: *const std::ffi::c_void) -> *const libc::c_char;
    fn class_createInstance(class: *const AnyClass, extra: usize) -> *mut AnyObject;
}
unsafe fn method_get_type_encoding(method: &Method) -> *const libc::c_char {
    unsafe { raw_type_encoding((method as *const Method).cast()) }
}
#[derive(Clone)]
struct InitializerOwner {
    lease: OwnedHandleLease,
    faulted: Arc<AtomicBool>,
}
static OWNERS: OnceLock<Mutex<BTreeMap<usize, InitializerOwner>>> = OnceLock::new();
// SDK proxy storage is zero-initialized and only the owned arena is observed.
// Runtime ancestor CXX dispatch must reach these exact proved data-only leaves.
unsafe extern "C" fn owned_construct(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    receiver
}
unsafe extern "C" fn owned_destruct(_: *mut AnyObject, _: Sel) {}
unsafe extern "C" fn owned_resolve(_: *mut AnyClass, _: Sel, _: Sel) -> objc2::runtime::Bool {
    objc2::runtime::Bool::NO
}
/// The integrator owns the single global allocator interpose. These routes
/// require actual selected-import/class-IMP proof, never a broad original
/// fallback. Only reviewed per-class grants may reach an owned factory.
pub const CONSTRUCTOR_ROUTES: &[&str] = &[
    "objc_alloc",
    "objc_alloc_init",
    "objc_allocWithZone",
    "objc_opt_new",
    "+new",
    "+alloc",
    "+allocWithZone:",
    "+initialize",
    "-init",
    ".cxx_construct per ancestor; raw class_createInstance is not a malloc-only proof",
    ".cxx_destruct per ancestor; NSObject dealloc/object_dispose are not destructor bypass proofs",
    "-dealloc / selected super-dealloc and ARC init-failure paths",
    "NSWindow.initWithContentRect:styleMask:backing:defer:",
    "NSWindow.initWithContentRect:styleMask:backing:defer:screen:",
    "NSPanel/native define_class inherited allocation + Rust ivars-before-super-init",
    "framework +load/C initializer ordering (bootstrap owns proof)",
];
unsafe extern "C" fn initialize(class: *mut AnyObject, _: Sel) {
    // Missing/retired context never invokes a previous initializer. The caller
    // cannot allocate through ClassGuard without a live, verified capability.
    let owner = OWNERS
        .get()
        .and_then(|owners| owners.lock().ok()?.get(&(class as usize)).cloned());
    let Some(owner) = owner else { return };
    if owner.faulted.load(Ordering::Acquire) {
        return;
    }
    let admitted =
        catch_unwind(AssertUnwindSafe(|| owner.lease.before_effect().is_ok())).unwrap_or(false);
    if !admitted {
        owner.faulted.store(true, Ordering::Release);
    }
}
unsafe extern "C" fn deny_unknown(_receiver: *mut AnyObject, _selector: Sel) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    unsafe { crate::workspace::objc::report_safari_discovery_selector(_receiver, _selector); }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if crate::sdk::system_providers::monitor_timeout::assembly_probe_active() && !_receiver.is_null() {
        if let Some(class) = unsafe { objc2::ffi::object_getClass(_receiver).as_ref() } {
            crate::sdk::system_providers::monitor_timeout::report_assembly_selector_refusal(class.name(), _selector.name());
        }
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if crate::sdk::private_network::reference_website_assembly_active() && !_receiver.is_null() {
        if let Some(class) = unsafe { objc2::ffi::object_getClass(_receiver).as_ref() } {
            crate::sdk::private_network::report_reference_website_selector_refusal(class.name(), _selector.name());
        }
    }
    unsafe { libc::_exit(78) }
}
pub struct ClassGuard {
    pub initializers: Vec<ConstructorBinding>,
    lifecycle: Vec<ConstructorBinding>,
    preserved_appkit_lifecycle: Vec<super::receiver::LifecycleBinding>,
    lease: OwnedHandleLease,
    families: BTreeSet<usize>,
    created: Mutex<BTreeMap<usize, Vec<(Sel, usize)>>>,
    additions: Mutex<Vec<OwnedStringBytesAddition>>,
    task_wait_addition: Mutex<Option<OwnedTaskWaitAddition>>,
}
/// An SDK-owned declaration, deliberately not an original-method replacement.
/// This one public initializer has no method slot on the loaded public class.
#[derive(Clone, Debug)]
pub struct StringBytesDeclaration {
    base: metadata::ClassHandle,
    sources: [(std::path::PathBuf, [u8; 32]); 2],
    replacement: metadata::CodeIdentity,
    implementation: usize,
    retired: Arc<AtomicBool>,
}
pub(super) type StringBytesInitializer = unsafe extern "C" fn(
    *mut AnyObject,
    Sel,
    *const std::ffi::c_void,
    usize,
    usize,
) -> *mut AnyObject;
impl StringBytesDeclaration {
    pub(super) const SELECTOR: &'static CStr = c"initWithBytes:length:encoding:";
    fn compiled_sources() -> Result<[(std::path::PathBuf, [u8; 32], u32); 2]> {
        // This literal is emitted by the source-owned producer only after the
        // actual same-build Foundation dep-info/extern join. Never runtime env.
        let literal = option_env!("RUYCAST_F1_NSSTRING_DECLARATION_PINS")
            .ok_or("NSString compiler declaration pins absent")?;
        if literal.len() > 16384 { return Err("NSString compiler pin bound".into()); }
        let rows: [(std::path::PathBuf, String, u32); 2] =
            serde_json::from_str(literal).map_err(|_| "NSString compiler pin schema")?;
        let mut result = Vec::new();
        for (index, (path, hash, owner)) in rows.into_iter().enumerate() {
            if !path.is_absolute() || hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || (index == 0 && owner != 0)
                || path.file_name() != Some(std::ffi::OsStr::new(if index == 0 { "NSString.h" } else { "NSString.rs" }))
            { return Err("NSString compiler pin fields".into()); }
            let mut bytes = [0; 32];
            for (at, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hash[at * 2..at * 2 + 2], 16)
                    .map_err(|_| "NSString compiler digest")?;
            }
            result.push((path, bytes, owner));
        }
        result.try_into().map_err(|_| "NSString compiler pin count".into())
    }
    /// Observation only. A Gate must independently match its compiled input
    /// pins and private grant; this never supplies constructor/source authority.
    pub fn compiled_source_pins() -> Result<[(std::path::PathBuf, [u8; 32], u32); 2]> {
        Self::compiled_sources()
    }
    fn read_source(path: &std::path::Path, expected: &[u8; 32], owner: u32) -> Result<Vec<u8>> {
        use std::{io::Read, os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt}}};
        use sha2::{Digest, Sha256};
        if std::fs::canonicalize(path).map_err(|_| "NSString source canonicalization")? != path {
            return Err("NSString source path alias".into());
        }
        let file = std::fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK).open(path)
            .map_err(|_| "NSString source readcap open")?;
        let before = file.metadata().map_err(|_| "NSString source fstat")?;
        let identity = |m: &std::fs::Metadata| (m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
        if !before.is_file() || before.nlink() != 1 || before.uid() != owner
            || before.mode() & 0o022 != 0 || before.len() > 2 * 1024 * 1024
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) } & libc::O_ACCMODE != libc::O_RDONLY
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC == 0
        { return Err("NSString source readcap identity/access".into()); }
        let mut bytes = Vec::new();
        (&file).take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)
            .map_err(|_| "NSString source bounded read")?;
        let after = file.metadata().map_err(|_| "NSString source refstat")?;
        let leaf = std::fs::symlink_metadata(path).map_err(|_| "NSString source leaf restat")?;
        let actual: [u8;32] = Sha256::digest(&bytes).into();
        if bytes.len() > 2 * 1024 * 1024 || identity(&before) != identity(&after)
            || identity(&after) != identity(&leaf) || actual != *expected
        { return Err("NSString source bytes/identity changed".into()); }
        Ok(bytes)
    }
    pub(super) fn capture(implementation: StringBytesInitializer) -> Result<Self> {
        let declarations = [
            "- (nullable instancetype)initWithBytes:(const void *)bytes length:(NSUInteger)len encoding:(NSStringEncoding)encoding;",
            "pub unsafe fn initWithBytes_length_encoding(",
        ];
        let mut sources = Vec::new();
        for (index, (path, hash, owner)) in Self::compiled_sources()?.into_iter().enumerate() {
            let bytes = Self::read_source(&path, &hash, owner)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| "NSString declaration UTF8")?;
            if !text.contains(declarations[index])
                || (index == 0 && !text.contains("typedef NSUInteger NSStringEncoding;"))
                || (index == 1 && !text.contains("this: Allocated<Self>,\n            bytes: NonNull<c_void>,\n            len: NSUInteger,\n            encoding: NSStringEncoding,\n        ) -> Option<Retained<Self>>;")) {
                return Err("NSString public declaration differs".into());
            }
            sources.push((path, hash));
        }
        let result = Self {
            base: metadata::loaded_class(c"NSString")?,
            sources: sources
                .try_into()
                .map_err(|_| "NSString declaration source count")?,
            replacement: metadata::code_identity(implementation as *const () as usize)?,
            implementation: implementation as *const () as usize,
            retired: Arc::new(AtomicBool::new(false)),
        };
        result.verify()?;
        Ok(result)
    }
    /// Observation only; Gate must independently compare these to compile pins.
    pub fn source_pins(&self) -> &[(std::path::PathBuf, [u8; 32]); 2] {
        &self.sources
    }
    pub fn base_class(&self) -> metadata::ClassHandle {
        self.base
    }
    pub fn implementation_code(&self) -> &metadata::CodeIdentity {
        &self.replacement
    }
    pub(super) fn signature() -> Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
        use objc2::runtime::MethodImplementation;
        let mut signature = vec![
            <StringBytesInitializer as MethodImplementation>::Return::ENCODING_RETURN,
            Encoding::Object,
            Sel::ENCODING,
        ];
        signature.extend_from_slice(
            <StringBytesInitializer as MethodImplementation>::Arguments::ENCODINGS,
        );
        signature
    }
    pub(super) fn verify(&self) -> Result<()> {
        metadata::with_verification_roster(|roster| self.verify_with_roster(roster))
    }
    fn verify_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        let result = (|| {
            if self.retired.load(Ordering::Acquire) {
                return Err("owned NSString declaration retired".into());
            }
            if roster.loaded_class(c"NSString")? != self.base
                || metadata::method(self.base, Self::SELECTOR, false)?.is_some()
            {
                return Err("owned NSString declaration base absence changed".into());
            }
            for ((path, hash), (compiled_path, compiled_hash, owner)) in self.sources.iter().zip(Self::compiled_sources()?) {
                if *path != compiled_path || *hash != compiled_hash { return Err("NSString declaration compiler binding changed".into()); }
                Self::read_source(path, hash, owner)?;
            }
            if metadata::code_identity(self.implementation)? != self.replacement {
                return Err("owned NSString declaration code changed".into());
            }
            Ok(())
        })();
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        result
    }
}
#[derive(Clone)]
pub(super) struct OwnedStringBytesAddition {
    declaration: StringBytesDeclaration,
    pub(super) class: &'static CStr,
    selected: metadata::MethodSnapshot,
    lifecycle: metadata::LifecycleSnapshot,
    parent_lifecycle: metadata::LifecycleSnapshot,
    constructors: Vec<ConstructorBinding>,
    retired: Arc<AtomicBool>,
}
impl OwnedStringBytesAddition {
    pub(super) fn implementation(&self) -> usize {
        self.selected.implementation()
    }
    pub(super) fn verify(&self) -> Result<()> {
        metadata::with_verification_roster(|roster| self.verify_with_roster(roster))
    }
    fn verify_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        self.verify_metadata_with_roster(roster)?;
        // Business admission may fail while physical metadata remains valid.
        // Never poison the cleanup proof because a guardian callback retired.
        for binding in &self.constructors {
            binding.verify_with_roster(roster)?;
        }
        Ok(())
    }
    pub(super) fn verify_metadata(&self) -> Result<()> {
        metadata::with_verification_roster(|roster| self.verify_metadata_with_roster(roster))
    }
    fn verify_metadata_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        let result = (|| {
            if self.retired.load(Ordering::Acquire) {
                return Err("owned NSString addition retired".into());
            }
            self.declaration.verify_with_roster(roster)?;
            let class = roster.loaded_class(self.class)?;
            let current = metadata::method(class, StringBytesDeclaration::SELECTOR, false)?;
            if current.as_ref() != Some(&self.selected)
                || self.selected.owner() != class.address()
                || metadata::lifecycle(class)? != self.lifecycle
                || metadata::lifecycle(self.declaration.base)? != self.parent_lifecycle
            {
                return Err("owned NSString addition metadata changed".into());
            }
            for binding in &self.constructors {
                binding.verify_metadata_with_roster(roster)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        result
    }
}
/// One exact public declaration, never an original-method replacement.
#[derive(Clone,Debug)]
pub struct TaskWaitDeclaration {
    base:metadata::ClassHandle,
    sources:[(std::path::PathBuf,[u8;32],u32);2],
    implementation:usize,
    code:metadata::CodeIdentity,
    retired:Arc<AtomicBool>,
}
pub(super) type TaskWaitImplementation=unsafe extern "C" fn(*mut AnyObject,Sel);
pub(super) type HandleInitializer=unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject;
impl TaskWaitDeclaration {
    pub(super) const SELECTOR:&'static CStr=c"waitUntilExit";
    #[cfg(rcf1_task_wait_declaration)]
    fn compiled_bytes()->Result<[(&'static str,&'static [u8]);2]> {
        Ok([(env!("RUYCAST_F1_TASK_WAIT_HEADER"),include_bytes!(env!("RUYCAST_F1_TASK_WAIT_HEADER"))),
            (env!("RUYCAST_F1_TASK_WAIT_BINDING"),include_bytes!(env!("RUYCAST_F1_TASK_WAIT_BINDING")))])
    }
    #[cfg(not(rcf1_task_wait_declaration))]
    fn compiled_bytes()->Result<[(&'static str,&'static [u8]);2]> {
        Err("Task wait declaration compiler inputs absent".into())
    }
    /// Observation only. A producer must join both includes to this SDK child.
    pub fn compiled_source_pins()->Result<[(std::path::PathBuf,[u8;32],u32);2]> {
        use sha2::{Digest,Sha256};
        let literal=option_env!("RUYCAST_F1_TASK_WAIT_DECLARATION_PINS")
            .ok_or("Task wait declaration compiler pins absent")?;
        if literal.len()>16384 { return Err("Task wait compiler literal bound".into()); }
        let rows:[(std::path::PathBuf,String,u32);2]=serde_json::from_str(literal)
            .map_err(|_|"Task wait compiler literal schema")?;
        let compiled=Self::compiled_bytes()?;
        let mut result=Vec::new();
        for (index,(path,hash,uid)) in rows.into_iter().enumerate() {
            if !path.is_absolute()||hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
                ||uid!=if index==0 {0}else{unsafe {libc::getuid()}}
                ||path.file_name()!=Some(std::ffi::OsStr::new(if index==0 {"NSTask.h"}else{"NSTask.rs"}))
                ||path.as_os_str()!=std::ffi::OsStr::new(compiled[index].0) {
                return Err("Task wait declaration compiler fields".into());
            }
            let mut digest=[0u8;32];
            for (at,byte) in digest.iter_mut().enumerate() {
                *byte=u8::from_str_radix(&hash[at*2..at*2+2],16).map_err(|_|"Task wait compiler digest")?;
            }
            let actual:[u8;32]=Sha256::digest(compiled[index].1).into();
            if actual!=digest { return Err("Task wait compiled declaration bytes differ".into()); }
            result.push((path,digest,uid));
        }
        result.try_into().map_err(|_|"Task wait compiler input count".into())
    }
    pub(super) fn signature()->Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode,EncodeArguments,EncodeReturn,Encoding};
        use objc2::runtime::MethodImplementation;
        let mut signature=vec![<TaskWaitImplementation as MethodImplementation>::Return::ENCODING_RETURN,
            Encoding::Object,Sel::ENCODING];
        signature.extend_from_slice(<TaskWaitImplementation as MethodImplementation>::Arguments::ENCODINGS);
        signature
    }
    pub(super) fn capture(implementation:TaskWaitImplementation)->Result<Self> {
        let sources=Self::compiled_source_pins()?;
        for (index,(path,hash,uid)) in sources.iter().enumerate() {
            let bytes=StringBytesDeclaration::read_source(path,hash,*uid)?;
            let text=std::str::from_utf8(&bytes).map_err(|_|"Task wait declaration UTF8")?;
            if (index==0&&!text.contains("- (void)waitUntilExit;"))
                ||(index==1&&(!text.contains("#[unsafe(method(waitUntilExit))]")
                    ||!text.contains("pub fn waitUntilExit(&self);"))) {
                return Err("Task wait public declaration differs".into());
            }
        }
        let result=Self {base:metadata::loaded_class(c"NSTask")?,sources,
            implementation:implementation as *const () as usize,
            code:metadata::code_identity(implementation as *const () as usize)?,retired:Arc::new(AtomicBool::new(false))};
        result.verify()?;
        Ok(result)
    }
    pub(super) fn verify(&self)->Result<()> {
        metadata::with_verification_roster(|roster|self.verify_with_roster(roster))
    }
    fn verify_with_roster(&self,roster:&metadata::VerificationRoster)->Result<()> {
        let result=(||{
            if self.retired.load(Ordering::Acquire)||roster.loaded_class(c"NSTask")?!=self.base
                ||metadata::method(self.base,Self::SELECTOR,false)?.is_some() {
                return Err("Task wait original absence changed/retired".into());
            }
            if self.sources!=Self::compiled_source_pins()? { return Err("Task wait compiler inputs changed".into()); }
            for (path,hash,uid) in &self.sources {StringBytesDeclaration::read_source(path,hash,*uid)?;}
            if metadata::code_identity(self.implementation)?!=self.code { return Err("Task wait implementation code changed".into()); }
            Ok(())
        })();
        if result.is_err() {self.retired.store(true,Ordering::Release);}
        result
    }
}
#[derive(Clone)]
pub(super) struct OwnedTaskWaitAddition {
    declaration:TaskWaitDeclaration,
    pub(super) class:&'static CStr,
    selected:metadata::MethodSnapshot,
    lifecycle:metadata::LifecycleSnapshot,
    parent_lifecycle:metadata::LifecycleSnapshot,
    constructors:Vec<ConstructorBinding>,
    retired:Arc<AtomicBool>,
}
impl OwnedTaskWaitAddition {
    pub(super) fn implementation(&self)->usize {self.selected.implementation()}
    pub(super) fn verify(&self)->Result<()> {
        metadata::with_verification_roster(|roster|self.verify_with_roster(roster))
    }
    fn verify_with_roster(&self,roster:&metadata::VerificationRoster)->Result<()> {
        self.verify_metadata_with_roster(roster)?;
        for binding in &self.constructors {binding.verify_with_roster(roster)?;}
        Ok(())
    }
    pub(super) fn verify_metadata(&self)->Result<()> {
        metadata::with_verification_roster(|roster|self.verify_metadata_with_roster(roster))
    }
    fn verify_metadata_with_roster(&self,roster:&metadata::VerificationRoster)->Result<()> {
        let result=(||{
            if self.retired.load(Ordering::Acquire) {return Err("Task wait owned addition retired".into());}
            self.declaration.verify_with_roster(roster)?;
            let class=roster.loaded_class(self.class)?;
            if metadata::method(class,TaskWaitDeclaration::SELECTOR,false)?.as_ref()!=Some(&self.selected)
                ||self.selected.owner()!=class.address()||metadata::lifecycle(class)?!=self.lifecycle
                ||metadata::lifecycle(self.declaration.base)?!=self.parent_lifecycle {
                return Err("Task wait owned addition metadata changed".into());
            }
            for binding in &self.constructors {binding.verify_metadata_with_roster(roster)?;}
            Ok(())
        })();
        if result.is_err() {self.retired.store(true,Ordering::Release);}
        result
    }
}
impl ClassGuard {
    /// Bind only NSFileHandle's observed inherited NSObject init to its fresh
    /// owned Handle child. This shadows a real nonzero original; it neither
    /// copies arbitrary NSObject leaves nor invokes that original initializer.
    /// # Safety
    /// Exclusive qualified preentry and the actual typed Handle implementation.
    pub(super) unsafe fn overlay_handle_root_initializer(
        &self,class:&'static AnyClass,implementation:HandleInitializer,
    )->Result<ObjcReplacement> {
        self.verify()?;
        let address=class as *const AnyClass as usize;
        let parent=metadata::loaded_class(c"NSFileHandle")?;
        let root=metadata::loaded_class(c"NSObject")?;
        let parent_class=unsafe {&*(parent.address() as *const AnyClass)};
        if class.name()!=c"RuycastF1OwnedHandleSource"||class.superclass()!=Some(parent_class)
            ||parent_class.superclass()!=Some(NSObject::class())
            ||!self.families.contains(&parent.address())
            ||!self.created.lock().map_err(|_|"created classes poisoned")?.contains_key(&address)
            ||class.instance_methods().iter().any(|method|method.name()==objc2::sel!(init)) {
            return Err("Handle root init outside fresh qualified direct child".into());
        }
        let handle=metadata::registered_class(address)?;
        let baseline=metadata::method(handle,c"init",false)?.ok_or("Handle inherited init absent")?;
        let parent_init=metadata::method(parent,c"init",false)?.ok_or("Handle parent init absent")?;
        let root_init=metadata::method(root,c"init",false)?.ok_or("NSObject init absent")?;
        use objc2::encode::{Encode,EncodeArguments,EncodeReturn,Encoding};
        use objc2::runtime::MethodImplementation;
        let mut signature=vec![<HandleInitializer as MethodImplementation>::Return::ENCODING_RETURN,Encoding::Object,Sel::ENCODING];
        signature.extend_from_slice(<HandleInitializer as MethodImplementation>::Arguments::ENCODINGS);
        verify_typed_abi(&baseline,&signature)?;
        verify_typed_abi(&parent_init,&signature)?;
        verify_typed_abi(&root_init,&signature)?;
        if baseline.owner()!=root.address()||parent_init.owner()!=root.address()
            ||root_init.owner()!=root.address()||root_init.implementation()==0
            ||root_init.code().image()!=b"/usr/lib/libobjc.A.dylib"
            ||[&baseline,&parent_init].iter().any(|snapshot|
                snapshot.method_address()!=root_init.method_address()
                ||snapshot.implementation()!=root_init.implementation()||snapshot.code()!=root_init.code()
                ||snapshot.encoding()!=root_init.encoding()) {
            return Err("Handle init is not the exact inherited NSObject method".into());
        }
        let replacement=implementation as *const () as usize;
        let replacement_code=metadata::code_identity(replacement)?;
        if replacement==baseline.implementation() {return Err("Handle initializer did not replace original".into());}
        let lifecycle_before=metadata::lifecycle(handle)?;
        let encoding=std::ffi::CString::new(baseline.encoding()).map_err(|_|"Handle init encoding NUL")?;
        // No dynamic guard or unknown callback after these exact pure checks.
        if metadata::method(handle,c"init",false)?.as_ref()!=Some(&baseline)
            ||metadata::method(parent,c"init",false)?.as_ref()!=Some(&parent_init)
            ||metadata::method(root,c"init",false)?.as_ref()!=Some(&root_init) {
            return Err("Handle root initializer changed before shadow".into());
        }
        let mut created=self.created.lock().map_err(|_|"created classes poisoned")?;
        let leaves=created.get_mut(&address).ok_or("Handle child disappeared")?;
        if leaves.iter().any(|(selector,_)|*selector==objc2::sel!(init)) {
            return Err("Handle root init already materialized".into());
        }
        if unsafe {class_addMethod((class as *const AnyClass).cast(),sel_registerName(c"init".as_ptr()),
            (implementation as *const ()).cast(),encoding.as_ptr())}==0 {
            return Err("Handle root init local shadow add failed".into());
        }
        let selected=metadata::method(handle,c"init",false)?.ok_or("Handle local init disappeared")?;
        verify_typed_abi(&selected,&signature)?;
        if selected.owner()!=address||selected.implementation()!=replacement||selected.code()!=&replacement_code {
            return Err("Handle root init selected identity differs".into());
        }
        let provenance=metadata::ReplacementProvenance::bind(baseline.clone(),baseline.clone(),selected,
            lifecycle_before,false,None)?;
        leaves.push((objc2::sel!(init),replacement));
        Ok(ObjcReplacement{class:class.name(),selector:c"init",class_method:false,
            original:baseline.implementation(),replacement,provenance})
    }
    /// # Safety
    /// Exclusive preentry; only this guard's fresh direct owned NSTask child.
    pub(super) unsafe fn add_task_wait(
        &self,class:&'static AnyClass,declaration:&TaskWaitDeclaration,implementation:TaskWaitImplementation,
    )->Result<OwnedTaskWaitAddition> {
        let result=unsafe {self.add_task_wait_checked(class,declaration,implementation)};
        if result.is_err() {declaration.retired.store(true,Ordering::Release);}
        result
    }
    unsafe fn add_task_wait_checked(
        &self,class:&'static AnyClass,declaration:&TaskWaitDeclaration,implementation:TaskWaitImplementation,
    )->Result<OwnedTaskWaitAddition> {
        self.verify()?;
        declaration.verify()?;
        let address=class as *const AnyClass as usize;
        if class.name()!=c"RuycastF1OwnedTaskSource"
            ||class.superclass().map(|p|p as *const AnyClass as usize)!=Some(declaration.base.address())
            ||!self.created.lock().map_err(|_|"created classes poisoned")?.contains_key(&address)
            ||self.task_wait_addition.lock().map_err(|_|"Task wait addition poisoned")?.is_some()
            ||implementation as *const () as usize!=declaration.implementation
            ||raw(class,Sel::register(TaskWaitDeclaration::SELECTOR),false)?.is_some() {
            return Err("Task wait addition outside fresh direct owned child".into());
        }
        let signature=TaskWaitDeclaration::signature();
        let encoding=std::ffi::CString::new(signature.iter().map(ToString::to_string).collect::<String>())
            .map_err(|_|"Task wait encoding NUL")?;
        let lifecycle=metadata::lifecycle(metadata::registered_class(address)?)?;
        let parent_lifecycle=metadata::lifecycle(declaration.base)?;
        // No dynamic guard occurs between this final absence/code/source check and write.
        declaration.verify()?;
        if unsafe {class_addMethod((class as *const AnyClass).cast(),sel_registerName(TaskWaitDeclaration::SELECTOR.as_ptr()),
            (implementation as *const ()).cast(),encoding.as_ptr())}==0 {
            declaration.retired.store(true,Ordering::Release);
            return Err("Task wait owned declaration add failed".into());
        }
        let selected=raw(class,Sel::register(TaskWaitDeclaration::SELECTOR),false)?.ok_or("Task wait local method absent")?;
        verify_typed_abi(&selected,&signature)?;
        if selected.owner()!=address||selected.implementation()!=declaration.implementation||selected.code()!=&declaration.code {
            declaration.retired.store(true,Ordering::Release);
            return Err("Task wait selected local implementation differs".into());
        }
        let mut names=BTreeSet::new();let mut ancestor=class.superclass();
        while let Some(parent)=ancestor {if parent==NSObject::class(){break;}names.insert(parent.name());ancestor=parent.superclass();}
        let proof=OwnedTaskWaitAddition {declaration:declaration.clone(),class:class.name(),selected,lifecycle,parent_lifecycle,
            constructors:self.initializers.iter().chain(&self.lifecycle).filter(|b|names.contains(b.class)).cloned().collect(),
            retired:Arc::new(AtomicBool::new(false))};
        proof.verify()?;
        self.created.lock().map_err(|_|"created classes poisoned")?.get_mut(&address)
            .ok_or("Task proxy disappeared")?.push((Sel::register(TaskWaitDeclaration::SELECTOR),proof.implementation()));
        *self.task_wait_addition.lock().map_err(|_|"Task wait addition poisoned")?=Some(proof.clone());
        Ok(proof)
    }
    /// Only the fresh direct owned NSString child may receive this declaration.
    /// No absent original is manufactured or exported as ReplacementProvenance.
    pub(super) unsafe fn add_string_bytes_initializer(
        &self,
        class: &'static AnyClass,
        declaration: &StringBytesDeclaration,
        implementation: StringBytesInitializer,
    ) -> Result<OwnedStringBytesAddition> {
        self.verify()?;
        declaration.verify()?; // After the last dynamic guard, before this write.
        let address = class as *const AnyClass as usize;
        if class.superclass().map(|p| p as *const AnyClass as usize)
            != Some(declaration.base.address())
            || class.name() != c"RuycastF1OwnedStringSource"
            || !self
                .created
                .lock()
                .map_err(|_| "created classes poisoned")?
                .contains_key(&address)
            || implementation as *const () as usize != declaration.implementation
            || raw(
                class,
                Sel::register(StringBytesDeclaration::SELECTOR),
                false,
            )?
            .is_some()
        {
            return Err("owned NSString addition outside fresh direct proxy".into());
        }
        let signature = StringBytesDeclaration::signature();
        let encoding = std::ffi::CString::new(
            signature
                .iter()
                .map(ToString::to_string)
                .collect::<String>(),
        )
        .map_err(|_| "owned declaration encoding NUL")?;
        let before = metadata::lifecycle(metadata::registered_class(address)?)?;
        if unsafe {
            class_addMethod(
                (class as *const AnyClass).cast(),
                sel_registerName(StringBytesDeclaration::SELECTOR.as_ptr()),
                (implementation as *const ()).cast(),
                encoding.as_ptr(),
            )
        } == 0
        {
            return Err("owned NSString declaration add failed".into());
        }
        let selected = raw(
            class,
            Sel::register(StringBytesDeclaration::SELECTOR),
            false,
        )?
        .ok_or("owned declaration absent")?;
        verify_typed_abi(&selected, &signature)?;
        if selected.implementation() != declaration.implementation
            || selected.code() != &declaration.replacement
        {
            declaration.retired.store(true, Ordering::Release);
            return Err("owned NSString declaration selected code differs".into());
        }
        let proof = OwnedStringBytesAddition {
            declaration: declaration.clone(),
            class: class.name(),
            selected,
            lifecycle: before,
            parent_lifecycle: metadata::lifecycle(declaration.base)?,
            constructors: self
                .initializers
                .iter()
                .chain(&self.lifecycle)
                .cloned()
                .collect(),
            retired: Arc::new(AtomicBool::new(false)),
        };
        proof.verify()?;
        self.created
            .lock()
            .map_err(|_| "created classes poisoned")?
            .get_mut(&address)
            .ok_or("owned proxy disappeared")?
            .push((
                Sel::register(StringBytesDeclaration::SELECTOR),
                proof.implementation(),
            ));
        self.additions
            .lock()
            .map_err(|_| "owned addition proofs poisoned")?
            .push(proof.clone());
        Ok(proof)
    }
    /// Bind an exact compiled Rust method signature to an ordinary SDK leaf.
    /// Lifecycle mutations must use the atomic family transaction instead.
    /// # Safety
    /// Caller proves this exact source selector is required and replacement is
    /// its owned implementation. The complete preentry family is exclusive.
    pub unsafe fn overlay_bound(
        &self,
        class: &'static AnyClass,
        selector: &'static CStr,
        class_method: bool,
        replacement: usize,
        signature: &[objc2::encode::Encoding],
    ) -> Result<ObjcReplacement> {
        self.verify()?;
        if [
            c"initialize",
            c"resolveClassMethod:",
            c"resolveInstanceMethod:",
            c".cxx_construct",
            c".cxx_destruct",
            c"dealloc",
        ]
        .contains(&selector)
        {
            return Err("ordinary overlay cannot mutate lifecycle".into());
        }
        if class_method {
            if !self.families.contains(&(class as *const AnyClass as usize)) {
                return Err("class overlay outside qualified SDK family".into());
            }
        } else if !self
            .created
            .lock()
            .map_err(|_| "created class registry poisoned")?
            .contains_key(&(class as *const AnyClass as usize))
        {
            return Err("instance overlay outside owned SDK proxy".into());
        }
        let handle = metadata::registered_class(class as *const AnyClass as usize)?;
        let baseline =
            metadata::method(handle, selector, class_method)?.ok_or("source SDK leaf missing")?;
        verify_typed_abi(&baseline, signature)?;
        let before_lifecycle = metadata::lifecycle(handle)?;
        let encoding = std::ffi::CString::new(baseline.encoding())
            .map_err(|_| "SDK leaf encoding contains NUL")?;
        if class_method {
            let target = unsafe { &*(class as *const AnyClass as *const AnyObject) }.class();
            unsafe {
                class_replaceMethod(
                    target,
                    Sel::register(selector),
                    std::mem::transmute::<usize, Imp>(replacement),
                    encoding.as_ptr(),
                )
            };
        } else {
            unsafe {
                self.overlay(
                    class,
                    selector,
                    baseline.implementation(),
                    replacement,
                    &encoding,
                )
            }?;
        }
        let selected =
            metadata::method(handle, selector, class_method)?.ok_or("selected SDK leaf missing")?;
        let provenance = metadata::ReplacementProvenance::bind(
            baseline.clone(),
            baseline.clone(),
            selected,
            before_lifecycle,
            class_method,
            None,
        )?;
        Ok(ObjcReplacement {
            class: class.name(),
            selector,
            class_method,
            original: baseline.implementation(),
            replacement,
            provenance,
        })
    }
    /// Add an inherited, typed window operation on the exact original Swift
    /// receiver. The selector must map to an existing private window operation.
    /// # Safety
    /// The caller owns exclusive preentry before any instance is allocated.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) unsafe fn shadow_reference_panel_leaf(
        &self,
        admission: &crate::sdk::core_preentry::ReferenceAdmission,
        class: &'static AnyClass,
        selector: &'static CStr,
    ) -> Result<ObjcReplacement> {
        self.verify()?;
        match class.name().to_bytes() {
            b"RuycastF1Reference.PalettePanel" => admission.verify_reference_palette_panel_source()?,
            b"RuycastF1Reference.SnippetChoicePanel" if super::objc::REFERENCE_SNIPPET_WINDOW_SELECTORS.contains(&selector)
                || super::objc::REFERENCE_SNIPPET_APPEARANCE_SELECTORS.iter().any(|&(_, allowed)| selector == allowed) => {}
            _ => return Err("Reference window selector outside the original panel contract".into()),
        }
        let (replacement, signature) = super::objc::reference_palette_window_leaf(selector)?;
        let handle = metadata::loaded_class(class.name())?;
        let panel = metadata::loaded_class(c"NSPanel")?;
        let window = metadata::loaded_class(c"NSWindow")?;
        if class as *const AnyClass as usize != handle.address()
            || class.superclass().map(|parent| parent as *const AnyClass as usize) != Some(panel.address())
            || !self.families.contains(&panel.address()) || !self.families.contains(&window.address())
        {
            return Err("Reference Palette window family differs".into());
        }
        let baseline = metadata::method(handle, selector, false)?.ok_or("inherited Palette method absent")?;
        if ![panel.address(), window.address()].contains(&baseline.owner())
            || baseline.implementation() == replacement
        {
            return Err("Reference Palette method is not an inherited window operation".into());
        }
        verify_typed_abi(&baseline, &signature)?;
        let code = metadata::code_identity(replacement)?;
        let lifecycle = metadata::lifecycle(handle)?;
        let encoding = std::ffi::CString::new(baseline.encoding()).map_err(|_| "Palette encoding contains NUL")?;
        if unsafe { class_addMethod((class as *const AnyClass).cast(), sel_registerName(selector.as_ptr()),
            replacement as *const _, encoding.as_ptr()) } == 0 {
            return Err("Reference Palette local method add failed".into());
        }
        let selected = metadata::method(handle, selector, false)?.ok_or("selected Palette method absent")?;
        if selected.implementation() != replacement || selected.code() != &code {
            return Err("Reference Palette selected implementation differs".into());
        }
        let original = baseline.implementation();
        let provenance = metadata::ReplacementProvenance::bind(
            baseline.clone(), baseline, selected, lifecycle, false, None,
        )?;
        Ok(ObjcReplacement { class: class.name(), selector, class_method: false, original, replacement, provenance })
    }

    pub fn verify(&self) -> Result<()> {
        self.lease.before_effect()?;
        let additions = self
            .additions
            .lock()
            .map_err(|_| "owned addition proofs poisoned")?
            .clone();
        let task_wait=self.task_wait_addition.lock().map_err(|_|"Task wait addition poisoned")?.clone();
        // All following calls are pure metadata/source verification. No lease
        // callback or class write may occur inside this borrowed roster scope.
        metadata::with_verification_roster(|roster| {
            if let Some(proof)=task_wait {proof.verify_with_roster(roster)?;}
            for proof in additions {
                proof.verify_with_roster(roster)?;
            }
            for row in &self.initializers {
                row.verify_with_roster(roster)?;
            }
            for row in &self.lifecycle {
                row.verify_with_roster(roster)?;
            }
            for row in &self.preserved_appkit_lifecycle {
                row.verify_with_roster(roster)?;
            }
            Ok(())
        })
    }
    pub fn lifecycle_bindings(&self) -> Vec<super::receiver::LifecycleBinding> {
        self.initializers
            .iter()
            .chain(&self.lifecycle)
            .cloned()
            .map(super::receiver::LifecycleBinding::constructor)
            .chain(self.preserved_appkit_lifecycle.iter().cloned())
            .collect()
    }
    /// A shared coordinator may select several sibling families. A receiver
    /// grant gets only its actual parent chain, retaining each binding's full
    /// original/inherited provenance and shared sticky retirement state.
    pub(super) fn lifecycle_bindings_for(
        &self,
        parent: &'static AnyClass,
    ) -> Result<Vec<super::receiver::LifecycleBinding>> {
        self.verify()?;
        if !self
            .families
            .contains(&(parent as *const AnyClass as usize))
        {
            return Err("unowned lifecycle projection parent".into());
        }
        let mut names = BTreeSet::new();
        let mut current = Some(parent);
        while let Some(class) = current {
            if class == NSObject::class() {
                break;
            }
            names.insert(class.name());
            current = class.superclass();
        }
        let selected = self
            .lifecycle_bindings()
            .into_iter()
            .filter(|binding| names.contains(binding.class))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("empty receiver lifecycle projection".into());
        }
        Ok(selected)
    }
    /// Select a complete SDK ancestor lifecycle in one metadata transaction per
    /// class. Every original method baseline is read before the first mutation.
    /// # Safety
    /// Classes belong to the exact preentry/source-qualified SDK image family;
    /// no original class message, instance or concurrent mutation may race this
    /// operation. Deallocator is the reviewed typed owned-arena cleanup route.
    pub unsafe fn select_sdk_family(
        classes: &[&'static AnyClass],
        lease: OwnedHandleLease,
        deallocator: unsafe extern "C" fn(*mut AnyObject, Sel),
    ) -> Result<Self> {
        unsafe { Self::select_sdk_family_checked(classes, lease, deallocator, || Ok(())) }
    }
    /// Preserve the caller's source pins across the dynamic lease callback.
    /// # Safety
    /// Same exclusive source authority as select_sdk_family. The validator
    /// checks exact previously qualified metadata and must not mutate classes.
    pub(super) unsafe fn select_sdk_family_checked(
        classes: &[&'static AnyClass],
        lease: OwnedHandleLease,
        deallocator: unsafe extern "C" fn(*mut AnyObject, Sel),
        prewrite: impl FnOnce() -> Result<()>,
    ) -> Result<Self> {
        unsafe {
            Self::select_sdk_family_checked_memory(
                classes,
                lease,
                |_| Ok(deallocator),
                prewrite,
                None,
                || Ok(()),
                false,
            )
        }
    }
    pub(super) unsafe fn select_foundation_memory_family_checked(
        classes: &[&'static AnyClass],
        lease: OwnedHandleLease,
        deallocator: impl Fn(&AnyClass) -> Result<unsafe extern "C" fn(*mut AnyObject, Sel)>,
        memory: &super::factory::FoundationMemoryPlan,
        prewrite: impl FnOnce() -> Result<()>,
        before_write: impl Fn() -> Result<()>,
    ) -> Result<Self> {
        unsafe {
            Self::select_sdk_family_checked_memory(
                classes,
                lease,
                deallocator,
                prewrite,
                Some(memory),
                before_write,
                false,
            )
        }
    }
    /// One exclusive family claim, with a fixed handler for each actual class.
    /// Missing ancestors are refused before any lifecycle replacement.
    pub(super) unsafe fn select_sdk_joint_family_checked(
        classes: &[&'static AnyClass],
        lease: OwnedHandleLease,
        handlers: &[(&'static AnyClass, unsafe extern "C" fn(*mut AnyObject, Sel))],
        prewrite: impl FnOnce() -> Result<()>,
        preserve_appkit_lifecycle: bool,
    ) -> Result<Self> {
        unsafe { Self::select_sdk_family_checked_memory(classes, lease, |class| {
            let mut matches=handlers.iter().filter(|(candidate,_)|*candidate==class);
            let handler=matches.next().ok_or("joint lifecycle handler absent")?.1;
            if matches.next().is_some() { return Err("joint lifecycle handler duplicated".into()); }
            Ok(handler)
        }, prewrite, None, || Ok(()), preserve_appkit_lifecycle) }
    }
    unsafe fn select_sdk_family_checked_memory(
        classes: &[&'static AnyClass],
        lease: OwnedHandleLease,
        deallocator: impl Fn(&AnyClass) -> Result<unsafe extern "C" fn(*mut AnyObject, Sel)>,
        prewrite: impl FnOnce() -> Result<()>,
        memory: Option<&super::factory::FoundationMemoryPlan>,
        before_write: impl Fn() -> Result<()>,
        preserve_appkit_lifecycle: bool,
    ) -> Result<Self> {
        lease.before_effect()?;
        prewrite()?;
        before_write()?;
        if let Some(memory) = memory {
            memory.verify_current()?;
        }
        if classes.is_empty() {
            return Err("empty SDK lifecycle family".into());
        }
        let mut family = BTreeMap::new();
        for class in classes {
            let mut current = Some(*class);
            while let Some(class) = current {
                if class == NSObject::class() {
                    break;
                }
                family.insert(class as *const AnyClass as usize, class);
                current = class.superclass();
            }
        }
        let mut plan = vec![];
        let mut preserved_appkit_lifecycle = Vec::new();
        for class in family.values() {
            let deallocator=deallocator(class)?;
            let shared_view = preserve_appkit_lifecycle
                && [c"NSResponder", c"NSView", c"NSControl", c"NSButton", c"NSStatusBarButton", c"NSEvent"]
                    .contains(&class.name());
            let initializer = raw(class, objc2::sel!(initialize), true)?
                .ok_or("SDK initializer metadata missing")?;
            if super::simple_abi(&initializer)? != b"v@:" {
                return Err("SDK initializer ABI differs".into());
            }
            let mut leaves = Vec::new();
            if shared_view {
                preserved_appkit_lifecycle.push(super::receiver::LifecycleBinding::preserve_appkit_lifecycle(
                    class, c"initialize", initializer,
                )?);
            } else {
                leaves.push((c"initialize", true, initializer, initialize as *const () as usize));
            }
            for selector in [c"resolveClassMethod:", c"resolveInstanceMethod:"] {
                let snapshot = raw(class, Sel::register(selector), true)?
                    .ok_or("SDK resolver metadata missing")?;
                use objc2::encode::Encode;
                let expected = format!("{}@::", objc2::runtime::Bool::ENCODING);
                if super::simple_abi(&snapshot)? != expected.as_bytes() {
                    return Err("SDK resolver ABI differs".into());
                }
                if shared_view {
                    preserved_appkit_lifecycle.push(super::receiver::LifecycleBinding::preserve_appkit_lifecycle(
                        class, selector, snapshot,
                    )?);
                } else {
                    leaves.push((selector, true, snapshot, owned_resolve as *const () as usize));
                }
            }
            for (selector, replacement, abi) in [
                (
                    c".cxx_construct",
                    owned_construct as *const () as usize,
                    b"@@:".as_slice(),
                ),
                (
                    c".cxx_destruct",
                    owned_destruct as *const () as usize,
                    b"v@:".as_slice(),
                ),
                (
                    c"dealloc",
                    deallocator as *const () as usize,
                    b"v@:".as_slice(),
                ),
            ] {
                if let Some(snapshot) = raw(class, Sel::register(selector), false)? {
                    if selector != c"dealloc"
                        && snapshot.owner() != *class as *const AnyClass as usize
                    {
                        continue;
                    }
                    if super::simple_abi(&snapshot)? != abi {
                        return Err("SDK CXX/deallocation ABI differs".into());
                    }
                    if shared_view && selector == c".cxx_construct" {
                        return Err("shared Reference view family has a CXX constructor".into());
                    }
                    if shared_view &&
                        ((selector == c".cxx_destruct" && [c"NSView", c"NSButton", c"NSEvent"].contains(&class.name()))
                            || (selector == c"dealloc" && [c"NSResponder", c"NSView", c"NSControl", c"NSButton", c"NSEvent"]
                                .contains(&class.name())))
                    {
                        preserved_appkit_lifecycle.push(super::receiver::LifecycleBinding::preserve_appkit_lifecycle(class, selector, snapshot)?);
                        continue;
                    }
                    if shared_view && selector == c"dealloc" && class.name() == c"NSStatusBarButton" {
                        let button = metadata::loaded_class(c"NSButton")?;
                        if snapshot.owner() != button.address() {
                            return Err("Reference status button does not inherit NSButton dealloc".into());
                        }
                        continue;
                    }
                    leaves.push((selector, false, snapshot, replacement));
                } else if selector == c"dealloc" {
                    return Err("SDK memory lifecycle missing".into());
                }
            }
            let mut depth = 0;
            let mut ancestor = class.superclass();
            while let Some(parent) = ancestor {
                depth += 1;
                ancestor = parent.superclass();
            }
            plan.push((depth, *class, leaves));
        }
        if preserve_appkit_lifecycle {
            let expected = if classes.len() == 1 && classes[0].name() == c"NSEvent" { 5 } else { 21 };
            if preserved_appkit_lifecycle.len() != expected {
                return Err("preserved Reference AppKit lifecycle roster differs".into());
            }
        }
        plan.sort_by_key(|(depth, _, _)| *depth);
        let mut owners = OWNERS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| "initializer owner poisoned")?;
        if family.keys().any(|key| owners.contains_key(key)) {
            return Err("SDK initializer already owned".into());
        }
        let faulted = Arc::new(AtomicBool::new(false));
        let mut bindings: Vec<ConstructorBinding> = vec![];
        for (_, class, leaves) in plan {
            let handle = metadata::registered_class(class as *const AnyClass as usize)?;
            let mut ancestors = vec![];
            let mut ancestor = class.superclass();
            while let Some(parent) = ancestor {
                ancestors.push(parent.name());
                ancestor = parent.superclass();
            }
            let mut mutations = vec![];
            let mut inherited = vec![];
            for (selector, meta, baseline, replacement) in leaves {
                let actual = metadata::method(handle, selector, meta)?
                    .ok_or("SDK lifecycle disappeared before selection")?;
                if actual.implementation() == replacement {
                    // A parent's completed selection is not a second child
                    // write. Keep the original pre-family baseline and the
                    // closest ancestor RECEIVER's complete provenance, even
                    // when its actual declaring owner is a grandparent.
                    let prior = ancestors
                        .iter()
                        .find_map(|name| {
                            bindings.iter().find(|binding| {
                                binding.class == *name
                                    && binding.selector == selector
                                    && binding.class_method == meta
                            })
                        })
                        .ok_or("SDK inherited selection lacks ancestor provenance")?;
                    prior.verify()?;
                    inherited.push((selector, meta, baseline, actual, prior.provenance.clone()));
                } else {
                    mutations.push((selector, meta, baseline, replacement));
                }
            }
            let changes = mutations
                .iter()
                .map(
                    |(selector, meta, snapshot, replacement)| metadata::LifecycleChange {
                        selector,
                        class_method: *meta,
                        replacement: *replacement,
                        encoding: snapshot.encoding().to_vec(),
                    },
                )
                .collect();
            let batch = if mutations.is_empty() {
                None
            } else {
                Some(metadata::LifecycleBatch::capture(handle, changes)?)
            };
            let mut written = vec![];
            for (selector, meta, baseline, replacement) in mutations {
                before_write()?;
                if let Some(memory) = memory {
                    memory.verify_arc_sources()?;
                }
                let before_write = metadata::method(handle, selector, meta)?
                    .ok_or("SDK lifecycle disappeared before write")?;
                let encoding = std::ffi::CString::new(before_write.encoding())
                    .map_err(|_| "SDK lifecycle encoding contains NUL")?;
                let target = if meta {
                    unsafe { &*(class as *const AnyClass as *const AnyObject) }.class()
                } else {
                    class
                };
                unsafe {
                    class_replaceMethod(
                        target,
                        Sel::register(selector),
                        std::mem::transmute::<usize, Imp>(replacement),
                        encoding.as_ptr(),
                    )
                };
                let selected = metadata::method(handle, selector, meta)?
                    .ok_or("SDK selected lifecycle missing")?;
                written.push((selector, meta, baseline, before_write, selected));
            }
            let proof = batch.map(metadata::LifecycleBatch::finish).transpose()?;
            for (selector, meta, baseline, before_write, selected) in written {
                let prior = if baseline != before_write {
                    Some(
                        &bindings
                            .iter()
                            .find(|binding| {
                                binding.selector == selector
                                    && binding.class_method == meta
                                    && binding.provenance.selected_owner() == before_write.owner()
                                    && binding.replacement == before_write.implementation()
                            })
                            .ok_or("SDK inherited lifecycle provenance missing")?
                            .provenance,
                    )
                } else {
                    None
                };
                let provenance = metadata::ReplacementProvenance::bind_in_batch(
                    baseline.clone(),
                    before_write,
                    selected.clone(),
                    meta,
                    prior,
                    proof.as_ref().ok_or("SDK mutation batch proof missing")?,
                )?;
                bindings.push(ConstructorBinding {
                    class: class.name(),
                    selector,
                    class_method: meta,
                    original: baseline.implementation(),
                    replacement: selected.implementation(),
                    provenance,
                    faulted: faulted.clone(),
                });
            }
            for (selector, meta, baseline, before_child_writes, prior) in inherited {
                let selected = metadata::method(handle, selector, meta)?
                    .ok_or("SDK inherited lifecycle disappeared after local writes")?;
                if selected != before_child_writes {
                    return Err("SDK inherited lifecycle changed during child selection".into());
                }
                let provenance = metadata::ReplacementProvenance::inherit(
                    baseline.clone(),
                    selected.clone(),
                    meta,
                    &prior,
                    proof.as_ref(),
                )?;
                bindings.push(ConstructorBinding {
                    class: class.name(),
                    selector,
                    class_method: meta,
                    original: baseline.implementation(),
                    replacement: selected.implementation(),
                    provenance,
                    faulted: faulted.clone(),
                });
            }
            owners.insert(
                class as *const AnyClass as usize,
                InitializerOwner {
                    lease: lease.clone(),
                    faulted: faulted.clone(),
                },
            );
        }
        drop(owners);
        let (initializers, lifecycle) = bindings
            .into_iter()
            .partition(|binding| binding.class_method && binding.selector == c"initialize");
        let guard = Self {
            initializers,
            lifecycle,
            preserved_appkit_lifecycle,
            lease,
            families: family.keys().copied().collect(),
            created: Mutex::new(BTreeMap::new()),
            additions: Mutex::new(Vec::new()),
            task_wait_addition: Mutex::new(None),
        };
        guard.verify()?;
        Ok(guard)
    }
    /// # Safety
    /// Only retained classes from the exact reviewed owned endpoint image may be
    /// passed. No live original SDK object, initializer, callback or other thread
    /// may race registration. The integrator separately proves loader admission.
    pub unsafe fn select(classes: &[&'static AnyClass], lease: OwnedHandleLease) -> Result<Self> {
        lease.before_effect()?;
        if classes.is_empty() {
            return Err("empty class family".into());
        }
        let mut family = BTreeMap::new();
        for class in classes {
            let mut c = Some(*class);
            while let Some(parent) = c {
                if parent == NSObject::class() {
                    break;
                }
                family.insert(parent as *const AnyClass as usize, parent);
                c = parent.superclass();
            }
        }
        let replacement = initialize as *const () as usize;
        let mut plan = vec![];
        for c in family.values() {
            let method =
                raw(c, objc2::sel!(initialize), true)?.ok_or("missing class initializer ABI")?;
            let encoding = std::ffi::CString::new(method.encoding())
                .map_err(|_| "initializer encoding contains NUL")?;
            let name = unsafe { CStr::from_ptr(c.name().as_ptr()) };
            plan.push((*c, name, method, encoding));
        }
        plan.sort_by_key(|(class, _, _, _)| {
            let mut depth = 0;
            let mut current = class.superclass();
            while let Some(parent) = current {
                depth += 1;
                current = parent.superclass();
            }
            depth
        });
        let owners = OWNERS.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut owner = owners.lock().map_err(|_| "initializer owner poisoned")?;
        for (c, _, _, _) in &plan {
            if owner.contains_key(&(*c as *const AnyClass as usize)) {
                return Err("duplicate initializer ownership".into());
            }
        }
        let mut initializers = vec![];
        let faulted = Arc::new(AtomicBool::new(false));
        for (c, name, baseline, encoding) in plan {
            let before_write = raw(c, objc2::sel!(initialize), true)?
                .ok_or("initializer disappeared before write")?;
            let before_lifecycle =
                metadata::lifecycle(metadata::registered_class(c as *const AnyClass as usize)?)?;
            let inherited = if before_write != baseline {
                Some(
                    initializers
                        .iter()
                        .find(|p: &&ConstructorBinding| {
                            p.provenance.selected_owner() == before_write.owner()
                                && p.replacement == before_write.implementation()
                        })
                        .map(|p| &p.provenance)
                        .ok_or("unowned inherited initializer change")?,
                )
            } else {
                None
            };
            owner.insert(
                c as *const AnyClass as usize,
                InitializerOwner {
                    lease: lease.clone(),
                    faulted: faulted.clone(),
                },
            );
            let old = unsafe {
                class_replaceMethod(
                    c.metaclass(),
                    objc2::sel!(initialize),
                    std::mem::transmute::<usize, Imp>(replacement),
                    encoding.as_ptr(),
                )
            };
            if old.is_some_and(|p| p as usize != before_write.implementation()) {
                return Err("initializer changed during selection".into());
            }
            let selected =
                raw(c, objc2::sel!(initialize), true)?.ok_or("selected initializer missing")?;
            let original = baseline.implementation();
            let provenance = metadata::ReplacementProvenance::bind_lifecycle(
                baseline,
                before_write,
                selected,
                before_lifecycle,
                true,
                inherited,
            )?;
            initializers.push(ConstructorBinding {
                class: name,
                selector: c"initialize",
                class_method: true,
                original,
                replacement,
                provenance,
                faulted: faulted.clone(),
            });
        }
        drop(owner);
        let guard = Self {
            initializers,
            lifecycle: vec![],
            preserved_appkit_lifecycle: Vec::new(),
            lease,
            families: family.keys().copied().collect(),
            created: Mutex::new(BTreeMap::new()),
            additions: Mutex::new(Vec::new()),
            task_wait_addition: Mutex::new(None),
        };
        guard.verify()?;
        Ok(guard)
    }
    /// Creates a subclass that preserves runtime type checks while every inherited
    /// SDK instance leaf initially terminates the own endpoint. The integrator
    /// overlays only reviewed typed replacements afterwards. NSObject retain/type
    /// data operations are inherited; no AppKit -init is performed.
    /// # Safety
    /// The name is globally unique and the class is one of this selected family's
    /// source-bound classes. All future calls stay inside the guarded endpoint.
    pub unsafe fn subclass(
        &self,
        name: &CStr,
        parent: &'static AnyClass,
    ) -> Result<&'static AnyClass> {
        unsafe { self.subclass_with_skips(name, parent, &[]) }
    }
    /// Keep precisely the already-qualified ancestor lifecycle. All other SDK
    /// leaves still deny until their exact typed implementation is overlaid.
    /// # Safety
    /// Same source/preentry requirements as select_sdk_family; allocation must
    /// use its full WorkspaceGrant rather than the primitive own-class allocator.
    pub unsafe fn subclass_qualified(
        &self,
        name: &CStr,
        parent: &'static AnyClass,
    ) -> Result<&'static AnyClass> {
        self.verify()?;
        super::factory::RootDeallocator::capture()?.verify_proxy_arc(parent)?;
        let mut ancestor = Some(parent);
        while let Some(class) = ancestor {
            if class == NSObject::class() {
                break;
            }
            let dealloc = raw(class, objc2::sel!(dealloc), false)?
                .ok_or("qualified SDK deallocation missing")?;
            if !self.lifecycle.iter().any(|binding| {
                binding.class == class.name()
                    && binding.selector == c"dealloc"
                    && !binding.class_method
                    && binding.replacement == dealloc.implementation()
            }) && !self.preserved_appkit_lifecycle.iter().any(|binding| binding.matches_preserved_appkit_method(c"dealloc", &dealloc)) {
                return Err("SDK typed deallocation provenance missing".into());
            }
            for method in class.instance_methods().iter() {
                let selector = method.name();
                if [
                    Sel::register(c".cxx_construct"),
                    Sel::register(c".cxx_destruct"),
                ]
                .contains(&selector)
                    && !self.lifecycle.iter().any(|binding| {
                        binding.class == class.name()
                            && !binding.class_method
                            && Sel::register(binding.selector) == selector
                            && binding.replacement == method.implementation() as usize
                    })
                    && !self.preserved_appkit_lifecycle.iter().any(|binding| {
                        binding.class == class.name()
                            && binding.selector == selector.name()
                            && binding.replacement == method.implementation() as usize
                    })
                {
                    return Err("SDK ancestor CXX provenance missing".into());
                }
            }
            ancestor = class.superclass();
        }
        unsafe {
            self.subclass_with_skips(
                name,
                parent,
                &[c".cxx_construct", c".cxx_destruct", c"dealloc"],
            )
        }
    }
    /// Install private initialization and deallocation on proxies whose
    /// AppKit ancestors also have genuine platform instances.
    /// # Safety
    /// This guard owns exclusive preentry. The proxy has no live instances.
    pub(super) unsafe fn bind_preserved_appkit_proxy_lifecycle(
        &self, proxy: &'static AnyClass,
        deallocate: unsafe extern "C" fn(*mut AnyObject, Sel),
    ) -> Result<Vec<super::receiver::LifecycleBinding>> {
        if self.preserved_appkit_lifecycle.is_empty() {
            return Ok(Vec::new());
        }
        self.verify()?;
        let parent = match proxy.name().to_bytes() {
            b"RuycastF1OwnedResponderSource" => c"NSResponder",
            b"RuycastF1OwnedStatusButtonSource" => c"NSStatusBarButton",
            b"RuycastF1OwnedEventSource" => c"NSEvent",
            _ => return Err("unexpected private shared-view deallocator".into()),
        };
        let handle = metadata::registered_class(proxy as *const AnyClass as usize)?;
        if proxy.superclass().map(|class| class.name()) != Some(parent)
            || !self.created.lock().map_err(|_| "created classes poisoned")?.contains_key(&handle.address())
        {
            return Err("private shared-view deallocator proxy differs".into());
        }
        let parent = proxy.superclass().ok_or("private proxy parent absent")?;
        let mut owners = OWNERS.get().ok_or("shared-view initializer owners absent")?
            .lock().map_err(|_| "shared-view initializer owners poisoned")?;
        let owner = owners.get(&(parent as *const AnyClass as usize)).cloned()
            .ok_or("shared-view proxy parent owner absent")?;
        if owners.contains_key(&handle.address()) {
            return Err("shared-view proxy initializer already owned".into());
        }
        let mut leaves = Vec::new();
        for (selector, class_method, replacement) in [
            (c"dealloc", false, deallocate as *const () as usize),
            (c"initialize", true, initialize as *const () as usize),
        ] {
            let baseline = metadata::method(handle, selector, class_method)?
                .ok_or("inherited proxy lifecycle missing")?;
            if !self.preserved_appkit_lifecycle.iter().any(|binding| binding.matches_preserved_appkit_method(selector, &baseline)) {
                return Err("private proxy does not inherit the preserved AppKit lifecycle".into());
            }
            leaves.push((selector, class_method, replacement, baseline));
        }
        let batch = metadata::LifecycleBatch::capture(handle, leaves.iter()
            .map(|(selector, class_method, replacement, baseline)| metadata::LifecycleChange {
                selector, class_method: *class_method, replacement: *replacement,
                encoding: baseline.encoding().to_vec(),
            }).collect())?;
        for (selector, class_method, replacement, _) in &leaves {
            let target = if *class_method { proxy.metaclass() } else { proxy };
            if unsafe { class_addMethod((target as *const AnyClass).cast(), sel_registerName(selector.as_ptr()),
                *replacement as *const _, c"v16@0:8".as_ptr()) } == 0 {
                return Err("private shared-view local lifecycle add failed".into());
            }
        }
        let batch = batch.finish()?;
        let bindings = leaves.into_iter().map(|(selector, class_method, replacement, baseline)| {
            let selected = metadata::method(handle, selector, class_method)?
                .ok_or("private shared-view lifecycle missing")?;
            let original = baseline.implementation();
            let provenance = metadata::ReplacementProvenance::bind_in_batch(
                baseline.clone(), baseline, selected, class_method, None, &batch,
            )?;
            Ok(super::receiver::LifecycleBinding::constructor(ConstructorBinding {
                class: proxy.name(), selector, class_method, original, replacement,
                provenance, faulted: owner.faulted.clone(),
            }))
        }).collect::<Result<Vec<_>>>()?;
        owners.insert(handle.address(), owner);
        Ok(bindings)
    }
    pub(super) unsafe fn subclass_foundation_memory_qualified(
        &self,
        name: &CStr,
        parent: &'static AnyClass,
        role: &'static CStr,
        memory: &super::factory::FoundationMemoryPlan,
        model: &Arc<Workspace>,
    ) -> Result<(&'static AnyClass, OwnedMemoryArcBinding)> {
        let generation = model
            .state
            .lock()
            .map_err(|_| "workspace state poisoned")?
            .generation;
        super::factory::verify_memory_epoch(model, generation)?;
        self.verify()?;
        super::factory::verify_memory_epoch(model, generation)?;
        if !Arc::ptr_eq(&self.lease.life, &model.context.life)
            || self.lease.root.path() != model.context.home.path()
        {
            return Err("memory profile belongs to another workspace lease".into());
        }
        let parent_plan = memory.parent_for(role, parent)?.clone();
        memory.verify_arc_sources()?;
        let constructors = self
            .initializers
            .iter()
            .chain(&self.lifecycle)
            .cloned()
            .collect::<Vec<_>>();
        // Every ancestor still requires selected CXX and deallocation proof.
        let mut ancestor = Some(parent);
        while let Some(class) = ancestor {
            if class == NSObject::class() {
                break;
            }
            for selector in [c"dealloc", c".cxx_construct", c".cxx_destruct"] {
                if let Some(actual) = raw(class, Sel::register(selector), false)? {
                    if (selector == c"dealloc"
                        || actual.owner() == class as *const AnyClass as usize)
                        && !constructors.iter().any(|row| {
                            row.class == class.name()
                                && row.selector == selector
                                && !row.class_method
                                && row.replacement == actual.implementation()
                        })
                    {
                        return Err("memory profile ancestor lifecycle unqualified".into());
                    }
                } else if selector == c"dealloc" {
                    return Err("memory profile deallocation absent".into());
                }
            }
            ancestor = class.superclass();
        }
        let class = unsafe {
            self.subclass_with_skips_checked(
                name,
                parent,
                &[c".cxx_construct", c".cxx_destruct", c"dealloc"],
                || {
                    super::factory::verify_memory_epoch(model, generation)?;
                    memory.verify_arc_sources()
                },
            )
        }?;
        let mut before = vec![];
        let mut selected = vec![];
        for (selector, root) in &memory.roots {
            self.verify()?;
            super::factory::verify_memory_epoch(model, generation)?;
            memory.verify_arc_sources()?;
            let old = raw(class, Sel::register(selector), false)?
                .ok_or("owned memory original lookup missing")?;
            super::factory::verify_memory_abi(selector, &old)?;
            let encoding =
                std::ffi::CString::new(root.encoding()).map_err(|_| "owned memory encoding NUL")?;
            let previous = unsafe {
                class_replaceMethod(
                    class,
                    Sel::register(selector),
                    std::mem::transmute::<usize, Imp>(root.implementation()),
                    encoding.as_ptr(),
                )
            };
            if (old.owner() == class as *const AnyClass as usize
                && previous.map(|p| p as usize) != Some(old.implementation()))
                || (old.owner() != class as *const AnyClass as usize && previous.is_some())
            {
                return Err("owned memory declaration raced".into());
            }
            let actual = raw(class, Sel::register(selector), false)?
                .ok_or("owned memory declaration absent")?;
            if actual.owner() != class as *const AnyClass as usize
                || actual.implementation() != root.implementation()
                || actual.encoding()
                    != if old.owner() == class as *const AnyClass as usize {
                        old.encoding()
                    } else {
                        root.encoding()
                    }
                || actual.code() != root.code()
            {
                return Err(format!(
                    "owned memory declaration differs for {}: owner={} imp={} encoding={} code={}",
                    selector.to_string_lossy(),
                    actual.owner() == class as *const AnyClass as usize,
                    actual.implementation() == root.implementation(),
                    actual.encoding() == root.encoding(),
                    actual.code() == root.code()
                ));
            }
            {
                let mut created = self.created.lock().map_err(|_| "created class poisoned")?;
                let rows = created
                    .get_mut(&(class as *const AnyClass as usize))
                    .ok_or("memory class not created here")?;
                if let Some(row) = rows
                    .iter_mut()
                    .find(|(name, _)| *name == Sel::register(selector))
                {
                    row.1 = root.implementation();
                } else {
                    rows.push((Sel::register(selector), root.implementation()));
                }
            }
            super::factory::verify_memory_abi(selector, &actual)?;
            before.push((*selector, old));
            selected.push((*selector, actual));
        }
        let proof = OwnedMemoryArcBinding(Arc::new(OwnedMemoryArcInner {
            class: class.name(),
            parent: parent_plan,
            plan: memory.clone(),
            before,
            selected,
            lifecycle: metadata::lifecycle(metadata::registered_class(
                class as *const AnyClass as usize,
            )?)?,
            constructors,
            model: model.clone(),
            generation,
            retired: AtomicBool::new(false),
        }));
        proof.verify()?;
        Ok((class, proof))
    }
    unsafe fn subclass_with_skips(
        &self,
        name: &CStr,
        parent: &'static AnyClass,
        skip: &[&CStr],
    ) -> Result<&'static AnyClass> {
        unsafe { self.subclass_with_skips_checked(name, parent, skip, || Ok(())) }
    }
    unsafe fn subclass_with_skips_checked(
        &self,
        name: &CStr,
        parent: &'static AnyClass,
        skip: &[&CStr],
        prewrite: impl Fn() -> Result<()>,
    ) -> Result<&'static AnyClass> {
        self.verify()?;
        prewrite()?;
        if !self
            .families
            .contains(&(parent as *const AnyClass as usize))
        {
            return Err("unowned superclass".into());
        }
        prewrite()?;
        let builder = ClassBuilder::new(name, parent).ok_or("owned class name collision")?;
        let class = builder.register();
        let mut overridden = std::collections::HashSet::new();
        let mut selected = vec![];
        let mut c = Some(parent);
        while let Some(current) = c {
            if current == NSObject::class() {
                break;
            }
            for method in current.instance_methods().iter() {
                let selector = method.name();
                if skip.contains(&selector.name()) {
                    continue;
                }
                if !overridden.insert(selector) {
                    continue;
                }
                let encoding = unsafe { method_get_type_encoding(method) };
                if encoding.is_null() {
                    return Err("inherited method has no encoding".into());
                }
                prewrite()?;
                unsafe {
                    class_replaceMethod(
                        class,
                        selector,
                        std::mem::transmute::<usize, Imp>(deny_unknown as *const () as usize),
                        encoding,
                    )
                };
                selected.push((selector, deny_unknown as *const () as usize));
            }
            c = current.superclass();
        }
        self.created
            .lock()
            .map_err(|_| "created class registry poisoned")?
            .insert(class as *const AnyClass as usize, selected);
        Ok(class)
    }
    /// # Safety
    /// Every supported leaf on class must already be overlaid and independently
    /// checked. The returned owned object has zeroed SDK storage and must never
    /// reach an original SDK method; unknown inherited methods terminate instead.
    pub unsafe fn allocate(&self, class: &'static AnyClass) -> Result<Retained<AnyObject>> {
        self.verify()?;
        let created = self
            .created
            .lock()
            .map_err(|_| "created class registry poisoned")?;
        let leaves = created
            .get(&(class as *const AnyClass as usize))
            .ok_or("class not created by this guard")?;
        for (selector, expected) in leaves {
            if raw(class, *selector, false)?
                .ok_or("guarded leaf disappeared")?
                .implementation()
                != *expected
            {
                return Err("guarded inherited leaf changed".into());
            }
        }
        // Runtime raw allocation/destruction may execute each ancestor's local
        // C++ constructor/destructor. A child override cannot prove these safe.
        // The common factory owner must supply those exact separate proofs;
        // this primitive deliberately refuses every such unresolved family.
        let mut ancestor = Some(class);
        while let Some(current) = ancestor {
            if current == NSObject::class() {
                break;
            }
            for name in [c".cxx_construct", c".cxx_destruct"] {
                if current
                    .instance_methods()
                    .iter()
                    .any(|m| m.name() == Sel::register(name))
                {
                    return Err("ancestor CXX constructor/destructor factory proof required".into());
                }
            }
            ancestor = current.superclass();
        }
        if leaves.iter().any(|(selector, expected)| {
            *selector == objc2::sel!(dealloc) && *expected == deny_unknown as *const () as usize
        }) {
            return Err("typed owned dealloc overlay required before allocation".into());
        }
        if raw(class, objc2::sel!(dealloc), false)?
            .ok_or("owned deallocation metadata missing")?
            .owner()
            != NSObject::class() as *const AnyClass as usize
        {
            return Err("typed SDK deallocation requires the common allocation grant".into());
        }
        let mut c = class.superclass();
        let mut found = false;
        while let Some(parent) = c {
            if self
                .families
                .contains(&(parent as *const AnyClass as usize))
            {
                found = true;
                break;
            }
            c = parent.superclass();
        }
        if !found {
            return Err("allocation outside selected family".into());
        }
        unsafe { Retained::from_raw(class_createInstance(class, 0)) }
            .ok_or("owned allocation failed".into())
    }
    /// Overlays one reviewed typed instance leaf and updates the allocation
    /// identity snapshot. Never mutates a superclass's inherited Method.
    /// # Safety
    /// replacement has exactly the supplied/current Objective-C ABI and is an
    /// owned source-bound implementation which never calls a forbidden original.
    /// No live instance or callback may race this pre-entry mutation.
    pub unsafe fn overlay(
        &self,
        class: &'static AnyClass,
        selector: &CStr,
        expected: usize,
        replacement: usize,
        encoding: &CStr,
    ) -> Result<()> {
        self.verify()?;
        if expected == 0 || replacement == 0 || expected == replacement {
            return Err("invalid typed leaf overlay".into());
        }
        let mut created = self
            .created
            .lock()
            .map_err(|_| "created class registry poisoned")?;
        let leaves = created
            .get_mut(&(class as *const AnyClass as usize))
            .ok_or("overlay outside guard-created class")?;
        let selector = Sel::register(selector);
        let row = leaves
            .iter_mut()
            .find(|(s, _)| *s == selector)
            .ok_or("overlay outside guarded SDK instance leaves")?;
        let method = raw(class, selector, false)?.ok_or("overlay leaf missing")?;
        let actual_encoding = std::ffi::CString::new(method.encoding())
            .map_err(|_| "overlay encoding contains NUL")?;
        if actual_encoding.as_c_str() != encoding
            || row.1 != expected
            || method.implementation() != expected
        {
            return Err("overlay current ABI/identity differs".into());
        }
        let previous = unsafe {
            class_replaceMethod(
                class,
                selector,
                std::mem::transmute::<usize, Imp>(replacement),
                actual_encoding.as_ptr(),
            )
        };
        if previous.is_none_or(|p| p as usize != expected) {
            return Err("overlay leaf changed during selection".into());
        }
        row.1 = replacement;
        Ok(())
    }
}
pub(crate) fn verify_typed_abi(
    snapshot: &metadata::MethodSnapshot,
    expected: &[objc2::encode::Encoding],
) -> Result<()> {
    if expected.len() < 3 {
        return Err("SDK signature lacks hidden arguments".into());
    }
    // This is a snapshot-proven Method pointer, not a resolving class lookup.
    let method = unsafe { &*(snapshot.method_address() as *const Method) };
    if method.arguments_count() + 1 != expected.len() {
        return Err("SDK compiled argument count differs".into());
    }
    for (index, encoding) in expected.iter().enumerate() {
        let actual = if index == 0 {
            method
                .return_type()
                .to_str()
                .map_err(|_| "SDK ABI encoding is not UTF8")?
                .to_owned()
        } else {
            method
                .argument_type(index - 1)
                .ok_or("SDK argument encoding missing")?
                .to_str()
                .map_err(|_| "SDK ABI encoding is not UTF8")?
                .to_owned()
        };
        if normalize_abi(&actual)? != encoding.to_string() {
            return Err("SDK compiled argument or return ABI differs".into());
        }
    }
    Ok(())
}
fn normalize_abi(value: &str) -> Result<String> {
    let value = value.trim_start_matches(['r', 'n', 'N', 'o', 'O', 'R', 'V']);
    let mut normalized = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        normalized.push(ch);
        if ch == '@' && chars.peek() == Some(&'"') {
            chars.next();
            let mut closed = false;
            for quoted in chars.by_ref() {
                if quoted == '"' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("unterminated SDK object ABI name".into());
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_mount_frame_abi_matches_loaded_appkit() {
        use objc2::encode::{Encode, Encoding};
        let view = metadata::loaded_class(c"NSView").unwrap();
        let method = metadata::method(view, c"setFrame:", false).unwrap().unwrap();
        assert_eq!(
            method.code().image(),
            b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit",
        );
        verify_typed_abi(
            &method,
            &[Encoding::Void, Encoding::Object, Encoding::Sel, objc2_foundation::NSRect::ENCODING],
        ).unwrap();
    }

    #[test]
    fn declaration_readcap_refuses_fifo_without_waiting_for_a_writer() {
        use sha2::{Digest, Sha256};
        use std::os::unix::ffi::OsStrExt;
        let fixture = super::super::tests::Fixture::new();
        let path = fixture.root.join("temp/NSString.rs");
        let bytes = b"owned declaration readcap bytes";
        let expected: [u8; 32] = Sha256::digest(bytes).into();
        std::fs::write(&path, bytes).unwrap();
        let owner = unsafe { libc::geteuid() };
        assert_eq!(StringBytesDeclaration::read_source(&path, &expected, owner).unwrap(), bytes);
        std::fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // Same bounded watchdog style as existing owned allocator canaries.
        // It is never a writer: a blocking-open regression must fail rather
        // than silently completing because the test supplied a FIFO peer.
        let done = Arc::new(AtomicBool::new(false));
        let observed = done.clone();
        let watch = std::thread::spawn(move || {
            for _ in 0..200 {
                if observed.load(Ordering::Acquire) { return; }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            eprintln!("owned declaration FIFO open blocked without a writer");
            std::process::exit(77);
        });
        let result = StringBytesDeclaration::read_source(&path, &expected, owner);
        done.store(true, Ordering::Release);
        watch.join().unwrap();
        assert!(result.is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_pins_are_rechecked_after_guard_before_family_writes() {
        use crate::sdk::owned::AliveLease;
        use std::sync::atomic::AtomicUsize;
        static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn original(_: *mut AnyClass, _: Sel) {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C" fn changed(_: *mut AnyClass, _: Sel) {
            ORIGINAL.fetch_add(2, Ordering::SeqCst);
        }
        unsafe extern "C" fn dealloc(_: *mut AnyObject, _: Sel) {}
        struct Mutate {
            method: usize,
            calls: AtomicUsize,
        }
        impl AliveLease for Mutate {
            fn is_alive(&self) -> bool {
                true
            }
            fn before_effect(&self) -> Result<()> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    unsafe {
                        (&*(self.method as *const Method)).set_implementation(
                            std::mem::transmute::<usize, Imp>(changed as *const () as usize),
                        );
                    }
                }
                Ok(())
            }
        }
        let fixture = super::super::tests::Fixture::new();
        for (name, checked) in [
            (c"RuycastF1OwnedSourcePinEarly", false),
            (c"RuycastF1OwnedSourcePinAfterGuard", true),
        ] {
            let mut builder = ClassBuilder::new(name, NSObject::class()).unwrap();
            unsafe {
                builder.add_class_method(
                    objc2::sel!(initialize),
                    original as unsafe extern "C" fn(*mut AnyClass, Sel),
                );
            }
            let class = builder.register();
            let baseline = raw(class, objc2::sel!(initialize), true).unwrap().unwrap();
            let dealloc_before = raw(class, objc2::sel!(dealloc), false).unwrap().unwrap();
            let life = Arc::new(Mutate {
                method: baseline.method_address(),
                calls: AtomicUsize::new(0),
            });
            let mut lease = fixture.model.lease();
            lease.life = life.clone();
            let validate = || {
                if raw(class, objc2::sel!(initialize), true)?.as_ref() != Some(&baseline) {
                    return Err("qualified source changed during guard".into());
                }
                Ok(())
            };
            if checked {
                let result = unsafe {
                    ClassGuard::select_sdk_family_checked(&[class], lease, dealloc, validate)
                };
                assert!(result.is_err());
                assert_eq!(
                    raw(class, objc2::sel!(dealloc), false).unwrap().unwrap(),
                    dealloc_before
                );
                assert_eq!(
                    raw(class, objc2::sel!(initialize), true)
                        .unwrap()
                        .unwrap()
                        .implementation(),
                    changed as *const () as usize
                );
            } else {
                // Reproduce the prior callsite ordering: checking pins before
                // the dynamic guardian admits the changed source baseline.
                validate().unwrap();
                assert!(unsafe { ClassGuard::select_sdk_family(&[class], lease, dealloc) }.is_ok());
                assert_eq!(
                    raw(class, objc2::sel!(dealloc), false)
                        .unwrap()
                        .unwrap()
                        .implementation(),
                    dealloc as *const () as usize
                );
            }
            assert_eq!(
                life.calls.load(Ordering::SeqCst),
                if checked { 1 } else { 2 }
            );
        }
        assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn sdk_family_inherits_nearest_receiver_with_local_child_cxx() {
        use std::sync::atomic::AtomicUsize;
        static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn original_initialize(_: *mut AnyClass, _: Sel) {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C" fn original_construct(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
            p
        }
        unsafe extern "C" fn selected_dealloc(_: *mut AnyObject, _: Sel) {}
        let fixture = super::super::tests::Fixture::new();
        let mut builder =
            ClassBuilder::new(c"RuycastF1OwnedInheritedRoot", NSObject::class()).unwrap();
        unsafe {
            builder.add_class_method(
                objc2::sel!(initialize),
                original_initialize as unsafe extern "C" fn(*mut AnyClass, Sel),
            );
        }
        let root = builder.register();
        let middle = ClassBuilder::new(c"RuycastF1OwnedInheritedMiddle", root)
            .unwrap()
            .register();
        let child = ClassBuilder::new(c"RuycastF1OwnedInheritedLocalChild", middle)
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
        // ClassBuilder::add_method performs a resolving superclass lookup in
        // debug builds. Use only raw own-class registration before selection.
        unsafe {
            assert_ne!(
                class_addMethod(
                    (child as *const AnyClass).cast(),
                    sel_registerName(c".cxx_construct".as_ptr()),
                    original_construct as *const () as *const std::ffi::c_void,
                    c"@@:".as_ptr()
                ),
                0
            );
        }
        assert_eq!(
            ORIGINAL.load(Ordering::SeqCst),
            0,
            "fixture setup invoked original hooks"
        );
        let guard = unsafe {
            ClassGuard::select_sdk_family(&[child], fixture.model.lease(), selected_dealloc)
        }
        .unwrap();
        let proxy =
            unsafe { guard.subclass_qualified(c"RuycastF1OwnedInheritedProxy", child) }.unwrap();
        let grant = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                fixture.model.clone(),
                proxy,
                guard.lifecycle_bindings(),
            )
        }
        .unwrap();
        grant.verify().unwrap();
        assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn actual_initializer_guard_panics_retire_constructor_family_without_original_call() {
        static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn original(_: *mut AnyClass, _: Sel) {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
        }
        for mode in [1, 2] {
            let f = super::super::tests::Fixture::new();
            let name = if mode == 1 {
                c"RuycastF1OwnedBeforePanicInitializer"
            } else {
                c"RuycastF1OwnedAlivePanicInitializer"
            };
            let mut builder = ClassBuilder::new(name, NSObject::class()).unwrap();
            unsafe {
                builder.add_class_method(
                    objc2::sel!(initialize),
                    original as unsafe extern "C" fn(*mut AnyClass, Sel),
                );
            }
            let class = builder.register();
            let guard = unsafe { ClassGuard::select(&[class], f.model.lease()) }.unwrap();
            let selected = raw(class, objc2::sel!(initialize), true).unwrap().unwrap();
            assert_eq!(selected.implementation(), guard.initializers[0].replacement);
            let call: unsafe extern "C" fn(*mut AnyObject, Sel) =
                unsafe { std::mem::transmute(selected.implementation()) };
            f.life.1.store(mode, Ordering::Release);
            unsafe {
                call(
                    (class as *const AnyClass).cast_mut().cast(),
                    objc2::sel!(initialize),
                )
            };
            f.life.1.store(0, Ordering::Release);
            assert!(guard.verify().is_err());
            assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
        }
    }
    #[test]
    fn ancestor_cxx_constructor_is_refused_before_raw_allocation() {
        static CXX: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn cpp(this: *mut AnyObject, _: Sel) -> *mut AnyObject {
            CXX.fetch_add(1, Ordering::SeqCst);
            this
        }
        unsafe extern "C" fn init(_: *mut AnyClass, _: Sel) {}
        let f = super::super::tests::Fixture::new();
        let mut b =
            ClassBuilder::new(c"RuycastF1WorkspaceOwnedCXXAncestor", NSObject::class()).unwrap();
        unsafe {
            b.add_class_method(
                objc2::sel!(initialize),
                init as unsafe extern "C" fn(*mut AnyClass, Sel),
            );
            b.add_method(
                Sel::register(c".cxx_construct"),
                cpp as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
        }
        let ancestor = b.register();
        let guard = unsafe { ClassGuard::select(&[ancestor], f.model.lease()) }.unwrap();
        let child =
            unsafe { guard.subclass(c"RuycastF1WorkspaceOwnedCXXChild", ancestor) }.unwrap();
        let result = unsafe { guard.allocate(child) };
        assert!(result.err().unwrap().contains("CXX"));
        assert_eq!(CXX.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn selected_ancestor_initializer_and_unknown_leaves_never_reach_original() {
        static INITIALIZED: AtomicUsize = AtomicUsize::new(0);
        static DANGER: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn original_initialize(_: *mut AnyClass, _: Sel) {
            INITIALIZED.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C" fn dangerous(_: *mut AnyObject, _: Sel) -> usize {
            DANGER.fetch_add(1, Ordering::SeqCst);
            1
        }
        unsafe extern "C" fn owned_result(_: *mut AnyObject, _: Sel) -> usize {
            77
        }
        let f = super::super::tests::Fixture::new();
        let mut b =
            ClassBuilder::new(c"RuycastF1WorkspaceOwnedAncestor", NSObject::class()).unwrap();
        unsafe {
            b.add_class_method(
                objc2::sel!(initialize),
                original_initialize as unsafe extern "C" fn(*mut AnyClass, Sel),
            );
            b.add_method(
                objc2::sel!(ownedDangerousInventory),
                dangerous as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
        }
        let ancestor = b.register();
        let guard = unsafe { ClassGuard::select(&[ancestor], f.model.lease()) }.unwrap();
        guard.verify().unwrap();
        let child =
            unsafe { guard.subclass(c"RuycastF1WorkspaceOwnedDescendant", ancestor) }.unwrap();
        assert_eq!(
            child
                .instance_method(objc2::sel!(ownedDangerousInventory))
                .unwrap()
                .implementation() as usize,
            deny_unknown as *const () as usize
        );
        let selector = objc2::sel!(ownedDangerousInventory);
        let method = child.instance_method(selector).unwrap();
        let encoding = unsafe { CStr::from_ptr(method_get_type_encoding(method)) };
        assert!(unsafe {
            guard.overlay(
                child,
                c"ownedDangerousInventory",
                deny_unknown as *const () as usize,
                owned_result as *const () as usize,
                c"v@:",
            )
        }
        .is_err());
        assert!(unsafe {
            guard.overlay(
                ancestor,
                c"ownedDangerousInventory",
                dangerous as *const () as usize,
                owned_result as *const () as usize,
                encoding,
            )
        }
        .is_err());
        unsafe {
            guard.overlay(
                child,
                c"ownedDangerousInventory",
                deny_unknown as *const () as usize,
                owned_result as *const () as usize,
                encoding,
            )
        }
        .unwrap();
        let instance = unsafe { guard.allocate(child) }.unwrap();
        let value: usize = unsafe { objc2::msg_send![&*instance, ownedDangerousInventory] };
        assert_eq!(value, 77);
        let is_ancestor: objc2::runtime::Bool =
            unsafe { objc2::msg_send![&*instance,isKindOfClass:ancestor] };
        assert!(is_ancestor.as_bool());
        assert_eq!(INITIALIZED.load(Ordering::SeqCst), 0);
        assert_eq!(DANGER.load(Ordering::SeqCst), 0);
        // Keep zeroed fake SDK storage alive until the owned process exits. No
        // original subclass -dealloc is allowed to observe it.
        let _held = Retained::into_raw(instance);
        f.life.0.store(false, Ordering::Release);
        assert!(guard.verify().is_err());
        assert!(unsafe { guard.allocate(child) }.is_err());
    }
    #[test]
    fn selected_sibling_families_require_per_proxy_lifecycle_chains() {
        unsafe extern "C" fn selected_dealloc(_: *mut AnyObject, _: Sel) {}
        let fixture = super::super::tests::Fixture::new();
        let first = ClassBuilder::new(c"RuycastF1OwnedSiblingFirst", NSObject::class())
            .unwrap()
            .register();
        let second = ClassBuilder::new(c"RuycastF1OwnedSiblingSecond", NSObject::class())
            .unwrap()
            .register();
        let guard = unsafe {
            ClassGuard::select_sdk_family(&[first, second], fixture.model.lease(), selected_dealloc)
        }
        .unwrap();
        let proxy =
            unsafe { guard.subclass_qualified(c"RuycastF1OwnedSiblingProxy", first) }.unwrap();
        let union = guard.lifecycle_bindings();
        let result = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                fixture.model.clone(),
                proxy,
                union.clone(),
            )
        };
        assert!(
            matches!(result,Err(ref error) if error=="foreign/duplicate receiver lifecycle identity")
        );
        let scoped = guard.lifecycle_bindings_for(first).unwrap();
        assert!(scoped.iter().all(|row| row.class == first.name()));
        let wrong = guard.lifecycle_bindings_for(second).unwrap();
        let result = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                fixture.model.clone(),
                proxy,
                wrong.clone(),
            )
        };
        assert!(
            matches!(result,Err(ref error) if error=="foreign/duplicate receiver lifecycle identity")
        );
        let mut contaminated = scoped.clone();
        contaminated.push(wrong[0].clone());
        assert!(unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                fixture.model.clone(),
                proxy,
                contaminated,
            )
        }
        .is_err());
        assert!(guard.lifecycle_bindings_for(NSObject::class()).is_err());
        let result = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                fixture.model.clone(),
                proxy,
                scoped,
            )
        };
        assert!(
            result.is_ok(),
            "each proxy requires only its actual ancestor lifecycle chain: {}",
            result.err().unwrap_or_default()
        );
    }
    #[test]
    fn immutable_sdk_ledger_keeps_origin_and_allows_final_worker_cleanup_after_retirement() {
        // Ledger mechanics only: this is an explicitly own NSObject class,
        // never evidence that an actual Foundation or NSEvent family is ready.
        static DEALLOCATED: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn cleanup(p: *mut AnyObject, _: Sel) {
            unsafe {
                super::super::factory::deallocate_owned(p, || {
                    DEALLOCATED.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }
            .unwrap();
        }
        let fixture = super::super::tests::Fixture::new();
        let parent = ClassBuilder::new(c"RuycastF1OwnedLedgerImmutableParent", NSObject::class())
            .unwrap()
            .register();
        let guard =
            unsafe { ClassGuard::select_sdk_family(&[parent], fixture.model.lease(), cleanup) }
                .unwrap();
        let proxy =
            unsafe { guard.subclass_qualified(c"RuycastF1OwnedLedgerImmutableChild", parent) }
                .unwrap();
        let grant = Arc::new(
            unsafe {
                super::super::receiver::WorkspaceGrant::for_owned_sdk(
                    fixture.model.clone(),
                    proxy,
                    guard.lifecycle_bindings(),
                )
            }
            .unwrap(),
        );
        super::super::factory::publish(grant.clone()).unwrap();
        let pointer = super::super::factory::proxy_class(proxy, &fixture.model).unwrap();
        let address = pointer as usize;
        let serial = grant.owned_reference_serial(address).unwrap();
        let origin = std::thread::current().id();
        let unknown = address.wrapping_add(1);
        assert!(grant.owned_reference_serial(unknown).is_err());
        let foreign = grant.clone();
        assert!(
            std::thread::spawn(move || foreign.verify_owned_reference(address).is_err())
                .join()
                .unwrap()
        );
        // The real factory rejects an Event/other family before marking any
        // ledger policy. This unit then exercises the explicitly unsafe ledger
        // step with its own known immutable empty payload, not a type label.
        assert!(unsafe {
            super::super::factory::publish_immutable_data(pointer, c"NSEvent", &fixture.model)
        }
        .is_err());
        unsafe { grant.publish_immutable_sdk_data(address) }.unwrap();
        let identity = grant.receiver_identity(address).unwrap();
        assert_eq!(identity.origin_thread(), origin);
        assert_eq!(identity.allocation_serial(), serial);
        let weak = objc2::rc::Weak::new(unsafe { &*pointer });
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (drop_tx, drop_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let retained =
                unsafe { super::super::factory::retain_owned(address as *mut AnyObject) }.unwrap();
            let held =
                unsafe { super::super::factory::OwnedReceiver::from_owned(retained) }.unwrap();
            assert_eq!(
                super::super::factory::owned_reference_serial(retained).unwrap(),
                serial
            );
            held_tx.send(()).unwrap();
            drop_rx.recv().unwrap();
            drop(held);
        });
        held_rx.recv().unwrap();
        unsafe { super::super::factory::release_owned(pointer) }.unwrap();
        fixture.life.0.store(false, Ordering::Release);
        drop_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(DEALLOCATED.load(Ordering::SeqCst), 1);
        assert!(weak.load().is_none());
        assert_eq!(grant.live_allocations().unwrap(), 0);
        assert!(grant.owned_reference_serial(address).is_err());
        assert!(!fixture.model.faulted.load(Ordering::Acquire));
    }
}

/// One actual source AppDelegate initializer overlay. It neither admits an SDK
/// class nor adds an existing source class to the guard-created proxy roster.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[derive(Clone)]
pub(crate) struct AppDelegateInitBinding {
    data:Arc<super::receiver::app_delegate::constructor::AppDelegateConstructionData>,
    selected:metadata::MethodSnapshot,
    provenance:metadata::ReplacementProvenance,
    initializers:Vec<ConstructorBinding>,
    retired:Arc<AtomicBool>,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl AppDelegateInitBinding {
    pub(crate) fn class(&self)->&'static CStr {self.data.class().name()}
    pub(crate) fn original(&self)->usize {self.data.original_init().implementation()}
    pub(crate) fn replacement(&self)->usize {self.selected.implementation()}
    pub(crate) fn verify_published_metadata(&self)->Result<()> {
        if self.retired.load(Ordering::Acquire) {
            return Err("AppDelegate init overlay retired".into());
        }
        metadata::verify_current_method(
            metadata::registered_class(self.data.class() as *const AnyClass as usize)?,
            c"init", false, &self.selected,
        )?;
        if self.retired.load(Ordering::Acquire) {
            return Err("AppDelegate init overlay retired during observation".into());
        }
        Ok(())
    }
    pub(crate) fn verify(&self)->Result<()> {
        self.data.context().cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire){return Err("AppDelegate init overlay retired".into());}
        let result=(||{
            self.data.verify_bodies_and_layout()?;
            for row in &self.initializers {row.verify()?;}
            if self.provenance.verify()?!=self.replacement(){return Err("AppDelegate init overlay provenance differs".into());}
            metadata::verify_current_method(metadata::registered_class(self.data.class() as *const AnyClass as usize)?,c"init",false,&self.selected)
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl ClassGuard {
    /// # Safety
    /// Exclusive pre-entry, no live source instances. The fixed declaration
    /// binds this exact genuine Main class and actual initTo + typed fields.
    pub(crate) unsafe fn overlay_app_delegate_initializer(&self,
        data:Arc<super::receiver::app_delegate::constructor::AppDelegateConstructionData>,
    )->Result<AppDelegateInitBinding> {
        self.verify()?;data.verify_original()?;
        let class=data.class();
        if !self.families.contains(&(class as *const AnyClass as usize))
            ||self.created.lock().map_err(|_|"created registry poisoned")?.contains_key(&(class as *const AnyClass as usize)) {
            return Err("AppDelegate source overlay outside original guarded family".into());
        }
        let expected=data.original_init().clone();
        type Entry=unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject;
        let entry:Entry=super::receiver::app_delegate::constructor::app_delegate_init_entry;
        use objc2::encode::{Encoding,Encode,EncodeReturn,EncodeArguments};
        use objc2::runtime::MethodImplementation;
        let mut signature=vec![<Entry as MethodImplementation>::Return::ENCODING_RETURN,Encoding::Object,Sel::ENCODING];
        signature.extend_from_slice(<Entry as MethodImplementation>::Arguments::ENCODINGS);
        verify_typed_abi(&expected,&signature)?;
        let handle=metadata::registered_class(class as *const AnyClass as usize)?;
        let before=metadata::lifecycle(handle)?;
        let encoding=std::ffi::CString::new(expected.encoding()).map_err(|_|"AppDelegate init encoding contains NUL")?;
        metadata::verify_current_method(handle,c"init",false,&expected)?;
        unsafe{class_replaceMethod(class,Sel::register(c"init"),std::mem::transmute::<usize,Imp>(entry as *const () as usize),encoding.as_ptr())};
        let selected=metadata::method(handle,c"init",false)?.ok_or("AppDelegate selected init absent")?;
        verify_typed_abi(&selected,&signature)?;
        if selected.owner()!=handle.address()||selected.implementation()!=entry as *const () as usize {
            return Err("AppDelegate selected init entry differs".into());
        }
        let provenance=metadata::ReplacementProvenance::bind(expected.clone(),expected,selected.clone(),before,false,None)?;
        let result=AppDelegateInitBinding{data,selected,provenance,initializers:self.initializers.clone(),retired:Arc::new(AtomicBool::new(false))};
        result.verify()?;Ok(result)
    }
}
