use rustcast_core::script::{self, End};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn source_uses_token(source: &str, token: &str) -> bool {
    source.match_indices(token).any(|(start, _)| {
        let before_is_word = source[..start]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        let after_is_word = source[start + token.len()..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        !before_is_word && !after_is_word
    })
}

#[test]
fn portable_models_have_no_platform_or_runtime_dependency() {
    let root = repository();
    let manifest = fs::read_to_string(root.join("rust/Cargo.toml")).unwrap();
    for dependency in [
        "objc2",
        "objc2-app-kit",
        "objc2-foundation",
        "core-foundation",
    ] {
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{dependency} ="))),
            "portable Rust core depends on {dependency}"
        );
    }
    let source = fs::read_to_string(root.join("rust/src/lib.rs")).unwrap();
    for symbol in [
        "FileManager",
        "URLSession",
        "NSWorkspace",
        "NSApplication",
        "AppKit",
    ] {
        assert!(
            !source_uses_token(&source, symbol),
            "portable Rust core contains {symbol}"
        );
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rc-architecture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compile_fixture(name: &str) -> (bool, String) {
    let fixture = Fixture::new();
    let source = repository().join("Tests/fixtures").join(name);
    let output = fixture.0.join("architecture-probe");
    let rustc = std::env::split_paths(&std::env::var_os("PATH").expect("Rust toolchain PATH"))
        .map(|directory| directory.join("rustc"))
        .find(|path| path.is_file())
        .expect("rustc for architecture compile check");
    let deps = std::env::current_exe()
        .expect("integration test executable")
        .parent()
        .expect("integration test deps directory")
        .to_owned();
    let objc2_artifacts: Vec<PathBuf> = fs::read_dir(&deps)
        .expect("Rust dependency directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy().starts_with("libobjc2-")
                    && path.extension().is_some_and(|ext| ext == "rlib")
            })
        })
        .collect();
    assert_eq!(
        objc2_artifacts.len(),
        1,
        "expected exactly one current Cargo objc2 artifact, found {objc2_artifacts:?}"
    );
    let objc2 = &objc2_artifacts[0];
    let command: rustcast_core::Command = serde_json::from_value(json!({
        "id":"owned.architecture.rust-compile",
        "title":"Rust thread ownership compile check",
        "kind":"script",
        "target":rustc,
        "arguments":["--edition=2021","--emit=metadata","-L",format!("dependency={}",deps.display()),"--extern",format!("objc2={}",objc2.display()),source,"-o",output],
        "timeout":60.0
    }))
    .unwrap();
    let completion = script::run(&command, &fixture.0, None, || false).unwrap();
    let success = matches!(completion.end, End::Exited(status) if status.success());
    (
        success,
        format!(
            "end={:?}; stdout={}; stderr={}",
            completion.end, completion.stdout, completion.stderr
        ),
    )
}

#[test]
fn main_actor_and_worker_boundaries_are_compiler_checked() {
    let root = repository();
    let services = fs::read_to_string(root.join("rust-native/src/native_services.rs")).unwrap();
    assert!(services.contains("main:crate::native_thread_boundary::MainThreadState"));
    assert!(services.contains("_main: crate::native_thread_boundary::MainThreadState"));
    assert!(services.contains("crate::native_thread_boundary::spawn_runtime(move ||"));
    let boundary =
        fs::read_to_string(root.join("rust-native/src/native_thread_boundary.rs")).unwrap();
    for production_fragment in [
        "pub struct MainThreadState",
        "marker: MainThreadMarker",
        "pub fn spawn_runtime<F>",
        "thread::Builder::new()",
        ".name(\"ruycast-runtime\".into())",
        ".spawn(worker)",
    ] {
        assert!(
            boundary.contains(production_fragment),
            "production worker boundary changed: {production_fragment}"
        );
    }
    let (success, stderr) = compile_fixture("architecture_boundary_positive.rs");
    assert!(success, "positive control failed: {stderr}");
    for name in [
        "architecture_boundary_main_state_on_worker.rs",
        "architecture_boundary_worker_captures_main_state.rs",
    ] {
        let (success, stderr) = compile_fixture(name);
        assert!(
            !success,
            "forbidden main-thread state transfer compiled: {name}"
        );
        assert!(
            stderr.contains("Send"),
            "expected a Rust thread-safety diagnostic for {name}: {stderr}"
        );
    }
}
