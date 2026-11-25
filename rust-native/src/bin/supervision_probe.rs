//! Own-process protocol/fault harness, excluded from production builds.
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
    helper: PathBuf,
    command: rustcast_core::Command,
    home: PathBuf,
    cancel_ms: Option<u64>,
    panic_ms: Option<u64>,
    deadline_ms: Option<u64>,
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "__supervision-heartbeat")
    {
        assert_eq!(args.len(), 3);
        let index: usize = args[2].to_str().unwrap().parse().unwrap();
        assert!(index < 15);
        heartbeat(tree_home(PathBuf::from(&args[1])), index);
        return;
    }
    if args
        .first()
        .is_some_and(|arg| arg == "__supervision-tree" || arg == "__supervision-tree-descendant")
    {
        assert_eq!(args.len(), 2);
        let home = tree_home(PathBuf::from(&args[1]));
        if args[0] == "__supervision-tree" {
            tree_leader(home);
        } else {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
            std::thread::sleep(Duration::from_millis(800));
            std::fs::write(home.join("descendant"), "bad").unwrap();
        }
        return;
    }
    let mut data = String::new();
    io::stdin()
        .take(8 * 1024 * 1024 + 8192)
        .read_to_string(&mut data)
        .unwrap();
    let input: Input = serde_json::from_str(&data).unwrap();
    let start = Instant::now();
    let result = std::panic::catch_unwind(|| {
        ruycast_native::supervision::probe(
            &input.helper,
            &input.command,
            &input.home,
            input
                .deadline_ms
                .map(|ms| start + Duration::from_millis(ms)),
            || {
                assert!(
                    input
                        .panic_ms
                        .is_none_or(|ms| start.elapsed() < Duration::from_millis(ms)),
                    "own supervisor callback panic"
                );
                input
                    .cancel_ms
                    .is_some_and(|ms| start.elapsed() >= Duration::from_millis(ms))
            },
        )
    });
    let value = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => json!({"error":error}),
        Err(_) => json!({"panicCaught":true}),
    };
    let mut status = 0;
    let child = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    let clean = child == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD);
    println!(
        "{}",
        json!({"result":value,"noChildren":clean,"elapsedMs":start.elapsed().as_millis(),"performance":ruycast_native::performance_trace::shared().snapshot()})
    );
    assert!(clean, "supervisor left an owned helper");
}

fn tree_home(home: PathBuf) -> PathBuf {
    assert!(home.is_absolute() && home.is_dir());
    assert_eq!(home.canonicalize().unwrap(), home);
    assert_eq!(
        std::fs::read(home.join(".ruycast-supervision-fixture")).unwrap(),
        b"owned-supervision-helper-v1"
    );
    home
}
// The actual supervisor owns reaping this deliberately unreaped descendant.
#[allow(
    clippy::zombie_processes,
    reason = "Original fixture leaves its descendant to the actual supervisor group cleanup"
)]
fn tree_leader(home: PathBuf) {
    std::fs::write(home.join("script-pid"), std::process::id().to_string()).unwrap();
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    let _child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            std::ffi::OsStr::new("__supervision-tree-descendant"),
            home.as_os_str(),
        ])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(3));
}

fn heartbeat(home: PathBuf, index: usize) {
    let path = home.join(index.to_string());
    let temporary = path.with_extension("tmp");
    let deadline = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < deadline {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
            0
        );
        let value = json!({"pid":std::process::id(),"helper":unsafe {libc::getppid()},"time":time.tv_sec as f64 + time.tv_nsec as f64 / 1_000_000_000.0});
        std::fs::write(&temporary, serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::rename(&temporary, &path).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
}
