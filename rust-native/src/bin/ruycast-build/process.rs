//! Bounded tool subprocesses. Own process groups are retired and the direct
//! child is waited on every outcome; pipe reads never create stuck threads.
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
thread_local! {static OUTER_DEADLINE:std::cell::Cell<Option<u64>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
pub(super) fn with_deadline<T>(
    deadline: u64,
    run: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    super::parity_bootstrap::inner_budget(deadline)?;
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OUTER_DEADLINE.with(|value| value.set(self.0));
        }
    }
    let prior = OUTER_DEADLINE
        .with(|value| value.replace(Some(value.get().map_or(deadline, |old| old.min(deadline)))));
    let _restore = Restore(prior);
    run()
}
pub(super) struct Captured {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
}
struct Owned {
    child: Child,
    group_retired: bool,
}
impl Owned {
    fn retire(&mut self) {
        if !self.group_retired {
            // The direct child is still unreaped here. Its PID cannot be
            // recycled into an unrelated process group before this signal.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            self.group_retired = true;
        }
    }
    fn exited_unreaped(&self) -> Result<bool, String> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error.to_string());
        }
        Ok(info.si_pid == self.child.id() as i32)
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.retire();
        let _ = self.child.wait();
    }
}
struct Pipe<T> {
    reader: T,
    bytes: Vec<u8>,
    eof: bool,
    log: Option<std::fs::File>,
    count: usize,
    retain: bool,
}
impl<T: Read + AsRawFd> Pipe<T> {
    fn new(reader: T, log: Option<std::fs::File>, retain: bool) -> Result<Self, String> {
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self {
            reader,
            bytes: vec![],
            eof: false,
            log,
            count: 0,
            retain,
        })
    }
    fn drain(&mut self, limit: usize, end: Instant) -> Result<(), String> {
        if self.eof {
            return Ok(());
        }
        let mut buffer = [0u8; 8192];
        // Yield between bounded batches even when the producer never reaches
        // WouldBlock, and enforce the deadline inside each draining batch.
        for _ in 0..16 {
            if Instant::now() >= end {
                return Err("Tool process timed out; owned process group was retired.".into());
            }
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(count) => {
                    let count = self
                        .count
                        .checked_add(count)
                        .ok_or("Tool output exceeds its bounded size.")?;
                    if count > limit {
                        return Err("Tool output exceeds its bounded size.".into());
                    }
                    let added = count - self.count;
                    if let Some(log) = self.log.as_mut() {
                        use std::io::Write;
                        log.write_all(&buffer[..added]).map_err(|e| e.to_string())?;
                    }
                    if self.retain {
                        self.bytes.extend_from_slice(&buffer[..added]);
                    }
                    self.count = count;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
}
pub(super) fn capture(
    command: Command,
    timeout: Duration,
    limit: usize,
) -> Result<Captured, String> {
    capture_logged(command, timeout, limit, None)
}
pub(super) fn capture_logged(
    command: Command,
    timeout: Duration,
    limit: usize,
    logs: Option<(std::fs::File, std::fs::File)>,
) -> Result<Captured, String> {
    let (out, err) = logs.map_or((None, None), |(out, err)| (Some(out), Some(err)));
    capture_stream(
        command,
        timeout,
        Capture {
            input: Stdio::null(),
            out,
            err,
            out_limit: limit,
            err_limit: limit,
            retain_out: true,
        },
    )
}
pub(super) fn capture_file_input(
    command: Command,
    input: std::fs::File,
    timeout: Duration,
    limit: usize,
) -> Result<Captured, String> {
    capture_stream(
        command,
        timeout,
        Capture {
            input: Stdio::from(input),
            out: None,
            err: None,
            out_limit: limit,
            err_limit: limit,
            retain_out: true,
        },
    )
}
pub(super) fn capture_stdout_file(
    command: Command,
    output: std::fs::File,
    timeout: Duration,
    out_limit: usize,
    err_limit: usize,
) -> Result<Captured, String> {
    capture_stream(
        command,
        timeout,
        Capture {
            input: Stdio::null(),
            out: Some(output),
            err: None,
            out_limit,
            err_limit,
            retain_out: false,
        },
    )
}
struct Capture {
    input: Stdio,
    out: Option<std::fs::File>,
    err: Option<std::fs::File>,
    out_limit: usize,
    err_limit: usize,
    retain_out: bool,
}
fn capture_stream(
    mut command: Command,
    timeout: Duration,
    settings: Capture,
) -> Result<Captured, String> {
    let timeout = OUTER_DEADLINE.with(|value| match value.get() {
        Some(deadline) => {
            super::parity_bootstrap::inner_budget(deadline).map(|remaining| timeout.min(remaining))
        }
        None => Ok(timeout),
    })?;
    let Capture {
        input,
        out,
        err,
        out_limit,
        err_limit,
        retain_out,
    } = settings;
    command
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = Owned {
        child: command.spawn().map_err(|e| e.to_string())?,
        group_retired: false,
    };
    let mut stdout = Pipe::new(
        child
            .child
            .stdout
            .take()
            .ok_or("Tool stdout unavailable.")?,
        out,
        retain_out,
    )?;
    let mut stderr = Pipe::new(
        child
            .child
            .stderr
            .take()
            .ok_or("Tool stderr unavailable.")?,
        err,
        true,
    )?;
    let end = Instant::now() + timeout;
    let mut status = None;
    loop {
        stdout.drain(out_limit, end)?;
        stderr.drain(err_limit, end)?;
        if status.is_none() && child.exited_unreaped()? {
            child.retire();
            status = Some(child.child.wait().map_err(|e| e.to_string())?);
        }
        if let Some(status) = status {
            if stdout.eof && stderr.eof {
                return Ok(Captured {
                    stdout: stdout.bytes,
                    stderr: stderr.bytes,
                    status,
                });
            }
        }
        if Instant::now() >= end {
            return Err("Tool process timed out; owned process group was retired.".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Internal owned Rust subprocess, explicitly executed by capture tests"]
    fn owned_entry() {
        match std::env::var("RUYCAST_BUILD_OWNED_PROCESS")
            .unwrap()
            .as_str()
        {
            "streams" => {
                println!("owned stdout");
                eprintln!("owned stderr");
                std::process::exit(7);
            }
            "continuous" => {
                use std::io::Write;
                let mut output = std::io::stdout().lock();
                let bytes = [b'x'; 8192];
                loop {
                    output.write_all(&bytes).unwrap();
                }
            }
            "stdin" => {
                let mut data = String::new();
                std::io::stdin().read_to_string(&mut data).unwrap();
                println!("{data}");
            }
            "hold" => std::thread::sleep(Duration::from_secs(5)),
            "flood" => {
                use std::io::Write;
                std::io::stdout().write_all(&vec![b'x'; 100000]).unwrap();
            }
            "exited-leader-descendant" => {
                use std::os::fd::AsRawFd;
                let path = std::env::var("RUYCAST_BUILD_OWNED_LATE_WRITE").unwrap();
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .unwrap();
                let descriptor = file.as_raw_fd();
                let pid = unsafe { libc::fork() };
                assert!(pid >= 0);
                if pid == 0 {
                    // Only async-signal-safe libc calls in the fork child.
                    unsafe {
                        libc::signal(libc::SIGTERM, libc::SIG_IGN);
                        libc::usleep(500_000);
                        libc::write(descriptor, b"late write".as_ptr().cast(), 10);
                        libc::_exit(0);
                    }
                }
                println!("owned descendant {pid}");
                std::process::exit(0);
            }
            _ => panic!("unknown owned mode"),
        }
    }
    #[test]
    fn scoped_deadline_caps_child_and_restores_after_error() {
        let now = super::super::parity_bootstrap::monotonic_nanos().unwrap();
        let start = Instant::now();
        let result = with_deadline(now + 5_100_000_000, || {
            capture(command("hold"), Duration::from_secs(5), 65536)
        });
        assert!(result.err().unwrap().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(OUTER_DEADLINE.with(|value| value.get()).is_none());
        assert!(capture(command("streams"), Duration::from_secs(5), 65536).is_ok());
    }
    #[test]
    fn streaming_output_is_capped_without_memory_retention() {
        let fixture = super::super::tests::Fixture::new();
        let path = fixture.0.join("stream.data");
        let result = capture_stdout_file(
            command("streams"),
            std::fs::File::create(&path).unwrap(),
            Duration::from_secs(5),
            65536,
            65536,
        )
        .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert!(result.stdout.is_empty());
        assert!(std::fs::read(&path)
            .unwrap()
            .windows(12)
            .any(|w| w == b"owned stdout"));
        let result = capture_stdout_file(
            command("continuous"),
            std::fs::File::create(&path).unwrap(),
            Duration::from_secs(5),
            32768,
            65536,
        );
        assert!(result.err().unwrap().contains("output"));
        assert!(std::fs::metadata(&path).unwrap().len() <= 32768);
        let start = Instant::now();
        let result = capture_stdout_file(
            command("continuous"),
            std::fs::File::create(&path).unwrap(),
            Duration::from_millis(80),
            64 * 1024 * 1024,
            65536,
        );
        assert!(result.err().unwrap().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn continuous_output_cannot_bypass_deadline_or_cap() {
        let start = Instant::now();
        let result = capture(
            command("continuous"),
            Duration::from_millis(80),
            64 * 1024 * 1024,
        );
        assert!(result.err().unwrap().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let result = capture(command("continuous"), Duration::from_secs(5), 32 * 1024);
        assert!(result.err().unwrap().contains("output"));
    }
    #[test]
    fn file_input_uses_bounded_owned_capture() {
        let fixture = super::super::tests::Fixture::new();
        let path = fixture.0.join("input.data");
        std::fs::write(&path, b"owned input bytes").unwrap();
        let result = capture_file_input(
            command("stdin"),
            std::fs::File::open(path).unwrap(),
            Duration::from_secs(5),
            65536,
        )
        .unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8(result.stdout)
            .unwrap()
            .contains("owned input bytes"));
    }
    fn command(mode: &str) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--ignored",
            "--exact",
            "process::tests::owned_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env("RUYCAST_BUILD_OWNED_PROCESS", mode);
        c
    }
    #[test]
    fn actual_exit_and_complete_streams_are_not_fabricated() {
        let result = capture(command("streams"), Duration::from_secs(5), 65536).unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert!(result.stdout.windows(12).any(|w| w == b"owned stdout"));
        assert!(result.stderr.windows(12).any(|w| w == b"owned stderr"));
    }
    #[test]
    fn exited_leader_cannot_leave_a_pipe_holder_or_late_writing_descendant() {
        let fixture = super::super::tests::Fixture::new();
        let sentinel = fixture.0.join("owned-late-write");
        let mut child = command("exited-leader-descendant");
        child.env("RUYCAST_BUILD_OWNED_LATE_WRITE", &sentinel);
        let captured = capture(child, Duration::from_millis(250), 65536)
            .expect("leader is terminal; owned descendants must be retired before waiting for EOF");
        assert_eq!(captured.status.code(), Some(0));
        assert!(String::from_utf8(captured.stdout)
            .unwrap()
            .contains("owned descendant "));
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"");
    }
    #[test]
    fn owned_timeout_and_output_overflow_are_failures() {
        assert!(capture(command("hold"), Duration::from_millis(80), 65536)
            .err()
            .unwrap()
            .contains("timed out"));
        assert!(capture(command("flood"), Duration::from_secs(5), 2048)
            .err()
            .unwrap()
            .contains("output"));
    }
}
