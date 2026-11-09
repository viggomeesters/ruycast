use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use ruycast_native::workspace::{self, OpenRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    request: OpenRequest,
    #[serde(default = "timeout")]
    timeout_ms: u64,
    #[serde(default)]
    cancel_after_ms: Option<u64>,
    #[serde(default)]
    reject_before_commit: bool,
    #[serde(default)]
    cancel_after_open: bool,
    #[serde(default)]
    panic_before_commit: bool,
    #[serde(default)]
    panic_after_open: bool,
}
fn timeout() -> u64 {
    500
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    cases: Vec<Case>,
    #[serde(default)]
    release_callbacks: bool,
    #[serde(default)]
    settle_ms: u64,
}
fn run() -> Result<Value, String> {
    let mtm = MainThreadMarker::new().ok_or("Main thread required")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let home = PathBuf::from(
        std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME")
            .ok_or("Explicit fixture home required")?,
    )
    .canonicalize()
    .map_err(|_| "Invalid fixture home")?;
    let mut raw = vec![];
    io::stdin()
        .take(1_048_577)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > 1_048_576 {
        return Err("Input exceeds limit".into());
    }
    let input: Input = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    if input.cases.len() > 64 {
        return Err("Too many fixture cases".into());
    }
    let mut responses = vec![];
    for case in input.cases {
        if case.timeout_ms > 3000 {
            return Err("Fixture timeout too long".into());
        }
        let live = Arc::new(AtomicBool::new(true));
        let cancel = live.clone();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let marker = home.join("open-started");
        let _ = std::fs::remove_file(&marker);
        let timer = if case.cancel_after_open {
            Some(std::thread::spawn(move || {
                while !finished.load(Ordering::Acquire) && !marker.exists() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                if marker.exists() {
                    cancel.store(false, Ordering::Release);
                }
            }))
        } else {
            case.cancel_after_ms.map(|ms| {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(ms.min(3000)));
                    cancel.store(false, Ordering::Release);
                })
            })
        };
        let home = home.clone();
        let worker = std::thread::spawn(move || {
            let before = Instant::now();
            let mut calls = 0;
            let result = workspace::execute_fixture(
                &case.request,
                &home,
                &|| {
                    assert!(
                        !(case.panic_after_open && home.join("open-started").exists()),
                        "Fixture panic after native entry"
                    );
                    live.load(Ordering::Acquire)
                },
                before + Duration::from_millis(case.timeout_ms),
                || {
                    calls += 1;
                    assert!(
                        !case.panic_before_commit,
                        "Fixture panic before native entry"
                    );
                    if case.reject_before_commit {
                        Err("Fixture rejected before commit.".into())
                    } else {
                        Ok(())
                    }
                },
            );
            json!({"response":result.response,"committed":result.committed,"beforeCommitCalls":calls,"seconds":before.elapsed().as_secs_f64(),"pendingCallbacks":workspace::pending_callbacks()})
        });
        responses.push(worker.join().map_err(|_| "Fixture worker panicked")?);
        done.store(true, Ordering::Release);
        if let Some(t) = timer {
            t.join().map_err(|_| "Fixture timer panicked")?;
        }
    }
    let before_release = workspace::pending_callbacks();
    if input.release_callbacks {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_workspace_fixture_release".as_ptr(),
            )
        };
        if p.is_null() {
            return Err("Missing release fixture".into());
        }
        let release: unsafe extern "C" fn() = unsafe { std::mem::transmute(p) };
        unsafe {
            release();
        }
    }
    std::thread::sleep(Duration::from_millis(input.settle_ms.min(1000)));
    Ok(
        json!({"ok":true,"cases":responses,"beforeRelease":before_release,"afterRelease":workspace::pending_callbacks(),"ownApplicationActive":app.isActive(),"ownWindowCount":app.windows().count(),"fixtureOnly":true}),
    )
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
