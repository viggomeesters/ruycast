//! Test-feature owned profile IO boundary. Plain C layout only: Rust allocators,
//! trait objects, File ownership and Arc never cross the loaded-image boundary.
pub const VERSION: u32 = 1;
pub const OK: i32 = 0;
pub const MISSING: i32 = 1;
pub const DENIED: i32 = 2;
pub const FAILED: i32 = 3;
pub const PUBLISHED: i32 = 4;
pub const READ: u32 = 1;
pub const STAT: u32 = 2;
pub const MKDIR: u32 = 3;
pub const SECURE_DIRECTORY: u32 = 4;
pub const WRITE: u32 = 5;
pub const REMOVE_OWNED: u32 = 6;
pub const LOCK: u32 = 7;
pub const WATCH_OPEN: u32 = 8;
pub const WATCH_DIRECTORY: u32 = 9;
pub const WATCH_POLL: u32 = 10;
pub const WATCH_WAKE: u32 = 11;
// WATCH_POLL returns exactly two little-endian u32 words: event kind, fflags.
pub const WATCH_TIMEOUT: u32 = 0;
pub const WATCH_CHANGED: u32 = 1;
pub const WATCH_WOKEN: u32 = 2;
pub const RECURSIVE: u32 = 1;
pub const REPLACE: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Handle {
    pub slot: u64,
    pub generation: u64,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Bytes {
    pub pointer: *const u8,
    pub length: usize,
}
impl Bytes {
    pub fn borrowed(bytes: &[u8]) -> Self {
        Self {
            pointer: bytes.as_ptr(),
            length: bytes.len(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Metadata {
    pub device: u64,
    pub inode: u64,
    pub length: u64,
    pub links: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub reserved: u32,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub changed_seconds: i64,
    pub changed_nanoseconds: i64,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Request {
    pub operation: u32,
    pub flags: u32,
    pub path: Bytes,
    pub data: Bytes,
    pub limit: u64,
    pub expected_device: u64,
    pub expected_inode: u64,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Buffer {
    pub token: u64,
    pub pointer: *const u8,
    pub length: usize,
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Reply {
    pub status: i32,
    pub reserved: u32,
    pub handle: Handle,
    pub buffer: Buffer,
    pub metadata: Metadata,
}
impl Reply {
    pub const fn denied() -> Self {
        Self {
            status: DENIED,
            reserved: 0,
            handle: Handle {
                slot: 0,
                generation: 0,
            },
            buffer: Buffer {
                token: 0,
                pointer: std::ptr::null(),
                length: 0,
            },
            metadata: Metadata {
                device: 0,
                inode: 0,
                length: 0,
                links: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                reserved: 0,
                modified_seconds: 0,
                modified_nanoseconds: 0,
                changed_seconds: 0,
                changed_nanoseconds: 0,
            },
        }
    }
}
/// The caller must pin and qualify this exact SDK image/table/function roster
/// before dereferencing or invoking it. A version/size match alone is no grant.
#[repr(C)]
pub struct Api {
    pub version: u32,
    pub bytes: u32,
    pub reserved: [u64; 2],
    pub bind: unsafe extern "C" fn(Bytes) -> Reply,
    pub request: unsafe extern "C" fn(Handle, Request) -> Reply,
    pub release_handle: extern "C" fn(Handle) -> i32,
    pub return_buffer: extern "C" fn(Handle, u64) -> i32,
}

/// Constructor-to-main handoff for the test-tools complete owned host. This
/// frame conveys no authority: the receiver must independently qualify its
/// compile-pinned private grant, both loaded images and every supplied address
/// before reading the Api or calling its getter/ready/function roster.
/// All borrowed bytes must remain readable until the sink returns. No Rust
/// ownership, allocator, Arc, File or trait object crosses this boundary.
#[repr(C)]
pub struct HostBinding {
    pub version: u32,
    pub bytes: u32,
    pub flags: u32,
    pub reserved: u32,
    pub profile: Bytes,
    pub home: Bytes,
    pub config_override: Bytes,
    pub api: *const Api,
    pub main_header: usize,
    pub sdk_header: usize,
    pub getter: usize,
    pub ready: usize,
    pub input_publisher: [u64; 2],
}
pub const HOST_BINDING_VERSION: u32 = 2;
pub const HOST_HAS_CONFIG_OVERRIDE: u32 = 1;
/// Unsafe caller contract includes readable borrowed frame/path storage and
/// the separately source-qualified constructor. Success is0; refusal is2.
pub type HostSink = unsafe extern "C" fn(*const HostBinding) -> i32;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_host_frame_preserves_existing_profile_table_layout() {
        assert_eq!(std::mem::size_of::<Api>(), 56);
        assert_eq!(std::mem::offset_of!(Api, bind), 24);
        assert_eq!(std::mem::size_of::<Request>(), 64);
        assert_eq!(std::mem::size_of::<Reply>(), 128);
        assert_eq!(std::mem::size_of::<HostBinding>(), 120);
        assert_eq!(std::mem::align_of::<HostBinding>(), 8);
        assert_eq!(std::mem::offset_of!(HostBinding, api), 64);
        assert_eq!(std::mem::offset_of!(HostBinding, ready), 96);
        assert_eq!(std::mem::offset_of!(HostBinding, input_publisher), 104);
    }
}
