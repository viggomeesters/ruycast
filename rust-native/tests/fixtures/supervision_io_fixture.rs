//! Deterministic script I/O for the real supervised CLI; test-tools only.
use std::io::{Read, Write};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "__run-script-supervised" => helper_protocol(),
        [mode] if mode == "success" => println!("literal Ω"),
        [mode] if mode == "exit7" => {
            println!("out");
            eprintln!("err");
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            std::process::exit(7);
        }
        [mode] if mode == "signal" => {
            assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGTERM) }, 0);
        }
        [mode, literal] if mode == "stdin-literal" => {
            let mut byte = [0];
            let read = std::io::stdin().read(&mut byte).unwrap();
            print!("{}|{literal}", if read == 0 { 1 } else { 0 });
        }
        [mode] if mode == "invalid-bytes" => {
            std::io::stdout().write_all(&[255; 100000]).unwrap();
            std::io::stderr().write_all(&[0; 100000]).unwrap();
        }
        _ => panic!("Unknown owned supervision I/O fixture"),
    }
}

// Real child endpoints for protocol refusal and pipe cleanup checks.
fn helper_protocol() {
    use std::io::BufRead;
    let mode =
        std::env::var("RUYCAST_SUPERVISION_HELPER_MODE").expect("Explicit owned helper mode");
    match mode.as_str() {
        "old-ready" => {
            println!("old runner");
            std::io::stdout().flush().unwrap();
            return;
        }
        "old-ready-wait" => {
            println!("old runner");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            return;
        }
        "old-version" => {
            println!("{}", serde_json::json!({"scriptSupervisor":1,"ready":true}));
            std::io::stdout().flush().unwrap();
            return;
        }
        "no-ready" => {
            std::thread::sleep(std::time::Duration::from_secs(1));
            return;
        }
        "partial" | "malformed" | "missing-capture" | "wrong-exit" | "oversized-final"
        | "parent-panic" | "stderr-flood" => {}
        _ => panic!("Unknown owned helper protocol mode"),
    }
    println!("{}", serde_json::json!({"scriptSupervisor":3,"ready":true}));
    std::io::stdout().flush().unwrap();
    let mut header = String::new();
    std::io::stdin().lock().read_line(&mut header).unwrap();
    match mode.as_str() {
        "parent-panic" => {
            assert!(header.ends_with('\n'));
            let home = std::path::PathBuf::from(
                std::env::var_os("RUYCAST_SUPERVISION_HELPER_HOME")
                    .expect("Explicit own helper home"),
            );
            assert!(home.is_absolute() && home.is_dir());
            assert_eq!(home.canonicalize().unwrap(), home);
            assert_eq!(
                std::fs::read(home.join(".ruycast-supervision-fixture")).unwrap(),
                b"owned-supervision-helper-v1"
            );
            std::fs::write(home.join("helper-committed"), "committed").unwrap();
            let mut rest = vec![];
            std::io::stdin().read_to_end(&mut rest).unwrap();
            if let Err(error) = std::io::stdout().write_all(&vec![b'x'; 1000000]) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
            std::fs::write(home.join("helper-cleaned"), "closed").unwrap();
            // Like os._exit, do not retry a buffered stdout flush after parent closure.
            std::process::exit(0);
        }
        "stderr-flood" => {
            std::io::stderr().write_all(&vec![b'x'; 1000000]).unwrap();
            println!(
                "{}",
                serde_json::json!({"ok":true,"scriptSupervisor":3,"response":{"ok":true,"result":{"exitCode":0,"stdout":"","stderr":""}},"capture":{"stdout":{"truncated":false,"eof":true,"validUtf8":true},"stderr":{"truncated":false,"eof":true,"validUtf8":true}}})
            );
        }
        "partial" => print!("partial"),
        "malformed" => println!("not json"),
        "missing-capture" => println!(
            "{}",
            serde_json::json!({"ok":true,"scriptSupervisor":3,"response":{"ok":true}})
        ),
        "wrong-exit" => {
            println!(
                "{}",
                serde_json::json!({"ok":true,"scriptSupervisor":3,"response":{"ok":false,"error":"failure"}})
            );
            std::io::stdout().flush().unwrap();
            std::process::exit(7);
        }
        "oversized-final" => {
            if std::io::stdout()
                .write_all(&vec![b'x'; 3 * 1024 * 1024])
                .is_err()
            {
                std::process::exit(1);
            }
        }
        _ => unreachable!(),
    }
    std::io::stdout().flush().unwrap();
}
