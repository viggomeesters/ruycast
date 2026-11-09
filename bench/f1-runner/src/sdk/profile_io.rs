//! Owned low-level profile IO, not another ConfigStore implementation. The
//! source route gate remains closed until actual native/observer hooks qualify.
use super::{
    descriptors::{DescriptorKind, DescriptorTable, SdkDescriptor},
    owned::{AliveLease, FileAccess, FileCreation, OwnedRoot, Result},
};
use abi::{Bytes, Handle, Reply, Request};
pub use ruycast_native::profile_io_abi as abi;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, OsStr},
    fs::File,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

const LIMIT: usize = 32_000_000;
#[derive(Clone)]
enum Record {
    Profile(PathBuf),
    Lock(Arc<SdkDescriptor>),
    Watch(Arc<Watch>),
    NativeInputPublisher(Arc<super::core_preentry::NativeInputAdmission>),
}
const INPUT_OWNER_PUBLISH:u32=0x1001;
const INPUT_OWNER_REVOKE:u32=0x1002;
/// Transport only. Only an actual matching Broker record grants this species;
/// ordinary file/path bind cannot create it. No descriptor integer is invented.
#[repr(C)]
pub struct NativeInputPublisherCapability { opaque:[u64;2] }
impl NativeInputPublisherCapability {pub fn words(&self)->[u64;2]{self.opaque}}
#[derive(Clone,Copy)]
#[repr(C)]
pub struct NativeInputOwnerNoticeV1 {
    pub version:u32,pub bytes:u32,pub role:u32,pub ticket_kind:u32,
    pub userdata:usize,pub owner_epoch:u64,pub ticket_slot:u64,pub ticket_generation:u64,
}
impl NativeInputOwnerNoticeV1 {
    pub(crate) fn check(&self,revoke:bool)->Result<()> {
        if self.version!=1 || self.bytes!=48 || !matches!((self.role,self.ticket_kind),(1,1)|(2,2)|(3,3))
            || self.userdata==0 || self.owner_epoch==0
            || if revoke {self.ticket_slot!=u64::from(self.role) || self.ticket_generation==0}
                else {self.ticket_slot!=0 || self.ticket_generation!=0} {
            return Err("Native Input owner notice scalar shape differs".into());
        }
        Ok(())
    }
}
/// Reply DATA, never a Broker/File Handle or an allocation grant.
#[derive(Clone,Copy)]
#[repr(C)]
pub struct NativeInputOwnerKeyV1 {pub slot:u64,pub generation:u64}
pub(crate) fn issue_native_input_publisher(authority:Arc<super::core_preentry::NativeInputAdmission>)->Result<NativeInputPublisherCapability> {
    let broker=BROKER.get().ok_or("Native Input profile broker is not published")?;
    broker.before()?;
    authority.verify_publisher_issuer(&broker.life,&broker.descriptors)?;
    let handle=broker.issue(Record::NativeInputPublisher(authority))?;
    Ok(NativeInputPublisherCapability{opaque:[handle.slot,handle.generation]})
}
pub(crate) fn qualified_profile_provider(life:&Arc<dyn AliveLease>,descriptors:&Arc<DescriptorTable>)->Result<*const abi::Api> {
    let broker=BROKER.get().ok_or("Host profile broker absent")?;
    if !Arc::ptr_eq(life,&broker.life) || !Arc::ptr_eq(descriptors,&broker.descriptors) {
        return Err("Host profile broker issuer differs".into());
    }
    broker.before()?;
    if PHASE.load(Ordering::Acquire)!=1 {return Err("Host profile broker retired".into());}
    Ok(&API)
}
pub(crate) fn input_publisher_current(
    capability:&NativeInputPublisherCapability,authority:&Arc<super::core_preentry::NativeInputAdmission>,
    life:&Arc<dyn AliveLease>,descriptors:&Arc<DescriptorTable>,
)->Result<()> {
    let broker=BROKER.get().ok_or("Native Input broker absent")?;
    if !Arc::ptr_eq(life,&broker.life) || !Arc::ptr_eq(descriptors,&broker.descriptors) || PHASE.load(Ordering::Acquire)!=1 {
        return Err("Native Input capability broker retired or differs".into());
    }
    let [slot,generation]=capability.words();
    let current=input_publisher_record(broker.record(Handle{slot,generation})?)?;
    if !Arc::ptr_eq(authority,&current) {return Err("Native Input publisher capability was replaced".into());}
    Ok(())
}
pub(crate) fn verify_native_input_publisher(
    capability:&NativeInputPublisherCapability,life:&Arc<dyn AliveLease>,descriptors:&Arc<DescriptorTable>,
)->Result<Arc<super::core_preentry::NativeInputAdmission>> {
    let broker=BROKER.get().ok_or("Native Input broker absent")?;
    if !Arc::ptr_eq(life,&broker.life) || !Arc::ptr_eq(descriptors,&broker.descriptors) {
        return Err("Native Input capability broker issuer differs".into());
    }
    let [slot,generation]=capability.words();let handle=Handle{slot,generation};
    let authority=input_publisher_record(broker.before_handle(handle)?)?;
    authority.verify_publisher_issuer(life,descriptors)?;
    let current=input_publisher_record(broker.record(handle)?)?;
    if PHASE.load(Ordering::Acquire)!=1 || !Arc::ptr_eq(&authority,&current) {
        return Err("Native Input publisher capability was retired or replaced".into());
    }
    Ok(authority)
}
fn input_publisher_record(record:Record)->Result<Arc<super::core_preentry::NativeInputAdmission>> {
    match record {Record::NativeInputPublisher(authority)=>Ok(authority),_=>Err("Native Input operation requires its publisher capability".into())}
}
struct Watch {
    profile: Handle,
    queue: SdkDescriptor,
    directories: Mutex<Vec<(PathBuf, SdkDescriptor)>>,
}
fn watch_event(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}
impl Watch {
    // Cleanup only: wake a known own queue even after guardian/profile retirement.
    fn wake(&self) -> Result<()> {
        let event = watch_event(1, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
        if unsafe {
            libc::kevent(
                self.queue.raw(),
                &event,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}
// Reserve the bounded event reply before consuming a kernel event. Any failure
// or unwind returns this unpublished buffer instead of leaking its capacity.
struct PollBuffer {
    owner: Handle,
    reply: Option<Reply>,
}
impl Drop for PollBuffer {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            return_buffer(self.owner, reply.buffer.token);
        }
    }
}
struct BufferRecord {
    owner: Handle,
    bytes: Box<[u8]>,
}
struct State {
    next_slot: u64,
    next_generation: u64,
    next_buffer: u64,
    free: BTreeSet<u64>,
    records: BTreeMap<u64, (u64, Record)>,
    buffers: BTreeMap<u64, BufferRecord>,
    buffer_bytes: usize,
}
pub struct Broker {
    roots: Vec<OwnedRoot>,
    life: Arc<dyn AliveLease>,
    descriptors: Arc<DescriptorTable>,
    state: Mutex<State>,
    in_flight: AtomicUsize,
}
// The existing descriptor guardian also binds the profile generation: a
// callback may retire that handle without retiring the shared global guardian.
// Weak ownership avoids a Broker -> table -> descriptor -> Broker cycle.
struct ProfileLife {
    broker: Weak<Broker>,
    handle: Handle,
}
impl AliveLease for ProfileLife {
    fn is_alive(&self) -> bool {
        let Some(broker) = self.broker.upgrade() else {
            return false;
        };
        PHASE.load(Ordering::Acquire) == 1
            && broker.life.is_alive()
            && broker.authority_record(self.handle).is_ok()
            && PHASE.load(Ordering::Acquire) == 1
    }
    fn before_effect(&self) -> Result<()> {
        self.broker
            .upgrade()
            .ok_or("profile IO broker retired")?
            .before_handle(self.handle)
            .map(|_| ())
    }
}
static BROKER: OnceLock<Arc<Broker>> = OnceLock::new();
// 0 unpublished, 1 published, 2 permanently retired. Cleanup may return only
// previously issued handles/buffers after retirement; it grants no new IO.
static PHASE: AtomicU8 = AtomicU8::new(0);
thread_local! { static ENTERED: Cell<bool> = const { Cell::new(false) }; }
struct Entry;
impl Entry {
    fn enter() -> Option<Self> {
        ENTERED
            .try_with(|entered| {
                if entered.replace(true) {
                    None
                } else {
                    Some(Self)
                }
            })
            .ok()
            .flatten()
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        let _ = ENTERED.try_with(|entered| entered.set(false));
    }
}
impl Broker {
    pub fn new(
        roots: Vec<OwnedRoot>,
        life: Arc<dyn AliveLease>,
        descriptors: Arc<DescriptorTable>,
    ) -> Result<Arc<Self>> {
        if roots.is_empty() || roots.len() > 4 {
            return Err("explicit owned profile roots required".into());
        }
        if !life.is_alive() {
            return Err("profile IO guardian retired".into());
        }
        life.before_effect()?;
        Ok(Arc::new(Self {
            roots,
            life,
            descriptors,
            state: Mutex::new(State {
                next_slot: 1,
                next_generation: 1,
                next_buffer: 1,
                free: BTreeSet::new(),
                records: BTreeMap::new(),
                buffers: BTreeMap::new(),
                buffer_bytes: 0,
            }),
            in_flight: AtomicUsize::new(0),
        }))
    }
    fn before(&self) -> Result<()> {
        if PHASE.load(Ordering::Acquire) != 1 {
            return Err("profile IO authority retired".into());
        }
        let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.check_life()));
        match guarded {
            Ok(result) => result,
            Err(payload) => {
                retire();
                std::panic::resume_unwind(payload)
            }
        }
    }
    fn check_life(&self) -> Result<()> {
        if !self.life.is_alive() {
            retire();
            return Err("profile IO guardian retired".into());
        }
        if let Err(error) = self.life.before_effect() {
            retire();
            return Err(error);
        }
        if !self.life.is_alive() {
            retire();
            return Err("profile IO guardian retired during callback".into());
        }
        if PHASE.load(Ordering::Acquire) != 1 {
            return Err("profile IO retired during callback".into());
        }
        Ok(())
    }
    fn owner(&self, path: &Path) -> Result<OwnedRoot> {
        if !path.is_absolute()
            || path
                .components()
                .any(|p| matches!(p, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("invalid owned profile path".into());
        }
        self.roots
            .iter()
            .filter(|r| path.strip_prefix(r.path()).is_ok())
            .max_by_key(|r| r.path().as_os_str().len())
            .cloned()
            .ok_or_else(|| "profile path outside explicit owned roots".into())
    }
    fn record(&self, handle: Handle) -> Result<Record> {
        if handle.slot == 0 || handle.generation == 0 {
            return Err("invalid profile IO handle".into());
        }
        let state = self.state.lock().map_err(|_| "profile IO state poisoned")?;
        state
            .records
            .get(&handle.slot)
            .filter(|(g, _)| *g == handle.generation)
            .map(|(_, r)| r.clone())
            .ok_or_else(|| "retired profile IO generation".into())
    }
    fn authority_record(&self, handle: Handle) -> Result<Record> {
        let record = self.record(handle)?;
        if let Record::Watch(watch) = &record {
            if !matches!(self.record(watch.profile)?, Record::Profile(_)) {
                return Err("watch profile authority retired".into());
            }
        }
        Ok(record)
    }
    fn before_handle(&self, handle: Handle) -> Result<Record> {
        self.before()?;
        self.authority_record(handle)
    }
    fn handle_life(&self, handle: Handle) -> Result<Arc<dyn AliveLease>> {
        let broker = BROKER
            .get()
            .filter(|broker| std::ptr::eq(Arc::as_ptr(broker), self))
            .ok_or("profile IO broker is not published")?;
        self.authority_record(handle)?;
        Ok(Arc::new(ProfileLife {
            broker: Arc::downgrade(broker),
            handle,
        }))
    }
    fn issue(&self, record: Record) -> Result<Handle> {
        let mut state = self.state.lock().map_err(|_| "profile IO state poisoned")?;
        if PHASE.load(Ordering::Acquire) != 1 {
            return Err("profile IO retired before handle issue".into());
        }
        if state.records.len() >= 256 {
            return Err("profile IO handle bound exceeded".into());
        }
        let generation = state.next_generation;
        state.next_generation = generation
            .checked_add(1)
            .ok_or("profile IO generation exhausted")?;
        let slot = if let Some(slot) = state.free.pop_first() {
            slot
        } else {
            let slot = state.next_slot;
            state.next_slot = slot.checked_add(1).ok_or("profile IO slots exhausted")?;
            slot
        };
        state.records.insert(slot, (generation, record));
        Ok(Handle { slot, generation })
    }
    fn buffer(
        &self,
        owner: Handle,
        bytes: Vec<u8>,
        status: i32,
        metadata: abi::Metadata,
    ) -> Result<Reply> {
        let mut state = self.state.lock().map_err(|_| "profile IO state poisoned")?;
        if status == abi::OK
            && (PHASE.load(Ordering::Acquire) != 1
                || state.records.get(&owner.slot).map(|(g, _)| *g) != Some(owner.generation))
        {
            return Err("profile IO result handle retired before reply".into());
        }
        if state.buffers.len() >= 128
            || state
                .buffer_bytes
                .checked_add(bytes.len())
                .is_none_or(|n| n > 64_000_000)
        {
            return Err("profile IO reply buffer bound exceeded".into());
        }
        let token = state.next_buffer;
        state.next_buffer = token
            .checked_add(1)
            .ok_or("profile IO buffer tokens exhausted")?;
        let bytes = bytes.into_boxed_slice();
        let buffer = abi::Buffer {
            token,
            pointer: bytes.as_ptr(),
            length: bytes.len(),
        };
        state.buffer_bytes += bytes.len();
        state.buffers.insert(token, BufferRecord { owner, bytes });
        Ok(Reply {
            status,
            buffer,
            metadata,
            ..Reply::denied()
        })
    }
    fn parent(&self, path: &Path) -> Result<(File, CString)> {
        let root = self.owner(path)?;
        let parent = path.parent().ok_or("missing owned profile parent")?;
        let file = File::from(root.checked(parent)?);
        let info = file.metadata().map_err(|e| e.to_string())?;
        if !info.is_dir() {
            return Err("owned profile parent is not a directory".into());
        }
        let leaf = path.file_name().ok_or("missing profile filename")?;
        let leaf = CString::new(leaf.as_bytes()).map_err(|_| "NUL in profile filename")?;
        Ok((file, leaf))
    }
    fn stat(&self, path: &Path) -> Result<Option<abi::Metadata>> {
        let root = self.owner(path)?;
        let Some(file) = root.checked_optional(path)? else {
            return Ok(None);
        };
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), info.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let info = unsafe { info.assume_init() };
        Ok(Some(abi::Metadata {
            device: info.st_dev as u64,
            inode: info.st_ino,
            length: info.st_size.max(0) as u64,
            links: info.st_nlink as u64,
            mode: info.st_mode as u32,
            uid: info.st_uid,
            gid: info.st_gid,
            reserved: 0,
            modified_seconds: info.st_mtime,
            modified_nanoseconds: info.st_mtime_nsec,
            changed_seconds: info.st_ctime,
            changed_nanoseconds: info.st_ctime_nsec,
        }))
    }
    fn checked_watch(&self, handle: Handle) -> Result<Arc<Watch>> {
        let Record::Watch(watch) = self.before_handle(handle)? else {
            return Err("watch handle required".into());
        };
        if watch.queue.lease().kind()? != DescriptorKind::EventQueue {
            return Err("watch queue kind changed".into());
        }
        let Record::Watch(current) = self.authority_record(handle)? else {
            return Err("watch retired".into());
        };
        if !Arc::ptr_eq(&watch, &current) {
            return Err("watch generation changed".into());
        }
        Ok(watch)
    }
    fn watch_operation(
        &self,
        handle: Handle,
        request: Request,
        path: PathBuf,
        data: &[u8],
    ) -> Result<Reply> {
        if request.flags != 0 || !data.is_empty() {
            return Err("invalid watch request".into());
        }
        let watch = self.checked_watch(handle)?;
        match request.operation {
            abi::WATCH_DIRECTORY if request.limit == 0 => {
                if watch
                    .directories
                    .lock()
                    .map_err(|_| "watch directories poisoned")?
                    .len()
                    >= 2
                {
                    return Err("watch directory bound exceeded".into());
                }
                let root = self.owner(&path)?;
                let directory = self.descriptors.directory_for_watch(
                    &root,
                    &path,
                    self.handle_life(handle)?,
                )?;
                let current = self.checked_watch(handle)?;
                let mut directories = current
                    .directories
                    .lock()
                    .map_err(|_| "watch directories poisoned")?;
                if directories.len() >= 2 {
                    return Err("watch directory bound exceeded".into());
                }
                let event = watch_event(
                    directory.raw() as usize,
                    libc::EVFILT_VNODE,
                    libc::EV_ADD | libc::EV_CLEAR,
                    libc::NOTE_WRITE
                        | libc::NOTE_LINK
                        | libc::NOTE_RENAME
                        | libc::NOTE_DELETE
                        | libc::NOTE_ATTRIB
                        | libc::NOTE_REVOKE,
                );
                if unsafe {
                    libc::kevent(
                        current.queue.raw(),
                        &event,
                        1,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null(),
                    )
                } != 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                directories.push((path, directory));
                Ok(Reply {
                    status: abi::OK,
                    ..Reply::denied()
                })
            }
            abi::WATCH_POLL if request.limit <= 5_000 => {
                let Record::Profile(profile) = self.authority_record(watch.profile)? else {
                    return Err("watch profile missing".into());
                };
                if path != profile {
                    return Err("watch poll profile mismatch".into());
                }
                let mut pending = PollBuffer {
                    owner: handle,
                    reply: Some(self.buffer(
                        handle,
                        vec![0; 8],
                        abi::OK,
                        abi::Metadata::default(),
                    )?),
                };
                // No table/state/directory lock crosses the blocking kernel wait.
                let timeout = libc::timespec {
                    tv_sec: (request.limit / 1_000) as i64,
                    tv_nsec: ((request.limit % 1_000) * 1_000_000) as i64,
                };
                let mut event = watch_event(0, 0, 0, 0);
                let count = unsafe {
                    libc::kevent(
                        watch.queue.raw(),
                        std::ptr::null(),
                        0,
                        &mut event,
                        1,
                        &timeout,
                    )
                };
                if count < 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                self.checked_watch(handle)?;
                let (kind, flags) = if count == 0 {
                    (abi::WATCH_TIMEOUT, 0)
                } else if event.flags & libc::EV_ERROR != 0 {
                    return Err("watch kernel event error".into());
                } else if event.filter == libc::EVFILT_USER && event.ident == 1 {
                    (abi::WATCH_WOKEN, 0)
                } else if event.filter == libc::EVFILT_VNODE {
                    (abi::WATCH_CHANGED, event.fflags)
                } else {
                    return Err("unexpected watch event".into());
                };
                let mut payload = [0u8; 8];
                payload[..4].copy_from_slice(&kind.to_le_bytes());
                payload[4..].copy_from_slice(&flags.to_le_bytes());
                let reply = pending.reply.as_ref().ok_or("watch reply missing")?;
                let mut state = self.state.lock().map_err(|_| "profile IO state poisoned")?;
                if PHASE.load(Ordering::Acquire) != 1
                    || state.records.get(&handle.slot).map(|(g, _)| *g) != Some(handle.generation)
                    || state.records.get(&watch.profile.slot).map(|(g, _)| *g)
                        != Some(watch.profile.generation)
                {
                    return Err("watch authority retired before reply publication".into());
                }
                let buffer = state
                    .buffers
                    .get_mut(&reply.buffer.token)
                    .filter(|b| b.owner == handle)
                    .ok_or("watch reply retired before publication")?;
                buffer.bytes.copy_from_slice(&payload);
                drop(state);
                Ok(pending.reply.take().ok_or("watch reply missing")?)
            }
            _ => Err("unknown or malformed watch operation".into()),
        }
    }
    fn operation(
        &self,
        handle: Handle,
        request: Request,
        path: PathBuf,
        data: &[u8],
    ) -> Result<Reply> {
        if matches!(request.operation, abi::WATCH_DIRECTORY | abi::WATCH_POLL) {
            return self.watch_operation(handle, request, path, data);
        }
        let Record::Profile(profile) = self.before_handle(handle)? else {
            return Err("profile IO operation requires a profile handle".into());
        };
        let root = self.owner(&path)?;
        let mut reply = Reply {
            status: abi::OK,
            ..Reply::denied()
        };
        match request.operation {
            abi::WATCH_OPEN
                if request.flags == 0
                    && data.is_empty()
                    && request.limit == 0
                    && path == profile =>
            {
                // The profile itself must be resolvable from a retained ancestor
                // for replacement rebind; an exact old-inode root is insufficient.
                if root.path() == path {
                    return Err("watch requires explicit ancestor authority".into());
                }
                let queue = self.descriptors.event_queue(self.handle_life(handle)?)?;
                self.before_handle(handle)?;
                let event = watch_event(1, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0);
                if unsafe {
                    libc::kevent(
                        queue.raw(),
                        &event,
                        1,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null(),
                    )
                } != 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                reply.handle = self.issue(Record::Watch(Arc::new(Watch {
                    profile: handle,
                    queue,
                    directories: Mutex::new(vec![]),
                })))?;
            }
            abi::READ if request.flags == 0 && data.is_empty() && request.limit <= LIMIT as u64 => {
                // The central issuer validates actual regular-file ownership,
                // no-follow traversal and one link before returning any bytes.
                let bytes = if root.read_optional(&path, 0).is_ok_and(|v| v.is_none()) {
                    None
                } else {
                    let descriptor = self.descriptors.regular_owned(
                        &root,
                        &path,
                        FileAccess::Read,
                        FileCreation::Existing,
                        self.handle_life(handle)?,
                    )?;
                    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
                    if unsafe { libc::fstat(descriptor.raw(), info.as_mut_ptr()) } != 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    let info = unsafe { info.assume_init() };
                    if info.st_size < 0 || info.st_size as u64 > request.limit {
                        return Err("profile IO input exceeds limit".into());
                    }
                    let mut output = Vec::new();
                    loop {
                        let remaining = (request.limit as usize).saturating_sub(output.len());
                        let chunk = descriptor
                            .lease()
                            .read(remaining.saturating_add(1).min(16_384))?;
                        if chunk.is_empty() {
                            break;
                        }
                        output.extend_from_slice(&chunk);
                        if output.len() > request.limit as usize {
                            return Err("profile IO input grew beyond limit".into());
                        }
                    }
                    Some(output)
                };
                self.before_handle(handle)?;
                return match bytes {
                    Some(bytes) => self.buffer(handle, bytes, abi::OK, abi::Metadata::default()),
                    None => Ok(Reply {
                        status: abi::MISSING,
                        ..Reply::denied()
                    }),
                };
            }
            abi::STAT if request.flags == 0 && data.is_empty() => match self.stat(&path)? {
                Some(metadata) => reply.metadata = metadata,
                None => reply.status = abi::MISSING,
            },
            abi::MKDIR if request.flags & !abi::RECURSIVE == 0 && data.is_empty() => {
                root.mkdir(&path, request.flags & abi::RECURSIVE != 0)?;
            }
            abi::SECURE_DIRECTORY if request.flags == 0 && data.is_empty() && path == profile => {
                root.mkdir(&path, true)?;
                let directory = File::from(root.checked(&path)?);
                self.before_handle(handle)?;
                ruycast_native::private_storage::secure_directory_fd(&directory)?;
            }
            abi::WRITE if request.flags & !abi::REPLACE == 0 => {
                let (parent, leaf) = self.parent(&path)?;
                let outcome = ruycast_native::private_storage::write_outcome_at(
                    &parent,
                    OsStr::from_bytes(leaf.as_bytes()),
                    data,
                    request.flags & abi::REPLACE != 0,
                    |publish| {
                        self.before_handle(handle)?;
                        publish()?;
                        self.before_handle(handle)?;
                        Ok(())
                    },
                )?;
                let (identity, warning) = match outcome {
                    ruycast_native::private_storage::WriteOutcome::Durable(id) => (id, None),
                    ruycast_native::private_storage::WriteOutcome::Published {
                        identity,
                        error,
                    } => (identity, Some(error)),
                };
                let metadata = abi::Metadata {
                    device: identity.device,
                    inode: identity.inode,
                    ..abi::Metadata::default()
                };
                if let Some(warning) = warning {
                    // Rename already happened. A bounded warning buffer is optional;
                    // exhausting it cannot erase the factual publication identity.
                    return Ok(self
                        .buffer(handle, warning.into_bytes(), abi::PUBLISHED, metadata)
                        .unwrap_or(Reply {
                            status: abi::PUBLISHED,
                            metadata,
                            ..Reply::denied()
                        }));
                }
                return Ok(Reply {
                    status: abi::OK,
                    metadata,
                    ..Reply::denied()
                });
            }
            abi::REMOVE_OWNED if request.flags == 0 => {
                let metadata = self.stat(&path)?.ok_or("owned profile copy missing")?;
                if metadata.mode & libc::S_IFMT as u32 != libc::S_IFREG as u32
                    || metadata.links != 1
                    || metadata.device != request.expected_device
                    || metadata.inode != request.expected_inode
                    || root.read_optional(&path, LIMIT as u64)?.as_deref() != Some(data)
                {
                    return Err("owned profile copy changed".into());
                }
                let (parent, leaf) = self.parent(&path)?;
                self.before_handle(handle)?;
                // Directory replacement cannot trigger recursive deletion. Like
                // the original ConfigLock path this is not malicious-user CAS.
                if unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            abi::LOCK
                if request.flags == 0 && data.is_empty() && path == profile.join("config.lock") =>
            {
                let lock = self.descriptors.regular_owned(
                    &root,
                    &path,
                    FileAccess::ReadWrite,
                    FileCreation::CreateOrOpen,
                    self.handle_life(handle)?,
                )?;
                self.before_handle(handle)?;
                if unsafe { libc::flock(lock.raw(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    return Err("configuration is already locked".into());
                }
                reply.handle = self.issue(Record::Lock(Arc::new(lock)))?;
            }
            _ => return Err("unknown or malformed profile IO operation".into()),
        }
        if let Err(error) = self.before_handle(handle) {
            if reply.handle.slot != 0 {
                let _ = release_handle(reply.handle);
            }
            return Err(error);
        }
        if reply.handle.slot != 0 {
            if let Err(error) = self.authority_record(reply.handle) {
                let _ = release_handle(reply.handle);
                return Err(error);
            }
        }
        Ok(reply)
    }
}
pub fn publish(broker: Arc<Broker>) -> Result<()> {
    if PHASE
        .compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("profile IO publication is one-shot".into());
    }
    if BROKER.set(broker).is_err() {
        retire();
        return Err("profile IO already published".into());
    }
    PHASE
        .compare_exchange(3, 1, Ordering::Release, Ordering::Acquire)
        .map_err(|_| "profile IO retired during publication")?;
    Ok(())
}
pub fn retire() {
    if PHASE.swap(2, Ordering::AcqRel) != 1 {
        return;
    }
    let Some(broker) = BROKER.get() else { return };
    let watches: Vec<_> = {
        let state = broker.state.lock().unwrap_or_else(|error| error.into_inner());
        state.records.values().filter_map(|(_, record)| match record {
            Record::Watch(watch) => Some(watch.clone()),
            _ => None,
        }).collect()
    };
    // Wake issued queues outside the state lock. A pending poll still has
    // to pass the retired authority check before it can publish a result.
    for watch in watches {
        let _ = watch.wake();
    }
}
fn guarded(owner: Handle, f: impl FnOnce(&Broker) -> Result<Reply>) -> Reply {
    if PHASE.load(Ordering::Acquire) != 1 {
        return Reply::denied();
    }
    let Some(_entry) = Entry::enter() else {
        return Reply::denied();
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let broker = BROKER.get().ok_or("profile IO unpublished")?;
        broker
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < 16).then_some(current + 1)
            })
            .map_err(|_| "profile IO concurrency bound exceeded")?;
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _active = Active(&broker.in_flight);
        broker.before()?;
        f(broker)
    })) {
        Ok(Ok(reply)) => reply,
        Ok(Err(error)) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            BROKER
                .get()
                .and_then(|b| {
                    b.buffer(
                        owner,
                        error.into_bytes(),
                        abi::FAILED,
                        abi::Metadata::default(),
                    )
                    .ok()
                })
                .unwrap_or(Reply {
                    status: abi::FAILED,
                    ..Reply::denied()
                })
        }))
        .unwrap_or_else(|_| {
            retire();
            Reply::denied()
        }),
        Err(_) => {
            retire();
            Reply::denied()
        }
    }
}
unsafe fn bytes<'a>(input: Bytes, limit: usize) -> Result<&'a [u8]> {
    if input.length > limit || (input.length != 0 && input.pointer.is_null()) {
        return Err("invalid profile IO borrowed bytes".into());
    }
    if input.length == 0 {
        return Ok(&[]);
    }
    Ok(unsafe { std::slice::from_raw_parts(input.pointer, input.length) })
}
unsafe fn path(input: Bytes) -> Result<PathBuf> {
    let input = unsafe { bytes(input, 32_768)? };
    let text = std::str::from_utf8(input).map_err(|_| "profile IO path is not UTF8")?;
    if text.is_empty() || text.contains('\0') {
        return Err("invalid profile IO path".into());
    }
    Ok(PathBuf::from(text))
}
unsafe extern "C" fn bind(input: Bytes) -> Reply {
    guarded(Handle::default(), |broker| {
        let path = unsafe { path(input)? };
        broker.owner(&path)?;
        let handle = broker.issue(Record::Profile(path))?;
        if let Err(error) = broker.before_handle(handle) {
            let _ = release_handle(handle);
            return Err(error);
        }
        Ok(Reply {
            status: abi::OK,
            handle,
            ..Reply::denied()
        })
    })
}
unsafe extern "C" fn request(handle: Handle, input: Request) -> Reply {
    if matches!(input.operation,INPUT_OWNER_PUBLISH|INPUT_OWNER_REVOKE) {
        return guarded(handle,|broker| {
            let authority=input_publisher_record(broker.before_handle(handle)?)?;
            if input.flags!=0 || input.limit!=0 || input.expected_device!=0 || input.expected_inode!=0
                || input.path.length!=0 || !input.path.pointer.is_null() || input.data.length!=48 || input.data.pointer.is_null() {
                return Err("Native Input owner request shape differs".into());
            }
            authority.verify_publisher_issuer(&broker.life,&broker.descriptors)?;
            // The source-qualified unsafe C caller retains this complete scalar
            // payload for the request. Never read the userdata pointer itself.
            let notice=unsafe{input.data.pointer.cast::<NativeInputOwnerNoticeV1>().read_unaligned()};
            let revoke=input.operation==INPUT_OWNER_REVOKE;
            notice.check(revoke)?;
            let Some(ticket)=authority.owner_notice(notice,revoke)? else {
                return Ok(Reply{status:abi::OK,..Reply::denied()});
            };
            // A failed allocation/unwind must revoke the real pending ticket
            // before the Native owner can interpret failure and free its Box.
            struct Publication {ticket:Arc<super::core_preentry::NativeInputOwnerTicket>,complete:bool}
            impl Drop for Publication {
                fn drop(&mut self){if !self.complete {let _=self.ticket.revoke();}}
            }
            let mut publication=Publication{ticket,complete:false};
            let key=NativeInputOwnerKeyV1{slot:publication.ticket.slot(),generation:publication.ticket.generation()};
            let mut bytes=Vec::with_capacity(16);
            bytes.extend_from_slice(&key.slot.to_ne_bytes());bytes.extend_from_slice(&key.generation.to_ne_bytes());
            let reply=broker.buffer(handle,bytes,abi::OK,abi::Metadata::default())?;
            publication.complete=true;
            // buffer() leaves Reply.handle ZERO; the caller copies exactly16B
            // then returns the buffer against this same publisher capability.
            Ok(reply)
        });
    }
    if input.operation == abi::WATCH_WAKE {
        // Typed cleanup only: do not dereference input path/data or invoke a
        // retired guardian. No unknown or stale handle can create an eventqueue.
        return cleanup_wake(handle, input.flags, input.data.length);
    }
    guarded(handle, |broker| {
        let path = unsafe { path(input.path)? };
        let data = unsafe { bytes(input.data, LIMIT)? };
        broker.operation(handle, input, path, data)
    })
}
fn cleanup_wake(handle: Handle, flags: u32, data_length: usize) -> Reply {
    if !matches!(PHASE.load(Ordering::Acquire), 1 | 2) || flags != 0 || data_length != 0 {
        return Reply::denied();
    }
    match std::panic::catch_unwind(|| {
        let broker = BROKER.get().ok_or("profile broker absent")?;
        let Record::Watch(watch) = broker.record(handle)? else {
            return Err("watch cleanup handle required".to_string());
        };
        watch.wake()?;
        Ok(Reply {
            status: abi::OK,
            ..Reply::denied()
        })
    }) {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) => Reply::denied(),
        Err(_) => {
            retire();
            Reply::denied()
        }
    }
}
extern "C" fn release_handle(handle: Handle) -> i32 {
    if !matches!(PHASE.load(Ordering::Acquire), 1 | 2) {
        return abi::DENIED;
    }
    std::panic::catch_unwind(|| {
        let Some(broker) = BROKER.get() else {
            return abi::DENIED;
        };
        let removed = {
            let Ok(mut state) = broker.state.lock() else {
                return abi::DENIED;
            };
            if state.records.get(&handle.slot).map(|(g, _)| *g) != Some(handle.generation) {
                return abi::DENIED;
            }
            let removed = state.records.remove(&handle.slot);
            state.free.insert(handle.slot);
            removed
        };
        if let Some(Record::Lock(lock)) = removed.as_ref().map(|(_, r)| r) {
            unsafe {
                libc::flock(lock.raw(), libc::LOCK_UN);
            }
        }
        if let Some(Record::Watch(watch)) = removed.as_ref().map(|(_, r)| r) {
            let _ = watch.wake();
        }
        drop(removed);
        abi::OK
    })
    .unwrap_or_else(|_| {
        retire();
        abi::DENIED
    })
}
extern "C" fn return_buffer(owner: Handle, token: u64) -> i32 {
    if !matches!(PHASE.load(Ordering::Acquire), 1 | 2) {
        return abi::DENIED;
    }
    std::panic::catch_unwind(|| {
        let Some(broker) = BROKER.get() else {
            return abi::DENIED;
        };
        let Ok(mut state) = broker.state.lock() else {
            return abi::DENIED;
        };
        if state.buffers.get(&token).map(|b| b.owner) != Some(owner) {
            return abi::DENIED;
        }
        let buffer = state.buffers.remove(&token).expect("checked buffer");
        state.buffer_bytes -= buffer.bytes.len();
        drop(state);
        drop(buffer);
        abi::OK
    })
    .unwrap_or_else(|_| {
        retire();
        abi::DENIED
    })
}
#[used]
#[export_name = "rcf1_profile_io_api_v1"]
static API: abi::Api = abi::Api {
    version: abi::VERSION,
    bytes: std::mem::size_of::<abi::Api>() as u32,
    reserved: [0; 2],
    bind,
    request,
    release_handle,
    return_buffer,
};
/// Return only a published, live owned table. Its address is not full SDK scope
/// admission; the caller must qualify the real image/function roster first.
#[no_mangle]
pub extern "C" fn rcf1_profile_io_v1() -> *const abi::Api {
    if PHASE.load(Ordering::Acquire) != 1 {
        return std::ptr::null();
    }
    let Some(_entry) = Entry::enter() else {
        return std::ptr::null();
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        BROKER
            .get()
            .filter(|b| b.before().is_ok())
            .map_or(std::ptr::null(), |_| &API)
    })) {
        Ok(api) => api,
        Err(_) => {
            retire();
            std::ptr::null()
        }
    }
}
#[cfg(test)]
mod input_publisher_boundary_tests {
    use super::*;
    #[test]
    fn owned_profile_observer_recovers_selector_and_retires_issued_watches() {
        use ruycast_native::{control::HostEvent, observe::ConfigurationObserver, profile::ProfilePaths};
        use std::time::Duration;

        let fixture = crate::workspace::tests::Fixture::new();
        let home = fixture.root.join("home");
        let profile = home.join("profile");
        std::fs::create_dir(&profile).unwrap();
        let broker = Broker::new(
            vec![OwnedRoot::open(&home).unwrap()],
            fixture.life.clone(),
            DescriptorTable::new(),
        ).unwrap();
        publish(broker.clone()).unwrap();

        let selector = profile.join("config-location.json");
        let selected = home.join("selected");
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(&selector, br#"{"directory":"~synthetic_account/config"}"#).unwrap();
        // The test owns the actual broker and its static ABI table in this image.
        let paths = unsafe {
            ProfilePaths::new(profile.clone(), home, None).with_pinned_io(&API)
        }.unwrap();
        assert!(paths.config_file().unwrap_err().contains("owned profile"));
        let (events, received) = std::sync::mpsc::sync_channel(2);
        let mut observer = ConfigurationObserver::start(paths.clone(), events).unwrap();
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)).unwrap(), HostEvent::ConfigurationChanged));
        observer.acknowledge();
        let replacement = profile.join("selector-replacement.json");
        std::fs::write(
            &replacement,
            serde_json::to_vec(&serde_json::json!({"directory": selected})).unwrap(),
        ).unwrap();
        std::fs::rename(replacement, selector).unwrap();
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)).unwrap(), HostEvent::ConfigurationChanged));
        assert_eq!(paths.config_file().unwrap(), selected.join("config.json"));

        let path = Bytes::borrowed(profile.as_os_str().as_bytes());
        let owner = unsafe { bind(path) };
        assert_eq!(owner.status, abi::OK);
        let mut input = Request {
            operation: abi::WATCH_OPEN,
            flags: 0,
            path,
            data: Bytes::borrowed(&[]),
            limit: 0,
            expected_device: 0,
            expected_inode: 0,
        };
        let opened = unsafe { request(owner.handle, input) };
        assert_eq!(opened.status, abi::OK);
        let watch = broker.checked_watch(opened.handle).unwrap();

        retire();
        observer.stop();
        let mut event = watch_event(0, 0, 0, 0);
        let timeout = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        assert_eq!(unsafe {
            libc::kevent(watch.queue.raw(), std::ptr::null(), 0, &mut event, 1, &timeout)
        }, 1);
        assert_eq!((event.filter, event.ident), (libc::EVFILT_USER, 1));
        input.operation = abi::WATCH_POLL;
        assert_eq!(unsafe { request(opened.handle, input) }.status, abi::DENIED);
        assert_eq!(release_handle(opened.handle), abi::OK);
        assert_eq!(release_handle(owner.handle), abi::OK);
    }

    #[test]
    fn ordinary_profile_record_cannot_become_publisher_and_notice_is_data_only() {
        assert!(input_publisher_record(Record::Profile(PathBuf::from("/owned/profile"))).is_err());
        assert_eq!(std::mem::size_of::<NativeInputPublisherCapability>(),16);
        assert_eq!(std::mem::size_of::<NativeInputOwnerNoticeV1>(),48);
        assert_eq!(std::mem::align_of::<NativeInputOwnerNoticeV1>(),8);
        assert_eq!(std::mem::offset_of!(NativeInputOwnerNoticeV1,userdata),16);
        assert_eq!(std::mem::offset_of!(NativeInputOwnerNoticeV1,ticket_generation),40);
        let notice=NativeInputOwnerNoticeV1{version:1,bytes:48,role:1,ticket_kind:1,userdata:1,owner_epoch:1,ticket_slot:1,ticket_generation:1};
        // Even well-shaped scalars create no publisher/admission/owner object.
        notice.check(true).unwrap();
        assert!(notice.check(false).is_err());
        let publish=NativeInputOwnerNoticeV1{ticket_slot:0,ticket_generation:0,..notice};
        publish.check(false).unwrap();
        assert!(publish.check(true).is_err());
        assert_eq!(std::mem::size_of::<NativeInputOwnerKeyV1>(),16);
        assert!(NativeInputOwnerNoticeV1{ticket_kind:2,..notice}.check(true).is_err());
        assert!(NativeInputOwnerNoticeV1{owner_epoch:0,..notice}.check(true).is_err());
        assert!(NativeInputOwnerNoticeV1{ticket_generation:0,..notice}.check(true).is_err());
        assert!(NativeInputOwnerNoticeV1{userdata:0,..notice}.check(true).is_err());
    }
}
