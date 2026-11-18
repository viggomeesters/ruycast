//! Durable preparation of a release pair. The artifact lease belongs to the
//! caller and remains held through recovery, tools, publication and consumers.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
const RECEIPT: &str = ".build/full-rust-release/release-preparation.json";
const PENDING: &str = ".build/full-rust-release/release-preparation.pending.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Symbols {
    pub uuid: String,
    pub unstripped_path: String,
    pub unstripped_sha256: String,
    pub unstripped_bytes: u64,
    pub stripped_bytes: u64,
    pub backed_sections_sha256: BTreeMap<String, String>,
    pub d_sym_path: String,
    pub d_sym_sha256: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    schema: String,
    target: String,
    strip_arguments: Vec<String>,
    pub artifacts: BTreeMap<String, Symbols>,
    pub binaries: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    schema: String,
    changes: BTreeMap<String, String>,
    preparation: Record,
}
pub(super) struct Stage(PathBuf, std::cell::Cell<bool>);
impl Stage {
    pub(super) fn new(root: &Path, prefix: &str) -> Result<Self> {
        let parent = root.join(OUTPUT);
        Self::in_parent(&parent, prefix)
    }
    pub(super) fn in_parent(parent: &Path, prefix: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        ruycast_native::private_storage::secure_directory(parent)?;
        for _ in 0..128 {
            let serial = NEXT
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .map_err(|_| "Stage counter exhausted.")?;
            let path = parent.join(format!(".{prefix}-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(error)?;
                    return Ok(Self(path, std::cell::Cell::new(false)));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(e)),
            }
        }
        Err("Cannot allocate an exclusive preparation stage.".into())
    }
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.1.get() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
pub(super) struct ShortPathAlias {
    path: PathBuf,
    target: PathBuf,
}
impl ShortPathAlias {
    pub(super) fn new(target: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let target = fs::canonicalize(target).map_err(error)?;
        ruycast_native::private_storage::secure_directory(&target)?;
        let parent = fs::canonicalize("/tmp").map_err(error)?;
        let parent_meta = fs::metadata(&parent).map_err(error)?;
        if !parent_meta.is_dir()
            || parent_meta.mode() & libc::S_ISVTX as u32 == 0
            || parent_meta.mode() & libc::S_IWOTH as u32 == 0
        {
            return Err("Short temporary parent is not a sticky shared directory.".into());
        }
        for _ in 0..128 {
            let serial = NEXT
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .map_err(|_| "Short temporary alias counter exhausted.")?;
            let path = parent.join(format!(".rvf-{}-{serial}", std::process::id()));
            match std::os::unix::fs::symlink(&target, &path) {
                Ok(()) => return Ok(Self { path, target }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error(e)),
            }
        }
        Err("Cannot allocate an exclusive short fixture alias.".into())
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for ShortPathAlias {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|meta| meta.file_type().is_symlink())
            && fs::read_link(&self.path).is_ok_and(|target| target == self.target)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
// A publication stage may contain displaced external writes, including writes
// through already-open descriptors. Retain it durably even after success; never
// infer that a later cleanup is safe from a pre-publication digest.
fn exchange(from: &Path, to: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(error)?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(error)?;
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } != 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn sync_parents(from: &Path, to: &Path) -> Result<()> {
    for path in [from, to] {
        File::open(path.parent().ok_or("Missing exchange parent.")?)
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
    }
    Ok(())
}
fn replace_preserving(stage: &Stage, from: &Path, to: &Path, expected: &str) -> Result<()> {
    stage.1.set(true);
    exchange(from, to)?;
    // Validate the actual atomically displaced preimage, not the destination
    // checked earlier. Unknown bytes are retained by the exchange itself.
    let checked = digest(from).and_then(|actual| {
        if actual == expected {
            Ok(())
        } else {
            Err("Release preimage changed at publication.".into())
        }
    });
    if let Err(problem) = checked {
        // A concurrent later destination write is also preserved by swapping,
        // rather than overwriting it during rollback. Neither path is deleted.
        exchange(from, to)?;
        sync_parents(from, to)?;
        return Err(problem);
    }
    sync_parents(from, to)
}
pub(super) fn require_settled(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root.join(PENDING)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(e)),
        Ok(_) => Err("Unresolved release preparation intent; build recovery is required.".into()),
    }
}
fn relative(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .map_err(error)?
        .to_str()
        .map(str::to_owned)
        .ok_or("Preparation path must be UTF-8.".into())
}
fn atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ruycast_native::private_storage::write(
        path,
        &serde_json::to_vec_pretty(value).map_err(error)?,
        true,
    )
}
fn remove_synced(path: &Path) -> Result<()> {
    fs::remove_file(path).map_err(error)?;
    File::open(path.parent().ok_or("Missing receipt parent.")?)
        .map_err(error)?
        .sync_all()
        .map_err(error)
}
fn copy(source: &Path, target: &Path) -> Result<()> {
    let mut input = regular_file(source)?;
    let before = input.metadata().map_err(error)?;
    if before.nlink() != 1 {
        return Err("Diagnostic source has multiple links.".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(before.mode() & 0o777)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(target)
        .map_err(error)?;
    std::io::copy(&mut input, &mut output).map_err(error)?;
    output.sync_all().map_err(error)?;
    let after = input.metadata().map_err(error)?;
    if (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
    ) != (
        after.dev(),
        after.ino(),
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
    ) || digest(source)? != digest(target)?
    {
        return Err("Diagnostic source changed during copy.".into());
    }
    Ok(())
}
fn tree(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut rows = BTreeMap::new();
    source_tree(path, path, &mut rows)?;
    Ok(rows)
}
fn allowed(root: &Path, record: &Record) -> Result<()> {
    if record.schema != "ruycast.release-preparation.v2"
        || record.strip_arguments != ["-S", "-x"]
        || record.artifacts.len() != 2
        || record.binaries.len() != 2
    {
        return Err("Invalid release preparation schema.".into());
    }
    for (name, _) in artifact_paths(&record.target)? {
        let row = record
            .artifacts
            .get(&name)
            .ok_or("Missing prepared artifact.")?;
        if row.unstripped_sha256.len() != 64
            || !row
                .unstripped_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Invalid retained diagnostic digest.".into());
        }
        if !record.binaries.contains_key(&name) {
            return Err("Missing prepared binary digest.".into());
        }
        let expected = format!(
            "{OUTPUT}/symbols/{}/{}",
            row.unstripped_sha256,
            Path::new(&name)
                .file_name()
                .ok_or("Missing binary name.")?
                .to_str()
                .ok_or("Invalid binary name.")?
        );
        if row.unstripped_path != expected || row.d_sym_path != format!("{expected}.dSYM") {
            return Err("Unrecognized retained diagnostic path.".into());
        }
        let original = root.join(&row.unstripped_path);
        let identity = macho::read(&original)?;
        if digest(&original)? != row.unstripped_sha256
            || identity.uuid != row.uuid
            || identity.backed_sections_sha256 != row.backed_sections_sha256
            || original.metadata().map_err(error)?.len() != row.unstripped_bytes
            || tree(&root.join(&row.d_sym_path))? != row.d_sym_sha256
        {
            return Err("Retained release diagnostics changed.".into());
        }
        let dwarf = root
            .join(&row.d_sym_path)
            .join("Contents/Resources/DWARF")
            .join(
                original
                    .file_name()
                    .ok_or("Missing retained binary name.")?,
            );
        if macho::read(&dwarf)?.uuid != row.uuid {
            return Err("Retained dSYM UUID does not match release binary.".into());
        }
    }
    Ok(())
}
pub(super) fn verify(root: &Path, target: &str) -> Result<Record> {
    require_settled(root)?;
    let result = verify_prepared(root, target)?;
    require_settled(root)?;
    Ok(result)
}
fn verify_prepared(root: &Path, target: &str) -> Result<Record> {
    let record: Record =
        serde_json::from_slice(&regular_bytes(&root.join(RECEIPT))?).map_err(error)?;
    if record.target != target {
        return Err("Release preparation target changed.".into());
    }
    allowed(root, &record)?;
    for (name, expected) in &record.binaries {
        let path = root.join(name);
        let identity = macho::read(&path)?;
        let row = &record.artifacts[name];
        if digest(&path)? != *expected
            || identity.uuid != row.uuid
            || identity.backed_sections_sha256 != row.backed_sections_sha256
            || path.metadata().map_err(error)?.len() != row.stripped_bytes
        {
            return Err("Prepared release artifact changed.".into());
        }
    }
    Ok(record)
}
pub(super) fn binding(root: &Path, target: &str) -> Result<super::PreparationBinding> {
    verify(root, target)?;
    Ok(super::PreparationBinding {
        path: RECEIPT.into(),
        sha256: digest(&root.join(RECEIPT))?,
    })
}
pub(super) fn recover(root: &Path) -> Result<()> {
    recover_with(root, |_, _| Ok(()))
}
fn recover_with(
    root: &Path,
    mut before_copy: impl FnMut(usize, &Path) -> Result<()>,
) -> Result<()> {
    let pending = root.join(PENDING);
    match fs::symlink_metadata(&pending) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(e)),
        Ok(_) => (),
    }
    let intent: Intent = serde_json::from_slice(&regular_bytes(&pending)?).map_err(error)?;
    if intent.schema != "ruycast.release-preparation-intent.v2" {
        return Err("Unknown release recovery intent.".into());
    }
    allowed(root, &intent.preparation)?;
    if verify_prepared(root, &intent.preparation.target).is_ok_and(|r| r == intent.preparation) {
        return remove_synced(&pending);
    }
    // All originals and all current states are validated before either binary
    // changes. Unknown writes must never be overwritten during recovery.
    let mut observed = BTreeMap::new();
    for (name, before) in &intent.changes {
        let row = intent
            .preparation
            .artifacts
            .get(name)
            .ok_or("Unknown recovery artifact.")?;
        if before != &row.unstripped_sha256 {
            return Err("Recovery original does not match intent.".into());
        }
        let now = digest(&root.join(name))?;
        if &now != before && Some(&now) != intent.preparation.binaries.get(name) {
            return Err("Release artifact changed during interrupted preparation.".into());
        }
        observed.insert(name.clone(), now);
    }
    let stage = Stage::new(root, "recover")?;
    let mut changes = vec![];
    for (index, name) in intent.changes.keys().enumerate() {
        let row = &intent.preparation.artifacts[name];
        let original = root.join(&row.unstripped_path);
        let candidate = stage.path().join(
            Path::new(name)
                .file_name()
                .ok_or("Missing recovery name.")?,
        );
        before_copy(index, &original)?;
        copy(&original, &candidate)?;
        if digest(&candidate)? != row.unstripped_sha256 {
            return Err("Recovery diagnostic changed after validation.".into());
        }
        changes.push((candidate, root.join(name), observed[name].clone()));
    }
    for (from, to, expected) in changes {
        replace_preserving(&stage, &from, &to, &expected)?;
    }
    remove_synced(&pending)
}
pub(super) fn prepare(root: &Path, target: &str) -> Result<super::PreparationBinding> {
    prepare_with(root, target, |_, _| Ok(()))
}
fn prepare_with(
    root: &Path,
    target: &str,
    mut before_replace: impl FnMut(usize, &Path) -> Result<()>,
) -> Result<super::PreparationBinding> {
    recover(root)?;
    let previous = match fs::symlink_metadata(root.join(RECEIPT)) {
        Ok(_) => Some(
            serde_json::from_slice::<Record>(&regular_bytes(&root.join(RECEIPT))?)
                .map_err(error)?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(error(e)),
    };
    if let Some(record) = &previous {
        allowed(root, record)?;
    }
    let stage = Stage::new(root, "strip")?;
    let mut record = Record {
        schema: "ruycast.release-preparation.v2".into(),
        target: target.into(),
        strip_arguments: vec!["-S".into(), "-x".into()],
        artifacts: BTreeMap::new(),
        binaries: BTreeMap::new(),
    };
    let mut changes = BTreeMap::new();
    let mut staged = vec![];
    for (name, _) in artifact_paths(target)? {
        let source = root.join(&name);
        let before = digest(&source)?;
        if let Some(old) = &previous {
            if old.target == target && old.binaries.get(&name) == Some(&before) {
                record
                    .artifacts
                    .insert(name.clone(), old.artifacts[&name].clone());
                record.binaries.insert(name, before);
                continue;
            }
        }
        let identity = macho::read(&source)?;
        let filename = source.file_name().ok_or("Missing release file name.")?;
        let retained_dir = root.join(OUTPUT).join("symbols").join(&before);
        ruycast_native::private_storage::secure_directory(&retained_dir)?;
        let original = retained_dir.join(filename);
        match fs::symlink_metadata(&original) {
            Ok(_) => {
                if digest(&original)? != before {
                    return Err("Existing retained executable differs.".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => copy(&source, &original)?,
            Err(e) => return Err(error(e)),
        }
        let dsym = retained_dir.join(format!(
            "{}.dSYM",
            filename.to_str().ok_or("Invalid release file name.")?
        ));
        match fs::symlink_metadata(&dsym) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                output(
                    root,
                    "/usr/bin/xcrun",
                    &[
                        "dsymutil",
                        source.to_str().ok_or("Invalid release path.")?,
                        "-o",
                        dsym.to_str().ok_or("Invalid dSYM path.")?,
                    ],
                )?;
            }
            Err(e) => return Err(error(e)),
        }
        let dwarf = dsym.join("Contents/Resources/DWARF").join(filename);
        if macho::read(&dwarf)?.uuid != identity.uuid {
            return Err("Generated dSYM UUID does not match release executable.".into());
        }
        let candidate = stage.path().join(filename);
        copy(&source, &candidate)?;
        output(
            root,
            "/usr/bin/strip",
            &["-S", "-x", candidate.to_str().ok_or("Invalid strip path.")?],
        )?;
        if macho::read(&candidate)? != identity {
            return Err("Stripping changed backed executable/data sections or UUID.".into());
        }
        output(
            root,
            "/usr/bin/codesign",
            &[
                "--verify",
                "--strict",
                candidate.to_str().ok_or("Invalid verify path.")?,
            ],
        )?;
        File::open(&candidate)
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
        record.artifacts.insert(
            name.clone(),
            Symbols {
                uuid: identity.uuid,
                unstripped_path: relative(root, &original)?,
                unstripped_sha256: before.clone(),
                unstripped_bytes: source.metadata().map_err(error)?.len(),
                stripped_bytes: candidate.metadata().map_err(error)?.len(),
                backed_sections_sha256: identity.backed_sections_sha256,
                d_sym_path: relative(root, &dsym)?,
                d_sym_sha256: tree(&dsym)?,
            },
        );
        record.binaries.insert(name.clone(), digest(&candidate)?);
        changes.insert(name.clone(), before);
        staged.push((candidate, source));
    }
    for (name, before) in &changes {
        if digest(&root.join(name))? != *before {
            return Err("Release source changed before preparation commit.".into());
        }
    }
    allowed(root, &record)?;
    atomic(
        &root.join(PENDING),
        &Intent {
            schema: "ruycast.release-preparation-intent.v2".into(),
            changes: changes.clone(),
            preparation: record.clone(),
        },
    )?;
    let commit = (|| {
        for (index, (from, to)) in staged.into_iter().enumerate() {
            before_replace(index, &to)?;
            let name = relative(root, &to)?;
            replace_preserving(&stage, &from, &to, &changes[&name])?;
        }
        atomic(&root.join(RECEIPT), &record)?;
        verify_prepared(root, target)?;
        remove_synced(&root.join(PENDING))
    })();
    if let Err(error) = commit {
        recover(root)?;
        return Err(error);
    }
    binding(root, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_validation_aliases_are_unique_bounded_and_remove_only_the_alias() {
        use std::os::unix::fs::MetadataExt;
        let root = super::super::tests::Fixture::new();
        let target = root.0.join("validation-fixtures");
        fs::create_dir(&target).unwrap();
        ruycast_native::private_storage::secure_directory(&target).unwrap();
        let canonical_target = fs::canonicalize(&target).unwrap();
        let alias = ShortPathAlias::new(&target).unwrap();
        let alias_path = alias.path().to_path_buf();
        let second = ShortPathAlias::new(&target).unwrap();
        let alias_metadata = fs::symlink_metadata(&alias_path).unwrap();
        assert!(alias_path.starts_with(fs::canonicalize("/tmp").unwrap()));
        assert!(alias_path.as_os_str().as_encoded_bytes().len() < 48);
        assert!(alias_metadata.file_type().is_symlink());
        assert_eq!(alias_metadata.uid(), unsafe { libc::getuid() });
        assert_eq!(fs::canonicalize(&alias_path).unwrap(), canonical_target);
        assert_ne!(alias_path, second.path());
        let nested = Stage::in_parent(alias.path(), "u").unwrap();
        let nested_path = nested.path().to_path_buf();
        assert!(nested_path.as_os_str().as_encoded_bytes().len() < 72);
        assert!(fs::canonicalize(&nested_path)
            .unwrap()
            .starts_with(&canonical_target));
        drop(nested);
        assert!(!fs::symlink_metadata(&target.join(nested_path.file_name().unwrap())).is_ok());
        let second_path = second.path().to_path_buf();
        drop(alias);
        drop(second);
        assert!(fs::symlink_metadata(&alias_path).is_err());
        assert!(fs::symlink_metadata(&second_path).is_err());
        assert!(target.is_dir());
    }
    struct Fixture {
        root: super::super::tests::Fixture,
        target: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = super::super::tests::Fixture::new();
            let target = host_target(&root.0).unwrap();
            let source = root.0.join("owned_diagnostic.rs");
            fs::write(&source, b"#[inline(never)] fn owned(v:u64)->u64 { std::hint::black_box(v).wrapping_mul(37) } fn main(){println!(\"{}\",owned(7));}\n").unwrap();
            let image = root.0.join("owned_diagnostic");
            output(
                &root.0,
                "rustc",
                &[
                    "--crate-name",
                    "owned_diagnostic",
                    "--edition=2021",
                    "-g",
                    "-C",
                    "opt-level=1",
                    source.to_str().unwrap(),
                    "-o",
                    image.to_str().unwrap(),
                ],
            )
            .unwrap();
            for (name, _) in artifact_paths(&target).unwrap() {
                let path = root.0.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                copy(&image, &path).unwrap();
            }
            Self { root, target }
        }
        fn names(&self) -> Vec<String> {
            artifact_paths(&self.target)
                .unwrap()
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        }
        fn current(&self) -> BTreeMap<String, String> {
            self.names()
                .into_iter()
                .map(|name| {
                    let sha = digest(&self.root.0.join(&name)).unwrap();
                    (name, sha)
                })
                .collect()
        }
        fn compile_variant(&self, image: &Path, multiplier: u64) {
            let source = self.root.0.join(format!("variant-{multiplier}.rs"));
            fs::write(
                &source,
                format!(
                    "#[inline(never)] fn owned(v:u64)->u64 {{ std::hint::black_box(v).wrapping_mul({multiplier}) }} fn main(){{println!(\"{{}}\",owned(7));}}\n"
                ),
            )
            .unwrap();
            output(
                &self.root.0,
                "rustc",
                &[
                    "--crate-name",
                    "owned_diagnostic",
                    "--edition=2021",
                    "-g",
                    "-C",
                    "opt-level=1",
                    source.to_str().unwrap(),
                    "-o",
                    image.to_str().unwrap(),
                ],
            )
            .unwrap();
        }
    }
    #[test]
    fn exchanged_preimages_survive_open_descriptor_writes_and_stage_drop() {
        use std::io::Write;
        let f = super::super::tests::Fixture::new();
        let target = f.0.join("owned-target");
        fs::write(&target, b"known preimage").unwrap();
        let expected = digest(&target).unwrap();
        let mut old_descriptor = OpenOptions::new()
            .write(true)
            .truncate(false)
            .open(&target)
            .unwrap();
        let stage = Stage::new(&f.0, "exchange-test").unwrap();
        let displaced = stage.path().join("candidate");
        fs::write(&displaced, b"known replacement").unwrap();
        replace_preserving(&stage, &displaced, &target, &expected).unwrap();
        old_descriptor.write_all(b"late unknown bytes").unwrap();
        old_descriptor.sync_all().unwrap();
        drop(stage);
        assert_eq!(regular_bytes(&displaced).unwrap(), b"late unknown bytes");
        assert_eq!(regular_bytes(&target).unwrap(), b"known replacement");
        // Atomic exchange has no replacing-rename fallback for a missing peer.
        let stage = Stage::new(&f.0, "missing-test").unwrap();
        let from = stage.path().join("candidate");
        fs::write(&from, b"keep candidate").unwrap();
        assert!(replace_preserving(&stage, &from, &f.0.join("absent"), &expected).is_err());
        drop(stage);
        assert_eq!(regular_bytes(&from).unwrap(), b"keep candidate");
    }
    #[test]
    fn review_late_unknown_commit_write_is_not_overwritten() {
        let f = Fixture::new();
        let selected = f.names()[0].clone();
        let unknown = b"late owned external write after preflight";
        let result = prepare_with(&f.root.0, &f.target, |index, destination| {
            if index == 0 {
                fs::write(destination, unknown).map_err(error)?;
            }
            Ok(())
        });
        let bytes = regular_bytes(&f.root.0.join(&selected)).unwrap();
        eprintln!(
            "Preparation succeeded: {}; late write preserved: {}",
            result.is_ok(),
            bytes == unknown
        );
        assert!(result.is_err());
        assert_eq!(bytes, unknown);
    }
    #[test]
    fn real_rust_strip_keeps_code_uuid_and_matching_diagnostics_and_cached_originals() {
        let f = Fixture::new();
        let before = f.current();
        let binding = prepare(&f.root.0, &f.target).unwrap();
        assert_eq!(binding.path, RECEIPT);
        let first = verify(&f.root.0, &f.target).unwrap();
        for name in f.names() {
            let row = &first.artifacts[&name];
            assert_eq!(row.unstripped_sha256, before[&name]);
            assert!(row.stripped_bytes < row.unstripped_bytes);
        }
        assert_eq!(
            prepare(&f.root.0, &f.target).unwrap().sha256,
            binding.sha256
        );
        assert_eq!(verify(&f.root.0, &f.target).unwrap(), first);
        // A real host relink must leave the untouched CLI's full diagnostic
        // row and stripped bytes unchanged.
        let names = f.names();
        let host = names[0].clone();
        let cli = names[1].clone();
        f.compile_variant(&f.root.0.join(&host), 41);
        assert_ne!(
            macho::read(&f.root.0.join(&host)).unwrap().uuid,
            first.artifacts[&host].uuid
        );
        prepare(&f.root.0, &f.target).unwrap();
        let after_host = verify(&f.root.0, &f.target).unwrap();
        assert_eq!(
            after_host.artifacts[&cli], first.artifacts[&cli],
            "host rebuild changed the CLI diagnostic artifact"
        );
        assert_eq!(after_host.binaries[&cli], first.binaries[&cli]);

        // Repeat in the other direction so the untouched host row is bound.
        f.compile_variant(&f.root.0.join(&cli), 43);
        assert_ne!(
            macho::read(&f.root.0.join(&cli)).unwrap().uuid,
            after_host.artifacts[&cli].uuid
        );
        prepare(&f.root.0, &f.target).unwrap();
        let after_cli = verify(&f.root.0, &f.target).unwrap();
        assert_eq!(after_cli.artifacts[&host], after_host.artifacts[&host]);
        assert_eq!(after_cli.binaries[&host], after_host.binaries[&host]);

        // Tampering with a retained original is refused without changing
        // either currently published executable.
        let retained = f.root.0.join(&after_cli.artifacts[&host].unstripped_path);
        let mut bytes = regular_bytes(&retained).unwrap();
        bytes.push(0xA5);
        fs::write(&retained, bytes).unwrap();
        let published = f.current();
        assert!(prepare(&f.root.0, &f.target).is_err());
        assert_eq!(f.current(), published);
    }
    #[test]
    fn failed_second_replace_rolls_back_pair_and_interrupted_pair_is_recovered() {
        let f = Fixture::new();
        let before = f.current();
        let failure = prepare_with(&f.root.0, &f.target, |index, _| {
            if index == 1 {
                Err("Owned second replacement failure".into())
            } else {
                Ok(())
            }
        });
        assert!(failure.err().unwrap().contains("second replacement"));
        assert_eq!(f.current(), before);
        assert!(!f.root.0.join(PENDING).exists());
        assert!(!f.root.0.join(RECEIPT).exists());
        prepare(&f.root.0, &f.target).unwrap();
        let record = verify(&f.root.0, &f.target).unwrap();
        let intent = Intent {
            schema: "ruycast.release-preparation-intent.v2".into(),
            changes: before.clone(),
            preparation: record.clone(),
        };
        atomic(&f.root.0.join(PENDING), &intent).unwrap();
        // Simulate process interruption after the first known replacement.
        let name = &f.names()[1];
        let original = f.root.0.join(&record.artifacts[name].unstripped_path);
        fs::remove_file(f.root.0.join(name)).unwrap();
        copy(&original, &f.root.0.join(name)).unwrap();
        // The next ordinary preparation call must recover the interrupted
        // pair before building the settled record.
        let settled_binding = prepare(&f.root.0, &f.target).unwrap();
        let settled = verify(&f.root.0, &f.target).unwrap();
        for name in f.names() {
            assert_eq!(settled.artifacts[&name].unstripped_sha256, before[&name]);
        }
        assert!(!f.root.0.join(PENDING).exists());
        let repeated_binding = prepare(&f.root.0, &f.target).unwrap();
        assert_eq!(repeated_binding.path, settled_binding.path);
        assert_eq!(repeated_binding.sha256, settled_binding.sha256);
        assert_eq!(verify(&f.root.0, &f.target).unwrap(), settled);
    }
    #[test]
    fn recovery_rejects_changed_staged_original_before_replacing_either_binary() {
        let f = Fixture::new();
        let original_pair = f.current();
        prepare(&f.root.0, &f.target).unwrap();
        let record = verify(&f.root.0, &f.target).unwrap();
        let name = f.names()[0].clone();
        let original = f.root.0.join(&record.artifacts[&name].unstripped_path);
        let intent = Intent {
            schema: "ruycast.release-preparation-intent.v2".into(),
            changes: original_pair,
            preparation: record,
        };
        atomic(&f.root.0.join(PENDING), &intent).unwrap();

        // Simulate interruption after one executable returned to its known
        // preimage while the peer remains in its stripped state.
        fs::remove_file(f.root.0.join(&name)).unwrap();
        copy(&original, &f.root.0.join(&name)).unwrap();
        let interrupted_pair = f.current();
        let pending_digest = digest(&f.root.0.join(PENDING)).unwrap();

        let result = recover_with(&f.root.0, |index, source| {
            // The first staged copy is already verified; a later mismatch
            // must still be rejected before either destination is replaced.
            if index == 1 {
                fs::write(source, b"changed after recovery validation").map_err(error)?;
            }
            Ok(())
        });
        assert!(result
            .unwrap_err()
            .contains("Recovery diagnostic changed after validation"));
        assert_eq!(f.current(), interrupted_pair);
        assert_eq!(digest(&f.root.0.join(PENDING)).unwrap(), pending_digest);
    }
    #[test]
    fn mismatched_diagnostics_and_unknown_recovery_writes_never_mutate_the_pair() {
        let f = Fixture::new();
        let before = f.current();
        prepare(&f.root.0, &f.target).unwrap();
        let record = verify(&f.root.0, &f.target).unwrap();
        let names = f.names();
        let name = &names[0];
        let intent = Intent {
            schema: "ruycast.release-preparation-intent.v2".into(),
            changes: before,
            preparation: record.clone(),
        };
        atomic(&f.root.0.join(PENDING), &intent).unwrap();
        fs::write(f.root.0.join(name), b"owned unknown write").unwrap();
        let unknown = f.current();
        assert!(recover(&f.root.0).is_err());
        assert_eq!(f.current(), unknown);
        fs::remove_file(f.root.0.join(PENDING)).unwrap();
        let row = &record.artifacts[&names[1]];
        let dwarf = f
            .root
            .0
            .join(&row.d_sym_path)
            .join("Contents/Resources/DWARF")
            .join(Path::new(&names[1]).file_name().unwrap());
        let mut bytes = regular_bytes(&dwarf).unwrap();
        // UUID resides in the LC_UUID command, not a fabricated metadata row.
        let identity = macho::parse(&bytes).unwrap();
        assert_eq!(identity.uuid, row.uuid);
        bytes[0] ^= 1;
        fs::write(&dwarf, bytes).unwrap();
        assert!(prepare(&f.root.0, &f.target).is_err());
        assert_eq!(f.current(), unknown);
    }
    #[test]
    fn valid_dsym_with_wrong_uuid_is_refused_before_binary_publication() {
        let f = Fixture::new();
        let before = f.current();
        let name = f.names()[0].clone();
        let other_dir = f.root.0.join("other-image");
        fs::create_dir_all(&other_dir).unwrap();
        let source = other_dir.join("different_uuid.rs");
        let other = other_dir.join(Path::new(&name).file_name().unwrap());
        fs::write(
            &source,
            b"#[inline(never)] fn owned(v:u64)->u64 { std::hint::black_box(v).wrapping_add(91) } fn main(){println!(\"{}\",owned(7));}\n",
        )
        .unwrap();
        output(
            &f.root.0,
            "rustc",
            &[
                "--crate-name",
                "different_uuid",
                "--edition=2021",
                "-g",
                "-C",
                "opt-level=1",
                source.to_str().unwrap(),
                "-o",
                other.to_str().unwrap(),
            ],
        )
        .unwrap();
        let original = f.root.0.join(OUTPUT).join("symbols").join(&before[&name]);
        let wrong_dsym = original.join(format!(
            "{}.dSYM",
            Path::new(&name).file_name().unwrap().to_str().unwrap()
        ));
        fs::create_dir_all(wrong_dsym.parent().unwrap()).unwrap();
        output(
            &f.root.0,
            "/usr/bin/xcrun",
            &[
                "dsymutil",
                other.to_str().unwrap(),
                "-o",
                wrong_dsym.to_str().unwrap(),
            ],
        )
        .unwrap();
        let target_uuid = macho::read(&f.root.0.join(&name)).unwrap().uuid;
        let wrong_uuid = macho::read(
            &wrong_dsym
                .join("Contents/Resources/DWARF")
                .join(Path::new(&name).file_name().unwrap()),
        )
        .unwrap()
        .uuid;
        assert_ne!(wrong_uuid, target_uuid);
        let error = match prepare(&f.root.0, &f.target) {
            Ok(_) => panic!("mismatched dSYM UUID must be refused"),
            Err(error) => error,
        };
        assert!(error.contains("dSYM UUID does not match"));
        assert_eq!(f.current(), before);
    }
}
