//! Bounded, volatile software-stage measurements. No input or command payloads.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{atomic::{AtomicBool, Ordering}, Mutex, OnceLock},
    time::{Duration, Instant},
};

const ACTIVE_LIMIT: usize = 32;
const SAMPLE_LIMIT: usize = 256;
const RUN_LIMIT: usize = 32;
const SCOPE: &str = "software stages; field reconciliation is not result drawing or physical presentation; host.drawCallback may be absent for layer-backed display; hotkey queue timing begins at callback receipt";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    SearchModel,
    ClipboardSearchSlice,
    InputModel,
    InputReconcile,
    OpenReady,
    HostDraw,
    HotkeyHandler,
    HotkeyQueue,
    OpenPublish,
    OpenMainQueue,
    OpenAppKit,
    OpenLayout,
    OpenReconcile,
    OpenKeyWindow,
    OpenActivate,
    OpenFocus,
    MenuHealth,
    MenuHealthState,
    MenuHealthActivation,
    MenuHealthLaunch,
    MenuHealthTerminate,
    MenuHealthAccessibility,
    MenuHealthInputMonitoring,
    MenuHealthRaycast,
    LauncherBecomeKey,
    LauncherOutsideMonitors,
    LauncherMakeKey,
    LauncherOrderFront,
    ScriptHelperSpawn,
    ScriptHelperReady,
    ScriptDispatch,
    ScriptResponse,
    ScriptFinalize,
    LauncherOpenFirst,
    LauncherOpenLater,
    LauncherFieldFocus,
    LauncherActiveSpace,
    LauncherFocusFallback,
    LauncherParkedCheck,
    LauncherParkedFallback,
    ScriptCapability,
}
#[derive(Clone, Copy)]
pub enum MenuHealthTrigger { State, Activation, Launch, Terminate }
impl MenuHealthTrigger {
    fn stage(self) -> Stage {
        match self {
            Self::State => Stage::MenuHealthState,
            Self::Activation => Stage::MenuHealthActivation,
            Self::Launch => Stage::MenuHealthLaunch,
            Self::Terminate => Stage::MenuHealthTerminate,
        }
    }
}
impl Stage {
    const ALL: [Self; 41] = [
        Self::SearchModel,
        Self::ClipboardSearchSlice,
        Self::InputModel,
        Self::InputReconcile,
        Self::OpenReady,
        Self::HostDraw,
        Self::HotkeyHandler,
        Self::HotkeyQueue,
        Self::OpenPublish,
        Self::OpenMainQueue,
        Self::OpenAppKit,
        Self::OpenLayout,
        Self::OpenReconcile,
        Self::OpenKeyWindow,
        Self::OpenActivate,
        Self::OpenFocus,
        Self::MenuHealth,
        Self::MenuHealthState,
        Self::MenuHealthActivation,
        Self::MenuHealthLaunch,
        Self::MenuHealthTerminate,
        Self::MenuHealthAccessibility,
        Self::MenuHealthInputMonitoring,
        Self::MenuHealthRaycast,
        Self::LauncherBecomeKey,
        Self::LauncherOutsideMonitors,
        Self::LauncherMakeKey,
        Self::LauncherOrderFront,
        Self::ScriptHelperSpawn,
        Self::ScriptHelperReady,
        Self::ScriptDispatch,
        Self::ScriptResponse,
        Self::ScriptFinalize,
        Self::LauncherOpenFirst,
        Self::LauncherOpenLater,
        Self::LauncherFieldFocus,
        Self::LauncherActiveSpace,
        Self::LauncherFocusFallback,
        Self::LauncherParkedCheck,
        Self::LauncherParkedFallback,
        Self::ScriptCapability,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::SearchModel => "search.model",
            Self::ClipboardSearchSlice => "clipboard.searchSlice",
            Self::InputModel => "input.model",
            Self::InputReconcile => "input.reconcile",
            Self::OpenReady => "open.ready",
            Self::HostDraw => "host.drawCallback",
            Self::HotkeyHandler => "hotkey.handler",
            Self::HotkeyQueue => "hotkey.queue",
            Self::OpenPublish => "open.publish",
            Self::OpenMainQueue => "open.mainQueue",
            Self::OpenAppKit => "open.appKit",
            Self::OpenLayout => "open.layout",
            Self::OpenReconcile => "open.reconcile",
            Self::OpenKeyWindow => "open.keyWindow",
            Self::OpenActivate => "open.activate",
            Self::OpenFocus => "open.focus",
            Self::MenuHealth => "menu.health",
            Self::MenuHealthState => "menu.health.state",
            Self::MenuHealthActivation => "menu.health.activation",
            Self::MenuHealthLaunch => "menu.health.launch",
            Self::MenuHealthTerminate => "menu.health.terminate",
            Self::MenuHealthAccessibility => "menu.health.accessibility",
            Self::MenuHealthInputMonitoring => "menu.health.inputMonitoring",
            Self::MenuHealthRaycast => "menu.health.raycast",
            Self::LauncherBecomeKey => "launcher.becomeKey",
            Self::LauncherOutsideMonitors => "launcher.outsideMonitors",
            Self::LauncherMakeKey => "launcher.makeKey",
            Self::LauncherOrderFront => "launcher.orderFront",
            Self::ScriptHelperSpawn => "script.helperSpawn",
            Self::ScriptHelperReady => "script.helperReady",
            Self::ScriptDispatch => "script.dispatch",
            Self::ScriptResponse => "script.response",
            Self::ScriptFinalize => "script.finalize",
            Self::LauncherOpenFirst => "launcher.openFirst",
            Self::LauncherOpenLater => "launcher.openLater",
            Self::LauncherFieldFocus => "launcher.fieldFocus",
            Self::LauncherActiveSpace => "launcher.activeSpace",
            Self::LauncherFocusFallback => "launcher.focusFallback",
            Self::LauncherParkedCheck => "launcher.parkedCheck",
            Self::LauncherParkedFallback => "launcher.parkedFallback",
            Self::ScriptCapability => "script.writerCapability",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

/// A private monotonic identity; copies cannot complete an interval twice.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    id: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunKind { Launcher, Script }
#[derive(Clone, Copy, Debug)]
pub(crate) enum RunOutcome { Ready, OwnerProcessed, SpawnFailed, Cancelled, Handoff }
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run { id: Option<u64> }
#[derive(Clone, Copy, Debug)]
pub(crate) enum RunPoint { WorkerStart, WorkerResult, OwnerHarvest, OpenPublish, OpenMainQueue, OpenAppKit, CapabilityChecked, HelperSpawned, HelperReady, Dispatched, ResponseReceived, HelperExited, FinalizeStart }
impl RunPoint {
    fn index(self) -> usize { self as usize }
    fn name(self) -> &'static str { match self {
        Self::WorkerStart => "workerStart", Self::WorkerResult => "workerResult", Self::OwnerHarvest => "ownerHarvest",
        Self::OpenPublish => "ownerStart", Self::OpenMainQueue => "published", Self::OpenAppKit => "appKitStart",
        Self::CapabilityChecked => "capabilityPhaseEnd", Self::HelperSpawned => "helperSpawned", Self::HelperReady => "helperReady", Self::Dispatched => "dispatched", Self::ResponseReceived => "responseReceived", Self::HelperExited => "helperExited", Self::FinalizeStart => "ownerFinalizeStart",
    } }
}
struct RunEntry { kind: RunKind, started: Instant, points: [Option<f64>; 13] }
#[derive(Clone)]
struct CompletedRun { kind: RunKind, outcome: RunOutcome, elapsed_ms: f64, points: [Option<f64>; 13] }
impl CompletedRun {
    fn json(&self) -> Value {
        let mut points = serde_json::Map::new();
        for point in [RunPoint::WorkerStart, RunPoint::WorkerResult, RunPoint::OwnerHarvest, RunPoint::OpenPublish, RunPoint::OpenMainQueue, RunPoint::OpenAppKit, RunPoint::CapabilityChecked, RunPoint::HelperSpawned, RunPoint::HelperReady, RunPoint::Dispatched, RunPoint::ResponseReceived, RunPoint::HelperExited, RunPoint::FinalizeStart] {
            if let Some(value) = self.points[point.index()] { points.insert(point.name().into(), json!(value)); }
        }
        json!({"kind": match self.kind { RunKind::Launcher => "paletteOpen", RunKind::Script => "script" }, "outcome": match self.outcome { RunOutcome::Ready => "ready", RunOutcome::OwnerProcessed => "ownerProcessed", RunOutcome::SpawnFailed => "spawnFailed", RunOutcome::Cancelled => "cancelled", RunOutcome::Handoff => "handoff" }, "elapsedMS": self.elapsed_ms, "pointsMS": points})
    }
}
struct Entry {
    started: Instant,
    stage: Stage,
    signpost: Option<crate::performance_signposts::Interval>,
}
#[derive(Clone, Default)]
struct Metric {
    total: u64,
    cancelled: u64,
    samples: VecDeque<f64>,
}
impl Metric {
    fn finish(&mut self, milliseconds: f64, cancelled: bool) {
        if cancelled {
            self.cancelled = self.cancelled.saturating_add(1);
            return;
        }
        self.total = self.total.saturating_add(1);
        if self.samples.len() == SAMPLE_LIMIT {
            self.samples.pop_front();
        }
        self.samples.push_back(milliseconds);
    }
}
struct State {
    sequence: u64,
    active: BTreeMap<u64, Entry>,
    metrics: [Metric; Stage::ALL.len()],
    pending_input: Option<(Span, u64)>,
    runs: BTreeMap<u64, RunEntry>,
    completed_runs: VecDeque<CompletedRun>,
}
impl Default for State {
    fn default() -> Self {
        Self { sequence: 0, active: BTreeMap::new(), metrics: std::array::from_fn(|_| Metric::default()), pending_input: None, runs: BTreeMap::new(), completed_runs: VecDeque::new() }
    }
}
impl State {
    fn begin(&mut self, stage: Stage) -> Span {
        // Reset never reuses identities. Exhaustion cannot resurrect a stale span.
        let Some(id) = self.sequence.checked_add(1) else {
            return Span { id: None };
        };
        self.sequence = id;
        if self.active.len() >= ACTIVE_LIMIT {
            if let Some((_, oldest)) = self.active.pop_first() {
                if let Some(interval) = oldest.signpost {
                    crate::performance_signposts::end(interval);
                }
                self.metrics[oldest.stage.index()].finish(0., true);
            }
        }
        // Match Swift: first close an evicted interval, then start the clock
        // before the fixed OS begin call. No application callback runs here.
        let started = Instant::now();
        let signpost = crate::performance_signposts::begin(stage);
        self.active.insert(
            id,
            Entry {
                started,
                stage,
                signpost,
            },
        );
        Span { id: Some(id) }
    }
    fn finish(&mut self, span: Span, cancelled: bool) {
        if let Some(entry) = span.id.and_then(|id| self.active.remove(&id)) {
            if let Some(interval) = entry.signpost {
                crate::performance_signposts::end(interval);
            }
            let ms = entry.started.elapsed().as_secs_f64() * 1000.;
            self.metrics[entry.stage.index()].finish(ms, cancelled);
        }
    }
}

#[derive(Default)]
pub struct Trace {
    state: Mutex<State>,
    launcher_open_seen: AtomicBool,
}
pub fn shared() -> &'static Trace {
    static TRACE: OnceLock<Trace> = OnceLock::new();
    TRACE.get_or_init(Trace::default)
}
pub struct Guard<'a> {
    trace: &'a Trace,
    span: Span,
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if self.span.id.is_some() {
            self.trace.finish(self.span, false);
        }
    }
}
impl Trace {
    pub(crate) fn begin_run(&self, kind: RunKind, started: Instant) -> Run {
        let mut state = self.lock();
        let Some(id) = state.sequence.checked_add(1) else { return Run { id: None }; };
        state.sequence = id;
        if state.runs.len() >= ACTIVE_LIMIT { state.runs.pop_first(); }
        state.runs.insert(id, RunEntry { kind, started, points: [None; 13] });
        Run { id: Some(id) }
    }
    pub(crate) fn mark_run(&self, run: Run, point: RunPoint) {
        let mut state = self.lock();
        if let Some(entry) = run.id.and_then(|id| state.runs.get_mut(&id)) {
            if entry.points[point.index()].is_none() {
                entry.points[point.index()] = Some(entry.started.elapsed().as_secs_f64() * 1000.);
            }
        }
    }
    pub(crate) fn finish_run(&self, run: Run, outcome: RunOutcome) {
        let mut state = self.lock();
        if let Some(entry) = run.id.and_then(|id| state.runs.remove(&id)) {
            let value = CompletedRun { kind: entry.kind, outcome, elapsed_ms: entry.started.elapsed().as_secs_f64() * 1000., points: entry.points };
            if state.completed_runs.len() == RUN_LIMIT { state.completed_runs.pop_front(); }
            state.completed_runs.push_back(value);
        }
    }
    pub fn launcher_open(&self) -> Guard<'_> {
        let stage = if self.launcher_open_seen.swap(true, Ordering::Relaxed) {
            Stage::LauncherOpenLater
        } else {
            Stage::LauncherOpenFirst
        };
        self.scope(stage)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn begin(&self, stage: Stage) -> Span {
        // Fixed log-handle initialization stays outside the measured interval.
        crate::performance_signposts::prepare();
        self.lock().begin(stage)
    }
    pub fn finish(&self, span: Span, cancelled: bool) {
        self.lock().finish(span, cancelled);
    }
    pub(crate) fn begin_open(&self) -> (Span, Span) {
        crate::performance_signposts::prepare();
        let mut state = self.lock();
        (state.begin(Stage::OpenReady), state.begin(Stage::OpenPublish))
    }
    pub(crate) fn advance(&self, parent: Span, previous: Span, next: Stage) -> Option<Span> {
        let mut state = self.lock();
        if !parent.id.is_some_and(|id| state.active.contains_key(&id))
            || !previous.id.is_some_and(|id| state.active.contains_key(&id))
        {
            return None;
        }
        state.finish(previous, false);
        Some(state.begin(next))
    }
    pub fn record(&self, stage: Stage, elapsed: Duration) {
        self.lock().metrics[stage.index()].finish(elapsed.as_secs_f64() * 1000., false);
    }
    pub fn scope(&self, stage: Stage) -> Guard<'_> {
        Guard {
            trace: self,
            span: self.begin(stage),
        }
    }
    pub fn measure<T>(&self, stage: Stage, work: impl FnOnce() -> T) -> T {
        let _scope = self.scope(stage);
        work()
    }
    pub fn measure_menu_health(
        &self,
        reason: MenuHealthTrigger,
        accessibility: impl FnOnce() -> bool,
        input_monitoring: impl FnOnce() -> bool,
        raycast_absent: impl FnOnce() -> bool,
    ) -> bool {
        fn timed(work: impl FnOnce() -> bool) -> (bool, Duration) {
            let began = Instant::now();
            let result = work();
            (result, began.elapsed())
        }
        let mut guard = self.scope(Stage::MenuHealth);
        let start = Instant::now();
        let mut elapsed = [None; 3];
        let (ax, ax_time) = timed(accessibility);
        elapsed[0] = Some(ax_time);
        let healthy = if ax {
            let (input, input_time) = timed(input_monitoring);
            elapsed[1] = Some(input_time);
            if input {
                let (raycast, raycast_time) = timed(raycast_absent);
                elapsed[2] = Some(raycast_time);
                raycast
            } else { false }
        } else { false };
        let ended = Instant::now();
        let total = ended.duration_since(start);
        let span = std::mem::replace(&mut guard.span, Span { id: None });
        let mut state = self.lock();
        let Some(entry) = span.id.and_then(|id| state.active.remove(&id)) else {
            return healthy;
        };
        if let Some(interval) = entry.signpost {
            crate::performance_signposts::end(interval);
        }
        state.metrics[Stage::MenuHealth.index()].finish(ended.duration_since(entry.started).as_secs_f64() * 1000., false);
        state.metrics[reason.stage().index()].finish(total.as_secs_f64() * 1000., false);
        for (stage, duration) in [Stage::MenuHealthAccessibility, Stage::MenuHealthInputMonitoring, Stage::MenuHealthRaycast]
            .into_iter().zip(elapsed)
        {
            if let Some(duration) = duration {
                state.metrics[stage.index()].finish(duration.as_secs_f64() * 1000., false);
            }
        }
        healthy
    }
    pub fn input_changed(&self, owner: u64) {
        crate::performance_signposts::prepare();
        let mut state = self.lock();
        if let Some((span, _)) = state.pending_input.take() {
            state.finish(span, true);
        }
        let span = state.begin(Stage::InputReconcile);
        state.pending_input = Some((span, owner));
    }
    pub fn did_reconcile_field(&self, owner: u64) {
        let mut state = self.lock();
        if state.pending_input.is_some_and(|(_, field)| field == owner) {
            if let Some((span, _)) = state.pending_input.take() {
                state.finish(span, false);
            }
        }
    }
    pub fn cancel_input(&self, owner: Option<u64>) {
        let mut state = self.lock();
        if state
            .pending_input
            .is_some_and(|(_, field)| owner.is_none_or(|owner| owner == field))
        {
            if let Some((span, _)) = state.pending_input.take() {
                state.finish(span, true);
            }
        }
    }
    pub fn reset(&self) {
        let mut state = self.lock();
        state.active.clear();
        state.metrics = std::array::from_fn(|_| Metric::default());
        state.pending_input = None;
        state.runs.clear();
        state.completed_runs.clear();
    }
    pub fn snapshot(&self) -> Value {
        let (active, metrics, active_runs, completed_runs) = {
            let state = self.lock();
            (state.active.len(), state.metrics.clone(), state.runs.len(), state.completed_runs.clone())
        };
        let mut stages = serde_json::Map::new();
        for stage in Stage::ALL {
            let metric = &metrics[stage.index()];
            let mut samples: Vec<_> = metric.samples.iter().copied().collect();
            samples.sort_by(f64::total_cmp);
            let percentile = |fraction: f64| {
                samples
                    .get(((samples.len() as f64 * fraction).ceil() as usize).saturating_sub(1))
                    .copied()
            };
            stages.insert(stage.name().into(), json!({"count":metric.total,"retainedSamples":samples.len(),"cancelled":metric.cancelled,"p50MS":percentile(0.5),"p95MS":percentile(0.95),"maxMS":samples.last()}));
        }
        let completed_runs: Vec<_> = completed_runs.iter().map(CompletedRun::json).collect();
        json!({"schema":2,"scope":SCOPE,"sampleLimitPerStage":SAMPLE_LIMIT,"activeIntervals":active,"activeRuns":active_runs,"completedRunLimit":RUN_LIMIT,"completedRuns":completed_runs,"stages":stages})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_health_preserves_short_circuit_and_counts_only_executed_checks() {
        use std::cell::RefCell;
        let trace = Trace::default();
        for (pass_ax, pass_input, pass_raycast, expected) in [
            (false, true, true, vec!["ax"]),
            (true, false, true, vec!["ax", "input"]),
            (true, true, false, vec!["ax", "input", "raycast"]),
            (true, true, true, vec!["ax", "input", "raycast"]),
        ] {
            let calls = RefCell::new(Vec::new());
            assert_eq!(trace.measure_menu_health(MenuHealthTrigger::Launch,
                || { calls.borrow_mut().push("ax"); pass_ax },
                || { calls.borrow_mut().push("input"); pass_input },
                || { calls.borrow_mut().push("raycast"); pass_raycast },
            ), pass_ax && pass_input && pass_raycast);
            assert_eq!(*calls.borrow(), expected);
        }
        let snapshot = trace.snapshot();
        assert_eq!(snapshot["stages"]["menu.health.launch"]["count"], 4);
        assert_eq!(snapshot["stages"]["menu.health.accessibility"]["count"], 4);
        assert_eq!(snapshot["stages"]["menu.health.inputMonitoring"]["count"], 3);
        assert_eq!(snapshot["stages"]["menu.health.raycast"]["count"], 2);
        trace.reset();
        assert_eq!(trace.snapshot()["stages"]["menu.health.launch"]["count"], 0);
    }
    #[test]
    fn menu_health_reset_during_preflight_discards_the_entire_measurement() {
        let trace = Trace::default();
        assert!(trace.measure_menu_health(MenuHealthTrigger::Activation,
            || { trace.reset(); true }, || true, || true));
        let snapshot = trace.snapshot();
        for stage in ["menu.health", "menu.health.activation", "menu.health.accessibility", "menu.health.inputMonitoring", "menu.health.raycast"] {
            assert_eq!(snapshot["stages"][stage]["count"], 0, "{stage}");
        }
    }
    #[test]
    fn correlated_run_outlives_legacy_span_eviction() {
        let trace = Trace::default();
        let (ready, publish) = trace.begin_open();
        let run = trace.begin_run(RunKind::Launcher, Instant::now());
        let spans: Vec<_> = (0..31).map(|_| trace.begin(Stage::SearchModel)).collect();
        assert!(trace.advance(ready, publish, Stage::OpenMainQueue).is_none());
        trace.finish(publish, true);
        trace.mark_run(run, RunPoint::OpenMainQueue);
        trace.mark_run(run, RunPoint::OpenAppKit);
        trace.finish_run(run, RunOutcome::Ready);
        let snapshot = trace.snapshot();
        assert_eq!(snapshot["completedRuns"][0]["outcome"], "ready");
        assert_eq!(snapshot["stages"]["open.publish"]["count"], 0);
        assert_eq!(snapshot["stages"]["open.publish"]["cancelled"], 1);
        for span in spans { trace.finish(span, true); }
    }
    #[test]
    fn correlated_runs_are_bounded_private_and_reset_does_not_resurrect_old_work() {
        let trace = Trace::default();
        let old = trace.begin_run(RunKind::Script, Instant::now());
        let other = trace.begin_run(RunKind::Launcher, Instant::now());
        trace.mark_run(old, RunPoint::WorkerStart);
        trace.finish_run(other, RunOutcome::Cancelled);
        trace.reset();
        trace.mark_run(old, RunPoint::WorkerResult);
        trace.finish_run(old, RunOutcome::OwnerProcessed);
        assert_eq!(trace.snapshot()["completedRuns"].as_array().unwrap().len(), 0);
        let evicted = trace.begin_run(RunKind::Script, Instant::now());
        let live: Vec<_> = (0..ACTIVE_LIMIT).map(|_| trace.begin_run(RunKind::Script, Instant::now())).collect();
        trace.finish_run(evicted, RunOutcome::OwnerProcessed);
        assert_eq!(trace.snapshot()["completedRuns"].as_array().unwrap().len(), 0);
        for run in live { trace.finish_run(run, RunOutcome::Cancelled); }
        trace.reset();
        for _ in 0..(RUN_LIMIT + 3) {
            let run = trace.begin_run(RunKind::Script, Instant::now());
            trace.mark_run(run, RunPoint::WorkerStart);
            trace.mark_run(run, RunPoint::WorkerResult);
            trace.mark_run(run, RunPoint::OwnerHarvest);
            trace.finish_run(run, RunOutcome::OwnerProcessed);
            trace.finish_run(run, RunOutcome::Ready);
        }
        let snapshot = trace.snapshot();
        let runs = snapshot["completedRuns"].as_array().unwrap();
        assert_eq!(runs.len(), RUN_LIMIT);
        for run in runs {
            assert_eq!(run["kind"], "script");
            assert_eq!(run["outcome"], "ownerProcessed");
            let start = run["pointsMS"]["workerStart"].as_f64().unwrap();
            let end = run["pointsMS"]["workerResult"].as_f64().unwrap();
            assert!(start <= end && end <= run["elapsedMS"].as_f64().unwrap());
            assert!(!run.to_string().contains("path"));
        }
    }
    #[test]
    fn original_retention_field_ownership_cancellation_reset_and_overflow_contract() {
        let trace = Trace::default();
        for _ in 0..350 {
            trace.measure(Stage::SearchModel, || {});
        }
        let value = trace.snapshot();
        assert_eq!(value["stages"]["search.model"]["count"], 350);
        assert_eq!(value["stages"]["search.model"]["retainedSamples"], 256);
        trace.input_changed(1);
        trace.did_reconcile_field(2);
        trace.cancel_input(Some(2));
        assert_eq!(trace.snapshot()["activeIntervals"], 1);
        trace.input_changed(1);
        trace.cancel_input(None);
        assert_eq!(
            trace.snapshot()["stages"]["input.reconcile"]["cancelled"],
            2
        );
        assert_eq!(trace.snapshot()["stages"]["input.reconcile"]["count"], 0);
        let stale = trace.begin(Stage::OpenReady);
        trace.reset();
        let fresh = trace.begin(Stage::OpenReady);
        trace.finish(stale, false);
        assert_eq!(trace.snapshot()["activeIntervals"], 1);
        trace.finish(fresh, false);
        let spans: Vec<_> = (0..100).map(|_| trace.begin(Stage::InputModel)).collect();
        assert_eq!(trace.snapshot()["activeIntervals"], 32);
        for span in spans {
            trace.finish(span, true);
        }
        assert_eq!(trace.snapshot()["stages"]["input.model"]["cancelled"], 100);
        trace.reset();
        assert_eq!(trace.snapshot()["activeIntervals"], 0);
    }
    #[test]
    fn schema_nearest_rank_percentiles_real_completion_and_reset_are_separate_from_analytics() {
        let trace = Trace::default();
        let empty = trace.snapshot();
        assert_eq!(empty["schema"], 2);
        assert_eq!(empty["scope"], SCOPE);
        assert_eq!(empty["stages"].as_object().unwrap().len(), Stage::ALL.len());
        for metric in empty["stages"].as_object().unwrap().values() {
            assert_eq!(metric["count"], 0);
            assert!(metric["p50MS"].is_null());
            assert!(metric["p95MS"].is_null());
            assert!(metric["maxMS"].is_null());
        }
        {
            let mut state = trace.lock();
            for n in 1..=10 {
                // Percentile arithmetic uses fixed metric data; production
                // timestamps always surround the actual OS begin/end calls.
                state.metrics[Stage::HostDraw.index()].finish(f64::from(n), false);
            }
        }
        let result = trace.snapshot();
        let metric = &result["stages"]["host.drawCallback"];
        assert_eq!(metric["p50MS"], 5.);
        assert_eq!(metric["p95MS"], 10.);
        assert_eq!(metric["maxMS"], 10.);
        trace.record(Stage::LauncherFieldFocus, Duration::from_millis(54));
        trace.record(Stage::LauncherFieldFocus, Duration::from_millis(1459));
        assert_eq!(trace.snapshot()["stages"]["launcher.fieldFocus"]["p95MS"], 1459.);
        trace.input_changed(u64::MAX);
        trace.did_reconcile_field(u64::MAX);
        assert_eq!(trace.snapshot()["stages"]["input.reconcile"]["count"], 1);
        let span = trace.begin(Stage::HotkeyHandler);
        trace.finish(span, false);
        trace.finish(span, false);
        assert_eq!(trace.snapshot()["stages"]["hotkey.handler"]["count"], 1);
        assert!(!trace.snapshot().to_string().contains(&u64::MAX.to_string()));
        trace.reset();
        assert!(trace.snapshot()["stages"]["host.drawCallback"]["p50MS"].is_null());
    }
    #[test]
    fn reset_cannot_start_another_phase_of_an_old_open() {
        let trace = Trace::default();
        let ready = trace.begin(Stage::OpenReady);
        let publish = trace.begin(Stage::OpenPublish);
        let queued = trace.advance(ready, publish, Stage::OpenMainQueue).unwrap();
        trace.reset();
        assert!(trace.advance(ready, queued, Stage::OpenAppKit).is_none());
        trace.finish(ready, false);
        let value = trace.snapshot();
        assert_eq!(value["activeIntervals"], 0);
        assert_eq!(value["stages"]["open.ready"]["count"], 0);
        assert_eq!(value["stages"]["open.appKit"]["count"], 0);
    }
    #[test]
    fn first_launcher_open_is_per_process_and_survives_measurement_reset() {
        let trace = Trace::default();
        drop(trace.launcher_open());
        drop(trace.launcher_open());
        let snapshot = trace.snapshot();
        assert_eq!(snapshot["stages"]["launcher.openFirst"]["count"], 1);
        assert_eq!(snapshot["stages"]["launcher.openLater"]["count"], 1);
        trace.reset();
        drop(trace.launcher_open());
        let after = trace.snapshot();
        assert_eq!(after["stages"]["launcher.openFirst"]["count"], 0);
        assert_eq!(after["stages"]["launcher.openLater"]["count"], 1);
    }
}
