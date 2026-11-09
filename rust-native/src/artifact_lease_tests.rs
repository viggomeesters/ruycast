use super::*;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::{
    os::unix::fs::{symlink, DirBuilderExt},
    process::{Child, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("explicit owned fixture parent required"),
        );
        assert!(parent.is_absolute());
        let path = parent.join(format!(
            "lease-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait(path: &Path) {
    let until = Instant::now() + Duration::from_secs(8);
    while !path.exists() {
        assert!(
            Instant::now() < until,
            "owned fixture timeout: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn child(root: &Path, mode: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args([
        "--ignored",
        "--exact",
        "artifact_lease::tests::owned_process_entry",
        "--test-threads=1",
    ])
    .env_clear()
    .env("RUYCAST_LEASE_OWNED_ROOT", root)
    .env("RUYCAST_LEASE_OWNED_MODE", mode)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    c
}
#[test]
#[ignore = "Internal owned subprocess entry; actual parent tests invoke this exact fixture explicitly"]
fn owned_process_entry() {
    let root =
        PathBuf::from(std::env::var_os("RUYCAST_LEASE_OWNED_ROOT").expect("owned child root"));
    let mode = std::env::var("RUYCAST_LEASE_OWNED_MODE").expect("owned child mode");
    match mode.as_str() {
        "controller" => {
            let lease = Lease::acquire(&root).unwrap();
            let mut c = child(&root, "consumer");
            lease.apply_to_child(&mut c).unwrap();
            let consumer = Reap(c.spawn().unwrap());
            fs::write(root.join("consumer-pid"), consumer.0.id().to_string()).unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        "consumer" => {
            let lease = Lease::acquire(&root).unwrap();
            fs::write(root.join("consumer-entered"), b"held").unwrap();
            wait(&root.join("release-consumer"));
            lease.validate().unwrap();
            fs::write(
                root.join("final-consumer-finished"),
                b"finished while leased",
            )
            .unwrap();
        }
        "contender" => {
            fs::write(root.join("contender-requested"), b"requested").unwrap();
            let _lease = Lease::acquire(&root).unwrap();
            assert!(
                root.join("final-consumer-finished").exists()
                    || root.join("nested-parent-released").exists()
            );
            fs::write(root.join("contender-acquired"), b"acquired").unwrap();
        }
        "nested" => {
            let lease = Lease::acquire(&root).unwrap();
            lease.validate().unwrap();
            fs::write(root.join("nested-entered"), b"valid same capability").unwrap();
        }
        "reject" => {
            assert!(Lease::acquire(&root).is_err());
        }
        _ => panic!("unknown owned mode"),
    }
}
#[test]
fn owned_crash_retains_child_lease_through_the_final_consumer() {
    controller_signal_retains_consumer_lease(libc::SIGKILL);
}
#[test]
fn owned_interrupt_retains_child_lease_through_the_final_consumer() {
    controller_signal_retains_consumer_lease(libc::SIGINT);
}
fn controller_signal_retains_consumer_lease(signal: i32) {
    let f = Fixture::new();
    let mut controller = Reap(child(&f.0, "controller").spawn().unwrap());
    wait(&f.0.join("consumer-entered"));
    wait(&f.0.join("consumer-pid"));
    let pid = fs::read_to_string(f.0.join("consumer-pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(controller.0.id() as i32, signal) }, 0);
    assert_eq!(controller.0.wait().unwrap().signal(), Some(signal));
    let mut contender = Reap(child(&f.0, "contender").spawn().unwrap());
    wait(&f.0.join("contender-requested"));
    std::thread::sleep(Duration::from_millis(80));
    assert!(!f.0.join("contender-acquired").exists());
    fs::write(f.0.join("release-consumer"), b"release").unwrap();
    wait(&f.0.join("contender-acquired"));
    assert!(contender.0.wait().unwrap().success());
    let until = Instant::now() + Duration::from_secs(8);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < until, "owned orphan did not retire");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn owned_nested_consumers_do_not_unlock_parent_and_bad_capabilities_fail_closed() {
    let f = Fixture::new();
    let lease = Lease::acquire_from_environment(&f.0, &BTreeMap::new()).unwrap();
    let mut c = child(&f.0, "nested");
    lease.apply_to_child(&mut c).unwrap();
    assert!(c.status().unwrap().success());
    assert!(f.0.join("nested-entered").exists());
    for fault in 0..3 {
        let mut c = child(&f.0, "reject");
        lease.apply_to_child(&mut c).unwrap();
        match fault {
            0 => {
                c.env_remove(key("INO"));
            }
            1 => {
                c.env(
                    key("UID"),
                    (unsafe { libc::getuid() } as u64 + 1).to_string(),
                );
            }
            _ => {
                c.env(key("FD"), "999999");
            }
        }
        assert!(c.status().unwrap().success());
    }
    let foreign = Fixture::new();
    let mut c = child(&foreign.0, "reject");
    lease.apply_to_child(&mut c).unwrap();
    assert!(c.status().unwrap().success());
    assert!(!foreign.0.join(".build").exists());
    let mut contender_command = child(&f.0, "contender");
    let inherited = lease.file.as_raw_fd();
    // An independent contender must not retain the parent's capability.
    // close is async-signal-safe and affects only this forked child's FD.
    unsafe {
        contender_command.pre_exec(move || {
            libc::close(inherited);
            Ok(())
        });
    }
    let mut contender = Reap(contender_command.spawn().unwrap());
    wait(&f.0.join("contender-requested"));
    std::thread::sleep(Duration::from_millis(80));
    assert!(!f.0.join("contender-acquired").exists());
    fs::write(f.0.join("nested-parent-released"), b"release").unwrap();
    drop(lease);
    assert!(contender.0.wait().unwrap().success());
}
#[test]
fn owned_lock_types_modes_and_identity_are_checked_without_replacing_existing_data() {
    let f = Fixture::new();
    fs::create_dir(f.0.join(".build")).unwrap();
    let path = lock_path(&f.0);
    fs::write(&path, b"legacy owner file").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let lease = Lease::acquire_from_environment(&f.0, &BTreeMap::new()).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(lease);
    assert_eq!(fs::read(&path).unwrap(), b"legacy owner file");
    fs::rename(&path, f.0.join("preserved")).unwrap();
    symlink(f.0.join("preserved"), &path).unwrap();
    assert!(Lease::acquire_from_environment(&f.0, &BTreeMap::new()).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(f.0.join("preserved"), &path).unwrap();
    assert!(Lease::acquire_from_environment(&f.0, &BTreeMap::new()).is_err());
    fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(Lease::acquire_from_environment(&f.0, &BTreeMap::new()).is_err());
    assert_eq!(
        fs::read(f.0.join("preserved")).unwrap(),
        b"legacy owner file"
    );
}
#[test]
fn owned_replaced_lock_namespace_revokes_child_transfer() {
    let f = Fixture::new();
    let lease = Lease::acquire_from_environment(&f.0, &BTreeMap::new()).unwrap();
    fs::rename(lock_path(&f.0), f.0.join("original-lock")).unwrap();
    fs::write(lock_path(&f.0), b"replacement").unwrap();
    assert!(lease.validate().is_err());
    assert!(lease.environment().is_err());
    assert_eq!(fs::read(lock_path(&f.0)).unwrap(), b"replacement");
}
