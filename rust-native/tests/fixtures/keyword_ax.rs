//! Synthetic AX objects/values only; never invokes Accessibility services.
use super::keyword_core::{self as core, Object, Ptr, Range};
use std::{ffi::CString, sync::Mutex};
pub(super) static COUNTS: Mutex<(u32, u32)> = Mutex::new((0, 0)); // copies, value reads
macro_rules! sdk {
    ($local:ident => $native:ident($($arg:ident: $ty:ty),*) -> $ret:ty $body:block) => {
        #[export_name = concat!("ruycast_keyword_fixture_", stringify!($native))]
        unsafe extern "C" fn $local($($arg: $ty),*) -> $ret $body
    };
}
sdk!(system => AXUIElementCreateSystemWide()->Ptr {
    if core::call(c"system") {std::ptr::null_mut()} else {core::object(11).cast()}
});
sdk!(application => AXUIElementCreateApplication(pid:i32)->Ptr {
    assert!(pid>0); if core::call(c"ax_app") {return std::ptr::null_mut();}
    let value=core::object(12); (*value).pid=pid; value.cast()
});
sdk!(timeout => AXUIElementSetMessagingTimeout(value:Ptr,seconds:f32)->i32 {
    assert!(seconds>0.009 && seconds<0.011);
    let kind=(*value.cast::<Object>()).kind;
    if core::call(if kind==11 {c"timeout:system"} else if kind==12 {c"timeout:app"} else {c"timeout:element"}) {-25204} else {0}
});
sdk!(copy => AXUIElementCopyAttributeValue(value:Ptr,attribute:Ptr,out:*mut Ptr)->i32 {
    assert!(!out.is_null() && (*out).is_null());
    {let mut counts=COUNTS.lock().unwrap(); counts.0=counts.0.wrapping_add(1);}
    let value=value.cast::<Object>(); let kind=(attribute as usize).wrapping_sub(101); assert!(kind<4);
    let name=CString::new(format!("copy:{kind}")).unwrap(); let failed=core::call(&name);
    let (flags,element,selection,text)={let state=core::STATE.lock().unwrap();
        let text=if kind==3 && state.flags&1024==0 {state.text.clone()} else {Vec::new()};
        (state.flags,state.element_identity,state.selection,text)};
    let mut result=std::ptr::null_mut();
    if kind==0 {
        assert!((*value).kind==11 || (*value).kind==12);
        if flags&32==0 {result=core::object(if flags&64!=0 {4} else {1}); (*result).identity=element;}
    } else {
        assert_eq!((*value).kind,1);
        if kind==1 && flags&128==0 {result=core::string(&if flags&16!=0 {"AXSecureTextField"} else {"AXTextField"}.encode_utf16().collect::<Vec<_>>());}
        if kind==2 && flags&256==0 {result=core::object(3); (*result).value_kind=if flags&512!=0 {1} else {4}; (*result).range=selection;}
        if kind==3 {
            {let mut counts=COUNTS.lock().unwrap(); counts.1=counts.1.wrapping_add(1);}
            if flags&1024==0 {result=core::string(&text); (*result).identity=u64::MAX;}
        }
    }
    if failed && flags&2048==0 {if !result.is_null() {core::release(result);} result=std::ptr::null_mut();}
    *out=result.cast(); if failed {-25204} else {0}
});
sdk!(ax_type => AXUIElementGetTypeID()->usize {core::call(c"ax_type"); 1});
#[allow(
    dead_code,
    reason = "The negative artifact intentionally omits its value-type export wrapper"
)]
pub(super) unsafe fn value_type() -> usize {
    core::call(c"value_type");
    3
}
sdk!(value_kind => AXValueGetType(value:Ptr)->u32 {assert_eq!((*value.cast::<Object>()).kind,3); core::call(c"value_kind"); (*value.cast::<Object>()).value_kind});
sdk!(range => AXValueGetValue(value:Ptr,kind:u32,out:Ptr)->u8 {
    assert!((*value.cast::<Object>()).kind==3 && kind==4 && !out.is_null());
    if core::call(c"value_range") {return 0;} out.cast::<Range>().write((*value.cast::<Object>()).range); 1
});
sdk!(secure => IsSecureEventInputEnabled()->u8 {core::call(c"secure"); u8::from(core::STATE.lock().unwrap().flags&8!=0)});
sdk!(trusted => AXIsProcessTrusted()->u8 {core::call(c"trusted"); u8::from(core::STATE.lock().unwrap().flags&4096==0)});
sdk!(common => common()->Ptr {core::call(c"common"); 200usize as Ptr});
sdk!(raycast => raycast()->bool {core::call(c"raycast"); core::STATE.lock().unwrap().flags&8192!=0});
