//! Configured native lists through actual isolated AppKit profile checks.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Duration};

fn profile() -> Host {
    // These profile probes use only the host executable and direct RPC. Keep
    // the unused CLI path empty so a CLI artifact is not a fixture requirement.
    let host = Host::new(
        Config {
            host: PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native")),
            cli: PathBuf::new(),
        },
        false,
    );
    fs::write(
        host.base.join(".ruycast-ui-fixture-parent"),
        b"owned-ui-fixture-parent-v1",
    )
    .unwrap();
    host
}
fn contains(value: &Value, text: &str) -> bool {
    value.as_array().unwrap().iter().any(|v| v == text)
}
fn render(host: &Host, case: &str, query: &str, theme: &str, width: &str, success: bool) -> Value {
    let image = host.base.join(format!("native-{theme}-{width}.png"));
    let mut command = host.command(&host.settings.host);
    command
        .args([
            "--check-profile-native",
            image.to_str().unwrap(),
            theme,
            width,
            query,
        ])
        .env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", &host.base);
    let output = runtime_host::run(&mut command, Duration::from_secs(10));
    assert_eq!(
        output.status.success(),
        success,
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], success);
    if success {
        assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        for field in ["windowShown", "externalExecution", "productionReady"] {
            assert_eq!(reply[field], false);
        }
        if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let folder = PathBuf::from(folder);
            fs::create_dir_all(&folder).unwrap();
            let hash = format!("{:x}", Sha256::digest(query.as_bytes()));
            let name = format!("test_{case}-{}-{theme}-{width}", &hash[..8]);
            fs::copy(image, folder.join(format!("{name}.png"))).unwrap();
            fs::write(
                folder.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&reply).unwrap(),
            )
            .unwrap();
        }
    }
    assert!(!host.profile.join("control.sock").exists());
    reply
}
#[test]
fn loaded_config_alias_unicode_disabled_and_native_labels() {
    let host = profile();
    let mut config = default_config();
    config["commands"] = json!([
        {"id":"fixture.active","title":"Café Ω notes","kind":"url","target":"https://example.com/","aliases":["café"],"arguments":[],"hotkey":"right-cmd+n"},
        {"id":"fixture.disabled","title":"Hidden Ω","kind":"url","target":"https://example.com/","aliases":["hidden"],"arguments":[],"enabled":false}
    ]);
    host.replace(&config, None);
    let before = fs::read(&host.config).unwrap();
    for (theme, width) in [("light", "600"), ("dark", "760")] {
        let reply = render(
            &host,
            "loaded_config_alias_unicode_disabled_and_native_labels",
            "café",
            theme,
            width,
            true,
        );
        assert_eq!(reply["configuredCommands"], 2);
        assert_eq!(reply["nativeRows"], 1);
        assert_eq!(reply["selectedId"], "fixture.active");
        for text in ["Café Ω notes", "Quicklink", "R⌘ N", "Open ↵"] {
            assert!(contains(&reply["screenText"], text));
        }
        assert!(contains(&reply["accessibilityText"], "Right Command + n"));
        assert!(!contains(&reply["screenText"], "Hidden Ω"));
    }
    assert_eq!(fs::read(&host.config).unwrap(), before);
}
#[test]
fn different_command_kinds_keep_contextual_footer() {
    let host = profile();
    let mut config = default_config();
    let defaults = config["commands"].as_array().unwrap();
    let mut fixtures = Vec::new();
    for kind in ["app", "snippet", "script", "window"] {
        let mut command = defaults.iter().find(|c| c["kind"] == kind).cloned().unwrap_or_else(|| {
            let target = match kind { "script" => "/usr/bin/true", "window" => "left", _ => "fixture" };
            json!({"id":format!("fixture.{kind}"),"title":kind,"kind":kind,"target":target,"aliases":[],"arguments":[]})
        });
        command["id"] = json!(format!("fixture.{kind}"));
        command["title"] = json!(format!("Configured {kind}"));
        command["aliases"] = json!([format!("pick-{kind}")]);
        for key in ["hotkey", "keyword", "snippetOutput"] {
            command.as_object_mut().unwrap().remove(key);
        }
        fixtures.push(command);
    }
    config["commands"] = json!(fixtures);
    host.replace(&config, None);
    for (kind, expected) in [
        ("app", "Open"),
        ("snippet", "Insert Snippet"),
        ("script", "Run"),
        ("window", "Run"),
    ] {
        let reply = render(
            &host,
            "different_command_kinds_keep_contextual_footer",
            &format!("pick-{kind}"),
            "dark",
            "760",
            true,
        );
        assert_eq!(reply["selectedId"], format!("fixture.{kind}"));
        assert!(contains(&reply["screenText"], &format!("{expected} ↵")));
    }
}
#[test]
fn empty_and_long_native_content() {
    let host = profile();
    let mut config = default_config();
    config["commands"] = json!([{"id":"fixture.long","title":"Extremely long configured command Ω ".repeat(24),"kind":"url","target":"https://example.com/","aliases":["long"],"arguments":[]}]);
    host.replace(&config, None);
    let reply = render(
        &host,
        "empty_and_long_native_content",
        "long",
        "light",
        "600",
        true,
    );
    assert_eq!(reply["nativeRows"], 1);
    let reply = render(
        &host,
        "empty_and_long_native_content",
        "missing-zzzzzzzzzzzzzzzzzz",
        "dark",
        "760",
        true,
    );
    assert_eq!(reply["nativeRows"], 0);
    assert!(reply.get("selectedId").unwrap().is_null());
    assert!(contains(&reply["screenText"], "No commands found"));
    assert!(!contains(&reply["screenText"], "Open ↵"));
}
#[test]
fn invalid_configuration_releases_profile_lease() {
    let host = profile();
    host.replace_bytes(b"broken", None);
    render(
        &host,
        "invalid_configuration_releases_profile_lease",
        "",
        "dark",
        "760",
        false,
    );
    host.replace(&default_config(), None);
    render(
        &host,
        "invalid_configuration_releases_profile_lease",
        "",
        "dark",
        "760",
        true,
    );
}
#[test]
fn existing_owner_is_not_borrowed_for_native_probe() {
    let mut host = profile();
    host.start();
    let image = host.base.join("must-not-render.png");
    let mut command = host.command(&host.settings.host);
    command.args(["--check-profile-native", image.to_str().unwrap()]);
    let output = runtime_host::run(&mut command, Duration::from_secs(5));
    assert!(!output.status.success());
    assert!(!image.exists());
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
    assert!(host.profile.join("control.sock").exists());
}
