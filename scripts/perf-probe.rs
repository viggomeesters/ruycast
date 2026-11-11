//! Read-only Darwin process counters and own-process CPU-unit calibration.
use serde_json::{json, Value};
use std::{hint::black_box, process::ExitCode};

// Exact public mach/mach_time.h ABI; avoid libc's deprecated Mach namespace.
#[repr(C)]
#[derive(Clone, Copy)]
struct Timebase {
    numer: u32,
    denom: u32,
}
unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    fn mach_absolute_time() -> u64;
}
type Result<T> = std::result::Result<T, String>;

fn timebase() -> Result<Timebase> {
    let mut value = Timebase { numer: 0, denom: 0 };
    if unsafe { mach_timebase_info(&mut value) } != 0 || value.denom == 0 {
        return Err("Mach timebase unavailable".into());
    }
    Ok(value)
}
fn ns(ticks: u64, tb: Timebase) -> u64 {
    ((ticks as u128 * tb.numer as u128 / tb.denom as u128).min(u64::MAX as u128)) as u64
}
fn usage(pid: i32) -> Result<libc::rusage_info_v4> {
    let mut value: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            (&mut value as *mut libc::rusage_info_v4).cast(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(value)
}
fn reference_cpu_ns() -> Result<i128> {
    let mut value: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut value) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(
        (value.ru_utime.tv_sec as i128 + value.ru_stime.tv_sec as i128) * 1_000_000_000
            + (value.ru_utime.tv_usec as i128 + value.ru_stime.tv_usec as i128) * 1_000,
    )
}
fn snapshot(pid: i32, tb: Timebase) -> Result<Value> {
    let r = usage(pid)?;
    let raw = r.ri_user_time.wrapping_add(r.ri_system_time);
    Ok(json!({
        "cpuNS": ns(raw, tb), "cpuRawMach": raw,
        "timebaseNumer": tb.numer, "timebaseDenom": tb.denom,
        "footprintBytes": r.ri_phys_footprint, "residentBytes": r.ri_resident_size,
        "peakFootprintBytes": r.ri_lifetime_max_phys_footprint,
        "packageIdleWakeups": r.ri_pkg_idle_wkups, "interruptWakeups": r.ri_interrupt_wkups,
        "diskReadBytes": r.ri_diskio_bytesread, "diskWriteBytes": r.ri_diskio_byteswritten,
        "logicalWriteBytes": r.ri_logical_writes
    }))
}
fn self_check(tb: Timebase) -> Result<Value> {
    let before = reference_cpu_ns()?;
    let a = usage(std::process::id() as i32)?;
    let start = unsafe { mach_absolute_time() };
    let mut threads = Vec::with_capacity(4);
    let mut spawn_error = None;
    for _ in 0..4 {
        match std::thread::Builder::new().spawn(move || {
            let start = unsafe { mach_absolute_time() };
            let mut value = 1_u64;
            while ns(unsafe { mach_absolute_time() }.wrapping_sub(start), tb) < 250_000_000 {
                value = value.wrapping_mul(1664525).wrapping_add(1013904223);
                black_box(value);
            }
        }) {
            Ok(thread) => threads.push(thread),
            Err(error) => {
                spawn_error = Some(error.to_string());
                break;
            }
        }
    }
    // Always reap already-created workers, including a later spawn failure.
    let mut panicked = false;
    for thread in threads {
        panicked |= thread.join().is_err();
    }
    if let Some(error) = spawn_error {
        return Err(error);
    }
    if panicked {
        return Err("CPU calibration worker panicked".into());
    }
    let b = usage(std::process::id() as i32)?;
    let after = reference_cpu_ns()?;
    let ticks = b
        .ri_user_time
        .wrapping_add(b.ri_system_time)
        .wrapping_sub(a.ri_user_time.wrapping_add(a.ri_system_time));
    let observed = ns(ticks, tb) as f64 / 1e9;
    let reference = (after - before) as f64 / 1e9;
    let ratio = observed / reference;
    Ok(json!({
        "ok": ratio > 0.95 && ratio < 1.05,
        "procCPUSeconds": observed, "getrusageCPUSeconds": reference,
        "wallSeconds": ns(unsafe { mach_absolute_time() }.wrapping_sub(start), tb) as f64 / 1e9,
        "ratio": ratio, "threads": 4, "timebaseNumer": tb.numer, "timebaseDenom": tb.denom
    }))
}
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return ExitCode::from(2);
    }
    let result = timebase().and_then(|tb| {
        if args[0] == "--self-check" {
            self_check(tb)
        } else {
            let pid: i32 = args[0].parse().map_err(|_| "Expected a positive PID")?;
            if pid <= 0 {
                return Err("Expected a positive PID".into());
            }
            snapshot(pid, tb)
        }
    });
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::from(u8::from(value.get("ok") == Some(&Value::Bool(false))))
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mach_conversion_preserves_units_and_wide_saturation() {
        assert_eq!(
            ns(
                24_000_000,
                Timebase {
                    numer: 125,
                    denom: 3
                }
            ),
            1_000_000_000
        );
        assert_eq!(
            ns(
                u64::MAX,
                Timebase {
                    numer: u32::MAX,
                    denom: 1
                }
            ),
            u64::MAX
        );
    }
    #[test]
    fn own_process_snapshot_preserves_all_counter_fields_and_native_units() {
        let tb = timebase().unwrap();
        let value = snapshot(std::process::id() as i32, tb).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 12);
        for key in [
            "cpuNS",
            "cpuRawMach",
            "timebaseNumer",
            "timebaseDenom",
            "footprintBytes",
            "residentBytes",
            "peakFootprintBytes",
            "packageIdleWakeups",
            "interruptWakeups",
            "diskReadBytes",
            "diskWriteBytes",
            "logicalWriteBytes",
        ] {
            assert!(value[key].as_u64().is_some(), "{key}");
        }
        assert_eq!(
            value["cpuNS"],
            ns(value["cpuRawMach"].as_u64().unwrap(), tb)
        );
        assert!(value["residentBytes"].as_u64().unwrap() > 0);
        assert!(usage(i32::MAX).is_err());
    }
}
