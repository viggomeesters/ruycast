use super::{
    abi,
    model::{self, Attribute as A, Context, Faults, NodeSpec},
    opaque::{self, Payload as P, Ptr, Range, Token},
};
use crate::sdk::owned::AliveLease;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Life(AtomicBool);
impl AliveLease for Life {
    fn is_alive(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn before_effect(&self) -> std::result::Result<(), String> {
        if self.is_alive() {
            Ok(())
        } else {
            Err("owned unit lease retired".into())
        }
    }
}
fn nodes() -> Vec<NodeSpec> {
    let make = |id, pid, values: Vec<(&str, A)>, writes: &[&str], actions: &[&str]| NodeSpec {
        id,
        pid,
        attributes: values.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        writable: writes.iter().map(|s| (*s).into()).collect(),
        actions: actions.iter().map(|s| (*s).into()).collect(),
    };
    vec![
        make(1, 0, vec![("AXFocusedUIElement", A::Element(4))], &[], &[]),
        make(
            2,
            700001,
            vec![
                ("AXFocusedWindow", A::Element(3)),
                ("AXFocusedUIElement", A::Element(4)),
                ("AXWindows", A::Elements(vec![3])),
                ("AXMenuBar", A::Element(5)),
            ],
            &[],
            &[],
        ),
        make(
            3,
            700001,
            vec![
                ("AXTitle", A::Text("own e\u{301}\0🦀 window".into())),
                ("AXPosition", A::Value(P::Point([100.0, 50.0]))),
                ("AXSize", A::Value(P::Size([600.0, 400.0]))),
                ("AXMinimized", A::Boolean(false)),
                ("AXWindowNumber", A::Integer(77)),
            ],
            &["AXPosition", "AXSize", "AXMinimized"],
            &["AXRaise"],
        ),
        make(
            4,
            700001,
            vec![
                ("AXValue", A::Text("own field".into())),
                (
                    "AXSelectedTextRange",
                    A::Value(P::Range(Range {
                        location: 3,
                        length: 0,
                    })),
                ),
            ],
            &[],
            &[],
        ),
        make(
            5,
            700001,
            vec![("AXChildren", A::Elements(vec![6]))],
            &[],
            &[],
        ),
        make(
            6,
            700001,
            vec![
                ("AXTitle", A::Text("File".into())),
                ("AXChildren", A::Elements(vec![7])),
            ],
            &[],
            &[],
        ),
        make(
            7,
            700001,
            vec![
                ("AXTitle", A::Text("New".into())),
                ("AXEnabled", A::Boolean(true)),
            ],
            &[],
            &["AXPress"],
        ),
    ]
}
fn fixture() -> (Arc<Life>, Arc<Context>) {
    let life = Arc::new(Life(AtomicBool::new(true)));
    let c = Context::new(life.clone(), 1, BTreeMap::from([(700001, 2)]), nodes()).unwrap();
    (life, c)
}
struct Returned(Ptr);
impl Drop for Returned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                opaque::CFRelease(self.0);
            }
        }
    }
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(p: Ptr) -> isize;
    fn CFArrayGetValueAtIndex(p: Ptr, index: isize) -> Ptr;
}
#[test]
fn constructor_refuses_unknown_references_duplicates_and_dead_lease() {
    let life = Arc::new(Life(AtomicBool::new(true)));
    let mut data = nodes();
    data[0]
        .attributes
        .insert("AXChildren".into(), A::Elements(vec![999]));
    assert!(Context::new(life.clone(), 1, BTreeMap::from([(700001, 2)]), data).is_err());
    let mut data = nodes();
    data.push(data[0].clone());
    assert!(Context::new(life.clone(), 1, BTreeMap::from([(700001, 2)]), data).is_err());
    assert!(Context::new(life.clone(), 1, BTreeMap::from([(700002, 2)]), nodes()).is_err());
    life.0.store(false, Ordering::Release);
    assert!(Context::new(life, 1, BTreeMap::from([(700001, 2)]), nodes()).is_err());
}
#[test]
fn owned_generations_and_partial_resize_errors_preserve_actual_readback() {
    let (life, c) = fixture();
    let app = c.application(700001).unwrap();
    let window = c.attribute(app.ptr(), "AXFocusedWindow").unwrap();
    c.faults(
        3,
        Faults {
            writes: [(-25204, true)].into(),
            maximum_size: Some([650.0, 450.0]),
            ..Faults::default()
        },
    )
    .unwrap();
    assert_eq!(
        c.write(window.ptr(), "AXSize", A::Value(P::Size([800.0, 600.0]))),
        Err(-25204)
    );
    let size = c.attribute(window.ptr(), "AXSize").unwrap();
    assert_eq!(
        opaque::payload(size.ptr()).unwrap(),
        P::Size([650.0, 450.0])
    );
    c.invalidate(3).unwrap();
    assert_eq!(c.pid(window.ptr()), Err(model::INVALID));
    assert!(c.attribute(app.ptr(), "AXFocusedWindow").is_err());
    life.0.store(false, Ordering::Release);
    assert!(!c.trusted());
    life.0.store(true, Ordering::Release);
    assert!(!c.trusted());
}
#[test]
fn concrete_c_abi_covers_windows_menus_ranges_failures_and_retirement() {
    let all = abi::replacements();
    assert_eq!(all.iter().map(|x| x.0).collect::<BTreeSet<_>>().len(), 15);
    macro_rules! function {
        ($name:literal,$t:ty) => {{
            let p = all
                .iter()
                .find(|(n, _)| n.to_bytes() == $name.as_bytes())
                .unwrap()
                .1;
            unsafe { std::mem::transmute::<usize, $t>(p) }
        }};
    }
    let trusted = function!("AXIsProcessTrusted", unsafe extern "C" fn() -> u8);
    let application = function!(
        "AXUIElementCreateApplication",
        unsafe extern "C" fn(i32) -> Ptr
    );
    let system = function!("AXUIElementCreateSystemWide", unsafe extern "C" fn() -> Ptr);
    let element_type = function!("AXUIElementGetTypeID", unsafe extern "C" fn() -> usize);
    let value_type = function!("AXValueGetTypeID", unsafe extern "C" fn() -> usize);
    let pid = function!(
        "AXUIElementGetPid",
        unsafe extern "C" fn(Ptr, *mut i32) -> i32
    );
    let timeout = function!(
        "AXUIElementSetMessagingTimeout",
        unsafe extern "C" fn(Ptr, f32) -> i32
    );
    let attribute = function!(
        "AXUIElementCopyAttributeValue",
        unsafe extern "C" fn(Ptr, Ptr, *mut Ptr) -> i32
    );
    let count = function!(
        "AXUIElementGetAttributeValueCount",
        unsafe extern "C" fn(Ptr, Ptr, *mut isize) -> i32
    );
    let elements = function!(
        "AXUIElementCopyAttributeValues",
        unsafe extern "C" fn(Ptr, Ptr, isize, isize, *mut Ptr) -> i32
    );
    let write = function!(
        "AXUIElementSetAttributeValue",
        unsafe extern "C" fn(Ptr, Ptr, Ptr) -> i32
    );
    let action = function!(
        "AXUIElementPerformAction",
        unsafe extern "C" fn(Ptr, Ptr) -> i32
    );
    let value_create = function!(
        "AXValueCreate",
        unsafe extern "C" fn(u32, *const std::ffi::c_void) -> Ptr
    );
    let value_kind = function!("AXValueGetType", unsafe extern "C" fn(Ptr) -> u32);
    let value_get = function!("AXValueGetValue", unsafe extern "C" fn(Ptr, u32, Ptr) -> u8);
    assert_eq!(unsafe { trusted() }, 0);
    assert!(unsafe { application(700001) }.is_null());
    let (life, c) = fixture();
    abi::install(c.clone()).unwrap();
    assert_eq!(unsafe { trusted() }, 1);
    assert_eq!(
        abi::replace(&c, c.clone()).unwrap_err(),
        "same AX fixture owner"
    );
    assert_eq!(unsafe { trusted() }, 1);
    let app = Returned(unsafe { application(700001) });
    let sys = Returned(unsafe { system() });
    assert_eq!(unsafe { opaque::CFGetTypeID(app.0) }, unsafe {
        element_type()
    });
    assert_eq!(unsafe { opaque::CFGetTypeID(sys.0) }, unsafe {
        element_type()
    });
    assert!(unsafe { application(700002) }.is_null());
    let mut process = -1;
    assert_eq!(unsafe { pid(app.0, &mut process) }, 0);
    assert_eq!(process, 700001);
    assert_eq!(
        unsafe { pid(std::ptr::dangling_mut(), &mut process) },
        model::INVALID
    );
    assert_eq!(process, 0);
    assert_eq!(unsafe { timeout(app.0, 0.12) }, 0);
    assert_eq!(unsafe { timeout(app.0, f32::NAN) }, model::ILLEGAL);
    let copy = |p: Ptr, name: &str| {
        let name = Token::string(name).unwrap();
        let mut out = std::ptr::null_mut();
        assert_eq!(unsafe { attribute(p, name.ptr(), &mut out) }, 0);
        assert!(!out.is_null());
        Returned(out)
    };
    let window = copy(app.0, "AXFocusedWindow");
    let title = copy(window.0, "AXTitle");
    assert_eq!(
        unsafe { opaque::string(title.0) }.unwrap(),
        "own e\u{301}\0🦀 window"
    );
    let range = copy(copy(app.0, "AXFocusedUIElement").0, "AXSelectedTextRange");
    let mut got = Range {
        location: -1,
        length: -1,
    };
    assert_eq!(unsafe { opaque::CFGetTypeID(range.0) }, unsafe {
        value_type()
    });
    assert_eq!(unsafe { value_kind(range.0) }, 4);
    assert_eq!(
        unsafe { value_get(range.0, 4, (&mut got as *mut Range).cast()) },
        1
    );
    assert_eq!(
        got,
        Range {
            location: 3,
            length: 0
        }
    );
    let windows = Token::string("AXWindows").unwrap();
    let mut n = -1;
    assert_eq!(unsafe { count(app.0, windows.ptr(), &mut n) }, 0);
    assert_eq!(n, 1);
    let mut out = std::ptr::null_mut();
    assert_eq!(unsafe { elements(app.0, windows.ptr(), 0, 1, &mut out) }, 0);
    let list = Returned(out);
    assert_eq!(unsafe { CFArrayGetCount(list.0) }, 1);
    assert_eq!(unsafe { CFArrayGetValueAtIndex(list.0, 0) }, window.0);
    out = std::ptr::dangling_mut();
    assert_eq!(
        unsafe { elements(app.0, windows.ptr(), -1, 1, &mut out) },
        model::ILLEGAL
    );
    assert!(out.is_null());
    let menu = copy(app.0, "AXMenuBar");
    let children = copy(menu.0, "AXChildren");
    let file = unsafe { CFArrayGetValueAtIndex(children.0, 0) };
    let children = copy(file, "AXChildren");
    let item = unsafe { CFArrayGetValueAtIndex(children.0, 0) };
    let press = Token::string("AXPress").unwrap();
    assert_eq!(unsafe { action(item, press.ptr()) }, 0);
    assert_eq!(c.action_count(7, "AXPress").unwrap(), 1);
    let unsupported = Token::string("AXUnknown").unwrap();
    assert_eq!(
        unsafe { action(item, unsupported.ptr()) },
        model::ACTION_UNSUPPORTED
    );
    out = std::ptr::dangling_mut();
    assert_eq!(
        unsafe { attribute(item, unsupported.ptr(), &mut out) },
        model::UNSUPPORTED
    );
    assert!(out.is_null());
    let size = [800.0_f64, 600.0];
    let value = Returned(unsafe { value_create(2, size.as_ptr().cast()) });
    let name = Token::string("AXSize").unwrap();
    c.faults(
        3,
        Faults {
            writes: [(-25204, true)].into(),
            maximum_size: Some([650.0, 450.0]),
            ..Faults::default()
        },
    )
    .unwrap();
    assert_eq!(unsafe { write(window.0, name.ptr(), value.0) }, -25204);
    let readback = copy(window.0, "AXSize");
    let mut actual = [0.0_f64; 2];
    assert_eq!(
        unsafe { value_get(readback.0, 2, actual.as_mut_ptr().cast()) },
        1
    );
    assert_eq!(actual, [650.0, 450.0]);
    assert_eq!(
        unsafe { value_get(readback.0, 1, actual.as_mut_ptr().cast()) },
        0
    );
    c.permission(false).unwrap();
    assert_eq!(unsafe { trusted() }, 0);
    out = std::ptr::dangling_mut();
    assert_eq!(
        unsafe { attribute(window.0, name.ptr(), &mut out) },
        model::DISABLED
    );
    assert!(out.is_null());
    c.permission(true).unwrap();
    let (next_life, next) = fixture();
    abi::replace(&c, next.clone()).unwrap();
    assert_eq!(unsafe { pid(window.0, &mut process) }, model::INVALID);
    assert!(abi::replace(&c, next.clone()).is_err());
    life.0.store(false, Ordering::Release);
    assert!(!c.trusted());
    next_life.0.store(false, Ordering::Release);
    assert_eq!(unsafe { trusted() }, 0);
    next_life.0.store(true, Ordering::Release);
    assert_eq!(unsafe { trusted() }, 0);
    assert!(unsafe { application(700001) }.is_null());
    assert!(abi::retire(&c).is_err());
    abi::retire(&next).unwrap();
    assert_eq!(unsafe { trusted() }, 0);
}
