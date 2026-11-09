use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Component,
    sync::atomic::{AtomicU64, Ordering},
};
pub fn absolute(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path.to_str().is_none()
        || path.components().any(|c| matches!(c, Component::ParentDir))
        || path.as_os_str().is_empty()
    {
        return Err("Choose an absolute UTF-8 path without parent components.".into());
    }
    Ok(path.components().collect())
}
pub fn no_links(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for part in absolute(path)?.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("Linked installation path preserved.".into())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
pub fn bytes(path: &Path) -> Result<Vec<u8>> {
    no_links(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(err)?;
    let before = file.metadata().map_err(err)?;
    if !before.is_file() || before.nlink() != 1 || before.len() > 256 * 1024 * 1024 {
        return Err("Unsafe installation file preserved.".into());
    }
    let mut out = vec![];
    Read::by_ref(&mut file)
        .take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut out)
        .map_err(err)?;
    let after = file.metadata().map_err(err)?;
    let named = std::fs::symlink_metadata(path).map_err(err)?;
    if out.len() as u64 != before.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || named.dev() != before.dev()
        || named.ino() != before.ino()
    {
        return Err("Installation input changed while reading.".into());
    }
    Ok(out)
}
pub fn hash(path: &Path) -> Result<String> {
    Ok(digest(&bytes(path)?))
}
pub fn directory(path: &Path, mode: u32) -> Result<()> {
    no_links(path)?;
    std::fs::create_dir_all(path).map_err(err)?;
    let m = std::fs::symlink_metadata(path).map_err(err)?;
    if !m.is_dir() || m.uid() != unsafe { libc::getuid() } {
        return Err("Installation directory ownership mismatch.".into());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(err)
}
pub fn fresh(parent: &Path, prefix: &str) -> Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    no_links(parent)?;
    for _ in 0..32 {
        let path = parent.join(format!(
            "{prefix}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .map_err(err)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(err(e)),
        }
    }
    Err("Could not reserve an exclusive staging directory.".into())
}
pub fn create(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    no_links(path)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(err)?;
    let identity = f.metadata().map_err(err)?;
    let result = f
        .set_permissions(std::fs::Permissions::from_mode(mode))
        .and_then(|_| f.write_all(data))
        .and_then(|_| f.sync_all())
        .map_err(err);
    if result.is_err()
        && std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.dev() == identity.dev() && m.ino() == identity.ino())
    {
        let _ = std::fs::remove_file(path);
    }
    result
}
pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path).and_then(|f| f.sync_all()).map_err(err)
}
pub fn replace(path: &Path, data: &[u8], expected: Option<&[u8]>, mode: u32) -> Result<()> {
    if expected
        .map(|v| bytes(path).map(|b| b != v))
        .transpose()?
        .unwrap_or_else(|| std::fs::symlink_metadata(path).is_ok())
    {
        return Err("Installation metadata changed; preserved.".into());
    }
    let parent = path.parent().ok_or("Missing installation parent.")?;
    let work = fresh(parent, ".ruycast-write-")?;
    let staged = work.join("value");
    create(&staged, data, mode)?;
    let result = (|| {
        if let Some(prior) = expected {
            if bytes(path)? != prior {
                return Err("Installation metadata changed; preserved.".into());
            }
            std::fs::rename(&staged, path).map_err(err)?;
        } else {
            rename(&staged, path, false)?;
        }
        sync_dir(parent)
    })();
    let _ = std::fs::remove_dir_all(work);
    result
}
pub fn rename(from: &Path, to: &Path, swap: bool) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let a = std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(err)?;
    let b = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(err)?;
    unsafe extern "C" {
        fn renamex_np(
            a: *const libc::c_char,
            b: *const libc::c_char,
            flags: libc::c_uint,
        ) -> libc::c_int;
    }
    // Darwin stdio.h: RENAME_SWAP=0x2, RENAME_EXCL=0x4.
    if unsafe { renamex_np(a.as_ptr(), b.as_ptr(), if swap { 2 } else { 4 }) } != 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    Ok(())
}
pub fn tree(root: &Path) -> Result<BTreeMap<String, (String, u32)>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, (String, u32)>) -> Result<()> {
        no_links(at)?;
        let m = std::fs::symlink_metadata(at).map_err(err)?;
        if m.is_dir() {
            for item in std::fs::read_dir(at).map_err(err)? {
                walk(root, &item.map_err(err)?.path(), out)?;
            }
        } else {
            out.insert(
                at.strip_prefix(root)
                    .map_err(err)?
                    .to_str()
                    .ok_or("Non-UTF8 bundle path.")?
                    .into(),
                (hash(at)?, m.mode() & 0o7777),
            );
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}
pub fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let before = tree(source)?;
    std::fs::create_dir(target).map_err(err)?;
    for (name, (expected, mode)) in &before {
        let path = target.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        let data = bytes(&source.join(name))?;
        if digest(&data) != *expected {
            return Err("Bundle changed during staging.".into());
        }
        create(&path, &data, *mode)?;
    }
    if tree(source)? != before || tree(target)? != before {
        return Err("Bundle changed during staging.".into());
    }
    Ok(())
}
pub struct Lock(File);
impl Lock {
    pub fn acquire(path: &Path, retries: usize) -> Result<Self> {
        no_links(path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(err)?;
        let m = file.metadata().map_err(err)?;
        if !m.is_file() || m.nlink() != 1 || m.uid() != unsafe { libc::getuid() } {
            return Err("Invalid installation lock.".into());
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(err)?;
        for _ in 0..=retries {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self(file));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("Existing launcher or builder still owns the lock; installation preserved.".into())
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
