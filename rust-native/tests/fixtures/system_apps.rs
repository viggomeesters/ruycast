//! Synthetic app/process identity only; never forward discovery or activation.
#![allow(
    dead_code,
    reason = "Typed callbacks are assembled into the complete SDK only in300"
)]
use super::system_nodes::{Ptr, State};
use objc2::{define_class, msg_send, rc::Retained, runtime::Bool, AnyThread, DefinedClass};
use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};
use std::sync::Arc;
define_class!(
    #[unsafe(super=NSObject)]
    #[ivars=Arc<State>]
    pub(super) struct OwnedSystemApp;
    unsafe impl NSObjectProtocol for OwnedSystemApp {}
    impl OwnedSystemApp {
        #[unsafe(method(processIdentifier))]
        fn fixture_pid(&self)->i32 {730001}
        #[unsafe(method(isTerminated))]
        fn terminated(&self)->Bool {Bool::NO}
        #[unsafe(method(localizedName))]
        fn name(&self)->*mut NSString {Retained::autorelease_return(NSString::from_str("Owned Native App"))}
        #[unsafe(method(bundleIdentifier))]
        fn bundle(&self)->*mut NSString {Retained::autorelease_return(NSString::from_str("com.apple.Safari"))}
        #[unsafe(method(bundleURL))]
        fn url(&self)->*mut NSURL {Retained::autorelease_return(NSURL::fileURLWithPath(&NSString::from_str("/Applications/Safari.app")))}
        #[unsafe(method(activationPolicy))]
        fn policy(&self)->NSApplicationActivationPolicy {NSApplicationActivationPolicy::Regular}
        #[unsafe(method(activateWithOptions:))]
        fn activate(&self,options:NSApplicationActivationOptions)->Bool {
            assert!(unsafe{libc::pthread_main_np()!=0} && options.is_empty());
            let mut data=self.ivars().data.lock().unwrap();data.effects+=1;
            if data.mode=="mutate-activate" {data.mode="renamed".into();}
            Bool::YES
        }
    }
);
pub(super) struct Apps {
    pub state: Arc<State>,
    pub app: Retained<OwnedSystemApp>,
    pub workspace: Retained<NSObject>,
}
impl Apps {
    pub fn new(state: Arc<State>) -> Self {
        let allocated = OwnedSystemApp::alloc().set_ivars(state.clone());
        let app: Retained<OwnedSystemApp> = unsafe { msg_send![super(allocated), init] };
        Self {
            state,
            app,
            workspace: NSObject::new(),
        }
    }
    pub fn shared(&self) -> Retained<NSObject> {
        self.workspace.clone()
    }
    pub fn running(&self) -> Retained<NSArray<OwnedSystemApp>> {
        self.state.data.lock().unwrap().reads += 1;
        NSArray::from_slice(&[&*self.app])
    }
    pub fn front(&self) -> Retained<OwnedSystemApp> {
        self.state.data.lock().unwrap().reads += 1;
        self.app.clone()
    }
    pub fn by_bundle(&self, bundle: &NSString) -> Retained<NSArray<OwnedSystemApp>> {
        self.state.data.lock().unwrap().reads += 1;
        assert!(bundle.isEqualToString(&NSString::from_str("com.apple.Safari")));
        NSArray::from_slice(&[&*self.app])
    }
    pub fn by_pid(&self, pid: i32) -> Retained<OwnedSystemApp> {
        assert_eq!(pid, 730001);
        self.app.clone()
    }
}
pub(super) unsafe fn proc(
    state: &State,
    pid: i32,
    flavor: i32,
    arg: u64,
    out: Ptr,
    size: i32,
) -> i32 {
    let required = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    if pid != 730001 || flavor != libc::PROC_PIDTBSDINFO || arg != 0 || size != required {
        return 0;
    }
    let mut info: libc::proc_bsdinfo = std::mem::zeroed();
    info.pbi_pid = pid as u32;
    info.pbi_uid = libc::getuid();
    info.pbi_start_tvsec = if state.data.lock().unwrap().mode == "restarted" {
        12
    } else {
        11
    };
    info.pbi_start_tvusec = 22;
    std::ptr::copy_nonoverlapping(
        (&info as *const libc::proc_bsdinfo).cast::<u8>(),
        out.cast(),
        required as usize,
    );
    required
}
