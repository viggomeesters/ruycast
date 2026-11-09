//! Test-only bounded manager + synthetic four-selector SDK boundary.
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use ruycast_native::{
    execution::{self, Manager, Outcome, WorkspaceMode},
    workspace::{self, Kind, OpenRequest},
};
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

fn wait_for(mut condition: impl FnMut() -> bool) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(3);
    while !condition() {
        if Instant::now() >= end {
            return Err("Own fixture barrier timed out.".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
fn request(mode: &str, url: bool) -> OpenRequest {
    OpenRequest {
        kind: if url { Kind::Url } else { Kind::App },
        command_id: "fixture".into(),
        title: "Fixture".into(),
        target: if url {
            format!("ruycast-fixture://{mode}/path")
        } else {
            format!("dev.fixture.{mode}")
        },
        application_id: None,
    }
}
fn wrap(items: Vec<execution::Finished<usize>>) -> Value {
    json!(items.into_iter().map(|item| {
        let value = match item.result {
            Ok(Outcome::Workspace(out)) => json!({"kind":"workspace","response":out.response,"committed":out.committed,"captureAvailable":false}),
            Ok(Outcome::Script(_)) => json!({"kind":"unexpected-script"}),
            Ok(Outcome::Snippet(_)) => json!({"kind":"unexpected-snippet"}),
            Err(error) => json!({"kind":"driver-error","error":error}),
        };
        json!({"id":item.id,"context":item.context,"outcome":value})
    }).collect::<Vec<_>>())
}
#[allow(clippy::too_many_arguments)]
fn admit(
    manager: &mut Manager<usize>,
    home: &Path,
    mode: &str,
    url: bool,
    cli: bool,
    id: usize,
    hook: bool,
    calls: Arc<AtomicUsize>,
) -> Result<u64, (String, usize)> {
    let path = home.to_path_buf();
    manager.admit_open(
        request(mode, url),
        WorkspaceMode::Fixture(home.to_path_buf()),
        cli,
        Arc::new(|| true),
        Instant::now() + Duration::from_secs(2),
        Box::new(move || {
            calls.fetch_add(1, Ordering::AcqRel);
            if hook {
                std::fs::write(path.join("hook-entered"), b"entered").map_err(|e| e.to_string())?;
                wait_for(|| path.join("workspace-release").exists())?;
            }
            Ok(())
        }),
        id,
    )
}
fn run() -> Result<Value, String> {
    let home = PathBuf::from(
        std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME")
            .ok_or("Explicit fixture home required")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if std::fs::read(home.join(".ruycast-test-fixture"))
        .ok()
        .as_deref()
        != Some(b"own-disposable-fixture-v1")
    {
        return Err("Invalid fixture marker.".into());
    }
    let ready = unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            c"ruycast_workspace_fixture_ready".as_ptr(),
        )
    };
    if ready.is_null() {
        return Err("Missing workspace fixture boundary.".into());
    }
    let ready: unsafe extern "C" fn() -> u64 = unsafe { std::mem::transmute(ready) };
    if unsafe { ready() } != 0x52555943 {
        return Err("Incomplete workspace fixture boundary.".into());
    }
    let app =
        NSApplication::sharedApplication(MainThreadMarker::new().ok_or("Main thread required")?);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let mut bytes = vec![];
    io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Fixture input too large.".into());
    }
    let input: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let scenario = input["scenario"].as_str().ok_or("Missing scenario")?;
    let (sender, _rx) = mpsc::sync_channel(1);
    let mut manager = Manager::new(home.clone(), sender.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let data = match scenario {
        "ready" => {
            admit(
                &mut manager,
                &home,
                "inline",
                false,
                true,
                7,
                false,
                calls.clone(),
            )
            .map_err(|e| e.0)?;
            wait_for(|| manager.ready_count() == 1)?;
            let result = wrap(manager.shutdown());
            wait_for(|| manager.live_workers() == 0)?;
            json!({"deliveries":result,"secondDelivery":wrap(manager.harvest()),"liveWorkers":manager.live_workers()})
        }
        "retire-lookup" | "retire-sync" | "retire-hook" => {
            let (mode, url, barrier, hook) = match scenario {
                "retire-lookup" => ("hold-resolve", false, "resolve-entered", false),
                "retire-sync" => ("hold-sync", true, "open-started", false),
                _ => ("success", false, "hook-entered", true),
            };
            admit(
                &mut manager,
                &home,
                mode,
                url,
                true,
                11,
                hook,
                calls.clone(),
            )
            .map_err(|e| e.0)?;
            wait_for(|| home.join(barrier).exists())?;
            let start = Instant::now();
            let deliveries = wrap(manager.shutdown());
            let seconds = start.elapsed().as_secs_f64();
            let before_release = manager.live_workers();
            let old_slot_redelivery = wrap(manager.harvest());
            drop(manager);
            let replacement: Manager<usize> = Manager::new(home.clone(), sender.clone());
            let replacement_before = replacement.live_workers();
            std::fs::write(home.join("workspace-release"), b"release")
                .map_err(|e| e.to_string())?;
            wait_for(|| replacement.live_workers() == 0)?;
            json!({"deliveries":deliveries,"seconds":seconds,"beforeRelease":before_release,"replacementBeforeRelease":replacement_before,"afterRelease":replacement.live_workers(),"secondDelivery":old_slot_redelivery,"releasedOnlyAfterShutdown":true})
        }
        "capacity" => {
            admit(
                &mut manager,
                &home,
                "hold-resolve",
                false,
                true,
                1,
                false,
                calls.clone(),
            )
            .map_err(|e| e.0)?;
            wait_for(|| home.join("resolve-entered").exists())?;
            let old_deliveries = wrap(manager.shutdown());
            drop(manager);
            let mut next = Manager::new(home.clone(), sender.clone());
            for i in 1..execution::CLI_LIMIT {
                admit(
                    &mut next,
                    &home,
                    "hold-resolve",
                    false,
                    true,
                    i + 1,
                    false,
                    calls.clone(),
                )
                .map_err(|e| e.0)?;
            }
            let cli_rejected = admit(
                &mut next,
                &home,
                "hold-resolve",
                false,
                true,
                100,
                false,
                calls.clone(),
            )
            .is_err();
            let native_reserved = admit(
                &mut next,
                &home,
                "hold-resolve",
                false,
                false,
                101,
                false,
                calls.clone(),
            )
            .is_ok();
            let native_rejected = admit(
                &mut next,
                &home,
                "hold-resolve",
                false,
                false,
                102,
                false,
                calls.clone(),
            )
            .is_err();
            let before_release = next.live_workers();
            let deliveries = wrap(next.shutdown());
            let after_shutdown = next.live_workers();
            std::fs::write(home.join("workspace-release"), b"release")
                .map_err(|e| e.to_string())?;
            wait_for(|| next.live_workers() == 0)?;
            json!({"oldDeliveries":old_deliveries,"deliveries":deliveries,"cliRejected":cli_rejected,"nativeReserved":native_reserved,"nativeRejected":native_rejected,"beforeRelease":before_release,"afterShutdown":after_shutdown,"afterRelease":next.live_workers(),"limit":execution::LIMIT,"cliLimit":execution::CLI_LIMIT})
        }
        _ => return Err("Unknown manager fixture scenario.".into()),
    };
    Ok(
        json!({"ok":true,"scenario":scenario,"result":data,"beforeCommitCalls":calls.load(Ordering::Acquire),"pendingCallbacks":workspace::pending_callbacks(),"ownApplicationActive":app.isActive(),"ownWindowCount":app.windows().count(),"fixtureOnly":true}),
    )
}
fn main() {
    match run() {
        Ok(v) => println!("{v}"),
        Err(e) => {
            println!("{}", json!({"ok":false,"error":e}));
            std::process::exit(1);
        }
    }
}
