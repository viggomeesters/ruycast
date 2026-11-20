//! Unsigned, source-bound app staging. No signing, installation or publication.
use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

const CANDIDATE_SCHEMA: &str = "ruycast.rust-unsigned-candidate.v2";

fn accepted_notary_submission_id(bytes: &[u8]) -> Result<String> {
    let result: serde_json::Value = serde_json::from_slice(bytes).map_err(error)?;
    if result.get("status").and_then(serde_json::Value::as_str) != Some("Accepted") {
        return Err("Notarization not accepted; inspect the submission log.".into());
    }
    result
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Accepted notarization result has no submission ID.".into())
}

pub(super) fn notary_result(args: &[String]) -> Result<Option<String>> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok(None);
    }
    if args.len() != 2 || args[0] != "--file" {
        return Err("Usage: ruycast-build notary-result --file PATH".into());
    }
    let bytes = fs::read(Path::new(&args[1])).map_err(error)?;
    accepted_notary_submission_id(&bytes).map(Some)
}

fn release_receipt_value(
    candidate: &[u8],
    notarization: &[u8],
    zip: &[u8],
    public_commit: &str,
    signature: &str,
) -> Result<serde_json::Value> {
    let unsigned_candidate: serde_json::Value = serde_json::from_slice(candidate).map_err(error)?;
    let notarization_value: serde_json::Value =
        serde_json::from_slice(notarization).map_err(error)?;
    accepted_notary_submission_id(notarization)?;
    if public_commit.trim().is_empty() || signature.trim().is_empty() {
        return Err("Release receipt needs the public commit and codesign details.".into());
    }
    Ok(serde_json::json!({
        "unsignedCandidate": unsigned_candidate,
        "notarization": notarization_value,
        "zipSHA256": hash(zip),
        "publicCommit": public_commit.trim(),
        "signature": signature,
    }))
}

pub(super) fn release_receipt(args: &[String]) -> Result<()> {
    if args.len() != 6
        || args[0] != "--candidate"
        || args[2] != "--public-commit"
        || args[4] != "--signature"
    {
        return Err("Usage: ruycast-build release-receipt --candidate PATH --public-commit HASH --signature TEXT".into());
    }
    no_links(Path::new(&args[1]))?;
    let candidate = fs::canonicalize(&args[1]).map_err(error)?;
    let record = release_receipt_value(
        &regular_bytes(&candidate.join("candidate.json"))?,
        &regular_bytes(&candidate.join("notary-result.json"))?,
        &regular_bytes(&candidate.join("Ruycast-macOS.zip"))?,
        &args[3],
        &args[5],
    )?;
    let bytes = serde_json::to_vec_pretty(&record).map_err(error)?;
    let target = candidate.join("release-receipt.json");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target)
        .map_err(error)?;
    output.write_all(&bytes).map_err(error)?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    sha256: String,
    mode: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Candidate {
    schema: String,
    version: String,
    build: u64,
    build_receipt_sha256: String,
    public_source_sha256: Option<String>,
    files: BTreeMap<String, Entry>,
    production_accepted: bool,
    signed_for_distribution: bool,
    notarized: bool,
    installed_or_published: bool,
}
struct AppInputs {
    files: BTreeMap<String, Entry>,
    icon: Option<Vec<u8>>,
    info: Vec<u8>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn no_links(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        prefix.push(part);
        if fs::symlink_metadata(&prefix)
            .map_err(error)?
            .file_type()
            .is_symlink()
        {
            return Err(format!("Linked candidate input: {}", prefix.display()));
        }
    }
    Ok(())
}
fn version() -> Result<&'static str> {
    let value = rustcast_core::product_version::CURRENT;
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|p| {
            p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
                || p.parse::<u32>().is_err()
        })
    {
        return Err(
            "The compiled candidate version must be a stable three-part semantic version.".into(),
        );
    }
    Ok(value)
}
fn plist(version: &str, build: u64, icon: bool) -> Vec<u8> {
    // Version/build are validated numeric literals; every other value is fixed.
    let icon = if icon {
        "<key>CFBundleIconFile</key><string>AppIcon</string>\n"
    } else {
        ""
    };
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>nl.viggomeesters.ruycast</string>
<key>CFBundleName</key><string>Ruycast</string>
<key>CFBundleDisplayName</key><string>Ruycast</string>
<key>CFBundleExecutable</key><string>RuycastApp</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{build}</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
<key>NSAppleEventsUsageDescription</key><string>Ruycast searches open Safari tabs and switches to the tab you select.</string>
{icon}</dict></plist>
"#).into_bytes()
}
fn expected(root: &Path, build: u64, receipt: &Receipt) -> Result<AppInputs> {
    if build == 0 {
        return Err("Build number must be positive.".into());
    }
    let mut files = BTreeMap::new();
    for (source, component) in artifact_paths(&receipt.target)? {
        let target = if component == "host" {
            "Contents/MacOS/RuycastApp"
        } else {
            "Contents/Resources/ruycast"
        };
        no_links(&root.join(&source))?;
        files.insert(
            target.into(),
            Entry {
                sha256: receipt
                    .artifacts
                    .get(&source)
                    .ok_or("Incomplete Rust build receipt.")?
                    .clone(),
                mode: 0o755,
            },
        );
    }
    let icon_path = root.join("assets/AppIcon.icns");
    let icon = match fs::symlink_metadata(&icon_path) {
        Ok(_) => {
            no_links(&icon_path)?;
            Some(regular_bytes(&icon_path)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(error(e)),
    };
    if let Some(bytes) = &icon {
        files.insert(
            "Contents/Resources/AppIcon.icns".into(),
            Entry {
                sha256: hash(bytes),
                mode: 0o644,
            },
        );
    }
    let info = plist(version()?, build, icon.is_some());
    files.insert(
        "Contents/Info.plist".into(),
        Entry {
            sha256: hash(&info),
            mode: 0o644,
        },
    );
    Ok(AppInputs { files, icon, info })
}
fn tree(path: &Path, relative: &str, files: &mut BTreeMap<String, Entry>) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if meta.file_type().is_symlink() {
        return Err("Candidate app contains a symlink.".into());
    }
    if meta.is_dir() {
        for item in fs::read_dir(path).map_err(error)? {
            let item = item.map_err(error)?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| "Non-UTF8 candidate path.")?;
            tree(
                &item.path(),
                &if relative.is_empty() {
                    name
                } else {
                    format!("{relative}/{name}")
                },
                files,
            )?;
        }
    } else if meta.is_file() && meta.nlink() == 1 {
        files.insert(
            relative.into(),
            Entry {
                sha256: digest(path)?,
                mode: meta.permissions().mode() & 0o7777,
            },
        );
    } else {
        return Err("Candidate contains nonregular or multiply linked content.".into());
    }
    Ok(())
}
fn payload(app: &Path, expected: &BTreeMap<String, Entry>) -> Result<()> {
    no_links(app)?;
    let mut actual = BTreeMap::new();
    tree(app, "", &mut actual)?;
    if &actual != expected {
        return Err("Candidate app bytes, modes or file inventory changed.".into());
    }
    Ok(())
}
fn put(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("Candidate parent missing.")?).map_err(error)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(error)?;
    std::io::Write::write_all(&mut file, bytes).map_err(error)?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(error)?;
    file.sync_all().map_err(error)
}
fn build_receipt(root: &Path) -> Result<(Receipt, String)> {
    let path = root.join(OUTPUT).join("development-build.json");
    no_links(&path)?;
    let bytes = regular_bytes(&path)?;
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(error)?;
    verify_value(root, &receipt)?;
    if receipt.configuration != "release" || receipt.release_preparation.is_none() {
        return Err("Packaging requires a prepared release Rust build.".into());
    }
    Ok((receipt, hash(&bytes)))
}
pub(super) fn stage(root: &Path, build: u64) -> Result<PathBuf> {
    let lock = BuildLock::acquire(root)?;
    let parent = root.join(OUTPUT).join("candidates");
    ruycast_native::private_storage::secure_directory(&parent)?;
    no_links(&parent)?;
    let destination = parent.join(format!("{}-{build}", version()?));
    stage_under_lease(root, build, &destination, &lock, None)?;
    Ok(destination)
}
pub(super) fn stage_public(
    root: &Path,
    build: u64,
    destination: &Path,
) -> Result<serde_json::Value> {
    if build == 0 {
        return Err("Build number must be positive.".into());
    }
    let lock = BuildLock::acquire(root)?;
    let source = public_source::bind(root)?;
    ensure_parent(destination.parent().ok_or("Missing candidate parent.")?)?;
    stage_under_lease(root, build, destination, &lock, Some(source))?;
    verify_under_lease(root, destination, &lock)
}
fn ensure_parent(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        if !matches!(
            part,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        ) {
            return Err("Unsafe candidate parent.".into());
        }
        prefix.push(part);
        let meta = match fs::symlink_metadata(&prefix) {
            Ok(meta) => meta,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&prefix).map_err(error)?;
                fs::symlink_metadata(&prefix).map_err(error)?
            }
            Err(failure) => return Err(error(failure)),
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("Candidate parent contains a link or non-directory.".into());
        }
    }
    Ok(())
}
fn stage_under_lease(
    root: &Path,
    build: u64,
    destination: &Path,
    lock: &BuildLock,
    public_source_sha256: Option<String>,
) -> Result<()> {
    stage_with(root, build, destination, lock, public_source_sha256, || {
        Ok(())
    })
}
fn stage_with(
    root: &Path,
    build: u64,
    destination: &Path,
    lock: &BuildLock,
    public_source_sha256: Option<String>,
    before_final: impl FnOnce() -> Result<()>,
) -> Result<()> {
    lock.validate()?;
    if !destination.is_absolute()
        || destination.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("Choose an absolute new candidate directory without dot components.".into());
    }
    no_links(destination.parent().ok_or("Missing candidate parent.")?)?;
    let (receipt, receipt_hash) = build_receipt(root)?;
    let AppInputs { files, icon, info } = expected(root, build, &receipt)?;
    // Exclusive reservation. Existing output is never removed or replaced.
    fs::create_dir(destination).map_err(error)?;
    let identity = fs::symlink_metadata(destination).map_err(error)?;
    let namespace = || -> Result<()> {
        no_links(destination)?;
        let actual = fs::symlink_metadata(destination).map_err(error)?;
        if !actual.is_dir()
            || actual.file_type().is_symlink()
            || (actual.dev(), actual.ino()) != (identity.dev(), identity.ino())
        {
            return Err("Candidate output namespace changed.".into());
        }
        lock.validate()
    };
    let result = (|| {
        namespace()?;
        let app = destination.join("Ruycast.app");
        for (source, component) in artifact_paths(&receipt.target)? {
            namespace()?;
            let target = if component == "host" {
                "Contents/MacOS/RuycastApp"
            } else {
                "Contents/Resources/ruycast"
            };
            put(
                &app.join(target),
                &regular_bytes(&root.join(&source))?,
                0o755,
            )?;
        }
        if let Some(bytes) = icon {
            put(&app.join("Contents/Resources/AppIcon.icns"), &bytes, 0o644)?;
        }
        put(&app.join("Contents/Info.plist"), &info, 0o644)?;
        payload(&app, &files)?;
        before_final()?;
        if build_receipt(root)?.1 != receipt_hash || expected(root, build, &receipt)?.files != files
        {
            return Err("Build inputs/resources changed while staging the app.".into());
        }
        if let Some(source) = &public_source_sha256 {
            if public_source::bind(root)? != *source {
                return Err("Public source changed while staging.".into());
            }
        }
        namespace()?;
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            version: version()?.into(),
            build,
            build_receipt_sha256: receipt_hash,
            public_source_sha256,
            files,
            production_accepted: false,
            signed_for_distribution: false,
            notarized: false,
            installed_or_published: false,
        };
        put(
            &destination.join("candidate.json"),
            &serde_json::to_vec_pretty(&candidate).map_err(error)?,
            0o600,
        )?;
        File::open(destination)
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
        Ok(())
    })();
    if let Err(error) = result {
        if namespace().is_ok() {
            let _ = fs::remove_dir_all(destination);
        }
        return Err(error);
    }
    namespace()
}
pub(super) fn verify(root: &Path, destination: &Path) -> Result<serde_json::Value> {
    let lock = BuildLock::acquire(root)?;
    verify_under_lease(root, destination, &lock)
}
fn verify_under_lease(
    root: &Path,
    destination: &Path,
    lock: &BuildLock,
) -> Result<serde_json::Value> {
    lock.validate()?;
    if !destination.is_absolute() {
        return Err("Choose an absolute candidate path.".into());
    }
    no_links(destination)?;
    let value: Candidate =
        serde_json::from_slice(&regular_bytes(&destination.join("candidate.json"))?)
            .map_err(error)?;
    if let Some(expected) = &value.public_source_sha256 {
        if public_source::bind(root)? != *expected {
            return Err("Candidate public source manifest changed.".into());
        }
    } else if destination.parent() != Some(root.join(OUTPUT).join("candidates").as_path()) {
        return Err("Development candidate must remain in its declared build directory.".into());
    }
    let (receipt, receipt_hash) = build_receipt(root)?;
    let AppInputs { files, .. } = expected(root, value.build, &receipt)?;
    if value.schema != CANDIDATE_SCHEMA
        || value.version != version()?
        || value.build_receipt_sha256 != receipt_hash
        || value.files != files
        || value.production_accepted
        || value.signed_for_distribution
        || value.notarized
        || value.installed_or_published
        || (value.public_source_sha256.is_none()
            && destination.file_name().and_then(|s| s.to_str())
                != Some(format!("{}-{}", version()?, value.build).as_str()))
    {
        return Err("Stale or invalid unsigned Rust candidate receipt.".into());
    }
    payload(&destination.join("Ruycast.app"), &files)?;
    Ok(
        serde_json::json!({"ok":true,"candidate":destination,"version":value.version,"build":value.build,"files":files,"productionAccepted":false,"installedOrPublished":false,"signedForDistribution":false,"notarized":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_notary_result_returns_only_its_submission_id() {
        assert_eq!(
            accepted_notary_submission_id(br#"{"status":"Accepted","id":"submission-123"}"#),
            Ok("submission-123".into())
        );
    }
    #[test]
    fn non_accepted_or_missing_notary_status_is_rejected() {
        for bytes in [
            br#"{"status":"In Progress","id":"submission-123"}"#.as_slice(),
            br#"{"status":"Rejected","id":"submission-123"}"#,
            br#"{"id":"submission-123"}"#,
        ] {
            assert!(accepted_notary_submission_id(bytes).is_err());
        }
    }
    #[test]
    fn missing_notary_id_or_malformed_json_is_rejected() {
        for bytes in [
            br#"{"status":"Accepted"}"#.as_slice(),
            br#"{"status":"Accepted","id":""}"#,
            br#"{"status":"Accepted","id":123}"#,
            b"{",
        ] {
            assert!(accepted_notary_submission_id(bytes).is_err());
        }
    }
    #[test]
    fn release_receipt_binds_candidate_notarization_zip_commit_and_signature() {
        let candidate = br#"{"build":68,"productionAccepted":false}"#;
        let notarization = br#"{"status":"Accepted","id":"submission-68"}"#;
        let record = release_receipt_value(
            candidate,
            notarization,
            b"synthetic final zip",
            "abc123\n",
            "Identifier=nl.viggomeesters.ruycast\nTeamIdentifier=TEAM123",
        )
        .unwrap();
        assert_eq!(record["unsignedCandidate"]["build"], 68);
        assert_eq!(record["notarization"]["id"], "submission-68");
        assert_eq!(record["zipSHA256"], hash(b"synthetic final zip"));
        assert_eq!(record["publicCommit"], "abc123");
        assert!(record["signature"].as_str().unwrap().contains("TEAM123"));
    }
    #[test]
    fn release_receipt_refuses_unaccepted_notary_or_missing_capture() {
        let candidate = br#"{"build":68}"#;
        let accepted = br#"{"status":"Accepted","id":"submission-68"}"#;
        assert!(release_receipt_value(candidate, br#"{"status":"Invalid"}"#, b"zip", "abc", "sig").is_err());
        assert!(release_receipt_value(candidate, accepted, b"zip", "", "sig").is_err());
        assert!(release_receipt_value(candidate, accepted, b"zip", "abc", " ").is_err());
    }
    #[test]
    fn real_owned_prepared_pair_stages_exclusively_and_rejects_mixed_source() {
        let f = super::super::tests::Fixture::compiler_pair();
        let lease = BuildLock::acquire_from_environment(&f.0, &BTreeMap::new()).unwrap();
        build_under_lease(&f.0, "release", &lease).unwrap();
        fs::write(f.0.join("private-profile.json"), b"synthetic private data").unwrap();
        fs::create_dir(f.0.join("scripts")).unwrap();
        fs::write(
            f.0.join("scripts/personal_bridge.py"),
            b"synthetic local adapter",
        )
        .unwrap();
        let parent = f.0.join(OUTPUT).join("candidates");
        ruycast_native::private_storage::secure_directory(&parent).unwrap();
        let destination = parent.join(format!("{}-68", version().unwrap()));
        stage_under_lease(&f.0, 68, &destination, &lease, None).unwrap();
        let value = verify_under_lease(&f.0, &destination, &lease).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["productionAccepted"], false);
        assert_eq!(value["signedForDistribution"], false);
        assert_eq!(value["notarized"], false);
        let app = destination.join("Ruycast.app");
        let mut files = BTreeMap::new();
        tree(&app, "", &mut files).unwrap();
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "Contents/Info.plist",
                "Contents/MacOS/RuycastApp",
                "Contents/Resources/ruycast"
            ]
        );
        let info =
            String::from_utf8(regular_bytes(&app.join("Contents/Info.plist")).unwrap()).unwrap();
        let short_version = format!(
            "<key>CFBundleShortVersionString</key><string>{}</string>",
            version().unwrap()
        );
        for entry in [
            "<key>CFBundleIdentifier</key><string>nl.viggomeesters.ruycast</string>",
            "<key>CFBundleVersion</key><string>68</string>",
            "<key>LSUIElement</key><true/>",
            short_version.as_str(),
        ] {
            assert!(info.contains(entry), "missing bundle metadata: {entry}");
        }
        let candidate: serde_json::Value =
            serde_json::from_slice(&regular_bytes(&destination.join("candidate.json")).unwrap())
                .unwrap();
        assert_eq!(candidate["productionAccepted"], false);
        assert_eq!(candidate["signedForDistribution"], false);
        assert_eq!(candidate["notarized"], false);
        assert_eq!(candidate["installedOrPublished"], false);
        assert!(candidate.get("publishable").is_none());
        let executable = app.join("Contents/Resources/ruycast");
        let executable_bytes = regular_bytes(&executable).unwrap();
        fs::write(&executable, b"changed staged Rust executable").unwrap();
        assert!(verify_under_lease(&f.0, &destination, &lease).is_err());
        fs::write(&executable, &executable_bytes).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let extra = app.join("Contents/Resources/private.txt");
        put(&extra, b"unreviewed", 0o644).unwrap();
        assert!(verify_under_lease(&f.0, &destination, &lease).is_err());
        fs::remove_file(extra).unwrap();
        assert_eq!(
            verify_under_lease(&f.0, &destination, &lease).unwrap()["ok"],
            true
        );
        let before = regular_bytes(&destination.join("candidate.json")).unwrap();
        assert!(stage_under_lease(&f.0, 68, &destination, &lease, None).is_err());
        assert_eq!(
            regular_bytes(&destination.join("candidate.json")).unwrap(),
            before
        );
        let occupied = parent.join("owned-existing");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("human-marker"), b"preserve candidate").unwrap();
        assert!(stage_under_lease(&f.0, 69, &occupied, &lease, None).is_err());
        assert_eq!(
            regular_bytes(&occupied.join("human-marker")).unwrap(),
            b"preserve candidate"
        );
        let link = parent.join("owned-link");
        std::os::unix::fs::symlink(&destination, &link).unwrap();
        assert!(stage_under_lease(&f.0, 69, &link, &lease, None).is_err());
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            regular_bytes(&destination.join("candidate.json")).unwrap(),
            before
        );
        let bad = parent.join("owned-zero");
        assert!(stage_under_lease(&f.0, 0, &bad, &lease, None).is_err());
        assert!(!bad.exists());
        let receipt = build_receipt(&f.0).unwrap().0;
        let artifact = f.0.join(&artifact_paths(&receipt.target).unwrap()[1].0);
        let original_artifact = regular_bytes(&artifact).unwrap();
        fs::write(&artifact, b"stale owned Rust artifact").unwrap();
        let stale = parent.join("owned-stale-artifact");
        assert!(stage_under_lease(&f.0, 69, &stale, &lease, None).is_err());
        assert!(!stale.exists());
        fs::write(&artifact, &original_artifact).unwrap();
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
        let mixed = parent.join("owned-mixed");
        let config = f.0.join("config/default.json");
        let original = regular_bytes(&config).unwrap();
        let result = stage_with(&f.0, 69, &mixed, &lease, None, || {
            fs::write(&config, b"owned config mutation").map_err(error)
        });
        assert!(result.is_err());
        assert!(!mixed.exists());
        fs::write(config, original).unwrap();
        let payload = destination.join("Ruycast.app/Contents/Resources/ruycast");
        fs::write(&payload, b"owned payload tamper").unwrap();
        assert!(verify_under_lease(&f.0, &destination, &lease).is_err());
    }
    #[test]
    fn public_candidate_refuses_private_checkout_or_missing_manifest_before_staging() {
        const CHILD: &str = "RUYCAST_PUBLIC_CANDIDATE_TEST_CHILD";
        const NAME: &str = "candidate::tests::public_candidate_refuses_private_checkout_or_missing_manifest_before_staging";
        if std::env::var_os(CHILD).is_none() {
            let fixture_parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("explicit owned fixture parent");
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", NAME, "--test-threads=1"])
                .env_clear()
                .env(CHILD, "1")
                .env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", fixture_parent);
            let mut child = child.spawn().unwrap();
            let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "owned candidate test child exited {status}");
                    break;
                }
                if std::time::Instant::now() >= until {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("owned candidate test child timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            return;
        }
        for private_checkout in [true, false] {
            let fixture = super::super::tests::Fixture::new();
            if private_checkout {
                fs::create_dir(fixture.0.join(".go")).unwrap();
            }
            let parent = fixture.0.join("candidate-output");
            let destination = parent.join("candidate");
            let error = stage_public(&fixture.0, 68, &destination).unwrap_err();
            if private_checkout {
                assert!(error.contains("reviewed public source export"), "{error}");
            }
            assert!(
                !parent.exists(),
                "candidate staging began before source validation"
            );
        }
    }
    #[test]
    fn safari_automation_purpose_survives_rust_packaging_with_or_without_icon() {
        for icon in [true, false] {
            let text = String::from_utf8(plist("0.9.0", 68, icon)).unwrap();
            assert!(text.contains("<key>NSAppleEventsUsageDescription</key><string>Ruycast searches open Safari tabs and switches to the tab you select.</string>"));
            assert_eq!(text.contains("CFBundleIconFile"), icon);
            assert!(text.contains("<key>CFBundleVersion</key><string>68</string>"));
        }
    }
    #[test]
    fn payload_verification_rejects_tampering_extra_helpers_modes_and_links() {
        let fixture = super::super::tests::Fixture::new();
        let app = fixture.0.join("Ruycast.app");
        let executable = app.join("Contents/MacOS/RuycastApp");
        put(&executable, b"owned fixture executable", 0o755).unwrap();
        let mut expected = BTreeMap::new();
        tree(&app, "", &mut expected).unwrap();
        payload(&app, &expected).unwrap();
        fs::write(&executable, b"changed payload").unwrap();
        assert!(payload(&app, &expected).is_err());
        fs::write(&executable, b"owned fixture executable").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(payload(&app, &expected).is_err());
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let helper = app.join("Contents/MacOS/unreviewed-helper");
        put(&helper, b"unexpected", 0o755).unwrap();
        assert!(payload(&app, &expected).is_err());
        fs::remove_file(&helper).unwrap();
        std::os::unix::fs::symlink(&executable, &helper).unwrap();
        assert!(payload(&app, &expected).is_err());
        fs::remove_file(helper).unwrap();
        payload(&app, &expected).unwrap();
    }
}
