//! Optional, bounded diagnostics from the admitted Work task router.
use rustcast_core::{script::Capture, Command};
use serde_json::Value;
use std::{fs, os::unix::fs::MetadataExt, path::Path};

pub(crate) const ENV: &str = "VIGGOBSIDIAN_DIAGNOSTIC_ID";
const SCHEMA: &str = "ruycast.work-failure-context.v1";

pub(crate) fn invocation(original: &Command, prepared: &Command, home: &Path) -> Option<String> {
    let target = original
        .target
        .strip_prefix("~/")
        .map(|p| home.join(p))
        .unwrap_or_else(|| original.target.as_str().into());
    if original.kind != "script" || target != home.join(".local/libexec/viggo-work-writer") {
        return None;
    }
    let info = fs::symlink_metadata(&target).ok()?;
    if fs::canonicalize(&target).ok()? != target {
        return None;
    }
    if !info.is_file()
        || info.file_type().is_symlink()
        || info.uid() != unsafe { libc::getuid() }
        || info.mode() & 0o6022 != 0
        || info.mode() & 0o111 == 0
    {
        return None;
    }
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
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    {
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
