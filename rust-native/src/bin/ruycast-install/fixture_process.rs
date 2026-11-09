//! Compiled only by the owned process test, never part of a shipped tool.
use std::io::{Read, Write};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args
        .get(1)
        .is_some_and(|a| a == "install" || a == "signing")
    {
        let root = std::path::PathBuf::from(
            std::env::var_os("RUYCAST_DISPATCH_FIXTURE_ROOT")
                .expect("Owned dispatch fixture required"),
        );
        assert!(root.join("owned-process-marker").is_file());
        if let Ok(descriptor) = std::env::var("RUYCAST_ARTIFACT_LEASE_FD") {
            use std::os::{fd::FromRawFd, unix::fs::MetadataExt};
            let descriptor = descriptor.parse::<i32>().unwrap();
            assert!(descriptor >= 3);
            let inherited = unsafe { std::fs::File::from_raw_fd(descriptor) };
            let metadata = inherited.metadata().unwrap();
            let repo =
                std::path::PathBuf::from(std::env::var_os("RUYCAST_ARTIFACT_LEASE_ROOT").unwrap());
            let named = std::fs::symlink_metadata(repo.join(".build/artifacts.lock")).unwrap();
            assert!(metadata.is_file() && !named.file_type().is_symlink());
            assert_eq!(
                (metadata.dev(), metadata.ino(), metadata.uid()),
                (named.dev(), named.ino(), named.uid())
            );
            assert_eq!(
                metadata.dev().to_string(),
                std::env::var("RUYCAST_ARTIFACT_LEASE_DEV").unwrap()
            );
            assert_eq!(
                metadata.ino().to_string(),
                std::env::var("RUYCAST_ARTIFACT_LEASE_INO").unwrap()
            );
            assert_eq!(
                metadata.uid().to_string(),
                std::env::var("RUYCAST_ARTIFACT_LEASE_UID").unwrap()
            );
            assert_eq!(metadata.mode() & 0o777, 0o600);
            let at = args.iter().position(|a| a == "--repo").unwrap();
            assert_eq!(std::path::Path::new(&args[at + 1]), repo);
            // Close only this child's inherited descriptor, never LOCK_UN.
        }
        std::io::stdout()
            .write_all(args[1..].join("\0").as_bytes())
            .unwrap();
        return;
    }
    let marker = std::path::Path::new(&args[2]);
    assert!(marker
        .parent()
        .unwrap()
        .join("owned-process-marker")
        .is_file());
    std::fs::write(marker, std::process::id().to_string()).unwrap();
    match args[1].as_str() {
        "hang" => std::thread::sleep(std::time::Duration::from_secs(20)),
        "flood" => {
            let block = [b'x'; 8192];
            for _ in 0..512 {
                if std::io::stdout().write_all(&block).is_err() {
                    break;
                }
            }
        }
        "stdin" => {
            let mut bytes = vec![];
            std::io::stdin().read_to_end(&mut bytes).unwrap();
            std::io::stdout().write_all(&bytes).unwrap();
        }
        _ => std::process::exit(2),
    }
}
