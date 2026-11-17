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
    for existing_alias in [false,true] {
    let f = Fixture::new();
    f.legacy();
    f.signer();
    let old = f.o.apps.join("Rustcast.app");
    stdfs::write(
        old.join("Contents/Info.plist"),
        tools::plist_bytes(&json!({"CFBundleIdentifier":"org.unrelated.product"})).unwrap(),
    )
    .unwrap();
    if existing_alias {f.bundle(NEW_ID,"Ruycast.app","17");let link=f.o.home.join(".local/bin/rustcast");stdfs::remove_file(&link).unwrap();symlink(f.o.apps.join("Ruycast.app/Contents/Resources/ruycast"),&link).unwrap();}
    let original_cli=stdfs::read_link(f.o.home.join(".local/bin/rustcast")).unwrap();let old_agent=f.o.home.join(format!("Library/LaunchAgents/{OLD_ID}.plist"));let registration=stdfs::read(&old_agent).unwrap();
    let old_tree = fs::tree(&old).unwrap();
    let profile = f.o.home.join(".config/rustcast");
    let before = profile::hashes(&profile, None, &f.o.home).unwrap();
    let unknown = f.o.apps.join(".Ruycast.new.app");
    stdfs::create_dir(&unknown).unwrap();
    stdfs::write(unknown.join("user-file"), b"keep").unwrap();
    let repo_stage=f.o.repo.join(".dist/Ruycast.app");stdfs::create_dir_all(&repo_stage).unwrap();stdfs::write(repo_stage.join("user-file"),b"keep me").unwrap();
    let mut r = f.runner();
    owned_install(&f.o, &mut r).unwrap();
    assert_eq!(fs::tree(&old).unwrap(), old_tree);
    assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), before);
    assert!(!profile.is_symlink());
    let actual_cli=stdfs::read_link(f.o.home.join(".local/bin/rustcast")).unwrap();assert_eq!(actual_cli,original_cli);if !existing_alias {assert!(actual_cli.starts_with(&old));}
    assert!(old_agent.is_file());assert_eq!(stdfs::read(&old_agent).unwrap(),registration);
    assert_eq!(stdfs::read(repo_stage.join("user-file")).unwrap(),b"keep me");
    assert!(r.calls.iter().all(|(_, a, _)| !a
        .iter()
        .any(|v| v.ends_with(OLD_ID) || v.ends_with(&format!("{OLD_ID}.plist")))));
    assert_eq!(stdfs::read(unknown.join("user-file")).unwrap(), b"keep");
    }
}

#[test]
fn installer_owned_tampered_receipts_links_bundles_and_config_fail_before_stop() {
    for fault in [
        "binary",
        "source",
        "missing-receipt",
        "canonical-version",
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
            "canonical-version" => {
                let path = f.o.repo.join("rust/src/product_version.rs");
                stdfs::write(&path, include_bytes!("../../../../rust/src/product_version.rs")).unwrap();
                f.build();
                stdfs::write(path, b"invalid canonical source").unwrap();
            }
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
        let error = owned_install(&f.o, &mut r).unwrap_err();
        if ["binary", "source", "missing-receipt", "canonical-version"].contains(&fault) {
            assert_eq!(error, "Rust release verification failed; run make install to create a fresh prepared build.");
            assert_eq!(r.calls.len(), 1, "{fault}: {:?}", r.calls);
            let (program, args, _) = &r.calls[0];
            assert!(program.ends_with("cargo"));
            assert!(args.windows(2).any(|pair| pair == ["--", "verify"]));
            assert!(!f.o.apps.join("Ruycast.app").exists());
            assert!(!old.is_symlink());
        }
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
fn installer_owned_incomplete_signer_without_manifest_is_preserved() {
    let f = Fixture::new();
    f.signer();
    stdfs::remove_file(f.o.signer.join("identity.json")).unwrap();
    let remaining = [
        "certificate.pem",
        "identity.keychain-db",
        "keychain-password",
    ];
    let before: Vec<_> = remaining
        .iter()
        .map(|name| stdfs::read(f.o.signer.join(name)).unwrap())
        .collect();
    let mut r = f.runner();

    let error = signing::prepare(&f.o.signer, &f.o.repo, &f.o.home, &mut r).unwrap_err();

    assert!(error.contains("Incomplete signing state"), "{error}");
    assert!(r.calls.is_empty());
    assert!(!f.o.signer.join("identity.json").exists());
    assert_eq!(stdfs::read_dir(&f.o.signer).unwrap().count(), remaining.len());
    for (name, bytes) in remaining.iter().zip(before) {
        assert_eq!(stdfs::read(f.o.signer.join(name)).unwrap(), bytes);
    }
}

#[test]
fn installer_owned_signer_root_symlink_or_permissive_mode_is_refused() {
    for linked in [true, false] {
        let f = Fixture::new();
        f.signer();
        let before: BTreeMap<_, _> = stdfs::read_dir(&f.o.signer)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), stdfs::read(entry.path()).unwrap())
            })
            .collect();
        let root = if linked {
            let alias = f.o.home.join("signing-alias");
            symlink(&f.o.signer, &alias).unwrap();
            alias
        } else {
            stdfs::set_permissions(&f.o.signer, stdfs::Permissions::from_mode(0o755)).unwrap();
            f.o.signer.clone()
        };
        let mut r = f.runner();

        let error = signing::prepare(&root, &f.o.repo, &f.o.home, &mut r).unwrap_err();
        assert!(
            error.contains(if linked {
                "Linked installation path"
            } else {
                "private and owned"
            }),
            "{error}"
        );
        assert!(signing::load(&root, &f.o.repo).is_err());
        assert!(r.calls.is_empty());
        assert_eq!(stdfs::read_dir(&f.o.signer).unwrap().count(), before.len());
        for (name, bytes) in before {
            assert_eq!(stdfs::read(f.o.signer.join(name)).unwrap(), bytes);
        }
        if linked {
            assert_eq!(stdfs::read_link(&root).unwrap(), f.o.signer);
        } else {
            assert_eq!(
                stdfs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o755
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
    for (legacy, sparse) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut f = Fixture::new();
        f.signer();
        f.o.apps = f.root.join("separate app directory");
        if legacy {
            f.legacy();
        }
        if !legacy {
            f.o.profile = Some(f.root.join("separate runtime"));
        }
        let selected = f.root.join("synced config");
        stdfs::create_dir(&selected).unwrap();
        let authored = if sparse {
            b"{}".to_vec()
        } else {
            let mut bytes = serde_json::to_vec_pretty(&rustcast_core::default_config()).unwrap();
            bytes.extend_from_slice(b" \n");
            bytes
        };
        stdfs::write(selected.join("config.json"), &authored).unwrap();
        f.o.config = Some(selected.display().to_string());
        let unknown = f.o.repo.join(".dist/Ruycast.app");
        stdfs::create_dir_all(&unknown).unwrap();
        stdfs::write(unknown.join("user-file"), b"keep me").unwrap();
        let config_before = stdfs::read(selected.join("config.json")).unwrap();
        let old_profile = f.o.home.join(".config/rustcast");
        let legacy_hashes = legacy
            .then(|| profile::hashes(&old_profile, f.o.config.as_deref(), &f.o.home).unwrap());
        let mut r = f.runner();
        let result = owned_install(&f.o, &mut r);
        assert_eq!(
            stdfs::read(selected.join("config.json")).unwrap(),
            config_before
        );
        assert_eq!(stdfs::read(unknown.join("user-file")).unwrap(), b"keep me");
        if sparse {
            assert!(result.unwrap_err().contains("missing field `version`"));
            assert!(!r.stopped());
            assert!(!f.o.apps.join("Ruycast.app").exists());
            if let Some(before) = legacy_hashes {
                assert!(f.o.apps.join("Rustcast.app").is_dir());
                assert!(!old_profile.is_symlink());
                assert_eq!(
                    profile::hashes(&old_profile, f.o.config.as_deref(), &f.o.home).unwrap(),
                    before
                );
            }
            continue;
        }
        result.unwrap();
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
            json!(f
                .o
                .profile
                .clone()
                .unwrap_or_else(|| f.o.home.join(".config/ruycast")))
        );
        if let Some(before) = legacy_hashes {
            let migrated = f.o.home.join(".config/ruycast");
            assert_eq!(
                profile::hashes(&migrated, f.o.config.as_deref(), &f.o.home).unwrap(),
                before
            );
            assert!(old_profile.is_symlink());
            assert_eq!(stdfs::canonicalize(&old_profile).unwrap(), migrated);
        }
        let old = f.root.join("legacy-profile");
        let new = f.root.join("new-profile");
        stdfs::create_dir(&new).unwrap();
        symlink("new-profile", &old).unwrap();
        assert_eq!(profile::active(&old, &new).unwrap(), new);
    }
}

#[test]
fn installer_owned_profile_filesystem_preserves_bytes_and_idempotent_alias() {
    let f = Fixture::new();
    let (old, new) = profile::paths(&f.o, true);
    stdfs::create_dir_all(&old).unwrap();
    for name in ["config.json", "clipboard.json", "usage.json", "paused.json"] {
        stdfs::write(old.join(name), format!("private fixture Ω {name}")).unwrap();
    }
    let before = profile::hashes(&old, None, &f.o.home).unwrap();
    assert_eq!(profile::migrate(&old, &new).unwrap(), new);
    assert!(old.is_symlink());
    assert_eq!(stdfs::canonicalize(&old).unwrap(), new);
    assert_eq!(profile::hashes(&new, None, &f.o.home).unwrap(), before);
    assert_eq!(profile::migrate(&old, &new).unwrap(), new);
    assert!(old.is_symlink());
    assert_eq!(stdfs::canonicalize(&old).unwrap(), new);
    let active = profile::active(&old, &new).unwrap();
    assert_eq!(profile::hashes(&active, None, &f.o.home).unwrap(), before);
}

#[test]
fn installer_owned_profile_filesystem_selected_config_hash_is_separate_from_runtime() {
    let f = Fixture::new();
    let runtime = f.root.join("runtime");
    let custom = f.root.join("synced/config");
    stdfs::create_dir(&runtime).unwrap();
    stdfs::create_dir_all(&custom).unwrap();
    let selected = custom.join("config.json");
    stdfs::write(&selected, b"{\"private\":\"fixture\"}").unwrap();
    stdfs::write(
        runtime.join("config-location.json"),
        serde_json::to_vec(&json!({"directory":custom})).unwrap(),
    )
    .unwrap();
    stdfs::write(runtime.join("clipboard.json"), b"[]").unwrap();
    stdfs::write(
        runtime.join("search-ranking.json"),
        b"{\"schema\":1,\"entries\":{}}",
    )
    .unwrap();
    let files = profile::data(&runtime, None, &f.o.home).unwrap();
    assert_eq!(files["selected-config.json"], selected);
    assert_eq!(
        files["search-ranking.json"],
        runtime.join("search-ranking.json")
    );
    let before = profile::hashes(&runtime, None, &f.o.home).unwrap();
    stdfs::write(&selected, b"{\"private\":\"changed\"}").unwrap();
    let after = profile::hashes(&runtime, None, &f.o.home).unwrap();
    assert_ne!(
        after["selected-config.json"],
        before["selected-config.json"]
    );
    assert_eq!(after["clipboard.json"], before["clipboard.json"]);
}

#[test]
fn installer_owned_profile_filesystem_conflicts_and_unrelated_cli_are_preserved() {
    let f = Fixture::new();
    let (old, new) = profile::paths(&f.o, true);
    stdfs::create_dir_all(&old).unwrap();
    stdfs::create_dir(&new).unwrap();
    stdfs::write(old.join("config.json"), b"old").unwrap();
    stdfs::write(new.join("config.json"), b"new").unwrap();
    assert!(profile::migrate(&old, &new)
        .unwrap_err()
        .contains("Both profiles"));
    assert_eq!(stdfs::read(old.join("config.json")).unwrap(), b"old");
    assert_eq!(stdfs::read(new.join("config.json")).unwrap(), b"new");
    let link = f.root.join("ruycast");
    let target = f.root.join("Ruycast.app/Contents/Resources/ruycast");
    stdfs::write(&link, b"user executable").unwrap();
    assert!(profile::link(&link, std::slice::from_ref(&target))
        .unwrap_err()
        .contains("non-symlink"));
    assert!(profile::set_link(&link, &target, None).is_err());
    assert_eq!(stdfs::read(&link).unwrap(), b"user executable");
    stdfs::remove_file(&link).unwrap();
    let other = f.root.join("other-tool");
    stdfs::write(&other, b"user executable").unwrap();
    symlink(&other, &link).unwrap();
    assert!(profile::link(&link, std::slice::from_ref(&target))
        .unwrap_err()
        .contains("Unrelated CLI symlink"));
    assert!(profile::set_link(&link, &target, None).is_err());
    assert_eq!(stdfs::read_link(&link).unwrap(), other);
    assert_eq!(stdfs::canonicalize(&link).unwrap(), other);
    assert_eq!(stdfs::read(&other).unwrap(), b"user executable");
}

#[test]
fn installer_owned_personal_adapter_external_stages_without_public_source_copy() {
    let mut f = Fixture::new();
    let installed = f.root.join("installed");
    let stage = f.root.join("stage");
    stdfs::create_dir_all(stage.join("Contents/Resources")).unwrap();
    let adapter = f.root.join("local-adapter.py");
    stdfs::write(&adapter, b"fixture local adapter").unwrap();
    f.o.adapter = Some(adapter.clone());
    profile::adapter(&f.o, &installed, &stage).unwrap();
    let staged = stage.join("Contents/Resources/personal_bridge.py");
    assert_eq!(
        stdfs::read(&staged).unwrap(),
        stdfs::read(&adapter).unwrap()
    );
    assert!(stdfs::symlink_metadata(f.o.repo.join("scripts/personal_bridge.py")).is_err());
    f.o.adapter = Some(PathBuf::from("relative.py"));
    assert!(profile::adapter(&f.o, &installed, &stage)
        .unwrap_err()
        .contains("absolute UTF-8 path"));
    assert_eq!(stdfs::read(&staged).unwrap(), b"fixture local adapter");
}

#[test]
fn installer_owned_personal_adapter_installed_bytes_preserved_and_link_refused() {
    let f = Fixture::new();
    let installed = f.root.join("Ruycast.app");
    let stage = f.root.join("stage");
    let resource = installed.join("Contents/Resources");
    stdfs::create_dir_all(&resource).unwrap();
    stdfs::create_dir_all(stage.join("Contents/Resources")).unwrap();
    let adapter = resource.join("personal_bridge.py");
    stdfs::write(&adapter, b"private adapter fixture").unwrap();
    profile::adapter(&f.o, &installed, &stage).unwrap();
    let staged = stage.join("Contents/Resources/personal_bridge.py");
    assert_eq!(
        stdfs::read(&staged).unwrap(),
        stdfs::read(&adapter).unwrap()
    );
    stdfs::remove_file(&adapter).unwrap();
    let external = f.root.join("external");
    symlink(&external, &adapter).unwrap();
    assert_eq!(
        profile::adapter(&f.o, &installed, &stage).unwrap_err(),
        "Linked installation path preserved."
    );
    assert_eq!(stdfs::read_link(&adapter).unwrap(), external);
    assert_eq!(stdfs::read(&staged).unwrap(), b"private adapter fixture");
}

#[test]
fn installer_owned_bundle_identity_refuses_unrelated_app_without_mutation() {
    let f = Fixture::new();
    let app = f.root.join("Rustcast.app");
    stdfs::create_dir_all(app.join("Contents")).unwrap();
    let path = app.join("Contents/Info.plist");
    let before =
        tools::plist_bytes(&json!({"CFBundleIdentifier":"org.unrelated.rustcast"})).unwrap();
    stdfs::write(&path, &before).unwrap();
    let mut runner = f.runner();
    assert!(transaction::bundle(&app, OLD_ID, &mut runner)
        .unwrap_err()
        .contains("Unrelated app identity"));
    assert!(app.exists());
    assert_eq!(stdfs::read(path).unwrap(), before);
    assert!(!runner.stopped());
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
fn installer_owned_local_signing_python_dispatcher_forwards_only() {
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
    stdfs::copy(
        source_repo.join("scripts/local_signing.py"),
        repo.join("scripts/local_signing.py"),
    )
    .unwrap();
    let lease = Lease::acquire_from_environment(&repo, &BTreeMap::new()).unwrap();
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
    lease.validate().unwrap();
    assert!(!f.o.apps.exists());
}

#[test]
fn make_install_routes_directly_to_the_rust_installer() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
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
    let cargo_dir = f.root.join("cargo-bin");
    stdfs::create_dir(&cargo_dir).unwrap();
    std::os::unix::fs::symlink(&tool, cargo_dir.join("cargo")).unwrap();
    let path = format!("{}:/usr/bin:/bin", cargo_dir.display());
    let output = std::process::Command::new("make")
        .arg("install")
        .current_dir(&repo)
        .env_clear()
        .env("PATH", path)
        .env("RUYCAST_DISPATCH_FIXTURE_ROOT", &f.root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let command = String::from_utf8(output.stdout).unwrap();
    let mut lines = command.lines();
    let shell_command = lines.next().unwrap();
    for expected in [
        "cargo run --locked --offline --release",
        "--manifest-path rust-native/Cargo.toml",
        "--target-dir .build/installer-tools",
        "--bin ruycast-install -- install --repo",
        "--build",
    ] {
        assert!(shell_command.contains(expected), "missing {expected}: {shell_command}");
    }
    assert!(!shell_command.contains("install.sh"));
    assert!(!shell_command.contains("python"));
    let repo_path = repo.display().to_string();
    assert_eq!(lines.next().unwrap(), ["install", "--repo", &repo_path, "--build"].join("\0"));
    assert!(lines.next().is_none());
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
    let unknown=f.o.repo.join(".dist/Ruycast.app");stdfs::create_dir_all(&unknown).unwrap();stdfs::write(unknown.join("user-file"),b"keep me").unwrap();
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
    assert!(f.o.apps.join("Ruycast.app").is_dir());
    let agents=f.o.home.join("Library/LaunchAgents");assert!(!agents.join(format!("{OLD_ID}.plist")).exists());assert!(agents.join(format!("{NEW_ID}.plist")).is_file());
    assert_eq!(stdfs::canonicalize(&old).unwrap(),stdfs::canonicalize(f.o.home.join(".config/ruycast")).unwrap());
    assert_eq!(stdfs::read(unknown.join("user-file")).unwrap(),b"keep me");

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
        let unknown=f.o.repo.join(".dist/Ruycast.app");stdfs::create_dir_all(&unknown).unwrap();stdfs::write(unknown.join("user-file"),b"keep me").unwrap();
        let original_cli=stdfs::read_link(f.o.home.join(".local/bin/rustcast")).unwrap();
        let mut r = f.runner();
        r.fault = Some(fault);
        let error=owned_install(&f.o,&mut r).unwrap_err();
        if fault=="bootstrap"{assert!(error.contains("bootstrap"),"{error}");}
        if fault=="version"{assert!(error.contains("host")&&error.contains("version"),"{error}");}
        if fault == "sign" {
            assert!(error.contains("signer unavailable"), "{error}");
            assert!(old.is_dir() && !old.is_symlink());
        }
        let profile = if old.is_dir() && !old.is_symlink() {
            old
        } else {
            f.o.home.join(".config/ruycast")
        };
        assert_eq!(profile::hashes(&profile, None, &f.o.home).unwrap(), before);
        assert!(f.o.apps.join("Rustcast.app").exists());
        assert!(!f.o.apps.join("Ruycast.app").exists());
        assert_eq!(stdfs::read(unknown.join("user-file")).unwrap(),b"keep me");
        if ["bootstrap","version"].contains(&fault) {
            assert_eq!(stdfs::read_link(f.o.home.join(".local/bin/rustcast")).unwrap(),original_cli);
            assert!(stdfs::symlink_metadata(f.o.home.join(".local/bin/ruycast")).is_err());
            let alias=f.o.home.join(".config/rustcast");let migrated=f.o.home.join(".config/ruycast");assert!(alias.is_symlink());assert_eq!(stdfs::canonicalize(&alias).unwrap(),stdfs::canonicalize(&migrated).unwrap());assert_eq!(profile::hashes(&migrated,None,&f.o.home).unwrap(),before);
            assert!(f.o.home.join(format!("Library/LaunchAgents/{OLD_ID}.plist")).is_file());
        }

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

// Checker tests never delegate to System, codesign, security or a real compiler.
struct CheckRunner {
    calls: Vec<tools::Call>,
    requirement: String,
}
impl tools::Runner for CheckRunner {
    fn secret(&mut self) -> Result<String> { panic!("checker must never provision an identity") }
    fn running_host(&mut self, _: i32, _: &Path) -> Result<()> { panic!("checker must never launch a host") }
    fn run(&mut self, call: tools::Call) -> Result<tools::Output> {
        assert_eq!(call.timeout, std::time::Duration::from_secs(30));
        assert!(call.input.is_empty());
        let mut stderr = Vec::new();
        if call.program == Path::new("/owned/rustc") {
            assert_eq!(call.args[0], "--edition=2021");
            assert_eq!(call.args[2], "-o");
            // Model distinct compiler outputs without running any code.
            let source = stdfs::read(&call.args[1]).unwrap();
            fs::create(Path::new(&call.args[3]), &source, 0o700).unwrap();
        } else {
            assert_eq!(call.program, Path::new("/usr/bin/codesign"));
            match call.args[0].as_str() {
                "--force" | "--verify" => (),
                "--display" => stderr = format!("# designated => {}\n", self.requirement).into_bytes(),
                _ => panic!("unexpected checker action"),
            }
        }
        self.calls.push(call);
        Ok(tools::Output { success: true, stdout: vec![], stderr })
    }
}
#[test]
fn signing_check_owned_success_uses_two_distinct_pinned_bundles_without_state_mutation() {
    let f = Fixture::new();
    f.signer();
    let before: BTreeMap<_, _> = stdfs::read_dir(&f.o.signer).unwrap().map(|entry| {
        let entry = entry.unwrap();
        (entry.file_name(), (stdfs::read(entry.path()).unwrap(), stdfs::metadata(entry.path()).unwrap().permissions().mode()))
    }).collect();
    let parent = f.root.join("checker-work");
    fs::directory(&parent, 0o700).unwrap();
    let requirement = signing::requirement(&"A".repeat(40));
    let mut runner = CheckRunner { calls: vec![], requirement: signing::requirement(&"a".repeat(40)) };
    let result = signing::check(&f.o.signer, &f.o.repo, &f.o.home, &parent, Path::new("/owned/rustc"), &mut runner).unwrap();
    assert_eq!(result, json!({"ok":true,"differentNativeBinaries":true,"sameDesignatedRequirement":true,
        "bothVerifyAgainstPinnedCertificate":true,"certificate_sha1":"A".repeat(40),"requirement":requirement,
        "privacySettingsChanged":false,"fixturesLaunched":false}));
    assert_eq!(runner.calls.len(), 8);
    for (variant, calls) in runner.calls.chunks_exact(4).enumerate() {
        let actual_app = Path::new(calls[1].args.last().unwrap());
        assert!(actual_app.ends_with(format!("Variant{}.app", variant + 1)));
        assert!(actual_app.starts_with(&parent));
        assert_eq!(calls[0].program, Path::new("/owned/rustc"));
        assert_eq!(calls[1].args, ["--force", "--sign", &"A".repeat(40), "--keychain", f.o.signer.join("identity.keychain-db").to_str().unwrap(), "--timestamp=none", "--identifier", NEW_ID, "--requirements", &format!("=designated => {requirement}"), actual_app.to_str().unwrap()]);
        assert_eq!(calls[2].args, ["--verify", "--deep", "--strict", "-R", &format!("={requirement}"), actual_app.to_str().unwrap()]);
        assert_eq!(calls[3].args, ["--display", "--requirements", "-", actual_app.to_str().unwrap()]);
    }
    assert!(runner.calls.iter().all(|call| call.program == Path::new("/owned/rustc") || call.program == Path::new("/usr/bin/codesign")));
    assert!(runner.calls.iter().all(|call| !call.args.iter().any(|arg| arg.contains("owned-fixture-password"))));
    assert_eq!(stdfs::read_dir(&parent).unwrap().count(), 0);
    assert_eq!(stdfs::read_dir(&f.o.signer).unwrap().count(), before.len());
    for (name, (bytes, mode)) in before {
        let path = f.o.signer.join(name);
        assert_eq!(stdfs::read(&path).unwrap(), bytes);
        assert_eq!(stdfs::metadata(path).unwrap().permissions().mode(), mode);
    }
}
#[test]
fn signing_check_missing_or_unsafe_owned_identity_refuses_before_runner_or_staging() {
    for fault in ["missing", "unsafe"] {
        let f = Fixture::new();
        f.signer();
        if fault == "missing" { stdfs::remove_file(f.o.signer.join("identity.keychain-db")).unwrap(); }
        else { stdfs::set_permissions(f.o.signer.join("keychain-password"), stdfs::Permissions::from_mode(0o644)).unwrap(); }
        let before = stdfs::read(f.o.signer.join("identity.json")).unwrap();
        let parent = f.root.join("must-not-be-created");
        let mut runner = CheckRunner { calls: vec![], requirement: String::new() };
        assert!(signing::check(&f.o.signer, &f.o.repo, &f.o.home, &parent, Path::new("/owned/rustc"), &mut runner).is_err());
        assert!(runner.calls.is_empty());
        assert!(!parent.exists());
        assert_eq!(stdfs::read(f.o.signer.join("identity.json")).unwrap(), before);
    }
}

struct FailingCheckRunner {
    base: CheckRunner,
    fault: &'static str,
    at: usize,
    stages: Vec<PathBuf>,
}
impl tools::Runner for FailingCheckRunner {
    fn secret(&mut self) -> Result<String> { panic!("checker must never provision") }
    fn running_host(&mut self, _: i32, _: &Path) -> Result<()> { panic!("checker must never launch") }
    fn run(&mut self, call: tools::Call) -> Result<tools::Output> {
        let step = self.base.calls.len() + 1;
        assert_eq!(call.timeout, std::time::Duration::from_secs(30));
        assert!(call.input.is_empty());
        let compiler = call.program == Path::new("/owned/rustc");
        if compiler {
            let stage = call.cwd.as_ref().unwrap();
            assert_eq!(stdfs::metadata(stage).unwrap().permissions().mode() & 0o777, 0o700);
            self.stages.push(stage.clone());
        } else { assert_eq!(call.program, Path::new("/usr/bin/codesign")); }
        let executable = compiler.then(|| PathBuf::from(&call.args[3]));
        if step == self.at && matches!(self.fault, "exit" | "runner-error") {
            self.base.calls.push(call);
            let private = "owned-fixture-password raw-runner-stderr-canary";
            if self.fault == "runner-error" { return Err(private.into()); }
            return Ok(tools::Output { success: false, stdout: private.as_bytes().to_vec(), stderr: private.as_bytes().to_vec() });
        }
        let mut output = self.base.run(call)?;
        if self.fault == "same-binaries" {
            if let Some(path) = executable { stdfs::write(path, b"same owned executable bytes").unwrap(); }
        }
        if step == self.at {
            match self.fault {
                "missing" => output.stderr = b"raw-runner-stderr-canary owned-fixture-password".to_vec(),
                "multiple" => output.stderr.extend_from_within(..),
                "wrong" => output.stderr = b"# designated => identifier only raw-runner-stderr-canary owned-fixture-password\n".to_vec(),
                _ => (),
            }
        }
        Ok(output)
    }
}
#[test]
fn signing_check_owned_failure_table_redacts_stops_and_cleans_only_its_staging() {
    let mut cases = Vec::new();
    for step in 1..=8 {
        for fault in ["exit", "runner-error"] {
            let message = match (step - 1) % 4 {
                0 => "Signing check compiler failed.",
                1 => "Signing check signer failed.",
                2 => "Signing check verification failed.",
                _ => "Signing check requirement display failed.",
            };
            cases.push((fault, step, step, message));
        }
    }
    for step in [4, 8] {
        for fault in ["missing", "multiple", "wrong"] {
            cases.push((fault, step, step, "Signed app does not carry the exact pinned requirement."));
        }
    }
    cases.push(("same-binaries", 8, 8, "Changed native builds did not retain the same identity."));
    for (fault, at, count, expected) in cases {
        let f = Fixture::new();
        f.signer();
        let before: BTreeMap<_, _> = stdfs::read_dir(&f.o.signer).unwrap().map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), (stdfs::read(entry.path()).unwrap(), stdfs::metadata(entry.path()).unwrap().permissions().mode()))
        }).collect();
        let parent = f.root.join("failure-checker-work");
        fs::directory(&parent, 0o700).unwrap();
        let sibling = parent.join("preserve-existing-directory");
        fs::directory(&sibling, 0o700).unwrap();
        fs::create(&sibling.join("keep"), b"existing sibling must survive", 0o600).unwrap();
        let mut runner = FailingCheckRunner {
            base: CheckRunner { calls: vec![], requirement: signing::requirement(&"a".repeat(40)) },
            fault, at, stages: vec![],
        };
        let result = signing::check(&f.o.signer, &f.o.repo, &f.o.home, &parent, Path::new("/owned/rustc"), &mut runner);
        let error = result.unwrap_err(); // No success JSON is available on any fault.
        assert_eq!(error, expected, "{fault} at {at}");
        assert!(!error.contains("owned-fixture-password"));
        assert!(!error.contains("raw-runner-stderr-canary"));
        assert_eq!(runner.base.calls.len(), count, "no calls after {fault} at {at}");
        assert!(!runner.stages.is_empty());
        for stage in &runner.stages {
            assert!(stage.starts_with(&parent));
            assert!(stage.file_name().unwrap().to_str().unwrap().starts_with("signing-check-"));
            assert!(!stage.exists(), "only owned checker staging must be retired");
        }
        assert_eq!(stdfs::read_dir(&parent).unwrap().count(), 1);
        assert_eq!(stdfs::read(sibling.join("keep")).unwrap(), b"existing sibling must survive");
        assert_eq!(stdfs::metadata(&sibling).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(stdfs::read_dir(&f.o.signer).unwrap().count(), before.len());
        for (name, (bytes, mode)) in before {
            let path = f.o.signer.join(name);
            assert_eq!(stdfs::read(&path).unwrap(), bytes, "{fault} at {at}");
            assert_eq!(stdfs::metadata(path).unwrap().permissions().mode(), mode);
        }
    }
}

#[test]
fn signing_check_cli_owned_dispatch_and_legacy_status_help_preserve_boundaries() {
    let f = Fixture::new();
    f.signer();
    let parent = f.root.join("cli-check-work");
    fs::directory(&parent, 0o700).unwrap();
    let mut runner = CheckRunner { calls: vec![], requirement: signing::requirement(&"a".repeat(40)) };
    let result = signing_cli_dispatch(vec!["check".into(), "--root".into(), f.o.signer.display().to_string(),
        "--repo".into(), f.o.repo.display().to_string()], &f.o.home, &f.o.repo,
        || Ok((parent.clone(), PathBuf::from("/owned/rustc"))), &mut runner).unwrap().unwrap();
    assert_eq!(result, json!({"ok":true,"differentNativeBinaries":true,"sameDesignatedRequirement":true,
        "bothVerifyAgainstPinnedCertificate":true,"certificate_sha1":"A".repeat(40),
        "requirement":signing::requirement(&"A".repeat(40)),"privacySettingsChanged":false,"fixturesLaunched":false}));
    assert_eq!(runner.calls.len(), 8);
    assert_eq!(stdfs::read_dir(&parent).unwrap().count(), 0);
    runner.calls.clear();
    let status = signing_cli_dispatch(vec!["status".into()], &f.o.home, &f.o.repo,
        || panic!("status must not discover compiler or staging"), &mut runner).unwrap().unwrap();
    assert_eq!(status, signing::load(&f.o.signer, &f.o.repo).unwrap());
    for args in [vec![], vec!["check".into(), "--help".into()]] {
        assert!(signing_cli_dispatch(args, &f.o.home, &f.o.repo,
            || panic!("help must not discover compiler or staging"), &mut runner).unwrap().is_none());
    }
    assert!(runner.calls.is_empty());
    assert!(SIGNING_USAGE.contains("prepare|status|check|sign"));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(!repo.join("scripts/check-signing.py").exists());
    assert!(repo.join("scripts/local_signing.py").is_file());
}
#[test]
fn signing_check_cli_owned_invalid_arguments_or_identity_never_reach_effects() {
    let f = Fixture::new();
    let mut runner = CheckRunner { calls: vec![], requirement: String::new() };
    for args in [vec!["unknown"], vec!["check", "APP"], vec!["sign"], vec!["check", "--root"],
        vec!["check", "--repo", "one", "--repo", "two"], vec!["check", "--root", "one", "--root", "two"]] {
        assert!(signing_cli_dispatch(args.into_iter().map(str::to_owned).collect(), &f.o.home, &f.o.repo,
            || panic!("invalid command must not reach staging/compiler"), &mut runner).is_err());
    }
    // Default root is inside the injected owned HOME, never the local signer.
    assert!(signing_cli_dispatch(vec!["check".into()], &f.o.home, &f.o.repo,
        || panic!("missing identity must refuse before effects"), &mut runner).is_err());
    f.signer();
    stdfs::set_permissions(f.o.signer.join("keychain-password"), stdfs::Permissions::from_mode(0o644)).unwrap();
    assert!(signing_cli_dispatch(vec!["check".into()], &f.o.home, &f.o.repo,
        || panic!("unsafe identity must refuse before effects"), &mut runner).is_err());
    assert!(runner.calls.is_empty());
}

struct OrdinarySignRunner {
    keychain: PathBuf,
    refuse: bool,
    calls: Vec<tools::Call>,
}
impl tools::Runner for OrdinarySignRunner {
    fn secret(&mut self) -> Result<String> { panic!("ordinary sign must not provision") }
    fn running_host(&mut self, _: i32, _: &Path) -> Result<()> { panic!("ordinary sign must not launch") }
    fn run(&mut self, call: tools::Call) -> Result<tools::Output> {
        // Fake only; never delegate to System or execute any tool.
        assert_eq!(call.timeout, std::time::Duration::from_secs(30));
        let mut output = tools::Output { success: true, stdout: vec![], stderr: vec![] };
        match self.calls.len() {
            0 => {
                assert_eq!(call.program, Path::new("/usr/bin/security"));
                assert_eq!(call.args, ["list-keychains", "-d", "user"]);
                // Already enrolled in the search list: no list mutation needed.
                output.stdout = format!("\"{}\"\n", self.keychain.display()).into_bytes();
            }
            1 => {
                assert_eq!(call.program, Path::new("/usr/bin/security"));
                assert_eq!(call.args, ["-i"]);
                assert_eq!(call.input, format!("'unlock-keychain' '-p' 'owned-fixture-password' '{}'\n", self.keychain.display()).as_bytes());
            }
            2 => {
                assert_eq!(call.program, Path::new("/usr/bin/codesign"));
                assert_eq!(call.args[0], "--force");
                if self.refuse {
                    output.success = false;
                    output.stdout = b"owned-fixture-password raw-signer-stdout".to_vec();
                    output.stderr = b"owned-fixture-password raw-signer-stderr".to_vec();
                }
            }
            3 if !self.refuse => {
                assert_eq!(call.program, Path::new("/usr/bin/codesign"));
                assert_eq!(call.args[0], "--verify");
            }
            _ => panic!("no retry, fallback, trust repair or extra search-list call permitted"),
        }
        self.calls.push(call);
        Ok(output)
    }
}
#[test]
fn ordinary_sign_owned_pins_verification_and_refuses_without_fallback_or_mutation() {
    for refuse in [false, true] {
        let f = Fixture::new();
        f.signer();
        let identity = signing::load(&f.o.signer, &f.o.repo).unwrap();
        let before: BTreeMap<_, _> = stdfs::read_dir(&f.o.signer).unwrap().map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), (stdfs::read(entry.path()).unwrap(), stdfs::metadata(entry.path()).unwrap().permissions().mode()))
        }).collect();
        let directory_mode = stdfs::metadata(&f.o.signer).unwrap().permissions().mode();
        let app = f.root.join("owned-sign-test.app");
        let mut runner = OrdinarySignRunner { keychain: f.o.signer.join("identity.keychain-db"), refuse, calls: vec![] };
        let result = signing::sign(&app, &f.o.signer, &f.o.repo, &f.o.home, &mut runner);
        let requirement = signing::requirement(&"A".repeat(40));
        assert_eq!(runner.calls.len(), if refuse { 3 } else { 4 });
        assert_eq!(runner.calls[2].args, ["--force", "--sign", &"A".repeat(40), "--keychain",
            runner.keychain.to_str().unwrap(), "--timestamp=none", "--identifier", NEW_ID,
            "--requirements", &format!("=designated => {requirement}"), app.to_str().unwrap()]);
        assert_eq!(runner.calls.iter().filter(|call| call.program == Path::new("/usr/bin/codesign")
            && call.args.first().is_some_and(|arg| arg == "--force")).count(), 1);
        if refuse {
            let error = result.unwrap_err();
            assert_eq!(error, "codesign: operation failed; installation state preserved or rolled back.");
            for private in ["owned-fixture-password", "raw-signer-stdout", "raw-signer-stderr"] {
                assert!(!error.contains(private));
            }
            assert!(!runner.calls.iter().any(|call| call.args.first().is_some_and(|arg| arg == "--verify")));
        } else {
            let value = result.unwrap();
            assert_eq!(value, identity);
            assert!(!value.to_string().contains("owned-fixture-password"));
            assert_eq!(runner.calls[3].args, ["--verify", "--deep", "--strict", "-R",
                &format!("={requirement}"), app.to_str().unwrap()]);
        }
        assert!(runner.calls.iter().all(|call| !call.args.iter().any(|arg| arg.contains("owned-fixture-password")
            || arg == "add-trusted-cert" || arg == "-s") && !call.args.windows(2).any(|args| args == ["--sign", "-"])));
        assert!(!app.exists());
        assert!(!f.o.apps.exists());
        assert_eq!(stdfs::read_dir(&f.o.signer).unwrap().count(), before.len());
        assert_eq!(stdfs::metadata(&f.o.signer).unwrap().permissions().mode(), directory_mode);
        for (name, (bytes, mode)) in before {
            let path = f.o.signer.join(name);
            assert_eq!(stdfs::read(&path).unwrap(), bytes);
            assert_eq!(stdfs::metadata(path).unwrap().permissions().mode(), mode);
        }
    }
}
