#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn supervised_work_failures_log_only_invocation_bound_source() {
    let mut host = Host::new(Config::current(), false);
    let dir = host.base.join(".local/libexec");
    fs::create_dir_all(&dir).unwrap();
    let writer = dir.join("viggo-work-writer");
    fs::copy(env!("CARGO_BIN_EXE_work-failure-fixture"), &writer).unwrap();
    fs::set_permissions(&writer, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = default_config();
    for (id, action) in [
        ("capture.task-from-note", "from-selection"),
        ("capture.convert-note", "convert-note"),
    ] {
        config["commands"].as_array_mut().unwrap().push(json!({"id":id,"title":id,"kind":"script","target":writer,"arguments":["task-active",action,"{uuid}",""],"timeout":10}));
    }
    host.replace(&config, None);
    host.start();
    for id in ["capture.task-from-note", "capture.convert-note"] {
        let result = host.cli_output(&["run", id]);
        let response = String::from_utf8_lossy(&result.stdout);
        assert!(!response.contains("00-inbox/demo/note.md"), "{response}");
        let log = host.rpc(&json!({"op":"logs","commandID":id,"errorsOnly":true}));
        assert_eq!(
            log["result"][0]["affectedFile"], "00-inbox/demo/note.md",
            "{log}"
        );
        assert!(!log.to_string().contains("Synthetic error at"));
    }
    let performance = host.rpc(&json!({"op":"performance"}));
    for stage in ["script.helperSpawn", "script.helperReady", "script.dispatch", "script.response", "script.finalize"] {
        assert_eq!(performance["result"]["stages"][stage]["count"], 2, "{performance}");
    }
    assert!(!performance.to_string().contains("00-inbox/demo/note.md"));
    let analytics = host.rpc(&json!({"op":"analytics"}));
    assert_eq!(analytics["ok"], true);
    assert!(!analytics.to_string().contains("00-inbox/demo/note.md"));
    let export = host.rpc(&json!({"op":"analytics.export"}));
    assert_eq!(export["ok"], true);
    assert!(!export.to_string().contains("00-inbox/demo/note.md"));
}
