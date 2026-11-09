//! Only the PID owned by a live, unreaped Child is observable through this API.
use super::{json, Result, Value};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct OwnedChild {
    child: Child,
    birth: Option<u64>,
}
// Exact public Darwin mach/mach_time.h ABI (two uint32_t fields). A single
// immutable clock query needs no new crate; libc deprecated its Mach namespace.
#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}
unsafe extern "C" {
    #[link_name = "mach_timebase_info"]
    fn clock_timebase(info: *mut MachTimebase) -> i32;
}
fn timebase() -> Result<(u32, u32)> {
    let mut tb = MachTimebase { numer: 0, denom: 0 };
    if unsafe { clock_timebase(&mut tb) } != 0 || tb.denom == 0 || tb.numer == 0 {
        return Err("Mach timebase unavailable".into());
    }
    Ok((tb.numer, tb.denom))
}
fn ticks_ns(ticks: u64, numer: u32, denom: u32) -> Result<u64> {
    if denom == 0 {
        return Err("zero Mach timebase denominator".into());
    }
    Ok(((ticks as u128) * (numer as u128) / (denom as u128)).min(u64::MAX as u128) as u64)
}
fn usage(pid: u32) -> Result<libc::rusage_info_v4> {
    let mut usage: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::proc_pid_rusage(
            pid as i32,
            libc::RUSAGE_INFO_V4,
            (&mut usage as *mut libc::rusage_info_v4).cast(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(usage)
}
fn ensure_unreaped(pid: u32) -> Result<()> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if info.si_pid != 0 {
        return Err("owned child exited; remains unreaped, no kernelcounter reuse".into());
    }
    Ok(())
}
fn counters(pid: u32, birth: &mut Option<u64>) -> Result<Value> {
    ensure_unreaped(pid)?;
    // No proc enumeration, user/account lookup or sampling of another process.
    let usage = usage(pid)?;
    let (numer, denom) = timebase()?;
    let raw_cpu = usage.ri_user_time.saturating_add(usage.ri_system_time);
    if birth.is_some_and(|b| b != usage.ri_proc_start_abstime) {
        return Err("owned child identity changed".into());
    }
    *birth = Some(usage.ri_proc_start_abstime);
    let mut task: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut task as *mut libc::proc_taskinfo).cast(),
            std::mem::size_of::<libc::proc_taskinfo>() as i32,
        )
    };
    if n != std::mem::size_of::<libc::proc_taskinfo>() as i32 {
        return Err("owned task counters unavailable".into());
    }
    let after = self::usage(pid)?;
    if after.ri_proc_start_abstime == 0
        || after.ri_proc_start_abstime != usage.ri_proc_start_abstime
    {
        return Err("owned kernel counter identity changed during query".into());
    }
    ensure_unreaped(pid)?;
    Ok(
        json!({"resource_source":"owned child kernel counters","pid":pid,"start_abstime":usage.ri_proc_start_abstime,"cpu_ns":ticks_ns(raw_cpu,numer,denom)?,"cpu_raw_mach_ticks":raw_cpu,"timebase_numer":numer,"timebase_denom":denom,"interrupt_wakeups":usage.ri_interrupt_wkups,"idle_wakeups":usage.ri_pkg_idle_wkups,"disk_read_bytes":usage.ri_diskio_bytesread,"disk_write_bytes":usage.ri_diskio_byteswritten,"pageins":usage.ri_pageins,"faults":task.pti_faults,"physical_footprint_bytes":usage.ri_phys_footprint,"rss_bytes":usage.ri_resident_size}),
    )
}
/// The borrower cannot reap/reuse the Guardian PID while kernel queries run.
/// Endpoint watcher never reaps; its exclusive Process remains the owner.
pub(super) fn snapshot_guardian(
    process: &crate::guardian::Process,
    birth: &mut Option<u64>,
) -> Result<Value> {
    counters(process.id() as u32, birth)
}

impl OwnedChild {
    pub fn spawn(mut command: Command) -> Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(Self {
            child: command.spawn().map_err(|e| e.to_string())?,
            birth: None,
        })
    }
    pub fn snapshot(&mut self) -> Result<Value> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("owned child exited; never observe a recycled PID".into());
        }
        counters(self.child.id(), &mut self.birth)
    }

    pub fn finish(&mut self) -> Result<()> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            self.child.kill().map_err(|e| e.to_string())?;
        }
        self.child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
pub fn self_test() -> Result<Value> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    let owned_home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    command
        .arg("owned-resource-child")
        .env_clear()
        .env("HOME", owned_home)
        .env("RUYCAST_HOME", owned_home)
        .env("RUSTCAST_HOME", owned_home);
    let mut owned = OwnedChild::spawn(command)?;
    let start = Instant::now();
    let mut raw = vec![];
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(100));
        let mut snapshot = owned.snapshot()?;
        snapshot["monotonic_ns"] = json!(start.elapsed().as_nanos());
        raw.push(snapshot);
    }
    owned.finish()?;
    if owned.snapshot().is_ok() {
        return Err("exited PID was observable".into());
    }
    Ok(
        json!({"scope":"runner resource primitive self-test, own Rust sleeping child only; not full host idle, fault recovery or lifecycle acceptance","raw":raw,"child_reaped":true,"no_host_enumeration":true}),
    )
}
pub fn child() {
    let mut bytes = vec![0_u8; 2 * 1024 * 1024];
    for i in (0..bytes.len()).step_by(4096) {
        bytes[i] = 1;
    }
    std::hint::black_box(&bytes);
    std::thread::sleep(Duration::from_secs(30));
}

pub fn counter_units() -> Result<Value> {
    fn get() -> Result<i128> {
        let mut r: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(
            (r.ru_utime.tv_sec as i128 + r.ru_stime.tv_sec as i128) * 1_000_000_000
                + (r.ru_utime.tv_usec as i128 + r.ru_stime.tv_usec as i128) * 1000,
        )
    }
    let (numer, denom) = timebase()?;
    let pid = std::process::id();
    let reference_before = get()?;
    let a = usage(pid)?;
    let start = Instant::now();
    let threads = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let start = Instant::now();
                let mut x = 1_u64;
                while start.elapsed() < Duration::from_millis(250) {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    std::hint::black_box(x);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread
            .join()
            .map_err(|_| "own counter-unit thread panicked")?;
    }
    let b = usage(pid)?;
    let reference_after = get()?;
    let ticks = (b.ri_user_time + b.ri_system_time)
        .checked_sub(a.ri_user_time + a.ri_system_time)
        .ok_or("CPU counter reset")?;
    let observed = ticks_ns(ticks, numer, denom)?;
    let reference = reference_after - reference_before;
    if reference <= 0 {
        return Err("nonpositive own CPU reference interval".into());
    }
    let ratio = observed as f64 / reference as f64;
    Ok(
        json!({"ok":(0.95..1.05).contains(&ratio),"scope":"own-process CPU counter-unit calibration only; no whole-host idle/energy/SDK query","pid":pid,"threads":4,"raw_mach_delta":ticks,"timebase_numer":numer,"timebase_denom":denom,"converted_cpu_ns":observed,"getrusage_cpu_ns":reference,"ratio":ratio,"wall_ns":start.elapsed().as_nanos()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticks_are_scaled_with_wide_intermediate_and_zero_is_rejected() {
        assert_eq!(ticks_ns(24_000_000, 125, 3).unwrap(), 1_000_000_000);
        assert_eq!(ticks_ns(u64::MAX, u32::MAX, 1).unwrap(), u64::MAX);
        assert!(ticks_ns(1, 1, 0).is_err());
    }
}
