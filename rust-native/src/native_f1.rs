//! Feature-only, source-qualified complete F1 endpoint contract.
//! This module never constructs a profile, SDK grant or alternate Session.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    time::{Duration, Instant},
};

pub const SCHEMA: &str = "ruycast.f1.complete.v1";
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub const QUERIES: [&str; 10] = [
    "m",
    "mi",
    "mic",
    "microsoft",
    "alias42",
    "left",
    "project dashboard",
    "missing Ω",
    "",
    "notes",
];
pub const SIZES: [usize; 4] = [500, 1000, 5000, 10000];
pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryStage {
    Search,
    Reconciliation,
    Input,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Startup {
    ProcessCold,
    ProfileCold,
    WarmOpen,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Artifact {
    Binary,
    CompleteApp,
    LinkedRuntimes,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Build {
    Cold,
    Cached,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Functional {
    Recency512,
    UnicodeNulDisabledNfc,
    CachedSearch,
    CachedProcesses1000,
    ClipboardCommandSearch,
    ClipboardEnqueue,
    ClipboardCompletion,
    ClipboardPrune,
    ClipboardEncode,
    ClipboardPreview,
    ConfigRead,
    AnalyticsTiming,
    WarmIcon,
    ClipboardFilter,
    ClipboardImage,
    ClipboardHistoryPruneEncode,
    ConfigReload,
    ProcessSort1000,
    AliasHotkey,
    RichTemplate,
    SnippetKeyboardChoice,
    ScriptProviderChoice,
    CaptureDailyError,
    TimeoutCancelTransport,
    PersistenceExternalEdit,
    StaleSelectionPermission,
}
const FUNCTIONAL: [(&str, Functional); 26] = [
    ("search.recency512", Functional::Recency512),
    (
        "parity.unicode-nul-disabled-nfc",
        Functional::UnicodeNulDisabledNfc,
    ),
    ("search.cached-result-read", Functional::CachedSearch),
    (
        "processes.cached-result-read1000",
        Functional::CachedProcesses1000,
    ),
    (
        "clipboard.command-search-with-history",
        Functional::ClipboardCommandSearch,
    ),
    ("clipboard.enqueue200x8000", Functional::ClipboardEnqueue),
    (
        "clipboard.completion200x8000",
        Functional::ClipboardCompletion,
    ),
    ("clipboard.prune200x8000", Functional::ClipboardPrune),
    ("clipboard.encode200x8000", Functional::ClipboardEncode),
    ("clipboard.preview960000", Functional::ClipboardPreview),
    ("config.raw-read", Functional::ConfigRead),
    ("analytics.timing-record", Functional::AnalyticsTiming),
    ("icons.warm-owned-app", Functional::WarmIcon),
    ("clipboard.filter200x8000", Functional::ClipboardFilter),
    ("clipboard.image960000", Functional::ClipboardImage),
    (
        "clipboard.history-prune-encode",
        Functional::ClipboardHistoryPruneEncode,
    ),
    ("config.reload", Functional::ConfigReload),
    ("processes.sort1000", Functional::ProcessSort1000),
    ("actions.alias-hotkey", Functional::AliasHotkey),
    ("editor.rich-template", Functional::RichTemplate),
    ("snippet.keyboard-choice", Functional::SnippetKeyboardChoice),
    ("script.provider-choice", Functional::ScriptProviderChoice),
    ("capture.result-daily-error", Functional::CaptureDailyError),
    (
        "fault.timeout-cancel-transport",
        Functional::TimeoutCancelTransport,
    ),
    (
        "fault.persistence-external-edit",
        Functional::PersistenceExternalEdit,
    ),
    (
        "fault.stale-selection-permission",
        Functional::StaleSelectionPermission,
    ),
];
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lane {
    Startup(Startup),
    Query {
        stage: QueryStage,
        size: usize,
        query: String,
    },
    Functional(Functional),
    CopiesAllocationsInput,
    Lifecycle10000,
    Idle60Seconds,
    Artifact(Artifact),
    Build(Build),
}
impl Lane {
    pub fn parse(id: &str) -> Result<Self> {
        let fixed = match id {
            "startup.process-cold" => Some(Self::Startup(Startup::ProcessCold)),
            "startup.profile-cold" => Some(Self::Startup(Startup::ProfileCold)),
            "startup.warm-open" => Some(Self::Startup(Startup::WarmOpen)),
            "copies-allocations.input" => Some(Self::CopiesAllocationsInput),
            "lifecycle.10000" => Some(Self::Lifecycle10000),
            "idle.60seconds" => Some(Self::Idle60Seconds),
            "artifact.binary" => Some(Self::Artifact(Artifact::Binary)),
            "artifact.complete-app" => Some(Self::Artifact(Artifact::CompleteApp)),
            "artifact.linked-runtimes" => Some(Self::Artifact(Artifact::LinkedRuntimes)),
            "build.cold" => Some(Self::Build(Build::Cold)),
            "build.cached" => Some(Self::Build(Build::Cached)),
            _ => None,
        };
        if let Some(lane) = fixed {
            return Ok(lane);
        }
        if let Some((_, functional)) = FUNCTIONAL.iter().find(|(name, _)| *name == id) {
            return Ok(Self::Functional(*functional));
        }
        let mut parts = id.splitn(3, '.');
        let stage = match parts.next() {
            Some("search") => QueryStage::Search,
            Some("reconciliation") => QueryStage::Reconciliation,
            Some("input") => QueryStage::Input,
            _ => return Err("unknown complete F1 lane".into()),
        };
        let size = parts
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| SIZES.contains(n))
            .ok_or("unknown F1 catalog size")?;
        let query = parts
            .next()
            .filter(|q| QUERIES.contains(q))
            .ok_or("unknown F1 query")?;
        let canonical = format!(
            "{}.{}.{}",
            match stage {
                QueryStage::Search => "search",
                QueryStage::Reconciliation => "reconciliation",
                QueryStage::Input => "input",
            },
            size,
            query
        );
        if canonical != id {
            return Err("noncanonical F1 lane".into());
        }
        Ok(Self::Query {
            stage,
            size,
            query: query.into(),
        })
    }
    pub fn ids() -> Vec<String> {
        let mut ids = vec![
            "startup.process-cold".into(),
            "startup.profile-cold".into(),
            "startup.warm-open".into(),
        ];
        for size in SIZES {
            for query in QUERIES {
                for stage in ["search", "reconciliation", "input"] {
                    ids.push(format!("{stage}.{size}.{query}"));
                }
            }
        }
        ids.extend(FUNCTIONAL.iter().map(|(name, _)| (*name).into()));
        ids.extend(
            [
                "copies-allocations.input",
                "lifecycle.10000",
                "idle.60seconds",
                "artifact.binary",
                "artifact.complete-app",
                "artifact.linked-runtimes",
                "build.cold",
                "build.cached",
            ]
            .map(str::to_owned),
        );
        ids
    }
    pub fn counts(&self) -> (u64, u64) {
        match self {
            Self::Startup(Startup::ProcessCold | Startup::ProfileCold) => (1000, 0),
            Self::Lifecycle10000 => (3, 100),
            Self::Idle60Seconds => (5, 0),
            Self::Artifact(_) => (1, 0),
            Self::Build(_) => (31, 1),
            _ => (1000, 100),
        }
    }
    pub fn budget(&self) -> Duration {
        Duration::from_secs(match self {
            Self::Lifecycle10000 => 1800,
            Self::Build(_) => 900,
            Self::Idle60Seconds => 90,
            _ => 30,
        })
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Warmup,
    Sample,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    phase: Phase,
    lane: String,
    cycle: u64,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub phase: Phase,
    pub lane: Lane,
    pub id: String,
    pub cycle: u64,
}
impl Request {
    pub fn parse(value: Value) -> Result<Self> {
        let raw: RawRequest =
            serde_json::from_value(value).map_err(|e| format!("F1 request: {e}"))?;
        let lane = Lane::parse(&raw.lane)?;
        let (samples, warmup) = lane.counts();
        if raw.cycle
            >= match raw.phase {
                Phase::Sample => samples,
                Phase::Warmup => warmup,
            }
        {
            return Err("F1 request cycle outside fixed protocol".into());
        }
        Ok(Self {
            phase: raw.phase,
            lane,
            id: raw.lane,
            cycle: raw.cycle,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Completed {
    pub elapsed_ns: u64,
    pub output: Value,
    pub diagnostics: Value,
}
impl Completed {
    pub fn response(self, request: &Request) -> Result<Value> {
        if !self.diagnostics.is_object()
            || self.diagnostics["pending_owners"] != 0
            || self.diagnostics["unexpected_effects"] != json!([])
            || self.diagnostics["fixture_coverage_complete"] != true
        {
            return Err("endpoint completion lacks actual drain/effect/fixture evidence".into());
        }
        Ok(match request.phase {
            Phase::Warmup => {
                json!({"phase":"warmup-complete","lane":request.id,"cycle":request.cycle,"pending_owners":0})
            }
            Phase::Sample => {
                json!({"phase":"sample-complete","lane":request.id,"cycle":request.cycle,"elapsed_ns":self.elapsed_ns,"output":self.output,"diagnostics":self.diagnostics})
            }
        })
    }
}
/// These bindings come from the independently qualified live producer, never
/// from environment values or a JSON request. They grant no authority themselves.
#[derive(Clone, Debug)]
pub struct ReadyBindings {
    pub fixture_sha256: String,
    pub authority_sha256: String,
    pub sdk_guard_sha256: String,
    pub clock_resolution_ns: u64,
}
impl ReadyBindings {
    pub fn envelope(&self) -> Result<Value> {
        for value in [
            &self.fixture_sha256,
            &self.authority_sha256,
            &self.sdk_guard_sha256,
        ] {
            if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("incomplete endpoint ready bindings".into());
            }
        }
        if self.clock_resolution_ns == 0 {
            return Err("invalid endpoint clock resolution".into());
        }
        Ok(
            json!({"schema":SCHEMA,"phase":"ready","fixture_sha256":self.fixture_sha256,"authority_sha256":self.authority_sha256,"sdk_guard_sha256":self.sdk_guard_sha256,"clock_resolution_ns":self.clock_resolution_ns}),
        )
    }
}
/// Bounded NDJSON on already-owned descriptors. This type opens no path and
/// never creates/adopts a descriptor or authorizes an SDK/product entry.
pub struct Frames<R, W> {
    reader: R,
    writer: W,
    pending: Vec<u8>,
}
impl<R: Read, W: Write> Frames<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            pending: Vec::new(),
        }
    }
    pub fn read(&mut self) -> Result<Value> {
        loop {
            if let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
                if end > MAX_FRAME_BYTES {
                    return Err("F1 request exceeds64MiB".into());
                }
                let line = self.pending.drain(..=end).collect::<Vec<_>>();
                return serde_json::from_slice(&line).map_err(|e| format!("F1 frame: {e}"));
            }
            if self.pending.len() > MAX_FRAME_BYTES {
                return Err("F1 request exceeds64MiB".into());
            }
            let mut chunk = [0; 16384];
            match self.reader.read(&mut chunk) {
                Ok(0) => return Err("F1 input closed; no retry".into()),
                Ok(n) => self.pending.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    pub fn write(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("F1 response exceeds64MiB".into());
        }
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .and_then(|_| self.writer.flush())
            .map_err(|e| e.to_string())
    }
}
/// Borrow an already-qualified, exclusive stdio pipe. These wrappers neither
/// adopt nor close/regrant descriptors and never alter parent/child file flags.
/// The caller owns the sole reader/writer until it has joined the endpoint.
pub mod borrowed_stdio {
    use std::{
        io::{self, Read, Write},
        os::fd::{AsRawFd, BorrowedFd},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    struct Pipe<'a> {
        fd: BorrowedFd<'a>,
        stop: Arc<AtomicBool>,
    }
    impl<'a> Pipe<'a> {
        fn new(fd: BorrowedFd<'a>, stop: Arc<AtomicBool>, access: i32) -> io::Result<Self> {
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0 {
                return Err(io::Error::last_os_error());
            }
            if stat.st_mode & libc::S_IFMT != libc::S_IFIFO || flags & libc::O_ACCMODE != access {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "qualified exclusive stdio pipe required",
                ));
            }
            Ok(Self { fd, stop })
        }
        fn wait(&self, events: i16) -> io::Result<()> {
            loop {
                if self.stop.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "owned endpoint stopped",
                    ));
                }
                let mut event = libc::pollfd {
                    fd: self.fd.as_raw_fd(),
                    events,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut event, 1, 10) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if result == 0 {
                    continue;
                }
                if event.revents & libc::POLLNVAL != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "owned stdio descriptor invalid",
                    ));
                }
                if event.revents & (events | libc::POLLHUP | libc::POLLERR) != 0 {
                    return Ok(());
                }
            }
        }
    }
    pub struct Reader<'a>(Pipe<'a>);
    impl<'a> Reader<'a> {
        pub fn new(fd: BorrowedFd<'a>, stop: Arc<AtomicBool>) -> io::Result<Self> {
            Pipe::new(fd, stop, libc::O_RDONLY).map(Self)
        }
    }
    impl Read for Reader<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if bytes.is_empty() {
                return Ok(0);
            }
            self.0.wait(libc::POLLIN)?;
            // With the exclusive reader contract poll-read cannot lose readiness
            // to a second consumer; blocking read returns currently available data.
            let count = unsafe {
                libc::read(
                    self.0.fd.as_raw_fd(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len().min(8192),
                )
            };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }
    }
    pub struct Writer<'a>(Pipe<'a>);
    impl<'a> Writer<'a> {
        pub fn new(fd: BorrowedFd<'a>, stop: Arc<AtomicBool>) -> io::Result<Self> {
            Pipe::new(fd, stop, libc::O_WRONLY).map(Self)
        }
    }
    impl Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.is_empty() {
                return Ok(0);
            }
            self.0.wait(libc::POLLOUT)?;
            //512 is the POSIX minimum atomic PIPE_BUF; poll-ready plus the sole
            //writer prevents another producer taking the reserved writable space.
            let count = unsafe {
                libc::write(
                    self.0.fd.as_raw_fd(),
                    bytes.as_ptr().cast(),
                    bytes.len().min(512),
                )
            };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

// Shared collector/child resource rendezvous. Kernel counters originate only
// in the uninterposed parent, while drain observations are fresh child replies.
pub const LIFECYCLE_POINTS: [u64; 4] = [0, 1000, 5000, 10000];
pub const LIFECYCLE_OUTPUT_ENCODING: &str = "ruycast.f1.lifecycle-output-rle.v1";
/// Lossless ordered runs. Equal means complete serde_json::Value equality;
/// neither hashes, counts alone, normalization nor sorting establish equality.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleOutputRun {
    pub repetitions: u64,
    pub output: Value,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum ResourceLane {
    #[serde(rename = "lifecycle.10000")]
    Lifecycle,
    #[serde(rename = "idle.60seconds")]
    Idle,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "phase", deny_unknown_fields)]
pub enum ResourceFrame {
    #[serde(rename = "lifecycle-cycle")]
    Cycle {
        lane: ResourceLane,
        cycle: u64,
        inner_cycle: u64,
        elapsed_ns: u64,
        output: Value,
        drain: Value,
    },
    #[serde(rename = "checkpoint")]
    Checkpoint {
        lane: ResourceLane,
        cycle: u64,
        completed_cycles: u64,
        drain: Value,
    },
    #[serde(rename = "checkpoint-probe")]
    Probe {
        lane: ResourceLane,
        cycle: u64,
        completed_cycles: u64,
        index: u64,
    },
    #[serde(rename = "checkpoint-drained")]
    Drained {
        lane: ResourceLane,
        cycle: u64,
        completed_cycles: u64,
        index: u64,
        drain: Value,
    },
    #[serde(rename = "checkpoint-continue")]
    Continue {
        lane: ResourceLane,
        cycle: u64,
        completed_cycles: u64,
        observations: Vec<Value>,
    },
    #[serde(rename = "idle-ready")]
    IdleReady {
        lane: ResourceLane,
        cycle: u64,
        drain: Value,
    },
    #[serde(rename = "idle-probe")]
    IdleProbe { lane: ResourceLane, cycle: u64 },
    #[serde(rename = "idle-drained")]
    IdleDrained {
        lane: ResourceLane,
        cycle: u64,
        drain: Value,
    },
    #[serde(rename = "idle-continue")]
    IdleContinue {
        lane: ResourceLane,
        cycle: u64,
        before: Value,
        after: Value,
    },
}
impl ResourceFrame {
    pub fn parse(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| format!("resource rendezvous: {e}"))
    }
}

/// Source-owned evidence required before emitting ready. Merely implementing
/// an enum or a callback cannot claim any lane or SDK boundary is qualified.
pub fn require_complete_routes(actual: &BTreeSet<String>) -> Result<()> {
    let expected: BTreeSet<_> = Lane::ids().into_iter().collect();
    if actual != &expected {
        return Err("production endpoint routes are incomplete; no ready or collection".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiAction {
    Select { index: usize },
    Actions { open: bool },
}

/// The existing PerformanceSoak work outside its per-cycle autorelease pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleBoundary {
    /// Pump one second, draw, and retain the actual native results container.
    PreparedWarmStart,
    /// After each 100 cycles: pump 0.2 seconds and draw the actual view.
    RenderedBlock,
    /// After the final rendered block, verify retained container identity and
    /// both native mode visibilities with their original 0.1-second pumps,
    /// then pump two seconds. This precedes the final footprint checkpoint.
    PreparedTrialFootprint,
    /// Only after that checkpoint: the original 1000 monitor start/stop
    /// cancellations, pump 0.2 seconds, and assert inactive/empty/hidden state.
    /// Allocator pressure relief is deliberately excluded by the scorecard.
    FinishedMonitorStorm,
}

/// Data-only bridge to the *existing* live production owner and coordinator.
/// Native implements these on its Connection/main-loop; reference implements
/// them on rcf1_query/rcf1_call/content_view and the borrowed real AppDelegate.
/// Unqualified optional primitives refuse by default; no substitute Session
/// or shortened lifecycle is supplied here.
pub trait Host {
    fn verify_live(&mut self) -> Result<()>;
    fn bindings(&self) -> &ReadyBindings;
    fn admitted_routes(&self) -> &BTreeSet<String>;
    fn stage_catalog(&mut self, size: usize, deadline: Instant) -> Result<()>;
    fn query(&mut self, stage: QueryStage, query: &str, deadline: Instant) -> Result<()>;
    fn snapshot(&mut self, deadline: Instant) -> Result<Value>;
    fn wire_call(&mut self, request: Value, deadline: Instant) -> Result<Value>;
    fn native_action(&mut self, action: UiAction, deadline: Instant) -> Result<Value>;
    fn prepare_functional(&mut self, lane: Functional, deadline: Instant) -> Result<()>;
    fn functional(&mut self, lane: Functional, deadline: Instant) -> Result<Completed>;
    fn startup(&mut self, kind: Startup, deadline: Instant) -> Result<Completed>;
    /// Exactly one source-qualified complete production cycle, including its
    /// actual completion and owner drain. Warmup and sample use this same call.
    fn lifecycle_cycle(&mut self, _deadline: Instant) -> Result<Completed> {
        Err("complete production lifecycle cycle is not qualified".into())
    }
    fn lifecycle_boundary(
        &mut self,
        _kind: LifecycleBoundary,
        _deadline: Instant,
    ) -> Result<Value> {
        Err("production lifecycle render/container/monitor boundary is not qualified".into())
    }
    fn lifecycle_10000(&mut self, phase: Phase, deadline: Instant) -> Result<Completed>;
    fn idle_60seconds(&mut self, deadline: Instant) -> Result<Completed>;
    fn copies_allocations_input(&mut self, deadline: Instant) -> Result<Completed>;
    fn artifact(&mut self, kind: Artifact, deadline: Instant) -> Result<Completed>;
    fn build(&mut self, kind: Build, deadline: Instant) -> Result<Completed>;
    fn drain(&mut self, deadline: Instant) -> Result<Value>;
    fn shutdown(&mut self, deadline: Instant) -> Result<()>;
}
/// Time actual owner/coordinator completion; serialize full semantic state only
/// after that timer. Source-owned host adapters supply the production routes.
pub fn dispatch(host: &mut impl Host, request: &Request) -> Result<Completed> {
    host.verify_live()?;
    let deadline = Instant::now() + request.lane.budget();
    match &request.lane {
        Lane::Startup(kind) => host.startup(*kind, deadline),
        Lane::Lifecycle10000 => Err("lifecycle requires the parent checkpoint rendezvous".into()),
        Lane::Idle60Seconds => host.idle_60seconds(deadline),
        Lane::CopiesAllocationsInput => host.copies_allocations_input(deadline),
        Lane::Artifact(kind) => host.artifact(*kind, deadline),
        Lane::Build(kind) => host.build(*kind, deadline),
        Lane::Query { stage, size, query } => {
            host.stage_catalog(*size, deadline)?;
            let start = Instant::now();
            host.query(*stage, query, deadline)?;
            let elapsed_ns =
                u64::try_from(start.elapsed().as_nanos()).map_err(|_| "endpoint timer overflow")?;
            let output = host.snapshot(deadline)?;
            let diagnostics = host.drain(deadline)?;
            Ok(Completed {
                elapsed_ns,
                output,
                diagnostics,
            })
        }
        Lane::Functional(kind) => {
            host.prepare_functional(*kind, deadline)?;
            let start = Instant::now();
            // The ordinary reload is the real serialized production request,
            // not a parallel configuration parser or a precomputed fixture.
            let result = match kind {
                Functional::ConfigReload => host.wire_call(json!({"op":"reload"}), deadline)?,
                Functional::UnicodeNulDisabledNfc => {
                    let mut results = Vec::new();
                    for query in ["résumé", "re\u{301}sume\u{301}", "a\0b", "disabled"] {
                        host.query(QueryStage::Search, query, deadline)?;
                        results.push(host.snapshot(deadline)?);
                    }
                    Value::Array(results)
                }
                Functional::ClipboardFilter => host.wire_call(
                    json!({"op":"clipboard.list","query":"F1 matching"}),
                    deadline,
                )?,
                Functional::ClipboardCommandSearch => {
                    host.query(QueryStage::Search, "notes", deadline)?;
                    host.snapshot(deadline)?
                }
                // Native controls/cache getters/IO/cancellation need their
                // real source-owned primitive and measured completion scope.
                // Never relabel a serialized snapshot or mock as that route.
                _ => return host.functional(*kind, deadline),
            };
            let elapsed_ns =
                u64::try_from(start.elapsed().as_nanos()).map_err(|_| "endpoint timer overflow")?;
            let state = host.snapshot(deadline)?;
            let diagnostics = host.drain(deadline)?;
            Ok(Completed {
                elapsed_ns,
                output: json!({"result":result,"state":state}),
                diagnostics,
            })
        }
    }
}

fn require_drain(evidence: &Value) -> Result<()> {
    if !evidence.is_object()
        || evidence["pending_owners"] != 0
        || evidence["unexpected_effects"] != json!([])
        || evidence["fixture_coverage_complete"] != true
    {
        return Err("resource checkpoint lacks fresh qualified drain evidence".into());
    }
    Ok(())
}

fn lifecycle_checkpoint<R: Read, W: Write>(
    host: &mut impl Host,
    frames: &mut Frames<R, W>,
    cycle: u64,
    completed_cycles: u64,
    deadline: Instant,
) -> Result<()> {
    let drain = host.drain(deadline)?;
    require_drain(&drain)?;
    frames.write(
        &serde_json::to_value(ResourceFrame::Checkpoint {
            lane: ResourceLane::Lifecycle,
            cycle,
            completed_cycles,
            drain,
        })
        .map_err(|e| e.to_string())?,
    )?;
    // Each of the parent's five kernel observations has its own new owner
    // drain. An earlier zero cannot stand in for a later observation.
    for expected in 0..5 {
        host.verify_live()?;
        if Instant::now() >= deadline {
            return Err("lifecycle checkpoint deadline".into());
        }
        match ResourceFrame::parse(frames.read()?)? {
            ResourceFrame::Probe {
                lane: ResourceLane::Lifecycle,
                cycle: actual_cycle,
                completed_cycles: actual_completed,
                index,
            } if actual_cycle == cycle
                && actual_completed == completed_cycles
                && index == expected => {}
            _ => return Err("lifecycle checkpoint probe identity/order".into()),
        }
        let drain = host.drain(deadline)?;
        require_drain(&drain)?;
        frames.write(
            &serde_json::to_value(ResourceFrame::Drained {
                lane: ResourceLane::Lifecycle,
                cycle,
                completed_cycles,
                index: expected,
                drain,
            })
            .map_err(|e| e.to_string())?,
        )?;
    }
    match ResourceFrame::parse(frames.read()?)? {
        ResourceFrame::Continue {
            lane: ResourceLane::Lifecycle,
            cycle: actual_cycle,
            completed_cycles: actual_completed,
            observations,
        } if actual_cycle == cycle
            && actual_completed == completed_cycles
            && observations.len() == 5 => {}
        _ => return Err("lifecycle checkpoint continuation identity/count".into()),
    }
    host.verify_live()?;
    if Instant::now() >= deadline {
        return Err("lifecycle checkpoint deadline".into());
    }
    Ok(())
}

#[derive(Default)]
struct LifecycleProgress {
    warmup: u64,
    samples: u64,
}
impl LifecycleProgress {
    fn require_next(&self, request: &Request) -> Result<()> {
        let valid = match request.phase {
            Phase::Warmup => self.samples == 0 && self.warmup < 100 && request.cycle == self.warmup,
            Phase::Sample => {
                self.warmup == 100 && self.samples < 3 && request.cycle == self.samples
            }
        };
        if !valid {
            return Err(
                "lifecycle requires exactly 100 ordered real warmups then three ordered samples"
                    .into(),
            );
        }
        Ok(())
    }

    fn execute<R: Read, W: Write>(
        &mut self,
        host: &mut impl Host,
        frames: &mut Frames<R, W>,
        request: &Request,
    ) -> Result<Completed> {
        self.require_next(request)?;
        let deadline = Instant::now() + request.lane.budget();
        if request.phase == Phase::Warmup {
            let completed = host.lifecycle_cycle(deadline)?;
            require_drain(&completed.diagnostics)?;
            if completed.elapsed_ns == 0 {
                return Err("actual lifecycle cycle timing is zero".into());
            }
            if self.warmup == 99 {
                host.lifecycle_boundary(LifecycleBoundary::PreparedWarmStart, deadline)?;
            }
            let diagnostics = host.drain(deadline)?;
            require_drain(&diagnostics)?;
            self.warmup += 1;
            return Ok(Completed {
                diagnostics,
                ..completed
            });
        }

        lifecycle_checkpoint(host, frames, request.cycle, 0, deadline)?;
        let mut elapsed_ns = 0_u64;
        for cycle in 1..=10000 {
            host.verify_live()?;
            if Instant::now() >= deadline {
                return Err("lifecycle trial deadline".into());
            }
            let completed = host.lifecycle_cycle(deadline)?;
            require_drain(&completed.diagnostics)?;
            if completed.elapsed_ns == 0 {
                return Err("actual lifecycle cycle timing is zero".into());
            }
            elapsed_ns = elapsed_ns
                .checked_add(completed.elapsed_ns)
                .ok_or("lifecycle timer overflow")?;
            // Serialization is outside the actual cycle timer. The parent
            // retains all timings and losslessly compresses only equal full
            // outputs; no growing output/timing cache lives in the measured
            // child at a physical-footprint checkpoint.
            frames.write(
                &serde_json::to_value(ResourceFrame::Cycle {
                    lane: ResourceLane::Lifecycle,
                    cycle: request.cycle,
                    inner_cycle: cycle,
                    elapsed_ns: completed.elapsed_ns,
                    output: completed.output,
                    drain: completed.diagnostics,
                })
                .map_err(|e| e.to_string())?,
            )?;
            if cycle % 100 == 0 {
                host.lifecycle_boundary(LifecycleBoundary::RenderedBlock, deadline)?;
            }
            if cycle == 10000 {
                host.lifecycle_boundary(LifecycleBoundary::PreparedTrialFootprint, deadline)?;
            }
            if LIFECYCLE_POINTS.contains(&cycle) {
                lifecycle_checkpoint(host, frames, request.cycle, cycle, deadline)?;
            }
        }
        let final_state =
            host.lifecycle_boundary(LifecycleBoundary::FinishedMonitorStorm, deadline)?;
        let mut diagnostics = host.drain(deadline)?;
        require_drain(&diagnostics)?;
        let object = diagnostics
            .as_object_mut()
            .ok_or("lifecycle drain object required")?;
        // Counts describe the successful calls above; they never substitute for
        // those calls, their timings, or the fresh parent/kernel rendezvous.
        for key in [
            "cycles",
            "warmup_cycles",
            "allocator_relief_used",
            "cycle_elapsed_ns",
        ] {
            if object.contains_key(key) {
                return Err("lifecycle backend attempted to replace common trial evidence".into());
            }
        }
        object.insert("cycles".into(), json!(10000));
        object.insert("warmup_cycles".into(), json!(self.warmup));
        object.insert("allocator_relief_used".into(), json!(false));
        self.samples += 1;
        Ok(Completed {
            elapsed_ns,
            output: json!({"final_state":final_state}),
            diagnostics,
        })
    }
}
/// Call only after the affine qualified Host entry and real coordinator-ready
/// rendezvous. The caller keeps the existing life/child ownership throughout.
pub fn serve<R: Read, W: Write>(host: &mut impl Host, frames: &mut Frames<R, W>) -> Result<()> {
    host.verify_live()?;
    require_complete_routes(host.admitted_routes())?;
    frames.write(&host.bindings().envelope()?)?;
    let mut lifecycle = LifecycleProgress::default();
    loop {
        let raw = frames.read()?;
        host.verify_live()?;
        if raw == json!({"phase":"shutdown"}) {
            let deadline = Instant::now() + Duration::from_secs(10);
            host.shutdown(deadline)?;
            let evidence = host.drain(deadline)?;
            if evidence["pending_owners"] != 0 {
                return Err("shutdown still owns pending work".into());
            }
            frames.write(&json!({"phase":"shutdown-complete","pending_owners":0}))?;
            return Ok(());
        }
        let request = Request::parse(raw)?;
        let completed = if request.lane == Lane::Lifecycle10000 {
            lifecycle.execute(host, frames, &request)?
        } else {
            dispatch(host, &request)?
        };
        let response = completed.response(&request)?;
        frames.write(&response)?;
    }
}

/// Deterministic original fixtures, assembled as data in Rust. A qualified host
/// publishes these through its existing config/clipboard production paths; this
/// module neither writes profiles nor invents an independent search/history.
pub mod fixture {
    use super::*;
    pub fn catalog(size: usize) -> Result<Vec<Value>> {
        if !SIZES.contains(&size) {
            return Err("catalog size outside original scorecard".into());
        }
        Ok((0..size).map(|i|json!({"id":format!("fixture.{i}"),"title":format!("{} {i}",["Microsoft Teams","Open Notes","Left Half","Project Dashboard"][i%4]),"kind":"url","target":format!("https://example.com/{i}"),"aliases":[format!("alias{i}")],"arguments":[]})).collect())
    }
    pub fn configuration(base: &Value, size: usize) -> Result<Value> {
        let mut configuration = base
            .as_object()
            .cloned()
            .ok_or("complete configuration object required")?;
        if !configuration.contains_key("version") || !configuration.contains_key("clipboard") {
            return Err("production configuration fields absent".into());
        }
        configuration.insert("commands".into(), Value::Array(catalog(size)?));
        Ok(Value::Object(configuration))
    }
    pub fn unicode_catalog() -> Vec<Value> {
        [("nfc","résumé",true),("nfd","re\u{301}sume\u{301}",true),("nul","a\0b",true),("disabled","disabled",false)].into_iter().map(|(id,title,enabled)|json!({"id":format!("fixture.{id}"),"title":title,"kind":"url","target":"https://example.com/fixture","aliases":[title],"arguments":[],"enabled":enabled})).collect()
    }
    /// Same literal text shape as immutable 0.8.26 PerformanceChecks. The old
    /// lane's '8kB' name is shorthand: preserve Unicode bytes, never truncate.
    pub fn clipboard_text(index: usize) -> Result<String> {
        if index >= 200 {
            return Err("clipboard index outside original200items".into());
        }
        Ok(format!(
            "Fixture {index}: {}",
            "native clipboard Ω ".repeat(400)
        ))
    }
    pub fn history(epoch: i64) -> Result<Vec<Value>> {
        (0..200).map(|i|{
            let date=chrono::DateTime::from_timestamp(epoch-i as i64,0).ok_or("fixture epoch outside supported date range")?.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            Ok(json!({"id":format!("raycast-f1-fixture-{i}"),"text":clipboard_text(i)?,"source":"fixture","date":date,"contentType":"text"}))
        }).collect()
    }
    pub fn recency(epoch: i64) -> Value {
        let entries = (0..512)
            .map(|i| (format!("fixture.{i}"), json!(epoch - (511 - i))))
            .collect::<serde_json::Map<_, _>>();
        json!({"schema":1,"entries":entries})
    }
    /// The image fixture has960000PIXELS, not960000encodedbytes. Generation
    /// preserves the original LCG per RGBA pixel; encoding/capture stay in the
    /// real image owner of each arm and are measured only in their own lane.
    pub fn image_rgba() -> Vec<u8> {
        let mut pixels = Vec::with_capacity(1200 * 800 * 4);
        let mut seed = 42u32;
        for _ in 0..1200 * 800 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            pixels.extend_from_slice(&[seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]);
        }
        pixels
    }
    pub fn process_resources(uid: u32) -> serde_json::Map<String, Value> {
        (0..1000).map(|i|(format!("fixture.process.{i}"),json!({
            "identity":{"pid":200000+i,"uid":uid,"seconds":i,"micros":0},
            "name":format!("Process {i}"),"path":"/usr/bin/true","cpuPercent":i%97,"memoryBytes":i*1024
        }))).collect()
    }
    pub fn process_commands() -> Vec<Value> {
        (0..1000).map(|i|json!({"id":format!("fixture.process.{i}"),"title":format!("Process {i}"),"kind":"builtin","target":format!("fixture.process.{i}"),"aliases":[],"arguments":[]})).collect()
    }
}

/// Exact existing SwiftF1Bridge C ABI, populated only from its independently
/// source-qualified image. No dlopen/dlsym, raw legacy entry or Native fallback.
pub mod reference {
    use super::*;
    use std::{
        cell::Cell,
        collections::BTreeMap,
        ffi::{CStr, c_char, c_void},
        marker::PhantomData,
        ptr::NonNull,
        rc::Rc,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    pub type Guard = extern "C" fn(u32) -> i32;
    /// The caller must keep the supplied borrowed buffer valid for count bytes
    /// through the callback; qualification never makes arbitrary pointers safe.
    pub type Reply = unsafe extern "C" fn(*mut c_void, *const c_void, isize);
    pub type ProcessProvider = extern "C" fn(*mut c_void) -> *mut c_void;
    pub type Wake = Arc<dyn Fn() + Send + Sync>;
    /// Actual typed callback addresses for the SDK's private source/image
    /// qualification. Merely returning them grants no callback invocation,
    /// object ownership, image lifetime or original-code permission.
    pub fn callback_entrypoints() -> (Reply, ProcessProvider) {
        (reply, process_provider)
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Functions {
        pub install_guard: unsafe extern "C" fn(Option<Guard>),
        pub borrow_delegate: unsafe extern "C" fn() -> *mut c_void,
        pub create_with_process_provider: unsafe extern "C" fn(
            *const c_char,
            i32,
            i32,
            Option<ProcessProvider>,
            *mut c_void,
        ) -> *mut c_void,
        pub cancel_system_search: unsafe extern "C" fn(*mut c_void),
        pub drop_box: unsafe extern "C" fn(*mut c_void),
        pub query: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        pub mode: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        pub select: unsafe extern "C" fn(*mut c_void, isize),
        pub actions: unsafe extern "C" fn(*mut c_void, i32),
        pub set_commands: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        pub snapshot: unsafe extern "C" fn(*mut c_void) -> *mut objc2_foundation::NSData,
        pub result_count: unsafe extern "C" fn(*mut c_void) -> isize,
        pub visible_count: unsafe extern "C" fn(*mut c_void) -> isize,
        pub selected_id_count: unsafe extern "C" fn(*mut c_void) -> isize,
        pub filtered_count: unsafe extern "C" fn(*mut c_void) -> isize,
        pub apply_system_results: unsafe extern "C" fn(
            *mut c_void,
            *const c_void,
            isize,
            *const c_void,
            isize,
            i32,
        ) -> i32,
        pub process_sort: unsafe extern "C" fn(*mut c_void) -> i32,
        pub set_process_sort: unsafe extern "C" fn(*mut c_void, i32) -> i32,
        pub process_snapshot: unsafe extern "C" fn(*mut c_void) -> *mut objc2_foundation::NSData,
        pub config_data: unsafe extern "C" fn(*mut c_void) -> *mut objc2_foundation::NSData,
        pub encode_clips: unsafe extern "C" fn(*mut c_void) -> *mut objc2_foundation::NSData,
        pub record_timing: unsafe extern "C" fn(*mut c_void, *const c_void, isize, f64) -> i32,
        pub record_recency: unsafe extern "C" fn(*mut c_void, *const c_void, isize, f64) -> i32,
        pub call:
            unsafe extern "C" fn(*mut c_void, *const c_void, isize, Reply, *mut c_void) -> i32,
        pub content_view: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    }
    pub const MAIN_FUNCTIONS_MAGIC: u64 = 0xF100_0026_4D41_494E;
    pub const MAIN_FUNCTIONS_ABI: u32 = 1;
    pub const MAIN_FUNCTIONS_COUNT: u32 = 25;
    /// Main-image transport only. The SDK's sealed ReferenceAdmission must
    /// qualify the exact table range/offset and every typed function against
    /// private source/image pins BEFORE even dereferencing this value.
    /// A matching header supplies no image lease, code or scope authority.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct MainFunctionsV1 {
        pub magic: u64,
        pub abi: u32,
        pub count: u32,
        pub functions: Functions,
    }
    impl MainFunctionsV1 {
        pub fn check_header(&self) -> Result<()> {
            if self.magic != MAIN_FUNCTIONS_MAGIC
                || self.abi != MAIN_FUNCTIONS_ABI
                || self.count != MAIN_FUNCTIONS_COUNT
            {
                return Err("reference main function table transport header differs".into());
            }
            Ok(())
        }
    }
    /// Fixed Reference bridge issuer, implemented by the SDK around its sealed
    /// ReferenceAdmission and actual source/code owners, never by a receipt.
    ///
    /// # Safety
    /// A successful qualified_main_table must establish the exact immutable,
    /// aligned, loaded 216-byte table BEFORE returning its address: all 25 typed
    /// main-image C slots, private source/ABI pins, main initializer and SDK-zero
    /// role, and the actual five SDK entrypoints. The three supplied callbacks
    /// must equal that SDK's qualified guard/reply/provider addresses. No dynamic
    /// callback may intervene after the final memory/code comparison.
    /// The retained issuer must keep the real admission/roots and code custody
    /// alive. MAIN and the initial inserted SDK are process-lifetime images,
    /// with no unload/replacement route; Arc<()> or a matching header is not
    /// custody. Every method must refuse retired/rebound/unqualified state.
    /// before_main_call requires the actual main owner and original call scope.
    /// before_worker_callback must independently qualify ONLY the existing
    /// provider/data/ARC callback on its real worker; it grants no main access.
    /// Implementations must tolerate reentrancy and must not hold a model or
    /// registry mutex across callbacks. Neither check has a permissive default.
    pub unsafe trait SourceCodeReader: Send + Sync {
        fn qualified_main_table(
            &self,
            guard: Guard,
            reply: Reply,
            provider: ProcessProvider,
        ) -> Result<usize>;
        /// Exact private profile from that SAME sealed admission, never an
        /// environment/path supplied by the benchmark request or caller.
        fn qualified_profile(&self) -> Result<std::ffi::CString>;
        /// Main-owner-only immutable fixture epoch from that sealed admission.
        fn qualified_fixture_epoch(&self) -> Result<u64>;
        /// Main-owner-only bounded durable search-ranking.json read from the
        /// SAME private profile/owned-root issuer; no caller-selected path.
        fn qualified_process_ranking(&self) -> Result<Vec<u8>>;
        /// The same qualified Bridge owns the original palette throughout this
        /// request. Implementations must retain the actual field/delegate and
        /// dispatch its real same-field notification, never assign state.query.
        /// False means the retained field is still mounting and the delegate
        /// has not been called for this epoch.
        fn begin_field_input(&self, bridge:&Bridge, query:&str, epoch:u64)->Result<bool>;
        fn verify_field_input(&self, bridge:&Bridge, epoch:u64)->Result<()>;
        /// Prove that this retained epoch has not entered the original delegate.
        /// An empty trace alone cannot establish that fact.
        fn field_delegate_not_entered(&self, _bridge: &Bridge, _epoch: u64) -> Result<bool> {
            Ok(false)
        }
        fn retire_field_input(&self, bridge:&Bridge, epoch:u64,completion_or_reset_confirmed:bool)->Result<()>;
        /// Release this Bridge's exact borrowed box while its original parents
        /// stay independently held. An error must precede the foreign release.
        ///
        /// # Safety
        /// The object and drop function must describe this Bridge's live borrowed
        /// box; its original parents must remain retained throughout the release.
        unsafe fn release_borrowed_field_box(
            &self,
            _object: NonNull<c_void>,
            _drop_box: unsafe extern "C" fn(*mut c_void),
        ) -> Result<()> {
            Err("reference borrowed field box release is unqualified".into())
        }
        fn before_main_call(&self) -> Result<()>;
        fn before_worker_callback(&self) -> Result<()>;
    }
    /// Main-affine typed copy plus its genuine, retained SDK issuer. Copying the
    /// function words does not independently grant a route or full owner drain.
    pub struct QualifiedFunctions {
        functions: Functions,
        guard: Guard,
        source: Arc<dyn SourceCodeReader>,
        _main: PhantomData<Rc<()>>,
    }
    impl QualifiedFunctions {
        pub fn read(source: Arc<dyn SourceCodeReader>, guard: Guard) -> Result<Self> {
            if unsafe { libc::pthread_main_np() } != 1 {
                return Err("reference typed table read off the actual main owner".into());
            }
            let address = source.qualified_main_table(guard, reply, process_provider)?;
            let address = checked_table_address(address)?;
            // SAFETY: only the unsafe issuer contract above can reach this read;
            // no header check, arbitrary manifest or raw address grants access.
            let table = unsafe { address.read() };
            table.check_header()?;
            source.before_main_call()?;
            Ok(Self { functions: table.functions, guard, source, _main: PhantomData })
        }
    }
    fn checked_table_address(address: usize) -> Result<*const MainFunctionsV1> {
        if address == 0
            || !address.is_multiple_of(std::mem::align_of::<MainFunctionsV1>())
            || address.checked_add(std::mem::size_of::<MainFunctionsV1>()).is_none()
        {
            return Err("reference typed table address/range differs".into());
        }
        Ok(address as *const MainFunctionsV1)
    }
    fn source_main_call(source: Option<&Arc<dyn SourceCodeReader>>, guard: Guard, operation: u32) -> Result<()> {
        if unsafe { libc::pthread_main_np() } != 1 {
            return Err("reference bridge off the actual main owner".into());
        }
        if let Some(source) = source { source.before_main_call()?; }
        if guard(operation) != 1 { return Err("reference bridge source scope refused".into()); }
        // The dynamic guard may retire/rebind proof. Recheck immediately before
        // the foreign call rather than trusting its earlier return value.
        if let Some(source) = source { source.before_main_call()?; }
        Ok(())
    }
    /// C names in exact repr(C) field order. These are declared symbol labels,
    /// never runtime dlsym/mangled-name guesses or an original-code grant.
    pub const MAIN_FUNCTION_SYMBOLS: [&str; 25] = [
        "rcf1_install_guard", "rcf1_borrow_delegate", "rcf1_create_with_process_provider",
        "rcf1_cancel_system_search", "rcf1_drop", "rcf1_query", "rcf1_mode", "rcf1_select",
        "rcf1_actions", "rcf1_set_commands", "rcf1_snapshot", "rcf1_result_count",
        "rcf1_visible_count", "rcf1_selected_id_count", "rcf1_filtered_count",
        "rcf1_apply_system_results", "rcf1_process_sort", "rcf1_set_process_sort",
        "rcf1_process_snapshot", "rcf1_config_data", "rcf1_encode_clips",
        "rcf1_record_timing", "rcf1_record_recency", "rcf1_call", "rcf1_content_view",
    ];
    /// Explicit C-ABI enum marshalling, not a second sorting policy.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(i32)]
    pub enum ProcessSort {
        Cpu = 0,
        Memory = 1,
        Name = 2,
    }
    impl TryFrom<i32> for ProcessSort {
        type Error = String;
        fn try_from(value: i32) -> Result<Self> {
            match value {
                0 => Ok(Self::Cpu),
                1 => Ok(Self::Memory),
                2 => Ok(Self::Name),
                _ => Err("reference process-sort ABI value refused".into()),
            }
        }
    }
    fn process_owner_ready(state: &Value) -> Result<()> {
        if state["mode"] != "processes"
            || state["paletteVisible"] != false
            || state["query"] != ""
            || state["actionsOpen"] != false
        {
            return Err("reference processes require the qualified settled hidden owner and original empty query".into());
        }
        Ok(())
    }
    fn process_sort_name(sort: ProcessSort) -> &'static str {
        match sort {
            ProcessSort::Cpu => "CPU Usage",
            ProcessSort::Memory => "Memory",
            ProcessSort::Name => "Name",
        }
    }
    fn process_sort_value(snapshot: &Value) -> Result<ProcessSort> {
        match snapshot["processSort"].as_str() {
            Some("CPU Usage") => Ok(ProcessSort::Cpu),
            Some("Memory") => Ok(ProcessSort::Memory),
            Some("Name") => Ok(ProcessSort::Name),
            _ => Err("reference process-sort snapshot absent or unknown".into()),
        }
    }
    // Fixed-fixture expected order only; never used to construct measured state
    // or to replace AppState.rebuildProcessOrder/setProcessSort.
    fn process_fixture_order(sort: ProcessSort) -> Vec<usize> {
        match sort {
            ProcessSort::Cpu => (0..97)
                .rev()
                .flat_map(|cpu| (0..1000).filter(move |i| i % 97 == cpu))
                .collect(),
            ProcessSort::Memory => (0..1000).rev().collect(),
            ProcessSort::Name => (0..1000).collect(),
        }
    }
    fn validate_process_snapshot(snapshot: &Value, uid: u32) -> Result<()> {
        let commands = fixture::process_commands();
        if snapshot["results"] != Value::Array(commands.clone()) {
            return Err("reference original process results changed".into());
        }
        let expected = fixture::process_resources(uid);
        let resources = snapshot["processResources"]
            .as_object()
            .ok_or("reference process resources absent")?;
        if resources.len() != expected.len() {
            return Err("reference process-resource count changed".into());
        }
        for (id, wanted) in expected {
            let actual = resources
                .get(&id)
                .and_then(Value::as_object)
                .ok_or("reference process resource absent")?;
            let wanted = wanted.as_object().ok_or("invalid Rust process fixture")?;
            // Swift's Codable Double may encode an integral CPU as96, whereas
            // Rust's f64 may encode96.0. Check that exact typed CPU value here;
            // retain the untouched actual snapshot for output/parity review.
            if actual.len() != wanted.len()
                || actual.get("cpuPercent").and_then(Value::as_f64) != wanted["cpuPercent"].as_f64()
                || wanted
                    .iter()
                    .any(|(key, value)| key != "cpuPercent" && actual.get(key) != Some(value))
            {
                return Err("reference original process resource changed".into());
            }
        }
        let sort = process_sort_value(snapshot)?;
        if sort == ProcessSort::Name {
            return Err("reference process workload left CPU/Memory alternation".into());
        }
        let order = process_fixture_order(sort);
        let visible = snapshot["visibleCommands"]
            .as_array()
            .ok_or("reference process visible rows absent")?;
        if visible.len() != 1000
            || visible
                .iter()
                .zip(order)
                .any(|(actual, i)| actual != &commands[i])
        {
            return Err("reference complete ordered process rows changed".into());
        }
        let index = snapshot["selection"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|i| *i < visible.len())
            .ok_or("reference process selection is invalid")?;
        if snapshot["selectedID"] != "fixture.process.96"
            || snapshot["selectedID"] != visible[index]["id"]
        {
            return Err("reference selected process identity changed".into());
        }
        Ok(())
    }
    const REPLY_CONTEXT_LIMIT: usize = 16;
    const QUALIFIED_SOURCE_HOLD_LIMIT: usize = 18;
    const QUALIFIED_BOX_LIMIT: usize = QUALIFIED_SOURCE_HOLD_LIMIT - REPLY_CONTEXT_LIMIT;
    static QUALIFIED_BOXES: AtomicUsize = AtomicUsize::new(0);
    // Counting only: this permit is NOT image/code authority. The real issuer
    // below is separately retained alongside the actual +1 foreign F1Box.
    struct SourceBoxPermit { count: &'static AtomicUsize }
    impl SourceBoxPermit {
        fn acquire() -> Result<Self> { Self::from_counter(&QUALIFIED_BOXES) }
        fn from_counter(count: &'static AtomicUsize) -> Result<Self> {
            count.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < QUALIFIED_BOX_LIMIT).then_some(value + 1)
            }).map_err(|_| "reference main/headless qualified box capacity exhausted")?;
            Ok(Self { count })
        }
    }
    impl Drop for SourceBoxPermit {
        fn drop(&mut self) {
            let before = self.count.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(before > 0);
        }
    }
    struct QualifiedSourceBox {
        reader: Arc<dyn SourceCodeReader>,
        _permit: SourceBoxPermit,
    }
    impl QualifiedSourceBox {
        fn finish(self, local_owners_idle_and_authorized: bool, release: impl FnOnce()) {
            if local_owners_idle_and_authorized {
                release();
                // The real issuer/permit remain held through foreign return.
                drop(self);
            } else {
                // The corresponding +1 F1Box is intentionally NOT released.
                // Keep its actual issuer/permit for process lifetime as well.
                // No second registry, fake cleanup or whole-owner drain claim.
                std::mem::forget(self);
            }
        }
    }
    struct Completion {
        result: Mutex<Option<Result<Vec<u8>>>>,
        wake: Option<Wake>,
        source: Option<Arc<dyn SourceCodeReader>>,
        guard: Option<Guard>,
        retired: AtomicBool,
        reply_received: AtomicBool,
        submission_started: AtomicBool,
    }
    impl Completion {
        fn start_submission(&self, request_live: bool) -> Result<()> {
            if !request_live {
                return Err("reference RPC expired before original handle admission".into());
            }
            self.submission_started.store(true, Ordering::Release);
            Ok(())
        }
    }
    fn remove_completion(key: usize, state: &Arc<Completion>) {
        // Dropping a real source owner may release resources. Do that only
        // after the registry mutex is gone, including a reentrant late reply.
        let removed = PENDING.lock().ok().and_then(|mut table| {
            if table.get(&key).is_some_and(|s| Arc::ptr_eq(s, state)) {
                table.remove(&key)
            } else { None }
        });
        drop(removed);
    }
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    static PENDING: Mutex<BTreeMap<usize, Arc<Completion>>> = Mutex::new(BTreeMap::new());

    pub fn verify_reply_contexts_drained() -> Result<()> {
        if !PENDING.try_lock().map_err(|_| "reference callback registry busy or poisoned")?.is_empty() {
            return Err("reference callbacks not drained".into());
        }
        Ok(())
    }
    // Numeric cookies are memory-lifetime lookup only, never source admission.
    // The existing NEXT namespace is shared with replies and never reused.
    struct ProcessProviderState {
        bytes: Vec<u8>,
        guard: Guard,
        source: Option<Arc<dyn SourceCodeReader>>,
        retired: AtomicBool,
        in_flight: AtomicUsize,
    }
    impl ProcessProviderState {
        fn live(&self) -> bool {
            if self.retired.load(Ordering::Acquire) { return false; }
            if let Some(source) = &self.source {
                if source.before_worker_callback().is_err() || self.retired.load(Ordering::Acquire) {
                    return false;
                }
            }
            if (self.guard)(8) != 1 || self.retired.load(Ordering::Acquire) { return false; }
            if let Some(source) = &self.source {
                if source.before_worker_callback().is_err() { return false; }
            }
            !self.retired.load(Ordering::Acquire)
        }
    }
    static PROCESS_PROVIDERS: Mutex<BTreeMap<usize, Arc<ProcessProviderState>>> =
        Mutex::new(BTreeMap::new());
    struct ProcessProviderOwner {
        key: usize,
        state: Arc<ProcessProviderState>,
    }
    impl ProcessProviderOwner {
        fn register(guard: Guard) -> Result<Self> {
            Self::register_source(guard, None)
        }
        fn register_source(guard: Guard, source: Option<Arc<dyn SourceCodeReader>>) -> Result<Self> {
            let resources = fixture::process_resources(unsafe { libc::getuid() });
            let rows = (0..1000)
                .map(|i| resources[&format!("fixture.process.{i}")].clone())
                .collect::<Vec<_>>();
            let bytes = serde_json::to_vec(&rows).map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("reference process provider exceeds16MiB".into());
            }
            let key = NEXT
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                    old.checked_add(1)
                })
                .map_err(|_| "reference callback generation exhausted")?;
            let state = Arc::new(ProcessProviderState {
                bytes,
                guard,
                source,
                retired: AtomicBool::new(false),
                in_flight: AtomicUsize::new(0),
            });
            let mut table = PROCESS_PROVIDERS
                .lock()
                .map_err(|_| "reference provider registry poisoned")?;
            if table.len() >= 16 {
                return Err("reference process-provider capacity exhausted".into());
            }
            table.insert(key, state.clone());
            Ok(Self { key, state })
        }
        fn retire(&self) {
            self.state.retired.store(true, Ordering::Release);
            if let Ok(mut table) = PROCESS_PROVIDERS.lock() {
                if table
                    .get(&self.key)
                    .is_some_and(|state| Arc::ptr_eq(state, &self.state))
                {
                    table.remove(&self.key);
                }
            }
        }
    }
    impl Drop for ProcessProviderOwner {
        fn drop(&mut self) {
            self.retire();
        }
    }
    struct ProcessProviderCall(Arc<ProcessProviderState>);
    impl ProcessProviderCall {
        fn enter(key: usize) -> Option<Self> {
            let state = {
                let table = PROCESS_PROVIDERS.lock().ok()?;
                let state = table.get(&key).cloned()?;
                // Retirement uses this SAME lookup lock. A cloned-but-not-yet
                // counted call cannot hide between admission and the fence.
                state.in_flight.fetch_add(1, Ordering::AcqRel);
                state
            };
            // No registry mutex is held over source/guard/Foundation callbacks.
            let call = Self(state);
            call.0.live().then_some(call)
        }
    }
    impl Drop for ProcessProviderCall {
        fn drop(&mut self) {
            self.0.in_flight.fetch_sub(1, Ordering::AcqRel);
        }
    }
    #[unsafe(export_name = "ruycast_f1_sdk_process_provider_v1")]
    extern "C" fn process_provider(context: *mut c_void) -> *mut c_void {
        let result = std::panic::catch_unwind(|| {
            let Some(call) = ProcessProviderCall::enter(context as usize) else {
                return std::ptr::null_mut();
            };
            // Guard8 must actually be worker-capable and source-qualified.
            // A main-only/retired guard refuses; no thread bypass is supplied.
            if !call.0.live() {
                return std::ptr::null_mut();
            }
            let data = objc2_foundation::NSData::with_bytes(&call.0.bytes);
            if !call.0.live() {
                return std::ptr::null_mut();
            }
            objc2::rc::Retained::into_raw(data).cast()
        });
        match result {
            Ok(pointer) => pointer,
            Err(payload) => {
                std::mem::forget(payload);
                std::ptr::null_mut()
            }
        }
    }
    #[unsafe(export_name = "ruycast_f1_sdk_reply_v1")]
    extern "C" fn reply(context: *mut c_void, bytes: *const c_void, count: isize) {
        let outcome = std::panic::catch_unwind(|| {
            let key = context as usize;
            let pending = PENDING
                .lock()
                .ok()
                .and_then(|table| table.get(&key).cloned());
            let Some(pending) = pending else { return };
            if let Some(source) = &pending.source {
                let Some(guard) = pending.guard else { return };
                if !pending.submission_started.load(Ordering::Acquire)
                    || source_main_call(Some(source), guard, 9).is_err()
                {
                    return;
                }
            }
            // Only an authorized reply can discharge a qualified submission,
            // even if its result is no longer wanted by the caller.
            if pending.reply_received.swap(true, Ordering::AcqRel) {
                return;
            }
            if pending.retired.load(Ordering::Acquire) {
                remove_completion(key, &pending);
                return;
            }
            let result = if count <= 0 || count as usize > MAX_FRAME_BYTES || bytes.is_null() {
                Err("reference reply storage/size refused".into())
            } else {
                // Borrowed by the source-qualified Swift callback for this call
                // only. Copy before returning; retain no foreign slice/pointer.
                Ok(
                    unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), count as usize) }
                        .to_vec(),
                )
            };
            let stored = if let Ok(mut slot) = pending.result.lock() {
                if slot.is_none() && !pending.retired.load(Ordering::Acquire) {
                    *slot = Some(result);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if stored && !pending.retired.load(Ordering::Acquire) {
                if let Some(wake) = &pending.wake {
                    wake();
                }
            }
        });
        if let Err(payload) = outcome {
            std::mem::forget(payload);
        }
    }
    pub struct Pending {
        key: usize,
        state: Arc<Completion>,
        _main: PhantomData<Rc<()>>,
    }
    impl Pending {
        pub fn take(&mut self) -> Result<Option<Value>> {
            let result = self
                .state
                .result
                .lock()
                .map_err(|_| "reference completion lock poisoned")?
                .take();
            if let Some(result) = result {
                remove_completion(self.key, &self.state);
                let bytes = result?;
                return serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| format!("reference response: {e}"));
            }
            Ok(None)
        }
    }
    impl Drop for Pending {
        fn drop(&mut self) {
            self.state.retired.store(true, Ordering::Release);
            // Timeout revokes business lookup authority but cannot free the
            // real issuer while a qualified Swift reply is still outstanding.
            // The existing 16-entry cap also bounds these retired contexts.
            if self.state.source.is_none()
                || !self.state.submission_started.load(Ordering::Acquire)
                || self.state.reply_received.load(Ordering::Acquire)
            {
                remove_completion(self.key, &self.state);
            }
        }
    }
    /// Held only on the actual reference main thread. Async pending replies use
    /// non-reused numeric contexts; timeout removes lookup authority, never
    /// frees a borrowed callback pointer or retargets a late reply to a new job.
    pub struct Bridge {
        functions: Functions,
        object: NonNull<c_void>,
        guard: Guard,
        source: Option<QualifiedSourceBox>,
        process_prepared: Cell<bool>,
        borrowed_box_released: bool,
        process_provider: Option<ProcessProviderOwner>,
        _main: PhantomData<Rc<()>>,
    }
    impl Bridge {
        /// Consume only the SDK's qualified table AFTER the real AppDelegate's
        /// DidFinishLaunching. A separate AppState constructor is not startup.
        pub fn borrow_qualified_delegate(binding: QualifiedFunctions) -> Result<Self> {
            let QualifiedFunctions { functions, guard, source, .. } = binding;
            source_main_call(Some(&source), guard, 11)?;
            let permit = SourceBoxPermit::acquire()?;
            unsafe { (functions.install_guard)(Some(guard)); }
            source_main_call(Some(&source), guard, 11)?;
            let object = NonNull::new(unsafe { (functions.borrow_delegate)() })
                .ok_or("original reference AppDelegate/state not ready")?;
            let bridge = Self { functions, object, guard, source: Some(QualifiedSourceBox { reader: source, _permit: permit }),
                process_prepared: Cell::new(false), borrowed_box_released: false, process_provider: None, _main: PhantomData };
            bridge.live(11)?;
            Ok(bridge)
        }
        /// Original PerformanceChecks' own headless AppState, limited to the
        /// existing CachedProcesses/ProcessSort fixture route. This creates no
        /// AppDelegate and supplies no startup, observer-drain or Ready proof.
        pub fn create_qualified_process_state(binding: QualifiedFunctions) -> Result<Self> {
            let QualifiedFunctions { functions, guard, source, .. } = binding;
            source_main_call(Some(&source), guard, 1)?;
            let permit = SourceBoxPermit::acquire()?;
            let root = source.qualified_profile()?;
            source_main_call(Some(&source), guard, 1)?;
            let provider = ProcessProviderOwner::register_source(guard, Some(source.clone()))?;
            unsafe { (functions.install_guard)(Some(guard)); }
            source_main_call(Some(&source), guard, 1)?;
            let object = NonNull::new(unsafe {
                (functions.create_with_process_provider)(root.as_ptr(), 1, 0,
                    Some(process_provider), provider.key as *mut c_void)
            }).ok_or("original headless reference process state refused")?;
            let bridge = Self { functions, object, guard, source: Some(QualifiedSourceBox { reader: source, _permit: permit }),
                process_prepared: Cell::new(false), borrowed_box_released: false, process_provider: Some(provider), _main: PhantomData };
            bridge.live(1)?;
            Ok(bridge)
        }
        /// # Safety
        /// Every function must belong to the exact qualified immutable bridge
        /// image and preserve the published C ABI/borrowed-buffer contract.
        /// A complete live SDK must precede the original AppDelegate creation.
        /// This function supplies no such qualification itself.
        pub unsafe fn borrow_actual_delegate(functions: Functions, guard: Guard) -> Result<Self> {
            if unsafe { libc::pthread_main_np() } != 1 || guard(11) != 1 {
                return Err("reference delegate not source-qualified on main thread".into());
            }
            unsafe {
                (functions.install_guard)(Some(guard));
            }
            let object = NonNull::new(unsafe { (functions.borrow_delegate)() })
                .ok_or("original reference AppDelegate/state not ready")?;
            Ok(Self {
                functions,
                object,
                guard,
                source: None,
                process_prepared: Cell::new(false), borrowed_box_released: false,
                process_provider: None,
                _main: PhantomData,
            })
        }
        /// # Safety
        /// Every function/guard must be the exact qualified live image. The
        /// root must be the source-owned private fixture. Complete SDK guards
        /// precede AppState construction; guard8 must separately permit its
        /// worker data callback. The SDK image and physical NSData ARC cleanup
        /// must outlive every in-flight callback, including after retirement.
        /// This constructor loads nothing and supplies none of that authority.
        pub unsafe fn create_with_process_provider(
            functions: Functions,
            guard: Guard,
            root: &CStr,
            headless: bool,
            native_view: bool,
        ) -> Result<Self> {
            if unsafe { libc::pthread_main_np() } != 1 || guard(1) != 1 {
                return Err("reference constructor not source-qualified on main thread".into());
            }
            let provider = ProcessProviderOwner::register(guard)?;
            unsafe {
                (functions.install_guard)(Some(guard));
            }
            let object = NonNull::new(unsafe {
                (functions.create_with_process_provider)(
                    root.as_ptr(),
                    i32::from(headless),
                    i32::from(native_view),
                    Some(process_provider),
                    provider.key as *mut c_void,
                )
            })
            .ok_or("original reference process-provider construction refused")?;
            Ok(Self {
                functions,
                object,
                guard,
                source: None,
                process_prepared: Cell::new(false), borrowed_box_released: false,
                process_provider: Some(provider),
                _main: PhantomData,
            })
        }
        pub fn cancel_system_search(&self) -> Result<()> {
            self.live(8)?;
            unsafe {
                (self.functions.cancel_system_search)(self.object.as_ptr());
            }
            Ok(())
        }
        fn live(&self, operation: u32) -> Result<()> {
            source_main_call(self.source.as_ref().map(|owner| &owner.reader), self.guard, operation)
        }
        pub fn query(&self, value: &str) -> Result<()> {
            self.live(3)?;
            let result = unsafe {
                (self.functions.query)(
                    self.object.as_ptr(),
                    value.as_ptr().cast(),
                    value.len() as isize,
                )
            };
            if result != 1 {
                return Err("reference production query refused".into());
            }
            Ok(())
        }
        pub fn mode(&self, value: &str) -> Result<()> {
            self.live(4)?;
            let result = unsafe {
                (self.functions.mode)(
                    self.object.as_ptr(),
                    value.as_ptr().cast(),
                    value.len() as isize,
                )
            };
            if result != 1 {
                return Err("reference production mode refused".into());
            }
            if value != "processes" {
                self.process_prepared.set(false);
            }
            Ok(())
        }
        pub fn select(&self, index: isize) -> Result<()> {
            self.live(5)?;
            unsafe {
                (self.functions.select)(self.object.as_ptr(), index);
            }
            Ok(())
        }
        pub fn actions(&self, opened: bool) -> Result<()> {
            self.live(6)?;
            unsafe {
                (self.functions.actions)(self.object.as_ptr(), i32::from(opened));
            }
            Ok(())
        }
        fn stage_catalog(&self, size: usize, live: impl Fn() -> bool) -> Result<()> {
            let before = self.snapshot()?;
            let expected = fixture::configuration(&before["config"], size)?;
            let bytes = serde_json::to_vec(&expected["commands"]).map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("reference catalog exceeds existing16MiB bridge limit".into());
            }
            let count = isize::try_from(bytes.len()).map_err(|_| "reference catalog length overflow")?;
            self.live(8)?;
            if !live() {
                return Err("reference catalog request expired before actual publication".into());
            }
            if unsafe {
                (self.functions.set_commands)(self.object.as_ptr(), bytes.as_ptr().cast(), count)
            } != 1 {
                return Err("reference original config.commands setter refused".into());
            }
            // Compare the complete actual configuration. Do not manufacture
            // a successful snapshot or normalize the original command shape.
            if self.snapshot()?["config"] != expected {
                return Err("reference actual catalog publication differs from its complete fixture".into());
            }
            Ok(())
        }
        pub fn snapshot(&self) -> Result<Value> {
            self.live(7)?;
            // Swift passRetained(NSData) transfers exactly one +1 here.
            let data = unsafe {
                objc2::rc::Retained::from_raw((self.functions.snapshot)(self.object.as_ptr()))
            }
            .ok_or("reference complete snapshot absent")?;
            if data.length() > MAX_FRAME_BYTES {
                return Err("reference snapshot exceeds64MiB".into());
            }
            let bytes = data.to_vec();
            serde_json::from_slice(&bytes).map_err(|e| format!("reference complete snapshot: {e}"))
        }
        fn count(&self, getter: unsafe extern "C" fn(*mut c_void) -> isize) -> Result<usize> {
            self.live(7)?;
            usize::try_from(unsafe { getter(self.object.as_ptr()) })
                .map_err(|_| "reference count getter returned negative result".into())
        }
        pub fn result_count(&self) -> Result<usize> {
            self.count(self.functions.result_count)
        }
        pub fn visible_count(&self) -> Result<usize> {
            self.count(self.functions.visible_count)
        }
        pub fn selected_id_count(&self) -> Result<usize> {
            self.count(self.functions.selected_id_count)
        }
        pub fn apply_system_results(
            &self,
            commands: &[Value],
            resources: &serde_json::Map<String, Value>,
            preserve: bool,
        ) -> Result<()> {
            self.live(8)?;
            let commands = serde_json::to_vec(commands).map_err(|e| e.to_string())?;
            let resources = serde_json::to_vec(resources).map_err(|e| e.to_string())?;
            if commands.len() > 16 * 1024 * 1024 || resources.len() > 16 * 1024 * 1024 {
                return Err(
                    "reference process input exceeds existing16MiB per-buffer limit".into(),
                );
            }
            let commands_len = isize::try_from(commands.len())
                .map_err(|_| "reference commands length overflow")?;
            let resources_len = isize::try_from(resources.len())
                .map_err(|_| "reference resources length overflow")?;
            self.live(8)?;
            if unsafe {
                (self.functions.apply_system_results)(
                    self.object.as_ptr(),
                    commands.as_ptr().cast(),
                    commands_len,
                    resources.as_ptr().cast(),
                    resources_len,
                    i32::from(preserve),
                )
            } != 1
            {
                return Err("reference applySystemResults forwarder refused".into());
            }
            Ok(())
        }
        pub fn process_sort(&self) -> Result<ProcessSort> {
            self.live(7)?;
            ProcessSort::try_from(unsafe { (self.functions.process_sort)(self.object.as_ptr()) })
        }
        pub fn set_process_sort(&self, sort: ProcessSort) -> Result<()> {
            self.live(8)?;
            if unsafe { (self.functions.set_process_sort)(self.object.as_ptr(), sort as i32) } != 1
            {
                return Err("reference setProcessSort forwarder refused".into());
            }
            Ok(())
        }
        pub fn process_snapshot(&self) -> Result<Value> {
            self.live(7)?;
            // Swift passRetained transfers one +1; parsing stays outside timers.
            let data = unsafe {
                objc2::rc::Retained::from_raw((self.functions.process_snapshot)(
                    self.object.as_ptr(),
                ))
            }
            .ok_or("reference process-owner snapshot absent")?;
            if data.length() > MAX_FRAME_BYTES {
                return Err("reference process snapshot exceeds64MiB".into());
            }
            serde_json::from_slice(&data.to_vec())
                .map_err(|e| format!("reference process snapshot: {e}"))
        }
        /// The actual ReferenceHost must first enter processes mode and drain
        /// the original mode-triggered asynchronous processDetails owner. This
        /// method supplies no such authority and is not a Host/Ready shortcut.
        pub fn prepare_processes1000(&self) -> Result<()> {
            let state = self.snapshot()?;
            process_owner_ready(&state)?;
            let uid = unsafe { libc::getuid() };
            if self.process_prepared.get() {
                return validate_process_snapshot(&self.process_snapshot()?, uid);
            }
            self.set_process_sort(ProcessSort::Cpu)?;
            self.apply_system_results(
                &fixture::process_commands(),
                &fixture::process_resources(uid),
                false,
            )?;
            self.select(0)?;
            let actual = self.process_snapshot()?;
            validate_process_snapshot(&actual, uid)?;
            if actual["processSort"] != process_sort_name(ProcessSort::Cpu)
                || actual["selectedID"] != "fixture.process.96"
            {
                return Err("reference initial process order/selection refused".into());
            }
            self.process_prepared.set(true);
            Ok(())
        }
        /// Component result only. The future concrete Host still owes real
        /// post-operation owner drain/diagnostics before it can make Completed.
        /// Timing includes the necessary C bridge/guard overhead, not JSON.
        pub fn measure_processes1000(&self, lane: Functional) -> Result<(u64, Value)> {
            if !self.process_prepared.get() {
                return Err("reference process fixture was not prepared".into());
            }
            process_owner_ready(&self.snapshot()?)?;
            let before = self.process_snapshot()?;
            validate_process_snapshot(&before, unsafe { libc::getuid() })?;
            let selected = before["selectedID"].clone();
            let start = Instant::now();
            let visible_count = match lane {
                Functional::CachedProcesses1000 => Some(self.visible_count()?),
                Functional::ProcessSort1000 => {
                    let next = if self.process_sort()? == ProcessSort::Cpu {
                        ProcessSort::Memory
                    } else {
                        ProcessSort::Cpu
                    };
                    self.set_process_sort(next)?;
                    None
                }
                _ => return Err("reference component is not this functional lane".into()),
            };
            let selected_id_count = self.selected_id_count()?;
            let elapsed_ns = u64::try_from(start.elapsed().as_nanos())
                .map_err(|_| "reference process timer overflow")?;
            process_owner_ready(&self.snapshot()?)?;
            let actual = self.process_snapshot()?;
            validate_process_snapshot(&actual, unsafe { libc::getuid() })?;
            if actual["selectedID"] != selected {
                return Err("reference process sort retargeted selection".into());
            }
            let expected_count = selected
                .as_str()
                .ok_or("reference selected process ID absent")?
                .chars()
                .count();
            if visible_count.is_some_and(|count| count != 1000)
                || selected_id_count != expected_count
            {
                return Err(
                    "reference cached process getter disagrees with actual owner state".into(),
                );
            }
            if lane == Functional::CachedProcesses1000 {
                if actual != before {
                    return Err("reference cached process read changed owner state".into());
                }
            } else {
                let expected = if process_sort_value(&before)? == ProcessSort::Cpu {
                    ProcessSort::Memory
                } else {
                    ProcessSort::Cpu
                };
                if process_sort_value(&actual)? != expected {
                    return Err("reference process sort did not perform the original toggle".into());
                }
            }
            Ok((
                elapsed_ns,
                json!({"result":{"visible_count":visible_count,"selected_id_count":selected_id_count},"state":actual}),
            ))
        }
        pub fn filtered_count(&self) -> Result<usize> {
            self.count(self.functions.filtered_count)
        }
        /// +1 result transfer only. Byte length/copy/parsing belong after the
        /// operation timer; NSData bridging overhead remains in that timer.
        pub fn config_data(&self) -> Result<objc2::rc::Retained<objc2_foundation::NSData>> {
            self.live(7)?;
            unsafe {
                objc2::rc::Retained::from_raw((self.functions.config_data)(self.object.as_ptr()))
            }
            .ok_or_else(|| "reference production config read refused".into())
        }
        pub fn encode_clips(&self) -> Result<objc2::rc::Retained<objc2_foundation::NSData>> {
            self.live(7)?;
            unsafe {
                objc2::rc::Retained::from_raw((self.functions.encode_clips)(self.object.as_ptr()))
            }
            .ok_or_else(|| "reference production clipboard encoding refused".into())
        }
        pub fn record_timing(&self, name: &str, milliseconds: f64) -> Result<()> {
            self.live(8)?;
            if name.len() > 16 * 1024 * 1024 {
                return Err("reference metric name exceeds bridge transport limit".into());
            }
            if unsafe {
                (self.functions.record_timing)(
                    self.object.as_ptr(),
                    name.as_ptr().cast(),
                    name.len() as isize,
                    milliseconds,
                )
            } != 1
            {
                return Err("reference timing forwarder refused".into());
            }
            Ok(())
        }
        pub fn record_recency(&self, id: &str, epoch: f64) -> Result<()> {
            self.live(8)?;
            if id.len() > 16 * 1024 * 1024 || !epoch.is_finite() {
                return Err("reference recency transport refused".into());
            }
            if unsafe {
                (self.functions.record_recency)(
                    self.object.as_ptr(),
                    id.as_ptr().cast(),
                    id.len() as isize,
                    epoch,
                )
            } != 1
            {
                return Err("reference production recency save refused".into());
            }
            Ok(())
        }
        pub fn call(&self, request: &Value) -> Result<Pending> {
            self.call_notifying(request, None)
        }
        pub fn call_notifying(&self, request: &Value, wake: Option<Wake>) -> Result<Pending> {
            self.call_live_notifying(request, wake, || true)
        }
        fn call_live_notifying(
            &self,
            request: &Value,
            wake: Option<Wake>,
            live: impl Fn() -> bool,
        ) -> Result<Pending> {
            self.live(8)?;
            let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("reference request exceeds existing16MiB bridge limit".into());
            }
            let state = Arc::new(Completion {
                result: Mutex::new(None),
                wake,
                source: self.source.as_ref().map(|owner| owner.reader.clone()),
                guard: self.source.as_ref().map(|_| self.guard),
                retired: AtomicBool::new(false),
                reply_received: AtomicBool::new(false),
                submission_started: AtomicBool::new(false),
            });
            let key = NEXT
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                    old.checked_add(1)
                })
                .map_err(|_| "reference callback generation exhausted")?;
            {
                let mut table = PENDING
                    .lock()
                    .map_err(|_| "reference callback registry poisoned")?;
                if table.len() >= REPLY_CONTEXT_LIMIT {
                    return Err("reference callback capacity exhausted".into());
                }
                table.insert(key, state.clone());
            }
            let pending = Pending { key, state, _main: PhantomData };
            if !live() {
                return Err("reference RPC expired before original handle admission".into());
            }
            if self.source.is_some() { self.live(8)?; }
            // A rejected/pre-admission request has no future callback to hold.
            // Mark before the call because handle may reply synchronously.
            pending.state.start_submission(live())?;
            let accepted = unsafe {
                (self.functions.call)(
                    self.object.as_ptr(),
                    bytes.as_ptr().cast(),
                    bytes.len() as isize,
                    reply,
                    key as *mut c_void,
                )
            } == 1;
            if !accepted {
                pending.state.submission_started.store(false, Ordering::Release);
                return Err("reference production handle request refused".into());
            }
            Ok(pending)
        }
        pub fn content_view(&self) -> Result<NonNull<c_void>> {
            self.live(10)?;
            NonNull::new(unsafe { (self.functions.content_view)(self.object.as_ptr()) })
                .ok_or_else(|| "original reference palette content view absent".into())
        }
        /// Existing qualified getter, restricted to the actual retained issuer.
        /// A raw historical Bridge or unrelated SourceReader cannot issue a
        /// field-root witness through this entry.
        pub fn content_view_for(&self,issuer:&dyn SourceCodeReader)->Result<NonNull<c_void>>{
            let source=self.source.as_ref().ok_or("Reference field requires a qualified Bridge")?;
            if self.process_provider.is_some(){return Err("Reference field requires the borrowed actual AppDelegate, not a headless process owner".into());}
            if !std::ptr::addr_eq(source.reader.as_ref(),issuer){return Err("Reference field Bridge issuer differs".into());}
            self.content_view()
        }
        /// Borrow the exact live box identity for the source issuer's layout
        /// observation. This does not transfer ownership or allow release.
        pub fn borrowed_delegate_box_for(&self, issuer: &dyn SourceCodeReader) -> Result<NonNull<c_void>> {
            let source = self.source.as_ref().ok_or("Reference box requires a qualified Bridge")?;
            if self.process_provider.is_some() || !std::ptr::addr_eq(source.reader.as_ref(), issuer) {
                return Err("Reference borrowed box issuer or role differs".into());
            }
            self.live(11)?;
            Ok(self.object)
        }
        fn fence_local_callbacks(&self) -> Result<()> {
            // Close new provider admission FIRST, then inspect the count that
            // was incremented under the same lookup lock before any callback.
            if let Some(provider) = &self.process_provider {
                provider.retire();
                if provider.state.in_flight.load(Ordering::Acquire) != 0 {
                    return Err("reference process provider still has in-flight calls".into());
                }
            }
            verify_reply_contexts_drained()
        }
        fn release_borrowed_field_box(&mut self) -> Result<()> {
            if self.borrowed_box_released || self.process_provider.is_some() {
                return Err("reference field box release role/replay differs".into());
            }
            self.fence_local_callbacks()?;
            let source = self.source.as_ref().ok_or("reference field box issuer absent")?;
            unsafe {
                source.reader.release_borrowed_field_box(self.object, self.functions.drop_box)?;
            }
            self.borrowed_box_released = true;
            Ok(())
        }
        pub fn close(mut self) -> Result<()> {
            if self.source.is_some() {
                self.fence_local_callbacks()?;
                self.live(2)?;
                // A dynamic authorization callback may have reentered; inspect
                // again after it, with no mutex held over the foreign release.
                self.fence_local_callbacks()?;
                let provider = self.process_provider.take();
                let source = self.source.take().expect("qualified source exists");
                source.finish(true, || unsafe {
                    (self.functions.drop_box)(self.object.as_ptr());
                });
                drop(provider);
                std::mem::forget(self);
                return Ok(());
            }
            // Historical raw unsafe route: its caller owes external custody.
            self.live(2)?;
            if self.process_provider.as_ref().is_some_and(|p| p.state.in_flight.load(Ordering::Acquire) != 0) {
                return Err("reference process provider still has in-flight calls".into());
            }
            if !PENDING.lock().map_err(|_| "reference callback registry poisoned")?.is_empty() {
                return Err("reference callbacks not drained before close".into());
            }
            if let Some(provider) = self.process_provider.take() { provider.retire(); }
            unsafe { (self.functions.drop_box)(self.object.as_ptr()); }
            std::mem::forget(self);
            Ok(())
        }
    }
    impl Drop for Bridge {
        fn drop(&mut self) {
            if self.borrowed_box_released {
                return;
            }
            if self.source.is_some() {
                let idle = self.fence_local_callbacks().is_ok();
                let authorized = idle && self.live(2).is_ok();
                let idle_after_guard = authorized && self.fence_local_callbacks().is_ok();
                let source = self.source.take().expect("qualified source exists");
                source.finish(idle_after_guard, || unsafe {
                    (self.functions.drop_box)(self.object.as_ptr());
                });
                return;
            }
            if let Some(provider) = &self.process_provider { provider.retire(); }
            if self.live(2).is_ok() {
                unsafe { (self.functions.drop_box)(self.object.as_ptr()); }
            }
        }
    }
    #[derive(Clone)]
    enum Operation {
        Verify,
        Snapshot,
        Query(QueryStage, String),
        NativeAction(UiAction),
        PrepareFunctional(Functional),
        Functional(Functional),
        Drain,
        Shutdown,
        StageCatalog(usize),
        ProcessObservation,
        WireCall(Value),
    }
    enum OwnerReply {
        Unit,
        Value(Value),
        Measured(u64, Value),
    }
    struct OwnerRequest {
        operation: Operation,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
        response: std::sync::mpsc::SyncSender<Result<OwnerReply>>,
    }
    impl OwnerRequest {
        fn live(&self) -> bool {
            !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.deadline
        }
    }
    struct RequestWait {
        cancelled: Arc<AtomicBool>,
        wake: Wake,
    }
    impl RequestWait {
        fn cancel(&self) {
            if self.cancelled.swap(true, Ordering::AcqRel) {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)()));
            if let Err(payload) = result {
                std::mem::forget(payload);
            }
        }
    }
    impl Drop for RequestWait {
        fn drop(&mut self) {
            self.cancel();
        }
    }
    const FIELD_CLEANUP_GRACE: Duration = Duration::from_secs(5);
    struct ProcessPrelude {
        epoch: u64,
        config: Value,
    }
    fn process_epoch(epoch: u64) -> Result<u64> {
        if !(511..=9_007_199_254_740_991).contains(&epoch) {
            return Err("reference process fixture epoch is not an exact supported timestamp".into());
        }
        Ok(epoch)
    }
    fn process_prelude_state(state: &Value, expected_config: &Value) -> Result<()> {
        if state["config"] != *expected_config || state["mode"] != "commands"
            || state["query"] != "m" || state["paletteVisible"] != false
            || state["actionsOpen"] != false || state["busy"] != false
            || state["clipboardSearchBusy"] != false {
            return Err("reference process prelude is not the original hidden catalog10k/querym owner".into());
        }
        Ok(())
    }
    fn process_ranking_bytes(bytes: &[u8], epoch: u64) -> Result<()> {
        process_epoch(epoch)?;
        if bytes.len() > 1_000_000 { return Err("reference process ranking exceeds original store bound".into()); }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| "reference process ranking is malformed")?;
        let object = value.as_object().ok_or("reference process ranking is not an object")?;
        let entries = value["entries"].as_object().ok_or("reference process ranking entries absent")?;
        if object.len() != 2 || value["schema"] != 1 || entries.len() != 512 {
            return Err("reference process ranking schema/complete key count differs".into());
        }
        for i in 0..512 {
            if entries.get(&format!("fixture.{i}")).and_then(Value::as_f64)
                != Some((epoch - (511 - i)) as f64) {
                return Err("reference process ranking fixture ID/timestamp differs".into());
            }
        }
        Ok(())
    }
    fn process_ranking_reply(response: &Value, count: usize) -> Result<()> {
        if response != &json!({"ok":true,"result":{"entries":count,"limit":512,"retentionDays":90}}) {
            return Err("reference original ranking reset/summary refused or differs".into());
        }
        Ok(())
    }
    fn prepared_process_owner(
        state: &Value, config: &Value, process: &Value, ranking: &[u8], epoch: u64, uid: u32,
    ) -> Result<()> {
        process_owner_ready(state)?;
        if state["config"] != *config {
            return Err("reference process fixture displaced catalog10k configuration".into());
        }
        if !matches!(process_sort_value(process)?, ProcessSort::Cpu | ProcessSort::Memory) {
            return Err("reference process fixture is outside its original CPU/Memory sequence".into());
        }
        // Both actual CPU and Memory order are admissible. This is validation
        // only: never reset/rebuild/restore the original sort or selected row.
        validate_process_snapshot(process, uid)?;
        process_ranking_bytes(ranking, epoch)
    }
    #[derive(Clone,Copy)]
    enum PendingPurpose {
        ProcessRankingReset,
        ProcessRankingVerify,
        ProcessPreparation,
        WireCall,
        FieldReset(u64),
        FieldMount(u64),
        FieldCompletion(u64),
        FieldCancel { epoch: u64, phase: FieldCancelPhase },
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum FieldCancelPhase {
        Reset,
        Mount,
        Completion,
    }
    struct OwnerPending {
        request: OwnerRequest,
        reply: Pending,
        purpose: PendingPurpose,
        field_cancel:Option<(String,Instant)>,
    }
    static NEXT_FIELD_REQUEST:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
    fn field_trace(value:&Value,purpose:PendingPurpose)->Result<bool>{
        let reset=matches!(purpose,PendingPurpose::FieldReset(_)|PendingPurpose::FieldCancel { phase: FieldCancelPhase::Reset, .. });
        let settling=matches!(purpose,PendingPurpose::FieldCancel { phase: FieldCancelPhase::Completion, .. });
        let mount_cancel=matches!(purpose,PendingPurpose::FieldCancel { phase: FieldCancelPhase::Mount, .. });
        let mounting=mount_cancel||matches!(purpose,PendingPurpose::FieldMount(_));
        if !reset&&!mounting&&!settling&&!matches!(purpose,PendingPurpose::FieldCompletion(_)){return Err("Wrong original field trace phase".into());}
        if value["ok"]!=true||value["result"]["schema"]!=2||value["result"]["sampleLimitPerStage"]!=256{
            return Err("Original field trace reply/schema differs".into());
        }
        let trace=&value["result"];
        let active=trace["activeIntervals"].as_u64().ok_or("Original field trace active count absent")?;
        let stages=trace["stages"].as_object().ok_or("Original field trace stages absent")?;
        for name in ["search.model","input.model","input.reconcile","open.ready","host.drawCallback","hotkey.handler"]{
            let row=stages.get(name).ok_or("Original field trace stage absent")?;
            let count=row["count"].as_u64().ok_or("Original field trace stage count absent")?;
            let retained=row["retainedSamples"].as_u64().ok_or("Original field trace samples absent")?;
            let cancelled=row["cancelled"].as_u64().ok_or("Original field trace cancellation count absent")?;
            if reset&&(count!=0||retained!=0||cancelled!=0){return Err("Original input baseline was not reset".into());}
            if mounting&&matches!(name,"input.model"|"input.reconcile")&&(count!=0||retained!=0||cancelled!=0){return Err("Original input effect appeared before field mount completed".into());}
            let valid_cancel = settling && name == "input.reconcile" && count == 0 && cancelled == 1;
            if matches!(name,"input.model"|"input.reconcile")&&((cancelled!=0&&!valid_cancel)||count>1||retained!=count){return Err("Original field interval superseded/cancelled or repeated".into());}
        }
        if reset||mounting{if active!=0{return Err("Original input baseline has active spans".into());}return Ok(mount_cancel);}
        if trace["stages"]["input.model"]["count"]!=1{return Err("Original field delegate model effect was not measured once".into());}
        let reconciled = trace["stages"]["input.reconcile"]["count"] == 1
            || settling && trace["stages"]["input.reconcile"]["cancelled"] == 1;
        if !reconciled && active == 0 {
            return Err("Original field reconciliation interval disappeared without completion".into());
        }
        Ok(reconciled && active == 0)
    }
    fn field_cancel_trace(
        value: &Value,
        epoch: u64,
        phase: FieldCancelPhase,
        delegate_not_entered: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        let phase = if phase == FieldCancelPhase::Completion
            && value["result"]["stages"]["input.model"]["count"] == 0
            && delegate_not_entered()?
        {
            FieldCancelPhase::Mount
        } else {
            phase
        };
        field_trace(value, PendingPurpose::FieldCancel { epoch, phase })
    }
    /// Concrete existing-Host adapter. Protocol IO stays on its worker, while
    /// every actual Bridge/AppState access is delivered to ReferenceOwner.
    /// Route implementation is not independent admission; the set stays empty.
    pub struct ReferenceHost {
        requests: std::sync::mpsc::SyncSender<OwnerRequest>,
        wake: Wake,
        stopped: Arc<AtomicBool>,
        bindings: ReadyBindings,
        admitted: BTreeSet<String>,
        field_probe: bool,
    }
    /// Main-only production coordinator. Install poll on the already-qualified
    /// existing main_loop source; this type starts no thread, SDK or run loop.
    pub struct ReferenceOwner {
        bridge: Bridge,
        // The original PerformanceChecks' own headless state is used only by
        // these two process lanes; actual UI/startup stays on the delegate.
        process_bridge: Option<Bridge>,
        process_prelude: Option<ProcessPrelude>,
        requests: std::sync::mpsc::Receiver<OwnerRequest>,
        wake: Wake,
        stopped: Arc<AtomicBool>,
        pending: Option<OwnerPending>,
        field_poisoned:Cell<bool>,
        process_publication_fenced: bool,
    }
    pub fn coordinator(
        bridge: Bridge,
        bindings: ReadyBindings,
        wake: Wake,
        stopped: Arc<AtomicBool>,
        field_probe: bool,
    ) -> Result<(ReferenceHost, ReferenceOwner)> {
        // Shape validation is not source or SDK authority.
        bindings.envelope()?;
        if stopped.load(Ordering::Acquire) {
            return Err("reference coordinator stopped before setup".into());
        }
        let (requests, receiver) = std::sync::mpsc::sync_channel(1);
        Ok((
            ReferenceHost {
                requests,
                wake: wake.clone(),
                stopped: stopped.clone(),
                bindings,
                admitted: BTreeSet::new(),
                field_probe,
            },
            ReferenceOwner {
                bridge,
                process_bridge: None,
                process_prelude: None,
                requests: receiver,
                wake,
                stopped,
                pending: None,
                field_poisoned:Cell::new(false),
                process_publication_fenced: false,
            },
        ))
    }
    impl ReferenceHost {
        fn call(&self, operation: Operation, deadline: Instant) -> Result<OwnerReply> {
            if self.field_probe && !matches!(
                operation,
                Operation::Verify | Operation::Snapshot
                    | Operation::Query(QueryStage::Input | QueryStage::Reconciliation, _)
            ) {
                return Err("reference field probe operation is outside the retained field route".into());
            }
            if self.stopped.load(Ordering::Acquire) {
                return Err("reference coordinator stopped".into());
            }
            if Instant::now() >= deadline {
                return Err("reference request expired before admission".into());
            }
            let (response, reply) = std::sync::mpsc::sync_channel(1);
            let cancelled = Arc::new(AtomicBool::new(false));
            let wait = RequestWait {
                cancelled: cancelled.clone(),
                wake: self.wake.clone(),
            };
            let field_input = matches!(operation, Operation::Query(QueryStage::Input | QueryStage::Reconciliation, _));
            self.requests
                .try_send(OwnerRequest {
                    operation,
                    deadline,
                    cancelled,
                    response,
                })
                .map_err(|_| "reference coordinator busy or stopped")?;
            (self.wake)();
            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) => remaining,
                None if field_input => Duration::ZERO,
                None => return Err("reference request expired during wake; publication may be unknown".into()),
            };
            let result = reply.recv_timeout(remaining);
            if field_input && matches!(result, Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
                wait.cancel();
                // Keep the wire worker alive while the owner confirms cleanup.
                // A reply during this grace period cannot turn timeout into success.
                if let Some(until) = deadline.checked_add(FIELD_CLEANUP_GRACE) {
                    let _ = reply.recv_timeout(until.saturating_duration_since(Instant::now()));
                }
            }
            let result = result
                .map_err(|_| "reference owner request expired; publication may be unknown")?;
            if Instant::now() >= deadline || self.stopped.load(Ordering::Acquire) {
                return Err(
                    "reference reply arrived outside its live deadline; publication may be unknown"
                        .into(),
                );
            }
            result
        }
        fn unit(&self, operation: Operation, deadline: Instant) -> Result<()> {
            match self.call(operation, deadline)? {
                OwnerReply::Unit => Ok(()),
                _ => Err("reference owner returned another response kind".into()),
            }
        }
        fn value(&self, operation: Operation, deadline: Instant) -> Result<Value> {
            match self.call(operation, deadline)? {
                OwnerReply::Value(v) => Ok(v),
                _ => Err("reference owner returned another response kind".into()),
            }
        }
        pub fn process_observation(&self, deadline: Instant) -> Result<Value> {
            self.value(Operation::ProcessObservation, deadline)
        }
    }
    impl Drop for ReferenceHost {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            (self.wake)();
        }
    }
    impl Host for ReferenceHost {
        fn verify_live(&mut self) -> Result<()> {
            self.unit(Operation::Verify, Instant::now() + Duration::from_secs(5))
        }
        fn bindings(&self) -> &ReadyBindings {
            &self.bindings
        }
        fn admitted_routes(&self) -> &BTreeSet<String> {
            &self.admitted
        }
        fn stage_catalog(&mut self, size: usize, deadline: Instant) -> Result<()> {
            self.unit(Operation::StageCatalog(size), deadline)
        }
        fn query(&mut self, stage: QueryStage, query: &str, deadline: Instant) -> Result<()> {
            self.unit(Operation::Query(stage, query.into()), deadline)
        }
        fn snapshot(&mut self, deadline: Instant) -> Result<Value> {
            self.value(Operation::Snapshot, deadline)
        }
        fn wire_call(&mut self, request: Value, deadline: Instant) -> Result<Value> {
            self.value(Operation::WireCall(request), deadline)
        }
        fn native_action(&mut self, action: UiAction, deadline: Instant) -> Result<Value> {
            self.value(Operation::NativeAction(action), deadline)
        }
        fn prepare_functional(&mut self, lane: Functional, deadline: Instant) -> Result<()> {
            self.unit(Operation::PrepareFunctional(lane), deadline)
        }
        fn functional(&mut self, lane: Functional, deadline: Instant) -> Result<Completed> {
            // Unsettled original async owners must not contaminate the timer.
            self.drain(deadline)?;
            let (elapsed_ns, output) = match self.call(Operation::Functional(lane), deadline)? {
                OwnerReply::Measured(elapsed, output) => (elapsed, output),
                _ => return Err("reference owner did not measure this operation".into()),
            };
            // A scoped old-provider fence is not whole-owner/SDK drain.
            let diagnostics = self.drain(deadline)?;
            Ok(Completed {
                elapsed_ns,
                output,
                diagnostics,
            })
        }
        fn startup(&mut self, _: Startup, _: Instant) -> Result<Completed> {
            Err("reference startup owner route is not qualified".into())
        }
        fn lifecycle_10000(&mut self, _: Phase, _: Instant) -> Result<Completed> {
            Err("reference complete lifecycle owner route is not qualified".into())
        }
        fn idle_60seconds(&mut self, _: Instant) -> Result<Completed> {
            Err("reference idle ownership route is not qualified".into())
        }
        fn copies_allocations_input(&mut self, _: Instant) -> Result<Completed> {
            Err("reference input allocation owner route is not qualified".into())
        }
        fn artifact(&mut self, _: Artifact, _: Instant) -> Result<Completed> {
            Err("reference artifact source owner is not qualified".into())
        }
        fn build(&mut self, _: Build, _: Instant) -> Result<Completed> {
            Err("reference build source owner is not qualified".into())
        }
        fn drain(&mut self, deadline: Instant) -> Result<Value> {
            self.value(Operation::Drain, deadline)
        }
        fn shutdown(&mut self, deadline: Instant) -> Result<()> {
            self.unit(Operation::Shutdown, deadline)
        }
    }
    impl ReferenceOwner {
        fn start_field_input(&mut self,request:OwnerRequest){
            let result=(||{
                self.check(&request)?;
                if self.field_poisoned.get(){return Err("Original field owner quarantined after unconfirmed cleanup".into());}
                if !matches!(request.operation,Operation::Query(QueryStage::Input|QueryStage::Reconciliation,_))||self.bridge.source.is_none(){return Err("Original field requires the qualified owner input route".into());}
                let epoch=NEXT_FIELD_REQUEST.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_add(1)).map_err(|_|"Original field request epochs exhausted")?;
                let reply=self.bridge.call_live_notifying(&json!({"op":"performance.reset"}),Some(self.wake.clone()),||request.live()&&!self.stopped.load(Ordering::Acquire))?;
                Ok((epoch,reply))
            })();
            match result{Ok((epoch,reply))=>self.pending=Some(OwnerPending{request,reply,purpose:PendingPurpose::FieldReset(epoch),field_cancel:None}),Err(error)=>self.respond(request,Err(error))}
        }
        fn field_step(&self,request:&OwnerRequest,epoch:u64,purpose:&mut PendingPurpose,value:Value)->Result<Option<(Pending,PendingPurpose)>>{
            self.check(request)?;
            let source=&self.bridge.source.as_ref().ok_or("Original field source absent")?.reader;
            let next=if matches!(purpose,PendingPurpose::FieldReset(_)|PendingPurpose::FieldMount(_)){
                field_trace(&value,*purpose)?;
                let Operation::Query(QueryStage::Input|QueryStage::Reconciliation,query)=&request.operation else{return Err("Original field operation changed".into());};
                // A failed entry may already have acquired original custody.
                // Mark the phase before calling into the source.
                *purpose = PendingPurpose::FieldCompletion(epoch);
                if !source.begin_field_input(&self.bridge,query,epoch)? {
                    *purpose = PendingPurpose::FieldMount(epoch);
                }
                *purpose
            }else{
                if field_trace(&value,*purpose)?{
                    source.verify_field_input(&self.bridge,epoch)?;
                    self.check(request)?;
                    source.retire_field_input(&self.bridge,epoch,true)?;
                    // The reply boundary still checks the deadline. A late
                    // reply must not restart cancellation of a retired field.
                    return Ok(None);
                }
                PendingPurpose::FieldCompletion(epoch)
            };
            if !matches!(next,PendingPurpose::FieldMount(_)){source.verify_field_input(&self.bridge,epoch)?;}
            self.check(request)?;
            self.bridge.call_live_notifying(&json!({"op":"performance"}),Some(self.wake.clone()),||request.live()&&!self.stopped.load(Ordering::Acquire)).map(|reply|Some((reply,next)))
        }
        fn retire_failed_field(&self,purpose:PendingPurpose){
            if let PendingPurpose::FieldReset(epoch)|PendingPurpose::FieldMount(epoch)|PendingPurpose::FieldCompletion(epoch)|PendingPurpose::FieldCancel { epoch, .. }=purpose{
                self.field_poisoned.set(true);
                if let Some(source)=&self.bridge.source{let _=source.reader.retire_field_input(&self.bridge,epoch,false);}
            }
        }
        fn start_field_cancel(&mut self,request:OwnerRequest,purpose:PendingPurpose,error:String){
            let (epoch, phase) = match purpose {
                PendingPurpose::FieldReset(epoch) => (epoch, FieldCancelPhase::Reset),
                PendingPurpose::FieldMount(epoch) => (epoch, FieldCancelPhase::Mount),
                PendingPurpose::FieldCompletion(epoch) => (epoch, FieldCancelPhase::Completion),
                _ => {self.respond(request,Err(error));return;}
            };
            // Cleanup has its own finite owner lease after caller expiry. It
            // never admits a second request or reports the failed input as OK.
            let mut until = Instant::now() + FIELD_CLEANUP_GRACE;
            if let Some(limit) = request.deadline.checked_add(FIELD_CLEANUP_GRACE) {
                until = until.min(limit);
            }
            // Reset erases the trace and pending input ID. Once field entry
            // may have run, observe the existing interval instead.
            let op = if phase == FieldCancelPhase::Reset { "performance.reset" } else { "performance" };
            let observation=self.bridge.call_live_notifying(&json!({"op":op}),Some(self.wake.clone()),||Instant::now()<until&&!self.stopped.load(Ordering::Acquire));
            match observation{
                Ok(reply)=>self.pending=Some(OwnerPending{request,reply,purpose:PendingPurpose::FieldCancel { epoch, phase },field_cancel:Some((error,until))}),
                Err(cleanup)=>{self.retire_failed_field(purpose);self.respond(request,Err(format!("{error}; original input cancellation could not be confirmed: {cleanup}")));}
            }
        }
        fn process_bridge(&self) -> Result<&Bridge> {
            self.process_bridge.as_ref().ok_or("reference process headless owner absent".into())
        }
        fn ensure_process_bridge(&mut self) -> Result<()> {
            if self.process_bridge.is_none() {
                let source = self.bridge.source.as_ref()
                    .ok_or("reference headless process owner requires a sealed source issuer")?
                    .reader.clone();
                let functions = QualifiedFunctions::read(source, self.bridge.guard)?;
                self.process_bridge = Some(Bridge::create_qualified_process_state(functions)?);
            }
            Ok(())
        }
        fn process_call(&self, request: &OwnerRequest, value: &Value) -> Result<Pending> {
            self.check(request)?;
            self.process_bridge()?.call_live_notifying(value, Some(self.wake.clone()), || {
                request.live() && !self.stopped.load(Ordering::Acquire)
            })
        }
        fn verify_process_prelude(&self, request: &OwnerRequest, commands: bool) -> Result<()> {
            self.check(request)?;
            let bridge = self.process_bridge()?;
            let prelude = self.process_prelude.as_ref().ok_or("reference process prelude absent")?;
            let state = bridge.snapshot()?;
            if commands {
                process_prelude_state(&state, &prelude.config)?;
            }
            let source = &bridge.source.as_ref().ok_or("reference process source absent")?.reader;
            let ranking = source.qualified_process_ranking()?;
            if commands { process_ranking_bytes(&ranking, prelude.epoch)?; }
            else { prepared_process_owner(&state, &prelude.config, &bridge.process_snapshot()?,
                &ranking, prelude.epoch, unsafe { libc::getuid() })?; }
            self.check(request)
        }
        fn finish_process_ranking_reset(&mut self, request: &OwnerRequest, response: Value) -> Result<Pending> {
            self.check(request)?;
            process_ranking_reply(&response, 0)?;
            let prelude = self.process_prelude.as_ref().ok_or("reference process prelude absent")?;
            let bridge = self.process_bridge()?;
            process_prelude_state(&bridge.snapshot()?, &prelude.config)?;
            for i in 0..512 {
                self.check(request)?;
                bridge.record_recency(&format!("fixture.{i}"), (prelude.epoch - (511 - i)) as f64)?;
            }
            self.verify_process_prelude(request, true)?;
            self.process_call(request, &json!({"op":"search-ranking"}))
        }
        fn finish_process_ranking_verify(&mut self, request: &OwnerRequest, response: Value) -> Result<Pending> {
            self.check(request)?;
            process_ranking_reply(&response, 512)?;
            self.verify_process_prelude(request, true)?;
            self.process_bridge()?.mode("processes")?;
            self.check(request)?;
            self.process_bridge()?.cancel_system_search()?;
            self.check(request)?;
            // Same original serial worker and main-reply fence as before. It
            // covers this process publication only, never all async owners.
            self.process_call(request, &json!({"op":"search","mode":"files","query":""}))
        }
        fn check(&self, request: &OwnerRequest) -> Result<()> {
            if self.stopped.load(Ordering::Acquire) || !request.live() {
                return Err("reference request expired or stopped before owner access".into());
            }
            self.bridge.live(7)
        }
        fn respond(&self, request: OwnerRequest, result: Result<OwnerReply>) {
            let result = if !request.live() || self.stopped.load(Ordering::Acquire) {
                Err("reference request expired or stopped; its operation may have completed".into())
            } else {
                result.and_then(|value| {
                    self.bridge.live(7)?;
                    Ok(value)
                })
            };
            let _ = request.response.try_send(result);
        }
        fn finish_process_preparation(
            &mut self,
            request: &OwnerRequest,
            response: Value,
        ) -> Result<OwnerReply> {
            self.check(request)?;
            if response != json!({"ok":true,"result":[]}) {
                return Err("original same-worker empty-files publication fence refused".into());
            }
            let bridge = self.process_bridge()?;
            let provider = bridge
                .process_provider
                .as_ref()
                .ok_or("reference original process provider is not injected")?;
            if provider.state.retired.load(Ordering::Acquire)
                || provider.state.in_flight.load(Ordering::Acquire) != 0
            {
                return Err("reference Rust process provider is not quiescent".into());
            }
            bridge.prepare_processes1000()?;
            self.verify_process_prelude(request, false)?;
            self.process_publication_fenced = true;
            Ok(OwnerReply::Unit)
        }
        fn start_process_preparation(&mut self, request: OwnerRequest) {
            let result = (|| -> Result<Option<Pending>> {
                self.check(&request)?;
                if self.process_publication_fenced
                    && self.process_bridge.as_ref().is_some_and(|bridge|bridge.process_prepared.get()) {
                    // dispatch prepares EVERY sample. Revalidate the real
                    // baseline without undoing the prior sample's sort toggle.
                    self.verify_process_prelude(&request, false)?;
                    self.process_bridge()?.prepare_processes1000()?;
                    self.check(&request)?;
                    return Ok(None);
                }
                self.process_publication_fenced = false;
                self.process_prelude = None;
                self.ensure_process_bridge()?;
                let bridge = self.process_bridge()?;
                let before = bridge.snapshot()?;
                if before["paletteVisible"] != false {
                    return Err(
                        "reference process fixture requires the hidden original owner".into(),
                    );
                }
                let source = &bridge.source.as_ref().ok_or("reference process source absent")?.reader;
                let epoch = process_epoch(source.qualified_fixture_epoch()?)?;
                bridge.mode("commands")?;
                bridge.process_prepared.set(false);
                bridge.stage_catalog(10000, || request.live() && !self.stopped.load(Ordering::Acquire))?;
                self.check(&request)?;
                bridge.query("m")?;
                let state = bridge.snapshot()?;
                let config = state["config"].clone();
                if fixture::configuration(&config, 10000)? != config {
                    return Err("reference process prelude catalog differs from original10k fixture".into());
                }
                process_prelude_state(&state, &config)?;
                self.process_prelude = Some(ProcessPrelude { epoch, config });
                self.process_call(&request, &json!({"op":"search-ranking.reset","confirmed":true})).map(Some)
            })();
            match result {
                Ok(Some(reply)) => {
                    self.pending = Some(OwnerPending {
                        request,
                        reply,
                        purpose: PendingPurpose::ProcessRankingReset,
                        field_cancel:None,
                    });
                }
                Ok(None) => self.respond(request, Ok(OwnerReply::Unit)),
                Err(error) => self.respond(request, Err(error)),
            }
        }
        fn start_wire_call(&mut self, request: OwnerRequest) {
            let result = (|| {
                self.check(&request)?;
                let Operation::WireCall(value) = &request.operation else {
                    return Err("reference owner received another RPC operation".into());
                };
                // The original handle can mutate modes, configuration or async
                // providers. A previous scoped process fence proves none of
                // those later effects; reprepare before another process lane.
                self.process_publication_fenced = false;
                self.process_prelude = None;
                if let Some(bridge) = &self.process_bridge { bridge.process_prepared.set(false); }
                self.bridge.call_live_notifying(value, Some(self.wake.clone()), || {
                    request.live() && !self.stopped.load(Ordering::Acquire)
                })
            })();
            match result {
                Ok(reply) => {
                    self.pending = Some(OwnerPending {
                        request,
                        reply,
                        purpose: PendingPurpose::WireCall,
                        field_cancel:None,
                    });
                }
                Err(error) => self.respond(request, Err(error)),
            }
        }
        fn execute(&mut self, request: &OwnerRequest) -> Result<OwnerReply> {
            self.check(request)?;
            match &request.operation {
                Operation::Verify=>Ok(OwnerReply::Unit),
                Operation::Snapshot=>self.bridge.snapshot().map(OwnerReply::Value),
                Operation::NativeAction(action)=>{
                    match action {UiAction::Select{index}=>self.bridge.select(isize::try_from(*index).map_err(|_|"reference selection index overflow")?)?,UiAction::Actions{open}=>self.bridge.actions(*open)?}
                    self.bridge.snapshot().map(OwnerReply::Value)
                },
                Operation::Query(QueryStage::Search,query)=>{self.bridge.query(query)?;Ok(OwnerReply::Unit)},
                Operation::Query(_,_)=>Err("reference native input/reconciliation completion is not qualified".into()),
                Operation::Functional(lane @ (Functional::CachedProcesses1000|Functional::ProcessSort1000))=>{
                    if !self.process_publication_fenced{return Err("original process provider/publication fence is absent".into());}
                    self.verify_process_prelude(request, false)?;
                    let (elapsed,output)=self.process_bridge()?.measure_processes1000(*lane)?;
                    self.verify_process_prelude(request, false)?;
                    Ok(OwnerReply::Measured(elapsed,output))
                },
                Operation::ProcessObservation=>{
                    let callbacks=PENDING.lock().map_err(|_|"reference callback registry poisoned")?.len();
                    Ok(OwnerReply::Value(json!({"old_process_publication_fenced":self.process_publication_fenced,
                        "headless_process_owner_present":self.process_bridge.is_some(),
                        "rust_provider_in_flight":self.process_bridge.as_ref().and_then(|b|b.process_provider.as_ref().map(|p|p.state.in_flight.load(Ordering::Acquire))),
                        "bridge_reply_contexts":callbacks,"full_owner_coverage_complete":false})))
                },
                Operation::Drain=>Err("reference full original async/SDK owner coverage, including the extra headless state observers/analytics, is unqualified; process publication fence is not physical running0 or all-owner drain".into()),
                Operation::StageCatalog(size)=>{
                    self.process_publication_fenced = false;
                    self.process_prelude = None;
                    if let Some(bridge) = &self.process_bridge { bridge.process_prepared.set(false); }
                    self.bridge.stage_catalog(*size, || request.live() && !self.stopped.load(Ordering::Acquire))?;
                    self.check(request)?;
                    Ok(OwnerReply::Unit)
                },
                Operation::Shutdown=>Err("reference post-teardown original owner drain is not qualified".into()),
                _=>Err("reference functional original-owner preparation/completion is not qualified".into()),
            }
        }
        /// Nonblocking main-owner turn. A real async reply wakes this same
        /// source after storage; no main-thread sleep or new scheduler exists.
        pub fn poll(&mut self) -> Result<()> {
            let pending_purpose = self.pending.as_ref().map(|pending| pending.purpose);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.poll_once())) {
                Ok(result) => result,
                Err(payload) => {
                    // poll_once may have taken the pending request before
                    // entering original field code. Preserve its cleanup
                    // identity across unwind, without attempting original ARC.
                    if let Some(purpose) = pending_purpose {
                        self.retire_failed_field(purpose);
                    }
                    std::panic::resume_unwind(payload);
                }
            }
        }

        /// A failed request may have left an original field operation in
        /// quarantine. In that case normal lifecycle callbacks stay closed.
        pub fn finish_borrowed_field_box(mut self) -> Result<()> {
            if !self.stopped.load(Ordering::Acquire)
                || !self.field_termination_safe()
                || self.process_bridge.is_some()
            {
                return Err("reference field owner is not stopped and locally drained".into());
            }
            self.bridge.release_borrowed_field_box()
        }

        pub fn field_termination_safe(&self) -> bool {
            self.pending.is_none()
                && !self.field_poisoned.get()
                && verify_reply_contexts_drained().is_ok()
        }

        fn poll_once(&mut self) -> Result<()> {
            if let Some(mut pending) = self.pending.take() {
                if let PendingPurpose::FieldCancel { epoch, phase } = pending.purpose {
                    let (original,until)=pending.field_cancel.take().ok_or("Original input cleanup lease absent")?;
                    let outcome=if Instant::now()>=until||self.stopped.load(Ordering::Acquire){Err("Original input cleanup deadline/stop".into())}else{pending.reply.take()};
                    match outcome{
                        Ok(None)=>{pending.field_cancel=Some((original,until));self.pending=Some(pending);},
                        Ok(Some(value))=>{
                            let cleaned = (|| -> Result<Option<Pending>> {
                                let settled = field_cancel_trace(&value, epoch, phase, || {
                                    let source = &self.bridge.source.as_ref()
                                        .ok_or("Original field cleanup source absent")?.reader;
                                    source.field_delegate_not_entered(&self.bridge, epoch)
                                })?;
                                if phase == FieldCancelPhase::Completion && !settled {
                                    let next = self.bridge.call_live_notifying(
                                        &json!({"op":"performance"}), Some(self.wake.clone()),
                                        || Instant::now()<until&&!self.stopped.load(Ordering::Acquire),
                                    )?;
                                    return Ok(Some(next));
                                }
                                if phase != FieldCancelPhase::Reset {
                                    self.bridge.source.as_ref().ok_or("Original field cleanup source absent")?.reader.retire_field_input(&self.bridge,epoch,true)?;
                                }
                                Ok(None)
                            })();
                            match cleaned {
                                Ok(Some(reply)) => {
                                    pending.reply = reply;
                                    pending.field_cancel = Some((original,until));
                                    self.pending = Some(pending);
                                }
                                Ok(None) => self.respond(pending.request,Err(original)),
                                Err(cleanup) => {
                                    self.retire_failed_field(pending.purpose);
                                    self.respond(pending.request,Err(format!("{original}; original input cleanup incomplete: {cleanup}")));
                                }
                            }
                        }
                        Err(cleanup)=>{self.retire_failed_field(pending.purpose);self.respond(pending.request,Err(format!("{original}; original input cleanup incomplete: {cleanup}")));}
                    }
                    if self.pending.is_none() {
                        // A queued request's wake may have coalesced with this reply.
                        (self.wake)();
                    }
                    return Ok(());
                }
                let outcome = self
                    .check(&pending.request)
                    .and_then(|_| pending.reply.take());
                match outcome {
                    Ok(Some(value)) => {
                        let result = match pending.purpose {
                            PendingPurpose::FieldReset(epoch)|PendingPurpose::FieldMount(epoch)|PendingPurpose::FieldCompletion(epoch)=>{
                                match self.field_step(&pending.request,epoch,&mut pending.purpose,value){
                                    Ok(Some((reply,purpose)))=>self.pending=Some(OwnerPending{request:pending.request,reply,purpose,field_cancel:None}),
                                    Ok(None)=>self.respond(pending.request,Ok(OwnerReply::Unit)),
                                    Err(error)=>self.start_field_cancel(pending.request,pending.purpose,error),
                                }
                                if self.pending.is_none() {
                                    // A queued request may already have spent
                                    // its wake while the field reply was pending.
                                    (self.wake)();
                                }
                                return Ok(());
                            }
                            PendingPurpose::ProcessRankingReset | PendingPurpose::ProcessRankingVerify => {
                                let (next, purpose) = if matches!(pending.purpose, PendingPurpose::ProcessRankingReset) {
                                    (self.finish_process_ranking_reset(&pending.request, value), PendingPurpose::ProcessRankingVerify)
                                } else {
                                    (self.finish_process_ranking_verify(&pending.request, value), PendingPurpose::ProcessPreparation)
                                };
                                match next {
                                    Ok(reply) => self.pending = Some(OwnerPending { request: pending.request, reply, purpose,field_cancel:None }),
                                    Err(error) => self.respond(pending.request, Err(error)),
                                }
                                return Ok(());
                            }
                            PendingPurpose::ProcessPreparation => {
                                self.finish_process_preparation(&pending.request, value)
                            }
                            // Preserve the actual complete RPC response, including
                            // errors and extra fields; receipt is not owner drain.
                            PendingPurpose::WireCall => Ok(OwnerReply::Value(value)),
                            PendingPurpose::FieldCancel { .. }=>return Err("Original input cleanup dispatched as an ordinary operation".into()),
                        };
                        self.respond(pending.request, result);
                    }
                    Ok(None) => {
                        self.pending = Some(pending);
                        return Ok(());
                    }
                    Err(error) => {
                        self.start_field_cancel(pending.request, pending.purpose, error);
                        // A queued operation must wait for the cancellation reply.
                        if self.pending.is_some() {
                            return Ok(());
                        }
                    }
                }
            }
            if let Ok(request) = self.requests.try_recv() {
                if matches!(
                    request.operation,
                    Operation::PrepareFunctional(
                        Functional::CachedProcesses1000 | Functional::ProcessSort1000
                    )
                ) {
                    self.start_process_preparation(request);
                } else if matches!(request.operation,Operation::Query(QueryStage::Input|QueryStage::Reconciliation,_)){
                    self.start_field_input(request);
                } else if matches!(request.operation, Operation::WireCall(_)) {
                    self.start_wire_call(request);
                } else {
                    let result = self.execute(&request);
                    self.respond(request, result);
                }
            }
            Ok(())
        }
    }
    impl Drop for ReferenceOwner {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            if let Some(pending)=&self.pending{self.retire_failed_field(pending.purpose);}
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        // These tests share the actual process-wide reply registry. Serialize
        // only its probes so a destructor's try_lock still detects a lock
        // held by its own code path, without competing with another test.
        static REPLY_REGISTRY_TEST: Mutex<()> = Mutex::new(());
        #[test]
        fn field_trace_requires_delegate_then_same_interval_completion_and_clean_cancel(){
            let mut stages=serde_json::Map::new();
            for name in ["search.model","input.model","input.reconcile","open.ready","host.drawCallback","hotkey.handler"]{
                stages.insert(name.into(),json!({"count":0,"retainedSamples":0,"cancelled":0}));
            }
            let baseline=json!({"ok":true,"result":{"schema":2,"sampleLimitPerStage":256,"activeIntervals":0,"stages":stages}});
            assert!(!field_trace(&baseline,PendingPurpose::FieldReset(1)).unwrap());
            assert!(!field_trace(&baseline,PendingPurpose::FieldCancel { epoch: 1, phase: FieldCancelPhase::Reset }).unwrap());
            assert!(field_trace(&baseline,PendingPurpose::FieldCompletion(1)).is_err());
            let mut mounting=baseline.clone();mounting["result"]["stages"]["search.model"]["count"]=json!(2);
            mounting["result"]["stages"]["search.model"]["retainedSamples"]=json!(2);
            assert!(!field_trace(&mounting,PendingPurpose::FieldMount(1)).unwrap());
            let mount_cancel = PendingPurpose::FieldCancel { epoch: 1, phase: FieldCancelPhase::Mount };
            assert!(field_trace(&mounting, mount_cancel).unwrap());
            assert!(field_trace(&mounting,PendingPurpose::FieldReset(1)).is_err());
            let mut changed=baseline.clone();changed["result"]["stages"]["input.model"]=json!({"count":1,"retainedSamples":1,"cancelled":0});
            changed["result"]["activeIntervals"]=json!(1);
            assert!(!field_trace(&changed,PendingPurpose::FieldCompletion(1)).unwrap());
            let mut lost=changed.clone();lost["result"]["activeIntervals"]=json!(0);
            assert!(field_trace(&lost,PendingPurpose::FieldCompletion(1)).is_err());
            assert!(field_trace(&changed,PendingPurpose::FieldMount(1)).is_err());
            assert!(field_trace(&changed, mount_cancel).is_err());
            let mut completed=changed.clone();completed["result"]["stages"]["input.reconcile"]=json!({"count":1,"retainedSamples":1,"cancelled":0});completed["result"]["activeIntervals"]=json!(0);
            assert!(field_trace(&completed,PendingPurpose::FieldCompletion(1)).unwrap());
            for stage in ["input.model","input.reconcile"]{
                let mut cancelled=completed.clone();cancelled["result"]["stages"][stage]["cancelled"]=json!(1);
                assert!(field_trace(&cancelled,PendingPurpose::FieldCompletion(1)).is_err());
                let mut repeated=completed.clone();repeated["result"]["stages"][stage]["count"]=json!(2);
                assert!(field_trace(&repeated,PendingPurpose::FieldCompletion(1)).is_err());
                let mut missing=completed.clone();missing["result"]["stages"][stage]["retainedSamples"]=json!(0);
                assert!(field_trace(&missing,PendingPurpose::FieldCompletion(1)).is_err());
            }
            let cleanup = PendingPurpose::FieldCancel { epoch: 1, phase: FieldCancelPhase::Completion };
            assert!(field_trace(&baseline,cleanup).is_err());
            let phase = FieldCancelPhase::Completion;
            assert!(field_cancel_trace(&baseline, 1, phase, || Ok(false)).is_err());
            assert!(field_cancel_trace(&baseline, 1, phase, || Ok(true)).unwrap());
            let mut active_baseline = baseline.clone();
            active_baseline["result"]["activeIntervals"] = json!(1);
            assert!(field_cancel_trace(&active_baseline, 1, phase, || Ok(true)).is_err());
            assert!(!field_cancel_trace(&changed, 1, phase, || panic!("existing effect")).unwrap());
            assert!(!field_trace(&changed,cleanup).unwrap());
            assert!(field_trace(&completed,cleanup).unwrap());
            let mut cancelled = changed.clone();
            cancelled["result"]["activeIntervals"] = json!(0);
            cancelled["result"]["stages"]["input.reconcile"]["cancelled"] = json!(1);
            assert!(field_trace(&cancelled,cleanup).unwrap());
            assert!(field_trace(&cancelled,PendingPurpose::FieldCompletion(1)).is_err());
            assert!(field_trace(&baseline,PendingPurpose::WireCall).is_err());
        }
        // A Rust lifetime/denial probe only: this issuer NEVER returns a table,
        // profile or original-call permission, and never loads any SDK/image.
        struct RefusingSource {
            drops: Arc<AtomicUsize>,
            table_attempts: Arc<AtomicUsize>,
            worker_attempts: Arc<AtomicUsize>,
        }
        unsafe impl SourceCodeReader for RefusingSource {
            fn qualified_main_table(&self, _: Guard, _: Reply, _: ProcessProvider) -> Result<usize> {
                self.table_attempts.fetch_add(1, Ordering::AcqRel);
                Err("owned denying issuer has no table authority".into())
            }
            fn qualified_profile(&self) -> Result<std::ffi::CString> {
                Err("owned denying issuer has no profile authority".into())
            }
            fn qualified_fixture_epoch(&self) -> Result<u64> {
                Err("owned denying issuer has no fixture epoch authority".into())
            }
            fn qualified_process_ranking(&self) -> Result<Vec<u8>> {
                Err("owned denying issuer has no profile read authority".into())
            }
            fn begin_field_input(&self,_:&Bridge,_:&str,_:u64)->Result<bool>{Err("no field authority".into())}
            fn verify_field_input(&self,_:&Bridge,_:u64)->Result<()>{Err("no field authority".into())}
            fn retire_field_input(&self,_:&Bridge,_:u64,_:bool)->Result<()>{Err("no field authority".into())}
            fn before_main_call(&self) -> Result<()> { Err("no main authority".into()) }
            fn before_worker_callback(&self) -> Result<()> {
                self.worker_attempts.fetch_add(1, Ordering::AcqRel);
                Err("no worker authority".into())
            }
        }
        impl Drop for RefusingSource {
            fn drop(&mut self) {
                assert!(PENDING.try_lock().is_ok(), "source destructor under reply registry mutex");
                self.drops.fetch_add(1, Ordering::AcqRel);
            }
        }
        fn refusing_source() -> (Arc<RefusingSource>, Arc<AtomicUsize>) {
            let drops = Arc::new(AtomicUsize::new(0));
            (Arc::new(RefusingSource { drops: drops.clone(), table_attempts: Arc::new(AtomicUsize::new(0)),
                worker_attempts: Arc::new(AtomicUsize::new(0)) }), drops)
        }
        #[test]
        fn process_prelude_validates_exact_512_durable_ids_timestamps_and_epoch() {
            let epoch = 1_700_000_000;
            let correct = fixture::recency(epoch as i64);
            process_ranking_bytes(&serde_json::to_vec(&correct).unwrap(), epoch).unwrap();
            for invalid in [0, 510, 9_007_199_254_740_992] { assert!(process_epoch(invalid).is_err()); }
            for changed in 0..5 {
                let mut value = correct.clone();
                match changed {
                    0 => value["schema"] = json!(2),
                    1 => value["entries"]["fixture.27"] = json!(epoch),
                    2 => { let entries=value["entries"].as_object_mut().unwrap();
                        let value=entries.remove("fixture.1").unwrap(); entries.insert("different".into(),value); }
                    3 => { value.as_object_mut().unwrap().insert("extra".into(),json!(true)); }
                    _ => value["entries"]["fixture.511"] = json!("1700000000"),
                }
                assert!(process_ranking_bytes(&serde_json::to_vec(&value).unwrap(), epoch).is_err());
            }
            assert!(process_ranking_bytes(b"not JSON", epoch).is_err());
            assert!(process_ranking_bytes(&vec![b' ';1_000_001], epoch).is_err());
        }
        #[test]
        fn process_prelude_requires_original_hidden_catalog_query_and_real_summary() {
            let config = fixture::configuration(&json!({"version":1,"clipboard":{},"commands":[]}),10000).unwrap();
            let state=json!({"config":config,"mode":"commands","query":"m","paletteVisible":false,
                "actionsOpen":false,"busy":false,"clipboardSearchBusy":false});
            process_prelude_state(&state,&config).unwrap();
            for (key,value) in [("mode",json!("processes")),("query",json!("")),("paletteVisible",json!(true)),
                ("actionsOpen",json!(true)),("busy",json!(true)),("clipboardSearchBusy",json!(true)),("config",json!({}))] {
                let mut changed=state.clone();changed[key]=value;
                assert!(process_prelude_state(&changed,&config).is_err());
            }
            process_ranking_reply(&json!({"ok":true,"result":{"entries":0,"limit":512,"retentionDays":90}}),0).unwrap();
            process_ranking_reply(&json!({"ok":true,"result":{"entries":512,"limit":512,"retentionDays":90}}),512).unwrap();
            assert!(process_ranking_reply(&json!({"ok":true,"result":{"entries":511,"limit":512,"retentionDays":90}}),512).is_err());
            assert!(process_ranking_reply(&json!({"ok":false,"error":"refused"}),0).is_err());
        }
        #[test]
        fn process_fixture_inputs_never_fall_back_to_unqualified_profile_or_clock() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let (source,_) = refusing_source();
            assert!(source.qualified_fixture_epoch().is_err());
            assert!(source.qualified_process_ranking().is_err());
            assert_eq!(source.table_attempts.load(Ordering::Acquire),0);
            assert_eq!(source.worker_attempts.load(Ordering::Acquire),0);
        }
        #[test]
        fn pending_type_is_not_send_and_keeps_the_actual_main_owner_contract() {
            // Deliberately ambiguous if Pending ever becomes Send. This is a
            // compile-time type assertion, not a thread/SDK invocation.
            struct Probe<T: ?Sized>(PhantomData<T>);
            trait AmbiguousIfSend<A> { fn check() {} }
            impl<T: ?Sized> AmbiguousIfSend<()> for Probe<T> {}
            impl<T: ?Sized + Send> AmbiguousIfSend<u8> for Probe<T> {}
            let _ = <Probe<Pending> as AmbiguousIfSend<_>>::check;
        }
        #[test]
        fn qualified_box_constructor_failure_releases_its_bounded_counting_permit() {
            static BOXES: AtomicUsize = AtomicUsize::new(0);
            assert_eq!(REPLY_CONTEXT_LIMIT, 16);
            assert_eq!(QUALIFIED_BOX_LIMIT, 2);
            assert_eq!(QUALIFIED_SOURCE_HOLD_LIMIT, 18);
            let first = SourceBoxPermit::from_counter(&BOXES).unwrap();
            let second = SourceBoxPermit::from_counter(&BOXES).unwrap();
            assert!(SourceBoxPermit::from_counter(&BOXES).is_err());
            drop(first);
            drop(second);
            assert_eq!(BOXES.load(Ordering::Acquire), 0);
            let refused_before_foreign_construction = || -> Result<()> {
                let _permit = SourceBoxPermit::from_counter(&BOXES)?;
                Err("owned profile/constructor prerequisite refused".into())
            };
            assert!(refused_before_foreign_construction().is_err());
            assert_eq!(BOXES.load(Ordering::Acquire), 0);
        }
        #[test]
        fn qualified_nondrained_quarantine_keeps_real_issuer_and_never_calls_release() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            static BOXES: AtomicUsize = AtomicUsize::new(0);
            let (reader, drops) = refusing_source();
            let weak = Arc::downgrade(&reader);
            let release_calls = AtomicUsize::new(0);
            let held = QualifiedSourceBox { reader: reader.clone(),
                _permit: SourceBoxPermit::from_counter(&BOXES).unwrap() };
            drop(reader);
            held.finish(false, || { release_calls.fetch_add(1, Ordering::AcqRel); });
            assert_eq!(release_calls.load(Ordering::Acquire), 0);
            assert_eq!(drops.load(Ordering::Acquire), 0);
            assert_eq!(BOXES.load(Ordering::Acquire), 1);
            assert!(weak.upgrade().is_some());
            // This is an intentional process-lifetime Rust-denial quarantine;
            // no foreign F1Box/table/function pointer was created or called.
        }
        #[test]
        fn qualified_idle_release_holds_issuer_through_owned_rust_release_only() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            static BOXES: AtomicUsize = AtomicUsize::new(0);
            let (reader, drops) = refusing_source();
            let weak = Arc::downgrade(&reader);
            let release_calls = AtomicUsize::new(0);
            let held = QualifiedSourceBox { reader: reader.clone(),
                _permit: SourceBoxPermit::from_counter(&BOXES).unwrap() };
            drop(reader);
            held.finish(true, || {
                assert!(weak.upgrade().is_some());
                assert_eq!(BOXES.load(Ordering::Acquire), 1);
                release_calls.fetch_add(1, Ordering::AcqRel);
            });
            assert_eq!(release_calls.load(Ordering::Acquire), 1);
            assert_eq!(drops.load(Ordering::Acquire), 1);
            assert_eq!(BOXES.load(Ordering::Acquire), 0);
            // Only the private counting/retention function is exercised. True
            // here is NOT an Original-code, SDK or whole-owner-drain grant.
        }
        #[test]
        fn typed_table_bounds_never_turn_an_address_or_header_into_authority() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            assert!(checked_table_address(0).is_err());
            assert!(checked_table_address(1).is_err());
            assert!(checked_table_address(usize::MAX & !7).is_err());
            // Range arithmetic only: do NOT dereference this non-issued address.
            assert_eq!(checked_table_address(4096).unwrap() as usize, 4096);
            let (source, drops) = refusing_source();
            let issuer: Arc<dyn SourceCodeReader> = source.clone();
            let result = QualifiedFunctions::read(issuer, provider_deny);
            assert!(result.is_err());
            if unsafe { libc::pthread_main_np() } != 1 {
                assert_eq!(source.table_attempts.load(Ordering::Acquire), 0);
            }
            drop(source);
            assert_eq!(drops.load(Ordering::Acquire), 1);
        }
        #[test]
        fn unqualified_late_reply_cannot_discharge_timed_out_issuer() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let (source, drops) = refusing_source();
            let wake_count = Arc::new(AtomicUsize::new(0));
            let signal = wake_count.clone();
            let state = Arc::new(Completion { result: Mutex::new(None),
                wake: Some(Arc::new(move || { signal.fetch_add(1, Ordering::AcqRel); })),
                source: Some(source.clone()), guard: Some(provider_deny),
                retired: AtomicBool::new(false), reply_received: AtomicBool::new(false),
                submission_started: AtomicBool::new(true) });
            let key = NEXT.fetch_add(1, Ordering::AcqRel);
            PENDING.lock().unwrap().insert(key, state.clone());
            let weak = Arc::downgrade(&state);
            drop(source);
            drop(Pending { key, state, _main: PhantomData });
            assert!(PENDING.lock().unwrap().contains_key(&key));
            assert_eq!(drops.load(Ordering::Acquire), 0);
            let bytes = b"{\"late\":true}";
            reply(key as *mut c_void, bytes.as_ptr().cast(), bytes.len() as isize);
            let held = weak.upgrade().expect("unconfirmed submission retains its issuer");
            assert!(!held.reply_received.load(Ordering::Acquire));
            assert!(held.result.lock().unwrap().is_none());
            assert_eq!(wake_count.load(Ordering::Acquire), 0);
            assert_eq!(drops.load(Ordering::Acquire), 0);
            remove_completion(key, &held); // No actual Swift submission in this test.
            drop(held);
            assert_eq!(drops.load(Ordering::Acquire), 1);
        }
        #[test]
        fn qualified_pre_admission_failure_has_no_future_callback_to_retain() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let (source, drops) = refusing_source();
            let state = Arc::new(Completion { result: Mutex::new(None), wake: None,
                source: Some(source.clone()), guard: Some(provider_deny),
                retired: AtomicBool::new(false), reply_received: AtomicBool::new(false),
                submission_started: AtomicBool::new(false) });
            let key = NEXT.fetch_add(1, Ordering::AcqRel);
            PENDING.lock().unwrap().insert(key, state.clone());
            drop(source);
            drop(Pending { key, state, _main: PhantomData });
            assert!(!PENDING.lock().unwrap().contains_key(&key));
            assert_eq!(drops.load(Ordering::Acquire), 1);
        }
        #[test]
        fn final_request_check_prevents_submission_after_guard_cancellation() {
            let completion = || Completion { result: Mutex::new(None), wake: None,
                source: None, guard: None, retired: AtomicBool::new(false),
                reply_received: AtomicBool::new(false), submission_started: AtomicBool::new(false) };
            let (response, _) = std::sync::mpsc::sync_channel(1);
            let request = OwnerRequest { operation: Operation::Verify,
                deadline: Instant::now() + Duration::from_secs(30),
                cancelled: Arc::new(AtomicBool::new(false)), response };
            let state = completion();
            let calls = AtomicUsize::new(0);
            assert!(request.live());
            // A successful dynamic guard may reenter and cancel the request.
            let guard = || -> Result<()> { request.cancelled.store(true, Ordering::Release); Ok(()) };
            guard().unwrap();
            let effect = state.start_submission(request.live()).map(|()| { calls.fetch_add(1, Ordering::AcqRel); });
            assert!(effect.is_err());
            assert!(!state.submission_started.load(Ordering::Acquire));
            assert_eq!(calls.load(Ordering::Acquire), 0);
            let expired = completion();
            let expired_deadline = Instant::now();
            assert!(expired.start_submission(Instant::now() < expired_deadline).is_err());
            assert!(!expired.submission_started.load(Ordering::Acquire));
            // Positive control is an owned Rust counter, never an Original call.
            let admitted = completion();
            admitted.start_submission(true).unwrap();
            calls.fetch_add(1, Ordering::AcqRel);
            assert!(admitted.submission_started.load(Ordering::Acquire));
            assert_eq!(calls.load(Ordering::Acquire), 1);
        }
        static QUALIFIED_PROVIDER_GUARD_CALLS: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn qualified_provider_guard(_: u32) -> i32 {
            QUALIFIED_PROVIDER_GUARD_CALLS.fetch_add(1, Ordering::AcqRel);
            1
        }
        #[test]
        fn qualified_provider_source_refusal_precedes_dynamic_guard_and_foundation() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let (source, drops) = refusing_source();
            let attempts = source.worker_attempts.clone();
            QUALIFIED_PROVIDER_GUARD_CALLS.store(0, Ordering::Release);
            let provider = ProcessProviderOwner::register_source(qualified_provider_guard, Some(source.clone())).unwrap();
            drop(source);
            assert!(ProcessProviderCall::enter(provider.key).is_none());
            assert_eq!(provider.state.in_flight.load(Ordering::Acquire), 0);
            assert_eq!(attempts.load(Ordering::Acquire), 1);
            assert!(process_provider(provider.key as *mut c_void).is_null());
            assert_eq!(attempts.load(Ordering::Acquire), 2);
            assert_eq!(QUALIFIED_PROVIDER_GUARD_CALLS.load(Ordering::Acquire), 0);
            provider.retire();
            assert!(ProcessProviderCall::enter(provider.key).is_none());
            drop(provider);
            assert_eq!(drops.load(Ordering::Acquire), 1);
        }
        #[test]
        fn main_functions_table_has_exact_typed_transport_layout_without_authority() {
            use std::mem::{align_of, offset_of, size_of};
            assert_eq!(size_of::<usize>(), 8);
            assert_eq!(size_of::<Functions>(), 25 * 8);
            assert_eq!(size_of::<MainFunctionsV1>(), 216);
            assert_eq!(align_of::<MainFunctionsV1>(), 8);
            assert_eq!(offset_of!(MainFunctionsV1, functions), 16);
            let offsets = [
                offset_of!(Functions, install_guard), offset_of!(Functions, borrow_delegate),
                offset_of!(Functions, create_with_process_provider), offset_of!(Functions, cancel_system_search),
                offset_of!(Functions, drop_box), offset_of!(Functions, query), offset_of!(Functions, mode),
                offset_of!(Functions, select), offset_of!(Functions, actions), offset_of!(Functions, set_commands),
                offset_of!(Functions, snapshot), offset_of!(Functions, result_count), offset_of!(Functions, visible_count),
                offset_of!(Functions, selected_id_count), offset_of!(Functions, filtered_count),
                offset_of!(Functions, apply_system_results), offset_of!(Functions, process_sort),
                offset_of!(Functions, set_process_sort), offset_of!(Functions, process_snapshot),
                offset_of!(Functions, config_data), offset_of!(Functions, encode_clips),
                offset_of!(Functions, record_timing), offset_of!(Functions, record_recency),
                offset_of!(Functions, call), offset_of!(Functions, content_view),
            ];
            assert_eq!(offsets, std::array::from_fn::<_, 25, _>(|index| index * 8));
            assert_eq!(MAIN_FUNCTION_SYMBOLS.len(), MAIN_FUNCTIONS_COUNT as usize);
            assert_eq!(MAIN_FUNCTION_SYMBOLS[1], "rcf1_borrow_delegate");
            assert_eq!(MAIN_FUNCTION_SYMBOLS[24], "rcf1_content_view");
            assert_eq!(MAIN_FUNCTIONS_MAGIC, 0xF100_0026_4D41_494E);
            assert_eq!(MAIN_FUNCTIONS_ABI, 1);
            let (reply_entry, provider_entry) = callback_entrypoints();
            unsafe extern "C" fn owned_unsafe_reply(_: *mut c_void, _: *const c_void, _: isize) {}
            let _borrowed_buffer_contract: Reply = owned_unsafe_reply;
            assert!(std::ptr::fn_addr_eq(reply_entry, reply as Reply));
            assert!(std::ptr::fn_addr_eq(provider_entry, process_provider as ProcessProvider));
            // This constructs or calls no original pointer and grants no route.
        }
        #[test]
        fn general_rpc_proxy_preserves_complete_request_error_and_response_fields() {
            // Exercise the real worker proxy/channel only, without inventing
            // a Bridge, SDK guard or original AppState completion authority.
            let (sender, receiver) = std::sync::mpsc::sync_channel::<OwnerRequest>(1);
            let receiver = Arc::new(Mutex::new(receiver));
            let inbox = receiver.clone();
            let observed = Arc::new(Mutex::new(Vec::new()));
            let calls = observed.clone();
            let response = json!({"ok":false,"error":"owned Ω\nerror", "extra":{"nul":"a\0b"}});
            let expected = response.clone();
            let wake: Wake = Arc::new(move || {
                if let Ok(request) = inbox.lock().unwrap().try_recv() {
                    assert!(request.live());
                    let Operation::WireCall(value) = request.operation else {
                        panic!("The real proxy must retain its RPC operation.");
                    };
                    calls.lock().unwrap().push(value);
                    request.response.try_send(Ok(OwnerReply::Value(response.clone())))
                        .unwrap_or_else(|_| panic!("RPC requester disappeared"));
                }
            });
            let stopped = Arc::new(AtomicBool::new(false));
            let mut host = ReferenceHost {
                requests: sender,
                wake,
                stopped: stopped.clone(),
                bindings: ReadyBindings {
                    fixture_sha256: "1".repeat(64),
                    authority_sha256: "2".repeat(64),
                    sdk_guard_sha256: "3".repeat(64),
                    clock_resolution_ns: 1,
                },
                admitted: BTreeSet::new(),
                field_probe: false,
            };
            let request = json!({"op":"search","query":"Ω\n\0","limit":0,"unrecognized":{"keep":true}});
            assert_eq!(host.wire_call(request.clone(), Instant::now()+Duration::from_secs(1)).unwrap(), expected);
            stopped.store(true, Ordering::Release);
            assert!(host.wire_call(request.clone(), Instant::now()+Duration::from_secs(1)).is_err());
            assert_eq!(*observed.lock().unwrap(), vec![request]);
            assert!(host.admitted_routes().is_empty());
        }
        #[test]
        fn field_timeout_keeps_receiver_alive_for_cleanup_without_accepting_late_success() {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<OwnerRequest>(1);
            let receiver = Mutex::new(receiver);
            let request = Mutex::new(None::<OwnerRequest>);
            let cleanup = Arc::new(Mutex::new(None));
            let scheduled = cleanup.clone();
            let host = ReferenceHost {
                requests: sender,
                wake: Arc::new(move || {
                    if let Ok(incoming) = receiver.lock().unwrap().try_recv() {
                        *request.lock().unwrap() = Some(incoming);
                    } else if let Some(pending) = request.lock().unwrap().take() {
                        assert!(pending.cancelled.load(Ordering::Acquire));
                        *scheduled.lock().unwrap() = Some(std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(20));
                            pending.response.try_send(Ok(OwnerReply::Unit)).is_ok()
                        }));
                    }
                }),
                stopped: Arc::new(AtomicBool::new(false)),
                bindings: ReadyBindings {
                    fixture_sha256: "1".repeat(64),
                    authority_sha256: "2".repeat(64),
                    sdk_guard_sha256: "3".repeat(64),
                    clock_resolution_ns: 1,
                },
                admitted: BTreeSet::new(),
                field_probe: true,
            };
            let deadline = Instant::now() + Duration::from_millis(100);
            for operation in [
                Operation::Query(QueryStage::Search, "direct setter".into()),
                Operation::WireCall(json!({"op":"search"})),
            ] {
                match host.call(operation, deadline) {
                    Err(error) => assert_eq!(error, "reference field probe operation is outside the retained field route"),
                    Ok(_) => panic!("field probe accepted a general request"),
                }
            }
            assert!(host.call(Operation::Query(QueryStage::Input, "fixture".into()), deadline).is_err());
            assert!(cleanup.lock().unwrap().take().unwrap().join().unwrap());
        }
        #[test]
        fn process_proxy_refuses_queued_success_after_absolute_deadline_during_wake() {
            // Pure Rust requester/channel only: no Bridge/guard/AppState exists.
            let (sender, receiver) = std::sync::mpsc::sync_channel::<OwnerRequest>(1);
            let receiver = Arc::new(Mutex::new(receiver));
            let inbox = receiver.clone();
            let deadline = Instant::now() + Duration::from_millis(50);
            let acknowledged = Arc::new(AtomicBool::new(false));
            let ack = acknowledged.clone();
            let wake: Wake = Arc::new(move || {
                if let Ok(request) = inbox.lock().unwrap().try_recv() {
                    assert!(
                        Instant::now() < request.deadline,
                        "counterexample requires a genuinely on-time response"
                    );
                    request
                        .response
                        .try_send(Ok(OwnerReply::Unit))
                        .unwrap_or_else(|_| {
                            panic!("requester unexpectedly disappeared before wake returned")
                        });
                    ack.store(true, Ordering::Release);
                    let after = request.deadline + Duration::from_millis(1);
                    while Instant::now() < after {
                        std::thread::park_timeout(after.saturating_duration_since(Instant::now()));
                    }
                }
            });
            let host = ReferenceHost {
                requests: sender,
                wake,
                stopped: Arc::new(AtomicBool::new(false)),
                bindings: ReadyBindings {
                    fixture_sha256: "1".repeat(64),
                    authority_sha256: "2".repeat(64),
                    sdk_guard_sha256: "3".repeat(64),
                    clock_resolution_ns: 1,
                },
                admitted: BTreeSet::new(),
                field_probe: false,
            };
            let response = host.call(Operation::Verify, deadline);
            assert!(acknowledged.load(Ordering::Acquire));
            assert!(Instant::now() >= deadline);
            assert!(
                response.is_err(),
                "Queued success cannot bypass the requester's absolute deadline."
            );
        }
        static REENTRANT_PROVIDER_KEY: AtomicUsize = AtomicUsize::new(0);
        static REENTRANT_INFLIGHT: AtomicUsize = AtomicUsize::new(usize::MAX);
        extern "C" fn provider_guard_retires(_: u32) -> i32 {
            let key = REENTRANT_PROVIDER_KEY.load(Ordering::Acquire);
            if let Some(state) = PROCESS_PROVIDERS.lock().unwrap().get(&key).cloned() {
                REENTRANT_INFLIGHT
                    .store(state.in_flight.load(Ordering::Acquire), Ordering::Release);
                state.retired.store(true, Ordering::Release);
            }
            1
        }
        #[test]
        fn process_provider_counts_the_dynamic_guard_as_inflight_and_refuses_reentrant_retirement()
        {
            let owner = ProcessProviderOwner::register(provider_guard_retires).unwrap();
            REENTRANT_PROVIDER_KEY.store(owner.key, Ordering::Release);
            REENTRANT_INFLIGHT.store(usize::MAX, Ordering::Release);
            // Only the pure Rust lookup/guard scope runs. No NSData allocation.
            assert!(ProcessProviderCall::enter(owner.key).is_none());
            assert_eq!(
                REENTRANT_INFLIGHT.load(Ordering::Acquire),
                1,
                "A live callback's dynamic guard belongs to its in-flight scope."
            );
            assert_eq!(owner.state.in_flight.load(Ordering::Acquire), 0);
            assert!(owner.state.retired.load(Ordering::Acquire));
        }
        extern "C" fn provider_deny(_: u32) -> i32 {
            0
        }
        extern "C" fn provider_memory_only(_: u32) -> i32 {
            1
        }
        #[test]
        fn process_provider_unknown_denied_retired_never_enters_foundation() {
            assert!(process_provider(usize::MAX as *mut c_void).is_null());
            let owner = ProcessProviderOwner::register(provider_deny).unwrap();
            let key = owner.key;
            let state = owner.state.clone();
            assert!(process_provider(key as *mut c_void).is_null());
            assert_eq!(state.in_flight.load(Ordering::Acquire), 0);
            owner.retire();
            assert!(process_provider(key as *mut c_void).is_null());
            assert!(state.retired.load(Ordering::Acquire));
            assert!(!PROCESS_PROVIDERS.lock().unwrap().contains_key(&key));
        }
        #[test]
        fn process_provider_rust_context_survives_retirement_without_cookie_reuse() {
            // This positive guard is used ONLY by the pure Rust lifetime helper.
            // It never reaches process_provider/NSData/AppState or an SDK call.
            let owner = ProcessProviderOwner::register(provider_memory_only).unwrap();
            let first = owner.key;
            let state = owner.state.clone();
            let call = ProcessProviderCall::enter(first).unwrap();
            assert_eq!(state.in_flight.load(Ordering::Acquire), 1);
            assert_eq!(
                serde_json::from_slice::<Value>(&call.0.bytes)
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .len(),
                1000
            );
            drop(owner);
            assert!(!call.0.live());
            assert!(ProcessProviderCall::enter(first).is_none());
            let next = ProcessProviderOwner::register(provider_memory_only).unwrap();
            assert_ne!(next.key, first);
            drop(call);
            assert_eq!(state.in_flight.load(Ordering::Acquire), 0);
        }
        #[test]
        fn process_reply_notifies_after_storage_with_no_registry_or_result_lock() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let key = NEXT.fetch_add(1, Ordering::AcqRel);
            let observed = Arc::new(AtomicUsize::new(0));
            let signal = observed.clone();
            let wake: Wake = Arc::new(move || {
                let table = PENDING
                    .try_lock()
                    .expect("reply must release registry lock before wake");
                let state = table.get(&key).unwrap().clone();
                drop(table);
                let slot = state
                    .result
                    .try_lock()
                    .expect("reply must release result lock before wake");
                assert!(slot.is_some());
                signal.fetch_add(1, Ordering::AcqRel);
            });
            let state = Arc::new(Completion {
                result: Mutex::new(None),
                wake: Some(wake),
                source: None,
                guard: None,
                retired: AtomicBool::new(false),
                reply_received: AtomicBool::new(false),
                submission_started: AtomicBool::new(false),
            });
            PENDING.lock().unwrap().insert(key, state.clone());
            let bytes = b"{\"ok\":true}";
            reply(
                key as *mut c_void,
                bytes.as_ptr().cast(),
                bytes.len() as isize,
            );
            assert_eq!(observed.load(Ordering::Acquire), 1);
            let mut pending = Pending { key, state, _main: PhantomData };
            assert_eq!(pending.take().unwrap(), Some(json!({"ok":true})));
        }
        #[test]
        fn process_request_timeout_cleanup_wakes_main_owner_and_host_is_worker_sendable() {
            fn worker<T: Send + Host>() {}
            worker::<ReferenceHost>();
            let cancelled = Arc::new(AtomicBool::new(false));
            let observed = Arc::new(AtomicUsize::new(0));
            let signal = observed.clone();
            let wake: Wake = Arc::new(move || {
                signal.fetch_add(1, Ordering::AcqRel);
            });
            {
                let _wait = RequestWait {
                    cancelled: cancelled.clone(),
                    wake,
                };
            }
            assert!(cancelled.load(Ordering::Acquire));
            assert_eq!(observed.load(Ordering::Acquire), 1);
            let (response, _) = std::sync::mpsc::sync_channel(1);
            let request = OwnerRequest {
                operation: Operation::Verify,
                deadline: Instant::now() + Duration::from_secs(1),
                cancelled,
                response,
            };
            assert!(!request.live());
            assert!(require_complete_routes(&BTreeSet::new()).is_err());
        }
        fn process_validation_fixture() -> Value {
            let commands = fixture::process_commands();
            let order = process_fixture_order(ProcessSort::Cpu);
            json!({"results":commands,"visibleCommands":order.iter().map(|i|commands[*i].clone()).collect::<Vec<_>>(),
                "processResources":fixture::process_resources(501),"processSort":"CPU Usage","selection":0,"selectedID":"fixture.process.96"})
        }
        #[test]
        fn process_prepared_prefix_accepts_cpu_memory_cpu_without_rebuilding_selection() {
            let config=fixture::configuration(&json!({"version":1,"clipboard":{},"commands":[]}),10000).unwrap();
            let state=json!({"config":config,"mode":"processes","query":"","paletteVisible":false,"actionsOpen":false});
            let epoch=1_700_000_000;
            let ranking=serde_json::to_vec(&fixture::recency(epoch as i64)).unwrap();
            let cpu=process_validation_fixture();
            let mut memory=cpu.clone();
            memory["processSort"]=json!("Memory");
            memory["visibleCommands"]=json!(fixture::process_commands().into_iter().rev().collect::<Vec<_>>());
            memory["selection"]=json!(903);
            for actual in [&cpu,&memory,&cpu] {
                let unchanged=actual.clone();
                prepared_process_owner(&state,&config,actual,&ranking,epoch,501).unwrap();
                assert_eq!(*actual,unchanged,"Prepared-prefix validation cannot reset actual order/selection.");
            }
            let mut wrong=memory.clone();wrong["selection"]=json!(0);wrong["selectedID"]=json!("fixture.process.999");
            assert!(prepared_process_owner(&state,&config,&wrong,&ranking,epoch,501).is_err());
            let mut name=cpu.clone();name["processSort"]=json!("Name");
            assert!(prepared_process_owner(&state,&config,&name,&ranking,epoch,501).is_err());
        }
        #[test]
        fn process_abi_codes_and_full_fixture_snapshot_validation_are_exact() {
            assert_eq!(ProcessSort::try_from(0).unwrap(), ProcessSort::Cpu);
            assert_eq!(ProcessSort::try_from(1).unwrap(), ProcessSort::Memory);
            assert_eq!(ProcessSort::try_from(2).unwrap(), ProcessSort::Name);
            for invalid in [-1, 3, i32::MAX] {
                assert!(ProcessSort::try_from(invalid).is_err());
            }
            let ready =
                json!({"mode":"processes","paletteVisible":false,"query":"","actionsOpen":false});
            process_owner_ready(&ready).unwrap();
            for (key, value) in [
                ("mode", json!("commands")),
                ("paletteVisible", json!(true)),
                ("query", json!("m")),
            ] {
                let mut changed = ready.clone();
                changed[key] = value;
                assert!(process_owner_ready(&changed).is_err());
            }
            let initial = process_validation_fixture();
            validate_process_snapshot(&initial, 501).unwrap();
            assert_eq!(initial["visibleCommands"].as_array().unwrap().len(), 1000);
            assert_eq!(initial["processResources"].as_object().unwrap().len(), 1000);
            let mut memory = initial.clone();
            memory["processSort"] = json!("Memory");
            memory["visibleCommands"] = json!(
                fixture::process_commands()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
            );
            memory["selection"] = json!(903); // 999-96: preserve the selected ID96.
            validate_process_snapshot(&memory, 501).unwrap();
            let mut wrong_cpu = initial.clone();
            wrong_cpu["selection"] = json!(1);
            wrong_cpu["selectedID"] = json!("fixture.process.193");
            assert!(validate_process_snapshot(&wrong_cpu, 501).is_err());
            let mut wrong_memory = memory.clone();
            wrong_memory["selection"] = json!(0);
            wrong_memory["selectedID"] = json!("fixture.process.999");
            assert!(validate_process_snapshot(&wrong_memory, 501).is_err());
            let mut numeric = initial.clone();
            numeric["processResources"]["fixture.process.96"]["cpuPercent"] = json!(96.0);
            validate_process_snapshot(&numeric, 501).unwrap();
            assert_ne!(
                numeric, initial,
                "Validation must not rewrite integral Double JSON representations."
            );
        }
        #[test]
        fn process_fixture_validation_rejects_each_changed_owner_component() {
            let initial = process_validation_fixture();
            for path in [
                vec!["processResources", "fixture.process.7", "identity", "pid"],
                vec!["processResources", "fixture.process.7", "identity", "uid"],
                vec![
                    "processResources",
                    "fixture.process.7",
                    "identity",
                    "seconds",
                ],
                vec![
                    "processResources",
                    "fixture.process.7",
                    "identity",
                    "micros",
                ],
                vec!["processResources", "fixture.process.7", "name"],
                vec!["processResources", "fixture.process.7", "path"],
                vec!["processResources", "fixture.process.7", "cpuPercent"],
                vec!["processResources", "fixture.process.7", "memoryBytes"],
            ] {
                let mut changed = initial.clone();
                let mut slot = &mut changed;
                for key in path {
                    slot = &mut slot[key];
                }
                *slot = json!(null);
                assert!(validate_process_snapshot(&changed, 501).is_err());
            }
            let mut duplicate = initial.clone();
            duplicate["visibleCommands"][1] = duplicate["visibleCommands"][0].clone();
            assert!(validate_process_snapshot(&duplicate, 501).is_err());
            let mut changed = initial.clone();
            changed["results"][0]["target"] = json!("system.kill.opaque");
            assert!(validate_process_snapshot(&changed, 501).is_err());
            for (key, value) in [
                ("selection", json!(-1)),
                ("selectedID", json!("fixture.process.0")),
                ("processSort", json!("Name")),
            ] {
                let mut changed = initial.clone();
                changed[key] = value;
                assert!(validate_process_snapshot(&changed, 501).is_err());
            }
            let mut absent = initial;
            absent["processResources"]
                .as_object_mut()
                .unwrap()
                .remove("fixture.process.999");
            assert!(validate_process_snapshot(&absent, 501).is_err());
        }
        #[test]
        fn late_callback_cannot_retarget_a_new_request() {
            let _registry = REPLY_REGISTRY_TEST.lock().unwrap();
            let first = NEXT.fetch_add(1, Ordering::AcqRel);
            let second = NEXT.fetch_add(1, Ordering::AcqRel);
            let state = Arc::new(Completion {
                result: Mutex::new(None),
                wake: None,
                source: None,
                guard: None,
                retired: AtomicBool::new(false),
                reply_received: AtomicBool::new(false),
                submission_started: AtomicBool::new(false),
            });
            PENDING.lock().unwrap().insert(second, state.clone());
            let bytes = b"{\"ok\":true}";
            reply(
                first as *mut c_void,
                bytes.as_ptr().cast(),
                bytes.len() as isize,
            );
            assert!(state.result.lock().unwrap().is_none());
            reply(
                second as *mut c_void,
                bytes.as_ptr().cast(),
                bytes.len() as isize,
            );
            reply(second as *mut c_void, std::ptr::null(), 0);
            let mut pending = Pending { key: second, state, _main: PhantomData };
            assert_eq!(pending.take().unwrap(), Some(json!({"ok":true})));
            assert!(pending.take().unwrap().is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_frames_reject_unknown_fields_and_lifecycle_phase_skips() {
        assert!(ResourceFrame::parse(json!({"phase":"checkpoint-probe","lane":"lifecycle.10000","cycle":0,"completed_cycles":1000,"index":0,"extra":true})).is_err());
        assert!(ResourceFrame::parse(json!({"phase":"checkpoint-probe","lane":"other","cycle":0,"completed_cycles":1000,"index":0})).is_err());
        let mut progress = LifecycleProgress::default();
        let request = |phase, cycle| Request {
            phase,
            lane: Lane::Lifecycle10000,
            id: "lifecycle.10000".into(),
            cycle,
        };
        assert!(progress.require_next(&request(Phase::Sample, 0)).is_err());
        for cycle in 0..100 {
            assert!(
                progress
                    .require_next(&request(Phase::Warmup, cycle + 1))
                    .is_err()
            );
            progress
                .require_next(&request(Phase::Warmup, cycle))
                .unwrap();
            progress.warmup += 1;
        }
        for cycle in 0..3 {
            assert!(progress.require_next(&request(Phase::Warmup, 0)).is_err());
            assert!(
                progress
                    .require_next(&request(Phase::Sample, cycle + 1))
                    .is_err()
            );
            progress
                .require_next(&request(Phase::Sample, cycle))
                .unwrap();
            progress.samples += 1;
        }
        assert!(progress.require_next(&request(Phase::Sample, 0)).is_err());
    }
    #[test]
    fn complete_lanes_preserve_empty_unicode_and_stage_identity() {
        let ids = Lane::ids();
        assert_eq!(ids.len(), 157);
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 157);
        for id in ids {
            assert!(Lane::parse(&id).is_ok(), "{id}");
        }
        assert_eq!(
            Lane::parse("input.10000.").unwrap(),
            Lane::Query {
                stage: QueryStage::Input,
                size: 10000,
                query: String::new()
            }
        );
        for bad in [
            "input.01000.m",
            "search.501.m",
            "search.500.a\0b",
            "search.500.M",
            "search.500.m.extra",
            "lifecycle.9999",
        ] {
            assert!(Lane::parse(bad).is_err(), "{bad}")
        }
    }
    #[test]
    fn strict_request_and_frames_keep_complete_values() {
        let bytes=b"{\"phase\":\"sample\",\"lane\":\"input.500.\",\"cycle\":4}\n{\"phase\":\"shutdown\"}\n";
        let mut f = Frames::new(&bytes[..], Vec::new());
        let r = Request::parse(f.read().unwrap()).unwrap();
        assert_eq!(r.cycle, 4);
        assert_eq!(f.read().unwrap(), json!({"phase":"shutdown"}));
        assert!(f.read().is_err());
        assert!(
            Request::parse(
                json!({"phase":"sample","lane":"search.500.m","cycle":0,"unknown":true})
            )
            .is_err()
        );
        let v = json!({"ordered":["résumé","a\0b",""],"disabled":false});
        f.write(&v).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&f.writer).unwrap(), v)
    }
    #[test]
    fn borrowed_stdio_cancel_and_flags_are_real_kernel_properties() {
        use std::{
            os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
        };
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let reader = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let original = [
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) },
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) },
        ];
        let stop = Arc::new(AtomicBool::new(false));
        let mut output = borrowed_stdio::Writer::new(writer.as_fd(), stop.clone()).unwrap();
        assert_eq!(
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) },
            original[1]
        );
        let descriptor_flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) };
        assert_eq!(output.write(&[7; 800]).unwrap(), 512);
        let mut input = borrowed_stdio::Reader::new(reader.as_fd(), stop.clone()).unwrap();
        let mut bytes = [0; 512];
        assert_eq!(input.read(&mut bytes).unwrap(), 512);
        assert_eq!(bytes, [7; 512]);
        assert_eq!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) },
            original[0]
        );
        // Darwin changes an opaque F_GETFL bit after a plain write as well.
        // Compare the entire word against an unwrapped own-pipe control rather
        // than masking an unexplained difference or blaming the wrapper.
        let mut control = [-1; 2];
        assert_eq!(unsafe { libc::pipe(control.as_mut_ptr()) }, 0);
        let _control_reader = unsafe { OwnedFd::from_raw_fd(control[0]) };
        let control_writer = unsafe { OwnedFd::from_raw_fd(control[1]) };
        assert_eq!(
            unsafe { libc::fcntl(control_writer.as_raw_fd(), libc::F_GETFL) },
            original[1]
        );
        assert_eq!(
            unsafe { libc::write(control_writer.as_raw_fd(), [7u8].as_ptr().cast(), 1) },
            1
        );
        assert_eq!(
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) },
            unsafe { libc::fcntl(control_writer.as_raw_fd(), libc::F_GETFL) }
        );
        assert_eq!(
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) },
            descriptor_flags
        );
        let cancellation = stop.clone();
        let task = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            cancellation.store(true, Ordering::Release);
        });
        let began = Instant::now();
        assert_eq!(
            input.read(&mut bytes).unwrap_err().kind(),
            std::io::ErrorKind::ConnectionAborted
        );
        assert!(began.elapsed() < Duration::from_secs(1));
        task.join().unwrap();
    }
    #[test]
    fn fixtures_preserve_original_payload_shape_without_private_data() {
        let commands = fixture::catalog(10000).unwrap();
        assert_eq!(commands.len(), 10000);
        assert_eq!(commands[42]["id"], "fixture.42");
        assert_eq!(commands[42]["aliases"], json!(["alias42"]));
        assert_eq!(commands[42]["title"], "Left Half 42");
        let text = fixture::clipboard_text(17).unwrap();
        assert_eq!(
            text,
            "Fixture 17: ".to_owned() + &"native clipboard Ω ".repeat(400)
        );
        let history = fixture::history(1700000000).unwrap();
        assert_eq!(history.len(), 200);
        assert_eq!(history[0]["date"], "2023-11-14T22:13:20Z");
        let recency = fixture::recency(1700000000);
        assert_eq!(recency["entries"].as_object().unwrap().len(), 512);
        assert_eq!(recency["entries"]["fixture.511"], 1700000000);
        let rgba = fixture::image_rgba();
        assert_eq!(rgba.len(), 1200 * 800 * 4);
        assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
    }
    #[test]
    fn no_ready_from_partial_routes_or_synthetic_drain() {
        assert!(require_complete_routes(&BTreeSet::new()).is_err());
        let r =
            Request::parse(json!({"phase":"sample","lane":"config.raw-read","cycle":0})).unwrap();
        assert!(
            Completed {
                elapsed_ns: 1,
                output: Value::Null,
                diagnostics: json!({})
            }
            .response(&r)
            .is_err()
        );
    }
}
