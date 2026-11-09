//! Bounded, volatile software-stage measurements. No input or command payloads.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Mutex, OnceLock},
    time::Instant,
};

const ACTIVE_LIMIT: usize = 32;
const SAMPLE_LIMIT: usize = 256;
const SCOPE: &str = "software stages; field reconciliation is not result drawing or physical presentation; host.drawCallback may be absent for layer-backed display; hotkey timing begins at handler delivery";

#[derive(Clone, Copy, Debug)]
pub enum Stage {
    SearchModel,
    InputModel,
    InputReconcile,
    OpenReady,
    HostDraw,
    HotkeyHandler,
}
impl Stage {
    const ALL: [Self; 6] = [
        Self::SearchModel,
        Self::InputModel,
        Self::InputReconcile,
        Self::OpenReady,
        Self::HostDraw,
        Self::HotkeyHandler,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::SearchModel => "search.model",
            Self::InputModel => "input.model",
            Self::InputReconcile => "input.reconcile",
            Self::OpenReady => "open.ready",
            Self::HostDraw => "host.drawCallback",
            Self::HotkeyHandler => "hotkey.handler",
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
#[derive(Default)]
struct State {
    sequence: u64,
    active: BTreeMap<u64, Entry>,
    metrics: [Metric; 6],
    pending_input: Option<(Span, u64)>,
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
        self.trace.finish(self.span, false);
    }
}
impl Trace {
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
        state.metrics = Default::default();
        state.pending_input = None;
    }
    pub fn snapshot(&self) -> Value {
        let (active, metrics) = {
            let state = self.lock();
            (state.active.len(), state.metrics.clone())
        };
        let mut stages = serde_json::Map::new();
        for stage in Stage::ALL {
            let metric = &metrics[stage.index()];
            let mut samples: Vec<_> = metric.samples.iter().copied().collect();
            samples.sort_by(f64::total_cmp);
            let percentile = |fraction: f64| {
                samples
                    .get(((samples.len().saturating_sub(1)) as f64 * fraction) as usize)
                    .copied()
            };
            stages.insert(stage.name().into(), json!({"count":metric.total,"retainedSamples":samples.len(),"cancelled":metric.cancelled,"p50MS":percentile(0.5),"p95MS":percentile(0.95),"maxMS":samples.last()}));
        }
        json!({"schema":2,"scope":SCOPE,"sampleLimitPerStage":SAMPLE_LIMIT,"activeIntervals":active,"stages":stages})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn schema_floor_percentiles_real_completion_and_reset_are_separate_from_analytics() {
        let trace = Trace::default();
        let empty = trace.snapshot();
        assert_eq!(empty["schema"], 2);
        assert_eq!(empty["scope"], SCOPE);
        assert_eq!(empty["stages"].as_object().unwrap().len(), 6);
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
        assert_eq!(metric["p95MS"], 9.);
        assert_eq!(metric["maxMS"], 10.);
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
}
