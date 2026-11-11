//! Exercise the loaded fsync interpose, never a copied fault implementation.
use std::{
    fs,
    io::Write,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::PathBuf,
};

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove owned probe");
    }
}

fn main() {
    let parent = PathBuf::from(std::env::args_os().nth(1).expect("explicit owned parent"))
        .canonicalize()
        .expect("existing parent");
    let home = parent.join(format!("fsync-probe-{}", std::process::id()));
    fs::create_dir(&home).expect("fresh owned probe");
    let _cleanup = Cleanup(home.clone());
    for mode in ["plain", "edit", "replace"] {
        let directory = home.join(mode);
        fs::create_dir(&directory).unwrap();
        let target = directory.join("config.json");
        let marker = home.join(format!("{mode}.marker"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .unwrap();
        file.write_all(b"original fixture").unwrap();
        std::env::set_var("RUYCAST_TEST_FSYNC_MARKER", &marker);
        std::env::remove_var("RUYCAST_TEST_FSYNC_MUTATE");

        // An ordinary file must still reach the original OS fsync.
        std::env::set_var("RUYCAST_TEST_FSYNC_DIRECTORY", &directory);
        assert_eq!(unsafe { libc::fsync(file.as_raw_fd()) }, 0);
        assert!(!marker.exists());
        assert_eq!(fs::read(&target).unwrap(), b"original fixture");
        drop(file);

        let dir = fs::File::open(&directory).unwrap();
        std::env::set_var("RUYCAST_TEST_FSYNC_DIRECTORY", home.join("other"));
        assert_eq!(unsafe { libc::fsync(dir.as_raw_fd()) }, 0);
        assert!(!marker.exists());

        std::env::set_var("RUYCAST_TEST_FSYNC_DIRECTORY", &directory);
        if mode != "plain" {
            std::env::set_var("RUYCAST_TEST_FSYNC_MUTATE", mode);
        }
        assert_eq!(
            unsafe { libc::fsync(dir.as_raw_fd()) },
            -1,
            "interpose must run"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EIO)
        );
        assert_eq!(fs::read(&marker).unwrap(), b"injected");
        if mode == "plain" {
            assert_eq!(fs::read(&target).unwrap(), b"original fixture");
        } else {
            assert_eq!(fs::read(&target).unwrap(), b"external fixture edit");
        }
        if mode == "replace" {
            assert_eq!(
                fs::read(directory.join("published.copy")).unwrap(),
                b"original fixture"
            );
        } else {
            assert!(!directory.join("published.copy").exists());
        }
    }
    std::env::remove_var("RUYCAST_TEST_FSYNC_DIRECTORY");
    std::env::remove_var("RUYCAST_TEST_FSYNC_MARKER");
    std::env::remove_var("RUYCAST_TEST_FSYNC_MUTATE");
    assert_eq!(unsafe { libc::fsync(-1) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EBADF)
    );
    println!(
        "PASS: matched directory/EIO/marker/edit/replace; file, directory and invalid-fd fallback"
    );
}
