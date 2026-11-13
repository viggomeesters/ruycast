//! Auto Quit preferences through the existing prohibited/offscreen app editor.
#![cfg(feature = "test-tools")]
#[path = "support/native_bridge.rs"]
#[allow(dead_code)]
mod native_bridge;
use native_bridge::{runtime_host, Bridge};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn view(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let result = host.probe("read", json!({}));
        if predicate(&result) {
            return result;
        }
        assert!(Instant::now() < deadline, "Editor did not settle: {result}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn edit(host: &Bridge) {
    host.rpc(&json!({"op":"show","query":"app.synthetic"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    host.probe("key-event", json!({"code":40,"flags":1<<20,"chars":"k"}));
    let menu = view(host, |v| v["actionsVisible"] == true);
    let index = menu["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|label| label.as_str().unwrap().starts_with("Edit"))
        .unwrap();
    host.probe("action-button", json!({"index":index}));
    view(host, |v| v["formVisible"] == true);
}
fn input(host: &Bridge, key: &str, text: &str) {
    host.probe("command-input", json!({"key":key,"text":text}));
}
fn save(host: &Bridge) {
    host.probe("save-editor", json!({}));
    view(host, |v| v["formVisible"] == false);
}
#[test]
fn app_editor_persists_auto_quit_and_renders_light_and_dark() {
    let mut host = Bridge::new(runtime_host::Config::current(), false);
    let mut config = runtime_host::default_config();
    config["commands"] = json!([{
        "id":"app.synthetic","title":"Synthetic Editor","kind":"app",
        "target":"test.synthetic.Editor","aliases":[]
    }]);
    config["autoQuit"] = json!({"test.synthetic.Player":{"enabled":true,"minutes":44}});
    config["theme"] = json!("light");
    host.replace(&config, None);
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.start())).is_err() {
        let output = host.stop(false).unwrap();
        panic!(
            "Owned editor host startup failed: {} {} {}", output.status,
            String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout)
        );
    }
    host.settled(|_| true);
    edit(&host);
    assert_eq!(
        host.probe("command-read", json!({}))["value"]["_autoQuit"],
        json!({"enabled":false,"minutes":30})
    );
    let initial = fs::read(&host.config).unwrap();
    save(&host);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap(),
        serde_json::from_slice::<Value>(&initial).unwrap()
    );
    let initial = fs::read(&host.config).unwrap();
    edit(&host);
    input(&host, "autoQuitMinutes", "0");
    host.probe("save-editor", json!({}));
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("positive whole number"))
    });
    assert_eq!(fs::read(&host.config).unwrap(), initial);
    input(&host, "autoQuitMinutes", "17");
    host.probe("command-input", json!({"key":"autoQuit","value":true}));
    host.probe("command-input", json!({"key":"autoQuitRisk","value":true}));
    save(&host);
    for theme in ["light", "dark"] {
        let mut current: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        assert_eq!(
            current["autoQuit"]["test.synthetic.Editor"],
            json!({"enabled":true,"minutes":17,"allowUnverifiedActivity":true})
        );
        assert_eq!(current["autoQuit"]["test.synthetic.Player"]["minutes"], 44);
        current["theme"] = json!(theme);
        host.replace(&current, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        host.settled(|s| s["theme"] == theme);
        edit(&host);
        host.probe("command-scroll", json!({"y":140}));
        let screen = host.probe("read", json!({}));
        assert_eq!(screen["windowShown"], false);
        assert!(screen["screenText"]
            .as_array()
            .unwrap()
            .contains(&json!("Auto Quit")));
        assert!(screen["screenText"]
            .as_array()
            .unwrap()
            .contains(&json!("Quit after (minutes)")));
        assert!(screen["screenText"]
            .as_array()
            .unwrap()
            .contains(&json!("May interrupt recordings or helper audio.")));
        assert!(screen["screenText"]
            .as_array()
            .unwrap()
            .contains(&json!("Allow quitting when activity is unknown")));
        if let Some(directory) = std::env::var_os("RUYCAST_AUTO_QUIT_EVIDENCE_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            host.probe("screenshot", json!({}));
            fs::copy(
                host.profile.join("native-probe.png"),
                directory.join(format!("auto-quit-{theme}.png")),
            )
            .unwrap();
            fs::write(
                directory.join(format!("auto-quit-{theme}.json")),
                serde_json::to_vec_pretty(&screen).unwrap(),
            )
            .unwrap();
        }
        host.probe("cancel-editor", json!({}));
        view(&host, |v| v["formVisible"] == false);
    }
    edit(&host);
    host.probe("command-input", json!({"key":"autoQuit","value":false}));
    save(&host);
    edit(&host);
    assert_eq!(
        host.probe("command-read", json!({}))["value"]["_autoQuit"],
        json!({"enabled":false,"minutes":17,"allowUnverifiedActivity":true})
    );
    host.probe("cancel-editor", json!({}));
}
