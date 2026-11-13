//! Profile/configuration authority for the Rust host. Call from its serialized
//! worker, never the AppKit thread. Every mutating operation uses explicit paths.
use crate::private_storage::{self, Publication, WriteOutcome};
use rustcast_core::{validate, Config};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub struct ProfilePaths {
    pub root: PathBuf,
    pub home: PathBuf,
    pub config_override: Option<String>,
    io: Option<BoundIo>,
    #[cfg(test)]
    read_counter: Option<(PathBuf, std::sync::Arc<std::sync::atomic::AtomicU64>)>,
}
#[derive(Clone, Debug)]
struct BoundIo {
    profile: PathBuf,
    binding: crate::profile_io::Binding,
}
#[derive(Serialize, Deserialize)]
struct Selection {
    directory: String,
}

impl ProfilePaths {
    pub fn new(root: PathBuf, home: PathBuf, config_override: Option<String>) -> Self {
        Self {
            root,
            home,
            config_override,
            io: None,
            #[cfg(test)]
            read_counter: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn count_reads_for_test(
        &mut self,
        path: PathBuf,
    ) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        self.read_counter = Some((path, counter.clone()));
        counter
    }
    /// # Safety
    /// The caller must qualify and retain the exact loaded SDK image/table and
    /// its live profile authority as required by Binding::bind_pinned_api.
    /// Header/version matching or an environment marker is insufficient.
    #[cfg(feature = "test-tools")]
    pub unsafe fn with_pinned_io(
        mut self,
        api: &'static crate::profile_io_abi::Api,
    ) -> Result<Self> {
        let binding = unsafe { crate::profile_io::Binding::bind_pinned_api(api, &self.root)? };
        self.io = Some(BoundIo {
            profile: self.root.clone(),
            binding,
        });
        Ok(self)
    }
    pub(crate) fn io(&self) -> Result<Option<&crate::profile_io::Binding>> {
        match &self.io {
            Some(io) if io.profile != self.root => {
                Err("Profile IO authority does not match this profile.".into())
            }
            Some(io) => Ok(Some(&io.binding)),
            None => Ok(None),
        }
    }
    fn read(&self, path: &Path, limit: Option<usize>) -> Result<Vec<u8>> {
        let io = self.io()?;
        #[cfg(test)]
        if let Some((counted_path, counter)) = &self.read_counter {
            if counted_path == path {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        match io {
            Some(io) => io
                .read_optional(path, limit.unwrap_or(32_000_000))?
                .ok_or_else(|| "Configuration is unavailable.".into()),
            None => read_regular_limited(path, limit),
        }
    }
    fn exists(&self, path: &Path) -> Result<bool> {
        match self.io()? {
            Some(io) => Ok(io.metadata(path)?.is_some()),
            None => path.try_exists().map_err(|e| e.to_string()),
        }
    }
    fn secure_root(&self) -> Result<()> {
        match self.io()? {
            Some(io) => io.secure_directory(&self.root),
            None => private_storage::secure_directory(&self.root),
        }
    }
    fn write(&self, path: &Path, bytes: &[u8], replace: bool) -> Result<WriteOutcome> {
        match self.io()? {
            Some(io) => io.write_outcome(path, bytes, replace),
            None => private_storage::write_outcome(path, bytes, replace),
        }
    }
    fn create_directory(&self, path: &Path) -> Result<()> {
        match self.io()? {
            Some(io) => io.create_directory(path, true),
            None => fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(|e| e.to_string()),
        }
    }
    fn same_directory(&self, left: &Path, right: Option<&Path>) -> Result<bool> {
        match self.io()? {
            Some(io) => {
                // Match native canonicalize's best-effort equality check. A
                // missing intermediate directory still reaches guarded MKDIR;
                // no failed attached request falls back to native filesystem IO.
                let left = io.metadata(left).ok().flatten();
                let right = right.and_then(|path| io.metadata(path).ok().flatten());
                Ok(left.zip(right).is_some_and(|(a, b)| {
                    a.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32
                        && b.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32
                        && (a.device, a.inode) == (b.device, b.inode)
                }))
            }
            None => Ok(fs::canonicalize(left).ok().as_ref()
                == right.and_then(|p| fs::canonicalize(p).ok()).as_ref()
                && left.exists()),
        }
    }
    fn owns_copy(&self, path: &Path, bytes: &[u8], identity: Publication) -> Result<bool> {
        match self.io()? {
            Some(io) => {
                let Some(info) = io.metadata(path)? else {
                    return Ok(false);
                };
                let owner = io
                    .metadata(&self.root)?
                    .ok_or("Profile is unavailable.")?
                    .uid;
                Ok(info.mode & libc::S_IFMT as u32 == libc::S_IFREG as u32
                    && info.links == 1
                    && info.uid == owner
                    && (info.device, info.inode) == (identity.device, identity.inode)
                    && self.read(path, None)? == bytes)
            }
            None => Ok(owned_copy(path, bytes, identity)),
        }
    }
    fn remove_copy(&self, path: &Path, bytes: &[u8], identity: Publication) {
        match self.io() {
            Ok(Some(io)) => {
                let _ = io.remove_owned(path, bytes, identity);
            }
            Ok(None) => remove_owned_copy(path, bytes, identity),
            Err(_) => (),
        }
    }
    pub fn absolute(&self, value: &str) -> Result<PathBuf> {
        self.absolute_with_named_home(value, |user| {
            if self.io()?.is_some() {
                return Err("Named configuration homes are unavailable in an owned profile.".into());
            }
            named_home(user)
        })
    }
    fn absolute_with_named_home(
        &self,
        value: &str,
        resolve_named: impl FnOnce(&str) -> Result<PathBuf>,
    ) -> Result<PathBuf> {
        let path = if value == "~" {
            self.home.clone()
        } else if let Some(tail) = value.strip_prefix("~/") {
            self.home.join(tail)
        } else if let Some(named) = value.strip_prefix('~') {
            let (user, tail) = named.split_once('/').unwrap_or((named, ""));
            resolve_named(user)?.join(tail)
        } else {
            PathBuf::from(value)
        };
        if !path.is_absolute() || value.contains(['\n', '\r']) {
            return Err("Choose an absolute configuration folder path.".into());
        }
        if path.components().any(|part| part == Component::ParentDir) {
            return Err("Choose a configuration folder without '..' components.".into());
        }
        Ok(path.components().collect())
    }
    pub fn config_file(&self) -> Result<PathBuf> {
        if let Some(value) = self.config_override.as_deref().filter(|s| !s.is_empty()) {
            return Ok(self.absolute(value)?.join("config.json"));
        }
        let selector = self.root.join("config-location.json");
        if let Some(io) = self.io()? {
            return match io.read_optional(&selector, 1_000_000)? {
                Some(bytes) => {
                    let selection: Selection = serde_json::from_slice(&bytes)
                        .map_err(|_| "Invalid configuration location; fix config-location.json.")?;
                    Ok(self.absolute(&selection.directory)?.join("config.json"))
                }
                None => Ok(self.root.join("config.json")),
            };
        }
        match read_regular_limited(&selector, Some(1_000_000)) {
            Ok(bytes) => {
                let selection: Selection = serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid configuration location; fix config-location.json.")?;
                Ok(self.absolute(&selection.directory)?.join("config.json"))
            }
            Err(error) if selector.try_exists().unwrap_or(true) => Err(error),
            Err(_) => Ok(self.root.join("config.json")),
        }
    }
    pub fn initialize(&self) -> Result<PathBuf> {
        self.secure_root()?;
        let path = self.config_file()?;
        if !self.exists(&path)? {
            if self
                .config_override
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none()
                && self.exists(&self.root.join("config-location.json"))?
            {
                return Err("Selected configuration is unavailable. Restore the folder or fix config-location.json.".into());
            }
            let bytes = encode(&rustcast_core::default_config())?;
            // Create-only publication never replaces a file created by another writer.
            match self.write(&path, &bytes, false)? {
                WriteOutcome::Durable(_) => (),
                WriteOutcome::Published { error, .. } => {
                    return Err(format!(
                        "Configuration created but durability is unconfirmed: {error}"
                    ))
                }
            }
        }
        Ok(path)
    }
}

fn named_home(user: &str) -> Result<PathBuf> {
    // SAFETY: the real resolver follows getpwnam_r's record/buffer contract.
    unsafe {
        named_home_with(user, |name, record, buffer, found| {
            libc::getpwnam_r(
                name.as_ptr(),
                record,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                found,
            )
        })
    }
}
/// # Safety
/// The selected lookup must obey getpwnam_r's pointer lifetime contract: on
/// success with non-null result/home, pw_dir is a NUL-terminated string in the
/// supplied live scratch buffer. Select a complete owned callback in tests.
unsafe fn named_home_with(
    user: &str,
    mut lookup: impl FnMut(
        &std::ffi::CStr,
        &mut libc::passwd,
        &mut [u8],
        &mut *mut libc::passwd,
    ) -> libc::c_int,
) -> Result<PathBuf> {
    use std::ffi::{CStr, CString, OsStr};
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(user).map_err(|_| "Invalid home directory name")?;
    let mut size = 4096;
    loop {
        let mut buffer = vec![0u8; size];
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found = std::ptr::null_mut();
        let status = lookup(&name, &mut record, &mut buffer, &mut found);
        if status == libc::ERANGE && size < 1_048_576 {
            size *= 2;
            continue;
        }
        if status != 0 || found.is_null() || record.pw_dir.is_null() {
            return Err("Choose an existing user's home directory.".into());
        }
        return Ok(PathBuf::from(OsStr::from_bytes(
            unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes(),
        )));
    }
}

/// Nonblocking native/CLI configuration lock, including no-follow and descriptor
/// checks. The root is secured before any file is created inside it.
enum ConfigLock {
    Native(File),
    Attached { _lock: crate::profile_io::Lock },
}
impl ConfigLock {
    fn acquire(paths: &ProfilePaths) -> Result<Self> {
        if let Some(io) = paths.io()? {
            paths.secure_root()?;
            return Ok(Self::Attached {
                _lock: io.lock(&paths.root.join("config.lock"))?,
            });
        }
        let root = &paths.root;
        private_storage::secure_directory(root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("config.lock"))
            .map_err(|e| e.to_string())?;
        let info = file.metadata().map_err(|e| e.to_string())?;
        if !info.is_file() || info.uid() != unsafe { libc::getuid() } {
            return Err("Invalid configuration lock".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Configuration is being saved. Try again.".into());
        }
        Ok(Self::Native(file))
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        if let Self::Native(file) = self {
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    read_regular_limited(path, None)
}
fn read_regular_limited(path: &Path, limit: Option<usize>) -> Result<Vec<u8>> {
    // O_NONBLOCK prevents a FIFO masquerading as a config from hanging a worker.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Expected a regular configuration file".into());
    }
    if limit.is_some_and(|limit| metadata.len() > limit as u64) {
        return Err("Configuration exceeds the supported size.".into());
    }
    let mut bytes = vec![];
    if let Some(limit) = limit {
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > limit {
            return Err("Configuration exceeds the supported size.".into());
        }
    } else {
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}
fn parse(bytes: &[u8]) -> Result<Config> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    validate(&value)
}
fn encode(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|e| e.to_string())
}

#[derive(Debug)]
pub struct SaveReceipt {
    pub durable: bool,
    pub warning: Option<String>,
}
impl SaveReceipt {
    pub(crate) fn from_outcome(outcome: WriteOutcome) -> Self {
        match outcome {
            WriteOutcome::Durable(_) => Self {
                durable: true,
                warning: None,
            },
            WriteOutcome::Published { error, .. } => Self {
                durable: false,
                warning: Some(error),
            },
        }
    }
}

pub struct ConfigStore {
    keyboard_generation: u64,
    publication: Option<std::sync::Arc<crate::configuration_jobs::Publication>>,
    publication_live: Option<crate::execution::Live>,
    published_writes: u64,
    write_in_progress: bool,
    effects: Option<crate::config_effects::Factory>,
    effects_pending: bool,
    pub paths: ProfilePaths,
    pub file: PathBuf,
    bytes: Vec<u8>,
    current: Config,
    pub revision: u64,
    pub last_error: Option<String>,
}
impl ConfigStore {
    pub fn open(paths: ProfilePaths) -> Result<Self> {
        let file = paths.initialize()?;
        Self::read_existing(paths, file)
    }
    /// Offline relocation must never initialize a missing source configuration.
    pub fn open_existing(paths: ProfilePaths) -> Result<Self> {
        let file = paths.config_file()?;
        Self::read_existing(paths, file)
    }
    fn read_existing(paths: ProfilePaths, file: PathBuf) -> Result<Self> {
        let bytes = paths.read(&file, None)?;
        let current = parse(&bytes)?;
        Ok(Self {
            keyboard_generation: 0,
            publication: None,
            publication_live: None,
            published_writes: 0,
            write_in_progress: false,
            effects: None,
            effects_pending: false,
            paths,
            file,
            bytes,
            current,
            revision: 0,
            last_error: None,
        })
    }
    /// Serializes final read-only effect admission with the existing native/
    /// CLI writer and relocation lock. Never waits for a busy writer.
    pub(crate) fn with_locked_current<T>(paths: &ProfilePaths, operation: impl FnOnce(&Config) -> Result<T>) -> Result<T> {
        let _lock = ConfigLock::acquire(paths)?;
        let store = Self::open_existing(paths.clone())?;
        operation(store.current())
    }
    pub fn current(&self) -> &Config {
        &self.current
    }
    /// Immutable last-accepted data copy. No actor copy carries a blocking
    /// platform factory; only the isolated writer installs one.
    pub fn actor_copy(&self) -> Self {
        Self {
            keyboard_generation: self.keyboard_generation,
            publication: None,
            publication_live: None,
            paths: self.paths.clone(),
            file: self.file.clone(),
            bytes: self.bytes.clone(),
            current: self.current.clone(),
            revision: self.revision,
            last_error: self.last_error.clone(),
            effects: None,
            effects_pending: self.effects_pending,
            published_writes: self.published_writes,
            write_in_progress: false,
        }
    }
    pub fn configure_writer_effects(
        &mut self,
        effects: crate::config_effects::Factory,
        force: bool,
        live: crate::execution::Live,
    ) {
        self.publication_live = Some(live);
        self.effects = Some(effects);
        self.effects_pending |= force;
    }
    pub fn needs_effects(&self) -> bool {
        self.effects_pending
    }
    pub fn keyboard_generation(&self) -> u64 {
        self.keyboard_generation
    }
    pub fn require_reconciliation(&mut self) {
        self.effects_pending = true;
    }
    fn check_publication_live(&self) -> Result<()> {
        if self.publication_live.as_ref().is_some_and(|live| !live()) {
            Err("Configuration update was cancelled before publication.".into())
        } else {
            Ok(())
        }
    }
    pub fn publication_state(&self) -> (u64, bool) {
        (self.published_writes, self.write_in_progress)
    }
    pub fn bind_publication(
        &mut self,
        publication: std::sync::Arc<crate::configuration_jobs::Publication>,
    ) {
        self.publication = Some(publication);
    }
    fn enter_publication(&self) -> Result<()> {
        self.check_publication_live()?;
        if let Some(publication) = &self.publication {
            publication.enter(|| self.publication_live.as_ref().is_none_or(|live| live()))?;
        }
        Ok(())
    }
    fn complete_publication(&self, published: bool) {
        if let Some(publication) = &self.publication {
            publication.complete(published);
        }
    }
    pub fn snapshot_is_current(&self, limit: Option<usize>) -> Result<bool> {
        let file = self.paths.config_file()?;
        Ok(file == self.file && self.paths.read(&file, limit)? == self.bytes)
    }
    pub fn snapshot(&self) -> &[u8] {
        &self.bytes
    }
    pub fn attach_effects(&mut self, effects: crate::config_effects::Factory) -> Result<()> {
        self.effects = Some(effects);
        self.effects_pending = true;
        self.reconcile_effects()
    }
    fn reconcile_effects(&mut self) -> Result<()> {
        let mut warning = None;
        let result = (|| {
            if let Some(effects) = &self.effects {
                let mut stage = effects(&self.current)?;
                stage.ready()?;
                self.check_snapshot()?;
                stage.before_adopt(self.publication.clone())?;
                self.check_snapshot()?;
                self.keyboard_generation = 0;
                let receipt = stage.commit()?;
                self.keyboard_generation = receipt.generation.unwrap_or(0);
                warning = receipt.warning;
            }
            self.effects_pending = warning.is_some();
            Ok(())
        })();
        self.last_error = result.as_ref().err().cloned().or(warning);
        result
    }
    pub fn reload(&mut self) -> Result<bool> {
        self.reload_limited(None)
    }
    pub fn reload_limited(&mut self, limit: Option<usize>) -> Result<bool> {
        let mut activation_warning = None;
        let result = (|| {
            let file = self.paths.config_file()?;
            let bytes = self.paths.read(&file, limit)?;
            if file == self.file && bytes == self.bytes {
                if self.effects_pending {
                    self.reconcile_effects()?;
                    activation_warning = self.last_error.clone();
                }
                return Ok(false);
            }
            let next = parse(&bytes)?;
            let changed = file != self.file || bytes != self.bytes;
            if changed {
                let mut stage = self
                    .effects
                    .as_ref()
                    .map(|effect| effect(&next))
                    .transpose()?;
                // Native staging may yield to other processes; never adopt an
                // overwritten candidate after that asynchronous service boundary.
                if stage.is_some()
                    && (self.paths.config_file()? != file
                        || self.paths.read(&file, limit)? != bytes)
                {
                    return Err(
                        "Configuration changed while preparing shortcuts. Reload and try again."
                            .into(),
                    );
                }
                if let Some(stage) = stage.as_mut() {
                    stage.ready()?;
                    if self.paths.config_file()? != file || self.paths.read(&file, limit)? != bytes {
                        return Err("Configuration changed while preparing shortcuts. Reload and try again.".into());
                    }
                    stage.before_adopt(self.publication.clone())?;
                    if self.paths.config_file()? != file || self.paths.read(&file, limit)? != bytes {
                        return Err("Configuration changed while applying settings. Reload and try again.".into());
                    }
                }
                self.file = file;
                self.bytes = bytes;
                self.current = next;
                self.revision = self.revision.wrapping_add(1);
                self.effects_pending = false;
                if let Some(stage) = stage.as_mut() {
                    self.keyboard_generation = 0;
                    match stage.commit() {
                        Ok(receipt) => {
                            self.keyboard_generation = receipt.generation.unwrap_or(0);
                            self.effects_pending = receipt.warning.is_some();
                            activation_warning = receipt.warning;
                        }
                        Err(error) => {
                            self.effects_pending = true;
                            activation_warning = Some(format!(
                                "Configuration loaded; keyboard activation is pending: {error}"
                            ));
                        }
                    }
                }
            }
            Ok(changed)
        })();
        self.last_error = result.as_ref().err().cloned().or(activation_warning);
        result
    }
    fn check_snapshot(&self) -> Result<()> {
        if self.paths.config_file()? != self.file
            || self.paths.read(&self.file, None)? != self.bytes
        {
            return Err("Configuration changed. Reload and try again.".into());
        }
        Ok(())
    }
    /// Stage attached platform effects before publication. Last-good config
    /// and routes survive an unpublished failure; published warnings adopt both.
    pub fn save(&mut self, value: &Value) -> Result<SaveReceipt> {
        let next = validate(value)?;
        let bytes = encode(value)?;
        let _lock = ConfigLock::acquire(&self.paths)?;
        self.check_snapshot()?;
        let mut stage = self
            .effects
            .as_ref()
            .map(|effect| effect(&next))
            .transpose()?;
        if stage.is_some() {
            self.check_snapshot()?;
        }
        if let Some(stage) = stage.as_ref() {
            stage.ready()?;
            self.check_snapshot()?;
        }
        self.enter_publication()?;
        self.write_in_progress = true;
        let outcome = self.paths.write(&self.file, &bytes, true);
        self.complete_publication(outcome.is_ok());
        self.write_in_progress = false;
        let outcome = outcome?;
        self.published_writes = self.published_writes.saturating_add(1);
        let mut receipt = SaveReceipt::from_outcome(outcome);
        if self.paths.config_file()? != self.file || self.paths.read(&self.file, None)? != bytes {
            return Err("Configuration changed after publication. Reload and try again.".into());
        }
        if let Some(stage) = stage.as_mut() {
            stage.before_adopt(self.publication.clone())?;
            if self.paths.config_file()? != self.file || self.paths.read(&self.file, None)? != bytes {
                return Err("Configuration changed after publication. Reload and try again.".into());
            }
        }
        // Rename already happened even if directory fsync failed. Adopt the
        // published state and return its explicit durability warning.
        self.bytes = bytes;
        self.current = next;
        self.revision = self.revision.wrapping_add(1);
        self.last_error = receipt.warning.clone();
        self.effects_pending = false;
        if let Some(stage) = stage.as_mut() {
            self.keyboard_generation = 0;
            let warning = match stage.commit() {
                Ok(receipt) => {
                    self.keyboard_generation = receipt.generation.unwrap_or(0);
                    receipt.warning
                }
                Err(error) => Some(format!(
                    "Configuration saved; keyboard activation is pending: {error}"
                )),
            };
            self.effects_pending = warning.is_some();
            if let Some(warning) = warning {
                receipt.warning = Some(
                    receipt
                        .warning
                        .map(|prior| format!("{prior} {warning}"))
                        .unwrap_or(warning),
                );
                self.last_error = receipt.warning.clone();
            }
        }
        Ok(receipt)
    }
    pub fn relocate(&mut self, directory: &str) -> Result<SaveReceipt> {
        self.relocate_before_selector(directory, || Ok(()))
    }
    fn relocate_before_selector(
        &mut self,
        directory: &str,
        before_selector: impl FnOnce() -> Result<()>,
    ) -> Result<SaveReceipt> {
        if self
            .paths
            .config_override
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_some()
        {
            return Err(
                "RUYCAST_CONFIG_DIR controls this configuration. Change or unset it first.".into(),
            );
        }
        let destination = self.paths.absolute(directory)?;
        let _lock = ConfigLock::acquire(&self.paths)?;
        self.check_snapshot()?;
        if self
            .paths
            .same_directory(&destination, self.file.parent())?
        {
            return Ok(SaveReceipt {
                durable: true,
                warning: None,
            });
        }
        self.paths.create_directory(&destination)?;
        let target = destination.join("config.json");
        let identity = match self.paths.write(&target, &self.bytes, false)? {
            WriteOutcome::Durable(identity) => identity,
            WriteOutcome::Published { identity, error } => {
                self.paths.remove_copy(&target, &self.bytes, identity);
                return Err(format!("Destination durability is unconfirmed: {error}"));
            }
        };
        let result = (|| {
            before_selector()?;
            self.check_snapshot()?;
            if !self.paths.owns_copy(&target, &self.bytes, identity)? {
                return Err("Destination configuration changed during the move.".into());
            }
            let selection = serde_json::to_value(Selection {
                directory: destination.to_string_lossy().into_owned(),
            })
            .map_err(|e| e.to_string())?;
            self.enter_publication()?;
            self.write_in_progress = true;
            let outcome = self.paths.write(
                &self.paths.root.join("config-location.json"),
                &encode(&selection)?,
                true,
            );
            self.complete_publication(outcome.is_ok());
            self.write_in_progress = false;
            if outcome.is_ok() {
                self.published_writes = self.published_writes.saturating_add(1);
            }
            outcome
        })();
        match result {
            Ok(outcome) => {
                if self.paths.config_file()? != target
                    || !self.paths.owns_copy(&target, &self.bytes, identity)?
                {
                    // Selector publication already happened. Keep both original
                    // and copy for recovery; never remove an externally chosen target.
                    return Err("Configuration location or destination changed after publication. Reload and try again.".into());
                }
                let receipt = SaveReceipt::from_outcome(outcome);
                self.file = target;
                self.revision = self.revision.wrapping_add(1);
                self.last_error = receipt.warning.clone();
                Ok(receipt)
            }
            Err(error) => {
                self.paths.remove_copy(&target, &self.bytes, identity);
                Err(error)
            }
        }
    }
}
fn owned_copy(path: &Path, bytes: &[u8], identity: Publication) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::getuid() }
            && m.dev() == identity.device
            && m.ino() == identity.inode
    }) && read_regular(path).is_ok_and(|current| current == bytes)
}
fn remove_owned_copy(path: &Path, bytes: &[u8], identity: Publication) {
    // Cooperating writers use config.lock. Do not delete replacements or edits;
    // this is not an atomic CAS against malicious same-user filesystem races.
    if owned_copy(path, bytes, identity) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn auto_quit_preferences_survive_save_reload_and_disabled_duration() {
        let temp = Temp::new();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        assert!(store.current().auto_quit.is_empty());
        let mut value: Value = serde_json::from_slice(store.snapshot()).unwrap();
        value["autoQuit"] = serde_json::json!({
            "test.synthetic.Editor": {"enabled": true, "minutes": 17},
            "test.synthetic.Player": {"minutes": 30}
        });
        store.save(&value).unwrap();
        let mut reopened = ConfigStore::open(temp.paths()).unwrap();
        assert!(reopened.current().auto_quit["test.synthetic.Editor"].enabled);
        assert!(!reopened.current().auto_quit["test.synthetic.Player"].enabled);
        value["autoQuit"]["test.synthetic.Editor"]["enabled"] = false.into();
        reopened.save(&value).unwrap();
        let mut reopened = ConfigStore::open(temp.paths()).unwrap();
        let preference = &reopened.current().auto_quit["test.synthetic.Editor"];
        assert!(!preference.enabled);
        assert_eq!(preference.minutes, 17);
        assert_eq!(
            reopened.current().auto_quit["test.synthetic.Player"].minutes,
            30
        );
        let saved = reopened.snapshot().to_vec();
        for duration in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("17"),
            Value::Null,
        ] {
            value["autoQuit"]["test.synthetic.Editor"]["minutes"] = duration;
            assert!(reopened.save(&value).is_err());
            assert_eq!(reopened.snapshot(), saved);
            assert_eq!(read_regular(&reopened.file).unwrap(), saved);
        }
        value["autoQuit"]["test.synthetic.Editor"]["minutes"] = 17.into();
        for identity in [
            "",
            "test..Editor",
            "test synthetic.Editor",
            "/Applications/Editor.app",
            "TEST.SYNTHETIC.EDITOR",
        ] {
            value["autoQuit"][identity] = serde_json::json!({"minutes": 10});
            assert!(reopened.save(&value).is_err());
            value["autoQuit"].as_object_mut().unwrap().remove(identity);
        }
        assert_eq!(read_regular(&reopened.file).unwrap(), saved);
        assert!(rustcast_core::validate(&value).is_ok());
    }
    #[test]
    fn post_retention_snapshot_conflicts_refuse_reload_save_and_reconcile_commit() {
        struct ExternalWrite {
            file: PathBuf,
            bytes: Vec<u8>,
            commits: std::sync::Arc<AtomicU64>,
        }
        impl crate::config_effects::Stage for ExternalWrite {
            fn before_adopt(&mut self, _: Option<std::sync::Arc<crate::configuration_jobs::Publication>>) -> Result<()> {
                fs::write(&self.file, &self.bytes).map_err(|e|e.to_string())
            }
            fn commit(&mut self) -> Result<crate::config_effects::Receipt> {
                self.commits.fetch_add(1, Ordering::SeqCst);
                Ok(Default::default())
            }
        }
        for mode in 0..3 {
            for selector in [false, true] {
                let temp = Temp::new();
                let mut store = ConfigStore::open(temp.paths()).unwrap();
                let before = store.snapshot().to_vec();
                let old_limit = store.current().clipboard.limit;
                let mut candidate: Value = serde_json::from_slice(&before).unwrap();
                candidate["clipboard"]["limit"] = serde_json::json!(old_limit + 1);
                let mut external = candidate.clone();
                external["clipboard"]["limit"] = serde_json::json!(old_limit + 2);
                let external_bytes = encode(&external).unwrap();
                let (output, mutation, external_file) = if selector {
                    let alternate = temp.0.join("alternate");
                    fs::create_dir(&alternate).unwrap();
                    let selected = alternate.join("config.json");
                    fs::write(&selected, &external_bytes).unwrap();
                    (store.paths.root.join("config-location.json"), serde_json::to_vec(&serde_json::json!({"directory":alternate})).unwrap(), selected)
                } else { (store.file.clone(), external_bytes.clone(), store.file.clone()) };
                let commits = std::sync::Arc::new(AtomicU64::new(0));
                let calls = commits.clone();
                store.configure_writer_effects(std::sync::Arc::new(move |_| Ok(Box::new(ExternalWrite {
                    file: output.clone(), bytes: mutation.clone(), commits: calls.clone(),
                }))), mode == 2, std::sync::Arc::new(||true));
                let result = match mode {
                    0 => { fs::write(&store.file, encode(&candidate).unwrap()).unwrap(); store.reload().map(|_|()) },
                    1 => store.save(&candidate).map(|_|()),
                    _ => store.reload().map(|_|()),
                };
                assert!(result.is_err(), "post-hook mutation must refuse mode {mode}, selector {selector}");
                assert_eq!(commits.load(Ordering::SeqCst), 0);
                assert_eq!(store.snapshot(), before);
                assert_eq!(store.current().clipboard.limit, old_limit);
                assert_eq!(fs::read(&external_file).unwrap(), external_bytes);
                assert_eq!(store.publication_state().0, u64::from(mode == 1));
                assert!(store.save(&candidate).is_err());
                assert_eq!(fs::read(&external_file).unwrap(), external_bytes);
            }
        }
    }
    #[test]
    fn accepted_keyboard_generation_is_data_bound_and_unknown_activation_invalidates_it() {
        use std::sync::{atomic::AtomicBool, Arc};
        struct Generation {
            next: Arc<AtomicU64>,
            fail: Arc<AtomicBool>,
        }
        impl crate::config_effects::Stage for Generation {
            fn commit(&mut self) -> Result<crate::config_effects::Receipt> {
                if self.fail.load(Ordering::Acquire) {
                    return Err("own lost activation".into());
                }
                Ok(crate::config_effects::Receipt {
                    generation: Some(self.next.fetch_add(1, Ordering::AcqRel) + 1),
                    warning: None,
                })
            }
        }
        let temp = effect_temp();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let count = Arc::new(AtomicU64::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let (c, f) = (count.clone(), fail.clone());
        store
            .attach_effects(Arc::new(move |_| {
                Ok(Box::new(Generation {
                    next: c.clone(),
                    fail: f.clone(),
                }))
            }))
            .unwrap();
        assert_eq!(store.keyboard_generation(), 1);
        assert_eq!(store.actor_copy().keyboard_generation(), 1);
        fail.store(true, Ordering::Release);
        let mut value: Value = serde_json::from_slice(store.snapshot()).unwrap();
        value["theme"] = "light".into();
        assert!(store.save(&value).unwrap().warning.is_some());
        assert_eq!(store.current().theme.as_deref(), Some("light"));
        assert_eq!(store.keyboard_generation(), 0);
        assert!(store.needs_effects());
        fail.store(false, Ordering::Release);
        assert!(!store.reload().unwrap());
        assert_eq!(store.keyboard_generation(), 2);
    }
    #[test]
    fn active_bindings_cleanup_warning_survives_unchanged_reload_until_reconciled() {
        use std::sync::{atomic::AtomicBool, Arc};
        struct Warning(Arc<AtomicBool>);
        impl crate::config_effects::Stage for Warning {
            fn commit(&mut self) -> Result<crate::config_effects::Receipt> {
                Ok(crate::config_effects::Receipt {
                    generation: None,
                    warning: self
                        .0
                        .load(Ordering::Acquire)
                        .then(|| "Keyboard bindings active; cleanup pending".into()),
                })
            }
        }
        let temp = effect_temp();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let f = fail.clone();
        store
            .attach_effects(Arc::new(move |_| Ok(Box::new(Warning(f.clone())))))
            .unwrap();
        fail.store(true, Ordering::Release);
        let mut value = rustcast_core::default_config();
        value["launcherHotkey"] = "cmd+a".into();
        let receipt = store.save(&value).unwrap();
        assert!(receipt
            .warning
            .as_ref()
            .unwrap()
            .contains("bindings active"));
        assert!(store.effects_pending);
        assert!(!store.reload().unwrap());
        assert!(store
            .last_error
            .as_ref()
            .unwrap()
            .contains("bindings active"));
        assert!(store.effects_pending);
        fail.store(false, Ordering::Release);
        assert!(!store.reload().unwrap());
        assert!(store.last_error.is_none());
        assert!(!store.effects_pending);
    }
    fn effect_temp() -> Temp {
        Temp::new()
    }
    struct Effect {
        committed: bool,
        fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl crate::config_effects::Stage for Effect {
        fn commit(&mut self) -> Result<crate::config_effects::Receipt> {
            if self.fail.load(Ordering::Acquire) {
                return Err("Own stopped keyboard stage".into());
            }
            self.committed = true;
            self.events.lock().unwrap().push("commit".into());
            Ok(crate::config_effects::Receipt::default())
        }
    }
    impl Drop for Effect {
        fn drop(&mut self) {
            if !self.committed {
                self.events.lock().unwrap().push("rollback".into());
            }
        }
    }
    #[test]
    fn published_effect_commit_failure_is_pending_warning_and_unchanged_reload_retries() {
        use std::sync::{atomic::AtomicBool, Arc, Mutex};
        let temp = effect_temp();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(vec![]));
        let f = fail.clone();
        let e = events.clone();
        store
            .attach_effects(Arc::new(move |_| {
                Ok(Box::new(Effect {
                    committed: false,
                    fail: f.clone(),
                    events: e.clone(),
                }))
            }))
            .unwrap();
        let mut value = rustcast_core::default_config();
        value["launcherHotkey"] = "cmd+a".into();
        fail.store(true, Ordering::Release);
        let receipt = store.save(&value).unwrap();
        assert!(receipt.warning.unwrap().contains("activation is pending"));
        assert_eq!(store.current().launcher_hotkey, "cmd+a");
        assert!(store.effects_pending);
        assert_eq!(read_regular(&store.file).unwrap(), store.snapshot());
        assert!(store.last_error.is_some());
        fail.store(false, Ordering::Release);
        assert!(!store.reload().unwrap());
        assert!(!store.effects_pending);
        assert!(store.last_error.is_none());
        assert!(events.lock().unwrap().contains(&"rollback".into()));
    }
    #[test]
    fn changed_candidate_during_platform_stage_preserves_external_file_and_old_active_state() {
        use std::sync::{atomic::AtomicBool, Arc, Mutex};
        let temp = effect_temp();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let before = store.snapshot().to_vec();
        let path = store.file.clone();
        let events = Arc::new(Mutex::new(vec![]));
        let e = events.clone();
        let mut external = rustcast_core::default_config();
        external["theme"] = "light".into();
        let external = encode(&external).unwrap();
        let written = external.clone();
        store
            .attach_effects(Arc::new(move |config| {
                if config.launcher_hotkey == "cmd+a" {
                    fs::write(&path, &written).unwrap();
                }
                Ok(Box::new(Effect {
                    committed: false,
                    fail: Arc::new(AtomicBool::new(false)),
                    events: e.clone(),
                }))
            }))
            .unwrap();
        let mut value = rustcast_core::default_config();
        value["launcherHotkey"] = "cmd+a".into();
        assert!(store.save(&value).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(read_regular(&store.file).unwrap(), external);
        assert_eq!(events.lock().unwrap().last().unwrap(), "rollback");
    }
    #[test]
    fn reload_effect_failure_keeps_last_good_and_external_bytes_for_retry() {
        use std::sync::{atomic::AtomicBool, Arc};
        let temp = effect_temp();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let old = store.snapshot().to_vec();
        let reject = Arc::new(AtomicBool::new(false));
        let r = reject.clone();
        store
            .attach_effects(Arc::new(move |_| {
                if r.load(Ordering::Acquire) {
                    Err("Own registration conflict".into())
                } else {
                    Ok(Box::new(Effect {
                        committed: false,
                        fail: Arc::new(AtomicBool::new(false)),
                        events: Arc::default(),
                    }))
                }
            }))
            .unwrap();
        let mut value = rustcast_core::default_config();
        value["launcherHotkey"] = "cmd+a".into();
        let bytes = encode(&value).unwrap();
        fs::write(&store.file, &bytes).unwrap();
        reject.store(true, Ordering::Release);
        assert!(store.reload().is_err());
        assert_eq!(store.snapshot(), old);
        assert_eq!(read_regular(&store.file).unwrap(), bytes);
        reject.store(false, Ordering::Release);
        assert!(store.reload().unwrap());
        assert_eq!(store.current().launcher_hotkey, "cmd+a");
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/profile-fixtures")
                });
            assert!(
                parent.is_absolute(),
                "explicit absolute fixture parent required"
            );
            fs::create_dir_all(&parent).unwrap();
            let path = parent.join(format!(
                "ruycast-native-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn paths(&self) -> ProfilePaths {
            ProfilePaths {
                root: self.0.join("profile"),
                home: self.0.clone(),
                config_override: None,
                io: None,
                read_counter: None,
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn invalid_reload_keeps_last_good_then_valid_change_recovers() {
        let temp = Temp::new();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let original = store.snapshot().to_vec();
        fs::write(&store.file, b"broken").unwrap();
        assert!(store.reload().is_err());
        assert_eq!(store.snapshot(), original);
        assert_eq!(store.revision, 0);
        let mut value = rustcast_core::default_config();
        value["launcherHotkey"] = "cmd+space".into();
        fs::write(&store.file, encode(&value).unwrap()).unwrap();
        assert!(store.reload().unwrap());
        assert_eq!(store.current().launcher_hotkey, "cmd+space");
        assert!(store.last_error.is_none());
    }
    #[test]
    fn save_refuses_external_edits_invalid_input_and_busy_lock() {
        let temp = Temp::new();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let old = store.snapshot().to_vec();
        let lock = ConfigLock::acquire(&store.paths).unwrap();
        assert!(store.save(&rustcast_core::default_config()).is_err());
        drop(lock);
        assert!(store.save(&serde_json::json!({"version":999})).is_err());
        assert_eq!(read_regular(&store.file).unwrap(), old);
        fs::write(&store.file, b"external edit").unwrap();
        assert!(store.save(&rustcast_core::default_config()).is_err());
        assert_eq!(read_regular(&store.file).unwrap(), b"external edit");
        assert_eq!(store.snapshot(), old);
    }
    #[test]
    fn relocate_preserves_original_profile_history_and_never_overwrites_target() {
        let temp = Temp::new();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let old = store.file.clone();
        let bytes = store.snapshot().to_vec();
        fs::write(store.paths.root.join("history.json"), b"local history").unwrap();
        let destination = temp.0.join("portable");
        assert!(
            store
                .relocate(destination.to_str().unwrap())
                .unwrap()
                .durable
        );
        assert_eq!(read_regular(&old).unwrap(), bytes);
        assert_eq!(read_regular(&store.file).unwrap(), bytes);
        assert!(!destination.join("history.json").exists());
        let reopened = ConfigStore::open(temp.paths()).unwrap();
        assert_eq!(reopened.file, store.file);
        let second = temp.0.join("occupied");
        fs::create_dir(&second).unwrap();
        fs::write(second.join("config.json"), b"keep").unwrap();
        assert!(store.relocate(second.to_str().unwrap()).is_err());
        assert_eq!(fs::read(second.join("config.json")).unwrap(), b"keep");
    }
    #[test]
    fn failed_move_cleans_only_own_unchanged_copy_and_retains_external_changes() {
        let temp = Temp::new();
        let mut store = ConfigStore::open(temp.paths()).unwrap();
        let destination = temp.0.join("first");
        let source = store.file.clone();
        assert!(store
            .relocate_before_selector(destination.to_str().unwrap(), || {
                fs::write(&source, b"external source").unwrap();
                Ok(())
            })
            .is_err());
        assert!(!destination.join("config.json").exists());
        assert_eq!(fs::read(&source).unwrap(), b"external source");
        fs::write(&source, store.snapshot()).unwrap();
        let second = temp.0.join("second");
        assert!(store
            .relocate_before_selector(second.to_str().unwrap(), || {
                fs::write(second.join("config.json"), b"external destination").unwrap();
                Err("injected abort".into())
            })
            .is_err());
        assert_eq!(
            fs::read(second.join("config.json")).unwrap(),
            b"external destination"
        );
    }
    #[test]
    fn missing_selected_configuration_does_not_create_fresh_defaults() {
        let temp = Temp::new();
        let paths = temp.paths();
        private_storage::secure_directory(&paths.root).unwrap();
        let target = temp.0.join("missing");
        fs::write(
            paths.root.join("config-location.json"),
            serde_json::to_vec(&Selection {
                directory: target.to_str().unwrap().into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(ConfigStore::open(paths).is_err());
        assert!(!target.exists());
    }
    #[test]
    fn overrides_aliases_and_nonregular_files_obey_path_policy() {
        let temp = Temp::new();
        let mut paths = temp.paths();
        paths.config_override = Some("~/portable".into());
        let mut store = ConfigStore::open(paths).unwrap();
        assert_eq!(store.file, temp.0.join("portable/config.json"));
        assert!(store.relocate(temp.0.to_str().unwrap()).is_err());
        assert!(store.paths.absolute("relative").is_err());
        assert!(store.paths.absolute("/tmp/../bad").is_err());
        let alias = temp.0.join("alias");
        std::os::unix::fs::symlink(&store.paths.root, &alias).unwrap();
        let mut alias_paths = store.paths.clone();
        alias_paths.root = alias;
        assert!(ConfigStore::open(alias_paths).is_ok());
        let fifo = temp.0.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_regular(&fifo).is_err());
    }
    #[test]
    fn named_user_home_expansion_matches_existing_native_path_contract() {
        let paths = ProfilePaths {
            root: "/owned/profile".into(),
            home: "/owned/current".into(),
            config_override: None,
            io: None,
            read_counter: None,
        };
        let mut calls = 0;
        assert_eq!(
            paths
                .absolute_with_named_home("~owned_user/portable", |user| {
                    // SAFETY: only the owned buffer below is returned to the decoder.
                    unsafe {
                        named_home_with(user, |name, record, buffer, found| {
                            calls += 1;
                            assert_eq!(name.to_bytes(), b"owned_user");
                            owned_home_record(record, buffer, found, b"/owned/fixture-user")
                        })
                    }
                })
                .unwrap(),
            Path::new("/owned/fixture-user/portable")
        );
        assert_eq!(calls, 1);
        assert_eq!(
            paths
                .absolute_with_named_home("~owned_user", |user| unsafe {
                    named_home_with(user, |_, r, b, f| {
                        owned_home_record(r, b, f, b"/owned/fixture-user")
                    })
                })
                .unwrap(),
            Path::new("/owned/fixture-user")
        );
        assert!(paths
            .absolute_with_named_home("~unknown/portable", |user| unsafe {
                named_home_with(user, |name, _, _, _| {
                    assert_eq!(name.to_bytes(), b"unknown");
                    0
                })
            })
            .is_err());
        for value in ["~", "~/portable", "/owned/./portable"] {
            assert!(paths
                .absolute_with_named_home(value, |_| panic!("unexpected lookup"))
                .is_ok());
        }
        for value in [
            "~owned/../escape",
            "~owned/line\nbreak",
            "~owned/line\rbreak",
        ] {
            assert!(paths
                .absolute_with_named_home(value, |user| unsafe {
                    named_home_with(user, |_, r, b, f| {
                        owned_home_record(r, b, f, b"/owned/user")
                    })
                })
                .is_err());
        }
        for home in [b"".as_slice(), b"relative-home", b"/owned/../escape"] {
            assert!(paths
                .absolute_with_named_home("~owned/portable", |user| unsafe {
                    named_home_with(user, |_, r, b, f| owned_home_record(r, b, f, home))
                })
                .is_err());
        }
        use std::os::unix::ffi::OsStrExt;
        let home = unsafe {
            named_home_with("owned", |_, r, b, f| {
                owned_home_record(r, b, f, b"/owned/\xff")
            })
        }
        .unwrap();
        assert_eq!(home.as_os_str().as_bytes(), b"/owned/\xff");
    }
    fn owned_home_record(
        record: &mut libc::passwd,
        buffer: &mut [u8],
        found: &mut *mut libc::passwd,
        home: &[u8],
    ) -> libc::c_int {
        assert!(home.len() < buffer.len());
        buffer[..home.len()].copy_from_slice(home);
        buffer[home.len()] = 0;
        record.pw_dir = buffer.as_mut_ptr().cast();
        *found = record;
        0
    }
    #[test]
    fn owned_named_home_retries_erange_with_the_existing_bound() {
        let mut sizes = vec![];
        let home = unsafe {
            named_home_with("owned", |_, r, b, f| {
                sizes.push(b.len());
                if sizes.len() < 3 {
                    libc::ERANGE
                } else {
                    owned_home_record(r, b, f, b"/owned/retry")
                }
            })
        }
        .unwrap();
        assert_eq!(home, Path::new("/owned/retry"));
        assert_eq!(sizes, vec![4096, 8192, 16384]);
        sizes.clear();
        assert!(unsafe {
            named_home_with("owned", |_, _, b, _| {
                sizes.push(b.len());
                libc::ERANGE
            })
        }
        .is_err());
        assert_eq!(
            sizes,
            (12..=20).map(|power| 1usize << power).collect::<Vec<_>>()
        );
    }
    #[test]
    fn owned_named_home_rejects_errors_null_fields_and_invalid_names() {
        for mode in 0..3 {
            let mut calls = 0;
            assert!(unsafe {
                named_home_with("owned", |_, r, b, f| {
                    calls += 1;
                    match mode {
                        0 => {
                            owned_home_record(r, b, f, b"/owned/ignored");
                            libc::EIO
                        }
                        1 => {
                            owned_home_record(r, b, f, b"/owned/ignored");
                            *f = std::ptr::null_mut();
                            0
                        }
                        _ => {
                            *f = r;
                            0
                        }
                    }
                })
            }
            .is_err());
            assert_eq!(calls, 1);
        }
        assert!(unsafe {
            named_home_with("invalid\0name", |_, _, _, _| {
                panic!("invalid name reached lookup")
            })
        }
        .is_err());
    }
}
