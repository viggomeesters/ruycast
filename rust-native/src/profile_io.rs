//! Retained test IO transport. Construct only after the external loaded-image
//! authority has qualified and pinned the exact C table and function roster.
//! This module discovers no symbols and has no ambient filesystem fallback.
use crate::{
    private_storage::{Publication, WriteOutcome},
    profile_io_abi as abi,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = 32_000_000;
struct Session {
    api: &'static abi::Api,
    handle: abi::Handle,
}
impl Drop for Session {
    fn drop(&mut self) {
        (self.api.release_handle)(self.handle);
    }
}
/// Clones retain the same profile generation through worker/observer handoff.
#[derive(Clone)]
pub struct Binding(Arc<Session>);
#[derive(Clone)]
pub struct Watch {
    session: Arc<Session>,
    profile: PathBuf,
    _binding: Binding,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchEvent {
    Timeout,
    Changed { flags: u32 },
    Woken,
}
pub struct Lock {
    _session: Session,
}
impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileIoBinding")
            .field("handle", &self.0.handle)
            .finish()
    }
}
struct ReturnedBuffer<'a> {
    api: &'a abi::Api,
    owner: abi::Handle,
    token: u64,
}
impl Drop for ReturnedBuffer<'_> {
    fn drop(&mut self) {
        if self.token != 0 {
            (self.api.return_buffer)(self.owner, self.token);
        }
    }
}
struct Answer {
    status: i32,
    handle: Option<Session>,
    bytes: Vec<u8>,
    metadata: abi::Metadata,
}
fn answer(
    api: &'static abi::Api,
    owner: abi::Handle,
    reply: abi::Reply,
    new_handle: bool,
) -> Result<Answer> {
    // Only bind/LOCK replies transfer a new handle. An echo of the input owner
    // is never a new owned resource and must not retire the caller's binding.
    let handle = if new_handle && valid_handle(reply.handle) && reply.handle != owner {
        Some(Session {
            api,
            handle: reply.handle,
        })
    } else {
        None
    };
    let _buffer = ReturnedBuffer {
        api,
        owner,
        token: reply.buffer.token,
    };
    if (!new_handle && reply.handle != abi::Handle::default())
        || (new_handle && valid_handle(reply.handle) && reply.handle == owner)
        || reply.reserved != 0
        || reply.metadata.reserved != 0
        || !matches!(
            reply.status,
            abi::OK | abi::MISSING | abi::DENIED | abi::FAILED | abi::PUBLISHED
        )
        || reply.buffer.length > MAX_BYTES
        || (reply.buffer.token == 0
            && (!reply.buffer.pointer.is_null() || reply.buffer.length != 0))
        || (reply.buffer.token != 0 && reply.buffer.pointer.is_null())
    {
        return Err("Invalid owned profile IO reply.".into());
    }
    // The pinned qualified table owns readable immutable reply bytes until
    // return_buffer; no allocation is transferred across the image boundary.
    let bytes = if reply.buffer.token == 0 {
        vec![]
    } else {
        unsafe { std::slice::from_raw_parts(reply.buffer.pointer, reply.buffer.length) }.to_vec()
    };
    if matches!(reply.status, abi::DENIED | abi::FAILED) {
        return Err(if bytes.is_empty() {
            "Owned profile IO refused.".into()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        });
    }
    Ok(Answer {
        status: reply.status,
        handle,
        bytes,
        metadata: reply.metadata,
    })
}
fn path_bytes(path: &Path) -> Result<&[u8]> {
    let path = path.to_str().ok_or("Owned profile IO path is not UTF8.")?;
    if path.is_empty() || path.len() > 32_768 || path.contains('\0') {
        return Err("Invalid owned profile IO path.".into());
    }
    Ok(path.as_bytes())
}
fn valid_handle(handle: abi::Handle) -> bool {
    handle.slot != 0 && handle.generation != 0
}
fn session_request(
    session: &Session,
    operation: u32,
    flags: u32,
    path: &Path,
    data: &[u8],
    limit: u64,
    identity: Publication,
) -> Result<Answer> {
    if data.len() > MAX_BYTES {
        return Err("Owned profile IO payload exceeds limit.".into());
    }
    let request = abi::Request {
        operation,
        flags,
        path: abi::Bytes::borrowed(path_bytes(path)?),
        data: abi::Bytes::borrowed(data),
        limit,
        expected_device: identity.device,
        expected_inode: identity.inode,
    };
    let reply = unsafe { (session.api.request)(session.handle, request) };
    answer(
        session.api,
        session.handle,
        reply,
        matches!(operation, abi::LOCK | abi::WATCH_OPEN),
    )
}
impl Binding {
    /// # Safety
    /// The caller must independently qualify this table's exact loaded image,
    /// immutable memory, ABI/function roster and live owned profile authority.
    /// That image must remain loaded for the process lifetime (`api` is static).
    /// All callbacks must obey the ABI, including valid retained output buffers
    /// and no unwinding. Header matching alone does not establish this contract.
    pub unsafe fn bind_pinned_api(api: &'static abi::Api, profile: &Path) -> Result<Self> {
        if api.version != abi::VERSION
            || api.bytes as usize != std::mem::size_of::<abi::Api>()
            || api.reserved != [0; 2]
        {
            return Err("Unsupported owned profile IO table.".into());
        }
        let reply = unsafe { (api.bind)(abi::Bytes::borrowed(path_bytes(profile)?)) };
        let mut value = answer(api, abi::Handle::default(), reply, true)?;
        if value.status != abi::OK {
            return Err("Invalid owned profile binding.".into());
        }
        let session = value
            .handle
            .take()
            .ok_or("Missing owned profile binding.")?;
        Ok(Self(Arc::new(session)))
    }
    fn request(
        &self,
        operation: u32,
        flags: u32,
        path: &Path,
        data: &[u8],
        limit: u64,
        identity: Publication,
    ) -> Result<Answer> {
        session_request(&self.0, operation, flags, path, data, limit, identity)
    }
    fn simple(&self, operation: u32, flags: u32, path: &Path) -> Result<Answer> {
        self.request(
            operation,
            flags,
            path,
            &[],
            0,
            Publication {
                device: 0,
                inode: 0,
            },
        )
    }
    pub fn read_optional(&self, path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
        if limit > MAX_BYTES {
            return Err("Owned profile IO read limit exceeds maximum.".into());
        }
        let answer = self.request(
            abi::READ,
            0,
            path,
            &[],
            limit as u64,
            Publication {
                device: 0,
                inode: 0,
            },
        )?;
        match answer.status {
            abi::OK if answer.bytes.len() <= limit => Ok(Some(answer.bytes)),
            abi::MISSING if answer.bytes.is_empty() => Ok(None),
            _ => Err("Invalid owned profile read result.".into()),
        }
    }
    pub fn metadata(&self, path: &Path) -> Result<Option<abi::Metadata>> {
        let answer = self.simple(abi::STAT, 0, path)?;
        match answer.status {
            abi::OK => Ok(Some(answer.metadata)),
            abi::MISSING => Ok(None),
            _ => Err("Invalid owned profile metadata result.".into()),
        }
    }
    pub fn create_directory(&self, path: &Path, recursive: bool) -> Result<()> {
        self.require_ok(self.simple(
            abi::MKDIR,
            if recursive { abi::RECURSIVE } else { 0 },
            path,
        )?)
    }
    pub fn secure_directory(&self, path: &Path) -> Result<()> {
        self.require_ok(self.simple(abi::SECURE_DIRECTORY, 0, path)?)
    }
    fn require_ok(&self, answer: Answer) -> Result<()> {
        if answer.status == abi::OK {
            Ok(())
        } else {
            Err("Invalid owned profile operation result.".into())
        }
    }
    pub fn write_outcome(&self, path: &Path, bytes: &[u8], replace: bool) -> Result<WriteOutcome> {
        let answer = self.request(
            abi::WRITE,
            if replace { abi::REPLACE } else { 0 },
            path,
            bytes,
            0,
            Publication {
                device: 0,
                inode: 0,
            },
        )?;
        let identity = Publication {
            device: answer.metadata.device,
            inode: answer.metadata.inode,
        };
        match answer.status {
            abi::OK => Ok(WriteOutcome::Durable(identity)),
            abi::PUBLISHED => Ok(WriteOutcome::Published {
                identity,
                error: if answer.bytes.is_empty() {
                    "Durability could not be confirmed.".into()
                } else {
                    String::from_utf8_lossy(&answer.bytes).into_owned()
                },
            }),
            _ => Err("Invalid owned profile publication result.".into()),
        }
    }
    pub fn remove_owned(&self, path: &Path, bytes: &[u8], identity: Publication) -> Result<()> {
        self.require_ok(self.request(abi::REMOVE_OWNED, 0, path, bytes, 0, identity)?)
    }
    pub fn open_watch(&self, profile: &Path) -> Result<Watch> {
        let mut answer = self.simple(abi::WATCH_OPEN, 0, profile)?;
        if answer.status != abi::OK {
            return Err("Invalid owned profile watch result.".into());
        }
        let session = answer.handle.take().ok_or("Missing owned profile watch.")?;
        Ok(Watch {
            session: Arc::new(session),
            profile: profile.into(),
            _binding: self.clone(),
        })
    }
    pub fn lock(&self, path: &Path) -> Result<Lock> {
        let mut answer = self.simple(abi::LOCK, 0, path)?;
        if answer.status != abi::OK {
            return Err("Invalid owned profile lock result.".into());
        }
        let session = answer.handle.take().ok_or("Missing owned profile lock.")?;
        Ok(Lock { _session: session })
    }
}

impl Watch {
    fn request(&self, operation: u32, path: &Path, limit: u64) -> Result<Answer> {
        session_request(
            &self.session,
            operation,
            0,
            path,
            &[],
            limit,
            Publication {
                device: 0,
                inode: 0,
            },
        )
    }
    pub fn add_directory(&self, path: &Path) -> Result<()> {
        if self.request(abi::WATCH_DIRECTORY, path, 0)?.status != abi::OK {
            return Err("Invalid owned watch registration result.".into());
        }
        Ok(())
    }
    /// Cleanup wake remains available for this exact issued watch after global
    /// retirement. It grants no filesystem access or new descriptor.
    pub fn wake(&self) -> Result<()> {
        if self.request(abi::WATCH_WAKE, &self.profile, 0)?.status != abi::OK {
            return Err("Owned watch wake refused.".into());
        }
        Ok(())
    }
    pub fn poll(&self, timeout: Duration) -> Result<WatchEvent> {
        if timeout > Duration::from_secs(5) {
            return Err("Owned watch poll exceeds five seconds.".into());
        }
        let answer = self.request(abi::WATCH_POLL, &self.profile, timeout.as_millis() as u64)?;
        if answer.status != abi::OK || answer.bytes.len() != 8 {
            return Err("Invalid owned watch event reply.".into());
        }
        let kind = u32::from_le_bytes(
            answer.bytes[..4]
                .try_into()
                .map_err(|_| "watch event kind")?,
        );
        let flags = u32::from_le_bytes(
            answer.bytes[4..]
                .try_into()
                .map_err(|_| "watch event flags")?,
        );
        let allowed = libc::NOTE_WRITE
            | libc::NOTE_LINK
            | libc::NOTE_RENAME
            | libc::NOTE_DELETE
            | libc::NOTE_ATTRIB
            | libc::NOTE_REVOKE;
        match kind {
            abi::WATCH_TIMEOUT if flags == 0 => Ok(WatchEvent::Timeout),
            abi::WATCH_WOKEN if flags == 0 => Ok(WatchEvent::Woken),
            abi::WATCH_CHANGED if flags & !allowed == 0 => Ok(WatchEvent::Changed { flags }),
            _ => Err("Unknown owned watch event.".into()),
        }
    }
}
