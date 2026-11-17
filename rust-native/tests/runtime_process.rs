//! Fifteen original headless runtime cases on exclusive profiles and Unix sockets.
#![cfg(feature = "test-tools")]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host, OwnedDir};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::{symlink, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_file(path: &Path) -> String {
    hash(&fs::read(path).unwrap())
}
fn verify_artifact_hashes(paths: &[PathBuf], expected: &[String]) -> Result<(), String> {
    for (path, digest) in paths.iter().zip(expected) {
        if hash_file(path) != *digest {
            return Err(format!("Binary changed during measurement: {}", path.display()));
        }
    }
    Ok(())
}
fn latency_stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples": sorted.len(),
        "medianMS": sorted[sorted.len() / 2],
        "p95MS": sorted[((sorted.len() - 1) as f64 * 0.95) as usize],
        "p99MS": if sorted.len() >= 1000 { json!(sorted[((sorted.len() - 1) as f64 * 0.99) as usize]) } else { Value::Null },
        "maxMS": sorted[sorted.len() - 1]
    })
}
fn prepare_benchmark_fixture(h: &runtime_host::Host) {
    let mut config: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    config["commands"] = (0..500)
        .map(|i| json!({
            "id": format!("fixture.{i}"),
            "title": format!("Open Project {i}"),
            "kind": "url",
            "target": format!("https://example.com/{i}"),
            "aliases": [format!("p{i}")],
            "arguments": []
        }))
        .collect::<Vec<_>>()
        .into();
    fs::write(&h.config, serde_json::to_vec(&config).unwrap()).unwrap();
    let created_at = current_rfc3339();
    let clipboard = (0..200)
        .map(|i| json!({
            "id": format!("fixture-{i}"),
            "text": format!("Fixture {i}: {}", "native clipboard Ω ".repeat(400)),
            "source": "fixture",
            "date": created_at
        }))
        .collect::<Vec<_>>();
    fs::write(
        h.profile.join("clipboard.json"),
        serde_json::to_vec(&clipboard).unwrap(),
    )
    .unwrap();
}
fn current_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as libc::time_t;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    assert!(!unsafe { libc::gmtime_r(&now, &mut parts) }.is_null());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        parts.tm_year + 1900,
        parts.tm_mon + 1,
        parts.tm_mday,
        parts.tm_hour,
        parts.tm_min,
        parts.tm_sec
    )
}
fn benchmark_probe(probe: &Path, pid: u32, timeout: Duration) -> Result<Value, String> {
    let output = runtime_host::run_result(
        Command::new(probe).arg(pid.to_string()),
        timeout,
    )
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Probe exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    if !value.is_object() {
        return Err("Probe response is not a JSON object".into());
    }
    for key in [
        "cpuNS",
        "packageIdleWakeups",
        "interruptWakeups",
        "footprintBytes",
        "diskReadBytes",
        "diskWriteBytes",
        "logicalWriteBytes",
    ] {
        if value[key].as_u64().is_none() {
            return Err(format!("Probe response has no unsigned {key} counter"));
        }
    }
    Ok(value)
}
fn benchmark_observe<F>(
    h: &runtime_host::Host,
    probe: &Path,
    duration: Duration,
    active: bool,
    mut request: F,
) -> Value
where
    F: FnMut(Value) -> Result<Value, String>,
{
    let pid = h.process.as_ref().unwrap().id();
    let mut requests = 0_u64;
    let fail = |error: String, requests: u64, samples: Vec<Value>| {
        json!({"requestedDurationS":duration.as_secs_f64(),"samples":samples,"requests":requests,"complete":false,"error":error})
    };
    let mut first = match benchmark_probe(probe, pid, Duration::from_secs(5)) {
        Ok(row) => row,
        Err(error) => return fail(error, requests, Vec::new()),
    };
    first["elapsedS"] = json!(0.0);
    let mut samples = vec![first];
    let started = Instant::now();
    let mut last_sample = started;
    let mut last_monitor = started;
    while started.elapsed() < duration {
        if active {
            let index = requests % 100;
            for call in [
                json!({"op":"search","query":format!("Project {index}")}),
                json!({"op":"clipboard.list","query":format!("Fixture {index}")}),
            ] {
                match request(call) {
                    Ok(value) if value["ok"] == true => requests += 1,
                    Ok(value) => return fail(format!("RPC refused: {value}"), requests, samples),
                    Err(error) => return fail(error, requests, samples),
                }
            }
            if last_monitor.elapsed() >= Duration::from_secs(2) {
                match request(json!({"op":"monitor"})) {
                    Ok(value) if value["ok"] == true => requests += 1,
                    Ok(value) => return fail(format!("Monitor refused: {value}"), requests, samples),
                    Err(error) => return fail(error, requests, samples),
                }
                last_monitor = Instant::now();
            }
        }
        thread::sleep(
            Duration::from_millis(100)
                .min(duration.saturating_sub(started.elapsed()).max(Duration::from_millis(1))),
        );
        if last_sample.elapsed() >= Duration::from_secs(5) {
            let mut row = match benchmark_probe(probe, pid, Duration::from_secs(5)) {
                Ok(row) => row,
                Err(error) => return fail(error, requests, samples),
            };
            row["elapsedS"] = json!(started.elapsed().as_secs_f64());
            samples.push(row);
            last_sample = Instant::now();
        }
    }
    let mut final_row = match benchmark_probe(probe, pid, Duration::from_secs(5)) {
        Ok(row) => row,
        Err(error) => return fail(error, requests, samples),
    };
    final_row["elapsedS"] = json!(started.elapsed().as_secs_f64());
    samples.push(final_row);
    let first = &samples[0];
    let last = samples.last().unwrap();
    let seconds = last["elapsedS"].as_f64().unwrap() - first["elapsedS"].as_f64().unwrap();
    let delta = |key: &str| {
        (last[key].as_u64().unwrap() as i128 - first[key].as_u64().unwrap() as i128) as f64
    };
    json!({
        "requestedDurationS":duration.as_secs_f64(), "durationS":seconds, "samples":samples,
        "requests":requests, "complete":true,
        "cpuPercentOneCore":100.0*delta("cpuNS")/1e9/seconds,
        "packageIdleWakeupsPerSecond":delta("packageIdleWakeups")/seconds,
        "interruptWakeupsPerSecond":delta("interruptWakeups")/seconds,
        "footprintStartMiB":first["footprintBytes"].as_f64().unwrap()/1048576.0,
        "footprintEndMiB":last["footprintBytes"].as_f64().unwrap()/1048576.0,
        "footprintMaxMiB":samples.iter().map(|row|row["footprintBytes"].as_f64().unwrap()).fold(0.0_f64,f64::max)/1048576.0,
        "diskReadBytes":delta("diskReadBytes"), "diskWriteBytes":delta("diskWriteBytes"),
        "logicalWriteBytes":delta("logicalWriteBytes")
    })
}
fn settings() -> Config {
    let settings = Config::current();
    runtime_receipt(&settings);
    settings
}
fn runtime_receipt(settings: &Config) {
    let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") else {
        return;
    };
    fs::create_dir_all(&directory).unwrap();
    let root = runtime_host::root();
    let sources = [
        "config/default.json",
        "rust-native/tests/runtime_process.rs",
        "rust-native/tests/support/runtime_host.rs",
        "rust-native/tests/support/status_rpc.rs",
        "rust-native/src/benchmark_host.rs",
        "rust-native/src/benchmark_host/rpc.rs",
        "rust-native/src/bin/ruycast-build/benchmark_host.rs",
        "rust-native/tests/support/child_output.rs",
        "rust-native/tests/fixtures/fcntl_sync_shim.rs",
        "rust-native/tests/fixtures/fcntl_abi.rs",
        "rust-native/src/main.rs",
        "rust-native/src/native_services.rs",
        "rust-native/src/runtime.rs",
        "rust-native/src/control.rs",
        "rust-native/src/configuration_jobs.rs",
        "rust-native/src/profile.rs",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src/lib.rs",
        "rust/src/bin/ruycast.rs",
        "rust-toolchain.toml",
    ]
    .into_iter()
    .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
    .collect::<BTreeMap<_, _>>();
    let sdk = settings
        .host
        .parent()
        .unwrap()
        .join("examples/libfcntl_sync_shim.dylib");
    let mut artifacts = BTreeMap::new();
    // Immutable content-addressed copies avoid retaining another large host/CLI
    // binary for every separately bounded case. Publish only complete bytes.
    for (name, path) in [
        ("host", &settings.host),
        ("cli", &settings.cli),
        ("writer-sdk", &sdk),
    ] {
        if name == "writer-sdk" && !path.is_file() {
            continue;
        }
        let (path, digest) = runtime_host::bind_artifact(path, name);
        artifacts.insert(path, digest);
    }
    fs::write(PathBuf::from(directory).join("runtime-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"methods":15,"hostArguments":["--headless-test"],"scope":"Exclusive synthetic profiles, empty own Applications root, actual Rust CLI and optional same-profile Rust writer provider; no native-hidden, desktop input or applied app actions"})).unwrap()).unwrap();
}
fn host(start: bool) -> Host {
    Host::new(settings(), start)
}
fn wait_marker(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "Actual directory sync not reached"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn benchmark_fixture_owns_500_commands_200_clipboard_rows_and_cold_readiness() {
    let mut h = host(false);
    prepare_benchmark_fixture(&h);

    let started = Instant::now();
    h.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(8));
    let cold_ready_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = h.rpc_timeout(
        &json!({"op":"status"}),
        Duration::from_secs(2),
    )["result"]
        .clone();
    assert_eq!(status["headless"], true);
    assert_eq!(Path::new(status["configPath"].as_str().unwrap()), h.config);
    assert_eq!(status["commands"], 500);
    assert_eq!(status["clipboardCount"], 200);
    assert!(cold_ready_ms.is_finite() && cold_ready_ms > 0.0);
    let exit = h.stop(true).unwrap();
    assert!(exit.status.success(), "{}", String::from_utf8_lossy(&exit.stderr));
}

#[test]
fn benchmark_cli_uses_real_warmups_latency_statistics_and_bound_artifacts() {
    let mut h = host(false);
    prepare_benchmark_fixture(&h);
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_perf-probe"));
    let artifacts = [h.settings.host.clone(), h.settings.cli.clone(), probe];
    let expected = artifacts.iter().map(|path| hash_file(path)).collect::<Vec<_>>();
    h.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(8));

    for (name, args) in [("status", vec!["status"]), ("search", vec!["search", "Project 42"])] {
        verify_artifact_hashes(&artifacts, &expected).unwrap();
        let mut measurements = Vec::new();
        for index in 0..20 {
            let began = Instant::now();
            let output = h.cli_output(&args);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["ok"], true, "{name} CLI response: {value}");
            if name == "search" {
                assert!(value["result"].as_array().unwrap().iter().any(|row| row["id"] == "fixture.42"));
            }
            if index >= 10 {
                measurements.push(began.elapsed().as_secs_f64() * 1000.0);
            }
        }
        verify_artifact_hashes(&artifacts, &expected).unwrap();
        let stats = latency_stats(&measurements);
        assert_eq!(stats["samples"], 10);
        assert!(stats["medianMS"].as_f64().unwrap().is_finite());
        assert!(stats["p95MS"].as_f64().unwrap().is_finite());
        assert_eq!(stats["p99MS"], Value::Null);
        assert!(stats["maxMS"].as_f64().unwrap().is_finite());
    }
    let exit = h.stop(true).unwrap();
    assert!(exit.status.success(), "{}", String::from_utf8_lossy(&exit.stderr));
}

#[test]
fn benchmark_cli_mutation_is_refused_and_host_still_quits_gracefully() {
    let mut h = host(false);
    prepare_benchmark_fixture(&h);
    let changed_cli = h.base.join("changing-cli");
    fs::copy(&h.settings.cli, &changed_cli).unwrap();
    fs::set_permissions(&changed_cli, fs::Permissions::from_mode(0o700)).unwrap();
    h.settings.cli = changed_cli.clone();
    let artifacts = [h.settings.host.clone(), h.settings.cli.clone()];
    let expected = artifacts.iter().map(|path| hash_file(path)).collect::<Vec<_>>();
    h.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(8));

    let output = h.cli_output(&["status"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut changed = fs::OpenOptions::new().append(true).open(&changed_cli).unwrap();
    changed.write_all(b"\nmutation during benchmark\n").unwrap();
    changed.flush().unwrap();
    let refusal = verify_artifact_hashes(&artifacts, &expected).unwrap_err();
    assert!(refusal.contains("Binary changed during measurement"));

    let exit = h.stop(true).unwrap();
    assert!(exit.status.success(), "{}", String::from_utf8_lossy(&exit.stderr));
}

#[test]
fn benchmark_active_idle_windows_use_probe_deltas_and_keep_partial_requests() {
    let mut h = host(false);
    prepare_benchmark_fixture(&h);
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_perf-probe"));
    h.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(8));
    let mut operations = Vec::new();
    let active = benchmark_observe(&h, &probe, Duration::from_millis(2200), true, |request| {
        operations.push(request["op"].as_str().unwrap().to_owned());
        let reply = runtime_host::status_rpc::request(
            &h.socket(),
            &request,
            Instant::now() + Duration::from_secs(2),
        )
        .map_err(|error| error.to_string())?;
        if reply["ok"] == true { Ok(reply) } else { Err(reply.to_string()) }
    });
    assert_eq!(active["complete"], true);
    assert!(active["requests"].as_u64().unwrap() >= 3, "{active}");
    assert!(operations.contains(&"search".into()));
    assert!(operations.contains(&"clipboard.list".into()));
    assert!(operations.contains(&"monitor".into()));
    assert_eq!(active["samples"].as_array().unwrap().len(), 2);
    for key in ["cpuPercentOneCore", "packageIdleWakeupsPerSecond", "interruptWakeupsPerSecond", "footprintMaxMiB", "diskReadBytes", "diskWriteBytes", "logicalWriteBytes"] {
        assert!(active[key].as_f64().unwrap().is_finite(), "{key}: {active}");
    }

    let idle = benchmark_observe(&h, &probe, Duration::from_millis(150), false, |_| unreachable!());
    assert_eq!(idle["complete"], true);
    assert_eq!(idle["requests"], 0);
    assert_eq!(idle["samples"].as_array().unwrap().len(), 2);

    let mut completed_requests = 0;
    let partial = benchmark_observe(&h, &probe, Duration::from_secs(1), true, |_| {
        completed_requests += 1;
        if completed_requests == 1 { Ok(json!({"ok":true})) } else { Err("bounded RPC timed out".into()) }
    });
    assert_eq!(partial["complete"], false);
    assert_eq!(partial["requests"], 1);
    assert_eq!(partial["samples"].as_array().unwrap().len(), 1);
    assert_eq!(partial["error"], "bounded RPC timed out");

    let exit = h.stop(true).unwrap();
    assert!(exit.status.success(), "{}", String::from_utf8_lossy(&exit.stderr));
}

#[test]
fn benchmark_probe_failure_keeps_prior_samples_and_completed_requests() {
    let timeout = benchmark_probe(Path::new("/bin/sleep"), 10, Duration::from_millis(50))
        .unwrap_err();
    assert!(timeout.contains("timeout"), "{timeout}");
    let malformed = benchmark_probe(Path::new("/bin/echo"), 10, Duration::from_secs(1))
        .unwrap_err();
    assert!(malformed.contains("Probe response is not a JSON object"));

    let mut h = host(false);
    prepare_benchmark_fixture(&h);
    let probe = h.base.join("owned-probe");
    fs::copy(env!("CARGO_BIN_EXE_perf-probe"), &probe).unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    h.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(8));

    let (replace_started, replacement_ready) = mpsc::channel();
    let replacement = probe.clone();
    let replace = thread::spawn(move || {
        replacement_ready.recv_timeout(Duration::from_secs(3)).unwrap();
        fs::remove_file(&replacement).unwrap();
        symlink("/bin/echo", &replacement).unwrap();
    });
    let mut signaled = false;
    let partial = benchmark_observe(&h, &probe, Duration::from_secs(7), true, |request| {
        if !signaled {
            replace_started.send(()).unwrap();
            signaled = true;
        }
        let reply = runtime_host::status_rpc::request(
            &h.socket(),
            &request,
            Instant::now() + Duration::from_secs(2),
        )
        .map_err(|error| error.to_string())?;
        if reply["ok"] == true { Ok(reply) } else { Err(reply.to_string()) }
    });
    replace.join().unwrap();
    assert_eq!(partial["complete"], false);
    assert!(partial["requests"].as_u64().unwrap() > 0);
    assert_eq!(partial["samples"].as_array().unwrap().len(), 1);
    assert!(partial["error"].as_str().unwrap().contains("Probe response"), "{partial}");

    let exit = h.stop(true).unwrap();
    assert!(exit.status.success(), "{}", String::from_utf8_lossy(&exit.stderr));
}
fn acl_rows(path: &Path) -> Vec<String> {
    let output = runtime_host::run(
        Command::new("/bin/ls").args(["-lde"]).arg(path),
        Duration::from_secs(4),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect()
}
fn writer_host(delayed: bool, mutation: bool) -> (Host, PathBuf) {
    let mut host = host(false);
    let marker = host.base.join("writer-entered");
    let sdk = host
        .settings
        .host
        .parent()
        .unwrap()
        .join("examples/libfcntl_sync_shim.dylib");
    assert!(
        sdk.is_file(),
        "Build the same-profile fcntl-sync-shim example first"
    );
    for (k, v) in [
        ("DYLD_INSERT_LIBRARIES", sdk.into_os_string()),
        (
            "RUYCAST_TEST_FSYNC_DIRECTORY",
            host.profile.as_os_str().to_owned(),
        ),
        ("RUYCAST_TEST_FSYNC_MARKER", marker.as_os_str().to_owned()),
    ] {
        host.env.insert(k.into(), v);
    }
    if delayed {
        host.env
            .insert("RUYCAST_TEST_FSYNC_DELAY_MS".into(), "750".into());
        host.env
            .insert("RUYCAST_TEST_FSYNC_DELAY_ONLY".into(), "1".into());
    }
    if mutation {
        host.env
            .insert("RUYCAST_TEST_FSYNC_MUTATE".into(), "selector".into());
        host.env.insert(
            "RUYCAST_TEST_SELECTOR_JSON".into(),
            json!({"directory":host.profile}).to_string().into(),
        );
    }
    host.start();
    assert!(!marker.exists());
    (host, marker)
}
struct RpcWorker {
    result: mpsc::Receiver<std::io::Result<Value>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl RpcWorker {
    fn start(socket: PathBuf, request: Value) -> Self {
        let (tx, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            let value = runtime_host::status_rpc::request(
                &socket,
                &request,
                Instant::now() + Duration::from_secs(3),
            );
            let _ = tx.send(value);
        });
        Self {
            result,
            worker: Some(worker),
        }
    }
    fn finish(mut self) -> Value {
        let result = self.result.recv_timeout(Duration::from_secs(4)).unwrap();
        self.worker.take().unwrap().join().unwrap();
        result.unwrap()
    }
}
impl Drop for RpcWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if self.result.recv_timeout(Duration::from_secs(4)).is_ok() {
                let _ = worker.join();
            }
        }
    }
}
fn read_reply(stream: UnixStream) -> Value {
    let mut reader = BufReader::new(stream.take(1024 * 1024 + 1));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).unwrap();
    assert!(
        line.len() <= 1024 * 1024 && line.last() == Some(&b'\n'),
        "Incomplete or oversized own host response"
    );
    serde_json::from_slice(&line).unwrap()
}
#[test]
fn held_configuration_write_keeps_status_search_responsive_and_second_writer_bounded() {
    let (host, marker) = writer_host(true, false);
    let worker = RpcWorker::start(
        host.socket(),
        json!({"op":"config.directory","directory":host.base.join("portable")}),
    );
    wait_marker(&marker);
    let at = Instant::now();
    let status = host.rpc(&json!({"op":"status"}))["result"].clone();
    assert_eq!(status["configurationUpdating"], true);
    assert_eq!(
        host.rpc(&json!({"op":"search","query":"finder"}))["ok"],
        true
    );
    let rejected = host.rpc(&json!({"op":"config.directory","directory":host.base.join("second")}));
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"].as_str().unwrap().contains("updated"));
    assert!(at.elapsed() < Duration::from_millis(500));
    let result = worker.finish();
    assert_eq!(result["ok"], true);
    assert_eq!(result["configurationWrite"]["published"], true);
    assert_eq!(result["configurationWrite"]["outcomeUncertain"], false);
    assert!(!host.base.join("second/config.json").exists());
    host.wait(|s| s["configurationUpdating"] == false);
}
#[test]
fn quit_while_filesystem_write_is_entered_returns_conservative_configuration_truth() {
    let (mut host, marker) = writer_host(true, false);
    let mut pending = host.connect(Duration::from_secs(3));
    writeln!(
        pending,
        "{}",
        json!({"op":"config.directory","directory":host.base.join("portable")})
    )
    .unwrap();
    wait_marker(&marker);
    let at = Instant::now();
    assert_eq!(host.rpc(&json!({"op":"quit"}))["ok"], true);
    let response = read_reply(pending);
    assert!(at.elapsed() < Duration::from_millis(500));
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["configurationWrite"],
        json!({"published":false,"outcomeUncertain":true})
    );
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("may have been saved"));
    let output = host.wait_exit(Duration::from_secs(4), false);
    assert!(output.status.success());
    assert!(host.base.join("portable/config.json").exists());
    assert!(host.profile.join("config-location.json").exists());
}
#[test]
fn postpublication_selector_conflict_reports_saved_outcome_and_preserves_both_copies() {
    let (host, marker) = writer_host(false, true);
    let before = fs::read(&host.config).unwrap();
    let response =
        host.rpc(&json!({"op":"config.directory","directory":host.base.join("portable")}));
    assert!(marker.exists());
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["configurationWrite"],
        json!({"published":true,"outcomeUncertain":false})
    );
    assert_eq!(response["configurationReconciliationRequired"], true);
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("may have been saved"));
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(
        fs::read(host.base.join("portable/config.json")).unwrap(),
        before
    );
    host.wait(|s| s["configurationUpdating"] == false);
}
#[test]
fn empty_or_missing_explicit_profile_never_modifies_cwd() {
    let owned = OwnedDir::new();
    fs::set_permissions(&owned.0, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = settings();
    for name in [None, Some("RUYCAST_HOME"), Some("RUSTCAST_HOME")] {
        let mut command = Command::new(&settings.host);
        command
            .arg("--headless-test")
            .current_dir(&owned.0)
            .env_clear()
            .env("HOME", &owned.0)
            .env("TMPDIR", &owned.0);
        if let Some(name) = name {
            command.env(name, "");
        }
        let output = runtime_host::run(&mut command, Duration::from_secs(3));
        assert!(!output.status.success());
        assert_eq!(
            fs::metadata(&owned.0).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(fs::read_dir(&owned.0).unwrap().next().is_none());
    }
}
#[test]
fn headless_start_removes_profile_acl_through_legacy_alias() {
    let mut host = host(false);
    let alias = host.base.join("profile-alias");
    symlink(&host.profile, &alias).unwrap();
    fs::set_permissions(&host.profile, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&host.config, fs::Permissions::from_mode(0o600)).unwrap();
    let original = fs::read(&host.config).unwrap();
    let acl = "everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit";
    let installed = runtime_host::run(
        Command::new("/bin/chmod")
            .args(["+a", acl])
            .arg(&host.profile),
        Duration::from_secs(4),
    );
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(
        !acl_rows(&host.profile).is_empty(),
        "fixture must add inherited ACL"
    );
    host.env
        .insert("RUYCAST_HOME".into(), alias.as_os_str().to_owned());

    let deadline = Instant::now() + Duration::from_secs(6);
    host.start_entry_with_timeout(
        runtime_host::Entry::HeadlessTest,
        deadline.saturating_duration_since(Instant::now()),
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    assert!(!remaining.is_zero(), "headless status exceeded six seconds");
    let status_output =
        runtime_host::run(host.command(&host.settings.cli).arg("status"), remaining);
    assert_eq!(
        status_output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let response: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["pid"],
        host.process.as_ref().unwrap().id()
    );
    assert_eq!(response["result"]["headless"], true);
    assert_eq!(
        fs::metadata(&host.profile).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert!(
        acl_rows(&host.profile).is_empty(),
        "profile ACL must be removed"
    );
    assert!(fs::symlink_metadata(&alias)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&alias).unwrap(), host.profile);
    assert_eq!(fs::read(&host.config).unwrap(), original);
}
#[test]
fn legacy_cli_status_search_reload_and_config_edit() {
    let host = host(true);
    let status = host.cli(&["status"])["result"].clone();
    assert_eq!(status["name"], "Ruycast");
    assert_eq!(status["implementation"], "rust");
    assert_eq!(status["productionReady"], false);
    assert_eq!(status["headless"], true);
    assert!(matches!(
        status["health"].as_str(),
        Some("healthy" | "needs-attention")
    ));
    assert_eq!(
        host.rpc(&json!({"op":"search","query":"finder"}))["result"][0]["id"],
        "app.finder"
    );
    assert_eq!(
        host.cli(&["config", "set", "launcherHotkey", "\"cmd+space\""])["ok"],
        true
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap()
            ["launcherHotkey"],
        "cmd+space"
    );
    assert_eq!(host.cli(&["reload"])["ok"], true);
    assert_eq!(
        host.rpc(&json!({"op":"execute","id":"finder","dryRun":true}))["dryRun"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"execute","id":"finder"}))["ok"],
        false
    );
    let original = fs::read(&host.config).unwrap();
    for args in [
        vec!["config", "set", "clipboard.limit", "0"],
        vec!["config", "set", "clipboard.limit", "50", "--expect-sha256", "wrong"],
    ] {
        let output = host.cli_output(&args);
        assert_eq!(output.status.code(), Some(1));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(fs::read(&host.config).unwrap(), original);
    }
    assert_eq!(host.cli(&["config", "set", "clipboard.limit", "50"])["ok"], true);
    assert_eq!(host.cli(&["config", "get"])["result"]["clipboard"]["limit"], 50);
    let unused_legacy = host.base.join("unused-legacy-profile");
    for (new_home, legacy_home) in [
        (None, Some(host.profile.as_path())),
        (Some(host.profile.as_path()), Some(unused_legacy.as_path())),
    ] {
        let mut command = host.command(&host.settings.cli);
        command.arg("status");
        command.env_remove("RUYCAST_HOME");
        if let Some(path) = new_home {
            command.env("RUYCAST_HOME", path);
        }
        if let Some(path) = legacy_home {
            command.env("RUSTCAST_HOME", path);
        }
        let output = runtime_host::run(&mut command, Duration::from_secs(6));
        assert_eq!(output.status.code(), Some(0));
        let status: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(status["ok"], true);
        assert_eq!(status["result"]["profile"], host.profile.to_string_lossy().as_ref());
    }
    assert!(!unused_legacy.exists());
}
#[test]
fn raw_nullable_or_invalid_command_arrays_preserve_last_good() {
    let host = host(true);
    let value = default_config();
    for (field, invalid) in [
        ("aliases", Value::Null),
        ("arguments", Value::Null),
        ("aliases", json!("bad")),
        ("arguments", json!([null])),
        ("aliases", json!({})),
    ] {
        let mut bad = value.clone();
        bad["commands"][0][field] = invalid;
        host.replace(&bad, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], false);
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["commands"],
            value["commands"].as_array().unwrap().len()
        );
        let cli = host.cli_output(&["config", "validate", host.config.to_str().unwrap()]);
        assert!(!cli.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&cli.stdout).unwrap()["ok"],
            false
        );
        host.replace(&value, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    }
    assert_eq!(
        host.cli(&["config", "validate", host.config.to_str().unwrap()])["ok"],
        true
    );
}
#[test]
fn persisted_directory_is_shared_between_cli_and_owned_host_startup() {
    let mut host = host(false);
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    fs::create_dir_all(&portable).unwrap();
    let original = fs::read(&host.config).unwrap();
    fs::write(portable.join("config.json"), &original).unwrap();
    fs::write(
        host.profile.join("config-location.json"),
        serde_json::to_vec(&json!({"directory":portable})).unwrap(),
    )
    .unwrap();
    let selected = portable.join("config.json");
    assert_eq!(host.cli(&["config", "path"])["result"], json!(selected));
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let status = host.cli(&["status"])["result"].clone();
    assert_eq!(status["configPath"], json!(selected));
    assert_eq!(status["profile"], json!(host.profile));
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(fs::read(selected).unwrap(), original);
    // Match the original six-second teardown bound on this exact owned child.
    host.process.as_mut().unwrap().kill().unwrap();
    host.wait_exit(Duration::from_secs(6), true);
}

#[test]
fn owned_host_restart_uses_relocated_configuration() {
    let mut host = host(false);
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    let original = fs::read(&host.config).unwrap();
    assert_eq!(
        host.cli(&["config", "directory", portable.to_str().unwrap()])["ok"],
        true
    );
    for _ in 0..2 {
        host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
        let response = host.cli(&["status"]);
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["configPath"],
            json!(portable.join("config.json"))
        );
        assert_eq!(fs::read(&host.config).unwrap(), original);
        assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
        // Terminate only this unreaped owned child, as the original test did.
        assert_eq!(
            unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
            0
        );
        host.wait_exit(Duration::from_secs(6), false);
    }
}

#[test]
fn atomic_replace_invalid_last_good_and_recovery() {
    let host = host(true);
    let mut changed = default_config();
    changed["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.extra","title":"Extra","kind":"url","target":"https://example.com","aliases":[]}));
    changed["commands"].as_array_mut().unwrap().push(json!({"id":"test.echo","title":"Echo test fixture","kind":"url","target":"https://example.com/echo","aliases":["echo-test"]}));
    host.replace(&changed, None);
    host.wait(|s| s["commands"] == changed["commands"].as_array().unwrap().len());
    host.replace_bytes(b"broken PRIVATE-SENTINEL", None);
    let status = host.wait(|s| !s["configError"].is_null());
    assert_eq!(
        status["commands"],
        changed["commands"].as_array().unwrap().len()
    );
    assert!(!status.to_string().contains("PRIVATE-SENTINEL"));
    assert_eq!(fs::read(&host.config).unwrap(), b"broken PRIVATE-SENTINEL");
    let reload = host.cli_output(&["reload"]);
    assert_eq!(reload.status.code(), Some(1));
    let reload: Value = serde_json::from_slice(&reload.stdout).unwrap();
    assert_eq!(reload["ok"], false);
    let search = host.cli(&["search", "echo-test"]);
    assert_eq!(search["result"][0]["id"], "test.echo");
    assert_eq!(host.cli(&["status"])["result"]["headless"], true);
    host.replace(&default_config(), None);
    host.wait(|s| {
        s["configError"].is_null()
            && s["commands"] == default_config()["commands"].as_array().unwrap().len()
    });
}
#[test]
fn in_place_edit_is_seen_by_five_second_fallback() {
    let host = host(true);
    let mut value = default_config();
    value["theme"] = json!("dark");
    fs::write(&host.config, serde_json::to_vec(&value).unwrap()).unwrap();
    host.wait(|s| s["theme"] == "dark");
}
#[test]
fn external_selector_rebinds_observation_to_portable_directory() {
    let mut host = host(false);
    let mut stale = ruycast_native::profile::ConfigStore::open(
        ruycast_native::profile::ProfilePaths::new(host.profile.clone(), host.base.clone(), None),
    )
    .unwrap();
    let original = fs::read(&host.config).unwrap();
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    fs::create_dir_all(&portable).unwrap();
    fs::set_permissions(&portable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(portable.join("config.json"), &original).unwrap();
    host.replace(
        &json!({"directory":portable}),
        Some(&host.profile.join("config-location.json")),
    );
    let mut changed = default_config();
    changed["clipboard"]["limit"] = json!(51);
    let error = match stale.save(&changed) {
        Ok(_) => panic!("stale selected-file owner must refuse the write"),
        Err(error) => error,
    };
    assert!(error.contains("Configuration changed"), "{error}");
    assert_eq!(stale.file, host.config);
    assert_eq!(stale.snapshot(), original);
    assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    // The Rust host intentionally adopts a valid selector. The frozen Swift
    // test's immediate CLI refusal/old host path are not timing-independent here.
    host.wait(|s| s["configPath"] == portable.join("config.json").to_str().unwrap());
    assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
    assert_eq!(
        host.cli(&["config", "set", "clipboard.limit", "51"])["ok"],
        true
    );
    let mut changed: Value =
        serde_json::from_slice(&fs::read(portable.join("config.json")).unwrap()).unwrap();
    assert_eq!(changed["clipboard"]["limit"], 51);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    changed["theme"] = json!("light");
    host.replace(&changed, Some(&portable.join("config.json")));
    host.wait(|s| s["theme"] == "light");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap(),
        default_config()
    );
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
        0
    );
    host.wait_exit(Duration::from_secs(6), false);
}

#[test]
fn selector_size_bound_and_recovery_keep_valid_state() {
    let host = host(true);
    host.replace_bytes(
        &vec![b' '; 1_000_001],
        Some(&host.profile.join("config-location.json")),
    );
    let status = host.wait(|s| !s["configError"].is_null());
    assert_eq!(
        status["commands"],
        default_config()["commands"].as_array().unwrap().len()
    );
    host.replace(
        &json!({"directory":host.profile}),
        Some(&host.profile.join("config-location.json")),
    );
    host.wait(|s| s["configError"].is_null());
}
#[test]
fn relocate_keeps_original_then_cli_edits_new_target() {
    let mut host = host(false);
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let original = fs::read(&host.config).unwrap();
    let destination = host.base.join("Synced vault with spaces/.appdata/ruycast");
    let initial = host.cli(&["status"]);
    assert_eq!(initial["ok"], true);
    let pid = initial["result"]["pid"].clone();
    assert_eq!(pid, json!(host.process.as_ref().unwrap().id()));
    let result = host.cli(&["config", "directory", destination.to_str().unwrap()]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["originalRetained"], true);
    let status = host.cli(&["status"]);
    assert_eq!(status["ok"], true);
    assert_eq!(status["result"]["pid"], pid);
    assert_eq!(
        status["result"]["configPath"],
        json!(destination.join("config.json"))
    );
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        host.cli(&["config", "set", "clipboard.limit", "51"])["ok"],
        true
    );
    let mut config: Value =
        serde_json::from_slice(&fs::read(destination.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["clipboard"]["limit"], 51);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    config["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.relocated","title":"Relocated fixture","kind":"url","target":"https://example.com"}));
    let candidate = destination.join("replacement.json");
    fs::write(&candidate, serde_json::to_vec(&config).unwrap()).unwrap();
    fs::rename(candidate, destination.join("config.json")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("New location was not observed within original six-second polling budget");
        let output = runtime_host::run(
            host.command(&host.settings.cli).arg("commands"),
            remaining.min(Duration::from_secs(10)),
        );
        assert_eq!(output.status.code(), Some(0));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["ok"], true);
        if response["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["id"] == "fixture.relocated")
        {
            assert!(Instant::now() <= deadline);
            break;
        }
        thread::sleep(
            Duration::from_millis(60).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    assert!(!destination.join("control.sock").exists());
    assert!(host.profile.join("control.sock").exists());
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
        0
    );
    host.wait_exit(Duration::from_secs(6), false);
}

#[test]
fn slow_incomplete_request_has_absolute_read_deadline() {
    let host = host(true);
    let stream = host.connect(Duration::from_secs(5));
    let mut sender = stream.try_clone().unwrap();
    sender
        .set_write_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let (stop_tx, stop_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let at = Instant::now();
    let worker = thread::spawn(move || {
        for _ in 0..20 {
            if sender.write_all(b" ").is_err() {
                break;
            }
            if !matches!(
                stop_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                break;
            }
        }
        let _ = done_tx.send(());
    });
    let shutdown = stream.try_clone().unwrap();
    assert_eq!(read_reply(stream)["ok"], false);
    assert!(at.elapsed() < Duration::from_secs(5));
    let _ = shutdown.shutdown(Shutdown::Both);
    let _ = stop_tx.send(());
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
}
#[test]
fn second_instance_refused_and_crashed_socket_recovers() {
    let mut host = host(true);
    let second = runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(3),
    );
    assert!(!second.status.success());
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
    host.stop(false);
    assert!(host.socket().exists());
    host.start();
    assert_eq!(
        fs::metadata(&host.profile).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(host.socket()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    host.stop(true);
    assert!(!host.socket().exists());
}
#[test]
fn invalid_initial_config_releases_instance_and_socket() {
    let mut host = host(false);
    fs::write(&host.config, b"invalid").unwrap();
    let run = runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(3),
    );
    assert!(!run.status.success());
    assert!(!host.socket().exists());
    fs::write(&host.config, serde_json::to_vec(&default_config()).unwrap()).unwrap();
    host.start();
}
#[test]
fn quit_acknowledges_then_exits_normally_without_kill_fallback() {
    let mut host = host(true);
    let response = runtime_host::run(
        host.command(&host.settings.cli).arg("quit"),
        Duration::from_secs(5),
    );
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&response.stdout).unwrap(),
        json!({"ok":true,"result":{"state":"stopping"}})
    );
    let out = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({"ok":true,"result":{"state":"stopped"}})
    );
    assert!(!host.socket().exists());
}

fn rejected_socket_start(host: &Host) -> std::process::Output {
    runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(4),
    )
}

fn request_open_stream(stream: &mut UnixStream, request: &Value) -> Value {
    stream
        .write_all(&serde_json::to_vec(request).unwrap())
        .unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn start_socket_recovery(host: &mut Host) {
    host.start();
    let status = host.rpc_timeout(&json!({"op": "status"}), Duration::from_secs(2));
    assert_eq!(status["ok"], true);
    assert_eq!(status["result"]["pid"], host.process.as_ref().unwrap().id());
    assert_eq!(status["result"]["headless"], true);
    let profile = PathBuf::from(status["result"]["profile"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(profile, host.profile.canonicalize().unwrap());
}

#[test]
fn socket_recovery_regular_endpoint_is_preserved_then_retry_recovers() {
    let mut host = host(false);
    let payload = b"An unrelated file must survive failed server startup.";
    fs::write(host.socket(), payload).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert_eq!(fs::read(host.socket()).unwrap(), payload);
    fs::remove_file(host.socket()).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
    assert!(!host.socket().exists());
}

#[test]
fn socket_recovery_linked_endpoint_and_target_are_preserved() {
    let mut host = host(false);
    let target = host.base.join("endpoint-target");
    fs::write(&target, b"preserved target").unwrap();
    symlink(&target, host.socket()).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(host.socket())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"preserved target");
    fs::remove_file(host.socket()).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_live_endpoint_without_instance_lock_is_preserved() {
    let mut host = host(false);
    let listener = UnixListener::bind(host.socket()).unwrap();
    let before = fs::metadata(host.socket()).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    let after = fs::metadata(host.socket()).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    drop(listener);
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_stale_endpoint_recovers_and_graceful_quit_removes_it() {
    let mut host = host(false);
    drop(UnixListener::bind(host.socket()).unwrap());
    start_socket_recovery(&mut host);
    host.stop(true);
    assert!(!host.socket().exists());
}

#[test]
fn socket_recovery_graceful_stop_preserves_replacement_regular_file() {
    let mut host = host(false);
    start_socket_recovery(&mut host);
    let mut stream = host.connect(Duration::from_secs(2));
    fs::remove_file(host.socket()).unwrap();
    let replacement = b"replacement while host is alive";
    fs::write(host.socket(), replacement).unwrap();
    assert_eq!(
        request_open_stream(&mut stream, &json!({"op":"quit"}))["ok"],
        true
    );
    let output = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(host.socket()).unwrap(), replacement);
}

#[test]
fn socket_recovery_graceful_stop_preserves_replacement_socket() {
    let mut host = host(false);
    start_socket_recovery(&mut host);
    let mut stream = host.connect(Duration::from_secs(2));
    fs::remove_file(host.socket()).unwrap();
    let replacement = UnixListener::bind(host.socket()).unwrap();
    let before = fs::metadata(host.socket()).unwrap();
    assert_eq!(
        request_open_stream(&mut stream, &json!({"op":"quit"}))["ok"],
        true
    );
    let output = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = fs::metadata(host.socket()).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert!(runtime_host::status_rpc::connect(
        &host.socket(),
        Instant::now() + Duration::from_secs(3)
    )
    .is_ok());
    drop(replacement);
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_linked_instance_lock_is_preserved_then_retry_recovers() {
    let mut host = host(false);
    let target = host.base.join("lock-target");
    fs::write(&target, b"preserved lock target").unwrap();
    let lock = host.profile.join("instance.lock");
    symlink(&target, &lock).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(&lock)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"preserved lock target");
    fs::remove_file(&lock).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_fifo_instance_lock_is_rejected_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let mut host = host(false);
    let lock = host.profile.join("instance.lock");
    let name = std::ffi::CString::new(lock.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(&lock).unwrap().file_type().is_fifo());
    fs::remove_file(&lock).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}

fn run_rust_benchmark_driver(out: &Path, probe: &Path, active: &str, idle: &str) -> std::process::Output {
    let settings = settings();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruycast-build"));
    command.args(["benchmark-host", "--host", settings.host.to_str().unwrap(), "--cli", settings.cli.to_str().unwrap(), "--probe", probe.to_str().unwrap(), "--out", out.to_str().unwrap(), "--cold-starts", "1", "--samples", "1", "--active-seconds", active, "--idle-seconds", idle, "--operation-timeout", "2"]);
    runtime_host::run(&mut command, Duration::from_secs(60))
}

#[test]
fn rust_benchmark_driver_publishes_complete_success_and_graceful_failure_reports() {
    let success_dir = runtime_host::OwnedDir::new();
    let success_path = success_dir.0.join("success.json");
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_perf-probe"));
    let success = run_rust_benchmark_driver(&success_path, &probe, "0.05", "0.05");
    assert!(success.status.success(), "{}", String::from_utf8_lossy(&success.stderr));
    let report: Value = serde_json::from_slice(&fs::read(&success_path).unwrap()).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["fixtureConfigurationIsolated"], true);
    assert_eq!(report["idleEndpoint"]["clipboardCount"], 200);
    assert_eq!(report["idleEndpoint"]["commandID"], "fixture.42");
    assert_eq!(report["coldReady"]["samples"], 1);
    assert_eq!(report["coldShutdowns"][0]["graceful"], true);
    assert_eq!(report["warmCLI"]["status"]["samples"], 1);
    assert_eq!(report["warmCLI"]["search"]["samples"], 1);
    assert_eq!(report["active"]["complete"], true);
    assert_eq!(report["idle"]["complete"], true);
    assert_eq!(report["finalShutdown"]["graceful"], true);
    assert_eq!(fs::read_dir(&success_dir.0).unwrap().count(), 1);

    let failure_dir = runtime_host::OwnedDir::new();
    let failure_path = failure_dir.0.join("failure.json");
    let failed = run_rust_benchmark_driver(&failure_path, Path::new("/usr/bin/false"), "0.05", "0");
    assert!(!failed.status.success(), "{}", String::from_utf8_lossy(&failed.stdout));
    assert!(failure_path.is_file(), "{}", String::from_utf8_lossy(&failed.stderr));
    let failure: Value = serde_json::from_slice(&fs::read(&failure_path).unwrap()).unwrap();
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["phase"], "active");
    assert_eq!(failure["errorType"], "RuntimeError");
    assert!(failure["error"].as_str().unwrap().contains("Probe failed"));
    assert_eq!(failure["coldReady"]["samples"], 1);
    assert_eq!(failure["coldShutdowns"][0]["graceful"], true);
    assert_eq!(failure["warmCLI"]["status"]["samples"], 1);
    assert_eq!(failure["warmCLI"]["search"]["samples"], 1);
    assert_eq!(failure["active"]["complete"], false);
    assert_eq!(failure["active"]["samples"].as_array().unwrap().len(), 0);
    assert_eq!(failure["cleanup"]["graceful"], true);
    assert_eq!(failure["cleanup"]["forcedCleanup"], false);
    assert_eq!(fs::read_dir(&failure_dir.0).unwrap().count(), 1);
}

#[test]
#[ignore = "Requires configured Quick Note provider fixture"]
fn existing_quick_note_provider_writes_only_into_its_owned_fixture_vault() {
    let configured_provider = std::env::var_os("RUYCAST_CAPTURE_PROVIDER")
        .expect("Set RUYCAST_CAPTURE_PROVIDER to the owned fixture provider");
    let configured_provider = PathBuf::from(configured_provider);
    let provider = if configured_provider.is_absolute() {
        configured_provider
    } else {
        std::env::current_dir().unwrap().join(configured_provider)
    };
    assert!(provider.is_file(), "{}", provider.display());

    let mut host = Host::new(Config::current(), false);
    let vault = host.base.join("fixture-vault");
    let inbox = vault.join("00-inbox");
    fs::create_dir_all(&inbox).unwrap();
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    config["commands"].as_array_mut().unwrap().push(json!({
        "id": "capture.quick-note",
        "title": "Quick Note",
        "kind": "script",
        "target": "/bin/sh",
        "arguments": [provider, inbox, "{input}", vault],
        "inputPrompt": "Note"
    }));
    host.replace(&config, None);
    host.start();

    let text = "Rust provider fixture\n- [ ] literal text stays intact";
    let input = json!({"input": text}).to_string();
    let response = host.cli(&["run", "capture.quick-note", "--input", &input]);
    let saved = PathBuf::from(response["result"]["note_path"].as_str().unwrap());
    assert!(saved.starts_with(&vault), "{}", saved.display());
    assert!(fs::read_to_string(saved).unwrap().contains(text));
    assert_eq!(response["result"]["kind"], "note");
}
