//! Real selected Objective-C delegate/control callbacks. Original source is
//! never invoked before complete SDK admission. Fixture-only constructor is
//! unavailable outside tests and accepts only NSObject-rooted own classes.
use super::*;
use event_loop::{Callback, Lifecycle};
#[cfg(test)]
use objc2::ClassType;
use objc2::{
    rc::Retained,
    runtime::{AnyClass, AnyObject, Sel},
};
#[cfg(test)]
use objc2_foundation::NSObject;
use objc2_foundation::NSNotification;
#[cfg(test)]
use objc2_foundation::NSString;
use std::{
    ffi::CStr,
    thread::{self, ThreadId},
};
struct Selected {
    selector: Sel,
    imp: usize,
    arguments: usize,
    snapshot: crate::sdk::metadata::MethodSnapshot,
}
// Rust-panic containment only. This does not qualify Objective-C/C++ exceptions
// or any original entrypoint/source ABI. Those require separate admission.
fn contain_rust_call(
    retired: &AtomicBool,
    business_faulted: &AtomicBool,
    fault: impl FnOnce(),
    call: impl FnOnce(),
) -> Result<()> {
    if retired.load(Ordering::Acquire) || business_faulted.load(Ordering::Acquire) {
        return Err("selected callback admission retired".into());
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(()) => Ok(()),
        Err(payload) => {
            retired.store(true, Ordering::Release);
            fault();
            std::mem::forget(payload);
            Err("selected Rust callback panicked; unknown outcome, no retry".into())
        }
    }
}
/// REQUIRED common-coordinator capability; no production implementation,
/// default or closure adapter is provided by this slice.
/// # Safety
/// Prove class+selector -> method entry -> exact bound source/image/code,
/// including Swift trampolines/aliases and strip/dSYM bindings. A pointer in a
/// pinned .text range alone is insufficient. Qualify late classes before use.
pub unsafe trait CallbackCodeAuthority: Send + Sync {
    /// Qualify the actual compiler-owned alloc/set_ivars/super-init caller and
    /// every inert CXX construction/drop-flag path before allocation. This is
    /// construction permission only, never full application callback admission.
    fn verify_source_construction(
        &self,
        _class: &AnyClass,
        _source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        Err("original source construction authority absent".into())
    }
    /// Qualify physical cleanup of this exact known partial allocation. A
    /// partial identity is not an initialized receiver or a business grant.
    fn verify_source_partial_cleanup(
        &self,
        _receiver: &super::receiver::SourceConstructionIdentity<'_>,
        _source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        Err("original source partial cleanup authority absent".into())
    }
    /// Qualify this class's complete retain/release/dealloc/autorelease and CXX ancestor
    /// paths and weak-reference RR entrypoints (allowsWeakReference and
    /// retainWeakReference), including source ownership. Bind the weak target's
    /// object identity/isa stability without dereferencing a dead raw target;
    /// qualification must cover all secondary code, never just business IMPs.
    /// This fail-closed default grants no original ARC/destructor execution.
    fn verify_cleanup(
        &self,
        _class: &AnyClass,
        _source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        Err("original callback cleanup authority absent".into())
    }
    /// Bind this exact observed original allocation, including serial, source
    /// initialization/deallocation and isa stability. Class metadata alone
    /// grants neither an object lifetime nor original reference-counting code.
    fn verify_receiver_cleanup(
        &self,
        _receiver: &super::receiver::ReceiverIdentity,
        _source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        Err("original callback receiver cleanup authority absent".into())
    }
    /// Prove the action's source-declared parameter class; plain '@' ABI does
    /// not establish NSMenuItem versus another otherwise valid owned object.
    fn verify_sender(
        &self,
        class: &AnyClass,
        selector: Sel,
        actual_imp: usize,
        source: &crate::sdk::SourceCallsite,
        sender_class: &AnyClass,
    ) -> Result<()>;
    /// Bind the compiler's real Objective-C entry ABI (including unwind ABI),
    /// not merely runtime type encoding. The selected caller permits C-unwind;
    /// original Rust entry/dealloc/drop-glue qualification remains mandatory.
    fn verify_method(
        &self,
        class: &AnyClass,
        selector: Sel,
        actual_imp: usize,
        source: &crate::sdk::SourceCallsite,
    ) -> Result<()>;

    /// Verify a set of selected methods under the same class/source contract.
    fn verify_methods(
        &self,
        class: &AnyClass,
        methods: &[(&CStr, usize)],
        source: &crate::sdk::SourceCallsite,
    ) -> Result<()> {
        if methods.is_empty() {
            return Err("callback method roster empty".into());
        }
        for &(selector, implementation) in methods {
            self.verify_method(class, Sel::register(selector), implementation, source)?;
        }
        Ok(())
    }
}
struct Cleanup {
    class: &'static AnyClass,
    handle: crate::sdk::metadata::ClassHandle,
    lifecycle: crate::sdk::metadata::LifecycleSnapshot,
    arc: Vec<crate::sdk::metadata::MethodSnapshot>,
    weak: Vec<Option<crate::sdk::metadata::MethodSnapshot>>,
    root: Option<super::factory::RootDeallocator>,
}
enum Identity {
    Observed(super::receiver::ReceiverIdentity),
    #[cfg(test)]
    Fixture {
        receiver: usize,
        owner: ThreadId,
    },
}
impl Identity {
    fn capture(model: &Arc<Workspace>, receiver: *mut AnyObject, original: bool) -> Result<Self> {
        if original {
            let identity = super::factory::receiver_identity(receiver)?;
            if !identity.belongs_to(model) {
                return Err("callback receiver belongs to another workspace".into());
            }
            identity.verify_pure()?;
            return Ok(Self::Observed(identity));
        }
        #[cfg(test)]
        {
            Ok(Self::Fixture {
                receiver: receiver as usize,
                owner: thread::current().id(),
            })
        }
        #[cfg(not(test))]
        {
            Err("callback fixture construction unavailable".into())
        }
    }
    fn observed(&self) -> Option<&super::receiver::ReceiverIdentity> {
        match self {
            Self::Observed(identity) => Some(identity),
            #[cfg(test)]
            Self::Fixture { .. } => None,
        }
    }
    fn pointer(&self) -> usize {
        match self {
            Self::Observed(identity) => identity.receiver(),
            #[cfg(test)]
            Self::Fixture { receiver, .. } => *receiver,
        }
    }
    fn allocation(&self) -> Result<()> {
        match self {
            Self::Observed(identity) => identity.verify_allocation(),
            #[cfg(test)]
            Self::Fixture { owner, .. } if *owner == thread::current().id() => Ok(()),
            #[cfg(test)]
            Self::Fixture { .. } => Err("callback fixture on foreign owner thread".into()),
        }
    }
    fn pure(&self) -> Result<()> {
        match self {
            Self::Observed(identity) => identity.verify_pure(),
            #[cfg(test)]
            Self::Fixture { .. } => self.allocation(),
        }
    }
}
impl Cleanup {
    fn capture(class: &'static AnyClass, original: bool) -> Result<Self> {
        let handle = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
        let lifecycle = crate::sdk::metadata::lifecycle(handle)?;
        let mut arc = vec![];
        for name in [c"retain", c"release", c"dealloc", c"autorelease"] {
            arc.push(
                crate::sdk::metadata::method(handle, name, false)?
                    .ok_or("callback ARC method missing")?,
            );
        }
        let weak = [c"allowsWeakReference", c"retainWeakReference"]
            .iter()
            .map(|name| crate::sdk::metadata::method(handle, name, false))
            .collect::<Result<Vec<_>>>()?;
        let root = if original {
            None
        } else {
            let root = super::factory::RootDeallocator::capture()?;
            root.verify_proxy_arc(class)?;
            let nsobject = crate::sdk::metadata::loaded_class(c"NSObject")?;
            let dealloc = crate::sdk::metadata::method(nsobject, c"dealloc", false)?
                .ok_or("NSObject dealloc missing")?;
            for (name, actual) in [c"allowsWeakReference", c"retainWeakReference"]
                .iter()
                .zip(&weak)
            {
                let expected = crate::sdk::metadata::method(nsobject, name, false)?;
                if actual
                    .as_ref()
                    .map(|m| (m.owner(), m.implementation(), m.code()))
                    != expected
                        .as_ref()
                        .map(|m| (m.owner(), m.implementation(), m.code()))
                {
                    return Err("owned callback weak RR must be inherited from NSObject".into());
                }
            }
            if arc[2].owner() != nsobject.address()
                || arc[2].implementation() != dealloc.implementation()
                || arc[2].code() != dealloc.code()
                || [c".cxx_construct", c".cxx_destruct"].iter().any(|name| {
                    crate::sdk::metadata::method(handle, name, false).map_or(true, |m| m.is_some())
                })
            {
                return Err(
                    "owned callback requires NSObject-only ARC/dealloc and no CXX cleanup".into(),
                );
            }
            Some(root)
        };
        Ok(Self {
            class,
            handle,
            lifecycle,
            arc,
            weak,
            root,
        })
    }
    fn verify(&self) -> Result<()> {
        if let Some(root) = &self.root {
            root.verify_proxy_arc(self.class)?;
        }
        if crate::sdk::metadata::lifecycle(self.handle)? != self.lifecycle {
            return Err("callback cleanup lifecycle changed".into());
        }
        for (name, expected) in [c"retain", c"release", c"dealloc", c"autorelease"]
            .iter()
            .zip(&self.arc)
        {
            if crate::sdk::metadata::method(self.handle, name, false)?.as_ref() != Some(expected) {
                return Err("callback ARC identity changed".into());
            }
        }
        for (name, expected) in [c"allowsWeakReference", c"retainWeakReference"]
            .iter()
            .zip(&self.weak)
        {
            if &crate::sdk::metadata::method(self.handle, name, false)? != expected {
                return Err("callback weak RR identity changed".into());
            }
        }
        Ok(())
    }
}
pub struct ObjectiveC {
    model: Arc<Workspace>,
    target: usize,
    application: usize,
    class: &'static AnyClass,
    owner: ThreadId,
    methods: BTreeMap<String, Selected>,
    original: bool,
    authority: Option<Arc<dyn CallbackCodeAuthority>>,
    source: Option<crate::sdk::SourceCallsite>,
    cleanup: [Cleanup; 2],
    identities: [Identity; 2],
    retired: AtomicBool,
}
impl ObjectiveC {
    /// Already qualified class identity only; no receiver dereference or retain.
    pub(super) fn class_identity(&self) -> &'static AnyClass {
        self.class
    }
    pub(super) fn target_identity(&self) -> usize {
        self.target
    }
    /// # Safety
    /// Target is a live retained object of the exact source/artifact-bound image.
    /// Expected pointers are independently matched to that image's reviewed code,
    /// not untrusted receipt values. All listed actions are source-qualified.
    /// No constructor/source may be loaded to discover missing guards.
    pub unsafe fn from_original(
        model: Arc<Workspace>,
        target: *mut AnyObject,
        application: *mut AnyObject,
        expected: &[(&CStr, usize)],
        source: &crate::sdk::SourceCallsite,
        authority: Arc<dyn CallbackCodeAuthority>,
    ) -> Result<Self> {
        if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            return Err("complete SDK not admitted before original delegate".into());
        }
        if !source_roster()
            .iter()
            .any(|s| s.file == source.file && s.source_sha256 == source.source_sha256)
        {
            return Err("original delegate source binding differs".into());
        }
        if application.is_null() {
            return Err("missing owned application notification object".into());
        }
        unsafe {
            Self::selected(
                model,
                target,
                application,
                expected,
                Some((authority, source.clone())),
            )
        }
    }
    unsafe fn selected(
        model: Arc<Workspace>,
        target: *mut AnyObject,
        application: *mut AnyObject,
        expected: &[(&CStr, usize)],
        qualified: Option<(Arc<dyn CallbackCodeAuthority>, crate::sdk::SourceCallsite)>,
    ) -> Result<Self> {
        model.with(|_| Ok(()))?;
        if target.is_null() || application.is_null() {
            return Err("null delegate/control target".into());
        }
        let original = qualified.is_some();
        // Production lookup proves an existing observed allocation before any
        // raw receiver/class access. Fixtures retain the explicit unsafe live
        // construction-reference premise and cannot grant production authority.
        let identities = [
            Identity::capture(&model, target, original)?,
            Identity::capture(&model, application, original)?,
        ];
        if original
            && !identities[0]
                .observed()
                .is_some_and(|identity| identity.original_source())
        {
            return Err("original callback target has no original allocation identity".into());
        }
        let class = match &identities[0] {
            Identity::Observed(identity) => identity.class(),
            #[cfg(test)]
            Identity::Fixture { .. } => unsafe { &*target }.class(),
        };
        let app_class = match &identities[1] {
            Identity::Observed(identity) => identity.class(),
            #[cfg(test)]
            Identity::Fixture { .. } => unsafe { &*application }.class(),
        };
        let mut methods = BTreeMap::new();
        for (name, expected) in expected {
            let selector = Sel::register(name);
            let m =
                super::raw_method(class, selector, false)?.ok_or("selected callback missing")?;
            let abi = super::simple_abi(&m)?;
            if m.implementation() as usize != *expected
                || *expected == 0
                || ![vec![b'v', b'@', b':'], vec![b'v', b'@', b':', b'@']].contains(&abi)
            {
                return Err("callback IMP/void ABI differs".into());
            }
            if methods
                .insert(
                    name.to_string_lossy().into_owned(),
                    Selected {
                        selector,
                        imp: *expected,
                        arguments: abi.len() - 1,
                        snapshot: m,
                    },
                )
                .is_some()
            {
                return Err("duplicate selected callback".into());
            }
        }
        for selector in [
            c"applicationWillFinishLaunching:",
            c"applicationDidFinishLaunching:",
            c"applicationWillTerminate:",
        ] {
            if super::raw_method(class, Sel::register(selector), false)?.is_some()
                && !methods.contains_key(selector.to_str().map_err(|_| "nonUTF8 selector")?)
            {
                return Err("present lifecycle callback omitted from selection".into());
            }
        }
        let cleanup = [
            Cleanup::capture(class, original)?,
            Cleanup::capture(app_class, original)?,
        ];
        // This value owns only fields that have actually acquired a +1. Its
        // Drop requalifies cleanup independently of retired business admission;
        // no automatic Retained destructor runs on an acquisition error.
        let mut selected = Self {
            model,
            target: 0,
            application: 0,
            class,
            owner: thread::current().id(),
            methods,
            original,
            authority: qualified.as_ref().map(|(a, _)| a.clone()),
            source: qualified.map(|(_, s)| s),
            cleanup,
            identities,
            retired: AtomicBool::new(false),
        };
        selected.verify_acquisition()?;
        let held = unsafe { Retained::retain(target) }.ok_or("delegate retain failed")?;
        selected.target = Retained::into_raw(held) as usize;
        selected.verify_acquisition()?;
        let held = unsafe { Retained::retain(application) }.ok_or("application retain failed")?;
        selected.application = Retained::into_raw(held) as usize;
        selected.verify_acquisition()?;
        Ok(selected)
    }
    fn verify_pure(&self) -> Result<()> {
        self.verify_structure_pure()?;
        // Complete readiness invokes source/guardian verifiers and may reenter.
        // Keep it before the final argument fence, never inside the callback-
        // free structure check used after that fence.
        if self.original && crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            self.retired.store(true,Ordering::Release);
            return Err("original callback admission retired".into());
        }
        self.verify_structure_pure()
    }
    fn verify_structure_pure(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err("callback on foreign owner thread".into());
        }
        let result = (|| -> Result<()> {
            if self.retired.load(Ordering::Acquire) {
                return Err("callback proof permanently retired".into());
            }
            if self.model.faulted.load(Ordering::Acquire)
                || self
                    .model
                    .state
                    .lock()
                    .map_err(|_| "workspace state poisoned")?
                    .retired
            {
                return Err("callback business admission retired".into());
            }
            for (index, identity) in self.identities.iter().enumerate() {
                identity.pure()?;
                if let Some(identity) = identity.observed() {
                    if identity.class() != self.cleanup[index].class {
                        return Err("callback class differs from observed allocation".into());
                    }
                }
                let held = [self.target, self.application][index];
                if held != 0 && held != identity.pointer() {
                    return Err("callback retain returned another receiver".into());
                }
                if unsafe { &*(identity.pointer() as *mut AnyObject) }.class()
                    != self.cleanup[index].class
                {
                    return Err("callback receiver class changed".into());
                }
                self.cleanup[index].verify()?;
            }
            for selected in self.methods.values() {
                if super::raw_method(self.class, selected.selector, false)?.as_ref()
                    != Some(&selected.snapshot)
                {
                    return Err("callback selected metadata changed".into());
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        result
    }
    fn verify_acquisition(&self) -> Result<()> {
        // Wrong-thread use is refused without revoking the owner's intact proof.
        if thread::current().id() != self.owner {
            return Err("callback on foreign owner thread".into());
        }
        let result = (|| -> Result<()> {
            self.verify_pure()?;
            self.model.with(|_| Ok(()))?;
            self.verify_pure()?;
            for identity in &self.identities {
                if let Some(identity) = identity.observed() {
                    identity.verify()?;
                    self.verify_pure()?;
                    if identity.original_source() {
                        let authority = self
                            .authority
                            .as_ref()
                            .ok_or("callback cleanup authority absent")?;
                        let source = self
                            .source
                            .as_ref()
                            .ok_or("callback cleanup source absent")?;
                        authority.verify_cleanup(identity.class(), source)?;
                        self.verify_pure()?;
                        authority.verify_receiver_cleanup(identity, source)?;
                        self.verify_pure()?;
                    } else {
                        identity.verify_sdk_cleanup()?;
                        self.verify_pure()?;
                    }
                }
            }
            if let Some(authority) = &self.authority {
                for method in self.methods.values() {
                    authority.verify_method(
                        self.class,
                        method.selector,
                        method.imp,
                        self.source.as_ref().ok_or("callback code source absent")?,
                    )?;
                    self.verify_pure()?;
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        result
    }
    fn verify_reference_cleanup(&self, index: usize, pointer: usize) -> Result<()> {
        let identity = &self.identities[index];
        identity.allocation()?;
        if pointer != identity.pointer() {
            return Err("callback cleanup receiver identity differs".into());
        }
        if let Some(identity) = identity.observed() {
            if identity.original_source() {
                let authority = self
                    .authority
                    .as_ref()
                    .ok_or("callback cleanup authority absent")?;
                let source = self
                    .source
                    .as_ref()
                    .ok_or("callback cleanup source absent")?;
                authority.verify_cleanup(identity.class(), source)?;
                identity.verify_allocation()?;
                authority.verify_receiver_cleanup(identity, source)?;
                identity.verify_allocation()?;
                if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
                    return Err("original callback cleanup admission retired".into());
                }
            } else {
                identity.verify_sdk_cleanup()?;
                identity.verify_allocation()?;
            }
        }
        if unsafe { &*(pointer as *mut AnyObject) }.class() != self.cleanup[index].class {
            return Err("callback cleanup receiver class changed".into());
        }
        self.cleanup[index].verify()
    }
    fn invoke(&self, name: &str, argument: *mut AnyObject, optional: bool) -> Result<()> {
        self.invoke_checked(name, argument, optional, || Ok(()))
    }
    fn invoke_checked(
        &self,
        name: &str,
        argument: *mut AnyObject,
        optional: bool,
        check: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        self.verify_acquisition()?;
        if thread::current().id() != self.owner {
            return Err("callback on foreign owner thread".into());
        }
        if self.original && crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            return Err("original callback admission retired".into());
        }
        let Some(selected) = self.methods.get(name) else {
            if optional {
                return Ok(());
            }
            return Err("unknown/unselected control callback".into());
        };
        let target = self.target as *mut AnyObject;
        let class = unsafe { &*target }.class();
        if class != self.class
            || super::raw_method(class, selected.selector, false)?
                .ok_or("callback disappeared")?
                .implementation() as usize
                != selected.imp
        {
            return Err("callback target/IMP changed".into());
        }
        if self.original {
            self.authority
                .as_ref()
                .ok_or("original callback lacks code authority")?
                .verify_method(
                    class,
                    selected.selector,
                    selected.imp,
                    self.source
                        .as_ref()
                        .ok_or("original callback source disappeared")?,
                )?;
        }
        self.verify_acquisition()?;
        check()?;
        unsafe { self.call_selected(selected, argument) }?;
        Ok(())
    }
    // Call only after the last dynamic argument guard and pure rechecks of
    // receiver/selector/code identity; no foreign verifier runs in this helper.
    unsafe fn call_selected(&self, selected: &Selected, argument: *mut AnyObject) -> Result<()> {
        contain_rust_call(&self.retired, &self.model.faulted, || self.model.fault(), || {
            if selected.arguments == 2 {
                let call: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) =
                    unsafe { std::mem::transmute(selected.imp) };
                unsafe { call(self.target as *mut AnyObject, selected.selector) };
            } else {
                let call: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) =
                    unsafe { std::mem::transmute(selected.imp) };
                unsafe { call(self.target as *mut AnyObject, selected.selector, argument) };
            }
        })
    }
    pub(super) fn accepts_notification(
        &self,
        model: &Arc<Workspace>,
        selector: &str,
    ) -> Result<()> {
        if !Arc::ptr_eq(model, &self.model) {
            return Err("notification target belongs to a foreign workspace".into());
        }
        self.model.with(|_| Ok(()))?;
        if thread::current().id() != self.owner {
            return Err("notification target on foreign owner thread".into());
        }
        if self.methods.get(selector).is_none_or(|m| m.arguments != 3) {
            return Err("notification selector requires exactly one object argument".into());
        }
        Ok(())
    }
    /// Dispatch only a genuine, process-owned notification constructed by Hub.
    /// # Safety
    /// The notification and its object/userInfo are exclusively owned fixture
    /// data, never borrowed from a real desktop notification center.
    pub unsafe fn notification(&self, selector: &str, notice: &NSNotification) -> Result<()> {
        self.accepts_notification(&self.model, selector)?;
        #[cfg(test)]
        if super::objc::notification_fixture_payload(&self.model) {
            return self.invoke(selector,(notice as *const NSNotification).cast_mut().cast(),false);
        }
        self.invoke_owned_notification(selector,(notice as *const NSNotification).cast_mut().cast())
    }
    fn invoke_owned_notification(&self,selector:&str,notice:*mut AnyObject)->Result<()> {
        self.verify_acquisition()?;
        let selected=self.methods.get(selector)
            .filter(|selected|selected.arguments==3)
            .ok_or("notification selector absent or has another argument ABI")?;
        let identity=super::factory::receiver_identity(notice)?;
        if !identity.belongs_to(&self.model)||identity.original_source() {
            return Err("Notification belongs to another owned model".into());
        }
        // Capture the actual allocation/record, never merely its class. The
        // caller keeps the payload's +1 throughout this synchronous dispatch.
        let pure=super::objc::notification_payload_pure_check(notice,&self.model)?;
        identity.verify()?;
        self.verify_acquisition()?;
        if self.original {
            if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
                return Err("original notification callback not admitted".into());
            }
            self.authority
                .as_ref()
                .ok_or("notification code authority absent")?
                .verify_sender(
                    self.class,
                    selected.selector,
                    selected.imp,
                    self.source.as_ref().ok_or("notification source absent")?,
                    identity.class(),
                )?;
        }
        // No dynamic source/guardian verifier after this argument boundary.
        // Such a verifier may reenter and retire or replace the payload.
        self.verify_pure()?;
        identity.verify_pure()?;
        pure()?;
        self.verify_structure_pure()?;
        unsafe{self.call_selected(selected,notice)}
    }
    #[cfg(test)]
    pub(crate) unsafe fn owned_fixture(
        model: Arc<Workspace>,
        target: *mut AnyObject,
        expected: &[(&CStr, usize)],
    ) -> Result<Self> {
        if target.is_null() {
            return Err("null fixture".into());
        }
        let mut c = Some(unsafe { &*target }.class());
        while let Some(class) = c {
            if class == NSObject::class() {
                break;
            }
            if !class.name().to_bytes().starts_with(b"RuycastF1Owned") {
                return Err("fixture inherits a real SDK class".into());
            }
            c = class.superclass();
        }
        unsafe { Self::selected(model, target, target, expected, None) }
    }
}
impl Drop for ObjectiveC {
    fn drop(&mut self) {
        // Take only the +1s actually acquired, before either destructor can
        // reenter. Qualification is fresh for EACH release. Business retirement
        // alone does not invalidate independently proven physical cleanup.
        let references = [
            std::mem::replace(&mut self.target, 0),
            std::mem::replace(&mut self.application, 0),
        ];
        for (index, pointer) in references.into_iter().enumerate() {
            if pointer == 0 {
                continue;
            }
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    if thread::current().id() != self.owner {
                        return Err("callback cleanup on foreign owner thread".into());
                    }
                    self.verify_reference_cleanup(index, pointer)?;
                    drop(unsafe { Retained::from_raw(pointer as *mut AnyObject) });
                    Ok(())
                }));
            if let Err(payload) = result {
                std::mem::forget(payload);
            }
            // Err (including a foreign thread or changed ARC/isa) intentionally
            // leaves this particular real +1 quarantined, never auto-dropped.
        }
    }
}
impl Callback for ObjectiveC {
    fn native_event(&self, event: &super::objc::EventLease) -> Result<()> {
        if !event.handle().belongs_to(&self.model) {
            return Err("window event belongs to another workspace".into());
        }
        self.verify_acquisition()?;
        let selected = self
            .methods
            .get("sendEvent:")
            .filter(|selected| selected.arguments == 3)
            .ok_or("window has no selected source-qualified sendEvent: callback")?;
        let authority = self
            .authority
            .as_ref()
            .ok_or("window event code authority absent")?;
        authority.verify_sender(
            self.class,
            selected.selector,
            selected.imp,
            self.source.as_ref().ok_or("window event source absent")?,
            event.class()?,
        )?;
        self.verify_pure()?;
        event.handle().check_pure()?;
        // This is the LAST dynamic argument guard. Calling invoke() afterwards
        // could reenter and retire the event, so dispatch only through the pure
        // selected call after checking both already-owned identities again.
        let pointer = event.borrowed_pointer()?;
        self.verify_structure_pure()?;
        event.handle().check_pure()?;
        unsafe { self.call_selected(selected, pointer) }?;
        Ok(())
    }
    fn lifecycle(&self, event: Lifecycle) -> Result<()> {
        let (selector, name) = match event {
            Lifecycle::WillFinishLaunching => (
                "applicationWillFinishLaunching:",
                "NSApplicationWillFinishLaunchingNotification",
            ),
            Lifecycle::DidFinishLaunching => (
                "applicationDidFinishLaunching:",
                "NSApplicationDidFinishLaunchingNotification",
            ),
            Lifecycle::WillTerminate => (
                "applicationWillTerminate:",
                "NSApplicationWillTerminateNotification",
            ),
        };
        self.model.with(|_| Ok(()))?;
        #[cfg(test)]
        if super::objc::notification_fixture_payload(&self.model) {
            let notice = unsafe {
                NSNotification::notificationWithName_object(
                    &NSString::from_str(name),
                    Some(&*(self.application as *mut AnyObject)),
                )
            };
            return self.invoke(selector, Retained::as_ptr(&notice).cast_mut().cast(), true);
        }
        self.verify_acquisition()?;
        let Some(selected)=self.methods.get(selector) else {return Ok(());};
        let notice=super::objc::new_owned_notification(
            &self.model,name,Some(self.application as *mut AnyObject),None,
        )?;
        let notice_identity=super::factory::receiver_identity(notice.as_ptr())?;
        if !notice_identity.belongs_to(&self.model)||notice_identity.original_source() {
            return Err("Lifecycle notification belongs to another owned model".into());
        }
        let notice_pure=super::objc::notification_payload_pure_check(notice.as_ptr(),&self.model)?;
        // Finish every target/receiver dynamic guard BEFORE the last sender
        // verifier. invoke() must not run after that argument boundary.
        notice_identity.verify()?;
        self.verify_acquisition()?;
        if self.original {
            self.authority.as_ref().ok_or("Original lifecycle callback authority absent")?
                .verify_sender(self.class,selected.selector,selected.imp,
                    self.source.as_ref().ok_or("Original lifecycle callback source absent")?,
                    notice_identity.class())?;
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.original && event == Lifecycle::DidFinishLaunching {
            // Swift creates the ControlServer and resolves config.json in
            // this callback. Recheck the named profile at this boundary.
            crate::sdk::reference_runtime::verify_original_launch_profile(&self.model)?;
        }
        self.verify_pure()?;
        notice_identity.verify_pure()?;
        notice_pure()?;
        self.verify_structure_pure()?;
        // The actual payload +1 and captured exact record/serial remain held
        // through the whole generated To bridge/destroy, nested calls and Err.
        unsafe {self.call_selected(selected,notice.as_ptr())}
    }
    fn action(&self, selector: &str) -> Result<()> {
        if self.methods.get(selector).is_some_and(|m| m.arguments == 3) {
            return Err("sender-taking control requires an owned sender lease".into());
        }
        self.invoke(selector, std::ptr::null_mut(), false)
    }
    fn action_with_sender(
        &self,
        control: Handle,
        selector: &str,
        sender: &super::receiver::SenderLease,
    ) -> Result<()> {
        let p = sender.pointer()?;
        let selected = self
            .methods
            .get(selector)
            .ok_or("unknown sender-taking action")?;
        if selected.arguments == 3 && self.original {
            self.authority
                .as_ref()
                .ok_or("control sender lacks code authority")?
                .verify_sender(
                    self.class,
                    selected.selector,
                    selected.imp,
                    self.source.as_ref().ok_or("control source missing")?,
                    sender.class(),
                )?;
        }
        self.invoke_checked(selector, p, false, || {
            super::objc::controls::verify_action_control(control)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::runtime::ClassBuilder;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    #[ignore="explicit isolated owned-payload contract; needs authentic compiler pins and an exclusive SDK arena"]
    fn owned_notification_payload_dispatch_and_retirement() {
        objc2::rc::autoreleasepool(|_| {
            static CALLS:AtomicU64=AtomicU64::new(0);
            unsafe extern "C" fn received(_: *mut AnyObject,_:Sel,notice:*mut AnyObject) {
                let name:*mut AnyObject=unsafe{objc2::msg_send![notice,name]};
                let length:usize=unsafe{objc2::msg_send![name,length]};
                assert_eq!(length,"fixture.notification".len());
                let object:*mut AnyObject=unsafe{objc2::msg_send![notice,object]};
                let info:*mut AnyObject=unsafe{objc2::msg_send![notice,userInfo]};
                assert!(object.is_null()&&info.is_null());
                CALLS.fetch_add(1,Ordering::SeqCst);
            }
            let f=super::super::tests::Fixture::new();
            super::super::objc::install_fixture(f.model.clone()).unwrap();
            unsafe{super::super::objc::prepare_typed_data_factories_checked(
                f.model.clone(),||f.model.with(|_|Ok(())))}.unwrap();
            unsafe{super::super::objc::prepare_notification_payload_factory_checked(
                f.model.clone(),||f.model.with(|_|Ok(())))}.unwrap();
            let board=super::super::objc::clipboard_bridge::prove_private_board(&f.model);
            let mut builder=ClassBuilder::new(c"RuycastF1OwnedPayloadDispatch",NSObject::class()).unwrap();
            unsafe{builder.add_method(objc2::sel!(received:),received as unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject));}
            let class=builder.register();
            let target:Retained<NSObject>=unsafe{objc2::msg_send![class,new]};
            let pointer=Retained::as_ptr(&target).cast_mut().cast();
            let callback=unsafe{ObjectiveC::owned_fixture(f.model.clone(),pointer,
                &[(c"received:",received as *const () as usize)])}.unwrap();
            let notice=super::super::objc::new_owned_notification(&f.model,"fixture.notification",None,None).unwrap();
            // Deliberately consume the production helper, not the explicit
            // real-Foundation fallback retained by notification() in tests.
            callback.invoke_owned_notification("received:",notice.as_ptr()).unwrap();
            assert_eq!(CALLS.load(Ordering::SeqCst),1);
            assert!(callback.invoke_owned_notification("received:",pointer).is_err());
            assert_eq!(CALLS.load(Ordering::SeqCst),1,"foreign payload dispatched");
            f.model.retire().unwrap();
            super::super::objc::clipboard_bridge::prove_retired_private_board(board);
            assert!(callback.invoke_owned_notification("received:",notice.as_ptr()).is_err());
            assert_eq!(CALLS.load(Ordering::SeqCst),1,"retired payload dispatched");
        });
    }
    #[test]
    fn final_structure_check_never_calls_guardian_or_grants_original_admission() {
        let f=super::super::tests::Fixture::new();
        let class=ClassBuilder::new(c"RuycastF1OwnedFinalStructure",NSObject::class()).unwrap().register();
        let object:Retained<NSObject>=unsafe{objc2::msg_send![class,new]};
        let pointer=Retained::as_ptr(&object).cast_mut().cast();
        let mut callback=unsafe{ObjectiveC::owned_fixture(f.model.clone(),pointer,&[])}.unwrap();
        // Test only the structural predicate with the original bit set: no
        // original object/code is installed, invoked or granted by this test.
        // Any life.before_effect/is_alive callback now panics.
        callback.original=true;
        f.life.1.store(2,Ordering::Release);
        assert!(callback.verify_structure_pure().is_ok());
        assert_eq!(crate::sdk::ruycast_f1_preentry_ready_v1(),0);
        assert_eq!(callback.verify_pure().unwrap_err(),"original callback admission retired");
        assert!(callback.retired.load(Ordering::Acquire));
        callback.original=false;
        f.life.1.store(0,Ordering::Release);
    }
    #[test]
    fn correctly_typed_rust_unwind_is_contained_without_payload_drop_or_retry() {
        static CALLS: AtomicU64 = AtomicU64::new(0);
        static PAYLOAD_DROPS: AtomicU64 = AtomicU64::new(0);
        struct Payload;
        impl Drop for Payload {
            fn drop(&mut self) {
                PAYLOAD_DROPS.fetch_add(1, Ordering::SeqCst);
                panic!("unknown panic payload must not be dropped");
            }
        }
        unsafe extern "C-unwind" fn failing() {
            CALLS.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(Payload);
        }
        let retired = AtomicBool::new(false);
        let faulted = AtomicBool::new(false);
        let call: unsafe extern "C-unwind" fn() = failing;
        assert!(
            contain_rust_call(&retired, &faulted, || faulted.store(true, Ordering::Release), || unsafe { call() }).is_err(),
            "Rust panic escaped the selected callback boundary"
        );
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(PAYLOAD_DROPS.load(Ordering::SeqCst), 0);
        assert!(retired.load(Ordering::Acquire) && faulted.load(Ordering::Acquire));
        assert!(contain_rust_call(&retired, &faulted, || faulted.store(true, Ordering::Release), || unsafe { call() }).is_err());
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            1,
            "panicked callback was retried"
        );
    }
    fn retire_after_actual_retain(second: bool, change_isa: bool) {
        struct RetainLife {
            watched: usize,
            application: usize,
            changed_class: usize,
            owner: ThreadId,
            retired: AtomicBool,
            observed_application_retain: AtomicBool,
        }
        impl crate::sdk::owned::AliveLease for RetainLife {
            fn is_alive(&self) -> bool {
                !self.retired.load(Ordering::Acquire)
            }
            fn before_effect(&self) -> Result<()> {
                assert_eq!(self.owner, thread::current().id());
                let application_count: usize =
                    unsafe { objc2::msg_send![self.application as *mut AnyObject, retainCount] };
                if application_count > 1 {
                    self.observed_application_retain
                        .store(true, Ordering::Release);
                }
                // Actual NSObject ownership, not a mirrored mock retain count.
                let count: usize =
                    unsafe { objc2::msg_send![self.watched as *mut AnyObject, retainCount] };
                if count > 1 {
                    if self.changed_class != 0 {
                        unsafe {
                            objc2::ffi::object_setClass(
                                self.watched as *mut AnyObject,
                                self.changed_class as *const AnyClass,
                            )
                        };
                    }
                    self.retired.store(true, Ordering::Release);
                    return Err("retired after actual callback retain".into());
                }
                Ok(())
            }
        }
        let f = super::super::tests::Fixture::new();
        let target_class = ClassBuilder::new(
            if change_isa {
                c"RuycastF1OwnedRetainIsaTarget"
            } else if second {
                c"RuycastF1OwnedSecondRetainTarget"
            } else {
                c"RuycastF1OwnedFirstRetainTarget"
            },
            NSObject::class(),
        )
        .unwrap()
        .register();
        let application_class = ClassBuilder::new(
            if change_isa {
                c"RuycastF1OwnedRetainIsaApplication"
            } else if second {
                c"RuycastF1OwnedSecondRetainApplication"
            } else {
                c"RuycastF1OwnedFirstRetainApplication"
            },
            NSObject::class(),
        )
        .unwrap()
        .register();
        let target: Retained<NSObject> = unsafe { objc2::msg_send![target_class, new] };
        let application: Retained<NSObject> = unsafe { objc2::msg_send![application_class, new] };
        let target_pointer = Retained::as_ptr(&target).cast_mut().cast::<AnyObject>();
        let application_pointer = Retained::as_ptr(&application)
            .cast_mut()
            .cast::<AnyObject>();
        let target_weak = objc2::rc::Weak::from_retained(&target);
        let application_weak = objc2::rc::Weak::from_retained(&application);
        let changed_class = if change_isa {
            ClassBuilder::new(c"RuycastF1OwnedRetainChangedIsa", NSObject::class())
                .unwrap()
                .register() as *const AnyClass as usize
        } else {
            0
        };
        let life = Arc::new(RetainLife {
            watched: if second {
                application_pointer
            } else {
                target_pointer
            } as usize,
            application: application_pointer as usize,
            changed_class,
            owner: thread::current().id(),
            retired: AtomicBool::new(false),
            observed_application_retain: AtomicBool::new(false),
        });
        let mut context = f.model.context.clone();
        context.life = life.clone();
        let frame = Rect {
            origin: Point { x: 0., y: 0. },
            size: Point { x: 64., y: 64. },
        };
        let model = Workspace::new(
            context,
            vec![],
            vec![Screen {
                frame,
                visible: frame,
                scale: 1.,
            }],
            Point { x: 0., y: 0. },
        )
        .unwrap();
        let result =
            unsafe { ObjectiveC::selected(model, target_pointer, application_pointer, &[], None) };
        assert!(
            life.retired.load(Ordering::Acquire),
            "actual acquired +1 never reached a guardian postcheck"
        );
        assert!(
            result.is_err(),
            "callback published after retain retired its guardian"
        );
        assert_eq!(
            life.observed_application_retain.load(Ordering::Acquire),
            second,
            "first-reference rejection must precede the second actual retain"
        );
        drop(result);
        if change_isa {
            // Only these two empty, NSObject-only owned classes have identical
            // layout. Restore our own test object before reclaiming its known
            // construction reference and the deliberately quarantined +1.
            unsafe { objc2::ffi::object_setClass(target_pointer, target_class as *const AnyClass) };
        }
        drop(target);
        drop(application);
        if change_isa {
            assert!(
                target_weak.load().is_some(),
                "changed isa must quarantine the acquired +1"
            );
            drop(unsafe { Retained::from_raw(target_pointer) });
        }
        assert!(
            target_weak.load().is_none(),
            "fresh qualified NSObject target cleanup leaked"
        );
        assert!(
            application_weak.load().is_none(),
            "fresh qualified NSObject application cleanup leaked"
        );
    }
    #[test]
    fn first_actual_retain_reentry_cannot_publish_or_take_the_second_reference() {
        retire_after_actual_retain(false, false);
    }
    #[test]
    fn second_actual_retain_reentry_cannot_publish_and_balances_qualified_cleanup() {
        retire_after_actual_retain(true, false);
    }
    #[test]
    fn changed_isa_after_actual_retain_quarantines_only_the_unqualified_reference() {
        retire_after_actual_retain(false, true);
    }
    #[test]
    fn owned_callback_releases_both_plus_ones_on_its_owner() {
        let f = super::super::tests::Fixture::new();
        let class = ClassBuilder::new(c"RuycastF1OwnedCallbackCleanup", NSObject::class())
            .unwrap()
            .register();
        let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
        let weak = objc2::rc::Weak::from_retained(&object);
        let p = Retained::as_ptr(&object).cast_mut().cast::<AnyObject>();
        let callback = unsafe { ObjectiveC::owned_fixture(f.model.clone(), p, &[]) }.unwrap();
        assert_eq!(callback.class_identity(), class);
        drop(object);
        assert!(weak.load().is_some());
        drop(callback);
        assert!(
            weak.load().is_none(),
            "callback leaked target/application +1 retains"
        );
    }
    #[test]
    fn foreign_or_changed_cleanup_quarantines_but_proven_retired_cleanup_releases() {
        static UNQUALIFIED_RELEASE: AtomicU64 = AtomicU64::new(0);
        unsafe extern "C" fn changed_release(_: *mut AnyObject, _: Sel) {
            UNQUALIFIED_RELEASE.fetch_add(1, Ordering::SeqCst);
        }
        for (name, lane) in [
            (c"RuycastF1OwnedCleanupForeign", 0),
            (c"RuycastF1OwnedCleanupRetired", 1),
            (c"RuycastF1OwnedCleanupChanged", 2),
        ] {
            let f = super::super::tests::Fixture::new();
            let class = ClassBuilder::new(name, NSObject::class())
                .unwrap()
                .register();
            let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
            let weak = objc2::rc::Weak::from_retained(&object);
            let p = Retained::as_ptr(&object).cast_mut().cast::<AnyObject>();
            let callback = unsafe { ObjectiveC::owned_fixture(f.model.clone(), p, &[]) }.unwrap();
            let inherited_release = class
                .instance_method(objc2::sel!(release))
                .unwrap()
                .implementation();
            drop(object);
            if lane == 2 {
                // Add an own override; never mutate NSObject's inherited Method.
                unsafe extern "C" {
                    fn class_addMethod(
                        class: *const std::ffi::c_void,
                        sel: *const std::ffi::c_void,
                        imp: *const std::ffi::c_void,
                        types: *const std::ffi::c_char,
                    ) -> u8;
                }
                assert!(
                    unsafe {
                        class_addMethod(
                            (class as *const AnyClass).cast(),
                            std::mem::transmute::<Sel, *const std::ffi::c_void>(objc2::sel!(
                                release
                            )),
                            changed_release as *const () as *const std::ffi::c_void,
                            c"v@:".as_ptr(),
                        )
                    } != 0
                );
            }
            if lane == 0 {
                std::thread::spawn(move || drop(callback)).join().unwrap();
            } else {
                if lane == 1 {
                    f.life.0.store(false, Ordering::Release);
                }
                drop(callback);
            }
            if lane == 2 {
                unsafe {
                    class
                        .instance_method(objc2::sel!(release))
                        .unwrap()
                        .set_implementation(inherited_release);
                }
            }
            if lane == 1 {
                assert!(
                    weak.load().is_none(),
                    "business retirement must not leak qualified NSObject cleanup"
                );
                continue;
            }
            assert!(
                weak.load().is_some(),
                "unqualified finalizer must retain rather than execute"
            );
            assert_eq!(
                UNQUALIFIED_RELEASE.load(Ordering::SeqCst),
                0,
                "unqualified release must never execute"
            );
            // Reclaim only these known fixture +1s on the original owner after
            // restoring genuine NSObject cleanup; no production quarantine drain.
            drop(unsafe { Retained::from_raw(p) });
            drop(unsafe { Retained::from_raw(p) });
            assert!(weak.load().is_none());
        }
    }
    #[test]
    fn missing_original_cleanup_authority_fails_before_either_retain() {
        struct NoCleanup;
        unsafe impl CallbackCodeAuthority for NoCleanup {
            fn verify_sender(
                &self,
                _: &AnyClass,
                _: Sel,
                _: usize,
                _: &crate::sdk::SourceCallsite,
                _: &AnyClass,
            ) -> Result<()> {
                Ok(())
            }
            fn verify_method(
                &self,
                _: &AnyClass,
                _: Sel,
                _: usize,
                _: &crate::sdk::SourceCallsite,
            ) -> Result<()> {
                Ok(())
            }
        }
        let f = super::super::tests::Fixture::new();
        let class = ClassBuilder::new(c"RuycastF1OwnedCleanupAbsent", NSObject::class())
            .unwrap()
            .register();
        let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
        let weak = objc2::rc::Weak::from_retained(&object);
        let p = Retained::as_ptr(&object).cast_mut().cast::<AnyObject>();
        let source = source_roster().into_iter().next().unwrap();
        assert_eq!(
            NoCleanup.verify_cleanup(class, &source).unwrap_err(),
            "original callback cleanup authority absent"
        );
        assert!(unsafe {
            ObjectiveC::selected(
                f.model.clone(),
                p,
                p,
                &[],
                Some((Arc::new(NoCleanup), source)),
            )
        }
        .is_err());
        drop(object);
        assert!(
            weak.load().is_none(),
            "failed cleanup authority must not acquire +1s"
        );
    }
    #[test]
    fn owned_fixture_label_does_not_authorize_custom_cxx_cleanup() {
        unsafe extern "C" fn cxx(_: *mut AnyObject, _: Sel) {}
        let f = super::super::tests::Fixture::new();
        let mut builder =
            ClassBuilder::new(c"RuycastF1OwnedCleanupCXX", NSObject::class()).unwrap();
        unsafe {
            builder.add_method(
                Sel::register(c".cxx_destruct"),
                cxx as unsafe extern "C" fn(*mut AnyObject, Sel),
            );
        }
        let class = builder.register();
        let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
        let weak = objc2::rc::Weak::from_retained(&object);
        let p = Retained::as_ptr(&object).cast_mut().cast::<AnyObject>();
        assert!(unsafe { ObjectiveC::owned_fixture(f.model.clone(), p, &[]) }.is_err());
        drop(object);
        assert!(weak.load().is_none());
    }
    #[test]
    fn selected_real_own_delegate_and_control_imps_preserve_lifecycle_order_and_reject_tamper() {
        static ORDER: AtomicU64 = AtomicU64::new(0);
        unsafe extern "C" fn will(_: *mut AnyObject, _: Sel, n: *mut AnyObject) {
            assert_eq!(
                unsafe { &*n.cast::<NSNotification>() }.name().to_string(),
                "NSApplicationWillFinishLaunchingNotification"
            );
            assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 0);
        }
        unsafe extern "C" fn did(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
            assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 1);
        }
        unsafe extern "C" fn action(_: *mut AnyObject, _: Sel) {
            assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 2);
        }
        unsafe extern "C" fn shutdown(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
            assert_eq!(ORDER.fetch_add(1, Ordering::SeqCst), 3);
        }
        unsafe extern "C" fn changed(_: *mut AnyObject, _: Sel) {}
        let f = super::super::tests::Fixture::new();
        let mut b = ClassBuilder::new(c"RuycastF1OwnedLoopDelegate", NSObject::class()).unwrap();
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
                objc2::sel!(toggle),
                action as unsafe extern "C" fn(*mut AnyObject, Sel),
            );
            b.add_method(
                objc2::sel!(applicationWillTerminate:),
                shutdown as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        let class = b.register();
        let object: Retained<NSObject> = unsafe { objc2::msg_send![class, new] };
        let p = Retained::as_ptr(&object).cast_mut().cast::<AnyObject>();
        let selected = [
            (
                c"applicationWillFinishLaunching:",
                will as *const () as usize,
            ),
            (c"applicationDidFinishLaunching:", did as *const () as usize),
            (c"toggle", action as *const () as usize),
            (c"applicationWillTerminate:", shutdown as *const () as usize),
        ];
        let cb =
            Arc::new(unsafe { ObjectiveC::owned_fixture(f.model.clone(), p, &selected) }.unwrap());
        let engine = event_loop::Engine::new(f.model.clone()).unwrap();
        engine.set_delegate(cb.clone()).unwrap();
        let target = Handle {
            id: 501,
            generation: 1,
        };
        engine.register_target(target, cb.clone()).unwrap();
        engine
            .enqueue(event_loop::Event::Action {
                target,
                selector: "toggle".into(),
            })
            .unwrap();
        engine.enqueue(event_loop::Event::Terminate).unwrap();
        engine.launch().unwrap();
        engine.pump().unwrap();
        engine.pump().unwrap();
        assert_eq!(ORDER.load(Ordering::SeqCst), 4);
        assert!(cb.action("unknown").is_err());
        unsafe {
            class
                .instance_method(objc2::sel!(toggle))
                .unwrap()
                .set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                    changed as *const () as usize,
                ));
        }
        assert!(cb.action("toggle").is_err());
        assert_eq!(ORDER.load(Ordering::SeqCst), 4);
        let source = source_roster().into_iter().next().unwrap();
        struct RejectOriginal;
        unsafe impl CallbackCodeAuthority for RejectOriginal {
            fn verify_sender(
                &self,
                _: &AnyClass,
                _: Sel,
                _: usize,
                _: &crate::sdk::SourceCallsite,
                _: &AnyClass,
            ) -> Result<()> {
                Err("owned fixture has no original sender provenance".into())
            }
            fn verify_method(
                &self,
                _: &AnyClass,
                _: Sel,
                _: usize,
                _: &crate::sdk::SourceCallsite,
            ) -> Result<()> {
                Err("owned fixture has no original method provenance".into())
            }
        }
        assert!(unsafe {
            ObjectiveC::from_original(
                f.model.clone(),
                p,
                p,
                &selected,
                &source,
                Arc::new(RejectOriginal),
            )
        }
        .is_err());
    }
}
