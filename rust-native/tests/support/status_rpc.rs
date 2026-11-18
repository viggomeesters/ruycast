//! Owned host readiness uses the shared production absolute-deadline RPC.
pub use ruycast_native::benchmark_host::rpc::request;
use serde_json::{json, Value};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
const RESPONSE_LIMIT: usize = 1024 * 1024;
fn probe(socket: &Path, deadline: Instant) -> io::Result<Value> {
    request(socket, &json!({"op":"status"}), deadline)
}
pub fn wait(socket: &Path, timeout: Duration, mut alive: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    let mut last_error = String::from("no status reply");
    loop {
        assert!(alive(), "owned host exited before readiness");
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "owned host readiness timeout: {last_error}"
        );
        match probe(socket, deadline) {
            Ok(response) => {
                assert_eq!(response["ok"], true, "{response}");
                assert!(Instant::now() < deadline, "owned host readiness timeout");
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                panic!("invalid readiness reply: {error}")
            }
            Err(error) => last_error = error.to_string(),
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::{
            fs::{DirBuilderExt, MetadataExt},
            net::UnixListener,
        },
        path::PathBuf,
        sync::{
            atomic::{AtomicU64, AtomicUsize, Ordering},
            Arc,
        },
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = PathBuf::from(
                std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("Own private parent"),
            );
            let m = fs::symlink_metadata(&parent).unwrap();
            assert!(parent.is_absolute() && m.is_dir() && !m.file_type().is_symlink());
            assert_eq!(m.uid(), unsafe { libc::geteuid() });
            assert_eq!(m.mode() & 0o077, 0);
            assert_eq!(fs::canonicalize(&parent).unwrap(), parent);
            let directory = parent.join(format!(
                "ready-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            Self(directory)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn existing_path_does_not_qualify_before_a_real_status_reply() {
        let fixture = Fixture::new();
        let socket = fixture.0.join("control.sock");
        fs::write(&socket, b"not a listener").unwrap();
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let own_socket = socket.clone();
        let waiter = std::thread::spawn(move || {
            wait(&own_socket, Duration::from_secs(2), || {
                observed.fetch_add(1, Ordering::SeqCst);
                true
            })
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while polls.load(Ordering::SeqCst) < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            !waiter.is_finished(),
            "A pathname without a status server is not readiness"
        );
        fs::remove_file(&socket).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(stream) => break stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({"op":"status"})
        );
        stream.write_all(b"{\"ok\":true}\n").unwrap();
        waiter.join().unwrap();
    }
    #[test]
    fn child_exit_and_readiness_deadline_are_refusals() {
        let fixture = Fixture::new();
        let socket = fixture.0.join("absent.sock");
        assert!(
            std::panic::catch_unwind(|| wait(&socket, Duration::from_secs(1), || false)).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| wait(&socket, Duration::from_millis(20), || true)).is_err()
        );
    }
    #[test]
    fn fragmented_and_oversized_status_replies_stay_bounded() {
        for oversized in [false, true] {
            let fixture = Fixture::new();
            let socket = fixture.0.join("reply.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(peer) => break peer,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                if oversized {
                    let _ = stream.write_all(b"{\"ok\":true}");
                    let _ = stream.write_all(&vec![b' '; RESPONSE_LIMIT]);
                    let _ = stream.write_all(b"\n");
                } else {
                    for _ in 0..100 {
                        if stream.write_all(b" ").is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            });
            let budget = if oversized {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(30)
            };
            let error = probe(&socket, Instant::now() + budget).unwrap_err();
            assert_eq!(
                error.kind(),
                if oversized {
                    io::ErrorKind::InvalidData
                } else {
                    io::ErrorKind::TimedOut
                }
            );
            server.join().unwrap();
        }
    }
}
