//! Own NSObject-only equivalent of the frozen native Panel allocation pattern.
//! This executes real objc2 partial-init/ivar/drop machinery, not AppKit.
use super::*;
use objc2::{
    define_class, extern_class, msg_send,
    rc::{Allocated, Retained},
    runtime::ClassBuilder,
    AnyThread, DefinedClass,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    OnceLock,
};
extern_class!(
    #[unsafe(super(NSObject))]
    #[name = "RuycastF1OwnedReceiverParent"]
    struct OwnedParent;
);
unsafe impl NSObjectProtocol for OwnedParent {}
struct Fields {
    value: bool,
    receiver: usize,
}
impl Drop for Fields {
    fn drop(&mut self) {
        let c = CTX.get().unwrap();
        c.trace.lock().unwrap().push("rust-ivars-drop");
        c.drops.fetch_add(1, Ordering::SeqCst);
        c.grant.retire(self.receiver as *mut AnyObject).unwrap();
    }
}
define_class!(
 #[unsafe(super(OwnedParent))]#[thread_kind=AnyThread]#[ivars=Fields]
 struct RuycastF1OwnedReceiverNative;
 unsafe impl NSObjectProtocol for RuycastF1OwnedReceiverNative {}
 impl RuycastF1OwnedReceiverNative {
  #[unsafe(method(ownedIvarsReady))]fn ready(&self)->bool{self.ivars().value}
 }
);
struct Context {
    grant: Arc<WorkspaceGrant>,
    trace: Mutex<Vec<&'static str>>,
    fail: AtomicBool,
    drops: AtomicUsize,
    parent_drops: AtomicUsize,
}
static CTX: OnceLock<Context> = OnceLock::new();
#[test]
fn lifecycle_binding_cannot_borrow_another_class_or_original_field() {
    unsafe extern "C" fn initialize(_: *mut AnyClass, _: Sel) {}
    unsafe extern "C" fn before(_: *mut AnyObject, _: Sel) -> usize {
        1
    }
    unsafe extern "C" fn after(_: *mut AnyObject, _: Sel) -> usize {
        2
    }
    unsafe extern "C" {
        fn class_replaceMethod(
            class: *const AnyClass,
            selector: Sel,
            implementation: objc2::runtime::Imp,
            encoding: *const libc::c_char,
        ) -> Option<objc2::runtime::Imp>;
    }
    let f = super::super::tests::Fixture::new();
    let classes = [
        c"RuycastF1OwnedBindingFirst",
        c"RuycastF1OwnedBindingSecond",
    ]
    .map(|name| {
        let mut builder = ClassBuilder::new(name, NSObject::class()).unwrap();
        unsafe {
            builder.add_class_method(
                objc2::sel!(initialize),
                initialize as unsafe extern "C" fn(*mut AnyClass, Sel),
            );
            builder.add_method(
                objc2::sel!(ownedValue),
                before as unsafe extern "C" fn(*mut AnyObject, Sel) -> usize,
            );
        }
        builder.register()
    });
    let guard =
        unsafe { super::super::classes::ClassGuard::select(&classes, f.model.lease()) }.unwrap();
    let constructor = guard
        .initializers
        .iter()
        .find(|p| p.class == classes[0].name())
        .unwrap()
        .clone();
    let valid = LifecycleBinding::constructor(constructor);
    valid.verify().unwrap();
    let mut wrong_class = valid.clone();
    wrong_class.class = classes[1].name();
    assert!(wrong_class.verify().is_err());
    let mut wrong_original = valid;
    wrong_original.original ^= 1;
    assert!(wrong_original.verify().is_err());

    let handle = metadata::registered_class(classes[0] as *const AnyClass as usize).unwrap();
    let baseline = metadata::method(handle, c"ownedValue", false)
        .unwrap()
        .unwrap();
    let lifecycle = metadata::lifecycle(handle).unwrap();
    let encoding = std::ffi::CString::new(baseline.encoding()).unwrap();
    for class in classes {
        unsafe {
            class_replaceMethod(
                class,
                objc2::sel!(ownedValue),
                std::mem::transmute::<usize, objc2::runtime::Imp>(after as *const () as usize),
                encoding.as_ptr(),
            );
        }
    }
    let selected = metadata::method(handle, c"ownedValue", false)
        .unwrap()
        .unwrap();
    let leaf = ObjcReplacement {
        class: classes[0].name(),
        selector: c"ownedValue",
        class_method: false,
        original: baseline.implementation(),
        replacement: selected.implementation(),
        provenance: metadata::ReplacementProvenance::bind(
            baseline.clone(),
            baseline,
            selected,
            lifecycle,
            false,
            None,
        )
        .unwrap(),
    };
    let valid = LifecycleBinding::leaf(leaf.clone());
    valid.verify().unwrap();
    let mut wrong_class = valid.clone();
    wrong_class.class = classes[1].name();
    assert!(wrong_class.verify().is_err());
    let mut wrong_original = valid;
    wrong_original.original ^= 1;
    assert!(wrong_original.verify().is_err());
    let mut inner_key = leaf;
    inner_key.class = classes[1].name();
    assert!(LifecycleBinding::leaf(inner_key).verify().is_err());
}
unsafe extern "C" fn original_initialize(_: *mut AnyClass, _: Sel) {
    panic!("original own ancestor initializer must have been replaced before messages")
}
unsafe extern "C" fn init(
    this: *mut AnyObject,
    _: Sel,
    frame: objc2_foundation::NSRect,
    style: usize,
    backing: usize,
    defer: Bool,
) -> *mut AnyObject {
    assert_eq!(frame.size.width, 640.0);
    assert_eq!(style, 0);
    assert_eq!(backing, 2);
    assert!(!defer.as_bool());
    let context = CTX.get().unwrap();
    let token = unsafe { context.grant.before_super_init(this) }.unwrap();
    assert_eq!(token.observed_scalars(), &[true]);
    context
        .trace
        .lock()
        .unwrap()
        .push("actual-super-init-observed-ivars");
    if context.fail.load(Ordering::SeqCst) {
        // Objective-C init failure releases self, then returns nil. Generated Rust
        // dealloc must drop initialized ivars once even though super-init failed.
        drop(unsafe { Retained::from_raw(this) }.unwrap());
        std::ptr::null_mut()
    } else {
        context.grant.initialized(token).unwrap();
        this
    }
}
unsafe extern "C" fn parent_dealloc(this: *mut AnyObject, _: Sel) {
    let context = CTX.get().unwrap();
    context
        .trace
        .lock()
        .unwrap()
        .push("selected-parent-dealloc");
    context.parent_drops.fetch_add(1, Ordering::SeqCst);
    let method = NSObject::class()
        .instance_method(objc2::sel!(dealloc))
        .unwrap();
    let call: unsafe extern "C" fn(*mut AnyObject, Sel) =
        unsafe { std::mem::transmute(method.implementation()) };
    unsafe { call(this, objc2::sel!(dealloc)) };
}
fn construct() -> Option<Retained<RuycastF1OwnedReceiverNative>> {
    let allocated = RuycastF1OwnedReceiverNative::alloc();
    let p = Allocated::as_ptr(&allocated).cast_mut().cast::<AnyObject>();
    unsafe { CTX.get().unwrap().grant.track_allocated(p) }.unwrap();
    CTX.get().unwrap().trace.lock().unwrap().push("allocated");
    let partial = allocated.set_ivars(Fields {
        value: true,
        receiver: p as usize,
    });
    CTX.get().unwrap().trace.lock().unwrap().push("set-ivars");
    unsafe {
        msg_send![super(partial),initWithContentRect:NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(640.0,480.0)),styleMask:0usize,backing:2usize,defer:false]
    }
}
#[test]
fn actual_native_pattern_preserves_receiver_ivars_and_arc_success_failure_drop_order() {
    let f = super::super::tests::Fixture::new();
    let mut b = ClassBuilder::new(c"RuycastF1OwnedReceiverParent", NSObject::class()).unwrap();
    unsafe {
        b.add_class_method(
            objc2::sel!(initialize),
            original_initialize as unsafe extern "C" fn(*mut AnyClass, Sel),
        );
        b.add_method(
            objc2::sel!(initWithContentRect:styleMask:backing:defer:),
            init as unsafe extern "C" fn(
                *mut AnyObject,
                Sel,
                NSRect,
                usize,
                usize,
                Bool,
            ) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(dealloc),
            parent_dealloc as unsafe extern "C" fn(*mut AnyObject, Sel),
        );
    }
    let parent = b.register();
    // Signature checks during define_class registration may initialize the
    // superclass: select it BEFORE requesting the source-owned child class.
    let guard =
        unsafe { super::super::classes::ClassGuard::select(&[parent], f.model.lease()) }.unwrap();
    let class = RuycastF1OwnedReceiverNative::class();
    assert_eq!(class.superclass(), Some(parent));
    let mut selected: Vec<_> = guard
        .initializers
        .iter()
        .cloned()
        .map(LifecycleBinding::constructor)
        .collect();
    for current in [class, parent] {
        for method in current.instance_methods().iter() {
            if [
                objc2::sel!(dealloc),
                Sel::register(c".cxx_construct"),
                Sel::register(c".cxx_destruct"),
                objc2::sel!(initWithContentRect:styleMask:backing:defer:),
            ]
            .contains(&method.name())
            {
                let selector = unsafe { CStr::from_ptr(method.name().name().as_ptr()) };
                selected.push(
                    LifecycleBinding::fixture(
                        unsafe { CStr::from_ptr(current.name().as_ptr()) },
                        selector,
                        false,
                    )
                    .unwrap(),
                );
            }
        }
    }
    let probe = class
        .instance_method(objc2::sel!(ownedIvarsReady))
        .unwrap()
        .implementation() as usize;
    let grant = Arc::new(WorkspaceGrant {
        owner: "owned-NSobject-receiver-canary",
        source: SourceCallsite {
            file: "bench/f1-runner/src/sdk/workspace_receiver_canary.rs",
            source_sha256: "bound by own final source/artifact manifest; no original source claim"
                .into(),
            operation:
                "alloc -> set_ivars -> actual super-init -> generated ivar drop -> parent dealloc",
        },
        class,
        selected,
        probes: vec![ScalarProbe {
            selector: c"ownedIvarsReady",
            expected_imp: probe,
        }],
        model: f.model.clone(),
        receivers: Mutex::new(BTreeMap::new()),
        original_source: false,
        id: serial().unwrap(),
        authority: None,
        root_deallocator: None,
        construction: OnceLock::new(),
    });
    grant.verify().unwrap();
    assert!(CTX
        .set(Context {
            grant,
            trace: Mutex::new(vec![]),
            fail: AtomicBool::new(false),
            drops: AtomicUsize::new(0),
            parent_drops: AtomicUsize::new(0)
        })
        .is_ok());
    let first = construct().unwrap();
    let raw = Retained::as_ptr(&first);
    assert!(unsafe { msg_send![&*first, ownedIvarsReady] });
    drop(first);
    let context = CTX.get().unwrap();
    assert_eq!(context.drops.load(Ordering::SeqCst), 1);
    assert_eq!(context.parent_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        *context.trace.lock().unwrap(),
        vec![
            "allocated",
            "set-ivars",
            "actual-super-init-observed-ivars",
            "rust-ivars-drop",
            "selected-parent-dealloc"
        ]
    );
    context.trace.lock().unwrap().clear();
    context.fail.store(true, Ordering::SeqCst);
    assert!(construct().is_none());
    assert_eq!(context.drops.load(Ordering::SeqCst), 2);
    assert_eq!(context.parent_drops.load(Ordering::SeqCst), 2);
    assert_eq!(
        *context.trace.lock().unwrap(),
        vec![
            "allocated",
            "set-ivars",
            "actual-super-init-observed-ivars",
            "rust-ivars-drop",
            "selected-parent-dealloc"
        ]
    );
    let _ = raw; // Never dereference a pointer after selected parent deallocation.
}
