//! Serialized private usage and bounded operational metadata. No stdout,
//! arguments or input values are persisted. Failed operational metadata uses
//! the original bounded, sanitized diagnostic allowlist.
use crate::{private_storage, profile::SaveReceipt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn bytes(path: &Path, limit: usize, restrict: bool) -> Result<Option<Vec<u8>>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let info = file.metadata().map_err(|e| e.to_string())?;
    if !info.is_file()
        || info.uid() != unsafe { libc::getuid() }
        || info.nlink() != 1
        || info.len() > limit as u64
    {
        return Err("Invalid private activity file".into());
    }
    // Restrict the verified open descriptor before reading even malformed data.
    // Never chmod the path: a linked/replaced entry cannot redirect this repair.
    if restrict && unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
        return Err("Could not restrict command log permissions".into());
    }
    let mut out = vec![];
    file.take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() > limit {
        return Err("Activity file is too large".into());
    }
    Ok(Some(out))
}
fn check_log_directory(root: &Path) -> Result<(), String> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .map_err(|e| e.to_string())?;
    let info = directory.metadata().map_err(|e| e.to_string())?;
    if !info.is_dir() || info.uid() != unsafe { libc::getuid() } || info.mode() & 0o077 != 0 {
        return Err("Private profile required".into());
    }
    Ok(())
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    date: String,
    #[serde(rename = "traceID")]
    trace_id: String,
    #[serde(rename = "commandID")]
    command_id: String,
    source: String,
    phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    success: Option<bool>,
    #[serde(rename = "durationMS", skip_serializing_if = "Option::is_none")]
    duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<HashMap<String, String>>,
}

pub struct Activity {
    root: PathBuf,
    pub counts: HashMap<String, i64>,
    entries: Vec<Value>,
    writable: bool,
    pub storage_error: Option<String>,
    pub usage_revision: u64,
}
impl Activity {
    fn clean(value: &str, secrets: &[String], home: &Path, limit: usize) -> String {
        use objc2_foundation::{NSCharacterSet, NSRange, NSString, NSStringCompareOptions};
        use unicode_segmentation::UnicodeSegmentation;
        objc2::rc::autoreleasepool(|_| {
            let mut text = value.graphemes(true).take(16384).collect::<String>();
            let mut secrets = secrets.iter().filter(|s| !s.is_empty()).collect::<Vec<_>>();
            secrets.sort_by_key(|s| std::cmp::Reverse(s.graphemes(true).count()));
            for secret in secrets {
                text = text.replace(secret, "[redacted]");
            }
            if let Some(home) = home.to_str().filter(|s| !s.is_empty()) {
                text = text.replace(home, "~");
            }
            for (pattern, replacement) in [
                (r#"(?i)\b(?:https?|obsidian)://[^\s\"<>]+"#, "[url]"),
                (r#"(?i)\bBearer\s+[^\s\",;]+"#, "Bearer [redacted]"),
                (
                    r#"(?i)\b(password|passwd|token|secret|api[_-]?key|authorization)\b[\"']?\s*[:=]\s*[\"']?[^\s\"',;]+"#,
                    "$1=[redacted]",
                ),
            ] {
                let native = NSString::from_str(&text);
                text = native
                    .stringByReplacingOccurrencesOfString_withString_options_range(
                        &NSString::from_str(pattern),
                        &NSString::from_str(replacement),
                        NSStringCompareOptions::RegularExpressionSearch,
                        NSRange::new(0, native.length()),
                    )
                    .to_string();
            }
            let controls = NSCharacterSet::controlCharacterSet();
            let text = text
                .chars()
                .filter(|c| *c == '\n' || *c == '\t' || !controls.longCharacterIsMember(*c as u32))
                .collect::<String>();
            let mut remaining = limit;
            text.graphemes(true)
                .take_while(|s| {
                    if s.len() > remaining {
                        false
                    } else {
                        remaining -= s.len();
                        true
                    }
                })
                .collect()
        })
    }
    pub fn open(root: &Path) -> Result<Self, String> {
        let counts = Self::read_counts(root)?;
        let load = || -> Result<Vec<Value>, String> {
            check_log_directory(root)?;
            let Some(data) = bytes(&root.join("command-log.json"), 2_000_000, true)? else {
                return Ok(vec![]);
            };
            let document: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            let entries = document["entries"]
                .as_array()
                .ok_or("Invalid command log")?;
            if document["schema"] != 1 || entries.len() > 500 {
                return Err("Invalid command log".into());
            }
            entries
                .iter()
                .map(|entry| {
                    let entry: LogEntry =
                        serde_json::from_value(entry.clone()).map_err(|_| "Invalid command log")?;
                    if entry.command_id.len() > 512
                        || chrono::DateTime::parse_from_rfc3339(&entry.date).is_err()
                        || entry.message.as_ref().is_some_and(|s| s.len() > 4096)
                        || entry.details.as_ref().is_some_and(|d| d.len() > 6)
                    {
                        return Err("Invalid command log".into());
                    }
                    serde_json::to_value(entry).map_err(|e| e.to_string())
                })
                .collect()
        };
        let (entries, writable, storage_error) = match load() {
            Ok(e) => (e, true, None),
            Err(_) => (
                vec![],
                false,
                Some(
                    "Command log could not be read. Use ruycast logs clear --confirm to reset it."
                        .into(),
                ),
            ),
        };
        let mut activity = Self {
            root: root.to_path_buf(),
            counts,
            entries,
            writable,
            storage_error,
            usage_revision: 0,
        };
        activity.prune();
        Ok(activity)
    }
    fn read_counts(root: &Path) -> Result<HashMap<String, i64>, String> {
        match bytes(&root.join("usage.json"), 1_000_000, false)? {
            Some(b) => serde_json::from_slice(&b).map_err(|_| "Invalid usage data".into()),
            None => Ok(HashMap::new()),
        }
    }
    pub fn success(&mut self, id: &str) -> Result<(), String> {
        let mut counts = Self::read_counts(&self.root)?;
        let count = counts
            .get(id)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Usage count overflow")?;
        counts.insert(id.into(), count);
        let data = serde_json::to_vec_pretty(&counts).map_err(|e| e.to_string())?;
        if data.len() > 1_000_000 {
            return Err("Usage data exceeds the supported size".into());
        }
        let receipt = SaveReceipt::from_outcome(private_storage::write_outcome(
            &self.root.join("usage.json"),
            &data,
            true,
        )?);
        self.counts = counts;
        self.usage_revision = self.usage_revision.wrapping_add(1);
        if let Some(warning) = receipt.warning {
            return Err(warning);
        }
        Ok(())
    }
    pub fn log_id(id: &str, values: &Value, home: &Path) -> String {
        let mut id = id.chars().take(16384).collect::<String>();
        let mut secrets = values
            .as_object()
            .into_iter()
            .flat_map(|v| v.values())
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for secret in secrets {
            id = id.replace(secret, "[redacted]");
        }
        if let Some(home) = home.to_str() {
            if !home.is_empty() {
                id = id.replace(home, "~");
            }
        }
        let lower = id.to_lowercase();
        let compact = lower
            .chars()
            .filter(|c| !c.is_whitespace() && !['\"', '\''].contains(c))
            .collect::<String>();
        if id.contains("://")
            || lower.split_whitespace().any(|w| w == "bearer")
            || [
                "password",
                "passwd",
                "token",
                "secret",
                "api_key",
                "api-key",
                "apikey",
                "authorization",
            ]
            .iter()
            .any(|key| compact.contains(&format!("{key}=")) || compact.contains(&format!("{key}:")))
        {
            id = "[redacted command]".into();
        }
        let mut bytes = 0;
        id.chars()
            .filter(|c| !c.is_control())
            .take_while(|c| {
                bytes += c.len_utf8();
                bytes <= 512
            })
            .collect()
    }
    pub fn record(
        &mut self,
        trace: &str,
        id: &str,
        source: &str,
        finish: Option<(bool, f64, Option<i64>)>,
    ) {
        self.record_entry(trace, id, source, finish, None);
    }
    pub fn record_response(
        &mut self,
        trace: &str,
        id: &str,
        source: &str,
        ms: f64,
        response: &Value,
        redaction: (&[String], &Path),
    ) {
        let (secrets, home) = redaction;
        let ok = response["ok"] == true;
        let error = response["error"].as_str().unwrap_or("Action failed");
        let mut metadata = json!({"code": Self::clean(response["code"].as_str().unwrap_or_else(|| if ok { "success" } else { crate::analytics::failure_code(error) }), secrets, home, 80)});
        if !ok {
            metadata["message"] = json!(Self::clean(error, secrets, home, 1800));
        }
        if let Some(applied) = response["applied"]
            .as_bool()
            .or_else(|| response["actionCompleted"].as_bool())
        {
            metadata["applied"] = json!(applied);
        }
        let mut details = serde_json::Map::new();
        if let Some(raw) = response["diagnostics"].as_object() {
            for key in [
                "provider",
                "providerExitCode",
                "providerStderr",
                "exception",
                "runnerError",
            ] {
                if let Some(value) = raw.get(key) {
                    let text = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    details.insert(key.into(), json!(Self::clean(&text, secrets, home, 1000)));
                }
            }
        }
        if !details.is_empty() {
            metadata["details"] = Value::Object(details);
        }
        self.record_entry(
            trace,
            id,
            source,
            Some((ok, ms, response["exitCode"].as_i64())),
            Some(metadata),
        );
    }
    fn record_entry(
        &mut self,
        trace: &str,
        id: &str,
        source: &str,
        finish: Option<(bool, f64, Option<i64>)>,
        metadata: Option<Value>,
    ) {
        let timestamp = now();
        let date = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
            .expect("current timestamp")
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut entry = json!({"date":date,"traceID":trace,"commandID":id,"source":source,"phase":if finish.is_some(){"command.finish"}else{"command.start"}});
        if let Some((ok, ms, exit)) = finish {
            entry["success"] = json!(ok);
            entry["durationMS"] = json!(ms);
            entry["code"] = json!(if ok { "success" } else { "execution-failed" });
            if let Some(exit) = exit {
                entry["exitStatus"] = json!(exit);
            }
        }
        if let Some(metadata) = metadata {
            entry.as_object_mut().expect("log entry object").extend(
                metadata
                    .as_object()
                    .expect("bounded metadata object")
                    .clone(),
            );
        }
        self.entries.push(entry);
        self.prune();
        self.save();
    }
    fn prune(&mut self) {
        let cutoff = now() - 7. * 86400.;
        self.entries.retain(|e| {
            e["date"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|d| d.timestamp() as f64 >= cutoff)
        });
        if self.entries.len() > 500 {
            self.entries.drain(..self.entries.len() - 500);
        }
        let mut cost = self
            .entries
            .iter()
            .map(|e| e.to_string().len() + 512)
            .sum::<usize>();
        while cost > 1_500_000 && self.entries.len() > 1 {
            cost -= self.entries.remove(0).to_string().len() + 512;
        }
    }
    fn save(&mut self) {
        if !self.writable {
            return;
        }
        let result = (|| -> Result<(), String> {
            check_log_directory(&self.root)?;
            let data = serde_json::to_vec_pretty(&json!({"schema":1,"entries":self.entries}))
                .map_err(|e| e.to_string())?;
            let receipt = SaveReceipt::from_outcome(private_storage::write_outcome(
                &self.root.join("command-log.json"),
                &data,
                true,
            )?);
            receipt.warning.map_or(Ok(()), Err)
        })();
        self.storage_error = result.err().map(|_| {
            "Command log could not be saved. Check disk space and profile permissions.".into()
        });
    }
    pub fn path(&self) -> PathBuf {
        self.root.join("command-log.json")
    }
    pub fn entries(&self, request: &Value) -> Vec<Value> {
        self.entries
            .iter()
            .rev()
            .filter(|e| {
                request["commandID"]
                    .as_str()
                    .is_none_or(|id| e["commandID"] == id)
                    && (request["errorsOnly"] != true || e["success"] == false)
            })
            .take(request["limit"].as_i64().unwrap_or(50).clamp(1, 500) as usize)
            .cloned()
            .collect()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.writable = true;
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        let path = std::env::temp_dir().join(format!(
            "ruycast-activity-{}-{}",
            std::process::id(),
            now().to_bits()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Temp(path)
    }
    fn entry() -> Value {
        json!({"date":chrono::DateTime::<chrono::Utc>::from_timestamp(now() as i64,0).unwrap().to_rfc3339(),
            "traceID":"trace","commandID":"fixture","source":"cli","phase":"command.finish","success":true})
    }
    #[test]
    fn complete_metadata_budget_survives_save_reopen_and_retains_newest_entries() {
        let dir = temp();
        let mut activity = Activity::open(&dir.0).unwrap();
        let response = json!({"ok":false,"code":"provider_failed","error":"m".repeat(1800),"diagnostics":{"provider":"a".repeat(1000),"providerExitCode":"b".repeat(1000),"providerStderr":"c".repeat(1000),"exception":"d".repeat(1000),"runnerError":"e".repeat(1000)}});
        activity.record_response("prototype", "fixture", "cli", 1., &response, (&[], &dir.0));
        let prototype = activity.entries.last().unwrap().clone();
        activity.entries = (0..500).map(|index| {
            let mut entry = prototype.clone();
            entry["traceID"] = json!(index.to_string());
            entry
        }).collect();
        activity.prune();
        activity.save();
        let mut loaded = Activity::open(&dir.0).unwrap();
        assert!(loaded.storage_error.is_none());
        assert!(loaded.writable);
        let entries = loaded.entries(&json!({"commandID":"fixture","errorsOnly":true,"limit":500}));
        assert!(fs::metadata(activity.path()).unwrap().len() <= 1_500_000);
        assert!(entries.len()>1 && entries.len()<500);
        assert_eq!(entries[0]["traceID"], "499");
        assert!(fs::metadata(activity.path()).unwrap().len() <= 2_000_000);
        loaded.record_response("after-reopen", "fixture", "cli", 1., &response, (&[], &dir.0));
        assert!(loaded.storage_error.is_none());
        assert_eq!(Activity::open(&dir.0).unwrap().entries(&json!({}))[0]["traceID"], "after-reopen");
    }
    #[test]
    fn response_metadata_redacts_inputs_credentials_urls_home_and_unlisted_details() {
        let dir = temp();
        let mut activity = Activity::open(&dir.0).unwrap();
        let home = Path::new("/owned/private-home");
        let secrets = vec!["literal input Ω".to_owned()];
        let response = json!({"ok":false,"code":"provider_failed","exitCode":23,"actionCompleted":true,
            "error":"Status writer rejected literal input Ω at /owned/private-home token=sample-secret-value Bearer bearer-value https://example.invalid/private obsidian://private; password:'password-value'\u{0001}",
            "diagnostics":{"provider":"task-status.sh","providerExitCode":23,"providerStderr":"literal input Ω token=sample-secret-value","exception":"exception owned","runnerError":"/owned/private-home/runner","stdout":"UNLISTED_PRIVATE_PAYLOAD"},
            "stdout":"UNLISTED_PRIVATE_PAYLOAD"});
        activity.record_response("trace", "fixture", "cli", 1., &response, (&secrets, home));
        let entry = &activity.entries(&json!({"errorsOnly":true}))[0];
        assert_eq!(entry["code"], "provider_failed");
        assert_eq!(entry["exitStatus"], 23);
        assert_eq!(entry["applied"], true);
        assert_eq!(entry["details"].as_object().unwrap().len(), 5);
        assert_eq!(entry["details"]["providerExitCode"], "23");
        assert!(entry["message"]
            .as_str()
            .unwrap()
            .contains("Status writer rejected [redacted]"));
        let persisted = fs::read_to_string(activity.path()).unwrap();
        for secret in [
            "literal input Ω",
            "private-home",
            "sample-secret-value",
            "bearer-value",
            "example.invalid",
            "obsidian://",
            "password-value",
            "UNLISTED_PRIVATE_PAYLOAD",
        ] {
            assert!(!persisted.contains(secret), "{secret}");
        }
        assert!(!entry["message"].as_str().unwrap().contains('\u{0001}'));
    }
    #[test]
    fn response_metadata_respects_utf8_limits_and_success_never_records_stdout_or_error() {
        let dir = temp();
        assert_eq!(Activity::clean(&"e\u{301}".repeat(100), &[], &dir.0, 80), "e\u{301}".repeat(26));
        let mut activity = Activity::open(&dir.0).unwrap();
        let response = json!({"ok":false,"code":"🙂".repeat(100),"error":"🙂".repeat(1000),"diagnostics":{"exception":"Ω".repeat(1000)}});
        activity.record_response("trace", "fixture", "cli", 1., &response, (&[], &dir.0));
        let entry = &activity.entries(&json!({}))[0];
        assert_eq!(entry["code"].as_str().unwrap().len(), 80);
        assert_eq!(entry["message"].as_str().unwrap().len(), 1800);
        assert_eq!(entry["details"]["exception"].as_str().unwrap().len(), 1000);
        activity.record_response("success", "fixture", "cli", 1., &json!({"ok":true,"error":"PRIVATE_SUCCESS_ERROR","stdout":"PRIVATE_SUCCESS_OUTPUT","applied":false,"actionCompleted":true}), (&[], &dir.0));
        let loaded = Activity::open(&dir.0).unwrap();
        let entry = &loaded.entries(&json!({}))[0];
        assert_eq!(entry["code"], "success");
        assert_eq!(entry["applied"], false);
        assert!(entry.get("message").is_none());
        let persisted = fs::read_to_string(activity.path()).unwrap();
        assert!(!persisted.contains("PRIVATE_SUCCESS_"));
    }
    #[test]
    fn credential_patterns_input_and_home_are_redacted() {
        for id in [
            "Bearer abc123",
            "token:abc123",
            "passwd=abc123",
            "api-key = \"abc123\"",
            "secret='abc123'",
            "authorization:abc123",
            "https://example.invalid/abc123",
        ] {
            assert!(
                !Activity::log_id(id, &json!({}), Path::new("/Users/test")).contains("abc123"),
                "{id}"
            );
        }
        assert_eq!(
            Activity::log_id(
                "/Users/test/fixture-secret",
                &json!({"input":"secret"}),
                Path::new("/Users/test")
            ),
            "~/fixture-[redacted]"
        );
    }
    #[test]
    fn load_prunes_retention_and_whitelists_unknown_payloads() {
        let dir = temp();
        let mut old = entry();
        old["date"] = json!("2020-01-01T00:00:00Z");
        let mut recent = entry();
        recent["stdout"] = json!("fixture secret payload");
        recent["arguments"] = json!(["fixture secret payload"]);
        fs::write(
            dir.0.join("command-log.json"),
            serde_json::to_vec(&json!({"schema":1,"entries":[old,recent]})).unwrap(),
        )
        .unwrap();
        let mut activity = Activity::open(&dir.0).unwrap();
        let entries = activity.entries(&json!({}));
        assert_eq!(entries.len(), 1);
        assert!(entries[0].get("stdout").is_none());
        activity.record("new", "fixture", "cli", None);
        let bytes = fs::read_to_string(dir.0.join("command-log.json")).unwrap();
        assert!(!bytes.contains("fixture secret payload"));
    }
    #[test]
    fn malformed_optional_types_preserve_existing_log_until_clear() {
        for invalid in [
            json!({"success":"true"}),
            json!({"details":{"key":["not a string"]}}),
        ] {
            let dir = temp();
            let mut value = entry();
            value
                .as_object_mut()
                .unwrap()
                .extend(invalid.as_object().unwrap().clone());
            let bytes = serde_json::to_vec(&json!({"schema":1,"entries":[value]})).unwrap();
            fs::write(dir.0.join("command-log.json"), &bytes).unwrap();
            let mut activity = Activity::open(&dir.0).unwrap();
            assert!(activity.storage_error.is_some());
            activity.record("new", "fixture", "cli", None);
            assert_eq!(fs::read(dir.0.join("command-log.json")).unwrap(), bytes);
            activity.clear();
            assert!(activity.storage_error.is_none());
        }
    }
    #[test]
    fn existing_log_is_restricted_before_decoding_and_unqualified_files_stay_untouched() {
        let dir = temp();
        let path = dir.0.join("command-log.json");
        let original = b"broken diagnostic file";
        fs::write(&path, original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let activity = Activity::open(&dir.0).unwrap();
        assert!(activity.storage_error.is_some());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read(&path).unwrap(), original);

        let linked = temp();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&path, linked.0.join("command-log.json")).unwrap();
        let mut refused = Activity::open(&linked.0).unwrap();
        assert!(refused.storage_error.is_some());
        refused.record("own", "fixture", "cli", None);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
        assert_eq!(fs::read(&path).unwrap(), original);

        let hardlinked = temp();
        fs::hard_link(&path, hardlinked.0.join("command-log.json")).unwrap();
        let refused = Activity::open(&hardlinked.0).unwrap();
        assert!(refused.storage_error.is_some());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn original_log_bounds_filters_reload_and_clear_contract() {
        let dir = temp();
        let mut activity = Activity::open(&dir.0).unwrap();
        for index in 0..600 {
            activity.record(
                &index.to_string(),
                "fixture",
                "cli",
                Some((index % 2 == 0, 1., None)),
            );
        }
        let path = activity.path();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut loaded = Activity::open(&dir.0).unwrap();
        assert!(loaded.storage_error.is_none());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let entries = loaded.entries(&json!({"limit":500}));
        assert_eq!(entries.len(), 500);
        assert_eq!(entries[0]["traceID"], "599");
        assert_eq!(entries[499]["traceID"], "100");
        assert_eq!(
            loaded
                .entries(&json!({"commandID":"fixture","errorsOnly":true,"limit":3}))
                .iter()
                .map(|e| e["traceID"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["599", "597", "595"]
        );
        assert!(loaded
            .entries(&json!({"commandID":"other","limit":500}))
            .is_empty());
        let cleaned = Activity::log_id(&"🙂".repeat(1000), &json!({}), &dir.0);
        assert_eq!(cleaned.len(), 512);
        assert!(cleaned.chars().all(|c| c == '🙂'));
        loaded.clear();
        assert!(loaded.storage_error.is_none());
        assert!(Activity::open(&dir.0)
            .unwrap()
            .entries(&json!({"limit":500}))
            .is_empty());
    }
    #[test]
    fn insecure_or_linked_profile_refuses_log_read_and_never_repairs_its_mode() {
        let dir = temp();
        let path = dir.0.join("command-log.json");
        let original = serde_json::to_vec(&json!({"schema":1,"entries":[entry()]})).unwrap();
        fs::write(&path, &original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        let mut refused = Activity::open(&dir.0).unwrap();
        assert!(refused.storage_error.is_some());
        assert!(refused.entries(&json!({})).is_empty());
        refused.record("own", "fixture", "cli", None);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
        assert_eq!(fs::metadata(&dir.0).unwrap().mode() & 0o777, 0o755);
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
        let link_owner = temp();
        let alias = link_owner.0.join("profile-alias");
        std::os::unix::fs::symlink(&dir.0, &alias).unwrap();
        let mut refused = Activity::open(&alias).unwrap();
        assert!(refused.storage_error.is_some());
        assert!(refused.entries(&json!({})).is_empty());
        refused.record("own", "fixture", "cli", None);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
    }
}
