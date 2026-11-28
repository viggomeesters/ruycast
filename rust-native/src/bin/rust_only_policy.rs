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
                if runtime.is_some() || scope == "tooling" || scope == "product" {
                    let source = fs::read_to_string(&path)?;
                    let syntax = syn::parse_file(&source).map_err(|error| {
                        io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {error}"))
                    })?;
                    let mut check = InlineInterpreter::default();
                    syn::visit::Visit::visit_file(&mut check, &syntax);
                    if check.found {
                        self.errors.push(format!(
                            "repository-owned inline interpreter program: {name}"
                        ));
                    }
                }
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

#[derive(Default)]
struct InlineInterpreter {
    found: bool,
}

impl<'ast> syn::visit::Visit<'ast> for InlineInterpreter {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if !self.found {
            self.found = inline_interpreter_call(node);
            if !self.found {
                syn::visit::visit_expr_method_call(self, node);
            }
        }
    }
}

// This checks direct Command::new builder chains, not aliases, variable data flow,
// arbitrary macro expansion, or the behavior of user-configured commands.
fn inline_interpreter_call(mut node: &syn::ExprMethodCall) -> bool {
    let mut groups = Vec::new();
    let call = loop {
        groups.push(node);
        match node.receiver.as_ref() {
            syn::Expr::MethodCall(parent) => node = parent,
            syn::Expr::Call(call) => break call,
            _ => return false,
        }
    };
    let syn::Expr::Path(constructor) = call.func.as_ref() else {
        return false;
    };
    let mut names = constructor.path.segments.iter().rev().map(|s| &s.ident);
    if names.next().is_none_or(|name| name != "new")
        || names.next().is_none_or(|name| name != "Command")
        || call.args.len() != 1
    {
        return false;
    }
    let Some(program) = literal_string(&call.args[0]) else {
        return false;
    };
    let executable = program.rsplit('/').next().unwrap_or_default();
    let flags: &[&str] = match executable {
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "python" | "python2" | "python3" => {
            &["-c"]
        }
        name if name
            .strip_prefix("python3.")
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) =>
        {
            &["-c"]
        }
        "osascript" | "perl" | "ruby" | "swift" => &["-e"],
        "node" | "nodejs" => &["-e", "--eval", "-p", "--print"],
        _ => return false,
    };
    let mut pending = false;
    for method in groups.into_iter().rev() {
        if method.args.len() != 1 {
            continue;
        }
        let expression = &method.args[0];
        let args: Box<dyn Iterator<Item = &syn::Expr>> = if method.method == "arg" {
            Box::new(std::iter::once(expression))
        } else if method.method == "args" {
            match expression {
                syn::Expr::Array(array) => Box::new(array.elems.iter()),
                syn::Expr::Reference(reference)
                    if matches!(&*reference.expr, syn::Expr::Array(_)) =>
                {
                    let syn::Expr::Array(array) = &*reference.expr else {
                        unreachable!()
                    };
                    Box::new(array.elems.iter())
                }
                _ => {
                    pending = false;
                    continue;
                }
            }
        } else {
            continue;
        };
        for arg in args {
            if pending && (literal_string(arg).is_some() || string_builder(arg, "format")) {
                return true;
            }
            pending = literal_string(arg).is_some_and(|value| flags.contains(&value.as_str()));
        }
    }
    false
}

fn string_builder(expr: &syn::Expr, name: &str) -> bool {
    matches!(expr, syn::Expr::Macro(mac) if mac.mac.path.is_ident(name))
}

fn literal_string(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(value) => Some(value.value()),
            _ => None,
        },
        syn::Expr::Paren(paren) => literal_string(&paren.expr),
        syn::Expr::Macro(mac) if mac.mac.path.is_ident("concat") => {
            let parts = mac
                .mac
                .parse_body_with(
                    syn::punctuated::Punctuated::<syn::LitStr, syn::Token![,]>::parse_terminated,
                )
                .ok()?;
            Some(parts.iter().map(syn::LitStr::value).collect())
        }
        _ => None,
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
