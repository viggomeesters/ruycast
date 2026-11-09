//! Existing analytics.json schema and privacy contract. Mutations share
//! one lock; one coalescing writer owns disk I/O and the profile lease.
use objc2::{rc::autoreleasepool, AllocAnyThread};
use objc2_foundation::{
    NSCalendar, NSCalendarOptions, NSCalendarUnit, NSDate, NSISO8601DateFormatOptions,
    NSISO8601DateFormatter, NSString, NSTimeZone,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

const MAX_FILE: u64 = 8_000_000;
const READ_ERROR: &str = "Analytics data could not be read. Reset analytics to start again.";
const SAVE_ERROR: &str =
    "Analytics could not be saved. Check available disk space and folder permissions.";
pub const TIMINGS: &[&str] = &[
    "search.model",
    "search.clipboard",
    "launcher.open.model",
    "process.query",
    "clipboard.capture",
    "clipboard.preview",
    "clipboard.save",
    "config.read",
    "config.reload",
    "command.execute",
    "cli.dispatch",
    "monitor.sample",
    "startup.model",
    "process.sort",
    "icons.lookup",
];
const CATEGORIES: &[&str] = &["command", "config", "clipboard", "keyboard", "runtime"];
const SOURCES: &[&str] = &["launcher", "hotkey", "cli", "snippet"];

mod date {
    use super::*;
    pub fn serialize<S: serde::Serializer>(n: &f64, s: S) -> Result<S::Ok, S::Error> {
        if !n.is_finite() {
            return Err(serde::ser::Error::custom("Invalid date"));
        }
        let text = autoreleasepool(|_| {
            NSISO8601DateFormatter::new()
                .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(*n))
                .to_string()
        });
        s.serialize_str(&text)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        let text = String::deserialize(d)?;
        autoreleasepool(|_| {
            let formatter = NSISO8601DateFormatter::new();
            let text = NSString::from_str(&text);
            formatter
                .dateFromString(&text)
                .or_else(|| {
                    formatter.setFormatOptions(
                        NSISO8601DateFormatOptions::WithInternetDateTime
                            | NSISO8601DateFormatOptions::WithFractionalSeconds,
                    );
                    formatter.dateFromString(&text)
                })
                .map(|d| d.timeIntervalSince1970())
        })
        .filter(|v| v.is_finite())
        .ok_or_else(|| serde::de::Error::custom("Invalid analytics date"))
    }
}
// Scoped Codable Int conversion. Do not enable serde_json raw_value or
// arbitrary_precision: their magic-map handling also changes general Value.
fn stored_integer(value: f64) -> Option<i64> {
    (value.is_finite()
        && value.fract() == 0.
        && value >= i64::MIN as f64
        && value < 9_223_372_036_854_775_808.)
        .then_some(value as i64)
}
fn integer<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an analytics integer")
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(E::custom)
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<i64, E> {
            stored_integer(v).ok_or_else(|| E::custom("Invalid analytics integer"))
        }
    }
    d.deserialize_any(Visitor)
}
struct IntegerValue(i64);
impl<'de> Deserialize<'de> for IntegerValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        integer(d).map(Self)
    }
}
fn optional_integer<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(Option::<IntegerValue>::deserialize(d)?.map(|v| v.0))
}
fn source_counts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, i64>, D::Error> {
    Ok(BTreeMap::<String, IntegerValue>::deserialize(d)?
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect())
}

mod optional_date {
    use super::*;
    pub fn serialize<S: serde::Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(n) => date::serialize(n, s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
        #[derive(Deserialize)]
        struct Wrapped(#[serde(with = "date")] f64);
        Ok(Option::<Wrapped>::deserialize(d)?.map(|v| v.0))
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    #[serde(deserialize_with = "integer")]
    pub count: i64,
    #[serde(rename = "totalMS")]
    pub total_ms: f64,
    #[serde(rename = "maxMS")]
    pub max_ms: f64,
    pub samples: Vec<f64>,
}
impl Timing {
    fn add(&mut self, ms: f64, limit: usize) {
        if !ms.is_finite() || ms < 0. {
            return;
        }
        self.count = self.count.saturating_add(1);
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        self.samples.push(ms);
        if self.samples.len() > limit {
            self.samples.drain(..self.samples.len() - limit);
        }
    }
    fn valid(&self) -> bool {
        count(self.count)
            && finite(self.total_ms)
            && finite(self.max_ms)
            && self.samples.iter().all(|v| finite(*v))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandActivity {
    #[serde(deserialize_with = "integer")]
    pub attempts: i64,
    #[serde(deserialize_with = "integer")]
    pub successes: i64,
    #[serde(deserialize_with = "integer")]
    pub failures: i64,
    #[serde(deserialize_with = "integer")]
    pub prior_successes: i64,
    #[serde(with = "date")]
    pub last_used: f64,
    #[serde(deserialize_with = "source_counts")]
    pub sources: BTreeMap<String, i64>,
    pub timing: Timing,
}
impl Default for CommandActivity {
    fn default() -> Self {
        Self {
            attempts: 0,
            successes: 0,
            failures: 0,
            prior_successes: 0,
            last_used: autoreleasepool(|_| NSDate::distantPast().timeIntervalSince1970()),
            sources: BTreeMap::new(),
            timing: Timing::default(),
        }
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Day {
    #[serde(deserialize_with = "integer")]
    pub uses: i64,
    #[serde(deserialize_with = "integer")]
    pub successes: i64,
    #[serde(deserialize_with = "integer")]
    pub failures: i64,
}
impl Day {
    fn valid(&self) -> bool {
        [self.uses, self.successes, self.failures]
            .into_iter()
            .all(count)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub id: String,
    #[serde(with = "date")]
    pub date: f64,
    pub category: String,
    #[serde(rename = "commandID", skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    pub code: String,
    #[serde(
        default,
        deserialize_with = "optional_integer",
        skip_serializing_if = "Option::is_none"
    )]
    pub exit_status: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    #[serde(deserialize_with = "integer")]
    pub schema: i64,
    pub enabled: bool,
    #[serde(deserialize_with = "integer")]
    pub retention_days: i64,
    #[serde(with = "date")]
    pub started_at: f64,
    #[serde(with = "date")]
    pub updated_at: f64,
    pub legacy_imported: bool,
    pub commands: BTreeMap<String, CommandActivity>,
    pub timings: BTreeMap<String, Timing>,
    pub events: Vec<Diagnostic>,
    pub days: BTreeMap<String, Day>,
    #[serde(default, deserialize_with = "nullable_hours")]
    pub hours: BTreeMap<String, Day>,
    #[serde(
        default,
        with = "optional_date",
        skip_serializing_if = "Option::is_none"
    )]
    pub hourly_started_at: Option<f64>,
    #[serde(deserialize_with = "integer")]
    pub total_attempts: i64,
    #[serde(deserialize_with = "integer")]
    pub total_successes: i64,
    #[serde(deserialize_with = "integer")]
    pub total_failures: i64,
    #[serde(deserialize_with = "integer")]
    pub prior_successes: i64,
    #[serde(deserialize_with = "integer")]
    pub omitted_commands: i64,
}
impl Document {
    pub fn new(now: f64) -> Self {
        Self {
            schema: 2,
            enabled: true,
            retention_days: 30,
            started_at: now,
            updated_at: now,
            legacy_imported: false,
            commands: BTreeMap::new(),
            timings: BTreeMap::new(),
            events: vec![],
            days: BTreeMap::new(),
            hours: BTreeMap::new(),
            hourly_started_at: Some(now),
            total_attempts: 0,
            total_successes: 0,
            total_failures: 0,
            prior_successes: 0,
            omitted_commands: 0,
        }
    }
    pub fn valid(&self) -> bool {
        [1, 2].contains(&self.schema)
            && (self.schema == 1 || self.hourly_started_at.is_some())
            && (1..=90).contains(&self.retention_days)
            && self.commands.len() <= 2048
            && self.timings.len() <= TIMINGS.len()
            && self.events.len() <= 200
            && self.days.len() <= 91
            && self.hours.len() <= 2161
            && [
                self.total_attempts,
                self.total_successes,
                self.total_failures,
                self.prior_successes,
                self.omitted_commands,
            ]
            .into_iter()
            .all(count)
            && self
                .timings
                .iter()
                .all(|(k, v)| TIMINGS.contains(&k.as_str()) && v.samples.len() <= 256 && v.valid())
            && self.commands.iter().all(|(k, v)| {
                valid_key(k)
                    && [v.attempts, v.successes, v.failures, v.prior_successes]
                        .into_iter()
                        .all(count)
                    && v.timing.samples.len() <= 32
                    && v.timing.valid()
                    && v.sources
                        .iter()
                        .all(|(k, v)| SOURCES.contains(&k.as_str()) && count(*v))
            })
            && self.events.iter().all(|e| {
                CATEGORIES.contains(&e.category.as_str())
                    && safe_code(&e.code) == e.code
                    && e.command_id.as_ref().is_none_or(|k| valid_key(k))
                    && e.exit_status.is_none_or(|v| (-1..=255).contains(&v))
            })
            && self
                .days
                .iter()
                .all(|(k, v)| k.graphemes(true).count() == 10 && v.valid())
            && self.hours.iter().all(|(k, v)| {
                k.parse::<i64>().is_ok_and(|s| {
                    (0..=253402300800).contains(&s) && s % 3600 == 0 && s.to_string() == *k
                }) && v.valid()
            })
            && self.hourly_started_at.is_none_or(f64::is_finite)
    }
    pub fn export(&self, version: &str) -> Value {
        let mut errors = BTreeMap::<String, i64>::new();
        for e in &self.events {
            *errors
                .entry(format!("{}:{}", e.category, e.code))
                .or_default() += 1;
        }
        json!({"schema":1,"version":version,"uses":self.total_attempts.saturating_add(self.prior_successes),"attempts":self.total_attempts,"successes":self.total_successes,"failures":self.total_failures,"legacySuccesses":self.prior_successes,"timings":self.timings,"errors":errors,"privacy":"No command IDs, input, queries, URLs, arguments or clipboard contents"})
    }
    pub fn hourly_activity(&self, now: f64) -> Vec<(f64, Option<Day>)> {
        (0..24)
            .rev()
            .map(|i| {
                let date = hour_start(now) - f64::from(i) * 3600.;
                let recorded = self.hourly_started_at.is_some_and(|s| date + 3600. > s);
                (
                    date,
                    recorded.then(|| self.hours.get(&hour_key(date)).cloned().unwrap_or_default()),
                )
            })
            .collect()
    }
}
fn nullable_hours<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Day>, D::Error> {
    Ok(Option::<BTreeMap<String, Day>>::deserialize(d)?.unwrap_or_default())
}
fn count(n: i64) -> bool {
    (0..=1_000_000_000_000).contains(&n)
}
fn finite(n: f64) -> bool {
    n.is_finite() && n >= 0.
}
fn valid_key(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn command_key(id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ruycast.command:");
    hash.update(id.as_bytes());
    format!("{:x}", hash.finalize())
}
/// Only the analytics raw-request entry uses Foundation NSNumber casts.
/// This is not a configuration/wire-wide coercion rule.
pub fn request_bool(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| match value.as_f64() {
        Some(0.) => Some(false),
        Some(1.) => Some(true),
        _ => None,
    })
}
pub fn request_int(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_bool().map(i64::from))
        .or_else(|| {
            value
                .as_f64()
                .filter(|v| {
                    v.is_finite()
                        && v.fract() == 0.
                        && *v >= i64::MIN as f64
                        && *v < 9_223_372_036_854_775_808.
                })
                .map(|v| v as i64)
        })
}
pub fn safe_code(s: &str) -> &str {
    if [
        "action-failed",
        "permission",
        "timeout",
        "unavailable",
        "invalid",
        "io",
        "config-invalid",
        "clipboard-io",
        "interrupted",
        "script-exit",
    ]
    .contains(&s)
    {
        s
    } else {
        "action-failed"
    }
}
pub fn failure_code(s: &str) -> &'static str {
    let t = s.to_lowercase();
    if ["permission", "accessibility", "not permitted"]
        .iter()
        .any(|s| t.contains(s))
    {
        "permission"
    } else if t.contains("script exited") {
        "script-exit"
    } else if t.contains("timed out") || t.contains("timeout") {
        "timeout"
    } else if ["unavailable", "not found", "missing"]
        .iter()
        .any(|s| t.contains(s))
    {
        "unavailable"
    } else if t.contains("invalid") || t.contains("disabled") {
        "invalid"
    } else {
        "action-failed"
    }
}
fn hour_start(now: f64) -> f64 {
    (now / 3600.).floor() * 3600.
}
fn hour_key(now: f64) -> String {
    (hour_start(now) as i64).to_string()
}

/// Value-only snapshot of Calendar.current at store creation. Native calendar
/// objects are created and dropped on their caller, never passed between threads.
#[derive(Clone)]
pub struct Calendar {
    identifier: String,
    zone: String,
}
impl Default for Calendar {
    fn default() -> Self {
        autoreleasepool(|_| {
            let c = NSCalendar::currentCalendar();
            Self {
                identifier: c.calendarIdentifier().to_string(),
                zone: c.timeZone().name().to_string(),
            }
        })
    }
}
impl Calendar {
    pub(crate) fn native(&self) -> objc2::rc::Retained<NSCalendar> {
        let c = NSCalendar::initWithCalendarIdentifier(
            NSCalendar::alloc(), &NSString::from_str(&self.identifier),
        ).expect("captured calendar identifier");
        c.setTimeZone(&NSTimeZone::timeZoneWithName(&NSString::from_str(&self.zone))
            .expect("captured time zone"));
        c
    }
    #[cfg(feature = "test-tools")]
    pub fn named(identifier: &str, zone: &str) -> Self {
        Self {
            identifier: identifier.into(),
            zone: zone.into(),
        }
    }
    fn keys(&self, now: f64, retention: i64) -> (String, String, f64) {
        autoreleasepool(|_| {
            let c = NSCalendar::initWithCalendarIdentifier(
                NSCalendar::alloc(),
                &NSString::from_str(&self.identifier),
            )
            .expect("captured calendar identifier");
            c.setTimeZone(
                &NSTimeZone::timeZoneWithName(&NSString::from_str(&self.zone))
                    .expect("captured time zone"),
            );
            let date = NSDate::dateWithTimeIntervalSince1970(now);
            let cutoff = c
                .dateByAddingUnit_value_toDate_options(
                    NSCalendarUnit::Day,
                    -retention as isize,
                    &date,
                    NSCalendarOptions::empty(),
                )
                .unwrap_or_else(NSDate::distantPast);
            let key = |d: &NSDate| {
                let p = c.components_fromDate(
                    NSCalendarUnit::Year | NSCalendarUnit::Month | NSCalendarUnit::Day,
                    d,
                );
                format!("{:04}-{:02}-{:02}", p.year(), p.month(), p.day())
            };
            (key(&date), key(&cutoff), cutoff.timeIntervalSince1970())
        })
    }
}
#[derive(Clone)]
pub struct Attempt {
    key: String,
    epoch: u64,
    hour: String,
}
struct State {
    data: Document,
    calendar: Calendar,
    epoch: u64,
    revision: u64,
    saved: u64,
    persist_allowed: bool,
    error: Option<String>,
    due: Option<Instant>,
    stop: bool,
    flush: u64,
    flushed: u64,
    waiters: Vec<(u64, std::sync::mpsc::Sender<()>)>,
    spans: usize,
    done: bool,
    last_hour: Option<(String, i64)>,
}
impl State {
    fn changed(&mut self, now: f64, delay: Duration) {
        self.revision = self.revision.saturating_add(1);
        self.data.updated_at = now;
        self.prune(now);
        if self.persist_allowed && self.due.is_none() {
            self.due = Some(Instant::now() + delay);
        }
    }
    fn prune(&mut self, now: f64) {
        let (_, cutoff, cutoff_date) = self.calendar.keys(now, self.data.retention_days);
        let hour = hour_key(now);
        let retention = self.data.retention_days;

        self.data.events.retain(|e| e.date >= cutoff_date);
        let n = self.data.events.len();
        if n > 200 {
            self.data.events.drain(..n - 200);
        }
        self.data.days.retain(|k, _| k >= &cutoff);
        if self.last_hour.as_ref() != Some(&(hour.clone(), retention)) {
            let first = hour_start(now - retention as f64 * 86400.);
            let last = hour_start(now);
            self.data
                .hours
                .retain(|k, _| k.parse::<f64>().is_ok_and(|s| s >= first && s <= last));
            self.last_hour = Some((hour, retention));
        }
    }
}
struct Shared {
    notify: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    state: Mutex<State>,
    wake: Condvar,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    delay: Duration,
}
#[derive(Clone)]
pub struct Client(Arc<Shared>);
pub struct Writer {
    client: Client,
    worker: Option<JoinHandle<()>>,
}
fn read(path: &Path) -> Result<Option<Document>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::getuid() }
        || m.nlink() != 1
        || m.len() == 0
        || m.len() > MAX_FILE
    {
        return Err(READ_ERROR.into());
    }
    let mut bytes = vec![];
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(READ_ERROR.into());
    }
    let d: Document = serde_json::from_slice(&bytes).map_err(|_| READ_ERROR)?;
    if !d.valid() {
        return Err(READ_ERROR.into());
    }
    Ok(Some(d))
}
impl Writer {
    pub fn open(root: &Path, lease: Option<crate::control::ProfileLease>) -> Result<Self, String> {
        Self::open_with(
            root,
            lease,
            Arc::new(crate::activity::now),
            Calendar::default(),
            Duration::from_secs(3),
        )
    }
    pub fn open_with(
        root: &Path,
        lease: Option<crate::control::ProfileLease>,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
        calendar: Calendar,
        delay: Duration,
    ) -> Result<Self, String> {
        let path = root.join("analytics.json");
        let now = clock();
        let (mut data, persist_allowed, error) = match read(&path) {
            Ok(Some(d)) => (d, true, None),
            Ok(None) => (Document::new(now), true, None),
            Err(_) => (
                Document::new(crate::activity::now()),
                false,
                Some(READ_ERROR.to_owned()),
            ),
        };
        data.schema = 2;
        if data.hourly_started_at.is_none() {
            data.hourly_started_at = Some(now);
        }
        let mut state = State {
            data,
            calendar,
            epoch: 0,
            revision: 0,
            saved: 0,
            persist_allowed,
            error,
            due: None,
            stop: false,
            flush: 0,
            flushed: 0,
            waiters: vec![],
            spans: 0,
            done: false,
            last_hour: None,
        };
        state.prune(now);
        let client = Client(Arc::new(Shared {
            notify: Mutex::new(None),
            state: Mutex::new(state),
            wake: Condvar::new(),
            clock,
            delay,
        }));
        let shared = client.0.clone();
        let worker = thread::Builder::new()
            .name("ruycast-analytics".into())
            .spawn(move || {
                let _lease = lease;
                writer_loop(shared, path)
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            worker: Some(worker),
        })
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        {
            let mut s = self
                .client
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            s.stop = true;
            self.client.0.wake.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn writer_loop(shared: Arc<Shared>, path: PathBuf) {
    loop {
        let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while !(s.stop && s.spans == 0)
            && s.flush == s.flushed
            && s.due.is_none_or(|due| Instant::now() < due)
        {
            s = if let Some(due) = s.due {
                shared
                    .wake
                    .wait_timeout(s, due.saturating_duration_since(Instant::now()))
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            } else {
                shared.wake.wait(s).unwrap_or_else(|e| e.into_inner())
            };
        }
        let flush = s.flush;
        let should_save = s.persist_allowed && s.revision != s.saved;
        let candidate = should_save.then(|| (s.data.clone(), s.revision));
        let attempted_save = candidate.is_some();
        s.due = None;
        drop(s);
        let result = candidate.as_ref().map(|(d, _)| {
            serde_json::to_vec_pretty(d)
                .map_err(|_| ())
                .and_then(|bytes| {
                    if bytes.len() as u64 > MAX_FILE {
                        Err(())
                    } else {
                        crate::private_storage::write(&path, &bytes, true).map_err(|_| ())
                    }
                })
        });
        let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(result) = result {
            match result {
                Ok(()) => {
                    s.saved = candidate.unwrap().1;
                    s.error = None;
                }
                Err(()) => s.error = Some(SAVE_ERROR.into()),
            }
        }
        s.flushed = flush;
        let mut pending = std::mem::take(&mut s.waiters);
        let count = pending.len();
        pending.retain(|(ticket, tx)| {
            if *ticket <= flush {
                let _ = tx.send(());
                false
            } else {
                true
            }
        });
        let acknowledged = pending.len() != count;
        s.waiters = pending;
        shared.wake.notify_all();
        let mut done = false;
        if s.stop && s.spans == 0 {
            if s.revision == s.saved || !s.persist_allowed || s.error.is_some() {
                s.done = true;
                shared.wake.notify_all();
                done = true;
            } else {
                s.due = Some(Instant::now());
            }
        }
        drop(s);
        if acknowledged || attempted_save {
            let notify = shared
                .notify
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(notify) = notify {
                notify();
            }
        }
        if done {
            return;
        }
    }
}
impl Client {
    /// One lock binds the dashboard's document, reset epoch and storage outcome.
    pub fn dashboard_snapshot(&self) -> (Document, Calendar, Option<String>, u64) {
        let s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.data.clone(), s.calendar.clone(), s.error.clone(), s.epoch)
    }
    pub fn dashboard_version(&self) -> (u64, u64, u64, Option<String>) {
        let s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.revision, s.saved, s.flushed, s.error.clone())
    }
    pub fn attach_wake(&self, notify: Arc<dyn Fn() + Send + Sync>) {
        *self.0.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(notify);
    }
    fn mutate<R>(&self, paused: bool, f: impl FnOnce(&mut State, f64) -> R) -> Option<R> {
        self.mutate_inner(paused, false, f)
    }
    fn mutate_inner<R>(
        &self,
        paused: bool,
        closing: bool,
        f: impl FnOnce(&mut State, f64) -> R,
    ) -> Option<R> {
        let now = (self.0.clock)();
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.done || (s.stop && !closing) || (!s.data.enabled && !paused) {
            return None;
        }
        let result = f(&mut s, now);
        s.changed(now, self.0.delay);
        self.0.wake.notify_all();
        Some(result)
    }
    pub fn snapshot(&self) -> Document {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .data
            .clone()
    }
    pub fn storage_error(&self) -> Option<String> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .clone()
    }
    pub fn attempt(&self, id: &str, source: &str) -> Option<Attempt> {
        let key = command_key(id);
        let source = if SOURCES.contains(&source) {
            source
        } else {
            "cli"
        };
        self.mutate(false, |s, now| {
            let hour = hour_key(now);
            let day = s.calendar.keys(now, s.data.retention_days).0;
            let d = &mut s.data;
            d.total_attempts = d.total_attempts.saturating_add(1);
            d.days.entry(day).or_default().uses += 1;
            d.hours.entry(hour.clone()).or_default().uses += 1;
            if d.commands.contains_key(&key) || d.commands.len() < 2048 {
                let row = d.commands.entry(key.clone()).or_default();
                row.attempts += 1;
                row.last_used = now;
                *row.sources.entry(source.into()).or_default() += 1;
            } else {
                d.omitted_commands += 1;
            }
            Attempt {
                key,
                epoch: s.epoch,
                hour,
            }
        })
    }
    pub fn complete(&self, ticket: Attempt, ok: bool, ms: f64, code: &str, exit: Option<i64>) {
        self.mutate_inner(true, true, |s, now| {
            if s.epoch != ticket.epoch {
                return;
            }
            let day = s.calendar.keys(now, s.data.retention_days).0;
            let d = &mut s.data;
            if ok {
                d.total_successes += 1;
                d.days.entry(day).or_default().successes += 1;
            } else {
                d.total_failures += 1;
                d.days.entry(day).or_default().failures += 1;
            }
            if let Some(hour) = d.hours.get_mut(&ticket.hour) {
                if ok {
                    hour.successes += 1
                } else {
                    hour.failures += 1
                }
            }
            if let Some(row) = d.commands.get_mut(&ticket.key) {
                if ok {
                    row.successes += 1
                } else {
                    row.failures += 1
                }
                row.timing.add(ms, 32);
            }
            d.timings
                .entry("command.execute".into())
                .or_default()
                .add(ms, 256);
            if !ok {
                d.events
                    .push(diagnostic(now, "command", Some(ticket.key), code, exit));
            }
        });
    }
    pub fn timing(&self, name: &str, ms: f64) {
        if TIMINGS.contains(&name) && finite(ms) {
            self.mutate(false, |s, _| {
                s.data.timings.entry(name.into()).or_default().add(ms, 256)
            });
        }
    }
    pub fn error(&self, category: &str, code: &str) {
        if ["config", "clipboard", "keyboard", "runtime"].contains(&category) {
            self.mutate(false, |s, now| {
                s.data
                    .events
                    .push(diagnostic(now, category, None, code, None))
            });
        }
    }
    pub fn configure(&self, enabled: Option<bool>, retention: Option<i64>) -> Result<(), String> {
        if retention.is_some_and(|n| !(1..=90).contains(&n)) {
            return Err("Choose 1–90 days.".into());
        }
        self.mutate(true, |s, _| {
            if let Some(v) = enabled {
                s.data.enabled = v;
            }
            if let Some(v) = retention {
                s.data.retention_days = v;
            }
        });
        Ok(())
    }
    pub fn reset(&self) {
        self.mutate(true, |s, now| {
            s.epoch = s.epoch.checked_add(1).expect("analytics epoch exhausted");
            let enabled = s.data.enabled;
            let retention = s.data.retention_days;
            s.data = Document::new(now);
            s.data.enabled = enabled;
            s.data.retention_days = retention;
            s.data.legacy_imported = true;
            s.persist_allowed = true;
            s.error = None;
        });
    }
    pub fn import_legacy(&self, counts: &std::collections::HashMap<String, i64>) {
        self.mutate(true, |s, _| {
            let d = &mut s.data;
            if d.legacy_imported {
                return;
            }
            d.legacy_imported = true;
            d.prior_successes = counts.values().fold(0i64, |a, n| {
                (a + (*n).clamp(0, 1_000_000_000)).min(1_000_000_000_000)
            });
            let mut sorted = counts.iter().collect::<Vec<_>>();
            sorted.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
            let mut represented = 0;
            for (id, n) in sorted.into_iter().take(2048).filter(|(_, n)| **n > 0) {
                let row = d.commands.entry(command_key(id)).or_default();
                row.prior_successes = (*n).min(1_000_000_000);
                represented += row.prior_successes;
            }
            d.omitted_commands += (d.prior_successes - represented).max(0);
        });
    }
    pub fn flush_async(&self) -> Result<std::sync::mpsc::Receiver<()>, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.stop || s.waiters.len() >= 16 {
            return Err("Analytics writer is busy.".into());
        }
        s.flush = s
            .flush
            .checked_add(1)
            .ok_or("Analytics writer sequence exhausted.")?;
        let ticket = s.flush;
        s.waiters.push((ticket, tx));
        self.0.wake.notify_all();
        Ok(rx)
    }
    /// Called on background/control owners, never from the eventtap/UI thread.
    pub fn flush(&self) {
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.stop {
            return;
        }
        s.flush = s.flush.saturating_add(1);
        let ticket = s.flush;
        self.0.wake.notify_all();
        while !s.stop && s.flushed < ticket {
            s = self.0.wake.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
}
fn diagnostic(
    now: f64,
    category: &str,
    key: Option<String>,
    code: &str,
    exit: Option<i64>,
) -> Diagnostic {
    Diagnostic {
        id: autoreleasepool(|_| objc2_foundation::NSUUID::UUID().UUIDString().to_string()),
        date: now,
        category: category.into(),
        command_id: key,
        code: safe_code(code).into(),
        exit_status: exit.filter(|v| (-1..=255).contains(v)),
    }
}

pub struct Span {
    client: Client,
    ticket: Option<Attempt>,
    started: Instant,
    _active: ActiveSpan,
}
// Retire admission even if registration or completion unwinds. Keeping this
// separate from completion also makes shutdown independent of the clock.
struct ActiveSpan(Arc<Shared>);
impl Drop for ActiveSpan {
    fn drop(&mut self) {
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        s.spans -= 1;
        self.0.wake.notify_all();
    }
}
impl Span {
    pub fn new(client: Client, id: &str, source: &str) -> Option<Self> {
        let started = Instant::now();
        {
            let mut s = client.0.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.stop {
                return None;
            }
            s.spans = s.spans.checked_add(1)?;
        }
        let mut span = Self {
            _active: ActiveSpan(client.0.clone()),
            client,
            ticket: None,
            started,
        };
        span.ticket = span.client.attempt(id, source);
        Some(span)
    }
    pub fn finish(self, ok: bool, code: &str) {
        self.finish_with_exit(ok, code, None);
    }
    pub fn finish_with_exit(mut self, ok: bool, code: &str, exit: Option<i64>) {
        if let Some(ticket) = self.ticket.take() {
            self.client.complete(
                ticket,
                ok,
                self.started.elapsed().as_secs_f64() * 1000.,
                code,
                exit,
            );
        }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            // This destructor can itself run during execution unwinding.
            // Failed best-effort diagnostics must not trigger a second panic.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.client.complete(
                    ticket,
                    false,
                    self.started.elapsed().as_secs_f64() * 1000.,
                    "interrupted",
                    None,
                );
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn root() -> Temp {
        let id = autoreleasepool(|_| objc2_foundation::NSUUID::UUID().UUIDString().to_string());
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/analytics-tests")
            .join(id);
        crate::private_storage::secure_directory(&p).unwrap();
        Temp(p)
    }
    #[test]
    fn completion_survives_pause_but_reset_retires_old_ticket_and_private_export() {
        let t = root();
        let writer = Writer::open(&t.0, None).unwrap();
        let c = writer.client();
        let a = c.attempt("secret own command", "snippet").unwrap();
        c.configure(Some(false), None).unwrap();
        assert!(c.attempt("not recorded", "snippet").is_none());
        c.complete(a, true, 7., "invalid-private-error", None);
        assert_eq!(c.snapshot().total_successes, 1);
        c.configure(Some(true), None).unwrap();
        let old = c.attempt("old", "snippet").unwrap();
        c.reset();
        c.complete(old, false, 99., "io", None);
        let d = c.snapshot();
        assert_eq!(d.total_attempts, 0);
        assert_eq!(d.total_failures, 0);
        assert!(d.commands.is_empty());
        let a = c.attempt("private target query token", "snippet").unwrap();
        c.complete(a, false, 3., "arbitrary raw failure", Some(999));
        let out = c.snapshot().export("fixture").to_string();
        assert!(!out.contains("private target"));
        assert!(!out.contains(&command_key("private target query token")));
        assert!(!out.contains("arbitrary raw"));
        c.flush();
        assert_eq!(
            read(&t.0.join("analytics.json"))
                .unwrap()
                .unwrap()
                .total_failures,
            1
        );
    }
    #[test]
    fn concurrent_original_hourly_counts_bounds_and_aged_retention() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            Barrier,
        };
        let t = root();
        let now = 1_792_890_900_f64;
        let clock = Arc::new(AtomicU64::new(now.to_bits()));
        let writer = controlled(&t.0, &clock, "Europe/Amsterdam");
        let client = writer.client();
        let legacy = std::collections::HashMap::from([("legacy".into(), 5)]);
        client.import_legacy(&legacy);
        client.import_legacy(&legacy);
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let client = client.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    for index in 0..125 {
                        let ticket = client.attempt(&format!("fixture.{worker}"), "cli").unwrap();
                        client.complete(ticket, index % 5 != 0, f64::from(index), "timeout", None);
                        client.timing("search.model", f64::from(index) / 100.);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let document = client.snapshot();
        assert_eq!(
            (
                document.total_attempts,
                document.total_successes,
                document.total_failures,
                document.prior_successes
            ),
            (1000, 800, 200, 5)
        );
        assert_eq!(document.hours.values().map(|h| h.uses).sum::<i64>(), 1000);
        assert_eq!(
            document.hours.values().map(|h| h.successes).sum::<i64>(),
            800
        );
        assert_eq!(
            document.hours.values().map(|h| h.failures).sum::<i64>(),
            200
        );
        assert_eq!(document.events.len(), 200);
        assert_eq!(document.timings["search.model"].samples.len(), 256);
        for worker in 0..8 {
            let row = &document.commands[&command_key(&format!("fixture.{worker}"))];
            assert_eq!(
                (
                    row.attempts,
                    row.successes,
                    row.failures,
                    row.sources["cli"]
                ),
                (125, 100, 25, 125)
            );
            assert_eq!(row.timing.samples.len(), 32);
        }
        assert!(document.valid());
        clock.store((now + 100. * 86400.).to_bits(), Ordering::Release);
        client.timing("search.model", 1.);
        let pruned = client.snapshot();
        assert!(pruned.events.is_empty());
        assert!(pruned.hours.is_empty());
        assert_eq!(
            (
                pruned.total_attempts,
                pruned.total_successes,
                pruned.total_failures,
                pruned.prior_successes
            ),
            (1000, 800, 200, 5)
        );
        client.flush();
        drop(writer);
        let reloaded = controlled(&t.0, &clock, "Europe/Amsterdam");
        assert!(reloaded.client().storage_error().is_none());
        assert_eq!(
            serde_json::to_value(reloaded.client().snapshot()).unwrap(),
            serde_json::to_value(pruned).unwrap()
        );
    }
    fn controlled(root: &Path, clock: &Arc<std::sync::atomic::AtomicU64>, zone: &str) -> Writer {
        let time = clock.clone();
        Writer::open_with(
            root,
            None,
            Arc::new(move || f64::from_bits(time.load(std::sync::atomic::Ordering::Acquire))),
            Calendar {
                identifier: "gregorian".into(),
                zone: zone.into(),
            },
            Duration::from_secs(60),
        )
        .unwrap()
    }
    #[test]
    fn populated_schema_one_upgrade_preserves_every_legacy_field_and_reload() {
        use std::sync::atomic::AtomicU64;
        let t = root();
        let now = 1_792_890_900_f64;
        let clock = Arc::new(AtomicU64::new(now.to_bits()));
        let writer = controlled(&t.0, &clock, "Europe/Amsterdam");
        let client = writer.client();
        client.import_legacy(&std::collections::HashMap::from([("earlier".into(), 7)]));
        for (id, source, ok) in [("first", "launcher", true), ("second", "hotkey", false)] {
            let ticket = client.attempt(id, source).unwrap();
            client.complete(ticket, ok, 3., "timeout", Some(2));
        }
        client.timing("search.model", 1.);
        client.configure(Some(false), Some(7)).unwrap();
        let mut legacy = serde_json::to_value(client.snapshot()).unwrap();
        drop(writer);
        let object = legacy.as_object_mut().unwrap();
        object.insert("schema".into(), json!(1));
        object.insert("omittedCommands".into(), json!(3));
        object.remove("hours");
        object.remove("hourlyStartedAt");
        assert!(!object["commands"].as_object().unwrap().is_empty());
        assert!(!object["events"].as_array().unwrap().is_empty());
        assert!(!object["timings"].as_object().unwrap().is_empty());
        assert!(!object["days"].as_object().unwrap().is_empty());
        std::fs::write(
            t.0.join("analytics.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let upgraded = controlled(&t.0, &clock, "Europe/Amsterdam");
        let client = upgraded.client();
        assert!(client.storage_error().is_none());
        let document = client.snapshot();
        let actual = serde_json::to_value(&document).unwrap();
        for (key, expected) in legacy.as_object().unwrap() {
            if key != "schema" {
                assert_eq!(&actual[key], expected, "legacy field {key}");
            }
        }
        assert_eq!(document.schema, 2);
        assert!(document.hours.is_empty());
        assert_eq!(document.hourly_started_at, Some(now));
        assert!(document
            .hourly_activity(now)
            .iter()
            .take(23)
            .all(|(_, day)| day.is_none()));
        client.configure(Some(true), None).unwrap();
        client.attempt("new-hour", "cli").unwrap();
        client.flush();
        drop(upgraded);
        let reloaded = controlled(&t.0, &clock, "Europe/Amsterdam");
        assert!(reloaded.client().storage_error().is_none());
        let after = reloaded.client().snapshot();
        assert_eq!(
            (
                after.total_attempts,
                after.total_successes,
                after.total_failures,
                after.prior_successes
            ),
            (3, 1, 1, 7)
        );
        assert_eq!(after.hours.values().map(|h| h.uses).sum::<i64>(), 1);
        assert_eq!(after.hourly_started_at, Some(now));
    }
    #[test]
    fn repeated_dst_hour_keeps_attempt_completion_and_timezone_reload_distinct() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let t = root();
        // Original controlled clocks: 2026-10-25 00:35Z and 01:15Z.
        let first = 1_792_888_500_f64;
        let second = 1_792_890_900_f64;
        let clock = Arc::new(AtomicU64::new(first.to_bits()));
        let writer = controlled(&t.0, &clock, "Europe/Amsterdam");
        let client = writer.client();
        let ticket = client.attempt("first", "launcher").unwrap();
        clock.store(second.to_bits(), Ordering::Release);
        client.complete(ticket, true, 3., "timeout", None);
        let ticket = client.attempt("second", "hotkey").unwrap();
        client.complete(ticket, false, 2., "timeout", None);
        let document = client.snapshot();
        let first_key = hour_key(first);
        let second_key = hour_key(second);
        assert_ne!(first_key, second_key);
        assert_eq!(document.hours.len(), 2);
        assert_eq!(
            (
                document.hours[&first_key].uses,
                document.hours[&first_key].successes
            ),
            (1, 1)
        );
        assert_eq!(
            (
                document.hours[&second_key].uses,
                document.hours[&second_key].failures
            ),
            (1, 1)
        );
        autoreleasepool(|_| {
            let calendar = Calendar {
                identifier: "gregorian".into(),
                zone: "Europe/Amsterdam".into(),
            }
            .native();
            for stamp in [hour_start(first), hour_start(second)] {
                let parts = calendar.components_fromDate(
                    NSCalendarUnit::Hour | NSCalendarUnit::Minute,
                    &NSDate::dateWithTimeIntervalSince1970(stamp),
                );
                assert_eq!((parts.hour(), parts.minute()), (2, 0));
            }
        });
        let points = document.hourly_activity(second);
        assert_eq!(points.len(), 24);
        assert!(points.windows(2).all(|p| p[1].0 - p[0].0 == 3600.));
        assert_eq!(points.iter().filter(|(_, h)| h.is_none()).count(), 22);
        assert_eq!(
            points
                .iter()
                .filter_map(|(_, h)| h.as_ref())
                .map(|h| h.uses)
                .sum::<i64>(),
            2
        );
        client.flush();
        drop(writer);
        let reloaded = controlled(&t.0, &clock, "Asia/Kathmandu");
        assert!(reloaded.client().storage_error().is_none());
        assert_eq!(
            serde_json::to_value(reloaded.client().snapshot().hours).unwrap(),
            serde_json::to_value(document.hours).unwrap()
        );
    }
    #[test]
    fn hourly_retention_accepts_2161_prunes_on_advance_and_never_resurrects() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let t = root();
        let now = 1_792_890_900_f64;
        let clock = Arc::new(AtomicU64::new(now.to_bits()));
        let mut document = Document::new(now);
        document.retention_days = 90;
        document.hourly_started_at = Some(now - 2160. * 3600.);
        for offset in 0..=2160 {
            document.hours.insert(
                hour_key(now - f64::from(offset) * 3600.),
                Day {
                    uses: 1,
                    ..Day::default()
                },
            );
        }
        assert!(document.valid());
        std::fs::write(
            t.0.join("analytics.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let writer = controlled(&t.0, &clock, "Europe/Amsterdam");
        let client = writer.client();
        assert!(client.storage_error().is_none());
        assert_eq!(client.snapshot().hours.len(), 2161);
        let oldest = hour_key(now - 2160. * 3600.);
        clock.store((now + 3600.).to_bits(), Ordering::Release);
        client.attempt("boundary", "cli").unwrap();
        assert_eq!(client.snapshot().hours.len(), 2161);
        assert!(!client.snapshot().hours.contains_key(&oldest));
        client.configure(None, Some(1)).unwrap();
        for offset in 0..26 {
            clock.store(
                (now + 3600. + f64::from(offset) * 3600.).to_bits(),
                Ordering::Release,
            );
            client.attempt("retention", "cli").unwrap();
        }
        let retained = client.snapshot();
        assert_eq!(retained.total_attempts, 27);
        assert_eq!(retained.hours.len(), 25);
        assert!(!retained.hours.contains_key(&hour_key(now + 3600.)));
        assert_eq!(retained.hourly_activity(now + 26. * 3600.).len(), 24);
        client.flush();
        drop(writer);
        let reloaded = controlled(&t.0, &clock, "Europe/Amsterdam");
        assert!(reloaded.client().storage_error().is_none());
        assert_eq!(reloaded.client().snapshot().total_attempts, 27);
        assert_eq!(
            serde_json::to_value(reloaded.client().snapshot().hours).unwrap(),
            serde_json::to_value(retained.hours).unwrap()
        );
    }
    #[test]
    fn invalid_known_file_is_not_overwritten_until_explicit_reset() {
        let t = root();
        let path = t.0.join("analytics.json");
        std::fs::write(&path, b"invalid own document").unwrap();
        let writer = Writer::open(&t.0, None).unwrap();
        let c = writer.client();
        assert_eq!(c.storage_error().as_deref(), Some(READ_ERROR));
        let a = c.attempt("own", "snippet").unwrap();
        c.complete(a, false, 1., "io", None);
        c.flush();
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid own document");
        c.reset();
        c.flush();
        assert!(c.storage_error().is_none());
        assert!(read(&path).unwrap().unwrap().legacy_imported);
    }
    #[test]
    fn writer_shutdown_waits_for_real_span_completion_without_losing_final_snapshot() {
        let t = root();
        let writer = Writer::open(&t.0, None).unwrap();
        let client = writer.client();
        let span = Span::new(client.clone(), "own", "snippet").unwrap();
        let (done, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            drop(writer);
            done.send(()).unwrap();
        });
        while !client.0.state.lock().unwrap().stop {
            thread::yield_now();
        }
        assert!(rx.try_recv().is_err());
        assert!(Span::new(client, "late", "snippet").is_none());
        span.finish(true, "action-failed");
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
        let d = read(&t.0.join("analytics.json")).unwrap().unwrap();
        assert_eq!((d.total_attempts, d.total_successes), (1, 1));
    }
    #[test]
    fn async_flush_and_failed_private_write_report_honest_error_and_recover() {
        let t = root();
        let writer = Writer::open(&t.0, None).unwrap();
        let c = writer.client();
        std::fs::create_dir(t.0.join("analytics.json")).unwrap();
        let a = c.attempt("own", "snippet").unwrap();
        c.complete(a, true, 1., "action-failed", None);
        c.flush_async()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(c.storage_error().as_deref(), Some(SAVE_ERROR));
        std::fs::remove_dir(t.0.join("analytics.json")).unwrap();
        c.flush();
        assert!(c.storage_error().is_none());
        assert_eq!(
            read(&t.0.join("analytics.json"))
                .unwrap()
                .unwrap()
                .total_successes,
            1
        );
    }

    fn panic_retires_span(phase: &str) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let t = root();
        let fail = Arc::new(AtomicBool::new(false));
        let clock_fail = fail.clone();
        let writer = Writer::open_with(
            &t.0,
            None,
            Arc::new(move || {
                assert!(!clock_fail.load(Ordering::SeqCst), "injected clock panic");
                crate::activity::now()
            }),
            Calendar::default(),
            Duration::from_secs(3),
        )
        .unwrap();
        let client = writer.client();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if phase == "registration" {
                fail.store(true, Ordering::SeqCst);
                let _span = Span::new(client.clone(), "own", "snippet");
            } else {
                let span = Span::new(client.clone(), "own", "snippet").unwrap();
                fail.store(true, Ordering::SeqCst);
                if phase == "completion" {
                    span.finish(true, "action-failed");
                } else {
                    // Its destructor must remain safe while an outer panic unwinds.
                    let _span = span;
                    panic!("injected execution panic");
                }
            }
        }));
        assert!(outcome.is_err());
        let (done, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            drop(writer);
            let _ = done.send(());
        });
        rx.recv_timeout(Duration::from_secs(3))
            .expect("panicking span must not strand writer shutdown");
        worker.join().unwrap();
        assert_eq!(
            client
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .spans,
            0
        );
    }
    #[test]
    fn registration_panic_retires_span_and_allows_shutdown() {
        panic_retires_span("registration");
    }
    #[test]
    fn completion_panic_retires_span_and_allows_shutdown() {
        panic_retires_span("completion");
    }
    #[test]
    fn unwinding_execution_contains_completion_panic_and_allows_shutdown() {
        panic_retires_span("unwind");
    }
}
