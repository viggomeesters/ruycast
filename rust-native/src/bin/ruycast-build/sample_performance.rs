//! Passive counter sampling of an explicitly selected PID. Never launches a host.
use super::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
pub(super) struct Options {
    pid: String,
    seconds: f64,
    probe: PathBuf,
    out: PathBuf,
    label: String,
}
pub(super) fn parse(mut args: Vec<String>) -> Result<Options> {
    let pid = compat::option(&mut args, &["--pid"])?
        .ok_or("--pid is required")?
        .parse::<i64>()
        .map_err(error)?
        .to_string();
    let seconds = compat::option(&mut args, &["--seconds"])?
        .unwrap_or("600".into())
        .parse::<f64>()
        .map_err(error)?;
    if !(seconds > 0.0 && seconds <= 86400.0) {
        return Err("--seconds must be between0 and86400".into());
    }
    let probe =
        PathBuf::from(compat::option(&mut args, &["--probe"])?.ok_or("--probe is required")?);
    let out = PathBuf::from(compat::option(&mut args, &["--out"])?.ok_or("--out is required")?);
    let label = compat::option(&mut args, &["--label"])?.ok_or("--label is required")?;
    if !args.is_empty() {
        return Err("Unexpected sampler arguments".into());
    }
    Ok(Options {
        pid,
        seconds,
        probe,
        out,
        label,
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary<'a> {
    label: &'a str,
    #[serde(rename = "durationS")]
    duration: f64,
    sample_count: usize,
    error: Option<&'a str>,
    scope: &'static str,
    cpu_percent_one_core: f64,
    #[serde(rename = "footprintStartMiB")]
    footprint_start: f64,
    #[serde(rename = "footprintEndMiB")]
    footprint_end: f64,
    #[serde(rename = "footprintMaxMiB")]
    footprint_max: f64,
    package_idle_wakeups_per_second: f64,
    interrupt_wakeups_per_second: f64,
    disk_read_bytes: Numeric,
    disk_write_bytes: Numeric,
    logical_write_bytes: Numeric,
}
#[derive(Serialize)]
struct Report<'a> {
    #[serde(flatten)]
    summary: &'a Summary<'a>,
    samples: &'a [Value],
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum Numeric {
    Integer(i128),
    Float(f64),
}
impl Numeric {
    fn float(self) -> f64 {
        match self {
            Self::Integer(value) => value as f64,
            Self::Float(value) => value,
        }
    }
    fn subtract(self, other: Self) -> Self {
        match (self, other) {
            (Self::Integer(a), Self::Integer(b)) => Self::Integer(a - b),
            (a, b) => Self::Float(a.float() - b.float()),
        }
    }
}
impl Serialize for Numeric {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Integer(value) => serializer.serialize_i128(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
        }
    }
}
fn metric(row: &Value, key: &str) -> Result<Numeric> {
    row[key]
        .as_i64()
        .map(|value| Numeric::Integer(i128::from(value)))
        .or_else(|| {
            row[key]
                .as_u64()
                .map(|value| Numeric::Integer(i128::from(value)))
        })
        .or_else(|| {
            row[key]
                .as_bool()
                .map(|value| Numeric::Integer(if value { 1 } else { 0 }))
        })
        .or_else(|| row[key].as_f64().map(Numeric::Float))
        .ok_or_else(|| format!("Missing/nonnumeric sample metric: {key}"))
}
fn aggregate<'a>(
    label: &'a str,
    samples: &[Value],
    failure: Option<&'a str>,
) -> Result<Summary<'a>> {
    let first = samples.first().ok_or("No samples")?;
    let last = samples.last().unwrap();
    let duration = (last["elapsedS"].as_f64().ok_or("Invalid elapsed time")?
        - first["elapsedS"].as_f64().ok_or("Invalid elapsed time")?)
    .max(0.001);
    let delta = |key| -> Result<Numeric> { Ok(metric(last, key)?.subtract(metric(first, key)?)) };
    let maximum = samples
        .iter()
        .map(|row| metric(row, "footprintBytes").map(Numeric::float))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .fold(f64::NEG_INFINITY, f64::max);
    Ok(Summary{label,duration,sample_count:samples.len(),error:failure,scope:"passive observed host; user clipboard/commands and system load may vary; no energy or display-latency claim",cpu_percent_one_core:100.0*delta("cpuNS")?.float()/1e9/duration,footprint_start:metric(first,"footprintBytes")?.float()/1048576.0,footprint_end:metric(last,"footprintBytes")?.float()/1048576.0,footprint_max:maximum/1048576.0,package_idle_wakeups_per_second:delta("packageIdleWakeups")?.float()/duration,interrupt_wakeups_per_second:delta("interruptWakeups")?.float()/duration,disk_read_bytes:delta("diskReadBytes")?,disk_write_bytes:delta("diskWriteBytes")?,logical_write_bytes:delta("logicalWriteBytes")?})
}
fn probe(path: &Path, pid: &str, timeout: Duration) -> std::result::Result<Value, &'static str> {
    let mut command = Command::new(path);
    command.arg(pid);
    let result = process::capture(command, timeout, 1024 * 1024).map_err(|message| {
        if message.contains("timed out") {
            "TimeoutExpired"
        } else if message.contains("os error 2") {
            "FileNotFoundError"
        } else if message.contains("os error 13") {
            "PermissionError"
        } else {
            "OSError"
        }
    })?;
    if std::str::from_utf8(&result.stdout).is_err() || std::str::from_utf8(&result.stderr).is_err()
    {
        return Err("UnicodeDecodeError");
    }
    if !result.status.success() {
        return Err("CalledProcessError");
    }
    let value: Value = serde_json::from_slice(&result.stdout).map_err(|_| "JSONDecodeError")?;
    if !value.is_object() {
        return Err("TypeError");
    }
    Ok(value)
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(error)?;
    }
    fs::write(path, bytes).map_err(error)
}
pub(super) fn run(options: Options) -> Result<Option<String>> {
    let started = Instant::now();
    let mut samples = Vec::new();
    let failure;
    loop {
        let mut row = match probe(&options.probe, &options.pid, Duration::from_secs(5)) {
            Ok(row) => row,
            Err(reason) => {
                failure = Some(reason);
                break;
            }
        };
        let elapsed = started.elapsed().as_secs_f64();
        row["elapsedS"] = json!(elapsed);
        samples.push(row);
        if elapsed >= options.seconds {
            failure = None;
            break;
        }
        std::thread::sleep(Duration::from_secs_f64(
            (options.seconds - elapsed).clamp(0.01, 5.0),
        ));
    }
    if samples.is_empty() {
        let bytes = format!(
            "{}\n",
            json!({"label":options.label,"error":failure,"sampleCount":0})
        );
        write(&options.out, bytes.as_bytes())?;
        return Ok(None);
    }
    let summary = aggregate(&options.label, &samples, failure)?;
    let mut report = serde_json::to_vec_pretty(&Report {
        summary: &summary,
        samples: &samples,
    })
    .map_err(error)?;
    report.push(b'\n');
    write(&options.out, &report)?;
    Ok(Some(serde_json::to_string(&summary).map_err(error)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_owned_rust_probe_first_partial_malformed_and_timeout_results() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let binary = root.join("probe");
        let mut compiler = Command::new("rustc");
        compiler
            .args(["--edition=2021"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/performance_sampler_probe.rs"),
            )
            .arg("-o")
            .arg(&binary);
        assert!(
            process::capture(compiler, Duration::from_secs(30), 1024 * 1024)
                .unwrap()
                .status
                .success()
        );
        let out = root.join("report.json");
        for (mode, expected) in [
            ("fail", "CalledProcessError"),
            ("nonjson", "JSONDecodeError"),
            ("notobject", "TypeError"),
        ] {
            fs::write(root.join("mode"), mode).unwrap();
            let result = run(Options {
                pid: std::process::id().to_string(),
                seconds: 0.02,
                probe: binary.clone(),
                out: out.clone(),
                label: "owned".into(),
            })
            .unwrap();
            assert!(result.is_none());
            let report: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
            assert_eq!(
                report,
                json!({"label":"owned","error":expected,"sampleCount":0})
            );
            assert!(!fs::read_to_string(&out).unwrap().contains("canary"));
        }
        fs::write(root.join("mode"), "partial").unwrap();
        let _ = fs::remove_file(root.join("sampled"));
        let result = run(Options {
            pid: std::process::id().to_string(),
            seconds: 1.0,
            probe: binary.clone(),
            out: out.clone(),
            label: "owned".into(),
        })
        .unwrap()
        .unwrap();
        let summary: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(summary["sampleCount"], 1);
        assert_eq!(summary["error"], "CalledProcessError");
        assert!(summary.get("samples").is_none());
        let report: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        assert_eq!(report["samples"].as_array().unwrap().len(), 1);
        fs::write(root.join("mode"), "hold").unwrap();
        assert_eq!(
            probe(
                &binary,
                &std::process::id().to_string(),
                Duration::from_millis(80)
            )
            .unwrap_err(),
            "TimeoutExpired"
        );
    }
    #[test]
    fn aggregation_preserves_delta_units_partial_error_and_minimum_duration() {
        let samples = vec![
            json!({"elapsedS":0.0,"cpuNS":9007199254740993u64,"footprintBytes":1048576,"packageIdleWakeups":100,"interruptWakeups":10,"diskReadBytes":30,"diskWriteBytes":40,"logicalWriteBytes":50}),
            json!({"elapsedS":2.0,"cpuNS":9007200254740993u64,"footprintBytes":2097152,"packageIdleWakeups":104,"interruptWakeups":16,"diskReadBytes":35,"diskWriteBytes":38,"logicalWriteBytes":60}),
        ];
        let result = aggregate("owned", &samples, Some("CalledProcessError")).unwrap();
        assert_eq!(result.cpu_percent_one_core, 50.0);
        assert_eq!(result.package_idle_wakeups_per_second, 2.0);
        assert_eq!(result.interrupt_wakeups_per_second, 3.0);
        assert_eq!(result.footprint_max, 2.0);
        assert_eq!(result.disk_write_bytes, Numeric::Integer(-2));
        assert_eq!(result.error, Some("CalledProcessError"));
        let mut floating = samples.clone();
        floating[0]["diskWriteBytes"] = json!(1.5);
        floating[1]["diskWriteBytes"] = json!(2.75);
        assert_eq!(
            aggregate("owned", &floating, None)
                .unwrap()
                .disk_write_bytes,
            Numeric::Float(1.25)
        );

        assert_eq!(
            aggregate("owned", &samples[..1], None).unwrap().duration,
            0.001
        );
    }
    #[test]
    fn flags_reject_nonfinite_or_invalid_durations_and_missing_fields() {
        assert!(parse(vec![]).is_err());
        for value in ["0", "-1", "NaN", "inf", "86401"] {
            assert!(parse(vec![
                "--pid".into(),
                "1".into(),
                "--seconds".into(),
                value.into(),
                "--probe".into(),
                "probe".into(),
                "--out".into(),
                "out".into(),
                "--label".into(),
                "owned".into()
            ])
            .is_err());
        }
    }
}
