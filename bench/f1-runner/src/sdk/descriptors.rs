//! One issuer/retirement authority. An integer and fstat never create a grant.
//! C routing is deliberately not installed here: every actual source close,
//! duplicate and read route must be selected before full endpoint admission.
use super::owned::{AliveLease, DescriptorParts, FileAccess, FileCreation, OwnedRoot, Result};
use std::{
    collections::BTreeMap,
    io,
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorKind {
    Regular,
    PipeRead,
    PipeWrite,
    NullRead,
    NullWrite,
    DirectoryWatch,
    EventQueue,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseOwner {
    Caller,
    Sdk,
}
#[derive(Clone)]
enum Backing {
    Caller,
    Borrowed(Weak<OwnedFd>),
}
#[derive(Clone)]
struct Record {
    generation: u64,
    kind: DescriptorKind,
    access: FileAccess,
    identity: (u64, u64, libc::mode_t),
    backing: Backing,
    root: Option<OwnedRoot>,
    path: Option<PathBuf>,
    life: Arc<dyn AliveLease>,
    retired: Arc<AtomicBool>,
    child_stdio: bool,
}
struct State {
    next: u64,
    records: BTreeMap<i32, Record>,
}
pub struct DescriptorTable {
    state: Mutex<State>,
}
/// A retained descriptor lease has a generation, not merely its current number.
#[derive(Clone)]
pub struct DescriptorLease {
    table: Arc<DescriptorTable>,
    fd: i32,
    generation: u64,
}
/// Actual SDK-owned descriptor. Only this Arc owns close; a C caller may borrow
/// its number for API interoperability but cannot close or take ownership of it.
pub struct SdkDescriptor {
    descriptor: Arc<OwnedFd>,
    lease: DescriptorLease,
}
/// Plain pre-fork data, bound to the parent's descriptor generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StdioRow {
    pub generation: u64,
    pub fd: i32,
    pub identity: (u64, u64, i32, libc::mode_t),
}
/// The parent retains pins only through the last transfer/admission ACK, never
/// child/job exit. After fork the child closes pin_fds and forgets this value.
pub struct StdioSnapshot {
    rows: Vec<StdioRow>,
    pin_fds: Vec<i32>,
    _pins: Vec<OwnedFd>,
    table: Arc<DescriptorTable>,
    lives: Vec<StdioLife>,
}
struct StdioLife {
    lease: Arc<dyn AliveLease>,
    retired: Arc<AtomicBool>,
}
impl StdioSnapshot {
    pub fn rows(&self) -> &[StdioRow] {
        &self.rows
    }
    pub fn pin_fds(&self) -> &[i32] {
        &self.pin_fds
    }
    pub fn pin_for(&self, fd: i32, generation: u64) -> Result<i32> {
        self.rows
            .iter()
            .position(|r| r.fd == fd && r.generation == generation)
            .map(|i| self.pin_fds[i])
            .ok_or_else(|| "descriptor absent from immutable stdio snapshot".into())
    }
    pub fn verify(&self) -> Result<()> {
        self.verify_transferred()?;
        for row in &self.rows {
            self.table.authorize(row.fd, Some(row.generation))?;
        }
        let state = self
            .table
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?;
        for row in &self.rows {
            let (record, _) = DescriptorTable::checked(&state, row.fd, Some(row.generation))?;
            if !record.child_stdio
                || identity(row.fd)? != (row.identity.0, row.identity.1, row.identity.3)
            {
                return Err("stdio snapshot source retired/changed".into());
            }
        }
        Ok(())
    }
    /// Requires separately proven kernel transfer. Normal parent close after
    /// that boundary cannot revoke a child's admitted dup. This checks retained
    /// OFDs/lifecycle only; it does not mint transfer proof or regrant parent FDs.
    pub fn verify_transferred(&self) -> Result<()> {
        for life in &self.lives {
            if life.retired.load(Ordering::Acquire) {
                return Err("stdio guardian authority retired".into());
            }
            before(&*life.lease).inspect_err(|_| life.retired.store(true, Ordering::Release))?;
        }
        // A later callback may retire an earlier row. After all dynamic work,
        // check every sticky state again before validating retained OFDs.
        for ((row, pin), life) in self.rows.iter().zip(&self._pins).zip(&self.lives) {
            if life.retired.load(Ordering::Acquire) {
                return Err("stdio guardian authority retired during admission".into());
            }
            if identity(pin.as_raw_fd())? != (row.identity.0, row.identity.1, row.identity.3) {
                return Err("retained stdio OFD identity changed".into());
            }
            let flags = unsafe { libc::fcntl(pin.as_raw_fd(), libc::F_GETFL) };
            if flags < 0
                || flags & libc::O_ACCMODE != row.identity.2
                || flags & libc::O_NONBLOCK != 0
            {
                return Err("retained stdio access/blocking mode changed".into());
            }
        }
        Ok(())
    }
}
impl SdkDescriptor {
    pub fn raw(&self) -> i32 {
        self.descriptor.as_raw_fd()
    }
    pub fn lease(&self) -> DescriptorLease {
        self.lease.clone()
    }
}
fn before(life: &dyn AliveLease) -> Result<()> {
    if !life.is_alive() {
        return Err("descriptor lifecycle retired".into());
    }
    life.before_effect()
}
fn raw(fd: i32) -> Result<OwnedFd> {
    if fd < 0 {
        Err(io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}
fn identity(fd: i32) -> Result<(u64, u64, libc::mode_t)> {
    let info = file_info(fd)?;
    Ok((info.st_dev as u64, info.st_ino, info.st_mode & libc::S_IFMT))
}
fn file_info(fd: i32) -> Result<libc::stat> {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, info.as_mut_ptr()) } != 0 {
        return Err("issued descriptor no longer exists".into());
    }
    Ok(unsafe { info.assume_init() })
}
fn pipe_pair(nonblocking: bool) -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err("own pipe failed".into());
    }
    let read = raw(fds[0])?;
    let write = raw(fds[1])?;
    for fd in [&read, &write] {
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err("own pipe CLOEXEC failed".into());
        }
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe {
                libc::fcntl(
                    fd.as_raw_fd(),
                    libc::F_SETFL,
                    if nonblocking {
                        flags | libc::O_NONBLOCK
                    } else {
                        flags & !libc::O_NONBLOCK
                    },
                )
            } != 0
        {
            return Err("own pipe blocking policy failed".into());
        }
    }
    // Darwin F_SETNOSIGPIPE=73 changes only this newly issued write endpoint.
    if unsafe { libc::fcntl(write.as_raw_fd(), 73, 1) } != 0 {
        return Err("own pipe no-SIGPIPE setup failed".into());
    }
    Ok((read, write))
}
impl DescriptorTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                next: 1,
                records: BTreeMap::new(),
            }),
        })
    }
    fn issue_owned(self: &Arc<Self>, fd: OwnedFd, mut record: Record) -> Result<SdkDescriptor> {
        before(&*record.life)?;
        let actual = identity(fd.as_raw_fd())?;
        let expected = match record.kind {
            DescriptorKind::Regular => libc::S_IFREG,
            DescriptorKind::PipeRead | DescriptorKind::PipeWrite => libc::S_IFIFO,
            DescriptorKind::NullRead | DescriptorKind::NullWrite => libc::S_IFCHR,
            DescriptorKind::DirectoryWatch => libc::S_IFDIR,
            // Observed Darwin queue mode; only event_queue() can issue this kind.
            DescriptorKind::EventQueue => libc::S_IFIFO,
        };
        if actual.2 != expected {
            return Err("newly issued descriptor kind differs".into());
        }
        let mut state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
        let number = fd.as_raw_fd();
        // Stale weak SDK records cannot survive actual owner retirement/reuse.
        if let Some(old) = state.records.get(&number) {
            if matches!(&old.backing,Backing::Borrowed(w)if w.upgrade().is_some()) {
                return Err("descriptor number still issued".into());
            }
        }
        let generation = state.next;
        state.next = state
            .next
            .checked_add(1)
            .ok_or("descriptor generation exhausted")?;
        let descriptor = Arc::new(fd);
        record.generation = generation;
        record.identity = actual;
        record.backing = Backing::Borrowed(Arc::downgrade(&descriptor));
        state.records.insert(number, record);
        Ok(SdkDescriptor {
            descriptor,
            lease: DescriptorLease {
                table: self.clone(),
                fd: number,
                generation,
            },
        })
    }
    fn issue_caller(self: &Arc<Self>, fd: OwnedFd, mut record: Record) -> Result<DescriptorLease> {
        before(&*record.life)?;
        record.identity = identity(fd.as_raw_fd())?;
        let number = fd.as_raw_fd();
        let mut state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
        if let Some(old) = state.records.get(&number) {
            if matches!(&old.backing,Backing::Borrowed(w)if w.upgrade().is_some()) {
                return Err("descriptor number still issued".into());
            }
        }
        let generation = state.next;
        state.next = state
            .next
            .checked_add(1)
            .ok_or("descriptor generation exhausted")?;
        record.generation = generation;
        record.backing = Backing::Caller;
        state.records.insert(number, record);
        assert_eq!(fd.into_raw_fd(), number);
        Ok(DescriptorLease {
            table: self.clone(),
            fd: number,
            generation,
        })
    }
    pub fn regular(
        self: &Arc<Self>,
        root: &OwnedRoot,
        path: &Path,
        access: FileAccess,
        creation: FileCreation,
        owner: CloseOwner,
        life: Arc<dyn AliveLease>,
    ) -> Result<DescriptorLease> {
        if owner == CloseOwner::Sdk {
            return Err("SDK ownership uses regular_owned".into());
        }
        let DescriptorParts {
            file,
            root,
            path,
            access,
            life,
        } = root
            .open_regular(path, access, creation, life)?
            .into_descriptor()?;
        self.issue_caller(
            file,
            Record {
                generation: 0,
                kind: DescriptorKind::Regular,
                access,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: Some(root),
                path: Some(path),
                life,
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )
    }
    pub fn regular_owned(
        self: &Arc<Self>,
        root: &OwnedRoot,
        path: &Path,
        access: FileAccess,
        creation: FileCreation,
        life: Arc<dyn AliveLease>,
    ) -> Result<SdkDescriptor> {
        let DescriptorParts {
            file,
            root,
            path,
            access,
            life,
        } = root
            .open_regular(path, access, creation, life)?
            .into_descriptor()?;
        self.issue_owned(
            file,
            Record {
                generation: 0,
                kind: DescriptorKind::Regular,
                access,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: Some(root),
                path: Some(path),
                life,
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )
    }
    pub fn directory_for_watch(
        self: &Arc<Self>,
        root: &OwnedRoot,
        path: &Path,
        life: Arc<dyn AliveLease>,
    ) -> Result<SdkDescriptor> {
        before(&*life)?;
        let directory = root.open_watch_directory(path)?;
        before(&*life)?;
        self.issue_owned(
            directory,
            Record {
                generation: 0,
                kind: DescriptorKind::DirectoryWatch,
                access: FileAccess::Read,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: Some(root.clone()),
                path: Some(path.into()),
                life,
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )
    }
    pub fn event_queue(self: &Arc<Self>, life: Arc<dyn AliveLease>) -> Result<SdkDescriptor> {
        before(&*life)?;
        // Creation provenance + private owner/generation issue the queue. Its
        // observed dev/ino0 and FIFO mode cannot independently grant authority.
        let queue = raw(unsafe { libc::kqueue() })?;
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err("watch queue CLOEXEC failed".into());
        }
        before(&*life)?;
        self.issue_owned(
            queue,
            Record {
                generation: 0,
                kind: DescriptorKind::EventQueue,
                access: FileAccess::ReadWrite,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: None,
                path: None,
                life,
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )
    }
    pub fn pipe_owned(
        self: &Arc<Self>,
        life: Arc<dyn AliveLease>,
    ) -> Result<(SdkDescriptor, SdkDescriptor)> {
        before(&*life)?;
        let (read, write) = pipe_pair(true)?;
        let r = self.issue_owned(
            read,
            Record {
                generation: 0,
                kind: DescriptorKind::PipeRead,
                access: FileAccess::Read,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: None,
                path: None,
                life: life.clone(),
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )?;
        let w = self.issue_owned(
            write,
            Record {
                generation: 0,
                kind: DescriptorKind::PipeWrite,
                access: FileAccess::Write,
                identity: (0, 0, 0),
                backing: Backing::Caller,
                root: None,
                path: None,
                life,
                retired: Arc::new(AtomicBool::new(false)),
                child_stdio: false,
            },
        )?;
        Ok((r, w))
    }
    /// Blocking child stdio, distinct from the nonblocking SDK callback pair.
    /// Only this actual factory creates grants; returned OwnedFds are caller-owned.
    pub fn pipe_for_process(
        self: &Arc<Self>,
        life: Arc<dyn AliveLease>,
    ) -> Result<(OwnedFd, OwnedFd)> {
        self.process_pipe(life, false)
    }
    pub(crate) fn process_pipe(
        self: &Arc<Self>,
        life: Arc<dyn AliveLease>,
        parent_reader: bool,
    ) -> Result<(OwnedFd, OwnedFd)> {
        before(&*life)?;
        let (read, write) = pipe_pair(false)?;
        let mut reader = Self::process_record(DescriptorKind::PipeRead, FileAccess::Read, life.clone());
        if parent_reader {
            let fd = read.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
                return Err("own parent pipe reader nonblocking setup failed".into());
            }
            reader.child_stdio = false;
        }
        let r = self.issue_caller(read, reader)?;
        let read = unsafe { OwnedFd::from_raw_fd(r.raw()) };
        let w = self.issue_caller(
            write,
            Self::process_record(DescriptorKind::PipeWrite, FileAccess::Write, life),
        )?;
        Ok((read, unsafe { OwnedFd::from_raw_fd(w.raw()) }))
    }
    pub fn null_for_process(
        self: &Arc<Self>,
        flags: i32,
        life: Arc<dyn AliveLease>,
    ) -> Result<OwnedFd> {
        before(&*life)?;
        let allowed = libc::O_ACCMODE | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NOCTTY;
        if flags & !allowed != 0 || flags & libc::O_ACCMODE > libc::O_RDWR {
            return Err("unqualified null-device flags".into());
        }
        let access = match flags & libc::O_ACCMODE {
            libc::O_RDONLY => FileAccess::Read,
            libc::O_WRONLY => FileAccess::Write,
            _ => FileAccess::ReadWrite,
        };
        let fd = raw(unsafe {
            libc::open(
                c"/dev/null".as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        })?;
        if file_info(fd.as_raw_fd())?.st_mode & libc::S_IFMT != libc::S_IFCHR {
            return Err("null device is not character data".into());
        }
        let kind = if access == FileAccess::Read {
            DescriptorKind::NullRead
        } else {
            DescriptorKind::NullWrite
        };
        let lease = self.issue_caller(fd, Self::process_record(kind, access, life))?;
        Ok(unsafe { OwnedFd::from_raw_fd(lease.raw()) })
    }
    fn process_record(
        kind: DescriptorKind,
        access: FileAccess,
        life: Arc<dyn AliveLease>,
    ) -> Record {
        Record {
            generation: 0,
            kind,
            access,
            identity: (0, 0, 0),
            backing: Backing::Caller,
            root: None,
            path: None,
            life,
            retired: Arc::new(AtomicBool::new(false)),
            child_stdio: true,
        }
    }
    /// RAII pins end at the last FD transfer/admission acknowledgment, never
    /// child/job exit. Pins are not newly granted product descriptors.
    pub fn stdio_snapshot(self: &Arc<Self>) -> Result<StdioSnapshot> {
        let keys: Vec<_> = self
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?
            .records
            .iter()
            .filter(|(_, r)| r.child_stdio)
            .map(|(&fd, r)| (fd, r.generation))
            .collect();
        let mut snapshot = StdioSnapshot {
            rows: vec![],
            pin_fds: vec![],
            _pins: vec![],
            table: self.clone(),
            lives: vec![],
        };
        for (fd, generation) in keys {
            self.authorize(fd, Some(generation))?;
            let mut state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
            if state.records.get(&fd).map(|r| r.generation) != Some(generation) {
                return Err("stdio generation changed during admission".into());
            }
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0
                && io::Error::last_os_error().raw_os_error() == Some(libc::EBADF)
            {
                state.records.remove(&fd);
                continue;
            }
            let (record, _) = Self::checked(&state, fd, Some(generation))?;
            let minimum = state
                .records
                .keys()
                .next_back()
                .copied()
                .unwrap_or(2)
                .checked_add(1)
                .ok_or("descriptor number exhausted")?;
            let pin = raw(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, minimum) })?;
            let access = match record.access {
                FileAccess::Read => libc::O_RDONLY,
                FileAccess::Write => libc::O_WRONLY,
                FileAccess::ReadWrite => libc::O_RDWR,
            };
            snapshot.rows.push(StdioRow {
                generation,
                fd,
                identity: (
                    record.identity.0,
                    record.identity.1,
                    access,
                    record.identity.2,
                ),
            });
            snapshot.pin_fds.push(pin.as_raw_fd());
            snapshot._pins.push(pin);
            snapshot.lives.push(StdioLife {
                lease: record.life,
                retired: record.retired,
            });
        }
        snapshot.verify()?;
        Ok(snapshot)
    }
    fn checked(
        state: &State,
        fd: i32,
        generation: Option<u64>,
    ) -> Result<(Record, Option<Arc<OwnedFd>>)> {
        let record = state
            .records
            .get(&fd)
            .ok_or("unknown descriptor; no filesystem query")?;
        if generation.is_some_and(|n| n != record.generation) {
            return Err("retired descriptor generation".into());
        }
        if record.retired.load(Ordering::Acquire) {
            return Err("descriptor authority permanently retired".into());
        }
        let pin = match &record.backing {
            Backing::Caller => None,
            Backing::Borrowed(weak) => Some(weak.upgrade().ok_or("SDK descriptor owner retired")?),
        };
        if identity(fd)? != record.identity {
            return Err("issued descriptor identity changed".into());
        }
        if let Some(root) = &record.root {
            let info = file_info(fd)?;
            if (record.kind == DescriptorKind::Regular && info.st_nlink != 1)
                || (record.kind == DescriptorKind::DirectoryWatch
                    && info.st_mode & libc::S_IFMT != libc::S_IFDIR)
                || (info.st_uid, info.st_gid) != root.owner_ids()?
            {
                return Err("issued regular descriptor ownership changed".into());
            }
        }
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err("issued descriptor flags unavailable".into());
        }
        let allowed = match record.access {
            FileAccess::Read => libc::O_RDONLY,
            FileAccess::Write => libc::O_WRONLY,
            FileAccess::ReadWrite => libc::O_RDWR,
        };
        if flags & libc::O_ACCMODE != allowed {
            return Err("issued access mode changed".into());
        }
        if matches!(
            record.kind,
            DescriptorKind::PipeRead | DescriptorKind::PipeWrite
        ) && (flags & libc::O_NONBLOCK != 0) == record.child_stdio
        {
            return Err("issued pipe changed its blocking policy".into());
        }
        if matches!(
            record.kind,
            DescriptorKind::DirectoryWatch | DescriptorKind::EventQueue
        ) {
            let fdflags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if fdflags < 0
                || fdflags & libc::FD_CLOEXEC == 0
                || (record.kind == DescriptorKind::DirectoryWatch && flags & libc::O_EVTONLY == 0)
                || (record.kind == DescriptorKind::EventQueue && flags & libc::O_NONBLOCK != 0)
            {
                return Err("private watch descriptor flags changed".into());
            }
        }
        Ok((record.clone(), pin))
    }
    fn authorize(&self, fd: i32, generation: Option<u64>) -> Result<u64> {
        let (life, retired, admitted_generation) = {
            let state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
            let record = state
                .records
                .get(&fd)
                .ok_or("unknown descriptor; no filesystem query")?;
            if generation.is_some_and(|n| n != record.generation) {
                return Err("retired descriptor generation".into());
            }
            if record.retired.load(Ordering::Acquire) {
                return Err("descriptor authority permanently retired".into());
            }
            (
                record.life.clone(),
                record.retired.clone(),
                record.generation,
            )
        };
        // A guardian callback may reenter this authority; no table lock crosses it.
        before(&*life).inspect_err(|_| retired.store(true, Ordering::Release))?;
        Ok(admitted_generation)
    }
    pub fn borrow(self: &Arc<Self>, fd: i32) -> Result<DescriptorLease> {
        let admitted_generation = self.authorize(fd, None)?;
        let state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
        let (record, _) = Self::checked(&state, fd, Some(admitted_generation))?;
        Ok(DescriptorLease {
            table: self.clone(),
            fd,
            generation: record.generation,
        })
    }
    pub fn close(&self, fd: i32) -> Result<()> {
        let admitted_generation = self.authorize(fd, None)?;
        let mut state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
        let (record, _) = Self::checked(&state, fd, Some(admitted_generation))?;
        if !matches!(record.backing, Backing::Caller) {
            return Err("C close cannot retire an SDK-owned descriptor".into());
        }
        state.records.remove(&fd);
        let code = unsafe { libc::close(fd) };
        if code != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    pub fn duplicate(self: &Arc<Self>, lease: &DescriptorLease) -> Result<DescriptorLease> {
        if !Arc::ptr_eq(self, &lease.table) {
            return Err("foreign descriptor authority".into());
        }
        self.authorize(lease.fd, Some(lease.generation))?;
        let state = self.state.lock().map_err(|_| "descriptor table poisoned")?;
        let (record, _pin) = Self::checked(&state, lease.fd, Some(lease.generation))?;
        if matches!(
            record.kind,
            DescriptorKind::DirectoryWatch | DescriptorKind::EventQueue
        ) {
            return Err("private watch descriptors cannot become caller grants".into());
        }
        let fd = raw(unsafe { libc::fcntl(lease.fd, libc::F_DUPFD_CLOEXEC, 0) })?;
        drop(state);
        self.issue_caller(fd, record)
    }
}
impl DescriptorLease {
    fn io_pin(&self) -> Result<(Record, OwnedFd)> {
        self.table.authorize(self.fd, Some(self.generation))?;
        let state = self
            .table
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?;
        let (record, _owner) = DescriptorTable::checked(&state, self.fd, Some(self.generation))?;
        if matches!(
            record.kind,
            DescriptorKind::DirectoryWatch | DescriptorKind::EventQueue
        ) {
            return Err("watch descriptors do not grant stream IO".into());
        }
        let minimum = state
            .records
            .keys()
            .next_back()
            .copied()
            .unwrap_or(2)
            .checked_add(1)
            .ok_or("descriptor number exhausted")?;
        let pin = raw(unsafe { libc::fcntl(self.fd, libc::F_DUPFD_CLOEXEC, minimum) })?;
        Ok((record, pin))
    }
    fn after_io(&self) -> Result<()> {
        self.table.authorize(self.fd, Some(self.generation))?;
        let state = self
            .table
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?;
        DescriptorTable::checked(&state, self.fd, Some(self.generation))?;
        Ok(())
    }
    pub fn raw(&self) -> i32 {
        self.fd
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn kind(&self) -> Result<DescriptorKind> {
        self.table.authorize(self.fd, Some(self.generation))?;
        let state = self
            .table
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?;
        Ok(
            DescriptorTable::checked(&state, self.fd, Some(self.generation))?
                .0
                .kind,
        )
    }
    pub fn regular_path(&self) -> Result<PathBuf> {
        self.table.authorize(self.fd, Some(self.generation))?;
        let state = self
            .table
            .state
            .lock()
            .map_err(|_| "descriptor table poisoned")?;
        let (record, _) = DescriptorTable::checked(&state, self.fd, Some(self.generation))?;
        if record.kind != DescriptorKind::Regular || record.root.is_none() {
            return Err("descriptor has no regular root provenance".into());
        }
        record
            .path
            .ok_or_else(|| "regular descriptor path absent".into())
    }
    pub fn read(&self, count: usize) -> Result<Vec<u8>> {
        if count > 32_000_001 {
            return Err("descriptor read exceeds bound".into());
        }
        let (record, pin) = self.io_pin()?;
        if record.access == FileAccess::Write {
            return Err("descriptor is not readable".into());
        }
        let mut bytes = vec![0; count];
        // May block for an explicitly issued child pipe. Only this retained OFD,
        // never the central table mutex, survives the kernel operation.
        let n = unsafe { libc::read(pin.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        bytes.truncate(n as usize);
        self.after_io()?;
        Ok(bytes)
    }
    pub fn write(&self, bytes: &[u8]) -> Result<usize> {
        if bytes.len() > 32_000_000 {
            return Err("descriptor write exceeds bound".into());
        }
        let (record, pin) = self.io_pin()?;
        if record.access == FileAccess::Read {
            return Err("descriptor is not writable".into());
        }
        let n = unsafe { libc::write(pin.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        self.after_io()?;
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    };
    struct Life {
        alive: AtomicBool,
        calls: AtomicUsize,
        table: Mutex<Option<Weak<DescriptorTable>>>,
    }
    impl Life {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                alive: AtomicBool::new(true),
                calls: AtomicUsize::new(0),
                table: Mutex::new(None),
            })
        }
    }
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<()> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(table) = self.table.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                if table.state.try_lock().is_err() {
                    return Err("guardian called with descriptor mutex held".into());
                }
            }
            if self.is_alive() {
                Ok(())
            } else {
                Err("own guardian retired".into())
            }
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .map(PathBuf::from)
                .expect("explicit owned fixture parent required");
            let path = parent.join(format!(
                "descriptor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn root(&self) -> OwnedRoot {
            OwnedRoot::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn regular_grants_come_only_from_owned_traversal_and_recheck_hardlinks() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let table = DescriptorTable::new();
        let life = Life::new();
        let path = fixture.0.join("own");
        let file = table
            .regular_owned(
                &root,
                &path,
                FileAccess::ReadWrite,
                FileCreation::CreateNew,
                life.clone(),
            )
            .unwrap();
        assert_eq!(file.lease().write(b"own-data").unwrap(), 8);
        assert_eq!(file.lease().regular_path().unwrap(), path);
        assert!(table
            .regular(
                &root,
                &path,
                FileAccess::Read,
                FileCreation::CreateNew,
                CloseOwner::Caller,
                life.clone()
            )
            .is_err());
        assert!(table
            .regular(
                &root,
                &fixture.0.join("../outside"),
                FileAccess::Read,
                FileCreation::Existing,
                CloseOwner::Caller,
                life.clone()
            )
            .is_err());
        assert!(table
            .regular(
                &root,
                Path::new("/unowned-no-filesystem-query"),
                FileAccess::Read,
                FileCreation::Existing,
                CloseOwner::Caller,
                life.clone()
            )
            .is_err());
        std::os::unix::fs::symlink(&path, fixture.0.join("linked")).unwrap();
        assert!(table
            .regular(
                &root,
                &fixture.0.join("linked"),
                FileAccess::Read,
                FileCreation::Existing,
                CloseOwner::Caller,
                life.clone()
            )
            .is_err());
        fs::hard_link(&path, fixture.0.join("hard")).unwrap();
        assert!(file.lease().write(b"denied").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"own-data");
        fs::remove_file(fixture.0.join("hard")).unwrap();
        assert_eq!(file.lease().kind().unwrap(), DescriptorKind::Regular);
    }
    #[test]
    fn caller_close_retires_generation_and_sdk_borrow_cannot_close_owner() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let table = DescriptorTable::new();
        let life = Life::new();
        let path = fixture.0.join("own");
        fs::write(&path, b"data").unwrap();
        let sdk = table
            .regular_owned(
                &root,
                &path,
                FileAccess::Read,
                FileCreation::Existing,
                life.clone(),
            )
            .unwrap();
        let borrowed = table.borrow(sdk.raw()).unwrap();
        assert!(table.close(sdk.raw()).is_err());
        assert_eq!(borrowed.read(4).unwrap(), b"data");
        let caller = table.duplicate(&borrowed).unwrap();
        let old_generation = caller.generation();
        table.close(caller.raw()).unwrap();
        assert!(caller.kind().is_err());
        assert!(table.close(caller.raw()).is_err());
        let next = table
            .regular(
                &root,
                &path,
                FileAccess::Read,
                FileCreation::Existing,
                CloseOwner::Caller,
                life,
            )
            .unwrap();
        assert!(next.generation() > old_generation);
        assert!(caller.kind().is_err());
        table.close(next.raw()).unwrap();
        drop(sdk);
        assert!(borrowed.kind().is_err());
    }
    #[test]
    fn actual_pipes_are_nonblocking_bounded_and_cannot_be_reclassified_as_regular() {
        let table = DescriptorTable::new();
        let life = Life::new();
        let (read, write) = table.pipe_owned(life).unwrap();
        for pipe in [&read, &write] {
            assert_ne!(
                unsafe { libc::fcntl(pipe.raw(), libc::F_GETFL) } & libc::O_NONBLOCK,
                0
            );
            assert_ne!(
                unsafe { libc::fcntl(pipe.raw(), libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
            assert!(pipe.lease().regular_path().is_err());
            assert!(table.close(pipe.raw()).is_err());
        }
        assert!(read.lease().read(1).is_err());
        assert!(read.lease().write(b"no").is_err());
        assert!(write.lease().read(1).is_err());
        assert_eq!(write.lease().write(b"ready").unwrap(), 5);
        assert_eq!(read.lease().read(5).unwrap(), b"ready");
        let block = [0u8; 4096];
        let mut full = false;
        for _ in 0..2048 {
            if write.lease().write(&block).is_err() {
                full = true;
                break;
            }
        }
        assert!(
            full,
            "owned pipe must report backpressure instead of blocking"
        );
        let flags = unsafe { libc::fcntl(read.raw(), libc::F_GETFL) };
        assert_eq!(
            unsafe { libc::fcntl(read.raw(), libc::F_SETFL, flags & !libc::O_NONBLOCK) },
            0
        );
        assert!(read.lease().read(1).is_err());
        assert_eq!(unsafe { libc::fcntl(read.raw(), libc::F_SETFL, flags) }, 0);
    }
    #[test]
    fn guardian_callbacks_do_not_hold_table_mutex_and_retirement_cannot_rearm_lease() {
        let table = DescriptorTable::new();
        let life = Life::new();
        *life.table.lock().unwrap() = Some(Arc::downgrade(&table));
        let (read, write) = table.pipe_owned(life.clone()).unwrap();
        write.lease().write(b"one").unwrap();
        assert_eq!(read.lease().read(3).unwrap(), b"one");
        let copy = table.duplicate(&read.lease()).unwrap();
        table.close(copy.raw()).unwrap();
        assert!(life.calls.load(Ordering::Relaxed) > 8);
        life.alive.store(false, Ordering::Release);
        assert!(read.lease().read(1).is_err());
        assert!(write.lease().write(b"late").is_err());
        assert!(table.borrow(read.raw()).is_err());
        assert!(table.duplicate(&read.lease()).is_err());
        life.alive.store(true, Ordering::Release);
        assert!(read.lease().kind().is_err());
        assert!(write.lease().write(b"revive").is_err());
    }
    #[test]
    fn reentrant_close_and_borrow_cannot_consume_a_new_descriptor_generation() {
        use std::sync::{atomic::AtomicI32, OnceLock};
        struct Reentry {
            armed: AtomicBool,
            closing: AtomicI32,
            table: OnceLock<Weak<DescriptorTable>>,
            donor: Mutex<Option<DescriptorLease>>,
            replacement: Mutex<Option<DescriptorLease>>,
        }
        impl AliveLease for Reentry {
            fn is_alive(&self) -> bool {
                true
            }
            fn before_effect(&self) -> Result<()> {
                if self.armed.swap(false, Ordering::AcqRel) {
                    let table = self.table.get().unwrap().upgrade().unwrap();
                    let closing = self.closing.load(Ordering::Acquire);
                    table.close(closing)?;
                    let replacement =
                        table.duplicate(self.donor.lock().unwrap().as_ref().unwrap())?;
                    assert_eq!(replacement.raw(), closing, "own released descriptor reused");
                    *self.replacement.lock().unwrap() = Some(replacement);
                }
                Ok(())
            }
        }
        for borrow in [false, true] {
            let table = DescriptorTable::new();
            let life = Arc::new(Reentry {
                armed: AtomicBool::new(false),
                closing: AtomicI32::new(-1),
                table: OnceLock::new(),
                donor: Mutex::new(None),
                replacement: Mutex::new(None),
            });
            assert!(life.table.set(Arc::downgrade(&table)).is_ok());
            let (read, _write) = table.pipe_owned(life.clone()).unwrap();
            let original = table.duplicate(&read.lease()).unwrap();
            life.closing.store(original.raw(), Ordering::Release);
            *life.donor.lock().unwrap() = Some(read.lease());
            life.armed.store(true, Ordering::Release);
            if borrow {
                assert!(table.borrow(original.raw()).is_err());
            } else {
                assert!(table.close(original.raw()).is_err());
            }
            let replacement = life.replacement.lock().unwrap().take().unwrap();
            assert!(replacement.generation() > original.generation());
            assert_eq!(replacement.kind().unwrap(), DescriptorKind::PipeRead);
            assert!(original.kind().is_err());
            table.close(replacement.raw()).unwrap();
        }
    }
    #[test]
    fn process_stdio_is_blocking_pinned_and_sdk_descriptors_are_not_inheritable() {
        let table = DescriptorTable::new();
        let life = Life::new();
        let (_sdk_read, _sdk_write) = table.pipe_owned(life.clone()).unwrap();
        let (read, write) = table.pipe_for_process(life.clone()).unwrap();
        let null = table.null_for_process(libc::O_RDONLY, life).unwrap();
        for fd in [&read, &write] {
            assert_eq!(
                unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK,
                0
            );
            assert_ne!(
                unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
        }
        let reader = table.borrow(read.as_raw_fd()).unwrap();
        let writer = table.borrow(write.as_raw_fd()).unwrap();
        assert_eq!(writer.write(b"own").unwrap(), 3);
        assert_eq!(reader.read(3).unwrap(), b"own");
        let snapshot = table.stdio_snapshot().unwrap();
        assert_eq!(snapshot.rows().len(), 3);
        assert_eq!(snapshot.pin_fds().len(), 3);
        for row in snapshot.rows() {
            let pin = snapshot.pin_for(row.fd, row.generation).unwrap();
            assert_ne!(pin, row.fd);
            assert_eq!(
                identity(pin).unwrap(),
                (row.identity.0, row.identity.1, row.identity.3)
            );
            assert!(table.borrow(pin).is_err());
            assert!(snapshot.pin_for(row.fd, row.generation + 1).is_err());
        }
        snapshot.verify().unwrap();
        // Logical close retires future admission. Once the bounded transfer/ACK
        // scope drops its pins, no private writer delays the actual pipe EOF.
        let writer_fd = write.into_raw_fd();
        table.close(writer_fd).unwrap();
        assert!(snapshot.verify().is_err());
        snapshot.verify_transferred().unwrap();
        drop(snapshot);
        assert!(reader.read(1).unwrap().is_empty());
        drop(null);
        assert_eq!(table.stdio_snapshot().unwrap().rows().len(), 1);
    }
    #[test]
    fn parent_only_process_reader_is_nonblocking_and_never_transferred() {
        let table = DescriptorTable::new();
        let (read, write) = table.process_pipe(Life::new(), true).unwrap();
        let reader = table.borrow(read.as_raw_fd()).unwrap();
        let writer = table.borrow(write.as_raw_fd()).unwrap();
        assert!(reader.read(1).is_err(), "empty read must not block");
        let snapshot = table.stdio_snapshot().unwrap();
        assert_eq!(snapshot.rows().len(), 1);
        assert_eq!(snapshot.rows()[0].fd, write.as_raw_fd());
        assert!(snapshot.pin_for(reader.raw(), reader.generation()).is_err());
        assert_eq!(writer.write(b"own").unwrap(), 3);
        assert_eq!(reader.read(3).unwrap(), b"own");
        drop(snapshot);
        table.close(write.into_raw_fd()).unwrap();
        assert!(reader.read(1).unwrap().is_empty());
    }
    #[test]
    fn fresh_factory_reissues_dropped_caller_numbers_without_reviving_old_lease() {
        let table = DescriptorTable::new();
        let life = Life::new();
        let (read, write) = table.pipe_for_process(life.clone()).unwrap();
        let original = table.borrow(read.as_raw_fd()).unwrap();
        let number = read.as_raw_fd();
        drop(read);
        let (next_read, _next_write) = table.pipe_for_process(life.clone()).unwrap();
        assert_eq!(next_read.as_raw_fd(), number);
        let current = table.borrow(number).unwrap();
        assert!(current.generation() > original.generation());
        assert!(original.kind().is_err());
        assert!(table
            .null_for_process(libc::O_WRONLY | libc::O_CREAT, life.clone())
            .is_err());
        assert!(table
            .null_for_process(libc::O_WRONLY | libc::O_TRUNC, life)
            .is_err());
        drop(write);
    }
    #[test]
    fn closed_sdk_pipe_writes_fail_without_changing_process_signal_disposition() {
        let table = DescriptorTable::new();
        let life = Life::new();
        let (read, write) = table.pipe_owned(life).unwrap();
        drop(read);
        assert!(write.lease().write(b"no-reader").is_err());
    }
    #[test]
    fn transferred_verification_rechecks_retirement_after_first_and_later_callbacks() {
        use std::sync::atomic::AtomicI32;
        struct Retire {
            alive: AtomicBool,
            remaining: AtomicUsize,
            table: Weak<DescriptorTable>,
            target: AtomicI32,
        }
        impl AliveLease for Retire {
            fn is_alive(&self) -> bool {
                self.alive.load(Ordering::Acquire)
            }
            fn before_effect(&self) -> Result<()> {
                if self.remaining.load(Ordering::Acquire) > 0
                    && self.remaining.fetch_sub(1, Ordering::AcqRel) == 1
                {
                    self.alive.store(false, Ordering::Release);
                    assert!(self
                        .table
                        .upgrade()
                        .unwrap()
                        .borrow(self.target.load(Ordering::Acquire))
                        .is_err());
                    self.alive.store(true, Ordering::Release);
                }
                Ok(())
            }
        }
        for trigger in [1, 2] {
            let table = DescriptorTable::new();
            let life = Arc::new(Retire {
                alive: AtomicBool::new(true),
                remaining: AtomicUsize::new(0),
                table: Arc::downgrade(&table),
                target: AtomicI32::new(-1),
            });
            let (read, _write) = table.pipe_for_process(life.clone()).unwrap();
            life.target.store(read.as_raw_fd(), Ordering::Release);
            let snapshot = table.stdio_snapshot().unwrap();
            assert_eq!(snapshot.rows().len(), 2);
            life.remaining.store(trigger, Ordering::Release);
            assert!(snapshot.verify_transferred().is_err(), "retired first row must be refused on its first admission, including later-row reentry");
            assert!(snapshot.verify_transferred().is_err());
            assert!(table.borrow(read.as_raw_fd()).is_err());
        }
    }
}
