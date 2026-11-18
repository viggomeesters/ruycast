//! Disk headroom checks and a narrow, lease-protected Cargo-cache cleanup.
use super::{BuildLock, Result};
use serde_json::json;
use std::{
    ffi::CString,
    fs,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

pub const FREE_SPACE_FLOOR: u64 = 50_000_000_000;
pub const DEBUG_BUILD_PEAK: u64 = 2_000_000_000;
pub const RELEASE_BUILD_PEAK: u64 = 3_000_000_000;
pub const VALIDATION_PEAK: u64 = 8_000_000_000;
pub const VERIFICATION_PIPELINE_PEAK: u64 = 11_000_000_000;

const CACHE_ROOTS: &[&str] = &[
    ".build/full-rust-debug/core",
    ".build/full-rust-debug/native",
    ".build/full-rust-release/core",
    ".build/full-rust-release/native",
    ".build/delivery-tools",
    "rust-native/target",
    "rust-native/target/validation-build",
    "rust-native/target/validation-headless",
    "rust-native/target/validation-native",
    "rust-native/target/validation-release",
];
const CACHE_DIRECTORIES: &[&str] = &["build", "deps", "incremental", ".fingerprint"];
const PROFILES: &[&str] = &["debug", "release", "test"];
const TARGET_TRIPLES: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin"];

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

pub fn available_bytes(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(error)?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(format!(
            "Cannot measure available filesystem space: {}",
            std::io::Error::last_os_error()
        ));
    }
    let info = unsafe { info.assume_init() };
    let blocks = u64::from(info.f_bavail);
    let block_size = info.f_frsize;
    blocks
        .checked_mul(block_size)
        .ok_or_else(|| "Available filesystem byte count overflowed.".into())
}

pub fn capacity(path: &Path, estimated_peak: u64) -> Result<serde_json::Value> {
    capacity_for(available_bytes(path)?, estimated_peak)
}

fn capacity_for(available: u64, estimated_peak: u64) -> Result<serde_json::Value> {
    let required = required_bytes(estimated_peak)?;
    Ok(json!({
        "availableBytes": available,
        "freeSpaceFloorBytes": FREE_SPACE_FLOOR,
        "estimatedPeakBytes": estimated_peak,
        "requiredBytes": required,
        "canStart": can_start(available, estimated_peak)?,
    }))
}

fn required_bytes(estimated_peak: u64) -> Result<u64> {
    FREE_SPACE_FLOOR
        .checked_add(estimated_peak)
        .ok_or_else(|| "Disk headroom requirement overflowed.".into())
}

fn can_start(available: u64, estimated_peak: u64) -> Result<bool> {
    Ok(available >= required_bytes(estimated_peak)?)
}

pub fn require_capacity(path: &Path, estimated_peak: u64, operation: &str) -> Result<()> {
    let status = capacity(path, estimated_peak)?;
    if status["canStart"] != true {
        return Err(format!(
            "Insufficient disk headroom for {operation}: {} bytes available, {} required (50 GB floor plus estimated peak).",
            status["availableBytes"], status["requiredBytes"]
        ));
    }
    Ok(())
}

pub fn status(path: &Path) -> Result<serde_json::Value> {
    let available = available_bytes(path)?;
    Ok(json!({
        "ok": true,
        "schema": "ruycast.disk-budget.v1",
        "repo": path,
        "availableBytes": available,
        "freeSpaceFloorBytes": FREE_SPACE_FLOOR,
        "operations": {
            "debugBuild": capacity_for(available, DEBUG_BUILD_PEAK)?,
            "releaseBuild": capacity_for(available, RELEASE_BUILD_PEAK)?,
            "fullNativeValidation": capacity_for(available, VALIDATION_PEAK)?,
            "verificationPipeline": capacity_for(available, VERIFICATION_PIPELINE_PEAK)?,
        }
    }))
}

fn checked_dir(root: &Path, relative: &Path) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Cache paths must be relative and normalized.".into());
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("Refusing linked cache path: {}", path.display()));
            }
            Ok(meta) if path == root.join(relative) && !meta.is_dir() => {
                return Err(format!("Cache path is not a directory: {}", path.display()));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(error(e)),
        }
    }
    Ok(Some(path))
}

fn cache_bytes(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if meta.file_type().is_symlink() {
        return Err(format!("Refusing linked cache content: {}", path.display()));
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Err(format!("Unsupported cache content: {}", path.display()));
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(error)? {
        total = total
            .checked_add(cache_bytes(&entry.map_err(error)?.path())?)
            .ok_or("Cache byte count overflowed.")?;
    }
    Ok(total)
}

fn profile_directories(target_root: &Path) -> Result<Vec<PathBuf>> {
    let mut profiles = Vec::new();
    let meta = match fs::symlink_metadata(target_root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(profiles),
        Err(e) => return Err(error(e)),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "Invalid Cargo target directory: {}",
            target_root.display()
        ));
    }
    for first in fs::read_dir(target_root).map_err(error)? {
        let first = first.map_err(error)?;
        let first_path = first.path();
        let first_meta = fs::symlink_metadata(&first_path).map_err(error)?;
        if first_meta.file_type().is_symlink() {
            return Err(format!(
                "Refusing linked Cargo target entry: {}",
                first_path.display()
            ));
        }
        if !first_meta.is_dir() {
            continue;
        }
        if PROFILES.contains(&first.file_name().to_string_lossy().as_ref()) {
            profiles.push(first_path);
            continue;
        }
        if !TARGET_TRIPLES.contains(&first.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        for second in fs::read_dir(&first_path).map_err(error)? {
            let second = second.map_err(error)?;
            if !PROFILES.contains(&second.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            let second_path = second.path();
            let second_meta = fs::symlink_metadata(&second_path).map_err(error)?;
            if second_meta.file_type().is_symlink() || !second_meta.is_dir() {
                return Err(format!(
                    "Invalid Cargo profile directory: {}",
                    second_path.display()
                ));
            }
            profiles.push(second_path);
        }
    }
    Ok(profiles)
}

pub fn clean(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let before = available_bytes(root)?;
    let mut removed = Vec::<String>::new();
    let mut removed_bytes = 0u64;
    for relative in CACHE_ROOTS {
        let Some(target_root) = checked_dir(root, Path::new(relative))? else {
            continue;
        };
        for profile in profile_directories(&target_root)? {
            for name in CACHE_DIRECTORIES {
                let candidate = profile.join(name);
                let metadata = match fs::symlink_metadata(&candidate) {
                    Ok(metadata) => metadata,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(error(e)),
                };
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Refusing non-directory Cargo cache: {}",
                        candidate.display()
                    ));
                }
                let size = cache_bytes(&candidate)?;
                lease.validate()?;
                let current = fs::symlink_metadata(&candidate).map_err(error)?;
                if current.file_type().is_symlink()
                    || !current.is_dir()
                    || (current.dev(), current.ino()) != (metadata.dev(), metadata.ino())
                {
                    return Err(format!(
                        "Cargo cache identity changed: {}",
                        candidate.display()
                    ));
                }
                fs::remove_dir_all(&candidate).map_err(error)?;
                removed_bytes = removed_bytes
                    .checked_add(size)
                    .ok_or("Removed cache byte count overflowed.")?;
                removed.push(
                    candidate
                        .strip_prefix(root)
                        .map_err(error)?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    lease.validate()?;
    let after = available_bytes(root)?;
    Ok(json!({
        "ok": true,
        "schema": "ruycast.cache-cleanup.v1",
        "repo": root,
        "availableBytesBefore": before,
        "availableBytesAfter": after,
        "removedLogicalBytes": removed_bytes,
        "removedDirectories": removed,
        "leaseHeld": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn capacity_requires_floor_plus_estimated_peak_without_overflow() {
        assert!(!can_start(FREE_SPACE_FLOOR + 4, 5).unwrap());
        assert!(can_start(FREE_SPACE_FLOOR + 5, 5).unwrap());
        assert!(can_start(FREE_SPACE_FLOOR + 6, 5).unwrap());
        assert!(required_bytes(u64::MAX).is_err());
    }

    #[test]
    fn cleanup_removes_only_allowlisted_cache_and_keeps_receipts_and_evidence() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ruycast-disk-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let cleanup = || {
            let release_root = root.join(".build/full-rust-release");
            fs::create_dir_all(release_root.join("core/aarch64-apple-darwin/release/deps"))
                .unwrap();
            fs::create_dir_all(root.join(".build/delivery-tools/debug/incremental")).unwrap();
            fs::create_dir_all(root.join(".build/delivery-tools/debug/deps")).unwrap();
            fs::create_dir_all(root.join("rust-native/target/debug/.fingerprint")).unwrap();
            fs::create_dir_all(root.join("rust-native/target/debug/build")).unwrap();
            fs::create_dir_all(root.join("rust-native/target/debug/deps")).unwrap();
            fs::create_dir_all(root.join("rust-native/target/debug/incremental")).unwrap();
            fs::create_dir_all(root.join("rust-native/target/verification")).unwrap();
            let evidence_cache =
                root.join("rust-native/target/validation-build/evidence/debug/deps");
            fs::create_dir_all(&evidence_cache).unwrap();
            fs::write(release_root.join("development-build.json"), b"receipt").unwrap();
            fs::write(release_root.join("release-preparation.json"), b"pending").unwrap();
            fs::write(
                release_root.join("core/aarch64-apple-darwin/release/deps/old.rlib"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join(".build/delivery-tools/debug/incremental/old.o"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join(".build/delivery-tools/debug/deps/old.rlib"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/debug/.fingerprint/old.json"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/debug/build/old.out"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/debug/deps/old.rlib"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/debug/incremental/old.o"),
                b"cache",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/debug/ruycast-native"),
                b"binary",
            )
            .unwrap();
            fs::write(
                root.join(".build/delivery-tools/debug/ruycast-build"),
                b"tool",
            )
            .unwrap();
            fs::write(
                root.join("rust-native/target/verification/report.json"),
                b"evidence",
            )
            .unwrap();
            fs::write(evidence_cache.join("preserve.rlib"), b"unknown").unwrap();
            let lease = BuildLock::acquire_from_environment(
                &root,
                &std::collections::BTreeMap::new(),
            )
            .unwrap();
            let lock_inode = fs::metadata(root.join(".build/artifacts.lock"))
                .unwrap()
                .ino();
            let result = clean(&root, &lease).unwrap();
            assert_eq!(result["removedLogicalBytes"], 35);
            assert_eq!(result["leaseHeld"], true);
            assert!(!release_root
                .join("core/aarch64-apple-darwin/release/deps")
                .exists());
            for cache in [
                root.join(".build/delivery-tools/debug/incremental"),
                root.join(".build/delivery-tools/debug/deps"),
                root.join("rust-native/target/debug/.fingerprint"),
                root.join("rust-native/target/debug/build"),
                root.join("rust-native/target/debug/deps"),
                root.join("rust-native/target/debug/incremental"),
            ] {
                assert!(!cache.exists(), "cache remains: {}", cache.display());
            }
            assert_eq!(
                fs::read(root.join("rust-native/target/debug/ruycast-native")).unwrap(),
                b"binary"
            );
            assert_eq!(
                fs::read(root.join(".build/delivery-tools/debug/ruycast-build")).unwrap(),
                b"tool"
            );
            assert_eq!(
                fs::read(release_root.join("development-build.json")).unwrap(),
                b"receipt"
            );
            assert_eq!(
                fs::read(release_root.join("release-preparation.json")).unwrap(),
                b"pending"
            );
            assert_eq!(
                fs::read(root.join("rust-native/target/verification/report.json")).unwrap(),
                b"evidence"
            );
            assert_eq!(
                fs::read(evidence_cache.join("preserve.rlib")).unwrap(),
                b"unknown"
            );
            assert_eq!(
                fs::metadata(root.join(".build/artifacts.lock"))
                    .unwrap()
                    .ino(),
                lock_inode
            );
            lease.validate().unwrap();
        };
        cleanup();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_refuses_symlinked_cache_and_preserves_its_target() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let suffix = format!(
            "ruycast-disk-budget-link-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(&suffix);
        let outside = std::env::temp_dir().join(format!("{suffix}-outside"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        let cache = root.join(".build/full-rust-debug/core/aarch64-apple-darwin/debug/deps");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(outside.join("keep.rlib"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, &cache).unwrap();
        let lease = BuildLock::acquire_from_environment(
            &root,
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert!(clean(&root, &lease).is_err());
        assert_eq!(fs::read(outside.join("keep.rlib")).unwrap(), b"outside");
        lease.validate().unwrap();
        drop(lease);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn cache_allowlist_never_includes_receipts_or_verification_evidence() {
        assert!(!CACHE_ROOTS.iter().any(|path| path.contains("receipts")));
        assert!(!CACHE_ROOTS
            .iter()
            .any(|path| path.ends_with("verification")));
        assert_eq!(
            CACHE_DIRECTORIES,
            &["build", "deps", "incremental", ".fingerprint"]
        );
    }
}
