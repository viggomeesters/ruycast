//! Fault injection for one explicitly attached named board. Never read the general board.
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyClass, AnyObject, ClassBuilder, Imp, Sel},
    sel, AnyThread, ClassType, Message,
};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeHTML, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSString};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    LazyLock, Mutex,
};

type Items = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
type StringForType =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSString) -> *mut NSString;
#[derive(Default)]
struct State {
    board: usize,
    items: usize,
    original_items: usize,
    original_string: usize,
    mode: i32,
}
static STATE: Mutex<State> = Mutex::new(State {
    board: 0,
    items: 0,
    original_items: 0,
    original_string: 0,
    mode: 0,
});
static GUARDED: AtomicBool = AtomicBool::new(false);

unsafe fn string(units: &[u16]) -> *mut NSString {
    // Construct genuine NSStrings without normalizing ill-formed UTF-16.
    let value: Retained<NSString> =
        msg_send![NSString::alloc(), initWithCharacters:units.as_ptr(), length:units.len()];
    Retained::autorelease_return(value)
}
unsafe fn text(value: &str) -> *mut NSString {
    Retained::autorelease_return(NSString::from_str(value))
}
unsafe extern "C-unwind" fn types(_: *mut AnyObject, _: Sel) -> *mut NSArray<NSString> {
    let mode = STATE.lock().unwrap().mode;
    let values = match mode {
        3 => vec![Retained::retain(string(&[0xd800])).unwrap()],
        4 => vec![NSPasteboardTypeString.retain()],
        6 => vec![
            NSPasteboardTypeString.retain(),
            NSPasteboardTypeHTML.retain(),
            NSString::from_str("own.extra"),
        ],
        _ => vec![
            NSPasteboardTypeHTML.retain(),
            NSPasteboardTypeString.retain(),
        ],
    };
    Retained::autorelease_return(NSArray::from_retained_slice(&values))
}
unsafe extern "C-unwind" fn item_string(
    _: *mut AnyObject,
    _: Sel,
    ty: *mut NSString,
) -> *mut NSString {
    assert!(!ty.is_null());
    let mode = STATE.lock().unwrap().mode;
    if (*ty).isEqualToString(NSPasteboardTypeString) {
        let units = match mode {
            1 => vec![0xd800],
            10 => vec![0xdc00],
            11 => vec![32, 0xd800, 0xd800, 0xdc00, 0xdfff, 0, 10],
            12 | 13 => {
                let mut v = vec![b'x' as u16; 4095];
                v.extend([0xd83d, if mode == 12 { 0xde00 } else { b'A' as u16 }]);
                v
            }
            14 => {
                let mut v = Vec::with_capacity(1_000_005);
                v.extend([32, 10]);
                v.extend(std::iter::repeat_n(b'x' as u16, 1_000_001));
                v.extend([10, 32]);
                v
            }
            15 => return text(" \n\u{200b}"),
            16 => return text(""),
            17 => return std::ptr::null_mut(),
            7 => vec![65, 0, 67, 0xd83d, 0xde00],
            8 | 9 => {
                let mut v = vec![b'x' as u16; 5000];
                v.extend([65, 0, if mode == 8 { 67 } else { 66 }, 0xd83d, 0xde00]);
                v
            }
            _ => vec![65, 0, 66, 0xd83d, 0xde00],
        };
        return string(&units);
    }
    if (*ty).isEqualToString(NSPasteboardTypeHTML) {
        return match mode {
            2 => string(&[0xdc00]),
            4 => std::ptr::null_mut(),
            5 => text("<i>foreign</i>"),
            _ => text("<b>A</b>"),
        };
    }
    std::ptr::null_mut()
}
static ITEM_CLASS: LazyLock<&'static AnyClass> = LazyLock::new(|| {
    let mut builder =
        ClassBuilder::new(c"RuycastOwnedFaultItem", NSPasteboardItem::class()).unwrap();
    unsafe {
        builder.add_method(sel!(types), types as unsafe extern "C-unwind" fn(_, _) -> _);
        builder.add_method(
            sel!(stringForType:),
            item_string as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
    }
    builder.register()
});
unsafe extern "C-unwind" fn fixture_items(board: *mut AnyObject, selector: Sel) -> *mut AnyObject {
    let (owned, items, original) = {
        let state = STATE.lock().unwrap();
        (state.board, state.items, state.original_items)
    };
    if board as usize == owned {
        return items as *mut AnyObject;
    }
    assert_ne!(original, 0);
    std::mem::transmute::<usize, Items>(original)(board, selector)
}
unsafe extern "C-unwind" fn fixture_string(
    board: *mut AnyObject,
    selector: Sel,
    ty: *mut NSString,
) -> *mut NSString {
    let (owned, items, original) = {
        let state = STATE.lock().unwrap();
        (state.board, state.items, state.original_string)
    };
    if board as usize == owned {
        let item: *mut AnyObject = msg_send![items as *mut AnyObject, objectAtIndex:0usize];
        return msg_send![item, stringForType:ty];
    }
    assert_ne!(original, 0);
    std::mem::transmute::<usize, StringForType>(original)(board, selector, ty)
}
unsafe extern "C-unwind" fn forbidden_general(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    libc::abort()
}
#[no_mangle]
pub extern "C" fn ruycast_owned_board_general_guard() -> bool {
    GUARDED.load(Ordering::Acquire)
}
#[no_mangle]
/// # Safety
/// The caller owns a live, unique named board on the main thread and keeps it
/// alive until detach. No other attachment may be active.
pub unsafe extern "C" fn ruycast_owned_board_attach(board: *mut NSPasteboard, mode: i32) -> bool {
    assert!(libc::pthread_main_np() != 0 && !board.is_null());
    let name: Retained<NSString> = msg_send![board, name];
    assert!(!name.isEqualToString(objc2_app_kit::NSPasteboardNameGeneral));
    assert_eq!(STATE.lock().unwrap().board, 0);
    let item: Retained<AnyObject> = msg_send![*ITEM_CLASS, new];
    let items = NSArray::from_retained_slice(&[item]);
    let class = (*board).class();
    let Some(items_method) = class.instance_method(sel!(pasteboardItems)) else {
        return false;
    };
    let string_method = class
        .instance_method(sel!(stringForType:))
        .expect("Original board string method");
    let original_items = items_method.implementation();
    let original_string = string_method.implementation();
    *STATE.lock().unwrap() = State {
        board: board as usize,
        items: Retained::into_raw(items) as usize,
        original_items: original_items as usize,
        original_string: original_string as usize,
        mode,
    };
    items_method.set_implementation(std::mem::transmute::<Items, Imp>(fixture_items));
    string_method.set_implementation(std::mem::transmute::<StringForType, Imp>(fixture_string));
    true
}
#[no_mangle]
/// # Safety
/// Call on the main thread before releasing an attached board, after its reads
/// have finished. This restores its original methods before releasing items.
pub unsafe extern "C" fn ruycast_owned_board_detach() {
    assert!(libc::pthread_main_np() != 0);
    let (board, items, original_items, original_string) = {
        let state = STATE.lock().unwrap();
        (
            state.board,
            state.items,
            state.original_items,
            state.original_string,
        )
    };
    if board == 0 {
        return;
    }
    let class = (*(board as *mut NSPasteboard)).class();
    class
        .instance_method(sel!(pasteboardItems))
        .unwrap()
        .set_implementation(std::mem::transmute::<usize, Imp>(original_items));
    class
        .instance_method(sel!(stringForType:))
        .unwrap()
        .set_implementation(std::mem::transmute::<usize, Imp>(original_string));
    *STATE.lock().unwrap() = State::default();
    drop(Retained::from_raw(items as *mut NSArray<AnyObject>));
}
unsafe extern "C" fn install() {
    autoreleasepool(|_| {
        // Preserve the original class message and lazy initialization before hooking.
        let class = NSPasteboard::class();
        let _: *const AnyClass = msg_send![class, class];
        let general = class.class_method(sel!(generalPasteboard)).unwrap();
        general.set_implementation(std::mem::transmute::<Items, Imp>(forbidden_general));
        GUARDED.store(true, Ordering::Release);
    });
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
