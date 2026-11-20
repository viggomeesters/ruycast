//! Disposable process bodies for the owned shortcut capture contract.
use std::{io::{self, Write}, process::Command, thread, time::Duration};

fn main() {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|value| value.to_str()) {
        Some("success") => io::stdout().write_all(b"owned").unwrap(),
        Some("nonzero") => std::process::exit(1),
        Some("sleep") => thread::sleep(Duration::from_secs(5)),
        Some("stdout-flood") => flood(io::stdout().lock()),
        Some("stderr-flood") => flood(io::stderr().lock()),
        Some("inherited-pipes") => {
            // spawn inherits both output handles; dropping Child does not wait.
            let _child = Command::new(std::env::current_exe().unwrap())
                .arg("hold-pipes").spawn().unwrap();
        }
        Some("hold-pipes") => thread::sleep(Duration::from_secs(1)),
        Some("cancel") => {
            std::fs::write(args.next().expect("entered marker path"), b"entered").unwrap();
            thread::sleep(Duration::from_secs(5));
        }
        _ => panic!("unknown owned shortcut fixture mode"),
    }
}

fn flood(mut output: impl Write) {
    loop {
        if output.write_all(b"xxxxxxxxxx").is_err() {
            return;
        }
    }
}
