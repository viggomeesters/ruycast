// Source-included data-only endpoint. Compile with rustc --test and producer
// pins; the ordinary Cargo example cannot load the SDK or execute this route.
#![cfg_attr(test, no_main)]

#[cfg(not(test))]
fn main() {
    std::process::exit(2);
}

#[cfg(test)]
#[path = "../src/sdk/data_family_preentry.rs"]
pub mod data_family_preentry;
#[cfg(test)]
#[path = "../src/sdk/mod.rs"]
pub mod sdk_facade;
#[cfg(test)]
pub mod sdk {
    pub use crate::data_family_preentry;
    pub use crate::sdk_facade::*;
}
#[cfg(test)]
pub use sdk::{metadata, owned};
#[cfg(test)]
#[path = "../src/guardian.rs"]
pub mod guardian;
#[cfg(test)]
#[path = "../src/guardian_alive.rs"]
pub mod guardian_alive;
#[cfg(test)]
#[path = "../src/sdk/workspace.rs"]
pub mod workspace;

// Producer-generated Rust literals only. No runtime environment/file values
// construct Pins or authority. The producer binds this include as source.
#[cfg(test)]
include!(env!("F1_DATA_PINS_SOURCE"));

#[cfg(test)]
fn run() -> owned::Result<()> {
    let pins = make_data_pins();
    // First source-owned effect; no logging, lookup, fixture or model before it.
    let admission = sdk::data_family_preentry::enter(&pins)?;
    let fixture = admission.fixture();
    let context = owned::ModuleContext {
        home: fixture.clone(),
        cache: fixture.clone(),
        temp: fixture.clone(),
        applications: fixture,
        life: admission.life(),
    };
    let frame = workspace::Rect {
        origin: workspace::Point { x: 0.0, y: 0.0 },
        size: workspace::Point { x: 64.0, y: 64.0 },
    };
    let model = workspace::Workspace::new(
        context,
        vec![],
        vec![workspace::Screen {
            frame,
            visible: frame,
            scale: 1.0,
        }],
        workspace::Point { x: 0.0, y: 0.0 },
    )?;
    admission.prepare(model.clone())?;
    prove_typed_normal_arc()?;
    if model.frontmost()?.is_some() {
        return Err("unexpected application in empty admitted model".into());
    }
    Ok(())
}

#[cfg(test)]
fn prove_typed_normal_arc() -> owned::Result<()> {
    use objc2::{
        msg_send,
        rc::{autoreleasepool, Retained, Weak},
        runtime::{AnyClass, AnyObject, Bool},
    };
    // These are registered-class metadata identities after sealed admission,
    // never an original instance or a resolving Foundation factory lookup.
    let family = metadata::loaded_class(c"NSAppleEventDescriptor")?;
    let family = unsafe { &*(family.address() as *const AnyClass) };
    for _ in 0..2 {
        let (retained, weak, proxy, baseline) = autoreleasepool(|_| -> owned::Result<_> {
            let value: *mut AnyObject = unsafe { msg_send![family, descriptorWithInt32: 7_i32] };
            let retained = unsafe { Retained::retain(value) }.ok_or("nil typed descriptor")?;
            let typed: Bool = unsafe { msg_send![&*retained, isKindOfClass: family] };
            if !typed.as_bool() || retained.class() == family {
                return Err("descriptor is not the owned type-preserving subclass".into());
            }
            let proxy = retained.class();
            let current = workspace::factory::live_owned_allocations(proxy)?;
            let baseline = current
                .checked_sub(1)
                .ok_or("descriptor allocation absent")?;
            if baseline != 0 {
                return Err("unexpected pre-existing descriptor allocation".into());
            }
            let weak = Weak::from_retained(&retained);
            Ok((retained, weak, proxy, baseline))
        })?;
        let value: i32 = unsafe { msg_send![&*retained, int32Value] };
        if value != 7 || weak.load().is_none() {
            return Err("retained +0 descriptor did not survive pool drain".into());
        }
        drop(retained);
        if weak.load().is_some() || workspace::factory::live_owned_allocations(proxy)? != baseline {
            return Err("last owner did not reclaim the typed descriptor".into());
        }
        // An init-family result remains +1 across a pool, then releases once.
        let initialized = autoreleasepool(|_| -> owned::Result<_> {
            let allocated = unsafe { workspace::factory::allocate(proxy) }?;
            let number = 9_i32;
            let initialized: *mut AnyObject = unsafe {
                msg_send![allocated, initWithDescriptorType: u32::from_be_bytes(*b"long"),
                    bytes: (&number as *const i32).cast::<std::ffi::c_void>(), length: 4_usize]
            };
            if initialized != allocated {
                return Err("typed initializer changed its +1 receiver".into());
            }
            Ok(initialized)
        })?;
        let value: i32 = unsafe { msg_send![initialized, int32Value] };
        if value != 9 {
            return Err("typed +1 initializer did not survive pool drain".into());
        }
        let initialized_weak = Weak::new(unsafe { &*initialized });
        unsafe { workspace::factory::release_owned(initialized) }?;
        if initialized_weak.load().is_some()
            || workspace::factory::live_owned_allocations(proxy)? != baseline
        {
            return Err("typed +1 initializer allocation was not reclaimed".into());
        }
        let allocated = unsafe { workspace::factory::allocate(proxy) }?;
        let rejected_weak = Weak::new(unsafe { &*allocated });
        let rejected: *mut AnyObject = unsafe {
            msg_send![allocated, initWithDescriptorType: u32::from_be_bytes(*b"long"),
                bytes: std::ptr::null::<std::ffi::c_void>(), length: 1_usize]
        };
        if !rejected.is_null()
            || rejected_weak.load().is_some()
            || workspace::factory::live_owned_allocations(proxy)? != baseline
        {
            return Err("failed typed initializer did not consume its allocated +1".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    match std::panic::catch_unwind(|| match run() {
        Ok(()) => 0,
        Err(error) => {
            use std::io::Write;
            let _ = writeln!(
                std::io::stdout().lock(),
                "{}",
                serde_json::json!({ "ok": false, "error": error })
            );
            1
        }
    }) {
        Ok(status) => status,
        Err(payload) => {
            // A foreign panic payload may itself panic when dropped. The
            // isolated failing process exits; never unwind across this C ABI.
            std::mem::forget(payload);
            2
        }
    }
}
