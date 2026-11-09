//! Main-thread native system effects. Test keyboard tables replace every SDK
//! call before event construction; merely constructing a table has no effect.
use crate::system_search::Ticket;
use std::{ffi::c_void, path::PathBuf};
type Ptr = *mut c_void;
#[derive(Clone, Debug)]
pub enum Effect {
    ActivityMonitor,
    Configuration(PathBuf),
    Lock,
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(source: Ptr, key: u16, down: bool) -> Ptr;
    fn CGEventSetFlags(event: Ptr, flags: u64);
    fn CGEventSetIntegerValueField(event: Ptr, field: u32, value: i64);
    fn CGEventPost(location: u32, event: Ptr);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ptr);
}
struct Keyboard {
    trusted: unsafe extern "C" fn() -> u8,
    create: unsafe extern "C" fn(Ptr, u16, bool) -> Ptr,
    flags: unsafe extern "C" fn(Ptr, u64),
    field: unsafe extern "C" fn(Ptr, u32, i64),
    post: unsafe extern "C" fn(u32, Ptr),
    release: unsafe extern "C" fn(Ptr),
}
impl Keyboard {
    fn production() -> Self {
        Self {
            trusted: AXIsProcessTrusted,
            create: CGEventCreateKeyboardEvent,
            flags: CGEventSetFlags,
            field: CGEventSetIntegerValueField,
            post: CGEventPost,
            release: CFRelease,
        }
    }
    fn lock(&self, ticket: &Ticket) -> Result<(), String> {
        ticket.check()?;
        if unsafe { (self.trusted)() } == 0 {
            return Err("Allow Accessibility access to lock the screen.".into());
        }
        struct Owned<'a> {
            value: Ptr,
            io: &'a Keyboard,
        }
        impl Drop for Owned<'_> {
            fn drop(&mut self) {
                if !self.value.is_null() {
                    unsafe {
                        (self.io.release)(self.value);
                    }
                }
            }
        }
        let down = Owned {
            value: unsafe { (self.create)(std::ptr::null_mut(), 12, true) },
            io: self,
        };
        let up = Owned {
            value: unsafe { (self.create)(std::ptr::null_mut(), 12, false) },
            io: self,
        };
        if down.value.is_null() || up.value.is_null() {
            return Err("Could not lock the screen.".into());
        }
        for event in [down.value, up.value] {
            unsafe {
                (self.flags)(event, 0x100000 | 0x40000);
                (self.field)(event, 42, crate::keyboard_state::GENERATED_HYPER);
            }
        }
        ticket.check()?;
        // Once keydown is posted, keyup must be paired even if cancellation
        // arrives in between. Posting is dispatch, not proof of physical lock.
        unsafe {
            (self.post)(0, down.value);
            (self.post)(0, up.value);
        }
        Ok(())
    }
}
/// The legacy process, when present, is collected on the bounded action worker.
/// AppKit's main thread only handles the immediate native keyboard dispatch.
pub fn legacy_lock(ticket: &Ticket) -> Result<bool, String> {
    ticket.check()?;
    let session =
        c"/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession";
    if unsafe { libc::access(session.as_ptr(), libc::X_OK) } != 0 {
        return Ok(false);
    }
    if objc2::MainThreadMarker::new().is_some() {
        return Err("Legacy screen locking requires the action worker.".into());
    }
    crate::os_command::stream(
        session.to_str().map_err(|_| "Invalid lock command")?,
        &["-suspend"],
        ticket,
        65536,
        false,
        |_| false,
    )
    .map_err(|_| "Screen lock result is unknown. Check before retrying.".to_string())?;
    Ok(true)
}
pub fn perform(effect: Effect, ticket: &Ticket) -> Result<(), String> {
    let _main = objc2::MainThreadMarker::new().ok_or("System effects require the main thread.")?;
    ticket.check()?;
    match effect {
        Effect::Lock => Keyboard::production().lock(ticket),
        Effect::ActivityMonitor | Effect::Configuration(_) => {
            let path = match effect {
                Effect::Configuration(path) => path,
                _ => PathBuf::from("/System/Applications/Utilities/Activity Monitor.app"),
            };
            let path = path
                .to_str()
                .filter(|s| !s.contains('\0'))
                .ok_or("Cannot open an invalid configuration path.")?;
            if !std::path::Path::new(path).is_absolute() {
                return Err("Choose an absolute configuration path.".into());
            }
            let url = objc2_foundation::NSURL::fileURLWithPath(
                &objc2_foundation::NSString::from_str(path),
            );
            ticket.check()?;
            if objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url) {
                Ok(())
            } else {
                Err("macOS could not open the requested item.".into())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        sync::Arc,
        time::{Duration, Instant},
    };
    #[derive(Default)]
    struct State {
        events: Vec<String>,
        deny: bool,
        fail_up: bool,
    }
    thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());}
    unsafe extern "C" fn trusted() -> u8 {
        STATE.with(|s| u8::from(!s.borrow().deny))
    }
    unsafe extern "C" fn create(_: Ptr, key: u16, down: bool) -> Ptr {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.events.push(format!("create:{key}:{down}"));
            if !down && s.fail_up {
                std::ptr::null_mut()
            } else {
                (if down { 1usize } else { 2usize }) as Ptr
            }
        })
    }
    unsafe extern "C" fn flags(_: Ptr, value: u64) {
        STATE.with(|s| s.borrow_mut().events.push(format!("flags:{value}")));
    }
    unsafe extern "C" fn field(_: Ptr, field: u32, value: i64) {
        STATE.with(|s| s.borrow_mut().events.push(format!("field:{field}:{value}")));
    }
    unsafe extern "C" fn post(_: u32, event: Ptr) {
        STATE.with(|s| {
            s.borrow_mut()
                .events
                .push(format!("post:{}", event as usize))
        });
    }
    unsafe extern "C" fn release(event: Ptr) {
        STATE.with(|s| {
            s.borrow_mut()
                .events
                .push(format!("release:{}", event as usize))
        });
    }
    fn io() -> Keyboard {
        STATE.with(|s| *s.borrow_mut() = State::default());
        Keyboard {
            trusted,
            create,
            flags,
            field,
            post,
            release,
        }
    }
    fn ticket() -> Ticket {
        Ticket {
            deadline: Instant::now() + Duration::from_secs(1),
            live: Arc::new(|| true),
        }
    }
    #[test]
    fn complete_owned_keyboard_sdk_pairs_and_tags_lock_shortcut() {
        let io = io();
        io.lock(&ticket()).unwrap();
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(
                s.events
                    .iter()
                    .filter(|e| e.starts_with("post:"))
                    .cloned()
                    .collect::<Vec<_>>(),
                ["post:1", "post:2"]
            );
            assert_eq!(
                s.events
                    .iter()
                    .filter(|e| e.starts_with("release:"))
                    .count(),
                2
            );
            assert!(s.events.contains(&format!(
                "field:42:{}",
                crate::keyboard_state::GENERATED_HYPER
            )));
        });
    }
    #[test]
    fn denied_and_failed_construction_never_post() {
        let io = io();
        STATE.with(|s| s.borrow_mut().deny = true);
        assert!(io.lock(&ticket()).is_err());
        STATE.with(|s| assert!(s.borrow().events.is_empty()));
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.deny = false;
            s.fail_up = true;
        });
        assert!(io.lock(&ticket()).is_err());
        STATE.with(|s| {
            let s = s.borrow();
            assert!(!s.events.iter().any(|e| e.starts_with("post:")));
            assert!(s.events.contains(&"release:1".into()));
        });
    }
}
