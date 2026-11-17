//! Isolated host benchmark runner used after native build artifacts exist.
use super::{error, process};
use ruycast_native::benchmark_host::{self, Cleanup};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::DirBuilderExt,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
struct Options {
    host: PathBuf,
    cli: PathBuf,
    probe: PathBuf,
    out: PathBuf,
    cold_starts: usize,
    samples: usize,
    active_seconds: Duration,
    idle_seconds: Duration,
    operation_timeout: Duration,
}

fn take(options: &mut BTreeMap<String, String>, name: &str) -> Result<String> {
    options
        .remove(name)
        .ok_or_else(|| format!("Missing {name}"))
}

fn parse(arguments: Vec<String>) -> Result<Options> {
    let mut values = BTreeMap::new();
    let mut args = arguments.into_iter();
    while let Some(name) = args.next() {
        if !name.starts_with("--") {
            return Err(format!("Unexpected benchmark argument: {name}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {name}"))?;
        if values.insert(name.clone(), value).is_some() {
            return Err(format!("Duplicate benchmark argument: {name}"));
        }
    }
    let path = |name: &str, values: &mut BTreeMap<String, String>| -> Result<PathBuf> {
        let raw = PathBuf::from(take(values, name)?);
        if !raw.is_absolute() {
            return Err(format!("{name} must be absolute"));
        }
        if name == "--out" {
            Ok(raw)
        } else {
            raw.canonicalize().map_err(error)
        }
    };
    let host = path("--host", &mut values)?;
    let cli = path("--cli", &mut values)?;
    let probe = path("--probe", &mut values)?;
    let out = path("--out", &mut values)?;
    let cold_starts = values
        .remove("--cold-starts")
        .unwrap_or_else(|| "30".into())
        .parse::<usize>()
        .map_err(error)?;
    let samples = values
        .remove("--samples")
        .unwrap_or_else(|| "1000".into())
        .parse::<usize>()
        .map_err(error)?;
    let active_seconds = values
        .remove("--active-seconds")
        .unwrap_or_else(|| "600".into())
        .parse::<f64>()
        .map_err(error)?;
    let idle_seconds = values
        .remove("--idle-seconds")
        .unwrap_or_else(|| "600".into())
        .parse::<f64>()
        .map_err(error)?;
    let operation_timeout = values
        .remove("--operation-timeout")
        .unwrap_or_else(|| "5".into())
        .parse::<f64>()
        .map_err(error)?;
    if !values.is_empty() {
        return Err(format!(
            "Unknown benchmark options: {}",
            values.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if cold_starts == 0
        || cold_starts > 1_000
        || samples == 0
        || samples > 10_000
        || !active_seconds.is_finite()
        || active_seconds < 0.0
        || !idle_seconds.is_finite()
        || idle_seconds < 0.0
        || !operation_timeout.is_finite()
        || operation_timeout <= 0.0
    {
        return Err("Counts must be positive; durations finite and nonnegative; operation timeout finite and positive".into());
    }
    let seconds = |name: &str, value: f64, allow_zero: bool| -> Result<Duration> {
        if !value.is_finite() || value < 0.0 || (!allow_zero && value == 0.0) || value > 86_400.0 {
            return Err(format!(
                "{name} must be within the supported 24-hour duration bound"
            ));
        }
        let duration = Duration::try_from_secs_f64(value).map_err(error)?;
        if value > 0.0 && duration.is_zero() {
            return Err(format!("{name} is below the supported duration precision"));
        }
        Ok(duration)
    };
    let active_duration = seconds("--active-seconds", active_seconds, true)?;
    let idle_duration = seconds("--idle-seconds", idle_seconds, true)?;
    let operation_timeout = seconds("--operation-timeout", operation_timeout, false)?;
    Ok(Options {
        host,
        cli,
        probe,
        out,
        cold_starts,
        samples,
        active_seconds: active_duration,
        idle_seconds: idle_duration,
        operation_timeout,
    })
}

struct Fixture(PathBuf);
impl Fixture {
    fn create() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "rc-h-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(error)?;
        Ok(Self(root))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn digest(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(error)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(format!(
            "Benchmark tool exceeds size bound: {}",
            path.display()
        ));
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn verify_tools(options: &Options, expected: &[String; 3]) -> Result<()> {
    for (path, expected_digest) in [
        (&options.host, &expected[0]),
        (&options.cli, &expected[1]),
        (&options.probe, &expected[2]),
    ] {
        if digest(path)? != *expected_digest {
            return Err(format!(
                "{} binary changed during measurement",
                if path == &options.host {
                    "Host"
                } else if path == &options.cli {
                    "CLI"
                } else {
                    "Probe"
                }
            ));
        }
    }
    Ok(())
}
fn tool_hashes(options: &Options) -> Result<[String; 3]> {
    Ok([
        digest(&options.host)?,
        digest(&options.cli)?,
        digest(&options.probe)?,
    ])
}

fn now_rfc3339() -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(error)?
        .as_secs() as libc::time_t;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::gmtime_r(&now, &mut parts) }.is_null() {
        return Err("UTC timestamp conversion failed".into());
    }
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        parts.tm_year + 1900,
        parts.tm_mon + 1,
        parts.tm_mday,
        parts.tm_hour,
        parts.tm_min,
        parts.tm_sec
    ))
}

fn child_env(home: &Path) -> BTreeMap<std::ffi::OsString, std::ffi::OsString> {
    let mut env = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            ![
                "RUYCAST_HOME",
                "RUSTCAST_HOME",
                "RUYCAST_CONFIG_DIR",
                "RUYCAST_APP_BINARY",
            ]
            .contains(&key.as_ref())
        })
        .collect::<BTreeMap<_, _>>();
    env.insert("RUYCAST_HOME".into(), home.as_os_str().to_owned());
    env
}

fn init_fixture(
    options: &Options,
    root: &Path,
    env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
) -> Result<PathBuf> {
    let mut command = Command::new(&options.host);
    command
        .args(["--init-config"])
        .env_clear()
        .envs(env)
        .current_dir(root);
    let output = process::capture(command, options.operation_timeout, 1024 * 1024)?;
    if !output.status.success() {
        return Err(format!(
            "Host configuration initialization failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let config_path = root.join("config.json");
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(error)?).map_err(error)?;
    if !config.is_object() {
        return Err("Host configuration initialization did not produce a JSON object".into());
    }
    config["commands"] = (0..500).map(|i| json!({"id":format!("fixture.{i}"),"title":format!("Open Project {i}"),"kind":"url","target":format!("https://example.com/{i}"),"aliases":[format!("p{i}")]})).collect::<Vec<_>>().into();
    fs::write(&config_path, serde_json::to_vec(&config).map_err(error)?).map_err(error)?;
    let date = now_rfc3339()?;
    let history = (0..200).map(|i| json!({"id":format!("fixture-{i}"),"text":format!("Fixture {i}: {}", "native clipboard Ω ".repeat(400)),"source":"fixture","date":date})).collect::<Vec<_>>();
    fs::write(
        root.join("clipboard.json"),
        serde_json::to_vec(&history).map_err(error)?,
    )
    .map_err(error)?;
    Ok(config_path)
}

fn status(socket: &Path, config: &Path, deadline: Instant) -> Result<Value> {
    let response =
        benchmark_host::rpc::request(socket, &json!({"op":"status"}), deadline).map_err(error)?;
    if response["ok"] != true {
        return Err(format!("Status refused: {response}"));
    }
    let result = &response["result"];
    if result["headless"] != true
        || Path::new(
            result["configPath"]
                .as_str()
                .ok_or("Status omitted config path")?,
        ) != config
        || result["commands"] != 500
        || result["clipboardCount"] != 200
    {
        return Err("Benchmark escaped its 500-command/200-clipboard headless fixture".into());
    }
    Ok(result.clone())
}

fn start(
    options: &Options,
    root: &Path,
    config: &Path,
    env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    expected: &[String; 3],
    startup_cleanup: &mut Option<Cleanup>,
) -> Result<(Child, f64)> {
    verify_tools(options, expected)?;
    let began = Instant::now();
    let child = Command::new(&options.host)
        .arg("--headless")
        .env_clear()
        .envs(env)
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(error)?;
    let mut child = ChildGuard(Some(child));
    let socket = root.join("control.sock");
    let deadline = began
        .checked_add(Duration::from_secs(10))
        .ok_or("Startup deadline overflow")?;
    loop {
        match child.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => {
                *startup_cleanup = Some(Cleanup {
                    pid: child.as_ref().unwrap().id(),
                    graceful: false,
                    forced_cleanup: false,
                    method: "exited",
                    exit_code: status.code(),
                    signal: status.signal(),
                    quit_error: Some("Host exited during startup".into()),
                });
                let _ = child.take();
                return Err(format!("Host exited during startup: {status}"));
            }
            Err(error) => {
                *startup_cleanup = Some(benchmark_host::shutdown(
                    child.take().unwrap(),
                    options.operation_timeout,
                    |_| Err("host startup status check failed".into()),
                ));
                return Err(format!("Could not check host startup: {error}"));
            }
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            *startup_cleanup = Some(benchmark_host::shutdown(
                child.take().unwrap(),
                options.operation_timeout,
                |_| Err("cold host startup timed out".into()),
            ));
            return Err("Cold host startup timed out".into());
        }
        if status(
            &socket,
            config,
            Instant::now()
                .checked_add(options.operation_timeout)
                .ok_or("Status deadline overflow")?
                .min(deadline),
        )
        .is_ok()
        {
            return Ok((
                child.take().unwrap(),
                began.elapsed().as_secs_f64() * 1000.0,
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

struct ChildGuard(Option<Child>);
impl ChildGuard {
    fn as_ref(&self) -> Option<&Child> {
        self.0.as_ref()
    }
    fn as_mut(&mut self) -> Option<&mut Child> {
        self.0.as_mut()
    }
    fn take(&mut self) -> Option<Child> {
        self.0.take()
    }
    fn set(&mut self, child: Child) {
        self.0 = Some(child);
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            let _ = benchmark_host::shutdown(child, Duration::from_secs(2), |_| {
                Err("benchmark child guard cleanup".into())
            });
        }
    }
}

fn stop_host(child: Child, socket: &Path, timeout: Duration) -> Cleanup {
    benchmark_host::shutdown(child, timeout, |deadline| {
        let reply =
            benchmark_host::rpc::request(socket, &json!({"op":"quit"}), deadline).map_err(error)?;
        if reply["ok"] == true {
            Ok(())
        } else {
            Err(format!("Quit refused: {reply}"))
        }
    })
}

fn cleanup_value(cleanup: &Cleanup) -> Value {
    let mut value = serde_json::to_value(cleanup).unwrap();
    if let Some(signal) = cleanup.signal {
        value["exitCode"] = json!(-signal);
    }
    value
}

fn stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"samples":sorted.len(),"medianMS":sorted[sorted.len()/2],"p95MS":sorted[((sorted.len()-1) as f64*0.95) as usize],"p99MS":if sorted.len()>=1000{json!(sorted[((sorted.len()-1) as f64*0.99) as usize])}else{Value::Null},"maxMS":sorted[sorted.len()-1]})
}

fn cli_measure(
    options: &Options,
    env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    expected: &[String; 3],
    args: &[&str],
    name: &str,
    samples: usize,
) -> Result<Value> {
    verify_tools(options, expected)?;
    let mut times = Vec::with_capacity(samples);
    let total = samples.checked_add(10).ok_or("CLI sample count overflow")?;
    for index in 0..total {
        let began = Instant::now();
        let mut command = Command::new(&options.cli);
        command.args(args).env_clear().envs(env);
        let output = process::capture(command, options.operation_timeout, 1024 * 1024)?;
        if !output.status.success() {
            return Err(format!(
                "{name} CLI failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let result: Value = serde_json::from_slice(&output.stdout).map_err(error)?;
        if result["ok"] != true {
            return Err(format!("{name} CLI returned failure: {result}"));
        }
        if index >= 10 {
            times.push(began.elapsed().as_secs_f64() * 1000.0);
        }
    }
    verify_tools(options, expected)?;
    Ok(stats(&times))
}

fn probe(options: &Options, pid: u32, deadline: Duration) -> Result<Value> {
    let mut command = Command::new(&options.probe);
    command.arg(pid.to_string());
    let output = process::capture(command, deadline, 1024 * 1024)?;
    if !output.status.success() {
        return Err(format!(
            "Probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let row: Value = serde_json::from_slice(&output.stdout).map_err(error)?;
    if !row.is_object() {
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
        if row[key].as_u64().is_none() {
            return Err(format!("Probe response has no unsigned {key} counter"));
        }
    }
    Ok(row)
}

fn observe(
    options: &Options,
    child: &Child,
    socket: &Path,
    expected: &[String; 3],
    duration: Duration,
    active: bool,
) -> Value {
    let mut requests = 0_u64;
    let mut samples = Vec::new();
    let fail = |error: String, requests, samples: Vec<Value>| json!({"requestedDurationS":duration.as_secs_f64(),"samples":samples,"requests":requests,"complete":false,"error":error});
    if let Err(error) = verify_tools(options, expected) {
        return fail(error, requests, samples);
    }
    let mut first = match probe(options, child.id(), options.operation_timeout) {
        Ok(row) => row,
        Err(error) => return fail(error, requests, samples),
    };
    first["elapsedS"] = json!(0.0);
    samples.push(first);
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
                match benchmark_host::rpc::request(
                    socket,
                    &call,
                    Instant::now() + options.operation_timeout,
                ) {
                    Ok(reply) if reply["ok"] == true => requests += 1,
                    Ok(reply) => return fail(format!("RPC refused: {reply}"), requests, samples),
                    Err(error) => return fail(error.to_string(), requests, samples),
                }
            }
            if last_monitor.elapsed() >= Duration::from_secs(2) {
                match benchmark_host::rpc::request(
                    socket,
                    &json!({"op":"monitor"}),
                    Instant::now() + options.operation_timeout,
                ) {
                    Ok(reply) if reply["ok"] == true => requests += 1,
                    Ok(reply) => {
                        return fail(format!("Monitor refused: {reply}"), requests, samples)
                    }
                    Err(error) => return fail(error.to_string(), requests, samples),
                }
                last_monitor = Instant::now();
            }
        }
        thread::sleep(
            Duration::from_millis(100).min(
                duration
                    .saturating_sub(started.elapsed())
                    .max(Duration::from_millis(1)),
            ),
        );
        if last_sample.elapsed() >= Duration::from_secs(5) {
            if let Err(error) = verify_tools(options, expected) {
                return fail(error, requests, samples);
            }
            let mut row = match probe(options, child.id(), options.operation_timeout) {
                Ok(row) => row,
                Err(error) => return fail(error, requests, samples),
            };
            row["elapsedS"] = json!(started.elapsed().as_secs_f64());
            samples.push(row);
            last_sample = Instant::now();
        }
    }
    if let Err(error) = verify_tools(options, expected) {
        return fail(error, requests, samples);
    }
    let mut last = match probe(options, child.id(), options.operation_timeout) {
        Ok(row) => row,
        Err(error) => return fail(error, requests, samples),
    };
    last["elapsedS"] = json!(started.elapsed().as_secs_f64());
    samples.push(last);
    let first = &samples[0];
    let last = samples.last().unwrap();
    let seconds = last["elapsedS"].as_f64().unwrap() - first["elapsedS"].as_f64().unwrap();
    let delta = |key: &str| {
        (last[key].as_u64().unwrap() as i128 - first[key].as_u64().unwrap() as i128) as f64
    };
    json!({"requestedDurationS":duration.as_secs_f64(),"durationS":seconds,"samples":samples,"requests":requests,"complete":true,
        "cpuPercentOneCore":100.0*delta("cpuNS")/1e9/seconds,"packageIdleWakeupsPerSecond":delta("packageIdleWakeups")/seconds,"interruptWakeupsPerSecond":delta("interruptWakeups")/seconds,
        "footprintStartMiB":first["footprintBytes"].as_f64().unwrap()/1048576.0,"footprintEndMiB":last["footprintBytes"].as_f64().unwrap()/1048576.0,
        "footprintMaxMiB":samples.iter().map(|row|row["footprintBytes"].as_f64().unwrap()).fold(0.0_f64,f64::max)/1048576.0,
        "diskReadBytes":delta("diskReadBytes"),"diskWriteBytes":delta("diskWriteBytes"),"logicalWriteBytes":delta("logicalWriteBytes")})
}

fn atomic_report(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or("Benchmark report requires a parent directory")?;
    fs::create_dir_all(parent).map_err(error)?;
    ruycast_native::private_storage::write(
        path,
        &serde_json::to_vec_pretty(value).map_err(error)?,
        true,
    )
    .map_err(error)
}

pub(super) fn run(arguments: Vec<String>) -> Result<()> {
    let options = parse(arguments)?;
    let hashes = tool_hashes(&options)?;
    let fixture = Fixture::create()?;
    let root = &fixture.0;
    let env = child_env(root);
    let mut state = json!({"ok":false,"phase":"init-config","fixtureConfigurationIsolated":true,"coldReady":[],"coldShutdowns":[],"warmCLI":{},"active":null,"idle":null,"hostSHA256":hashes[0],"cliSHA256":hashes[1],"probeSHA256":hashes[2],"operationTimeoutS":options.operation_timeout.as_secs_f64(),"onDiskHostChangedDuringRun":false,"onDiskCLIChangedDuringRun":false});
    let mut running = ChildGuard(None);
    let mut startup_cleanup = None;
    let result = (|| -> Result<()> {
        let config = init_fixture(&options, root, &env)?;
        for index in 0..options.cold_starts {
            state["phase"] = json!(format!("cold-start-{}", index + 1));
            let (child, elapsed) =
                start(&options, root, &config, &env, &hashes, &mut startup_cleanup)?;
            state["coldReady"]
                .as_array_mut()
                .unwrap()
                .push(json!(elapsed));
            state["phase"] = json!(format!("cold-shutdown-{}", index + 1));
            let cleanup = stop_host(child, &root.join("control.sock"), options.operation_timeout);
            if !cleanup.graceful {
                state["cleanup"] = cleanup_value(&cleanup);
                return Err(format!(
                    "Cold host did not shut down gracefully: {}",
                    serde_json::to_string(&cleanup).unwrap()
                ));
            }
            state["coldShutdowns"]
                .as_array_mut()
                .unwrap()
                .push(cleanup_value(&cleanup));
        }
        let cold = state["coldReady"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect::<Vec<_>>();
        state["coldReady"] = stats(&cold);
        state["phase"] = json!("warm-start");
        let (child, _) = start(&options, root, &config, &env, &hashes, &mut startup_cleanup)?;
        running.set(child);
        for (name, args) in [
            ("status", vec!["status"]),
            ("search", vec!["search", "Project 42"]),
        ] {
            state["phase"] = json!(format!("cli-{name}"));
            let stats = cli_measure(&options, &env, &hashes, &args, name, options.samples)?;
            state["warmCLI"][name] = stats;
        }
        verify_tools(&options, &hashes)?;
        let child = running.as_ref().unwrap();
        let socket = root.join("control.sock");
        if !options.active_seconds.is_zero() {
            state["phase"] = json!("active");
            state["active"] = observe(
                &options,
                child,
                &socket,
                &hashes,
                options.active_seconds,
                true,
            );
            if state["active"]["complete"] != true {
                return Err(state["active"]["error"]
                    .as_str()
                    .unwrap_or("Active measurement failed")
                    .into());
            }
        }
        if !options.idle_seconds.is_zero() {
            state["phase"] = json!("idle");
            state["idle"] = observe(
                &options,
                child,
                &socket,
                &hashes,
                options.idle_seconds,
                false,
            );
            if state["idle"]["complete"] != true {
                return Err(state["idle"]["error"]
                    .as_str()
                    .unwrap_or("Idle measurement failed")
                    .into());
            }
        }
        state["phase"] = json!("idle-endpoint");
        let after = status(&socket, &config, Instant::now() + options.operation_timeout)?;
        let search = benchmark_host::rpc::request(
            &socket,
            &json!({"op":"search","query":"Project 42"}),
            Instant::now() + options.operation_timeout,
        )
        .map_err(error)?;
        if search["ok"] != true
            || !search["result"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|row| row["id"] == "fixture.42"))
        {
            return Err("Post-idle search lost fixture command".into());
        }
        state["idleEndpoint"] = json!({"statusVerified":true,"searchVerified":true,"commandID":"fixture.42","idleSeconds":options.idle_seconds.as_secs_f64(),"clipboardCount":after["clipboardCount"]});
        state["phase"] = json!("final-shutdown");
        let child = running.take().unwrap();
        let cleanup = stop_host(child, &socket, options.operation_timeout);
        if !cleanup.graceful {
            state["cleanup"] = cleanup_value(&cleanup);
            return Err(format!(
                "Final host did not shut down gracefully: {}",
                serde_json::to_string(&cleanup).unwrap()
            ));
        }
        state["finalShutdown"] = cleanup_value(&cleanup);
        verify_tools(&options, &hashes)?;
        state["scope"]=json!("500 commands / 200 text fixtures, headless isolated host, no hotkey/discovery/display/pasteboard or physical energy claim; active includes monitor RPC every 2 seconds");
        state["ok"] = json!(true);
        state["phase"] = json!("done");
        Ok(())
    })();
    if let Err(problem) = result {
        let cleanup = running
            .take()
            .map(|child| stop_host(child, &root.join("control.sock"), options.operation_timeout));
        let error_type = if problem.contains("Tool process timed out") {
            "TimeoutExpired"
        } else if problem.contains("RPC deadline")
            || problem.to_lowercase().contains("timeout")
            || problem.to_lowercase().contains("timed out")
        {
            "TimeoutError"
        } else {
            "RuntimeError"
        };
        state["ok"] = json!(false);
        state["errorType"] = json!(error_type);
        state["error"] = json!(problem);
        state["onDiskHostChangedDuringRun"] = match digest(&options.host) {
            Ok(value) => json!(value != hashes[0]),
            Err(_) => Value::Null,
        };
        state["onDiskCLIChangedDuringRun"] = match digest(&options.cli) {
            Ok(value) => json!(value != hashes[1]),
            Err(_) => Value::Null,
        };
        if let Some(cold) = state["coldReady"].as_array() {
            if cold.is_empty() {
                state["coldReady"] = Value::Null;
            } else {
                let values = cold.iter().filter_map(Value::as_f64).collect::<Vec<_>>();
                state["coldReady"] = stats(&values);
            }
        }
        if let Some(cleanup) = cleanup {
            state["cleanup"] = cleanup_value(&cleanup);
        } else if let Some(cleanup) = startup_cleanup {
            state["cleanup"] = cleanup_value(&cleanup);
        }
        atomic_report(&options.out, &state)?;
        return Err(format!(
            "Benchmark failed during {}: {}",
            state["phase"].as_str().unwrap_or("unknown"),
            state["error"].as_str().unwrap_or("unknown")
        ));
    }
    atomic_report(&options.out, &state)?;
    println!(
        "{}",
        serde_json::to_string(&json!({"phase":"done","ok":true,"report":options.out}))
            .map_err(error)?
    );
    Ok(())
}
