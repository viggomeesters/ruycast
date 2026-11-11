//! Own synthetic tap/events and bounded output trace. No real event APIs.
use super::{
    keyword_board as board,
    keyword_core::{self as core, Object, Ptr, TapCallback},
};
use std::{
    ffi::CString,
    sync::{LazyLock, Mutex},
};
struct State {
    posts: u32,
    creates: u32,
    output: String,
    buffer: CString,
}
static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        posts: 0,
        creates: 0,
        output: String::new(),
        buffer: CString::default(),
    })
});
fn tap() -> *mut Object {
    core::STATE.lock().unwrap().tap as *mut Object
}
macro_rules! sdk {
    ($local:ident => $native:ident($($arg:ident: $ty:ty),*) -> $ret:ty $body:block) => {
        #[export_name = concat!("ruycast_keyword_fixture_", stringify!($native))]
        unsafe extern "C" fn $local($($arg: $ty),*) -> $ret $body
    };
}
sdk!(tap_create => CGEventTapCreate(location:u32,place:u32,options:u32,mask:u64,callback:Option<TapCallback>,context:Ptr)->Ptr {
    assert!(location==1 && place==0 && options==0 && mask==((1<<1)|(1<<3)|(1<<10)));
    if core::call(c"tap_create") {return std::ptr::null_mut();}
    assert!(tap().is_null()); let value=core::object(8); (*value).valid=true; (*value).callback=callback; (*value).context=context as usize;
    core::STATE.lock().unwrap().tap=value as usize;
    {let mut state=STATE.lock().unwrap(); state.creates=state.creates.wrapping_add(1);} value.cast()
});
sdk!(enable => CGEventTapEnable(value:Ptr,enabled:bool)->() {
    assert_eq!(value,tap().cast()); if !core::call(if enabled {c"tap_enable"} else {c"tap_disable"}) {(*tap()).enabled=enabled;}
});
sdk!(enabled => CGEventTapIsEnabled(value:Ptr)->bool {assert_eq!(value,tap().cast()); !core::call(c"tap_enabled") && (*tap()).enabled});
sdk!(invalidate => CFMachPortInvalidate(value:Ptr)->() {assert_eq!(value,tap().cast()); if !core::call(c"invalidate") {(*tap()).valid=false;}});
sdk!(valid => CFMachPortIsValid(value:Ptr)->u8 {assert_eq!(value,tap().cast()); core::call(c"valid"); (*tap()).valid as u8});
sdk!(source => CFMachPortCreateRunLoopSource(allocator:Ptr,value:Ptr,order:isize)->Ptr {
    assert!(allocator.is_null() && value==tap().cast() && order==0); if core::call(c"source") {std::ptr::null_mut()} else {core::object(9).cast()}
});
sdk!(runloop => CFRunLoopGetMain()->Ptr {core::call(c"runloop"); 201usize as Ptr});
sdk!(add_source => CFRunLoopAddSource(runloop:Ptr,source:Ptr,mode:Ptr)->() {assert!(runloop as usize==201 && (*source.cast::<Object>()).kind==9 && mode as usize==200); core::call(c"add_source");});
sdk!(remove_source => CFRunLoopRemoveSource(runloop:Ptr,source:Ptr,mode:Ptr)->() {assert!(runloop as usize==201 && (*source.cast::<Object>()).kind==9 && mode as usize==200); core::call(c"remove_source");});
sdk!(field => CGEventGetIntegerValueField(value:Ptr,field:u32)->i64 {
    core::call(c"field"); let value=value.cast::<Object>();
    if field==42 {(*value).tag} else if field==9 {(*value).key} else {assert_eq!(field,8); (*value).repeat}
});
sdk!(flags => CGEventGetFlags(value:Ptr)->u64 {core::call(c"flags"); (*value.cast::<Object>()).event_flags});
sdk!(unicode => CGEventKeyboardGetUnicodeString(value:Ptr,max:usize,actual:*mut usize,out:*mut u16)->() {
    core::call(c"unicode"); let value=value.cast::<Object>(); assert!(max==16 && !actual.is_null() && !out.is_null());
    *actual=(*value).units.len(); let n=max.min(*actual); if n!=0 {std::ptr::copy_nonoverlapping((*value).units.as_ptr(),out,n);}
});
sdk!(keyboard => CGEventCreateKeyboardEvent(source:Ptr,key:u16,down:bool)->Ptr {
    assert!(source.is_null()); if core::call(c"keyboard_event") || core::call(if down {c"event:down"} else {c"event:up"}) {return std::ptr::null_mut();}
    let value=core::object(5); (*value).key=key.into(); (*value).down=down; if key==9 {(*value).units=vec![118];} value.cast()
});
sdk!(set_flags => CGEventSetFlags(value:Ptr,flags:u64)->() {(*value.cast::<Object>()).event_flags=flags; core::call(c"set_flags");});
sdk!(set_field => CGEventSetIntegerValueField(value:Ptr,field:u32,data:i64)->() {assert_eq!(field,42); (*value.cast::<Object>()).tag=data; core::call(c"set_field");});
sdk!(set_unicode => CGEventKeyboardSetUnicodeString(value:Ptr,count:usize,units:*const u16)->() {
    let data=if count==0 {&[]} else {assert!(!units.is_null()); std::slice::from_raw_parts(units,count)};
    (*value.cast::<Object>()).units=data.to_vec(); core::call(c"set_unicode");
});
sdk!(post => CGEventPost(location:u32,value:Ptr)->() {
    let value=value.cast::<Object>(); assert!(location==0 && (*value).kind==5 && (*value).tag==0x5255535443415354);
    {let mut state=STATE.lock().unwrap(); state.posts=state.posts.wrapping_add(1);}
    if (*value).key==9 && (*value).down && (*value).event_flags&(1<<20)!=0 {
        let mut state=board::STATE.lock().unwrap(); state.paste_text=state.text.clone(); state.pastes=state.pastes.wrapping_add(1);
    }
    if (*value).key==123 && (*value).down {
        if let Some(home)=std::env::var_os("RUYCAST_KEYWORD_FIXTURE_HOME") {std::fs::write(std::path::PathBuf::from(home).join("keyword-cursor-entered"),"issued").unwrap();}
    }
    {let mut state=STATE.lock().unwrap();
        if state.output.len()+100+(*value).units.len()*8<262144 {
            if !state.output.is_empty() {state.output.push(',');}
            state.output.push_str(&format!("{{\"key\":{},\"down\":{},\"flags\":{},\"units\":[",(*value).key,(*value).down,(*value).event_flags));
            for (index,unit) in (*value).units.iter().enumerate() {if index!=0 {state.output.push(',');} state.output.push_str(&unit.to_string());}
            state.output.push_str("]}");
        }
    }
    core::call(if (*value).down {c"post:down"} else {c"post:up"}); core::call(c"post");
});
sdk!(emit => emit(kind:u32,key:i64,flags:u64,repeat:i64,tag:i64,units:*const u16,count:usize)->bool {
    let data=if count==0 {&[]} else {assert!(!units.is_null()); std::slice::from_raw_parts(units,count)};
    let value=core::string(data); (*value).kind=5; (*value).key=key; (*value).event_flags=flags; (*value).repeat=repeat; (*value).tag=tag;
    let tap=tap(); let consumed=if !tap.is_null() && (*tap).valid && (*tap).enabled {
        let callback=(*tap).callback.expect("Live tap callback"); let context=(*tap).context;
        callback(205usize as Ptr,kind,value.cast(),context as Ptr).is_null()
    } else {false};
    core::release(value); consumed
});
sdk!(clear_output => clear_output()->() {
    {let mut state=STATE.lock().unwrap(); state.output.clear(); state.posts=0;}
    let mut state=board::STATE.lock().unwrap(); state.pastes=0; state.paste_text.clear();
});
fn byte_json(data: &[u8]) -> serde_json::Value {
    if data.len() > 8192 {
        serde_json::Value::Null
    } else {
        serde_json::json!(data)
    }
}
sdk!(state => state()->*const libc::c_char {
    let (allocated,apps,calls,activations,unitreads)={let state=core::STATE.lock().unwrap(); (state.allocated,state.apps,state.calls,state.activation_requests,state.value_unit_reads)};
    let (copies,values)=*super::keyword_ax::COUNTS.lock().unwrap();
    let (posts,creates,output)={let state=STATE.lock().unwrap(); (state.posts,state.creates,state.output.clone())};
    let tap=tap(); let (valid,enabled)=if tap.is_null() {(false,false)} else {((*tap).valid,(*tap).enabled)};
    let mut value={let state=board::STATE.lock().unwrap(); serde_json::json!({
        "clipboardReads":state.clipboard_reads,"boardCount":state.count,"boardClears":state.clears,"boardWrites":state.writes,"pasteIssues":state.pastes,
        "boardTextBytes":state.text.len(),"boardHTMLBytes":state.html.len(),"boardText":byte_json(&state.text),"boardHTML":if state.has_html {byte_json(&state.html)} else {serde_json::Value::Null},"pasteText":byte_json(&state.paste_text)
    })};
    let extra=serde_json::json!({"allocated":allocated,"apps":apps,"copies":copies,"valueReads":values,"valueUnitReads":unitreads,"calls":calls,"posts":posts,"activationRequests":activations,"tapCreates":creates,"tapValid":valid,"tapEnabled":enabled,"output":serde_json::from_str::<serde_json::Value>(&format!("[{output}]")).unwrap()});
    value.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let bytes=serde_json::to_vec(&value).unwrap(); assert!(bytes.len()<600000);
    let mut state=STATE.lock().unwrap(); state.buffer=CString::new(bytes).unwrap(); state.buffer.as_ptr()
});
