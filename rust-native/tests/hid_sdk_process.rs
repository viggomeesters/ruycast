//! Original standalone HID SDK scenarios through actual Rust owner/callback probes.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
const HYPER: u64 = (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20);
const CMD: u64 = 1 << 20;
struct Owned(PathBuf);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn run_case(name: &str, operations: Value, final_caps: bool) -> Value {
    let probe = Path::new(env!("CARGO_BIN_EXE_hid-probe"));
    assert!(probe.is_absolute() && probe.is_file());
    let artifact = std::env::var_os("RUYCAST_TEST_HID_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            probe
                .parent()
                .unwrap()
                .join("examples/libhid_fixture.dylib")
        });
    assert!(
        artifact.is_absolute() && artifact.is_file(),
        "Build hid-fixture first"
    );
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Own absolute fixture parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    assert_eq!(parent.canonicalize().unwrap(), parent);
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = parent.join(format!(
        "hid-sdk-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let owned = Owned(directory);
    let library = owned.0.join("complete.dylib");
    fs::copy(&artifact, &library).unwrap();
    let sdk_hash = hash(&library);
    assert_eq!(hash(&artifact), sdk_hash);
    let input = owned.0.join("input.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"operations":operations,"original":true,"env":0})).unwrap(),
    )
    .unwrap();
    let child = Command::new(probe)
        .env_clear()
        .env("DYLD_INSERT_LIBRARIES", &library)
        .stdin(fs::File::open(input).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(10));
    let trace = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{trace}");
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["cleanup"]["allocated"], 0, "{trace}");
    assert_eq!(
        data["cleanup"]["caps"],
        if final_caps { 1 } else { 0 },
        "{trace}"
    );
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        fs::write(PathBuf::from(directory).join(format!("hid-sdk-{name}.json")),serde_json::to_vec_pretty(&json!({"data":data,"trace":trace,"probe":{"path":probe,"sha256":hash(probe)},"sdk":{"path":artifact,"sha256":sdk_hash,"privateCopyHashChecked":true},"timeoutSeconds":10})).unwrap()).unwrap();
    }
    data["rows"].clone()
}
#[test]
fn hyper_physical_down_up_flags_and_balanced_stop() {
    let rows = run_case(
        "physical",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"event","type":10,"key":0,"flags":(1_u64<<63)|(1<<16)|0x10},
            {"op":"input","usage":57,"down":false,"time":0,"device":0,"page":7}, {"op":"stop"}
        ]),
        false,
    );
    assert_eq!(rows[1]["status"], "On");
    assert_eq!(rows[2]["state"]["lastDown"], true);
    assert_eq!(rows[2]["state"]["lastFlags"], HYPER);
    assert_eq!(
        rows[3]["result"],
        json!({"consumed":false,"flags":(1_u64<<63)|0x10|HYPER})
    );
    assert_eq!(rows[4]["state"]["lastFlags"], 0);
    assert_eq!(rows[5]["state"]["caps"], 0);
}
#[test]
fn caps_on_two_devices_and_removal_releases_only_last() {
    let rows = run_case(
        "two-devices",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"input","usage":57,"down":true,"time":0,"device":1,"page":7},
            {"op":"remove","device":0}, {"op":"event","type":10,"key":0,"flags":0},
            {"op":"remove","device":1}, {"op":"event","type":10,"key":0,"flags":0}
        ]),
        false,
    );
    assert_eq!(rows[5]["result"]["flags"], HYPER);
    assert_eq!(rows[7]["result"]["flags"], 0);
    assert_eq!(rows[6]["state"]["lastDown"], false);
}

#[test]
fn sided_repeat_release_and_backpressure_do_not_eat_input() {
    let rows = run_case(
        "sided-admission",
        json!([
            {"op":"plan","hyper":false,"sided":{"right-cmd+a":"item"}}, {"op":"refresh"},
            {"op":"accept","value":false}, {"op":"event","type":10,"key":0,"flags":CMD|16},
            {"op":"event","type":11,"key":0,"flags":CMD|16}, {"op":"accept","value":true},
            {"op":"event","type":10,"key":0,"flags":CMD|16},
            {"op":"event","type":10,"key":0,"flags":CMD|16,"repeat":true},
            {"op":"event","type":11,"key":0,"flags":CMD|16},
            {"op":"event","type":11,"key":0,"flags":CMD|16},
            {"op":"event","type":10,"key":0,"flags":CMD|8}
        ]),
        true,
    );
    assert_eq!(rows[3]["result"]["consumed"], false);
    assert_eq!(rows[4]["result"]["consumed"], false);
    for index in [6, 7, 8] {
        assert_eq!(rows[index]["result"]["consumed"], true);
    }
    for index in [9, 10] {
        assert_eq!(rows[index]["result"]["consumed"], false);
    }
    assert_eq!(
        rows.as_array().unwrap().last().unwrap()["actions"],
        json!([{"generation":1,"canonical":"right-cmd+a","action":"item"}])
    );
}

#[test]
fn exact_canonical_route_is_preserved_when_two_bindings_share_an_id() {
    let rows = run_case(
        "canonical-route",
        json!([
            {"op":"plan","hyper":false,"sided":{"right-cmd+a":"same","left-cmd+a":"same"}}, {"op":"refresh"},
            {"op":"event","type":10,"key":0,"flags":CMD|16},
            {"op":"event","type":11,"key":0,"flags":CMD|16},
            {"op":"event","type":10,"key":0,"flags":CMD|8}
        ]),
        true,
    );
    let actions = rows.as_array().unwrap().last().unwrap()["actions"]
        .as_array()
        .unwrap();
    assert_eq!(
        actions
            .iter()
            .map(|action| action["canonical"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["right-cmd+a", "left-cmd+a"]
    );
}

#[test]
fn recorder_episode_without_callbacks_invalidates_taps_and_swallowed_keys() {
    let rows = run_case(
        "recording-epoch",
        json!([
            {"op":"plan","hyper":true,"modifiers":{"224":"tap"},"sided":{"right-cmd+a":"side"}},
            {"op":"refresh"}, {"op":"input","usage":224,"down":true,"time":0,"device":0,"page":7},
            {"op":"gate","epoch":1,"recording":true}, {"op":"gate","epoch":2},
            {"op":"input","usage":224,"down":false,"time":0.2,"device":0,"page":7},
            {"op":"event","type":10,"key":0,"flags":CMD|16},
            {"op":"gate","epoch":3,"recording":true}, {"op":"event","type":11,"key":0,"flags":CMD|16},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"event","type":10,"key":0,"flags":CMD|16}
        ]),
        false,
    );
    assert_eq!(rows[5]["actions"], json!([]));
    assert_eq!(rows[8]["result"]["consumed"], false);
    assert_eq!(rows[10]["result"]["consumed"], false);
    assert_eq!(rows[10]["result"]["flags"], HYPER | 16);
}

#[test]
fn scroll_and_mouse_cancel_modifier_taps_with_native_types() {
    for kind in [1, 3, 10, 22, 25] {
        let rows = run_case(
            &format!("modifier-interrupt-{kind}"),
            json!([
                {"op":"plan","hyper":false,"modifiers":{"224":"tap"}}, {"op":"refresh"},
                {"op":"input","usage":224,"down":true,"time":0,"device":0,"page":7},
                {"op":"event","type":kind,"key":0,"flags":0},
                {"op":"input","usage":224,"down":false,"time":0.2,"device":0,"page":7}
            ]),
            true,
        );
        assert_eq!(
            rows.as_array().unwrap().last().unwrap()["actions"],
            json!([]),
            "Native event type {kind}"
        );
    }
    let rows = run_case(
        "modifier-uninterrupted",
        json!([
            {"op":"plan","hyper":false,"modifiers":{"224":"tap"}}, {"op":"refresh"},
            {"op":"input","usage":224,"down":true,"time":0,"device":0,"page":7},
            {"op":"input","usage":224,"down":false,"time":0.5,"device":0,"page":7}
        ]),
        true,
    );
    assert_eq!(
        rows.as_array().unwrap().last().unwrap()["actions"],
        json!([{"generation":1,"canonical":"ctrl+left-control","action":"tap"}])
    );
}
