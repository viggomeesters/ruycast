//! Owned nonactivating AppKit app. No windows, documents, helpers, IO or capture.
//! Its only native behavior is accepting, refusing or deferring a normal quit.
use objc2::{
    define_class, msg_send, runtime::ProtocolObject, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply,
};
use objc2_foundation::{NSObject, NSObjectProtocol};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
struct State {
    mode: String,
    receipt: PathBuf,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    #[ivars=State]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            let mut receipt = OpenOptions::new()
                .append(true)
                .open(&self.ivars().receipt)
                .unwrap();
            writeln!(receipt, "quit:{}", self.ivars().mode).unwrap();
            receipt.sync_all().unwrap();
            match self.ivars().mode.as_str() {
                "accept" => NSApplicationTerminateReply::TerminateNow,
                "crash" => std::process::exit(37),
                "refuse" => NSApplicationTerminateReply::TerminateCancel,
                "defer" => NSApplicationTerminateReply::TerminateLater,
                _ => unreachable!(),
            }
        }
    }
);
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3);
    assert!(["accept", "refuse", "defer", "crash"].contains(&args[1].as_str()));
    let receipt = PathBuf::from(&args[2]);
    assert!(receipt.is_absolute());
    let main = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(main);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    assert_eq!(
        app.activationPolicy(),
        NSApplicationActivationPolicy::Accessory
    );
    let allocated = Delegate::alloc(main).set_ivars(State {
        mode: args[1].clone(),
        receipt: receipt.clone(),
    });
    let delegate: objc2::rc::Retained<Delegate> = unsafe { msg_send![super(allocated), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.finishLaunching();
    // Announce readiness from the running main event loop, after AppKit has
    // registered its event receiver. Merely constructing NSApplication raced
    // the first external normal-quit request in the native proof.
    unsafe extern "C" fn ready(context: *mut std::ffi::c_void) {
        let receipt = unsafe { Box::from_raw(context.cast::<PathBuf>()) };
        std::fs::write(&*receipt, "ready\n").unwrap();
    }
    unsafe extern "C" {
        fn dispatch_async_f(
            queue: *mut std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: unsafe extern "C" fn(*mut std::ffi::c_void),
        );
        static _dispatch_main_q: std::ffi::c_void;
    }
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q).cast_mut(),
            Box::into_raw(Box::new(receipt)).cast(),
            ready,
        );
    }
    // Cleanup affects this child only, never another PID. The watchdog bounds
    // failures even if the parent disappears; no kill/force-quit API is used.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        std::process::exit(80);
    });
    std::thread::spawn(|| {
        let mut byte = [0];
        let _ = std::io::stdin().read(&mut byte);
        std::process::exit(0);
    });
    app.run();
}
