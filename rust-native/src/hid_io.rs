//! Complete main-only HID/Quartz boundary. Test mode requires every method;
//! no missing method can fall back to an actual listener, permission or post.
use objc2::MainThreadMarker;
use std::ffi::{c_char, c_void};
pub(crate) type Ptr = *mut c_void;
pub(crate) type HidCallback = unsafe extern "C" fn(Ptr, i32, Ptr, Ptr);
pub(crate) type TapCallback = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> Ptr;
#[link(name = "IOKit", kind = "framework")]
extern "C" {}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {}
#[link(name = "Carbon", kind = "framework")]
extern "C" {}
macro_rules! api {
    ($($field:ident => $native:ident ($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        extern "C" { $(fn $native($($arg:$ty),*) -> $ret;)* }
        #[derive(Clone, Copy)]
        pub(crate) struct Api { $(pub $field:unsafe extern "C" fn($($ty),*) -> $ret,)*
            pub dictionary:unsafe extern "C" fn(bool)->Ptr,
            pub common:unsafe extern "C" fn()->Ptr,
            pub uptime:unsafe extern "C" fn()->f64,
            pub raycast:unsafe extern "C" fn()->bool,
        }
        impl Api {
            pub fn load(_main: MainThreadMarker, fixture:bool)->Result<Self,String> {
                if !fixture { return Ok(Self { $($field:$native,)* dictionary,common,uptime,raycast }); }
                #[cfg(feature="test-tools")]
                unsafe {
                    let stamp=libc::dlsym(libc::RTLD_DEFAULT,c"ruycast_hid_fixture_ready".as_ptr());
                    if stamp.is_null() || std::mem::transmute::<Ptr,unsafe extern "C" fn()->u64>(stamp)()!=0x5255484944303031 {
                        return Err("Complete HID SDK fixture unavailable.".into());
                    }
                    $(let $field=libc::dlsym(libc::RTLD_DEFAULT,concat!("ruycast_hid_fixture_",stringify!($native),"\0").as_ptr().cast());
                      if $field.is_null() {return Err("Complete HID SDK fixture unavailable.".into());})*
                    let helpers=[c"ruycast_hid_fixture_dictionary",c"ruycast_hid_fixture_common",c"ruycast_hid_fixture_uptime",c"ruycast_hid_fixture_raycast"]
                        .map(|name|libc::dlsym(libc::RTLD_DEFAULT,name.as_ptr()));
                    if helpers.iter().any(|p|p.is_null()) {return Err("Complete HID SDK fixture unavailable.".into());}
                    Ok(Self { $($field:std::mem::transmute::<Ptr,unsafe extern "C" fn($($ty),*)->$ret>($field),)*
                        dictionary:std::mem::transmute::<Ptr,unsafe extern "C" fn(bool)->Ptr>(helpers[0]),
                        common:std::mem::transmute::<Ptr,unsafe extern "C" fn()->Ptr>(helpers[1]),
                        uptime:std::mem::transmute::<Ptr,unsafe extern "C" fn()->f64>(helpers[2]),
                        raycast:std::mem::transmute::<Ptr,unsafe extern "C" fn()->bool>(helpers[3]),
                    })
                }
                #[cfg(not(feature="test-tools"))]
                Err("HID fixtures require test-tools.".into())
            }
        }
    }
}
api! {
    manager_create => IOHIDManagerCreate(allocator:Ptr,options:u32)->Ptr;
    manager_open => IOHIDManagerOpen(manager:Ptr,options:u32)->i32;
    manager_close => IOHIDManagerClose(manager:Ptr,options:u32)->i32;
    device_matching => IOHIDManagerSetDeviceMatching(manager:Ptr,dictionary:Ptr)->();
    input_matching => IOHIDManagerSetInputValueMatching(manager:Ptr,dictionary:Ptr)->();
    register_input => IOHIDManagerRegisterInputValueCallback(manager:Ptr,callback:Option<HidCallback>,context:Ptr)->();
    register_remove => IOHIDManagerRegisterDeviceRemovalCallback(manager:Ptr,callback:Option<HidCallback>,context:Ptr)->();
    schedule => IOHIDManagerScheduleWithRunLoop(manager:Ptr,runloop:Ptr,mode:Ptr)->();
    unschedule => IOHIDManagerUnscheduleFromRunLoop(manager:Ptr,runloop:Ptr,mode:Ptr)->();
    element => IOHIDValueGetElement(value:Ptr)->Ptr;
    integer => IOHIDValueGetIntegerValue(value:Ptr)->isize;
    usage_page => IOHIDElementGetUsagePage(element:Ptr)->u32;
    usage => IOHIDElementGetUsage(element:Ptr)->u32;
    device => IOHIDElementGetDevice(element:Ptr)->Ptr;
    tap_create => CGEventTapCreate(location:u32,place:u32,options:u32,mask:u64,callback:TapCallback,context:Ptr)->Ptr;
    tap_enable => CGEventTapEnable(tap:Ptr,enable:bool)->();
    tap_enabled => CGEventTapIsEnabled(tap:Ptr)->bool;
    port_valid => CFMachPortIsValid(port:Ptr)->u8;
    invalidate => CFMachPortInvalidate(port:Ptr)->();
    source => CFMachPortCreateRunLoopSource(allocator:Ptr,port:Ptr,order:isize)->Ptr;
    runloop => CFRunLoopGetMain()->Ptr;
    add_source => CFRunLoopAddSource(runloop:Ptr,source:Ptr,mode:Ptr)->();
    remove_source => CFRunLoopRemoveSource(runloop:Ptr,source:Ptr,mode:Ptr)->();
    release => CFRelease(value:Ptr)->();
    flags => CGEventGetFlags(event:Ptr)->u64;
    set_flags => CGEventSetFlags(event:Ptr,flags:u64)->();
    field => CGEventGetIntegerValueField(event:Ptr,field:u32)->i64;
    set_field => CGEventSetIntegerValueField(event:Ptr,field:u32,value:i64)->();
    set_type => CGEventSetType(event:Ptr,kind:u32)->();
    event_source => CGEventSourceCreate(state:i32)->Ptr;
    keyboard_event => CGEventCreateKeyboardEvent(source:Ptr,key:u16,down:bool)->Ptr;
    post => CGEventPost(location:u32,event:Ptr)->();
    secure => IsSecureEventInputEnabled()->u8;
    accessibility => AXIsProcessTrusted()->u8;
    monitoring => CGPreflightListenEventAccess()->bool;
}
extern "C" {
    static kCFRunLoopCommonModes: Ptr;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFStringCreateWithCString(allocator: Ptr, value: *const c_char, encoding: u32) -> Ptr;
    fn CFNumberCreate(allocator: Ptr, kind: isize, value: *const c_void) -> Ptr;
    fn CFDictionaryCreate(
        allocator: Ptr,
        keys: *const Ptr,
        values: *const Ptr,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> Ptr;
}
unsafe extern "C" fn common() -> Ptr {
    unsafe { kCFRunLoopCommonModes }
}
unsafe extern "C" fn uptime() -> f64 {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut value) } != 0 {
        return f64::NAN;
    }
    value.tv_sec as f64 + value.tv_nsec as f64 / 1e9
}
unsafe extern "C" fn raycast() -> bool {
    !objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str("com.raycast.macos"),
    )
    .is_empty()
}
unsafe extern "C" fn dictionary(device: bool) -> Ptr {
    let names = if device {
        vec![c"DeviceUsagePage", c"DeviceUsage"]
    } else {
        vec![c"UsagePage"]
    };
    let numbers = if device { vec![1i32, 6] } else { vec![7i32] };
    let mut keys = Vec::new();
    let mut values = Vec::new();
    for (name, value) in names.iter().zip(numbers.iter()) {
        keys.push(unsafe {
            CFStringCreateWithCString(std::ptr::null_mut(), name.as_ptr(), 0x08000100)
        });
        values.push(unsafe { CFNumberCreate(std::ptr::null_mut(), 3, (value as *const i32).cast()) });
    }
    let result = if keys.iter().chain(values.iter()).any(|p| p.is_null()) {
        std::ptr::null_mut()
    } else {
        unsafe {
            CFDictionaryCreate(
                std::ptr::null_mut(),
                keys.as_ptr(),
                values.as_ptr(),
                keys.len() as isize,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
        }
    };
    for value in keys.into_iter().chain(values) {
        if !value.is_null() {
            unsafe { CFRelease(value) }
        }
    }
    result
}
