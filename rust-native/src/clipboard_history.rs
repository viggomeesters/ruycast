//! Serialized clipboard history authority. Metadata keeps the legacy JSON schema;
//! only PNG assets are owned. File clipboard entries remain references.
use crate::{private_storage, profile::SaveReceipt};
use rustcast_core::{Clip, ClipboardSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Payload {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<String>),
}
#[derive(Clone)]
pub struct Policy {
    pub settings: ClipboardSettings,
    pub paused: bool,
}
#[derive(Clone, Default)]
pub struct CapturePort {
    pub policy: Arc<Mutex<Option<Policy>>>,
    pub busy: Arc<AtomicBool>,
    retention: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}
impl CapturePort {
    pub(crate) fn bind_retention(&self, phase: Arc<AtomicBool>) {
        let mut bound = self.retention.lock().unwrap_or_else(|p|p.into_inner());
        if let Some(previous) = bound.as_ref() { assert!(Arc::ptr_eq(previous, &phase), "capture owner cannot change retention writer"); }
        else { *bound = Some(phase); }
    }
    fn retention_pending(&self) -> bool {
        self.retention.lock().unwrap_or_else(|p|p.into_inner()).as_ref().is_some_and(|p|p.load(Ordering::SeqCst))
    }
    pub fn admission_busy(&self) -> bool { self.busy.load(Ordering::SeqCst) || self.retention_pending() }
    pub fn policy(&self) -> Option<Policy> {
        self.policy
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn publish(&self, settings: &ClipboardSettings, paused: bool) {
        *self.policy.lock().unwrap_or_else(|p| p.into_inner()) = Some(Policy {
            settings: settings.clone(),
            paused,
        });
    }
    pub fn acquire(&self) -> Option<CaptureLease> {
        self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok()?;
        if self.retention_pending() { self.busy.store(false, Ordering::SeqCst); return None; }
        Some(CaptureLease(self.busy.clone()))
    }
}
pub struct CaptureLease(Arc<AtomicBool>);
impl Drop for CaptureLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
pub struct Capture {
    pub payload: Payload,
    pub source: String,
    pub at: String,
    pub _lease: CaptureLease,
}
pub fn now() -> String {
    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn same(a: &Clip, b: &Clip) -> bool {
    a.id == b.id
        && a.text == b.text
        && a.date == b.date
        && a.source == b.source
        && a.content_type == b.content_type
        && a.data_file == b.data_file
        && a.file_urls == b.file_urls
}
pub fn matches(text: &str, query: &str) -> bool {
    use objc2_foundation::NSString;
    objc2::rc::autoreleasepool(|_| {
        NSString::from_str(text).localizedCaseInsensitiveContainsString(&NSString::from_str(query))
    })
}
fn asset_name(path: &str) -> Result<&str> {
    let name = path
        .strip_prefix("clipboard-assets/")
        .ok_or("Unsafe clipboard asset path.")?;
    let id = name
        .strip_suffix(".png")
        .ok_or("Unsafe clipboard asset path.")?;
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    {
        return Err("Unsafe clipboard asset path.".into());
    }
    Ok(name)
}
pub fn validate_files(files: &[String]) -> Result<()> {
    use objc2_foundation::{NSString, NSURL};
    if files.is_empty() || files.len() > 1000 {
        return Err("Invalid clipboard file list.".into());
    }
    for raw in files {
        if raw.len() > 16384 {
            return Err("Clipboard file list exceeds the supported size.".into());
        }
        let u = NSURL::URLWithString(&NSString::from_str(raw))
            .ok_or("Invalid clipboard file reference.")?;
        if !u.isFileURL()
            || u.host()
                .is_some_and(|h| !h.is_empty() && h.to_string() != "localhost")
        {
            return Err("Invalid clipboard file reference.".into());
        }
    }
    Ok(())
}
fn read(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || m.uid() != unsafe { libc::getuid() } || m.nlink() != 1 || m.len() > limit {
        return Err("Unsafe clipboard storage file.".into());
    }
    let mut bytes = vec![];
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Clipboard storage exceeds the supported size.".into());
    }
    Ok(Some(bytes))
}
struct Lock(File);
impl Lock {
    fn acquire(root: &Path) -> Result<Self> {
        // The profile itself belongs to the profile/control owner, never chmod its parent.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(root.join("clipboard.lock"))
            .map_err(|e| e.to_string())?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.nlink() != 1 || m.uid() != unsafe { libc::getuid() } {
            return Err("Unsafe clipboard lock.".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Clipboard history is being updated. Try again.".into());
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
struct AssetDirectory {
    fd: File,
}
#[link(name = "System")]
extern "C" {
    fn acl_init(count: libc::c_int) -> *mut libc::c_void;
    fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    fn acl_set_fd_np(fd: libc::c_int, acl: *mut libc::c_void, kind: libc::c_int) -> libc::c_int;
    fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut libc::c_void;
    fn acl_valid(acl: *mut libc::c_void) -> libc::c_int;
    fn acl_get_entry(
        acl: *mut libc::c_void,
        index: libc::c_int,
        entry: *mut *mut libc::c_void,
    ) -> libc::c_int;
}
fn secure(fd: &File, mode: libc::mode_t) -> Result<()> {
    unsafe {
        if libc::fchmod(fd.as_raw_fd(), mode) != 0 {
            return Err("Clipboard permissions could not be enforced.".into());
        }
        let acl = acl_init(0);
        if acl.is_null() {
            return Err("Clipboard ACL unavailable.".into());
        }
        let result = acl_set_fd_np(fd.as_raw_fd(), acl, 0x100);
        acl_free(acl);
        if result != 0 {
            return Err("Clipboard ACL could not be enforced.".into());
        }
        let acl = acl_get_fd_np(fd.as_raw_fd(), 0x100);
        if !acl.is_null() {
            let mut entry = std::ptr::null_mut();
            let okay = acl_valid(acl) == 0 && acl_get_entry(acl, 0, &mut entry) != 0;
            acl_free(acl);
            if !okay {
                return Err("Clipboard ACL could not be verified.".into());
            }
        } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
            return Err("Clipboard ACL could not be verified.".into());
        }
    }
    Ok(())
}
impl AssetDirectory {
    fn names(&self) -> Result<Vec<String>> {
        unsafe {
            let fd = libc::dup(self.fd.as_raw_fd());
            if fd < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let dir = libc::fdopendir(fd);
            if dir.is_null() {
                libc::close(fd);
                return Err("Clipboard assets could not be enumerated.".into());
            }
            libc::rewinddir(dir);
            let mut names = vec![];
            loop {
                let e = libc::readdir(dir);
                if e.is_null() {
                    break;
                }
                let name = std::ffi::CStr::from_ptr((*e).d_name.as_ptr())
                    .to_string_lossy()
                    .into_owned();
                if name != "." && name != ".." {
                    names.push(name)
                }
            }
            libc::closedir(dir);
            Ok(names)
        }
    }
    fn open(&self, name: &str) -> Result<File> {
        let name = std::ffi::CString::new(name).map_err(|_| "Invalid clipboard asset name.")?;
        let fd = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("Clipboard image is unavailable.".into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.uid() != unsafe { libc::getuid() } || m.nlink() != 1 {
            return Err("Invalid clipboard image asset.".into());
        }
        Ok(file)
    }
    fn remove(&self, name: &str) -> Result<()> {
        let name = std::ffi::CString::new(name).map_err(|_| "Invalid clipboard asset name.")?;
        if unsafe { libc::unlinkat(self.fd.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err("Unused clipboard image could not be removed.".into());
        }
        Ok(())
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let name = std::ffi::CString::new(name).map_err(|_| "Invalid clipboard asset name.")?;
        let raw = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw < 0 {
            return Err("Clipboard image could not be stored.".into());
        }
        let mut file = unsafe { File::from_raw_fd(raw) };
        let result = (|| {
            secure(&file, 0o600)?;
            let m = file.metadata().map_err(|e| e.to_string())?;
            if !m.is_file() || m.uid() != unsafe { libc::getuid() } || m.nlink() != 1 {
                return Err("Invalid clipboard image asset.".into());
            }
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            self.fd.sync_all().map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let info = file.metadata().ok();
            let mut current: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    &mut current,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } == 0
                && info
                    .is_some_and(|m| m.dev() == current.st_dev as u64 && m.ino() == current.st_ino)
            {
                unsafe {
                    libc::unlinkat(self.fd.as_raw_fd(), name.as_ptr(), 0);
                }
            }
        }
        result
    }
}
#[derive(Default)]
pub struct Publication {
    direct: crate::configuration_jobs::Publication,
    configuration: Option<Arc<crate::configuration_jobs::Publication>>,
}
impl Publication {
    fn for_configuration(publication: Arc<crate::configuration_jobs::Publication>) -> Self {
        Self { direct: Default::default(), configuration: Some(publication) }
    }
    pub fn enter(&self, live: impl FnOnce() -> bool) -> Result<()> {
        match &self.configuration {
            Some(configuration) => configuration.enter_retention(live),
            None => self.direct.enter(live),
        }
            .map_err(|_| "Clipboard update cancelled before publication.".into())
    }
    pub fn complete(&self, published: bool) {
        self.complete_with_warning(published, None);
    }
    fn complete_with_warning(&self, published: bool, warning: Option<String>) {
        if let Some(configuration) = &self.configuration { configuration.complete_retention(published, warning); }
        else { self.direct.complete(published); }
    }
    pub fn retire(&self) -> Value {
        if let Some(configuration) = &self.configuration { return configuration.retire(); }
        let state = self.direct.retire();
        json!({"ok":false,"error":"Clipboard history may have changed. List it before trying again.","clipboardWrite":state["configurationWrite"],"clipboardReconciliationRequired":true})
    }
}
pub struct Store {
    publication: Option<Arc<Publication>>,
    root: PathBuf,
    pub items: Vec<Clip>,
    pub paused: bool,
    pub revision: u64,
    pub cleanup_error: Option<String>,
    /// An unreadable pause file never grants permission to capture.
    pub pause_error: Option<String>,
    pub warning: Option<String>,
    metadata_confirmed: bool,
    retention_pending: Option<Arc<AtomicBool>>,
}
impl Store {
    pub fn open(root: &Path, settings: &ClipboardSettings) -> Result<Self> {
        Self::open_with_phase(root, settings, None)
    }
    pub(crate) fn open_with_retention(root: &Path, settings: &ClipboardSettings, pending: Arc<AtomicBool>) -> Result<Self> {
        Self::open_with_phase(root, settings, Some(pending))
    }
    fn open_with_phase(root: &Path, settings: &ClipboardSettings, pending: Option<Arc<AtomicBool>>) -> Result<Self> {
        let mut s = Self {
            root: root.into(),
            items: vec![],
            paused: false,
            revision: 0,
            cleanup_error: None,
            pause_error: None,
            warning: None,
            publication: None,
            metadata_confirmed: false,
            retention_pending: pending,
        };
        s.items = s.load()?.1;
        match read(&root.join("paused.json"), 100).and_then(|bytes| {
            bytes
                .map(|v| {
                    serde_json::from_slice::<bool>(&v)
                        .map_err(|_| String::from("Invalid clipboard pause state."))
                })
                .transpose()
                .map(|value| value.unwrap_or(false))
        }) {
            Ok(paused) => s.paused = paused,
            Err(error) => {
                s.paused = true;
                s.pause_error = Some(format!("Clipboard capture is paused: {error}"));
            }
        }
        s.reconcile_settings(settings, false, || true)?;
        Ok(s)
    }
    /// One ephemeral writer-stage value; never adopted as the owner's model.
    /// The existing lock and current disk base remain the transaction authority.
    pub(crate) fn retain_metadata(
        root: &Path,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
        publication: Option<Arc<crate::configuration_jobs::Publication>>,
    ) -> Result<SaveReceipt> {
        let mut stage = Self {
            root: root.into(), items: vec![], paused: false, revision: 0,
            cleanup_error: None, pause_error: None, warning: None,
            publication: publication.map(|p|Arc::new(Publication::for_configuration(p))),
            metadata_confirmed: false,
            retention_pending: None,
        };
        let _lock = Lock::acquire(root)?;
        stage.update_metadata_locked(live, |base| rustcast_core::history(base, None, settings, &now()), false)
    }
    /// Rebase the same serialized owner's model after accepted configuration.
    /// A real retry write, rather than byte equality, confirms uncertain sync.
    pub(crate) fn reconcile_settings(
        &mut self,
        settings: &ClipboardSettings,
        force_confirmation: bool,
        live: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        let _lock = Lock::acquire(&self.root)?;
        let mut receipt = self.update_metadata_locked(&live, |base| rustcast_core::history(base, None, settings, &now()), force_confirmation)?;
        self.finish_cleanup(&mut receipt, &live);
        Ok(receipt)
    }
    pub fn bind_publication(&mut self, value: Option<Arc<Publication>>) {
        self.publication = value;
    }
    fn publish(
        &mut self,
        path: &Path,
        bytes: &[u8],
        live: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        if !live() {
            return Err("Clipboard update cancelled before publication.".into());
        }
        if let Some(p) = &self.publication {
            p.enter(live)?;
        }
        let result = private_storage::write_outcome(path, bytes, true);
        if let Some(p) = &self.publication {
            let warning = match &result { Ok(private_storage::WriteOutcome::Published { error, .. }) => Some(error.clone()), _=>None };
            p.complete_with_warning(result.is_ok(), warning);
        }
        result.map(SaveReceipt::from_outcome)
    }
    fn load(&self) -> Result<(Option<Vec<u8>>, Vec<Clip>)> {
        let bytes = read(&self.root.join("clipboard.json"), 1_100_000_000)?;
        let items = bytes
            .as_ref()
            .map(|b| {
                serde_json::from_slice(b).map_err(|_| String::from("Invalid clipboard history."))
            })
            .transpose()?
            .unwrap_or_default();
        Ok((bytes, items))
    }
    fn assets(&self) -> Result<AssetDirectory> {
        use std::os::unix::fs::DirBuilderExt;
        let path = self.root.join("clipboard-assets");
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.to_string()),
        }
        let fd = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| "Unsafe clipboard asset directory.")?;
        let m = fd.metadata().map_err(|e| e.to_string())?;
        if !m.is_dir() || m.uid() != unsafe { libc::getuid() } {
            return Err("Unsafe clipboard asset directory.".into());
        }
        secure(&fd, 0o700)?;
        Ok(AssetDirectory { fd })
    }
    pub fn image(&self, path: &str) -> Result<Vec<u8>> {
        let name = asset_name(path)?;
        let root = self.assets()?;
        let mut file = root.open(name)?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        if m.len() == 0 || m.len() > 100_000_000 {
            return Err("Invalid clipboard image asset.".into());
        }
        let mut bytes = vec![];
        (&mut file)
            .take(100_000_001)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 100_000_000 {
            return Err("Invalid clipboard image asset.".into());
        }
        Ok(bytes)
    }
    pub fn payload(&self, id: &str) -> Result<Payload> {
        let c = self
            .items
            .iter()
            .find(|c| c.id == id)
            .ok_or("Clipboard item not found.")?;
        match c.content_type.as_deref().unwrap_or("text") {
            "text" => Ok(Payload::Text(c.text.clone())),
            "files" => {
                let files = c.file_urls.clone().ok_or("Invalid clipboard file list.")?;
                validate_files(&files)?;
                Ok(Payload::Files(files))
            }
            "image" => {
                let data = self.image(
                    c.data_file
                        .as_deref()
                        .ok_or("Clipboard image is missing.")?,
                )?;
                Ok(Payload::Image(crate::clipboard_image::png(
                    &data,
                    100_000_000,
                )?))
            }
            _ => Err("Unsupported clipboard item type.".into()),
        }
    }
    pub fn update(
        &mut self,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
        change: impl FnOnce(&[Clip]) -> Result<Vec<Clip>>,
    ) -> Result<SaveReceipt> {
        let _lock = Lock::acquire(&self.root)?;
        self.update_locked(settings, live, change)
    }
    fn update_locked(
        &mut self,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
        change: impl FnOnce(&[Clip]) -> Result<Vec<Clip>>,
    ) -> Result<SaveReceipt> {
        let mut receipt = self.update_metadata_locked(&live, change, self.warning.is_some())?;
        self.finish_cleanup(&mut receipt, &live);
        let _ = settings;
        Ok(receipt)
    }
    fn update_metadata_locked(
        &mut self,
        live: impl Fn() -> bool,
        change: impl FnOnce(&[Clip]) -> Result<Vec<Clip>>,
        force_confirmation: bool,
    ) -> Result<SaveReceipt> {
        self.metadata_confirmed = false;
        let (before, base) = self.load()?;
        let next = change(&base)?;
        if !live() {
            return Err("Clipboard update cancelled before publication.".into());
        }
        if read(&self.root.join("clipboard.json"), 1_100_000_000)? != before {
            return Err("Clipboard history changed externally. Try again.".into());
        }
        let bytes = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
        let receipt = if force_confirmation || (before.as_deref() != Some(bytes.as_slice())
            && !(before.is_none() && next.is_empty()))
        {
            let receipt = self.publish(&self.root.join("clipboard.json"), &bytes, &live)?;
            self.metadata_confirmed = receipt.durable;
            receipt
        } else {
            SaveReceipt {
                durable: true,
                warning: None,
            }
        };
        if self.items.len() != next.len() || self.items.iter().zip(&next).any(|(a, b)| !same(a, b))
        {
            self.revision = self.revision.wrapping_add(1);
        }
        self.items = next;
        self.warning = receipt.warning.clone();
        Ok(receipt)
    }
    fn finish_cleanup(&mut self, receipt: &mut SaveReceipt, live: &impl Fn() -> bool) {
        if self.retention_pending.as_ref().is_some_and(|p|p.load(Ordering::Acquire)) {
            self.cleanup_error = if !receipt.durable { Some("Clipboard publication durability is unconfirmed; assets retained.".into()) }
                else { match self.pending_prune() {
                    Ok(true) => Some("Clipboard asset cleanup is pending configuration activation.".into()),
                    Ok(false) => None,
                    Err(error) => Some(error),
                }};
            return;
        }
        let result = (|| -> Result<()> {
            if receipt.durable && self.pending_prune()? {
                // Equal bytes alone are not a durability receipt. Only when a
                // real deletion is pending, confirm current metadata under this
                // transaction's existing lock before deleting any owned asset.
                let bytes = serde_json::to_vec_pretty(&self.items).map_err(|e|e.to_string())?;
                if read(&self.root.join("clipboard.json"), 1_100_000_000)?.as_deref() != Some(bytes.as_slice()) {
                    return Err("Clipboard history changed externally. Assets retained.".into());
                }
                if !self.metadata_confirmed {
                    *receipt = self.publish(&self.root.join("clipboard.json"), &bytes, live)?;
                    self.metadata_confirmed = receipt.durable;
                    self.warning = receipt.warning.clone();
                }
                if read(&self.root.join("clipboard.json"), 1_100_000_000)?.as_deref() != Some(bytes.as_slice()) {
                    return Err("Clipboard history changed externally. Assets retained.".into());
                }
            }
            if receipt.durable { self.prune() }
            else { Err("Clipboard publication durability is unconfirmed; assets retained.".into()) }
        })();
        // Published-but-not-durable metadata is authoritative in memory; never roll back
        // or delete assets until publication is confirmed durable.
        self.cleanup_error = result.err();
    }
    fn pending_prune(&self) -> Result<bool> {
        if !self.root.join("clipboard-assets").try_exists().map_err(|e|e.to_string())? { return Ok(false); }
        let dir = self.assets()?;
        let keep = self.items.iter().filter_map(|c|c.data_file.as_deref().and_then(|s|asset_name(s).ok())).collect::<std::collections::HashSet<_>>();
        Ok(dir.names()?.iter().any(|name|asset_name(&format!("clipboard-assets/{name}")).is_ok() && !keep.contains(name.as_str()) && dir.open(name).is_ok()))
    }
    pub fn capture(
        &mut self,
        payload: Payload,
        source: String,
        at: String,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        if !settings.enabled || self.paused || settings.excluded_apps.contains(&source) {
            return Ok(SaveReceipt {
                durable: true,
                warning: None,
            });
        }
        let _lock = Lock::acquire(&self.root)?;
        let mut c = Clip {
            id: objc2_foundation::NSUUID::UUID().UUIDString().to_string(),
            text: String::new(),
            date: at,
            source,
            content_type: None,
            data_file: None,
            file_urls: None,
        };
        match payload {
            Payload::Text(text) => {
                if text.is_empty() || text.len() > settings.max_text_bytes {
                    return Ok(SaveReceipt {
                        durable: true,
                        warning: None,
                    });
                }
                c.text = text
            }
            Payload::Files(files) => {
                validate_files(&files)?;
                c.text = format!(
                    "Files: {}",
                    files
                        .iter()
                        .map(|s| objc2_foundation::NSURL::URLWithString(
                            &objc2_foundation::NSString::from_str(s)
                        )
                        .and_then(|u| u.lastPathComponent())
                        .map(|s| s.to_string())
                        .unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                if c.text.len() > settings.max_text_bytes {
                    return Err("Clipboard file list exceeds the text limit.".into());
                }
                c.content_type = Some("files".into());
                c.file_urls = Some(files)
            }
            Payload::Image(data) => {
                let max = settings.max_asset_bytes.unwrap_or(15_000_000);
                let png = crate::clipboard_image::png(&data, max)?;
                let dir = self.assets()?;
                let total_max = settings.max_total_asset_bytes.unwrap_or(100_000_000);
                let mut total = 0usize;
                for name in dir.names()? {
                    if asset_name(&format!("clipboard-assets/{name}")).is_ok() {
                        if let Ok(file) = dir.open(&name) {
                            total = total
                                .checked_add(
                                    file.metadata().map_err(|e| e.to_string())?.len() as usize
                                )
                                .ok_or("Clipboard image storage is full.")?;
                        }
                    }
                }
                if png.len() > total_max || total > total_max - png.len() {
                    return Err("Clipboard image storage is full.".into());
                }
                let name = format!("{}.png", c.id);
                dir.write(&name, &png)?;
                c.text = "Image".into();
                c.content_type = Some("image".into());
                c.data_file = Some(format!("clipboard-assets/{name}"));
            }
        }
        // Re-read under the history lock: concurrent delete/import is never replaced
        // with a stale capture base. Unpublished assets remain safe orphans for pruning.
        self.update_locked(settings, live, |base| {
            rustcast_core::history(base, Some(c), settings, &now())
        })
    }
    pub fn delete(
        &mut self,
        id: Option<&str>,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        self.update(settings, live, |base| {
            if let Some(id) = id {
                if !base.iter().any(|c| c.id == id) {
                    return Err("Clipboard item not found.".into());
                }
                Ok(base.iter().filter(|c| c.id != id).cloned().collect())
            } else {
                Ok(vec![])
            }
        })
    }
    pub fn pause(&mut self, value: bool, live: impl Fn() -> bool) -> Result<SaveReceipt> {
        let _lock = Lock::acquire(&self.root)?;
        if !live() {
            return Err("Clipboard update cancelled before publication.".into());
        }
        let path = self.root.join("paused.json");
        // Invalid JSON in a safe owned file is recoverable explicitly. Unsafe
        // filesystem objects require the owner to repair the path first.
        let receipt = read(&path, 100)
            .and_then(|_| self.publish(&path, if value { b"true" } else { b"false" }, live));
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                self.paused = true;
                self.pause_error = Some(format!("Clipboard capture is paused: {error}"));
                self.revision = self.revision.wrapping_add(1);
                return Err(error);
            }
        };
        // A published write with an unconfirmed sync is not authority to resume.
        let paused = value || !receipt.durable;
        if self.paused != paused || self.pause_error.is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
        self.paused = paused;
        self.pause_error = if receipt.durable {
            None
        } else {
            Some(String::from("Clipboard capture is paused: pause state storage could not be confirmed. Resume to retry."))
        };
        Ok(receipt)
    }
    pub fn import(
        &mut self,
        raw: &Value,
        settings: &ClipboardSettings,
        live: impl Fn() -> bool,
    ) -> Result<Value> {
        let incoming: Vec<Clip> =
            serde_json::from_value(raw.clone()).map_err(|_| "Invalid imported clipboard items.")?;
        if incoming.len() > 100 {
            return Err("Import at most 100 clipboard items per batch.".into());
        }
        let (mut accepted, mut duplicated, mut excluded) = (0, 0, 0);
        let receipt = self.update(settings, live, |base| {
            let mut next = base.to_vec();
            for mut c in incoming.into_iter().rev() {
                if c.source.starts_with('/') {
                    if let Some(bundle) = objc2_foundation::NSBundle::bundleWithPath(
                        &objc2_foundation::NSString::from_str(&c.source),
                    ) {
                        if let Some(id) = bundle.bundleIdentifier() {
                            c.source = id.to_string();
                        }
                    }
                }
                if !c.id.starts_with("raycast-")
                    || !matches!(
                        c.content_type.as_deref().unwrap_or("text"),
                        "text" | "files"
                    )
                    || c.data_file.is_some()
                {
                    return Err("Invalid imported clipboard item.".into());
                }
                if c.content_type.as_deref() == Some("files") {
                    validate_files(
                        c.file_urls
                            .as_deref()
                            .ok_or("Invalid clipboard file list.")?,
                    )?;
                }
                if settings.excluded_apps.contains(&c.source) {
                    excluded += 1;
                    continue;
                }
                if next
                    .iter()
                    .any(|x| x.id == c.id || (x.text == c.text && x.file_urls == c.file_urls))
                {
                    duplicated += 1;
                    continue;
                }
                next.push(c);
                accepted += 1;
            }
            next.sort_by_key(|c| {
                std::cmp::Reverse(
                    chrono::DateTime::parse_from_rfc3339(&c.date)
                        .map(|d| d.timestamp())
                        .unwrap_or(i64::MIN),
                )
            });
            rustcast_core::history(&next, None, settings, &now())
        })?;
        Ok(
            json!({"ok":true,"result":{"accepted":accepted,"deduplicated":duplicated,"excluded":excluded,"retained":self.items.len()},"durabilityWarning":receipt.warning}),
        )
    }
    pub fn list(&self, query: &str) -> Value {
        json!(self
            .items
            .iter()
            .filter(|c| query.is_empty() || matches(&c.text, query))
            .collect::<Vec<_>>())
    }
    pub fn prune(&self) -> Result<()> {
        if !self
            .root
            .join("clipboard-assets")
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Ok(());
        }
        let dir = self.assets()?;
        let keep = self
            .items
            .iter()
            .filter_map(|c| c.data_file.as_deref().and_then(|s| asset_name(s).ok()))
            .collect::<std::collections::HashSet<_>>();
        for name in dir.names()? {
            if asset_name(&format!("clipboard-assets/{name}")).is_err()
                || keep.contains(name.as_str())
            {
                continue;
            }
            if dir.open(&name).is_ok() {
                dir.remove(&name)?;
            }
        }
        Ok(())
    }
}

pub fn preview_asset(root: &Path, path: &str) -> Result<Vec<u8>> {
    let store = Store {
        root: root.into(),
        items: vec![],
        paused: false,
        revision: 0,
        cleanup_error: None,
        pause_error: None,
        warning: None,
        publication: None,
        metadata_confirmed: false,
        retention_pending: None,
    };
    store.image(path)
}
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub revision: u64,
    pub selected_id: Option<String>,
    pub text: String,
    pub source: String,
    pub date: String,
    pub kind: String,
    pub files: Vec<String>,
    pub paused: bool,
    pub loading: bool,
    pub preview: Option<Arc<crate::clipboard_image::Preview>>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn temp() -> Temp {
        let p = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"))
            .join(format!(
                "clipboard-unit-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        fs::create_dir_all(&p).unwrap();
        Temp(p)
    }
    fn settings() -> ClipboardSettings {
        rustcast_core::validate(&rustcast_core::default_config())
            .unwrap()
            .clipboard
    }
    #[test]
    fn unchanged_history_open_without_pending_asset_deletion_keeps_metadata_inode() {
        let dir = temp();
        let settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        store.capture(Payload::Text("owned unchanged history".into()), "fixture".into(), now(), &settings, ||true).unwrap();
        let file = fs::File::open(dir.0.join("clipboard.json")).unwrap();
        let identity = file.metadata().unwrap().ino();
        let before = fs::read(dir.0.join("clipboard.json")).unwrap();
        drop(store);
        let reopened = Store::open(&dir.0, &settings).unwrap();
        assert_eq!(fs::metadata(dir.0.join("clipboard.json")).unwrap().ino(), identity);
        assert_eq!(fs::read(dir.0.join("clipboard.json")).unwrap(), before);
        assert_eq!(reopened.items[0].text, "owned unchanged history");
        assert!(!dir.0.join("clipboard-assets").exists());
    }
    #[test]
    fn accepted_image_reopens_with_identical_metadata_disk_and_decodable_asset() {
        let dir = temp();
        let settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        let png = vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112,
            80, 248, 15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69,
            78, 68, 174, 66, 96, 130,
        ];
        store
            .capture(
                Payload::Image(png),
                "fixture".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        let before = store.items.clone();
        let disk = fs::read(dir.0.join("clipboard.json")).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&disk).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        let asset = before[0].data_file.as_deref().unwrap();
        let bytes = store.image(asset).unwrap();
        let preview = crate::clipboard_image::preview(&bytes).unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        drop(store);
        let reopened = Store::open(&dir.0, &settings).unwrap();
        assert_eq!(
            serde_json::to_value(&reopened.items).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert_eq!(fs::read(dir.0.join("clipboard.json")).unwrap(), disk);
        assert_eq!(reopened.image(asset).unwrap(), bytes);
        assert_eq!(reopened.items[0].content_type.as_deref(), Some("image"));
    }
    #[test]
    fn capture_cleanup_failure_keeps_published_text_and_reports_error() {
        let dir = temp();
        let settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        assert!(!dir.0.join("clipboard-assets").exists());
        fs::write(
            dir.0.join("clipboard-assets"),
            b"own unavailable directory fixture",
        )
        .unwrap();
        let receipt = store
            .capture(
                Payload::Text("saved despite cleanup failure".into()),
                "fixture".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        assert!(receipt.durable);
        assert_eq!(store.items[0].text, "saved despite cleanup failure");
        assert!(store
            .cleanup_error
            .as_deref()
            .is_some_and(|error| error.contains("asset directory")));
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(dir.0.join("clipboard.json")).unwrap())
                .unwrap(),
            serde_json::to_value(&store.items).unwrap()
        );
        assert_eq!(
            fs::read(dir.0.join("clipboard-assets")).unwrap(),
            b"own unavailable directory fixture"
        );
    }
    #[test]
    fn legacy_schema_dedup_retention_exclusions_and_pause_survive_reopen() {
        let dir = temp();
        let mut settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        store
            .capture(
                Payload::Text("Synthetic Å\nSecond line".into()),
                "fixture.app".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        let original = store.items[0].id.clone();
        store
            .capture(
                Payload::Text("Synthetic Å\nSecond line".into()),
                "fixture.app".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        assert_eq!(store.items.len(), 1);
        assert_ne!(store.items[0].id, original);
        settings.excluded_apps.push("fixture.private".into());
        store
            .capture(
                Payload::Text("ignored".into()),
                "fixture.private".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.list("å\nsecond").as_array().unwrap().len(), 1);
        store.pause(true, || true).unwrap();
        store
            .capture(
                Payload::Text("paused".into()),
                "fixture.app".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        assert_eq!(store.items.len(), 1);
        let mut reopened = Store::open(&dir.0, &settings).unwrap();
        assert!(reopened.paused);
        assert_eq!(reopened.items[0].text, "Synthetic Å\nSecond line");
        reopened.pause(false, || true).unwrap();
        reopened
            .capture(
                Payload::Text("expired".into()),
                "fixture.app".into(),
                "2001-01-01T00:00:00Z".into(),
                &settings,
                || true,
            )
            .unwrap();
        assert_eq!(reopened.items.len(), 2); // newly captured date follows original core contract
        reopened
            .update(
                &settings,
                || true,
                |base| rustcast_core::history(base, None, &settings, &now()),
            )
            .unwrap();
        assert_eq!(reopened.items.len(), 1);
        assert_eq!(
            fs::metadata(dir.0.join("clipboard.json")).unwrap().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn imports_are_atomic_deduplicated_bounded_and_keep_existing_file_references() {
        let dir = temp();
        let settings = settings();
        let mut s = Store::open(&dir.0, &settings).unwrap();
        let values = json!([{"id":"raycast-a","text":"Files: absent.png","source":"fixture","date":now(),"contentType":"files","fileURLs":["file:///synthetic/absent.png"]}]);
        let r = s.import(&values, &settings, || true).unwrap();
        assert_eq!(r["result"]["accepted"], 1);
        assert!(matches!(s.payload("raycast-a").unwrap(), Payload::Files(_)));
        let r = s.import(&values, &settings, || true).unwrap();
        assert_eq!(r["result"]["deduplicated"], 1);
        let before = fs::read(dir.0.join("clipboard.json")).unwrap();
        let malicious = json!([{"id":"raycast-b","text":"valid first","date":now(),"source":"fixture"},{"id":"invalid","text":"bad","date":now(),"source":"fixture"}]);
        assert!(s.import(&malicious, &settings, || true).is_err());
        assert_eq!(fs::read(dir.0.join("clipboard.json")).unwrap(), before);
        assert!(s.delete(Some("raycast-a"), &settings, || false).is_err());
        assert_eq!(s.items.len(), 1);
        assert!(s.delete(Some("missing"), &settings, || true).is_err());
        s.delete(None, &settings, || true).unwrap();
        assert!(s.items.is_empty());
    }
    #[test]
    fn external_edits_are_the_transaction_base_and_links_never_become_assets() {
        use std::os::unix::fs::symlink;
        let dir = temp();
        let settings = settings();
        let mut s = Store::open(&dir.0, &settings).unwrap();
        let external =
            json!([{"id":"external","text":"external edit","date":now(),"source":"fixture"}]);
        fs::write(
            dir.0.join("clipboard.json"),
            serde_json::to_vec(&external).unwrap(),
        )
        .unwrap();
        s.capture(
            Payload::Text("next".into()),
            "fixture".into(),
            now(),
            &settings,
            || true,
        )
        .unwrap();
        assert_eq!(s.items.len(), 2);
        assert!(s.items.iter().any(|c| c.id == "external"));
        let original_mode = fs::metadata(&dir.0).unwrap().mode() & 0o777;
        assert!(s.image("clipboard-assets/../outside.png").is_err());
        assert_eq!(fs::metadata(&dir.0).unwrap().mode() & 0o777, original_mode);
        let assets = s.assets().unwrap();
        let name = "11111111-1111-4111-8111-111111111111.png";
        let outside = dir.0.join("outside");
        fs::write(&outside, b"owned fixture").unwrap();
        symlink(&outside, dir.0.join("clipboard-assets").join(name)).unwrap();
        assert!(s.image(&format!("clipboard-assets/{name}")).is_err());
        s.prune().unwrap();
        assert!(
            fs::symlink_metadata(dir.0.join("clipboard-assets").join(name))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(dir.0.join("clipboard-assets").join(name)).unwrap();
        fs::hard_link(&outside, dir.0.join("clipboard-assets").join(name)).unwrap();
        assert!(s.image(&format!("clipboard-assets/{name}")).is_err());
        s.prune().unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"owned fixture");
        assert!(dir.0.join("clipboard-assets").join(name).exists());
        drop(assets);
        fs::remove_dir_all(dir.0.join("clipboard-assets")).unwrap();
        symlink(&dir.0, dir.0.join("clipboard-assets")).unwrap();
        assert!(s.image(&format!("clipboard-assets/{name}")).is_err());
        assert!(s.prune().is_err());
    }
    #[test]
    fn capture_admission_is_single_flight_and_lease_drop_reopens_it() {
        let port = CapturePort::default();
        let lease = port.acquire().unwrap();
        assert!(port.acquire().is_none());
        drop(lease);
        assert!(port.acquire().is_some());
    }
    #[test]
    fn history_publication_retirement_preserves_storage_truth() {
        let dir = temp();
        let settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        let before = Arc::new(Publication::default());
        let response = before.retire();
        assert_eq!(response["clipboardWrite"]["published"], false);
        assert_eq!(response["clipboardWrite"]["outcomeUncertain"], false);
        store.bind_publication(Some(before));
        assert!(store.pause(true, || true).is_err());
        assert!(!dir.0.join("paused.json").exists());
        let during = Publication::default();
        during.enter(|| true).unwrap();
        let response = during.retire();
        assert_eq!(response["clipboardWrite"]["outcomeUncertain"], true);
        assert!(during.enter(|| true).is_err());
        let published = Arc::new(Publication::default());
        store.bind_publication(Some(published.clone()));
        store.pause(true, || true).unwrap();
        let response = published.retire();
        assert_eq!(response["clipboardWrite"]["published"], true);
        assert_eq!(response["clipboardWrite"]["outcomeUncertain"], false);
        assert_eq!(fs::read(dir.0.join("paused.json")).unwrap(), b"true");
        store.bind_publication(None);
        store.pause(false, || true).unwrap();
        assert!(!store.paused);
    }
    #[test]
    fn image_roundtrip_bounds_acl_and_owned_pruning() {
        let dir = temp();
        let mut settings = settings();
        let mut store = Store::open(&dir.0, &settings).unwrap();
        let bytes = vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112,
            80, 248, 15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69,
            78, 68, 174, 66, 96, 130,
        ];
        let assets = dir.0.join("clipboard-assets");
        fs::create_dir(&assets).unwrap();
        let status=std::process::Command::new("/bin/chmod").args(["+a","everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit"]).arg(&assets).status().unwrap();
        assert!(status.success());
        store
            .capture(
                Payload::Image(bytes.clone()),
                "fixture".into(),
                now(),
                &settings,
                || true,
            )
            .unwrap();
        let clip = store.items[0].clone();
        let path = clip.data_file.as_deref().unwrap();
        let image = store.image(path).unwrap();
        let bitmap=objc2_app_kit::NSBitmapImageRep::imageRepWithData(&objc2_foundation::NSData::with_bytes(&image)).unwrap();
        let original=objc2_app_kit::NSBitmapImageRep::imageRepWithData(&objc2_foundation::NSData::with_bytes(&bytes)).unwrap();
        let actual=bitmap.colorAtX_y(1,1).unwrap();let expected=original.colorAtX_y(1,1).unwrap();
        assert_eq!((bitmap.pixelsWide(),bitmap.pixelsHigh()),(2,2));
        assert!((actual.redComponent()-expected.redComponent()).abs()<0.005);
        assert!((actual.greenComponent()-expected.greenComponent()).abs()<0.005);
        assert!((actual.blueComponent()-expected.blueComponent()).abs()<0.005);
        assert_eq!(actual.alphaComponent(),1.);
        let preview = crate::clipboard_image::preview(&image).unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        assert_eq!(
            fs::metadata(dir.0.join(path)).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::metadata(&assets).unwrap().mode() & 0o777, 0o700);
        settings.max_asset_bytes = Some(1);
        assert!(store
            .capture(
                Payload::Image(bytes.clone()),
                "fixture".into(),
                now(),
                &settings,
                || true
            )
            .is_err());
        settings.max_asset_bytes = None;
        settings.max_total_asset_bytes = Some(image.len());
        assert!(store
            .capture(
                Payload::Image(bytes),
                "fixture".into(),
                now(),
                &settings,
                || true
            )
            .is_err());
        assert!(store
            .capture(
                Payload::Image(b"invalid".to_vec()),
                "fixture".into(),
                now(),
                &settings,
                || true
            )
            .is_err());
        fs::write(assets.join("unrelated.txt"), b"keep").unwrap();
        store.delete(None, &settings, || true).unwrap();
        assert!(!dir.0.join(path).exists());
        assert!(assets.join("unrelated.txt").exists());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub count: usize,
    pub paused: bool,
    pub revision: u64,
    pub settings: Value,
    pub busy: bool,
}
