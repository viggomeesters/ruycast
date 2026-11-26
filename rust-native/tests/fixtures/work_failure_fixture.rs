//! Isolated scripted provider; never accesses an operational vault.
use serde_json::json;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--ruycast-writer-capability-v2") {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        if home.join("legacy-writer").exists() {
            println!("{}", json!({"ok":false,"error":"Gebruik: viggo-work-writer capture ROOT note|task REQUEST-ID TEKST"}));
            std::process::exit(1);
        }
        println!("{}", json!({"ok":true,"workMutationAdmission":2,"routes":["capture"]}));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--ruycast-writer-grant-v2") {
        use std::{io::{Read, Write}, os::fd::FromRawFd, os::unix::net::UnixStream, time::{Duration, Instant}};
        let fd: i32 = args[2].parse().unwrap();
        let mut channel = unsafe { UnixStream::from_raw_fd(fd) };
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(home.join("owned-work-writer-fixture").is_file());
        assert_eq!(args[3], "capture");
        assert!(matches!(args[5].as_str(), "note" | "task"));
        let mode = std::fs::read_to_string(home.join("fixture-mode")).unwrap();
        if mode == "pre-error" { println!("{}", json!({"ok":false,"error":"Writer-toestemming vereist de gecontroleerde Work-root"})); std::process::exit(1); }
        if mode == "pre-private" { println!("{}", json!({"ok":false,"error":"SECRET_PAYLOAD","detail":"SECRET_PAYLOAD","actionCompleted":true})); std::process::exit(1); }
        let wait = |name: &str| { let deadline = Instant::now() + Duration::from_secs(5); while !home.join(name).exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5)); } };
        if mode == "before" { std::fs::write(home.join("before-grant"), b"ready").unwrap(); wait("release-before"); }
        if mode == "decline" { channel.write_all(b"D").unwrap(); drop(channel); println!("{}", json!({"ok":true})); return; }
        if mode == "silent" { drop(channel); println!("{}", json!({"ok":true})); return; }
        channel.write_all(b"W").unwrap();
        let mut response = [0]; channel.read_exact(&mut response).unwrap(); assert_eq!(response, [b'A']);
        let count = std::fs::read_to_string(home.join("capture-count")).ok().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0) + 1;
        std::fs::write(home.join("capture-count"), count.to_string()).unwrap();
        std::fs::write(home.join("writer-pid"), std::process::id().to_string()).unwrap();
        std::fs::write(home.join("postimage"), b"written").unwrap();
        wait("release-after");
        if mode == "unknown" { println!("not a writer result"); return; }
        std::fs::write(home.join("receipt"), b"committed").unwrap();
        if mode == "incomplete" { println!("{}", json!({"ok":true})); return; }
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(args[6].as_bytes()));
        let folder = format!("fixture-{}", &digest[..20]);
        let path = std::path::Path::new(&args[4]).join("00-inbox").join(&folder).join(format!("{folder}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fictional note").unwrap();
        println!("{}", json!({"ok":true,"path":path,"note_path":path,"kind":args[5],"reused":false}));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--ruycast-writer-capability-v1") {
        if std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("legacy-writer").exists() {
            println!("{}", json!({"ok":false,"error":"Gebruik: viggo-work-writer capture ROOT note|task REQUEST-ID TEKST"}));
            std::process::exit(1);
        }
        println!("{}", json!({"ok":true,"workStatusAdmission":1}));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "task-active") && args.get(4).is_some_and(|arg| arg == "legacy") {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        std::fs::write(home.join("legacy-started"), b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--ruycast-writer-grant-v1") {
        use std::{io::{Read, Write}, os::fd::FromRawFd, os::unix::net::UnixStream, time::{Duration, Instant}};
        let fd: i32 = args[2].parse().unwrap();
        let mut channel = unsafe { UnixStream::from_raw_fd(fd) };
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(home.join("owned-work-writer-fixture").is_file());
        assert_eq!(args[3], "task-active");
        let mode = &args[6];
        if mode == "silent" { drop(channel); println!("{}", json!({"ok":true})); return; }
        let wait = |name: &str| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !home.join(name).exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        if mode == "before" {
            std::fs::write(home.join("before-grant"), b"ready").unwrap();
            wait("release-before");
        }
        if mode == "private" || mode == "private-success" {
            channel.write_all(b"D").unwrap();
            drop(channel);
            if mode == "private-success" { println!("{}", json!({"ok":true})); return; }
            std::fs::write(home.join("private-started"), b"ready").unwrap();
            wait("release-private");
            return;
        }
        channel.write_all(b"W").unwrap();
        let mut response = [0];
        channel.read_exact(&mut response).unwrap();
        assert_eq!(response, [b'A']);
        if mode == "malformed" { println!("not a writer result"); return; }
        std::fs::write(home.join("writer-pid"), std::process::id().to_string()).unwrap();
        std::fs::write(home.join("postimage"), b"written").unwrap();
        wait("release-after");
        std::fs::write(home.join("receipt"), b"committed").unwrap();
        println!("{}", json!({"ok":true}));
        return;
    }
    let id = args.get(3).cloned().unwrap_or_default();
    let token = std::env::var("VIGGOBSIDIAN_DIAGNOSTIC_ID").unwrap_or_default();
    let action = args.get(2).map(String::as_str).unwrap_or("");
    let body = json!({"ok":false,"error":"Synthetic error at 00-inbox/demo/note.md","detail":"Synthetic error at 00-inbox/demo/note.md","failureContext":{"schema":"ruycast.work-failure-context.v1","invocationID":token,"scope":"work","relativePath":"00-inbox/demo/note.md"}});
    assert_eq!(id, body["failureContext"]["invocationID"]);
    println!("{body}");
    if action != "convert-note" {
        std::process::exit(1);
    }
}
