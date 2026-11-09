//! Bounded plain-data presentation of the existing AnalyticsDashboard contract.
//! Native objects are constructed locally and never retained in the view model.
use crate::analytics::{command_key, Calendar, Document, Timing};
use objc2::{rc::autoreleasepool, AllocAnyThread};
use objc2_foundation::{
    NSCalendarOptions, NSCalendarUnit, NSDate, NSDateFormatter, NSLocale, NSString,
};
use rustcast_core::Command;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Point {
    pub date: f64,
    pub label: String,
    pub accessibility: String,
    pub uses: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommandRow {
    pub id: String,
    pub name: String,
    pub uses: i64,
    pub failures: i64,
    pub p95: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ErrorRow {
    pub id: String,
    pub name: String,
    pub reason: String,
    pub date: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Surface {
    pub token: u64,
    pub enabled: bool,
    pub retention_days: i64,
    pub uses: i64,
    pub successes: i64,
    pub failures: i64,
    pub prior_successes: i64,
    pub omitted_commands: i64,
    pub hourly_uses: i64,
    pub recording_since: Option<String>,
    pub hours: Vec<Point>,
    pub days: Vec<Point>,
    pub commands: Vec<CommandRow>,
    pub timings: Vec<(String, String)>,
    pub errors: Vec<ErrorRow>,
    pub storage_error: Option<String>,
    pub busy: bool,
    pub confirming_reset: bool,
    pub confirmation_token: Option<u64>,
    pub message: String,
}
pub fn milliseconds(metric: &Timing) -> String {
    let mut samples = metric.samples.clone();
    samples.sort_by(f64::total_cmp);
    if samples.is_empty() {
        return "—".into();
    }
    let v = samples[((samples.len() - 1) as f64 * 0.95) as usize];
    if v < 1. {
        format!("{v:.2} ms")
    } else {
        format!("{v:.1} ms")
    }
}
pub fn title(name: &str) -> &str {
    match name {
        "search.model" => "Search",
        "search.clipboard" => "Clipboard search",
        "command.execute" => "Commands",
        "cli.dispatch" => "CLI dispatch",
        "launcher.open.model" => "Open launcher",
        "clipboard.preview" => "Image preview",
        "clipboard.capture" => "Clipboard capture",
        "clipboard.save" => "Clipboard storage",
        "process.query" => "Processes",
        "config.read" => "Configuration read",
        "config.reload" => "Reload configuration",
        "monitor.sample" => "System Monitor",
        "process.sort" => "Process sorting",
        "icons.lookup" => "App icons",
        "startup.model" => "Startup",
        other => other,
    }
}
pub fn reason(code: &str) -> &'static str {
    match code {
        "permission" => "Permission required",
        "timeout" => "Timed out",
        "unavailable" => "Item or app unavailable",
        "invalid" | "config-invalid" => "Invalid configuration or command",
        "clipboard-io" => "Clipboard storage failed",
        "interrupted" => "Operation interrupted",
        _ => "Action failed",
    }
}
impl Surface {
    pub fn build<'a>(
        document: &Document,
        calendar: &Calendar,
        commands: impl IntoIterator<Item = &'a Command>,
        now: f64,
        token: u64,
        storage_error: Option<String>,
    ) -> Self {
        let names: HashMap<_, _> = commands
            .into_iter()
            .map(|c| (command_key(&c.id), c.title.clone()))
            .collect();
        autoreleasepool(|_| {
            let c = calendar.native();
            let formatter = NSDateFormatter::new();
            formatter.setLocale(Some(&NSLocale::currentLocale()));
            formatter.setCalendar(Some(&c));
            formatter.setTimeZone(Some(&c.timeZone()));
            let format = |timestamp: f64, pattern: &str| {
                formatter.setDateFormat(Some(&NSString::from_str(pattern)));
                formatter
                    .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(timestamp))
                    .to_string()
            };
            let hours: Vec<_> = document
                .hourly_activity(now)
                .into_iter()
                .map(|(date, value)| Point {
                    date,
                    label: format(date, "HH:mm"),
                    accessibility: format(date, "MMM d, HH:mm z"),
                    uses: value.map(|v| v.uses),
                })
                .collect();
            let today = c.startOfDayForDate(&NSDate::dateWithTimeIntervalSince1970(now));
            let started =
                c.startOfDayForDate(&NSDate::dateWithTimeIntervalSince1970(document.started_at));
            let day_formatter = NSDateFormatter::new();
            day_formatter.setCalendar(Some(&c));
            day_formatter.setTimeZone(Some(&c.timeZone()));
            day_formatter.setLocale(Some(&NSLocale::initWithLocaleIdentifier(
                NSLocale::alloc(),
                &NSString::from_str("en_US_POSIX"),
            )));
            day_formatter.setDateFormat(Some(&NSString::from_str("yyyy-MM-dd")));
            let mut days = vec![];
            for offset in (0..document.retention_days).rev() {
                if let Some(date) = c.dateByAddingUnit_value_toDate_options(
                    NSCalendarUnit::Day,
                    -offset as isize,
                    &today,
                    NSCalendarOptions::empty(),
                ) {
                    if date.timeIntervalSince1970() < started.timeIntervalSince1970() {
                        continue;
                    }
                    let timestamp = date.timeIntervalSince1970();
                    let key = day_formatter.stringFromDate(&date).to_string();
                    days.push(Point {
                        date: timestamp,
                        label: key.clone(),
                        accessibility: key.clone(),
                        uses: document.days.get(&key).map(|v| v.uses),
                    });
                }
            }
            let mut rows: Vec<_> = document.commands.iter().collect();
            rows.sort_by(|(a, x), (b, y)| {
                (y.attempts + y.prior_successes)
                    .cmp(&(x.attempts + x.prior_successes))
                    .then_with(|| a.cmp(b))
            });
            let commands = rows
                .into_iter()
                .take(30)
                .map(|(id, v)| CommandRow {
                    id: id.clone(),
                    name: names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| format!("Removed command · {}", &id[..6])),
                    uses: v.attempts + v.prior_successes,
                    failures: v.failures,
                    p95: milliseconds(&v.timing),
                })
                .collect();
            let errors = document
                .events
                .iter()
                .rev()
                .take(20)
                .map(|v| ErrorRow {
                    id: v.id.clone(),
                    name: v
                        .command_id
                        .as_ref()
                        .and_then(|id| names.get(id))
                        .cloned()
                        .unwrap_or_else(|| {
                            let mut s = v.category.clone();
                            if let Some(first) = s.get_mut(..1) {
                                first.make_ascii_uppercase();
                            }
                            s
                        }),
                    reason: v
                        .exit_status
                        .map(|n| format!("Script exited with status {n}"))
                        .unwrap_or_else(|| reason(&v.code).into()),
                    date: format(v.date, "MMM d, yyyy HH:mm"),
                })
                .collect();
            Self {
                token,
                enabled: document.enabled,
                retention_days: document.retention_days,
                uses: document.total_attempts + document.prior_successes,
                successes: document.total_successes + document.prior_successes,
                failures: document.total_failures,
                prior_successes: document.prior_successes,
                omitted_commands: document.omitted_commands,
                hourly_uses: hours.iter().filter_map(|p| p.uses).sum(),
                recording_since: document
                    .hourly_started_at
                    .filter(|s| hours.first().is_some_and(|p| *s > p.date))
                    .map(|s| format(s, "MMM d, yyyy HH:mm")),
                hours,
                days: if document.days.is_empty() {
                    vec![]
                } else {
                    days
                },
                commands,
                timings: document
                    .timings
                    .iter()
                    .map(|(k, v)| (title(k).into(), milliseconds(v)))
                    .collect(),
                errors,
                storage_error,
                busy: false,
                confirming_reset: false,
                confirmation_token: None,
                message: String::new(),
            }
        })
    }
}

#[derive(Clone, Debug)]
pub enum Operation {
    Refresh,
    ExportPathUnavailable,
    Enabled { expected: bool, value: bool },
    Retention { expected: i64, value: i64 },
    ConfirmReset,
    CancelReset { confirmation: u64 },
    Reset { confirmation: u64 },
    Export(std::path::PathBuf),
}

use std::{
    sync::{mpsc, Arc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Window-local dialog/flush state. No persistent store or AppKit ownership.
pub struct Owner {
    token: u64,
    active: bool,
    confirming_epoch: Option<(u64, u64)>,
    confirmation_serial: u64,
    pending: Option<(mpsc::Receiver<()>, Instant)>,
    message: String,
    cached: Option<(CacheKey, Arc<Surface>)>,
}
type CacheKey = ((u64, u64, u64, Option<String>), i64, [u64; 2], bool);
impl Default for Owner {
    fn default() -> Self {
        Self {
            token: 1,
            active: false,
            confirming_epoch: None,
            confirmation_serial: 0,
            pending: None,
            message: String::new(),
            cached: None,
        }
    }
}
impl Owner {
    pub fn token(&self) -> u64 {
        self.token
    }
    pub fn retire(&mut self) {
        self.token = if self.token == 0 {
            0
        } else {
            self.token.checked_add(1).unwrap_or(0)
        };
        self.cached = None;
        self.active = false;
        self.confirming_epoch = None;
        self.pending = None;
        self.message.clear();
    }
    pub fn activate(&mut self, active: bool) {
        if self.active && !active {
            self.retire();
        }
        self.active = active;
    }
    pub fn wake(&self) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|(_, deadline)| *deadline)
            .or_else(|| {
                self.active
                    .then(|| Instant::now() + Duration::from_secs(60))
            })
    }
    pub fn poll(&mut self, client: &crate::analytics::Client) {
        if let Some((receive, deadline)) = &self.pending {
            let result = match receive.try_recv() {
                Ok(()) => Some(client.storage_error().unwrap_or_default()),
                Err(mpsc::TryRecvError::Empty) if Instant::now() < *deadline => None,
                Err(_) => Some(
                    "Analytics settings were applied in memory; storage completion is unconfirmed."
                        .into(),
                ),
            };
            if let Some(message) = result {
                self.pending = None;
                self.message = message;
                self.cached = None;
            }
        }
    }
    fn flush(&mut self, client: &crate::analytics::Client) {
        match client.flush_async() {
            Ok(receive) => {
                self.pending = Some((receive, Instant::now() + Duration::from_secs(5)));
                self.message.clear();
            }
            Err(_) => {
                self.message =
                    "Analytics settings were applied in memory; storage completion is unconfirmed."
                        .into()
            }
        }
    }
    pub fn operation(
        &mut self,
        client: &crate::analytics::Client,
        op: &Operation,
        export_busy: bool,
    ) -> Result<(), String> {
        self.cached = None;
        if self.token == 0 || !self.active {
            return Err("Dashboard is no longer open.".into());
        }
        if let Operation::CancelReset { confirmation } = op {
            if self
                .confirming_epoch
                .is_some_and(|(_, s)| s == *confirmation)
            {
                self.confirming_epoch = None;
            }
            return Ok(());
        }
        if self.pending.is_some() || export_busy {
            return Err("Analytics operation is still in progress.".into());
        }
        if self.confirming_epoch.is_some() && !matches!(op, Operation::Reset { .. }) {
            return Err("Confirm or cancel resetting analytics.".into());
        }
        let (d, _, _, epoch) = client.dashboard_snapshot();
        match op {
            Operation::ExportPathUnavailable => {
                return Err(
                    "Could not read the selected file path. Choose another file for diagnostics."
                        .into(),
                )
            }
            Operation::Refresh => self.flush(client),
            Operation::Enabled { expected, value } => {
                if d.enabled != *expected {
                    return Err("Analytics settings changed. Try again.".into());
                }
                client.configure(Some(*value), None)?;
                self.flush(client);
            }
            Operation::Retention { expected, value } => {
                if d.retention_days != *expected {
                    return Err("Analytics settings changed. Try again.".into());
                }
                client.configure(None, Some(*value))?;
                self.flush(client);
            }
            Operation::ConfirmReset => {
                self.confirmation_serial = self
                    .confirmation_serial
                    .checked_add(1)
                    .ok_or("Reset confirmation unavailable.")?;
                self.confirming_epoch = Some((epoch, self.confirmation_serial));
            }
            Operation::Reset { confirmation } => {
                if self.confirming_epoch != Some((epoch, *confirmation)) {
                    return Ok(());
                }
                self.confirming_epoch = None;
                client.reset();
                self.flush(client);
            }
            Operation::CancelReset { .. } | Operation::Export(_) => {}
        }
        Ok(())
    }
    pub fn fail(&mut self, error: String) {
        self.message = error;
        self.cached = None;
    }
    pub fn export_complete(&mut self, token: u64, message: String) {
        if self.active && self.token == token {
            self.message = message;
            self.cached = None;
        }
    }
    pub fn surface(
        &mut self,
        client: &crate::analytics::Client,
        commands: impl FnOnce() -> Vec<Command>,
        now: f64,
        export_busy: bool,
        names_revision: [u64; 2],
    ) -> Arc<Surface> {
        self.poll(client);
        let key = (
            client.dashboard_version(),
            (now / 60.).floor() as i64,
            names_revision,
            export_busy,
        );
        if let Some((old, surface)) = &self.cached {
            if *old == key {
                return surface.clone();
            }
        }
        let (d, _, error, epoch) = client.dashboard_snapshot();
        let calendar = Calendar::default();
        if self.confirming_epoch.is_some_and(|(e, _)| e != epoch) {
            self.confirming_epoch = None;
        }
        let mut surface = Surface::build(&d, &calendar, &commands(), now, self.token, error);
        surface.busy = self.pending.is_some() || export_busy;
        surface.confirming_reset = self.confirming_epoch.is_some();
        surface.confirmation_token = self.confirming_epoch.map(|(_, serial)| serial);
        surface.message = self.message.clone();
        let surface = Arc::new(surface);
        self.cached = Some((key, surface.clone()));
        surface
    }
}

struct ExportJob {
    thread: JoinHandle<()>,
    receive: mpsc::Receiver<String>,
    token: u64,
    dismissal: u64,
    publication: Arc<std::sync::Mutex<bool>>,
}
#[derive(Default)]
pub struct Exporter {
    job: Option<ExportJob>,
}
impl Exporter {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn begin(
        &mut self,
        client: &crate::analytics::Client,
        path: std::path::PathBuf,
        token: u64,
        dismissal: u64,
        events: mpsc::SyncSender<crate::control::HostEvent>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Diagnostics export is still in progress.".into());
        }
        if !path.is_absolute() || path.file_name().is_none() {
            return Err("Choose a file for diagnostics.".into());
        }
        // Snapshot at confirmed admission, never at save-panel opening. Runtime
        // refuses reset while this publication is in flight, preserving order.
        let summary = client.snapshot().export("0.9.0-dev");
        let publication = Arc::new(std::sync::Mutex::new(false));
        let permit = publication.clone();
        let (send, receive) = mpsc::channel();
        let thread=std::thread::Builder::new().name("ruycast-diagnostics-export".into()).spawn(move || {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let bytes=serde_json::to_vec_pretty(&summary).map_err(|_|())?;
                crate::private_storage::write_outcome_guarded(&path,&bytes,true,|publish|{
                    let retired=permit.lock().unwrap_or_else(|p|p.into_inner());
                    if *retired{return Err("Diagnostics export was cancelled before publication.".into());}
                    // Hold permit across the actual rename and identity tracking.
                    publish()
                }).map_err(|_|())
            }));
            let message=match result {
                Ok(Ok(crate::private_storage::WriteOutcome::Durable(_))) => "Diagnostics exported",
                Ok(Ok(crate::private_storage::WriteOutcome::Published {..})) => "Diagnostics exported; durability could not be confirmed. Check the file before retrying.",
                Ok(Err(())) => "Could not export diagnostics. Choose a writable folder.",
                Err(_) => "Export completion is unconfirmed. Check the selected file before retrying.",
            };
            let _=send.send(message.into());
            let _=events.try_send(crate::control::HostEvent::UiWake);
        }).map_err(|_|"Could not start diagnostics export.".to_string())?;
        self.job = Some(ExportJob {
            thread,
            receive,
            token,
            dismissal,
            publication,
        });
        Ok(())
    }
    pub fn cancel_unless(&self, token: u64, dismissal: u64, live: bool) {
        if let Some(job) = &self.job {
            if !live || job.token != token || job.dismissal != dismissal {
                // The main/serialized owner never waits for a filesystem call.
                // A held gate already owns publication; late close only retires feedback.
                if let Ok(mut retired) = job.publication.try_lock() {
                    *retired = true;
                }
            }
        }
    }
    pub fn poll(&mut self) -> Option<(u64, u64, String)> {
        if !self.job.as_ref().is_some_and(|j| j.thread.is_finished()) {
            return None;
        }
        let job = self.job.take()?;
        let _ = job.thread.join();
        Some((
            job.token,
            job.dismissal,
            job.receive.try_recv().unwrap_or_else(|_| {
                "Export completion is unconfirmed. Check the selected file before retrying.".into()
            }),
        ))
    }
    pub fn finish(&mut self) {
        self.cancel_unless(0, 0, false);
        if let Some(job) = self.job.take() {
            let _ = job.thread.join();
        }
    }
}
impl Drop for Exporter {
    fn drop(&mut self) {
        self.finish();
    }
}
