// Source-built, never-loaded keyboard SDK qualification for this consumer.
struct OwnedKeyboardFixture {
    directory: PathBuf,
    artifact: PathBuf,
    expected_sha: String,
    sourcepins: BTreeMap<PathBuf, String>,
    owner: u32,
}
impl OwnedKeyboardFixture {
    fn verify(&self) {
        use std::os::unix::fs::MetadataExt;
        for (path, is_directory) in [(&self.directory, true), (&self.artifact, false)] {
            let m = fs::symlink_metadata(path).unwrap();
            assert!(!m.file_type().is_symlink() && m.uid() == self.owner);
            assert_eq!(m.is_dir(), is_directory);
            assert!(is_directory || m.is_file());
        }
        assert!(!self.expected_sha.is_empty());
        assert_eq!(
            fixture_sha(&self.artifact),
            self.expected_sha,
            "owned keyboard artifact changed after qualification"
        );
        assert_eq!(
            keyboard_fixture_sources(),
            self.sourcepins,
            "owned keyboard sources changed after qualification"
        );
    }
}
impl Drop for OwnedKeyboardFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn fixture_sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn assert_keyboard_source_hash(name: &str, actual: &str, expected: &str) {
    assert_eq!(actual, expected, "qualified fixture source changed: {name}");
}
fn keyboard_fixture_sources() -> BTreeMap<PathBuf, String> {
    let expected = [
        (
            "rust-native/tests/fixtures/keyboard_host.rs",
            "242f1ea4139965718fb48bd695d2c28b2266b70034314ebf3d271b87e58f5c0b",
        ),
        (
            "rust-native/tests/fixtures/caps_sdk.rs",
            "3e072d4b60cdb10d2f6d85f70589c1ad1353617ee63443c5c6d1806879cad2ac",
        ),
        (
            "rust-native/tests/fixtures/hid_sdk.rs",
            "0938d1bc1661386ff5d2ce353c25fcad81b147872cdda5ee877ce4451580ad47",
        ),
        (
            "rust-native/tests/fixtures/keyboard_sdk.rs",
            "5fbf16e3a849c0c3347cc5570a48bfcc5557decf0b92f03dec468af9a12b799c",
        ),
        (
            "rust-native/tests/fixtures/keyword_core.rs",
            "5269f7bfb20f44c4304fe7e46ec79b3f6cecf9a0d6aa4145a8b14d97a17314e6",
        ),
        (
            "rust-native/tests/fixtures/keyword_notification.rs",
            "9c98172da3060f1a7919e9ecec4c835739a671d5c5dcdb3816b6f6256e607d29",
        ),
        (
            "rust-native/tests/fixtures/keyword_board.rs",
            "ffd4554ee103d49aefcf4b5babc5b587d5741a188ad2e5f02ac3a8dbe287ab97",
        ),
        (
            "rust-native/tests/fixtures/keyword_ax.rs",
            "16164b329741c774aef0205f5d3582b0966aee5dadb87b98169241046a650cb0",
        ),
        (
            "rust-native/tests/fixtures/keyword_events.rs",
            "eb12af1f9d700d9c53cd73be3ba4caa961deaec88ce8e0a10aa3a5cfef74f199",
        ),
        (
            "rust-native/tests/fixtures/workspace_sdk.rs",
            "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05",
        ),
        (
            "rust-native/tests/fixtures/keyboard_host_loader.json",
            "e386ad6acd031f7122d15f0f3e05a7b94e2ce2d9047c96293d406ad96716ae25",
        ),
        (
            "rust-native/Cargo.toml",
            "3f0eb307ac5a065bd6a8322ccc318656484365a0b947bb195f7af020654870ba",
        ),
        (
            "rust-native/Cargo.lock",
            "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5",
        ),
        (
            "rust/Cargo.toml",
            "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949",
        ),
        (
            "rust-toolchain.toml",
            "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82",
        ),
    ];
    expected
        .into_iter()
        .map(|(name, expected)| {
            let path = root().parent().unwrap().join(name);
            let m = fs::symlink_metadata(&path).unwrap();
            assert!(m.is_file() && !m.file_type().is_symlink());
            let actual = fixture_sha(&path);
            assert_keyboard_source_hash(name, &actual, expected);
            (path, actual)
        })
        .collect()
}
fn fixture_tool(
    directory: &Path,
    name: &str,
    program: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
    let output_path = directory.join(format!("{name}.stdout"));
    let error_path = directory.join(format!("{name}.stderr"));
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output_path)
        .unwrap();
    let error = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&error_path)
        .unwrap();
    fs::write(
        directory.join(format!("{name}.argv.json")),
        serde_json::to_vec_pretty(&json!({"program":program,"arguments":args,"env":env})).unwrap(),
    )
    .unwrap();
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(directory)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(error)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let oversized = [&output_path, &error_path]
            .iter()
            .any(|p| fs::metadata(p).unwrap().len() > 2 * 1024 * 1024);
        if Instant::now() >= deadline || oversized {
            // Every compiler/linker descendant inherits this dedicated group.
            unsafe {
                libc::killpg(child.id() as i32, libc::SIGKILL);
            }
            let _ = child.wait();
            panic!("owned fixture tool exceeded time/output bound: {name}");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let read = |p: &Path| {
        assert!(fs::metadata(p).unwrap().len() <= 2 * 1024 * 1024);
        fs::read_to_string(p).unwrap()
    };
    let error = read(&error_path);
    assert!(
        status.success(),
        "owned fixture tool {name} failed: {error}"
    );
    read(&output_path)
}
fn keyboard_loader_policy() -> Value {
    serde_json::from_str(include_str!("../fixtures/keyboard_host_loader.json")).unwrap()
}
fn check_keyboard_loader(commands: &str, dependencies: &str, artifact: &Path) {
    let policy = keyboard_loader_policy();
    for forbidden in policy["forbiddenCommands"]
        .as_array()
        .unwrap()
        .iter()
        .chain(policy["forbiddenSections"].as_array().unwrap())
    {
        let forbidden = forbidden.as_str().unwrap();
        assert!(
            !commands.contains(forbidden),
            "unexpected keyboard loader entry: {forbidden}"
        );
    }
    for forbidden in [
        "LC_LOAD_WEAK_DYLIB",
        "LC_REEXPORT_DYLIB",
        "LC_LOAD_UPWARD_DYLIB",
        "LC_LAZY_LOAD_DYLIB",
    ] {
        assert!(
            !commands.contains(forbidden),
            "unexpected dependency type: {forbidden}"
        );
    }
    let sections = commands.split("Section").skip(1).collect::<Vec<_>>();
    for section in &sections {
        let flags = section
            .lines()
            .find_map(|line| line.trim().strip_prefix("flags "))
            .unwrap();
        let ty = u32::from_str_radix(flags.trim_start_matches("0x"), 16).unwrap() & 0xff;
        assert!(
            ![0x15, 0x16].contains(&ty),
            "unqualified TLS/offset initializer section"
        );
    }
    for (kind, section_type) in [("initializers", 9), ("finalizers", 10)] {
        let expected = &policy[kind];
        let name = expected["section"].as_str().unwrap();
        let field = |section: &str, key: &str| {
            section
                .lines()
                .find_map(|line| {
                    let (k, v) = line.trim().split_once(' ')?;
                    (k == key).then_some(v.trim())
                })
                .unwrap()
                .to_owned()
        };
        let matching = sections
            .iter()
            .filter(|s| {
                let flags = field(s, "flags");
                let ty = u32::from_str_radix(flags.trim_start_matches("0x"), 16).unwrap() & 0xff;
                ty == section_type || field(s, "sectname") == name
            })
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "exact keyboard {kind} section required");
        let section = matching[0];
        assert_eq!(field(section, "sectname"), name);
        assert_eq!(
            field(section, "segname"),
            expected["segment"].as_str().unwrap()
        );
        assert_eq!(
            u64::from_str_radix(field(section, "size").trim_start_matches("0x"), 16).unwrap(),
            expected["bytes"].as_u64().unwrap()
        );
        assert_eq!(
            u32::from_str_radix(field(section, "flags").trim_start_matches("0x"), 16).unwrap()
                & 0xff,
            section_type
        );
    }
    let mut expected = policy["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    expected.push(artifact.to_str().unwrap());
    let actual = dependencies
        .lines()
        .skip(1)
        .map(|line| line.trim().split(" (").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len());
    assert_eq!(
        actual.into_iter().collect::<BTreeSet<_>>(),
        expected.into_iter().collect(),
        "keyboard dependency roster changed"
    );
    let ids = commands
        .split("cmd LC_ID_DYLIB")
        .skip(1)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 1);
    assert!(
        ids[0].lines().any(|line| line
            .trim()
            .starts_with(&format!("name {} (offset ", artifact.display()))),
        "keyboard install identity must bind this owned output"
    );
}
fn check_keyboard_pointers(offsets: &str, symbols: &str, kind: &str) {
    let policy = keyboard_loader_policy();
    assert_eq!(policy["pointerWidthBytes"], 8);
    assert_eq!(policy["byteOrder"], "little");
    let words = offsets
        .lines()
        .skip(2)
        .flat_map(|line| line.split_whitespace().skip(1))
        .map(|word| u32::from_str_radix(word, 16).unwrap())
        .collect::<Vec<_>>();
    let expected = policy[kind]["symbols"].as_array().unwrap();
    assert_eq!(
        words.len(),
        expected.len() * 2,
        "keyboard {kind} count changed"
    );
    let names = words
        .chunks_exact(2)
        .map(|pair| {
            let address = u64::from(pair[0]) | (u64::from(pair[1]) << 32);
            let matches = symbols
                .lines()
                .filter_map(|line| {
                    let fields = line.split_whitespace().collect::<Vec<_>>();
                    (fields.first().and_then(|v| u64::from_str_radix(v, 16).ok()) == Some(address))
                        .then(|| *fields.last().unwrap())
                })
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "unique {kind} code symbol required");
            assert!(symbols
                .lines()
                .any(|line| line.contains("(__TEXT,__text)") && line.ends_with(matches[0])));
            matches[0]
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        expected
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        "keyboard {kind} code roster changed"
    );
}
fn build_owned_keyboard_fixture() -> OwnedKeyboardFixture {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sources = keyboard_fixture_sources();
    let setting = |key| {
        let p = PathBuf::from(std::env::var_os(key).expect(key));
        assert!(p.is_absolute());
        p
    };
    let parent = setting("RUYCAST_NATIVE_TEST_FIXTURE_DIR");
    let parent_metadata = fs::symlink_metadata(&parent).unwrap();
    assert!(
        parent_metadata.is_dir()
            && !parent_metadata.file_type().is_symlink()
            && parent_metadata.permissions().mode() & 0o077 == 0
    );
    let marker = parent.join(".ruycast-ui-fixture-parent");
    let metadata = fs::symlink_metadata(&marker).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert_eq!(fs::read(marker).unwrap(), b"owned-ui-fixture-parent-v1");
    let evidence = setting("RUYCAST_NATIVE_EVIDENCE_DIR");
    let metadata = fs::symlink_metadata(&evidence).unwrap();
    assert!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == parent_metadata.uid()
    );
    let name = format!(
        "keyboard-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let directory = parent.join(&name);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let mut owned = OwnedKeyboardFixture {
        artifact: directory.join("keyboard.dylib"),
        directory,
        expected_sha: String::new(),
        sourcepins: sources.clone(),
        owner: parent_metadata.uid(),
    };
    let text = |p: &Path| p.to_str().unwrap().to_owned();
    let toolchain = setting("RUYCAST_FIXTURE_TOOLCHAIN");
    let cache = setting("RUYCAST_FIXTURE_CARGO_HOME");
    let target = setting("RUYCAST_FIXTURE_TARGET_DIR");
    // Cargo runs in the new owned directory, with explicit cache/toolchain paths.
    // Reject cache configuration rather than inheriting wrappers or foreign flags.
    for base in
        std::iter::once(cache.clone()).chain(owned.directory.ancestors().map(|p| p.join(".cargo")))
    {
        for file in ["config", "config.toml"] {
            assert!(
                !base.join(file).exists(),
                "unqualified Cargo configuration: {}",
                base.display()
            );
        }
    }
    let cargo = toolchain.join("cargo");
    let rustc = toolchain.join("rustc");
    let mut env = BTreeMap::from([
        ("HOME".into(), text(&owned.directory)),
        ("TMPDIR".into(), text(&owned.directory)),
        (
            "PATH".into(),
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", toolchain.display()),
        ),
        ("CARGO_HOME".into(), text(&cache)),
        ("CARGO_TARGET_DIR".into(), text(&target)),
        ("CARGO_INCREMENTAL".into(), "0".into()),
        ("RUSTC".into(), text(&rustc)),
    ]);
    let compiler = fixture_tool(&owned.directory, "compiler", &rustc, &["-vV".into()], &env);
    let cargo_version = fixture_tool(
        &owned.directory,
        "cargo-version",
        &cargo,
        &["-Vv".into()],
        &env,
    );
    let policy = keyboard_loader_policy();
    assert!(cargo_version.lines().any(|line| line == "release: 1.92.0"));
    assert!(compiler.lines().any(|line| line
        == format!(
            "LLVM version: {}",
            policy["observed"]["llvm"].as_str().unwrap()
        )));
    assert!(compiler
        .lines()
        .any(|line| line == format!("rustc {}", policy["observed"]["rustc"].as_str().unwrap())));
    assert!(compiler.lines().any(|line| line
        == format!(
            "host: {}",
            policy["observed"]["architecture"].as_str().unwrap()
        )));
    let sdk = fixture_tool(
        &owned.directory,
        "sdk",
        Path::new("/usr/bin/xcrun"),
        &["--show-sdk-path".into()],
        &env,
    );
    let sdk_version = fixture_tool(
        &owned.directory,
        "sdk-version",
        Path::new("/usr/bin/xcrun"),
        &["--show-sdk-version".into()],
        &env,
    );
    assert_eq!(
        sdk_version.trim(),
        policy["observed"]["macOSSDK"].as_str().unwrap()
    );
    env.insert("SDKROOT".into(), sdk.trim().into());
    let compiler_sha = fixture_sha(&rustc);
    let cargo_sha = fixture_sha(&cargo);
    fixture_tool(
        &owned.directory,
        "build",
        &cargo,
        &[
            "build",
            "--manifest-path",
            &text(&root().join("Cargo.toml")),
            "--locked",
            "--offline",
            "--release",
            "--features",
            "test-tools",
            "--example",
            "keyboard-host",
        ]
        .map(str::to_owned),
        &env,
    );
    assert_eq!(
        sources,
        keyboard_fixture_sources(),
        "fixture sources changed during Rust build"
    );
    assert_eq!(compiler_sha, fixture_sha(&rustc));
    assert_eq!(cargo_sha, fixture_sha(&cargo));
    let built = target.join("release/examples/libkeyboard_host.dylib");
    let built_sha = fixture_sha(&built);
    fs::copy(&built, &owned.artifact).unwrap();
    assert_eq!(built_sha, fixture_sha(&owned.artifact));
    fixture_tool(
        &owned.directory,
        "identity",
        Path::new("/usr/bin/install_name_tool"),
        &["-id".into(), text(&owned.artifact), text(&owned.artifact)],
        &env,
    );
    fixture_tool(
        &owned.directory,
        "sign",
        Path::new("/usr/bin/codesign"),
        &["--force", "--sign", "-", &text(&owned.artifact)].map(str::to_owned),
        &env,
    );
    fixture_tool(
        &owned.directory,
        "signature",
        Path::new("/usr/bin/codesign"),
        &["--verify", "--strict", &text(&owned.artifact)].map(str::to_owned),
        &env,
    );
    let commands = fixture_tool(
        &owned.directory,
        "loadcommands",
        Path::new("/usr/bin/otool"),
        &["-l".into(), text(&owned.artifact)],
        &env,
    );
    let dependencies = fixture_tool(
        &owned.directory,
        "dependencies",
        Path::new("/usr/bin/otool"),
        &["-L".into(), text(&owned.artifact)],
        &env,
    );
    check_keyboard_loader(&commands, &dependencies, &owned.artifact);
    let symbols = fixture_tool(
        &owned.directory,
        "symbols",
        Path::new("/usr/bin/nm"),
        &["-nm".into(), text(&owned.artifact)],
        &env,
    );
    for kind in ["initializers", "finalizers"] {
        let section = &policy[kind];
        let offsets = fixture_tool(
            &owned.directory,
            kind,
            Path::new("/usr/bin/otool"),
            &[
                "-s".into(),
                section["segment"].as_str().unwrap().into(),
                section["section"].as_str().unwrap().into(),
                text(&owned.artifact),
            ],
            &env,
        );
        check_keyboard_pointers(&offsets, &symbols, kind);
    }
    owned.expected_sha = fixture_sha(&owned.artifact);
    assert_eq!(sources, keyboard_fixture_sources());
    assert_eq!(
        built_sha,
        fixture_sha(&built),
        "shared build artifact must remain unchanged"
    );
    fs::write(owned.directory.join("receipt.json"), serde_json::to_vec_pretty(&json!({"sources":sources,"compiler":compiler,"compilerSHA256":compiler_sha,"cargo":cargo_version,"cargoSHA256":cargo_sha,"sdk":sdk.trim(),"sdkVersion":sdk_version.trim(),"builtArtifact":built,"builtSHA256":built_sha,"artifact":owned.artifact,"sha256":owned.expected_sha,"loaderPolicy":policy,"loaded":false,"qualification":"source-built-owned-rust-keyboard"})).unwrap()).unwrap();
    let output = evidence.join(name);
    fs::DirBuilder::new().mode(0o700).create(&output).unwrap();
    for entry in fs::read_dir(&owned.directory).unwrap() {
        let path = entry.unwrap().path();
        // SDK/compiler temporary directories are not evidence artifacts.
        if !path.is_file() {
            continue;
        }
        let copy = output.join(path.file_name().unwrap());
        fs::copy(&path, &copy).unwrap();
        fs::set_permissions(copy, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(
        owned.expected_sha,
        fixture_sha(&output.join("keyboard.dylib"))
    );
    owned.verify();
    owned
}
