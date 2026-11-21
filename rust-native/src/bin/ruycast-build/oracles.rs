//! Explicit historical gate inventory. Only source-qualified, data-only
//! oracles execute here; unqualified old-host branches must never run raw.
use super::*;
use std::collections::BTreeSet;
#[path = "../../../tests/support/hid_state_goldens.rs"]
mod hid_state_goldens;

const CORE_SMOKE_SCOPE: &str = "Rust static library and SwiftPM RuycastApp compile/link smoke only; no behavior, ABI-call, startup, AppKit or desktop acceptance";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoreReceipt {
    schema: String,
    scope: String,
    sources: BTreeMap<String, String>,
    artifacts: BTreeMap<String, String>,
    commands: Vec<Vec<String>>,
    outcome: String,
    production_accepted: bool,
    swift_compatibility: SwiftCompatibility,
    policy: CorePolicyEvidence,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwiftCompatibility {
    modulemap: String,
    architecture: String,
    alias_target: Option<String>,
}
pub(super) fn swift_alias(root: &Path, architecture: &str) -> Result<Option<String>> {
    if !matches!(architecture, "arm64-apple-macosx" | "x86_64-apple-macosx") {
        return Err("Unexpected Swift oracle architecture.".into());
    }
    let alias = root.join(".build/swift/debug");
    let expected = PathBuf::from(architecture).join("debug");
    match fs::symlink_metadata(&alias) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if fs::read_link(&alias).map_err(error)?.as_os_str() != expected.as_os_str() {
                return Err("Unexpected SwiftPM debug alias; preserve existing target.".into());
            }
            Ok(Some(expected.to_string_lossy().into_owned()))
        }
        Ok(meta) if meta.is_dir() => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(_) => Err("SwiftPM debug alias must be the expected relative link.".into()),
        Err(e) => Err(error(e)),
    }
}
fn swift_compatibility(root: &Path, scratch: &Path, bin: &Path, target: &str) -> Result<SwiftCompatibility> {
    let architecture = match target {
        "aarch64-apple-darwin" => "arm64-apple-macosx",
        "x86_64-apple-darwin" => "x86_64-apple-macosx",
        _ => return Err("Unsupported Swift oracle target.".into()),
    };
    let suffix = bin.strip_prefix(scratch).map_err(error)?;
    if suffix.as_os_str() != PathBuf::from(architecture).join("debug").as_os_str() {
        return Err("Swift oracle reported an unexpected physical debug suffix.".into());
    }
    let alias_target = swift_alias(root, architecture)?;
    let compatibility_bin = if alias_target.is_some() { suffix } else { Path::new("debug") };
    let modulemap = PathBuf::from(".build/swift").join(compatibility_bin).join("CRuycastCore.build/module.modulemap");
    Ok(SwiftCompatibility { modulemap: modulemap.to_string_lossy().into_owned(), architecture: architecture.into(), alias_target })
}
fn verify_swift_compatibility(root: &Path, binding: &SwiftCompatibility, artifacts: &BTreeMap<String, String>) -> Result<()> {
    let compatibility_bin = if binding.alias_target.is_some() { PathBuf::from(&binding.architecture).join("debug") } else { PathBuf::from("debug") };
    let expected = PathBuf::from(".build/swift").join(compatibility_bin).join("CRuycastCore.build/module.modulemap");
    if Path::new(&binding.modulemap) != expected
        || swift_alias(root, &binding.architecture)? != binding.alias_target
        || artifacts.get(&binding.modulemap) != Some(&digest(&root.join(&expected))?)
    {
        return Err("Swift oracle compatibility binding changed.".into());
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct PolicyArtifact {
    target: String, build_source: String, path: String, sha256: String, build_command: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct PolicyInvocation { arguments: Vec<String>, result: serde_json::Value }
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct CorePolicyEvidence { artifact: PolicyArtifact, before: PolicyInvocation, after: PolicyInvocation, inputs: BTreeMap<String,PolicyInput> }
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct PolicyInput { kind:String, sha256:Option<String>, link_target:Option<String> }
#[derive(Clone, Copy)]
enum PolicyTree { Legacy, Runtime, Product, Tooling }
fn policy_input_name(root:&Path,path:&Path)->Result<String> {
    path.strip_prefix(root).map_err(error)?.to_str().map(str::to_owned).ok_or("Unrepresentable policy input path.".into())
}
fn record_policy_input(root:&Path,path:&Path,kind:&str,bytes:bool,link:bool,values:&mut BTreeMap<String,PolicyInput>)->Result<()> {
    let target=if link {Some(fs::read_link(path).map_err(error)?.to_str().ok_or("Unrepresentable policy input link.")?.to_owned())} else {None};
    values.insert(policy_input_name(root,path)?,PolicyInput{kind:kind.into(),sha256:if bytes{Some(digest(path)?)}else{None},link_target:target});Ok(())
}
fn policy_input_tree(root:&Path,path:&Path,tree:PolicyTree,values:&mut BTreeMap<String,PolicyInput>)->Result<()> {
    policy_input_name(root,path)?;
    let metadata=fs::symlink_metadata(path).map_err(error)?;
    let ignored=matches!(tree,PolicyTree::Product)&&path.file_name().is_some_and(|name|name==".DS_Store")&&!path.is_dir();
    if metadata.is_symlink() {
        if ignored {return record_policy_input(root,path,"ignored-file-symlink",false,true,values);}
        return Err("Linked policy input cannot qualify a core receipt.".into());
    }
    if metadata.is_dir() {
        record_policy_input(root,path,"directory",false,false,values)?;
        for entry in fs::read_dir(path).map_err(error)? {
            let child=entry.map_err(error)?.path();
            let metadata=fs::symlink_metadata(&child).map_err(error)?;
            if matches!(tree,PolicyTree::Tooling)&&metadata.is_dir()&&child.file_name().is_some_and(|name|name=="target")&&path.join("Cargo.toml").is_file() {
                record_policy_input(root,&child,"pruned-directory",false,false,values)?;
                continue;
            }
            policy_input_tree(root,&child,tree,values)?;
        }
    } else if metadata.is_file() {
        let ignored=ignored||(matches!(tree,PolicyTree::Runtime|PolicyTree::Tooling)&&path.file_name().is_some_and(|name|name==".DS_Store"));
        record_policy_input(root,path,if ignored{"ignored-file"}else{"file"},!ignored,false,values)?;
    } else if ignored {record_policy_input(root,path,"ignored-nonregular",false,false,values)?;}
    else {return Err("Nonregular policy input cannot qualify a core receipt.".into());}
    Ok(())
}
fn core_policy_footprint(root:&Path)->Result<BTreeMap<String,PolicyInput>> {
    let mut values=BTreeMap::new();
    let legacy=fs::symlink_metadata(root.join("Sources")).map_err(error)?;
    if legacy.is_symlink()||!legacy.is_dir(){return Err("Legacy policy root must be a real directory.".into());}
    for name in ["Package.swift","rust-native/swift-reference-lock.json"] {if !fs::symlink_metadata(root.join(name)).map_err(error)?.is_file(){return Err("Policy lock and package must be regular files.".into());}}
    for name in ["Sources","Package.swift","rust-native/swift-reference-lock.json"] {policy_input_tree(root,&root.join(name),PolicyTree::Legacy,&mut values)?;}
    for name in ["rust/src","rust-native/src"] {
        let path=root.join(name);let metadata=fs::symlink_metadata(&path).map_err(error)?;
        if metadata.is_symlink()||!metadata.is_dir(){return Err("Runtime policy root must be a real directory.".into());}
        policy_input_tree(root,&path,PolicyTree::Runtime,&mut values)?;
    }
    for name in ["rust","rust-native"] {
        let product=root.join(name);let metadata=fs::symlink_metadata(&product).map_err(error)?;
        if metadata.is_symlink()||!metadata.is_dir(){return Err("Product policy root must be a real directory.".into());}
        record_policy_input(root,&product,"directory",false,false,&mut values)?;
        for tooling in ["tests","scripts"] {if product.join(tooling).is_symlink(){return Err("Tooling policy link cannot qualify a core receipt.".into());}}
        for entry in fs::read_dir(&product).map_err(error)? {
            let path=entry.map_err(error)?.path();let relative=policy_input_name(root,&path)?;
            let name=path.file_name().unwrap().to_str().ok_or("Unrepresentable policy input name.")?;
            if path.is_dir()&&["target","tests","scripts","src",".git"].contains(&name) {
                if name!="src" {record_policy_input(root,&path,if path.is_symlink(){"pruned-directory-symlink"}else{"pruned-directory"},false,path.is_symlink(),&mut values)?;}
                continue;
            }
            policy_input_tree(root,&path,PolicyTree::Product,&mut values).map_err(|e|format!("{relative}: {e}"))?;
        }
    }
    // Keep the helper roots and Cargo-output semantics aligned with rust_only_policy.rs.
    for name in ["Tests","scripts","bench","rust/tests","rust/scripts","rust-native/tests","rust-native/scripts"] {
        let path=root.join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>continue,
            Err(error)=>return Err(error.to_string()),
            Ok(metadata) if metadata.is_symlink()||!metadata.is_dir()=>return Err("Tooling policy root must be a real directory.".into()),
            Ok(_)=>policy_input_tree(root,&path,PolicyTree::Tooling,&mut values)?,
        }
    }
    Ok(values)
}
fn core_policy_build_command(target: &str) -> Result<Vec<String>> {
    artifact_paths(target)?;
    Ok(["cargo","build","--quiet","--manifest-path","rust-native/Cargo.toml","--target-dir",".build/oracle-policy","--target",target,"--bin","rust-only-policy","--locked","--offline"].into_iter().map(str::to_owned).collect())
}
fn bind_core_policy_artifact(root: &Path, target: &str) -> Result<PolicyArtifact> {
    let command=core_policy_build_command(target)?;
    let source=format!(".build/oracle-policy/{target}/debug/rust-only-policy");
    let bytes=regular_bytes(&root.join(&source))?;
    let sha=format!("{:x}",Sha256::digest(&bytes));
    let path=format!(".build/oracle-policy-artifacts/{sha}");
    validation::publish_rust_policy_snapshot(&root.join(&path),&bytes,&sha,||Ok(()))?;
    let artifact=PolicyArtifact{target:target.into(),build_source:source,path,sha256:sha,build_command:command};
    check_core_policy_artifact(root,&artifact)?;Ok(artifact)
}
fn check_core_policy_artifact(root: &Path, artifact: &PolicyArtifact) -> Result<()> {
    if artifact.build_command!=core_policy_build_command(&artifact.target)?
        || artifact.build_source!=format!(".build/oracle-policy/{}/debug/rust-only-policy",artifact.target)
        || artifact.sha256.len()!=64 || !artifact.sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
        || artifact.path!=format!(".build/oracle-policy-artifacts/{}",artifact.sha256)
        || digest(&root.join(&artifact.path))?!=artifact.sha256 {
        return Err("Pure core policy artifact or original build binding changed.".into());
    } Ok(())
}
fn core_policy_arguments(artifact: &PolicyArtifact)->Vec<String> {vec![artifact.path.clone(),"--root".into(),".".into()]}
fn execute_core_policy(root:&Path,lease:&BuildLock,artifact:&PolicyArtifact,phase:&str)->Result<PolicyInvocation> {
    if !matches!(phase,"before"|"after") {return Err("Unknown core policy phase.".into());}
    check_core_policy_artifact(root,artifact)?;
    let arguments=core_policy_arguments(artifact);let value=tool_capture(root,lease,&arguments)?;
    let prefix=root.join(format!(".build/receipts/pure-core-policy-{phase}"));
    ruycast_native::private_storage::write(&prefix.with_extension("stdout"),&value.stdout,true)?;
    ruycast_native::private_storage::write(&prefix.with_extension("stderr"),&value.stderr,true)?;
    check_core_policy_artifact(root,artifact)?;
    if !value.status.success() {return Err(format!("Rust core policy {phase} refused; inspect its stdout/stderr receipt logs."));}
    let result=validation::rust_policy_result(&value.stdout)?;
    Ok(PolicyInvocation{arguments,result})
}
fn check_core_policy_evidence(root:&Path,policy:&CorePolicyEvidence)->Result<()> {
    let lane_prefix="pure-core";
    if policy.inputs!=core_policy_footprint(root)? {return Err("Core policy input path/type/content footprint changed.".into());}
    check_core_policy_artifact(root,&policy.artifact)?;
    for (phase, invocation) in [("before",&policy.before),("after",&policy.after)] {
        if invocation.arguments!=core_policy_arguments(&policy.artifact) {return Err("Pure core policy caller changed.".into());}
        validation::rust_policy_result(&serde_json::to_vec(&invocation.result).map_err(error)?)?;
        let logged:serde_json::Value=serde_json::from_slice(&regular_bytes(&root.join(format!(".build/receipts/{lane_prefix}-policy-{phase}.stdout")))?).map_err(error)?;
        if logged!=invocation.result {return Err("Pure core policy log differs from its receipt.".into());}
    }
    if policy.before.result!=policy.after.result || policy.before.result["lockSHA256"]!=digest(&root.join("rust-native/swift-reference-lock.json"))? {
        return Err("Pure core policy inputs changed between gates.".into());
    } Ok(())
}
fn core_sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for name in [
        "Package.swift",
        "Sources/LauncherModels",
        "Sources/LauncherCore",
        "Sources/CRuycastCore",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src",
        "Sources",
        "rust-native/src",
        "rust-native/src/bin/rust_only_policy.rs",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust-native/swift-reference-lock.json",
        "rust-toolchain.toml",
    ] {
        validation::source_folder(root, &root.join(name), &mut values)?;
    }
    Ok(values)
}
fn tool_capture(root: &Path, lease: &BuildLock, args: &[String]) -> Result<process::Captured> {
    lease.validate()?;
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]).current_dir(root);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("RUYCAST_")
            || name_text.starts_with("RUSTCAST_")
            || name_text.starts_with("DYLD_")
        {
            command.env_remove(name);
        }
    }
    lease.apply_to_child(&mut command)?;
    let value = process::capture(
        command,
        std::time::Duration::from_secs(180),
        16 * 1024 * 1024,
    )?;
    Ok(value)
}
fn tool(root: &Path, lease: &BuildLock, args: &[String]) -> Result<String> {
    let value=tool_capture(root,lease,args)?;
    if !value.status.success() {
        return Err(format!(
            "Historical pure oracle tool failed: {}",
            String::from_utf8_lossy(&value.stderr)
        ));
    }
    String::from_utf8(value.stdout).map_err(error)
}
fn data_copy(root: &Path, source: &Path, destination: &Path) -> Result<()> {
    for path in [source, destination] {
        if !path.starts_with(root) || path.components().any(|part| !matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))) {
            return Err("Oracle artifact escaped repository.".into());
        }
    }
    // The destination may not exist yet; check its existing prefix before the
    // private writer creates anything, retaining the final descriptor checks.
    let mut prefix = PathBuf::new();
    for component in destination.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(format!("Linked oracle copy destination: {}", prefix.display())),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(error(e)),
        }
    }
    let before = digest(source)?;
    let bytes = regular_bytes(source)?;
    ruycast_native::private_storage::write(destination, &bytes, true)?;
    use std::os::unix::fs::PermissionsExt;
    regular_file(destination)?
        .set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(error)?;
    if digest(source)? != before || digest(destination)? != before {
        return Err("Test-only oracle library changed during copy.".into());
    }
    Ok(())
}
pub(super) fn core(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    let before = core_sources(root)?;
    let policy_inputs=core_policy_footprint(root)?;
    let target = host_target(root)?;
    tool(root,lease,&core_policy_build_command(&target)?)?;
    let policy_artifact=bind_core_policy_artifact(root,&target)?;
    let policy_before=execute_core_policy(root,lease,&policy_artifact,"before")?;
    let rust_build = vec![
        "cargo", "build", "--manifest-path", "rust/Cargo.toml", "--target-dir",
        ".build/oracle-core", "--target", target.as_str(), "--lib", "--locked", "--offline",
    ].into_iter().map(str::to_owned).collect::<Vec<_>>();
    tool(root, lease, &rust_build)?;
    let archive = root.join(format!(".build/oracle-core/{target}/debug/librustcast_core.a"));
    let compatibility_archive = root.join(".build/rust/release/librustcast_core.a");
    data_copy(root, &archive, &compatibility_archive)?;
    let archive_sha256 = digest(&archive)?;
    let commands = core_build_commands(&target, &archive_sha256)?;
    let scratch = format!(".build/oracle-swift/{archive_sha256}");
    tool(root, lease, &commands[1])?;
    let bin_path = tool(root, lease, &commands[2])?;
    let bin_path = PathBuf::from(bin_path.trim());
    if !bin_path.is_absolute()
        || !bin_path.starts_with(root.join(".build/oracle-swift"))
        || bin_path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("Swift oracle build reported an unexpected artifact path.".into());
    }
    let modulemap = bin_path.join("CRuycastCore.build/module.modulemap");
    let swift_compatibility = swift_compatibility(root, &root.join(&scratch), &bin_path, &target)?;
    let compatibility_modulemap = root.join(&swift_compatibility.modulemap);
    data_copy(root, &modulemap, &compatibility_modulemap)?;
    let binary = bin_path.join("RuycastApp");
    regular_file(&binary)?;
    let policy_after=execute_core_policy(root,lease,&policy_artifact,"after")?;
    let policy=CorePolicyEvidence{artifact:policy_artifact,before:policy_before,after:policy_after,inputs:policy_inputs};
    check_core_policy_evidence(root,&policy)?;
    if core_sources(root)? != before {
        return Err("Pure oracle source changed; no receipt published.".into());
    }
    let mut artifacts = BTreeMap::new();
    for path in [
        &archive,
        &compatibility_archive,
        &modulemap,
        &compatibility_modulemap,
        &binary,
    ] {
        artifacts.insert(
            path.strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Invalid oracle artifact path.")?
                .into(),
            digest(path)?,
        );
    }
    artifacts.insert(policy.artifact.path.clone(),policy.artifact.sha256.clone());
    verify_swift_compatibility(root, &swift_compatibility, &artifacts)?;
    let receipt=CoreReceipt{schema:"ruycast.core-build-link-smoke.v2".into(),scope:CORE_SMOKE_SCOPE.into(),sources:before,artifacts,commands,outcome:"Core build/link smoke passed".into(),production_accepted:false,swift_compatibility,policy};
    lease.validate()?;
    let path = root.join(".build/receipts/pure-core-oracle.json");
    ruycast_native::private_storage::write(
        &path,
        &serde_json::to_vec_pretty(&receipt).map_err(error)?,
        true,
    )?;
    verify_core(root, lease)
}
pub(super) fn qualify_hid_reference(
    root: &Path,
    binaries: &serde_json::Value,
    raw: &[u8],
) -> Result<serde_json::Value> {
    const ALIAS: &str = "rust-native/target/debug/hid-state-probe";
    hid_state_goldens::parse(&regular_bytes(&root.join(hid_state_goldens::PATH))?)?;
    let mut value: serde_json::Value = serde_json::from_slice(raw).map_err(error)?;
    if value["schema"] != "ruycast.hid-state-golden-binding.v1"
        || value["goldenSHA256"] != hid_state_goldens::SHA256
        || value["positiveCases"] != 4623
        || value["observedSnapshots"] != 10940
        || value["firstOperationRefusals"] != 4
        || value.get("hidGoldenQualification").is_some()
    {
        return Err("Incomplete or already-qualified HID golden receipt.".into());
    }
    let sources = value["sources"]
        .as_object()
        .ok_or("Missing HID source bindings.")?;
    if sources.len() != hid_state_goldens::SOURCES.len() {
        return Err("HID source roster changed.".into());
    }
    for name in hid_state_goldens::SOURCES {
        if sources.get(*name).and_then(serde_json::Value::as_str)
            != Some(digest(&root.join(name))?.as_str())
        {
            return Err("HID golden/producer source changed.".into());
        }
    }
    let expected = binaries
        .get(ALIAS)
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing actual HID probe producer.")?;
    let probe = value["probe"]
        .as_str()
        .ok_or("Missing live HID probe binding.")?;
    if Path::new(probe).is_absolute()
        || Path::new(probe)
            .components()
            .any(|p| !matches!(p, std::path::Component::Normal(_)))
    {
        return Err("HID probe escaped repository.".into());
    }
    let artifacts = value["artifacts"]
        .as_object()
        .ok_or("Missing HID probe artifact.")?;
    if artifacts.len() != 1
        || artifacts.get(probe).and_then(serde_json::Value::as_str) != Some(expected)
        || digest(&root.join(ALIAS))? != expected
        || digest(&root.join(probe))? != expected
    {
        return Err("HID probe copy differs from its actual producer.".into());
    }
    value["hidGoldenQualification"] = serde_json::json!({
        "goldenSHA256":hid_state_goldens::SHA256,
        "probeProducer":ALIAS,
        "probeSHA256":expected,
        "rawBindingSHA256":format!("{:x}",Sha256::digest(raw))
    });
    Ok(value)
}
fn core_build_commands(target: &str, archive_sha256: &str) -> Result<Vec<Vec<String>>> {
    if !matches!(target, "aarch64-apple-darwin" | "x86_64-apple-darwin")
        || archive_sha256.len() != 64
        || !archive_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Invalid core build/link smoke binding.".into());
    }
    let scratch = format!(".build/oracle-swift/{archive_sha256}");
    Ok(vec![
        vec![
            "cargo", "build", "--manifest-path", "rust/Cargo.toml", "--target-dir",
            ".build/oracle-core", "--target", target, "--lib", "--locked", "--offline",
        ].into_iter().map(str::to_owned).collect(),
        vec![
            "swift", "build", "--scratch-path", &scratch, "--configuration", "debug",
            "--product", "RuycastApp",
        ].into_iter().map(str::to_owned).collect(),
        vec![
            "swift", "build", "--scratch-path", &scratch, "--configuration", "debug",
            "--show-bin-path",
        ].into_iter().map(str::to_owned).collect(),
    ])
}

pub(super) fn verify_core(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let receipt: CoreReceipt = serde_json::from_slice(&regular_bytes(
        &root.join(".build/receipts/pure-core-oracle.json"),
    )?)
    .map_err(error)?;
    let target = receipt.policy.artifact.target.as_str();
    let architecture=match target{"aarch64-apple-darwin"=>"arm64-apple-macosx","x86_64-apple-darwin"=>"x86_64-apple-macosx",_=>return Err("Unexpected policy target.".into())};
    let archive_path = format!(".build/oracle-core/{target}/debug/librustcast_core.a");
    let archive_sha256 = receipt.artifacts.get(&archive_path).ok_or("Core smoke archive binding is missing.")?;
    let expected_commands = core_build_commands(target, archive_sha256)?;
    let scratch=format!(".build/oracle-swift/{archive_sha256}/{architecture}/debug");
    let modulemap=format!("{scratch}/CRuycastCore.build/module.modulemap");
    let binary=format!("{scratch}/RuycastApp");
    let compatibility_archive=".build/rust/release/librustcast_core.a";
    let expected_artifacts=BTreeSet::from([
        archive_path.clone(),modulemap.clone(),binary.clone(),compatibility_archive.into(),
        receipt.swift_compatibility.modulemap.clone(),receipt.policy.artifact.path.clone(),
    ]);
    let actual_artifacts=receipt.artifacts.keys().cloned().collect::<BTreeSet<_>>();
    if receipt.schema != "ruycast.core-build-link-smoke.v2"
        || receipt.production_accepted
        || receipt.sources != core_sources(root)?
        || actual_artifacts != expected_artifacts
        || receipt.outcome != "Core build/link smoke passed"
        || receipt.commands != expected_commands
        || receipt.scope != CORE_SMOKE_SCOPE
        || receipt.artifacts.get(&archive_path) != receipt.artifacts.get(compatibility_archive)
        || receipt.artifacts.get(&modulemap) != receipt.artifacts.get(&receipt.swift_compatibility.modulemap)
    {
        return Err("Core build/link smoke receipt is invalid or stale.".into());
    }
    check_core_policy_evidence(root,&receipt.policy)?;
    if receipt.artifacts.get(&receipt.policy.artifact.path)!=Some(&receipt.policy.artifact.sha256)
        || receipt.swift_compatibility.architecture!=architecture {return Err("Core policy lost its actual artifact/architecture role.".into());}
    verify_swift_compatibility(root, &receipt.swift_compatibility, &receipt.artifacts)?;
    for (name, expected) in &receipt.artifacts {
        if Path::new(name).is_absolute()
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || digest(&root.join(name))? != *expected
        {
            return Err("Pure oracle artifact changed.".into());
        }
    }
    serde_json::to_value(receipt).map_err(error)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsGate {
    id: &'static str,
    original_source: &'static str,
    required_native_steps: &'static [&'static str],
    native_cases: &'static [&'static str],
    unproven_invariants: &'static [&'static str],
}
// These are case mappings, not pass receipts. The normal pipeline still runs
// and verifies the complete source-bound native matrix after qualification.
const SETTINGS: &[SettingsGate] = &[
    SettingsGate { id:"--check-edit-commands", original_source:"Sources/RuycastApp/EditCommandChecks.swift", required_native_steps:&["native-light-600","native-light-760","native-dark-600","native-dark-760"], native_cases:&["native_edit::check"], unproven_invariants:&[] },
    SettingsGate { id:"--check-settings", original_source:"Sources/RuycastApp/SettingsChecks.swift", required_native_steps:&[
        "core-unit", "unit", "settings-preferences-process", "settings-keyboard-process",
        "library-process", "library-checkbox-process", "library-drafts-process", "library-navigation-process", "actions-process", "native-keyboard-process", "hid-sdk-process",
        "keyword-process", "applications-discovery-process", "app-picker-process", "capture-process", "bridge-process", "clipboard-history-host-process", "startup-process",
        "feedback-actions-process", "snippet-links-process", "snippet-process", "snippet-board-process",
        "keyword-rich-process-rust", "clipboard-rich-paste", "settings-ui-process", "snippet-html-import",
    ], native_cases:&[
        "ui_completion::settings_parity", "ui_completion::use_either_command", "ui_completion::snippet_links",
        "snippet_probe::check_html_import", "rustcast_core::tests::command_sides_are_independent",
        "hotkeys::tests::plan_preserves_carbon_flags_sided_policy_modifier_taps_and_disabled_items",
        "recorder::tests::physical_keys_sides_hyper_and_clear_match_shipped_contract",
        "runtime_health::tests::actual_hook_and_keyword_state_are_required_without_fabricating_permissions",
        "profile::tests::post_retention_snapshot_conflicts_refuse_reload_save_and_reconcile_commit",
        "startup_process::settings_rejected_registration_preserves_four_clips_bytes_and_old_callback",
        "startup_process::settings_clipboard_write_refusal_preserves_old_config_and_active_callback",
        "startup_process::settings_unchanged_reload_retries_repaired_clipboard_cleanup_immediately",
        "startup_process::settings_rejected_image_retention_preserves_asset_then_disabled_paused_acceptance_prunes_it",
        "startup_process::settings_original_sided_filter_preserves_both_routes_repeat_release_and_modifier_boundaries",
        "startup_process::settings_disabled_dryrun_and_discovered_identity_cannot_bypass_saved_policy",
        "startup_process::settings_reenable_conflict_then_alias_preservation_and_retention_fourteen_reload",
        "settings_preferences_process::general_actual_controls_write_config_preserve_unknown_fields_and_themes",
        "settings_preferences_process::record_both_shortcuts_local_events_cancel_and_navigation_release",
        "settings_preferences_process::advanced_reload_open_permissions_and_fake_folder_cancel_stale_guard",
        "settings_preferences_process::menu_bar_hidden_substitution_live_visibility_and_retired_callbacks",
        "settings_keyboard_process::general_recorder_uses_shared_lease_and_cannot_resume_peer_recorder",
        "settings_keyboard_process::general_recorded_hotkey_replaces_physical_plan_through_config_staging",
        "library_lifecycle_process::toolbar_add_is_private_until_save_then_shared_with_palette",
        "library_lifecycle_process::invalid_config_keeps_both_last_good_views_and_refuses_draft_save",
        "library_lifecycle_process::theme_and_application_resign_update_both_windows",
        "library_lifecycle_process::hidden_settings_callbacks_do_not_change_navigation_or_cancel_palette",
        "library_lifecycle_process::normal_quit_with_both_windows_and_drafts_releases_socket",
        "library_checkbox_process::inline_enabled_checkbox_targets_unselected_row_and_updates_palette",
        "library_checkbox_process::first_checkbox_click_after_same_results_query_and_reopen_uses_current_binding",
        "library_checkbox_process::old_checkbox_cannot_write_after_query_reload_close_or_editor_open",
        "library_checkbox_process::inline_checkbox_failure_keeps_published_value_and_allows_retry",
        "library_checkbox_process::inline_enable_does_not_discard_a_palette_draft",
        "library_drafts_process::different_window_drafts_merge_and_keep_each_others_focus",
        "library_drafts_process::same_field_conflict_retains_second_draft",
        "library_drafts_process::cmd_w_closes_only_settings_and_discards_only_its_draft",
        "library_drafts_process::repeated_show_preserves_actual_alias_and_name_editor_focus",
        "library_drafts_process::cmd_w_is_recorded_before_window_close_and_repeated_show_keeps_recording",
        "library_navigation_process::cli_settings_is_separate_and_reusable_without_showing_a_window",
        "library_navigation_process::disabled_items_can_be_found_and_enabled_without_execution",
        "library_navigation_process::tabs_groups_filters_and_query_reset_keep_all_saved_items",
        "library_navigation_process::library_has_no_launcher_hundred_item_cap",
        "actions_process::actions_keyboard_badges_and_escape_keeps_palette",
        "actions_process::alias_save_clear_and_cli_resolution", "actions_process::alias_conflict_keeps_draft_and_allows_repair",
        "actions_process::failed_async_save_restores_native_field_focus_and_undo", "actions_process::other_external_fields_merge_but_same_alias_change_conflicts",
        "actions_process::deleted_item_and_invalid_schema_are_not_overwritten", "actions_process::hotkey_sides_hyper_delete_and_repeat",
        "actions_process::hotkey_conflict_and_solo_modifiers", "actions_process::recorder_stops_on_window_app_resign_unfocus_hide_and_cancel",
        "actions_process::alias_native_composition_does_not_save_or_cancel", "actions_process::remove_confirmation_cancel_then_commit_and_disable",
        "actions_process::visible_selection_is_bound_and_unseen_selection_is_refused", "actions_process::busy_config_lock_keeps_draft_and_retries",
        "actions_process::queued_save_cancel_and_hide_prevent_dispatch_and_zombie_editor", "actions_process::queued_menu_return_cancel_does_not_open_editor",
        "actions_process::forms_and_actions_render_with_long_titles_in_light_and_dark",
        "native_keyboard_process::native_callback_toggles_launcher_and_two_queued_presses_preserve_toggle_order",
        "native_keyboard_process::item_callback_uses_exact_command_not_selected_row_and_logs_hotkey_without_showing_palette",
        "native_keyboard_process::hotkey_snippet_failure_keeps_exact_analytics_source_before_missing_sdk_refusal",
        "native_keyboard_process::reload_retires_old_ids_and_disabled_bindings",
        "native_keyboard_process::actual_recording_waits_for_physical_unregistration_and_restores_accepted_plan",
        "native_keyboard_process::two_window_recorders_do_not_resume_on_other_window_cancel",
        "native_keyboard_process::headless_hidden_does_not_enable_keyboard_without_explicit_optin",
        "native_keyboard_process::missing_owned_sdk_stamp_fails_before_any_global_target_or_handler",
        "native_keyboard_process::positive_held_main_registration_keeps_owner_responsive_and_profile_locked_until_retirement",
        "native_keyboard_process::launcher_from_hotkey_report_returns_to_commands_before_toggle_hides",
        "native_keyboard_process::failed_registration_does_not_publish_editor_save_and_old_routes_survive",
        "native_keyboard_process::pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold",
        "native_keyboard_process::callback_during_commit_before_main_ack_is_deferred_until_exact_store_adoption",
        "native_keyboard_process::recorder_begin_reenters_registration_blocks_immediately_and_retires_new_handle",
        "native_keyboard_process::current_native_copy_and_recorder_error_renders_in_both_themes",
        "snippet_process::plain_unknown_modes_and_complete_literal_text",
        "snippet_process::teams_paragraphs_html_breaks_and_safe_escaping",
        "snippet_process::links_balanced_parentheses_escape_labels_and_invalid_syntax",
        "snippet_process::url_validation_and_link_source_preserve_actual_bridge_behavior",
        "snippet_process::utf16_cursor_mapping_at_every_position_and_grapheme_boundary",
        "snippet_process::native_named_board_contains_one_complete_plain_or_rich_item",
        "snippet_process::grapheme_suffix_count_across_scripts_controls_and_joiners",
        "snippet_process::real_sdk_item_failure_keeps_previous_board_but_write_failure_is_partial",
        "snippet_process::large_source_is_complete_without_an_application_quota",
        "snippet_process::generated_unicode_and_markup_corpus",
        "snippet_process::invalid_long_url_does_not_repeat_legacy_exponential_match",
        "keyword_rich_process::plain_leaves_existing_clipboard_intact",
        "keyword_rich_process::html_and_teams_prepare_item_then_erase_and_command_v",
        "keyword_rich_process::item_preparation_failure_never_clears_or_consumes",
        "keyword_rich_process::clear_write_and_partial_failures_pass_trigger",
        "keyword_rich_process::external_replacement_is_not_owned_or_pasted",
        "keyword_rich_process::foreign_html_with_same_plain_text_is_not_suppressed",
        "keyword_rich_process::clipboard_only_cancel_drop_or_panic_never_swallows_trigger",
        "keyword_rich_process::external_board_change_after_first_erase_stops_before_paste",
        "keyword_rich_process::entered_paste_down_retains_cleared_up_on_revoke_and_panic",
        "keyword_rich_process::rich_cursor_is_delayed_and_not_sent_with_paste",
        "keyword_rich_process::cursor_cancelled_by_input_record_hyper_activation_generation_and_target",
        "keyword_rich_process::sampled_loss_or_pending_rollback_never_revives_old_cursor",
        "keyword_rich_process::own_generated_input_and_unrelated_copy_do_not_cancel_paste_cursor",
        "keyword_rich_process::large_cursor_runs_bounded_batches_and_allows_cancellation",
        "keyword_rich_process::cursor_panic_balances_keyup_and_poison_only_keyword_component",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-interactions", original_source:"Sources/RuycastApp/InteractionChecks.swift", required_native_steps:&["bridge-process","native-profile-process","input-process-rust","native-choice-host-process","native-open-basic-process","creation-process","library-process","settings-preferences-process","feedback-actions-process","native-snippet-choice-process-rust","native-light-600","native-light-760","native-dark-600","native-dark-760"], native_cases:&[
        "bridge_process::show_search_and_local_native_query_share_the_owner",
        "bridge_process::rapid_arrows_and_click_on_other_row_keep_intended_identity",
        "input_process::expand_preserves_actual_field_editor_text_and_token_without_execution",
        "input_process::marked_text_blocks_run_and_escape_without_effects",
        "native_open_basic_process::saved_app_hides_exact_panel_before_sdk_and_records_success",
        "creation_process::external_changes_survive_and_identity_collision_never_overwrites",
        "library_lifecycle_process::toolbar_add_is_private_until_save_then_shared_with_palette",
        "settings_preferences_process::general_actual_controls_write_config_preserve_unknown_fields_and_themes",
        "native_choice_host_process::live_trigger_uses_original_anchor_and_single_use_replacement",
        "native_choice_host_process::changed_anchor_and_secure_policy_fail_closed",
        "ui_completion::feedback_actions",
        "native_edit::check_snippet_replacement",
        "native_snippet_choice_process::option_selection_and_arrow_clamping_without_os_menu",
        "native_snippet_choice_process::repeat_modifiers_and_marked_text_do_not_submit",
        "native_snippet_choice_process::cancel_and_external_hide_have_distinct_restore_policy",
        "native_open_basic_process::new_show_after_sdk_entry_cannot_be_hidden_or_receive_old_failure",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-actions-menu", original_source:"Sources/RuycastApp/ActionMenuChecks.swift", required_native_steps:&["actions-process","actions-menu-process","input-process-rust","feedback-actions-process","snippet-hotkey-process","native-keyboard-process","polish-original-process","item-binding-original-process"], native_cases:&[
        "ui_completion::actions_menu",
        "ui_completion::feedback_actions",
        "ui_completion::snippet_hotkey",
        "ui_completion::polish_original",
        "actions_process::alias_save_clear_and_cli_resolution",
        "actions_process::hotkey_sides_hyper_delete_and_repeat",
        "actions_process::hotkey_conflict_and_solo_modifiers",
        "actions_process::recorder_stops_on_window_app_resign_unfocus_hide_and_cancel",
        "input_process::alias_and_hotkey_saves_resume_same_input",
        "native_keyboard_process::pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold",
        "ui_completion::item_binding_original",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-browser-tabs", original_source:"Sources/RuycastApp/BrowserTabChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::safari_position_snapshot_never_falls_back_to_matching_title_or_url",
        "system_search::tests::safari_original_snapshot_mutations_fail_before_the_next_effect",
        "system_search::tests::safari_selection_mismatch_stops_before_foreground_effect",
        "system_search::tests::safari_short_cache_reuses_tokens_and_case_insensitive_url_filter_without_reads",
        "system_search::tests::slow_safari_discovery_does_not_extend_snapshot_cache_lifetime",
        "system_search::tests::safari_missing_empty_and_original_collection_and_metadata_bounds",
        "system_search::providers::gate_tests::safari_thirty_second_target_expiry_refuses_before_any_native_read_or_effect",
        "system_search::tests::root_and_dedicated_tokens_cannot_cross_stores_or_retarget_old_results",
        "system_search::tests::optional_main_tab_caps_preserve_metadata_and_dedicated_results",
        "system_search::tests::safari_cancelled_consent_never_reaches_the_owned_permission_effect",
        "runtime::system::shortcut_budget_tests::browser_expired_queued_activation_never_reaches_main_queue_eligible_or_effect",
        "runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response",
        "runtime::system::shortcut_budget_tests::browser_dedicated_busy_and_expired_cli_requests_remain_errors",
        "runtime::system::shortcut_budget_tests::browser_eight_pending_cli_requests_do_not_block_live_local_root_results",
        "runtime::system::shortcut_budget_tests::browser_late_main_provider_preserves_navigation_without_retargeting_actions",
        "runtime::system::shortcut_budget_tests::browser_late_main_provider_never_crosses_hide_or_mode_change",
        "runtime::system::shortcut_budget_tests::browser_superseded_query_and_old_deadline_cannot_cancel_or_republish_latest_rows",
        "runtime::system::shortcut_budget_tests::browser_main_missing_denied_throttle_cache_and_explicit_refresh_never_prompt",
        "runtime::system::shortcut_budget_tests::browser_safari_send_crossing_the_real_deadline_stops_before_foreground_effects",
        "runtime::system::shortcut_budget_tests::held_consent_navigation_refreshes_without_resetting_the_selected_row",
        "runtime::system::shortcut_budget_tests::held_consent_default_and_closed_actions_choose_the_new_default_tab",
        "runtime::system::shortcut_budget_tests::held_consent_down_up_and_direct_selection_preserve_user_navigation",
        "runtime::system::shortcut_budget_tests::held_consent_current_query_is_refreshed_once_without_old_query_rows",
        "runtime::system::shortcut_budget_tests::held_consent_rejects_current_interaction_hide_mode_and_configuration_changes",
        "runtime::system::shortcut_budget_tests::held_denied_consent_refreshes_local_results_once_without_granting_access",
        "system_search_sdk::browser_original_native_return_and_forty_eight_hidden_variants_are_owned",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-apple-shortcuts", original_source:"Sources/RuycastApp/AppleShortcutChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk","private-results-process","script-process","script-result-process","script-result-native-process"], native_cases:&[
        "system_search::tests::shortcut_uuid_parser_overrides_and_execution_tuple_are_exact",
        "system_search::tests::owned_shortcut_children_drain_both_pipes_and_never_wait_for_inherited_eof",
        "system_search::tests::generated_uuid_and_builtin_bindings_persist_without_resurrecting_deleted_saved_rows",
        "system_search::providers::gate_tests::shortcut_discovery_cache_hit_thirty_one_second_expiry_and_explicit_refresh",
        "system_search::apple_shortcuts::tests::workflow_output_and_private_failures_never_become_ui_or_public_error_data",
        "system_search::apple_shortcuts::tests::elapsed_capture_deadline_reports_unknown_result_without_reinterpreting_output",
        "system_search_sdk::shortcuts_original_native_return_and_twenty_four_hidden_states_keep_uuid_identity",
        "system_search_cli_process::cli_shortcut_uuid_duplicate_titles_bindings_and_private_output",
        "system_search_cli_process::native_shortcut_alias_override_and_reopen",
        "script_result_basic_process::cli_preserves_choices_detail_unknown_fields_and_success_usage",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-window-search", original_source:"Sources/RuycastApp/WindowSearchChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact",
        "system_search::tests::window_post_activation_pid_role_title_membership_and_restore_state_are_rechecked",
        "system_search::tests::window_unknown_minimized_untitled_and_old_snapshot_metadata_are_preserved",
        "system_search::tests::window_invalid_identity_and_unreadable_membership_refuse_before_activation",
        "system_search::tests::window_permission_and_app_exit_during_activation_stop_before_raise",
        "system_search::tests::window_restore_status_readback_and_membership_are_verified_before_raise",
        "system_search::tests::window_raise_error_is_not_reported_as_success",
        "system_search::tests::window_scanner_caps_two_hundred_apps_fifty_windows_and_one_hundred_results",
        "system_search::tests::window_internal_two_second_budget_returns_partial_while_request_is_live",
        "system_search::tests::window_internal_two_second_budget_can_return_empty_without_cancelling_request",
        "system_search::tests::window_original_application_and_title_filters_preserve_distinct_duplicate_targets",
        "system_search::tests::absolute_deadline_and_cancellation_stop_traversal_and_every_effect",
        "system_search::tests::result_caps_and_ax_permission_recovery_are_bounded",
        "system_search::tests::scan_platform_error_is_not_reclassified_as_internal_budget_exhaustion",
        "runtime::system::shortcut_budget_tests::ax_request_budget_windows_delivers_original_internal_partial_results",
        "system_search_sdk::window_original_native_return_and_twenty_four_hidden_states_preserve_durable_choices",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-menu-actions", original_source:"Sources/RuycastApp/MenuActionChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::app_shortcuts_decode_without_registering_launcher_hotkeys",
        "system_search::tests::menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed",
        "system_search::tests::detached_menu_path_is_never_replaced_by_a_same_title_item",
        "system_search::tests::menu_unknown_enabled_and_unreadable_submenu_never_offer_an_executable_leaf",
        "system_search::tests::menu_original_identity_must_match_the_complete_captured_identity",
        "system_search::tests::menu_post_activation_item_ancestor_identity_and_trust_are_revalidated",
        "system_search::tests::menu_scanner_caps_results_nodes_and_depth_without_visiting_hidden_tail",
        "system_search::tests::menu_nonmatching_scan_preserves_original_attribute_read_budget",
        "system_search::tests::menu_internal_two_second_budget_stops_while_request_is_live",
        "system_search::tests::menu_node_cap_must_not_turn_a_submenu_into_a_leaf",
        "system_search::tests::scan_platform_error_is_not_reclassified_as_internal_budget_exhaustion",
        "system_search::tests::menu_initial_identity_validation_precedes_the_internal_scan_budget",
        "system_search::tests::menu_original_four_items_duplicate_targets_queries_and_immutable_metadata_are_exact",
        "system_search::tests::menu_internal_two_second_budget_returns_empty_after_entered_reads_while_request_is_live",
        "system_search::tests::absolute_deadline_and_cancellation_stop_traversal_and_every_effect",
        "system_search::tests::result_caps_and_ax_permission_recovery_are_bounded",
        "system_search::native::tests::owned_cf_menu_attribute_consumers_refuse_wrong_types_and_preserve_values",
        "runtime::system::shortcut_budget_tests::ax_request_budget_menus_delivers_original_internal_partial_results",
        "runtime::system::shortcut_budget_tests::ax_request_budget_menu_identity_does_not_spend_private_traversal_clock",
        "system_search_sdk::menu_original_native_return_and_twenty_four_hidden_states_preserve_captured_actions",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-render-updates", original_source:"Sources/RuycastApp/InteractionChecks.swift", required_native_steps:&["render-updates-process"], native_cases:&[
        "ui_completion::render_updates",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-feedback", original_source:"Sources/RuycastApp/FeedbackChecks.swift", required_native_steps:&["unit","creation-process","input-process-rust","actions-process","feedback-actions-process","utility-process","polish-original-process","system-search-cli-process","remaining-macos-process"], native_cases:&[
        "ui_completion::feedback_actions",
        "ui_completion::polish_original",
        "utility_process::quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries",
        "actions::tests::binding_save_preserves_external_execution_definition",
        "actions_process::remove_confirmation_cancel_then_commit_and_disable",
        "input_process::alias_and_hotkey_saves_resume_same_input",
        "processes::tests::matching_self_and_foreign_uid_identities_refuse_before_termination",
        "system_search::providers::gate_tests::cross_kind_and_missing_opaque_tokens_refuse_before_native_reads_or_effects",
        "ui_completion::closed_creation",
        "creation_process::empty_results_keep_all_three_creation_actions_available",
        "processes::tests::owned_table_sort_and_immutable_kill_identity",
        "system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact",
        "system_search::tests::menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed",
        "system_search::tests::menu_original_identity_must_match_the_complete_captured_identity",
        "system_search::tests::menu_snapshot_tokens_never_redirect_to_replaced_leaf",
        "system_actions::tests::complete_owned_keyboard_sdk_pairs_and_tags_lock_shortcut",
        "system_actions::tests::denied_and_failed_construction_never_post",
        "file_search::tests::literal_predicate_and_normalization",
        "system_search::tests::main_service_queued_effect_is_revoked_before_native_adapter",
        "runtime::system::shortcut_budget_tests::system_search_worker_completion_requires_current_owner_adoption_before_publication",
        "system_search_cli_process::superseded_discovery_config_change_and_cli_disconnect_cannot_publish",
        "remaining_macos_process::real_cli_window_files_processes_and_monitor_data_use_owned_backends",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-rich-resources", original_source:"Sources/RuycastApp/RichResourceChecks.swift", required_native_steps:&["unit","clipboard-preview-original","remaining-macos-process","clipboard-history-host-process","clipboard-rich-paste"], native_cases:&[
        "remaining_macos_process::real_cli_window_files_processes_and_monitor_data_use_owned_backends",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
        "remaining_macos_process::native_process_sort_refresh_keeps_identity_and_kill_keeps_palette_open",
        "remaining_macos_process::vanished_native_process_selection_cannot_kill_the_remaining_owned_row",
        "remaining_macos_process::native_clipboard_two_color_image_file_pixels_and_idle_text_are_preserved_in_both_themes",
        "remaining_macos_process::native_monitor_delegate_pause_resume_close_and_reopen_use_actual_callback_route",
        "remaining_macos_process::real_read_only_resource_sample_satisfies_original_bounds_without_serializing_metadata",
        "remaining_macos_process::native_monitor_six_tabs_light_dark_minimum_wide_and_no_ordering",
        "remaining_macos_process::actual_cli_monitor_and_process_query_use_the_owned_running_host",
        "clipboard_preview::tests::original_two_color_image_and_local_file_previews_preserve_pixels_and_refuse_bad_assets",
        "system_search::providers::gate_tests::concurrent_process_queries_keep_each_result_and_metadata_snapshot",
        "system_search::providers::gate_tests::process_result_cap_bounds_opaque_target_registration",
        "processes::tests::owned_table_sort_and_immutable_kill_identity",
        "monitor::lifecycle_tests::visible_only_two_second_sampling_history_bound_and_reopen_preserve_state",
        "clipboard_history_host_process::test_native_text_image_empty_error_busy_light_dark_narrow_wide",
        "clipboard_rich_paste::file_and_image_paste_complete_owned_ticket_without_recapture",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-analytics", original_source:"Sources/RuycastApp/AnalyticsChecks.swift", required_native_steps:&["unit","analytics-runtime-process","analytics-cast-reference","dashboard-presentation-process","native-dashboard-dialog-process","dashboard-export-process","startup-process"], native_cases:&[
        "analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention",
        "analytics::tests::opening_prunes_events_outside_configured_retention",
        "analytics::tests::completion_survives_pause_but_reset_retires_old_ticket_and_private_export",
        "analytics::tests::invalid_known_file_is_not_overwritten_until_explicit_reset",
        "analytics::tests::populated_schema_one_upgrade_preserves_every_legacy_field_and_reload",
        "analytics::tests::repeated_dst_hour_keeps_attempt_completion_and_timezone_reload_distinct",
        "analytics::tests::hourly_retention_accepts_2161_prunes_on_advance_and_never_resurrects",
        "startup_process::analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch",
        "startup_process::analytics_same_command_tracks_actual_cli_launcher_and_owned_carbon_callback",
        "analytics_cast_reference::sample_caps_error_whitelist_and_hashed_raw_utf8_identity",
        "analytics_cast_reference::existing_document_rejection_and_date_codec_match_foundation",
        "dashboard_presentation_reference::populated_top30_hash_ties_names_samples_errors_and_legacy",
        "dashboard_presentation_reference::elapsed_hours_retention_dst_and_calendar_year",
        "native_dashboard_dialog_process::populated_paused_errors_confirmation_roles_values_and_local_key_loop",
        "native_dashboard_dialog_process::owned_save_dialog_takes_current_data_and_cancel_writes_nothing",
        "native_dashboard_dialog_process::actual_dashboard_renders_themes_widths_and_preserves_scroll",
        "native_dashboard_dialog_process::standard_builtin_cli_native_sources_usage_logs_and_override",
        "native_dashboard_dialog_process::builtin_input_validation_and_native_prompt_do_not_log_private_values",
        "native_dashboard_dialog_process::management_routing_controls_custom_retention_and_reset",
        "native_dashboard_dialog_process::repeated_confirmation_retires_old_queued_reset_callback",
        "native_dashboard_dialog_process::retired_controls_and_dialog_cannot_mutate_a_reopened_dashboard",
        "dashboard_export_process::positive_held_export_global_reset_order_and_parent_mode",
        "dashboard_export_process::file_sync_and_rename_failure_preserve_prior_destination",
        "dashboard_export_process::postrename_parent_sync_failure_reports_published_outcome",
        "dashboard_export_process::close_before_commit_revokes_destination_without_feedback_to_reopened_window",
        "dashboard_export_process::commit_entered_before_close_keeps_published_outcome_and_retires_feedback",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-performance-pipeline", original_source:"Sources/RuycastApp/PerformancePipelineChecks.swift", required_native_steps:&["unit","clipboard-history-host-process","clipboard-rich-paste"], native_cases:&[
        "clipboard_history::tests::capture_admission_is_single_flight_and_lease_drop_reopens_it",
        "configuration_jobs::tests::capture_retention_handshake_preserves_both_orders_and_stops_without_owner_lock",
        "native_services::check_owned_poll",
        "clipboard_rich_paste::file_and_image_paste_complete_owned_ticket_without_recapture",
        "clipboard_history_host_process::test_capture_text_files_image_and_private_own_activation_filters",
        "clipboard_history_host_process::test_pause_disable_and_source_exclusion",
        "runtime::inline_tests::clipboard_pipeline_admitted_maintenance_waits_for_held_capture",
        "runtime::inline_tests::clipboard_pipeline_unicode_latest_identity_hidden_mutation_and_reentry_use_real_owner",
        "runtime::inline_tests::clipboard_hidden_publication_prunes_ids_and_refreshes_payload_without_searching",
        "runtime::inline_tests::clipboard_query_and_data_change_reset_selection_before_the_same_publication",
        "runtime::inline_tests::clipboard_pipeline_observed_atomic_inplace_invalid_and_identical_recovery_reach_owner_views",
        "runtime::inline_tests::clipboard_pipeline_twenty_unrelated_atomic_writes_do_not_notify_configuration_owner",
        "clipboard_history::tests::accepted_image_reopens_with_identical_metadata_disk_and_decodable_asset",
        "clipboard_history::tests::capture_cleanup_failure_keeps_published_text_and_reports_error",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-visual-assets", original_source:"Sources/RuycastApp/VisualAssetChecks.swift", required_native_steps:&["unit","native-profile-process"], native_cases:&[
        "presentation::icon_tests::original_compact_keycaps_and_sided_accessibility_labels",
        "website_icons::tests::origin_and_candidate_privacy",
        "website_icons::tests::actual_transport_stream_bounds_status_cookies_and_redirects",
        "website_icons::tests::four_workers_cache_publication_and_cancel_join",
        "website_icons::tests::two_thousand_origins_bound_admission_and_deduplicate_while_four_workers_wait",
        "website_icons::tests::cache_special_files_and_close_are_bounded",
        "website_icons::tests::active_download_close_cancels_and_joins_without_cache_publication",
        "native_rows::check_cache",
        "native_process::loaded_config_alias_unicode_disabled_and_native_labels",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-command-log", original_source:"Sources/RuycastApp/CommandLogChecks.swift", required_native_steps:&["unit","private-results-process"], native_cases:&[
        "activity::tests::credential_patterns_input_and_home_are_redacted",
        "activity::tests::load_prunes_retention_and_whitelists_unknown_payloads",
        "activity::tests::malformed_optional_types_preserve_existing_log_until_clear",
        "activity::tests::existing_log_is_restricted_before_decoding_and_unqualified_files_stay_untouched",
        "activity::tests::original_log_bounds_filters_reload_and_clear_contract",
        "activity::tests::insecure_or_linked_profile_refuses_log_read_and_never_repairs_its_mode",
        "activity::tests::response_metadata_redacts_inputs_credentials_urls_home_and_unlisted_details",
        "activity::tests::response_metadata_respects_utf8_limits_and_success_never_records_stdout_or_error",
        "activity::tests::complete_metadata_budget_survives_save_reopen_and_retains_newest_entries",
        "private_results::provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace",
        "private_results::synchronous_management_failure_log_keeps_message_and_trace",
        "private_results::synchronous_history_failure_log_keeps_message_and_trace",
        "runtime::system::shortcut_budget_tests::system_action_error_log_retains_worker_message_and_response_trace",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-polish", original_source:"Sources/RuycastApp/PolishChecks.swift", required_native_steps:&["unit","polish-original-process","script-advanced-process","task-date-process","polish-bindings-process","remaining-macos-process","clipboard-preview-cancel-original", "clipboard-preview-owner-different-id", "clipboard-preview-cancel-before-publish", "clipboard-preview-failure-after-success", "clipboard-preview-stale-success-same-id"], native_cases:&[
        "ui_completion::polish_original",
        "ui_completion::script_advanced",
        "ui_completion::task_date",
        "ui_completion::polish_bindings",
        "native_menu_bar::tests::genuine_health_images_preserve_template_state_and_accessible_description",
        "runtime_health::tests::actual_hook_and_keyword_state_are_required_without_fabricating_permissions",
        "management_commands::catalog_tests::original_nine_defaults_keep_source_order_and_saved_overrides",
        "clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset",
        "clipboard_preview::tests::equal_metadata_different_id_replaces_preview_owner_and_releases_old_image",
        "clipboard_preview::tests::decoded_preview_cancelled_before_poll_is_not_published_and_released",
        "clipboard_preview::tests::failed_reopen_releases_published_preview_and_clears_on_deselect",
        "clipboard_preview::tests::stale_success_cannot_replace_same_id_reopened_preview",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
        "task_date::tests::original_natural_date_vectors_and_labels_match",
    ], unproven_invariants:&[] },
];
const LOG_SOURCE_SHA256: &str = "15c93afc3731db2ca870db6d824dcc90bc6f0f367fe9d3ca12497b90f26df97a";
const LOG_ORACLE_SHA256: &str = "9dd2ff68552ebcae0979530623309cafeb7a145a193444dc55a717e94a953455";
const EDIT_COMMAND_ORACLE_SHA256: &str = "a91248845b9256c30f2906adc818e5e706034b6277a52519df2f8758203b1145";
const VISUAL_ASSET_ORACLE_SHA256: &str = "4bd5994c6cc34a504e3f1576a0a21ca903636ed8293986ccde561a4c3b7304f0";
const RENDER_UPDATES_ORACLE_SHA256: &str = "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858";
const ANALYTICS_ORACLE_SHA256: &str = "b7e97f390de005268b1a7aa0ceeda22ca165bb2a84f8fcf1917858c4dcb73876";
const PERFORMANCE_PIPELINE_ORACLE_SHA256: &str = "429130589dd29b1c12fea7854b9e868b9d365f85ec8334a80782da999fa90659";
const BROWSER_TABS_ORACLE_SHA256: &str = "82189e7c80a7baab3e38a8b34619de06f343c21ddf3b023df2bf0cafd6f0ebb9";
const WINDOW_SEARCH_ORACLE_SHA256: &str = "cae0f60d635d6aa5af41d8fe8ecc9bc8102c28e49e2aec87a7e53462829ddf7f";
const MENU_ACTIONS_ORACLE_SHA256: &str = "41d224340e00d1970fb953aad90c3c4a539e6af84634b1aa04b90972c166f1ed";
const ACTIONS_MENU_ORACLE_SHA256: &str = "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f";
const RICH_RESOURCES_ORACLE_SHA256: &str = "5c2322b47bc436f51103e022a9878a53f4cb1565952d165e83aa63cd03e31eda";
const APPLE_SHORTCUTS_ORACLE_SHA256: &str = "a5e0806bf33b5a4cbe365b4861e7c6143c5f7ca5a6c10ce347239af8da4fce74";
const POLISH_ORACLE_SHA256: &str = "0f43dfcf6376d18d153ea857e40d27e3f4a27ca29bf5eea3c5a901120b38088b";
const SETTINGS_ORACLE_SHA256: &str = "cb2b26a91a94e7537d1db308b272945ea7002946100245e84c70954b18ce9e1d";
const FEEDBACK_ORACLE_SHA256: &str = "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde";
const SYSTEM_CHECKS_ORACLE_SHA256: &str = "c70cc5149a16a43102cc8a853bfb5a38bc6a9b3b542b4f6b7388339f1737ffa4";
const SNIPPET_EDITING_ORACLE_SHA256: &str = "adfae471c755454375ff7af2b1c99108ab08bd4a9c2a50556809e2dad5f0b99e";
fn settings_sources(id: &str) -> &'static [(&'static str, &'static [u8], &'static str)] {
    match id {
        "--check-polish" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/native_settings.rs", include_bytes!("../../native_settings.rs"), "5d662f5ad70e3be4407acf74babbf97345ac0b05079fcdd88069181d63a23041"),
            ("rust-native/src/native_snippet_editor.rs", include_bytes!("../../native_snippet_editor.rs"), "c047b34e4abf31b111d804f9bc1e5a9efe75a002e5e0ac612c0cb48fe71d6150"),
            ("rust-native/src/script_result.rs", include_bytes!("../../script_result.rs"), "6e311e6045f6f3e5d3bf8998e7e34d23323ad324d449dcac3be1ecb1e9547cbf"),
            ("rust-native/src/task_date.rs", include_bytes!("../../task_date.rs"), "ebc3df3a84c149af7f6fd6bcabea58073ea1209204e01b3a14571c3f393db856"),
            ("rust-native/src/runtime_health.rs", include_bytes!("../../runtime_health.rs"), "ee787b5ccd193c1f1a400ce0da4798132d8dd68d48f85f40b63a776cfccf7d86"),
            ("rust-native/src/native_menu_bar.rs", include_bytes!("../../native_menu_bar.rs"), "6c87e3f883b6f0abfbd96f3df1733a0e889fe347465f27fc55b3b81d3263d9d3"),
            ("rust-native/src/management_commands.rs", include_bytes!("../../management_commands.rs"), "14f68e0678ba748a65fc88a200f3cea59b9c9de8950462c2369ce5c526c75a31"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "1406185c311781ff5be404caaa0d92ac76746a68f44fb54e3eedff5419241738"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "59bc0cc716ba16dea07bbd20d1f8951ed3b4580c7cc7ea9a56116156b9c17556"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/catalog.rs", include_bytes!("../../catalog.rs"), "a9848cc7ab9e9432d4ae87c5a3ffb75b3f5a709bbe2cc83a803e19cc916fa72b"),
            ("rust-native/src/performance_trace.rs", include_bytes!("../../performance_trace.rs"), "f00bf9774d869674d3680ac57c58d7e429b436640f2a5fad61acae5b5e441781"),
            ("rust-native/src/performance_signposts.rs", include_bytes!("../../performance_signposts.rs"), "c0dcaaaf7b427d9363f536d2bc81ff097efc8042e1778e09fc6eac3302df1dcd"),
            ("rust-native/src/control.rs", include_bytes!("../../control.rs"), "ddf75e56e36fe83d474355b4e11c543e4ce87a5111577d21e9a9386ff26203b1"),
            ("rust-native/src/lib.rs", include_bytes!("../../lib.rs"), "ec3b20d3a35fe914df778b4ebdc8a3ba9e7a0c23d98386c94fa65ef518fdd732"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "e870bc1f9ea23e6e36b2157531b6086baa1b2a85f8c033469581810376de19de"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "1946a4cb06bc145fbdcfa2a4e7c9367893fbe8a23c8fbab02a490ea0bc32a886"),
        ],
        "--check-feedback" => &[
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "ffc22cd5744d2f9fa0f6928a8465dcf77602c89d1279e6e1595121b0231e2ca3"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "efaba95818f4668834be05942525865bb261de779f08f702a25babbbe4d779dc"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/src/benchmark_host/rpc.rs", include_bytes!("../../benchmark_host/rpc.rs"), "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            // Feedback and the retired nested system helper have scoped Rust
            // evidence; historical helper provenance is not an active caller,
            // a host/matrix grant or exact callback-thread/token parity.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "ed5a3adb94c3e9c24feb333265969c18e197a62d3928508a0e5f33c414527d17"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "7ac4dfb6af7e66a9584ef1052973805a34bbc0eeb53c8043c5f8ce8eb02b6366"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
            ("rust-native/src/system_actions.rs", include_bytes!("../../system_actions.rs"), "b97944f7f49e0fee4442084d7ef51dc7788e3f583cdf062283f9d141d5ed1833"),
            ("rust-native/src/file_search.rs", include_bytes!("../../file_search.rs"), "adf3f5fc557f9e22a3d2227624f8d795d133300fc417c94a48738f087f94b62a"),
            ("rust-native/src/os_command.rs", include_bytes!("../../os_command.rs"), "1faa996a7c7257702cabcd00bc2ecdb8c6b1e2951e0d24b563693ba062501da2"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "781395bce10a03e634e63fbb811498069e4679a016648b8a16ce7e42b1cf2900"),
            ("rust-native/tests/fixtures/shortcut_child.rs", include_bytes!("../../../tests/fixtures/shortcut_child.rs"), "0776e88ac058de87d07f8e92a7f819d1132a276f6b63013dee08959c944bf8dd"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "e870bc1f9ea23e6e36b2157531b6086baa1b2a85f8c033469581810376de19de"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "a4a171e74c80e06295d175dcb1d4858313a0a414fabf43926c6a9b72ebcaeef0"),
        ],

        "--check-actions-menu" => &[
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "a4a171e74c80e06295d175dcb1d4858313a0a414fabf43926c6a9b72ebcaeef0"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "1946a4cb06bc145fbdcfa2a4e7c9367893fbe8a23c8fbab02a490ea0bc32a886"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "efaba95818f4668834be05942525865bb261de779f08f702a25babbbe4d779dc"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/src/benchmark_host/rpc.rs", include_bytes!("../../benchmark_host/rpc.rs"), "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            // The final eight binding fields and real queued-recorder ACK are
            // carried by item-binding-original-process, never by Polish alone.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "ed5a3adb94c3e9c24feb333265969c18e197a62d3928508a0e5f33c414527d17"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "7ac4dfb6af7e66a9584ef1052973805a34bbc0eeb53c8043c5f8ce8eb02b6366"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
        ],

        "--check-interactions" => &[
            ("rust-native/tests/native_open_basic_process.rs", include_bytes!("../../../tests/native_open_basic_process.rs"), "629c4befb66bc921ed3b645ee44cc25052374a9a131c53e53ddeab20a72c4c32"),
            ("rust-native/tests/fixtures/workspace_fixture.rs", include_bytes!("../../../tests/fixtures/workspace_fixture.rs"), "893eaba2a1a537958085554a78d7cd69131003c0c705d0a6c5952b16a9f5cb06"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            // Original local-data conjunctions and retained native-choice/open
            // components are qualified separately from full matrix execution.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "ed5a3adb94c3e9c24feb333265969c18e197a62d3928508a0e5f33c414527d17"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "7ac4dfb6af7e66a9584ef1052973805a34bbc0eeb53c8043c5f8ce8eb02b6366"),
            ("rust-native/tests/bridge_process.rs", include_bytes!("../../../tests/bridge_process.rs"), "5e281d85d5122f4758e648ce78133657377df963c3fedf827a17b609e778ec3d"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/src/benchmark_host/rpc.rs", include_bytes!("../../benchmark_host/rpc.rs"), "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),

            ("rust-native/src/native_edit.rs", include_bytes!("../../native_edit.rs"), "985f3d19de00ee70242e9310cef1373c2a330b5bc70a04263af90ce24ed58e0d"),
            ("rust-native/src/keyword_matcher.rs", include_bytes!("../../keyword_matcher.rs"), "4f35db2014bc5ec4832a10ad42b7ba9318930d1d09e8937839d3c0a37c1433dd"),
            ("rust-native/src/templates.rs", include_bytes!("../../templates.rs"), "420e51cd15454dd20a366bde6e8c02cdec659726e0d45a2069f77d37660bdf7f"),
            ("rust-native/src/snippet_output.rs", include_bytes!("../../snippet_output.rs"), "40f779d97941aec293f6a53c89970051189e82a0fa2a98a86027c196724138be"),
            ("rust-native/src/native_snippet_choice.rs", include_bytes!("../../native_snippet_choice.rs"), "caa9be3b6d84594d8fabc13f797a99a5042515ba94760cae86506773139a09a9"),
            ("rust-native/src/snippet_choice.rs", include_bytes!("../../snippet_choice.rs"), "d340d341185ef6000347355f8b88f68da6b34458730ecf646c5eb5541e2b95aa"),
            ("rust-native/src/snippet_choice_request.rs", include_bytes!("../../snippet_choice_request.rs"), "9f42eb8c27f93929e66a631de7dda8cc39630d2225459fa758d71195f391d6eb"),
            ("rust-native/src/bin/native_snippet_choice_probe.rs", include_bytes!("../native_snippet_choice_probe.rs"), "66f534623863df99f352a304bbb0a09669be6e609bfabb5db58699356f8ada03"),
            ("rust-native/tests/native_snippet_choice_process.rs", include_bytes!("../../../tests/native_snippet_choice_process.rs"), "a377189c20cb7af7710acfe989d3f514a92b4905f221e01e2e4a22934744af1d"),
            ("rust-native/tests/fixtures/choice_board_sdk.rs", include_bytes!("../../../tests/fixtures/choice_board_sdk.rs"), "b2cf3257801da488997df127933a9f29a09d3b93abec66f2dd14ea6808a34450"),
            ("rust-native/tests/fixtures/choice_monitor_sdk.rs", include_bytes!("../../../tests/fixtures/choice_monitor_sdk.rs"), "7d83558c04fb39fcac4369de5dac244c5387962b896bb4653817d55327325e74"),
            ("rust-native/tests/fixtures/choice_fixture.rs", include_bytes!("../../../tests/fixtures/choice_fixture.rs"), "56c974f5848de285196f5d3b59b30a058c684157415f4d8d04e195aa93a3022a"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "efaba95818f4668834be05942525865bb261de779f08f702a25babbbe4d779dc"),
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "a4a171e74c80e06295d175dcb1d4858313a0a414fabf43926c6a9b72ebcaeef0"),
        ],
        "--check-rich-resources" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/native_monitor.rs", include_bytes!("../../native_monitor.rs"), "54716cd3c8d6d03f0c5da68ec83d723621854d9647d5dbc62e38d0eb2ffd89c2"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/monitor.rs", include_bytes!("../../monitor.rs"), "a0332045b3c8cb7f2ca8c2acf0a4c76c52acd5efff0b0a2b1d50f7822d25cf91"),
            ("rust-native/src/resource_sampler.rs", include_bytes!("../../resource_sampler.rs"), "14e577615f4d9807aed14da0f103e9338193cb50d1ccec6c75b6f667af755a98"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "f0d80d646ee5df36906abff98612a02e2dbdfff0d3bbdb13896a85ef39cfdc37"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "59bc0cc716ba16dea07bbd20d1f8951ed3b4580c7cc7ea9a56116156b9c17556"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "1406185c311781ff5be404caaa0d92ac76746a68f44fb54e3eedff5419241738"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_ownership.rs", include_bytes!("../../clipboard_ownership.rs"), "b38329314cae144ece5ca7063135db60c9deea8e3df6d3728e0b631c1f0e68b4"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "15c93afc3731db2ca870db6d824dcc90bc6f0f367fe9d3ca12497b90f26df97a"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust/src/private_storage.rs", include_bytes!("../../../../rust/src/private_storage.rs"), "cefa77c11c4cd3543dc547f0696854d4e4601550fc701440803b74f4a8287092"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "e870bc1f9ea23e6e36b2157531b6086baa1b2a85f8c033469581810376de19de"),
            ("rust-native/tests/clipboard_history_host_process.rs", include_bytes!("../../../tests/clipboard_history_host_process.rs"), "8cf10e2943cf3f48c6a2a7edebbf1721ad305a34116e9fe357ae66b93eb7a0f8"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/fixtures/snippet_effects_sdk.rs", include_bytes!("../../../tests/fixtures/snippet_effects_sdk.rs"), "4070c904b8d6cd406487561c108d03ea7870da176b64e66b14de718bfcadb4af"),
            ("rust-native/tests/fixtures/clipboard_history_sdk.rs", include_bytes!("../../../tests/fixtures/clipboard_history_sdk.rs"), "712c823e91e7670f738c52aff5781dbeac77314983434e19ec4c0b2e7984aae3"),
            ("rust-native/tests/fixtures/clipboard_history_fixture.rs", include_bytes!("../../../tests/fixtures/clipboard_history_fixture.rs"), "94473b2d9d0000dfd436c4acdcec048e6ae771d4f4a745d8686359709549ce92"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/fixtures/workspace_fixture.rs", include_bytes!("../../../tests/fixtures/workspace_fixture.rs"), "893eaba2a1a537958085554a78d7cd69131003c0c705d0a6c5952b16a9f5cb06"),
        ],
        "--check-menu-actions" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "ffc22cd5744d2f9fa0f6928a8465dcf77602c89d1279e6e1595121b0231e2ca3"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "781395bce10a03e634e63fbb811498069e4679a016648b8a16ce7e42b1cf2900"),
            ("rust-native/tests/fixtures/shortcut_child.rs", include_bytes!("../../../tests/fixtures/shortcut_child.rs"), "0776e88ac058de87d07f8e92a7f819d1132a276f6b63013dee08959c944bf8dd"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "f0d80d646ee5df36906abff98612a02e2dbdfff0d3bbdb13896a85ef39cfdc37"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "d31bb31a216a3a522fc106bbf3f90fd170bcea17ca0c6be462a71fb8c2587414"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "603e1d9dfaee17a7081b391a24ea34fe2139e2317f401219bfacdf6e0adecfd2"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "15c93afc3731db2ca870db6d824dcc90bc6f0f367fe9d3ca12497b90f26df97a"),
            ("rust-native/src/analytics.rs", include_bytes!("../../analytics.rs"), "da2b82369139cbdf6bb843136f24094daa0bbf083daacc51d99ec502b7e67736"),
            ("config/default.json", include_bytes!("../../../../config/default.json"), "a13a90a2ccfd5c92cc39dba19e3792a55c04700c26118721636ad036eb7ebd7c"),
        ],
        "--check-window-search" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "ffc22cd5744d2f9fa0f6928a8465dcf77602c89d1279e6e1595121b0231e2ca3"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "781395bce10a03e634e63fbb811498069e4679a016648b8a16ce7e42b1cf2900"),
            ("rust-native/tests/fixtures/shortcut_child.rs", include_bytes!("../../../tests/fixtures/shortcut_child.rs"), "0776e88ac058de87d07f8e92a7f819d1132a276f6b63013dee08959c944bf8dd"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "f0d80d646ee5df36906abff98612a02e2dbdfff0d3bbdb13896a85ef39cfdc37"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "d31bb31a216a3a522fc106bbf3f90fd170bcea17ca0c6be462a71fb8c2587414"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
        ],
        "--check-apple-shortcuts" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "ffc22cd5744d2f9fa0f6928a8465dcf77602c89d1279e6e1595121b0231e2ca3"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "781395bce10a03e634e63fbb811498069e4679a016648b8a16ce7e42b1cf2900"),
            ("rust-native/tests/fixtures/shortcut_child.rs", include_bytes!("../../../tests/fixtures/shortcut_child.rs"), "0776e88ac058de87d07f8e92a7f819d1132a276f6b63013dee08959c944bf8dd"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "f0d80d646ee5df36906abff98612a02e2dbdfff0d3bbdb13896a85ef39cfdc37"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "d31bb31a216a3a522fc106bbf3f90fd170bcea17ca0c6be462a71fb8c2587414"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/src/system_search/apple_shortcuts.rs", include_bytes!("../../system_search/apple_shortcuts.rs"), "f2642015ad57c2d7b08af1f23c1e5125fbe841f9658714f7cc0e5adc56d21581"),
            ("rust-native/src/runtime_private.rs", include_bytes!("../../runtime_private.rs"), "88c10d0c32c7fa0e163ee6c3a320bde9ee81828852c0d204effaf1d6a151219b"),
            ("rust-native/src/script_result.rs", include_bytes!("../../script_result.rs"), "6e311e6045f6f3e5d3bf8998e7e34d23323ad324d449dcac3be1ecb1e9547cbf"),
            ("rust-native/src/script_session.rs", include_bytes!("../../script_session.rs"), "0323530ebe9eacfea569ce5da1b123a2c47d43c140a24340e1d97b3c4cc3774a"),
            ("rust-native/src/execution.rs", include_bytes!("../../execution.rs"), "a211e57ffabbb0cb0cbb8ce19ad894939ca310c36a34a9fea055a0df3e023c8f"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "15c93afc3731db2ca870db6d824dcc90bc6f0f367fe9d3ca12497b90f26df97a"),
            ("rust/src/script.rs", include_bytes!("../../../../rust/src/script.rs"), "16974362c399c6c5a92596d7563d501eb0492df9f5e2463d8147eeb70108bfb0"),
            ("rust-native/tests/script_result_basic_process.rs", include_bytes!("../../../tests/script_result_basic_process.rs"), "70e1a2de21b80f8702438acb49b03fac13576c84cc90e3857af8fc3aa48eb498"),
            ("rust-native/tests/script_result_native_process.rs", include_bytes!("../../../tests/script_result_native_process.rs"), "6791270af079e8a05e679dc86de1528dd4dfb7f881b7a291dc8b9878a8786410"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "1946a4cb06bc145fbdcfa2a4e7c9367893fbe8a23c8fbab02a490ea0bc32a886"),
            ("rust-native/tests/private_results.rs", include_bytes!("../../../tests/private_results.rs"), "be0a6101e83ea2557f4ed70eb17f396943e3a9178f060391a89722aa81aba7e8"),
            ("config/default.json", include_bytes!("../../../../config/default.json"), "a13a90a2ccfd5c92cc39dba19e3792a55c04700c26118721636ad036eb7ebd7c"),
        ],
        "--check-render-updates" => &[
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "d2a7fa23998199283807ab0e0506c04420b34c2a6166557e16adaa77ee4afc25"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/observe.rs", include_bytes!("../../observe.rs"), "b12d21a4673556e09d603c69cae28eb720c92287b195f7244110fce5769cef8f"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6c32d019b3df9d25a8ce5543c17ab33d6c1acd1fc67b1f53eb2898b805e66426"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
        ],
        "--check-browser-tabs" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "ffc22cd5744d2f9fa0f6928a8465dcf77602c89d1279e6e1595121b0231e2ca3"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/src/benchmark_host/rpc.rs", include_bytes!("../../benchmark_host/rpc.rs"), "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "edf6196b295eadc26e546b99fb741a378d0f775c3cbcdf1b7636546ed2581d31"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "4a450aa2eb6a0eec8e681a67f241c0147726229de742adbd741aa9bdce980367"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "781395bce10a03e634e63fbb811498069e4679a016648b8a16ce7e42b1cf2900"),
            ("rust-native/tests/fixtures/shortcut_child.rs", include_bytes!("../../../tests/fixtures/shortcut_child.rs"), "0776e88ac058de87d07f8e92a7f819d1132a276f6b63013dee08959c944bf8dd"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "f0d80d646ee5df36906abff98612a02e2dbdfff0d3bbdb13896a85ef39cfdc37"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "d31bb31a216a3a522fc106bbf3f90fd170bcea17ca0c6be462a71fb8c2587414"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
        ],
        "--check-performance-pipeline" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_private_tests.rs", include_bytes!("../../runtime_private_tests.rs"), "00feb5e6befffefed0d19de776a13708727763b3b55bc42a45bf14ceb1bc29c3"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "611637d4df9bf371e15274e7a71beca99e214a1582c092e1a4a2a40341cb2569"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "59bc0cc716ba16dea07bbd20d1f8951ed3b4580c7cc7ea9a56116156b9c17556"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "d2a7fa23998199283807ab0e0506c04420b34c2a6166557e16adaa77ee4afc25"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6c32d019b3df9d25a8ce5543c17ab33d6c1acd1fc67b1f53eb2898b805e66426"),
            ("rust-native/src/observe.rs", include_bytes!("../../observe.rs"), "b12d21a4673556e09d603c69cae28eb720c92287b195f7244110fce5769cef8f"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "1406185c311781ff5be404caaa0d92ac76746a68f44fb54e3eedff5419241738"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_ownership.rs", include_bytes!("../../clipboard_ownership.rs"), "b38329314cae144ece5ca7063135db60c9deea8e3df6d3728e0b631c1f0e68b4"),
            ("rust/src/private_storage.rs", include_bytes!("../../../../rust/src/private_storage.rs"), "cefa77c11c4cd3543dc547f0696854d4e4601550fc701440803b74f4a8287092"),
            ("rust/src/lib.rs", include_bytes!("../../../../rust/src/lib.rs"), "291731f83d4278ad4b1f892558a6b155de3288e8478cf64caed9489b9fa61d49"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/clipboard_history_host_process.rs", include_bytes!("../../../tests/clipboard_history_host_process.rs"), "8cf10e2943cf3f48c6a2a7edebbf1721ad305a34116e9fe357ae66b93eb7a0f8"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/fixtures/snippet_effects_sdk.rs", include_bytes!("../../../tests/fixtures/snippet_effects_sdk.rs"), "4070c904b8d6cd406487561c108d03ea7870da176b64e66b14de718bfcadb4af"),
            ("rust-native/tests/fixtures/clipboard_history_sdk.rs", include_bytes!("../../../tests/fixtures/clipboard_history_sdk.rs"), "712c823e91e7670f738c52aff5781dbeac77314983434e19ec4c0b2e7984aae3"),
            ("rust-native/tests/fixtures/clipboard_history_fixture.rs", include_bytes!("../../../tests/fixtures/clipboard_history_fixture.rs"), "94473b2d9d0000dfd436c4acdcec048e6ae771d4f4a745d8686359709549ce92"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "9de2408564b5c751b1dbe084a49ce25e4267176e88205c95fbd49d6748a03c1c"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
        ],
        "--check-settings" => &[
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "a4a171e74c80e06295d175dcb1d4858313a0a414fabf43926c6a9b72ebcaeef0"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "1946a4cb06bc145fbdcfa2a4e7c9367893fbe8a23c8fbab02a490ea0bc32a886"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "efaba95818f4668834be05942525865bb261de779f08f702a25babbbe4d779dc"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "b972915f1a9cfabe18298d65e45e3442bbccd48bff002333e1c38f20562bc2fb"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "fb6e3a1b9d32485241d2f219155d4d21b5923fe1d64c61edba085d2f18b00cfb"),
            ("rust-native/src/bin/snippet_probe.rs", include_bytes!("../../bin/snippet_probe.rs"), "e8322e4076b5ddfe71b0e79c3039f34c7fe1dcfcf3f66f42f512e0207387361a"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "59bc0cc716ba16dea07bbd20d1f8951ed3b4580c7cc7ea9a56116156b9c17556"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "d2a7fa23998199283807ab0e0506c04420b34c2a6166557e16adaa77ee4afc25"),
            ("rust-native/src/hotkeys.rs", include_bytes!("../../hotkeys.rs"), "2b9448c0540a378d5f338c554c74ad4614fb550ebccc084d29a3be9bf72b3258"),
            ("rust-native/src/keyboard_state.rs", include_bytes!("../../keyboard_state.rs"), "e030ec9022301b2ccd89e924bd1885fe72a09c9aed2e17f63e63e93503e71cac"),
            ("rust-native/src/library.rs", include_bytes!("../../library.rs"), "4788c64fe77873c6f71ceff312f248421c8255c0e0a64387713b280964815525"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "42e192f5f311792de41ec9a6e1d0cdc5db3c09efe96a902fba509826d5eb3b14"),
            ("rust-native/src/native_library.rs", include_bytes!("../../native_library.rs"), "2da5d1b8b410bd90ae4f63855a0ff06be6b0677038b33acd0a2a69cde1542ecb"),
            ("rust-native/src/native_paste.rs", include_bytes!("../../native_paste.rs"), "aa76bdf2224e9ed4a249b91d9a41712ae067b4f6a47a265bb9bcb6365da3d626"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "13e89ec76488a84fb9c9970aff9285e62639742fca6e3b0bc4723b382b8504f7"),
            ("rust-native/src/native_settings.rs", include_bytes!("../../native_settings.rs"), "5d662f5ad70e3be4407acf74babbf97345ac0b05079fcdd88069181d63a23041"),
            ("rust-native/src/native_snippet_editor.rs", include_bytes!("../../native_snippet_editor.rs"), "c047b34e4abf31b111d804f9bc1e5a9efe75a002e5e0ac612c0cb48fe71d6150"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "1760da639b8753f278f209bcab2ece32edc01121cb4a5f15ae4698088e69fdc8"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6c32d019b3df9d25a8ce5543c17ab33d6c1acd1fc67b1f53eb2898b805e66426"),
            ("rust-native/src/recorder.rs", include_bytes!("../../recorder.rs"), "582360a4be08b798ea4d798265af417143d6e0f88a388210a76cf8bce9a5334c"),
            ("rust-native/src/runtime_health.rs", include_bytes!("../../runtime_health.rs"), "ee787b5ccd193c1f1a400ce0da4798132d8dd68d48f85f40b63a776cfccf7d86"),
            ("rust-native/src/settings_surface.rs", include_bytes!("../../settings_surface.rs"), "0578d288549d01d76d3168cc8607ab56bd46df03d8583a49ab266ce95b5e5b37"),
            ("rust-native/src/snippet_output.rs", include_bytes!("../../snippet_output.rs"), "40f779d97941aec293f6a53c89970051189e82a0fa2a98a86027c196724138be"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/keyword_process.rs", include_bytes!("../../../tests/keyword_process.rs"), "7d21b25eb979f3bacd663111e1f206cc5ace2dcc312f1ff7e60066c449bbe3e9"),
            ("rust-native/tests/keyword_reference.json", include_bytes!("../../../tests/keyword_reference.json"), "b3ad168109a4712a792756f6227a730cec365fae7399824803034131ce956119"),
            ("rust-native/tests/keyword_rich_process.rs", include_bytes!("../../../tests/keyword_rich_process.rs"), "f9d5bfbdeeaebaba0e9568d9772d08bf77f5606911dd1e22105700453ca5c9ad"),
            ("rust-native/tests/support/keyword_capture.rs", include_bytes!("../../../tests/support/keyword_capture.rs"), "44869110fc147b039d54e068be36cbaf668ce15fd493be3ee263b633f95b21d1"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_keyboard_process.rs", include_bytes!("../../../tests/settings_keyboard_process.rs"), "0c4326feb11b031e79f30926410dc44719347b0988628b813c1286052df9e762"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/snippet_board_process.rs", include_bytes!("../../../tests/snippet_board_process.rs"), "8f5841d6591e369d7b120534b1bb2f78f87cfafc098efe4066992d7f0fef6d3a"),
            ("rust-native/tests/snippet_process.rs", include_bytes!("../../../tests/snippet_process.rs"), "cfe1f20e499e4da81bb7c8508f1a5c951a0114da28a6b7feabd4ba58dee673c0"),
            ("rust-native/tests/snippet_reference.json", include_bytes!("../../../tests/snippet_reference.json"), "b19d5abc788642df64402065797d255db3fa399b0b763eb2cf66544746f1f746"),
            ("rust-native/tests/startup_process.rs", include_bytes!("../../../tests/startup_process.rs"), "cb1441b9c63056ed828df688e8af5a4f6cbfd3208352d37f493c51a8ef0a06cd"),
            ("rust/src/lib.rs", include_bytes!("../../../../rust/src/lib.rs"), "291731f83d4278ad4b1f892558a6b155de3288e8478cf64caed9489b9fa61d49"),
        ],
        "--check-command-log" => &[
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a6ef829a2c87091d9133e377ef12e8ba7d3fe008fae7275e3cd8f019c2c7a4ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
            ("rust-native/src/runtime_utilities.rs", include_bytes!("../../runtime_utilities.rs"), "209d90ba65f4fb8fd9a1341d419fddf5ada441dcebb641914cd2ac1e6692e8bd"),
            ("rust-native/src/runtime_builtins.rs", include_bytes!("../../runtime_builtins.rs"), "43bc0ddfcfde25438465f7dc2f9c245bd7084d8906f44baf5f229be43e6f991c"),
            ("rust-native/tests/private_results.rs", include_bytes!("../../../tests/private_results.rs"), "be0a6101e83ea2557f4ed70eb17f396943e3a9178f060391a89722aa81aba7e8"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "1946a4cb06bc145fbdcfa2a4e7c9367893fbe8a23c8fbab02a490ea0bc32a886"),
            (
                "rust-native/src/activity.rs",
                include_bytes!("../../activity.rs"),
                LOG_SOURCE_SHA256,
            ),
        ],
        "--check-edit-commands" => &[
            (
                "rust-native/src/native_edit.rs",
                include_bytes!("../../native_edit.rs"),
                "985f3d19de00ee70242e9310cef1373c2a330b5bc70a04263af90ce24ed58e0d",
            ),
            (
                "rust-native/src/native_edit_menu.rs",
                include_bytes!("../../native_edit_menu.rs"),
                "8213290295ec55cd13adf37a5fd30f6648854fadc471272be9ed49e3385c438b",
            ),
        ],
        "--check-analytics" => &[
            ("rust-native/tests/native_dashboard_dialog_process.rs", include_bytes!("../../../tests/native_dashboard_dialog_process.rs"), "7f6c542b17d14c05f5187475c0d7d4c76e135e0699397ae25233f8fd1ae1e32a"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "3daa927ea7f7312fced525cfcbdba488a0a5a178cedeb0e12d403e00d44f0dc4"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/src/benchmark_host/rpc.rs", include_bytes!("../../benchmark_host/rpc.rs"), "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            (
                "rust-native/src/analytics.rs",
                include_bytes!("../../analytics.rs"),
                "da2b82369139cbdf6bb843136f24094daa0bbf083daacc51d99ec502b7e67736",
            ),
            (
                "rust-native/src/analytics_dashboard.rs",
                include_bytes!("../../analytics_dashboard.rs"),
                "df90b83a49b965a6eeea0ee7c7810eace54a825d226a35f892d4d518d57e6e28",
            ),
            (
                "rust-native/src/native_dashboard.rs",
                include_bytes!("../../native_dashboard.rs"),
                "44eadd8b53ea51c8f8617de4db3c5eac18464d0b966b24ec465162f86b3c2485",
            ),
            (
                "rust-native/src/management_commands.rs",
                include_bytes!("../../management_commands.rs"),
                "14f68e0678ba748a65fc88a200f3cea59b9c9de8950462c2369ce5c526c75a31",
            ),
            (
                "rust-native/tests/startup_process.rs",
                include_bytes!("../../../tests/startup_process.rs"),
                "cb1441b9c63056ed828df688e8af5a4f6cbfd3208352d37f493c51a8ef0a06cd",
            ),
            (
                "rust-native/tests/dashboard_export_process.rs",
                include_bytes!("../../../tests/dashboard_export_process.rs"),
                "dd70465e75821d667282bc207d7a64bff98fff4bc1d91675b2d5c1ca848b0ed5",
            ),
            (
                "rust-native/tests/fixtures/dashboard_export_shim.rs",
                include_bytes!("../../../tests/fixtures/dashboard_export_shim.rs"),
                "843e90a21bd0018028b3343c17f08b5ae491fd650c48da37095120f8d5238a47",
            ),
            (
                "rust-native/tests/fixtures/fcntl_abi.rs",
                include_bytes!("../../../tests/fixtures/fcntl_abi.rs"),
                "b2b7194db0bf1adef57aab17e3424a90c67818eadac43ee6b651d3a7b4eb0029",
            ),
            (
                "rust-native/src/test_fixtures.rs",
                include_bytes!("../../test_fixtures.rs"),
                "3ee56f1afc0008bda747798f54aa1d3a039603d16451ed24b67f6ea8fb0b06b5",
            ),
            ("rust-native/tests/analytics_runtime_process.rs", include_bytes!("../../../tests/analytics_runtime_process.rs"), "8ef18a852c782df0a469ea5ca4f22e2e4d2c9f0392f1edad6103389adedd871b"),
            ("rust-native/tests/fixtures/analytics_write_shim.rs", include_bytes!("../../../tests/fixtures/analytics_write_shim.rs"), "9e2add20464e7336d854171083e3861f79a4dbe6d49f67ecf723cf72f98b790c"),
            ("rust-native/tests/analytics_cast_reference.rs", include_bytes!("../../../tests/analytics_cast_reference.rs"), "9af609d17784a42a1847feec74d5e70ceccf84efc83be03c5a9578e3aac482c3"),
            ("rust-native/tests/fixtures/analytics_cast_reference.json", include_bytes!("../../../tests/fixtures/analytics_cast_reference.json"), "6bd89546d290d6e3f634c185af4cf322b4d141a0426f77eb034ff115bc8df9cb"),
            ("rust-native/tests/fixtures/analytics_lifecycle_reference.json", include_bytes!("../../../tests/fixtures/analytics_lifecycle_reference.json"), "cd0b799d618f950147b3f318c698bd29aaf1e7790d785c597b3643639eeeaac9"),
            ("rust-native/tests/fixtures/analytics_retention_reference.json", include_bytes!("../../../tests/fixtures/analytics_retention_reference.json"), "ddaea60e8771a91a4a51332b14e2e1a0d58f3b361155759eaa6919da1397f7e6"),
            ("rust-native/tests/fixtures/analytics_decoder_reference.json", include_bytes!("../../../tests/fixtures/analytics_decoder_reference.json"), "22a3ec58d6f51df2173d686b2f98770a3b0d39ce015af85eaa674327a15b32cc"),
            ("rust-native/tests/fixtures/analytics_integer_reference.json", include_bytes!("../../../tests/fixtures/analytics_integer_reference.json"), "3c0fed3cabad017dc90b7d2396b27b455c57489eba80d58ef19682d524befc4c"),
            ("rust-native/tests/fixtures/analytics_limits_reference.json", include_bytes!("../../../tests/fixtures/analytics_limits_reference.json"), "e58f134fddec3a7ecb65a6c9fa341dca1feee40dde9abd7c3b03e11bc2563bfb"),
            ("rust-native/tests/dashboard_presentation_reference.rs", include_bytes!("../../../tests/dashboard_presentation_reference.rs"), "bf3748beebc236ffecd75289e95011704dbef19dbe8f52083b263e0ff34efc85"),
            ("rust-native/tests/fixtures/dashboard_presentation_reference.json", include_bytes!("../../../tests/fixtures/dashboard_presentation_reference.json"), "3e1bf1619960487cf831ad00001a7b575b1b434c89bb395522734e44a2cba283"),
        ],
        "--check-visual-assets" => &[
            ("rust-native/tests/native_process.rs", include_bytes!("../../../tests/native_process.rs"), "f86e9575dbaee80d9c8e00471abfeece4c5b416c0852cfa6e426cd84352254c7"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "7ee547661e2140346ef50fdbcad5d4979382fe7085a73c546a722f7d93fde323"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "f3a3ef095dec610720d9685ed6f6ec439b8aa9dab9f1db1ffd8aad601f1ffc4b"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "417c818c0cf77cd5fb87dfa96ecf139860771791327090bcb157ca38e14d4e28"),
            ("config/default.json", include_bytes!("../../../../config/default.json"), "a13a90a2ccfd5c92cc39dba19e3792a55c04700c26118721636ad036eb7ebd7c"),
            (
                "rust-native/src/presentation.rs",
                include_bytes!("../../presentation.rs"),
                "c6e7983dfa614386470786624f7e2664f1b61ae9024d3a41268120e436a3242b",
            ),
            (
                "rust-native/src/website_icons.rs",
                include_bytes!("../../website_icons.rs"),
                "af4f29f6940a8bcb698c24af783a234caa80326f48dbe4705ea39d2de2be8fb9",
            ),
            (
                "rust-native/src/clipboard_image.rs",
                include_bytes!("../../clipboard_image.rs"),
                "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db",
            ),
            (
                "rust-native/src/native_rows.rs",
                include_bytes!("../../native_rows.rs"),
                "f6c5ae61b701dcd876c09220c6900a86be261d957d6708ce968e70ce3d73d309",
            ),
            (
                "rust-native/src/native.rs",
                include_bytes!("../../native.rs"),
                "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc",
            ),
        ],
        _ => &[],
    }
}
fn settings_provenance_source(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "--check-command-log" => Some((
            "Sources/RuycastApp/CommandLogChecks.swift",
            LOG_ORACLE_SHA256,
        )),
        "--check-edit-commands" => Some((
            "Sources/RuycastApp/EditCommandChecks.swift",
            EDIT_COMMAND_ORACLE_SHA256,
        )),
        "--check-visual-assets" => Some((
            "Sources/RuycastApp/VisualAssetChecks.swift",
            VISUAL_ASSET_ORACLE_SHA256,
        )),
        "--check-render-updates" => Some((
            "Sources/RuycastApp/InteractionChecks.swift",
            RENDER_UPDATES_ORACLE_SHA256,
        )),
        "--check-interactions" => Some((
            "Sources/RuycastApp/InteractionChecks.swift",
            RENDER_UPDATES_ORACLE_SHA256,
        )),
        "--check-analytics" => Some((
            "Sources/RuycastApp/AnalyticsChecks.swift",
            ANALYTICS_ORACLE_SHA256,
        )),
        "--check-performance-pipeline" => Some((
            "Sources/RuycastApp/PerformancePipelineChecks.swift",
            PERFORMANCE_PIPELINE_ORACLE_SHA256,
        )),
        "--check-browser-tabs" => Some((
            "Sources/RuycastApp/BrowserTabChecks.swift",
            BROWSER_TABS_ORACLE_SHA256,
        )),
        "--check-window-search" => Some((
            "Sources/RuycastApp/WindowSearchChecks.swift",
            WINDOW_SEARCH_ORACLE_SHA256,
        )),
        "--check-menu-actions" => Some((
            "Sources/RuycastApp/MenuActionChecks.swift",
            MENU_ACTIONS_ORACLE_SHA256,
        )),
        "--check-actions-menu" => Some((
            "Sources/RuycastApp/ActionMenuChecks.swift",
            ACTIONS_MENU_ORACLE_SHA256,
        )),
        "--check-rich-resources" => Some((
            "Sources/RuycastApp/RichResourceChecks.swift",
            RICH_RESOURCES_ORACLE_SHA256,
        )),
        "--check-apple-shortcuts" => Some((
            "Sources/RuycastApp/AppleShortcutChecks.swift",
            APPLE_SHORTCUTS_ORACLE_SHA256,
        )),
        "--check-polish" => Some(("Sources/RuycastApp/PolishChecks.swift", POLISH_ORACLE_SHA256)),
        "--check-settings" => Some(("Sources/RuycastApp/SettingsChecks.swift", SETTINGS_ORACLE_SHA256)),
        "--check-feedback" => Some(("Sources/RuycastApp/FeedbackChecks.swift", FEEDBACK_ORACLE_SHA256)),
        _ => None,
    }
}
fn settings_source_bindings(id: &str) -> Vec<(&'static str, &'static str)> {
    let mut bindings = settings_sources(id)
        .iter()
        .map(|(path, _, sha)| (*path, *sha))
        .collect::<Vec<_>>();
    if let Some(provenance) = settings_provenance_source(id) {
        bindings.push(provenance);
    }
    bindings.extend_from_slice(settings_additional_provenance_sources(id));
    bindings
}
fn settings_additional_provenance_sources(id: &str) -> &'static [(&'static str, &'static str)] {
    match id {
        "--check-feedback" => &[
            ("Sources/RuycastApp/SystemChecks.swift", SYSTEM_CHECKS_ORACLE_SHA256),
            ("Sources/RuycastApp/ActionMenuChecks.swift", ACTIONS_MENU_ORACLE_SHA256),
            ("Sources/RuycastApp/InteractionChecks.swift", RENDER_UPDATES_ORACLE_SHA256),
        ],
        "--check-settings" => &[("Sources/RuycastApp/SnippetEditingChecks.swift", SNIPPET_EDITING_ORACLE_SHA256)],
        "--check-actions-menu" => &[
            ("Sources/RuycastApp/FeedbackChecks.swift", FEEDBACK_ORACLE_SHA256),
            ("Sources/RuycastApp/InteractionChecks.swift", RENDER_UPDATES_ORACLE_SHA256),
        ],
        "--check-interactions" => &[
            ("Sources/RuycastApp/FeedbackChecks.swift", FEEDBACK_ORACLE_SHA256),
            ("Sources/RuycastApp/ActionMenuChecks.swift", ACTIONS_MENU_ORACLE_SHA256),
        ],
        _ => &[],
    }
}
fn settings_mapping_state(gate: &SettingsGate) -> &'static str {
    if !gate.unproven_invariants.is_empty() || gate.native_cases.is_empty() {
        return "mapped-with-unproven-invariants";
    }
    // These exact inputs have completed original-case crosswalks and scoped
    // independent review. This qualifies mapping, never a validation pass.
    let sources = settings_sources(gate.id);
    let provenance_current = settings_provenance_source(gate.id).is_none_or(|(path, sha)| {
        let expected = match gate.id {
            "--check-command-log" => Some(LOG_ORACLE_SHA256),
            "--check-edit-commands" => Some(EDIT_COMMAND_ORACLE_SHA256),
            "--check-visual-assets" => Some(VISUAL_ASSET_ORACLE_SHA256),
            "--check-render-updates" => Some(RENDER_UPDATES_ORACLE_SHA256),
            "--check-interactions" => Some(RENDER_UPDATES_ORACLE_SHA256),
            "--check-analytics" => Some(ANALYTICS_ORACLE_SHA256),
            "--check-performance-pipeline" => Some(PERFORMANCE_PIPELINE_ORACLE_SHA256),
            "--check-browser-tabs" => Some(BROWSER_TABS_ORACLE_SHA256),
            "--check-window-search" => Some(WINDOW_SEARCH_ORACLE_SHA256),
            "--check-menu-actions" => Some(MENU_ACTIONS_ORACLE_SHA256),
            "--check-actions-menu" => Some(ACTIONS_MENU_ORACLE_SHA256),
            "--check-rich-resources" => Some(RICH_RESOURCES_ORACLE_SHA256),
            "--check-apple-shortcuts" => Some(APPLE_SHORTCUTS_ORACLE_SHA256),
            "--check-polish" => Some(POLISH_ORACLE_SHA256),
            "--check-settings" => Some(SETTINGS_ORACLE_SHA256),
            "--check-feedback" => Some(FEEDBACK_ORACLE_SHA256),
            _ => None,
        };
        path == gate.original_source
            && expected == Some(sha)
            && settings_source_bindings(gate.id).contains(&(path, sha))
    });
    let additional = settings_additional_provenance_sources(gate.id);
    let expected_additional_count = match gate.id {
        "--check-settings" => 1,
        "--check-interactions" => 2,
        "--check-feedback" => 3,
        "--check-actions-menu" => 2,
        _ => 0,
    };
    let additional_provenance_current = additional.len() == expected_additional_count
        && additional.iter().all(|(path, sha)| {
            let expected = match (gate.id, *path) {
                ("--check-feedback", "Sources/RuycastApp/SystemChecks.swift") => Some(SYSTEM_CHECKS_ORACLE_SHA256),
                ("--check-feedback", "Sources/RuycastApp/ActionMenuChecks.swift") => Some(ACTIONS_MENU_ORACLE_SHA256),
                ("--check-feedback", "Sources/RuycastApp/InteractionChecks.swift") => Some(RENDER_UPDATES_ORACLE_SHA256),
                ("--check-settings", "Sources/RuycastApp/SnippetEditingChecks.swift") => Some(SNIPPET_EDITING_ORACLE_SHA256),
                ("--check-actions-menu", "Sources/RuycastApp/FeedbackChecks.swift") => Some(FEEDBACK_ORACLE_SHA256),
                ("--check-actions-menu", "Sources/RuycastApp/InteractionChecks.swift") => Some(RENDER_UPDATES_ORACLE_SHA256),
                ("--check-interactions", "Sources/RuycastApp/FeedbackChecks.swift") => Some(FEEDBACK_ORACLE_SHA256),
                ("--check-interactions", "Sources/RuycastApp/ActionMenuChecks.swift") => Some(ACTIONS_MENU_ORACLE_SHA256),
                _ => None,
            };
            expected == Some(*sha) && settings_source_bindings(gate.id).contains(&(*path, *sha))
        });
    let current = !sources.is_empty()
        && sources
            .iter()
            .all(|(_, bytes, expected)| format!("{:x}", Sha256::digest(bytes)) == *expected)
        && provenance_current
        && additional_provenance_current;
    if current {
        "source-qualified-rust-case-crosswalk"
    } else {
        "mapped-requires-source-requalification"
    }
}
fn root_git_mapping_state(source: &[u8]) -> &'static str {
    if format!("{:x}", Sha256::digest(source)) == "4fb35b273771a011b7b015cc39d29be2e2641d0862d81a6e66b90b23de8838b1" {
        "source-qualified-root-bound-whitespace-check"
    } else {
        "requires-source-requalification"
    }
}
fn discover_python_tests(root: &Path) -> Result<(Vec<String>, Vec<String>)> {
    fn visit(
        root: &Path,
        directory: &Path,
        modules: &mut Vec<String>,
        packages: &mut Vec<String>,
    ) -> Result<()> {
        let mut children = Vec::new();
        for entry in fs::read_dir(directory).map_err(error)? {
            let entry = entry.map_err(error)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(error)?;
            if metadata.file_type().is_symlink() {
                return Err("Python test discovery refuses symlinked entries.".into());
            }
            if metadata.is_dir() {
                children.push(path);
            } else if metadata.is_file() {
                let name = path.file_name().and_then(|name| name.to_str())
                    .ok_or("Python test path is not UTF-8.")?;
                if name.starts_with("test_") && path.extension().is_some_and(|extension| extension == "py") {
                    let relative = path.strip_prefix(root).map_err(error)?;
                    modules.push(relative.to_str().ok_or("Python test path is not UTF-8.")?.to_owned());
                }
            } else {
                return Err("Python test discovery refuses non-regular entries.".into());
            }
        }
        for child in children {
            match fs::symlink_metadata(child.join("__init__.py")) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    packages.push(
                        child.join("__init__.py").strip_prefix(root).map_err(error)?
                            .to_str().ok_or("Python package path is not UTF-8.")?.to_owned(),
                    );
                    visit(root, &child, modules, packages)?;
                }
                Ok(_) => return Err("Python test discovery refuses an unsafe package marker.".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    let tests = root.join("Tests");
    let metadata = fs::symlink_metadata(&tests).map_err(error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Python test discovery requires a real Tests directory.".into());
    }
    let mut modules = Vec::new();
    let mut packages = Vec::new();
    visit(root, &tests, &mut modules, &mut packages)?;
    modules.sort();
    packages.sort();
    Ok((modules, packages))
}
fn python_discovery_state(modules: &[String], packages: &[String]) -> &'static str {
    if modules.is_empty() && packages.is_empty() {
        "source-qualified-empty-test-discovery"
    } else {
        "requires-complete-transport-and-case-mapping"
    }
}
pub(super) fn require_compiled_source_root(root: &Path) -> Result<()> {
    let selected = root.canonicalize().map_err(error)?;
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Rust tool has no repository source root.")?
        .canonicalize()
        .map_err(error)?;
    if selected != compiled {
        return Err("Gate qualification requires --repo to be the Rust tool's source checkout.".into());
    }
    Ok(())
}
const SEARCH_CASES: &[(&str, &str, &str)] = &[
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.unchangedRankingReuse", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingChangeReprepare", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingResetReprepare", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.rankedDisabledItem", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.preparedCatalogReplacement", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.disabledTypoItem", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::disabled_catalog_item_is_filtered_from_typo_fallback"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.enabledTypoCliParity", "rust-native/src/runtime_system.rs", "runtime::system::shortcut_budget_tests::typo_fallback_matches_palette_and_local_cli_without_disabled_items"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoPaletteOrder", "rust/src/lib.rs", "tests::direct_title_typo_outranks_weak_subsequence_with_higher_recency"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoCliOrder", "rust-native/src/runtime_system.rs", "runtime::system::shortcut_budget_tests::cli_direct_title_typo_outranks_higher_recency_subsequence"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.appearancePreservesCatalog", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_palette_theme_does_not_reindex_reprepare_or_reevaluate"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.usageRankingRefreshes", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.cachedReads", "rust-native/src/session.rs", "session::tests::repeated_result_reads_preserve_prepared_rows_and_session_state"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.indexDispatchABIParity", "rust/src/lib.rs", "tests::original_256_command_ten_query_index_dispatch_and_abi_parity"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.cliSearchLeavesPaletteUntouched", "rust-native/src/runtime_system.rs", "runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.discoveredAppsRefresh", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::discovered_application_refreshes_an_existing_palette_query"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.reloadDeduplicatesDiscoveredApp", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::configuration_reload_deduplicates_discovered_app_in_active_palette"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.selectiveResultObservation", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::palette_status_changes_preserve_rows_and_selection_updates_publish"),
    ("Sources/RuycastApp/SearchChecks.swift::checkSearch.settingsSurfaceIsolation", "rust-native/src/runtime_private_tests.rs", "runtime::inline_tests::settings_presentation_ignores_palette_navigation_and_clears_configuration_error"),
];
fn search_source_bindings() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("rust-native/src/runtime_private_tests.rs", include_str!("../../runtime_private_tests.rs"), "00feb5e6befffefed0d19de776a13708727763b3b55bc42a45bf14ceb1bc29c3"),
        ("rust-native/src/runtime_system.rs", include_str!("../../runtime_system.rs"), "8daf62239444738e7048795883bd8de625ae4bcbfcb7e73716c7063f68beefc7"),
        ("rust/src/lib.rs", include_str!("../../../../rust/src/lib.rs"), "291731f83d4278ad4b1f892558a6b155de3288e8478cf64caed9489b9fa61d49"),
        ("rust-native/src/session.rs", include_str!("../../session.rs"), "c23e3d163e5cc8b87e6cdd11c80ed5380980885281499e5472065a169aac6d61"),
    ]
}
fn search_crosswalk(mappings: &serde_json::Value) -> Result<serde_json::Value> {
    let sources = search_source_bindings();
    let mut native_cases = BTreeSet::new();
    let mut bindings = BTreeMap::new();
    let mut scenario_ids = Vec::new();
    for (id, path, case) in SEARCH_CASES {
        let mapping = &mappings[*id];
        let (_, source, pin) = sources.iter().find(|(name, _, _)| name == path)
            .ok_or("Search crosswalk source is not qualified.")?;
        let function = case.rsplit("::").next().ok_or("Missing Rust search case.")?;
        if mapping["status"] != "source-qualified-rust-case-crosswalk"
            || mapping["nativeSource"] != *path
            || mapping["nativeCases"] != serde_json::json!([case])
            || mapping["originalScenarios"].as_array().is_none_or(|cases| cases.is_empty())
            || format!("{:x}", Sha256::digest(source.as_bytes())) != *pin
            || !source.contains(&format!("fn {function}()"))
        {
            return Err(format!("Search scenario {id} has no current qualified Rust case."));
        }
        bindings.insert(*path, *pin);
        native_cases.insert(*case);
        scenario_ids.push(*id);
    }
    Ok(serde_json::json!({
        "status": "source-qualified-rust-case-crosswalk",
        "scope": "Aggregate search acceptance is owned by the current Rust cases; SwiftPM RuycastApp is compile/link smoke only. The former Swift search fixture and --benchmark-search route are retired; the distinct Rust --benchmark-model route remains and is not an AppState timing equivalent. Historical checkSearch mappings and the --check-search retirement are retained.",
        "scenarioIDs": scenario_ids,
        "nativeCases": native_cases,
        "qualifiedCaseSourceBindings": bindings,
        "retiredAssertions": [
            "Swift CLI/JSON wrapper and withObservationTracking callback counts are excluded; actual Rust socket responses, query/rows/selection/results revisions and Settings surface values are checked instead.",
            "expansionStatus has no independently mutable Rust counterpart; keyboard status belongs to the keyboard client.",
            "No Swift host startup, AppKit, desktop, performance benchmark or hardware acceptance is inferred from compile/link smoke."
        ],
        "requiresCurrentCompleteNativeValidation": true,
        "productionAccepted": false
    }))
}
pub(super) fn inventory(kind: &str) -> Result<serde_json::Value> {
    let (python_modules, python_packages) = if kind == "check" {
        discover_python_tests(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())?
    } else {
        (Vec::new(), Vec::new())
    };
    let gates: Vec<(&str, &str)> = match kind {
        "check" => vec![
            ("RuycastApp", "source-qualified-core-build-link-smoke"),
            ("--check-search", "source-qualified-rust-case-crosswalk"),
            ("--check-concurrency", "source-qualified-rust-case-crosswalk"),
            (
                "python-unittest-discover-Tests-test_*.py",
                python_discovery_state(&python_modules, &python_packages),
            ),
            ("git-diff-check", root_git_mapping_state(include_bytes!("compat.rs"))),
        ],
        "settings" => SETTINGS
            .iter()
            .map(|gate| (gate.id, settings_mapping_state(gate)))
            .collect(),
        _ => return Err("Unknown legacy gate inventory.".into()),
    };
    let unresolved = gates
        .iter()
        .filter(|(_, state)| !state.starts_with("source-qualified"))
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    let mut qualified = BTreeMap::new();
    for gate in SETTINGS {
        if settings_mapping_state(gate) == "source-qualified-rust-case-crosswalk" {
            qualified.insert(
                gate.id,
                settings_source_bindings(gate.id)
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
            );
        }
    }
    let mut inventory =
        serde_json::json!({"schema":"ruycast.original-gate-inventory.v1","pipeline":kind,"gates":gates,"unmappedGateIDs":unresolved,"settingsCaseCrosswalk":if kind=="settings"{serde_json::to_value(SETTINGS).map_err(error)?}else{serde_json::Value::Null},"qualifiedCaseSourceBindings":if kind=="settings"{serde_json::to_value(qualified).map_err(error)?}else{serde_json::Value::Null},"requiresCurrentCompleteNativeValidation":true,"pythonModuleInventory":python_modules,"pythonPackageInitializers":python_packages,"rustRootTestCrosswalk":if kind=="check"{serde_json::json!({"Tests/test_config_activation.py":{"scope":"both original activation rollback scenarios ported using actual Rust CLI and owned Unix socket; no live host","originalMethods":["test_rejected_candidate_restores_previous_bytes_and_runtime","test_newer_external_edit_is_never_rolled_back"],"nativeTarget":"startup_config_repair","nativeCases":["rejected_candidate_restores_previous_bytes_and_runtime","newer_external_edit_is_never_rolled_back"],"remainingPythonCases":0},"Tests/test_product_version.py":{"authority":"rust/src/product_version.rs","scope":"retired historical Swift-authority test consumer; five scenario intents now covered against actual Rust authority","originalMethods":["test_reader_and_actual_swift_compiler_agree_after_one_version_edit","test_missing_linked_ambiguous_or_nonliteral_version_is_rejected","test_export_follows_changed_compiled_version","test_invalid_version_creates_no_export_or_archive","test_nonregular_source_is_rejected_without_waiting_for_a_writer"],"nativeCases":["version::tests::owned_literal_matches_the_rust_compiler_and_rejects_link_fifo_directory_or_missing","version::tests::owned_rust_literal_tokens_preserve_whitespace_and_reject_ambiguity","version::tests::changed_compiled_version_exports_and_invalid_source_leaves_no_outputs"]},"Tests/test_config_directory.py":{"scope":"all fourteen scenarios ported, including qualified current-Rust manual-selector contract and actual Rust Raycast importer","originalMethods":["test_offline_relocation_preserves_bytes_and_runtime_files","test_existing_destination_is_not_overwritten","test_busy_writer_cannot_be_relocated","test_environment_pinned_directory_cannot_be_relocated","test_default_remains_compatible","test_invalid_selector_never_falls_back_to_stale_config","test_unavailable_persisted_config_is_not_initialized","test_persisted_directory_is_shared_and_survives_cli_restart","test_host_restart_uses_relocated_configuration","test_parent_component_after_symlink_is_rejected_by_cli_and_host","test_live_relocation_keeps_host_and_observer_on_new_file","test_manual_selector_change_does_not_silently_activate_wrong_file","test_environment_overrides_persisted_location","test_raycast_import_defaults_to_active_cli_config"],"currentContractDifferences":{"test_manual_selector_change_does_not_silently_activate_wrong_file":"Current Rust host automatically rebinds valid selectors; historical immediate CLI refusal and old host path are not guaranteed. Actual stale ConfigStore write refusal and unchanged copies are preserved; subsequent actual CLI edits target only the rebound selected file."},"additionalNativeTargets":{"runtime_process":["persisted_directory_is_shared_between_cli_and_owned_host_startup","owned_host_restart_uses_relocated_configuration","relocate_keeps_original_then_cli_edits_new_target","external_selector_rebinds_observation_to_portable_directory"]},"nativeTarget":"startup_config_repair","nativeCases":["offline_cli_relocation_retains_source_and_refuses_existing_target","offline_cli_busy_writer_and_environment_pin_refuse_relocation","offline_cli_default_path_and_invalid_selectors_never_use_stale_config","offline_cli_and_host_initializer_refuse_unavailable_persisted_config","offline_cli_and_host_initializer_refuse_parent_component_after_symlink","offline_cli_and_host_initializer_override_persisted_location_without_overwrite","raycast_import_defaults_to_active_cli_config"],"remainingPythonCases":0},"Tests/test_install_migration.py":{"scope":"all sixteen original scenarios ported; Python module and unused helper retired","originalMethods":["test_failed_bootstrap_restores_old_app_cli_and_registration","test_wrong_running_version_restores_old_app_and_registration","test_installer_migrates_single_service_and_both_cli_names","test_other_rustcast_app_cli_and_profile_are_preserved","test_existing_compatibility_alias_does_not_own_other_rustcast_app","test_signing_failure_keeps_existing_host_and_profile_untouched","test_installer_pins_explicit_configuration_for_login_startup","test_receipt_failure_never_stops_installed_app","test_invalid_canonical_version_never_stages_or_stops_current_app","test_profile_preserves_bytes_and_idempotent_alias","test_custom_config_is_backed_up_and_checked_separately_from_runtime","test_conflicting_profiles_and_links_are_preserved","test_unrelated_cli_symlink_is_not_hijacked","test_external_local_adapter_stages_without_entering_public_source","test_public_source_update_preserves_existing_local_adapter","test_wrong_bundle_identity_refused"],"authority":"rust/src/product_version.rs","buildAuthorityCases":["tests::genuine_owned_cargo_pair_emits_v4_and_rejects_stale_or_tampered_authority"],"nativeBinary":"ruycast-install","nativeCases":["installer::tests::installer_owned_failures_rollback_or_fail_before_stopping_without_config_changes","installer::tests::installer_owned_legacy_migration_preserves_authored_state_and_both_cli_names","installer::tests::installer_owned_unrelated_product_and_unknown_stage_are_preserved","installer::tests::installer_owned_explicit_app_profile_config_and_relative_legacy_alias","installer::tests::installer_owned_tampered_receipts_links_bundles_and_config_fail_before_stop","installer::tests::installer_owned_profile_filesystem_preserves_bytes_and_idempotent_alias","installer::tests::installer_owned_profile_filesystem_selected_config_hash_is_separate_from_runtime","installer::tests::installer_owned_profile_filesystem_conflicts_and_unrelated_cli_are_preserved","installer::tests::installer_owned_personal_adapter_external_stages_without_public_source_copy","installer::tests::installer_owned_personal_adapter_installed_bytes_preserved_and_link_refused","installer::tests::installer_owned_bundle_identity_refuses_unrelated_app_without_mutation"],"faults":["bootstrap","version","sign"],"remainingPythonFlowConsumers":0,"remainingPythonFilesystemCases":0}})}else{serde_json::Value::Null},"pythonDiscovery":"The original Tests/test_*.py unittest discovery is source-qualified from the actual package-aware Tests tree; any discovered module remains unresolved until its cases are transported and mapped","retiredSwiftChecks":if kind=="check"{serde_json::json!({"Tests/LauncherCoreTests/ScriptChoiceChecks.swift":{"status":"source-qualified-rust-case-crosswalk","scope":"The data-only helper is retired; the exact two original choices and all five search/validation scenarios remain explicit in one Rust test. The separate RuycastApp UI helper is now retired under its own assertion crosswalk; the no-op LauncherCoreChecks target is retired.","nativeSource":"rust-native/src/script_result.rs","originalScenarios":["accent-insensitive multi-term title/subtitle search","unmatched query returns no rows","empty query preserves all original rows in order","exact duplicate choice is rejected","empty choices validate"],"nativeCases":["script_result::tests::original_script_choice_search_and_validation_scenarios"]},"Tests/LauncherCoreTests/TaskDateInputChecks.swift":{"status":"source-qualified-rust-case-crosswalk","scope":"The data-only Swift helper is retired; all fourteen valid inputs, seven invalid inputs, three calendar boundaries, Dutch label and both prompt-support assertions remain in the existing Rust tests. The separate RuycastApp compile/link smoke remains active.","nativeSource":"rust-native/src/task_date.rs","nativeCases":["task_date::tests::original_natural_date_vectors_and_labels_match","task_date::tests::calendar_month_year_and_amsterdam_dst_boundaries_are_not_fixed_seconds","task_date::tests::strict_absolute_roundtrip_year_bounds_invalid_and_frozen_clock","task_date::tests::only_script_prompts_with_the_original_case_sensitive_hint_opt_in"]},"--check-concurrency":{"status":"source-qualified-rust-case-crosswalk","scope":"The Swift helper and its pre-main entry point are retired after mapping current Rust behavior. The Swift LockedValue stress assertion tested a Swift-only primitive, not a shared product contract. The old one-pending/latest/fair-order assertion has no exact Rust equivalent: Rust bounds the lane and returns busy for a second request. Synchronous-main ReplySink timing also differs; Rust uses asynchronous owner replies.","retiredAssertions":["Swift LockedValue 1000-update primitive stress test; the helper-only assertion is retired with its Swift primitive.","One queued latest request and fair [0,2000,1000] ordering; Rust uses busy-error admission and cancellation/revision fencing instead.","Synchronous-main intent reply; Rust completion is asynchronous through the owner queue."],"nativeCases":["control::tests::failed_start_retains_nonsocket_then_retry_releases_profile_lock","control::tests::failed_directory_bind_preserves_path_and_releases_profile_lock","control::tests::active_owner_and_replacement_paths_are_preserved","control::tests::large_wire_reply_roundtrips_without_truncation","system_search::service::tests::held_worker_bounds_admission_and_dropped_receipt_revokes_late_work","runtime::system::shortcut_budget_tests::browser_superseded_query_and_old_deadline_cannot_cancel_or_republish_latest_rows","runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response","system_search::tests::empty_files_query_returns_successful_empty_results_without_effects"]}})}else{serde_json::Value::Null},"unmappedIsFailure":true,"productionAccepted":false});
    if kind == "settings" {
        inventory["retiredSwiftChecks"] = serde_json::json!({
            "Sources/RuycastApp/SnippetEditingChecks.swift::checkSnippetEditing": {
                "originalSourceSHA256": SNIPPET_EDITING_ORACLE_SHA256,
                "status": "source-qualified-rust-case-crosswalk; Swift helper retired",
                "scope": "Only the snippet-specific Swift check helper/caller is retired. The broader SettingsChecks helper and product snippet editor remain active; the --check-settings mapping stays source-qualified and productionAccepted=false.",
                "productionAccepted": false,
                "contractDifferences": [
                    "The Rust native view records its actual ruler/text geometry; it does not reproduce the old SwiftUI screenshot widths or pixel captures.",
                    "The test host uses an owned hidden synthetic profile and local events; it does not prove physical desktop input or general-clipboard behavior.",
                    "Existing owned named-board, frozen corpus, rich keyword and HTML import tests remain their own Rust routes; this crosswalk does not claim live provider or personal-vault writes."
                ],
                "assertionCrosswalk": {
                    "Original six logical lines, trailing empty line, visible gutter, and minimum 44-point ruler": ["ui_completion::snippet_links"],
                    "Long wrapped first line and actual second-line layout below the original threshold": ["ui_completion::snippet_links"],
                    "Seventy logical lines scroll to visible markers above 50 while marker 1 leaves view": ["ui_completion::snippet_links"],
                    "Native text insertion followed by undo restores the exact original snippet": ["ui_completion::snippet_links"],
                    "Selection-bound link creation/edit, stale-source refusal, unsafe URL refusal and link undo/redo": ["ui_completion::snippet_links"],
                    "Rich/Teams serialization and private-board output": ["snippet_process::teams_paragraphs_html_breaks_and_safe_escaping", "snippet_process::native_named_board_contains_one_complete_plain_or_rich_item"],
                    "HTML hyperlink import, preserved blank paragraphs and no table blocks": ["snippet_probe::check_html_import"],
                    "Rich keyword one-paste path and legacy plain-text clipboard behavior": ["keyword_rich_process::html_and_teams_prepare_item_then_erase_and_command_v", "keyword_rich_process::plain_leaves_existing_clipboard_intact"]
                }
            }
        });
    }
    if kind == "check" {
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionChecks.swift::checkRenderedUpdates"] = serde_json::json!({
            "originalSourceSHA256": RENDER_UPDATES_ORACLE_SHA256,
            "status": "source-qualified-rust-case-crosswalk; Swift check route retired",
            "scope": "Only the --check-render-updates caller and checkRenderedUpdates helper are retired. The shared InteractionChecks.swift file, --check-interactions and --render-features routes remain active.",
            "nativeSource": "rust-native/examples/ui_completion.rs",
            "nativeSteps": ["render-updates-process"],
            "nativeCases": ["ui_completion::render_updates"],
            "swiftOriginalCaptures": 12,
            "rustRenderedCaptures": 26,
            "productionAccepted": false
        });
    }
    if kind == "check" {
        inventory["rustRootTestCrosswalk"]["Tests/test_cli.py"]["supplementalCurrentMethodByMethod"] = serde_json::json!({
            "test_existing_quick_note_provider_in_fixture_vault": "runtime_process::existing_quick_note_provider_writes_only_into_its_owned_fixture_vault",
            "test_clipboard_import_keeps_file_metadata_and_replays_without_duplicates": "clipboard_history_host_process::test_live_import_retains_file_metadata_deduplicates_and_rejects_unsafe_data_file",
            "test_script_timeout_and_bounded_output": "runtime_process::legacy_cli_script_timeout_and_bounded_output",
            "test_appearance_preferences_validate_and_persist": "runtime_process::legacy_cli_status_search_reload_and_config_edit",
            "test_launcher_management_commands_and_settings_hotkey": "remaining_macos_process::builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly",
            "test_clipboard_control_private_permissions_and_no_ui": "remaining_macos_process::cli_clipboard_controls_keep_private_files_and_reject_headless_ui",
            "test_second_instance_does_not_replace_socket": "runtime_process::second_instance_refused_and_crashed_socket_recovers",
            "test_script_timeout_kills_descendants_and_inherited_pipes": [
                "script_basic_process::timeout_cleans_group",
                "script_basic_process::early_success_closes_inherited_pipes"
            ]
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_cli.py"]["migratedMethods"] = serde_json::json!([
            "test_performance_fixture_ignores_external_configuration_override",
            "test_host_benchmark_rejects_changed_cli",
            "test_search_ranking_summary_and_confirmed_reset",
            "test_unknown_operation_returns_json_error",
            "test_failed_bridge_retains_provider_error_exit_status_and_queryable_log",
            "test_graceful_quit_removes_control_socket",
            "test_bounded_privacy_safe_performance_summary_and_confirmed_reset",
            "test_invalid_external_config_retains_working_runtime",
            "test_concurrent_config_edits_preserve_both_settings",
            "test_alias_search_and_dry_run",
            "test_add_remove_and_duplicate_alias",
            "test_unknown_config_field_is_rejected_by_rust_and_native_bridge",
            "test_command_separation_legacy_config_and_cli_toggle",
            "test_atomic_replacement_preserves_new_fields_and_rejects_drift",
            "test_snippet_output_roundtrip_and_rejection",
            "test_snippets_and_quicklinks_and_input_are_literal",
            "test_settings_command_is_searchable_and_headless_guarded",
            "test_script_literal_arguments_and_failure",
            "test_new_identity_and_legacy_environment",
            "test_config_set_validation_and_conflict"
        ]);
        inventory["rustRootTestCrosswalk"]["Tests/test_cli.py"]["nativeTargets"] = serde_json::json!({
            "startup_process": [
                "real_rust_benchmark_ignores_external_configuration_with_500_commands",
                "real_rust_benchmark_rejects_changed_cli_and_quits_gracefully"
            ],
            "rust_cli": ["startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects"],
            "private_results": [
                "provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace",
                "public_performance_cli_and_rpc_preserve_real_search_reset_schema_and_privacy"
            ],
            "runtime_process": [
                "quit_acknowledges_then_exits_normally_without_kill_fallback",
                "atomic_replace_invalid_last_good_and_recovery",
                "legacy_cli_status_search_reload_and_config_edit",
                "legacy_cli_script_timeout_and_bounded_output",
                "second_instance_refused_and_crashed_socket_recovers"
            ],
            "clipboard_history_host_process": ["test_live_import_retains_file_metadata_deduplicates_and_rejects_unsafe_data_file"],
            "script_basic_process": [
                "timeout_cleans_group",
                "early_success_closes_inherited_pipes"
            ],
            "execution_basic_process": ["cli_literal_input_uuid_usage_and_private_logs"],
            "remaining_macos_process": [
                "builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly",
                "cli_clipboard_controls_keep_private_files_and_reject_headless_ui"
            ]
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_cli.py"]["remainingPythonModule"] = serde_json::json!(false);
        inventory["rustRootTestCrosswalk"]["Tests/test_cli.py"]["remainingPythonCases"] = serde_json::json!(0);
        inventory["rustRootTestCrosswalk"]["Tests/test_architecture.py"] = serde_json::json!({
            "scope": "The two behavior-relevant architecture checks run in Rust; the old Swift import guard is retired with its frozen-source consumer under #256.",
            "originalMethods": [
                "test_portable_models_have_no_platform_or_runtime_dependency",
                "test_swift_uses_imported_c_symbols",
                "test_main_actor_and_worker_boundaries_are_compiler_checked"
            ],
            "retiredOriginalMethods": ["test_swift_uses_imported_c_symbols"],
            "nativeTarget": "architecture_policy",
            "nativeCases": [
                "portable_models_have_no_platform_or_runtime_dependency",
                "main_actor_and_worker_boundaries_are_compiler_checked"
            ],
            "remainingPythonCases": 0
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_build_pipeline.py"] = serde_json::json!({
    "scope": "All eight original scenario groups are bound to existing Rust cases; Python consumer retired. These scoped fixtures do not qualify a complete check/native/F1 pipeline.",
    "originalMethods": [
        "test_checks_keep_debug_artifacts_until_the_last_consumer",
        "test_lease_survives_controller_crash_while_consumer_is_live",
        "test_lease_survives_controller_interrupt_while_consumer_is_live",
        "test_source_edit_during_checks_cannot_receive_passed_receipt",
        "test_artifact_edit_during_checks_cannot_receive_passed_receipt",
        "test_settings_and_performance_share_the_whole_pipeline_lease",
        "test_failed_build_releases_lease_and_does_not_start_consumer",
        "test_receipt_revalidation_and_failed_attempt_remove_old_pass"
    ],
    "nativeCaseByMethod": {
        "test_checks_keep_debug_artifacts_until_the_last_consumer": [
            "compat::temporal_contract_tests::final_consumer_keeps_artifacts_leased_through_actual_receipt_publication"
        ],
        "test_lease_survives_controller_crash_while_consumer_is_live": [
            "artifact_lease::tests::owned_crash_retains_child_lease_through_the_final_consumer"
        ],
        "test_lease_survives_controller_interrupt_while_consumer_is_live": [
            "artifact_lease::tests::owned_interrupt_retains_child_lease_through_the_final_consumer"
        ],
        "test_source_edit_during_checks_cannot_receive_passed_receipt": [
            "compat::temporal_contract_tests::source_mutation_during_final_consumer_cannot_publish_pipeline_pass"
        ],
        "test_artifact_edit_during_checks_cannot_receive_passed_receipt": [
            "compat::temporal_contract_tests::artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass"
        ],
        "test_settings_and_performance_share_the_whole_pipeline_lease": [
            "compat::temporal_contract_tests::settings_writer_and_performance_authority_refusal_keep_the_pipeline_lease",
            "compat::temporal_contract_tests::shared_pipeline_lease_survives_successful_consumer_and_final_return"
        ],
        "test_failed_build_releases_lease_and_does_not_start_consumer": [
            "compat::tests::actual_failed_build_stops_consumers_and_releases_pipeline_lease"
        ],
        "test_receipt_revalidation_and_failed_attempt_remove_old_pass": [
            "compat::tests::pipeline_receipt_bindings_accept_check_and_settings_and_reject_field_drift",
            "compat::tests::pipeline_receipt_readback_rejects_live_source_and_artifact_drift",
            "compat::tests::failed_final_consumer_revokes_previous_pass_and_releases_pipeline_lease",
            "compat::tests::incomplete_full_gate_invalidates_its_previous_pass_and_cannot_reverify",
            "compat::temporal_contract_tests::artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass"
        ]
    },
    "nativeTargets": {
        "rust-native --lib": {
            "source": "rust-native/src/artifact_lease_tests.rs",
            "cases": [
                "artifact_lease::tests::owned_crash_retains_child_lease_through_the_final_consumer",
                "artifact_lease::tests::owned_interrupt_retains_child_lease_through_the_final_consumer"
            ]
        },
        "rust-native --bin ruycast-build": {
            "source": "rust-native/src/bin/ruycast-build/compat.rs",
            "cases": [
                "compat::temporal_contract_tests::artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass",
                "compat::temporal_contract_tests::final_consumer_keeps_artifacts_leased_through_actual_receipt_publication",
                "compat::temporal_contract_tests::settings_writer_and_performance_authority_refusal_keep_the_pipeline_lease",
                "compat::temporal_contract_tests::shared_pipeline_lease_survives_successful_consumer_and_final_return",
                "compat::temporal_contract_tests::source_mutation_during_final_consumer_cannot_publish_pipeline_pass",
                "compat::tests::actual_failed_build_stops_consumers_and_releases_pipeline_lease",
                "compat::tests::failed_final_consumer_revokes_previous_pass_and_releases_pipeline_lease",
                "compat::tests::incomplete_full_gate_invalidates_its_previous_pass_and_cannot_reverify",
                "compat::tests::pipeline_receipt_bindings_accept_check_and_settings_and_reject_field_drift",
                "compat::tests::pipeline_receipt_readback_rejects_live_source_and_artifact_drift"
            ]
        }
    },
    "contractDifferences": {
        "artifactConfiguration": "The historical disposable Swift debug host/modules are retired production artifacts. Current Rust check/settings builds use genuine prepared release host/CLI artifacts; temporal cases hold the same lease through final receipt publication and verify real artifact mutation.",
        "performance": "The actual public performance path holds its lease while an owned authority validator refuses (exit 17), publishing no performance result. A separate successful disposable consumer proves the shared lease survives child success and the post-child tail. Neither case claims a successful reviewed F1 comparison.",
        "receiptSchema": "Historical verification-receipt.v1 asserted status passed and steps [rustTests, build, verification]; it also carried elapsedSeconds, exitCode and lockWaitSeconds. Pipeline v2 replaces this receipt structure with sources, buildReceiptSha256 and nativeReportSha256 plus productionAccepted=false. v1 stage order/timings/lockWaitSeconds are not v2 fields or retained wire promises; actual producer/consumer ordering and refusal are covered separately.",
        "staleArtifacts": "The old revalidation test changed a Swift module and CLI binary. V2 readback covers test edits, added docs and changed build/native-report binding bytes. Actual executable mutation/no-pass is covered by the real prepared-artifact temporal case, not claimed from synthetic receipt hashes. No current Swift-module mutation equivalent is asserted."
    },
    "failureSemantics": "Producer failure starts no later consumer; incomplete gates revoke old passes and refuse revalidation; an actual final consumer exits 17 after observing the old fixturepass removed, cannot publish a new pass, and releases the lease to an independent process. Current production errors replace historical stub exit 7 and v1 error wording; no stub compiler status is a product CLI contract.",
    "subprocessPolicy": "Rust owned subprocess fixtures replace Python Popen/communicate. Marker waits and captures remain bounded; process::capture retires owned process groups and reaps direct children. Lease tests separately check SIGKILL/SIGINT status, live-consumer nonblocking flock refusal, subsequent independent acquisition and orphan retirement. Temporal fixtures retain exclusively owned evidence for explicit post-review cleanup.",
    "timeoutContract": {
        "historical": "Python marker waits 6s, disposable consumer 10s, communicate 5/6/8s, controller wait 3s; cleanup releases consumers then waits 6s, kills and reaps within 3s.",
        "rust": "Lease marker/orphan retirement waits 8s; temporal markers 45s, consumer capture 60s and outer capture 90s accommodate genuine tiny Rust producers. Failed-producer capture 30s and contender 8s; failed-final-consumer capture 5s, attempt 15s, contender 8s. These are explicit fixture budgets, not product-response promises."
    },
    "remainingMethods": [],
    "remainingPythonCases": 0
});
        inventory["rustRootTestCrosswalk"]["Tests/test_local_signing.py"] = serde_json::json!({
    "scope": "All eleven original signer methods are bound to existing Rust tests, including private-keychain stdin/redaction, ordinary pinned sign and installed-certificate/ad-hoc contracts. The Python implementation and its test consumer are retired; no real signer acceptance is claimed.",
    "originalMethods": [
        "test_repeat_prepare_reuses_without_generating_or_touching_keychain",
        "test_missing_private_state_never_rotates_identity",
        "test_unsafe_private_modes_and_symlinks_are_refused",
        "test_requirement_cannot_be_weakened",
        "test_signer_failure_never_uses_adhoc_or_modifies_trust",
        "test_success_signs_and_verifies_exact_pinned_requirement",
        "test_unexpected_installed_certificate_is_refused",
        "test_one_time_adhoc_migration_checks_bundle_identity",
        "test_signing_keychain_preserves_existing_search_entries",
        "test_adhoc_is_refused_after_first_certificate_install",
        "test_private_keychain_password_is_only_sent_via_stdin"
    ],
    "nativeBinary": "ruycast-install",
    "nativeCaseByMethod": {
        "test_repeat_prepare_reuses_without_generating_or_touching_keychain": [
            "installer::tests::installer_owned_first_install_provisions_and_pins_without_secret_argv",
            "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades"
        ],
        "test_missing_private_state_never_rotates_identity": [
            "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades",
            "installer::tests::installer_owned_incomplete_signer_without_manifest_is_preserved"
        ],
        "test_unsafe_private_modes_and_symlinks_are_refused": [
            "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades",
            "installer::tests::installer_owned_signer_root_symlink_or_permissive_mode_is_refused"
        ],
        "test_requirement_cannot_be_weakened": [
            "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades"
        ],
        "test_signer_failure_never_uses_adhoc_or_modifies_trust": [
            "installer::tests::ordinary_sign_owned_pins_verification_and_refuses_without_fallback_or_mutation"
        ],
        "test_success_signs_and_verifies_exact_pinned_requirement": [
            "installer::tests::ordinary_sign_owned_pins_verification_and_refuses_without_fallback_or_mutation"
        ],
        "test_unexpected_installed_certificate_is_refused": [
            "installer::tests::installed_signing_owned_certificate_refusal_and_initial_adhoc_preserve_state"
        ],
        "test_one_time_adhoc_migration_checks_bundle_identity": [
            "installer::tests::installed_signing_owned_certificate_refusal_and_initial_adhoc_preserve_state"
        ],
        "test_signing_keychain_preserves_existing_search_entries": [
            "installer::tests::installer_owned_enrollment_blocks_adhoc_and_search_list_keeps_existing_entries"
        ],
        "test_adhoc_is_refused_after_first_certificate_install": [
            "installer::tests::installer_owned_enrollment_blocks_adhoc_and_search_list_keeps_existing_entries",
            "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades"
        ],
        "test_private_keychain_password_is_only_sent_via_stdin": [
            "installer::signing::private_keychain_tests::private_keychain_stdin_and_redacted_failures_use_only_fake_runner"
        ]
    },
    "nativeCases": [
        "installer::signing::private_keychain_tests::private_keychain_stdin_and_redacted_failures_use_only_fake_runner",
        "installer::tests::installed_signing_owned_certificate_refusal_and_initial_adhoc_preserve_state",
        "installer::tests::installer_owned_enrollment_blocks_adhoc_and_search_list_keeps_existing_entries",
        "installer::tests::installer_owned_first_install_provisions_and_pins_without_secret_argv",
        "installer::tests::installer_owned_incomplete_signer_without_manifest_is_preserved",
        "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades",
        "installer::tests::installer_owned_signer_root_symlink_or_permissive_mode_is_refused",
        "installer::tests::ordinary_sign_owned_pins_verification_and_refuses_without_fallback_or_mutation"
    ],
    "nativeCaseSources": {
        "installer::signing::private_keychain_tests::private_keychain_stdin_and_redacted_failures_use_only_fake_runner": "rust-native/src/bin/ruycast-install/signing.rs",
        "installer::tests::installed_signing_owned_certificate_refusal_and_initial_adhoc_preserve_state": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::installer_owned_enrollment_blocks_adhoc_and_search_list_keeps_existing_entries": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::installer_owned_first_install_provisions_and_pins_without_secret_argv": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::installer_owned_incomplete_signer_without_manifest_is_preserved": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::installer_owned_signer_never_rotates_weakens_or_downgrades": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::installer_owned_signer_root_symlink_or_permissive_mode_is_refused": "rust-native/src/bin/ruycast-install/tests.rs",
        "installer::tests::ordinary_sign_owned_pins_verification_and_refuses_without_fallback_or_mutation": "rust-native/src/bin/ruycast-install/tests.rs"
    },
    "contractDifferences": {
        "authority": "The removed Python __main__ already forwarded to the Rust installer; no signing authority, identity schema, certificate pin, search-list or enrollment policy is replaced. Historical Python error wording is represented by current Rust refusal contracts.",
        "execution": "Original Python mocks and temporary directories are replaced by owned fixture identities and fake Runner calls. Ordinary sign and checker tests remain distinct: ordinary sign tests model its existing unlock/search-list protocol; checker tests never unlock or provision. No real security/codesign/trust/privacy/app effects are required.",
        "deadlinesAndCleanup": "Private keychain/sign/check calls retain explicit 30-second Call budgets. Existing Rust runner tests preserve bounded stdin/stdout, timeout and process-group reaping; owned filesystem fixtures remove only their own temporary roots."
    },
    "retiredConsumers": [
        "Tests/test_local_signing.py",
        "scripts/local_signing.py"
    ],
    "remainingMethods": [],
    "remainingPythonCases": 0
});
        inventory["rustRootTestCrosswalk"]["Tests/test_benchmark_host.py"] = serde_json::json!({
            "scope": "All seven original benchmark-driver scenarios now run through the Rust benchmark-host executable and Rust-only child fixtures; the Python test module has been deleted.",
            "originalMethods": [
                "test_success_requires_idle_responsiveness_and_graceful_exit",
                "test_forced_termination_is_a_failure_even_when_child_exits_zero",
                "test_ignored_termination_is_killed_and_never_marked_successful",
                "test_child_timeouts_write_failure_receipts",
                "test_idle_socket_hang_cannot_pass_on_healthy_os_counters",
                "test_partial_response_cannot_extend_rpc_deadline_indefinitely",
                "test_failure_keeps_completed_requests_and_partial_samples"
            ],
            "nativeTarget": "benchmark_host_process",
            "nativeCases": [
                "shutdown_requires_acknowledged_quit_and_distinguishes_signals",
                "child_timeouts_write_failure_receipts",
                "idle_status_hang_and_drip_cannot_pass_on_healthy_counters",
                "active_socket_failure_keeps_completed_requests_and_partial_samples"
            ],
            "additionalNativeTargets": {"startup_process": [
                "real_rust_benchmark_ignores_external_configuration_with_500_commands",
                "real_rust_benchmark_rejects_changed_cli_and_quits_gracefully"
            ]},
            "remainingPythonCases": 0
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_public_contract.py"] = serde_json::json!({
            "scope": "All seven original behavioral cases are bound to Rust; the Python test module has been deleted.",
            "originalMethods": [
                "test_neutral_defaults_and_structured_contract",
                "test_release_strip_preserves_real_sections_and_matching_diagnostics",
                "test_cached_artifacts_keep_their_original_diagnostics_across_partial_rebuilds",
                "test_second_replace_failure_and_interrupted_pair_recover_before_retry",
                "test_mismatched_dsym_cannot_mutate_build_artifacts",
                "test_source_mutation_during_build_cannot_receive_valid_receipt",
                "test_build_and_install_share_exclusive_artifact_lock"
            ],
            "nativeCaseByMethod": {
                "test_neutral_defaults_and_structured_contract": "oracles::tests::public_defaults_and_structured_vision_contract_are_neutral",
                "test_release_strip_preserves_real_sections_and_matching_diagnostics": "preparation::tests::real_rust_strip_keeps_code_uuid_and_matching_diagnostics_and_cached_originals",
                "test_cached_artifacts_keep_their_original_diagnostics_across_partial_rebuilds": "preparation::tests::real_rust_strip_keeps_code_uuid_and_matching_diagnostics_and_cached_originals",
                "test_second_replace_failure_and_interrupted_pair_recover_before_retry": "preparation::tests::failed_second_replace_rolls_back_pair_and_interrupted_pair_is_recovered",
                "test_mismatched_dsym_cannot_mutate_build_artifacts": "preparation::tests::valid_dsym_with_wrong_uuid_is_refused_before_binary_publication",
                "test_source_mutation_during_build_cannot_receive_valid_receipt": "compat::tests::build_receipt_record_refuses_source_mutation_without_creating_receipt",
                "test_build_and_install_share_exclusive_artifact_lock": "artifact_lease::tests::owned_nested_consumers_do_not_unlock_parent_and_bad_capabilities_fail_closed"
            },
            "remainingMethods": [],
            "remainingPythonCases": 0,
            "artifactLeasePath": "<repository>/.build/artifacts.lock",
            "artifactLeaseCallers": {
                "build": "ruycast-build.rs uses BuildLock, a type alias for ruycast_native::artifact_lease::Lease",
                "install": "ruycast-install/mod.rs acquires the same Lease type"
            }
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_package_app.py"] = serde_json::json!({
            "scope": "all seven packaging test contracts are bound to Rust candidate/source checks; the Python test consumer is retired",
            "originalMethods": ["test_candidate_has_only_explicit_bundle_resources_and_no_publication_claim", "test_private_checkout_and_missing_manifest_are_rejected_before_staging", "test_candidate_verification_rejects_post_staging_mutation_or_extra_file", "test_stale_source_or_binary_receipt_cannot_be_packaged", "test_existing_artifact_and_symlink_are_preserved", "test_changes_during_copy_do_not_expose_mixed_candidate", "test_linked_source_and_invalid_metadata_are_rejected"],
            "nativeCases": ["candidate::tests::real_owned_prepared_pair_stages_exclusively_and_rejects_mixed_source", "candidate::tests::public_candidate_refuses_private_checkout_or_missing_manifest_before_staging", "candidate::tests::payload_verification_rejects_tampering_extra_helpers_modes_and_links", "candidate::tests::safari_automation_purpose_survives_rust_packaging_with_or_without_icon", "public_source::tests::owned_postbuild_binding_allows_outputs_and_rejects_tamper_new_sources_and_private_authority", "compat::tests::legacy_argument_contract_rejects_duplicates_unknowns_and_unreviewed_performance"],
            "remainingPythonCases": 0
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/PrivateFileChecks.swift"] = serde_json::json!({
            "originalSourceSHA256": "ce4aa302f38466fc22cd557234644e9291337f18b0cfe7287a4a7318d7decf9f",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The data-only Swift helper is retired; its original ordered sequence runs through the shared public write(path, bytes, replace) entrypoint. The separate RuycastApp compile/link smoke remains active.",
            "nativeSource": "rust/src/private_storage.rs",
            "originalScenarios": ["replace a symlink without changing target bytes; publish a regular 0600 file", "refuse directory replacement and preserve child bytes", "create-only first write succeeds; second refuses and preserves first payload", "create-only refuses a symlink and preserves target bytes", "exact five directory-entry names prove temporary stages are removed"],
            "nativeCases": ["private_storage::guarded_publication_tests::original_private_file_replacement_and_create_only_sequence"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/SnippetOutputChecks.swift"] = serde_json::json!({
            "originalSourceSHA256": "abaa04463b8b7fcfa1edecfd712d4911d01735991f3ecfd3c2e32455a8746965",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The data-only helper is retired; its original rich rendering, literal/unsafe input, cursor, three-link, link-source, choice replacement and mode-codec cases remain covered by Rust tests. The separate RuycastApp compile/link smoke remains active.",
            "nativeSources": ["rust-native/src/snippet_output.rs", "rust-native/tests/snippet_process.rs", "rust/src/lib.rs"],
            "originalScenarios": ["Teams/html rich payload text, escaping, paragraph layout and cursor", "nil mode preserves the exact original text without HTML", "newline link labels are rejected", "unsafe image and javascript/file links produce no raw image or anchor", "four UTF16 emoji-link cursor offsets and one in-link cursor mapping", "three Teams links retain labels and order", "rich argument choice replacement preserves text and hyperlink", "link source round-trips bracketed, escaped, emoji and parenthesized labels/URLs", "plain, teams and html command JSON codec and configuration validation", "unknown snippet output mode is rejected"],
            "nativeCases": ["snippet_process::teams_paragraphs_html_breaks_and_safe_escaping", "snippet_process::plain_unknown_modes_and_complete_literal_text", "snippet_process::links_balanced_parentheses_escape_labels_and_invalid_syntax", "snippet_process::utf16_cursor_mapping_at_every_position_and_grapheme_boundary", "snippet_output::link_editor_tests::teams_rendering_preserves_all_original_three_links", "snippet_output::link_editor_tests::rich_choice_replacement_preserves_original_text_and_hyperlink", "snippet_output::link_editor_tests::link_source_round_trips_original_escaped_labels_and_parentheses", "snippet_output::link_editor_tests::original_literal_and_unsafe_snippet_sources_stay_safe", "rustcast_core::tests::snippet_output_modes_roundtrip_validate_and_reject_unknown"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/ABIChecks.swift::checkPreparedRanking"] = serde_json::json!({
            "originalSourceSHA256": "ef4c6e126a2c2183a86aa8fc97c7accf9924d5eaaa379d518c9cf7fce97a8334",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The prepared ranking snapshot assertions moved to Rust. The remaining ABI behavior assertions are crosswalked separately; Swift C-importer and PreparedCommandSearch owner-retention behavior are retired with the Swift helper and are not claimed as Rust coverage.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["an already prepared catalog keeps its original ranking", "recording a successful choice increments revision and a new catalog ranks that choice first", "reset increments revision and restores catalog order", "stale recency for a different command does not affect a replacement catalog"],
            "nativeCases": ["ranking::tests::original_prepared_ranking_snapshots_track_success_reset_and_replacement"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/ABIChecks.swift::checkImportedABI"] = serde_json::json!({
            "originalSourceSHA256": "19811571df19e5dd8c5cb066f5a5a15ad13f5c37d482219aa17d96a4dbc48eb3",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The ABI version and exact weighted/empty-query index results are checked through the Rust C ABI. Swift C-importer dispatch and PreparedCommandSearch ARC owner-retention are Swift-wrapper behavior retired with the helper, not Rust coverage.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["the Rust ABI reports version 1", "same with recency second=5 returns command indices second then first", "an empty query returns the original command indices first then second"],
            "retiredAssertions": ["Swift C-imported declarations invoke the linked archive through the Swift wrapper", "PreparedCommandSearch retains the Swift CommandSearchIndex owner after its local creator exits"],
            "nativeCases": ["tests::original_imported_abi_version_ranked_and_empty_query_order"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testExactAliasBeatsTitleAndSupportsMultipleTerms"] = serde_json::json!({
            "originalSourceSHA256": "bb86e8804323ae68cfd126f75d9c5d4af70e354fadc1356a53c89eea3452ee34",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The exact Finder title, multi-term local query and uppercase saved alias assertions moved to the existing Rust runtime search and resolver; unrelated CoreTests remain active.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["finder resolves to the existing app.finder command", "finder documentation resolves to the added link command", "uppercase DOCS resolves the saved link alias"],
            "nativeCases": ["runtime::inline_tests::original_exact_alias_title_multiterm_and_uppercase_resolution"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testBoundedTypoFallback"] = serde_json::json!({
            "originalSourceSHA256": "82ba1d4e045a1c35f59834ed005e764240f4d8fbd211fd7d130b113fbeb57c8e",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "All eighteen original bounded typo-fallback search outcomes and both literal-resolution outcomes move to the existing Rust search index and runtime resolver. Search order, recency, alias precedence, direct literal matching, Unicode and the 64/65-character boundary remain explicit; the product implementation is unchanged.",
            "nativeSources": ["rust/src/lib.rs", "rust-native/src/runtime_private_tests.rs"],
            "originalScenarios": ["exact alias outranks Finder recency", "Calendar typo resolves", "equal Finder results preserve order", "Finder recency orders equal-title results", "Mail and Main typo preserve order", "recency moves Main before Mail", "literal resolver refuses fnider", "literal resolver accepts finder", "Finder typo outranks weak Find Reminders subsequence", "high distractor recency does not displace direct title typo", "Reminders transposition returns direct title and weak distractor", "distractor recency does not alter that ordering", "Notes typo keeps original ranking despite decorated recency", "punctuated noets! returns no result", "literal calender URL substring keeps priority", "Cat typo cta returns no result", "Åbcd typo Åbdc matches", "decomposed-accent query does not match", "64-character transposition matches", "65-character transposition is rejected"],
            "nativeCases": ["tests::original_bounded_typo_fallback_search_cases", "runtime::inline_tests::original_bounded_typo_fallback_keeps_resolution_literal"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.longText"] = serde_json::json!({
            "originalSourceSHA256": "6eda2747d7ea8c664a44070efbb38078fae2be761b939530b9270669be8ab84e",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The two long-text chunk assertions are covered by Rust. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust-native/src/keyword_matcher.rs",
            "originalScenarios": ["100 repetitions of Hello Ω family emoji and newline concatenate to the original text", "every inserted chunk contains at most 16 UTF-16 code units"],
            "nativeCases": ["keyword_matcher::cancellation_checks::original_long_snippet_unicode_chunks_preserve_complete_text_and_utf16_limit"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.requiredArgument"] = serde_json::json!({
            "originalSourceSHA256": "56df09b0b0669f0a326ebcde08f54c2f9db3b539bc6a77b65d6b33dd497b37d3",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The required-argument snippet eligibility assertion is covered by Rust. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust-native/src/keyword_matcher.rs",
            "originalScenarios": ["a snippet with required Name input and no options or default does not expand its ;name keyword"],
            "nativeCases": ["keyword_matcher::cancellation_checks::original_required_snippet_argument_prevents_keyword_expansion"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.matcher"] = serde_json::json!({
            "originalSourceSHA256": "dba3ad2b3ec56498c7fa2613b850a488ddb1e99d496847c0bdaabbf7e886206b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The six snippet-prefix and word-boundary assertions are covered by one Rust matcher test. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust-native/src/keyword_matcher.rs",
            "originalScenarios": ["incomplete ;h prefix returns no match", "typing i completes ;hi and returns the full Greeting command", "a second i after the successful match returns no match", "backspace changes ;hx to ;h before i completes the match", "xabc does not match at an internal word boundary", "abc returns the full Word command"],
            "nativeCases": ["keyword_matcher::cancellation_checks::original_snippet_prefix_reset_backspace_and_word_boundary"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.snippetRender"] = serde_json::json!({
            "originalSourceSHA256": "bdb976419921267514aba471ddb15032aeda190a3893315a8233113d8407352b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The three Greeting output, UTF-16 cursor-prefix and missing-clipboard assertions are covered by Rust. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust-native/src/templates.rs",
            "originalScenarios": ["clipboard Ω with the default Tone option renders Hi Ω, Hello", "the cursor prefix is exactly Hi Ω, ", "rendering without clipboard text returns Clipboard text is unavailable."],
            "nativeCases": ["templates::tests::original_snippet_clipboard_default_cursor_and_missing_clipboard"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.urlRender"] = serde_json::json!({
            "originalSourceSHA256": "200fc776a731ffdf77637be04f67868d50f4c0b5cdcfeb98e2efeb446ce5c4e0",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The four URL-template field, encoded query-value and missing-input outcomes are covered by Rust. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust-native/src/templates.rs",
            "originalScenarios": ["URL template fields are Query then Place", "encoded query q decodes to a&b #/Ω", "encoded query place decodes to New York", "rendering without URL field values throws"],
            "nativeCases": ["templates::tests::original_url_template_fields_encoding_and_missing_input"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.duplicateKeyword"] = serde_json::json!({
            "originalSourceSHA256": "a4ed4e769adecebd65fcc7a2cc2e95c85574ac34b7468cb84988b8af03f22686",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The duplicate snippet keyword validation assertion is covered by Rust. The Swift helper was retired after every scenario in the method received a Rust crosswalk.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["a single snippet with keyword abc validates", "a second snippet with the same keyword abc is rejected"],
            "nativeCases": ["tests::original_duplicate_snippet_keyword_is_rejected"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.emptyCatalog"] = serde_json::json!({
            "originalSourceSHA256": "f4cbecca6b0db72586d7cd56f1a3e0c1b0c7a2c6322d981be1e989508c467975",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the empty-catalog empty-query assertion moved to Rust. The surrounding Swift method and runner call remain active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["searching an empty catalog with an empty query returns no commands"],
            "nativeCases": ["tests::original_empty_catalog_search_returns_no_commands"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.unicodeScalars"] = serde_json::json!({
            "originalSourceSHA256": "30bd876a5a37a0cf0363a24b5aa75ac877a34fff60eb3259be91f0faf5f22b68",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the two Unicode-scalar search assertions move to the existing Rust search index. The remaining ranking method and runner call stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query Ω returns only the scalar command", "query İ lowercases to two Unicode scalars and returns only the expanded command"],
            "nativeCases": ["tests::original_unicode_scalar_and_expanded_lowercase_search"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.largeTies"] = serde_json::json!({
            "originalSourceSHA256": "063c54f1cf0c835dd3547f07155d2dc9b446e93ed43c34459647913f3d38e435",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the three equal-title tie assertions move to Rust. The remaining ranking and storage assertions, Swift method and runner call stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["128 equal-title results preserve catalog order for title", "exact alias same remains ahead of recency-ranked tie.64", "128 equal-title fuzzy results preserve catalog order for s t"],
            "nativeCases": ["tests::original_large_equal_title_ties_preserve_order_and_alias_precedence"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.qualityRanking"] = serde_json::json!({
            "originalSourceSHA256": "a24ca5f72b7ec2c1f3f428ef93720450f723477157bad995a47eb4ef79ef6455",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the four quality-ranking assertions and their fixture move to Rust. The remaining search and storage assertions, Swift method and runner call stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["exact alias mt stays ahead of recent result when recency is high", "multi-term prefix query mic tea returns teams first", "multi-term fuzzy query mft returns teams first", "exact Microsoft Teams title stays ahead of recent history result"],
            "nativeCases": ["tests::original_quality_ranking_alias_recency_and_multi_term_search"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.legacyParity"] = serde_json::json!({
            "originalSourceSHA256": "b6bb2be6a66ec71d5467fcd1363d9cd9e2f668b6dfd9d081f1b3bd01140f1427",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The seven-query four-command search parity loop now checks complete Rust index/dispatch results and ranked ABI order. Swift ranking storage, runner and private library route remain active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query go", "query BETA with surrounding spaces", "query ÅNGSTRÖM notes", "query nul\\0alias", "blank query", "query does not exist", "query example"],
            "nativeCases": ["tests::original_seven_query_index_dispatch_and_abi_parity"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.blankQueries"] = serde_json::json!({
            "originalSourceSHA256": "a0f320926f7fd9743b2b669d8e636bf3fb2714d102099b18f20d340b1abc523d",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the two blank-query assertions on the nonempty catalog move to Rust. The fixture, other search and storage assertions, Swift method and runner stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["whitespace query returns all commands in catalog order", "empty query with the original z recency entry returns all commands in catalog order"],
            "nativeCases": ["tests::original_blank_queries_preserve_nonempty_catalog_order"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.missingQuery"] = serde_json::json!({
            "originalSourceSHA256": "4aeec882071caf47f6f6c27890b3c32c0f93dad869fdc64470f1dce75802122a",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct no-match assertion on the nonempty catalog moves to Rust. The same query in the Swift legacy-parity loop remains active, as do the fixture, other assertions, method and runner.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query does not exist returns no commands on the four-command catalog"],
            "nativeCases": ["tests::original_nonempty_catalog_missing_query_returns_no_commands"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.embeddedNulAlias"] = serde_json::json!({
            "originalSourceSHA256": "e1e6c77a1161ae3c0410fe50ac1e5c4e475926928e6e22b6d85226a17d7fdd9b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct embedded-NUL alias assertion moves to Rust. The shared Swift fixture, legacy comparison loop and remaining ranking/storage checks stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query nul\\0alias returns only command b"],
            "nativeCases": ["tests::original_embedded_nul_alias_search_returns_matching_command"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.trimmedUppercaseQuery"] = serde_json::json!({
            "originalSourceSHA256": "42804ffaac4e94b6c7576b899e9f4cb74002a2d30ac9fb76c833aa65c20c1eee",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct trimmed uppercase query assertion moves to Rust. The shared Swift fixture, legacy comparison loop and remaining ranking/storage checks stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query BETA with surrounding spaces returns command IDs z then c"],
            "nativeCases": ["tests::original_trimmed_uppercase_query_returns_matching_commands"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.accentedUppercaseQuery"] = serde_json::json!({
            "originalSourceSHA256": "0ee7c7c4c529fd9a875b98b0cc8c6a19a48f6cf3a1f6fa8b7ac45a0ec1765dd1",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct accented uppercase title query assertion moves to Rust. The shared Swift fixture, legacy comparison loop and remaining ranking/storage checks stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["case-insensitive query ÅNGSTRÖM notes returns only command b"],
            "nativeCases": ["tests::original_accented_uppercase_title_query_returns_matching_command"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.goAliasOrder"] = serde_json::json!({
            "originalSourceSHA256": "8376288be5aa8ecd4a2eb4794ee49281b575bfc91cff835acf1b8e95909e6c63",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct full-result go alias assertion moves to Rust. The shared Swift fixture, legacy comparison loop and remaining ranking/storage checks stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["query go returns ordered command IDs z, a, b"],
            "nativeCases": ["tests::original_go_alias_query_preserves_complete_result_order"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.goAliasTemporaryIndex"] = serde_json::json!({
            "originalSourceSHA256": "6329a81b5813d5242f548ccea7f514e54ba40670e9a250796e57831c1200b9b5",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the 100 temporary Swift index constructions and first go-alias result assertions move to Rust. The shared fixture, legacy comparison, method, runner and other checks stay active.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["100 fresh search indexes return command z first for the go alias"],
            "nativeCases": ["tests::original_go_alias_query_preserves_complete_result_order"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.privatePermissions"] = serde_json::json!({
            "originalSourceSHA256": "d318506a3da5ce51fad4040432486b656bb8553840306964b9df698f3a93f718",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the successful-ranking file's 0600 mode assertion moves to Rust. The remaining Swift ranking/storage flow, assertions, method and runner stay active.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["after a successful choice, search-ranking.json has mode 0600"],
            "nativeCases": ["ranking::tests::original_successful_choice_ranking_file_has_private_permissions"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.persistedTimestamp"] = serde_json::json!({
            "originalSourceSHA256": "80b3dc47d5203860e1e775eb4338ce974267902a2bd1b353b146ce28b8e5325b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the exact chosen-timestamp-after-reopen assertion moves to Rust. The remaining Swift retention/reset assertions, method and runner stay active.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["after record_success, reopening RankingStore preserves chosen at the exact original timestamp"],
            "nativeCases": ["ranking::tests::original_successful_choice_timestamp_survives_reopen"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::CheckRunner.library"] = serde_json::json!({
            "originalSourceSHA256": "5f09411bdd0943a44188f2b1f6e851aa9ca14cee681fdaba092ce374e694ff36",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The uncalled private-library route is retired; its config, default/option/fallback template values, clipboard/date rendering and keyword-to-own-command aggregation are covered using synthetic Rust configuration.",
            "nativeSource": "rust-native/tests/library_verification.rs",
            "originalScenarios": ["valid URL and snippet commands load from configuration", "template fields choose default, first option or Fixture", "clipboard and fixed date render without printing private text", "every command keyword returns its own command", "commands with no fields and no keyword complete"],
            "nativeCases": ["original_private_library_aggregation_renders_templates_and_checks_keywords"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.rankingSmoke"] = serde_json::json!({
            "originalSourceSHA256": "36067c3a65ad03c93f76c6664118bedc1caed1fc6c89d046f292b132acb3e1f2",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The original Swift seeded ranking load, recordSuccess and reset smoke flow is covered by the Rust reset/reopen behavior test. The no-op core runner and retired Swift private-library route provide no active behavior authority.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["load a 600-entry ranking seeded with expired and future timestamps", "record success for chosen", "reset the ranking store"],
            "nativeCases": ["ranking::tests::original_reset_ranking_is_empty_after_reopen"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.resetReopen"] = serde_json::json!({
            "originalSourceSHA256": "11160b6434aafdad4a752f09616cae0bcb42d46acca01d6e00596650b759a525",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the empty ranking-after-reset-and-reopen assertion moves to Rust. The active Swift reset call, method and runner stay active; loaded-limit and timestamp-filter checks are crosswalked separately.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["after a successful choice, reset leaves zero ranking entries after reopening the store"],
            "nativeCases": ["ranking::tests::original_reset_ranking_is_empty_after_reopen"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.loadedEntryLimit"] = serde_json::json!({
            "originalSourceSHA256": "c069c4eb7a5bb4941bb1d7ffb4840b330e3e00ab8b514f0d4fcd7fbf2809cfab",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the loaded-entry count limit assertion moves to Rust. The source seed and record/reset calls, method and runner stay active; expired and future filtering are crosswalked separately.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["loading the original schema-1 ranking seed caps retained entries at 512"],
            "nativeCases": ["ranking::tests::original_loaded_ranking_is_capped_at_512_entries"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.futureTimestamp"] = serde_json::json!({
            "originalSourceSHA256": "4490761aa2666c7410a1aedfc0d821b509d597edf007a8069a242ba6557b1f8d",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct future-timestamp rejection assertion moves to Rust. The original seed, record/reset calls, method and runner remain active; expired filtering is crosswalked separately.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["loading the original schema-1 ranking seed rejects future at now plus 61 seconds"],
            "nativeCases": ["ranking::tests::original_loaded_ranking_rejects_future_timestamp"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.expiredTimestamp"] = serde_json::json!({
            "originalSourceSHA256": "3d99229bda839e28e50b1448cc9d587b923f9bedaaaba5e30312c84d15a1b0a8",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the direct expired-timestamp rejection assertion moves to Rust. The original seed, record/reset calls, method and runner remain active; future filtering is crosswalked separately.",
            "nativeSource": "rust-native/src/ranking.rs",
            "originalScenarios": ["loading the original schema-1 ranking seed rejects expired entries"],
            "nativeCases": ["ranking::tests::original_loaded_ranking_rejects_expired_timestamp"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.unchangedRankingReuse"] = serde_json::json!({
            "originalSourceSHA256": "7c9174e4d63490a1a7962b60c8621d3ab51d67568f23413809c54a2564f87ccf",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the unchanged-ranking-reuse assertion moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["1000 identical searches do not rebuild the catalog index or prepare ranking again"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingChangeReprepare"] = serde_json::json!({
            "originalSourceSHA256": "7823f355133f4084ad263b919fb1a970a42d04e284a55eb146a689a4a676cc03",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the successful-ranking result and one-repreparation assertion moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["successful ranking change moves cached.b first and triggers exactly one ranking preparation"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingResetReprepare"] = serde_json::json!({
            "originalSourceSHA256": "c06de8ddfc1f33428f64d4d01c73bf351cf0a86db39a14ca986a4e261fe1f5b1",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the reset-order and one-repreparation assertion moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["reset restores cached.a first and triggers exactly one ranking preparation"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankedDisabledItem"] = serde_json::json!({
            "originalSourceSHA256": "485ba7145885bf21409dc40d99cbf69ecd5861a93e517f4e89482a9532a023b7",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the assertion that a recently ranked item disappears after being disabled moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["rank cached.b first among two identical titles, disable it without resetting ranking, then return only cached.a for identical"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.preparedCatalogReplacement"] = serde_json::json!({
            "originalSourceSHA256": "21fe6af7bffa87228e8101d368f4cad7f2bd3e33880556a4615dada2ca377213",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the exact one-entry prepared-catalog replacement result moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["replace the previous catalog with only cached.replacement and return exactly that ID for identical"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.disabledTypoItem"] = serde_json::json!({
            "originalSourceSHA256": "f1e627323033abfcea7068439bae41a22246f8045ddc6f47788f5132d350618d",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the assertion that disabled Finder is absent from the fnider typo search moves to Rust. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["with Calendar enabled and Finder disabled, searching fnider returns no disabled Finder result"],
            "nativeCases": ["runtime::inline_tests::disabled_catalog_item_is_filtered_from_typo_fallback"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.rankedFilterAfterReload"] = serde_json::json!({
            "originalSourceSHA256": "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the disabled and removed ranked-command filtering assertions move to the real Rust host test; other interaction benchmark scenarios have separate Rust crosswalks.",
            "nativeSource": "rust-native/tests/execution_native_process.rs",
            "originalScenarios": ["a successful launcher choice is ranked and remains in ranking after disabling; config reload omits the command from alias search", "after removing the disabled command and reloading again, alias search still omits it while stale ranking remains"],
            "nativeCases": ["execution_native_process::ranked_command_is_filtered_after_disable_and_removal"],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.traceStateBoundsAndIdentity"] = serde_json::json!({
            "originalSourceSHA256": "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the five pure trace-state assertions move to the existing Rust test; native field/capture scenarios have separate Rust crosswalks.",
            "nativeSource": "rust-native/src/performance_trace.rs",
            "originalScenarios": ["350 search-model samples retain at most 256", "a different field owner cannot complete or cancel pending reconciliation", "superseding input and explicit cancellation count as two cancelled and zero completed reconciliations", "reset prevents an old span from completing a new span", "100 in-flight spans remain bounded at 32 and reset leaves no active spans"],
            "nativeCases": ["performance_trace::tests::original_retention_field_ownership_cancellation_reset_and_overflow_contract"],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.nativeFieldCycles"] = serde_json::json!({
            "originalSourceSHA256": "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the 100 retained-field update/reconcile/capture cycles and their trace privacy assertions map to the existing native Rust route. This is offscreen host evidence, not physical desktop input or key-to-screen latency.",
            "nativeSource": "rust-native/examples/ui_completion.rs",
            "validationSteps": ["interaction-trace-process"],
            "originalScenarios": ["100 query updates across five terms reconcile through the actual retained native field before capture and cause a real host draw each cycle", "the hidden-host trace retains no query, command, or profile-path text"],
            "nativeCases": ["ui_completion::interaction_trace"],
            "contractDifferences": ["Offscreen native host captures do not prove physical desktop input, onscreen presentation, or key-to-screen latency."],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.recencyWriteAndArgumentPrivacy"] = serde_json::json!({
            "originalSourceSHA256": "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Launcher recency write conditions and argument privacy map to the existing Rust process test; no launcher dry-run interface is claimed.",
            "nativeSource": "rust-native/tests/execution_native_process.rs",
            "originalScenarios": ["successful CLI execution, launcher failure, and dry-run leave ranking empty", "successful launcher execution records exactly one expected command", "argument sentinel is not retained in the ranking file"],
            "nativeCases": ["execution_native_process::only_real_launcher_success_writes_recency_without_argument_text"],
            "contractDifferences": ["The old Swift helper invokes dry-run through AppState.execute for a launcher source. Current Rust uses the public CLI dry-run route; the launcher recording API has no dry-run context or launcher dry-run path."],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.closedFieldCancellation"] = serde_json::json!({
            "originalSourceSHA256": "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "A pending interval on a closed native field is cancelled without a successful reconciliation or query text in the trace.",
            "nativeSource": "rust-native/examples/ui_completion.rs",
            "validationSteps": ["interaction-trace-process"],
            "originalScenarios": ["tearing down a retained Swift field with pending input leaves no active trace interval", "the Rust closed-palette path cancels a late query interval and excludes its sentinel from the trace"],
            "nativeCases": ["ui_completion::interaction_trace"],
            "contractDifferences": ["SwiftUI dismantleNSView has no literal Rust API equivalent; the Rust crosswalk proves owner/close cancellation in the native host, not that callback."],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.enabledTypoCliParity"] = serde_json::json!({
            "originalSourceSHA256": "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the Calendar typo-fallback assertion that compares active palette results with the real local CLI search response moves to the Rust runtime test; disabled-item exclusion remains asserted in the same case.",
            "nativeSource": "rust-native/src/runtime_system.rs",
            "originalScenarios": ["with Calendar enabled and Finder disabled, the calender typo returns exactly typo.enabled in both active palette and CLI search"],
            "nativeCases": ["runtime::system::shortcut_budget_tests::typo_fallback_matches_palette_and_local_cli_without_disabled_items"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoPaletteOrder"] = serde_json::json!({
            "originalSourceSHA256": "69997e74e26bc8029be2630175124c7a7dcb2276767024d61534c1e34f17a6ef",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the Swift palette-result ordering for a direct title typo over a higher-recency weak subsequence moves to the Rust core test. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["with Find Reminders ranked higher, query fnider orders the direct Finder title before the weak subsequence distractor"],
            "nativeCases": ["tests::direct_title_typo_outranks_weak_subsequence_with_higher_recency"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoCliOrder"] = serde_json::json!({
            "originalSourceSHA256": "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the CLI reply assertion that preserves direct title typo order over a higher-recency weak subsequence moves to the Rust local control-socket runtime test; palette ordering remains bound to its existing core test.",
            "nativeSource": "rust-native/src/runtime_system.rs",
            "originalScenarios": ["with Find Reminders ranked higher, the fnider CLI reply orders direct Finder before the weak subsequence distractor"],
            "nativeCases": ["runtime::system::shortcut_budget_tests::cli_direct_title_typo_outranks_higher_recency_subsequence"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.appearancePreservesCatalog"] = serde_json::json!({
            "originalSourceSHA256": "a3c7038a6a68b8ccbc5cebc38ef5dbfae358eb3079e2424c79337906c8704d22",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the assertion that changing the appearance theme does not rebuild or reevaluate the command catalog moves to the existing Rust runtime test. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["changing theme to graphite and syncing leaves catalog preparation work, results revision and selection unchanged; a following title change does rebuild the catalog"],
            "nativeCases": ["runtime::inline_tests::prepared_palette_theme_does_not_reindex_reprepare_or_reevaluate"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.usageRankingRefreshes"] = serde_json::json!({
            "originalSourceSHA256": "4ed29d77d3c72b1ee1e7489ddad15bfc30f7bb2bb9864b91650f6ae4a6d0be0b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the empty-query usage-ranking result assertion moves to the existing Rust runtime test. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["after usage count increases for one catalog item, the empty query returns that item first while non-empty catalog order remains unchanged"],
            "nativeCases": ["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.cachedReads"] = serde_json::json!({
            "originalSourceSHA256": "5e1d2ddaffc976900b0c72b60fee701b3484dbbfb7b61a013acb23c522d441f5",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The complete 1,000-read prepared-results scenario moves to the existing Rust Session test, including an explicit catalog-search work counter. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust-native/src/session.rs",
            "originalScenarios": ["after preparing a non-empty result set, 1,000 result reads retain identical rows without reevaluating the search or changing query, selection or results revision"],
            "nativeCases": ["session::tests::repeated_result_reads_preserve_prepared_rows_and_session_state"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.indexDispatchABIParity"] = serde_json::json!({
            "originalSourceSHA256": "ce5f4ed8c8cac22da5cfca994c948725930cd3a63946b29a9d266c4643f2baa7",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the duplicate 256-command, ten-query SearchIndex versus AppState dispatch parity loop is retired. The aggregate --check-search Rust crosswalk now owns acceptance; the Swift search helper and CLI route are retired; only its synthetic benchmark fixture remains.",
            "nativeSource": "rust/src/lib.rs",
            "originalScenarios": ["for the 256-command fixture and all ten queries including Unicode and embedded NUL, compare full SearchIndex and dispatch outputs and verify ranked C ABI count and order"],
            "nativeCases": ["tests::original_256_command_ten_query_index_dispatch_and_abi_parity"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.cliSearchLeavesPaletteUntouched"] = serde_json::json!({
            "originalSourceSHA256": "c2c2962d6d5da6e9d2f17cd54eae21345ad402be9975f9c398b84141cc682920",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The active Rust CLI socket route preserves the current query, complete rows, selection and results revision while returning one successful search response. The legacy Swift Observation callback assertion has no Rust primitive counterpart.",
            "nativeSource": "rust-native/src/runtime_system.rs",
            "originalScenarios": ["while the optional Safari provider is busy, search Owned Local over the real control socket; preserve the active Owned palette query, full rows, selection and results revision; return one successful local match and no second response"],
            "retiredAssertions": ["Swift withObservationTracking callback-count check is a legacy wrapper detail; Rust invalidation is checked at query, rows, selection and results_revision"],
            "nativeCases": ["runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.discoveredAppsRefresh"] = serde_json::json!({
            "originalSourceSHA256": "d4af372db2da26489feaf66123891ad1a1930d33b726f72fa30d6f7bdf0b2dc5",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the assertion that a newly discovered application refreshes the already-active palette query moves to the Rust runtime test. The application fixture remains for the following config reload/deduplication scenario.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["with New App queried and no matching saved command, increment applications_revision, synchronize the window, and return exactly discovered.test in the active palette"],
            "nativeCases": ["runtime::inline_tests::discovered_application_refreshes_an_existing_palette_query"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.reloadDeduplicatesDiscoveredApp"] = serde_json::json!({
            "originalSourceSHA256": "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the active-query configuration reload assertion that a saved app replaces its discovered duplicate moves to the Rust runtime test.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["with New App queried and discovered.test visible, write replacement with the same app target to configuration, reload from disk, preserve the query, and return exactly replacement"],
            "nativeCases": ["runtime::inline_tests::configuration_reload_deduplicates_discovered_app_in_active_palette"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.selectiveResultObservation"] = serde_json::json!({
            "originalSourceSHA256": "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the palette-result observation assertions move to Rust presentation-boundary checks. Full View revisions may change for visible configuration errors; query rows and results revision stay stable, and explicit selection is published independently.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["clipboard pause and configuration error preserve active query results; selecting another result updates selection without rebuilding the catalog"],
            "retiredAssertions": ["Swift withObservationTracking callback counts are wrapper implementation details; Rust View.revision can advance when the user-visible Config message changes", "expansionStatus has no independent mutable Rust field; keyboard status is owned by the keyboard client"],
            "nativeCases": ["runtime::inline_tests::palette_status_changes_preserve_rows_and_selection_updates_publish"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SearchChecks.swift::checkSearch.settingsSurfaceIsolation"] = serde_json::json!({
            "originalSourceSHA256": "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the final Settings-observation assertion moves to a Rust-owned Settings surface comparison: palette query/clipboard navigation leaves the surface intact, and clearing last_error publishes the recovered message without changing config revision or path.",
            "nativeSource": "rust-native/src/runtime_private_tests.rs",
            "originalScenarios": ["after establishing a settings error, palette query and clipboard selection do not change the Settings surface; clearing the error publishes recovery"],
            "retiredAssertions": ["Swift withObservationTracking callback counts are wrapper details; Rust validates the complete value-based Settings surface and the actual View revision when visible feedback clears", "The original check reads but does not mutate configDirectory; relocation remains covered by its existing integration test"],
            "nativeCases": ["runtime::inline_tests::settings_presentation_ignores_palette_navigation_and_clears_configuration_error"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/UtilityChecks.swift::checkUtilities"] = serde_json::json!({
            "originalSourceSHA256": "a62950d4e891b79c3d311da4db2ec831bea1f4eca0744709a25be63db05bacc2",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The Utilities acceptance helper and its CLI route are retired. Its original missing-text, history, password and username, malformed-Keychain, import and privacy assertions remain in the focused Rust scenario; real profile reopening and 0600 permissions stay in the existing storage test, with frozen Swift results retained separately.",
            "nativeSources": ["rust-native/src/utilities.rs", "rust-native/src/utility_storage.rs", "rust-native/tests/utility_oracle.rs"],
            "originalScenarios": ["missing active text reports Set Text", "text history deduplicates and selecting an entry pastes the active text", "reopened profile retains active text and is private on disk", "password generation is deterministic and secret-marked while existing Keychain history is preserved with exactly one write", "password values are absent from command payloads and profile storage", "old password history copies as concealed text", "username generation is deterministic and searchable", "malformed Keychain data remains intact without another write", "utility import merges once, replay adds zero, and active native text is retained"],
            "nativeCases": ["utilities::tests::original_utility_acceptance_scenarios_remain_in_rust", "utility_storage::tests::real_profile_reopen_private_mode_external_edit_and_no_password_on_disk", "utility_oracle::frozen_swift_utilities_and_codable_match_rust_on_owned_cases"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/KeyboardChecks.swift::checkKeyboard"] = serde_json::json!({
            "originalSourceSHA256": "5cbaf9f3e805ccc44df691fafc0fdf9f2681d6854d6381e2e818115a2e4dac53",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The deterministic physical-state and flag assertions moved to Rust. The helper records model outputs rather than OS events; its hardware-acceptance status, Swift CLI/JSON wrapper and generic callback diagnostic text retire with it and are not presented as hardware evidence.",
            "nativeSource": "rust-native/src/keyboard_state.rs",
            "originalScenarios": ["duplicate Caps reports plus a letter produce balanced Hyper down/up edges and no held usages", "isolated left Control and Option produce their tap usages", "Control/letter, Option/Shift, a one-second modifier and right-Command/Control do not produce solo taps", "cancelTap prevents the pending Option tap", "releasing one keyboard leaves another keyboard's Caps held", "removing the second keyboard releases Hyper", "reset clears Caps and held usages", "a stray modifier release after reset stays unheld", "ordinary right-Command flags remain unchanged", "Hyper rewriting preserves the unowned device bit and removes Caps", "both generated tags preserve Command/Control and empty flags"],
            "retiredAssertions": ["returned hardwareAcceptance status is a static requirement, not a hardware test", "test-recorder strings are not posted OS events; event tap, HID manager, Carbon registration, permissions and desktop input are not exercised"],
            "nativeCases": ["keyboard_state::tests::original_keyboard_acceptance_scenarios_remain_in_rust"]
        });
        inventory["retiredSwiftChecks"]["Tests/LauncherCoreTests/CoreTests.swift::testWindowReadsRetryOnlyTemporaryFailures"] = serde_json::json!({
            "originalSourceSHA256": "26a71c8720522dabdccadb3972b65f7546d9b9979f20af20f22b58c15011ce5c",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the bounded AX attribute retry test and its runner call are retired; historical CoreTests mappings remain in provenance; the current RuycastApp compile/link smoke is separate. The existing production AX boundary keeps its deadline, cancellation, timeout and owned-CF guards.",
            "nativeSource": "rust-native/src/system_search/native.rs",
            "originalScenarios": ["two temporary failures then success: three reads and 20/40 ms waits", "temporary failures during WindowPlacement readback: four reads and 20/40/80 ms waits", "persistent -25204 fails after four reads", "-25205, -25212, -25202 and status zero without a value fail immediately", "immediate success reads once without waiting"],
            "nativeCases": ["system_search::native::tests::placement_attribute_preserves_original_retry_scenarios", "system_search::native::tests::placement_attribute_retries_within_verified_window_readback", "system_search::native::tests::placement_attribute_preserves_deadline_and_cancellation_guards"]
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_private_storage.py"] = serde_json::json!({
            "scope": "all twelve private-storage scenarios are covered by the shared Rust writer, actual Rust CLI, owned Rust headless runtime and Rust fcntl fault fixture; the Python and forwarding consumers are retired",
            "originalMethods": [
                "test_private_throughout_write_with_typical_umask",
                "test_private_throughout_write_with_permissive_umask",
                "test_private_throughout_write_with_inherited_read_acl",
                "test_owner_permissions_with_restrictive_umask",
                "test_unsearchable_creation_mask_fails_before_payload_and_preserves_old_file",
                "test_cli_init_with_inherited_read_acl",
                "test_cli_edit_with_inherited_read_acl",
                "test_profile_acl_removed_and_legacy_alias_preserved",
                "test_cli_failed_private_write_preserves_old_config",
                "test_relocation_directory_fsync_failure_rolls_back_own_publication",
                "test_relocation_directory_fsync_failure_preserves_inplace_edit",
                "test_relocation_directory_fsync_failure_preserves_replacement"
            ],
            "nativeTargets": {
                "rust-native --lib": [
                    "private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies",
                    "private_storage::guarded_publication_tests::unsearchable_umask_refuses_before_payload_in_an_isolated_process"
                ],
                "rust-native --test startup_config_repair": [
                    "cli_init_with_inherited_read_acl_publishes_private_config",
                    "cli_edit_with_inherited_read_acl_keeps_parent_and_publishes_private_config",
                    "cli_failed_private_write_preserves_old_config_under_unsearchable_umask"
                ],
                "rust-native --test runtime_process": [
                    "headless_start_removes_profile_acl_through_legacy_alias"
                ],
                "rust-native --test foundation_process": [
                    "destination_fsync_failure_cleans_only_owned_copy",
                    "destination_fsync_failure_preserves_in_place_edit",
                    "destination_replacement_is_preserved_on_failed_move"
                ]
            },
            "nativeCaseByMethod": {
                "test_private_throughout_write_with_typical_umask": "rust-native --lib -> private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies",
                "test_private_throughout_write_with_permissive_umask": "rust-native --lib -> private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies",
                "test_private_throughout_write_with_inherited_read_acl": "rust-native --lib -> private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies",
                "test_owner_permissions_with_restrictive_umask": "rust-native --lib -> private_storage::guarded_publication_tests::nonempty_payloads_remain_private_before_publication_under_four_creation_policies",
                "test_unsearchable_creation_mask_fails_before_payload_and_preserves_old_file": "rust-native --lib -> private_storage::guarded_publication_tests::unsearchable_umask_refuses_before_payload_in_an_isolated_process",
                "test_cli_init_with_inherited_read_acl": "rust-native --test startup_config_repair -> cli_init_with_inherited_read_acl_publishes_private_config",
                "test_cli_edit_with_inherited_read_acl": "rust-native --test startup_config_repair -> cli_edit_with_inherited_read_acl_keeps_parent_and_publishes_private_config",
                "test_profile_acl_removed_and_legacy_alias_preserved": "rust-native --test runtime_process -> headless_start_removes_profile_acl_through_legacy_alias",
                "test_cli_failed_private_write_preserves_old_config": "rust-native --test startup_config_repair -> cli_failed_private_write_preserves_old_config_under_unsearchable_umask",
                "test_relocation_directory_fsync_failure_rolls_back_own_publication": "rust-native --test foundation_process -> destination_fsync_failure_cleans_only_owned_copy",
                "test_relocation_directory_fsync_failure_preserves_inplace_edit": "rust-native --test foundation_process -> destination_fsync_failure_preserves_in_place_edit",
                "test_relocation_directory_fsync_failure_preserves_replacement": "rust-native --test foundation_process -> destination_replacement_is_preserved_on_failed_move"
            },
            "retiredConsumers": [
                "Tests/test_private_storage.py",
                "Tests/LauncherCoreTests/CoreTests.swift::--private-storage-probe",
                "Tests/LauncherCoreTests/PrivateFileChecks.swift::runPrivateStorageWriteProbe"
            ],
            "remainingPythonCases": 0
        });
        inventory["rustRootTestCrosswalk"]["Tests/test_socket_recovery.py"] = serde_json::json!({
            "scope": "all eight original socket recovery scenarios run against the actual Rust headless host in an owned profile; Python consumer retired",
            "originalMethods": [
                "test_regular_endpoint_is_preserved_then_retry_recovers",
                "test_linked_endpoint_and_target_are_preserved",
                "test_live_endpoint_without_instance_lock_is_preserved",
                "test_stale_endpoint_recovers_and_graceful_quit_removes_it",
                "test_graceful_stop_preserves_replacement_regular_file",
                "test_graceful_stop_preserves_replacement_socket",
                "test_linked_instance_lock_is_preserved_then_retry_recovers",
                "test_fifo_instance_lock_is_rejected_without_blocking"
            ],
            "nativeTarget": "runtime_process",
            "nativeCases": [
                "socket_recovery_regular_endpoint_is_preserved_then_retry_recovers",
                "socket_recovery_linked_endpoint_and_target_are_preserved",
                "socket_recovery_live_endpoint_without_instance_lock_is_preserved",
                "socket_recovery_stale_endpoint_recovers_and_graceful_quit_removes_it",
                "socket_recovery_graceful_stop_preserves_replacement_regular_file",
                "socket_recovery_graceful_stop_preserves_replacement_socket",
                "socket_recovery_linked_instance_lock_is_preserved_then_retry_recovers",
                "socket_recovery_fifo_instance_lock_is_rejected_without_blocking"
            ],
            "remainingPythonCases": 0
        });
    }
    if kind == "check" {
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/FeedbackChecks.swift::checkFeedback"] = serde_json::json!({
            "originalSourceSHA256":FEEDBACK_ORACLE_SHA256,
            "status":"source-qualified-rust-case-crosswalk; Swift helper retired",
            "scope":"Only the Swift feedback-check helper and --check-feedback entrypoint are retired. Product creation, capture, bindings and clipboard code remain active. The historical SETTINGS selector, 23 cases, nine steps and Actions/Interactions provenance remain preserved.",
            "productionAccepted":false,
            "contractDifferences":[
                "Native captures prove Rust controls and themes, not pixel-identical SwiftUI rendering or identical capture filenames/dimensions.",
                "Quick capture uses owned fake runner targets and marker files; it does not write to a real vault or invoke a personal provider.",
                "Real Rust native Return/Command-Return, IME and busy-runner routes replace the Swift callback-interception counts; literal Swift CaptureInput class identity is not claimed.",
                "Creation actions are reached through the Rust contextual menu and Settings toolbar; fresh draft identities and unchanged configuration prove independence from the selected command.",
                "Owned hidden windows, local NSEvents and private named boards do not prove physical input, general-clipboard behavior or complete product acceptance."
            ],
            "assertionCrosswalk":{
                "Closed creation opens both kinds without repeat presentation":["ui_completion::closed_creation"],
                "Both creation entries preserve trimmed named-board prefill and original board/config bytes":["ui_completion::feedback_actions"],
                "Later board mutation preserves edited draft; cancel is private and explicit save persists target":["ui_completion::feedback_actions"],
                "Non-URL defaults and empty/nontext URL fallback":["ui_completion::feedback_actions"],
                "Quick-note/task native focus, exact multiline text and Shift-Return":["utility_process::quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries","ui_completion::polish_original"],
                "Return/Command-Return submit, IME refuses, and busy runner executes once":["utility_process::quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries"],
                "Selected item actions and alias/hotkey editor identity":["actions_process::actions_keyboard_badges_and_escape_keeps_palette","input_process::alias_and_hotkey_saves_resume_same_input"],
                "Binding save preserves external title, target and arguments":["actions::tests::binding_save_preserves_external_execution_definition"],
                "Alias resolution, saved disable and search exclusion":["actions_process::alias_save_clear_and_cli_resolution","actions_process::remove_confirmation_cancel_then_commit_and_disable"],
                "Creation independent of selected row and available with no results":["ui_completion::feedback_actions","creation_process::empty_results_keep_all_three_creation_actions_available"],
                "Light/dark native captures and no displayed fixture window":["ui_completion::feedback_actions","ui_completion::polish_original","utility_process::quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries"]
            }
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/EditCommandChecks.swift::checkNativeEditCommands"] = serde_json::json!({
            "originalSourceSHA256":EDIT_COMMAND_ORACLE_SHA256,
            "status":"source-qualified-rust-case-crosswalk; Swift helper retired",
            "scope":"Only the Swift edit-check helper, its private test textview and --check-edit-commands entrypoint are retired. Product editing, capture and recorder code remain active. Historical SETTINGS selector/path/hash and Rust native steps remain preserved.",
            "productionAccepted":false,
            "contractDifferences":[
                "Rust observes the mounted production CommandText, not the exact pre-attachment SwiftUI constructor return instant.",
                "Native undo/redo uses owned Rust/NSTextView fixtures with different text; production-input undo is also covered separately. Literal Swift CaptureInput class parity is not claimed.",
                "The real recorder takes focus before capturing keys; simultaneous recorder/text focus is not claimed.",
                "Focused Cmd+K and item binding prove the actual handled UI route and target, not the old Swift stub callback counts.",
                "Owned hidden windows, local NSEvents and private named boards do not prove physical keyboard, visible desktop or complete product parity."
            ],
            "assertionCrosswalk":{
                "Panel text focus, exact single paste, full UTF16 Select All, Copy and Cut":["native_edit::check_owned_text_roundtrip"],
                "Caps Lock paste and modified-paste refusal":["native_edit::check_modified_paste"],
                "Input undo policy, storage/container presence and actual input focus":["input_process::open_and_cancel_leave_configuration_untouched"],
                "Native typing, Undo and Redo":["native_edit::check","input_process::failure_preserves_text_focus_typing_and_undo"],
                "Panel NSTextField field-editor Select All":["native_edit::check_panel_field_select_all"],
                "Recorder captures Cmd+V, Cmd+A and Cmd+Z":["actions_process::hotkey_sides_hyper_delete_and_repeat"],
                "One standard Edit menu with the original selectors/keys and Redo":["native_edit::check_menu"],
                "Ordinary-window menu exact paste, UTF16 Select All, Copy and Cut":["native_edit::check_menu"],
                "Ordinary-window Undo/Redo, field editor and disabled editing without responder":["native_edit::check_menu"],
                "Cmd+K and item binding while a text responder has focus":["actions_process::actions_keyboard_badges_and_escape_keeps_palette"],
                "No displayed window or physical input":["native_edit::check_owned_text_roundtrip","native_edit::check_panel_field_select_all","native_edit::check_menu","actions_process::actions_keyboard_badges_and_escape_keeps_palette"]
            }
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/ScriptChoiceChecks.swift::checkScriptChoicesUI"] = serde_json::json!({
            "originalSourceSHA256":"7d98578478074c4c8c25d39ffc0dda8048debadd62f643ef1265700d449336cc",
            "status":"source-qualified-rust-case-crosswalk",
            "scope":"The Swift UI test helper, embedded Python fixtures and --check-script-choices entrypoint are retired. Product script-choice code remains active. This is historical assertion provenance, not whole-product or pixel-identical Swift UI acceptance.",
            "productionAccepted":false,
            "contractDifferences":[
                "Native captures prove the Rust views and labels, not pixel-identical Swift rendering.",
                "Generic Rust empty/error/render fixtures use different labels from the old mail fixture.",
                "A real held Rust provider replaces the manually toggled Swift busy screenshot; no Python provider remains in this helper."
            ],
            "assertionCrosswalk":{
                "Initial choices, filtering, Down, empty-search no-write, pinned single submit and Escape":["rich_process::original_project_part_navigation_and_single_submit"],
                "Stale refusal and reload reset":["rich_process::stale_project_part_refusal_recovers_with_command_r_reload"],
                "Wide and narrow labels/captures":["rich_process::original_project_part_choices_capture_at_760_and_480_pixels"],
                "Loading capture":["rich_process::original_project_part_navigation_and_single_submit"],
                "Light/dark and unmatched-query rendering":["rich_process::light_dark_empty_and_long_labels_render"],
                "Empty provider choices":["rich_process::empty_choices_and_blank_labels_keep_accessibility"],
                "Failure report retained and not accepted as success":["rich_process::failure_keeps_query_selection_and_readable_detail","rich_process::failure_can_add_first_readable_detail_without_replacing_choices","rich_process::failure_report_back_retains_ordinary_input_draft","script_result::tests::process_failure_wins_and_failure_detail_survives_without_accepting_choices"],
                "Generic provider choices without personal_bridge.py":["rich_process::choices_filter_literal_selection_and_ok_only_closes"]
            }
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/SystemChecks.swift::checkSystemServices"] = serde_json::json!({
            "originalSourceSHA256": SYSTEM_CHECKS_ORACLE_SHA256,
            "originalFeedbackSourceSHA256": "1f91b690d51f82c2c2d6ea653c0f0ec4c06747dc21603472302c30c098cfaa04",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The isolated helper, --check-system entry point and nested Feedback call/result flag are retired. SystemServices.swift product behavior remains active. This is historical assertion provenance, not a current Swift helper or whole-product acceptance.",
            "productionAccepted": false,
            "contractDifferences": {
                "Menu identity was unstable.": "The helper required a stable token for repeated registration. Rust deliberately creates a fresh opaque token per scan; each retained token stays bound to its captured leaf and refuses a replaced leaf.",
                "Stale results or background callbacks reached the consumer.": "Rust adopts completed results on its runtime owner after a worker wake, rechecking current authority before publication. It does not promise a literal AppKit-main-thread callback. Empty file results and cancellation are checked separately."
            },
            "assertionCrosswalk": {
                "Kill still requires confirmation.": ["remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes"],
                "Direct Kill did not terminate the verified fixture.": ["processes::tests::owned_table_sort_and_immutable_kill_identity"],
                "A reused process ID was terminated.": ["processes::tests::owned_table_sort_and_immutable_kill_identity"],
                "Confirmed force quit did not use the verified process.": ["processes::tests::owned_table_sort_and_immutable_kill_identity"],
                "The launcher could terminate itself.": ["processes::tests::matching_self_and_foreign_uid_identities_refuse_before_termination"],
                "Another user's process could be terminated.": ["processes::tests::matching_self_and_foreign_uid_identities_refuse_before_termination"],
                "Menu identity was unstable.": ["system_search::tests::menu_snapshot_tokens_never_redirect_to_replaced_leaf"],
                "Menu execution lost its original app.": ["system_search::tests::menu_original_identity_must_match_the_complete_captured_identity"],
                "Menu permission was bypassed.": ["system_search::tests::menu_post_activation_item_ancestor_identity_and_trust_are_revalidated"],
                "Menu action was not routed to its original app.": ["system_search::tests::menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed"],
                "Window enumeration raised a window.": ["system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact"],
                "Window action did not raise and activate.": ["system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact"],
                "A token could be used for another action.": ["system_search::providers::gate_tests::cross_kind_and_missing_opaque_tokens_refuse_before_native_reads_or_effects"],
                "An expired token was accepted.": ["system_search::providers::gate_tests::cross_kind_and_missing_opaque_tokens_refuse_before_native_reads_or_effects"],
                "A system action ran during search.": ["system_search::tests::empty_files_query_returns_successful_empty_results_without_effects"],
                "Monitor action did not execute.": ["remaining_macos_process::native_monitor_delegate_pause_resume_close_and_reopen_use_actual_callback_route"],
                "Lock action did not execute.": ["system_actions::tests::complete_owned_keyboard_sdk_pairs_and_tags_lock_shortcut", "system_actions::tests::denied_and_failed_construction_never_post"],
                "File search did not escape the Spotlight literal.": ["file_search::tests::literal_predicate_and_normalization"],
                "Stale results or background callbacks reached the consumer.": ["runtime::system::shortcut_budget_tests::system_search_worker_completion_requires_current_owner_adoption_before_publication", "system_search::tests::main_service_queued_effect_is_revoked_before_native_adapter", "system_search::tests::empty_files_query_returns_successful_empty_results_without_effects"]
            }
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.staleLoadRetention"] = serde_json::json!({
            "originalSourceSHA256": "80a8968e86e157a262368fe98484335d7022e6fc61591edb6b4be5e7eabedecd",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the stale-event pruning-on-open fixture and assertion are retired. The remaining analytics helper, dashboard rendering, command execution and other checks remain active.",
            "nativeSource": "rust-native/src/analytics.rs",
            "originalScenarios": ["opening a valid analytics document removes a runtime event older than the configured seven-day retention"],
            "nativeCases": ["analytics::tests::opening_prunes_events_outside_configured_retention"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.managementRouting"] = serde_json::json!({
            "originalSourceSHA256": "926b61901b45c6623d0c68a60379ec3926f71275aadae47ce5978fb6b36ad65b",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only opening Analytics through management routing and confirming the main launcher panel stays hidden are retired. Remaining analytics fixture, command execution and rendering work remains active.",
            "nativeSource": "rust-native/tests/native_dashboard_dialog_process.rs",
            "originalScenarios": ["opening analytics routes to the management dashboard and leaves the main palette hidden"],
            "nativeCases": ["native_dashboard_dialog_process::management_routing_controls_custom_retention_and_reset"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.concurrentWorkload"] = serde_json::json!({
            "originalSourceSHA256": "64fca861ac1a01105517d4e4c16546deccc9bf054f16bf29b1e62af34f2c62c9",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The unasserted eight-worker Swift workload is removed. Rust checks the same workload and exact counters; this does not claim to test Swift DispatchQueue thread safety.",
            "nativeSource": "rust-native/src/analytics.rs",
            "originalScenarios": ["eight workers each record 125 attempts, completions and timings with exact aggregate and per-command counts"],
            "nativeCases": ["analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.detachedStoreFixture"] = serde_json::json!({
            "originalSourceSHA256": "24e8731e4bf1da3622e28ef982f3ad085b374a5135bb0ac3fa16b6348c5671ab",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The detached local Swift store had no remaining assertions or consumer and is removed. Its historical constant result keys are retained for output compatibility and are not evidence. Existing Rust analytics tests cover the asserted storage, import and pause behavior.",
            "nativeSource": "rust-native/src/analytics.rs",
            "originalScenarios": ["import a legacy successful count once", "complete an in-flight command while collection is paused", "omit unknown diagnostic and timing names from persisted analytics", "flush the writer before later execution and rendering fixtures"],
            "nativeCases": ["analytics::tests::completion_survives_pause_but_reset_retires_old_ticket_and_private_export", "analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.dryRunFixture"] = serde_json::json!({
            "originalSourceSHA256": "189f08d3eda7409b3f5487c3c7d2cb84d66c56b7c1e753e825511b951f4ad25f",
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "The Swift dry-run invocation and helper parameter are removed. Existing Rust process tests verify that dry-runs add no analytics records; this does not claim Swift callback parity.",
            "nativeSource": "rust-native/tests/startup_process.rs",
            "originalScenarios": ["a CLI dry-run returns without execution or analytics side effects"],
            "nativeCases": ["startup_process::analytics_same_command_tracks_actual_cli_launcher_and_owned_carbon_callback", "startup_process::analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::renderWebsiteIcons"] = serde_json::json!({
            "originalSourceSHA256": "e73210b69d30795488372866e45036770b3d6e5355c11b48d31a051a857ed70e",
            "status": "source-retired; unconsumed development preview",
            "retiredOutputs": ["sites-dark.png", "sites-light.png"],
            "scope": "The generated website-icon screenshots and their command have no tracked consumer; the Rust WebsiteIcons product source and its privacy/cache behavior remain active."
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::renderVisualAssets"] = serde_json::json!({
            "originalSourceSHA256": "e0b3a9b53aa1041019cc10d7214e42727ed0e2f79162c6144afc35be2725ba78",
            "status": "source-retired; unconsumed development preview",
            "retiredOutputs": ["shortcuts-dark.png", "shortcuts-light.png"],
            "scope": "No tracked source, script, documentation or asset references these generated shortcut previews; no Rust renderer replaces this unused output contract."
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/CommandGlyph.swift::CommandIconSheet"] = serde_json::json!({
            "originalSourceSHA256": "f9b6b16cf6e3ee41c58835ba71cdea0270a4adb7d528498e6865e2dfc8eb9feb",
            "status": "source-retired; unconsumed development preview",
            "scope": "The --render-icons developer route and its standalone sheet have no tracked consumers. The shared CommandGlyph remains for palette validation and WebsiteIcons."
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::checkVisualAssets"] = serde_json::json!({
            "originalSourceSHA256": "4bd5994c6cc34a504e3f1576a0a21ca903636ed8293986ccde561a4c3b7304f0",
            "status": "source-qualified-rust-case-crosswalk; Swift check route retired",
            "scope": "The visual-assets check and both unconsumed SwiftUI screenshot generators are retired. Product WebsiteIcons behavior remains in the dedicated app module and Rust implementation.",
            "nativeCases": ["presentation::icon_tests::original_compact_keycaps_and_sided_accessibility_labels", "website_icons::tests::origin_and_candidate_privacy", "website_icons::tests::actual_transport_stream_bounds_status_cookies_and_redirects", "website_icons::tests::four_workers_cache_publication_and_cancel_join", "website_icons::tests::two_thousand_origins_bound_admission_and_deduplicate_while_four_workers_wait", "website_icons::tests::cache_special_files_and_close_are_bounded", "website_icons::tests::active_download_close_cancels_and_joins_without_cache_publication", "native_rows::check_cache", "native_process::loaded_config_alias_unicode_disabled_and_native_labels"]
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::file"] = serde_json::json!({
            "originalSourceSHA256": "e86146a3b3d15a10c652d34d9158b93efce730157b175407f7a0852bcce93f55",
            "status": "source-retired; development-only SwiftUI PNG contract retired",
            "nativeCases": ["analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention", "analytics::tests::opening_prunes_events_outside_configured_retention", "native_dashboard_dialog_process::management_routing_controls_custom_retention_and_reset", "native_dashboard_dialog_process::actual_dashboard_renders_themes_widths_and_preserves_scroll", "native_dashboard_dialog_process::populated_paused_errors_confirmation_roles_values_and_local_key_loop", "startup_process::analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch"]
        });
        inventory["retiredSwiftChecks"]["--check-search"] = search_crosswalk(&inventory["retiredSwiftChecks"])?;
    }
    if kind == "check" {
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/ClipboardChecks.swift::checkClipboardStore"] = serde_json::json!({
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Only the obsolete clipboard checker and its --check-clipboard entrypoint are retired. ClipboardStore remains active for AppState and preview runtime callers. Existing Rust cases cover the helper's distinct assertion groups; this is not full product acceptance.",
            "originalSourceSHA256": "bda0fc08d8b64c4598538078362da8c811617c99c223f3fa26caf62d2e33a6ae",
            "originalScenarios": [
                "named-board text capture, copy, and exact readback",
                "maxTextBytes skips longer captured text",
                "file URL capture/copy/readback without copying a missing source file",
                "image capture and exact dimensions/pixel/alpha readback",
                "private asset permissions and removal of inherited readable ACLs",
                "image size limits and invalid image rejection",
                "non-file URL and unsafe image path rejection",
                "owned asset pruning preserves valid keep entries, external files, unrelated files, and symlinks"
            ],
            "nativeCases": [
                "clipboard_history_host_process::test_cli_text_copy_roundtrips_to_owned_named_board",
                "clipboard_history_host_process::test_native_capture_skips_text_above_max_text_bytes_without_changing_history",
                "clipboard_history_host_process::test_rich_copy_routes_and_invalid_data_do_not_clear_destination",
                "clipboard_history_host_process::test_capture_text_files_image_and_private_own_activation_filters",
                "clipboard_history::tests::image_roundtrip_bounds_acl_and_owned_pruning",
                "clipboard_history::tests::file_copy_payload_rejects_non_file_urls",
                "clipboard_history::tests::image_copy_payload_rejects_unsafe_asset_paths",
                "clipboard_history::tests::external_edits_are_the_transaction_base_and_links_never_become_assets"
            ],
            "qualifiedCaseSourceBindings": {
                "rust-native/tests/clipboard_history_host_process.rs": "8cf10e2943cf3f48c6a2a7edebbf1721ad305a34116e9fe357ae66b93eb7a0f8",
                "rust-native/src/clipboard_history.rs": "59bc0cc716ba16dea07bbd20d1f8951ed3b4580c7cc7ea9a56116156b9c17556"
            },
            "retiredAssertions": [
                "The Swift JSON/CLI wrapper shape is retired with this helper; no product CLI compatibility claim follows.",
                "Native checker compilation is smoke coverage only; production clipboard behavior is not certified by helper retirement."
            ],
            "productionAccepted": false
        });
        inventory["retiredSwiftChecks"]["Sources/RuycastApp/ClipboardPreviewChecks.swift::checkClipboardPreviewCancellation"] = serde_json::json!({
            "status": "source-qualified-rust-case-crosswalk",
            "scope": "Retire only the development preview check helper and its result field. ClipboardPreview.swift and the production clipboard preview runtime remain in place; this does not claim complete clipboard product parity.",
            "originalSourceSHA256": "369905c6a65da6ef8995b570466b0d1d3fcd54d3dfeda96d21b940128f3292a6",
            "originalScenarios": [
                "a stale successful same-ID decode cannot overwrite the reopened preview",
                "the latest successful same-ID preview publishes",
                "same-metadata selection with a different clip ID replaces the image owner",
                "deselection rejects a decoded but unpublished preview and releases its image",
                "a failed reopen clears a previously published image and surfaces the error"
            ],
            "nativeCases": [
                "clipboard_preview::tests::stale_success_cannot_replace_same_id_reopened_preview",
                "clipboard_preview::tests::equal_metadata_different_id_replaces_preview_owner_and_releases_old_image",
                "clipboard_preview::tests::decoded_preview_cancelled_before_poll_is_not_published_and_released",
                "clipboard_preview::tests::failed_reopen_releases_published_preview_and_clears_on_deselect"
            ],
            "qualifiedCaseSourceBindings": {
                "rust-native/src/clipboard_preview.rs": "1406185c311781ff5be404caaa0d92ac76746a68f44fb54e3eedff5419241738"
            },
            "productionAccepted": false
        });
    }
    Ok(inventory)
}
pub(super) fn require_complete(kind: &str) -> Result<()> {
    let inventory = inventory(kind)?;
    if inventory["unmappedGateIDs"]
        .as_array()
        .ok_or("Missing original gate inventory")?
        .is_empty()
    {
        return Ok(());
    }
    Err(format!("Complete {kind} validation is not yet qualified: {inventory}. Native-only checks cannot replace these original gates."))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_system_helper_has_no_callers_and_preserves_historical_assertions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/SystemChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(root.join("Sources/RuycastApp/SystemServices.swift").exists());
        let main = include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!root.join("Sources/RuycastApp/FeedbackChecks.swift").exists());
        assert!(!main.contains("--check-system"));
        assert!(!main.contains("checkSystemServices"));
        assert_eq!(settings_provenance_source("--check-feedback"),Some(("Sources/RuycastApp/FeedbackChecks.swift",FEEDBACK_ORACLE_SHA256)));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        ).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert!(lock["files"].get("Sources/RuycastApp/FeedbackChecks.swift").is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"],format!("{:x}",Sha256::digest(main.as_bytes())));
        let inventory = inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]["Sources/RuycastApp/SystemChecks.swift::checkSystemServices"];
        assert_eq!(mapping["originalSourceSHA256"], "c70cc5149a16a43102cc8a853bfb5a38bc6a9b3b542b4f6b7388339f1737ffa4");
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(mapping["assertionCrosswalk"].as_object().unwrap().len(), 19);
        assert_eq!(mapping["contractDifferences"].as_object().unwrap().len(), 2);
        let inputs = settings_sources("--check-feedback");
        for cases in mapping["assertionCrosswalk"].as_object().unwrap().values() {
            for case in cases.as_array().unwrap() {
                let name = case.as_str().unwrap().rsplit("::").next().unwrap();
                assert!(inputs.iter().any(|(_, source, _)| std::str::from_utf8(source).unwrap().contains(&format!("fn {name}("))), "missing historical system case: {case}");
            }
        }
    }

    #[test]
    fn retired_interaction_benchmark_has_no_caller_and_preserves_all_crosswalks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/InteractionPerformanceChecks.swift";
        assert!(!root.join(retired).exists());
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("--benchmark-interactions"));
        assert!(!main.contains("checkInteractionPerformance"));
        assert!(!include_str!("../ruycast-source/policy.rs")
            .lines()
            .any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(
            lock["files"]["Sources/RuycastApp/main.swift"],
            format!("{:x}", Sha256::digest(main.as_bytes()))
        );
        let docs = std::fs::read_to_string(root.join("docs/PERFORMANCE.md")).unwrap();
        assert!(!docs.contains("--benchmark-interactions"));
        assert!(docs.contains("interaction-trace-process"));
        assert!(docs.contains("physical desktop input"));
        assert!(!docs.contains("NSViewRepresentable.updateNSView"));
        assert!(!docs.contains("Swift result conversion"));
        assert!(root.join("Sources/RuycastApp/PerformanceTrace.swift").exists());
        let public: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(public["schema"], "ruycast.public-source.v2");
        assert!(public["files"].get(retired).is_none());
        for path in [
            "Sources/RuycastApp/main.swift",
            "docs/PERFORMANCE.md",
            "rust-native/src/bin/ruycast-build/oracles.rs",
            "rust-native/src/bin/ruycast-source/policy.rs",
            "rust-native/swift-reference-lock.json",
        ] {
            assert_eq!(
                public["files"][path],
                format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())),
                "stale public source hash: {path}"
            );
        }

        let inventory = inventory("check").unwrap();
        let mappings = &inventory["retiredSwiftChecks"];
        let expected = [
            (
                "checkInteractionPerformance.rankedFilterAfterReload",
                "rust-native/tests/execution_native_process.rs",
                "execution_native_process::ranked_command_is_filtered_after_disable_and_removal",
            ),
            (
                "checkInteractionPerformance.traceStateBoundsAndIdentity",
                "rust-native/src/performance_trace.rs",
                "performance_trace::tests::original_retention_field_ownership_cancellation_reset_and_overflow_contract",
            ),
            (
                "checkInteractionPerformance.nativeFieldCycles",
                "rust-native/examples/ui_completion.rs",
                "ui_completion::interaction_trace",
            ),
            (
                "checkInteractionPerformance.recencyWriteAndArgumentPrivacy",
                "rust-native/tests/execution_native_process.rs",
                "execution_native_process::only_real_launcher_success_writes_recency_without_argument_text",
            ),
            (
                "checkInteractionPerformance.closedFieldCancellation",
                "rust-native/examples/ui_completion.rs",
                "ui_completion::interaction_trace",
            ),
        ];
        for (scenario, source, case) in expected {
            let key = format!("{retired}::{scenario}");
            let mapping = &mappings[&key];
            assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
            assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
            assert_eq!(mapping["nativeSource"], source);
            assert!(mapping["nativeCases"].as_array().unwrap().contains(&serde_json::json!(case)));
            assert_eq!(mapping["productionAccepted"], false);
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn retired_clipboard_helper_is_absent_and_its_assertions_are_crosswalked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/ClipboardChecks.swift";
        assert!(!root.join(retired).exists());
        let main = include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-clipboard"));
        assert!(!main.contains("checkClipboardStore"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        ).unwrap();
        assert!(lock["files"].get(retired).is_none());
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]["Sources/RuycastApp/ClipboardChecks.swift::checkClipboardStore"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "bda0fc08d8b64c4598538078362da8c811617c99c223f3fa26caf62d2e33a6ae");
        assert_eq!(mapping["productionAccepted"], false);
        let cases = mapping["nativeCases"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        let host = include_str!("../../../tests/clipboard_history_host_process.rs");
        let history = include_str!("../../clipboard_history.rs");
        for case in cases {
            let id = case.as_str().unwrap();
            let function = id.rsplit("::").next().unwrap();
            let source = if id.starts_with("clipboard_history_host_process::") { host } else { history };
            assert!(source.contains(&format!("fn {function}(")), "missing Rust case {id}");
        }
        let host_hash = format!("{:x}", Sha256::digest(host.as_bytes()));
        let history_hash = format!("{:x}", Sha256::digest(history.as_bytes()));
        assert_eq!(mapping["qualifiedCaseSourceBindings"]["rust-native/tests/clipboard_history_host_process.rs"], host_hash);
        assert_eq!(mapping["qualifiedCaseSourceBindings"]["rust-native/src/clipboard_history.rs"], history_hash);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn retired_clipboard_preview_helper_is_absent_and_scenarios_are_crosswalked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/ClipboardPreviewChecks.swift";
        assert!(!root.join(retired).exists());
        let polish = include_str!("../../../../Sources/RuycastApp/PolishChecks.swift");
        assert!(!polish.contains("checkClipboardPreviewCancellation"));
        assert!(!polish.contains("previewCancellation"));
        assert!(polish.contains("clipboardLargeAndSmall"));
        assert!(polish.contains("clearedPreviewContentRemoved"));

        let policy = include_str!("../ruycast-source/policy.rs");
        assert!(!policy.lines().any(|line| line == retired));
        assert!(policy.lines().any(|line| line == "Sources/RuycastApp/ClipboardPreview.swift"));
        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert!(lock["files"]["Sources/RuycastApp/ClipboardPreview.swift"].is_string());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest["files"].get(retired).is_none());

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/ClipboardPreviewChecks.swift::checkClipboardPreviewCancellation"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "369905c6a65da6ef8995b570466b0d1d3fcd54d3dfeda96d21b940128f3292a6");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["productionAccepted"], false);
        let cases = mapping["nativeCases"].as_array().unwrap();
        assert_eq!(cases.len(), 4);
        let source_path = "rust-native/src/clipboard_preview.rs";
        let source = std::fs::read(root.join(source_path)).unwrap();
        let source_text = std::str::from_utf8(&source).unwrap();
        let source_hash = format!("{:x}", Sha256::digest(&source));
        assert_eq!(mapping["qualifiedCaseSourceBindings"][source_path], source_hash);
        for case in cases {
            let function = case.as_str().unwrap().rsplit("::").next().unwrap();
            assert!(source_text.contains(&format!("fn {function}(")), "missing Rust case {case}");
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn ax_retry_crosswalk_preserves_historical_source_and_rust_cases() {
        let mapping = &super::inventory("check").unwrap()["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testWindowReadsRetryOnlyTemporaryFailures"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "26a71c8720522dabdccadb3972b65f7546d9b9979f20af20f22b58c15011ce5c");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["nativeCases"].as_array().unwrap().len(), 3);
        assert_eq!(mapping["productionAccepted"], serde_json::Value::Null);
    }

    #[test]
    fn retired_snippet_output_helper_is_removed_and_all_cases_are_crosswalked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Tests/LauncherCoreTests/SnippetOutputChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(!include_str!("../ruycast-source/policy.rs")
            .lines()
            .any(|line| line == retired));
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"][retired];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "abaa04463b8b7fcfa1edecfd712d4911d01735991f3ecfd3c2e32455a8746965"
        );
        let cases = mapping["nativeCases"].as_array().unwrap();
        assert!(cases.len() >= 8);
        assert_eq!(inventory["productionAccepted"], false);

    }

    #[test]
    fn retired_private_file_helper_preserves_original_ordered_public_write_sequence() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Tests/LauncherCoreTests/PrivateFileChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let value = inventory("check").unwrap();
        let mapping = &value["retiredSwiftChecks"][retired];
        assert_eq!(mapping["nativeSource"], "rust/src/private_storage.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["private_storage::guarded_publication_tests::original_private_file_replacement_and_create_only_sequence"]));
        assert!(include_str!("../../../../rust/src/private_storage.rs").contains("fn original_private_file_replacement_and_create_only_sequence("));
        assert_eq!(value["productionAccepted"], false);
    }
    #[test]
    fn retired_feedback_helper_has_no_caller_and_preserves_native_assertions() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired="Sources/RuycastApp/FeedbackChecks.swift";
        assert!(!root.join(retired).exists());
        for product in ["Sources/RuycastApp/FeatureState.swift","Sources/RuycastApp/CaptureTextEditor.swift","Sources/RuycastApp/InteractionChecks.swift"] {
            assert!(root.join(product).is_file());
        }
        let main=include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-feedback"));
        assert!(!main.contains("checkFeedback"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line|line==retired));
        let lock:serde_json::Value=serde_json::from_slice(&fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"],format!("{:x}",Sha256::digest(main.as_bytes())));
        let gate=SETTINGS.iter().find(|gate|gate.id=="--check-feedback").unwrap();
        assert_eq!(settings_provenance_source(gate.id),Some((retired,FEEDBACK_ORACLE_SHA256)));
        assert_eq!(gate.native_cases.len(),23);
        assert_eq!(gate.required_native_steps,&["unit","creation-process","input-process-rust","actions-process","feedback-actions-process","utility-process","polish-original-process","system-search-cli-process","remaining-macos-process"]);
        for id in ["--check-actions-menu","--check-interactions"] {
            assert!(settings_additional_provenance_sources(id).contains(&(retired,FEEDBACK_ORACLE_SHA256)));
        }
        let value=inventory("check").unwrap();
        let mapping=&value["retiredSwiftChecks"]["Sources/RuycastApp/FeedbackChecks.swift::checkFeedback"];
        assert_eq!(mapping["originalSourceSHA256"],"03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde");
        assert_eq!(value["productionAccepted"],false);
        assert_eq!(mapping["productionAccepted"],false);
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(),5);
        let crosswalk=mapping["assertionCrosswalk"].as_object().unwrap();
        assert_eq!(crosswalk.len(),11);
        let mut cases=std::collections::BTreeSet::new();
        for names in crosswalk.values() {
            for name in names.as_array().unwrap() {
                let case=name.as_str().unwrap();
                // The preserved gate keeps its 23 historical case IDs; focused
                // additional assertions reuse its already-bound current sources.
                let function=format!("fn {}(",case.rsplit("::").next().unwrap());
                assert!(settings_sources(gate.id).iter().any(|(_,bytes,_)|std::str::from_utf8(bytes).unwrap().contains(&function)),"case outside bound feedback sources: {case}");
                let source=if case.starts_with("ui_completion::") {include_str!("../../../examples/ui_completion.rs")}
                    else if case.starts_with("utility_process::") {include_str!("../../../tests/utility_process.rs")}
                    else if case.starts_with("actions_process::") {include_str!("../../../tests/actions_process.rs")}
                    else if case.starts_with("input_process::") {include_str!("../../../tests/input_process.rs")}
                    else if case.starts_with("creation_process::") {include_str!("../../../tests/creation_process.rs")}
                    else {assert!(case.starts_with("actions::tests::"));include_str!("../../actions.rs")};
                assert!(source.contains(&format!("fn {}(",case.rsplit("::").next().unwrap())),"missing mapped case {case}");
                cases.insert(case);
            }
        }
        assert_eq!(cases.len(),10);
    }
    #[test]
    fn retired_edit_command_helper_has_no_caller_and_preserves_native_assertions() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired="Sources/RuycastApp/EditCommandChecks.swift";
        assert!(!root.join(retired).exists());
        for product in ["Sources/RuycastApp/NativeEditCommands.swift","Sources/RuycastApp/CaptureTextEditor.swift","Sources/RuycastApp/ShortcutRecorder.swift"] {
            assert!(root.join(product).is_file());
        }
        let main=include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-edit-commands"));
        assert!(!main.contains("checkNativeEditCommands"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line|line==retired));
        let lock:serde_json::Value=serde_json::from_slice(&fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"],format!("{:x}",Sha256::digest(main.as_bytes())));
        let gate=SETTINGS.iter().find(|gate|gate.id=="--check-edit-commands").unwrap();
        assert_eq!(settings_provenance_source(gate.id),Some((retired,EDIT_COMMAND_ORACLE_SHA256)));
        assert_eq!(gate.native_cases,&["native_edit::check"]);
        assert_eq!(gate.required_native_steps,&["native-light-600","native-light-760","native-dark-600","native-dark-760"]);
        let value=inventory("check").unwrap();
        let mapping=&value["retiredSwiftChecks"]["Sources/RuycastApp/EditCommandChecks.swift::checkNativeEditCommands"];
        assert_eq!(mapping["originalSourceSHA256"],"a91248845b9256c30f2906adc818e5e706034b6277a52519df2f8758203b1145");
        assert_eq!(mapping["productionAccepted"],false);
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(),5);
        let crosswalk=mapping["assertionCrosswalk"].as_object().unwrap();
        assert_eq!(crosswalk.len(),11);
        let mut cases=std::collections::BTreeSet::new();
        for names in crosswalk.values() {
            for name in names.as_array().unwrap() {
                let case=name.as_str().unwrap();
                let source=if case.starts_with("native_edit::") {include_str!("../../native_edit.rs")}
                    else if case.starts_with("input_process::") {include_str!("../../../tests/input_process.rs")}
                    else {assert!(case.starts_with("actions_process::"));include_str!("../../../tests/actions_process.rs")};
                assert!(source.contains(&format!("fn {}(",case.rsplit("::").next().unwrap())),"missing mapped case {case}");
                cases.insert(case);
            }
        }
        assert_eq!(cases.len(),9);
    }
    #[test]
    fn retired_script_choice_ui_helper_has_no_caller_and_preserves_assertion_crosswalk() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired="Sources/RuycastApp/ScriptChoiceChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(root.join("Sources/RuycastApp/ScriptChoiceView.swift").is_file());
        let main=include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-script-choices"));
        assert!(!main.contains("checkScriptChoicesUI"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line|line==retired));
        let lock:serde_json::Value=serde_json::from_slice(&fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"],format!("{:x}",Sha256::digest(main.as_bytes())));
        let value=inventory("check").unwrap();
        let mapping=&value["retiredSwiftChecks"]["Sources/RuycastApp/ScriptChoiceChecks.swift::checkScriptChoicesUI"];
        assert_eq!(mapping["originalSourceSHA256"],"7d98578478074c4c8c25d39ffc0dda8048debadd62f643ef1265700d449336cc");
        assert_eq!(mapping["productionAccepted"],false);
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(),3);
        let crosswalk=mapping["assertionCrosswalk"].as_object().unwrap();
        assert_eq!(crosswalk.len(),8);
        let mut cases=std::collections::BTreeSet::new();
        for names in crosswalk.values() {
            for name in names.as_array().unwrap() {
                let case=name.as_str().unwrap();
                let source=if case.starts_with("rich_process::") {include_str!("../../../tests/rich_process.rs")} else {assert!(case.starts_with("script_result::tests::"));include_str!("../../script_result.rs")};
                assert!(source.contains(&format!("fn {}(",case.rsplit("::").next().unwrap())),"missing mapped case {case}");
                cases.insert(case);
            }
        }
        assert_eq!(cases.len(),10);
    }
    #[test]
    fn retired_script_choice_helper_preserves_all_original_scenarios_in_rust() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Tests/LauncherCoreTests/ScriptChoiceChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        assert!(!root.join("Sources/RuycastApp/ScriptChoiceChecks.swift").exists());
        let value = inventory("check").unwrap();
        let mapping = &value["retiredSwiftChecks"][retired];
        assert_eq!(mapping["nativeSource"], "rust-native/src/script_result.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["script_result::tests::original_script_choice_search_and_validation_scenarios"]));
        assert!(include_str!("../../script_result.rs").contains("fn original_script_choice_search_and_validation_scenarios("));
        assert_eq!(value["productionAccepted"], false);
    }
    #[test]
    fn retired_task_date_helper_preserves_all_original_scenarios_in_rust() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Tests/LauncherCoreTests/TaskDateInputChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let value = inventory("check").unwrap();
        let mapping = &value["retiredSwiftChecks"][retired];
        let expected = [
            "task_date::tests::original_natural_date_vectors_and_labels_match",
            "task_date::tests::calendar_month_year_and_amsterdam_dst_boundaries_are_not_fixed_seconds",
            "task_date::tests::strict_absolute_roundtrip_year_bounds_invalid_and_frozen_clock",
            "task_date::tests::only_script_prompts_with_the_original_case_sensitive_hint_opt_in",
        ];
        assert_eq!(mapping["nativeSource"], "rust-native/src/task_date.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(expected));
        let source = include_str!("../../task_date.rs");
        for case in expected {
            assert!(source.contains(&format!("fn {}(", case.rsplit("::").next().unwrap())));
        }
        assert_eq!(value["productionAccepted"], false);
    }
    #[test]
    fn prepared_ranking_core_test_is_crosswalked_to_rust_snapshot_test() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(!root.join("Tests/LauncherCoreTests/ABIChecks.swift").exists());

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/ABIChecks.swift::checkPreparedRanking"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        let case = "original_prepared_ranking_snapshots_track_success_reset_and_replacement";
        assert_eq!(mapping["nativeCases"], serde_json::json!([format!("ranking::tests::{case}")]));
        assert!(include_str!("../../ranking.rs").contains(&format!("fn {case}()")));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn retired_abi_helper_assertions_are_crosswalked_and_wrapper_claims_are_retired() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Tests/LauncherCoreTests/ABIChecks.swift";
        assert!(!root.join(retired).exists());
        assert!(!include_str!("../ruycast-source/policy.rs")
            .lines()
            .any(|line| line == retired));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/ABIChecks.swift::checkImportedABI"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "19811571df19e5dd8c5cb066f5a5a15ad13f5c37d482219aa17d96a4dbc48eb3"
        );
        assert_eq!(
            mapping["nativeCases"],
            serde_json::json!(["tests::original_imported_abi_version_ranked_and_empty_query_order"])
        );
        assert_eq!(
            mapping["retiredAssertions"],
            serde_json::json!([
                "Swift C-imported declarations invoke the linked archive through the Swift wrapper",
                "PreparedCommandSearch retains the Swift CommandSearchIndex owner after its local creator exits"
            ])
        );
        assert!(include_str!("../../../../rust/src/lib.rs")
            .contains("fn original_imported_abi_version_ranked_and_empty_query_order()"));
        assert_eq!(inventory["productionAccepted"], false);
    }
    #[test]
    fn exact_alias_core_test_is_crosswalked_to_rust_runtime_test() {
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testExactAliasBeatsTitleAndSupportsMultipleTerms"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        let case = "original_exact_alias_title_multiterm_and_uppercase_resolution";
        assert_eq!(mapping["nativeCases"], serde_json::json!([format!("runtime::inline_tests::{case}")]));
        assert!(include_str!("../../runtime_private_tests.rs").contains(&format!("fn {case}()")));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn bounded_typo_fallback_core_test_is_retired_to_two_rust_cases() {
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testBoundedTypoFallback"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "82ba1d4e045a1c35f59834ed005e764240f4d8fbd211fd7d130b113fbeb57c8e"
        );
        let expected = [
            "tests::original_bounded_typo_fallback_search_cases",
            "runtime::inline_tests::original_bounded_typo_fallback_keeps_resolution_literal",
        ];
        assert_eq!(mapping["nativeCases"], serde_json::json!(expected));
        assert!(include_str!("../../../../rust/src/lib.rs")
            .contains("fn original_bounded_typo_fallback_search_cases()"));
        assert!(include_str!("../../runtime_private_tests.rs")
            .contains("fn original_bounded_typo_fallback_keeps_resolution_literal()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn long_snippet_chunk_assertions_remain_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.longText"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "6eda2747d7ea8c664a44070efbb38078fae2be761b939530b9270669be8ab84e"
        );
        let case = "keyword_matcher::cancellation_checks::original_long_snippet_unicode_chunks_preserve_complete_text_and_utf16_limit";
        assert_eq!(mapping["nativeCases"], serde_json::json!([case]));
        assert!(include_str!("../../keyword_matcher.rs")
            .contains("fn original_long_snippet_unicode_chunks_preserve_complete_text_and_utf16_limit()"));

        let input = super::settings_sources("--check-interactions")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/keyword_matcher.rs")
            .unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
        assert!(super::settings_source_bindings("--check-interactions").contains(&(input.0, input.2)));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn required_snippet_argument_assertion_remains_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.requiredArgument"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "56df09b0b0669f0a326ebcde08f54c2f9db3b539bc6a77b65d6b33dd497b37d3"
        );
        let case = "keyword_matcher::cancellation_checks::original_required_snippet_argument_prevents_keyword_expansion";
        assert_eq!(mapping["nativeCases"], serde_json::json!([case]));
        assert!(include_str!("../../keyword_matcher.rs")
            .contains("fn original_required_snippet_argument_prevents_keyword_expansion()"));

        let input = super::settings_sources("--check-interactions")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/keyword_matcher.rs")
            .unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
        assert!(super::settings_source_bindings("--check-interactions")
            .contains(&(input.0, input.2)));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn snippet_matcher_assertions_remain_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.matcher"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "dba3ad2b3ec56498c7fa2613b850a488ddb1e99d496847c0bdaabbf7e886206b");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 6);
        let case = "keyword_matcher::cancellation_checks::original_snippet_prefix_reset_backspace_and_word_boundary";
        assert_eq!(mapping["nativeCases"], serde_json::json!([case]));
        assert!(include_str!("../../keyword_matcher.rs").contains("fn original_snippet_prefix_reset_backspace_and_word_boundary()"));

        let input = super::settings_sources("--check-interactions")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/keyword_matcher.rs")
            .unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
        assert!(super::settings_source_bindings("--check-interactions").contains(&(input.0, input.2)));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn snippet_render_assertions_remain_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.snippetRender"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "bdb976419921267514aba471ddb15032aeda190a3893315a8233113d8407352b");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 3);
        let case = "templates::tests::original_snippet_clipboard_default_cursor_and_missing_clipboard";
        assert_eq!(mapping["nativeCases"], serde_json::json!([case]));
        assert!(include_str!("../../templates.rs").contains("fn original_snippet_clipboard_default_cursor_and_missing_clipboard()"));

        let input = super::settings_sources("--check-interactions")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/templates.rs")
            .unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
        assert!(super::settings_source_bindings("--check-interactions").contains(&(input.0, input.2)));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn duplicate_snippet_keyword_assertion_remains_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.duplicateKeyword"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "a4ed4e769adecebd65fcc7a2cc2e95c85574ac34b7468cb84988b8af03f22686"
        );
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(
            mapping["nativeCases"],
            serde_json::json!(["tests::original_duplicate_snippet_keyword_is_rejected"])
        );
        assert!(include_str!("../../../../rust/src/lib.rs")
            .contains("fn original_duplicate_snippet_keyword_is_rejected()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn url_template_assertions_are_crosswalked_after_swift_helper_retirement() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.urlRender"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "200fc776a731ffdf77637be04f67868d50f4c0b5cdcfeb98e2efeb446ce5c4e0");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 4);
        let case = "templates::tests::original_url_template_fields_encoding_and_missing_input";
        assert_eq!(mapping["nativeCases"], serde_json::json!([case]));
        assert!(include_str!("../../templates.rs").contains("fn original_url_template_fields_encoding_and_missing_input()"));
        for scenario in ["longText", "requiredArgument", "matcher", "snippetRender", "duplicateKeyword"] {
            let key = format!(
                "Tests/LauncherCoreTests/CoreTests.swift::testTemplatesAndSnippetMatching.{scenario}"
            );
            let mapping = inventory["retiredSwiftChecks"].get(&key).unwrap();
            assert!(mapping["scope"].as_str().unwrap().contains("Swift helper was retired"));
        }

        let input = super::settings_sources("--check-interactions")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/templates.rs")
            .unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
        assert!(super::settings_source_bindings("--check-interactions").contains(&(input.0, input.2)));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn empty_catalog_search_crosswalk_preserves_rust_behavior_mapping() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.emptyCatalog"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "f4cbecca6b0db72586d7cd56f1a3e0c1b0c7a2c6322d981be1e989508c467975"
        );
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(
            mapping["nativeCases"],
            serde_json::json!(["tests::original_empty_catalog_search_returns_no_commands"])
        );
        assert!(include_str!("../../../../rust/src/lib.rs")
            .contains("fn original_empty_catalog_search_returns_no_commands()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn unicode_scalar_search_assertions_are_crosswalked_to_rust() {
        use sha2::Digest;

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.unicodeScalars"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30bd876a5a37a0cf0363a24b5aa75ac877a34fff60eb3259be91f0faf5f22b68");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_unicode_scalar_and_expanded_lowercase_search"]));
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_unicode_scalar_and_expanded_lowercase_search()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn seven_query_index_parity_is_crosswalked_to_rust() {
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.legacyParity"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "b6bb2be6a66ec71d5467fcd1363d9cd9e2f668b6dfd9d081f1b3bd01140f1427");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 7);
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_seven_query_index_dispatch_and_abi_parity"]));
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_seven_query_index_dispatch_and_abi_parity()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn private_library_aggregation_is_crosswalked_to_rust() {
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::CheckRunner.library"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "5f09411bdd0943a44188f2b1f6e851aa9ca14cee681fdaba092ce374e694ff36");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["nativeSource"], "rust-native/tests/library_verification.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["original_private_library_aggregation_renders_templates_and_checks_keywords"]));
        assert!(include_str!("../../../../rust-native/tests/library_verification.rs").contains("fn original_private_library_aggregation_renders_templates_and_checks_keywords()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn ranking_smoke_flow_is_crosswalked_to_rust() {
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.rankingSmoke"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "36067c3a65ad03c93f76c6664118bedc1caed1fc6c89d046f292b132acb3e1f2");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 3);
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_reset_ranking_is_empty_after_reopen"]));
        assert!(include_str!("../../ranking.rs").contains("fn original_reset_ranking_is_empty_after_reopen()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn large_equal_title_ties_are_crosswalked_to_rust() {
        use sha2::Digest;
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.largeTies"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "063c54f1cf0c835dd3547f07155d2dc9b446e93ed43c34459647913f3d38e435");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 3);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_large_equal_title_ties_preserve_order_and_alias_precedence"]));
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_large_equal_title_ties_preserve_order_and_alias_precedence()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn large_equal_title_ties_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn large_equal_title_ties_are_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    }\n\n    #[test]")
            .unwrap()
            .0;
        let mut code = String::with_capacity(assertion.len());
        let mut chars = assertion.chars().peekable();
        let mut in_string = false;
        let mut escaped = false;
        while let Some(character) = chars.next() {
            if in_string {
                code.push(character);
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                }
            } else if character == '"' {
                in_string = true;
                code.push(character);
            } else if character == '/' && chars.peek() == Some(&'/') {
                chars.next();
                for rest in chars.by_ref() {
                    if rest == '\n' {
                        code.push(rest);
                        break;
                    }
                }
            } else if character == '/' && chars.peek() == Some(&'*') {
                chars.next();
                let mut depth = 1usize;
                while let Some(comment_character) = chars.next() {
                    if comment_character == '/' && chars.peek() == Some(&'*') {
                        chars.next();
                        depth += 1;
                    } else if comment_character == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else {
                code.push(character);
            }
        }
        assert!(!code.contains("../../../../Tests/LauncherCoreTests/CoreTests.swift"));
    }

    #[test]
    fn future_ranking_timestamp_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../ranking.rs").contains("fn original_loaded_ranking_rejects_future_timestamp()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.futureTimestamp"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "4490761aa2666c7410a1aedfc0d821b509d597edf007a8069a242ba6557b1f8d");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_loaded_ranking_rejects_future_timestamp"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn future_ranking_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn future_ranking_timestamp_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn expired_ranking_timestamp_assertion_is_crosswalked_to_rust() {
        let source = include_str!("../../ranking.rs");
        assert!(source.contains("fn original_loaded_ranking_rejects_expired_timestamp()"));
        assert!(source.contains("original.entries().contains_key(\"expired\")"));
        assert!(source.contains("isolated.entries().contains_key(\"recent\")"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.expiredTimestamp"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "3d99229bda839e28e50b1448cc9d587b923f9bedaaaba5e30312c84d15a1b0a8");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_loaded_ranking_rejects_expired_timestamp"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn expired_ranking_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn expired_ranking_timestamp_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn loaded_ranking_entry_limit_is_crosswalked_to_rust() {
        assert!(include_str!("../../ranking.rs").contains("fn original_loaded_ranking_is_capped_at_512_entries()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.loadedEntryLimit"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "c069c4eb7a5bb4941bb1d7ffb4840b330e3e00ab8b514f0d4fcd7fbf2809cfab");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_loaded_ranking_is_capped_at_512_entries"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn loaded_ranking_entry_limit_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn loaded_ranking_entry_limit_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn reset_reopen_ranking_assertion_is_crosswalked_to_rust() {
        let source = include_str!("../../ranking.rs");
        assert!(source.contains("fn original_reset_ranking_is_empty_after_reopen()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.resetReopen"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "11160b6434aafdad4a752f09616cae0bcb42d46acca01d6e00596650b759a525");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_reset_ranking_is_empty_after_reopen"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn reset_reopen_ranking_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn reset_reopen_ranking_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn persisted_ranking_timestamp_assertion_is_crosswalked_to_rust() {
        let source = include_str!("../../ranking.rs");
        assert!(source.contains("fn original_successful_choice_timestamp_survives_reopen()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.persistedTimestamp"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "80b3dc47d5203860e1e775eb4338ce974267902a2bd1b353b146ce28b8e5325b");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_successful_choice_timestamp_survives_reopen"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn persisted_ranking_timestamp_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn persisted_ranking_timestamp_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn ranking_private_permissions_assertion_is_crosswalked_to_rust() {
        let source = include_str!("../../ranking.rs");
        assert!(source.contains("fn original_successful_choice_ranking_file_has_private_permissions()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.privatePermissions"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "d318506a3da5ce51fad4040432486b656bb8553840306964b9df698f3a93f718");
        assert_eq!(mapping["nativeSource"], "rust-native/src/ranking.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ranking::tests::original_successful_choice_ranking_file_has_private_permissions"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn ranking_private_permissions_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn ranking_private_permissions_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn go_alias_query_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_go_alias_query_preserves_complete_result_order()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.goAliasOrder"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "8376288be5aa8ecd4a2eb4794ee49281b575bfc91cff835acf1b8e95909e6c63");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_go_alias_query_preserves_complete_result_order"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn go_alias_query_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn go_alias_query_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn repeated_go_alias_index_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../../../rust/src/lib.rs");
        assert!(rust.contains("fn original_go_alias_query_preserves_complete_result_order()"));
        assert!(rust.contains("for _ in 0..100 {"));
        assert!(rust.contains("assert_eq!(first_id, Some(\"z\"));"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.goAliasTemporaryIndex"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "6329a81b5813d5242f548ccea7f514e54ba40670e9a250796e57831c1200b9b5");
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_go_alias_query_preserves_complete_result_order"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn repeated_go_alias_index_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("fn repeated_go_alias_index_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn aggregate_search_crosswalk_requires_every_current_rust_case_and_wrapper_exclusions() {
        let inventory = super::inventory("check").unwrap();
        assert!(inventory["gates"].as_array().unwrap().contains(&serde_json::json!(["--check-search", "source-qualified-rust-case-crosswalk"])));
        let aggregate = &inventory["retiredSwiftChecks"]["--check-search"];
        assert_eq!(aggregate["scenarioIDs"].as_array().unwrap().len(), SEARCH_CASES.len());
        assert_eq!(aggregate["qualifiedCaseSourceBindings"].as_object().unwrap().len(), 4);
        assert_eq!(aggregate["retiredAssertions"].as_array().unwrap().len(), 3);
        for (id, _, _) in SEARCH_CASES {
            for field in ["status", "nativeSource", "nativeCases", "originalScenarios"] {
                let mut mappings = inventory["retiredSwiftChecks"].clone();
                mappings[*id].as_object_mut().unwrap().remove(field);
                assert!(search_crosswalk(&mappings).is_err(), "{id}: {field}");
            }
        }
    }

    #[test]
    fn unchanged_search_ranking_reuse_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("fn prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection()"));
        assert!(rust.contains("(0,0,1000)"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.unchangedRankingReuse"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7c9174e4d63490a1a7962b60c8621d3ab51d67568f23413809c54a2564f87ccf");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_ranking_change_repreparation_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("state.ranking.record_success(\"cache.b\", 1_700_000_000.)"));
        assert!(rust.contains("[\"cache.b\", \"cache.a\"]"));
        assert!(rust.contains("(ranked.0-after.0,ranked.1-after.1),(0,1)"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingChangeReprepare"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7823f355133f4084ad263b919fb1a970a42d04e284a55eb146a689a4a676cc03");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_ranking_reset_repreparation_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("fn prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection()"));
        assert!(rust.contains("state.ranking.reset().unwrap();"));
        assert!(rust.contains("[\"cache.a\", \"cache.b\"]"));
        assert!(rust.contains("crate::catalog::preparation_work().1,ranked.1+1"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankingResetReprepare"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "c06de8ddfc1f33428f64d4d01c73bf351cf0a86db39a14ca986a4e261fe1f5b1");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_ranked_disabled_item_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("state.ranking.record_success(\"cached.b\",1_700_000_001.).unwrap();"));
        assert!(rust.contains("assert_eq!(ids(system::local(&state,\"identical\",0)),[\"cached.b\",\"cached.a\"]);"));
        assert!(rust.contains("scenario[\"commands\"][1][\"enabled\"]=json!(false);"));
        assert!(rust.contains("assert_eq!(ids(system::local(&state,\"identical\",0)),[\"cached.a\"]);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.rankedDisabledItem"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "485ba7145885bf21409dc40d99cbf69ecd5861a93e517f4e89482a9532a023b7");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn interaction_rank_filter_after_reload_is_crosswalked_to_rust() {
        let test = include_str!("../../../tests/execution_native_process.rs");
        let case = test
            .split_once("fn ranked_command_is_filtered_after_disable_and_removal()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        assert!(case.contains("search_has_command(true)"));
        assert!(case.contains("[\"enabled\"] = json!(false)"));
        assert!(case.contains("host.rpc(&json!({\"op\":\"reload\"}))"));
        assert!(case.contains("removed[\"commands\"]"));
        assert!(case.contains("assert_eq!(fs::read(&ranking_path).unwrap(), ranking_bytes)"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.rankedFilterAfterReload"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
        assert_eq!(mapping["nativeSource"], "rust-native/tests/execution_native_process.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["execution_native_process::ranked_command_is_filtered_after_disable_and_removal"]));
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn interaction_trace_state_assertions_are_crosswalked_to_rust() {
        let rust = include_str!("../../performance_trace.rs");
        let case = rust
            .split_once("fn original_retention_field_ownership_cancellation_reset_and_overflow_contract()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        assert!(case.contains("for _ in 0..350"));
        assert!(case.contains("[\"retainedSamples\"], 256"));
        assert!(case.contains("trace.did_reconcile_field(2)"));
        assert!(case.contains("trace.cancel_input(Some(2))"));
        assert!(case.contains("[\"cancelled\"],\n            2"));
        assert!(case.contains("trace.reset();"));
        assert!(case.contains("let spans: Vec<_> = (0..100)"));
        assert!(case.contains("assert_eq!(trace.snapshot()[\"activeIntervals\"], 32)"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.traceStateBoundsAndIdentity"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
        assert_eq!(mapping["nativeSource"], "rust-native/src/performance_trace.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 5);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["performance_trace::tests::original_retention_field_ownership_cancellation_reset_and_overflow_contract"]));
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn interaction_native_field_cycles_are_crosswalked_to_rust_route() {
        let rust = include_str!("../../../examples/ui_completion.rs");
        let case = rust
            .split_once("fn interaction_trace(root: &Path)")
            .unwrap()
            .1
            .split_once("\nfn ")
            .unwrap()
            .0;
        assert!(case.contains("for index in 0..100u64"));
        assert!(case.contains("let terms=[\"E\",\"Ex\",\"Example 4\",\"Example 42\",\"\"]"));
        assert!(case.contains("input.reconcile"));
        assert!(case.contains("h.capture(false,&format!(\"interaction-trace-cycle-{index:03}\"))"));
        assert!(case.contains("Actual host capture did not draw"));
        assert!(case.contains("for rejected in [\"Example\",\"fixture.\",\"example.invalid\""));
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        assert!(plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["name"] == "interaction-trace-process"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.nativeFieldCycles"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
        assert_eq!(mapping["nativeSource"], "rust-native/examples/ui_completion.rs");
        assert_eq!(mapping["validationSteps"], serde_json::json!(["interaction-trace-process"]));
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ui_completion::interaction_trace"]));
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn interaction_launcher_recency_is_crosswalked_to_rust() {
        let test = include_str!("../../../tests/execution_native_process.rs");
        let case = test
            .split_once("fn only_real_launcher_success_writes_recency_without_argument_text()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        assert!(case.contains("host.rpc_timeout(&json!({\"op\":\"execute\",\"id\":success_id})"));
        assert!(case.contains("\"dryRun\":true"));
        assert!(case.contains("\"source\"] == \"launcher\" && entry[\"success\"] == false"));
        assert!(case.contains("entries.len(), 1"));
        assert!(case.contains("contains(&sentinel)"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.recencyWriteAndArgumentPrivacy"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
        assert_eq!(mapping["nativeSource"], "rust-native/tests/execution_native_process.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 3);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["execution_native_process::only_real_launcher_success_writes_recency_without_argument_text"]));
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn interaction_closed_field_cancellation_is_crosswalked_to_rust() {
        let rust = include_str!("../../../examples/ui_completion.rs");
        let case = rust
            .split_once("fn interaction_trace(root: &Path)")
            .unwrap()
            .1
            .split_once("\nfn ")
            .unwrap()
            .0;
        assert!(case.contains("h.probe(false,\"hide-palette\",json!({}))"));
        assert!(case.contains("OWN_CLOSED_QUERY_SENTINEL"));
        assert!(case.contains("value[\"stages\"][\"input.reconcile\"][\"cancelled\"]==1"));
        assert!(case.contains("cancelled[\"stages\"][\"input.reconcile\"][\"count\"],0"));
        assert!(case.contains("cancelled[\"activeIntervals\"],0"));
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        assert!(plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["name"] == "interaction-trace-process"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/InteractionPerformanceChecks.swift::checkInteractionPerformance.closedFieldCancellation"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "7d755a7b89046aff2f7325c0c24625b49cd187deb5c4dd7e2992bd7d96bb35da");
        assert_eq!(mapping["nativeSource"], "rust-native/examples/ui_completion.rs");
        assert_eq!(mapping["validationSteps"], serde_json::json!(["interaction-trace-process"]));
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["ui_completion::interaction_trace"]));
        assert_eq!(mapping["contractDifferences"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["productionAccepted"], false);
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_prepared_catalog_replacement_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("replacement[\"id\"]=json!(\"cached.replacement\");"));
        assert!(rust.contains("replacement[\"kind\"]=json!(\"url\");"));
        assert!(rust.contains("scenario[\"commands\"]=json!([replacement]);"));
        assert!(rust.contains("assert_eq!(ids(system::local(&state,\"identical\",0)),[\"cached.replacement\"]);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.preparedCatalogReplacement"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "21fe6af7bffa87228e8101d368f4cad7f2bd3e33880556a4615dada2ca377213");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn disabled_typo_item_assertion_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("fn disabled_catalog_item_is_filtered_from_typo_fallback()"));
        assert!(rust.contains("disabled.title = \"Finder\".into();"));
        assert!(rust.contains("disabled.enabled = Some(false);"));
        assert!(rust.contains("system::local(&state, \"fnider\", 0).is_empty()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.disabledTypoItem"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "f1e627323033abfcea7068439bae41a22246f8045ddc6f47788f5132d350618d");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::disabled_catalog_item_is_filtered_from_typo_fallback"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_direct_title_typo_palette_order_is_crosswalked_to_rust() {
        let rust = include_str!("../../../../rust/src/lib.rs");
        assert!(rust.contains("fn direct_title_typo_outranks_weak_subsequence_with_higher_recency()"));
        assert!(rust.contains(r#"index.search_with_recency("fnider", &[0.0, 10_000.0])"#));
        assert!(rust.contains(r#"assert_eq!(ids, ["typo.finder", "typo.distractor"]);"#));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoPaletteOrder"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "69997e74e26bc8029be2630175124c7a7dcb2276767024d61534c1e34f17a6ef");
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::direct_title_typo_outranks_weak_subsequence_with_higher_recency"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_direct_title_typo_cli_order_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_system.rs");
        let scenario = rust
            .split_once("fn cli_direct_title_typo_outranks_higher_recency_subsequence()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        assert!(scenario.contains("case.state.ranking.record_success(\"typo.distractor\", 10_000.).unwrap();"));
        assert!(scenario.contains("case.window.session.set_query(\"fnider\");"));
        assert!(scenario.contains("let expected = [\"typo.finder\", \"typo.distractor\"]"));
        assert!(scenario.contains("\"op\":\"search\",\"mode\":\"commands\",\"query\":\"fnider\""));
        assert!(scenario.contains("let request = match requests.recv_timeout"));
        assert!(scenario.contains("assert_eq!(extra, 0, \"direct title typo search must send one response\");"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.directTitleTypoCliOrder"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_system.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::system::shortcut_budget_tests::cli_direct_title_typo_outranks_higher_recency_subsequence"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_appearance_catalog_stability_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("fn prepared_palette_theme_does_not_reindex_reprepare_or_reevaluate()"));
        assert!(rust.contains(r#"config["theme"]=json!("graphite"); state.configuration.save(&config).unwrap();"#));
        assert!(rust.contains("window.sync(&state);"));
        assert!(rust.contains("assert_eq!(crate::catalog::preparation_work(),work);"));
        assert!(rust.contains("assert_eq!(window.session.results_revision(),results);"));
        assert!(rust.contains(r#"config["commands"][0]["title"]=json!("Different command");"#));
        assert!(rust.contains("assert_eq!(crate::catalog::preparation_work().0,work.0+1);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.appearancePreservesCatalog"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "a3c7038a6a68b8ccbc5cebc38ef5dbfae358eb3079e2424c79337906c8704d22");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_palette_theme_does_not_reindex_reprepare_or_reevaluate"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_usage_ranking_refresh_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        assert!(rust.contains("fn prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection()"));
        assert!(rust.contains(r#"state.activity.counts.insert("cache.b".into(),7); state.activity.usage_revision+=1;"#));
        assert!(rust.contains(r#"assert_eq!(system::local(&state,"",0)[0].id,"cache.b");"#));
        assert!(rust.contains(r#"assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]);"#));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.usageRankingRefreshes"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "4ed29d77d3c72b1ee1e7489ddad15bfc30f7bb2bb9864b91650f6ae4a6d0be0b");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_cached_result_reads_are_crosswalked_to_rust() {
        let rust = include_str!("../../session.rs");
        assert!(rust.contains("fn repeated_result_reads_preserve_prepared_rows_and_session_state()"));
        assert!(rust.contains("let search_work = crate::catalog::preparation_work();"));
        assert!(rust.contains("for _ in 0..1_000 {"));
        assert!(rust.contains("let current: Vec<String> = session.rows().map(|command| command.id.clone()).collect();"));
        assert!(rust.contains("assert_eq!(crate::catalog::preparation_work(), search_work);"));
        assert!(rust.contains("assert_eq!(session.results_revision(), revision);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.cachedReads"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "5e1d2ddaffc976900b0c72b60fee701b3484dbbfb7b61a013acb23c522d441f5");
        assert_eq!(mapping["nativeSource"], "rust-native/src/session.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["session::tests::repeated_result_reads_preserve_prepared_rows_and_session_state"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_index_dispatch_abi_parity_is_crosswalked_to_rust() {
        let rust = include_str!("../../../../rust/src/lib.rs");
        assert!(rust.contains("fn original_256_command_ten_query_index_dispatch_and_abi_parity()"));
        assert!(rust.contains("let commands: Vec<Command> = (0..256)"));
        assert!(rust.contains(r#"let queries = [
            "",
            " ",
            "MICROSOFT",
            "a42",
            "item.11",
            "project dashboard",
            "example.com/42",
            "missing",
            "Ångström",
            "a\0b",
        ];"#));
        assert!(rust.contains("assert_eq!(dispatched, indexed, \"index/dispatch parity"));
        assert!(rust.contains(r#"let dispatched = dispatch(
                "search",
                json!({"commands": commands, "query": query}),
            )
            .unwrap();"#));
        assert!(rust.contains("rustcast_index_search_ranked_into("));
        assert!(rust.contains("ABI result count for {query:?}"));
        assert!(rust.contains("ABI order for {query:?}"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.indexDispatchABIParity"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "ce5f4ed8c8cac22da5cfca994c948725930cd3a63946b29a9d266c4643f2baa7");
        assert_eq!(mapping["nativeSource"], "rust/src/lib.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_256_command_ten_query_index_dispatch_and_abi_parity"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_cli_palette_isolation_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_system.rs");
        assert!(rust.contains("fn browser_busy_main_cli_preserves_local_results_and_exactly_one_response()"));
        assert!(rust.contains("palette_query"));
        assert!(rust.contains("palette_rows"));
        assert!(rust.contains("palette_selection"));
        assert!(rust.contains("palette_revision"));
        assert!(rust.contains("CLI search changed the active palette query"));
        assert!(rust.contains("CLI search changed the full active palette rows"));
        assert!(rust.contains("CLI search invalidated the active palette"));
        assert!(rust.contains("assert_eq!(extra,0,\"late provider must not produce a second wire response\")"));
        assert!(rust.contains("busy optional provider must preserve CLI locals"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.cliSearchLeavesPaletteUntouched"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "c2c2962d6d5da6e9d2f17cd54eae21345ad402be9975f9c398b84141cc682920");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_system.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["retiredAssertions"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_discovered_application_palette_refresh_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        let scenario = rust
            .split_once("fn discovered_application_refreshes_an_existing_palette_query()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        let query = scenario.find("window.session.set_query(\"New App\");").unwrap();
        let empty = scenario.find("assert!(window.session.rows().next().is_none());").unwrap();
        let app_revision = scenario.find("state.applications_revision += 1;").unwrap();
        let sync = scenario.find("window.sync(&state);").unwrap();
        let palette_result = scenario.find("window.session.rows().map(|command| command.id.as_str()).collect::<Vec<_>>()").unwrap();
        assert!(query < empty && empty < app_revision && app_revision < sync && sync < palette_result);
        assert!(scenario.contains("state.applications = vec![discovered];"));
        assert!(scenario.contains("[\"discovered.test\"]"));
        assert!(scenario.contains("window.session.results_revision() > before_revision"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.discoveredAppsRefresh"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "d4af372db2da26489feaf66123891ad1a1930d33b726f72fa30d6f7bdf0b2dc5");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::discovered_application_refreshes_an_existing_palette_query"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_configuration_reload_deduplication_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        let scenario = rust
            .split_once("fn configuration_reload_deduplicates_discovered_app_in_active_palette()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        let query = scenario.find("window.session.set_query(\"New App\");").unwrap();
        let discovery = scenario.find("state.applications = vec![discovered];").unwrap();
        let discovery_rows = scenario.find("[\"discovered.test\"]").unwrap();
        let disk_write = scenario.find("crate::private_storage::write(").unwrap();
        let reload = scenario.find("state.reload_watched().unwrap()").unwrap();
        let sync = scenario.rfind("window.sync(&state);").unwrap();
        let replacement_rows = scenario.find("[\"replacement\"]").unwrap();
        assert!(query < discovery && discovery < discovery_rows && discovery_rows < disk_write);
        assert!(disk_write < reload && reload < sync && sync < replacement_rows);
        assert!(scenario.contains("assert_eq!(window.session.query, \"New App\")"));
        assert!(scenario.contains("replacement.target = \"org.test.app\".into();"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.reloadDeduplicatesDiscoveredApp"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::configuration_reload_deduplicates_discovered_app_in_active_palette"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_palette_status_observation_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        let scenario = rust
            .split_once("fn palette_status_changes_preserve_rows_and_selection_updates_publish()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        let scope = scenario.find("window.session.launcher_scope = Some(crate::session::LauncherScope::Quicklinks);").unwrap();
        let query = scenario.find("window.session.set_query(\"Owned private\");").unwrap();
        assert!(scope < query);
        assert!(scenario.contains("state.clipboard.as_mut().unwrap().pause(true, || true).unwrap();"));
        assert!(scenario.contains("state.configuration.last_error = Some(\"Fixture configuration error\".into());"));
        assert!(scenario.contains("assert_eq!(error_status.message, \"Config: Fixture configuration error\");"));
        assert!(scenario.contains("assert_eq!(error_status.selection, baseline.selection);"));
        assert!(scenario.contains("assert_eq!(selected.selection, Some(1));"));
        assert!(scenario.contains("assert!(selected.revision > error_status.revision);"));
        assert!(scenario.contains("assert_eq!(selected.rows_revision, baseline.rows_revision);"));
        assert!(scenario.contains("assert_eq!(crate::catalog::preparation_work(), preparation);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.selectiveResultObservation"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["retiredAssertions"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::palette_status_changes_preserve_rows_and_selection_updates_publish"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_settings_surface_observation_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_private_tests.rs");
        let scenario = rust
            .split_once("fn settings_presentation_ignores_palette_navigation_and_clears_configuration_error()")
            .unwrap()
            .1
            .split_once("\n#[test]")
            .unwrap()
            .0;
        assert!(scenario.contains("settings.session.navigate(\"tab\", \"advanced\").unwrap();"));
        assert!(scenario.contains("state.configuration.last_error = Some(\"Fixture configuration error\".into());"));
        assert!(scenario.contains("palette.session.set_query(\"changed query\");"));
        assert!(scenario.contains("palette.clipboard_selection = 2;"));
        assert!(scenario.contains("assert_eq!(navigated.settings.as_ref(), Some(&initial_surface));"));
        assert!(scenario.contains("state.configuration.last_error = None;"));
        assert!(scenario.contains("assert!(recovered.revision > navigated.revision);"));
        assert!(scenario.contains("assert_eq!(state.configuration.revision, revision);"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.settingsSurfaceIsolation"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_private_tests.rs");
        assert_eq!(mapping["retiredAssertions"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::inline_tests::settings_presentation_ignores_palette_navigation_and_clears_configuration_error"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_enabled_typo_cli_parity_is_crosswalked_to_rust() {
        let rust = include_str!("../../runtime_system.rs");
        let scenario = rust
            .split_once("fn typo_fallback_matches_palette_and_local_cli_without_disabled_items()")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        assert!(scenario.contains("case.state.system.main_tabs_enabled = false;"));
        assert!(scenario.contains("case.window.session.set_query(\"calender\");"));
        assert!(scenario.contains("assert_eq!(palette_ids, [\"typo.enabled\"]);"));
        assert!(scenario.contains("\"op\":\"search\",\"mode\":\"commands\",\"query\":\"calender\""));
        assert!(scenario.contains("let request = match requests.recv_timeout"));
        assert!(scenario.contains("assert_eq!(extra, 0, \"local typo search must send one response\");"));
        assert!(scenario.contains("\"typo.disabled\""));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/SearchChecks.swift::checkSearch.enabledTypoCliParity"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "30ed7b6fe358db743c16f8a17ed6fb574226834fbeeb19420694f0826cda0add");
        assert_eq!(mapping["nativeSource"], "rust-native/src/runtime_system.rs");
        assert_eq!(mapping["nativeCases"], serde_json::json!(["runtime::system::shortcut_budget_tests::typo_fallback_matches_palette_and_local_cli_without_disabled_items"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn search_ranking_crosswalks_do_not_embed_live_swift_sources() {
        let source = include_str!("oracles.rs");
        let ranking_guards = source
            .split_once("fn unchanged_search_ranking_reuse_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .1
            .split_once("fn accented_uppercase_query_assertion_is_crosswalked_to_rust()")
            .unwrap()
            .0;
        let search_checks = ["SearchChecks", ".swift"].concat();
        let search_include = format!(
            "include_str!(\"../../../../Sources/RuycastApp/{search_checks}\")"
        );
        let main_swift = ["main", ".swift"].concat();
        let runner_include = format!(
            "include_str!(\"../../../../Sources/RuycastApp/{main_swift}\")"
        );
        assert!(!ranking_guards.contains(&search_include));
        assert!(!ranking_guards.contains(&runner_include));
    }

    #[test]
    fn accented_uppercase_query_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_accented_uppercase_title_query_returns_matching_command()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.accentedUppercaseQuery"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "0ee7c7c4c529fd9a875b98b0cc8c6a19a48f6cf3a1f6fa8b7ac45a0ec1765dd1");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_accented_uppercase_title_query_returns_matching_command"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn accented_uppercase_query_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn accented_uppercase_query_assertion_is_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn trimmed_uppercase_query_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_trimmed_uppercase_query_returns_matching_commands()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.trimmedUppercaseQuery"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "42804ffaac4e94b6c7576b899e9f4cb74002a2d30ac9fb76c833aa65c20c1eee");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_trimmed_uppercase_query_returns_matching_commands"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn trimmed_uppercase_query_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn trimmed_uppercase_query_assertion_is_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn embedded_nul_alias_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_embedded_nul_alias_search_returns_matching_command()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.embeddedNulAlias"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "e1e6c77a1161ae3c0410fe50ac1e5c4e475926928e6e22b6d85226a17d7fdd9b");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_embedded_nul_alias_search_returns_matching_command"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn embedded_nul_alias_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn embedded_nul_alias_assertion_is_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn missing_query_assertion_is_crosswalked_to_rust() {
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_nonempty_catalog_missing_query_returns_no_commands()"));

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.missingQuery"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "4aeec882071caf47f6f6c27890b3c32c0f93dad869fdc64470f1dce75802122a");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 1);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_nonempty_catalog_missing_query_returns_no_commands"]));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn missing_query_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn missing_query_assertion_is_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn nonempty_catalog_blank_queries_are_crosswalked_to_rust() {
        use sha2::Digest;
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.blankQueries"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "a0f320926f7fd9743b2b669d8e636bf3fb2714d102099b18f20d340b1abc523d");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_blank_queries_preserve_nonempty_catalog_order"]));
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_blank_queries_preserve_nonempty_catalog_order()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn nonempty_catalog_blank_query_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn nonempty_catalog_blank_queries_are_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn quality_ranking_assertions_are_crosswalked_to_rust() {
        use sha2::Digest;
        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Tests/LauncherCoreTests/CoreTests.swift::testPersistentIndexRankingAndUnicode.qualityRanking"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(mapping["originalSourceSHA256"], "a24ca5f72b7ec2c1f3f428ef93720450f723477157bad995a47eb4ef79ef6455");
        assert_eq!(mapping["originalScenarios"].as_array().unwrap().len(), 4);
        assert_eq!(mapping["nativeCases"], serde_json::json!(["tests::original_quality_ranking_alias_recency_and_multi_term_search"]));
        assert!(include_str!("../../../../rust/src/lib.rs").contains("fn original_quality_ranking_alias_recency_and_multi_term_search()"));

        for pipeline in ["--check-performance-pipeline", "--check-settings"] {
            let input = super::settings_sources(pipeline)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(input.1)), input.2);
            assert!(super::settings_source_bindings(pipeline).contains(&(input.0, input.2)));
        }
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn quality_ranking_crosswalk_does_not_embed_live_swift_source() {
        let source = include_str!("oracles.rs");
        let assertion = source
            .split_once("\n    fn quality_ranking_assertions_are_crosswalked_to_rust() {\n")
            .unwrap()
            .1
            .split_once("\n    #[test]")
            .unwrap()
            .0;
        let swift_include = [
            "include_str!(\"../../../../Tests/LauncherCoreTests/",
            "CoreTests.swift\")",
        ]
        .concat();
        assert!(!assertion.contains(&swift_include));
    }

    #[test]
    fn retired_utilities_helper_is_removed_and_its_scenarios_are_crosswalked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/UtilityChecks.swift";
        assert!(!root.join(retired).exists());
        let main = include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-utilities"));
        assert!(!main.contains("checkUtilities"));
        assert!(!include_str!("../ruycast-source/policy.rs")
            .lines()
            .any(|line| line == retired));

        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock["files"].get(retired).is_none());

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/UtilityChecks.swift::checkUtilities"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "a62950d4e891b79c3d311da4db2ec831bea1f4eca0744709a25be63db05bacc2"
        );
        assert_eq!(
            mapping["nativeCases"],
            serde_json::json!([
                "utilities::tests::original_utility_acceptance_scenarios_remain_in_rust",
                "utility_storage::tests::real_profile_reopen_private_mode_external_edit_and_no_password_on_disk",
                "utility_oracle::frozen_swift_utilities_and_codable_match_rust_on_owned_cases"
            ])
        );
        assert!(include_str!("../../utilities.rs")
            .contains("fn original_utility_acceptance_scenarios_remain_in_rust()"));
        assert!(include_str!("../../utility_storage.rs")
            .contains("fn real_profile_reopen_private_mode_external_edit_and_no_password_on_disk()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn retired_keyboard_helper_is_removed_and_its_assertions_are_crosswalked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/KeyboardChecks.swift";
        assert!(!root.join(retired).exists());
        let main = include_str!("../../../../Sources/RuycastApp/main.swift");
        assert!(!main.contains("--check-keyboard"));
        assert!(!main.contains("checkKeyboard"));
        assert!(!include_str!("../ruycast-source/policy.rs")
            .lines()
            .any(|line| line == retired));

        let lock: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock["files"].get(retired).is_none());

        let inventory = super::inventory("check").unwrap();
        let mapping = &inventory["retiredSwiftChecks"]
            ["Sources/RuycastApp/KeyboardChecks.swift::checkKeyboard"];
        assert_eq!(mapping["status"], "source-qualified-rust-case-crosswalk");
        assert_eq!(
            mapping["originalSourceSHA256"],
            "5cbaf9f3e805ccc44df691fafc0fdf9f2681d6854d6381e2e818115a2e4dac53"
        );
        assert_eq!(
            mapping["nativeCases"],
            serde_json::json!(["keyboard_state::tests::original_keyboard_acceptance_scenarios_remain_in_rust"])
        );
        assert_eq!(
            mapping["retiredAssertions"],
            serde_json::json!([
                "returned hardwareAcceptance status is a static requirement, not a hardware test",
                "test-recorder strings are not posted OS events; event tap, HID manager, Carbon registration, permissions and desktop input are not exercised"
            ])
        );
        assert!(include_str!("../../keyboard_state.rs")
            .contains("fn original_keyboard_acceptance_scenarios_remain_in_rust()"));
        assert_eq!(inventory["productionAccepted"], false);
    }

    #[test]
    fn python_unittest_discovery_qualifies_only_real_package_discovered_sources() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let (modules, packages) = super::discover_python_tests(repo).unwrap();
        assert!(modules.is_empty());
        assert!(packages.is_empty());
        let inventory = super::inventory("check").unwrap();
        assert_eq!(inventory["pythonModuleInventory"], serde_json::json!([]));
        assert_eq!(inventory["pythonPackageInitializers"], serde_json::json!([]));
        assert_eq!(inventory["unmappedGateIDs"], serde_json::json!([]));
        assert_eq!(
            inventory["gates"].as_array().unwrap().iter()
                .find(|gate| gate[0] == "--check-concurrency").unwrap()[1],
            "source-qualified-rust-case-crosswalk"
        );
        assert_eq!(
            inventory["retiredSwiftChecks"]["--check-concurrency"]["nativeCases"].as_array().unwrap().len(),
            8
        );
        assert_eq!(
            inventory["retiredSwiftChecks"]["--check-concurrency"]["retiredAssertions"].as_array().unwrap().len(),
            3
        );
        assert_eq!(
            inventory["gates"].as_array().unwrap().iter()
                .find(|gate| gate[0] == "python-unittest-discover-Tests-test_*.py").unwrap()[1],
            "source-qualified-empty-test-discovery"
        );
        assert_eq!(super::root_git_mapping_state(include_bytes!("compat.rs")), "source-qualified-root-bound-whitespace-check");

        let fixture = super::super::tests::Fixture::new();
        assert!(super::require_compiled_source_root(repo).is_ok());
        let tests = fixture.0.join("Tests");
        std::fs::create_dir_all(tests.join("loose")).unwrap();
        std::fs::create_dir_all(tests.join("package")).unwrap();
        std::fs::write(tests.join("test_root.py"), "").unwrap();
        std::fs::write(tests.join("loose/test_ignored.py"), "").unwrap();
        std::fs::write(tests.join("package/__init__.py"), "").unwrap();
        std::fs::write(tests.join("package/test_nested.py"), "").unwrap();
        std::fs::create_dir_all(tests.join("init_only")).unwrap();
        std::fs::write(
            tests.join("init_only/__init__.py"),
            "import unittest\nclass PackageTest(unittest.TestCase):\n def test_package(self): pass\n",
        )
        .unwrap();
        let (discovered, packages) = super::discover_python_tests(&fixture.0).unwrap();
        assert_eq!(discovered, ["Tests/package/test_nested.py", "Tests/test_root.py"]);
        assert_eq!(
            packages,
            ["Tests/init_only/__init__.py", "Tests/package/__init__.py"]
        );
        assert!(super::require_compiled_source_root(&fixture.0)
            .unwrap_err()
            .contains("Rust tool's source checkout"));
        assert_eq!(
            super::python_discovery_state(&[], &["Tests/init_only/__init__.py".to_owned()]),
            "requires-complete-transport-and-case-mapping"
        );
        assert_eq!(
            super::python_discovery_state(&discovered, &packages),
            "requires-complete-transport-and-case-mapping"
        );
    }

    #[test]
    fn public_defaults_and_structured_vision_contract_are_neutral() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../../config/default.json")).unwrap();
        let config_text = serde_json::to_string(&config).unwrap();
        assert!(!config_text.contains("Syncthing"));
        assert!(!config_text.contains("link.vault"));
        assert_eq!(config["hyperKeyEnabled"], false);
        assert_eq!(config["snippetExpansionEnabled"], false);

        let vision: serde_json::Value =
            serde_json::from_str(include_str!("../../../../docs/vision.json")).unwrap();
        assert_eq!(vision["artifact_kind"], "repo_design_vision_contract");
        let principles = vision["principles"].as_array().unwrap();
        let principle_ids = principles
            .iter()
            .map(|principle| principle["id"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        for id in ["agent-first", "json-first"] {
            assert!(principle_ids.contains(id));
        }
    }

    #[test]
    fn retired_public_contract_python_suite_is_fully_bound_to_rust() {
        let inventory = super::inventory("check").unwrap();
        assert!(!inventory["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_public_contract.py"));
        let mapping = &inventory["rustRootTestCrosswalk"]["Tests/test_public_contract.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 7);
        assert_eq!(mapping["nativeCaseByMethod"].as_object().unwrap().len(), 7);
        assert_eq!(mapping["remainingMethods"].as_array().unwrap().len(), 0);
        assert_eq!(mapping["remainingPythonCases"], 0);
        assert_eq!(
            mapping["artifactLeasePath"],
            "<repository>/.build/artifacts.lock"
        );
        assert!(mapping["artifactLeaseCallers"]["build"]
            .as_str()
            .unwrap()
            .contains("ruycast_native::artifact_lease::Lease"));
        assert!(mapping["artifactLeaseCallers"]["install"]
            .as_str()
            .unwrap()
            .contains("same Lease type"));
        let sources = [
            include_str!("oracles.rs"),
            include_str!("preparation.rs"),
            include_str!("compat.rs"),
            include_str!("../../artifact_lease_tests.rs"),
        ];
        for case in mapping["nativeCaseByMethod"].as_object().unwrap().values() {
            let case = case.as_str().unwrap();
            let name = case.rsplit("::").next().unwrap();
            assert!(
                sources
                    .iter()
                    .any(|source| source.contains(&format!("fn {name}("))),
                "missing Rust case {case}"
            );
        }
        let build = include_str!("../ruycast-build.rs");
        let install = include_str!("../ruycast-install/mod.rs");
        let lease = include_str!("../../artifact_lease.rs");
        assert!(build.contains("type BuildLock = ruycast_native::artifact_lease::Lease;"));
        assert!(build.contains("BuildLock::acquire(root)?"));
        assert!(install.contains("Lease::acquire(&options.repo)?"));
        assert!(lease.contains("root.join(\".build/artifacts.lock\")"));
        let export_policy = include_str!("../ruycast-source/policy.rs");
        assert!(!export_policy.contains("Tests/test_public_contract.py"));
    }

    #[test]
    fn no_op_launcher_core_checks_target_is_retired_from_package_and_export_policy() {
        let package = include_str!("../../../../Package.swift");
        let export_policy = include_str!("../ruycast-source/policy.rs");
        assert!(!package.contains("LauncherCoreChecks"));
        assert!(!export_policy
            .lines()
            .any(|line| line == "Tests/LauncherCoreTests/CoreTests.swift"));
        let commands = core_build_commands(
            "aarch64-apple-darwin",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert!(commands[1].windows(2).any(|pair| pair == ["--product", "RuycastApp"]));
    }

    fn core_policy_fixture() -> super::super::tests::Fixture {
        let fixture=super::super::tests::Fixture::new();let root=&fixture.0;
        for name in ["Sources/LauncherModels","Sources/LauncherCore","Sources/CRuycastCore"] {fs::create_dir_all(root.join(name)).unwrap();}
        for name in ["Sources/Legacy.swift","Package.swift","rust-toolchain.toml"] {fs::write(root.join(name),b"// owned policy fixture").unwrap();}
        fs::create_dir_all(root.join("rust-native/src/bin")).unwrap();
        fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/rust_only_policy.rs"),root.join("rust-native/src/bin/rust_only_policy.rs")).unwrap();
        let files=["Sources/Legacy.swift","Package.swift"].into_iter().map(|name|(name,digest(&root.join(name)).unwrap())).collect::<BTreeMap<_,_>>();
        fs::write(root.join("rust-native/swift-reference-lock.json"),serde_json::to_vec(&serde_json::json!({"schema":"ruycast.rust-only-reference-lock.v1","legacy_roots":["Sources"],"runtime_roots":["rust/src","rust-native/src"],"files":files})).unwrap()).unwrap();
        let binary=std::env::var_os("RUYCAST_TEST_POLICY_CLI").map(PathBuf::from).unwrap_or_else(||Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/rust-only-policy"));
        assert!(binary.is_absolute()&&binary.is_file());
        let destination=root.join(".build/oracle-policy/aarch64-apple-darwin/debug/rust-only-policy");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();fs::copy(binary,destination).unwrap();fixture
    }
    fn core_smoke_receipt(root:&Path,policy:CorePolicyEvidence)->CoreReceipt {
        let target=policy.artifact.target.as_str();
        let architecture=match target {"aarch64-apple-darwin"=>"arm64-apple-macosx","x86_64-apple-darwin"=>"x86_64-apple-macosx",_=>panic!("unexpected target")};
        let archive=format!(".build/oracle-core/{target}/debug/librustcast_core.a");
        fs::create_dir_all(root.join(&archive).parent().unwrap()).unwrap();
        fs::write(root.join(&archive),b"owned Rust archive fixture").unwrap();
        let archive_sha256=digest(&root.join(&archive)).unwrap();
        let scratch=format!(".build/oracle-swift/{archive_sha256}");
        let bin=format!("{scratch}/{architecture}/debug");
        let modulemap=format!("{bin}/CRuycastCore.build/module.modulemap");
        let compatibility_modulemap=".build/swift/debug/CRuycastCore.build/module.modulemap";
        let compatibility_archive=".build/rust/release/librustcast_core.a";
        let binary=format!("{bin}/RuycastApp");
        let mut artifacts=BTreeMap::new();
        for name in [&modulemap,compatibility_modulemap,compatibility_archive,&binary] {
            fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
            fs::write(root.join(name),b"owned nonexecutable receipt fixture").unwrap();
        }
        fs::copy(root.join(&archive),root.join(compatibility_archive)).unwrap();
        fs::copy(root.join(&modulemap),root.join(compatibility_modulemap)).unwrap();
        for name in [&modulemap,compatibility_modulemap,compatibility_archive,&binary] {
            artifacts.insert((*name).into(),digest(&root.join(name)).unwrap());
        }
        artifacts.insert(archive.clone(),archive_sha256.clone());
        artifacts.insert(policy.artifact.path.clone(),policy.artifact.sha256.clone());
        CoreReceipt{schema:"ruycast.core-build-link-smoke.v2".into(),scope:CORE_SMOKE_SCOPE.into(),sources:core_sources(root).unwrap(),artifacts,commands:core_build_commands(target,&archive_sha256).unwrap(),outcome:"Core build/link smoke passed".into(),production_accepted:false,swift_compatibility:SwiftCompatibility{modulemap:compatibility_modulemap.into(),architecture:architecture.into(),alias_target:None},policy}
    }
    #[test]
    fn core_rust_policy_snapshot_preserves_actual_calls_after_rebuild_and_fails_closed() {
        let fixture=core_policy_fixture();let root=&fixture.0;
        let lease=BuildLock::acquire_from_environment(root,&BTreeMap::new()).unwrap();
        let artifact=bind_core_policy_artifact(root,"aarch64-apple-darwin").unwrap();
        assert!(artifact.build_command.windows(2).any(|pair|pair==["--bin","rust-only-policy"]));
        assert!(artifact.build_command.iter().any(|arg|arg=="--locked"));assert!(artifact.build_command.iter().any(|arg|arg=="--offline"));
        let before=execute_core_policy(root,&lease,&artifact,"before").unwrap();
        fs::write(root.join(&artifact.build_source),b"legitimate mutable rebuild").unwrap();
        let after=execute_core_policy(root,&lease,&artifact,"after").unwrap();
        let policy=CorePolicyEvidence{artifact,before,after,inputs:core_policy_footprint(root).unwrap()};check_core_policy_evidence(root,&policy).unwrap();
        let mut altered=policy.clone();altered.after.arguments[0]=altered.artifact.build_source.clone();assert!(check_core_policy_evidence(root,&altered).is_err());
        altered=policy.clone();altered.after.result["productionReady"]=serde_json::json!(true);assert!(check_core_policy_evidence(root,&altered).is_err());
        fs::write(root.join("Sources/Legacy.swift"),b"changed legacy fixture").unwrap();
        assert!(execute_core_policy(root,&lease,&policy.artifact,"after").is_err());
        let failed:serde_json::Value=serde_json::from_slice(&fs::read(root.join(".build/receipts/pure-core-policy-after.stdout")).unwrap()).unwrap();
        assert_eq!(failed["ok"],false);assert!(!root.join(".build/receipts/pure-core-oracle.json").exists());
        assert!(check_core_policy_evidence(root,&policy).is_err());
    }
    #[test]
    fn core_build_link_smoke_receipt_requires_current_policy_source_artifact_and_both_gates() {
        let fixture=core_policy_fixture();let root=&fixture.0;
        let helper=root.join("scripts/helper.rs");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let helper_bytes=b"fn main() {}\n";
        fs::write(&helper,helper_bytes).unwrap();
        let lease=BuildLock::acquire_from_environment(root,&BTreeMap::new()).unwrap();
        let artifact=bind_core_policy_artifact(root,"aarch64-apple-darwin").unwrap();
        let before=execute_core_policy(root,&lease,&artifact,"before").unwrap();let after=execute_core_policy(root,&lease,&artifact,"after").unwrap();
        let policy=CorePolicyEvidence{artifact,before,after,inputs:core_policy_footprint(root).unwrap()};
        let mut receipt=core_smoke_receipt(root,policy);
        let path=root.join(".build/receipts/pure-core-oracle.json");
        let save=|value:&CoreReceipt|fs::write(&path,serde_json::to_vec(value).unwrap()).unwrap();
        save(&receipt);verify_core(root,&lease).unwrap();
        fs::write(&helper,b"fn main() { let changed = true; }\n").unwrap();
        let captured=tool_capture(root,&lease,&core_policy_arguments(&receipt.policy.artifact)).unwrap();
        assert!(captured.status.success());
        assert_eq!(validation::rust_policy_result(&captured.stdout).unwrap(),receipt.policy.after.result);
        assert_eq!(verify_core(root,&lease).unwrap_err(),"Core policy input path/type/content footprint changed.");
        fs::write(&helper,helper_bytes).unwrap();
        verify_core(root,&lease).unwrap();
        let good=serde_json::to_value(&receipt).unwrap();
        for mutation in ["command-scratch", "outcome", "scope", "missing-modulemap", "wrong-binary"] {
            let mut changed=good.clone();
            match mutation {
                "command-scratch"=>changed["commands"][1][3]=serde_json::json!(".build/oracle-swift/unbound"),
                "outcome"=>changed["outcome"]=serde_json::json!("Core behavior checks passed"),
                "scope"=>changed["scope"]=serde_json::json!("behavior and ABI call acceptance"),
                "missing-modulemap"=>{changed["artifacts"].as_object_mut().unwrap().remove(".build/swift/debug/CRuycastCore.build/module.modulemap");},
                _=>{let artifacts=changed["artifacts"].as_object_mut().unwrap();let binary=artifacts.keys().find(|name|name.ends_with("/RuycastApp")).unwrap().clone();artifacts.remove(&binary);},
            }
            fs::write(&path,serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(verify_core(root,&lease).is_err(),"{mutation}");
        }
        save(&receipt);
        let policy=receipt.policy.clone();
        let compat_archive=root.join(".build/rust/release/librustcast_core.a");
        fs::write(&compat_archive,b"changed compatibility archive").unwrap();
        assert!(verify_core(root,&lease).is_err());
        receipt=core_smoke_receipt(root,policy);save(&receipt);verify_core(root,&lease).unwrap();
        for (name,allowed) in [("rust-native/NewHost.swift",false),("rust-native/web/runtime.js",false),("rust-native/build.rs",true),("rust-native/__pycache__/hidden.py",false),("rust/src/__pycache__/host.py",false),("Sources/__pycache__/hidden.swift",false)] {
            let path=root.join(name);fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(&path,b"// own footprint regression fixture").unwrap();
            assert!(verify_core(root,&lease).is_err(),"{name}");
            let captured=tool_capture(root,&lease,&core_policy_arguments(&receipt.policy.artifact)).unwrap();
            let result:serde_json::Value=serde_json::from_slice(&captured.stdout).unwrap();
            assert_eq!(result["ok"],allowed,"{name}");assert_eq!(captured.status.success(),allowed,"{name}");
            fs::remove_file(&path).unwrap();let mut parent=path.parent().unwrap();
            while parent!=root {if fs::remove_dir(parent).is_err(){break;}parent=parent.parent().unwrap();}
            verify_core(root,&lease).unwrap();
        }
        for old in ["ruycast.core-build-link-smoke.v1", "ruycast.pure-core-oracle.v2", "ruycast.pure-core-oracle.v3", "ruycast.pure-core-oracle.v4", "ruycast.pure-core-oracle.v5", "ruycast.pure-core-oracle.v6"] {
            receipt.schema=old.into();save(&receipt);assert_eq!(verify_core(root,&lease).unwrap_err(), "Core build/link smoke receipt is invalid or stale.");
        }
        receipt.schema="ruycast.core-build-link-smoke.v2".into();save(&receipt);
        let mut missing=serde_json::to_value(&receipt).unwrap();missing.as_object_mut().unwrap().remove("policy");
        fs::write(&path,serde_json::to_vec(&missing).unwrap()).unwrap();assert!(verify_core(root,&lease).is_err());
        save(&receipt);fs::write(root.join("rust-native/src/main.rs"),b"changed bound runtime fixture").unwrap();assert!(verify_core(root,&lease).is_err());
    }
    #[test]
    fn core_rust_policy_footprint_binds_tooling_and_keeps_cargo_pruning_and_link_refusal() {
        use std::os::unix::fs::symlink;
        let fixture=core_policy_fixture();let root=&fixture.0;
        for name in ["Tests","scripts","bench","rust/tests","rust/scripts","rust-native/tests","rust-native/scripts"] {
            let path=root.join(name).join("helper.rs");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path,b"// owned helper source").unwrap();
            let values=core_policy_footprint(root).unwrap();
            assert_eq!(values[&format!("{name}/helper.rs")].sha256,Some(digest(&path).unwrap()));
        }
        let crate_root=root.join("bench/fixture");
        fs::create_dir_all(crate_root.join("target")).unwrap();
        fs::write(crate_root.join("Cargo.toml"),b"# owned crate metadata").unwrap();
        let ds_store=root.join("scripts/.DS_Store");
        fs::write(&ds_store,b"owned ignored metadata").unwrap();
        let before=core_policy_footprint(root).unwrap();
        fs::write(crate_root.join("target/owned.py"),b"// owned generated output").unwrap();
        fs::write(&ds_store,b"changed ignored metadata").unwrap();
        assert_eq!(serde_json::to_value(&before).unwrap(),serde_json::to_value(core_policy_footprint(root).unwrap()).unwrap());
        // A target directory without Cargo metadata is source, not generated output.
        let nested=root.join("scripts/target/helper.rs");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&nested,b"// owned nested source").unwrap();
        assert!(core_policy_footprint(root).unwrap().contains_key("scripts/target/helper.rs"));
        symlink(crate_root.join("target"),root.join("scripts/linked-output")).unwrap();
        assert!(core_policy_footprint(root).is_err());
        fs::remove_file(root.join("scripts/linked-output")).unwrap();
        fs::remove_dir_all(root.join("rust-native/scripts")).unwrap();
        fs::create_dir(root.join("owned-link-target")).unwrap();
        symlink(root.join("owned-link-target"),root.join("rust-native/scripts")).unwrap();
        assert!(core_policy_footprint(root).is_err());
    }
    #[test]
    fn core_rust_policy_snapshot_artifact_and_build_recipe_cannot_be_rebound() {
        use std::os::unix::fs::PermissionsExt;
        let fixture=core_policy_fixture();let root=&fixture.0;
        let artifact=bind_core_policy_artifact(root,"aarch64-apple-darwin").unwrap();
        let mut changed=artifact.clone();changed.build_command[0]="python3".into();assert!(check_core_policy_artifact(root,&changed).is_err());
        changed=artifact.clone();changed.path=changed.build_source.clone();assert!(check_core_policy_artifact(root,&changed).is_err());
        let path=root.join(&artifact.path);fs::set_permissions(&path,fs::Permissions::from_mode(0o700)).unwrap();fs::write(&path,b"changed own snapshot").unwrap();
        assert!(check_core_policy_artifact(root,&artifact).is_err());
    }

    #[test]
    fn native_profile_crosswalk_binds_actual_rust_case_and_inputs() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-visual-assets").unwrap();
        assert!(gate.required_native_steps.contains(&"native-profile-process"));
        assert!(gate.native_cases.contains(&"native_process::loaded_config_alias_unicode_disabled_and_native_labels"));
        let inputs = settings_sources(gate.id);
        for path in ["rust-native/tests/native_process.rs", "rust-native/tests/support/runtime_host.rs", "rust-native/tests/support/status_rpc.rs", "rust-native/tests/support/child_output.rs", "config/default.json"] {
            let entries = inputs.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
            assert_eq!(entries.len(), 1, "{path}");
            assert_eq!(format!("{:x}", Sha256::digest(entries[0].1)), entries[0].2, "{path}");
        }
        let test = std::str::from_utf8(inputs.iter().find(|entry| entry.0 == "rust-native/tests/native_process.rs").unwrap().1).unwrap();
        assert!(test.contains("fn loaded_config_alias_unicode_disabled_and_native_labels("));
    }
    #[test]
    fn render_updates_provenance_keeps_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-render-updates").unwrap();
        assert_eq!(gate.required_native_steps, &["render-updates-process"]);
        assert_eq!(gate.native_cases, &["ui_completion::render_updates"]);
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 13);
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/")));
        let bindings = settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 14);
        for (path, _, hash) in inputs {
            assert!(bindings.contains(&(*path, *hash)), "missing Rust source binding {path}");
        }
        assert!(bindings.contains(&(gate.original_source, RENDER_UPDATES_ORACLE_SHA256)));
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let value = inventory("settings").unwrap();
        assert_eq!(value["productionAccepted"], false);
        assert!(!value["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn retired_render_updates_swift_route_keeps_shared_interaction_routes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        let interactions = std::fs::read_to_string(root.join("Sources/RuycastApp/InteractionChecks.swift")).unwrap();
        assert!(!main.contains("--check-render-updates"));
        assert!(!interactions.contains("checkRenderedUpdates"));
        assert!(main.contains("--check-interactions"));
        assert!(main.contains("output(try checkInteractions())"));
        assert!(main.contains("--render-features"));
        assert!(main.contains("try renderFeatures("));
        assert!(interactions.contains("func checkInteractions"));
        assert!(interactions.contains("func renderFeatures"));
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-render-updates").unwrap();
        assert_eq!(gate.required_native_steps, &["render-updates-process"]);
        assert_eq!(gate.native_cases, &["ui_completion::render_updates"]);
        let inventory = inventory("check").unwrap();
        let retired = &inventory["retiredSwiftChecks"]["Sources/RuycastApp/InteractionChecks.swift::checkRenderedUpdates"];
        assert_eq!(retired["originalSourceSHA256"], RENDER_UPDATES_ORACLE_SHA256);
        assert_eq!(retired["status"], "source-qualified-rust-case-crosswalk; Swift check route retired");
        assert_eq!(retired["scope"], "Only the --check-render-updates caller and checkRenderedUpdates helper are retired. The shared InteractionChecks.swift file, --check-interactions and --render-features routes remain active.");
        assert_eq!(retired["nativeSource"], "rust-native/examples/ui_completion.rs");
        assert_eq!(retired["nativeSteps"], serde_json::json!(["render-updates-process"]));
        assert_eq!(retired["nativeCases"], serde_json::json!(["ui_completion::render_updates"]));
        assert_eq!(retired["swiftOriginalCaptures"], 12);
        assert_eq!(retired["rustRenderedCaptures"], 26);
        assert_eq!(retired["productionAccepted"], false);
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        for (path, source) in [("Sources/RuycastApp/main.swift", &main), ("Sources/RuycastApp/InteractionChecks.swift", &interactions)] {
            assert_eq!(lock["files"][path], format!("{:x}", Sha256::digest(source.as_bytes())));
        }
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        let interaction_rows = roster.lines().filter(|line| line.contains("\"frozen-current/Sources/RuycastApp/InteractionChecks.swift\"")).collect::<Vec<_>>();
        assert_eq!(interaction_rows.len(), 2);
        let current_hash = format!("{:x}", Sha256::digest(interactions.as_bytes()));
        let mut current_lines = Vec::new();
        for row in interaction_rows {
            let encoded = row.trim().trim_end_matches(',').trim_start_matches('(').trim_end_matches(')');
            let fields: Vec<String> = serde_json::from_str(&format!("[{encoded}]")).unwrap();
            assert_eq!(fields[1], current_hash);
            let (line, expected) = fields[2].strip_prefix("line ").unwrap().split_once(": ").unwrap();
            let line = line.parse::<usize>().unwrap();
            current_lines.push(line);
            assert_eq!(interactions.lines().nth(line - 1).unwrap().trim(), expected);
        }
        current_lines.sort_unstable();
        assert_eq!(current_lines, [14, 124]);
        let reference_rows = roster.lines().filter(|line| line.contains("\"reference-0.8.26/Sources/RuycastApp/InteractionChecks.swift\"")).collect::<Vec<_>>();
        assert_eq!(reference_rows, [
            "(\"reference-0.8.26/Sources/RuycastApp/InteractionChecks.swift\",\"7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858\",\"line 16: _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)\"),",
            "(\"reference-0.8.26/Sources/RuycastApp/InteractionChecks.swift\",\"7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858\",\"line 126: _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)\"),",
            "(\"reference-0.8.26/Sources/RuycastApp/InteractionChecks.swift\",\"7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858\",\"line 148: _=NSApplication.shared;NSApp.setActivationPolicy(.prohibited)\"),",
            "(\"reference-0.8.26/Sources/RuycastApp/InteractionChecks.swift\",\"7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858\",\"line 153: let panel=NSPanel(contentRect:NSRect(x:0,y:0,width:760,height:490),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)\"),"
        ]);
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        for path in ["Sources/RuycastApp/main.swift", "Sources/RuycastApp/InteractionChecks.swift"] {
            assert_eq!(public["files"][path], format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())));
        }
    }
    #[test]
    fn window_search_provenance_keeps_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-window-search").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "system-search-cli-process", "system-search-sdk"]);
        assert_eq!(gate.native_cases.len(), 18);
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 34);
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("rust/") || *path == "rust-toolchain.toml"));
        let bindings = settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 35);
        for (path, _, hash) in inputs {
            assert!(bindings.contains(&(*path, *hash)), "missing current window-search binding {path}");
        }
        assert!(bindings.contains(&(gate.original_source, WINDOW_SEARCH_ORACLE_SHA256)));
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let value = inventory("settings").unwrap();
        assert_eq!(value["productionAccepted"], false);
        assert!(!value["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn visual_assets_provenance_keeps_all_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-visual-assets").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "native-profile-process"]);
        assert_eq!(gate.native_cases.len(), 9);
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 10);
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("config/")));
        let bindings = settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 11);
        for (path, _, hash) in inputs {
            assert!(bindings.contains(&(*path, *hash)), "missing current source binding {path}");
        }
        assert!(bindings.contains(&(
            gate.original_source,
            "4bd5994c6cc34a504e3f1576a0a21ca903636ed8293986ccde561a4c3b7304f0",
        )));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(!root.join("Sources/RuycastApp/VisualAssetChecks.swift").exists());
        let product_icons = std::fs::read_to_string(root.join("Sources/RuycastApp/WebsiteIcons.swift")).unwrap();
        assert!(product_icons.contains("WebsiteIcons"));
        let rust_icons = std::fs::read_to_string(root.join("rust-native/src/website_icons.rs")).unwrap();
        assert!(rust_icons.contains("fn origin_and_candidate_privacy"));
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("if args.first==\"--check-visual-assets\""));
        assert!(!main.contains("if args.first==\"--render-visual-assets\""));
        assert!(!main.contains("if args.first==\"--render-website-icons\""));
        let source_policy = std::fs::read_to_string(root.join("rust-native/src/bin/ruycast-source/policy.rs")).unwrap();
        assert!(!source_policy.contains("Sources/RuycastApp/VisualAssetChecks.swift"));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get("Sources/RuycastApp/VisualAssetChecks.swift").is_none());
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        assert!(!roster.contains("frozen-current/Sources/RuycastApp/VisualAssetChecks.swift"));
        assert!(roster.contains("reference-0.8.26/Sources/RuycastApp/VisualAssetChecks.swift"));
        let public_manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        assert!(public_manifest["files"].get("Sources/RuycastApp/VisualAssetChecks.swift").is_none());
        let check_inventory = inventory("check").unwrap();
        let retired = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::checkVisualAssets"];
        assert_eq!(retired["originalSourceSHA256"], "4bd5994c6cc34a504e3f1576a0a21ca903636ed8293986ccde561a4c3b7304f0");
        assert_eq!(retired["status"], "source-qualified-rust-case-crosswalk; Swift check route retired");
        assert_eq!(retired["nativeCases"], serde_json::to_value(gate.native_cases).unwrap());
        let retired_preview = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::renderVisualAssets"];
        assert_eq!(retired_preview["originalSourceSHA256"], "e0b3a9b53aa1041019cc10d7214e42727ed0e2f79162c6144afc35be2725ba78");
        assert_eq!(retired_preview["status"], "source-retired; unconsumed development preview");
        assert_eq!(retired_preview["retiredOutputs"], serde_json::json!(["shortcuts-dark.png", "shortcuts-light.png"]));
        let retired_icons = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/VisualAssetChecks.swift::renderWebsiteIcons"];
        assert_eq!(retired_icons["originalSourceSHA256"], "e73210b69d30795488372866e45036770b3d6e5355c11b48d31a051a857ed70e");
        assert_eq!(retired_icons["status"], "source-retired; unconsumed development preview");
        assert_eq!(retired_icons["retiredOutputs"], serde_json::json!(["sites-dark.png", "sites-light.png"]));
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let value = inventory("settings").unwrap();
        assert_eq!(value["productionAccepted"], false);
        assert!(!value["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn retired_command_icon_sheet_preview_keeps_shared_glyph_callers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        let glyph = std::fs::read_to_string(root.join("Sources/RuycastApp/CommandGlyph.swift")).unwrap();
        assert!(!main.contains("--render-icons"));
        assert!(!main.contains("CommandIconSheet"));
        assert!(main.contains("--check-palette"));
        assert!(main.contains("CommandGlyph(kind:\"window\""));
        assert!(!glyph.contains("CommandIconSheet"));
        assert!(glyph.contains("struct CommandGlyph:View"));
        let website_icons = std::fs::read_to_string(root.join("Sources/RuycastApp/WebsiteIcons.swift")).unwrap();
        assert!(website_icons.contains("CommandGlyph(kind:command.kind,target:command.target)"));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        for (path, source) in [("Sources/RuycastApp/main.swift", &main), ("Sources/RuycastApp/CommandGlyph.swift", &glyph)] {
            assert_eq!(lock["files"][path], format!("{:x}", Sha256::digest(source.as_bytes())));
        }
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        let main_rows = roster.lines().filter(|line| line.contains("\"frozen-current/Sources/RuycastApp/main.swift\"")).collect::<Vec<_>>();
        assert_eq!(main_rows.len(), 12);
        let current_hash = format!("{:x}", Sha256::digest(main.as_bytes()));
        for row in main_rows {
            let encoded = row.trim().trim_end_matches(',').trim_start_matches('(').trim_end_matches(')');
            let fields: Vec<String> = serde_json::from_str(&format!("[{encoded}]")).unwrap();
            assert_eq!(fields[0], "frozen-current/Sources/RuycastApp/main.swift");
            assert_eq!(fields[1], current_hash);
            let (line, expected) = fields[2].strip_prefix("line ").unwrap().split_once(": ").unwrap();
            let line = line.parse::<usize>().unwrap();
            assert_eq!(main.lines().nth(line - 1).unwrap().trim(), expected, "roster callsite line {line}");
        }
        assert!(roster.contains("reference-0.8.26/Sources/RuycastApp/main.swift"));
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        for path in ["Sources/RuycastApp/main.swift", "Sources/RuycastApp/CommandGlyph.swift"] {
            assert_eq!(public["files"][path], format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())));
        }
        let inventory = inventory("check").unwrap();
        let retired = &inventory["retiredSwiftChecks"]["Sources/RuycastApp/CommandGlyph.swift::CommandIconSheet"];
        assert_eq!(retired["originalSourceSHA256"], "f9b6b16cf6e3ee41c58835ba71cdea0270a4adb7d528498e6865e2dfc8eb9feb");
        assert_eq!(retired["status"], "source-retired; unconsumed development preview");
    }
    #[test]
    fn owned_shortcut_child_fixture_is_bound_to_every_system_search_test_consumer() {
        for id in ["--check-feedback", "--check-menu-actions", "--check-window-search", "--check-apple-shortcuts", "--check-browser-tabs"] {
            let inputs = settings_sources(id);
            for path in ["rust-native/src/system_search/tests.rs", "rust-native/tests/fixtures/shortcut_child.rs"] {
                let entries = inputs.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
                assert_eq!(entries.len(), 1, "{id}: {path}");
                assert_eq!(format!("{:x}", Sha256::digest(entries[0].1)), entries[0].2, "{id}: {path}");
                assert!(settings_source_bindings(id).contains(&(path, entries[0].2)), "{id}: {path}");
            }
        }
    }
    #[test]
    fn script_result_basic_binds_current_sources_and_route() {
        use sha2::Digest;
        let inputs = settings_sources("--check-apple-shortcuts");
        for path in ["rust-native/tests/script_result_basic_process.rs", "rust-native/tests/script_result_native_process.rs", "rust-native/src/bin/private_results_fixture.rs"] {
            let (_, bytes, expected) = inputs.iter().find(|entry| entry.0 == path).unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(bytes)), *expected, "{path}");
        }
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-apple-shortcuts").unwrap();
        assert!(gate.required_native_steps.contains(&"script-result-process"));
        assert!(gate.required_native_steps.contains(&"script-result-native-process"));
    }
    #[test]
    fn library_checkbox_crosswalk_binds_current_sources_and_cases() {
        use sha2::{Digest, Sha256};
        for id in ["--check-feedback", "--check-actions-menu", "--check-interactions", "--check-settings"] {
            let inputs = settings_sources(id);
            let matches: Vec<_> = inputs.iter().filter(|(path, _, _)| path.ends_with("library_checkbox_process.rs") || path.ends_with("library_drafts_process.rs") || path.ends_with("library_navigation_process.rs") || path.ends_with("library_lifecycle_process.rs")).collect();
            assert_eq!(matches.len(), 4, "{id}");
            for (path, bytes, hash) in matches {
                assert_eq!(format!("{:x}", Sha256::digest(bytes)), *hash, "{id}: {path}");
            }
        }
        let gate = SETTINGS.iter().find(|g| g.id == "--check-settings").unwrap();
        assert!(gate.required_native_steps.contains(&"library-checkbox-process"));
        assert!(gate.required_native_steps.contains(&"library-drafts-process"));
        assert!(gate.required_native_steps.contains(&"library-navigation-process"));
        let lifecycle = include_str!("../../../tests/library_lifecycle_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_lifecycle_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(lifecycle.contains(&format!("fn {case}(")), "{case}"); }
        let navigation = include_str!("../../../tests/library_navigation_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_navigation_process::")).collect();
        assert_eq!(cases.len(), 4);
        for case in cases { assert!(navigation.contains(&format!("fn {case}(")), "{case}"); }
        let drafts = include_str!("../../../tests/library_drafts_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_drafts_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(drafts.contains(&format!("fn {case}(")), "{case}"); }
        let source = include_str!("../../../tests/library_checkbox_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_checkbox_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(source.contains(&format!("fn {case}(")), "{case}"); }
    }
    #[test]
    fn creation_crosswalk_uses_real_rust_cases_and_shared_actions_fixture() {
        use sha2::Digest;
        let source = include_str!("../../../tests/creation_process.rs");
        for (id,case) in [("--check-feedback","empty_results_keep_all_three_creation_actions_available"),("--check-interactions","external_changes_survive_and_identity_collision_never_overwrites")] {
            let gate=super::SETTINGS.iter().find(|gate|gate.id==id).unwrap();
            assert!(gate.native_cases.contains(&format!("creation_process::{case}").as_str()));
            assert!(source.contains(&format!("fn {case}()")));
        }
        for id in ["--check-feedback","--check-actions-menu","--check-interactions"] {
            let entries=super::settings_sources(id);
            for path in ["rust-native/tests/creation_process.rs","rust-native/tests/support/native_actions.rs"] {
                let matches=entries.iter().filter(|entry|entry.0==path).collect::<Vec<_>>();
                assert_eq!(matches.len(),1,"{id}: {path}");
                assert_eq!(format!("{:x}",sha2::Sha256::digest(matches[0].1)),matches[0].2);
            }
            assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/creation_process.py"));
        }
    }
    #[test]
    fn analytics_dashboard_export_crosswalk_uses_existing_rust_cases() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate|gate.id=="--check-analytics").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "analytics-runtime-process", "analytics-cast-reference", "dashboard-presentation-process", "native-dashboard-dialog-process", "dashboard-export-process", "startup-process"]);
        assert_eq!(gate.native_cases.len(), 26);
        let entries=super::settings_sources(gate.id);
        assert_eq!(entries.len(), 26);
        assert!(entries.iter().all(|(path, _, _)| path.starts_with("rust-native/")));
        let bindings = super::settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 27);
        for (path, _, hash) in entries {
            assert!(bindings.contains(&(*path, *hash)), "missing current analytics binding {path}");
        }
        assert!(bindings.contains(&(
            gate.original_source,
            "b7e97f390de005268b1a7aa0ceeda22ca165bb2a84f8fcf1917858c4dcb73876",
        )));
        assert_eq!(super::settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = super::inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
        let status_rpc = entries.iter().find(|entry| entry.0 == "rust-native/tests/support/status_rpc.rs").unwrap();
        assert_eq!(format!("{:x}", sha2::Sha256::digest(status_rpc.1)), status_rpc.2);
        assert!(std::str::from_utf8(status_rpc.1).unwrap().contains("benchmark_host::rpc::{connect, request}"));
        let rpc = entries.iter().find(|entry| entry.0 == "rust-native/src/benchmark_host/rpc.rs").unwrap();
        assert_eq!(rpc.2, "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9");
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn connect("));
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn request("));
        let dialog = entries.iter().find(|entry| entry.0=="rust-native/tests/native_dashboard_dialog_process.rs").unwrap();
        assert_eq!(format!("{:x}",sha2::Sha256::digest(dialog.1)),dialog.2);
        let dialog_source = std::str::from_utf8(dialog.1).unwrap();
        for name in ["owned_save_dialog_takes_current_data_and_cancel_writes_nothing", "retired_controls_and_dialog_cannot_mutate_a_reopened_dashboard", "management_routing_controls_custom_retention_and_reset", "repeated_confirmation_retires_old_queued_reset_callback", "standard_builtin_cli_native_sources_usage_logs_and_override", "builtin_input_validation_and_native_prompt_do_not_log_private_values", "actual_dashboard_renders_themes_widths_and_preserves_scroll", "populated_paused_errors_confirmation_roles_values_and_local_key_loop"] {
            assert!(gate.native_cases.contains(&format!("native_dashboard_dialog_process::{name}").as_str()));
            assert!(dialog_source.contains(&format!("fn {name}()")));
        }
        assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/dashboard_export_process.py"));
        let check_inventory = super::inventory("check").unwrap();
        let retired = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.staleLoadRetention"];
        assert_eq!(retired["originalSourceSHA256"], "80a8968e86e157a262368fe98484335d7022e6fc61591edb6b4be5e7eabedecd");
        assert_eq!(retired["nativeCases"], serde_json::json!(["analytics::tests::opening_prunes_events_outside_configured_retention"]));
        let routing = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.managementRouting"];
        assert_eq!(routing["originalSourceSHA256"], "926b61901b45c6623d0c68a60379ec3926f71275aadae47ce5978fb6b36ad65b");
        assert_eq!(routing["nativeCases"], serde_json::json!(["native_dashboard_dialog_process::management_routing_controls_custom_retention_and_reset"]));
        let workload = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.concurrentWorkload"];
        assert_eq!(workload["originalSourceSHA256"], "64fca861ac1a01105517d4e4c16546deccc9bf054f16bf29b1e62af34f2c62c9");
        assert_eq!(workload["nativeCases"], serde_json::json!(["analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention"]));
        let detached_store = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.detachedStoreFixture"];
        assert_eq!(detached_store["originalSourceSHA256"], "24e8731e4bf1da3622e28ef982f3ad085b374a5135bb0ac3fa16b6348c5671ab");
        assert_eq!(detached_store["nativeCases"], serde_json::json!(["analytics::tests::completion_survives_pause_but_reset_retires_old_ticket_and_private_export", "analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention"]));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(!root.join("Sources/RuycastApp/AnalyticsChecks.swift").exists());
        let swift_main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!swift_main.contains("if args.first==\"--check-analytics\""));
        let source_policy = std::fs::read_to_string(root.join("rust-native/src/bin/ruycast-source/policy.rs")).unwrap();
        assert!(!source_policy.contains("Sources/RuycastApp/AnalyticsChecks.swift"));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get("Sources/RuycastApp/AnalyticsChecks.swift").is_none());
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        assert!(!roster.contains("frozen-current/Sources/RuycastApp/AnalyticsChecks.swift"));
        let whole_file = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::file"];
        assert_eq!(whole_file["originalSourceSHA256"], "e86146a3b3d15a10c652d34d9158b93efce730157b175407f7a0852bcce93f55");
        assert_eq!(whole_file["status"], "source-retired; development-only SwiftUI PNG contract retired");
        assert_eq!(whole_file["nativeCases"].as_array().unwrap().len(), 6);
        let dry_run = &check_inventory["retiredSwiftChecks"]["Sources/RuycastApp/AnalyticsChecks.swift::checkAnalytics.dryRunFixture"];
        assert_eq!(dry_run["originalSourceSHA256"], "189f08d3eda7409b3f5487c3c7d2cb84d66c56b7c1e753e825511b951f4ad25f");
        assert_eq!(dry_run["nativeCases"], serde_json::json!(["startup_process::analytics_same_command_tracks_actual_cli_launcher_and_owned_carbon_callback", "startup_process::analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch"]));
        assert!(dialog_source.contains("let palette = h.rpc(&json!({\"op\":\"native.test\",\"action\":\"read\"}));"));
        assert!(dialog_source.contains("assert_eq!(palette[\"result\"][\"windowShown\"], false);"));
        let sources=entries.iter().filter(|entry|entry.0=="rust-native/tests/dashboard_export_process.rs").collect::<Vec<_>>();
        assert_eq!(sources.len(),1);
        assert_eq!(format!("{:x}",sha2::Sha256::digest(sources[0].1)),sources[0].2);
        let source=std::str::from_utf8(sources[0].1).unwrap();
        let cases=gate.native_cases.iter().filter_map(|case|case.strip_prefix("dashboard_export_process::")).collect::<Vec<_>>();
        assert_eq!(cases.len(),5);
        for case in cases {assert!(source.contains(&format!("fn {case}()")));}
    }
    #[test]
    fn retired_snippet_editing_helper_preserves_rust_crosswalk_and_provenance() {
        use sha2::Digest;
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired="Sources/RuycastApp/SnippetEditingChecks.swift";
        assert!(!root.join(retired).exists());
        let caller=include_str!("../../../../Sources/RuycastApp/SettingsChecks.swift");
        assert!(!caller.contains("checkSnippetEditing"));
        assert!(!caller.contains("snippetEditing"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line|line==retired));
        let lock:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/SettingsChecks.swift"],format!("{:x}",Sha256::digest(caller.as_bytes())));

        let gate=super::SETTINGS.iter().find(|gate|gate.id=="--check-settings").unwrap();
        assert!(gate.native_cases.contains(&"ui_completion::snippet_links"));
        let historical=("Sources/RuycastApp/SnippetEditingChecks.swift",super::SNIPPET_EDITING_ORACLE_SHA256);
        assert_eq!(super::settings_additional_provenance_sources(gate.id),&[historical]);
        assert!(super::settings_source_bindings(gate.id).contains(&historical));
        assert!(super::settings_source_bindings(gate.id).contains(&(gate.original_source,super::SETTINGS_ORACLE_SHA256)));
        let inventory=super::inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"],false);
        let mapping=&inventory["retiredSwiftChecks"]["Sources/RuycastApp/SnippetEditingChecks.swift::checkSnippetEditing"];
        assert_eq!(mapping["originalSourceSHA256"],"adfae471c755454375ff7af2b1c99108ab08bd4a9c2a50556809e2dad5f0b99e");
        assert_eq!(mapping["status"],"source-qualified-rust-case-crosswalk; Swift helper retired");
        assert_eq!(mapping["productionAccepted"],false);
        let crosswalk=mapping["assertionCrosswalk"].as_object().unwrap();
        assert_eq!(crosswalk.len(),8);
        let sources=[
            ("ui_completion::","rust-native/examples/ui_completion.rs",include_str!("../../../examples/ui_completion.rs")),
            ("snippet_probe::","rust-native/src/bin/snippet_probe.rs",include_str!("../snippet_probe.rs")),
            ("snippet_process::","rust-native/tests/snippet_process.rs",include_str!("../../../tests/snippet_process.rs")),
            ("keyword_rich_process::","rust-native/tests/keyword_rich_process.rs",include_str!("../../../tests/keyword_rich_process.rs")),
        ];
        let mut cases=std::collections::BTreeSet::new();
        for names in crosswalk.values() {
            for name in names.as_array().unwrap() {
                let case=name.as_str().unwrap();
                assert!(gate.native_cases.contains(&case),"retired snippet case not in settings crosswalk: {case}");
                let (prefix,path,source)=sources.iter().find(|(prefix,_,_)|case.starts_with(prefix)).unwrap();
                let function=case.strip_prefix(prefix).unwrap().rsplit("::").next().unwrap();
                let present=if *prefix=="snippet_process::" {
                    source.lines().any(|line|line.trim()==format!("method!({function});"))
                } else {
                    source.contains(&format!("fn {function}("))
                };
                assert!(present,"missing current snippet assertion: {case}");
                assert!(super::settings_sources(gate.id).iter().any(|(input,_,_)|input==path),"unbound snippet assertion source: {path}");
                cases.insert(case);
            }
        }
        assert_eq!(cases.len(),6);
    }

    #[test]
    fn settings_preferences_crosswalk_uses_four_current_rust_cases() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|case|case.strip_prefix("settings_preferences_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(),4);
        for id in ["--check-feedback","--check-actions-menu","--check-interactions","--check-settings"] {
            let entries=super::settings_sources(id);
            let source=entries.iter().filter(|entry|entry.0=="rust-native/tests/settings_preferences_process.rs").collect::<Vec<_>>();
            assert_eq!(source.len(),1,"{id}");
            assert_eq!(format!("{:x}",sha2::Sha256::digest(source[0].1)),source[0].2);
            let text=std::str::from_utf8(source[0].1).unwrap();
            for name in &names {assert!(text.contains(&format!("fn {name}()")));}
            assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/settings_preferences_process.py"));
        }
    }
    #[test]
    fn settings_provenance_preserves_inputs_routes_and_qualified_status() {
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let inputs = super::settings_sources(gate.id);
        let provenance = (gate.original_source, super::SETTINGS_ORACLE_SHA256);
        let additional = super::settings_additional_provenance_sources(gate.id);
        let bindings = super::settings_source_bindings(gate.id);

        assert_eq!(gate.native_cases.len(), 98);
        assert_eq!(gate.required_native_steps.len(), 26);
        assert_eq!(inputs.len(), 47);
        assert_eq!(bindings.len(), 49);
        assert_eq!(bindings.iter().collect::<std::collections::BTreeSet<_>>().len(), 49);
        assert_eq!(super::settings_provenance_source(gate.id), Some(provenance));
        assert!(bindings.contains(&provenance));
        assert_eq!(additional, &[("Sources/RuycastApp/SnippetEditingChecks.swift", "adfae471c755454375ff7af2b1c99108ab08bd4a9c2a50556809e2dad5f0b99e")]);
        assert!(bindings.contains(&additional[0]));
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/SnippetEditingChecks.swift"));
        assert_eq!(super::settings_additional_provenance_sources("--check-feedback"), &[("Sources/RuycastApp/SystemChecks.swift", "c70cc5149a16a43102cc8a853bfb5a38bc6a9b3b542b4f6b7388339f1737ffa4"), ("Sources/RuycastApp/ActionMenuChecks.swift", "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f"), ("Sources/RuycastApp/InteractionChecks.swift", "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858")]);
        assert_eq!(super::settings_additional_provenance_sources("--check-actions-menu"), &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/InteractionChecks.swift", "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858")]);
        assert_eq!(super::settings_additional_provenance_sources("--check-interactions"), &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/ActionMenuChecks.swift", "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f")]);
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("rust/") || *path == "rust-toolchain.toml"));
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current settings binding: {path}");
        }
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        let planned_steps = plan["steps"].as_array().unwrap();
        for name in gate.required_native_steps {
            assert!(planned_steps.iter().any(|step| step["name"] == *name), "missing settings route: {name}");
        }
        let preferences = gate.native_cases.iter().filter_map(|case| case.strip_prefix("settings_preferences_process::")).collect::<Vec<_>>();
        assert_eq!(preferences.len(), 4);
        let source = std::str::from_utf8(inputs.iter().find(|(path, _, _)| *path == "rust-native/tests/settings_preferences_process.rs").unwrap().1).unwrap();
        for name in preferences {
            assert!(source.contains(&format!("fn {name}()")), "missing preferences case: {name}");
        }
        assert_eq!(super::settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = super::inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
        assert_eq!(inventory["qualifiedCaseSourceBindings"][gate.id].as_object().unwrap().len(), 49);
    }
    #[test]
    fn general_keyboard_crosswalk_names_both_rust_tests() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let cases = gate.native_cases.iter().filter_map(|name|name.strip_prefix("settings_keyboard_process::")).collect::<Vec<_>>();
        assert_eq!(cases.len(), 2);
        let entries = super::settings_sources("--check-settings");
        let source = entries.iter().find(|entry|entry.0 == "rust-native/tests/settings_keyboard_process.rs").unwrap();
        assert_eq!(format!("{:x}",sha2::Sha256::digest(source.1)),source.2);
        let text = std::str::from_utf8(source.1).unwrap();
        for case in cases { assert!(text.contains(&format!("fn {case}()"))); }
        assert!(!gate.native_cases.iter().any(|name|name.starts_with("settings_keyboard_process.")));
    }
    #[test]
    fn settings_keyboard_crosswalk_uses_current_rust_suite_and_owned_sdk() {
        use sha2::Digest;
        let source = include_str!("../../../tests/native_keyboard_process.rs");
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|name| name.strip_prefix("native_keyboard_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(), 15);
        for name in names { assert!(source.contains(&format!("fn {name}()")), "{name}"); }
        assert!(!gate.native_cases.iter().any(|name| name.starts_with("native_keyboard_process.")));
        for id in ["--check-actions-menu", "--check-settings"] {
            let entries = super::settings_sources(id);
            for path in ["rust-native/tests/native_keyboard_process.rs", "rust-native/tests/support/native_keyboard.rs", "rust-native/src/bin/private_results_fixture.rs"] {
                let matches = entries.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "{id}: {path}");
                assert_eq!(format!("{:x}", sha2::Sha256::digest(matches[0].1)), matches[0].2);
            }
        }
        let matrix = include_str!("validation.rs");
        assert!(matrix.contains("native-keyboard-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example keyboard-host --locked"));
        assert!(matrix.contains("native-keyboard-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_keyboard_process --locked"));
    }
    #[test]
    fn settings_actions_crosswalk_uses_current_rust_suite_and_helpers() {
        use sha2::Digest;
        let source = include_str!("../../../tests/actions_process.rs");
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|name| name.strip_prefix("actions_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(), 16);
        for name in names { assert!(source.contains(&format!("fn {name}()")), "{name}"); }
        assert!(!gate.native_cases.iter().any(|name| name.starts_with("actions_process.")));
        for id in ["--check-feedback", "--check-actions-menu", "--check-settings"] {
            let entries = super::settings_sources(id);
            for path in ["rust-native/tests/actions_process.rs", "rust-native/tests/support/native_actions.rs", "rust-native/tests/support/native_bridge.rs", "rust-native/tests/support/runtime_host.rs", "rust-native/tests/support/status_rpc.rs", "rust-native/tests/support/child_output.rs"] {
                let matches = entries.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "{id}: {path}");
                assert_eq!(format!("{:x}", sha2::Sha256::digest(matches[0].1)), matches[0].2);
            }
        }
        assert!(include_str!("validation.rs").contains("actions-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test actions_process --locked"));
    }
    #[test]
    fn settings_snippet_crosswalk_names_the_eleven_compiled_rust_tests() {
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        assert!(gate.required_native_steps.contains(&"snippet-process"));
        assert!(gate.required_native_steps.contains(&"snippet-links-process"));
        assert!(gate.native_cases.contains(&"ui_completion::snippet_links"));
        assert!(!gate.native_cases.iter().any(|case| case.starts_with("snippet_process.")));
        let source = include_str!("../../../tests/snippet_process.rs");
        let names = gate.native_cases.iter().filter_map(|case| case.strip_prefix("snippet_process::"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), 11);
        let generated = source.lines().filter_map(|line| line.trim().strip_prefix("method!(")
            .and_then(|line| line.strip_suffix(");"))).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(generated.len(), 10);
        assert!(generated.is_subset(&names));
        // These ten names expand to actual #[test] functions, not just labels.
        let compact = source.split_whitespace().collect::<String>();
        assert!(compact.contains("macro_rules!method{($name:ident)=>{#[test]fn$name(){compare(stringify!($name));}};"));
        let direct = names.difference(&generated).copied().collect::<Vec<_>>();
        assert_eq!(direct, ["invalid_long_url_does_not_repeat_legacy_exponential_match"]);
        assert!(compact.contains(&format!("#[test]fn{}()", direct[0])));
    }
    #[test]
    fn product_version_root_crosswalk_preserves_five_intents_and_real_rust_cases() {
        let value=inventory("check").unwrap();assert!(!value["pythonModuleInventory"].as_array().unwrap().iter().any(|path|path=="Tests/test_product_version.py"));
        let mapping=&value["rustRootTestCrosswalk"]["Tests/test_product_version.py"];assert_eq!(mapping["authority"],"rust/src/product_version.rs");assert_eq!(mapping["originalMethods"].as_array().unwrap().len(),5);
        let source=include_str!("version.rs");let cases=mapping["nativeCases"].as_array().unwrap();assert_eq!(cases.len(),3);for case in cases{let name=case.as_str().unwrap().rsplit("::").next().unwrap();assert!(source.contains(&format!("fn {name}(")),"{name}");}
        let installer = &value["rustRootTestCrosswalk"]["Tests/test_install_migration.py"];
        assert_eq!(installer["remainingPythonFlowConsumers"], 0);
        assert_eq!(installer["originalMethods"].as_array().unwrap().len(), 16);
        assert_eq!(installer["remainingPythonFilesystemCases"], 0);
        assert!(!value["pythonModuleInventory"].as_array().unwrap().iter().any(|path| path == "Tests/test_install_migration.py"));
        let native = include_str!("../ruycast-install/tests.rs");
        for case in installer["nativeCases"].as_array().unwrap() {
            let name = case.as_str().unwrap().rsplit("::").next().unwrap();
            assert!(native.contains(&format!("fn {name}(")), "{name}");
        }
    }
    #[test]
    fn retired_benchmark_python_cases_map_to_the_rust_driver_contracts() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_benchmark_host.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_benchmark_host.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 7);
        assert_eq!(mapping["nativeCases"].as_array().unwrap().len(), 4);
        assert_eq!(mapping["remainingPythonCases"], 0);
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_cli.py"));
        let tests = include_str!("../../../tests/benchmark_host_process.rs");
        let startup = include_str!("../../../tests/startup_process.rs");
        for case in mapping["nativeCases"].as_array().unwrap() {
            let name = case.as_str().unwrap();
            assert!(tests.contains(&format!("fn {name}(")), "{name}");
        }
        assert!(startup
            .contains("fn real_rust_benchmark_ignores_external_configuration_with_500_commands("));
        assert!(
            startup.contains("fn real_rust_benchmark_rejects_changed_cli_and_quits_gracefully(")
        );
    }
    #[test]
    fn search_ranking_cli_case_maps_to_owned_rust_process_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "test_search_ranking_summary_and_confirmed_reset"));
        let ranking_case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], ranking_case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains(&format!("fn {}(", ranking_case.rsplit("::").next().unwrap())));
        let validation = include_str!("validation.rs");
        assert!(validation.contains(ranking_case));
        assert!(validation.contains(
            "cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast startup_discovery_tests::current_cli_"
        ));
    }
    #[test]
    fn unknown_operation_cli_case_maps_to_bounded_rust_socket_assertion() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert!(cli_mapping["migratedMethods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "test_unknown_operation_returns_json_error"));
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("b\"{\\\"op\\\":\\\"missing\\\"}\\n\""));
        assert!(rust_cli.contains("response.len() > 8192"));
        assert!(rust_cli.contains(
            "while written < request.len() {\n            if std::time::Instant::now() >= deadline"
        ));
        assert!(rust_cli.contains("line.len()<=8192&&line.ends_with(b\"\\n\")"));
        assert!(rust_cli.contains("response[\"ok\"],false"));
        assert!(rust_cli.contains("response[\"error\"].is_string()"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains(
            "cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast startup_discovery_tests::current_cli_"
        ));
    }
    #[test]
    fn provider_failure_cli_case_maps_to_rust_private_results_process_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "test_failed_bridge_retains_provider_error_exit_status_and_queryable_log"));
        let case = "provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace";
        let private_results = include_str!("../../../tests/private_results.rs");
        assert!(private_results.contains("fn provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace()"));
        assert!(private_results.contains("assert_eq!(status.code(), Some(1))"));
        assert!(private_results.contains("assert_eq!(response[\"exitCode\"], 23)"));
        assert!(private_results.contains("assert!(!logs.to_string().contains(\"sample-secret-value\"))"));
        assert_eq!(cli_mapping["nativeTargets"]["private_results"][0], case);
        let validation = include_str!("validation.rs");
        assert!(validation.contains("private-results-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test private_results --locked"));
    }
    #[test]
    fn quick_note_provider_cli_case_maps_to_the_optional_owned_vault_test() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"]
            ["supplementalCurrentMethodByMethod"]["test_existing_quick_note_provider_in_fixture_vault"];
        assert_eq!(
            mapping,
            "runtime_process::existing_quick_note_provider_writes_only_into_its_owned_fixture_vault"
        );
        let runtime = include_str!("../../../tests/runtime_process.rs");
        assert!(runtime.contains(
            "fn existing_quick_note_provider_writes_only_into_its_owned_fixture_vault("
        ));
        assert!(runtime.contains("#[ignore = \"Requires configured Quick Note provider fixture\"]"));
        assert!(runtime.contains("RUYCAST_CAPTURE_PROVIDER"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains(
            "runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"
        ));
    }
    #[test]
    fn graceful_quit_cli_case_maps_to_owned_runtime_process_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_graceful_quit_removes_control_socket"));
        let case = "quit_acknowledges_then_exits_normally_without_kill_fallback";
        let runtime_process = include_str!("../../../tests/runtime_process.rs");
        assert!(runtime_process.contains(&format!("fn {case}()")));
        assert!(runtime_process.contains("host.command(&host.settings.cli).arg(\"quit\")"));
        assert!(runtime_process.contains("Duration::from_secs(5)"));
        assert!(runtime_process.contains("assert!(!host.socket().exists())"));
        assert_eq!(cli_mapping["nativeTargets"]["runtime_process"][0], case);
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn performance_cli_case_maps_to_bounded_rust_private_results_process_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_bounded_privacy_safe_performance_summary_and_confirmed_reset"));
        let case = "public_performance_cli_and_rpc_preserve_real_search_reset_schema_and_privacy";
        assert!(cli_mapping["nativeTargets"]["private_results"].as_array().unwrap()
            .iter().any(|name| name == case));
        let private_results = include_str!("../../../tests/private_results.rs");
        assert!(private_results.contains(&format!("fn {case}()")));
        assert!(private_results.contains("child_output::wait("));
        assert!(private_results.contains("Duration::from_secs(10)"));
        assert!(private_results.contains("result.status.code(),"));
        assert!(private_results.contains("assert!(cleared[\"result\"].get(\"query\").is_none())"));
        assert!(private_results.contains("let (post_reset_success, post_reset) = run_cli(&[\"performance\"]);"));
        assert!(private_results.contains("assert_eq!(metric[\"count\"], 0)"));
        assert!(private_results.contains("assert!(post_reset[\"result\"].get(\"query\").is_none())"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("private-results-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test private_results --locked"));
    }
    #[test]
    fn invalid_external_config_cli_case_maps_to_last_good_runtime_process_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_invalid_external_config_retains_working_runtime"));
        let case = "atomic_replace_invalid_last_good_and_recovery";
        let runtime_process = include_str!("../../../tests/runtime_process.rs");
        assert!(runtime_process.contains(&format!("fn {case}()")));
        assert!(runtime_process.contains("host.cli_output(&[\"reload\"])") );
        assert!(runtime_process.contains("Some(1)"));
        assert!(runtime_process.contains("host.cli(&[\"search\", \"echo-test\"])") );
        assert!(runtime_process.contains("host.cli(&[\"status\"])") );
        assert!(cli_mapping["nativeTargets"]["runtime_process"].as_array().unwrap()
            .iter().any(|name| name == case));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn alias_dry_run_cli_case_maps_to_existing_rust_execution_case() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_alias_search_and_dry_run"));
        let case = "cli_literal_input_uuid_usage_and_private_logs";
        let execution = include_str!("../../../tests/execution_basic_process.rs");
        assert!(execution.contains(&format!("fn {case}()")));
        assert!(execution.contains("cmd[\"aliases\"] = json!([\"runfixture\", \"echo-test\"]);"));
        assert!(execution.contains("h.cli(&[\"search\", \"echo-test\"])"));
        assert!(execution.contains("h.cli(&[\"run\", \"echo-test\", \"--dry-run\"])"));
        assert!(execution.contains("assert_eq!(h.cli(&[\"usage\"]), before_usage)"));
        assert_eq!(cli_mapping["nativeTargets"]["execution_basic_process"][0], case);
        let validation = include_str!("validation.rs");
        assert!(validation.contains("execution-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test execution_basic_process --locked"));
    }
    #[test]
    fn concurrent_config_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_concurrent_config_edits_preserve_both_settings"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("let first=scope.spawn(||run(&[\"config\",\"set\",\"clipboard.limit\",\"77\"]));"));
        assert!(rust_cli.contains("let second=scope.spawn(||run(&[\"config\",\"set\",\"clipboard.retentionDays\",\"12\"]));"));
        assert!(rust_cli.contains("current[\"result\"][\"clipboard\"][\"limit\"],77"));
        assert!(rust_cli.contains("current[\"result\"][\"clipboard\"][\"retentionDays\"],12"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn add_remove_alias_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_add_remove_and_duplicate_alias"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("let command=json!({\"id\":\"test.added\",\"title\":\"Added\",\"kind\":\"path\",\"target\":\"/\",\"aliases\":[\"added\"]});"));
        assert!(rust_cli.contains("run(&[\"run\",\"added\",\"--dry-run\"])"));
        assert!(rust_cli.contains("run(&[\"config\",\"remove\",\"test.added\"])"));
        assert!(rust_cli.contains("run(&[\"config\",\"get\"])[\"sha256\"],before"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn unknown_config_field_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_unknown_config_field_is_rejected_by_rust_and_native_bridge"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("invalid[\"misspelledSetting\"]=json!(true)"));
        assert!(rust_cli.contains("run(&[\"config\",\"validate\"])[\"ok\"],false"));
        assert!(rust_cli.contains("run(&[\"reload\"])[\"ok\"],false"));
        assert!(rust_cli.contains("An invalid external field must preserve the last-good runtime catalog"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn command_separation_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_command_separation_legacy_config_and_cli_toggle"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("Absent legacy setting must retain its original default"));
        assert!(rust_cli.contains("current=run(&[\"config\",\"get\"])") && rust_cli.contains("current[\"sha256\"]"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn atomic_replacement_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_atomic_replacement_preserves_new_fields_and_rejects_drift"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("run(&[\"config\",\"replace\",&input])[\"ok\"],false"));
        assert!(rust_cli.contains("A stale CAS must preserve the newer complete configuration"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn snippet_output_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_snippet_output_roundtrip_and_rejection"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("\\\"dark\\\""));
        assert!(rust_cli.contains("wrong_kind_null"));
        assert!(rust_cli.contains("Rejected snippet output/kind must not publish configuration"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn snippets_quicklinks_input_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let cli_mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(cli_mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(cli_mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_snippets_and_quicklinks_and_input_are_literal"));
        let case = "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects";
        assert_eq!(cli_mapping["nativeTargets"]["rust_cli"][0], case);
        let rust_cli = include_str!("../../../../rust/src/bin/ruycast.rs");
        assert!(rust_cli.contains("snippet.test"));
        assert!(rust_cli.contains("quicklinks\",\"list\",\"find-test"));
        assert!(rust_cli.contains("literal $(do not run)\\nΩ"));
        assert!(rust_cli.contains("snippetExpansionEnabled"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast"));
        assert!(validation.contains(case));
    }
    #[test]
    fn settings_headless_cli_case_maps_to_existing_rust_process_contract() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_settings_command_is_searchable_and_headless_guarded"));
        let case = "builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly";
        assert_eq!(mapping["nativeTargets"]["remaining_macos_process"][0], case);
        let source = include_str!("../../../tests/remaining_macos_process.rs");
        assert!(source.contains(r#".args(["search", "settings"])"#));
        assert!(source.contains("Some(1)"));
        assert!(source.contains(r#"contains("headless")"#));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("remaining-macos-process|c|test --manifest-path rust-native/Cargo.toml"));
        assert!(source.contains(&format!("fn {case}(")));
    }
    #[test]
    fn script_literal_failure_cli_case_maps_to_existing_rust_execution_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_script_literal_arguments_and_failure"));
        let case = "cli_literal_input_uuid_usage_and_private_logs";
        assert_eq!(mapping["nativeTargets"]["execution_basic_process"][0], case);
        let source = include_str!("../../../tests/execution_basic_process.rs");
        assert!(source.contains("literal $(not a command)\\nΩ"));
        assert!(source.contains("Some(1)"));
        assert!(source.contains(r#"["run", "test.echo"]"#));
        assert!(source.contains(r#"failure["exitCode"], 1"#));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("execution-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test execution_basic_process --locked"));
    }
    #[test]
    fn identity_legacy_environment_cli_case_maps_to_existing_runtime_process_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_new_identity_and_legacy_environment"));
        let case = "legacy_cli_status_search_reload_and_config_edit";
        assert!(mapping["nativeTargets"]["runtime_process"].as_array().unwrap().iter().any(|name| name == case));
        let source = include_str!("../../../tests/runtime_process.rs");
        assert!(source.contains("status[\"name\"], \"Ruycast\""));
        assert!(source.contains("RUSTCAST_HOME"));
        assert!(source.contains("unused-legacy-profile"));
        assert!(source.contains("Some(0)"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn config_validation_conflict_cli_case_maps_to_existing_runtime_process_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(mapping["migratedMethods"].as_array().unwrap().len(), 20);
        assert!(mapping["migratedMethods"].as_array().unwrap().iter()
            .any(|method| method == "test_config_set_validation_and_conflict"));
        let case = "legacy_cli_status_search_reload_and_config_edit";
        assert!(mapping["nativeTargets"]["runtime_process"].as_array().unwrap().iter().any(|name| name == case));
        let source = include_str!("../../../tests/runtime_process.rs");
        assert!(source.contains("let original = fs::read(&host.config).unwrap();"));
        assert!(source.contains("clipboard.limit"));
        assert!(source.contains("Some(1)"));
        assert!(source.contains(r#"response["ok"], false"#));
        assert!(source.contains("fs::read(&host.config).unwrap(), original"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn appearance_preferences_cli_case_maps_to_existing_runtime_process_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(
            mapping["supplementalCurrentMethodByMethod"]["test_appearance_preferences_validate_and_persist"],
            "runtime_process::legacy_cli_status_search_reload_and_config_edit"
        );
        assert!(mapping["nativeTargets"]["runtime_process"]
            .as_array().unwrap().iter()
            .any(|name| name == "legacy_cli_status_search_reload_and_config_edit"));
        let source = include_str!("../../../tests/runtime_process.rs");
        assert!(source.contains("for theme in [\"system\", \"light\", \"dark\", \"graphite\"]"));
        assert!(source.contains("\"menuBarVisible\", \"\\\"yes\\\"\""));
        assert!(source.contains("assert_eq!(fs::read(&host.config).unwrap(), appearance_config)"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn clipboard_import_cli_case_maps_to_live_history_host_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        let case = "clipboard_history_host_process::test_live_import_retains_file_metadata_deduplicates_and_rejects_unsafe_data_file";
        assert_eq!(mapping["supplementalCurrentMethodByMethod"]["test_clipboard_import_keeps_file_metadata_and_replays_without_duplicates"], case);
        assert!(mapping["nativeTargets"]["clipboard_history_host_process"].as_array().unwrap()
            .iter().any(|name| name == "test_live_import_retains_file_metadata_deduplicates_and_rejects_unsafe_data_file"));
        let history = include_str!("../../../tests/clipboard_history_host_process.rs");
        assert!(history.contains("fn test_live_import_retains_file_metadata_deduplicates_and_rejects_unsafe_data_file()"));
        assert!(history.contains("assert_eq!(replay[\"result\"][\"deduplicated\"], 2)"));
        assert!(history.contains("assert_eq!(file[\"fileURLs\"], values[1][\"fileURLs\"]);"));
        assert!(history.contains("dataFile\":\"../private\""));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("clipboard-history-host-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test clipboard_history_host_process --locked"));
    }
    #[test]
    fn launcher_management_cli_case_maps_to_existing_headless_builtin_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        let case = "remaining_macos_process::builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly";
        assert_eq!(mapping["supplementalCurrentMethodByMethod"]["test_launcher_management_commands_and_settings_hotkey"], case);
        assert!(mapping["nativeTargets"]["remaining_macos_process"].as_array().unwrap()
            .iter().any(|name| name == "builtin_reload_uses_existing_writer_and_headless_ui_builtins_fail_explicitly"));
        let source = include_str!("../../../tests/remaining_macos_process.rs");
        assert!(source.contains("(\"open-config\", \"builtin.open-config\")"));
        assert!(source.contains("(\"reload-config\", \"builtin.reload-config\")"));
        assert!(source.contains("\"hotkey\":\"ctrl+alt+cmd+s\""));
        assert!(source.contains("settings_search[\"result\"]"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("remaining-macos-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test remaining_macos_process --locked"));
    }
    #[test]
    fn clipboard_control_cli_case_maps_to_private_headless_process_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        let case = "remaining_macos_process::cli_clipboard_controls_keep_private_files_and_reject_headless_ui";
        assert_eq!(mapping["supplementalCurrentMethodByMethod"]["test_clipboard_control_private_permissions_and_no_ui"], case);
        assert!(mapping["nativeTargets"]["remaining_macos_process"].as_array().unwrap()
            .iter().any(|name| name == "cli_clipboard_controls_keep_private_files_and_reject_headless_ui"));
        let source = include_str!("../../../tests/remaining_macos_process.rs");
        assert!(source.contains("fn cli_clipboard_controls_keep_private_files_and_reject_headless_ui()"));
        assert!(source.contains("run(&[\"clipboard\", \"copy\", \"missing\"])[\"ok\"], false"));
        assert!(source.contains("fs::metadata(host.profile.join(\"clipboard.json\"))"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("remaining-macos-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test remaining_macos_process --locked"));
    }
    #[test]
    fn script_timeout_output_cli_case_maps_to_owned_rust_cli_process_case() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(
            mapping["supplementalCurrentMethodByMethod"]["test_script_timeout_and_bounded_output"],
            "runtime_process::legacy_cli_script_timeout_and_bounded_output"
        );
        assert!(mapping["nativeTargets"]["runtime_process"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "legacy_cli_script_timeout_and_bounded_output"));
        let runtime = include_str!("../../../tests/runtime_process.rs");
        assert!(runtime.contains("fn legacy_cli_script_timeout_and_bounded_output()"));
        assert!(runtime.contains("/usr/bin/head"));
        assert!(runtime.contains("\"200000\", \"/dev/zero\""));
        assert!(runtime.contains("Duration::from_secs(2)"));
        assert!(runtime.contains(r#"output["result"]["stdout"].as_str().unwrap().len(), 65536"#));
        let support = include_str!("../../../tests/support/runtime_host.rs");
        assert!(support.contains("Duration::from_secs(6)"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn second_instance_cli_case_maps_to_owned_socket_recovery_test() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(
            mapping["supplementalCurrentMethodByMethod"]["test_second_instance_does_not_replace_socket"],
            "runtime_process::second_instance_refused_and_crashed_socket_recovers"
        );
        assert!(mapping["nativeTargets"]["runtime_process"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "second_instance_refused_and_crashed_socket_recovers"));
        let runtime = include_str!("../../../tests/runtime_process.rs");
        assert!(runtime.contains("fn second_instance_refused_and_crashed_socket_recovers()"));
        assert!(runtime.contains("response[\"error\"].as_str().unwrap().contains(\"already running\")"));
        assert!(runtime.contains("assert_eq!(after[\"result\"][\"pid\"], before)"));
        assert!(runtime.contains("Duration::from_secs(3)"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked"));
    }
    #[test]
    fn descendant_timeout_cli_case_maps_to_two_owned_rust_process_cases() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        let cases = mapping["supplementalCurrentMethodByMethod"]
            ["test_script_timeout_kills_descendants_and_inherited_pipes"]
            .as_array()
            .unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0], "script_basic_process::timeout_cleans_group");
        assert_eq!(cases[1], "script_basic_process::early_success_closes_inherited_pipes");
        let targets = mapping["nativeTargets"]["script_basic_process"]
            .as_array()
            .unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], "timeout_cleans_group");
        assert_eq!(targets[1], "early_success_closes_inherited_pipes");
        let runtime = include_str!("../../../tests/script_basic_process.rs");
        assert!(runtime.contains("fn timeout_cleans_group()"));
        assert!(runtime.contains("fn early_success_closes_inherited_pipes()"));
        assert!(runtime.contains("started.elapsed() < Duration::from_secs(2)"));
        assert!(runtime.contains("descendant survived"));
        let validation = include_str!("validation.rs");
        assert!(validation.contains("script-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test script_basic_process --locked"));
    }
    #[test]
    fn cli_python_module_is_retired_after_all_cases_map_to_rust() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array().unwrap().iter()
            .any(|path| path == "Tests/test_cli.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_cli.py"];
        assert_eq!(mapping["remainingPythonModule"], false);
        assert_eq!(mapping["remainingPythonCases"], 0);
        assert_eq!(mapping["migratedMethods"].as_array().unwrap().len(), 20);
        let supplemental = mapping["supplementalCurrentMethodByMethod"].as_object().unwrap();
        assert_eq!(supplemental.len(), 8);
        for method in [
            "test_clipboard_import_keeps_file_metadata_and_replays_without_duplicates",
            "test_appearance_preferences_validate_and_persist",
            "test_launcher_management_commands_and_settings_hotkey",
            "test_clipboard_control_private_permissions_and_no_ui",
            "test_existing_quick_note_provider_in_fixture_vault",
        ] {
            assert!(supplemental.contains_key(method), "missing Rust mapping for {method}");
        }
        let quick_note = include_str!("../../../tests/runtime_process.rs");
        assert!(quick_note.contains("#[ignore = \"Requires configured Quick Note provider fixture\"]"));
        assert!(quick_note.contains("fn existing_quick_note_provider_writes_only_into_its_owned_fixture_vault()"));
        let validation = include_str!("validation.rs");
        assert!(!validation.contains("\"Tests/test_cli.py\""));
        let source_policy = include_str!("../ruycast-source/policy.rs");
        assert!(!source_policy.lines().any(|line| line == "Tests/test_cli.py"));
    }
    #[test]
    fn retired_architecture_python_cases_map_to_rust_policy_tests() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_architecture.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_architecture.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 3);
        assert_eq!(mapping["nativeCases"].as_array().unwrap().len(), 2);
        assert_eq!(mapping["retiredOriginalMethods"], serde_json::json!(["test_swift_uses_imported_c_symbols"]));
        assert_eq!(mapping["remainingPythonCases"], 0);
        let tests = include_str!("../../../tests/architecture_policy.rs");
        for case in mapping["nativeCases"].as_array().unwrap() {
            assert!(tests.contains(&format!("fn {}(", case.as_str().unwrap())));
        }
    }
    #[test]
    fn package_app_root_crosswalk_preserves_seven_rust_candidate_contracts() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_package_app.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_package_app.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 7);
        assert_eq!(mapping["remainingPythonCases"], 0);
        let candidate = include_str!("candidate.rs");
        let public_source = include_str!("public_source.rs");
        let compat = include_str!("compat.rs");
        for case in mapping["nativeCases"].as_array().unwrap() {
            let name = case.as_str().unwrap().rsplit("::").next().unwrap();
            assert!(candidate.contains(&format!("fn {name}("))
                || public_source.contains(&format!("fn {name}("))
                || compat.contains(&format!("fn {name}(")),
                "{name}");
        }
    }
    #[test]
    fn private_storage_root_crosswalk_resolves_all_twelve_original_methods() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_private_storage.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_private_storage.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 12);
        assert_eq!(mapping["remainingPythonCases"], 0);
        let sources = [
            include_str!("../../../../rust/src/private_storage.rs"),
            include_str!("../../../tests/startup_config_repair.rs"),
            include_str!("../../../tests/runtime_process.rs"),
            include_str!("../../../tests/foundation_process.rs"),
        ];
        let targets = mapping["nativeTargets"].as_object().unwrap();
        let bindings = mapping["nativeCaseByMethod"].as_object().unwrap();
        assert_eq!(bindings.len(), 12);
        let methods = mapping["originalMethods"].as_array().unwrap();
        let mut unique_cases = std::collections::BTreeSet::new();
        for method in methods {
            let method = method.as_str().unwrap();
            let binding = bindings[method].as_str().unwrap();
            let (target, case) = binding.split_once(" -> ").unwrap();
            let cases = targets[target].as_array().unwrap();
            assert!(cases.iter().any(|candidate| candidate == case));
            unique_cases.insert(case);
            let name = case.rsplit("::").next().unwrap();
            assert!(
                sources
                    .iter()
                    .any(|source| source.contains(&format!("fn {name}("))),
                "{name}"
            );
        }
        assert_eq!(unique_cases.len(), 9);
    }
    #[test]
    fn retired_local_signing_crosswalk_preserves_all_eleven_scenarios() {
        let value = inventory("check").unwrap();
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_local_signing.py"];
        let expected = serde_json::json!([
            "test_repeat_prepare_reuses_without_generating_or_touching_keychain",
            "test_missing_private_state_never_rotates_identity",
            "test_unsafe_private_modes_and_symlinks_are_refused",
            "test_requirement_cannot_be_weakened",
            "test_signer_failure_never_uses_adhoc_or_modifies_trust",
            "test_success_signs_and_verifies_exact_pinned_requirement",
            "test_unexpected_installed_certificate_is_refused",
            "test_one_time_adhoc_migration_checks_bundle_identity",
            "test_signing_keychain_preserves_existing_search_entries",
            "test_adhoc_is_refused_after_first_certificate_install",
            "test_private_keychain_password_is_only_sent_via_stdin"
]);
        assert_eq!(mapping["originalMethods"], expected);
        assert_eq!(mapping["remainingPythonCases"], 0);
        assert_eq!(mapping["remainingMethods"], serde_json::json!([]));
        assert_eq!(mapping["nativeBinary"], "ruycast-install");
        let bindings = mapping["nativeCaseByMethod"].as_object().unwrap();
        assert_eq!(bindings.len(), 11);
        let mut bound = std::collections::BTreeSet::new();
        for method in expected.as_array().unwrap() {
            let cases = bindings[method.as_str().unwrap()].as_array().unwrap();
            assert!(!cases.is_empty());
            for case in cases {
                let case = case.as_str().unwrap();
                let path = mapping["nativeCaseSources"][case].as_str().unwrap();
                let source = match path {
                    "rust-native/src/bin/ruycast-install/tests.rs" => include_str!("../ruycast-install/tests.rs"),
                    "rust-native/src/bin/ruycast-install/signing.rs" => include_str!("../ruycast-install/signing.rs"),
                    _ => panic!("unbound signer source: {path}"),
                };
                assert!(source.contains(&format!("fn {}(", case.rsplit("::").next().unwrap())), "{case}");
                bound.insert(case);
            }
        }
        let declared: std::collections::BTreeSet<_> = mapping["nativeCases"].as_array().unwrap().iter()
            .map(|case| case.as_str().unwrap()).collect();
        assert_eq!(bound, declared);
        assert_eq!(bound.len(), 8);
        assert_eq!(mapping["nativeCaseSources"].as_object().unwrap().len(), 8);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for path in ["Tests/test_local_signing.py", "scripts/local_signing.py"] {
            assert!(!root.join(path).exists());
            assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == path));
        }
        let modules = value["pythonModuleInventory"].as_array().unwrap();
        assert!(!modules.iter().any(|path| path == "Tests/test_local_signing.py"));
        assert!(require_complete("check").is_ok());
    }
    #[test]
    fn retired_build_pipeline_crosswalk_preserves_all_eight_scenario_groups() {
        let value = inventory("check").unwrap();
        let retired = "Tests/test_build_pipeline.py";
        assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(retired).exists());
        let modules = value["pythonModuleInventory"].as_array().unwrap();
        assert!(!modules.iter().any(|path| path == retired));
        assert!(require_complete("check").is_ok());
        let mapping = &value["rustRootTestCrosswalk"][retired];
        let expected = serde_json::json!([
            "test_checks_keep_debug_artifacts_until_the_last_consumer",
            "test_lease_survives_controller_crash_while_consumer_is_live",
            "test_lease_survives_controller_interrupt_while_consumer_is_live",
            "test_source_edit_during_checks_cannot_receive_passed_receipt",
            "test_artifact_edit_during_checks_cannot_receive_passed_receipt",
            "test_settings_and_performance_share_the_whole_pipeline_lease",
            "test_failed_build_releases_lease_and_does_not_start_consumer",
            "test_receipt_revalidation_and_failed_attempt_remove_old_pass"
]);
        assert_eq!(mapping["originalMethods"], expected);
        assert_eq!(mapping["remainingPythonCases"], 0);
        assert_eq!(mapping["remainingMethods"], serde_json::json!([]));
        let bindings = mapping["nativeCaseByMethod"].as_object().unwrap();
        assert_eq!(bindings.len(), 8);
        let sources = [
            ("rust-native --lib", "rust-native/src/artifact_lease_tests.rs", include_str!("../../artifact_lease_tests.rs")),
            ("rust-native --bin ruycast-build", "rust-native/src/bin/ruycast-build/compat.rs", include_str!("compat.rs")),
        ];
        let mut bound = std::collections::BTreeSet::new();
        for method in expected.as_array().unwrap() {
            let cases = bindings[method.as_str().unwrap()].as_array().unwrap();
            assert!(!cases.is_empty());
            for case in cases {
                let case = case.as_str().unwrap();
                let (target, source_path, source) = if case.starts_with("artifact_lease::tests::") { sources[0] } else { sources[1] };
                assert_eq!(mapping["nativeTargets"][target]["source"], source_path);
                assert!(mapping["nativeTargets"][target]["cases"].as_array().unwrap().iter().any(|entry| entry == case));
                let name = case.rsplit("::").next().unwrap();
                assert!(source.contains(&format!("fn {name}(")), "{case}");
                bound.insert(case);
            }
        }
        let declared: std::collections::BTreeSet<_> = mapping["nativeTargets"].as_object().unwrap().values()
            .flat_map(|target| target["cases"].as_array().unwrap().iter().map(|case| case.as_str().unwrap())).collect();
        assert_eq!(bound, declared);
        assert_eq!(mapping["contractDifferences"].as_object().unwrap().len(), 4);
        let temporal = include_str!("validation.rs");
        for case in bound.iter().filter(|case| case.starts_with("compat::temporal_contract_tests::")) {
            assert!(temporal.contains(*case), "temporal case must be explicitly selected: {case}");
        }
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
    }
    #[test]
    fn socket_recovery_root_crosswalk_resolves_all_eight_original_methods() {
        let value = inventory("check").unwrap();
        assert!(!value["pythonModuleInventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "Tests/test_socket_recovery.py"));
        let mapping = &value["rustRootTestCrosswalk"]["Tests/test_socket_recovery.py"];
        assert_eq!(mapping["originalMethods"].as_array().unwrap().len(), 8);
        assert_eq!(mapping["nativeTarget"], "runtime_process");
        assert_eq!(mapping["nativeCases"].as_array().unwrap().len(), 8);
        assert_eq!(mapping["remainingPythonCases"], 0);
        let source = include_str!("../../../tests/runtime_process.rs");
        for case in mapping["nativeCases"].as_array().unwrap() {
            let name = case.as_str().unwrap();
            assert!(source.contains(&format!("fn {name}(")), "{name}");
        }
    }
    #[test]
    fn bridge_rust_crosswalk_resolves_cases_and_current_shared_inputs() {
        let gate = SETTINGS
            .iter()
            .find(|gate| gate.id == "--check-interactions")
            .unwrap();
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 44);
        assert_eq!(
            inputs
                .iter()
                .map(|entry| entry.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            44
        );
        let source = std::str::from_utf8(
            inputs
                .iter()
                .find(|entry| entry.0 == "rust-native/tests/bridge_process.rs")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(
            gate.native_cases
                .iter()
                .filter(|case| case.starts_with("bridge_process::"))
                .count(),
            2
        );
        assert!(!gate
            .native_cases
            .iter()
            .any(|case| case.starts_with("bridge_process.")));
        for name in [
            "show_search_and_local_native_query_share_the_owner",
            "rapid_arrows_and_click_on_other_row_keep_intended_identity",
        ] {
            let case = format!("bridge_process::{name}");
            assert!(gate.native_cases.contains(&case.as_str()));
            assert!(source.contains(&format!("fn {name}(")), "{case}");
        }
        for path in [
            "rust-native/tests/bridge_process.rs",
            "rust-native/tests/support/native_bridge.rs",
            "rust-native/tests/support/runtime_host.rs",
            "rust-native/tests/support/status_rpc.rs",
            "rust-native/tests/support/child_output.rs",
        ] {
            let (_, bytes, expected) = inputs.iter().find(|entry| entry.0 == path).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), *expected, "{path}");
        }
        for id in [
            "--check-feedback",
            "--check-actions-menu",
            "--check-interactions",
        ] {
            let inputs=settings_sources(id);
            assert!(!inputs.iter().any(|entry|entry.0.ends_with(".py")),"{id}: retired Python helper must not remain an input");
            for path in ["rust-native/tests/support/native_bridge.rs","rust-native/tests/support/runtime_host.rs","rust-native/tests/support/status_rpc.rs","rust-native/tests/support/child_output.rs"] {
                let (_,bytes,expected)=inputs.iter().find(|entry|entry.0==path).unwrap();
                assert_eq!(format!("{:x}",Sha256::digest(bytes)),*expected,"{id}: {path}");
            }

        }
    }
    #[test]
    fn hid_goldens_bind_frozen_corpus_current_sources_and_actual_probe_copy() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        for name in hid_state_goldens::SOURCES {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                if *name == hid_state_goldens::PATH {
                    include_bytes!("../../../tests/hid_state_reference.json").as_slice()
                } else {
                    b"owned fixture source"
                },
            )
            .unwrap();
        }
        let alias = "rust-native/target/debug/hid-state-probe";
        let probe = "rust-native/target/owned-hid-probe";
        for name in [alias, probe] {
            fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
            fs::write(root.join(name), b"owned producer and identical probe copy").unwrap();
        }
        let binaries = serde_json::json!({alias:digest(&root.join(alias)).unwrap()});
        let sources = hid_state_goldens::SOURCES
            .iter()
            .map(|name| (*name, digest(&root.join(name)).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let original = serde_json::json!({"schema":"ruycast.hid-state-golden-binding.v1","sources":sources,"artifacts":{probe:digest(&root.join(probe)).unwrap()},"probe":probe,"goldenSHA256":hid_state_goldens::SHA256,"positiveCases":4623,"observedSnapshots":10940,"firstOperationRefusals":4});
        let raw = serde_json::to_vec(&original).unwrap();
        let qualified = qualify_hid_reference(root, &binaries, &raw).unwrap();
        assert_eq!(
            qualified["hidGoldenQualification"]["rawBindingSHA256"],
            format!("{:x}", Sha256::digest(&raw))
        );
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&qualified).unwrap()).is_err()
        );
        for field in [
            "goldenSHA256",
            "positiveCases",
            "observedSnapshots",
            "firstOperationRefusals",
            "probe",
        ] {
            let mut changed = original.clone();
            changed[field] = serde_json::json!("changed");
            assert!(
                qualify_hid_reference(root, &binaries, &serde_json::to_vec(&changed).unwrap()).is_err()
            );
        }
        let mut omitted = original.clone();
        omitted["sources"]
            .as_object_mut()
            .unwrap()
            .remove("rust-native/tests/support/hid_state_goldens.rs");
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&omitted).unwrap()).is_err()
        );
        fs::write(root.join(probe), b"different probe").unwrap();
        let mut rebound = original.clone();
        rebound["artifacts"][probe] = serde_json::json!(digest(&root.join(probe)).unwrap());
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&rebound).unwrap()).is_err()
        );
        fs::write(root.join(probe), b"owned producer and identical probe copy").unwrap();
        fs::write(root.join(hid_state_goldens::PATH), b"changed golden corpus").unwrap();
        rebound = original;
        rebound["sources"][hid_state_goldens::PATH] =
            serde_json::json!(digest(&root.join(hid_state_goldens::PATH)).unwrap());
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&rebound).unwrap()).is_err()
        );
    }
    #[test]
    fn swift_compatibility_fresh_and_plain_debug_directories_support_legacy_import() {
        use std::os::unix::fs::symlink;
        for existing_debug in [false, true] {
            let fixture = super::super::tests::Fixture::new();
            let root = &fixture.0;
            let scratch = root.join(".build/oracle-swift/owned");
            let bin = scratch.join("arm64-apple-macosx/debug");
            let source = bin.join("CRuycastCore.build/module.modulemap");
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(&source, b"fresh legacy modulemap").unwrap();
            let debug = root.join(".build/swift/debug");
            if existing_debug { fs::create_dir_all(&debug).unwrap(); }
            let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
            assert!(binding.alias_target.is_none());
            assert_eq!(binding.modulemap, ".build/swift/debug/CRuycastCore.build/module.modulemap");
            data_copy(root, &source, &root.join(&binding.modulemap)).unwrap();
            // Legacy import compatibility remains for the other Swift/Core oracles.
            let legacy = debug.join("CRuycastCore.build/module.modulemap");
            assert_eq!(regular_bytes(&legacy).unwrap(), b"fresh legacy modulemap");
            let artifacts = BTreeMap::from([(binding.modulemap.clone(), digest(&legacy).unwrap())]);
            verify_swift_compatibility(root, &binding, &artifacts).unwrap();
            fs::rename(&debug, root.join(".build/swift/retained-debug")).unwrap();
            symlink("arm64-apple-macosx/debug", &debug).unwrap();
            assert!(verify_swift_compatibility(root, &binding, &artifacts).is_err());
        }
    }
    #[test]
    fn swift_compatibility_uses_physical_path_and_receipt_rejects_repointed_alias() {
        use std::os::unix::fs::symlink;
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let scratch = root.join(".build/oracle-swift/owned");
        let bin = scratch.join("arm64-apple-macosx/debug");
        let source = bin.join("CRuycastCore.build/module.modulemap");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"owned modulemap").unwrap();
        fs::create_dir_all(root.join(".build/swift")).unwrap();
        let alias = root.join(".build/swift/debug");
        symlink("arm64-apple-macosx/debug", &alias).unwrap();
        let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
        let physical = root.join(&binding.modulemap);
        data_copy(root, &source, &physical).unwrap();
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("arm64-apple-macosx/debug"));
        let artifacts = BTreeMap::from([(binding.modulemap.clone(), digest(&physical).unwrap())]);
        // Read the exact serialized field used in CoreReceipt verification.
        let captured: SwiftCompatibility = serde_json::from_slice(&serde_json::to_vec(&binding).unwrap()).unwrap();
        verify_swift_compatibility(root, &captured, &artifacts).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink("x86_64-apple-macosx/debug", &alias).unwrap();
        assert!(verify_swift_compatibility(root, &captured, &artifacts).is_err());
        assert_eq!(fs::read(&physical).unwrap(), b"owned modulemap");
    }
    #[test]
    fn swift_compatibility_refuses_hostile_aliases_and_copy_escapes_before_write() {
        use std::os::unix::fs::symlink;
        let fixture = super::super::tests::Fixture::new();
        let outside = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let scratch = root.join(".build/oracle-swift/owned");
        let bin = scratch.join("arm64-apple-macosx/debug");
        let source = bin.join("CRuycastCore.build/module.modulemap");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"must stay inside").unwrap();
        fs::create_dir_all(root.join(".build/swift")).unwrap();
        let alias = root.join(".build/swift/debug");
        let sentinel = outside.0.join("sentinel"); fs::write(&sentinel, b"unchanged").unwrap();
        for target in [outside.0.clone(), PathBuf::from("../escape"), PathBuf::from("arm64-apple-macosx/../debug"), PathBuf::from("arm64-apple-macosx/./debug"), PathBuf::from("arm64-apple-macosx//debug"), PathBuf::from("x86_64-apple-macosx/debug")] {
            symlink(target, &alias).unwrap();
            assert!(swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").is_err());
            assert!(!root.join(".build/swift/arm64-apple-macosx/debug/CRuycastCore.build/module.modulemap").exists());
            assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
            fs::remove_file(&alias).unwrap();
        }
        assert!(data_copy(root, &source, &sentinel).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        assert!(data_copy(root, &source, &root.join("../escaped-modulemap")).is_err());
        // An otherwise correct alias cannot permit a linked physical parent.
        symlink("arm64-apple-macosx/debug", &alias).unwrap();
        symlink(&outside.0, root.join(".build/swift/arm64-apple-macosx")).unwrap();
        let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
        assert!(data_copy(root, &source, &root.join(binding.modulemap)).is_err());
        assert!(!outside.0.join("debug/CRuycastCore.build/module.modulemap").exists());
    }
    #[test]
    fn settings_crosswalk_preserves_every_original_selector_in_order() {
        // Historical selector list from scripts/check-settings-locked.sh.
        // The script began at b391db1a0ce9327b8ca35b7fd25462a2bdf0a57a;
        // this 16-selector snapshot is from 50d66a4a171bddba157223d995e26d662e3a0232
        // (SHA-256 21dd51c98e8b04d82e82168a4bd804cb36d2e5eff914a183451ab08329757cd8).
        let ids = [
            "--check-edit-commands",
            "--check-settings",
            "--check-interactions",
            "--check-actions-menu",
            "--check-browser-tabs",
            "--check-apple-shortcuts",
            "--check-window-search",
            "--check-menu-actions",
            "--check-render-updates",
            "--check-feedback",
            "--check-rich-resources",
            "--check-analytics",
            "--check-performance-pipeline",
            "--check-visual-assets",
            "--check-command-log",
            "--check-polish",
        ];
        assert_eq!(ids.len(), 16);
        assert_eq!(SETTINGS.iter().map(|gate| gate.id).collect::<Vec<_>>(), ids);
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            16
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("--check-browser-tabs"));
        assert!(!main.contains("--check-performance-pipeline"));
        assert!(!main.contains("--check-window-search"));
        assert!(!root.join("Sources/RuycastApp/BrowserTabChecks.swift").exists());
        assert!(!root.join("Sources/RuycastApp/PerformancePipelineChecks.swift").exists());
        assert!(!root.join("Sources/RuycastApp/WindowSearchChecks.swift").exists());
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        assert!(!roster.contains("frozen-current/Sources/RuycastApp/BrowserTabChecks.swift"));
        assert!(roster.contains("reference-0.8.26/Sources/RuycastApp/BrowserTabChecks.swift"));
        assert!(!roster.contains("frozen-current/Sources/RuycastApp/PerformancePipelineChecks.swift"));
        assert!(roster.contains("reference-0.8.26/Sources/RuycastApp/PerformancePipelineChecks.swift"));
        for gate in SETTINGS {
            if matches!(gate.id, "--check-command-log" | "--check-window-search"
                | "--check-analytics" | "--check-visual-assets" | "--check-edit-commands" | "--check-feedback"
                | "--check-actions-menu" | "--check-menu-actions" | "--check-browser-tabs" | "--check-performance-pipeline") {
                // Retired helpers retain their original path/hash as historical provenance.
                assert!(!root.join(gate.original_source).exists());
            } else {
                regular_file(&root.join(gate.original_source)).unwrap();
            }
        }
    }
    #[test]
    fn settings_hid_gate_binds_existing_rust_sdk_step() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        assert!(gate.required_native_steps.contains(&"hid-sdk-process"));
        assert!(!gate.required_native_steps.contains(&"hid-process"));
        let plan = validation::plan("aarch64-apple-darwin").unwrap();
        let step = plan["steps"].as_array().unwrap().iter()
            .find(|step| step["name"] == "hid-sdk-process").expect("Required settings HID step must exist");
        let arguments = step["arguments"].as_array().unwrap();
        assert!(arguments.windows(2).any(|pair| pair[0] == "--test" && pair[1] == "hid_sdk_process"));
    }
    #[test]
    fn settings_mapping_only_names_existing_matrix_steps_and_real_log_cases() {
        let plan = validation::plan("aarch64-apple-darwin").unwrap();
        let mut names = plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["name"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        for render in plan["renders"].as_array().unwrap() {
            names.insert(format!("native-{}-{}", render[0].as_str().unwrap(), render[1].as_str().unwrap()));
        }
        for gate in SETTINGS {
            assert!(!gate.required_native_steps.is_empty(), "{}", gate.id);
            for step in gate.required_native_steps {
                assert!(names.contains(*step), "{}: {step}", gate.id);
            }
        }
        let log = SETTINGS
            .iter()
            .find(|gate| gate.id == "--check-command-log")
            .unwrap();
        assert_eq!(log.native_cases.len(), 13);
        assert_eq!(log.required_native_steps, &["unit", "private-results-process"]);
        let rust_sources = settings_sources(log.id);
        assert_eq!(rust_sources.len(), 7);
        assert!(rust_sources.iter().all(|(path, _, _)| path.starts_with("rust-native/")));
        let bindings = settings_source_bindings(log.id);
        assert_eq!(bindings.len(), 8);
        assert_eq!(
            bindings.iter().find(|(path, _)| *path == log.original_source),
            Some(&(log.original_source, "9dd2ff68552ebcae0979530623309cafeb7a145a193444dc55a717e94a953455")),
        );
        assert_eq!(settings_mapping_state(log), "source-qualified-rust-case-crosswalk");
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
        assert!(!inventory("settings").unwrap()["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(log.id)));
        let sources = [
            ("activity::tests::", include_str!("../../activity.rs")),
            ("private_results::", include_str!("../../../tests/private_results.rs")),
            ("runtime::system::shortcut_budget_tests::", include_str!("../../runtime_system.rs")),
        ];
        for case in log.native_cases {
            let (_,source)=sources.iter().find(|(prefix,_)|case.starts_with(*prefix)).expect("mapped command log source");
            assert!(
                source.contains(&format!("fn {}(", case.rsplit("::").next().unwrap())),
                "{case}"
            );
        }
    }
    #[test]
    fn browser_and_performance_maps_name_existing_assertions_and_complete_routes() {
        let sources = [
            (
                "runtime::system::shortcut_budget_tests::",
                include_str!("../../runtime_system.rs"),
            ),
            (
                "system_search::providers::gate_tests::",
                include_str!("../../system_search/providers.rs"),
            ),
            (
                "system_search::tests::",
                include_str!("../../system_search/tests.rs"),
            ),
            (
                "system_search_sdk::",
                include_str!("../../../tests/system_search_sdk.rs"),
            ),
            ("system_search_cli_process::", include_str!("../../../tests/system_search_cli_process.rs")),
            (
                "runtime::inline_tests::",
                include_str!("../../runtime_private_tests.rs"),
            ),
            (
                "configuration_jobs::tests::",
                include_str!("../../configuration_jobs.rs"),
            ),
            (
                "clipboard_history::tests::",
                include_str!("../../clipboard_history.rs"),
            ),
            (
                "native_services::",
                include_str!("../../native_services.rs"),
            ),
            (
                "clipboard_rich_paste::",
                include_str!("../../../tests/clipboard_rich_paste.rs"),
            ),
            (
                "clipboard_history_host_process::",
                include_str!("../../../tests/clipboard_history_host_process.rs"),
            ),
        ];
        for (id, expected_steps, expected_sources) in [
            (
                "--check-browser-tabs",
                &["unit",  "system-search-cli-process", "system-search-sdk"][..],
                32,
            ),
            (
                "--check-performance-pipeline",
                &["unit", "clipboard-history-host-process", "clipboard-rich-paste"][..],
                28,
            ),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.required_native_steps, expected_steps);
            let entries = settings_sources(id);
            assert_eq!(entries.len(), expected_sources);
            if id == "--check-browser-tabs" {
                assert_eq!(gate.native_cases.len(), 28);
                assert!(!entries.iter().any(|(path, _, _)| *path == gate.original_source));
                let bindings = settings_source_bindings(id);
                assert_eq!(bindings.len(), 33);
                for (path, _, hash) in entries {
                    assert!(bindings.contains(&(*path, *hash)), "missing current browser binding {path}");
                }
                let status_rpc = entries.iter().find(|entry| entry.0 == "rust-native/tests/support/status_rpc.rs").unwrap();
                assert!(std::str::from_utf8(status_rpc.1).unwrap().contains("benchmark_host::rpc::{connect, request}"));
                let rpc = entries.iter().find(|entry| entry.0 == "rust-native/src/benchmark_host/rpc.rs").unwrap();
                assert_eq!(rpc.2, "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9");
                assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn connect("));
                assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn request("));
                assert!(bindings.contains(&(gate.original_source, BROWSER_TABS_ORACLE_SHA256)));
                assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
                let inventory = inventory("settings").unwrap();
                assert_eq!(inventory["productionAccepted"], false);
                assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
            }
            if id == "--check-performance-pipeline" {
                assert_eq!(gate.native_cases.len(), 14);
                assert!(!entries.iter().any(|(path, _, _)| *path == gate.original_source));
                let bindings = settings_source_bindings(id);
                assert_eq!(bindings.len(), 29);
                for (path, _, hash) in entries {
                    assert!(bindings.contains(&(*path, *hash)), "missing current performance binding {path}");
                }
                assert!(bindings.contains(&(
                    gate.original_source,
                    PERFORMANCE_PIPELINE_ORACLE_SHA256,
                )));
                assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
                let inventory = inventory("settings").unwrap();
                assert_eq!(inventory["productionAccepted"], false);
                assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
            }
            assert!(gate.unproven_invariants.is_empty());
            assert!(!gate.native_cases.is_empty());
            for case in gate.native_cases {
                let (prefix, source) = sources
                    .iter()
                    .find(|(prefix, _)| case.starts_with(*prefix))
                    .unwrap();
                let name = case
                    .strip_prefix(*prefix)
                    .unwrap()
                    .rsplit('.')
                    .next()
                    .unwrap();
                assert!(
                    source.contains(&format!("fn {name}("))
                        || source.contains(&format!("def {name}(")),
                    "{id}: {case}"
                );
            }
        }
    }
    #[test]
    fn window_shortcuts_and_render_maps_preserve_named_cases_routes_and_source_inputs() {
        let case_sources = [
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("system_search::apple_shortcuts::tests::", "rust-native/src/system_search/apple_shortcuts.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_search_sdk::", "rust-native/tests/system_search_sdk.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
            ("script_result_basic_process::", "rust-native/tests/script_result_basic_process.rs"),
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
        ];
        for (id, steps, case_count, source_count) in [
            ("--check-window-search", &["unit",  "system-search-cli-process", "system-search-sdk"][..], 18, 34),
            ("--check-apple-shortcuts", &["unit",  "system-search-cli-process", "system-search-sdk", "private-results-process", "script-process", "script-result-process", "script-result-native-process"][..], 10, 46),
            ("--check-render-updates", &["render-updates-process"][..], 1, 13),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.required_native_steps, steps);
            assert!(gate.unproven_invariants.is_empty());
            assert_eq!(gate.native_cases.len(), case_count);
            let inputs = settings_sources(id);
            assert_eq!(inputs.len(), source_count);
            if id == "--check-window-search" {
                assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
                let bindings = settings_source_bindings(id);
                assert_eq!(bindings.len(), 35);
                assert!(bindings.contains(&(gate.original_source, WINDOW_SEARCH_ORACLE_SHA256)));
                assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
            } else if id == "--check-render-updates" {
                assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
                let bindings = settings_source_bindings(id);
                assert_eq!(bindings.len(), 14);
                assert!(bindings.contains(&(gate.original_source, RENDER_UPDATES_ORACLE_SHA256)));
                assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
            } else if id == "--check-apple-shortcuts" {
                assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
                let bindings = settings_source_bindings(id);
                assert_eq!(bindings.len(), 47);
                assert!(bindings.contains(&(gate.original_source, APPLE_SHORTCUTS_ORACLE_SHA256)));
                assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
            } else {
                assert!(inputs.iter().any(|(path, _, _)| *path == gate.original_source));
            }
            assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), source_count);
            for case in gate.native_cases {
                let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
                let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
                let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
                assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{id}: {case}");
                if prefix.ends_with('.') {
                    let class = case.strip_prefix(*prefix).unwrap().split('.').next().unwrap();
                    assert!(source.contains(&format!("class {class}(")), "{id}: {case}");
                }
            }
        }
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn menu_actions_provenance_keeps_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-menu-actions").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "system-search-cli-process", "system-search-sdk"]);
        assert_eq!(gate.native_cases.len(), 22);
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 38);
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("rust/" ) || *path == "rust-toolchain.toml" || *path == "config/default.json"));
        let bindings = settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 39);
        for (path, _, hash) in inputs {
            assert!(bindings.contains(&(*path, *hash)), "missing current menu-actions binding {path}");
        }
        assert!(bindings.contains(&(gate.original_source, MENU_ACTIONS_ORACLE_SHA256)));
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let value = inventory("settings").unwrap();
        assert_eq!(value["productionAccepted"], false);
        assert!(!value["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn apple_shortcuts_provenance_keeps_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-apple-shortcuts").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, APPLE_SHORTCUTS_ORACLE_SHA256);
        let bindings = settings_source_bindings(gate.id);

        assert_eq!(gate.required_native_steps, &["unit", "system-search-cli-process", "system-search-sdk", "private-results-process", "script-process", "script-result-process", "script-result-native-process"]);
        assert_eq!(gate.native_cases.len(), 10);
        assert_eq!(inputs.len(), 46);
        assert_eq!(bindings.len(), 47);
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert!(bindings.contains(&provenance));
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("rust/") || *path == "rust-toolchain.toml" || *path == "config/default.json"));
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current Apple Shortcuts binding: {path}");
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn retired_actions_menu_check_has_no_caller_and_preserves_crosswalk() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/ActionMenuChecks.swift";
        assert!(!root.join(retired).exists());
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("--check-actions-menu"));
        assert!(!main.contains("checkActionsMenu"));
        assert!(!main.contains("checkItemBindingEditing"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"], format!("{:x}", Sha256::digest(main.as_bytes())));
        assert_eq!(ACTIONS_MENU_ORACLE_SHA256, "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f");
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        assert_eq!(public["schema"], "ruycast.public-source.v2");
        assert!(public["files"].get(retired).is_none());
        for path in ["Sources/RuycastApp/main.swift", "rust-native/src/bin/ruycast-build/oracles.rs", "rust-native/src/bin/ruycast-source/policy.rs", "rust-native/swift-reference-lock.json"] {
            assert_eq!(public["files"][path], format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())), "stale public source hash: {path}");
        }
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-actions-menu").unwrap();
        assert_eq!(gate.native_cases.len(), 11);
        assert_eq!(gate.required_native_steps.len(), 8);
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }

    #[test]
    fn actions_menu_provenance_preserves_cases_shared_sources_and_routes() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-actions-menu").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, ACTIONS_MENU_ORACLE_SHA256);
        let bindings = settings_source_bindings(gate.id);
        let case_sources = [
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("actions_process::", "rust-native/tests/actions_process.rs"),
            ("input_process::", "rust-native/tests/input_process.rs"),
            ("native_keyboard_process::", "rust-native/tests/native_keyboard_process.rs"),
        ];

        assert_eq!(gate.required_native_steps, &["actions-process", "actions-menu-process", "input-process-rust", "feedback-actions-process", "snippet-hotkey-process", "native-keyboard-process", "polish-original-process", "item-binding-original-process"]);
        assert_eq!(gate.native_cases.len(), 11);
        assert_eq!(inputs.len(), 25);
        assert_eq!(bindings.len(), 28);
        assert_eq!(bindings.iter().collect::<std::collections::BTreeSet<_>>().len(), 28);
        assert_eq!(provenance.1, "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f");
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/FeedbackChecks.swift"));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/InteractionChecks.swift"));
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert!(bindings.contains(&provenance));
        let additional = settings_additional_provenance_sources(gate.id);
        assert_eq!(additional, &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/InteractionChecks.swift", "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858")]);
        for provenance in additional { assert!(bindings.contains(provenance)); }
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current actions-menu binding: {path}");
        }
        let status_rpc = inputs.iter().find(|entry| entry.0 == "rust-native/tests/support/status_rpc.rs").unwrap();
        assert!(std::str::from_utf8(status_rpc.1).unwrap().contains("benchmark_host::rpc::{connect, request}"));
        let rpc = inputs.iter().find(|entry| entry.0 == "rust-native/src/benchmark_host/rpc.rs").unwrap();
        assert_eq!(rpc.2, "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9");
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn connect("));
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn request("));
        for case in gate.native_cases {
            let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        let planned_steps = plan["steps"].as_array().unwrap();
        for name in gate.required_native_steps {
            assert!(planned_steps.iter().any(|step| step["name"] == *name), "missing actions-menu route: {name}");
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn menu_map_preserves_original_cases_required_routes_and_source_inputs() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-menu-actions").unwrap();
        assert_eq!(gate.required_native_steps, &["unit",  "system-search-cli-process", "system-search-sdk"]);
        assert_eq!(gate.native_cases.len(), 22);
        assert!(gate.unproven_invariants.is_empty());
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 38);
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        let bindings = settings_source_bindings(gate.id);
        assert_eq!(bindings.len(), 39);
        assert!(bindings.contains(&(gate.original_source, MENU_ACTIONS_ORACLE_SHA256)));
        assert!(inputs.iter().any(|(path, _, _)| *path == "rust-native/tests/fixtures/shortcut_child.rs"));
        assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), 38);
        let sources = [
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("system_search::native::tests::", "rust-native/src/system_search/native.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_search_sdk::", "rust-native/tests/system_search_sdk.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
        ];
        for case in gate.native_cases {
            let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert_eq!(
            settings_mapping_state(gate),
            "source-qualified-rust-case-crosswalk"
        );
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }

    #[test]
    fn retired_menu_action_check_has_no_caller_and_preserves_crosswalk() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/MenuActionChecks.swift";
        assert!(!root.join(retired).exists());
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("--check-menu-actions"));
        assert!(!main.contains("checkMenuActions"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"], format!("{:x}", Sha256::digest(main.as_bytes())));
        assert_eq!(MENU_ACTIONS_ORACLE_SHA256, "41d224340e00d1970fb953aad90c3c4a539e6af84634b1aa04b90972c166f1ed");
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        assert_eq!(public["schema"], "ruycast.public-source.v2");
        assert!(public["files"].get(retired).is_none());
        for path in ["Sources/RuycastApp/main.swift", "rust-native/src/bin/ruycast-build/oracles.rs", "rust-native/src/bin/ruycast-source/policy.rs", "rust-native/swift-reference-lock.json"] {
            assert_eq!(public["files"][path], format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())), "stale public source hash: {path}");
        }
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-menu-actions").unwrap();
        assert_eq!(gate.native_cases.len(), 22);
        assert_eq!(gate.required_native_steps, &["unit", "system-search-cli-process", "system-search-sdk"]);
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }

    #[test]
    fn retired_swift_performance_routes_preserve_rust_benchmark_and_crosswalk() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let retired = "Sources/RuycastApp/PerformancePipelineChecks.swift";
        assert!(!root.join(retired).exists());
        let main = std::fs::read_to_string(root.join("Sources/RuycastApp/main.swift")).unwrap();
        assert!(!main.contains("--check-performance-pipeline"));
        assert!(!main.contains("checkPerformancePipeline"));
        assert!(!main.contains("--benchmark-clipboard-poll"));
        assert!(!main.contains("RUYCAST_POLL_BENCHMARK"));
        let performance = std::fs::read_to_string(root.join("Sources/RuycastApp/PerformanceChecks.swift")).unwrap();
        assert!(!performance.contains("benchmarkClipboardPoll"));
        assert!(!performance.contains("RUYCAST_POLL_BENCHMARK"));
        assert!(!include_str!("../ruycast-source/policy.rs").lines().any(|line| line == retired));
        let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("rust-native/swift-reference-lock.json")).unwrap()).unwrap();
        assert!(lock["files"].get(retired).is_none());
        assert_eq!(lock["files"]["Sources/RuycastApp/main.swift"], format!("{:x}", Sha256::digest(main.as_bytes())));
        assert_eq!(lock["files"]["Sources/RuycastApp/PerformanceChecks.swift"], format!("{:x}", Sha256::digest(performance.as_bytes())));
        assert_eq!(PERFORMANCE_PIPELINE_ORACLE_SHA256, "429130589dd29b1c12fea7854b9e868b9d365f85ec8334a80782da999fa90659");
        let roster = std::fs::read_to_string(root.join("bench/f1-runner/src/sdk/workspace_roster.rs")).unwrap();
        let current_performance_roster = roster
            .lines()
            .filter(|line| line.contains("\"frozen-current/Sources/RuycastApp/PerformanceChecks.swift\""))
            .collect::<Vec<_>>();
        assert_eq!(current_performance_roster.len(), 1);
        assert!(current_performance_roster[0].contains("\"a0534165a6df3d6afda628abaefdc8f740b4b0a54a969f814af76e8860382950\",\"line 7: _=NSApplication.shared"));
        assert!(roster.contains("reference-0.8.26/Sources/RuycastApp/PerformancePipelineChecks.swift"));
        let rust_main = std::fs::read_to_string(root.join("rust-native/src/main.rs")).unwrap();
        assert!(rust_main.contains("#[cfg(feature = \"test-tools\")]\n        Some(\"--benchmark-clipboard-poll\")"));
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        assert_eq!(public["schema"], "ruycast.public-source.v2");
        assert!(public["files"].get(retired).is_none());
        for path in ["Sources/RuycastApp/main.swift", "Sources/RuycastApp/PerformanceChecks.swift", "rust-native/src/main.rs", "rust-native/src/native_services.rs", "rust-native/tests/clipboard_poll_benchmark_process.rs", "rust-native/src/bin/ruycast-build/oracles.rs", "rust-native/src/bin/ruycast-source/policy.rs", "rust-native/swift-reference-lock.json", "bench/f1-runner/src/sdk/workspace_roster.rs", "docs/PERFORMANCE.md"] {
            assert_eq!(public["files"][path], format!("{:x}", Sha256::digest(std::fs::read(root.join(path)).unwrap())), "stale public source hash: {path}");
        }
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-performance-pipeline").unwrap();
        assert_eq!(gate.native_cases.len(), 14);
        assert_eq!(gate.required_native_steps, &["unit", "clipboard-history-host-process", "clipboard-rich-paste"]);
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }

    #[test]
    fn clipboard_history_crosswalk_binds_current_rust_consumer_and_existing_steps() {
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        let steps = plan["steps"].as_array().unwrap();
        for id in ["--check-rich-resources", "--check-performance-pipeline"] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert!(gate.required_native_steps.contains(&"clipboard-history-host-process"));
            for name in gate.required_native_steps { assert!(steps.iter().any(|step| step["name"] == *name)); }
            let (_, bytes, expected) = settings_sources(id).iter().find(|(path, _, _)| *path == "rust-native/tests/clipboard_history_host_process.rs").unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), *expected);
            assert!(!settings_sources(id).iter().any(|(path, _, _)| *path == "rust-native/tests/clipboard_history_process.py"));
            assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        }
    }
    #[test]
    fn rich_resources_map_carries_original_groups_and_existing_preview_route() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-rich-resources").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "clipboard-preview-original", "remaining-macos-process", "clipboard-history-host-process", "clipboard-rich-paste"]);
        assert_eq!(gate.native_cases.len(), 16);
        assert!(gate.unproven_invariants.is_empty());
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 39);
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), 39);
        let sources = [
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
            ("clipboard_preview::tests::", "rust-native/src/clipboard_preview.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("processes::tests::", "rust-native/src/processes.rs"),
            ("monitor::lifecycle_tests::", "rust-native/src/monitor.rs"),
            ("clipboard_history_host_process::", "rust-native/tests/clipboard_history_host_process.rs"),
            ("clipboard_rich_paste::", "rust-native/tests/clipboard_rich_paste.rs"),
        ];
        for case in gate.native_cases {
            let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn rich_resources_provenance_keeps_current_inputs_and_gate_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-rich-resources").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, RICH_RESOURCES_ORACLE_SHA256);
        let bindings = settings_source_bindings(gate.id);

        assert_eq!(gate.required_native_steps, &["unit", "clipboard-preview-original", "remaining-macos-process", "clipboard-history-host-process", "clipboard-rich-paste"]);
        assert_eq!(gate.native_cases.len(), 16);
        assert_eq!(inputs.len(), 39);
        assert_eq!(bindings.len(), 40);
        assert!(bindings.contains(&provenance));
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/") || path.starts_with("rust/") || *path == "rust-toolchain.toml"));
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current binding: {path}");
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn polish_provenance_preserves_current_inputs_cases_routes_and_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-polish").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, POLISH_ORACLE_SHA256);
        let bindings = settings_source_bindings(gate.id);
        let case_sources = [
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("native_menu_bar::tests::", "rust-native/src/native_menu_bar.rs"),
            ("runtime_health::tests::", "rust-native/src/runtime_health.rs"),
            ("management_commands::catalog_tests::", "rust-native/src/management_commands.rs"),
            ("clipboard_preview::tests::", "rust-native/src/clipboard_preview.rs"),
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
            ("task_date::tests::", "rust-native/src/task_date.rs"),
        ];

        assert_eq!(gate.required_native_steps, &["unit", "polish-original-process", "script-advanced-process", "task-date-process", "polish-bindings-process", "remaining-macos-process", "clipboard-preview-cancel-original", "clipboard-preview-owner-different-id", "clipboard-preview-cancel-before-publish", "clipboard-preview-failure-after-success", "clipboard-preview-stale-success-same-id"]);
        assert_eq!(gate.native_cases.len(), 14);
        assert!(gate.unproven_invariants.is_empty());
        assert_eq!(inputs.len(), 32);
        assert_eq!(bindings.len(), 33);
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert!(bindings.contains(&provenance));
        assert!(inputs.iter().all(|(path, _, _)| path.starts_with("rust-native/")));
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current Polish binding: {path}");
        }
        for case in gate.native_cases {
            let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn polish_catalog_source_pin_matches_current_catalog_bytes() {
        let (path, bytes, expected) = settings_sources("--check-polish")
            .iter()
            .find(|(path, _, _)| *path == "rust-native/src/catalog.rs")
            .unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), *expected);
        assert_eq!(*expected, "a9848cc7ab9e9432d4ae87c5a3ffb75b3f5a709bbe2cc83a803e19cc916fa72b");
        assert!(settings_source_bindings("--check-polish").contains(&(*path, *expected)));
    }
    #[test]
    fn history_source_pins_match_current_rust_bytes() {
        let expected = "9a44dee7a431ff2f56016d4a984e5966dfd2f4e08e05346a3676f472111cdfa7";
        for gate in ["--check-performance-pipeline", "--check-settings"] {
            let (path, bytes, pin) = settings_sources(gate)
                .iter()
                .find(|(path, _, _)| *path == "rust/src/lib.rs")
                .unwrap();
            assert_eq!(*pin, expected, "{gate} pin for {path}");
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), *pin, "{gate} pin");
        }
    }
    #[test]
    fn native_source_pins_match_current_bytes_across_settings_gates() {
        let mut uses = 0;
        for gate in SETTINGS {
            for (path, bytes, pin) in settings_sources(gate.id) {
                if *path == "rust-native/src/native.rs" {
                    uses += 1;
                    assert_eq!(
                        format!("{:x}", Sha256::digest(bytes)),
                        *pin,
                        "{} source pin",
                        gate.id
                    );
                    assert_eq!(
                        *pin,
                        "aeca7fa758f28ea3162f78b0261417b9a89cdf987afeace27cad9ade954bf6bc",
                        "{} pinned an unexpected native.rs revision",
                        gate.id
                    );
                }
            }
        }
        assert_eq!(uses, 13, "every existing native.rs consumer must be pinned");
    }
    #[test]
    fn feedback_provenance_preserves_inputs_cases_routes_and_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-feedback").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, FEEDBACK_ORACLE_SHA256);
        let additional = settings_additional_provenance_sources(gate.id);
        let bindings = settings_source_bindings(gate.id);
        let case_sources = [
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("utility_process::", "rust-native/tests/utility_process.rs"),
            ("actions::tests::", "rust-native/src/actions.rs"),
            ("actions_process::", "rust-native/tests/actions_process.rs"),
            ("input_process::", "rust-native/tests/input_process.rs"),
            ("processes::tests::", "rust-native/src/processes.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_actions::tests::", "rust-native/src/system_actions.rs"),
            ("file_search::tests::", "rust-native/src/file_search.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
            ("creation_process::", "rust-native/tests/creation_process.rs"),
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
        ];

        assert_eq!(gate.required_native_steps, &["unit", "creation-process", "input-process-rust", "actions-process", "feedback-actions-process", "utility-process", "polish-original-process", "system-search-cli-process", "remaining-macos-process"]);
        assert_eq!(gate.native_cases.len(), 23);
        assert_eq!(inputs.len(), 36);
        assert_eq!(bindings.len(), 40);
        assert_eq!(bindings.iter().collect::<std::collections::BTreeSet<_>>().len(), 40);
        assert_eq!(provenance.1, "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde");
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert_eq!(additional, &[("Sources/RuycastApp/SystemChecks.swift", "c70cc5149a16a43102cc8a853bfb5a38bc6a9b3b542b4f6b7388339f1737ffa4"), ("Sources/RuycastApp/ActionMenuChecks.swift", "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f"), ("Sources/RuycastApp/InteractionChecks.swift", "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858")]);
        assert_eq!(settings_additional_provenance_sources("--check-actions-menu"), &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/InteractionChecks.swift", "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858")]);
        assert_eq!(settings_additional_provenance_sources("--check-interactions"), &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/ActionMenuChecks.swift", "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f")]);
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/SystemChecks.swift"));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/ActionMenuChecks.swift"));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/InteractionChecks.swift"));
        let status_rpc = inputs.iter().find(|entry| entry.0 == "rust-native/tests/support/status_rpc.rs").unwrap();
        assert!(std::str::from_utf8(status_rpc.1).unwrap().contains("benchmark_host::rpc::{connect, request}"));
        let rpc = inputs.iter().find(|entry| entry.0 == "rust-native/src/benchmark_host/rpc.rs").unwrap();
        assert_eq!(rpc.2, "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9");
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn connect("));
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn request("));
        for provenance in additional { assert!(bindings.contains(provenance)); }
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current feedback binding: {path}");
        }
        for case in gate.native_cases {
            let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        let plan = super::super::validation::plan("aarch64-apple-darwin").unwrap();
        let planned_steps = plan["steps"].as_array().unwrap();
        for name in gate.required_native_steps {
            assert!(planned_steps.iter().any(|step| step["name"] == *name), "missing feedback route: {name}");
        }
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn interactions_provenance_preserves_inputs_cases_routes_and_status() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-interactions").unwrap();
        let inputs = settings_sources(gate.id);
        let provenance = (gate.original_source, RENDER_UPDATES_ORACLE_SHA256);
        let bindings = settings_source_bindings(gate.id);
        let case_sources = [
            ("bridge_process::", "rust-native/tests/bridge_process.rs"),
            ("input_process::", "rust-native/tests/input_process.rs"),
            ("native_open_basic_process::", "rust-native/tests/native_open_basic_process.rs"),
            ("creation_process::", "rust-native/tests/creation_process.rs"),
            ("library_lifecycle_process::", "rust-native/tests/library_lifecycle_process.rs"),
            ("settings_preferences_process::", "rust-native/tests/settings_preferences_process.rs"),
            ("native_choice_host_process::", "rust-native/tests/native_choice_host_process.rs"),
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("native_edit::", "rust-native/src/native_edit.rs"),
            ("native_snippet_choice_process::", "rust-native/tests/native_snippet_choice_process.rs"),
        ];

        assert_eq!(gate.required_native_steps, &["bridge-process", "native-profile-process", "input-process-rust", "native-choice-host-process", "native-open-basic-process", "creation-process", "library-process", "settings-preferences-process", "feedback-actions-process", "native-snippet-choice-process-rust", "native-light-600", "native-light-760", "native-dark-600", "native-dark-760"]);
        assert_eq!(gate.native_cases.len(), 16);
        assert_eq!(inputs.len(), 44);
        assert_eq!(bindings.len(), 47);
        assert_eq!(provenance.1, "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858");
        assert_eq!(settings_provenance_source(gate.id), Some(provenance));
        assert!(!inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/FeedbackChecks.swift"));
        assert!(!inputs.iter().any(|(path, _, _)| *path == "Sources/RuycastApp/ActionMenuChecks.swift"));
        let additional = settings_additional_provenance_sources(gate.id);
        assert_eq!(additional, &[("Sources/RuycastApp/FeedbackChecks.swift", "03e96e9101f7a9045606962dbf23d3f1082f5e8adfa1ffaea84e37cd26ffadde"), ("Sources/RuycastApp/ActionMenuChecks.swift", "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f")]);
        for provenance in additional { assert!(bindings.contains(provenance)); }
        for (path, _, sha) in inputs {
            assert!(bindings.contains(&(path, sha)), "missing current interactions binding: {path}");
        }
        let status_rpc = inputs.iter().find(|entry| entry.0 == "rust-native/tests/support/status_rpc.rs").unwrap();
        assert!(std::str::from_utf8(status_rpc.1).unwrap().contains("benchmark_host::rpc::{connect, request}"));
        let rpc = inputs.iter().find(|entry| entry.0 == "rust-native/src/benchmark_host/rpc.rs").unwrap();
        assert_eq!(rpc.2, "1faf3f1955d922d30dc409fddb6ab499931c3ae3918b651f32cec21c97d206f9");
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn connect("));
        assert!(std::str::from_utf8(rpc.1).unwrap().contains("pub fn request("));
        for case in gate.native_cases {
            let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert_eq!(gate.required_native_steps.len(), 14);
        assert_eq!(settings_mapping_state(gate), "source-qualified-rust-case-crosswalk");
        let inventory = inventory("settings").unwrap();
        assert_eq!(inventory["productionAccepted"], false);
        assert!(!inventory["unmappedGateIDs"].as_array().unwrap().contains(&serde_json::json!(gate.id)));
    }
    #[test]
    fn polish_composition_and_partial_original_gates_keep_precise_cases_and_refusals() {
        let sources = [
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("native_menu_bar::tests::", "rust-native/src/native_menu_bar.rs"),
            ("runtime_health::tests::", "rust-native/src/runtime_health.rs"),
            ("management_commands::catalog_tests::", "rust-native/src/management_commands.rs"),
            ("clipboard_preview::tests::", "rust-native/src/clipboard_preview.rs"),
            ("task_date::tests::", "rust-native/src/task_date.rs"),
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
            ("utility_process::", "rust-native/tests/utility_process.rs"),
            ("actions::tests::", "rust-native/src/actions.rs"),
            ("processes::tests::", "rust-native/src/processes.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("actions_process::", "rust-native/tests/actions_process.rs"),
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_actions::tests::", "rust-native/src/system_actions.rs"),
            ("file_search::tests::", "rust-native/src/file_search.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
            ("native_edit::", "rust-native/src/native_edit.rs"),
            ("native_snippet_choice_process::", "rust-native/tests/native_snippet_choice_process.rs"),
            ("native_keyboard_process::", "rust-native/tests/native_keyboard_process.rs"),
            ("bridge_process::", "rust-native/tests/bridge_process.rs"),
            ("input_process::", "rust-native/tests/input_process.rs"),
            ("native_open_basic_process::", "rust-native/tests/native_open_basic_process.rs"),
            ("creation_process::", "rust-native/tests/creation_process.rs"),
            ("library_lifecycle_process::", "rust-native/tests/library_lifecycle_process.rs"),
            ("library_checkbox_process::", "rust-native/tests/library_checkbox_process.rs"),
            ("library_drafts_process::", "rust-native/tests/library_drafts_process.rs"),
            ("library_navigation_process::", "rust-native/tests/library_navigation_process.rs"),
            ("settings_preferences_process::", "rust-native/tests/settings_preferences_process.rs"),
            ("native_choice_host_process::", "rust-native/tests/native_choice_host_process.rs"),
        ];
        for (id, count, input_count, complete) in [
            ("--check-polish", 10, 32, true),
            ("--check-feedback", 23, 36, true),
            ("--check-actions-menu", 11, 25, true),
            ("--check-interactions", 16, 44, true),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.native_cases.len(), count);
            assert_eq!(gate.unproven_invariants.is_empty(), complete);
            let inputs = settings_sources(id);
            assert_eq!(inputs.len(), input_count);
            assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), input_count);
            assert!(settings_source_bindings(id).iter().any(|(path, _)| *path == gate.original_source));
            for case in gate.native_cases {
                let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
                let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
                let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
                assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{id}: {case}");
                if prefix.ends_with('.') {
                    let class = case.strip_prefix(*prefix).unwrap().split('.').next().unwrap();
                    assert!(source.contains(&format!("class {class}(")), "{id}: {case}");
                }
            }
            if !complete {
                assert_eq!(settings_mapping_state(gate), "mapped-with-unproven-invariants");
            }
        }
        let polish = SETTINGS.iter().find(|gate| gate.id == "--check-polish").unwrap();
        assert_eq!(polish.required_native_steps, &["unit", "polish-original-process", "script-advanced-process", "task-date-process", "polish-bindings-process", "remaining-macos-process", "clipboard-preview-cancel-original", "clipboard-preview-owner-different-id", "clipboard-preview-cancel-before-publish", "clipboard-preview-failure-after-success", "clipboard-preview-stale-success-same-id"]);
        let matrix = include_str!("validation.rs");
        assert!(matrix.contains("polish-original-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --polish-original-only"));
        assert!(matrix.contains("clipboard-preview-cancel-original|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset --locked -- --exact"));
        assert!(matrix.contains("clipboard-preview-owner-different-id|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::equal_metadata_different_id_replaces_preview_owner_and_releases_old_image --locked -- --exact"));
        assert!(matrix.contains("clipboard-preview-cancel-before-publish|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::decoded_preview_cancelled_before_poll_is_not_published_and_released --locked -- --exact"));
        assert!(matrix.contains("clipboard-preview-failure-after-success|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::failed_reopen_releases_published_preview_and_clears_on_deselect --locked -- --exact"));
        assert!(matrix.contains("clipboard-preview-stale-success-same-id|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::stale_success_cannot_replace_same_id_reopened_preview --locked -- --exact"));
        assert!(matrix.contains("actions-menu-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --actions-menu-only"));
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn settings_inventory_qualifies_all_crosswalks_without_product_acceptance() {
        let value = inventory("settings").unwrap();
        let unresolved = value["unmappedGateIDs"].as_array().unwrap();
        let expected_unresolved = serde_json::json!([]);
        assert_eq!(
            unresolved,
            expected_unresolved
            .as_array()
            .unwrap()
        );
        assert!(!unresolved.contains(&serde_json::json!("--check-edit-commands")));
        let gates = value["gates"].as_array().unwrap();
        assert_eq!(gates.len(), 16);
        for gate in gates {
            if gate[0] == "--check-edit-commands"
                || gate[0] == "--check-analytics"
                || gate[0] == "--check-command-log"
                || gate[0] == "--check-visual-assets"
                || gate[0] == "--check-render-updates"
                || gate[0] == "--check-browser-tabs"
                || gate[0] == "--check-actions-menu"
                || gate[0] == "--check-performance-pipeline"
                || gate[0] == "--check-feedback"
                || gate[0] == "--check-interactions"
                || gate[0] == "--check-window-search"
                || gate[0] == "--check-menu-actions"
                || gate[0] == "--check-rich-resources"
                || gate[0] == "--check-polish"
                || gate[0] == "--check-apple-shortcuts"
                || gate[0] == "--check-settings"
            {
                assert_eq!(gate[1], "source-qualified-rust-case-crosswalk");
            } else {
                assert_eq!(gate[1], "mapped-requires-source-requalification");
            }
        }
        let edit = SETTINGS.iter().find(|gate| gate.id == "--check-edit-commands").unwrap();
        assert_eq!(edit.native_cases, &["native_edit::check"]);
        assert_eq!(edit.required_native_steps, &["native-light-600", "native-light-760", "native-dark-600", "native-dark-760"]);
        let edit_bindings = settings_source_bindings(edit.id);
        assert_eq!(edit_bindings.len(), 3);
        assert!(edit_bindings.contains(&(
            "rust-native/src/native_edit.rs",
            "985f3d19de00ee70242e9310cef1373c2a330b5bc70a04263af90ce24ed58e0d",
        )));
        assert!(edit_bindings.contains(&(
            "rust-native/src/native_edit_menu.rs",
            "8213290295ec55cd13adf37a5fd30f6648854fadc471272be9ed49e3385c438b",
        )));
        assert!(edit_bindings.contains(&(
            edit.original_source,
            "a91248845b9256c30f2906adc818e5e706034b6277a52519df2f8758203b1145",
        )));
        assert_eq!(
            value["qualifiedCaseSourceBindings"][edit.id].as_object().unwrap().len(),
            3,
        );
        assert_eq!(
            value["qualifiedCaseSourceBindings"][edit.id][edit.original_source],
            "a91248845b9256c30f2906adc818e5e706034b6277a52519df2f8758203b1145",
        );
        assert!(value["settingsCaseCrosswalk"]
            .as_array()
            .unwrap()
            .iter()
            .all(|gate| [
                "--check-command-log",
                "--check-edit-commands",
                "--check-analytics",
                "--check-visual-assets",
                "--check-settings",
                "--check-browser-tabs",
                "--check-performance-pipeline",
                "--check-window-search",
                "--check-render-updates",
                "--check-menu-actions",
                "--check-rich-resources",
                "--check-polish",
                "--check-apple-shortcuts",
                "--check-feedback",
                "--check-actions-menu",
                "--check-interactions",
            ]
            .iter()
            .any(|id| gate["id"] == *id)
                || !gate["unprovenInvariants"].as_array().unwrap().is_empty()));
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("check").unwrap()["unmappedGateIDs"], serde_json::json!([]));
        assert_eq!(
            inventory("check").unwrap()["gates"].as_array().unwrap().iter()
                .find(|gate| gate[0] == "git-diff-check").unwrap()[1],
            "source-qualified-root-bound-whitespace-check"
        );
        assert_eq!(root_git_mapping_state(include_bytes!("compat.rs")), "source-qualified-root-bound-whitespace-check");
        assert_eq!(root_git_mapping_state(b"unreviewed compatibility source"), "requires-source-requalification");
        assert!(require_complete("check").is_ok());
        assert!(require_complete("unknown").is_err());
        assert_eq!(value["productionAccepted"], false);
    }
    #[test]
    fn absent_sources_and_explicit_unproven_invariants_still_refuse_qualification() {
        let unknown = SettingsGate {
            id: "--not-reviewed", original_source: "Sources/RuycastApp/PolishChecks.swift",
            required_native_steps: &[], native_cases: &["owned-case"], unproven_invariants: &[],
        };
        assert_eq!(settings_mapping_state(&unknown), "mapped-requires-source-requalification");
        let unproven = SettingsGate {
            id: "--check-polish", unproven_invariants: &["not yet proved"], ..unknown
        };
        assert_eq!(settings_mapping_state(&unproven), "mapped-with-unproven-invariants");
    }
}
