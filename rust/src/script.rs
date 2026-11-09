//! Shared synchronous script primitive. Call from a bounded execution worker,
//! never the UI or the serialized configuration owner. No shell interpolation.
use crate::Command;
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command as Process, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub const OUTPUT_LIMIT: usize = 65_536;

#[derive(Debug)]
pub enum End {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
    Deadline,
}

/// Internal supervisor metadata, never merged into the legacy CLI response.
pub const SUPERVISOR_VERSION: u64 = 3;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamCapture {
    pub truncated: bool,
    /// Observed read(0), or a logically closed/nonexistent pre-launch stream.
    pub eof: bool,
    /// Validity of the retained raw prefix, not the discarded/full stream.
    pub valid_utf8: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capture {
    pub stdout: StreamCapture,
    pub stderr: StreamCapture,
}
impl Capture {
    pub fn empty() -> Self {
        Self::from_raw(&[], &[], false, false, true, true)
    }
    fn from_raw(
        stdout: &[u8],
        stderr: &[u8],
        out_truncated: bool,
        err_truncated: bool,
        out_eof: bool,
        err_eof: bool,
    ) -> Self {
        Self {
            stdout: StreamCapture {
                truncated: out_truncated,
                eof: out_eof,
                valid_utf8: std::str::from_utf8(stdout).is_ok(),
            },
            stderr: StreamCapture {
                truncated: err_truncated,
                eof: err_eof,
                valid_utf8: std::str::from_utf8(stderr).is_ok(),
            },
        }
    }
}

#[derive(Debug)]
pub struct Completion {
    pub end: End,
    pub stdout: String,
    pub stderr: String,
    pub capture: Capture,
    timeout: f64,
}

impl Completion {
    /// Preserve the CLI's existing script response shape. Cancellation is an
    /// unknown-effect outcome: callers must not automatically retry a script.
    pub fn response(&self) -> Value {
        match self.end {
            End::TimedOut => {
                json!({"ok":false,"error":format!("Script timed out after {} seconds.",self.timeout)})
            }
            End::Cancelled => {
                json!({"ok":false,"error":"Script cancelled; execution may have already occurred."})
            }
            End::Deadline => {
                json!({"ok":false,"error":"Script request deadline expired; execution may have already occurred."})
            }
            End::Exited(status) if status.success() => {
                json!({"ok":true,"result":{"exitCode":0,"stdout":self.stdout,"stderr":self.stderr}})
            }
            End::Exited(status) => {
                json!({"ok":false,"error":format!("Script exited with status {}.",status.code().unwrap_or(-1)),"exitCode":status.code().unwrap_or(-1),"stdout":self.stdout,"stderr":self.stderr})
            }
        }
    }
}

/// Keeps the leader unreaped until the final process-group signal. This pins
/// its PID/PGID and prevents signalling a recycled group after a quick exit.
struct OwnedProcess {
    child: Child,
    armed: bool,
}

impl OwnedProcess {
    fn signal(&self, signal: i32) -> io::Result<()> {
        if unsafe { libc::kill(-(self.child.id() as i32), signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH)
            || (error.raw_os_error() == Some(libc::EPERM) && no_live_group_members(self.child.id()))
        {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn exited(&mut self) -> io::Result<bool> {
        // WNOWAIT observes termination without allowing PID reuse. The host
        // must not independently reap this runner's children.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let code = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if code == 0 {
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        if error.raw_os_error() == Some(libc::ECHILD) {
            // Ownership was violated by an external reaper. Fail closed;
            // neither the old PID nor its group is safe to signal now.
            self.armed = false;
        }
        Err(error)
    }

    fn reap(&mut self) -> io::Result<ExitStatus> {
        self.signal(libc::SIGKILL)?;
        match self.child.wait() {
            Ok(status) => {
                self.armed = false;
                Ok(status)
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.armed = false;
                }
                Err(error)
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn no_live_group_members(leader: u32) -> bool {
    // Darwin returns EPERM for signalling a zombie-only group. Do not ignore
    // EPERM for a live inaccessible member: enumerate the held group and fail
    // closed on truncation, permission errors or an unknown membership set.
    let mut exit: libc::siginfo_t = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::waitid(
            libc::P_PID,
            leader as libc::id_t,
            &mut exit,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
        || unsafe { exit.si_pid() } != leader as i32
    {
        return false;
    }
    let mut pids = [0i32; 4096];
    unsafe {
        *libc::__error() = 0;
    }
    let bytes = unsafe {
        libc::proc_listpids(
            2,
            leader,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(&pids) as i32,
        )
    };
    if bytes <= 0
        || bytes as usize >= std::mem::size_of_val(&pids)
        || !(bytes as usize).is_multiple_of(std::mem::size_of::<i32>())
    {
        return false;
    }
    let pids = &pids[..bytes as usize / std::mem::size_of::<i32>()];
    if !pids.contains(&(leader as i32)) {
        return false;
    }
    for &pid in pids.iter().filter(|&&pid| pid > 0) {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        unsafe {
            *libc::__error() = 0;
        }
        let result = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if result == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            // proc_listpids includes zombies; proc_pidinfo cannot inspect them.
            continue;
        }
        if result != size || (info.pbi_pgid == leader && info.pbi_status != 5) {
            return false;
        }
    }
    true
}

#[cfg(not(target_os = "macos"))]
fn no_live_group_members(_: u32) -> bool {
    false
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if self.armed {
            // Also cover failures before the first regular exit observation.
            let _ = self.exited();
            if !self.armed {
                return;
            }
            let _ = self.signal(libc::SIGKILL);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Wait only for existing pipe readiness, with a short cancellation quantum.
/// The caller retains its one absolute deadline and checks cancellation again
/// after every return, including EINTR. Readiness never implies data or EOF.
pub fn wait_pipe_readiness(
    descriptors: &mut [libc::pollfd],
    deadline: Option<Instant>,
) -> io::Result<()> {
    if !(1..=2).contains(&descriptors.len())
        || descriptors.iter().any(|descriptor| descriptor.fd < -1)
    {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let quantum = Duration::from_millis(5);
    let wait = deadline
        .map(|end| end.saturating_duration_since(Instant::now()).min(quantum))
        .unwrap_or(quantum);
    let timeout = wait.as_nanos().div_ceil(1_000_000) as i32;
    let count = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout,
        )
    };
    if count < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(())
        } else {
            Err(error)
        };
    }
    if descriptors.iter().any(|descriptor| descriptor.revents & libc::POLLNVAL != 0) {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    Ok(())
}

fn drain(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    truncated: &mut bool,
    eof: &mut bool,
) -> io::Result<()> {
    if *eof {
        return Ok(());
    }
    let mut buffer = [0; 16_384];
    // Bound each stream's turn even if a writer continuously produces data.
    for _ in 0..8 {
        match reader.read(&mut buffer) {
            Ok(0) => {
                *eof = true;
                break;
            }
            Ok(count) => {
                let retained = count.min(OUTPUT_LIMIT.saturating_sub(output.len()));
                *truncated |= retained < count;
                output.extend_from_slice(&buffer[..retained]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// `deadline` is absolute and also checked before launch. `cancelled` must be
/// fast and nonblocking. Cooperative cancellation has an unavoidable race with
/// spawn; a cancelled result never promises the script had no side effects.
pub fn run(
    command: &Command,
    home: &Path,
    deadline: Option<Instant>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Completion, String> {
    let timeout = command.timeout.unwrap_or(30.);
    if command.kind != "script" {
        return Err("The script runner requires a script command.".into());
    }
    if (!command.target.starts_with('/') && !command.target.starts_with("~/"))
        || !(0.1..=300.).contains(&timeout)
    {
        return Err("Script needs an absolute executable path and timeout 0.1–300 seconds.".into());
    }
    let interrupt = |cancelled: &mut dyn FnMut() -> bool| {
        if cancelled() {
            Some(End::Cancelled)
        } else if deadline.is_some_and(|d| Instant::now() >= d) {
            Some(End::Deadline)
        } else {
            None
        }
    };
    if let Some(end) = interrupt(&mut cancelled) {
        return Ok(Completion {
            end,
            stdout: String::new(),
            stderr: String::new(),
            capture: Capture::empty(),
            timeout,
        });
    }
    let path = command
        .target
        .strip_prefix("~/")
        .map(|tail| home.join(tail))
        .unwrap_or_else(|| PathBuf::from(&command.target));
    let mut launcher = Process::new(path);
    // Use the fork/exec error-pipe path so a failed exec is reaped by std.
    // The child hook performs no allocation, locks or non-async-safe calls.
    unsafe {
        launcher.pre_exec(|| Ok(()));
    }
    let child = launcher
        .args(&command.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut process = OwnedProcess { child, armed: true };
    let mut out = process
        .child
        .stdout
        .take()
        .ok_or("Script stdout unavailable")?;
    let mut err = process
        .child
        .stderr
        .take()
        .ok_or("Script stderr unavailable")?;
    nonblocking(out.as_raw_fd()).map_err(|e| e.to_string())?;
    nonblocking(err.as_raw_fd()).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let command_deadline = started + Duration::from_secs_f64(timeout);
    let mut ending: Option<Instant> = None;
    let mut kill_sent = false;
    let mut end = None;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut out_truncated = false;
    let mut err_truncated = false;
    let mut out_eof = false;
    let mut err_eof = false;
    loop {
        drain(&mut out, &mut stdout, &mut out_truncated, &mut out_eof)
            .map_err(|e| e.to_string())?;
        drain(&mut err, &mut stderr, &mut err_truncated, &mut err_eof)
            .map_err(|e| e.to_string())?;
        if ending.is_none() {
            end = interrupt(&mut cancelled);
            let exited = process.exited().map_err(|e| e.to_string())?;
            // A leader already finished within this observation is an exit,
            // even if the next poll runs just beyond its timeout boundary.
            if end.is_none() && !exited && started.elapsed().as_secs_f64() >= timeout {
                end = Some(End::TimedOut);
            }
            if end.is_some() || exited {
                ending = Some(Instant::now());
                process.signal(libc::SIGTERM).map_err(|e| e.to_string())?;
            }
        }
        if let Some(ending) = ending {
            if !kill_sent && ending.elapsed() >= Duration::from_millis(100) {
                process.signal(libc::SIGKILL).map_err(|e| e.to_string())?;
                kill_sent = true;
            }
            if ending.elapsed() >= Duration::from_millis(150) {
                break;
            }
        }
        let next_check = ending.map_or_else(
            || deadline.map_or(command_deadline, |end| end.min(command_deadline)),
            |end| end + Duration::from_millis(150),
        );
        // Both streams retain their existing bounded drain turn. A ready pipe
        // immediately starts the next turn; EOF descriptors cannot spin on HUP.
        wait_pipe_readiness(
            &mut [
                libc::pollfd {
                    fd: if out_eof { -1 } else { out.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if err_eof { -1 } else { err.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ],
            Some(next_check),
        )
        .map_err(|error| error.to_string())?;
    }
    let status = process.reap().map_err(|e| e.to_string())?;
    drain(&mut out, &mut stdout, &mut out_truncated, &mut out_eof).map_err(|e| e.to_string())?;
    drain(&mut err, &mut stderr, &mut err_truncated, &mut err_eof).map_err(|e| e.to_string())?;
    Ok(Completion {
        end: end.unwrap_or(End::Exited(status)),
        capture: Capture::from_raw(
            &stdout,
            &stderr,
            out_truncated,
            err_truncated,
            out_eof,
            err_eof,
        ),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timeout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn capture_records_actual_discard_and_retained_utf8_independently() {
        for size in [OUTPUT_LIMIT, OUTPUT_LIMIT + 1] {
            let mut out = Vec::new();
            let mut truncated = false;
            let mut eof = false;
            drain(
                &mut std::io::Cursor::new(vec![b'a'; size]),
                &mut out,
                &mut truncated,
                &mut eof,
            )
            .unwrap();
            assert_eq!(out.len(), OUTPUT_LIMIT);
            assert_eq!(truncated, size > OUTPUT_LIMIT);
            assert!(eof);
        }
        let c = Capture::from_raw(&[0xc3], &[0xff], true, false, true, false);
        assert!(c.stdout.truncated && !c.stdout.valid_utf8 && c.stdout.eof);
        assert!(!c.stderr.truncated && !c.stderr.valid_utf8 && !c.stderr.eof);
        let c = Capture::from_raw("Ω".as_bytes(), b"plain", false, true, false, true);
        assert!(!c.stdout.truncated && c.stdout.valid_utf8 && !c.stdout.eof);
        assert!(c.stderr.truncated && c.stderr.valid_utf8 && c.stderr.eof);
        assert_eq!(
            Capture::empty(),
            Capture::from_raw(b"", b"", false, false, true, true)
        );
    }
    #[test]
    fn would_block_and_turn_budget_never_invent_eof() {
        struct Busy;
        impl Read for Busy {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        let mut out = vec![];
        let mut truncated = false;
        let mut eof = false;
        drain(&mut Busy, &mut out, &mut truncated, &mut eof).unwrap();
        assert!(!eof && !truncated);
        drain(
            &mut std::io::Cursor::new(vec![b'a'; 16_384 * 9]),
            &mut out,
            &mut truncated,
            &mut eof,
        )
        .unwrap();
        assert!(!eof && truncated);
    }
    #[test]
    fn invalid_values_never_reach_launch_or_cancellation() {
        let mut command: Command = serde_json::from_value(json!({
            "id":"fixture", "title":"Fixture", "kind":"script", "target":"/bin/true"
        }))
        .unwrap();
        for timeout in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.,
            0.,
            0.099,
            300.1,
        ] {
            command.timeout = Some(timeout);
            assert!(run(&command, Path::new("/tmp"), None, || panic!(
                "invalid command passed validation"
            ))
            .is_err());
        }
    }
}
