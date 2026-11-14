//! Real NSWorkspace ABI behind an owned, complete synthetic SDK.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned directory shares bounded child support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
struct Fixture {
    owned: runtime_host::OwnedDir,
    trace: Vec<Value>,
}
fn request(kind: &str, target: &str) -> Value {
    json!({"kind":kind,"target":target,"commandId":"fixture","title":"Fixture"})
}
impl Fixture {
    fn new() -> Self {
        let owned = runtime_host::OwnedDir::new();
        fs::write(
            owned.0.join(".ruycast-test-fixture"),
            "own-disposable-fixture-v1",
        )
        .unwrap();
        Self {
            owned,
            trace: vec![],
        }
    }
    fn probe(&mut self, name: &str, requests: Vec<Value>) -> Value {
        self.probe_cases(
            name,
            requests.into_iter().map(|r| json!({"request":r})).collect(),
            json!({}),
        )
    }
    fn probe_cases(&mut self, name: &str, cases: Vec<Value>, input: Value) -> Value {
        self.run_cases(name, cases, input, true)
    }
    fn run_cases(&mut self, name: &str, cases: Vec<Value>, mut input: Value, shim: bool) -> Value {
        let probe = PathBuf::from(env!("CARGO_BIN_EXE_workspace-probe"));
        let original = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                probe
                    .parent()
                    .unwrap()
                    .join("examples/libworkspace_fixture.dylib")
            });
        assert!(
            original.is_absolute() && original.is_file(),
            "Build workspace-fixture first"
        );
        let copy = self.owned.0.join("workspace-shim.dylib");
        fs::copy(&original, &copy).unwrap();
        assert_eq!(
            Sha256::digest(fs::read(&original).unwrap()),
            Sha256::digest(fs::read(&copy).unwrap())
        );
        let mut cmd = Command::new(probe);
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            if key_text.starts_with("RUYCAST_")
                || key_text.starts_with("RUSTCAST_")
                || key == "DYLD_INSERT_LIBRARIES"
            {
                cmd.env_remove(key);
            }
        }
        cmd.env("HOME", &self.owned.0)
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &self.owned.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if shim {
            cmd.env("DYLD_INSERT_LIBRARIES", &copy);
        }
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        input["cases"] = json!(cases);
        let bytes = serde_json::to_vec(&input).unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(&bytes));
        let out = child_output::wait(child, Duration::from_secs(12));
        writer.join().unwrap().unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        let result: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["ownApplicationActive"], false);
        assert_eq!(result["ownWindowCount"], 0);
        let trace = self.owned.0.join("workspace-trace.jsonl");
        self.trace = if trace.exists() {
            fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        } else {
            vec![]
        };
        if let Some(first) = self.trace.first().filter(|_| shim) {
            assert_eq!(first["phase"], "installed");
            assert_eq!(first["selectors"], 4);
        }
        if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let folder = PathBuf::from(folder);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(format!("workspace-{name}.json")),serde_json::to_vec_pretty(&json!({"input":input,"result":result,"trace":self.trace,"sdkInjected":shim,"sdkSha256":format!("{:x}",Sha256::digest(fs::read(&copy).unwrap()))})).unwrap()).unwrap();
        }
        result
    }
    fn opens(&self) -> Vec<&Value> {
        self.trace
            .iter()
            .filter(|e| e["phase"].as_str().unwrap().starts_with("open-"))
            .collect()
    }
}
#[test]
fn app_callback_from_worker_and_exact_response() {
    let mut f = Fixture::new();
    let data = f.probe("app", vec![request("app", "dev.fixture.success")]);
    let v = &data["cases"][0];
    assert_eq!(
        v["response"],
        json!({"ok":true,"result":{"command":"fixture","state":"opened"}})
    );
    assert_eq!(v["beforeCommitCalls"], 1);
    assert_eq!(
        f.trace
            .iter()
            .map(|e| e["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["installed", "resolve", "open-app", "callback"]
    );
    assert!(f.trace[1..].iter().all(|e| e["mainThread"] == false));
}
#[test]
fn missing_and_invalid_application_never_commit() {
    let mut f = Fixture::new();
    let d = f.probe(
        "missing",
        ["missing", "invalid-location"]
            .into_iter()
            .map(|m| request("app", &format!("dev.fixture.{m}")))
            .collect(),
    );
    for v in d["cases"].as_array().unwrap() {
        assert_eq!(v["response"]["ok"], false);
        assert_eq!(v["committed"], false);
        assert_eq!(v["beforeCommitCalls"], 0);
    }
    assert!(f.opens().is_empty());
    assert_eq!(
        d["cases"][0]["response"]["error"],
        "App is not installed: dev.fixture.missing"
    );
}
#[test]
fn default_url_uses_synchronous_boolean_and_encoding() {
    let mut f = Fixture::new();
    let d = f.probe(
        "url",
        vec![request(
            "url",
            "ruycast-fixture://success/Ω space?q=hello world",
        )],
    );
    assert_eq!(d["cases"][0]["response"]["ok"], true);
    assert_eq!(d["cases"][0]["pendingCallbacks"], 0);
    assert_eq!(f.opens()[0]["phase"], "open-default");
    assert!(f.opens()[0]["url"].as_str().unwrap().contains("%20"));
}
#[test]
fn default_url_known_false_is_not_unknown() {
    let mut f = Fixture::new();
    let d = f.probe(
        "reject",
        vec![request("url", "ruycast-fixture://reject/path")],
    );
    assert_eq!(d["cases"][0]["committed"], true);
    assert_eq!(
        d["cases"][0]["response"],
        json!({"ok":false,"error":"macOS could not open Fixture."})
    );
}
#[test]
fn path_unicode_newline_literal_and_missing() {
    let mut f = Fixture::new();
    let path = f.owned.0.join("Ω file\n$(literal)");
    fs::write(&path, "fixture").unwrap();
    let d = f.probe(
        "path",
        vec![
            request("path", path.to_str().unwrap()),
            request("path", f.owned.0.join("missing").to_str().unwrap()),
        ],
    );
    assert_eq!(d["cases"][0]["response"]["ok"], true);
    assert!(f.opens()[0]["url"].as_str().unwrap().contains("%0A"));
    assert_eq!(d["cases"][1]["committed"], false);
    assert_eq!(d["cases"][1]["beforeCommitCalls"], 0);
    assert!(d["cases"][1]["response"]["error"]
        .as_str()
        .unwrap()
        .contains("Path does not exist:"));
}
#[test]
fn explicit_open_with_preserves_state_only_shape() {
    let mut f = Fixture::new();
    let mut r = request("url", "https://example.invalid/a");
    r["applicationId"] = json!("dev.fixture.success");
    let d = f.probe("open-with", vec![r]);
    assert_eq!(
        d["cases"][0]["response"],
        json!({"ok":true,"result":{"state":"opened"}})
    );
    let targets = f
        .trace
        .iter()
        .find(|e| e["phase"] == "open-with-targets")
        .unwrap();
    assert_eq!(targets["urls"], json!(["https://example.invalid/a"]));
    assert_eq!(f.opens().last().unwrap()["phase"], "open-with");
}
#[test]
fn missing_open_with_never_commit() {
    let mut f = Fixture::new();
    let mut r = request("url", "https://example.invalid");
    r["applicationId"] = json!("dev.fixture.missing");
    let d = f.probe("open-with-missing", vec![r]);
    assert_eq!(d["cases"][0]["committed"], false);
    assert_eq!(
        d["cases"][0]["response"]["error"],
        "The selected app is not installed. Choose another app in Settings."
    );
    assert!(f.opens().is_empty());
}
#[test]
fn native_error_and_bounded_large_error() {
    let mut f = Fixture::new();
    let d = f.probe(
        "error",
        ["error", "large-error"]
            .into_iter()
            .map(|m| request("app", &format!("dev.fixture.{m}")))
            .collect(),
    );
    assert_eq!(
        d["cases"][0]["response"],
        json!({"ok":false,"error":"Fixture native error."})
    );
    assert_eq!(d["cases"][1]["committed"], true);
    let error = d["cases"][1]["response"]["error"].as_str().unwrap();
    assert!(error.len() < 65536);
    assert!(error.ends_with('…'));
}

fn unknown(value: &Value) {
    assert_eq!(value["committed"], true);
    assert_eq!(value["response"]["ok"], false);
    assert_eq!(value["response"]["actionCompleted"], true);
    assert_eq!(value["response"]["code"], "open-outcome-unknown");
}
#[test]
fn nil_callback_result_is_unknown() {
    let mut f = Fixture::new();
    let d = f.probe("nil-result", vec![request("app", "dev.fixture.nil-result")]);
    unknown(&d["cases"][0]);
}
#[test]
fn reentrant_and_duplicate_first_callback_wins() {
    let mut f = Fixture::new();
    let d = f.probe(
        "duplicate",
        ["inline", "duplicate"]
            .into_iter()
            .map(|mode| request("app", &format!("dev.fixture.{mode}")))
            .collect(),
    );
    assert!(d["cases"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["response"]["ok"] == true));
    assert_eq!(
        f.trace.iter().filter(|e| e["phase"] == "callback").count(),
        3
    );
}
#[test]
fn cancellation_after_sdk_entry_retires_late_completion() {
    let mut f = Fixture::new();
    let d = f.probe_cases(
        "callback-cancel",
        vec![json!({"request":request("app","dev.fixture.slow"),"cancelAfterOpen":true})],
        json!({"settleMs":300}),
    );
    unknown(&d["cases"][0]);
    assert_eq!(d["afterRelease"], 0);
    assert_eq!(
        f.trace
            .iter()
            .map(|e| e["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["installed", "resolve", "open-app", "callback"]
    );
}
#[test]
fn callback_timeout_retires_late_completion() {
    let mut f = Fixture::new();
    // Warm NSWorkspace in the same process before measuring the callback deadline.
    let d = f.probe_cases(
        "callback-timeout",
        vec![
            json!({"request":request("app","dev.fixture.success")}),
            json!({"request":request("app","dev.fixture.slow"),"timeoutMs":75}),
        ],
        json!({"settleMs":300}),
    );
    assert_eq!(d["cases"][0]["response"]["ok"], true);
    unknown(&d["cases"][1]);
    assert_eq!(d["afterRelease"], 0);
}
#[test]
fn retained_callbacks_bound_capacity_until_sdk_releases() {
    let mut f = Fixture::new();
    let mut cases = vec![json!({"request":request("app","dev.fixture.success")})];
    cases.extend(
        (0..17).map(|_| json!({"request":request("app","dev.fixture.hold"),"timeoutMs":20})),
    );
    let d = f.probe_cases("callback-capacity", cases, json!({"releaseCallbacks":true}));
    for v in &d["cases"].as_array().unwrap()[1..17] {
        unknown(v);
    }
    let blocked = &d["cases"][17];
    assert_eq!(blocked["committed"], false);
    assert_eq!(blocked["beforeCommitCalls"], 0);
    assert!(blocked["response"]["error"]
        .as_str()
        .unwrap()
        .contains("busy"));
    assert_eq!(d["beforeRelease"], 16);
    assert_eq!(d["afterRelease"], 0);
    assert_eq!(
        f.trace.iter().filter(|e| e["phase"] == "open-app").count(),
        17
    );
}
#[test]
fn panic_before_and_after_commit_preserves_truth_and_releases_callbacks() {
    let mut f = Fixture::new();
    let d = f.probe_cases(
        "callback-panic",
        vec![
            json!({"request":request("app","dev.fixture.success"),"panicBeforeCommit":true}),
            json!({"request":request("app","dev.fixture.slow"),"panicAfterOpen":true}),
        ],
        json!({"settleMs":300}),
    );
    assert_eq!(d["cases"][0]["committed"], false);
    unknown(&d["cases"][1]);
    assert_eq!(d["afterRelease"], 0);
    assert_eq!(
        f.trace.iter().filter(|e| e["phase"] == "open-app").count(),
        1
    );
}

#[test]
fn precommit_rejection_has_no_open() {
    let mut f = Fixture::new();
    let d = f.probe_cases(
        "precommit-rejected",
        vec![json!({"request":request("app","dev.fixture.success"),"rejectBeforeCommit":true})],
        json!({}),
    );
    assert_eq!(d["cases"][0]["beforeCommitCalls"], 1);
    assert_eq!(d["cases"][0]["committed"], false);
    assert!(f.opens().is_empty());
}
#[test]
fn preexpired_request_does_not_even_resolve() {
    let mut f = Fixture::new();
    let d = f.probe_cases(
        "preexpired",
        vec![json!({"request":request("app","dev.fixture.success"),"timeoutMs":0})],
        json!({}),
    );
    assert_eq!(d["cases"][0]["committed"], false);
    assert_eq!(f.trace.len(), 1);
}
#[test]
fn blocked_resolution_is_not_claimed_as_hard_deadline() {
    let mut f = Fixture::new();
    let d = f.probe_cases(
        "blocked-resolution",
        vec![json!({"request":request("app","dev.fixture.slow-resolve"),"timeoutMs":75})],
        json!({}),
    );
    assert_eq!(d["cases"][0]["committed"], false);
    assert_eq!(d["cases"][0]["beforeCommitCalls"], 0);
    assert!(d["cases"][0]["seconds"].as_f64().unwrap() >= 0.15);
    assert_eq!(
        f.trace
            .iter()
            .map(|e| e["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["installed", "resolve"]
    );
}
#[test]
fn missing_interposer_fails_closed() {
    let mut f = Fixture::new();
    let d = f.run_cases(
        "missing-interposer",
        vec![json!({"request":request("app","dev.fixture.success")})],
        json!({}),
        false,
    );
    assert_eq!(d["cases"][0]["committed"], false);
    assert!(d["cases"][0]["response"]["error"]
        .as_str()
        .unwrap()
        .contains("boundary"));
    assert!(f.trace.is_empty());
}
#[test]
fn invalid_marker_fails_closed() {
    let mut f = Fixture::new();
    fs::write(f.owned.0.join(".ruycast-test-fixture"), "wrong").unwrap();
    let d = f.probe(
        "invalid-marker",
        vec![request("app", "dev.fixture.success")],
    );
    assert_eq!(d["cases"][0]["committed"], false);
    assert!(f.trace.is_empty());
}
#[test]
fn fixture_denies_real_targets_and_external_paths() {
    let mut f = Fixture::new();
    std::os::unix::fs::symlink("/Applications", f.owned.0.join("escape")).unwrap();
    let d = f.probe(
        "denied-targets",
        vec![
            request("app", "com.apple.finder"),
            request("url", "https://apple.com"),
            request("path", "/Applications"),
            request(
                "path",
                f.owned.0.join("escape/Safari.app").to_str().unwrap(),
            ),
            request("path", f.owned.0.join("../outside").to_str().unwrap()),
            request("url", "http://[bad"),
        ],
    );
    assert!(d["cases"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["committed"] == false && c["response"]["ok"] == false));
    assert!(f.opens().is_empty());
}
#[test]
fn nul_rejected_before_native_resolution() {
    let mut f = Fixture::new();
    let d = f.probe("nul", vec![request("app", "dev.fixture.\0bad")]);
    assert_eq!(d["cases"][0]["committed"], false);
    assert_eq!(f.trace.len(), 1);
}
// Match pathlib.Path.as_uri without canonicalizing the deliberately hostile .. path.
fn file_uri(path: &std::path::Path) -> String {
    assert!(path.is_absolute());
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/_.-~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(url, "%{byte:02X}").unwrap();
        }
    }
    url
}
#[test]
fn file_url_and_resolved_app_urls_cannot_escape_fixture() {
    let mut f = Fixture::new();
    std::os::unix::fs::symlink("/Applications", f.owned.0.join("escape")).unwrap();
    let mut open_with = request("url", "https://example.invalid");
    open_with["applicationId"] = json!("dev.fixture.x/../../../outside");
    let d = f.probe(
        "denied-file-urls",
        vec![
            request("url", &file_uri(&f.owned.0.join("escape/Fake.app"))),
            request("url", &file_uri(&f.owned.0.join("../outside"))),
            request("app", "dev.fixture.x/../../../outside"),
            open_with,
        ],
    );
    assert!(d["cases"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["committed"] == false && c["response"]["ok"] == false));
    assert!(f.opens().is_empty());
}
