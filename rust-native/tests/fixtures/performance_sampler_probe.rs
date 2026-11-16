//! Owned Rust-only sampler fault fixture; reads/writes only beside its own executable.
use std::{fs, path::PathBuf};
fn main() {
    let root: PathBuf = std::env::current_exe().unwrap().parent().unwrap().into();
    let mode = fs::read_to_string(root.join("mode")).unwrap();
    match mode.as_str() {
        "fail" => {
            eprintln!("owned-private-counter-canary");
            std::process::exit(7);
        }
        "nonjson" => println!("invalid counter data"),
        "notobject" => println!("[]"),
        "hold" => std::thread::sleep(std::time::Duration::from_secs(5)),
        "partial" if root.join("sampled").exists() => {
            eprintln!("owned-private-counter-canary");
            std::process::exit(7);
        }
        _ => {
            fs::write(root.join("sampled"), b"owned fixture state").unwrap();
            println!("{{\"cpuNS\":100,\"footprintBytes\":1048576,\"packageIdleWakeups\":1,\"interruptWakeups\":2,\"diskReadBytes\":3,\"diskWriteBytes\":4,\"logicalWriteBytes\":5}}");
        }
    }
}
