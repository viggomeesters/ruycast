use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
// Static compiler readers retain independently callable diagnostic entrypoints.
#[allow(dead_code)]
mod callbacks;
mod collection;
mod preparation;
mod resources;

type Result<T> = std::result::Result<T, String>;
const SCHEMA: &str = "ruycast.f1.complete.v1";
const BOUNDARIES: &[&str] = &[
    "profile-home",
    "account-lookup",
    "clipboard",
    "workspace-apps-icons",
    "windows-menus-accessibility",
    "screens-mouse",
    "hotkeys-input-permissions",
    "safari-appleevents",
    "shortcuts-processes",
    "monitor-host-process-network-battery",
    "provider-network-keychain",
    "open-activate-focus",
    "filesystem-owned",
    "startup-before-initializers",
];
const FUNCTIONAL_BLOCKS: &[&str] = &[
    "state-persistence",
    "cli-30-wire-operations",
    "native-gui-actions-keyboard",
    "clipboard-editor-snippets",
    "system-utilities",
    "private-adapter-capture",
    "privacy-errors-cancellation",
    "delivery-default-startup",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub id: String,
    pub dimension: String,
    pub samples_per_arm: usize,
    pub warmup_per_arm: usize,
    pub unit: String,
    pub notes: String,
    pub semantic_output: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol {
    schema: String,
    lanes: Vec<Lane>,
    ordered_outputs: bool,
    physical_display_battery_excluded: bool,
    schedule: String,
    seed: u64,
}

fn protocol() -> Protocol {
    let mut lanes = vec![];
    let mut add = |id: &str, dimension: &str, n: usize, warmup: usize, unit: &str, notes: &str| {
        lanes.push(Lane {
            id: id.into(),
            dimension: dimension.into(),
            samples_per_arm: n,
            warmup_per_arm: warmup,
            unit: unit.into(),
            notes: notes.into(),
            semantic_output: !id.starts_with("artifact.") && !id.starts_with("build."),
        })
    };
    add("startup.process-cold", "startup", 1000, 0, "ns", "Fresh owned process, same seeded profile; filesystem cache not purged. Through ready, no normal application launch.");
    add("startup.profile-cold", "startup", 1000, 0, "ns", "Fresh owned process and empty owned profile; include initialization and durable ready state.");
    add("startup.warm-open", "startup", 1000, 100, "ns", "Same initialized owned process; show-to-hidden-native-ready, no activation or physical display claim.");
    for size in [500, 1000, 5000, 10000] {
        for query in [
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
        ] {
            for stage in ["search", "reconciliation", "input"] {
                add(&format!("{stage}.{size}.{query}"), stage, 1000, 100, "ns", "Identical complete ordered rows, selection, aliases, actions, enabled state and recency. Native reconciliation/input requires production coordinator and hidden own views.");
            }
        }
    }
    for id in [
        "search.recency512",
        "parity.unicode-nul-disabled-nfc",
        "search.cached-result-read",
        "processes.cached-result-read1000",
        "clipboard.command-search-with-history",
        "clipboard.enqueue200x8000",
        "clipboard.completion200x8000",
        "clipboard.prune200x8000",
        "clipboard.encode200x8000",
        "clipboard.preview960000",
        "config.raw-read",
        "analytics.timing-record",
        "icons.warm-owned-app",
        "clipboard.filter200x8000",
        "clipboard.image960000",
        "clipboard.history-prune-encode",
        "config.reload",
        "processes.sort1000",
        "actions.alias-hotkey",
        "editor.rich-template",
        "snippet.keyboard-choice",
        "script.provider-choice",
        "capture.result-daily-error",
        "fault.timeout-cancel-transport",
        "fault.persistence-external-edit",
        "fault.stale-selection-permission",
    ] {
        add(id, "complete-functional-workload", 1000, 100, "ns", "Original end-to-end owner/grant/effect/completion and persisted state; fixture effects only. Compare full result/error/privacy/cancellation dispositions.");
    }
    add("copies-allocations.input", "copies-allocations", 1000, 100, "diagnostic", "Symmetric separate diagnostic run. Report observed dynamic malloc/realloc/free and memcpy/memmove calls/bytes only; inlined copies, stack and ARC not inferred. Never subtract instrumentation overhead.");
    add("lifecycle.10000", "lifecycle", 3, 100, "diagnostic", "Exactly 10000 complete hidden native cycles; stable natural settle after warmup and 1000/5000/10000. No allocator relief. Physical footprint growth <=10MiB at every checkpoint; preserve every settling observation and pending-owner counts.");
    add("idle.60seconds", "resources", 5, 0, "diagnostic", "Own child PID only: CPU ns, interrupt+idle wakeups, disk bytes, faults, RSS and physical footprint. Real 60second monotonic interval, complete ready host idle, no injected synthetic resource readings.");
    for id in [
        "artifact.binary",
        "artifact.complete-app",
        "artifact.linked-runtimes",
    ] {
        add(id, "artifacts", 1, 0, "diagnostic", "Production optimized artifact independent from test-instrumented executable; complete regular-file byte count, architecture, signatures/frameworks and linked runtimes.");
    }
    for id in ["build.cold", "build.cached"] {
        add(id, "build", 31, 1, "ns", "Same offline dependency cache and full product target. Cold uses new owned target, cached rebuilds identical source; preserve command/compiler/flags, wall/CPU/I/O. p99 sparse (<1000) explicitly not tail-confidence proof.");
    }
    Protocol {
        schema: SCHEMA.into(),
        lanes,
        ordered_outputs: true,
        physical_display_battery_excluded: true,
        schedule: "paired alternating AB/BA; cycle 0 A,B; cycle 1 B,A; no adaptive retries".into(),
        seed: 0xF1000026,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    path: PathBuf,
    sha256: String,
}
impl Binding {
    fn verify(&self) -> Result<()> {
        let metadata =
            fs::symlink_metadata(&self.path).map_err(|e| format!("binding metadata: {e}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("binding must be a regular non-symlink file".into());
        }
        if sha(&fs::read(&self.path).map_err(|e| e.to_string())?) != self.sha256 {
            return Err(format!("binding changed: {}", self.path.display()));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    schema: String,
    decision: String,
    final_functional_freeze: bool,
    functional_blocks: BTreeSet<String>,
    protocol: Binding,
    fixture: Binding,
    baseline_source: Binding,
    candidate_source: Binding,
    baseline_artifact: Binding,
    candidate_artifact: Binding,
    baseline_pre_entry_audit: Binding,
    candidate_pre_entry_audit: Binding,
    collection_adapters: Binding,
    runner_artifact: Binding,
    counter_units: Binding,
    boundaries: BTreeSet<String>,
    baseline_version: String,
    baseline_commit: String,
    candidate_version: String,
    baseline_optimized: bool,
    candidate_optimized: bool,
    same_instrumentation: bool,
    full_workloads: bool,
    exclusions: Vec<String>,
}
fn check_authority(file: &Path, pinned: &str) -> Result<Authority> {
    let raw = fs::read(file).map_err(|e| e.to_string())?;
    if pinned.len() != 64 || sha(&raw) != pinned {
        return Err("authority SHA-256 must match the independent review/final-freeze receipt supplied by root".into());
    }
    let a: Authority = serde_json::from_slice(&raw).map_err(|e| format!("authority: {e}"))?;
    if a.schema != SCHEMA
        || a.decision != "accepted"
        || !a.final_functional_freeze
        || !a.full_workloads
        || !a.same_instrumentation
        || !a.baseline_optimized
        || !a.candidate_optimized
    {
        return Err("incomplete or incomparable product freeze; preparation measurements cannot become a whole-product pass".into());
    }
    if a.functional_blocks != FUNCTIONAL_BLOCKS.iter().map(|s| (*s).into()).collect()
        || a.boundaries != BOUNDARIES.iter().map(|s| (*s).into()).collect()
    {
        return Err("missing or unknown functional/SDK pre-entry audit scope".into());
    }
    if a.baseline_version != "0.8.26"
        || a.baseline_commit != "6e3ae12e5e1734f36db9361b80eef0ade8db8a44"
    {
        return Err("current baseline must be immutable shipped 0.8.26; retain 0.8.25 separately as history".into());
    }
    for b in [
        &a.protocol,
        &a.fixture,
        &a.baseline_source,
        &a.candidate_source,
        &a.baseline_artifact,
        &a.candidate_artifact,
        &a.baseline_pre_entry_audit,
        &a.candidate_pre_entry_audit,
        &a.collection_adapters,
        &a.runner_artifact,
        &a.counter_units,
    ] {
        b.verify()?;
    }
    preparation::verify(&a.baseline_source)?;
    preparation::verify(&a.candidate_source)?;
    if fs::canonicalize(&a.runner_artifact.path).map_err(|e| e.to_string())?
        != std::env::current_exe().map_err(|e| e.to_string())?
    {
        return Err("reviewed measurement producer differs from executing runner".into());
    }
    let units: Value = read(&a.counter_units.path)?;
    if units["ok"] != true
        || units["ratio"]
            .as_f64()
            .is_none_or(|r| !(0.95..1.05).contains(&r))
        || integer(&units, "timebase_denom")? == 0
        || integer(&units, "timebase_numer")? == 0
    {
        return Err("owned CPU counter-unit calibration missing or failed".into());
    }
    let p: Protocol = read(&a.protocol.path)?;
    if serde_json::to_value(p).map_err(|e| e.to_string())?
        != serde_json::to_value(protocol()).map_err(|e| e.to_string())?
    {
        return Err("protocol differs from complete required scorecard".into());
    }
    Ok(a)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    lane: String,
    cycle: usize,
    arm: String,
    position: usize,
    elapsed_ns: u64,
    output_key: String,
    diagnostics: Value,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dataset {
    schema: String,
    authority_sha256: String,
    fixture_sha256: String,
    protocol_sha256: String,
    raw_observations: Vec<Observation>,
    complete_outputs: BTreeMap<String, Value>,
    warmup_completed: BTreeMap<String, usize>,
    no_auto_retries: bool,
    clock: String,
    clock_resolution_ns: u64,
    run_started_utc: String,
    run_finished_utc: String,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn write_new(p: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .map_err(|e| e.to_string())?;
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn quantile(sorted: &[f64], p: f64) -> f64 {
    sorted[((p * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1)]
}
fn stats(raw: &[f64]) -> Value {
    assert!(!raw.is_empty());
    let mut sorted = raw.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mean = raw.iter().sum::<f64>() / raw.len() as f64;
    let stddev = (raw.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / raw.len() as f64).sqrt();
    let p50 = quantile(&sorted, 0.5);
    let mut deviations = raw.iter().map(|v| (v - p50).abs()).collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    json!({"n":raw.len(),"min":sorted[0],"p50":p50,"p95":quantile(&sorted,0.95),"p99":quantile(&sorted,0.99),"max":sorted[sorted.len()-1],"mean":mean,"population_stddev":stddev,"median_absolute_deviation":quantile(&deviations,0.5),"coefficient_of_variation":if mean==0.0 {Value::Null}else{json!(stddev/mean)},"p99_tail_confidence":if raw.len()>=1000{"at least 1000 samples; empirical, not worst-case guarantee"}else{"sparse; p99 can equal max, no reliable tail claim"},"quantile":"nearest-rank, raw observations retained"})
}

fn schedule(n: usize) -> Vec<(&'static str, usize, usize)> {
    (0..n)
        .flat_map(|cycle| {
            let arms = if cycle % 2 == 0 {
                ["A", "B"]
            } else {
                ["B", "A"]
            };
            arms.into_iter()
                .enumerate()
                .map(move |(position, arm)| (arm, cycle, position))
        })
        .collect()
}

fn integer(v: &Value, key: &str) -> Result<u64> {
    v.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing nonnegative integer {key}"))
}
fn natural_settle(values: &Value, expected_end: u64) -> Result<()> {
    let observations = values
        .as_array()
        .ok_or("raw natural settling observations required")?;
    if observations.len() < 5 {
        return Err(
            "five or more raw settled observations required, never cherry-pick minimum".into(),
        );
    }
    let mut times = vec![];
    let mut footprint = vec![];
    for s in observations {
        times.push(integer(s, "monotonic_ns")?);
        footprint.push(integer(s, "physical_footprint_bytes")?);
        if integer(s, "pending_owners")? != 0 {
            return Err("unsettled owner at checkpoint".into());
        }
    }
    if times.windows(2).any(|w| w[1] <= w[0])
        || times.last().unwrap() - times[0] < 1_000_000_000
        || footprint.iter().max().unwrap() - footprint.iter().min().unwrap() > 1024 * 1024
        || footprint.last().copied() != Some(expected_end)
    {
        return Err("unstable or cherry-picked physical-footprint checkpoint".into());
    }
    Ok(())
}
fn validate_diagnostics(lane: &Lane, o: &Observation) -> Result<()> {
    let d = &o.diagnostics;
    if integer(d, "pending_owners")? != 0
        || d.get("unexpected_effects") != Some(&json!([]))
        || d.get("fixture_coverage_complete") != Some(&json!(true))
    {
        return Err(format!(
            "{} incomplete drain, unowned effect or fixture",
            lane.id
        ));
    }
    match lane.id.as_str() {
        "lifecycle.10000" => {
            if integer(d, "cycles")? != 10000
                || integer(d, "warmup_cycles")? != 100
                || d.get("allocator_relief_used") != Some(&json!(false))
            {
                return Err("invalid 10000-cycle natural lifecycle".into());
            }
            let start = integer(d, "settled_start_physical_footprint_bytes")?;
            natural_settle(
                d.get("settled_start_observations")
                    .ok_or("natural settled warm start evidence missing")?,
                start,
            )?;
            let cycles = d
                .get("cycle_elapsed_ns")
                .and_then(Value::as_array)
                .ok_or("all 10000 completed cycle timings required")?;
            if cycles.len() != 10000 || cycles.iter().any(|v| v.as_u64().is_none_or(|n| n == 0)) {
                return Err(
                    "10000 complete cycle timings required, not a cycle-count assertion".into(),
                );
            }
            let checkpoints = d
                .get("checkpoints")
                .and_then(Value::as_array)
                .ok_or("lifecycle checkpoint array missing")?;
            if checkpoints.len() != 3 {
                return Err("exactly 1000/5000/10000 checkpoints required".into());
            }
            for (point, cycle) in checkpoints.iter().zip([1000, 5000, 10000]) {
                if integer(point, "cycle")? != cycle
                    || integer(point, "pending_owners")? != 0
                    || integer(point, "physical_footprint_bytes")?.saturating_sub(start)
                        > 10 * 1024 * 1024
                    || point.get("natural_settle") != Some(&json!(true))
                {
                    return Err(format!(
                        "{} lifecycle growth/drain gate failed at {cycle}",
                        o.arm
                    ));
                }
                let settle = point
                    .get("settling_observations")
                    .ok_or("raw natural settling observations required")?;
                natural_settle(settle, integer(point, "physical_footprint_bytes")?)?;
            }
        }
        "idle.60seconds" => {
            if integer(d, "interval_ns")? < 60_000_000_000
                || d.get("resource_source") != Some(&json!("owned child kernel counters"))
            {
                return Err(
                    "idle must observe actual owned child kernel counters for >=60s".into(),
                );
            }
            for key in [
                "cpu_ns",
                "interrupt_wakeups",
                "idle_wakeups",
                "disk_read_bytes",
                "disk_write_bytes",
                "pageins",
                "faults",
                "physical_footprint_bytes",
                "rss_bytes",
            ] {
                integer(d, key)?;
            }
            let before = d
                .get("before")
                .ok_or("idle raw before kernel counters missing")?;
            let after = d
                .get("after")
                .ok_or("idle raw after kernel counters missing")?;
            for point in [before, after] {
                let numer = integer(point, "timebase_numer")?;
                let denom = integer(point, "timebase_denom")?;
                if numer == 0 || denom == 0 || numer > u32::MAX as u64 || denom > u32::MAX as u64 {
                    return Err("idle Mach timebase invalid".into());
                }
                let expected = ((integer(point, "cpu_raw_mach_ticks")? as u128) * (numer as u128)
                    / (denom as u128))
                    .min(u64::MAX as u128) as u64;
                if integer(point, "cpu_ns")? != expected {
                    return Err("idle CPU raw Mach ticks were not converted to ns".into());
                }
            }
            if integer(before, "timebase_numer")? != integer(after, "timebase_numer")?
                || integer(before, "timebase_denom")? != integer(after, "timebase_denom")?
            {
                return Err("idle Mach timebase changed".into());
            }
            if integer(before, "pid")? != integer(after, "pid")?
                || integer(before, "start_abstime")? != integer(after, "start_abstime")?
                || integer(after, "monotonic_ns")?.checked_sub(integer(before, "monotonic_ns")?)
                    != Some(integer(d, "interval_ns")?)
            {
                return Err("idle counter identity or interval changed".into());
            }
            for key in [
                "cpu_ns",
                "interrupt_wakeups",
                "idle_wakeups",
                "disk_read_bytes",
                "disk_write_bytes",
                "pageins",
                "faults",
            ] {
                if integer(after, key)?.checked_sub(integer(before, key)?) != Some(integer(d, key)?)
                {
                    return Err("idle must report measured interval deltas, not lifetime or synthetic counters".into());
                }
            }
            for key in ["physical_footprint_bytes", "rss_bytes"] {
                if integer(after, key)? != integer(d, key)? {
                    return Err("idle footprint/RSS must be actual ending counters".into());
                }
            }
        }
        "copies-allocations.input" => {
            if d.get("scope") != Some(&json!("observed dynamic calls only"))
                || d.get("separate_from_latency") != Some(&json!(true))
            {
                return Err(
                    "allocation/copy observations overclaim total or contaminate latency".into(),
                );
            }
            for key in [
                "malloc_calls",
                "realloc_calls",
                "free_calls",
                "allocated_bytes",
                "memcpy_calls",
                "memcpy_bytes",
                "memmove_calls",
                "memmove_bytes",
            ] {
                integer(d, key)?;
            }
        }
        "artifact.complete-app" => {
            integer(d, "regular_file_bytes")?;
            integer(d, "regular_file_count")?;
            if d.get("production_artifact") != Some(&json!(true)) {
                return Err("test artifact is not shipping app size".into());
            }
        }
        "artifact.binary" => {
            integer(d, "regular_file_bytes")?;
            if d.get("production_artifact") != Some(&json!(true)) {
                return Err("instrumented artifact is not shipping binary size".into());
            }
        }
        "artifact.linked-runtimes" => {
            if !d.get("linked_runtimes").is_some_and(Value::is_array) {
                return Err("linked-runtime inventory absent".into());
            }
        }
        "build.cold" | "build.cached" => {
            for key in [
                "compiler_receipt_sha256",
                "build_command_receipt_sha256",
                "artifact_sha256",
            ] {
                if d.get(key)
                    .and_then(Value::as_str)
                    .is_none_or(|s| s.len() != 64)
                {
                    return Err("build source/command/artifact binding missing".into());
                }
            }
            if d.get("offline") != Some(&json!(true)) {
                return Err("network-dependent build comparison".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn compare(a: &Authority, authority_hash: &str, d: &Dataset) -> Result<Value> {
    compare_protocol(a, authority_hash, d, &protocol())
}
fn diagnostic_statistics(rows: &[Observation]) -> Value {
    let mut metrics = BTreeMap::new();
    let Some(first) = rows.first().and_then(|o| o.diagnostics.as_object()) else {
        return json!({});
    };
    for (key, v) in first {
        if !v.is_u64()
            || rows
                .iter()
                .any(|o| o.diagnostics.get(key).and_then(Value::as_u64).is_none())
        {
            continue;
        }
        let values = |arm: &str| {
            rows.iter()
                .filter(|o| o.arm == arm)
                .map(|o| o.diagnostics[key].as_u64().unwrap() as f64)
                .collect::<Vec<_>>()
        };
        let a = values("A");
        let b = values("B");
        let delta = a.iter().zip(&b).map(|(a, b)| b - a).collect::<Vec<_>>();
        metrics.insert(key.clone(),json!({"baseline":stats(&a),"candidate":stats(&b),"paired_absolute_delta":stats(&delta)}));
    }
    json!(metrics)
}
fn compare_protocol(
    a: &Authority,
    authority_hash: &str,
    d: &Dataset,
    p: &Protocol,
) -> Result<Value> {
    if d.schema != SCHEMA
        || d.authority_sha256 != authority_hash
        || d.fixture_sha256 != a.fixture.sha256
        || d.protocol_sha256 != a.protocol.sha256
        || !d.no_auto_retries
        || d.clock != "monotonic"
        || d.clock_resolution_ns == 0
    {
        return Err("dataset binding, retry policy or monotonic clock invalid".into());
    }
    let required = p
        .lanes
        .iter()
        .map(|l| l.id.as_str())
        .collect::<BTreeSet<_>>();
    let units = if p.lanes.iter().any(|l| l.id == "idle.60seconds") {
        Some(read::<Value>(&a.counter_units.path)?)
    } else {
        None
    };
    if d.raw_observations
        .iter()
        .any(|o| !required.contains(o.lane.as_str()))
    {
        return Err("unknown lane; no silent partial/full substitution".into());
    }
    let mut result = vec![];
    let mut at = 0;
    for lane in &p.lanes {
        for arm in ["A", "B"] {
            if d.warmup_completed
                .get(&format!("{}:{arm}", lane.id))
                .copied()
                != Some(lane.warmup_per_arm)
            {
                return Err(format!("{} {arm} warmup evidence missing", lane.id));
            }
        }
        let count = lane.samples_per_arm * 2;
        let rows = d
            .raw_observations
            .get(at..at + count)
            .ok_or_else(|| format!("missing full lane {}", lane.id))?;
        at += count;
        let expected = schedule(lane.samples_per_arm);
        for (o, (arm, cycle, position)) in rows.iter().zip(expected) {
            if o.lane != lane.id
                || o.arm != arm
                || o.cycle != cycle
                || o.position != position
                || o.elapsed_ns == 0
            {
                return Err(format!(
                    "{} schedule/sample failure; observations cannot be reordered after collection",
                    lane.id
                ));
            }
            validate_diagnostics(lane, o)?;
            if lane.id == "idle.60seconds" {
                let units = units.as_ref().unwrap();
                let point = &o.diagnostics["before"];
                if point["timebase_numer"] != units["timebase_numer"]
                    || point["timebase_denom"] != units["timebase_denom"]
                {
                    return Err(
                        "idle timebase differs from independently retained calibration".into(),
                    );
                }
            }
            let output = d
                .complete_outputs
                .get(&o.output_key)
                .ok_or("raw observation references missing complete output")?;
            if sha(&serde_json::to_vec(output).map_err(|e| e.to_string())?) != o.output_key {
                return Err("complete ordered output binding mismatch".into());
            }
            if lane.semantic_output
                && (output.get("case_id").and_then(Value::as_str) != Some(&lane.id)
                    || ["state", "persisted", "completions", "errors", "privacy"]
                        .iter()
                        .any(|k| output.get(k).is_none()))
            {
                return Err(format!(
                    "{} missing complete semantic output envelope",
                    lane.id
                ));
            }
        }
        for pair in rows.chunks_exact(2) {
            if lane.semantic_output
                && d.complete_outputs.get(&pair[0].output_key)
                    != d.complete_outputs.get(&pair[1].output_key)
            {
                return Err(format!("{} cycle {} full ordered result mismatch (arrays are never sorted or reduced to checksum)",lane.id,pair[0].cycle));
            }
        }
        let arm_values = |name: &str| {
            rows.iter()
                .filter(|o| o.arm == name)
                .map(|o| o.elapsed_ns as f64)
                .collect::<Vec<_>>()
        };
        let av = arm_values("A");
        let bv = arm_values("B");
        let deltas = av.iter().zip(&bv).map(|(a, b)| b - a).collect::<Vec<_>>();
        let relative = av
            .iter()
            .zip(&bv)
            .map(|(a, b)| 100.0 * (b - a) / a)
            .collect::<Vec<_>>();
        let sa = stats(&av);
        let sb = stats(&bv);
        let sd = stats(&deltas);
        let delta50 = sd["p50"].as_f64().unwrap();
        let noise = sa["median_absolute_deviation"].as_f64().unwrap()
            + sb["median_absolute_deviation"].as_f64().unwrap();
        let model_budget = if lane.id.starts_with("search.500.") {
            Some(5_000_000)
        } else if lane.id.starts_with("search.5000.") {
            Some(15_000_000)
        } else if lane.id == "processes.cached-result-read1000" {
            Some(500_000)
        } else if lane.id == "clipboard.enqueue200x8000" {
            Some(5_000_000)
        } else {
            None
        };
        if model_budget.is_some_and(|budget| sb["p95"].as_f64().unwrap() > budget as f64) {
            return Err(format!("{} candidate model p95 budget failed", lane.id));
        }
        let order_stats = |arm: &str, pos: usize| {
            let values = rows
                .iter()
                .filter(|o| o.arm == arm && o.position == pos)
                .map(|o| o.elapsed_ns as f64)
                .collect::<Vec<_>>();
            if values.is_empty() {
                Value::Null
            } else {
                stats(&values)
            }
        };
        result.push(json!({"lane":lane.id,"unit":lane.unit,"baseline":sa,"candidate":sb,"paired_absolute_delta":sd,"paired_relative_delta_percent":stats(&relative),"direction":if delta50.abs()<=noise{"inconclusive relative to observed variation"}else if delta50<0.0{"candidate lower"}else{"candidate higher"},"order_effect":{"A_first":order_stats("A",0),"A_second":order_stats("A",1),"B_first":order_stats("B",0),"B_second":order_stats("B",1)},"numeric_diagnostics":diagnostic_statistics(rows),"diagnostic_first_samples":{"A":rows.iter().find(|o|o.arm=="A").map(|o|&o.diagnostics),"B":rows.iter().find(|o|o.arm=="B").map(|o|&o.diagnostics)},"ordered_parity":if lane.semantic_output{"all complete paired outputs equal"}else{"artifact/build diagnostics retained; differing artifacts are compared as metrics, not asserted equal"},"model_p95_budget_ns":model_budget}));
    }
    if at != d.raw_observations.len() {
        return Err("extra observations/retries after complete protocol".into());
    }
    Ok(
        json!({"schema":SCHEMA,"scope":"review-bound complete owned hidden-native comparison; physical display/battery excluded","authority_sha256":authority_hash,"baseline_version":a.baseline_version,"candidate_version":a.candidate_version,"raw_observation_count":at,"lanes":result,"excluded":a.exclusions,"interpretation":"Per-lane observed results only. No aggregate speed multiplier. Sparse p99 and observed variation remain explicit; this result is not a public release/install gate."}),
    )
}

fn model_diagnostic(out: &Path) -> Result<()> {
    use ruycast_native::session::{fixture, Session};
    if cfg!(debug_assertions) {
        return Err("model timing diagnostic requires the optimized release runner".into());
    }
    let source = preparation::inventory(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))?;
    let producer = std::env::current_exe().map_err(|e| e.to_string())?;
    let producer_sha = sha(&fs::read(&producer).map_err(|e| e.to_string())?);
    fs::create_dir(out).map_err(|e| e.to_string())?;
    write_new(&out.join("source-before.json"), &source)?;
    let terms = [
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
    let mut lanes = vec![];
    for size in [500, 1000, 5000, 10000] {
        let mut session = Session::new(fixture(size));
        for (term_index, term) in terms.into_iter().enumerate() {
            for _ in 0..100 {
                session.set_query("fixture reset, not a timed input");
                session.set_query(term);
                std::hint::black_box(session.rows().count());
            }
            session.set_query(term);
            let expected = serde_json::to_vec(&session.rows().cloned().collect::<Vec<_>>())
                .map_err(|e| e.to_string())?;
            let output_path = out.join(format!("rows-{size}-{term_index}.json"));
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output_path)
                .and_then(|mut f| f.write_all(&expected))
                .map_err(|e| e.to_string())?;
            let mut observations = vec![];
            for sample in 0..1000 {
                session.set_query("fixture reset, not a timed input");
                let start = Instant::now();
                session.set_query(term);
                let elapsed = start.elapsed().as_nanos();
                // Exact byte comparison against preserved complete ordered rows, outside timing.
                // Identical results are deduplicated, never merely counted or checksummed.
                let actual = serde_json::to_vec(&session.rows().cloned().collect::<Vec<_>>())
                    .map_err(|e| e.to_string())?;
                if actual != expected {
                    return Err(
                        "same deterministic model input produced different complete ordered output"
                            .into(),
                    );
                }
                observations.push(json!({"sample":sample,"elapsed_ns":elapsed,"selection":session.selection,"complete_ordered_output_exact_byte_equal":true}));
            }
            lanes.push(json!({"size":size,"query":term,"ordered_rows_file":output_path.file_name().and_then(|n|n.to_str()).ok_or("owned output filename is not UTF8")?,"ordered_rows_sha256":sha(&expected),"raw_observations":observations}));
        }
    }
    let path = out.join("model-raw.json");
    write_new(&path, &lanes)?;
    let after = preparation::inventory(&source.root)?;
    if after.files != source.files {
        return Err("model producer source changed during measurement; raw exploratory data retained, no receipt accepted".into());
    }
    write_new(&out.join("source-after.json"), &after)?;
    write_new(
        &out.join("receipt.json"),
        &json!({"schema":SCHEMA,"scope":"preparation only: current Rust Session model; no legacy comparator, production owner/GUI/resources/lifecycle or whole-product gain claim","optimized":!cfg!(debug_assertions),"producer_path":producer,"producer_sha256":producer_sha,"source_before_sha256":sha(&fs::read(out.join("source-before.json")).map_err(|e|e.to_string())?),"source_after_sha256":sha(&fs::read(out.join("source-after.json")).map_err(|e|e.to_string())?),"raw_sha256":sha(&fs::read(&path).map_err(|e|e.to_string())?),"observations":40000,"effects":"Session/fixture pure in-memory only; no provider/desktop/account queries","deduplication":"Every measured output is serialized after timing and exactly byte-compared to its preserved full ordered output file; repeated identical bytes stored once, not checksum-only parity."}),
    )
}

pub fn entry(args: Vec<String>) -> Result<()> {
    match args.as_slice() {
        [cmd,out] if cmd=="owned-counter-units"=>{let value=resources::counter_units()?;write_new(Path::new(out),&value)?;if value["ok"]==true{Ok(())}else{Err("own CPU counter units disagree with getrusage; calibration retained".into())}},
        [cmd,out] if cmd=="preentry-selftest"=>preparation::preentry_test(Path::new(out)),
        [cmd,root,out] if cmd=="summarize-model"=> {
            let dir=Path::new(root);let receipt:Value=read(&dir.join("receipt.json"))?;let raw=fs::read(dir.join("model-raw.json")).map_err(|e|e.to_string())?;
            if receipt["raw_sha256"]!=sha(&raw){return Err("model raw binding changed".into());}
            let lanes:Vec<Value>=serde_json::from_slice(&raw).map_err(|e|e.to_string())?;let mut rows=vec![];
            for lane in lanes {
                let relative=lane["ordered_rows_file"].as_str().ok_or("missing complete output file")?;
                if Path::new(relative).components().count()!=1{return Err("model output path must be a single owned filename".into());}
                let output=fs::read(dir.join(relative)).map_err(|e|e.to_string())?;
                if lane["ordered_rows_sha256"]!=sha(&output){return Err("model complete ordered output binding changed".into());}
                let samples=lane["raw_observations"].as_array().ok_or("model observations missing")?;
                if samples.len()!=1000 || samples.iter().any(|s|s["complete_ordered_output_exact_byte_equal"]!=true){return Err("model raw samples/exact output comparisons incomplete".into());}
                let times=samples.iter().map(|s|integer(s,"elapsed_ns").map(|n|n as f64)).collect::<Result<Vec<_>>>()?;
                rows.push(json!({"size":lane["size"],"query":lane["query"],"latency_ns":stats(&times),"ordered_output_sha256":sha(&output),"ordered_output_file":relative}));
            }
            write_new(Path::new(out),&json!({"scope":receipt["scope"],"optimized":receipt["optimized"],"raw_sha256":sha(&raw),"lanes":rows,"whole_product_comparison":false,"legacy_comparator":false}))
        },
        [cmd,out] if cmd=="prepare-complete-fixture" =>preparation::complete_fixture(Path::new(out)),
        [cmd,out] if cmd=="prepare-foundation-data" =>preparation::foundation_data(Path::new(out),false),
        [cmd,out] if cmd=="run-foundation-data" =>preparation::foundation_data(Path::new(out),true),
        [cmd,out] if cmd=="diagnose-foundation-data" =>preparation::diagnose_foundation_data(Path::new(out)),
        [cmd,out] if cmd=="prepare-apple-event-data" =>preparation::apple_event_data(Path::new(out),false),
        [cmd,out] if cmd=="run-apple-event-data" =>preparation::apple_event_data(Path::new(out),true),
        [cmd,out] if cmd=="diagnose-apple-event-data" =>preparation::diagnose_apple_event_data(Path::new(out)),
        [cmd,root,fixture,out] if cmd=="prepare-owned-host-images" =>preparation::owned_host_images(Path::new(root),Path::new(fixture),Path::new(out)),
        [cmd,..] if cmd=="prepare-owned-host-images" =>Err("usage: f1-runner prepare-owned-host-images ROOT FIXTURE_JSON OUT; an explicit source-bound fixture manifest is required; the old ROOT OUT form is refused".into()),
        [cmd,root,main,pinned,fixture,arm,out] if matches!(cmd.as_str(),"run-reference-assembly"|"run-reference-input") =>preparation::reference_images(Path::new(root),Path::new(main),Path::new(fixture),arm,Path::new(out),Some(pinned),cmd=="run-reference-input"),
        [cmd,..] if matches!(cmd.as_str(),"run-reference-assembly"|"run-reference-input") =>Err(format!("usage: f1-runner {cmd} ROOT MAIN PINNED_MAIN_SHA256 FIXTURE_JSON A|B OUT; fresh compile/grant, no saved cap reuse")),
        [cmd,root,main,fixture,arm,out] if cmd=="prepare-reference-images" =>preparation::reference_images(Path::new(root),Path::new(main),Path::new(fixture),arm,Path::new(out),None,false),
        [cmd,..] if cmd=="prepare-reference-images" =>Err("usage: f1-runner prepare-reference-images ROOT MAIN FIXTURE_JSON A|B OUT; same-build compile, no execution, cap expires on return".into()),
        [cmd,root,main,fixture,arm,out] if cmd=="prepare-reference-pins" =>preparation::reference_pins_preparation(Path::new(root),Path::new(main),Path::new(fixture),arm,Path::new(out)),
        [cmd,..] if cmd=="prepare-reference-pins" =>Err("usage: f1-runner prepare-reference-pins ROOT MAIN FIXTURE_JSON A|B OUT; source-only, expires on return".into()),
        [cmd,root,out] if cmd=="prepare-native-owned-host" =>preparation::native_owned_host(Path::new(root),Path::new(out)),
        [cmd,root,manifest,pinned,main_sha,out] if cmd=="postprocess-native-owned-data" =>preparation::native_retained_data(Path::new(root),Path::new(manifest),pinned,main_sha,Path::new(out)),
        [cmd,root,plan,pinned] if cmd=="collect-reference-hotkeys-ast" =>preparation::reference_hotkeys_ast(Path::new(root),Path::new(plan),pinned),
        [cmd,root,plan,pinned] if cmd=="collect-reference-hyperkey-ast" =>preparation::reference_hyperkey_ast(Path::new(root),Path::new(plan),pinned),
        [cmd,root,plan,pinned] if cmd=="collect-reference-snippetexpander-ast" =>preparation::reference_snippetexpander_ast(Path::new(root),Path::new(plan),pinned),
        [cmd,root,out] if cmd=="inventory" =>write_new(Path::new(out),&preparation::inventory(Path::new(root))?),
        [cmd,root,bridge,out] if cmd=="build-reference-interface" =>preparation::bridge(Path::new(root),Path::new(bridge),Path::new(out),false),
        [cmd,root,bridge,build_root,inputs,pinned,out] if cmd=="build-reference-full-host-interface" =>preparation::full_reference_bridge(Path::new(root),Path::new(bridge),Path::new(build_root),Path::new(inputs),pinned,Path::new(out)),
        [cmd,..] if cmd=="build-reference-full-host-interface" =>Err("usage: f1-runner build-reference-full-host-interface SNAPSHOT_ROOT BRIDGE EXISTING_BUILD_ROOT INPUTS_JSON PINNED_SHA256 OUT; no inferred sibling/build fallback".into()),
        [cmd,build_root,inputs,pinned,out] if cmd=="resume-reference-debug" =>preparation::resume_reference_debug(Path::new(build_root),Path::new(inputs),pinned,Path::new(out)),
        [cmd,..] if cmd=="resume-reference-debug" =>Err("usage: f1-runner resume-reference-debug EXISTING_BUILD_ROOT INPUTS_JSON PINNED_SHA256 EXISTING_OUT; capture only, no compilation or app launch".into()),
        [cmd,receipt,pinned,out] if cmd=="collect"=>collection::collect(Path::new(receipt),pinned,Path::new(out)),
        [cmd,fixture,pinned,out] if cmd=="owned-transport-selftest"=>collection::transport_selftest(Path::new(fixture),pinned,Path::new(out)),
        [cmd] if cmd == "owned-resource-child" => {resources::child();Ok(())},
        [cmd,out] if cmd == "owned-resource-selftest" => write_new(Path::new(out),&resources::self_test()?),
        [cmd,out] if cmd=="protocol"=>write_new(Path::new(out),&protocol()),
        [cmd,out] if cmd=="model-diagnostic"=>model_diagnostic(Path::new(out)),
        [cmd,receipt,pinned] if cmd=="validate-authority"=> {check_authority(Path::new(receipt),pinned)?;println!("{}",json!({"ok":true,"schema":SCHEMA,"operation":"validate-authority","authority_sha256":pinned,"status":"authority-valid; no product executed"}));Ok(())},
        [cmd,receipt,pinned,dataset,out] if cmd=="compare"=> {
            let a=check_authority(Path::new(receipt),pinned)?;let raw=fs::read(dataset).map_err(|e|e.to_string())?;let d=serde_json::from_slice(&raw).map_err(|e|e.to_string())?;let mut report=compare(&a,pinned,&d)?;
            report["ok"]=json!(true);report["source_dataset_sha256"]=json!(sha(&raw));
            check_authority(Path::new(receipt),pinned)?;write_new(Path::new(out),&report)?;
            println!("{}",json!({"ok":true,"schema":SCHEMA,"operation":"compare","authority_sha256":pinned,"dataset_sha256":sha(&raw),"report_path":fs::canonicalize(out).map_err(|e|e.to_string())?,"report_sha256":sha(&fs::read(out).map_err(|e|e.to_string())?)}));Ok(())
        },
        _=>Err("usage: f1-runner protocol NEW.json | model-diagnostic NEW_DIRECTORY | prepare-owned-host-images ROOT FIXTURE_JSON OUT | prepare-reference-pins ROOT MAIN FIXTURE_JSON A|B OUT | validate-authority RECEIPT PINNED_SHA256 | compare RECEIPT PINNED_SHA256 RAW_DATASET NEW_REPORT. No normal-app/raw legacy flag launch is supported.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_host_preparation_refuses_the_legacy_missing_fixture_argv_before_io() {
        let args=["prepare-owned-host-images","/not-opened/root","/not-created/out"];
        let error=entry(args.into_iter().map(str::to_owned).collect()).unwrap_err();
        assert!(error.contains("ROOT FIXTURE_JSON OUT"));
        assert!(error.contains("old ROOT OUT form is refused"));
        let args=["prepare-reference-pins","/not-opened/root"];
        assert!(entry(args.into_iter().map(str::to_owned).collect()).unwrap_err().contains("A|B OUT"));
    }
    #[test]
    fn full_reference_refuses_implicit_build_inputs_before_io() {
        let args=["build-reference-full-host-interface","/not-opened/reference",
            "/not-opened/SwiftF1Bridge.swift","/not-created/out"];
        let error=entry(args.into_iter().map(str::to_owned).collect()).unwrap_err();
        assert!(error.contains("EXISTING_BUILD_ROOT INPUTS_JSON PINNED_SHA256"));
        assert!(error.contains("no inferred sibling/build fallback"));
    }
    #[test]
    fn idle_uses_same_owned_identity_and_interval_deltas_not_lifetime_counters() {
        let lane = protocol()
            .lanes
            .into_iter()
            .find(|l| l.id == "idle.60seconds")
            .unwrap();
        let keys = [
            "cpu_ns",
            "interrupt_wakeups",
            "idle_wakeups",
            "disk_read_bytes",
            "disk_write_bytes",
            "pageins",
            "faults",
        ];
        let mut before = json!({"pid":770001,"start_abstime":100,"monotonic_ns":1,"physical_footprint_bytes":4000000,"rss_bytes":8000000,"cpu_raw_mach_ticks":100,"timebase_numer":1,"timebase_denom":1});
        let mut after = before.clone();
        after["cpu_raw_mach_ticks"] = json!(105);
        after["monotonic_ns"] = json!(60_000_000_001_u64);
        let mut d = json!({"pending_owners":0,"unexpected_effects":[],"fixture_coverage_complete":true,"resource_source":"owned child kernel counters","interval_ns":60_000_000_000_u64,"physical_footprint_bytes":4000000,"rss_bytes":8000000});
        for key in keys {
            before[key] = json!(100);
            after[key] = json!(105);
            d[key] = json!(5);
        }
        d["before"] = before;
        d["after"] = after;
        let make = |diagnostics| Observation {
            lane: lane.id.clone(),
            cycle: 0,
            arm: "A".into(),
            position: 0,
            elapsed_ns: 1,
            output_key: String::new(),
            diagnostics,
        };
        assert!(validate_diagnostics(&lane, &make(d.clone())).is_ok());
        for mutation in ["lifetime", "identity", "interval", "gauge"] {
            let mut bad = d.clone();
            match mutation {
                "lifetime" => bad["cpu_ns"] = json!(105),
                "identity" => bad["after"]["start_abstime"] = json!(101),
                "interval" => bad["interval_ns"] = json!(59_000_000_000_u64),
                _ => bad["rss_bytes"] = json!(1),
            }
            assert!(
                validate_diagnostics(&lane, &make(bad)).is_err(),
                "{mutation}"
            );
        }
    }
    #[test]
    fn incomplete_final_freeze_cannot_launch_or_create_collection() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/f1-preparation-evidence");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "admission-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let (a, _, _) = comparison_fixture("search.500.m");
        let mut a = a;
        a.final_functional_freeze = false;
        let receipt = root.join("authority.json");
        write_new(&receipt, &a).unwrap();
        let pinned = sha(&fs::read(&receipt).unwrap());
        let output = root.join("must-not-be-created");
        assert!(collection::collect(&receipt, &pinned, &output)
            .unwrap_err()
            .contains("incomplete or incomparable"));
        assert!(!output.exists());
        fs::remove_file(receipt).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn settling_accepts_stable_last_sample_but_rejects_owner_time_and_cherry_pick() {
        let stable=json!((0..5).map(|i|json!({"monotonic_ns":i*250_000_000_u64,"physical_footprint_bytes":4_000_000_u64,"pending_owners":0})).collect::<Vec<_>>());
        assert!(natural_settle(&stable, 4_000_000).is_ok());
        for mutation in ["owner", "time", "last", "variance"] {
            let mut changed = stable.clone();
            match mutation {
                "owner" => changed[2]["pending_owners"] = json!(1),
                "time" => changed[3]["monotonic_ns"] = json!(0),
                "last" => changed[4]["physical_footprint_bytes"] = json!(3_999_999),
                _ => changed[0]["physical_footprint_bytes"] = json!(8_000_000),
            }
            assert!(natural_settle(&changed, 4_000_000).is_err(), "{mutation}");
        }
    }
    #[test]
    fn lifecycle_requires_every_checkpoint_and_every_completed_cycle() {
        fn settle(bytes: u64) -> Value {
            json!((0..5).map(|i|json!({"monotonic_ns":i*250_000_000_u64,"physical_footprint_bytes":bytes,"pending_owners":0})).collect::<Vec<_>>())
        }
        let lane = protocol()
            .lanes
            .into_iter()
            .find(|l| l.id == "lifecycle.10000")
            .unwrap();
        let start = 4_000_000_u64;
        let checkpoints=[1000,5000,10000].into_iter().map(|c|json!({"cycle":c,"pending_owners":0,"physical_footprint_bytes":start+1024,"natural_settle":true,"settling_observations":settle(start+1024)})).collect::<Vec<_>>();
        let valid = json!({"pending_owners":0,"unexpected_effects":[],"fixture_coverage_complete":true,"cycles":10000,"warmup_cycles":100,"allocator_relief_used":false,"settled_start_physical_footprint_bytes":start,"settled_start_observations":settle(start),"cycle_elapsed_ns":vec![100_u64;10000],"checkpoints":checkpoints});
        let make = |diagnostics| Observation {
            lane: lane.id.clone(),
            cycle: 0,
            arm: "A".into(),
            position: 0,
            elapsed_ns: 1,
            output_key: String::new(),
            diagnostics,
        };
        assert!(validate_diagnostics(&lane, &make(valid.clone())).is_ok());
        for checkpoint in 0..3 {
            let mut bad = valid.clone();
            let bytes = start + 10 * 1024 * 1024 + 1;
            bad["checkpoints"][checkpoint]["physical_footprint_bytes"] = json!(bytes);
            bad["checkpoints"][checkpoint]["settling_observations"] = settle(bytes);
            assert!(validate_diagnostics(&lane, &make(bad))
                .unwrap_err()
                .contains("growth/drain"));
        }
        let mut missing = valid;
        missing["cycle_elapsed_ns"].as_array_mut().unwrap().pop();
        assert!(validate_diagnostics(&lane, &make(missing)).is_err());
    }
    fn comparison_fixture(id: &str) -> (Authority, Dataset, Protocol) {
        let binding = Binding {
            path: PathBuf::from("/owned/not-read-by-pure-comparison"),
            sha256: "fixture".into(),
        };
        let a = Authority {
            schema: SCHEMA.into(),
            decision: "accepted".into(),
            final_functional_freeze: true,
            functional_blocks: BTreeSet::new(),
            protocol: Binding {
                sha256: "protocol".into(),
                ..binding.clone()
            },
            fixture: binding.clone(),
            baseline_source: binding.clone(),
            candidate_source: binding.clone(),
            baseline_artifact: binding.clone(),
            candidate_artifact: binding.clone(),
            baseline_pre_entry_audit: binding.clone(),
            candidate_pre_entry_audit: binding.clone(),
            collection_adapters: binding.clone(),
            runner_artifact: binding.clone(),
            counter_units: binding,
            boundaries: BTreeSet::new(),
            baseline_version: "0.8.26".into(),
            baseline_commit: "6e3ae12e5e1734f36db9361b80eef0ade8db8a44".into(),
            candidate_version: "0.9.0".into(),
            baseline_optimized: true,
            candidate_optimized: true,
            same_instrumentation: true,
            full_workloads: true,
            exclusions: vec![],
        };
        let lane = Lane {
            id: id.into(),
            dimension: "search".into(),
            samples_per_arm: 4,
            warmup_per_arm: 0,
            unit: "ns".into(),
            notes: "owned pure comparator test".into(),
            semantic_output: true,
        };
        let p = Protocol {
            schema: SCHEMA.into(),
            lanes: vec![lane],
            ordered_outputs: true,
            physical_display_battery_excluded: true,
            schedule: "test".into(),
            seed: 1,
        };
        let output = json!({"case_id":id,"state":{"rows":[{"id":"a","enabled":true},{"id":"b","enabled":false}],"selection":0},"persisted":{},"completions":[],"errors":[],"privacy":[]});
        let key = sha(&serde_json::to_vec(&output).unwrap());
        let d=Dataset{schema:SCHEMA.into(),authority_sha256:"approved".into(),fixture_sha256:"fixture".into(),protocol_sha256:"protocol".into(),raw_observations:schedule(4).into_iter().map(|(arm,cycle,position)|Observation{lane:id.into(),arm:arm.into(),cycle,position,elapsed_ns:if arm=="A"{2_000_000}else{1_000_000},output_key:key.clone(),diagnostics:json!({"pending_owners":0,"unexpected_effects":[],"fixture_coverage_complete":true})}).collect(),complete_outputs:BTreeMap::from([(key,output)]),warmup_completed:BTreeMap::from([(format!("{id}:A"),0),(format!("{id}:B"),0)]),no_auto_retries:true,clock:"monotonic".into(),clock_resolution_ns:1,run_started_utc:"owned-start".into(),run_finished_utc:"owned-end".into()};
        (a, d, p)
    }
    #[test]
    fn actual_comparison_preserves_absolute_relative_and_order_statistics() {
        let (a, d, p) = comparison_fixture("search.500.m");
        let result = compare_protocol(&a, "approved", &d, &p).unwrap();
        let row = &result["lanes"][0];
        assert_eq!(row["paired_absolute_delta"]["p50"], -1_000_000.);
        assert_eq!(row["paired_relative_delta_percent"]["p50"], -50.);
        assert_eq!(row["baseline"]["p99"], 2_000_000.);
        assert_eq!(row["order_effect"]["A_first"]["n"], 2);
    }
    #[test]
    fn full_order_and_non_id_fields_are_compared() {
        for mutation in ["order", "enabled", "error"] {
            let (a, mut d, p) = comparison_fixture("search.500.m");
            let mut changed = d.complete_outputs.values().next().unwrap().clone();
            match mutation {
                "order" => changed["state"]["rows"].as_array_mut().unwrap().reverse(),
                "enabled" => changed["state"]["rows"][0]["enabled"] = json!(false),
                _ => changed["errors"] = json!(["owned error"]),
            }
            let key = sha(&serde_json::to_vec(&changed).unwrap());
            d.complete_outputs.insert(key.clone(), changed);
            d.raw_observations[1].output_key = key;
            assert!(compare_protocol(&a, "approved", &d, &p)
                .unwrap_err()
                .contains("ordered result mismatch"));
        }
    }
    #[test]
    fn missing_lane_bad_binding_and_retries_fail_closed() {
        for mutation in [
            "missing", "binding", "retry", "clock", "schedule", "owner", "effect", "fixture",
            "output",
        ] {
            let (a, mut d, p) = comparison_fixture("search.500.m");
            match mutation {
                "missing" => {
                    d.raw_observations.pop();
                }
                "binding" => d.authority_sha256 = "unreviewed".into(),
                "retry" => d.no_auto_retries = false,
                "clock" => d.clock = "wall".into(),
                "schedule" => d.raw_observations.swap(0, 1),
                "owner" => d.raw_observations[0].diagnostics["pending_owners"] = json!(1),
                "effect" => {
                    d.raw_observations[0].diagnostics["unexpected_effects"] = json!(["open"])
                }
                "fixture" => {
                    d.raw_observations[0].diagnostics["fixture_coverage_complete"] = json!(false)
                }
                _ => d.complete_outputs.clear(),
            }
            assert!(
                compare_protocol(&a, "approved", &d, &p).is_err(),
                "{mutation}"
            );
        }
    }
    #[test]
    fn model_budget_and_no_empty_state_pass() {
        let (a, mut d, p) = comparison_fixture("search.500.m");
        for o in &mut d.raw_observations {
            if o.arm == "B" {
                o.elapsed_ns = 5_000_001;
            }
        }
        assert!(compare_protocol(&a, "approved", &d, &p)
            .unwrap_err()
            .contains("p95 budget"));
        let (a, mut d, p) = comparison_fixture("search.500.m");
        let key = sha(b"{}");
        d.complete_outputs = BTreeMap::from([(key.clone(), json!({}))]);
        for o in &mut d.raw_observations {
            o.output_key = key.clone();
        }
        assert!(compare_protocol(&a, "approved", &d, &p)
            .unwrap_err()
            .contains("semantic output"));
        for (id, budget) in [
            ("processes.cached-result-read1000", 500_000),
            ("clipboard.enqueue200x8000", 5_000_000),
        ] {
            let (a, mut d, p) = comparison_fixture(id);
            for o in &mut d.raw_observations {
                if o.arm == "B" {
                    o.elapsed_ns = budget + 1;
                }
            }
            assert!(compare_protocol(&a, "approved", &d, &p)
                .unwrap_err()
                .contains("p95 budget"));
        }
    }
    #[test]
    fn empirical_tail_uses_all_thousand_samples() {
        let values = (1..=1000).map(f64::from).collect::<Vec<_>>();
        let s = stats(&values);
        assert_eq!(s["p50"], 500.);
        assert_eq!(s["p95"], 950.);
        assert_eq!(s["p99"], 990.);
        assert_eq!(s["max"], 1000.);
    }
    #[test]
    fn balanced_alternation_preserves_paired_position() {
        assert_eq!(
            schedule(4),
            vec![
                ("A", 0, 0),
                ("B", 0, 1),
                ("B", 1, 0),
                ("A", 1, 1),
                ("A", 2, 0),
                ("B", 2, 1),
                ("B", 3, 0),
                ("A", 3, 1)
            ]
        );
    }
    #[test]
    fn nearest_rank_and_variation_not_interpolated() {
        let s = stats(&[4., 1., 3., 2.]);
        assert_eq!(s["p50"], 2.);
        assert_eq!(s["p95"], 4.);
        assert_eq!(s["p99"], 4.);
        assert!(s["p99_tail_confidence"]
            .as_str()
            .unwrap()
            .starts_with("sparse"));
        assert_eq!(s["median_absolute_deviation"], 1.);
    }
    #[test]
    fn arrays_never_become_checksum_parity() {
        assert_ne!(
            json!([{"id":"a"},{"id":"b"}]),
            json!([{"id":"b"},{"id":"a"}])
        );
        assert_ne!(
            json!({"rows":[{"id":"a","title":"one"}]}),
            json!({"rows":[{"id":"a","title":"two"}]})
        );
    }
    #[test]
    fn complete_protocol_has_native_and_memory_and_build_gates() {
        let p = protocol();
        assert_eq!(p.lanes.len(), 157);
        assert!(p.lanes.iter().any(|l| l.id == "input.10000.missing Ω"));
        assert!(p.lanes.iter().any(|l| l.id == "lifecycle.10000"));
        assert!(p.lanes.iter().any(|l| l.id == "build.cold"));
        assert!(p.lanes.iter().all(|l| l.samples_per_arm > 0));
    }
    #[test]
    fn missing_boundary_cannot_authorize_query() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/f1-preparation-evidence");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "scope-admission-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        for (index, missing) in BOUNDARIES.iter().chain(FUNCTIONAL_BLOCKS).enumerate() {
            let (mut a, _, _) = comparison_fixture("search.500.m");
            a.boundaries = BOUNDARIES.iter().map(|s| (*s).into()).collect();
            a.functional_blocks = FUNCTIONAL_BLOCKS.iter().map(|s| (*s).into()).collect();
            a.boundaries.remove(*missing);
            a.functional_blocks.remove(*missing);
            let receipt = root.join(format!("authority-{index}.json"));
            write_new(&receipt, &a).unwrap();
            let pinned = sha(&fs::read(&receipt).unwrap());
            let output = root.join(format!("must-not-start-{index}"));
            let error = collection::collect(&receipt, &pinned, &output).unwrap_err();
            assert!(
                error.contains("missing or unknown functional/SDK pre-entry audit scope"),
                "{missing}: {error}"
            );
            assert!(!output.exists());
            fs::remove_file(receipt).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn lifecycle_requires_natural_stable_settle_not_relief() {
        let lane = protocol()
            .lanes
            .into_iter()
            .find(|l| l.id == "lifecycle.10000")
            .unwrap();
        let o = Observation {
            lane: lane.id.clone(),
            cycle: 0,
            arm: "A".into(),
            position: 0,
            elapsed_ns: 1,
            output_key: String::new(),
            diagnostics: json!({"pending_owners":0,"unexpected_effects":[],"fixture_coverage_complete":true,"cycles":10000,"warmup_cycles":100,"allocator_relief_used":true}),
        };
        assert!(validate_diagnostics(&lane, &o)
            .unwrap_err()
            .contains("natural"));
    }
    #[test]
    fn endpoint_typed_lanes_match_existing_complete_protocol(){
        use ruycast_native::native_f1::Lane;
        let protocol=protocol();assert_eq!(Lane::ids(),protocol.lanes.iter().map(|lane|lane.id.clone()).collect::<Vec<_>>());
        for lane in protocol.lanes{let counts=Lane::parse(&lane.id).unwrap().counts();assert_eq!(counts,(lane.samples_per_arm as u64,lane.warmup_per_arm as u64),"{}",lane.id);}
    }

}
