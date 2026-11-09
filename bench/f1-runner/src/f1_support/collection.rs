//! Review-gated collection transport. Endpoints must implement the full protocol;
//! there is deliberately no normal-app/raw-legacy-flag fallback.
use super::*;
use crate::guardian;
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::io::AsRawFd,
    time::Duration,
};

fn append_cycle_output(
    runs: &mut Vec<ruycast_native::native_f1::LifecycleOutputRun>,
    output: Value,
) {
    if let Some(last) = runs.last_mut().filter(|last| last.output == output) {
        last.repetitions += 1;
    } else {
        runs.push(ruycast_native::native_f1::LifecycleOutputRun {
            repetitions: 1,
            output,
        });
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    executable: Binding,
    sdk_guard: Binding,
    reference_bridge: Option<Binding>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adapters {
    schema: String,
    endpoints: BTreeMap<String, Endpoint>,
    pre_entry_review_sha256: String,
}
struct Worker {
    endpoint: guardian::Endpoint,
    input: File,
    output: File,
    buffer: Vec<u8>,
    clock_resolution_ns: u64,
    counter_birth: Option<u64>,
    counter_epoch: Instant,
    raw_directory: Option<PathBuf>,
    transport_finished: bool,
}
fn native_file(binding: &Binding, kind: u32) -> Result<()> {
    binding.verify()?;
    let canonical = fs::canonicalize(&binding.path).map_err(|e| e.to_string())?;
    if canonical != binding.path
        || !canonical.starts_with(super::preparation::checkout_root()?.join("rust-native/target"))
    {
        return Err("endpoints/guards must be exact owned task paths; installed apps and symlink wrappers are forbidden".into());
    }
    let bytes = fs::read(&canonical).map_err(|e| e.to_string())?;
    if bytes.len() < 32
        || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 0x0100000c
        || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) != kind
    {
        return Err("endpoint must be a bound arm64 Mach-O executable; guard/bridge a Mach-O dylib, no scripts or loader fallback".into());
    }
    Ok(())
}
impl Worker {
    fn start(
        spec: &Endpoint,
        role: &str,
        root: &Path,
        fixture: &Binding,
        authority: &str,
    ) -> Result<Self> {
        native_file(&spec.executable, 2)?;
        native_file(&spec.sdk_guard, 6)?;
        if let Some(b) = &spec.reference_bridge {
            native_file(b, 6)?;
        }
        let home = root.join(role);
        fs::create_dir(&home).map_err(|e| e.to_string())?;
        let profile = home.join("profile");
        fs::create_dir(&profile).map_err(|e| e.to_string())?;
        let stderr = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("{role}.stderr")))
            .map_err(|e| e.to_string())?;
        let mut env = vec![
            ("HOME".into(), home.display().to_string()),
            ("TMPDIR".into(), home.display().to_string()),
            ("RUYCAST_HOME".into(), profile.display().to_string()),
            ("RUSTCAST_HOME".into(), profile.display().to_string()),
            (
                "RUYCAST_F1_FIXTURE".into(),
                fixture.path.display().to_string(),
            ),
            ("RUYCAST_F1_FIXTURE_SHA256".into(), fixture.sha256.clone()),
            ("RUYCAST_F1_AUTHORITY_SHA256".into(), authority.into()),
            ("RUYCAST_F1_ARM".into(), role.into()),
            (
                "DYLD_INSERT_LIBRARIES".into(),
                spec.sdk_guard.path.display().to_string(),
            ),
            (
                "RUYCAST_F1_GUARD_SHA256".into(),
                spec.sdk_guard.sha256.clone(),
            ),
        ];
        if let Some(b) = &spec.reference_bridge {
            env.push((
                "RUYCAST_F1_REFERENCE_BRIDGE".into(),
                b.path.display().to_string(),
            ));
            env.push((
                "RUYCAST_F1_REFERENCE_BRIDGE_SHA256".into(),
                b.sha256.clone(),
            ));
        }
        let mut worker = Self::spawn(
            &spec.executable.path,
            &["__f1-owned-host".into()],
            &home,
            &env,
            &stderr,
        )?;
        worker.raw_directory = Some(home.clone());
        let handshake = (|| -> Result<()> {
            let mut ready = worker.line(Instant::now() + Duration::from_secs(15))?;
            worker.clock_resolution_ns = integer(&ready, "clock_resolution_ns")?;
            if worker.clock_resolution_ns == 0 {
                return Err("invalid endpoint timer resolution".into());
            }
            ready
                .as_object_mut()
                .ok_or("endpoint ready envelope missing")?
                .remove("clock_resolution_ns");
            if ready
                != json!({"schema":SCHEMA,"phase":"ready","fixture_sha256":fixture.sha256,"authority_sha256":authority,"sdk_guard_sha256":spec.sdk_guard.sha256})
            {
                return Err("endpoint handshake mismatch; post-entry handshake is not a substitute for pinned pre-entry review".into());
            }
            Ok(())
        })();
        if let Err(error) = handshake {
            let cleanup = worker.cancel_and_drain(Duration::from_secs(3));
            let record = match &cleanup {
                Ok(value) => json!({"handshake_error":error,"cleanup":value}),
                Err(cleanup_error) => json!({"handshake_error":error,"cleanup_error":cleanup_error}),
            };
            worker.preserve("startup-cleanup.json", &record)?;
            return Err(match cleanup {
                Ok(_) => error,
                Err(cleanup_error) => format!("{error}; endpoint cleanup: {cleanup_error}"),
            });
        }
        Ok(worker)
    }
    fn spawn(
        program: &Path,
        args: &[String],
        home: &Path,
        env: &[(String, String)],
        stderr: &File,
    ) -> Result<Self> {
        let (input_r, input_w) = guardian::pipe().map_err(|e| e.to_string())?;
        let (output_r, output_w) = guardian::pipe().map_err(|e| e.to_string())?;
        let input: File = input_w.into();
        let output: File = output_r.into();
        for fd in [input.as_raw_fd(), output.as_raw_fd()] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err("cannot install bounded endpoint IO".into());
            }
        }
        let endpoint = guardian::Endpoint::start_with_io(
            program,
            args,
            env,
            &guardian::SpawnIo {
                cwd: home,
                descriptors: [
                    input_r.as_raw_fd(),
                    output_w.as_raw_fd(),
                    stderr.as_raw_fd(),
                ],
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            endpoint,
            input,
            output,
            buffer: vec![],
            clock_resolution_ns: 0,
            counter_birth: None,
            counter_epoch: Instant::now(),
            raw_directory: None,
            transport_finished: false,
        })
    }
    fn poll(fd: i32, events: i16, end: Instant) -> Result<()> {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("endpoint deadline; unknown outcome, no retry".into());
        }
        let mut poll = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, remaining.as_millis().clamp(1, 10) as i32) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    fn line(&mut self, end: Instant) -> Result<Value> {
        loop {
            if let Some(i) = self.buffer.iter().position(|b| *b == b'\n') {
                if i >= 64 * 1024 * 1024 {
                    return Err("endpoint response exceeds owned 64MiB limit".into());
                }
                let bytes = self.buffer.drain(..=i).collect::<Vec<_>>();
                return serde_json::from_slice(&bytes)
                    .map_err(|e| format!("endpoint response: {e}"));
            }
            if self.buffer.len() > 64 * 1024 * 1024 {
                return Err("endpoint response exceeds owned 64MiB limit".into());
            }
            let mut chunk = [0; 16384];
            match self.output.read(&mut chunk) {
                Ok(0) => return Err("endpoint closed transport; unknown outcome, no retry".into()),
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
            Self::poll(self.output.as_raw_fd(), libc::POLLIN, end)?;
        }
    }
    fn normal_exit(&mut self, end: Instant) -> Result<()> {
        let pid = self.endpoint.process.id();
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("shutdown deadline before normal child exit".into());
        }
        let status = self
            .endpoint
            .process
            .wait(remaining)
            .map_err(|e| e.to_string())?;
        if status != 0 {
            return Err(
                "owned endpoint did not exit normally after shutdown acknowledgement".into(),
            );
        }
        if !self.buffer.is_empty() {
            return Err("extra buffered stdout after shutdown acknowledgement".into());
        }
        loop {
            let mut extra = [0u8; 1];
            match self.output.read(&mut extra) {
                Ok(0) => break,
                Ok(_) => return Err("extra stdout after shutdown acknowledgement".into()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    Self::poll(self.output.as_raw_fd(), libc::POLLIN, end)?
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        self.transport_finished = true;
        self.preserve("shutdown-normal-exit.json",&json!({"pid":pid,"original_wait_status":status,"stdout_complete":true,"normal_exit_before_parent_cancel":true,"reaped_by":"exclusive Guardian Process owner"}))
    }
    /// Revoke the inherited lease before retiring the exclusive direct child.
    /// EOF proves closure of this transport's writers, not arbitrary descendant
    /// death. Separately grouped jobs still require the guarded spawn chain.
    fn cancel_and_drain(&mut self, budget: Duration) -> Result<Option<Value>> {
        if self.transport_finished {
            return Ok(None);
        }
        // Drop is a fallback only; an explicit failure must not be retried into
        // a successful cleanup receipt or a successful product measurement.
        self.transport_finished = true;
        let end = Instant::now() + budget;
        self.endpoint.cancel();
        let status = self.endpoint.process.terminate().map_err(|e| e.to_string())?;
        let mut discarded = self.buffer.len();
        self.buffer.clear();
        if discarded > 64 * 1024 * 1024 {
            return Err("cancelled endpoint stdout exceeds owned 64MiB limit".into());
        }
        loop {
            if Instant::now() >= end {
                return Err("cancelled endpoint stdout drain deadline".into());
            }
            let mut chunk = [0_u8; 16384];
            match self.output.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    discarded = discarded.checked_add(n).ok_or("cancelled endpoint stdout size overflow")?;
                    if discarded > 64 * 1024 * 1024 {
                        return Err("cancelled endpoint stdout exceeds owned 64MiB limit".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    Self::poll(self.output.as_raw_fd(), libc::POLLIN, end)?;
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(Some(json!({"direct_child_wait_status":status,"stdout_complete":true,
            "discarded_stdout_bytes":discarded,"descendant_termination_proven":false,
            "lease_revoked_before_direct_termination":true})))
    }
    fn send(&mut self, request: &Value, end: Instant) -> Result<()> {
        let mut bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("endpoint request exceeds owned 64MiB limit".into());
        }
        let mut offset = 0;
        while offset < bytes.len() {
            Self::poll(self.input.as_raw_fd(), libc::POLLOUT, end)?;
            match self.input.write(&bytes[offset..]) {
                Ok(0) => return Err("endpoint request closed; unknown outcome, no retry".into()),
                Ok(n) => offset += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => {
                    return Err(format!(
                        "endpoint request lost; unknown outcome, no retry: {e}"
                    ));
                }
            }
        }
        Ok(())
    }
    fn kernel_snapshot(&mut self) -> Result<Value> {
        let mut value =
            resources::snapshot_guardian(&self.endpoint.process, &mut self.counter_birth)?;
        value["monotonic_ns"] = json!(self.counter_epoch.elapsed().as_nanos());
        Ok(value)
    }
    fn preserve(&self, name: &str, value: &Value) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let directory = self
            .raw_directory
            .as_ref()
            .ok_or("resource capture requires source-owned collector evidence directory")?;
        let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(directory.join(name))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    }
    fn settle(&mut self, cycle: u64, point: u64, end: Instant) -> Result<Value> {
        if end.saturating_duration_since(Instant::now()) < Duration::from_secs(1) {
            return Err("lifecycle collection deadline before natural settling".into());
        }
        let mut values = Vec::new();
        for index in 0..5 {
            if index > 0 {
                std::thread::sleep(Duration::from_millis(250));
            }
            if Instant::now() >= end {
                return Err("lifecycle collection deadline during natural settling".into());
            }
            let probe = json!({"phase":"checkpoint-probe","lane":"lifecycle.10000","cycle":cycle,"completed_cycles":point,"index":index});
            self.send(&probe, end)?;
            let observed = self.line(end)?;
            Self::validate_resource_frame(
                &observed,
                &json!({"phase":"sample","lane":"lifecycle.10000","cycle":cycle}),
                "checkpoint-drained",
            )?;
            if observed["completed_cycles"] != point || observed["index"] != index {
                return Err("wrong or stale lifecycle drain observation".into());
            }
            let mut value = self.kernel_snapshot()?;
            value["pending_owners"] = observed["drain"]["pending_owners"].clone();
            value["drain"] = observed["drain"].clone();
            self.preserve(&format!("lifecycle-{cycle}-{point}-{index}.json"), &value)?;
            values.push(value);
        }
        let values = json!(values);
        natural_settle(
            &values,
            integer(
                values.as_array().unwrap().last().unwrap(),
                "physical_footprint_bytes",
            )?,
        )?;
        Ok(values)
    }
    fn validate_resource_frame(frame: &Value, request: &Value, phase: &str) -> Result<()> {
        ruycast_native::native_f1::ResourceFrame::parse(frame.clone())?;
        if frame["phase"] != phase
            || frame["lane"] != request["lane"]
            || frame["cycle"] != request["cycle"]
            || request["phase"] != "sample"
        {
            return Err("resource rendezvous identity mismatch".into());
        }
        let drain = &frame["drain"];
        if drain["pending_owners"] != 0
            || drain["unexpected_effects"] != json!([])
            || drain["fixture_coverage_complete"] != true
        {
            return Err("resource rendezvous is not actually drained/covered".into());
        }
        Ok(())
    }
    fn call(&mut self, request: &Value, budget: Duration) -> Result<Value> {
        let end = Instant::now() + budget;
        self.send(request, end)?;
        let mut response = self.line(end)?;
        let cycle = request["cycle"].as_u64().unwrap_or(0);
        let mut lifecycle = Vec::new();
        let mut cycle_timings = Vec::new();
        let mut cycle_outputs: Vec<ruycast_native::native_f1::LifecycleOutputRun> = Vec::new();
        let mut cycle_elapsed = 0_u64;
        let mut cycle_raw = if request["phase"] == "sample" && request["lane"] == "lifecycle.10000"
        {
            use std::os::unix::fs::OpenOptionsExt;
            let directory = self
                .raw_directory
                .as_ref()
                .ok_or("lifecycle transcript directory missing")?;
            Some(
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                    .open(directory.join(format!("lifecycle-{cycle}-cycles.jsonl")))
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let mut idle = None;
        while matches!(
            response["phase"].as_str(),
            Some("checkpoint" | "lifecycle-cycle" | "idle-ready")
        ) {
            if response["phase"] == "lifecycle-cycle" {
                Self::validate_resource_frame(&response, request, "lifecycle-cycle")?;
                if request["lane"] != "lifecycle.10000" || cycle_timings.len() >= 10000 {
                    return Err("extra lifecycle cycle or wrong lane".into());
                }
                let completed = cycle_timings.len() as u64;
                let expected_checkpoint_count = ruycast_native::native_f1::LIFECYCLE_POINTS
                    .iter()
                    .filter(|point| **point <= completed)
                    .count();
                if lifecycle.len() != expected_checkpoint_count
                    || response["inner_cycle"] != completed + 1
                {
                    return Err(
                        "lifecycle cycle missing, reordered, duplicated or before checkpoint"
                            .into(),
                    );
                }
                let elapsed = integer(&response, "elapsed_ns")?;
                if elapsed == 0 {
                    return Err("zero actual lifecycle cycle timing".into());
                }
                let raw = cycle_raw.as_mut().ok_or("unsolicited lifecycle cycle")?;
                serde_json::to_writer(&mut *raw, &response).map_err(|e| e.to_string())?;
                raw.write_all(b"\n").map_err(|e| e.to_string())?;
                cycle_elapsed = cycle_elapsed
                    .checked_add(elapsed)
                    .ok_or("lifecycle aggregate timer overflow")?;
                cycle_timings.push(elapsed);
                let output = response
                    .get("output")
                    .ok_or("lifecycle complete output missing")?
                    .clone();
                append_cycle_output(&mut cycle_outputs, output);
            } else if response["phase"] == "checkpoint" {
                Self::validate_resource_frame(&response, request, "checkpoint")?;
                if request["lane"] != "lifecycle.10000" {
                    return Err("checkpoint outside lifecycle lane".into());
                }
                let expected = ruycast_native::native_f1::LIFECYCLE_POINTS
                    .get(lifecycle.len())
                    .copied()
                    .ok_or("extra lifecycle checkpoint")?;
                if response["completed_cycles"] != expected
                    || cycle_timings.len() as u64 != expected
                {
                    return Err("lifecycle checkpoint missing, reordered or duplicated".into());
                }
                cycle_raw
                    .as_mut()
                    .ok_or("lifecycle transcript missing")?
                    .sync_all()
                    .map_err(|e| e.to_string())?;
                let observations = self.settle(cycle, expected, end)?;
                let continuing = json!({"phase":"checkpoint-continue","lane":request["lane"],"cycle":cycle,"completed_cycles":expected,"observations":observations});
                self.preserve(
                    &format!("lifecycle-{cycle}-{expected}-continue.json"),
                    &continuing,
                )?;
                self.send(&continuing, end)?;
                lifecycle.push(observations);
            } else {
                Self::validate_resource_frame(&response, request, "idle-ready")?;
                if request["lane"] != "idle.60seconds"
                    || idle.is_some()
                    || end.saturating_duration_since(Instant::now()) < Duration::from_secs(61)
                {
                    return Err("invalid repeated or deadline-bound idle rendezvous".into());
                }
                let before = self.kernel_snapshot()?;
                self.preserve(&format!("idle-{cycle}-before.json"), &before)?;
                std::thread::sleep(Duration::from_secs(60));
                if Instant::now() >= end {
                    return Err("idle collection deadline".into());
                }
                self.send(
                    &json!({"phase":"idle-probe","lane":"idle.60seconds","cycle":cycle}),
                    end,
                )?;
                let drained = self.line(end)?;
                Self::validate_resource_frame(&drained, request, "idle-drained")?;
                let after = self.kernel_snapshot()?;
                self.preserve(&format!("idle-{cycle}-after.json"), &after)?;
                let continuing = json!({"phase":"idle-continue","lane":request["lane"],"cycle":cycle,"before":before,"after":after});
                self.send(&continuing, end)?;
                idle = Some((before, after));
            }
            response = self.line(end)?;
        }
        if request["phase"] == "sample" && request["lane"] == "lifecycle.10000" {
            if lifecycle.len() != 4
                || cycle_timings.len() != 10000
                || response["elapsed_ns"] != cycle_elapsed
            {
                return Err("all four actual lifecycle resource checkpoints required".into());
            }
            cycle_raw
                .as_mut()
                .ok_or("lifecycle transcript missing")?
                .sync_all()
                .map_err(|e| e.to_string())?;
            let output = response
                .get_mut("output")
                .and_then(Value::as_object_mut)
                .ok_or("lifecycle final output object missing")?;
            if output.contains_key("cycles") || output.contains_key("cycle_output_encoding") {
                return Err("child cannot replace parent-preserved lifecycle outputs".into());
            }
            output.insert(
                "cycle_output_encoding".into(),
                json!(ruycast_native::native_f1::LIFECYCLE_OUTPUT_ENCODING),
            );
            output.insert(
                "cycles".into(),
                serde_json::to_value(cycle_outputs).map_err(|e| e.to_string())?,
            );
            let d = response
                .get_mut("diagnostics")
                .and_then(Value::as_object_mut)
                .ok_or("lifecycle diagnostics missing")?;
            for field in [
                "settled_start_physical_footprint_bytes",
                "settled_start_observations",
                "checkpoints",
                "cycle_elapsed_ns",
            ] {
                if d.contains_key(field) {
                    return Err(
                        "child-supplied physical footprint cannot replace parent kernel counters"
                            .into(),
                    );
                }
            }
            d.insert("cycle_elapsed_ns".into(), json!(cycle_timings));
            let last = |values: &Value| -> Result<u64> {
                integer(
                    values
                        .as_array()
                        .and_then(|a| a.last())
                        .ok_or("settling observations absent")?,
                    "physical_footprint_bytes",
                )
            };
            d.insert(
                "settled_start_physical_footprint_bytes".into(),
                json!(last(&lifecycle[0])?),
            );
            d.insert("settled_start_observations".into(), lifecycle[0].clone());
            let mut points = Vec::new();
            for (values, cycle) in lifecycle.iter().skip(1).zip([1000, 5000, 10000]) {
                points.push(json!({"cycle":cycle,"pending_owners":values.as_array().and_then(|a|a.last()).ok_or("settling evidence absent")?["pending_owners"],"physical_footprint_bytes":last(values)?,"natural_settle":true,"settling_observations":values}));
            }
            d.insert("checkpoints".into(), json!(points));
        }
        if request["phase"] == "sample" && request["lane"] == "idle.60seconds" {
            let (before, after) = idle.ok_or("actual parent idle interval required")?;
            let d = response
                .get_mut("diagnostics")
                .and_then(Value::as_object_mut)
                .ok_or("idle diagnostics missing")?;
            for field in [
                "before",
                "after",
                "interval_ns",
                "cpu_ns",
                "cpu_raw_mach_ticks",
                "interrupt_wakeups",
                "idle_wakeups",
                "disk_read_bytes",
                "disk_write_bytes",
                "pageins",
                "faults",
                "physical_footprint_bytes",
                "rss_bytes",
            ] {
                if d.contains_key(field) {
                    return Err("child-supplied idle counters refused".into());
                }
            }
            d.insert(
                "resource_source".into(),
                json!("owned child kernel counters"),
            );
            d.insert(
                "interval_ns".into(),
                json!(
                    integer(&after, "monotonic_ns")?
                        .checked_sub(integer(&before, "monotonic_ns")?)
                        .ok_or("idle clock went backwards")?
                ),
            );
            for key in [
                "cpu_ns",
                "interrupt_wakeups",
                "idle_wakeups",
                "disk_read_bytes",
                "disk_write_bytes",
                "pageins",
                "faults",
            ] {
                d.insert(
                    key.into(),
                    json!(
                        integer(&after, key)?
                            .checked_sub(integer(&before, key)?)
                            .ok_or("idle kernel counter went backwards")?
                    ),
                );
            }
            for key in ["physical_footprint_bytes", "rss_bytes"] {
                d.insert(key.into(), after[key].clone());
            }
            d.insert("before".into(), before);
            d.insert("after".into(), after);
        }
        Ok(response)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the guardian's sole life writer also revokes separately grouped
        // admitted descendants. Endpoint owns the only direct-child reaper.
        // No error swallowed here is ever promoted to positive evidence.
        // Normal collection and its error path perform this explicitly first.
        let _ = self.cancel_and_drain(Duration::from_secs(3));
    }
}

/// Actual transport contract on one pinned Rust fake, never product admission.
pub fn transport_selftest(fixture: &Path, pinned: &str, root: &Path) -> Result<()> {
    let checkout = super::preparation::checkout_root()?;
    let owned = checkout.as_path();
    if fixture != owned.join("rust-native/target/debug/f1-transport-fixture")
        || !root.starts_with(owned.join(".build/transport-evidence"))
        || root.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || std::env::var_os("F1_FORBIDDEN_SELFTEST").is_none()
    {
        return Err("exact own fake/proof paths and parent environment canary required".into());
    }
    native_file(
        &Binding {
            path: fixture.to_path_buf(),
            sha256: pinned.into(),
        },
        2,
    )?;
    fs::create_dir(root).map_err(|e| e.to_string())?;
    if fs::canonicalize(root).map_err(|e| e.to_string())? != root {
        return Err("owned transport proof path contains symlink".into());
    }
    let negative_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join("negative-stdio"))
        .map_err(|e| e.to_string())?;
    let mut negatives = vec![];
    for (name, cwd, descriptors) in [
        ("stdio-capability-alias", root.to_path_buf(), [0, 1, 2]),
        (
            "missing-owned-cwd",
            root.join("absent-directory"),
            [negative_file.as_raw_fd(); 3],
        ),
    ] {
        match guardian::Endpoint::start_with_io(
            fixture,
            &["echo".into()],
            &[],
            &guardian::SpawnIo {
                cwd: &cwd,
                descriptors,
            },
        ) {
            Ok(_) => return Err(format!("invalid own spawn {name} was admitted")),
            Err(error) => {
                negatives.push(json!({"case":name,"error":error.to_string(),"admitted":false}))
            }
        }
    }
    write_new(&root.join("spawn-negatives.json"), &negatives)?;
    let mut cases = vec![];
    for mode in [
        "echo",
        "eof",
        "malformed",
        "no-read",
        "partial",
        "escaped",
        "drop",
        "panic-drop",
    ] {
        let home = root.join(mode);
        fs::create_dir(&home).map_err(|e| e.to_string())?;
        let stderr = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(home.join("stderr"))
            .map_err(|e| e.to_string())?;
        let mut worker = Worker::spawn(
            fixture,
            &[mode.into()],
            &home,
            &[("HOME".into(), home.display().to_string())],
            &stderr,
        )?;
        if worker.line(Instant::now() + Duration::from_secs(3))?
            != json!({"phase":"owned-fake-ready"})
        {
            return Err("own fake readiness differs".into());
        }
        let parent_pid = worker.endpoint.process.id();
        let descendant = if matches!(mode, "escaped" | "partial" | "drop" | "panic-drop") {
            let marker = home.join("descendant.pid");
            let end = Instant::now() + Duration::from_secs(3);
            while !marker.is_file() {
                if Instant::now() >= end {
                    return Err("own escaped descendant marker absent".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let pid: i32 = fs::read_to_string(marker)
                .map_err(|e| e.to_string())?
                .parse::<i32>()
                .map_err(|e| e.to_string())?;
            if pid <= 1 || pid == parent_pid || unsafe { libc::getpgid(pid) } != pid {
                return Err("own descendant is not in a separate live process group".into());
            }
            use std::os::fd::{FromRawFd, OwnedFd};
            let queue = unsafe { libc::kqueue() };
            if queue < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let queue = unsafe { OwnedFd::from_raw_fd(queue) };
            if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let event = libc::kevent {
                ident: pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT | libc::EV_RECEIPT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let mut receipt: libc::kevent = unsafe { std::mem::zeroed() };
            let immediate = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            if unsafe { libc::kevent(queue.as_raw_fd(), &event, 1, &mut receipt, 1, &immediate) }
                != 1
                || receipt.flags & libc::EV_ERROR == 0
                || receipt.data != 0
            {
                return Err("own escaped descendant exit watch not installed".into());
            }
            Some((pid, queue))
        } else {
            None
        };
        let budget = Duration::from_millis(150);
        let request = if mode == "no-read" {
            json!({"payload":"x".repeat(2 * 1024 * 1024)})
        } else {
            json!({"text":"owned Ω\u{0000}text"})
        };
        let start = Instant::now();
        let reply = if matches!(mode, "escaped" | "drop" | "panic-drop") {
            Ok(Value::Null)
        } else {
            worker.call(&request, budget)
        };
        let elapsed = start.elapsed();
        let (result, detail) = match (mode, reply) {
            ("echo", Ok(reply))
                if reply
                    == json!({"reply":request,"cwd":home,"environment_cleared":true,"guard_alive":true}) =>
            {
                (
                    "passed",
                    "complete UTF8/NUL echo; owned cwd; environment cleared".into(),
                )
            }
            ("escaped" | "drop" | "panic-drop", Ok(_)) => (
                "passed",
                "separate admitted group cancelled through collector transport owner".into(),
            ),
            ("eof", Err(error))
                if error.contains("closed transport") || error.contains("request lost") =>
            {
                ("passed", error)
            }
            ("malformed", Err(error)) if error.contains("endpoint response") => ("passed", error),
            ("no-read" | "partial", Err(error))
                if error.contains("deadline") && elapsed < Duration::from_secs(3) =>
            {
                ("passed", error)
            }
            (_, result) => return Err(format!("own {mode} transport contract failed: {result:?}")),
        };
        let mut cleanup = Value::Null;
        let status = if mode == "panic-drop" {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _owned = worker;
                panic!("owned collector transport fault injection");
            }))
            .is_ok()
            {
                return Err("collector fault did not unwind".into());
            }
            None
        } else if mode == "drop" {
            drop(worker);
            None
        } else {
            cleanup = worker.cancel_and_drain(Duration::from_secs(3))?.ok_or("own cleanup absent")?;
            let status = cleanup["direct_child_wait_status"].as_i64().ok_or("own wait status absent")? as i32;
            drop(worker);
            Some(status)
        };
        let escaped_pid = if let Some((pid, queue)) = descendant {
            let mut observed: libc::kevent = unsafe { std::mem::zeroed() };
            let timeout = libc::timespec {
                tv_sec: 3,
                tv_nsec: 0,
            };
            if unsafe {
                libc::kevent(
                    queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    &mut observed,
                    1,
                    &timeout,
                )
            } != 1
                || observed.ident != pid as usize
                || observed.filter != libc::EVFILT_PROC
                || observed.fflags & libc::NOTE_EXIT == 0
            {
                return Err("own escaped descendant survived transport cancellation".into());
            }
            Some(pid)
        } else {
            None
        };
        cases.push(json!({"mode":mode,"result":result,"detail":detail,"elapsed_ns":elapsed.as_nanos(),"direct_child_pid":parent_pid,"direct_child_wait_status":status,"cleanup":cleanup,"escaped_descendant_pid":escaped_pid,"escaped_exit_observed":escaped_pid.is_some(),"descendant_reaped_by_collector":false,"no_retry":true}));
        write_new(&root.join(format!("{mode}.json")), cases.last().unwrap())?;
    }
    write_new(
        &root.join("RESULT.json"),
        &json!({"scope":"pinned-own-Rust-fake collector transport only","fixture_sha256":pinned,"cases":cases,"complete_sdk":false,"product_endpoint":false,"collection_enabled":false}),
    )
}

pub fn collect(authority_file: &Path, pinned: &str, out: &Path) -> Result<()> {
    let a = check_authority(authority_file, pinned)?;
    // Preparation snapshot: per-job supervision creates another PGID. This
    // transport must not run until descendant leases are integrated/reviewed.
    // A receipt cannot override a known missing implementation.
    const DESCENDANT_CONTAINMENT_VERIFIED: bool = false;
    if !DESCENDANT_CONTAINMENT_VERIFIED {
        return Err("collection disabled: reviewed descendant cancellation/drain/termination is not implemented; no endpoint started".into());
    }
    let adapters: Adapters = read(&a.collection_adapters.path)?;
    if adapters.schema != SCHEMA
        || adapters
            .endpoints
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != BTreeSet::from(["A", "B"])
        || adapters.pre_entry_review_sha256.len() != 64
    {
        return Err("two complete reviewed owned endpoints required".into());
    }
    fs::create_dir(out).map_err(|e| e.to_string())?;
    fs::write(out.join(".f1-owned"), pinned).map_err(|e| e.to_string())?;
    let mut raw = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("observations.ndjson"))
        .map_err(|e| e.to_string())?;
    let utc = || {
        format!(
            "unix-ns:{} UTC",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    };
    let started = utc();
    let mut workers = BTreeMap::new();
    let mut run = (|| -> Result<(Dataset, Value)> {
        for (role, spec) in &adapters.endpoints {
            workers.insert(
                role.clone(),
                Worker::start(spec, role, out, &a.fixture, pinned)?,
            );
        }
        fs::create_dir(out.join("outputs")).map_err(|e| e.to_string())?;
        let mut data = Dataset {
            schema: SCHEMA.into(),
            authority_sha256: pinned.into(),
            fixture_sha256: a.fixture.sha256.clone(),
            protocol_sha256: a.protocol.sha256.clone(),
            raw_observations: vec![],
            complete_outputs: BTreeMap::new(),
            warmup_completed: BTreeMap::new(),
            no_auto_retries: true,
            clock: "monotonic".into(),
            clock_resolution_ns: workers
                .values()
                .map(|w| w.clock_resolution_ns)
                .max()
                .unwrap(),
            run_started_utc: started,
            run_finished_utc: String::new(),
        };
        for lane in &protocol().lanes {
            let budget = Duration::from_secs(if lane.id.starts_with("lifecycle.") {
                1800
            } else if lane.id.starts_with("build.") {
                900
            } else if lane.id.starts_with("idle.") {
                90
            } else {
                30
            });
            for (arm, cycle, _) in schedule(lane.warmup_per_arm) {
                let response = workers.get_mut(arm).unwrap().call(
                    &json!({"phase":"warmup","lane":lane.id,"cycle":cycle}),
                    budget,
                )?;
                if response
                    != json!({"phase":"warmup-complete","lane":lane.id,"cycle":cycle,"pending_owners":0})
                {
                    return Err("warmup did not complete and drain".into());
                }
            }
            for arm in ["A", "B"] {
                data.warmup_completed
                    .insert(format!("{}:{arm}", lane.id), lane.warmup_per_arm);
            }
            for (arm, cycle, position) in schedule(lane.samples_per_arm) {
                let host_start = Instant::now();
                let response = workers.get_mut(arm).unwrap().call(
                    &json!({"phase":"sample","lane":lane.id,"cycle":cycle}),
                    budget,
                )?;
                if response.get("phase") != Some(&json!("sample-complete"))
                    || response.get("lane") != Some(&json!(lane.id))
                    || response.get("cycle") != Some(&json!(cycle))
                {
                    return Err("wrong endpoint response identity".into());
                }
                let output = response
                    .get("output")
                    .ok_or("complete ordered output absent")?
                    .clone();
                if response
                    .as_object()
                    .ok_or("sample envelope is not an object")?
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    != BTreeSet::from([
                        "phase",
                        "lane",
                        "cycle",
                        "elapsed_ns",
                        "output",
                        "diagnostics",
                    ])
                {
                    return Err("unknown or incomplete sample envelope fields".into());
                }
                let key = sha(&serde_json::to_vec(&output).map_err(|e| e.to_string())?);
                if data
                    .complete_outputs
                    .get(&key)
                    .is_some_and(|old| old != &output)
                {
                    return Err("output digest collision; full bytes differ".into());
                }
                if !data.complete_outputs.contains_key(&key) {
                    write_new(&out.join("outputs").join(format!("{key}.json")), &output)?;
                }
                data.complete_outputs.insert(key.clone(), output);
                let mut diagnostics = response
                    .get("diagnostics")
                    .ok_or("diagnostics absent")?
                    .clone();
                if !diagnostics.is_object() {
                    return Err("diagnostics envelope must be an object".into());
                }
                diagnostics["runner_transport_round_trip_ns"] =
                    json!(host_start.elapsed().as_nanos());
                let observation = Observation {
                    lane: lane.id.clone(),
                    cycle,
                    arm: arm.into(),
                    position,
                    elapsed_ns: integer(&response, "elapsed_ns")?,
                    output_key: key,
                    diagnostics,
                };
                validate_diagnostics(lane, &observation)?;
                serde_json::to_writer(&mut raw, &observation).map_err(|e| e.to_string())?;
                raw.write_all(b"\n").map_err(|e| e.to_string())?;
                raw.flush().map_err(|e| e.to_string())?;
                data.raw_observations.push(observation);
            }
        }
        for worker in workers.values_mut() {
            let shutdown_deadline = Instant::now() + Duration::from_secs(10);
            if worker.call(
                &json!({"phase":"shutdown"}),
                shutdown_deadline.saturating_duration_since(Instant::now()),
            )? != json!({"phase":"shutdown-complete","pending_owners":0})
            {
                return Err("endpoint could not drain shutdown".into());
            }
            worker.normal_exit(shutdown_deadline)?;
        }
        data.run_finished_utc = utc();
        // Source/artifact/fixture receipts are re-read after collection, before a report can pass.
        check_authority(authority_file, pinned)?;
        let report = compare(&a, pinned, &data)?;
        Ok((data, report))
    })();
    // Keep ownership outside the sampling closure so every rejection performs
    // explicit revocation, direct-child reaping and bounded transport drain.
    // A cleanup failure remains a rejection, never a measured success.
    for (role, worker) in &mut workers {
        let cleanup = worker.cancel_and_drain(Duration::from_secs(3));
        let record = match &cleanup {
            Ok(Some(value)) => value.clone(),
            Ok(None) => json!({"normal_shutdown_already_verified":true}),
            Err(error) => json!({"accepted":false,"error":error,"stdout_complete":false}),
        };
        if let Err(error) = write_new(&out.join(format!("{role}-cleanup.json")), &record) {
            let original = run.as_ref().err().map(String::as_str).unwrap_or("measurement cleanup rejected");
            run = Err(format!("{original}; cleanup evidence: {error}"));
        }
        if let Err(error) = cleanup {
            let original = run.as_ref().err().map(String::as_str).unwrap_or("measurement cleanup rejected");
            run = Err(format!("{original}; endpoint cleanup: {error}"));
        }
    }
    if let Err(e) = &run {
        write_new(
            &out.join("rejected.json"),
            &json!({"accepted":false,"reason":e,"no_retries":true,"partial_observations":"observations.ndjson; never a complete-product result"}),
        )?;
    }
    let (data, mut report) = run?;
    // Publish a successful dataset only after every explicit cleanup receipt.
    write_new(&out.join("dataset.json"), &data)?;
    report["ok"] = json!(true);
    report["source_dataset_sha256"] = json!(sha(
        &fs::read(out.join("dataset.json")).map_err(|e| e.to_string())?
    ));
    write_new(&out.join("comparison.json"), &report)
}

#[cfg(test)]
mod cycle_output_tests {
    use super::*;
    #[test]
    fn cancelled_owned_child_is_reaped_and_transport_drained_once() {
        let stderr = fs::OpenOptions::new().write(true).open("/dev/null").unwrap();
        let mut worker = Worker::spawn(Path::new("/bin/cat"), &[], Path::new("/"), &[], &stderr).unwrap();
        let request = json!({"owned":"collector cancellation control"});
        assert_eq!(worker.call(&request, Duration::from_secs(1)).unwrap(), request);
        let record = worker.cancel_and_drain(Duration::from_secs(3)).unwrap().unwrap();
        assert_eq!(record["stdout_complete"], true);
        assert!(record["direct_child_wait_status"].as_i64().is_some());
        assert_eq!(record["descendant_termination_proven"], false);
        // Reaping cleared Process's live identity. No second id()/signal is
        // permitted, and repeated cleanup must not panic or invent another wait.
        assert!(worker.cancel_and_drain(Duration::from_secs(1)).unwrap().is_none());
    }

    #[test]
    fn direct_child_exit_without_transport_eof_is_not_cleanup_success() {
        let stderr = fs::OpenOptions::new().write(true).open("/dev/null").unwrap();
        let mut worker = Worker::spawn(Path::new("/bin/cat"), &[], Path::new("/"), &[], &stderr).unwrap();
        let (read, held_writer) = guardian::pipe().unwrap();
        let flags = unsafe { libc::fcntl(read.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(unsafe { libc::fcntl(read.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
        // A deliberately retained own writer models the missing EOF conjunct;
        // it is not asserted to be an admitted process or descendant witness.
        worker.output = read.into();
        let began = Instant::now();
        let error = worker.cancel_and_drain(Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("deadline"));
        assert!(began.elapsed() < Duration::from_secs(2));
        drop(held_writer);
    }

    #[test]
    fn lossless_runs_preserve_every_changed_field_and_nonadjacent_repeat() {
        let a = json!({"rows":["é","a\0b"],"selected":0,"enabled":true});
        let b = json!({"rows":["e\u{301}","a\0b"],"selected":0,"enabled":true});
        let c = json!({"rows":["é","a\0b"],"selected":1,"enabled":true});
        let input = vec![a.clone(), a.clone(), b, c, a];
        let mut runs = Vec::new();
        for value in &input {
            append_cycle_output(&mut runs, value.clone());
        }
        assert_eq!(
            runs.iter().map(|r| r.repetitions).collect::<Vec<_>>(),
            vec![2, 1, 1, 1]
        );
        let expanded = runs
            .iter()
            .flat_map(|r| std::iter::repeat_n(r.output.clone(), r.repetitions as usize))
            .collect::<Vec<_>>();
        assert_eq!(expanded, input);
    }
}
