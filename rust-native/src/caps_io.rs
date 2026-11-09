//! Typed IOKit Caps boundary. Loading this table does not open a connection.
//! Fixture mode requires every method and never uses a real SDK fallback.
use crate::caps_lock::{Error, Io, Recovery};
use objc2::MainThreadMarker;
use std::cell::{Cell, RefCell};
use std::{ffi::c_void, marker::PhantomData, rc::Rc};

type Matching = unsafe extern "C" fn(*const libc::c_char) -> *mut c_void;
type Service = unsafe extern "C" fn(u32, *mut c_void) -> u32;
type Open = unsafe extern "C" fn(u32, u32, u32, *mut u32) -> i32;
type Release = unsafe extern "C" fn(u32) -> i32;
type Get = unsafe extern "C" fn(u32, i32, *mut bool) -> i32;
type Set = unsafe extern "C" fn(u32, i32, bool) -> i32;
type Task = unsafe extern "C" fn() -> u32;
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const libc::c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main: u32, matching: *mut c_void) -> u32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
    fn IOHIDGetModifierLockState(connection: u32, selector: i32, state: *mut bool) -> i32;
    fn IOHIDSetModifierLockState(connection: u32, selector: i32, state: bool) -> i32;
    static mach_task_self_: u32;
}
unsafe extern "C" fn task() -> u32 {
    unsafe { mach_task_self_ }
}
#[derive(Clone)]
pub struct Native {
    matching: Matching,
    service: Service,
    open: Open,
    release: Release,
    close: Release,
    get: Get,
    set: Set,
    task: Task,
    ownership: Rc<Cell<Option<u64>>>,
    _main: PhantomData<Rc<()>>,
}
struct Pending {
    io: Native,
    recovery: Recovery,
}
thread_local! {
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static RECOVERING: Cell<bool> = const { Cell::new(false) };
    static OWNER: Cell<Option<u64>> = const { Cell::new(None) };
    static NEXT_OWNER: Cell<u64> = const { Cell::new(0) };
}
struct Reservation {
    token: u64,
    ownership: Rc<Cell<Option<u64>>>,
    release: bool,
}
impl Reservation {
    fn acquire(ownership: &Rc<Cell<Option<u64>>>) -> Result<Self, i32> {
        if OWNER.with(|owner| owner.get().is_some()) {
            return Err(-5);
        }
        let token = NEXT_OWNER.with(|next| {
            let token = next.get().checked_add(1).ok_or(-6)?;
            next.set(token);
            Ok::<u64, i32>(token)
        })?;
        OWNER.with(|owner| owner.set(Some(token)));
        ownership.set(Some(token));
        Ok(Self {
            token,
            ownership: ownership.clone(),
            release: true,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.release {
            OWNER.with(|owner| {
                if owner.get() == Some(self.token) {
                    owner.set(None);
                }
            });
            if self.ownership.get() == Some(self.token) {
                self.ownership.set(None);
            }
        }
    }
}
impl Native {
    /// Retry final-owner quarantines on the main thread, without borrowing the
    /// queue across SDK calls. New acquisitions remain blocked during reentry.
    pub fn retry_recovery(_main: MainThreadMarker) -> Result<(), Error> {
        if RECOVERING.with(|busy| busy.replace(true)) {
            return Err(Error::Busy);
        }
        let pending = PENDING.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        let mut error = None;
        for mut value in pending {
            if value.recovery.dirty {
                let original = value
                    .recovery
                    .original
                    .expect("dirty quarantine has original");
                if let Err(code) = value.io.set(value.recovery.handle, original) {
                    error = Some(Error::Restore(code));
                    PENDING.with(|queue| queue.borrow_mut().push(value));
                    continue;
                }
                value.recovery.dirty = false;
            }
            if let Err(code) = value.io.close(value.recovery.handle) {
                error = Some(Error::Close(code));
            }
        }
        RECOVERING.with(|busy| busy.set(false));
        if let Some(error) = error {
            Err(error)
        } else if PENDING.with(|queue| !queue.borrow().is_empty()) {
            Err(Error::RecoveryPending)
        } else {
            Ok(())
        }
    }
    pub fn recovery_pending(_main: MainThreadMarker) -> bool {
        RECOVERING.with(Cell::get) || PENDING.with(|queue| !queue.borrow().is_empty())
    }
    pub fn load(_main: MainThreadMarker, fixture: bool) -> Result<Self, String> {
        if !fixture {
            return Ok(Self {
                matching: IOServiceMatching,
                service: IOServiceGetMatchingService,
                open: IOServiceOpen,
                release: IOObjectRelease,
                close: IOServiceClose,
                get: IOHIDGetModifierLockState,
                set: IOHIDSetModifierLockState,
                task,
                ownership: Rc::new(Cell::new(None)),
                _main: PhantomData,
            });
        }
        #[cfg(feature = "test-tools")]
        unsafe {
            let ready = libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_caps_fixture_ready".as_ptr());
            let pointers = [
                c"ruycast_caps_fixture_matching",
                c"ruycast_caps_fixture_service",
                c"ruycast_caps_fixture_open",
                c"ruycast_caps_fixture_release",
                c"ruycast_caps_fixture_close",
                c"ruycast_caps_fixture_get",
                c"ruycast_caps_fixture_set",
                c"ruycast_caps_fixture_task",
            ]
            .map(|name| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()));
            if ready.is_null() || pointers.iter().any(|p| p.is_null()) {
                return Err("Complete Caps SDK fixture unavailable.".into());
            }
            if std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> u64>(ready)()
                != 0x5255434150533031
            {
                return Err("Complete Caps SDK fixture unavailable.".into());
            }
            Ok(Self {
                matching: std::mem::transmute::<*mut c_void, Matching>(pointers[0]),
                service: std::mem::transmute::<*mut c_void, Service>(pointers[1]),
                open: std::mem::transmute::<*mut c_void, Open>(pointers[2]),
                release: std::mem::transmute::<*mut c_void, Release>(pointers[3]),
                close: std::mem::transmute::<*mut c_void, Release>(pointers[4]),
                get: std::mem::transmute::<*mut c_void, Get>(pointers[5]),
                set: std::mem::transmute::<*mut c_void, Set>(pointers[6]),
                task: std::mem::transmute::<*mut c_void, Task>(pointers[7]),
                ownership: Rc::new(Cell::new(None)),
                _main: PhantomData,
            })
        }
        #[cfg(not(feature = "test-tools"))]
        Err("Caps fixtures require test-tools.".into())
    }
}
fn result(code: i32) -> Result<(), i32> {
    if code == 0 {
        Ok(())
    } else {
        Err(code)
    }
}
impl Io for Native {
    fn open(&self) -> Result<u32, i32> {
        if RECOVERING.with(Cell::get) || PENDING.with(|queue| !queue.borrow().is_empty()) {
            return Err(-4);
        }
        let mut reservation = Reservation::acquire(&self.ownership)?;
        unsafe {
            let matching = (self.matching)(c"IOHIDSystem".as_ptr());
            if matching.is_null() {
                return Err(-1);
            }
            // The matching lookup consumes the dictionary, even on failure.
            let service = (self.service)(0, matching);
            if service == 0 {
                return Err(-2);
            }
            let mut connection = 0;
            let opened = (self.open)(service, (self.task)(), 1, &mut connection);
            let released = (self.release)(service);
            // A connection is returned on success only. Never use an undefined
            // output from a failed IOServiceOpen as an owned Mach port.
            if opened != 0 {
                return Err(opened);
            }
            if connection == 0 {
                return Err(-3);
            }
            if released != 0 {
                if let Err(error) = self.close(connection) {
                    self.abandoned(Error::Close(error), None);
                }
                return Err(released);
            }
            reservation.release = false;
            Ok(connection)
        }
    }
    fn get(&self, connection: u32) -> Result<bool, i32> {
        let mut value = false;
        result(unsafe { (self.get)(connection, 1, &mut value) })?;
        Ok(value)
    }
    fn set(&self, connection: u32, value: bool) -> Result<(), i32> {
        result(unsafe { (self.set)(connection, 1, value) })
    }
    fn close(&self, connection: u32) -> Result<(), i32> {
        let token = self.ownership.get().ok_or(-7)?;
        // Keep the process latch reserved through physical close, including
        // callbacks during close; only the exact owner token may release it.
        let _retiring = Reservation {
            token,
            ownership: self.ownership.clone(),
            release: true,
        };
        result(unsafe { (self.close)(connection) })
    }
    fn abandoned(&self, error: Error, recovery: Option<Recovery>) {
        if let Some(recovery) = recovery {
            PENDING.with(|queue| {
                queue.borrow_mut().push(Pending {
                    io: self.clone(),
                    recovery,
                })
            });
        }
        eprintln!("Caps Lock cleanup failed: {error:?}");
    }
}
