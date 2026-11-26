//! Optional, bounded diagnostics from the admitted Work task router.
use rustcast_core::{script::Capture, Command};
use serde_json::Value;
use std::{fs, os::unix::fs::MetadataExt, path::Path};

pub(crate) const ENV: &str = "VIGGOBSIDIAN_DIAGNOSTIC_ID";
const SCHEMA: &str = "ruycast.work-failure-context.v1";

fn trusted_writer(original: &Command, home: &Path) -> bool {
    let target = original
        .target
        .strip_prefix("~/")
        .map(|p| home.join(p))
        .unwrap_or_else(|| original.target.as_str().into());
    if original.kind != "script" || target != home.join(".local/libexec/viggo-work-writer") {
        return false;
    }
    let Ok(info) = fs::symlink_metadata(&target) else { return false; };
    if fs::canonicalize(&target).ok().as_deref() != Some(target.as_path()) {
        return false;
    }
    if !info.is_file()
        || info.file_type().is_symlink()
        || info.uid() != unsafe { libc::getuid() }
        || info.mode() & 0o6022 != 0
        || info.mode() & 0o111 == 0
    {
        return false;
    }
    true
}
fn uuid(id: &str) -> bool {
    id.len() == 36 && id.bytes().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == b'-' } else { c.is_ascii_hexdigit() })
}
pub(crate) fn valid_capture_receipt(command: &Command, request_id: &str, response: &Value, capture: Option<&Capture>) -> bool {
    use sha2::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt { ok: bool, path: String, note_path: String, kind: String, reused: bool }
    let Some(capture) = capture else { return false; };
    if !capture.stdout.eof || capture.stdout.truncated || !capture.stdout.valid_utf8 || !capture.stderr.eof || capture.stderr.truncated || !capture.stderr.valid_utf8 || response["ok"] != true || response["result"]["exitCode"] != 0 { return false; }
    let Some(stdout) = response["result"]["stdout"].as_str() else { return false; };
    let Ok(receipt) = serde_json::from_str::<Receipt>(stdout) else { return false; };
    let expected_kind = match command.id.as_str() { "capture.quick-note" => "note", "capture.quick-task" => "task", _ => return false };
    if !receipt.ok || receipt.kind != expected_kind || receipt.path != receipt.note_path { return false; }
    let _ = receipt.reused;
    let args = command.arguments.as_slice();
    let offset = usize::from(args.first().is_some_and(|v| v == "{invocation-provenance}"));
    let Some(root) = args.get(offset + 1).and_then(|path| fs::canonicalize(path).ok()) else { return false; };
    if !uuid(request_id) { return false; }
    let path = Path::new(&receipt.path);
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink()) { return false; }
    if fs::canonicalize(path).ok().as_deref() != Some(path) { return false; }
    let Ok(relative) = path.strip_prefix(root) else { return false; };
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 3 { return false; }
    let [std::path::Component::Normal(inbox), std::path::Component::Normal(folder), std::path::Component::Normal(file)] = parts.as_slice() else { return false; };
    let digest = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    *inbox == std::ffi::OsStr::new("00-inbox") && folder.to_str().is_some_and(|name| name.ends_with(&format!("-{}", &digest[..20])) && file.to_str().is_some_and(|file| file == format!("{name}.md")))
}
pub(crate) fn pregrant_capture_error(response: &Value, capture: Option<&Capture>) -> Option<&'static str> {
    let capture = capture?;
    if response["ok"] != false || !capture.stdout.eof || capture.stdout.truncated || !capture.stdout.valid_utf8 { return None; }
    let body: Value = serde_json::from_str(response["stdout"].as_str()?).ok()?;
    if body["ok"] != false { return None; }
    match body["error"].as_str()? {
        "Writer-toestemming vereist de gecontroleerde Work-root" => Some("Writer-toestemming vereist de gecontroleerde Work-root"),
        "Work-root mag geen symlink zijn" => Some("Work-root mag geen symlink zijn"),
        "Kies de expliciete Work-root met lokale providerbinding" => Some("Kies de expliciete Work-root met lokale providerbinding"),
        "Work-captureprovider is niet aan deze root gebonden" => Some("Work-captureprovider is niet aan deze root gebonden"),
        "Geef note/task, volledige tekst en een geldige request-id" => Some("Geef note/task, volledige tekst en een geldige request-id"),
        _ => None,
    }
}
pub(crate) fn capture_admission(original: &Command, prepared: &Command, home: &Path) -> Result<bool, String> {
    let expected_kind = match original.id.as_str() { "capture.quick-note" => "note", "capture.quick-task" => "task", _ => return Ok(false) };
    let target = original.target.strip_prefix("~/").map(|p| home.join(p)).unwrap_or_else(|| original.target.as_str().into());
    if target != home.join(".local/libexec/viggo-work-writer") { return Ok(false); }
    let args = original.arguments.as_slice();
    let offset = usize::from(args.first().is_some_and(|v| v == "{invocation-provenance}"));
    let args = args.get(offset..).unwrap_or_default();
    let valid = trusted_writer(original, home) && original.kind == prepared.kind && original.target == prepared.target
        && args.len() == 5 && args[0] == "capture" && Path::new(&args[1]).is_absolute() && args[2] == expected_kind && args[3] == "{uuid}" && args[4] == "{input}"
        && prepared.arguments.len() == 5 && prepared.arguments[0] == args[0] && prepared.arguments[1] == args[1] && prepared.arguments[2] == args[2] && uuid(&prepared.arguments[3]);
    if valid { Ok(true) } else { Err("The Work capture configuration could not be verified.".into()) }
}
pub(crate) fn invocation(original: &Command, prepared: &Command, home: &Path) -> Option<String> {
    if !trusted_writer(original, home) { return None; }
    let args = original.arguments.as_slice();
    let offset = usize::from(args.first().is_some_and(|v| v == "{invocation-provenance}"));
    let args = args.get(offset..)?;
    if args.len() != 4
        || args[0] != "task-active"
        || args[2] != "{uuid}"
        || !matches!(
            args[1].as_str(),
            "from-selection"
                | "todo"
                | "backlog"
                | "in-progress"
                | "waiting"
                | "done"
                | "cancelled"
                | "plan"
                | "deadline"
                | "task-project"
                | "format-bubbles"
                | "convert-note"
        )
    {
        return None;
    }
    if prepared.kind != original.kind
        || prepared.target != original.target
        || prepared.arguments.len() != 4
        || prepared.arguments[0] != args[0]
        || prepared.arguments[1] != args[1]
    {
        return None;
    }
    let id = prepared.arguments.get(2)?;
    if !uuid(id) {
        return None;
    }
    Some(id.clone())
}

fn relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.ends_with(".md")
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

/// Strip the reserved metadata before ordinary result parsing, even if invalid.
/// Return its file only for a complete, invocation-bound failure envelope.
pub(crate) fn take(
    response: &mut Value,
    capture: Option<&Capture>,
    id: &str,
) -> Option<Option<String>> {
    let nested = response
        .get("result")
        .and_then(|v| v.get("stdout"))
        .is_some_and(Value::is_string);
    let stdout = if nested {
        response.get_mut("result")?.get_mut("stdout")?
    } else {
        response.get_mut("stdout")?
    };
    if capture.is_none_or(|c| !c.stdout.eof || c.stdout.truncated || !c.stdout.valid_utf8) {
        if stdout
            .as_str()
            .is_some_and(|text| text.contains("failureContext"))
        {
            *stdout = Value::String(String::new());
        }
        return None;
    }
    let text = stdout.as_str()?;
    let mut payload: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            if text.contains("failureContext") {
                *stdout = Value::String(String::new());
            }
            return None;
        }
    };
    let Some(object) = payload.as_object_mut() else {
        return None;
    };
    let context = object.remove("failureContext")?;
    let is_failure = object.get("ok") == Some(&Value::Bool(false));
    let trusted = is_failure
        && context["schema"] == SCHEMA
        && context["invocationID"] == id
        && context["scope"] == "work";
    let path = context["relativePath"]
        .as_str()
        .filter(|path| trusted && relative_path(path))
        .map(str::to_owned);
    if let Some(path) = &path {
        for key in ["error", "detail"] {
            if let Some(text) = object.get(key).and_then(Value::as_str) {
                object.insert(key.into(), Value::String(text.replace(path, "[source]")));
            }
        }
    }
    *stdout = Value::String(serde_json::to_string(&payload).ok()?);
    if let Some(path) = &path {
        for field in ["stderr", "error"] {
            if let Some(value) = response.get_mut(field) {
                if let Some(text) = value.as_str() {
                    *value = Value::String(text.replace(path, "[source]"));
                }
            }
            if let Some(value) = response
                .get_mut("result")
                .and_then(|result| result.get_mut(field))
            {
                if let Some(text) = value.as_str() {
                    *value = Value::String(text.replace(path, "[source]"));
                }
            }
        }
    }
    trusted.then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn capture_admission_requires_exact_native_writer_and_literal_template() {
        let home = std::env::temp_dir().join(format!("ruycast-capture-admission-{}", std::process::id()));
        fs::create_dir_all(home.join(".local/libexec")).unwrap();
        let home = home.canonicalize().unwrap();
        let target = home.join(".local/libexec/viggo-work-writer");
        fs::write(&target, b"owned fictional writer").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        for (id, kind) in [("capture.quick-note", "note"), ("capture.quick-task", "task")] {
            let original: Command = serde_json::from_value(json!({"id":id,"title":"Fixture","kind":"script","target":target,"arguments":["capture","/fictional-work",kind,"{uuid}","{input}"]})).unwrap();
            let mut prepared = original.clone();
            prepared.arguments[3] = "12345678-1234-1234-1234-123456789abc".into();
            prepared.arguments[4] = "fictional".into();
            assert_eq!(capture_admission(&original, &prepared, &home), Ok(true));
            let mut changed = prepared.clone();
            changed.arguments[1] = "/other".into();
            assert!(capture_admission(&original, &changed, &home).is_err());
            changed = prepared.clone(); changed.arguments[3] = "bad".into();
            assert!(capture_admission(&original, &changed, &home).is_err());
            let mut unrelated = original.clone(); unrelated.id = "capture.private-quick-note".into();
            assert_eq!(capture_admission(&unrelated, &prepared, &home), Ok(false));
        }
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn admitted_capture_requires_complete_scoped_typed_receipt() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("ruycast-capture-receipt-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let command: Command = serde_json::from_value(json!({"id":"capture.quick-note","title":"Fixture","kind":"script","target":"/fixture","arguments":["capture",root,"note","{uuid}","{input}"]})).unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        let digest = format!("{:x}", Sha256::digest(id.as_bytes()));
        let folder = format!("example-{}", &digest[..20]);
        let note = root.join("00-inbox").join(&folder).join(format!("{folder}.md"));
        fs::create_dir_all(note.parent().unwrap()).unwrap(); fs::write(&note, b"fictional").unwrap();
        let path = note.to_string_lossy().into_owned();
        let receipt = json!({"ok":true,"path":path,"note_path":path,"kind":"note","reused":false});
        let reply = |value: &Value| json!({"ok":true,"result":{"exitCode":0,"stdout":value.to_string()}});
        let capture = Capture::empty();
        assert!(valid_capture_receipt(&command, id, &reply(&receipt), Some(&capture)));
        assert!(!valid_capture_receipt(&command, "aaaaaaaa-1234-1234-1234-123456789abc", &reply(&receipt), Some(&capture)));
        for invalid in [json!({"ok":true}), json!({"ok":true,"path":"/outside","note_path":"/outside","kind":"note","reused":false}), json!({"ok":true,"path":path,"note_path":path,"kind":"task","reused":false}), json!({"ok":true,"path":path,"note_path":path,"kind":"note","reused":"false"})] {
            assert!(!valid_capture_receipt(&command, id, &reply(&invalid), Some(&capture)));
        }
        let mut truncated = capture.clone(); truncated.stdout.truncated = true;
        assert!(!valid_capture_receipt(&command, id, &reply(&receipt), Some(&truncated)));
        assert!(!valid_capture_receipt(&command, id, &reply(&receipt), None));
        fs::remove_file(&note).unwrap(); fs::create_dir(&note).unwrap();
        assert!(!valid_capture_receipt(&command, id, &reply(&receipt), Some(&capture)));
        fs::remove_dir_all(root).unwrap();
    }
    fn result(id: &str, path: &str, ok: bool) -> Value {
        let body = json!({"ok":ok,"error":"failure","detail":"failure","failureContext":{"schema":SCHEMA,"scope":"work","invocationID":id,"relativePath":path}});
        json!({"ok":true,"result":{"stdout":body.to_string()}})
    }
    #[test]
    fn local_context_is_bound_bounded_and_removed_from_public_output() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let capture = Capture::empty();
        let mut value = result(id, "00-inbox/demo/één.md", false);
        assert_eq!(
            take(&mut value, Some(&capture), id).flatten().as_deref(),
            Some("00-inbox/demo/één.md")
        );
        assert!(!value.to_string().contains("failureContext"));
        for invalid in [
            "/absolute.md",
            "a/../b.md",
            "a//b.md",
            "a/./b.md",
            "a\\b.md",
            "a\nb.md",
            "a.txt",
        ] {
            let mut value = result(id, invalid, false);
            assert_eq!(take(&mut value, Some(&capture), id), Some(None));
            assert!(!value.to_string().contains("failureContext"));
        }
        let mut wrong = result(id, "a.md", false);
        assert_eq!(take(&mut wrong, Some(&capture), "wrong"), None);
        let mut success = result(id, "a.md", true);
        assert_eq!(take(&mut success, Some(&capture), id), None);
        let mut incomplete = Capture::empty();
        incomplete.stdout.truncated = true;
        let mut value = result(id, "private.md", false);
        assert_eq!(take(&mut value, Some(&incomplete), id), None);
        assert!(!value.to_string().contains("private.md"));
    }
    #[test]
    fn only_an_owned_literal_work_router_gets_a_diagnostic_id() {
        let home = std::env::temp_dir().join(format!("ruycast-context-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".local/libexec")).unwrap();
        let home = home.canonicalize().unwrap();
        let target = home.join(".local/libexec/viggo-work-writer");
        std::fs::write(&target, b"fixture").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        let original: Command = serde_json::from_value(json!({"id":"capture.fixture","title":"Fixture","kind":"script","target":target,"arguments":["task-active","from-selection","{uuid}",""]})).unwrap();
        let mut prepared = original.clone();
        prepared.arguments[2] = id.into();
        assert_eq!(invocation(&original, &prepared, &home).as_deref(), Some(id));
        let mut forged = original.clone();
        forged.arguments[1] = "other".into();
        assert!(invocation(&forged, &prepared, &home).is_none());
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o722)).unwrap();
        assert!(invocation(&original, &prepared, &home).is_none());
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn capture_and_ordinary_exit_failures_remove_locator_before_interpretation() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let path = "00-inbox/demo/note.md";
        let capture = Capture::empty();
        for (command_id, exit_ok) in [
            ("capture.task-from-note", false),
            ("capture.convert-note", true),
            ("task.status.todo", false),
        ] {
            let command: Command = serde_json::from_value(
                json!({"id":command_id,"title":"Fixture","kind":"script","target":"/fixture"}),
            )
            .unwrap();
            let body = json!({"ok":false,"error":format!("failed {path}"),"detail":format!("failed {path}"),"failureContext":{"schema":SCHEMA,"scope":"work","invocationID":id,"relativePath":path}}).to_string();
            let mut raw = if exit_ok {
                json!({"ok":true,"result":{"stdout":body,"exitCode":0}})
            } else {
                json!({"ok":false,"error":"Script exited with status 1.","stdout":body,"exitCode":1})
            };
            assert_eq!(
                take(&mut raw, Some(&capture), id).flatten().as_deref(),
                Some(path)
            );
            let interpreted = crate::script_result::interpret_command(
                &command,
                raw,
                false,
                crate::script_result::Audience::Native,
                Some(&capture),
            );
            assert_eq!(interpreted.response["ok"], false);
            assert!(!interpreted.response.to_string().contains(path));
            assert!(interpreted
                .detail
                .as_deref()
                .is_none_or(|d| !d.contains(path)));
        }
    }
}
