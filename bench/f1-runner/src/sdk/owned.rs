//! Directory capabilities, not ambient path permissions. All traversal uses
//! retained directory descriptors and O_NOFOLLOW; links never grant ownership.
use std::{
    ffi::{CStr, CString},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
pub type Result<T> = std::result::Result<T, String>;
fn c(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| "NUL in owned path".into())
}
fn parts(p: &Path) -> Result<Vec<CString>> {
    if p.components().any(|x| {
        matches!(
            x,
            Component::ParentDir | Component::CurDir | Component::Prefix(_)
        )
    }) {
        return Err("parent/relative path component refused".into());
    }
    p.components()
        .filter_map(|x| match x {
            Component::Normal(v) => Some(c(v.as_bytes())),
            _ => None,
        })
        .collect()
}
fn fd(raw: i32) -> Result<OwnedFd> {
    if raw < 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}
fn duplicate(value: &OwnedFd) -> Result<OwnedFd> {
    fd(unsafe { libc::fcntl(value.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })
}
fn dir_at(base: &OwnedFd, name: &CStr) -> Result<OwnedFd> {
    fd(unsafe {
        libc::openat(
            base.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })
}
#[derive(Clone, Debug)]
pub struct OwnedRoot {
    path: PathBuf,
    descriptor: Arc<OwnedFd>,
}
#[derive(Debug)]
pub struct Entry {
    pub name: String,
    pub directory: bool,
    pub bytes: u64,
}
impl OwnedRoot {
    /// Create an actual descriptor from this retained directory capability.
    /// An arbitrary integer or successful fstat can never construct this grant.
    pub fn open_regular(
        &self,
        path: &Path,
        access: FileAccess,
        creation: FileCreation,
        life: Arc<dyn AliveLease>,
    ) -> Result<RegularFile> {
        live(&*life)?;
        let (directory, name) = self.parent(path)?;
        let mode = match access {
            FileAccess::Read => libc::O_RDONLY,
            FileAccess::Write => libc::O_WRONLY,
            FileAccess::ReadWrite => libc::O_RDWR,
        };
        let flags = mode
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | match creation {
                FileCreation::Existing => 0,
                FileCreation::CreateNew => libc::O_CREAT | libc::O_EXCL,
                FileCreation::CreateOrOpen => libc::O_CREAT,
            };
        // No O_TRUNC: validate the newly opened descriptor before any mutation.
        let opened =
            fd(unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) })?;
        let metadata = stat(&opened)?;
        let owner = self.owner_ids()?;
        if metadata.st_mode & libc::S_IFMT != libc::S_IFREG
            || metadata.st_nlink != 1
            || (metadata.st_uid, metadata.st_gid) != owner
        {
            return Err("unqualified regular descriptor".into());
        }
        live(&*life)?;
        Ok(RegularFile {
            root: self.clone(),
            path: path.into(),
            file: Mutex::new(std::fs::File::from(opened)),
            identity: (metadata.st_dev as u64, metadata.st_ino),
            access,
            life,
        })
    }
    /// The caller grants only this exact, existing root. No environment lookup.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err("owned root must be absolute".into());
        }
        let names = parts(path)?;
        if names.is_empty() {
            return Err("filesystem root cannot be granted".into());
        }
        let mut current = fd(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        for name in names {
            current = dir_at(&current, &name)?;
        }
        Ok(Self {
            path: path.into(),
            descriptor: Arc::new(current),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn owner_ids(&self) -> Result<(libc::uid_t, libc::gid_t)> {
        let s = stat(&self.descriptor)?;
        Ok((s.st_uid, s.st_gid))
    }
    fn names(&self, path: &Path) -> Result<Vec<CString>> {
        if !path.is_absolute() {
            return Err("relative path refused".into());
        }
        parts(path)?;
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| "unowned namespace refused before filesystem access")?;
        parts(relative)
    }
    fn parent(&self, path: &Path) -> Result<(OwnedFd, CString)> {
        let mut names = self.names(path)?;
        let leaf = names.pop().ok_or("root mutation refused")?;
        let mut current = duplicate(&self.descriptor)?;
        for name in names {
            current = dir_at(&current, &name)?
        }
        Ok((current, leaf))
    }
    /// Resolve a current directory only below a retained ancestor. An exact
    /// target-inode root alone cannot grant reopening after path replacement.
    pub fn open_watch_directory(&self, path: &Path) -> Result<OwnedFd> {
        if self.names(path)?.is_empty() {
            return Err("watch rebind requires an explicit ancestor grant".into());
        }
        let (parent, leaf) = self.parent(path)?;
        let directory = fd(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_EVTONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let metadata = stat(&directory)?;
        if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
            || (metadata.st_uid, metadata.st_gid) != self.owner_ids()?
        {
            return Err("unqualified watch directory".into());
        }
        Ok(directory)
    }
    pub fn checked(&self, path: &Path) -> Result<OwnedFd> {
        self.checked_optional(path)?
            .ok_or_else(|| std::io::Error::from_raw_os_error(libc::ENOENT).to_string())
    }
    /// Only an absent final leaf is missing. Traversal failures, including an
    /// absent parent, retain their errors and cannot become a missing grant.
    pub fn checked_optional(&self, path: &Path) -> Result<Option<OwnedFd>> {
        let names = self.names(path)?;
        if names.is_empty() {
            return duplicate(&self.descriptor).map(Some);
        }
        let (parent, leaf) = self.parent(path)?;
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ENOENT) {
                Ok(None)
            } else {
                Err(error.to_string())
            };
        }
        let value = fd(raw)?;
        let metadata = stat(&value)?;
        if metadata.st_mode & libc::S_IFMT != libc::S_IFREG
            && metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            return Err("nonregular owned entry refused".into());
        }
        if metadata.st_mode & libc::S_IFMT == libc::S_IFREG && metadata.st_nlink != 1 {
            return Err("hardlinked file refused".into());
        }
        Ok(Some(value))
    }
    pub fn info(&self, path: &Path) -> Result<(bool, u64)> {
        let s = self.metadata(path)?;
        Ok((
            s.st_mode & libc::S_IFMT == libc::S_IFDIR,
            s.st_size.max(0) as u64,
        ))
    }
    pub(super) fn metadata(&self, path: &Path) -> Result<libc::stat> {
        stat(&self.checked(path)?)
    }
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    fn reference_parent(&self, path: &Path) -> std::result::Result<(Option<OwnedFd>, CString), i32> {
        let mut names = self.names(path).map_err(|_| libc::EPERM)?;
        let leaf = names.pop().ok_or(libc::EPERM)?;
        let mut directory = None::<OwnedFd>;
        for name in names {
            let parent = directory.as_ref().unwrap_or(&self.descriptor);
            let opened = unsafe {
                libc::openat(parent.as_raw_fd(), name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            };
            if opened < 0 {
                return Err(reference_errno());
            }
            directory = Some(unsafe { OwnedFd::from_raw_fd(opened) });
        }
        Ok((directory, leaf))
    }

    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    pub(crate) fn lstat(&self, path: &Path) -> std::result::Result<libc::stat, i32> {
        if path == self.path {
            return stat(&self.descriptor).map_err(|_| libc::EIO);
        }
        let (directory, leaf) = self.reference_parent(path)?;
        reference_leaf_stat(directory.as_ref().unwrap_or(&self.descriptor), &leaf)
    }

    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    pub(crate) fn unlink_reference_leaf(&self, path: &Path, socket_owner: Option<libc::uid_t>) -> std::result::Result<(), i32> {
        let (directory, leaf) = self.reference_parent(path)?;
        let parent = directory.as_ref().unwrap_or(&self.descriptor);
        if let Some(owner) = socket_owner {
            let info = reference_leaf_stat(parent, &leaf)?;
            if info.st_mode & libc::S_IFMT != libc::S_IFSOCK || info.st_uid != owner {
                return Err(libc::EPERM);
            }
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
            return Err(reference_errno());
        }
        Ok(())
    }

    pub fn read_optional(&self, path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
        use std::io::Read;
        let (parent, leaf) = self.parent(path)?;
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ENOENT) {
                Ok(None)
            } else {
                Err(error.to_string())
            };
        }
        let value = fd(raw)?;
        let s = stat(&value)?;
        if s.st_mode & libc::S_IFMT != libc::S_IFREG
            || s.st_nlink != 1
            || s.st_size < 0
            || s.st_size as u64 > limit
        {
            return Err("unqualified/oversized data input".into());
        }
        let mut bytes = vec![];
        std::fs::File::from(value)
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("owned data grew beyond bound".into());
        }
        Ok(Some(bytes))
    }
    pub fn create_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let (parent, name) = self.parent(path)?;
        let value = fd(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let mut file = std::fs::File::from(value);
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        let value = self.checked(path)?;
        if stat(&value)?.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err("enumeration requires directory".into());
        }
        let copy = dir_at(&value, c".")?;
        use std::os::fd::IntoRawFd;
        let raw = copy.into_raw_fd();
        let directory = unsafe { libc::fdopendir(raw) };
        if directory.is_null() {
            unsafe { libc::close(raw) };
            return Err("fdopendir failed".into());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let stream = Stream(directory);
        let mut result = vec![];
        loop {
            unsafe { *libc::__error() = 0 };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if unsafe { *libc::__error() } != 0 {
                    return Err("owned readdir failed".into());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| "non-UTF8 fixture name")?
                .to_owned();
            if name == "." || name == ".." {
                continue;
            }
            let (directory, bytes) = self.info(&path.join(&name))?;
            result.push(Entry {
                name,
                directory,
                bytes,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
    pub fn mkdir(&self, path: &Path, recursive: bool) -> Result<()> {
        let names = self.names(path)?;
        if names.is_empty() && !recursive {
            return Err("owned directory already exists".into());
        }
        let mut current = duplicate(&self.descriptor)?;
        for (index, name) in names.iter().enumerate() {
            if recursive || index + 1 == names.len() {
                let status = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) };
                if status != 0
                    && (!recursive
                        || std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST))
                {
                    return Err("owned mkdir failed".into());
                }
            }
            current = dir_at(&current, name)?;
        }
        Ok(())
    }
    pub fn remove(&self, path: &Path) -> Result<()> {
        let (directory, _) = self.info(path)?;
        if directory {
            for entry in self.list(path)? {
                self.remove(&path.join(entry.name))?;
            }
        }
        let (parent, name) = self.parent(path)?;
        if unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err("owned unlink failed".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileAccess {
    Read,
    Write,
    ReadWrite,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileCreation {
    Existing,
    CreateNew,
    CreateOrOpen,
}
fn live(life: &dyn AliveLease) -> Result<()> {
    if !life.is_alive() {
        return Err("regular descriptor guardian retired".into());
    }
    life.before_effect()
}
/// Owned open-description capability. Fields and construction are sealed to
/// root-relative open; no raw-fd adoption or ownership-by-metadata operation.
pub struct RegularFile {
    root: OwnedRoot,
    path: PathBuf,
    file: Mutex<std::fs::File>,
    identity: (u64, u64),
    access: FileAccess,
    life: Arc<dyn AliveLease>,
}
pub(crate) struct DescriptorParts {
    pub file: OwnedFd,
    pub root: OwnedRoot,
    pub path: PathBuf,
    pub access: FileAccess,
    pub life: Arc<dyn AliveLease>,
}
impl RegularFile {
    pub(crate) fn into_descriptor(self) -> Result<DescriptorParts> {
        live(&*self.life)?;
        let file = self
            .file
            .into_inner()
            .map_err(|_| "regular descriptor poisoned")?;
        Ok(DescriptorParts {
            file: file.into(),
            root: self.root,
            path: self.path,
            access: self.access,
            life: self.life,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn access(&self) -> FileAccess {
        self.access
    }
    fn checked(&self) -> Result<std::sync::MutexGuard<'_, std::fs::File>> {
        live(&*self.life)?;
        let file = self
            .file
            .lock()
            .map_err(|_| "regular descriptor poisoned")?;
        let mut s = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), s.as_mut_ptr()) } != 0 {
            return Err("issued regular descriptor unavailable".into());
        }
        let s = unsafe { s.assume_init() };
        if (s.st_dev as u64, s.st_ino) != self.identity
            || s.st_mode & libc::S_IFMT != libc::S_IFREG
            || s.st_nlink != 1
            || (s.st_uid, s.st_gid) != self.root.owner_ids()?
        {
            return Err("issued regular descriptor identity changed".into());
        }
        live(&*self.life)?;
        Ok(file)
    }
    pub fn read(&self, count: usize) -> Result<Vec<u8>> {
        use std::io::Read;
        if self.access == FileAccess::Write || count > 32_000_001 {
            return Err("regular descriptor read authority/bound refused".into());
        }
        let mut file = self.checked()?;
        let mut bytes = vec![0; count];
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        bytes.truncate(n);
        live(&*self.life)?;
        Ok(bytes)
    }
    pub fn write_all(&self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        if self.access == FileAccess::Read || bytes.len() > 32_000_000 {
            return Err("regular descriptor write authority/bound refused".into());
        }
        self.checked()?
            .write_all(bytes)
            .map_err(|e| e.to_string())?;
        live(&*self.life)
    }
    pub fn truncate(&self, length: u64) -> Result<()> {
        if self.access == FileAccess::Read || length > 32_000_000 {
            return Err("regular descriptor truncate authority/bound refused".into());
        }
        self.checked()?.set_len(length).map_err(|e| e.to_string())?;
        live(&*self.life)
    }
    pub fn seek(&self, offset: u64) -> Result<u64> {
        use std::io::{Seek, SeekFrom};
        let result = self
            .checked()?
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        live(&*self.life)?;
        Ok(result)
    }
    pub fn sync(&self) -> Result<()> {
        self.checked()?.sync_all().map_err(|e| e.to_string())?;
        live(&*self.life)
    }
    /// Duplicate remains a typed capability with the same explicit access/life.
    /// It does not expose close authority to a raw integer caller.
    pub fn duplicate(&self) -> Result<Self> {
        let file = self.checked()?.try_clone().map_err(|e| e.to_string())?;
        live(&*self.life)?;
        Ok(Self {
            root: self.root.clone(),
            path: self.path.clone(),
            file: Mutex::new(file),
            identity: self.identity,
            access: self.access,
            life: self.life.clone(),
        })
    }
}
#[cfg(test)]
#[test]
#[ignore = "requires a disposable private fixture parent"]
fn lstat_preserves_missing_and_does_not_follow_links() {
    use std::os::unix::fs::symlink;
    let path = PathBuf::from(std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT").unwrap());
    let root = OwnedRoot::open(&path).unwrap();
    assert_eq!(root.lstat(&path.join("missing/leaf")).err(), Some(libc::ENOENT));
    symlink(path.join("missing"), path.join("link")).unwrap();
    let info = root.lstat(&path.join("link")).unwrap();
    assert_eq!(info.st_mode & libc::S_IFMT, libc::S_IFLNK);
    assert!(root.lstat(&path.join("link/leaf")).is_err());
    assert_eq!(root.lstat(&path.join("../outside")).err(), Some(libc::EPERM));
    let owner = unsafe { libc::getuid() };
    assert_eq!(root.unlink_reference_leaf(&path.join("link"), Some(owner)), Err(libc::EPERM));
    root.unlink_reference_leaf(&path.join("link"), None).unwrap();
    assert_eq!(root.lstat(&path.join("link")).err(), Some(libc::ENOENT));
    let socket = path.join("socket");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    root.unlink_reference_leaf(&socket, Some(owner)).unwrap();
    assert_eq!(root.lstat(&socket).err(), Some(libc::ENOENT));
    drop(listener);
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn reference_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn reference_leaf_stat(parent: &OwnedFd, leaf: &CStr) -> std::result::Result<libc::stat, i32> {
    let mut output = std::mem::MaybeUninit::uninit();
    if unsafe { libc::fstatat(parent.as_raw_fd(), leaf.as_ptr(), output.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(reference_errno());
    }
    Ok(unsafe { output.assume_init() })
}

fn stat(value: &OwnedFd) -> Result<libc::stat> {
    let mut s = std::mem::MaybeUninit::uninit();
    if unsafe { libc::fstat(value.as_raw_fd(), s.as_mut_ptr()) } != 0 {
        return Err("owned fstat failed".into());
    }
    Ok(unsafe { s.assume_init() })
}
/// Shared lifecycle capability. Implemented by the sole guardian owner; never
/// contains a writer, arbitrary PID, or a way for the SDK to rearm a dead lease.
pub trait AliveLease: Send + Sync {
    fn is_alive(&self) -> bool;
    fn before_effect(&self) -> Result<()>;
}
/// Explicit capability bundle passed by the integrator. Modules never discover
/// these roots through environment/account/cache defaults.
#[derive(Clone)]
pub struct ModuleContext {
    pub home: OwnedRoot,
    pub cache: OwnedRoot,
    pub temp: OwnedRoot,
    pub applications: OwnedRoot,
    pub life: Arc<dyn AliveLease>,
}
impl ModuleContext {
    pub fn lease(&self, namespace: &'static str) -> Result<OwnedHandleLease> {
        self.life.before_effect()?;
        if namespace.is_empty() || !self.life.is_alive() {
            return Err("invalid namespace or retired guardian".into());
        }
        Ok(OwnedHandleLease {
            namespace,
            root: self.home.clone(),
            life: self.life.clone(),
        })
    }
}
#[derive(Clone)]
pub struct OwnedHandleLease {
    pub namespace: &'static str,
    pub root: OwnedRoot,
    pub life: Arc<dyn AliveLease>,
}
impl OwnedHandleLease {
    pub fn before_effect(&self) -> Result<()> {
        if !self.life.is_alive() {
            return Err("guardian retired".into());
        }
        self.life.before_effect()
    }
}
