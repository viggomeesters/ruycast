//! Module-local receiver contribution. The sole common allocator consumes these
//! callbacks; no global objc_alloc interpose or permissive original fallback.
use super::*;
use objc2::runtime::{AnyClass, AnyObject};
use receiver::WorkspaceGrant;
use std::sync::OnceLock;
/// The existing NSObject memory terminator, pinned independently from business
/// admission. It is usable only after every proxy ancestor's CXX/dealloc path
/// has been qualified and the exact allocation has entered deallocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootDeallocator {
    class: crate::sdk::metadata::ClassHandle,
    method: crate::sdk::metadata::MethodSnapshot,
    lifecycle: crate::sdk::metadata::LifecycleSnapshot,
    arc: Vec<(
        &'static std::ffi::CStr,
        crate::sdk::metadata::MethodSnapshot,
    )>,
}
impl RootDeallocator {
    pub(crate) fn capture() -> Result<Self> {
        let class = crate::sdk::metadata::loaded_class(c"NSObject")?;
        let method = crate::sdk::metadata::method(class, c"dealloc", false)?
            .ok_or("NSObject memory terminator missing")?;
        if method.owner() != class.address()
            || super::simple_abi(&method)? != b"v@:"
            || method.code().image() != b"/usr/lib/libobjc.A.dylib"
        {
            return Err("NSObject memory terminator identity or ABI differs".into());
        }
        for selector in [c".cxx_construct", c".cxx_destruct"] {
            if crate::sdk::metadata::method(class, selector, false)?.is_some() {
                return Err("NSObject CXX memory-root qualification required".into());
            }
        }
        let lifecycle = crate::sdk::metadata::lifecycle(class)?;
        let mut arc = vec![];
        for (selector, abi) in [
            (c"retain", b"@@:".as_slice()),
            (c"release", b"v@:".as_slice()),
            (c"autorelease", b"@@:".as_slice()),
        ] {
            let snapshot = crate::sdk::metadata::method(class, selector, false)?
                .ok_or("NSObject ARC primitive missing")?;
            if snapshot.owner() != class.address()
                || super::simple_abi(&snapshot)? != abi
                || snapshot.code().image() != b"/usr/lib/libobjc.A.dylib"
            {
                return Err("NSObject ARC primitive owner or ABI differs".into());
            }
            arc.push((selector, snapshot));
        }
        Ok(Self {
            class,
            method,
            lifecycle,
            arc,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        crate::sdk::metadata::verify_current_method(self.class, c"dealloc", false, &self.method)?;
        if crate::sdk::metadata::lifecycle(self.class)? != self.lifecycle {
            return Err("NSObject memory-root provenance changed".into());
        }
        for (selector, before) in &self.arc {
            crate::sdk::metadata::verify_current_method(self.class, selector, false, before)?;
        }
        Ok(())
    }
    pub(crate) fn verify_proxy_arc(&self, class: &AnyClass) -> Result<()> {
        self.verify()?;
        let class = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
        for (selector, expected) in &self.arc {
            let actual = crate::sdk::metadata::method(class, selector, false)?
                .ok_or("SDK proxy ARC primitive missing")?;
            if actual.owner() != expected.owner()
                || actual.implementation() != expected.implementation()
                || actual.method_address() != expected.method_address()
                || actual.encoding() != expected.encoding()
                || actual.code() != expected.code()
            {
                return Err("SDK proxy custom ARC requires explicit qualification".into());
            }
        }
        Ok(())
    }
    /// A source class may use this root terminator only if the selected
    /// dealloc is inherited. Its own CXX destructor still needs a separate
    /// source proof.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_inherited_dealloc(&self, class: &AnyClass) -> Result<()> {
        self.verify_proxy_arc(class)?;
        let handle = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
        let selected = crate::sdk::metadata::method(handle, c"dealloc", false)?
            .ok_or("source receiver deallocator missing")?;
        if selected.owner() != self.class.address()
            || selected.method_address() != self.method.method_address()
            || selected.implementation() != self.method.implementation()
            || selected.encoding() != self.method.encoding()
            || selected.code() != self.method.code()
        {
            return Err("source receiver does not inherit the qualified NSObject deallocator".into());
        }
        Ok(())
    }
    pub(crate) fn verify_proxy_arc_with_bindings(
        &self,
        class: &AnyClass,
        bindings: &[super::receiver::LifecycleBinding],
        model: &Arc<Workspace>,
    ) -> Result<()> {
        let profiles = bindings
            .iter()
            .filter(|row| row.is_owned_memory())
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            return self.verify_proxy_arc(class);
        }
        if profiles.len() != 1 {
            return Err("duplicate owned memory profile".into());
        }
        self.verify()?;
        profiles[0].verify_owned_memory(class, self, model)
    }
    /// # Safety
    /// The caller verified this root and the complete lifecycle immediately
    /// before removing the exact non-null receiver's ledger row. No callbacks
    /// may run between verification and this final dealloc. The runtime alone
    /// invokes the selected CXX destructors, including the original Palette's.
    pub(crate) unsafe fn finish_verified(&self, receiver: *mut AnyObject) {
        let call: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel) =
            unsafe { std::mem::transmute(self.method.implementation()) };
        unsafe { call(receiver, objc2::sel!(dealloc)) };
    }
}
/// Pure metadata observation, never a source/entry or allocation authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundationMemoryPlan {
    pub(crate) root: RootDeallocator,
    pub(crate) roots: Vec<(
        &'static std::ffi::CStr,
        crate::sdk::metadata::MethodSnapshot,
    )>,
    pub(crate) parents: Vec<FoundationMemoryParent>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FoundationMemoryParent {
    pub(crate) role: &'static std::ffi::CStr,
    pub(crate) class: crate::sdk::metadata::ClassHandle,
    pub(crate) lifecycle: crate::sdk::metadata::LifecycleSnapshot,
    pub(crate) ancestry: Vec<usize>,
    pub(crate) methods: Vec<(
        &'static std::ffi::CStr,
        crate::sdk::metadata::MethodSnapshot,
    )>,
}
/// Pure epoch check: no guardian callback may follow this check before a write.
pub(crate) fn preflight_default_proxy_arc(class: &AnyClass) -> Result<()> {
    RootDeallocator::capture()?.verify_proxy_arc(class)
}
pub(crate) fn verify_memory_epoch(model: &Workspace, generation: u64) -> Result<()> {
    if model.faulted.load(std::sync::atomic::Ordering::Acquire) {
        return Err("memory construction business authority faulted".into());
    }
    let state = model.state.lock().map_err(|_| "workspace state poisoned")?;
    if state.retired || state.generation != generation {
        return Err("memory construction generation retired".into());
    }
    Ok(())
}

pub(crate) const MEMORY_SELECTORS: [&std::ffi::CStr; 8] = [
    c"retain",
    c"release",
    c"autorelease",
    c"retainCount",
    c"allowsWeakReference",
    c"retainWeakReference",
    c"_tryRetain",
    c"_isDeallocating",
];
fn memory_signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut value = vec![
        F::Return::ENCODING_RETURN,
        Encoding::Object,
        objc2::runtime::Sel::ENCODING,
    ];
    value.extend_from_slice(F::Arguments::ENCODINGS);
    value
}
pub(crate) fn verify_memory_abi(
    selector: &std::ffi::CStr,
    method: &crate::sdk::metadata::MethodSnapshot,
) -> Result<()> {
    use objc2::runtime::{Bool, Sel};
    let signature = if [c"retain", c"autorelease"].contains(&selector) {
        memory_signature::<unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject>()
    } else if selector == c"release" {
        memory_signature::<unsafe extern "C" fn(*mut AnyObject, Sel)>()
    } else if selector == c"retainCount" {
        memory_signature::<unsafe extern "C" fn(*mut AnyObject, Sel) -> usize>()
    } else if MEMORY_SELECTORS.contains(&selector) {
        memory_signature::<unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool>()
    } else {
        return Err("unknown owned memory selector".into());
    };
    super::classes::verify_typed_abi(method, &signature)
}
impl FoundationMemoryPlan {
    fn parent(
        role: &'static std::ffi::CStr,
        class: crate::sdk::metadata::ClassHandle,
    ) -> Result<FoundationMemoryParent> {
        let mut ancestry = vec![];
        let mut current = Some(unsafe { &*(class.address() as *const AnyClass) });
        while let Some(class) = current {
            ancestry.push(class as *const AnyClass as usize);
            current = class.superclass();
        }
        let methods = MEMORY_SELECTORS
            .into_iter()
            .map(|selector| {
                let method = crate::sdk::metadata::method(class, selector, false)?
                    .ok_or("Foundation memory method absent")?;
                verify_memory_abi(selector, &method)?;
                Ok((selector, method))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(FoundationMemoryParent {
            role,
            class,
            lifecycle: crate::sdk::metadata::lifecycle(class)?,
            ancestry,
            methods,
        })
    }
    pub(crate) fn capture() -> Result<Self> {
        let root = RootDeallocator::capture()?;
        let mut roots = vec![];
        for selector in MEMORY_SELECTORS {
            let method = crate::sdk::metadata::method(root.class, selector, false)?
                .ok_or("NSObject memory profile missing")?;
            verify_memory_abi(selector, &method)?;
            if method.owner() != root.class.address()
                || method.code().image() != b"/usr/lib/libobjc.A.dylib"
            {
                return Err("NSObject memory profile requires explicit root qualification".into());
            }
            roots.push((selector, method));
        }
        let parents = [
            c"NSString",
            c"NSData",
            c"NSURL",
            c"NSNumber",
            c"NSValue",
            c"NSArray",
            c"NSDictionary",
        ]
        .into_iter()
        .map(|role| Self::parent(role, crate::sdk::metadata::loaded_class(role)?))
        .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            root,
            roots,
            parents,
        })
    }
    pub(crate) fn verify_current(&self) -> Result<()> {
        if Self::capture()? != *self {
            return Err("Foundation memory prewrite plan changed".into());
        }
        Ok(())
    }
    /// Authorised lifecycle substitutions change whole-six metadata; memory and
    /// class ancestry still remain exactly the pre-entry source observations.
    pub(crate) fn verify_arc_sources(&self) -> Result<()> {
        self.root.verify()?;
        if self.roots.len() != MEMORY_SELECTORS.len()
            || self
                .roots
                .iter()
                .zip(MEMORY_SELECTORS)
                .any(|((actual, _), expected)| *actual != expected)
        {
            return Err("memory root selector order differs".into());
        }
        let mut checks = Vec::with_capacity(8 + self.parents.len() * 8);
        for (selector, expected) in &self.roots {
            verify_memory_abi(selector, expected)?;
            checks.push((self.root.class, *selector, false, expected));
        }
        for parent in &self.parents {
            let mut ancestry = vec![];
            let mut current = Some(unsafe { &*(parent.class.address() as *const AnyClass) });
            while let Some(class) = current {
                ancestry.push(class as *const AnyClass as usize);
                current = class.superclass();
            }
            if ancestry != parent.ancestry
                || parent.methods.len() != MEMORY_SELECTORS.len()
                || parent
                    .methods
                    .iter()
                    .zip(MEMORY_SELECTORS)
                    .any(|((actual, _), expected)| *actual != expected)
            {
                return Err("Foundation parent memory ancestry/roster changed".into());
            }
            for (selector, expected) in &parent.methods {
                verify_memory_abi(selector, expected)?;
                checks.push((parent.class, *selector, false, expected));
            }
        }
        if checks.len() != 8 + self.parents.len() * 8 {
            return Err("complete memory method verification roster differs".into());
        }
        crate::sdk::metadata::verify_current_methods(&checks)?;
        Ok(())
    }
    pub(crate) fn parent_for(
        &self,
        role: &std::ffi::CStr,
        parent: &AnyClass,
    ) -> Result<&FoundationMemoryParent> {
        self.parents
            .iter()
            .find(|row| {
                row.role == role && row.class.address() == parent as *const AnyClass as usize
            })
            .ok_or_else(|| "foreign Foundation memory plan parent".into())
    }
    #[cfg(test)]
    pub(crate) fn own_fixture(parent: &'static AnyClass) -> Result<Self> {
        let mut plan = Self::capture()?;
        plan.parents = vec![Self::parent(
            parent.name(),
            crate::sdk::metadata::registered_class(parent as *const AnyClass as usize)?,
        )?];
        Ok(plan)
    }
}
static GRANTS: OnceLock<Mutex<BTreeMap<usize, Arc<WorkspaceGrant>>>> = OnceLock::new();
struct Proxy {
    source: crate::sdk::metadata::ClassHandle,
    grant: Arc<WorkspaceGrant>,
}
pub(crate) struct OwnedReceiver {
    receiver: *mut AnyObject,
    serial: u64,
    model: Arc<Workspace>,
    returned: bool,
}
impl OwnedReceiver {
    /// # Safety
    /// Adopt exactly one actual caller-owned +1 from the common factory.
    pub(crate) unsafe fn from_owned(receiver: *mut AnyObject) -> Result<Self> {
        let grant = allocation_owner(receiver)?;
        let serial = grant.owned_reference_serial(receiver as usize)?;
        Ok(Self {
            receiver,
            serial,
            model: grant.workspace().clone(),
            returned: false,
        })
    }
    /// Borrow identity only. This does not grant business or ARC authority;
    /// retain/release/handoff still reverify the existing allocation grant.
    pub(crate) fn as_ptr(&self) -> *mut AnyObject {
        self.receiver
    }
    pub(crate) fn into_raw(mut self) -> *mut AnyObject {
        self.returned = true;
        self.receiver
    }
}
impl Drop for OwnedReceiver {
    fn drop(&mut self) {
        if !self.returned {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                if owned_reference_serial(self.receiver)? != self.serial {
                    return Err("owned receiver allocation serial changed".into());
                }
                release_owned(self.receiver)
            }));
            match result {
                Ok(Ok(())) => (),
                other => {
                    if let Err(payload) = other {
                        std::mem::forget(payload);
                    }
                    // The acquired +1 remains quarantined. New business work
                    // cannot resume after an unqualified cleanup outcome.
                    self.model
                        .faulted
                        .store(true, std::sync::atomic::Ordering::Release);
                }
            }
        }
    }
}
/// A concrete strong reference held by owned collections/event queues. Its
/// integer identity is Send; every use and release remains owner/serial checked.
/// No receiver registry, business grant or original ARC policy is added here.
pub(crate) struct RetainedReference {
    receiver: usize,
    serial: u64,
    owner: std::thread::ThreadId,
    model: Arc<Workspace>,
}
/// One genuine retained Pipe/Handle wrapper stored by a mutable Task. Its
/// payload remains owner-only; only serial-bound ARC cleanup may cross threads.
pub(crate) struct TaskChildReference {
    receiver: usize,
    serial: u64,
    owner: std::thread::ThreadId,
    grant: Arc<WorkspaceGrant>,
}
impl TaskChildReference {
    pub(crate) fn from_owned_child(receiver:*mut AnyObject) -> Result<Self> {
        let grant=allocation_owner(receiver)?;
        let role=receiver_role(receiver,grant.workspace())?;
        if !matches!(role.to_bytes(),b"NSPipe"|b"NSFileHandle")
            || !super::objc::task_payload_for_factory(receiver,role,grant.workspace())? {
            return Err("Task child is not a qualified Pipe/Handle".into());
        }
        let serial=grant.owned_reference_serial(receiver as usize)?;
        grant.verify_task_reference(receiver as usize,serial,false)?;
        let retained=unsafe {retain_owned(receiver)}?;
        Ok(Self {receiver:retained as usize,serial,owner:std::thread::current().id(),grant})
    }
    fn pointer(&self)->Result<*mut AnyObject> {
        if self.owner!=std::thread::current().id() {return Err("Task child property on foreign owner".into());}
        self.grant.verify_task_reference(self.receiver,self.serial,false)?;
        Ok(self.receiver as *mut AnyObject)
    }
    pub(crate) fn autoreleased_pointer(&self)->Result<*mut AnyObject> {
        let pointer=self.pointer()?;
        let held=unsafe {retain_owned(pointer)}?;
        let pending=unsafe {OwnedReceiver::from_owned(held)}?;
        self.pointer()?;
        let result=unsafe {autorelease_owned(held)}?;
        pending.into_raw();
        Ok(result)
    }
}
impl Drop for TaskChildReference {
    fn drop(&mut self) {
        let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<()> {
            self.grant.verify_task_reference(self.receiver,self.serial,false)?;
            unsafe {release_owned(self.receiver as *mut AnyObject)}
        }));
        if !matches!(outcome,Ok(Ok(()))) {
            if let Err(payload)=outcome {std::mem::forget(payload);}
            self.grant.workspace().fault();
            // A revoked known +1 is quarantined, never released through a fallback.
        }
    }
}
pub(crate) unsafe fn publish_task_reference(receiver:*mut AnyObject,role:&std::ffi::CStr,
    model:&Arc<Workspace>)->Result<()> {
    let grant=allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(),model) || receiver_role(receiver,model)?!=role
        || !matches!(role.to_bytes(),b"NSTask"|b"NSPipe"|b"NSFileHandle")
        || !super::objc::task_payload_for_factory(receiver,role,model)? {
        return Err("Task payload/factory/model differs".into());
    }
    unsafe {grant.publish_task_reference(receiver as usize,role==c"NSTask")}
}
pub(crate) unsafe fn publish_error_data(receiver:*mut AnyObject,model:&Arc<Workspace>)->Result<()> {
    let grant=allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(),model) || receiver_role(receiver,model)?!=c"NSError"
        || !super::objc::error_payload_for_factory(receiver,model)?
        || unsafe {&*receiver}.class()!=grant.class() {
        return Err("NSError immutable payload/family/isa differs".into());
    }
    unsafe {grant.publish_error_data(receiver as usize)}
}
pub(crate) fn verify_task_job(receiver:*mut AnyObject,serial:u64,model:&Arc<Workspace>)->Result<()> {
    let grant=allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(),model) || receiver_role(receiver,model)?!=c"NSTask" {
        return Err("Task job family/model differs".into());
    }
    let context=crate::sdk::process_abi::installed_context()?;
    context.registry.before_effect()?;
    grant.verify_task_job(receiver as usize,serial)?;
    context.registry.not_retired()
}
pub(crate) fn verify_task_child_cleanup(receiver:*mut AnyObject,serial:u64,model:&Arc<Workspace>)->Result<()> {
    let grant=allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(),model)
        || !matches!(receiver_role(receiver,model)?.to_bytes(),b"NSPipe"|b"NSFileHandle") {
        return Err("Task child cleanup family/model differs".into());
    }
    grant.verify_task_reference(receiver as usize,serial,false)
}
pub(crate) fn verify_task_allocation_pure(receiver:*mut AnyObject,serial:u64,owner:std::thread::ThreadId,
    role:&std::ffi::CStr,model:&Arc<Workspace>)->Result<()> {
    let grant=allocation_owner(receiver)?;
    let expected={
        let proxies=PROXIES.get().ok_or("Task proxies absent")?.lock().map_err(|_| "workspace proxy registry poisoned")?;
        proxies.get(role).ok_or("Task role absent")?.grant.clone()
    };
    if !Arc::ptr_eq(&grant,&expected) || !Arc::ptr_eq(grant.workspace(),model) {
        return Err("Task role/model changed during callback".into());
    }
    grant.verify_task_allocation_pure(receiver as usize,serial,owner,role==c"NSTask")?;
    // Allocation membership/serial/phase is established before this sole
    // physical read. A role label alone is never object identity evidence.
    if unsafe {&*receiver}.class()!=grant.class() {return Err("Task physical isa changed".into());}
    Ok(())
}
impl RetainedReference {
    pub(crate) fn immutable_constant_identity(&self,model:&Arc<Workspace>)->Result<(*mut AnyObject, u64)> {
        if !Arc::ptr_eq(&self.model,model) {return Err("foreign retained constant model".into());}
        let grant=allocation_owner(self.receiver as *mut AnyObject)?;
        if !Arc::ptr_eq(grant.workspace(),model) {return Err("foreign constant allocation model".into());}
        grant.verify_immutable_constant_reference(self.receiver,self.serial)?;
        Ok((self.receiver as *mut AnyObject, self.serial))
    }
    pub(crate) fn acquire(receiver: *mut AnyObject, model: &Arc<Workspace>) -> Result<Self> {
        let grant = allocation_owner(receiver)?;
        if !Arc::ptr_eq(grant.workspace(), model) {
            return Err("retained reference belongs to another workspace".into());
        }
        let serial = grant.owned_reference_serial(receiver as usize)?;
        let retained = unsafe { retain_owned(receiver) }?;
        Ok(Self {
            receiver: retained as usize,
            serial,
            owner: std::thread::current().id(),
            model: model.clone(),
        })
    }
    pub(crate) fn pointer(&self) -> Result<*mut AnyObject> {
        if self.owner != std::thread::current().id()
            && !is_immutable_data(self.receiver as *mut AnyObject, &self.model)?
        {
            return Err("retained reference on foreign owner thread".into());
        }
        let receiver = self.receiver as *mut AnyObject;
        if owned_reference_serial(receiver)? != self.serial {
            return Err("retained reference allocation serial changed".into());
        }
        Ok(receiver)
    }
}
impl Drop for RetainedReference {
    fn drop(&mut self) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            unsafe { release_owned(self.pointer()?) }
        }));
        match outcome {
            Ok(Ok(())) => (),
            other => {
                if let Err(payload) = other {
                    std::mem::forget(payload);
                }
                self.model
                    .faulted
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
    }
}
static PROXIES: OnceLock<Mutex<BTreeMap<&'static std::ffi::CStr, Proxy>>> = OnceLock::new();

/// Opaque snapshot of the same published proxy owners. No receiver, class or
/// grant can be supplied by a receipt or reconstructed from a pointer.
#[derive(Clone)]
pub struct PublishedWorkspaceFactories {
    model: Arc<Workspace>,
    roles: Vec<(&'static std::ffi::CStr, Arc<WorkspaceGrant>)>,
}
impl PublishedWorkspaceFactories {
    pub(crate) fn verify(&self) -> Result<()> {
        if self.roles.is_empty() || self.roles.iter().map(|(role, _)| *role)
            .collect::<std::collections::BTreeSet<_>>().len() != self.roles.len() {
            return Err("empty/duplicate published factory proof".into());
        }
        self.model.with(|_| Ok(()))?;
        let current = {
            let proxies = PROXIES.get().ok_or("published SDK factories absent")?
                .lock().map_err(|_| "workspace proxy registry poisoned")?;
            self.roles.iter().map(|(role, grant)| {
                let actual = proxies.get(role).ok_or("attested SDK role disappeared")?;
                if !Arc::ptr_eq(&actual.grant, grant)
                    || !Arc::ptr_eq(grant.workspace(), &self.model) {
                    return Err("attested SDK factory/model identity changed".into());
                }
                Ok(grant.clone())
            }).collect::<Result<Vec<_>>>()?
        };
        // All guardian/metadata callbacks execute outside the registry lock.
        for grant in current { grant.attested_owned_leaves()?; }
        self.model.with(|_| Ok(()))
    }
    pub(crate) fn leaves(&self) -> Result<Vec<crate::sdk::ObjcReplacement>> {
        self.verify()?;
        let mut leaves = Vec::new();
        for (_, grant) in &self.roles { leaves.extend(grant.attested_owned_leaves()?); }
        Ok(leaves)
    }
    pub(crate) fn roles(&self) -> Vec<&'static std::ffi::CStr> {
        self.roles.iter().map(|(role, _)| *role).collect()
    }
    pub(crate) fn belongs_to(&self, model: &Arc<Workspace>) -> bool {
        Arc::ptr_eq(&self.model, model)
    }
    pub(crate) fn verify_required_selectors(
        &self,
        required: &[(&std::ffi::CStr, &std::ffi::CStr, bool, Option<usize>)],
    ) -> Result<()> {
        self.verify()?;
        // No callbacks or effects occur during this immutable scan. Both
        // boundaries verify the actual model, proxy owners and typed grants.
        for &(role, selector, class_method, replacement) in required {
            let grant = self.roles.iter().find(|(name, _)| *name == role)
                .map(|(_, grant)| grant);
            let covered = grant.is_some_and(|grant| {
                if selector == c"initialize" && class_method {
                    grant.has_initialize_binding(role)
                } else {
                    grant.selected().iter().any(|binding| {
                        binding.selector == selector && binding.class_method == class_method
                            && Some(binding.replacement) == replacement
                            && binding.class == if class_method { role } else { grant.class().name() }
                    })
                }
            });
            if !covered {
                return Err(format!("unbound workspace selector {role:?}/{selector:?}/{class_method}"));
            }
        }
        self.verify()
    }
    pub(crate) fn covers(&self, role: &std::ffi::CStr, selector: &std::ffi::CStr,
        class_method: bool, replacement: usize) -> Result<bool> {
        self.verify()?;
        let Some((_, grant)) = self.roles.iter().find(|(name, _)| *name == role) else {
            return Ok(false);
        };
        Ok(grant.selected().iter().any(|binding| binding.selector == selector
            && binding.class_method == class_method && binding.replacement == replacement
            && if class_method { binding.class == role } else { binding.class == grant.class().name() }))
    }
}
pub(crate) fn attest_published_factories(model: &Arc<Workspace>,
    required: &[&'static std::ffi::CStr]) -> Result<PublishedWorkspaceFactories> {
    if required.is_empty() || required.iter().copied().collect::<std::collections::BTreeSet<_>>().len() != required.len() {
        return Err("empty/duplicate factory attestation roster".into());
    }
    let roles = {
        let proxies = PROXIES.get().ok_or("published SDK factories absent")?
            .lock().map_err(|_| "workspace proxy registry poisoned")?;
        required.iter().map(|role| {
            let grant = proxies.get(role)
                .ok_or_else(|| format!("required SDK factory not published: {role:?}"))?
                .grant.clone();
            Ok((*role, grant))
        }).collect::<Result<Vec<_>>>()?
    };
    let proof = PublishedWorkspaceFactories { model: model.clone(), roles };
    proof.verify()?;
    Ok(proof)
}
pub(crate) fn verify_published_callbacks(
    model: &Arc<Workspace>,
    catalog: &dyn super::objc::application::CallbackCatalog,
) -> Result<()> {
    let grants = GRANTS.get().ok_or("callback allocation grants absent")?
        .lock().map_err(|_| "workspace factory poisoned")?
        .values().filter(|grant| Arc::ptr_eq(grant.workspace(), model)
            && grant.is_original_source()).cloned().collect::<Vec<_>>();
    if grants.is_empty() { return Err("original callback allocation roster absent".into()); }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let (constructors, grants) = {
        let (constructors, callbacks): (Vec<_>, Vec<_>) = grants.into_iter().partition(|grant| {
            grant.is_reference_edit_construction() || grant.is_reference_palette_construction()
                || grant.is_reference_snippet_panel_construction()
        });
        if constructors.len() != 3
            || !constructors.iter().any(|grant| grant.is_reference_edit_construction())
            || !constructors.iter().any(|grant| grant.is_reference_palette_construction())
            || !constructors.iter().any(|grant| grant.is_reference_snippet_panel_construction()) {
            return Err("expected exact Reference Edit, Palette and SnippetChoicePanel construction grants".into());
        }
        if unsafe { libc::pthread_main_np() } != 1 {
            let [grant] = callbacks.as_slice() else {
                return Err("expected the sole Reference AppDelegate callback grant".into());
            };
            model.with(|_| Ok(()))?;
            for grant in &constructors { grant.observe_reference_construction()?; }
            grant.observe_published_callbacks(catalog)?;
            return constructors.iter().try_for_each(|grant| grant.observe_reference_construction());
        }
        for grant in &constructors { grant.verify()?; }
        (constructors, callbacks)
    };
    if grants.is_empty() { return Err("original callback allocation roster absent".into()); }
    for grant in grants { grant.attested_original_callbacks(catalog)?; }
    model.with(|_| Ok(()))?;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    for grant in &constructors { grant.observe_reference_construction()?; }
    Ok(())
}
pub fn publish_proxy(role: &'static std::ffi::CStr, grant: Arc<WorkspaceGrant>) -> Result<()> {
    grant.verify()?;
    let mut ancestor = Some(grant.class());
    let mut matches = false;
    while let Some(class) = ancestor {
        if class.name() == role {
            matches = true;
            break;
        }
        ancestor = class.superclass();
    }
    if !matches {
        return Err("SDK proxy does not preserve required runtime type".into());
    }
    let source = crate::sdk::metadata::loaded_class(role)?;
    let mut proxies = PROXIES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "workspace proxy registry poisoned")?;
    if proxies.contains_key(role) {
        return Err("duplicate SDK proxy role".into());
    }
    let mut all = GRANTS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "workspace factory poisoned")?;
    let key = grant.class() as *const AnyClass as usize;
    if let Some(old) = all.get(&key) {
        if !Arc::ptr_eq(old, &grant) {
            return Err("class proxy has a different grant".into());
        }
    }
    all.insert(key, grant.clone());
    proxies.insert(role, Proxy { source, grant });
    Ok(())
}
/// Exact class entry only: no unknown class dereference or ancestry fallback.
pub(crate) fn class_receiver(
    receiver: *mut AnyClass,
    role: &std::ffi::CStr,
    model: &Arc<Workspace>,
) -> Result<()> {
    let (source, grant) = {
        let proxies = PROXIES
            .get()
            .ok_or("typed SDK proxy factories absent")?
            .lock()
            .map_err(|_| "workspace proxy registry poisoned")?;
        let proxy = proxies.get(role).ok_or("typed SDK proxy role missing")?;
        (proxy.source.address(), proxy.grant.clone())
    };
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("foreign class factory context".into());
    }
    if receiver as usize != source && !std::ptr::eq(receiver, grant.class()) {
        return Err("unknown or unqualified class factory receiver".into());
    }
    grant.verify()
}
/// False means another actually published, qualified class role; unknown class
/// integers and changed metadata remain errors, not permissive classification.
/// Read only the actual published allocation owner; no raw receiver/class
/// dereference or inferred role from an address is permitted.
pub(crate) fn attested_owned_method(receiver: *mut AnyObject,
    selector: &std::ffi::CStr) -> Result<crate::sdk::ObjcReplacement> {
    let grant = allocation_owner(receiver)?;
    grant.verify_owned_reference(receiver as usize)?;
    let leaves = grant.attested_owned_leaves()?;
    let mut selected = leaves.into_iter().filter(|leaf| !leaf.class_method
        && leaf.class == grant.class().name() && leaf.selector == selector);
    let leaf = selected.next().ok_or("responder method has no typed owned leaf")?;
    if selected.next().is_some() { return Err("duplicate typed responder leaf".into()); }
    Ok(leaf)
}
pub(crate) fn receiver_role(receiver: *mut AnyObject, model: &Arc<Workspace>)
    -> Result<&'static std::ffi::CStr> {
    let grant = allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(), model) { return Err("foreign receiver role model".into()); }
    grant.verify_owned_reference(receiver as usize)?;
    let roles = {
        let proxies = PROXIES.get().ok_or("typed SDK proxies absent")?
            .lock().map_err(|_| "workspace proxy registry poisoned")?;
        proxies.iter().filter_map(|(role, proxy)| Arc::ptr_eq(&proxy.grant, &grant)
            .then_some(*role)).collect::<Vec<_>>()
    };
    if roles.len() != 1 { return Err("missing/ambiguous actual receiver role".into()); }
    Ok(roles[0])
}
pub(crate) fn class_role_matches(
    receiver: *mut AnyClass,
    role: &std::ffi::CStr,
    model: &Arc<Workspace>,
) -> Result<bool> {
    let (expected, actual) = {
        let proxies = PROXIES
            .get()
            .ok_or("typed SDK proxy factories absent")?
            .lock()
            .map_err(|_| "workspace proxy registry poisoned")?;
        let expected = proxies
            .get(role)
            .ok_or("typed SDK proxy role missing")?
            .grant
            .clone();
        let actual = proxies
            .values()
            .find(|proxy| {
                receiver as usize == proxy.source.address()
                    || std::ptr::eq(receiver, proxy.grant.class())
            })
            .map(|proxy| proxy.grant.clone())
            .ok_or("unknown class reference")?;
        (expected, actual)
    };
    if !Arc::ptr_eq(expected.workspace(), model) || !Arc::ptr_eq(actual.workspace(), model) {
        return Err("foreign class reference context".into());
    }
    expected.verify()?;
    actual.verify()?;
    Ok(Arc::ptr_eq(&expected, &actual))
}
/// Atomically claim only the exact role's actual +1 allocation. Unknown/wrong
/// roles are never adopted or released by this check.
pub(crate) fn claim_owned_responder_role(
    receiver:*mut AnyObject,model:&Arc<Workspace>,standalone:usize,joint:usize,
) -> Result<()> {
    let expected={
        let proxies=PROXIES.get().ok_or("typed SDK proxy factories absent")?
            .lock().map_err(|_|"workspace proxy registry poisoned")?;
        proxies.get(c"NSResponder").ok_or("typed responder factory absent")?.grant.clone()
    };
    if !Arc::ptr_eq(expected.workspace(),model) { return Err("foreign responder model".into()); }
    expected.verify_owned_cleanup()?;
    let method=super::raw_method(expected.class(),objc2::sel!(dealloc),false)?
        .ok_or("responder deallocation metadata absent")?;
    let selected=method.implementation();
    if selected!=standalone && selected!=joint { return Err("responder deallocator differs".into()); }
    claim_owned_role(receiver,c"NSResponder",model,selected)
}
pub(crate) fn joint_responder_cleanup_role(receiver:*mut AnyObject,model:&Arc<Workspace>)->Result<&'static std::ffi::CStr> {
    let actual=allocation_owner(receiver)?;
    if !Arc::ptr_eq(actual.workspace(),model) { return Err("foreign joint responder model".into()); }
    let proxies=PROXIES.get().ok_or("joint responder proxies absent")?
        .lock().map_err(|_|"workspace proxy registry poisoned")?;
    [c"NSResponder",c"NSApplication",c"NSWindow",c"NSPanel"].into_iter()
        .find(|role|proxies.get(role).is_some_and(|proxy|Arc::ptr_eq(&proxy.grant,&actual)))
        .ok_or("joint responder allocation family differs".into())
}
// Exact two-family cleanup projection in the existing allocation owner registry.
// Exact notification-token grant; no root NSObject allocation permission.
pub(crate) fn verify_status_item_cleanup(receiver:*mut AnyObject,model:&Arc<Workspace>)->Result<()> {
    let expected=PROXIES.get().ok_or("status-item factory absent")?
        .lock().map_err(|_|"workspace proxy registry poisoned")?
        .get(c"NSStatusItem").ok_or("status-item role absent")?.grant.clone();
    let actual=allocation_owner(receiver)?;
    if !Arc::ptr_eq(&actual,&expected)||!Arc::ptr_eq(actual.workspace(),model)||actual.is_original_source() {
        return Err("status-item actual SDK allocation/model differs".into());
    }
    actual.verify_owned_reference(receiver as usize)?;
    if unsafe{&*receiver}.class()!=actual.class(){return Err("status-item physical SDK class changed".into());}
    actual.verify_owned_cleanup()
}
pub(crate) fn verify_notification_token_cleanup(receiver:*mut AnyObject,model:&Arc<Workspace>)->Result<()> {
    let expected=PROXIES.get().ok_or("notification token factory absent")?
        .lock().map_err(|_|"workspace proxy registry poisoned")?
        .get(c"NSObject").ok_or("notification token role absent")?.grant.clone();
    let actual=owned_family(receiver,model,super::objc::owned_notification_token_dealloc as *const () as usize)?;
    if !Arc::ptr_eq(&actual,&expected)||!Arc::ptr_eq(actual.workspace(),model) {
        return Err("notification token actual grant differs".into());
    }
    Ok(())
}

pub(crate) fn verify_notification_payload_cleanup(receiver:*mut AnyObject,model:&Arc<Workspace>)->Result<()> {
    let expected=PROXIES.get().ok_or("notification payload factory absent")?
        .lock().map_err(|_|"workspace proxy registry poisoned")?
        .get(c"NSNotification").ok_or("notification payload role absent")?.grant.clone();
    let actual=owned_family(receiver,model,super::objc::owned_notification_payload_dealloc as *const () as usize)?;
    if !Arc::ptr_eq(&actual,&expected)||!Arc::ptr_eq(actual.workspace(),model) {
        return Err("notification payload actual grant differs".into());
    }
    Ok(())
}

pub(crate) fn notification_family_cleanup_role(receiver:*mut AnyObject,model:&Arc<Workspace>)
    ->Result<&'static std::ffi::CStr> {
    let actual=allocation_owner(receiver)?;
    if !Arc::ptr_eq(actual.workspace(),model) {return Err("foreign notification cleanup model".into());}
    let selected={
        let proxies=PROXIES.get().ok_or("notification factories absent")?
            .lock().map_err(|_|"workspace proxy registry poisoned")?;
        [c"NSWorkspace",c"NSNotificationCenter",c"NSOperationQueue",c"NSWorkspaceOpenConfiguration",c"NSImage"].into_iter()
            .find(|role|proxies.get(role).is_some_and(|proxy|Arc::ptr_eq(&proxy.grant,&actual)))
            .ok_or("notification cleanup actual family differs")?
    };
    let deallocator=match selected {
        role if role==c"NSWorkspace"=>super::objc::owned_workspace_dealloc as *const () as usize,
        role if role==c"NSNotificationCenter"=>super::objc::owned_notification_center_dealloc as *const () as usize,
        role if role==c"NSWorkspaceOpenConfiguration"=>super::objc::owned_open_configuration_dealloc as *const () as usize,
        role if role==c"NSImage"=>super::objc::owned_bundle_icon_dealloc as *const () as usize,
        _=>super::objc::owned_main_queue_dealloc as *const () as usize,
    };
    let checked=owned_family(receiver,model,deallocator)?;
    if !Arc::ptr_eq(&actual,&checked) {return Err("notification cleanup grant changed".into());}
    Ok(selected)
}

#[cfg(all(rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) fn publish_running_application_reference(
    receiver: *mut AnyObject, model: &Arc<Workspace>,
) -> Result<()> {
    let role = c"NSRunningApplication";
    if running_screen_cleanup_role(receiver, model)? != role {
        return Err("Reference running application reference role differs".into());
    }
    // This proxy carries only an immutable workspace handle. Activation
    // still mutates the guarded workspace model, never the proxy payload.
    let grant = allocation_owner(receiver)?;
    unsafe { grant.publish_immutable_sdk_data(receiver as usize) }
}

pub(crate) fn running_screen_cleanup_role(receiver:*mut AnyObject,model:&Arc<Workspace>)
    ->Result<&'static std::ffi::CStr> {
    let actual=allocation_owner(receiver)?;
    if !Arc::ptr_eq(actual.workspace(),model) {return Err("foreign running/screen cleanup model".into());}
    let selected={
        let proxies=PROXIES.get().ok_or("running/screen factories absent")?
            .lock().map_err(|_|"workspace proxy registry poisoned")?;
        [c"NSRunningApplication",c"NSScreen"].into_iter()
            .find(|role|proxies.get(role).is_some_and(|proxy|Arc::ptr_eq(&proxy.grant,&actual)))
            .ok_or("running/screen cleanup actual family differs")?
    };
    let deallocator=if selected==c"NSRunningApplication" {
        super::objc::owned_running_application_dealloc as *const () as usize
    }else {super::objc::owned_screen_dealloc as *const () as usize};
    let checked=owned_family(receiver,model,deallocator)?;
    if !Arc::ptr_eq(&actual,&checked) {return Err("running/screen cleanup grant changed".into());}
    Ok(selected)
}

pub(crate) fn claim_owned_role(
    receiver: *mut AnyObject,
    role: &std::ffi::CStr,
    model: &Arc<Workspace>,
    deallocator: usize,
) -> Result<()> {
    let expected = {
        let proxies = PROXIES
            .get()
            .ok_or("typed SDK proxy factories absent")?
            .lock()
            .map_err(|_| "workspace proxy registry poisoned")?;
        proxies
            .get(role)
            .ok_or("typed SDK proxy role missing")?
            .grant
            .clone()
    };
    let actual = owned_family(receiver, model, deallocator)?;
    if !Arc::ptr_eq(&actual, &expected) {
        return Err("initializer role differs from allocation".into());
    }
    actual.claim_owned_initializer(receiver as usize)
}
pub fn proxy(role: &std::ffi::CStr, model: &Arc<Workspace>) -> Result<*mut AnyObject> {
    let grant = PROXIES
        .get()
        .ok_or("required typed SDK proxy factories absent")?
        .lock()
        .map_err(|_| "workspace proxy registry poisoned")?
        .get(role)
        .map(|proxy| proxy.grant.clone())
        .ok_or("typed SDK proxy role missing")?;
    initialize_proxy(grant, model)
}
#[cfg(test)]
pub(crate) fn proxy_class(class: &AnyClass, model: &Arc<Workspace>) -> Result<*mut AnyObject> {
    initialize_proxy(grant_for(class)?, model)
}
fn initialize_proxy(grant: Arc<WorkspaceGrant>, model: &Arc<Workspace>) -> Result<*mut AnyObject> {
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("foreign SDK proxy context".into());
    }
    grant.verify_owned_cleanup()?;
    let p = unsafe { grant.allocate() }?;
    let pending = unsafe { OwnedReceiver::from_owned(p) }?;
    let token = unsafe { grant.before_super_init(p) }?;
    grant.initialized(token)?;
    Ok(pending.into_raw())
}
pub fn publish(grant: Arc<WorkspaceGrant>) -> Result<()> {
    grant.verify()?;
    let mut grants = GRANTS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "workspace factory poisoned")?;
    let key = grant.class() as *const AnyClass as usize;
    if grants.contains_key(&key) {
        return Err("duplicate workspace class grant".into());
    }
    grants.insert(key, grant);
    Ok(())
}
/// Publish a real source class's construction contribution without claiming
/// full14 business admission. The authority's compiler/lifecycle proof is
/// mandatory before this grant can enter the existing allocation table.
/// # Safety
/// Same source-caller, inert CXX and partial-cleanup contract as
/// WorkspaceGrant::admit_source_construction; no original AppKit constructor.
pub unsafe fn publish_source_construction(grant: Arc<WorkspaceGrant>) -> Result<()> {
    unsafe { grant.admit_source_construction() }?;
    publish(grant)
}
pub fn grant_for(class: &AnyClass) -> Result<Arc<WorkspaceGrant>> {
    let grant = GRANTS
        .get()
        .ok_or("workspace factory grants absent")?
        .lock()
        .map_err(|_| "workspace factory poisoned")?
        .get(&(class as *const AnyClass as usize))
        .cloned()
        .ok_or("unowned workspace class")?;
    grant.verify()?;
    Ok(grant)
}
/// # Safety
/// class is a valid actual class from the source-bound common allocation plan;
/// no original class initializer/allocator may have executed before this entry.
pub unsafe fn allocate(class: &AnyClass) -> Result<*mut AnyObject> {
    let key = class as *const AnyClass as usize;
    let direct = GRANTS
        .get()
        .map(|grants| {
            grants
                .lock()
                .map_err(|_| "workspace factory poisoned")
                .map(|grants| grants.get(&key).cloned())
        })
        .transpose()?
        .flatten();
    let grant = if let Some(grant) = direct {
        grant
    } else {
        PROXIES
            .get()
            .ok_or("workspace allocation grants absent")?
            .lock()
            .map_err(|_| "workspace proxy registry poisoned")?
            .iter()
            // NSObject is the opaque observer-token role, never a generic
            // source allocation alias. Only the exact internal Kind producer
            // reaches proxy(role); unrelated NSObject calls remain unadmitted.
            .find(|(role,proxy)| **role != c"NSObject" && proxy.source.address() == key)
            .map(|(_,proxy)| proxy.grant.clone())
            .ok_or("unqualified source allocation class")?
    };
    unsafe { grant.allocate() }
}
/// # Safety
/// The common source-qualified allocator returned this exact new +1 receiver;
/// this hook runs before actual caller set_ivars and selected super-init.
pub unsafe fn observed_allocation(p: *mut AnyObject) -> Result<()> {
    if p.is_null() {
        return Err("null workspace allocation".into());
    }
    unsafe { grant_for((&*p).class())?.track_allocated(p) }
}
/// Execute only on the actual source super-init callsite; no ivars-ready boolean.
/// # Safety
/// Same as WorkspaceGrant::before_super_init. All source probes are safe at this
/// exact frozen alloc->set_ivars->super-init point; no arbitrary raw receiver.
pub unsafe fn initialize(p: *mut AnyObject, model: &Arc<Workspace>) -> Result<()> {
    if p.is_null() {
        return Err("null workspace initialization".into());
    }
    let grant = grant_for(unsafe { &*p }.class())?;
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("foreign workspace allocation capability".into());
    }
    let token = unsafe { grant.before_super_init(p) }?;
    grant.initialized(token)
}
pub fn retire(class: &AnyClass, p: *mut AnyObject) -> Result<()> {
    grant_for(class)?.retire(p)
}
/// # Safety
/// Invoked only by the selected typed SDK destructor. It never decodes a raw
/// receiver until the sole existing grant table proves this live allocation.
/// Cleanup must retire leaf/arena identity; it cannot call original SDK code.
pub(crate) unsafe fn deallocate_owned(
    receiver: *mut AnyObject,
    cleanup: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if receiver.is_null() {
        return Err("null SDK deallocation receiver".into());
    }
    let grant = allocation_owner(receiver)?;
    let ticket = grant.begin_owned_deallocation(receiver)?;
    let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(cleanup));
    unsafe { grant.finish_owned_deallocation(ticket) }?;
    match cleanup {
        Ok(result) => result,
        Err(payload) => {
            std::mem::forget(payload);
            Err("SDK leaf cleanup panicked during deallocation".into())
        }
    }
}
fn allocation_owner(receiver: *mut AnyObject) -> Result<Arc<WorkspaceGrant>> {
    find_allocation_owner(receiver)?.ok_or("unknown SDK allocation receiver".into())
}
fn find_allocation_owner(receiver: *mut AnyObject) -> Result<Option<Arc<WorkspaceGrant>>> {
    let grants = GRANTS
        .get()
        .ok_or("SDK allocation grants absent")?
        .lock()
        .map_err(|_| "workspace factory poisoned")?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut selected = None;
    for grant in grants {
        if grant.owns_allocation(receiver as usize)? {
            if selected.is_some() {
                return Err("SDK allocation has multiple grant owners".into());
            }
            selected = Some(grant);
        }
    }
    Ok(selected)
}
pub(crate) fn check_owned_family(
    receiver: *mut AnyObject,
    model: &Arc<Workspace>,
    deallocator: usize,
) -> Result<()> {
    owned_family(receiver, model, deallocator)?.claim_owned_initializer(receiver as usize)
}
pub(crate) fn check_owned_initializer_claim(
    receiver: *mut AnyObject,
    model: &Arc<Workspace>,
    deallocator: usize,
) -> Result<()> {
    owned_family(receiver, model, deallocator)?.verify_owned_initializer_claim(receiver as usize)
}
fn owned_family(
    receiver: *mut AnyObject,
    model: &Arc<Workspace>,
    deallocator: usize,
) -> Result<Arc<WorkspaceGrant>> {
    let grant = allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("owned initializer belongs to another workspace".into());
    }
    grant.verify_owned_reference(receiver as usize)?;
    let actual = super::raw_method(grant.class(), objc2::sel!(dealloc), false)?
        .ok_or("owned initializer deallocator missing")?;
    if actual.implementation() != deallocator {
        return Err("owned initializer belongs to another data family".into());
    }
    Ok(grant)
}
#[cfg(test)]
pub(crate) fn live_owned_allocations(class: &AnyClass) -> Result<usize> {
    let grant = GRANTS
        .get()
        .ok_or("SDK allocation grants absent")?
        .lock()
        .map_err(|_| "workspace factory poisoned")?
        .get(&(class as *const AnyClass as usize))
        .cloned()
        .ok_or("SDK allocation class unqualified")?;
    grant.live_allocations()
}
/// Find an Initialized observed receiver without dereferencing an unknown p.
pub(crate) fn receiver_identity(
    receiver: *mut AnyObject,
) -> Result<super::receiver::ReceiverIdentity> {
    allocation_owner(receiver)?.receiver_identity(receiver as usize)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn observed_receiver_identity(
    receiver: *mut AnyObject,
) -> Result<Option<super::receiver::ReceiverIdentity>> {
    find_allocation_owner(receiver)?.map(|grant| grant.receiver_identity(receiver as usize)).transpose()
}
/// # Safety
/// Publish only this exact READY immutable SDK payload. Both role and actual
/// payload/children are independently rechecked, outside every receiver lock.
pub(crate) unsafe fn publish_immutable_data(
    receiver: *mut AnyObject,
    role: &std::ffi::CStr,
    model: &Arc<Workspace>,
) -> Result<()> {
    if !matches!(
        role.to_bytes(),
        b"NSString" | b"NSData" | b"NSURL" | b"NSNumber" | b"NSArray" | b"NSDictionary"
    ) {
        return Err("this SDK family cannot become shared immutable data".into());
    }
    let grant = allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("foreign immutable data model".into());
    }
    let expected = {
        let proxies = PROXIES
            .get()
            .ok_or("typed data factory absent")?
            .lock()
            .map_err(|_| "workspace proxy registry poisoned")?;
        proxies
            .get(role)
            .ok_or("typed data factory role absent")?
            .grant
            .clone()
    };
    if !Arc::ptr_eq(&grant, &expected) {
        return Err("immutable payload role differs from allocation".into());
    }
    if !super::objc::immutable_payload_for_factory(receiver, role, model)? {
        return Err("typed payload or child is not READY immutable data".into());
    }
    unsafe { grant.publish_immutable_sdk_data(receiver as usize) }
}
pub(crate) fn is_immutable_data(receiver: *mut AnyObject, model: &Arc<Workspace>) -> Result<bool> {
    let grant = allocation_owner(receiver)?;
    if !Arc::ptr_eq(grant.workspace(), model) {
        return Err("foreign immutable data model".into());
    }
    grant.is_immutable_sdk_data(receiver as usize)
}
/// Observe the existing grant's true allocation identity, without decoding p.
pub(crate) fn owned_reference_serial(receiver: *mut AnyObject) -> Result<u64> {
    allocation_owner(receiver)?.owned_reference_serial(receiver as usize)
}
/// # Safety
/// Consumes exactly one caller-owned +1 reference to this tracked SDK proxy.
/// No guardian callback or opaque pointer inspection is needed for ARC cleanup.
pub(crate) unsafe fn release_owned(receiver: *mut AnyObject) -> Result<()> {
    allocation_owner(receiver)?.verify_owned_reference(receiver as usize)?;
    drop(
        unsafe { objc2::rc::Retained::<AnyObject>::from_raw(receiver) }
            .ok_or("null owned SDK reference")?,
    );
    Ok(())
}
/// # Safety
/// Receiver is a currently live borrowed identity on its allocation owner thread.
pub(crate) unsafe fn retain_owned(receiver: *mut AnyObject) -> Result<*mut AnyObject> {
    allocation_owner(receiver)?.verify_owned_reference(receiver as usize)?;
    let retained = unsafe { objc2::rc::Retained::<AnyObject>::retain(receiver) }
        .ok_or("null owned SDK reference")?;
    Ok(objc2::rc::Retained::into_raw(retained))
}
/// # Safety
/// Consumes exactly one fresh +1 after the caller's final business-cap check.
/// Only qualified NSObject ARC is executed; no later dynamic business callback
/// may invalidate the originating getter between its check and this handoff.
pub(crate) unsafe fn autorelease_owned(receiver: *mut AnyObject) -> Result<*mut AnyObject> {
    allocation_owner(receiver)?.verify_owned_reference(receiver as usize)?;
    let retained = unsafe { objc2::rc::Retained::<AnyObject>::from_raw(receiver) }
        .ok_or("null owned SDK reference")?;
    Ok(objc2::rc::Retained::autorelease_return(retained))
}

#[cfg(test)]
mod memory_arc_tests {
    use super::*;
    use objc2::{
        runtime::{Bool, ClassBuilder, Sel},
        ClassType,
    };
    use objc2_foundation::NSObject;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CUSTOM: AtomicUsize = AtomicUsize::new(0);
    static DEAD: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn bad_retain(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
        CUSTOM.fetch_add(1, Ordering::SeqCst);
        p
    }
    unsafe extern "C" fn bad_release(_: *mut AnyObject, _: Sel) {
        CUSTOM.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn bad_count(_: *mut AnyObject, _: Sel) -> usize {
        CUSTOM.fetch_add(1, Ordering::SeqCst);
        999
    }
    unsafe extern "C" fn bad_bool(_: *mut AnyObject, _: Sel) -> Bool {
        CUSTOM.fetch_add(1, Ordering::SeqCst);
        Bool::NO
    }
    unsafe extern "C" fn cleanup(p: *mut AnyObject, _: Sel) {
        unsafe {
            deallocate_owned(p, || {
                DEAD.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
        .unwrap();
    }
    fn parent(name: &std::ffi::CStr) -> &'static AnyClass {
        let mut builder = ClassBuilder::new(name, NSObject::class()).unwrap();
        unsafe {
            builder.add_method(
                objc2::sel!(retain),
                bad_retain as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
            builder.add_method(
                objc2::sel!(release),
                bad_release as unsafe extern "C" fn(*mut AnyObject, Sel),
            );
            builder.add_method(
                objc2::sel!(retainCount),
                bad_count as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
            builder.add_method(
                objc2::sel!(_tryRetain),
                bad_bool as unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
            builder.add_method(
                objc2::sel!(_isDeallocating),
                bad_bool as unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
        }
        builder.register()
    }
    #[test]
    fn explicit_memory_profile_owns_arc_weak_pool_retirement_and_sticky_metadata() {
        let fixture = super::super::tests::Fixture::new();
        let parent = parent(c"RuycastF1OwnedExplicitMemoryParent");
        let memory = FoundationMemoryPlan::own_fixture(parent).unwrap();
        // The mechanical own-class observation is deliberately NOT the production
        // seven-parent Gate plan and cannot pass its pre-write qualification.
        assert!(memory.verify_current().is_err());
        let guard = unsafe {
            super::super::classes::ClassGuard::select_sdk_family(
                &[parent],
                fixture.model.lease(),
                cleanup,
            )
        }
        .unwrap();
        assert!(RootDeallocator::capture()
            .unwrap()
            .verify_proxy_arc(parent)
            .is_err());
        let (proxy, proof) = unsafe {
            guard.subclass_foundation_memory_qualified(
                c"RuycastF1OwnedExplicitMemoryProxy",
                parent,
                parent.name(),
                &memory,
                &fixture.model,
            )
        }
        .unwrap();
        let mut bindings = guard.lifecycle_bindings_for(parent).unwrap();
        let profile = super::super::receiver::LifecycleBinding::owned_memory(proof.clone());
        for field in 0..5 {
            let mut wrong = profile.clone();
            match field {
                0 => wrong.class = parent.name(),
                1 => wrong.selector = c"release",
                2 => wrong.class_method = true,
                3 => wrong.original = wrong.original.wrapping_add(1),
                _ => wrong.replacement = wrong.replacement.wrapping_add(1),
            }
            assert!(wrong.verify().is_err());
        }
        let mut wrong_root = RootDeallocator::capture().unwrap();
        wrong_root.arc.swap(0, 1);
        assert!(profile
            .verify_owned_memory(proxy, &wrong_root, &fixture.model)
            .is_err());
        let foreign = super::super::tests::Fixture::new();
        assert!(profile
            .verify_owned_memory(proxy, &memory.root, &foreign.model)
            .is_err());
        bindings.push(profile.clone());
        let grant = Arc::new(
            unsafe { WorkspaceGrant::for_owned_sdk(fixture.model.clone(), proxy, bindings) }
                .unwrap(),
        );
        publish(grant.clone()).unwrap();
        publish_proxy(parent.name(), grant.clone()).unwrap();
        assert!(attest_published_factories(&fixture.model, &[]).is_err());
        assert!(attest_published_factories(&fixture.model, &[c"RuycastF1MissingPublishedRole"]).is_err());
        assert!(attest_published_factories(&fixture.model, &[parent.name(), parent.name()]).is_err());
        let published = attest_published_factories(&fixture.model, &[parent.name()]).unwrap();
        assert!(published.belongs_to(&fixture.model));
        assert!(!published.belongs_to(&foreign.model));
        assert!(published.leaves().unwrap().iter().all(|leaf| leaf.original != 0));
        let mut mismatched = published.clone();
        mismatched.model = foreign.model.clone();
        assert!(mismatched.verify().is_err());
        let mut missing = published.clone();
        missing.roles.clear();
        assert!(missing.verify().is_err());
        let baseline = grant.live_allocations().unwrap();
        assert_eq!(baseline, 0);
        assert!(check_owned_family(
            std::ptr::dangling_mut(),
            &fixture.model,
            cleanup as *const () as usize
        )
        .is_err());
        let (held, weak) = objc2::rc::autoreleasepool(|_| {
            let (held, weak) = objc2::rc::autoreleasepool(|_| {
                let pointer = proxy_class(proxy, &fixture.model).unwrap();
                let same: Bool = unsafe { objc2::msg_send![&*pointer,isKindOfClass:parent] };
                assert!(same.as_bool());
                let count: usize = unsafe { objc2::msg_send![&*pointer, retainCount] };
                assert_eq!(count, 1);
                let weak = objc2::rc::Weak::new(unsafe { &*pointer });
                let kept = unsafe { retain_owned(pointer) }.unwrap();
                let held = unsafe { OwnedReceiver::from_owned(kept) }.unwrap();
                assert_eq!(unsafe { autorelease_owned(pointer) }.unwrap(), pointer);
                assert!(weak.load().is_some());
                let count: usize = unsafe { objc2::msg_send![&*pointer, retainCount] };
                assert_eq!(count, 2);
                assert_eq!(grant.live_allocations().unwrap(), baseline + 1);
                assert_eq!(DEAD.load(Ordering::SeqCst), 0);
                (held, weak)
            });
            // Inner drain consumes its +0 while the explicit +1 remains alive.
            assert!(weak.load().is_some());
            let pointer = held.as_ptr();
            let count: usize = unsafe { objc2::msg_send![&*pointer, retainCount] };
            assert_eq!(count, 1);
            assert_eq!(grant.live_allocations().unwrap(), baseline + 1);
            assert_eq!(DEAD.load(Ordering::SeqCst), 0);
            // Give the outer pool a separate qualified +1/+0 obligation too.
            let outer = unsafe { retain_owned(pointer) }.unwrap();
            assert_eq!(unsafe { autorelease_owned(outer) }.unwrap(), pointer);
            assert!(weak.load().is_some());
            let count: usize = unsafe { objc2::msg_send![&*pointer, retainCount] };
            assert_eq!(count, 2);
            assert_eq!(grant.live_allocations().unwrap(), baseline + 1);
            assert_eq!(DEAD.load(Ordering::SeqCst), 0);
            (held, weak)
        });
        assert!(weak.load().is_some());
        let count: usize = unsafe { objc2::msg_send![&*held.as_ptr(), retainCount] };
        assert_eq!(count, 1);
        assert_eq!(grant.live_allocations().unwrap(), baseline + 1);
        assert_eq!(DEAD.load(Ordering::SeqCst), 0);
        drop(held);
        assert!(weak.load().is_none());
        assert_eq!(grant.live_allocations().unwrap(), baseline);
        assert_eq!(DEAD.load(Ordering::SeqCst), 1);
        let failed = unsafe { grant.allocate() }.unwrap();
        check_owned_family(failed, &fixture.model, cleanup as *const () as usize).unwrap();
        assert!(check_owned_initializer_claim(
            failed,
            &fixture.model,
            cleanup as *const () as usize
        )
        .is_ok());
        {
            let _failed_initializer = unsafe { OwnedReceiver::from_owned(failed) }.unwrap();
        }
        assert_eq!(grant.live_allocations().unwrap(), baseline);
        assert_eq!(DEAD.load(Ordering::SeqCst), 2);
        let initialized = proxy_class(proxy, &fixture.model).unwrap();
        assert!(
            check_owned_family(initialized, &fixture.model, cleanup as *const () as usize).is_err()
        );
        unsafe { release_owned(initialized) }.unwrap();
        assert_eq!(DEAD.load(Ordering::SeqCst), 3);
        let pointer = proxy_class(proxy, &fixture.model).unwrap();
        let address = pointer as usize;
        assert!(std::thread::spawn({
            let grant = grant.clone();
            move || grant.verify_owned_reference(address).is_err()
        })
        .join()
        .unwrap());
        // Only the exact known immutable empty own payload is granted sharing here;
        // this does not qualify a Foundation value or extend an owner-only policy.
        unsafe { grant.publish_immutable_sdk_data(address) }.unwrap();
        let weak = objc2::rc::Weak::new(unsafe { &*pointer });
        let serial_pointer = proxy_class(proxy, &fixture.model).unwrap();
        let serial = owned_reference_serial(serial_pointer).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (go, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let pointer = address as *mut AnyObject;
            let held = unsafe { retain_owned(pointer) }.unwrap();
            let held = unsafe { OwnedReceiver::from_owned(held) }.unwrap();
            tx.send(()).unwrap();
            wait.recv().unwrap();
            drop(held);
        });
        rx.recv().unwrap();
        unsafe { release_owned(pointer) }.unwrap();
        fixture.model.retire().unwrap();
        assert!(published.verify().is_err());
        fixture.life.0.store(false, Ordering::Release);
        go.send(()).unwrap();
        worker.join().unwrap();
        assert!(weak.load().is_none());
        assert_eq!(DEAD.load(Ordering::SeqCst), 4);
        assert!(!fixture.model.faulted.load(Ordering::Acquire));
        let mut wrong_serial = unsafe { OwnedReceiver::from_owned(serial_pointer) }.unwrap();
        wrong_serial.serial = serial.wrapping_add(1);
        drop(wrong_serial);
        assert_eq!(DEAD.load(Ordering::SeqCst), 4);
        assert!(fixture.model.faulted.load(Ordering::Acquire));
        assert_eq!(owned_reference_serial(serial_pointer).unwrap(), serial);
        proof.verify_metadata().unwrap();
        unsafe { release_owned(serial_pointer) }.unwrap();
        assert_eq!(DEAD.load(Ordering::SeqCst), 5);
        assert_eq!(grant.live_allocations().unwrap(), baseline);
        assert_eq!(CUSTOM.load(Ordering::SeqCst), 0);
        // Metadata faults are separately sticky and tested only after every actual
        // owned instance is gone, so restoration cannot mask a destructor hazard.
        let retain = crate::sdk::metadata::method(
            crate::sdk::metadata::registered_class(proxy as *const AnyClass as usize).unwrap(),
            c"retain",
            false,
        )
        .unwrap()
        .unwrap();
        assert!(verify_memory_abi(c"retain", &memory.roots[1].1).is_err());
        assert!(verify_memory_abi(c"release", &memory.roots[0].1).is_err());
        let mut swapped = memory.clone();
        swapped.roots.swap(0, 1);
        assert!(swapped.verify_arc_sources().is_err());
        let method = unsafe { &*(retain.method_address() as *const objc2::runtime::Method) };
        let prior = unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                bad_retain as *const () as usize,
            ))
        };
        assert_eq!(prior as usize, retain.implementation());
        assert!(proof.verify_metadata().is_err());
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                retain.implementation(),
            ));
        }
        assert!(proof.verify_metadata().is_err());
        assert_eq!(CUSTOM.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn last_callback_retirement_or_memory_mutation_prevents_class_publication() {
        use crate::sdk::owned::AliveLease;
        struct Armed {
            ticks: AtomicUsize,
            model: Mutex<std::sync::Weak<Workspace>>,
            method: AtomicUsize,
            retire: bool,
        }
        impl AliveLease for Armed {
            fn is_alive(&self) -> bool {
                true
            }
            fn before_effect(&self) -> Result<()> {
                if self
                    .ticks
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                        if n > 0 {
                            Some(n - 1)
                        } else {
                            None
                        }
                    })
                    .ok()
                    == Some(1)
                {
                    if self.retire {
                        self.model.lock().unwrap().upgrade().unwrap().retire()?;
                    } else {
                        let method = self.method.load(Ordering::SeqCst);
                        unsafe {
                            (&*(method as *const objc2::runtime::Method)).set_implementation(
                                std::mem::transmute::<usize, objc2::runtime::Imp>(
                                    bad_retain as *const () as usize,
                                ),
                            );
                        }
                    }
                }
                Ok(())
            }
        }
        for (retire, initial, parent_name, proxy_name) in [
            (
                false,
                false,
                c"RuycastF1LateMemoryParent",
                c"RuycastF1LateMemoryMustNotExist",
            ),
            (
                true,
                false,
                c"RuycastF1LateRetireParent",
                c"RuycastF1LateRetireMustNotExist",
            ),
            (
                true,
                true,
                c"RuycastF1InitialRetireParent",
                c"RuycastF1InitialRetireMustNotExist",
            ),
        ] {
            let fixture = super::super::tests::Fixture::new();
            let armed = Arc::new(Armed {
                ticks: AtomicUsize::new(0),
                model: Mutex::new(std::sync::Weak::new()),
                method: AtomicUsize::new(0),
                retire,
            });
            let mut context = fixture.model.context.clone();
            context.life = armed.clone();
            let rect = super::super::Rect {
                origin: super::super::Point { x: 0.0, y: 0.0 },
                size: super::super::Point { x: 100.0, y: 100.0 },
            };
            let model = Workspace::new(
                context,
                vec![],
                vec![super::super::Screen {
                    frame: rect,
                    visible: rect,
                    scale: 1.0,
                }],
                super::super::Point { x: 0.0, y: 0.0 },
            )
            .unwrap();
            *armed.model.lock().unwrap() = Arc::downgrade(&model);
            let parent = ClassBuilder::new(parent_name, NSObject::class())
                .unwrap()
                .register();
            // Install an own retain slot with the genuine root primitive before capture.
            let root = FoundationMemoryPlan::capture().unwrap();
            if initial {
                root.verify_current().unwrap();
                let handle =
                    crate::sdk::metadata::registered_class(parent as *const AnyClass as usize)
                        .unwrap();
                let before = crate::sdk::metadata::lifecycle(handle).unwrap();
                let generation = model.state.lock().unwrap().generation;
                armed.ticks.store(1, Ordering::SeqCst);
                let result = unsafe {
                    super::super::classes::ClassGuard::select_foundation_memory_family_checked(
                        &[parent],
                        model.lease(),
                        |_| Ok(cleanup),
                        &root,
                        || Ok(()),
                        || verify_memory_epoch(&model, generation),
                    )
                };
                assert!(result.is_err());
                assert_eq!(armed.ticks.load(Ordering::SeqCst), 0);
                assert!(model.state.lock().unwrap().retired);
                assert_eq!(crate::sdk::metadata::lifecycle(handle).unwrap(), before);
                continue;
            }
            let root_retain = &root.roots[0].1;
            unsafe extern "C" {
                fn class_replaceMethod(
                    class: *const AnyClass,
                    selector: Sel,
                    implementation: objc2::runtime::Imp,
                    encoding: *const std::ffi::c_char,
                ) -> Option<objc2::runtime::Imp>;
            }
            unsafe {
                class_replaceMethod(
                    parent,
                    objc2::sel!(retain),
                    std::mem::transmute::<usize, objc2::runtime::Imp>(root_retain.implementation()),
                    std::ffi::CString::new(root_retain.encoding())
                        .unwrap()
                        .as_ptr(),
                );
            }
            let memory = FoundationMemoryPlan::own_fixture(parent).unwrap();
            let guard = unsafe {
                super::super::classes::ClassGuard::select_sdk_family(
                    &[parent],
                    model.lease(),
                    cleanup,
                )
            }
            .unwrap();
            let original = memory.parents[0].methods[0].1.clone();
            armed
                .method
                .store(original.method_address(), Ordering::SeqCst);
            // The second dynamic callback is inside subclass_with_skips_checked,
            // after the outer memory/model validation and before ClassBuilder.
            armed.ticks.store(2, Ordering::SeqCst);
            let result = unsafe {
                guard.subclass_foundation_memory_qualified(
                    proxy_name,
                    parent,
                    parent.name(),
                    &memory,
                    &model,
                )
            };
            assert!(result.is_err());
            assert_eq!(armed.ticks.load(Ordering::SeqCst), 0);
            assert!(AnyClass::get(proxy_name).is_none());
            if !retire {
                unsafe {
                    (&*(original.method_address() as *const objc2::runtime::Method))
                        .set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                            original.implementation(),
                        ));
                }
            }
            model.retire().unwrap();
        }
    }
    #[test]
    fn lifecycle_only_change_still_retires_full_memory_proof_after_observation_match() {
        let fixture = super::super::tests::Fixture::new();
        let parent = parent(c"RuycastF1OwnedObservedLifecycleParent");
        let memory = FoundationMemoryPlan::own_fixture(parent).unwrap();
        let guard = unsafe {
            super::super::classes::ClassGuard::select_sdk_family(
                &[parent],
                fixture.model.lease(),
                cleanup,
            )
        }
        .unwrap();
        let (_, proof) = unsafe {
            guard.subclass_foundation_memory_qualified(
                c"RuycastF1OwnedObservedLifecycleProxy",
                parent,
                parent.name(),
                &memory,
                &fixture.model,
            )
        }
        .unwrap();
        proof.verify_metadata().unwrap();
        let handle =
            crate::sdk::metadata::registered_class(parent as *const AnyClass as usize).unwrap();
        let retained = crate::sdk::metadata::method(handle, c"retain", false)
            .unwrap()
            .unwrap();
        let initialized = crate::sdk::metadata::method(handle, c"initialize", true)
            .unwrap()
            .unwrap();
        let method = unsafe { &*(initialized.method_address() as *const objc2::runtime::Method) };
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                bad_release as *const () as usize,
            ));
        }
        // The original equality ignores this new unused whole-six capture too.
        assert_eq!(
            crate::sdk::metadata::method(handle, c"retain", false)
                .unwrap()
                .as_ref(),
            Some(&retained)
        );
        crate::sdk::metadata::verify_current_method(handle, c"retain", false, &retained).unwrap();
        assert!(proof.verify_metadata().is_err());
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                initialized.implementation(),
            ));
        }
        assert!(proof.verify_metadata().is_err());
        assert!(fixture.model.faulted.load(Ordering::Acquire));
    }
    #[test]
    fn complete_foundation_memory_batch_matches_eager_current_observations() {
        let memory = FoundationMemoryPlan::capture().unwrap();
        assert_eq!(memory.parents.len(), 7);
        let mut checked = 0;
        for (selector, expected) in &memory.roots {
            assert_eq!(
                crate::sdk::metadata::method(memory.root.class, selector, false)
                    .unwrap()
                    .as_ref(),
                Some(expected)
            );
            checked += 1;
        }
        for parent in &memory.parents {
            assert_eq!(parent.methods.len(), 8);
            for (selector, expected) in &parent.methods {
                assert_eq!(
                    crate::sdk::metadata::method(parent.class, selector, false)
                        .unwrap()
                        .as_ref(),
                    Some(expected)
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 64);
        memory.verify_arc_sources().unwrap();
        let mut changed = memory.clone();
        changed.parents[0].methods.swap(0, 1);
        assert!(changed.verify_arc_sources().is_err());
    }
}

#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub use super::receiver::app_delegate::constructor::{PreparedAppDelegate,PreparedAppDelegateClass,InitializedAppDelegateRoot,AppStateCustody,ReferenceInputChildKind,ReferenceInputChildCustody};
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub unsafe fn prepare_app_delegate_class(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>)->Result<PreparedAppDelegateClass> {
    unsafe{super::receiver::app_delegate::constructor::prepare_class(context,model)}
}
/// # Safety
/// Actual exclusive Reference pre-entry. Constructor-only contribution; no
/// business callback, weak/final ARC or full Workspace/Source14 attestation.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub unsafe fn prepare_app_delegate_constructor(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>,class:PreparedAppDelegateClass)->Result<PreparedAppDelegate> {
    unsafe{super::receiver::app_delegate::constructor::prepare(context,model,class)}
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn source_allocation_grant(pointer:*mut AnyObject)->Result<Arc<WorkspaceGrant>> {allocation_owner(pointer)}
#[cfg(rcf1_reference_sdk)]
pub(crate) fn reference_edit_construction_callsite() -> crate::sdk::SourceCallsite {
    crate::sdk::SourceCallsite {
        file: "Sources/RuycastApp/NativeEditCommands.swift",
        source_sha256: "c2c0683ca011e23a8fe7f99e23c9e1540f31f43d1d80d09116a005ffefbebc90".into(),
        operation: "NativeEditCommands.shared exact original allocation and super-init",
    }
}

#[cfg(rcf1_reference_sdk)]
pub(crate) fn reference_palette_construction_callsite() -> crate::sdk::SourceCallsite {
    use sha2::{Digest, Sha256};
    crate::sdk::SourceCallsite {
        file: "Sources/RuycastApp/Palette.swift",
        source_sha256: format!("{:x}", Sha256::digest(include_bytes!(
            "../../../reference/0.8.26/Sources/RuycastApp/Palette.swift"
        ))),
        operation: "PaletteController exact original PalettePanel allocation and super-init",
    }
}

#[cfg(rcf1_reference_sdk)]
pub(crate) fn reference_snippet_panel_construction_callsite() -> crate::sdk::SourceCallsite {
    crate::sdk::SourceCallsite {
        file: "Sources/RuycastApp/SnippetChoice.swift",
        source_sha256: "3d43c84d15dc76aaed1a38c5e7d4131232168fb0d0723c79e52deb1e447b9d69".into(),
        operation: "SnippetChoicePanel original allocation and super-init",
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) unsafe fn prepare_reference_snippet_panel_construction(
    admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    model: Arc<Workspace>,
    guard: &super::classes::ClassGuard,
) -> Result<()> {
    use super::delegate::CallbackCodeAuthority;
    use objc2::runtime::Sel;

    struct SnippetPanelConstruction {
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        context: crate::sdk::core_preentry::ReferenceAppDelegateContext,
        class: &'static AnyClass,
        source: crate::sdk::SourceCallsite,
        published_metadata: (usize, usize),
    }
    unsafe impl CallbackCodeAuthority for SnippetPanelConstruction {
        fn verify_source_construction(&self, class: &AnyClass, source: &crate::sdk::SourceCallsite) -> Result<()> {
            if class != self.class || source != &self.source {
                return Err("Reference SnippetChoicePanel construction class or source differs".into());
            }
            self.context.verify_reference_snippet_panel_constructor()?;
            self.admission.verify_reference_snippet_published_destroy_cache(self.published_metadata)
        }
        fn verify_source_partial_cleanup(
            &self, identity: &super::receiver::SourceConstructionIdentity<'_>, source: &crate::sdk::SourceCallsite,
        ) -> Result<()> {
            identity.verify_allocation()?;
            if identity.class() != self.class || source != &self.source {
                return Err("Reference SnippetChoicePanel partial cleanup class or source differs".into());
            }
            let offset = self.admission.verify_reference_snippet_panel_cleanup()?;
            identity.verify_allocation()?;
            let pointer = identity.receiver();
            let slot = pointer.checked_add(offset).ok_or("Reference SnippetChoicePanel model address overflow")?;
            if unsafe { &*(pointer as *const AnyObject) }.class() != self.class
                || unsafe { (slot as *const usize).read_volatile() } != 0
            {
                return Err("Reference SnippetChoicePanel partial cleanup requires an empty model slot".into());
            }
            Ok(())
        }
        fn verify_sender(&self, _: &AnyClass, _: Sel, _: usize, _: &crate::sdk::SourceCallsite,
            _: &AnyClass) -> Result<()> {
            Err("Reference SnippetChoicePanel construction grants no action dispatch".into())
        }
        fn verify_method(&self, _: &AnyClass, _: Sel, _: usize, _: &crate::sdk::SourceCallsite) -> Result<()> {
            Err("Reference SnippetChoicePanel construction grants no callback methods".into())
        }
    }

    let context = admission.app_delegate_context(model.clone())?;
    context.verify_reference_snippet_panel_constructor()?;
    let published_metadata = admission.observe_reference_snippet_published_destroy()?;
    let handle = crate::sdk::metadata::loaded_class(c"RuycastF1Reference.SnippetChoicePanel")?;
    let class = unsafe { &*(handle.address() as *const AnyClass) };
    let parent = class.superclass().ok_or("Reference SnippetChoicePanel parent absent")?;
    let mut selected = guard.lifecycle_bindings_for(parent)?;
    selected.push(super::receiver::LifecycleBinding::snippet_panel_destructor(admission.clone(), published_metadata)?);
    for selector in super::objc::REFERENCE_SNIPPET_APPEARANCE_SELECTORS.iter().map(|&(_, selector)| selector)
        .chain(super::objc::REFERENCE_SNIPPET_WINDOW_SELECTORS.iter().copied())
    {
        let leaf = unsafe { guard.shadow_reference_panel_leaf(&admission, class, selector) }?;
        selected.push(super::receiver::LifecycleBinding::leaf(leaf));
    }
    let source = reference_snippet_panel_construction_callsite();
    let authority = Arc::new(SnippetPanelConstruction {
        admission, context, class, source: source.clone(), published_metadata,
    });
    let grant = unsafe { WorkspaceGrant::for_original(model, class, source, selected, Vec::new(), authority) }?;
    grant.verify_owned_cleanup()?;
    unsafe { publish_source_construction(Arc::new(grant)) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) unsafe fn prepare_reference_palette_construction(
    admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    model: Arc<Workspace>,
    guard: &super::classes::ClassGuard,
) -> Result<()> {
    use super::delegate::CallbackCodeAuthority;
    use objc2::runtime::Sel;

    struct PaletteConstruction {
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        class: &'static AnyClass,
        source: crate::sdk::SourceCallsite,
    }

    unsafe impl CallbackCodeAuthority for PaletteConstruction {
        fn verify_source_construction(&self, class: &AnyClass, source: &crate::sdk::SourceCallsite) -> Result<()> {
            if class != self.class || source != &self.source {
                return Err("Reference Palette construction class/source differs".into());
            }
            self.admission.verify_reference_palette_panel_source()
        }

        fn verify_source_partial_cleanup(&self, identity: &super::receiver::SourceConstructionIdentity<'_>,
            source: &crate::sdk::SourceCallsite) -> Result<()> {
            identity.verify_allocation()?;
            if identity.class() != self.class || source != &self.source {
                return Err("Reference Palette partial cleanup class/source differs".into());
            }
            self.admission.verify_reference_palette_panel_metadata()?;
            identity.verify_allocation()?;
            let receiver = identity.receiver() as *const AnyObject;
            if unsafe { &*receiver }.class() != self.class {
                return Err("Reference Palette partial cleanup actual class differs".into());
            }
            verify_empty_palette_closures(receiver, self.class)
        }

        fn verify_sender(&self, _: &AnyClass, _: Sel, _: usize, _: &crate::sdk::SourceCallsite,
            _: &AnyClass) -> Result<()> {
            Err("Reference Palette construction grants no action dispatch".into())
        }

        fn verify_method(&self, _: &AnyClass, _: Sel, _: usize, _: &crate::sdk::SourceCallsite) -> Result<()> {
            Err("Reference Palette construction grants no callback methods".into())
        }
    }

    model.verify_reference_context(&admission)?;
    admission.verify_reference_palette_panel_source()?;
    let handle = crate::sdk::metadata::loaded_class(c"RuycastF1Reference.PalettePanel")?;
    let class = unsafe { &*(handle.address() as *const AnyClass) };
    let parent = class.superclass().ok_or("Reference Palette parent absent")?;
    let mut selected = guard.lifecycle_bindings_for(parent)?;
    selected.push(super::receiver::LifecycleBinding::palette_destructor(admission.clone())?);
    for &selector in super::objc::REFERENCE_PALETTE_WINDOW_SELECTORS {
        let leaf = unsafe { guard.shadow_reference_panel_leaf(&admission, class, selector) }?;
        selected.push(super::receiver::LifecycleBinding::leaf(leaf));
    }
    let source = reference_palette_construction_callsite();
    let authority = Arc::new(PaletteConstruction { admission, class, source: source.clone() });
    let grant = unsafe { WorkspaceGrant::for_original(model, class, source, selected, Vec::new(), authority) }?;
    grant.verify_owned_cleanup()?;
    unsafe { publish_source_construction(Arc::new(grant)) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn verify_empty_palette_closures(pointer: *const AnyObject, class: &AnyClass) -> Result<()> {
    let fields = class.instance_variables();
    for name in [c"actions", c"itemBindingKey"] {
        let field = fields.iter().find(|ivar| ivar.name() == name)
            .ok_or("Reference PalettePanel closure field absent")?;
        let offset = usize::try_from(field.offset()).map_err(|_| "Reference PalettePanel closure offset differs")?;
        if !offset.is_multiple_of(8) || offset.checked_add(16).is_none_or(|end| end > class.instance_size())
            || unsafe { pointer.cast::<u8>().add(offset).cast::<[usize; 2]>().read_volatile() } != [0, 0]
        {
            return Err("Reference PalettePanel closure not initialized to nil".into());
        }
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) unsafe fn prepare_reference_edit_construction(
    admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>, model: Arc<Workspace>,
) -> Result<()> {
    use super::delegate::CallbackCodeAuthority;
    use objc2::runtime::Sel;

    struct EditConstruction {
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        class: &'static AnyClass,
        source: crate::sdk::SourceCallsite,
        destructor: crate::sdk::metadata::MethodSnapshot,
    }
    unsafe impl CallbackCodeAuthority for EditConstruction {
        fn verify_source_construction(&self, class: &AnyClass, source: &crate::sdk::SourceCallsite) -> Result<()> {
            if class != self.class || source != &self.source {
                return Err("Reference Edit construction class/source differs".into());
            }
            self.admission.verify_native_edit_construction_source()
        }
        fn verify_source_partial_cleanup(&self, identity: &super::receiver::SourceConstructionIdentity<'_>,
            source: &crate::sdk::SourceCallsite) -> Result<()> {
            identity.verify_allocation()?;
            self.verify_source_construction(identity.class(), source)?;
            identity.verify_allocation()?;
            let pointer = identity.receiver() as *const AnyObject;
            if unsafe { &*pointer }.class() != self.class {
                return Err("Reference Edit partial cleanup class differs".into());
            }
            let window = unsafe { pointer.cast::<usize>().add(1).cast::<[usize; 2]>().read_volatile() };
            if window != [0, 0] && window != [native_edit_super::window_function()?, 0] {
                return Err("Reference Edit partial cleanup has foreign closure storage".into());
            }
            identity.verify_allocation()
        }
        fn verify_sender(&self, _: &AnyClass, _: Sel, _: usize, _: &crate::sdk::SourceCallsite,
            _: &AnyClass) -> Result<()> {
            Err("Reference Edit construction grants no action dispatch".into())
        }
        fn verify_method(&self, class: &AnyClass, selector: Sel, imp: usize,
            source: &crate::sdk::SourceCallsite) -> Result<()> {
            if selector.name() != c".cxx_destruct" || imp != self.destructor.implementation() {
                return Err("Reference Edit construction grants only its destructor metadata".into());
            }
            self.verify_source_construction(class, source)?;
            let handle = crate::sdk::metadata::registered_class(class as *const _ as usize)?;
            crate::sdk::metadata::verify_current_method(handle, c".cxx_destruct", false, &self.destructor)
        }
    }

    model.verify_reference_context(&admission)?;
    admission.verify_native_edit_construction_source()?;
    let handle = crate::sdk::metadata::loaded_class(c"RuycastF1Reference.NativeEditCommands")?;
    let class = unsafe { &*(handle.address() as *const AnyClass) };
    let source = reference_edit_construction_callsite();
    let guard = unsafe { super::classes::ClassGuard::select(&[class], model.lease()) }?;
    let destructor = crate::sdk::metadata::method(handle, c".cxx_destruct", false)?
        .ok_or("Reference Edit destructor absent")?;
    let authority = Arc::new(EditConstruction { admission, class, source: source.clone(), destructor });
    let mut selected = guard.lifecycle_bindings();
    selected.push(unsafe {
        super::receiver::LifecycleBinding::source(class.name(), c".cxx_destruct", false, source.clone(), authority.clone())
    }?);
    let grant = unsafe { WorkspaceGrant::for_original(model, class, source, selected, Vec::new(), authority) }?;
    unsafe { publish_source_construction(Arc::new(grant)) }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
mod native_edit_super {
    use super::*;
    use objc2::runtime::Sel;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[link(name = "objc")]
    unsafe extern "C" {
        #[link_name = "objc_msgSendSuper2"]
        static ORIGINAL: u8;
    }
    static CALLER: AtomicUsize = AtomicUsize::new(0);
    static ALLOC_CALLER: AtomicUsize = AtomicUsize::new(0);
    static PALETTE_INIT_CALLER: AtomicUsize = AtomicUsize::new(0);
    static PALETTE_KEY_CALLER: AtomicUsize = AtomicUsize::new(0);
    static SNIPPET_INIT_CALLER: AtomicUsize = AtomicUsize::new(0);
    static ORIGINAL_ENTRY: AtomicUsize = AtomicUsize::new(0);
    // Only the exclusive workspace import transaction may move the phase.
    static INSTALL_PHASE: AtomicUsize = AtomicUsize::new(0);

    pub(super) fn advance(from: usize, to: usize) -> Result<()> {
        INSTALL_PHASE.compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ()).map_err(|_| "Reference Edit super-init install phase differs".into())
    }

    pub(super) fn verify_selection(row: &crate::sdk::ImportReplacement) -> Result<()> {
        let spec = spec();
        if row.symbol != spec.symbol || row.original != spec.original || row.replacement != spec.replacement {
            return Err("Reference Edit super-init selection has foreign binding".into());
        }
        let selected = row.selected_address();
        let valid = match INSTALL_PHASE.load(Ordering::Acquire) {
            0 => selected == spec.original,
            1 => selected == spec.original || selected == spec.replacement,
            2 => selected == spec.replacement,
            _ => false,
        };
        if valid { Ok(()) } else { Err("Reference Edit super-init selection differs".into()) }
    }


    pub(super) fn spec() -> crate::sdk::ImportSpec {
        crate::sdk::ImportSpec {
            symbol: c"objc_msgSendSuper2",
            original: (&raw const ORIGINAL) as usize,
            replacement: entry as *const () as usize,
        }
    }

    pub(super) fn prepare(caller: usize, alloc_caller: usize, palette_init: usize, palette_key: usize, snippet_init: usize) -> Result<()> {
        if caller == 0 || alloc_caller == 0 || palette_init == 0 || palette_key == 0 || snippet_init == 0 {
            return Err("Reference Edit constructor caller absent".into());
        }
        let original = spec().original;
        for (slot, address) in [(&ORIGINAL_ENTRY, original), (&CALLER, caller), (&ALLOC_CALLER, alloc_caller),
            (&PALETTE_INIT_CALLER, palette_init), (&PALETTE_KEY_CALLER, palette_key),
            (&SNIPPET_INIT_CALLER, snippet_init)] {
            match slot.compare_exchange(0, address, Ordering::Release, Ordering::Acquire) {
                Ok(_) => {}
                Err(prior) if prior == address => {}
                Err(_) => return Err("Reference Edit super-init binding changed".into()),
            }
        }
        Ok(())
    }

    pub(super) fn is_alloc_caller(caller: usize) -> bool {
        caller != 0 && caller == ALLOC_CALLER.load(Ordering::Acquire)
    }

    pub(super) fn window_function() -> Result<usize> {
        const INIT_RETURN_OFFSET: usize = 0x1443e0;
        const WINDOW_FUNCTION_OFFSET: usize = 0x1444d4;
        let caller = CALLER.load(Ordering::Acquire);
        if caller == 0 {
            return Err("Reference Edit super-init caller absent".into());
        }
        caller.checked_add(WINDOW_FUNCTION_OFFSET - INIT_RETURN_OFFSET)
            .ok_or_else(|| "Reference Edit window function overflow".into())
    }

    pub(super) fn verify_alloc_caller(caller: usize) -> Result<()> {
        if INSTALL_PHASE.load(Ordering::Acquire) != 2
            || !is_alloc_caller(caller)
            || unsafe { libc::pthread_main_np() } != 1
            || crate::sdk::process_fork::is_child()
        {
            return Err("Reference Edit allocation caller/thread differs".into());
        }
        Ok(())
    }

    #[repr(C)]
    struct Super {
        receiver: *mut AnyObject,
        class: *const AnyClass,
    }

    #[unsafe(naked)]
    unsafe extern "C" fn entry() {
        core::arch::naked_asm!(
            "adrp x16, {caller}@PAGE",
            "add x16, x16, {caller}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 2f",
            "adrp x16, {palette}@PAGE",
            "add x16, x16, {palette}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 3f",
            "adrp x16, {palette_key}@PAGE",
            "add x16, x16, {palette_key}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 4f",
            "adrp x16, {snippet}@PAGE",
            "add x16, x16, {snippet}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 5f",
            "adrp x16, {original}@PAGE",
            "add x16, x16, {original}@PAGEOFF",
            "ldar x16, [x16]",
            "br x16",
            "2:",
            "b {initialize}",
            "3:",
            "b {initialize_palette}",
            "4:",
            "b {palette_key_equivalent}",
            "5:",
            "b {initialize_snippet_panel}",
            caller = sym CALLER,
            palette = sym PALETTE_INIT_CALLER,
            palette_key = sym PALETTE_KEY_CALLER,
            snippet = sym SNIPPET_INIT_CALLER,
            original = sym ORIGINAL_ENTRY,
            initialize = sym initialize,
            initialize_palette = sym initialize_palette,
            palette_key_equivalent = sym palette_key_equivalent,
            initialize_snippet_panel = sym initialize_snippet_panel,
        );
    }

    unsafe extern "C" fn initialize_snippet_panel(
        frame: *const Super, selector: Sel, style: usize, backing: usize,
        defer: objc2::runtime::Bool, rect: objc2_foundation::NSRect,
    ) -> *mut AnyObject {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            if INSTALL_PHASE.load(Ordering::Acquire) != 2
                || unsafe { libc::pthread_main_np() } != 1
                || crate::sdk::process_fork::is_child()
                || selector.name() != c"initWithContentRect:styleMask:backing:defer:"
                || style != 128 || backing != 2 || defer.as_bool()
            {
                return Err("Reference SnippetChoicePanel super-init arguments differ".into());
            }
            let frame = unsafe { frame.as_ref() }.ok_or("Reference SnippetChoicePanel super frame absent")?;
            let grant = source_allocation_grant(frame.receiver)?;
            if !grant.is_reference_snippet_panel_construction()
                || frame.class != grant.class() as *const AnyClass
            {
                return Err("Reference SnippetChoicePanel super-init class or grant differs".into());
            }
            let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
            let token = unsafe { grant.before_super_init(frame.receiver)? };
            let model = grant.snippet_panel_model(&token)?;
            admission.verify_reference_snippet_panel_model_slot(frame.receiver as usize, model, rect)?;
            if stopped.load(Ordering::Acquire) {
                return Err("Reference SnippetChoicePanel super-init ingress closed".into());
            }
            super::super::objc::initialize_reference_panel_payload(frame.receiver, rect, &grant, token)?;
            admission.verify_reference_snippet_panel_model_slot(frame.receiver as usize, model, rect)?;
            if stopped.load(Ordering::Acquire) {
                return Err("Reference SnippetChoicePanel stopped during super-init".into());
            }
            Ok(frame.receiver)
        }));
        match result {
            Ok(Ok(receiver)) => receiver,
            Err(payload) => { std::mem::forget(payload); unsafe { libc::_exit(95) } }
            Ok(Err(_)) => unsafe { libc::_exit(95) },
        }
    }

    /// Send the original fallback to the real retained content of the
    /// private panel, whose NSWindow ivars have never been initialized.
    unsafe extern "C" fn palette_key_equivalent(
        frame: *const Super, selector: Sel, event: *mut AnyObject,
    ) -> objc2::runtime::Bool {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            if INSTALL_PHASE.load(Ordering::Acquire) != 2 || unsafe { libc::pthread_main_np() } != 1
                || crate::sdk::process_fork::is_child() || selector.name() != c"performKeyEquivalent:"
            {
                return Err("Reference Palette key fallback owner or selector differs".into());
            }
            let frame = unsafe { frame.as_ref() }.ok_or("Reference Palette key frame absent")?;
            let grant = source_allocation_grant(frame.receiver)?;
            if !grant.is_reference_palette_construction() || frame.class != grant.class() as *const AnyClass {
                return Err("Reference Palette key fallback class differs".into());
            }
            super::super::objc::reference_palette_key_fallback(frame.receiver, event)
        }));
        match result {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) | Err(_) => unsafe { libc::_exit(95) },
        }
    }

    unsafe extern "C" fn initialize_palette(
        frame: *const Super, selector: Sel, style: usize, backing: usize,
        defer: objc2::runtime::Bool, rect: objc2_foundation::NSRect,
    ) -> *mut AnyObject {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            if INSTALL_PHASE.load(Ordering::Acquire) != 2 || unsafe { libc::pthread_main_np() } != 1
                || crate::sdk::process_fork::is_child()
                || selector.name() != c"initWithContentRect:styleMask:backing:defer:"
                || style != 128 || backing != 2 || defer.as_bool()
                || rect != objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(760.0, 490.0),
                )
            {
                return Err("Reference PalettePanel super-init arguments differ".into());
            }
            let frame = unsafe { frame.as_ref() }.ok_or("Reference PalettePanel super-init frame absent")?;
            let grant = source_allocation_grant(frame.receiver)?;
            let class = crate::sdk::metadata::loaded_class(c"RuycastF1Reference.PalettePanel")?;
            if frame.class as usize != class.address()
                || grant.class() as *const AnyClass as usize != class.address()
                || !grant.is_reference_palette_construction()
            {
                return Err("Reference PalettePanel super-init class or grant differs".into());
            }
            let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
            admission.verify_reference_palette_panel_source()?;
            let token = unsafe { grant.before_super_init(frame.receiver)? };
            verify_empty_palette_closures(frame.receiver, grant.class())?;
            if stopped.load(Ordering::Acquire) {
                return Err("Reference PalettePanel super-init ingress closed".into());
            }
            super::super::objc::initialize_reference_panel_payload(frame.receiver, rect, &grant, token)?;
            if stopped.load(Ordering::Acquire) {
                return Err("Reference PalettePanel stopped during super-init".into());
            }
            Ok(frame.receiver)
        }));
        match result {
            Ok(Ok(receiver)) => receiver,
            Err(payload) => { std::mem::forget(payload); unsafe { libc::_exit(95) } }
            Ok(Err(_)) => unsafe { libc::_exit(95) },
        }
    }

    unsafe extern "C" fn initialize(frame: *const Super, selector: Sel) -> *mut AnyObject {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
            if INSTALL_PHASE.load(Ordering::Acquire) != 2
                || selector.name() != c"init" || unsafe { libc::pthread_main_np() } != 1
                || crate::sdk::process_fork::is_child()
            {
                return Err("Reference Edit super-init selector/thread differs".into());
            }
            let frame = unsafe { frame.as_ref() }.ok_or("Reference Edit super-init frame absent")?;
            let grant = source_allocation_grant(frame.receiver)?;
            let class = crate::sdk::metadata::loaded_class(c"RuycastF1Reference.NativeEditCommands")?;
            if frame.class as usize != class.address()
                || grant.class() as *const AnyClass as usize != class.address()
                || !grant.is_original_source()
            {
                return Err("Reference Edit super-init class or grant differs".into());
            }
            // The pinned singleton stores a thin function and a nil context
            // in its only ivar. Observe the live allocation before dispatch.
            let function = window_function()?;
            let token = unsafe { grant.before_super_init(frame.receiver)? };
            let window = unsafe { frame.receiver.cast::<usize>().add(1).cast::<[usize; 2]>().read() };
            if window != [function, 0] {
                // Only a successful claim proves this call owns the phase.
                // Super-init has not consumed the construction +1 yet.
                unsafe { grant.cleanup_source_partial(frame.receiver) }?;
                return Err("Reference Edit singleton window function or context differs".into());
            }
            let original: unsafe extern "C-unwind" fn(*const Super, Sel) -> *mut AnyObject =
                unsafe { std::mem::transmute(spec().original) };
            let initialized = unsafe { original(frame, selector) };
            if initialized != frame.receiver {
                return Err("Reference Edit super-init changed the allocation".into());
            }
            grant.initialized(token)?;
            Ok(initialized)
        }));
        match result {
            Ok(Ok(receiver)) => receiver,
            Err(payload) => { std::mem::forget(payload); unsafe { libc::_exit(95) } }
            Ok(Err(_)) => unsafe { libc::_exit(95) },
        }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_edit_super_spec() -> crate::sdk::ImportSpec {
    native_edit_super::spec()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn prepare_reference_edit_super(caller: usize, alloc_caller: usize, palette_init: usize, palette_key: usize, snippet_init: usize) -> Result<()> {
    native_edit_super::prepare(caller, alloc_caller, palette_init, palette_key, snippet_init)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_edit_alloc_caller(caller: usize) -> Result<()> {
    native_edit_super::verify_alloc_caller(caller)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn is_reference_edit_alloc_caller(caller: usize) -> bool {
    native_edit_super::is_alloc_caller(caller)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn advance_reference_edit_super_install(from: usize, to: usize) -> Result<()> {
    native_edit_super::advance(from, to)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_edit_super_selection(row: &crate::sdk::ImportReplacement) -> Result<()> {
    native_edit_super::verify_selection(row)
}

/// Transfer the already retained initialized root, rather than creating another
/// F1Box or retaining an arbitrary Swift pointer. Caller still proves children.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub fn take_initialized_app_delegate_root(model:&Arc<Workspace>)->Result<Arc<InitializedAppDelegateRoot>> {
    let class=crate::sdk::metadata::loaded_class(c"RuycastF1Reference.AppDelegate")?;
    let grant=grant_for(unsafe{&*(class.address() as *const AnyClass)})?;
    super::receiver::app_delegate::constructor::take_root(grant,model)
}

/// Observe the real class-local original callback qualifier. Construction,
/// sender and final/secondary ARC are independently checked by the authority;
/// an incomplete original closure refuses actual catalog qualification.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub fn prepare_app_delegate_callback_authority(
    context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<Workspace>,
)->Result<Arc<dyn super::delegate::CallbackCodeAuthority>> {
    Ok(super::receiver::app_delegate::constructor::callbacks::AppDelegateCallbackAuthority::observe(context,model)?)
}
