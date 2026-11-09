//! Owned ABI leaves for the complete monitor/process inventory. No original
//! provider is ever invoked. Installation does not admit an endpoint or SDK.
use super::owned::{ModuleContext, Result};
use std::{
    collections::BTreeMap,
    ffi::{c_void, CStr},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
type Ptr = *mut c_void;
const HOST: u32 = 0xF100;
const CPU_BRAND: &str = "Owned F1 CPU";
const MACHINE_MODEL: &str = "OwnedMac1,1";
const MEMORY_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const UPTIME_ORIGIN: f64 = 90000.;
const WALL_UNIX_ORIGIN: f64 = 946728000.;
const BATTERY_CURRENT: u64 = 75;
const BATTERY_MAXIMUM: u64 = 100;
const BATTERY_CHARGING: bool = false;
const BATTERY_SOURCE: &str = "Battery Power";

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct SwiftString(pub(super) usize, pub(super) usize);
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "$sSS10FoundationE36_unconditionallyBridgeFromObjectiveCySSSo8NSStringCSgFZ"]
    pub(super) fn bridge_monitor_string(value: &objc2::runtime::AnyObject) -> SwiftString;
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn verify_original_monitor_sysctl(
    admission: &super::core_preentry::ReferenceAdmission,
    module: &super::ModuleAttestation,
) -> Result<()> {
    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        #[link_name = "$sSS10FoundationE19_bridgeToObjectiveCSo8NSStringCyF"]
        fn bridge_output(value: SwiftString) -> *mut NSString;
    }
    #[link(name = "swiftCore")]
    unsafe extern "C" {
        fn swift_bridgeObjectRelease();
    }
    module.verify()?;
    let import = module.imports.iter().find(|row| row.symbol == c"sysctlbyname")
        .ok_or("Reference monitor sysctl import absent")?;
    let [text_entry, scalar_entry, swap_entry] = admission.reference_monitor_sysctl_entries(import)?;
    let state = installed_fixture()?.clone();
    objc2::rc::autoreleasepool(|_| {
        let release: unsafe extern "C" fn(usize) =
            unsafe { std::mem::transmute(swift_bridgeObjectRelease as *const ()) };
        let input = |key: &str| -> Result<SwiftString> {
            let name = NSString::from_str(key);
            let input = unsafe { bridge_monitor_string(&name) };
            drop(name);
            if import.selected_address() != owned_sysctlbyname as *const () as usize {
                unsafe { release(input.1) };
                return Err("Reference monitor sysctl import changed after string bridge".into());
            }
            Ok(input)
        };
        let text: unsafe extern "C" fn(SwiftString) -> SwiftString =
            unsafe { std::mem::transmute(text_entry) };
        for (key, expected) in [
            ("machdep.cpu.brand_string", CPU_BRAND),
            ("hw.model", MACHINE_MODEL),
            ("ruycast.synthetic.denied", "Unavailable"),
        ] {
            let input = input(key)?;
            unsafe {
                let result = text(input);
                release(input.1);
                let object = Retained::from_raw(bridge_output(result));
                release(result.1);
                if object.is_none_or(|value| value.to_string() != expected) {
                    return Err("Reference original monitor text differs from fixture".into());
                }
            }
        }
        let scalar: unsafe extern "C" fn(SwiftString, u64) -> u64 =
            unsafe { std::mem::transmute(scalar_entry) };
        for (key, expected) in [
            ("hw.memsize", MEMORY_TOTAL),
            ("ruycast.synthetic.denied", 37),
        ] {
            let input = input(key)?;
            let observed = unsafe { scalar(input, 37) };
            unsafe { release(input.1) };
            if observed != expected {
                return Err("Reference original monitor scalar differs from fixture".into());
            }
        }
        let input = input("vm.swapusage")?;
        let mut observed = [0_u64; 4];
        unsafe {
            call_monitor_swap(swap_entry, input, &mut observed);
            release(input.1);
        }
        if observed.into_iter().flat_map(u64::to_ne_bytes).ne(swap()) {
            return Err("Reference original monitor swap differs from fixture".into());
        }
        verify_same_fixture(&state)?;
        module.verify()
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_monitor_swap(_: usize, _: SwiftString, _: *mut [u64; 4]) {
    // The pinned Swift specialization takes four fallback words in x2–x5
    // and returns the C xsw_usage value directly in x0–x3.
    core::arch::naked_asm!(
        "stp x19, x30, [sp, #-16]!",
        "mov x9, x0",
        "mov x0, x1",
        "mov x1, x2",
        "mov x19, x3",
        "mov x2, xzr",
        "mov x3, xzr",
        "mov x4, xzr",
        "mov x5, xzr",
        "blr x9",
        "stp x0, x1, [x19]",
        "stp x2, x3, [x19, #16]",
        "ldp x19, x30, [sp], #16",
        "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
thread_local! {
    static MONITOR_CONSTRUCTION: std::cell::RefCell<Option<Arc<super::core_preentry::ReferenceAdmission>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn take_monitor_construction_admission(caller: usize) -> Option<Arc<super::core_preentry::ReferenceAdmission>> {
    if !super::core_preentry::ReferenceAdmission::is_monitor_constructor_lock(caller) {
        return None;
    }
    MONITOR_CONSTRUCTION.with(|slot| slot.borrow_mut().take())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_monitor_constructor(_: usize, _: usize) -> usize {
    core::arch::naked_asm!(
        "stp x20, x30, [sp, #-16]!",
        "mov x20, x1",
        "blr x0",
        "ldp x20, x30, [sp], #16",
        "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn verify_original_monitor_sample(
    admission: &Arc<super::core_preentry::ReferenceAdmission>,
    module: &super::ModuleAttestation,
) -> Result<()> {
    #[link(name = "swiftCore")]
    unsafe extern "C" {
        fn swift_release();
    }
    let [metadata_entry, constructor, allocation_cell, sample, snapshot_metadata, destroy_entry] =
        admission.reference_monitor_sample_entries()?;
    let metadata: unsafe extern "C" fn(usize) -> usize =
        unsafe { std::mem::transmute(metadata_entry) };
    let metadata = unsafe { metadata(0) };
    if metadata == 0 {
        return Err("Reference original monitor metadata absent".into());
    }
    let replacement = super::bootstrap::source_allocation_with_zone_import_spec().replacement;
    if unsafe { (allocation_cell as *const usize).read_volatile() } != replacement {
        return Err("Reference monitor allocation import changed after metadata".into());
    }
    MONITOR_CONSTRUCTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err("Reference original monitor constructor reentered".to_owned());
        }
        *slot = Some(admission.clone());
        Ok(())
    })?;
    let sampler = unsafe { call_monitor_constructor(constructor, metadata) };
    let used = MONITOR_CONSTRUCTION.with(|slot| slot.borrow_mut().take().is_none());
    if sampler == 0 {
        return Err("Reference original monitor constructor returned no sampler".into());
    }
    let release: unsafe extern "C" fn(usize) =
        unsafe { std::mem::transmute(swift_release as *const ()) };
    let result = (|| {
        if !used {
            return Err("Reference original monitor constructor did not consume its lock admission".into());
        }
        let lock = unsafe { (sampler as *const usize).add(2).read() };
        if lock == 0 || unsafe { objc2::ffi::object_getClass(lock as *const _) } as usize
            != super::metadata::loaded_class(c"NSLock")?.address()
        {
            return Err("Reference original monitor sampler has no Foundation lock".into());
        }
        let metadata: unsafe extern "C" fn(usize) -> *const usize =
            unsafe { std::mem::transmute(snapshot_metadata) };
        let metadata = unsafe { metadata(0) };
        let size = admission.verify_monitor_snapshot_layout(metadata, destroy_entry)?;
        #[repr(align(16))]
        struct Snapshot([u8; 512]);
        let state = installed_fixture()?.clone();
        let destroy: unsafe extern "C" fn(*mut u8, *const usize) =
            unsafe { std::mem::transmute(destroy_entry) };
        let mut previous = None;
        for _ in 0..2 {
            module.verify()?;
            let mut snapshot = Snapshot([0; 512]);
            unsafe { call_monitor_sample(sample, sampler, snapshot.0.as_mut_ptr()) };
            let field = |slot: usize, width: usize| -> Result<&[u8]> {
                let offset = usize::try_from(unsafe { metadata.cast::<i32>().add(slot).read() })
                    .map_err(|_| "Reference monitor snapshot negative field offset")?;
                let end = offset.checked_add(width).filter(|end| *end <= size)
                    .ok_or("Reference monitor snapshot field outside its value")?;
                Ok(&snapshot.0[offset..end])
            };
            let number = |slot| -> Result<u64> {
                Ok(u64::from_ne_bytes(field(slot, 8)?.try_into().unwrap()))
            };
            let observed = (|| -> Result<(u64, u64)> {
                let process_count = state.processes.lock().map_err(|_| "Reference monitor process fixture poisoned")?.len() as u64;
                let page = unsafe { owned_getpagesize() } as u64;
                let vm_used = ((262144 + 131072 + 65536) * page).min(MEMORY_TOTAL);
                let battery = field(0x50 / 4, 9)?;
                let cpu = field(0x14 / 4, 9)?;
                let cpu_matches = if previous.is_some() {
                    cpu[8] == 0 && f64::from_ne_bytes(cpu[..8].try_into().unwrap()) == 20.0
                } else {
                    cpu[8] == 1
                };
                for (field, passed) in [
                    ("CPU", cpu_matches),
                    ("process count", number(0x30 / 4)? == process_count),
                    ("memory total", number(0x34 / 4)? == MEMORY_TOTAL),
                    ("VM used", number(0x38 / 4)? == vm_used),
                    ("disk", number(0x48 / 4)? == 4096 * 100_000_000),
                    ("battery", number(0x50 / 4)? == BATTERY_CURRENT && battery[8] == 0),
                ] {
                    if !passed {
                        return Err(format!("Reference original monitor {field} differs from fixture"));
                    }
                }
                Ok((number(0x5c / 4)?, number(0x60 / 4)?))
            })();
            unsafe { destroy(snapshot.0.as_mut_ptr(), metadata) };
            let (received, sent) = observed?;
            if let Some((last_received, last_sent)) = previous {
                if received.checked_sub(last_received) != Some(1024) || sent.checked_sub(last_sent) != Some(512) {
                    return Err("Reference original monitor network delta differs".into());
                }
            } else if received < 10_000 || sent < 20_000 {
                return Err("Reference original monitor initial network counters differ".into());
            }
            previous = Some((received, sent));
        }
        verify_same_fixture(&state)?;
        module.verify()
    })();
    unsafe { release(sampler) };
    result.and_then(|()| admission.before_reference_initialization())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_monitor_sample(_: usize, _: usize, _: *mut u8) {
    core::arch::naked_asm!(
        "stp x20, x30, [sp, #-16]!",
        "mov x20, x1",
        "mov x8, x2",
        "blr x0",
        "ldp x20, x30, [sp], #16",
        "ret",
    );
}

// The Swift Date value remains in caller-owned opaque storage. This child is
// only a Reference source leaf; it cannot create a Foundation receiver grant.
#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
mod monitor_clock;
#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(crate) mod monitor_timeout;

/// Owned fixture data only. These values grant no Foundation/CF constructor,
/// receiver, source admission or ability to read an original host provider.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorFacts {
    pub cpu_cores: usize,
    pub cpu_brand: &'static str,
    pub model: &'static str,
    pub os: &'static str,
    pub hostname: &'static str,
    pub memory_total: u64,
    pub uptime: f64,
    pub wall_unix: f64,
    pub battery_current: u64,
    pub battery_maximum: u64,
    pub battery_charging: bool,
    pub battery_source: &'static str,
}
#[derive(Clone, Debug)]
pub struct ProcessRow {
    pub pid: i32,
    pub uid: u32,
    pub started_seconds: u64,
    pub started_micros: u64,
    pub path: String,
    pub cpu: f64,
    pub rss_kib: u64,
}
/// Stable plain identity only, never a module/source/receiver admission.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) struct ProcessIdentity {
    pub pid:i32,pub uid:u32,pub seconds:u64,pub micros:u64,
}
impl ProcessRow {
    pub(crate) fn identity(&self)->ProcessIdentity {
        ProcessIdentity{pid:self.pid,uid:self.uid,seconds:self.started_seconds,micros:self.started_micros}
    }
}
pub struct Fixture {
    pub context: ModuleContext,
    pub(crate) module: OnceLock<super::process_router::ProcessModule>,
    pub processes: Mutex<BTreeMap<i32, ProcessRow>>,
    pub events: Mutex<Vec<String>>,
    pub cpu_sequence: AtomicU64,
    network_sequence: AtomicU64,
    pub monotonic_start: Instant,
    wall_unix_origin:f64,
    power: OnceLock<Power>,
    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    clock: OnceLock<Arc<monitor_clock::ClockSource>>,
}
struct Power {
    description: usize,
    list: usize,
    source: Arc<PowerSource>,
    validated_input_bytes: usize,
}
impl Drop for Power {
    fn drop(&mut self) {
        self.source.release_known(self.list as Ptr);
        self.source.release_known(self.description as Ptr);
    }
}
static FIXTURE: OnceLock<Arc<Fixture>> = OnceLock::new();
static FAULTED: AtomicBool = AtomicBool::new(false);
fn callback_boundary<T: Copy>(denied: T, callback: impl FnOnce() -> T) -> T {
    if FAULTED.load(Ordering::Acquire) {
        unsafe { *libc::__error() = libc::EPERM };
        return denied;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(payload) => {
            FAULTED.store(true, Ordering::Release);
            std::mem::forget(payload);
            unsafe { *libc::__error() = libc::EPERM };
            denied
        }
    }
}
pub const OWNED_EDITOR_PID: i32 = 730001;
pub const OWNED_EDITOR_CPU: f64 = 12.5;
pub const OWNED_EDITOR_RSS_KIB: u64 = 4096;

impl Fixture {
    pub fn new(context: ModuleContext) -> Result<Self> {
        Self::new_at_epoch(context,WALL_UNIX_ORIGIN as u64)
    }
    /// The caller supplies the genuine admitted fixture epoch before this
    /// fixture is published. Both time domains share one monotonic start.
    pub(crate) fn new_at_epoch(context:ModuleContext,epoch:u64)->Result<Self> {
        if epoch>1_u64<<53 {return Err("monitor fixture epoch exceeds exact clock representation".into());}
        context.life.before_effect()?;
        let (uid, _) = context.home.owner_ids()?;
        let row = ProcessRow {
            pid: OWNED_EDITOR_PID,
            uid,
            started_seconds: 11,
            started_micros: 22,
            path: "/owned/Owned Editor".into(),
            cpu: OWNED_EDITOR_CPU,
            rss_kib: OWNED_EDITOR_RSS_KIB,
        };
        Ok(Self {
            context,
            module: OnceLock::new(),
            processes: Mutex::new(BTreeMap::from([(row.pid, row)])),
            events: Mutex::new(vec![]),
            cpu_sequence: AtomicU64::new(0),
            network_sequence: AtomicU64::new(0),
            monotonic_start: Instant::now(),
            wall_unix_origin:epoch as f64,
            power: OnceLock::new(),
            #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
            clock: OnceLock::new(),
        })
    }
    #[cfg(any(test,all(not(test),rcf1_reference_sdk,target_arch="aarch64")))]
    fn matches_epoch(&self,epoch:u64)->bool {epoch<=1_u64<<53&&self.wall_unix_origin==epoch as f64}
    pub fn install(self: Arc<Self>) -> Result<()> {
        self.context.life.before_effect()?;
        FIXTURE
            .set(self)
            .map_err(|_| "system fixture already installed".to_string())
    }
    fn power(self: &Arc<Self>) -> Option<&Power> {
        // OnceLock initialization may race; the loser releases its own retained data.
        if self.power.get().is_none() {
            let value = Power::create(self).ok()?;
            let _ = self.power.set(value);
        }
        self.power.get()
    }
}
fn fixture() -> Option<&'static Arc<Fixture>> {
    // This gate also serves the Rust helper called by Objective-C metadata
    // handlers; they must receive refusal rather than a dynamic lease unwind.
    callback_boundary(None, || {
        let state = FIXTURE.get()?;
        if let Some(module) = state.module.get() {
            module.registry.before_effect().ok()?;
        } else {
            state.context.life.before_effect().ok()?;
        }
        Some(state)
    })
}

pub(crate) fn installed_fixture() -> Result<&'static Arc<Fixture>> {
    fixture().ok_or_else(|| "process monitor absent/retired".into())
}

/// Recheck the exact retained fixture after materializing Foundation children.
/// Legacy fixture installation alone is not an installed Process-module issuer.
pub fn verify_same_fixture(expected: &Arc<Fixture>) -> Result<()> {
    let actual = installed_fixture()?;
    if !Arc::ptr_eq(actual, expected) {
        return Err("monitor fixture identity differs".into());
    }
    let module = actual.module.get().ok_or("monitor module absent")?;
    let jobs = super::process_abi::installed_context()?;
    if !Arc::ptr_eq(&module.registry, &jobs.registry)
        || !Arc::ptr_eq(&actual.context.life, &jobs.life)
    {
        return Err("monitor process issuer differs".into());
    }
    // installed_context invokes the dynamic guardian. Only pure identity and
    // monotonic retirement checks follow it; no second guardian can reopen it.
    if FAULTED.load(Ordering::Acquire)
        || !FIXTURE.get().is_some_and(|current| Arc::ptr_eq(current, expected))
    {
        return Err("monitor fixture faulted/replaced".into());
    }
    module.registry.not_retired()
}

/// Both time domains advance from the existing single monotonic fixture clock.
/// Consumers retain the returned fixture and recheck it after dynamic callbacks.
pub fn monitor_facts() -> Result<(Arc<Fixture>, MonitorFacts)> {
    let state = installed_fixture()?.clone();
    verify_same_fixture(&state)?;
    let elapsed = state.monotonic_start.elapsed().as_secs_f64();
    let facts = monitor_snapshot(&state,elapsed);
    // No callback or object materialization occurred after the dynamic gate.
    state.module.get().ok_or("monitor module absent")?.registry.not_retired()?;
    Ok((state, facts))
}
fn monitor_snapshot(state:&Fixture,elapsed:f64)->MonitorFacts {
    MonitorFacts {
        cpu_cores: 4,
        cpu_brand: CPU_BRAND,
        model: MACHINE_MODEL,
        os: "Owned F1 OS",
        hostname: "owned-fixture",
        memory_total: MEMORY_TOTAL,
        uptime: UPTIME_ORIGIN + elapsed,
        wall_unix: state.wall_unix_origin + elapsed,
        battery_current: BATTERY_CURRENT,
        battery_maximum: BATTERY_MAXIMUM,
        battery_charging: BATTERY_CHARGING,
        battery_source: BATTERY_SOURCE,
    }
}

#[cfg(test)]
mod clock_epoch_tests {
    use super::*;
    fn check_vm_statistics_prefix() {
        let mut storage = [u64::MAX; 21];
        let mut words = 39;
        assert_eq!(unsafe {
            owned_host_statistics64(HOST, libc::HOST_VM_INFO64, storage.as_mut_ptr().cast(), &mut words)
        }, libc::KERN_FAILURE);
        assert_eq!(words, 39);
        assert_eq!(storage, [u64::MAX; 21]);
        words = 40;
        assert_eq!(unsafe {
            owned_host_statistics64(HOST, libc::HOST_VM_INFO64, storage.as_mut_ptr().cast(), &mut words)
        }, 0);
        assert_eq!(words, 40);
        let bytes: Vec<_> = storage.into_iter().flat_map(u64::to_ne_bytes).collect();
        assert_eq!(&bytes[160..], &[255; 8]);
        assert_eq!(&bytes[4..8], &262144_u32.to_ne_bytes());
        assert_eq!(&bytes[12..16], &131072_u32.to_ne_bytes());
        assert_eq!(&bytes[128..132], &65536_u32.to_ne_bytes());
    }
    #[test]
    fn monitor_vm_prefix_and_network_samples_use_owned_buffers() {
        let workspace = crate::workspace::tests::Fixture::new();
        let context = ModuleContext {
            home: super::super::owned::OwnedRoot::open(&workspace.root.join("home")).unwrap(),
            cache: super::super::owned::OwnedRoot::open(&workspace.root.join("cache")).unwrap(),
            temp: super::super::owned::OwnedRoot::open(&workspace.root.join("temp")).unwrap(),
            applications: super::super::owned::OwnedRoot::open(&workspace.root.join("apps")).unwrap(),
            life: workspace.life.clone(),
        };
        Arc::new(Fixture::new(context).unwrap()).install().unwrap();
        check_vm_statistics_prefix();
        let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
        let mut size = 0;
        let mut read = |out, size| unsafe {
            owned_sysctl(mib.as_mut_ptr(), 6, out, size, std::ptr::null_mut(), 0)
        };
        assert_eq!(read(std::ptr::null_mut(), &mut size), 0);
        assert_eq!(size, std::mem::size_of::<libc::if_msghdr2>());
        let mut buffer = vec![255u8; size];
        let mut invalid_size = [usize::MAX; 2];
        let misaligned_size = unsafe { invalid_size.as_mut_ptr().cast::<u8>().add(1).cast() };
        assert_eq!(read(buffer.as_mut_ptr().cast(), misaligned_size), -1);
        assert_eq!(invalid_size, [usize::MAX; 2]);
        assert!(buffer.iter().all(|&byte| byte == 255));
        let mut short = size - 1;
        assert_eq!(read(buffer.as_mut_ptr().cast(), &mut short), -1);
        assert_eq!(short, size);
        assert!(buffer.iter().all(|&byte| byte == 255));
        let base = std::mem::offset_of!(libc::if_msghdr2, ifm_data);
        for expected in [(10_000, 20_000), (11_024, 20_512)] {
            assert_eq!(read(buffer.as_mut_ptr().cast(), &mut size), 0);
            let counter = |offset| {
                let at = base + offset;
                u64::from_ne_bytes(buffer[at..at + 8].try_into().unwrap())
            };
            assert_eq!((counter(std::mem::offset_of!(libc::if_data64, ifi_ibytes)),
                counter(std::mem::offset_of!(libc::if_data64, ifi_obytes))), expected);
        }
    }

    #[test]
    fn admitted_epoch_and_default_share_one_monotonic_delta() {
        let workspace=crate::workspace::tests::Fixture::new();
        let context=||->ModuleContext {ModuleContext {
            home:super::super::owned::OwnedRoot::open(&workspace.root.join("home")).unwrap(),
            cache:super::super::owned::OwnedRoot::open(&workspace.root.join("cache")).unwrap(),
            temp:super::super::owned::OwnedRoot::open(&workspace.root.join("temp")).unwrap(),
            applications:super::super::owned::OwnedRoot::open(&workspace.root.join("apps")).unwrap(),
            life:workspace.life.clone(),
        }};
        let epoch=1_793_993_862_u64;
        let state=Fixture::new_at_epoch(context(),epoch).unwrap();
        assert!(state.matches_epoch(epoch));
        assert!(!state.matches_epoch(epoch+1));
        let first=monitor_snapshot(&state,0.25);
        let later=monitor_snapshot(&state,12.75);
        assert_eq!(first.wall_unix,epoch as f64+0.25);
        assert_eq!(first.uptime,UPTIME_ORIGIN+0.25);
        assert_eq!(later.wall_unix-first.wall_unix,later.uptime-first.uptime);
        assert_eq!(later.wall_unix-later.uptime,epoch as f64-UPTIME_ORIGIN);
        let legacy=Fixture::new(context()).unwrap();
        assert_eq!(monitor_snapshot(&legacy,0.25).wall_unix,WALL_UNIX_ORIGIN+0.25);
        assert!(Fixture::new_at_epoch(context(),1_u64<<53).unwrap().matches_epoch(1_u64<<53));
        for invalid in [(1_u64<<53)+1,u64::MAX] {
            assert!(Fixture::new_at_epoch(context(),invalid).is_err());
            assert!(!state.matches_epoch(invalid));
        }
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(crate) fn install_reference_clock(
    state: &Arc<Fixture>,
    admission: Arc<super::core_preentry::ReferenceAdmission>,
    guard: &dyn super::owned::AliveLease,
) -> Result<()> {
    monitor_clock::install(state, admission, guard)
}

/// No dynamic guardian is invoked here. The surrounding module transaction
/// owns that boundary; this verifies the selected clock cells/code afterwards.
pub(crate) fn verify_reference_clock(state: &Fixture) -> Result<()> {
    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    if let Some(clock) = state.clock.get() {
        return clock.verify_pure();
    }
    let _ = state;
    Err("Reference clock is not source-installed".into())
}

fn denied() -> i32 {
    unsafe { *libc::__error() = libc::EPERM };
    -1
}
/// Membership selects a fixture leaf, never grants effect authority. The
/// combined process router guards this lookup before entering any C callback.
pub fn contains_virtual(pid: i32) -> bool {
    FIXTURE
        .get()
        .is_some_and(|state| state.processes.lock().unwrap().contains_key(&pid))
}
/// Callback-free final data check. The caller first binds the actual installed
/// Reference issuer; this identity alone grants nothing. Lock order used by
/// Wire commits: model -> Wire -> registered jobs -> these virtual rows.
pub(crate) fn with_process_identity<T>(expected:ProcessIdentity,commit:impl FnOnce()->Result<T>)->Result<T> {
    let fixture=FIXTURE.get().ok_or("monitor fixture absent")?;
    if FAULTED.load(Ordering::Acquire) {return Err("monitor fixture faulted".into());}
    if let Some(module)=fixture.module.get() {module.registry.not_retired()?;}
    let rows=fixture.processes.lock().map_err(|_|"monitor process rows poisoned")?;
    if rows.get(&expected.pid).map(ProcessRow::identity)!=Some(expected) {
        return Err("synthetic process identity changed or disappeared".into());
    }
    if FAULTED.load(Ordering::Acquire) {return Err("monitor fixture faulted while waiting for identity".into());}
    if let Some(module)=fixture.module.get() {module.registry.not_retired()?;}
    commit()
}
/// # Safety
/// This receives only the scalar PID/signal selected by the owned router.
pub unsafe extern "C" fn owned_virtual_kill(pid: i32, signal: i32) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        if pid <= 1 || pid == unsafe { libc::getpid() } || ![0, libc::SIGKILL].contains(&signal) {
            return denied();
        }
        let Ok((owner, _)) = state.context.home.owner_ids() else {
            return denied();
        };
        let expected = state.processes.lock().unwrap().get(&pid).map(ProcessRow::identity);
        let Some(expected) = expected.filter(|identity| identity.uid == owner) else {
            return denied();
        };
        // The guardian may reenter either table. Requalify before holding
        // job classification and the virtual identity through the effect.
        let Ok(jobs) = super::process_abi::installed_context() else {
            return denied();
        };
        if owner != jobs.owner_uid || !Arc::ptr_eq(&state.context.life, &jobs.life)
            || state.module.get().is_some_and(|module| !Arc::ptr_eq(&module.registry, &jobs.registry))
        {
            return denied();
        }
        let event = (signal == libc::SIGKILL).then(|| format!("virtual.force-quit:{pid}"));
        let outcome = super::process_abi::with_no_registered_job(pid, || {
            let mut rows = state.processes.lock().map_err(|_| "monitor process rows poisoned")?;
            if rows.get(&pid).map(ProcessRow::identity) != Some(expected) {
                return Err("monitor process identity changed".into());
            }
            if let Some(event) = event {
                let mut events = state.events.lock().map_err(|_| "monitor events poisoned")?;
                events.try_reserve(1).map_err(|_| "monitor event allocation failed")?;
                if FAULTED.load(Ordering::Acquire) {
                    return Err("monitor process fixture faulted".into());
                }
                jobs.registry.not_retired()?;
                super::process_abi::verify_virtual_route_live()?;
                rows.remove(&pid);
                events.push(event);
            } else {
                if FAULTED.load(Ordering::Acquire) {
                    return Err("monitor process fixture faulted".into());
                }
                jobs.registry.not_retired()?;
                super::process_abi::verify_virtual_route_live()?;
            }
            Ok(())
        });
        outcome.map_or_else(|_| denied(), |()| 0)
    })
}
unsafe fn bytes_out(bytes: &[u8], out: Ptr, size: *mut usize) -> i32 {
    if size.is_null() || !size.is_aligned() {
        return denied();
    }
    let capacity = unsafe { *size };
    unsafe {
        *size = bytes.len();
    }
    if out.is_null() {
        return 0;
    }
    if capacity < bytes.len() {
        unsafe { *libc::__error() = libc::ENOMEM };
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), bytes.len());
    }
    0
}
// Encode only explicitly initialized fields in an initialized byte vector;
// never form a byte slice over Rust/C struct padding.
fn swap() -> Vec<u8> {
    let mut b = vec![0; std::mem::size_of::<libc::xsw_usage>()];
    for (at, v) in [
        (
            std::mem::offset_of!(libc::xsw_usage, xsu_total),
            1024_u64 * 1024 * 1024,
        ),
        (
            std::mem::offset_of!(libc::xsw_usage, xsu_avail),
            960_u64 * 1024 * 1024,
        ),
        (
            std::mem::offset_of!(libc::xsw_usage, xsu_used),
            64_u64 * 1024 * 1024,
        ),
    ] {
        b[at..at + 8].copy_from_slice(&v.to_ne_bytes());
    }
    b
}
pub(crate) unsafe extern "C" fn owned_sysctlbyname(
    name: *const libc::c_char,
    out: Ptr,
    size: *mut usize,
    new: Ptr,
    new_size: usize,
) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        if name.is_null() || !new.is_null() || new_size != 0 {
            return denied();
        }
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        let bytes = match name {
            b"machdep.cpu.brand_string" => [CPU_BRAND.as_bytes(), &[0]].concat(),
            b"hw.model" => [MACHINE_MODEL.as_bytes(), &[0]].concat(),
            b"hw.memsize" => MEMORY_TOTAL.to_ne_bytes().to_vec(),
            b"vm.swapusage" => swap(),
            _ => {
                state
                    .events
                    .lock()
                    .unwrap()
                    .push("denied.sysctlbyname".into());
                return denied();
            }
        };
        unsafe { bytes_out(&bytes, out, size) }
    })
}
pub(crate) unsafe extern "C" fn owned_proc_listallpids(buffer: Ptr, capacity: i32) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        let output = buffer.cast::<i32>();
        if capacity < 0 || !output.is_aligned() {
            return denied();
        }
        super::process_abi::with_job_classification(|registered| {
            let rows = state.processes.lock().map_err(|_| "monitor process rows poisoned")?;
            if FAULTED.load(Ordering::Acquire) {
                return Err("monitor fixture faulted before PID enumeration".into());
            }
            if let Some(module) = state.module.get() {
                module.registry.not_retired()?;
            }
            super::process_abi::verify_virtual_route_live()?;
            let pids = rows.keys().copied().filter(|pid| !registered(*pid));
            if output.is_null() {
                return Ok(pids.count() as i32);
            }
            let limit = capacity as usize / std::mem::size_of::<i32>();
            let mut count = 0;
            for pid in pids.take(limit) {
                unsafe { output.add(count).write(pid); }
                count += 1;
            }
            Ok(count as i32)
        }).unwrap_or_else(|_| denied())
    })
}
pub(crate) unsafe extern "C" fn owned_proc_pidinfo(
    pid: i32,
    flavor: i32,
    arg: u64,
    out: Ptr,
    capacity: i32,
) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        let p = out.cast::<libc::proc_bsdinfo>();
        if flavor != libc::PROC_PIDTBSDINFO || arg != 0 || p.is_null()
            || !p.is_aligned() || capacity < size as i32
        {
            return denied();
        }
        // The router's earlier membership check only selects the candidate.
        // Hold the job classification and the virtual row through publication.
        super::process_abi::with_no_registered_job(pid, || {
            let rows = state.processes.lock().map_err(|_| "monitor process rows poisoned")?;
            let row = rows.get(&pid).ok_or("synthetic process disappeared before BSD read")?;
            if FAULTED.load(Ordering::Acquire) {
                return Err("monitor fixture faulted before BSD read".into());
            }
            if let Some(module) = state.module.get() {
                module.registry.not_retired()?;
            }
            super::process_abi::verify_virtual_route_live()?;
            unsafe {
                std::ptr::write_bytes(out, 0, size);
                (*p).pbi_pid = pid as u32;
                (*p).pbi_uid = row.uid;
                (*p).pbi_ruid = row.uid;
                (*p).pbi_svuid = row.uid;
                (*p).pbi_start_tvsec = row.started_seconds;
                (*p).pbi_start_tvusec = row.started_micros;
            }
            Ok(size as i32)
        }).unwrap_or_else(|_| denied())
    })
}
pub(crate) unsafe extern "C" fn owned_proc_pidpath(pid: i32, out: Ptr, capacity: u32) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        super::process_abi::with_no_registered_job(pid, || {
            let rows = state.processes.lock().map_err(|_| "monitor process rows poisoned")?;
            let row = rows.get(&pid).ok_or("synthetic process disappeared before path read")?;
            let bytes = row.path.as_bytes();
            if out.is_null() || capacity as usize <= bytes.len() || FAULTED.load(Ordering::Acquire) {
                return Err("synthetic process path output or fixture differs".into());
            }
            if let Some(module) = state.module.get() {
                module.registry.not_retired()?;
            }
            super::process_abi::verify_virtual_route_live()?;
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), bytes.len());
                out.cast::<u8>().add(bytes.len()).write(0);
            }
            Ok(bytes.len() as i32)
        }).unwrap_or_else(|_| denied())
    })
}
pub(crate) unsafe extern "C" fn owned_mach_host_self() -> u32 {
    callback_boundary(0, || if fixture().is_some() { HOST } else { 0 })
}
pub(crate) unsafe extern "C" fn owned_mach_port_deallocate(_task: u32, port: u32) -> i32 {
    callback_boundary(libc::KERN_FAILURE, || {
        if fixture().is_some() && port == HOST {
            0
        } else {
            libc::KERN_FAILURE
        }
    })
}
unsafe extern "C" {
    static vm_page_size: u32;
}
// Darwin loader-initialized geometry for this own address space. Runtime
// stack protection needs this before guardian-thread startup. No host inventory
// acquisition, fallback guess or SDK readiness is provided by this scalar.
pub(crate) unsafe extern "C" fn owned_getpagesize() -> i32 {
    let page = unsafe { vm_page_size };
    if (4096..=65536).contains(&page) && page.is_power_of_two() {
        page as i32
    } else {
        unsafe { libc::_exit(90) }
    }
}
pub(crate) unsafe extern "C" fn owned_host_statistics(
    host: u32,
    flavor: i32,
    out: *mut i32,
    count: *mut u32,
) -> i32 {
    callback_boundary(libc::KERN_FAILURE, || {
        let Some(state) = fixture() else {
            return libc::KERN_FAILURE;
        };
        if host != HOST
            || flavor != libc::HOST_CPU_LOAD_INFO
            || out.is_null()
            || !out.is_aligned()
            || count.is_null()
            || !count.is_aligned()
            || unsafe { *count } < libc::HOST_CPU_LOAD_INFO_COUNT
        {
            return libc::KERN_FAILURE;
        }
        let n = state.cpu_sequence.fetch_add(1, Ordering::AcqRel) as u32;
        let ticks = [
            100_u32.wrapping_add(n.wrapping_mul(10)),
            50_u32.wrapping_add(n.wrapping_mul(5)),
            800_u32.wrapping_add(n.wrapping_mul(80)),
            50_u32.wrapping_add(n.wrapping_mul(5)),
        ];
        unsafe {
            std::ptr::copy_nonoverlapping(ticks.as_ptr(), out.cast(), 4);
            *count = libc::HOST_CPU_LOAD_INFO_COUNT;
        }
        0
    })
}
pub(crate) unsafe extern "C" fn owned_host_statistics64(
    host: u32,
    flavor: i32,
    out: *mut i32,
    count: *mut u32,
) -> i32 {
    // The frozen Swift SDK uses the 160-byte VM_INFO64 prefix. The
    // current Rust libc extends that struct with counters at its tail.
    const REFERENCE_WORDS: u32 = 40;
    const WORD_BYTES: usize = std::mem::size_of::<i32>();
    const COUNTERS: [(usize, u32); 3] = [
        (std::mem::offset_of!(libc::vm_statistics64, active_count), 262144),
        (std::mem::offset_of!(libc::vm_statistics64, wire_count), 131072),
        (std::mem::offset_of!(libc::vm_statistics64, compressor_page_count), 65536),
    ];
    const {
        assert!(COUNTERS[0].0 == 4);
        assert!(COUNTERS[1].0 == 12);
        assert!(COUNTERS[2].0 == 128);
    }
    callback_boundary(libc::KERN_FAILURE, || {
        if fixture().is_none()
            || host != HOST
            || flavor != libc::HOST_VM_INFO64
            || out.is_null()
            || !out.cast::<libc::vm_statistics64>().is_aligned()
            || count.is_null()
            || !count.is_aligned()
            || unsafe { *count } < REFERENCE_WORDS
        {
            return libc::KERN_FAILURE;
        }
        let words = if unsafe { *count } >= libc::HOST_VM_INFO64_COUNT {
            libc::HOST_VM_INFO64_COUNT
        } else {
            REFERENCE_WORDS
        };
        let bytes = out.cast::<u8>();
        unsafe {
            std::ptr::write_bytes(bytes, 0, words as usize * WORD_BYTES);
            for (offset, value) in COUNTERS {
                bytes.add(offset).cast::<u32>().write_unaligned(value);
            }
            *count = words;
        }
        0
    })
}
pub(crate) unsafe extern "C" fn owned_statfs(
    path: *const libc::c_char,
    out: *mut libc::statfs,
) -> i32 {
    callback_boundary(-1, || {
        if fixture().is_none()
            || path.is_null()
            || out.is_null()
            || !out.is_aligned()
            || unsafe { CStr::from_ptr(path) } != c"/"
        {
            return denied();
        }
        unsafe {
            std::ptr::write_bytes(out.cast::<u8>(), 0, std::mem::size_of::<libc::statfs>());
            (*out).f_bsize = 4096;
            (*out).f_blocks = 100_000_000;
            (*out).f_bavail = 40_000_000;
            (*out).f_bfree = 40_000_000;
        }
        0
    })
}
fn network(sample: u64) -> Vec<u8> {
    let mut b = vec![0; std::mem::size_of::<libc::if_msghdr2>()];
    let n = b.len() as u16;
    b[..2].copy_from_slice(&n.to_ne_bytes());
    b[2] = libc::RTM_VERSION as u8;
    b[3] = libc::RTM_IFINFO2 as u8;
    let at = std::mem::offset_of!(libc::if_msghdr2, ifm_index);
    b[at..at + 2].copy_from_slice(&7_u16.to_ne_bytes());
    let at = std::mem::offset_of!(libc::if_msghdr2, ifm_flags);
    b[at..at + 4].copy_from_slice(&libc::IFF_UP.to_ne_bytes());
    let base = std::mem::offset_of!(libc::if_msghdr2, ifm_data);
    for (at, value) in [
        (
            base + std::mem::offset_of!(libc::if_data64, ifi_ibytes),
            10_000_u64.saturating_add(sample.saturating_mul(1_024)),
        ),
        (
            base + std::mem::offset_of!(libc::if_data64, ifi_obytes),
            20_000_u64.saturating_add(sample.saturating_mul(512)),
        ),
    ] {
        b[at..at + 8].copy_from_slice(&value.to_ne_bytes());
    }
    b
}
pub(crate) unsafe extern "C" fn owned_sysctl(
    mib: *mut i32,
    len: u32,
    out: Ptr,
    size: *mut usize,
    new: Ptr,
    new_size: usize,
) -> i32 {
    callback_boundary(-1, || {
        let Some(state) = fixture() else {
            return denied();
        };
        if mib.is_null() || !mib.is_aligned() || len != 6
            || size.is_null() || !size.is_aligned() || !new.is_null() || new_size != 0
        {
            return denied();
        }
        if unsafe { std::slice::from_raw_parts(mib, 6) }
            != [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0]
        {
            return denied();
        }
        // ResourceSampler probes the size before reading the route buffer.
        // Only a complete data read counts as one sample in either arm.
        let sample = if !out.is_null()
            && unsafe { *size } >= std::mem::size_of::<libc::if_msghdr2>()
        {
            state.network_sequence.fetch_update(Ordering::AcqRel, Ordering::Acquire,
                |sample| Some(sample.saturating_add(1))).unwrap_or_else(|sample| sample)
        } else {
            0
        };
        unsafe { bytes_out(&network(sample), out, size) }
    })
}
pub(crate) unsafe extern "C" fn owned_if_indextoname(
    index: u32,
    out: *mut libc::c_char,
) -> *mut libc::c_char {
    callback_boundary(std::ptr::null_mut(), || {
        if fixture().is_none() || index != 7 || out.is_null() {
            denied();
            return std::ptr::null_mut();
        }
        unsafe {
            std::ptr::copy_nonoverlapping(c"en0".as_ptr(), out, 4);
        }
        out
    })
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
    fn CFPropertyListCreateWithData(
        allocator: Ptr,
        data: Ptr,
        options: usize,
        format: *mut isize,
        error: *mut Ptr,
    ) -> Ptr;
    fn CFArrayCreate(
        allocator: Ptr,
        values: *const Ptr,
        count: isize,
        callbacks: *const usize,
    ) -> Ptr;
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFAllocatorGetTypeID() -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(value: *const c_void) -> isize;
    fn CFDataGetBytePtr(value: *const c_void) -> *const u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(value: Ptr) -> isize;
    fn CFArrayGetValueAtIndex(value: Ptr, index: isize) -> Ptr;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetCount(value: *const c_void) -> isize;
    fn CFDictionaryGetKeysAndValues(value: Ptr, keys: *mut Ptr, values: *mut Ptr);
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberIsFloatType(value: Ptr) -> u8;
    fn CFNumberGetValue(value: *const c_void, kind: isize, output: *mut c_void) -> u8;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ptr) -> u8;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: Ptr) -> isize;
    fn CFStringGetCharacters(value: Ptr, range: CfRange, output: *mut u16);
    static kCFTypeArrayCallBacks: [usize; 5];
    static kCFAllocatorSystemDefault: Ptr;
}
#[repr(C)]
struct CfRange { location: isize, length: isize }
const CF_IMAGE: &[u8] = b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation";
const POWER_FILES: [&str; 9] = ["CFBase.h", "CFData.h", "CFPropertyList.h", "CFArray.h",
    "CFDictionary.h", "CFNumber.h", "CFString.h", "MacTypes.h", "system_providers.rs"];
#[derive(Clone, Debug)]
pub struct PowerSourcePin {pub path: std::path::PathBuf,pub sha256: [u8;32],pub owner: u32}
/// Existing-object observations only: never constructs or grants a module scope.
#[derive(Clone, Debug)]
pub struct PowerAttestation {
    pub sources: Vec<PowerSourcePin>,
    pub code: Vec<(&'static str,super::metadata::CodeIdentity)>,
    pub description: usize,pub list: usize,pub allocator: usize,pub array_callbacks: [usize;5],
    pub validated_input_bytes: usize,
    pub payload: PowerPayloadFootprint,
}
/// Actual live container/value observations. Plain data, no receiver authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerPayloadFootprint {
    pub dictionary_type: usize,
    pub array_type: usize,
    pub dictionary_entries: usize,
    pub array_entries: usize,
    pub value_types: BTreeMap<String,usize>,
    pub minimum_capacity: i64,
    pub current_capacity: i64,
    pub maximum_capacity: i64,
    pub charging: bool,
    pub power_source: String,
}
struct PowerSource {
    fixture: std::sync::Weak<Fixture>,registry: Arc<super::process_dispatch::Registry>,
    pins: Vec<PowerSourcePin>,code: Vec<(&'static str,usize,super::metadata::CodeIdentity)>,
    globals_base: usize,allocator: usize,callbacks: [usize;5],
    retired: AtomicBool,
    retain: unsafe extern "C" fn(*const c_void)->*const c_void,
    release: unsafe extern "C" fn(*const c_void),
}
fn power_functions()->Vec<(&'static str,usize)> {
    vec![("CFDataCreate",CFDataCreate as *const () as usize),
        ("CFPropertyListCreateWithData",CFPropertyListCreateWithData as *const () as usize),
        ("CFArrayCreate",CFArrayCreate as *const () as usize),
        ("CFRetain",CFRetain as *const () as usize),("CFRelease",CFRelease as *const () as usize),
        ("CFGetTypeID",CFGetTypeID as *const () as usize),
        ("CFAllocatorGetTypeID",CFAllocatorGetTypeID as *const () as usize),
        ("CFDataGetTypeID",CFDataGetTypeID as *const () as usize),
        ("CFDataGetLength",CFDataGetLength as *const () as usize),
        ("CFDataGetBytePtr",CFDataGetBytePtr as *const () as usize),
        ("CFArrayGetTypeID",CFArrayGetTypeID as *const () as usize),
        ("CFArrayGetCount",CFArrayGetCount as *const () as usize),
        ("CFArrayGetValueAtIndex",CFArrayGetValueAtIndex as *const () as usize),
        ("CFDictionaryGetTypeID",CFDictionaryGetTypeID as *const () as usize),
        ("CFDictionaryGetCount",CFDictionaryGetCount as *const () as usize),
        ("CFDictionaryGetKeysAndValues",CFDictionaryGetKeysAndValues as *const () as usize),
        ("CFNumberGetTypeID",CFNumberGetTypeID as *const () as usize),
        ("CFNumberIsFloatType",CFNumberIsFloatType as *const () as usize),
        ("CFNumberGetValue",CFNumberGetValue as *const () as usize),
        ("CFBooleanGetTypeID",CFBooleanGetTypeID as *const () as usize),
        ("CFBooleanGetValue",CFBooleanGetValue as *const () as usize),
        ("CFStringGetTypeID",CFStringGetTypeID as *const () as usize),
        ("CFStringGetLength",CFStringGetLength as *const () as usize),
        ("CFStringGetCharacters",CFStringGetCharacters as *const () as usize)]
}
fn power_image(address:usize)->Result<usize> {
    let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
    if address==0 || unsafe {libc::dladdr(address as Ptr,info.as_mut_ptr())}==0 {return Err("Power CF image absent".into());}
    let info=unsafe {info.assume_init()};
    if info.dli_fname.is_null() || info.dli_fbase.is_null()
        || unsafe {CStr::from_ptr(info.dli_fname)}.to_bytes()!=CF_IMAGE {return Err("Power symbol is not actual CoreFoundation".into());}
    Ok(info.dli_fbase as usize)
}
fn read_power_source(pin:&PowerSourcePin)->Result<Vec<u8>> {
    use std::{io::Read,os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}}};
    use sha2::{Digest,Sha256};
    if std::fs::canonicalize(&pin.path).map_err(|_|"Power source canonicalization")?!=pin.path {return Err("Power source alias".into());}
    let file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK)
        .open(&pin.path).map_err(|_|"Power source readcap")?;
    let before=file.metadata().map_err(|_|"Power source fstat")?;
    let identity=|m:&std::fs::Metadata|(m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    let flags=unsafe {libc::fcntl(file.as_raw_fd(),libc::F_GETFL)};
    let fd_flags=unsafe {libc::fcntl(file.as_raw_fd(),libc::F_GETFD)};
    if !before.is_file() || before.nlink()!=1 || before.uid()!=pin.owner || before.mode()&0o022!=0 || before.len()>2*1024*1024
        || flags<0 || fd_flags<0 || flags&libc::O_ACCMODE!=libc::O_RDONLY
        || fd_flags&libc::FD_CLOEXEC==0 {return Err("Power source readcap identity/access".into());}
    let mut bytes=Vec::new();
    (&file).take(2*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"Power source bounded read")?;
    let after=file.metadata().map_err(|_|"Power source refstat")?;
    let leaf=std::fs::symlink_metadata(&pin.path).map_err(|_|"Power source restat")?;
    if bytes.len()>2*1024*1024 || identity(&before)!=identity(&after) || identity(&after)!=identity(&leaf)
        || <[u8;32]>::from(Sha256::digest(&bytes))!=pin.sha256 {return Err("Power source bytes/identity changed".into());}
    Ok(bytes)
}
impl PowerSource {
    #[cfg(rcf1_monitor_cf_source)]
    fn compiled_inputs()->Result<[(&'static str,&'static [u8]);9]> {
        Ok([
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFBase.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFBase.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFData.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFData.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFPropertyList.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFPropertyList.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFArray.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFArray.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFDictionary.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFDictionary.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFNumber.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFNumber.h"))),
            (concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFString.h"),include_bytes!(concat!(env!("RUYCAST_F1_MONITOR_CF_HEADERS"),"/CFString.h"))),
            (env!("RUYCAST_F1_MONITOR_MAC_TYPES_HEADER"),include_bytes!(env!("RUYCAST_F1_MONITOR_MAC_TYPES_HEADER"))),
            (env!("RUYCAST_F1_MONITOR_DECLARATION_SOURCE"),include_bytes!(env!("RUYCAST_F1_MONITOR_DECLARATION_SOURCE"))),
        ])
    }
    #[cfg(not(rcf1_monitor_cf_source))]
    fn compiled_inputs()->Result<[(&'static str,&'static [u8]);9]> {
        Err("Power actual compiler source inputs absent".into())
    }
    fn pins(owner:u32)->Result<Vec<PowerSourcePin>> {
        use sha2::{Digest,Sha256};
        let compiled=Self::compiled_inputs()?;
        // Producer-owned actual header/declaration/compiler join, never runtime env.
        let literal=option_env!("RUYCAST_F1_MONITOR_CF_DECLARATION_PINS").ok_or("Power compiler declaration pins absent")?;
        if literal.len()>32768 {return Err("Power compiler pin bound".into());}
        let rows:[(std::path::PathBuf,String,u32);9]=serde_json::from_str(literal).map_err(|_|"Power compiler pin schema")?;
        let mut pins=Vec::new();
        for (index,(path,hash,uid)) in rows.into_iter().enumerate() {
            if !path.is_absolute() || path.file_name()!=Some(std::ffi::OsStr::new(POWER_FILES[index]))
                || uid!=(if index==8 {owner}else{0}) || hash.len()!=64
                || !hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {return Err("Power compiler pin fields".into());}
            let mut digest=[0;32];
            for (at,byte) in digest.iter_mut().enumerate() {*byte=u8::from_str_radix(&hash[at*2..at*2+2],16).map_err(|_|"Power compiler digest")?;}
            if path!=std::path::Path::new(compiled[index].0) || digest!=<[u8;32]>::from(Sha256::digest(compiled[index].1)) {
                return Err("Power pins differ from actually compiled source inputs".into());
            }
            if index==8 && digest!=<[u8;32]>::from(Sha256::digest(include_bytes!("system_providers.rs"))) {
                return Err("Power declaration differs from actual compiled source".into());
            }
            pins.push(PowerSourcePin {path,sha256:digest,owner:uid});
        }
        if pins.iter().take(7).any(|pin|pin.path.parent()!=pins[0].path.parent()) {return Err("Power CF header issuers differ".into());}
        Ok(pins)
    }
    fn capture(state:&Arc<Fixture>)->Result<Arc<Self>> {
        verify_same_fixture(state)?;
        let pins=Self::pins(state.context.home.owner_ids()?.0)?;
        let requirements:[&[&str];8]=[
            &["CFTypeID CFGetTypeID(CFTypeRef cf);","CFTypeRef CFRetain(CFTypeRef cf);","void CFRelease(CFTypeRef cf);","const CFAllocatorRef kCFAllocatorSystemDefault;","CFAllocatorGetTypeID(void);","typedef struct { CFIndex location; CFIndex length; } CFRange;","typedef signed long CFIndex;","typedef unsigned long CFTypeID;","typedef unsigned long CFOptionFlags;"],
            &["CFTypeID CFDataGetTypeID(void);","CFDataRef CFDataCreate(CFAllocatorRef allocator, const UInt8 *bytes, CFIndex length);","CFIndex CFDataGetLength(CFDataRef theData);","const UInt8 *CFDataGetBytePtr(CFDataRef theData);"],
            &["CFPropertyListRef CFPropertyListCreateWithData(CFAllocatorRef allocator, CFDataRef data, CFOptionFlags options, CFPropertyListFormat *format, CFErrorRef *error)","kCFPropertyListImmutable = 0,","typedef CF_ENUM(CFIndex, CFPropertyListFormat)"],
            &["CFTypeID CFArrayGetTypeID(void);","CFArrayRef CFArrayCreate(CFAllocatorRef allocator, const void **values, CFIndex numValues, const CFArrayCallBacks *callBacks);","CFIndex CFArrayGetCount(CFArrayRef theArray);","const void *CFArrayGetValueAtIndex(CFArrayRef theArray, CFIndex idx);","const CFArrayCallBacks kCFTypeArrayCallBacks;","typedef struct { CFIndex version; CFArrayRetainCallBack retain; CFArrayReleaseCallBack release; CFArrayCopyDescriptionCallBack copyDescription; CFArrayEqualCallBack equal; } CFArrayCallBacks;","typedef const void * (*CFArrayRetainCallBack)(CFAllocatorRef allocator, const void *value);","typedef void (*CFArrayReleaseCallBack)(CFAllocatorRef allocator, const void *value);","typedef CFStringRef (*CFArrayCopyDescriptionCallBack)(const void *value);","typedef Boolean (*CFArrayEqualCallBack)(const void *value1, const void *value2);"],
            &["CFTypeID CFDictionaryGetTypeID(void);","CFIndex CFDictionaryGetCount(CFDictionaryRef theDict);","void CFDictionaryGetKeysAndValues(CFDictionaryRef theDict, const void **keys, const void **values);"],
            &["CFTypeID CFNumberGetTypeID(void);","CFTypeID CFBooleanGetTypeID(void);","typedef CF_ENUM(CFIndex, CFNumberType)","kCFNumberSInt64Type = 4,","Boolean CFNumberGetValue(CFNumberRef number, CFNumberType theType, void *valuePtr);","Boolean CFNumberIsFloatType(CFNumberRef number);","Boolean CFBooleanGetValue(CFBooleanRef boolean);"],
            &["CFTypeID CFStringGetTypeID(void);","CFIndex CFStringGetLength(CFStringRef theString);","void CFStringGetCharacters(CFStringRef theString, CFRange range, UniChar *buffer);"],
            &["typedef unsigned char UInt8;","typedef unsigned short UInt16;","typedef unsigned char Boolean;","typedef UInt16 UniChar;"],
        ];
        let compact=|bytes:&[u8]|bytes.iter().copied().filter(|b|!b.is_ascii_whitespace()).collect::<Vec<_>>();
        for (index,pin) in pins.iter().enumerate() {
            let bytes=read_power_source(pin)?;
            if index<8 {
                let text=compact(&bytes);
                for required in requirements[index] {
                    let expected=compact(required.as_bytes());
                    if !text.windows(expected.len()).any(|slice|slice==expected.as_slice()) {return Err(format!("Power typed declaration absent: {}",POWER_FILES[index]));}
                }
            }
        }
        if std::mem::size_of::<usize>()!=8 || std::mem::size_of::<libc::c_long>()!=8
            || std::mem::size_of::<CfRange>()!=16 || std::mem::size_of::<[usize;5]>()!=40 {
            return Err("Power CF ABI layout unsupported".into());
        }
        let globals_base=power_image((&raw const kCFTypeArrayCallBacks) as usize)?;
        if power_image((&raw const kCFAllocatorSystemDefault) as usize)?!=globals_base {return Err("Power CF global image differs".into());}
        let callbacks=unsafe {std::ptr::read_volatile(&raw const kCFTypeArrayCallBacks)};
        let allocator=unsafe {std::ptr::read_volatile(&raw const kCFAllocatorSystemDefault)} as usize;
        if callbacks[0]!=0 || callbacks[1..].contains(&0) || allocator==0 {return Err("Power allocator/callback slots differ".into());}
        let mut functions=power_functions();
        for (index,name) in ["array.retain","array.release","array.copyDescription","array.equal"].into_iter().enumerate() {functions.push((name,callbacks[index+1]));}
        let mut code=Vec::new();
        for (name,address) in functions {
            if power_image(address)?!=globals_base {return Err("Power CF function image differs".into());}
            let actual=super::metadata::code_identity(address)?;
            if actual.image()!=CF_IMAGE {return Err("Power CF executable image differs".into());}
            code.push((name,address,actual));
        }
        let source=Arc::new(Self {fixture:Arc::downgrade(state),registry:state.module.get().ok_or("Power module absent")?.registry.clone(),pins,code,globals_base,allocator,callbacks,retired:AtomicBool::new(false),retain:CFRetain,release:CFRelease});
        source.verify(state)?;
        // Public CF allocator data inspection only, after source/code/layout checks.
        if unsafe {CFGetTypeID(source.allocator as Ptr)!=CFAllocatorGetTypeID()} {return Err("Power explicit allocator type differs".into());}
        source.verify_pure(state)?;
        Ok(source)
    }
    fn verify_code(&self)->Result<()> {
        for (_,address,identity) in &self.code {
            if super::metadata::code_identity(*address)?!=*identity || power_image(*address)?!=self.globals_base {return Err("Power CF code changed".into());}
        }
        if power_functions().iter().any(|(name,address)|!self.code.iter().any(|(n,a,_)|n==name&&a==address)) {return Err("Power CF compiler import changed".into());}
        if power_image((&raw const kCFTypeArrayCallBacks) as usize)?!=self.globals_base
            || power_image((&raw const kCFAllocatorSystemDefault) as usize)?!=self.globals_base
            || unsafe {std::ptr::read_volatile(&raw const kCFTypeArrayCallBacks)}!=self.callbacks
            || unsafe {std::ptr::read_volatile(&raw const kCFAllocatorSystemDefault)} as usize!=self.allocator {return Err("Power CF global changed".into());}
        Ok(())
    }
    fn verify(&self,state:&Arc<Fixture>)->Result<()> {
        if self.retired.load(Ordering::Acquire) {return Err("Power source retired".into());}
        let result=self.verify_inner(state);
        if result.is_err() {self.retired.store(true,Ordering::Release);}
        result
    }
    fn verify_inner(&self,state:&Arc<Fixture>)->Result<()> {
        verify_same_fixture(state)?;
        self.verify_pure(state)
    }
    fn verify_pure(&self,state:&Arc<Fixture>)->Result<()> {
        if self.retired.load(Ordering::Acquire) || FAULTED.load(Ordering::Acquire)
            || !FIXTURE.get().is_some_and(|actual|Arc::ptr_eq(actual,state)) {
            return Err("Power fixture/source retired or replaced".into());
        }
        if !self.fixture.upgrade().is_some_and(|actual|Arc::ptr_eq(&actual,state))
            || !Arc::ptr_eq(&self.registry,&state.module.get().ok_or("Power module absent")?.registry) {return Err("Power custody issuer differs".into());}
        for pin in &self.pins {read_power_source(pin)?;}
        self.verify_code()?;
        self.registry.not_retired()?;
        if self.retired.load(Ordering::Acquire) {return Err("Power source retired during guard".into());}
        Ok(())
    }
    fn release_known(&self,pointer:Ptr) {
        // Cleanup bypasses business/source-file callbacks. Held original code
        // remains required, including the copied array cleanup callbacks.
        if pointer.is_null() || self.code.iter().any(|(_,address,identity)|super::metadata::code_identity(*address).ok().as_ref()!=Some(identity))
            || !self.code.iter().any(|(name,address,_)|*name=="CFRelease"&&*address==self.release as *const () as usize) {
            unsafe {libc::_exit(93)}
        }
        unsafe {(self.release)(pointer)};
    }
}
struct PowerOwned {pointer:Ptr,source:Arc<PowerSource>}
impl Drop for PowerOwned {
    fn drop(&mut self) {if !self.pointer.is_null() {self.source.release_known(self.pointer);}}
}
impl PowerOwned {
    fn new(pointer:Ptr,source:&Arc<PowerSource>)->Result<Self> {
        if pointer.is_null() {return Err("Power CF constructor returned nil".into());}
        Ok(Self {pointer,source:source.clone()})
    }
    fn into_raw(mut self)->Ptr {let result=self.pointer;self.pointer=std::ptr::null_mut();result}
}
fn power_text(value:Ptr)->Result<String> {
    if value.is_null() || unsafe {CFGetTypeID(value)!=CFStringGetTypeID()} {return Err("Power string type differs".into());}
    let count=unsafe {CFStringGetLength(value)};
    if !(0..=64).contains(&count) {return Err("Power string bound".into());}
    let mut units=[0_u16;64];
    unsafe {CFStringGetCharacters(value,CfRange {location:0,length:count},units.as_mut_ptr())};
    String::from_utf16(&units[..count as usize]).map_err(|_|"Power UTF16 differs".into())
}
fn verify_power_payload(description:Ptr,list:Ptr)->Result<PowerPayloadFootprint> {
    if description.is_null() || list.is_null() {return Err("Power nil container".into());}
    let dictionary_type=unsafe {CFGetTypeID(description)};
    let array_type=unsafe {CFGetTypeID(list)};
    if unsafe {dictionary_type!=CFDictionaryGetTypeID() || array_type!=CFArrayGetTypeID()} {
        return Err("Power container type differs".into());
    }
    let dictionary_entries=unsafe {CFDictionaryGetCount(description)};
    let array_entries=unsafe {CFArrayGetCount(list)};
    if dictionary_entries!=4 || array_entries!=1 || unsafe {CFArrayGetValueAtIndex(list,0)}!=description {
        return Err("Power container count/pointer differs".into());
    }
    let mut keys=[std::ptr::null_mut();4];let mut values=[std::ptr::null_mut();4];
    unsafe {CFDictionaryGetKeysAndValues(description,keys.as_mut_ptr(),values.as_mut_ptr())};
    let mut seen=std::collections::BTreeSet::new();
    let mut value_types=BTreeMap::new();
    let (mut current_capacity,mut maximum_capacity,mut charging,mut power_source)=(None,None,None,None);
    for (key,value) in keys.into_iter().zip(values) {
        let key=power_text(key)?;
        if value.is_null() || !seen.insert(key.clone()) {return Err("Power duplicate/nil value".into());}
        value_types.insert(key.clone(),unsafe {CFGetTypeID(value)});
        match key.as_str() {
            "Current Capacity"|"Max Capacity"=>{
                let mut number=0_i64;
                if unsafe {CFGetTypeID(value)!=CFNumberGetTypeID() || CFNumberIsFloatType(value)!=0 || CFNumberGetValue(value,4,(&mut number as *mut i64).cast())==0}
                    || number!=(if key=="Current Capacity" {BATTERY_CURRENT as i64}else{BATTERY_MAXIMUM as i64}) {return Err("Power capacity differs".into());}
                if key=="Current Capacity" {current_capacity=Some(number);} else {maximum_capacity=Some(number);}
            },
            "Is Charging"=>{
                if unsafe {CFGetTypeID(value)!=CFBooleanGetTypeID()} {return Err("Power Boolean type differs".into());}
                let actual=unsafe {CFBooleanGetValue(value)};
                if actual!=u8::from(BATTERY_CHARGING) {return Err("Power Boolean differs".into());}
                charging=Some(actual!=0);
            },
            "Power Source State"=>{
                let actual=power_text(value)?;
                if actual!=BATTERY_SOURCE {return Err("Power state differs".into());}
                power_source=Some(actual);
            },
            _=>return Err("Power unknown key".into()),
        }
    }
    let current_capacity=current_capacity.ok_or("Power current capacity absent")?;
    let maximum_capacity=maximum_capacity.ok_or("Power maximum capacity absent")?;
    if maximum_capacity<=0 || !(0..=maximum_capacity).contains(&current_capacity) {return Err("Power capacity range differs".into());}
    Ok(PowerPayloadFootprint {dictionary_type,array_type,dictionary_entries:dictionary_entries as usize,
        array_entries:array_entries as usize,value_types,minimum_capacity:0,current_capacity,maximum_capacity,
        charging:charging.ok_or("Power charging absent")?,power_source:power_source.ok_or("Power state absent")?})
}
impl Power {
    fn create(state:&Arc<Fixture>) -> Result<Self> {
        let source=PowerSource::capture(state)?;
        let xml = format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Current Capacity</key><integer>{BATTERY_CURRENT}</integer><key>Max Capacity</key><integer>{BATTERY_MAXIMUM}</integer><key>Is Charging</key><{}/><key>Power Source State</key><string>{BATTERY_SOURCE}</string></dict></plist>", if BATTERY_CHARGING { "true" } else { "false" });
        source.verify(state)?;
        unsafe {
            let data=PowerOwned::new(CFDataCreate(source.allocator as Ptr,xml.as_ptr(),xml.len() as isize).cast_mut(),&source)?;
            source.verify(state)?;
            if CFGetTypeID(data.pointer)!=CFDataGetTypeID() || CFDataGetLength(data.pointer)!=xml.len() as isize {
                return Err("Power input data type/length differs".into());
            }
            let bytes=CFDataGetBytePtr(data.pointer);
            if bytes.is_null() || std::slice::from_raw_parts(bytes,xml.len())!=xml.as_bytes() {
                return Err("Power input data bytes differ".into());
            }
            let mut error = std::ptr::null_mut();
            let description = CFPropertyListCreateWithData(
                source.allocator as Ptr,
                data.pointer,
                0,
                std::ptr::null_mut(),
                &mut error,
            );
            let error=(!error.is_null()).then(||PowerOwned {pointer:error,source:source.clone()});
            let description=PowerOwned::new(description,&source)?;
            if error.is_some() {return Err("Power fixed XML parse failed".into());}
            source.verify(state)?;
            let list = PowerOwned::new(CFArrayCreate(
                source.allocator as Ptr,
                &description.pointer,
                1,
                source.callbacks.as_ptr(),
            ),&source)?;
            source.verify(state)?;
            verify_power_payload(description.pointer,list.pointer)?;
            source.verify_pure(state)?;
            Ok(Self {description:description.into_raw() as usize,list:list.into_raw() as usize,source,validated_input_bytes:xml.len()})
        }
    }
    fn verify(&self,state:&Arc<Fixture>)->Result<PowerPayloadFootprint> {
        self.source.verify(state)?;
        let payload=verify_power_payload(self.description as Ptr,self.list as Ptr).inspect_err(|_|self.source.retired.store(true,Ordering::Release))?;
        let result=self.source.verify_pure(state);
        if result.is_err() {self.source.retired.store(true,Ordering::Release);}
        result?;
        Ok(payload)
    }
    fn retained(&self,state:&Arc<Fixture>,pointer:usize)->Result<Ptr> {
        self.verify(state)?;
        if pointer!=self.description && pointer!=self.list {return Err("Power retain pointer differs".into());}
        let returned=unsafe {(self.source.retain)(pointer as Ptr)}.cast_mut();
        if returned as usize!=pointer {
            self.source.retired.store(true,Ordering::Release);
            // Never adopt/release an unexpected foreign pointer.
            return Err("Power retain identity differs".into());
        }
        let pending=PowerOwned::new(returned,&self.source)?;
        self.verify(state)?;
        Ok(pending.into_raw())
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) mod monitor_process_info;

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(crate) fn prepare_reference_power(state: &Arc<Fixture>) -> Result<()> {
    let power = Power::create(state)?;
    state.power.set(power).map_err(|_| "Reference Power objects already installed".to_owned())
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(crate) fn verify_reference_power(state: &Arc<Fixture>) -> Result<()> {
    state.power.get().ok_or("Reference Power objects not prepared")?.verify(state)?;
    Ok(())
}

pub fn verify_power_attestation(state:&Arc<Fixture>)->Result<PowerAttestation> {
    verify_same_fixture(state)?;
    let power=state.power.get().ok_or("Power objects not constructed")?;
    let payload=power.verify(state)?;
    Ok(PowerAttestation {sources:power.source.pins.clone(),
        code:power.source.code.iter().map(|(name,_,identity)|(*name,identity.clone())).collect(),
        description:power.description,list:power.list,allocator:power.source.allocator,array_callbacks:power.source.callbacks,
        validated_input_bytes:power.validated_input_bytes,payload})
}
pub(crate) unsafe extern "C" fn owned_power_info() -> Ptr {
    callback_boundary(std::ptr::null_mut(), || {
        let Some(state) = fixture() else {
            return std::ptr::null_mut();
        };
        state
            .power()
            .and_then(|value|value.retained(state,value.description).ok())
            .unwrap_or(std::ptr::null_mut())
    })
}
pub(crate) unsafe extern "C" fn owned_power_list(info: Ptr) -> Ptr {
    callback_boundary(std::ptr::null_mut(), || {
        let Some(state) = fixture() else {
            return std::ptr::null_mut();
        };
        let Some(p) = state.power() else {
            return std::ptr::null_mut();
        };
        if info as usize != p.description {
            return std::ptr::null_mut();
        }
        p.retained(state,p.list).unwrap_or(std::ptr::null_mut())
    })
}
pub(crate) unsafe extern "C" fn owned_power_description(info: Ptr, source: Ptr) -> Ptr {
    callback_boundary(std::ptr::null_mut(), || {
        let Some(state) = fixture() else {
            return std::ptr::null_mut();
        };
        let Some(p) = state.power() else {
            return std::ptr::null_mut();
        };
        if info as usize != p.description || source as usize != p.description || p.verify(state).is_err() {
            return std::ptr::null_mut();
        }
        p.description as Ptr
    })
}

/// Synthetic monitor uptime plus actual elapsed monotonic time for deadlines.
/// No host uptime query is involved, and deadlines never see a frozen clock.
pub fn metadata() -> Result<f64> {
    metadata_fast().ok_or_else(|| "system fixture missing/retired".to_string())
}
/// Allocation-free callback gate, including the not-yet-installed path.
pub fn metadata_fast() -> Option<f64> {
    let state = fixture()?;
    Some(UPTIME_ORIGIN + state.monotonic_start.elapsed().as_secs_f64())
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    #[link_name = "IOPSCopyPowerSourcesInfo"]
    fn original_power_info() -> Ptr;
    #[link_name = "IOPSCopyPowerSourcesList"]
    fn original_power_list(info: Ptr) -> Ptr;
    #[link_name = "IOPSGetPowerSourceDescription"]
    fn original_power_description(info: Ptr, source: Ptr) -> Ptr;
    fn mach_host_self() -> u32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
    fn getpagesize() -> i32;
}
pub fn replacement(symbol: &CStr) -> Option<usize> {
    let values: [(&CStr, usize); 15] = [
        (c"sysctlbyname", owned_sysctlbyname as *const () as usize),
        (
            c"proc_listallpids",
            owned_proc_listallpids as *const () as usize,
        ),
        (c"proc_pidinfo", owned_proc_pidinfo as *const () as usize),
        (c"proc_pidpath", owned_proc_pidpath as *const () as usize),
        (
            c"mach_host_self",
            owned_mach_host_self as *const () as usize,
        ),
        (
            c"mach_port_deallocate",
            owned_mach_port_deallocate as *const () as usize,
        ),
        (c"getpagesize", owned_getpagesize as *const () as usize),
        (
            c"host_statistics",
            owned_host_statistics as *const () as usize,
        ),
        (
            c"host_statistics64",
            owned_host_statistics64 as *const () as usize,
        ),
        (c"statfs", owned_statfs as *const () as usize),
        (c"sysctl", owned_sysctl as *const () as usize),
        (
            c"if_indextoname",
            owned_if_indextoname as *const () as usize,
        ),
        (
            c"IOPSCopyPowerSourcesInfo",
            owned_power_info as *const () as usize,
        ),
        (
            c"IOPSCopyPowerSourcesList",
            owned_power_list as *const () as usize,
        ),
        (
            c"IOPSGetPowerSourceDescription",
            owned_power_description as *const () as usize,
        ),
    ];
    values
        .into_iter()
        .find_map(|(n, p)| (n == symbol).then_some(p))
}

#[path = "system_providers/import_patch.rs"]
pub mod import_patch;
pub fn import_specs() -> Vec<super::ImportSpec> {
    let values: [(&CStr, usize); 15] = [
        (c"sysctlbyname", libc::sysctlbyname as *const () as usize),
        (
            c"proc_listallpids",
            libc::proc_listallpids as *const () as usize,
        ),
        (c"proc_pidinfo", libc::proc_pidinfo as *const () as usize),
        (c"proc_pidpath", libc::proc_pidpath as *const () as usize),
        (c"mach_host_self", mach_host_self as *const () as usize),
        (
            c"mach_port_deallocate",
            mach_port_deallocate as *const () as usize,
        ),
        (c"getpagesize", getpagesize as *const () as usize),
        (
            c"host_statistics",
            libc::host_statistics as *const () as usize,
        ),
        (
            c"host_statistics64",
            libc::host_statistics64 as *const () as usize,
        ),
        (c"statfs", libc::statfs as *const () as usize),
        (c"sysctl", libc::sysctl as *const () as usize),
        (
            c"if_indextoname",
            libc::if_indextoname as *const () as usize,
        ),
        (
            c"IOPSCopyPowerSourcesInfo",
            original_power_info as *const () as usize,
        ),
        (
            c"IOPSCopyPowerSourcesList",
            original_power_list as *const () as usize,
        ),
        (
            c"IOPSGetPowerSourceDescription",
            original_power_description as *const () as usize,
        ),
    ];
    values
        .into_iter()
        .map(|(symbol, original)| super::ImportSpec {
            symbol,
            original,
            replacement: replacement(symbol).expect("hardcoded symbol replacement"),
        })
        .collect()
}
