use super::tools::Runner;
use super::*;
use std::{
    fs as stdfs,
    os::unix::fs::{symlink, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
};
struct Fixture {
    root: PathBuf,
    o: Options,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_INSTALL_TEST_ROOT")
                .expect("Explicit owned installer fixtureparent required"),
        );
        stdfs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "in{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        stdfs::create_dir(&root).unwrap();
        stdfs::write(
            root.join("owned-installer-marker"),
            b"owned-disposable-installer-v1",
        )
        .unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        stdfs::create_dir_all(&home).unwrap();
        stdfs::create_dir_all(&repo).unwrap();
        let o = Options {
            apps: home.join("Applications"),
            signer: home.join(".local/share/ruycast/signing"),
            repo,
            home,
            profile: None,
            config: None,
            adapter: None,
        };
        let f = Self { root, o };
        f.build();
        f
    }
    fn build(&self) {
        for p in [
            "rust/Cargo.toml",
            "rust/Cargo.lock",
            "rust/src/lib.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust-native/src/main.rs",
            "config/default.json",
        ] {
            let path = self.o.repo.join(p);
            stdfs::create_dir_all(path.parent().unwrap()).unwrap();
            stdfs::write(path, b"owned source fixture").unwrap();
        }
        let target = "aarch64-apple-darwin";
        let mut artifacts = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        for (part, name) in [("native", "ruycast-native"), ("core", "ruycast")] {
            let p = format!(".build/full-rust-release/{part}/{target}/release/{name}");
            let path = self.o.repo.join(&p);
            stdfs::create_dir_all(path.parent().unwrap()).unwrap();
            stdfs::write(&path, format!("owned {name} executable fixture")).unwrap();
            artifacts.insert(p.clone(), fs::hash(&path).unwrap());
            libraries.insert(p,vec!["/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)"]);
        }
        let prep_path = ".build/full-rust-release/release-preparation.json";
        stdfs::write(
            self.o.repo.join(prep_path),
            b"owned release-preparation authority fixture",
        )
        .unwrap();
        let receipt = json!({"schema":receipt::SCHEMA,"configuration":"release","releasePreparation":{"path":prep_path,"sha256":fs::hash(&self.o.repo.join(prep_path)).unwrap()},"productVersion":VERSION,"target":target,"sources":receipt::inventory(&self.o.repo).unwrap(),"artifacts":artifacts,"linkedLibraries":libraries,"toolchains":{"cargo":"owned compiler","rustc":"owned compiler"},"commands":[["cargo","--target",target],["cargo","--target",target]],"environmentOverrides":{},"productionAccepted":false,"installedOrPublished":false});
        stdfs::write(
            self.o
                .repo
                .join(".build/full-rust-release/development-build.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    }
    fn bundle(&self, id: &str, name: &str, build: &str) {
        let app = self.o.apps.join(name);
        stdfs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        stdfs::create_dir_all(app.join("Contents/Resources")).unwrap();
        stdfs::write(
            app.join("Contents/Info.plist"),
            tools::plist_bytes(&json!({"CFBundleIdentifier":id,"CFBundleVersion":build})).unwrap(),
        )
        .unwrap();
        stdfs::write(
            app.join("Contents/MacOS").join(if id == OLD_ID {
                "RustcastApp"
            } else {
                "RuycastApp"
            }),
            b"old host",
        )
        .unwrap();
        stdfs::write(
            app.join("Contents/Resources")
                .join(if id == OLD_ID { "rustcast" } else { "ruycast" }),
            b"old cli",
        )
        .unwrap();
    }
    fn legacy(&self) {
        self.bundle(OLD_ID, "Rustcast.app", "6");
        let profile = self.o.home.join(".config/rustcast");
        stdfs::create_dir_all(&profile).unwrap();
        for name in [
            "config.json",
            "clipboard.json",
            "usage.json",
            "paused.json",
            "utilities.json",
            "search-ranking.json",
        ] {
            stdfs::write(
                profile.join(name),
                if name == "config.json" {
                    serde_json::to_vec(&rustcast_core::default_config()).unwrap()
                } else {
                    serde_json::to_vec(&json!({"owned":"private Ω","name":name})).unwrap()
                },
            )
            .unwrap();
        }
        let agents = self.o.home.join("Library/LaunchAgents");
        stdfs::create_dir_all(&agents).unwrap();
        stdfs::write(agents.join(format!("{OLD_ID}.plist")),tools::plist_bytes(&json!({"Label":OLD_ID,"ProgramArguments":[self.o.apps.join("Rustcast.app/Contents/MacOS/RustcastApp")]})).unwrap()).unwrap();
        let link = self.o.home.join(".local/bin/rustcast");
        stdfs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(
            self.o.apps.join("Rustcast.app/Contents/Resources/rustcast"),
            link,
        )
        .unwrap();
    }
    fn signer(&self) {
        fs::directory(&self.o.signer, 0o700).unwrap();
        for p in [
            "certificate.pem",
            "identity.keychain-db",
            "keychain-password",
        ] {
            fs::create(
                &self.o.signer.join(p),
                if p == "keychain-password" {
                    b"owned-fixture-password"
                } else {
                    b"owned state"
                },
                0o600,
            )
            .unwrap();
        }
        let value = json!({"schema":"ruycast.local-signing.v1","name":"Ruycast Local Signing","certificate_sha1":"A".repeat(40),"certificate_sha256":fs::hash(&self.o.signer.join("certificate.pem")).unwrap(),"requirement":signing::requirement(&"A".repeat(40))});
        fs::create(
            &self.o.signer.join("identity.json"),
            &serde_json::to_vec(&value).unwrap(),
            0o600,
        )
        .unwrap();
    }
    fn runner(&self) -> Fake {
        Fake {
            root: self.root.clone(),
            expected_host: self.o.apps.join("Ruycast.app/Contents/MacOS/RuycastApp"),
            calls: vec![],
            fault: None,
            failed: false,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = stdfs::remove_dir_all(&self.root);
    }
}
struct Fake {
    root: PathBuf,
    expected_host: PathBuf,
    calls: Vec<(String, Vec<String>, bool)>,
    fault: Option<&'static str>,
    failed: bool,
}
fn owned_install(o: &Options, r: &mut Fake) -> Result<Value> {
    assert!(o.repo.starts_with(&r.root));
    assert_eq!(
        stdfs::read(r.root.join("owned-installer-marker")).unwrap(),
        b"owned-disposable-installer-v1"
    );
    let lease = Lease::acquire_from_environment(&o.repo, &BTreeMap::new())?;
    install_pipeline(o, false, &lease, r)
}
impl Fake {
    fn revoke(&self) {
        let lock = self.root.join("repo/.build/artifacts.lock");
        stdfs::rename(&lock, self.root.join("revoked-lease")).unwrap();
        fs::create(&lock, b"", 0o600).unwrap();
    }
    fn stopped(&self) -> bool {
        self.calls
            .iter()
            .any(|(p, a, _)| p.ends_with("launchctl") || a == &["quit"])
    }
    fn hit(&mut self, name: &str) -> bool {
        if self.fault == Some(name) && !self.failed {
            self.failed = true;
            true
        } else {
            false
        }
    }
}
impl tools::Runner for Fake {
    fn running_host(&mut self, pid: i32, expected: &Path) -> Result<()> {
        assert_eq!(pid, 730001);
        assert_eq!(expected, self.expected_host);
        if self.fault == Some("wrong-process") {
            Err("Owned mismatching host process".into())
        } else {
            Ok(())
        }
    }
    fn secret(&mut self) -> Result<String> {
        Ok("b".repeat(64))
    }
    fn run(&mut self, c: tools::Call) -> Result<tools::Output> {
        let name = c.program.file_name().unwrap().to_str().unwrap();
        self.calls.push((
            c.program.display().to_string(),
            c.args.clone(),
            !c.input.is_empty(),
        ));
        let output = |text: Vec<u8>| {
            Ok(tools::Output {
                success: true,
                stdout: text,
                stderr: vec![],
            })
        };
        if name == "plutil" {
            let path = PathBuf::from(c.args.last().unwrap());
            assert!(path.starts_with(&self.root));
            fs::no_links(&path)?;
            return tools::System.run(c);
        } // Standard parser on owned fixture bytes only.
        if name == "cargo" {
            let repo = self.root.join("repo");
            assert_eq!(c.cwd.as_deref(), Some(repo.as_path()));
            let lease = Lease::acquire_from_environment(&repo, &c.env)?;
            lease.validate()?;
            assert!(c.args.windows(2).any(|a| a == ["--bin", "ruycast-build"]));
            assert!(
                !c.args
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--target" | "host-tuple")),
                "native Cargo bootstrap must use its default host target"
            );
            assert!(c.args.iter().any(|a| a == "--offline"));
            assert_eq!(c.args.last().unwrap(), &repo.display().to_string());
            let operation = c
                .args
                .windows(2)
                .find_map(|a| (a[0] == "--").then_some(a[1].as_str()))
                .unwrap();
            assert!(matches!(operation, "build" | "verify"));
            if operation == "build" && self.hit("build") {
                return Err("Owned Rust build failure before installation".into());
            }
            let path = repo.join(".build/full-rust-release/development-build.json");
            let value: Value = serde_json::from_slice(&fs::bytes(&path)?).map_err(err)?;
            if value["schema"] != receipt::SCHEMA
                || value["configuration"] != "release"
                || value["sources"] != json!(receipt::inventory(&repo)?)
            {
                return Err(
                    "Owned build authority rejected stale source/schema/configuration.".into(),
                );
            }
            for (p, expected) in value["artifacts"].as_object().unwrap() {
                if fs::hash(&repo.join(p))? != expected.as_str().unwrap() {
                    return Err("Owned build authority rejected changed binary.".into());
                }
            }
            let prep = value["releasePreparation"]["path"]
                .as_str()
                .ok_or("Owned build authority requires release preparation.")?;
            if value["releasePreparation"]["sha256"] != fs::hash(&repo.join(prep))? {
                return Err("Owned build authority rejected changed release preparation.".into());
            }
            return output(serde_json::to_vec(&json!({"ok":true,"schema":value["schema"],"configuration":"release","target":value["target"],"artifacts":value["artifacts"],"receipt":path,"releasePreparation":value["releasePreparation"]})).unwrap());
        }
        if name == "rustc" {
            return output(b"host: aarch64-apple-darwin\n".to_vec());
        }
        if name == "otool" {
            return output(b"owned:\n /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)\n".to_vec());
        }
        if name == "security" {
            if c.args == ["-i"] {
                let input = String::from_utf8(c.input).unwrap();
                assert!(!c.args.iter().any(|a| a.contains("password")));
                if input.contains("create-keychain") {
                    let keychain = self
                        .root
                        .join("home/.local/share/ruycast/signing/identity.keychain-db");
                    fs::create(&keychain, b"owned keychain", 0o600)?;
                }
                return output(vec![]);
            }
            if c.args == ["list-keychains", "-d", "user"] {
                return output(b"\"/owned/existing login.keychain-db\"\n".to_vec());
            }
            return output(vec![]);
        }
        if name == "openssl" {
            if c.args[0] == "req" {
                let i = c.args.iter().position(|a| a == "-out").unwrap();
                let cert = PathBuf::from(&c.args[i + 1]);
                assert!(cert.starts_with(&self.root));
                stdfs::write(cert, b"owned fixture certificate").unwrap();
                let i = c.args.iter().position(|a| a == "-keyout").unwrap();
                stdfs::write(&c.args[i + 1], b"owned private fixture").unwrap();
            }
            if c.args[0] == "pkcs12" {
                let i = c.args.iter().position(|a| a == "-out").unwrap();
                stdfs::write(&c.args[i + 1], b"owned p12 fixture").unwrap();
            }
            return output(if c.args[0] == "x509" {
                format!("sha1 Fingerprint={}\n", "AA:".repeat(19) + "AA").into_bytes()
            } else {
                vec![]
            });
        }
        if name == "codesign" {
            if c.args.contains(&"--force".into()) && self.hit("lease-stage") {
                self.revoke();
            }
            if c.args.contains(&"--force".into()) && self.hit("source") {
                stdfs::write(self.root.join("repo/rust/src/lib.rs"), b"changed source").unwrap();
            }
            if self.hit("sign") && c.args.contains(&"--force".into()) {
                return Err("Owned signer unavailable".into());
            }
            if c.args.contains(&"--display".into()) {
                return output(format!("Identifier={NEW_ID}\nSignature=adhoc\n").into_bytes());
            }
            return output(vec![]);
        }
        if name == "launchctl" {
            if c.args[0] == "bootout" && self.hit("concurrent-app") {
                stdfs::write(
                    self.root
                        .join("home/Applications/Ruycast.app/Contents/MacOS/RuycastApp"),
                    b"external app change",
                )
                .unwrap();
            }
            if c.args[0] == "bootstrap"
                && c.args.last().unwrap().ends_with(&format!("{NEW_ID}.plist"))
                && self.hit("concurrent-link")
            {
                let path = self.root.join("home/.local/bin/ruycast");
                stdfs::remove_file(&path).unwrap();
                stdfs::write(path, b"external CLI replacement").unwrap();
            }
            if c.args[0] == "bootstrap"
                && c.args.last().unwrap().ends_with(&format!("{NEW_ID}.plist"))
                && self.hit("bootstrap")
            {
                return Err("Owned bootstrap failure".into());
            }
            return output(vec![]);
        }
        if name == "lsregister" {
            return output(vec![]);
        }
        if !c.program.starts_with(&self.root) {
            return Err("Owned fixture refuses unregistered tool; no real fallback.".into());
        }
        if c.args == ["--version"] {
            return output(serde_json::to_vec(&json!({"ok":true,"result":{"implementation":"rust","version":VERSION,"component":if name=="ruycast-native"{"host"}else{"cli"}}})).unwrap());
        }
        if c.args == ["--init-config"] {
            let profile = PathBuf::from(c.env.get("RUYCAST_HOME").unwrap());
            let path = profile::config(
                &profile,
                c.env.get("RUYCAST_CONFIG_DIR").map(String::as_str),
                Path::new(c.env.get("HOME").unwrap()),
            )?;
            if self.hit("init-missing") {
                return output(vec![]);
            }
            if self.hit("init-invalid") {
                fs::create(&path, b"invalid owned config", 0o600)?;
                return output(vec![]);
            }
            if !path.exists() {
                fs::create(
                    &path,
                    &serde_json::to_vec(&rustcast_core::default_config()).unwrap(),
                    0o600,
                )?;
            }
            if self.hit("source") {
                stdfs::write(self.root.join("repo/rust/src/lib.rs"), b"changed source").unwrap();
            }
            return output(vec![]);
        }
        if c.args == ["quit"] {
            return output(vec![]);
        }
        if c.args == ["status"] {
            if self.hit("lease-final") {
                self.revoke();
            }
            let profile = PathBuf::from(c.env.get("RUYCAST_HOME").unwrap());
            let config = profile::config(
                &profile,
                c.env.get("RUYCAST_CONFIG_DIR").map(String::as_str),
                Path::new(c.env.get("HOME").unwrap()),
            )?;
            return output(serde_json::to_vec(&json!({"ok":true,"result":{"name":"Ruycast","pid":730001,"version":if matches!(self.fault,Some("version"|"concurrent-link")){"0.0.0"}else{VERSION},"configPath":config}})).unwrap());
        }
        Err("Owned fixture refuses unknown operation before any effect.".into())
    }
}

#[test]
fn installer_owned_first_install_provisions_and_pins_without_secret_argv() {
    let f = Fixture::new();
    let mut r = f.runner();
    let installed = owned_install(&f.o, &mut r).unwrap();
    assert_eq!(installed["status"]["result"]["version"], VERSION);
    assert!(f.o.signer.join("installed-identity.json").exists());
    let calls = r.calls.len();
    signing::prepare(&f.o.signer, &f.o.repo, &f.o.home, &mut r).unwrap();
    assert_eq!(calls, r.calls.len());
    assert!(r
        .calls
        .iter()
        .all(|(_, a, _)| !a.iter().any(|a| a.contains(&"b".repeat(64)))));
    assert!(r.calls.iter().any(|(p, a, _)| p.ends_with("codesign")
        && a.contains(&format!(
            "=designated => {}",
            signing::requirement(&"A".repeat(40))
        ))));
}

#[test]
fn installer_owned_current_app_config_selector_adapter_and_rollback_are_exact() {
    for rollback in [false, true] {
        let f = Fixture::new();
        f.bundle(NEW_ID, "Ruycast.app", "17");
        f.signer();
        let profile = f.o.home.join(".config/ruycast");
        stdfs::create_dir_all(&profile).unwrap();
        let selected = f.o.home.join("synced Ω config");
        stdfs::create_dir_all(&selected).unwrap();
        stdfs::write(
            selected.join("config.json"),
            serde_json::to_vec(&rustcast_core::default_config()).unwrap(),
        )
        .unwrap();
        stdfs::write(
            profile.join("config-location.json"),
            serde_json::to_vec(&json!({"directory":selected})).unwrap(),
        )
        .unwrap();
        let adapter =
            f.o.apps
                .join("Ruycast.app/Contents/Resources/personal_bridge.py");
        stdfs::write(&adapter, b"owned adapter fixture, never invoked").unwrap();
        stdfs::set_permissions(&adapter, stdfs::Permissions::from_mode(0o755)).unwrap();
        let before = fs::tree(&f.o.apps.join("Ruycast.app")).unwrap();
        let hashes = profile::hashes(&profile, None, &f.o.home).unwrap();
        let agents = f.o.home.join("Library/LaunchAgents");
        stdfs::create_dir_all(&agents).unwrap();
        let registration=tools::plist_bytes(&json!({"Label":NEW_ID,"ProgramArguments":[f.o.apps.join("Ruycast.app/Contents/MacOS/RuycastApp")]})).unwrap();
        stdfs::write(agents.join(format!("{NEW_ID}.plist")), &registration).unwrap();
        let mut runner = f.runner();
        if rollback {
            runner.fault = Some("version");
        }
        let result = owned_install(&f.o, &mut runner);
        assert_eq!(result.is_err(), rollback);
        assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), hashes);
        assert_eq!(
            stdfs::read(&adapter).unwrap(),
            b"owned adapter fixture, never invoked"
        );
        assert_eq!(
            stdfs::metadata(&adapter).unwrap().permissions().mode() & 0o777,
            0o755
        );
        if rollback {
            assert_eq!(fs::tree(&f.o.apps.join("Ruycast.app")).unwrap(), before);
            assert_eq!(
                stdfs::read(agents.join(format!("{NEW_ID}.plist"))).unwrap(),
                registration
            );
        } else {
            let info = tools::plist(
                &mut runner,
                &f.o.apps.join("Ruycast.app/Contents/Info.plist"),
            )
            .unwrap();
            assert_eq!(info["CFBundleVersion"], "18");
        }
    }
}

#[test]
fn installer_owned_unrelated_product_and_unknown_stage_are_preserved() {
    let f = Fixture::new();
    f.legacy();
    f.signer();
    let old = f.o.apps.join("Rustcast.app");
    stdfs::write(
        old.join("Contents/Info.plist"),
        tools::plist_bytes(&json!({"CFBundleIdentifier":"org.unrelated.product"})).unwrap(),
    )
    .unwrap();
    let old_tree = fs::tree(&old).unwrap();
    let profile = f.o.home.join(".config/rustcast");
    let before = profile::hashes(&profile, None, &f.o.home).unwrap();
    let unknown = f.o.apps.join(".Ruycast.new.app");
    stdfs::create_dir(&unknown).unwrap();
    stdfs::write(unknown.join("user-file"), b"keep").unwrap();
    let mut r = f.runner();
    owned_install(&f.o, &mut r).unwrap();
    assert_eq!(fs::tree(&old).unwrap(), old_tree);
    assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), before);
    assert!(!profile.is_symlink());
    assert!(stdfs::read_link(f.o.home.join(".local/bin/rustcast"))
        .unwrap()
        .starts_with(&old));
    assert!(r.calls.iter().all(|(_, a, _)| !a
        .iter()
        .any(|v| v.ends_with(OLD_ID) || v.ends_with(&format!("{OLD_ID}.plist")))));
    assert_eq!(stdfs::read(unknown.join("user-file")).unwrap(), b"keep");
}

#[test]
fn installer_owned_tampered_receipts_links_bundles_and_config_fail_before_stop() {
    for fault in [
        "binary",
        "source",
        "missing-receipt",
        "unrelated-link",
        "linked-app",
        "linked-selector",
        "profile-conflict",
    ] {
        let f = Fixture::new();
        f.legacy();
        f.signer();
        let old = f.o.home.join(".config/rustcast");
        let before = profile::hashes(&old, None, &f.o.home).unwrap();
        match fault {
            "binary" => stdfs::write(
                f.o.repo.join(
                    ".build/full-rust-release/native/aarch64-apple-darwin/release/ruycast-native",
                ),
                b"tampered",
            )
            .unwrap(),
            "source" => stdfs::write(f.o.repo.join("rust/src/lib.rs"), b"tampered").unwrap(),
            "missing-receipt" => stdfs::remove_file(
                f.o.repo
                    .join(".build/full-rust-release/development-build.json"),
            )
            .unwrap(),
            "unrelated-link" => {
                stdfs::write(f.root.join("unrelated-tool"), b"unrelated").unwrap();
                symlink(
                    f.root.join("unrelated-tool"),
                    f.o.home.join(".local/bin/ruycast"),
                )
                .unwrap();
            }
            "linked-app" => {
                symlink(f.o.apps.join("Rustcast.app"), f.o.apps.join("Ruycast.app")).unwrap()
            }
            "linked-selector" => {
                stdfs::write(f.root.join("selector"), b"{}").unwrap();
                symlink(f.root.join("selector"), old.join("config-location.json")).unwrap();
            }
            "profile-conflict" => {
                stdfs::create_dir(f.o.home.join(".config/ruycast")).unwrap();
            }
            _ => unreachable!(),
        }
        let mut r = f.runner();
        assert!(owned_install(&f.o, &mut r).is_err(), "{fault}");
        assert!(!r.stopped(), "{fault}");
        if fault != "linked-selector" {
            assert_eq!(profile::hashes(&old, None, &f.o.home).unwrap(), before);
        }
        assert!(f.o.apps.join("Rustcast.app").is_dir());
    }
}

#[test]
fn installer_owned_signer_never_rotates_weakens_or_downgrades() {
    for fault in [
        "missing",
        "certificate",
        "mode",
        "requirement",
        "enrollment",
    ] {
        let f = Fixture::new();
        f.signer();
        let before = stdfs::read(f.o.signer.join("identity.json")).unwrap();
        match fault {
            "missing" => stdfs::remove_file(f.o.signer.join("identity.keychain-db")).unwrap(),
            "certificate" => stdfs::write(f.o.signer.join("certificate.pem"), b"changed").unwrap(),
            "mode" => stdfs::set_permissions(
                f.o.signer.join("keychain-password"),
                stdfs::Permissions::from_mode(0o644),
            )
            .unwrap(),
            "requirement" => {
                let mut value: Value = serde_json::from_slice(&before).unwrap();
                value["requirement"] = json!("identifier only");
                stdfs::write(
                    f.o.signer.join("identity.json"),
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
            }
            "enrollment" => {
                f.bundle(NEW_ID, "Ruycast.app", "17");
                fs::create(&f.o.signer.join("installed-identity.json"), b"{}", 0o600).unwrap();
            }
            _ => unreachable!(),
        };
        let mut r = f.runner();
        let failed = if fault == "enrollment" {
            owned_install(&f.o, &mut r).is_err()
        } else {
            signing::prepare(&f.o.signer, &f.o.repo, &f.o.home, &mut r).is_err()
        };
        assert!(failed, "{fault}");
        assert!(!r.stopped());
        assert!(!r.calls.iter().any(|(_, a, _)| a == &["-i"]));
        if fault != "requirement" {
            assert_eq!(
                stdfs::read(f.o.signer.join("identity.json")).unwrap(),
                before
            );
        }
    }
}

#[test]
fn installer_owned_concurrent_app_or_cli_changes_are_never_clobbered() {
    for fault in ["concurrent-app", "concurrent-link"] {
        let f = Fixture::new();
        f.bundle(NEW_ID, "Ruycast.app", "17");
        f.signer();
        let mut r = f.runner();
        r.fault = Some(fault);
        let error = owned_install(&f.o, &mut r).unwrap_err();
        if fault == "concurrent-app" {
            assert!(error.contains("changed"), "{error}");
            assert_eq!(
                stdfs::read(f.o.apps.join("Ruycast.app/Contents/MacOS/RuycastApp")).unwrap(),
                b"external app change"
            );
        } else {
            assert!(error.contains("Rollback requires repair"), "{error}");
            assert_eq!(
                stdfs::read(f.o.home.join(".local/bin/ruycast")).unwrap(),
                b"external CLI replacement"
            );
        }
    }
}

#[test]
fn installer_owned_download_checksum_and_symlink_guard() {
    let f = Fixture::new();
    let archive = f.root.join("source.tar.gz");
    stdfs::write(&archive, b"owned archive bytes").unwrap();
    let expected = fs::hash(&archive).unwrap();
    assert_eq!(
        verify_download(&archive, &expected).unwrap()["sha256"],
        expected
    );
    assert!(verify_download(&archive, &"0".repeat(64)).is_err());
    let link = f.root.join("linked.tar.gz");
    symlink(&archive, &link).unwrap();
    assert!(verify_download(&link, &expected).is_err());
    assert!(verify_download(&archive, "not a hash").is_err());
}

#[test]
fn installer_owned_borrowed_lease_root_and_stage_final_namespace_guards() {
    let first = Fixture::new();
    let second = Fixture::new();
    let lease = Lease::acquire_from_environment(&second.o.repo, &BTreeMap::new()).unwrap();
    let mut r = first.runner();
    assert!(transaction::install(&first.o, &lease, &mut r)
        .unwrap_err()
        .contains("different source"));
    assert!(r.calls.is_empty());
    drop(lease);
    for fault in ["lease-stage", "lease-final"] {
        let f = Fixture::new();
        f.legacy();
        f.signer();
        let old = f.o.home.join(".config/rustcast");
        let before = profile::hashes(&old, None, &f.o.home).unwrap();
        let mut r = f.runner();
        r.fault = Some(fault);
        assert!(owned_install(&f.o, &mut r).unwrap_err().contains("lease"));
        assert!(f.o.apps.join("Rustcast.app").exists());
        assert!(!f.o.apps.join("Ruycast.app").exists());
        let profile = if old.is_symlink() {
            f.o.home.join(".config/ruycast")
        } else {
            old
        };
        assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), before);
        if fault == "lease-stage" {
            assert!(!r.stopped());
        }
    }
}

#[test]
fn installer_owned_explicit_app_profile_config_and_relative_legacy_alias() {
    let mut f = Fixture::new();
    f.signer();
    f.o.apps = f.root.join("separate app directory");
    f.o.profile = Some(f.root.join("separate runtime"));
    let selected = f.root.join("synced config");
    stdfs::create_dir(&selected).unwrap();
    stdfs::write(
        selected.join("config.json"),
        serde_json::to_vec(&rustcast_core::default_config()).unwrap(),
    )
    .unwrap();
    f.o.config = Some(selected.display().to_string());
    let mut r = f.runner();
    owned_install(&f.o, &mut r).unwrap();
    let registration = tools::plist(
        &mut r,
        &f.o.home
            .join(format!("Library/LaunchAgents/{NEW_ID}.plist")),
    )
    .unwrap();
    assert_eq!(
        registration["EnvironmentVariables"]["RUYCAST_CONFIG_DIR"],
        json!(selected)
    );
    assert_eq!(
        registration["EnvironmentVariables"]["RUYCAST_HOME"],
        json!(f.o.profile)
    );
    let old = f.root.join("legacy-profile");
    let new = f.root.join("new-profile");
    stdfs::create_dir(&new).unwrap();
    symlink("new-profile", &old).unwrap();
    assert_eq!(profile::active(&old, &new).unwrap(), new);
}

#[test]
fn installer_owned_profile_lock_is_exclusive_and_released_after_owner_drop() {
    let f = Fixture::new();
    let path = f.root.join("instance.lock");
    let lock = fs::Lock::acquire(&path, 0).unwrap();
    assert!(fs::Lock::acquire(&path, 0).is_err());
    drop(lock);
    assert!(fs::Lock::acquire(&path, 0).is_ok());
    assert_eq!(
        stdfs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn installer_owned_build_then_verify_shares_one_real_lease_and_build_failure_never_stops() {
    for fail in [false, true] {
        let f = Fixture::new();
        f.legacy();
        f.signer();
        let lease = Lease::acquire_from_environment(&f.o.repo, &BTreeMap::new()).unwrap();
        let mut r = f.runner();
        if fail {
            r.fault = Some("build");
        }
        let result = install_pipeline(&f.o, true, &lease, &mut r);
        assert_eq!(result.is_err(), fail);
        lease.validate().unwrap();
        let operations: Vec<_> = r
            .calls
            .iter()
            .filter(|(p, _, _)| p.ends_with("cargo"))
            .map(|(_, a, _)| {
                a.windows(2)
                    .find_map(|v| (v[0] == "--").then_some(v[1].as_str()))
                    .unwrap()
            })
            .collect();
        assert_eq!(operations.first().copied(), Some("build"));
        if fail {
            assert!(!r.stopped());
            assert_eq!(operations, vec!["build"]);
        } else {
            assert_eq!(operations, vec!["build", "verify", "verify"]);
        }
    }
}

#[test]
fn installer_owned_v3_debug_missing_changed_or_unknown_release_receipts_fail_closed() {
    for fault in [
        "v3",
        "debug",
        "missing-preparation",
        "changed-preparation",
        "unknown-field",
    ] {
        let f = Fixture::new();
        f.legacy();
        f.signer();
        let path =
            f.o.repo
                .join(".build/full-rust-release/development-build.json");
        let mut value: Value = serde_json::from_slice(&stdfs::read(&path).unwrap()).unwrap();
        match fault {
            "v3" => value["schema"] = json!("ruycast.rust-development-build.v3"),
            "debug" => {
                value["configuration"] = json!("debug");
                value["releasePreparation"] = Value::Null;
            }
            "missing-preparation" => {
                value.as_object_mut().unwrap().remove("releasePreparation");
            }
            "changed-preparation" => stdfs::write(
                f.o.repo
                    .join(".build/full-rust-release/release-preparation.json"),
                b"changed diagnostic preparation",
            )
            .unwrap(),
            "unknown-field" => value["unreviewedAuthority"] = json!(true),
            _ => unreachable!(),
        };
        stdfs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut r = f.runner();
        assert!(owned_install(&f.o, &mut r).is_err(), "{fault}");
        assert!(!r.stopped(), "{fault}");
        assert!(f.o.apps.join("Rustcast.app").exists());
        assert!(!f.o.apps.join("Ruycast.app").exists());
    }
}

#[test]
fn installer_owned_real_process_stdin_output_bounds_timeout_and_reaping() {
    let f = Fixture::new();
    let source = f.root.join("tool.rs");
    let tool = f.root.join("owned-tool");
    stdfs::write(&source, include_str!("fixture_process.rs")).unwrap();
    let built = std::process::Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&tool)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    stdfs::write(f.root.join("owned-process-marker"), b"owned").unwrap();
    for mode in ["stdin", "flood", "hang"] {
        let marker = f.root.join(format!("pid-{mode}"));
        let mut call = tools::Call::new(&tool, [mode, &marker.display().to_string()]).timeout(
            if mode == "hang" {
                std::time::Duration::from_millis(250)
            } else {
                std::time::Duration::from_secs(3)
            },
        );
        call.input = b"literal fixture stdin, never argv".to_vec();
        let started = std::time::Instant::now();
        let result = tools::System.run(call);
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        if mode == "stdin" {
            assert_eq!(result.unwrap().stdout, b"literal fixture stdin, never argv");
        } else {
            let error = result.err().expect("Owned bounded process must fail");
            assert!(
                error.contains(if mode == "hang" {
                    "timed out"
                } else {
                    "exceeded"
                }),
                "{error}"
            );
        }
        let pid: libc::pid_t = stdfs::read_to_string(marker).unwrap().parse().unwrap();
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

#[test]
fn installer_owned_compat_dispatchers_forward_only_and_old_mock_repo_fails_closed() {
    let f = Fixture::new();
    let source = f.root.join("dispatch.rs");
    let tool = f.root.join("dispatch-tool");
    stdfs::write(&source, include_str!("fixture_process.rs")).unwrap();
    let output = std::process::Command::new("rustc")
        .args(["--edition=2021"])
        .arg(&source)
        .arg("-o")
        .arg(&tool)
        .output()
        .unwrap();
    assert!(output.status.success());
    stdfs::write(f.root.join("owned-process-marker"), b"owned").unwrap();
    let source_repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let repo = f.o.repo.clone();
    stdfs::create_dir(repo.join("scripts")).unwrap();
    for name in ["install.sh", "install.py", "local_signing.py"] {
        stdfs::copy(
            source_repo.join("scripts").join(name),
            repo.join("scripts").join(name),
        )
        .unwrap();
    }
    let lease = Lease::acquire_from_environment(&repo, &BTreeMap::new()).unwrap();
    for shell in [false, true] {
        let program = if shell {
            "/bin/bash"
        } else {
            "/usr/bin/python3"
        };
        let script = repo.join(if shell {
            "scripts/install.sh"
        } else {
            "scripts/install.py"
        });
        let mut c = std::process::Command::new(program);
        let output = c
            .arg(script)
            .args([
                "--archive",
                "literal supplied path",
                "--sha256",
                "literal trusted digest",
            ])
            .env_clear()
            .env("HOME", &f.o.home)
            .env("PATH", "/usr/bin:/bin")
            .env("RUYCAST_INSTALL_TOOL", &tool)
            .env("RUYCAST_DISPATCH_FIXTURE_ROOT", &f.root)
            .envs(lease.environment().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = vec![
            "install".to_owned(),
            "--repo".into(),
            repo.display().to_string(),
        ];
        if shell {
            expected.push("--build".into());
        }
        expected.extend(
            [
                "--archive",
                "literal supplied path",
                "--sha256",
                "literal trusted digest",
            ]
            .map(str::to_owned),
        );
        assert_eq!(output.stdout, expected.join("\0").as_bytes());
    }
    let output = std::process::Command::new("/usr/bin/python3")
        .arg(repo.join("scripts/local_signing.py"))
        .args(["--root", "literal signing root", "status"])
        .env_clear()
        .env("HOME", &f.o.home)
        .env("PATH", "/usr/bin:/bin")
        .env("RUYCAST_INSTALL_TOOL", &tool)
        .env("RUYCAST_DISPATCH_FIXTURE_ROOT", &f.root)
        .envs(lease.environment().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        [
            "signing",
            "--root",
            "literal signing root",
            "status",
            "--repo",
            &repo.display().to_string()
        ]
        .join("\0")
        .as_bytes()
    );
    let scripts = f.root.join("old-mock-repo/scripts");
    stdfs::create_dir_all(&scripts).unwrap();
    stdfs::copy(repo.join("scripts/install.py"), scripts.join("install.py")).unwrap();
    let result = std::process::Command::new("/usr/bin/python3")
        .arg(scripts.join("install.py"))
        .env_clear()
        .env("HOME", &f.o.home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Rust installer source is unavailable")
    );
    lease.validate().unwrap();
    assert!(!f.o.apps.exists());
}

#[test]
fn installer_owned_enrollment_blocks_adhoc_and_search_list_keeps_existing_entries() {
    let f = Fixture::new();
    f.signer();
    f.bundle(NEW_ID, "Ruycast.app", "17");
    let mut r = f.runner();
    let identity = signing::load(&f.o.signer, &f.o.repo).unwrap();
    let app = f.o.apps.join("Ruycast.app");
    signing::record(&app, &f.o.signer, &f.o.repo, &identity, &f.o.home, &mut r).unwrap();
    let marker = stdfs::read(f.o.signer.join("installed-identity.json")).unwrap();
    assert!(
        signing::check_installed(&app, &f.o.signer, &identity, &f.o.home, &mut r)
            .unwrap_err()
            .contains("downgrade")
    );
    signing::record(&app, &f.o.signer, &f.o.repo, &identity, &f.o.home, &mut r).unwrap();
    assert_eq!(
        stdfs::read(f.o.signer.join("installed-identity.json")).unwrap(),
        marker
    );
    signing::sign(&app, &f.o.signer, &f.o.repo, &f.o.home, &mut r).unwrap();
    assert!(r.calls.iter().any(|(p, a, _)| p.ends_with("security")
        && a == &vec![
            "list-keychains".to_owned(),
            "-d".into(),
            "user".into(),
            "-s".into(),
            "/owned/existing login.keychain-db".into(),
            f.o.signer
                .join("identity.keychain-db")
                .display()
                .to_string()
        ]));
    assert!(r
        .calls
        .iter()
        .all(|(_, a, _)| !a.iter().any(|v| v == "add-trusted-cert")
            && !a.windows(2).any(|v| v == ["--sign", "-"])));
}
#[test]
fn installer_owned_legacy_migration_preserves_authored_state_and_both_cli_names() {
    let f = Fixture::new();
    f.legacy();
    f.signer();
    let old = f.o.home.join(".config/rustcast");
    let before = profile::hashes(&old, None, &f.o.home).unwrap();
    let mut r = f.runner();
    let value = owned_install(&f.o, &mut r).unwrap();
    assert_eq!(value["preservedDataHashes"], json!(before));
    assert_eq!(
        profile::hashes(&f.o.home.join(".config/ruycast"), None, &f.o.home).unwrap(),
        before
    );
    assert!(stdfs::symlink_metadata(&old)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(!f.o.apps.join("Rustcast.app").exists());
    for name in ["ruycast", "rustcast"] {
        assert_eq!(
            stdfs::read_link(f.o.home.join(".local/bin").join(name)).unwrap(),
            f.o.apps.join("Ruycast.app/Contents/Resources/ruycast")
        );
    }
}
#[test]
fn installer_owned_failures_rollback_or_fail_before_stopping_without_config_changes() {
    for fault in ["sign", "source", "bootstrap", "version", "wrong-process"] {
        let f = Fixture::new();
        f.legacy();
        f.signer();
        let old = f.o.home.join(".config/rustcast");
        let before = profile::hashes(&old, None, &f.o.home).unwrap();
        let mut r = f.runner();
        r.fault = Some(fault);
        assert!(owned_install(&f.o, &mut r).is_err(), "{fault}");
        let profile = if old.is_dir() && !old.is_symlink() {
            old
        } else {
            f.o.home.join(".config/ruycast")
        };
        assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), before);
        assert!(f.o.apps.join("Rustcast.app").exists());
        assert!(!f.o.apps.join("Ruycast.app").exists());
        if ["sign", "source"].contains(&fault) {
            assert!(!r.stopped(), "{fault}");
        } else {
            assert!(r.calls.iter().any(|(p, a, _)| p.ends_with("launchctl")
                && a[0] == "bootstrap"
                && a.last().unwrap().ends_with(&format!("{OLD_ID}.plist"))));
        }
    }
}

#[test]
fn review_first_install_with_empty_custom_config_directory_reaches_initializer() {
    let mut f = Fixture::new();
    f.signer();
    let selected = f.root.join("new selected config");
    stdfs::create_dir(&selected).unwrap();
    f.o.config = Some(selected.display().to_string());
    let mut runner = f.runner();
    let result = owned_install(&f.o, &mut runner);
    assert!(result.is_ok(), "An empty explicit first-install configuration should be initialized: {result:?}; calls={:?}", runner.calls);
    assert!(selected.join("config.json").is_file());
    assert!(runner
        .calls
        .iter()
        .any(|(_, args, _)| args == &["--init-config"]));
    let hashes = profile::hashes(
        &f.o.home.join(".config/ruycast"),
        f.o.config.as_deref(),
        &f.o.home,
    )
    .unwrap();
    assert!(hashes.contains_key("selected-config.json"));
    assert_eq!(result.unwrap()["preservedDataHashes"], json!(hashes));
}

#[test]
fn review_timeout_reaps_descendant_after_its_direct_parent_exits() {
    let f = Fixture::new();
    stdfs::write(
        f.root.join("owned-process-marker"),
        b"review-owned-process-v1",
    )
    .unwrap();
    let source = f.root.join("descendant_fixture.rs");
    stdfs::write(&source, include_str!("descendant_fixture.rs")).unwrap();
    let fixture = f.root.join("descendant-fixture");
    let built = std::process::Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(std::process::Command::new(&fixture)
        .arg("--warm")
        .status()
        .unwrap()
        .success());
    let lease = Lease::acquire_from_environment(&f.o.repo, &BTreeMap::new()).unwrap();
    let mut system = tools::System;
    let mut call = tools::Call::new(fixture, ["parent", &f.root.display().to_string()])
        .timeout(std::time::Duration::from_millis(250));
    call.env.extend(lease.environment().unwrap());
    let result = system.run(call);
    assert!(result.is_err());
    assert!(
        f.root.join("started").is_file(),
        "Owned descendant must be live before timeout"
    );
    drop(lease);
    use std::os::fd::AsRawFd;
    let contender = stdfs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.o.repo.join(".build/artifacts.lock"))
        .unwrap();
    let release_deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(std::time::Instant::now() < release_deadline, "Timed-out descendants must release the inherited artifact lease within bounded teardown");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    std::thread::sleep(std::time::Duration::from_millis(900));
    let pid: i32 = stdfs::read_to_string(f.root.join("started"))
        .unwrap()
        .parse()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            std::time::Instant::now() < until,
            "Owned descendant has not naturally exited"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert!(
        !f.root.join("completed-after-parent").exists(),
        "Runner returned timeout but left a live descendant able to perform a late side effect"
    );
}

#[test]
fn installer_repair_custom_config_is_strict_and_initializer_must_produce_valid_bytes() {
    for fault in [
        "existing-invalid",
        "existing-linked",
        "existing-directory",
        "init-missing",
        "init-invalid",
    ] {
        let mut f = Fixture::new();
        f.signer();
        let selected = f.root.join("selected config");
        stdfs::create_dir(&selected).unwrap();
        f.o.config = Some(selected.display().to_string());
        let path = selected.join("config.json");
        match fault {
            "existing-invalid" => stdfs::write(&path, b"invalid existing config").unwrap(),
            "existing-linked" => {
                let owned = f.root.join("owned-config-target");
                stdfs::write(&owned, b"never replace").unwrap();
                symlink(owned, &path).unwrap();
            }
            "existing-directory" => stdfs::create_dir(&path).unwrap(),
            _ => (),
        }
        let before = stdfs::read(&path).ok();
        let mut runner = f.runner();
        if fault.starts_with("init-") {
            runner.fault = Some(fault);
        }
        assert!(owned_install(&f.o, &mut runner).is_err(), "{fault}");
        assert!(!runner.stopped(), "{fault}");
        if fault.starts_with("existing-") {
            assert_eq!(stdfs::read(&path).ok(), before, "{fault}");
            assert!(
                !runner
                    .calls
                    .iter()
                    .any(|(_, args, _)| args == &["--init-config"]),
                "{fault}"
            );
        }
    }
}

#[test]
fn installer_repair_successful_eof_retires_descendant_before_reaping_leader() {
    let f = Fixture::new();
    stdfs::write(
        f.root.join("owned-process-marker"),
        b"review-owned-process-v1",
    )
    .unwrap();
    let source = f.root.join("descendant.rs");
    stdfs::write(&source, include_str!("descendant_fixture.rs")).unwrap();
    let fixture = f.root.join("descendant");
    let compiled = std::process::Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(std::process::Command::new(&fixture)
        .arg("--warm")
        .status()
        .unwrap()
        .success());
    let lease = Lease::acquire_from_environment(&f.o.repo, &BTreeMap::new()).unwrap();
    let mut call = tools::Call::new(
        fixture,
        ["parent-null-pipes", &f.root.display().to_string()],
    )
    .timeout(std::time::Duration::from_secs(2));
    call.env.extend(lease.environment().unwrap());
    let began = std::time::Instant::now();
    let out = tools::System.run(call).unwrap();
    assert!(out.success);
    assert!(began.elapsed() < std::time::Duration::from_millis(650));
    drop(lease);
    use std::os::fd::AsRawFd;
    let contender = stdfs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.o.repo.join(".build/artifacts.lock"))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    std::thread::sleep(std::time::Duration::from_millis(900));
    assert!(!f.root.join("completed-after-parent").exists());
    let pid: i32 = stdfs::read_to_string(f.root.join("started"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
