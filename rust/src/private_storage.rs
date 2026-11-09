//! macOS CLI storage: secure an empty stage before creating any payload.
use std::{
    ffi::CString,
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::DirBuilderExt},
    },
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, String>;
fn error() -> String {
    io::Error::last_os_error().to_string()
}
fn name(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| "Invalid storage path".into())
}
static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn stage_name() -> Result<CString> {
    let sequence = STAGE_SEQUENCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| "Private staging sequence exhausted".to_string())?;
    name(format!(".ruycast-{}-{sequence}.tmp", std::process::id()).as_bytes())
}

fn create_stage(parent: &File, mut next: impl FnMut() -> Result<CString>) -> Result<CString> {
    // Existing names may belong to another writer or a prior process with a
    // reused PID. Never open, remove, or adopt them.
    for _ in 0..128 {
        let temporary = next()?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), temporary.as_ptr(), 0o700) } == 0 {
            return Ok(temporary);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(error.to_string());
        }
    }
    Err("Private staging directory names unavailable".into())
}
fn descriptor(fd: libc::c_int) -> Result<File> {
    if fd < 0 {
        Err(error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn metadata(parent: &File, path: &CString) -> Result<libc::stat> {
    let mut info = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            path.as_ptr(),
            &mut info,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(error());
    }
    Ok(info)
}
#[cfg(target_os = "macos")]
mod acl {
    use super::*;
    use libc::{c_int, c_void};
    extern "C" {
        fn acl_init(count: c_int) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> c_int;
        fn acl_set_fd_np(fd: c_int, acl: *mut c_void, kind: c_int) -> c_int;
        fn acl_get_fd_np(fd: c_int, kind: c_int) -> *mut c_void;
        fn acl_valid(acl: *mut c_void) -> c_int;
        fn acl_get_entry(acl: *mut c_void, index: c_int, entry: *mut *mut c_void) -> c_int;
    }
    struct Acl(*mut c_void);
    impl Drop for Acl {
        fn drop(&mut self) {
            unsafe {
                acl_free(self.0);
            }
        }
    }
    pub fn clear(fd: c_int) -> Result<()> {
        let raw = unsafe { acl_init(0) };
        if raw.is_null() {
            return Err(error());
        }
        let acl = Acl(raw);
        if unsafe { acl_set_fd_np(fd, acl.0, 0x100) } != 0 {
            return Err(error());
        }
        verify(fd)
    }
    pub fn verify(fd: c_int) -> Result<()> {
        let raw = unsafe { acl_get_fd_np(fd, 0x100) };
        if raw.is_null() {
            return if io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                Ok(())
            } else {
                Err(error())
            };
        }
        let acl = Acl(raw);
        let mut entry = std::ptr::null_mut();
        if unsafe { acl_valid(acl.0) } != 0 || unsafe { acl_get_entry(acl.0, 0, &mut entry) } == 0 {
            return Err("Private storage ACL could not be enforced".into());
        }
        Ok(())
    }
}
#[cfg(not(target_os = "macos"))]
mod acl {
    use super::*;
    pub fn clear(_: libc::c_int) -> Result<()> {
        Ok(())
    }
    pub fn verify(_: libc::c_int) -> Result<()> {
        Ok(())
    }
}
fn verify(file: &File, mode: libc::mode_t, kind: libc::mode_t) -> Result<()> {
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut info) } != 0 {
        return Err(error());
    }
    if info.st_uid != unsafe { libc::getuid() }
        || info.st_mode & libc::S_IFMT != kind
        || info.st_mode & 0o777 != mode
    {
        return Err("Private storage permissions could not be enforced".into());
    }
    acl::verify(file.as_raw_fd())
}
fn directory(path: &Path) -> Result<File> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())
}
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;

/// Identity captured from the open payload before publication. A caller can
/// distinguish a pre-publication failure from a post-rename durability error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // The legacy CLI ignores identities; the Rust host consumes them.
pub struct Publication {
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug)]
#[allow(dead_code)] // This shared source also compiles into the legacy Result<()> CLI.
pub enum WriteOutcome {
    Durable(Publication),
    Published {
        identity: Publication,
        error: String,
    },
}

/// Publication may have succeeded even if directory fsync failed. Preserve
/// this fact for runtime reconciliation; never report that old bytes remain.
pub fn write_outcome(path: &Path, bytes: &[u8], replace: bool) -> Result<WriteOutcome> {
    write_outcome_guarded(path, bytes, replace, |publish| publish())
}
/// Stage and sync privately before entering a caller-owned publication gate.
/// The gate must execute `publish` at most once while holding its permit; the
/// publication identity is recorded inside that closure immediately after rename.
/// Returning an error before invocation leaves the destination unchanged.
pub fn write_outcome_guarded(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    gate: impl FnOnce(&mut dyn FnMut() -> Result<()>) -> Result<()>,
) -> Result<WriteOutcome> {
    let parent = directory(path.parent().ok_or("Missing configuration directory")?)?;
    let leaf = path.file_name().ok_or("Missing configuration filename")?;
    write_outcome_at(&parent, leaf, bytes, replace, gate)
}
/// Borrow an already retained directory descriptor. No descriptor ownership is
/// taken and no ambient path is resolved. A caller supplying an SDK capability
/// must separately qualify its root/identity/lifecycle before this operation.
/// Existing chosen parent modes/ACLs are preserved, as in the path entrypoint.
pub fn write_outcome_at(
    parent: &File,
    leaf: &std::ffi::OsStr,
    bytes: &[u8],
    replace: bool,
    gate: impl FnOnce(&mut dyn FnMut() -> Result<()>) -> Result<()>,
) -> Result<WriteOutcome> {
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(parent.as_raw_fd(), &mut info) } != 0 {
        return Err(error());
    }
    if info.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err("Publication parent must be an open directory".into());
    }
    let raw = leaf.as_bytes();
    if raw.is_empty() || raw == b"." || raw == b".." || raw.contains(&b'/') {
        return Err("Publication requires one filename component".into());
    }
    let destination = name(raw)?;
    let mut published = None;
    match write_impl(parent, &destination, bytes, replace, &mut published, gate) {
        Ok(()) => Ok(WriteOutcome::Durable(
            published.ok_or("Missing published identity")?,
        )),
        Err(error) => match published {
            Some(identity) => Ok(WriteOutcome::Published { identity, error }),
            None => Err(error),
        },
    }
}
pub fn secure_directory(path: &Path) -> Result<()> {
    // Existing profile aliases retain their selection policy; secure the actual FD.
    let file = directory(path)?;
    secure_directory_fd(&file)
}
/// Secure the actual borrowed directory descriptor without resolving a path,
/// changing descriptor ownership, or modifying an unrelated numeric descriptor.
pub fn secure_directory_fd(file: &File) -> Result<()> {
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut info) } != 0 {
        return Err(error());
    }
    if info.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err("Profile descriptor must be a directory".into());
    }
    if info.st_uid != unsafe { libc::getuid() } {
        return Err("Profile must be owned by the current user".into());
    }
    if unsafe { libc::fchmod(file.as_raw_fd(), 0o700) } != 0 {
        return Err(error());
    }
    acl::clear(file.as_raw_fd())?;
    verify(file, 0o700, libc::S_IFDIR)
}
struct Stage<'a> {
    parent: &'a File,
    name: CString,
    device: libc::dev_t,
    inode: libc::ino_t,
    directory: Option<File>,
}
impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if let Ok(info) = metadata(self.parent, &self.name) {
            if info.st_mode & libc::S_IFMT == libc::S_IFDIR
                && info.st_dev == self.device
                && info.st_ino == self.inode
            {
                unsafe {
                    libc::unlinkat(
                        self.parent.as_raw_fd(),
                        self.name.as_ptr(),
                        libc::AT_REMOVEDIR,
                    );
                }
            }
        }
    }
}
pub fn write(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    match write_outcome(path, bytes, replace)? {
        WriteOutcome::Durable(_) => Ok(()),
        WriteOutcome::Published { error, .. } => Err(error),
    }
}
fn write_impl(
    parent: &File,
    destination: &CString,
    bytes: &[u8],
    replace: bool,
    published: &mut Option<Publication>,
    gate: impl FnOnce(&mut dyn FnMut() -> Result<()>) -> Result<()>,
) -> Result<()> {
    let temporary = create_stage(parent, stage_name)?;
    let created = metadata(parent, &temporary)?;
    if created.st_uid != unsafe { libc::getuid() }
        || created.st_mode & libc::S_IFMT != libc::S_IFDIR
    {
        return Err("Invalid private staging directory".into());
    }
    let mut stage = Stage {
        parent,
        name: temporary,
        device: created.st_dev,
        inode: created.st_ino,
        directory: None,
    };
    stage.directory = Some(descriptor(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            stage.name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })?);
    let staging = stage
        .directory
        .as_ref()
        .ok_or("Missing staging descriptor")?;
    let mut opened: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(staging.as_raw_fd(), &mut opened) } != 0 {
        return Err(error());
    }
    if opened.st_dev != stage.device
        || opened.st_ino != stage.inode
        || opened.st_uid != unsafe { libc::getuid() }
    {
        return Err("Private staging directory changed".into());
    }
    if unsafe { libc::fchmod(staging.as_raw_fd(), 0o700) } != 0 {
        return Err(error());
    }
    acl::clear(staging.as_raw_fd())?;
    verify(staging, 0o700, libc::S_IFDIR)?;
    let payload = name(b"data")?;
    let mut created_payload = false;
    let result = (|| {
        let mut file = descriptor(unsafe {
            libc::openat(
                staging.as_raw_fd(),
                payload.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_int,
            )
        })?;
        created_payload = true;
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
            return Err(error());
        }
        verify(&file, 0o600, libc::S_IFREG)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        let info = file.metadata().map_err(|e| e.to_string())?;
        let identity = Publication {
            device: info.dev(),
            inode: info.ino(),
        };
        let mut publish = || {
            let status = unsafe {
                if replace {
                    libc::renameat(
                        staging.as_raw_fd(),
                        payload.as_ptr(),
                        parent.as_raw_fd(),
                        destination.as_ptr(),
                    )
                } else {
                    #[cfg(target_os = "macos")]
                    {
                        libc::renameatx_np(
                            staging.as_raw_fd(),
                            payload.as_ptr(),
                            parent.as_raw_fd(),
                            destination.as_ptr(),
                            libc::RENAME_EXCL,
                        )
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        libc::linkat(
                            staging.as_raw_fd(),
                            payload.as_ptr(),
                            parent.as_raw_fd(),
                            destination.as_ptr(),
                            0,
                        )
                    }
                }
            };
            if status != 0 {
                return Err(error());
            }
            *published = Some(identity);
            Ok(())
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gate(&mut publish)))
            .unwrap_or_else(|_| Err("Private publication gate failed.".into()))?;
        #[cfg(not(target_os = "macos"))]
        if !replace && unsafe { libc::unlinkat(staging.as_raw_fd(), payload.as_ptr(), 0) } != 0 {
            return Err(error());
        }
        parent.sync_all().map_err(|e| e.to_string())
    })();
    if created_payload {
        unsafe {
            libc::unlinkat(staging.as_raw_fd(), payload.as_ptr(), 0);
        }
    }
    result
}

#[cfg(test)]
mod guarded_publication_tests {
    use super::*;
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        path::PathBuf,
        sync::{Arc, Barrier},
    };
    struct Own(PathBuf);
    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    impl Own {
        fn new() -> Self {
            let root = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"))
                .join(format!(
                    "guarded-publication-{}-{}",
                    std::process::id(),
                    FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(root)
        }
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn nonempty_payloads_remain_private_before_publication_under_four_creation_policies() {
        use std::io::Read;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::time::{Duration, Instant};
        const CHILD: &str = "RUYCAST_PRIVATE_OBSERVER_CHILD";
        const CASE: &str = "private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies";
        let parent=std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent required");
        assert!(Path::new(&parent).is_absolute());
        let Some(policy)=std::env::var_os(CHILD) else {
            for policy in ["022", "000", "acl", "077"] {
                let mut child=std::process::Command::new(std::env::current_exe().unwrap())
                    .args([CASE,"--exact","--test-threads=1"])
                    .env_clear().env("PATH","/usr/bin:/bin").env("HOME",&parent)
                    .env("RUYCAST_NATIVE_TEST_FIXTURE_DIR",&parent).env(CHILD,policy)
                    .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
                let deadline=Instant::now()+Duration::from_secs(20);
                loop {
                    if child.try_wait().unwrap().is_some(){break;}
                    if Instant::now()>=deadline {let _=child.kill();let _=child.wait();panic!("owned storage observer child timed out: {policy}");}
                    std::thread::sleep(Duration::from_millis(5));
                }
                let output=child.wait_with_output().unwrap();
                assert!(output.status.success(),"policy {policy}: {} {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
                assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed; 0 ignored;"));
            }
            return;
        };
        assert_ne!(unsafe{libc::getuid()},0);
        let own=Own::new();
        let inherited=policy=="acl";
        if inherited {
            let output=std::process::Command::new("/bin/chmod").args(["+a","everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit"]).arg(&own.0).output().unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
            assert!(acl::verify(File::open(&own.0).unwrap().as_raw_fd()).is_err(),"real inherited ACL required");
        }
        let acl_rows=|| {
            let output=std::process::Command::new("/bin/ls").arg("-lde").arg(&own.0).output().unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().lines().skip(1).map(str::to_owned).collect::<Vec<_>>()
        };
        let parent_acl=acl_rows();
        assert!(!inherited||!parent_acl.is_empty());
        struct Mask(libc::mode_t);
        impl Drop for Mask {fn drop(&mut self){unsafe{libc::umask(self.0);}}}
        let mask=match policy.to_str().unwrap(){"022"|"acl"=>0o022,"000"=>0,"077"=>0o077,_=>panic!("unknown owned policy")};
        let _mask=Mask(unsafe{libc::umask(mask)});
        let done=Arc::new(AtomicBool::new(false));
        let staged=Arc::new(AtomicUsize::new(0));
        let observations=Arc::new(AtomicUsize::new(0));
        let (ready_tx,ready_rx)=std::sync::mpsc::sync_channel(1);
        let root=own.0.clone();let observer_done=done.clone();let observer_staged=staged.clone();let observer_count=observations.clone();
        let observer=std::thread::spawn(move|| {
            let mut seen=std::collections::BTreeSet::new();
            ready_tx.send(()).unwrap();
            while !observer_done.load(Ordering::Acquire) {
                let mut paths=Vec::new();
                for entry in std::fs::read_dir(&root).unwrap() {
                    let entry=entry.unwrap();
                    match entry.file_type() {
                        Ok(kind) if kind.is_dir()=>paths.push((entry.path().join("data"),true)),
                        Ok(kind) if kind.is_file()=>paths.push((entry.path(),false)),
                        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                        other=>panic!("unexpected owned entry: {other:?}"),
                    }
                }
                for (path,is_stage) in paths {
                    let mut file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&path) {
                        Ok(file)=>file,
                        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>continue,
                        Err(error)=>panic!("owned observer open: {error}"),
                    };
                    if file.metadata().unwrap().len()==0 {continue;}
                    verify(&file,0o600,libc::S_IFREG).unwrap();
                    let mut byte=[0u8;1];assert_eq!(file.read(&mut byte).unwrap(),1);
                    observer_count.fetch_add(1,Ordering::Release);
                    if is_stage&&seen.insert(path){observer_staged.fetch_add(1,Ordering::Release);}
                }
                std::thread::yield_now();
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let target=own.0.join("private.json");
        let result=(||->Result<()> {
            for iteration in 0..20 {
                let payload=serde_json::to_vec(&serde_json::json!({"iteration":iteration.to_string(),"text":"owned synthetic bytes Ω"})).unwrap();
                let outcome=write_outcome_guarded(&target,&payload,true,|publish| {
                    let deadline=Instant::now()+Duration::from_secs(2);
                    while staged.load(Ordering::Acquire)<iteration+1 {
                        if Instant::now()>=deadline{return Err("observer did not see this nonempty staged payload".into());}
                        std::thread::yield_now();
                    }
                    publish()
                })?;
                assert!(matches!(outcome,WriteOutcome::Durable(_)));
            }
            Ok(())
        })();
        done.store(true,Ordering::Release);
        observer.join().unwrap();result.unwrap();
        assert_eq!(staged.load(Ordering::Acquire),20);
        assert!(observations.load(Ordering::Acquire)>=20);
        verify(&File::open(&target).unwrap(),0o600,libc::S_IFREG).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&target).unwrap()).unwrap()["iteration"],"19");
        assert_eq!(std::fs::read_dir(&own.0).unwrap().map(|entry|entry.unwrap().file_name()).collect::<Vec<_>>(),vec![std::ffi::OsString::from("private.json")]);
        assert_eq!(std::fs::metadata(&own.0).unwrap().mode()&0o777,0o755);
        assert_eq!(acl_rows(),parent_acl);
    }
    #[test]
    fn unsearchable_umask_refuses_before_payload_in_an_isolated_process() {
        const CHILD: &str = "RUYCAST_PRIVATE_UMASK_CHILD";
        const CASE: &str = "private_storage::guarded_publication_tests::unsearchable_umask_refuses_before_payload_in_an_isolated_process";
        assert!(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").is_some(),
            "explicit owned fixture parent required"
        );
        if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([CASE, "--exact", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "isolated mask test failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let output = String::from_utf8(result.stdout).unwrap();
            assert!(
                output.contains(&format!("test {CASE} ... ok")),
                "exact child case did not pass: {output}"
            );
            assert!(
                output.contains("1 passed; 0 failed; 0 ignored;"),
                "child case was skipped: {output}"
            );
            return;
        }
        // umask is process-global: only this single-case child changes it.
        assert_ne!(
            unsafe { libc::getuid() },
            0,
            "this permission refusal requires a non-root fixture owner"
        );
        let own = Own::new();
        let path = own.0.join("config.json");
        std::fs::write(&path, b"prior synthetic bytes").unwrap();
        let previous = unsafe { libc::umask(0o777) };
        let outcome = write_outcome(&path, b"must not publish", true);
        unsafe {
            libc::umask(previous);
        }
        assert!(outcome.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"prior synthetic bytes");
        assert_eq!(
            std::fs::read_dir(&own.0).unwrap().count(),
            1,
            "a failed stage must not leave payload or staging entries"
        );
        assert_eq!(std::fs::metadata(&own.0).unwrap().mode() & 0o777, 0o755);
    }
    #[test]
    fn create_only_preserves_destination_symlink_and_directory_contents() {
        let own = Own::new();
        let target = own.0.join("existing.json");
        std::fs::write(&target, b"preserved synthetic payload").unwrap();
        let link = own.0.join("linked.json");
        symlink(&target, &link).unwrap();
        assert!(write_outcome(&link, b"must not replace link", false).is_err());
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"preserved synthetic payload"
        );
        let directory = own.0.join("directory.json");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("sentinel"), b"preserved child").unwrap();
        for replace in [false, true] {
            assert!(write_outcome(&directory, b"must not replace directory", replace).is_err());
            assert_eq!(
                std::fs::read(directory.join("sentinel")).unwrap(),
                b"preserved child"
            );
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
            assert_eq!(
                std::fs::read_dir(&own.0).unwrap().count(),
                3,
                "failed publication must clean only its own stage"
            );
        }
    }
    #[test]
    fn descriptor_publication_uses_retained_directory_not_replaced_path() {
        assert!(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").is_some(),
            "explicit owned fixture parent required"
        );
        let own = Own::new();
        let selected = own.0.join("selected");
        let moved = own.0.join("retained");
        std::fs::create_dir(&selected).unwrap();
        std::fs::set_permissions(&selected, std::fs::Permissions::from_mode(0o755)).unwrap();
        let descriptor = File::open(&selected).unwrap();
        let before = descriptor.metadata().unwrap();
        std::fs::rename(&selected, &moved).unwrap();
        std::fs::create_dir(&selected).unwrap();
        std::fs::set_permissions(&selected, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(selected.join("sentinel"), b"do-not-change").unwrap();
        let leaf = std::ffi::OsStr::new("config.json");
        let outcome =
            write_outcome_at(&descriptor, leaf, b"original", false, |publish| publish()).unwrap();
        let WriteOutcome::Durable(identity) = outcome else {
            panic!("own directory should sync")
        };
        let actual = std::fs::metadata(moved.join("config.json")).unwrap();
        assert_eq!(
            (actual.dev(), actual.ino()),
            (identity.device, identity.inode)
        );
        assert_eq!(actual.mode() & 0o777, 0o600);
        assert_eq!(descriptor.metadata().unwrap().mode() & 0o777, 0o755);
        assert_eq!(
            (
                descriptor.metadata().unwrap().dev(),
                descriptor.metadata().unwrap().ino()
            ),
            (before.dev(), before.ino())
        );
        assert!(!selected.join("config.json").exists());
        assert_eq!(
            std::fs::read(selected.join("sentinel")).unwrap(),
            b"do-not-change"
        );
        assert!(
            write_outcome_at(&descriptor, leaf, b"do-not-replace", false, |publish| {
                publish()
            })
            .is_err()
        );
        assert_eq!(
            std::fs::read(moved.join("config.json")).unwrap(),
            b"original"
        );
        assert!(
            write_outcome_at(&descriptor, leaf, b"cancel", true, |_| Err(
                "own before publication".into()
            ))
            .is_err()
        );
        assert_eq!(
            std::fs::read(moved.join("config.json")).unwrap(),
            b"original"
        );
        let outcome = write_outcome_at(&descriptor, leaf, b"published", true, |publish| {
            publish()?;
            Err("own after publication".into())
        })
        .unwrap();
        let WriteOutcome::Published { identity, error } = outcome else {
            panic!("must retain published outcome")
        };
        assert_eq!(error, "own after publication");
        let actual = std::fs::metadata(moved.join("config.json")).unwrap();
        assert_eq!(
            (actual.dev(), actual.ino()),
            (identity.device, identity.inode)
        );
        assert_eq!(
            std::fs::read(moved.join("config.json")).unwrap(),
            b"published"
        );
        // The borrowed descriptor remains open and caller-owned after every outcome.
        assert!(descriptor.metadata().unwrap().is_dir());
        secure_directory_fd(&descriptor).unwrap();
        assert_eq!(std::fs::metadata(&moved).unwrap().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&selected).unwrap().mode() & 0o777, 0o755);
    }
    #[test]
    fn descriptor_entry_refuses_non_directory_and_non_leaf_without_mutation() {
        assert!(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").is_some(),
            "explicit owned fixture parent required"
        );
        let own = Own::new();
        let sentinel = own.0.join("sentinel");
        std::fs::write(&sentinel, b"untouched").unwrap();
        std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o644)).unwrap();
        let file = File::open(&sentinel).unwrap();
        assert!(secure_directory_fd(&file).is_err());
        assert!(write_outcome_at(
            &file,
            std::ffi::OsStr::new("config.json"),
            b"no",
            true,
            |publish| publish()
        )
        .is_err());
        let directory = File::open(&own.0).unwrap();
        for invalid in [
            "",
            ".",
            "..",
            "../escape",
            "nested/file",
            "/absolute",
            "nul\0leaf",
        ] {
            assert!(write_outcome_at(
                &directory,
                std::ffi::OsStr::new(invalid),
                b"no",
                true,
                |publish| publish()
            )
            .is_err());
        }
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched");
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o644);
        assert_eq!(std::fs::read_dir(&own.0).unwrap().count(), 1);
    }
    impl Drop for Own {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn concurrent_publications_keep_whole_payloads_and_leave_no_stages() {
        let own = Own::new();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let root = &own.0;
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for item in 0..8 {
                        let payload = format!("writer-{writer}-item-{item}\n").repeat(2048);
                        let path = root.join(format!("{writer}-{item}.json"));
                        write(&path, payload.as_bytes(), false).unwrap();
                        assert_eq!(std::fs::read(&path).unwrap(), payload.as_bytes());
                        assert!(write(&path, b"must not replace", false).is_err());
                        assert_eq!(std::fs::read(&path).unwrap(), payload.as_bytes());
                    }
                    let payload = format!("writer-{writer}\n").repeat(4096);
                    write(&root.join("shared.json"), payload.as_bytes(), true).unwrap();
                });
            }
        });
        let shared = std::fs::read(own.0.join("shared.json")).unwrap();
        assert!((0..8).any(|writer| shared == format!("writer-{writer}\n").repeat(4096).as_bytes()));
        let entries: Vec<_> = std::fs::read_dir(&own.0)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        assert_eq!(entries.len(), 65);
        for entry in entries {
            assert!(entry.file_type().unwrap().is_file());
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn existing_stage_names_are_never_adopted_or_removed() {
        let own = Own::new();
        let target = own.0.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("sentinel"), b"unchanged").unwrap();
        symlink(&target, own.0.join("occupied-link")).unwrap();
        std::fs::create_dir(own.0.join("occupied-directory")).unwrap();
        let parent = File::open(&own.0).unwrap();
        let mut names = ["occupied-link", "occupied-directory", "new-stage"].into_iter();
        let created = create_stage(&parent, || name(names.next().unwrap().as_bytes())).unwrap();
        assert_eq!(created.as_bytes(), b"new-stage");
        assert!(std::fs::symlink_metadata(own.0.join("occupied-link"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read(target.join("sentinel")).unwrap(),
            b"unchanged"
        );
        assert!(own.0.join("occupied-directory").is_dir());
        let mut attempts = 0;
        let result = create_stage(&parent, || {
            attempts += 1;
            name(b"occupied-link")
        });
        assert_eq!(attempts, 128);
        assert_eq!(
            result.unwrap_err(),
            "Private staging directory names unavailable"
        );
        assert_eq!(std::fs::read_dir(&own.0).unwrap().count(), 4);
    }

    #[test]
    fn non_collision_creation_failure_is_not_retried() {
        let own = Own::new();
        let path = own.0.join("not-directory");
        std::fs::write(&path, b"unchanged").unwrap();
        let parent = File::open(&path).unwrap();
        let mut attempts = 0;
        assert!(create_stage(&parent, || {
            attempts += 1;
            name(b"not-created")
        })
        .is_err());
        assert_eq!(attempts, 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
        assert_eq!(std::fs::read_dir(&own.0).unwrap().count(), 1);
    }
    #[test]
    fn rejected_or_panicking_gate_preserves_prior_bytes_and_cleans_staging() {
        let own = Own::new();
        let path = own.0.join("diagnostics.json");
        std::fs::write(&path, b"prior").unwrap();
        assert!(write_outcome_guarded(&path, b"new", true, |_| Err("revoked".into())).is_err());
        assert!(
            write_outcome_guarded(&path, b"new", true, |_| panic!("owned gate panic")).is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"prior");
        assert_eq!(std::fs::read_dir(&own.0).unwrap().count(), 1);
        assert_eq!(
            std::fs::metadata(&own.0).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
    #[test]
    fn gate_owns_actual_publication_and_postcommit_panic_reports_published() {
        let own = Own::new();
        let path = own.0.join("diagnostics.json");
        let target = own.0.join("prior-target");
        std::fs::write(&target, b"untouched").unwrap();
        symlink(&target, &path).unwrap();
        let result = write_outcome_guarded(&path, b"new", true, |publish| {
            assert_eq!(std::fs::read(&path).unwrap(), b"untouched");
            publish()?;
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
            panic!("owned postcommit panic")
        })
        .unwrap();
        assert!(matches!(result, WriteOutcome::Published { .. }));
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(&own.0).unwrap().count(), 2);
    }
}
