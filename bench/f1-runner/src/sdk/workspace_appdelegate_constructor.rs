//! Class-local AppDelegate constructor contribution. No general callback or
//! final ARC permission is supplied by its data/extra-reference components.
use super::super::*;
use crate::sdk::metadata;
use objc2::encode::{Encode, Encoding};
use sha2::Digest;
use objc2::runtime::{AnyClass,AnyObject,Bool,Sel};
#[path="workspace_appdelegate_code.rs"]
mod app_delegate_code;
// OUTSIDE CURRENT DATA-COMPILE DI. Class-local constructor DATA for raw3.
// This observes source/code/layout/root-init only. It cannot mint physical
// retain/release permission or publish an Initialized receiver by itself.
pub(crate) struct AppDelegateConstructionData {
    context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,
    model:Arc<Workspace>,
    class:metadata::ClassHandle,
    original_init:metadata::MethodSnapshot,
    destructor:metadata::MethodSnapshot,
    root_init:metadata::ObjectRootInitializer,
    singleton:super::AppDelegateSingletonData,
    system:SystemBindings,
    main_base:usize,
    retired:AtomicBool,
}
impl AppDelegateConstructionData {
    pub(crate) fn observe(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>)->Result<Self> {
        context.verify_model(&model)?;
        if context.constructor_source().file!="Sources/RuycastApp/main.swift"
            ||context.constructor_source().source_sha256!="9a2ed4af5088d3dfa31dc743475fdfe6677c223feaedb075cd4be2e61147b648"
            ||context.callback_source().file!="reference-0.8.26/Sources/RuycastApp/main.swift"
            ||context.callback_source().source_sha256!=context.constructor_source().source_sha256 {
            return Err("AppDelegate compiled source closure differs".into());
        }
        // class_getName returns the runtime's demangled top-level Swift name,
        // not the compiler symbol spelling. Exact address/image checks follow.
        let class=metadata::loaded_class(c"RuycastF1Reference.AppDelegate")?;
        let original_init=metadata::method(class,c"init",false)?.ok_or("AppDelegate actual initTo absent")?;
        let destructor=metadata::method(class,c".cxx_destruct",false)?.ok_or("AppDelegate actual CXX destructor absent")?;
        if original_init.owner()!=class.address()||super::super::super::simple_abi(&original_init)?!=b"@@:"
            ||destructor.owner()!=class.address()||super::super::super::simple_abi(&destructor)?!=b"v@:" {
            return Err("AppDelegate original entry ABI/owner differs".into());
        }
        // The sealed artifact hash includes this build's UUID. Qualify its
        // complete loaded compiler text/layout/UUID against that same artifact
        // instead of comparing against a UUID from an older compiler capture.
        context.verify_original_compiler_text()?;
        let main_base=context.observe_app_delegate_offset_cells()?.main_header();
        let init_entry=main_base.checked_add(0x1fd6f8).ok_or("AppDelegate init entry overflow")?;
        if original_init.implementation()!=init_entry
            ||*original_init.code()!=metadata::code_identity(init_entry)?
            ||destructor.code().uuid()!=original_init.code().uuid() {
            return Err("AppDelegate original entry exact admitted build differs".into());
        }
        if main_base.checked_add(0x3eb088)!=Some(class.address())
            ||main_base.checked_add(0x1fd784)!=Some(destructor.implementation()) {
            return Err("AppDelegate actual class/entry image offsets differ".into());
        }
        let root_init=metadata::ObjectRootInitializer::observe()?;
        let singleton=super::AppDelegateSingletonData::observe(context.clone())?;
        let system=SystemBindings::observe(&context)?;
        let value=Self{context,model,class,original_init,destructor,root_init,singleton,system,main_base,retired:AtomicBool::new(false)};
        value.verify_original()?;Ok(value)
    }
    pub(crate) fn verify_bodies_and_layout(&self)->Result<()> {
        self.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire) {return Err("AppDelegate constructor DATA retired".into());}
        let result=(|| {
            if metadata::registered_class(self.class.address())?!=self.class {return Err("AppDelegate runtime class changed".into());}
            self.context.verify_app_delegate_sdk_members(app_delegate_init_entry as *const () as usize,app_delegate_init_checked as *const () as usize)?;
            let offsets=self.context.observe_app_delegate_offset_cells()?;
            self.verify_offset_observation(&offsets)?;
            self.verify_sdk_entry_body()?;
            let class=unsafe{&*(self.class.address() as *const AnyClass)};
            if class.instance_size()!=64
                ||class.superclass().map(|v|v as *const AnyClass as usize)!=Some(self.root_init.method().owner()) {
                return Err("AppDelegate actual size/superclass differs".into());
            }
            let fields=class.instance_variables();
            if fields.len()!=7 {return Err("AppDelegate seven-field layout differs".into());}
            for (name,offset) in [(c"state",8),(c"server",16),(c"palette",24),(c"management",32),(c"monitor",40),(c"statusItem",48),(c"subscriptions",56)] {
                let matches=fields.iter().filter(|v|v.name()==name).collect::<Vec<_>>();
                if matches.len()!=1||matches[0].offset()!=offset {return Err("AppDelegate actual field identity differs".into());}
            }
            let pins = app_delegate_code::CODE.iter().map(|pin| {
                let address = self.main_base.checked_add(pin.offset)
                    .ok_or("AppDelegate code offset overflow")?;
                match pin.role {
                    app_delegate_code::Role::WholeObjcInit if address != self.original_init.implementation() =>
                        return Err("AppDelegate init code role differs".into()),
                    app_delegate_code::Role::CxxDestroySevenFields if address != self.destructor.implementation() =>
                        return Err("AppDelegate destructor code role differs".into()),
                    _ => (),
                }
                Ok((address, pin))
            }).collect::<Result<Vec<_>>>()?;
            // Only fixed RX checks share the source observation. Role,
            // system, metadata and layout errors keep their own boundaries.
            let verify_pins = || {
                for &(address, pin) in &pins {
                    self.context.verify_main_code(address, pin.bytes, pin.sha)?;
                }
                Ok(())
            };
            if self.context.callback_source_observation_active()? {
                verify_pins()?;
            } else {
                unsafe { self.context.with_callback_source_observation(verify_pins) }?;
            }
            self.system.verify(&self.context)?;
            self.root_init.verify()?;
            self.singleton.verify()?;
            metadata::verify_current_method(self.class,c".cxx_destruct",false,&self.destructor)?;
            let after=self.context.observe_app_delegate_offset_cells()?;
            self.verify_offset_observation(&after)?;
            if offsets!=after {return Err("AppDelegate full-width offset/selector cells changed".into());}
            self.context.verify_app_delegate_sdk_members(app_delegate_init_entry as *const () as usize,app_delegate_init_checked as *const () as usize)?;
            if self.retired.load(Ordering::Acquire) {return Err("AppDelegate constructor DATA revoked".into());}
            Ok(())
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    // Setup-only check. After class-local controlled init overlay, the issuer
    // instead composes bodies/layout with the exact SDK replacement binding;
    // it must not pretend the original IMP remains selected.
    pub(crate) fn verify_original(&self)->Result<()> {
        self.context.verify_model(&self.model)?;
        self.verify_bodies_and_layout()?;
        metadata::verify_current_method(self.class,c"init",false,&self.original_init)?;
        self.context.after_dynamic()?;
        self.verify_bodies_and_layout()
    }
}

// A narrowly qualified NONFINAL reference operation. It cannot release the
// constructor's input or a published root, and supplies no destructor permit.
struct NonfinalArc {
    data:Arc<AppDelegateConstructionData>,
    lifecycle:metadata::LifecycleSnapshot,
    retain:metadata::MethodSnapshot,
    release:metadata::MethodSnapshot,
    retain_entry:metadata::CodeIdentity,
    release_entry:metadata::CodeIdentity,
}
impl NonfinalArc {
    fn observe(data:Arc<AppDelegateConstructionData>)->Result<Self> {
        data.context.cleanup_metadata_only()?;
        let retain=metadata::method(data.class,c"retain",false)?.ok_or("source default retain absent")?;
        let release=metadata::method(data.class,c"release",false)?.ok_or("source default release absent")?;
        let root=data.root_init.method().owner();
        if retain.owner()!=root||release.owner()!=root
            ||super::super::super::simple_abi(&retain)?!=b"@@:"
            ||super::super::super::simple_abi(&release)?!=b"v@:"
            ||retain.code().image()!=b"/usr/lib/libobjc.A.dylib"
            ||release.code().image()!=b"/usr/lib/libobjc.A.dylib" {
            return Err("source class overrides the qualified NSObject reference primitives".into());
        }
        let retain_entry=metadata::code_identity(objc2::ffi::objc_retain as *const () as usize)?;
        let release_entry=metadata::code_identity(objc2::ffi::objc_release as *const () as usize)?;
        if retain_entry.image()!=b"/usr/lib/libobjc.A.dylib"||release_entry.image()!=b"/usr/lib/libobjc.A.dylib" {
            return Err("source nonfinal ARC entry image differs".into());
        }
        let lifecycle=metadata::lifecycle(data.class)?;
        let value=Self{data,lifecycle,retain,release,retain_entry,release_entry};value.verify()?;Ok(value)
    }
    fn verify(&self)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        self.data.verify_bodies_and_layout()?;
        if metadata::lifecycle(self.data.class)?!=self.lifecycle
            ||metadata::code_identity(objc2::ffi::objc_retain as *const () as usize)?!=self.retain_entry
            ||metadata::code_identity(objc2::ffi::objc_release as *const () as usize)?!=self.release_entry {
            return Err("source nonfinal ARC metadata changed".into());
        }
        metadata::verify_current_method(self.data.class,c"retain",false,&self.retain)?;
        metadata::verify_current_method(self.data.class,c"release",false,&self.release)
    }
}
#[derive(Clone,Copy,PartialEq,Eq)]
enum ConstructionReference {InputHeld,ForeignCall,OwnedReturn,Transferred}
#[derive(Clone,PartialEq,Eq)]
struct AppDelegateFields {optional:[usize;6],subscriptions:usize}
pub(crate) struct AppDelegateReceiverState {
    reference:ConstructionReference,
    fields:Option<AppDelegateFields>,
    extra:Option<ExtraReference>,
}
struct InitializerClaim {
    receiver:usize,serial:u64,generation:u64,owner:std::thread::ThreadId,
    declaration:Arc<AppDelegateConstruction>,
}
pub(crate) struct AppDelegateConstruction {
    data:Arc<AppDelegateConstructionData>,
    nonfinal:Arc<NonfinalArc>,
    // Exact class-local selected init binding will be installed by ClassGuard.
    // No public arbitrary IMP, class, process or source assertion creates this.
    installed:std::sync::OnceLock<super::super::super::classes::AppDelegateInitBinding>,
    verifying:AtomicBool,
    retired:AtomicBool,
}
struct ExtraReference {
    pointer:usize,serial:u64,generation:u64,owner:std::thread::ThreadId,
    grant:std::sync::Weak<WorkspaceGrant>,arc:Arc<NonfinalArc>,armed:bool,
}
impl ExtraReference {
    fn quarantine(&mut self){self.armed=false;}
    fn allocation_reference(&self)->Result<()> {
        self.arc.data.context.cleanup_metadata_only()?;
        if self.owner!=std::thread::current().id(){return Err("source extra reference foreign owner".into());}
        let grant=self.grant.upgrade().ok_or("source extra reference ledger absent")?;
        let rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
        let row=rows.get(&self.pointer).ok_or("source extra reference allocation absent")?;
        let state=row.app_delegate.as_ref().ok_or("source constructor ownership absent")?;
        if row.serial!=self.serial||row.generation!=self.generation||row.owner!=self.owner
            ||!matches!(state.reference,ConstructionReference::InputHeld|ConstructionReference::OwnedReturn) {
            return Err("source extra release has no independent construction reference".into());
        }
        drop(rows);
        if unsafe{&*(self.pointer as *mut AnyObject)}.class()!=self.arc.data.class(){return Err("source extra reference physical class changed".into());}
        Ok(())
    }
}
impl Drop for ExtraReference {
    fn drop(&mut self){
        // A published/foreign-call/unknown/retired-code outcome quarantines the
        // known +1. It must never become ordinary Retained::Drop. Business-only
        // retirement is distinct from this allocation + code/ARC obligation.
        if !self.armed||crate::sdk::process_fork::is_child(){return;}
        if self.allocation_reference().is_err()||self.arc.verify().is_err()
            ||self.allocation_reference().is_err(){return;}
        self.armed=false;
        // The separate input/owned-return +1 remains live. No final destructor
        // or unknown returned object can be released by this narrow operation.
        unsafe{objc2::ffi::objc_release(self.pointer as *mut AnyObject)};
    }
}
impl AppDelegateConstructionData {
    pub(crate) fn class(&self)->&'static AnyClass {unsafe{&*(self.class.address() as *const AnyClass)}}
    pub(crate) fn original_init(&self)->&metadata::MethodSnapshot {&self.original_init}
    pub(crate) fn context(&self)->&crate::sdk::core_preentry::ReferenceAppDelegateContext {&self.context}
}
impl AppDelegateConstruction {
    pub(crate) fn verify_published_epoch(&self) -> Result<()> {
        self.data.context.verify_published_code_epoch()?;
        if self.retired.load(Ordering::Acquire) || self.installed.get().is_none() {
            return Err("AppDelegate constructor publication retired or absent".into());
        }
        Ok(())
    }
    pub(crate) fn verify_construction(&self)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire)||self.verifying.swap(true,Ordering::AcqRel) {
            self.retired.store(true,Ordering::Release);
            return Err("typed AppDelegate constructor retired or reentered".into());
        }
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_>{fn drop(&mut self){self.0.store(false,Ordering::Release)}}
        let _reset=Reset(&self.verifying);
        let result=(|| {
            self.data.context.verify_model(&self.data.model)?;
            if self.retired.load(Ordering::Acquire){return Err("typed constructor revoked during model guard".into());}
            // The nonfinal ARC witness shares this exact construction data and
            // checks bodies/layout before its own metadata. No effect or
            // dynamic verifier needs a separate observation in between.
            self.nonfinal.verify()?;
            if self.retired.load(Ordering::Acquire){return Err("typed constructor revoked during ARC check".into());}
            self.installed.get().ok_or("typed init overlay not installed")?.verify()?;
            if self.retired.load(Ordering::Acquire){return Err("typed constructor revoked during guard".into());}
            Ok(())
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
}

struct ConstructorAuthority {data:Arc<AppDelegateConstructionData>}
// Only constructor/CXX code identity is issued here. All application callbacks,
// senders, weak/final disposal and generic partial cleanup remain refused.
unsafe impl super::super::super::delegate::CallbackCodeAuthority for ConstructorAuthority {
    fn verify_source_construction(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        if class as *const AnyClass as usize!=self.data.class.address()||source!=self.data.context.constructor_source(){return Err("typed constructor source differs".into());}
        self.data.context.verify_model(&self.data.model)?;self.data.verify_bodies_and_layout()
    }
    fn verify_method(&self,class:&AnyClass,selector:Sel,imp:usize,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        if class as *const AnyClass as usize!=self.data.class.address()||selector!=Sel::register(c".cxx_destruct")
            ||imp!=self.data.destructor.implementation()||source!=self.data.context.callback_source(){
            return Err("constructor-only authority does not qualify this callback".into());
        }
        self.data.verify_bodies_and_layout()
    }
    fn verify_sender(&self,_class:&AnyClass,_selector:Sel,_imp:usize,_source:&crate::sdk::SourceCallsite,_sender:&AnyClass)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        Err("constructor-only authority supplies no sender permission".into())
    }
}
struct InitializerLife {context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,inner:Arc<dyn crate::sdk::owned::AliveLease>}
impl crate::sdk::owned::AliveLease for InitializerLife {
    fn is_alive(&self)->bool {self.context.cleanup_metadata_only().is_ok()&&self.inner.is_alive()}
    fn before_effect(&self)->Result<()> {self.context.before_init()?;self.inner.before_effect()?;self.context.after_dynamic()}
}
/// Actual class/ledger contribution only. A caller must add separately genuine
/// callback/cleanup authority before this can be a complete Source actor.
pub struct PreparedAppDelegate {
    grant:Arc<WorkspaceGrant>,
    authority:Arc<dyn super::super::super::delegate::CallbackCodeAuthority>,
}
/// Actual class-selection transaction, retained across independent callback
/// qualification. It cannot be cloned or reconstructed from runtime snapshots.
pub struct PreparedAppDelegateClass {
    data:Arc<AppDelegateConstructionData>,
    guard:Option<super::super::super::classes::ClassGuard>,
    generation:u64,
    owner:std::thread::ThreadId,
    consumed:bool,
}
impl PreparedAppDelegateClass {
    fn checked<T>(&self,result:Result<T>)->Result<T> {
        result.inspect_err(|_|self.data.context.retire_code())
    }
}
impl Drop for PreparedAppDelegateClass {
    fn drop(&mut self) {
        // Revoke before dropping the actual selected guard and its authorities.
        if !self.consumed {self.data.context.retire_code();}
    }
}
/// # Safety
/// Exclusive Reference pre-entry, no original instances. Select only the
/// source-qualified SDK lifecycle before callback ARC baselines are captured.
/// This phase neither overlays init nor publishes a construction grant.
pub unsafe fn prepare_class(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>)->Result<PreparedAppDelegateClass> {
    context.verify_model(&model)?;
    let data=Arc::new(AppDelegateConstructionData::observe(context.clone(),model.clone())?);
    let generation=model.with(|state|Ok(state.generation))?;
    let mut prepared=PreparedAppDelegateClass{data,guard:None,generation,owner:std::thread::current().id(),consumed:false};
    let mut lease=model.lease();lease.life=Arc::new(InitializerLife{context,inner:lease.life.clone()});
    prepared.guard=Some(prepared.checked(unsafe{super::super::super::classes::ClassGuard::select(&[prepared.data.class()],lease)})?);
    prepared.checked(prepared.guard.as_ref().ok_or("AppDelegate selected class guard absent")?.verify())?;
    prepared.checked(prepared.data.verify_original())?;
    Ok(prepared)
}
impl PreparedAppDelegate {
    pub fn grant(&self)->Arc<WorkspaceGrant> {self.grant.clone()}
    pub fn construction_authority(&self)->Arc<dyn super::super::super::delegate::CallbackCodeAuthority> {self.authority.clone()}
}
/// # Safety
/// Exclusive actual Reference pre-entry; no live AppDelegate exists. This uses
/// the same sole factory map and per-class receiver ledger, not another arena.
pub unsafe fn prepare(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>,mut prepared:PreparedAppDelegateClass)->Result<PreparedAppDelegate> {
    prepared.checked(context.verify_model(&model))?;
    let generation=prepared.checked(model.with(|state|Ok(state.generation)))?;
    if !Arc::ptr_eq(&context,&prepared.data.context)||!Arc::ptr_eq(&model,&prepared.data.model)
        ||prepared.owner!=std::thread::current().id()||prepared.generation!=generation {
        context.retire_code();return Err("AppDelegate class preparation issuer/owner/generation differs".into());
    }
    let data=prepared.data.clone();
    let guard=prepared.checked(prepared.guard.as_ref().ok_or_else(||"AppDelegate selected class guard absent".to_owned()))?;
    prepared.checked(guard.verify())?;
    prepared.checked(data.verify_original())?;
    let nonfinal=Arc::new(prepared.checked(NonfinalArc::observe(data.clone()))?);
    let authority:Arc<dyn super::super::super::delegate::CallbackCodeAuthority>=Arc::new(ConstructorAuthority{data:data.clone()});
    let mut selected=guard.lifecycle_bindings();
    selected.push(prepared.checked(unsafe{LifecycleBinding::source(data.class().name(),c".cxx_destruct",false,context.callback_source().clone(),authority.clone())})?);
    let mut grant=prepared.checked(unsafe{WorkspaceGrant::for_original(model.clone(),data.class(),context.callback_source().clone(),selected,Vec::new(),authority.clone())})?;
    let declaration=Arc::new(AppDelegateConstruction{data:data.clone(),nonfinal,installed:std::sync::OnceLock::new(),verifying:AtomicBool::new(false),retired:AtomicBool::new(false)});
    let binding=prepared.checked(unsafe{guard.overlay_app_delegate_initializer(data.clone())})?;
    let initializer=LifecycleBinding{class:binding.class(),selector:c"init",class_method:false,original:binding.original(),replacement:binding.replacement(),proof:LifecycleProof::AppDelegateInit(binding.clone())};
    prepared.checked(declaration.installed.set(binding).map_err(|_|"duplicate AppDelegate init binding".to_owned()))?;
    grant.selected.push(initializer);
    prepared.checked(grant.construction.set(SourceConstructionAdmission{constructor_source:context.constructor_source().clone(),authority:authority.clone(),model:model.clone(),class:data.class(),grant:grant.id,generation,owner:std::thread::current().id(),retired:AtomicBool::new(false),cleanup_retired:AtomicBool::new(false),app_delegate:Some(declaration)})
        .map_err(|_|"duplicate typed AppDelegate construction admission".to_owned()))?;
    let grant=Arc::new(grant);prepared.checked(grant.verify())?;
    // The entry's PID gate is armed before a callable source grant is exposed.
    APP_DELEGATE_PID.store(unsafe{libc::getpid()},Ordering::Release);
    prepared.checked(super::super::super::factory::publish(grant.clone()))?;
    prepared.consumed=true;
    Ok(PreparedAppDelegate{grant,authority})
}

static APP_DELEGATE_PID:std::sync::atomic::AtomicI32=std::sync::atomic::AtomicI32::new(0);
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn app_delegate_init_entry(_pointer:*mut AnyObject,_selector:Sel)->*mut AnyObject {
    core::arch::naked_asm!("stp x29,x30,[sp,#-16]!","mov x29,sp","mov x2,x30","bl {checked}","ldp x29,x30,[sp],#16","ret",checked=sym app_delegate_init_checked);
}
unsafe extern "C" fn app_delegate_init_checked(pointer:*mut AnyObject,selector:Sel,caller:usize)->*mut AnyObject {
    if crate::sdk::process_fork::is_child()||APP_DELEGATE_PID.load(Ordering::Acquire)!=unsafe{libc::getpid()} {unsafe{libc::_exit(95)}}
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||unsafe{run_initializer(pointer,selector,caller)}));
    match result {Ok(Ok(receiver))=>receiver,failure=>{if let Err(payload)=failure{std::mem::forget(payload);}
        unsafe{libc::_exit(95)}}}
}
fn pure_claim(grant:&WorkspaceGrant,claim:&InitializerClaim,reference:ConstructionReference)->Result<()> {
    let proof=grant.construction.get().ok_or("source construction absent")?;
    if proof.retired.load(Ordering::Acquire)||proof.cleanup_retired.load(Ordering::Acquire)
        ||claim.declaration.retired.load(Ordering::Acquire)||claim.owner!=std::thread::current().id()
        ||!proof.app_delegate.as_ref().is_some_and(|p|Arc::ptr_eq(p,&claim.declaration)) {
        return Err("typed source claim retired or replaced".into());
    }
    grant.verify_construction_state(claim.generation)?;
    let rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
    let row=rows.get(&claim.receiver).ok_or("typed source receiver absent")?;
    if row.serial!=claim.serial||row.generation!=claim.generation||row.owner!=claim.owner
        ||row.phase!=ReceiverPhase::InitializingOriginal
        ||row.app_delegate.as_ref().is_none_or(|s|s.reference!=reference) {
        return Err("typed source allocation or ownership changed".into());
    }
    Ok(())
}
fn claim_initializer(grant:&Arc<WorkspaceGrant>,pointer:*mut AnyObject,selector:Sel,caller:usize)->Result<InitializerClaim> {
    let proof=grant.construction.get().ok_or("source construction absent")?;
    let declaration=proof.app_delegate.as_ref().ok_or("not the typed AppDelegate grant")?.clone();
    // Hardware LR captured by the fixed C entry; all caller/allocator code is
    // separately byte/source-bound. A PC/row string alone supplies no permit.
    if selector!=objc2::sel!(init)||declaration.data.main_base.checked_add(0x1f86a0)!=Some(caller)
        ||!grant.original_source||!grant.probes.is_empty()||grant.class!=declaration.data.class() {
        return Err("typed source actual constructor caller differs".into());
    }
    let mut rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
    let row=rows.get_mut(&(pointer as usize)).ok_or("untracked typed source receiver")?;
    if row.phase!=ReceiverPhase::Allocated||row.owner!=std::thread::current().id()||row.app_delegate.is_some() {
        proof.retired.store(true,Ordering::Release);declaration.retired.store(true,Ordering::Release);
        return Err("typed source initializer reentered or replayed".into());
    }
    row.phase=ReceiverPhase::InitializingOriginal;
    row.app_delegate=Some(AppDelegateReceiverState{reference:ConstructionReference::InputHeld,fields:None,extra:None});
    Ok(InitializerClaim{receiver:pointer as usize,serial:row.serial,generation:row.generation,owner:row.owner,declaration})
}
fn set_reference(grant:&WorkspaceGrant,claim:&InitializerClaim,from:ConstructionReference,to:ConstructionReference)->Result<()> {
    pure_claim(grant,claim,from)?;
    let mut rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
    let row=rows.get_mut(&claim.receiver).ok_or("typed source receiver disappeared")?;
    if row.serial!=claim.serial||row.phase!=ReceiverPhase::InitializingOriginal
        ||row.app_delegate.as_ref().is_none_or(|s|s.reference!=from){return Err("typed reference transition changed".into());}
    row.app_delegate.as_mut().ok_or("typed reference state absent")?.reference=to;Ok(())
}
unsafe fn read_initialized_fields(grant:&WorkspaceGrant,claim:&InitializerClaim)->Result<AppDelegateFields> {
    pure_claim(grant,claim,ConstructionReference::OwnedReturn)?;
    claim.declaration.data.verify_bodies_and_layout()?;
    pure_claim(grant,claim,ConstructionReference::OwnedReturn)?;
    let pointer=claim.receiver as *mut AnyObject;
    if unsafe{&*pointer}.class()!=grant.class{return Err("typed source physical class changed".into());}
    let read=||{let mut words=[0usize;7];for (word,offset) in words.iter_mut().zip([8usize,16,24,32,40,48,56]){*word=unsafe{pointer.cast::<u8>().add(offset).cast::<usize>().read()};}words};
    let before=read();if before[..6]!=[0usize;6]{return Err("typed source optional fields are not initialized nil".into());}
    claim.declaration.data.singleton.verify_initialized_word(before[6])?;
    pure_claim(grant,claim,ConstructionReference::OwnedReturn)?;
    if unsafe{&*pointer}.class()!=grant.class||read()!=before{return Err("typed source fields changed across dynamic verification".into());}
    Ok(AppDelegateFields{optional:[before[0],before[1],before[2],before[3],before[4],before[5]],subscriptions:before[6]})
}
unsafe fn run_initializer(pointer:*mut AnyObject,selector:Sel,caller:usize)->Result<*mut AnyObject> {
    let grant=super::super::super::factory::source_allocation_grant(pointer)?;
    let claim=claim_initializer(&grant,pointer,selector,caller)?;
    claim.declaration.verify_construction()?;pure_claim(&grant,&claim,ConstructionReference::InputHeld)?;
    claim.declaration.nonfinal.verify()?;pure_claim(&grant,&claim,ConstructionReference::InputHeld)?;
    if unsafe{&*pointer}.class()!=grant.class{return Err("typed source class changed before retain".into());}
    let retained=unsafe{objc2::ffi::objc_retain(pointer)};
    // Immediately own the actual extra effect before any fallible postcheck.
    let mut extra=ExtraReference{pointer:claim.receiver,serial:claim.serial,generation:claim.generation,owner:claim.owner,grant:Arc::downgrade(&grant),arc:claim.declaration.nonfinal.clone(),armed:true};
    if retained!=pointer {extra.quarantine();return Err("typed source retain returned another pointer".into());}
    pure_claim(&grant,&claim,ConstructionReference::InputHeld)?;
    claim.declaration.verify_construction()?;pure_claim(&grant,&claim,ConstructionReference::InputHeld)?;
    if unsafe{&*pointer}.class()!=grant.class{return Err("typed source class changed before original init".into());}
    claim.declaration.data.context.before_original_initialization()?;
    pure_claim(&grant,&claim,ConstructionReference::InputHeld)?;
    set_reference(&grant,&claim,ConstructionReference::InputHeld,ConstructionReference::ForeignCall)?;
    // Whole actual generated Objective-C initTo C entry, never a Swift-body
    // cast or original SDK leaf fallback. Finite actual NSObject.init contract
    // and source-bound init family retain input/result ownership semantics.
    let original:unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject=unsafe{std::mem::transmute(claim.declaration.data.original_init.implementation())};
    let returned=unsafe{original(pointer,selector)};
    if returned!=pointer {extra.quarantine();return Err("typed original init returned nil or another receiver".into());}
    claim.declaration.data.verify_bodies_and_layout()?;
    set_reference(&grant,&claim,ConstructionReference::ForeignCall,ConstructionReference::OwnedReturn)?;
    let observed=unsafe{read_initialized_fields(&grant,&claim)}?;
    claim.declaration.verify_construction()?;
    // Final read AFTER the last dynamic callback and before pure commit.
    let final_fields=unsafe{read_initialized_fields(&grant,&claim)}?;
    if final_fields!=observed{return Err("typed source initialization changed before commit".into());}
    pure_claim(&grant,&claim,ConstructionReference::OwnedReturn)?;
    {
        let state=grant.model.state.lock().map_err(|_|"source model poisoned")?;
        if state.retired||state.generation!=claim.generation||grant.model.faulted.load(Ordering::Acquire){return Err("typed source model retired before commit".into());}
        let proof=grant.construction.get().ok_or("typed source admission disappeared")?;
        if proof.retired.load(Ordering::Acquire)||claim.declaration.retired.load(Ordering::Acquire){return Err("typed source admission retired before commit".into());}
        let mut rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
        let row=rows.get_mut(&claim.receiver).ok_or("typed source allocation disappeared")?;
        if row.serial!=claim.serial||row.generation!=claim.generation||row.owner!=claim.owner||row.phase!=ReceiverPhase::InitializingOriginal
            ||row.app_delegate.as_ref().is_none_or(|s|s.reference!=ConstructionReference::OwnedReturn){return Err("typed source commit stale or replayed".into());}
        let app=row.app_delegate.as_mut().ok_or("typed source commit state absent")?;
        app.fields=Some(observed);app.reference=ConstructionReference::Transferred;
        // Transfer the extra +1 itself, no second retain and no cycle to grant.
        app.extra=Some(extra);row.phase=ReceiverPhase::Initialized;
    }
    Ok(pointer)
}

/// Actual extra physical +1 from the completed source initializer. Opaque
/// lifetime custody is shareable; original object access stays creator-only.
/// Drop quarantines until a separately qualified final/main-scope release
/// contract exists. This never claims full original-source ARC or child ABI.
pub struct InitializedAppDelegateRoot {
    state_capture_claimed:AtomicBool,
    state_capture_retired:AtomicBool,
    extra:ExtraReference,
    identity:ReceiverIdentity,
    declaration:Arc<AppDelegateConstruction>,
}
impl InitializedAppDelegateRoot {
    fn monitor_field_slot(owner: usize, class: metadata::ClassHandle, name: &CStr) -> Result<*const usize> {
        Ok(Self::monitor_ivar_address(owner, class, name, 8)? as *const usize)
    }

    fn monitor_ivar_address(owner: usize, class: metadata::ClassHandle, name: &CStr, width: usize) -> Result<usize> {
        if owner == 0
            || unsafe { objc2::ffi::object_getClass(owner as *const _) } as usize != class.address()
        {
            return Err("Reference monitor ivar owner class differs".into());
        }
        let class = unsafe { &*(class.address() as *const AnyClass) };
        let fields = class.instance_variables();
        let mut matching = fields.iter().filter(|field| field.name() == name);
        let offset = matching.next().ok_or("Reference monitor ivar absent")?.offset();
        let offset = usize::try_from(offset).map_err(|_| "Reference monitor ivar offset differs")?;
        if matching.next().is_some() || offset < 8 || !offset.is_multiple_of(width)
            || offset.checked_add(width).is_none_or(|end| end > class.instance_size())
        {
            return Err("Reference monitor ivar duplicated or outside its owner".into());
        }
        owner.checked_add(offset).ok_or_else(|| "Reference monitor field address overflow".into())
    }

    fn monitor_model(&self, root: usize) -> Result<Option<(usize, metadata::ClassHandle)>> {
        let controller = unsafe { ((root + 40) as *const usize).read_volatile() };
        if controller == 0 {
            return Ok(None);
        }
        let controller_class = metadata::loaded_class(c"RuycastF1Reference.SystemMonitorController")?;
        let model_class = metadata::loaded_class(c"RuycastF1Reference.SystemMonitorModel")?;
        let base = self.declaration.data.main_base;
        if base.checked_add(0x3eae10) != Some(controller_class.address())
            || base.checked_add(0x3f4ef8) != Some(model_class.address())
        {
            return Err("Reference monitor controller or model class differs".into());
        }
        let model = unsafe { Self::monitor_field_slot(controller, controller_class, c"model")?.read_volatile() };
        Ok(Some((model, model_class)))
    }

    pub(crate) fn verify_monitor_worker_queue(
        &self, window: bool, queue: usize, captured: usize,
    ) -> Result<()> {
        let data = &self.declaration.data;
        let root = self.pointer_on_creator()? as usize;
        let strong_field = |owner, class, name| -> Result<usize> {
            Ok(unsafe { Self::monitor_field_slot(owner, class, name)?.read_volatile() })
        };
        let expected = if window {
            let (model, model_class) = self.monitor_model(root)?.ok_or("Reference monitor controller absent")?;
            if strong_field(model, model_class, c"sampler")? != captured {
                return Err("Reference window monitor sampler capture differs".into());
            }
            strong_field(model, model_class, c"worker")?
        } else {
            let state = unsafe { ((root + 8) as *const usize).read_volatile() };
            let class = metadata::loaded_class(c"RuycastF1Reference.AppState")?;
            if state != captured || data.main_base.checked_add(0x3ec968) != Some(class.address()) {
                return Err("Reference CLI monitor AppState capture differs".into());
            }
            strong_field(state, class, c"monitorWorker")?
        };
        if expected != queue {
            return Err("Reference monitor Dispatch queue differs from original owner".into());
        }
        self.verify_held_epoch()
    }

    pub(crate) fn stop_monitor(&self) -> Result<()> {
        let data = &self.declaration.data;
        let root = self.pointer_on_creator()? as usize;
        let Some((model, model_class)) = self.monitor_model(root)? else { return Ok(()) };
        let timer_slot = Self::monitor_field_slot(model, model_class, c"timer")?;
        let offsets = data.context.verify_controller_compiler_metadata()?;
        if model.checked_add(offsets[5]) != Some(timer_slot as usize) {
            return Err("Reference monitor timer offset differs from original stop".into());
        }
        let at = |offset| data.main_base.checked_add(offset).ok_or("Reference monitor stop address overflow");
        for (offset, len, sha) in [
            (0x1ba1dc, 120, "daaf42b34cfde84bf9fb3d2846858ac3a662b119cfc1cb464f1e3111f9397b0c"),
            (0x7ba20, 8, "7ee3a4b6a73bacb9083309b1f3f2e966535256c0bd6dc365bda57205280b6b85"),
            (0x196e4, 12, "e9c467136f6157fff774fa73f71602bed11820a30dda216d62ec9dc63c61cd20"),
            (0x2fbf9c, 12, "7928d7185a73cd076ff53c082e37ecf0a2ac92370f8978978b188dac6bd8324d"),
            (0x2fbb04, 12, "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f"),
            (0x2fbb28, 12, "5e49d9900665dfed9d32a8a353b4ab725f78a1018bd53d877b62ed2f8b08463a"),
        ] {
            data.context.verify_main_code(at(offset)?, len, sha)?;
        }
        let bindings = crate::sdk::imports::observe_unpatched_main_imports(&[
            (c"swift_beginAccess", STATE_BEGIN_ACCESS_LABEL as *const () as usize),
            (c"objc_msgSend", MESSAGE_SEND_LABEL as *const () as usize),
            (c"objc_release", objc2::ffi::objc_release as *const () as usize),
        ])?;
        for (symbol, cell, image) in [
            (c"swift_beginAccess", 0x3c72d0, b"/usr/lib/swift/libswiftCore.dylib".as_slice()),
            (c"objc_msgSend", 0x3c6940, b"/usr/lib/libobjc.A.dylib".as_slice()),
            (c"objc_release", 0x3c6958, b"/usr/lib/libobjc.A.dylib".as_slice()),
        ] {
            let mut rows = bindings.iter().filter(|row| row.symbol == symbol);
            let row = rows.next().ok_or("Reference monitor stop import absent")?;
            if rows.next().is_some() || row.cell_address() != at(cell)?
                || metadata::code_identity(row.original_address())?.image() != image
            {
                return Err("Reference monitor stop import differs".into());
            }
        }
        let timer = unsafe { timer_slot.read_volatile() };
        if timer != 0 {
            let class = metadata::loaded_class(c"__NSCFTimer")?;
            let method = metadata::method(class, c"invalidate", false)?
                .ok_or("Reference monitor timer invalidate absent")?;
            if unsafe { objc2::ffi::object_getClass(timer as *const _) } as usize != class.address()
                || method.owner() != class.address()
                || super::super::super::simple_abi(&method)? != b"v@:"
                || method.code().image() != b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
            {
                return Err("Reference monitor timer invalidate receiver or ABI differs".into());
            }
            metadata::verify_current_method(class, c"invalidate", false, &method)?;
        }
        let generation_slot = Self::monitor_field_slot(model, model_class, c"generation")?.cast::<i64>();
        let active_slot = Self::monitor_ivar_address(model, model_class, c"active", 1)? as *const u8;
        let generation = unsafe { generation_slot.read_volatile() };
        let next = generation.checked_add(1).ok_or("Reference monitor generation exhausted")?;
        self.verify_held_epoch()?;
        let [active_offset, generation_offset] = data.context.monitor_stop_offsets()?;
        if model.checked_add(active_offset) != Some(active_slot as usize)
            || model.checked_add(generation_offset) != Some(generation_slot as usize)
        {
            return Err("Reference monitor stop field offsets differ".into());
        }
        data.context.verify_sdk_code(swift_self_adapter as *const () as usize, 100,
            "30291925bf7831296003aa608d880c227737963d1023bcbf739c61de5f69da71")?;
        if self.monitor_model(root)? != Some((model, model_class)) || unsafe { timer_slot.read_volatile() } != timer {
            return Err("Reference monitor owner or timer changed before stop".into());
        }
        for row in &bindings { row.verify_unpatched()?; }
        unsafe { swift_self_adapter(model, at(0x1ba1dc)?); }
        if self.monitor_model(root)? != Some((model, model_class))
            || unsafe { timer_slot.read_volatile() } != 0
            || unsafe { generation_slot.read_volatile() } != next
            || unsafe { active_slot.read_volatile() } != 0
        {
            return Err("Reference monitor timer remained attached after stop".into());
        }
        self.verify_held_epoch()
    }

    fn control_server_slot(&self) -> Result<*const usize> {
        let data = &self.declaration.data;
        let pointer = self.pointer_on_creator()? as usize;
        let offsets = data.context.observe_app_delegate_offset_cells()?;
        if offsets.main_header() != data.main_base || offsets.cells()[1].1 != 16 {
            return Err("Reference AppDelegate server offset differs".into());
        }
        self.verify_held_epoch()?;
        Ok(pointer.checked_add(16).ok_or("Reference server field overflow")? as *const usize)
    }

    pub(crate) fn verify_started_control(&self, admission: &crate::sdk::core_preentry::ReferenceAdmission) -> Result<()> {
        let data = &self.declaration.data;
        let slot = self.control_server_slot()?;
        let server = unsafe { slot.read_volatile() };
        let (lock, listener) = unsafe { data.context.observe_control_server_descriptors(server)? };
        admission.verify_started_control(lock, listener)?;
        self.verify_held_epoch()?;
        if unsafe { slot.read_volatile() } != server
            || unsafe { data.context.observe_control_server_descriptors(server)? } != (lock, listener)
        {
            return Err("Reference ControlServer changed during descriptor validation".into());
        }
        Ok(())
    }

    pub(crate) fn verify_stopped_control(&self) -> Result<()> {
        let slot = self.control_server_slot()?;
        if unsafe { slot.read_volatile() } != 0 {
            return Err("Reference ControlServer still attached at termination".into());
        }
        self.verify_held_epoch()
    }

    pub fn pointer_on_creator(&self)->Result<*mut AnyObject> {
        self.declaration.data.context.cleanup_metadata_only()?;
        self.declaration.verify_construction()?;
        self.identity.verify()?;self.verify_held_epoch()?;
        Ok(self.extra.pointer as *mut AnyObject)
    }
    /// Only pure existing-ledger/model checks, no object/code callback or
    /// foreign-thread ReceiverIdentity widening. The caller's real Input
    /// admission separately verifies its own fresh Source/code/retirement,
    /// body/userdata and registration BEFORE EVERY callback. Constructor/code
    /// revocation does not destroy this already held physical +1; this method
    /// is intentionally NOT a Root Source-liveness or dispatch permission.
    pub fn verify_held_epoch(&self)->Result<()> {
        if crate::sdk::process_fork::is_child(){unsafe{libc::_exit(95)}}
        if APP_DELEGATE_PID.load(Ordering::Acquire)!=unsafe{libc::getpid()}{return Err("source root foreign process".into());}
        let grant=&self.identity.grant;
        {
            let rows=grant.receivers.lock().map_err(|_|"source root ledger poisoned")?;
            let row=rows.get(&self.extra.pointer).ok_or("source root allocation absent")?;
            if row.serial!=self.extra.serial||row.generation!=self.extra.generation||row.owner!=self.extra.owner
                ||row.phase!=ReceiverPhase::Initialized
                ||row.app_delegate.as_ref().is_none_or(|s|s.reference!=ConstructionReference::Transferred||s.fields.is_none()||s.extra.is_some()) {
                return Err("source root constructor epoch changed".into());
            }
        }
        let state=grant.model.state.lock().map_err(|_|"source root model poisoned")?;
        if state.retired||state.generation!=self.extra.generation||grant.model.faulted.load(Ordering::Acquire){return Err("source root model retired".into());}
        Ok(())
    }
    /// Bind custody to the very same sealed reader admission. This verifies no
    /// Swift state/child layout and supplies no callback or Input permission.
    pub fn verify_reference_binding(&self,
        admission:&Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        model:&Arc<Workspace>,
    )->Result<()> {
        self.declaration.data.context.verify_admission_binding(admission,model)?;
        if !self.belongs_to(model){return Err("source root belongs to another workspace".into());}
        self.pointer_on_creator()?;
        self.declaration.data.context.verify_admission_binding(admission,model)?;
        self.verify_held_epoch()
    }
    pub fn belongs_to(&self,model:&Arc<Workspace>)->bool {self.identity.belongs_to(model)}
}
pub(crate) fn take_root(grant:Arc<WorkspaceGrant>,model:&Arc<Workspace>)->Result<Arc<InitializedAppDelegateRoot>> {
    let proof=grant.construction.get().ok_or("source construction absent")?;
    let declaration=proof.app_delegate.as_ref().ok_or("not an AppDelegate source grant")?.clone();
    declaration.data.context.verify_model(model)?;grant.verify()?;
    let (extra,identity)={
        let mut rows=grant.receivers.lock().map_err(|_|"source receiver ledger poisoned")?;
        let candidates=rows.iter().filter(|(_,r)|r.phase==ReceiverPhase::Initialized
            &&r.owner==std::thread::current().id()&&r.app_delegate.as_ref().is_some_and(|s|s.reference==ConstructionReference::Transferred&&s.fields.is_some()&&s.extra.is_some()))
            .map(|(p,_)|*p).collect::<Vec<_>>();
        if candidates.len()!=1{return Err("typed source root is absent or ambiguous".into());}
        let pointer=candidates[0];let row=rows.get_mut(&pointer).ok_or("typed source root disappeared")?;
        let extra=row.app_delegate.as_mut().ok_or("typed source root state absent")?.extra.take().ok_or("typed source root custody already transferred")?;
        let identity=ReceiverIdentity{grant:grant.clone(),receiver:pointer,serial:row.serial,generation:row.generation,owner:row.owner};
        (extra,identity)
    };
    let root=Arc::new(InitializedAppDelegateRoot{state_capture_claimed:AtomicBool::new(false),state_capture_retired:AtomicBool::new(false),extra,identity,declaration});root.pointer_on_creator()?;Ok(root)
}

// Address-only Swift labels. No Rust C declaration below is ever invoked or
// used as a Swift calling-convention cast; the genuine compiled initTo performs
// its original Swift/runtime calls with its own source-bound conventions.
#[link(name="swift_Concurrency")]
unsafe extern "C" {
    #[link_name="$sScMMa"] fn ACTOR_METADATA_LABEL();
    #[link_name="$sScM6sharedScMvgZ"] fn ACTOR_SHARED_LABEL();
    #[link_name="$sScA15unownedExecutorScevgTj"] fn ACTOR_EXECUTOR_LABEL();
    #[link_name="swift_task_isCurrentExecutor"] fn CURRENT_EXECUTOR_LABEL();
    #[link_name="swift_task_getMainExecutor"] fn MAIN_EXECUTOR_LABEL();
    #[link_name="swift_task_isCurrentExecutorWithFlags"] fn EXECUTOR_CHECK_LABEL();
    #[link_name="swift_task_reportUnexpectedExecutor"] fn REPORT_EXECUTOR_LABEL();
    #[link_name="$sScMScAsMc"] static ACTOR_CONFORMANCE_LABEL:u8;

}
#[link(name="swiftCore")]
unsafe extern "C" {
    #[link_name="swift_getObjectType"] fn GET_OBJECT_TYPE_LABEL();
    #[link_name="swift_getWitnessTable"] fn GET_WITNESS_LABEL();
    #[link_name="swift_release"] fn RELEASE_ACTOR_LABEL();
    #[link_name="swift_isUniquelyReferenced_nonNull_native"] fn UNIQUE_ACTOR_LABEL();
}
#[link(name="objc")]
unsafe extern "C" {
    #[link_name="objc_opt_self"] fn OBJECT_CLASS_SELF_LABEL();
    #[link_name="objc_msgSend"] fn MESSAGE_SEND_LABEL();
}
struct SystemBindings {
    super_binding: (crate::sdk::ImportReplacement, metadata::CodeIdentity),
    code:Vec<(crate::sdk::imports::UnpatchedImportObservation,metadata::CodeIdentity)>,
    conformance:crate::sdk::imports::UnpatchedImportObservation,
    conformance_header:usize,
}
impl SystemBindings {
    // Exact runtime transitions in the complete original Notification CTo.
    // These are the same typed actor/runtime imports used by construction;
    // this grants no arbitrary Swift-body call or source receiver cleanup.
    fn verify_notification_runtime(&self,context:&crate::sdk::core_preentry::ReferenceAppDelegateContext)->Result<()> {
        self.verify(context)?;
        let base=context.observe_app_delegate_offset_cells()?.main_header();
        for (name,cell) in [(c"$sScA15unownedExecutorScevgTj",0x3c77f8usize),
            (c"swift_task_isCurrentExecutor",0x3c7850),
            (c"swift_task_reportUnexpectedExecutor",0x3c7858),
            (c"swift_release",0x3c7438)] {
            let (row,_)=self.code.iter().find(|(row,_)|row.symbol==name)
                .ok_or("Notification typed actor/runtime import absent")?;
            if base.checked_add(cell)!=Some(row.cell_address()) {
                return Err("Notification typed actor/runtime cell differs".into());
            }
            row.verify_unpatched()?;
        }
        for (offset,bytes,sha) in [
            (0x2fa778usize,12,"55502911b7765954e57f58b4846c525ac858bfd624fee948f621f82442973e65"),
            (0x2fc260,24,"d0b911f1e818211af478f790bab920925e23f4ac23ea2a470626bdac9e93b309"),
            (0x2fc1ac,12,"c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
        ] {
            context.verify_main_code(base.checked_add(offset)
                .ok_or("Notification runtime stub address overflow")?,bytes,sha)?;
        }
        self.verify(context)
    }
    fn observe(context:&crate::sdk::core_preentry::ReferenceAppDelegateContext)->Result<Self> {
        context.cleanup_metadata_only()?;
        let concurrency=b"/usr/lib/swift/libswift_Concurrency.dylib".as_slice();
        let core=b"/usr/lib/swift/libswiftCore.dylib".as_slice();
        let objc=b"/usr/lib/libobjc.A.dylib".as_slice();
        let addresses=[(c"swift_getObjectType",GET_OBJECT_TYPE_LABEL as *const () as usize,core),
            (c"swift_getWitnessTable",GET_WITNESS_LABEL as *const () as usize,core),
            (c"swift_release",RELEASE_ACTOR_LABEL as *const () as usize,core),
            (c"$sScMMa",ACTOR_METADATA_LABEL as *const () as usize,concurrency),
            (c"$sScM6sharedScMvgZ",ACTOR_SHARED_LABEL as *const () as usize,concurrency),
            (c"$sScA15unownedExecutorScevgTj",ACTOR_EXECUTOR_LABEL as *const () as usize,concurrency),
            (c"swift_task_isCurrentExecutor",CURRENT_EXECUTOR_LABEL as *const () as usize,concurrency),
            (c"swift_task_reportUnexpectedExecutor",REPORT_EXECUTOR_LABEL as *const () as usize,concurrency),
            (c"objc_msgSend",MESSAGE_SEND_LABEL as *const () as usize,objc),
            (c"objc_opt_self",OBJECT_CLASS_SELF_LABEL as *const () as usize,objc)];
        let requests=addresses.iter().map(|(name,address,_)|(*name,*address)).collect::<Vec<_>>();
        let rows=crate::sdk::imports::observe_unpatched_main_imports(&requests)?;
        let mut code=Vec::new();
        for (name,address,image) in addresses {
            let mut matches=rows.iter().filter(|row|row.symbol==name);
            let row=matches.next().ok_or("constructor system import absent")?.clone();
            if matches.next().is_some()||row.original_address()!=address{return Err("constructor system import duplicate or foreign".into());}
            let identity=metadata::code_identity(address)?;
            if identity.image()!=image{return Err("constructor system runtime image differs".into());}
            code.push((row,identity));
        }
        let spec = super::super::super::factory::reference_edit_super_spec();
        let mut super_rows = crate::sdk::observe_main_imports(&[spec])?;
        if super_rows.len() != 1 {
            return Err("constructor super-init import count differs".into());
        }
        let row = super_rows.remove(0);
        let super_code = metadata::code_identity(spec.original)?;
        if super_code.image() != objc || context.observe_app_delegate_offset_cells()?.main_header()
            .checked_add(0x3c6948) != Some(row.cell_address())
        {
            return Err("constructor super-init import provider or cell differs".into());
        }
        let super_binding = (row, super_code);
        let address=std::ptr::addr_of!(ACTOR_CONFORMANCE_LABEL) as usize;
        let rows=crate::sdk::imports::observe_unpatched_main_imports(&[(c"$sScMScAsMc",address)])?;
        let conformance=rows.into_iter().next().ok_or("constructor conformance import absent")?;
        let actor=code.iter().find(|(row,_)|row.symbol==c"$sScMMa").ok_or("constructor actor metadata code absent")?;
        let conformance_header=actor.0.original_address().checked_sub(actor.1.offset()).ok_or("constructor actor image base underflow")?;
        let value=Self{super_binding,code,conformance,conformance_header};value.verify(context)?;Ok(value)
    }
    fn verify(&self,context:&crate::sdk::core_preentry::ReferenceAppDelegateContext)->Result<()> {
        context.cleanup_metadata_only()?;
        let (super_row, super_code) = &self.super_binding;
        let verify_super = || {
            super::super::super::factory::verify_reference_edit_super_selection(super_row)?;
            if metadata::code_identity(super_row.original)? != *super_code {
                return Err("constructor super-init original changed".into());
            }
            Ok::<_, String>(())
        };
        verify_super()?;
        for (row,expected) in &self.code {
            row.verify_unpatched()?;
            if metadata::code_identity(row.original_address())?!=*expected{return Err("constructor system code changed".into());}
        }
        self.conformance.verify_unpatched()?;
        let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
        if unsafe{libc::dladdr(self.conformance.original_address() as *const libc::c_void,info.as_mut_ptr())}==0{return Err("constructor conformance image absent".into());}
        let info=unsafe{info.assume_init()};
        if info.dli_fbase as usize!=self.conformance_header||info.dli_fname.is_null()||info.dli_sname.is_null()
            ||unsafe{CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/usr/lib/swift/libswift_Concurrency.dylib"
            ||unsafe{CStr::from_ptr(info.dli_sname)}!=c"$sScMScAsMc"
            ||info.dli_saddr as usize!=self.conformance.original_address(){return Err("constructor named conformance data binding differs".into());}
        // No descriptor/body load or code permission is inferred for DATA.
        for (row,expected) in &self.code {row.verify_unpatched()?;if metadata::code_identity(row.original_address())?!=*expected{return Err("constructor system binding changed across data check".into());}}
        verify_super()?;
        self.conformance.verify_unpatched()
    }
}
impl AppDelegateConstructionData {
    fn verify_offset_observation(&self,observation:&crate::sdk::core_preentry::AppDelegateOffsetCellsObservation)->Result<()> {
        if observation.main_header()!=self.main_base{return Err("AppDelegate actual Main data/code header differs".into());}
        for ((address,value),(offset,expected)) in observation.cells().iter().zip([(0x3f60d0,8u64),(0x3f60d8,16),(0x3f60e0,24),(0x3f60e8,32),(0x3f60f0,40),(0x3f60f8,48),(0x3f6100,56)]) {
            if self.main_base.checked_add(offset)!=Some(*address)||*value!=expected{return Err("AppDelegate full64 field offset differs".into());}
        }
        let (selector_cell,selector)=observation.init_selector_cell();
        if self.main_base.checked_add(0x3e8480)!=Some(selector_cell)||selector!=unsafe{std::mem::transmute::<Sel,usize>(objc2::sel!(init))} {
            return Err("AppDelegate actual original super selector differs".into());
        }
        Ok(())
    }
    fn verify_sdk_entry_body(&self)->Result<()> {
        let entry=app_delegate_init_entry as *const () as usize;
        let checked=app_delegate_init_checked as *const () as usize;
        let pc=entry.checked_add(12).ok_or("AppDelegate SDK branch PC overflow")?;
        let delta=checked as i128-pc as i128;
        if delta%4!=0||!(-(1i128<<27)..(1i128<<27)).contains(&delta){return Err("AppDelegate SDK checked helper branch range differs".into());}
        let branch=0x94000000|(((delta>>2) as u32)&0x03ffffff);
        let words=[0xa9bf7bfdu32,0x910003fd,0xaa1e03e2,branch,0xa8c17bfd,0xd65f03c0];
        let bytes=words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        let digest=format!("{:x}",sha2::Sha256::digest(&bytes));
        // Exact fixed C entry preserves original LR and passes it as x2; this
        // is a code/ABI join to the declared helper, not hash-as-Initialized.
        self.context.verify_sdk_code(entry,bytes.len(),&digest)
    }
}

// The original, sealed Swift compiler owns AppState's internal value layout,
// partial-initializer cleanup and generated destruction. Rust does not invoke
// a Swift destructor/VWT or manufacture an NSObject allocation grant for it.
// This fixed recipe is usable before allocation: a lazy metadata cache may
// legitimately be empty then. Actual held instances additionally require the
// completed metadata observation in AppStateCustody::verify_class.
struct AppStateCompilerCleanup {
    imports:Vec<(crate::sdk::imports::UnpatchedImportObservation,metadata::CodeIdentity,usize)>,
}
#[link(name="swiftCore")]
unsafe extern "C" {
    #[link_name="swift_bridgeObjectRelease"] fn STATE_BRIDGE_RELEASE_LABEL();
    #[link_name="swift_unknownObjectRelease"] fn STATE_UNKNOWN_RELEASE_LABEL();
    #[link_name="swift_deallocClassInstance"] fn STATE_DEALLOC_LABEL();
    #[link_name="swift_getSingletonMetadata"] fn STATE_SINGLETON_METADATA_LABEL();
    #[link_name="swift_updateClassMetadata2"] fn STATE_COMPLETE_METADATA_LABEL();
    #[link_name="swift_allocObject"] fn STATE_ALLOC_LABEL();
}
#[link(name="Foundation",kind="framework")]
unsafe extern "C" {
    #[link_name="$s10Foundation3URLVMa"] fn STATE_URL_METADATA_LABEL();
    #[link_name="$s10Foundation4DateVMa"] fn STATE_DATE_METADATA_LABEL();
}
impl AppStateCompilerCleanup {
    fn observe(data:&AppDelegateConstructionData)->Result<Self> {
        let core=b"/usr/lib/swift/libswiftCore.dylib".as_slice();
        let foundation=b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation".as_slice();
        let requests=[
            (c"swift_release",RELEASE_ACTOR_LABEL as *const () as usize,core,0x2fc1ac),
            (c"swift_bridgeObjectRelease",STATE_BRIDGE_RELEASE_LABEL as *const () as usize,core,0x2fbfa8),
            (c"swift_unknownObjectRelease",STATE_UNKNOWN_RELEASE_LABEL as *const () as usize,core,0x2fc290),
            (c"swift_deallocClassInstance",STATE_DEALLOC_LABEL as *const () as usize,core,0x2fbff0),
            (c"swift_getSingletonMetadata",STATE_SINGLETON_METADATA_LABEL as *const () as usize,core,0x2fc110),
            (c"swift_updateClassMetadata2",STATE_COMPLETE_METADATA_LABEL as *const () as usize,core,0x2fc2f0),
            (c"swift_allocObject",STATE_ALLOC_LABEL as *const () as usize,core,0x2fbf54),
            (c"$s10Foundation3URLVMa",STATE_URL_METADATA_LABEL as *const () as usize,foundation,0x2f8f18),
            (c"$s10Foundation4DateVMa",STATE_DATE_METADATA_LABEL as *const () as usize,foundation,0x2f917c),
        ];
        let selected=requests.iter().map(|(name,address,_,_)|(*name,*address)).collect::<Vec<_>>();
        let observed=crate::sdk::imports::observe_unpatched_main_imports(&selected)?;
        let mut imports=Vec::new();
        for (name,address,provider,stub) in requests {
            let mut rows=observed.iter().filter(|row|row.symbol==name);
            let row=rows.next().ok_or("AppState compiler runtime import absent")?.clone();
            let code=metadata::code_identity(address)?;
            if rows.next().is_some()||row.original_address()!=address||code.image()!=provider {
                return Err("AppState compiler runtime import/provider differs".into());
            }
            imports.push((row,code,stub));
        }
        let value=Self{imports};value.verify(data)?;Ok(value)
    }
    fn verify(&self,data:&AppDelegateConstructionData)->Result<()> {
        // Metadata, code and import cells are observations of the same sealed
        // Main. Keep both full source fences around this effect-free sequence.
        unsafe { data.context.with_callback_source_observation(|| self.verify_observation(data)) }
    }
    fn verify_observation(&self,data:&AppDelegateConstructionData)->Result<()> {
        data.context.verify_app_state_compiler_metadata()?;
        data.context.verify_input_parent_source_members()?;
        // Complete allocating/throwing initializer, native fd/fD, lazy accessor
        // and its completion function. Initializer error blocks are INCLUDED;
        // success-only runtime metadata is never substituted for this recipe.
        // Remaining rows are the compiler's outlined destroy/value helpers and
        // concrete typed import stubs. Internal Swift values stay compiler-owned.
        for (offset,bytes,sha) in [
            (0x46268,96,"e76217af0148809b537db8d25e0f93dca6cd74cc00d8a6fdf7550417cec6340e"),
            (0x462c8,4924,"40631910e012fca968cfb1ac018d51fb491ac1e5dca945041f373841e0407422"),
            (0x66280,504,"e497cd1915efa0cffa87410d5e325bad677265039b06892b2cd309a14f36cf01"),
            (0x66478,36,"65884dba58d4b893a3844346526450dd988eba58aab34ab2e3e89cd3c4a0338a"),
            (0x6e260,52,"cd1bdfb096abbc734dfc001f2f0f5370a435370aa2ca3c7fcae6f196740d6c0a"),
            (0x6e294,360,"b862af0136c96e7f93e754160841bd6bfecd4d17747e96a9d4ce10626af08f9b"),
            (0x70e08,12,"39bde89db1e2f6b9143448426a138f03c0fbac232ecf702361260790f91a68ac"),
            (0x70ecc,8,"24cbd69a11d5bf7f512aa5af22ef7a1795710928b33ef7374cedec3017ad3071"),
            (0x70de8,8,"2fd661a55f7333b31221c25cd5d42ea842a959939c47a4a24845ce2d7409d234"),
            (0x71770,8,"be324c1e85dece8f8035ab5b6b95992dcb8cf7201b4e7c740b0b279294cb91a2"),
            (0x71d6c,8,"f3b1b3d0b8d70e6144a1cc7979099b84a2a09b872a827f9c9b77b15c01ed589e"),
            (0x70b08,8,"1298a3c240bdda856cafd14039ed3aa45f2aa5d3f6fbb997cc54b853784f4687"),
            (0x3d4e0,16,"6686a8ba5b7de86a3163d5d7deb3ffd9a3b1ffb42f58581db3814250778fcc1b"),
            (0x70c80,8,"672a8504f3a62e0a514d260f11dc9d57281a98e5fa457a442ce3cbaa7c55a74a"),
            (0x70cac,12,"e535292a10841e68e8e3a573e10d05c235388aa1451eb913cd4a6352830eac22"),
            (0x7088c,12,"7cb803abf2c38b1b3aa1446cd85427d0c36fc83de140ff30969a9112c1ddf263"),
            (0x71d1c,8,"967a14a746d9ef2685e7c5da535b3e8388dffaac57cc7304f76d593fd77f8585"),
            (0x71a74,12,"6971e3fabf43389c32576f9408671761bcf5cd7e63b4e1143b0014c8e44884c5"),
            (0x71fd0,12,"223d0f3a2025eaaecad56e705a32f529896f3264fddae4ee552cd9cb8e58987c"),
            (0xbe0c,8,"6ecaaf5c61eab45d8696cb73cab9ff4c67767f6db06170ed590d6af75ff02aa2"),
            (0x194b4,16,"768102b754b1c7fe5d6ee8b30d8a77705bd2b3eb62c91e01305cda64a3acf493"),
            (0x3d568,8,"67b9e382638cc06044841d8137ffa883bc2ae5288f3b5318348887f39be80183"),
            (0x2f8f18,12,"76333aac7a0058adf6eb5bbf86863deba56325540ffd07474f24c51d9f451d4f"),
            (0x2f917c,12,"aa4266016907bf23f8d7cb32e5bcf6b14240f7eb69714b4f2c6353fc551b710e"),
            (0x2fbfa8,12,"a2012bd00742bab4ef5498c650651d5cf358d8ed85ec7750844cd614b9853cd0"),
            (0x2fbff0,12,"0ce9a5a37f21099bbd755b829f71dfc623fda7e466d48b610261375d4dbbe537"),
            (0x2fc110,12,"e9507c6f5566296f85c74c3656af999d8080df18df5cec8be77fa58d5065e02f"),
            (0x2fc290,12,"ec5d66c13abd12f1c6644fc82e6d7b324743b68e2f3739dbca117b887265c895"),
            (0x2fc2f0,12,"0b110d5040311e76dbd8f8eea54d151eed0ed60652a800180fc5410684e81b19"),
            (0x2fbf54,12,"7d426483accac52771625b86d33ba8ad37b8ddb88307529c4203dfba4411ac48"),
            (0x2fc1ac,12,"c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
        ] {data.context.verify_main_code(data.main_base.checked_add(offset)
            .ok_or("AppState compiler cleanup code overflow")?,bytes,sha)?;}
        for (row,code,offset) in &self.imports {
            row.verify_unpatched()?;
            if metadata::code_identity(row.original_address())?!=*code {
                return Err("AppState compiler runtime code changed".into());
            }
            let stub=data.main_base.checked_add(*offset).ok_or("AppState runtime stub overflow")?;
            // Whole stub bytes were checked above before this bounded read.
            let words=unsafe{std::slice::from_raw_parts(stub as *const u32,3)};
            let (adrp,ldr,branch)=(words[0],words[1],words[2]);
            if adrp&0x9f00001f!=0x90000010||ldr&0xffc003ff!=0xf9400210||branch!=0xd61f0200 {
                return Err("AppState runtime stub ABI differs".into());
            }
            let immediate=(((adrp>>5)&0x7ffff)<<2)|((adrp>>29)&3);
            let signed=if immediate&(1<<20)!=0{immediate as isize-(1<<21)}else{immediate as isize};
            let cell=(stub&!0xfff).checked_add_signed(signed<<12)
                .and_then(|page|page.checked_add((((ldr>>10)&0xfff) as usize)*8))
                .ok_or("AppState runtime import cell overflow")?;
            if cell!=row.cell_address(){return Err("AppState compiler stub targets another import".into());}
            row.verify_unpatched()?;
        }
        data.context.verify_app_state_compiler_metadata()
    }
}

// Compiler-owned server/listener destruction, including MainActor modern and
// back-deployment paths. stop/shutdown is NOT aggregate client/callback drain.
struct ControlServerCompilerCleanup {
    imports:Vec<(crate::sdk::imports::UnpatchedImportObservation,metadata::CodeIdentity,usize,&'static str)>,
    uid:crate::sdk::ImportReplacement,
    files:[crate::sdk::ImportReplacement;10],
}
#[link(name="swift_Concurrency")]
unsafe extern "C" {
    #[link_name="swift_task_deinitOnExecutor"] fn SERVER_DEINIT_EXECUTOR_LABEL();
    #[link_name="swift_task_create"] fn SERVER_TASK_CREATE_LABEL();
    #[link_name="$sScP8rawValues5UInt8Vvg"] fn SERVER_PRIORITY_RAW_LABEL();
    #[link_name="$sScPMa"] fn SERVER_PRIORITY_METADATA_LABEL();
}
#[link(name="swiftCore")]
unsafe extern "C" {
    #[link_name="$ss042_stdlib_isOSVersionAtLeastOrVariantVersiondE0yBi1_Bw_BwBwBwBwBwtF"] fn SERVER_OS_VERSION_LABEL();
    #[link_name="swift_unknownObjectRetain"] fn SERVER_UNKNOWN_RETAIN_LABEL();
    #[link_name="swift_deallocObject"] fn SERVER_DEALLOC_LABEL();
}
impl ControlServerCompilerCleanup {
    fn observe(data:&AppDelegateConstructionData,state:&AppStateCompilerCleanup)->Result<Self> {
        let core=b"/usr/lib/swift/libswiftCore.dylib".as_slice();
        let concurrency=b"/usr/lib/swift/libswift_Concurrency.dylib".as_slice();
        let pthread=b"/usr/lib/system/libsystem_pthread.dylib".as_slice();
        let requests=[
            (c"swift_task_deinitOnExecutor",SERVER_DEINIT_EXECUTOR_LABEL as *const () as usize,concurrency,0x2fc248,"3090070a4519c8365a820a7973df9b9ee270cfed2e5c739cf06cdeac3e6a015f"),
            (c"swift_task_create",SERVER_TASK_CREATE_LABEL as *const () as usize,concurrency,0x2fc230,"6a3dbff1ef33abf8d5e1bb145fc0971e3b843bcf80da941413b0e010d63e8611"),
            (c"$sScP8rawValues5UInt8Vvg",SERVER_PRIORITY_RAW_LABEL as *const () as usize,concurrency,0x2fa79c,"345f7aa0ccc70a16e302cf62b1d32fdb585815c617ada58eefa9b1d3ef2d8f4a"),
            (c"$sScPMa",SERVER_PRIORITY_METADATA_LABEL as *const () as usize,concurrency,0x2fa7a8,"32a496022f7a16c6e936b7d7a1766fdf062ee2e7952737d3aa37031c2a52a9cd"),
            (c"$ss042_stdlib_isOSVersionAtLeastOrVariantVersiondE0yBi1_Bw_BwBwBwBwBwtF",SERVER_OS_VERSION_LABEL as *const () as usize,core,0x2fab2c,"210b08cc754aca9bce694104e3134d1503ea7edfa4a6f75e354ca6d462e86520"),
            (c"swift_unknownObjectRetain",SERVER_UNKNOWN_RETAIN_LABEL as *const () as usize,core,0x2fc2a8,"3b40ac86dc7edf09edba8373b67dd6b25c90f0f1ef1299fa2795e61c363d0ab8"),
            (c"swift_deallocObject",SERVER_DEALLOC_LABEL as *const () as usize,core,0x2fbffc,"9a43a65f2d464d3c961ffc9ed50d0c2291c341b94efdfd377866c0c6743f54ea"),
            (c"pthread_main_np",libc::pthread_main_np as *const () as usize,pthread,0x2fbce4,"ad806d9d41fa7a9408bf111e7736410c05a59d78ae2e7159b8a14ffd41c92c58"),
        ];
        let selected=requests.iter().map(|(name,address,_,_,_)|(*name,*address)).collect::<Vec<_>>();
        let observed=crate::sdk::imports::observe_unpatched_main_imports(&selected)?;
        let mut imports=Vec::new();
        for (name,address,provider,stub,sha) in requests {
            let mut rows=observed.iter().filter(|row|row.symbol==name);
            let row=rows.next().ok_or("ControlServer compiler runtime import absent")?.clone();
            let code=metadata::code_identity(address)?;
            if rows.next().is_some()||row.original_address()!=address||code.image()!=provider {
                return Err("ControlServer compiler runtime import/provider differs".into());
            }
            imports.push((row,code,stub,sha));
        }
        let uid=data.context.observe_control_server_getuid()?;
        let files = [
            data.context.observe_control_server_file_import(c"lstat")?,
            data.context.observe_control_server_file_import(c"unlink")?,
            data.context.observe_control_server_file_import(c"bind")?,
            data.context.observe_control_server_file_import(c"connect")?,
            data.context.observe_control_server_file_import(c"fchmodat")?,
            data.context.observe_control_server_file_import(c"listen")?,
            data.context.observe_control_server_file_import(c"socket")?,
            data.context.observe_control_server_file_import(c"shutdown")?,
            data.context.observe_control_server_file_import(c"accept")?,
            data.context.observe_control_server_file_import(c"close")?,
        ];
        let value=Self{imports,uid,files};value.verify(data,state)?;Ok(value)
    }
    fn verify(&self,data:&AppDelegateConstructionData,state:&AppStateCompilerCleanup)->Result<()> {
        data.context.verify_control_server_compiler_metadata()?;
        // Shared, concretely bound ARC/allocation/URL imports from the same
        // sealed Main; no implicit dependence on caller verification order.
        state.verify(data)?;
        data.system.verify_notification_runtime(&data.context)?;
        // This includes ALL compiler-generated fD/stop/SocketListener/locked
        // update/back-deployment task code; only code provenance is composed.
        data.context.verify_original_compiler_text()?;
        let uid=data.context.observe_control_server_getuid()?;
        if uid.cell_address()!=self.uid.cell_address()||uid.original!=self.uid.original
            ||uid.replacement!=self.uid.replacement||self.uid.selected_address()!=uid.replacement {
            return Err("ControlServer owned getuid binding changed".into());
        }
        let mut stubs=Vec::new();
        for (row,code,offset,sha) in &self.imports {
            row.verify_unpatched()?;
            if metadata::code_identity(row.original_address())?!=*code {
                return Err("ControlServer compiler runtime code changed".into());
            }
            stubs.push((row.cell_address(),*offset,*sha));
        }
        stubs.push((uid.cell_address(),0x2fb9a8,"4b98dcbe53362f9f7098f663adbbf9f6b22bcd3fc700d714ca3a53d1e9713fb2"));
        for (expected, offset, hash) in [
            (&self.files[0], 0x2fba20, "5cf197002599debdb2f3d987458f841f27b514afb9d3fb9d8ee62577411b7a6e"),
            (&self.files[1], 0x2fc380, "f02afcaaf8d63eced4228fd41767501a2a6b059554d5969d2d014f6f03284a43"),
            (&self.files[2], 0x2fb6c0, "1457217524c3935a0c787b143742289b3af67fa6a8dee6bf714e70c43d202610"),
            (&self.files[3], 0x2fb744, "e53b0ecf2e74fc392756f800e4c563760bc0de3746a69d568396f8a1fae98e72"),
            (&self.files[4], 0x2fb834, "346de823713fc1a6f54854546d8d14d8c2a70b5bec3f32d7bfd2de71c7c23f5d"),
            (&self.files[5], 0x2fba08, "3de5503d10619bbc54c17383949219a112be93ae7ca2e41cb8b067919407ead8"),
            (&self.files[6], 0x2fbedc, "ff5be9c527e5ad253d354c5309f0291dbd54be2a43d67413d00cb9397d8a8901"),
            (&self.files[7], 0x2fbe94, "417456edeeac713c2961205847a753345d65acea17a50c42e52887895cdff6f9"),
            (&self.files[8], 0x2fb660, "28c0d48d4396cd34fcbfaa57ae90d94314cd354b13241d9545b809fc1888f9fb"),
            (&self.files[9], 0x2fb720, "2c5caf8a0c2d06209a5bd8c34ad2319c3db8846a0b98a1bab62634ffc2725f03"),
        ] {
            let actual = data.context.observe_control_server_file_import(expected.symbol)?;
            if actual.cell_address() != expected.cell_address() || actual.original != expected.original
                || actual.replacement != expected.replacement || expected.selected_address() != expected.replacement
            {
                return Err("ControlServer owned file import binding changed".into());
            }
            stubs.push((actual.cell_address(), offset, hash));
        }
        // This fixed stub loop only observes current RX bytes and cell targets.
        // Both dynamic getuid/guardian observations remain outside the scope.
        unsafe{data.context.with_callback_source_observation(|| {
        for (slot,offset,sha) in stubs {
            let stub=data.main_base.checked_add(offset).ok_or("ControlServer runtime stub overflow")?;
            data.context.verify_main_code(stub,12,sha)?;
            let words=std::slice::from_raw_parts(stub as *const u32,3);
            let (adrp,ldr,branch)=(words[0],words[1],words[2]);
            if adrp&0x9f00001f!=0x90000010||ldr&0xffc003ff!=0xf9400210||branch!=0xd61f0200 {
                return Err("ControlServer runtime stub ABI differs".into());
            }
            let immediate=(((adrp>>5)&0x7ffff)<<2)|((adrp>>29)&3);
            let signed=if immediate&(1<<20)!=0{immediate as isize-(1<<21)}else{immediate as isize};
            let cell=(stub&!0xfff).checked_add_signed(signed<<12)
                .and_then(|page|page.checked_add((((ldr>>10)&0xfff) as usize)*8))
                .ok_or("ControlServer runtime import cell overflow")?;
            if cell!=slot{return Err("ControlServer compiler stub targets another import".into());}
        }
        Ok(())
        })}?;
        for (row,_,_,_) in &self.imports {row.verify_unpatched()?;}
        let after=data.context.observe_control_server_getuid()?;
        if after.cell_address()!=uid.cell_address()||after.original!=uid.original||after.replacement!=uid.replacement
            ||self.uid.selected_address()!=after.replacement {return Err("ControlServer owned getuid changed after stub qualification".into());}
        data.context.verify_control_server_compiler_metadata()?;
        if self.files.iter().any(|row| row.selected_address() != row.replacement) {
            return Err("ControlServer owned file import changed after stub qualification".into());
        }
        Ok(())
    }
}

// Fixed original controller memory cohort. These are constructed by the Swift
// compiler, not allocated SDK proxies. UI/timer retirement remains a distinct
// lifecycle requirement: releasing a token never means removeMonitor/hide/stop.
struct ControllerCompilerCleanup {
    classes:[metadata::ClassHandle;4],
    objc:Vec<(metadata::ClassHandle,metadata::LifecycleSnapshot,metadata::MethodSnapshot,metadata::MethodSnapshot)>,
    timer:metadata::ClassHandle,
    concrete_timer:metadata::ClassHandle,
    timer_factory:metadata::MethodSnapshot,
    timer_invalidate:metadata::MethodSnapshot,
}
impl ControllerCompilerCleanup {
    fn observe(data:&AppDelegateConstructionData,state:&AppStateCompilerCleanup,server:&ControlServerCompilerCleanup)->Result<Self> {
        let classes=[
            metadata::loaded_class(c"RuycastF1Reference.PaletteController")?,
            metadata::loaded_class(c"RuycastF1Reference.ManagementController")?,
            metadata::loaded_class(c"RuycastF1Reference.SystemMonitorController")?,
            metadata::loaded_class(c"RuycastF1Reference.SystemMonitorModel")?,
        ];
        let mut objc=Vec::new();
        for class in [classes[1],classes[2]] {
            objc.push((class,metadata::lifecycle(class)?,
                metadata::method(class,c"dealloc",false)?.ok_or("Original controller NSObject dealloc absent")?,
                metadata::method(class,c".cxx_destruct",false)?.ok_or("Original controller CXX destruction absent")?));
        }
        let timer=metadata::loaded_class(c"NSTimer")?;
        let concrete_timer=metadata::loaded_class(c"__NSCFTimer")?;
        let timer_factory=metadata::method(timer,c"scheduledTimerWithTimeInterval:repeats:block:",true)?
            .ok_or("Original Foundation timer factory absent")?;
        let timer_invalidate=metadata::method(concrete_timer,c"invalidate",false)?
            .ok_or("Concrete Foundation timer invalidate absent")?;
        let value=Self{classes,objc,timer,concrete_timer,timer_factory,timer_invalidate};value.verify(data,state,server)?;Ok(value)
    }
    fn verify(&self,data:&AppDelegateConstructionData,state:&AppStateCompilerCleanup,server:&ControlServerCompilerCleanup)->Result<()> {
        // Includes the concrete shared Swift/Core/Foundation/ObjC and modern/
        // fallback MainActor runtime bindings; not an arbitrary image permit.
        server.verify(data,state)?;
        self.verify_timer_dispatch(data)?;
        data.context.verify_controller_compiler_metadata()?;
        for (class,offset) in self.classes.iter().zip([0x3f2968usize,0x3ea430,0x3eae10,0x3f4ef8]) {
            if data.main_base.checked_add(offset)!=Some(class.address())||metadata::registered_class(class.address())?!=*class {
                return Err("Original controller class metadata differs".into());
            }
        }
        let root=data.root_init.method().owner();
        for ((class,lifecycle,dealloc,destruct),destruct_offset) in self.objc.iter().zip([0x12a730usize,0x1c2414]) {
            let physical=unsafe{&*(class.address() as *const AnyClass)};
            if physical.superclass().map(|parent|parent as *const AnyClass as usize)!=Some(root)
                ||dealloc.owner()!=root||dealloc.code().image()!=b"/usr/lib/libobjc.A.dylib"
                ||super::super::super::simple_abi(dealloc)?!=b"v@:"
                ||destruct.owner()!=class.address()||super::super::super::simple_abi(destruct)?!=b"v@:"
                ||data.main_base.checked_add(destruct_offset)!=Some(destruct.implementation())
                ||destruct.code().uuid()!=data.destructor.code().uuid()
                ||metadata::lifecycle(*class)?!=*lifecycle {
                return Err("Original NSObject controller destruction ABI/owner/lifecycle differs".into());
            }
            metadata::verify_current_method(*class,c"dealloc",false,dealloc)?;
            metadata::verify_current_method(*class,c".cxx_destruct",false,destruct)?;
        }
        data.context.verify_original_compiler_text()?;
        data.context.verify_controller_compiler_metadata().map(|_|())
    }
    fn verify_timer_dispatch(&self,data:&AppDelegateConstructionData)->Result<()> {
        // Exact observed platform class cluster, not arbitrary NSTimer
        // subclasses. No timer is created or invalidated by this observation.
        let class=unsafe{&*(self.concrete_timer.address() as *const AnyClass)};
        if class.superclass().map(|parent|parent as *const AnyClass as usize)!=Some(self.timer.address())
            ||self.timer_factory.encoding()!=b"@36@0:8d16B24@?28"
            ||self.timer_factory.code().image()!=b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
            ||self.timer_invalidate.owner()!=self.concrete_timer.address()
            ||super::super::super::simple_abi(&self.timer_invalidate)?!=b"v@:"
            ||self.timer_invalidate.code().image()!=b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation" {
            return Err("Original timer factory/concrete invalidate contract differs".into());
        }
        metadata::verify_current_method(self.timer,c"scheduledTimerWithTimeInterval:repeats:block:",true,&self.timer_factory)?;
        metadata::verify_current_method(self.concrete_timer,c"invalidate",false,&self.timer_invalidate)?;
        let (row,code)=data.system.code.iter().find(|(row,_)|row.symbol==c"objc_msgSend")
            .ok_or("Original timer dispatch import absent")?;
        if data.main_base.checked_add(0x3c6940)!=Some(row.cell_address())
            ||code.image()!=b"/usr/lib/libobjc.A.dylib" {
            return Err("Original timer dispatch provider/cell differs".into());
        }
        row.verify_unpatched()?;
        data.context.verify_main_code(data.main_base.checked_add(0x2fbb04).ok_or("Timer message stub overflow")?,12,
            "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f")?;
        row.verify_unpatched()
    }
    fn verify_receiver_fields(&self,data:&AppDelegateConstructionData,receiver:&ReceiverIdentity)->Result<()> {
        if !receiver.belongs_to(&data.model)||!receiver.original_source()
            ||receiver.class() as *const AnyClass as usize!=data.class.address() {
            return Err("Controller cleanup has another original root issuer".into());
        }
        receiver.verify_allocation()?;
        let offsets=data.context.verify_controller_compiler_metadata()?;
        let pointer=receiver.receiver() as *const AnyObject;
        if unsafe{&*pointer}.class()!=data.class(){return Err("Controller cleanup physical root differs".into());}
        let read=||[24usize,32,40,48].map(|offset|unsafe{pointer.cast::<u8>().add(offset).cast::<usize>().read()});
        let before=read();
        if before[3]!=0 {
            super::super::super::objc::verify_status_item_cleanup(before[3] as *mut AnyObject,&data.model)?;
            receiver.verify_allocation()?;
            if read()!=before{return Err("Status-item/source fields changed during SDK cleanup qualification".into());}
        }
        for (pointer,class,names,start) in [
            (before[1],self.classes[1],[c"state",c"navigation",c"window"].as_slice(),0usize),
            (before[2],self.classes[2],[c"model",c"window"].as_slice(),3usize),
        ] {
            if pointer==0{continue;}
            let class=unsafe{&*(class.address() as *const AnyClass)};
            let fields=class.instance_variables();
            for (index,name) in names.iter().enumerate() {
                let mut matches=fields.iter().filter(|field|field.name()==*name);
                let field=matches.next().ok_or("Controller destructor field absent")?;
                let offset=usize::try_from(field.offset()).map_err(|_|"Controller field offset width")?;
                if matches.next().is_some()||offset<8||!offset.is_multiple_of(8)||offsets[start+index]!=offset
                    ||offset.checked_add(8).is_none_or(|end|end>class.instance_size()) {
                    return Err("Controller destructor offset cell differs from actual bounded field".into());
                }
            }
        }
        for (field,class) in before.iter().zip(&self.classes[..3]) {
            // These words are read only from the actual known strong fields
            // of this retained source root, not from an arbitrary VM pointer.
            if *field!=0&&unsafe{&*(*field as *const AnyObject)}.class() as *const AnyClass as usize!=class.address() {
                return Err("Original strong controller field has another concrete class".into());
            }
        }
        if before[2]!=0 {
            let class=unsafe{&*(self.classes[2].address() as *const AnyClass)};
            let fields=class.instance_variables();
            let mut matches=fields.iter().filter(|field|field.name()==c"model");
            let field=matches.next().ok_or("SystemMonitorController model field absent")?;
            let offset=usize::try_from(field.offset()).map_err(|_|"Monitor model field offset width")?;
            if matches.next().is_some()||offset<8||!offset.is_multiple_of(8)||offset!=offsets[3]
                ||offset.checked_add(8).is_none_or(|end|end>class.instance_size()) {
                return Err("Monitor model field is not a unique bounded strong field".into());
            }
            let model_slot=before[2].checked_add(offset).ok_or("Monitor model slot overflow")?;
            let model=unsafe{(model_slot as *const usize).read()};
            if model==0||unsafe{&*(model as *const AnyObject)}.class() as *const AnyClass as usize!=self.classes[3].address() {
                return Err("Monitor controller owns another concrete model".into());
            }
            let model_class=unsafe{&*(self.classes[3].address() as *const AnyClass)};
            let fields=model_class.instance_variables();
            let mut fields=fields.iter().filter(|field|field.name()==c"timer");
            let timer_field=fields.next().ok_or("Monitor model timer field absent")?;
            let timer_offset=usize::try_from(timer_field.offset()).map_err(|_|"Monitor timer offset width")?;
            if fields.next().is_some()||timer_offset<16||!timer_offset.is_multiple_of(8)||timer_offset!=offsets[5]
                ||timer_offset.checked_add(8).is_none_or(|end|end>model_class.instance_size()) {
                return Err("Monitor timer does not match its actual destructor field".into());
            }
            let timer_slot=model.checked_add(timer_offset).ok_or("Monitor timer slot overflow")?;
            let timer=unsafe{(timer_slot as *const usize).read()};
            if timer!=0&&unsafe{&*(timer as *const AnyObject)}.class() as *const AnyClass as usize!=self.concrete_timer.address() {
                return Err("Monitor timer receiver differs from the qualified Foundation class".into());
            }
            self.verify_timer_dispatch(data)?;
            receiver.verify_allocation()?;
            if unsafe{(model_slot as *const usize).read()}!=model
                ||unsafe{(timer_slot as *const usize).read()}!=timer {
                return Err("Monitor model/timer changed during cleanup check".into());
            }
        }
        receiver.verify_allocation()?;
        if data.context.verify_controller_compiler_metadata()?!=offsets{return Err("Controller destructor offset cells changed during custody check".into());}
        if unsafe{&*pointer}.class()!=data.class()||read()!=before{return Err("Strong controller fields changed during cleanup check".into());}
        Ok(())
    }
}

struct SubscriptionsCompilerMemory {
    imports:Vec<(crate::sdk::imports::UnpatchedImportObservation,metadata::CodeIdentity)>,
}
#[link(name="Combine",kind="framework")]
unsafe extern "C" {
    #[link_name="$s7Combine14AnyCancellableCMa"] fn SUBSCRIPTIONS_ANY_METADATA_LABEL();
    #[link_name="$s7Combine14AnyCancellableC5store2inyShyACGz_tF"] fn SUBSCRIPTIONS_STORE_LABEL();
}
impl SubscriptionsCompilerMemory {
    fn observe(data:&AppDelegateConstructionData)->Result<Self> {
        let requested=[(c"$s7Combine14AnyCancellableCMa",SUBSCRIPTIONS_ANY_METADATA_LABEL as *const () as usize),
            (c"$s7Combine14AnyCancellableC5store2inyShyACGz_tF",SUBSCRIPTIONS_STORE_LABEL as *const () as usize)];
        let observed=crate::sdk::imports::observe_unpatched_main_imports(&requested)?;
        let mut imports=Vec::new();
        for (symbol,address) in requested {
            let mut rows=observed.iter().filter(|row|row.symbol==symbol);
            let row=rows.next().ok_or("Subscriptions Combine import absent")?.clone();
            let code=metadata::code_identity(address)?;
            if rows.next().is_some()||row.original_address()!=address
                ||code.image()!=b"/System/Library/Frameworks/Combine.framework/Versions/A/Combine" {
                return Err("Subscriptions Combine import/provider differs".into());
            }
            imports.push((row,code));
        }
        let value=Self{imports};value.verify(data)?;Ok(value)
    }
    fn verify(&self,data:&AppDelegateConstructionData)->Result<()> {
        // Source/type/layout/import observations only; never a Set access,
        // cancellation, callback, guardian call or ARC effect.
        unsafe{data.context.with_callback_source_observation(||self.verify_observation(data))}
    }
    fn verify_observation(&self,data:&AppDelegateConstructionData)->Result<()> {
        // Exact compiled main source owns the field declaration, empty init
        // and all three store(in:) subscriptions. The compiler owns values.
        data.verify_bodies_and_layout()?;
        data.context.verify_original_compiler_text()?;
        data.context.verify_subscriptions_compiler_type()?;
        for (row,code) in &self.imports {
            row.verify_unpatched()?;
            if metadata::code_identity(row.original_address())?!=*code{return Err("Subscriptions Combine runtime changed".into());}
            let (stub,cell,sha)=if row.symbol==c"$s7Combine14AnyCancellableCMa" {
                (0x2f962cusize,0x3c4dc0usize,"bf039c47e3e845f80d20013d4c36adf36208add111609b672911a31250c75a5e")
            } else {(0x2f9620,0x3c4db8,"3a7f51dce85db1d4cae8953b25aaf8f3f0877ab3eab9f39813d05e5c7c3234c5")};
            if data.main_base.checked_add(cell)!=Some(row.cell_address()){return Err("Subscriptions Combine cell differs".into());}
            data.context.verify_main_code(data.main_base.checked_add(stub).ok_or("Subscriptions stub overflow")?,12,sha)?;
            row.verify_unpatched()?;
        }
        data.context.verify_subscriptions_compiler_type()
    }
}

// Class-local contribution for workspace_appdelegate_constructor.rs.
// Finite compiler getter effect: only an additional opaque AppState reference.
// No child memory layout, input callback, final ARC or Source14 permission.
#[link(name="swiftCore")]
unsafe extern "C" {
    #[link_name="swift_retain"] fn STATE_RETAIN_LABEL();
    #[link_name="swift_beginAccess"] fn STATE_BEGIN_ACCESS_LABEL();
}
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn swift_self_adapter(_root:usize,_entry:usize)->usize {
    // Preserve the entire C callee-save set, independently of Swift's
    // context/error/async register roles. The finite getter returns x0 only.
    core::arch::naked_asm!(
        "sub sp,sp,#160",
        "stp x19,x20,[sp,#0]","stp x21,x22,[sp,#16]",
        "stp x23,x24,[sp,#32]","stp x25,x26,[sp,#48]",
        "stp x27,x28,[sp,#64]","stp x29,x30,[sp,#80]",
        "stp d8,d9,[sp,#96]","stp d10,d11,[sp,#112]",
        "stp d12,d13,[sp,#128]","stp d14,d15,[sp,#144]",
        "mov x20,x0","blr x1",
        "ldp d8,d9,[sp,#96]","ldp d10,d11,[sp,#112]",
        "ldp d12,d13,[sp,#128]","ldp d14,d15,[sp,#144]",
        "ldp x19,x20,[sp,#0]","ldp x21,x22,[sp,#16]",
        "ldp x23,x24,[sp,#32]","ldp x25,x26,[sp,#48]",
        "ldp x27,x28,[sp,#64]","ldp x29,x30,[sp,#80]",
        "add sp,sp,#160","ret");
}
#[unsafe(naked)]
unsafe extern "C" fn swift_clear_optional_child(_parent: usize, _setter: usize) {
    core::arch::naked_asm!(
        "sub sp,sp,#160",
        "stp x19,x20,[sp,#0]", "stp x21,x22,[sp,#16]",
        "stp x23,x24,[sp,#32]", "stp x25,x26,[sp,#48]",
        "stp x27,x28,[sp,#64]", "stp x29,x30,[sp,#80]",
        "stp d8,d9,[sp,#96]", "stp d10,d11,[sp,#112]",
        "stp d12,d13,[sp,#128]", "stp d14,d15,[sp,#144]",
        "mov x20,x0", "mov x0,xzr", "blr x1",
        "ldp d8,d9,[sp,#96]", "ldp d10,d11,[sp,#112]",
        "ldp d12,d13,[sp,#128]", "ldp d14,d15,[sp,#144]",
        "ldp x19,x20,[sp,#0]", "ldp x21,x22,[sp,#16]",
        "ldp x23,x24,[sp,#32]", "ldp x25,x26,[sp,#48]",
        "ldp x27,x28,[sp,#64]", "ldp x29,x30,[sp,#80]",
        "add sp,sp,#160", "ret",
    )
}
struct StateGetterPlan {
    imports:Vec<(crate::sdk::imports::UnpatchedImportObservation,metadata::CodeIdentity)>,
    retired:AtomicBool,
}
impl StateGetterPlan {
    fn observe(root:&InitializedAppDelegateRoot)->Result<Self> {
        root.pointer_on_creator()?;
        if root.state_capture_retired.load(Ordering::Acquire){return Err("AppState capture revoked during parent guard".into());}
        let requests=[(c"swift_retain",STATE_RETAIN_LABEL as *const () as usize),
            (c"swift_beginAccess",STATE_BEGIN_ACCESS_LABEL as *const () as usize)];
        let rows=crate::sdk::imports::observe_unpatched_main_imports(&requests)?;
        let mut imports=Vec::new();
        for (name,address) in requests {
            let mut found=rows.iter().filter(|row|row.symbol==name);
            let row=found.next().ok_or("AppState getter runtime import absent")?.clone();
            if found.next().is_some()||row.original_address()!=address{return Err("AppState getter runtime import differs".into());}
            let code=metadata::code_identity(address)?;
            if code.image()!=b"/usr/lib/swift/libswiftCore.dylib"{return Err("AppState getter runtime provider differs".into());}
            imports.push((row,code));
        }
        let plan=Self{imports,retired:AtomicBool::new(false)};plan.verify(root)?;Ok(plan)
    }
    fn verify_stub(&self,root:&InitializedAppDelegateRoot,row:&crate::sdk::imports::UnpatchedImportObservation)->Result<()> {
        let (offset,cell_offset,sha)=match row.symbol {
            name if name==c"swift_retain"=>(0x2fc1c4,0x3c7448,"b681305e25fa18b584abf72b26653eb8a3fab78666524b51f2d98df7a2e73e01"),
            name if name==c"swift_beginAccess"=>(0x2fbf9c,0x3c72d0,"7928d7185a73cd076ff53c082e37ecf0a2ac92370f8978978b188dac6bd8324d"),
            _=>return Err("AppState getter has an unknown runtime edge".into()),
        };
        let data=&root.declaration.data;
        let stub=data.main_base.checked_add(offset).ok_or("getter stub overflow")?;
        row.verify_unpatched()?; // full64 cell before stub qualification
        data.context.verify_main_code(stub,12,sha)?;
        let words=unsafe{std::slice::from_raw_parts(stub as *const u32,3)};
        let (adrp,ldr,branch)=(words[0],words[1],words[2]);
        if adrp&0x9f00001f!=0x90000010 || ldr&0xffc003ff!=0xf9400210 || branch!=0xd61f0200 {
            return Err("getter runtime stub is not ADRP/LDR/BR x16".into());
        }
        let immediate=(((adrp>>5)&0x7ffff)<<2)|((adrp>>29)&3);
        let signed=if immediate&(1<<20)!=0 {(immediate as isize)-(1<<21)} else {immediate as isize};
        let page=(stub&!0xfff).checked_add_signed(signed<<12).ok_or("getter stub page overflow")?;
        let cell=page.checked_add((((ldr>>10)&0xfff) as usize)*8).ok_or("getter stub cell overflow")?;
        if cell!=row.cell_address() || data.main_base.checked_add(cell_offset)!=Some(cell) {
            return Err("getter runtime stub targets another import cell".into());
        }
        row.verify_unpatched()?;
        data.context.verify_main_code(stub,12,sha)?;
        row.verify_unpatched() // same full64 cell after bounded RX/body reads
    }
    fn verify(&self,root:&InitializedAppDelegateRoot)->Result<()> {
        let data=&root.declaration.data;
        data.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire)||root.state_capture_retired.load(Ordering::Acquire){return Err("AppState getter contribution retired".into());}
        let result=(|| {
            root.pointer_on_creator()?;
            if root.state_capture_retired.load(Ordering::Acquire){return Err("AppState capture revoked during parent guard".into());}
            for (offset,bytes,sha) in [
                (0x1fb92c,12,"bc1621b0c8607f72798585bed9ec9aa6c5d2478c3430d4f0664bd646f2b44975"),
                (0x460b8,40,"dc0bb379231d6d65e99444493308690da6d8f43d634d8ce72afaad753a719d72"),
                (0x704a0,20,"6bcb08063bfe9cce75bdf3971713859a88c54c7c308238b4de73e5d1c3764351"),
                (0x196e4,12,"e9c467136f6157fff774fa73f71602bed11820a30dda216d62ec9dc63c61cd20"),
            ] {data.context.verify_main_code(data.main_base.checked_add(offset).ok_or("AppState getter range overflow")?,bytes,sha)?;}
            // Core fixed full64 state cell has expected value8; the genuine
            // getter reads this same cell. No child extent is inferred here.
            let offsets=data.context.observe_app_delegate_offset_cells()?;
            data.verify_offset_observation(&offsets)?;
            for (row,code) in &self.imports {
                self.verify_stub(root,row)?;
                if metadata::code_identity(row.original_address())?!=*code{return Err("AppState getter runtime code changed".into());}
                row.verify_unpatched()?;
            }
            let pair=|base:u32,first:u32,second:u32,offset:u32| {
                base|((offset/8)<<15)|(second<<10)|(31<<5)|first
            };
            let mut words=vec![0xd10283ffu32]; // sub sp,sp,#160
            for (a,b,offset) in [(19,20,0),(21,22,16),(23,24,32),(25,26,48),(27,28,64),(29,30,80)] {
                words.push(pair(0xa9000000,a,b,offset));
            }
            for (a,b,offset) in [(8,9,96),(10,11,112),(12,13,128),(14,15,144)] {
                words.push(pair(0x6d000000,a,b,offset));
            }
            words.extend([0xaa0003f4,0xd63f0020]); // self x20; BLR fixed entry x1
            for (a,b,offset) in [(8,9,96),(10,11,112),(12,13,128),(14,15,144)] {
                words.push(pair(0x6d400000,a,b,offset));
            }
            for (a,b,offset) in [(19,20,0),(21,22,16),(23,24,32),(25,26,48),(27,28,64),(29,30,80)] {
                words.push(pair(0xa9400000,a,b,offset));
            }
            words.extend([0x910283ff,0xd65f03c0]); // restore SP; return retained x0
            let bytes=words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
            let sha=format!("{:x}",sha2::Sha256::digest(&bytes));
            data.context.verify_sdk_code(swift_self_adapter as *const () as usize,bytes.len(),&sha)?;
            // No business/model callback follows the fresh code/import/cell
            // checks. Only pure custody/monotone retirement checks may follow.
            root.verify_held_epoch()?;
            if self.retired.load(Ordering::Acquire)
                ||root.declaration.retired.load(Ordering::Acquire)
                ||data.retired.load(Ordering::Acquire)
                ||root.state_capture_retired.load(Ordering::Acquire) {
                return Err("AppState getter contribution revoked".into());
            }
            Ok(())
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
}
/// Physical extra reference returned by the actual compiled state getter.
/// It is an opaque parent anchor, NOT an AppState/child access or Input permit.
/// Final release is quarantined until its source ARC/cleanup is qualified.
pub struct AppStateCustody {
    child_capture_claimed:[AtomicBool;2],
    child_capture_retired:[AtomicBool;2],
    root:Arc<InitializedAppDelegateRoot>,
    pointer:usize,
    plan:StateGetterPlan,
    retired:AtomicBool,
}
impl AppStateCustody {
    // The caller holds this exact custody. Read only immutable custody fields,
    // refusal flags and the strong slot, without re-locking the Workspace.
    pub(crate) fn verify_open_owner_slot(&self, expected: usize, generation: u64) -> Result<()> {
        if expected != self.pointer || self.root.extra.generation != generation
            || self.retired.load(Ordering::Acquire)
            || self.root.state_capture_retired.load(Ordering::Acquire)
            || self.plan.retired.load(Ordering::Acquire)
            || self.root.declaration.retired.load(Ordering::Acquire)
            || self.root.declaration.data.retired.load(Ordering::Acquire)
        {
            return Err("Reference Open AppState custody retired or changed".into());
        }
        let slot = (self.root.extra.pointer + 8) as *const usize;
        if unsafe { slot.read_volatile() } != expected {
            return Err("Reference Open AppState strong slot changed".into());
        }
        Ok(())
    }

    pub(crate) fn verify_safari_cache_owner(&self, services: usize) -> Result<()> {
        let owner = self.pointer_on_creator()?;
        let class = metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        let slot = InitializedAppDelegateRoot::monitor_field_slot(owner, class, c"systemServices")?;
        self.verify_held_epoch()?;
        if services == 0 || unsafe { slot.read_volatile() } != services {
            return Err("Reference Safari cache does not belong to the active AppState".into());
        }
        Ok(())
    }

    pub(crate) fn invalidate_shutdown_poll_timer(&self) -> Result<()> {
        use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopTimer, CFType, CFRetained};
        let data = &self.root.declaration.data;
        let owner = self.pointer_on_creator()?;
        let class = metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        if data.main_base.checked_add(0x3ec968) != Some(class.address()) {
            return Err("Reference AppState timer class differs".into());
        }
        let slot = InitializedAppDelegateRoot::monitor_field_slot(owner, class, c"timer")?;
        if owner.checked_add(data.context.app_state_poll_timer_offset()?) != Some(slot as usize) {
            return Err("Reference AppState timer offset differs".into());
        }
        self.verify_held_epoch()?;
        let address = unsafe { slot.read_volatile() };
        let Some(receiver) = std::ptr::NonNull::new(address as *mut AnyObject) else { return Ok(()) };
        let timer_class = metadata::loaded_class(c"__NSCFTimer")?;
        if unsafe { receiver.as_ref() }.class() as *const AnyClass as usize != timer_class.address() {
            return Err("Reference AppState poll receiver is not a Foundation timer".into());
        }
        let timer = unsafe { receiver.cast::<CFType>().as_ref() }.downcast_ref::<CFRunLoopTimer>()
            .ok_or("Reference AppState poll is not a CF timer")?;
        let timer = unsafe { CFRetained::retain(std::ptr::NonNull::from(timer)) };
        let run_loop = CFRunLoop::current().ok_or("Reference AppState poll runloop absent")?;
        let mode = unsafe { kCFRunLoopDefaultMode }.ok_or("Reference AppState poll mode absent")?;
        if self.pointer_on_creator()? != owner || unsafe { slot.read_volatile() } != address {
            return Err("Reference AppState poll changed before invalidation".into());
        }
        if timer.is_valid() {
            if !run_loop.contains_timer(Some(&timer), Some(mode)) {
                return Err("Reference AppState poll is not on the owner loop".into());
            }
            timer.invalidate();
        }
        if timer.is_valid() || run_loop.contains_timer(Some(&timer), Some(mode))
            || unsafe { slot.read_volatile() } != address
        {
            return Err("Reference AppState poll survived invalidation or changed owner".into());
        }
        drop(timer);
        self.verify_held_epoch()
    }

    pub(crate) fn verify_configuration_observer(&self, attached: bool) -> Result<()> {
        self.observe_configuration_sources(attached, false).map(|_| ())
    }

    pub(crate) fn cancel_configuration_sources(&self) -> Result<usize> {
        self.observe_configuration_sources(true, true)?
            .ok_or("Reference configuration worker queue absent".into())
    }

    fn observe_configuration_sources(&self, attached: bool, cancel: bool) -> Result<Option<usize>> {
        let data = &self.root.declaration.data;
        let pointer = self.pointer_on_creator()?;
        let class = metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        if data.main_base.checked_add(0x3ec968) != Some(class.address()) {
            return Err("Reference configuration observer AppState class differs".into());
        }
        let offset = bounded_swift_field(class, data.context.configuration_observer_offset()?)?;
        let fields = unsafe { &*(class.address() as *const AnyClass) }.instance_variables();
        let mut matches = fields.iter().filter(|field| field.name() == c"configObserver");
        if matches.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(offset)
            || matches.next().is_some()
        {
            return Err("Reference configuration observer named ivar differs".into());
        }
        self.verify_held_epoch()?;
        let slot = pointer.checked_add(offset).ok_or("Reference configuration observer field overflow")?;
        let observer = unsafe { (slot as *const usize).read_volatile() };
        if (observer != 0) != attached {
            return Err("Reference configuration observer attachment differs".into());
        }
        let queue = if attached {
            let queue = unsafe { data.context.verify_configuration_observer_sources(observer, cancel) }?;
            if unsafe { (slot as *const usize).read_volatile() } != observer {
                return Err("Reference configuration observer changed during source observation".into());
            }
            Some(queue)
        } else {
            None
        };
        self.verify_held_epoch()?;
        Ok(queue)
    }

    /// # Safety
    /// The caller keeps the exact qualified borrowed Bridge alive and unchanged.
    pub(crate) unsafe fn verify_borrowed_reference_box(&self, pointer: usize) -> Result<()> {
        let state = self.pointer_on_creator()?;
        let parent = self.root.pointer_on_creator()?;
        let context = &self.root.declaration.data.context;
        let offsets = context.observe_app_delegate_offset_cells()?;
        if offsets.cells()[2].1 != 24 {
            return Err("Reference AppDelegate palette offset differs".into());
        }
        self.verify_held_epoch()?;
        let palette_slot = unsafe { parent.cast::<u8>().add(24).cast::<usize>() };
        let palette = unsafe { palette_slot.read_volatile() };
        unsafe { context.verify_borrowed_reference_box(pointer, state, palette) }?;
        if self.pointer_on_creator()? != state || self.root.pointer_on_creator()? != parent
            || unsafe { palette_slot.read_volatile() } != palette
        {
            return Err("Reference borrowed box parent fields changed".into());
        }
        self.verify_held_epoch()
    }

    /// Pure custody and known-refusal tail only. Every future userdata read
    /// still requires separate fresh typed child/source/code verification.
    pub fn verify_held_epoch(&self)->Result<()> {
        self.root.verify_held_epoch()?; // early fork/PID gate before any Err allocation
        let check=||->Result<()> {
            if self.retired.load(Ordering::Acquire)
                ||self.root.state_capture_retired.load(Ordering::Acquire)
                ||self.plan.retired.load(Ordering::Acquire)
                ||self.root.declaration.retired.load(Ordering::Acquire)
                ||self.root.declaration.data.retired.load(Ordering::Acquire) {
                return Err("AppState held binding has a monotone refusal".into());
            }
            Ok(())
        };
        check()?;
        self.root.verify_held_epoch()?;
        check()
    }
    pub fn verify_reference_binding(&self,
        admission:&Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        model:&Arc<Workspace>,
    )->Result<()> {
        self.root.verify_held_epoch()?; // early fork/PID before refusal or dynamic work
        let result=(|| {
            self.root.declaration.data.context.cleanup_metadata_only()?;
            self.verify_held_epoch()?;
            self.root.verify_reference_binding(admission,model)?;
            self.pointer_on_creator()?;
            self.root.verify_reference_binding(admission,model)?;
            self.pointer_on_creator()?;
            self.verify_held_epoch()
        })();
        // A refused state binding stays refused even if a nested caller ignores
        // Err. This retires only this binding, never its physical known +1.
        if result.is_err(){self.retired.store(true,Ordering::Release);}
        result
    }
    pub fn pointer_on_creator(&self)->Result<usize> {
        self.root.declaration.data.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire)||self.root.state_capture_retired.load(Ordering::Acquire){return Err("AppState parent binding retired".into());}
        let result=(|| {
            let parent=self.root.pointer_on_creator()?;
            self.plan.verify(&self.root)?;
            self.verify_class()?;
            // The parent has genuine64B layout and a qualified full-width8
            // state offset. Compare only; never dereference a child field.
            let state=unsafe{parent.cast::<u8>().add(8).cast::<usize>().read()};
            if state!=self.pointer{return Err("AppDelegate current state differs from held parent".into());}
            self.root.verify_held_epoch()?;
            Ok(self.pointer)
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    fn verify_class(&self)->Result<()> {
        let class=metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        if self.root.declaration.data.main_base.checked_add(0x3ec968)!=Some(class.address())
            ||unsafe{&*(self.pointer as *const AnyObject)}.class() as *const AnyClass as usize!=class.address() {
            return Err("Actual getter returned another AppState class".into());
        }
        let observed=self.root.declaration.data.context.observe_initialized_app_state_destroyer()?;
        if observed.class_metadata()!=class.address()
            ||self.root.declaration.data.main_base.checked_add(0x66478)!=Some(observed.destroyer())
            ||observed.instance_size()!=unsafe{&*(class.address() as *const AnyClass)}.instance_size() {
            return Err("Held AppState differs from its completed native-destructor metadata".into());
        }
        Ok(())
    }
}
impl Drop for AppStateCustody {
    fn drop(&mut self) {
        // Intentionally no swift_release/objc_release: this known getter +1
        // remains quarantined. Neither an unknown outcome nor final Swift ARC
        // can be disposed by a data/lifetime-only contribution.
    }
}
impl InitializedAppDelegateRoot {
    /// Obtain an actual extra +1 through the finite generated Swift getter.
    /// No unknown pointer is retained, no Swift body is cast to an externC fn.
    /// One attempt per existing root, including None/error outcomes. Call only
    /// after the real did-finish initialized its state; no automatic retry.
    pub fn capture_app_state(self:&Arc<Self>)->Result<Option<Arc<AppStateCustody>>> {
        self.declaration.data.context.cleanup_metadata_only()?; // fork/creator before claim/Err
        if self.state_capture_claimed.swap(true,Ordering::AcqRel) {
            self.state_capture_retired.store(true,Ordering::Release);
            return Err("AppState getter capture was already claimed".into());
        }
        struct Claim<'a>{root:&'a InitializedAppDelegateRoot,complete:bool}
        impl Drop for Claim<'_>{fn drop(&mut self){if !self.complete {self.root.state_capture_retired.store(true,Ordering::Release);}}}
        let mut claim=Claim{root:self,complete:false};
        // Never reset on None, Err, foreign outcome or a swallowed nested refusal.
        let plan=StateGetterPlan::observe(self)?;
        let parent=self.pointer_on_creator()?;
        let entry=self.declaration.data.main_base.checked_add(0x1fb92c).ok_or("AppState getter entry overflow")?;
        // Requalify AFTER the last dynamic parent guard and before the effect.
        plan.verify(self)?;
        let pointer=unsafe{swift_self_adapter(parent as usize,entry)};
        if pointer==0 {plan.verify(self)?;return Ok(None);}
        // Immediately retain the effect in an opaque quarantine-on-Drop value,
        // before any fallible callback, metadata check or foreign class read.
        let state=Arc::new(AppStateCustody{child_capture_claimed:[AtomicBool::new(false),AtomicBool::new(false)],child_capture_retired:[AtomicBool::new(false),AtomicBool::new(false)],root:self.clone(),pointer,plan,retired:AtomicBool::new(false)});
        state.pointer_on_creator()?;
        claim.complete=true;
        Ok(Some(state))
    }
}

// Proposed class-local extension. No callback or userdata-read permission.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum ReferenceInputChildKind {Hotkeys,SnippetExpander,HyperKey}
impl ReferenceInputChildKind {
    fn index(self)->usize {match self {Self::Hotkeys|Self::HyperKey=>0,Self::SnippetExpander=>1}}
    fn class(self)->(&'static CStr,usize) {match self {
        Self::Hotkeys=>(c"RuycastF1Reference.Hotkeys",0x3efd90),
        Self::SnippetExpander=>(c"RuycastF1Reference.SnippetExpander",0x3f4848),
        Self::HyperKey=>(c"RuycastF1Reference.HyperKey",0x3f0078),
    }}
    fn getter(self)->Result<(usize,&'static str)> {match self {
        Self::Hotkeys=>Ok((0x46064,"197ab8df025e6be335f43f80c8301a5a066d6137c55ad0eb11f19bab4dac7bd9")),
        Self::SnippetExpander=>Ok((0x460ac,"864b0a192da52f911695c272e4571afd54e9e92281a8d1816653e3be58a42efe")),
        Self::HyperKey=>Err("HyperKey is an immutable keyboard projection, not an AppState getter".into()),
    }}
}
fn bounded_swift_field(class:metadata::ClassHandle,offset:u64)->Result<usize> {
    let size=unsafe{&*(class.address() as *const AnyClass)}.instance_size();
    let offset=usize::try_from(offset).map_err(|_|"Swift child offset exceeds address width")?;
    // Runtime class size is the bound, never DWARF's eight-byte reference form.
    if size<24 || size>16*1024*1024 || offset<16 || !offset.is_multiple_of(8)
        ||offset.checked_add(8).is_none_or(|end|end>size) {
        return Err("Swift child field lies outside its actual class instance".into());
    }
    Ok(offset)
}
impl AppStateCustody {
    fn verify_current_child_ivars(&self,observed:&crate::sdk::core_preentry::AppStateChildOffsetCellsObservation)->Result<[usize;2]> {
        let data=&self.root.declaration.data;
        if observed.main_header()!=data.main_base{return Err("Typed child observation belongs to another Main".into());}
        let class=metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        if data.main_base.checked_add(0x3ec968)!=Some(class.address()){return Err("AppState metadata identity changed".into());}
        let fields=unsafe{&*(class.address() as *const AnyClass)}.instance_variables();
        if fields.len()!=53{return Err("AppState actual field roster differs".into());}
        let mut offsets=[0usize;2];
        for (name,slot) in [(c"hotkeys",0usize),(c"expander",1usize)] {
            offsets[slot]=bounded_swift_field(class,observed.cells()[slot].1)?;
            let matched=fields.iter().filter(|field|field.name()==name).collect::<Vec<_>>();
            if matched.len()!=1 || usize::try_from(matched[0].offset()).ok()!=Some(offsets[slot]) {
                return Err("Typed full-width vector/global does not match current named ivar".into());
            }
        }
        Ok(offsets)
    }
    fn child_location(&self,kind:ReferenceInputChildKind)->Result<(usize,usize,crate::sdk::core_preentry::AppStateChildOffsetCellsObservation)> {
        let pointer=self.pointer_on_creator()?; // last model/Source callback
        let data=&self.root.declaration.data;
        let class=metadata::loaded_class(c"RuycastF1Reference.AppState")?;
        if data.main_base.checked_add(0x3ec968)!=Some(class.address())
            ||unsafe{&*(pointer as *const AnyObject)}.class() as *const AnyClass as usize!=class.address() {
            return Err("Swift child parent is not the same AppState class".into());
        }
        let observed=data.context.observe_app_state_child_offset_cells()?;
        if observed.main_header()!=data.main_base{return Err("Swift child field observer has another Main".into());}
        for ((address,_),offset) in observed.cells().iter().zip([0x40c908,0x40c910]) {
            if data.main_base.checked_add(offset)!=Some(*address){return Err("Swift child field cell identity differs".into());}
        }
        let index=match kind {ReferenceInputChildKind::Hotkeys=>0,ReferenceInputChildKind::SnippetExpander=>1,
            ReferenceInputChildKind::HyperKey=>return Err("HyperKey has no AppState field".into())};
        // Typed RO record index + full64 metadata vector/global must also
        // agree with the current actual named ivar before the getter can retain.
        let offset=self.verify_current_child_ivars(&observed)?[index];
        let after=data.context.observe_app_state_child_offset_cells()?;
        if after!=observed || bounded_swift_field(class,after.cells()[index].1)?!=offset {
            return Err("Actual Swift child layout changed during qualification".into());
        }
        self.verify_held_epoch()?;
        Ok((pointer,offset,after))
    }
    /// One genuine getter attempt per child kind. Returned objects are opaque
    /// physical anchors; Source/callback/registration/typed userdata remain separate.
    pub fn capture_input_child(self:&Arc<Self>,kind:ReferenceInputChildKind)->Result<Option<Arc<ReferenceInputChildCustody>>> {
        self.root.declaration.data.context.cleanup_metadata_only()?;
        let (entry_offset,sha)=kind.getter()?;
        let index=kind.index();
        if self.child_capture_claimed[index].swap(true,Ordering::AcqRel) {
            self.child_capture_retired[index].store(true,Ordering::Release);
            return Err("Swift input child capture was already claimed".into());
        }
        struct Claim<'a>{parent:&'a AppStateCustody,index:usize,complete:bool}
        impl Drop for Claim<'_>{fn drop(&mut self){if !self.complete {self.parent.child_capture_retired[self.index].store(true,Ordering::Release);}}}
        let mut claim=Claim{parent:self,index,complete:false};
        let (parent,offset,lineage)=self.child_location(kind)?;
        let data=&self.root.declaration.data;
        let entry=data.main_base.checked_add(entry_offset).ok_or("Swift child getter entry overflow")?;
        data.context.verify_main_code(entry,12,sha)?;
        // Last data re-observation follows every dynamic/code/source guard.
        // This observer now proves exact typed RO descriptor/record/fixup,
        // loaded description, and full64 metadata vector == getter globals.
        let before_effect=data.context.observe_app_state_child_offset_cells()?;
        if before_effect!=lineage || self.verify_current_child_ivars(&before_effect)?[index]!=offset {
            return Err("Typed child layout changed before getter retain".into());
        }
        self.verify_held_epoch()?;
        if self.child_capture_retired[index].load(Ordering::Acquire){return Err("Swift child capture retired before effect".into());}
        let pointer=unsafe{swift_self_adapter(parent,entry)};
        if pointer==0{return Ok(None);}
        // Actual finite getter already performed the child swift_retain.
        let held=Arc::new(ReferenceInputChildCustody{parent:self.clone(),hotkeys:None,kind,pointer,retired:AtomicBool::new(false),
            stopped_poll:std::sync::atomic::AtomicUsize::new(0)});
        held.pointer_on_creator()?;
        // Recheck actual slot, not a previously read word, after dynamic work.
        let (parent,after_offset,_)=self.child_location(kind)?;
        if after_offset!=offset || unsafe{((parent+offset) as *const usize).read()}!=pointer {
            return Err("Swift child getter no longer matches its parent field".into());
        }
        held.verify_held_epoch()?;
        claim.complete=true;
        Ok(Some(held))
    }
}
/// A real getter extra +1, or HyperKey strongly borrowed from an immutable
/// private-let field of that retained Hotkeys parent. Drop quarantines; this
/// grants no input callback, userdata dereference or final Swift ARC permission.
pub struct ReferenceInputChildCustody {
    parent:Arc<AppStateCustody>,
    hotkeys:Option<Arc<ReferenceInputChildCustody>>,
    kind:ReferenceInputChildKind,
    pointer:usize,
    retired:AtomicBool,
    stopped_poll:std::sync::atomic::AtomicUsize,
}
impl ReferenceInputChildCustody {
    pub fn kind(&self)->ReferenceInputChildKind {self.kind}
    pub fn verify_held_epoch(&self)->Result<()> {
        self.parent.verify_held_epoch()?;
        if self.retired.load(Ordering::Acquire)
            ||self.parent.child_capture_retired[self.kind.index()].load(Ordering::Acquire) {
            return Err("Swift input child binding retired".into());
        }
        if let Some(hotkeys)=&self.hotkeys {hotkeys.verify_held_epoch()?;}
        self.parent.verify_held_epoch()
    }
    fn verify_class(&self)->Result<()> {
        let (name,offset)=self.kind.class();let data=&self.parent.root.declaration.data;
        let class=metadata::loaded_class(name)?;
        if data.main_base.checked_add(offset)!=Some(class.address())
            ||unsafe{&*(self.pointer as *const AnyObject)}.class() as *const AnyClass as usize!=class.address() {
            return Err("Actual Swift input child class differs".into());
        }
        Ok(())
    }
    pub fn pointer_on_creator(&self)->Result<usize> {
        self.parent.root.declaration.data.context.cleanup_metadata_only()?;
        self.verify_held_epoch()?;
        let result=(|| {
            let current=if self.kind==ReferenceInputChildKind::HyperKey {
                let hotkeys=self.hotkeys.as_ref().ok_or("HyperKey has no retained Hotkeys parent")?;
                hotkeys.keyboard_location()?.1
            } else {
                let (parent,offset,_)=self.parent.child_location(self.kind)?;
                unsafe{((parent+offset) as *const usize).read()}
            };
            if current!=self.pointer{return Err("Swift input userdata no longer matches its typed parent".into());}
            self.verify_class()?;
            self.verify_held_epoch()?;
            Ok(self.pointer)
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    pub fn verify_reference_binding(&self,admission:&Arc<crate::sdk::core_preentry::ReferenceAdmission>,model:&Arc<Workspace>)->Result<()> {
        self.parent.root.verify_held_epoch()?;
        let result=(|| {
            self.verify_held_epoch()?;
            self.parent.verify_reference_binding(admission,model)?;
            self.pointer_on_creator()?;
            self.parent.verify_reference_binding(admission,model)?;
            self.pointer_on_creator()?;
            self.verify_held_epoch()
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    pub(crate) fn carbon_handler_on_creator(&self) -> Result<usize> {
        if self.kind != ReferenceInputChildKind::Hotkeys {
            return Err("Reference Carbon handler requires Hotkeys custody".into());
        }
        self.input_registration_on_creator(c"handler", 48)
    }

    pub(crate) fn detach_stopped_snippet_expander(
        &self,
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        model: &Arc<Workspace>,
    ) -> Result<()> {
        if self.kind != ReferenceInputChildKind::SnippetExpander
            || self.stopped_poll.load(Ordering::Acquire) <= 1
        {
            return Err("Reference getter release requires stopped SnippetExpander custody".into());
        }
        crate::sdk::reference_runtime::verify_terminal_input_cleanup(admission, model)?;
        let data = &self.parent.root.declaration.data;
        let at = |offset| data.main_base.checked_add(offset).ok_or("Reference SnippetExpander address overflow");
        for (offset, length, sha) in [
            (0x460e0, 12, "9283de7890bccdf0ef09280d47c536bbc67ccf0e4b60d119f7cc3400e49cebc9"),
            (0x460ec, 52, "202ef64ac4cd4c27717e3f17ea44b55c64b3dbba30a7668c20cdaa8e5341d141"),
            (0x70434, 20, "28d207682bd5a60617b3c2d205208c574f74b3423a7a3d1707a865b95dc3e984"),
            (0x70480, 16, "7f9e268be60aec56345ee4b093ddab0a34dd64148d4cb3c43bae517a9176d86e"),
            (0x1ae058, 0xdc, "0422d794cccf39abe274d1e64741ad6371d3433c9cf1f28ae5d1d01a72586ab7"),
            (0x1ae134, 0xd4, "b06dec910adae7c4c578958754f62c35d5220dde16d2ce5aa8b7e4d4e2431bcc"),
            (0x1ae208, 0x2c, "67d3c655834c760251f8281948971bfbe6b5ce64d94592b195c5f5d62ef90f5a"),
            (0x1ae234, 0x314, "446884b2fcbc222b0beb90ba0b1e76b57d5f04fbdb348af47a05ca8f09ebb1eb"),
            (0x2fc248, 12, "3090070a4519c8365a820a7973df9b9ee270cfed2e5c739cf06cdeac3e6a015f"),
        ] {
            data.context.verify_main_code(at(offset)?, length, sha)?;
        }
        let (parent, offset, lineage) = self.parent.child_location(self.kind)?;
        let offset_cell = at(0x40c910)? as *const usize;
        if lineage.cells()[1] != (offset_cell as usize, offset as u64) {
            return Err("Reference SnippetExpander setter offset cell differs".into());
        }
        let slot = (parent + offset) as *const usize;
        let poll = self.input_registration_address(self.pointer, c"poll", 64)?;
        let resources = [
            self.input_registration_address(self.pointer, c"tap", 24)?,
            self.input_registration_address(self.pointer, c"source", 32)?,
            self.input_registration_address(self.pointer, c"choice", 72)?,
        ];
        self.invalidate_input_poll_timer()?;
        let stopped_poll = self.stopped_poll.load(Ordering::Acquire);
        let timer = metadata::loaded_class(c"__NSCFTimer")?;
        let invalidate = metadata::method(timer, c"invalidate", false)?
            .ok_or("Reference SnippetExpander timer invalidate absent")?;
        if invalidate.encoding() != b"v16@0:8" || invalidate.code().offset() != 0xc78a0
            || invalidate.code().image() != b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
            || invalidate.code().uuid() != [0x9b, 0x67, 0x27, 0x62, 0x7b, 0x1f, 0x30, 0xbc,
                0x96, 0xde, 0xf1, 0x76, 0xb3, 0x72, 0xd6, 0x6d]
        {
            return Err("Reference SnippetExpander timer invalidate differs".into());
        }
        let deinit = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"swift_task_deinitOnExecutor".as_ptr()) } as usize;
        // On this pinned runtime, a current executor invokes the original
        // deinit body inline. A cleared strong slot alone cannot prove that.
        for (address, offset) in [
            (deinit, 0x5668),
            (MAIN_EXECUTOR_LABEL as *const () as usize, 0x8f9c),
            (EXECUTOR_CHECK_LABEL as *const () as usize, 0x4f88),
        ] {
            let code = metadata::code_identity(address)?;
            if code.image() != b"/usr/lib/swift/libswift_Concurrency.dylib"
                || code.offset() != offset
                || code.uuid() != [0x8e, 0x16, 0x88, 0x57, 0x47, 0xf4, 0x34, 0x9f,
                    0xa7, 0x18, 0xa1, 0x8d, 0xb1, 0x44, 0xfc, 0xb0]
            {
                return Err("Reference SnippetExpander isolated deinit executor differs".into());
            }
        }
        let rows = crate::sdk::imports::observe_unpatched_main_imports(&[(c"swift_task_deinitOnExecutor", deinit)])?;
        let [row] = rows.as_slice() else { return Err("Reference SnippetExpander isolated deinit import absent".into()); };
        if row.cell_address() != at(0x3c7840)? {
            return Err("Reference SnippetExpander isolated deinit import differs".into());
        }
        let unique = UNIQUE_ACTOR_LABEL as *const () as usize;
        if metadata::code_identity(unique)?.image() != b"/usr/lib/swift/libswiftCore.dylib" {
            return Err("Reference SnippetExpander uniqueness issuer differs".into());
        }
        let unique: unsafe extern "C" fn(usize) -> bool = unsafe { std::mem::transmute(unique) };
        self.release_input_getter_reference()?;
        row.verify_unpatched()?;
        data.context.verify_snippet_expander_deinit_cell()?;
        verify_original_main_executor(data)?;
        metadata::verify_current_method(timer, c"invalidate", false, &invalidate)?;
        self.parent.verify_held_epoch()?;
        row.verify_unpatched()?;
        if unsafe { offset_cell.read_volatile() } != offset
            || unsafe { slot.read_volatile() } != self.pointer || unsafe { poll.read_volatile() } != stopped_poll
            || resources.into_iter().any(|address| unsafe { address.read_volatile() } != 0)
            || !unsafe { unique(self.pointer) }
        {
            return Err("Reference SnippetExpander is not solely owned by AppState with its stopped timer".into());
        }
        unsafe { swift_clear_optional_child(parent, at(0x460e0)?) };
        if unsafe { slot.read_volatile() } != 0 {
            return Err("Original AppState setter did not clear SnippetExpander".into());
        }
        self.parent.verify_held_epoch()
    }

    /// Discharge the getter's extra retain while AppState's strong field
    /// still owns the original child.
    fn release_input_getter_reference(&self) -> Result<()> {
        let data = &self.parent.root.declaration.data;
        let (import, code) = data.system.code.iter().find(|(row, _)| row.symbol == c"swift_release")
            .ok_or("Reference input child release import absent")?;
        let release = RELEASE_ACTOR_LABEL as *const () as usize;
        if data.main_base.checked_add(0x3c7438) != Some(import.cell_address())
            || import.original_address() != release
            || code.image() != b"/usr/lib/swift/libswiftCore.dylib"
        {
            return Err("Reference input child release issuer differs".into());
        }
        data.context.verify_main_code(
            data.main_base.checked_add(0x2fc1ac).ok_or("Reference input child release stub overflow")?,
            12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797",
        )?;
        let (parent, offset, _) = self.parent.child_location(self.kind)?;
        let snippet_resources = if self.kind == ReferenceInputChildKind::SnippetExpander {
            Some([
                self.input_registration_address(self.pointer, c"tap", 24)?,
                self.input_registration_address(self.pointer, c"source", 32)?,
                self.input_registration_address(self.pointer, c"choice", 72)?,
            ])
        } else {
            None
        };
        self.verify_held_epoch()?;
        self.verify_class()?;
        import.verify_unpatched()?;
        if metadata::code_identity(release)? != *code
            || unsafe { ((parent + offset) as *const usize).read_volatile() } != self.pointer
        {
            return Err("Reference input child strong owner or release provider changed".into());
        }
        if snippet_resources.is_some_and(|slots| slots.into_iter().any(|slot| unsafe { slot.read_volatile() } != 0)) {
            return Err("Reference SnippetExpander resources reappeared before getter release".into());
        }
        if self.retired.swap(true, Ordering::AcqRel) {
            return Err("Reference input child getter release replay".into());
        }
        let release: unsafe extern "C" fn(usize) = unsafe { std::mem::transmute(release) };
        unsafe { release(self.pointer) };
        Ok(())
    }

    pub(crate) fn detach_hotkeys_after_stop(&self, keyboard: &Self) -> Result<()> {
        if self.kind != ReferenceInputChildKind::Hotkeys
            || keyboard.kind != ReferenceInputChildKind::HyperKey
            || keyboard.stopped_poll.load(Ordering::Acquire) <= 1
            || !keyboard.hotkeys.as_ref().is_some_and(|owner| std::ptr::eq(owner.as_ref(), self))
        {
            return Err("Reference Hotkeys setter requires its original stopped keyboard".into());
        }
        let data = &self.parent.root.declaration.data;
        let at = |offset| data.main_base.checked_add(offset).ok_or("Reference Hotkeys setter address overflow");
        for (offset, length, sha) in [
            (0x46070, 12, "c585a42485c4102cf0f42186cbaad702b1f4cf0401e6d81df082bd11592e4342"),
            (0x460ec, 52, "202ef64ac4cd4c27717e3f17ea44b55c64b3dbba30a7668c20cdaa8e5341d141"),
            (0x70434, 20, "28d207682bd5a60617b3c2d205208c574f74b3423a7a3d1707a865b95dc3e984"),
            (0x70480, 16, "7f9e268be60aec56345ee4b093ddab0a34dd64148d4cb3c43bae517a9176d86e"),
            (0xf5c38, 396, "0b72de9847f64bf20e1379382f6d4601c3445420f0f3318213c7541c47571121"),
            (0xf5dc4, 44, "64268455c9e9b2eb2b7e43c2bb01ece1bac0d6275d1439dfc1fcee6b0a0dab61"),
            (0xf5df0, 740, "26f65be02eebec67c56221ccb3b853d105a247ddafb222b3bfffc1f4c021fc9c"),
            (0xf881c, 532, "55c328c5d93ca68c1b091da50ad90c344220df5a2c9e0f4d08839d9b5f6d2a9f"),
            (0xf8a30, 44, "0acab176b5a34137d4a148cef6a52e9b44c957f2572f8171bfd4dd22ded26f80"),
            (0xf8a5c, 792, "9525e32434dcae14e64f148b534b5525acd33ba43c4ca4d30e631cfb7e59a969"),
            (0x2fc248, 12, "3090070a4519c8365a820a7973df9b9ee270cfed2e5c739cf06cdeac3e6a015f"),
        ] {
            data.context.verify_main_code(at(offset)?, length, sha)?;
        }
        // AppState custody also revalidates the original swift_beginAccess
        // slot used by the setter's outlined access helper.
        let (parent, offset, lineage) = self.parent.child_location(self.kind)?;
        let offset_cell = at(0x40c908)? as *const usize;
        if lineage.cells()[0] != (offset_cell as usize, offset as u64) {
            return Err("Reference Hotkeys setter offset cell differs".into());
        }
        let slot = (parent + offset) as *const usize;
        let (owner, private_keyboard) = self.keyboard_location()?;
        if owner != self.pointer || private_keyboard != keyboard.pointer_on_creator()? {
            return Err("Reference Hotkeys setter owner changed".into());
        }
        verify_original_main_executor(data)?;
        data.context.verify_input_deinit_cells()?;
        let deinit = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"swift_task_deinitOnExecutor".as_ptr()) } as usize;
        if deinit == 0 || metadata::code_identity(deinit)?.image() != b"/usr/lib/swift/libswift_Concurrency.dylib" {
            return Err("Reference Swift isolated deinit issuer differs".into());
        }
        let rows = crate::sdk::imports::observe_unpatched_main_imports(&[(c"swift_task_deinitOnExecutor", deinit)])?;
        let row = rows.first().ok_or("Reference Swift isolated deinit import absent")?;
        if row.cell_address() != at(0x3c7840)? {
            return Err("Reference Swift isolated deinit import differs".into());
        }
        row.verify_unpatched()?;
        let unique = UNIQUE_ACTOR_LABEL as *const () as usize;
        if metadata::code_identity(unique)?.image() != b"/usr/lib/swift/libswiftCore.dylib" {
            return Err("Reference Swift uniqueness issuer differs".into());
        }
        let unique: unsafe extern "C" fn(usize) -> bool = unsafe { std::mem::transmute(unique) };
        self.verify_held_epoch()?;
        keyboard.verify_held_epoch()?;
        self.release_input_getter_reference()?;
        // The child object is live only through the original strong slot.
        // The released custody now refuses access through both child handles.
        row.verify_unpatched()?;
        data.context.verify_input_deinit_cells()?;
        verify_original_main_executor(data)?;
        let setter = at(0x46070)?;
        let keyboard_slot = self.pointer.checked_add(56).ok_or("Reference Hotkeys keyboard slot overflow")? as *const usize;
        crate::workspace::objc::notification_bridge::drain_hyper_observers(at(0xf8980)?, private_keyboard, || {
            row.verify_unpatched()?;
            self.parent.verify_held_epoch()?;
            if unsafe { offset_cell.read_volatile() } != offset
                || unsafe { slot.read_volatile() } != self.pointer
                || unsafe { keyboard_slot.read_volatile() } != private_keyboard
            {
                return Err("Reference Hotkeys setter fields changed before release".into());
            }
            if !unsafe { unique(self.pointer) } || !unsafe { unique(private_keyboard) } {
                return Err("Reference Hotkeys or HyperKey has another strong owner".into());
            }
            unsafe { swift_clear_optional_child(parent, setter) };
            Ok(())
        })?;
        if unsafe { slot.read_volatile() } != 0 {
            return Err("Original AppState setter did not clear Hotkeys".into());
        }
        self.parent.verify_held_epoch()
    }

    pub(crate) fn hid_manager_on_creator(&self) -> Result<usize> {
        let manager = self.hid_manager_slot_on_creator()?;
        if manager == 0 {
            return Err("Reference HID manager absent".into());
        }
        Ok(manager)
    }

    pub(crate) fn hid_manager_slot_on_creator(&self) -> Result<usize> {
        if self.kind != ReferenceInputChildKind::HyperKey {
            return Err("Reference HID manager requires HyperKey custody".into());
        }
        let pointer = self.pointer_on_creator()?;
        let slot = self.input_registration_address(pointer, c"manager", 16)?;
        self.verify_held_epoch()?;
        Ok(unsafe { slot.read_volatile() })
    }

    pub(crate) fn event_tap_on_creator(&self) -> Result<(usize, usize)> {
        let (tap, source) = match self.kind {
            ReferenceInputChildKind::HyperKey => (32, 40),
            ReferenceInputChildKind::SnippetExpander => (24, 32),
            ReferenceInputChildKind::Hotkeys => return Err("Reference Hotkeys does not own an event tap".into()),
        };
        let pointer = self.pointer_on_creator()?;
        let tap = self.input_registration_address(pointer, c"tap", tap)?;
        let source = self.input_registration_address(pointer, c"source", source)?;
        self.verify_held_epoch()?;
        let (tap, source) = unsafe { (tap.read_volatile(), source.read_volatile()) };
        if tap == 0 || source == 0 {
            return Err("Reference input tap or source absent".into());
        }
        Ok((tap, source))
    }

    pub(crate) fn verify_keyboard_tap_cleared(&self) -> Result<()> {
        if self.kind != ReferenceInputChildKind::HyperKey {
            return Err("Reference interrupted tap requires HyperKey custody".into());
        }
        let pointer = self.pointer_on_creator()?;
        let tap = self.input_registration_address(pointer, c"tap", 32)?;
        let source = self.input_registration_address(pointer, c"source", 40)?;
        self.verify_held_epoch()?;
        if unsafe { tap.read_volatile() != 0 || source.read_volatile() != 0 } {
            return Err("Reference HyperKey did not clear its interrupted tap".into());
        }
        Ok(())
    }

    pub(crate) fn verify_hyper_resources_cleared(&self) -> Result<()> {
        if self.kind != ReferenceInputChildKind::HyperKey {
            return Err("Reference stopped resources require HyperKey custody".into());
        }
        let pointer = self.pointer_on_creator()?;
        let slots = [
            self.input_registration_address(pointer, c"manager", 16)?,
            self.input_registration_address(pointer, c"tap", 32)?,
            self.input_registration_address(pointer, c"source", 40)?,
        ];
        let caps = self.caps_connection_on_creator()?;
        self.verify_held_epoch()?;
        if caps != 0 || slots.into_iter().any(|slot| unsafe { slot.read_volatile() } != 0) {
            return Err("Reference HyperKey did not clear its stopped resources".into());
        }
        Ok(())
    }

    fn input_registration_on_creator(&self, field: &CStr, field_offset: u64) -> Result<usize> {
        let pointer = self.pointer_on_creator()?;
        let address = self.input_registration_address(pointer, field, field_offset)?;
        self.verify_held_epoch()?;
        let registration = unsafe { address.read_volatile() };
        if registration == 0 {
            return Err(format!("Reference input registration {field:?} absent"));
        }
        Ok(registration)
    }

    fn input_registration_address(&self, pointer: usize, field: &CStr, field_offset: u64) -> Result<*const usize> {
        let (name, _) = self.kind.class();
        let class = metadata::loaded_class(name)?;
        let offset = bounded_swift_field(class, field_offset)?;
        let fields = unsafe { &*(class.address() as *const AnyClass) }.instance_variables();
        let mut matching = fields.iter().filter(|ivar| ivar.name() == field);
        if matching.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(offset)
            || matching.next().is_some()
        {
            return Err(format!("Reference input registration ivar {field:?} differs"));
        }
        pointer.checked_add(offset).map(|address| address as *const usize)
            .ok_or_else(|| "Reference input registration address overflow".into())
    }

    pub(crate) fn caps_connection_on_creator(&self) -> Result<u32> {
        if self.kind != ReferenceInputChildKind::HyperKey {
            return Err("Reference Caps connection requires HyperKey custody".into());
        }
        let pointer = self.pointer_on_creator()?;
        let data = &self.parent.root.declaration.data;
        let (name, image_offset) = self.kind.class();
        let class = metadata::loaded_class(name)?;
        let offset = 24;
        if data.main_base.checked_add(image_offset) != Some(class.address()) {
            return Err("Reference Caps connection class differs".into());
        }
        let class = unsafe { &*(class.address() as *const AnyClass) };
        let fields = class.instance_variables();
        let mut matching = fields.iter().filter(|field| field.name() == c"capsConnection");
        if matching.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(offset)
            || matching.next().is_some() || class.instance_size() < offset + std::mem::size_of::<u32>()
        {
            return Err("Reference Caps connection ivar differs".into());
        }
        self.verify_held_epoch()?;
        let address = pointer.checked_add(offset).ok_or("Reference Caps connection address overflow")?;
        Ok(unsafe { (address as *const u32).read_volatile() })
    }

    pub(crate) fn stop_original_hyper(
        &self, admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>, model: &Arc<Workspace>,
    ) -> Result<()> {
        self.stop_original_hyper_inner(admission, model, false)
    }

    pub(crate) fn rollback_hyper_without_tap(
        &self, admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>, model: &Arc<Workspace>,
    ) -> Result<bool> {
        use crate::sdk::{input_events, input_hid, input_permissions_caps};
        let pointer = self.pointer_on_creator()?;
        if self.caps_connection_on_creator()? == 0
            || unsafe { self.input_registration_address(pointer, c"tap", 32)?.read_volatile() } != 0
        {
            return Ok(false);
        }
        self.verify_reference_binding(admission, model)?;
        if unsafe { self.input_registration_address(pointer, c"manager", 16)?.read_volatile() } != 0
            || unsafe { self.input_registration_address(pointer, c"source", 40)?.read_volatile() } != 0
        {
            return Err("Reference HyperKey partial startup has unexpected resources".into());
        }
        input_events::verify_reference_keyboard_taps_stopped(self, pointer, true)?;
        input_hid::verify_reference_manager_stopped(self, true)?;
        input_permissions_caps::verify_reference_startup_connection(admission, model, self)?;
        self.invalidate_input_poll_timer()?;
        self.stop_original_hyper_inner(admission, model, true)?;
        Ok(true)
    }

    fn stop_original_hyper_inner(
        &self, admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>, model: &Arc<Workspace>,
        before_publication: bool,
    ) -> Result<()> {
        use crate::sdk::{input_events, input_hid, input_permissions_caps};
        if self.kind != ReferenceInputChildKind::HyperKey || self.stopped_poll.load(Ordering::Acquire) <= 1 {
            return Err("Reference HyperKey requires its stopped poll before original stop".into());
        }
        self.verify_reference_binding(admission, model)?;
        let data = &self.parent.root.declaration.data;
        let pointer = self.pointer_on_creator()?;
        let manager_slot = self.input_registration_address(pointer, c"manager", 16)?;
        let tap_slot = self.input_registration_address(pointer, c"tap", 32)?;
        let source_slot = self.input_registration_address(pointer, c"source", 40)?;
        let slots = || unsafe { (manager_slot.read_volatile(), tap_slot.read_volatile(), source_slot.read_volatile()) };
        let (manager, tap, source) = slots();
        let caps = self.caps_connection_on_creator()?;
        if before_publication && ((manager, tap, source) != (0, 0, 0) || caps == 0) {
            return Err("Reference HyperKey partial startup differs".into());
        }
        if (manager, tap, source, caps) == (0, 0, 0, 0) {
            input_events::verify_reference_keyboard_taps_stopped(self, pointer, false)?;
            input_hid::verify_reference_manager_stopped(self, false)?;
            return input_permissions_caps::verify_reference_closed_caps_connection(admission, model, None);
        }
        input_permissions_caps::verify_reference_startup_connection(admission, model, self)?;
        let manager_stopped = (!before_publication)
            .then(|| input_hid::observe_reference_manager_stop(manager, pointer)).transpose()?;
        let tap_stopped = (!before_publication)
            .then(|| input_events::observe_reference_tap_stop(tap, source, pointer)).transpose()?;
        admission.before_reference_publication()?;
        if self.pointer_on_creator()? != pointer {
            return Err("Reference HyperKey owner changed before original stop".into());
        }
        data.context.verify_original_compiler_text()?;
        let entry = data.main_base.checked_add(0xf7e0c).ok_or("Reference HyperKey stop entry overflow")?;
        data.context.verify_main_code(entry, 0x1dc,
            "946e9d0defdca00a3f63597635fa3f8592c438d7c2ded4dd2a80614d4025a4e5")?;
        verify_original_main_executor(data)?;
        self.verify_held_epoch()?;
        if before_publication {
            input_events::verify_reference_keyboard_taps_stopped(self, pointer, true)?;
            input_hid::verify_reference_manager_stopped(self, true)?;
        }
        if slots() != (manager, tap, source)
            || unsafe { ((pointer + 24) as *const u32).read_volatile() } != caps
        {
            return Err("Reference HyperKey resources changed before original stop".into());
        }
        unsafe { swift_self_adapter(pointer, entry); }
        if self.pointer_on_creator()? != pointer {
            return Err("Reference HyperKey owner changed during original stop".into());
        }
        self.verify_held_epoch()?;
        if slots() != (0, 0, 0) || unsafe { ((pointer + 24) as *const u32).read_volatile() } != 0 {
            return Err("Original HyperKey stop did not clear its resource fields".into());
        }
        let hid = match manager_stopped {
            Some(stopped) => stopped(),
            None => input_hid::verify_reference_manager_stopped(self, true),
        };
        let cg = match tap_stopped {
            Some(stopped) => stopped(),
            None => input_events::verify_reference_keyboard_taps_stopped(self, pointer, true),
        };
        let caps = input_permissions_caps::verify_reference_closed_caps_connection(admission, model, Some(caps));
        hid.and(cg).and(caps)
    }

    pub(crate) fn stop_original_snippet_expander(
        &self,
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        model: &Arc<Workspace>,
    ) -> Result<()> {
        if self.kind != ReferenceInputChildKind::SnippetExpander || self.stopped_poll.load(Ordering::Acquire) <= 1 {
            return Err("Reference SnippetExpander requires its stopped poll before original stop".into());
        }
        self.verify_reference_binding(admission, model)?;
        let data = &self.parent.root.declaration.data;
        let pointer = self.pointer_on_creator()?;
        let tap_slot = self.input_registration_address(pointer, c"tap", 24)?;
        let source_slot = self.input_registration_address(pointer, c"source", 32)?;
        let choice_slot = self.input_registration_address(pointer, c"choice", 72)?;
        let slots = || unsafe {
            (tap_slot.read_volatile(), source_slot.read_volatile(), choice_slot.read_volatile())
        };
        let (tap, source, choice) = slots();
        let tap_stopped = if (tap, source) == (0, 0) {
            None
        } else {
            Some(crate::sdk::input_events::observe_reference_tap_stop(tap, source, pointer)?)
        };
        admission.before_reference_publication()?;
        if self.pointer_on_creator()? != pointer {
            return Err("Reference SnippetExpander owner changed before original stop".into());
        }
        data.context.verify_original_compiler_text()?;
        let entry = data.main_base.checked_add(0x1ae134).ok_or("Reference SnippetExpander stop entry overflow")?;
        data.context.verify_main_code(entry, 0xd4,
            "b06dec910adae7c4c578958754f62c35d5220dde16d2ce5aa8b7e4d4e2431bcc")?;
        verify_original_main_executor(data)?;
        self.verify_held_epoch()?;
        if slots() != (tap, source, choice) {
            return Err("Reference SnippetExpander resources changed before original stop".into());
        }
        let choice_drained = if choice != 0 {
            crate::sdk::reference_runtime::verify_terminal_input_cleanup(admission, model)?;
            data.context.verify_snippet_choice_deinit()?;
            self.verify_current_snippet_choice(admission, model, choice)?;
            let (notifications, click) = unsafe { data.context.snippet_choice_notification_slots(choice)? };
            let drained = crate::workspace::objc::notification_bridge::verify_snippet_choice_observers(model, notifications)?;
            crate::workspace::objc::verify_reference_snippet_monitor(admission, choice, click)?;
            self.verify_current_snippet_choice(admission, model, choice)?;
            if unsafe { data.context.snippet_choice_notification_slots(choice)? } != (notifications, click) {
                return Err("Reference SnippetChoice observers changed during qualification".into());
            }
            let unique = UNIQUE_ACTOR_LABEL as *const () as usize;
            if metadata::code_identity(unique)?.image() != b"/usr/lib/swift/libswiftCore.dylib" {
                return Err("Reference SnippetChoice uniqueness issuer differs".into());
            }
            let unique: unsafe extern "C" fn(usize) -> bool = unsafe { std::mem::transmute(unique) };
            if slots() != (tap, source, choice) || !unsafe { unique(choice) } {
                return Err("Reference SnippetChoice is not solely owned by the original expander".into());
            }
            Some(drained)
        } else {
            None
        };
        unsafe { swift_self_adapter(pointer, entry); }
        if self.pointer_on_creator()? != pointer || slots() != (0, 0, 0) {
            return Err("Original SnippetExpander stop did not clear its resource fields".into());
        }
        if let Some(verify) = choice_drained {
            verify()?;
            crate::workspace::objc::verify_reference_snippet_windows_drained(model)?;
        }
        if let Some(verify) = tap_stopped {
            verify()?;
        }
        Ok(())
    }

    pub(crate) fn verify_current_snippet_choice(
        &self,
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        model: &Arc<Workspace>,
        controller: usize,
    ) -> Result<()> {
        if self.kind != ReferenceInputChildKind::SnippetExpander || controller == 0 {
            return Err("Reference SnippetChoice requires its original expander".into());
        }
        self.verify_reference_binding(admission, model)?;
        let expander = self.pointer_on_creator()?;
        let choice = self.input_registration_address(expander, c"choice", 72)?;
        if unsafe { choice.read_volatile() } != controller {
            return Err("Reference SnippetChoice differs from the expander's strong slot".into());
        }
        unsafe { admission.verify_snippet_choice_completion(controller, expander)?; }
        self.verify_held_epoch()?;
        if unsafe { choice.read_volatile() } != controller {
            return Err("Reference SnippetChoice changed during ownership verification".into());
        }
        Ok(())
    }

    pub(crate) fn invalidate_input_poll_timer(&self) -> Result<()> {
        use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopTimer, CFType, CFRetained};
        let (field_name, field_offset) = match self.kind {
            ReferenceInputChildKind::SnippetExpander => (c"poll", 64),
            ReferenceInputChildKind::HyperKey => (c"timer", 48),
            ReferenceInputChildKind::Hotkeys => return Err("Reference Hotkeys has no input poll timer".into()),
        };
        let data = &self.parent.root.declaration.data;
        data.context.verify_input_parent_source_members()?;
        data.context.verify_original_compiler_text()?;
        let pointer = self.pointer_on_creator()?;
        let (name, image_offset) = self.kind.class();
        let class = metadata::loaded_class(name)?;
        if data.main_base.checked_add(image_offset) != Some(class.address()) {
            return Err("Reference input poll class differs".into());
        }
        let offset = bounded_swift_field(class, field_offset)?;
        let fields = unsafe { &*(class.address() as *const AnyClass) }.instance_variables();
        let mut matching = fields.iter().filter(|field| field.name() == field_name);
        if matching.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(offset)
            || matching.next().is_some()
        {
            return Err("Reference input poll ivar differs".into());
        }
        let slot = pointer.checked_add(offset).ok_or("Reference input poll field overflow")? as *const usize;
        self.verify_held_epoch()?;
        let address = unsafe { slot.read_volatile() };
        let receiver = std::ptr::NonNull::new(address as *mut AnyObject).ok_or("Reference input poll absent")?;
        let timer_class = metadata::loaded_class(c"__NSCFTimer")?;
        if unsafe { receiver.as_ref() }.class() as *const AnyClass as usize != timer_class.address() {
            return Err("Reference input poll receiver is not a Foundation timer".into());
        }
        let timer = unsafe { receiver.cast::<CFType>().as_ref() }.downcast_ref::<CFRunLoopTimer>()
            .ok_or("Reference input poll is not a CF timer")?;
        // Hold a physical reference while invalidation can release the callback.
        let timer = unsafe { CFRetained::retain(std::ptr::NonNull::from(timer)) };
        let run_loop = CFRunLoop::current().ok_or("Reference input poll runloop absent")?;
        let mode = unsafe { kCFRunLoopDefaultMode }.ok_or("Reference input poll mode absent")?;
        let stopped = self.stopped_poll.load(Ordering::Acquire);
        if stopped != 0 {
            if stopped != address || timer.is_valid() || run_loop.contains_timer(Some(&timer), Some(mode)) {
                return Err("Reference input poll stop changed or reentered".into());
            }
            self.verify_held_epoch()?;
            if unsafe { slot.read_volatile() } != address {
                return Err("Reference input poll changed after invalidation".into());
            }
            return Ok(());
        }
        if !timer.is_valid() || !run_loop.contains_timer(Some(&timer), Some(mode)) {
            return Err("Reference input poll was not active on its owner loop".into());
        }
        self.verify_held_epoch()?;
        if unsafe { slot.read_volatile() } != address {
            return Err("Reference input poll changed before invalidation".into());
        }
        self.stopped_poll.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Reference input poll stop reentered")?;
        timer.invalidate();
        if timer.is_valid() || run_loop.contains_timer(Some(&timer), Some(mode)) {
            return Err("Reference input poll still active after invalidation".into());
        }
        drop(timer);
        self.verify_held_epoch()?;
        if unsafe { slot.read_volatile() } != address {
            return Err("Reference input poll changed during invalidation".into());
        }
        self.verify_held_epoch()?;
        self.stopped_poll.store(address, Ordering::Release);
        Ok(())
    }

    fn keyboard_location(&self)->Result<(usize,usize)> {
        if self.kind!=ReferenceInputChildKind::Hotkeys{return Err("Keyboard projection requires actual Hotkeys custody".into());}
        let pointer=self.pointer_on_creator()?;
        let data=&self.parent.root.declaration.data;
        let class=metadata::loaded_class(c"RuycastF1Reference.Hotkeys")?;
        if data.main_base.checked_add(0x3efd90)!=Some(class.address()){return Err("Hotkeys class changed".into());}
        let (cell,value)=data.context.observe_hotkeys_keyboard_offset_cell()?;
        if data.main_base.checked_add(0x302b38)!=Some(cell)||value!=56{return Err("Private keyboard offset differs".into());}
        let offset=bounded_swift_field(class,value)?;
        let verify_ivar=|| -> Result<()> {
            let fields=unsafe{&*(class.address() as *const AnyClass)}.instance_variables();
            let matched=fields.iter().filter(|field|field.name()==c"keyboard").collect::<Vec<_>>();
            if fields.len()!=10 || matched.len()!=1
                ||usize::try_from(matched[0].offset()).ok()!=Some(offset) {
                return Err("Typed private keyboard vector does not match its current named ivar".into());
            }
            Ok(())
        };
        verify_ivar()?;
        // Original Hotkeys initializer preserves self in x19 and stores the
        // actual HyperKey allocation/init result at self+56 before registration.
        data.context.verify_main_code(data.main_base.checked_add(0xf33bc).ok_or("Hotkeys init address overflow")?,32,
            "6c14966a32145166cbf260d16e1bd90a357d1b6520564068c53208825bf09526")?;
        data.context.verify_main_code(data.main_base.checked_add(0xf3468).ok_or("Keyboard store address overflow")?,24,
            "b9bb45e751ceaadda7e4265f8cf88e5ee530118ac5372af5c80d826d0a6fa311")?;
        // The fixed typed observer binds RO let record5 -> HyperKey, actual
        // loaded descriptions and full64 vector5 == global56. Run it AFTER
        // every code/source guard and before any child-memory read.
        if data.context.observe_hotkeys_keyboard_offset_cell()? != (cell,value) {
            return Err("Typed private keyboard lineage changed before read".into());
        }
        verify_ivar()?;
        if bounded_swift_field(class,value)?!=offset{return Err("Keyboard instance bound changed".into());}
        self.verify_held_epoch()?;
        let child=unsafe{((pointer+offset) as *const usize).read()};
        if child==0{return Err("Initialized Hotkeys private-let keyboard is nil".into());}
        let after=data.context.observe_hotkeys_keyboard_offset_cell()?;
        if after!=(cell,value) {return Err("Private keyboard field layout changed".into());}
        self.verify_held_epoch()?;
        Ok((pointer,child))
    }
    /// No new retain: the real Hotkeys getter extra +1 holds its strong private
    /// let keyboard for this child lease. Whole compiled source membership and
    /// the actual bounded cell/instance checks are mandatory, not a class name.
    pub fn capture_keyboard(self:&Arc<Self>)->Result<Arc<ReferenceInputChildCustody>> {
        let (_,pointer)=self.keyboard_location()?;
        let held=Arc::new(ReferenceInputChildCustody{parent:self.parent.clone(),hotkeys:Some(self.clone()),
            kind:ReferenceInputChildKind::HyperKey,pointer,retired:AtomicBool::new(false),
            stopped_poll:std::sync::atomic::AtomicUsize::new(0)});
        held.pointer_on_creator()?;
        Ok(held)
    }
}

fn verify_original_main_executor(data: &AppDelegateConstructionData) -> Result<()> {
    #[repr(C)]
    struct Executor { identity: usize, implementation: usize }

    let main = MAIN_EXECUTOR_LABEL as *const () as usize;
    let current = CURRENT_EXECUTOR_LABEL as *const () as usize;
    let predicate = EXECUTOR_CHECK_LABEL as *const () as usize;
    let (row, code) = data.system.code.iter().find(|(row, _)| row.symbol == c"swift_task_isCurrentExecutor")
        .ok_or("Original Swift executor check import absent")?;
    let concurrency = b"/usr/lib/swift/libswift_Concurrency.dylib";
    if unsafe { libc::pthread_main_np() } != 1
        || data.main_base.checked_add(0x3c7850) != Some(row.cell_address())
        || row.original_address() != current
        || metadata::code_identity(current)? != *code
        || code.image() != concurrency
        || metadata::code_identity(main)?.image() != concurrency
        || metadata::code_identity(predicate)?.image() != concurrency
    {
        return Err("Original Swift main executor binding differs".into());
    }
    row.verify_unpatched()?;
    let get: unsafe extern "C" fn() -> Executor = unsafe { std::mem::transmute(main) };
    let check: unsafe extern "C" fn(usize, usize, u32) -> bool = unsafe { std::mem::transmute(predicate) };
    let executor = unsafe { get() };
    if executor.identity == 0 || !unsafe { check(executor.identity, executor.implementation, 0) } {
        return Err("Original Swift teardown requires MainActor".into());
    }
    row.verify_unpatched()
}
impl Drop for ReferenceInputChildCustody {
    fn drop(&mut self) {
        // Known original getter extra references remain quarantined. HyperKey
        // has no extra +1 of its own and its genuine strong parent stays held
        // through this value's lifetime; no unknown child is ever released.
    }
}

// The finite original SearchInput route. None of these objects is an SDK
// allocation, so it must never enter OwnedReceiver/retain_owned bookkeeping.
thread_local!{
    static SEARCH_FIELD:std::cell::RefCell<Option<std::rc::Rc<ReferenceSearchField>>>=const{std::cell::RefCell::new(None)};
    static SEARCH_MOUNT:std::cell::RefCell<Option<SearchMount>>=const{std::cell::RefCell::new(None)};
    static SEARCH_FIELD_ENTERED:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
    static SEARCH_FIELD_RELEASED:std::cell::Cell<Option<(usize,u64)>>=const{std::cell::Cell::new(None)};
    static SEARCH_FIELD_UNENTERED:std::cell::Cell<Option<(usize,u64)>>=const{std::cell::Cell::new(None)};
}
struct SearchMount {root:Arc<InitializedAppDelegateRoot>,state:Arc<AppStateCustody>,host:SearchReference,epoch:u64}
static SEARCH_FIELD_EPOCH:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
static SEARCH_EXTRA_REFS:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn verify_reference_field_drained(allow_unentered: bool) -> Result<()> {
    use std::sync::atomic::Ordering;
    let epoch = SEARCH_FIELD_EPOCH.load(Ordering::Acquire);
    let released = SEARCH_FIELD_RELEASED.with(std::cell::Cell::get);
    // The first mount publishes a nonzero epoch after taking its first retain.
    // An early failed wire request can leave the field completely untouched.
    let confirmed = if epoch == 0 {
        allow_unentered && released.is_none()
    } else {
        released.is_some_and(|(_, released_epoch)| released_epoch == epoch)
    };
    if unsafe { libc::pthread_main_np() } != 1
        || SEARCH_FIELD_ENTERED.with(|entered| entered.get())
        || !SEARCH_FIELD.with(|slot| slot.try_borrow().is_ok_and(|slot| slot.is_none()))
        || !SEARCH_MOUNT.with(|slot| slot.try_borrow().is_ok_and(|slot| slot.is_none()))
        || SEARCH_FIELD_UNENTERED.with(|slot| slot.get().is_some())
        || SEARCH_EXTRA_REFS.load(Ordering::Acquire) != 0
        || !confirmed
    {
        return Err("Reference field probe lacks confirmed field release".into());
    }
    Ok(())
}
struct SearchFieldEntry;
impl Drop for SearchFieldEntry {
    fn drop(&mut self) {
        SEARCH_FIELD_ENTERED.with(|entered| entered.set(false));
    }
}
#[derive(Clone)]
pub(crate) struct ReferenceFieldToken {owner:std::thread::ThreadId,epoch:u64,pointer:usize}
struct SearchReference {
    pointer:usize,class:metadata::ClassHandle,retain:metadata::MethodSnapshot,release:metadata::MethodSnapshot,
    retain_entry:metadata::CodeIdentity,release_entry:metadata::CodeIdentity,
}
impl SearchReference {
    fn verify_arc(&self)->Result<()> {
        if unsafe{&*(self.pointer as *const AnyObject)}.class() as *const AnyClass as usize!=self.class.address()
            ||metadata::code_identity(objc2::ffi::objc_retain as *const () as usize)?!=self.retain_entry
            ||metadata::code_identity(objc2::ffi::objc_release as *const () as usize)?!=self.release_entry {
            return Err("Reference field nonfinal ARC class/entry changed".into());
        }
        metadata::verify_current_method(self.class,c"retain",false,&self.retain)?;
        metadata::verify_current_method(self.class,c"release",false,&self.release)
    }
}
struct ReferenceSearchField {
    root:Arc<InitializedAppDelegateRoot>,state:Arc<AppStateCustody>,epoch:u64,
    host:usize,field:usize,coordinator:usize,field_class:metadata::ClassHandle,coordinator_class:metadata::ClassHandle,
    callback:metadata::MethodSnapshot,held:std::cell::RefCell<Vec<SearchReference>>,retired:std::cell::Cell<bool>,
    delegate_entered: std::cell::Cell<bool>,
}
// Only the exact finite read-only AppKit getters below use this entry. Their
// selected implementation must belong to Apple's AppKit/SwiftUI framework;
// neither source callbacks nor forwarding/resolve machinery are called.
fn search_platform_method(pointer:usize,selector:&'static std::ffi::CStr,abi:&[Encoding])->Result<metadata::MethodSnapshot>{
    if pointer==0{return Err("SearchInput platform receiver absent".into());}
    let class=unsafe{&*(pointer as *const AnyObject)}.class();
    let handle=metadata::registered_class(class as *const AnyClass as usize)?;
    let method=metadata::method(handle,selector,false)?.ok_or("SearchInput platform method absent")?;
    if !matches!(method.code().image(),b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit"|b"/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI"){
        return Err("SearchInput platform method provider differs".into());
    }
    super::super::super::classes::verify_typed_abi(&method, abi)?;
    Ok(method)
}
fn search_get(pointer:usize,selector:&'static std::ffi::CStr)->Result<usize>{
    if ![c"window",c"subviews",c"superview",c"delegate"].contains(&selector){return Err("Unknown SearchInput getter".into());}
    let method=search_platform_method(pointer,selector,&[Encoding::Object,Encoding::Object,Encoding::Sel])?;
    let call:unsafe extern "C-unwind" fn(*mut AnyObject,Sel)->*mut AnyObject=unsafe{std::mem::transmute(method.implementation())};
    Ok(unsafe{call(pointer as *mut AnyObject,Sel::register(selector))} as usize)
}
fn verify_offscreen_search_field(host: usize, field: usize) -> Result<()> {
    if search_get(host, c"window")? != 0 || search_get(field, c"window")? != 0 {
        return Err("Original SearchInput or host became visible".into());
    }
    let mut node = field;
    for _ in 0..64 {
        if node == host {
            return Ok(());
        }
        node = search_get(node, c"superview")?;
        if node == 0 {
            break;
        }
    }
    Err("Original SearchInput no longer belongs to retained host".into())
}
fn search_retain(pointer:usize,stopped:&AtomicBool)->Result<SearchReference>{
    if pointer==0{return Err("Reference search retain receiver absent".into());}
    let class=unsafe{&*(pointer as *const AnyObject)}.class();
    let root=metadata::loaded_class(c"NSObject")?;
    let class=metadata::registered_class(class as *const AnyClass as usize)?;
    let retain=metadata::method(class,c"retain",false)?.ok_or("SearchInput ARC retain absent")?;
    let release=metadata::method(class,c"release",false)?.ok_or("SearchInput ARC release absent")?;
    for (method,abi) in [(&retain,b"@@:".as_slice()),(&release,b"v@:".as_slice())]{
        if method.owner()!=root.address()||method.code().image()!=b"/usr/lib/libobjc.A.dylib"
            ||super::super::super::simple_abi(&method)?!=abi{return Err("SearchInput has unqualified custom ARC".into());}
    }
    let retain_entry=metadata::code_identity(objc2::ffi::objc_retain as *const () as usize)?;
    let release_entry=metadata::code_identity(objc2::ffi::objc_release as *const () as usize)?;
    if retain_entry.image()!=b"/usr/lib/libobjc.A.dylib"||release_entry.image()!=b"/usr/lib/libobjc.A.dylib"{return Err("Reference field ARC entry provider differs".into());}
    let reference=SearchReference{pointer,class,retain,release,retain_entry,release_entry};reference.verify_arc()?;
    SEARCH_EXTRA_REFS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<64).then_some(n+1)).map_err(|_|"Reference field extra-reference quarantine limit reached")?;
    if stopped.load(Ordering::Acquire) {
        SEARCH_EXTRA_REFS.fetch_sub(1, Ordering::AcqRel);
        return Err("Reference SearchInput retain stopped".into());
    }
    let retained=unsafe{objc2::ffi::objc_retain(pointer as *mut AnyObject)};
    if retained as usize!=pointer{return Err("SearchInput retain changed identity".into());}Ok(reference)
}
/// # Safety
/// The retained offscreen host and its hierarchy must stay live and unchanged
/// on the creator thread throughout the walk and use of the borrowed result.
/// Only qualified read-only platform getters may run before acquiring custody.
unsafe fn search_find(pointer:usize,field_class:metadata::ClassHandle,depth:usize,budget:&mut usize)->Result<Option<usize>>{
    if depth>64||*budget==0{return Err("SearchInput hierarchy exceeds bound".into());}*budget-=1;
    if unsafe{&*(pointer as *const AnyObject)}.class() as *const AnyClass as usize==field_class.address(){return Ok(Some(pointer));}
    let array=search_get(pointer,c"subviews")?;
    let array=unsafe{objc2::rc::Retained::retain(array as *mut objc2_foundation::NSArray<AnyObject>)}.ok_or("SearchInput subviews absent")?;
    let mut found=None;
    for child in array.iter(){
        if let Some(field)=unsafe { search_find(std::ptr::from_ref(&*child) as usize,field_class,depth+1,budget) }?{
            if found.replace(field).is_some(){return Err("Ambiguous original SearchInput hierarchy".into());}
        }
    }
    Ok(found)
}

/// The caller holds the exact mounted palette host on the main thread.
/// Only the unique original field reached by walking that host can receive
/// the stock detached NSTextField responder attempt.
pub(crate) fn try_reference_detached_search_focus(
    host: usize, requested: usize, stopped: &AtomicBool,
) -> Result<Bool> {
    if unsafe { libc::pthread_main_np() } != 1 || stopped.load(Ordering::Acquire) {
        return Err("detached SearchInput focus is stopped or off owner".into());
    }
    let field_class = metadata::loaded_class(c"RuycastF1Reference.SearchInput")?;
    let field = unsafe { search_find(host, field_class, 0, &mut 8192) }?
        .ok_or("original SearchInput missing from mounted host")?;
    if field != requested {
        return Err("requested responder differs from mounted SearchInput".into());
    }
    verify_offscreen_search_field(host, field)?;
    let method = search_platform_method(field, c"becomeFirstResponder", &[
        Bool::ENCODING, Encoding::Object, Encoding::Sel,
    ])?;
    let appkit_field = metadata::loaded_class(c"NSTextField")?;
    if method.owner() != appkit_field.address() || method.code().offset() != 0x1b8cec
        || method.code().uuid() != [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64,
            0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46]
    {
        return Err("detached SearchInput platform responder entry differs".into());
    }
    let held = search_retain(field, stopped)?;
    let result = (|| -> Result<Bool> {
        held.verify_arc()?;
        metadata::verify_current_method(field_class, c"becomeFirstResponder", false, &method)?;
        if stopped.load(Ordering::Acquire) { return Err("detached SearchInput focus stopped".into()); }
        let call: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool =
            unsafe { std::mem::transmute(method.implementation()) };
        let answer = unsafe { call(field as *mut AnyObject, objc2::sel!(becomeFirstResponder)) };
        verify_offscreen_search_field(host, field)?;
        if stopped.load(Ordering::Acquire) { return Err("detached SearchInput focus stopped after dispatch".into()); }
        if answer.as_bool() {
            return Err("detached SearchInput unexpectedly became first responder".into());
        }
        Ok(answer)
    })();
    // This extra reference is only qualified for a nonfinal release.
    // A detached field after the attempt must remain quarantined.
    verify_offscreen_search_field(host, field)?;
    held.verify_arc()?;
    unsafe { objc2::ffi::objc_release(field as *mut AnyObject) };
    SEARCH_EXTRA_REFS.fetch_sub(1, Ordering::AcqRel);
    result
}
impl ReferenceSearchField {
    fn pure(&self)->Result<()> {
        self.root.verify_held_epoch()?;self.state.verify_held_epoch()?;
        if self.retired.get()||unsafe{libc::pthread_main_np()}!=1{return Err("Reference field epoch retired or foreign-thread".into());}
        if unsafe{&*(self.field as *const AnyObject)}.class() as *const AnyClass as usize!=self.field_class.address()
            ||unsafe{&*(self.coordinator as *const AnyObject)}.class() as *const AnyClass as usize!=self.coordinator_class.address(){return Err("Original field/coordinator class changed".into());}
        metadata::verify_current_method(self.coordinator_class,c"controlTextDidChange:",false,&self.callback)?;
        for reference in self.held.try_borrow().map_err(|_|"Reference field ARC list borrowed")?.iter(){reference.verify_arc()?;}
        // These are qualified read-only platform getters, with no model/Source
        // guardian callback. Original dismantle sets delegate=nil BEFORE the
        // hosted view/coordinator can leave SwiftUI's ownership.
        verify_offscreen_search_field(self.host, self.field)?;
        if search_get(self.field, c"delegate")? != self.coordinator {
            return Err("Original field delegate changed".into());
        }
        self.root.verify_held_epoch()
    }
    fn verify(&self,host:usize)->Result<()> {
        if host!=self.host{return Err("Reference Bridge content root changed".into());}
        let data=&self.root.declaration.data;
        // These are synchronous compiler, metadata and import observations.
        // Keep the state/custody check and platform getters outside the scope.
        unsafe { data.context.with_callback_source_observation(|| {
            data.context.verify_search_field_conformance()?;
            callbacks::verify_search_notification_type(data.context.clone())?;
            data.context.verify_main_code(self.callback.implementation(),236,"5a9d6e40bd9d4e38bd0c494ee370c7b8da95cacc79b7c411a4c4f4116e2dacb0")
        }) }?;
        self.state.pointer_on_creator()?;
        self.pure()
    }
}
impl Drop for ReferenceSearchField {
    fn drop(&mut self){
        // Unknown detach/failure never performs final original ARC. Successful
        // retirement below empties this list while independent framework/root
        // ownership remains verified. Otherwise the bounded +1s quarantine.
        self.held.get_mut().clear();
    }
}
impl ReferenceFieldToken {
    pub(crate) fn verify_model(&self,model:&Arc<Workspace>)->Result<()>{
        self.object()?;
        let same=SEARCH_FIELD.with(|s|s.try_borrow().ok().is_some_and(|s|s.as_ref().is_some_and(|field|Arc::ptr_eq(&field.root.declaration.data.model,model))));
        if !same{return Err("Reference field notification belongs to another model".into());}Ok(())
    }
    pub(crate) fn object(&self)->Result<*mut AnyObject>{
        if self.owner!=std::thread::current().id(){return Err("Reference Notification field on foreign thread".into());}
        let field=SEARCH_FIELD.with(|slot|slot.try_borrow().ok().and_then(|slot|slot.clone())).ok_or("Reference Notification field retired")?;
        if field.epoch!=self.epoch||field.field!=self.pointer{return Err("Reference Notification field/epoch differs".into());}
        field.pure()?;
        // Notification.object is +0. The active TLS custody and synchronous
        // original callback keep this exact field alive; no SDK ARC operation.
        Ok(self.pointer as *mut AnyObject)
    }
}
pub(crate) fn begin_reference_field(root:Arc<InitializedAppDelegateRoot>,state:Arc<AppStateCustody>,host:usize,query:&str,epoch:u64,stopped:&AtomicBool)->Result<bool> {
    if epoch==0||query.len()>4096||SEARCH_FIELD_ENTERED.with(|s|s.replace(true)){
        return Err("Reference field input busy/invalid".into());
    }
    let _entered = SearchFieldEntry;
    if SEARCH_FIELD.with(|slot| slot.try_borrow().map_or(true, |slot| slot.is_some())) {
        return Err("Reference field input already active".into());
    }
    if SEARCH_MOUNT.with(|slot| slot.try_borrow().is_ok_and(|slot| slot.is_none())) {
        if host == 0 || epoch <= SEARCH_FIELD_EPOCH.load(Ordering::Acquire)
            || SEARCH_EXTRA_REFS.load(Ordering::Acquire) != 0
            || SEARCH_FIELD_UNENTERED.with(|slot| slot.get().is_some())
        {
            return Err("Reference field cannot begin an unentered attempt".into());
        }
        // Bind this exact attempt before preflight. A failure before the
        // first retain can be retired after the original trace proves no effect.
        SEARCH_FIELD_UNENTERED.with(|slot| slot.set(Some((host, epoch))));
    }
    let data=&root.declaration.data;
    // The conformance verifier performs several immutable source checks.
    // Use the existing source observation for this preflight only. Layout,
    // platform getters and the original delegate run outside that scope.
    unsafe {
        data.context.with_callback_source_observation(|| data.context.verify_search_field_conformance())?;
    }
    let field_class=metadata::loaded_class(c"RuycastF1Reference.SearchInput")?;
    let coordinator_class=metadata::loaded_class(c"_TtCV18RuycastF1Reference18PaletteSearchField11Coordinator")?;
    let callback=metadata::method(coordinator_class,c"controlTextDidChange:",false)?.ok_or("Original field delegate method absent")?;
    if data.main_base.checked_add(0x3ea8c0)!=Some(field_class.address())||data.main_base.checked_add(0x3ea800)!=Some(coordinator_class.address())
        ||data.main_base.checked_add(0x159d64)!=Some(callback.implementation())||callback.owner()!=coordinator_class.address()
        ||super::super::super::simple_abi(&callback)?!=b"v@:@"{return Err("Original field/coordinator metadata or callback differs".into());}
    if unsafe{&*(field_class.address() as *const AnyClass)}.superclass().map(|class|class as *const AnyClass as usize)!=Some(metadata::loaded_class(c"NSTextField")?.address())
        ||unsafe{&*(coordinator_class.address() as *const AnyClass)}.superclass().map(|class|class as *const AnyClass as usize)!=Some(metadata::loaded_class(c"NSObject")?.address()){
        return Err("Original SearchInput/coordinator superclass changed".into());
    }
    if search_get(host,c"window")?!=0{return Err("Reference field requires an offscreen host".into());}
    let prior=SEARCH_MOUNT.with(|s|->Result<_>{let mut s=s.try_borrow_mut().map_err(|_|"Reference field mount slot borrowed")?;
        if s.as_ref().is_some_and(|mount|mount.epoch!=epoch||mount.host.pointer!=host||!Arc::ptr_eq(&mount.root,&root)||!Arc::ptr_eq(&mount.state,&state)){return Err("Reference field mount issuer/epoch differs".into());}
        Ok(s.take())})?;
    let mount=match prior{
        Some(mount)=>mount,
        None=>{
            if SEARCH_EXTRA_REFS.load(Ordering::Acquire) != 0 {
                return Err("Reference field has unretired extra references".into());
            }
            if epoch <= SEARCH_FIELD_EPOCH.load(Ordering::Acquire) {
                return Err("Reference field epoch replay".into());
            }
            // The creator's SearchFieldEntry excludes reentry. Do not publish
            // an epoch when stop prevents taking the first physical reference.
            let held_host = search_retain(host,stopped)?;
            SEARCH_FIELD_EPOCH.store(epoch,Ordering::Release);
            SEARCH_FIELD_UNENTERED.with(|slot| slot.set(None));
            SEARCH_FIELD_RELEASED.with(|released| released.set(None));
            SearchMount{host:held_host,root:root.clone(),state:state.clone(),epoch}
        }
    };
    let mounted = (|| -> Result<Option<(SearchReference, SearchReference)>> {
        // Keep a resumed mount available for the confirmed reset path even if
        // its ARC check fails before any layout or delegate dispatch.
        mount.host.verify_arc()?;
        root.verify_held_epoch()?;
        state.verify_held_epoch()?;
        mount.host.verify_arc()?;
        super::super::super::objc::layout_reference_field_host(&root.declaration.data.model,host,stopped)?;
        root.verify_held_epoch()?;
        state.verify_held_epoch()?;
        mount.host.verify_arc()?;
        if search_get(host,c"window")?!=0{return Err("Reference field host became visible during mount".into());}
        let Some(field)=(unsafe { search_find(host,field_class,0,&mut 8192) })? else{
            root.verify_held_epoch()?;state.verify_held_epoch()?;mount.host.verify_arc()?;
            if search_get(host,c"window")?!=0{return Err("Reference field host became visible during mount".into());}
            return Ok(None);
        };
        verify_offscreen_search_field(host, field)?;
        let coordinator=search_get(field,c"delegate")?;
        if coordinator == 0 {
            // The first layout may expose the field before installing its
            // delegate. Retry with the same retained offscreen mount.
            root.verify_held_epoch()?;
            state.verify_held_epoch()?;
            mount.host.verify_arc()?;
            return Ok(None);
        }
        if unsafe { &*(coordinator as *const AnyObject) }.class() as *const AnyClass as usize
            != coordinator_class.address()
        {
            return Err("Original field delegate has a foreign class".into());
        }
        let field_reference = search_retain(field,stopped)?;
        match search_retain(coordinator,stopped) {
            Ok(coordinator_reference) => Ok(Some((field_reference, coordinator_reference))),
            Err(error) => {
                verify_offscreen_search_field(host, field)?;
                root.verify_held_epoch()?;
                state.verify_held_epoch()?;
                mount.host.verify_arc()?;
                field_reference.verify_arc()?;
                unsafe { objc2::ffi::objc_release(field as *mut AnyObject) };
                SEARCH_EXTRA_REFS.fetch_sub(1, Ordering::AcqRel);
                Err(error)
            }
        }
    })();
    let (field_reference, coordinator_reference) = match mounted {
        Ok(Some(references)) => references,
        incomplete => {
            // Layout can fail after the host has been retained. Leave that
            // exact mount available to the caller's confirmed reset path.
            SEARCH_MOUNT.with(|slot| -> Result<()> {
                let mut slot = slot.try_borrow_mut().map_err(|_| "Reference field mount slot borrowed")?;
                if slot.is_some() {
                    return Err("Reference field mount publication replay".into());
                }
                *slot = Some(mount);
                Ok(())
            })?;
            return incomplete.map(|_| false);
        }
    };
    let field = field_reference.pointer;
    let coordinator = coordinator_reference.pointer;
    let session=std::rc::Rc::new(ReferenceSearchField{root,state,epoch,host,field,coordinator,field_class,coordinator_class,callback,
        held:std::cell::RefCell::new(vec![mount.host,field_reference,coordinator_reference]),retired:std::cell::Cell::new(false),
        delegate_entered: std::cell::Cell::new(false)});
    SEARCH_FIELD.with(|slot|->Result<()>{let mut slot=slot.try_borrow_mut().map_err(|_|"Reference field slot borrowed")?;if slot.is_some(){return Err("Reference field publication replay".into());}*slot=Some(session.clone());Ok(())})?;
    session.verify(host)?;
    objc2::rc::autoreleasepool(|_|->Result<()>{
        let token=ReferenceFieldToken{owner:std::thread::current().id(),epoch,pointer:field};
        let notification=super::super::super::objc::new_reference_field_notification(&session.root.declaration.data.model,token.clone())?;
        let text=objc2_foundation::NSString::from_str(query);
        session.verify(host)?;
        let setter=search_platform_method(field,c"setStringValue:",&[Encoding::Void,Encoding::Object,Encoding::Sel,Encoding::Object])?;
        let setter:unsafe extern "C-unwind" fn(*mut AnyObject,Sel,*const objc2_foundation::NSString)=unsafe{std::mem::transmute(setter.implementation())};
        session.pure()?;
        if stopped.load(Ordering::Acquire) { return Err("Reference field stopped before text effect".into()); }
        unsafe{setter(field as *mut AnyObject,Sel::register(c"setStringValue:"),&*text);}
        session.verify(host)?;token.object()?;
        let payload=super::super::super::objc::notification_payload_pure_check(notification.as_ptr(),&session.root.declaration.data.model)?;
        session.pure()?;payload()?;token.object()?;
        let call:unsafe extern "C-unwind" fn(*mut AnyObject,Sel,*mut AnyObject)=unsafe{std::mem::transmute(session.callback.implementation())};
        if stopped.load(Ordering::Acquire) { return Err("Reference field stopped before delegate effect".into()); }
        session.delegate_entered.set(true);
        unsafe{call(coordinator as *mut AnyObject,Sel::register(c"controlTextDidChange:"),notification.as_ptr());}
        session.verify(host)?;payload()?;token.object()?;
        if stopped.load(Ordering::Acquire) { return Err("Reference field stopped during delegate effect".into()); }
        Ok(())
    })?;Ok(true)
}
pub(crate) fn verify_reference_field(host:usize,epoch:u64)->Result<()> {
    let session=SEARCH_FIELD.with(|s|s.try_borrow().ok().and_then(|s|s.clone())).ok_or("Reference field epoch absent")?;
    if session.epoch!=epoch{return Err("Reference field epoch differs".into());}session.verify(host)
}
pub(crate) fn field_delegate_not_entered(host: usize, epoch: u64) -> Result<bool> {
    if unsafe { libc::pthread_main_np() } != 1 || SEARCH_FIELD_ENTERED.with(|entered| entered.get()) {
        return Err("Reference delegate entry check requires the idle creator thread".into());
    }
    let session = SEARCH_FIELD.with(|slot| {
        slot.try_borrow().map(|slot| slot.clone()).map_err(|_| "Reference field slot borrowed")
    })?;
    if let Some(session) = session {
        return Ok(session.host == host && session.epoch == epoch
            && !session.retired.get() && !session.delegate_entered.get());
    }
    SEARCH_MOUNT.with(|slot| {
        let slot = slot.try_borrow().map_err(|_| "Reference mount slot borrowed")?;
        Ok(if let Some(mount) = slot.as_ref() {
            mount.host.pointer == host && mount.epoch == epoch
        } else {
            unentered_reference_field(host, epoch)
        })
    })
}
fn unentered_reference_field(host: usize, epoch: u64) -> bool {
    SEARCH_FIELD_UNENTERED.with(|slot| slot.get() == Some((host, epoch)))
        && epoch > SEARCH_FIELD_EPOCH.load(Ordering::Acquire)
        && SEARCH_EXTRA_REFS.load(Ordering::Acquire) == 0
}
pub(crate) fn quarantine_reference_field(epoch:u64){
    SEARCH_FIELD.with(|s|{if let Ok(mut s)=s.try_borrow_mut(){if s.as_ref().is_some_and(|field|field.epoch==epoch){s.take();}}});
    SEARCH_MOUNT.with(|s|{if let Ok(mut s)=s.try_borrow_mut(){if s.as_ref().is_some_and(|field|field.epoch==epoch){s.take();}}});
}
pub(crate) fn retire_reference_field(host:usize,epoch:u64)->Result<()> {
    if unsafe { libc::pthread_main_np() } != 1 || SEARCH_FIELD_ENTERED.with(|entered| entered.replace(true)) {
        return Err("Reference field retirement requires the idle creator thread".into());
    }
    let _entered = SearchFieldEntry;
    let mounting=SEARCH_MOUNT.with(|s|->Result<_>{let mut s=s.try_borrow_mut().map_err(|_|"Reference mount retirement slot borrowed")?;
        if s.as_ref().is_some_and(|mount|mount.epoch!=epoch||mount.host.pointer!=host){return Err("Reference mount retirement issuer/epoch differs".into());}Ok(s.take())})?;
    if let Some(mount)=mounting{
        let verified = (|| -> Result<()> {
            mount.root.verify_held_epoch()?;
            mount.state.verify_held_epoch()?;
            mount.host.verify_arc()?;
            if search_get(host, c"window")? != 0 {
                return Err("Reference mount no longer offscreen".into());
            }
            mount.root.verify_held_epoch()?;
            mount.state.verify_held_epoch()?;
            mount.host.verify_arc()
        })();
        if let Err(error) = verified {
            SEARCH_MOUNT.with(|slot| -> Result<()> {
                let mut slot = slot.try_borrow_mut().map_err(|_| "Reference mount retirement slot borrowed")?;
                if slot.is_some() {
                    return Err("Reference mount changed during retirement".into());
                }
                *slot = Some(mount);
                Ok(())
            })?;
            return Err(error);
        }
        unsafe{objc2::ffi::objc_release(host as *mut AnyObject);}SEARCH_EXTRA_REFS.fetch_sub(1,Ordering::AcqRel);
        if SEARCH_EXTRA_REFS.load(Ordering::Acquire) != 0 {
            return Err("Reference mount retired with unconfirmed field references".into());
        }
        SEARCH_FIELD_RELEASED.with(|released| released.set(Some((host, epoch))));
        return Ok(());
    }
    if SEARCH_FIELD.with(|s|s.try_borrow().ok().is_some_and(|s|s.is_none()))
    {
        if unentered_reference_field(host, epoch) {
            SEARCH_FIELD_UNENTERED.with(|slot| slot.set(None));
            return Ok(());
        }
        if SEARCH_FIELD_RELEASED.with(|released| released.get() == Some((host, epoch))) {
            return Ok(());
        }
    }
    let session=SEARCH_FIELD.with(|s|s.try_borrow().ok().and_then(|s|s.clone())).ok_or("Reference field epoch absent")?;
    if session.host != host || session.epoch != epoch {
        return Err("Reference field retirement host or epoch differs".into());
    }
    session.verify(host)?;
    SEARCH_FIELD.with(|slot|->Result<()>{
        let mut slot=slot.try_borrow_mut().map_err(|_|"Reference field retirement slot borrowed")?;
        if !slot.as_ref().is_some_and(|current|current.epoch==epoch&&std::rc::Rc::ptr_eq(current,&session)){
            return Err("Reference field slot changed during retirement".into());
        }
        slot.take();Ok(())
    })?;
    // WWDC22/10075: coordinator lifetime follows the hosted view. The bound
    // conformance + source's nil-before-dismantle + current original hierarchy
    // establish framework ownership independent of these three extra retains.
    // No dynamic policy callback follows the final pure owner check.
    let held=std::mem::take(&mut *session.held.borrow_mut());
    session.retired.set(true);
    for reference in held.into_iter().rev(){reference.verify_arc()?;unsafe{objc2::ffi::objc_release(reference.pointer as *mut AnyObject);}SEARCH_EXTRA_REFS.fetch_sub(1,Ordering::AcqRel);}
    if SEARCH_EXTRA_REFS.load(Ordering::Acquire) != 0 {
        return Err("Reference field retired with unconfirmed extra references".into());
    }
    SEARCH_FIELD_RELEASED.with(|released| released.set(Some((host, epoch))));
    Ok(())
}

#[path="workspace_appdelegate_callbacks.rs"]
pub(crate) mod callbacks;
