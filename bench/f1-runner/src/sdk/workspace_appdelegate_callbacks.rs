//! Actual raw3 callback evidence and closed original-code qualification.
//! Whole generated Objective-C To entries are selected; no Swift-body cast.
use super::*;
#[path="workspace_appdelegate_callback_pins.rs"]
mod pins;
#[path="workspace_appdelegate_notification.rs"]
mod notification;

pub(super) fn verify_search_notification_type(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>)->Result<()> {
    // Reuse actual Foundation Notification metadata/bridge/type witnesses.
    // The SearchInput CTo and object effect are qualified separately.
    notification::NotificationValueWitnessData::observe(context)?.verify()
}

pub struct AppDelegateCallbackAuthority {
    data:Arc<AppDelegateConstructionData>,
    arc:NonfinalArc,
    notification:notification::NotificationValueWitnessData,
    state_cleanup:AppStateCompilerCleanup,
    server_cleanup:ControlServerCompilerCleanup,
    controller_cleanup:ControllerCompilerCleanup,
    subscriptions_memory:SubscriptionsCompilerMemory,
    stack_probe:crate::sdk::imports::UnpatchedImportObservation,
    stack_probe_code:metadata::CodeIdentity,
    methods:Vec<metadata::MethodSnapshot>,
    weak:Vec<metadata::MethodSnapshot>,
    retired:AtomicBool,
    verifying:AtomicBool,
}
impl AppDelegateCallbackAuthority {
    pub fn observe(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,model:Arc<Workspace>)->Result<Arc<Self>> {
        let notification=notification::NotificationValueWitnessData::observe(context.clone())?;
        // Address binding only: Rust never calls this non-C compiler ABI.
        unsafe extern "C" {#[link_name="__chkstk_darwin"] fn STACK_PROBE_LABEL();}
        let probe=STACK_PROBE_LABEL as *const () as usize;
        let stack_probe_code=metadata::code_identity(probe)?;
        if stack_probe_code.image()!=b"/usr/lib/system/libsystem_pthread.dylib" {
            return Err("Original compiler stack probe has another system provider".into());
        }
        let stack_probe=crate::sdk::imports::observe_unpatched_main_imports(&[(c"__chkstk_darwin",probe)])?
            .pop().ok_or("Actual Main stack probe import absent")?;
        let data=Arc::new(AppDelegateConstructionData::observe(context,model)?);
        let arc=NonfinalArc::observe(data.clone())?;
        let state_cleanup=AppStateCompilerCleanup::observe(&data)?;
        let server_cleanup=ControlServerCompilerCleanup::observe(&data,&state_cleanup)?;
        let controller_cleanup=ControllerCompilerCleanup::observe(&data,&state_cleanup,&server_cleanup)?;
        let subscriptions_memory=SubscriptionsCompilerMemory::observe(&data)?;
        let mut methods=Vec::new();
        for row in &pins::ROWS {
            let method=metadata::method(data.class,row.selector,false)?.ok_or("Original AppDelegate callback absent")?;
            methods.push(method);
        }
        let mut weak=Vec::new();
        for selector in [c"allowsWeakReference",c"retainWeakReference"] {
            weak.push(metadata::method(data.class,selector,false)?.ok_or("AppDelegate inherited weak primitive absent")?);
        }
        let result=Arc::new(Self{data,arc,notification,state_cleanup,server_cleanup,controller_cleanup,subscriptions_memory,stack_probe,stack_probe_code,methods,weak,retired:AtomicBool::new(false),verifying:AtomicBool::new(false)});
        result.verify_entry_evidence()?;
        Ok(result)
    }
    fn pure(&self)->Result<()> {
        self.data.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire){return Err("AppDelegate callback qualification retired".into());}
        Ok(())
    }
    fn qualify<T>(&self,call:impl FnOnce()->Result<T>)->Result<T> {
        self.pure()?;
        if self.verifying.swap(true,Ordering::AcqRel) {
            self.retired.store(true,Ordering::Release);
            return Err("AppDelegate callback qualification reentered".into());
        }
        struct Guard<'a>{authority:&'a AppDelegateCallbackAuthority,complete:bool}
        impl Drop for Guard<'_>{fn drop(&mut self){
            if !self.complete {self.authority.retired.store(true,Ordering::Release);}
            self.authority.verifying.store(false,Ordering::Release);
        }}
        let mut guard=Guard{authority:self,complete:false};
        let value=call()?;
        self.pure()?;
        guard.complete=true;Ok(value)
    }
    /// DATA/source/entry ABI evidence only, never callback or disposal permission.
    pub fn verify_entry_evidence(&self)->Result<()> {self.verify_evidence(true)}
    fn verify_evidence(&self,business:bool)->Result<()> {self.qualify(||self.evidence_body(business))}
    fn evidence_body(&self,business:bool)->Result<()> {
            if business {self.data.context.verify_model(&self.data.model)?;}
            // Dynamic model/guardian checks remain outside the pure source
            // observation. No receiver, sender or cleanup effect is enclosed.
            unsafe{self.data.context.with_callback_source_observation(||self.evidence_observation())}
    }
    fn evidence_observation(&self)->Result<()> {
            self.pure()?;
            self.data.verify_bodies_and_layout()?;
            self.pure()?;
            self.data.context.verify_original_compiler_text()?;
            self.pure()?;
            self.arc.verify()?;
            self.pure()?;
            self.notification.verify()?;
            self.pure()?;
            for pin in pins::PINS {
                self.data.context.verify_main_code(self.data.main_base.checked_add(pin.offset).ok_or("Callback code address overflow")?,pin.bytes,pin.sha)?;
                self.pure()?;
            }
            for (row,expected) in pins::ROWS.iter().zip(&self.methods) {
                let pin=&pins::PINS[row.entry];
                let abi=if row.notification {b"v@:@".as_slice()} else {b"v@:".as_slice()};
                if expected.owner()!=self.data.class.address()
                    ||expected.implementation()!=self.data.main_base.checked_add(pin.offset).ok_or("Callback entry overflow")?
                    ||super::super::super::super::simple_abi(expected)?!=abi {
                    return Err("Original AppDelegate callback owner/entry/ABI differs".into());
                }
                metadata::verify_current_method(self.data.class,row.selector,false,expected)?;
                // The separate original Swift body remains DATA. Only the
                // complete generated To entry may become a callable target.
                let body=&pins::PINS[row.body];
                self.data.context.verify_main_code(self.data.main_base+body.offset,body.bytes,body.sha)?;
            }
            self.verify_weak_entry_evidence()?;
            self.pure()
    }
    fn verify_weak_entry_evidence(&self)->Result<()> {
        for (selector,expected) in [c"allowsWeakReference",c"retainWeakReference"].into_iter().zip(&self.weak) {
            let abi=super::super::super::super::simple_abi(expected)?;
            if expected.owner()!=self.data.root_init.method().owner()
                ||expected.code().image()!=b"/usr/lib/libobjc.A.dylib"
                ||(abi!=b"B@:"&&abi!=b"c@:") {
                return Err("Original AppDelegate weak primitive ABI/owner differs".into());
            }
            metadata::verify_current_method(self.data.class,selector,false,expected)?;
        }
        // This verifies inherited Objective-C primitive identity only. Swift
        // weak-load/destroy, queued closures and Combine cancellation are not
        // inferred from those two methods.
        Ok(())
    }
    fn binding(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.pure()?;
        if class as *const AnyClass as usize!=self.data.class.address()
            ||source!=self.data.context.callback_source() {
            return Err("Original AppDelegate callback class/source differs".into());
        }
        Ok(())
    }
    fn require_callback_and_disposal_contract(&self)->Result<()> {
        self.verify_notification_thunk()?;
        self.verify_control_thunks()?;
        // Decode the actual Rust-called CTo, compiler adapter and ownership
        // corpus. Swift business bodies retain their separate full-build/source
        // hash evidence; this is not a general whole-program callgraph gate.
        // An unqualified interop/witness/disposal edge remains closed.
        for pin in pins::PINS {
            if pin.role.ends_with("/body") {continue;}
            if matches!(pin.role,"notification_shared_CTo"|"notification_retain_self_tail"
                |"notification_postbody_objc_release_tail"|"notification_restore_registers") {
                // All four exact complete members were qualified together
                // above, including typed imports and receiver/value custody.
                continue;
            }
            if matches!(pin.role,"action_shared_CTo"|"quit/entry"|"quit_release_actor"|"control_release_actor") {
                // Exact original compiler members are qualified as one
                // receiver/executor contract, never as arbitrary indirect calls.
                continue;
            }
            for (index,word) in pin.words.iter().copied().enumerate() {
                let pc=pin.offset.checked_add(index*4).ok_or("Callback instruction offset overflow")?;
                if word&0x7c000000==0x14000000 {
                    let delta=((word&0x03ffffff) as i32)<<6>>4;
                    let target=pc.checked_add_signed(delta as isize).ok_or("Callback branch target overflow")?;
                    // Only this exact state-field edge is covered by the
                    // class-local compiler recipe, not every swift_release.
                    if pin.role=="ivar_destructor_CTo"&&pc==0x1fd7a0&&target==0x2fc1ac {continue;}
                    if pin.role=="ivar_destructor_CTo"&&pc==0x1fd7b0&&target==0x2fc1ac {continue;}
                    if pin.role=="ivar_destructor_CTo"&&pc==0x1fd7c0&&target==0x2fc1ac {continue;}
                    // Two original NSObject controllers have fixed lifecycle
                    // contracts; a non-nil status item must pass its actual
                    // same-arena SDK allocation/family check before cleanup.
                    if pin.role=="destructor_objc_field_release_tail"&&pc==0x1ffb68&&target==0x2fbb28 {continue;}
                    if pin.role=="ivar_destructor_CTo"&&pc==0x1fd7fc&&target==0x2fbfa8 {continue;}
                    let inside_current=target>=pin.offset&&target<pin.offset+pin.bytes;
                    let known_entry=pins::PINS.iter().any(|known|target==known.offset)
                        ||super::app_delegate_code::CODE.iter().any(|known|target==known.offset);
                    if !inside_current&&!known_entry {
                        return Err(format!("Original callback secondary edge {}+0x{:x} -> Main+0x{:x} has no typed source closure",pin.role,index*4,target));
                    }
                } else if matches!(word&0xfffffc1f,0xd63f0000|0xd61f0000) {
                    return Err(format!("Original callback indirect Swift/Notification witness at Main+0x{pc:x} lacks actual typed target/value custody"));
                }
            }
        }
        self.pure()?;
        self.require_memory_disposal_contract()
    }
    fn verify_notification_thunk(&self)->Result<()> {
        // Fixed synchronous code/metadata observations only; no original
        // Notification thunk, Swift witness or ARC operation is invoked here.
        unsafe{self.data.context.with_callback_source_observation(|| {
        self.notification.verify_metadata_transition(0x1fd564,0x2f8c24)?;
        self.notification.verify_compiler_value_lifetime()?;
        self.verify_notification_stack_probe()?;
        self.data.system.verify_notification_runtime(&self.data.context)?;
        self.verify_nonfinal_callback_arc()?;
        self.notification.verify_compiler_value_lifetime()?;
        self.data.system.verify_notification_runtime(&self.data.context)?;
        self.pure()
        })}
    }
    fn verify_control_thunks(&self)->Result<()> {
        // Keep this scope separate from controller/server checks: those can
        // reach the owned bootstrap guardian and must not inherit reuse.
        unsafe{self.data.context.with_callback_source_observation(|| {
        self.pure()?;
        let action=pins::PINS.iter().find(|p|p.role=="action_shared_CTo").ok_or("Original control CTo absent")?;
        if !pins::control_receiver_lifetime(action) {return Err("Original control receiver flow differs".into());}
        let mut count=0;
        for row in pins::ROWS.iter().filter(|row|!row.notification) {
            let entry=&pins::PINS[row.entry];let body=&pins::PINS[row.body];
            let qualified=if row.selector==c"quit" {pins::quit_receiver_lifetime(entry,body)}
                else {count+=1;pins::control_entry_body(row.selector,entry,body)};
            if !qualified {return Err("Original control CTo/selected body binding differs".into());}
            for pin in [entry,body] {
                self.data.context.verify_main_code(self.data.main_base.checked_add(pin.offset)
                    .ok_or("Control code address overflow")?,pin.bytes,pin.sha)?;
            }
        }
        if count!=7{return Err("Original control entry cohort differs".into());}
        for role in ["action_shared_CTo","quit_release_actor","control_release_actor"] {
            let pin=pins::PINS.iter().find(|p|p.role==role).ok_or("Control executor lifetime member absent")?;
            self.data.context.verify_main_code(self.data.main_base.checked_add(pin.offset)
                .ok_or("Control executor lifetime address overflow")?,pin.bytes,pin.sha)?;
        }
        // Same genuine MainActor metadata/conformance and runtime imports as
        // the Notification CTo. The compiler keeps its executor reference;
        // Rust calls only the complete selected Objective-C entry, C-unwind.
        self.data.system.verify_notification_runtime(&self.data.context)?;
        self.verify_nonfinal_callback_arc()?;
        self.data.system.verify_notification_runtime(&self.data.context)?;
        self.pure()
        })}
    }
    fn verify_nonfinal_callback_arc(&self)->Result<()> {
        self.arc.verify()?;
        // The CTo's own retain/release of self is balanced and nonfinal: the
        // selected ObjectiveC caller holds its physical receiver +1 throughout
        // dispatch. No permission for final AppDelegate/child ARC follows.
        let rows=crate::sdk::imports::observe_unpatched_main_imports(&[
            (c"objc_retain",objc2::ffi::objc_retain as *const () as usize),
            (c"objc_release",objc2::ffi::objc_release as *const () as usize),
        ])?;
        for (name,cell,expected) in [(c"objc_retain",0x3c6960usize,&self.arc.retain_entry),
            (c"objc_release",0x3c6958,&self.arc.release_entry)] {
            let row=rows.iter().find(|row|row.symbol==name).ok_or("Callback nonfinal ARC import absent")?;
            if self.data.main_base.checked_add(cell)!=Some(row.cell_address())
                ||metadata::code_identity(row.original_address())?!=*expected {
                return Err("Callback nonfinal self ARC binding differs".into());
            }
            row.verify_unpatched()?;
        }
        self.data.context.verify_main_code(self.data.main_base.checked_add(0x2fbb28)
            .ok_or("Callback nonfinal ARC stub address overflow")?,24,
            "7ad93404e5101a05313301e7a1733176cd0c43963f38eb96ea1ed448719c0ace")?;
        self.arc.verify()?;
        for row in rows {row.verify_unpatched()?;}
        self.pure()
    }
    fn verify_notification_stack_probe(&self)->Result<()> {
        self.pure()?;
        // The original Ma's actual VWT-size has already been bounded. The
        // pinned compiler thunk loads that size into x9; its own stack remains
        // compiler-owned throughout. No Rust Swift-ABI/value call is issued.
        self.notification.verify()?;
        let base=self.data.main_base;
        if base.checked_add(0x3c4370)!=Some(self.stack_probe.cell_address())
            ||self.stack_probe_code.image()!=b"/usr/lib/system/libsystem_pthread.dylib" {
            return Err("Notification stack probe cell/provider differs".into());
        }
        self.stack_probe.verify_unpatched()?;
        if metadata::code_identity(self.stack_probe.original_address())?!=self.stack_probe_code {
            return Err("Notification stack probe original code identity changed".into());
        }
        self.data.context.verify_main_code(base.checked_add(0x1fd56c)
            .ok_or("Notification stack probe caller address overflow")?,24,
            "2804885fc4e8d71cf26af7295eb662814bf8f3d73199866df72c00175612f7ba")?;
        self.notification.verify()?;
        self.stack_probe.verify_unpatched()?;
        if metadata::code_identity(self.stack_probe.original_address())?!=self.stack_probe_code {
            return Err("Notification stack probe provider changed during qualification".into());
        }
        self.pure()
    }
    fn require_memory_disposal_contract(&self)->Result<()> {
        self.controller_cleanup.verify(&self.data,&self.state_cleanup,&self.server_cleanup)?;
        self.subscriptions_memory.verify(&self.data)?;
        // Typed compiler-owned memory is separate from owner quiescence,
        // nonempty cancellation and callback drain. Reference runtime release
        // remains refused until its actual operational retirement is supplied.
        let destructor=pins::PINS.iter().find(|pin|pin.role=="ivar_destructor_CTo").ok_or("Actual destructor pin absent")?;
        let word=*destructor.words.last().ok_or("Actual destructor body empty")?;
        if word&0xfc000000!=0x14000000{return Err("Original Set disposal tail is not the pinned branch".into());}
        let pc=destructor.offset+destructor.bytes-4;
        let delta=((word&0x03ffffff) as i32)<<6>>4;
        let target=pc.checked_add_signed(delta as isize).ok_or("Set disposal tail overflow")?;
        if pc!=0x1fd7fc||target!=0x2fbfa8{return Err("Original subscriptions disposal tail differs".into());}
        self.pure()
    }
}
// No callback/cleanup method can return Ok merely from the entry-evidence
// checks. Both real secondary and final-disposal closure checks are required.
unsafe impl super::super::super::super::delegate::CallbackCodeAuthority for AppDelegateCallbackAuthority {
    fn verify_source_construction(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.qualify(|| {
            if class as *const AnyClass as usize!=self.data.class.address()||source!=self.data.context.constructor_source(){return Err("AppDelegate constructor source differs".into());}
            self.data.context.verify_model(&self.data.model)?;self.pure()?;
            self.data.verify_bodies_and_layout()?;self.pure()
        })
    }
    fn verify_method(&self,class:&AnyClass,selector:Sel,imp:usize,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.verify_methods(class, &[(selector.name(), imp)], source)
    }
    fn verify_methods(&self, class: &AnyClass, methods: &[(&CStr, usize)],
        source: &crate::sdk::SourceCallsite) -> Result<()> {
        self.binding(class,source)?;
        self.verify_entry_evidence()?;
        if methods.is_empty() {
            return Err("AppDelegate callback method roster empty".into());
        }
        for &(selector, implementation) in methods {
            let index = pins::ROWS.iter().position(|row| row.selector == selector)
                .ok_or("AppDelegate callback is not one of the fixed ten")?;
            if self.methods[index].implementation() != implementation {
                return Err("AppDelegate selected original IMP differs".into());
            }
        }
        self.qualify(||self.require_callback_and_disposal_contract())
    }
    fn verify_cleanup(&self,class:&AnyClass,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.binding(class,source)?;self.verify_evidence(false)?;
        self.qualify(||self.require_callback_and_disposal_contract())
    }
    fn verify_receiver_cleanup(&self,receiver:&ReceiverIdentity,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.pure()?;
        if !receiver.belongs_to(&self.data.model)||!receiver.original_source(){return Err("AppDelegate cleanup receiver belongs to another issuer".into());}
        receiver.verify_allocation()?;self.verify_cleanup(receiver.class(),source)?;
        receiver.verify_allocation()?;
        if unsafe{&*(receiver.receiver() as *mut AnyObject)}.class()!=receiver.class(){return Err("AppDelegate cleanup physical class changed".into());}
        self.controller_cleanup.verify_receiver_fields(&self.data,receiver)?;
        self.pure()
    }
    fn verify_source_partial_cleanup(&self,receiver:&SourceConstructionIdentity<'_>,source:&crate::sdk::SourceCallsite)->Result<()> {
        self.pure()?;
        if !receiver.belongs_to(&self.data.model)||receiver.class() as *const AnyClass as usize!=self.data.class.address()
            ||source!=self.data.context.constructor_source(){return Err("AppDelegate partial cleanup constructor binding differs".into());}
        let admission=receiver.ledger.construction.get().ok_or("Actual partial construction admission absent")?;
        if admission.constructor_source!=*source||admission.app_delegate.as_ref().is_none_or(|declaration|!Arc::ptr_eq(&declaration.data.context,&self.data.context)) {
            return Err("Partial cleanup uses another constructor/context issuer".into());
        }
        receiver.verify_allocation()?;
        if !matches!(receiver.phase(),ReceiverPhase::Allocated|ReceiverPhase::SourceIvarsObserved) {
            return Err("Original partial cleanup cannot adopt an initializer-in-flight phase".into());
        }
        let observed={
            let rows=receiver.ledger.receivers.lock().map_err(|_|"Partial cleanup ledger poisoned")?;
            let row=rows.get(&receiver.receiver()).ok_or("Partial cleanup allocation absent")?;
            let initialized=row.app_delegate.as_ref().ok_or("Original partial cleanup has no typed seven-field ownership witness")?;
            if initialized.reference!=ConstructionReference::OwnedReturn {
                return Err("Original partial cleanup has no returned construction ownership".into());
            }
            initialized.fields.clone().ok_or("Original partial cleanup has no initialized-field witness")?
        };
        // Separate conjuncts: the real caller's constructor219 binding above
        // AND this exact class's callback158 cleanup/source closure. Neither
        // source row is accepted as an alias for the other.
        self.verify_cleanup(receiver.class(),self.data.context.callback_source())?;
        self.data.singleton.verify_initialized_word(observed.subscriptions)?;
        receiver.verify_allocation()?;
        let pointer=receiver.receiver() as *const AnyObject;
        if unsafe{&*pointer}.class()!=receiver.class(){return Err("Partial cleanup physical class differs".into());}
        let mut words=[0usize;7];
        for (word,offset) in words.iter_mut().zip([8usize,16,24,32,40,48,56]) {*word=unsafe{pointer.cast::<u8>().add(offset).cast::<usize>().read()};}
        if words[..6]!=observed.optional||words[6]!=observed.subscriptions {
            return Err("Partial cleanup physical fields differ from the same typed witness".into());
        }
        receiver.verify_allocation()?;self.pure()
    }
    fn verify_sender(&self,class:&AnyClass,selector:Sel,imp:usize,source:&crate::sdk::SourceCallsite,sender:&AnyClass)->Result<()> {
        self.verify_method(class,selector,imp,source)?;
        if !matches!(selector.name().to_bytes(),b"applicationDidFinishLaunching:"|b"applicationWillTerminate:") {
            return Err("This AppDelegate action declares no sender argument".into());
        }
        self.qualify(|| {
        let grant=super::super::super::super::factory::grant_for(sender)?;
        self.pure()?;
        if grant.original_source||!Arc::ptr_eq(&grant.model,&self.data.model){return Err("Notification sender is not a same-model owned payload class".into());}
        let notification=metadata::loaded_class(c"NSNotification")?;
        if sender.superclass().map(|parent|parent as *const AnyClass as usize)!=Some(notification.address()) {
            return Err("Notification sender is not the exact owned notification parent projection".into());
        }
        let leaves=grant.attested_owned_leaves()?;
        self.pure()?;
        for selector in [c"name",c"object",c"userInfo"] {
            if !leaves.iter().any(|leaf|leaf.selector==selector&&leaf.class==grant.class.name()&&!leaf.class_method){return Err("Notification sender lacks its actual typed payload leaf".into());}
        }
        self.evidence_body(false)?;self.pure()
        })
    }
}
