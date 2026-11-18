use ruycast_native::artifact_lease::Lease;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
mod fs;
mod preview;
mod profile;
mod receipt;
mod signing;
#[cfg(test)]
mod tests;
mod tools;
mod transaction;
type Result<T> = std::result::Result<T, String>;
const VERSION: &str = rustcast_core::product_version::CURRENT;
const NEW_ID: &str = "nl.viggomeesters.ruycast";
const OLD_ID: &str = "nl.viggomeesters.rustcast";
fn err(value: impl std::fmt::Display) -> String {
    value.to_string()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub struct Options {
    repo: PathBuf,
    home: PathBuf,
    apps: PathBuf,
    signer: PathBuf,
    profile: Option<PathBuf>,
    config: Option<String>,
    adapter: Option<PathBuf>,
}
fn environment(key: &str) -> Result<Option<String>> {
    std::env::var_os(key)
        .map(|v| v.into_string().map_err(|_| format!("{key} must be UTF-8.")))
        .transpose()
}
fn alias(modern: &str, legacy: &str) -> Result<Option<String>> {
    match environment(modern)? {
        Some(v) => Ok(Some(v)),
        None => environment(legacy),
    }
}
fn compiler_command(root: &Path, operation: &str) -> Result<tools::Call> {
    let cargo = std::env::var_os("PATH")
        .and_then(|v| {
            std::env::split_paths(&v)
                .map(|p| p.join("cargo"))
                .find(|p| p.is_absolute() && p.is_file())
        })
        .ok_or("Cargo is required to build the existing Rust build tool.")?;
    Ok(tools::Call::new(
        cargo,
        [
            "run",
            "--locked",
            "--offline",
            "--release",
            "--manifest-path",
            &root.join("rust-native/Cargo.toml").display().to_string(),
            "--target-dir",
            &root.join(".build/installer-tools").display().to_string(),
            "--bin",
            "ruycast-build",
            "--",
            operation,
            "--repo",
            &root.display().to_string(),
        ],
    ))
}
fn compiler_call(root: &Path, home: &Path, lease: &Lease, operation: &str) -> Result<tools::Call> {
    let mut command = compiler_command(root, operation)?.cwd(root);
    command.env = std::env::vars_os()
        .map(|(k, v)| {
            Ok((
                k.into_string()
                    .map_err(|_| "Invalid build environment key.")?,
                v.into_string()
                    .map_err(|_| "Invalid build environment value.")?,
            ))
        })
        .collect::<Result<_>>()?;
    command
        .env
        .insert("HOME".into(), home.display().to_string());
    command.env.extend(lease.environment()?);
    command.timeout = std::time::Duration::from_secs(1800);
    Ok(command)
}
fn install_pipeline(
    options: &Options,
    build: bool,
    lease: &Lease,
    runner: &mut dyn tools::Runner,
) -> Result<Value> {
    lease.validate()?;
    if build {
        signing::prepare(&options.signer, &options.repo, &options.home, runner)?;
        tools::checked(
            runner,
            compiler_call(&options.repo, &options.home, lease, "build")?,
        )?;
        lease.validate()?;
    }
    transaction::install(options, lease, runner)
}
pub fn cli() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "install-preview") {
        return preview::install(&args[1..], false);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "install-preview-reenroll")
    {
        return preview::install(&args[1..], true);
    }
    if args.first().is_some_and(|v| v == "signing") {
        return signing_cli(args[1..].to_vec());
    }
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("ruycast-install install [--build] [--repo ABSOLUTE] [--archive FILE --sha256 TRUSTED_SHA]\nruycast-install prepare|signing-status [--repo ABSOLUTE] [--signing-root ABSOLUTE]\nruycast-install verify-download --archive ABSOLUTE --sha256 TRUSTED_SHA\nruycast-install install-preview ABSOLUTE_APP\nruycast-install install-preview-reenroll ABSOLUTE_APP");
        return Ok(());
    }
    let op = args.remove(0);
    let build = if let Some(i) = args.iter().position(|a| a == "--build") {
        args.remove(i);
        true
    } else {
        false
    };
    let mut flags = BTreeMap::new();
    if !args.len().is_multiple_of(2) {
        return Err("Expected named option/value pairs.".into());
    }
    for pair in args.chunks_exact(2) {
        if !["--repo", "--signing-root", "--archive", "--sha256"].contains(&pair[0].as_str())
            || flags.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err("Unknown or repeated installer option.".into());
        }
    }
    if op == "verify-download" {
        if build || flags.len() != 2 {
            return Err("verify-download requires archive and trusted SHA only.".into());
        }
        let value = verify_download(
            Path::new(flags.get("--archive").ok_or("Missing archive.")?),
            flags.get("--sha256").ok_or("Missing trusted SHA.")?,
        )?;
        println!("{value}");
        return Ok(());
    }
    if !["install", "prepare", "signing-status"].contains(&op.as_str()) {
        return Err("Unknown installer command.".into());
    }
    if let Some(archive) = flags.get("--archive") {
        verify_download(
            Path::new(archive),
            flags
                .get("--sha256")
                .ok_or("An archive requires its trusted SHA.")?,
        )?;
    } else if flags.contains_key("--sha256") {
        return Err("A trusted SHA requires an archive.".into());
    }
    let repo = fs::absolute(
        &flags
            .get("--repo")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir().map_err(err)?),
    )?;
    let home = match std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        Some(v) => fs::absolute(&PathBuf::from(v))?,
        None => fs::absolute(&PathBuf::from(
            objc2_foundation::NSHomeDirectory().to_string(),
        ))?,
    };
    let apps = alias("RUYCAST_APP_DIR", "RUSTCAST_APP_DIR")?
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let p = std::ffi::CString::new("/Applications".as_bytes()).unwrap();
            if unsafe { libc::access(p.as_ptr(), libc::W_OK) } == 0 {
                PathBuf::from("/Applications")
            } else {
                home.join("Applications")
            }
        });
    let profile = alias("RUYCAST_HOME", "RUSTCAST_HOME")?
        .map(|p| {
            if p.is_empty() {
                Err("An explicitly empty profile is invalid.".into())
            } else {
                fs::absolute(&if Path::new(&p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    repo.join(p)
                })
            }
        })
        .transpose()?;
    let signer = flags
        .get("--signing-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share/ruycast/signing"));
    let options = Options {
        repo,
        home,
        apps: fs::absolute(&apps)?,
        signer: fs::absolute(&signer)?,
        profile,
        config: alias("RUYCAST_CONFIG_DIR", "RUSTCAST_CONFIG_DIR")?,
        adapter: environment("RUYCAST_PERSONAL_BRIDGE")?.map(PathBuf::from),
    };
    let mut runner = tools::System;
    let value = match op.as_str() {
        "prepare" => signing::prepare(&options.signer, &options.repo, &options.home, &mut runner)?,
        "signing-status" => signing::load(&options.signer, &options.repo)?,
        _ => {
            let lease = Lease::acquire(&options.repo)?;
            install_pipeline(&options, build, &lease, &mut runner)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value).map_err(err)?);
    Ok(())
}
const SIGNING_USAGE: &str = "ruycast-install signing prepare|status|check|sign [APP] [--root SIGNING_ROOT] [--repo ABSOLUTE_REPOSITORY]";
fn signing_cli(args: Vec<String>) -> Result<()> {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{SIGNING_USAGE}");
        return Ok(());
    }
    let home = match std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        Some(v) => fs::absolute(&PathBuf::from(v))?,
        None => fs::absolute(&PathBuf::from(objc2_foundation::NSHomeDirectory().to_string()))?,
    };
    let cwd = std::env::current_dir().map_err(err)?;
    let value = signing_cli_dispatch(args, &home, &cwd, || {
        // This closure is reached only by explicit check, after identity load.
        let base = std::env::var_os("VIGGO_TEST_ROOT").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        // Use an existing parent; even directory discovery must not modify
        // signer state when VIGGO_TEST_ROOT points inside it.
        let staging = std::fs::canonicalize(base)
            .map_err(|_| "Signing check work directory must already exist.".to_string())?;
        let compiler = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path).map(|entry| entry.join("rustc"))
                .find(|path| path.is_absolute() && path.is_file())
        }).ok_or("Rust compiler is required for signing check.")?;
        Ok((staging, compiler))
    }, &mut tools::System)?;
    match value {
        Some(value) => println!("{}", serde_json::to_string_pretty(&value).map_err(err)?),
        None => println!("{SIGNING_USAGE}"),
    }
    Ok(())
}
fn signing_cli_dispatch(
    mut args: Vec<String>,
    home: &Path,
    cwd: &Path,
    check_context: impl FnOnce() -> Result<(PathBuf, PathBuf)>,
    runner: &mut dyn tools::Runner,
) -> Result<Option<Value>> {
    if args.is_empty() || args.iter().any(|v| v == "--help" || v == "-h") {
        return Ok(None);
    }
    let mut root = None;
    let mut repo = None;
    while let Some(i) = args.iter().position(|v| v == "--root" || v == "--repo") {
        let flag = args.remove(i);
        if i >= args.len() {
            return Err("Missing signing option value.".into());
        }
        let value = args.remove(i);
        let target = if flag == "--root" {
            &mut root
        } else {
            &mut repo
        };
        if target.replace(value).is_some() {
            return Err("Repeated signing option.".into());
        }
    }
    if args.is_empty() {
        return Err("Missing signing command.".into());
    }
    let command = args.remove(0);
    if !["prepare", "status", "check", "sign"].contains(&command.as_str())
        || args.len() != usize::from(command == "sign")
    {
        return Err("Expected prepare|status|check or sign APP.".into());
    }
    let root = match root {
        None => home.join(".local/share/ruycast/signing"),
        Some(value) => {
            let expanded = if value == "~" {
                home.to_path_buf()
            } else if let Some(tail) = value.strip_prefix("~/") {
                home.join(tail)
            } else {
                PathBuf::from(value)
            };
            fs::absolute(&if expanded.is_absolute() {
                expanded
            } else {
                cwd.join(expanded)
            })?
        }
    };
    let repo = repo.map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    fs::absolute(&repo)?;
    let value = match command.as_str() {
        "prepare" => signing::prepare(&root, &repo, home, runner)?,
        "status" => signing::load(&root, &repo)?,
        "check" => {
            // No temporary-directory or compiler discovery effects on refusal.
            signing::load(&root, &repo)?;
            let (staging, compiler) = check_context()?;
            signing::check(&root, &repo, home, &staging, &compiler, runner)?
        }
        _ => {
            let app = PathBuf::from(&args[0]);
            signing::sign(
                &fs::absolute(&if app.is_absolute() {
                    app
                } else {
                    cwd.join(app)
                })?,
                &root,
                &repo,
                home,
                runner,
            )?
        }
    };
    Ok(Some(value))
}
pub fn verify_download(archive: &Path, expected: &str) -> Result<Value> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("A trusted 64-digit SHA-256 checksum is required.".into());
    }
    let archive = fs::absolute(archive)?;
    let actual = fs::hash(&archive)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("Downloaded source checksum mismatch; nothing installed.".into());
    }
    Ok(json!({"ok":true,"archive":archive,"sha256":actual,"installedOrPublished":false}))
}
