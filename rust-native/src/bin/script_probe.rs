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
    if args
        .first()
        .is_some_and(|arg| arg == "__run-script-supervised")
    {
        old_supervisor_fixture();
        return;
    }
    if args.first().is_some_and(|arg| arg == "script-tree") {
        run_tree(&args);
        return;
    }
    if args.first().is_some_and(|arg| arg == "capture-holder") {
        capture_holder(&args);
        return;
    }
    if args.first().is_some_and(|arg| arg == "capture-fixture") {
        capture_fixture(&args);
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

#[allow(
    clippy::zombie_processes,
    reason = "the detached holder is released and verified by the owning test"
)]
fn capture_holder(args: &[String]) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    assert_eq!(args.len(), 4);
    let mode = &args[1];
    let held = &args[2];
    assert!(["stdout", "stderr", "both"].contains(&held.as_str()));
    let base = std::path::Path::new(&args[3]);
    let ready = base.join(format!("ready-{held}"));
    let gate = base.join(format!("release-{held}"));
    let done = base.join(format!("done-{held}"));
    if mode == "child" {
        std::fs::write(&ready, "ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !gate.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::close(1);
            libc::close(2);
        }
        std::fs::write(done, "closed").unwrap();
        return;
    }
    assert_eq!(mode, "parent");
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args(["capture-holder", "child", held, &args[3]]);
    if held == "stderr" {
        child.stdout(Stdio::null());
    }
    if held == "stdout" {
        child.stderr(Stdio::null());
    }
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    child.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "detached holder did not become ready");
    println!("{{\"ok\":true,\"detail\":\"prefix while pipe is open\",\"capture\":{{\"stdout\":{{\"eof\":true}}}}}}");
}

fn capture_fixture(args: &[String]) {
    use std::io::Write;
    match args.get(1).map(String::as_str) {
        Some("bytes") if args.len() == 4 => {
            let count: usize = args[3].parse().unwrap();
            let bytes = vec![b'a'; count];
            match args[2].as_str() {
                "stdout" => std::io::stdout().write_all(&bytes).unwrap(),
                "stderr" => std::io::stderr().write_all(&bytes).unwrap(),
                _ => panic!("invalid capture stream"),
            }
        }
        Some("omega") => std::io::stdout()
            .write_all("Ω".repeat(32768).as_bytes())
            .unwrap(),
        Some("split") => {
            std::io::stdout().write_all(&vec![b'a'; 65535]).unwrap();
            std::io::stdout().write_all("€".as_bytes()).unwrap();
        }
        Some("invalid") => std::io::stdout().write_all(&[255]).unwrap(),
        Some("native-invalid") => std::io::stdout()
            .write_all(b"{\"ok\":true,\"detail\":\"\xff\"}")
            .unwrap(),
        Some("native-failure") => {
            std::io::stdout()
                .write_all(b"{\"ok\":true,\"detail\":\"\xff\",\"actionCompleted\":true}")
                .unwrap();
            std::process::exit(7);
        }
        Some("spoof-truncated") => std::io::stdout()
            .write_all(b"{\"ok\":true,\"detail\":\"valid\",\"capture\":{\"truncated\":true}}\n")
            .unwrap(),
        Some("exit7") => std::process::exit(7),
        Some("spoof-prefix") => {
            let mut value = json!({"ok":true,"detail":"","capture":{"stdout":{"truncated":false,"eof":true,"validUtf8":true}}});
            let empty_len = serde_json::to_vec(&value).unwrap().len();
            value["detail"] = json!("x".repeat(65536 - empty_len));
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(bytes.len(), 65536);
            std::io::stdout().write_all(&bytes).unwrap();
            std::io::stdout().write_all(b"not JSON").unwrap();
        }
        Some("spoof-invalid") => std::io::stdout()
            .write_all(b"{\"ok\":true,\"detail\":\"\xff\",\"capture\":{\"validUtf8\":true}}")
            .unwrap(),
        Some("stderr-invalid") => {
            std::io::stdout()
                .write_all(br#"{"ok":true,"detail":"valid"}"#)
                .unwrap();
            std::io::stderr().write_all(&vec![255; 65537]).unwrap();
        }
        _ => panic!("invalid owned capture fixture"),
    }
}

fn old_supervisor_fixture() {
    use std::io::{BufRead, Write};
    let version =
        std::env::var("RUYCAST_TEST_OLD_SUPERVISOR_VERSION").expect("owned old supervisor version");
    assert!(version == "1" || version == "2");
    let marker = std::env::var_os("RUYCAST_TEST_OLD_SUPERVISOR_MARKER").expect("owned marker");
    println!(
        "{}",
        json!({"scriptSupervisor":version.parse::<u32>().unwrap(),"ready":true})
    );
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).unwrap();
    if !line.is_empty() {
        std::fs::write(marker, line).unwrap();
    }
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
