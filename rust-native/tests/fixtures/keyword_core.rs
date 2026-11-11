//! Partial shared synthetic-object component. Deliberately exports no SDK-ready stamp.
#![allow(
    dead_code,
    reason = "Shared internal interfaces are consumed by subsequent notification/AX/board ports"
)]
use std::{
    ffi::{c_void, CStr, CString, OsStr},
    os::unix::ffi::OsStrExt,
    sync::{LazyLock, Mutex},
};
pub(super) type Ptr = *mut c_void;
pub(super) type Hook = unsafe extern "C" fn(Ptr);
pub(super) type TapCallback = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> Ptr;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Range {
    pub location: isize,
    pub length: isize,
}
#[derive(Default)]
pub(super) struct Object {
    pub kind: u32,
    pub identity: u64,
    pub pid: i32,
    pub units: Vec<u16>,
    pub utf8: Vec<u8>,
    pub html: Option<Vec<u8>>,
    pub valid: bool,
    pub enabled: bool,
    pub range: Range,
    pub value_kind: u32,
    pub down: bool,
    pub event_flags: u64,
    pub key: i64,
    pub tag: i64,
    pub repeat: i64,
    pub callback: Option<TapCallback>,
    pub context: usize,
}
pub(super) struct State {
    pub allocated: u32,
    pub apps: u32,
    pub calls: u32,
    pub activation_requests: u32,
    pub value_unit_reads: u32,
    pub flags: u32,
    pub front_pid: i32,
    pub app_identity: u64,
    pub element_identity: u64,
    pub selection: Range,
    pub text: Vec<u16>,
    pub bundle: Option<Vec<u16>>,
    pub tap: usize,
    failure: CString,
    hook_name: CString,
    hook: Option<Hook>,
    hook_context: usize,
    change_name: CString,
    change_after: u32,
    change_flags: u32,
    change_pid: i32,
    change_app: u64,
    change_element: u64,
}
pub(super) static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        allocated: 0,
        apps: 0,
        calls: 0,
        activation_requests: 0,
        value_unit_reads: 0,
        flags: 0,
        front_pid: 42,
        app_identity: 1,
        element_identity: 1,
        selection: Range {
            location: 3,
            length: 0,
        },
        text: Vec::new(),
        bundle: None,
        tap: 0,
        failure: CString::default(),
        hook_name: CString::default(),
        hook: None,
        hook_context: 0,
        change_name: CString::default(),
        change_after: 1,
        change_flags: 0,
        change_pid: 0,
        change_app: 0,
        change_element: 0,
    })
});
fn bounded_name(name: &CStr) -> CString {
    CString::new(&name.to_bytes()[..name.to_bytes().len().min(99)]).unwrap()
}
pub(super) unsafe fn call(name: &CStr) -> bool {
    {
        let mut state = STATE.lock().unwrap();
        state.calls = state.calls.wrapping_add(1);
    }
    let mut file_failure = false;
    if let Some(home) = std::env::var_os("RUYCAST_KEYWORD_FIXTURE_HOME") {
        let home = std::path::PathBuf::from(home);
        let named = |prefix: &[u8], suffix: &[u8]| {
            let mut bytes = prefix.to_vec();
            bytes.extend_from_slice(name.to_bytes());
            bytes.extend_from_slice(suffix);
            home.join(OsStr::from_bytes(&bytes))
        };
        file_failure = named(b"keyword-fail-", b"").exists();
        if named(b"hold-keyword-", b"").exists() {
            std::fs::write(named(b"keyword-", b"-entered"), "entered").unwrap();
            // Existing release protocol: bounded by the owning test child's timeout.
            while !named(b"keyword-", b"-release").exists() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    let hook = {
        let mut state = STATE.lock().unwrap();
        if !state.change_name.as_bytes().is_empty() && state.change_name.as_c_str() == name {
            state.change_after = state.change_after.wrapping_sub(1);
            if state.change_after == 0 {
                state.change_name = CString::default();
                state.flags = state.change_flags;
                state.front_pid = state.change_pid;
                state.app_identity = state.change_app;
                state.element_identity = state.change_element;
            }
        }
        if state.hook_name.as_c_str() == name {
            state
                .hook
                .take()
                .map(|callback| (callback, state.hook_context))
        } else {
            None
        }
    };
    // Match C ordering: change, one-shot callback, then current failure selection.
    if let Some((callback, context)) = hook {
        callback(context as Ptr);
    }
    file_failure || STATE.lock().unwrap().failure.as_c_str() == name
}
pub(super) fn object(kind: u32) -> *mut Object {
    let value = Box::into_raw(Box::new(Object {
        kind,
        ..Object::default()
    }));
    let mut state = STATE.lock().unwrap();
    state.allocated = state.allocated.wrapping_add(1);
    value
}
pub(super) fn string(units: &[u16]) -> *mut Object {
    let value = object(2);
    unsafe {
        (*value).units = units.to_vec();
    }
    value
}
pub(super) unsafe fn release(value: *mut Object) {
    assert!(!value.is_null());
    {
        let mut state = STATE.lock().unwrap();
        assert_ne!(state.allocated, 0);
        if value as usize == state.tap {
            assert!(!(*value).valid);
            state.tap = 0;
        }
        state.allocated = state.allocated.wrapping_sub(1);
    }
    drop(Box::from_raw(value));
}
macro_rules! sdk {
    ($local:ident => $native:ident($($arg:ident: $ty:ty),*) -> $ret:ty $body:block) => {
        #[export_name = concat!("ruycast_keyword_fixture_", stringify!($native))]
        pub(super) unsafe extern "C" fn $local($($arg: $ty),*) -> $ret $body
    };
}
sdk!(hook => hook(name:*const libc::c_char,callback:Option<Hook>,context:Ptr)->() {
    let mut state=STATE.lock().unwrap(); state.hook_name=bounded_name(CStr::from_ptr(name)); state.hook=callback; state.hook_context=context as usize;
});
sdk!(failure => failure(name:*const libc::c_char)->() { STATE.lock().unwrap().failure=bounded_name(CStr::from_ptr(name)); });
sdk!(environment => environment(flags:u32,pid:i32,app:u64,element:u64)->() {
    let mut state=STATE.lock().unwrap(); state.flags=flags; state.front_pid=pid; state.app_identity=app; state.element_identity=element;
});
sdk!(change => change(name:*const libc::c_char,flags:u32,pid:i32,app:u64,element:u64)->() { change_after(name,1,flags,pid,app,element); });
sdk!(change_after => change_after(name:*const libc::c_char,nth:u32,flags:u32,pid:i32,app:u64,element:u64)->() {
    assert!(nth>0); let mut state=STATE.lock().unwrap(); state.change_name=bounded_name(CStr::from_ptr(name)); state.change_after=nth;
    state.change_flags=flags; state.change_pid=pid; state.change_app=app; state.change_element=element;
});
unsafe fn units<'a>(data: *const u16, length: usize) -> &'a [u16] {
    if length == 0 {
        &[]
    } else {
        assert!(!data.is_null());
        std::slice::from_raw_parts(data, length)
    }
}
sdk!(text => text(data:*const u16,length:usize,at:isize,count:isize)->() {
    let mut state=STATE.lock().unwrap(); state.text=units(data,length).to_vec(); state.selection=Range{location:at,length:count};
});
sdk!(set_bundle => set_bundle(data:*const u16,length:usize)->() { STATE.lock().unwrap().bundle=Some(units(data,length).to_vec()); });
sdk!(front => front()->Ptr {
    if call(c"front") {return std::ptr::null_mut();}
    let snapshot={let state=STATE.lock().unwrap(); if state.flags&1!=0{return std::ptr::null_mut();} (state.front_pid,state.app_identity)};
    let value=object(10); (*value).pid=snapshot.0; (*value).identity=snapshot.1; STATE.lock().unwrap().apps+=1; value.cast()
});
sdk!(app_release => app_release(value:Ptr)->() {
    call(c"app_release"); let value=value.cast::<Object>();
    {let mut state=STATE.lock().unwrap(); assert!(state.apps!=0 && (*value).kind==10); state.apps-=1;} release(value);
});
sdk!(app_pid => app_pid(value:Ptr)->i32 { call(c"app_pid"); (*value.cast::<Object>()).pid });
sdk!(app_bundle => app_bundle(value:Ptr)->Ptr {
    assert_eq!((*value.cast::<Object>()).kind,10);
    if call(c"app_bundle") {return std::ptr::null_mut();}
    let bundle={let state=STATE.lock().unwrap(); if state.flags&2!=0{return std::ptr::null_mut();} state.bundle.clone().unwrap_or_else(|| "own.keyword.target".encode_utf16().collect())};
    string(&bundle).cast()
});
sdk!(app_equal => app_equal(a:Ptr,b:Ptr)->bool { call(c"app_equal"); let(a,b)=(a.cast::<Object>(),b.cast::<Object>()); (*a).identity==(*b).identity && (*a).pid==(*b).pid });
sdk!(match_notification_identity => match_notification_identity(value:Ptr,identity:u64,pid:i32)->bool {
    let value=value.cast::<Object>(); assert_eq!((*value).kind,10);
    !call(c"app_matches_notification") && (*value).identity==identity && (*value).pid==pid
});
sdk!(app_terminated => app_terminated(value:Ptr)->bool { assert_eq!((*value.cast::<Object>()).kind,10); call(c"app_terminated"); STATE.lock().unwrap().flags&4!=0 });
sdk!(app_activate => app_activate(value:Ptr)->bool { STATE.lock().unwrap().activation_requests+=1; assert_eq!((*value.cast::<Object>()).kind,10); !call(c"app_activate") });
sdk!(attr => attr(kind:u8)->Ptr { call(c"attr"); assert!(kind<4); (101+usize::from(kind)) as Ptr });
sdk!(cf_release => CFRelease(value:Ptr)->() { assert!(!value.is_null() && (*value.cast::<Object>()).kind!=13); call(c"release"); release(value.cast()); });
sdk!(type_id => CFGetTypeID(value:Ptr)->usize { call(c"type_id"); (*value.cast::<Object>()).kind as usize });
sdk!(equal => CFEqual(a:Ptr,b:Ptr)->u8 { call(c"equal"); let(a,b)=(a.cast::<Object>(),b.cast::<Object>()); u8::from((*a).kind==(*b).kind && (*a).identity==(*b).identity) });
sdk!(string_type => CFStringGetTypeID()->usize { call(c"string_type"); 2 });
sdk!(string_len => CFStringGetLength(value:Ptr)->isize {
    let value=value.cast::<Object>(); assert_eq!((*value).kind,2); if call(c"string_len") {return -1;}
    if (*value).identity==u64::MAX && STATE.lock().unwrap().failure.as_c_str()==c"value_len_huge" {return isize::MAX/2;}
    (*value).units.len() as isize
});
sdk!(string_units => CFStringGetCharacters(value:Ptr,range:Range,output:*mut u16)->() {
    let value=value.cast::<Object>(); assert!((*value).kind==2 && range.location>=0 && range.length>=0 && !output.is_null());
    let(start,length)=(range.location as usize,range.length as usize);
    assert!(start<=(*value).units.len() && length<=(*value).units.len()-start);
    call(c"string_units");
    if (*value).identity==u64::MAX { STATE.lock().unwrap().value_unit_reads+=1; call(c"value_units"); }
    if length!=0 { std::ptr::copy_nonoverlapping((*value).units.as_ptr().add(start),output,length); }
});
