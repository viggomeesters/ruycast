//! Bounded read-only OS CLI collection, with no shell and no retained stderr.
use crate::system_search::Ticket;
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn stream(
    executable: &str,
    args: &[&str],
    ticket: &Ticket,
    limit: usize,
    truncate: bool,
    output: impl FnMut(&[u8]) -> bool,
) -> Result<(), String> {
    stream_inner(executable, args, ticket, limit, truncate, None, output)
}
/// Spotlight's private collection stop preserves complete records already read,
/// including on a nonzero child exit. Caller expiry/cancellation is still hard.
pub(crate) fn file_query(
    executable: &str,
    args: &[&str],
    ticket: &Ticket,
    output: impl FnMut(&[u8]) -> bool,
) -> Result<(), String> {
    stream_inner(executable, args, ticket, 1_048_576, true, Some(Duration::from_secs(3)), output)
}
fn stream_inner(
    executable: &str,
    args: &[&str],
    ticket: &Ticket,
    limit: usize,
    truncate: bool,
    collection_budget: Option<Duration>,
    mut output: impl FnMut(&[u8]) -> bool,
) -> Result<(), String> {
    ticket.check()?;
    let mut child = Command::new(executable)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| "Could not start the system query.")?;
    let collection_deadline = collection_budget.map(|budget| Instant::now() + budget);
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let out = child.stdout.take().ok_or("System output unavailable.")?;
    let err = child.stderr.take().ok_or("System output unavailable.")?;
    let mut owned = Owned(child);
    let mut pipes: [Box<dyn Pipe>; 2] = [Box::new(out), Box::new(err)];
    for pipe in &pipes {
        let fd = pipe.fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err("Could not read the system query.".into());
        }
    }
    let mut eof = [false; 2];
    let mut counts = [0usize; 2];
    let mut buffer = [0u8; 16384];
    loop {
        ticket.check()?;
        if collection_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(());
        }
        for index in 0..2 {
            if eof[index] {
                continue;
            }
            match pipes[index].read(&mut buffer) {
                Ok(0) => eof[index] = true,
                Ok(n) => {
                    let remaining = limit.saturating_sub(counts[index]);
                    counts[index] = counts[index].saturating_add(n);
                    if counts[index] > limit && !truncate {
                        return Err("System query output is too large.".into());
                    }
                    if index == 0 && output(&buffer[..n.min(remaining)]) {
                        ticket.check()?;
                        return Ok(());
                    }
                    if counts[index] >= limit {
                        if truncate {
                            ticket.check()?;
                            return Ok(());
                        }
                        return Err("System query output is too large.".into());
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("Could not read the system query.".into()),
            }
        }
        // The original file collector finishes on stdout EOF, not on a child's
        // exit status or on stderr closing. Drop still retires/reaps our child.
        if collection_deadline.is_some() && eof[0] {
            ticket.check()?;
            return Ok(());
        }
        if let Some(status) = owned
            .0
            .try_wait()
            .map_err(|_| "Could not collect the system query.")?
        {
            if eof.iter().all(|v| *v) {
                ticket.check()?;
                return if status.success() {
                    Ok(())
                } else {
                    Err("The system query failed.".into())
                };
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
trait Pipe: Read {
    fn fd(&self) -> i32;
}
impl<T: Read + AsRawFd> Pipe for T {
    fn fd(&self) -> i32 {
        self.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Instant};
    fn ticket(ms: u64) -> Ticket {
        Ticket {
            deadline: Instant::now() + Duration::from_millis(ms),
            live: Arc::new(|| true),
        }
    }
    #[test]
    #[ignore = "Explicit own child fixture, not a standalone assertion"]
    fn owned_file_query_fixture() {
        use std::io::Write;
        let args = std::env::args().collect::<Vec<_>>();
        if !args.iter().any(|arg| matches!(arg.as_str(), "owned-file-nonzero" | "owned-file-hold" | "owned-file-stdout-eof")) {
            return;
        }
        // Start with a NUL to discard the test harness's non-path preamble.
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(b"\0/owned/first\0/owned/second\0/owned/incomplete").unwrap();
        stdout.flush().unwrap();
        drop(stdout);
        if args.iter().any(|arg| arg == "owned-file-stdout-eof") {
            assert_eq!(unsafe { libc::close(libc::STDOUT_FILENO) }, 0);
        }
        if args.iter().any(|arg| matches!(arg.as_str(), "owned-file-hold" | "owned-file-stdout-eof")) {
            std::thread::sleep(Duration::from_secs(10));
        }
        std::process::exit(17);
    }
    #[test]
    fn file_collection_preserves_hard_cancellation_and_outer_expiry() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let binary = std::env::current_exe().unwrap();
        let args = ["--exact", "os_command::tests::owned_file_query_fixture", "--ignored",
            "--nocapture", "--test-threads=1", "--skip", "owned-file-nonzero"];
        let live = Arc::new(AtomicBool::new(true));
        let check = live.clone();
        let request = Ticket { deadline: Instant::now() + Duration::from_secs(1),
            live: Arc::new(move || check.load(Ordering::Acquire)) };
        let mut collected = Vec::new();
        let result = file_query(binary.to_str().unwrap(), &args, &request, |bytes| {
            collected.extend_from_slice(bytes);
            if collected.windows(b"/owned/first\0".len()).any(|w|w==b"/owned/first\0") {
                live.store(false, Ordering::Release);
            }
            false
        });
        assert!(!live.load(Ordering::Acquire), "Fixture must supply output before cancellation.");
        assert_eq!(result.unwrap_err(), crate::system_search::CANCELLED);

        let request = ticket(40);
        let mut observed = false;
        let result = file_query(binary.to_str().unwrap(), &args, &request, |_| {
            observed = true;
            // Actual elapsed caller time, not a private collection-stop flag.
            std::thread::sleep(Duration::from_millis(60));
            false
        });
        assert!(observed);
        assert_eq!(result.unwrap_err(), crate::system_search::CANCELLED);

        assert_eq!(stream(binary.to_str().unwrap(), &args, &ticket(1000),
            1_048_576, true, |_| false).unwrap_err(), "The system query failed.");

        let args = ["--exact", "os_command::tests::owned_file_query_fixture", "--ignored",
            "--nocapture", "--test-threads=1", "--skip", "owned-file-stdout-eof"];
        let start = Instant::now();
        let mut collected = Vec::new();
        file_query(binary.to_str().unwrap(), &args, &ticket(1000), |bytes| {
            collected.extend_from_slice(bytes); false
        }).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(collected.windows(b"/owned/first\0".len()).any(|w|w==b"/owned/first\0"));
    }
    #[test]
    fn owned_process_output_stop_limits_and_deadline() {
        let mut bytes = vec![];
        stream(
            "/usr/bin/printf",
            &["%s", "owned literal ; $(nothing)"],
            &ticket(1000),
            100,
            false,
            |data| {
                bytes.extend_from_slice(data);
                false
            },
        )
        .unwrap();
        assert_eq!(bytes, b"owned literal ; $(nothing)");
        assert!(stream(
            "/usr/bin/printf",
            &["%s", "12345"],
            &ticket(1000),
            3,
            false,
            |_| false
        )
        .is_err());
        bytes.clear();
        stream(
            "/usr/bin/printf",
            &["%s", "12345"],
            &ticket(1000),
            3,
            true,
            |data| {
                bytes.extend_from_slice(data);
                false
            },
        )
        .unwrap();
        assert_eq!(bytes, b"123");
        let start = Instant::now();
        assert!(stream("/bin/sleep", &["10"], &ticket(30), 100, false, |_| false).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
