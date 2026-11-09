//! Typed IMPs for the private workspace. Original AppKit calls are confined
//! to separately source-bound operations such as the Reference palette host.
use super::*;
use objc2::{
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, Sel},
    ClassType,
};
#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
use objc2_foundation::NSObject;
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
#[cfg(test)]
use objc2_foundation::NSURL;
#[cfg(test)]
use objc2_foundation::NSArray;
use std::{ffi::CStr, sync::OnceLock};
use std::sync::atomic::AtomicU64;
#[path = "workspace_apple_events.rs"]
pub mod apple_events;
#[path = "workspace_application.rs"]
pub mod application;
#[path = "workspace_clipboard.rs"]
pub mod clipboard_bridge;
#[path = "workspace_controls.rs"]
pub mod controls;
#[path = "workspace_foundation.rs"]
pub mod foundation;
#[path = "workspace_notification_objc.rs"]
pub mod notification_bridge;
#[path = "workspace_data_objc.rs"]
pub mod typed_data;
#[path = "task_objects.rs"]
mod task_objects;
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use task_objects::reference_write_spec;
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use task_objects::{bridge_swift_data, SwiftData};
use typed_data::*;
use foundation::{FoundationValue,Text};
#[derive(Clone, PartialEq, Eq)]
enum Kind {
    Workspace,
    OpenConfiguration,
    BundleIcon(IconIdentity),
    ApplicationShell,
    Menu(Handle),
    MenuItem(Handle),
    StatusBar,
    StatusItem(Handle),
    Button(Handle),
    Application(Handle),
    Screen(usize),
    Window(Handle),
    WindowPayload(WindowIdentity),
    NotificationCenter(usize),
    NotificationToken(usize, u64),
    NotificationPayload(NotificationIdentity),
    MainQueue,
    AppleEventDescriptor(Handle, std::thread::ThreadId, DescriptorLife),
    Event(EventIdentity),
    Data(typed_data::Identity),
    TaskPayload(TaskIdentity),
}
#[derive(Clone)]
struct IconIdentity(Arc<super::images::IconPixels>);
impl PartialEq for IconIdentity {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl Eq for IconIdentity {}
#[derive(Clone)]
struct NotificationIdentity(Arc<NotificationRecord>);
impl PartialEq for NotificationIdentity {
    fn eq(&self,other:&Self)->bool {Arc::ptr_eq(&self.0,&other.0)}
}
impl Eq for NotificationIdentity {}
struct NotificationRecord {
    model:Arc<Workspace>,
    generation:u64,
    owner:std::thread::ThreadId,
    serial:AtomicU64,
    ready:AtomicBool,
    name:FoundationReference,
    object:Option<FoundationReference>,
    window_object:Option<WindowReference>,
    user_info:Option<FoundationReference>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    reference_field:OnceLock<super::receiver::app_delegate::constructor::ReferenceFieldToken>,
}
#[derive(Clone)]
struct TaskIdentity(Arc<TaskRecord>);
impl PartialEq for TaskIdentity {
    fn eq(&self,other:&Self)->bool {Arc::ptr_eq(&self.0,&other.0)}
}
impl Eq for TaskIdentity {}
enum TaskPayload {
    Task(Arc<task_objects::Fields>),
    Pipe(TaskPipeRecord),
    Handle(crate::sdk::task_model::Handle),
    Error(TaskErrorValue),
}
type TaskErrorValue=crate::sdk::task_model::ErrorFields;
struct TaskPipeRecord {
    value:crate::sdk::task_model::Pipe,
    children:Mutex<[Option<Arc<super::factory::TaskChildReference>>;2]>,
}
impl TaskPipeRecord {
    fn new(value:crate::sdk::task_model::Pipe)->Self {Self {value,children:Mutex::new([None,None])}}
}
struct TaskRecord {
    model:Arc<Workspace>,
    registry:Option<Arc<crate::sdk::process_dispatch::Registry>>,
    owner:std::thread::ThreadId,
    serial:std::sync::atomic::AtomicU64,
    ready:AtomicBool,
    payload:TaskPayload,
}
impl TaskRecord {
    fn role(&self)->&'static CStr {
        match self.payload {TaskPayload::Task(_)=>c"NSTask",TaskPayload::Pipe(_)=>c"NSPipe",TaskPayload::Handle(_)=>c"NSFileHandle",TaskPayload::Error(_)=>c"NSError"}
    }
}
fn task_record(pointer:*mut AnyObject,owner_only:bool)->Result<Arc<TaskRecord>> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let record={
        let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {
            Some(Kind::TaskPayload(value))=>value.0.clone(),
            _=>return Err("not a qualified Task/Pipe/Handle payload".into()),
        }
    };
    if !Arc::ptr_eq(&record.model,&installed.model) || !record.ready.load(Ordering::Acquire)
        || (owner_only && record.owner!=std::thread::current().id()
            && !(record.role()==c"NSError" && super::factory::is_immutable_data(pointer,&record.model)?)) {
        return Err("Task payload unready, foreign owner or model".into());
    }
    let serial=record.serial.load(Ordering::Acquire);
    if serial==0 {return Err("Task allocation serial absent".into());}
    let context=if record.role()==c"NSError" {None} else {
        let context=crate::sdk::process_abi::installed_context()?;
        if !record.registry.as_ref().is_some_and(|registry|Arc::ptr_eq(&context.registry,registry)) {
            return Err("Task process registry issuer differs".into());
        }
        context.registry.before_effect()?;
        Some(context)
    };
    if record.role()==c"NSTask" {
        super::factory::verify_task_job(pointer,serial,&record.model)?;
    } else {
        let identity=super::factory::receiver_identity(pointer)?;
        if !identity.belongs_to(&record.model) || identity.allocation_serial()!=serial {
            return Err("Task child allocation changed".into());
        }
        identity.verify()?;
    }
    if let Some(context)=context {context.registry.not_retired()?;}
    super::factory::verify_task_allocation_pure(pointer,serial,record.owner,record.role(),&record.model)?;
    {
        let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        if !matches!(objects.get(&(pointer as usize)),Some(Kind::TaskPayload(value)) if Arc::ptr_eq(&value.0,&record)) {
            return Err("Task arena allocation replaced during callback".into());
        }
    }
    if !record.ready.load(Ordering::Acquire) {return Err("Task payload retired during guard".into());}
    Ok(record)
}
pub(super) fn task_payload_for_factory(pointer:*mut AnyObject,role:&CStr,model:&Arc<Workspace>)->Result<bool> {
    let record={
        let installed=INSTALLED.get().ok_or("workspace absent")?;
        if !Arc::ptr_eq(&installed.model,model) {return Err("Task model differs".into());}
        let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {Some(Kind::TaskPayload(value))=>value.0.clone(),_=>return Ok(false)}
    };
    if !Arc::ptr_eq(&record.model,model) || record.owner!=std::thread::current().id()
        || !record.ready.load(Ordering::Acquire) || record.role()!=role {
        return Ok(false);
    }
    Ok(record.serial.load(Ordering::Acquire)==super::factory::owned_reference_serial(pointer)?)
}
pub(super) fn error_payload_for_factory(pointer:*mut AnyObject,model:&Arc<Workspace>)->Result<bool> {
    let record={
        let installed=INSTALLED.get().ok_or("workspace absent")?;
        if !Arc::ptr_eq(&installed.model,model) {return Err("foreign NSError arena model".into());}
        let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {Some(Kind::TaskPayload(value))=>value.0.clone(),_=>return Ok(false)}
    };
    if !Arc::ptr_eq(&record.model,model) || record.role()!=c"NSError" || !record.ready.load(Ordering::Acquire)
        || (record.owner!=std::thread::current().id() && !super::factory::is_immutable_data(pointer,model)?) {
        return Ok(false);
    }
    super::factory::verify_task_allocation_pure(pointer,record.serial.load(Ordering::Acquire),record.owner,c"NSError",model)?;
    match &record.payload {TaskPayload::Error(fields)=>{validate_task_error_fields(fields,0)?;Ok(true)},_=>Ok(false)}
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_monitor_timeout_task(pointer: *mut AnyObject, files: bool) -> Result<()> {
    use crate::sdk::task_model::TimeoutLaunchRoute;
    let route = if files { TimeoutLaunchRoute::Files } else { TimeoutLaunchRoute::Monitor };
    task_fields(pointer)?.task.verify_timeout_launch(route)
}

fn task_fields(pointer:*mut AnyObject)->Result<Arc<task_objects::Fields>> {
    match &task_record(pointer,true)?.payload {TaskPayload::Task(fields)=>Ok(fields.clone()),_=>Err("not a Task".into())}
}
fn task(pointer:*mut AnyObject)->Result<Arc<crate::sdk::task_model::Task>> {
    match &task_record(pointer,false)?.payload {TaskPayload::Task(fields)=>Ok(fields.task.clone()),_=>Err("not a Task".into())}
}
fn task_pipe_handle(pointer:*mut AnyObject,writing:bool)->Result<*mut AnyObject> {
    let record=task_record(pointer,true)?;
    let TaskPayload::Pipe(pipe)=&record.payload else {return Err("not a Pipe".into());};
    let index=usize::from(writing);
    let held=pipe.children.lock().map_err(|_| "Pipe child references poisoned")?[index].clone();
    let held=if let Some(held)=held {held} else {
        let value=if writing {pipe.value.write.clone()} else {pipe.value.read.clone()};
        let child=new_task_handle(value)?;
        let pending=unsafe {super::factory::OwnedReceiver::from_owned(child)}?;
        let candidate=Arc::new(super::factory::TaskChildReference::from_owned_child(child)?);
        drop(pending);
        if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Pipe changed during child creation".into());}
        let selected={
            let mut children=pipe.children.lock().map_err(|_| "Pipe child references poisoned")?;
            children[index].get_or_insert_with(||candidate.clone()).clone()
        };
        // A racing/reentrant candidate's qualified +1 drops outside the lock.
        drop(candidate);
        selected
    };
    let result=held.autoreleased_pointer()?;
    if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Pipe changed during child return".into());}
    Ok(result)
}
fn handle(pointer:*mut AnyObject)->Result<crate::sdk::task_model::Handle> {
    match &task_record(pointer,true)?.payload {TaskPayload::Handle(value)=>Ok(value.clone()),_=>Err("not a Handle".into())}
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_monitor_stdout(task: *mut AnyObject, reader: *mut AnyObject) -> Result<()> {
    let fields = task_fields(task)?;
    let reader = handle(reader)?;
    fields.task.verify_monitor_stdout(&reader)
}
fn cleanup_task_handle(pointer:*mut AnyObject)->Result<crate::sdk::task_model::Handle> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let record={
        let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {Some(Kind::TaskPayload(value))=>value.0.clone(),_=>return Err("unknown Handle cleanup receiver".into())}
    };
    if record.owner!=std::thread::current().id() || !Arc::ptr_eq(&record.model,&installed.model)
        || !record.ready.load(Ordering::Acquire) {return Err("Handle cleanup owner/model differs".into());}
    super::factory::verify_task_child_cleanup(pointer,record.serial.load(Ordering::Acquire),&record.model)?;
    match &record.payload {
        TaskPayload::Handle(value) if record.registry.as_ref().is_some_and(|registry|value.belongs_to_registry(registry))=>Ok(value.clone()),
        _=>Err("Handle cleanup descriptor issuer differs".into()),
    }
}
fn task_stream_reference(pointer:*mut AnyObject)->Result<(crate::sdk::task_model::Stream,Arc<super::factory::TaskChildReference>)> {
    let record=task_record(pointer,true)?;
    let stream=match &record.payload {
        TaskPayload::Pipe(value)=>crate::sdk::task_model::Stream::Pipe(value.value.clone()),
        TaskPayload::Handle(value)=>crate::sdk::task_model::Stream::Handle(value.clone()),
        _=>return Err("Task stream is not Pipe/Handle".into()),
    };
    let reference=Arc::new(super::factory::TaskChildReference::from_owned_child(pointer)?);
    if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Task stream changed during retain".into());}
    Ok((stream,reference))
}
unsafe fn attach_task_payload(pointer:*mut AnyObject,payload:TaskPayload,initialize:bool)->Result<*mut AnyObject> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(pointer)}?;
    let registry=if matches!(&payload,TaskPayload::Error(_)) {None} else {Some(task_registry()?.registry.clone())};
    let record=Arc::new(TaskRecord {model:installed.model.clone(),registry,owner:std::thread::current().id(),
        serial:std::sync::atomic::AtomicU64::new(0),ready:AtomicBool::new(false),payload});
    {
        let mut objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
        if objects.contains_key(&(pointer as usize)) {return Err("Task initializer replay".into());}
        objects.insert(pointer as usize,Kind::TaskPayload(TaskIdentity(record.clone())));
    }
    if initialize {unsafe {super::factory::initialize(pointer,&installed.model)}?;}
    record.serial.store(super::factory::owned_reference_serial(pointer)?,Ordering::Release);
    record.ready.store(true,Ordering::Release);
    if record.role()==c"NSError" {
        // An immutable receiver may later be messaged on another thread. Its
        // first real Objective-C dispatch must complete the selected class
        // initializer on this creator before foreign-read publication.
        initialize_task_error_on_creator(pointer,&record)?;
        unsafe {super::factory::publish_error_data(pointer,&installed.model)}?;
    } else {
        unsafe {super::factory::publish_task_reference(pointer,record.role(),&installed.model)}?;
    }
    task_record(pointer,true)?;
    Ok(pending.into_raw())
}
fn initialize_task_error_on_creator(pointer:*mut AnyObject,record:&Arc<TaskRecord>)->Result<()> {
    if record.owner!=std::thread::current().id() || record.role()!=c"NSError"
        || !Arc::ptr_eq(record,&task_record(pointer,true)?) {
        return Err("NSError initialization requires its actual creator record".into());
    }
    let TaskPayload::Error(fields)=&record.payload else {
        return Err("NSError initialization has no typed fields".into());
    };
    let identity=super::factory::receiver_identity(pointer)?;
    if !identity.belongs_to(&record.model)||identity.allocation_serial()!=record.serial.load(Ordering::Acquire) {
        return Err("NSError creator allocation changed".into());
    }
    let class=crate::sdk::metadata::registered_class(identity.class() as *const AnyClass as usize)?;
    let before=crate::sdk::metadata::method(class,c"code",false)?.ok_or("NSError selected code getter absent")?;
    if before.implementation()!=task_error_code as *const () as usize {
        return Err("NSError creator code getter is not its typed replacement".into());
    }
    let signature=task_factory_signatures().into_iter().find(|(role,selector,meta,_)|
        *role==c"NSError"&&*selector==c"code"&&!*meta).ok_or("NSError code signature absent")?;
    super::classes::verify_typed_abi(&before,&signature.3)?;
    let initializer=crate::sdk::metadata::method(class,c"initialize",true)?
        .ok_or("NSError selected class initializer absent")?;
    // Actual selected public API, not a direct IMP call or invented readiness
    // probe. Objective-C performs its normal parent/proxy +initialize dispatch.
    // No arena, receiver or model mutex spans that dispatch.
    let observed:isize=unsafe {objc2::msg_send![pointer,code]};
    if observed!=fields.code {return Err("NSError creator initialization returned different code".into());}
    if !Arc::ptr_eq(record,&task_record(pointer,true)?) {
        return Err("NSError creator record changed during initialization".into());
    }
    crate::sdk::metadata::verify_current_method(class,c"code",false,&before)?;
    crate::sdk::metadata::verify_current_method(class,c"initialize",true,&initializer)?;
    super::factory::verify_task_allocation_pure(pointer,record.serial.load(Ordering::Acquire),
        record.owner,c"NSError",&record.model)?;
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let objects=installed.objects.lock().map_err(|_| "object arena poisoned")?;
    if !matches!(objects.get(&(pointer as usize)),Some(Kind::TaskPayload(value)) if Arc::ptr_eq(&value.0,record)) {
        return Err("NSError creator arena record changed after initialization".into());
    }
    Ok(())
}
fn task_registry()->Result<&'static crate::sdk::process_abi::Context> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if installed.foundation_data_only || installed.apple_event_data_only {return Err("Task outside full arena".into());}
    let context=crate::sdk::process_abi::installed_context()?;
    if !Arc::ptr_eq(&context.life,&installed.model.lease().life) {return Err("Task guardian issuer differs".into());}
    context.registry.before_effect()?;
    Ok(context)
}
unsafe fn init_task_payload(pointer:*mut AnyObject,role:&CStr,make:impl FnOnce()->Result<TaskPayload>)->Result<*mut AnyObject> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    super::factory::claim_owned_role(pointer,role,&installed.model,owned_task_dealloc as *const () as usize)?;
    // Claim precedes adoption; failures retain their known +1 cleanup obligation.
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(pointer)}?;
    let payload=make()?;
    unsafe {attach_task_payload(pending.into_raw(),payload,true)}
}
unsafe fn init_task(pointer:*mut AnyObject)->Result<*mut AnyObject> {
    unsafe {init_task_payload(pointer,c"NSTask",|| {
        let context=task_registry()?;
        Ok(TaskPayload::Task(Arc::new(task_objects::Fields::new(crate::sdk::task_model::Task::new(context.life.clone())?))))
    })}
}
unsafe fn init_pipe(pointer:*mut AnyObject, parent_reader: bool)->Result<*mut AnyObject> {
    unsafe {init_task_payload(pointer,c"NSPipe",||Ok(TaskPayload::Pipe(TaskPipeRecord::new(
        crate::sdk::task_model::Pipe::with_parent_reader(&task_registry()?.registry, parent_reader)?))))}
}
unsafe fn init_handle(pointer:*mut AnyObject)->Result<*mut AnyObject> {
    unsafe {init_task_payload(pointer,c"NSFileHandle",||Ok(TaskPayload::Handle(crate::sdk::task_model::Handle::null(&task_registry()?.registry)?)))}
}
fn new_task_pipe(class:*mut AnyClass)->Result<*mut AnyObject> {
    let context=task_registry()?;
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if !super::factory::class_role_matches(class,c"NSPipe",&installed.model)? {return Err("Pipe class differs".into());}
    let payload=TaskPayload::Pipe(TaskPipeRecord::new(crate::sdk::task_model::Pipe::new(&context.registry)?));
    let pointer=super::factory::proxy(c"NSPipe",&installed.model)?;
    unsafe {attach_task_payload(pointer,payload,false)}
}
fn new_task_null_handle(class:*mut AnyClass)->Result<*mut AnyObject> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if !super::factory::class_role_matches(class,c"NSFileHandle",&installed.model)? {return Err("Handle class differs".into());}
    new_task_handle(crate::sdk::task_model::Handle::null(&task_registry()?.registry)?)
}
fn new_task_handle(value:crate::sdk::task_model::Handle)->Result<*mut AnyObject> {
    value.verify_registry(&task_registry()?.registry)?;
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let pointer=super::factory::proxy(c"NSFileHandle",&installed.model)?;
    unsafe {attach_task_payload(pointer,TaskPayload::Handle(value),false)}
}
fn new_task_error(failure:&crate::sdk::task_model::Failure)->Result<*mut AnyObject> {
    new_task_error_fields(failure.ns_error_fields()?)
}
fn validate_task_error_fields(fields:&TaskErrorValue,depth:usize)->Result<()> {
    if depth>8 || fields.domain.is_empty() || fields.domain.len()>16384 || fields.description.len()>65536
        || fields.user_info.len()>64 {return Err("NSError immutable field bounds differ".into());}
    for (key,value) in &fields.user_info {
        if key.len()>16384 {return Err("NSError info key exceeds bound".into());}
        match value {
            crate::sdk::task_model::ErrorInfo::Text(text) if text.len()<=65536=>(),
            crate::sdk::task_model::ErrorInfo::Text(_)=>return Err("NSError info text exceeds bound".into()),
            crate::sdk::task_model::ErrorInfo::Underlying(error)=>validate_task_error_fields(error,depth+1)?,
        }
    }
    Ok(())
}
fn new_task_error_fields(fields:TaskErrorValue)->Result<*mut AnyObject> {
    validate_task_error_fields(&fields,0)?;
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if installed.foundation_data_only || installed.apple_event_data_only {return Err("Task error outside full arena".into());}
    installed.model.with(|_|Ok(()))?;
    let pointer=super::factory::proxy(c"NSError",&installed.model)?;
    unsafe {attach_task_payload(pointer,TaskPayload::Error(fields),false)}
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_file_fixture_error() -> Result<*mut AnyObject> {
    new_task_error_fields(TaskErrorValue {
        domain: "RuycastOwnedSDK".into(),
        code: libc::EPERM as isize,
        description: "The owned file operation failed.".into(),
        user_info: Default::default(),
    })
}
fn task_error_fields(pointer:*mut AnyObject)->Result<TaskErrorValue> {
    match &task_record(pointer,true)?.payload {
        TaskPayload::Error(value)=>Ok(value.clone()),
        _=>Err("not an owned Task error".into()),
    }
}
fn task_error_text(pointer:*mut AnyObject,text:&str)->Result<*mut AnyObject> {
    let record=task_record(pointer,true)?;
    let result=new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(text.as_bytes())?)))?;
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(result)}?;
    if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Task error changed during string construction".into());}
    let returned=unsafe {autorelease_foundation(pending.into_raw())}?;
    if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Task error changed during string return".into());}
    Ok(returned)
}
unsafe extern "C" fn task_error_domain(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(),||task_error_text(pointer,&task_error_fields(pointer)?.domain))
}
unsafe extern "C" fn task_error_code(pointer:*mut AnyObject,_:Sel)->isize {
    window_boundary(0,||Ok(task_error_fields(pointer)?.code))
}
unsafe extern "C" fn task_error_localized_description(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(),|| task_error_text(pointer,&task_error_fields(pointer)?.description))
}
unsafe extern "C" fn task_error_user_info(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(),|| {
        let fields=task_error_fields(pointer)?;
        let record=task_record(pointer,true)?;
        let mut values=Vec::new();
        for (key,value) in fields.user_info {
            let key_pointer=new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(key.as_bytes())?)))?;
            let key_owner=unsafe {super::factory::OwnedReceiver::from_owned(key_pointer)}?;
            let key_reference=foundation_reference(key_pointer)?;
            drop(key_owner);
            let value_pointer=match value {
                crate::sdk::task_model::ErrorInfo::Text(text)=>new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(text.as_bytes())?)))?,
                crate::sdk::task_model::ErrorInfo::Underlying(error)=>new_task_error_fields(*error)?,
            };
            let value_owner=unsafe {super::factory::OwnedReceiver::from_owned(value_pointer)}?;
            let value_reference=foundation_reference(value_pointer)?;
            drop(value_owner);
            values.push((key_reference,value_reference));
        }
        let result=new_foundation_error_dictionary(values)?;
        let pending=unsafe {super::factory::OwnedReceiver::from_owned(result)}?;
        if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Task error changed during userInfo construction".into());}
        let returned=unsafe {autorelease_foundation(pending.into_raw())}?;
        if !Arc::ptr_eq(&record,&task_record(pointer,true)?) {return Err("Task error changed during userInfo return".into());}
        Ok(returned)
    })
}
fn task_factory_rows()->Vec<(&'static CStr,&'static CStr,bool,usize)> {
    let mut rows=task_objects::replacements();
    rows.extend([(c"NSError",c"domain",false,task_error_domain as *const () as usize),
        (c"NSError",c"code",false,task_error_code as *const () as usize),
        (c"NSError",c"userInfo",false,task_error_user_info as *const () as usize),
        (c"NSError",c"localizedDescription",false,task_error_localized_description as *const () as usize)]);
    rows
}
fn task_factory_signatures()->Vec<(&'static CStr,&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
    let mut rows=task_objects::signatures();
    for selector in [c"domain",c"userInfo",c"localizedDescription"] {
        rows.push((c"NSError",selector,false,window_signature::<unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject>()));
    }
    rows.push((c"NSError",c"code",false,window_signature::<unsafe extern "C" fn(*mut AnyObject,Sel)->isize>()));
    rows
}
unsafe extern "C" fn owned_task_dealloc(pointer:*mut AnyObject,_:Sel) {
    let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        super::factory::deallocate_owned(pointer,|| {
            let removed={
                let mut objects=INSTALLED.get().ok_or("workspace absent")?.objects.lock().map_err(|_| "object arena poisoned")?;
                match objects.get(&(pointer as usize)) {
                    Some(Kind::TaskPayload(_))=>objects.remove(&(pointer as usize)),
                    None=>None,
                    _=>return Err("Task deallocation payload differs".into()),
                }
            };
            if let Some(Kind::TaskPayload(value))=&removed {value.0.ready.store(false,Ordering::Release);}
            // Task child reference ARC and owned descriptors drop outside locks.
            drop(removed);
            Ok(())
        })
    }));
    if !matches!(outcome,Ok(Ok(()))) {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if !pointer.is_null() && outcome.as_ref().is_ok_and(|result|
            result.as_ref().is_err_and(|reason| reason == "unknown SDK allocation receiver"))
        {
            if let Some(class) = unsafe { objc2::ffi::object_getClass(pointer).as_ref() } {
                crate::sdk::private_network::report_reference_website_selector_refusal(class.name(), c"dealloc");
            }
        }
        if let Err(payload)=outcome {std::mem::forget(payload);}
        if let Some(installed)=INSTALLED.get() {installed.model.fault();}
    }
}
pub enum TaskMethodSource {
    Actual(crate::sdk::metadata::MethodSnapshot),
    OwnedWait(super::classes::TaskWaitDeclaration),
}
pub struct TaskMethodPreflight {
    pub role:&'static CStr,
    pub selector:&'static CStr,
    pub class_method:bool,
    pub source:TaskMethodSource,
    pub replacement:crate::sdk::metadata::CodeIdentity,
}
pub fn preflight_task_factories()->Result<Vec<TaskMethodPreflight>> {
    let signatures=task_factory_signatures();
    let mut seen=std::collections::BTreeSet::new();
    let mut methods=Vec::new();
    for (role,selector,meta,implementation) in task_factory_rows() {
        if !matches!(role.to_bytes(),b"NSTask"|b"NSPipe"|b"NSFileHandle"|b"NSError")
            || !seen.insert((role,selector,meta)) {return Err("Task method roster differs".into());}
        let class=crate::sdk::metadata::loaded_class(role)?;
        let source=crate::sdk::metadata::method(class,selector,meta)?;
        let matching=signatures.iter().filter(|(r,s,m,_)|*r==role&&*s==selector&&*m==meta).collect::<Vec<_>>();
        if matching.len()!=1 {return Err("Task typed signature roster differs".into());}
        let source=match source {
            Some(source)=>{super::classes::verify_typed_abi(&source,&matching[0].3)?;TaskMethodSource::Actual(source)},
            None if role==c"NSTask"&&selector==super::classes::TaskWaitDeclaration::SELECTOR&&!meta=>{
                if matching[0].3!=super::classes::TaskWaitDeclaration::signature()
                    ||implementation!=task_objects::wait_implementation() as *const () as usize {
                    return Err("Task wait typed declaration roster differs".into());
                }
                TaskMethodSource::OwnedWait(super::classes::TaskWaitDeclaration::capture(task_objects::wait_implementation())?)
            },
            None=>return Err(format!("Task source method absent: {role:?}/{selector:?}/{meta}")),
        };
        methods.push(TaskMethodPreflight {role,selector,class_method:meta,source,
            replacement:crate::sdk::metadata::code_identity(implementation)?});
    }
    Ok(methods)
}
/// # Safety
/// Exclusive source-qualified setup of the three actual typed source families.
/// No original Task/Pipe/Handle initializer, process or descriptor operation is
/// executed here. Complete per-class lifecycle/ARC and every leaf remain required.
pub unsafe fn prepare_task_factories_checked(model:Arc<Workspace>,prewrite:impl FnOnce()->Result<()>)->Result<()> {
    // Factory proof publication precedes install_workspace_checked's sole arena
    // publication. Preparation itself allocates no Task/Pipe/Handle instance.
    if let Some(installed)=INSTALLED.get() {
        if !Arc::ptr_eq(&installed.model,&model) || installed.foundation_data_only || installed.apple_event_data_only {
            return Err("Task factory arena/model differs".into());
        }
    }
    let preflight=preflight_task_factories()?;
    let families=[(c"NSTask",c"RuycastF1OwnedTaskSource"),(c"NSPipe",c"RuycastF1OwnedPipeSource"),
        (c"NSFileHandle",c"RuycastF1OwnedHandleSource"),(c"NSError",c"RuycastF1OwnedTaskErrorSource")];
    let parents=families.iter().map(|(role,_)| {
        let observed=crate::sdk::metadata::loaded_class(role)?;
        Ok(unsafe {&*(observed.address() as *const AnyClass)})
    }).collect::<Result<Vec<_>>>()?;
    for parent in &parents {super::factory::preflight_default_proxy_arc(parent)?;}
    // These Foundation objects are created on the original process worker.
    // Their mutable payload still belongs to its individual creating thread.
    let lease = workspace_factory_lease_with_owner(&model, None)?;
    let guard=unsafe {super::classes::ClassGuard::select_sdk_family_checked(&parents,lease,owned_task_dealloc,prewrite)}?;
    let signatures=task_factory_signatures();
    for ((role,name),parent) in families.into_iter().zip(parents) {
        let proxy=unsafe {guard.subclass_qualified(name,parent)}?;
        let mut bindings=guard.lifecycle_bindings_for(parent)?;
        for (_,selector,meta,implementation) in task_factory_rows().into_iter().filter(|(r,_,_,_)|*r==role) {
            let signature=signatures.iter().find(|(r,s,m,_)|*r==role&&*s==selector&&*m==meta).ok_or("Task signature absent")?;
            let observed=preflight.iter().find(|row|row.role==role&&row.selector==selector&&row.class_method==meta)
                .ok_or("Task preflight row disappeared")?;
            match &observed.source {
                TaskMethodSource::Actual(_)=>{
                    let leaf=if role==c"NSFileHandle"&&selector==c"init"&&!meta {
                        if implementation!=task_objects::handle_initializer() as *const () as usize {
                            return Err("Handle typed initializer roster differs".into());
                        }
                        unsafe {guard.overlay_handle_root_initializer(proxy,task_objects::handle_initializer())}
                    } else {
                        unsafe {guard.overlay_bound(if meta {parent}else{proxy},selector,meta,implementation,&signature.3)}
                    }.map_err(|error|format!("Task overlay {role:?}/{selector:?}/{meta}: {error}"))?;
                    bindings.push(super::receiver::LifecycleBinding::leaf(leaf));
                },
                TaskMethodSource::OwnedWait(declaration)=>{
                    if role!=c"NSTask"||selector!=super::classes::TaskWaitDeclaration::SELECTOR||meta {
                        return Err("Task owned wait proof moved to another row".into());
                    }
                    let addition=unsafe {guard.add_task_wait(proxy,declaration,task_objects::wait_implementation())}?;
                    bindings.push(super::receiver::LifecycleBinding::owned_task_wait(addition));
                },
            }
        }
        let grant=unsafe {super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(),proxy,bindings)}?;
        super::factory::publish_proxy(role,Arc::new(grant))?;
    }
    Ok(())
}
#[derive(Clone)]
struct WindowIdentity(Arc<WindowRecord>);
impl PartialEq for WindowIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.handle == other.0.handle && Arc::ptr_eq(&self.0.model, &other.0.model)
    }
}
impl Eq for WindowIdentity {}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowOrigin {
    Sdk,
    ReferencePalette,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    ReferenceSnippetChoice,
}

struct WindowRecord {
    model: Arc<Workspace>,
    handle: Handle,
    owner: std::thread::ThreadId,
    origin: WindowOrigin,
    serial: std::sync::atomic::AtomicU64,
    ready: AtomicBool,
    live: AtomicBool,
    references: Mutex<WindowReferences>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    palette_appearance: Mutex<PaletteAppearance>,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[derive(Default)]
struct PaletteAppearance {
    title: Option<String>,
    clear_background: bool,
    opaque: Option<bool>,
    shadow: Option<bool>,
    movable_by_background: Option<bool>,
    released_when_closed: Option<bool>,
    hides_on_deactivate: Option<bool>,
    level: Option<isize>,
    collection_behavior: Option<usize>,
}
#[derive(Default)]
struct WindowReferences {
    content: Option<Arc<WindowReference>>,
    responder: Option<Arc<WindowWeak>>,
    delegate: Option<Arc<WindowWeak>>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pending_palette_host: Option<PaletteHostAllocation>,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
struct PaletteHostAllocation {
    pointer: usize,
    class: &'static AnyClass,
    controller: usize,
    root: super::factory::RootDeallocator,
    lifecycle: Vec<(crate::sdk::metadata::ClassHandle, crate::sdk::metadata::LifecycleSnapshot)>,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl PaletteHostAllocation {
    fn verify(&self) -> Result<()> {
        self.root.verify_proxy_arc(self.class)?;
        for (class, expected) in &self.lifecycle {
            if &crate::sdk::metadata::lifecycle(*class)? != expected {
                return Err("palette host lifecycle changed".into());
            }
        }
        Ok(())
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn palette_view_method(class: &'static AnyClass, selector: &'static CStr) -> Result<usize> {
    use crate::sdk::metadata;
    let (offset, encoding) = match selector.to_bytes() {
        b"alloc" => (0xca6c, b"@16@0:8".as_slice()),
        b"initWithFrame:" => (0x4cc5c, b"@48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16".as_slice()),
        b"addSubview:" => (0x54280, b"v24@0:8@16".as_slice()),
        b"layoutSubtreeIfNeeded" => (0x7e6b0, b"v16@0:8".as_slice()),
        b"superview" => (0x4da18, b"@16@0:8".as_slice()),
        b"window" => (0x50750, b"@16@0:8".as_slice()),
        b"frame" => (0x66ff8, b"{CGRect={CGPoint=dd}{CGSize=dd}}16@0:8".as_slice()),
        b"setFrame:" => (0x71308, b"v48@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16".as_slice()),
        b"removeFromSuperview" => (0x739a8, b"v16@0:8".as_slice()),
        _ => return Err("unknown palette mount selector".into()),
    };
    let class_method = selector == c"alloc";
    let (owner, image, uuid) = if class_method {
        (NSObject::class().metaclass() as *const AnyClass as usize, b"/usr/lib/libobjc.A.dylib".as_slice(),
            [0x03, 0xbd, 0x9e, 0x32, 0xcf, 0x0a, 0x37, 0xb0, 0x89, 0x8a, 0x3c, 0xe8, 0xde, 0x06, 0xd8, 0x42])
    } else {
        (metadata::loaded_class(c"NSView")?.address(), b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".as_slice(),
            [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64, 0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46])
    };
    let handle = metadata::registered_class(class as *const AnyClass as usize)?;
    let method = metadata::method(handle, selector, class_method)?.ok_or("palette mount method absent")?;
    let code = method.code();
    if method.owner() != owner || method.encoding() != encoding
        || code.image() != image || code.offset() != offset || code.uuid() != uuid
    {
        return Err("palette mount method owner, code or ABI differs".into());
    }
    Ok(method.implementation())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn verify_palette_key_event_class(class: &'static AnyClass) -> Result<()> {
    use crate::sdk::metadata;
    let handle = metadata::loaded_class(c"NSEvent")?;
    if handle.address() != class as *const AnyClass as usize || class.superclass() != Some(NSObject::class()) {
        return Err("physical palette key event class differs".into());
    }
    super::factory::RootDeallocator::capture()?.verify_proxy_arc(class)?;
    for (selector, meta) in [
        (c"initialize", true), (c"resolveClassMethod:", true), (c"resolveInstanceMethod:", true),
        (c"dealloc", false), (c".cxx_destruct", false),
    ] {
        let method = metadata::method(handle, selector, meta)?.ok_or("physical key event lifecycle absent")?;
        super::receiver::LifecycleBinding::preserve_appkit_lifecycle(class, selector, method)?.verify()?;
    }
    if metadata::method(handle, c".cxx_construct", false)?.is_some() {
        return Err("physical key event gained a CXX constructor".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
const PALETTE_KEY_EVENT_FACTORY: &CStr =
    c"keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:";

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn palette_key_event_method(class: &'static AnyClass, selector: &'static CStr) -> Result<usize> {
    let (meta, offset, encoding) = if selector == PALETTE_KEY_EVENT_FACTORY {
        (true, 0x9924e8, b"@96@0:8Q16{CGPoint=dd}24Q40d48q56@64@72@80B88S92".as_slice())
    } else {
        match selector.to_bytes() {
            b"windowNumber" => (false, 0x15cd10, b"q16@0:8".as_slice()),
            b"window" => (false, 0x15c280, b"@16@0:8".as_slice()),
            _ => return Err("unknown physical palette event method".into()),
        }
    };
    let owner = if meta { class.metaclass() } else { class };
    let handle = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
    let method = crate::sdk::metadata::method(handle, selector, meta)?.ok_or("physical key event method absent")?;
    if method.owner() != owner as *const AnyClass as usize || method.encoding() != encoding
        || method.code().image() != b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit"
        || method.code().offset() != offset
        || method.code().uuid() != [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64,
            0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46]
    {
        return Err("physical key event method owner, ABI or code differs".into());
    }
    Ok(method.implementation())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_palette_key_fallback(panel: *mut AnyObject, event: *mut AnyObject) -> Result<Bool> {
    use crate::sdk::metadata;
    let record = window_identity(panel)?;
    if record.origin != WindowOrigin::ReferencePalette { return Err("palette key receiver is not original".into()); }
    let event = event_handle(event)?;
    if !Arc::ptr_eq(&event.record.model, &record.model) {
        return Err("palette key event belongs to another workspace".into());
    }
    let data = event.data()?;
    if data.kind != 10 || data.window.is_some_and(|window| window != record.handle) {
        return Err("palette key event is outside this window".into());
    }
    let EventDetails::Key { characters, unmodified, repeat, key_code } = &data.details else {
        return Err("palette event has no key payload".into());
    };
    let characters = NSString::from_str(&String::from_utf16(characters).map_err(|_| "invalid palette key text")?);
    let unmodified = NSString::from_str(&String::from_utf16(unmodified).map_err(|_| "invalid palette unmodified key text")?);
    let content = record.references.lock().map_err(|_| "window references poisoned")?
        .content.clone().ok_or("palette key host absent")?;
    let WindowReference::Palette(host) = content.as_ref() else {
        return Err("palette key host owner differs".into());
    };
    let _dispatch = host.dispatching_key.try_lock().map_err(|_| "palette key dispatch already in progress")?;
    let (_, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
    let host_pointer = host.verify()?;
    verify_reference_mounted_palette_host(&record.model, host_pointer as usize, stopped)?;
    let method = metadata::method(metadata::registered_class(host.class as *const AnyClass as usize)?,
        c"performKeyEquivalent:", false)?.ok_or("SwiftUI host key method absent")?;
    let owner = host.class.superclass().ok_or("SwiftUI key host parent absent")?;
    if method.owner() != owner as *const AnyClass as usize || method.encoding() != b"B24@0:8@16"
        || method.code().image() != b"/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI"
        || method.code().offset() != 0x88373c
        || method.code().uuid() != [0x53, 0xe7, 0x98, 0x75, 0x60, 0xe4, 0x3b, 0x8d,
            0xba, 0x48, 0x02, 0xf6, 0xa4, 0xcb, 0xc4, 0x29]
    {
        return Err("SwiftUI host key method differs".into());
    }
    let event_class = metadata::loaded_class(c"NSEvent")?;
    let class = unsafe { &*(event_class.address() as *const AnyClass) };
    verify_palette_key_event_class(class)?;
    type Create = unsafe extern "C-unwind" fn(*const AnyClass, Sel, usize, NSPoint, usize, f64,
        isize, *mut AnyObject, *const NSString, *const NSString, Bool, u16) -> *mut AnyObject;
    let create: Create = unsafe { std::mem::transmute(palette_key_event_method(class, PALETTE_KEY_EVENT_FACTORY)?) };
    let perform: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool =
        unsafe { std::mem::transmute(method.implementation()) };
    let host_class = metadata::registered_class(host.class as *const AnyClass as usize)?;
    let answer = objc2::rc::autoreleasepool(|_| -> Result<Bool> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Bool> {
            // Preserve the key payload without attaching the platform event
            // to any real window or sending it to the operating system.
            if stopped.load(Ordering::Acquire) { return Err("palette key creation stopped".into()); }
            let native = unsafe { create(class, Sel::register(PALETTE_KEY_EVENT_FACTORY), data.kind,
                NSPoint::new(data.location.x, data.location.y), data.modifiers, data.timestamp,
                0, std::ptr::null_mut(), &*characters, &*unmodified, Bool::new(*repeat), *key_code) };
            if native.is_null() || unsafe { &*native }.class() != class {
                return Err("physical palette key event differs".into());
            }
            let window_number: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> isize =
                unsafe { std::mem::transmute(palette_key_event_method(class, c"windowNumber")?) };
            let window: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject =
                unsafe { std::mem::transmute(palette_key_event_method(class, c"window")?) };
            if unsafe { window_number(native, objc2::sel!(windowNumber)) } != 0
                || !unsafe { window(native, objc2::sel!(window)) }.is_null()
            {
                return Err("physical palette key event attached to a window".into());
            }
            event.check()?;
            verify_reference_mounted_palette_host(&record.model, host_pointer as usize, stopped)?;
            host.verify()?;
            verify_palette_key_event_class(class)?;
            metadata::verify_current_method(host_class, c"performKeyEquivalent:", false, &method)?;
            event.check_pure()?;
            if stopped.load(Ordering::Acquire) { return Err("palette key dispatch stopped".into()); }
            let answer = unsafe { perform(host_pointer, objc2::sel!(performKeyEquivalent:), native) };
            verify_reference_mounted_palette_host(&record.model, host_pointer as usize, stopped)?;
            event.check()?;
            verify_palette_key_event_class(class)?;
            if stopped.load(Ordering::Acquire) { return Err("palette key dispatch stopped after effect".into()); }
            Ok(answer)
        }));
        match result {
            Ok(Ok(answer)) => Ok(answer),
            // No unqualified native event may be released by pool drain.
            Ok(Err(_)) | Err(_) => unsafe { libc::_exit(95) },
        }
    })?;
    event.check()?;
    verify_reference_mounted_palette_host(&record.model, host_pointer as usize, stopped)?;
    event.check_pure()?;
    Ok(answer)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
struct WindowPaletteHost {
    host: usize,
    class: &'static AnyClass,
    container: usize,
    container_class: &'static AnyClass,
    owner: std::thread::ThreadId,
    model: Arc<Workspace>,
    root: super::factory::RootDeallocator,
    lifecycle: Vec<(crate::sdk::metadata::ClassHandle, crate::sdk::metadata::LifecycleSnapshot)>,
    resizing: Mutex<()>,
    dispatching_key: Mutex<()>,
    retirement_attempted: bool,
    released: Arc<AtomicBool>,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl WindowPaletteHost {
    fn verify_lifecycle(&self) -> Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("palette host on foreign owner".into());
        }
        self.root.verify_proxy_arc(self.class)?;
        self.root.verify_proxy_arc(self.container_class)?;
        for (class, expected) in &self.lifecycle {
            if &crate::sdk::metadata::lifecycle(*class)? != expected {
                return Err("palette host lifecycle changed".into());
            }
        }
        Ok(())
    }

    fn verify(&self) -> Result<*mut AnyObject> {
        self.verify_lifecycle()?;
        let host = self.host as *mut AnyObject;
        if unsafe { &*host }.class() != self.class
            || unsafe { &*(self.container as *const AnyObject) }.class() != self.container_class
        {
            return Err("palette host or container physical class changed".into());
        }
        Ok(host)
    }

    fn view_relationship(&self, host: *mut AnyObject, selector: &'static CStr) -> Result<*mut AnyObject> {
        let call: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject =
            unsafe { std::mem::transmute(palette_view_method(self.class, selector)?) };
        Ok(unsafe { call(host, Sel::register(selector)) })
    }

    fn attach(&self, bounds: NSRect, stopped: &AtomicBool) -> Result<()> {
        let host = self.verify()?;
        let container = self.container as *mut AnyObject;
        let before = self.view_relationship(host, c"superview")?;
        let window = self.view_relationship(host, c"window")?;
        if !before.is_null() || !window.is_null() {
            return Err("palette host already mounted".into());
        }
        let add: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) =
            unsafe { std::mem::transmute(palette_view_method(self.container_class, c"addSubview:")?) };
        if stopped.load(Ordering::Acquire) { return Err("palette host mount stopped".into()); }
        unsafe { add(container, objc2::sel!(addSubview:), host) };
        let parent = self.view_relationship(host, c"superview")?;
        let window = self.view_relationship(host, c"window")?;
        if parent != container || !window.is_null() {
            return Err("palette host detached mount differs".into());
        }
        self.layout(bounds, stopped)
    }

    fn layout(&self, bounds: NSRect, stopped: &AtomicBool) -> Result<()> {
        let host = self.verify()?;
        let container = self.container as *mut AnyObject;
        for (pointer, class) in [(container, self.container_class), (host, self.class)] {
            let set_frame: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect) =
                unsafe { std::mem::transmute(palette_view_method(class, c"setFrame:")?) };
            if stopped.load(Ordering::Acquire) { return Err("palette host frame stopped".into()); }
            unsafe { set_frame(pointer, objc2::sel!(setFrame:), bounds) };
            self.verify()?;
        }
        let layout: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(palette_view_method(self.container_class, c"layoutSubtreeIfNeeded")?) };
        if stopped.load(Ordering::Acquire) { return Err("palette host layout stopped".into()); }
        unsafe { layout(container, objc2::sel!(layoutSubtreeIfNeeded)) };
        self.verify()?;
        let parent = self.view_relationship(host, c"superview")?;
        let window = self.view_relationship(host, c"window")?;
        if parent != container || !window.is_null() {
            return Err("palette host detached layout changed its mount".into());
        }
        for (pointer, class) in [(container, self.container_class), (host, self.class)] {
            let frame: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> NSRect =
                unsafe { std::mem::transmute(palette_view_method(class, c"frame")?) };
            if unsafe { frame(pointer, objc2::sel!(frame)) } != bounds {
                return Err("palette view frame differs after layout".into());
            }
        }
        Ok(())
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl WindowPaletteHost {
    fn retire(&mut self) -> Result<()> {
        if std::mem::replace(&mut self.retirement_attempted, true) {
            return Err("palette host retirement already attempted".into());
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            let host = self.verify()?;
            let container = self.container as *mut AnyObject;
            let parent = self.view_relationship(host, c"superview")?;
            if parent == container {
                let remove: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) =
                    unsafe { std::mem::transmute(palette_view_method(self.class, c"removeFromSuperview")?) };
                unsafe { remove(host, objc2::sel!(removeFromSuperview)) };
                let parent = self.view_relationship(host, c"superview")?;
                if !parent.is_null() { return Err("palette host did not detach".into()); }
            } else if !parent.is_null() {
                return Err("palette host moved to a foreign view".into());
            }
            self.verify()?;
            drop(unsafe { Retained::from_raw(host) });
            self.verify_lifecycle()?;
            if unsafe { &*container }.class() != self.container_class {
                return Err("palette mount container changed during host release".into());
            }
            drop(unsafe { Retained::from_raw(container) });
            self.released.store(true, Ordering::Release);
            Ok(())
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                std::mem::forget(payload);
                Err("palette host retirement panicked".into())
            }
        };
        result.inspect_err(|_| self.model.fault())
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl Drop for WindowPaletteHost {
    fn drop(&mut self) {
        if !self.retirement_attempted {
            let _ = self.retire();
        }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn retire_reference_palette_host(model: &Arc<Workspace>) -> Result<()> {
    if unsafe { libc::pthread_main_np() } != 1 {
        return Err("palette host retirement off main thread".into());
    }
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(model, &installed.model) {
        return Err("palette host retirement workspace differs".into());
    }
    let released = installed.palette_host_release.get().ok_or("no original palette host was published")?;
    let panels = installed.objects.lock().map_err(|_| "object arena poisoned")?.values()
        .filter_map(|kind| match kind {
            Kind::WindowPayload(identity) if identity.0.origin == WindowOrigin::ReferencePalette => Some(identity.0.clone()),
            _ => None,
        }).collect::<Vec<_>>();
    for panel in panels {
        let host = {
            let mut references = panel.references.lock().map_err(|_| "window references poisoned")?;
            if references.pending_palette_host.is_some() {
                return Err("palette host allocation handoff still pending at termination".into());
            }
            let Some(content) = references.content.as_ref() else { continue; };
            if !matches!(content.as_ref(), WindowReference::Palette(host) if Arc::ptr_eq(&host.released, released)) {
                return Err("terminal PalettePanel content differs".into());
            }
            let content = references.content.take().ok_or("terminal palette content disappeared")?;
            match Arc::try_unwrap(content) {
                Ok(WindowReference::Palette(host)) => host,
                Ok(other) => {
                    references.content = Some(Arc::new(other));
                    return Err("terminal PalettePanel content differs".into());
                }
                Err(content) => {
                    references.content = Some(content);
                    return Err("terminal palette host still has a live borrower".into());
                }
            }
        };
        let mut host = host;
        host.retire()?;
    }
    if !released.load(Ordering::Acquire) {
        return Err("palette host SDK references were not released".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_snippet_monitor(
    admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    controller: usize,
    token: usize,
) -> Result<()> {
    event_methods::verify_snippet_monitor(admission, controller, token)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_snippet_windows_drained(model: &Arc<Workspace>) -> Result<()> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if unsafe { libc::pthread_main_np() } != 1 || !Arc::ptr_eq(model, &installed.model) {
        return Err("SnippetChoice drain owner differs".into());
    }
    event_methods::verify_snippet_monitors_drained()?;
    let objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
    if model.faulted.load(Ordering::Acquire) || objects.values().any(|object| {
        matches!(object, Kind::WindowPayload(record) if record.0.origin == WindowOrigin::ReferenceSnippetChoice)
    }) {
        return Err("SnippetChoice window cleanup incomplete".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) unsafe fn allocate_reference_palette_host(
    panel: *mut AnyObject, class: &'static AnyClass, controller: usize, snippet: bool, stopped: &AtomicBool,
    verify_source: impl FnOnce() -> Result<()>,
) -> Result<*mut AnyObject> {
    let record = window_identity(panel)?;
    let expected = if snippet { WindowOrigin::ReferenceSnippetChoice } else { WindowOrigin::ReferencePalette };
    if record.origin != expected || controller == 0 {
        return Err("Reference palette host does not belong to an original panel".into());
    }
    let root = super::factory::RootDeallocator::capture()?;
    root.verify_proxy_arc(class)?;
    let mut lifecycle = Vec::new();
    let mut ancestor = Some(class);
    for _ in 0..16 {
        let Some(current) = ancestor else { break; };
        if current == NSObject::class() { break; }
        let handle = crate::sdk::metadata::registered_class(current as *const AnyClass as usize)?;
        if [c"NSView", c"NSResponder"].contains(&current.name()) {
            for (selector, class_method) in [(c"initialize", true), (c"dealloc", false)] {
                let snapshot = crate::sdk::metadata::method(handle, selector, class_method)?
                    .ok_or("original AppKit view lifecycle absent")?;
                super::receiver::LifecycleBinding::preserve_appkit_lifecycle(current, selector, snapshot)?.verify()?;
            }
            if current.name() == c"NSView" {
                let cxx = crate::sdk::metadata::method(handle, c".cxx_destruct", false)?
                    .ok_or("original AppKit view destructor absent")?;
                super::receiver::LifecycleBinding::preserve_appkit_lifecycle(current, c".cxx_destruct", cxx)?.verify()?;
            }
        }
        lifecycle.push((handle, crate::sdk::metadata::lifecycle(handle)?));
        ancestor = current.superclass();
    }
    if ancestor != Some(NSObject::class()) {
        return Err("palette host root ancestry differs".into());
    }
    {
        let references = record.references.lock().map_err(|_| "window references poisoned")?;
        if references.pending_palette_host.is_some() || references.content.is_some() {
            return Err("Reference palette host allocation repeated".into());
        }
    }
    if stopped.load(Ordering::Acquire) { return Err("palette host allocation stopped".into()); }
    let pointer = unsafe { objc2::ffi::objc_allocWithZone(class) };
    if pointer.is_null() || unsafe { &*pointer }.class() != class {
        return Err("Reference palette host allocation class differs".into());
    }
    // Swift consumes the sole +1 in the original initializer. A failed
    // handoff is terminal at the import boundary; do not release an
    // uninitialized framework object through its initialized cleanup path.
    let allocation = PaletteHostAllocation { pointer: pointer as usize, class, controller, root, lifecycle };
    record.check(panel)?;
    verify_source()?;
    allocation.verify()?;
    if unsafe { &*pointer }.class() != class {
        return Err("Reference palette host class changed during qualification".into());
    }
    let mut references = record.references.lock().map_err(|_| "window references poisoned")?;
    if stopped.load(Ordering::Acquire) || references.pending_palette_host.is_some() || references.content.is_some() {
        return Err("Reference palette host allocation repeated".into());
    }
    references.pending_palette_host = Some(allocation);
    Ok(pointer)
}
impl WindowRecord {
    fn check(&self, pointer: *mut AnyObject) -> Result<()> {
        if self.owner != std::thread::current().id() || !self.live.load(Ordering::Acquire)
            || !self.ready.load(Ordering::Acquire) {
            return Err("window payload unready, retired or foreign owner".into());
        }
        self.model.with(|_| Ok(()))?;
        let serial = if self.origin != WindowOrigin::Sdk {
            let identity = super::factory::receiver_identity(pointer)?;
            if !identity.original_source() || !identity.belongs_to(&self.model) {
                return Err("original window receiver issuer differs".into());
            }
            identity.verify_pure()?;
            identity.allocation_serial()
        } else {
            super::factory::owned_reference_serial(pointer)?
        };
        if serial == 0 || serial != self.serial.load(Ordering::Acquire) {
            return Err("window allocation serial changed".into());
        }
        self.model.window_snapshot(self.handle)?;
        Ok(())
    }
    fn retire(&self) -> Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("window cleanup on foreign owner".into());
        }
        if self.live.swap(false, Ordering::AcqRel) {
            self.ready.store(false, Ordering::Release);
            self.model.retire_window_allocation(self.handle)?;
        }
        Ok(())
    }
}
impl Drop for WindowRecord {
    fn drop(&mut self) {
        if self.retire().is_err() { self.model.fault(); }
    }
}
fn window_identity(pointer: *mut AnyObject) -> Result<Arc<WindowRecord>> {
    let record = {
        let objects = INSTALLED.get().ok_or("workspace absent")?.objects
            .lock().map_err(|_| "object arena poisoned")?;
        match objects.get(&(pointer as usize)) {
            Some(Kind::WindowPayload(identity)) => identity.0.clone(),
            _ => return Err("not an owned window payload".into()),
        }
    };
    record.check(pointer)?;
    Ok(record)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_mounted_palette_host(
    model: &Arc<Workspace>, pointer: usize, stopped: &AtomicBool,
) -> Result<()> {
    reference_mounted_palette_host(model, pointer, stopped).map(|_| ())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn reference_mounted_palette_host(
    model: &Arc<Workspace>, pointer: usize, stopped: &AtomicBool,
) -> Result<Arc<WindowReference>> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(model, &installed.model) || pointer == 0 || stopped.load(Ordering::Acquire) {
        return Err("field host owner differs from workspace".into());
    }
    let panels = installed.objects.lock().map_err(|_| "object arena poisoned")?.iter()
        .filter_map(|(&receiver, kind)| match kind {
            Kind::WindowPayload(identity) if identity.0.origin == WindowOrigin::ReferencePalette => Some((receiver, identity.0.clone())),
            _ => None,
        }).collect::<Vec<_>>();
    let mut selected = None;
    for (receiver, record) in panels {
        let content = record.references.lock().map_err(|_| "window references poisoned")?.content.clone();
        if let Some(content) = content {
            if matches!(content.as_ref(), WindowReference::Palette(host) if host.host == pointer)
                && selected.replace((receiver, record, content)).is_some()
            {
                return Err("field host is registered to multiple original panels".into());
            }
        }
    }
    let (receiver, record, content) = selected.ok_or("field host is not mounted on an original panel")?;
    record.check(receiver as *mut AnyObject)?;
    let WindowReference::Palette(host) = content.as_ref() else {
        return Err("field host is not original Palette content".into());
    };
    let host_pointer = host.verify()?;
    if host.view_relationship(host_pointer, c"superview")? as usize != host.container
        || !host.view_relationship(host_pointer, c"window")?.is_null()
    {
        return Err("field host is no longer in its detached container".into());
    }
    host.verify()?;
    let still_registered = installed.objects.lock().map_err(|_| "object arena poisoned")?
        .get(&receiver).is_some_and(|kind| matches!(kind, Kind::WindowPayload(identity) if Arc::ptr_eq(&identity.0, &record)));
    let references = record.references.lock().map_err(|_| "window references poisoned")?;
    if !still_registered || stopped.load(Ordering::Acquire)
        || !record.live.load(Ordering::Acquire) || !record.ready.load(Ordering::Acquire)
        || !references.content.as_ref().is_some_and(|current| Arc::ptr_eq(current, &content))
    {
        return Err("original field host retired during qualification".into());
    }
    Ok(content)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn layout_reference_field_host(
    model: &Arc<Workspace>, pointer: usize, stopped: &AtomicBool,
) -> Result<()> {
    let content = reference_mounted_palette_host(model, pointer, stopped)?;
    let WindowReference::Palette(host) = content.as_ref() else {
        return Err("field layout requires original Palette content".into());
    };
    let _view_update = host.resizing.try_lock().map_err(|_| "palette view update already in progress")?;
    // The original InteractionPerformanceChecks sizes its offscreen view to
    // this exact viewport. The private container must match that view.
    let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(760.0, 490.0));
    host.layout(bounds, stopped)?;
    verify_reference_mounted_palette_host(model, pointer, stopped)
}
fn window_argument(pointer: *mut AnyObject, role: &CStr) -> Result<super::receiver::ReceiverIdentity> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    let identity = super::factory::receiver_identity(pointer)?;
    if !identity.belongs_to(&installed.model) { return Err("foreign window argument".into()); }
    identity.verify()?;
    let expected = crate::sdk::metadata::loaded_class(role)?;
    let mut ancestor = Some(identity.class());
    let mut seen = std::collections::BTreeSet::new();
    while let Some(class) = ancestor {
        if !seen.insert(class as *const AnyClass as usize) || seen.len() > 256 {
            return Err("window argument ancestry differs".into());
        }
        if class as *const AnyClass as usize == expected.address() { return Ok(identity); }
        ancestor = class.superclass();
    }
    Err("window argument has wrong qualified type".into())
}
enum WindowReference {
    Sdk(super::factory::RetainedReference),
    Original(WindowOriginalReference),
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    Palette(WindowPaletteHost),
}
struct WindowOriginalReference {
    pointer: usize,
    identity: super::receiver::ReceiverIdentity,
    catalog: Arc<dyn application::CallbackCatalog>,
    model: Arc<Workspace>,
    owner: std::thread::ThreadId,
    returned: bool,
}
impl WindowOriginalReference {
    fn cleanup(&self) -> Result<()> {
        if self.owner != std::thread::current().id() { return Err("window reference on foreign owner".into()); }
        self.identity.verify_allocation()?;
        self.catalog.verify_cleanup(self.identity.class(), self.identity.source())?;
        self.identity.verify_allocation()?;
        self.catalog.verify_receiver_cleanup(&self.identity, self.identity.source())?;
        self.identity.verify_allocation()?;
        if self.pointer != self.identity.receiver()
            || unsafe { &*(self.pointer as *mut AnyObject) }.class() != self.identity.class() {
            return Err("window reference physical identity changed".into());
        }
        Ok(())
    }
}
impl Drop for WindowOriginalReference {
    fn drop(&mut self) {
        if self.returned { return; }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            self.cleanup()?;
            drop(unsafe { Retained::from_raw(self.pointer as *mut AnyObject) });
            Ok(())
        })) {
            Ok(Ok(())) => (),
            other => {
                if let Err(payload) = other { std::mem::forget(payload); }
                self.model.fault();
                // The known +1 is quarantined; no unqualified ARC destructor.
            }
        }
    }
}
impl WindowReference {
    fn acquire(pointer: *mut AnyObject, role: &CStr) -> Result<Self> {
        let identity = window_argument(pointer, role)?;
        let installed = INSTALLED.get().ok_or("workspace absent")?;
        if !identity.original_source() {
            return Ok(Self::Sdk(super::factory::RetainedReference::acquire(pointer, &installed.model)?));
        }
        if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            return Err("original window reference before complete admission".into());
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if identity.class().name() == c"RuycastF1Reference.SnippetChoicePanel" {
            let record = window_identity(pointer)?;
            if role != c"NSWindow" || record.origin != WindowOrigin::ReferenceSnippetChoice
                || identity.source() != &super::factory::reference_snippet_panel_construction_callsite()
            {
                return Err("SnippetChoice window reference lacks its original panel grant".into());
            }
            // This original panel has a typed deallocator and uses the same
            // root ARC and allocation serial as the modeled SDK windows.
            let held = super::factory::RetainedReference::acquire(pointer, &record.model)?;
            record.check(pointer)?;
            return Ok(Self::Sdk(held));
        }
        let catalog = installed.assembly.get().ok_or("window callback catalog absent")?.catalog.clone();
        catalog.verify_cleanup(identity.class(), identity.source())?;
        identity.verify_allocation()?;
        catalog.verify_receiver_cleanup(&identity, identity.source())?;
        identity.verify_allocation()?;
        let held = unsafe { Retained::retain(pointer) }.ok_or("window content retain failed")?;
        let owned = WindowOriginalReference { pointer: Retained::into_raw(held) as usize,
            identity, catalog, model: installed.model.clone(), owner: std::thread::current().id(), returned: false };
        owned.cleanup()?;
        owned.identity.verify()?;
        Ok(Self::Original(owned))
    }
    fn pointer(&self) -> Result<*mut AnyObject> {
        match self {
            Self::Sdk(held) => held.pointer(),
            Self::Original(held) => { held.cleanup()?; held.identity.verify()?; Ok(held.pointer as *mut AnyObject) }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Self::Palette(held) => held.verify(),
        }
    }
    fn invoke_bool(&self, selector: &'static CStr) -> Result<bool> {
        let pointer = self.pointer()?;
        let identity = super::factory::receiver_identity(pointer)?;
        let handle = crate::sdk::metadata::registered_class(identity.class() as *const AnyClass as usize)?;
        let before = crate::sdk::metadata::method(handle, selector, false)?
            .ok_or("responder callback method missing")?;
        super::classes::verify_typed_abi(&before,
            &window_signature::<unsafe extern "C" fn(*mut AnyObject,Sel)->Bool>())?;
        match self {
            Self::Sdk(_) => {
                let leaf = super::factory::attested_owned_method(pointer,selector)?;
                if leaf.replacement != before.implementation() || leaf.provenance.verify()? != leaf.replacement {
                    return Err("responder callback leaf changed".into());
                }
            }
            Self::Original(held) => {
                if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
                    return Err("original responder callback before admission".into());
                }
                let (source, methods) = held.catalog.class_binding(identity.class())?;
                if source != *identity.source()
                    || methods.iter().filter(|(name,imp)|*name==selector&&*imp==before.implementation()).count()!=1 {
                    return Err("responder callback source roster differs".into());
                }
                held.catalog.verify_method(identity.class(),Sel::register(selector),before.implementation(),&source)?;
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Self::Palette(_) => return Err("palette host does not own responder callbacks".into()),
        }
        if crate::sdk::metadata::method(handle,selector,false)?.as_ref()!=Some(&before) {
            return Err("responder method changed after qualification".into());
        }
        identity.verify_pure()?;
        let call: unsafe extern "C-unwind" fn(*mut AnyObject,Sel)->Bool =
            unsafe { std::mem::transmute(before.implementation()) };
        let answer = unsafe { call(pointer,Sel::register(selector)) }.as_bool();
        identity.verify_pure()?;
        if crate::sdk::metadata::method(handle,selector,false)?.as_ref()!=Some(&before) {
            return Err("responder callback metadata changed during invocation".into());
        }
        Ok(answer)
    }
    fn returned(&self, role: &CStr) -> Result<*mut AnyObject> {
        let pointer = self.pointer()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if matches!(self, Self::Palette(_)) {
            if role != c"NSView" { return Err("palette host can only return as NSView".into()); }
            let retained = unsafe { Retained::retain(pointer) }.ok_or("palette host retain failed")?;
            return Ok(Retained::autorelease_return(retained));
        }
        match Self::acquire(pointer, role)? {
            Self::Sdk(held) => {
                let pointer = held.pointer()?;
                let retained = unsafe { super::factory::retain_owned(pointer) }?;
                let pending = unsafe { super::factory::OwnedReceiver::from_owned(retained) }?;
                let result = unsafe { super::factory::autorelease_owned(retained) }?;
                pending.into_raw();
                Ok(result)
            }
            Self::Original(mut held) => {
                held.cleanup()?;
                held.returned = true;
                let retained = unsafe { Retained::from_raw(held.pointer as *mut AnyObject) }
                    .ok_or("window reference disappeared")?;
                Ok(Retained::autorelease_return(retained))
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Self::Palette(_) => Err("unqualified palette host acquisition".into()),
        }
    }
}
// The integer below owns a Rust Box<Weak>, not evidence for an ObjC receiver.
// Its immutable ReceiverIdentity is the object proof. The Box is accessed and
// destroyed only on its creator thread; off-thread destruction quarantines it.
struct WindowWeak {
    slot: usize,
    identity: super::receiver::ReceiverIdentity,
    owner: std::thread::ThreadId,
    model: Arc<Workspace>,
    role: &'static CStr,
}
impl WindowWeak {
    fn new(pointer: *mut AnyObject, role: &'static CStr) -> Result<Self> {
        let pin = WindowReference::acquire(pointer, role)?;
        let identity = window_argument(pointer, role)?;
        let value = objc2::rc::Weak::new(unsafe { &*pin.pointer()? });
        identity.verify()?;
        Ok(Self { slot: Box::into_raw(Box::new(value)) as usize, identity,
            owner: std::thread::current().id(), model: INSTALLED.get().ok_or("workspace absent")?.model.clone(), role })
    }
    fn returned(&self) -> Result<*mut AnyObject> {
        if self.owner != std::thread::current().id() { return Err("window weak reference on foreign owner".into()); }
        if !self.identity.allocation_is_live()? { return Ok(std::ptr::null_mut()); }
        self.identity.verify()?;
        if self.identity.original_source() {
            if crate::sdk::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
                return Err("original weak window reference before admission".into());
            }
            let catalog = &INSTALLED.get().ok_or("workspace absent")?.assembly.get()
                .ok_or("window catalog absent")?.catalog;
            catalog.verify_cleanup(self.identity.class(), self.identity.source())?;
            catalog.verify_receiver_cleanup(&self.identity, self.identity.source())?;
            self.identity.verify_allocation()?;
        } else { self.identity.verify_sdk_cleanup()?; }
        let value = unsafe { &*(self.slot as *const objc2::rc::Weak<AnyObject>) }.load();
        let Some(value) = value else { return Ok(std::ptr::null_mut()); };
        let pointer = Retained::into_raw(value);
        if pointer as usize != self.identity.receiver() || self.identity.verify_allocation().is_err() {
            self.model.fault();
            return Err("window weak load allocation changed; +1 quarantined".into());
        }
        // A separate qualified strong pin keeps the getter result alive through
        // its +0 return. Consume the weak-load +1 through its exact cleanup path.
        let result = WindowReference::acquire(pointer, self.role);
        if self.identity.original_source() {
            let catalog = INSTALLED.get().ok_or("workspace absent")?.assembly.get()
                .ok_or("window catalog absent")?.catalog.clone();
            let loaded = WindowOriginalReference { pointer: pointer as usize, identity: self.identity.clone(),
                catalog, model: self.model.clone(), owner: self.owner, returned: false };
            loaded.cleanup()?;
            drop(loaded);
        } else {
            drop(unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?);
        }
        result?.returned(self.role)
    }
}
impl Drop for WindowWeak {
    fn drop(&mut self) {
        if self.owner == std::thread::current().id() {
            drop(unsafe { Box::from_raw(self.slot as *mut objc2::rc::Weak<AnyObject>) });
        } else { self.model.fault(); }
    }
}
#[derive(Clone)]
struct DescriptorLife(Arc<AtomicBool>);
impl PartialEq for DescriptorLife {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for DescriptorLife {}
/// Pure immutable event data, never an Objective-C receiver or authority.
#[derive(Clone, Debug)]
pub struct EventData {
    pub kind: usize,
    pub location: Point,
    pub modifiers: usize,
    pub timestamp: f64,
    pub window: Option<Handle>,
    pub details: EventDetails,
}
#[derive(Clone, Debug)]
pub enum EventDetails {
    Key {
        characters: Vec<u16>,
        unmodified: Vec<u16>,
        repeat: bool,
        key_code: u16,
    },
    Mouse {
        number: isize,
        click_count: isize,
        pressure: f32,
    },
    Other {
        subtype: i16,
        data1: isize,
        data2: isize,
    },
}
impl EventData {
    pub fn validate(&self) -> Result<()> {
        if !self.location.x.is_finite()
            || !self.location.y.is_finite()
            || !self.timestamp.is_finite()
            || self.timestamp < 0.0
        {
            return Err("invalid event geometry or timestamp".into());
        }
        match &self.details {
            EventDetails::Key {
                characters,
                unmodified,
                ..
            } if matches!(self.kind, 10..=12)
                && characters.len() <= 8192
                && unmodified.len() <= 8192 =>
            {
                Ok(())
            }
            EventDetails::Mouse {
                number,
                click_count,
                pressure,
            } if matches!(self.kind, 1..=9)
                && *number >= 0
                && *click_count >= 0
                && pressure.is_finite()
                && (0.0..=1.0).contains(pressure) =>
            {
                Ok(())
            }
            EventDetails::Other { .. } if matches!(self.kind, 13..=17) => Ok(()),
            _ => Err("unsupported event kind or payload combination".into()),
        }
    }
}
struct EventRecord {
    model: Arc<Workspace>,
    handle: Handle,
    owner: std::thread::ThreadId,
    life: DescriptorLife,
    serial: std::sync::atomic::AtomicU64,
    data: EventData,
}
#[derive(Clone)]
struct EventIdentity(Arc<EventRecord>);
impl PartialEq for EventIdentity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for EventIdentity {}
/// Same-arena typed event capability. No raw object is decoded by lookup.
#[derive(Clone)]
pub struct EventHandle {
    receiver: usize,
    record: Arc<EventRecord>,
}
impl EventHandle {
    pub(super) fn check_pure(&self) -> Result<()> {
        let installed = INSTALLED.get().ok_or("workspace absent")?;
        if self.record.owner != std::thread::current().id()
            || !Arc::ptr_eq(&installed.model, &self.record.model)
            || !self.record.life.0.load(Ordering::Acquire)
            || self.record.model.faulted.load(Ordering::Acquire)
        {
            return Err("event identity retired, foreign or unowned".into());
        }
        {
            let state = self
                .record
                .model
                .state
                .lock()
                .map_err(|_| "workspace state poisoned")?;
            if state.retired || state.generation != self.record.handle.generation {
                return Err("event generation expired".into());
            }
        }
        let serial = self.record.serial.load(Ordering::Acquire);
        if serial == 0
            || super::factory::owned_reference_serial(self.receiver as *mut AnyObject)? != serial
        {
            return Err("event allocation serial expired".into());
        }
        let objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        if !matches!(objects.get(&self.receiver), Some(Kind::Event(identity)) if Arc::ptr_eq(&identity.0, &self.record))
        {
            return Err("event receiver identity no longer published".into());
        }
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        self.check_pure()?;
        self.record.model.with(|_| Ok(()))?;
        self.check_pure()
    }
    pub fn data(&self) -> Result<EventData> {
        self.check()?;
        Ok(self.record.data.clone())
    }
    pub fn belongs_to(&self, model: &Arc<Workspace>) -> bool {
        Arc::ptr_eq(&self.record.model, model)
    }
    pub fn class(&self) -> Result<&'static AnyClass> {
        self.check()?;
        Ok(super::factory::receiver_identity(self.receiver as *mut AnyObject)?.class())
    }
    pub fn borrowed_pointer(&self) -> Result<*mut AnyObject> {
        self.check()?;
        Ok(self.receiver as *mut AnyObject)
    }
}
pub fn event_handle(receiver: *mut AnyObject) -> Result<EventHandle> {
    let record = {
        let objects = INSTALLED
            .get()
            .ok_or("workspace absent")?
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        match objects.get(&(receiver as usize)) {
            Some(Kind::Event(identity)) => identity.0.clone(),
            _ => return Err("unknown or wrong-kind event receiver".into()),
        }
    };
    let handle = EventHandle {
        receiver: receiver as usize,
        record,
    };
    handle.check()?;
    Ok(handle)
}
/// One genuine +1 held by an owned event queue. ARC cleanup is independent of
/// business admission and rechecks the true allocation serial on the owner.
pub struct EventLease {
    handle: EventHandle,
    reference: super::factory::RetainedReference,
}
impl EventLease {
    pub fn acquire(handle: &EventHandle) -> Result<Arc<Self>> {
        handle.check()?;
        let reference = super::factory::RetainedReference::acquire(
            handle.receiver as *mut AnyObject,
            &handle.record.model,
        )?;
        handle.check()?;
        Ok(Arc::new(Self {
            handle: handle.clone(),
            reference,
        }))
    }
    pub fn handle(&self) -> &EventHandle {
        &self.handle
    }
    pub fn data(&self) -> Result<EventData> {
        self.handle.data()
    }
    pub fn borrowed_pointer(&self) -> Result<*mut AnyObject> {
        self.handle.check()?;
        self.reference.pointer()
    }
    pub fn class(&self) -> Result<&'static AnyClass> {
        self.handle.class()
    }
}
/// Construct only in the installed, already-qualified NSEvent source factory.
/// Returns a genuine new +1; no original event constructor or fallback is used.
pub fn new_owned_event(model: &Arc<Workspace>, data: EventData) -> Result<*mut AnyObject> {
    deny_foundation_only_factory()?;
    data.validate()?;
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(&installed.model, model) {
        return Err("foreign event model".into());
    }
    let handle = model.with(|state| {
        if let Some(window) = data.window {
            if window.generation != state.generation || !state.windows.contains_key(&window) {
                return Err("event window identity is unowned or stale".into());
            }
        }
        let handle = Handle {
            id: state.next,
            generation: state.generation,
        };
        state.next = state
            .next
            .checked_add(1)
            .ok_or("event identities exhausted")?;
        Ok(handle)
    })?;
    let record = Arc::new(EventRecord {
        model: model.clone(),
        handle,
        owner: std::thread::current().id(),
        life: DescriptorLife(Arc::new(AtomicBool::new(true))),
        serial: std::sync::atomic::AtomicU64::new(0),
        data,
    });
    let pointer = object(Kind::Event(EventIdentity(record.clone())))?;
    let pending = unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?;
    record.serial.store(
        super::factory::owned_reference_serial(pointer)?,
        Ordering::Release,
    );
    let cap = EventHandle {
        receiver: pointer as usize,
        record,
    };
    cap.check()?;
    Ok(pending.into_raw())
}
unsafe extern "C" fn owned_event_dealloc(receiver: *mut AnyObject, _: Sel) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        super::factory::deallocate_owned(receiver, || {
            let installed = INSTALLED
                .get()
                .ok_or("workspace absent during event cleanup")?;
            let identity = {
                let mut objects = installed
                    .objects
                    .lock()
                    .map_err(|_| "object arena poisoned")?;
                match objects.get(&(receiver as usize)) {
                    Some(Kind::Event(identity)) => {
                        identity.0.life.0.store(false, Ordering::Release);
                        objects.remove(&(receiver as usize))
                    }
                    None => None,
                    Some(_) => return Err("event deallocation receiver kind differs".into()),
                }
            };
            drop(identity);
            Ok(())
        })
    }));
    let failed = match outcome {
        Ok(result) => result.is_err(),
        Err(payload) => {
            std::mem::forget(payload);
            true
        }
    };
    if failed {
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}
struct DescriptorPreparation {
    life: DescriptorLife,
    receiver: Option<usize>,
    owns_receiver: bool,
    published: bool,
}
impl Drop for DescriptorPreparation {
    fn drop(&mut self) {
        if !self.published {
            self.life.0.store(false, Ordering::Release);
            if self.owns_receiver {
                if let Some(receiver) = self.receiver {
                    let released =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                            release_apple_event_descriptor(receiver as *mut AnyObject)
                        }));
                    match released {
                        Ok(Ok(())) => return,
                        Ok(Err(_)) => (),
                        Err(payload) => std::mem::forget(payload),
                    }
                    if let Some(installed) = INSTALLED.get() {
                        installed.model.fault();
                    }
                }
            }
            if let (Some(receiver), Some(installed)) = (self.receiver, INSTALLED.get()) {
                if let Ok(mut objects) = installed.objects.lock() {
                    if matches!(objects.get(&receiver), Some(Kind::AppleEventDescriptor(_, _, life))
                        if life == &self.life)
                    {
                        objects.remove(&receiver);
                    }
                }
            }
        }
    }
}
struct Installed {
    model: Arc<Workspace>,
    apple_event_data_only: bool,
    foundation_data_only: bool,
    #[cfg(test)]
    fixture_classes: bool,
    objects: Mutex<BTreeMap<usize, Kind>>,
    notifications: notification_bridge::Centers,
    typed_data: typed_data::State,
    assembly: OnceLock<WorkspaceAssembly>,
    clipboard_constants: OnceLock<Arc<ClipboardConstants>>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    palette_host_release: OnceLock<Arc<AtomicBool>>,
}
const CLIPBOARD_CONSTANTS:[(&CStr,&str);5]=[
    (c"NSPasteboardTypeString","public.utf8-plain-text"),
    (c"NSPasteboardTypeHTML","public.html"),
    (c"NSPasteboardTypePNG","public.png"),
    (c"NSPasteboardTypeTIFF","public.tiff"),
    (c"NSPasteboardURLReadingFileURLsOnlyKey","NSPasteboardURLReadingFileURLsOnlyKey"),
];
/// Stable data slots, each backed by its actual same-arena retained immutable
/// NSString. A data import points to the slot, never directly to the object.
struct ClipboardConstants {
    model:Arc<Workspace>,
    generation:u64,
    values:[FoundationReference;5],
    slots:[usize;5],
    originals:[usize;5],
    image_bases:[usize;5],
}
fn original_clipboard_slots()->[usize;5] {
    #[link(name="AppKit",kind="framework")]
    unsafe extern "C" {
        static NSPasteboardTypeString:*mut AnyObject;
        static NSPasteboardTypeHTML:*mut AnyObject;
        static NSPasteboardTypePNG:*mut AnyObject;
        static NSPasteboardTypeTIFF:*mut AnyObject;
        static NSPasteboardURLReadingFileURLsOnlyKey:*mut AnyObject;
    }
    [(&raw const NSPasteboardTypeString) as usize,(&raw const NSPasteboardTypeHTML) as usize,
        (&raw const NSPasteboardTypePNG) as usize,(&raw const NSPasteboardTypeTIFF) as usize,
        (&raw const NSPasteboardURLReadingFileURLsOnlyKey) as usize]
}
fn original_clipboard_slot_image(slot:usize)->Result<usize> {
    let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
    if unsafe {libc::dladdr(slot as *const std::ffi::c_void,info.as_mut_ptr())}==0 {
        return Err("clipboard original data slot image absent".into());
    }
    let info=unsafe {info.assume_init()};
    if info.dli_fname.is_null() || info.dli_fbase.is_null()
        || unsafe {CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit" {
        return Err("clipboard original data slot is not actual AppKit".into());
    }
    Ok(info.dli_fbase as usize)
}
impl ClipboardConstants {
    fn pure_model(&self)->Result<()> {
        let state=self.model.state.lock().map_err(|_| "workspace state poisoned")?;
        if state.retired || state.generation!=self.generation || self.model.faulted.load(Ordering::Acquire) {
            return Err("clipboard constants model retired".into());
        }
        Ok(())
    }
    fn verify(&self)->Result<()> {
        self.pure_model()?;
        self.model.with(|_|Ok(()))?;
        self.verify_pure()
    }
    fn verify_pure(&self)->Result<()> {
        let installed=INSTALLED.get().ok_or("workspace absent")?;
        if !Arc::ptr_eq(&installed.model,&self.model) {
            return Err("clipboard constant model differs".into());
        }
        // Originals were captured before the parsed-main transaction. Do not
        // re-resolve these symbols through a GOT that this transaction may
        // itself replace when a bounded fixture is linked into MAIN. The fixed
        // linked AppKit image is process-lifetime; each captured data address is
        // checked against its actual image again below.
        self.pure_model()?;
        for (index,(_,text)) in CLIPBOARD_CONSTANTS.iter().enumerate() {
            if original_clipboard_slot_image(self.originals[index])?!=self.image_bases[index] {
                return Err("clipboard original data image changed".into());
            }
            let pointer=verify_constant_reference(&self.values[index],&text.encode_utf16().collect::<Vec<_>>())?;
            if pointer!=self.slots[index] {
                return Err("clipboard constant retained payload changed".into());
            }
        }
        self.pure_model()
    }
    fn import_specs(&self)->Result<Vec<crate::sdk::ImportSpec>> {
        self.verify()?;
        Ok(CLIPBOARD_CONSTANTS.iter().enumerate().map(|(index,(symbol,_))|crate::sdk::ImportSpec {
            symbol,original:self.originals[index],replacement:(&raw const self.slots[index]) as usize,
        }).collect())
    }
    fn replacement(&self,symbol:&CStr)->Option<usize> {
        CLIPBOARD_CONSTANTS.iter().position(|(name,_)|*name==symbol)
            .map(|index|(&raw const self.slots[index]) as usize)
    }
}
fn prepare_clipboard_constants(model:&Arc<Workspace>)->Result<Arc<ClipboardConstants>> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(&installed.model,model) || installed.foundation_data_only || installed.apple_event_data_only
        || installed.clipboard_constants.get().is_some() {return Err("clipboard constants arena/duplicate scope differs".into());}
    let generation=model.with(|state|Ok(state.generation))?;
    let mut values=Vec::new();
    let mut slots=Vec::new();
    for (_,text) in CLIPBOARD_CONSTANTS {
        let pointer=new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(text.as_bytes())?)))?;
        let pending=unsafe {super::factory::OwnedReceiver::from_owned(pointer)}?;
        values.push(foundation_reference(pointer)?);
        slots.push(pointer as usize);
        drop(pending);
    }
    let originals=original_clipboard_slots();
    let mut image_bases=Vec::new();
    for slot in originals {image_bases.push(original_clipboard_slot_image(slot)?);}
    let constants=Arc::new(ClipboardConstants {model:model.clone(),generation,
        values:values.try_into().map_err(|_| "clipboard constant count differs")?,
        slots:slots.try_into().map_err(|_| "clipboard slot count differs")?,originals,
        image_bases:image_bases.try_into().map_err(|_| "clipboard image count differs")?});
    constants.verify()?;
    installed.clipboard_constants.set(constants.clone()).map_err(|_| "clipboard constants already published")?;
    Ok(constants)
}
struct WorkspaceAssembly {
    catalog: Arc<dyn application::CallbackCatalog>,
    imports: Vec<crate::sdk::ImportReplacement>,
}
static INSTALLED: OnceLock<Installed> = OnceLock::new();
/// Data-only receiver identity. Construction and every use remain bound to the
/// same installed workspace, generation and creating worker thread.
#[derive(Clone)]
pub(super) struct AppleEventHandle {
    model: Arc<Workspace>,
    handle: Handle,
    owner: std::thread::ThreadId,
    life: DescriptorLife,
}
impl AppleEventHandle {
    pub(super) fn check(&self) -> Result<()> {
        if std::thread::current().id() != self.owner {
            return Err("AppleEvent descriptor belongs to another thread".into());
        }
        if !self.life.0.load(Ordering::Acquire) {
            return Err("AppleEvent descriptor retired".into());
        }
        let installed = INSTALLED.get().ok_or("workspace not installed")?;
        if !Arc::ptr_eq(&installed.model, &self.model) {
            return Err("AppleEvent descriptor belongs to another workspace".into());
        }
        self.model.with(|state| {
            if state.generation != self.handle.generation {
                return Err("AppleEvent descriptor generation expired".into());
            }
            Ok(())
        })?;
        // The dynamic model guardian may retire this capability reentrantly.
        // Reobserve pure local state after it returns, without a second guard.
        if !self.life.0.load(Ordering::Acquire) {
            return Err("AppleEvent descriptor retired during guardian".into());
        }
        Ok(())
    }
    pub(super) fn key(&self) -> Result<Handle> {
        self.check()?;
        Ok(self.handle)
    }
}
fn reserve_apple_event_handle() -> Result<AppleEventHandle> {
    let model = INSTALLED
        .get()
        .ok_or("workspace not installed")?
        .model
        .clone();
    let handle = model.with(|state| {
        let handle = Handle {
            id: state.next,
            generation: state.generation,
        };
        state.next = state.next.checked_add(1).ok_or("descriptor ID exhausted")?;
        Ok(handle)
    })?;
    Ok(AppleEventHandle {
        model,
        handle,
        owner: std::thread::current().id(),
        life: DescriptorLife(Arc::new(AtomicBool::new(true))),
    })
}
/// Prepare all bounded leaf data before a constructor can re-enter the arena.
/// Failed preparations/allocations are never published or assigned another ID.
pub(super) fn new_apple_event_descriptor(
    prepare: impl FnOnce(AppleEventHandle) -> Result<()>,
) -> Result<*mut AnyObject> {
    deny_foundation_only_factory()?;
    let handle = reserve_apple_event_handle()?;
    let mut preparation = DescriptorPreparation {
        life: handle.life.clone(),
        receiver: None,
        owns_receiver: true,
        published: false,
    };
    prepare(handle.clone())?;
    handle.check()?;
    let receiver = object(Kind::AppleEventDescriptor(
        handle.handle,
        handle.owner,
        handle.life.clone(),
    ))?;
    preparation.receiver = Some(receiver as usize);
    handle.check()?;
    preparation.published = true;
    Ok(receiver)
}
/// Bind the actual allocated receiver at the selected source initializer.
/// # Safety
/// The exact published class grant owns this new +1 allocation. No original
/// initializer or arbitrary raw receiver may enter this source-init hook.
pub(super) unsafe fn bind_apple_event_descriptor(
    receiver: *mut AnyObject,
    prepare: impl FnOnce(AppleEventHandle) -> Result<()>,
) -> Result<*mut AnyObject> {
    deny_foundation_only_factory()?;
    if receiver.is_null() {
        return Err("null AppleEvent initializer receiver".into());
    }
    let installed = INSTALLED.get().ok_or("workspace not installed")?;
    super::factory::check_owned_initializer_claim(
        receiver,
        &installed.model,
        owned_apple_event_dealloc as *const () as usize,
    )?;
    if installed
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?
        .contains_key(&(receiver as usize))
    {
        return Err("AppleEvent initializer receiver already published".into());
    }
    let handle = reserve_apple_event_handle()?;
    let mut preparation = DescriptorPreparation {
        life: handle.life.clone(),
        receiver: None,
        owns_receiver: false,
        published: false,
    };
    prepare(handle.clone())?;
    handle.check()?;
    unsafe { super::factory::initialize(receiver, &handle.model) }?;
    handle.check()?;
    {
        let mut objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        if objects.contains_key(&(receiver as usize)) {
            return Err("AppleEvent initializer publication replay".into());
        }
        objects.insert(
            receiver as usize,
            Kind::AppleEventDescriptor(handle.handle, handle.owner, handle.life.clone()),
        );
        preparation.receiver = Some(receiver as usize);
    }
    handle.check()?;
    preparation.published = true;
    Ok(receiver)
}
pub(super) fn apple_event_descriptor(receiver: *mut AnyObject) -> Result<AppleEventHandle> {
    let handle = apple_event_identity(receiver)?;
    handle.check()?;
    Ok(handle)
}
fn apple_event_identity(receiver: *mut AnyObject) -> Result<AppleEventHandle> {
    let installed = INSTALLED.get().ok_or("workspace not installed")?;
    // This lookup interprets only an existing integer identity; it does not
    // dereference unknown or foreign-thread Objective-C pointers.
    let kind = installed
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?
        .get(&(receiver as usize))
        .cloned();
    let Some(Kind::AppleEventDescriptor(handle, owner, life)) = kind else {
        return Err("not an owned AppleEvent descriptor".into());
    };
    if owner != std::thread::current().id() {
        return Err("AppleEvent descriptor belongs to another thread".into());
    }
    let handle = AppleEventHandle {
        model: installed.model.clone(),
        handle,
        owner,
        life,
    };
    Ok(handle)
}
/// Retire business authority before leaf cleanup can re-enter. The cleanup key
/// permits removing data only; every previously issued business cap now refuses.
#[cfg(test)]
pub(super) fn retire_apple_event_descriptor(
    receiver: *mut AnyObject,
    release: impl FnOnce(Handle) -> Result<()>,
) -> Result<()> {
    // Cleanup is allowed after guardian failure or generation retirement. This
    // pure identity lookup grants no business read and never invokes a guardian.
    let handle = apple_event_identity(receiver)?;
    if handle
        .life
        .0
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("AppleEvent descriptor retirement replay".into());
    }
    finish_apple_event_retirement(receiver, handle, release)
}
fn finish_apple_event_retirement(
    receiver: *mut AnyObject,
    handle: AppleEventHandle,
    release: impl FnOnce(Handle) -> Result<()>,
) -> Result<()> {
    let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(handle.handle)));
    let installed = INSTALLED.get().ok_or("workspace not installed")?;
    {
        let mut objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        if objects.get(&(receiver as usize))
            != Some(&Kind::AppleEventDescriptor(
                handle.handle,
                handle.owner,
                handle.life.clone(),
            ))
        {
            return Err("AppleEvent retirement identity changed".into());
        }
        objects.remove(&(receiver as usize));
    }
    match cleanup {
        Ok(result) => result,
        Err(payload) => {
            // An unknown panic payload can itself have a panicking destructor.
            std::mem::forget(payload);
            Err("AppleEvent leaf cleanup panicked; descriptor retired".into())
        }
    }
}
unsafe extern "C" fn owned_apple_event_dealloc(receiver: *mut AnyObject, _: Sel) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        super::factory::deallocate_owned(receiver, || {
            let installed = INSTALLED
                .get()
                .ok_or("workspace absent during SDK deallocation")?;
            let kind = installed
                .objects
                .lock()
                .map_err(|_| "object arena poisoned")?
                .get(&(receiver as usize))
                .cloned();
            match kind {
                None => Ok(()), // a failed, never-published allocated/init receiver
                Some(Kind::AppleEventDescriptor(handle, owner, life)) => {
                    if owner != std::thread::current().id() {
                        return Err("foreign AppleEvent deallocation owner".into());
                    }
                    life.0.store(false, Ordering::Release);
                    let cap = AppleEventHandle {
                        model: installed.model.clone(),
                        handle,
                        owner,
                        life,
                    };
                    finish_apple_event_retirement(receiver, cap, apple_events::release)
                }
                Some(_) => Err("AppleEvent deallocation receiver kind differs".into()),
            }
        })
    }));
    let failed = match result {
        Ok(result) => result.is_err(),
        Err(payload) => {
            std::mem::forget(payload);
            true
        }
    };
    if failed {
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}
/// # Safety
/// Transfer one fresh +1 only after the originating method's final cap check.
/// This performs no dynamic business callback after that check.
pub(super) unsafe fn autorelease_apple_event_descriptor(
    receiver: *mut AnyObject,
) -> Result<*mut AnyObject> {
    let cap = apple_event_identity(receiver)?;
    if !cap.life.0.load(Ordering::Acquire) {
        return Err("retired AppleEvent handoff".into());
    }
    unsafe { super::factory::autorelease_owned(receiver) }
}
/// # Safety
/// Consume one owned +1, including failed/unpublished initializer results.
pub(super) unsafe fn release_apple_event_descriptor(receiver: *mut AnyObject) -> Result<()> {
    unsafe { super::factory::release_owned(receiver) }
}
/// Check and atomically claim init-family provenance before leaf adoption of +1.
/// A preparing/initialized/published receiver cannot be claimed a second time.
/// This
/// permits cleanup of retired data but grants no business operation. Unknown or
/// foreign raw inputs cause no message, pointer dereference or release.
pub(super) fn check_apple_event_initializer(receiver: *mut AnyObject) -> Result<()> {
    let installed = INSTALLED.get().ok_or("workspace not installed")?;
    super::factory::check_owned_family(
        receiver,
        &installed.model,
        owned_apple_event_dealloc as *const () as usize,
    )
}
/// Observed method identities, without image/source authorization or mutation.
pub struct AppleEventMethodPreflight {
    pub selector: &'static CStr,
    pub class_method: bool,
    pub source: crate::sdk::metadata::MethodSnapshot,
    pub replacement: crate::sdk::metadata::CodeIdentity,
}
/// Check the complete compiled AE roster before lifecycle or factory writes.
/// Only already-loaded class metadata is read; no receiver message is sent.
pub fn preflight_apple_event_factory() -> Result<Vec<AppleEventMethodPreflight>> {
    let loaded = crate::sdk::metadata::loaded_class(c"NSAppleEventDescriptor")?;
    let signatures = apple_events::signatures();
    let replacements = apple_events::replacements();
    if signatures.len() != replacements.len() {
        return Err("AppleEvent signature roster length differs".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut observed = Vec::with_capacity(replacements.len());
    for (class, selector, class_method, implementation) in replacements {
        if class != c"NSAppleEventDescriptor" || !seen.insert((selector, class_method)) {
            return Err("foreign or duplicate AppleEvent replacement".into());
        }
        let mut matching = signatures
            .iter()
            .filter(|(name, meta, _)| *name == selector && *meta == class_method);
        let signature = matching.next().ok_or("AppleEvent signature missing")?;
        if matching.next().is_some() {
            return Err("duplicate AppleEvent signature".into());
        }
        let source = crate::sdk::metadata::method(loaded, selector, class_method)?
            .ok_or("source AppleEvent method missing")?;
        super::classes::verify_typed_abi(&source, &signature.2)?;
        observed.push(AppleEventMethodPreflight {
            selector,
            class_method,
            source,
            replacement: crate::sdk::metadata::code_identity(implementation)?,
        });
    }
    Ok(observed)
}
/// Publish the actual type-preserving source factory, never an NSObject stand-in.
/// # Safety
/// The common coordinator has verified actual preentry Pins, complete image and
/// header/lifecycle/source-family boundaries for this loaded NSAE family. No
/// original NSAE instance, method or concurrent class mutation may race setup.
/// This does not register a scope or authorize whole-SDK source readiness.
pub unsafe fn prepare_apple_event_factory(model: Arc<Workspace>) -> Result<()> {
    unsafe { prepare_apple_event_factory_checked(model, || Ok(())) }
}
/// Publish using a source-pin validator after the final dynamic lease callback.
/// # Safety
/// Same exclusive source/image authority as prepare_apple_event_factory. The
/// validator rechecks its exact admitted class/lifecycle/leaf/code identities
/// without class mutation or another dynamic guardian callback afterward.
pub unsafe fn prepare_apple_event_factory_checked(
    model: Arc<Workspace>,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    // Pure ABI/code observation must finish before any lifecycle replacement.
    // The preentry coordinator separately authorizes these exact identities.
    deny_foundation_only_factory()?;
    preflight_apple_event_factory()?;
    let loaded = crate::sdk::metadata::loaded_class(c"NSAppleEventDescriptor")?;
    let parent = unsafe { &*(loaded.address() as *const AnyClass) };
    let guard = unsafe {
        super::classes::ClassGuard::select_sdk_family_checked(
            &[parent],
            model.lease(),
            owned_apple_event_dealloc,
            prewrite,
        )
    }?;
    let proxy =
        unsafe { guard.subclass_qualified(c"RuycastF1OwnedAppleEventDescriptorSource", parent) }?;
    let mut bindings = guard.lifecycle_bindings();
    let signatures = apple_events::signatures();
    for (class, selector, meta, implementation) in apple_events::replacements() {
        if class != c"NSAppleEventDescriptor" {
            return Err("foreign AppleEvent replacement family".into());
        }
        let signature = signatures
            .iter()
            .find(|(name, class_method, _)| *name == selector && *class_method == meta)
            .ok_or("compiled AppleEvent method signature missing")?;
        let replacement = unsafe {
            guard.overlay_bound(
                if meta { parent } else { proxy },
                selector,
                meta,
                implementation,
                &signature.2,
            )
        }?;
        bindings.push(super::receiver::LifecycleBinding::leaf(replacement));
    }
    let grant = unsafe { super::receiver::WorkspaceGrant::for_owned_sdk(model, proxy, bindings) }?;
    super::factory::publish_proxy(c"NSAppleEventDescriptor", Arc::new(grant))
}
/// Actual source method/code observations; caller supplies image/source authority.
pub struct ApplicationMethodPreflight {
    pub selector: &'static CStr,
    pub class_method: bool,
    pub source: crate::sdk::metadata::MethodSnapshot,
    pub replacement: crate::sdk::metadata::CodeIdentity,
}
/// Observe the already-loaded App family without sending any receiver message.
pub fn preflight_application_factory() -> Result<Vec<ApplicationMethodPreflight>> {
    let class = crate::sdk::metadata::loaded_class(c"NSApplication")?;
    let signatures = application::signatures();
    let replacements = application::replacements();
    if signatures.len() != replacements.len() {
        return Err("application signature roster length differs".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut observations = vec![];
    for (role, selector, meta, implementation) in replacements {
        if role != c"NSApplication" || !seen.insert((selector, meta)) {
            return Err("foreign or duplicate application replacement".into());
        }
        let matching = signatures
            .iter()
            .filter(|(name, is_meta, _)| *name == selector && *is_meta == meta)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err("application signature missing or duplicated".into());
        }
        let source = crate::sdk::metadata::method(class, selector, meta)?
            .ok_or("source application method missing")?;
        super::classes::verify_typed_abi(&source, &matching[0].2)?;
        observations.push(ApplicationMethodPreflight {
            selector,
            class_method: meta,
            source,
            replacement: crate::sdk::metadata::code_identity(implementation)?,
        });
    }
    Ok(observations)
}
/// Prepare the genuine NSApplication subtype using the sole factory/arena.
/// # Safety
/// The coordinator holds independently reviewed actual App source, lifecycle,
/// ancestor/image and callsite Pins, exclusive setup, and no original UI object
/// or initializer can race this operation. prewrite is the final pure source
/// comparison after the last dynamic lease callback; it grants no new scope.
pub unsafe fn prepare_application_factory_checked(
    model: Arc<Workspace>,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    deny_foundation_only_factory()?;
    preflight_application_factory()?;
    let responder = crate::sdk::metadata::loaded_class(c"NSResponder")?;
    let application = crate::sdk::metadata::loaded_class(c"NSApplication")?;
    let responder = unsafe { &*(responder.address() as *const AnyClass) };
    let parent = unsafe { &*(application.address() as *const AnyClass) };
    if parent.superclass() != Some(responder) {
        return Err("application responder hierarchy differs".into());
    }
    let guard = unsafe {
        super::classes::ClassGuard::select_sdk_family_checked(
            &[responder, parent],
            model.lease(),
            owned_application_dealloc,
            prewrite,
        )
    }?;
    unsafe { prepare_application_with_guard(model, parent, &guard) }
}
unsafe fn prepare_application_with_guard(
    model:Arc<Workspace>,parent:&'static AnyClass,guard:&super::classes::ClassGuard,
) -> Result<()> {
    let proxy = unsafe { guard.subclass_qualified(c"RuycastF1OwnedApplicationSource", parent) }?;
    let mut bindings = guard.lifecycle_bindings_for(parent)?;
    let signatures = application::signatures();
    for (_, selector, meta, implementation) in application::replacements() {
        let signature = signatures
            .iter()
            .find(|(name, is_meta, _)| *name == selector && *is_meta == meta)
            .ok_or("application compiled signature missing")?;
        let binding = unsafe {
            guard.overlay_bound(
                if meta { parent } else { proxy },
                selector,
                meta,
                implementation,
                &signature.2,
            )
        }?;
        bindings.push(super::receiver::LifecycleBinding::leaf(binding));
    }
    let grant = unsafe { super::receiver::WorkspaceGrant::for_owned_sdk(model, proxy, bindings) }?;
    super::factory::publish_proxy(c"NSApplication", Arc::new(grant))
}
unsafe extern "C" fn owned_application_dealloc(receiver: *mut AnyObject, _: Sel) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        super::factory::deallocate_owned(receiver, || {
            let installed = INSTALLED
                .get()
                .ok_or("workspace absent during application deallocation")?;
            let mut objects = installed
                .objects
                .lock()
                .map_err(|_| "object arena poisoned")?;
            match objects.get(&(receiver as usize)) {
                None => Ok(()),
                Some(Kind::ApplicationShell) => {
                    objects.remove(&(receiver as usize));
                    Ok(())
                }
                Some(_) => Err("application deallocation receiver kind differs".into()),
            }
        })
    }));
    let failed = match outcome {
        Ok(result) => result.is_err(),
        Err(payload) => {
            std::mem::forget(payload);
            true
        }
    };
    if failed {
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}
/// Data-only NSEvent getter factory. Native constructor and monitor callsites
/// require their own complete roster before the full family can be admitted.
/// # Safety
/// Same exclusive, source/image/initializer preentry authority as the App
/// factory, specifically for this loaded NSEvent family and compiled getters.
pub unsafe fn prepare_event_data_factory_checked(
    model: Arc<Workspace>,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    deny_foundation_only_factory()?;
    let loaded = crate::sdk::metadata::loaded_class(c"NSEvent")?;
    let parent = unsafe { &*(loaded.address() as *const AnyClass) };
    // All typed signatures are observed before lifecycle mutation.
    let signatures = event_methods::signatures();
    for (_, selector, meta, _) in event_methods::replacements() {
        let source = crate::sdk::metadata::method(loaded, selector, meta)?
            .ok_or("source NSEvent getter missing")?;
        let signature = signatures
            .iter()
            .find(|(name, is_meta, _)| *name == selector && *is_meta == meta)
            .ok_or("event getter signature missing")?;
        super::classes::verify_typed_abi(&source, &signature.2)?;
    }
    let guard = unsafe {
        super::classes::ClassGuard::select_sdk_joint_family_checked(
            &[parent],
            model.lease(),
            &[(parent, owned_event_dealloc)],
            prewrite,
            cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")),
        )
    }?;
    let proxy = unsafe { guard.subclass_qualified(c"RuycastF1OwnedEventSource", parent) }?;
    let mut bindings = guard.lifecycle_bindings();
    bindings.extend(unsafe { guard.bind_preserved_appkit_proxy_lifecycle(proxy, owned_event_dealloc) }?);
    for (_, selector, meta, implementation) in event_methods::replacements() {
        let signature = signatures
            .iter()
            .find(|(name, is_meta, _)| *name == selector && *is_meta == meta)
            .ok_or("event getter signature missing")?;
        let binding = unsafe {
            guard.overlay_bound(
                if meta { parent } else { proxy },
                selector, meta, implementation, &signature.2,
            )
        }?;
        bindings.push(super::receiver::LifecycleBinding::leaf(binding));
    }
    let grant = unsafe { super::receiver::WorkspaceGrant::for_owned_sdk(model, proxy, bindings) }?;
    super::factory::publish_proxy(c"NSEvent", Arc::new(grant))
}
mod event_methods {
    use super::*;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    type SnippetMonitorBlock = block2::Block<dyn Fn(*mut AnyObject)>;

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    struct SnippetMonitor {
        token: Retained<NSObject>,
        block: std::mem::ManuallyDrop<block2::RcBlock<dyn Fn(*mut AnyObject)>>,
        capture: usize,
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    thread_local! {
        static SNIPPET_MONITORS: std::cell::RefCell<BTreeMap<usize, SnippetMonitor>> =
            const { std::cell::RefCell::new(BTreeMap::new()) };
        static DELIVERING_SNIPPET_MOUSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_snippet_monitor(
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        controller: usize,
        token: usize,
    ) -> Result<()> {
        if DELIVERING_SNIPPET_MOUSE.with(std::cell::Cell::get) {
            return Err("SnippetChoice mouse callback still in flight".into());
        }
        SNIPPET_MONITORS.with(|monitors| {
            let monitors = monitors.try_borrow().map_err(|_| "SnippetChoice monitors busy")?;
            match monitors.get(&controller) {
                None if token == 0 && monitors.is_empty() => Ok(()),
                Some(monitor) if token != 0 && monitors.len() == 1 && Arc::ptr_eq(admission, &monitor.admission)
                    && Retained::as_ptr(&monitor.token) as usize == token => {
                    let block = &**monitor.block as *const SnippetMonitorBlock as usize;
                    if admission.verify_snippet_monitor_block(block)? != monitor.capture {
                        return Err("SnippetChoice monitor capture changed".into());
                    }
                    Ok(())
                }
                _ => Err("SnippetChoice monitor is missing or has another owner".into()),
            }
        })
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_snippet_monitors_drained() -> Result<()> {
        if DELIVERING_SNIPPET_MOUSE.with(std::cell::Cell::get) {
            return Err("SnippetChoice mouse callback still in flight".into());
        }
        SNIPPET_MONITORS.with(|monitors| {
            if !monitors.try_borrow().map_err(|_| "SnippetChoice monitors busy")?.is_empty() {
                return Err("SnippetChoice monitor cleanup incomplete".into());
            }
            Ok(())
        })
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn deliver_external_mouse(
        model: &Arc<Workspace>,
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        kind: u32,
        flags: u64,
    ) -> Result<()> {
        let (issuer, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
        let engine = application::engine()?;
        engine.check_owner()?;
        if !Arc::ptr_eq(admission, &issuer) || !Arc::ptr_eq(model, engine.model())
            || stopped.load(Ordering::Acquire) || engine.phase()? != super::super::event_loop::Phase::Running
            || !matches!(kind, 1 | 3)
        {
            return Err("SnippetChoice external mouse event issuer or phase differs".into());
        }
        let owners = SNIPPET_MONITORS.with(|monitors| -> Result<Vec<_>> {
            Ok(monitors.try_borrow().map_err(|_| "SnippetChoice monitors busy")?.iter()
                .map(|(&controller, monitor)| (controller, monitor.token.clone())).collect())
        })?;
        if owners.is_empty() {
            return Ok(());
        }
        if DELIVERING_SNIPPET_MOUSE.with(|active| active.replace(true)) {
            return Err("reentrant SnippetChoice mouse delivery".into());
        }
        struct DeliveryTurn;
        impl Drop for DeliveryTurn {
            fn drop(&mut self) {
                DELIVERING_SNIPPET_MOUSE.with(|active| active.set(false));
            }
        }
        let _turn = DeliveryTurn;
        let mut clock = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) } != 0 {
            return Err("SnippetChoice external mouse clock unavailable".into());
        }
        let pointer = new_owned_event(model, EventData {
            kind: kind as usize,
            location: model.mouse()?,
            modifiers: usize::try_from(flags).map_err(|_| "external mouse modifiers overflow")?,
            timestamp: clock.tv_sec as f64 + clock.tv_nsec as f64 / 1e9,
            window: None,
            details: EventDetails::Mouse { number: 0, click_count: 1, pressure: 1.0 },
        })?;
        let _event = unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?;
        for (controller, token) in owners {
            model.verify_reference_context(admission)?;
            if stopped.load(Ordering::Acquire) {
                return Err("SnippetChoice external mouse delivery stopped".into());
            }
            let copy = SNIPPET_MONITORS.with(|monitors| -> Result<_> {
                let monitors = monitors.try_borrow().map_err(|_| "SnippetChoice monitors busy")?;
                let Some(monitor) = monitors.get(&controller) else { return Ok(None); };
                if Retained::as_ptr(&monitor.token) != Retained::as_ptr(&token) {
                    return Ok(None);
                }
                let block = &**monitor.block as *const SnippetMonitorBlock;
                if !Arc::ptr_eq(admission, &monitor.admission)
                    || admission.verify_snippet_monitor_block(block as usize)? != monitor.capture
                {
                    return Err("SnippetChoice external mouse block changed".into());
                }
                let copy = unsafe { block2::RcBlock::copy(block.cast_mut()) }.ok_or("SnippetChoice monitor borrow failed")?;
                Ok(Some((std::mem::ManuallyDrop::new(copy), monitor.capture)))
            })?;
            let Some((block, capture)) = copy else { continue; };
            let address = &**block as *const SnippetMonitorBlock as usize;
            if admission.verify_snippet_monitor_block(address)? != capture {
                return Err("SnippetChoice external mouse block changed after borrow".into());
            }
            let phase = engine.phase();
            if !matches!(phase, Ok(super::super::event_loop::Phase::Running)) || stopped.load(Ordering::Acquire) {
                drop(std::mem::ManuallyDrop::into_inner(block));
                return Err("SnippetChoice external mouse delivery left its running phase".into());
            }
            let callback_panicked = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block.call((pointer,)))) {
                Ok(()) => false,
                Err(payload) => { std::mem::forget(payload); true }
            };
            if admission.verify_snippet_monitor_block(address)? != capture {
                return Err("SnippetChoice external mouse block changed after callback".into());
            }
            drop(std::mem::ManuallyDrop::into_inner(block));
            if callback_panicked {
                return Err("SnippetChoice external mouse callback panicked".into());
            }
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    #[unsafe(naked)]
    unsafe extern "C" fn add_snippet_monitor(
        _: *mut AnyObject, _: Sel, _: usize, _: *mut SnippetMonitorBlock,
    ) -> *mut AnyObject {
        core::arch::naked_asm!("mov x4, x30", "mov x5, x19", "b {checked}", checked = sym add_snippet_monitor_checked);
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    unsafe extern "C" fn add_snippet_monitor_checked(
        class: *mut AnyObject, _: Sel, mask: usize, block: *mut SnippetMonitorBlock,
        caller: usize, controller: usize,
    ) -> *mut AnyObject {
        window_boundary(std::ptr::null_mut(), || {
            if class as usize != crate::sdk::metadata::loaded_class(c"NSEvent")?.address() || mask != 0xa {
                return Err("unexpected global mouse monitor class or mask".into());
            }
            let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
            admission.verify_snippet_monitor_call(caller, controller, false)?;
            let installed = INSTALLED.get().ok_or("SnippetChoice workspace absent")?;
            crate::sdk::reference_runtime::verify_live_snippet_choice(&admission, &installed.model, controller)?;
            let token = NSObject::new();
            let returned = token.clone();
            let capture = admission.verify_snippet_monitor_block(block as usize)?;
            let block = unsafe { block2::RcBlock::copy(block) }.ok_or("SnippetChoice monitor block copy failed")?;
            if !admission.verify_snippet_monitor_block(&*block as *const SnippetMonitorBlock as usize)
                .is_ok_and(|actual| actual == capture)
            {
                std::mem::forget(block);
                return Err("SnippetChoice monitor capture changed after copy".into());
            }
            SNIPPET_MONITORS.with(|monitors| {
                let mut monitors = monitors.try_borrow_mut().map_err(|_| "SnippetChoice monitors busy")?;
                if monitors.contains_key(&controller) || stopped.load(Ordering::Acquire) {
                    return Err("SnippetChoice monitor already registered or stopped");
                }
                monitors.insert(controller, SnippetMonitor { token, block: std::mem::ManuallyDrop::new(block), capture, admission });
                Ok(())
            })?;
            Ok(Retained::autorelease_return(returned).cast())
        })
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    #[unsafe(naked)]
    unsafe extern "C" fn remove_snippet_monitor(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
        core::arch::naked_asm!("mov x3, x30", "mov x4, x19", "b {checked}", checked = sym remove_snippet_monitor_checked);
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    unsafe extern "C" fn remove_snippet_monitor_checked(
        class: *mut AnyObject, _: Sel, token: *mut AnyObject, caller: usize, controller: usize,
    ) {
        window_boundary((), || {
            if class as usize != crate::sdk::metadata::loaded_class(c"NSEvent")?.address() {
                return Err("unexpected global mouse monitor class".into());
            }
            let monitor = SNIPPET_MONITORS.with(|monitors| -> Result<SnippetMonitor> {
                let mut monitors = monitors.try_borrow_mut().map_err(|_| "SnippetChoice monitors busy")?;
                let monitor = monitors.get(&controller).ok_or("unknown SnippetChoice monitor owner")?;
                monitor.admission.verify_snippet_monitor_call(caller, controller, true)?;
                if Retained::as_ptr(&monitor.token).cast::<AnyObject>() != token.cast_const()
                    || monitor.admission.verify_snippet_monitor_block(&**monitor.block as *const SnippetMonitorBlock as usize)? != monitor.capture
                {
                    return Err("SnippetChoice monitor token or capture differs".into());
                }
                monitors.remove(&controller).ok_or_else(|| "SnippetChoice monitor disappeared".into())
            })?;
            drop(std::mem::ManuallyDrop::into_inner(monitor.block));
            Ok(())
        });
    }
    fn get<T>(
        receiver: *mut AnyObject,
        fallback: T,
        read: impl FnOnce(EventData) -> Result<T>,
    ) -> T {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read(event_handle(receiver)?.data()?)
        })) {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => fallback,
            Err(payload) => {
                std::mem::forget(payload);
                if let Some(installed) = INSTALLED.get() {
                    installed.model.fault();
                }
                fallback
            }
        }
    }
    unsafe extern "C" fn event_type(p: *mut AnyObject, _: Sel) -> usize {
        get(p, 0, |d| Ok(d.kind))
    }
    unsafe extern "C" fn modifiers(p: *mut AnyObject, _: Sel) -> usize {
        get(p, 0, |d| Ok(d.modifiers))
    }
    unsafe extern "C" fn timestamp(p: *mut AnyObject, _: Sel) -> f64 {
        get(p, 0.0, |d| Ok(d.timestamp))
    }
    unsafe extern "C" fn location(p: *mut AnyObject, _: Sel) -> NSPoint {
        get(p, NSPoint::new(0.0, 0.0), |d| {
            Ok(NSPoint::new(d.location.x, d.location.y))
        })
    }
    unsafe extern "C" fn window_number(p: *mut AnyObject, _: Sel) -> isize {
        get(p, 0, |d| match d.window {
            None => Ok(0),
            Some(h) => isize::try_from(h.id).map_err(|_| "event window number overflow".into()),
        })
    }
    unsafe extern "C" fn key_code(p: *mut AnyObject, _: Sel) -> u16 {
        get(p, 0, |d| match d.details {
            EventDetails::Key { key_code, .. } => Ok(key_code),
            _ => Err("not a key event".into()),
        })
    }
    unsafe extern "C" fn repeat(p: *mut AnyObject, _: Sel) -> Bool {
        get(p, Bool::new(false), |d| match d.details {
            EventDetails::Key { repeat, .. } => Ok(Bool::new(repeat)),
            _ => Err("not a key event".into()),
        })
    }
    unsafe extern "C" fn event_number(p: *mut AnyObject, _: Sel) -> isize {
        get(p, 0, |d| match d.details {
            EventDetails::Mouse { number, .. } => Ok(number),
            _ => Err("not a mouse event".into()),
        })
    }
    unsafe extern "C" fn clicks(p: *mut AnyObject, _: Sel) -> isize {
        get(p, 0, |d| match d.details {
            EventDetails::Mouse { click_count, .. } => Ok(click_count),
            _ => Err("not a mouse event".into()),
        })
    }
    unsafe extern "C" fn pressure(p: *mut AnyObject, _: Sel) -> f32 {
        get(p, 0.0, |d| match d.details {
            EventDetails::Mouse { pressure, .. } => Ok(pressure),
            _ => Err("not a mouse event".into()),
        })
    }
    unsafe extern "C" fn subtype(p: *mut AnyObject, _: Sel) -> i16 {
        get(p, 0, |d| match d.details {
            EventDetails::Other { subtype, .. } => Ok(subtype),
            _ => Err("not an other event".into()),
        })
    }
    unsafe extern "C" fn data1(p: *mut AnyObject, _: Sel) -> isize {
        get(p, 0, |d| match d.details {
            EventDetails::Other { data1, .. } => Ok(data1),
            _ => Err("not an other event".into()),
        })
    }
    unsafe extern "C" fn data2(p: *mut AnyObject, _: Sel) -> isize {
        get(p, 0, |d| match d.details {
            EventDetails::Other { data2, .. } => Ok(data2),
            _ => Err("not an other event".into()),
        })
    }
    macro_rules! roster {
        ($consumer:ident) => {
            $consumer! {
                (c"type",event_type,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
                (c"modifierFlags",modifiers,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
                (c"timestamp",timestamp,unsafe extern "C" fn(*mut AnyObject,Sel)->f64),
                (c"locationInWindow",location,unsafe extern "C" fn(*mut AnyObject,Sel)->NSPoint),
                (c"windowNumber",window_number,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
                (c"keyCode",key_code,unsafe extern "C" fn(*mut AnyObject,Sel)->u16),
                (c"isARepeat",repeat,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
                (c"eventNumber",event_number,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
                (c"clickCount",clicks,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
                (c"pressure",pressure,unsafe extern "C" fn(*mut AnyObject,Sel)->f32),
                (c"subtype",subtype,unsafe extern "C" fn(*mut AnyObject,Sel)->i16),
                (c"data1",data1,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
                (c"data2",data2,unsafe extern "C" fn(*mut AnyObject,Sel)->isize)
            }
        };
    }
    fn signature<F: objc2::runtime::MethodImplementation>(_: F) -> Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
        let mut types = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
        types.extend_from_slice(F::Arguments::ENCODINGS);
        types
    }
    macro_rules! export {
        ($(($name:expr,$handler:ident,$ty:ty)),*$(,)?) => {
            pub(super) fn replacements() -> Vec<(&'static CStr,&'static CStr,bool,usize)> {
                let mut rows = vec![$((c"NSEvent",$name,false,$handler as *const () as usize)),*];
                rows.push((c"NSEvent", c"mouseLocation", true, mouse as *const () as usize));
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                rows.extend([
                    (c"NSEvent", c"addGlobalMonitorForEventsMatchingMask:handler:", true, add_snippet_monitor as *const () as usize),
                    (c"NSEvent", c"removeMonitor:", true, remove_snippet_monitor as *const () as usize),
                ]);
                rows
            }
            pub(super) fn signatures() -> Vec<(&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
                let mut rows = vec![$(($name,false,signature($handler as $ty))),*];
                rows.push((c"mouseLocation", true, signature(mouse as unsafe extern "C" fn(*mut AnyObject, Sel) -> NSPoint)));
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                rows.extend([
                    (c"addGlobalMonitorForEventsMatchingMask:handler:", true, signature(add_snippet_monitor as unsafe extern "C" fn(*mut AnyObject, Sel, usize, *mut SnippetMonitorBlock) -> *mut AnyObject)),
                    (c"removeMonitor:", true, signature(remove_snippet_monitor as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject))),
                ]);
                rows
            }
        };
    }
    roster!(export);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use event_methods::deliver_external_mouse as deliver_reference_external_mouse;
/// Complete compiled Foundation/Clipboard observations, without source/image
/// authorization. The coordinator binds every class/ancestor/lifecycle/code
/// identity before permitting the checked family write below.
pub use super::factory::FoundationMemoryPlan;
pub fn preflight_foundation_memory() -> Result<FoundationMemoryPlan> {
    FoundationMemoryPlan::capture()
}
pub enum TypedDataMethodPreflight {
    Existing(AppleEventMethodPreflight),
    OwnedDeclaration {
        selector: &'static CStr,
        declaration: super::classes::StringBytesDeclaration,
    },
}
pub fn preflight_typed_data_factories() -> Result<Vec<(&'static CStr, TypedDataMethodPreflight)>> {
    preflight_data_roster(false)
}
/// Exactly the six immutable Foundation source families, no Pasteboard grant.
pub fn preflight_foundation_factories() -> Result<Vec<(&'static CStr, TypedDataMethodPreflight)>> {
    let observed = preflight_data_roster(true)?;
    for role in [
        c"NSString",
        c"NSData",
        c"NSURL",
        c"NSNumber",
        c"NSArray",
        c"NSDictionary",
    ] {
        let handle = crate::sdk::metadata::loaded_class(role)?;
        let mut class = Some(unsafe { &*(handle.address() as *const AnyClass) });
        let expected: &[&CStr] = if role == c"NSNumber" {
            &[c"NSNumber", c"NSValue", c"NSObject"]
        } else {
            &[role, c"NSObject"]
        };
        for name in expected {
            let current = class.ok_or("Foundation ancestor missing")?;
            if current.name() != *name {
                return Err("Foundation ancestor chain differs".into());
            }
            class = current.superclass();
        }
        if class.is_some() {
            return Err("Foundation ancestor chain has extra root".into());
        }
    }
    if observed.len() != 35 {
        return Err("Foundation-only roster count differs".into());
    }
    Ok(observed)
}
fn preflight_data_roster(
    foundation_only: bool,
) -> Result<Vec<(&'static CStr, TypedDataMethodPreflight)>> {
    let mut replacements = foundation::replacements();
    let mut signatures = foundation::signatures();
    if !foundation_only {
        replacements.extend(clipboard_bridge::replacements());
        signatures.extend(clipboard_bridge::signatures());
    }
    if replacements.len() != signatures.len() {
        return Err("typed data signature roster differs".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut observed = vec![];
    for (role, selector, meta, implementation) in replacements {
        if !seen.insert((role, selector, meta)) {
            return Err("duplicate typed data method".into());
        }
        let class = crate::sdk::metadata::loaded_class(role)?;
        let source = crate::sdk::metadata::method(class, selector, meta)?;
        let matching = signatures
            .iter()
            .filter(|(class, name, is_meta, _)| {
                *class == role && *name == selector && *is_meta == meta
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err("typed data signature missing or duplicated".into());
        }
        if role == c"NSString"
            && selector == super::classes::StringBytesDeclaration::SELECTOR
            && !meta
        {
            if source.is_some()
                || matching[0].3 != super::classes::StringBytesDeclaration::signature()
                || implementation != foundation::string_bytes_initializer() as *const () as usize
            {
                return Err("owned NSString declaration roster/base differs".into());
            }
            let declaration = super::classes::StringBytesDeclaration::capture(
                foundation::string_bytes_initializer(),
            )?;
            observed.push((
                role,
                TypedDataMethodPreflight::OwnedDeclaration {
                    selector,
                    declaration,
                },
            ));
            continue;
        }
        let source = source.ok_or_else(|| {
            format!("source typed data method missing: {role:?} {selector:?} class_method={meta}")
        })?;
        super::classes::verify_typed_abi(&source, &matching[0].3).map_err(|error| {
            format!(
                "typed data {role:?} {selector:?}: {error}; runtime={:?}",
                source.encoding()
            )
        })?;
        observed.push((
            role,
            TypedDataMethodPreflight::Existing(AppleEventMethodPreflight {
                selector,
                class_method: meta,
                source,
                replacement: crate::sdk::metadata::code_identity(implementation)?,
            }),
        ));
    }
    Ok(observed)
}
/// Prepare only independently source-qualified exact typed data class families.
/// # Safety
/// The common coordinator holds actual immutable source/header/image/all-six
/// lifecycle and ancestor Pins for this exact roster and exclusive preentry.
/// prewrite repeats those pure pins after the final dynamic guard callback.
/// This does not register any source14/collector/readiness scope.
pub unsafe fn prepare_typed_data_factories_checked(
    model: Arc<Workspace>,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    deny_foundation_only_factory()?;
    // These six immutable Foundation parents have custom RR primitives. Use
    // their existing qualified owned-memory substitution instead of asking
    // the default NSObject-only path to accept Foundation's original ARC.
    let memory=FoundationMemoryPlan::capture()?;
    unsafe { prepare_data_roster(model, false, Some(&memory), prewrite) }
}

/// Prepare the same eight factory proofs for the actual Reference issuer
/// before the sole workspace install. This creates no receiver or arena.
/// # Safety
/// The source-built Reference assembler owns exclusive class preparation.
#[cfg(all(rcf1_reference_sdk, not(test)))]
pub(crate) unsafe fn prepare_reference_data_factories(
    model: Arc<Workspace>,
    admission: &crate::sdk::core_preentry::ReferenceAdmission,
) -> Result<()> {
    let absent=|| -> Result<()> {
        if INSTALLED.get().is_some() {
            return Err("Reference data factory requires uninstalled workspace".into());
        }
        Ok(())
    };
    let prewrite=|| {
        absent()?;
        admission.before_reference_initialization()?;
        model.verify_reference_context(admission)?;
        absent()
    };
    prewrite()?;
    struct Setup<'a> { model:&'a Workspace, complete:bool }
    impl Drop for Setup<'_> {
        fn drop(&mut self) {
            if !self.complete { self.model.fault(); }
        }
    }
    let mut setup=Setup{model:&model,complete:false};
    let memory=FoundationMemoryPlan::capture()?;
    #[cfg(target_arch = "aarch64")]
    foundation::bind_website_array_factory()?;
    unsafe { prepare_data_roster_after_arena_gate(model.clone(),false,Some(&memory),prewrite,absent) }?;
    prewrite()?;
    setup.complete=true;
    Ok(())
}
/// # Safety
/// Caller holds the independently admitted Foundation-only source/header/image
/// pins. The same immutable arena must already be installed for this model.
pub unsafe fn prepare_foundation_factories_checked(
    model: Arc<Workspace>,
    memory: &FoundationMemoryPlan,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let installed = INSTALLED.get().ok_or("Foundation-only arena absent")?;
    if !installed.foundation_data_only || !Arc::ptr_eq(&installed.model, &model) {
        return Err("Foundation-only factory/model scope differs".into());
    }
    unsafe { prepare_data_roster(model, true, Some(memory), prewrite) }
}
fn deny_foundation_only_factory() -> Result<()> {
    if INSTALLED
        .get()
        .is_some_and(|installed| installed.foundation_data_only)
    {
        return Err("non-Foundation factory in immutable-only arena".into());
    }
    Ok(())
}
unsafe fn prepare_data_roster(
    model: Arc<Workspace>,
    foundation_only: bool,
    memory: Option<&FoundationMemoryPlan>,
    prewrite: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if foundation_only && memory.is_none() {
        return Err("Foundation factory memory plan/mode differs".into());
    }
    if !foundation_only {
        let installed=INSTALLED.get().ok_or("full typed data arena absent")?;
        if installed.foundation_data_only||installed.apple_event_data_only
            ||!Arc::ptr_eq(&installed.model,&model) {
            return Err("mixed typed data factory outside its full workspace arena".into());
        }
    }
    unsafe { prepare_data_roster_after_arena_gate(model,foundation_only,memory,prewrite,||Ok(())) }
}

unsafe fn prepare_data_roster_after_arena_gate(
    model: Arc<Workspace>,
    foundation_only: bool,
    memory: Option<&FoundationMemoryPlan>,
    prewrite: impl FnOnce() -> Result<()>,
    arena_gate: impl Fn() -> Result<()>,
) -> Result<()> {
    arena_gate()?;
    let generation = model
        .state
        .lock()
        .map_err(|_| "workspace state poisoned")?
        .generation;
    if memory.is_some() {
        super::factory::verify_memory_epoch(&model, generation)?;
    }
    let observed = if foundation_only {
        preflight_foundation_factories()?
    } else {
        preflight_typed_data_factories()?
    };
    let declaration = observed
        .iter()
        .find_map(|(_, method)| match method {
            TypedDataMethodPreflight::OwnedDeclaration { declaration, .. } => {
                Some(declaration.clone())
            }
            _ => None,
        })
        .ok_or("owned NSString declaration missing")?;
    let all_families = [
        (c"NSString", c"RuycastF1OwnedStringSource"),
        (c"NSData", c"RuycastF1OwnedDataSource"),
        (c"NSURL", c"RuycastF1OwnedURLSource"),
        (c"NSNumber", c"RuycastF1OwnedNumberSource"),
        (c"NSArray", c"RuycastF1OwnedArraySource"),
        (c"NSDictionary", c"RuycastF1OwnedDictionarySource"),
        (c"NSPasteboard", c"RuycastF1OwnedPasteboardSource"),
        (c"NSPasteboardItem", c"RuycastF1OwnedPasteboardItemSource"),
    ];
    let families = &all_families[..if foundation_only { 6 } else { 8 }];
    let parents = families
        .iter()
        .map(|(role, _)| {
            crate::sdk::metadata::loaded_class(role)
                .map(|class| unsafe { &*(class.address() as *const AnyClass) })
        })
        .collect::<Result<Vec<_>>>()?;
    let validate = || {
        prewrite()?;
        declaration.verify()
    };
    let guard = unsafe {
        if let Some(memory) = memory {
            super::classes::ClassGuard::select_foundation_memory_family_checked(
                &parents,
                model.lease(),
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                typed_data::select_reference_data_dealloc,
                #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
                |_| Ok(typed_data::dealloc),
                memory,
                validate,
                || {
                    arena_gate()?;
                    super::factory::verify_memory_epoch(&model, generation)
                },
            )
        } else {
            super::classes::ClassGuard::select_sdk_family_checked(
                &parents,
                model.lease(),
                typed_data::dealloc,
                validate,
            )
        }
    }?;
    let mut replacements = foundation::replacements();
    let mut signatures = foundation::signatures();
    if !foundation_only {
        replacements.extend(clipboard_bridge::replacements());
        signatures.extend(clipboard_bridge::signatures());
    }
    for ((role, name), parent) in families.iter().copied().zip(parents) {
        arena_gate()?;
        let immutable= [c"NSString",c"NSData",c"NSURL",c"NSNumber",c"NSArray",c"NSDictionary"].contains(&role);
        let (proxy, arc) = if let Some(memory) = memory.filter(|_|immutable) {
            let (proxy, proof) = unsafe {
                guard.subclass_foundation_memory_qualified(name, parent, role, memory, &model)
            }?;
            (proxy, Some(proof))
        } else {
            (unsafe { guard.subclass_qualified(name, parent) }?, None)
        };
        let mut bindings = guard.lifecycle_bindings_for(parent)?;
        if let Some(arc) = arc {
            bindings.push(super::receiver::LifecycleBinding::owned_memory(arc));
        }
        for (_, selector, meta, implementation) in replacements
            .iter()
            .filter(|(class, _, _, _)| *class == role)
        {
            let signature = signatures
                .iter()
                .find(|(class, sel, is_meta, _)| {
                    *class == role && sel == selector && is_meta == meta
                })
                .ok_or("typed data method signature disappeared")?;
            if role == c"NSString"
                && *selector == super::classes::StringBytesDeclaration::SELECTOR
                && !meta
            {
                let addition = unsafe {
                    guard.add_string_bytes_initializer(
                        proxy,
                        &declaration,
                        foundation::string_bytes_initializer(),
                    )
                }?;
                bindings.push(super::receiver::LifecycleBinding::owned_addition(addition));
            } else {
                let replacement = unsafe {
                    guard.overlay_bound(
                        if *meta { parent } else { proxy },
                        selector,
                        *meta,
                        *implementation,
                        &signature.3,
                    )
                }?;
                bindings.push(super::receiver::LifecycleBinding::leaf(replacement));
            }
        }
        let grant = unsafe {
            super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(), proxy, bindings)
        }?;
        arena_gate()?;
        super::factory::publish_proxy(role, Arc::new(grant))?;
    }
    Ok(())
}
pub(super) fn immutable_payload_for_factory(
    receiver: *mut AnyObject,
    role: &CStr,
    model: &Arc<Workspace>,
) -> Result<bool> {
    if role==c"NSError" {
        return Ok(super::factory::is_immutable_data(receiver,model)?
            && error_payload_for_factory(receiver,model)?);
    }
    let identity = {
        let objects = INSTALLED
            .get()
            .ok_or("workspace absent")?
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        match objects.get(&(receiver as usize)) {
            Some(Kind::Data(identity)) => identity.clone(),
            _ => return Err("not a typed immutable data record".into()),
        }
    };
    if typed_data::role(&identity) != role {
        return Err("immutable payload role differs from qualified class factory".into());
    }
    typed_data::immutable_payload(receiver, &identity, model)
}
fn model() -> Result<&'static Workspace> {
    let s = INSTALLED.get().ok_or("workspace not installed")?;
    s.model.with(|_| Ok(()))?;
    Ok(&s.model)
}
fn kind(p: *mut AnyObject) -> Result<Kind> {
    model()?;
    let value = INSTALLED.get().ok_or("workspace absent")?.objects
        .lock().map_err(|_| "object arena poisoned")?.get(&(p as usize)).cloned()
        .ok_or("unknown/unowned Objective-C receiver")?;
    match value {
        Kind::WindowPayload(identity) => {
            identity.0.check(p)?;
            Ok(Kind::Window(identity.0.handle))
        }
        other => Ok(other),
    }
}

pub(super) fn resignation_window_handle(pointer: usize) -> Result<Option<Handle>> {
    if pointer == 0 {
        return Ok(None);
    }
    match kind(pointer as *mut AnyObject)? {
        Kind::Window(handle) => Ok(Some(handle)),
        _ => Ok(None),
    }
}
fn prepare_window_kind(kind: Kind, model: &Arc<Workspace>) -> Result<Kind> {
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed| installed.fixture_classes) { return Ok(kind); }
    let Kind::Window(handle) = kind else { return Ok(kind); };
    model.window_snapshot(handle)?;
    let identity = {
        let objects = INSTALLED.get().ok_or("workspace absent")?.objects
            .lock().map_err(|_| "object arena poisoned")?;
        objects.values().find_map(|kind| match kind {
            Kind::WindowPayload(identity) if identity.0.handle == handle => Some(identity.clone()),
            _ => None,
        }).ok_or("model window has no initialized receiver allocation")?
    };
    Ok(Kind::WindowPayload(identity))
}

unsafe fn register_receiver(p: *mut AnyObject, k: Kind) -> Result<()> {
    if p.is_null() {
        return Err("null owned receiver".into());
    }
    let model = INSTALLED.get().ok_or("workspace absent")?.model.clone();
    unsafe { super::factory::initialize(p, &model) }?;
    let mut objects = INSTALLED
        .get()
        .unwrap()
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?;
    if objects.insert(p as usize, k).is_some() {
        return Err("receiver initialization replay".into());
    }
    Ok(())
}
fn app(p: *mut AnyObject) -> Result<Application> {
    match kind(p)? {
        Kind::Application(h) => model()?.application(h),
        _ => Err("not an owned application".into()),
    }
}
fn object(k: Kind) -> Result<*mut AnyObject> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if installed.foundation_data_only {
        match &k {
            Kind::Data(identity) => data_role_allowed(typed_data::role(identity))?,
            _ => return Err("non-Foundation receiver in immutable-only arena".into()),
        }
    }
    model()?;
    if installed.apple_event_data_only && !matches!(k, Kind::AppleEventDescriptor(..)) {
        return Err("non-data receiver outside AppleEvent-only installation".into());
    }
    let k = prepare_window_kind(k, &installed.model)?;
    let cached = {
        let objects = installed
            .objects
            .lock()
            .map_err(|_| "object arena poisoned")?;
        objects.iter().find(|(_, v)| **v == k).map(|(p, _)| *p)
    };
    let owns_reference = matches!(
        k,
        Kind::AppleEventDescriptor(..) | Kind::Event(_) | Kind::Data(_) | Kind::NotificationPayload(_) | Kind::BundleIcon(_)
    ) || (matches!(k, Kind::ApplicationShell) && {
        #[cfg(test)]
        {
            !installed.fixture_classes
        }
        #[cfg(not(test))]
        {
            true
        }
    });
    if let Some(pointer) = cached {
        if let Kind::WindowPayload(identity) = &k {
            identity.0.check(pointer as *mut AnyObject)?;
        }
        return if owns_reference {
            unsafe { super::factory::retain_owned(pointer as *mut AnyObject) }
        } else {
            Ok(pointer as *mut AnyObject)
        };
    }
    // Allocation may call a selected constructor or dynamic capability. They
    // may re-enter a cached arena lookup, so no arena lock crosses this call.
    let role = match k {
        Kind::Workspace => c"NSWorkspace",
        Kind::OpenConfiguration => return Err("open configuration requires its owned initializer".into()),
        Kind::BundleIcon(_) => c"NSImage",
        Kind::ApplicationShell => c"NSApplication",
        Kind::Menu(_) => c"NSMenu",
        Kind::MenuItem(_) => c"NSMenuItem",
        Kind::StatusBar => c"NSStatusBar",
        Kind::StatusItem(_) => c"NSStatusItem",
        Kind::Button(_) => c"NSStatusBarButton",
        Kind::Application(_) => c"NSRunningApplication",
        Kind::Screen(_) => c"NSScreen",
        Kind::Window(_) => c"NSWindow",
        Kind::WindowPayload(ref identity) => match installed.model.window_snapshot(identity.0.handle)?.role {
            super::WindowRole::Window => c"NSWindow",
            super::WindowRole::Panel => c"NSPanel",
        },
        Kind::NotificationCenter(_) => c"NSNotificationCenter",
        Kind::NotificationToken(_, _) => c"NSObject",
        Kind::NotificationPayload(_) => c"NSNotification",
        Kind::MainQueue => c"NSOperationQueue",
        Kind::AppleEventDescriptor(_, _, _) => c"NSAppleEventDescriptor",
        Kind::Event(_) => c"NSEvent",
        Kind::Data(ref identity) => typed_data::role(identity),
        Kind::TaskPayload(ref identity) => identity.0.role(),
    };
    #[cfg(test)]
    let p = {
        if !installed.fixture_classes || matches!(k, Kind::Event(_) | Kind::Data(_)) {
            // Actual Host/App and data callers always use their admitted source
            // factories. Only install_fixture opts into ordinary own classes.
            super::factory::proxy(role, &installed.model)?
        } else if matches!(k, Kind::AppleEventDescriptor(..)) {
            super::factory::proxy_class(own_apple_event_class(), &installed.model)?
        } else {
            let retained: Retained<NSObject> = unsafe { objc2::msg_send![own_class(), new] };
            Retained::into_raw(retained).cast::<AnyObject>()
        }
    };
    #[cfg(not(test))]
    let p = super::factory::proxy(role, &installed.model)?;
    #[cfg(test)]
    let actual_factory = !installed.fixture_classes;
    #[cfg(not(test))]
    let actual_factory = true;
    // Non-data singleton/collection children also return a genuine factory +1.
    // Keep its failure/reentrant cleanup obligation until arena publication.
    let pending = if owns_reference || actual_factory {
        Some(unsafe { super::factory::OwnedReceiver::from_owned(p) }?)
    } else { None };
    #[cfg(all(rcf1_reference_sdk, target_arch = "aarch64"))]
    if actual_factory {
        if let Kind::Application(handle) = &k {
            installed.model.application(*handle)?;
            super::factory::publish_running_application_reference(p, &installed.model)?;
        }
    }
    if let Kind::WindowPayload(identity) = &k {
        identity.0.serial.store(super::factory::owned_reference_serial(p)?, Ordering::Release);
        identity.0.ready.store(true, Ordering::Release);
    }
    installed.model.with(|_| Ok(()))?;
    let mut objects = installed
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?;
    if let Some((existing, _)) = objects.iter().find(|(_, v)| **v == k) {
        let existing = *existing as *mut AnyObject;
        drop(objects);
        // A data-factory call returns a fresh +1 even on reentrant identity
        // publication; its unused allocation is reclaimed by the owned guard.
        return if owns_reference {
            unsafe { super::factory::retain_owned(existing) }
        } else {
            Ok(existing)
        };
    }
    if objects.contains_key(&(p as usize)) {
        return Err("factory reused a receiver with another kind".into());
    }
    objects.insert(p as usize, k);
    if let Some(pending) = pending {
        pending.into_raw();
    }
    Ok(p)
}
pub(crate) fn verify_status_item_cleanup(pointer:*mut AnyObject,model:&Arc<super::Workspace>)->Result<()> {
    let installed=INSTALLED.get().ok_or("status-item arena absent")?;
    if !Arc::ptr_eq(&installed.model,model)||installed.foundation_data_only||installed.apple_event_data_only {
        return Err("status-item cleanup outside its full workspace arena".into());
    }
    if qualified_arena_reference_role(pointer)?!=c"NSStatusItem" {return Err("status-item has another arena role".into());}
    super::factory::verify_status_item_cleanup(pointer,model)?;
    if qualified_arena_reference_role(pointer)?!=c"NSStatusItem" {return Err("status-item arena role changed during cleanup check".into());}
    Ok(())
}
fn qualified_arena_reference_role(pointer: *mut AnyObject) -> Result<&'static CStr> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    let value = kind(pointer)?;
    let expected = match &value {
        Kind::Workspace => c"NSWorkspace",
        Kind::OpenConfiguration => c"NSWorkspaceOpenConfiguration",
        Kind::BundleIcon(_) => c"NSImage",
        Kind::ApplicationShell => c"NSApplication",
        Kind::Menu(_) => c"NSMenu",
        Kind::MenuItem(_) => c"NSMenuItem",
        Kind::StatusBar => c"NSStatusBar",
        Kind::StatusItem(_) => c"NSStatusItem",
        Kind::Button(_) => c"NSStatusBarButton",
        Kind::Application(_) => c"NSRunningApplication",
        Kind::Screen(_) => c"NSScreen",
        Kind::Window(handle) => match installed.model.window_snapshot(*handle)?.role {
            super::WindowRole::Window => c"NSWindow",
            super::WindowRole::Panel => c"NSPanel",
        },
        Kind::NotificationCenter(_) => c"NSNotificationCenter",
        Kind::NotificationToken(_, _) => c"NSObject",
        Kind::NotificationPayload(_) => c"NSNotification",
        Kind::MainQueue => c"NSOperationQueue",
        Kind::AppleEventDescriptor(..) => c"NSAppleEventDescriptor",
        Kind::Event(_) => c"NSEvent",
        Kind::Data(identity) => typed_data::role(identity),
        Kind::WindowPayload(_) => return Err("unprojected internal window payload".into()),
        Kind::TaskPayload(value) if value.0.role()==c"NSError" => {
            task_error_fields(pointer)?;
            c"NSError"
        },
        Kind::TaskPayload(_) => return Err("mutable Task children require their specific reference capability".into()),
    };
    data_role_allowed(expected)?;
    if super::factory::receiver_role(pointer, &installed.model)? != expected
        || kind(pointer)? != value {
        return Err("collection child arena/factory role changed".into());
    }
    Ok(expected)
}
fn string(value: String) -> *mut AnyObject {
    if deny_foundation_only_factory().is_err() { return std::ptr::null_mut(); }
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed| installed.fixture_classes) {
        return Retained::autorelease_return(NSString::from_str(&value)).cast();
    }
    (|| -> Result<_> {
        let text = foundation::Text::from_utf8(value.as_bytes())?;
        let pointer = new_foundation_value(foundation::FoundationValue::String(Arc::new(text)))?;
        // Exactly the fresh factory +1 is consumed on success or failure.
        unsafe { autorelease_foundation(pointer) }
    })().unwrap_or(std::ptr::null_mut())
}
fn array(handles: impl IntoIterator<Item = Kind>) -> Result<*mut AnyObject> {
    deny_foundation_only_factory()?;
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed| installed.fixture_classes) {
        let mut values = Vec::new();
        for kind in handles { values.push(object(kind)?); }
        let borrowed = values.iter().map(|pointer| unsafe { &**pointer }).collect::<Vec<_>>();
        return Ok(Retained::autorelease_return(NSArray::from_slice(&borrowed)).cast());
    }
    let mut values = Vec::new();
    for kind in handles { values.push(foundation_reference(object(kind)?)?); }
    let pointer = new_foundation_array(values)?;
    unsafe { autorelease_foundation(pointer) }
}

fn get_string(p: *mut AnyObject) -> Result<String> {
    #[cfg(test)]
    if INSTALLED
        .get()
        .is_some_and(|s| s.fixture_classes || s.apple_event_data_only)
    {
        if p.is_null() {
            return Err("null fixture string".into());
        }
        let value = unsafe { &*p.cast::<NSString>() };
        if !value.isKindOfClass(NSString::class()) {
            return Err("nonstring fixture argument".into());
        }
        return Ok(value.to_string());
    }
    String::from_utf16(&foundation_utf16(p)?)
        .map_err(|_| "text argument is not valid Unicode".into())
}
fn url(p: *mut AnyObject) -> Result<String> {
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|s| s.fixture_classes) {
        if p.is_null() {
            return Err("null fixture URL".into());
        }
        let value = unsafe { &*p.cast::<NSURL>() };
        if !value.isKindOfClass(NSURL::class()) {
            return Err("non-URL fixture argument".into());
        }
        return Ok(value
            .absoluteString()
            .ok_or("missing absolute URL")?
            .to_string());
    }
    String::from_utf16(foundation_url(p)?.string().utf16())
        .map_err(|_| "URL argument is not valid Unicode".into())
}
fn install_arena(model: Arc<Workspace>, apple_event_data_only: bool) -> Result<()> {
    install_arena_mode(model, apple_event_data_only, false)
}
fn install_arena_mode(
    model: Arc<Workspace>,
    apple_event_data_only: bool,
    fixture_classes: bool,
) -> Result<()> {
    install_arena_policy(model, apple_event_data_only, fixture_classes, false)
}
fn install_arena_policy(
    model: Arc<Workspace>,
    apple_event_data_only: bool,
    _fixture_classes: bool,
    foundation_data_only: bool,
) -> Result<()> {
    // Reject every duplicate role/model before any dynamic lease or auxiliary
    // state creation. OnceLock::set remains the final concurrent-race guard.
    if INSTALLED.get().is_some() {
        return Err("workspace already installed".into());
    }
    model.lease.before_effect()?;
    {
        let state = model.state.lock().map_err(|_| "workspace state poisoned")?;
        if state.retired || model.faulted.load(Ordering::Acquire) {
            return Err("workspace retired before arena publication".into());
        }
    }
    let notifications = notification_bridge::Centers::new(model.clone())?;
    let typed_data = typed_data::State::new(&model)?;
    INSTALLED
        .set(Installed {
            model,
            apple_event_data_only,
            foundation_data_only,
            #[cfg(test)]
            fixture_classes: _fixture_classes,
            objects: Mutex::new(BTreeMap::new()),
            notifications,
            typed_data,
            assembly: OnceLock::new(),
            clipboard_constants: OnceLock::new(),
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            palette_host_release: OnceLock::new(),
        })
        .map_err(|_| "workspace already installed".to_string())
}
/// Install only the shared receiver arena for independently admitted AE data.
/// # Safety
/// Caller holds the independently reviewed data-family admission for this exact
/// model lease. This grants no controls, application shell, NSApp or Host route.
pub unsafe fn install_apple_event_data(model: Arc<Workspace>) -> Result<()> {
    install_arena(model, true)
}
/// Same sole arena, with no App/Window/Event/AE/Board/Item receiver authority.
/// # Safety
/// The independently reviewed FoundationData admission owns this model lease.
pub unsafe fn install_foundation_data(model: Arc<Workspace>) -> Result<()> {
    install_arena_policy(model, false, false, true)
}
pub(super) fn data_role_allowed(role: &CStr) -> Result<()> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if installed.apple_event_data_only
        || (installed.foundation_data_only
            && ![
                c"NSString",
                c"NSData",
                c"NSURL",
                c"NSNumber",
                c"NSArray",
                c"NSDictionary",
            ]
            .contains(&role))
    {
        return Err("typed role outside installed data scope".into());
    }
    Ok(())
}
pub fn install(model: Arc<Workspace>) -> Result<()> {
    let retained = model.clone();
    install_arena(model, false)?;
    controls::initialize(retained.clone())?;
    application::initialize(retained)
}
/// Explicit own-class fixture, never used by a source-qualified Host/App caller.
#[cfg(test)]
pub fn install_fixture(model: Arc<Workspace>) -> Result<()> {
    let retained = model.clone();
    install_arena_mode(model, false, true)?;
    controls::initialize(retained.clone())?;
    application::initialize(retained)
}
unsafe extern "C" fn shared(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    object(Kind::Workspace).unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn front(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (move || {
        if kind(p)? != Kind::Workspace {
            return Err("unowned workspace".into());
        }
        model()?
            .frontmost()?
            .map(|h| object(Kind::Application(h)))
            .transpose()
            .map(|p| p.unwrap_or(std::ptr::null_mut()))
    })()
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn running(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (move || {
        if kind(p)? != Kind::Workspace {
            return Err("unowned workspace".into());
        }
        array(
            model()?
                .applications()?
                .into_iter()
                .filter(|(_, a)| !a.terminated)
                .map(|(h, _)| Kind::Application(h)),
        )
    })()
    .unwrap_or(std::ptr::null_mut())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
thread_local! {
    static SAFARI_DISCOVERY_PROBE: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    static SAFARI_DISCOVERY_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) unsafe fn report_safari_discovery_selector(receiver: *mut AnyObject, selector: Sel) {
    use std::io::Write;
    if receiver.is_null() || !SAFARI_DISCOVERY_ACTIVE.with(std::cell::Cell::get) {
        return;
    }
    let Some(class) = (unsafe { objc2::ffi::object_getClass(receiver).as_ref() }) else { return };
    let (class, selector) = (class.name(), selector.name());
    let clean = |value: &CStr| value.to_str().is_ok_and(|name| {
        name.len() < 96 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_.$:".contains(&byte))
    });
    if clean(class) && clean(selector) {
        let packet = serde_json::json!({
            "reference_safari_discovery_refused_class": class.to_string_lossy(),
            "selector": selector.to_string_lossy(),
        });
        let _ = writeln!(std::io::stdout(), "{packet}");
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_original_safari_discovery(_: usize, _: *mut [usize; 4]) {
    core::arch::naked_asm!(
        "stp x19, x30, [sp, #-16]!",
        "mov x19, x1", "blr x0",
        "stp x0, x1, [x19]", "stp x2, x3, [x19, #16]",
        "ldp x19, x30, [sp], #16", "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn probe_reference_safari_discovery(
    admission: &crate::sdk::core_preentry::ReferenceAdmission,
    model: &Arc<Workspace>,
) -> Result<()> {
    let identity = crate::sdk::process_router::reference_browser_identity(
        model, crate::sdk::process_router::REFERENCE_SAFARI_PID,
    )?;
    let class = crate::sdk::metadata::loaded_class(c"NSRunningApplication")?;
    let (entry, caller) = admission.reference_safari_discovery_probe(class.address())?;
    let method = crate::sdk::metadata::method(class, c"runningApplicationsWithBundleIdentifier:", true)?
        .ok_or("Reference Safari discovery method absent")?;
    if method.implementation() != by_bundle as *const () as usize {
        return Err("Reference Safari discovery method is not private".into());
    }
    SAFARI_DISCOVERY_PROBE.with(|slot| {
        if slot.get().is_some() {
            return Err("Reference Safari discovery probe reentered".to_owned());
        }
        slot.set(Some((caller, class.address())));
        Ok(())
    })?;
    let mut value = [0usize; 4];
    SAFARI_DISCOVERY_ACTIVE.with(|slot| slot.set(true));
    unsafe { call_original_safari_discovery(entry, &mut value) };
    SAFARI_DISCOVERY_ACTIVE.with(|slot| slot.set(false));
    let used = SAFARI_DISCOVERY_PROBE.with(std::cell::Cell::take).is_none();
    if !used || value[3] as u32 != 0
        || value[0] as u32 as i32 != identity.pid || (value[0] >> 32) as u32 != identity.uid
        || value[1] as u64 != identity.seconds || value[2] as u64 != identity.micros
    {
        return Err("Reference original Safari discovery identity differs".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn by_bundle(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> *mut AnyObject {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym by_bundle_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn by_bundle(class: *mut AnyObject, selector: Sel, bundle: *mut AnyObject) -> *mut AnyObject {
    unsafe { by_bundle_checked(class, selector, bundle, 0) }
}

unsafe extern "C" fn by_bundle_checked(
    _receiver: *mut AnyObject,
    _: Sel,
    bundle: *mut AnyObject,
    caller: usize,
) -> *mut AnyObject {
    (move || -> Result<*mut AnyObject> {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if crate::sdk::core_preentry::ReferenceAdmission::is_reference_safari_discovery(caller) {
            let installed = INSTALLED.get().ok_or("Reference workspace absent")?;
            const SAFARI: &str = "com.apple.Safari";
            if SAFARI_DISCOVERY_PROBE.with(std::cell::Cell::get).is_some() {
                if SAFARI_DISCOVERY_PROBE.with(std::cell::Cell::take) != Some((caller, _receiver as usize))
                    || unsafe { libc::pthread_main_np() } != 1
                    || unsafe { task_objects::reference_string(bundle)? } != SAFARI
                {
                    return Err("Reference Safari discovery probe permit differs".into());
                }
                let expected = installed.model.by_pid(crate::sdk::process_router::REFERENCE_SAFARI_PID)?
                    .ok_or("Reference Safari fixture absent")?;
                let found = installed.model.by_bundle(SAFARI)?;
                if found.as_slice() != [expected] {
                    return Err("Reference Safari discovery fixture differs".into());
                }
                return array([Kind::Application(expected)]);
            }
            let stopped = crate::sdk::reference_runtime::verify_safari_discovery(&installed.model, caller)?;
            if stopped.load(Ordering::Acquire) || !crate::workspace::callbacks::reference_safari_dispatch_active() {
                return array(std::iter::empty());
            }
            let name = match get_string(bundle) {
                Ok(name) => name,
                Err(_) => unsafe { task_objects::reference_string(bundle)? },
            };
            if name != SAFARI {
                return Err("Reference Safari bundle identifier differs".into());
            }
            let result = array(installed.model.by_bundle(SAFARI)?.into_iter().map(Kind::Application))?;
            if stopped.load(Ordering::Acquire) || !crate::workspace::callbacks::reference_safari_dispatch_active() {
                return array(std::iter::empty());
            }
            return Ok(result);
        }
        let name = match get_string(bundle) {
            Ok(name) => name,
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Err(_) => {
                let installed = INSTALLED.get().ok_or("Reference workspace absent")?;
                // Both original keyboard refresh callers bridge the same
                // static Swift literal. Keep that separate from Safari.
                const RAYCAST: &str = "com.raycast.macos";
                let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
                    &installed.model, caller, &[0xf8224, 0x1adcc8],
                );
                check()?;
                if unsafe { task_objects::reference_string(bundle)? } != RAYCAST {
                    return Err("Reference keyboard bundle identifier differs".into());
                }
                check()?;
                RAYCAST.to_owned()
            }
            #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
            Err(error) => return Err(error),
        };
        let _ = caller;
        array(model()?.by_bundle(&name)?.into_iter().map(Kind::Application))
    })()
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn by_pid(_: *mut AnyObject, _: Sel, pid: i32) -> *mut AnyObject {
    (move || {
        model()?
            .by_pid(pid)?
            .map(|h| object(Kind::Application(h)))
            .transpose()
            .map(|p| p.unwrap_or(std::ptr::null_mut()))
    })()
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn current_application(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (move || {
        model()?
            .current_application()?
            .map(|h| object(Kind::Application(h)))
            .transpose()
            .map(|p| p.unwrap_or(std::ptr::null_mut()))
    })()
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn pid(p: *mut AnyObject, _: Sel) -> i32 {
    app(p).map_or(0, |a| a.pid)
}
unsafe extern "C" fn bundle(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    app(p)
        .ok()
        .and_then(|a| a.bundle)
        .map(string)
        .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn name(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    app(p)
        .ok()
        .and_then(|a| a.name)
        .map(string)
        .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn terminated(p: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(app(p).map_or(true, |a| a.terminated))
}
unsafe extern "C" fn policy(p: *mut AnyObject, _: Sel) -> isize {
    app(p).map_or(2, |a| if a.regular { 0 } else { 1 })
}
fn actor_parent_reference(receiver:*mut AnyObject)->Result<Option<FoundationReference>> {
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed|installed.fixture_classes) {return Ok(None);}
    Ok(Some(foundation_reference(receiver)?))
}
fn returned_actor_file_url(receiver:*mut AnyObject,parent:&Option<FoundationReference>,
    path:&std::path::Path,directory:bool)->Result<*mut AnyObject> {
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed|installed.fixture_classes)&&parent.is_none() {
        return Ok(Retained::autorelease_return(NSURL::fileURLWithPath_isDirectory(
            &NSString::from_str(&path.to_string_lossy()),directory)).cast());
    }
    let parent=parent.as_ref().ok_or("actual actor URL parent reference absent")?;
    if parent.as_ptr()?!=receiver {return Err("actor URL parent changed before creation".into());}
    // The bounded owned fixture contract supplies absolute UTF8 paths. Bundle
    // locations are directories; executable locations are files, without stat.
    let text=Text::from_utf8(path.to_str().ok_or("owned actor path is not UTF8")?.as_bytes())?;
    let value=foundation::Url::file_path(&text,directory)?;
    let pointer=new_foundation_value(FoundationValue::Url(Arc::new(value)))?;
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(pointer)}?;
    if parent.as_ptr()?!=receiver {return Err("actor URL parent changed during creation".into());}
    // Transfer exactly one fresh +1 to autorelease_foundation's own guard.
    let result=unsafe {autorelease_foundation(pending.into_raw())}?;
    if parent.as_ptr()?!=receiver {return Err("actor URL parent changed during return".into());}
    Ok(result)
}
unsafe extern "C" fn app_url(p:*mut AnyObject,_:Sel)->*mut AnyObject {
    (||->Result<_>{
        let parent=actor_parent_reference(p)?;
        let application=app(p)?;
        #[cfg(rcf1_reference_sdk)]
        let path=super::reference_bundle_metadata_path(&application);
        #[cfg(not(rcf1_reference_sdk))]
        let path=application.bundle_path;
        let Some(path)=path else {return Ok(std::ptr::null_mut());};
        returned_actor_file_url(p,&parent,&path,true)
    })().unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn executable(p:*mut AnyObject,_:Sel)->*mut AnyObject {
    (||->Result<_>{
        let parent=actor_parent_reference(p)?;
        let Some(path)=app(p)?.executable_path else {return Ok(std::ptr::null_mut());};
        returned_actor_file_url(p,&parent,&path,false)
    })().unwrap_or(std::ptr::null_mut())
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn activate(p: *mut AnyObject, selector: Sel, options: usize) -> Bool {
    unsafe { activate_checked(p, selector, options, 0) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn activate(_: *mut AnyObject, _: Sel, _: usize) -> Bool {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym activate_checked);
}

unsafe extern "C" fn activate_checked(p: *mut AnyObject, _selector: Sel, _options: usize, _caller: usize) -> Bool {
    Bool::new(
        (move || {
            let Kind::Application(handle) = kind(p)? else {
                return Err("unowned activation".into());
            };
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if crate::sdk::core_preentry::ReferenceAdmission::is_reference_safari_activation(_caller) {
                let installed = INSTALLED.get().ok_or("Reference workspace absent")?;
                return apple_events::activate_reference_safari(&installed.model, handle, _caller, _options);
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if crate::sdk::core_preentry::ReferenceAdmission::is_reference_ax_activation(_caller) {
                let installed = INSTALLED.get().ok_or("Reference workspace absent")?;
                return crate::sdk::reference_runtime::activate_ax_target(
                    &installed.model, handle, _caller, _selector, _options,
                );
            }
            model()?.activate(handle)
        })().unwrap_or(false),
    )
}
unsafe extern "C" fn app_lookup(
    p: *mut AnyObject,
    _: Sel,
    bundle: *mut AnyObject,
) -> *mut AnyObject {
    (move || -> Result<_> {
        let parent=actor_parent_reference(p)?;
        if kind(p)? != Kind::Workspace {
            return Err("unowned workspace".into());
        }
        let a = model()?
            .installed_application(&get_string(bundle)?)?
            .ok_or("unknown installed app")?;
        returned_actor_file_url(p,&parent,&a.bundle_path.ok_or("missing owned bundle URL")?,true)
    })()
    .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn open(p: *mut AnyObject, _: Sel, value: *mut AnyObject) -> Bool {
    Bool::new(
        (move || -> Result<_> {
            if kind(p)? != Kind::Workspace {
                return Err("unowned workspace".into());
            }
            Ok(model()?.open(&url(value)?)? == Outcome::Success)
        })()
        .unwrap_or(false),
    )
}
unsafe extern "C" fn workspace_icon(pointer: *mut AnyObject, _: Sel, path: *mut AnyObject) -> *mut AnyObject {
    (|| -> Result<_> {
        let parent = actor_parent_reference(pointer)?;
        if kind(pointer)? != Kind::Workspace {
            return Err("foreign workspace icon receiver".into());
        }
        let path = get_string(path)?;
        let icon = model()?.icon_for_owned_bundle(std::path::Path::new(&path))?;
        if let Some(parent) = parent {
            if parent.as_ptr()? != pointer {
                return Err("workspace icon parent changed".into());
            }
        }
        let pointer = object(Kind::BundleIcon(IconIdentity(icon)))?;
        let pending = unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?;
        let result = unsafe { super::factory::autorelease_owned(pointer) }?;
        pending.into_raw();
        Ok(result)
    })().unwrap_or(std::ptr::null_mut())
}

fn image_record(pointer: *mut AnyObject) -> Result<Arc<super::images::IconPixels>> {
    if qualified_arena_reference_role(pointer)? != c"NSImage" {
        return Err("foreign image receiver".into());
    }
    match kind(pointer)? {
        Kind::BundleIcon(icon) => Ok(icon.0),
        _ => Err("image payload absent".into()),
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn reference_open_application(
    _: *mut AnyObject, _: Sel, _: *mut AnyObject, _: *mut AnyObject, _: *mut super::callbacks::OpenReply,
) {
    core::arch::naked_asm!("mov x5,x30", "b {checked}", checked = sym reference_open_application_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_open_application_checked(
    pointer: *mut AnyObject, selector: Sel, target: *mut AnyObject, config: *mut AnyObject,
    block: *mut super::callbacks::OpenReply, return_pc: usize,
) {
    reference_open_checked(pointer, selector, target, config, block, return_pc, None);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn reference_open_urls(
    _: *mut AnyObject, _: Sel, _: *mut AnyObject, _: *mut AnyObject, _: *mut AnyObject,
    _: *mut super::callbacks::OpenReply,
) {
    core::arch::naked_asm!("mov x6,x30", "b {checked}", checked = sym reference_open_urls_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn reference_open_urls_checked(
    pointer: *mut AnyObject, selector: Sel, urls: *mut AnyObject, target: *mut AnyObject,
    config: *mut AnyObject, block: *mut super::callbacks::OpenReply, return_pc: usize,
) {
    reference_open_checked(pointer, selector, target, config, block, return_pc, Some(urls));
}

/// The verified Swift caller owns this borrowed array through return. Each
/// caller supplies its exact expected count and validates the borrowed element.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe fn reference_swift_array_element(
    array: *mut AnyObject, expected_count: isize, index: isize,
) -> Result<*mut AnyObject> {
    use objc2::encode::{Encode, Encoding};
    let array = unsafe { array.as_ref() }.ok_or("Reference open URL array absent")?;
    let class = array.class();
    if class.name() != c"Swift.__SwiftDeferredNSArray" {
        return Err("Reference open URL array class differs".into());
    }
    let handle = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
    let select = |selector, offset, encoding: &[Encoding]| -> Result<_> {
        let method = crate::sdk::metadata::method(handle, selector, false)?
            .ok_or("Reference deferred array method absent")?;
        let code = method.code();
        if code.image() != b"/usr/lib/swift/libswiftCore.dylib" || code.offset() != offset
            || code.uuid() != [0x83, 0x79, 0x4f, 0xb3, 0xde, 0x9b, 0x3d, 0x23,
                0xab, 0x5e, 0x2c, 0x1d, 0x5d, 0x30, 0xf1, 0x34]
        {
            return Err("Reference deferred array implementation differs".into());
        }
        super::classes::verify_typed_abi(&method, encoding)?;
        Ok(method.implementation())
    };
    // The Swift class uses signed Int encodings for both values.
    let count = select(c"count", 0x1a3e8, &[isize::ENCODING, Encoding::Object, Sel::ENCODING])?;
    let count: unsafe extern "C-unwind" fn(&AnyObject, Sel) -> isize =
        unsafe { std::mem::transmute(count) };
    if !(0..expected_count).contains(&index)
        || unsafe { count(array, Sel::register(c"count")) } != expected_count
    {
        return Err("Reference deferred array count or index differs".into());
    }
    let element = select(c"objectAtIndex:", 0x35a618,
        &[Encoding::Object, Encoding::Object, Sel::ENCODING, isize::ENCODING])?;
    let element: unsafe extern "C-unwind" fn(&AnyObject, Sel, isize) -> *mut AnyObject =
        unsafe { std::mem::transmute(element) };
    Ok(unsafe { element(array, Sel::register(c"objectAtIndex:"), index) })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
// Safety: the pinned Swift caller must hold this bridge result through decode.
unsafe fn reference_open_url(pointer: *mut AnyObject) -> Result<foundation::Url> {
    use objc2_core_foundation::{CFRange, CFString, CFType};

    if let Ok(url) = foundation_url(pointer) {
        return Ok(url);
    }
    let pointer = std::ptr::NonNull::new(pointer).ok_or("Reference open URL absent")?;
    let class = crate::sdk::metadata::loaded_class(c"NSURL")?;
    // The pinned Swift call owns its bridge result until this selector returns.
    if unsafe { objc2::ffi::object_getClass(pointer.as_ptr()) } as usize != class.address() {
        return Err("Reference open bridge URL class differs".into());
    }
    let selector = c"absoluteString";
    let method = crate::sdk::metadata::method(class, selector, false)?
        .ok_or("Reference open NSURL string method absent")?;
    let code = method.code();
    if method.owner() != class.address() || method.encoding() != b"@16@0:8"
        || code.image() != b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
        || code.offset() != 0x36324
        || code.uuid() != [0x91, 0xda, 0xce, 0x39, 0xfa, 0x28, 0x31, 0x91,
            0x81, 0x8d, 0x1f, 0xcc, 0x6a, 0x0e, 0x61, 0x5a]
    {
        return Err("Reference open NSURL string method differs".into());
    }
    let read: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *const CFType =
        unsafe { std::mem::transmute(method.implementation()) };
    let text = unsafe { read(pointer.as_ptr(), Sel::register(selector)).as_ref() }
        .and_then(CFType::downcast_ref::<CFString>)
        .ok_or("Reference open NSURL did not return a string")?;
    let length = text.length();
    if !(1..=8192).contains(&length) {
        return Err("Reference open URL length differs".into());
    }
    let mut units = vec![0; length as usize];
    unsafe { text.characters(CFRange { location: 0, length }, units.as_mut_ptr()) };
    if units.contains(&0) {
        return Err("Reference open URL contains NUL".into());
    }
    let text = Text::new(units);
    let url = foundation::Url::parse(&text)?;
    if url.string() != &text {
        return Err("Reference open URL canonical string differs".into());
    }
    Ok(url)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn reference_open_checked(
    pointer: *mut AnyObject, selector: Sel, target: *mut AnyObject, config: *mut AnyObject,
    block: *mut super::callbacks::OpenReply, return_pc: usize, urls: Option<*mut AnyObject>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        let installed = INSTALLED.get().ok_or("Reference open workspace absent")?;
        let (admission, owner) = crate::sdk::reference_runtime::open_application_admission(&installed.model)?;
        let opens_url = urls.is_some();
        let capture = unsafe { admission.verify_open_block(return_pc, block as usize, opens_url) }?;
        if capture.1[3] != owner {
            return Err("Reference Open finish capture has another AppState owner".into());
        }

        let expected = if opens_url {
            c"openURLs:withApplicationAtURL:configuration:completionHandler:"
        } else {
            c"openApplicationAtURL:configuration:completionHandler:"
        };
        if selector != Sel::register(expected)
            || qualified_arena_reference_role(pointer)? != c"NSWorkspace"
            || qualified_arena_reference_role(config)? != c"NSWorkspaceOpenConfiguration"
        {
            return Err("Reference open receiver, selector or configuration differs".into());
        }
        let config = foundation_reference(config)?;
        let target = unsafe { reference_open_url(target) }?;
        if !target.is_file() || target.host().is_some_and(|host| !host.utf16().is_empty()) {
            return Err("Reference open application requires a local file URL".into());
        }
        let path = String::from_utf16(target.path().ok_or("Reference application URL path absent")?.utf16())
            .map_err(|_| "Reference application URL path encoding")?;
        let url = match urls {
            Some(array) => unsafe { reference_open_url(reference_swift_array_element(array, 1, 0)?) }?,
            None => target,
        };
        let url = String::from_utf16(url.string().utf16()).map_err(|_| "Reference open URL encoding")?;
        config.as_ptr()?;
        let engine = application::engine()?;
        if !Arc::ptr_eq(engine.model(), &installed.model) {
            return Err("Reference open loop model differs".into());
        }
        if unsafe { admission.verify_open_block(return_pc, block as usize, opens_url) }? != capture {
            return Err("Reference open capture changed before retain".into());
        }
        let copied = unsafe { block2::RcBlock::copy(block) }.ok_or("Reference open block retain failed")?;
        struct PendingOpenBlock(Option<usize>);
        impl Drop for PendingOpenBlock {
            fn drop(&mut self) {
                if self.0.is_some() {
                    super::callbacks::mark_reference_open_cleanup_unconfirmed();
                }
            }
        }
        let copied = block2::RcBlock::into_raw(copied) as usize;
        let mut retained = PendingOpenBlock(Some(copied));
        if copied != block as usize
            || !unsafe { admission.verify_open_block(return_pc, copied, opens_url) }.is_ok_and(|actual| actual == capture)
        {
            return Err("Reference open retained block identity differs".into());
        }
        let result = engine.enqueue_reference_open(std::path::Path::new(&path), &url, opens_url, |app, completion| {
            Ok(super::callbacks::ReferenceOpenReply {
                model: installed.model.clone(), admission: admission.clone(), return_pc, opens_url, capture,
                block: std::sync::atomic::AtomicUsize::new(retained.0.take().ok_or("retained Reference block absent")?),
                app, completion: completion.clone(),
            })
        });
        if let Some(address) = retained.0 {
            if unsafe { admission.verify_open_block(return_pc, address, opens_url) }.is_ok_and(|actual| actual == capture) {
                drop(unsafe { block2::RcBlock::from_raw(address as *mut super::callbacks::OpenReply) });
                retained.0 = None;
            } else if result.is_ok() {
                return Err("Reference cancelled Open block could not be qualified for release".into());
            }
        }
        result
    }));
    if !matches!(&result, Ok(Ok(()))) {
        if let Err(payload) = result { std::mem::forget(payload); }
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn reference_open_result(
    model: &Arc<Workspace>, app: Handle, outcome: Outcome,
) -> Result<(Option<super::factory::OwnedReceiver>, Option<super::factory::OwnedReceiver>)> {
    let installed = INSTALLED.get().ok_or("Reference open result workspace absent")?;
    if !Arc::ptr_eq(model, &installed.model) {
        return Err("Reference open result model differs".into());
    }
    match outcome {
        Outcome::Success => {
            if model.application(app)?.terminated {
                return Err("Reference open result application retired".into());
            }
            let pointer = object(Kind::Application(app))?;
            if qualified_arena_reference_role(pointer)? != c"NSRunningApplication" {
                return Err("Reference open result application role differs".into());
            }
            let retained = unsafe { super::factory::retain_owned(pointer) }?;
            Ok((Some(unsafe { super::factory::OwnedReceiver::from_owned(retained) }?), None))
        }
        Outcome::Failure => {
            let pointer = new_task_error_fields(TaskErrorValue {
                domain: "RuycastOwnedWorkspace".into(), code: 1,
                description: "The owned application could not be opened.".into(),
                user_info: Default::default(),
            })?;
            Ok((None, Some(unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?)))
        }
        Outcome::Unknown => Err("unknown Reference open outcome cannot be delivered".into()),
    }
}
unsafe extern "C" fn workspace_image_size(pointer: *mut AnyObject, _: Sel) -> NSSize {
    image_record(pointer).map_or(NSSize::new(0.0, 0.0), |image| {
        NSSize::new(image.width as f64, image.height as f64)
    })
}
unsafe extern "C" fn workspace_image_tiff(pointer: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (|| -> Result<_> {
        let bytes = image_record(pointer)?.tiff()?;
        let data = new_foundation_value(FoundationValue::Data(Arc::new(foundation::Data::new(bytes))))?;
        unsafe { autorelease_foundation(data) }
    })().unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn screens(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    (move || array((0..model()?.screens()?.len()).map(Kind::Screen)))()
        .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn main_screen(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    object(Kind::Screen(0)).unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn mouse(_: *mut AnyObject, _: Sel) -> NSPoint {
    model()
        .and_then(Workspace::mouse)
        .map_or(NSPoint::new(0.0, 0.0), |p| NSPoint::new(p.x, p.y))
}
fn rect(r: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(r.origin.x, r.origin.y),
        NSSize::new(r.size.x, r.size.y),
    )
}
fn screen(p: *mut AnyObject) -> Result<Screen> {
    match kind(p)? {
        Kind::Screen(i) => model()?
            .screens()?
            .get(i)
            .cloned()
            .ok_or("unknown screen".into()),
        _ => Err("not an owned screen".into()),
    }
}
unsafe extern "C" fn frame(p: *mut AnyObject, _: Sel) -> NSRect {
    screen(p)
        .map(|s| s.frame)
        .or_else(|_| -> Result<_> {
            match kind(p)? {
                Kind::Window(h) => Ok(model()?.window(h)?.0),
                _ => Err("unknown geometry receiver".into()),
            }
        })
        .map_or(NSRect::ZERO, rect)
}
unsafe extern "C" fn visible_frame(p: *mut AnyObject, _: Sel) -> NSRect {
    screen(p).map_or(NSRect::ZERO, |s| rect(s.visible))
}
unsafe extern "C" fn scale(p: *mut AnyObject, _: Sel) -> f64 {
    screen(p).map_or(0.0, |s| s.scale)
}
unsafe extern "C" fn order(p: *mut AnyObject, sel: Sel, _sender: *mut AnyObject) {
    let action = if sel == objc2::sel!(orderOut:) {
        WindowOrder::Out
    } else if sel == objc2::sel!(close) {
        WindowOrder::Close
    } else if sel == objc2::sel!(makeKeyAndOrderFront:) || sel == objc2::sel!(makeKeyWindow) {
        WindowOrder::Key
    } else {
        WindowOrder::Front
    };
    window_boundary((), || {
        let receiver = INSTALLED.get().ok_or("workspace absent")?.objects
            .lock().map_err(|_| "object arena poisoned")?.get(&(p as usize)).cloned();
        match receiver {
            Some(Kind::Window(h)) => apply_window_order(model()?, h, action, || Ok(())),
            Some(Kind::WindowPayload(_)) => {
                let record = window_identity(p)?;
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                if record.origin == WindowOrigin::ReferencePalette && action == WindowOrder::Key {
                    let appearance = record.palette_appearance.lock().map_err(|_| "Palette appearance poisoned")?;
                    if appearance.title.as_deref() != Some("Ruycast")
                        || !appearance.clear_background
                        || appearance.opaque != Some(false)
                        || appearance.shadow != Some(true)
                        || appearance.movable_by_background != Some(true)
                        || appearance.released_when_closed != Some(false)
                        || appearance.hides_on_deactivate != Some(false)
                        || appearance.level != Some(3)
                        || appearance.collection_behavior != Some(0x101)
                    {
                        return Err("original PalettePanel appearance was not configured".into());
                    }
                }
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                if record.origin == WindowOrigin::ReferenceSnippetChoice && action == WindowOrder::Key {
                    let appearance = record.palette_appearance.lock().map_err(|_| "SnippetChoice appearance poisoned")?;
                    if !appearance.clear_background || appearance.opaque != Some(false)
                        || appearance.shadow != Some(true) || appearance.released_when_closed != Some(false)
                        || appearance.level != Some(3) || appearance.collection_behavior != Some(0x101)
                    {
                        return Err("original SnippetChoicePanel appearance was not configured".into());
                    }
                    drop(appearance);
                    let content = record.references.lock().map_err(|_| "window references poisoned")?
                        .content.clone().ok_or("SnippetChoice host absent before presentation")?;
                    let WindowReference::Palette(host) = content.as_ref() else {
                        return Err("SnippetChoice host ownership differs".into());
                    };
                    host.verify()?;
                }
                apply_window_order(&record.model, record.handle, action, || record.check(p))
            }
            _ => Err("unknown window".into()),
        }
    });
}
fn apply_window_order(
    model: &Workspace, handle: Handle, order: WindowOrder, verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let resigned = model.window_order_with_resignation(handle, order)?;
    verify()?;
    if let Some(resigned) = resigned {
        notification_bridge::window_resigned_key(model, resigned)?;
    }
    Ok(())
}
unsafe extern "C" fn order_noarg(p: *mut AnyObject, sel: Sel) {
    unsafe { order(p, sel, std::ptr::null_mut()) }
}
unsafe extern "C" fn visible(p: *mut AnyObject, _: Sel) -> Bool {
    window_boundary(Bool::new(false), || {
        let value = match kind(p)? {
            Kind::Window(h) => model()?.window(h)?.1,
            Kind::WindowPayload(_) => {
                let record = window_identity(p)?;
                let value = record.model.window_snapshot(record.handle)?.visible;
                record.check(p)?;
                value
            }
            _ => return Err("unknown window".into()),
        };
        Ok(Bool::new(value))
    })
}
/// Complete exported subset. Remaining source leaves are deliberately not
/// claimed qualified: see unresolved_roster. Integrator must refuse scope-ready.
// Exact existing function types, derived with MethodImplementation; no original calls.
// Concrete private notification-token family. No public NSObject alias.
unsafe extern "C" fn token_parent_initialize(_: *mut AnyClass,_:Sel) {}
unsafe extern "C" fn token_parent_init(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {pointer}
unsafe extern "C" fn token_parent_construct(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    if !(pointer as usize).is_multiple_of(std::mem::align_of::<usize>()) {return std::ptr::null_mut();}
    pointer
}
unsafe extern "C" fn token_parent_destruct(pointer:*mut AnyObject,_:Sel) {
    if pointer.is_null()||!(pointer as usize).is_multiple_of(std::mem::align_of::<usize>()) {unsafe {libc::_exit(78)}}
}
unsafe extern "C" fn token_parent_dealloc(pointer:*mut AnyObject,selector:Sel) {
    if pointer.is_null()||!(pointer as usize).is_multiple_of(std::mem::align_of::<usize>()) {unsafe {libc::_exit(78)}}
    unsafe {owned_notification_token_dealloc(pointer,selector)}
}
/// # Safety
/// Exclusive source-qualified setup of the one internal token parent. prewrite
/// verifies actual SDK/source custody; this cannot admit a public NSObject call.
pub unsafe fn prepare_notification_token_factory_checked(
    model:Arc<Workspace>,prewrite:impl Fn()->Result<()>,
)->Result<()> {
    deny_foundation_only_factory()?;
    model.with(|_|Ok(()))?;
    prewrite()?;
    let root=crate::sdk::metadata::loaded_class(c"NSObject")?;
    let root=unsafe {&*(root.address() as *const AnyClass)};
    let mut builder=objc2::runtime::ClassBuilder::new(c"RuycastF1OwnedNotificationTokenParent",root)
        .ok_or("notification token parent already exists")?;
    unsafe {
        builder.add_class_method(objc2::sel!(initialize),token_parent_initialize as unsafe extern "C" fn(*mut AnyClass,Sel));
        builder.add_method(objc2::sel!(init),token_parent_init as unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject);
        builder.add_method(Sel::register(c".cxx_construct"),token_parent_construct as unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject);
        builder.add_method(Sel::register(c".cxx_destruct"),token_parent_destruct as unsafe extern "C" fn(*mut AnyObject,Sel));
        builder.add_method(objc2::sel!(dealloc),token_parent_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel));
    }
    model.with(|_|Ok(()))?;
    prewrite()?;
    let parent=builder.register();
    // No original NSObject constructor is selected or invoked. Its methods
    // remain untouched; this fresh parent owns the four typed own bodies.
    super::factory::preflight_default_proxy_arc(parent)?;
    let guard=unsafe {super::classes::ClassGuard::select_sdk_family_checked(
        &[parent],model.lease(),owned_notification_token_dealloc,prewrite)}?;
    let proxy=unsafe {guard.subclass_qualified(c"RuycastF1OwnedNotificationTokenSource",parent)}?;
    let bindings=guard.lifecycle_bindings_for(parent)?;
    let grant=unsafe {super::receiver::WorkspaceGrant::for_owned_sdk(model,proxy,bindings)}?;
    super::factory::publish_proxy(c"NSObject",Arc::new(grant))
}
pub(super) unsafe extern "C" fn owned_notification_token_dealloc(pointer:*mut AnyObject,_:Sel) {
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        let installed=INSTALLED.get().ok_or("workspace absent during token cleanup")?;
        super::factory::verify_notification_token_cleanup(pointer,&installed.model)?;
        let serial=super::factory::owned_reference_serial(pointer)?;
        let expected={
            let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            match objects.get(&(pointer as usize)) {
                None=>None,
                Some(Kind::NotificationToken(center,id))=>Some((*center,*id)),
                Some(_)=>return Err("notification token cleanup kind differs".into()),
            }
        };
        if super::factory::owned_reference_serial(pointer)?!=serial {return Err("token cleanup serial changed".into());}
        super::factory::deallocate_owned(pointer,|| {
            let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            let same=match (objects.get(&(pointer as usize)),expected) {
                (None,None)=>true,
                (Some(Kind::NotificationToken(center,id)),Some((before_center,before_id)))=>
                    *center==before_center&&*id==before_id,
                _=>false,
            };
            if !same {return Err("notification token changed during cleanup".into());}
            objects.remove(&(pointer as usize));
            Ok(())
        })
    }));
    if !matches!(result,Ok(Ok(()))) {
        if let Err(payload)=result {std::mem::forget(payload);}
        if let Some(installed)=INSTALLED.get(){installed.model.fault();}
    }
}

fn running_screen_signatures() -> Vec<(&'static CStr, &'static CStr, bool, Vec<objc2::encode::Encoding>)> {
    fn typed<F: objc2::runtime::MethodImplementation>(_: F) -> Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
        let mut result=vec![F::Return::ENCODING_RETURN,Encoding::Object,Sel::ENCODING];
        result.extend_from_slice(F::Arguments::ENCODINGS);
        result
    }
    macro_rules! row { ($role:expr,$selector:expr,$meta:expr,$method:ident,$ty:ty) => {
        ($role,$selector,$meta,typed($method as $ty))
    }; }
    vec![
        row!(c"NSRunningApplication",c"runningApplicationsWithBundleIdentifier:",true,by_bundle,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        row!(c"NSRunningApplication",c"runningApplicationWithProcessIdentifier:",true,by_pid,unsafe extern "C" fn(*mut AnyObject,Sel,i32)->*mut AnyObject),
        row!(c"NSRunningApplication",c"currentApplication",true,current_application,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSRunningApplication",c"processIdentifier",false,pid,unsafe extern "C" fn(*mut AnyObject,Sel)->i32),
        row!(c"NSRunningApplication",c"bundleIdentifier",false,bundle,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSRunningApplication",c"localizedName",false,name,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSRunningApplication",c"bundleURL",false,app_url,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSRunningApplication",c"executableURL",false,executable,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSRunningApplication",c"activationPolicy",false,policy,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        row!(c"NSRunningApplication",c"isTerminated",false,terminated,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        row!(c"NSRunningApplication",c"activateWithOptions:",false,activate,unsafe extern "C" fn(*mut AnyObject,Sel,usize)->Bool),
        row!(c"NSScreen",c"screens",true,screens,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSScreen",c"mainScreen",true,main_screen,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        row!(c"NSScreen",c"frame",false,frame,unsafe extern "C" fn(*mut AnyObject,Sel)->NSRect),
        row!(c"NSScreen",c"visibleFrame",false,visible_frame,unsafe extern "C" fn(*mut AnyObject,Sel)->NSRect),
        row!(c"NSScreen",c"backingScaleFactor",false,scale,unsafe extern "C" fn(*mut AnyObject,Sel)->f64),
    ]
}


/// # Safety
/// Exclusive source-qualified setup under the same source-image/prewrite
/// authority as existing typed family preparation. This publishes only these
/// two actual families; full workspace assembly remains separate.
pub unsafe fn prepare_running_and_screen_factories_checked(
    model: Arc<Workspace>, prewrite: impl Fn() -> Result<()>,
) -> Result<()> {
    deny_foundation_only_factory()?;
    let roles=[(c"NSRunningApplication",c"RuycastF1OwnedRunningApplicationSource"),
        (c"NSScreen",c"RuycastF1OwnedScreenSource")];
    let signatures=running_screen_signatures();
    let selected=replacements().into_iter().filter(|(role,_,_,_)|
        roles.iter().any(|(wanted,_)|wanted==role)).collect::<Vec<_>>();
    if selected.len()!=16 || signatures.len()!=16 {return Err("running/screen fixed roster differs".into());}
    let mut keys=std::collections::BTreeSet::new();
    let mut parents=Vec::new();
    for (role,_) in roles {
        let loaded=crate::sdk::metadata::loaded_class(role)?;
        let parent=unsafe { &*(loaded.address() as *const AnyClass) };
        if parent.superclass() != Some(objc2_foundation::NSObject::class()) {
            return Err("running/screen lifecycle ancestry differs".into());
        }
        super::factory::preflight_default_proxy_arc(parent)?;
        parents.push(parent);
    }
    // Complete typed/raw read preflight precedes every ClassGuard write.
    for (role,selector,meta,_) in &selected {
        if !keys.insert((*role,*selector,*meta)) {return Err("duplicate running/screen row".into());}
        let matching=signatures.iter().filter(|(r,s,m,_)|r==role&&s==selector&&m==meta).collect::<Vec<_>>();
        if matching.len()!=1 {return Err("running/screen signature is not unique".into());}
        let loaded=crate::sdk::metadata::loaded_class(role)?;
        let method=crate::sdk::metadata::method(loaded,selector,*meta)?.ok_or("running/screen source method absent")?;
        super::classes::verify_typed_abi(&method,&matching[0].3)?;
    }
    let handlers = [
        owned_running_application_dealloc as unsafe extern "C" fn(*mut AnyObject, Sel),
        owned_screen_dealloc as unsafe extern "C" fn(*mut AnyObject, Sel),
    ];
    for (((role, name), parent), deallocator) in roles.into_iter().zip(parents).zip(handlers) {
        // Safari discovers applications on the original worker. Only the
        // running-application lifecycle can first initialize there.
        let lease = if cfg!(rcf1_reference_sdk) && role == c"NSRunningApplication" {
            workspace_factory_lease_with_owner(&model, None)?
        } else {
            workspace_factory_lease(&model)?
        };
        let guard = unsafe {
            super::classes::ClassGuard::select_sdk_family_checked(
                &[parent], lease, deallocator, &prewrite,
            )
        }?;
        let proxy=unsafe {guard.subclass_qualified(name,parent)}?;
        let mut bindings=guard.lifecycle_bindings_for(parent)?;
        for (row_role,selector,meta,implementation) in &selected {
            if *row_role!=role {continue;}
            let signature=signatures.iter().find(|(r,s,m,_)|r==row_role&&s==selector&&m==meta)
                .ok_or("running/screen signature disappeared")?;
            let proof=unsafe {guard.overlay_bound(if *meta {parent}else{proxy},selector,
                *meta,*implementation,&signature.3)}?;
            bindings.push(super::receiver::LifecycleBinding::leaf(proof));
        }
        let grant=unsafe {super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(),proxy,bindings)}?;
        super::factory::publish_proxy(role,Arc::new(grant))?;
    }
    Ok(())
}
pub(super) unsafe extern "C" fn owned_running_application_dealloc(pointer:*mut AnyObject,_:Sel) {
    owned_running_screen_cleanup(pointer,c"NSRunningApplication");
}
pub(super) unsafe extern "C" fn owned_screen_dealloc(pointer:*mut AnyObject,_:Sel) {
    owned_running_screen_cleanup(pointer,c"NSScreen");
}
fn owned_running_screen_cleanup(pointer:*mut AnyObject,role:&CStr) {
    let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        let installed=INSTALLED.get().ok_or("workspace absent during running/screen cleanup")?;
        if super::factory::running_screen_cleanup_role(pointer,&installed.model)?!=role {
            return Err("running/screen cleanup actual handler family differs".into());
        }
        let serial=super::factory::owned_reference_serial(pointer)?;
        let expected={
            let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            match objects.get(&(pointer as usize)) {
                None=>None,
                Some(Kind::Application(handle)) if role==c"NSRunningApplication"=>Some(Kind::Application(*handle)),
                Some(Kind::Screen(index)) if role==c"NSScreen"=>Some(Kind::Screen(*index)),
                Some(_)=>return Err("running/screen cleanup payload differs".into()),
            }
        };
        if super::factory::owned_reference_serial(pointer)?!=serial {
            return Err("running/screen cleanup allocation changed".into());
        }
        super::factory::deallocate_owned(pointer,|| {
            let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            if objects.get(&(pointer as usize))!=expected.as_ref() {
                return Err("running/screen cleanup identity changed".into());
            }
            objects.remove(&(pointer as usize));
            Ok(())
        })
    }));
    let failed=match outcome {Ok(result)=>result.is_err(),Err(payload)=>{std::mem::forget(payload);true}};
    if failed {if let Some(installed)=INSTALLED.get(){installed.model.fault();}}
}

/// Prepare the Workspace, open configuration, notification-center and main-queue
/// leaves. Async open/icon leaves remain separate required assembly inputs.
/// # Safety
/// The caller owns the exclusive source-qualified preentry transaction.
pub unsafe fn prepare_notification_and_workspace_factories_checked(
    model:Arc<Workspace>,prewrite:impl FnOnce()->Result<()>,
)->Result<()> {
    deny_foundation_only_factory()?;
    let observed=notification_bridge::preflight()?;
    let mut signatures=notification_bridge::signatures();
    fn typed<F:objc2::runtime::MethodImplementation>(_:F)->Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode,EncodeArguments,EncodeReturn,Encoding};
        let mut result=vec![F::Return::ENCODING_RETURN,Encoding::Object,Sel::ENCODING];
        result.extend_from_slice(F::Arguments::ENCODINGS);
        result
    }
    macro_rules! workspace_row {($selector:expr,$meta:expr,$handler:ident,$ty:ty)=>{
        (c"NSWorkspace",$selector,$meta,typed($handler as $ty))
    };}
    signatures.extend([
        workspace_row!(c"sharedWorkspace",true,shared,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        workspace_row!(c"frontmostApplication",false,front,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        workspace_row!(c"runningApplications",false,running,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        workspace_row!(c"URLForApplicationWithBundleIdentifier:",false,app_lookup,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        workspace_row!(c"openURL:",false,open,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->Bool),
        workspace_row!(c"iconForFile:",false,workspace_icon,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        workspace_row!(c"openApplicationAtURL:configuration:completionHandler:",false,reference_open_application,
            unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut super::callbacks::OpenReply)),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        workspace_row!(c"openURLs:withApplicationAtURL:configuration:completionHandler:",false,reference_open_urls,
            unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut super::callbacks::OpenReply)),
        (c"NSImage", c"size", false, typed(workspace_image_size as unsafe extern "C" fn(*mut AnyObject, Sel) -> NSSize)),
        (c"NSImage", c"TIFFRepresentation", false,
            typed(workspace_image_tiff as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject)),
        (c"NSWorkspaceOpenConfiguration", c"init", false,
            typed(init_open_configuration as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject)),
    ]);
    let roles=[(c"NSWorkspace",c"RuycastF1OwnedWorkspaceSource"),
        (c"NSNotificationCenter",c"RuycastF1OwnedNotificationCenterSource"),
        (c"NSOperationQueue",c"RuycastF1OwnedMainQueueSource"),
        (c"NSWorkspaceOpenConfiguration",c"RuycastF1OwnedOpenConfigurationSource"),
        (c"NSImage",c"RuycastF1OwnedBundleIconSource")];
    let selected=replacements().into_iter().filter(|(role,_,_,_)|
        roles.iter().any(|(wanted,_)|wanted==role)).collect::<Vec<_>>();
    let expected = 18 + 2 * usize::from(cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")));
    if selected.len()!=expected||signatures.len()!=expected||observed.len()!=9 {
        return Err("workspace/notification fixed roster differs".into());
    }
    let mut seen=std::collections::BTreeSet::new();
    let mut parents=Vec::new();
    for (role,_) in roles {
        let source=crate::sdk::metadata::loaded_class(role)?;
        let parent=unsafe {&*(source.address() as *const AnyClass)};
        super::factory::preflight_default_proxy_arc(parent)?;
        parents.push(parent);
    }
    for (role,selector,meta,implementation) in &selected {
        if !seen.insert((*role,*selector,*meta)) {return Err("duplicate workspace/notification row".into());}
        let matching=signatures.iter().filter(|(r,s,m,_)|r==role&&s==selector&&m==meta).collect::<Vec<_>>();
        if matching.len()!=1 {return Err("workspace/notification signature differs".into());}
        let source=crate::sdk::metadata::method(crate::sdk::metadata::loaded_class(role)?,selector,*meta)?
            .ok_or("workspace/notification source method absent")?;
        super::classes::verify_typed_abi(&source,&matching[0].3)?;
        if let Some(before)=observed.iter().find(|row|row.class==*role&&row.selector==*selector&&row.class_method==*meta) {
            if before.source!=source||before.replacement!=crate::sdk::metadata::code_identity(*implementation)? {
                return Err("notification preflight source/code changed".into());
            }
        }
    }
    let handlers=[(parents[0],owned_workspace_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel)),
        (parents[1],owned_notification_center_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel)),
        (parents[2],owned_main_queue_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel)),
        (parents[3],owned_open_configuration_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel)),
        (parents[4],owned_bundle_icon_dealloc as unsafe extern "C" fn(*mut AnyObject,Sel))];
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    {
        let original = super::raw_method(parents[1], objc2::sel!(defaultCenter), true)?
            .ok_or("Reference original notification center absent")?;
        notification_bridge::bind_platform_center(parents[1], original)?;
        let original = super::raw_method(parents[2], objc2::sel!(mainQueue), true)?
            .ok_or("Reference original main queue absent")?;
        notification_bridge::bind_platform_main_queue(parents[2], original)?;
        for (parent, slot, framework) in [
            (parents[4], &PLATFORM_IMAGE_DEALLOC, b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".as_slice()),
            (parents[2], &PLATFORM_QUEUE_DEALLOC, b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation".as_slice()),
        ] {
            let class = crate::sdk::metadata::loaded_class(parent.name())?;
            let original = crate::sdk::metadata::method(class, c"dealloc", false)?
                .ok_or("Reference original website deallocator absent")?;
            if class.address() != parent as *const AnyClass as usize
                || original.owner() != class.address() || original.encoding() != b"v16@0:8"
                || original.code().image() != framework
            {
                return Err("Reference original website deallocator differs".into());
            }
            slot.set((class, original))
                .map_err(|_| "Reference original website deallocator already bound")?;
        }
    }
    let guard=unsafe {super::classes::ClassGuard::select_sdk_joint_family_checked(
        &parents,workspace_factory_lease(&model)?,&handlers,prewrite,false)}?;
    for ((role,name),parent) in roles.into_iter().zip(parents) {
        let proxy=unsafe {guard.subclass_qualified(name,parent)}?;
        let mut bindings=guard.lifecycle_bindings_for(parent)?;
        for (row_role,selector,meta,implementation) in &selected {
            if *row_role!=role {continue;}
            let signature=signatures.iter().find(|(r,s,m,_)|r==row_role&&s==selector&&m==meta)
                .ok_or("workspace/notification signature disappeared")?;
            let proof=unsafe {guard.overlay_bound(if *meta {parent}else{proxy},selector,
                *meta,*implementation,&signature.3)}?;
            bindings.push(super::receiver::LifecycleBinding::leaf(proof));
        }
        let grant=unsafe {super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(),proxy,bindings)}?;
        super::factory::publish_proxy(role,Arc::new(grant))?;
    }
    Ok(())
}
pub(super) unsafe extern "C" fn owned_workspace_dealloc(pointer:*mut AnyObject,_:Sel) {
    owned_notification_family_cleanup(pointer,c"NSWorkspace");
}
pub(super) unsafe extern "C" fn owned_notification_center_dealloc(pointer:*mut AnyObject,_:Sel) {
    owned_notification_family_cleanup(pointer,c"NSNotificationCenter");
}
pub(super) unsafe extern "C" fn owned_main_queue_dealloc(pointer:*mut AnyObject,_selector:Sel) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if unsafe { deallocate_website_platform_object(pointer, _selector, c"NSOperationQueue", &PLATFORM_QUEUE_DEALLOC) } {
        return;
    }
    owned_notification_family_cleanup(pointer,c"NSOperationQueue");
}
unsafe extern "C" fn init_open_configuration(pointer: *mut AnyObject, _: Sel) -> *mut AnyObject {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<*mut AnyObject> {
        let installed = INSTALLED.get().ok_or("workspace absent during configuration init")?;
        super::factory::claim_owned_role(
            pointer, c"NSWorkspaceOpenConfiguration", &installed.model,
            owned_open_configuration_dealloc as *const () as usize,
        )?;
        let pending = unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?;
        unsafe { register_receiver(pointer, Kind::OpenConfiguration) }?;
        Ok(pending.into_raw())
    }));
    match result {
        Ok(result) => result.unwrap_or(std::ptr::null_mut()),
        Err(payload) => {
            std::mem::forget(payload);
            if let Some(installed) = INSTALLED.get() {
                installed.model.fault();
            }
            std::ptr::null_mut()
        }
    }
}
pub(super) unsafe extern "C" fn owned_open_configuration_dealloc(pointer: *mut AnyObject, _: Sel) {
    owned_notification_family_cleanup(pointer, c"NSWorkspaceOpenConfiguration");
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
type WebsiteDeallocator = OnceLock<(crate::sdk::metadata::ClassHandle, crate::sdk::metadata::MethodSnapshot)>;
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_IMAGE_DEALLOC: WebsiteDeallocator = OnceLock::new();
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_QUEUE_DEALLOC: WebsiteDeallocator = OnceLock::new();

pub(super) unsafe extern "C" fn owned_bundle_icon_dealloc(pointer: *mut AnyObject, _selector: Sel) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if unsafe { deallocate_website_platform_object(pointer, _selector, c"NSImage", &PLATFORM_IMAGE_DEALLOC) } {
        return;
    }
    owned_notification_family_cleanup(pointer, c"NSImage");
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe fn deallocate_website_platform_object(
    pointer: *mut AnyObject, selector: Sel, role: &CStr, slot: &WebsiteDeallocator,
) -> bool {
    if crate::sdk::private_network::take_assembly_object(role, pointer as usize) {
        let Some((class, original)) = slot.get() else {
            unsafe { libc::_exit(89) }
        };
        if selector != objc2::sel!(dealloc)
            || unsafe { objc2::ffi::object_getClass(pointer) } as usize != class.address()
            || !crate::sdk::metadata::code_identity(original.implementation()).is_ok_and(|code| code == *original.code())
        {
            unsafe { libc::_exit(89) }
        }
        let dealloc: unsafe extern "C" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(original.implementation()) };
        unsafe { dealloc(pointer, selector) };
        return true;
    }
    false
}
fn owned_notification_family_cleanup(pointer:*mut AnyObject,role:&CStr) {
    let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        let installed=INSTALLED.get().ok_or("workspace absent during notification cleanup")?;
        if super::factory::notification_family_cleanup_role(pointer,&installed.model)?!=role {
            return Err("notification cleanup actual handler differs".into());
        }
        let serial=super::factory::owned_reference_serial(pointer)?;
        let expected={
            let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            match objects.get(&(pointer as usize)) {
                None=>None,
                Some(Kind::Workspace) if role==c"NSWorkspace"=>Some(Kind::Workspace),
                Some(Kind::NotificationCenter(id)) if role==c"NSNotificationCenter"=>Some(Kind::NotificationCenter(*id)),
                Some(Kind::MainQueue) if role==c"NSOperationQueue"=>Some(Kind::MainQueue),
                Some(Kind::OpenConfiguration) if role==c"NSWorkspaceOpenConfiguration"=>Some(Kind::OpenConfiguration),
                Some(Kind::BundleIcon(icon)) if role==c"NSImage"=>Some(Kind::BundleIcon(icon.clone())),
                Some(_)=>return Err("notification cleanup payload differs".into()),
            }
        };
        if super::factory::owned_reference_serial(pointer)?!=serial {return Err("notification cleanup allocation changed".into());}
        super::factory::deallocate_owned(pointer,|| {
            let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            if objects.get(&(pointer as usize))!=expected.as_ref() {return Err("notification cleanup identity changed".into());}
            objects.remove(&(pointer as usize));
            Ok(())
        })
    }));
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if !matches!(&outcome, Ok(Ok(())))
        && crate::sdk::private_network::reference_website_assembly_active()
    {
        crate::sdk::private_network::report_reference_website_selector_refusal(role, c"dealloc");
        if !pointer.is_null() && outcome.as_ref().is_ok_and(|result| {
            result.as_ref().is_err_and(|reason| reason == "unknown SDK allocation receiver")
        }) {
            if let Some(class) = unsafe { objc2::ffi::object_getClass(pointer).as_ref() } {
                crate::sdk::private_network::report_reference_website_selector_refusal(
                    class.name(), c"unregisteredDealloc",
                );
            }
        }
    }
    let failed=match outcome {Ok(result)=>result.is_err(),Err(payload)=>{std::mem::forget(payload);true}};
    if failed {if let Some(installed)=INSTALLED.get(){installed.model.fault();}}
}

fn notification_record_pure(pointer:*mut AnyObject,record:&Arc<NotificationRecord>)->Result<()> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(&record.model,&installed.model)||record.owner!=std::thread::current().id()
        || !record.ready.load(Ordering::Acquire)||record.model.faulted.load(Ordering::Acquire) {
        return Err("notification payload retired or foreign".into());
    }
    {
        let state=record.model.state.lock().map_err(|_|"workspace poisoned")?;
        if state.retired||state.generation!=record.generation {return Err("notification generation retired".into());}
    }
    let serial=record.serial.load(Ordering::Acquire);
    if serial==0||super::factory::owned_reference_serial(pointer)?!=serial {
        return Err("notification allocation serial changed".into());
    }
    let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
    if !matches!(objects.get(&(pointer as usize)),Some(Kind::NotificationPayload(actual)) if Arc::ptr_eq(&actual.0,record)) {
        return Err("notification arena identity changed".into());
    }
    Ok(())
}
fn notification_record(pointer:*mut AnyObject)->Result<Arc<NotificationRecord>> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    let record={
        let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
        match objects.get(&(pointer as usize)) {
            Some(Kind::NotificationPayload(identity))=>identity.0.clone(),
            _=>return Err("not an owned notification payload".into()),
        }
    };
    notification_record_pure(pointer,&record)?;
    let identity=super::factory::receiver_identity(pointer)?;
    if !identity.belongs_to(&record.model)||identity.allocation_serial()!=record.serial.load(Ordering::Acquire) {
        return Err("notification physical identity differs".into());
    }
    identity.verify()?;
    notification_record_pure(pointer,&record)?;
    Ok(record)
}

/// Capture the existing payload record and allocation identity. The returned
/// fixed closure performs only metadata/ledger/model checks, never a guardian
/// or source callback. The caller separately holds the actual payload +1.
pub(crate) fn notification_payload_pure_check(
    pointer:*mut AnyObject,model:&Arc<Workspace>,
)->Result<impl Fn()->Result<()>> {
    let record=notification_record(pointer)?;
    let identity=super::factory::receiver_identity(pointer)?;
    if !Arc::ptr_eq(&record.model,model)||!identity.belongs_to(model)||identity.original_source()
        ||identity.allocation_serial()!=record.serial.load(Ordering::Acquire) {
        return Err("Notification pure check belongs to another payload/model".into());
    }
    notification_record_pure(pointer,&record)?;
    Ok(move || {
        identity.verify_pure()?;
        notification_record_pure(pointer,&record)?;
        if unsafe{&*pointer}.class()!=identity.class(){return Err("Notification physical class changed before callback".into());}
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        if let Some(field)=record.reference_field.get(){field.object()?;}
        identity.verify_pure()
    })
}
/// Fresh +1 payload, held by the existing owner-thread synchronous Hub pump.
/// All children are actual same-arena references, never PID/name substitutes.
#[cfg(test)]
pub(crate) fn notification_fixture_payload(model:&Arc<Workspace>)->bool {
    INSTALLED.get().is_none_or(|installed|
        Arc::ptr_eq(&installed.model,model)&&installed.fixture_classes)
}
pub(crate) fn new_owned_notification(
    model:&Arc<Workspace>,name:&str,object:Option<*mut AnyObject>,user_info:Option<*mut AnyObject>,
)->Result<super::factory::OwnedReceiver> {
    if !INSTALLED.get().is_some_and(|installed|Arc::ptr_eq(&installed.model,model)) {
        return Err("foreign notification model before name allocation".into());
    }
    model.with(|_|Ok(()))?;
    let is_window_resignation = name == "NSWindowDidResignKeyNotification";
    let text=new_foundation_value(FoundationValue::String(Arc::new(Text::from_utf8(name.as_bytes())?)))?;
    let text_pending=unsafe {super::factory::OwnedReceiver::from_owned(text)}?;
    let name=foundation_reference(text)?;
    drop(text_pending);
    new_owned_notification_with_name(model,name,object,user_info, is_window_resignation)
}
fn new_owned_notification_with_name(
    model:&Arc<Workspace>,name:FoundationReference,object:Option<*mut AnyObject>,user_info:Option<*mut AnyObject>,
    is_window_resignation: bool,
)->Result<super::factory::OwnedReceiver> {
    let installed=INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(&installed.model,model) {return Err("foreign notification model".into());}
    let generation=model.with(|state|Ok(state.generation))?;
    let (object, window_object) = if is_window_resignation {
        let window = object.ok_or("key-window resignation requires a window object")?;
        (None, Some(WindowReference::acquire(window, c"NSWindow")?))
    } else {
        (object.map(foundation_reference).transpose()?, None)
    };
    let user_info=user_info.map(|pointer| -> Result<FoundationReference> {
        let held=foundation_reference(pointer)?;
        if held.as_ptr()?!=pointer {return Err("notification userInfo identity differs".into());}
        foundation_dictionary_entries(held.as_ptr()?)?;
        if held.as_ptr()?!=pointer {return Err("notification userInfo changed during validation".into());}
        Ok(held)
    }).transpose()?;
    // All dynamic child checks and allocation happen outside the arena mutex.
    name.as_ptr()?;
    if let Some(window) = &window_object { window.pointer()?; }
    for child in [&object,&user_info].into_iter().flatten() {child.as_ptr()?;}
    let pointer=super::factory::proxy(c"NSNotification",model)?;
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(pointer)}?;
    let serial=super::factory::owned_reference_serial(pointer)?;
    let record=Arc::new(NotificationRecord {
        model:model.clone(),generation,owner:std::thread::current().id(),
        serial:AtomicU64::new(serial),ready:AtomicBool::new(true),name,object,window_object,user_info,
        #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
        reference_field:OnceLock::new(),
    });
    model.with(|state|if state.generation==generation {Ok(())}else{Err("notification generation changed".into())})?;
    for child in [Some(&record.name),record.object.as_ref(),record.user_info.as_ref()].into_iter().flatten() {child.as_ptr()?;}
    if let Some(window) = &record.window_object { window.pointer()?; }
    {
        let state=model.state.lock().map_err(|_|"workspace poisoned")?;
        if state.retired||state.generation!=generation||model.faulted.load(Ordering::Acquire) {
            return Err("notification generation retired before publication".into());
        }
        let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
        if objects.contains_key(&(pointer as usize)) {return Err("notification factory reused arena pointer".into());}
        objects.insert(pointer as usize,Kind::NotificationPayload(NotificationIdentity(record.clone())));
    }
    notification_record_pure(pointer,&record)?;
    Ok(pending)
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
pub(crate) fn new_reference_field_notification(model:&Arc<Workspace>,field:super::receiver::app_delegate::constructor::ReferenceFieldToken)->Result<super::factory::OwnedReceiver>{
    field.verify_model(model)?;
    let notice=new_owned_notification(model,"NSControlTextDidChangeNotification",None,None)?;
    let record=notification_record(notice.as_ptr())?;
    if record.object.is_some()||record.window_object.is_some()||record.user_info.is_some()||!Arc::ptr_eq(&record.model,model){return Err("Reference field notification envelope differs".into());}
    field.verify_model(model)?;
    record.reference_field.set(field).map_err(|_|"Reference field notification already bound")?;
    notification_record_pure(notice.as_ptr(),&record)?;
    record.reference_field.get().ok_or("Reference field token absent")?.object()?;
    Ok(notice)
}
fn notification_child(pointer:*mut AnyObject,which:u8)->Result<*mut AnyObject> {
    let record=notification_record(pointer)?;
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    if which==1 {
        if let Some(field)=record.reference_field.get(){
            notification_record_pure(pointer,&record)?;
            return field.object();
        }
    }
    if which == 1 {
        if let Some(window) = &record.window_object {
            notification_record_pure(pointer, &record)?;
            return window.returned(c"NSWindow");
        }
    }
    let child=match which {0=>Some(&record.name),1=>record.object.as_ref(),2=>record.user_info.as_ref(),_=>return Err("unknown notification field".into())};
    let Some(child)=child else {notification_record_pure(pointer,&record)?;return Ok(std::ptr::null_mut());};
    let held=unsafe {super::factory::retain_owned(child.as_ptr()?)}?;
    let pending=unsafe {super::factory::OwnedReceiver::from_owned(held)}?;
    notification_record_pure(pointer,&record)?;
    let result=unsafe {super::factory::autorelease_owned(held)}?;
    pending.into_raw();
    notification_record_pure(pointer,&record)?;
    Ok(result)
}
fn notification_payload_boundary(body:impl FnOnce()->Result<*mut AnyObject>)->*mut AnyObject {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Ok(value))=>return value,
        Err(payload)=>std::mem::forget(payload),
        Ok(Err(_))=>(),
    }
    if let Some(installed)=INSTALLED.get(){installed.model.fault();}
    std::ptr::null_mut()
}
unsafe extern "C" fn notification_name(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    notification_payload_boundary(||notification_child(pointer,0))
}
unsafe extern "C" fn notification_object(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    notification_payload_boundary(||notification_child(pointer,1))
}
unsafe extern "C" fn notification_user_info(pointer:*mut AnyObject,_:Sel)->*mut AnyObject {
    notification_payload_boundary(||notification_child(pointer,2))
}
unsafe extern "C" fn notification_create(class: *mut AnyClass,_:Sel,name:*mut AnyObject,object:*mut AnyObject,info:*mut AnyObject)->*mut AnyObject {
    notification_payload_boundary(|| {
        let model=INSTALLED.get().ok_or("workspace absent")?.model.clone();
        if !super::factory::class_role_matches(class,c"NSNotification",&model)? {
            return Err("notification class receiver differs".into());
        }
        let held_name=foundation_reference(name)?;
        get_string(name)?;
        if held_name.as_ptr()?!=name {return Err("notification name identity changed".into());}
        let pending=new_owned_notification_with_name(&model,held_name,(!object.is_null()).then_some(object),(!info.is_null()).then_some(info),false)?;
        let pointer=pending.as_ptr();
        let record=notification_record(pointer)?;
        let result=unsafe {super::factory::autorelease_owned(pointer)}?;
        pending.into_raw();
        notification_record_pure(pointer,&record)?;
        Ok(result)
    })
}

fn notification_payload_signatures()->Vec<(&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
    fn typed<F:objc2::runtime::MethodImplementation>(_:F)->Vec<objc2::encode::Encoding> {
        use objc2::encode::{Encode,EncodeArguments,EncodeReturn,Encoding};
        let mut result=vec![F::Return::ENCODING_RETURN,Encoding::Object,Sel::ENCODING];
        result.extend_from_slice(F::Arguments::ENCODINGS);result
    }
    vec![
        (c"notificationWithName:object:userInfo:",true,typed(notification_create as unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject,*mut AnyObject,*mut AnyObject)->*mut AnyObject)),
        (c"name",false,typed(notification_name as unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject)),
        (c"object",false,typed(notification_object as unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject)),
        (c"userInfo",false,typed(notification_user_info as unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject)),
    ]
}
/// # Safety
/// Exclusive source-qualified preentry; typed notification payload methods and
/// complete original parent memory/initializer paths are independently bound.
pub unsafe fn prepare_notification_payload_factory_checked(
    model:Arc<Workspace>,prewrite:impl FnOnce()->Result<()>,
)->Result<()> {
    deny_foundation_only_factory()?;
    let loaded=crate::sdk::metadata::loaded_class(c"NSNotification")?;
    let parent=unsafe {&*(loaded.address() as *const AnyClass)};
    super::factory::preflight_default_proxy_arc(parent)?;
    let signatures=notification_payload_signatures();
    let rows=replacements().into_iter().filter(|(role,_,_,_)|*role==c"NSNotification").collect::<Vec<_>>();
    if signatures.len()!=4||rows.len()!=4 {return Err("notification payload roster differs".into());}
    let mut seen=std::collections::BTreeSet::new();
    for (_,selector,meta,_) in &rows {
        if !seen.insert((*selector,*meta)) {return Err("duplicate notification payload method".into());}
        let matching=signatures.iter().filter(|(s,m,_)|s==selector&&m==meta).collect::<Vec<_>>();
        if matching.len()!=1 {return Err("notification payload ABI roster differs".into());}
        let source=crate::sdk::metadata::method(loaded,selector,*meta)?.ok_or("notification source method absent")?;
        super::classes::verify_typed_abi(&source,&matching[0].2)?;
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    {
        let concrete = crate::sdk::metadata::loaded_class(c"NSConcreteNotification")?;
        let original = crate::sdk::metadata::method(loaded, c"dealloc", false)?
            .ok_or("Reference original notification deallocator absent")?;
        let root = crate::sdk::metadata::loaded_class(c"NSObject")?;
        if unsafe { &*(concrete.address() as *const AnyClass) }.superclass() != Some(parent)
            || original.owner() != root.address() || original.encoding() != b"v16@0:8"
            || original.code().image() != b"/usr/lib/libobjc.A.dylib"
            || original.code().offset() != 0xb328
        {
            return Err("Reference platform notification deallocation differs".into());
        }
        PLATFORM_NOTIFICATION_DEALLOC.set((concrete, original))
            .map_err(|_| "Reference platform notification deallocation already bound")?;
    }
    let guard=unsafe {super::classes::ClassGuard::select_sdk_family_checked(
        &[parent],workspace_factory_lease(&model)?,owned_notification_payload_dealloc,prewrite)}?;
    let proxy=unsafe {guard.subclass_qualified(c"RuycastF1OwnedNotificationPayloadSource",parent)}?;
    let mut bindings=guard.lifecycle_bindings_for(parent)?;
    for (_,selector,meta,implementation) in rows {
        let signature=signatures.iter().find(|(s,m,_)|*s==selector&&*m==meta)
            .ok_or("notification payload signature disappeared")?;
        let leaf=unsafe {guard.overlay_bound(if meta{parent}else{proxy},selector,meta,implementation,&signature.2)}?;
        bindings.push(super::receiver::LifecycleBinding::leaf(leaf));
    }
    let grant=unsafe {super::receiver::WorkspaceGrant::for_owned_sdk(model,proxy,bindings)}?;
    super::factory::publish_proxy(c"NSNotification",Arc::new(grant))
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_NOTIFICATION_DEALLOC: OnceLock<(crate::sdk::metadata::ClassHandle, crate::sdk::metadata::MethodSnapshot)> = OnceLock::new();

pub(super) unsafe extern "C" fn owned_notification_payload_dealloc(pointer:*mut AnyObject,_selector:Sel) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if !pointer.is_null() && crate::sdk::private_network::reference_website_assembly_started() {
        if let Some((concrete, original)) = PLATFORM_NOTIFICATION_DEALLOC.get() {
            if unsafe { objc2::ffi::object_getClass(pointer) } as usize == concrete.address() {
                if _selector != objc2::sel!(dealloc)
                    || !crate::sdk::metadata::code_identity(original.implementation()).is_ok_and(|code| code == *original.code())
                {
                    unsafe { libc::_exit(89) };
                }
                // A Foundation callback can drain its autorelease pool after
                // the website probe closes. SDK payloads use a separate class.
                let platform: unsafe extern "C" fn(*mut AnyObject, Sel) =
                    unsafe { std::mem::transmute(original.implementation()) };
                unsafe { platform(pointer, _selector) };
                return;
            }
        }
    }
    let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        let installed=INSTALLED.get().ok_or("workspace absent during notification cleanup")?;
        super::factory::verify_notification_payload_cleanup(pointer,&installed.model)?;
        let serial=super::factory::owned_reference_serial(pointer)?;
        let expected={
            let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            match objects.get(&(pointer as usize)) {
                None=>None,
                Some(Kind::NotificationPayload(value))=>Some(value.0.clone()),
                Some(_)=>return Err("notification cleanup kind differs".into()),
            }
        };
        if super::factory::owned_reference_serial(pointer)?!=serial {return Err("notification cleanup serial changed".into());}
        super::factory::deallocate_owned(pointer,|| {
            let removed={
                let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
                let same=match (objects.get(&(pointer as usize)),&expected) {
                    (None,None)=>true,
                    (Some(Kind::NotificationPayload(value)),Some(before))=>Arc::ptr_eq(&value.0,before),
                    _=>false,
                };
                if !same {return Err("notification cleanup payload changed".into());}
                objects.remove(&(pointer as usize))
            };
            if let Some(Kind::NotificationPayload(value))=&removed {value.0.ready.store(false,Ordering::Release);}
            // Retained child ARC/drop occurs only after releasing the arena lock.
            drop(removed);Ok(())
        })
    }));
    let failed=match outcome {Ok(result)=>result.is_err(),Err(payload)=>{std::mem::forget(payload);true}};
    if failed {if let Some(installed)=INSTALLED.get(){installed.model.fault();}}
}

pub fn replacements() -> Vec<(&'static CStr, &'static CStr, bool, usize)> {
    let mut rows = vec![
        (c"NSWorkspaceOpenConfiguration", c"init", false, init_open_configuration as *const () as usize),
        (c"NSWorkspace", c"iconForFile:", false, workspace_icon as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSWorkspace", c"openApplicationAtURL:configuration:completionHandler:", false,
            reference_open_application as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSWorkspace", c"openURLs:withApplicationAtURL:configuration:completionHandler:", false,
            reference_open_urls as *const () as usize),
        (c"NSImage", c"size", false, workspace_image_size as *const () as usize),
        (c"NSImage", c"TIFFRepresentation", false, workspace_image_tiff as *const () as usize),
        (c"NSNotification",c"notificationWithName:object:userInfo:",true,notification_create as *const () as usize),
        (c"NSNotification",c"name",false,notification_name as *const () as usize),
        (c"NSNotification",c"object",false,notification_object as *const () as usize),
        (c"NSNotification",c"userInfo",false,notification_user_info as *const () as usize),
        (
            c"NSWorkspace",
            c"sharedWorkspace",
            true,
            shared as *const () as usize,
        ),
        (
            c"NSWorkspace",
            c"frontmostApplication",
            false,
            front as *const () as usize,
        ),
        (
            c"NSWorkspace",
            c"runningApplications",
            false,
            running as *const () as usize,
        ),
        (
            c"NSWorkspace",
            c"URLForApplicationWithBundleIdentifier:",
            false,
            app_lookup as *const () as usize,
        ),
        (
            c"NSWorkspace",
            c"openURL:",
            false,
            open as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"runningApplicationsWithBundleIdentifier:",
            true,
            by_bundle as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"runningApplicationWithProcessIdentifier:",
            true,
            by_pid as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"processIdentifier",
            false,
            pid as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"bundleIdentifier",
            false,
            bundle as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"localizedName",
            false,
            name as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"bundleURL",
            false,
            app_url as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"executableURL",
            false,
            executable as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"activationPolicy",
            false,
            policy as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"isTerminated",
            false,
            terminated as *const () as usize,
        ),
        (
            c"NSRunningApplication",
            c"activateWithOptions:",
            false,
            activate as *const () as usize,
        ),
        (c"NSScreen", c"screens", true, screens as *const () as usize),
        (
            c"NSScreen",
            c"mainScreen",
            true,
            main_screen as *const () as usize,
        ),
        (c"NSScreen", c"frame", false, frame as *const () as usize),
        (
            c"NSScreen",
            c"visibleFrame",
            false,
            visible_frame as *const () as usize,
        ),
        (
            c"NSScreen",
            c"backingScaleFactor",
            false,
            scale as *const () as usize,
        ),
        (
            c"NSEvent",
            c"mouseLocation",
            true,
            mouse as *const () as usize,
        ),
    ];
    rows.extend(window_replacements());
    rows.extend(application::replacements());
    rows.extend(foundation::replacements());
    rows.extend(clipboard_bridge::replacements());
    rows.extend(controls::replacements());
    rows.extend(notification_bridge::replacements());
    rows.extend(apple_events::replacements());
    rows.extend(task_factory_rows());
    rows.push((
        c"NSRunningApplication",
        c"currentApplication",
        true,
        current_application as *const () as usize,
    ));
    rows
}
#[cfg(test)]
fn own_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut b =
            objc2::runtime::ClassBuilder::new(c"RuycastF1OwnedWorkspaceData", NSObject::class())
                .expect("unique owned workspace class");
        unsafe {
            b.add_method(
                objc2::sel!(frontmostApplication),
                front as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
            b.add_method(
                objc2::sel!(runningApplications),
                running as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
            b.add_method(
                objc2::sel!(processIdentifier),
                pid as unsafe extern "C" fn(*mut AnyObject, Sel) -> i32,
            );
            b.add_method(
                objc2::sel!(bundleIdentifier),
                bundle as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
            b.add_method(
                objc2::sel!(localizedName),
                name as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            );
            b.add_method(
                objc2::sel!(frame),
                frame as unsafe extern "C" fn(*mut AnyObject, Sel) -> NSRect,
            );
            b.add_method(
                objc2::sel!(visibleFrame),
                visible_frame as unsafe extern "C" fn(*mut AnyObject, Sel) -> NSRect,
            );
            b.add_method(
                objc2::sel!(backingScaleFactor),
                scale as unsafe extern "C" fn(*mut AnyObject, Sel) -> f64,
            );
            b.add_method(
                objc2::sel!(activateWithOptions:),
                activate as unsafe extern "C" fn(*mut AnyObject, Sel, usize) -> Bool,
            );
            b.add_method(
                objc2::sel!(openURL:),
                open as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool,
            );
            b.add_method(
                objc2::sel!(orderOut:),
                order as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            b.add_method(
                objc2::sel!(makeKeyAndOrderFront:),
                order as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            b.add_method(
                objc2::sel!(isVisible),
                visible as unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
        }
        application::add_methods(&mut b);
        notification_bridge::add_methods(&mut b);
        controls::add_methods(&mut b);
        b.register()
    })
}
#[cfg(test)]
fn own_apple_event_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder =
            objc2::runtime::ClassBuilder::new(c"RuycastF1OwnedAppleEventData", NSObject::class())
                .expect("unique owned AppleEvent class");
        apple_events::add_methods(&mut builder);
        builder.register()
    })
}
pub fn unresolved_roster() -> &'static [&'static str] {
    &[
        "source-family class +initialize before AppKit subclass messages",
        "NSRunningApplication type-preserving subclasses/downcast",
        "NSWorkspace.notificationCenter observer/block delivery",
        "NSWorkspace iconForFile file-icon fixtures and original NSImage consumer dispatch",
        "NSWorkspace async open configured completion ABI",
        "NSApplication shared/run/stop/activate/mainMenu",
        "NSStatusBar item/menu/button",
        "NSWindow/NSPanel initializer and subclass ivar contract",
        "NSWindow complete geometry/order/key/miniaturization selectors",
        "native permission/focus imports CGEventPostToPid and ProcessSerialNumber coordination",
    ]
}
pub fn scope_ready() -> bool {
    false
}
fn to_nsrect(value: Rect) -> NSRect {
    NSRect::new(NSPoint::new(value.origin.x, value.origin.y), NSSize::new(value.size.x, value.size.y))
}
fn from_nsrect(value: NSRect) -> Rect {
    Rect { origin: Point { x: value.origin.x, y: value.origin.y },
        size: Point { x: value.size.width, y: value.size.height } }
}
unsafe fn initialize_window(pointer: *mut AnyObject, frame: NSRect, style: usize,
    backing: usize, _defer: Bool, screen: *mut AnyObject) -> Result<*mut AnyObject> {
    if !window_scope_active() { return Err("window outside full workspace arena".into()); }
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    let role = super::factory::receiver_role(pointer, &installed.model)?;
    let window_role = match role {
        value if value == c"NSWindow" => super::WindowRole::Window,
        value if value == c"NSPanel" => super::WindowRole::Panel,
        _ => return Err("window initializer allocation role differs".into()),
    };
    super::factory::claim_owned_role(pointer, role, &installed.model, owned_window_dealloc as *const () as usize)?;
    // Claim is the first ownership operation; an unknown/wrong pointer is never adopted.
    let pending = unsafe { super::factory::OwnedReceiver::from_owned(pointer) }?;
    const STYLE_BITS: usize = 1|2|4|8|16|64|128|256|4096|8192|16384|32768;
    if backing > 2 || style & !STYLE_BITS != 0 { return Err("unsupported window initializer flags".into()); }
    let screen = if screen.is_null() { None } else {
        if qualified_arena_reference_role(screen)? != c"NSScreen" { return Err("window screen has wrong role".into()); }
        match kind(screen)? { Kind::Screen(index) => Some(index), _ => return Err("window screen payload differs".into()) }
    };
    let record = register_window_payload(installed, pointer, frame, window_role, screen, WindowOrigin::Sdk)?;
    unsafe { super::factory::initialize(pointer, &installed.model) }?;
    record.serial.store(super::factory::owned_reference_serial(pointer)?, Ordering::Release);
    record.ready.store(true, Ordering::Release);
    record.check(pointer)?;
    Ok(pending.into_raw())
}

fn register_window_payload(installed: &Installed, pointer: *mut AnyObject, frame: NSRect,
    role: super::WindowRole, screen: Option<usize>, origin: WindowOrigin) -> Result<Arc<WindowRecord>> {
    let handle = installed.model.new_window_typed(from_nsrect(frame), role, screen)?;
    let record = Arc::new(WindowRecord { model: installed.model.clone(), handle,
        owner: std::thread::current().id(), origin, serial: std::sync::atomic::AtomicU64::new(0),
        ready: AtomicBool::new(false), live: AtomicBool::new(true), references: Mutex::new(WindowReferences::default()),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        palette_appearance: Mutex::new(PaletteAppearance::default()),
    });
    {
        let mut objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
        if objects.contains_key(&(pointer as usize)) { return Err("window initializer replay".into()); }
        objects.insert(pointer as usize, Kind::WindowPayload(WindowIdentity(record.clone())));
    }
    Ok(record)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn initialize_reference_panel_payload(
    pointer: *mut AnyObject, frame: NSRect, grant: &Arc<super::receiver::WorkspaceGrant>,
    token: super::receiver::ReceiverToken,
) -> Result<()> {
    let origin = if grant.is_reference_palette_construction() {
        WindowOrigin::ReferencePalette
    } else if grant.is_reference_snippet_panel_construction() {
        WindowOrigin::ReferenceSnippetChoice
    } else {
        return Err("Reference original panel grant differs".into());
    };
    if !window_scope_active() { return Err("Reference PalettePanel outside full workspace arena".into()); }
    let installed = INSTALLED.get().ok_or("Reference PalettePanel workspace absent")?;
    if !Arc::ptr_eq(grant.workspace(), &installed.model) {
        return Err("Reference PalettePanel model differs".into());
    }
    let record = register_window_payload(installed, pointer, frame, super::WindowRole::Panel, None, origin)?;
    grant.initialized(token)?;
    let identity = grant.receiver_identity(pointer as usize)?;
    record.serial.store(identity.allocation_serial(), Ordering::Release);
    record.ready.store(true, Ordering::Release);
    record.check(pointer)
}
unsafe extern "C" fn owned_window_dealloc(pointer: *mut AnyObject, _: Sel) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        super::factory::deallocate_owned(pointer, || {
            let removed = {
                let mut objects = INSTALLED.get().ok_or("workspace absent")?.objects
                    .lock().map_err(|_| "object arena poisoned")?;
                match objects.get(&(pointer as usize)) {
                    Some(Kind::WindowPayload(_)) => objects.remove(&(pointer as usize)),
                    None => None,
                    _ => return Err("window deallocation payload differs".into()),
                }
            };
            if let Some(Kind::WindowPayload(identity)) = &removed { identity.0.retire()?; }
            // The actual ARC children and weak slots are destroyed outside arena locks.
            drop(removed);
            Ok(())
        })
    }));
    if !matches!(result, Ok(Ok(()))) {
        if let Err(payload) = result { std::mem::forget(payload); }
        if let Some(installed) = INSTALLED.get() { installed.model.fault(); }
    }
}
fn window_scope_active() -> bool {
    INSTALLED.get().is_some_and(|installed| !installed.foundation_data_only && !installed.apple_event_data_only)
}
fn window_boundary<T>(fallback: T, body: impl FnOnce() -> Result<T>) -> T {
    if !window_scope_active() { return fallback; }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Ok(value)) => return value,
        Err(payload) => std::mem::forget(payload),
        Ok(Err(_)) => (),
    }
    if let Some(installed) = INSTALLED.get() { installed.model.fault(); }
    fallback
}
unsafe extern "C" fn window_init(p: *mut AnyObject, _: Sel, rect: NSRect, style: usize, backing: usize, defer: Bool) -> *mut AnyObject {
    window_boundary(std::ptr::null_mut(), || unsafe { initialize_window(p, rect, style, backing, defer, std::ptr::null_mut()) })
}
unsafe extern "C" fn window_init_screen(p: *mut AnyObject, _: Sel, rect: NSRect, style: usize, backing: usize, defer: Bool, screen: *mut AnyObject) -> *mut AnyObject {
    window_boundary(std::ptr::null_mut(), || unsafe { initialize_window(p, rect, style, backing, defer, screen) })
}
unsafe extern "C" fn window_frame(p: *mut AnyObject, _: Sel) -> NSRect {
    window_boundary(NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(0.0,0.0)), || {
        let record = window_identity(p)?; let snapshot = record.model.window_snapshot(record.handle)?;
        record.check(p)?; Ok(to_nsrect(snapshot.frame))
    })
}
unsafe extern "C" fn window_set_frame(p: *mut AnyObject, _: Sel, rect: NSRect, _: Bool) {
    window_boundary((), || {
        let record = window_identity(p)?;
        let frame = from_nsrect(rect);
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if record.origin == WindowOrigin::ReferencePalette {
            let content = record.references.lock().map_err(|_| "window references poisoned")?
                .content.clone().ok_or("palette host absent before resize")?;
            let WindowReference::Palette(host) = content.as_ref() else {
                return Err("palette resize content owner differs".into());
            };
            let _resize = host.resizing.try_lock().map_err(|_| "palette resize already in progress")?;
            let (_, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
            let previous = record.model.window_snapshot(record.handle)?.frame;
            record.model.with(|state| {
                if stopped.load(Ordering::Acquire) { return Err("palette resize stopped".into()); }
                super::set_window_frame(state, record.handle, frame)
            })?;
            if previous.size != frame.size {
                let bounds = NSRect::new(NSPoint::new(0.0, 0.0), rect.size);
                host.layout(bounds, stopped)?;
            }
            record.check(p)?;
            host.verify()?;
            if stopped.load(Ordering::Acquire) { return Err("palette resize stopped during layout".into()); }
            return Ok(());
        }
        record.model.set_frame(record.handle, frame)?;
        record.check(p)
    });
}
unsafe extern "C" fn window_set_frame_animated(p: *mut AnyObject, s: Sel, rect: NSRect, display: Bool, _: Bool) {
    unsafe { window_set_frame(p,s,rect,display) }
}
unsafe extern "C" fn window_set_origin(p: *mut AnyObject, _: Sel, point: NSPoint) {
    window_boundary((), || { let r = window_identity(p)?; r.model.set_window_origin(r.handle, Point{x:point.x,y:point.y})?; r.check(p) })
}
unsafe extern "C" fn window_set_size(p: *mut AnyObject, _: Sel, size: NSSize) {
    window_boundary((), || { let r = window_identity(p)?; r.model.set_window_size(r.handle, Point{x:size.width,y:size.height})?; r.check(p) })
}
unsafe extern "C" fn window_center(p: *mut AnyObject, _: Sel) {
    window_boundary((), || { let r = window_identity(p)?; r.model.center_window(r.handle)?; r.check(p) })
}
unsafe extern "C" fn window_order(p: *mut AnyObject, _: Sel, mode: isize, relative: isize) {
    window_boundary((), || {
        let r = window_identity(p)?;
        // No source-qualified caller currently supplies another window number.
        // Do not silently discard an unimplemented relative-order relationship.
        if relative != 0 { return Err("relative window ordering is outside the bound caller roster".into()); }
        let order = match mode { 1 => super::WindowOrder::Front, -1 => super::WindowOrder::Back,
            0 => super::WindowOrder::Out, _ => return Err("invalid window ordering mode".into()) };
        apply_window_order(&r.model, r.handle, order, || r.check(p))
    })
}
unsafe extern "C" fn window_mini(p: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    window_boundary((), || { let r=window_identity(p)?; apply_window_order(&r.model, r.handle, WindowOrder::Miniaturize, || r.check(p)) })
}
unsafe extern "C" fn window_demini(p: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    window_boundary((), || { let r=window_identity(p)?; r.model.window_order(r.handle,super::WindowOrder::Deminiaturize)?; r.check(p) })
}
fn window_flag(p: *mut AnyObject, flag: fn(super::WindowSnapshot)->bool) -> Bool {
    window_boundary(Bool::new(false), || { let r=window_identity(p)?; let value=flag(r.model.window_snapshot(r.handle)?); r.check(p)?; Ok(Bool::new(value)) })
}
unsafe extern "C" fn window_key(p: *mut AnyObject, _: Sel)->Bool { window_flag(p,|s|s.key) }
unsafe extern "C" fn window_main(p: *mut AnyObject, _: Sel)->Bool { window_flag(p,|s|s.main) }
unsafe extern "C" fn window_minimized(p: *mut AnyObject, _: Sel)->Bool { window_flag(p,|s|s.miniaturized) }
unsafe extern "C" fn window_screen(p: *mut AnyObject, _: Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(), || {
        let r=window_identity(p)?; let screen=r.model.window_snapshot(r.handle)?.screen;
        let result=screen.map(|index|object(Kind::Screen(index))).transpose()?.unwrap_or(std::ptr::null_mut());
        r.check(p)?; Ok(result)
    })
}
unsafe extern "C" fn window_number(p: *mut AnyObject, _: Sel)->isize {
    window_boundary(0, || { let r=window_identity(p)?; isize::try_from(r.handle.id).map_err(|_|"window number overflow".into()) })
}
unsafe extern "C" fn window_content(p: *mut AnyObject, _: Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(), || {
        let r=window_identity(p)?; let reference=r.references.lock().map_err(|_|"window references poisoned")?.content.clone();
        let result=reference.map(|reference|reference.returned(c"NSView")).transpose()?.unwrap_or(std::ptr::null_mut());
        r.check(p)?; Ok(result)
    })
}
unsafe extern "C" fn window_set_content(p: *mut AnyObject, _: Sel, content:*mut AnyObject) {
    window_boundary((), || {
        let r=window_identity(p)?;
        let content=if content.is_null(){None}else{Some(Arc::new(WindowReference::acquire(content,c"NSView")?))};
        r.check(p)?;
        let previous={let mut refs=r.references.lock().map_err(|_|"window references poisoned")?;
            (std::mem::replace(&mut refs.content,content),refs.responder.take())};
        drop(previous); r.check(p)
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_reference_palette_content(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "mov x4, x19", "b {checked}", checked = sym mount_reference_palette_host);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn mount_reference_palette_host(
    panel: *mut AnyObject, selector: Sel, host: *mut AnyObject, caller: usize, controller: usize,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        let record = window_identity(panel)?;
        if !matches!(record.origin, WindowOrigin::ReferencePalette | WindowOrigin::ReferenceSnippetChoice)
            || selector.name() != c"setContentView:" || host.is_null()
        {
            return Err("invalid original PalettePanel content call".into());
        }
        let allocation = {
            let mut references = record.references.lock().map_err(|_| "window references poisoned")?;
            if references.content.is_some() || references.responder.is_some() {
                return Err("palette host mount repeated".into());
            }
            references.pending_palette_host.take().ok_or("palette host allocation absent or already claimed")?
        };
        if allocation.pointer != host as usize || allocation.controller != controller {
            return Err("palette initializer returned a different host".into());
        }
        let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
        let verify_source = || -> Result<()> {
            let snippet_panel = if record.origin == WindowOrigin::ReferenceSnippetChoice { panel as usize } else { 0 };
            if admission.verify_palette_host_panel(caller, controller, allocation.class, true, snippet_panel)? != panel
                || stopped.load(Ordering::Acquire)
            {
                return Err("palette host mount issuer differs or stopped".into());
            }
            record.check(panel)?;
            if unsafe { &*host }.class() != allocation.class {
                return Err("palette initializer changed the host class".into());
            }
            Ok(())
        };
        verify_source()?;

        let view = crate::sdk::metadata::loaded_class(c"NSView")?;
        let container_class = unsafe { &*(view.address() as *const AnyClass) };
        allocation.verify()?;
        allocation.root.verify_proxy_arc(container_class)?;
        let frame = record.model.window_snapshot(record.handle)?.frame;
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.x, frame.size.y));
        allocation.verify()?;
        if unsafe { &*host }.class() != allocation.class {
            return Err("palette host class changed before mounting".into());
        }
        let alloc: unsafe extern "C-unwind" fn(*const AnyClass, Sel) -> *mut AnyObject =
            unsafe { std::mem::transmute(palette_view_method(container_class, c"alloc")?) };
        if stopped.load(Ordering::Acquire) { return Err("palette container allocation stopped".into()); }
        let allocated = unsafe { alloc(container_class, objc2::sel!(alloc)) };
        if allocated.is_null() || unsafe { &*allocated }.class() != container_class {
            return Err("palette mount container allocation differs".into());
        }
        let init: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect) -> *mut AnyObject =
            unsafe { std::mem::transmute(palette_view_method(container_class, c"initWithFrame:")?) };
        if stopped.load(Ordering::Acquire) { return Err("palette container initialization stopped".into()); }
        let container = unsafe { init(allocated, objc2::sel!(initWithFrame:), bounds) };
        if container != allocated || unsafe { &*container }.class() != container_class {
            return Err("palette mount container initializer differs".into());
        }
        allocation.verify()?;
        if unsafe { &*host }.class() != allocation.class || stopped.load(Ordering::Acquire) {
            return Err("palette host retention stopped or class changed".into());
        }
        let retained = unsafe { Retained::retain(host) }.ok_or("palette host retain failed")?;
        let mounted = WindowPaletteHost {
            host: Retained::into_raw(retained) as usize, class: allocation.class,
            container: container as usize, container_class, owner: record.owner,
            model: record.model.clone(), root: allocation.root, lifecycle: allocation.lifecycle,
            resizing: Mutex::new(()),
            dispatching_key: Mutex::new(()),
            retirement_attempted: false,
            released: Arc::new(AtomicBool::new(false)),
        };
        mounted.attach(bounds, stopped)?;
        verify_source()?;
        mounted.verify()?;
        let mut references = record.references.lock().map_err(|_| "window references poisoned")?;
        if stopped.load(Ordering::Acquire) || references.content.is_some() || references.responder.is_some()
            || references.pending_palette_host.is_some()
        {
            return Err("palette host handoff changed during mount".into());
        }
        if record.origin == WindowOrigin::ReferencePalette {
            INSTALLED.get().ok_or("workspace absent")?.palette_host_release
                .set(mounted.released.clone()).map_err(|_| "original palette host already published")?;
        }
        references.content = Some(Arc::new(WindowReference::Palette(mounted)));
        Ok(())
    }));
    // The original caller releases its construction +1 immediately after
    // this hook. A failed lifecycle or mount must not resume that path.
    match result {
        Ok(Ok(())) => (),
        Err(payload) => { std::mem::forget(payload); unsafe { libc::_exit(95) } }
        Ok(Err(_)) => unsafe { libc::_exit(95) },
    }
}

unsafe extern "C" fn window_first_responder(p: *mut AnyObject, _: Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(), || {
        let r=window_identity(p)?; let weak=r.references.lock().map_err(|_|"window references poisoned")?.responder.clone();
        let result=weak.map(|weak|weak.returned()).transpose()?.unwrap_or(p);r.check(p)?;Ok(result)
    })
}
unsafe extern "C" fn window_make_responder(p: *mut AnyObject, _: Sel, responder:*mut AnyObject)->Bool {
    window_boundary(Bool::new(false), || {
        let r=window_identity(p)?;
        let previous=r.references.lock().map_err(|_|"window references poisoned")?.responder.clone();
        let current=previous.as_ref().map(|weak|weak.returned()).transpose()?.unwrap_or(p);
        let current=if current.is_null(){p}else{current};
        let requested=if responder.is_null(){p}else{responder};
        if current==requested { r.check(p)?;return Ok(Bool::new(true)); }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if r.origin == WindowOrigin::ReferencePalette && current == p && requested != p {
            let content = r.references.lock().map_err(|_| "window references poisoned")?
                .content.clone().ok_or("palette host absent before focus attempt")?;
            let WindowReference::Palette(host) = content.as_ref() else {
                return Err("palette focus content owner differs".into());
            };
            let _view_update = host.resizing.try_lock().map_err(|_| "palette view update already in progress")?;
            let (_, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
            let host_pointer = host.verify()? as usize;
            verify_reference_mounted_palette_host(&r.model, host_pointer, stopped)?;
            let answer = super::receiver::app_delegate::constructor::try_reference_detached_search_focus(
                host_pointer, requested as usize, stopped,
            )?;
            verify_reference_mounted_palette_host(&r.model, host_pointer, stopped)?;
            return Ok(answer);
        }
        // The qualified original panel's implicit self responder is borrowed
        // from this very call. No weak slot or source callback grant is needed
        // for that fallback. External responders keep their full checks.
        if !(r.origin != WindowOrigin::Sdk && current == p) {
            let old=WindowReference::acquire(current,c"NSResponder")?;
            if !old.invoke_bool(c"resignFirstResponder")? { r.check(p)?;return Ok(Bool::new(false)); }
        }
        let selected = if r.origin != WindowOrigin::Sdk && requested == p {
            p
        } else {
            let candidate=WindowReference::acquire(requested,c"NSResponder")?;
            if candidate.invoke_bool(c"becomeFirstResponder")? {requested}else{p}
        };
        let weak = if r.origin != WindowOrigin::Sdk && selected == p {
            None
        } else {
            Some(Arc::new(WindowWeak::new(selected,c"NSResponder")?))
        };
        r.check(p)?;
        let discarded={let mut refs=r.references.lock().map_err(|_|"window references poisoned")?;
            std::mem::replace(&mut refs.responder,weak)};
        drop(discarded);r.check(p)?;Ok(Bool::new(true))
    })
}
unsafe extern "C" fn responder_accepts(p:*mut AnyObject,_:Sel)->Bool {
    window_boundary(Bool::new(false),||{window_argument(p,c"NSResponder")?;Ok(Bool::new(false))})
}
unsafe extern "C" fn responder_transition(p:*mut AnyObject,_:Sel)->Bool {
    window_boundary(Bool::new(false),||{window_argument(p,c"NSResponder")?;Ok(Bool::new(true))})
}
unsafe extern "C" fn responder_init(p:*mut AnyObject,_:Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(),||{
        deny_foundation_only_factory()?;
        let installed=INSTALLED.get().ok_or("workspace absent")?;
        super::factory::claim_owned_responder_role(p,&installed.model,
            owned_window_dealloc as *const () as usize,owned_joint_responder_dealloc as *const () as usize)?;
        let pending=unsafe{super::factory::OwnedReceiver::from_owned(p)}?;
        unsafe{super::factory::initialize(p,&installed.model)}?;
        Ok(pending.into_raw())
    })
}

unsafe extern "C" fn window_delegate(p: *mut AnyObject, _: Sel)->*mut AnyObject {
    window_boundary(std::ptr::null_mut(), || {
        let r=window_identity(p)?;let weak=r.references.lock().map_err(|_|"window references poisoned")?.delegate.clone();
        let result=weak.map(|weak|weak.returned()).transpose()?.unwrap_or(std::ptr::null_mut());r.check(p)?;Ok(result)
    })
}
unsafe extern "C" fn window_set_delegate(p: *mut AnyObject, _: Sel, delegate:*mut AnyObject) {
    window_boundary((), || {
        let r=window_identity(p)?;
        let weak=if delegate.is_null(){None}else{Some(Arc::new(WindowWeak::new(delegate,c"NSObject")?))};
        r.check(p)?;let previous={let mut refs=r.references.lock().map_err(|_|"window references poisoned")?;std::mem::replace(&mut refs.delegate,weak)};
        drop(previous);r.check(p)
    })
}
fn window_signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding>
where F::Return: objc2::encode::EncodeReturn {
    use objc2::encode::{EncodeArguments, EncodeReturn, Encoding};
    let mut types=vec![F::Return::ENCODING_RETURN,Encoding::Object,Encoding::Sel];
    types.extend_from_slice(F::Arguments::ENCODINGS);types
}
macro_rules! window_roster {
    ($consumer:ident) => {$consumer!{
        (c"orderFront:",order,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"makeKeyAndOrderFront:",order,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"orderOut:",order,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"orderFrontRegardless",order_noarg,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (c"makeKeyWindow",order_noarg,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (c"close",order_noarg,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (c"isVisible",visible,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"initWithContentRect:styleMask:backing:defer:",window_init,unsafe extern "C" fn(*mut AnyObject,Sel,NSRect,usize,usize,Bool)->*mut AnyObject),
        (c"initWithContentRect:styleMask:backing:defer:screen:",window_init_screen,unsafe extern "C" fn(*mut AnyObject,Sel,NSRect,usize,usize,Bool,*mut AnyObject)->*mut AnyObject),
        (c"frame",window_frame,unsafe extern "C" fn(*mut AnyObject,Sel)->NSRect),
        (c"setFrame:display:",window_set_frame,unsafe extern "C" fn(*mut AnyObject,Sel,NSRect,Bool)),
        (c"setFrame:display:animate:",window_set_frame_animated,unsafe extern "C" fn(*mut AnyObject,Sel,NSRect,Bool,Bool)),
        (c"setFrameOrigin:",window_set_origin,unsafe extern "C" fn(*mut AnyObject,Sel,NSPoint)),
        (c"setContentSize:",window_set_size,unsafe extern "C" fn(*mut AnyObject,Sel,NSSize)),
        (c"center",window_center,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (c"orderWindow:relativeTo:",window_order,unsafe extern "C" fn(*mut AnyObject,Sel,isize,isize)),
        (c"miniaturize:",window_mini,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"deminiaturize:",window_demini,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"isKeyWindow",window_key,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"isMainWindow",window_main,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"isMiniaturized",window_minimized,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"screen",window_screen,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"windowNumber",window_number,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        (c"contentView",window_content,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"setContentView:",window_set_content,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"firstResponder",window_first_responder,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"makeFirstResponder:",window_make_responder,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->Bool),
        (c"delegate",window_delegate,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"setDelegate:",window_set_delegate,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject))
    }};
}
macro_rules! window_rows {
    ($(($selector:expr,$function:ident,$ty:ty)),*$(,)?) => {
        fn window_replacements()->Vec<(&'static CStr,&'static CStr,bool,usize)>{
            let mut rows=Vec::new();
            for role in [c"NSWindow",c"NSPanel"] { $(rows.push((role,$selector,false,$function as $ty as *const () as usize));)* }
            for role in [c"NSResponder",c"NSWindow",c"NSPanel"] {
                rows.extend([(role,c"acceptsFirstResponder",false,responder_accepts as *const () as usize),
                    (role,c"becomeFirstResponder",false,responder_transition as *const () as usize),
                    (role,c"resignFirstResponder",false,responder_transition as *const () as usize)]);
            }
            rows.push((c"NSResponder",c"init",false,responder_init as *const () as usize));rows
        }
        fn window_signatures()->Vec<(&'static CStr,&'static CStr,Vec<objc2::encode::Encoding>)>{
            let mut rows=Vec::new();
            for role in [c"NSWindow",c"NSPanel"] { $(rows.push((role,$selector,window_signature::<$ty>()));)* }
            for role in [c"NSResponder",c"NSWindow",c"NSPanel"] {
                for selector in [c"acceptsFirstResponder",c"becomeFirstResponder",c"resignFirstResponder"] {
                    rows.push((role,selector,window_signature::<unsafe extern "C" fn(*mut AnyObject,Sel)->Bool>()));
                }
            }
            rows.push((c"NSResponder",c"init",window_signature::<unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject>()));rows
        }
    };
}
window_roster!(window_rows);

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) const REFERENCE_PALETTE_WINDOW_SELECTORS: &[&CStr] = &[
    c"setTitle:",
    c"setBackgroundColor:",
    c"setOpaque:",
    c"setHasShadow:",
    c"setMovableByWindowBackground:",
    c"setReleasedWhenClosed:",
    c"setHidesOnDeactivate:",
    c"setLevel:",
    c"setCollectionBehavior:",
    c"standardWindowButton:",
    c"isVisible",
    c"setFrame:display:",
    c"makeKeyAndOrderFront:",
    c"orderOut:",
    c"contentView",
    c"setContentView:",
    c"firstResponder",
    c"makeFirstResponder:",
];

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) const REFERENCE_SNIPPET_APPEARANCE_SELECTORS: &[(usize, &CStr)] = &[
    (0x3e8448, c"setReleasedWhenClosed:"),
    (0x3e8e40, c"setOpaque:"),
    (0x3e8e48, c"setBackgroundColor:"),
    (0x3e8e50, c"setHasShadow:"),
    (0x3e8e68, c"setLevel:"),
    (0x3e8e70, c"setCollectionBehavior:"),
];

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) const REFERENCE_SNIPPET_WINDOW_SELECTORS: &[&CStr] = &[
    c"setContentView:",
    c"setFrameOrigin:",
    c"makeKeyAndOrderFront:",
    c"orderOut:",
    c"isKeyWindow",
];

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn check_reference_window_appearance(
    record: &WindowRecord, selector: Sel, caller: usize, value: usize,
) -> Result<()> {
    if record.origin == WindowOrigin::ReferencePalette {
        return Ok(());
    }
    if record.origin != WindowOrigin::ReferenceSnippetChoice {
        return Err("appearance receiver is not an original Reference panel".into());
    }
    let offsets: &[usize] = match (selector.name().to_bytes(), value) {
        (b"setReleasedWhenClosed:", 0) => &[0x1a2efc, 0x1a60e4],
        (b"setOpaque:", 0) => &[0x1a60f4],
        (b"setBackgroundColor:", _) => &[0x1a6128],
        (b"setHasShadow:", 1) => &[0x1a6140],
        (b"setLevel:", 3) => &[0x1a6154],
        (b"setCollectionBehavior:", 0x101) => &[0x1a6168],
        _ => return Err("unsupported original SnippetChoicePanel appearance".into()),
    };
    crate::sdk::reference_runtime::verify_startup_control_caller(&record.model, caller, offsets)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn update_palette_appearance(
    pointer: *mut AnyObject, selector: Sel, caller: usize, value: usize,
    update: impl FnOnce(&mut PaletteAppearance) -> Result<()>,
) {
    window_boundary((), || {
        let record = window_identity(pointer)?;
        check_reference_window_appearance(&record, selector, caller, value)?;
        let mut appearance = record.palette_appearance.lock().map_err(|_| "Palette appearance poisoned")?;
        update(&mut appearance)
    });
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_palette_appearance_object(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_palette_appearance_object_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_palette_appearance_object_checked(
    pointer: *mut AnyObject, selector: Sel, value: *mut AnyObject, caller: usize,
) {
    window_boundary((), || {
        let record = window_identity(pointer)?;
        let offset = match selector.name().to_bytes() {
            b"setTitle:" => 0x14cb18,
            b"setBackgroundColor:" => 0x14cb74,
            _ => return Err("unsupported Palette appearance object".into()),
        };
        let check = || {
            check_reference_window_appearance(&record, selector, caller, value as usize)?;
            if record.origin == WindowOrigin::ReferencePalette {
                crate::sdk::reference_runtime::verify_startup_control_caller(&record.model, caller, &[offset])?;
            }
            Ok::<(), String>(())
        };
        check()?;
        let title = if selector.name() == c"setTitle:" {
            let title = unsafe { task_objects::reference_string(value) }?;
            if title != "Ruycast" {
                return Err("original PalettePanel title differs".into());
            }
            Some(title)
        } else {
            let color = crate::sdk::metadata::loaded_class(c"NSColor")?;
            let class = unsafe { &*(color.address() as *const AnyClass) };
            let method = crate::sdk::metadata::method(color, c"clearColor", true)?
                .ok_or("AppKit clearColor factory absent")?;
            let code = method.code();
            if method.owner() != class.metaclass() as *const AnyClass as usize
                || code.image() != b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit"
                || code.offset() != 0x14344
                || code.uuid() != [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64,
                    0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46]
            {
                return Err("AppKit clearColor factory owner or entry differs".into());
            }
            type ClearColorAbi = unsafe extern "C" fn(*const AnyClass, Sel) -> *mut AnyObject;
            super::classes::verify_typed_abi(&method, &window_signature::<ClearColorAbi>())?;
            let make_clear: unsafe extern "C-unwind" fn(*const AnyClass, Sel) -> *mut AnyObject =
                unsafe { std::mem::transmute(method.implementation()) };
            let clear = unsafe { make_clear(class, objc2::sel!(clearColor)) };
            crate::sdk::metadata::verify_current_method(color, c"clearColor", true, &method)?;
            if clear.is_null() || clear != value {
                return Err("original PalettePanel background is not AppKit clearColor".into());
            }
            None
        };
        check()?;
        record.check(pointer)?;
        let mut appearance = record.palette_appearance.lock().map_err(|_| "Palette appearance poisoned")?;
        if let Some(title) = title {
            appearance.title = Some(title);
        } else {
            appearance.clear_background = true;
        }
        Ok(())
    });
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_palette_appearance_flag(_: *mut AnyObject, _: Sel, _: Bool) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_palette_appearance_flag_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_palette_appearance_flag_checked(pointer: *mut AnyObject, selector: Sel, value: Bool, caller: usize) {
    update_palette_appearance(pointer, selector, caller, usize::from(value.as_bool()), |appearance| {
        let value = value.as_bool();
        let target = match selector.name().to_bytes() {
            b"setOpaque:" => &mut appearance.opaque,
            b"setHasShadow:" => &mut appearance.shadow,
            b"setMovableByWindowBackground:" => &mut appearance.movable_by_background,
            b"setReleasedWhenClosed:" if !value => &mut appearance.released_when_closed,
            b"setHidesOnDeactivate:" if !value => &mut appearance.hides_on_deactivate,
            _ => return Err("unsupported original PalettePanel flag".into()),
        };
        *target = Some(value);
        Ok(())
    });
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_palette_level(_: *mut AnyObject, _: Sel, _: isize) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_palette_level_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_palette_level_checked(pointer: *mut AnyObject, selector: Sel, value: isize, caller: usize) {
    update_palette_appearance(pointer, selector, caller, value as usize, |appearance| {
        appearance.level = Some(value);
        Ok(())
    });
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_palette_collection_behavior(_: *mut AnyObject, _: Sel, _: usize) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_palette_collection_behavior_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_palette_collection_behavior_checked(pointer: *mut AnyObject, selector: Sel, value: usize, caller: usize) {
    update_palette_appearance(pointer, selector, caller, value, |appearance| {
        appearance.collection_behavior = Some(value);
        Ok(())
    });
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn palette_standard_window_button(
    pointer: *mut AnyObject, _: Sel, button: usize,
) -> *mut AnyObject {
    window_boundary(std::ptr::null_mut(), || {
        let record = window_identity(pointer)?;
        if record.origin != WindowOrigin::ReferencePalette || button > 2 {
            return Err("window button outside original borderless PalettePanel".into());
        }
        // The exact original super-init admits only a nonactivating,
        // borderless panel. Its instance has no close/minimize/zoom controls.
        Ok(std::ptr::null_mut())
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn reference_palette_window_leaf(
    selector: &CStr,
) -> Result<(usize, Vec<objc2::encode::Encoding>)> {
    if !REFERENCE_PALETTE_WINDOW_SELECTORS.contains(&selector) && !REFERENCE_SNIPPET_WINDOW_SELECTORS.contains(&selector) {
        return Err("selector outside original Palette window operations".into());
    }
    match selector.to_bytes() {
        b"setContentView:" => {
            type Setter = unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject);
            return Ok((set_reference_palette_content as Setter as *const () as usize, window_signature::<Setter>()));
        }
        b"setTitle:" | b"setBackgroundColor:" => {
            type Setter = unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject);
            return Ok((set_palette_appearance_object as Setter as *const () as usize, window_signature::<Setter>()));
        }
        b"setOpaque:" | b"setHasShadow:" | b"setMovableByWindowBackground:"
        | b"setReleasedWhenClosed:" | b"setHidesOnDeactivate:" => {
            type Setter = unsafe extern "C" fn(*mut AnyObject, Sel, Bool);
            return Ok((set_palette_appearance_flag as Setter as *const () as usize, window_signature::<Setter>()));
        }
        b"setLevel:" => {
            type Setter = unsafe extern "C" fn(*mut AnyObject, Sel, isize);
            return Ok((set_palette_level as Setter as *const () as usize, window_signature::<Setter>()));
        }
        b"setCollectionBehavior:" => {
            type Setter = unsafe extern "C" fn(*mut AnyObject, Sel, usize);
            return Ok((set_palette_collection_behavior as Setter as *const () as usize, window_signature::<Setter>()));
        }
        b"standardWindowButton:" => {
            type Getter = unsafe extern "C" fn(*mut AnyObject, Sel, usize) -> *mut AnyObject;
            return Ok((palette_standard_window_button as Getter as *const () as usize, window_signature::<Getter>()));
        }
        _ => {}
    }
    let replacement = window_replacements().into_iter()
        .find(|(role, name, meta, _)| *role == c"NSPanel" && *name == selector && !*meta)
        .ok_or("private Palette window operation missing")?.3;
    let signature = window_signatures().into_iter()
        .find(|(role, name, _)| *role == c"NSPanel" && *name == selector)
        .ok_or("private Palette window ABI missing")?.2;
    Ok((replacement, signature))
}

pub struct WindowMethodPreflight {
    pub role: &'static CStr,
    pub selector: &'static CStr,
    pub source: crate::sdk::metadata::MethodSnapshot,
    pub replacement: crate::sdk::metadata::CodeIdentity,
}
/// Pure observation only; this does not initialize a framework class or qualify
/// custom ARC. Any non-NSObject memory path still requires its explicit plan.
pub fn preflight_window_factories()->Result<Vec<WindowMethodPreflight>> {
    let signatures=window_signatures();let mut seen=std::collections::BTreeSet::new();let mut result=Vec::new();
    for (role,selector,meta,implementation) in window_replacements() {
        if meta || !seen.insert((role,selector)) {return Err("duplicate/foreign Window method roster".into());}
        let class=crate::sdk::metadata::loaded_class(role)?;
        let source=crate::sdk::metadata::method(class,selector,false)?.ok_or("source Window method absent")?;
        let signature=signatures.iter().filter(|(r,s,_)|*r==role&&*s==selector).collect::<Vec<_>>();
        if signature.len()!=1{return Err("Window typed signature roster differs".into());}
        super::classes::verify_typed_abi(&source,&signature[0].2)?;
        result.push(WindowMethodPreflight{role,selector,source,replacement:crate::sdk::metadata::code_identity(implementation)?});
    }
    Ok(result)
}
/// # Safety
/// Exclusive source-pinned Window-family setup, no original instances, caller
/// has qualified every ancestor/lifecycle/ABI/code input. This invokes no
/// original NSWindow/NSPanel initializer or user-interface operation.
pub unsafe fn prepare_window_factories_checked(model:Arc<Workspace>,prewrite:impl FnOnce()->Result<()>)->Result<()> {
    if INSTALLED.get().is_some() && !window_scope_active() { return Err("Window preparation outside full workspace".into()); }
    preflight_window_factories()?;
    let responder=crate::sdk::metadata::loaded_class(c"NSResponder")?;
    let window=crate::sdk::metadata::loaded_class(c"NSWindow")?;
    let panel=crate::sdk::metadata::loaded_class(c"NSPanel")?;
    let responder=unsafe{&*(responder.address() as *const AnyClass)};
    let window=unsafe{&*(window.address() as *const AnyClass)};
    let panel=unsafe{&*(panel.address() as *const AnyClass)};
    if window.superclass()!=Some(responder)||panel.superclass()!=Some(window){return Err("Window source ancestry differs".into());}
    for parent in [responder,window,panel] {super::factory::preflight_default_proxy_arc(parent)?;}
    let lease=workspace_factory_lease(&model)?;
    let guard=unsafe{super::classes::ClassGuard::select_sdk_family_checked(&[responder,window,panel],lease,owned_window_dealloc,prewrite)}?;
    unsafe { prepare_windows_with_guard(model, [responder,window,panel], &guard) }
}
fn workspace_factory_lease(model:&Arc<Workspace>)->Result<crate::sdk::owned::OwnedHandleLease> {
    workspace_factory_lease_with_owner(model, Some(std::thread::current().id()))
}
fn workspace_factory_lease_with_owner(
    model: &Arc<Workspace>, owner: Option<std::thread::ThreadId>,
) -> Result<crate::sdk::owned::OwnedHandleLease> {
    let generation=model.state.lock().map_err(|_|"workspace state poisoned")?.generation;
    struct FactoryLife{model:Arc<Workspace>,inner:Arc<dyn crate::sdk::owned::AliveLease>,generation:u64,owner:Option<std::thread::ThreadId>}
    impl FactoryLife{
        fn pure(&self)->Result<()>{
            let state=self.model.state.lock().map_err(|_|"workspace state poisoned")?;
            if self.owner.is_some_and(|owner| owner != std::thread::current().id())||state.retired||state.generation!=self.generation
                ||self.model.faulted.load(Ordering::Acquire){return Err("Window factory epoch retired".into());}Ok(())
        }
    }
    impl crate::sdk::owned::AliveLease for FactoryLife{
        fn is_alive(&self)->bool{self.pure().is_ok()&&self.inner.is_alive()&&self.pure().is_ok()}
        fn before_effect(&self)->Result<()>{self.pure()?;self.inner.before_effect()?;self.pure()}
    }
    let mut lease=model.lease();lease.life=Arc::new(FactoryLife{model:model.clone(),inner:lease.life.clone(),generation,owner});
    Ok(lease)
}
unsafe fn prepare_windows_with_guard(model:Arc<Workspace>,parents:[&'static AnyClass;3],guard:&super::classes::ClassGuard)->Result<()> {
    let [responder,window,panel]=parents;
    let signatures=window_signatures();
    for (role,parent,name) in [(c"NSResponder",responder,c"RuycastF1OwnedResponderSource"),
        (c"NSWindow",window,c"RuycastF1OwnedWindowSource"),(c"NSPanel",panel,c"RuycastF1OwnedPanelSource")] {
        let proxy=unsafe{guard.subclass_qualified(name,parent)}?;
        let mut bindings=guard.lifecycle_bindings_for(parent)?;
        if role == c"NSResponder" {
            bindings.extend(unsafe { guard.bind_preserved_appkit_proxy_lifecycle(proxy, owned_joint_responder_dealloc) }?);
        }
        for (_,selector,meta,implementation) in window_replacements().into_iter().filter(|(r,_,_,_)|*r==role){
            let signature=signatures.iter().find(|(r,s,_)|*r==role&&*s==selector).ok_or("Window signature absent")?;
            let leaf=unsafe{guard.overlay_bound(proxy,selector,meta,implementation,&signature.2)}?;
            bindings.push(super::receiver::LifecycleBinding::leaf(leaf));
        }
        let grant=unsafe{super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(),proxy,bindings)}?;
        super::factory::publish_proxy(role,Arc::new(grant))?;
    }
    Ok(())
}
/// Prepare the overlapping App/Window/control family under one exclusive claim.
/// # Safety
/// Requires the same exclusive preentry and complete source/lifecycle authority
/// as both individual preparation functions. This grants no Original init call.
#[cfg(any(test, rcf1_reference_sdk))]
pub(crate) unsafe fn prepare_application_window_control_factories_checked(
    model:Arc<Workspace>,prewrite:impl FnOnce()->Result<()>,
) -> Result<super::classes::ClassGuard> {
    deny_foundation_only_factory()?;
    preflight_application_factory()?;
    preflight_window_factories()?;
    let mut classes=Vec::new();
    for name in [c"NSResponder",c"NSApplication",c"NSWindow",c"NSPanel"] {
        let class=crate::sdk::metadata::loaded_class(name)?;
        classes.push(unsafe { &*(class.address() as *const AnyClass) });
    }
    let [responder,application,window,panel]:[&'static AnyClass;4]=classes.try_into()
        .map_err(|_|"joint family length differs")?;
    if application.superclass()!=Some(responder)||window.superclass()!=Some(responder)
        ||panel.superclass()!=Some(window) { return Err("joint App/Window ancestry differs".into()); }
    for parent in [responder,application,window,panel] { super::factory::preflight_default_proxy_arc(parent)?; }
    controls::preflight_control_methods()?;
    let mut control_parents = Vec::new();
    for (role, _) in controls::FACTORY_ROLES {
        let handle = crate::sdk::metadata::loaded_class(role)?;
        let parent = unsafe { &*(handle.address() as *const AnyClass) };
        super::factory::preflight_default_proxy_arc(parent)?;
        control_parents.push(parent);
    }
    let control_parents: [&'static AnyClass; 5] = control_parents.try_into()
        .map_err(|_| "control factory roster length differs")?;
    let [status_bar, status_item, button, menu, item] = control_parents;
    let mut ancestor = button;
    let mut handlers: Vec<(&'static AnyClass, unsafe extern "C" fn(*mut AnyObject,Sel))> = vec![
        (responder,owned_joint_responder_dealloc),(application,owned_application_dealloc),
        (window,owned_window_dealloc),(panel,owned_window_dealloc),
        (status_bar, controls::deallocate), (status_item, controls::deallocate),
        (menu, controls::deallocate), (item, controls::deallocate)];
    for expected in [c"NSStatusBarButton", c"NSButton", c"NSControl", c"NSView"] {
        if ancestor.name() != expected {
            return Err("status button ancestry differs".into());
        }
        handlers.push((ancestor, controls::deallocate));
        ancestor = ancestor.superclass().ok_or("status button ancestor missing")?;
    }
    if ancestor != responder {
        return Err("status button and window responder differ".into());
    }
    let mut parents = vec![responder, application, window, panel];
    parents.extend(control_parents);
    let preserve_reference_views = cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"));
    let guard=unsafe { super::classes::ClassGuard::select_sdk_joint_family_checked(
        &parents,workspace_factory_lease(&model)?,&handlers,prewrite,preserve_reference_views) }?;
    unsafe {
        prepare_application_with_guard(model.clone(),application,&guard)?;
        prepare_windows_with_guard(model.clone(),[responder,window,panel],&guard)?;
        controls::prepare_factories(&model, control_parents, &guard)?;
    }
    Ok(guard)
}
unsafe extern "C" fn owned_joint_responder_dealloc(pointer:*mut AnyObject,_:Sel) {
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe {
        let installed=INSTALLED.get().ok_or("workspace absent during joint cleanup")?;
        let role=super::factory::joint_responder_cleanup_role(pointer,&installed.model)?;
        let serial=super::factory::owned_reference_serial(pointer)?;
        let expected={
            let objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
            match objects.get(&(pointer as usize)) {
                None=>None,
                Some(Kind::ApplicationShell) if role==c"NSApplication"=>Some(Kind::ApplicationShell),
                Some(Kind::WindowPayload(identity)) if [c"NSWindow",c"NSPanel"].contains(&role)
                    &&Arc::ptr_eq(&identity.0.model,&installed.model)
                    &&identity.0.serial.load(Ordering::Acquire)==serial=>Some(Kind::WindowPayload(identity.clone())),
                Some(_)=>return Err("joint responder cleanup kind differs".into()),
            }
        };
        super::factory::deallocate_owned(pointer,|| {
            let removed={
                let mut objects=installed.objects.lock().map_err(|_|"object arena poisoned")?;
                let same=match (objects.get(&(pointer as usize)),expected.as_ref()) {
                    (None,None)|(Some(Kind::ApplicationShell),Some(Kind::ApplicationShell))=>true,
                    (Some(Kind::WindowPayload(actual)),Some(Kind::WindowPayload(before)))=>
                        Arc::ptr_eq(&actual.0,&before.0)&&Arc::ptr_eq(&actual.0.model,&installed.model)
                        &&actual.0.serial.load(Ordering::Acquire)==serial,
                    _=>false,
                };
                if !same {
                    return Err("joint responder cleanup identity changed".into());
                }
                objects.remove(&(pointer as usize))
            };
            if let Some(Kind::WindowPayload(identity))=&removed { identity.0.retire()?; }
            drop(removed);
            Ok(())
        })
    }));
    if !matches!(result,Ok(Ok(()))) {
        if let Err(payload)=result { std::mem::forget(payload); }
        if let Some(installed)=INSTALLED.get() { installed.model.fault(); }
    }
}
pub fn new_owned_window(frame: Rect) -> Result<*mut AnyObject> {
    if !window_scope_active() { return Err("window outside full workspace arena".into()); }
    #[cfg(test)]
    if INSTALLED.get().is_some_and(|installed| installed.fixture_classes) {
        return object(Kind::Window(model()?.new_window(frame)?));
    }
    let source = crate::sdk::metadata::loaded_class(c"NSWindow")?;
    let source = unsafe { &*(source.address() as *const AnyClass) };
    let pointer = unsafe { super::factory::allocate(source) }?;
    unsafe { initialize_window(pointer, to_nsrect(frame), 0, 2, Bool::new(false), std::ptr::null_mut()) }
}

pub fn new_owned_menu() -> Result<*mut AnyObject> {
    deny_foundation_only_factory()?;
    let handle = model()?.with(|s| {
        let h = Handle {
            id: s.next,
            generation: s.generation,
        };
        s.next = s.next.checked_add(1).ok_or("menu ID exhausted")?;
        Ok(h)
    })?;
    object(Kind::Menu(handle))
}
/// Every unresolved leaf is explicit; the common registry must not admit this
/// slice from the smaller implemented replacement list above.
pub fn required_selectors() -> Vec<(&'static CStr, &'static CStr, bool)> {
    let mut all: Vec<_> = replacements()
        .into_iter()
        .map(|(c, s, m, _)| (c, s, m))
        .collect();
    for class in [
        c"NSWorkspace",
        c"NSRunningApplication",
        c"NSScreen",
        c"NSEvent",
        c"NSApplication",
        c"NSWindow",
        c"NSPanel",
        c"NSStatusBar",
        c"NSStatusItem",
        c"NSMenu",
        c"NSMenuItem",
        c"NSWorkspaceOpenConfiguration",
    ] {
        all.push((class, c"initialize", true));
    }
    all.extend([
        (c"NSWorkspace", c"notificationCenter", false),
        (c"NSWorkspace", c"iconForFile:", false),
        (
            c"NSWorkspace",
            c"openApplicationAtURL:configuration:completionHandler:",
            false,
        ),
        (
            c"NSWorkspace",
            c"openURLs:withApplicationAtURL:configuration:completionHandler:",
            false,
        ),
        (c"NSRunningApplication", c"currentApplication", true),
        (c"NSApplication", c"sharedApplication", true),
        (c"NSApplication", c"run", false),
        (c"NSApplication", c"stop:", false),
        (c"NSApplication", c"terminate:", false),
        (c"NSApplication", c"setActivationPolicy:", false),
        (c"NSApplication", c"activationPolicy", false),
        (c"NSApplication", c"activateIgnoringOtherApps:", false),
        (c"NSApplication", c"activate", false),
        (c"NSApplication", c"deactivate", false),
        (c"NSApplication", c"setDelegate:", false),
        (c"NSApplication", c"delegate", false),
        (c"NSApplication", c"mainMenu", false),
        (c"NSApplication", c"setMainMenu:", false),
        (c"NSApplication", c"keyWindow", false),
        (c"NSApplication", c"mainWindow", false),
        (c"NSApplication", c"isActive", false),
        (c"NSStatusBar", c"systemStatusBar", true),
        (c"NSStatusBar", c"statusItemWithLength:", false),
        (c"NSStatusBar", c"removeStatusItem:", false),
        (c"NSStatusItem", c"button", false),
        (c"NSStatusItem", c"setMenu:", false),
        (c"NSStatusItem", c"menu", false),
        (
            c"NSWindow",
            c"initWithContentRect:styleMask:backing:defer:",
            false,
        ),
        (
            c"NSWindow",
            c"initWithContentRect:styleMask:backing:defer:screen:",
            false,
        ),
        (c"NSWindow", c"orderWindow:relativeTo:", false),
        (c"NSWindow", c"deminiaturize:", false),
        (c"NSWindow", c"miniaturize:", false),
        (c"NSWindow", c"setFrame:display:", false),
        (c"NSWindow", c"setFrame:display:animate:", false),
        (c"NSWindow", c"setFrameOrigin:", false),
        (c"NSWindow", c"setContentSize:", false),
        (c"NSWindow", c"center", false),
        (c"NSWindow", c"isKeyWindow", false),
        (c"NSWindow", c"isMainWindow", false),
        (c"NSWindow", c"screen", false),
        (c"NSWindow", c"windowNumber", false),
        (c"NSWindow", c"isMiniaturized", false),
        (c"NSWindow", c"makeFirstResponder:", false),
        (c"NSWindow", c"firstResponder", false),
        (c"NSWindow", c"contentView", false),
        (c"NSWindow", c"setContentView:", false),
        (c"NSWindow", c"setDelegate:", false),
        (c"NSWindow", c"delegate", false),
        (c"NSWorkspaceOpenConfiguration", c"init", false),
    ]);
    all.sort();
    all.dedup();
    all
}
pub fn selector_replacement(class: &CStr, selector: &CStr, class_method: bool) -> Option<usize> {
    replacements()
        .into_iter()
        .find_map(|(c, s, m, p)| (c == class && s == selector && m == class_method).then_some(p))
}
pub fn unmapped_selectors() -> Vec<(&'static CStr, &'static CStr, bool)> {
    required_selectors()
        .into_iter()
        .filter(|(c, s, m)| selector_replacement(c, s, *m).is_none())
        .collect()
}
fn attestation_roles() -> Vec<&'static CStr> {
    required_selectors().into_iter().map(|(role, _, _)| role)
        // Notification observer tokens are returned objects, not selector owners.
        .chain([c"NSObject"])
        .collect::<std::collections::BTreeSet<_>>().into_iter().collect()
}
fn verify_factory_roster(proof: &super::factory::PublishedWorkspaceFactories) -> Result<()> {
    if proof.roles() != attestation_roles() {
        return Err("workspace factory role roster differs".into());
    }
    let required = required_selectors().into_iter().map(|(role, selector, meta)| {
        (role, selector, meta, selector_replacement(role, selector, meta))
    }).collect::<Vec<_>>();
    proof.verify_required_selectors(&required)
}
fn import_keys(rows: &[crate::sdk::ImportReplacement]) -> std::collections::BTreeSet<(&CStr, usize, usize, usize)> {
    rows.iter().map(|row| (row.symbol, row.original, row.replacement, row.cell_address())).collect()
}
pub(crate) fn verify_attestation(module: &crate::sdk::ModuleAttestation) -> Result<()> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if installed.foundation_data_only || installed.apple_event_data_only {
        return Err("data-only arena cannot attest workspace".into());
    }
    let proof = module.workspace_factories.as_ref().ok_or("typed workspace factories absent")?;
    if !proof.belongs_to(&installed.model) { return Err("foreign workspace attestation".into()); }
    verify_factory_roster(proof)?;
    let assembly = installed.assembly.get().ok_or("workspace application/import assembly absent")?;
    #[cfg(rcf1_reference_sdk)]
    crate::sdk::private_network::verify_automation_workspace(&installed.model,proof)?;
    let constants=installed.clipboard_constants.get().ok_or("clipboard constant imports absent")?;
    constants.verify()?;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let worker = unsafe { libc::pthread_main_np() } != 1;
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    let worker = false;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if worker {
        application::verify_attested_shell(&installed.model)?;
    }
    if !worker {
        application::engine()?;
        super::factory::verify_published_callbacks(&installed.model, assembly.catalog.as_ref())?;
    }
    let leaves = proof.leaves()?;
    let keys = |rows: &[crate::sdk::ObjcReplacement]| rows.iter().map(|row|
        (row.class, row.selector, row.class_method, row.original, row.replacement))
        .collect::<std::collections::BTreeSet<_>>();
    if keys(&module.objc) != keys(&leaves) || module.objc.len() != leaves.len()
        || import_keys(&module.imports) != import_keys(&assembly.imports)
        || module.imports.len() != assembly.imports.len()
        || module.imports.is_empty()
        || module.imports.iter().any(|row| {
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            {
                for spec in [
                    crate::sdk::core_preentry::ReferenceAdmission::reference_open_dispatch_spec(),
                    crate::sdk::core_preentry::ReferenceAdmission::reference_main_tabs_deadline_spec(),
                    crate::sdk::core_preentry::ReferenceAdmission::reference_control_wait_spec(),
                    reference_write_spec(),
                    super::factory::reference_edit_super_spec(),
                ] {
                    if row.symbol == spec.symbol {
                        return row.original != spec.original || row.replacement != spec.replacement
                            || row.selected_address() != row.replacement;
                    }
                }
            }
            let expected=if row.symbol==c"NSApp" { application::data_import_replacement(row.symbol) }
                else if row.symbol==c"objc_alloc" { Some(crate::sdk::bootstrap::source_allocation_import_spec().replacement) }
                else if row.symbol==c"objc_allocWithZone" { Some(crate::sdk::bootstrap::source_allocation_with_zone_import_spec().replacement) }
                else { constants.replacement(row.symbol) };
            expected!=Some(row.replacement)||row.selected_address()!=row.replacement
        }) {
        return Err("workspace selected method/data-import roster differs".into());
    }
    proof.verify()?;
    constants.verify_pure()?;
    if worker {
        super::factory::verify_published_callbacks(&installed.model, assembly.catalog.as_ref())?;
    }
    Ok(())
}
/// Construct the module from actual published owners and parsed import cells.
/// No address, lifecycle boolean or receipt can manufacture these proofs.
pub fn attestation() -> Result<crate::sdk::ModuleAttestation> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    let proof = super::factory::attest_published_factories(&installed.model, &attestation_roles())?;
    verify_factory_roster(&proof)?;
    let assembly = installed.assembly.get().ok_or("workspace application/import assembly absent")?;
    let module = crate::sdk::ModuleAttestation {
        module: "workspace",
        scopes: super::scopes(),
        callsites: super::source_roster(),
        imports: assembly.imports.clone(),
        objc: proof.leaves()?,
        handles: vec![installed.model.lease()],
        workspace_factories: Some(proof),
    };
    module.verify()?;
    Ok(module)
}

/// Complete source-prepared factory publication precedes arena/application
/// allocation; actual NSApp cells are patched only after the owned shell exists.
/// # Safety
/// Exclusive, independently source-qualified full workspace setup. All original
/// callback allocation/ivars/lifecycle hooks and the complete typed catalog are
/// already published. `prewrite` is a pure source/metadata validator, valid after
/// every dynamic guardian callback; no original AppKit initializer may run.
pub unsafe fn install_workspace_checked(
    model: Arc<Workspace>,
    catalog: Arc<dyn application::CallbackCatalog>,
    prewrite: impl Fn() -> Result<()> + Send + Sync,
) -> Result<crate::sdk::ModuleAttestation> {
    if INSTALLED.get().is_some() { return Err("workspace already installed".into()); }
    let proof = super::factory::attest_published_factories(&model, &attestation_roles())?;
    verify_factory_roster(&proof)?;
    super::factory::verify_published_callbacks(&model, catalog.as_ref())?;
    prewrite()?;
    install(model.clone())?;
    application::configure_catalog(catalog.clone())?;
    let constants=prepare_clipboard_constants(&model)?;
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" { #[link_name = "NSApp"] static ORIGINAL_NSAPP: *mut AnyObject; }
    struct ImportLease<'a> {
        model: &'a Arc<Workspace>,
        prewrite: &'a (dyn Fn() -> Result<()> + Send + Sync),
        constants: &'a ClipboardConstants,
    }
    impl crate::sdk::owned::AliveLease for ImportLease<'_> {
        fn is_alive(&self) -> bool { self.before_effect().is_ok() }
        fn before_effect(&self) -> Result<()> {
            self.model.lease.before_effect()?;
            self.constants.verify()?;
            application::engine()?;
            (self.prewrite)()?;
            self.constants.verify_pure()?;
            let state = self.model.state.lock().map_err(|_| "workspace state poisoned")?;
            if state.retired || self.model.faulted.load(Ordering::Acquire) {
                return Err("workspace retired before application import write".into());
            }
            Ok(())
        }
    }
    let original = (&raw const ORIGINAL_NSAPP) as usize;
    let replacement = application::data_import_replacement(c"NSApp")
        .ok_or("owned NSApp data export absent")?;
    let lease = ImportLease { model: &model, prewrite: &prewrite,constants:&constants };
    let mut specs=vec![crate::sdk::ImportSpec {
        symbol: c"NSApp", original, replacement,
    }, crate::sdk::bootstrap::source_allocation_import_spec()];
    // Reference imports allocWithZone and requires alloc to be absent.
    // Native retains its existing alloc roster.
    #[cfg(rcf1_reference_sdk)]
    specs.push(crate::sdk::bootstrap::source_allocation_with_zone_import_spec());
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    specs.extend([
        crate::sdk::core_preentry::ReferenceAdmission::reference_open_dispatch_spec(),
        crate::sdk::core_preentry::ReferenceAdmission::reference_main_tabs_deadline_spec(),
        crate::sdk::core_preentry::ReferenceAdmission::reference_control_wait_spec(),
        reference_write_spec(),
        super::factory::reference_edit_super_spec(),
    ]);
    specs.extend(constants.import_specs()?);
    #[cfg(rcf1_reference_sdk)]
    let imports = {
        let selected = specs.iter().copied().filter(|spec| spec.symbol != c"objc_alloc")
            .collect::<Vec<_>>();
        #[cfg(all(not(test), target_arch = "aarch64"))]
        struct EditSuperInstall(bool);
        #[cfg(all(not(test), target_arch = "aarch64"))]
        impl Drop for EditSuperInstall {
            fn drop(&mut self) {
                if !self.0 {
                    let _ = super::factory::advance_reference_edit_super_install(1, 3);
                }
            }
        }
        #[cfg(all(not(test), target_arch = "aarch64"))]
        let mut edit_super = {
            super::factory::advance_reference_edit_super_install(0, 1)?;
            EditSuperInstall(false)
        };
        let imports = crate::sdk::import_patch::install_exact_presence(&selected, &specs, &lease)?;
        #[cfg(all(not(test), target_arch = "aarch64"))]
        {
            super::factory::advance_reference_edit_super_install(1, 2)?;
            edit_super.0 = true;
        }
        imports
    };
    #[cfg(not(rcf1_reference_sdk))]
    let imports = crate::sdk::import_patch::install(&specs, &lease)?;
    INSTALLED.get().ok_or("workspace disappeared")?.assembly
        .set(WorkspaceAssembly { catalog, imports })
        .map_err(|_| "workspace assembly already published")?;
    attestation()
}

#[cfg(test)]
mod tests {
    #[cfg(all(rcf1_reference_sdk, target_arch = "aarch64"))]
    #[test]
    #[ignore = "isolated AppKit lifecycle transaction with process-wide class mutations"]
    fn reference_running_application_first_message_on_worker() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let model = fixture.model.clone();
        let (handle, app) = model.applications().unwrap().remove(0);
        unsafe {
            prepare_running_and_screen_factories_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        install_arena(model.clone(), false).unwrap();
        unsafe {
            prepare_typed_data_factories_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        let address = std::thread::spawn(move || objc2::rc::autoreleasepool(|_| {
            let class = crate::sdk::metadata::loaded_class(c"NSRunningApplication").unwrap();
            let class = unsafe { &*(class.address() as *const AnyClass) };
            let selected: *const AnyClass = unsafe { objc2::msg_send![class, class] };
            assert_eq!(selected, class as *const AnyClass);
            let missing: *mut AnyObject = unsafe { objc2::msg_send![class, runningApplicationWithProcessIdentifier:0i32] };
            assert!(missing.is_null());
            let application: *mut AnyObject = unsafe { objc2::msg_send![class, runningApplicationWithProcessIdentifier:app.pid] };
            assert!(!application.is_null());
            let pid: i32 = unsafe { objc2::msg_send![application, processIdentifier] };
            assert_eq!(pid, app.pid);
            let name: *mut AnyObject = unsafe { objc2::msg_send![application, localizedName] };
            assert_eq!(typed_data::foundation_utf16(name).unwrap(),
                app.name.unwrap().encode_utf16().collect::<Vec<_>>());
            application as usize
        })).join().unwrap();
        let application = object(Kind::Application(handle)).unwrap();
        assert_eq!(application as usize, address);
        assert_eq!(foundation_reference(application).unwrap().as_ptr().unwrap(), application);
        assert!(!model.faulted.load(Ordering::Acquire));
    }

    #[test]
    #[ignore = "isolated factory transaction with compiler-bound NSString declarations"]
    fn owned_bundle_icon_selector_returns_tiff_and_releases_both_receivers() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let mut app = fixture.model.installed_application("dev.fixture.Owned").unwrap().unwrap();
        app.icon_png = include_bytes!("../../../../assets/app-icon.png").to_vec();
        let path = app.bundle_path.clone().unwrap();
        let model = Workspace::new(
            fixture.model.context.clone(), vec![app],
            fixture.model.with(|state| Ok(state.screens.clone())).unwrap(),
            fixture.model.mouse().unwrap(),
        ).unwrap();
        model.prepare_bundle_icons().unwrap();
        unsafe {
            prepare_notification_and_workspace_factories_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        install_arena(model.clone(), false).unwrap();
        unsafe {
            prepare_typed_data_factories_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        let handle = crate::sdk::metadata::loaded_class(c"NSWorkspace").unwrap();
        let class = unsafe { &*(handle.address() as *const AnyClass) };
        let (icon, tiff) = objc2::rc::autoreleasepool(|_| {
            let workspace: *mut AnyObject = unsafe { objc2::msg_send![class, sharedWorkspace] };
            assert_eq!(qualified_arena_reference_role(workspace).unwrap(), c"NSWorkspace");
            let path = string(path.to_str().unwrap().into());
            assert!(!path.is_null());
            let icon: *mut AnyObject = unsafe { objc2::msg_send![workspace, iconForFile: path] };
            assert_eq!(qualified_arena_reference_role(icon).unwrap(), c"NSImage");
            let tiff: *mut AnyObject = unsafe { objc2::msg_send![icon, TIFFRepresentation] };
            let bytes = foundation_bytes(tiff).unwrap();
            assert_eq!(bytes, image_record(icon).unwrap().tiff().unwrap());
            let foreign = string("/Applications/Foreign.app".into());
            let refused: *mut AnyObject = unsafe { objc2::msg_send![workspace, iconForFile: foreign] };
            assert!(refused.is_null());
            (icon as usize, tiff as usize)
        });
        let arena = INSTALLED.get().unwrap().objects.lock().unwrap();
        assert!(!arena.contains_key(&icon));
        assert!(!arena.contains_key(&tiff));
        assert!(!model.faulted.load(Ordering::Acquire));
    }

    #[test]
    #[ignore = "isolated workspace lifecycle transaction with process-wide class mutations"]
    fn workspace_open_configuration_uses_its_owned_init_and_cleanup() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let model = fixture.model.clone();
        unsafe {
            prepare_notification_and_workspace_factories_checked(model.clone(), || {
                model.with(|_| Ok(()))
            })
        }.unwrap();
        assert!(INSTALLED.get().is_none());
        install_arena(model.clone(), false).unwrap();
        let handle = crate::sdk::metadata::loaded_class(c"NSWorkspaceOpenConfiguration").unwrap();
        let class = unsafe { &*(handle.address() as *const AnyClass) };
        let pointer = unsafe { super::super::factory::allocate(class) }.unwrap();
        let initialized: *mut AnyObject = unsafe { objc2::msg_send![pointer, init] };
        assert_eq!(pointer, initialized);
        assert_eq!(qualified_arena_reference_role(initialized).unwrap(), c"NSWorkspaceOpenConfiguration");
        let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(initialized) }.unwrap();
        drop(pending);
        assert!(!INSTALLED.get().unwrap().objects.lock().unwrap().contains_key(&(pointer as usize)));
        let image = super::super::images::decode_png(
            include_bytes!("../../../../assets/app-icon.png"), model.lease(),
        ).unwrap();
        let icon = object(Kind::BundleIcon(IconIdentity(image.icon_pixels()))).unwrap();
        assert_eq!(qualified_arena_reference_role(icon).unwrap(), c"NSImage");
        let size: NSSize = unsafe { objc2::msg_send![icon, size] };
        assert_eq!(size, NSSize::new(image.width as f64, image.height as f64));
        drop(unsafe { super::super::factory::OwnedReceiver::from_owned(icon) }.unwrap());
        assert!(!INSTALLED.get().unwrap().objects.lock().unwrap().contains_key(&(icon as usize)));
        assert!(!model.faulted.load(Ordering::Acquire));
    }

    #[test]
    #[ignore = "isolated NSEvent lifecycle transaction with process-wide class mutations"]
    fn event_factory_binds_mouse_location_to_owned_fixture() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let model = fixture.model.clone();
        unsafe {
            prepare_event_data_factory_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        let proof = super::super::factory::attest_published_factories(&model, &[c"NSEvent"]).unwrap();
        let mut required = event_methods::replacements().into_iter()
            .map(|(role, selector, meta, implementation)| (role, selector, meta, Some(implementation)))
            .collect::<Vec<_>>();
        required.push((c"NSEvent", c"initialize", true, None));
        proof.verify_required_selectors(&required).unwrap();
        install_arena(model.clone(), false).unwrap();
        let loaded = crate::sdk::metadata::loaded_class(c"NSEvent").unwrap();
        let class = unsafe { &*(loaded.address() as *const AnyClass) };
        let actual: NSPoint = unsafe { objc2::msg_send![class, mouseLocation] };
        assert_eq!(actual, NSPoint::new(50.0, 60.0));
        model.retire().unwrap();
        let refused: NSPoint = unsafe { objc2::msg_send![class, mouseLocation] };
        assert_eq!(refused, NSPoint::new(0.0, 0.0));
        assert!(proof.verify().is_err());
    }

    #[test]
    #[ignore = "isolated AppKit lifecycle transaction with process-wide class mutations"]
    fn joint_appkit_preparation_publishes_all_five_control_factories() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let model = fixture.model.clone();
        unsafe {
            prepare_application_window_control_factories_checked(model.clone(), || {
                model.with(|_| Ok(()))
            })
        }.unwrap();
        let roles = controls::FACTORY_ROLES.map(|(role, _)| role);
        let proof = super::super::factory::attest_published_factories(&model, &roles).unwrap();
        let leaves = proof.leaves().unwrap();
        for (role, selector, meta, implementation) in controls::replacements() {
            let owner = if meta {
                role
            } else {
                controls::FACTORY_ROLES.iter().find(|row| row.0 == role).unwrap().1
            };
            assert_eq!(leaves.iter().filter(|leaf| {
                leaf.class == owner && leaf.selector == selector &&
                    leaf.class_method == meta && leaf.replacement == implementation
            }).count(), 1, "{role:?}/{selector:?}/{meta}");
        }
        let mut required = controls::replacements().into_iter()
            .map(|(role, selector, meta, implementation)| (role, selector, meta, Some(implementation)))
            .collect::<Vec<_>>();
        required.extend(roles.map(|role| (role, c"initialize", true, None)));
        proof.verify_required_selectors(&required).unwrap();
        let (role, selector, meta, implementation) = required[0];
        assert!(proof.verify_required_selectors(&[
            (role, selector, meta, implementation.map(|address| address ^ 4)),
        ]).is_err());
        assert!(proof.verify_required_selectors(&[
            (c"NSMissingRuycastFactory", selector, meta, implementation),
        ]).is_err());
        assert!(proof.verify_required_selectors(&[
            (role, c"initialize", false, None),
        ]).is_err());
        assert!(INSTALLED.get().is_none());
    }

    #[test]
    fn fixed_task_roster_metadata_inventory_does_not_initialize_or_allocate() {
        use super::*;
        assert!(INSTALLED.get().is_none());
        let rows=task_factory_rows();
        assert_eq!(rows.len(),40);
        let signatures=task_factory_signatures();
        let keys:std::collections::BTreeSet<_>=rows.iter()
            .map(|(role,selector,meta,_)|(*role,*selector,*meta)).collect();
        assert_eq!(keys.len(),40,"fixed Task roster contains duplicate keys");
        assert_eq!(signatures.len(),40,"fixed Task signature roster differs");
        for (role,selector,meta) in &keys {
            assert_eq!(signatures.iter().filter(|(r,s,m,_)|
                r==role&&s==selector&&m==meta).count(),1,"fixed Task ABI is not unique");
        }
        let mut passes=Vec::new();
        let mut receipts=Vec::new();
        for pass in 0..2 {
            let mut observations=Vec::new();
            let mut receipt=Vec::new();
            for (role,selector,meta,implementation) in &rows {
                let signature=signatures.iter().find(|(r,s,m,_)|r==role&&s==selector&&m==meta)
                    .expect("fixed Task declaration ABI missing");
                let actual=crate::sdk::metadata::loaded_class(role)
                    .and_then(|class|crate::sdk::metadata::method(class,selector,*meta));
                let description=match &actual {
                    Ok(Some(method))=>serde_json::json!({"present":true,"owner":method.owner(),
                        "method":method.method_address(),"implementation":method.implementation(),
                        "encoding":method.encoding(),"snapshot":format!("{method:?}"),
                        "code_identity":format!("{:?}",method.code()),
                        "abi_valid":super::super::classes::verify_typed_abi(method,&signature.3).is_ok()}),
                    Ok(None)=>serde_json::json!({"present":false}),
                    Err(error)=>serde_json::json!({"lookup_error":error}),
                };
                receipt.push(serde_json::json!({"pass":pass,"class":role.to_str().unwrap(),
                    "selector":selector.to_str().unwrap(),"class_method":meta,
                    "declared_abi":format!("{:?}",signature.3),"replacement":implementation,
                    "observation":description}));
                observations.push(actual);
            }
            passes.push(observations);
            receipts.push(receipt);
        }
        println!("{}",serde_json::json!({"fixed_task_roster_inventory":receipts,
            "scope":"two full fixed40 metadata passes; no class selection/writes/instances/resolver/messages"}));
        assert_eq!(passes[0],passes[1],"fixed metadata changed between independent passes");
        assert!(INSTALLED.get().is_none());
    }
    #[test]
    fn actual_task_factories_keep_thread_and_cleanup_authorities_separate() {
        use super::*;
        use sha2::Digest;
        let fixture=super::super::tests::Fixture::new();
        let life=fixture.model.lease().life;
        let registry=Arc::new(crate::sdk::process_dispatch::Registry::new(life.clone(),
            crate::sdk::descriptors::DescriptorTable::new()).unwrap());
        crate::sdk::process_abi::install(crate::sdk::process_abi::Context {
            registry:registry.clone(),life:life.clone(),owner_uid:unsafe {libc::getuid()},
        }).unwrap();
        let source_path=concat!(env!("CARGO_MANIFEST_DIR"),"/src/sdk/workspace_objc.rs");
        let source_hash=sha2::Sha256::digest(include_bytes!("workspace_objc.rs"));
        unsafe {prepare_task_factories_checked(fixture.model.clone(),|| {
            use sha2::Digest;
            if sha2::Sha256::digest(std::fs::read(source_path).map_err(|e|e.to_string())?)!=source_hash {
                return Err("Task canary source changed before class write".into());
            }
            Ok(())
        })}.unwrap();
        // Use the actual sole arena, without installing an App/Window or
        // inventing a callback catalog/whole-Source14 admission.
        install_arena(fixture.model.clone(),false).unwrap();
        unsafe { prepare_typed_data_factories_checked(fixture.model.clone(), || Ok(())) }.unwrap();
        let source_class=|name:&CStr| {
            let handle=crate::sdk::metadata::loaded_class(name).unwrap();
            unsafe {&*(handle.address() as *const AnyClass)}
        };
        let task_pointer=unsafe {super::super::factory::allocate(source_class(c"NSTask"))}.unwrap();
        // Enter the exact bound own ObjC initializer on its creating owner
        // before any worker message; do not bypass runtime initialization.
        let task_pointer:*mut AnyObject=unsafe {objc2::msg_send![task_pointer,init]};
        assert!(!task_pointer.is_null());
        let task_owner=unsafe {super::super::factory::OwnedReceiver::from_owned(task_pointer)}.unwrap();
        assert!(!super::super::factory::is_immutable_data(task_pointer,&fixture.model).unwrap());
        assert!(task_fields(task_pointer).is_ok());
        let task_address=task_pointer as usize;
        std::thread::scope(|scope| {
            scope.spawn(move|| {
                let pointer=task_address as *mut AnyObject;
                let running:Bool=unsafe {objc2::msg_send![pointer,isRunning]};
                assert!(!running.as_bool());
                let _:()=unsafe {objc2::msg_send![pointer,terminate]};
                assert!(task_fields(pointer).is_err());
                assert!(super::super::factory::receiver_identity(pointer).is_err());
                let retained=unsafe {super::super::factory::retain_owned(pointer)}.unwrap();
                unsafe {super::super::factory::release_owned(retained)}.unwrap();
            }).join().unwrap();
        });
        // A worker policy refusal must not retire the owner's Task authority.
        assert!(task_fields(task_pointer).is_ok());
        std::thread::scope(|scope| {
            scope.spawn(|| objc2::rc::autoreleasepool(|_| {
                // The original Shortcuts worker creates these objects itself.
                // Worker-created mutable payloads must not require main custody.
                let task = unsafe { super::super::factory::allocate(source_class(c"NSTask")) }.unwrap();
                let task: *mut AnyObject = unsafe { objc2::msg_send![task, init] };
                let _owner = unsafe { super::super::factory::OwnedReceiver::from_owned(task) }.unwrap();
                let path = Text::from_utf8(b"/usr/bin/shortcuts").unwrap();
                let url = new_foundation_value(FoundationValue::Url(Arc::new(
                    foundation::Url::file_path(&path, false).unwrap(),
                ))).unwrap();
                let _url_owner = unsafe { super::super::factory::OwnedReceiver::from_owned(url) }.unwrap();
                let mut arguments = Vec::new();
                for value in ["list", "--show-identifiers"] {
                    let text = new_foundation_value(FoundationValue::String(Arc::new(
                        Text::from_utf8(value.as_bytes()).unwrap(),
                    ))).unwrap();
                    let _text_owner = unsafe { super::super::factory::OwnedReceiver::from_owned(text) }.unwrap();
                    arguments.push(foundation_reference(text).unwrap());
                }
                let arguments = new_foundation_array(arguments).unwrap();
                let _argument_owner = unsafe { super::super::factory::OwnedReceiver::from_owned(arguments) }.unwrap();
                let _: () = unsafe { objc2::msg_send![task, setExecutableURL: url] };
                let _: () = unsafe { objc2::msg_send![task, setArguments: arguments] };
                let fields = task_fields(task).unwrap();
                {
                    let config = fields.task.config.lock().unwrap();
                    assert_eq!(config.executable, std::path::Path::new("/usr/bin/shortcuts"));
                    assert_eq!(config.arguments, ["list", "--show-identifiers"]);
                }
                let pipe: *mut AnyObject = unsafe { objc2::msg_send![source_class(c"NSPipe"), pipe] };
                assert!(!pipe.is_null());
                let _: () = unsafe { objc2::msg_send![task, setStandardOutput: pipe] };
                let output: *mut AnyObject = unsafe { objc2::msg_send![task, standardOutput] };
                assert_eq!(output, pipe);
                let reader: *mut AnyObject = unsafe { objc2::msg_send![pipe, fileHandleForReading] };
                assert!(!reader.is_null());
                assert!(handle(reader).is_ok());
            })).join().unwrap();
        });
        // This cap, not a Send raw receiver, owns the last child +1 on a worker.
        let null_pointer=objc2::rc::autoreleasepool(|_| {
            let pointer:*mut AnyObject=unsafe {objc2::msg_send![source_class(c"NSFileHandle"),fileHandleWithNullDevice]};
            assert!(!pointer.is_null());
            // The actual public class factory returns +0; preserve exactly one
            // qualified +1 before draining this inner pool.
            unsafe {super::super::factory::retain_owned(pointer)}.unwrap()
        });
        let null_owner=unsafe {super::super::factory::OwnedReceiver::from_owned(null_pointer)}.unwrap();
        let null_cap=super::super::factory::TaskChildReference::from_owned_child(null_pointer).unwrap();
        drop(null_owner);
        std::thread::spawn(move||drop(null_cap)).join().unwrap();
        assert!(super::super::factory::receiver_identity(null_pointer).is_err());
        let task_class=unsafe {&*task_pointer}.class();
        let (pipe_class,handle_class,error_class)=objc2::rc::autoreleasepool(|_| {
            let returned_pipe:*mut AnyObject=unsafe {objc2::msg_send![source_class(c"NSPipe"),pipe]};
            assert!(!returned_pipe.is_null());
            let pipe_pointer=unsafe {super::super::factory::retain_owned(returned_pipe)}.unwrap();
            let pipe_owner=unsafe {super::super::factory::OwnedReceiver::from_owned(pipe_pointer)}.unwrap();
            let read1=task_pipe_handle(pipe_pointer,false).unwrap();
            let read2=task_pipe_handle(pipe_pointer,false).unwrap();
            assert_eq!(read1,read2);
            let write=task_pipe_handle(pipe_pointer,true).unwrap();
            let payload = b"/fixture/Ruycast Fixture.txt\0";
            handle(write).unwrap().write(payload, life.as_ref()).unwrap();
            let mut read_error = std::ptr::null_mut::<AnyObject>();
            let read: *mut AnyObject = unsafe {
                objc2::msg_send![read1, readDataUpToLength: 16384usize, error: &mut read_error as *mut _]
            };
            assert!(!read.is_null() && read_error.is_null());
            let length: usize = unsafe { objc2::msg_send![read, length] };
            let bytes: *const std::ffi::c_void = unsafe { objc2::msg_send![read, bytes] };
            assert_eq!(length, payload.len());
            assert!(!bytes.is_null());
            assert_eq!(unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }, payload);
            let failure=handle(write).unwrap().read_failure(1,life.as_ref()).unwrap_err();
            let expected=failure.ns_error_fields().unwrap();
            assert_eq!((expected.domain.as_str(),expected.code),("NSCocoaErrorDomain",256));
            cleanup_task_handle(write).unwrap().close().unwrap();
            let eof: *mut AnyObject = unsafe {
                objc2::msg_send![read1, readDataUpToLength: 16384usize, error: &mut read_error as *mut _]
            };
            assert!(!eof.is_null() && read_error.is_null());
            let eof_length: usize = unsafe { objc2::msg_send![eof, length] };
            assert_eq!(eof_length, 0);
            let error_pointer=new_task_error(&failure).unwrap();
            let error_owner=unsafe {super::super::factory::OwnedReceiver::from_owned(error_pointer)}.unwrap();
            assert!(super::super::factory::is_immutable_data(error_pointer,&fixture.model).unwrap());
            let error_address=error_pointer as usize;
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let pointer=error_address as *mut AnyObject;
                    assert_eq!(task_error_fields(pointer).unwrap(),expected);
                    let code:isize=unsafe {objc2::msg_send![pointer,code]};
                    assert_eq!(code,256);
                }).join().unwrap();
            });
            let classes=(unsafe {&*pipe_pointer}.class(),unsafe {&*write}.class(),unsafe {&*error_pointer}.class());
            registry.retire();
            assert!(task(task_pointer).is_err());
            assert!(handle(write).is_err());
            cleanup_task_handle(write).unwrap().close_failure().unwrap();
            // Error value reads do not revive retired process business.
            assert_eq!(task_error_fields(error_pointer).unwrap(),expected);
            fixture.model.retire().unwrap();
            drop(error_owner);
            drop(pipe_owner);
            drop(task_owner);
            classes
        });
        for class in [task_class,pipe_class,handle_class,error_class] {
            assert_eq!(super::super::factory::live_owned_allocations(class).unwrap(),0);
        }
        assert_eq!(crate::sdk::ruycast_f1_preentry_ready_v1(),0);
    }
    #[test]
    fn original_clipboard_constant_values_are_read_only_source_observations() {
        use super::*;
        use sha2::{Digest, Sha256};
        #[link(name="AppKit",kind="framework")]
        unsafe extern "C" {
            static NSPasteboardTypeString: *mut AnyObject;
            static NSPasteboardTypeHTML: *mut AnyObject;
            static NSPasteboardTypePNG: *mut AnyObject;
            static NSPasteboardTypeTIFF: *mut AnyObject;
            static NSPasteboardURLReadingFileURLsOnlyKey: *mut AnyObject;
        }
        let header_path="/Library/Developer/CommandLineTools/SDKs/MacOSX26.2.sdk/System/Library/Frameworks/AppKit.framework/Versions/C/Headers/NSPasteboard.h";
        let header=std::fs::read(header_path).unwrap();
        let header_hash=format!("{:x}",Sha256::digest(&header));
        let declarations=std::str::from_utf8(&header).unwrap();
        assert!(declarations.contains("typedef NSString * NSPasteboardType"));
        assert!(declarations.contains("typedef NSString * NSPasteboardReadingOptionKey"));
        let entries=unsafe { [
            (c"NSPasteboardTypeString",std::ptr::addr_of!(NSPasteboardTypeString),NSPasteboardTypeString),
            (c"NSPasteboardTypeHTML",std::ptr::addr_of!(NSPasteboardTypeHTML),NSPasteboardTypeHTML),
            (c"NSPasteboardTypePNG",std::ptr::addr_of!(NSPasteboardTypePNG),NSPasteboardTypePNG),
            (c"NSPasteboardTypeTIFF",std::ptr::addr_of!(NSPasteboardTypeTIFF),NSPasteboardTypeTIFF),
            (c"NSPasteboardURLReadingFileURLsOnlyKey",std::ptr::addr_of!(NSPasteboardURLReadingFileURLsOnlyKey),NSPasteboardURLReadingFileURLsOnlyKey),
        ] };
        let string_class=crate::sdk::metadata::loaded_class(c"NSString").unwrap();
        let mut observations=vec![];
        for (index,(symbol,slot,pointer)) in entries.into_iter().enumerate() {
            assert_eq!(CLIPBOARD_CONSTANTS[index].0,symbol);
            assert_eq!(original_clipboard_slots()[index],slot as usize);
            assert!(declarations.contains(symbol.to_str().unwrap()));
            assert!(!pointer.is_null());
            let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
            assert_ne!(unsafe { libc::dladdr(slot.cast(),info.as_mut_ptr()) },0);
            let info=unsafe {info.assume_init()};
            let image=unsafe {std::ffi::CStr::from_ptr(info.dli_fname)}.to_bytes().to_vec();
            assert!(image.starts_with(b"/System/Library/Frameworks/AppKit.framework/"));
            assert!(!info.dli_fbase.is_null());
            let object=unsafe { &*pointer };
            let class=object.class();
            let handle=crate::sdk::metadata::registered_class(class as *const AnyClass as usize).unwrap();
            let mut ancestor=Some(class);
            let mut matches=false;
            for _ in 0..256 {
                let Some(current)=ancestor else {break};
                if current as *const AnyClass as usize==string_class.address(){matches=true;break;}
                ancestor=current.superclass();
            }
            assert!(matches);
            let lifecycle=crate::sdk::metadata::lifecycle(handle).unwrap();
            let length=crate::sdk::metadata::method(handle,c"length",false).unwrap().unwrap();
            let character=crate::sdk::metadata::method(handle,c"characterAtIndex:",false).unwrap().unwrap();
            type Length=unsafe extern "C" fn(*mut AnyObject,Sel)->usize;
            type Character=unsafe extern "C" fn(*mut AnyObject,Sel,usize)->u16;
            super::super::classes::verify_typed_abi(&length,&window_signature::<Length>()).unwrap();
            super::super::classes::verify_typed_abi(&character,&window_signature::<Character>()).unwrap();
            let read_length:Length=unsafe {std::mem::transmute(length.implementation())};
            let read_character:Character=unsafe {std::mem::transmute(character.implementation())};
            let count=unsafe {read_length(pointer,objc2::sel!(length))};
            assert!((1..=4096).contains(&count));
            let mut utf16=Vec::with_capacity(count);
            for index in 0..count {
                utf16.push(unsafe {read_character(pointer,objc2::sel!(characterAtIndex:),index)});
            }
            crate::sdk::metadata::verify_current_method(handle,c"length",false,&length).unwrap();
            crate::sdk::metadata::verify_current_method(handle,c"characterAtIndex:",false,&character).unwrap();
            assert_eq!(crate::sdk::metadata::lifecycle(handle).unwrap(),lifecycle);
            assert_eq!(unsafe {slot.read()},pointer);
            assert_eq!(unsafe {&*pointer}.class(),class);
            let value=String::from_utf16(&utf16).unwrap();
            assert_eq!(value,CLIPBOARD_CONSTANTS[index].1);
            observations.push(serde_json::json!({"symbol":symbol.to_str().unwrap(),
                "slot":slot as usize,"object":pointer as usize,"image":String::from_utf8(image).unwrap(),
                "header_sha256":header_hash,"class":class.name().to_str().unwrap(),
                "length_imp":length.implementation(),"character_imp":character.implementation(),
                "utf16":utf16,"value":value}));
        }
        assert_eq!(format!("{:x}",Sha256::digest(std::fs::read(header_path).unwrap())),header_hash);
        println!("{}",serde_json::json!({"clipboard_constant_observations":observations,
            "scope":"five fixed AppKit NSString globals; public length/character getters only"}));
    }
    #[test]
    fn original_error_description_key_is_a_read_only_source_observation() {
        use super::*;
        use sha2::{Digest, Sha256};
        #[link(name="Foundation",kind="framework")]
        unsafe extern "C" {
            static NSLocalizedDescriptionKey: *mut AnyObject;
        }
        let header_path="/Library/Developer/CommandLineTools/SDKs/MacOSX26.2.sdk/System/Library/Frameworks/Foundation.framework/Versions/C/Headers/NSError.h";
        let header=std::fs::read(header_path).unwrap();
        let header_hash=format!("{:x}",Sha256::digest(&header));
        let declarations=std::str::from_utf8(&header).unwrap();
        assert!(declarations.contains("typedef NSString *NSErrorUserInfoKey"));
        let entries=unsafe { [
            (c"NSLocalizedDescriptionKey",std::ptr::addr_of!(NSLocalizedDescriptionKey),NSLocalizedDescriptionKey),
        ] };
        let string_class=crate::sdk::metadata::loaded_class(c"NSString").unwrap();
        let mut observations=vec![];
        for (symbol,slot,pointer) in entries {
            assert!(declarations.contains(symbol.to_str().unwrap()));
            assert!(!pointer.is_null());
            let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
            assert_ne!(unsafe { libc::dladdr(slot.cast(),info.as_mut_ptr()) },0);
            let info=unsafe {info.assume_init()};
            let image=unsafe {std::ffi::CStr::from_ptr(info.dli_fname)}.to_bytes().to_vec();
            assert!(image.starts_with(b"/System/Library/Frameworks/Foundation.framework/"));
            assert!(!info.dli_fbase.is_null());
            let object=unsafe { &*pointer };
            let class=object.class();
            let handle=crate::sdk::metadata::registered_class(class as *const AnyClass as usize).unwrap();
            let mut ancestor=Some(class);
            let mut matches=false;
            for _ in 0..256 {
                let Some(current)=ancestor else {break};
                if current as *const AnyClass as usize==string_class.address(){matches=true;break;}
                ancestor=current.superclass();
            }
            assert!(matches);
            let lifecycle=crate::sdk::metadata::lifecycle(handle).unwrap();
            let length=crate::sdk::metadata::method(handle,c"length",false).unwrap().unwrap();
            let character=crate::sdk::metadata::method(handle,c"characterAtIndex:",false).unwrap().unwrap();
            type Length=unsafe extern "C" fn(*mut AnyObject,Sel)->usize;
            type Character=unsafe extern "C" fn(*mut AnyObject,Sel,usize)->u16;
            super::super::classes::verify_typed_abi(&length,&window_signature::<Length>()).unwrap();
            super::super::classes::verify_typed_abi(&character,&window_signature::<Character>()).unwrap();
            let read_length:Length=unsafe {std::mem::transmute(length.implementation())};
            let read_character:Character=unsafe {std::mem::transmute(character.implementation())};
            let count=unsafe {read_length(pointer,objc2::sel!(length))};
            assert!((1..=4096).contains(&count));
            let mut utf16=Vec::with_capacity(count);
            for index in 0..count {
                utf16.push(unsafe {read_character(pointer,objc2::sel!(characterAtIndex:),index)});
            }
            crate::sdk::metadata::verify_current_method(handle,c"length",false,&length).unwrap();
            crate::sdk::metadata::verify_current_method(handle,c"characterAtIndex:",false,&character).unwrap();
            assert_eq!(crate::sdk::metadata::lifecycle(handle).unwrap(),lifecycle);
            assert_eq!(unsafe {slot.read()},pointer);
            assert_eq!(unsafe {&*pointer}.class(),class);
            let value=String::from_utf16(&utf16).unwrap();
            observations.push(serde_json::json!({"symbol":symbol.to_str().unwrap(),
                "slot":slot as usize,"object":pointer as usize,"image":String::from_utf8(image).unwrap(),
                "header_sha256":header_hash,"class":class.name().to_str().unwrap(),
                "length_imp":length.implementation(),"character_imp":character.implementation(),
                "utf16":utf16,"value":value}));
        }
        assert_eq!(format!("{:x}",Sha256::digest(std::fs::read(header_path).unwrap())),header_hash);
        println!("{}",serde_json::json!({"error_description_key_observations":observations,
            "scope":"one fixed Foundation NSString global; public length/character getters only"}));
    }
    #[test]
    fn window_preflight_is_exactly_68_typed_source_methods_without_factory_authority() {
        let methods=super::preflight_window_factories().unwrap();
        assert_eq!(methods.len(),68);
        let mut keys=std::collections::BTreeSet::new();
        for method in methods { assert!(keys.insert((method.role,method.selector))); }
        assert!(super::super::factory::attest_published_factories(
            &super::super::tests::Fixture::new().model,&[c"NSWindow",c"NSPanel"]).is_err());
    }
    #[test]
    #[ignore = "isolated window lifecycle transaction with process-wide class mutations"]
    fn window_factories_bind_ordering_for_windows_and_panels() {
        use super::*;
        let fixture = super::super::tests::Fixture::new();
        let model = fixture.model.clone();
        unsafe {
            prepare_window_factories_checked(model.clone(), || model.with(|_| Ok(())))
        }.unwrap();
        let proof = super::super::factory::attest_published_factories(
            &model, &[c"NSResponder", c"NSWindow", c"NSPanel"],
        ).unwrap();
        let required = window_replacements().into_iter()
            .map(|(role, selector, meta, implementation)| (role, selector, meta, Some(implementation)))
            .collect::<Vec<_>>();
        proof.verify_required_selectors(&required).unwrap();
        install_arena(model.clone(), false).unwrap();
        for role in [c"NSWindow", c"NSPanel"] {
            let loaded = crate::sdk::metadata::loaded_class(role).unwrap();
            let class = unsafe { &*(loaded.address() as *const AnyClass) };
            let pointer = unsafe { super::super::factory::allocate(class) }.unwrap();
            let pointer: *mut AnyObject = unsafe {
                objc2::msg_send![pointer,
                    initWithContentRect: NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(300.0, 200.0)),
                    styleMask: 0usize, backing: 2usize, defer: Bool::new(false)]
            };
            assert!(!pointer.is_null());
            let record = window_identity(pointer).unwrap();
            unsafe {
                let _: () = objc2::msg_send![pointer, orderFront: std::ptr::null_mut::<AnyObject>()];
                let visible: Bool = objc2::msg_send![pointer, isVisible];
                assert!(visible.as_bool());
                let _: () = objc2::msg_send![pointer, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
                assert!(model.window_snapshot(record.handle).unwrap().key);
                let _: () = objc2::msg_send![pointer, orderOut: std::ptr::null_mut::<AnyObject>()];
                let visible: Bool = objc2::msg_send![pointer, isVisible];
                assert!(!visible.as_bool());
                let _: () = objc2::msg_send![pointer, orderFrontRegardless];
                let _: () = objc2::msg_send![pointer, makeKeyWindow];
                assert!(model.window_snapshot(record.handle).unwrap().key);
                let _: () = objc2::msg_send![pointer, close];
                let visible: Bool = objc2::msg_send![pointer, isVisible];
                assert!(!visible.as_bool());
                drop(super::super::factory::OwnedReceiver::from_owned(pointer).unwrap());
            }
            assert!(model.window_snapshot(record.handle).is_err());
            assert!(!INSTALLED.get().unwrap().objects.lock().unwrap().contains_key(&(pointer as usize)));
        }
        assert!(!model.faulted.load(Ordering::Acquire));
    }
    #[test]
    fn data_only_window_imps_refuse_before_decoder_or_life_and_do_not_poison_data() {
        let fixture=super::super::tests::Fixture::new();
        unsafe{super::install_foundation_data(fixture.model.clone())}.unwrap();
        let before=fixture.model.state.lock().unwrap().next;
        let pointer=std::ptr::dangling_mut();
        let invalid=NSRect::new(NSPoint::new(f64::NAN,f64::INFINITY),NSSize::new(-1.0,-1.0));
        assert!(unsafe{super::window_init_screen(pointer,objc2::sel!(initWithContentRect:styleMask:backing:defer:screen:),invalid,usize::MAX,usize::MAX,Bool::new(true),pointer)}.is_null());
        assert_eq!(fixture.model.state.lock().unwrap().next,before);
        assert!(!fixture.model.faulted.load(Ordering::Acquire));
    }
    #[test]
    fn foundation_only_preflight_has_34_originals_and_one_owned_declaration() {
        let observed = super::preflight_foundation_factories().unwrap();
        assert_eq!(observed.len(), 35);
        assert_eq!(
            observed
                .iter()
                .filter(|(_, method)| matches!(
                    method,
                    super::TypedDataMethodPreflight::Existing(_)
                ))
                .count(),
            34
        );
        assert_eq!(
            observed
                .iter()
                .filter(|(_, method)| matches!(
                    method,
                    super::TypedDataMethodPreflight::OwnedDeclaration { .. }
                ))
                .count(),
            1
        );
        assert!(observed
            .iter()
            .all(|(role, _)| ![c"NSPasteboard", c"NSPasteboardItem"].contains(role)));
    }
    #[test]
    fn foundation_only_arena_refuses_other_roles_before_effect_or_decoder() {
        let fixture = super::super::tests::Fixture::new();
        struct Calls(AtomicUsize);
        impl crate::sdk::owned::AliveLease for Calls {
            fn before_effect(&self) -> Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn is_alive(&self) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                true
            }
        }
        let calls = Arc::new(Calls(AtomicUsize::new(0)));
        let model = Workspace::new(
            crate::sdk::owned::ModuleContext {
                life: calls.clone(),
                ..fixture.model.context.clone()
            },
            vec![],
            fixture.model.state.lock().unwrap().screens.clone(),
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        let memory = super::preflight_foundation_memory().unwrap();
        let mode_before = calls.0.load(Ordering::SeqCst);
        let next_before = model.state.lock().unwrap().next;
        let validators = AtomicUsize::new(0);
        for (foundation, plan) in [(true, None), (false, Some(&memory))] {
            let error=unsafe {
                super::prepare_data_roster(model.clone(), foundation, plan, || {
                    validators.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }.unwrap_err();
            assert_eq!(error,if foundation {"Foundation factory memory plan/mode differs"}
                else {"full typed data arena absent"});
        }
        assert_eq!(validators.load(Ordering::SeqCst), 0);
        assert_eq!(calls.0.load(Ordering::SeqCst), mode_before);
        assert_eq!(model.state.lock().unwrap().next, next_before);
        unsafe { super::install_foundation_data(model.clone()) }.unwrap();
        assert_eq!(unsafe{super::prepare_data_roster(model.clone(),false,Some(&memory),|| {
            validators.fetch_add(1,Ordering::SeqCst);Ok(())
        })}.unwrap_err(),"mixed typed data factory outside its full workspace arena");
        assert_eq!(validators.load(Ordering::SeqCst),0);
        let other_calls = Arc::new(Calls(AtomicUsize::new(0)));
        let other_model = Workspace::new(
            crate::sdk::owned::ModuleContext {
                life: other_calls.clone(),
                ..fixture.model.context.clone()
            },
            vec![],
            fixture.model.state.lock().unwrap().screens.clone(),
            Point { x: 0.0, y: 0.0 },
        )
        .unwrap();
        let before = calls.0.load(Ordering::SeqCst);
        let other_before = other_calls.0.load(Ordering::SeqCst);
        for attempted in [model.clone(), other_model.clone()] {
            assert!(super::install(attempted.clone()).is_err());
            assert!(super::install_fixture(attempted.clone()).is_err());
            assert!(unsafe { super::install_apple_event_data(attempted.clone()) }.is_err());
            assert!(unsafe { super::install_foundation_data(attempted) }.is_err());
        }
        assert_eq!(
            calls.0.load(Ordering::SeqCst),
            before,
            "duplicate installs must not call the original guardian"
        );
        assert_eq!(
            other_calls.0.load(Ordering::SeqCst),
            other_before,
            "foreign-model installs must not call its guardian"
        );
        let prepared = AtomicUsize::new(0);
        let next_before = model.state.lock().unwrap().next;
        assert!(super::new_owned_menu().is_err());
        assert!(
            super::new_owned_window(fixture.model.state.lock().unwrap().screens[0].frame).is_err()
        );
        assert_eq!(
            model.state.lock().unwrap().next,
            next_before,
            "scope denial must precede model allocation"
        );
        assert!(super::object(super::Kind::Workspace).is_err());
        assert!(super::object(super::Kind::ApplicationShell).is_err());
        assert!(super::object(super::Kind::MainQueue).is_err());
        assert!(super::new_apple_event_descriptor(|_| {
            prepared.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
        assert!(unsafe {
            super::bind_apple_event_descriptor(std::ptr::dangling_mut(), |_| {
                prepared.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
        .is_err());
        assert!(super::clipboard_general().is_err());
        assert!(super::clipboard_unique().is_err());
        assert!(super::clipboard_named(&[]).is_err());
        assert!(super::clipboard_board(std::ptr::dangling_mut()).is_err());
        assert!(super::clipboard_item(std::ptr::dangling_mut()).is_err());
        assert!(super::clipboard_write_object(std::ptr::dangling_mut()).is_err());
        assert!(super::clipboard_release(std::ptr::dangling_mut()).is_err());
        assert!(super::bind_clipboard_item_initializer(std::ptr::dangling_mut()).is_err());
        assert!(super::foundation_url_reading_file_only_key().is_err());
        assert!(
            super::clipboard_class(std::ptr::dangling_mut(), super::ClipboardRole::Item).is_err()
        );
        assert!(super::data_role_allowed(c"NSPasteboard").is_err());
        assert!(super::data_role_allowed(c"NSPasteboardItem").is_err());
        for role in [
            c"NSString",
            c"NSData",
            c"NSURL",
            c"NSNumber",
            c"NSArray",
            c"NSDictionary",
        ] {
            super::data_role_allowed(role).unwrap();
        }
        assert_eq!(prepared.load(Ordering::SeqCst), 0);
        assert_eq!(calls.0.load(Ordering::SeqCst), before);
        assert!(super::INSTALLED
            .get()
            .unwrap()
            .objects
            .lock()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn compiled_typed_data_roster_matches_all_actual_source_method_abis_without_messages() {
        let observed = super::preflight_typed_data_factories().unwrap();
        assert_eq!(observed.len(), 56);
        let mut identities = std::collections::BTreeSet::new();
        let mut originals = 0;
        let mut additions = 0;
        for (class, method) in observed {
            let (selector, meta) = match method {
                super::TypedDataMethodPreflight::Existing(method) => {
                    originals += 1;
                    (method.selector, method.class_method)
                }
                super::TypedDataMethodPreflight::OwnedDeclaration {
                    selector,
                    declaration,
                } => {
                    additions += 1;
                    assert_eq!(class, c"NSString");
                    declaration.verify().unwrap();
                    (selector, false)
                }
            };
            assert!(identities.insert((class, selector, meta)));
        }
        assert_eq!((originals, additions), (55, 1));
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn prove_owned_allocator_can_reenter_existing_arena_receiver() {
        static ENTERED: AtomicUsize = AtomicUsize::new(0);
        static INNER: AtomicUsize = AtomicUsize::new(0);
        extern "C" {
            fn class_addMethod(
                class: *const std::ffi::c_void,
                selector: *const std::ffi::c_void,
                implementation: *const std::ffi::c_void,
                types: *const std::ffi::c_char,
            ) -> u8;
            fn class_createInstance(class: *const AnyClass, extra: usize) -> *mut AnyObject;
            fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
        }
        unsafe extern "C" fn allocate(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
            ENTERED.fetch_add(1, Ordering::SeqCst);
            let inner = object(Kind::ApplicationShell).unwrap_or(std::ptr::null_mut());
            INNER.store(inner as usize, Ordering::SeqCst);
            unsafe { class_createInstance(class, 0) }
        }
        let class = own_class();
        assert_eq!(class.superclass().unwrap(), NSObject::class());
        assert!(
            super::super::raw_method(class, Sel::register(c".cxx_construct"), false)
                .unwrap()
                .is_none()
        );
        let original = class
            .class_method(objc2::sel!(new))
            .unwrap()
            .implementation();
        let metaclass = unsafe { &*(class as *const AnyClass as *const AnyObject) }.class();
        assert!(
            unsafe {
                class_addMethod(
                    (metaclass as *const AnyClass).cast(),
                    sel_registerName(c"new".as_ptr()),
                    allocate as *const () as *const std::ffi::c_void,
                    c"@@:".as_ptr(),
                )
            } != 0
        );
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = finished.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if !observed.load(Ordering::Acquire) {
                eprintln!(
                    "owned constructor reentry blocked; entered={}",
                    ENTERED.load(Ordering::Acquire)
                );
                std::process::exit(77);
            }
        });
        let screen = object(Kind::Screen(0)).unwrap();
        finished.store(true, Ordering::Release);
        assert_ne!(INNER.load(Ordering::Acquire), 0);
        assert_eq!(ENTERED.load(Ordering::Acquire), 1);
        assert_eq!(object(Kind::Screen(0)).unwrap(), screen);
        unsafe {
            class
                .class_method(objc2::sel!(new))
                .unwrap()
                .set_implementation(original);
        }
    }
    fn prove_thread_owned_descriptor_binding_and_retirement() {
        // The family-specific fixture class also receives the genuine existing
        // allocation/init grant, so leaf initializers can exercise that route.
        let descriptor_class = own_apple_event_class();
        let descriptor_model = INSTALLED.get().unwrap().model.clone();
        let descriptor_guard = unsafe {
            super::super::classes::ClassGuard::select_sdk_family(
                &[descriptor_class],
                descriptor_model.lease(),
                owned_apple_event_dealloc,
            )
        }
        .unwrap();
        let descriptor_grant = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                descriptor_model,
                descriptor_class,
                descriptor_guard.lifecycle_bindings(),
            )
        }
        .unwrap();
        super::super::factory::publish(Arc::new(descriptor_grant)).unwrap();
        let mut prepared = None;
        let receiver = new_apple_event_descriptor(|handle| {
            handle.check()?;
            prepared = Some(handle);
            Ok(())
        })
        .unwrap();
        let handle = prepared.unwrap();
        let key = handle.key().unwrap();
        assert_eq!(
            apple_event_descriptor(receiver).unwrap().key().unwrap(),
            key
        );
        assert!(apple_event_descriptor(std::ptr::null_mut()).is_err());
        assert!(apple_event_descriptor(std::ptr::dangling_mut::<AnyObject>()).is_err());
        assert!(apple_event_descriptor(object(Kind::ApplicationShell).unwrap()).is_err());
        let foreign = handle.clone();
        let pointer = receiver as usize;
        std::thread::spawn(move || {
            assert!(foreign.check().is_err());
            assert!(apple_event_descriptor(pointer as *mut AnyObject).is_err());
            let mut worker_cap = None;
            let worker_receiver = new_apple_event_descriptor(|cap| {
                worker_cap = Some(cap);
                Ok(())
            })
            .unwrap();
            worker_cap.as_ref().unwrap().check().unwrap();
            apple_event_descriptor(worker_receiver)
                .unwrap()
                .check()
                .unwrap();
            retire_apple_event_descriptor(worker_receiver, |_| Ok(())).unwrap();
            unsafe { release_apple_event_descriptor(worker_receiver) }.unwrap();
            assert!(worker_cap.unwrap().check().is_err());
        })
        .join()
        .unwrap();
        let mut stale = handle.clone();
        stale.handle.generation += 1;
        assert!(stale.check().is_err());
        let mut failed = None;
        assert!(new_apple_event_descriptor(|cap| {
            failed = Some(cap);
            Err("owned prepare failure".into())
        })
        .is_err());
        assert!(failed.unwrap().check().is_err());
        let mut releases = 0;
        retire_apple_event_descriptor(receiver, |cleanup_key| {
            releases += 1;
            assert_eq!(cleanup_key, key);
            assert!(handle.check().is_err());
            assert!(apple_event_descriptor(receiver).is_err());
            Ok(())
        })
        .unwrap();
        assert_eq!(releases, 1);
        assert!(handle.check().is_err());
        assert!(retire_apple_event_descriptor(receiver, |_| {
            releases += 1;
            Ok(())
        })
        .is_err());
        assert_eq!(releases, 1);
        unsafe { release_apple_event_descriptor(receiver) }.unwrap();
        let mut panicked = None;
        let pointer = new_apple_event_descriptor(|cap| {
            panicked = Some(cap);
            Ok(())
        })
        .unwrap();
        assert!(retire_apple_event_descriptor(pointer, |_| panic!("owned cleanup panic")).is_err());
        assert!(panicked.unwrap().check().is_err());
        assert!(apple_event_descriptor(pointer).is_err());
        unsafe { release_apple_event_descriptor(pointer) }.unwrap();

        // A distinct pure NSObject class exercises actual grant allocation and
        // the selected initializer, without a test-only initialization bypass.
        static ORIGINAL_INITIALIZERS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn original_initialize(_: *mut AnyClass, _: Sel) {
            ORIGINAL_INITIALIZERS.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C" fn selected_init(
            receiver: *mut AnyObject,
            _: Sel,
            _: u32,
            _: *mut AnyObject,
        ) -> *mut AnyObject {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                check_apple_event_initializer(receiver)?;
                bind_apple_event_descriptor(receiver, |cap| {
                    assert!(apple_event_descriptor(receiver).is_err());
                    cap.key()?;
                    Ok(())
                })
            })) {
                Ok(Ok(receiver)) => receiver,
                Ok(Err(_)) => std::ptr::null_mut(),
                Err(payload) => {
                    std::mem::forget(payload);
                    std::ptr::null_mut()
                }
            }
        }
        let mut builder = objc2::runtime::ClassBuilder::new(
            c"RuycastF1OwnedAppleEventInitializerData",
            NSObject::class(),
        )
        .unwrap();
        unsafe {
            builder.add_class_method(
                objc2::sel!(initialize),
                original_initialize as unsafe extern "C" fn(*mut AnyClass, Sel),
            );
            builder.add_method(
                objc2::sel!(initWithDescriptorType:data:),
                selected_init
                    as unsafe extern "C" fn(
                        *mut AnyObject,
                        Sel,
                        u32,
                        *mut AnyObject,
                    ) -> *mut AnyObject,
            );
        }
        let class = builder.register();
        let model = INSTALLED.get().unwrap().model.clone();
        let guard = unsafe {
            super::super::classes::ClassGuard::select_sdk_family(
                &[class],
                model.lease(),
                owned_apple_event_dealloc,
            )
        }
        .unwrap();
        let grant = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(
                model,
                class,
                guard.lifecycle_bindings(),
            )
        }
        .unwrap();
        super::super::factory::publish(Arc::new(grant)).unwrap();
        let allocated = unsafe { super::super::factory::allocate(class) }.unwrap();
        assert!(apple_event_descriptor(allocated).is_err());
        let actual = class
            .instance_method(objc2::sel!(initWithDescriptorType:data:))
            .unwrap()
            .implementation();
        assert_eq!(actual as usize, selected_init as *const () as usize);
        let invoke: unsafe extern "C" fn(
            *mut AnyObject,
            Sel,
            u32,
            *mut AnyObject,
        ) -> *mut AnyObject = unsafe { std::mem::transmute(actual) };
        let initialized = unsafe {
            invoke(
                allocated,
                objc2::sel!(initWithDescriptorType:data:),
                0,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(initialized, allocated);
        apple_event_descriptor(initialized)
            .unwrap()
            .check()
            .unwrap();
        let mut replay_prepare = false;
        assert!(unsafe {
            bind_apple_event_descriptor(initialized, |_| {
                replay_prepare = true;
                Ok(())
            })
        }
        .is_err());
        assert!(!replay_prepare);
        assert_eq!(ORIGINAL_INITIALIZERS.load(Ordering::SeqCst), 0);
        retire_apple_event_descriptor(initialized, |_| Ok(())).unwrap();
        unsafe { release_apple_event_descriptor(initialized) }.unwrap();
    }
    #[test]
    fn actual_owned_imp_selection_data_effects_and_retirement() {
        objc2::rc::autoreleasepool(|_| {
            prove_actual_owned_imp_selection_data_effects_and_retirement()
        });
    }
    fn prove_actual_owned_imp_selection_data_effects_and_retirement() {
        static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn canary(_: *mut AnyObject, _: Sel) -> i32 {
            ORIGINAL.fetch_add(1, Ordering::SeqCst);
            -1
        }
        assert!(unsafe { shared(std::ptr::null_mut(), objc2::sel!(sharedWorkspace)) }.is_null());
        let f = super::super::tests::Fixture::new();
        install_fixture(f.model.clone()).unwrap();
        prove_owned_allocator_can_reenter_existing_arena_receiver();
        prove_thread_owned_descriptor_binding_and_retirement();
        apple_events::prove_fixture_descriptors(own_apple_event_class()).unwrap();
        let class = own_class();
        let method = class
            .instance_method(objc2::sel!(processIdentifier))
            .unwrap();
        let expected = pid as *const () as usize;
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(
                canary as *const () as usize,
            ));
        }
        assert_ne!(method.implementation() as usize, expected);
        unsafe {
            method.set_implementation(std::mem::transmute::<usize, objc2::runtime::Imp>(expected));
        }
        assert_eq!(method.implementation() as usize, expected);
        let h = f.model.by_pid(701234).unwrap().unwrap();
        let application = object(Kind::Application(h)).unwrap();
        let value: i32 = unsafe { objc2::msg_send![&*application, processIdentifier] };
        assert_eq!(value, 701234);
        assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
        let workspace = unsafe { shared(std::ptr::null_mut(), objc2::sel!(sharedWorkspace)) };
        assert!(!workspace.is_null());
        let apps: *mut AnyObject = unsafe { objc2::msg_send![&*workspace, runningApplications] };
        let apps = unsafe { &*apps.cast::<NSArray<AnyObject>>() };
        assert_eq!(apps.count(), 1);
        let activated: Bool = unsafe { objc2::msg_send![&*application,activateWithOptions:0usize] };
        assert!(activated.as_bool());
        let foreground: *mut AnyObject =
            unsafe { objc2::msg_send![&*workspace, frontmostApplication] };
        assert_eq!(foreground, application);
        let screen = object(Kind::Screen(0)).unwrap();
        let geometry: NSRect = unsafe { objc2::msg_send![&*screen, visibleFrame] };
        assert_eq!(geometry.size.width, 1280.0);
        assert_eq!(
            unsafe { mouse(std::ptr::null_mut(), objc2::sel!(mouseLocation)) },
            NSPoint::new(50.0, 60.0)
        );
        let window = object(Kind::Window(
            f.model
                .new_window(f.model.screens().unwrap()[0].frame)
                .unwrap(),
        ))
        .unwrap();
        unsafe {
            let _: () =
                objc2::msg_send![&*window,makeKeyAndOrderFront:std::ptr::null::<AnyObject>()];
        }
        let shown: Bool = unsafe { objc2::msg_send![&*window, isVisible] };
        assert!(shown.as_bool());
        unsafe {
            let _: () = objc2::msg_send![&*window,orderOut:std::ptr::null::<AnyObject>()];
        }
        let shown: Bool = unsafe { objc2::msg_send![&*window, isVisible] };
        assert!(!shown.as_bool());
        f.model
            .set_open_result("https://fixture.invalid/open".into(), Outcome::Success)
            .unwrap();
        let owned =
            NSURL::URLWithString(&NSString::from_str("https://fixture.invalid/open")).unwrap();
        let opened: Bool = unsafe { objc2::msg_send![&*workspace,openURL:&*owned] };
        assert!(opened.as_bool());
        let unowned =
            NSURL::URLWithString(&NSString::from_str("https://personal.invalid/")).unwrap();
        let opened: Bool = unsafe { objc2::msg_send![&*workspace,openURL:&*unowned] };
        assert!(!opened.as_bool());
        notification_bridge::prove_fixture_centers(f.model.clone(), workspace, window);
        application::prove_fixture_application(f.model.clone());
        let cleanup_after_retire = new_apple_event_descriptor(|_| Ok(())).unwrap();
        let old_cap = apple_event_descriptor(cleanup_after_retire).unwrap();
        f.life.1.store(1, Ordering::Release);
        let denied_queue: *mut AnyObject = unsafe { objc2::msg_send![class, mainQueue] };
        assert!(denied_queue.is_null());
        let value: i32 = unsafe { objc2::msg_send![&*application, processIdentifier] };
        assert_eq!(value, 0);
        f.life.1.store(0, Ordering::Release);
        let still_denied_queue: *mut AnyObject = unsafe { objc2::msg_send![class, mainQueue] };
        assert!(still_denied_queue.is_null());
        let value: i32 = unsafe { objc2::msg_send![&*application, processIdentifier] };
        assert_eq!(value, 0);
        f.life.0.store(false, Ordering::Release);
        let value: i32 = unsafe { objc2::msg_send![&*application, processIdentifier] };
        assert_eq!(value, 0);
        let activated: Bool = unsafe { objc2::msg_send![&*application,activateWithOptions:0usize] };
        assert!(!activated.as_bool());
        notification_bridge::prove_retired_boundary();
        f.model.retire().unwrap();
        assert!(old_cap.check().is_err());
        let mut cleaned = false;
        retire_apple_event_descriptor(cleanup_after_retire, |_| {
            cleaned = true;
            assert!(old_cap.check().is_err());
            Ok(())
        })
        .unwrap();
        assert!(cleaned);
        assert!(old_cap.check().is_err());
        unsafe { release_apple_event_descriptor(cleanup_after_retire) }.unwrap();
        assert_eq!(ORIGINAL.load(Ordering::SeqCst), 0);
        assert!(!scope_ready());
    }
}
