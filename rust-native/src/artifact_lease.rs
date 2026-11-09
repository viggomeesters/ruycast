//! One repository artifact lease, retained by synchronous child consumers.
//! Capability metadata transfers the same open-file-description, never a claim
//! that can replace actual filesystem/owner/identity/flock checks.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, String>;
const PREFIX: &str = "RUYCAST_ARTIFACT_LEASE_";
const FIELDS: [&str; 5] = ["FD", "DEV", "INO", "UID", "ROOT"];
pub struct Lease {
    file: File,
    root: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
}
fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn key(field: &str) -> String {
    format!("{PREFIX}{field}")
}
fn lock_path(root: &Path) -> PathBuf {
    root.join(".build/artifacts.lock")
}
fn regular(meta: &fs::Metadata) -> Result<()> {
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.mode() & 0o7000 != 0
        || meta.uid() != unsafe { libc::getuid() }
    {
        return Err("Artifact lease must be an owned regular single-link file.".into());
    }
    Ok(())
}
fn inheritable(file: &File) -> Result<()> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if flags < 0
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0
    {
        return Err(fail(std::io::Error::last_os_error()));
    }
    Ok(())
}
impl Lease {
    pub fn acquire(repo: &Path) -> Result<Self> {
        let mut environment = BTreeMap::new();
        for field in FIELDS {
            let name = key(field);
            if let Some(value) = std::env::var_os(&name) {
                environment.insert(
                    name,
                    value
                        .into_string()
                        .map_err(|_| "Invalid artifact lease capability encoding.")?,
                );
            }
        }
        Self::acquire_from_environment(repo, &environment)
    }
    /// Explicit complete capability source for owned harnesses. An empty map
    /// obtains a real fresh lock; it never fabricates acquisition success.
    pub fn acquire_from_environment(
        repo: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let root = repo.canonicalize().map_err(fail)?;
        if !root.is_dir() {
            return Err("Artifact lease requires a repository directory.".into());
        }
        let count = FIELDS
            .iter()
            .filter(|field| environment.contains_key(&key(field)))
            .count();
        if count != 0 {
            if count != FIELDS.len() {
                return Err("Incomplete artifact lease capability.".into());
            }
            if Path::new(&environment[&key("ROOT")]) != root {
                return Err("Artifact lease belongs to another repository.".into());
            }
            let fd = environment[&key("FD")]
                .parse::<i32>()
                .map_err(|_| "Invalid artifact lease descriptor.")?;
            if fd < 3 || unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
                return Err("Missing inherited artifact lease descriptor.".into());
            }
            // Duplication retains the same flock open-file-description and
            // does not take ownership of an unrelated caller's descriptor.
            let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
            if duplicate < 0 {
                return Err(fail(std::io::Error::last_os_error()));
            }
            let file = unsafe { File::from_raw_fd(duplicate) };
            let metadata = file.metadata().map_err(fail)?;
            regular(&metadata)?;
            let device = environment[&key("DEV")]
                .parse::<u64>()
                .map_err(|_| "Invalid artifact lease device.")?;
            let inode = environment[&key("INO")]
                .parse::<u64>()
                .map_err(|_| "Invalid artifact lease inode.")?;
            let uid = environment[&key("UID")]
                .parse::<u32>()
                .map_err(|_| "Invalid artifact lease owner.")?;
            let lease = Self {
                file,
                root,
                device,
                inode,
                uid,
            };
            lease.validate()?;
            // Idempotent on an inherited shared description. No second open or
            // blocking acquisition can deadlock against the parent's lease.
            if unsafe { libc::flock(lease.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("Inherited artifact lease is not exclusively available.".into());
            }
            inheritable(&lease.file)?;
            return Ok(lease);
        }
        crate::private_storage::secure_directory(&root.join(".build"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(lock_path(&root))
            .map_err(fail)?;
        let metadata = file.metadata().map_err(fail)?;
        regular(&metadata)?;
        // Tighten an existing legacy owner-created lock without replacing its
        // inode or disturbing any flock already held on that description.
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(fail)?;
        let lease = Self {
            file,
            root,
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
        };
        lease.validate()?;
        loop {
            if unsafe { libc::flock(lease.file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(fail(error));
            }
        }
        lease.validate()?;
        inheritable(&lease.file)?;
        Ok(lease)
    }
    pub fn validate(&self) -> Result<()> {
        let directory = fs::symlink_metadata(self.root.join(".build")).map_err(fail)?;
        if !directory.is_dir()
            || directory.file_type().is_symlink()
            || directory.mode() & 0o7000 != 0
            || directory.uid() != unsafe { libc::getuid() }
            || directory.mode() & 0o777 != 0o700
        {
            return Err("Artifact lease directory changed.".into());
        }
        let path = fs::symlink_metadata(lock_path(&self.root)).map_err(fail)?;
        let opened = self.file.metadata().map_err(fail)?;
        regular(&opened)?;
        regular(&path)?;
        if path.file_type().is_symlink()
            || opened.mode() & 0o777 != 0o600
            || path.mode() & 0o777 != 0o600
            || (opened.dev(), opened.ino(), opened.uid()) != (self.device, self.inode, self.uid)
            || (path.dev(), path.ino(), path.uid()) != (self.device, self.inode, self.uid)
        {
            return Err("Artifact lease identity changed.".into());
        }
        Ok(())
    }
    pub fn environment(&self) -> Result<BTreeMap<String, String>> {
        inheritable(&self.file)?;
        self.validate()?;
        Ok([
            ("FD", self.file.as_raw_fd().to_string()),
            ("DEV", self.device.to_string()),
            ("INO", self.inode.to_string()),
            ("UID", self.uid.to_string()),
            (
                "ROOT",
                self.root
                    .to_str()
                    .ok_or("Artifact repository path must be UTF-8.")?
                    .to_owned(),
            ),
        ]
        .into_iter()
        .map(|(field, value)| (key(field), value))
        .collect())
    }
    pub fn apply_to_child(&self, command: &mut Command) -> Result<()> {
        inheritable(&self.file)?;
        command.envs(self.environment()?);
        Ok(())
    }
}
// File's destructor closes only our descriptor. Never explicitly LOCK_UN:
// children sharing its open-file-description may still consume artifacts.

#[cfg(test)]
#[path = "artifact_lease_tests.rs"]
mod tests;
