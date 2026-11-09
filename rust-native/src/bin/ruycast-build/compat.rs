//! CLI compatibility belongs here; Python/shell entrypoints only choose this
//! Rust executable and forward arguments. Historical import oracles stay tests.
use super::*;

pub(super) fn option(args: &mut Vec<String>, names: &[&str]) -> Result<Option<String>> {
    let mut index = 0;
    while index < args.len() {
        if let Some((key, value)) = args[index].split_once('=') {
            if names.contains(&key) {
                let pair = (key.to_owned(), value.to_owned());
                args.splice(index..=index, [pair.0, pair.1]);
                index += 1;
            }
        }
        index += 1;
    }
    let matches: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, value)| names.contains(&value.as_str()))
        .map(|(index, _)| index)
        .collect();
    if matches.len() > 1 {
        return Err(format!("Duplicate option {}.", names[0]));
    }
    let Some(index) = matches.first().copied() else {
        return Ok(None);
    };
    if index + 1 >= args.len() || args[index + 1].starts_with('-') {
        return Err(format!("{} needs a value.", names[0]));
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}
fn no_args(args: &[String]) -> Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err("Unexpected compatibility arguments.".into())
    }
}
fn absolute_output(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    })
}
fn positive_build(value: &str) -> Result<u64> {
    if !value
        .as_bytes()
        .first()
        .is_some_and(|byte| (b'1'..=b'9').contains(byte))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(
            "Build number must be a positive decimal integer without leading zeros.".into(),
        );
    }
    value.parse().map_err(error)
}
fn fingerprint(root: &Path) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&inventory(root)?).map_err(error)?)
    ))
}
fn verification_sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = inventory(root)?;
    for name in [
        "Tests",
        "scripts",
        "docs",
        "Sources",
        "Package.swift",
        "README.md",
        "CONTRIBUTING.md",
        "Makefile",
    ] {
        validation::source_folder(root, &root.join(name), &mut values)?;
    }
    Ok(values)
}
// Git may inspect the selected checkout only. Caller Git configuration and
// redirection variables never cross this boundary, nor can Git invoke helpers.
fn root_git_command(root: &Path) -> Command {
    let mut command = Command::new("/usr/bin/git");
    command.env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .args(["--no-pager", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-c", "core.attributesFile=/dev/null", "-c", "protocol.allow=never"])
        .arg("--git-dir").arg(root.join(".git"))
        .arg("--work-tree").arg(root)
        .current_dir(root);
    command
}
fn root_git_diff(root: &Path, lease: &BuildLock) -> Result<()> {
    lease.validate()?;
    if !root.is_absolute() || root.canonicalize().map_err(error)? != root {
        return Err("Git whitespace check requires the canonical repository root.".into());
    }
    let capability = lease.environment()?;
    if capability.get("RUYCAST_ARTIFACT_LEASE_ROOT").map(Path::new) != Some(root) {
        return Err("Git whitespace check belongs to another artifact lease.".into());
    }
    let marker = fs::symlink_metadata(root.join(".git")).map_err(error)?;
    if marker.file_type().is_symlink() || !(marker.is_dir() || marker.is_file()) {
        return Err("Git whitespace check requires its own repository metadata.".into());
    }
    let mut command = root_git_command(root);
    command.args(["diff", "--no-ext-diff", "--no-textconv", "--check"]);
    lease.apply_to_child(&mut command)?;
    let result = process::capture(command, std::time::Duration::from_secs(30), 1024 * 1024)?;
    lease.validate()?;
    if !result.status.success() {
        // Git diagnostics include changed source lines. Do not echo potentially
        // private source into a receipt or diagnostic stream.
        return Err(format!("Repository whitespace check failed ({}).", result.status));
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pipeline {
    schema: String,
    pipeline: String,
    status: String,
    sources: BTreeMap<String, String>,
    build_receipt_sha256: String,
    native_report_sha256: String,
    production_accepted: bool,
}
fn verify_pipeline(root: &Path, kind: &str, lease: &BuildLock) -> Result<serde_json::Value> {
    if !matches!(kind, "check" | "settings") {
        return Err("Unknown verification pipeline.".into());
    }
    oracles::require_complete(kind)?;
    lease.validate()?;
    let path = root.join(".build/receipts").join(format!("{kind}.json"));
    let receipt: Pipeline = serde_json::from_slice(&regular_bytes(&path)?).map_err(error)?;
    if receipt.schema != "ruycast.verification-receipt.v2"
        || receipt.pipeline != kind
        || receipt.status != "passed"
        || receipt.production_accepted
        || receipt.sources != verification_sources(root)?
        || receipt.build_receipt_sha256
            != digest(&root.join(OUTPUT).join("development-build.json"))?
        || receipt.native_report_sha256
            != digest(&root.join("rust-native/target/verification/report.json"))?
    {
        return Err(
            "Verification receipt is invalid or stale; rerun its complete pipeline.".into(),
        );
    }
    verify_under_lease(root, "release", lease)?;
    validation::verify(root, lease)?;
    if kind == "check" { root_git_diff(root, lease)?; }
    serde_json::to_value(receipt).map_err(error)
}
fn with_pipeline_lease<T>(root:&Path, consume:impl FnOnce(&BuildLock)->Result<T>)->Result<T> {
    let lease = BuildLock::acquire(root)?;
    consume(&lease)
}
fn pipeline(root: &Path, kind: &str) -> Result<serde_json::Value> {
    with_pipeline_lease(root,|lease|pipeline_under_lease(root,kind,lease))
}
fn reset_pipeline_receipt(root: &Path, kind: &str, lease: &BuildLock) -> Result<PathBuf> {
    lease.validate()?;
    if !matches!(kind, "check" | "settings") {
        return Err("Unknown verification pipeline.".into());
    }
    let directory = root.join(".build/receipts");
    ruycast_native::private_storage::secure_directory(&directory)?;
    let path = directory.join(format!("{kind}.json"));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            fs::remove_file(&path).map_err(error)?
        }
        Ok(_) => return Err("Invalid existing verification receipt.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(e)),
    }
    Ok(path)
}
fn publish_pipeline_receipt(root: &Path, kind: &str, lease: &BuildLock, path: &Path, before: BTreeMap<String,String>) -> Result<()> {
    if verification_sources(root)? != before {
        return Err("Verification sources changed; no pass published.".into());
    }
    let value = Pipeline {
        schema: "ruycast.verification-receipt.v2".into(),
        pipeline: kind.into(),
        status: "passed".into(),
        sources: before,
        build_receipt_sha256: digest(&root.join(OUTPUT).join("development-build.json"))?,
        native_report_sha256: digest(&root.join("rust-native/target/verification/report.json"))?,
        production_accepted: false,
    };
    lease.validate()?;
    ruycast_native::private_storage::write(
        path,
        &serde_json::to_vec_pretty(&value).map_err(error)?,
        true,
    )?;
    Ok(())
}
fn pipeline_under_lease(root: &Path, kind: &str, lease: &BuildLock) -> Result<serde_json::Value> {
    let path = reset_pipeline_receipt(root, kind, lease)?;
    oracles::require_complete(kind)?;
    let before = verification_sources(root)?;
    build_under_lease(root, "release", lease)?;
    validation::run(root, lease)?;
    if kind == "check" { root_git_diff(root, lease)?; }
    publish_pipeline_receipt(root, kind, lease, &path, before)?;
    verify_pipeline(root, kind, lease)
}
fn build_receipt(root: &Path, mut args: Vec<String>) -> Result<serde_json::Value> {
    if args.is_empty() {
        return Err(
            "Expected build, verify, fingerprint, record, check, settings or performance.".into(),
        );
    }
    let action = args.remove(0);
    if matches!(action.as_str(), "--help" | "-h") || args == ["--help"] || args == ["-h"] {
        return Ok(serde_json::Value::String("build_receipt.py build [--configuration debug|release]\nbuild_receipt.py fingerprint|record --before SHA|verify\nbuild_receipt.py check|settings|verify-check|verify-settings\nbuild_receipt.py performance --runner PATH --runner-sha SHA --authority PATH --authority-sha SHA --dataset PATH --out NEW_REPORT\nbuild_receipt.py verify-performance".into()));
    }
    match action.as_str() {
        "build" => {
            let configuration =
                option(&mut args, &["-c", "--configuration"])?.unwrap_or("debug".into());
            no_args(&args)?;
            Ok(serde_json::to_value(build(root, &configuration)?).map_err(error)?)
        }
        "verify" => {
            no_args(&args)?;
            Ok(serde_json::to_value(verify(root, "release")?).map_err(error)?)
        }
        "fingerprint" => {
            no_args(&args)?;
            Ok(serde_json::Value::String(fingerprint(root)?))
        }
        "record" => {
            let before = option(&mut args, &["--before"])?.ok_or("record requires --before.")?;
            no_args(&args)?;
            if before != fingerprint(root)? {
                return Err("Production inputs changed; cannot record a mixed Rust build.".into());
            }
            let lease = BuildLock::acquire(root)?;
            Ok(
                serde_json::to_value(verify_under_lease(root, "release", &lease)?)
                    .map_err(error)?,
            )
        }
        "check" | "settings" => {
            no_args(&args)?;
            pipeline(root, &action)
        }
        "verify-check" | "verify-settings" => {
            no_args(&args)?;
            let lease = BuildLock::acquire(root)?;
            verify_pipeline(
                root,
                action
                    .strip_prefix("verify-")
                    .ok_or("Missing verification kind.")?,
                &lease,
            )
        }
        "performance" => performance(root, args),
        "verify-performance" => {
            no_args(&args)?;
            verify_performance(root)
        }
        _ => Err("Unknown build receipt action.".into()),
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PerformanceReceipt {
    schema: String,
    arguments: Vec<String>,
    sources: BTreeMap<String, String>,
    build_receipt_sha256: String,
    result: serde_json::Value,
    production_accepted: bool,
}
fn performance(root: &Path, args: Vec<String>) -> Result<serde_json::Value> {
    // Argument validation (including the no-authority failure) happens before
    // any build, lookup of a user profile or artifact mutation.
    if args.is_empty() {
        return Err("Performance requires a reviewed full F1 runner, its pinned --runner-sha, --authority, independently pinned --authority-sha, preserved --dataset and exclusive --out. No raw Swift or installed-app fallback exists.".into());
    }
    with_pipeline_lease(root,|lease| {
    verify_under_lease(root, "release", lease)?;
    let before = verification_sources(root)?;
    let result = evaluate_performance(root, args.clone(), lease)?;
    if verification_sources(root)? != before {
        return Err("Sources changed during full F1 verification.".into());
    }
    verify_under_lease(root, "release", lease)?;
    let receipt = PerformanceReceipt {
        schema: "ruycast.performance-verification.v2".into(),
        arguments: args,
        sources: before,
        build_receipt_sha256: digest(&root.join(OUTPUT).join("development-build.json"))?,
        result: result.clone(),
        production_accepted: false,
    };
    let path = root.join(".build/receipts/performance.json");
    ruycast_native::private_storage::write(
        &path,
        &serde_json::to_vec_pretty(&receipt).map_err(error)?,
        true,
    )?;
    Ok(result)
    })
}
fn verify_performance(root: &Path) -> Result<serde_json::Value> {
    let lease = BuildLock::acquire(root)?;
    let receipt: PerformanceReceipt = serde_json::from_slice(&regular_bytes(
        &root.join(".build/receipts/performance.json"),
    )?)
    .map_err(error)?;
    if receipt.schema != "ruycast.performance-verification.v2"
        || receipt.production_accepted
        || receipt.sources != verification_sources(root)?
        || receipt.build_receipt_sha256
            != digest(&root.join(OUTPUT).join("development-build.json"))?
    {
        return Err("Full F1 verification receipt is invalid or stale.".into());
    }
    verify_under_lease(root, "release", &lease)?;
    let report = receipt.result["report"]
        .as_str()
        .ok_or("Missing retained F1 report.")?;
    if receipt.result["ok"] != true
        || receipt.result["reportSHA256"].as_str() != Some(digest(Path::new(report))?.as_str())
    {
        return Err("Retained F1 report changed.".into());
    }
    let mut arguments = receipt.arguments;
    let old_output = option(&mut arguments, &["--out"])?.ok_or("Missing original F1 output.")?;
    if old_output != report {
        return Err("F1 receipt output binding changed.".into());
    }
    let stage = preparation::Stage::new(root, "verify-f1")?;
    arguments.extend([
        "--out".into(),
        stage
            .path()
            .join("comparison.json")
            .to_str()
            .ok_or("Invalid owned F1 stage.")?
            .into(),
    ]);
    evaluate_performance(root, arguments, &lease)?;
    if receipt.sources != verification_sources(root)? {
        return Err("Sources changed while revalidating full F1 evidence.".into());
    }
    lease.validate()?;
    Ok(
        serde_json::json!({"ok":true,"report":report,"reportSHA256":receipt.result["reportSHA256"],"revalidatedFullDataset":true,"productionAccepted":false}),
    )
}
fn evaluate_performance(
    root: &Path,
    mut args: Vec<String>,
    lease: &BuildLock,
) -> Result<serde_json::Value> {
    let runner = option(&mut args, &["--runner"])?.ok_or("Performance requires a reviewed full F1 runner, its pinned --runner-sha, --authority, independently pinned --authority-sha, preserved --dataset and exclusive --out. No raw Swift or installed-app fallback exists.")?;
    let runner_sha = option(&mut args, &["--runner-sha"])?.ok_or("Missing reviewed runner SHA.")?;
    let authority = option(&mut args, &["--authority"])?.ok_or("Missing reviewed F1 authority.")?;
    let authority_sha = option(&mut args, &["--authority-sha"])?
        .ok_or("Missing independently reviewed authority SHA.")?;
    let dataset =
        option(&mut args, &["--dataset"])?.ok_or("Missing preserved complete F1 dataset.")?;
    let destination =
        option(&mut args, &["--out"])?.ok_or("Missing exclusive F1 report output.")?;
    no_args(&args)?;
    for hash in [&runner_sha, &authority_sha] {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Expected an independently pinned SHA256.".into());
        }
    }
    for path in [&runner, &authority, &dataset, &destination] {
        if !Path::new(path).is_absolute() {
            return Err("F1 evidence paths must be absolute.".into());
        }
    }
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("F1 output already exists; refusing overwrite.".into());
    }
    if digest(Path::new(&runner))? != runner_sha || digest(Path::new(&authority))? != authority_sha
    {
        return Err("F1 runner or independently reviewed authority changed.".into());
    }
    let dataset_sha = digest(Path::new(&dataset))?;
    for (index, arguments) in [
        vec!["validate-authority", &authority, &authority_sha],
        vec![
            "compare",
            &authority,
            &authority_sha,
            &dataset,
            &destination,
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let mut command = Command::new(&runner);
        command.args(&arguments).current_dir(root);
        lease.apply_to_child(&mut command)?;
        let captured = process::capture(
            command,
            std::time::Duration::from_secs(180),
            8 * 1024 * 1024,
        )?;
        if !captured.status.success() {
            return Err(format!(
                "Full F1 validator failed: {}",
                String::from_utf8_lossy(&captured.stderr)
            ));
        }
        if captured.stdout.last() != Some(&b'\n')
            || captured
                .stdout
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                != 1
        {
            return Err("F1 runner must emit precisely one UTF-8 JSON line.".into());
        }
        let value: serde_json::Value = serde_json::from_slice(&captured.stdout).map_err(error)?;
        let expected = if index == 0 {
            serde_json::json!({"ok":true,"schema":"ruycast.f1.complete.v1","operation":"validate-authority","authority_sha256":authority_sha,"status":"authority-valid; no product executed"})
        } else {
            serde_json::json!({"ok":true,"schema":"ruycast.f1.complete.v1","operation":"compare","authority_sha256":authority_sha,"dataset_sha256":dataset_sha,"report_path":destination,"report_sha256":digest(Path::new(&destination))?})
        };
        if value != expected {
            return Err("F1 runner success fields do not match the reviewed authority, dataset and actual report.".into());
        }
        if digest(Path::new(&runner))? != runner_sha
            || digest(Path::new(&authority))? != authority_sha
            || digest(Path::new(&dataset))? != dataset_sha
        {
            return Err("F1 evidence changed during validation.".into());
        }
    }
    lease.validate()?;
    let report: serde_json::Value =
        serde_json::from_slice(&regular_bytes(Path::new(&destination))?).map_err(error)?;
    if report["ok"] != true || report["source_dataset_sha256"] != dataset_sha {
        return Err("F1 report does not bind the validated complete source dataset.".into());
    }
    Ok(
        serde_json::json!({"ok":true,"report":destination,"reportSHA256":digest(Path::new(&destination))?,"runnerSHA256":runner_sha,"authoritySHA256":authority_sha,"datasetSHA256":dataset_sha,"productionAccepted":false}),
    )
}
pub(super) fn run(mut args: Vec<String>) -> Result<serde_json::Value> {
    if args.len() < 5 || args[0] != "compat" || args[2] != "--repo" || args[4] != "--" {
        return Err("Expected compat ENTRY --repo ABSOLUTE_ROOT -- [legacy arguments].".into());
    }
    let entry = args[1].clone();
    let root = Path::new(&args[3]);
    if !root.is_absolute() {
        return Err("Choose an absolute repository path.".into());
    }
    let root = root.canonicalize().map_err(error)?;
    let tail = args.split_off(5);
    if tail == ["--help"] || tail == ["-h"] {
        let usage=match entry.as_str(){"package"=>"package-app.py --out NEW_DIRECTORY --build-number POSITIVE_NUMBER | --verify DIRECTORY","version"=>"product_version.py","prepare-release"=>"prepare_release.py [--recover]","native-check"=>"check.py (complete native matrix; prepared v4 release required)","build-receipt"=>"build_receipt.py build|verify|fingerprint|record|check|settings|performance|verify-check|verify-settings|verify-performance",_=>return Err("Unknown Rust compatibility entrypoint.".into())};
        return Ok(serde_json::Value::String(usage.into()));
    }
    match entry.as_str() {
        "build-receipt" => build_receipt(&root, tail),
        "version" => {
            no_args(&tail)?;
            Ok(serde_json::Value::String(version::read(&root)?))
        }
        "prepare-release" => {
            let lease = BuildLock::acquire(&root)?;
            let value = if tail == ["--recover"] {
                preparation::recover(&root)?;
                serde_json::json!({"ok":true,"recovered":true})
            } else {
                no_args(&tail)?;
                serde_json::to_value(preparation::prepare(&root, &host_target(&root)?)?)
                    .map_err(error)?
            };
            lease.validate()?;
            Ok(value)
        }
        "native-check" => {
            no_args(&tail)?;
            let lease = BuildLock::acquire(&root)?;
            validation::run(&root, &lease)
        }
        "package" => {
            let mut tail = tail;
            let destination = option(&mut tail, &["--out"])?;
            let existing = option(&mut tail, &["--verify"])?;
            let build = option(&mut tail, &["--build-number"])?;
            no_args(&tail)?;
            match (destination, existing, build) {
                (Some(destination), None, Some(build)) => candidate::stage_public(&root, positive_build(&build)?, &absolute_output(&destination)?),
                (None, Some(existing), None) => candidate::verify(&root, &absolute_output(&existing)?),
                _ => Err("Expected --out ABSOLUTE_NEW_DIRECTORY --build-number POSITIVE_NUMBER, or --verify ABSOLUTE_CANDIDATE.".into()),
            }
        }
        _ => Err("Unknown Rust compatibility entrypoint.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Internal owned subprocess entry; invoked by the exact parent test"]
    fn owned_failed_build_process_entry() {
        let root = PathBuf::from(std::env::var_os("RUYCAST_OWNED_FAILED_BUILD_ROOT").unwrap());
        assert_eq!(root.canonicalize().unwrap(), root);
        match std::env::var("RUYCAST_OWNED_FAILED_BUILD_MODE").unwrap().as_str() {
            "pipeline" => {
                assert!(oracles::require_complete("settings").is_ok());
                let error = pipeline(&root, "settings").unwrap_err();
                assert!(error.contains("Rust release build failed"), "{error}");
                fs::write(root.join("actual-build-error"), error).unwrap();
            }
            "contender" => {
                let _lease = BuildLock::acquire_from_environment(&root, &BTreeMap::new()).unwrap();
                fs::write(root.join("contender-acquired"), b"acquired after failed pipeline").unwrap();
            }
            _ => panic!("unknown owned failed-build phase"),
        }
    }
    #[test]
    fn actual_failed_build_stops_consumers_revokes_pass_and_releases_pipeline_lease() {
        use std::time::Duration;
        let fixture = super::super::tests::Fixture::compiler_pair();
        for directory in ["Tests", "scripts", "docs", "Sources"] {
            fs::create_dir(fixture.0.join(directory)).unwrap();
        }
        for name in ["Package.swift", "README.md", "CONTRIBUTING.md", "Makefile"] {
            fs::write(fixture.0.join(name), b"owned verification input").unwrap();
        }
        // A real Cargo/rustc failure, not a synthetic failed receipt or gate.
        fs::write(fixture.0.join("rust/src/main.rs"), b"this is deliberately invalid Rust\n").unwrap();
        let receipts = fixture.0.join(".build/receipts");
        fs::create_dir_all(&receipts).unwrap();
        let pass = receipts.join("settings.json");
        fs::write(&pass, b"previous passed receipt").unwrap();
        let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
        assert!(sysroot.status.success());
        let tools = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim()).join("bin");
        let cargo = tools.join("cargo");
        let rustc = tools.join("rustc");
        assert!(cargo.is_file() && rustc.is_file());
        let directory = fixture.0.join("bin");
        fs::create_dir(&directory).unwrap();
        let source = directory.join("cargo-recorder.rs");
        // Observe the production pipeline's real Cargo invocation, then always
        // delegate to the actual compiler. A later build/consumer is witnessed;
        // the recorder never fabricates successful compiler or F1 output.
        fs::write(&source, format!(r#"
use std::io::Write;
fn main() {{
    let root = std::path::Path::new({root:?});
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "rustc" || a == "build") {{
        let first = std::fs::OpenOptions::new().write(true).create_new(true).open(root.join("producer-entered")).is_ok();
        if !first {{ std::fs::write(root.join("later-consumer-entered"), b"unexpected later command").unwrap(); }}
        std::fs::OpenOptions::new().append(true).create(true).open(root.join("compiler-invocations")).unwrap().write_all(b"compiler\n").unwrap();
    }}
    let status = std::process::Command::new({cargo:?}).args(args).status().unwrap();
    std::process::exit(status.code().unwrap_or(1));
}}
"#, root=fixture.0, cargo=cargo)).unwrap();
        let mut compile = Command::new(&rustc);
        compile.args(["--edition=2021", "-Dwarnings"]).arg(&source).arg("-o").arg(directory.join("cargo"));
        let compiled = process::capture(compile, Duration::from_secs(30), 1024 * 1024).unwrap();
        assert!(compiled.status.success(), "{}", String::from_utf8_lossy(&compiled.stderr));
        let path = std::env::join_paths([directory, tools]).unwrap();
        let child = |mode: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--ignored", "--exact", "compat::tests::owned_failed_build_process_entry", "--test-threads=1"])
                .env_clear().env("PATH", &path).env("HOME", &fixture.0).env("TMPDIR", &fixture.0)
                .env("CARGO_INCREMENTAL", "0").env("CARGO_HOME", fixture.0.join("cargo-home"))
                .env("RUYCAST_OWNED_FAILED_BUILD_ROOT", &fixture.0)
                .env("RUYCAST_OWNED_FAILED_BUILD_MODE", mode).current_dir(&fixture.0);
            command
        };
        let failed = process::capture(child("pipeline"), Duration::from_secs(30), 1024 * 1024).unwrap();
        assert!(failed.status.success(), "{}", String::from_utf8_lossy(&failed.stderr));
        assert!(fixture.0.join("producer-entered").is_file());
        assert_eq!(fs::read(fixture.0.join("compiler-invocations")).unwrap(), b"compiler\n");
        assert!(!fixture.0.join("later-consumer-entered").exists());
        assert!(!pass.exists());
        assert!(!fixture.0.join("rust-native/target/verification/report.json").exists());
        assert!(fs::read_to_string(fixture.0.join("actual-build-error")).unwrap().contains("Rust release build failed"));
        let contender = process::capture(child("contender"), Duration::from_secs(8), 1024 * 1024).unwrap();
        assert!(contender.status.success(), "{}", String::from_utf8_lossy(&contender.stderr));
        assert!(fixture.0.join("contender-acquired").is_file());
    }
    #[test]
    fn incomplete_full_gate_invalidates_its_previous_pass_and_cannot_reverify() {
        let fixture = super::super::tests::Fixture::new();
        let lease = BuildLock::acquire_from_environment(&fixture.0, &BTreeMap::new()).unwrap();
        let directory = fixture.0.join(".build/receipts");
        ruycast_native::private_storage::secure_directory(&directory).unwrap();
        for kind in ["check", "settings"] {
            let path = directory.join(format!("{kind}.json"));
            fs::write(&path, b"previous passed receipt").unwrap();
            let mapping_incomplete = oracles::require_complete(kind).is_err();
            let error = pipeline_under_lease(&fixture.0, kind, &lease).unwrap_err();
            // Settings may now have a complete source crosswalk; this fake
            // build must still fail and revoke its previous pass. Require the
            // mapping diagnostic only for a genuinely incomplete inventory.
            if mapping_incomplete { assert!(error.contains("not yet qualified")); }
            assert!(!path.exists());
            assert!(verify_pipeline(&fixture.0, kind, &lease).is_err());
        }
    }
    #[test]
    #[ignore = "Explicit reviewed F1 runner integration entry; requires pinned owned reviewer arguments"]
    fn reviewed_f1_rejection_entry() {
        let fixture = super::super::tests::Fixture::new();
        let runner = std::env::var("RUYCAST_REVIEWED_F1_RUNNER").unwrap();
        let runner_sha = std::env::var("RUYCAST_REVIEWED_F1_RUNNER_SHA256").unwrap();
        let authority = fixture.0.join("owned-invalid-authority.json");
        fs::write(&authority, b"{}\n").unwrap();
        let dataset = fixture.0.join("owned-not-collected-dataset.json");
        fs::write(&dataset, b"{}\n").unwrap();
        let report = fixture.0.join("must-not-exist.json");
        let lease = BuildLock::acquire_from_environment(&fixture.0, &BTreeMap::new()).unwrap();
        let error = evaluate_performance(
            &fixture.0,
            vec![
                "--runner".into(),
                runner,
                "--runner-sha".into(),
                runner_sha,
                "--authority".into(),
                authority.to_str().unwrap().into(),
                "--authority-sha".into(),
                digest(&authority).unwrap(),
                "--dataset".into(),
                dataset.to_str().unwrap().into(),
                "--out".into(),
                report.to_str().unwrap().into(),
            ],
            &lease,
        )
        .unwrap_err();
        assert!(error.contains("Full F1 validator failed"), "{error}");
        assert!(!report.exists());
    }
    #[test]
    fn real_compatibility_processes_only_forward_to_explicit_host_rust_bootstrap() {
        let fixture = super::super::tests::Fixture::new();
        let directory = fixture.0.join("owned-bootstrap");
        fs::create_dir(&directory).unwrap();
        let source = directory.join("capture.rs");
        fs::write(&source, r#"fn main(){let mut bytes=Vec::new();for value in std::env::args().skip(1){bytes.extend_from_slice(value.as_bytes());bytes.push(0);}std::fs::write(std::env::var("RUYCAST_OWNED_DISPATCH_RECORD").unwrap(),bytes).unwrap();std::process::exit(7);}"#).unwrap();
        let cargo = directory.join("cargo");
        output(
            &fixture.0,
            "rustc",
            &[source.to_str().unwrap(), "-o", cargo.to_str().unwrap()],
        )
        .unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let record = directory.join("arguments");
        for (interpreter, script, entry, tail) in [
            (
                "python3",
                "scripts/build_receipt.py",
                "build-receipt",
                vec!["verify"],
            ),
            ("python3", "scripts/product_version.py", "version", vec![]),
            (
                "python3",
                "scripts/prepare_release.py",
                "prepare-release",
                vec!["--recover"],
            ),
            (
                "python3",
                "scripts/package-app.py",
                "package",
                vec!["--out", "/owned/new-output", "--build-number=68"],
            ),
            (
                "python3",
                "rust-native/scripts/check.py",
                "native-check",
                vec![],
            ),
            (
                "bash",
                "scripts/build-native.sh",
                "build-receipt",
                vec!["--configuration=debug"],
            ),
            (
                "bash",
                "scripts/build-native-locked.sh",
                "build-receipt",
                vec!["--configuration=release"],
            ),
            ("bash", "scripts/check.sh", "build-receipt", vec![]),
            ("bash", "scripts/check-settings.sh", "build-receipt", vec![]),
            ("bash", "scripts/check-locked.sh", "build-receipt", vec!["--literal=owned value"]),
            ("bash", "scripts/check-settings-locked.sh", "build-receipt", vec!["--literal=owned value"]),
            ("bash", "scripts/check-performance-locked.sh", "build-receipt", vec!["--literal=owned value"]),
            (
                "bash",
                "scripts/check-performance.sh",
                "build-receipt",
                vec![],
            ),
        ] {
            let mut command = Command::new(interpreter);
            command
                .arg(root.join(script))
                .args(&tail)
                .env(
                    "PATH",
                    format!("{}:{}", directory.display(), std::env::var("PATH").unwrap()),
                )
                .env("RUYCAST_OWNED_DISPATCH_RECORD", &record)
                .env("PYTHONDONTWRITEBYTECODE", "1");
            let result =
                process::capture(command, std::time::Duration::from_secs(10), 65536).unwrap();
            assert_eq!(
                result.status.code(),
                Some(7),
                "{script}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let bytes = fs::read(&record).unwrap();
            let forwarded = bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| std::str::from_utf8(part).unwrap())
                .collect::<Vec<_>>();
            let prefix = ["run", "--quiet", "--locked", "--offline", "--target-dir"];
            assert_eq!(&forwarded[..prefix.len()], prefix);
            assert!(
                !forwarded
                    .iter()
                    .any(|arg| matches!(*arg, "--target" | "host-tuple")),
                "native Cargo bootstrap must use its default host target"
            );
            assert_eq!(
                forwarded[5],
                root.join(".build/delivery-tools").to_str().unwrap()
            );
            assert_eq!(forwarded[6], "--manifest-path");
            assert_eq!(
                forwarded[7],
                root.join("rust-native/Cargo.toml").to_str().unwrap()
            );
            assert_eq!(
                &forwarded[8..13],
                ["--bin", "ruycast-build", "--", "compat", entry]
            );
            assert_eq!(forwarded[13], "--repo");
            assert_eq!(forwarded[14], root.to_str().unwrap());
            assert_eq!(forwarded[15], "--");
            let action = match script {
                "scripts/build-native.sh" | "scripts/build-native-locked.sh" => Some("build"),
                "scripts/check.sh" | "scripts/check-locked.sh" => Some("check"),
                "scripts/check-settings.sh" | "scripts/check-settings-locked.sh" => Some("settings"),
                "scripts/check-performance.sh" | "scripts/check-performance-locked.sh" => Some("performance"),
                _ => None,
            };
            let mut expected = vec![];
            if let Some(action) = action {
                expected.push(action);
            }
            expected.extend(tail);
            assert_eq!(&forwarded[16..], expected);
        }
        // The two installer facades take their real Cargo fallback, never an
        // installed/overridden tool; the same owned Rust recorder exits first.
        for (script, operation, tail) in [
            (
                "scripts/install.py",
                "install",
                vec![
                    "--archive",
                    "literal owned archive",
                    "--sha256",
                    "literal digest",
                ],
            ),
            (
                "scripts/local_signing.py",
                "signing",
                vec!["status", "--root", "literal signing root"],
            ),
        ] {
            let mut command = Command::new("python3");
            command
                .arg(root.join(script))
                .args(&tail)
                .env(
                    "PATH",
                    format!("{}:{}", directory.display(), std::env::var("PATH").unwrap()),
                )
                .env_remove("RUYCAST_INSTALL_TOOL")
                .env("RUYCAST_OWNED_DISPATCH_RECORD", &record)
                .env("PYTHONDONTWRITEBYTECODE", "1");
            let result =
                process::capture(command, std::time::Duration::from_secs(10), 65536).unwrap();
            assert_eq!(
                result.status.code(),
                Some(7),
                "{script}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let bytes = fs::read(&record).unwrap();
            let forwarded = bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| std::str::from_utf8(part).unwrap())
                .collect::<Vec<_>>();
            let manifest = root.join("rust-native/Cargo.toml");
            let target = root.join(".build/installer-tools");
            let mut expected = vec![
                "run",
                "--locked",
                "--offline",
                "--release",
                "--manifest-path",
                manifest.to_str().unwrap(),
                "--target-dir",
                target.to_str().unwrap(),
                "--bin",
                "ruycast-install",
                "--",
                operation,
            ];
            if operation == "install" {
                expected.extend(["--repo", root.to_str().unwrap()]);
                expected.extend(tail);
            } else {
                expected.extend(tail);
                expected.extend(["--repo", root.to_str().unwrap()]);
            }
            assert_eq!(forwarded, expected, "{script}");
            assert!(!forwarded
                .iter()
                .any(|arg| matches!(*arg, "--target" | "host-tuple")));
        }
    }
    #[test]
    fn legacy_argument_contract_rejects_duplicates_unknowns_and_unreviewed_performance() {
        assert_eq!(positive_build("68").unwrap(), 68);
        for value in [
            "0",
            "00",
            "068",
            "+68",
            "-1",
            "1.2",
            "not-a-number",
            "18446744073709551616",
        ] {
            assert!(positive_build(value).is_err());
        }
        let fixture = super::super::tests::Fixture::new();
        assert!(build_receipt(&fixture.0, vec!["performance".into()])
            .err()
            .unwrap()
            .contains("reviewed full F1 runner"));
        assert!(build_receipt(
            &fixture.0,
            vec![
                "build".into(),
                "--configuration".into(),
                "debug".into(),
                "-c".into(),
                "release".into()
            ]
        )
        .err()
        .unwrap()
        .contains("Duplicate"));
        assert!(build_receipt(&fixture.0, vec!["check".into(), "unexpected".into()]).is_err());
        assert!(build_receipt(&fixture.0, vec!["record".into()]).is_err());
        assert!(run(vec![
            "compat".into(),
            "version".into(),
            "--repo".into(),
            "relative".into(),
            "--".into()
        ])
        .is_err());
    }

    fn owned_git_fixture(root: &Path) {
        assert!(!root.join(".git").exists());
        let mut init = root_git_command(root);
        init.args(["init", "--quiet", "--template="]);
        assert!(process::capture(init, std::time::Duration::from_secs(10), 65536).unwrap().status.success());
        fs::write(root.join("tracked.txt"), b"baseline\n").unwrap();
        let mut add = root_git_command(root);
        add.args(["add", "--", "tracked.txt"]);
        // Index initialization belongs only to this newly created disposable
        // fixture. No user repository is added, committed, reset or cleaned.
        assert!(process::capture(add, std::time::Duration::from_secs(10), 65536).unwrap().status.success());
    }
    #[test]
    #[ignore = "Owned Git-environment subprocess, invoked only by the exact parent test"]
    fn root_git_diff_owned_child() {
        let root = PathBuf::from(std::env::var_os("RUYCAST_OWNED_GIT_ROOT").unwrap());
        let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").unwrap());
        assert!(root.is_absolute() && root.starts_with(parent.canonicalize().unwrap()));
        let lease = BuildLock::acquire_from_environment(&root, &BTreeMap::new()).unwrap();
        root_git_diff(&root, &lease).unwrap();
    }
    #[test]
    fn root_git_diff_ignores_other_cwd_and_hostile_git_environment() {
        let chosen = super::super::tests::Fixture::new();
        let other = super::super::tests::Fixture::new();
        owned_git_fixture(&chosen.0); owned_git_fixture(&other.0);
        fs::write(chosen.0.join("tracked.txt"), b"chosen dirty without whitespace errors\n").unwrap();
        fs::write(other.0.join("tracked.txt"), b"other whitespace must not be selected \n").unwrap();
        let hostile_index = other.0.join("not-an-index-file");
        fs::create_dir(&hostile_index).unwrap();
        let external = PathBuf::from("/usr/bin/false");
        let global = other.0.join("hostile.gitconfig");
        fs::write(&global, format!("[core]\n fsmonitor = {}\n hooksPath = {}\n worktree = {}\n[diff]\n external = {}\n", external.display(), other.0.display(), other.0.display(), external.display())).unwrap();
        let before = [digest(&chosen.0.join("tracked.txt")).unwrap(), digest(&chosen.0.join(".git/index")).unwrap(), digest(&other.0.join("tracked.txt")).unwrap(), digest(&other.0.join(".git/index")).unwrap()];
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args(["compat::tests::root_git_diff_owned_child", "--exact", "--ignored", "--test-threads=1"])
            .current_dir(&other.0)
            .env("RUYCAST_OWNED_GIT_ROOT", &chosen.0)
            .env("GIT_DIR", other.0.join(".git"))
            .env("GIT_WORK_TREE", &other.0)
            .env("GIT_COMMON_DIR", other.0.join(".git"))
            .env("GIT_INDEX_FILE", &hostile_index)
            .env("GIT_CONFIG_GLOBAL", &global).env("GIT_CONFIG_SYSTEM", &global)
            .env("GIT_CONFIG_NOSYSTEM", "0")
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "core.worktree").env("GIT_CONFIG_VALUE_0", &other.0)
            .env("GIT_CONFIG_KEY_1", "core.fsmonitor").env("GIT_CONFIG_VALUE_1", &external)
            .env("GIT_EXTERNAL_DIFF", &external).env("GIT_PAGER", &external)
            .env("HOME", &other.0).env("XDG_CONFIG_HOME", &other.0);
        let result = process::capture(child, std::time::Duration::from_secs(30), 1024 * 1024).unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(String::from_utf8(result.stdout).unwrap().contains("1 passed; 0 failed; 0 ignored;"));
        assert_eq!(before, [digest(&chosen.0.join("tracked.txt")).unwrap(), digest(&chosen.0.join(".git/index")).unwrap(), digest(&other.0.join("tracked.txt")).unwrap(), digest(&other.0.join(".git/index")).unwrap()]);
    }
    #[test]
    fn root_git_diff_allows_dirty_content_but_refuses_whitespace_without_mutation() {
        let fixture = super::super::tests::Fixture::new();
        owned_git_fixture(&fixture.0);
        let lease = BuildLock::acquire_from_environment(&fixture.0, &BTreeMap::new()).unwrap();
        let file = fixture.0.join("tracked.txt");
        let index = digest(&fixture.0.join(".git/index")).unwrap();
        fs::write(&file, b"dirty is allowed\n").unwrap();
        root_git_diff(&fixture.0, &lease).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"dirty is allowed\n");
        fs::write(&file, b"OWNED_PRIVATE_PAYLOAD \n").unwrap();
        let before = fs::read(&file).unwrap();
        let rejected = root_git_diff(&fixture.0, &lease).unwrap_err();
        assert!(!rejected.contains("OWNED_PRIVATE_PAYLOAD"));
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(digest(&fixture.0.join(".git/index")).unwrap(), index);
    }
}

#[cfg(test)]
mod temporal_contract_tests {
    use super::*;
    use std::{os::{fd::AsRawFd, unix::fs::PermissionsExt}, process::Stdio, thread::JoinHandle, time::{Duration,Instant}};

    fn wait(root:&Path,name:&str) {
        let end=Instant::now()+Duration::from_secs(45);
        while !root.join(name).exists() {
            assert!(Instant::now()<end,"owned temporal marker absent: {name}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn event(root:&Path,name:&str,bytes:&[u8]) {
        use std::io::Write;
        let mut file=OpenOptions::new().write(true).create_new(true).open(root.join(name)).unwrap();
        file.write_all(bytes).unwrap();
    }
    fn child(root:&Path,mode:&str)->Command {
        let mut command=Command::new(std::env::current_exe().unwrap());
        command.args(["--ignored","--exact","compat::temporal_contract_tests::owned_temporal_process_entry","--test-threads=1","--nocapture"])
            .env_clear().env("PATH",std::env::var_os("PATH").unwrap())
            .env("HOME",root).env("TMPDIR",root).env("CARGO_HOME",root.join("cargo-home"))
            .env("CARGO_INCREMENTAL","0").env("CARGO_BUILD_JOBS","1")
            .env("RUYCAST_OWNED_TEMPORAL_ROOT",root).env("RUYCAST_OWNED_TEMPORAL_MODE",mode)
            .current_dir(root).stdin(Stdio::null());
        command
    }
    #[test]
    #[ignore="Internal owned temporal process; exact private parent invokes it"]
    fn owned_temporal_process_entry() {
        let root=PathBuf::from(std::env::var_os("RUYCAST_OWNED_TEMPORAL_ROOT").unwrap());
        assert_eq!(root.canonicalize().unwrap(),root);
        assert_eq!(fs::read(root.join(".temporal-owned")).unwrap(),b"disposable-temporal-v1");
        let mode=std::env::var("RUYCAST_OWNED_TEMPORAL_MODE").unwrap();
        if mode=="consumer" {
            let lease=BuildLock::acquire(&root).unwrap();
            lease.validate().unwrap();
            event(&root,"consumer-capability.json",&serde_json::to_vec_pretty(&lease.environment().unwrap()).unwrap());
            event(&root,"consumer-entered",std::process::id().to_string().as_bytes());
            wait(&root,"release-consumer");lease.validate().unwrap();
            event(&root,"consumer-finished",b"final child completed while leased");
            return;
        }
        if matches!(mode.as_str(),"contender"|"performance-contender") {
            event(&root,"contender-requested",b"requested");
            let lease=BuildLock::acquire_from_environment(&root,&BTreeMap::new()).unwrap();
            if mode=="performance-contender" {
                assert!(root.join("consumer-finished").is_file() && root.join("authority-validation-refused").is_file());
            } else {assert!(root.join("pipeline-finished").is_file());}
            lease.validate().unwrap();
            event(&root,"contender-acquired",b"after actual publication/refusal");return;
        }
        if mode=="performance-success" {
            // Same lexical lease owner used by BOTH public pipelines. This
            // disposable operation returns (), never F1 JSON or a receipt.
            with_pipeline_lease(&root,|lease| {
                lease.validate()?;
                event(&root,"pipeline-capability.json",&serde_json::to_vec_pretty(&lease.environment()?).unwrap());
                let receipt=build_under_lease(&root,"release",lease)?;
                let before=verification_sources(&root)?;
                let mut consumer=child(&root,"consumer");lease.apply_to_child(&mut consumer)?;
                let result=process::capture(consumer,Duration::from_secs(60),1024*1024)?;
                event(&root,"consumer.stdout",&result.stdout);event(&root,"consumer.stderr",&result.stderr);
                assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
                verify_value(&root,&receipt)?;
                assert_eq!(verification_sources(&root)?,before);
                event(&root,"successful-consumer-returned",b"actual disposable child exited zero; no F1 result");
                wait(&root,"release-final-return");
                verify_value(&root,&receipt)?;
                assert_eq!(verification_sources(&root)?,before);
                lease.validate()?;
                event(&root,"pipeline-finished",b"successful disposable operation returns while production boundary is leased");
                Ok(())
            }).unwrap();
            return;
        }
        if mode=="performance" {
            // Enter the actual public performance lock/verification path. The
            // disposable runner holds its inherited lease, then REFUSES; it
            // cannot manufacture authority-valid JSON or a benchmark pass.
            let receipt:Receipt=serde_json::from_slice(&fs::read(root.join(OUTPUT).join("development-build.json")).unwrap()).unwrap();
            let runner=root.join(artifact_paths_for(&receipt.target,"release").unwrap()[0].0.clone());
            let authority=root.join("owned-unreviewed-authority.json");
            let dataset=root.join("owned-uncollected-dataset.json");
            let output=root.join("must-not-publish-performance.json");
            let error=performance(&root,vec![
                "--runner".into(),runner.to_str().unwrap().into(),"--runner-sha".into(),digest(&runner).unwrap(),
                "--authority".into(),authority.to_str().unwrap().into(),"--authority-sha".into(),digest(&authority).unwrap(),
                "--dataset".into(),dataset.to_str().unwrap().into(),"--out".into(),output.to_str().unwrap().into(),
            ]).unwrap_err();
            assert!(error.contains("Full F1 validator failed:") && error.contains("owned disposable authority validation refused"),"{error}");
            assert!(root.join("consumer-finished").is_file());
            assert!(!output.exists() && !root.join(".build/receipts/performance.json").exists());
            event(&root,"actual-refusal.txt",error.as_bytes());
            event(&root,"pipeline-finished",b"actual performance authority validation refused while leased");
            return;
        }
        assert!(matches!(mode.as_str(),"none"|"source"|"artifact"|"settings"));
        let kind=if mode=="settings" {"settings"} else {"check"};
        let lease=BuildLock::acquire(&root).unwrap();
        event(&root,"pipeline-capability.json",&serde_json::to_vec_pretty(&lease.environment().unwrap()).unwrap());
        // Same production old-pass removal, before any producer/consumer.
        // Public require_complete remains unchanged; this is a private fixture.
        let path=reset_pipeline_receipt(&root,kind,&lease).unwrap();
        assert!(!path.exists());
        let receipt=build_under_lease(&root,"release",&lease).unwrap();
        let before=verification_sources(&root).unwrap();
        event(&root,"snapshot-before.json",&serde_json::to_vec_pretty(&before).unwrap());
        event(&root,"build-before.json",&serde_json::to_vec_pretty(&receipt).unwrap());
        let mut consumer=child(&root,"consumer");lease.apply_to_child(&mut consumer).unwrap();
        let result=process::capture(consumer,Duration::from_secs(60),1024*1024).unwrap();
        event(&root,"consumer.stdout",&result.stdout);event(&root,"consumer.stderr",&result.stderr);
        assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
        let final_result=if mode!="source" {
            // Actual verify_value is the same final artifact/receipt verifier
            // called by validation::run before it publishes its report.
            verify_value(&root,&receipt).and_then(|()|publish_pipeline_receipt(&root,kind,&lease,&path,before))
        } else {
            // Source case must reach the actual production snapshot guard,
            // not fail first in an independent test-only hash comparison.
            publish_pipeline_receipt(&root,kind,&lease,&path,before)
        };
        event(&root,"snapshot-after.json",&serde_json::to_vec_pretty(&verification_sources(&root).unwrap()).unwrap());
        match mode.as_str() {
            "none"|"settings"=>{
                final_result.unwrap();
                let actual:Pipeline=serde_json::from_slice(&regular_bytes(&path).unwrap()).unwrap();
                assert_eq!(actual.status,"passed");assert_eq!(actual.pipeline,kind);
                assert!(!actual.production_accepted);
                assert_eq!(actual.sources,verification_sources(&root).unwrap());
                event(&root,"fixture-receipt.json",&fs::read(&path).unwrap());
            }
            "source"|"artifact"=>{
                let error=final_result.unwrap_err();
                if mode=="source" {assert!(error.contains("Verification sources changed"),"{error}");}
                else {assert_eq!(error,"Prepared release artifact changed.");}
                assert!(!path.exists());event(&root,"actual-refusal.txt",error.as_bytes());
            }
            _=>unreachable!(),
        }
        lease.validate().unwrap();event(&root,"pipeline-finished",b"same lease through final publication decision");
    }
    struct Running {root:PathBuf,worker:Option<JoinHandle<()>>,release_final:bool}
    impl Running {fn finish(mut self){self.worker.take().unwrap().join().unwrap();}}
    impl Drop for Running {
        fn drop(&mut self){
            let _=fs::write(self.root.join("release-consumer"),b"cleanup release");
            if self.release_final {let _=fs::write(self.root.join("release-final-return"),b"cleanup release");}
            if let Some(worker)=self.worker.take(){let _=worker.join();}
        }
    }
    fn spawn(root:&Path,mode:&str)->Running {
        let command=child(root,mode);let own=root.to_owned();let label=mode.to_owned();let retained=root.to_owned();
        let worker=std::thread::spawn(move||{
            let result=process::capture(command,Duration::from_secs(90),4*1024*1024).unwrap();
            event(&own,&format!("{label}.stdout"),&result.stdout);event(&own,&format!("{label}.stderr"),&result.stderr);
            event(&own,&format!("{label}.status"),format!("{:?}",result.status).as_bytes());
            assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
        });
        Running{root:retained,worker:Some(worker),release_final:mode=="performance-success"}
    }
    fn performance_consumer(root:&Path) {
        // A real tiny Rust executable, compiled by compiler_pair's ordinary
        // producer. --version remains the actual fixture component contract.
        // The only other operation deliberately refuses authority validation.
        let version=serde_json::json!({"ok":true,"result":{"implementation":"rust","component":"host","version":rustcast_core::product_version::CURRENT}}).to_string();
        fs::write(root.join("rust-native/src/main.rs"),format!(r#"
use std::io::Write;
fn record(root:&std::path::Path,name:&str,bytes:&[u8]) {{
    std::fs::OpenOptions::new().write(true).create_new(true).open(root.join(name)).unwrap().write_all(bytes).unwrap();
}}
fn main() {{
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args==["--version"] {{println!("{{}}",{version:?});return;}}
    let root=std::path::Path::new({root:?});
    assert_eq!(args.len(),3);assert_eq!(args[0],"validate-authority");
    assert_eq!(args[1],root.join("owned-unreviewed-authority.json").to_str().unwrap());
    let mut capability=String::new();
    for field in ["DEV","INO","UID","ROOT"] {{
        let key=format!("RUYCAST_ARTIFACT_LEASE_{{field}}");
        capability.push_str(&format!("{{key}}={{}}\n",std::env::var(&key).unwrap()));
    }}
    record(root,"pipeline-capability.txt",capability.as_bytes());
    let result=std::process::Command::new({entry:?})
        .args(["--ignored","--exact","compat::temporal_contract_tests::owned_temporal_process_entry","--test-threads=1","--nocapture"])
        .env("RUYCAST_OWNED_TEMPORAL_MODE","consumer").output().unwrap();
    record(root,"consumer.stdout",&result.stdout);record(root,"consumer.stderr",&result.stderr);
    record(root,"consumer.status",format!("{{:?}}",result.status).as_bytes());
    assert!(result.status.success());
    record(root,"authority-validation-refused",b"owned validator refuses without a benchmark result");
    eprintln!("owned disposable authority validation refused");
    std::process::exit(17);
}}
"#,root=root.to_str().unwrap(),entry=std::env::current_exe().unwrap().to_str().unwrap())).unwrap();
        fs::write(root.join("owned-unreviewed-authority.json"),b"{}\n").unwrap();
        fs::write(root.join("owned-uncollected-dataset.json"),b"{}\n").unwrap();
        // performance() verifies a real existing release; it must not be
        // handed a fabricated producer receipt or bypass verify_under_lease.
        // This disposable repository owns its own lease; do not borrow the
        // enclosing native validation pipeline's different repository lease.
        let lease=BuildLock::acquire_from_environment(root,&BTreeMap::new()).unwrap();
        build_under_lease(root,"release",&lease).unwrap();
        lease.validate().unwrap();
    }
    fn temporal_case(mode:&str) {
        // Existing fixture compiles two actual tiny Rust programs. No fake
        // compiler output, validation report, F1 data or product gate is used.
        let fixture=std::mem::ManuallyDrop::new(super::super::tests::Fixture::compiler_pair());
        let root=&fixture.0;fs::set_permissions(root,fs::Permissions::from_mode(0o700)).unwrap();
        for directory in ["Tests","scripts","docs","Sources"] {fs::create_dir(root.join(directory)).unwrap();}
        for name in ["Package.swift","README.md","CONTRIBUTING.md","Makefile"] {fs::write(root.join(name),b"owned verification input\n").unwrap();}
        event(root,".temporal-owned",b"disposable-temporal-v1");
        let receipts=root.join(".build/receipts");fs::create_dir_all(&receipts).unwrap();
        let kind=match mode {"settings"=>"settings","performance"|"performance-success"=>"performance",_=>"check"};
        let pass=receipts.join(format!("{kind}.json"));
        if !matches!(mode,"performance"|"performance-success") {fs::write(&pass,b"previous passed fixture receipt").unwrap();}
        else if mode=="performance" {performance_consumer(root);}
        let report=root.join("rust-native/target/verification/report.json");fs::create_dir_all(report.parent().unwrap()).unwrap();
        // Plain owned Hold result, explicitly not a native matrix report. Its
        // bytes are bound by the real production receipt writer below.
        fs::write(&report,b"owned disposable final-consumer report; no native/F1 acceptance\n").unwrap();
        let pipeline=spawn(root,mode);wait(root,"consumer-entered");
        assert!(!pass.exists(),"no old or fabricated pass may survive into the held consumer");
        let build_path=root.join(".build/full-rust-release/development-build.json");
        let build_bytes=fs::read(&build_path).unwrap();
        let build:Receipt=serde_json::from_slice(&build_bytes).unwrap();
        let source=root.join("rust/src/main.rs");
        let cli=root.join(artifact_paths_for(&build.target,"release").unwrap()[1].0.clone());
        let before_source=fs::read(&source).unwrap();let before_cli=fs::read(&cli).unwrap();
        event(root,"source-before.rs",&before_source);event(root,"artifact-before.bin",&before_cli);
        let contender=spawn(root,if mode=="performance" {"performance-contender"} else {"contender"});wait(root,"contender-requested");
        std::thread::sleep(Duration::from_millis(80));
        assert!(!root.join("contender-acquired").exists());
        // An independent nonblocking flock is the original test's direct
        // refusal observation; an unscheduled contender cannot fake this.
        let consumer_pid=String::from_utf8(fs::read(root.join("consumer-entered")).unwrap()).unwrap().parse::<i32>().unwrap();
        assert_eq!(unsafe{libc::kill(consumer_pid,0)},0);
        let lock=OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC)
            .open(root.join(".build/artifacts.lock")).unwrap();
        let lock_meta=lock.metadata().unwrap();
        assert!(lock_meta.is_file() && lock_meta.nlink()==1 && lock_meta.uid()==unsafe{libc::geteuid()});
        let pipeline_cap:BTreeMap<String,String>=if mode=="performance" {
            fs::read_to_string(root.join("pipeline-capability.txt")).unwrap().lines()
                .map(|line|{let (key,value)=line.split_once('=').unwrap();(key.to_owned(),value.to_owned())}).collect()
        } else {serde_json::from_slice(&fs::read(root.join("pipeline-capability.json")).unwrap()).unwrap()};
        let consumer_cap:BTreeMap<String,String>=serde_json::from_slice(&fs::read(root.join("consumer-capability.json")).unwrap()).unwrap();
        for field in ["DEV","INO","UID","ROOT"] {
            let key=format!("RUYCAST_ARTIFACT_LEASE_{field}");
            assert_eq!(pipeline_cap[&key],consumer_cap[&key]);
        }
        assert_eq!(pipeline_cap["RUYCAST_ARTIFACT_LEASE_DEV"],lock_meta.dev().to_string());
        assert_eq!(pipeline_cap["RUYCAST_ARTIFACT_LEASE_INO"],lock_meta.ino().to_string());
        let probed=unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)};
        if probed==0 {unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_UN)};panic!("held pipeline unexpectedly unlocked");}
        let errno=std::io::Error::last_os_error().raw_os_error().unwrap();
        assert_eq!(errno,libc::EWOULDBLOCK);
        assert_eq!(unsafe{libc::kill(consumer_pid,0)},0);
        event(root,"held-lock-probe.json",&serde_json::to_vec_pretty(&serde_json::json!({"dev":lock_meta.dev(),"ino":lock_meta.ino(),"uid":lock_meta.uid(),"consumerPID":consumer_pid,"flockResult":probed,"errno":errno})).unwrap());
        assert_eq!(fs::read(&source).unwrap(),before_source);assert_eq!(fs::read(&cli).unwrap(),before_cli);
        match mode {
            "none"|"settings"|"performance"|"performance-success"=>(),
            "source"=>{let mut altered=before_source.clone();altered.extend_from_slice(b"\n// edit during actual held consumer\n");fs::write(&source,altered).unwrap();},
            "artifact"=>{let mut altered=before_cli.clone();altered.extend_from_slice(b"\nchanged during held consumer\n");fs::write(&cli,altered).unwrap();},
            _=>unreachable!(),
        }
        event(root,"source-during.rs",&fs::read(&source).unwrap());event(root,"artifact-during.bin",&fs::read(&cli).unwrap());
        event(root,"release-consumer",b"release controlled final child");
        if mode=="performance-success" {
            wait(root,"successful-consumer-returned");
            // Child exited successfully and released its inherited descriptor.
            // The common production boundary must STILL hold the lock until
            // the entire successful operation (including its tail) returns.
            assert_ne!(unsafe{libc::kill(consumer_pid,0)},0);
            assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
            assert!(!root.join("contender-acquired").exists());
            let held=unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)};
            if held==0 {unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_UN)};panic!("successful consumer lost the pipeline tail lease");}
            assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::EWOULDBLOCK));
            assert_eq!(fs::read(&source).unwrap(),before_source);assert_eq!(fs::read(&cli).unwrap(),before_cli);
            event(root,"post-consumer-flock-held",b"successful child finished; outer production boundary still leased");
            event(root,"release-final-return",b"allow successful operation return");
        }
        pipeline.finish();contender.finish();
        assert!(root.join("consumer-finished").is_file());assert!(root.join("contender-acquired").is_file());
        assert_eq!(fs::read(&build_path).unwrap(),build_bytes,"final checks do not replace the genuine producer receipt");
        assert_eq!(pass.exists(),matches!(mode,"none"|"settings"));
        // The positive control proves this exact real writer is reachable; the
        // two absent receipts are not a never-called-writer/vacuous assertion.
        if mode=="performance-success" {
            assert!(root.join("post-consumer-flock-held").is_file());
            assert!(!receipts.join("performance.json").exists());
            println!("shared production lease successful disposable operation only; no F1 comparison: {}",root.display());
        } else if mode=="performance" {
            assert!(root.join("actual-refusal.txt").is_file());
            assert!(!root.join("must-not-publish-performance.json").exists());
            println!("disposable performance authority-validation/refusal lease only: {}",root.display());
        } else {println!("disposable temporal source/artifact/receipt contract: {}",root.display());}
        // Preserve exclusively owned bytes/primaries, including altered files,
        // for review. Root cleans only this fixture after evidence retention.
    }
    #[test]
    #[ignore="Explicit disposable pipeline temporal contract; not a complete check/native/F1 pass"]
    fn final_consumer_keeps_artifacts_leased_through_actual_receipt_publication(){temporal_case("none")}
    #[test]
    #[ignore="Explicit disposable pipeline source-mutation/no-pass contract"]
    fn source_mutation_during_final_consumer_cannot_publish_pipeline_pass(){temporal_case("source")}
    #[test]
    #[ignore="Explicit disposable pipeline artifact-mutation/no-pass contract"]
    fn artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass(){temporal_case("artifact")}
    #[test]
    #[ignore="Disposable settings writer and actual performance authority-refusal lease only; not successful F1 comparison"]
    fn settings_writer_and_performance_authority_refusal_keep_the_pipeline_lease(){
        temporal_case("settings");temporal_case("performance");
    }
    #[test]
    #[ignore="Successful disposable consumer and post-child tail of the real shared lease boundary; no F1 result"]
    fn shared_pipeline_lease_survives_successful_consumer_and_final_return(){temporal_case("performance-success")}
}
