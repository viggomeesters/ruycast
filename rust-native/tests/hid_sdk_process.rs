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
    run_case_with_trace(name, operations, 0, final_caps).0
}
fn run_case_with_trace(
    name: &str,
    operations: Value,
    environment: u32,
    final_caps: bool,
) -> (Value, String) {
    run_case_options(name, operations, true, environment, final_caps)
}
fn run_case_options(
    name: &str,
    operations: Value,
    original: bool,
    environment: u32,
    final_caps: bool,
) -> (Value, String) {
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
        serde_json::to_vec(&json!({"operations":operations,"original":original,"env":environment}))
            .unwrap(),
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
    (data["rows"].clone(), trace)
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

#[test]
fn identical_plan_generation_preserves_held_physical_and_hardware() {
    let (rows, trace) = run_case_with_trace(
        "same-plan-generation",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"plan","hyper":true,"generation":2}, {"op":"refresh"},
            {"op":"event","type":10,"key":0,"flags":0}
        ]),
        0,
        false,
    );
    assert_eq!(
        rows.as_array().unwrap().last().unwrap()["result"]["flags"],
        HYPER
    );
    assert_eq!(trace.matches("hid-fixture:manager_create\n").count(), 1);
}

#[test]
fn generated_events_pass_unmodified_and_secure_input_recovers() {
    let rows = run_case(
        "generated-secure-recovery",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"event","type":10,"key":0,"flags":1_u64<<16,"tag":0x5255535443415354_i64},
            {"op":"env","value":1}, {"op":"event","type":10,"key":0,"flags":CMD|16},
            {"op":"refresh"}, {"op":"env","value":0}, {"op":"refresh"}
        ]),
        false,
    );
    assert_eq!(rows[3]["result"]["flags"], 1_u64 << 16);
    assert_eq!(rows[5]["result"]["flags"], CMD | 16);
    assert_eq!(rows[6]["status"], "Paused during secure input");
    assert_eq!(rows[8]["status"], "On");
}

#[test]
fn off_sleep_and_environment_precedence_short_circuit_discovery() {
    let (rows, trace) = run_case_with_trace(
        "off-precedence",
        json!([
            {"op":"plan","hyper":false}, {"op":"refresh"}
        ]),
        15,
        true,
    );
    assert_eq!(rows.as_array().unwrap().last().unwrap()["status"], "Off");
    assert!(!trace.contains("hid-fixture:raycast"));
    for (environment, expected) in [
        (15, "Quit Raycast"),
        (7, "Paused during secure"),
        (6, "Allow Accessibility"),
        (4, "Allow Input Monitoring"),
    ] {
        let (rows, _) = run_case_with_trace(
            &format!("environment-{environment}"),
            json!([
                {"op":"plan","hyper":true}, {"op":"refresh"}
            ]),
            environment,
            true,
        );
        assert!(rows.as_array().unwrap().last().unwrap()["status"]
            .as_str()
            .unwrap()
            .contains(expected));
    }
    let (rows, trace) = run_case_with_trace(
        "sleep-precedence",
        json!([
            {"op":"plan","hyper":true}, {"op":"sleep","value":true}, {"op":"refresh"}
        ]),
        0,
        true,
    );
    assert_eq!(rows.as_array().unwrap().last().unwrap()["status"], "Paused");
    assert!(!trace.contains("hid-fixture:raycast"));
}

#[test]
fn partial_setup_failure_always_restores_original_and_can_retry() {
    for stage in [
        "tap_create",
        "source",
        "manager_create",
        "device_dictionary",
        "input_dictionary",
        "manager_open",
        "tap_enable",
    ] {
        for original in [false, true] {
            let (rows, _) = run_case_options(
                &format!("setup-{stage}-{original}"),
                json!([
                    {"op":"plan","hyper":true}, {"op":"failure","name":stage}, {"op":"refresh"},
                    {"op":"failure","name":""}, {"op":"refresh"}
                ]),
                original,
                0,
                false,
            );
            assert_ne!(rows[2]["status"], "On", "{stage}");
            assert_eq!(
                rows[2]["state"]["caps"],
                if original { 1 } else { 0 },
                "{stage}"
            );
            assert_eq!(rows[2]["state"]["allocated"], 0, "{stage}");
            assert_eq!(rows[4]["status"], "On", "{stage}");
        }
    }
}

#[test]
fn failed_manager_or_tap_retirement_survives_drop_and_blocks_new_setup() {
    for stage in ["manager_close", "invalidate"] {
        let rows = run_case(
            &format!("retirement-{stage}"),
            json!([
                {"op":"plan","hyper":true}, {"op":"refresh"},
                {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
                {"op":"failure","name":stage}, {"op":"drop"}, {"op":"new"},
                {"op":"plan","hyper":true}, {"op":"refresh"}, {"op":"failure","name":""},
                {"op":"recover"}, {"op":"refresh"}
            ]),
            false,
        );
        assert_ne!(rows[7]["status"], "On");
        assert_eq!(rows[9]["result"], true);
        assert_eq!(rows[10]["status"], "On");
    }
}

#[test]
fn failed_balancing_creation_retries_even_when_features_are_off() {
    let rows = run_case(
        "balancing-off-retry",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"failure","name":"event_source"}, {"op":"plan","hyper":false},
            {"op":"refresh"}, {"op":"failure","name":""}, {"op":"refresh"}
        ]),
        false,
    );
    assert!(rows[5]["status"]
        .as_str()
        .unwrap()
        .contains("cleanup interrupted"));
    assert_eq!(rows[7]["status"], "Off");
    assert_eq!(rows[7]["state"]["lastDown"], false);
    assert_eq!(rows[7]["state"]["lastFlags"], 0);
}

#[test]
fn caps_restore_failure_retains_authority_and_recovers_after_features_off() {
    let failures: u64 = (8..30).map(|bit| 1_u64 << bit).sum();
    let rows = run_case(
        "caps-restore-off-recovery",
        json!([
            {"op":"plan","hyper":true}, {"op":"caps-failures","value":failures},
            {"op":"failure","name":"tap_create"}, {"op":"refresh"}, {"op":"drop"}, {"op":"new"},
            {"op":"plan","hyper":false}, {"op":"refresh"}, {"op":"caps-failures","value":0},
            {"op":"failure","name":""}, {"op":"recover"}, {"op":"refresh"}
        ]),
        true,
    );
    assert!(rows[3]["status"]
        .as_str()
        .unwrap()
        .contains("cleanup interrupted"));
    assert_eq!(rows[3]["state"]["caps"], 3);
    assert_ne!(rows[7]["status"], "On");
    assert_eq!(rows[10]["result"], true);
    assert_eq!(rows[11]["status"], "Off");
    assert_eq!(rows[11]["state"]["caps"], 1);
}

#[test]
fn setup_reentry_stop_at_each_native_boundary_never_reports_on() {
    for stage in [
        "tap_create",
        "source",
        "add_source",
        "manager_create",
        "device_dictionary",
        "device_matching",
        "input_dictionary",
        "input_matching",
        "register_input",
        "register_remove",
        "schedule",
        "manager_open",
        "tap_enable",
        "tap_enabled",
    ] {
        let rows = run_case(
            &format!("setup-reentry-{stage}"),
            json!([
                {"op":"plan","hyper":true}, {"op":"hook","name":stage,"mode":"stop"}, {"op":"refresh"}
            ]),
            true,
        );
        let last = rows.as_array().unwrap().last().unwrap();
        assert_ne!(last["status"], "On", "{stage}");
        assert_eq!(last["state"]["allocated"], 0, "{stage}");
        assert_eq!(last["state"]["caps"], 1, "{stage}");
    }
}

#[test]
fn hyper_emission_reentry_does_not_post_stale_down_or_leave_modifiers() {
    for stage in [
        "event_source",
        "keyboard_event",
        "set_type",
        "set_flags",
        "set_field",
        "post",
    ] {
        let rows = run_case(
            &format!("emission-reentry-{stage}"),
            json!([
                {"op":"plan","hyper":true}, {"op":"refresh"}, {"op":"hook","name":stage,"mode":"stop"},
                {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7}, {"op":"refresh"}, {"op":"stop"}
            ]),
            false,
        );
        let last = rows.as_array().unwrap().last().unwrap();
        assert_eq!(last["state"]["lastDown"], false, "{stage}");
        assert_eq!(last["state"]["lastFlags"], 0, "{stage}");
        assert_eq!(last["state"]["caps"], 0, "{stage}");
    }
}

#[test]
fn event_flags_restore_after_setter_reentry_stop() {
    let original = (1_u64 << 63) | CMD | 16;
    let rows = run_case(
        "setter-reentry-flags",
        json!([
            {"op":"plan","hyper":true,"sided":{"right-cmd+a":"side"}}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"hook","name":"set_flags","mode":"stop"}, {"op":"event","type":10,"key":0,"flags":original}
        ]),
        false,
    );
    let last = rows.as_array().unwrap().last().unwrap();
    assert_eq!(last["result"], json!({"consumed":false,"flags":original}));
    assert_eq!(last["actions"], json!([]));
}

#[test]
fn lifecycle_gate_panic_preserves_native_ownership_for_retirement() {
    let rows = run_case(
        "lifecycle-gate-panic",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"}, {"op":"panic-gate","value":true},
            {"op":"refresh"}, {"op":"panic-gate","value":false}, {"op":"stop"}
        ]),
        false,
    );
    assert!(rows[3]["result"].as_str().unwrap().contains("panic"));
    assert_eq!(rows[3]["state"]["manager"], true);
    assert_eq!(rows[5]["state"]["allocated"], 0);
}

#[test]
fn callback_gate_panic_frees_temporary_native_events() {
    let rows = run_case(
        "callback-gate-panic",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"}, {"op":"hook","name":"set_field","mode":"panic-gate"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"panic-gate","value":false}, {"op":"stop"}
        ]),
        false,
    );
    assert_eq!(rows[3]["failed"], true);
    assert_eq!(rows[3]["state"]["allocated"], 3);
    assert_eq!(rows[3]["state"]["posts"], 0);
    assert_eq!(rows[5]["state"]["allocated"], 0);
}

#[test]
fn gate_reentry_with_running_true_cannot_post_or_return_stale_hyper() {
    let rows = run_case(
        "gate-running-raw",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"}, {"op":"gate-stop","after":2},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7}
        ]),
        false,
    );
    let last = rows.as_array().unwrap().last().unwrap();
    assert_eq!(last["state"]["posts"], 0);
    assert_eq!(last["state"]["allocated"], 0);
    let original = (1_u64 << 63) | CMD | 16;
    let rows = run_case(
        "gate-running-event",
        json!([
            {"op":"plan","hyper":true}, {"op":"refresh"},
            {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
            {"op":"gate-stop","after":2}, {"op":"event","type":10,"key":0,"flags":original}
        ]),
        false,
    );
    let last = rows.as_array().unwrap().last().unwrap();
    assert_eq!(last["result"], json!({"consumed":false,"flags":original}));
    assert_eq!(last["state"]["lastDown"], false);
}

#[test]
fn disabled_tap_notification_stops_and_refresh_recovers() {
    for notification in [0xffff_ffff_u32, 0xffff_fffe_u32] {
        let rows = run_case(
            &format!("tap-disabled-{notification}"),
            json!([
                {"op":"plan","hyper":true}, {"op":"refresh"},
                {"op":"input","usage":57,"down":true,"time":0,"device":0,"page":7},
                {"op":"event","type":notification,"key":0,"flags":0}, {"op":"refresh"}
            ]),
            false,
        );
        assert_ne!(rows[3]["status"], "On");
        assert_eq!(rows[4]["status"], "On");
        assert_eq!(rows[4]["state"]["lastDown"], false);
    }
}

#[test]
fn unmatched_and_nonkey_callbacks_do_not_clone_or_allocate_the_binding_plan() {
    let mut bindings = serde_json::Map::new();
    for side in ["left-cmd", "right-cmd"] {
        for modifiers in [
            "",
            "ctrl+",
            "alt+",
            "shift+",
            "ctrl+alt+",
            "ctrl+shift+",
            "alt+shift+",
            "ctrl+alt+shift+",
        ] {
            for key in "abcdefghijklmnopqrstuvwxyz".chars() {
                bindings.insert(
                    format!("{modifiers}{side}+{key}"),
                    json!(format!("item.{key}")),
                );
            }
        }
    }
    let rows = run_case(
        "callback-allocation-budget",
        json!([
            {"op":"plan","hyper":false,"sided":bindings}, {"op":"refresh"}, {"op":"allocation-events","count":1000}
        ]),
        true,
    );
    assert_eq!(
        rows.as_array().unwrap().last().unwrap()["result"],
        json!({"rustAllocations":0,"callbacks":3000})
    );
}
