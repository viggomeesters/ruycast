//! Rust-only development build and unsigned app staging. This does not install,
//! sign or accept an incomplete product. Content-bound receipts are necessary;
//! the final delivery gate must additionally prove complete product parity.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
};
#[path = "ruycast-build/candidate.rs"]
mod candidate;
#[path = "ruycast-build/compat.rs"]
mod compat;
#[path = "ruycast-build/macho.rs"]
mod macho;
#[path = "ruycast-build/oracles.rs"]
mod oracles;
#[path = "ruycast-build/preparation.rs"]
mod preparation;
#[path = "ruycast-build/process.rs"]
mod process;
#[path = "ruycast-build/public_source.rs"]
mod public_source;
#[path = "ruycast-build/validation.rs"]
mod validation;
#[path = "ruycast-build/version.rs"]
mod version;

type Result<T> = std::result::Result<T, String>;
const SCHEMA: &str = "ruycast.rust-development-build.v4";
const OUTPUT: &str = ".build/full-rust-release";
static BUILD_EXIT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
const USAGE: &str = "ruycast-build build|verify --repo ABSOLUTE_REPOSITORY [--configuration debug|release]\nruycast-build version|fingerprint|validation-plan --repo ABSOLUTE_REPOSITORY\nruycast-build stage --repo ABSOLUTE_REPOSITORY --build POSITIVE_NUMBER\nruycast-build verify-candidate --repo ABSOLUTE_REPOSITORY --candidate ABSOLUTE_DIRECTORY\nruycast-build check-native|pure-core-oracle|verify-pure-core-oracle --repo ABSOLUTE_REPOSITORY\nruycast-build gate-inventory --repo ABSOLUTE_REPOSITORY --pipeline check|settings\nruycast-build compat ENTRY --repo ABSOLUTE_REPOSITORY -- [compatibility arguments]";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema: String,
    product_version: String,
    configuration: String,
    release_preparation: Option<PreparationBinding>,
    target: String,
    sources: BTreeMap<String, String>,
    artifacts: BTreeMap<String, String>,
    linked_libraries: BTreeMap<String, Vec<String>>,
    toolchains: BTreeMap<String, String>,
    commands: Vec<Vec<String>>,
    environment_overrides: BTreeMap<String, String>,
    production_accepted: bool,
    installed_or_published: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparationBinding {
    path: String,
    sha256: String,
}
fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}
fn regular_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = regular_file(path)?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).map_err(error)?;
    Ok(bytes)
}
fn regular_file(path: &Path) -> Result<File> {
    // O_NOFOLLOW protects the final component; directory components must not
    // redirect an allowlisted source or artifact through a linked tree either.
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        if fs::symlink_metadata(&prefix)
            .map_err(error)?
            .file_type()
            .is_symlink()
        {
            return Err(format!("Linked build input: {}", prefix.display()));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(error)?;
    if !file.metadata().map_err(error)?.is_file() {
        return Err(format!("Expected regular file: {}", path.display()));
    }
    Ok(file)
}
fn digest(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(regular_bytes(path)?)))
}
fn source_tree(root: &Path, path: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if meta.file_type().is_symlink() {
        return Err(format!("Linked build input: {}", path.display()));
    }
    if meta.is_dir() {
        for child in fs::read_dir(path).map_err(error)? {
            source_tree(root, &child.map_err(error)?.path(), files)?;
        }
    } else if meta.is_file() {
        let name = path
            .strip_prefix(root)
            .map_err(error)?
            .to_str()
            .ok_or("Build input path must be UTF-8.")?
            .to_owned();
        files.insert(name, digest(path)?);
    } else {
        return Err(format!("Unsupported build input: {}", path.display()));
    }
    Ok(())
}
fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    // Required compilation inputs. No Swift reference tree or personal profile
    // is a production build input. Shared private_storage.rs is under rust/src.
    for name in [
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust-native/src",
        "config/default.json",
    ] {
        source_tree(root, &root.join(name), &mut files)?;
    }
    // Cargo configuration/toolchain/build scripts may alter either build.
    for name in [
        ".cargo",
        "rust/.cargo",
        "rust-native/.cargo",
        "rust-toolchain",
        "rust-toolchain.toml",
        "rust/build.rs",
        "rust-native/build.rs",
    ] {
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => source_tree(root, &root.join(name), &mut files)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
    }
    Ok(files)
}
fn output(root: &Path, executable: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(executable);
    command.args(args).current_dir(root);
    let result = process::capture(command, std::time::Duration::from_secs(30), 8 * 1024 * 1024)?;
    if !result.status.success() {
        return Err(format!(
            "{executable} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(error)
}
fn artifact_paths(target: &str) -> Result<[(String, &'static str); 2]> {
    artifact_paths_for(target, "release")
}
fn configuration_output(configuration: &str) -> Result<&'static str> {
    match configuration {
        "release" => Ok(OUTPUT),
        "debug" => Ok(".build/full-rust-debug"),
        _ => Err("Configuration must be debug or release.".into()),
    }
}
fn artifact_paths_for(target: &str, configuration: &str) -> Result<[(String, &'static str); 2]> {
    if !matches!(target, "aarch64-apple-darwin" | "x86_64-apple-darwin") {
        return Err("Rust app builds require a supported Darwin host target.".into());
    }
    let output = configuration_output(configuration)?;
    Ok([
        (
            format!("{output}/native/{target}/{configuration}/ruycast-native"),
            "host",
        ),
        (
            format!("{output}/core/{target}/{configuration}/ruycast"),
            "cli",
        ),
    ])
}
fn host_target(root: &Path) -> Result<String> {
    let details = output(root, "rustc", &["-vV"])?;
    let target = details
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("Rust compiler did not report its host target.")?;
    artifact_paths(target)?;
    Ok(target.into())
}
fn libraries(root: &Path, artifact: &str) -> Result<Vec<String>> {
    // The system otool reads load commands; it never starts the new host/CLI.
    let text = output(root, "/usr/bin/otool", &["-L", artifact])?;
    let rows = text
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(format!("No Mach-O load dependencies: {artifact}"));
    }
    if rows
        .iter()
        .any(|line| line.contains("/libswift") || line.contains("/swift/"))
    {
        return Err(format!(
            "Rust executable directly links a Swift runtime: {artifact}"
        ));
    }
    Ok(rows)
}
fn product_version(root: &Path, artifact: &str, component: &str) -> Result<String> {
    // Both Rust entrypoints handle this before resolving a profile or creating
    // native services. Never use status/start or invoke an arbitrary candidate.
    let mut command = Command::new(root.join(artifact));
    command.arg("--version").env_clear();
    let result = process::capture(command, std::time::Duration::from_secs(10), 65536)?;
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).map_err(error)?;
    if !result.status.success()
        || value["ok"] != true
        || value["result"]["implementation"] != "rust"
        || value["result"]["component"] != component
        || value["result"]["version"] != rustcast_core::product_version::CURRENT
    {
        return Err(format!("Rust product metadata mismatch: {artifact}"));
    }
    Ok(rustcast_core::product_version::CURRENT.into())
}
type BuildLock = ruycast_native::artifact_lease::Lease;
fn build(root: &Path, configuration: &str) -> Result<Receipt> {
    let lock = BuildLock::acquire(root)?;
    build_under_lease(root, configuration, &lock)
}
fn build_under_lease(root: &Path, configuration: &str, lock: &BuildLock) -> Result<Receipt> {
    lock.validate()?;
    let output_directory = configuration_output(configuration)?;
    let prior = root.join(output_directory).join("development-build.json");
    match fs::symlink_metadata(&prior) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            fs::remove_file(&prior).map_err(error)?
        }
        Ok(_) => return Err("Invalid existing development receipt.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(e)),
    }
    preparation::recover(root)?;
    let before = inventory(root)?;
    let target = host_target(root)?;
    let core_target = format!("{output_directory}/core");
    let native_target = format!("{output_directory}/native");
    let mut commands = [
        vec![
            "cargo",
            "rustc",
            "--manifest-path",
            "rust/Cargo.toml",
            "--target-dir",
            &core_target,
            "--release",
            "--target",
            &target,
            "--locked",
            "--offline",
            "--bin",
            "ruycast",
            "--",
            "-C",
            "opt-level=s",
        ],
        vec![
            "cargo",
            "build",
            "--manifest-path",
            "rust-native/Cargo.toml",
            "--target-dir",
            &native_target,
            "--release",
            "--target",
            &target,
            "--locked",
            "--offline",
            "--bin",
            "ruycast-native",
        ],
    ];
    if configuration == "debug" {
        for command in &mut commands {
            command.retain(|value| *value != "--release");
        }
        let core = &mut commands[0];
        core.truncate(
            core.iter()
                .position(|value| *value == "--")
                .ok_or("Missing Rust compiler arguments.")?,
        );
    }
    let mut toolchains = BTreeMap::new();
    for executable in ["cargo", "rustc"] {
        toolchains.insert(
            executable.into(),
            output(root, executable, &["--version"])?.trim().into(),
        );
    }
    let mut environment_overrides: BTreeMap<String, String> = [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_PROFILE_RELEASE_LTO",
        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL",
        "CARGO_PROFILE_RELEASE_PANIC",
        "CARGO_PROFILE_RELEASE_DEBUG",
        "CARGO_INCREMENTAL",
        "SDKROOT",
        "DEVELOPER_DIR",
        "MACOSX_DEPLOYMENT_TARGET",
        "CARGO_BUILD_TARGET",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
    .collect();
    if configuration == "release" {
        environment_overrides.insert("CARGO_PROFILE_RELEASE_DEBUG".into(), "1".into());
    }
    for command in &commands {
        let mut child = Command::new(command[0]);
        child.args(&command[1..]).current_dir(root);
        if configuration == "release" {
            child.env("CARGO_PROFILE_RELEASE_DEBUG", "1");
        }
        lock.apply_to_child(&mut child)?;
        let captured =
            process::capture(child, std::time::Duration::from_secs(900), 32 * 1024 * 1024)?;
        use std::io::Write;
        std::io::stderr()
            .write_all(&captured.stderr)
            .map_err(error)?;
        let status = captured.status;
        if !status.success() {
            BUILD_EXIT.store(
                status
                    .code()
                    .filter(|code| (1..=255).contains(code))
                    .unwrap_or(1),
                std::sync::atomic::Ordering::Relaxed,
            );
            return Err(format!(
                "Rust release build failed ({status}); no new receipt published."
            ));
        }
    }
    if inventory(root)? != before {
        return Err("Sources changed during the Rust build; no new receipt published.".into());
    }
    let release_preparation = if configuration == "release" {
        Some(preparation::prepare(root, &target)?)
    } else {
        None
    };
    let mut artifacts = BTreeMap::new();
    let mut linked_libraries = BTreeMap::new();
    for (name, _) in artifact_paths_for(&target, configuration)? {
        artifacts.insert(name.clone(), digest(&root.join(&name))?);
        linked_libraries.insert(name.clone(), libraries(root, &name)?);
    }
    let receipt = Receipt {
        schema: SCHEMA.into(),
        product_version: product_version(
            root,
            &artifact_paths_for(&target, configuration)?[0].0,
            "host",
        )?,
        configuration: configuration.into(),
        release_preparation,
        target: target.clone(),
        sources: before,
        artifacts,
        linked_libraries,
        toolchains,
        commands: commands
            .iter()
            .map(|command| command.iter().map(|s| (*s).to_owned()).collect())
            .collect(),
        environment_overrides,
        production_accepted: false,
        installed_or_published: false,
    };
    // Last source/artifact readback prevents a mixed candidate from gaining a
    // valid receipt, even when another process changes an input after cargo.
    lock.validate()?;
    verify_value(root, &receipt)?;
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(error)?;
    ruycast_native::private_storage::write(
        &root.join(output_directory).join("development-build.json"),
        &bytes,
        true,
    )?;
    lock.validate()?;
    Ok(receipt)
}
fn verify_value(root: &Path, receipt: &Receipt) -> Result<()> {
    preparation::require_settled(root)?;
    configuration_output(&receipt.configuration)?;
    let expected_preparation = if receipt.configuration == "release" {
        Some(preparation::binding(root, &receipt.target)?)
    } else {
        None
    };
    if receipt.schema != SCHEMA
        || receipt.product_version != rustcast_core::product_version::CURRENT
        || receipt.product_version != version::read(root)?
        || receipt.release_preparation != expected_preparation
        || receipt.target != host_target(root)?
        || receipt.production_accepted
        || receipt.installed_or_published
        || receipt.sources != inventory(root)?
        || receipt.artifacts.len() != 2
        || receipt.linked_libraries.len() != 2
    {
        return Err("Invalid or stale development Rust build receipt.".into());
    }
    for (name, component) in artifact_paths_for(&receipt.target, &receipt.configuration)? {
        if receipt.artifacts.get(&name) != Some(&digest(&root.join(&name))?)
            || receipt.linked_libraries.get(&name) != Some(&libraries(root, &name)?)
        {
            return Err(format!("Built Rust artifact changed: {name}"));
        }
        product_version(root, &name, component)?;
    }
    preparation::require_settled(root)?;
    Ok(())
}
fn verify(root: &Path, configuration: &str) -> Result<Receipt> {
    let lock = BuildLock::acquire(root)?;
    verify_under_lease(root, configuration, &lock)
}
fn verify_under_lease(root: &Path, configuration: &str, lock: &BuildLock) -> Result<Receipt> {
    lock.validate()?;
    let receipt: Receipt = serde_json::from_slice(&regular_bytes(
        &root
            .join(configuration_output(configuration)?)
            .join("development-build.json"),
    )?)
    .map_err(error)?;
    if receipt.configuration != configuration {
        return Err("Receipt configuration does not match requested build.".into());
    }
    verify_value(root, &receipt)?;
    lock.validate()?;
    Ok(receipt)
}
fn run() -> Result<()> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|value| value == "compat") {
        let value = compat::run(args)?;
        if let Some(text) = value.as_str() {
            println!("{text}");
        } else {
            println!("{value}");
        }
        return Ok(());
    }
    let configuration =
        compat::option(&mut args, &["--configuration", "-c"])?.unwrap_or("release".into());
    configuration_output(&configuration)?;
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.len() == 5
        && args[0] == "gate-inventory"
        && args[1] == "--repo"
        && args[3] == "--pipeline"
    {
        if !Path::new(&args[2]).is_absolute() {
            return Err("Choose an absolute repository path.".into());
        }
        println!("{}", oracles::inventory(&args[4])?);
        return Ok(());
    }
    if args.len() == 3
        && args[1] == "--repo"
        && matches!(
            args[0].as_str(),
            "pure-core-oracle"
                | "verify-pure-core-oracle"
                | "pre-main-oracles"
                | "verify-pre-main-oracles"
        )
    {
        if !Path::new(&args[2]).is_absolute() {
            return Err("Choose an absolute repository path.".into());
        }
        let root = Path::new(&args[2]).canonicalize().map_err(error)?;
        let lease = BuildLock::acquire(&root)?;
        let value = match args[0].as_str() {
            "pure-core-oracle" => oracles::core(&root, &lease)?,
            "verify-pure-core-oracle" => oracles::verify_core(&root, &lease)?,
            "pre-main-oracles" => oracles::headless(&root, &lease)?,
            _ => oracles::verify_headless(&root, &lease)?,
        };
        println!("{value}");
        return Ok(());
    }
    if args.len() == 3
        && args[1] == "--repo"
        && matches!(args[0].as_str(), "validation-plan" | "check-native")
    {
        if !Path::new(&args[2]).is_absolute() {
            return Err("Choose an absolute repository path.".into());
        }
        let root = Path::new(&args[2]).canonicalize().map_err(error)?;
        let value = if args[0] == "validation-plan" {
            validation::plan(&host_target(&root)?)?
        } else {
            let lock = BuildLock::acquire(&root)?;
            validation::run(&root, &lock)?
        };
        println!("{value}");
        return Ok(());
    }
    if args.len() == 3
        && args[1] == "--repo"
        && matches!(args[0].as_str(), "version" | "fingerprint")
    {
        if !Path::new(&args[2]).is_absolute() {
            return Err("Choose an absolute repository path.".into());
        }
        let root = Path::new(&args[2]).canonicalize().map_err(error)?;
        let value = if args[0] == "version" {
            serde_json::json!({"ok":true,"version":version::read(&root)?,"source":"rust/src/product_version.rs"})
        } else {
            serde_json::json!({"ok":true,"fingerprint":format!("{:x}",Sha256::digest(serde_json::to_vec(&inventory(&root)?).map_err(error)?))})
        };
        println!("{value}");
        return Ok(());
    }
    if args.len() == 5
        && args[1] == "--repo"
        && matches!(args[0].as_str(), "stage" | "verify-candidate")
    {
        let path = PathBuf::from(&args[2]);
        if !path.is_absolute() {
            return Err("Choose an absolute repository path.".into());
        }
        let root = fs::canonicalize(path).map_err(error)?;
        let destination = if args[0] == "stage" && args[3] == "--build" {
            let build = args[4].parse::<u64>().map_err(error)?;
            candidate::stage(&root, build)?
        } else if args[0] == "verify-candidate" && args[3] == "--candidate" {
            PathBuf::from(&args[4])
        } else {
            return Err("Expected stage --repo ABSOLUTE --build NUMBER or verify-candidate --repo ABSOLUTE --candidate ABSOLUTE.".into());
        };
        println!("{}", candidate::verify(&root, &destination)?);
        return Ok(());
    }
    if args.len() != 3 || args[1] != "--repo" || !matches!(args[0].as_str(), "build" | "verify") {
        return Err(format!("Usage:\n{USAGE}"));
    }
    let path = PathBuf::from(&args[2]);
    if !path.is_absolute() {
        return Err("Choose an absolute repository path.".into());
    }
    let root = fs::canonicalize(path).map_err(error)?;
    let receipt = if args[0] == "build" {
        build(&root, &configuration)?
    } else {
        verify(&root, &configuration)?
    };
    println!(
        "{}",
        serde_json::json!({"ok":true,"schema":receipt.schema,"configuration":receipt.configuration,"releasePreparation":receipt.release_preparation,"target":receipt.target,"sources":receipt.sources.len(),"artifacts":receipt.artifacts,"receipt":root.join(configuration_output(&configuration)?).join("development-build.json"),"productionAccepted":false,"installedOrPublished":false})
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(BUILD_EXIT.load(std::sync::atomic::Ordering::Relaxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    #[ignore = "Explicit owned production-tool CLI entry; caller pins the just-built Rust artifact"]
    fn owned_producer_cli_entry() {
        let fixture = Fixture::compiler_pair();
        let executable = PathBuf::from(std::env::var_os("RUYCAST_OWNED_BUILD_PRODUCER").unwrap());
        let expected = std::env::var("RUYCAST_OWNED_BUILD_PRODUCER_SHA256").unwrap();
        assert_eq!(digest(&executable).unwrap(), expected);
        let invoke = |arguments: &[&str]| {
            let mut command = Command::new(&executable);
            command.args(arguments).current_dir(&fixture.0);
            for (key, _) in std::env::vars_os() {
                let text = key.to_string_lossy();
                if text.starts_with("RUYCAST_")
                    || text.starts_with("RUSTCAST_")
                    || text.starts_with("DYLD_")
                {
                    command.env_remove(key);
                }
            }
            let result = process::capture(
                command,
                std::time::Duration::from_secs(90),
                16 * 1024 * 1024,
            )
            .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
        };
        let root = fixture.0.to_str().unwrap();
        assert_eq!(
            invoke(&["version", "--repo", root])["version"],
            rustcast_core::product_version::CURRENT
        );
        assert!(!fixture.0.join(".build").exists());
        let produced = invoke(&["build", "--repo", root]);
        assert_eq!(produced["schema"], SCHEMA);
        assert_eq!(produced["configuration"], "release");
        assert!(produced["releasePreparation"]["sha256"]
            .as_str()
            .is_some_and(|value| value.len() == 64));
        assert_eq!(produced["productionAccepted"], false);
        let verified = invoke(&["verify", "--repo", root]);
        assert_eq!(verified["artifacts"], produced["artifacts"]);
        assert_eq!(
            verified["releasePreparation"],
            produced["releasePreparation"]
        );
        let legacy_debug = invoke(&["compat", "build-receipt", "--repo", root, "--", "build"]);
        assert_eq!(legacy_debug["configuration"], "debug");
        assert!(legacy_debug["releasePreparation"].is_null());
        let legacy_release = invoke(&[
            "compat",
            "build-receipt",
            "--repo",
            root,
            "--",
            "build",
            "--configuration=release",
        ]);
        assert_eq!(legacy_release["configuration"], "release");
        assert!(legacy_release["releasePreparation"].is_object());
        let candidate = invoke(&["stage", "--repo", root, "--build", "68"]);
        assert_eq!(candidate["ok"], true);
        assert_eq!(candidate["productionAccepted"], false);
        let directory = candidate["candidate"].as_str().unwrap();
        let verified_candidate =
            invoke(&["verify-candidate", "--repo", root, "--candidate", directory]);
        assert_eq!(verified_candidate["files"], candidate["files"]);
        let evidence = PathBuf::from(
            std::env::var_os("RUYCAST_OWNED_BUILD_EVIDENCE")
                .expect("Explicit new owned evidence destination is required"),
        );
        assert!(evidence.is_absolute());
        fs::create_dir(&evidence).unwrap();
        for (name, value) in [
            ("build-response.json", &produced),
            ("verify-response.json", &verified),
            ("legacy-debug-response.json", &legacy_debug),
            ("legacy-release-response.json", &legacy_release),
            ("candidate-response.json", &candidate),
            ("verify-candidate-response.json", &verified_candidate),
        ] {
            fs::write(
                evidence.join(name),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap();
        }
        let receipt_path = fixture.0.join(OUTPUT).join("development-build.json");
        let receipt: Receipt =
            serde_json::from_slice(&regular_bytes(&receipt_path).unwrap()).unwrap();
        let prep: preparation::Record = serde_json::from_slice(
            &regular_bytes(&fixture.0.join(OUTPUT).join("release-preparation.json")).unwrap(),
        )
        .unwrap();
        let mut paths = receipt
            .sources
            .keys()
            .chain(receipt.artifacts.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        paths.extend([
            format!("{OUTPUT}/development-build.json"),
            format!("{OUTPUT}/release-preparation.json"),
        ]);
        for row in prep.artifacts.values() {
            paths.insert(row.unstripped_path.clone());
            for name in row.d_sym_sha256.keys() {
                paths.insert(format!("{}/{name}", row.d_sym_path));
            }
        }
        let candidate_relative = Path::new(directory)
            .strip_prefix(&fixture.0)
            .unwrap()
            .to_str()
            .unwrap();
        paths.insert(format!("{candidate_relative}/candidate.json"));
        for name in candidate["files"].as_object().unwrap().keys() {
            paths.insert(format!("{candidate_relative}/Ruycast.app/{name}"));
        }
        for name in paths {
            let source = fixture.0.join(&name);
            let target = evidence.join("tree").join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(&source, &target).unwrap();
            assert_eq!(digest(&source).unwrap(), digest(&target).unwrap());
        }
        fs::write(evidence.join("SCOPE.md"),"Synthetic Rust metadata-only Cargo fixture. No real product host, profile, SDK, installation, signing or release acceptance. Original temporary root appears in raw responses; retained tree keeps identical relative source/artifact paths for read-only V4 revalidation.\n").unwrap();
        assert_eq!(digest(&executable).unwrap(), expected);
    }
    pub(super) struct Fixture(pub(super) PathBuf);
    impl Fixture {
        pub(super) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = PathBuf::from(
                std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                    .expect("Explicit owned fixture parent is required"),
            )
            .join("rust-build-receipt-fixtures")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(!root.exists());
            for name in [
                "rust/src/lib.rs",
                "rust/Cargo.toml",
                "rust/Cargo.lock",
                "rust-native/src/main.rs",
                "rust-native/Cargo.toml",
                "rust-native/Cargo.lock",
                "config/default.json",
            ] {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"owned fixture input").unwrap();
            }
            Self(root)
        }
        pub(super) fn compiler_pair() -> Self {
            let fixture = Self::new();
            fs::write(
                fixture.0.join("rust/src/product_version.rs"),
                format!(
                    "pub const CURRENT: &str = \"{}\";\n",
                    rustcast_core::product_version::CURRENT
                ),
            )
            .unwrap();
            for (directory, binary, component) in [
                ("rust", "ruycast", "cli"),
                ("rust-native", "ruycast-native", "host"),
            ] {
                fs::write(fixture.0.join(directory).join("Cargo.toml"), format!("[package]\nname=\"{binary}\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[profile.release]\npanic=\"unwind\"\n")).unwrap();
                fs::write(
                    fixture.0.join(directory).join("Cargo.lock"),
                    format!("version = 4\n[[package]]\nname = \"{binary}\"\nversion = \"0.0.0\"\n"),
                )
                .unwrap();
                let value = serde_json::json!({"ok":true,"result":{"implementation":"rust","component":component,"version":rustcast_core::product_version::CURRENT}}).to_string();
                let source = format!("fn main() {{ assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), [\"--version\"]); println!(\"{{}}\", {value:?}); }}\n");
                fs::write(fixture.0.join(directory).join("src/main.rs"), source).unwrap();
                if directory == "rust" {
                    fs::remove_file(fixture.0.join("rust/src/lib.rs")).unwrap();
                }
            }
            fixture
        }
    }
    #[test]
    fn cargo_run_dispatch_preserves_the_owned_rust_program_exit_code() {
        let fixture = Fixture::compiler_pair();
        fs::write(
            fixture.0.join("rust/src/main.rs"),
            b"fn main(){std::process::exit(7);}\n",
        )
        .unwrap();
        let mut command = Command::new("cargo");
        command.current_dir(&fixture.0).args([
            "run",
            "--manifest-path",
            "rust/Cargo.toml",
            "--target-dir",
            ".build/owned-dispatch",
            "--offline",
            "--locked",
            "--quiet",
            "--",
        ]);
        let captured =
            process::capture(command, std::time::Duration::from_secs(30), 1024 * 1024).unwrap();
        assert_eq!(captured.status.code(), Some(7));
    }
    #[test]
    fn genuine_owned_cargo_pair_emits_v4_and_rejects_stale_or_tampered_authority() {
        let fixture = Fixture::compiler_pair();
        let lease = BuildLock::acquire_from_environment(&fixture.0, &BTreeMap::new()).unwrap();
        let release = build_under_lease(&fixture.0, "release", &lease).unwrap();
        assert_eq!(release.schema, SCHEMA);
        assert!(release.release_preparation.is_some());
        assert_eq!(
            verify_under_lease(&fixture.0, "release", &lease)
                .unwrap()
                .artifacts,
            release.artifacts
        );
        assert!(!release.production_accepted && !release.installed_or_published);
        let mut historical = release.clone();
        historical.schema = "ruycast.rust-development-build.v3".into();
        assert!(verify_value(&fixture.0, &historical).is_err());
        let mut unprepared = release.clone();
        unprepared.release_preparation = None;
        assert!(verify_value(&fixture.0, &unprepared).is_err());
        let debug = build_under_lease(&fixture.0, "debug", &lease).unwrap();
        assert!(debug.release_preparation.is_none());
        assert_eq!(debug.configuration, "debug");
        assert!(debug
            .artifacts
            .keys()
            .all(|name| name.starts_with(".build/full-rust-debug/") && name.contains("/debug/")));
        assert!(verify_under_lease(&fixture.0, "debug", &lease).is_ok());
        assert!(verify_under_lease(&fixture.0, "release", &lease).is_ok());
        fs::write(
            fixture.0.join("config/default.json"),
            b"{\"ownedChange\":true}",
        )
        .unwrap();
        assert!(verify_under_lease(&fixture.0, "release", &lease).is_err());
        assert!(verify_under_lease(&fixture.0, "debug", &lease).is_err());
        fs::write(
            fixture.0.join("rust/src/main.rs"),
            b"fn main() { owned_invalid_source }\n",
        )
        .unwrap();
        assert!(build_under_lease(&fixture.0, "release", &lease).is_err());
        assert!(!fixture
            .0
            .join(OUTPUT)
            .join("development-build.json")
            .exists());
        BUILD_EXIT.store(1, std::sync::atomic::Ordering::Relaxed);
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn new_removed_changed_inputs_and_cargo_configuration_invalidate_the_snapshot() {
        let fixture = Fixture::new();
        let original = inventory(&fixture.0).unwrap();
        fs::write(
            fixture.0.join("rust-native/src/new_feature.rs"),
            b"pub fn owned() {}\n",
        )
        .unwrap();
        assert_ne!(inventory(&fixture.0).unwrap(), original);
        fs::remove_file(fixture.0.join("rust-native/src/new_feature.rs")).unwrap();
        assert_eq!(inventory(&fixture.0).unwrap(), original);
        fs::write(fixture.0.join("config/default.json"), b"{}").unwrap();
        assert_ne!(inventory(&fixture.0).unwrap(), original);
        fs::create_dir(fixture.0.join(".cargo")).unwrap();
        fs::write(
            fixture.0.join(".cargo/config.toml"),
            b"[build]\nrustflags=[]\n",
        )
        .unwrap();
        assert!(inventory(&fixture.0)
            .unwrap()
            .contains_key(".cargo/config.toml"));
        fs::remove_file(fixture.0.join("rust-native/Cargo.lock")).unwrap();
        assert!(inventory(&fixture.0).is_err());
    }
    #[test]
    fn linked_or_nonregular_sources_and_competing_builds_are_rejected() {
        let fixture = Fixture::new();
        let outside = fixture.0.join("owned-sentinel.rs");
        fs::write(&outside, b"owned sentinel").unwrap();
        let link = fixture.0.join("rust-native/src/linked.rs");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(inventory(&fixture.0).is_err());
        fs::remove_file(link).unwrap();
        let source_directory = fixture.0.join("rust/src");
        let retained_directory = fixture.0.join("rust/owned-source");
        fs::rename(&source_directory, &retained_directory).unwrap();
        std::os::unix::fs::symlink(&retained_directory, &source_directory).unwrap();
        assert!(inventory(&fixture.0).is_err());
        fs::remove_file(&source_directory).unwrap();
        fs::rename(retained_directory, source_directory).unwrap();
        let empty = BTreeMap::new();
        let first = BuildLock::acquire_from_environment(&fixture.0, &empty).unwrap();
        let inherited = first.environment().unwrap();
        let nested = BuildLock::acquire_from_environment(&fixture.0, &inherited).unwrap();
        nested.validate().unwrap();
        drop(nested);
        drop(first);
        assert!(BuildLock::acquire_from_environment(&fixture.0, &empty).is_ok());
        assert_eq!(fs::read(outside).unwrap(), b"owned sentinel");
    }
}
