//! Isolated test executable for the actual shared runner; no native surfaces.
use rustcast_core::script::{self, End};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{self, Read},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    command: rustcast_core::Command,
    home: PathBuf,
    cancel_ms: Option<u64>,
    deadline_ms: Option<u64>,
    panic_ms: Option<u64>,
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "script-tree") {
        run_tree(&args);
        return;
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let input: Input = serde_json::from_str(&input).unwrap();
    let started = Instant::now();
    let result = std::panic::catch_unwind(|| {
        script::run(
            &input.command,
            &input.home,
            input
                .deadline_ms
                .map(|ms| started + Duration::from_millis(ms)),
            || {
                assert!(
                    input
                        .panic_ms
                        .is_none_or(|ms| started.elapsed() < Duration::from_millis(ms)),
                    "fixture cancellation callback panic"
                );
                input
                    .cancel_ms
                    .is_some_and(|ms| started.elapsed() >= Duration::from_millis(ms))
            },
        )
    });
    let result = match result {
        Err(_) => json!({"panicCaught":true}),
        Ok(Err(error)) => json!({"ok":false,"error":error}),
        Ok(Ok(completion)) => json!({
            "response":completion.response(),
            "capture":completion.capture,
            "end":match completion.end { End::Exited(_) => "exited", End::Cancelled => "cancelled", End::TimedOut => "timeout", End::Deadline => "deadline" },
            "stdoutBytes":completion.stdout.len(),
            "stderrBytes":completion.stderr.len(),
        }),
    };
    // No runner-owned direct child may be left running or zombie.
    let mut status = 0;
    let child = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    let child_error = io::Error::last_os_error().raw_os_error();
    let clean = child == -1 && child_error == Some(libc::ECHILD);
    println!(
        "{}",
        json!({"result":result,"noChildren":clean,"childProbe":child,"childError":child_error,"elapsedMs":started.elapsed().as_millis()})
    );
    assert!(clean, "runner left an owned child");
}

// This mode deliberately creates an unreaped descendant for the supervisor under test.
#[allow(
    clippy::zombie_processes,
    reason = "the fixture tests supervisor cleanup of an unreaped descendant"
)]
fn run_tree(args: &[String]) {
    assert_eq!(args.len(), 4);
    let mode = args[1].as_str();
    if mode == "child-resistant" || mode == "child-ordinary" {
        if mode == "child-resistant" {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
        }
        std::thread::sleep(Duration::from_millis(800));
        std::fs::write(&args[3], "bad").unwrap();
        return;
    }
    if mode == "leader" {
        std::fs::write(&args[2], std::process::id().to_string()).unwrap();
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
    }
    assert!(mode == "leader" || mode == "early");
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "script-tree",
            if mode == "leader" {
                "child-resistant"
            } else {
                "child-ordinary"
            },
            &args[2],
            &args[3],
        ])
        .spawn()
        .unwrap();
    if mode == "early" {
        println!("done");
    } else {
        std::thread::sleep(Duration::from_secs(10));
    }
}
