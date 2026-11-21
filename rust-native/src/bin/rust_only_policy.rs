//! Read-only legacy freeze and Rust product/test/helper-source policy. No lock generation.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const LOCK: &str = "rust-native/swift-reference-lock.json";
const RUNTIME: [&str; 2] = ["rust/src", "rust-native/src"];
const TOOLING: [&str; 7] = [
    "Tests",
    "scripts",
    "bench",
    "rust/tests",
    "rust/scripts",
    "rust-native/tests",
    "rust-native/scripts",
];
const METADATA: [&str; 8] = [
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "LICENSE",
    "LICENSE.md",
    "AGENTS.md",
    ".gitignore",
    "swift-reference-lock.json",
];
fn digest(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut block = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut block).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn regular(path: &Path) -> Result<bool, String> {
    Ok(fs::symlink_metadata(path)
        .map_err(|error| {
            if let Some(code) = error.raw_os_error() {
                let message = error.to_string();
                let message = message.split(" (os error ").next().unwrap();
                let name = path.to_str().unwrap_or("<non-UTF8 path>");
                format!(
                    "[Errno {code}] {message}: '{}'",
                    name.replace('\\', "\\\\").replace('\'', "\\'")
                )
            } else {
                error.to_string()
            }
        })?
        .is_file())
}
fn entries(path: &Path) -> Result<Vec<PathBuf>, String> {
    fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect()
}
fn children(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = entries(path)?;
    paths.sort();
    Ok(paths)
}
fn relative(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .unwrap()
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "filesystem path is not UTF-8".into())
}
fn scan_legacy(
    root: &Path,
    path: &Path,
    actual: &mut BTreeMap<String, String>,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    for file in children(path)? {
        let name = relative(root, &file)?;
        let metadata = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            errors.push(format!("legacy symlink: {name}"));
        } else if metadata.is_file() {
            actual.insert(name, digest(&file)?);
        } else if metadata.is_dir() {
            scan_legacy(root, &file, actual, errors)?;
        } else {
            errors.push(format!("nonregular legacy input: {name}"));
        }
    }
    Ok(())
}
fn scan_runtime(root: &Path, path: &Path, errors: &mut Vec<String>) -> Result<usize, String> {
    let mut count = 0;
    for file in children(path)? {
        let name = relative(root, &file)?;
        let metadata = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            errors.push(format!("runtime source symlink: {name}"));
        } else if metadata.is_file() {
            if file.file_name().unwrap() == ".DS_Store" {
                continue;
            }
            if file.extension().is_none_or(|v| v != "rs") {
                errors.push(format!("non-Rust runtime source: {name}"));
            } else {
                count += 1;
            }
        } else if metadata.is_dir() {
            count += scan_runtime(root, &file, errors)?;
        } else {
            errors.push(format!("nonregular runtime source: {name}"));
        }
    }
    Ok(count)
}
fn scan_product(
    root: &Path,
    product: &Path,
    folder: &Path,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let paths = entries(folder)?;
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for path in paths {
        relative(root, &path)?; // Never collapse distinct filesystem names through lossy Unicode.
        if path.is_dir() {
            directories.push(path);
        } else {
            files.push(path);
        }
    }
    if folder == product {
        // os.walk's top-level tooling check includes broken and file-target links.
        for name in ["tests", "scripts"] {
            if folder.join(name).is_symlink() {
                errors.push(format!(
                    "tooling source symlink: {}",
                    relative(root, &folder.join(name))?
                ));
            }
        }
        directories.retain(|path| {
            !["target", "tests", "scripts", "src", ".git"]
                .iter()
                .any(|name| path.file_name().unwrap() == *name)
        });
    }
    let mut recurse = Vec::new();
    for path in directories {
        if path.is_symlink() {
            errors.push(format!(
                "product directory symlink: {}",
                relative(root, &path)?
            ));
        } else {
            recurse.push(path);
        }
    }
    // Match os.walk: directory-link refusals, then filenames, then descendants,
    // retaining the filesystem enumeration order within each group.
    for path in files {
        let name = path
            .file_name()
            .unwrap()
            .to_str()
            .ok_or("filesystem path is not UTF-8")?;
        if name == ".DS_Store" {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let first = path
            .strip_prefix(product)
            .unwrap()
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .ok_or("filesystem path is not UTF-8")?;
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let resource = ["assets", "resources", "docs", ".cargo"].contains(&first)
            && ["json", "toml", "plist", "png", "svg", "icns", "md", "txt"]
                .contains(&extension.as_str());
        if metadata.is_symlink()
            || !metadata.is_file()
            || !(path.extension().is_some_and(|e| e == "rs")
                || METADATA.contains(&name)
                || resource)
        {
            errors.push(format!(
                "non-Rust/unknown product input: {}",
                relative(root, &path)?
            ));
        }
    }
    for path in recurse {
        scan_product(root, product, &path, errors)?;
    }
    Ok(())
}
fn scan_tooling(root: &Path, folder: &Path, errors: &mut Vec<String>) -> Result<(), String> {
    for path in children(folder)? {
        let name = relative(root, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            errors.push(format!("tooling source symlink: {name}"));
        } else if metadata.is_dir() {
            // Cargo output is not source. Only prune the crate's own output directory.
            if path.file_name().unwrap() != "target" || !folder.join("Cargo.toml").is_file() {
                scan_tooling(root, &path, errors)?;
            }
        } else if metadata.is_file() {
            let filename = path
                .file_name()
                .unwrap()
                .to_str()
                .ok_or("filesystem path is not UTF-8")?;
            // JSON fixtures and Markdown documentation are data, not executable source.
            if filename != ".DS_Store"
                && !path.extension().is_some_and(|extension| {
                    extension == "rs" || extension == "json" || extension == "md"
                })
                && !METADATA.contains(&filename)
            {
                errors.push(format!("non-Rust/unknown tooling input: {name}"));
            }
        } else {
            errors.push(format!("nonregular tooling input: {name}"));
        }
    }
    Ok(())
}
fn checked(root: &Path) -> Result<Value, String> {
    root.to_str().ok_or("filesystem path is not UTF-8")?;
    let lock_path = root.join(LOCK);
    if !regular(&lock_path)? {
        return Err("reference lock must be a regular file".into());
    }
    let lock: Value = serde_json::from_slice(&fs::read(&lock_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let expected = lock["files"]
        .as_object()
        .ok_or("invalid reference-lock contract")?;
    if lock["schema"] != "ruycast.rust-only-reference-lock.v1"
        || lock["legacy_roots"] != json!(["Sources"])
        || lock["runtime_roots"] != json!(RUNTIME)
        || !expected.contains_key("Package.swift")
    {
        return Err("invalid reference-lock contract".into());
    }
    for (name, sha) in expected {
        let path = Path::new(name);
        if path.is_absolute()
            || path.components().any(|p| p == Component::ParentDir)
            || (name != "Package.swift" && !name.starts_with("Sources/"))
            || !sha.as_str().is_some_and(|sha| {
                sha.len() == 64
                    && sha
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err("invalid frozen-file entry".into());
        }
    }
    let mut errors = Vec::new();
    let mut actual = BTreeMap::new();
    let legacy = root.join("Sources");
    if legacy.is_symlink() || !legacy.is_dir() {
        return Err("legacy source root must be a real directory".into());
    }
    scan_legacy(root, &legacy, &mut actual, &mut errors)?;
    let package = root.join("Package.swift");
    if !regular(&package)? {
        return Err("Package.swift must be a regular file".into());
    }
    actual.insert("Package.swift".into(), digest(&package)?);
    let names = expected
        .keys()
        .chain(actual.keys())
        .collect::<std::collections::BTreeSet<_>>();
    for name in names {
        if !expected.contains_key(name) {
            errors.push(format!("added legacy input: {name}"));
        } else if !actual.contains_key(name) {
            errors.push(format!("removed legacy input: {name}"));
        } else if expected[name] != actual[name] {
            errors.push(format!("changed legacy input: {name}"));
        }
    }
    let mut count = 0;
    for name in RUNTIME {
        let folder = root.join(name);
        if folder.is_symlink() || !folder.is_dir() {
            errors.push(format!(
                "runtime source root must be a real directory: {name}"
            ));
            continue;
        }
        count += scan_runtime(root, &folder, &mut errors)?;
    }
    for name in ["rust", "rust-native"] {
        let folder = root.join(name);
        if folder.is_symlink() || !folder.is_dir() {
            errors.push(format!("product root must be a real directory: {name}"));
            continue;
        }
        scan_product(root, &folder, &folder, &mut errors)?;
    }
    for name in TOOLING {
        let folder = root.join(name);
        match fs::symlink_metadata(&folder) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                // Product-root tooling links already have the original policy errors.
                if ![
                    "rust/tests",
                    "rust/scripts",
                    "rust-native/tests",
                    "rust-native/scripts",
                ]
                .contains(&name)
                    || !metadata.is_symlink()
                {
                    errors.push(format!(
                        "tooling source root must be a real directory: {name}"
                    ));
                }
            }
            Ok(_) => scan_tooling(root, &folder, &mut errors)?,
        }
    }
    Ok(
        json!({"ok":errors.is_empty(), "policy":"Rust-only new product code", "frozenLegacyFiles":expected.len(), "rustRuntimeFiles":count,
        "lockSHA256":digest(&lock_path)?, "errors":errors, "productionReady":false}),
    )
}
fn verify(root: &Path) -> Value {
    checked(root).unwrap_or_else(|error| json!({"ok":false, "errors":[format!("Rust-only policy: {error}")], "productionReady":false}))
}
fn main() {
    use std::os::unix::ffi::OsStringExt;
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("rust-only-policy [--root ROOT]\nRead-only Rust product policy and frozen legacy reference check.");
            return;
        }
        if arg == "--root" {
            root = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--root requires a path");
                std::process::exit(2)
            });
        } else if let Some(path) =
            std::os::unix::ffi::OsStrExt::as_bytes(arg.as_os_str()).strip_prefix(b"--root=")
        {
            root = std::ffi::OsString::from_vec(path.to_vec()).into();
        } else {
            eprintln!("unrecognized argument");
            std::process::exit(2);
        }
    }
    let result = verify(&root);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    if result["ok"] != true {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    #[test]
    fn unrepresentable_filename_cannot_collide_with_frozen_replacement_character() {
        let root = Path::new("/owned");
        let invalid = root
            .join("Sources")
            .join(std::ffi::OsString::from_vec(b"\xff.swift".to_vec()));
        assert!(relative(root, &invalid).is_err());
        assert_eq!(
            relative(root, &root.join("Sources/�.swift")).unwrap(),
            "Sources/�.swift"
        );
    }
}
