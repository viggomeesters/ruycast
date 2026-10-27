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
    time::{SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, String>;
fn error() -> String {
    io::Error::last_os_error().to_string()
}
fn name(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| "Invalid storage path".into())
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
use std::os::unix::fs::OpenOptionsExt;
pub fn secure_directory(path: &Path) -> Result<()> {
    // Existing profile aliases retain their selection policy; secure the actual FD.
    let file = directory(path)?;
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut info) } != 0 {
        return Err(error());
    }
    if info.st_uid != unsafe { libc::getuid() } {
        return Err("Profile must be owned by the current user".into());
    }
    if unsafe { libc::fchmod(file.as_raw_fd(), 0o700) } != 0 {
        return Err(error());
    }
    acl::clear(file.as_raw_fd())?;
    verify(&file, 0o700, libc::S_IFDIR)
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
    let parent = directory(path.parent().ok_or("Missing configuration directory")?)?;
    let destination = name(
        path.file_name()
            .ok_or("Missing configuration filename")?
            .as_bytes(),
    )?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = name(format!(".ruycast-{}-{nonce}.tmp", std::process::id()).as_bytes())?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), temporary.as_ptr(), 0o700) } != 0 {
        return Err(error());
    }
    let created = metadata(&parent, &temporary)?;
    if created.st_uid != unsafe { libc::getuid() }
        || created.st_mode & libc::S_IFMT != libc::S_IFDIR
    {
        return Err("Invalid private staging directory".into());
    }
    let mut stage = Stage {
        parent: &parent,
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
    verify(&staging, 0o700, libc::S_IFDIR)?;
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
