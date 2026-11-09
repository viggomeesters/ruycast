//! Bounded tool subprocesses. Own process groups are retired and the direct
//! child is waited on every outcome; pipe reads never create stuck threads.
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
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
}
impl<T: Read + AsRawFd> Pipe<T> {
    fn new(reader: T, log: Option<std::fs::File>) -> Result<Self, String> {
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
        })
    }
    fn drain(&mut self, limit: usize) -> Result<(), String> {
        if self.eof {
            return Ok(());
        }
        let mut buffer = [0u8; 8192];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(count) => {
                    if let Some(log) = self.log.as_mut() {
                        use std::io::Write;
                        log.write_all(&buffer[..count])
                            .map_err(|error| error.to_string())?;
                    }
                    if self
                        .bytes
                        .len()
                        .checked_add(count)
                        .is_none_or(|n| n > limit)
                    {
                        return Err("Tool output exceeds its bounded size.".into());
                    }
                    self.bytes.extend_from_slice(&buffer[..count]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
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
    mut command: Command,
    timeout: Duration,
    limit: usize,
    logs: Option<(std::fs::File, std::fs::File)>,
) -> Result<Captured, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = Owned {
        child: command.spawn().map_err(|e| e.to_string())?,
        group_retired: false,
    };
    let (out, err) = logs.map_or((None, None), |(out, err)| (Some(out), Some(err)));
    let mut stdout = Pipe::new(
        child
            .child
            .stdout
            .take()
            .ok_or("Tool stdout unavailable.")?,
        out,
    )?;
    let mut stderr = Pipe::new(
        child
            .child
            .stderr
            .take()
            .ok_or("Tool stderr unavailable.")?,
        err,
    )?;
    let end = Instant::now() + timeout;
    let mut status = None;
    loop {
        stdout.drain(limit)?;
        stderr.drain(limit)?;
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
