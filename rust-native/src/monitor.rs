//! Visible-only native Monitor ownership. CLI sampling has its own previous
//! counters, preserving the shipped separate UI/CLI baselines.
use crate::{
    resource_sampler::{Sampler, Snapshot, Source},
    system_search::{
        service::{Pending, Service},
        Ticket,
    },
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    System,
    Cpu,
    Memory,
    Disk,
    Power,
    Network,
}
impl Tab {
    pub const ALL: [Self; 6] = [
        Self::System,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::Power,
        Self::Network,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::System => "System Info",
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Disk => "Macintosh HD",
            Self::Power => "Power",
            Self::Network => "Network",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub token: u64,
    pub shown: bool,
    pub active: bool,
    pub show_serial: u64,
    pub tab: Tab,
    pub snapshot: Option<Snapshot>,
    pub history: Vec<Snapshot>,
    pub busy: bool,
    pub error: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Close,
    Pause,
    Resume,
    Tab(Tab),
    ActivityMonitor,
}
pub struct Owner {
    surface: Surface,
    pub revision: u64,
    generation: Arc<AtomicU64>,
    sampler: Arc<Mutex<Sampler>>,
    pending: Option<(u64, Pending<Snapshot>)>,
    next: Option<Instant>,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            surface: Surface {
                token: 0,
                shown: false,
                active: false,
                show_serial: 0,
                tab: Tab::System,
                snapshot: None,
                history: vec![],
                busy: false,
                error: String::new(),
            },
            revision: 0,
            generation: Arc::new(AtomicU64::new(0)),
            sampler: Default::default(),
            pending: None,
            next: None,
        }
    }
}
impl Owner {
    pub fn live(&self) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let expected = self.generation.load(Ordering::Acquire);
        let generation = self.generation.clone();
        Arc::new(move || generation.load(Ordering::Acquire) == expected)
    }
    pub fn fail(&mut self, token: u64, error: String) {
        if self.surface.shown && self.surface.token == token {
            self.surface.error = error;
            self.changed();
        }
    }
    pub fn surface(&self) -> Option<Arc<Surface>> {
        (self.surface.token > 0).then(|| Arc::new(self.surface.clone()))
    }
    pub fn open(&mut self) -> Result<(), String> {
        if !self.surface.shown {
            self.surface.token = self
                .surface
                .token
                .checked_add(1)
                .ok_or("Monitor sequence exhausted.")?;
        }
        self.surface.show_serial = self
            .surface
            .show_serial
            .checked_add(1)
            .ok_or("Monitor sequence exhausted.")?;
        self.surface.shown = true;
        self.set_active(true);
        self.changed();
        Ok(())
    }
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    fn set_active(&mut self, value: bool) {
        if self.surface.active != value {
            self.surface.active = value;
            self.generation.fetch_add(1, Ordering::AcqRel);
            if !value {
                self.pending = None;
                self.next = None;
                self.surface.busy = false;
            } else {
                self.next = Some(Instant::now());
            }
        }
    }
    pub fn operation(&mut self, token: u64, operation: Operation) -> bool {
        if token != self.surface.token || !self.surface.shown {
            return false;
        }
        match operation {
            Operation::Close => {
                self.surface.shown = false;
                self.set_active(false);
            }
            Operation::Pause => self.set_active(false),
            Operation::Resume => self.set_active(true),
            Operation::Tab(tab) => self.surface.tab = tab,
            Operation::ActivityMonitor => {}
        }
        self.changed();
        true
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|(_, p)| p.deadline)
            .into_iter()
            .chain(self.next)
            .min()
    }
    pub fn tick(
        &mut self,
        service: &Service,
        source: Arc<dyn Source>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if let Some(result) = self.pending.as_ref().and_then(|(_, p)| p.poll()) {
            let (generation, _) = self.pending.take().unwrap();
            self.surface.busy = false;
            if self.surface.active && self.generation.load(Ordering::Acquire) == generation {
                match result {
                    Ok(snapshot) => {
                        self.surface.snapshot = Some(snapshot.clone());
                        self.surface.history.push(snapshot);
                        if self.surface.history.len() > 60 {
                            self.surface.history.remove(0);
                        }
                        self.surface.error.clear();
                    }
                    Err(_) => {
                        self.surface.error = "Could not sample system resources. Try again.".into()
                    }
                }
                self.next = Some(Instant::now() + Duration::from_secs(2));
                self.changed();
            }
        }
        if !self.surface.active
            || self.pending.is_some()
            || self.next.is_none_or(|at| Instant::now() < at)
        {
            return;
        }
        let expected = self.generation.load(Ordering::Acquire);
        let generation = self.generation.clone();
        let ticket = Ticket {
            deadline: Instant::now() + Duration::from_secs(3),
            live: Arc::new(move || generation.load(Ordering::Acquire) == expected),
        };
        let sampler = self.sampler.clone();
        match service.resource(ticket, wake, move |ticket| {
            ticket.check()?;
            let snapshot = sampler
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .sample(&*source);
            ticket.check()?;
            Ok(snapshot)
        }) {
            Ok(pending) => {
                self.pending = Some((expected, pending));
                self.next = None;
                self.surface.busy = true;
                self.changed();
            }
            Err(_) => self.next = Some(Instant::now() + Duration::from_millis(20)),
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}
pub fn percent(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.1}%"))
        .unwrap_or_else(|| "—".into())
}
pub fn rows(snapshot: &Snapshot, tab: Tab) -> Vec<(String, String)> {
    use crate::resource_sampler::memory_label as memory;
    let fields: Vec<(&str, String)> = match tab {
        Tab::System => vec![
            ("Uptime", {
                let hours = (snapshot.uptime.max(0.) / 3600.) as u64;
                if hours >= 24 {
                    format!("{} days, {} hours", hours / 24, hours % 24)
                } else {
                    format!("{hours} hours")
                }
            }),
            ("Processes", snapshot.process_count.to_string()),
            ("macOS", snapshot.os.clone()),
            ("Hostname", snapshot.hostname.clone()),
            ("Model", snapshot.model.clone()),
            ("Chip", snapshot.chip.clone()),
            ("CPU Cores", snapshot.cpu_cores.to_string()),
            ("Memory", memory(snapshot.memory_total)),
        ],
        Tab::Cpu => vec![
            ("Chip", snapshot.chip.clone()),
            ("Cores", snapshot.cpu_cores.to_string()),
            (
                "Idle",
                snapshot
                    .cpu_percent
                    .map(|v| percent(Some(100. - v)))
                    .unwrap_or_else(|| "Collecting…".into()),
            ),
        ],
        Tab::Memory => vec![
            ("Physical Memory", memory(snapshot.memory_total)),
            (
                "Available",
                memory(snapshot.memory_total.saturating_sub(snapshot.memory_used)),
            ),
            ("Wired", memory(snapshot.memory_wired)),
            ("Compressed", memory(snapshot.memory_compressed)),
            ("Swap Used", memory(snapshot.swap_used)),
        ],
        Tab::Disk => vec![
            ("Capacity", memory(snapshot.disk_total)),
            (
                "Used",
                memory(snapshot.disk_total.saturating_sub(snapshot.disk_available)),
            ),
            ("Available", memory(snapshot.disk_available)),
        ],
        Tab::Power => vec![
            (
                "Power Source",
                snapshot
                    .power_source
                    .clone()
                    .unwrap_or_else(|| "Unavailable".into()),
            ),
            (
                "Charging",
                snapshot
                    .charging
                    .map(|v| if v { "Yes" } else { "No" }.into())
                    .unwrap_or_else(|| "—".into()),
            ),
        ],
        Tab::Network => vec![
            (
                "Upload",
                snapshot
                    .upload_bytes_per_second
                    .map(|v| format!("{}/s", memory(v.max(0.) as u64)))
                    .unwrap_or_else(|| "—".into()),
            ),
            ("Received", memory(snapshot.network_received)),
            ("Sent", memory(snapshot.network_sent)),
            ("Interfaces", snapshot.interfaces.join(", ")),
        ],
    };
    fields
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect()
}
pub fn metric(snapshot: &Snapshot, tab: Tab) -> Option<(String, &'static str)> {
    use crate::resource_sampler::memory_label as memory;
    Some(match tab {
        Tab::System => return None,
        Tab::Cpu => (percent(snapshot.cpu_percent), "CPU Usage"),
        Tab::Memory => (memory(snapshot.memory_used), "Memory Used"),
        Tab::Disk => (memory(snapshot.disk_available), "Available"),
        Tab::Power => (
            snapshot
                .battery_percent
                .map(|v| format!("{v}%"))
                .unwrap_or_else(|| "No Battery".into()),
            "Battery",
        ),
        Tab::Network => (
            snapshot
                .download_bytes_per_second
                .map(|v| format!("{}/s", memory(v.max(0.) as u64)))
                .unwrap_or_else(|| "—".into()),
            "Download",
        ),
    })
}
pub fn graph(snapshot: &Snapshot, tab: Tab) -> Option<f64> {
    match tab {
        Tab::Cpu => snapshot.cpu_percent,
        Tab::Memory => Some(snapshot.memory_used as f64 / 1_073_741_824.),
        Tab::Network => snapshot.download_bytes_per_second.map(|v| v / 1024.),
        _ => None,
    }
}
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn service() -> Service {
        Service::new(Arc::new(crate::system_search::fixture::Fixture::default()))
    }
    fn settle(owner: &mut Owner, service: &Service, source: Arc<dyn Source>) {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            owner.tick(service, source.clone(), Arc::new(|| {}));
            if !owner.busy() && owner.surface.snapshot.is_some() {
                break;
            }
            assert!(Instant::now() < until, "Owned sample did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn visible_only_two_second_sampling_history_bound_and_reopen_preserve_state() {
        let service = service();
        let source: Arc<dyn Source> = Arc::new(crate::resource_sampler::Fixture::default());
        let mut owner = Owner::default();
        assert!(owner.deadline().is_none());
        owner.tick(&service, source.clone(), Arc::new(|| {}));
        assert!(!owner.busy());
        owner.open().unwrap();
        let token = owner.surface.token;
        for _ in 0..65 {
            owner.next = Some(Instant::now());
            settle(&mut owner, &service, source.clone());
        }
        assert_eq!(owner.surface.history.len(), 60);
        assert_eq!(owner.surface.history[0].timestamp, 110.);
        assert!(owner.next.unwrap() > Instant::now() + Duration::from_millis(1800));
        owner.operation(token, Operation::Tab(Tab::Network));
        let live = owner.live();
        assert!(live());
        assert!(owner.operation(token, Operation::Pause));
        assert!(!live());
        assert!(owner.deadline().is_none());
        assert!(owner.operation(token, Operation::Close));
        assert!(owner.deadline().is_none());
        owner.open().unwrap();
        assert_ne!(owner.surface.token, token);
        assert_eq!(owner.surface.tab, Tab::Network);
        assert_eq!(owner.surface.history.len(), 60);
        assert!(!owner.operation(token, Operation::Close));
        assert!(owner.surface.shown);
        owner.operation(owner.surface.token, Operation::Close);
    }
    #[test]
    fn paused_worker_late_result_cannot_update_reopened_monitor() {
        use std::sync::mpsc;
        struct Held {
            entered: mpsc::SyncSender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl Source for Held {
            fn collect(&self) -> crate::resource_sampler::Raw {
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                crate::resource_sampler::Raw {
                    snapshot: Snapshot {
                        hostname: "stale.invalid".into(),
                        ..Default::default()
                    },
                    timestamp: 1.,
                    cpu: None,
                }
            }
        }
        let service = service();
        let mut owner = Owner::default();
        let (entered, seen) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let source = Arc::new(Held {
            entered,
            release: Mutex::new(wait),
        });
        owner.open().unwrap();
        owner.tick(&service, source, Arc::new(|| {}));
        seen.recv_timeout(Duration::from_secs(2)).unwrap();
        let token = owner.surface.token;
        assert!(owner.busy());
        owner.operation(token, Operation::Close);
        assert!(!owner.busy());
        assert!(owner.deadline().is_none());
        owner.open().unwrap();
        release.send(()).unwrap();
        let source: Arc<dyn Source> = Arc::new(crate::resource_sampler::Fixture::default());
        settle(&mut owner, &service, source);
        assert_eq!(owner.surface.history.len(), 1);
        assert_eq!(
            owner.surface.snapshot.as_ref().unwrap().hostname,
            "owned.invalid"
        );
        owner.operation(owner.surface.token, Operation::Close);
    }
}
