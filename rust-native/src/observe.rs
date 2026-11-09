//! Coalesced vnode notifications with a five-second metadata fallback. One
//! worker owns registrations; it never sends native objects or config payloads.
use crate::{control::HostEvent, profile::ProfilePaths};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

type Stamp = Option<(u64, u64, u64, i64, i64, i64, i64)>;
fn stamp(paths: &ProfilePaths, path: &PathBuf) -> Stamp {
    match paths.io() {
        Ok(Some(io)) => {
            let m = io.metadata(path).ok()??;
            return Some((
                m.device,
                m.inode,
                m.length,
                m.modified_seconds,
                m.modified_nanoseconds,
                m.changed_seconds,
                m.changed_nanoseconds,
            ));
        }
        Err(_) => return None,
        Ok(None) => (),
    }
    let m = fs::metadata(path).ok()?;
    Some((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn key(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}
fn change(queue: &File, event: &libc::kevent) -> bool {
    (unsafe {
        libc::kevent(
            queue.as_raw_fd(),
            event,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    }) == 0
}
struct State {
    running: AtomicBool,
    queued: AtomicBool,
}
enum ObserverQueue {
    Native(Option<File>),
    Attached {
        binding: crate::profile_io::Binding,
        current: Mutex<Option<crate::profile_io::Watch>>,
    },
}
impl ObserverQueue {
    fn new(paths: &ProfilePaths) -> Result<Self, String> {
        if let Some(binding) = paths.io()? {
            return Ok(Self::Attached {
                binding: binding.clone(),
                current: Mutex::new(None),
            });
        }
        let fd = unsafe { libc::kqueue() };
        let queue = if fd >= 0 {
            let file = unsafe { File::from_raw_fd(fd) };
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } >= 0
                && change(
                    &file,
                    &key(1, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0),
                )
            {
                Some(file)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self::Native(queue))
    }
    fn native(&self) -> Option<&File> {
        match self {
            Self::Native(queue) => queue.as_ref(),
            Self::Attached { .. } => None,
        }
    }
    fn watch(&self) -> Option<crate::profile_io::Watch> {
        match self {
            Self::Attached { current, .. } => current.lock().ok().and_then(|slot| slot.clone()),
            Self::Native(_) => None,
        }
    }
    fn rebind(&self, root: &std::path::Path, directories: &[PathBuf]) -> bool {
        let Self::Attached { binding, current } = self else {
            return self.native().is_some();
        };
        // A fresh watch drops obsolete selected-parent registrations, preserving
        // the original two-directory limit after configuration relocation.
        let next = binding
            .open_watch(root)
            .and_then(|watch| {
                for path in directories {
                    watch.add_directory(path)?;
                }
                Ok(watch)
            })
            .ok();
        let available = next.is_some();
        let old = match current.lock() {
            Ok(mut slot) => std::mem::replace(&mut *slot, next),
            Err(_) => return false,
        };
        // SDK cleanup may call foreign code; no mutex guard crosses it.
        drop(old);
        available
    }
    fn poll(&self) -> Result<bool, ()> {
        if matches!(self, Self::Attached { .. }) {
            return self
                .watch()
                .ok_or(())?
                .poll(Duration::from_secs(5))
                .map(|event| matches!(event, crate::profile_io::WatchEvent::Changed { .. }))
                .map_err(|_| ());
        }
        let queue = self.native().ok_or(())?;
        let mut event = key(0, 0, 0, 0);
        let timeout = libc::timespec {
            tv_sec: 5,
            tv_nsec: 0,
        };
        let count = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &timeout,
            )
        };
        if count < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            Err(())
        } else {
            Ok(count > 0 && event.filter == libc::EVFILT_VNODE)
        }
    }
    fn wake(&self) {
        if let Some(queue) = self.native() {
            let _ = change(queue, &key(1, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER));
        } else if let Some(watch) = self.watch() {
            let _ = watch.wake();
        }
    }
}
pub struct ConfigurationObserver {
    state: Arc<State>,
    queue: Arc<ObserverQueue>,
    worker: Option<JoinHandle<()>>,
}
impl ConfigurationObserver {
    pub fn start(paths: ProfilePaths, events: SyncSender<HostEvent>) -> Result<Self, String> {
        let state = Arc::new(State {
            running: AtomicBool::new(true),
            queued: AtomicBool::new(false),
        });
        let queue = Arc::new(ObserverQueue::new(&paths)?);
        let worker_queue = queue.clone();
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("ruycast-config".into())
            .spawn(move || {
                let mut registrations: Vec<File> = vec![];
                let mut registered = vec![];
                let mut previous = None;
                let attached = matches!(worker_queue.as_ref(), ObserverQueue::Attached { .. });
                let mut use_queue = worker_queue.native().is_some();
                while worker_state.running.load(Ordering::Acquire) {
                    let config = match paths.config_file() {
                        Ok(path) => Some(path),
                        Err(_) if attached => None,
                        Err(_) => Some(paths.root.join("config.json")),
                    };
                    let mut directories = vec![paths.root.clone()];
                    if let Some(parent) = config.as_deref().and_then(std::path::Path::parent) {
                        if parent != paths.root {
                            directories.push(parent.into());
                        }
                    }
                    let identities: Vec<_> = directories
                        .iter()
                        .map(|p| (p.clone(), stamp(&paths, p).map(|s| (s.0, s.1))))
                        .collect();
                    if identities != registered || (attached && !use_queue) {
                        registrations.clear();
                        registered = identities;
                        if attached {
                            use_queue = worker_queue.rebind(&paths.root, &directories);
                        }
                        if let Some(queue) = worker_queue.native() {
                            for path in &directories {
                                if let Ok(file) = OpenOptions::new()
                                    .read(true)
                                    .custom_flags(libc::O_EVTONLY | libc::O_CLOEXEC)
                                    .open(path)
                                {
                                    if change(
                                        queue,
                                        &key(
                                            file.as_raw_fd() as usize,
                                            libc::EVFILT_VNODE,
                                            libc::EV_ADD | libc::EV_CLEAR,
                                            libc::NOTE_WRITE
                                                | libc::NOTE_RENAME
                                                | libc::NOTE_DELETE
                                                | libc::NOTE_ATTRIB
                                                | libc::NOTE_REVOKE,
                                        ),
                                    ) {
                                        registrations.push(file);
                                    }
                                }
                            }
                        }
                    }
                    let config_stamp = config.as_ref().and_then(|file| stamp(&paths, file));
                    let current = (
                        config,
                        config_stamp,
                        stamp(&paths, &paths.root.join("config-location.json")),
                    );
                    let changed = previous.as_ref() != Some(&current);
                    let mut retry = false;
                    if changed
                        && !worker_state.queued.swap(true, Ordering::AcqRel)
                        && events.try_send(HostEvent::ConfigurationChanged).is_err()
                    {
                        worker_state.queued.store(false, Ordering::Release);
                        retry = true;
                    }
                    if !retry {
                        previous = Some(current);
                    }
                    // Stop may race with publishing a replacement watch. Check
                    // after publication so either this exits or stop can wake
                    // the exact currently retained queue before its poll.
                    if !worker_state.running.load(Ordering::Acquire) {
                        break;
                    }
                    if use_queue {
                        match worker_queue.poll() {
                            Err(()) => {
                                use_queue = false;
                                if attached {
                                    thread::park_timeout(Duration::from_secs(5));
                                }
                            }
                            Ok(true) => {
                                // Match the existing short editor-burst debounce. Stop
                                // unparks this wait, so teardown never waits for a timer.
                                thread::park_timeout(Duration::from_millis(80));
                            }
                            Ok(false) => (),
                        }
                    } else {
                        thread::park_timeout(Duration::from_secs(5));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            state,
            queue,
            worker: Some(worker),
        })
    }
    pub fn acknowledge(&self) {
        self.state.queued.store(false, Ordering::Release);
    }
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
    }
    pub fn stop(&mut self) {
        self.state.running.store(false, Ordering::Release);
        self.queue.wake();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
impl Drop for ConfigurationObserver {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, feature = "test-tools"))]
mod tests {
    use super::*;
    use crate::profile_io_abi as abi;
    use std::sync::atomic::AtomicUsize;

    static RECOVERED: AtomicBool = AtomicBool::new(false);
    static GUESSED_CONFIG: AtomicBool = AtomicBool::new(false);
    static WATCH_OPENS: AtomicUsize = AtomicUsize::new(0);
    static WATCH_POLLED: AtomicBool = AtomicBool::new(false);
    const PROFILE: &str = "/owned/profile";
    const SELECTOR: &[u8] = br#"{"directory":"/owned/selected"}"#;

    unsafe extern "C" fn bind(_: abi::Bytes) -> abi::Reply {
        let mut reply = abi::Reply::denied();
        reply.status = abi::OK;
        reply.handle = abi::Handle { slot: 1, generation: 1 };
        reply
    }

    unsafe extern "C" fn request(_: abi::Handle, request: abi::Request) -> abi::Reply {
        let path = unsafe { std::slice::from_raw_parts(request.path.pointer, request.path.length) };
        let mut reply = abi::Reply::denied();
        match request.operation {
            abi::READ if RECOVERED.load(Ordering::Acquire) => {
                reply.status = abi::OK;
                reply.buffer = abi::Buffer { token: 1, pointer: SELECTOR.as_ptr(), length: SELECTOR.len() };
            }
            abi::STAT => {
                if path == b"/owned/profile/config.json" {
                    GUESSED_CONFIG.store(true, Ordering::Release);
                }
                reply.status = abi::OK;
            }
            abi::WATCH_OPEN => {
                let attempt = WATCH_OPENS.fetch_add(1, Ordering::AcqRel);
                if attempt != 0 {
                    reply.status = abi::OK;
                    reply.handle = abi::Handle { slot: 2, generation: 1 };
                }
            }
            abi::WATCH_DIRECTORY | abi::WATCH_WAKE => reply.status = abi::OK,
            abi::WATCH_POLL => WATCH_POLLED.store(true, Ordering::Release),
            _ => (),
        }
        reply
    }

    extern "C" fn release(_: abi::Handle) -> i32 { abi::OK }
    extern "C" fn return_buffer(_: abi::Handle, _: u64) -> i32 { abi::OK }

    static API: abi::Api = abi::Api {
        version: abi::VERSION,
        bytes: std::mem::size_of::<abi::Api>() as u32,
        reserved: [0; 2],
        bind,
        request,
        release_handle: release,
        return_buffer,
    };

    #[test]
    fn attached_observer_recovers_without_reading_a_guessed_config() {
        let paths = ProfilePaths::new(PROFILE.into(), "/owned".into(), None);
        // This test owns the static ABI table and all its reply storage.
        let paths = unsafe { paths.with_pinned_io(&API) }.unwrap();
        assert!(paths.absolute("~synthetic_account/config").is_err());
        assert_eq!(paths.absolute("~/config").unwrap(), PathBuf::from("/owned/config"));
        let (events, received) = std::sync::mpsc::sync_channel(2);
        let mut observer = ConfigurationObserver::start(paths, events).unwrap();
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)).unwrap(), HostEvent::ConfigurationChanged));
        assert_eq!(WATCH_OPENS.load(Ordering::Acquire), 1);
        observer.acknowledge();
        let worker = observer.worker.as_ref().unwrap().thread().clone();
        worker.unpark();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !WATCH_POLLED.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline, "watch did not rebind with unchanged paths");
            thread::sleep(Duration::from_millis(5));
        }
        RECOVERED.store(true, Ordering::Release);
        worker.unpark();
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)).unwrap(), HostEvent::ConfigurationChanged));
        observer.stop();
        assert!(WATCH_OPENS.load(Ordering::Acquire) >= 2);
        assert!(!GUESSED_CONFIG.load(Ordering::Acquire));
    }
}
