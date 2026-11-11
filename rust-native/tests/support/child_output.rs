//! Bounded output capture for already-spawned direct CLI children in owned tests.
use std::{process::{Child, Output}, time::{Duration, Instant}};
pub fn wait(child: Child, timeout: Duration) -> Output {
    wait_with_limit(child, timeout, 1024 * 1024)
}
/// An explicit bound for genuine large-output fixtures; ordinary callers keep 1 MiB.
pub fn wait_with_limit(mut child: Child, timeout: Duration, maximum_bytes: usize) -> Output {
    use std::io::{Error, ErrorKind, Read};
    use std::os::fd::AsRawFd;
    let until = Instant::now() + timeout;
    let result = (|| -> std::io::Result<Output> {
        let mut stdout = child.stdout.take().ok_or_else(|| Error::other("CLI stdout unavailable"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| Error::other("CLI stderr unavailable"))?;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(Error::last_os_error());
            }
        }
        let mut bytes = [Vec::new(), Vec::new()];
        let mut eof = [false, false];
        let mut status = None;
        loop {
            for (index, pipe) in [&mut stdout as &mut dyn Read, &mut stderr as &mut dyn Read].into_iter().enumerate() {
                if eof[index] { continue; }
                // Drain both bounded streams while the producer can still run.
                for _ in 0..8 {
                    let mut buffer = [0u8; 8192];
                    match pipe.read(&mut buffer) {
                        Ok(0) => { eof[index] = true; break; }
                        Ok(count) => {
                            if bytes[index].len() + count > maximum_bytes {
                                return Err(Error::other("Owned CLI output exceeds its explicit per-stream limit"));
                            }
                            bytes[index].extend_from_slice(&buffer[..count]);
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    }
                }
            }
            if status.is_none() { status = child.try_wait()?; }
            if let Some(status) = status.filter(|_| eof[0] && eof[1]) {
                return Ok(Output { status, stdout: std::mem::take(&mut bytes[0]), stderr: std::mem::take(&mut bytes[1]) });
            }
            if Instant::now() >= until {
                return Err(Error::new(ErrorKind::TimedOut, "owned CLI timeout"));
            }
            rustcast_core::script::wait_pipe_readiness(&mut [
                libc::pollfd { fd: if eof[0] { -1 } else { stdout.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: if eof[1] { -1 } else { stderr.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
            ], Some(until))?;
        }
    })();
    if result.is_err() {
        // Signal only a still-owned, unreaped child, then always reap it.
        if matches!(child.try_wait(), Ok(None)) { let _ = child.kill(); }
        let _ = child.wait();
    }
    result.expect("bounded owned CLI capture")
}
