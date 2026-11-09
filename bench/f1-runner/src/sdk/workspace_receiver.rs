//! Per-class allocation and real receiver-phase contributions for the sole
//! common factory owner. No public boolean can assert Rust ivars are ready.
use super::*;
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[path="workspace_appdelegate.rs"]
pub mod app_delegate;
use crate::sdk::metadata;
use crate::sdk::{ObjcReplacement, SourceCallsite};
use objc2::{
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, Sel},
    ClassType,
};
use objc2_foundation::NSObject;
use std::ffi::CStr;
use std::sync::OnceLock;
unsafe extern "C" {
    fn class_createInstance(class: *const AnyClass, extra: usize) -> *mut AnyObject;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverPhase {
    Allocated,
    PreparingInitializer,
    InitializingOriginal,
    SourceIvarsObserved,
    Initialized,
    Retired,
    Deallocating,
}
#[derive(Clone)]
enum LifecycleProof {
    OwnedMemory(super::classes::OwnedMemoryArcBinding),
    OwnedAddition(super::classes::OwnedStringBytesAddition),
    OwnedTaskWait(super::classes::OwnedTaskWaitAddition),
    Constructor(super::classes::ConstructorBinding),
    PreservedAppKitLifecycle(metadata::MethodSnapshot),
    Leaf(ObjcReplacement),
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    AppDelegateInit(super::classes::AppDelegateInitBinding),
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    PaletteDestructor(Arc<crate::sdk::core_preentry::ReferenceAdmission>),
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    SnippetPanelDestructor(Arc<crate::sdk::core_preentry::ReferenceAdmission>, (usize, usize)),
    Source {
        snapshot: metadata::MethodSnapshot,
        source: SourceCallsite,
        authority: Arc<dyn super::delegate::CallbackCodeAuthority>,
    },
    #[cfg(test)]
    Fixture(metadata::MethodSnapshot),
}
#[derive(Clone)]
pub struct LifecycleBinding {
    pub class: &'static CStr,
    pub selector: &'static CStr,
    pub class_method: bool,
    pub original: usize,
    pub replacement: usize,
    proof: LifecycleProof,
}
impl LifecycleBinding {
    pub(crate) fn owned_memory(proof: super::classes::OwnedMemoryArcBinding) -> Self {
        Self {
            class: proof.class(),
            selector: c"retain",
            class_method: false,
            original: proof.original(),
            replacement: proof.replacement(),
            proof: LifecycleProof::OwnedMemory(proof),
        }
    }
    pub(crate) fn is_owned_memory(&self) -> bool {
        matches!(&self.proof, LifecycleProof::OwnedMemory(_))
    }
    fn memory_envelope(&self, proof: &super::classes::OwnedMemoryArcBinding) -> Result<()> {
        if self.class != proof.class()
            || self.selector != c"retain"
            || self.class_method
            || self.original != proof.original()
            || self.replacement != proof.replacement()
        {
            return Err("owned memory declaration envelope differs".into());
        }
        Ok(())
    }
    pub(crate) fn verify_owned_memory(
        &self,
        class: &objc2::runtime::AnyClass,
        root: &super::factory::RootDeallocator,
        model: &Arc<Workspace>,
    ) -> Result<()> {
        let LifecycleProof::OwnedMemory(proof) = &self.proof else {
            return Err("missing explicit owned memory proof".into());
        };
        self.memory_envelope(proof)?;
        if class.name() != proof.class() || !proof.root_matches(root) || !proof.belongs_to(model) {
            return Err("foreign owned memory class/root/model".into());
        }
        proof.verify_metadata()
    }

    /// Zero is solely this legacy envelope's absent-original value, never
    /// exported as ObjcReplacement or original-code authority.
    pub(super) fn owned_addition(proof: super::classes::OwnedStringBytesAddition) -> Self {
        Self {
            class: proof.class,
            selector: super::classes::StringBytesDeclaration::SELECTOR,
            class_method: false,
            original: 0,
            replacement: proof.implementation(),
            proof: LifecycleProof::OwnedAddition(proof),
        }
    }
    pub(super) fn owned_task_wait(proof:super::classes::OwnedTaskWaitAddition)->Self {
        Self {class:proof.class,selector:super::classes::TaskWaitDeclaration::SELECTOR,
            class_method:false,original:0,replacement:proof.implementation(),proof:LifecycleProof::OwnedTaskWait(proof)}
    }
    fn verify_cleanup_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        match &self.proof {
            LifecycleProof::PreservedAppKitLifecycle(_) => self.verify_with_roster(roster),
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            LifecycleProof::PaletteDestructor(_) | LifecycleProof::SnippetPanelDestructor(..) => self.verify_with_roster(roster),
            LifecycleProof::OwnedMemory(proof) => {
                self.memory_envelope(proof)?;
                proof.verify_metadata()
            }
            LifecycleProof::Constructor(binding) => {
                if (
                    self.class,
                    self.selector,
                    self.class_method,
                    self.original,
                    self.replacement,
                ) != (
                    binding.class,
                    binding.selector,
                    binding.class_method,
                    binding.original,
                    binding.replacement,
                ) {
                    return Err("cleanup lifecycle key differs from bound proof".into());
                }
                binding.verify_metadata_with_roster(roster)
            }
            LifecycleProof::OwnedAddition(proof) => {
                if self.class != proof.class
                    || self.selector != super::classes::StringBytesDeclaration::SELECTOR
                    || self.class_method
                    || self.original != 0
                    || self.replacement != proof.implementation()
                {
                    return Err("owned declaration cleanup envelope differs".into());
                }
                proof.verify_metadata()
            }
            LifecycleProof::OwnedTaskWait(proof)=>{
                if self.class!=proof.class||self.selector!=super::classes::TaskWaitDeclaration::SELECTOR
                    ||self.class_method||self.original!=0||self.replacement!=proof.implementation() {
                    return Err("Task wait cleanup envelope differs".into());
                }
                proof.verify_metadata()
            }
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            LifecycleProof::AppDelegateInit(_)=>Err("source init binding is not SDK cleanup".into()),
            LifecycleProof::Leaf(_) => self.verify_with_roster(roster),
            LifecycleProof::Source { .. } => {
                Err("owned SDK cleanup cannot invoke original source authority".into())
            }
            #[cfg(test)]
            LifecycleProof::Fixture(_) => {
                Err("fixture-only lifecycle identity cannot authorize SDK disposal".into())
            }
        }
    }

    pub(super) fn preserve_appkit_lifecycle(
        class: &'static AnyClass, selector: &'static CStr, snapshot: metadata::MethodSnapshot,
    ) -> Result<Self> {
        let expected = match (class.name().to_bytes(), selector.to_bytes()) {
            (b"NSView", b".cxx_destruct") => 0x7a00c,
            (b"NSButton", b".cxx_destruct") => 0x6ebb9c,
            (b"NSEvent", b".cxx_destruct") => 0x185edc,
            (b"NSResponder", b"dealloc") => 0x797fc,
            (b"NSView", b"dealloc") => 0x77b50,
            (b"NSControl", b"dealloc") => 0xd2484,
            (b"NSButton", b"dealloc") => 0x1591ac,
            (b"NSEvent", b"dealloc") => 0x185e50,
            (b"NSResponder", b"initialize") => 0x24e070,
            (b"NSView", b"initialize") => 0xd220,
            (b"NSControl", b"initialize") => 0x1d8bc,
            (b"NSButton", b"initialize") => 0x1d914,
            (b"NSStatusBarButton", b"initialize") => 0x247080,
            (b"NSEvent", b"initialize") => 0xe2f4,
            (_, b"resolveClassMethod:") => 0x10bfc,
            (_, b"resolveInstanceMethod:") => 0xfbc4,
            _ => return Err("class outside shared Reference view lifecycle".into()),
        };
        let resolver = [c"resolveClassMethod:", c"resolveInstanceMethod:"].contains(&selector);
        let class_method = resolver || selector == c"initialize";
        let (owner, encoding, image, uuid) = if resolver {
            (NSObject::class().metaclass() as *const AnyClass as usize, b"B24@0:8:16".as_slice(),
                b"/usr/lib/libobjc.A.dylib".as_slice(),
                [0x03, 0xbd, 0x9e, 0x32, 0xcf, 0x0a, 0x37, 0xb0, 0x89, 0x8a, 0x3c, 0xe8, 0xde, 0x06, 0xd8, 0x42])
        } else {
            let owner = if class_method { class.metaclass() } else { class };
            (owner as *const AnyClass as usize, b"v16@0:8".as_slice(),
                b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".as_slice(),
                [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64, 0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46])
        };
        let code = snapshot.code();
        if snapshot.owner() != owner || snapshot.encoding() != encoding
            || code.image() != image || code.offset() != expected || code.uuid() != uuid
        {
            return Err("shared Reference view lifecycle owner or entry differs".into());
        }
        let implementation = snapshot.implementation();
        Ok(Self { class: class.name(), selector, class_method,
            original: implementation, replacement: implementation, proof: LifecycleProof::PreservedAppKitLifecycle(snapshot) })
    }

    pub(super) fn matches_preserved_appkit_method(
        &self, selector: &CStr, method: &metadata::MethodSnapshot,
    ) -> bool {
        let LifecycleProof::PreservedAppKitLifecycle(snapshot) = &self.proof else { return false; };
        self.selector == selector && snapshot.owner() == method.owner()
            && snapshot.method_address() == method.method_address()
            && snapshot.implementation() == method.implementation()
            && snapshot.encoding() == method.encoding() && snapshot.code() == method.code()
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn palette_destructor(
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    ) -> Result<Self> {
        admission.verify_reference_palette_panel_metadata()?;
        let class = c"RuycastF1Reference.PalettePanel";
        let selector = c".cxx_destruct";
        let handle = metadata::loaded_class(class)?;
        let method = metadata::method(handle, selector, false)?.ok_or("Palette destructor absent")?;
        Ok(Self { class, selector, class_method: false, original: method.implementation(),
            replacement: method.implementation(), proof: LifecycleProof::PaletteDestructor(admission) })
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn snippet_panel_destructor(
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        published_metadata: (usize, usize),
    ) -> Result<Self> {
        admission.verify_reference_snippet_panel_cleanup()?;
        let class = c"RuycastF1Reference.SnippetChoicePanel";
        let selector = c".cxx_destruct";
        let handle = metadata::loaded_class(class)?;
        let method = metadata::method(handle, selector, false)?.ok_or("SnippetChoicePanel destructor absent")?;
        Ok(Self {
            class, selector, class_method: false, original: method.implementation(),
            replacement: method.implementation(), proof: LifecycleProof::SnippetPanelDestructor(admission, published_metadata),
        })
    }
    pub fn constructor(binding: super::classes::ConstructorBinding) -> Self {
        Self {
            class: binding.class,
            selector: binding.selector,
            class_method: binding.class_method,
            original: binding.original,
            replacement: binding.replacement,
            proof: LifecycleProof::Constructor(binding),
        }
    }
    pub fn leaf(binding: ObjcReplacement) -> Self {
        Self {
            class: binding.class,
            selector: binding.selector,
            class_method: binding.class_method,
            original: binding.original,
            replacement: binding.replacement,
            proof: LifecycleProof::Leaf(binding),
        }
    }
    /// # Safety
    /// The method is original OWN source code, not an unchanged framework SDK
    /// implementation relabeled as a replacement. Authority proves exact code.
    pub unsafe fn source(
        class: &'static CStr,
        selector: &'static CStr,
        class_method: bool,
        source: SourceCallsite,
        authority: Arc<dyn super::delegate::CallbackCodeAuthority>,
    ) -> Result<Self> {
        let h = metadata::loaded_class(class)?;
        let snapshot = metadata::method(h, selector, class_method)?
            .ok_or("own source lifecycle method missing")?;
        let c = unsafe { &*(h.address() as *const AnyClass) };
        authority.verify_method(
            c,
            Sel::register(selector),
            snapshot.implementation(),
            &source,
        )?;
        Ok(Self {
            class,
            selector,
            class_method,
            original: snapshot.implementation(),
            replacement: snapshot.implementation(),
            proof: LifecycleProof::Source {
                snapshot,
                source,
                authority,
            },
        })
    }
    pub fn verify(&self) -> Result<()> {
        metadata::with_verification_roster(|roster| self.verify_with_roster(roster))
    }

    pub(super) fn verify_with_roster(&self, roster: &metadata::VerificationRoster) -> Result<()> {
        // Let the owned declaration observe and permanently retire on metadata
        // failure before the generic legacy envelope performs early returns.
        if let LifecycleProof::OwnedAddition(proof) = &self.proof {
            proof.verify()?;
        }
        if let LifecycleProof::OwnedTaskWait(proof)=&self.proof {proof.verify()?;}
        if let LifecycleProof::OwnedMemory(proof) = &self.proof {
            proof.verify_metadata()?;
        }
        let h = roster.loaded_class(self.class)?;
        let current = metadata::method(h, self.selector, self.class_method)?
            .ok_or("lifecycle method disappeared")?;
        if current.implementation() != self.replacement {
            return Err("lifecycle implementation changed".into());
        }
        match &self.proof {
            LifecycleProof::PreservedAppKitLifecycle(snapshot) => {
                if &current != snapshot || self.original != self.replacement
                {
                    return Err("preserved shared view lifecycle changed".into());
                }
                Ok(())
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            LifecycleProof::PaletteDestructor(admission) => {
                if self.class != c"RuycastF1Reference.PalettePanel" || self.selector != c".cxx_destruct"
                    || self.class_method || self.original != self.replacement || current.owner() != h.address() {
                    return Err("Palette destructor binding differs".into());
                }
                admission.verify_reference_palette_panel_metadata()
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            LifecycleProof::SnippetPanelDestructor(admission, published_metadata) => {
                if self.class != c"RuycastF1Reference.SnippetChoicePanel" || self.selector != c".cxx_destruct"
                    || self.class_method || self.original != self.replacement || current.owner() != h.address() {
                    return Err("SnippetChoicePanel destructor binding differs".into());
                }
                admission.verify_reference_snippet_panel_cleanup()?;
                admission.verify_reference_snippet_model_deallocator()?;
                admission.verify_reference_snippet_published_destroy_cache(*published_metadata)
            }
            LifecycleProof::OwnedMemory(proof) => {
                self.memory_envelope(proof)?;
                proof.verify()
            }
            LifecycleProof::OwnedAddition(p) => {
                if self.class != p.class
                    || self.selector != super::classes::StringBytesDeclaration::SELECTOR
                    || self.class_method
                    || self.original != 0
                    || self.replacement != p.implementation()
                {
                    return Err("owned declaration envelope differs".into());
                }
                p.verify()
            }
            LifecycleProof::OwnedTaskWait(proof)=>{
                if self.class!=proof.class||self.selector!=super::classes::TaskWaitDeclaration::SELECTOR
                    ||self.class_method||self.original!=0||self.replacement!=proof.implementation() {
                    return Err("Task wait owned addition envelope differs".into());
                }
                proof.verify()
            }
            LifecycleProof::Constructor(p) => {
                if (
                    self.class,
                    self.selector,
                    self.class_method,
                    self.original,
                    self.replacement,
                ) != (
                    p.class,
                    p.selector,
                    p.class_method,
                    p.original,
                    p.replacement,
                ) {
                    return Err("constructor lifecycle key differs from bound proof".into());
                }
                p.verify_with_roster(roster)
            }
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            LifecycleProof::AppDelegateInit(binding)=>{
                if self.class!=binding.class()||self.selector!=c"init"||self.class_method
                    ||self.original!=binding.original()||self.replacement!=binding.replacement() {
                    return Err("AppDelegate init binding envelope differs".into());
                }
                binding.verify()
            }
            LifecycleProof::Leaf(p) => {
                if (
                    self.class,
                    self.selector,
                    self.class_method,
                    self.original,
                    self.replacement,
                ) != (
                    p.class,
                    p.selector,
                    p.class_method,
                    p.original,
                    p.replacement,
                ) || !p
                    .provenance
                    .matches_key(h, self.selector, self.class_method)
                    || p.provenance.original() != self.original
                    || p.provenance.replacement() != self.replacement
                    || p.provenance.verify_with_roster(roster)? != self.replacement
                {
                    return Err("leaf lifecycle provenance changed".into());
                }
                Ok(())
            }
            LifecycleProof::Source {
                snapshot,
                source,
                authority,
            } => {
                if &current != snapshot || self.original != snapshot.implementation() {
                    return Err("own source lifecycle identity changed".into());
                }
                authority.verify_method(
                    unsafe { &*(h.address() as *const AnyClass) },
                    Sel::register(self.selector),
                    self.replacement,
                    source,
                )
            }
            #[cfg(test)]
            LifecycleProof::Fixture(snapshot) => {
                if &current != snapshot || self.original != snapshot.implementation() {
                    return Err("own fixture method changed".into());
                }
                Ok(())
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(
        class: &'static CStr,
        selector: &'static CStr,
        class_method: bool,
    ) -> Result<Self> {
        let h = metadata::loaded_class(class)?;
        let snapshot =
            metadata::method(h, selector, class_method)?.ok_or("fixture method missing")?;
        Ok(Self {
            class,
            selector,
            class_method,
            original: snapshot.implementation(),
            replacement: snapshot.implementation(),
            proof: LifecycleProof::Fixture(snapshot),
        })
    }
}
#[derive(Clone)]
pub struct ScalarProbe {
    pub selector: &'static CStr,
    pub expected_imp: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThreadPolicy {
    Owner,
    ImmutableSdkData,
    TaskJob,
    TaskChild,
    ErrorData,
}
struct Receiver {
    phase: ReceiverPhase,
    thread_policy: ThreadPolicy,
    generation: u64,
    observed: Vec<bool>,
    serial: u64,
    owner: std::thread::ThreadId,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    app_delegate:Option<app_delegate::constructor::AppDelegateReceiverState>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    snippet_panel_model: Option<usize>,
}
pub(crate) struct OwnedDeallocation {
    receiver: usize,
    serial: u64,
    grant: u64,
    owner: std::thread::ThreadId,
}
pub struct ReceiverToken {
    receiver: usize,
    generation: u64,
    observed: Vec<bool>,
    serial: u64,
    grant: u64,
}
/// Read-only view minted from the actual allocation ledger, never from a raw
/// pointer or a caller's assertion that Rust ivars have been installed.
pub struct SourceConstructionIdentity<'a> {
    ledger: &'a WorkspaceGrant,
    model: Arc<Workspace>,
    class: &'static AnyClass,
    receiver: usize,
    grant: u64,
    serial: u64,
    generation: u64,
    owner: std::thread::ThreadId,
    phase: ReceiverPhase,
}
impl SourceConstructionIdentity<'_> {
    pub fn belongs_to(&self, model: &Arc<Workspace>) -> bool { Arc::ptr_eq(&self.model, model) }
    pub fn class(&self) -> &'static AnyClass { self.class }
    pub fn receiver(&self) -> usize { self.receiver }
    pub fn grant_serial(&self) -> u64 { self.grant }
    pub fn allocation_serial(&self) -> u64 { self.serial }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn origin_thread(&self) -> std::thread::ThreadId { self.owner }
    pub fn phase(&self) -> ReceiverPhase { self.phase }
    /// Pure ledger identity only; business retirement is deliberately separate
    /// from ownership of the already allocated construction reference.
    pub fn verify_allocation(&self) -> Result<()> {
        if self.owner != std::thread::current().id() || self.ledger.id != self.grant
            || self.ledger.class != self.class || !self.belongs_to(&self.ledger.model) {
            return Err("partial source identity owner differs".into());
        }
        let rows = self.ledger.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get(&self.receiver).ok_or("partial source identity disappeared")?;
        if row.serial != self.serial || row.generation != self.generation
            || row.owner != self.owner || row.phase != self.phase {
            return Err("partial source identity stale or changed".into());
        }
        Ok(())
    }
}
struct SourceConstructionAdmission {
    constructor_source: SourceCallsite,
    authority: Arc<dyn super::delegate::CallbackCodeAuthority>,
    model: Arc<Workspace>,
    class: &'static AnyClass,
    grant: u64,
    generation: u64,
    owner: std::thread::ThreadId,
    retired: AtomicBool,
    cleanup_retired: AtomicBool,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    app_delegate:Option<Arc<app_delegate::constructor::AppDelegateConstruction>>,
}
/// Nonowning identity from the existing observed allocation ledger. This is
/// neither an ARC grant nor proof that original dealloc/isa hooks are installed.
#[derive(Clone)]
pub struct ReceiverIdentity {
    grant: Arc<WorkspaceGrant>,
    receiver: usize,
    serial: u64,
    generation: u64,
    owner: std::thread::ThreadId,
}
impl ReceiverIdentity {
    pub fn receiver(&self) -> usize {
        self.receiver
    }
    pub fn allocation_serial(&self) -> u64 {
        self.serial
    }
    pub fn origin_thread(&self) -> std::thread::ThreadId {
        self.owner
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn class(&self) -> &'static AnyClass {
        self.grant.class
    }
    pub fn source(&self) -> &SourceCallsite {
        &self.grant.source
    }
    /// Stored grant role only; an error never selects a different ARC policy.
    pub fn original_source(&self) -> bool {
        self.grant.original_source
    }
    pub fn belongs_to(&self, model: &Arc<Workspace>) -> bool {
        Arc::ptr_eq(&self.grant.model, model)
    }
    fn verify_row(&self) -> Result<()> {
        let rows = self
            .grant
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get(&self.receiver)
            .ok_or("receiver identity no longer observed")?;
        if (self.owner != std::thread::current().id()
            && (self.grant.original_source || !matches!(row.thread_policy,ThreadPolicy::ImmutableSdkData|ThreadPolicy::ErrorData)))
            || row.phase != ReceiverPhase::Initialized
            || row.serial != self.serial
            || row.generation != self.generation
            || row.owner != self.owner
        {
            return Err("receiver identity expired or reused".into());
        }
        Ok(())
    }
    /// A retired/reused known weak target can be treated as nil without reading
    /// its old address. This is only allocation status, never ARC authority.
    pub(crate) fn allocation_is_live(&self) -> Result<bool> {
        if self.owner != std::thread::current().id() {
            return Err("weak allocation inspected on foreign owner".into());
        }
        let rows = self.grant.receivers.lock().map_err(|_| "receiver table poisoned")?;
        Ok(rows.get(&self.receiver).is_some_and(|row| row.serial == self.serial
            && row.generation == self.generation && row.owner == self.owner
            && row.phase == ReceiverPhase::Initialized))
    }
    /// Pure physical-allocation check for separately authorized cleanup. This
    /// grants no original ARC and deliberately does not require business life.
    pub fn verify_allocation(&self) -> Result<()> {
        self.verify_row()
    }
    /// Pure post-callback ledger/model check. Never dereferences the receiver.
    pub fn verify_pure(&self) -> Result<()> {
        self.verify_row()?;
        if self.grant.model.faulted.load(Ordering::Acquire) {
            return Err("receiver model authority retired".into());
        }
        let state = self
            .grant
            .model
            .state
            .lock()
            .map_err(|_| "workspace state poisoned")?;
        if state.retired || state.generation != self.generation {
            return Err("receiver identity model generation expired".into());
        }
        Ok(())
    }
    /// Business admission plus exact Initialized serial, before an operation.
    pub fn verify(&self) -> Result<()> {
        self.verify_pure()?;
        self.grant.verify()?;
        self.verify_pure()
    }
    /// Only an already qualified SDK proxy's ARC path; never original cleanup.
    /// Cleanup remains available after business retirement of the model.
    pub fn verify_sdk_cleanup(&self) -> Result<()> {
        self.verify_row()?;
        self.grant.verify_owned_cleanup()?;
        self.verify_row()
    }
}
pub struct SenderLease {
    grant: Arc<WorkspaceGrant>,
    receiver: usize,
    serial: u64,
    generation: u64,
    owner: std::thread::ThreadId,
}
impl SenderLease {
    pub fn before_effect(&self) -> Result<()> {
        self.grant.verify()?;
        let rows = self
            .grant
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let r = rows
            .get(&self.receiver)
            .ok_or("sender receiver disappeared")?;
        if r.serial != self.serial
            || r.generation != self.generation
            || r.phase != ReceiverPhase::Initialized
        {
            return Err("sender receiver retired or reused".into());
        }
        Ok(())
    }
    pub fn class(&self) -> &'static AnyClass {
        self.grant.class
    }
    pub(crate) fn pointer(&self) -> Result<*mut AnyObject> {
        self.before_effect()?;
        Ok(self.receiver as *mut AnyObject)
    }
}
impl Drop for SenderLease {
    fn drop(&mut self) {
        // Otherwise retain until process exit rather than running an unqualified
        // destructor or dropping a main-only object on a foreign thread.
        if std::thread::current().id() == self.owner
            && self.grant.verify().is_ok()
            && (!self.grant.original_source
                || crate::sdk::ruycast_f1_preentry_ready_v1() == 0xF100002600003FFF)
        {
            drop(unsafe { Retained::from_raw(self.receiver as *mut AnyObject) });
        }
    }
}
impl ReceiverToken {
    pub fn receiver(&self) -> usize {
        self.receiver
    }
    pub fn observed_scalars(&self) -> &[bool] {
        &self.observed
    }
}
pub struct WorkspaceGrant {
    owner: &'static str,
    source: SourceCallsite,
    class: &'static AnyClass,
    selected: Vec<LifecycleBinding>,
    probes: Vec<ScalarProbe>,
    model: Arc<Workspace>,
    receivers: Mutex<BTreeMap<usize, Receiver>>,
    original_source: bool,
    id: u64,
    authority: Option<Arc<dyn super::delegate::CallbackCodeAuthority>>,
    root_deallocator: Option<super::factory::RootDeallocator>,
    construction: OnceLock<SourceConstructionAdmission>,
}
fn serial() -> Result<u64> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |n| n.checked_add(1),
    )
    .map_err(|_| "receiver grant/serial counter exhausted".into())
}
impl WorkspaceGrant {
    pub(crate) fn receiver_identity(self: &Arc<Self>, receiver: usize) -> Result<ReceiverIdentity> {
        self.verify()?;
        let (serial, generation, owner) = {
            let rows = self
                .receivers
                .lock()
                .map_err(|_| "receiver table poisoned")?;
            let row = rows
                .get(&receiver)
                .ok_or("receiver allocation was not observed")?;
            if row.phase != ReceiverPhase::Initialized
                || (row.owner != std::thread::current().id()
                    && (self.original_source
                        || !matches!(row.thread_policy,ThreadPolicy::ImmutableSdkData|ThreadPolicy::ErrorData)))
            {
                return Err("receiver was not initialized for this thread policy".into());
            }
            (row.serial, row.generation, row.owner)
        };
        let identity = ReceiverIdentity {
            grant: self.clone(),
            receiver,
            serial,
            generation,
            owner,
        };
        identity.verify_pure()?;
        Ok(identity)
    }
    pub(crate) fn workspace(&self) -> &Arc<Workspace> {
        &self.model
    }
    pub(crate) fn owns_allocation(&self, receiver: usize) -> Result<bool> {
        Ok(self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?
            .contains_key(&receiver))
    }
    #[cfg(test)]
    pub(crate) fn live_allocations(&self) -> Result<usize> {
        Ok(self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?
            .len())
    }
    pub(crate) fn verify_owned_reference(&self, receiver: usize) -> Result<()> {
        self.verify_owned_cleanup()?;
        let rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows.get(&receiver).ok_or("unknown owned SDK reference")?;
        if (row.owner != std::thread::current().id()
            && !matches!(row.thread_policy, ThreadPolicy::ImmutableSdkData | ThreadPolicy::TaskJob | ThreadPolicy::TaskChild | ThreadPolicy::ErrorData))
            || row.phase == ReceiverPhase::Deallocating
        {
            return Err("foreign or deallocating owned SDK reference".into());
        }
        let checked_isa=matches!(row.thread_policy,ThreadPolicy::TaskJob|ThreadPolicy::TaskChild|ThreadPolicy::ErrorData);
        drop(rows);
        if checked_isa && unsafe {&*(receiver as *mut AnyObject)}.class()!=self.class {
            return Err("Task/Error reference physical isa changed".into());
        }
        Ok(())
    }
    pub(crate) fn verify_immutable_constant_reference(&self,receiver:usize,serial:u64)->Result<()> {
        // This eligibility check precedes every cleanup read: a source-authority
        // callback must never be hidden inside the final pure import check.
        if self.original_source || !self.probes.is_empty() || self.root_deallocator.is_none()
            || self.selected.iter().any(|row|matches!(&row.proof,LifecycleProof::Source {..})) {
            return Err("constant reference has dynamic source authority".into());
        }
        self.verify_owned_reference(receiver)?;
        let rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row=rows.get(&receiver).ok_or("constant allocation disappeared")?;
        if row.serial!=serial || row.phase!=ReceiverPhase::Initialized || row.thread_policy!=ThreadPolicy::ImmutableSdkData {
            return Err("constant immutable allocation changed".into());
        }
        Ok(())
    }
    /// Only the common factory may publish this policy, after inspecting the
    /// actual Task/Pipe/Handle payload and its exact published class grant.
    pub(crate) unsafe fn publish_task_reference(&self, receiver: usize, job: bool) -> Result<()> {
        self.verify_owned_reference(receiver)?;
        let mut rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row=rows.get_mut(&receiver).ok_or("unknown task allocation")?;
        if self.original_source || !self.probes.is_empty() || self.root_deallocator.is_none()
            || self.selected.iter().any(|row|matches!(&row.proof,LifecycleProof::Source {..}))
            || row.owner != std::thread::current().id() || row.phase != ReceiverPhase::Initialized
            || row.thread_policy != ThreadPolicy::Owner {
            return Err("task policy requires an owner-initialized SDK allocation".into());
        }
        row.thread_policy=if job {ThreadPolicy::TaskJob} else {ThreadPolicy::TaskChild};
        Ok(())
    }
    pub(crate) unsafe fn publish_error_data(&self,receiver:usize)->Result<()> {
        if self.original_source || !self.probes.is_empty() || self.root_deallocator.is_none()
            || self.selected.iter().any(|row|matches!(&row.proof,LifecycleProof::Source {..})) {
            return Err("ErrorData requires source-less owned memory proof".into());
        }
        self.verify_owned_reference(receiver)?;
        let mut rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row=rows.get_mut(&receiver).ok_or("unknown ErrorData allocation")?;
        if row.phase!=ReceiverPhase::Initialized || row.owner!=std::thread::current().id() || row.thread_policy!=ThreadPolicy::Owner {
            return Err("ErrorData publication requires exact owner initialization".into());
        }
        row.thread_policy=ThreadPolicy::ErrorData;
        Ok(())
    }
    pub(crate) fn verify_task_allocation_pure(&self,receiver:usize,serial:u64,owner:std::thread::ThreadId,job:bool)->Result<()> {
        if self.original_source {return Err("original receiver is not a Task data allocation".into());}
        let generation={
            let rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
            let row=rows.get(&receiver).ok_or("Task allocation disappeared")?;
            let policy=if job {row.thread_policy==ThreadPolicy::TaskJob} else {
                row.thread_policy==ThreadPolicy::ErrorData || (row.owner==std::thread::current().id()
                    && matches!(row.thread_policy,ThreadPolicy::TaskChild|ThreadPolicy::Owner))
            };
            if row.phase!=ReceiverPhase::Initialized || row.serial!=serial || row.owner!=owner || !policy {
                return Err("Task allocation changed during callback".into());
            }
            row.generation
        };
        let state=self.model.state.lock().map_err(|_| "workspace state poisoned")?;
        if state.retired || state.generation!=generation || self.model.faulted.load(Ordering::Acquire) {
            return Err("Task model retired during callback".into());
        }
        Ok(())
    }
    pub(crate) fn verify_task_reference(&self, receiver:usize, serial:u64, job:bool) -> Result<()> {
        self.verify_owned_reference(receiver)?;
        let rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row=rows.get(&receiver).ok_or("unknown task allocation")?;
        let policy=if job {ThreadPolicy::TaskJob} else {ThreadPolicy::TaskChild};
        if self.original_source || row.serial!=serial || row.phase!=ReceiverPhase::Initialized
            || row.thread_policy!=policy {return Err("task reference policy or serial changed".into());}
        Ok(())
    }
    /// Narrow Task status/cancellation admission. This does not return a
    /// ReceiverIdentity that could authorize arbitrary foreign-thread methods.
    pub(crate) fn verify_task_job(&self, receiver:usize, serial:u64) -> Result<()> {
        self.verify_task_reference(receiver,serial,true)?;
        self.verify()?;
        self.verify_task_reference(receiver,serial,true)?;
        let generation={
            let rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
            rows.get(&receiver).ok_or("task allocation disappeared")?.generation
        };
        let state=self.model.state.lock().map_err(|_| "workspace state poisoned")?;
        if self.model.faulted.load(Ordering::Acquire) || state.retired || state.generation!=generation {
            return Err("task business generation retired".into());
        }
        Ok(())
    }
    /// This is only the ledger step. The common factory first verifies exact
    /// role and READY immutable payload/children in the sole object arena.
    pub(crate) unsafe fn publish_immutable_sdk_data(&self, receiver: usize) -> Result<()> {
        self.verify_owned_reference(receiver)?;
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get_mut(&receiver)
            .ok_or("unknown immutable SDK reference")?;
        if self.original_source
            || row.owner != std::thread::current().id()
            || row.phase != ReceiverPhase::Initialized
        {
            return Err("only an owner-initialized SDK value can become immutable".into());
        }
        row.thread_policy = ThreadPolicy::ImmutableSdkData;
        Ok(())
    }
    pub(crate) fn is_immutable_sdk_data(&self, receiver: usize) -> Result<bool> {
        self.verify_owned_cleanup()?;
        let rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get(&receiver)
            .ok_or("unknown immutable SDK reference")?;
        Ok(!self.original_source
            && matches!(row.thread_policy,ThreadPolicy::ImmutableSdkData|ThreadPolicy::ErrorData)
            && row.phase != ReceiverPhase::Deallocating)
    }
    /// Read the actual allocation serial; a caller nonce never substitutes it.
    pub(crate) fn owned_reference_serial(&self, receiver: usize) -> Result<u64> {
        self.verify_owned_reference(receiver)?;
        let rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        Ok(rows
            .get(&receiver)
            .ok_or("unknown owned SDK reference")?
            .serial)
    }
    pub(crate) fn claim_owned_initializer(&self, receiver: usize) -> Result<()> {
        self.verify_owned_reference(receiver)?;
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get_mut(&receiver)
            .ok_or("initializer allocation disappeared")?;
        if row.owner != std::thread::current().id() || row.phase != ReceiverPhase::Allocated {
            return Err("initializer ownership already claimed or published".into());
        }
        row.phase = ReceiverPhase::PreparingInitializer;
        Ok(())
    }
    pub(crate) fn verify_owned_initializer_claim(&self, receiver: usize) -> Result<()> {
        self.verify_owned_reference(receiver)?;
        let rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get(&receiver)
            .ok_or("initializer allocation disappeared")?;
        if row.owner != std::thread::current().id()
            || row.phase != ReceiverPhase::PreparingInitializer
        {
            return Err("initializer ownership was not claimed".into());
        }
        Ok(())
    }
    /// A proxy SDK class with no original application ivars or initializers.
    /// # Safety
    /// The common coordinator independently binds every selected SDK lifecycle
    /// implementation to the owned module artifact. Actual framework parents'
    /// CXX/init/dealloc paths must all be substituted before this is called.
    pub unsafe fn for_owned_sdk(
        model: Arc<Workspace>,
        class: &'static AnyClass,
        selected: Vec<LifecycleBinding>,
    ) -> Result<Self> {
        if !class.name().to_bytes().starts_with(b"RuycastF1Owned") {
            return Err("SDK proxy is not an owned class".into());
        }
        let source = SourceCallsite {
            file: "bench/f1-runner/src/sdk/workspace_receiver.rs",
            source_sha256: {
                use sha2::{Digest, Sha256};
                format!(
                    "{:x}",
                    Sha256::digest(include_bytes!("workspace_receiver.rs"))
                )
            },
            operation: "owned SDK proxy; full lifecycle source/selection proof required",
        };
        let grant = Self {
            owner: "workspace-owned-sdk",
            source,
            class,
            selected,
            probes: vec![],
            model,
            receivers: Mutex::new(BTreeMap::new()),
            original_source: false,
            id: serial()?,
            authority: None,
            root_deallocator: Some(super::factory::RootDeallocator::capture()?),
            construction: OnceLock::new(),
        };
        grant.verify()?;
        Ok(grant)
    }
    pub fn owner(&self) -> &'static str {
        self.owner
    }
    pub(crate) fn is_original_source(&self) -> bool {
        self.original_source
    }
    pub fn source(&self) -> &SourceCallsite {
        &self.source
    }
    pub fn class(&self) -> &'static AnyClass {
        self.class
    }
    pub fn selected(&self) -> &[LifecycleBinding] {
        &self.selected
    }
    /// Admission carries the actual typed lifecycle proofs, including owned
    /// declarations. They are never recast as original-IMP replacements.
    pub(crate) fn attested_owned_leaves(&self) -> Result<Vec<ObjcReplacement>> {
        if self.original_source {
            return Err("original callback grant is not an owned factory attestation".into());
        }
        self.verify()?;
        // The owned structure check above already verified each selected
        // binding. Collect the verified leaves without another class scan.
        let mut leaves = Vec::new();
        for binding in &self.selected {
            match &binding.proof {
                LifecycleProof::Leaf(leaf) => leaves.push(leaf.clone()),
                LifecycleProof::Constructor(_)
                | LifecycleProof::PreservedAppKitLifecycle(_)
                | LifecycleProof::OwnedAddition(_)
                | LifecycleProof::OwnedTaskWait(_)
                | LifecycleProof::OwnedMemory(_) => (),
                _ => return Err("non-owned lifecycle proof in factory attestation".into()),
            }
        }
        Ok(leaves)
    }
    pub(crate) fn attested_original_callbacks(
        &self,
        catalog: &dyn super::objc::application::CallbackCatalog,
    ) -> Result<()> {
        if !self.original_source {
            return Err("callback attestation requires an original source grant".into());
        }
        self.verify()?;
        let (source, methods) = catalog.class_binding(self.class)?;
        if source != self.source || !super::source_roster().contains(&source)
            || methods.is_empty() {
            return Err("original callback source/roster is not bound".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for &(selector, implementation) in &methods {
            if implementation == 0 || !seen.insert(selector) {
                return Err("missing/duplicate callback method identity".into());
            }
            let selected = super::raw_method(self.class, Sel::register(selector), false)?
                .ok_or("attested original callback method missing")?;
            if selected.implementation() != implementation {
                return Err("attested original callback changed".into());
            }
        }
        catalog.verify_methods(self.class, &methods, &source)?;
        catalog.verify_cleanup(self.class, &source)?;
        self.verify()
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_reference_edit_construction(&self) -> bool {
        self.original_source && self.class.name() == c"RuycastF1Reference.NativeEditCommands"
            && self.source == super::factory::reference_edit_construction_callsite()
    }

    pub(crate) fn is_reference_palette_construction(&self) -> bool {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.original_source && self.class.name() == c"RuycastF1Reference.PalettePanel"
            && self.source == super::factory::reference_palette_construction_callsite()
            && self.selected.iter().any(|row| matches!(row.proof, LifecycleProof::PaletteDestructor(_)))
        {
            return true;
        }
        false
    }

    pub(crate) fn is_reference_snippet_panel_construction(&self) -> bool {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.original_source && self.class.name() == c"RuycastF1Reference.SnippetChoicePanel"
            && self.source == super::factory::reference_snippet_panel_construction_callsite()
            && self.selected.iter().any(|row| matches!(row.proof, LifecycleProof::SnippetPanelDestructor(..)))
        {
            return true;
        }
        false
    }

    fn is_reference_panel_construction(&self) -> bool {
        self.is_reference_palette_construction() || self.is_reference_snippet_panel_construction()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn snippet_panel_cleanup(&self) -> Result<Option<(&crate::sdk::core_preentry::ReferenceAdmission, usize)>> {
        if !self.is_reference_snippet_panel_construction() {
            return Ok(None);
        }
        let admission = self.selected.iter().find_map(|binding| match &binding.proof {
            LifecycleProof::SnippetPanelDestructor(admission, _) => Some(admission),
            _ => None,
        }).ok_or("Reference SnippetChoicePanel cleanup authority absent")?;
        Ok(Some((admission.as_ref(), admission.verify_reference_snippet_panel_cleanup()?)))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn observe_reference_construction(&self) -> Result<()> {
        let proof = self.construction.get().ok_or("Reference construction grant absent")?;
        if !(self.is_reference_edit_construction() || self.is_reference_panel_construction()) || proof.app_delegate.is_some()
            || proof.class != self.class || proof.grant != self.id
            || proof.constructor_source != self.source || !Arc::ptr_eq(&proof.model, &self.model)
            || !self.authority.as_ref().is_some_and(|authority| Arc::ptr_eq(authority, &proof.authority))
            || proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire)
        {
            return Err("Reference published construction differs or retired".into());
        }
        self.verify_construction_state(proof.generation)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn observe_published_callbacks(
        &self, catalog: &dyn super::objc::application::CallbackCatalog,
    ) -> Result<()> {
        let proof = self.construction.get().ok_or("published callback construction absent")?;
        let current = || -> Result<()> {
            if !self.original_source
                || proof.class != self.class || proof.grant != self.id
                || !Arc::ptr_eq(&proof.model, &self.model)
                || !self.authority.as_ref().is_some_and(|authority| Arc::ptr_eq(authority, &proof.authority))
                || proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire)
            {
                return Err("published callback construction changed".into());
            }
            proof.app_delegate.as_ref().ok_or("published AppDelegate declaration absent")?.verify_published_epoch()?;
            self.verify_construction_state(proof.generation)
        };
        current()?;
        // Initializers first, before observing any instance lifecycle entries.
        for row in &self.selected {
            if let LifecycleProof::Constructor(binding) = &row.proof {
                binding.verify()?;
            }
        }
        catalog.verify_published_class(&self.model, self.class, &self.source)?;
        metadata::with_verification_roster(|roster| {
            for row in &self.selected {
                match &row.proof {
                    LifecycleProof::Constructor(_) => (),
                    LifecycleProof::AppDelegateInit(binding) => binding.verify_published_metadata()?,
                    LifecycleProof::Source { snapshot, .. } => {
                        let class = roster.loaded_class(row.class)?;
                        if metadata::method(class, row.selector, row.class_method)?.as_ref() != Some(snapshot) {
                            return Err("published original lifecycle method changed".into());
                        }
                    }
                    _ => return Err("unexpected lifecycle entry in published callbacks".into()),
                }
            }
            Ok(())
        })?;
        current()
    }
    /// Immutable shape only. Callers must verify the owning grant separately.
    pub(crate) fn has_initialize_binding(&self, role: &CStr) -> bool {
        let constructor = |class: &CStr| self.selected.iter().find(|binding| binding.class == class
            && binding.selector == c"initialize" && binding.class_method
            && matches!(&binding.proof, LifecycleProof::Constructor(_)));
        if constructor(role).is_some() {
            return true;
        }
        if role != c"NSEvent" || self.original_source
            || self.class.name() != c"RuycastF1OwnedEventSource"
            || self.class.superclass().map(|class| class.name()) != Some(role)
        {
            return false;
        }
        let Some(proxy) = constructor(self.class.name()) else { return false; };
        self.selected.iter().any(|binding| binding.class == role
            && binding.selector == c"initialize" && binding.class_method
            && binding.replacement == proxy.original
            && matches!(&binding.proof, LifecycleProof::PreservedAppKitLifecycle(_)))
    }
    /// # Safety
    /// Exact class, original/generated methods and selected SDK IMPs are bound to
    /// independently verified loaded artifacts by the common factory. selected
    /// includes every local initialize/init/CXX/dealloc route of the family. No
    /// original object is loaded/initialized to discover missing coverage.
    pub unsafe fn for_original(
        model: Arc<Workspace>,
        class: &'static AnyClass,
        source: SourceCallsite,
        selected: Vec<LifecycleBinding>,
        probes: Vec<ScalarProbe>,
        authority: Arc<dyn super::delegate::CallbackCodeAuthority>,
    ) -> Result<Self> {
        if !source_roster()
            .iter()
            .any(|s| s.file == source.file && s.source_sha256 == source.source_sha256)
            && !super::source_allocation_callsites().contains(&source)
        {
            return Err("receiver source binding differs".into());
        }
        let mut grant = Self {
            owner: "workspace-native-shell",
            source,
            class,
            selected,
            probes,
            model,
            receivers: Mutex::new(BTreeMap::new()),
            original_source: true,
            id: serial()?,
            authority: Some(authority),
            root_deallocator: None,
            construction: OnceLock::new(),
        };
        if grant.is_reference_panel_construction() {
            grant.root_deallocator = Some(super::factory::RootDeallocator::capture()?);
        }
        grant.verify()?;
        Ok(grant)
    }
    /// Publish only construction permission from the existing actual source
    /// authority. No metadata observation, roster row or ready flag can mint it.
    /// # Safety
    /// The authority binds the genuine compiler caller and inert allocation /
    /// partial-destruction code, with exclusive setup and no class mutation.
    pub unsafe fn admit_source_construction(&self) -> Result<()> {
        unsafe {self.admit_source_construction_at(self.source.clone())}
    }
    /// Keep the ordinary callback declaration separate from its actual source
    /// allocation/init caller. Both remain bound to this exact class/model and
    /// the same genuine code issuer; a roster row never supplies that proof.
    /// # Safety
    /// Same as admit_source_construction, for the separately compiler-qualified
    /// caller. Probe/layout qualification is unchanged and remains mandatory.
    pub unsafe fn admit_source_construction_at(&self,constructor_source:SourceCallsite)->Result<()> {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let exact_edit = self.class.name() == c"RuycastF1Reference.NativeEditCommands"
            && constructor_source == super::factory::reference_edit_construction_callsite();
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let exact_edit = false;
        let exact_panel = self.is_reference_panel_construction() && constructor_source == self.source;
        if !self.original_source || (self.probes.is_empty() && !(exact_edit || exact_panel))
            || !super::source_allocation_callsites().contains(&constructor_source) {
            return Err("source construction caller is not qualified".into());
        }
        let generation = self.model.with(|state| Ok(state.generation))?;
        self.verify()?;
        let authority=self.authority.as_ref().ok_or("source construction authority absent")?.clone();
        authority.verify_source_construction(self.class,&constructor_source)?;
        self.verify_structure(LifecycleBinding::verify_with_roster, false)?;
        self.verify_construction_state(generation)?;
        self.construction.set(SourceConstructionAdmission {
            constructor_source,authority,
            model: self.model.clone(), class: self.class, grant: self.id,
            generation, owner: std::thread::current().id(),
            retired: AtomicBool::new(false),
            cleanup_retired: AtomicBool::new(false),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            app_delegate:None,
        }).map_err(|_| "source construction already admitted".into())
    }
    fn verify_construction_state(&self, generation: u64) -> Result<()> {
        let state = self.model.state.lock().map_err(|_| "workspace poisoned")?;
        if state.retired || state.generation != generation
            || self.model.faulted.load(Ordering::Acquire) {
            return Err("source construction model retired or changed".into());
        }
        Ok(())
    }
    fn verify_source_construction(&self) -> Result<u64> {
        let proof = self.construction.get().ok_or("source construction not admitted")?;
        // A foreign caller has no construction authority and cannot revoke
        // the owner's live proof by attempting to verify it.
        if proof.owner != std::thread::current().id() {
            return Err("source construction on foreign thread".into());
        }
        if proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire) {
            return Err("source construction admission retired".into());
        }
        let result = (|| {
        if !self.original_source || proof.class != self.class || proof.grant != self.id
            || !Arc::ptr_eq(&proof.model, &self.model)
            || !self.authority.as_ref().is_some_and(|authority|Arc::ptr_eq(authority,&proof.authority)) {
            return Err("source construction identity differs".into());
        }
        self.verify_construction_state(proof.generation)?;
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(declaration)=&proof.app_delegate {
            declaration.verify_construction()?;
        } else {
            proof.authority.verify_source_construction(self.class,&proof.constructor_source)
                .inspect_err(|_| { proof.cleanup_retired.store(true, Ordering::Release); })?;
        }
        #[cfg(not(all(rcf1_reference_sdk,not(test),target_arch="aarch64")))]
        proof.authority.verify_source_construction(self.class,&proof.constructor_source)
            .inspect_err(|_| { proof.cleanup_retired.store(true, Ordering::Release); })?;
        if proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire) {
            return Err("source construction revoked during source authority".into());
        }
        self.verify_structure(LifecycleBinding::verify_with_roster, false)
            .inspect_err(|_| { proof.cleanup_retired.store(true, Ordering::Release); })?;
        if proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire) {
            return Err("source construction revoked during lifecycle verification".into());
        }
        self.verify_construction_state(proof.generation)?;
        Ok(proof.generation)
        })();
        if result.is_err() { proof.retired.store(true, Ordering::Release); }
        result
    }
    fn verify_source_construction_pure(&self, generation: u64) -> Result<()> {
        let proof=self.construction.get().ok_or("source construction not admitted")?;
        if proof.retired.load(Ordering::Acquire) || proof.cleanup_retired.load(Ordering::Acquire)
            || proof.generation != generation || proof.grant != self.id
            || proof.class != self.class || !Arc::ptr_eq(&proof.model,&self.model)
            || proof.owner != std::thread::current().id()
            || !self.authority.as_ref().is_some_and(|authority|Arc::ptr_eq(authority,&proof.authority)) {
            return Err("source construction revoked or changed".into());
        }
        self.verify_construction_state(generation)
    }
    pub fn verify(&self) -> Result<()> {
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if self.construction.get().is_some_and(|proof|proof.app_delegate.is_some()) {
            // Typed constructor claims its reentry guard before the first
            // dynamic model callback; the ordinary scalar route is unchanged.
            return self.verify_source_construction().map(|_|());
        }
        self.model.with(|_| Ok(()))?;
        if self.original_source && self.construction.get().is_some() {
            // An admitted construction proof must observe metadata failures
            // on every existing grant-verification entry, not only allocate's
            // final check. Restoration cannot rearm a revoked permission.
            self.verify_source_construction().map(|_| ())
        } else {
            self.verify_structure(LifecycleBinding::verify_with_roster, false)
        }
    }
    /// Disposal must not call a guardian or business callback after retirement.
    /// Only the exact Reference panels may use this path with their original, pinned
    /// compiler-generated destructor. Other grants require SDK-only lifecycle.
    pub(crate) fn verify_owned_cleanup(&self) -> Result<()> {
        if (self.original_source && !self.is_reference_panel_construction()) || !self.probes.is_empty() {
            return Err("cleanup requires a qualified SDK proxy or exact Reference panel grant".into());
        }
        self.root_deallocator
            .as_ref()
            .ok_or("owned SDK memory-root proof missing")?
            .verify()
            .inspect_err(|_| {
                self.model.fault();
            })?;
        let dealloc = super::raw_method(self.class, objc2::sel!(dealloc), false)?
            .ok_or("owned SDK deallocation leaf missing")?;
        let mut selected_dealloc = false;
        for binding in &self.selected {
            if !binding.class_method
                && binding.selector == c"dealloc"
                && !matches!(&binding.proof, LifecycleProof::PreservedAppKitLifecycle(_))
                && binding.replacement == dealloc.implementation()
                && metadata::loaded_class(binding.class)?.address() == dealloc.owner()
            {
                selected_dealloc = true;
                break;
            }
        }
        if !selected_dealloc {
            return Err("normal SDK ARC requires a selected typed deallocator".into());
        }
        self.verify_structure(LifecycleBinding::verify_cleanup_with_roster, true)
    }
    fn verify_structure(&self, verify_binding: fn(&LifecycleBinding, &metadata::VerificationRoster) -> Result<()>, cleanup: bool) -> Result<()> {
        if self.original_source && self.selected.iter().any(LifecycleBinding::is_owned_memory) {
            return Err("owned memory proof cannot qualify an original receiver".into());
        }
        if let Some(root) = &self.root_deallocator {
            root.verify_proxy_arc_with_bindings(self.class, &self.selected, &self.model)
                .inspect_err(|_| {
                    self.model.fault();
                })?;
        }
        if self.selected.is_empty() {
            return Err("missing receiver constructor/destructor identities".into());
        }
        // Even an instance-method metadata lookup can initialize an ancestor.
        // Validate the complete initializer family with class-method lookups
        // FIRST; only then may lifecycle/probe instance metadata be inspected.
        let mut family = BTreeMap::new();
        let mut current = Some(self.class);
        while let Some(class) = current {
            if class == NSObject::class() {
                break;
            }
            family.insert(class.name(), class);
            current = class.superclass();
        }
        let mut keys = std::collections::BTreeSet::new();
        for row in &self.selected {
            if !family.contains_key(row.class)
                || !keys.insert((row.class, row.selector, row.class_method))
            {
                return Err("foreign/duplicate receiver lifecycle identity".into());
            }
            if row.class_method && row.selector == c"initialize" {
                let actual = super::raw_method(family[row.class], objc2::sel!(initialize), true)?
                    .ok_or("initializer disappeared")?
                    .implementation() as usize;
                if actual != row.replacement {
                    return Err("receiver initializer IMP changed".into());
                }
            }
        }
        for class in family.values() {
            let actual = super::raw_method(class, objc2::sel!(initialize), true)?
                .map(|m| m.implementation());
            let mut ancestor = Some(*class);
            let mut selected = false;
            while let Some(parent) = ancestor {
                if let Some(row) = self.selected.iter().find(|r| {
                    r.class_method && r.class == parent.name() && r.selector == c"initialize"
                }) {
                    selected = actual == Some(row.replacement);
                    break;
                }
                ancestor = parent.superclass();
            }
            if !selected {
                return Err(
                    "receiver initializer must be selected before instance metadata".into(),
                );
            }
        }
        // Constructors and selected leaves only observe metadata. Every
        // other binding gets a fresh roster, including any source callbacks.
        let metadata_only = |row: &LifecycleBinding| {
            matches!(&row.proof, LifecycleProof::Constructor(_) | LifecycleProof::Leaf(_))
        };
        for rows in self.selected.chunk_by(|left, right| metadata_only(left) && metadata_only(right)) {
            metadata::with_verification_roster(|roster| -> Result<()> {
                for row in rows {
                    verify_binding(row, roster)?;
                    if self.original_source && !cleanup {
                        if let Some(proof)=self.construction.get() {
                            self.verify_source_construction_pure(proof.generation)?;
                        }
                    }
                    let class = family[row.class];
                    let method = super::raw_method(class, Sel::register(row.selector), row.class_method)?
                        .ok_or("receiver lifecycle leaf disappeared")?;
                    if row.replacement == 0 || method.implementation() as usize != row.replacement {
                        return Err("receiver lifecycle IMP changed".into());
                    }
                }
                Ok(())
            })?;
        }
        // Check each ancestor-local CXX/dealloc implementation, not just an inherited
        // method on the child. NSObject memory operations terminate this roster.
        let mut current = Some(self.class);
        while let Some(class) = current {
            if class == NSObject::class() {
                break;
            }
            for method in class.instance_methods().iter() {
                let name = method.name();
                if [
                    Sel::register(c".cxx_construct"),
                    Sel::register(c".cxx_destruct"),
                    objc2::sel!(dealloc),
                ]
                .contains(&name)
                    && !self.selected.iter().any(|r| {
                        !r.class_method
                            && r.class == class.name()
                            && Sel::register(r.selector) == name
                            && r.replacement == method.implementation() as usize
                    })
                {
                    return Err("ancestor-local ARC/CXX lifecycle proof missing".into());
                }
            }
            let actual_initializer = super::raw_method(class, objc2::sel!(initialize), true)?
                .map(|m| m.implementation());
            let mut owner = Some(class);
            let mut inherited_selected = false;
            while let Some(candidate) = owner {
                if let Some(row) = self.selected.iter().find(|r| {
                    r.class_method && r.class == candidate.name() && r.selector == c"initialize"
                }) {
                    inherited_selected = actual_initializer == Some(row.replacement);
                    break;
                }
                owner = candidate.superclass();
            }
            if !inherited_selected {
                return Err("ancestor initialization identity missing".into());
            }
            current = class.superclass();
        }
        for probe in &self.probes {
            let m = super::raw_method(self.class, Sel::register(probe.selector), false)?
                .ok_or("source ivar probe missing")?;
            if m.implementation() as usize != probe.expected_imp
                || ![vec![b'B', b'@', b':'], vec![b'c', b'@', b':']]
                    .contains(&super::simple_abi(&m)?)
            {
                return Err("source ivar probe ABI/identity changed".into());
            }
            if self.original_source {
                self.authority
                    .as_ref()
                    .ok_or("receiver source-code authority missing")?
                    .verify_method(
                        self.class,
                        Sel::register(probe.selector),
                        probe.expected_imp,
                        &self.source,
                    )?;
                if let Some(proof)=self.construction.get() {
                    self.verify_source_construction_pure(proof.generation)?;
                }
            }
        }
        Ok(())
    }
    /// The global alloc interceptor invokes this exact callback only for this
    /// source/class grant. Raw runtime allocation can execute CXX constructors,
    /// hence the complete ancestor verification above is mandatory first.
    /// # Safety
    /// All selected non-source SDK lifecycle pointers are actual owned substitutes;
    /// no original SDK implementation is falsely described as a replacement.
    pub unsafe fn allocate(&self) -> Result<*mut AnyObject> {
        self.verify()?;
        let source_generation = if self.original_source
            && crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            Some(self.verify_source_construction()?)
        } else { None };
        let owned_generation = if let Some(generation) = source_generation {
            Some(generation)
        } else if self.root_deallocator.is_some() {
            self.verify_owned_cleanup()?;
            Some(self.model.with(|state| Ok(state.generation))?)
        } else {
            None
        };
        let p = unsafe { class_createInstance(self.class, 0) };
        if p.is_null() {
            return Err("owned receiver allocation failed".into());
        }
        if let Some(generation) = owned_generation {
            // Record the actual new +1 before another dynamic guardian check:
            // even a reentrant retirement must leave a provable cleanup owner.
            unsafe { self.record_allocated(p, generation) }?;
            if let Err(error) = self.verify() {
                if self.original_source {
                    unsafe { self.cleanup_source_partial(p) }?;
                } else {
                    let ticket = self.begin_owned_deallocation(p)?;
                    // Nothing has published or initialized this private allocation.
                    unsafe { self.finish_owned_deallocation(ticket) }?;
                }
                return Err(error);
            }
        } else {
            unsafe { self.track_allocated(p) }?;
        }
        Ok(p)
    }
    /// Observation hook for the sole allocator interceptor after its real
    /// source-qualified allocation and BEFORE the caller's set_ivars. It does
    /// not assert initialization or skip constructor/destructor verification.
    /// # Safety
    /// p is a new live +1 receiver returned by this exact class's approved
    /// allocation route. The caller proves no original SDK constructor ran.
    pub unsafe fn track_allocated(&self, p: *mut AnyObject) -> Result<()> {
        self.verify()?;
        let generation = if self.original_source
            && crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            self.verify_source_construction()?
        } else { self.model.with(|s| Ok(s.generation))? };
        unsafe { self.record_allocated(p, generation) }
    }
    fn partial_source_identity(&self, receiver: usize) -> Result<SourceConstructionIdentity<'_>> {
        if !self.original_source { return Err("partial source cleanup requires an original grant".into()); }
        let rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get(&receiver).ok_or("unknown partial source allocation")?;
        if row.owner != std::thread::current().id()
            || !matches!(row.phase, ReceiverPhase::Allocated | ReceiverPhase::SourceIvarsObserved) {
            return Err("partial source allocation owner or phase differs".into());
        }
        Ok(SourceConstructionIdentity { ledger: self, model: self.model.clone(), class: self.class,
            receiver, grant: self.id, serial: row.serial, generation: row.generation,
            owner: row.owner, phase: row.phase })
    }
    /// Release only a genuinely tracked partial +1 through its separately
    /// qualified source ARC/drop-flag path. Business retirement is not cleanup
    /// authority, and no initialized receiver identity is fabricated here.
    /// # Safety
    /// The caller transfers the construction +1 exactly once; set_ivars has
    /// either not begun, or the actual selected source probe point was observed.
    pub(crate) unsafe fn cleanup_source_partial(&self, p: *mut AnyObject) -> Result<()> {
        let identity = self.partial_source_identity(p as usize)?;
        let proof = self.construction.get().ok_or("partial source construction proof absent")?;
        if proof.cleanup_retired.load(Ordering::Acquire) {
            return Err("partial source cleanup metadata retired".into());
        }
        if !self.authority.as_ref().is_some_and(|authority|Arc::ptr_eq(authority,&proof.authority)) {
            return Err("partial cleanup code issuer differs".into());
        }
        proof.authority.verify_source_partial_cleanup(&identity,&proof.constructor_source)
            .inspect_err(|_| { proof.cleanup_retired.store(true, Ordering::Release); })?;
        if proof.cleanup_retired.load(Ordering::Acquire) {
            return Err("partial source cleanup revoked during authority".into());
        }
        // Callbacks may retire the model, but cannot replace this allocation or
        // change its phase. Check the ledger again after the authority returns.
        let current = self.partial_source_identity(p as usize)?;
        if current.grant != identity.grant || current.serial != identity.serial
            || current.generation != identity.generation || current.phase != identity.phase
            || current.class != identity.class || !Arc::ptr_eq(&current.model, &identity.model) {
            return Err("partial source cleanup allocation changed".into());
        }
        if p.is_null() || unsafe { &*p }.class() != identity.class {
            return Err("partial source cleanup actual class differs".into());
        }
        if self.is_reference_panel_construction() {
            // The selected window dealloc owns the same ledger transition.
            // Do not mark the row twice or invoke the Swift destructor directly.
            self.verify_owned_cleanup()?;
            drop(unsafe { Retained::from_raw(p) });
            if self.owns_allocation(p as usize)? {
                return Err("partial panel deallocation did not retire its ledger row".into());
            }
            return Ok(());
        }
        {
            let mut rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
            let row = rows.get_mut(&(p as usize)).ok_or("partial source allocation disappeared")?;
            if row.serial != identity.serial || row.phase != identity.phase {
                return Err("partial source cleanup replay".into());
            }
            row.phase = ReceiverPhase::Deallocating;
        }
        // No receiver lock is held while source dealloc/drop glue executes.
        drop(unsafe { Retained::from_raw(p) });
        let mut rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get(&(p as usize)).ok_or("partial source cleanup ledger disappeared")?;
        if row.serial != identity.serial || row.phase != ReceiverPhase::Deallocating {
            return Err("partial source cleanup ledger changed".into());
        }
        rows.remove(&(p as usize));
        Ok(())
    }
    unsafe fn record_allocated(&self, p: *mut AnyObject, generation: u64) -> Result<()> {
        if p.is_null() || unsafe { &*p }.class() != self.class {
            return Err("allocation receiver/class differs".into());
        }
        let serial = serial()?;
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        if rows
            .get(&(p as usize))
            .is_some_and(|r| r.phase != ReceiverPhase::Retired)
        {
            return Err("allocation observation replay".into());
        }
        rows.insert(
            p as usize,
            Receiver {
                phase: ReceiverPhase::Allocated,
                thread_policy: ThreadPolicy::Owner,
                generation,
                observed: vec![],
                serial,
                owner: std::thread::current().id(),
                #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
                app_delegate:None,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                snippet_panel_model: None,
            },
        );
        Ok(())
    }
    /// Record the original model borrow before Swift writes its strong ivar.
    /// The caller proves one of the two exact allocator sites and the live
    /// Swift model. This records identity, with no additional retain.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn bind_snippet_panel_model(
        &self, receiver: *mut AnyObject, model: usize,
    ) -> Result<()> {
        if !self.is_reference_snippet_panel_construction() || model == 0 {
            return Err("Reference SnippetChoicePanel model grant differs".into());
        }
        let generation = self.verify_source_construction()?;
        let mut rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get_mut(&(receiver as usize)).ok_or("Reference SnippetChoicePanel allocation absent")?;
        if row.phase != ReceiverPhase::Allocated || row.generation != generation
            || row.owner != std::thread::current().id() || row.snippet_panel_model.is_some()
        {
            return Err("Reference SnippetChoicePanel model allocation changed".into());
        }
        row.snippet_panel_model = Some(model);
        Ok(())
    }

    /// Read only after the original super-init route claims the exact receiver.
    /// The matching token was minted by before_source_super_init.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn snippet_panel_model(&self, token: &ReceiverToken) -> Result<usize> {
        if !self.is_reference_snippet_panel_construction() || token.grant != self.id {
            return Err("Reference SnippetChoicePanel initializer grant differs".into());
        }
        self.verify_source_construction_pure(token.generation)?;
        let rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get(&token.receiver).ok_or("Reference SnippetChoicePanel initializer receiver absent")?;
        if row.phase != ReceiverPhase::SourceIvarsObserved || row.serial != token.serial
            || row.generation != token.generation || row.owner != std::thread::current().id()
        {
            return Err("Reference SnippetChoicePanel initializer token changed".into());
        }
        row.snippet_panel_model.ok_or_else(|| "Reference SnippetChoicePanel has no allocator model".into())
    }

    /// Called by the selected superclass init IMP on the REAL receiver, after the
    /// frozen native caller's alloc->set_ivars->super-init sequence. A source probe
    /// executes the real selected pure scalar getter reading those ivars. This is
    /// not an invented caller hook or an ivarsReady assertion.
    /// # Safety
    /// Only the exact source-qualified super-init callsite may invoke a probe; an
    /// arbitrary pre-set_ivars raw pointer is not a valid Rust reference.
    pub unsafe fn before_super_init(&self, p: *mut AnyObject) -> Result<ReceiverToken> {
        if self.original_source {
            return unsafe { self.before_source_super_init(p) };
        }
        self.verify()?;
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let r = rows
            .get_mut(&(p as usize))
            .ok_or("unowned receiver before init")?;
        let allocated = r.phase == ReceiverPhase::Allocated;
        let preparing_owned = r.phase == ReceiverPhase::PreparingInitializer
            && self.root_deallocator.is_some()
            && !self.original_source
            && self.probes.is_empty();
        if !(allocated || preparing_owned) || unsafe { &*p }.class() != self.class {
            return Err("receiver phase/class differs".into());
        }
        let mut observed = vec![];
        for probe in &self.probes {
            let call: unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool =
                unsafe { std::mem::transmute(probe.expected_imp) };
            observed.push(unsafe { call(p, Sel::register(probe.selector)) }.as_bool());
        }
        r.observed = observed.clone();
        r.phase = ReceiverPhase::SourceIvarsObserved;
        Ok(ReceiverToken {
            receiver: p as usize,
            generation: r.generation,
            observed,
            serial: r.serial,
            grant: self.id,
        })
    }
    unsafe fn before_source_super_init(&self, p: *mut AnyObject) -> Result<ReceiverToken> {
        self.verify()?;
        let generation = self.verify_source_construction()?;
        let serial = {
            let mut rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
            let row = rows.get_mut(&(p as usize)).ok_or("unobserved source allocation")?;
            if row.phase != ReceiverPhase::Allocated || row.generation != generation
                || row.owner != std::thread::current().id()
                || unsafe { &*p }.class() != self.class {
                return Err("source super-init allocation or phase differs".into());
            }
            // This is the actual source set_ivars -> selected-super-init point,
            // not a general-purpose ivars-ready setter. Claim before probes so
            // reentry cannot observe another valid Allocated transition.
            row.phase = ReceiverPhase::SourceIvarsObserved;
            row.serial
        };
        let mut observed = vec![];
        for probe in &self.probes {
            let call: unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool =
                unsafe { std::mem::transmute(probe.expected_imp) };
            observed.push(unsafe { call(p, Sel::register(probe.selector)) }.as_bool());
            self.verify_source_construction_pure(generation)?;
            let rows=self.receivers.lock().map_err(|_| "receiver table poisoned")?;
            let row=rows.get(&(p as usize)).ok_or("source allocation disappeared during probe")?;
            if row.serial != serial || row.generation != generation
                || row.owner != std::thread::current().id()
                || row.phase != ReceiverPhase::SourceIvarsObserved {
                return Err("source allocation changed during probe".into());
            }
        }
        // No map lock is held across source probes or authority callbacks.
        self.verify_source_construction_pure(generation)?;
        self.verify()?;
        self.verify_source_construction()?;
        let mut rows = self.receivers.lock().map_err(|_| "receiver table poisoned")?;
        let row = rows.get_mut(&(p as usize)).ok_or("source allocation disappeared during probes")?;
        if row.serial != serial || row.generation != generation
            || row.owner != std::thread::current().id()
            || row.phase != ReceiverPhase::SourceIvarsObserved {
            return Err("source super-init changed during probes".into());
        }
        row.observed = observed.clone();
        Ok(ReceiverToken { receiver: p as usize, generation, observed, serial, grant: self.id })
    }
    pub fn initialized(&self, token: ReceiverToken) -> Result<()> {
        self.verify()?;
        if self.original_source {
            self.verify_source_construction()?;
        }
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let r = rows
            .get_mut(&token.receiver)
            .ok_or("receiver disappeared")?;
        if token.grant != self.id
            || r.serial != token.serial
            || r.generation != token.generation
            || r.phase != ReceiverPhase::SourceIvarsObserved
            || r.observed != token.observed
            || (self.original_source && r.owner != std::thread::current().id())
        {
            return Err("receiver token stale/replayed".into());
        }
        r.phase = ReceiverPhase::Initialized;
        Ok(())
    }
    /// # Safety
    /// Qualified ARC and complete destructor paths stay selected through all
    /// pending callbacks; caller holds the construction reference during retain.
    pub unsafe fn sender(self: &Arc<Self>, p: *mut AnyObject) -> Result<Arc<SenderLease>> {
        self.verify()?;
        let rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let r = rows.get(&(p as usize)).ok_or("unowned control sender")?;
        if r.phase != ReceiverPhase::Initialized {
            return Err("control sender not initialized".into());
        }
        let held = unsafe { Retained::retain(p) }.ok_or("control sender retain failed")?;
        Ok(Arc::new(SenderLease {
            grant: self.clone(),
            receiver: Retained::into_raw(held) as usize,
            serial: r.serial,
            generation: r.generation,
            owner: std::thread::current().id(),
        }))
    }
    pub fn retire(&self, p: *mut AnyObject) -> Result<()> {
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let r = rows
            .get_mut(&(p as usize))
            .ok_or("unknown receiver retirement")?;
        if r.phase == ReceiverPhase::Retired {
            return Err("receiver already retired".into());
        }
        r.phase = ReceiverPhase::Retired;
        Ok(())
    }
    pub(crate) fn begin_owned_deallocation(&self, p: *mut AnyObject) -> Result<OwnedDeallocation> {
        self.verify_owned_cleanup()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let snippet_cleanup = self.snippet_panel_cleanup()?;
        let mut rows = self
            .receivers
            .lock()
            .map_err(|_| "receiver table poisoned")?;
        let row = rows
            .get_mut(&(p as usize))
            .ok_or("unowned SDK deallocation")?;
        if (row.owner != std::thread::current().id()
            && !matches!(row.thread_policy, ThreadPolicy::ImmutableSdkData | ThreadPolicy::TaskJob | ThreadPolicy::TaskChild | ThreadPolicy::ErrorData))
            || row.phase == ReceiverPhase::Deallocating
        {
            return Err("foreign or repeated SDK deallocation".into());
        }
        if self.is_reference_panel_construction() && unsafe { &*p }.class() != self.class {
            return Err("Reference panel deallocation receiver class differs".into());
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some((admission, offset)) = snippet_cleanup {
            let slot = (p as usize).checked_add(offset).ok_or("SnippetChoicePanel cleanup model offset overflow")?;
            let actual = unsafe { (slot as *const usize).read_volatile() };
            if actual == 0 && row.phase == ReceiverPhase::Allocated {
                row.snippet_panel_model = None;
            } else if Some(actual) != row.snippet_panel_model {
                return Err("SnippetChoicePanel cleanup model differs from allocation".into());
            }
            if actual != 0 {
                admission.verify_reference_snippet_model_cleanup_contexts(actual)?;
            }
        }
        row.phase = ReceiverPhase::Deallocating;
        Ok(OwnedDeallocation {
            receiver: p as usize,
            serial: row.serial,
            grant: self.id,
            owner: std::thread::current().id(),
        })
    }
    /// # Safety
    /// Called once by the selected typed SDK dealloc after leaf/arena cleanup.
    /// The ticket comes from this grant's exact live allocation; no business
    /// operation or callback may run between final metadata check and freeing.
    pub(crate) unsafe fn finish_owned_deallocation(&self, ticket: OwnedDeallocation) -> Result<()> {
        if ticket.receiver == 0 || ticket.grant != self.id || ticket.owner != std::thread::current().id() {
            return Err("foreign owned deallocation ticket".into());
        }
        let root = self.root_deallocator.as_ref().ok_or("SDK memory-root proof missing")?;
        self.verify_owned_cleanup()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let snippet_cleanup = self.snippet_panel_cleanup()?;
        {
            let mut rows = self
                .receivers
                .lock()
                .map_err(|_| "receiver table poisoned")?;
            let row = rows
                .get(&ticket.receiver)
                .ok_or("deallocation receiver disappeared")?;
            if row.serial != ticket.serial || row.phase != ReceiverPhase::Deallocating {
                return Err("stale or repeated owned deallocation ticket".into());
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if let Some((admission, offset)) = snippet_cleanup {
                let slot = ticket.receiver.checked_add(offset).ok_or("SnippetChoicePanel final model offset overflow")?;
                let actual = unsafe { (slot as *const usize).read_volatile() };
                if actual != row.snippet_panel_model.unwrap_or(0) {
                    return Err("SnippetChoicePanel model changed during deallocation".into());
                }
                if actual != 0 {
                    admission.verify_reference_snippet_model_cleanup_contexts(actual)?;
                }
            }
            if self.selected.iter().any(|binding| binding.selector == c".cxx_destruct"
                && matches!(&binding.proof, LifecycleProof::PreservedAppKitLifecycle(_))) {
                // The private proxy must leave its AppKit storage zeroed
                // before the real shared CXX destructors run.
                let receiver = ticket.receiver as *const AnyObject;
                if self.original_source
                    || ![c"RuycastF1OwnedStatusButtonSource", c"RuycastF1OwnedEventSource"].contains(&self.class.name())
                    || unsafe { &*receiver }.class() != self.class
                {
                    return Err("foreign receiver for shared view CXX cleanup".into());
                }
                let header = std::mem::size_of::<usize>();
                let size = self.class.instance_size().checked_sub(header)
                    .ok_or("shared view storage is shorter than its object header")?;
                let storage = unsafe { std::slice::from_raw_parts(receiver.cast::<u8>().add(header), size) };
                if storage.iter().any(|byte| *byte != 0) {
                    return Err("private status button contains unqualified AppKit state".into());
                }
            }
            rows.remove(&ticket.receiver);
        }
        unsafe { root.finish_verified(ticket.receiver as *mut AnyObject) };
        Ok(())
    }
}

#[cfg(test)]
#[path = "workspace_receiver_canary.rs"]
mod canary;

#[cfg(test)]
pub(crate) fn source_construction_canary_callsite() -> SourceCallsite {
    use sha2::{Digest, Sha256};
    SourceCallsite {
        file: "bench/f1-runner/src/sdk/workspace_receiver.rs",
        source_sha256: format!("{:x}", Sha256::digest(include_bytes!("workspace_receiver.rs"))),
        operation: "compiler-typed own source alloc/set_ivars/super-init/partial cleanup canary",
    }
}

#[cfg(test)]
mod source_construction_tests {
    use super::*;
    use objc2::{define_class, extern_class, AnyThread, DefinedClass,
        rc::Allocated, runtime::ClassBuilder};
    use objc2_foundation::NSObjectProtocol;
    use std::sync::atomic::AtomicUsize;

    extern_class!(
        #[unsafe(super(NSObject))]
        #[name = "RuycastF1SourceConstructionParent"]
        struct SourceParent;
    );
    unsafe impl NSObjectProtocol for SourceParent {}
    struct Fields { value: bool }
    impl Drop for Fields {
        fn drop(&mut self) { DROPS.fetch_add(1, Ordering::SeqCst); }
    }
    define_class!(
        #[unsafe(super(SourceParent))]
        #[thread_kind=AnyThread]
        #[ivars=Fields]
        struct RuycastF1SourceConstructionNative;
        unsafe impl NSObjectProtocol for RuycastF1SourceConstructionNative {}
        impl RuycastF1SourceConstructionNative {
            #[unsafe(method(sourceIvarsReady))]
            fn ready(&self) -> bool { self.ivars().value }
        }
    );
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static DEALLOCS: AtomicUsize = AtomicUsize::new(0);
    static CONTEXT: OnceLock<Arc<WorkspaceGrant>> = OnceLock::new();
    unsafe extern "C" fn initialize(_: *mut AnyClass, _: Sel) {}
    unsafe extern "C" fn source_alloc(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
        unsafe { super::super::factory::allocate(&*class) }.unwrap()
    }
    unsafe extern "C" fn changed_probe(_: *mut AnyObject, _: Sel) -> Bool { Bool::NO }
    unsafe extern "C" fn source_init(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
        let grant = CONTEXT.get().unwrap();
        let token = unsafe { grant.before_super_init(p) }.unwrap();
        assert_eq!(token.receiver(), p as usize);
        assert_eq!(token.observed_scalars(), &[true]);
        // The component can inspect the real partial phase without claiming
        // initialized callback/ARC authority or full14 readiness.
        p
    }
    unsafe extern "C" fn dealloc(p: *mut AnyObject, _: Sel) {
        DEALLOCS.fetch_add(1, Ordering::SeqCst);
        let method = NSObject::class().instance_method(objc2::sel!(dealloc)).unwrap();
        let call: unsafe extern "C" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(method.implementation()) };
        unsafe { call(p, objc2::sel!(dealloc)) };
    }
    struct OwnSource {
        class: &'static AnyClass,
        model: Arc<Workspace>,
        generation: u64,
        source: SourceCallsite,
        methods: Vec<(metadata::ClassHandle, &'static CStr, bool, metadata::MethodSnapshot)>,
        lives: Vec<(metadata::ClassHandle, metadata::LifecycleSnapshot)>,
        root: super::super::factory::RootDeallocator,
        reentry: Mutex<Option<std::sync::Weak<WorkspaceGrant>>>,
        reentry_mode: AtomicUsize,
        reentry_inside: AtomicBool,
        method_calls: AtomicUsize,
    }
    impl OwnSource {
        fn verify(&self) -> Result<()> {
            if self.class != RuycastF1SourceConstructionNative::class()
                || self.source != source_construction_canary_callsite() {
                return Err("own compiler source differs".into());
            }
            self.root.verify_proxy_arc(self.class)?;
            for (class, name, meta, before) in &self.methods {
                metadata::verify_current_method(*class, name, *meta, before)?;
            }
            for (class, before) in &self.lives {
                if metadata::lifecycle(*class)? != *before {
                    return Err("own source lifecycle differs".into());
                }
            }
            Ok(())
        }
    }
    // This concrete implementation qualifies only the compiler-generated own
    // class above and the fixed typed source functions in this pinned file.
    // It cannot admit a Native/AppKit class, sender or arbitrary callback.
    unsafe impl super::super::delegate::CallbackCodeAuthority for OwnSource {
        fn verify_source_construction(&self, class: &AnyClass, source: &SourceCallsite) -> Result<()> {
            if class != self.class || source != &self.source {
                return Err("foreign own source construction".into());
            }
            self.verify()?;
            if self.reentry_mode.load(Ordering::Acquire) == 1 {
                if self.reentry_inside.swap(true, Ordering::AcqRel) {
                    return Err("own nested source authority refusal".into());
                }
                let grant=self.reentry.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
                assert!(grant.verify_source_construction().is_err());
                self.reentry_inside.store(false,Ordering::Release);
            }
            Ok(())
        }
        fn verify_source_partial_cleanup(&self, identity: &SourceConstructionIdentity<'_>,
            source: &SourceCallsite) -> Result<()> {
            identity.verify_allocation()?;
            if source != &self.source || identity.class() != self.class
                || !identity.belongs_to(&self.model) || identity.generation() != self.generation
                || identity.origin_thread() != std::thread::current().id()
                || !matches!(identity.phase(), ReceiverPhase::Allocated | ReceiverPhase::SourceIvarsObserved) {
                return Err("foreign own source partial allocation".into());
            }
            self.verify()?;
            if self.reentry_mode.load(Ordering::Acquire) == 2 {
                if self.reentry_inside.swap(true, Ordering::AcqRel) {
                    return Err("own nested source cleanup refusal".into());
                }
                let grant=self.reentry.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
                assert!(unsafe { grant.cleanup_source_partial(identity.receiver() as *mut AnyObject) }.is_err());
                self.reentry_inside.store(false,Ordering::Release);
            }
            identity.verify_allocation()
        }
        fn verify_sender(&self, _: &AnyClass, _: Sel, _: usize, _: &SourceCallsite,
            _: &AnyClass) -> Result<()> { Err("own construction grants no sender".into()) }
        fn verify_method(&self, class: &AnyClass, selector: Sel, actual: usize,
            source: &SourceCallsite) -> Result<()> {
            self.method_calls.fetch_add(1,Ordering::SeqCst);
            if source != &self.source || !self.methods.iter().any(|(owner, name, _, before)|
                owner.address() == class as *const AnyClass as usize
                    && Sel::register(name) == selector && before.implementation() == actual) {
                return Err("own source method is not compiler-bound".into());
            }
            self.verify()?;
            if self.reentry_mode.load(Ordering::Acquire)==3 {
                if self.reentry_inside.swap(true,Ordering::AcqRel) {
                    return Err("own nested method authority refusal".into());
                }
                let grant=self.reentry.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
                assert!(grant.verify_source_construction().is_err());
                self.reentry_inside.store(false,Ordering::Release);
            }
            Ok(())
        }
    }
    #[test]
    fn actual_original_source_construction_keeps_partial_phases_and_cleanup_separate() {
        let fixture = super::super::tests::Fixture::new();
        let mut builder = ClassBuilder::new(c"RuycastF1SourceConstructionParent", NSObject::class()).unwrap();
        unsafe {
            builder.add_class_method(objc2::sel!(initialize), initialize as unsafe extern "C" fn(*mut AnyClass, Sel));
            builder.add_class_method(objc2::sel!(alloc), source_alloc as unsafe extern "C" fn(*mut AnyClass, Sel)->*mut AnyObject);
            builder.add_method(objc2::sel!(init), source_init as unsafe extern "C" fn(*mut AnyObject, Sel)->*mut AnyObject);
            builder.add_method(objc2::sel!(dealloc), dealloc as unsafe extern "C" fn(*mut AnyObject, Sel));
        }
        let parent = builder.register();
        let parent_guard = unsafe { super::super::classes::ClassGuard::select(&[parent], fixture.model.lease()) }.unwrap();
        let class = RuycastF1SourceConstructionNative::class();
        // The declared Rust child adds no +initialize. Its real inherited
        // entry is already owned by parent_guard; acquiring the parent again
        // through select(child)'s ancestor plan is correctly forbidden.
        assert!(!class.metaclass().instance_methods().iter()
            .any(|method| method.name()==objc2::sel!(initialize)));
        let child_handle=metadata::registered_class(class as *const AnyClass as usize).unwrap();
        let parent_handle=metadata::registered_class(parent as *const AnyClass as usize).unwrap();
        let child_initializer=metadata::method(child_handle,c"initialize",true).unwrap().unwrap();
        let parent_initializer=metadata::method(parent_handle,c"initialize",true).unwrap().unwrap();
        assert_eq!(child_initializer.owner(),parent_initializer.owner());
        assert_eq!(child_initializer.method_address(),parent_initializer.method_address());
        assert_eq!(child_initializer.implementation(),parent_initializer.implementation());
        assert_eq!(child_initializer.encoding(),parent_initializer.encoding());
        assert_eq!(child_initializer.code(),parent_initializer.code());
        assert_eq!(parent_guard.initializers.len(),1);
        assert_eq!(parent_guard.initializers[0].class,parent.name());
        assert_eq!(parent_guard.initializers[0].replacement,parent_initializer.implementation());
        let source = source_construction_canary_callsite();
        let mut methods = vec![];
        let mut lives = vec![];
        for current in [class, parent] {
            let handle = metadata::registered_class(current as *const AnyClass as usize).unwrap();
            lives.push((handle, metadata::lifecycle(handle).unwrap()));
            for method in current.instance_methods().iter() {
                let name = unsafe { CStr::from_ptr(method.name().name().as_ptr()) };
                methods.push((handle, name, false, metadata::method(handle,name,false).unwrap().unwrap()));
            }
            if current == parent {
                methods.push((handle, c"alloc", true, metadata::method(handle,c"alloc",true).unwrap().unwrap()));
            }
        }
        let generation = fixture.model.with(|state| Ok(state.generation)).unwrap();
        let authority = Arc::new(OwnSource { class, model: fixture.model.clone(), generation,
            source: source.clone(), methods, lives, root: super::super::factory::RootDeallocator::capture().unwrap(),
            reentry:Mutex::new(None),reentry_mode:AtomicUsize::new(0),reentry_inside:AtomicBool::new(false),method_calls:AtomicUsize::new(0) });
        for (name,meta,expected) in [
            (c"alloc",true,source_alloc as *const () as usize),
            (c"init",false,source_init as *const () as usize),
            (c"dealloc",false,dealloc as *const () as usize),
        ] {
            let handle=metadata::registered_class(parent as *const AnyClass as usize).unwrap();
            assert_eq!(metadata::method(handle,name,meta).unwrap().unwrap().implementation(),expected);
        }
        assert!(super::super::delegate::CallbackCodeAuthority::verify_source_construction(
            &*authority,NSObject::class(),&source).is_err());
        let mut wrong_source=source.clone();wrong_source.operation="not the actual construction caller";
        assert!(super::super::delegate::CallbackCodeAuthority::verify_source_construction(
            &*authority,class,&wrong_source).is_err());
        // One real inherited initializer proof covers both classes; source
        // CXX/dealloc bindings below still cover each actual local method.
        let mut selected = parent_guard.initializers.iter()
            .cloned().map(LifecycleBinding::constructor).collect::<Vec<_>>();
        for (handle,name,meta,_) in &authority.methods {
            if [c"dealloc",c".cxx_construct",c".cxx_destruct",c"init"].contains(name) {
                let owner = unsafe { &*(handle.address() as *const AnyClass) };
                selected.push(unsafe { LifecycleBinding::source(owner.name(),name,*meta,source.clone(),authority.clone()) }.unwrap());
            }
        }
        let probe = class.instance_method(Sel::register(c"sourceIvarsReady")).unwrap().implementation() as usize;
        let grant = Arc::new(unsafe { WorkspaceGrant::for_original(fixture.model.clone(),class,
            source.clone(),selected.clone(),vec![ScalarProbe { selector:c"sourceIvarsReady", expected_imp:probe }],authority.clone()) }.unwrap());
        assert!(unsafe { grant.allocate() }.is_err());
        unsafe { super::super::factory::publish_source_construction(grant.clone()) }.unwrap();
        assert!(CONTEXT.set(grant.clone()).is_ok());
        assert_eq!(crate::sdk::ruycast_f1_preentry_ready_v1(),0);
        std::thread::scope(|scope| {
            assert!(scope.spawn(|| grant.verify_source_construction()).join().unwrap().is_err());
        });
        assert_eq!(grant.verify_source_construction().unwrap(), generation);
        // Genuine compiler-generated partial init: zero drop flag, no ivars yet.
        let raw = unsafe { super::super::factory::allocate(class) }.unwrap();
        assert_eq!(grant.partial_source_identity(raw as usize).unwrap().phase(),ReceiverPhase::Allocated);
        let mut wrong_serial=grant.partial_source_identity(raw as usize).unwrap();
        wrong_serial.serial^=1;
        assert!(wrong_serial.verify_allocation().is_err());
        let address=raw as usize;
        std::thread::scope(|scope| {
            let grant=&grant;
            assert!(scope.spawn(move||unsafe { grant.cleanup_source_partial(address as *mut AnyObject) }).join().unwrap().is_err());
        });
        assert_eq!(grant.live_allocations().unwrap(),1);
        assert_eq!(DEALLOCS.load(Ordering::SeqCst),0);
        unsafe { grant.cleanup_source_partial(raw) }.unwrap();
        assert_eq!(DROPS.load(Ordering::SeqCst),0);
        assert_eq!(DEALLOCS.load(Ordering::SeqCst),1);
        assert_eq!(grant.live_allocations().unwrap(),0);
        let new_reentrant = || {
            let code=Arc::new(OwnSource { class,model:fixture.model.clone(),generation,
                source:source.clone(),methods:authority.methods.clone(),lives:authority.lives.clone(),root:authority.root.clone(),
                reentry:Mutex::new(None),reentry_mode:AtomicUsize::new(0),reentry_inside:AtomicBool::new(false),method_calls:AtomicUsize::new(0) });
            let mut rows=selected.clone();
            for row in &mut rows {
                if let LifecycleProof::Source { authority,.. }=&mut row.proof { *authority=code.clone(); }
            }
            let owner=Arc::new(unsafe { WorkspaceGrant::for_original(fixture.model.clone(),class,source.clone(),
                rows,vec![ScalarProbe { selector:c"sourceIvarsReady",expected_imp:probe }],code.clone()) }.unwrap());
            unsafe { owner.admit_source_construction() }.unwrap();
            *code.reentry.lock().unwrap()=Some(Arc::downgrade(&owner));
            (owner,code)
        };
        // A genuine nested refusal revokes the permission while the outer
        // authority still returns Ok. No allocation may follow that return.
        let (reentrant,code)=new_reentrant();
        code.reentry_mode.store(1,Ordering::Release);
        assert!(unsafe { reentrant.allocate() }.is_err());
        assert_eq!(reentrant.live_allocations().unwrap(),0);
        code.reentry_mode.store(0,Ordering::Release);
        assert!(reentrant.verify_source_construction().is_err());
        assert_eq!(DEALLOCS.load(Ordering::SeqCst),1);
        let (per_method,code)=new_reentrant();
        code.method_calls.store(0,Ordering::SeqCst);
        code.reentry_mode.store(3,Ordering::Release);
        assert!(unsafe { per_method.allocate() }.is_err());
        assert_eq!(code.method_calls.load(Ordering::SeqCst),2);
        assert_eq!(per_method.live_allocations().unwrap(),0);
        // A real own-method IMP change is first observed through the ordinary
        // grant.verify entry, then restored. The construction proof stays dead.
        let (mutated,_)=new_reentrant();
        let handle=metadata::registered_class(class as *const AnyClass as usize).unwrap();
        let before=metadata::method(handle,c"sourceIvarsReady",false).unwrap().unwrap();
        let encoding=std::ffi::CString::new(before.encoding()).unwrap();
        unsafe extern "C" {
            fn class_replaceMethod(class:*const AnyClass,selector:Sel,
                implementation:objc2::runtime::Imp,encoding:*const libc::c_char)->Option<objc2::runtime::Imp>;
        }
        unsafe { class_replaceMethod(class,Sel::register(c"sourceIvarsReady"),
            std::mem::transmute::<usize,objc2::runtime::Imp>(changed_probe as *const () as usize),encoding.as_ptr()); }
        assert!(mutated.verify().is_err());
        unsafe { class_replaceMethod(class,Sel::register(c"sourceIvarsReady"),
            std::mem::transmute::<usize,objc2::runtime::Imp>(before.implementation()),encoding.as_ptr()); }
        assert!(mutated.verify().is_err());
        assert!(unsafe { mutated.allocate() }.is_err());
        assert_eq!(mutated.live_allocations().unwrap(),0);
        // A nested cleanup refusal quarantines this known +1 without dealloc.
        let (quarantined,code)=new_reentrant();
        let quarantine=unsafe { quarantined.allocate() }.unwrap();
        code.reentry_mode.store(2,Ordering::Release);
        assert!(unsafe { quarantined.cleanup_source_partial(quarantine) }.is_err());
        assert_eq!(DEALLOCS.load(Ordering::SeqCst),1);
        assert_eq!(quarantined.live_allocations().unwrap(),1);
        code.reentry_mode.store(0,Ordering::Release);
        assert!(unsafe { quarantined.cleanup_source_partial(quarantine) }.is_err());
        // One explicit known quarantine fixture, not baseline-zero cleanup.
        // objc2's real typed alloc frontdoor reaches the compiler-owned +alloc
        // above, which invokes the same qualified allocator exactly once.
        // No raw-pointer-to-Allocated cast or private objc2 constructor is used.
        let allocated = RuycastF1SourceConstructionNative::alloc();
        let raw = Allocated::as_ptr(&allocated).cast_mut().cast::<AnyObject>();
        let partial = allocated.set_ivars(Fields { value:true });
        let result: Option<Retained<RuycastF1SourceConstructionNative>> = unsafe { objc2::msg_send![super(partial),init] };
        let retained = result.unwrap();
        assert_eq!(Retained::as_ptr(&retained) as usize,raw as usize);
        let raw = Retained::into_raw(retained).cast();
        assert_eq!(grant.partial_source_identity(raw as usize).unwrap().phase(),ReceiverPhase::SourceIvarsObserved);
        fixture.model.retire().unwrap();
        unsafe { grant.cleanup_source_partial(raw) }.unwrap();
        assert_eq!(DROPS.load(Ordering::SeqCst),1);
        assert_eq!(DEALLOCS.load(Ordering::SeqCst),2);
        assert_eq!(grant.live_allocations().unwrap(),0);
        assert!(unsafe { grant.cleanup_source_partial(raw) }.is_err());
        assert_eq!(crate::sdk::ruycast_f1_preentry_ready_v1(),0);
    }
}
