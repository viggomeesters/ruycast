//! Estimated first display refresh after a ready, visible launcher opening.
//!
//! A one-shot main run-loop observer ordered after Core Animation's commit
//! observer marks the commit. A display link then reports the output time of
//! the first refresh whose callback starts after that commit. The sample is
//! `opening receipt -> that output time`. WindowServer may composite one frame
//! later, and panel brightness/scanout are not observed: this is an estimate,
//! not photon latency. No window content, query or input is read.
use std::{
    ffi::c_void,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Abandon an estimate that has not resolved within this time.
const LIMIT: Duration = Duration::from_secs(1);
/// `kCFRunLoopBeforeWaiting | kCFRunLoopExit`, as Core Animation observes.
const ACTIVITIES: u64 = (1 << 5) | (1 << 7);
/// Core Animation's commit observer uses order 2,000,000.
const AFTER_COMMIT: isize = 2_000_001;

#[repr(C)]
#[allow(dead_code)]
struct SmpteTime {
    subframes: i16,
    subframe_divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}
/// Exact public `CVTimeStamp` ABI.
#[repr(C)]
#[allow(dead_code)]
struct TimeStamp {
    version: u32,
    video_time_scale: i32,
    video_time: i64,
    host_time: u64,
    rate_scalar: f64,
    video_refresh_period: i64,
    smpte_time: SmpteTime,
    flags: u64,
    reserved: u64,
}
#[repr(C)]
struct ObserverContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
}
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}
type OutputCallback = unsafe extern "C" fn(
    *mut c_void,
    *const TimeStamp,
    *const TimeStamp,
    u64,
    *mut u64,
    *mut c_void,
) -> i32;
type ObserverCallback = unsafe extern "C" fn(*mut c_void, u64, *mut c_void);

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVDisplayLinkCreateWithActiveCGDisplays(out: *mut *mut c_void) -> i32;
    fn CVDisplayLinkSetOutputCallback(link: *mut c_void, callback: OutputCallback, info: *mut c_void) -> i32;
    fn CVDisplayLinkStart(link: *mut c_void) -> i32;
    fn CVDisplayLinkStop(link: *mut c_void) -> i32;
    fn CVDisplayLinkIsRunning(link: *mut c_void) -> u8;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopObserverCreate(
        allocator: *const c_void,
        activities: u64,
        repeats: u8,
        order: isize,
        callout: ObserverCallback,
        context: *mut ObserverContext,
    ) -> *mut c_void;
    fn CFRunLoopAddObserver(run_loop: *mut c_void, observer: *mut c_void, mode: *const c_void);
    fn CFRelease(value: *mut c_void);
}
extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: unsafe extern "C" fn(*mut c_void));
}

#[derive(Clone, Copy)]
struct Pending {
    generation: u64,
    opened: Instant,
    committed: Option<u64>,
}
#[derive(Default)]
struct State {
    generation: u64,
    pending: Option<Pending>,
}
struct Link(*mut c_void);
// The display-link reference is only started/stopped; CoreVideo synchronizes it.
unsafe impl Send for Link {}
unsafe impl Sync for Link {}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}
fn link() -> Option<*mut c_void> {
    static LINK: OnceLock<Option<Link>> = OnceLock::new();
    LINK.get_or_init(|| {
        let mut link = std::ptr::null_mut();
        if unsafe { CVDisplayLinkCreateWithActiveCGDisplays(&mut link) } != 0 || link.is_null() {
            return None;
        }
        // Process-lifetime reference; never released, so the callback cannot outlive it.
        (unsafe { CVDisplayLinkSetOutputCallback(link, output, std::ptr::null_mut()) } == 0)
            .then_some(Link(link))
    })
    .as_ref()
    .map(|link| link.0)
}
fn host_ns(ticks: u64) -> u64 {
    static BASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *BASE.get_or_init(|| {
        let mut base = Timebase { numer: 0, denom: 0 };
        if unsafe { mach_timebase_info(&mut base) } != 0 || base.denom == 0 {
            (1, 1)
        } else {
            (base.numer, base.denom)
        }
    });
    (ticks as u128 * numer as u128 / denom as u128).min(u64::MAX as u128) as u64
}

/// Main thread only, after a visible launcher opening reached software
/// readiness. A newer opening supersedes an unresolved estimate.
pub fn after_ready(opened: Instant) {
    let generation = {
        let mut state = state();
        if state.pending.take().is_some() {
            crate::performance_trace::shared().cancel_presented();
        }
        state.generation = state.generation.wrapping_add(1);
        state.pending = Some(Pending { generation: state.generation, opened, committed: None });
        state.generation
    };
    let mut context = ObserverContext {
        version: 0,
        info: generation as usize as *mut c_void,
        retain: None,
        release: None,
        description: None,
    };
    unsafe {
        let observer = CFRunLoopObserverCreate(std::ptr::null(), ACTIVITIES, 0, AFTER_COMMIT, committed, &mut context);
        if observer.is_null() {
            cancel();
            return;
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, kCFRunLoopCommonModes);
        // The run loop retains the non-repeating observer until it fires.
        CFRelease(observer);
    }
}
/// Discard an unresolved estimate, for example when the launcher hides first.
pub fn cancel() {
    if state().pending.take().is_some() {
        crate::performance_trace::shared().cancel_presented();
    }
}

unsafe extern "C" fn committed(_observer: *mut c_void, _activity: u64, info: *mut c_void) {
    let generation = info as usize as u64;
    let start = {
        let mut state = state();
        match state.pending.as_mut() {
            Some(pending) if pending.generation == generation && pending.committed.is_none() => {
                pending.committed = Some(unsafe { mach_absolute_time() });
                true
            }
            _ => false,
        }
    };
    if start {
        match link() {
            Some(link) if unsafe { CVDisplayLinkIsRunning(link) } != 0 || unsafe { CVDisplayLinkStart(link) } == 0 => {}
            _ => cancel(),
        }
    }
}
unsafe extern "C" fn output(
    _link: *mut c_void,
    now: *const TimeStamp,
    out: *const TimeStamp,
    _flags: u64,
    _flags_out: *mut u64,
    _info: *mut c_void,
) -> i32 {
    let (now_host, out_host) = unsafe { ((*now).host_time, (*out).host_time) };
    let mut state = state();
    let resolved = match state.pending {
        Some(pending) if pending.opened.elapsed() > LIMIT => {
            state.pending = None;
            crate::performance_trace::shared().cancel_presented();
            true
        }
        Some(Pending { opened, committed: Some(commit), .. }) if now_host >= commit => {
            state.pending = None;
            let ahead = Duration::from_nanos(host_ns(out_host.saturating_sub(unsafe { mach_absolute_time() })));
            crate::performance_trace::shared().record_presented(opened, opened.elapsed() + ahead);
            true
        }
        Some(_) => false,
        None => true,
    };
    drop(state);
    if resolved {
        // Stopping from the display-link thread can wait on itself; stop on main.
        unsafe { dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), stop_if_idle) };
    }
    0
}
unsafe extern "C" fn stop_if_idle(_context: *mut c_void) {
    if state().pending.is_none() {
        if let Some(link) = link() {
            unsafe { CVDisplayLinkStop(link) };
        }
    }
}
