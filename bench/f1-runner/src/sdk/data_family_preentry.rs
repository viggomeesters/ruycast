//! Source-included, data-only admission. No SDK dylib, interpose, constructor,
//! HostReady or public-scope registration is linked or enabled by this module.
use super::{
    metadata,
    owned::{AliveLease, OwnedRoot, Result},
};
use crate::{guardian, guardian_alive::GuardianAlive, workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::{c_char, c_void, CStr},
    io::Read,
    os::fd::{FromRawFd, OwnedFd},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const ROLE: &str = "apple-event-descriptor-data-v1";
const FOUNDATION_ROLE: &str = "foundation-data-v1";
const MAX_GRANT: usize = 16_384;
const FOUNDATION: &[u8] = b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation";
const OBJC: &[u8] = b"/usr/lib/libobjc.A.dylib";
// Class/metaclass metadata ownership only; this does not grant Method IMP images.
const CORE_FOUNDATION: &[u8] = b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation";
const FOUNDATION_CLASS_IMAGES: [&[u8];8] = [OBJC,FOUNDATION,FOUNDATION,CORE_FOUNDATION,CORE_FOUNDATION,FOUNDATION,CORE_FOUNDATION,CORE_FOUNDATION];

/// Closed family: no caller-provided class name or Host role is accepted.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum DataFamily {
    AppleEventDescriptor,
    FoundationData,
}
impl DataFamily {
    fn role(self)->&'static str {match self {Self::AppleEventDescriptor=>ROLE,Self::FoundationData=>FOUNDATION_ROLE}}
    fn argument(self)->&'static [u8] {match self {Self::AppleEventDescriptor=>b"__f1-appleevent-data",Self::FoundationData=>b"__f1-foundation-data"}}
}
/// Exact separately compiled declaration inputs, not an external root grant.
pub struct ForeignInputPin<'a> {
    pub path: &'a Path,
    pub sha256: &'a str,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub kind: u32,
    pub access: i32,
}
impl Fingerprint {
    fn actual(fd: i32) -> Result<(Self, u32)> {
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fstat(fd, &mut s) } != 0 {
            return Err("data capability unavailable".into());
        }
        Ok((
            Self {
                device: s.st_dev as u64,
                inode: s.st_ino,
                kind: u32::from(s.st_mode & libc::S_IFMT),
                access: flags & libc::O_ACCMODE,
            },
            s.st_uid,
        ))
    }
}
/// Trusted producer captures the private readcap BEFORE compiling this image.
/// Final image/source hashes arrive through that same one-shot cap after build;
/// no self-hash circle, runtime packet or arbitrary fstat can mint these pins.
pub struct Pins<'a> {
    pub family: DataFamily,
    pub source_root: &'a Path,
    pub fixture_root: &'a Path,
    pub endpoint: &'a Path,
    pub source_manifest: &'a Path,
    pub abi_leaf: &'a Path,
    pub entry_source: &'a Path,
    /// Exact ordered compile-time dependency paths, including this gate, the
    /// actual main/Pins source, guardian, metadata and workspace typed leaves.
    pub source_paths: &'a [&'a Path],
    pub foreign_inputs: &'a [ForeignInputPin<'a>],
    pub nonce_sha256: &'a str,
    pub owner_uid: u32,
    pub grant_fd: i32,
    pub grant: Fingerprint,
    pub stdio: [Fingerprint; 3],
}
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub path: String,
    pub sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub role: String,
    pub family: DataFamily,
    pub owner_uid: u32,
    pub nonce: String,
    pub endpoint_sha256: String,
    pub source_set_sha256: String,
    pub source_manifest: Input,
    pub abi_leaf: Input,
    pub foreign_inputs: Vec<Input>,
    pub stdio: [Fingerprint; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    sources: Vec<Input>,
}

#[link(name = "System")]
unsafe extern "C" {
    fn _NSGetArgc() -> *const i32;
    fn _NSGetArgv() -> *const *const *const c_char;
    fn _NSGetEnviron() -> *const *const *const c_char;
    fn _NSGetExecutablePath(buffer: *mut c_char, size: *mut u32) -> i32;
    fn _NSGetMachExecuteHeader() -> *const u8;
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_name(index: u32) -> *const c_char;
}
#[link(name = "objc")]
unsafe extern "C" {
    fn class_getSuperclass(class: *const c_void) -> *const c_void;
    fn class_getImageName(class: *const c_void) -> *const c_char;
    fn object_getClass(object: *const c_void) -> *const c_void;
}
unsafe fn text(p: *const c_char, bound: usize) -> Result<Vec<u8>> {
    if !p.is_null() {
        for n in 0..bound {
            if unsafe { p.add(n).read() } == 0 {
                return Ok(unsafe { std::slice::from_raw_parts(p.cast(), n) }.to_vec());
            }
        }
    }
    Err("missing/overlong data entry value".into())
}
fn entry(pins: &Pins<'_>) -> Result<()> {
    unsafe {
        let argc = _NSGetArgc();
        let argv = _NSGetArgv();
        if argc.is_null()
            || argv.is_null()
            || *argc != 2
            || (*argv).is_null()
            || text((*argv).read(), 8192)? != pins.endpoint.as_os_str().as_encoded_bytes()
            || text((*argv).add(1).read(), 128)? != pins.family.argument()
            || !(*argv).add(2).read().is_null()
        {
            return Err("data role requires its exact source-included entry".into());
        }
        let env = _NSGetEnviron();
        if env.is_null() || (*env).is_null() {
            return Err("data entry environment absent".into());
        }
        let mut fields = EntryEnvironment::default();
        // Four exact producer-owned fields plus the terminating NULL slot.
        for n in 0..5 {
            let p = (*env).add(n).read();
            if p.is_null() {
                return fields.finish(pins.grant_fd);
            }
            let bytes = text(p, 8192)?;
            let value = std::str::from_utf8(&bytes).map_err(|_| "data environment UTF8")?;
            let (key, value) = value.split_once('=').ok_or("data environment encoding")?;
            fields.push(key, value, pins.owner_uid)?;
        }
    }
    Err("data environment count".into())
}
const CF_CONTEXT_KEY: &str = "__CF_USER_TEXT_ENCODING";
fn cf_context(owner_uid: u32) -> String {
    // Producer-synthetic context, not ambient locale or capability authority.
    // Apple CF parser uses base-zero UID/script/region decoding; only this
    // canonical representation of the already-pinned owner is accepted here.
    format!("0x{owner_uid:X}:0x0:0x0")
}
#[derive(Default)]
struct EntryEnvironment {
    seen: BTreeSet<String>,
    fds: [Option<i32>; 3],
    context: bool,
}
impl EntryEnvironment {
    fn push(&mut self, key: &str, value: &str, owner_uid: u32) -> Result<()> {
        if !self.seen.insert(key.to_owned()) {
            return Err("duplicate data entry field".into());
        }
        if key == CF_CONTEXT_KEY {
            if value != cf_context(owner_uid) {
                return Err("data environment mismatched producer CF context".into());
            }
            self.context = true;
        } else {
            let (field, fd) = environment_fd(key, value)?;
            let index = match field {
                EnvironmentFd::Life => 0,
                EnvironmentFd::Admit => 1,
                EnvironmentFd::Grant => 2,
            };
            self.fds[index] = Some(fd);
        }
        Ok(())
    }
    fn finish(self, expected_grant: i32) -> Result<()> {
        let [a, b, c] = self.fds;
        let (a, b, c) = (
            a.ok_or("data life absent")?,
            b.ok_or("data admit absent")?,
            c.ok_or("data grant absent")?,
        );
        if !self.context {
            return Err("data environment missing producer CF context".into());
        }
        if self.seen.len() != 4
            || a <= 2
            || b <= 2
            || c <= 2
            || a == b
            || a == c
            || b == c
            || c != expected_grant
        {
            return Err("data entry capability aliases/locator".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvironmentFd {
    Life,
    Admit,
    Grant,
}
fn environment_fd(key: &str, value: &str) -> Result<(EnvironmentFd, i32)> {
    // Recognize ONLY the closed key roster before decoding a descriptor. The
    // diagnostic names the bounded key/error class, never any supplied value.
    let field = match key {
        "F1_LIFE_FD" => EnvironmentFd::Life,
        "F1_ADMIT_FD" => EnvironmentFd::Admit,
        "F1_DATA_GRANT_FD" => EnvironmentFd::Grant,
        _ => {
            let mut label = String::new();
            for byte in key.bytes().take(48) {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    label.push(char::from(byte));
                } else {
                    label.push_str(&format!("\\x{byte:02x}"));
                }
            }
            if key.len() > 48 {
                label.push_str("...");
            }
            return Err(format!(
                "data environment unexpected key '{label}'; no fallback"
            ));
        }
    };
    let fd = value
        .parse::<i32>()
        .map_err(|_| format!("data environment invalid FD for {key}"))?;
    Ok((field, fd))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn checked(root: &OwnedRoot, path: &Path, expected: &str) -> Result<()> {
    if !valid_hash(expected) {
        return Err("data source hash absent".into());
    }
    let bytes = root
        .read_optional(path, 64 * 1024 * 1024)?
        .ok_or("data pinned input absent")?;
    if hash(&bytes) != expected {
        return Err("data pinned input changed".into());
    }
    Ok(())
}
// Same trusted compile-time declaration inputs as the owned declaration.
// Observation only: this emits no source proof, external-root grant or fallback.
fn compiled_foreign(family: DataFamily) -> Result<Vec<(PathBuf, String, u32)>> {
    match family {
        DataFamily::AppleEventDescriptor => Ok(Vec::new()),
        DataFamily::FoundationData => workspace::classes::StringBytesDeclaration::compiled_source_pins()?
            .into_iter().map(|(path, digest, owner)| {
                let sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                Ok((path, sha256, owner))
            }).collect(),
    }
}
fn foreign_roster(family:DataFamily,pins:&[ForeignInputPin<'_>],inputs:&[Input])->Result<Vec<Input>> {
    let expected = compiled_foreign(family)?;
    if pins.len()!=expected.len()||inputs.len()!=expected.len(){return Err("data foreign declaration roster count differs".into());}
    for ((pin,input),(path,compiled_hash,_)) in pins.iter().zip(inputs).zip(&expected) {
        if pin.path!=path||Path::new(&input.path)!=pin.path||!valid_hash(pin.sha256)||pin.sha256!=compiled_hash||input.sha256!=pin.sha256 {
            return Err("data foreign declaration differs from exact compiled path/hash".into());
        }
    }
    Ok(inputs.to_vec())
}
/// File-only bounded observation. The sole production caller below first
/// matches one of the two closed, independently compiled declaration paths.
fn read_declaration(path:&Path,expected:&str,owner_uid:u32)->Result<()> {
    if !valid_hash(expected){return Err("foreign declaration hash absent".into());}
    let file=std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK).open(path)
        .map_err(|_|"exact foreign declaration unavailable")?;
    let before=file.metadata().map_err(|_|"foreign declaration metadata unavailable")?;
    if !before.is_file()||before.uid()!=owner_uid||before.nlink()!=1||before.mode()&0o022!=0||before.len()>4*1024*1024 {
        return Err("foreign declaration owner/kind/link/mode/bound differs".into());
    }
    let identity=|m:&std::fs::Metadata|(m.dev(),m.ino(),m.mode(),m.uid(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    let mut bytes=Vec::new();
    let mut reader=&file;
    reader.by_ref().take(4*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"foreign declaration read failed")?;
    let after=file.metadata().map_err(|_|"foreign declaration changed during read")?;
    if bytes.len()>4*1024*1024||identity(&before)!=identity(&after)||hash(&bytes)!=expected {
        return Err("foreign declaration identity/content changed".into());
    }
    Ok(())
}
fn checked_foreign(inputs:&[Input],owner_uid:u32)->Result<()> {
    // AE's closed roster is empty; grant_authority already checks the family.
    if inputs.is_empty() { return Ok(()); }
    let expected = compiled_foreign(DataFamily::FoundationData)?;
    // The SDK header is system-owned; Cargo source belongs to this exact cap.
    // Reject mismatched compiled ownership before opening either declaration.
    if expected.len()!=2||expected[0].2!=0||expected[1].2!=owner_uid {
        return Err("foreign declaration owner differs from system/cap identity".into());
    }
    if inputs.len()!=expected.len(){return Err("extra or missing foreign declaration refused".into());}
    for (input,(path,compiled_hash,owner)) in inputs.iter().zip(&expected) {
        if Path::new(&input.path)!=path||input.sha256!=*compiled_hash{return Err("foreign declaration outside exact compiled file roster".into());}
        read_declaration(path,compiled_hash,*owner)?;
    }
    Ok(())
}
fn grant_authority(pins:&Pins<'_>,grant:&Grant)->Result<Vec<Input>> {
    if grant.family!=pins.family||grant.role!=pins.family.role()||grant.owner_uid!=pins.owner_uid
        ||hash(grant.nonce.as_bytes())!=pins.nonce_sha256||grant.stdio!=pins.stdio {
        return Err("data role/nonce/owner/stdio authority differs".into());
    }
    foreign_roster(pins.family,pins.foreign_inputs,&grant.foreign_inputs)
}
fn read_grant(pins: &Pins<'_>, life: &dyn AliveLease) -> Result<Grant> {
    let (actual, uid) = Fingerprint::actual(pins.grant_fd)?;
    if pins.grant_fd <= 2
        || pins.grant.kind != u32::from(libc::S_IFIFO)
        || pins.grant.access != libc::O_RDONLY
        || actual != pins.grant
        || uid != pins.owner_uid
    {
        return Err("data grant is not compiled private readcap/owner".into());
    }
    let _descriptor = unsafe { OwnedFd::from_raw_fd(pins.grant_fd) };
    let fd = pins.grant_fd;
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err("data grant nonblocking setup".into());
    }
    let end = Instant::now() + Duration::from_secs(3);
    let read = |out: &mut [u8]| -> Result<()> {
        let mut at = 0;
        while at < out.len() {
            life.before_effect()?;
            if !life.is_alive() || Instant::now() >= end {
                return Err("data grant retired/deadline".into());
            }
            let mut p = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut p, 1, 10) };
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if n < 0 || p.revents & libc::POLLNVAL != 0 {
                return Err("data grant poll failure".into());
            }
            if n == 0 {
                continue;
            }
            let n = unsafe { libc::read(fd, out.as_mut_ptr().add(at).cast(), out.len() - at) };
            if n > 0 {
                at += n as usize;
            } else if n == 0 {
                return Err("data grant incomplete".into());
            } else if !matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                return Err("data grant read failure".into());
            }
        }
        Ok(())
    };
    let mut length = [0; 4];
    read(&mut length)?;
    let length = u32::from_ne_bytes(length) as usize;
    if length == 0 || length > MAX_GRANT {
        return Err("data grant length".into());
    }
    let mut bytes = vec![0; length];
    read(&mut bytes)?;
    let mut trailing = 0u8;
    life.before_effect()?;
    if unsafe { libc::read(fd, (&mut trailing as *mut u8).cast(), 1) } != 0 {
        return Err("data grant requires immutable EOF snapshot".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "data grant schema".into())
}
/// Actual loader header only. No caller header, constructor receipt or ready
/// value can satisfy this. The producer must ALSO inspect before exec.
unsafe fn image_header(header: *const u8) -> Result<()> {
    unsafe {
        if header.is_null()
            || header.cast::<u32>().read_unaligned() != 0xfeedfacf
            || header.add(12).cast::<u32>().read_unaligned() != 2
        {
            return Err("data image is not MachO64 main".into());
        }
        let count = header.add(16).cast::<u32>().read_unaligned() as usize;
        let bytes = header.add(20).cast::<u32>().read_unaligned() as usize;
        if count > 4096 || bytes > 16 * 1024 * 1024 {
            return Err("data main load-command bounds".into());
        }
        let mut at = 0;
        for _ in 0..count {
            if at + 8 > bytes {
                return Err("data main command truncated".into());
            }
            let p = header.add(32 + at);
            let kind = p.cast::<u32>().read_unaligned();
            let size = p.add(4).cast::<u32>().read_unaligned() as usize;
            if size < 8 || at.checked_add(size).is_none_or(|n| n > bytes) {
                return Err("data main command invalid".into());
            }
            if matches!(kind, 0x11 | 0x1a) {
                return Err("data main LC_ROUTINES initializer".into());
            }
            if kind == 0x19 {
                if size < 72 {
                    return Err("data segment truncated".into());
                }
                let n = p.add(64).cast::<u32>().read_unaligned() as usize;
                if n > 4096 || 72 + n * 80 > size {
                    return Err("data section bounds".into());
                }
                for i in 0..n {
                    let s = p.add(72 + i * 80);
                    let flags = s.add(64).cast::<u32>().read_unaligned() & 0xff;
                    let len = s.add(40).cast::<u64>().read_unaligned();
                    let name = std::slice::from_raw_parts(s, 16)
                        .split(|b| *b == 0)
                        .next()
                        .unwrap_or_default();
                    if len != 0
                        && (matches!(flags, 9 | 0x15 | 0x16)
                            || matches!(
                                name,
                                b"__interpose" | b"__objc_nlclslist" | b"__objc_nlcatlist"
                            ))
                    {
                        return Err("data main initializer/interpose forbidden".into());
                    }
                }
            }
            at += size;
        }
        if at != bytes {
            return Err("data command roster differs".into());
        }
    }
    Ok(())
}
fn images(endpoint: &Path) -> Result<()> {
    let mut path = [0i8; 8192];
    let mut n = path.len() as u32;
    if unsafe { _NSGetExecutablePath(path.as_mut_ptr(), &mut n) } != 0
        || unsafe { text(path.as_ptr(), 8192) }? != endpoint.as_os_str().as_encoded_bytes()
    {
        return Err("actual data main path differs".into());
    }
    unsafe { image_header(_NSGetMachExecuteHeader()) }?;
    let count = unsafe { _dyld_image_count() };
    if count == 0 || count > 4096 {
        return Err("data loader image bound".into());
    }
    let mut mains = 0;
    for i in 0..count {
        let name = unsafe { text(_dyld_get_image_name(i), 8192) }?;
        if name == endpoint.as_os_str().as_encoded_bytes() {
            mains += 1;
        } else if !name.starts_with(b"/System/Library/") && !name.starts_with(b"/usr/lib/") {
            return Err("data role forbids non-system SDK/provider image".into());
        }
    }
    if mains != 1 {
        return Err("data loader main roster differs".into());
    }
    Ok(())
}
fn class_roster() -> Result<(metadata::ClassHandle, metadata::ClassHandle)> {
    let class = metadata::loaded_class(c"NSAppleEventDescriptor")?;
    let parent = metadata::loaded_class(c"NSObject")?;
    if unsafe { class_getSuperclass(class.address() as *const c_void) } as usize != parent.address()
        || !unsafe { class_getSuperclass(parent.address() as *const c_void) }.is_null()
        || unsafe { text(class_getImageName(class.address() as *const c_void), 8192) }?
            != FOUNDATION
        || unsafe { text(class_getImageName(parent.address() as *const c_void), 8192) }? != OBJC
    {
        return Err("data exact NSAE/NSObject class/image roster differs".into());
    }
    Ok((class, parent))
}
type Methods = Vec<workspace::objc::AppleEventMethodPreflight>;
fn method_roster(endpoint: &Path) -> Result<Methods> {
    let rows = workspace::objc::preflight_apple_event_factory()?;
    if rows.is_empty() || rows.len() > 128 {
        return Err("data typed ABI roster bounds".into());
    }
    let mut keys = BTreeSet::new();
    for row in &rows {
        if !keys.insert((row.selector.to_bytes(), row.class_method))
            || ![FOUNDATION, OBJC].contains(&row.source.code().image())
            || row.replacement.image() != endpoint.as_os_str().as_encoded_bytes()
        {
            return Err("data original/substitution code-image roster differs".into());
        }
    }
    Ok(rows)
}
#[derive(Eq,PartialEq)]
struct FoundationClass {
    class:metadata::ClassHandle,
    metaclass:usize,
    meta_parent:usize,
    lifecycle:metadata::LifecycleSnapshot,
}
fn foundation_classes()->Result<Vec<FoundationClass>> {
    let names=[c"NSObject",c"NSValue",c"NSString",c"NSData",c"NSURL",c"NSNumber",c"NSArray",c"NSDictionary"];
    let classes=names.into_iter().map(metadata::loaded_class).collect::<Result<Vec<_>>>()?;
    let mut result=Vec::new();
    for (index,class) in classes.iter().copied().enumerate() {
        let expected_parent=match index {0=>0,5=>classes[1].address(),_=>classes[0].address()};
        let expected_image=FOUNDATION_CLASS_IMAGES[index];
        let meta=unsafe{object_getClass(class.address() as *const c_void)};
        let expected_meta_parent=if index==0 {class.address()}else{(unsafe{object_getClass(expected_parent as *const c_void)}) as usize};
        let differs=|field:&str| format!("Foundation {} {field} differs",names[index].to_string_lossy());
        if meta.is_null() {
            return Err(differs("metaclass-null"));
        }
        if unsafe{class_getSuperclass(class.address() as *const c_void)} as usize!=expected_parent {
            return Err(differs("class-superclass"));
        }
        if unsafe{text(class_getImageName(class.address() as *const c_void),8192)}?!=expected_image {
            return Err(differs("class-image"));
        }
        if unsafe{text(class_getImageName(meta),8192)}?!=expected_image {
            return Err(differs("metaclass-image"));
        }
        if unsafe{class_getSuperclass(meta)} as usize!=expected_meta_parent {
            return Err(differs("metaclass-superclass"));
        }
        result.push(FoundationClass{class,metaclass:meta as usize,meta_parent:expected_meta_parent,lifecycle:metadata::lifecycle(class)?});
    }
    Ok(result)
}
fn foundation_keys()->BTreeSet<(&'static CStr,&'static CStr,bool)> {
    [
        (c"NSString",c"initWithCharacters:length:",false),(c"NSString",c"initWithBytes:length:encoding:",false),
        (c"NSString",c"length",false),(c"NSString",c"characterAtIndex:",false),(c"NSString",c"getCharacters:range:",false),
        (c"NSString",c"UTF8String",false),(c"NSString",c"lengthOfBytesUsingEncoding:",false),(c"NSString",c"isEqualToString:",false),
        (c"NSData",c"initWithBytes:length:",false),(c"NSData",c"dataWithBytes:length:",true),(c"NSData",c"length",false),(c"NSData",c"bytes",false),
        (c"NSURL",c"URLWithString:",true),(c"NSURL",c"fileURLWithPath:",true),(c"NSURL",c"fileURLWithPath:isDirectory:",true),
        (c"NSURL",c"absoluteString",false),(c"NSURL",c"isFileURL",false),(c"NSURL",c"host",false),(c"NSURL",c"path",false),(c"NSURL",c"lastPathComponent",false),
        (c"NSNumber",c"numberWithBool:",true),(c"NSNumber",c"boolValue",false),
        (c"NSArray",c"initWithObjects:count:",false),(c"NSArray",c"arrayWithObjects:count:",true),(c"NSArray",c"count",false),
        (c"NSArray",c"objectAtIndex:",false),(c"NSArray",c"countByEnumeratingWithState:objects:count:",false),
        (c"NSDictionary",c"initWithObjects:forKeys:count:",false),
    ].into_iter().collect()
}
// Method IMP ownership is independently pinned per closed (class, selector, meta) key.
fn foundation_existing_method_image(role:&CStr,selector:&CStr,meta:bool)->Option<&'static [u8]> {
    if !foundation_keys().contains(&(role,selector,meta))
        || (role==c"NSString"&&selector==c"initWithBytes:length:encoding:"&&!meta) {return None;}
    let core_foundation=[
        (c"NSArray",c"arrayWithObjects:count:",true),
        (c"NSArray",c"initWithObjects:count:",false),
        (c"NSArray",c"count",false),
        (c"NSArray",c"objectAtIndex:",false),
        (c"NSArray",c"countByEnumeratingWithState:objects:count:",false),
        (c"NSDictionary",c"initWithObjects:forKeys:count:",false),
    ];
    Some(if core_foundation.contains(&(role,selector,meta)) {CORE_FOUNDATION}else{FOUNDATION})
}
type FoundationMethods=Vec<(&'static CStr,workspace::objc::TypedDataMethodPreflight)>;
fn foundation_methods(endpoint:&Path,foreign:&[Input])->Result<FoundationMethods> {
    use workspace::objc::TypedDataMethodPreflight as Row;
    let rows=workspace::objc::preflight_foundation_factories()?;
    let mut keys=BTreeSet::new();let mut declarations=0;
    for (role,row) in &rows {
        let (selector,meta,replacement)=match row {
            Row::Existing(row)=>{
                let expected_image=foundation_existing_method_image(role,row.selector,row.class_method)
                    .ok_or("Foundation source method key differs")?;
                if row.source.code().image()!=expected_image {return Err("Foundation source code image differs".into());}
                (row.selector,row.class_method,&row.replacement)
            }
            Row::OwnedDeclaration{selector,declaration}=>{
                declarations+=1;
                if *role!=c"NSString"||*selector!=c"initWithBytes:length:encoding:"
                    ||declaration.base_class()!=metadata::loaded_class(c"NSString")?||foreign.len()!=2 {
                    return Err("Foundation owned declaration role/base differs".into());
                }
                for ((path,digest),expected) in declaration.source_pins().iter().zip(foreign) {
                    if path!=Path::new(&expected.path) {return Err("Foundation declaration source path differs".into());}
                    let actual=digest.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
                    if actual!=expected.sha256 {return Err("Foundation declaration is not compiled foreign source".into());}
                }
                (*selector,false,declaration.implementation_code())
            }
        };
        if !keys.insert((*role,selector,meta))||replacement.image()!=endpoint.as_os_str().as_encoded_bytes(){return Err("Foundation duplicate/substitution image differs".into());}
    }
    if rows.len()!=28||declarations!=1||keys!=foundation_keys(){return Err("Foundation exact 27-existing/1-declaration roster differs".into());}
    Ok(rows)
}
fn same_foundation_methods(a:&FoundationMethods,b:&FoundationMethods)->bool {
    use workspace::objc::TypedDataMethodPreflight as Row;
    a.len()==b.len()&&a.iter().zip(b).all(|((ar,a),(br,b))|ar==br&&match(a,b){
        (Row::Existing(a),Row::Existing(b))=>a.selector==b.selector&&a.class_method==b.class_method&&a.source==b.source&&a.replacement==b.replacement,
        (Row::OwnedDeclaration{selector:a,declaration:ad},Row::OwnedDeclaration{selector:b,declaration:bd})=>a==b&&ad.base_class()==bd.base_class()&&ad.source_pins()==bd.source_pins()&&ad.implementation_code()==bd.implementation_code(),
        _=>false,
    })
}
enum FamilyObservation {
    AppleEvent {class:metadata::ClassHandle,parent:metadata::ClassHandle,methods:Methods,lifecycle:metadata::LifecycleSnapshot,parent_lifecycle:metadata::LifecycleSnapshot},
    Foundation {classes:Vec<FoundationClass>,methods:FoundationMethods,memory:Box<workspace::objc::FoundationMemoryPlan>},
}
/// Private sealed qualification consumed at the actual source factory callsite.
/// Metadata/table/header/ready data cannot manufacture this admission.
pub struct Admission {
    life: Arc<dyn AliveLease>,
    source: OwnedRoot,
    fixture: OwnedRoot,
    endpoint: PathBuf,
    endpoint_sha256: String,
    owner_uid: u32,
    observation: FamilyObservation,
    foreign_inputs: Vec<Input>,
    sources: Vec<Input>,
    manifest: Input,
}
impl Admission {
    pub fn life(&self) -> Arc<dyn AliveLease> {
        self.life.clone()
    }
    pub fn fixture(&self) -> OwnedRoot {
        self.fixture.clone()
    }
    /// Same admitted model guardian and sole arena, with exactly this family's
    /// immutable mode. No controls/App/NSApp or cross-family authority follows.
    pub fn prepare(self, model: Arc<workspace::Workspace>) -> Result<()> {
        self.life.before_effect()?;
        if !self.life.is_alive()
            || !Arc::ptr_eq(&model.lease().life, &self.life)
            || model.lease().root.path() != self.fixture.path()
        {
            return Err("data model/capability guardian differs".into());
        }
        // Installer and class-selection guard callbacks precede the final pure
        // pin check inside select_sdk_family_checked. No new guardian callback
        // may sit between that check and the first capture/write.
        unsafe {
            match &self.observation {
                FamilyObservation::AppleEvent{..}=>{
                    workspace::objc::install_apple_event_data(model.clone())?;
                    workspace::objc::prepare_apple_event_factory_checked(model, || self.prewrite())
                }
                FamilyObservation::Foundation{memory,..}=>{
                    workspace::objc::install_foundation_data(model.clone())?;
                    workspace::objc::prepare_foundation_factories_checked(model, memory.as_ref(), || self.prewrite())
                }
            }
        }
    }
    fn prewrite(&self) -> Result<()> {
        // Any dynamic liveness observation occurs BEFORE all final pin checks.
        if !self.life.is_alive()
            || self.fixture.owner_ids()?.0 != self.owner_uid
            || self.source.owner_ids()?.0 != self.owner_uid
        {
            return Err("data owner/guardian retired before factory".into());
        }
        checked(&self.source, &self.endpoint, &self.endpoint_sha256)?;
        checked(
            &self.source,
            Path::new(&self.manifest.path),
            &self.manifest.sha256,
        )?;
        for input in &self.sources {
            checked(&self.source, Path::new(&input.path), &input.sha256)?;
        }
        checked_foreign(&self.foreign_inputs,self.owner_uid)?;
        images(&self.endpoint)?;
        match &self.observation {
            FamilyObservation::AppleEvent{class,parent,methods,lifecycle,parent_lifecycle}=>{
                if class_roster()?!=(*class,*parent)||metadata::lifecycle(*class)?!=*lifecycle||metadata::lifecycle(*parent)?!=*parent_lifecycle {
                    return Err("data class/whole-six roster changed before first write".into());
                }
                let current=method_roster(&self.endpoint)?;
                if current.len()!=methods.len()||current.iter().zip(methods).any(|(a,b)|a.selector!=b.selector||a.class_method!=b.class_method||a.source!=b.source||a.replacement!=b.replacement){
                    return Err("data typed source/ABI/code changed before first write".into());
                }
            }
            FamilyObservation::Foundation{classes,methods,memory}=>{
                if foundation_classes()?!=*classes||!same_foundation_methods(&foundation_methods(&self.endpoint,&self.foreign_inputs)?,methods){
                    return Err("Foundation class/metaclass/whole-six/source/declaration/code changed before first write".into());
                }
                if workspace::objc::preflight_foundation_memory()?!=**memory {
                    return Err("Foundation owned memory plan changed before first write".into());
                }
            }
        }
        Ok(())
    }
}
/// Must be the first source-owned operation in this dedicated main entry.
/// A different role is not a fallback to Host/Core/public SDK readiness.
pub fn enter(pins: &Pins<'_>) -> Result<Admission> {
    let guard = guardian::Guard::bootstrap().map_err(|_| "data guardian admission failed")?;
    let life: Arc<dyn AliveLease> = Arc::new(GuardianAlive::admitted(guard));
    entry(pins)?;
    images(pins.endpoint)?;
    let grant = read_grant(pins, life.as_ref())?;
    if !pins.source_paths.contains(&Path::new(file!()))
        || !pins.source_paths.contains(&pins.entry_source)
        || !pins.source_paths.contains(&pins.abi_leaf)
    {
        return Err("data source roster omits gate/main/typed ABI source".into());
    }
    let foreign_inputs=grant_authority(pins,&grant)?;
    checked_foreign(&foreign_inputs,pins.owner_uid)?;
    for (fd, pin) in pins.stdio.iter().enumerate() {
        let expected = if fd < 2 {
            u32::from(libc::S_IFIFO)
        } else {
            u32::from(libc::S_IFCHR)
        };
        let access = if fd == 0 {
            libc::O_RDONLY
        } else {
            libc::O_WRONLY
        };
        if pin.kind != expected || pin.access != access || Fingerprint::actual(fd as i32)?.0 != *pin
        {
            return Err("data stdio is not exact owned pipe/null".into());
        }
    }
    let source = OwnedRoot::open(pins.source_root)?;
    let fixture = OwnedRoot::open(pins.fixture_root)?;
    if source.owner_ids()?.0 != pins.owner_uid || fixture.owner_ids()?.0 != pins.owner_uid {
        return Err("data source/fixture owner differs".into());
    }
    checked(&source, pins.endpoint, &grant.endpoint_sha256)?;
    if Path::new(&grant.source_manifest.path) != pins.source_manifest
        || Path::new(&grant.abi_leaf.path) != pins.abi_leaf
    {
        return Err("data source/ABI paths differ from compiled pins".into());
    }
    checked(&source, pins.source_manifest, &grant.source_manifest.sha256)?;
    checked(&source, pins.abi_leaf, &grant.abi_leaf.sha256)?;
    let bytes = source
        .read_optional(pins.source_manifest, 2 * 1024 * 1024)?
        .ok_or("data source manifest absent")?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| "data source manifest schema")?;
    if manifest.sources.is_empty()
        || manifest.sources.len() > 4096
        || manifest.sources.len() != pins.source_paths.len()
    {
        return Err("data source closure bound".into());
    }
    let mut previous = None;
    let mut set = Sha256::new();
    let mut abi = false;
    for (input, expected_path) in manifest.sources.iter().zip(pins.source_paths) {
        if Path::new(&input.path) != *expected_path {
            return Err("data complete source path roster differs from compiled pins".into());
        }
        if previous.as_ref().is_some_and(|p: &String| p >= &input.path) {
            return Err("data sources not strictly ordered/unique".into());
        }
        checked(&source, Path::new(&input.path), &input.sha256)?;
        if input.path == grant.abi_leaf.path && input.sha256 == grant.abi_leaf.sha256 {
            abi = true;
        }
        set.update(input.path.as_bytes());
        set.update([0]);
        set.update(input.sha256.as_bytes());
        set.update(b"\n");
        previous = Some(input.path.clone());
    }
    if !abi || format!("{:x}", set.finalize()) != grant.source_set_sha256 {
        return Err("data source/compiled Rust ABI closure differs".into());
    }
    for address in [
        enter as *const () as usize,
        Admission::prepare as *const () as usize,
    ] {
        if metadata::code_identity(address)?.image() != pins.endpoint.as_os_str().as_encoded_bytes()
        {
            return Err("data gate is not source-included main code".into());
        }
    }
    let observation=match pins.family {
        DataFamily::AppleEventDescriptor=>{
            let (class,parent)=class_roster()?;
            FamilyObservation::AppleEvent{class,parent,methods:method_roster(pins.endpoint)?,lifecycle:metadata::lifecycle(class)?,parent_lifecycle:metadata::lifecycle(parent)?}
        }
        DataFamily::FoundationData=>FamilyObservation::Foundation{classes:foundation_classes()?,methods:foundation_methods(pins.endpoint,&foreign_inputs)?,memory:Box::new(workspace::objc::preflight_foundation_memory()?)},
    };
    life.before_effect()?;
    if !life.is_alive() {
        return Err("data guardian retired during admission".into());
    }
    Ok(Admission {
        life,
        source,
        fixture,
        endpoint: pins.endpoint.into(),
        endpoint_sha256: grant.endpoint_sha256,
        owner_uid: pins.owner_uid,
        observation,
        foreign_inputs,
        sources: manifest.sources,
        manifest: grant.source_manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::fd::{AsRawFd, IntoRawFd},
    };
    struct Life;
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            true
        }
        fn before_effect(&self) -> Result<()> {
            Ok(())
        }
    }
    fn pins(fd: i32, fp: Fingerprint, owner_uid: u32) -> Pins<'static> {
        Pins {
            family: DataFamily::AppleEventDescriptor,
            source_root: Path::new("/unqueried-data-source"),
            fixture_root: Path::new("/unqueried-data-fixture"),
            endpoint: Path::new("/unqueried-data-main"),
            source_manifest: Path::new("/unqueried-data-manifest"),
            abi_leaf: Path::new("/unqueried-data-ABI"),
            entry_source: Path::new("/unqueried-data-entry"),
            source_paths: &[],
            foreign_inputs: &[],
            nonce_sha256: "",
            owner_uid,
            grant_fd: fd,
            grant: fp,
            stdio: [fp; 3],
        }
    }
    #[test]
    fn closed_family_refuses_other_roles_and_unknown_grant_fields() {
        assert!(serde_json::from_str::<DataFamily>("\"NSApplication\"").is_err());
        assert!(serde_json::from_str::<DataFamily>("\"full-owned-host-v1\"").is_err());
        let value = serde_json::json!({"role":ROLE,"family":"AppleEventDescriptor","owner_uid":0,"nonce":"own","endpoint_sha256":"","source_set_sha256":"","source_manifest":{"path":"/own","sha256":""},"abi_leaf":{"path":"/own","sha256":""},"foreign_inputs":[],"stdio":([Fingerprint{device:0,inode:0,kind:0,access:0};3]),"ready":true});
        assert!(serde_json::from_value::<Grant>(value).is_err());
    }
    #[test]
    fn foundation_role_and_foreign_inputs_are_closed_and_independently_pinned() {
        let declarations=compiled_foreign(DataFamily::FoundationData).unwrap();
        let zero=declarations[0].1.clone();let one=declarations[1].1.clone();let nonce=hash(b"own");
        let sdk_nsstring=declarations[0].0.as_path();let rust_nsstring=declarations[1].0.as_path();
        let fp=Fingerprint{device:0,inode:0,kind:u32::from(libc::S_IFIFO),access:libc::O_RDONLY};
        let foreign=[ForeignInputPin{path:sdk_nsstring,sha256:&zero},ForeignInputPin{path:rust_nsstring,sha256:&one}];
        let p=Pins{family:DataFamily::FoundationData,foreign_inputs:&foreign,nonce_sha256:&nonce,..pins(9,fp,501)};
        let mut grant=Grant{role:FOUNDATION_ROLE.into(),family:DataFamily::FoundationData,owner_uid:501,nonce:"own".into(),endpoint_sha256:zero.clone(),source_set_sha256:zero.clone(),source_manifest:Input{path:"/unqueried".into(),sha256:zero.clone()},abi_leaf:Input{path:"/unqueried".into(),sha256:zero.clone()},foreign_inputs:vec![Input{path:sdk_nsstring.to_str().unwrap().into(),sha256:zero.clone()},Input{path:rust_nsstring.to_str().unwrap().into(),sha256:one.clone()}],stdio:[fp;3]};
        assert_eq!(grant_authority(&p,&grant).unwrap(),grant.foreign_inputs);
        // This refusal precedes declaration I/O even when path/hash pins match.
        let wrong_cap_owner=declarations[1].2.wrapping_add(1);
        assert_eq!(checked_foreign(&grant.foreign_inputs,wrong_cap_owner).unwrap_err(),
            "foreign declaration owner differs from system/cap identity");
        assert_eq!(DataFamily::FoundationData.argument(),b"__f1-foundation-data");
        assert_eq!(DataFamily::AppleEventDescriptor.argument(),b"__f1-appleevent-data");
        grant.family=DataFamily::AppleEventDescriptor;
        assert!(grant_authority(&p,&grant).is_err());
        grant.family=DataFamily::FoundationData;grant.role=ROLE.into();
        assert!(grant_authority(&p,&grant).is_err());
        grant.role=FOUNDATION_ROLE.into();
        grant.foreign_inputs.swap(0,1);assert!(grant_authority(&p,&grant).is_err());grant.foreign_inputs.swap(0,1);
        grant.foreign_inputs[0].sha256=one.clone();assert!(grant_authority(&p,&grant).is_err());grant.foreign_inputs[0].sha256=zero.clone();
        let removed=grant.foreign_inputs.pop().unwrap();assert!(grant_authority(&p,&grant).is_err());grant.foreign_inputs.push(removed.clone());
        grant.foreign_inputs.push(removed);assert!(grant_authority(&p,&grant).is_err());grant.foreign_inputs.pop();
        assert!(foreign_roster(DataFamily::AppleEventDescriptor,&[],&grant.foreign_inputs).is_err());
        assert!(foreign_roster(DataFamily::AppleEventDescriptor,&[],&[]).unwrap().is_empty());
        assert!(foreign_roster(DataFamily::FoundationData,&[],&[]).is_err());
        let mut escaped=foreign;escaped[0].path=Path::new("/ungranted/NSString.h");
        assert!(foreign_roster(DataFamily::FoundationData,&escaped,&grant.foreign_inputs).is_err());
        let keys=foundation_keys();assert_eq!(keys.len(),28);
        for denied in [c"NSPasteboard",c"NSPasteboardItem",c"NSApplication",c"NSAppleEventDescriptor",c"NSMutableData",c"__NSCFBoolean"] {
            assert!(!keys.iter().any(|(role,_,_)|*role==denied));
        }
    }
    #[test]
    fn foreign_file_reader_checks_real_owner_links_permissions_and_content() {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let parent=std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT").expect("explicit owned fixture parent required");
        let parent=OwnedRoot::open(Path::new(&parent)).unwrap();
        let directory=parent.path().join(format!("foundation-gate-{}",std::process::id()));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();let _cleanup=Cleanup(directory.clone());
        let path=directory.join("own-declaration");
        use std::io::Write;
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
        file.write_all(b"owned declaration bytes").unwrap();drop(file);
        let owner=std::fs::metadata(&path).unwrap().uid();let digest=hash(b"owned declaration bytes");
        read_declaration(&path,&digest,owner).unwrap();
        assert!(read_declaration(&path,&digest,owner^1).is_err());
        assert!(read_declaration(&path,&"0".repeat(64),owner).is_err());
        let link=directory.join("link");std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(read_declaration(&link,&digest,owner).is_err());
        let hard=directory.join("hard");std::fs::hard_link(&path,&hard).unwrap();
        assert!(read_declaration(&path,&digest,owner).is_err());std::fs::remove_file(hard).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o622)).unwrap();
        assert!(read_declaration(&path,&digest,owner).is_err());
    }
    #[test]
    fn actual_privatecap_owner_check_precedes_read_and_callback() {
        struct NoCallback;
        impl AliveLease for NoCallback {
            fn is_alive(&self) -> bool {
                panic!("UID must refuse before callback")
            }
            fn before_effect(&self) -> Result<()> {
                panic!("UID must refuse before callback")
            }
        }
        let (read, _write) = guardian::pipe().unwrap();
        let fd = read.as_raw_fd();
        let (fp, uid) = Fingerprint::actual(fd).unwrap();
        let error = match read_grant(&pins(fd, fp, uid ^ 1), &NoCallback) {
            Ok(_) => panic!("wrong owner admitted"),
            Err(e) => e,
        };
        assert!(error.contains("private readcap/owner"));
        assert_eq!(Fingerprint::actual(fd).unwrap().0, fp);
    }
    #[test]
    fn actual_privatecap_valid_schema_requires_eof_and_consumes_only_its_readcap() {
        let (read, write) = guardian::pipe().unwrap();
        let fd = read.as_raw_fd();
        let (fp, uid) = Fingerprint::actual(fd).unwrap();
        let grant = Grant {
            role: ROLE.into(),
            family: DataFamily::AppleEventDescriptor,
            owner_uid: uid,
            nonce: "own-fixture-nonce".into(),
            endpoint_sha256: "0".repeat(64),
            source_set_sha256: "0".repeat(64),
            source_manifest: Input {
                path: "/unqueried".into(),
                sha256: "0".repeat(64),
            },
            abi_leaf: Input {
                path: "/unqueried".into(),
                sha256: "0".repeat(64),
            },
            foreign_inputs: Vec::new(),
            stdio: [fp; 3],
        };
        let bytes = serde_json::to_vec(&grant).unwrap();
        let mut writer = std::fs::File::from(write);
        writer
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .unwrap();
        writer.write_all(&bytes).unwrap();
        drop(writer);
        let raw = read.into_raw_fd();
        let result = read_grant(&pins(raw, fp, uid), &Life).unwrap();
        assert_eq!(result.owner_uid, uid);
        assert_eq!(result.family, DataFamily::AppleEventDescriptor);
        assert_eq!(unsafe { libc::fcntl(raw, libc::F_GETFD) }, -1);
        let (read, write) = guardian::pipe().unwrap();
        let fd = read.as_raw_fd();
        let (fp, uid) = Fingerprint::actual(fd).unwrap();
        let mut writer = std::fs::File::from(write);
        writer
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .unwrap();
        writer.write_all(&bytes).unwrap();
        let raw = read.into_raw_fd();
        let error = match read_grant(&pins(raw, fp, uid), &Life) {
            Ok(_) => panic!("live writer accepted as immutable snapshot"),
            Err(e) => e,
        };
        assert!(error.contains("immutable EOF"));
        drop(writer);
    }
    #[test]
    fn synthetic_main_roster_rejects_initializers_interpose_and_nonlazy_objc() {
        fn main(command: u32, name: &[u8], flags: u32) -> Vec<u8> {
            let size = if command == 0x19 { 152 } else { 8 };
            let mut b = vec![0u8; 32 + size];
            b[0..4].copy_from_slice(&0xfeedfacfu32.to_ne_bytes());
            b[12..16].copy_from_slice(&2u32.to_ne_bytes());
            b[16..20].copy_from_slice(&1u32.to_ne_bytes());
            b[20..24].copy_from_slice(&(size as u32).to_ne_bytes());
            b[32..36].copy_from_slice(&command.to_ne_bytes());
            b[36..40].copy_from_slice(&(size as u32).to_ne_bytes());
            if command == 0x19 {
                b[96..100].copy_from_slice(&1u32.to_ne_bytes());
                b[104..104 + name.len()].copy_from_slice(name);
                b[144..152].copy_from_slice(&8u64.to_ne_bytes());
                b[168..172].copy_from_slice(&flags.to_ne_bytes());
            }
            b
        }
        for cmd in [0x11, 0x1a] {
            let b = main(cmd, b"", 0);
            assert!(unsafe { image_header(b.as_ptr()) }.is_err());
        }
        for name in [
            b"__interpose".as_slice(),
            b"__objc_nlclslist",
            b"__objc_nlcatlist",
        ] {
            let b = main(0x19, name, 0);
            assert!(unsafe { image_header(b.as_ptr()) }.is_err());
        }
        for flags in [9, 0x15, 0x16] {
            let b = main(0x19, b"__unrelated", flags);
            assert!(unsafe { image_header(b.as_ptr()) }.is_err());
        }
        let b = main(0x19, b"__objc_classlist", 0);
        unsafe { image_header(b.as_ptr()) }.unwrap();
        let b = main(0x80000034, b"", 0);
        unsafe { image_header(b.as_ptr()) }.unwrap();
    }
    #[test]
    fn environment_unknown_keys_and_invalid_fds_are_distinct_and_never_echo_values() {
        let secret = "private-value-never-echoed";
        for key in [
            "UNEXPECTED",
            "DYLD_INSERT_LIBRARIES",
            "__CF_USER_TEXT_ENCODING",
        ] {
            let error = environment_fd(key, secret).unwrap_err();
            assert!(error.contains("unexpected key"));
            assert!(error.contains(key));
            assert!(!error.contains(secret));
            assert!(!error.contains("invalid FD"));
        }
        for (key, field) in [
            ("F1_LIFE_FD", EnvironmentFd::Life),
            ("F1_ADMIT_FD", EnvironmentFd::Admit),
            ("F1_DATA_GRANT_FD", EnvironmentFd::Grant),
        ] {
            assert_eq!(environment_fd(key, "17").unwrap(), (field, 17));
            for value in [secret, "", "2147483648"] {
                assert_eq!(
                    environment_fd(key, value).unwrap_err(),
                    format!("data environment invalid FD for {key}")
                );
            }
        }
        let key = format!("{}\n{}", "X".repeat(47), "Y".repeat(1000));
        let error = environment_fd(&key, secret).unwrap_err();
        assert!(error.contains("\\x0a..."));
        assert!(!error.contains('\n'));
        assert!(!error.contains('Y'));
        assert!(error.len() < 256);
        assert!(!error.contains(secret));
    }
    #[test]
    fn exact_four_field_producer_context_protocol_is_fail_closed() {
        fn fields(owner: u32, pairs: &[(&str, &str)], grant: i32) -> Result<()> {
            let mut state = EntryEnvironment::default();
            for (key, value) in pairs {
                state.push(key, value, owner)?;
            }
            state.finish(grant)
        }
        let fds = [
            ("F1_LIFE_FD", "7"),
            ("F1_ADMIT_FD", "8"),
            ("F1_DATA_GRANT_FD", "9"),
        ];
        for (owner, canonical) in [
            (0, "0x0:0x0:0x0"),
            (501, "0x1F5:0x0:0x0"),
            (u32::MAX, "0xFFFFFFFF:0x0:0x0"),
        ] {
            assert_eq!(cf_context(owner), canonical);
            let mut valid = fds.to_vec();
            valid.push((CF_CONTEXT_KEY, canonical));
            fields(owner, &valid, 9).unwrap();
            valid.rotate_left(1);
            fields(owner, &valid, 9).unwrap();
            for omit in 0..valid.len() {
                let incomplete: Vec<_> = valid
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != omit)
                    .map(|(_, p)| *p)
                    .collect();
                assert!(fields(owner, &incomplete, 9).is_err());
            }
            for pair in &valid {
                let mut duplicate = valid.clone();
                duplicate.push(*pair);
                assert!(
                    fields(owner, &duplicate, 9)
                        .unwrap_err()
                        .contains("duplicate")
                );
            }
            let mut extra = valid.clone();
            extra.push(("OTHER_CONTEXT", "private-value"));
            let error = fields(owner, &extra, 9).unwrap_err();
            assert!(error.contains("unexpected key"));
            assert!(!error.contains("private-value"));
            assert!(fields(owner, &valid, 10).is_err());
        }
        for wrong in [
            "0x1f5:0x0:0x0",
            "501:0:0",
            "0x1F6:0x0:0x0",
            "0x1F5:0:0",
            "0x1F5:0x1:0x0",
            "0x1F5:0x0:0x1",
            "0x1F5:0x0:0x0 ",
            "0x1F5:0x0:0x0:extra",
            "",
        ] {
            let mut pairs = fds.to_vec();
            pairs.push((CF_CONTEXT_KEY, wrong));
            assert_eq!(
                fields(501, &pairs, 9).unwrap_err(),
                "data environment mismatched producer CF context"
            );
        }
        assert_eq!(
            fields(501, &fds, 9).unwrap_err(),
            "data environment missing producer CF context"
        );
        let mut alias = vec![
            ("F1_LIFE_FD", "7"),
            ("F1_ADMIT_FD", "7"),
            ("F1_DATA_GRANT_FD", "9"),
            (CF_CONTEXT_KEY, "0x1F5:0x0:0x0"),
        ];
        assert!(fields(501, &alias, 9).is_err());
        alias[0] = ("F1_LIFE_FD", "2");
        assert!(fields(501, &alias, 9).is_err());
    }
    #[test]
    fn foundation_metadata_images_are_exact_per_class_without_method_grant() {
        let expected:[(&str,&[u8]);8]=[
            ("NSObject",b"/usr/lib/libobjc.A.dylib"),
            ("NSValue",b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"),
            ("NSString",b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"),
            ("NSData",b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"),
            ("NSURL",b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"),
            ("NSNumber",b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"),
            ("NSArray",b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"),
            ("NSDictionary",b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"),
        ];
        assert_eq!(FOUNDATION_CLASS_IMAGES.len(),expected.len());
        for (index,(name,image)) in expected.into_iter().enumerate() {
            assert_eq!(FOUNDATION_CLASS_IMAGES[index],image,"{name}");
            for wrong in [OBJC,FOUNDATION,CORE_FOUNDATION] {
                assert_eq!(FOUNDATION_CLASS_IMAGES[index]==wrong,image==wrong,"{name}");
            }
        }
        assert!(![FOUNDATION,OBJC].contains(&CORE_FOUNDATION));
    }

    #[test]
    fn foundation_existing_code_images_are_pinned_per_exact_method_key() {
        let core=[
            (c"NSArray",c"arrayWithObjects:count:",true),
            (c"NSArray",c"initWithObjects:count:",false),
            (c"NSArray",c"count",false),
            (c"NSArray",c"objectAtIndex:",false),
            (c"NSArray",c"countByEnumeratingWithState:objects:count:",false),
            (c"NSDictionary",c"initWithObjects:forKeys:count:",false),
        ];
        let mut counts=[0,0,0];
        for (role,selector,meta) in foundation_keys() {
            match foundation_existing_method_image(role,selector,meta) {
                None=>{assert_eq!((role,selector,meta),(c"NSString",c"initWithBytes:length:encoding:",false));counts[2]+=1;},
                Some(image)=>{
                    let expected=if core.contains(&(role,selector,meta)){counts[0]+=1;CORE_FOUNDATION}else{counts[1]+=1;FOUNDATION};
                    assert_eq!(image,expected);
                    assert_ne!(image,OBJC);
                }
            }
        }
        assert_eq!(counts,[6,21,1]);
        for (role,selector,meta) in core {
            assert_eq!(foundation_existing_method_image(role,selector,!meta),None);
        }
        assert_eq!(foundation_existing_method_image(c"NSPasteboard",c"count",false),None);
        assert_eq!(foundation_existing_method_image(c"NSMutableArray",c"count",false),None);
        assert_eq!(foundation_existing_method_image(c"NSArray",c"unknown",false),None);
        assert_eq!(foundation_existing_method_image(c"NSData",c"bytes",false),Some(FOUNDATION));
        assert_eq!(foundation_existing_method_image(c"NSURL",c"absoluteString",false),Some(FOUNDATION));
    }

}
