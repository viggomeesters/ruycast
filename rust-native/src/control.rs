//! Bounded local JSON transport. This module owns sockets/leases, never UI or
//! action execution. The state owner receives cancellable, expiring requests.
use crate::private_storage;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const REQUEST_LIMIT: usize = 1_048_576;
pub(crate) const CLIENT_LIMIT: usize = 16;
#[derive(Clone, Copy)]
pub struct Limits {
    pub read: Duration,
    pub dispatch: Duration,
    pub write: Duration,
    pub reply_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            read: Duration::from_secs(3),
            dispatch: Duration::from_secs(310),
            write: Duration::from_secs(3),
            reply_bytes: 256 * 1024 * 1024,
        }
    }
}
pub enum HostEvent {
    ClipboardCaptureError(String),
    ClipboardCapture(crate::clipboard_history::Capture),
    SnippetFailure {
        epoch: u64,
        routing: u64,
        dismissal: u64,
        failure: crate::keyword_capture::Failure,
    },
    Hotkey(crate::hotkeys::RoutedPress),
    OpenCommit(crate::execution::CommitRequest),
    OpenHide(crate::execution::CommitRequest),
    PasteRead(crate::execution::CommitRequest),
    PasteCommit(crate::execution::CommitRequest),
    PasteHide(crate::execution::CommitRequest),
    Request(Envelope),
    ConfigurationChanged,
    ServerStopped,
    UiWake,
}
enum NativeDispatch {
    ClipboardHistory(Arc<crate::clipboard_history::Publication>),
    Workspace(Arc<crate::workspace::Dispatch>),
    Snippet(Arc<crate::paste::Dispatch>),
    Configuration(Arc<crate::configuration_jobs::Publication>),
    // A Shortcuts client can stop while its daemon/workflow keeps running.
    // Transport expiry must preserve that uncertainty after admission.
    Shortcut,
    Utility { storage:Arc<std::sync::atomic::AtomicU8>, paste:Option<Arc<crate::paste::Dispatch>> },
}
#[derive(Clone)]
pub struct RequestContext {
    deadline: Instant,
    received: Instant,
    cancelled: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    peer: Weak<UnixStream>,
    native_dispatch: Arc<Mutex<Option<NativeDispatch>>>,
}
impl RequestContext {
    pub fn clipboard_publication(&self)->Arc<crate::clipboard_history::Publication>{let mut slot=self.native_dispatch.lock().unwrap_or_else(|p|p.into_inner());if let Some(NativeDispatch::ClipboardHistory(p))=&*slot{return p.clone()}let p=Arc::new(crate::clipboard_history::Publication::default());if slot.is_some()||!self.is_live(){let _=p.retire();}
        if slot.is_none(){*slot=Some(NativeDispatch::ClipboardHistory(p.clone()));}p}
    pub fn bind_shortcut(&self) -> Result<(), String> {
        let mut slot = self.native_dispatch.lock().unwrap_or_else(|p| p.into_inner());
        match &*slot {
            None => { *slot = Some(NativeDispatch::Shortcut); Ok(()) }
            Some(NativeDispatch::Shortcut) => Ok(()),
            Some(_) => Err("Request operation changed before shortcut admission.".into()),
        }
    }
    /// One compound capability: persistence and optional clipboard output.
    /// A separately bound native operation is never overwritten or revived.
    pub fn utility_publication(&self,clipboard:bool)->Arc<std::sync::atomic::AtomicU8>{
        let mut slot=self.native_dispatch.lock().unwrap_or_else(|p|p.into_inner());
        if let Some(NativeDispatch::Utility{storage,paste})=&*slot{if paste.is_some()==clipboard{if !self.is_live(){crate::utility_storage::retire_publication(storage);}return storage.clone();}}
        let storage=Arc::new(std::sync::atomic::AtomicU8::new(0));
        if slot.is_some()||!self.is_live(){storage.store(3,Ordering::Release);return storage;}
        *slot=Some(NativeDispatch::Utility{storage:storage.clone(),paste:clipboard.then(||Arc::new(crate::paste::Dispatch::default()))});storage
    }
    fn decorate_utility(&self,response:&mut Value){
        if let Some(NativeDispatch::Utility{storage,..})=&*self.native_dispatch.lock().unwrap_or_else(|p|p.into_inner()){
            crate::utility_storage::decorate_publication(response,crate::utility_storage::retire_publication(storage));
        }
    }
    pub fn configuration_publication(&self) -> Arc<crate::configuration_jobs::Publication> {
        let mut slot = self
            .native_dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(NativeDispatch::Configuration(publication)) = &*slot {
            return publication.clone();
        }
        let publication = Arc::new(crate::configuration_jobs::Publication::default());
        if slot.is_some() || !self.is_live() {
            let _ = publication.retire();
        }
        if slot.is_none() {
            *slot = Some(NativeDispatch::Configuration(publication.clone()));
        }
        publication
    }
    pub fn cancel_response(&self) -> Value {
        self.timeout_response()
    }
    pub fn workspace_dispatch(&self) -> Arc<crate::workspace::Dispatch> {
        let mut slot = self
            .native_dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(NativeDispatch::Workspace(dispatch)) = &*slot {
            return dispatch.clone();
        }
        if slot.is_some() {
            let dispatch = Arc::new(crate::workspace::Dispatch::default());
            let _ = dispatch.retire();
            return dispatch;
        }
        let dispatch = Arc::new(crate::workspace::Dispatch::default());
        if !self.is_live() {
            let _ = dispatch.retire();
        }
        *slot = Some(NativeDispatch::Workspace(dispatch.clone()));
        dispatch
    }
    pub fn paste_dispatch(&self) -> Arc<crate::paste::Dispatch> {
        let mut slot = self
            .native_dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(NativeDispatch::Snippet(dispatch)) = &*slot {
            return dispatch.clone();
        }
        if let Some(NativeDispatch::Utility{paste:Some(dispatch),..})=&*slot{return dispatch.clone();}
        let dispatch = Arc::new(crate::paste::Dispatch::default());
        if slot.is_some() || !self.is_live() {
            let _ = dispatch.retire();
        }
        if slot.is_none() {
            *slot = Some(NativeDispatch::Snippet(dispatch.clone()));
        }
        dispatch
    }
    fn timeout_response(&self) -> Value {
        let slot = self
            .native_dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // Binding and fallback share this lock. An admission arriving after
        // transport expiry can never regain dispatch permission.
        self.cancelled.store(true, Ordering::Release);
        match &*slot {
            Some(dispatch) => {
                let mut response = match dispatch {
                    NativeDispatch::Workspace(d) => d.retire().response,
                    NativeDispatch::Snippet(d) => d.retire().response,
                    NativeDispatch::Configuration(p) => p.retire(),
                    NativeDispatch::ClipboardHistory(p) => p.retire(),
                    NativeDispatch::Shortcut => json!({"ok":false,"error":crate::system_search::apple_shortcuts::TIMEOUT}),
                    NativeDispatch::Utility{storage,paste}=>{
                        let mut response=paste.as_ref().map(|p|p.retire().response).unwrap_or_else(||json!({"ok":false,"error":"Utilities request timed out or was cancelled."}));
                        crate::utility_storage::decorate_publication(&mut response,crate::utility_storage::retire_publication(storage));response
                    },
                };
                response["transportExpired"] = json!(true);
                response
            }
            None => json!({"ok":false,"error":"Request timed out or was cancelled."}),
        }
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn received_at(&self) -> Instant {
        self.received
    }
    pub fn is_live(&self) -> bool {
        if !self.running.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire)
            || Instant::now() >= self.deadline
        {
            return false;
        }
        let Some(peer) = self.peer.upgrade() else {
            return false;
        };
        // A write-half close may still await a reply. Probe the peer's read
        // side without sending protocol bytes; SO_NOSIGPIPE is set at accept.
        let result =
            unsafe { libc::send(peer.as_raw_fd(), std::ptr::null(), 0, libc::MSG_DONTWAIT) };
        result >= 0
            || matches!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            )
    }
}
struct Reply {
    value: Value,
    delivered: SyncSender<bool>,
}
pub struct Envelope {
    pub value: Value,
    pub context: RequestContext,
    reply: Option<SyncSender<Reply>>,
    wake: Arc<ReplyWake>,
}
impl Envelope {
    /// The acknowledgement completes after the wire write, allowing quit to
    /// acknowledge before shutting down. Dropping it never blocks a sender.
    pub fn respond(self, mut value: Value) -> Receiver<bool> {
        self.context.decorate_utility(&mut value);
        let (tx, rx) = mpsc::sync_channel(1);
        let _ = self.reply.as_ref().expect("reply sender").try_send(Reply {
            value,
            delivered: tx,
        });
        rx
    }
}
impl Drop for Envelope {
    fn drop(&mut self) {
        self.reply.take();
        self.wake.notify();
    }
}
// EVFILT_WRITE reports the peer's read-side close. Unlike read EOF/poll HUP,
// this preserves a legitimate client shutdown(SHUT_WR) awaiting its reply.
// Reply publication/drop uses EVFILT_USER; no recurring response polling.
struct ReplyWake(File);
impl ReplyWake {
    fn new(stream: &UnixStream) -> io::Result<Self> {
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let changes = [
            Self::event(1, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0),
            Self::event(
                stream.as_raw_fd() as usize,
                libc::EVFILT_WRITE,
                libc::EV_ADD | libc::EV_CLEAR,
                0,
            ),
        ];
        if unsafe {
            libc::kevent(
                fd,
                changes.as_ptr(),
                2,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(file))
    }
    fn event(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
        libc::kevent {
            ident,
            filter,
            flags,
            fflags,
            data: 0,
            udata: std::ptr::null_mut(),
        }
    }
    fn notify(&self) {
        let event = Self::event(1, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
        unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                &event,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );
        }
    }
    fn wait(&self, deadline: Instant) -> io::Result<bool> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = libc::timespec {
            tv_sec: remaining.as_secs() as _,
            tv_nsec: remaining.subsec_nanos() as _,
        };
        let mut event = Self::event(0, 0, 0, 0);
        let count = unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &timeout,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(true);
            }
            return Err(error);
        }
        Ok(count > 0 && event.flags & (libc::EV_EOF | libc::EV_ERROR) == 0)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}
fn identity(path: &Path) -> Option<Identity> {
    let m = fs::symlink_metadata(path).ok()?;
    (m.file_type().is_socket() && m.uid() == unsafe { libc::getuid() }).then_some(Identity {
        device: m.dev(),
        inode: m.ino(),
    })
}
fn remove_owned(path: &Path, owned: Identity) {
    if identity(path) == Some(owned) {
        let _ = fs::remove_file(path);
    }
}
struct Lease {
    listener: UnixListener,
    _lock: Arc<File>,
    path: PathBuf,
    identity: Identity,
}
impl Drop for Lease {
    fn drop(&mut self) {
        remove_owned(&self.path, self.identity);
    }
}
struct Shared {
    running: Arc<AtomicBool>,
    clients: Mutex<HashMap<i32, Arc<UnixStream>>>,
}
impl Shared {
    fn shutdown_clients(&self) {
        for stream in self
            .clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}
struct Client {
    stream: Arc<UnixStream>,
    shared: Arc<Shared>,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.shared
            .clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.stream.as_raw_fd());
    }
}
struct Cancel(Arc<AtomicBool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct ExitGuard {
    shared: Arc<Shared>,
    events: SyncSender<HostEvent>,
}
impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.shutdown_clients();
        let _ = self.events.try_send(HostEvent::ServerStopped);
    }
}

pub struct ControlServer {
    profile: Option<ProfileLease>,
    shared: Arc<Shared>,
    wake: UnixStream,
    worker: Option<JoinHandle<()>>,
}
/// Opaque descriptor lease: writers keep the profile locked without keeping
/// the listener/socket alive after normal server shutdown.
#[derive(Clone)]
pub struct ProfileLease {
    _descriptor: Arc<File>,
}
impl ControlServer {
    pub fn start(
        root: &Path,
        events: SyncSender<HostEvent>,
        limits: Limits,
    ) -> Result<Self, String> {
        if limits.read.is_zero()
            || limits.write.is_zero()
            || limits.dispatch.is_zero()
            || limits.reply_bytes < 128
        {
            return Err("Invalid control-server limits".into());
        }
        private_storage::secure_directory(root)?;
        let path = root.join("control.sock");
        if path.as_os_str().as_bytes().len() >= 104 {
            return Err("Profile path is too long for a Unix socket.".into());
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(root.join("instance.lock"))
            .map_err(|e| e.to_string())?;
        let info = lock.metadata().map_err(|e| e.to_string())?;
        if !info.is_file() || info.uid() != unsafe { libc::getuid() } {
            return Err("Instance lock must be an owned regular file.".into());
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Ruycast is already running for this profile.".into());
        }
        if let Some(owned) = identity(&path) {
            probe_stale(&path)?;
            remove_owned(&path, owned);
        }
        let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
        let owned = identity(&path).ok_or("Control socket changed during startup.")?;
        // Lease cleans only this inode on every subsequent startup error.
        let lock = Arc::new(lock);
        let profile = ProfileLease {
            _descriptor: lock.clone(),
        };
        let lease = Arc::new(Lease {
            listener,
            _lock: lock,
            path,
            identity: owned,
        });
        let name =
            std::ffi::CString::new(lease.path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        if unsafe {
            libc::fchmodat(
                libc::AT_FDCWD,
                name.as_ptr(),
                0o600,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
            || identity(&lease.path) != Some(owned)
        {
            return Err("Cannot secure control socket.".into());
        }
        lease
            .listener
            .set_nonblocking(true)
            .map_err(|e| e.to_string())?;
        let (wake, reader) = UnixStream::pair().map_err(|e| e.to_string())?;
        no_sigpipe(&wake).map_err(|e| e.to_string())?;
        wake.set_nonblocking(true).map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared {
            running: Arc::new(AtomicBool::new(true)),
            clients: Mutex::new(HashMap::new()),
        });
        let task_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("ruycast-control".into())
            .spawn(move || {
                let _exit = ExitGuard {
                    shared: task_shared.clone(),
                    events: events.clone(),
                };
                let mut tasks: Vec<JoinHandle<()>> = vec![];
                while task_shared.running.load(Ordering::Acquire) {
                    let mut fds = [
                        libc::pollfd {
                            fd: lease.listener.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: reader.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    let result = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                    if result < 0 {
                        if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    if fds[1].revents != 0 || !task_shared.running.load(Ordering::Acquire) {
                        break;
                    }
                    if fds[0].revents & libc::POLLIN == 0 {
                        break;
                    }
                    loop {
                        if !task_shared.running.load(Ordering::Acquire) {
                            break;
                        }
                        let stream = match lease.listener.accept() {
                            Ok((stream, _)) => Arc::new(stream),
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(_) => {
                                task_shared.running.store(false, Ordering::Release);
                                break;
                            }
                        };
                        let mut clients = task_shared
                            .clients
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        if clients.len() >= CLIENT_LIMIT {
                            drop(clients);
                            continue;
                        }
                        clients.insert(stream.as_raw_fd(), stream.clone());
                        drop(clients);
                        let client = Client {
                            stream,
                            shared: task_shared.clone(),
                        };
                        let tx = events.clone();
                        if let Ok(task) = thread::Builder::new()
                            .name("ruycast-client".into())
                            .spawn(move || receive(client, tx, limits))
                        {
                            tasks.push(task);
                        }
                        let mut i = 0;
                        while i < tasks.len() {
                            if tasks[i].is_finished() {
                                let _ = tasks.swap_remove(i).join();
                            } else {
                                i += 1;
                            }
                        }
                    }
                }
                task_shared.running.store(false, Ordering::Release);
                task_shared.shutdown_clients();
                for task in tasks {
                    let _ = task.join();
                }
                // listener and instance lock remain leased through accept/client teardown.
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            profile: Some(profile),
            shared,
            wake,
            worker: Some(worker),
        })
    }
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }
    pub fn profile_lease(&self) -> ProfileLease {
        self.profile
            .as_ref()
            .expect("running profile lease")
            .clone()
    }
    pub fn active_clients(&self) -> usize {
        self.shared
            .clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.shutdown_clients();
        if let Some(worker) = self.worker.take() {
            let _ = (&self.wake).write_all(&[1]);
            let _ = worker.join();
        }
        self.profile.take();
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn probe_stale(path: &Path) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err("Cannot inspect control socket.".into());
    }
    // SAFETY: take ownership of the freshly allocated socket descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err("Cannot inspect control socket.".into());
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let bytes = path.as_os_str().as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        address.sun_path[i] = *byte as _;
    }
    let result = unsafe {
        libc::connect(
            file.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    let error = io::Error::last_os_error().raw_os_error();
    if result < 0 && matches!(error, Some(libc::ECONNREFUSED) | Some(libc::ENOENT)) {
        Ok(())
    } else {
        Err("Control socket is active or cannot be verified stale.".into())
    }
}
fn receive(client: Client, events: SyncSender<HostEvent>, limits: Limits) {
    if client.stream.set_nonblocking(false).is_err() {
        return;
    }
    if no_sigpipe(&client.stream).is_err() {
        return;
    }
    receive_request(client, events, limits);
}
fn no_sigpipe(stream: &UnixStream) -> io::Result<()> {
    let no_signal: libc::c_int = 1;
    if unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&no_signal as *const libc::c_int).cast(),
            std::mem::size_of_val(&no_signal) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn receive_request(client: Client, events: SyncSender<HostEvent>, limits: Limits) {
    let read_deadline = Instant::now() + limits.read;
    let mut bytes = vec![];
    let mut stream = &*client.stream;
    let request = (|| -> Result<Value, ()> {
        loop {
            let remaining = read_deadline
                .checked_duration_since(Instant::now())
                .ok_or(())?;
            stream.set_read_timeout(Some(remaining)).map_err(|_| ())?;
            let mut chunk = [0u8; 4096];
            let count = match stream.read(&mut chunk) {
                Ok(0) => return Err(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(()),
            };
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.len() >= REQUEST_LIMIT {
                return Err(());
            }
            if bytes.contains(&b'\n') {
                break;
            }
        }
        if bytes.last() != Some(&b'\n') {
            return Err(());
        }
        let value = crate::request_values::decode(&bytes).map_err(|_| ())?;
        if value.is_object() {
            Ok(value)
        } else {
            Err(())
        }
    })();
    let value = match request {
        Ok(value) => value,
        Err(()) => {
            let _ = send_wire(
                &client.stream,
                &json!({"ok":false,"error":"Expected one JSON object followed by newline (max 1 MiB), before the read deadline."}),
                &client.shared,
                limits,
            );
            return;
        }
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = Cancel(cancelled.clone());
    let context = RequestContext {
        deadline: Instant::now() + limits.dispatch,
        received: Instant::now(),
        cancelled,
        running: client.shared.running.clone(),
        peer: Arc::downgrade(&client.stream),
        native_dispatch: Arc::new(Mutex::new(None)),
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let wake = match ReplyWake::new(&client.stream) {
        Ok(wake) => Arc::new(wake),
        Err(_) => return,
    };
    if events
        .try_send(HostEvent::Request(Envelope {
            value,
            context: context.clone(),
            reply: Some(tx),
            wake: wake.clone(),
        }))
        .is_err()
    {
        let _ = send_wire(
            &client.stream,
            &json!({"ok":false,"error":"Control queue is busy or stopped."}),
            &client.shared,
            limits,
        );
        return;
    }
    while context.is_live() {
        match rx.try_recv() {
            Ok(reply) => {
                let sent = send_wire(&client.stream, &reply.value, &client.shared, limits).is_ok();
                let _ = reply.delivered.try_send(sent);
                return;
            }
            Err(TryRecvError::Empty) => {
                if !wake.wait(context.deadline).unwrap_or(false) {
                    break;
                }
            }
            Err(TryRecvError::Disconnected) => break,
        }
    }
    let _ = send_wire(
        &client.stream,
        &context.timeout_response(),
        &client.shared,
        limits,
    );
}
struct Buffer<'a> {
    bytes: Vec<u8>,
    max: usize,
    deadline: Instant,
    shared: &'a Shared,
}
impl Write for Buffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max
            || Instant::now() >= self.deadline
            || !self.shared.running.load(Ordering::Acquire)
        {
            return Err(io::Error::other("Reply exceeds limits or was cancelled"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn send_wire(
    stream: &UnixStream,
    value: &Value,
    shared: &Shared,
    limits: Limits,
) -> io::Result<()> {
    let deadline = Instant::now() + limits.write;
    let mut buffer = Buffer {
        bytes: vec![],
        max: limits.reply_bytes - 1,
        deadline,
        shared,
    };
    if serde_json::to_writer(&mut buffer, value).is_err() {
        buffer.bytes =
            b"{\"ok\":false,\"error\":\"Reply exceeds limits or was cancelled.\"}".to_vec();
    }
    buffer.bytes.push(b'\n');
    // Every caller has finished its only request read and closes this client
    // after the reply. No later protocol reader shares this flags transition.
    stream.set_nonblocking(true)?;
    let mut stream = stream;
    let mut offset = 0;
    while offset < buffer.bytes.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Reply timed out"));
        }
        if !shared.running.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Server stopped"));
        }
        // A blocking Darwin write can keep making partial progress beyond its
        // SO_SNDTIMEO. Nonblocking sends let the absolute deadline own each wait.
        match stream.write(&buffer.bytes[offset..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(sent) => offset += sent,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = deadline.checked_duration_since(Instant::now())
                    .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Reply timed out"))?;
                let mut ready = libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                let millis = remaining.as_nanos().div_ceil(1_000_000)
                    .min(i32::MAX as u128) as i32;
                let result = unsafe { libc::poll(&mut ready, 1, millis) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn utility_context()->(super::RequestContext,std::os::unix::net::UnixStream,std::sync::Arc<std::os::unix::net::UnixStream>){
        let (socket,other)=std::os::unix::net::UnixStream::pair().unwrap();let peer=std::sync::Arc::new(socket);
        let context=super::RequestContext{deadline:std::time::Instant::now()+std::time::Duration::from_secs(2),received:std::time::Instant::now(),cancelled:Default::default(),running:std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),peer:std::sync::Arc::downgrade(&peer),native_dispatch:Default::default()};(context,other,peer)
    }
    #[test]
    fn utility_publication_cas_preserves_unknown_published_and_refuses_mixed_late_binding(){
        let (context,_other,_peer)=utility_context();let publication=context.utility_publication(true);let dispatch=context.paste_dispatch();assert!(dispatch.preparing());
        assert_eq!(publication.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire),Ok(0));let response=context.cancel_response();assert_eq!(response["utilitiesStorageOutcome"],"unknown");assert_eq!(response["utilitiesReconciliationRequired"],true);assert_eq!(response["actionCompleted"],true);
        publication.store(2,Ordering::Release);assert_eq!(context.cancel_response()["utilitiesStorageOutcome"],"published");assert!(!context.workspace_dispatch().retire().committed);
        let (before,_other,_peer)=utility_context();let publication=before.utility_publication(false);assert!(!before.paste_dispatch().preparing());assert_eq!(before.cancel_response()["utilitiesStorageOutcome"],"unchanged");assert_eq!(publication.load(Ordering::Acquire),3);assert!(publication.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire).is_err());assert_eq!(before.utility_publication(false).load(Ordering::Acquire),3);
        let (mixed,_other,_peer)=utility_context();mixed.workspace_dispatch();assert_eq!(mixed.utility_publication(true).load(Ordering::Acquire),3);
    }
    #[test]
    fn utility_profile_durable_publication_is_reported_after_transport_retirement(){
        use crate::utilities::Storage;
        let root=Temp::new();let (context,_other,_peer)=utility_context();let publication=context.utility_publication(false);let check=context.clone();
        let mut authority=crate::utility_storage::Authority::read_write(std::sync::Arc::new(move||check.is_live()),std::sync::Arc::new(||Ok(())));authority.changed=publication;
        let mut store=crate::utility_storage::NativeStorage::production(root.0.clone()).unwrap();store.set_authority(Some(authority));store.read_profile().unwrap();store.save_profile(b"owned utility durable bytes").unwrap();
        let response=context.cancel_response();assert_eq!(response["utilitiesStorageOutcome"],"published");assert_eq!(response["utilitiesReconciliationRequired"],false);assert_eq!(std::fs::read(root.0.join("utilities.json")).unwrap(),b"owned utility durable bytes");
    }
    #[test]
    fn shortcut_transport_expiry_preserves_unknown_outcome_and_cannot_grant_late_work() {
        let (peer, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let peer = std::sync::Arc::new(peer);
        let context = super::RequestContext {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
            received: std::time::Instant::now(),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            peer: std::sync::Arc::downgrade(&peer),
            native_dispatch: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        context.bind_shortcut().unwrap();
        assert!(context.is_live());
        let response = context.timeout_response();
        assert_eq!(response["error"], crate::system_search::apple_shortcuts::TIMEOUT);
        assert_eq!(response["transportExpired"], true);
        assert!(!context.is_live());
        context.bind_shortcut().unwrap();
        assert!(!context.is_live());
        assert!(!context.workspace_dispatch().retire().committed);
    }
    #[test]
    fn late_native_binding_cannot_revive_transport_cancelled_dispatch() {
        let (peer, other) = std::os::unix::net::UnixStream::pair().unwrap();
        let peer = std::sync::Arc::new(peer);
        let context = super::RequestContext {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
            received: std::time::Instant::now(),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            peer: std::sync::Arc::downgrade(&peer),
            native_dispatch: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        assert!(context.is_live());
        assert!(!context.timeout_response()["ok"].as_bool().unwrap());
        let dispatch = context.workspace_dispatch();
        let cloned = context.clone().workspace_dispatch();
        assert!(std::sync::Arc::ptr_eq(&dispatch, &cloned));
        assert!(!dispatch.retire().committed);
        let response = context.timeout_response();
        assert_eq!(response["transportExpired"], true);
        assert!(response.get("actionCompleted").is_none());
        drop(other);
    }
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::fs::DirBuilderExt,
        sync::atomic::AtomicU64,
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").map(PathBuf::from)
                .unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
                .join(format!(
                    "rpc-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn limits() -> Limits {
        Limits {
            read: Duration::from_secs(2),
            dispatch: Duration::from_secs(2),
            write: Duration::from_secs(1),
            reply_bytes: 1024,
        }
    }
    fn client(root: &Path) -> UnixStream {
        let stream = UnixStream::connect(root.join("control.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }
    fn read(stream: UnixStream) -> Value {
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn request(rx: &Receiver<HostEvent>) -> Envelope {
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            HostEvent::Request(request) => request,
            _ => panic!("unexpected event"),
        }
    }
    fn until(mut ready: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(2);
        while !ready() {
            assert!(Instant::now() < end, "deadline exceeded");
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn failed_start_retains_nonsocket_then_retry_releases_profile_lock() {
        let temp = Temp::new();
        let path = temp.0.join("control.sock");
        fs::write(&path, b"keep").unwrap();
        let (tx, _rx) = mpsc::sync_channel(32);
        assert!(ControlServer::start(&temp.0, tx.clone(), limits()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep");
        fs::remove_file(&path).unwrap();
        let mut server = ControlServer::start(&temp.0, tx, limits()).unwrap();
        server.stop();
        assert!(!path.exists());
    }
    #[test]
    fn active_owner_and_replacement_paths_are_preserved() {
        let temp = Temp::new();
        let (tx, _rx) = mpsc::sync_channel(32);
        let mut first = ControlServer::start(&temp.0, tx.clone(), limits()).unwrap();
        assert!(ControlServer::start(&temp.0, tx, limits()).is_err());
        assert!(first.is_running());
        let path = temp.0.join("control.sock");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();
        first.stop();
        assert_eq!(fs::read(path).unwrap(), b"replacement");
    }
    #[test]
    fn stale_owned_socket_is_recovered_but_lock_symlink_is_refused() {
        let temp = Temp::new();
        let path = temp.0.join("control.sock");
        drop(UnixListener::bind(&path).unwrap());
        let (tx, _rx) = mpsc::sync_channel(32);
        let mut server = ControlServer::start(&temp.0, tx.clone(), limits()).unwrap();
        server.stop();
        fs::remove_file(temp.0.join("instance.lock")).unwrap();
        let target = temp.0.join("target");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, temp.0.join("instance.lock")).unwrap();
        assert!(ControlServer::start(&temp.0, tx, limits()).is_err());
        assert_eq!(fs::read(target).unwrap(), b"keep");
    }
    #[test]
    #[ignore = "Owned FIFO subprocess; invoked by the bounded exact parent test"]
    fn owned_instance_fifo_refusal_child() {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Explicit owned fixture parent required"),
        );
        // The existing supervisor launches this exact child directly. Its
        // home argument expands ~; it deliberately does not mutate child env.
        let home = parent.join(format!("instance-fifo-{}", unsafe { libc::getppid() }));
        assert!(parent.is_absolute() && home.is_absolute());
        assert!(home.canonicalize().unwrap().starts_with(parent.canonicalize().unwrap()));
        let metadata = fs::symlink_metadata(&home).unwrap();
        assert!(metadata.is_dir() && metadata.mode() & 0o777 == 0o700);
        assert_eq!(metadata.uid(), unsafe { libc::getuid() });
        assert_eq!(
            fs::read(home.join("owned-instance-fifo-marker")).unwrap(),
            b"owned-control-fifo-v1"
        );
        let path = home.join("fifo-case");
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let temp = Temp(path);
        let lock = temp.0.join("instance.lock");
        let name = std::ffi::CString::new(lock.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let before = fs::symlink_metadata(&lock).unwrap();
        assert!(before.file_type().is_fifo());
        let (tx, rx) = mpsc::sync_channel(32);
        match ControlServer::start(&temp.0, tx.clone(), limits()) {
            Err(error) => assert_eq!(error, "Instance lock must be an owned regular file."),
            Ok(mut server) => {
                server.stop();
                panic!("A FIFO must not acquire an instance lease");
            }
        }
        let after = fs::symlink_metadata(&lock).unwrap();
        assert!(after.file_type().is_fifo());
        assert_eq!(
            (before.dev(), before.ino(), before.mode(), before.nlink(), before.uid()),
            (after.dev(), after.ino(), after.mode(), after.nlink(), after.uid())
        );
        assert!(!temp.0.join("control.sock").exists());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        fs::remove_file(&lock).unwrap();
        // Recovery is only the existing owned socket service, never an app.
        let mut server = ControlServer::start(&temp.0, tx, limits()).unwrap();
        server.stop();
        assert!(!temp.0.join("control.sock").exists());
    }
    #[test]
    fn instance_fifo_refusal_is_bounded_and_preserves_owned_namespace() {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Explicit owned fixture parent required"),
        );
        assert!(parent.is_absolute());
        let path = parent.join(format!("instance-fifo-{}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let temp = Temp(path);
        fs::write(temp.0.join("owned-instance-fifo-marker"), b"owned-control-fifo-v1").unwrap();
        let executable = std::env::current_exe().unwrap();
        let command: rustcast_core::Command = serde_json::from_value(json!({
            "id":"owned.control.fifo", "title":"Owned FIFO refusal", "kind":"script",
            "target":executable, "arguments":["control::tests::owned_instance_fifo_refusal_child", "--exact", "--ignored", "--test-threads=1"],
            "timeout":4
        })).unwrap();
        // Reuse the existing process-group supervisor, so a blocking FIFO
        // regression fails and is reaped instead of hanging the unit runner.
        let result = rustcast_core::script::run(&command, &temp.0, None, || false).unwrap();
        assert!(matches!(result.end, rustcast_core::script::End::Exited(status) if status.success()), "{result:?}");
        assert!(result.capture.stdout.eof && !result.capture.stdout.truncated);
        assert!(result.capture.stderr.eof && !result.capture.stderr.truncated);
        assert!(result.stderr.is_empty(), "{}", result.stderr);
        assert!(result.stdout.contains("test control::tests::owned_instance_fifo_refusal_child ... ok"));
        assert!(result.stdout.contains("1 passed; 0 failed; 0 ignored;"));
        assert!(!temp.0.join("fifo-case").exists());
    }
    #[test]
    fn peer_disconnect_cancels_queued_request_but_half_close_receives_reply() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let _server = ControlServer::start(&temp.0, tx, limits()).unwrap();
        let mut stream = client(&temp.0);
        stream.write_all(b"{\"op\":\"quit\"}\n").unwrap();
        let abandoned = request(&rx);
        drop(stream);
        until(|| !abandoned.context.is_live());
        drop(abandoned);
        let mut half = client(&temp.0);
        half.write_all(b"{\"op\":\"status\"}\n").unwrap();
        half.shutdown(Shutdown::Write).unwrap();
        let pending = request(&rx);
        assert!(pending.context.is_live());
        let delivered = pending.respond(json!({"ok":true}));
        assert_eq!(read(half), json!({"ok":true}));
        assert!(delivered.recv_timeout(Duration::from_secs(1)).unwrap());
    }
    #[test]
    fn configuration_timeout_reports_possible_publication_and_cannot_reenter() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let mut options = limits();
        options.dispatch = Duration::from_millis(60);
        let _server = ControlServer::start(&temp.0, tx, options).unwrap();
        let mut stream = client(&temp.0);
        stream
            .write_all(b"{\"op\":\"config.directory\"}\n")
            .unwrap();
        let pending = request(&rx);
        let publication = pending.context.configuration_publication();
        publication.enter(|| pending.context.is_live()).unwrap();
        let response = read(stream);
        assert_eq!(
            response["configurationWrite"],
            json!({"published":false,"outcomeUncertain":true})
        );
        assert_eq!(response["transportExpired"], true);
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("may have been saved"));
        assert!(publication.enter(|| true).is_err());
        publication.complete(true);
        assert_eq!(
            pending.context.cancel_response()["configurationWrite"],
            json!({"published":true,"outcomeUncertain":false})
        );
    }
    #[test]
    fn stop_cancels_pending_requests_without_reusing_listener_or_hanging() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let mut server = ControlServer::start(&temp.0, tx.clone(), limits()).unwrap();
        let mut stream = client(&temp.0);
        stream.write_all(b"{}\n").unwrap();
        let pending = request(&rx);
        let start = Instant::now();
        server.stop();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!pending.context.is_live());
        let mut next = ControlServer::start(&temp.0, tx, limits()).unwrap();
        let late = pending.respond(json!({"ok":true}));
        assert!(late.recv_timeout(Duration::from_millis(100)).is_err());
        next.stop();
        server.stop();
    }
    #[test]
    fn request_deadlines_are_absolute_and_dropped_envelopes_wake_waiters() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let mut options = limits();
        options.dispatch = Duration::from_millis(60);
        let _server = ControlServer::start(&temp.0, tx, options).unwrap();
        let mut stream = client(&temp.0);
        stream.write_all(b"{}\n").unwrap();
        let pending = request(&rx);
        assert_eq!(read(stream)["ok"], false);
        assert!(!pending.context.is_live());
        drop(pending);
        let mut another = client(&temp.0);
        another.write_all(b"{}\n").unwrap();
        let pending = request(&rx);
        drop(pending);
        assert_eq!(read(another)["ok"], false);
    }
    #[test]
    fn maximum_sixteen_clients_and_replies_respect_wire_budget() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let mut options = limits();
        options.reply_bytes = 128;
        let server = ControlServer::start(&temp.0, tx, options).unwrap();
        let mut held = vec![];
        for _ in 0..16 {
            let mut stream = client(&temp.0);
            stream.write_all(b"{").unwrap();
            held.push(stream);
        }
        until(|| server.active_clients() == 16);
        let mut excess = client(&temp.0);
        let _ = excess.write_all(b"{}\n");
        let mut byte = [0];
        assert!(matches!(excess.read(&mut byte), Ok(0) | Err(_)));
        drop(held);
        until(|| server.active_clients() == 0);
        let mut stream = client(&temp.0);
        stream.write_all(b"{}\n").unwrap();
        request(&rx).respond(json!({"ok":true,"huge":"x".repeat(4096)}));
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert!(line.len() <= 128);
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], false);
    }
    #[test]
    fn wire_write_deadline_is_absolute_with_nonreading_and_dribbling_clients() {
        for dribble in [false, true] {
            let temp = Temp::new();
            let (tx, rx) = mpsc::sync_channel(32);
            let mut options = limits();
            options.write = Duration::from_millis(250);
            options.reply_bytes = 2 * 1024 * 1024;
            let mut server = ControlServer::start(&temp.0, tx, options).unwrap();
            let mut stream = client(&temp.0);
            stream.write_all(b"{\"op\":\"status\"}\n").unwrap();
            let pending = request(&rx);
            let peer = pending.context.peer.upgrade().unwrap();
            for (fd, name) in [
                (peer.as_raw_fd(), libc::SO_SNDBUF),
                (stream.as_raw_fd(), libc::SO_RCVBUF),
            ] {
                let size: libc::c_int = 4096;
                assert_eq!(unsafe {
                    libc::setsockopt(fd, libc::SOL_SOCKET, name,
                        (&size as *const libc::c_int).cast(), std::mem::size_of_val(&size) as _)
                }, 0);
            }
            stream.set_nonblocking(true).unwrap();
            let started = Instant::now();
            let delivered = pending.respond(json!({"ok":true,"payload":"x".repeat(1024 * 1024)}));
            let end = started + Duration::from_millis(750);
            let mut received = 0;
            let outcome = if dribble {
                loop {
                    match delivered.try_recv() {
                        Ok(sent) => break Some(sent),
                        Err(mpsc::TryRecvError::Disconnected) => break None,
                        Err(mpsc::TryRecvError::Empty) => (),
                    }
                    if Instant::now() >= end { break None; }
                    let mut bytes = [0; 128];
                    match stream.read(&mut bytes) {
                        Ok(count) => received += count,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => (),
                        result => panic!("unexpected owned socket read: {result:?}"),
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            } else {
                delivered.recv_timeout(end.saturating_duration_since(Instant::now())).ok()
            };
            let elapsed = started.elapsed();
            // Witness kernel bytes before cleanup; keeping the peer open prevents
            // disconnect or server.stop from manufacturing the delivery failure.
            let mut byte = 0u8;
            let peek = unsafe { libc::recv(stream.as_raw_fd(), (&mut byte as *mut u8).cast(),
                1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
            println!("dribble={dribble} delivery={outcome:?} elapsed_ms={} read_bytes={received} peek_bytes={peek}", elapsed.as_millis());
            server.stop();
            assert!(received > 0 || peek > 0, "reply must reach the real socket");
            assert_eq!(outcome, Some(false), "absolute write deadline; dribble={dribble}, elapsed={elapsed:?}");
            assert!(elapsed < Duration::from_millis(750));
        }
    }
    #[test]
    fn full_event_queue_returns_busy_without_request_admission() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        for _ in 0..32 { assert!(tx.try_send(HostEvent::UiWake).is_ok()); }
        let mut server = ControlServer::start(&temp.0, tx, limits()).unwrap();
        let mut stream = client(&temp.0);
        stream.write_all(b"{\"op\":\"execute\",\"id\":\"owned-never-admitted\"}\n").unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(),
            json!({"ok":false,"error":"Control queue is busy or stopped."}));
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0, "one Busy reply then EOF");
        let mut wakes = 0;
        let mut requests = 0;
        while let Ok(event) = rx.try_recv() {
            match event {
                HostEvent::UiWake => wakes += 1,
                HostEvent::Request(_) => requests += 1,
                _ => panic!("unexpected event"),
            }
        }
        server.stop();
        println!("queued_ui_wakes={wakes} admitted_requests={requests}");
        assert_eq!(wakes, 32);
        assert_eq!(requests, 0, "no owner admission means no dispatched effect");
    }
    #[test]
    fn malformed_and_multi_object_requests_never_enter_dispatch() {
        let temp = Temp::new();
        let (tx, rx) = mpsc::sync_channel(32);
        let _server = ControlServer::start(&temp.0, tx, limits()).unwrap();
        for bytes in [b"[]\n".as_slice(), b"{}\n{}\n", b"not-json\n"] {
            let mut stream = client(&temp.0);
            stream.write_all(bytes).unwrap();
            assert_eq!(read(stream)["ok"], false);
        }
        let mut oversized = client(&temp.0);
        oversized.write_all(&vec![b' '; REQUEST_LIMIT]).unwrap();
        assert_eq!(read(oversized)["ok"], false);
        assert!(rx.try_recv().is_err());
    }
}
