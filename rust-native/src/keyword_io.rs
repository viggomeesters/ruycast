//! Main-only keyword/AX boundary. Fixture mode replaces the complete table,
//! including app discovery and constants; no fallback to a real API.
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use objc2_foundation::NSObjectProtocol;
use std::ffi::c_void;
pub(crate) type Ptr = *mut c_void;
pub(crate) type TapCallback = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> Ptr;
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    pub location: isize,
    pub length: isize,
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {}
#[link(name = "Carbon", kind = "framework")]
extern "C" {}
#[cfg(feature = "test-tools")]
macro_rules! fixture_helper {
    ($name:literal,$ty:ty) => {{
        let p = libc::dlsym(
            libc::RTLD_DEFAULT,
            concat!("ruycast_keyword_fixture_", $name, "\0")
                .as_ptr()
                .cast(),
        );
        if p.is_null() {
            return Err("Complete keyword SDK fixture unavailable.".into());
        }
        std::mem::transmute::<Ptr, $ty>(p)
    }};
}
macro_rules! table {
    ($($field:ident => $native:ident ($($arg:ident:$ty:ty),*) -> $ret:ty;)*) => {
        extern "C" { $(fn $native($($arg:$ty),*)->$ret;)* }
        #[derive(Clone,Copy)]
        pub(crate) struct Api { $(pub $field:unsafe extern "C" fn($($ty),*)->$ret,)*
            pub front:unsafe extern "C" fn()->Ptr,
            pub app_release:unsafe extern "C" fn(Ptr),
            pub app_pid:unsafe extern "C" fn(Ptr)->i32,
            pub app_bundle:unsafe extern "C" fn(Ptr)->Ptr,
            pub app_equal:unsafe extern "C" fn(Ptr,Ptr)->bool,
            pub app_matches_notification:unsafe extern "C" fn(Ptr,Ptr)->bool,
            pub app_terminated:unsafe extern "C" fn(Ptr)->bool,
            pub app_activate:unsafe extern "C" fn(Ptr)->bool,
            pub attr:unsafe extern "C" fn(u8)->Ptr,
            pub common:unsafe extern "C" fn()->Ptr,
            pub raycast:unsafe extern "C" fn()->bool,
            pub clipboard:unsafe extern "C" fn()->Ptr,
            pub item_create:unsafe extern "C" fn(*const u8,usize,*const u8,usize,bool)->Ptr,
            pub item_release:unsafe extern "C" fn(Ptr),
            pub board_clear:unsafe extern "C" fn()->isize,
            pub board_write:unsafe extern "C" fn(Ptr)->bool,
            pub board_change:unsafe extern "C" fn()->isize,
            pub board_matches:unsafe extern "C" fn(Ptr)->bool,
            pub board_ours:unsafe extern "C" fn(Ptr)->bool,
        }
        impl Api {
            pub fn load(_main:MainThreadMarker,fixture:bool)->Result<Self,String> {
                if !fixture { return Ok(Self { $($field:$native,)*front,app_release,app_pid,app_bundle,app_equal,app_matches_notification,app_terminated,app_activate,attr,common,raycast,clipboard,item_create,item_release,board_clear,board_write,board_change,board_matches,board_ours }); }
                #[cfg(feature="test-tools")]
                unsafe {
                    let ready=libc::dlsym(libc::RTLD_DEFAULT,c"ruycast_keyword_fixture_ready".as_ptr());
                    if ready.is_null() || std::mem::transmute::<Ptr,unsafe extern "C" fn()->u64>(ready)()!=0x52554b5744303031 { return Err("Complete keyword SDK fixture unavailable.".into()); }
                    $(let $field=libc::dlsym(libc::RTLD_DEFAULT,concat!("ruycast_keyword_fixture_",stringify!($native),"\0").as_ptr().cast());
                      if $field.is_null(){return Err("Complete keyword SDK fixture unavailable.".into());})*
                    Ok(Self { $($field:std::mem::transmute::<Ptr,unsafe extern "C" fn($($ty),*)->$ret>($field),)*
                        front:fixture_helper!("front",unsafe extern "C" fn()->Ptr),
                        app_release:fixture_helper!("app_release",unsafe extern "C" fn(Ptr)),
                        app_pid:fixture_helper!("app_pid",unsafe extern "C" fn(Ptr)->i32),
                        app_bundle:fixture_helper!("app_bundle",unsafe extern "C" fn(Ptr)->Ptr),
                        app_equal:fixture_helper!("app_equal",unsafe extern "C" fn(Ptr,Ptr)->bool),
                        app_matches_notification:fixture_helper!("app_matches_notification",unsafe extern "C" fn(Ptr,Ptr)->bool),
                        app_terminated:fixture_helper!("app_terminated",unsafe extern "C" fn(Ptr)->bool),
                        app_activate:fixture_helper!("app_activate",unsafe extern "C" fn(Ptr)->bool),
                        attr:fixture_helper!("attr",unsafe extern "C" fn(u8)->Ptr),
                        common:fixture_helper!("common",unsafe extern "C" fn()->Ptr),
                        raycast:fixture_helper!("raycast",unsafe extern "C" fn()->bool),
                        clipboard:fixture_helper!("clipboard",unsafe extern "C" fn()->Ptr),
                        item_create:fixture_helper!("item_create",unsafe extern "C" fn(*const u8,usize,*const u8,usize,bool)->Ptr),
                        item_release:fixture_helper!("item_release",unsafe extern "C" fn(Ptr)),
                        board_clear:fixture_helper!("board_clear",unsafe extern "C" fn()->isize),
                        board_write:fixture_helper!("board_write",unsafe extern "C" fn(Ptr)->bool),
                        board_change:fixture_helper!("board_change",unsafe extern "C" fn()->isize),
                        board_matches:fixture_helper!("board_matches",unsafe extern "C" fn(Ptr)->bool),
                        board_ours:fixture_helper!("board_ours",unsafe extern "C" fn(Ptr)->bool),
                    })
                }
                #[cfg(not(feature="test-tools"))]
                Err("Keyword fixtures require test-tools.".into())
            }
        }
    }
}
table! {
    tap_create=>CGEventTapCreate(location:u32,place:u32,options:u32,mask:u64,callback:TapCallback,context:Ptr)->Ptr;
    tap_enable=>CGEventTapEnable(tap:Ptr,enable:bool)->();
    tap_enabled=>CGEventTapIsEnabled(tap:Ptr)->bool;
    invalidate=>CFMachPortInvalidate(port:Ptr)->();
    port_valid=>CFMachPortIsValid(port:Ptr)->u8;
    source=>CFMachPortCreateRunLoopSource(allocator:Ptr,port:Ptr,order:isize)->Ptr;
    runloop=>CFRunLoopGetMain()->Ptr;
    add_source=>CFRunLoopAddSource(runloop:Ptr,source:Ptr,mode:Ptr)->();
    remove_source=>CFRunLoopRemoveSource(runloop:Ptr,source:Ptr,mode:Ptr)->();
    field=>CGEventGetIntegerValueField(event:Ptr,field:u32)->i64;
    flags=>CGEventGetFlags(event:Ptr)->u64;
    unicode=>CGEventKeyboardGetUnicodeString(event:Ptr,max:usize,actual:*mut usize,buffer:*mut u16)->();
    keyboard_event=>CGEventCreateKeyboardEvent(source:Ptr,key:u16,down:bool)->Ptr;
    set_flags=>CGEventSetFlags(event:Ptr,flags:u64)->();
    set_field=>CGEventSetIntegerValueField(event:Ptr,field:u32,value:i64)->();
    set_unicode=>CGEventKeyboardSetUnicodeString(event:Ptr,count:usize,buffer:*const u16)->();
    post=>CGEventPost(location:u32,event:Ptr)->();
    release=>CFRelease(object:Ptr)->();
    type_id=>CFGetTypeID(object:Ptr)->usize;
    equal=>CFEqual(a:Ptr,b:Ptr)->u8;
    string_type=>CFStringGetTypeID()->usize;
    string_len=>CFStringGetLength(string:Ptr)->isize;
    string_units=>CFStringGetCharacters(string:Ptr,range:Range,buffer:*mut u16)->();
    ax_system=>AXUIElementCreateSystemWide()->Ptr;
    ax_app=>AXUIElementCreateApplication(pid:i32)->Ptr;
    ax_timeout=>AXUIElementSetMessagingTimeout(element:Ptr,seconds:f32)->i32;
    ax_copy=>AXUIElementCopyAttributeValue(element:Ptr,attribute:Ptr,out:*mut Ptr)->i32;
    ax_type=>AXUIElementGetTypeID()->usize;
    value_type=>AXValueGetTypeID()->usize;
    value_kind=>AXValueGetType(value:Ptr)->u32;
    value_range=>AXValueGetValue(value:Ptr,kind:u32,out:*mut c_void)->u8;
    secure=>IsSecureEventInputEnabled()->u8;
    trusted=>AXIsProcessTrusted()->u8;
}
thread_local! {
    // AXAttributeConstants.h defines CFSTR literals, not exported variables.
    static ATTRIBUTES:[Retained<objc2_foundation::NSString>;4] =
        ["AXFocusedUIElement","AXSubrole","AXSelectedTextRange","AXValue"]
            .map(objc2_foundation::NSString::from_str);
}
unsafe extern "C" fn attr(kind: u8) -> Ptr {
    ATTRIBUTES.with(|values| {
        values.get(kind as usize).map_or(std::ptr::null_mut(), |s| {
            Retained::as_ptr(s).cast_mut().cast()
        })
    })
}
// A retained app is boxed so every helper has an opaque, stable, main-owned
// handle. The fixture owns its own representation and its own release method.
unsafe extern "C" fn front() -> Ptr {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map_or(std::ptr::null_mut(), |app| {
            Box::into_raw(Box::new(app)).cast()
        })
}
unsafe extern "C" fn app_release(value: Ptr) {
    if !value.is_null() {
        drop(unsafe { Box::from_raw(value.cast::<Retained<NSRunningApplication>>()) });
    }
}
unsafe fn app<'a>(value: Ptr) -> &'a NSRunningApplication {
    unsafe { &*value.cast::<Retained<NSRunningApplication>>() }
}
unsafe extern "C" fn app_pid(value: Ptr) -> i32 {
    unsafe { app(value) }.processIdentifier()
}
unsafe extern "C" fn app_bundle(value: Ptr) -> Ptr {
    unsafe { app(value) }
        .bundleIdentifier()
        .map_or(std::ptr::null_mut(), |s| Retained::into_raw(s).cast())
}
unsafe extern "C" fn app_equal(a: Ptr, b: Ptr) -> bool {
    unsafe { app(a) }.isEqual(Some(unsafe { app(b) }))
}
unsafe extern "C" fn app_matches_notification(a: Ptr, b: Ptr) -> bool {
    // b is borrowed from the main-owned, typed retained notification Event.
    unsafe { app(a) }.isEqual(Some(unsafe { &*b.cast::<NSRunningApplication>() }))
}
unsafe extern "C" fn app_terminated(value: Ptr) -> bool {
    unsafe { app(value) }.isTerminated()
}
unsafe extern "C" fn app_activate(value: Ptr) -> bool {
    unsafe { app(value) }.activateWithOptions(NSApplicationActivationOptions::empty())
}

extern "C" {
    static kCFRunLoopCommonModes: Ptr;
}
unsafe extern "C" fn common() -> Ptr {
    unsafe { kCFRunLoopCommonModes }
}
unsafe extern "C" fn raycast() -> bool {
    NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str("com.raycast.macos"),
    )
    .iter()
    .any(|app| !app.isTerminated())
}
unsafe extern "C" fn clipboard() -> Ptr {
    objc2_app_kit::NSPasteboard::generalPasteboard()
        .stringForType(unsafe { objc2_app_kit::NSPasteboardTypeString })
        .map_or(std::ptr::null_mut(), |value| {
            Retained::into_raw(value).cast()
        })
}

unsafe fn utf8<'a>(p: *const u8, n: usize) -> Option<&'a str> {
    if n == 0 {
        return Some("");
    }
    if p.is_null() || n > isize::MAX as usize {
        return None;
    }
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, n) }).ok()
}
unsafe extern "C" fn item_create(
    text: *const u8,
    n: usize,
    html: *const u8,
    h: usize,
    has_html: bool,
) -> Ptr {
    let Some(main) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    let Some(text) = (unsafe { utf8(text, n) }) else {
        return std::ptr::null_mut();
    };
    let html = if has_html {
        match unsafe { utf8(html, h) } {
            Some(v) => Some(v.to_owned()),
            None => return std::ptr::null_mut(),
        }
    } else {
        None
    };
    let payload = crate::snippet_output::Payload {
        text: text.to_owned(),
        html,
        cursor: None,
    };
    crate::snippet_clipboard::PreparedItem::new(main, &payload)
        .map_or(std::ptr::null_mut(), |item| {
            Box::into_raw(Box::new(item)).cast()
        })
}
unsafe extern "C" fn item_release(value: Ptr) {
    if !value.is_null() {
        drop(unsafe { Box::from_raw(value.cast::<crate::snippet_clipboard::PreparedItem>()) });
    }
}
unsafe extern "C" fn board_clear() -> isize {
    objc2_app_kit::NSPasteboard::generalPasteboard().clearContents()
}
unsafe extern "C" fn board_write(value: Ptr) -> bool {
    unsafe { &*value.cast::<crate::snippet_clipboard::PreparedItem>() }
        .write_to(&objc2_app_kit::NSPasteboard::generalPasteboard())
}
unsafe extern "C" fn board_change() -> isize {
    objc2_app_kit::NSPasteboard::generalPasteboard().changeCount()
}
unsafe extern "C" fn board_matches(value: Ptr) -> bool {
    unsafe { &*value.cast::<crate::snippet_clipboard::PreparedItem>() }
        .matches(&objc2_app_kit::NSPasteboard::generalPasteboard())
}

unsafe extern "C" fn board_ours(value: Ptr) -> bool {
    unsafe { &*value.cast::<crate::snippet_clipboard::PreparedItem>() }
        .observes_own_content(&objc2_app_kit::NSPasteboard::generalPasteboard())
}
