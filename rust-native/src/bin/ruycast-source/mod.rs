use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "policy.rs"]
mod allowlist;
mod archive;
#[cfg(test)]
mod tests;
type Result<T> = std::result::Result<T, String>;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;
const MAX_ARTIFACT: u64 = MAX_TOTAL as u64 * 2;
const SCHEMA: &str = "ruycast.public-source.v2";
const VERSION_SOURCE: &str = "rust/src/product_version.rs";
const VERSION_BYTES: &[u8] = include_bytes!("../../../../rust/src/product_version.rs");
const DEFAULT_BYTES: &[u8] = include_bytes!("../../../../config/default.json");
const TOOL_SOURCES: &[(&str, &[u8])] = &[
    (
        "rust-native/src/bin/ruycast-source.rs",
        include_bytes!("../ruycast-source.rs"),
    ),
    (
        "rust-native/src/bin/ruycast-source/mod.rs",
        include_bytes!("mod.rs"),
    ),
    (
        "rust-native/src/bin/ruycast-source/archive.rs",
        include_bytes!("archive.rs"),
    ),
    (
        "rust-native/src/bin/ruycast-source/policy.rs",
        include_bytes!("policy.rs"),
    ),
];
const USAGE:&str="ruycast-source export --repo ABSOLUTE_ROOT --out ABSOLUTE_NEW_DIRECTORY\nruycast-source verify --source ABSOLUTE_EXPORT [--repo ABSOLUTE_ROOT]\nruycast-source list --repo ABSOLUTE_ROOT\nCreates only local source, .tar.gz and .sha256; never publishes. Verify a clean export before adding build output.";
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn safe_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['\0', '\n', '\r', '\\']) || Path::new(name).is_absolute() {
        return Err("Invalid source path".into());
    }
    for p in name.split('/') {
        if p.is_empty()
            || p == "."
            || p == ".."
            || [
                ".go",
                ".git",
                ".go-workflow",
                "target",
                ".build",
                ".dist",
                "__pycache__",
                "profiles",
                "private",
                "adapters",
                "workflows",
            ]
            .contains(&p)
        {
            return Err(format!("Forbidden source path: {name}"));
        }
    }
    Ok(())
}
fn policy() -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for line in allowlist::PATHS.lines() {
        safe_name(line)?;
        if !result.insert(line.to_string()) {
            return Err("Duplicate allowlist path".into());
        }
    }
    if result.is_empty() {
        return Err("Empty export policy".into());
    }
    Ok(result)
}
fn absolute(p: &Path) -> Result<()> {
    if !p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err("Use an absolute path without dot or parent components".into());
    }
    Ok(())
}
struct Dir(File);
impl Dir {
    fn identity(&self) -> Result<(u64, u64)> {
        let m = self.0.metadata().map_err(err)?;
        Ok((m.dev(), m.ino()))
    }
    fn open(p: &Path) -> Result<Self> {
        absolute(p)?;
        let mut dir = Self(File::open("/").map_err(err)?);
        for c in p.components() {
            if let Component::Normal(name) = c {
                dir = dir.child(name.as_bytes())?;
            }
        }
        Ok(dir)
    }
    fn child(&self, name: &[u8]) -> Result<Self> {
        let n = CString::new(name).map_err(err)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("Missing, linked or unreadable source directory".into());
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }
    fn file(&self, path: &str) -> Result<File> {
        safe_name(path)?;
        let mut components = path.split('/').collect::<Vec<_>>();
        let name = CString::new(components.pop().unwrap()).map_err(err)?;
        let mut dir = Self(self.0.try_clone().map_err(err)?);
        for p in components {
            dir = dir.child(p.as_bytes())?;
        }
        let fd = unsafe {
            libc::openat(
                dir.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "Missing, linked or unreadable allowlisted source: {path}"
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mode: u32,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn stamp(f: &File) -> Result<Stamp> {
    let m = f.metadata().map_err(err)?;
    if !m.is_file() || m.nlink() != 1 || m.len() > MAX_ARTIFACT || m.mode() & 0o7000 != 0 {
        return Err(
            "Source must be a bounded regular, single-link file without special modes".into(),
        );
    }
    Ok(Stamp {
        dev: m.dev(),
        ino: m.ino(),
        len: m.len(),
        mode: m.mode(),
        mtime: m.mtime(),
        mtime_ns: m.mtime_nsec(),
        ctime: m.ctime(),
        ctime_ns: m.ctime_nsec(),
    })
}
fn regular(p: &Path) -> Result<(File, Stamp)> {
    let parent = Dir::open(p.parent().ok_or("Missing parent")?)?;
    let f = parent.file(
        p.file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename")?,
    )?;
    let s = stamp(&f)?;
    Ok((f, s))
}
fn bytes(f: File) -> Result<(Vec<u8>, Stamp)> {
    let before = stamp(&f)?;
    let mut value = vec![];
    (&f).take(MAX_ARTIFACT + 1)
        .read_to_end(&mut value)
        .map_err(err)?;
    if stamp(&f)? != before || value.len() as u64 != before.len {
        return Err("Source changed while being read".into());
    }
    Ok((value, before))
}
fn private_bytes(bytes: &[u8]) -> Result<()> {
    let begin = b"-----BEGIN ";
    let boundary = b"-----";
    // Inspect complete labels on one line, independent of algorithm names.
    // Separate source literals and incomplete headers are not key envelopes.
    for line in bytes.split(|b| matches!(b, b'\n' | b'\r')) {
        for (at, candidate) in line.windows(begin.len()).enumerate() {
            if candidate != begin {
                continue;
            }
            let rest = &line[at + begin.len()..];
            let Some(end) = rest.windows(boundary.len()).position(|b| b == boundary) else {
                continue;
            };
            let mut private = false;
            for word in rest[..end]
                .split(u8::is_ascii_whitespace)
                .filter(|w| !w.is_empty())
            {
                if private && word.eq_ignore_ascii_case(b"KEY") {
                    return Err("Private signing/key material in an allowlisted file".into());
                }
                private = word.eq_ignore_ascii_case(b"PRIVATE");
            }
        }
    }
    Ok(())
}
#[derive(Clone, PartialEq, Eq)]
struct Input {
    bytes: Vec<u8>,
    stamp: Stamp,
    mode: u32,
}
type Snapshot = BTreeMap<String, Input>;
type ArchiveFiles = (BTreeMap<String, Vec<u8>>, BTreeMap<String, u32>);
fn source_coverage(dir: &Dir, prefix: &str, allowed: &BTreeSet<String>) -> Result<()> {
    let fd = unsafe { libc::dup(dir.0.as_raw_fd()) };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        unsafe { libc::close(fd) };
        return Err("Could not enumerate source descriptors".into());
    }
    struct Entries(*mut libc::DIR);
    impl Drop for Entries {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let entries = Entries(raw);
    loop {
        unsafe {
            *libc::__error() = 0;
        }
        let entry = unsafe { libc::readdir(entries.0) };
        if entry.is_null() {
            if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                return Err("Source directory enumeration failed".into());
            }
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }
            .to_str()
            .map_err(err)?;
        if [".", ".."].contains(&name) {
            continue;
        }
        let path = format!("{prefix}/{name}");
        safe_name(&path)?;
        let c = CString::new(name).map_err(err)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                dir.0.as_raw_fd(),
                c.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err("Source directory changed during enumeration".into());
        }
        match unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                if !allowed.iter().any(|p| p.starts_with(&format!("{path}/"))) {
                    return Err(format!("Unreviewed source directory: {path}"));
                }
                source_coverage(&dir.child(name.as_bytes())?, &path, allowed)?;
            }
            libc::S_IFREG if allowed.contains(&path) => {}
            _ => {
                return Err(format!(
                    "Unreviewed, linked or special product source: {path}"
                ))
            }
        }
    }
    Ok(())
}
fn snapshot(root: &Dir) -> Result<Snapshot> {
    let mut out = BTreeMap::new();
    let mut total = 0;
    let allowed = policy()?;
    for name in ["rust", "rust-native"] {
        source_coverage(
            &root.child(name.as_bytes())?.child(b"src")?,
            &format!("{name}/src"),
            &allowed,
        )?;
    }
    for path in allowed {
        let (data, stamp) = bytes(root.file(&path)?)?;
        if stamp.len > MAX_FILE {
            return Err(format!("Allowlisted file exceeds its size bound: {path}"));
        }
        private_bytes(&data)?;
        total += data.len();
        if total > MAX_TOTAL {
            return Err("Source export exceeds its bounded size".into());
        }
        let mode = if stamp.mode & 0o111 != 0 {
            0o755
        } else {
            0o644
        };
        out.insert(
            path,
            Input {
                bytes: data,
                stamp,
                mode,
            },
        );
    }
    if out.get(VERSION_SOURCE).map(|i| i.bytes.as_slice()) != Some(VERSION_BYTES) {
        return Err(
            "Rust product version source differs from the compiled source tool; rebuild it".into(),
        );
    }
    if out.get("config/default.json").map(|i| i.bytes.as_slice()) != Some(DEFAULT_BYTES) {
        return Err("Default configuration differs from the compiled public default; rebuild the source tool".into());
    }
    for (path, expected) in TOOL_SOURCES {
        if out.get(*path).map(|i| i.bytes.as_slice()) != Some(*expected) {
            return Err(format!(
                "Source tool or allowlist differs from compiled bytes; rebuild it: {path}"
            ));
        }
    }
    Ok(out)
}
fn ensure_parent(path: &Path) -> Result<()> {
    absolute(path)?;
    let mut dir = Dir::open(Path::new("/"))?;
    for component in path.components() {
        if let Component::Normal(name) = component {
            if dir.child(name.as_bytes()).is_err() {
                let name = CString::new(name.as_bytes()).map_err(err)?;
                if unsafe { libc::mkdirat(dir.0.as_raw_fd(), name.as_ptr(), 0o755) } != 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err("Could not create source staging parent".into());
                }
            }
            dir = dir.child(name.as_bytes())?;
        }
    }
    Ok(())
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    version: String,
    version_authority: String,
    files: BTreeMap<String, String>,
    modes: BTreeMap<String, u32>,
    roles: BTreeMap<String, String>,
    historical_references: Vec<String>,
    publication: String,
}
fn role(p: &str) -> String {
    if p.starts_with("Sources/")
        || p == "Package.swift"
        || p.starts_with("Tests/LauncherCoreTests/")
        || p.starts_with("bench/reference/0.8.26/")
    {
        "historical Swift reference/test oracle; not a product host"
    } else if p.starts_with("rust/src/") || p.starts_with("rust-native/src/") {
        "Rust product or Rust tool"
    } else {
        "product documentation, assets, default configuration or test support"
    }
    .into()
}
fn manifest(inputs: &Snapshot) -> Manifest {
    Manifest {
        schema: SCHEMA.into(),
        version: rustcast_core::product_version::CURRENT.into(),
        version_authority: VERSION_SOURCE.into(),
        files: inputs
            .iter()
            .map(|(p, i)| (p.clone(), digest(&i.bytes)))
            .collect(),
        modes: inputs.iter().map(|(p, i)| (p.clone(), i.mode)).collect(),
        roles: inputs.keys().map(|p| (p.clone(), role(p))).collect(),
        historical_references: vec![
            "Package.swift".into(),
            "Sources/".into(),
            "Tests/LauncherCoreTests/".into(),
            "bench/reference/0.8.26/".into(),
        ],
        publication: "export-only; not published, installed or production-accepted".into(),
    }
}
fn encode(m: &Manifest) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(m).map_err(err)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn artifact_files(inputs: &Snapshot) -> Result<ArchiveFiles> {
    let mut files = inputs
        .iter()
        .map(|(p, i)| (p.clone(), i.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut modes = inputs
        .iter()
        .map(|(p, i)| (p.clone(), i.mode))
        .collect::<BTreeMap<_, _>>();
    files.insert("PUBLIC-SOURCE.json".into(), encode(&manifest(inputs))?);
    modes.insert("PUBLIC-SOURCE.json".into(), 0o644);
    Ok((files, modes))
}
fn write_new(p: &Path, b: &[u8], mode: u32) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(p)
        .map_err(err)?;
    f.write_all(b).map_err(err)?;
    f.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(err)?;
    f.sync_all().map_err(err)
}
struct Stage {
    path: PathBuf,
    identity: (u64, u64),
}
impl Stage {
    fn new(parent: &Path) -> Result<Self> {
        Dir::open(parent)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let path = parent.join(format!(
                ".ruycast-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let m = fs::symlink_metadata(&path).map_err(err)?;
                    return Ok(Self {
                        path,
                        identity: (m.dev(), m.ino()),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err(e)),
            }
        }
        Err("Could not reserve source staging directory".into())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.is_dir() && (m.dev(), m.ino()) == self.identity)
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
fn siblings(out: &Path) -> Result<(PathBuf, PathBuf)> {
    absolute(out)?;
    let name = out
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid destination")?;
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Use an ASCII destination name without spaces or control characters".into());
    }
    let parent = out.parent().ok_or("Missing destination parent")?;
    Ok((
        parent.join(format!("{name}.tar.gz")),
        parent.join(format!("{name}.tar.gz.sha256")),
    ))
}
fn absent(p: &Path) -> Result<()> {
    match fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(format!(
            "Destination exists or is inaccessible: {}",
            p.display()
        )),
    }
}
fn rename_new(from: &Path, to: &Path) -> Result<()> {
    let a = CString::new(from.as_os_str().as_bytes()).map_err(err)?;
    let b = CString::new(to.as_os_str().as_bytes()).map_err(err)?;
    let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) };
    if result != 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn rollback_file(path: &Path, owned: &Stamp) {
    if let Ok((_, current)) = regular(path) {
        if (current.dev, current.ino) == (owned.dev, owned.ino)
            && current.len == owned.len
            && current.mtime == owned.mtime
            && current.mtime_ns == owned.mtime_ns
        {
            let _ = fs::remove_file(path);
        }
    }
}
pub(super) fn prepare(root: &Path, out: &Path, hook: impl FnOnce()) -> Result<Value> {
    let source = Dir::open(root)?;
    let (archive, checksum) = siblings(out)?;
    let parent = out.parent().unwrap();
    if out == root || root.starts_with(out) {
        return Err("Export may not replace its source or ancestor".into());
    }
    for p in [out, &archive, &checksum] {
        absent(p)?;
    }
    let inputs = snapshot(&source)?;
    ensure_parent(parent)?;
    let stage = Stage::new(parent)?;
    let tree = stage.path.join("tree");
    fs::create_dir(&tree).map_err(err)?;
    let (files, modes) = artifact_files(&inputs)?;
    for (p, b) in &files {
        let path = tree.join(p);
        fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        for dir in path
            .parent()
            .unwrap()
            .ancestors()
            .take_while(|p| p.starts_with(&tree))
        {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).map_err(err)?;
        }
        write_new(&path, b, modes[p])?;
    }
    let tar = archive::tar(&files, &modes)?;
    let tar_path = stage.path.join("source.tar");
    write_new(&tar_path, &tar, 0o600)?;
    let compressed = archive::gzip(&tar_path, false, tar.len() + tar.len() / 16 + 4096)?;
    let archive_path = stage.path.join("source.tar.gz");
    write_new(&archive_path, &compressed, 0o644)?;
    let checksum_bytes = format!(
        "{}  {}\n",
        digest(&compressed),
        archive.file_name().unwrap().to_str().unwrap()
    )
    .into_bytes();
    let checksum_path = stage.path.join("source.sha256");
    write_new(&checksum_path, &checksum_bytes, 0o644)?;
    hook();
    if snapshot(&source)? != inputs {
        return Err("Allowlisted source changed during export; no artifact was committed".into());
    }
    if Dir::open(root)?.identity()? != source.identity()? {
        return Err("Source root was replaced during export".into());
    }
    verify_tree(&tree, &inputs)?;
    for p in [out, &archive, &checksum] {
        absent(p)?;
    }
    let archive_identity = regular(&archive_path)?.1;
    let checksum_identity = regular(&checksum_path)?.1;
    rename_new(&archive_path, &archive)?;
    if let Err(e) = rename_new(&checksum_path, &checksum) {
        rollback_file(&archive, &archive_identity);
        return Err(e);
    }
    if let Err(e) = rename_new(&tree, out) {
        rollback_file(&archive, &archive_identity);
        rollback_file(&checksum, &checksum_identity);
        return Err(e);
    }
    Ok(
        json!({"ok":true,"source":out,"files":inputs.len(),"archive":archive,"archiveSHA256":digest(&compressed),"checksum":checksum,"version":rustcast_core::product_version::CURRENT,"schema":SCHEMA,"published":false}),
    )
}
fn all_files(
    root: &Path,
    p: &Path,
    expected: &BTreeSet<String>,
    result: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(p).map_err(err)? {
        let entry = entry.map_err(err)?;
        let path = entry.path();
        let m = fs::symlink_metadata(&path).map_err(err)?;
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .ok_or("Non-UTF8 public path")?;
        if m.file_type().is_symlink() {
            return Err("Linked public export entry".into());
        }
        if m.is_dir() {
            if m.mode() & 0o777 != 0o755
                || !expected
                    .iter()
                    .any(|p| p.starts_with(&format!("{relative}/")))
            {
                return Err("Unallowlisted export directory or changed directory mode".into());
            }
            all_files(root, &path, expected, result)?;
        } else if m.is_file() {
            if !expected.contains(relative) {
                return Err("Unallowlisted public file".into());
            }
            result.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .ok_or("Non-UTF8 public path")?
                    .into(),
            );
        } else {
            return Err("Special public export entry".into());
        }
    }
    Ok(())
}
fn verify_tree(root: &Path, expected: &Snapshot) -> Result<()> {
    let mut actual = BTreeSet::new();
    let mut names = policy()?;
    names.insert("PUBLIC-SOURCE.json".into());
    if fs::symlink_metadata(root).map_err(err)?.mode() & 0o777 != 0o755 {
        return Err("Export directory mode changed".into());
    }
    all_files(root, root, &names, &mut actual)?;
    if actual != names {
        return Err("Export contains missing or unallowlisted files".into());
    }
    let dir = Dir::open(root)?;
    for (p, i) in expected {
        let (b, s) = bytes(dir.file(p)?)?;
        if b != i.bytes || s.mode & 0o777 != i.mode {
            return Err(format!("Export content or mode changed: {p}"));
        }
    }
    let b = bytes(dir.file("PUBLIC-SOURCE.json")?)?.0;
    if b != encode(&manifest(expected))? {
        return Err("Public source manifest differs from source bindings".into());
    }
    Ok(())
}
fn verify(root: &Path, original: Option<&Path>) -> Result<Value> {
    let dir = Dir::open(root)?;
    let (mb, ms) = bytes(dir.file("PUBLIC-SOURCE.json")?)?;
    if ms.mode & 0o777 != 0o644 {
        return Err("Manifest mode changed".into());
    }
    let m: Manifest = serde_json::from_slice(&mb).map_err(err)?;
    let inputs = snapshot(&dir)?;
    if encode(&m)? != mb || encode(&manifest(&inputs))? != mb {
        return Err("Manifest is noncanonical or its source bindings changed".into());
    }
    verify_tree(root, &inputs)?;
    if let Some(original) = original {
        let current = snapshot(&Dir::open(original)?)?;
        if encode(&manifest(&current))? != mb {
            return Err("Original allowlisted source differs from export".into());
        }
    }
    let (archive, checksum) = siblings(root)?;
    let (ab, archive_stamp) = bytes(regular(&archive)?.0)?;
    let (cb, checksum_stamp) = bytes(regular(&checksum)?.0)?;
    if archive_stamp.mode & 0o777 != 0o644 || checksum_stamp.mode & 0o777 != 0o644 {
        return Err("Archive or checksum mode changed".into());
    }
    if cb
        != format!(
            "{}  {}\n",
            digest(&ab),
            archive.file_name().unwrap().to_str().unwrap()
        )
        .as_bytes()
    {
        return Err("Archive checksum changed".into());
    }
    let (files, modes) = artifact_files(&inputs)?;
    let tar = archive::tar(&files, &modes)?;
    if archive::gzip(&archive, true, tar.len())? != tar {
        return Err("Archive content, headers or modes differ from export".into());
    }
    Ok(
        json!({"ok":true,"files":inputs.len(),"version":m.version,"schema":m.schema,"archiveSHA256":digest(&ab),"published":false}),
    )
}
pub fn cli(args: Vec<String>) -> Result<Value> {
    if args.is_empty() || args == ["--help"] || args == ["help"] {
        return Ok(
            json!({"ok":true,"help":USAGE,"schema":SCHEMA,"version":rustcast_core::product_version::CURRENT}),
        );
    }
    let op = &args[0];
    let mut options = BTreeMap::new();
    for pair in args[1..].chunks(2) {
        if pair.len() != 2
            || !pair[0].starts_with("--")
            || options
                .insert(pair[0].as_str(), PathBuf::from(&pair[1]))
                .is_some()
        {
            return Err(USAGE.into());
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .map(PathBuf::as_path)
            .ok_or_else(|| format!("Missing {key}\n{USAGE}"))
    };
    match op.as_str() {
        "export" if options.len() == 2 => prepare(get("--repo")?, get("--out")?, || {}),
        "verify" if options.keys().all(|k| ["--source", "--repo"].contains(k)) => verify(
            get("--source")?,
            options.get("--repo").map(PathBuf::as_path),
        ),
        "list" if options.len() == 1 => {
            let inputs = snapshot(&Dir::open(get("--repo")?)?)?;
            Ok(json!({"ok":true,"files":inputs.keys().collect::<Vec<_>>()}))
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
#[test]
fn historical_reference_role_requires_the_exact_versioned_prefix() {
    let historical = role("Sources/LauncherCore/Files.swift");
    for path in [
        "bench/reference/0.8.26/Package.swift",
        "bench/reference/0.8.26/Sources/RuycastApp/main.swift",
        "bench/reference/0.8.26/rust/src/lib.rs",
    ] {
        assert_eq!(role(path), historical, "{path}");
    }
    for path in [
        "bench/reference/0.8.260/Sources/main.swift",
        "bench/reference/0.8.26-old/Sources/main.swift",
        "bench/reference/0.8.26",
        "bench/f1-runner/src/main.rs",
    ] {
        assert_ne!(role(path), historical, "{path}");
    }
    let manifest = manifest(&Snapshot::new());
    assert_eq!(manifest.historical_references, [
        "Package.swift", "Sources/", "Tests/LauncherCoreTests/", "bench/reference/0.8.26/",
    ]);
}
