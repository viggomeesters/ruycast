//! Read-only policy for Rust executable source and retired Swift inputs.
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const RETIRED: [&str; 4] = [
    "Sources",
    "Package.swift",
    "rust-native/swift-reference-lock.json",
    "Tests/LauncherCoreTests",
];
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
const OUTPUT_DIRS: [&str; 5] = [
    ".build",
    ".dist",
    "rust/target",
    "rust-native/target",
    "bench/f1-runner/target",
];
const METADATA: [&str; 7] = [
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "LICENSE",
    "LICENSE.md",
    "AGENTS.md",
    ".gitignore",
];

struct Scan {
    runtime_files: [usize; RUNTIME.len()],
    errors: Vec<String>,
}

impl Scan {
    fn walk(&mut self, root: &Path, folder: &Path) -> io::Result<()> {
        let mut paths = fs::read_dir(folder)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            let relative = path.strip_prefix(root).unwrap();
            let name = relative
                .to_str()
                .ok_or_else(|| io::Error::other("filesystem path is not UTF-8"))?;
            if RETIRED.contains(&name) {
                self.errors.push(format!("retired Swift input: {name}"));
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if name == ".git" && (metadata.is_dir() || metadata.is_file()) {
                continue;
            }
            if OUTPUT_DIRS.contains(&name) && metadata.is_dir() {
                continue;
            }
            let runtime = RUNTIME
                .iter()
                .position(|prefix| relative.starts_with(prefix));
            let scope = if runtime.is_some() {
                "runtime"
            } else if TOOLING.iter().any(|prefix| relative.starts_with(prefix)) {
                "tooling"
            } else if relative.starts_with("rust") || relative.starts_with("rust-native") {
                "product"
            } else {
                "repository"
            };
            if metadata.is_symlink() {
                self.errors.push(format!("{scope} source symlink: {name}"));
            } else if metadata.is_dir() {
                self.walk(root, &path)?;
            } else if !metadata.is_file() {
                self.errors
                    .push(format!("nonregular {scope} input: {name}"));
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                if let Some(index) = runtime {
                    self.runtime_files[index] += 1;
                }
            } else if metadata.permissions().mode() & 0o111 != 0
                || !data_file(relative, name, scope)
                || has_shebang(&path)?
            {
                self.errors
                    .push(format!("non-Rust/unknown {scope} input: {name}"));
            }
        }
        Ok(())
    }
}

fn data_file(path: &Path, name: &str, scope: &str) -> bool {
    let filename = path.file_name().unwrap().to_str().unwrap();
    if filename == ".DS_Store" {
        return true;
    }
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match scope {
        "runtime" => false,
        "tooling" => METADATA.contains(&filename) || ["json", "md"].contains(&extension),
        "product" => {
            let resource = path.components().nth(1).is_some_and(|part| {
                ["assets", "resources", "docs", ".cargo"]
                    .iter()
                    .any(|dir| part.as_os_str() == *dir)
            });
            METADATA.contains(&filename)
                || (resource
                    && ["json", "toml", "plist", "png", "svg", "icns", "md", "txt"]
                        .contains(&extension))
        }
        _ => {
            METADATA.contains(&name)
                || ["PUBLIC-SOURCE.json", "rust-toolchain.toml"].contains(&name)
                || (path.parent() == Some(Path::new("")) && extension == "md")
                || (path.starts_with(".github") && extension == "md")
                || (path.starts_with("assets") && ["png", "svg", "icns"].contains(&extension))
                || (path.starts_with("config") && extension == "json")
                || (path.starts_with("docs") && ["json", "md", "png", "svg"].contains(&extension))
        }
    }
}

fn has_shebang(path: &Path) -> io::Result<bool> {
    let mut header = [0; 2];
    match fs::File::open(path)?.read_exact(&mut header) {
        Ok(()) => Ok(header == *b"#!"),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

fn result(runtime_files: usize, errors: Vec<String>) -> Value {
    json!({
        "schema": "ruycast.rust-only-source-policy.v1",
        "ok": errors.is_empty(),
        "policy": "Rust-only executable source",
        "rustRuntimeFiles": runtime_files,
        "errors": errors,
        "productionReady": false,
    })
}

fn checked(root: &Path) -> io::Result<Value> {
    root.to_str()
        .ok_or_else(|| io::Error::other("filesystem path is not UTF-8"))?;
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("repository root must be a real directory"));
    }
    let mut scan = Scan {
        runtime_files: [0; RUNTIME.len()],
        errors: Vec::new(),
    };
    scan.walk(root, root)?;
    for (name, count) in RUNTIME.into_iter().zip(scan.runtime_files) {
        if count == 0 {
            scan.errors.push(format!(
                "runtime source root must contain Rust source: {name}"
            ));
        }
    }
    Ok(result(scan.runtime_files.iter().sum(), scan.errors))
}

fn verify(root: &Path) -> Value {
    checked(root).unwrap_or_else(|error| result(0, vec![format!("Rust-only policy: {error}")]))
}

fn main() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!(
                "rust-only-policy [--root ROOT]\nCheck repository source for non-Rust executable and retired Swift inputs."
            );
            return;
        }
        if arg == "--root" {
            root = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--root requires a path");
                std::process::exit(2)
            });
        } else if let Some(path) = arg.as_os_str().as_bytes().strip_prefix(b"--root=") {
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
