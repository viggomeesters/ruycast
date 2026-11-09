use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--warm" {
        return;
    }
    let root = PathBuf::from(&args[2]);
    assert_eq!(
        fs::read(root.join("owned-process-marker")).unwrap(),
        b"review-owned-process-v1"
    );
    match args[1].as_str() {
        "parent" | "parent-null-pipes" => {
            let detached_pipes = args[1] == "parent-null-pipes";
            let _child = Command::new(std::env::current_exe().unwrap())
                .args(["child", &args[2]])
                .stdin(Stdio::null())
                .stdout(if detached_pipes {
                    Stdio::null()
                } else {
                    Stdio::inherit()
                })
                .stderr(if detached_pipes {
                    Stdio::null()
                } else {
                    Stdio::inherit()
                })
                .spawn()
                .unwrap();
            if detached_pipes {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while !root.join("started").is_file() {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        "child" => {
            fs::write(root.join("started"), std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_millis(700));
            fs::write(
                root.join("completed-after-parent"),
                b"late owned side effect",
            )
            .unwrap();
        }
        _ => panic!("unknown owned mode"),
    }
}
