//! Read-only numeric sampling. One worker owns the previous CPU/network sample.
use objc2::{rc::autoreleasepool, rc::Retained};
use objc2_foundation::{
    NSByteCountFormatter, NSByteCountFormatterCountStyle, NSDate, NSISO8601DateFormatter,
    NSProcessInfo, NSString,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{c_void, CStr},
    mem::MaybeUninit,
};
type Ptr = *mut c_void;
extern "C" {
    fn mach_host_self() -> u32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
    fn getpagesize() -> i32;
    static mach_task_self_: u32;
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(skip)]
    pub timestamp: f64,
    pub sampled_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    pub cpu_cores: usize,
    pub chip: String,
    pub model: String,
    pub os: String,
    pub hostname: String,
    pub uptime: f64,
    pub process_count: usize,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_wired: u64,
    pub memory_compressed: u64,
    pub swap_used: u64,
    pub disk_total: u64,
    pub disk_available: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_percent: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_source: Option<String>,
    pub network_received: u64,
    pub network_sent: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_bytes_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_bytes_per_second: Option<f64>,
    pub interfaces: Vec<String>,
}
pub struct Raw {
    pub snapshot: Snapshot,
    pub timestamp: f64,
    pub cpu: Option<[u32; 4]>,
}
pub trait Source: Send + Sync {
    fn collect(&self) -> Raw;
}
#[derive(Default)]
pub struct Sampler {
    previous_cpu: Option<[u32; 4]>,
    previous_network: Option<(f64, u64, u64)>,
}
impl Sampler {
    pub fn sample(&mut self, source: &dyn Source) -> Snapshot {
        let Raw {
            mut snapshot,
            timestamp,
            cpu,
        } = source.collect();
        snapshot.timestamp = timestamp;
        if let Some(cpu) = cpu {
            if let Some(previous) = self.previous_cpu {
                let mut deltas = [0u64; 4];
                for index in 0..4 {
                    deltas[index] = u64::from(cpu[index].wrapping_sub(previous[index]));
                }
                let total = deltas.iter().sum::<u64>();
                if total > 0 {
                    snapshot.cpu_percent = Some(100. * (total - deltas[2]) as f64 / total as f64);
                }
            }
            self.previous_cpu = Some(cpu);
        }
        if let Some((at, received, sent)) = self.previous_network {
            if timestamp.is_finite()
                && timestamp > at
                && snapshot.network_received >= received
                && snapshot.network_sent >= sent
            {
                let seconds = timestamp - at;
                snapshot.download_bytes_per_second =
                    Some((snapshot.network_received - received) as f64 / seconds);
                snapshot.upload_bytes_per_second =
                    Some((snapshot.network_sent - sent) as f64 / seconds);
            }
        }
        self.previous_network = Some((timestamp, snapshot.network_received, snapshot.network_sent));
        snapshot
    }
}
pub fn memory_label(bytes: u64) -> String {
    autoreleasepool(|_| {
        NSByteCountFormatter::stringFromByteCount_countStyle(
            bytes.min(i64::MAX as u64) as i64,
            NSByteCountFormatterCountStyle::Memory,
        )
        .to_string()
    })
}
pub struct Native;
// One local SDK table: tests replace every acquisition before entering the
// production collector. No process-wide interposition or native fallback.
struct Io {
    basics: fn() -> (Snapshot, f64),
    named: unsafe fn(&CStr, Ptr, &mut usize) -> i32,
    host: unsafe fn() -> u32,
    release_host: unsafe fn(u32),
    cpu: unsafe fn(u32, Ptr, &mut u32) -> i32,
    vm: unsafe fn(u32, Ptr, &mut u32) -> i32,
    page: unsafe fn() -> i32,
    disk: unsafe fn(*mut libc::statfs) -> i32,
    routes: unsafe fn(Ptr, &mut usize) -> i32,
    interface: fn(u32) -> Option<String>,
    power_info: unsafe fn() -> Ptr,
    power_list: unsafe fn(Ptr) -> Ptr,
    power_description: unsafe fn(Ptr, Ptr) -> Ptr,
}
impl Io {
    fn native() -> Self {
        Self {
            basics: || {
                let now = NSDate::date();
                let info = NSProcessInfo::processInfo();
                (
                    Snapshot {
                        sampled_at: NSISO8601DateFormatter::new()
                            .stringFromDate(&now)
                            .to_string(),
                        cpu_cores: info.processorCount(),
                        os: info.operatingSystemVersionString().to_string(),
                        hostname: info.hostName().to_string(),
                        uptime: info.systemUptime(),
                        process_count: unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) }
                            .max(0) as usize,
                        ..Default::default()
                    },
                    now.timeIntervalSince1970(),
                )
            },
            named: |name, out, size| unsafe {
                libc::sysctlbyname(name.as_ptr(), out, size, std::ptr::null_mut(), 0)
            },
            host: || unsafe { mach_host_self() },
            release_host: |host| unsafe {
                mach_port_deallocate(mach_task_self_, host);
            },
            cpu: |host, out, count| unsafe {
                libc::host_statistics(host, libc::HOST_CPU_LOAD_INFO, out.cast(), count)
            },
            vm: |host, out, count| unsafe {
                libc::host_statistics64(host, libc::HOST_VM_INFO64, out.cast(), count)
            },
            page: || unsafe { getpagesize() },
            disk: |out| unsafe { libc::statfs(c"/".as_ptr(), out) },
            routes: |out, size| unsafe {
                let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
                libc::sysctl(mib.as_mut_ptr(), 6, out, size, std::ptr::null_mut(), 0)
            },
            interface: |index| {
                let mut name = [0i8; libc::IFNAMSIZ];
                if unsafe { libc::if_indextoname(index, name.as_mut_ptr()) }.is_null() {
                    None
                } else {
                    Some(
                        unsafe { CStr::from_ptr(name.as_ptr()) }
                            .to_string_lossy()
                            .into_owned(),
                    )
                }
            },
            power_info: || unsafe { IOPSCopyPowerSourcesInfo() },
            power_list: |info| unsafe { IOPSCopyPowerSourcesList(info) },
            power_description: |info, source| unsafe {
                IOPSGetPowerSourceDescription(info, source)
            },
        }
    }
}
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> Ptr;
    fn IOPSCopyPowerSourcesList(info: Ptr) -> Ptr;
    fn IOPSGetPowerSourceDescription(info: Ptr, source: Ptr) -> Ptr;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ptr);
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(value: Ptr) -> isize;
    fn CFArrayGetValueAtIndex(value: Ptr, index: isize) -> Ptr;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(value: Ptr, key: Ptr) -> Ptr;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: Ptr, kind: isize, output: Ptr) -> u8;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ptr) -> u8;
    fn CFStringGetTypeID() -> usize;
}
struct Owned(Ptr);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
fn sysctl_value<T: Copy>(io: &Io, name: &CStr, mut fallback: T) -> T {
    let original = fallback;
    let mut size = std::mem::size_of::<T>();
    let status = unsafe { (io.named)(name, (&mut fallback as *mut T).cast(), &mut size) };
    if status == 0 && size == std::mem::size_of::<T>() {
        fallback
    } else {
        original
    }
}
fn sysctl_text(io: &Io, name: &CStr) -> String {
    let mut size = 0;
    let status = unsafe { (io.named)(name, std::ptr::null_mut(), &mut size) };
    if status != 0 || size == 0 || size >= 4096 {
        return "Unavailable".into();
    }
    let mut bytes = vec![0u8; size];
    if unsafe { (io.named)(name, bytes.as_mut_ptr().cast(), &mut size) } != 0 || size > bytes.len()
    {
        return "Unavailable".into();
    }
    let end = bytes[..size].iter().position(|b| *b == 0).unwrap_or(size);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
impl Source for Native {
    fn collect(&self) -> Raw {
        collect(&Io::native())
    }
}
fn collect(io: &Io) -> Raw {
    autoreleasepool(|_| {
        let (mut snapshot, timestamp) = (io.basics)();
        snapshot.chip = sysctl_text(io, c"machdep.cpu.brand_string");
        snapshot.model = sysctl_text(io, c"hw.model");
        snapshot.memory_total = sysctl_value(io, c"hw.memsize", 0u64);
        let host = unsafe { (io.host)() };
        struct Host<'a>(u32, &'a Io);
        impl Drop for Host<'_> {
            fn drop(&mut self) {
                unsafe { (self.1.release_host)(self.0) }
            }
        }
        let _host = Host(host, io);
        let mut cpu = MaybeUninit::<libc::host_cpu_load_info>::zeroed();
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let cpu = if unsafe { (io.cpu)(host, cpu.as_mut_ptr().cast(), &mut count) } == 0
            && count == libc::HOST_CPU_LOAD_INFO_COUNT
        {
            Some(unsafe { cpu.assume_init() }.cpu_ticks)
        } else {
            None
        };
        let mut vm = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        if unsafe { (io.vm)(host, vm.as_mut_ptr().cast(), &mut count) } == 0
            && count == libc::HOST_VM_INFO64_COUNT
        {
            let vm = unsafe { vm.assume_init() };
            let page = unsafe { (io.page)() }.max(0) as u64;
            snapshot.memory_wired = u64::from(vm.wire_count).saturating_mul(page);
            snapshot.memory_compressed = u64::from(vm.compressor_page_count).saturating_mul(page);
            snapshot.memory_used = u64::from(vm.active_count)
                .saturating_mul(page)
                .saturating_add(snapshot.memory_wired)
                .saturating_add(snapshot.memory_compressed)
                .min(snapshot.memory_total);
        }
        let swap: libc::xsw_usage =
            sysctl_value(io, c"vm.swapusage", unsafe { std::mem::zeroed() });
        snapshot.swap_used = swap.xsu_used;
        let mut disk = MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { (io.disk)(disk.as_mut_ptr()) } == 0 {
            let disk = unsafe { disk.assume_init() };
            snapshot.disk_total = disk.f_blocks.saturating_mul(u64::from(disk.f_bsize));
            snapshot.disk_available = disk.f_bavail.saturating_mul(u64::from(disk.f_bsize));
        }
        power(io, &mut snapshot);
        let network = network(io);
        snapshot.network_received = network.0;
        snapshot.network_sent = network.1;
        snapshot.interfaces = network.2;
        Raw {
            snapshot,
            timestamp,
            cpu,
        }
    })
}
fn power(io: &Io, snapshot: &mut Snapshot) {
    let info = Owned(unsafe { (io.power_info)() });
    if info.0.is_null() {
        return;
    }
    let sources = Owned(unsafe { (io.power_list)(info.0) });
    if sources.0.is_null() || unsafe { CFGetTypeID(sources.0) != CFArrayGetTypeID() } {
        return;
    }
    let count = unsafe { CFArrayGetCount(sources.0) };
    if !(0..=256).contains(&count) {
        return;
    }
    for index in 0..count {
        let source = unsafe { CFArrayGetValueAtIndex(sources.0, index) };
        if source.is_null() {
            continue;
        }
        let description = unsafe { (io.power_description)(info.0, source) };
        if description.is_null() || unsafe { CFGetTypeID(description) != CFDictionaryGetTypeID() } {
            continue;
        }
        let value = |key: &str| {
            let key = NSString::from_str(key);
            unsafe { CFDictionaryGetValue(description, Retained::as_ptr(&key).cast_mut().cast()) }
        };
        let number = |key: &str| {
            let value = value(key);
            if value.is_null() || unsafe { CFGetTypeID(value) != CFNumberGetTypeID() } {
                return None;
            }
            let mut out = 0i64;
            if unsafe { CFNumberGetValue(value, 4, (&mut out as *mut i64).cast()) } != 0 {
                Some(out)
            } else {
                None
            }
        };
        let (Some(current), Some(maximum)) = (number("Current Capacity"), number("Max Capacity"))
        else {
            continue;
        };
        if maximum <= 0 {
            continue;
        }
        snapshot.battery_percent = Some((current.saturating_mul(100) / maximum).clamp(0, 100));
        let charging = value("Is Charging");
        if !charging.is_null() && unsafe { CFGetTypeID(charging) == CFBooleanGetTypeID() } {
            snapshot.charging = Some(unsafe { CFBooleanGetValue(charging) } != 0);
        }
        let source = value("Power Source State");
        if !source.is_null() && unsafe { CFGetTypeID(source) == CFStringGetTypeID() } {
            let text = unsafe { Retained::<NSString>::retain(source.cast()) };
            snapshot.power_source = text.map(|s| s.to_string());
        }
        break;
    }
}
fn network(io: &Io) -> (u64, u64, Vec<String>) {
    let mut size = 0;
    if unsafe { (io.routes)(std::ptr::null_mut(), &mut size) } != 0
        || size == 0
        || size >= 2_000_000
    {
        return (0, 0, vec![]);
    }
    let mut bytes = vec![0u8; size];
    if unsafe { (io.routes)(bytes.as_mut_ptr().cast(), &mut size) } != 0 || size > bytes.len() {
        return (0, 0, vec![]);
    }
    parse_network(&bytes[..size], io.interface)
}
pub fn parse_network(
    bytes: &[u8],
    mut name: impl FnMut(u32) -> Option<String>,
) -> (u64, u64, Vec<String>) {
    let mut offset = 0;
    let (mut received, mut sent) = (0u64, 0u64);
    let mut names = vec![];
    // if_msghdr2, ifa_msghdr and ifma_msghdr share only this prefix.
    // NET_RT_IFLIST2 interleaves shorter address records between interfaces.
    const PREFIX: usize = 4;
    while bytes.len().saturating_sub(offset) >= PREFIX {
        let length = usize::from(u16::from_ne_bytes([bytes[offset], bytes[offset + 1]]));
        let version = bytes[offset + 2];
        let kind = bytes[offset + 3];
        let Some(end) = offset.checked_add(length).filter(|end| *end <= bytes.len()) else {
            break;
        };
        if length < PREFIX || i32::from(version) != libc::RTM_VERSION {
            break;
        }
        if i32::from(kind) == libc::RTM_IFINFO2 {
            if length < std::mem::size_of::<libc::if_msghdr2>() {
                break;
            }
            let row = unsafe {
                std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<libc::if_msghdr2>())
            };
            if row.ifm_flags & libc::IFF_UP != 0 && row.ifm_flags & libc::IFF_LOOPBACK == 0 {
                if let Some(name) = name(u32::from(row.ifm_index)).filter(|s| s.starts_with("en")) {
                    received = received.wrapping_add(row.ifm_data.ifi_ibytes);
                    sent = sent.wrapping_add(row.ifm_data.ifi_obytes);
                    names.push(name);
                }
            }
        }
        offset = end;
    }
    names.sort();
    (received, sent, names)
}
#[cfg(any(test, feature = "test-tools"))]
#[derive(Default)]
pub struct Fixture {
    next: std::sync::atomic::AtomicU64,
}
#[cfg(any(test, feature = "test-tools"))]
impl Source for Fixture {
    fn collect(&self) -> Raw {
        let sample = self.next.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Raw {
            snapshot: Snapshot {
                sampled_at: chrono::DateTime::from_timestamp(1_793_534_400 + sample as i64 * 2, 0)
                    .expect("bounded fixture timestamp")
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                cpu_cores: 8,
                chip: "Owned Chip".into(),
                model: "Owned Model".into(),
                os: "Owned macOS".into(),
                hostname: "owned.invalid".into(),
                uptime: 90000.,
                process_count: 2,
                memory_total: 16 * 1_073_741_824,
                memory_used: 8 * 1_073_741_824,
                memory_wired: 1_073_741_824,
                memory_compressed: 512 * 1_048_576,
                swap_used: 256 * 1_048_576,
                disk_total: 500 * 1_073_741_824,
                disk_available: 200 * 1_073_741_824,
                battery_percent: Some(75),
                charging: Some(false),
                power_source: Some("Battery Power".into()),
                network_received: 4096 + sample * 2048,
                network_sent: 2048 + sample * 1024,
                interfaces: vec!["en0".into()],
                ..Default::default()
            },
            timestamp: 100. + sample as f64 * 2.,
            cpu: Some([
                100 + sample as u32 * 5,
                50 + sample as u32 * 3,
                200 + sample as u32 * 12,
                0,
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route(kind: i32, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        bytes[..2].copy_from_slice(&(length as u16).to_ne_bytes());
        bytes[2] = libc::RTM_VERSION as u8;
        bytes[3] = kind as u8;
        bytes
    }
    fn interface(index: u16, flags: i32, received: u64, sent: u64) -> Vec<u8> {
        let mut bytes = route(libc::RTM_IFINFO2, std::mem::size_of::<libc::if_msghdr2>());
        let at = std::mem::offset_of!(libc::if_msghdr2, ifm_index);
        bytes[at..at + 2].copy_from_slice(&index.to_ne_bytes());
        let at = std::mem::offset_of!(libc::if_msghdr2, ifm_flags);
        bytes[at..at + 4].copy_from_slice(&flags.to_ne_bytes());
        let data = std::mem::offset_of!(libc::if_msghdr2, ifm_data);
        let at = data + std::mem::offset_of!(libc::if_data64, ifi_ibytes);
        bytes[at..at + 8].copy_from_slice(&received.to_ne_bytes());
        let at = data + std::mem::offset_of!(libc::if_data64, ifi_obytes);
        bytes[at..at + 8].copy_from_slice(&sent.to_ne_bytes());
        bytes
    }
    fn address(ipv6: bool) -> Vec<u8> {
        let header = std::mem::size_of::<libc::ifa_msghdr>();
        let size = if ipv6 {
            std::mem::size_of::<libc::sockaddr_in6>()
        } else {
            std::mem::size_of::<libc::sockaddr_in>()
        };
        let mut bytes = route(libc::RTM_NEWADDR, header + size * 2);
        let at = std::mem::offset_of!(libc::ifa_msghdr, ifam_addrs);
        bytes[at..at + 4].copy_from_slice(&(0x20i32 | 0x4).to_ne_bytes());
        let at = std::mem::offset_of!(libc::ifa_msghdr, ifam_index);
        bytes[at..at + 2].copy_from_slice(&1u16.to_ne_bytes());
        for start in [header, header + size] {
            bytes[start] = size as u8;
            bytes[start + 1] = if ipv6 { libc::AF_INET6 } else { libc::AF_INET } as u8;
        }
        if ipv6 {
            let at = header + std::mem::offset_of!(libc::sockaddr_in6, sin6_addr);
            bytes[at..at + 16].fill(255);
            bytes[header + size + std::mem::offset_of!(libc::sockaddr_in6, sin6_addr) + 15] = 1;
        } else {
            let at = header + std::mem::offset_of!(libc::sockaddr_in, sin_addr);
            bytes[at..at + 4].copy_from_slice(&[255, 0, 0, 0]);
            let at = header + size + std::mem::offset_of!(libc::sockaddr_in, sin_addr);
            bytes[at..at + 4].copy_from_slice(&[127, 0, 0, 1]);
        }
        bytes
    }
    #[test]
    fn mixed_sdk_route_records_preserve_later_interfaces_and_bound_malformed_input() {
        // Verify the SDK layouts used for the shared prefix and payload offsets.
        assert_eq!(std::mem::offset_of!(libc::if_msghdr2, ifm_version), 2);
        assert_eq!(std::mem::offset_of!(libc::ifa_msghdr, ifam_type), 3);
        assert_eq!(std::mem::offset_of!(libc::ifma_msghdr, ifmam_type), 3);
        let lo = interface(1, libc::IFF_UP | libc::IFF_LOOPBACK, 900, 900);
        let en0 = interface(2, libc::IFF_UP, 1234, 5678);
        let en3 = interface(3, libc::IFF_UP, 10, 20);
        let mut multicast = route(
            libc::RTM_NEWMADDR2,
            std::mem::size_of::<libc::ifma_msghdr2>() + std::mem::size_of::<libc::sockaddr_dl>(),
        );
        let at = std::mem::size_of::<libc::ifma_msghdr2>();
        multicast[at] = std::mem::size_of::<libc::sockaddr_dl>() as u8;
        multicast[at + 1] = libc::AF_LINK as u8;
        let flags = std::mem::offset_of!(libc::ifma_msghdr2, ifmam_addrs);
        multicast[flags..flags + 4].copy_from_slice(&0x20i32.to_ne_bytes());
        let index = std::mem::offset_of!(libc::ifma_msghdr2, ifmam_index);
        multicast[index..index + 2].copy_from_slice(&2u16.to_ne_bytes());
        multicast[at + std::mem::offset_of!(libc::sockaddr_dl, sdl_alen)] = 6;
        let data = at + std::mem::offset_of!(libc::sockaddr_dl, sdl_data);
        multicast[data..data + 6].copy_from_slice(&[1, 0, 94, 0, 0, 1]);
        let mut mixed = vec![];
        for row in [
            &lo,
            &address(false),
            &en0,
            &address(true),
            &multicast,
            &en3,
            &interface(4, 0, 99, 99),
            &interface(5, libc::IFF_UP, 99, 99),
        ] {
            mixed.extend(row);
        }
        let resolve = |index| match index {
            2 => Some("en0".into()),
            3 => Some("en3".into()),
            5 => Some("utun-owned".into()),
            _ => panic!("Address/down/loopback record resolved an interface"),
        };
        assert_eq!(address(false).len(), 52);
        assert_eq!(
            parse_network(&mixed, resolve),
            (1244, 5698, vec!["en0".into(), "en3".into()])
        );
        for end in 0..mixed.len() {
            let _ = parse_network(&mixed[..end], resolve);
        }
        for mut bad in [
            vec![0, 0, 5, 12],
            vec![3, 0, 5, 12],
            vec![255, 255, 5, 12],
            vec![4, 0, 5, libc::RTM_IFINFO2 as u8],
            vec![4, 0, 99, 12],
        ] {
            let mut data = en0.clone();
            data.append(&mut bad);
            data.extend(&en3);
            assert_eq!(
                parse_network(&data, resolve),
                (1234, 5678, vec!["en0".into()])
            );
        }
        let overflow = [
            interface(2, libc::IFF_UP, u64::MAX, u64::MAX - 1),
            interface(3, libc::IFF_UP, 10, 20),
        ]
        .concat();
        assert_eq!(
            parse_network(&overflow, resolve),
            (9, 18, vec!["en0".into(), "en3".into()])
        );
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RELEASED: AtomicUsize = AtomicUsize::new(0);
    extern "C" {
        fn CFNumberCreate(allocator: Ptr, kind: isize, value: *const c_void) -> Ptr;
        fn CFDictionaryCreate(
            allocator: Ptr,
            keys: *const Ptr,
            values: *const Ptr,
            count: isize,
            keys_callbacks: *const u8,
            values_callbacks: *const u8,
        ) -> Ptr;
        fn CFArrayCreate(allocator: Ptr, values: *const Ptr, count: isize, callbacks: Ptr) -> Ptr;
        static kCFTypeDictionaryKeyCallBacks: u8;
        static kCFTypeDictionaryValueCallBacks: u8;
        static kCFTypeArrayCallBacks: u8;
        static kCFBooleanTrue: Ptr;
    }
    fn owned_io() -> Io {
        Io {
            basics: || {
                (
                    Snapshot {
                        sampled_at: "2026-11-01T12:00:00Z".into(),
                        cpu_cores: 8,
                        os: "Owned OS".into(),
                        hostname: "owned.invalid".into(),
                        uptime: 90000.,
                        process_count: 7,
                        ..Default::default()
                    },
                    1000.,
                )
            },
            named: |name, out, size| unsafe {
                let bytes = match name.to_bytes() {
                    b"hw.memsize" => 65536u64.to_ne_bytes().to_vec(),
                    b"hw.model" => b"Owned Model\0".to_vec(),
                    b"machdep.cpu.brand_string" => b"Owned Chip\0".to_vec(),
                    b"vm.swapusage" => {
                        let mut swap: libc::xsw_usage = std::mem::zeroed();
                        swap.xsu_used = 1234;
                        std::slice::from_raw_parts(
                            (&swap as *const libc::xsw_usage).cast(),
                            std::mem::size_of_val(&swap),
                        )
                        .to_vec()
                    }
                    _ => panic!("Unsubstituted SDK acquisition"),
                };
                if !out.is_null() {
                    assert!(*size >= bytes.len());
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), bytes.len());
                }
                *size = bytes.len();
                0
            },
            host: || 42,
            release_host: |host| {
                assert_eq!(host, 42);
                RELEASED.fetch_add(1, Ordering::AcqRel);
            },
            cpu: |host, out, count| unsafe {
                assert_eq!(host, 42);
                assert_eq!(*count, libc::HOST_CPU_LOAD_INFO_COUNT);
                out.cast::<libc::host_cpu_load_info>()
                    .write(libc::host_cpu_load_info {
                        cpu_ticks: [1, 2, 7, 0],
                    });
                0
            },
            vm: |host, out, count| unsafe {
                assert_eq!(host, 42);
                assert_eq!(*count, libc::HOST_VM_INFO64_COUNT);
                let mut vm: libc::vm_statistics64 = std::mem::zeroed();
                vm.active_count = 3;
                vm.wire_count = 2;
                vm.compressor_page_count = 1;
                out.cast::<libc::vm_statistics64>().write(vm);
                0
            },
            page: || 4096,
            disk: |out| unsafe {
                let mut disk: libc::statfs = std::mem::zeroed();
                disk.f_blocks = 100;
                disk.f_bavail = 20;
                disk.f_bsize = 4096;
                out.write(disk);
                0
            },
            routes: |out, size| unsafe {
                let mut row: libc::if_msghdr2 = std::mem::zeroed();
                row.ifm_msglen = std::mem::size_of_val(&row) as u16;
                row.ifm_type = libc::RTM_IFINFO2 as u8;
                row.ifm_version = libc::RTM_VERSION as u8;
                row.ifm_flags = libc::IFF_UP;
                row.ifm_index = 1;
                row.ifm_data.ifi_ibytes = 800;
                row.ifm_data.ifi_obytes = 400;
                if !out.is_null() {
                    assert!(*size >= std::mem::size_of_val(&row));
                    out.cast::<libc::if_msghdr2>().write_unaligned(row);
                }
                *size = std::mem::size_of_val(&row);
                0
            },
            interface: |index| {
                assert_eq!(index, 1);
                Some("en-owned".into())
            },
            power_info: || unsafe {
                let keys = [
                    NSString::from_str("Current Capacity"),
                    NSString::from_str("Max Capacity"),
                    NSString::from_str("Is Charging"),
                    NSString::from_str("Power Source State"),
                ];
                let keys = keys
                    .iter()
                    .map(|k| Retained::as_ptr(k).cast_mut().cast())
                    .collect::<Vec<Ptr>>();
                let current = Owned(CFNumberCreate(
                    std::ptr::null_mut(),
                    4,
                    (&mut 3i64 as *mut i64).cast(),
                ));
                let maximum = Owned(CFNumberCreate(
                    std::ptr::null_mut(),
                    4,
                    (&mut 4i64 as *mut i64).cast(),
                ));
                let source = NSString::from_str("AC Power");
                let values = [
                    current.0,
                    maximum.0,
                    kCFBooleanTrue,
                    Retained::as_ptr(&source).cast_mut().cast(),
                ];
                CFDictionaryCreate(
                    std::ptr::null_mut(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    4,
                    (&raw const kCFTypeDictionaryKeyCallBacks).cast_mut().cast(),
                    (&raw const kCFTypeDictionaryValueCallBacks)
                        .cast_mut()
                        .cast(),
                )
            },
            power_list: |info| unsafe {
                CFArrayCreate(
                    std::ptr::null_mut(),
                    &info,
                    1,
                    (&raw const kCFTypeArrayCallBacks).cast_mut().cast(),
                )
            },
            power_description: |info, source| {
                assert_eq!(info, source);
                info
            },
        }
    }
    #[test]
    fn native_collector_uses_complete_local_sdk_table_and_rejects_malformed_counts() {
        let before = RELEASED.load(Ordering::Acquire);
        let raw = collect(&owned_io());
        let s = raw.snapshot;
        assert_eq!(
            (s.hostname.as_str(), s.model.as_str(), s.chip.as_str()),
            ("owned.invalid", "Owned Model", "Owned Chip")
        );
        assert_eq!((s.cpu_cores, s.process_count, s.uptime), (8, 7, 90000.));
        assert_eq!(raw.cpu, Some([1, 2, 7, 0]));
        assert_eq!(
            (
                s.memory_total,
                s.memory_used,
                s.memory_wired,
                s.memory_compressed,
                s.swap_used
            ),
            (65536, 24576, 8192, 4096, 1234)
        );
        assert_eq!((s.disk_total, s.disk_available), (409600, 81920));
        assert_eq!(
            (s.battery_percent, s.charging, s.power_source.as_deref()),
            (Some(75), Some(true), Some("AC Power"))
        );
        assert_eq!(
            (s.network_received, s.network_sent, s.interfaces),
            (800, 400, vec!["en-owned".into()])
        );
        assert_eq!(raw.timestamp, 1000.);
        let mut broken = owned_io();
        broken.named = |_, _, size| {
            *size = usize::MAX;
            -1
        };
        broken.cpu = |_, _, count| {
            *count = 0;
            0
        };
        broken.vm = |_, _, count| {
            *count = 0;
            0
        };
        broken.page = || panic!("Malformed VM must not read page size");
        broken.disk = |_| -1;
        broken.routes = |_, size| {
            *size = 2_000_001;
            0
        };
        broken.interface = |_| panic!("Oversize route buffer must not resolve an interface");
        broken.power_list = |_| std::ptr::null_mut();
        broken.power_description = |_, _| panic!("Missing list must not read power description");
        let bad = collect(&broken);
        assert!(bad.cpu.is_none());
        assert_eq!(
            (
                bad.snapshot.memory_total,
                bad.snapshot.memory_used,
                bad.snapshot.disk_total
            ),
            (0, 0, 0)
        );
        assert!(bad.snapshot.battery_percent.is_none());
        assert!(bad.snapshot.interfaces.is_empty());
        assert_eq!(bad.snapshot.chip, "Unavailable");
        assert_eq!(RELEASED.load(Ordering::Acquire), before + 2);
    }
    struct OwnedSource {
        time: f64,
        cpu: [u32; 4],
        received: u64,
    }
    impl Source for OwnedSource {
        fn collect(&self) -> Raw {
            Raw {
                snapshot: Snapshot {
                    network_received: self.received,
                    network_sent: self.received / 2,
                    ..Default::default()
                },
                timestamp: self.time,
                cpu: Some(self.cpu),
            }
        }
    }
    #[test]
    fn cpu_wrap_network_rates_and_reset_are_measured_between_samples() {
        let mut sampler = Sampler::default();
        let a = OwnedSource {
            time: 10.,
            cpu: [u32::MAX - 4, 20, 50, 0],
            received: 100,
        };
        assert!(sampler.sample(&a).cpu_percent.is_none());
        let b = OwnedSource {
            time: 12.,
            cpu: [5, 25, 65, 0],
            received: 300,
        };
        let sample = sampler.sample(&b);
        assert_eq!(sample.cpu_percent, Some(50.));
        assert_eq!(sample.download_bytes_per_second, Some(100.));
        assert_eq!(sample.upload_bytes_per_second, Some(50.));
        let c = OwnedSource {
            time: 13.,
            cpu: b.cpu,
            received: 1,
        };
        assert!(sampler.sample(&c).download_bytes_per_second.is_none());
    }
    #[test]
    fn invalid_route_buffer_is_bounded() {
        assert_eq!(
            parse_network(&[0; 1024], |_| panic!("No valid interface")),
            (0, 0, vec![])
        );
        assert_eq!(memory_label(1024), "1 KB");
    }
}
