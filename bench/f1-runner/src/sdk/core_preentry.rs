//! PREPARED restricted CoreCLI entry. This is not the fourteen-scope host gate.
//! No actual CLI execution is authorized by this module or a ready marker.
//! The separately reviewed producer must pin the exact original CLI source,
//! artifact, this SDK artifact and the private typed-stdio preparation recipe.
use super::{
    descriptors::DescriptorTable,
    import_patch,
    owned::{AliveLease, OwnedRoot, Result},
    process_abi,
    process_dispatch::{PinnedFake, Registry, Rule},
    process_fork::ForkAwareLease,
};
use crate::{guardian, guardian_alive::GuardianAlive};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::{c_char, CStr},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
const ROLE: &str = "core-supervisor-only-v1";
const MAX_GRANT: usize = 16384;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub kind: u32,
    pub access: i32,
}
impl Fingerprint {
    fn own(fd: i32) -> Result<Self> {
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fstat(fd, &mut s) } != 0 {
            return Err("owned descriptor unavailable".into());
        }
        Ok(Self {
            device: s.st_dev as u64,
            inode: s.st_ino,
            kind: u32::from(s.st_mode & libc::S_IFMT),
            access: flags & libc::O_ACCMODE,
        })
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub role: String,
    pub endpoint_sha256: String,
    pub source_set_sha256: String,
    pub sdk_artifact_sha256: String,
    pub nonce: String,
    pub stdio: [Fingerprint; 3],
}
/// These pins are compile-time fields in the reviewed constructor producer.
/// Descriptor identities are captured only by OwnedIo::new, held by that same
/// producer across SDK build/admission; no raw-FD fingerprint is a grant API.
pub struct Pins<'a> {
    pub root: &'a Path,
    pub endpoint: &'a Path,
    pub endpoint_sha256: &'a str,
    pub source_set_sha256: &'a str,
    pub sdk_artifact: &'a Path,
    pub nonce_sha256: &'a str,
    pub grant: Fingerprint,
    pub stdio: [Fingerprint; 3],
    pub sources: &'a [(&'a str, &'a str)],
    pub provider: &'a Path,
    pub provider_sha256: &'a str,
    pub provider_source: &'a Path,
    pub provider_source_sha256: &'a str,
}
#[link(name = "System")]
unsafe extern "C" {
    fn _NSGetArgc() -> *const i32;
    fn _NSGetArgv() -> *const *const *const c_char;
    fn _NSGetEnviron() -> *const *const *const c_char;
    fn _NSGetExecutablePath(buffer: *mut c_char, size: *mut u32) -> i32;
    fn _NSGetMachExecuteHeader() -> *const u8;
}
unsafe fn bounded(p: *const c_char, max: usize) -> Result<Vec<u8>> {
    if p.is_null() {
        return Err("missing CRT value".into());
    }
    let mut n = 0;
    while n <= max && unsafe { p.add(n).read() } != 0 {
        n += 1;
    }
    if n > max {
        return Err("CRT bounds".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(p.cast(), n) }.to_vec())
}
fn args(endpoint: &Path) -> Result<()> {
    unsafe {
        let argc = _NSGetArgc();
        let argv = _NSGetArgv();
        if argc.is_null() || argv.is_null() || *argc != 2 || (*argv).is_null() {
            return Err("only exact supervised entry is permitted".into());
        }
        let first = bounded((*argv).read(), 8192)?;
        let second = bounded((*argv).add(1).read(), 128)?;
        if first != endpoint.as_os_str().as_encoded_bytes()
            || second != b"__run-script-supervised"
            || !(*argv).add(2).read().is_null()
        {
            return Err("only exact supervised entry is permitted".into());
        }
    }
    Ok(())
}
fn environment(sdk: &Path) -> Result<(i32, i32, i32)> {
    unsafe {
        let raw = _NSGetEnviron();
        if raw.is_null() || (*raw).is_null() {
            return Err("CRT environment absent".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut life = None;
        let mut admit = None;
        let mut grant = None;
        for i in 0..32 {
            let p = (*raw).add(i).read();
            if p.is_null() {
                let (a, b, c) = (
                    life.ok_or("life missing")?,
                    admit.ok_or("admit missing")?,
                    grant.ok_or("grant missing")?,
                );
                if seen.len() != 6 || a <= 2 || b <= 2 || c <= 2 || a == b || a == c || b == c {
                    return Err("capability/stdio alias or missing entry field".into());
                }
                return Ok((a, b, c));
            }
            let bytes = bounded(p, 8192)?;
            let entry = std::str::from_utf8(&bytes).map_err(|_| "environment UTF8")?;
            let (key, value) = entry.split_once('=').ok_or("environment encoding")?;
            if !seen.insert(key.to_owned()) {
                return Err("duplicate entry field".into());
            }
            match key {
                "F1_LIFE_FD" => life = Some(value.parse::<i32>().map_err(|_| "life encoding")?),
                "F1_ADMIT_FD" => admit = Some(value.parse::<i32>().map_err(|_| "admit encoding")?),
                "F1_CORE_GRANT_FD" => {
                    grant = Some(value.parse::<i32>().map_err(|_| "grant encoding")?)
                }
                "F1_CORE_ROLE" if value == ROLE => (),
                "DYLD_INSERT_LIBRARIES"
                    if value.as_bytes() == sdk.as_os_str().as_encoded_bytes() => {}
                "DYLD_BIND_AT_LAUNCH" if value == "1" => {}
                _ => return Err("unexpected entry environment; no fallback".into()),
            }
        }
    }
    Err("environment count".into())
}
fn image(endpoint: &Path) -> Result<()> {
    let mut buffer = [0i8; 8192];
    let mut size = buffer.len() as u32;
    if unsafe { _NSGetExecutablePath(buffer.as_mut_ptr(), &mut size) } != 0 {
        return Err("own executable path unavailable".into());
    }
    if unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes()
        != endpoint.as_os_str().as_encoded_bytes()
    {
        return Err("own loader image differs".into());
    }
    Ok(())
}
fn no_main_initializers() -> Result<()> {
    // The loader supplies this actual MH_EXECUTE header, never a caller packet.
    unsafe { check_main_initializers(_NSGetMachExecuteHeader()) }
}
fn nonlazy_objc_initializer(name: &[u8]) -> bool {
    matches!(name.split(|b| *b == 0).next(),
        Some(b"__objc_nlclslist" | b"__objc_nlcatlist"))
}
unsafe fn check_main_initializers(h: *const u8) -> Result<()> {
    unsafe {
        if h.is_null()
            || h.cast::<u32>().read_unaligned() != 0xfeedfacf
            || h.add(12).cast::<u32>().read_unaligned() != 2
        {
            return Err("main image is not MachO64 execute".into());
        }
        let count = h.add(16).cast::<u32>().read_unaligned() as usize;
        let bytes = h.add(20).cast::<u32>().read_unaligned() as usize;
        if count > 4096 || bytes > 16 * 1024 * 1024 {
            return Err("main initializer command bounds".into());
        }
        let mut at = 0;
        for _ in 0..count {
            if at + 8 > bytes {
                return Err("main initializer command truncated".into());
            }
            let command = h.add(32 + at);
            let kind = command.cast::<u32>().read_unaligned();
            let n = command.add(4).cast::<u32>().read_unaligned() as usize;
            if n < 8 || at + n > bytes {
                return Err("main initializer command invalid".into());
            }
            if matches!(kind, 0x11 | 0x1a) {
                return Err("unqualified main LC_ROUTINES initializer".into());
            }
            if kind == 0x19 {
                if n < 72 {
                    return Err("main segment truncated".into());
                }
                let sections = command.add(64).cast::<u32>().read_unaligned() as usize;
                if sections > 4096 || 72 + sections * 80 > n {
                    return Err("main initializer section bounds".into());
                }
                for i in 0..sections {
                    let section = command.add(72 + i * 80);
                    let flags = section.add(64).cast::<u32>().read_unaligned() & 0xff;
                    let size = section.add(40).cast::<u64>().read_unaligned();
                    if size != 0 && (matches!(flags, 9 | 0x15 | 0x16)
                        || nonlazy_objc_initializer(std::slice::from_raw_parts(section, 16))) {
                        return Err(
                            "unqualified original-main initializer; no callback permitted".into(),
                        );
                    }
                }
            }
            at += n;
        }
    }
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn checked_hash(root: &OwnedRoot, path: &Path, expected: &str) -> Result<()> {
    if expected.len() != 64 {
        return Err("missing artifact/source pin".into());
    }
    let bytes = root
        .read_optional(path, 64 * 1024 * 1024)?
        .ok_or("pinned input missing")?;
    if digest(&bytes) != expected {
        return Err("pinned input changed".into());
    }
    Ok(())
}
fn read_grant(fd: i32, pin: Fingerprint, life: &dyn AliveLease) -> Result<Grant> {
    // Compare against the producer's COMPILED private-pipe identity BEFORE reading
    // any byte. A random readable pipe, stdio alias or manifest assertion is not
    // ownership. The producer keeps that backing alive across SDK build/admission.
    if pin.kind != u32::from(libc::S_IFIFO)
        || pin.access != libc::O_RDONLY
        || Fingerprint::own(fd)? != pin
    {
        return Err("grant is not the exact retained owned pipe".into());
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err("grant pipe setup failed".into());
    }
    let end = Instant::now() + Duration::from_secs(3);
    let read = |out: &mut [u8]| -> Result<()> {
        let mut at = 0;
        while at < out.len() {
            life.before_effect()?;
            if Instant::now() >= end {
                return Err("grant pipe deadline".into());
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
                return Err("grant pipe failure".into());
            }
            if n == 0 {
                continue;
            }
            let n = unsafe { libc::read(fd, out.as_mut_ptr().add(at).cast(), out.len() - at) };
            if n > 0 {
                at += n as usize;
            } else if n == 0 {
                return Err("grant pipe incomplete".into());
            } else if !matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                return Err("grant pipe read failure".into());
            }
        }
        Ok(())
    };
    let mut n = [0; 4];
    read(&mut n)?;
    let n = u32::from_ne_bytes(n) as usize;
    if n > MAX_GRANT {
        return Err("grant packet size".into());
    }
    let mut bytes = vec![0; n];
    read(&mut bytes)?;
    drop(descriptor);
    serde_json::from_slice(&bytes).map_err(|_| "grant packet schema".into())
}
fn stdio(pins: &[Fingerprint; 3]) -> Result<()> {
    for (i, pin) in pins.iter().enumerate() {
        let valid = match i {
            0 => pin.kind == u32::from(libc::S_IFIFO) && pin.access == libc::O_RDONLY,
            1 => pin.kind == u32::from(libc::S_IFIFO) && pin.access == libc::O_WRONLY,
            2 => pin.kind == u32::from(libc::S_IFCHR) && pin.access == libc::O_WRONLY,
            _ => false,
        };
        if !valid || Fingerprint::own(i as i32)? != *pin {
            return Err("stdio is not exact retained typed own input/output/null".into());
        }
    }
    Ok(())
}
pub struct Admitted {
    pub life: Arc<dyn AliveLease>,
    pub descriptors: Arc<DescriptorTable>,
    pub imports: Vec<super::ImportReplacement>,
    pub stdio: [Fingerprint; 3],
}
/// Constructor-only restricted entry. It does not emit ready JSON, read fd0,
/// initialize a profile, invoke a provider or make the full SDK ready.
pub fn enter(pins: &Pins<'_>) -> Result<Admitted> {
    let guard = guardian::Guard::bootstrap().map_err(|_| "core guardian admission failed")?;
    enter_with_guard(pins, guard)
}
pub fn enter_with_guard(pins: &Pins<'_>, guard: guardian::Guard) -> Result<Admitted> {
    let life: Arc<dyn AliveLease> = Arc::new(ForkAwareLease::new(Arc::new(
        GuardianAlive::admitted(guard),
    )));
    args(pins.endpoint)?;
    let (_, _, grant_fd) = environment(pins.sdk_artifact)?;
    image(pins.endpoint)?;
    no_main_initializers()?;
    let grant = read_grant(grant_fd, pins.grant, life.as_ref())?;
    if grant.role != ROLE
        || grant.endpoint_sha256 != pins.endpoint_sha256
        || grant.source_set_sha256 != pins.source_set_sha256
        || digest(grant.nonce.as_bytes()) != pins.nonce_sha256
        || grant.stdio != pins.stdio
    {
        return Err("core entry authority/source/stdio differs".into());
    }
    stdio(&pins.stdio)?;
    let root = OwnedRoot::open(pins.root)?;
    checked_hash(&root, pins.endpoint, pins.endpoint_sha256)?;
    checked_hash(&root, pins.sdk_artifact, &grant.sdk_artifact_sha256)?;
    let mut ordered = std::collections::BTreeMap::new();
    for (path, sha) in pins.sources {
        if ordered.insert(*path, *sha).is_some() {
            return Err("duplicate source input".into());
        }
        checked_hash(&root, Path::new(path), sha)?;
    }
    let mut source_set = Sha256::new();
    for (path, sha) in ordered {
        source_set.update(path.as_bytes());
        source_set.update([0]);
        source_set.update(sha.as_bytes());
        source_set.update(b"\n");
    }
    if format!("{:x}", source_set.finalize()) != pins.source_set_sha256 {
        return Err("source set does not match ordered input roster".into());
    }
    let owner_uid = root.owner_ids()?.0;
    let fake = Arc::new(PinnedFake::bind(
        root.clone(),
        pins.provider.into(),
        pins.provider_sha256,
        pins.provider_source.into(),
        pins.provider_source_sha256,
    )?);
    let descriptors = DescriptorTable::new();
    let mut registry = Registry::new(life.clone(), descriptors.clone())?;
    registry.register(
        Path::new("/owned/core-provider-success"),
        &[],
        Rule {
            fake: fake.clone(),
            fake_args: vec!["ps".into()],
        },
    )?;
    registry.register(
        Path::new("/owned/core-provider-exit-17"),
        &[],
        Rule {
            fake,
            fake_args: vec!["exit-17".into()],
        },
    )?;
    process_abi::install(process_abi::Context {
        registry: Arc::new(registry),
        life: life.clone(),
        owner_uid,
    })?;
    let all = process_abi::import_specs();
    // Exact original artifact1699c029...0465 has13 imported names. These seven
    // are absent, not unselected source calls. No artificial import anchor is
    // inserted. Review must bind the forced empty pre_exec/no-cwd source graph.
    const ABSENT: [&CStr; 7] = [
        c"posix_spawn",
        c"posix_spawn_file_actions_addclose",
        c"posix_spawn_file_actions_addopen",
        c"posix_spawn_file_actions_addinherit_np",
        c"posix_spawn_file_actions_addchdir_np",
        c"posix_spawn_file_actions_addfchdir_np",
        c"execve",
    ];
    let observed = super::observe_main_imports(&all)?;
    let present = observed
        .iter()
        .map(|i| i.symbol)
        .collect::<std::collections::BTreeSet<_>>();
    let absent = all
        .iter()
        .filter(|s| !present.contains(s.symbol))
        .map(|s| s.symbol)
        .collect::<std::collections::BTreeSet<_>>();
    if absent != ABSENT.into_iter().collect() {
        return Err("original CLI import graph changed".into());
    }
    let active = all
        .into_iter()
        .filter(|s| present.contains(s.symbol))
        .collect::<Vec<_>>();
    let imports = import_patch::install(&active, life.as_ref())?;
    // Every required actual import must be selected before original Rust runtime/
    // main executes. Missing/lazy/PAC/old cells fail closed, never call originals.
    Ok(Admitted {
        life,
        descriptors,
        imports,
        stdio: pins.stdio,
    })
}
static ADMITTED: std::sync::OnceLock<Admitted> = std::sync::OnceLock::new();
/// A dedicated, separately pinned SDK image invokes this from __mod_init_func.
/// Any refusal terminates before original main/ready JSON with exit86. No
/// incoming env/receipt can switch this adapter to a normal/default CLI role.
pub fn constructor(pins: &Pins<'_>) {
    match enter(pins) {
        Ok(value) => {
            if ADMITTED.set(value).is_err() {
                unsafe {
                    libc::_exit(86);
                }
            }
        }
        Err(_) => unsafe { libc::_exit(86) },
    }
}
/// Private own-IO preparation. Fields are intentionally private: no arbitrary
/// integer/fstat guess can manufacture the grant fingerprint via this API.
pub struct OwnedIo {
    input: OwnedFd,
    input_writer: OwnedFd,
    output: OwnedFd,
    output_reader: OwnedFd,
    error: OwnedFd,
    grant: OwnedFd,
    grant_writer: OwnedFd,
}
impl OwnedIo {
    pub fn new() -> Result<Self> {
        let (input, input_writer) = guardian::pipe().map_err(|_| "own input pipe")?;
        let (output_reader, output) = guardian::pipe().map_err(|_| "own output pipe")?;
        let (grant, grant_writer) = guardian::pipe().map_err(|_| "own grant pipe")?;
        let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err("own null sink unavailable".into());
        }
        let error = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            input,
            input_writer,
            output,
            output_reader,
            error,
            grant,
            grant_writer,
        })
    }
    pub fn pins(&self) -> Result<(Fingerprint, [Fingerprint; 3])> {
        Ok((
            Fingerprint::own(self.grant.as_raw_fd())?,
            [
                Fingerprint::own(self.input.as_raw_fd())?,
                Fingerprint::own(self.output.as_raw_fd())?,
                Fingerprint::own(self.error.as_raw_fd())?,
            ],
        ))
    }
    /// Build-only review exporter; it does not spawn an endpoint. The IO holder
    /// must stay alive through the later separately authorized SDK-build+admission.
    pub fn preparation(&self) -> Result<serde_json::Value> {
        let (grant, stdio) = self.pins()?;
        Ok(
            serde_json::json!({"state":"prepared-only; not runnable authority","grant":grant,"stdio":stdio,"childInputFd":self.input.as_raw_fd(),"parentInputFd":self.input_writer.as_raw_fd(),"childOutputFd":self.output.as_raw_fd(),"parentOutputFd":self.output_reader.as_raw_fd(),"childErrorFd":self.error.as_raw_fd(),"grantReadFd":self.grant.as_raw_fd(),"grantWriteFd":self.grant_writer.as_raw_fd()}),
        )
    }
}

pub fn constructor_with_guard(pins: &Pins<'_>, guard: guardian::Guard) {
    match enter_with_guard(pins, guard) {
        Ok(value) => {
            if ADMITTED.set(value).is_err() {
                unsafe {
                    libc::_exit(86);
                }
            }
        }
        Err(_) => unsafe { libc::_exit(86) },
    }
}
/// Emits the actual Darwin initializer only in the dedicated helper SDK image.
/// make_pins is compiled source from the typed own-IO producer, never a runtime
/// profile/receipt parser. Guardian admission precedes even its construction.
#[macro_export]
macro_rules! core_supervisor_initializer {
    ($make_pins:path) => {
        #[cfg(rcf1_reference_sdk)]
        compile_error!("Reference SDK role forbids SDK initializers; use its explicit entry");
        extern "C" fn ruycast_core_owned_initializer() {
            let guard = match $crate::guardian::Guard::bootstrap() {
                Ok(g) => g,
                Err(_) => unsafe { libc::_exit(86) },
            };
            let pins = $make_pins();
            $crate::sdk::core_preentry::constructor_with_guard(&pins, guard);
        }
        #[used]
        #[link_section = "__DATA,__mod_init_func"]
        static CORE_OWNED_INITIALIZER: extern "C" fn() = ruycast_core_owned_initializer;
    };
}

// Full Host admission is a separate compiled role. It neither widens CoreCLI
// arguments nor treats a CoreCLI lease/ready receipt as fourteen-scope authority.
const HOST_ROLE: &str = "full-owned-host-v1";
pub struct HostPins<'a> {
    /// Captured by the trusted holder from the private SDK readcap before build.
    pub owner_uid: u32,
    pub root: &'a Path,
    pub endpoint: &'a Path,
    pub endpoint_sha256: &'a str,
    pub sdk_artifact: &'a Path,
    pub nonce_sha256: &'a str,
    pub grant: Fingerprint,
    pub stdio: [Fingerprint; 3],
    pub source_manifest: &'a Path,
    pub initializer_source: &'a Path,
    /// Locator only. Native independently checks its own compiled cap/nonce.
    /// The SDK borrows it; only the Native receiver can take its ownership.
    pub native_grant_fd: i32,
    pub native_grant: Fingerprint,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostInput {
    pub path: String,
    pub sha256: String,
}
/// This final snapshot is written only by the trusted holder of the separately
/// compile-pinned SDK readcap, after BOTH builds. No compiled self-artifact or
/// generated-source hash is required; CoreCLI Grant is deliberately unchanged.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostGrant {
    pub owner_uid: u32,
    pub role: String,
    pub endpoint_sha256: String,
    pub sdk_artifact_sha256: String,
    pub source_set_sha256: String,
    pub nonce: String,
    pub stdio: [Fingerprint; 3],
    pub source_manifest: HostInput,
    pub initializer_source: HostInput,
    pub input_callbacks: NativeInputCompilerPins,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostSourceManifest { sources: Vec<HostInput> }
/// Source/image/role proof sealed by actual Host admission, not a public
/// Admitted literal. Rust resource sharing stays inside the SDK image.
pub struct HostAdmission {
    owner_uid: u32,
    admitted: Admitted,
    root: OwnedRoot,
    endpoint: std::path::PathBuf,
    sdk: std::path::PathBuf,
    sdk_sha256: String,
    native_grant_fd: i32,
    native_grant: Fingerprint,
    initializer: usize,
    endpoint_sha256: String,
    source_inputs: std::collections::BTreeMap<String,String>,
    input_callbacks: NativeInputCompilerPins,
    profile_provider:std::sync::OnceLock<PreparedProfileProvider>,
    profile_preparing:std::sync::atomic::AtomicBool,
}
struct PreparedProfileProvider {
    profile:std::path::PathBuf,
    config_override:Option<String>,
    home:std::path::PathBuf,
    owner:OwnedRoot,
    directory:OwnedFd,
    device:u64,
    inode:u64,
    api:usize,
}
impl PreparedProfileProvider {
    fn check(&self,uid:u32)->Result<()> {
        let current=self.owner.open_watch_directory(&self.profile)?;
        for fd in [self.directory.as_raw_fd(),current.as_raw_fd()] {
            let mut stat:libc::stat=unsafe{std::mem::zeroed()};
            if unsafe{libc::fstat(fd,&mut stat)}!=0 || stat.st_mode&libc::S_IFMT!=libc::S_IFDIR
                || stat.st_uid!=uid || stat.st_mode&0o077!=0
                || stat.st_dev as u64!=self.device || stat.st_ino!=self.inode {
                return Err("Host prepared profile directory identity changed".into());
            }
        }
        Ok(())
    }
}
fn read_host_grant(fd: i32, pin: Fingerprint, owner_uid: u32, life: &dyn AliveLease) -> Result<HostGrant> {
    read_private_host_grant(fd, pin, owner_uid, life)
}
fn read_private_host_grant<T: serde::de::DeserializeOwned>(fd: i32, pin: Fingerprint, owner_uid: u32, life: &dyn AliveLease) -> Result<T> {
    // Compare against the producer's COMPILED private-pipe identity BEFORE reading
    // any byte. A random readable pipe, stdio alias or manifest assertion is not
    // ownership. The producer keeps that backing alive across SDK build/admission.
    if pin.kind != u32::from(libc::S_IFIFO)
        || pin.access != libc::O_RDONLY
        || Fingerprint::own(fd)? != pin
    {
        return Err("grant is not the exact retained owned pipe".into());
    }
    let mut owner_stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut owner_stat) } != 0 || owner_stat.st_uid != owner_uid {
        return Err("SDK grantcap owner differs from compiled producer owner".into());
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err("grant pipe setup failed".into());
    }
    let end = Instant::now() + Duration::from_secs(3);
    let read = |out: &mut [u8]| -> Result<()> {
        let mut at = 0;
        while at < out.len() {
            life.before_effect()?;
            if Instant::now() >= end {
                return Err("grant pipe deadline".into());
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
                return Err("grant pipe failure".into());
            }
            if n == 0 {
                continue;
            }
            let n = unsafe { libc::read(fd, out.as_mut_ptr().add(at).cast(), out.len() - at) };
            if n > 0 {
                at += n as usize;
            } else if n == 0 {
                return Err("grant pipe incomplete".into());
            } else if !matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                return Err("grant pipe read failure".into());
            }
        }
        Ok(())
    };
    let mut n = [0; 4];
    read(&mut n)?;
    let n = u32::from_ne_bytes(n) as usize;
    if n > MAX_GRANT {
        return Err("grant packet size".into());
    }
    let mut bytes = vec![0; n];
    read(&mut bytes)?;
    let mut trailing=0u8;
    life.before_effect()?;
    if unsafe { libc::read(fd,(&mut trailing as *mut u8).cast(),1) }!=0 {
        return Err("Host grant must be a complete immutable snapshot with EOF before exec".into());
    }
    drop(descriptor);
    serde_json::from_slice(&bytes).map_err(|_| "grant packet schema".into())
}
fn host_args(endpoint: &Path) -> Result<()> {
    unsafe {
        let argc = _NSGetArgc();
        let argv = _NSGetArgv();
        if argc.is_null() || argv.is_null() || *argc != 2 || (*argv).is_null() {
            return Err("only exact owned Host entry is permitted".into());
        }
        if bounded((*argv).read(), 8192)? != endpoint.as_os_str().as_encoded_bytes()
            || bounded((*argv).add(1).read(), 128)? != b"__f1-owned-host"
            || !(*argv).add(2).read().is_null()
        {
            return Err("owned Host argument/source role differs".into());
        }
    }
    Ok(())
}
fn host_environment(sdk: &Path) -> Result<(i32, i32, i32, i32)> {
    unsafe {
        let raw = _NSGetEnviron();
        if raw.is_null() || (*raw).is_null() {
            return Err("Host environment missing".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let (mut life, mut admit, mut grant, mut native) = (None, None, None, None);
        for i in 0..32 {
            let p = (*raw).add(i).read();
            if p.is_null() {
                let (a, b, c) = (
                    life.ok_or("Host life missing")?,
                    admit.ok_or("Host admit missing")?,
                    grant.ok_or("Host grant missing")?,
                );
                let native = native.ok_or("Native grant locator missing")?;
                if seen.len() != 6
                    || native <= 2
                    || [a, b, c].contains(&native)
                    || a <= 2
                    || b <= 2
                    || c <= 2
                    || a == b
                    || a == c
                    || b == c
                {
                    return Err("Host inherited capability aliases/missing fields".into());
                }
                return Ok((a, b, c, native));
            }
            let bytes = bounded(p, 8192)?;
            let value = std::str::from_utf8(&bytes).map_err(|_| "Host environment UTF8")?;
            let (key, value) = value.split_once('=').ok_or("Host environment encoding")?;
            if !seen.insert(key.to_owned()) {
                return Err("duplicate Host entry field".into());
            }
            match key {
                "F1_LIFE_FD" => {
                    life = Some(value.parse::<i32>().map_err(|_| "Host life encoding")?)
                }
                "F1_ADMIT_FD" => {
                    admit = Some(value.parse::<i32>().map_err(|_| "Host admit encoding")?)
                }
                "F1_CORE_GRANT_FD" => {
                    grant = Some(value.parse::<i32>().map_err(|_| "Host grant encoding")?)
                }
                "F1_NATIVE_GRANT_FD" => {
                    native = Some(
                        value
                            .parse::<i32>()
                            .map_err(|_| "Native grant locator encoding")?,
                    )
                }
                "DYLD_INSERT_LIBRARIES"
                    if value.as_bytes() == sdk.as_os_str().as_encoded_bytes() =>
                {}
                "DYLD_BIND_AT_LAUNCH" if value == "1" => (),
                _ => return Err("unexpected Host environment; no CoreCLI/default fallback".into()),
            }
        }
    }
    Err("Host environment count".into())
}
impl HostAdmission {
    pub fn life(&self) -> Arc<dyn AliveLease> {
        self.admitted.life.clone()
    }
    pub fn descriptors(&self) -> Arc<DescriptorTable> {
        self.admitted.descriptors.clone()
    }
    pub(super) fn verify_native_input_issuer(&self) -> Result<()> {
        self.before_host_initialization()?;
        checked_hash(&self.root,&self.endpoint,&self.endpoint_sha256)?;
        for (path,hash) in &self.source_inputs { checked_hash(&self.root,Path::new(path),hash)?; }
        Ok(())
    }
    #[cfg(not(test))]
    pub fn initialize_bootstrap(
        &self,
        context: super::bootstrap::Bootstrap,
    ) -> Result<super::ModuleAttestation> {
        self.before_host_initialization()?;
        // The private sealed admission is the only source callsite supplying
        // these already-qualified common resources to bootstrap initialization.
        unsafe { super::bootstrap::initialize_owned_host(context, self.life(), self.descriptors()) }
    }
    fn before_host_initialization(&self) -> Result<()> {
        if !super::SOURCE_ROUTES_QUALIFIED {
            return Err("owned Host source routes are not qualified".into());
        }
        self.admitted.life.before_effect()?;
        if !self.admitted.life.is_alive() {
            return Err("owned Host guardian retired".into());
        }
        if self.root.owner_ids()?.0 != self.owner_uid {
            return Err("SDK owned root owner changed before Host publication".into());
        }
        checked_hash(&self.root, &self.sdk, &self.sdk_sha256)?;
        image(&self.endpoint)?;
        no_main_initializers()?;
        host_provider_header(&self.sdk, self.initializer)?;
        Ok(())
    }
    /// Full registry/source admission remains required before even exposing a
    /// provider to Native. A matching packet/header can never satisfy this.
    pub fn before_host_publication(&self) -> Result<()> {
        if !super::SOURCE_ROUTES_QUALIFIED {
            return Err("owned Host source routes are not qualified".into());
        }
        self.before_host_initialization()?;
        if Fingerprint::own(self.native_grant_fd)? != self.native_grant {
            return Err("Native grantcap changed before handoff".into());
        }
        if super::ruycast_f1_preentry_ready_v1() != 0xF100002600003FFF {
            return Err("fourteen Host scopes not actually admitted".into());
        }
        Ok(())
    }
    /// Publish the one existing profile broker before Ready, without exposing
    /// a Native frame or requiring any per-instance Native userdata. The same
    /// retained binding is consumed later by push_profile after actual Ready.
    pub fn initialize_profile_provider(&self,profile:&Path,config_override:Option<&str>)->Result<()> {
        self.verify_native_input_issuer()?;
        self.profile_preparing.compare_exchange(false,true,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire)
            .map_err(|_|"Host profile provider preparation is one-shot")?;
        struct Publication(bool);
        impl Drop for Publication {fn drop(&mut self){if !self.0 {
            super::profile_io::retire();super::READY_RETIRED.store(true,std::sync::atomic::Ordering::Release);
        }}}
        let mut publication=Publication(false);
        let context=super::bootstrap::owned_module_context(&self.life(),&self.descriptors())?;
        let owner=[&context.home,&context.cache,&context.temp,&context.applications].into_iter()
            .filter(|root|profile.starts_with(root.path())).max_by_key(|root|root.path().as_os_str().len())
            .ok_or("Host profile is outside admitted roots")?.clone();
        let directory=owner.open_watch_directory(profile)?;
        let mut stat:libc::stat=unsafe{std::mem::zeroed()};
        if unsafe{libc::fstat(directory.as_raw_fd(),&mut stat)}!=0 || stat.st_uid!=self.owner_uid || stat.st_mode&0o077!=0 {
            return Err("Host profile directory is not private and owned".into());
        }
        let (api,home)=super::bootstrap::publish_profile_provider(&self.life(),&self.descriptors(),profile,config_override)?;
        self.verify_native_input_issuer()?;
        let binding=PreparedProfileProvider{profile:profile.into(),config_override:config_override.map(str::to_owned),home,owner,directory,
            device:stat.st_dev as u64,inode:stat.st_ino,api:api as usize};
        binding.check(self.owner_uid)?;
        if api.is_null() || super::profile_io::qualified_profile_provider(&self.life(),&self.descriptors())? as usize!=binding.api {
            return Err("Host prepared profile table differs from actual broker".into());
        }
        self.profile_provider.set(binding).map_err(|_|"Host profile provider already prepared")?;
        publication.0=true;
        Ok(())
    }
    /// Consume the actual constructor admission and invoke only the verified
    /// exported sink of the pinned main executable. The receiver has its OWN
    /// compile-pinned grant; this packet cannot grant itself authority.
    pub fn push_profile(self, profile: &Path, config_override: Option<&str>,publisher:super::profile_io::NativeInputPublisherCapability) -> Result<()> {
        use ruycast_native::profile_io_abi as wire;
        struct Publication(bool);
        impl Drop for Publication {fn drop(&mut self){if !self.0 {
            super::profile_io::retire();super::READY_RETIRED.store(true,std::sync::atomic::Ordering::Release);
        }}}
        let mut publication=Publication(false);
        self.before_host_publication()?;
        let symbol =
            unsafe { libc::dlsym(libc::RTLD_MAIN_ONLY, c"ruycast_owned_host_bind_v1".as_ptr()) };
        if symbol.is_null() {
            return Err("pinned main Host sink absent".into());
        }
        let sink_code = super::metadata::code_identity(symbol as usize)?;
        if sink_code.image() != self.endpoint.as_os_str().as_encoded_bytes() {
            return Err("Host sink is not in the pinned main image".into());
        }
        let mut info = std::mem::MaybeUninit::<libc::Dl_info>::uninit();
        if unsafe { libc::dladdr(symbol, info.as_mut_ptr()) } == 0 {
            return Err("Host sink image unavailable".into());
        }
        let main_header = unsafe { info.assume_init() }.dli_fbase as usize;
        if main_header != unsafe { _NSGetMachExecuteHeader() } as usize {
            return Err("Host sink is not MH_EXECUTE".into());
        }
        let sdk_header = host_provider_header(&self.sdk, self.initializer)?;
        if sdk_header == main_header {
            return Err("Host provider is not a distinct SDK image".into());
        }
        let prepared=self.profile_provider.get().ok_or("Host profile provider was not prepared before Ready")?;
        if prepared.profile!=profile || prepared.config_override.as_deref()!=config_override {
            return Err("Host handoff profile differs from the prepared binding".into());
        }
        prepared.check(self.owner_uid)?;
        let api=super::profile_io::qualified_profile_provider(&self.life(),&self.descriptors())?;
        if api as usize!=prepared.api {return Err("Host prepared profile API changed".into());}
        let input_authority=super::profile_io::verify_native_input_publisher(&publisher,&self.life(),&self.descriptors())?;
        input_authority.verify_installed_contexts()?;
        let home=&prepared.home;
        let profile = profile.to_str().ok_or("Host profile UTF8")?.as_bytes();
        let home = home.to_str().ok_or("Host home UTF8")?.as_bytes();
        let override_bytes = config_override.unwrap_or("").as_bytes();
        if [profile, home, override_bytes]
            .iter()
            .any(|v| v.len() > 8192 || v.contains(&0))
        {
            super::profile_io::retire();
            return Err("Host path bytes invalid".into());
        }
        self.before_host_publication()
            .inspect_err(|_| super::profile_io::retire())?;
        prepared.check(self.owner_uid)?;
        let input_authority=super::profile_io::verify_native_input_publisher(&publisher,&self.life(),&self.descriptors())?;
        let installations=input_authority.verify_installed_contexts()?;
        self.before_host_publication()?;
        // Close the last dynamic Ready/module checks with pure identity checks
        // before writing a frame. No guardian callback follows this tail.
        prepared.check(self.owner_uid)?;
        for installation in &installations {installation.pure_after()?;}
        super::profile_io::input_publisher_current(&publisher,&input_authority,&self.life(),&self.descriptors())?;
        let packet = wire::HostBinding {
            version: wire::HOST_BINDING_VERSION,
            bytes: std::mem::size_of::<wire::HostBinding>() as u32,
            flags: if config_override.is_some() {
                wire::HOST_HAS_CONFIG_OVERRIDE
            } else {
                0
            },
            reserved: 0,
            profile: wire::Bytes::borrowed(profile),
            home: wire::Bytes::borrowed(home),
            config_override: wire::Bytes::borrowed(override_bytes),
            api,
            main_header,
            sdk_header,
            getter: super::profile_io::rcf1_profile_io_v1 as *const () as usize,
            ready: super::ruycast_f1_preentry_ready_v1 as *const () as usize,
            input_publisher:publisher.words(),
        };
        // Valid borrowed frame/path storage lasts until the qualified sink
        // returns. No Rust resource ownership crosses this C boundary.
        let sink: wire::HostSink = unsafe { std::mem::transmute(symbol) };
        if unsafe { sink(&packet) } != wire::OK {
            super::profile_io::retire();
            return Err("Native refused qualified Host publication".into());
        }
        publication.0=true;
        Ok(())
    }
}
#[link(name = "System")]
unsafe extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(index: u32) -> *const u8;
    fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
}
fn host_provider_header(sdk: &Path, initializer: usize) -> Result<usize> {
    if initializer == 0 {
        return Err("actual Host initializer missing".into());
    }
    let code = super::metadata::code_identity(initializer)?;
    if code.image() != sdk.as_os_str().as_encoded_bytes() {
        return Err("Host initializer image differs".into());
    }
    let mut info = std::mem::MaybeUninit::<libc::Dl_info>::uninit();
    if unsafe { libc::dladdr(initializer as *const libc::c_void, info.as_mut_ptr()) } == 0 {
        return Err("Host initializer image absent".into());
    }
    let header = unsafe { info.assume_init() }.dli_fbase.cast::<u8>();
    if header.is_null() {
        return Err("Host SDK header absent".into());
    }
    unsafe {
        let word = |p: *const u8| p.cast::<u32>().read_unaligned();
        let wide = |p: *const u8| p.cast::<u64>().read_unaligned();
        if word(header) != 0xfeedfacf || word(header.add(12)) != 6 {
            return Err("Host provider is not MachO64 dylib".into());
        }
        let images = _dyld_image_count();
        if images > 4096 {
            return Err("Host image count bounds".into());
        }
        let index = (0..images)
            .find(|i| _dyld_get_image_header(*i) == header)
            .ok_or("Host image not in dyld roster")?;
        let slide = _dyld_get_image_vmaddr_slide(index);
        let count = word(header.add(16)) as usize;
        let bytes = word(header.add(20)) as usize;
        if count > 4096 || bytes > 16 * 1024 * 1024 {
            return Err("Host SDK command bounds".into());
        }
        let (mut at, mut found) = (0usize, 0usize);
        for _ in 0..count {
            if at.checked_add(8).is_none_or(|n| n > bytes) {
                return Err("Host SDK commands truncated".into());
            }
            let p = header.add(32 + at);
            let kind = word(p);
            let size = word(p.add(4)) as usize;
            if size < 8 || at.checked_add(size).is_none_or(|n| n > bytes) {
                return Err("Host SDK command invalid".into());
            }
            if matches!(kind, 0x11 | 0x1a) {
                return Err("unknown Host SDK LC_ROUTINES initializer".into());
            }
            if kind == 0x19 {
                if size < 72 {
                    return Err("Host SDK segment truncated".into());
                }
                let vm = wide(p.add(24));
                let vmsize = wide(p.add(32));
                let sections = word(p.add(64)) as usize;
                if sections > 4096 || 72 + sections * 80 > size {
                    return Err("Host SDK sections invalid".into());
                }
                for i in 0..sections {
                    let sec = p.add(72 + i * 80);
                    let flags = word(sec.add(64)) & 0xff;
                    if wide(sec.add(40)) != 0
                        && nonlazy_objc_initializer(std::slice::from_raw_parts(sec, 16)) {
                        return Err("unknown Host SDK nonlazy Objective-C initializer".into());
                    }
                    let addr = wide(sec.add(32));
                    let len = wide(sec.add(40));
                    if matches!(flags, 9 | 0x15 | 0x16) && len != 0 {
                        if flags != 9
                            || len != 8
                            || addr < vm
                            || addr
                                .checked_add(len)
                                .is_none_or(|n| vm.checked_add(vmsize).is_none_or(|e| n > e))
                        {
                            return Err("unknown Host SDK initializer roster".into());
                        }
                        let address = usize::try_from(addr)
                            .map_err(|_| "Host initializer address overflow")?
                            .checked_add_signed(slide)
                            .ok_or("Host initializer slide overflow")?;
                        if (address as *const usize).read_unaligned() != initializer {
                            return Err("Host SDK has a competing initializer".into());
                        }
                        found += 1;
                    }
                }
            }
            at += size;
        }
        if found != 1 {
            return Err("Host SDK initializer must be exactly the selected constructor".into());
        }
    }
    Ok(header as usize)
}

pub fn enter_host(pins: &HostPins<'_>, initializer: usize) -> Result<HostAdmission> {
    if !super::SOURCE_ROUTES_QUALIFIED {
        return Err("owned Host source routes are not qualified".into());
    }
    let guard = guardian::Guard::bootstrap().map_err(|_| "Host guardian admission failed")?;
    enter_host_with_guard(pins, guard, initializer)
}
pub fn enter_host_with_guard(
    pins: &HostPins<'_>,
    guard: guardian::Guard,
    initializer: usize,
) -> Result<HostAdmission> {
    if !super::SOURCE_ROUTES_QUALIFIED {
        return Err("owned Host source routes are not qualified".into());
    }
    let life: Arc<dyn AliveLease> = Arc::new(ForkAwareLease::new(Arc::new(
        GuardianAlive::admitted(guard),
    )));
    host_args(pins.endpoint)?;
    let (life_fd,admit_fd,grant_fd,native_fd)=host_environment(pins.sdk_artifact)?;
    image(pins.endpoint)?;no_main_initializers()?;
    if native_fd!=pins.native_grant_fd || pins.native_grant_fd<=2
        || [life_fd,admit_fd,grant_fd].contains(&pins.native_grant_fd)
        || pins.native_grant.kind!=u32::from(libc::S_IFIFO) || pins.native_grant.access!=libc::O_RDONLY
        || Fingerprint::own(pins.native_grant_fd)?!=pins.native_grant
        || pins.native_grant==pins.grant || pins.stdio.contains(&pins.native_grant) {
        return Err("Native grantcap is not separate compiled owned input".into());
    }
    let grant=read_host_grant(grant_fd,pins.grant,pins.owner_uid,life.as_ref())?;
    if grant.owner_uid!=pins.owner_uid || grant.role!=HOST_ROLE || grant.endpoint_sha256!=pins.endpoint_sha256
        || digest(grant.nonce.as_bytes())!=pins.nonce_sha256 || grant.stdio!=pins.stdio {
        return Err("Host role/source/nonce/stdio differs".into());
    }
    stdio(&pins.stdio)?;
    let root=OwnedRoot::open(pins.root)?;
    if root.owner_ids()?.0 != pins.owner_uid {
        return Err("SDK owned root differs from private producer owner".into());
    }
    checked_hash(&root,pins.endpoint,pins.endpoint_sha256)?;
    checked_hash(&root,pins.sdk_artifact,&grant.sdk_artifact_sha256)?;
    if Path::new(&grant.source_manifest.path)!=pins.source_manifest
        || Path::new(&grant.initializer_source.path)!=pins.initializer_source {
        return Err("Host source/initializer paths differ from compiled pins".into());
    }
    checked_hash(&root,pins.source_manifest,&grant.source_manifest.sha256)?;
    checked_hash(&root,pins.initializer_source,&grant.initializer_source.sha256)?;
    let manifest=root.read_optional(pins.source_manifest,2*1024*1024)?.ok_or("Host source manifest absent")?;
    let manifest:HostSourceManifest=serde_json::from_slice(&manifest).map_err(|_|"Host source manifest schema")?;
    if manifest.sources.is_empty() || manifest.sources.len()>4096 {return Err("Host source manifest bounds".into());}
    let mut previous=None;let mut set=Sha256::new();let mut initializer_found=false;
    let mut source_inputs=std::collections::BTreeMap::new();
    for input in manifest.sources {
        if previous.as_ref().is_some_and(|p:&String|p>=&input.path) {return Err("Host sources must be strictly sorted/unique".into());}
        checked_hash(&root,Path::new(&input.path),&input.sha256)?;
        if input.path==grant.initializer_source.path && input.sha256==grant.initializer_source.sha256 {initializer_found=true;}
        set.update(input.path.as_bytes());set.update([0]);set.update(input.sha256.as_bytes());set.update(b"\n");
        source_inputs.insert(input.path.clone(),input.sha256.clone());
        previous=Some(input.path);
    }
    if !initializer_found || format!("{:x}",set.finalize())!=grant.source_set_sha256 {
        return Err("Host exact source/initializer set differs".into());
    }
    let sdk_code=super::metadata::code_identity(enter_host_with_guard as *const () as usize)?;
    if sdk_code.image()!=pins.sdk_artifact.as_os_str().as_encoded_bytes() || pins.sdk_artifact==pins.endpoint {
        return Err("actual Host SDK image differs".into());
    }
    host_provider_header(pins.sdk_artifact,initializer)?;
    let descriptors = DescriptorTable::new();
    Ok(HostAdmission {
        owner_uid: pins.owner_uid,
        admitted: Admitted {
            life,
            descriptors,
            imports: vec![],
            stdio: pins.stdio,
        },
        root,
        endpoint: pins.endpoint.into(),
        sdk: pins.sdk_artifact.into(),
        sdk_sha256: grant.sdk_artifact_sha256,
        native_grant_fd: pins.native_grant_fd,
        native_grant: pins.native_grant,
        initializer,
        endpoint_sha256:grant.endpoint_sha256,
        source_inputs,
        input_callbacks:grant.input_callbacks,
        profile_provider:std::sync::OnceLock::new(),
        profile_preparing:std::sync::atomic::AtomicBool::new(false),
    })
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum NativeInputCallbackId { CarbonPressed, HidInput, HidRemoved, HidFilter, KeywordFilter }
const NATIVE_INPUT_IDS:[NativeInputCallbackId;5]=[
    NativeInputCallbackId::CarbonPressed,NativeInputCallbackId::HidInput,NativeInputCallbackId::HidRemoved,
    NativeInputCallbackId::HidFilter,NativeInputCallbackId::KeywordFilter,
];
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeInputCallbackPin {
    pub id:NativeInputCallbackId,
    pub offset:usize,
    pub bytes:usize,
    pub body_sha256:String,
    pub source:HostInput,
}
/// Data only until read through the actual Host private cap. The reviewed
/// producer must first prove all five fixed C DIEs, linked function-size/body,
/// capsule fixups, UUID and complete same-period compiler/source closure.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeInputCompilerPins {
    pub main_uuid:[u8;16],
    pub capsule_offset:usize,
    pub callbacks:[NativeInputCallbackPin;5],
    pub evidence:HostInput,
}
// These mirror the actual private producer's compiler DATA. Their successful
// decoding/shape checks are NOT a source/closure/lifetime authority constructor.
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherLinked {
    dwarf_name:String,linkage_name:String,dwarf_offset:u64,object_address:u64,
    image_address:u64,image_offset:u64,object_path:String,function_size:u64,
    parameter_types:Vec<String>,result_type:Option<String>,declaration_file:Option<String>,declaration_line:Option<String>,
}
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherOwnerTypes {
    parameter_die:u64,owner_die:u64,callback_die:u64,tuple_die:u64,result_die:u64,option_die:u64,ticket_die:u64,error_die:u64,
}
#[derive(Deserialize,PartialEq,Eq)]
enum PublisherFactoryRole {Carbon,Hid,Keyword}
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherHostIoSlot {
    name:String,offset:u64,member_die:u64,pointer_die:u64,signature_die:u64,
    parameter_dies:Vec<u64>,result_die:u64,
}
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherHostIoTypes {
    ticket_die:u64,publisher_arc_die:u64,publisher_die:u64,api_reference_die:u64,api_die:u64,
    consumed_phase_die:u64,consumed_payload_die:u64,bound_box_die:u64,bound_host_die:u64,
    slots:[PublisherHostIoSlot;4],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherFactoryData {role:PublisherFactoryRole,types:PublisherOwnerTypes,host_io_types:PublisherHostIoTypes,linked:PublisherLinked}
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherRelocation {
    byte_offset:u64,relocation_type:u32,immediate_mask:u32,symbol_index:u32,external:bool,pc_relative:bool,
    target_symbol:Option<String>,linked_target:Option<u64>,target_matches:Option<bool>,
}
#[derive(Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct PublisherText {
    bytes:u64,unchanged_bits_equal:bool,relocated_instructions:Vec<PublisherRelocation>,relocation_targets_verified:bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherBodyData {
    linked:PublisherLinked,text:PublisherText,outgoing_targets:Vec<u64>,body_sha256:String,
    source:HostInput,object:HostInput,dwarf:HostInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherClosureCapture {member_die:u64,name:String,offset:u64,type_die:u64}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherClosureNullSlot {name:String,offset:u64,null_object_slot:bool}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherClosureCodeSlot {
    name:String,offset:u64,symbol:String,self_die:Option<u64>,linked:PublisherLinked,text:PublisherText,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum PublisherClosureSlot {Null(PublisherClosureNullSlot),Code(Box<PublisherClosureCodeSlot>)}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherClosureVtable {
    object_path:String,variable_die:u64,table_die:u64,environment_die:u64,object_address:u64,
    captures:Vec<PublisherClosureCapture>,slots:[PublisherClosureSlot;4],authority:bool,missing_for_admission:Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherRouteData {
    schema:String,uuid:[u8;16],factories:[PublisherFactoryData;3],route_roots:Vec<PublisherLinked>,bodies:Vec<PublisherBodyData>,
    closure_object_vtables:Vec<PublisherClosureVtable>,
    authority:bool,missing_for_admission:Vec<String>,artifact:HostInput,compiler_closure:HostInput,debug_map:HostInput,symbols:HostInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeInputCompilerEvidence {
    schema:String,artifact:HostInput,compiler_closure:HostInput,
    #[serde(rename="body_proof")] _body_proof:serde_json::Value,
    publisher_route:PublisherRouteData,capsule_offset:usize,fixups:HostInput,
    callbacks:[NativeInputCallbackPin;5],runtime_admission:bool,
}
#[derive(Deserialize)]
struct PublisherObjectData {original:HostInput,retained:HostInput,dwarf:HostInput}
#[derive(Deserialize)]
struct PublisherObjcCompileData {
    command:Vec<String>,source:HostInput,dep_info:HostInput,
    actual_inputs:std::collections::BTreeMap<String,HostInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherObjcConsumerData { selected:HostInput,linked_output:HostInput }
#[derive(Deserialize)]
struct PublisherCompilerData {
    native:HostInput,native_uuid:[u8;16],compiler_roles:std::collections::BTreeMap<String,u64>,
    actual_source_inputs:std::collections::BTreeMap<String,HostInput>,objects:std::collections::BTreeMap<String,PublisherObjectData>,
    dep_info:Vec<HostInput>,foundation_actual_compile:HostInput,debug_map:HostInput,symbols:HostInput,
    compiler_arguments:HostInput,cargo_command:HostInput,toolchain:HostInput,
    externs:std::collections::BTreeMap<String,HostInput>,objc2_same_build_output:HostInput,
    objc2_native_consumers:std::collections::BTreeMap<String,PublisherObjcConsumerData>,objc2_actual_compile:PublisherObjcCompileData,
}
fn publisher_objc_consumer_shape(consumer:&PublisherObjcConsumerData,output:&HostInput,
    compiler:&PublisherObjcCompileData,inputs:&std::collections::BTreeMap<String,HostInput>)->Result<()> {
    if consumer.linked_output!=*output || inputs.get(&consumer.selected.path)!=Some(&consumer.selected) {
        return Err("Native Input actual objc2 consumer binding differs".into());
    }
    if consumer.selected==*output{return Ok(());}
    let output_path=Path::new(&output.path);
    if output_path.extension()!=Some(std::ffi::OsStr::new("rlib"))
        || Path::new(&consumer.selected.path)!=output_path.with_extension("rmeta") {
        return Err("Native Input objc2 metadata is not the matching linked output".into());
    }
    let argument=|name:&str|->Result<&str>{
        let values=compiler.command.windows(2).filter(|pair|pair[0]==name).map(|pair|pair[1].as_str()).collect::<Vec<_>>();
        if values.len()!=1{return Err("Native Input objc2 producer argument missing/duplicate".into());}
        Ok(values[0])
    };
    let suffix=compiler.command.windows(2).filter(|pair|pair[0]=="-C")
        .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
    let emits=compiler.command.iter().filter_map(|arg|arg.strip_prefix("--emit=")).collect::<Vec<_>>();
    if argument("--crate-name")?!="objc2" || !matches!(argument("--crate-type")?,"lib"|"rlib")
        || suffix.len()!=1 || suffix[0].is_empty() || suffix[0].contains(['/', '\\'])
        || emits.len()!=1 || ["metadata","link","dep-info"].iter().any(|required|!emits[0].split(',').any(|part|part==*required))
        || Path::new(argument("--out-dir")?).join(format!("libobjc2{}.rlib",suffix[0]))!=output_path {
        return Err("Native Input objc2 metadata lacks its exact linking producer".into());
    }
    Ok(())
}
fn publisher_evidence_bytes(root:&OwnedRoot,binding:&HostInput)->Result<Vec<u8>> {
    reference_hash(&binding.sha256)?;
    let bytes=root.read_optional(Path::new(&binding.path),32*1024*1024)?.ok_or("Native Input compiler evidence absent")?;
    if digest(&bytes)!=binding.sha256 {return Err("Native Input compiler evidence changed".into());}
    Ok(bytes)
}
fn publisher_linked_shape(linked:&PublisherLinked)->Result<()> {
    if linked.dwarf_name.is_empty() || linked.dwarf_name.len()>1024 || linked.linkage_name.is_empty() || linked.linkage_name.len()>4096
        || linked.dwarf_offset==0 || linked.image_offset==0 || linked.function_size==0 || linked.function_size>65536
        || linked.image_address.checked_sub(linked.image_offset).is_none()
        || linked.image_address.checked_add(linked.function_size).is_none()
        || linked.object_address.checked_add(linked.function_size).is_none()
        || linked.parameter_types.len()>32 || linked.parameter_types.iter().any(|p|p.is_empty()||p.len()>4096)
        || linked.result_type.as_ref().is_some_and(|p|p.is_empty()||p.len()>4096)
        || linked.declaration_file.as_ref().is_none_or(|p|p.is_empty()||p.len()>4096||p.contains('\0'))
        || linked.declaration_line.as_ref().is_none_or(|p|p.is_empty()||p.len()>64) {
        return Err("Native Input publisher linked compiler record is incomplete".into());
    }
    Ok(())
}
fn publisher_branch_target(instruction:u32,address:u64,byte_offset:u64)->Result<u64> {
    if !byte_offset.is_multiple_of(4)
        || !matches!(instruction&0xfc00_0000,0x1400_0000|0x9400_0000) {
        return Err("Native Input publisher relocated branch opcode/alignment differs".into());
    }
    let pc=address.checked_add(byte_offset).ok_or("Native Input publisher branch PC overflow")?;
    let delta=i64::from(((instruction&0x03ff_ffff) as i32)<<6>>6)*4;
    u64::try_from(i128::from(pc)+i128::from(delta))
        .map_err(|_|"Native Input publisher branch target overflow".into())
}
fn publisher_host_io_shape(types:&PublisherHostIoTypes,ticket_die:u64)->Result<()> {
    if types.ticket_die!=ticket_die || [types.ticket_die,types.publisher_arc_die,types.publisher_die,
        types.api_reference_die,types.api_die,types.consumed_phase_die,types.consumed_payload_die,
        types.bound_box_die,types.bound_host_die].contains(&0) {
        return Err("Native Input publisher Host IO type path differs".into());
    }
    let mut members=std::collections::BTreeSet::new();
    for (slot,(name,offset,parameters)) in types.slots.iter().zip([
        ("bind",24,1),("request",32,2),("release_handle",40,1),("return_buffer",48,2)]) {
        if slot.name!=name || slot.offset!=offset || slot.member_die==0 || !members.insert(slot.member_die)
            || slot.pointer_die==0 || slot.signature_die==0 || slot.result_die==0
            || slot.parameter_dies.len()!=parameters || slot.parameter_dies.contains(&0) {
            return Err("Native Input publisher fixed Host IO signature data differs".into());
        }
    }
    // These are exact object-local DATA references. The offline parser resolves
    // the real subtrees; no type offset is a C-ABI/runtime call permission.
    Ok(())
}
fn publisher_closure_data(
    tables:&[PublisherClosureVtable],closure:&PublisherCompilerData,
    bodies:&std::collections::BTreeMap<u64,&PublisherBodyData>,
)->Result<()> {
    if tables.len()>4096 {return Err("Native Input closure object table bound differs".into());}
    let mut seen=std::collections::BTreeSet::new();
    for table in tables {
        if table.authority || table.variable_die==0 || table.table_die==0 || table.environment_die==0
            || !table.object_address.is_multiple_of(8) || table.object_address.checked_add(48).is_none()
            || table.captures.len()>4096 || !seen.insert((&table.object_path,table.variable_die))
            || table.missing_for_admission.len()>16
            || table.missing_for_admission.iter().any(|v|v.is_empty()||v.len()>1024)
            || !closure.objects.contains_key(&table.object_path) {
            return Err("Native Input closure object/type record differs".into());
        }
        let mut members=std::collections::BTreeSet::new();
        let mut names=std::collections::BTreeSet::new();
        for capture in &table.captures {
            if capture.member_die==0 || capture.type_die==0 || capture.name.is_empty() || capture.name.len()>4096
                || capture.name.contains('\0') || capture.offset>1024*1024
                || !members.insert(capture.member_die) || !names.insert(&capture.name) {
                return Err("Native Input closure capture type data differs".into());
            }
        }
        let mut methods=Vec::new();
        for (slot,(name,offset)) in table.slots.iter().zip([
            ("drop_in_place",0),("__method3",24),("__method4",32),("__method5",40)]) {
            match slot {
                PublisherClosureSlot::Null(slot)=>{
                    if offset!=0 || slot.offset!=offset || slot.name!=name || !slot.null_object_slot {
                        return Err("Native Input closure null slot is not the exact object drop slot".into());
                    }
                }
                PublisherClosureSlot::Code(slot)=>{
                    publisher_linked_shape(&slot.linked)?;
                    if slot.offset!=offset || slot.name!=name || slot.symbol!=format!("_{}",slot.linked.linkage_name)
                        || slot.linked.object_path!=table.object_path
                        || slot.self_die!=if offset==0||offset==24 {Some(table.environment_die)} else {None} {
                        return Err("Native Input closure code slot object/type identity differs".into());
                    }
                    let body=bodies.get(&slot.linked.image_address)
                        .ok_or("Native Input closure slot lacks its source/body/compiler closure binding")?;
                    if body.linked!=slot.linked || body.text!=slot.text {
                        return Err("Native Input closure slot is not its exact retained linked body".into());
                    }
                    if offset!=0 {methods.push((slot.linked.image_address,body));}
                }
            }
        }
        if methods.len()!=3 || methods[1].0!=methods[2].0 || !methods[0].1.outgoing_targets.contains(&methods[2].0) {
            return Err("Native Input closure shim does not join its exact shared body".into());
        }
    }
    // Object const storage and typed relocation origins are DATA, not the MAIN
    // RO vtable, instance/upcast/callsite or capture-lifetime qualification.
    Ok(())
}
/// Consume the actual9190 evidence format, not a new grant/schema or a boolean
/// Source witness. The private caller must STILL qualify the explicitly missing
/// dynamic capture, same-owner/source semantics and retirement admission.
fn native_input_publisher_data(
    root:&OwnedRoot,endpoint:&Path,endpoint_sha:&str,sources:&std::collections::BTreeMap<String,String>,pins:&NativeInputCompilerPins,
)->Result<()> {
    let evidence:NativeInputCompilerEvidence=serde_json::from_slice(&publisher_evidence_bytes(root,&pins.evidence)?)
        .map_err(|_|"Native Input required typed publisher evidence schema differs")?;
    let route=&evidence.publisher_route;
    let artifact=HostInput{path:endpoint.to_str().ok_or("Native Input artifact path encoding")?.into(),sha256:endpoint_sha.into()};
    if evidence.schema!="ruycast.f1.native-input-compiler.v1" || evidence.runtime_admission || route.authority
        || route.schema!="ruycast.f1.native-input-publisher-code.v1" || route.uuid!=pins.main_uuid
        || evidence.artifact!=artifact || route.artifact!=artifact || evidence.callbacks!=pins.callbacks
        || evidence.capsule_offset!=pins.capsule_offset || route.compiler_closure!=evidence.compiler_closure {
        return Err("Native Input publisher evidence is not the same private compiler/callback image".into());
    }
    let closure:PublisherCompilerData=serde_json::from_slice(&publisher_evidence_bytes(root,&evidence.compiler_closure)?)
        .map_err(|_|"Native Input compiler closure schema differs")?;
    let roles=std::collections::BTreeMap::from([("native-bin".into(),1),("native-lib".into(),1),("objc2-lib".into(),1)]);
    if closure.native!=artifact || closure.native_uuid!=pins.main_uuid || closure.compiler_roles!=roles
        || closure.debug_map!=route.debug_map || closure.symbols!=route.symbols || closure.dep_info.len()!=3
        || closure.actual_source_inputs.is_empty() || closure.actual_source_inputs.len()>4096
        || closure.objects.is_empty() || closure.objects.len()>32 || route.bodies.is_empty() || route.bodies.len()>4096 {
        return Err("Native Input publisher lacks the actual complete compiler-period closure".into());
    }
    let objc=&closure.objc2_actual_compile;
    if closure.externs.is_empty() || closure.externs.len()>256
        || closure.externs.get("objc2")!=Some(&closure.objc2_same_build_output)
        || closure.objc2_native_consumers.len()!=2
        || ["native-bin","native-lib"].iter().any(|role|
            closure.objc2_native_consumers.get(*role).is_none_or(|consumer|
                publisher_objc_consumer_shape(consumer,&closure.objc2_same_build_output,objc,&closure.actual_source_inputs).is_err()))
        || objc.command.is_empty() || objc.command.len()>4096 || !Path::new(&objc.command[0]).is_absolute()
        || objc.command.iter().any(|a|a.len()>65536||a.contains('\0'))
        || objc.actual_inputs.is_empty() || objc.actual_inputs.len()>4096
        || objc.actual_inputs.get(&objc.source.path)!=Some(&objc.source)
        || !closure.dep_info.contains(&objc.dep_info) {
        return Err("Native Input publisher objc2 same-build consumer/input join differs".into());
    }
    for binding in [&evidence.fixups,&route.debug_map,&route.symbols,&closure.foundation_actual_compile,
        &closure.compiler_arguments,&closure.cargo_command,&closure.toolchain] {
        checked_hash(root,Path::new(&binding.path),&binding.sha256)?;
    }
    for binding in &closure.dep_info {checked_hash(root,Path::new(&binding.path),&binding.sha256)?;}
    // Extern objects are owned build outputs; objc2 source paths may belong to
    // the separately pinned toolchain source root. Do not expand OwnedRoot to
    // read them here or infer source custody from their path/receipt alone.
    for binding in closure.externs.values() {checked_hash(root,Path::new(&binding.path),&binding.sha256)?;}
    for consumer in closure.objc2_native_consumers.values() {
        checked_hash(root,Path::new(&consumer.selected.path),&consumer.selected.sha256)?;
    }
    let expected_roots=[
        ("owned_host_entry.rs","publish"),("owned_host_entry.rs","publish_from"),("owned_host_entry.rs","revoke"),("owned_host_entry.rs","request"),
        ("owned_host_entry.rs","receive"),("owned_host_entry.rs","bind_frame"),("owned_host_entry.rs","take_bound_host"),
        ("carbon_hotkeys.rs","new"),("carbon_hotkeys.rs","finish_callback_removal"),("carbon_hotkeys.rs","drop"),
        ("hid_service.rs","publish_source_ticket"),("hid_service.rs","revoke_source_ticket"),("hid_service.rs","new"),
        ("hid_service.rs","refresh"),("hid_service.rs","retire"),("hid_service.rs","stop"),("hid_service.rs","drop"),
        ("keyword_capture.rs","publish_source_ticket"),("keyword_capture.rs","revoke_source_ticket"),("keyword_capture.rs","with_ownership"),
        ("keyword_capture.rs","refresh"),("keyword_capture.rs","retire"),("keyword_capture.rs","stop"),("keyword_capture.rs","drop"),
        ("native_keyboard.rs","new"),
    ];
    if route.route_roots.len()!=expected_roots.len() {return Err("Native Input publisher registration/retirement roots differ".into());}
    for (linked,(file,name)) in route.route_roots.iter().zip(expected_roots) {
        publisher_linked_shape(linked)?;
        let expected=root.path().join("rust-native/src").join(file);
        if linked.dwarf_name!=name || linked.declaration_file.as_deref()!=expected.to_str() {
            return Err("Native Input publisher fixed source root differs".into());
        }
    }
    for (factory,(role,name)) in route.factories.iter().zip([
        (PublisherFactoryRole::Carbon,"for_carbon"),(PublisherFactoryRole::Hid,"for_hid"),(PublisherFactoryRole::Keyword,"for_keyword")]) {
        publisher_linked_shape(&factory.linked)?;
        let t=&factory.types;
        // Nonzero object-local references are shape, NOT ownership/type proof.
        // The actual producer resolves the referenced subtrees; strings/offsets
        // alone never open native_input_admission below.
        if factory.role!=role || factory.linked.dwarf_name!=name
            || factory.linked.declaration_file.as_deref()!=root.path().join("rust-native/src/owned_host_entry.rs").to_str()
            || [t.parameter_die,t.owner_die,t.callback_die,t.tuple_die,t.result_die,t.option_die,t.ticket_die,t.error_die].contains(&0) {
            return Err("Native Input publisher fixed typed factory data differs".into());
        }
        publisher_host_io_shape(&factory.host_io_types,t.ticket_die)?;
    }
    for (index,factory) in route.factories.iter().enumerate() {
        for other in &route.factories[..index] {
            if factory.linked.object_path==other.linked.object_path && factory.types.ticket_die==other.types.ticket_die
                && factory.host_io_types!=other.host_io_types {
                return Err("Native Input same-object publisher Host IO type records conflict".into());
            }
        }
    }
    let mut bodies=std::collections::BTreeMap::new();let mut preferred_base=None;
    for body in &route.bodies {
        let linked=&body.linked;publisher_linked_shape(linked)?;
        let base=linked.image_address-linked.image_offset;
        if preferred_base.is_some_and(|old|old!=base) || bodies.insert(linked.image_address,body).is_some() {
            return Err("Native Input publisher body image origin/identity differs".into());
        }
        preferred_base=Some(base);
        if linked.declaration_file.as_deref()!=Some(body.source.path.as_str())
            || sources.get(&body.source.path)!=Some(&body.source.sha256)
            || closure.actual_source_inputs.get(&body.source.path)!=Some(&body.source) {
            return Err("Native Input publisher body source is outside actual same-build DI".into());
        }
        let object=closure.objects.get(&linked.object_path).ok_or("Native Input publisher original object is absent")?;
        if object.original.path!=linked.object_path || object.retained!=body.object || object.dwarf!=body.dwarf {
            return Err("Native Input publisher object/DWARF origin differs".into());
        }
        for binding in [&body.source,&body.object,&body.dwarf] {checked_hash(root,Path::new(&binding.path),&binding.sha256)?;}
        reference_hash(&body.body_sha256)?;
        if body.text.bytes!=linked.function_size || !body.text.unchanged_bits_equal || !body.text.relocation_targets_verified
            || body.text.relocated_instructions.len()>16384 || body.outgoing_targets.len()>16384 {
            return Err("Native Input publisher text/target closure is incomplete".into());
        }
        for relocation in &body.text.relocated_instructions {
            if relocation.byte_offset.checked_add(4).is_none_or(|end|end>linked.function_size)
                || relocation.relocation_type!=2 || relocation.immediate_mask!=0x03ff_ffff
                || !relocation.external || !relocation.pc_relative || relocation.symbol_index>0x00ff_ffff
                || relocation.target_matches!=Some(true) || relocation.linked_target.is_none()
                || relocation.target_symbol.as_ref().is_none_or(|s|s.is_empty()||s.len()>4096) {
                return Err("Native Input publisher relocation is not bound to an actual target".into());
            }
        }
    }
    for linked in route.route_roots.iter().chain(route.factories.iter().map(|f|&f.linked)) {
        if bodies.get(&linked.image_address).is_none_or(|body|body.linked!=*linked) {
            return Err("Native Input publisher root is not its exact transitive body".into());
        }
    }
    for body in &route.bodies {
        if body.outgoing_targets.iter().any(|target|!bodies.contains_key(target)) {
            return Err("Native Input publisher outgoing target is not in its actual body closure".into());
        }
    }
    publisher_closure_data(&route.closure_object_vtables,&closure,&bodies)?;
    let header=unsafe{_NSGetMachExecuteHeader()} as usize;
    for body in &route.bodies {
        let address=header.checked_add(usize::try_from(body.linked.image_offset).map_err(|_|"Native Input publisher address bound")?)
            .ok_or("Native Input publisher address overflow")?;
        let bytes=usize::try_from(body.linked.function_size).map_err(|_|"Native Input publisher body bound")?;
        reference_code(address,endpoint)?;reference_mapping(address,bytes,Some(true))?;
        let actual=unsafe{std::slice::from_raw_parts(address as *const u8,bytes)};
        if digest(actual)!=body.body_sha256 {
            return Err("Native Input actual publisher linked body differs".into());
        }
        for relocation in &body.text.relocated_instructions {
            let offset=usize::try_from(relocation.byte_offset).map_err(|_|"Native Input relocation offset bound")?;
            let instruction=u32::from_le_bytes(actual.get(offset..offset+4).ok_or("Native Input relocation leaves body")?
                .try_into().map_err(|_|"Native Input relocation instruction bound")?);
            let destination=publisher_branch_target(instruction,body.linked.image_address,relocation.byte_offset)?;
            if Some(destination)!=relocation.linked_target
                || relocation.linked_target.is_none_or(|target|!bodies.contains_key(&target)) {
                return Err("Native Input actual publisher relocated target differs".into());
            }
        }
    }
    if route.missing_for_admission.len()>16 || route.missing_for_admission.iter().any(|v|v.is_empty()||v.len()>1024) {
        return Err("Native Input publisher missing-conjunct data differs".into());
    }
    // Compiler DATA checks complete. This is deliberately NOT an authority
    // result; dynamic captures/caller-before-register/retire-before-free still
    // require their real source-bound verification before the constructor opens.
    Ok(())
}
type InputCarbon=super::input_carbon::Handler;
type InputHid=super::input_hid::Callback;
type InputEvent=super::input_events::Callback;
#[derive(Clone,Copy)]
struct NativeInputFunctions {
    carbon:InputCarbon,input:InputHid,removed:InputHid,filter:InputEvent,keyword:InputEvent,
}
/// Fixed-family witness. No parser/usize-set/public constructor can create it.
/// Source-code identity is separate from the opaque registered userdata's
/// lifetime: each leaf must retain its own registration epoch/inflight lease.
pub struct NativeInputAdmission {
    token:super::NativeInputRegistryToken,
    root:OwnedRoot,
    endpoint:std::path::PathBuf,
    endpoint_sha256:String,
    source_inputs:std::collections::BTreeMap<String,String>,
    pins:NativeInputCompilerPins,
    functions:NativeInputFunctions,
    installation_next:std::sync::atomic::AtomicU64,
    owner_next:std::sync::atomic::AtomicU64,
    installations:[std::sync::atomic::AtomicU64;3],
}
/// Actual contexts remain in their existing module CURRENT slots. These fixed
/// identities contain no context pointer, Rc, TLS registry or userdata owner.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum NativeInputContextKind {Carbon,Hid,Events}
impl NativeInputContextKind {
    fn index(self)->usize {match self{Self::Carbon=>0,Self::Hid=>1,Self::Events=>2}}
}
#[derive(Clone)]
pub struct NativeInputInstallation {
    admission:Arc<NativeInputAdmission>,
    kind:NativeInputContextKind,
    id:u64,
    candidate:Option<Arc<std::sync::atomic::AtomicU8>>,
}
impl NativeInputInstallation {
    pub fn verify_candidate(&self,context:&super::owned::ModuleContext)->Result<()> {
        let candidate_live=||->Result<()> {
            self.admission.not_retired()?;
            if self.candidate.as_ref().is_some_and(|p|p.load(std::sync::atomic::Ordering::Acquire)==2) {
                return Err("Native Input installation candidate retired".into());
            }
            Ok(())
        };
        candidate_live()?;
        self.admission.verify(context)?;
        candidate_live()
    }
    /// Called only after the genuine context has been stored in its CURRENT.
    /// No dynamic guardian or context lookup occurs while that slot is borrowed.
    pub fn commit_installed(&self)->Result<()> {
        self.admission.not_retired()?;
        let phase=self.candidate.as_ref().ok_or("Native Input installation is not a candidate")?;
        phase.compare_exchange(0,1,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire)
            .map_err(|_|"Native Input installation already committed or retired")?;
        if self.admission.installations[self.kind.index()].compare_exchange(0,self.id,
            std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire).is_err() {
            phase.store(2,std::sync::atomic::Ordering::Release);
            return Err("Native Input context already installed".into());
        }
        if let Err(error)=self.pure_after(){self.retire();return Err(error);}
        Ok(())
    }
    pub fn verify(&self,context:&super::owned::ModuleContext)->Result<()> {
        self.pure_after()?;
        self.verify_candidate(context)?;
        self.pure_after()
    }
    pub fn pure_after(&self)->Result<()> {
        self.admission.not_retired()?;
        if self.id==0 || self.admission.installations[self.kind.index()].load(std::sync::atomic::Ordering::Acquire)!=self.id
            || self.candidate.as_ref().is_some_and(|p|p.load(std::sync::atomic::Ordering::Acquire)!=1) {
            return Err("Native Input context installation retired or replaced".into());
        }
        Ok(())
    }
    pub fn matches(&self,expected:&Self)->bool {
        Arc::ptr_eq(&self.admission,&expected.admission) && self.kind==expected.kind && self.id==expected.id
            && self.pure_after().is_ok() && expected.pure_after().is_ok()
    }
    /// Cleanup is monotone and does not call a guardian or touch CURRENT.
    pub fn retire(&self) {
        if let Some(phase)=&self.candidate {phase.store(2,std::sync::atomic::Ordering::Release);}
        let _=self.admission.installations[self.kind.index()].compare_exchange(self.id,0,
            std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire);
    }
}
#[derive(Clone)]
pub struct NativeInputCallbackWitness { admission:Arc<NativeInputAdmission>, id:NativeInputCallbackId }
pub struct NativeInputCallbackGuard { admission:Arc<NativeInputAdmission> }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum NativeInputOwnerFamily {Carbon,Hid,Keyword}
/// No public scalar/raw-pointer constructor. Only the qualified private owner
/// publisher can issue this ticket directly into the actual leaf registration.
/// It never reads or retains the Native userdata. Native must keep its actual
/// Box/Rc until successful revoke, and quarantine it while revoke reports busy.
pub struct NativeInputOwnerTicket {
    family:NativeInputOwnerFamily,
    userdata:usize,
    epoch:u64,
    slot:u64,
    generation:u64,
    issuer:Arc<NativeInputAdmission>,
    revoked:std::sync::atomic::AtomicBool,
    in_flight:std::sync::atomic::AtomicUsize,
    installations:[u64;3],
    bound_contexts:std::sync::atomic::AtomicU8,
}
enum NativeInputOwnerEntryPhase {Registration,Callback}
pub struct NativeInputOwnerEntry {ticket:Arc<NativeInputOwnerTicket>,phase:NativeInputOwnerEntryPhase}
impl NativeInputOwnerTicket {
    pub fn family(&self)->NativeInputOwnerFamily{self.family}
    pub fn epoch(&self)->u64{self.epoch}
    pub fn slot(&self)->u64{self.slot}
    pub fn generation(&self)->u64{self.generation}
    /// Data comparison against the actual stored ticket, not an issuer. It
    /// intentionally remains usable after revocation for known-owner cleanup;
    /// callers separately verify the publisher and exact installation identity.
    pub fn matches_owner_notice(&self,notice:&super::profile_io::NativeInputOwnerNoticeV1)->bool {
        let family=match self.family {NativeInputOwnerFamily::Carbon=>1,NativeInputOwnerFamily::Hid=>2,NativeInputOwnerFamily::Keyword=>3};
        notice.version==1 && notice.bytes==48 && notice.role==family && notice.ticket_kind==family
            && notice.userdata==self.userdata && notice.owner_epoch==self.epoch
            && notice.ticket_slot==self.slot && notice.ticket_generation==self.generation
    }
    pub fn retired(&self)->bool {
        self.revoked.load(std::sync::atomic::Ordering::Acquire) || self.issuer.not_retired().is_err()
            || self.installations.iter().enumerate().any(|(index,id)| {
                let required=match self.family {NativeInputOwnerFamily::Carbon=>index==0,NativeInputOwnerFamily::Hid=>index==1||index==2,NativeInputOwnerFamily::Keyword=>index==2};
                required && (*id==0 || self.issuer.installations[index].load(std::sync::atomic::Ordering::Acquire)!=*id)
            })
    }
    fn required_contexts(&self)->u8 {
        match self.family {NativeInputOwnerFamily::Carbon=>1,NativeInputOwnerFamily::Hid=>2|4,NativeInputOwnerFamily::Keyword=>4}
    }
    fn fully_bound(&self)->Result<()> {
        if self.bound_contexts.load(std::sync::atomic::Ordering::Acquire)!=self.required_contexts() {
            return Err("Native Input owner registration transaction is incomplete".into());
        }
        Ok(())
    }
    pub fn enter(self:&Arc<Self>)->Result<NativeInputOwnerEntry> {
        self.fully_bound()?;
        let mut entry=self.enter_registration()?;
        self.fully_bound()?;
        entry.phase=NativeInputOwnerEntryPhase::Callback;
        Ok(entry)
    }
    /// Pending publication holds the real owner inflight lease, but does not
    /// permit a Native callback until all fixed family contexts are committed.
    pub(crate) fn enter_registration(self:&Arc<Self>)->Result<NativeInputOwnerEntry> {
        if self.retired(){return Err("Native Input owner ticket retired".into());}
        self.in_flight.fetch_update(std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire,
            |n|n.checked_add(1)).map_err(|_|"Native Input owner inflight overflow")?;
        if self.retired(){self.in_flight.fetch_sub(1,std::sync::atomic::Ordering::AcqRel);return Err("Native Input owner retired during entry".into());}
        Ok(NativeInputOwnerEntry{ticket:self.clone(),phase:NativeInputOwnerEntryPhase::Registration})
    }
    pub fn revoke(&self)->Result<()> {
        self.revoked.store(true,std::sync::atomic::Ordering::Release);
        if self.in_flight.load(std::sync::atomic::Ordering::Acquire)!=0 {return Err("Native Input owner callback busy; retain Native allocation".into());}
        Ok(())
    }
    pub fn matches_callback(&self,witness:&NativeInputCallbackWitness)->bool {
        Arc::ptr_eq(&self.issuer,&witness.admission) && match self.family {
            NativeInputOwnerFamily::Carbon=>witness.id==NativeInputCallbackId::CarbonPressed,
            NativeInputOwnerFamily::Hid=>matches!(witness.id,NativeInputCallbackId::HidInput|NativeInputCallbackId::HidRemoved|NativeInputCallbackId::HidFilter),
            NativeInputOwnerFamily::Keyword=>witness.id==NativeInputCallbackId::KeywordFilter,
        }
    }
}
impl NativeInputOwnerEntry {
    pub fn userdata(&self)->Result<usize> {
        self.pure_after_callback()?;
        Ok(self.ticket.userdata)
    }
    pub fn pure_after_callback(&self)->Result<()> {
        if self.ticket.retired(){return Err("Native Input owner ticket retired during callback".into());}
        if matches!(self.phase,NativeInputOwnerEntryPhase::Callback) {self.ticket.fully_bound()?;}
        Ok(())
    }
}
impl Drop for NativeInputOwnerEntry {
    fn drop(&mut self){self.ticket.in_flight.fetch_sub(1,std::sync::atomic::Ordering::AcqRel);}
}
/// Fixed leaf transaction, not another context registry. These identities
/// contain no Rc/pointer; each leaf resolves and verifies its actual CURRENT.
enum NativeInputOwnerContexts {
    Carbon(NativeInputInstallation),
    Hid{hid:NativeInputInstallation,events:NativeInputInstallation},
    Keyword(NativeInputInstallation),
}
impl NativeInputOwnerContexts {
    fn pure_after(&self)->Result<()> {
        match self {
            Self::Carbon(c)|Self::Keyword(c)=>c.pure_after(),
            Self::Hid{hid,events}=>{hid.pure_after()?;events.pure_after()}
        }
    }
    fn ids(&self)->[u64;3] {
        match self {Self::Carbon(c)=>[c.id,0,0],Self::Hid{hid,events}=>[0,hid.id,events.id],Self::Keyword(e)=>[0,0,e.id]}
    }
    fn verify(&self)->Result<()> {
        self.pure_after()?;
        match self {
            Self::Carbon(c)=>super::input_carbon::verify_owner_installation(c)?,
            Self::Hid{hid,events}=>{
                super::input_hid::verify_owner_installation(hid)?;
                self.pure_after()?;
                super::input_events::verify_owner_installation(events)?;
            }
            Self::Keyword(e)=>super::input_events::verify_owner_installation(e)?,
        }
        self.pure_after()
    }
    fn lookup(&self,slot:u64,generation:u64)->Result<Arc<NativeInputOwnerTicket>> {
        self.pure_after()?;
        let ticket=match self {
            Self::Carbon(c)=>super::input_carbon::owner_ticket(c,slot,generation)?,
            Self::Keyword(e)=>super::input_events::owner_ticket(e,slot,generation)?,
            Self::Hid{hid,events}=>{
                let left=super::input_hid::owner_ticket(hid,slot,generation);
                self.pure_after()?;
                let right=super::input_events::owner_ticket(events,slot,generation);
                self.pure_after()?;
                match (left,right) {
                    (Ok(a),Ok(b)) if Arc::ptr_eq(&a,&b)=>a,
                    (Ok(a),Err(_)) if a.bound_contexts.load(std::sync::atomic::Ordering::Acquire)&4==0=>a,
                    (Err(_),Ok(b)) if b.bound_contexts.load(std::sync::atomic::Ordering::Acquire)&2==0=>b,
                    _=>return Err("Native HID owner records do not contain the same transaction ticket".into()),
                }
            }
        };
        self.pure_after()?;
        Ok(ticket)
    }
    fn publish(&self,ticket:&Arc<NativeInputOwnerTicket>)->Result<()> {
        self.verify()?;
        match self {
            Self::Carbon(c)=>{
                super::input_carbon::publish_owner_ticket(c,ticket.clone())?;
                ticket.bound_contexts.fetch_or(1,std::sync::atomic::Ordering::AcqRel);
            }
            Self::Hid{hid,events}=>{
                super::input_hid::publish_owner_ticket(hid,ticket.clone())?;
                ticket.bound_contexts.fetch_or(2,std::sync::atomic::Ordering::AcqRel);
                self.pure_after()?;
                super::input_events::publish_owner_ticket(events,ticket.clone())?;
                ticket.bound_contexts.fetch_or(4,std::sync::atomic::Ordering::AcqRel);
            }
            Self::Keyword(e)=>{
                super::input_events::publish_owner_ticket(e,ticket.clone())?;
                ticket.bound_contexts.fetch_or(4,std::sync::atomic::Ordering::AcqRel);
            }
        }
        self.pure_after()?;
        ticket.fully_bound()
    }
    fn revoke(&self,ticket:&NativeInputOwnerTicket)->Result<()> {
        // Mark first, so no new or queued Native callback can enter while one
        // leaf is being removed. Existing inflight Native allocations stay held.
        let mut result=ticket.revoke();
        let mut remove=|bit:u8,operation:&dyn Fn()->Result<()>| {
            if ticket.bound_contexts.load(std::sync::atomic::Ordering::Acquire)&bit!=0 {
                let removed=self.pure_after().and_then(|_|operation());
                if removed.is_ok() {ticket.bound_contexts.fetch_and(!bit,std::sync::atomic::Ordering::AcqRel);}
                if result.is_ok() {result=removed;}
            }
        };
        match self {
            Self::Carbon(c)=>remove(1,&||super::input_carbon::revoke_owner_ticket(c,ticket)),
            Self::Hid{hid,events}=>{
                remove(2,&||super::input_hid::revoke_owner_ticket(hid,ticket));
                remove(4,&||super::input_events::revoke_owner_ticket(events,ticket));
            }
            Self::Keyword(e)=>remove(4,&||super::input_events::revoke_owner_ticket(e,ticket)),
        }
        self.pure_after()?;
        result
    }
}
struct NativeInputFailure(Arc<std::sync::atomic::AtomicBool>);
impl Drop for NativeInputFailure {
    fn drop(&mut self) { self.0.store(true,std::sync::atomic::Ordering::Release); }
}
impl HostAdmission {
    pub fn native_input_publisher(&self,authority:Arc<NativeInputAdmission>)->Result<super::profile_io::NativeInputPublisherCapability> {
        self.verify_native_input_issuer()?;
        if self.profile_provider.get().is_none() {return Err("Host profile provider must be prepared before Input publisher mint".into());}
        authority.verify_publisher_issuer(&self.admitted.life,&self.admitted.descriptors)?;
        authority.verify_installed_contexts()?;
        super::profile_io::issue_native_input_publisher(authority)
    }
    pub fn native_input_admission(&self, token:super::NativeInputRegistryToken)->Result<Arc<NativeInputAdmission>> {
        let _failure=NativeInputFailure(token.retired.clone());
        self.verify_native_input_issuer()?;
        if !Arc::ptr_eq(&token.life,&self.admitted.life) || !Arc::ptr_eq(&token.descriptors,&self.admitted.descriptors)
            || token.owner!=std::thread::current().id() || token.retired.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Native Input requires the actual Host Registry/guardian/descriptors".into());
        }
        native_input_code(&self.root,&self.endpoint,&self.endpoint_sha256,&self.source_inputs,&self.input_callbacks)?;
        native_input_publisher_data(&self.root,&self.endpoint,&self.endpoint_sha256,&self.source_inputs,&self.input_callbacks)?;
        // Five genuine code entries do not prove that opaque userdata is an
        // actual Native Arc/Rc allocation or that its owner outlives callback.
        // No witness until the separate real owner handoff is wired.
        Err("Native Input complete dynamic capture and source-owner retirement qualification is absent".into())
    }
}
impl NativeInputAdmission {
    pub fn reserve_installation(self:&Arc<Self>,kind:NativeInputContextKind,context:&super::owned::ModuleContext)->Result<NativeInputInstallation> {
        self.verify(context)?;
        let id=self.installation_next.fetch_update(std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire,
            |n|n.checked_add(1)).map_err(|_|"Native Input installation identity exhausted")?;
        if id==0 {self.token.retired.store(true,std::sync::atomic::Ordering::Release);return Err("Native Input invalid installation issuer".into());}
        self.not_retired()?;
        Ok(NativeInputInstallation{admission:self.clone(),kind,id,candidate:Some(Arc::new(std::sync::atomic::AtomicU8::new(0)))})
    }
    /// Check thread/issuer BEFORE a caller accesses a module CURRENT slot.
    /// The returned identity alone is insufficient: the actual context must
    /// match it and verify its ModuleContext outside the CURRENT borrow.
    pub fn expected_installation(self:&Arc<Self>,kind:NativeInputContextKind)->Result<NativeInputInstallation> {
        self.not_retired()?;
        let id=self.installations[kind.index()].load(std::sync::atomic::Ordering::Acquire);
        let result=NativeInputInstallation{admission:self.clone(),kind,id,candidate:None};
        result.pure_after()?;
        Ok(result)
    }
    fn verify_installed_contexts(self:&Arc<Self>)->Result<[NativeInputInstallation;3]> {
        let carbon=self.expected_installation(NativeInputContextKind::Carbon)?;
        let hid=self.expected_installation(NativeInputContextKind::Hid)?;
        let events=self.expected_installation(NativeInputContextKind::Events)?;
        NativeInputOwnerContexts::Carbon(carbon.clone()).verify()?;
        NativeInputOwnerContexts::Hid{hid:hid.clone(),events:events.clone()}.verify()?;
        for installation in [&carbon,&hid,&events] {installation.pure_after()?;}
        Ok([carbon,hid,events])
    }
    pub(crate) fn verify_publisher_issuer(&self,life:&Arc<dyn AliveLease>,descriptors:&Arc<DescriptorTable>)->Result<()> {
        if !Arc::ptr_eq(life,&self.token.life) || !Arc::ptr_eq(descriptors,&self.token.descriptors) {
            return Err("Native Input publisher is not the same Host issuer".into());
        }
        let context=super::bootstrap::owned_module_context(life,descriptors)?;
        self.verify(&context)
    }
    /// Only an actual Publisher Broker record can call this path. The sole
    /// HostAdmission constructor remains closed until the separate Native
    /// source-owned factory contract is qualified. Scalars do not issue it.
    pub(crate) fn owner_notice(self:&Arc<Self>,notice:super::profile_io::NativeInputOwnerNoticeV1,revoke:bool)->Result<Option<Arc<NativeInputOwnerTicket>>> {
        notice.check(revoke)?;
        self.not_retired()?;
        let context=super::bootstrap::owned_module_context(&self.token.life,&self.token.descriptors)?;
        self.verify(&context)?;
        let (family,contexts)=match notice.role {
            1=>(NativeInputOwnerFamily::Carbon,NativeInputOwnerContexts::Carbon(self.expected_installation(NativeInputContextKind::Carbon)?)),
            2=>(NativeInputOwnerFamily::Hid,NativeInputOwnerContexts::Hid{
                hid:self.expected_installation(NativeInputContextKind::Hid)?,events:self.expected_installation(NativeInputContextKind::Events)?,
            }),
            3=>(NativeInputOwnerFamily::Keyword,NativeInputOwnerContexts::Keyword(self.expected_installation(NativeInputContextKind::Events)?)),
            _=>return Err("Native Input owner family differs".into()),
        };
        contexts.verify()?;
        if revoke {
            let ticket=contexts.lookup(notice.ticket_slot,notice.ticket_generation)?;
            if !Arc::ptr_eq(&ticket.issuer,self) || !ticket.matches_owner_notice(&notice) {
                return Err("Native Input revoke does not match the actual stored owner ticket".into());
            }
            // This is the real stored Arc with the real shared inflight count,
            // never a ticket reconstructed from caller-provided scalar fields.
            contexts.revoke(&ticket)?;
            contexts.pure_after()?;
            return Ok(None);
        }
        let generation=self.owner_next.fetch_update(std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire,
            |n|n.checked_add(1)).map_err(|_|"Native Input owner generation exhausted")?;
        if generation==0 {self.token.retired.store(true,std::sync::atomic::Ordering::Release);return Err("Native Input owner issuer invalid".into());}
        let ticket=Arc::new(NativeInputOwnerTicket{
            family,userdata:notice.userdata,epoch:notice.owner_epoch,
            slot:u64::from(notice.role),generation,issuer:self.clone(),revoked:std::sync::atomic::AtomicBool::new(false),
            in_flight:std::sync::atomic::AtomicUsize::new(0),installations:contexts.ids(),
            bound_contexts:std::sync::atomic::AtomicU8::new(0),
        });
        struct Pending<'a>{ticket:&'a NativeInputOwnerTicket,complete:bool}
        impl Drop for Pending<'_>{
            fn drop(&mut self){if !self.complete {let _=self.ticket.revoke();}}
        }
        let mut pending=Pending{ticket:&ticket,complete:false};
        if let Err(error)=contexts.publish(&ticket) {
            let _=contexts.revoke(&ticket);
            return Err(error);
        }
        contexts.pure_after()?;
        pending.complete=true;
        drop(pending);
        Ok(Some(ticket))
    }
    fn not_retired(&self)->Result<()> {
        if self.token.retired.load(std::sync::atomic::Ordering::Acquire)
            || super::READY_RETIRED.load(std::sync::atomic::Ordering::Acquire)
            || self.token.owner!=std::thread::current().id() {
            return Err("Native Input issuer retired or foreign owner".into());
        }
        Ok(())
    }
    fn verify(&self,context:&super::owned::ModuleContext)->Result<()> {
        let result=(|| {
            self.not_retired()?;
            if !super::SOURCE_ROUTES_QUALIFIED || !Arc::ptr_eq(&context.life,&self.token.life) {
                return Err("Native Input source/guardian identity differs".into());
            }
            self.token.life.before_effect()?;
            if !self.token.life.is_alive() { return Err("Native Input guardian retired".into()); }
            let actual=super::bootstrap::owned_module_context(&self.token.life,&self.token.descriptors)?;
            for (candidate,expected) in [&context.home,&context.cache,&context.temp,&context.applications].into_iter()
                .zip([&actual.home,&actual.cache,&actual.temp,&actual.applications]) {
                if candidate.path()!=expected.path() { return Err("Native Input module root differs".into()); }
                let (a,b)=(candidate.checked(candidate.path())?,expected.checked(expected.path())?);
                let (mut sa,mut sb):(libc::stat,libc::stat)=unsafe{(std::mem::zeroed(),std::mem::zeroed())};
                if unsafe{libc::fstat(a.as_raw_fd(),&mut sa)}!=0 || unsafe{libc::fstat(b.as_raw_fd(),&mut sb)}!=0
                    || sa.st_dev!=sb.st_dev || sa.st_ino!=sb.st_ino {
                    return Err("Native Input retained root identity differs".into());
                }
            }
            native_input_code(&self.root,&self.endpoint,&self.endpoint_sha256,&self.source_inputs,&self.pins)?;
            native_input_publisher_data(&self.root,&self.endpoint,&self.endpoint_sha256,&self.source_inputs,&self.pins)?;
            self.not_retired()
        })();
        if result.is_err() { self.token.retired.store(true,std::sync::atomic::Ordering::Release); }
        result
    }
    fn witness(self:&Arc<Self>,id:NativeInputCallbackId)->NativeInputCallbackWitness {
        NativeInputCallbackWitness {admission:self.clone(),id}
    }
    pub fn carbon_pressed(self:&Arc<Self>)->NativeInputCallbackWitness { self.witness(NativeInputCallbackId::CarbonPressed) }
    pub fn hid_input(self:&Arc<Self>)->NativeInputCallbackWitness { self.witness(NativeInputCallbackId::HidInput) }
    pub fn hid_removed(self:&Arc<Self>)->NativeInputCallbackWitness { self.witness(NativeInputCallbackId::HidRemoved) }
    pub fn hid_filter(self:&Arc<Self>)->NativeInputCallbackWitness { self.witness(NativeInputCallbackId::HidFilter) }
    pub fn keyword_filter(self:&Arc<Self>)->NativeInputCallbackWitness { self.witness(NativeInputCallbackId::KeywordFilter) }
}
impl NativeInputCallbackWitness {
    pub fn verify(&self,context:&super::owned::ModuleContext)->Result<()> { self.admission.verify(context) }
    pub fn id(&self)->NativeInputCallbackId { self.id }
    pub fn carbon_handler(&self)->Result<InputCarbon> {
        if self.id!=NativeInputCallbackId::CarbonPressed { return Err("Native Input callback is not Carbon".into()); }
        self.admission.not_retired()?;Ok(self.admission.functions.carbon)
    }
    pub fn hid_handler(&self)->Result<InputHid> {
        self.admission.not_retired()?;
        match self.id {NativeInputCallbackId::HidInput=>Ok(self.admission.functions.input),NativeInputCallbackId::HidRemoved=>Ok(self.admission.functions.removed),_=>Err("Native Input callback is not HID".into())}
    }
    pub fn event_handler(&self)->Result<InputEvent> {
        self.admission.not_retired()?;
        match self.id {NativeInputCallbackId::HidFilter=>Ok(self.admission.functions.filter),NativeInputCallbackId::KeywordFilter=>Ok(self.admission.functions.keyword),_=>Err("Native Input callback is not an event filter".into())}
    }
    /// Call only after acquiring the leaf's stable registration/inflight lease,
    /// with no leaf/Registry lock held. Module attestation uses verify(), which
    /// deliberately does NOT recurse into Registry::ready().
    pub fn before_callback(&self,context:&super::owned::ModuleContext)->Result<NativeInputCallbackGuard> {
        self.verify(context)?;
        let readiness={
            let registry=super::REGISTRY.get().ok_or("Native Input Registry is not published")?
                .try_lock().map_err(|_|"Native Input Registry busy during callback")?;
            if !Arc::ptr_eq(&registry.retired,&self.admission.token.retired) || !Arc::ptr_eq(&registry.life,&self.admission.token.life) {
                return Err("Native Input callback requires this actually ready Registry".into());
            }
            registry.ready_snapshot()
        };
        if readiness.ready()!=0xF100002600003FFF {return Err("Native Input callback requires this actually ready Registry".into());}
        // Registry::ready invokes module verifiers. Close that callback boundary
        // with a fresh source/body check before releasing a callback permit.
        self.verify(context)?;
        self.admission.not_retired()?;
        Ok(NativeInputCallbackGuard{admission:self.admission.clone()})
    }
}
impl NativeInputCallbackGuard {
    /// No dynamic guardian/metadata callback and no userdata read. This checks
    /// monotone retirement/owner state only. Every subsequent effect needs a
    /// fresh before_callback/verify; local registration epoch is the leaf's job.
    pub fn pure_after_callback(&self)->Result<()> { self.admission.not_retired() }
}
fn native_input_code(root:&OwnedRoot,endpoint:&Path,endpoint_sha:&str,sources:&std::collections::BTreeMap<String,String>,pins:&NativeInputCompilerPins)->Result<[usize;5]> {
    if !super::SOURCE_ROUTES_QUALIFIED { return Err("Native Input source routes are not qualified".into()); }
    native_input_pin_shape(root.path(),sources,pins)?;
    reference_mapping_infrastructure()?;
    image(endpoint)?;
    checked_hash(root,endpoint,endpoint_sha)?;
    checked_hash(root,Path::new(&pins.evidence.path),&pins.evidence.sha256)?;
    for (path,sha) in sources { checked_hash(root,Path::new(path),sha)?; }
    let header=unsafe{_NSGetMachExecuteHeader()} as usize;
    reference_mapping(header,32,None)?;
    let word=|at:usize|unsafe{(at as *const u32).read_unaligned()};
    if word(header)!=0xfeedfacf || word(header+12)!=2 { return Err("Native Input requires actual MH_EXECUTE".into()); }
    let (count,bytes)=(word(header+16) as usize,word(header+20) as usize);
    if count>4096 || bytes>16*1024*1024 { return Err("Native Input command bounds".into()); }
    reference_mapping(header,32+bytes,None)?;
    let image_count=unsafe{_dyld_image_count()};
    if image_count>4096 { return Err("Native Input image roster bound".into()); }
    let index=(0..image_count).find(|i|unsafe{_dyld_get_image_header(*i) as usize==header}).ok_or("Native Input main absent from dyld")?;
    let slide=unsafe{_dyld_get_image_vmaddr_slide(index)};
    let capsule=header.checked_add(pins.capsule_offset).ok_or("Native Input capsule overflow")?;
    let (mut at,mut uuid,mut capsules)=(0usize,None,0usize);
    for _ in 0..count {
        if at.checked_add(8).is_none_or(|n|n>bytes) { return Err("Native Input truncated command".into()); }
        let p=header+32+at;
        let (kind,size)=(word(p),word(p+4) as usize);
        if size<8 || at.checked_add(size).is_none_or(|n|n>bytes) { return Err("Native Input invalid command".into()); }
        if kind==0x1b {
            if size!=24 || uuid.is_some() { return Err("Native Input invalid UUID command".into()); }
            let mut value=[0u8;16];unsafe{std::ptr::copy_nonoverlapping((p+8) as *const u8,value.as_mut_ptr(),16)};uuid=Some(value);
        }
        if kind==0x19 {
            if size<72 { return Err("Native Input truncated segment".into()); }
            let sections=word(p+64) as usize;
            if sections>4096 || 72+sections*80>size { return Err("Native Input section bounds".into()); }
            for section in 0..sections {
                let sec=p+72+section*80;
                let name=unsafe{std::slice::from_raw_parts(sec as *const u8,16)};
                if name.split(|b|*b==0).next()!=Some(b"__ruycast_input") { continue; }
                let segment=unsafe{std::slice::from_raw_parts((sec+16) as *const u8,16)};
                let addr=unsafe{((sec+32) as *const u64).read_unaligned()};
                let length=unsafe{((sec+40) as *const u64).read_unaligned()};
                if segment.split(|b|*b==0).next()!=Some(b"__DATA_CONST") || length!=56
                    || usize::try_from(addr).ok().and_then(|a|a.checked_add_signed(slide))!=Some(capsule) {
                    return Err("Native Input exact capsule section differs".into());
                }
                capsules+=1;
            }
        }
        at+=size;
    }
    if at!=bytes || uuid!=Some(pins.main_uuid) || capsules!=1 { return Err("Native Input actual UUID/capsule roster differs".into()); }
    if !capsule.is_multiple_of(8) { return Err("Native Input capsule alignment".into()); }
    reference_mapping(capsule,56,Some(false))?;
    // Exact whole RO/nonexec capsule, fixed private offset and Main UUID have
    // been established BEFORE the first word/slot is read.
    if unsafe{(capsule as *const u64).read()}!=0xF1000026494E5054 || word(capsule+8)!=1 || word(capsule+12)!=5 {
        return Err("Native Input capsule header differs".into());
    }
    let mut actual=[0usize;5];
    for (i,pin) in pins.callbacks.iter().enumerate() {
        let address=header.checked_add(pin.offset).ok_or("Native Input callback overflow")?;
        actual[i]=unsafe{((capsule+16+i*8) as *const usize).read()};
        if actual[i]!=address { return Err("Native Input capsule code slot differs".into()); }
        reference_code(address,endpoint)?;reference_mapping(address,pin.bytes,Some(true))?;
        let body=unsafe{std::slice::from_raw_parts(address as *const u8,pin.bytes)};
        if digest(body)!=pin.body_sha256 { return Err("Native Input actual linked body differs".into()); }
    }
    Ok(actual)
}
// Pure data-shape rejection, not a source/ABI/owner admission constructor.
fn native_input_pin_shape(root:&Path,sources:&std::collections::BTreeMap<String,String>,pins:&NativeInputCompilerPins)->Result<()> {
    if pins.capsule_offset==0 || !pins.capsule_offset.is_multiple_of(8) || pins.main_uuid==[0;16] {
        return Err("Native Input missing capsule/UUID pins".into());
    }
    reference_hash(&pins.evidence.sha256)?;
    for (pin,id) in pins.callbacks.iter().zip(NATIVE_INPUT_IDS) {
        if pin.id!=id || pin.offset==0 || pin.bytes==0 || pin.bytes>65536 || pin.offset.checked_add(pin.bytes).is_none() {
            return Err("Native Input exact ordered callback pins differ".into());
        }
        reference_hash(&pin.body_sha256)?;reference_hash(&pin.source.sha256)?;
        let source=match id {NativeInputCallbackId::CarbonPressed=>"carbon_hotkeys.rs",NativeInputCallbackId::HidInput|NativeInputCallbackId::HidRemoved|NativeInputCallbackId::HidFilter=>"hid_service.rs",NativeInputCallbackId::KeywordFilter=>"keyword_capture.rs"};
        if Path::new(&pin.source.path)!=root.join("rust-native/src").join(source)
            || sources.get(&pin.source.path)!=Some(&pin.source.sha256) { return Err("Native Input callback is not the same compiler source member".into()); }
    }
    Ok(())
}
#[cfg(test)]
mod native_input_admission_tests {
    use super::*;
    #[test]
    fn objc_consumer_metadata_needs_same_producer_and_actual_selected_bytes() {
        let output=HostInput{path:"/owned/target/libobjc2-same.rlib".into(),sha256:"a".repeat(64)};
        let selected=HostInput{path:"/owned/target/libobjc2-same.rmeta".into(),sha256:"b".repeat(64)};
        let mut consumer=PublisherObjcConsumerData{selected:selected.clone(),linked_output:output.clone()};
        let mut compiler=PublisherObjcCompileData{command:["/compiler/rustc","--crate-name","objc2","--crate-type","lib",
            "--emit=dep-info,metadata,link","-C","extra-filename=-same","--out-dir","/owned/target"]
            .map(str::to_owned).to_vec(),source:output.clone(),dep_info:output.clone(),actual_inputs:Default::default()};
        let inputs=std::collections::BTreeMap::from([(selected.path.clone(),selected.clone()),(output.path.clone(),output.clone())]);
        publisher_objc_consumer_shape(&consumer,&output,&compiler,&inputs).unwrap();
        consumer.selected.sha256="c".repeat(64);
        assert!(publisher_objc_consumer_shape(&consumer,&output,&compiler,&inputs).is_err());
        consumer.selected=selected;
        compiler.command[7]="extra-filename=-different".into();
        assert!(publisher_objc_consumer_shape(&consumer,&output,&compiler,&inputs).is_err());
        compiler.command[7]="extra-filename=-same".into();compiler.command[5]="--emit=dep-info,metadata".into();
        assert!(publisher_objc_consumer_shape(&consumer,&output,&compiler,&inputs).is_err());
        consumer.selected=output.clone();
        publisher_objc_consumer_shape(&consumer,&output,&compiler,&inputs).unwrap();
        assert!(publisher_objc_consumer_shape(&consumer,&output,&compiler,&Default::default()).is_err());
        // Shape/data validation only; no compiler or callback authority minted.
    }
    #[test]
    fn publisher_branch_targets_use_signed_actual_pc_and_reject_nonbranches_and_wraparound() {
        assert_eq!(publisher_branch_target(0x9400_0002,0x1000,4).unwrap(),0x100c);
        assert_eq!(publisher_branch_target(0x17ff_ffff,0x1000,4).unwrap(),0x1000);
        assert!(publisher_branch_target(0x5400_0000,0x1000,0).is_err());
        assert!(publisher_branch_target(0x9400_0000,0x1000,1).is_err());
        assert!(publisher_branch_target(0x17ff_ffff,0,0).is_err());
        assert!(publisher_branch_target(0x1400_0001,u64::MAX-3,0).is_err());
        assert!(publisher_branch_target(0x1400_0000,u64::MAX-3,4).is_err());
    }
    #[test]
    fn publisher_host_io_data_rejects_ticket_slot_layout_and_incomplete_signature_joins() {
        let mut types=PublisherHostIoTypes {
            ticket_die:1,publisher_arc_die:2,publisher_die:3,api_reference_die:4,api_die:5,
            consumed_phase_die:6,consumed_payload_die:7,bound_box_die:8,bound_host_die:9,
            slots:[("bind",24,1),("request",32,2),("release_handle",40,1),("return_buffer",48,2)]
                .map(|(name,offset,count)|PublisherHostIoSlot{name:name.into(),offset,member_die:offset,
                    pointer_die:100+offset,signature_die:200+offset,parameter_dies:vec![300;count],result_die:400}),
        };
        publisher_host_io_shape(&types,1).unwrap();
        assert!(publisher_host_io_shape(&types,10).is_err());
        types.slots.swap(0,1);
        assert!(publisher_host_io_shape(&types,1).is_err());
        types.slots.swap(0,1);
        types.slots[1].parameter_dies.pop();
        assert!(publisher_host_io_shape(&types,1).is_err());
        types.slots[1].parameter_dies.push(300);
        types.consumed_payload_die=0;
        assert!(publisher_host_io_shape(&types,1).is_err());
        // Synthetic object-local DATA only; no code/capture/userdata authority,
        // Runtime table, Host admission or positive ticket is constructed.
    }
    #[test]
    fn native_input_global_authority_contains_no_main_thread_context() {
        fn send_sync<T:Send+Sync>(){}
        send_sync::<NativeInputAdmission>();
        send_sync::<NativeInputInstallation>();
        send_sync::<NativeInputOwnerTicket>();
        assert_eq!(std::mem::size_of::<[std::sync::atomic::AtomicU64;3]>(),24);
    }
    #[test]
    fn fixed_input_pins_reject_wrong_role_body_size_source_and_shape_without_authority() {
        let names=["carbon_hotkeys.rs","hid_service.rs","hid_service.rs","hid_service.rs","keyword_capture.rs"];
        let pins=NativeInputCompilerPins{main_uuid:[1;16],capsule_offset:256,
            callbacks:std::array::from_fn(|i|NativeInputCallbackPin{id:NATIVE_INPUT_IDS[i],offset:1024+i*16,bytes:16,body_sha256:"a".repeat(64),
                source:HostInput{path:format!("/owned/rust-native/src/{}",names[i]),sha256:"b".repeat(64)}}),
            evidence:HostInput{path:"/owned/compiled-evidence.json".into(),sha256:"c".repeat(64)}};
        let sources=pins.callbacks.iter().map(|p|(p.source.path.clone(),p.source.sha256.clone())).collect();
        native_input_pin_shape(Path::new("/owned"),&sources,&pins).unwrap();
        let mut wrong=pins.clone();wrong.callbacks.swap(0,1);
        assert!(native_input_pin_shape(Path::new("/owned"),&sources,&wrong).is_err());
        let mut wrong=pins.clone();wrong.callbacks[0].bytes=65537;
        assert!(native_input_pin_shape(Path::new("/owned"),&sources,&wrong).is_err());
        let mut wrong=pins.clone();wrong.callbacks[0].source.sha256="d".repeat(64);
        assert!(native_input_pin_shape(Path::new("/owned"),&sources,&wrong).is_err());
        let mut wrong=pins.clone();wrong.capsule_offset=257;
        assert!(native_input_pin_shape(Path::new("/owned"),&sources,&wrong).is_err());
        let mut value=serde_json::to_value(pins).unwrap();value.as_object_mut().unwrap().insert("authority".into(),serde_json::json!(true));
        assert!(serde_json::from_value::<NativeInputCompilerPins>(value).is_err());
    }
    #[test]
    fn actual_registry_retirement_arc_is_virgin_once_and_failure_drop_is_monotone() {
        struct Live;
        impl AliveLease for Live {fn is_alive(&self)->bool{true}fn before_effect(&self)->Result<()>{Ok(())}}
        let mut registry=super::super::Registry::new(Arc::new(Live));
        assert!(Arc::get_mut(&mut registry.retired).is_some());
        let observer=registry.retired.clone();
        assert!(Arc::get_mut(&mut registry.retired).is_none());
        {
            let _failure=NativeInputFailure(observer.clone());
            assert!(!registry.retired.load(std::sync::atomic::Ordering::Acquire));
        }
        assert!(registry.retired.load(std::sync::atomic::Ordering::Acquire));
        drop(observer);
        assert!(registry.retired.load(std::sync::atomic::Ordering::Acquire));
        // This checks actual shared-state mechanics only. No HostAdmission,
        // input witness, callback pointer or userdata owner is manufactured.
    }
    #[test]
    fn worker_tail_rejects_actual_registry_retirement_even_when_nested_verifier_returns_ok() {
        struct Live;
        impl AliveLease for Live {fn is_alive(&self)->bool{true}fn before_effect(&self)->Result<()>{Ok(())}}
        let registry=super::super::Registry::new(Arc::new(Live));
        let observed=registry.retired.clone();
        reference_retirement_tail(&observed).unwrap();
        let nested=||->Result<()> {registry.retired.store(true,std::sync::atomic::Ordering::Release);Ok(())};
        assert!(nested().is_ok());
        assert!(registry.life.is_alive());
        assert_eq!(reference_retirement_tail(&observed).unwrap_err(),"Reference worker Registry retired during verification");
        // This is the exact new pure tail over real Registry state, not an
        // invented positive Source14/Reference-admission fixture.
    }
}
/// Separate Reference constructor admission. It has no Native capability or
/// sink, and never substitutes a Reference packet for a Native Host grant.
pub struct ReferencePins<'a> {
    pub owner_uid: u32,
    pub root: &'a Path,
    pub endpoint: &'a Path,
    pub endpoint_sha256: &'a str,
    pub sdk_artifact: &'a Path,
    pub nonce_sha256: &'a str,
    pub grant: Fingerprint,
    pub stdio: [Fingerprint; 3],
    pub source_manifest: &'a Path,
    pub initializer_source: &'a Path,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceCodePin {
    pub symbol: String,
    pub image_offset: usize,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRuntime {
    pub home: String,
    pub cache: String,
    pub temp: String,
    pub applications: String,
    pub profile: String,
    pub fixture: HostInput,
    pub authority_sha256: String,
    pub arm: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceGrant {
    pub owner_uid: u32,
    pub role: String,
    pub endpoint_sha256: String,
    pub sdk_artifact_sha256: String,
    pub source_set_sha256: String,
    pub nonce: String,
    pub stdio: [Fingerprint; 3],
    pub source_manifest: HostInput,
    pub initializer_source: HostInput,
    pub main_initializer_offset: usize,
    pub main_table_offset: usize,
    pub main_functions: Vec<ReferenceCodePin>,
    pub sdk_functions: Vec<ReferenceCodePin>,
    pub runtime: ReferenceRuntime,
}
const REFERENCE_MAIN_SYMBOLS: [&str; 25] = [
    "rcf1_install_guard", "rcf1_borrow_delegate", "rcf1_create_with_process_provider",
    "rcf1_cancel_system_search", "rcf1_drop", "rcf1_query", "rcf1_mode", "rcf1_select",
    "rcf1_actions", "rcf1_set_commands", "rcf1_snapshot", "rcf1_result_count",
    "rcf1_visible_count", "rcf1_selected_id_count", "rcf1_filtered_count",
    "rcf1_apply_system_results", "rcf1_process_sort", "rcf1_set_process_sort",
    "rcf1_process_snapshot", "rcf1_config_data", "rcf1_encode_clips", "rcf1_record_timing",
    "rcf1_record_recency", "rcf1_call", "rcf1_content_view",
];
const REFERENCE_SDK_SYMBOLS: [&str; 5] = [
    "entry", "guard", "reply", "process_provider", "main_owner_perform",
];
fn reference_hash(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return Err("Reference hash syntax".into());
    }
    Ok(())
}
fn reference_code_pins(pins: &[ReferenceCodePin], names: &[&str]) -> Result<()> {
    if pins.len() != names.len() || pins.iter().zip(names).any(|(p,n)| p.symbol != *n || p.image_offset == 0) {
        return Err("Reference exact ordered code pins differ".into());
    }
    Ok(())
}
// fstat identifies an endpoint, not the connected peer of a Darwin pipe.
// Channel separation additionally depends on the trusted producer's distinct
// fresh pipe factories; it cannot be inferred from this endpoint comparison.
fn reference_endpoint_alias(a:Fingerprint,b:Fingerprint)->bool {
    a.device==b.device && a.inode==b.inode && a.kind==b.kind
}
// This fixed collection is an input comparison, not a permission obtained from
// getenv. Every value except the three descriptors comes from the private cap.
fn reference_environment_values(grant: &ReferenceGrant, sdk: &Path) -> Result<Vec<(&'static str, String)>> {
    let r = &grant.runtime;
    for path in [&r.home,&r.cache,&r.temp,&r.applications,&r.profile,&r.fixture.path] {
        let p = Path::new(path);
        if !p.is_absolute() || path.len() > 8192 || path.as_bytes().contains(&0)
            || p.components().any(|v| matches!(v,std::path::Component::CurDir|std::path::Component::ParentDir)) {
            return Err("Reference runtime path syntax".into());
        }
    }
    for hash in [&r.fixture.sha256,&r.authority_sha256,&grant.sdk_artifact_sha256] { reference_hash(hash)?; }
    if !matches!(r.arm.as_str(),"A"|"B") { return Err("Reference arm differs".into()); }
    Ok(vec![
        ("HOME",r.home.clone()), ("TMPDIR",r.temp.clone()),
        ("RUYCAST_HOME",r.profile.clone()), ("RUSTCAST_HOME",r.profile.clone()),
        ("RUYCAST_F1_FIXTURE",r.fixture.path.clone()),
        ("RUYCAST_F1_FIXTURE_SHA256",r.fixture.sha256.clone()),
        ("RUYCAST_F1_AUTHORITY_SHA256",r.authority_sha256.clone()),
        ("RUYCAST_F1_ARM",r.arm.clone()),
        ("RUYCAST_F1_GUARD_SHA256",grant.sdk_artifact_sha256.clone()),
        ("DYLD_INSERT_LIBRARIES",sdk.to_str().ok_or("Reference SDK path UTF8")?.into()),
        ("DYLD_BIND_AT_LAUNCH","1".into()),
        ("__CF_USER_TEXT_ENCODING",format!("0x{:X}:0x0:0x0",grant.owner_uid)),
    ])
}
fn reference_environment(entries: &[(String,String)], expected: &[(&str,String)]) -> Result<(i32,i32,i32)> {
    if entries.len() != 15 || expected.len() != 12 { return Err("Reference exact environment count".into()); }
    let mut seen = std::collections::BTreeSet::new();
    let (mut life,mut admit,mut grant) = (None,None,None);
    for (key,value) in entries {
        if !seen.insert(key.as_str()) { return Err("Reference duplicate environment field".into()); }
        match key.as_str() {
            "F1_LIFE_FD" => life=Some(value.parse::<i32>().map_err(|_|"Reference life descriptor encoding")?),
            "F1_ADMIT_FD" => admit=Some(value.parse::<i32>().map_err(|_|"Reference admit descriptor encoding")?),
            "F1_CORE_GRANT_FD" => grant=Some(value.parse::<i32>().map_err(|_|"Reference grant descriptor encoding")?),
            _ if expected.iter().any(|(k,v)| *k==key && v==value) => (),
            _ => return Err("Reference unexpected environment field or value".into()),
        }
    }
    let (a,b,c)=(life.ok_or("Reference life missing")?,admit.ok_or("Reference admit missing")?,grant.ok_or("Reference grant missing")?);
    if a<=2 || b<=2 || c<=2 || a==b || a==c || b==c { return Err("Reference descriptor alias".into()); }
    Ok((a,b,c))
}
fn reference_environment_snapshot() -> Result<Vec<(String,String)>> {
    unsafe {
        let raw=_NSGetEnviron();
        if raw.is_null() || (*raw).is_null() { return Err("Reference environment missing".into()); }
        let mut entries=Vec::new();
        for i in 0..16 {
            let p=(*raw).add(i).read();
            if p.is_null() { return Ok(entries); }
            let bytes=bounded(p,16384)?;
            let text=std::str::from_utf8(&bytes).map_err(|_|"Reference environment UTF8")?;
            let (key,value)=text.split_once('=').ok_or("Reference environment encoding")?;
            entries.push((key.into(),value.into()));
        }
    }
    Err("Reference environment bound".into())
}
/// Opaque constructor proof, not a public table or full fourteen-scope grant.
/// MAIN and the inserted SDK remain process-lifetime images; no unload/load API
/// or alternate code image can replace either stored image identity.
/// Read the epoch from the exact pinned manifest, not an environment clock.
/// Reopening the path after the bounded read rejects descriptor/path replacement.
fn reference_fixture_epoch(root:&OwnedRoot,input:&HostInput,owner_uid:u32)->Result<u64> {
    use std::io::Read;
    const LIMIT:u64=2*1024*1024;
    reference_hash(&input.sha256)?;
    let file=std::fs::File::from(root.checked(Path::new(&input.path))?);
    let capture=|fd:i32|->Result<libc::stat>{
        let mut stat:libc::stat=unsafe{std::mem::zeroed()};
        if unsafe{libc::fstat(fd,&mut stat)}!=0 {return Err("Reference fixture descriptor unavailable".into());}
        if stat.st_mode&libc::S_IFMT!=libc::S_IFREG || stat.st_nlink!=1 || stat.st_uid!=owner_uid
            || stat.st_size<0 || stat.st_size as u64>LIMIT {
            return Err("Reference fixture is not the bounded owned manifest".into());
        }
        Ok(stat)
    };
    let same=|a:&libc::stat,b:&libc::stat| {
        a.st_dev==b.st_dev && a.st_ino==b.st_ino && a.st_mode==b.st_mode && a.st_nlink==b.st_nlink
            && a.st_uid==b.st_uid && a.st_gid==b.st_gid && a.st_size==b.st_size
            && a.st_mtime==b.st_mtime && a.st_mtime_nsec==b.st_mtime_nsec
            && a.st_ctime==b.st_ctime && a.st_ctime_nsec==b.st_ctime_nsec
    };
    let before=capture(file.as_raw_fd())?;
    let mut bytes=Vec::new();
    (&file).take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"Reference fixture bounded read failed")?;
    let after=capture(file.as_raw_fd())?;
    let named=root.checked(Path::new(&input.path))?;
    let named=capture(named.as_raw_fd())?;
    if bytes.len() as u64>LIMIT || bytes.len() as i64!=before.st_size || !same(&before,&after) || !same(&before,&named)
        || digest(&bytes)!=input.sha256 {
        return Err("Reference fixture changed during read".into());
    }
    #[derive(Deserialize)]
    struct Epoch {schema:String,epoch_unix_seconds:u64}
    let manifest:Epoch=serde_json::from_slice(&bytes).map_err(|_|"Reference fixture epoch/schema invalid")?;
    if manifest.schema!="ruycast.f1.fixture.v1" {return Err("Reference fixture schema differs".into());}
    Ok(manifest.epoch_unix_seconds)
}
/// Fixed durable ranking leaf only. The actual descriptor stays open across
/// the sealed dynamic check; FD and fresh path are compared AFTER that check.
fn reference_process_ranking_bytes(
    home:&OwnedRoot,profile:&Path,uid:u32,after:impl FnOnce()->Result<()>,
)->Result<Vec<u8>> {
    use std::io::Read;
    const LIMIT:u64=1_000_000;
    let path=profile.join("search-ranking.json");
    let file=std::fs::File::from(home.checked(&path)?);
    let capture=|fd:i32|->Result<libc::stat>{
        let mut stat:libc::stat=unsafe{std::mem::zeroed()};
        if unsafe{libc::fstat(fd,&mut stat)}!=0 {return Err("Reference ranking descriptor unavailable".into());}
        if stat.st_mode&libc::S_IFMT!=libc::S_IFREG || stat.st_nlink!=1 || stat.st_uid!=uid
            || stat.st_size<0 || stat.st_size as u64>LIMIT {
            return Err("Reference ranking is not the bounded owned file".into());
        }
        Ok(stat)
    };
    let same=|a:&libc::stat,b:&libc::stat| {
        a.st_dev==b.st_dev && a.st_ino==b.st_ino && a.st_mode==b.st_mode && a.st_nlink==b.st_nlink
            && a.st_uid==b.st_uid && a.st_gid==b.st_gid && a.st_size==b.st_size
            && a.st_mtime==b.st_mtime && a.st_mtime_nsec==b.st_mtime_nsec
            && a.st_ctime==b.st_ctime && a.st_ctime_nsec==b.st_ctime_nsec
    };
    let before=capture(file.as_raw_fd())?;
    let mut bytes=Vec::new();
    (&file).take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"Reference ranking bounded read failed")?;
    if bytes.len() as u64>LIMIT || bytes.len() as i64!=before.st_size || !same(&before,&capture(file.as_raw_fd())?) {
        return Err("Reference ranking changed during read".into());
    }
    after()?;
    let named=home.checked(&path)?;
    if !same(&before,&capture(file.as_raw_fd())?) || !same(&before,&capture(named.as_raw_fd())?) {
        return Err("Reference ranking changed during qualification".into());
    }
    Ok(bytes)
}
pub struct ReferenceAdmission {
    admitted: Admitted,
    root: OwnedRoot,
    owner: std::thread::ThreadId,
    endpoint: std::path::PathBuf,
    sdk: std::path::PathBuf,
    grant: ReferenceGrant,
    table: usize,
    sdk_addresses: [(&'static str,usize);5],
    roots: [OwnedRoot;4],
    profile_directory: OwnedFd,
    #[cfg(rcf1_reference_sdk)]
    registry_retired:std::sync::OnceLock<std::sync::Weak<std::sync::atomic::AtomicBool>>,
}
fn reference_profile_identity(directory: &OwnedFd) -> Result<(u64, u64)> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(directory.as_raw_fd(), &mut stat) } != 0
        || stat.st_mode & libc::S_IFMT != libc::S_IFDIR
        || stat.st_mode & 0o077 != 0
    {
        return Err("Reference profile must remain a private directory".into());
    }
    Ok((stat.st_dev as u64, stat.st_ino))
}

fn verify_reference_named_root(root: &OwnedRoot) -> Result<OwnedRoot> {
    let named = OwnedRoot::open(root.path())?;
    if reference_profile_identity(&named.checked(root.path())?)?
        != reference_profile_identity(&root.checked(root.path())?)?
    {
        return Err("Reference named runtime root identity changed".into());
    }
    Ok(named)
}

fn verify_reference_profile_directory(home: &OwnedRoot, profile: &Path, held: &OwnedFd) -> Result<OwnedFd> {
    // Swift opens the absolute path. Reopening only below the old home FD
    // could miss replacement of a named ancestor of that path.
    let named_home = verify_reference_named_root(home)?;
    let current = named_home.open_watch_directory(profile)?;
    if reference_profile_identity(&current)? != reference_profile_identity(held)? {
        return Err("Reference profile directory identity changed".into());
    }
    Ok(current)
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
static REFERENCE_LOCK_OWNER: std::sync::OnceLock<Arc<ReferenceAdmission>> = std::sync::OnceLock::new();

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_file_root(owner: &ReferenceAdmission, path: &Path) -> Result<OwnedRoot> {
    if unsafe { libc::pthread_main_np() } == 1 {
        owner.before_reference_initialization()?;
    } else {
        owner.before_reference_worker_callback()?;
    }
    let context = super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors())?;
    let profile = Path::new(&owner.grant.runtime.profile);
    if path.starts_with(profile) {
        let root = OwnedRoot::open(profile)?;
        if reference_profile_identity(&root.checked(profile)?)?
            != reference_profile_identity(&owner.profile_directory)?
        {
            return Err("Reference file profile identity differs".into());
        }
        return Ok(root);
    }
    [context.cache, context.temp, context.applications, context.home]
        .into_iter().find(|root| path.starts_with(root.path()))
        .ok_or_else(|| "Reference file outside owned roots".to_owned())
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_file_lstat(path: *const libc::c_char, output: *mut libc::stat) -> i32 {
    use std::os::unix::ffi::OsStrExt;
    let result = std::panic::catch_unwind(|| -> std::result::Result<libc::stat, i32> {
        if path.is_null() || output.is_null() {
            return Err(libc::EFAULT);
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        let path = Path::new(std::ffi::OsStr::from_bytes(unsafe { CStr::from_ptr(path) }.to_bytes()));
        let root = reference_file_root(owner, path).map_err(|_| libc::EPERM)?;
        let metadata = root.lstat(path)?;
        owner.admitted.life.before_effect().map_err(|_| libc::EPERM)?;
        owner.registry_current().map_err(|_| libc::EPERM)?;
        owner.verify_profile_directory().map_err(|_| libc::EPERM)?;
        Ok(metadata)
    });
    let errno = match result {
        Ok(Ok(metadata)) => {
            unsafe { output.write(metadata) };
            return 0;
        }
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_lstat_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"lstat",
        original: libc::lstat as *const () as usize,
        replacement: reference_file_lstat as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_unlink_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"unlink",
        original: libc::unlink as *const () as usize,
        replacement: reference_file_unlink as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_bind_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"bind",
        original: libc::bind as *const () as usize,
        replacement: reference_control_bind as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_connect_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"connect",
        original: libc::connect as *const () as usize,
        replacement: reference_control_connect as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_chmod_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"fchmodat",
        original: libc::fchmodat as *const () as usize,
        replacement: reference_control_chmod as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_listen_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"listen",
        original: libc::listen as *const () as usize,
        replacement: reference_control_listen as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
struct ControlClients {
    listener: Option<i32>,
    accepting: bool,
    stopped: bool,
    clients: std::collections::BTreeSet<i32>,
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
static CONTROL_CLIENTS: std::sync::Mutex<ControlClients> = std::sync::Mutex::new(ControlClients {
    listener: None,
    accepting: false,
    stopped: false,
    clients: std::collections::BTreeSet::new(),
});

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn control_clients() -> std::sync::MutexGuard<'static, ControlClients> {
    CONTROL_CLIENTS.lock().unwrap_or_else(|_| unsafe { libc::_exit(89) })
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_accept_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"accept",
        original: libc::accept as *const () as usize,
        replacement: reference_control_accept as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_close_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"close",
        original: libc::close as *const () as usize,
        replacement: reference_control_close as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn reference_control_accept(
    _fd: i32, _address: *mut libc::sockaddr, _length: *mut libc::socklen_t,
) -> i32 {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym reference_control_accept_checked);
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
extern "C" fn reference_control_accept_checked(
    fd: i32, address: *mut libc::sockaddr, length: *mut libc::socklen_t, caller: usize,
) -> i32 {
    let base = unsafe { _NSGetMachExecuteHeader() } as usize;
    if base.checked_add(0xdebe8) != Some(caller) {
        return unsafe { libc::accept(fd, address, length) };
    }
    {
        let mut state = control_clients();
        if state.listener != Some(fd) || state.accepting || !address.is_null() || !length.is_null() {
            unsafe { libc::_exit(89) }
        }
        if state.stopped {
            unsafe { *libc::__error() = libc::ECONNABORTED; }
            return -1;
        }
        state.accepting = true;
    }
    // The original Swift worker keeps the listener alive throughout accept.
    let client = unsafe { libc::accept(fd, address, length) };
    let errno = unsafe { *libc::__error() };
    let mut state = control_clients();
    state.accepting = false;
    if client < 0 {
        unsafe { *libc::__error() = errno; }
        return client;
    }
    if state.stopped {
        if unsafe { libc::close(client) } != 0 {
            unsafe { libc::_exit(89) }
        }
        unsafe { *libc::__error() = libc::ECONNABORTED; }
        return -1;
    }
    // Swift admits at most sixteen workers, plus one accepted descriptor
    // checked against its semaphore before the next accept.
    if client <= 2 || state.clients.len() >= 17
        || unsafe { libc::fcntl(client, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
        || !state.clients.insert(client)
    {
        unsafe { libc::_exit(89) }
    }
    client
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn reference_control_close(_fd: i32) -> i32 {
    core::arch::naked_asm!("mov x1, x30", "b {checked}", checked = sym reference_control_close_checked);
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
extern "C" fn reference_control_close_checked(fd: i32, caller: usize) -> i32 {
    let base = unsafe { _NSGetMachExecuteHeader() } as usize;
    if !matches!(caller.checked_sub(base), Some(0xdee08 | 0xdee14 | 0xdf700)) {
        return unsafe { libc::close(fd) };
    }
    let mut state = control_clients();
    // Serialise the original close with stop's shutdown of this exact fd.
    if !state.clients.contains(&fd) || unsafe { libc::close(fd) } != 0 {
        unsafe { libc::_exit(89) }
    }
    state.clients.remove(&fd);
    0
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_shutdown_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"shutdown",
        original: libc::shutdown as *const () as usize,
        replacement: reference_control_shutdown as *const () as usize,
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn reference_control_shutdown(_fd: i32, _how: i32) -> i32 {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym reference_control_shutdown_checked);
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
extern "C" fn reference_control_shutdown_checked(fd: i32, how: i32, caller: usize) -> i32 {
    let base = unsafe { _NSGetMachExecuteHeader() } as usize;
    if base.checked_add(0xdfcfc) != Some(caller) {
        return unsafe { libc::shutdown(fd, how) };
    }
    let result = std::panic::catch_unwind(|| -> Result<i32> {
        let owner = REFERENCE_LOCK_OWNER.get().ok_or("Reference control shutdown owner absent")?;
        owner.before_reference_initialization()?;
        let path = Path::new(&owner.grant.runtime.profile).join("control.sock");
        let directory = owner.verify_profile_directory()?;
        if how != libc::SHUT_RDWR || verify_reference_socket_name(fd, &path).is_err()
            || REFERENCE_CONTROL_LISTENED.with(std::cell::Cell::get) != Some(fd)
        {
            return Err("Reference control shutdown listener differs".into());
        }
        let before = reference_control_socket_stat(directory.as_raw_fd(), owner.grant.owner_uid)
            .map_err(|_| "Reference control shutdown socket identity unavailable")?;
        if before.st_mode & 0o777 != 0o600
            || REFERENCE_CONTROL_SOCKET_ID.with(std::cell::Cell::get) != Some((fd, before.st_dev, before.st_ino))
        {
            return Err("Reference control shutdown socket identity or mode differs".into());
        }
        {
            let mut clients = control_clients();
            if clients.listener != Some(fd) || clients.stopped {
                return Err("Reference control client shutdown owner differs".into());
            }
            clients.stopped = true;
            for &client in &clients.clients {
                if unsafe { libc::shutdown(client, libc::SHUT_RDWR) } != 0
                    && unsafe { *libc::__error() } != libc::ENOTCONN
                {
                    return Err("Reference control client shutdown failed".into());
                }
            }
        }
        let status = unsafe { libc::shutdown(fd, how) };
        if status == 0 {
            return Ok(0);
        }
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO);
        if errno != libc::ENOTCONN {
            return Err(format!("Reference control shutdown failed with errno {errno}"));
        }
        // Darwin can leave accept blocked on a listening Unix socket. The
        // exact Swift caller has already made the listener inactive, so an
        // empty local connection is dropped before any request is handled.
        let wake = open_reference_control_socket(libc::AF_UNIX, libc::SOCK_STREAM, 0)
            .map_err(|_| "Reference control wake socket unavailable")?;
        if unsafe { libc::fcntl(wake.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } != 0 {
            return Err("Reference control wake nonblocking setup failed".into());
        }
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as _;
        let bytes = path.as_os_str().as_encoded_bytes();
        if bytes.len() >= address.sun_path.len() {
            return Err("Reference control wake path too long".into());
        }
        for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
            *slot = *byte as libc::c_char;
        }
        reference_control_socket_operation(wake.as_raw_fd(), &address, &path, true)
            .map_err(|errno| format!("Reference control wake connect failed with errno {errno}"))?;
        let after = reference_control_socket_stat(directory.as_raw_fd(), owner.grant.owner_uid)
            .map_err(|_| "Reference control wake socket identity unavailable")?;
        if (before.st_dev, before.st_ino, before.st_mode) != (after.st_dev, after.st_ino, after.st_mode) {
            return Err("Reference control socket changed during wake".into());
        }
        drop(wake);
        unsafe { *libc::__error() = errno };
        Ok(status)
    });
    match result {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => unsafe { libc::_exit(89) },
        Err(payload) => {
            std::mem::forget(payload);
            unsafe { libc::_exit(89) }
        }
    }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn reference_socket_spec() -> super::ImportSpec {
    super::ImportSpec {
        symbol: c"socket",
        original: libc::socket as *const () as usize,
        replacement: reference_control_new_socket as *const () as usize,
    }
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn open_reference_control_socket(domain: i32, kind: i32, protocol: i32) -> std::result::Result<OwnedFd, i32> {
    use std::os::fd::FromRawFd;
    if (domain, kind, protocol) != (libc::AF_UNIX, libc::SOCK_STREAM, 0) {
        return Err(libc::EPERM);
    }
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO);
    let fd = unsafe { libc::socket(domain, kind, protocol) };
    if fd < 0 {
        return Err(errno());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    if fd <= 2 {
        return Err(libc::EPERM);
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(errno());
    }
    Ok(socket)
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) extern "C" fn reference_control_new_socket(domain: i32, kind: i32, protocol: i32) -> i32 {
    use std::os::fd::IntoRawFd;
    let result = std::panic::catch_unwind(|| -> std::result::Result<OwnedFd, i32> {
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        owner.before_reference_initialization().map_err(|_| libc::EPERM)?;
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors()).map_err(|_| libc::EPERM)?;
        open_reference_control_socket(domain, kind, protocol)
    });
    let errno = match result {
        Ok(Ok(socket)) => return socket.into_raw_fd(),
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn listen_reference_control_socket(fd: i32, backlog: i32, path: &Path) -> std::result::Result<(), i32> {
    if backlog != 16 {
        return Err(libc::EPERM);
    }
    verify_reference_socket_name(fd, path)?;
    if unsafe { libc::listen(fd, backlog) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO));
    }
    Ok(())
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn verify_reference_socket_name(fd: i32, path: &Path) -> std::result::Result<(), i32> {
    if fd <= 2 {
        return Err(libc::EPERM);
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if unsafe { libc::getsockname(fd, (&mut address as *mut libc::sockaddr_un).cast(), &mut length) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO));
    }
    let bytes = address.sun_path.map(|byte| byte as u8);
    if i32::from(address.sun_family) != libc::AF_UNIX
        || CStr::from_bytes_until_nul(&bytes).map(CStr::to_bytes).ok() != Some(path.as_os_str().as_encoded_bytes())
    {
        return Err(libc::EPERM);
    }
    Ok(())
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) extern "C" fn reference_control_listen(fd: i32, backlog: i32) -> i32 {
    let result = std::panic::catch_unwind(|| -> std::result::Result<(), i32> {
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        owner.before_reference_initialization().map_err(|_| libc::EPERM)?;
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors()).map_err(|_| libc::EPERM)?;
        let directory = owner.verify_profile_directory().map_err(|_| libc::EPERM)?;
        let before = reference_control_socket_stat(directory.as_raw_fd(), owner.grant.owner_uid)?;
        if REFERENCE_CONTROL_LISTENED.with(|slot| slot.get().is_some()) {
            return Err(libc::EPERM);
        }
        listen_reference_control_socket(fd, backlog, &Path::new(&owner.grant.runtime.profile).join("control.sock"))?;
        let after = reference_control_socket_stat(directory.as_raw_fd(), owner.grant.owner_uid)?;
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino) {
            return Err(libc::EPERM);
        }
        {
            let mut clients = control_clients();
            if clients.listener.is_some() {
                return Err(libc::EPERM);
            }
            clients.listener = Some(fd);
        }
        REFERENCE_CONTROL_SOCKET_ID.with(|slot| slot.set(Some((fd, after.st_dev, after.st_ino))));
        REFERENCE_CONTROL_LISTENED.with(|slot| slot.set(Some(fd)));
        Ok(())
    });
    let errno = match result {
        Ok(Ok(())) => return 0,
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn reference_control_socket_stat(directory: i32, uid: libc::uid_t) -> std::result::Result<libc::stat, i32> {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstatat(directory, c"control.sock".as_ptr(), info.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO));
    }
    let info = unsafe { info.assume_init() };
    if info.st_mode & libc::S_IFMT != libc::S_IFSOCK || info.st_uid != uid {
        return Err(libc::EPERM);
    }
    Ok(info)
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn secure_reference_control_socket(directory: i32, uid: libc::uid_t) -> std::result::Result<(), i32> {
    let before = reference_control_socket_stat(directory, uid)?;
    if unsafe { libc::fchmodat(directory, c"control.sock".as_ptr(), 0o600, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO));
    }
    let after = reference_control_socket_stat(directory, uid)?;
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino) || after.st_mode & 0o777 != 0o600 {
        return Err(libc::EPERM);
    }
    Ok(())
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_control_chmod(
    dir: i32, path: *const libc::c_char, mode: libc::mode_t, flags: i32,
) -> i32 {
    let result = std::panic::catch_unwind(|| -> std::result::Result<(), i32> {
        if path.is_null() || dir != libc::AT_FDCWD || mode != 0o600 || flags != libc::AT_SYMLINK_NOFOLLOW {
            return Err(libc::EPERM);
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        owner.before_reference_initialization().map_err(|_| libc::EPERM)?;
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors()).map_err(|_| libc::EPERM)?;
        let socket = Path::new(&owner.grant.runtime.profile).join("control.sock");
        if unsafe { CStr::from_ptr(path) }.to_bytes() != socket.as_os_str().as_encoded_bytes() {
            return Err(libc::EPERM);
        }
        let directory = owner.verify_profile_directory().map_err(|_| libc::EPERM)?;
        secure_reference_control_socket(directory.as_raw_fd(), owner.grant.owner_uid)
    });
    let errno = match result {
        Ok(Ok(())) => return 0,
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
fn reference_control_socket_operation(fd: i32, address: &libc::sockaddr_un, path: &Path, connect: bool) -> std::result::Result<(), i32> {
    use std::os::unix::ffi::OsStrExt;
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO);
    let bytes = address.sun_path.map(|value| value as u8);
    let expected = path.as_os_str().as_bytes();
    if fd <= 2 || i32::from(address.sun_family) != libc::AF_UNIX || address.sun_len != 0
        || expected.len() >= bytes.len() || !bytes.starts_with(expected)
        || bytes[expected.len()..].iter().any(|&byte| byte != 0)
    {
        return Err(libc::EPERM);
    }
    let mut kind = 0i32;
    let mut kind_size = size_of::<i32>() as libc::socklen_t;
    if unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_TYPE, (&mut kind as *mut i32).cast(), &mut kind_size) } != 0 {
        return Err(errno());
    }
    if kind != libc::SOCK_STREAM || kind_size as usize != size_of::<i32>() {
        return Err(libc::EPERM);
    }
    let length = size_of::<libc::sockaddr_un>() as libc::socklen_t;
    let mut local: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut local_size = length;
    if unsafe { libc::getsockname(fd, (&mut local as *mut libc::sockaddr_un).cast(), &mut local_size) } != 0 {
        return Err(errno());
    }
    if i32::from(local.sun_family) != libc::AF_UNIX || local.sun_path[0] != 0 {
        return Err(libc::EPERM);
    }
    let address = (address as *const libc::sockaddr_un).cast();
    let status = if connect {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(errno());
        }
        if flags & libc::O_NONBLOCK == 0 {
            return Err(libc::EPERM);
        }
        unsafe { libc::connect(fd, address, length) }
    } else {
        unsafe { libc::bind(fd, address, length) }
    };
    if status != 0 {
        return Err(errno());
    }
    Ok(())
}

#[cfg(test)]
#[test]
#[ignore = "requires a disposable private fixture parent"]
fn reference_control_socket_accepts_only_the_private_unix_stream_address() {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let directory = std::path::PathBuf::from(std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT").unwrap());
    let path = directory.join("control.sock");
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dst, &src) in address.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
        *dst = src as libc::c_char;
    }
    let socket = |kind| {
        let fd = unsafe { libc::socket(libc::AF_UNIX, kind, 0) };
        assert!(fd > 2);
        unsafe { OwnedFd::from_raw_fd(fd) }
    };
    assert_eq!(open_reference_control_socket(libc::AF_INET, libc::SOCK_STREAM, 0).err(), Some(libc::EPERM));
    assert_eq!(open_reference_control_socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0).err(), Some(libc::EPERM));
    let stream = open_reference_control_socket(libc::AF_UNIX, libc::SOCK_STREAM, 0).unwrap();
    assert_eq!(unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) }, libc::FD_CLOEXEC);
    let datagram = socket(libc::SOCK_DGRAM);
    assert_eq!(reference_control_socket_operation(stream.as_raw_fd(), &address, &directory.join("foreign.sock"), false), Err(libc::EPERM));
    assert_eq!(reference_control_socket_operation(datagram.as_raw_fd(), &address, &path, false), Err(libc::EPERM));
    assert!(!path.exists());
    reference_control_socket_operation(stream.as_raw_fd(), &address, &path, false).unwrap();
    assert_eq!(reference_control_socket_operation(stream.as_raw_fd(), &address, &path, false), Err(libc::EPERM));
    let probe = socket(libc::SOCK_STREAM);
    assert_eq!(reference_control_socket_operation(probe.as_raw_fd(), &address, &path, true), Err(libc::EPERM));
    assert_eq!(unsafe { libc::fcntl(probe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) }, 0);
    assert_eq!(reference_control_socket_operation(probe.as_raw_fd(), &address, &path, true), Err(libc::ECONNREFUSED));
    assert_eq!(listen_reference_control_socket(stream.as_raw_fd(), 128, &path), Err(libc::EPERM));
    assert_eq!(listen_reference_control_socket(stream.as_raw_fd(), 16, &directory.join("foreign.sock")), Err(libc::EPERM));
    listen_reference_control_socket(stream.as_raw_fd(), 16, &path).unwrap();
    reference_control_socket_operation(probe.as_raw_fd(), &address, &path, true).unwrap();
    let name = std::ffi::CString::new(directory.as_os_str().as_bytes()).unwrap();
    let fd = unsafe { libc::open(name.as_ptr(), libc::O_EVTONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    assert!(fd > 2);
    let dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let uid = unsafe { libc::getuid() };
    secure_reference_control_socket(dir.as_raw_fd(), uid).unwrap();
    assert_eq!(reference_control_socket_stat(dir.as_raw_fd(), uid).unwrap().st_mode & 0o777, 0o600);
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(directory.join("missing"), &path).unwrap();
    assert_eq!(secure_reference_control_socket(dir.as_raw_fd(), uid), Err(libc::EPERM));
    std::fs::remove_file(path).unwrap();
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_control_bind(fd: i32, address: *const libc::sockaddr, length: libc::socklen_t) -> i32 {
    unsafe { reference_control_socket(fd, address, length, false) }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_control_connect(fd: i32, address: *const libc::sockaddr, length: libc::socklen_t) -> i32 {
    unsafe { reference_control_socket(fd, address, length, true) }
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
unsafe fn reference_control_socket(fd: i32, address: *const libc::sockaddr, length: libc::socklen_t, connect: bool) -> i32 {
    let result = std::panic::catch_unwind(|| -> std::result::Result<(), i32> {
        if address.is_null() || length as usize != size_of::<libc::sockaddr_un>() {
            return Err(libc::EINVAL);
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        owner.before_reference_initialization().map_err(|_| libc::EPERM)?;
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors()).map_err(|_| libc::EPERM)?;
        let profile = Path::new(&owner.grant.runtime.profile);
        let socket = profile.join("control.sock");
        let directory = owner.verify_profile_directory().map_err(|_| libc::EPERM)?;
        let address = unsafe { address.cast::<libc::sockaddr_un>().read_unaligned() };
        if connect {
            reference_control_socket_stat(directory.as_raw_fd(), owner.grant.owner_uid)?;
        }
        reference_control_socket_operation(fd, &address, &socket, connect)
    });
    let errno = match result {
        Ok(Ok(())) => return 0,
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_file_unlink(path: *const libc::c_char) -> i32 {
    use std::os::unix::ffi::OsStrExt;
    let result = std::panic::catch_unwind(|| -> std::result::Result<(), i32> {
        if path.is_null() {
            return Err(libc::EFAULT);
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or(libc::EPERM)?;
        let path = Path::new(std::ffi::OsStr::from_bytes(unsafe { CStr::from_ptr(path) }.to_bytes()));
        let root = reference_file_root(owner, path).map_err(|_| libc::EPERM)?;
        let socket = Path::new(&owner.grant.runtime.profile).join("control.sock");
        let socket_owner = (path == socket).then_some(owner.grant.owner_uid);
        owner.admitted.life.before_effect().map_err(|_| libc::EPERM)?;
        owner.registry_current().map_err(|_| libc::EPERM)?;
        owner.verify_profile_directory().map_err(|_| libc::EPERM)?;
        root.unlink_reference_leaf(path, socket_owner)
    });
    let errno = match result {
        Ok(Ok(())) => return 0,
        Ok(Err(errno)) => errno,
        Err(payload) => {
            std::mem::forget(payload);
            libc::EPERM
        }
    };
    unsafe { *libc::__error() = errno };
    -1
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
thread_local! {
    // The original Swift source owns and closes this FD. Keep only its number
    // on the creator thread and fail conservatively if the number is reused.
    static REFERENCE_WATCH_FD: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    static REFERENCE_CONFIG_READ: std::cell::Cell<Option<(i32, libc::dev_t, libc::ino_t)>> = const { std::cell::Cell::new(None) };
    static REFERENCE_CONTROL_LISTENED: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    static REFERENCE_CONTROL_SOCKET_ID: std::cell::Cell<Option<(i32, libc::dev_t, libc::ino_t)>> = const { std::cell::Cell::new(None) };
    static REFERENCE_CONTROL_LOCK: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn reference_profile_read_open(_path: *const libc::c_char, _flags: i32) -> i32 {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym reference_profile_read_open_checked);
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
unsafe extern "C" fn reference_profile_read_open_checked(path: *const libc::c_char, flags: i32, caller: usize) -> i32 {
    use std::os::{fd::IntoRawFd, unix::ffi::OsStrExt};
    let main = unsafe { _NSGetMachExecuteHeader() } as usize;
    let call = caller.checked_sub(main);
    let config_read = call == Some(0x80558);
    let watcher_open = call == Some(0x7f99c);
    let command_log_read = call == Some(0xd25d8);
    if config_read {
        REFERENCE_CONFIG_READ.with(|slot| slot.set(None));
    }
    let result = std::panic::catch_unwind(|| -> Result<OwnedFd> {
        if path.is_null() {
            return Err("Reference file open path absent".into());
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or("Reference file open owner absent")?;
        let on_main = unsafe { libc::pthread_main_np() } == 1;
        let path = Path::new(std::ffi::OsStr::from_bytes(unsafe { CStr::from_ptr(path) }.to_bytes()));
        let profile = Path::new(&owner.grant.runtime.profile);
        let preview = || path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|ext|
            ["png", "jpg", "jpeg", "gif", "tiff", "tif", "heic", "webp", "bmp"]
                .iter().any(|allowed| ext.eq_ignore_ascii_case(allowed)));
        const WATCH: i32 = libc::O_EVTONLY | libc::O_CLOEXEC;
        const READ: i32 = libc::O_NOFOLLOW | libc::O_CLOEXEC;
        const READ_NONBLOCK: i32 = READ | libc::O_NONBLOCK;
        const DIRECTORY: i32 = libc::O_DIRECTORY | libc::O_CLOEXEC;
        const DIRECTORY_NOFOLLOW: i32 = DIRECTORY | libc::O_NOFOLLOW;
        if config_read && (on_main || flags != READ_NONBLOCK || path != profile.join("config.json")) {
            return Err("Reference observer config open caller/path/flags differ".into());
        }
        if watcher_open && (!on_main || flags != WATCH || path != profile) {
            return Err("Reference watcher open caller/path/flags differ".into());
        }
        if command_log_read && (flags != READ_NONBLOCK || path != profile.join("command-log.json")) {
            return Err("Reference command log open caller/path/flags differ".into());
        }
        let source = match call {
            Some(0x7f99c) => Some((0x7f988, 24, "aee2487b1b158a8d6dc6b4e21fcf35446ee77dc3d120f941a2eedc8065a009a6")),
            Some(0x80558) => Some((0x80544, 24, "f2bdb74e6ba12872a923f90c3d5878935c8ddde224bf137f7d311dd05dd1c61d")),
            Some(0xd25d8) => Some((0xd25c4, 20, "9280a579b84a8d4b34ab1b2a6ce9ed31726c601d73b467c64c4ad2c6d5205845")),
            Some(0x209364) => Some((0x20934c, 24, "ec9615625df1b330e71956717afa18cf02856cef14aba2d71fa4e6fe226877c0")),
            _ => None,
        };
        if let Some((offset, length, expected)) = source {
            let start = main.checked_add(offset).ok_or("Reference profile open code overflow")?;
            reference_mapping(start, length, Some(true))?;
            if reference_image_header(start, &owner.endpoint)? != main
                || digest(unsafe { std::slice::from_raw_parts(start as *const u8, length) }) != expected
            {
                return Err("Reference profile open caller bytes differ".into());
            }
        }
        let root = if call == Some(0x209364) && flags == DIRECTORY
            && path == owner.roots[1].path().join("website-assembly")
            && super::private_network::reference_website_assembly_active()
        {
            super::private_network::qualify_reference_website_cache_directory(owner)?;
            owner.roots[1].clone()
        } else {
            reference_file_root(owner, path)?
        };
        let file = match flags {
            WATCH if watcher_open && on_main && path == profile => owner.verify_profile_directory()?,
            READ_NONBLOCK if config_read || command_log_read =>
                root.checked(path)?,
            READ if path == profile.join("analytics.json") || preview() => root.checked(path)?,
            DIRECTORY_NOFOLLOW if path == profile || path == profile.join("clipboard-assets") =>
                root.checked(path)?,
            DIRECTORY => root.checked(path)?,
            _ => return Err("Reference file open path/flags differ".into()),
        };
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err("Reference file open metadata unavailable".into());
        }
        let stat = unsafe { stat.assume_init() };
        let kind = if matches!(flags, WATCH | DIRECTORY | DIRECTORY_NOFOLLOW) { libc::S_IFDIR } else { libc::S_IFREG };
        if stat.st_mode & libc::S_IFMT != kind || stat.st_uid != owner.grant.owner_uid {
            return Err("Reference file open type or owner differs".into());
        }
        if matches!(flags, READ | DIRECTORY | DIRECTORY_NOFOLLOW)
            && unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, 0) } < 0
        {
            return Err("Reference file open status flags could not be restored".into());
        }
        owner.admitted.life.before_effect()?;
        owner.registry_current()?;
        owner.verify_profile_directory()?;
        if flags == WATCH {
            REFERENCE_WATCH_FD.with(|slot| {
                if slot.get().is_some() {
                    return Err("Reference watcher descriptor replay");
                }
                slot.set(Some(file.as_raw_fd()));
                Ok(())
            })?;
        }
        if config_read {
            REFERENCE_CONFIG_READ.with(|slot| slot.set(Some((file.as_raw_fd(), stat.st_dev, stat.st_ino))));
        }
        Ok(file)
    });
    match result {
        Ok(Ok(file)) => file.into_raw_fd(),
        Ok(Err(_)) => {
            unsafe { *libc::__error() = libc::EPERM };
            -1
        }
        Err(payload) => {
            std::mem::forget(payload);
            unsafe { *libc::__error() = libc::EPERM };
            -1
        }
    }
}

// The fixed Swift Darwin overlay signature uses three integer/pointer
// registers on arm64. It is separate from the variadic libc open import.
#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
pub(super) unsafe extern "C" fn reference_profile_lock_open(path: *const libc::c_char, flags: i32, mode: u16) -> i32 {
    use std::os::fd::{FromRawFd, IntoRawFd};
    let result = std::panic::catch_unwind(|| -> Result<OwnedFd> {
        const CONFIG_FLAGS: i32 = libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        const INSTANCE_FLAGS: i32 = CONFIG_FLAGS | libc::O_NONBLOCK;
        let leaf = match (flags, mode) {
            (CONFIG_FLAGS, 0o600) => c"config.lock",
            (INSTANCE_FLAGS, 0o600) => c"instance.lock",
            _ => return Err("Reference profile lock flags differ".into()),
        };
        if path.is_null() {
            return Err("Reference profile lock path absent".into());
        }
        let owner = REFERENCE_LOCK_OWNER.get().ok_or("Reference profile lock owner absent")?;
        owner.before_reference_initialization()?;
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors())?;
        let profile = Path::new(&owner.grant.runtime.profile);
        let expected = profile.join(leaf.to_str().expect("fixed ASCII lock leaf"));
        if unsafe { CStr::from_ptr(path) }.to_bytes() != expected.as_os_str().as_encoded_bytes() {
            return Err("Reference profile lock path differs".into());
        }
        let directory = owner.verify_profile_directory()?;
        let fd = unsafe { libc::openat(directory.as_raw_fd(), leaf.as_ptr(), flags | libc::O_NONBLOCK, 0o600) };
        if fd < 0 {
            return Err("Reference profile lock open failed".into());
        }
        let file = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), info.as_mut_ptr()) } != 0 {
            return Err("Reference profile lock metadata unavailable".into());
        }
        let info = unsafe { info.assume_init() };
        if info.st_mode & libc::S_IFMT != libc::S_IFREG || info.st_uid != owner.grant.owner_uid
            || info.st_nlink != 1 || info.st_mode & 0o077 != 0
        {
            return Err("Reference profile lock is not an owned private regular file".into());
        }
        if flags == CONFIG_FLAGS && unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, libc::O_RDWR) } < 0 {
            return Err("Reference profile lock access flags could not be restored".into());
        }
        owner.admitted.life.before_effect()?;
        owner.registry_current()?;
        owner.verify_profile_directory()?;
        Ok(file)
    });
    match result {
        Ok(Ok(file)) => file.into_raw_fd(),
        Ok(Err(_)) => {
            unsafe { *libc::__error() = libc::EPERM };
            -1
        }
        Err(payload) => {
            std::mem::forget(payload);
            unsafe { *libc::__error() = libc::EPERM };
            -1
        }
    }
}

#[cfg(rcf1_reference_sdk)]
fn reference_private_startup_profile(home: &OwnedRoot, profile: &Path, config_sha256: &str) -> Result<()> {
    // The guarded producer seeds only config.json. AppState and its stores
    // must create all other state themselves inside this private directory.
    let entries = home.list(profile)?;
    if entries.len() != 1 || entries[0].name != "config.json" || entries[0].directory {
        return Err("Reference startup requires a fresh profile containing only config.json".into());
    }
    checked_hash(home, &profile.join("config.json"), config_sha256)
}

#[cfg(any(test, rcf1_reference_sdk))]
fn verify_reference_control_leaves(profile: &OwnedFd, uid: u32) -> Result<()> {
    for (name, kind) in [(c"instance.lock", libc::S_IFREG), (c"control.sock", libc::S_IFSOCK)] {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(profile.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW)
        } != 0 {
            return Err(format!("Reference control leaf {name:?} is absent or unreadable"));
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_uid != uid || stat.st_nlink != 1 || stat.st_mode & libc::S_IFMT != kind
            || stat.st_mode & 0o777 != 0o600
        {
            return Err(format!("Reference control leaf {name:?} has unexpected owner, type or mode"));
        }
    }
    Ok(())
}
/// Actual retained Reference construction context. This is not the seven-field
/// initializer/ARC proof and cannot mark an allocation initialized. The common
/// class-local issuer must still supply every typed field/code/return/cleanup
/// conjunct before it can use this context for an original constructor.
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum SwiftSingletonDataKind {EmptySet,EmptyArray}
/// A fresh data observation, not an issuer or an object/ARC capability.
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct SwiftSingletonRegionObservation {
    uuid:[u8;16], image_offset:usize, address:usize, kind:SwiftSingletonDataKind,
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
impl SwiftSingletonRegionObservation {
    pub fn uuid(&self)->[u8;16] {self.uuid}
    pub fn image_offset(&self)->usize {self.image_offset}
    pub fn address(&self)->usize {self.address}
    pub fn kind(&self)->SwiftSingletonDataKind {self.kind}
}
/// Only fixed exported storage/address-point variants. The Native witness
/// independently binds the actual finite metadata-accessor return to this DATA.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum FoundationNotificationAddressPoint {Symbol,AfterValueWitness}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[link(name="Foundation",kind="framework")]
unsafe extern "C" {
    #[link_name="$s10Foundation12NotificationVN"]
    static FOUNDATION_NOTIFICATION_VN:u8;
    #[link_name="$s10Foundation12NotificationVMn"]
    static FOUNDATION_NOTIFICATION_MN:u8;
}
/// Fixed named metadata/table observation. Neither layout presence nor a
/// function address grants Complete state, Swift ABI, invocation or ARC rights.
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct FoundationNotificationMetadataObservation {
    symbol_address:usize,metadata_address:usize,descriptor_address:usize,
    image_header:usize,uuid:[u8;16],metadata_words:[u64;3],
    table_address:usize,table:[u8;88],destroy:super::metadata::CodeIdentity,
}
#[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
impl FoundationNotificationMetadataObservation {
    pub fn symbol_address(&self)->usize {self.symbol_address}
    pub fn metadata_address(&self)->usize {self.metadata_address}
    pub fn descriptor_address(&self)->usize {self.descriptor_address}
    pub fn image_header(&self)->usize {self.image_header}
    pub fn uuid(&self)->[u8;16] {self.uuid}
    pub fn metadata_words(&self)->[u64;3] {self.metadata_words}
    pub fn table_address(&self)->usize {self.table_address}
    pub fn table(&self)->&[u8;88] {&self.table}
    pub fn destroy_address(&self)->usize {u64::from_le_bytes(self.table[8..16].try_into().expect("fixed destroy word")) as usize}
    pub fn size(&self)->u64 {u64::from_le_bytes(self.table[64..72].try_into().expect("fixed size word"))}
    pub fn stride(&self)->u64 {u64::from_le_bytes(self.table[72..80].try_into().expect("fixed stride word"))}
    pub fn flags(&self)->u32 {u32::from_le_bytes(self.table[80..84].try_into().expect("fixed flags word"))}
    pub fn extra_inhabitants(&self)->u32 {u32::from_le_bytes(self.table[84..88].try_into().expect("fixed extra word"))}
    pub fn destroy_identity(&self)->&super::metadata::CodeIdentity {&self.destroy}
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
pub struct ReferenceAppDelegateContext {
    admission:Arc<ReferenceAdmission>,
    model:Arc<crate::workspace::Workspace>,
    constructor_source:super::SourceCallsite,
    callback_source:super::SourceCallsite,
    process:libc::pid_t,
    owner:std::thread::ThreadId,
    code_retired:std::sync::atomic::AtomicBool,
    callback_sources:CallbackSourceObservation,
}
/// One synchronous, effect-free callback-code observation. Phase1 always
/// performs complete source checks; only phase2 may reuse those disk hashes.
/// No result or reusable observation token escapes this fixed scope.
#[cfg(any(test,rcf1_reference_sdk))]
#[derive(Default)]
struct CallbackSourceObservation { phase:std::sync::atomic::AtomicU8 }
#[cfg(any(test,rcf1_reference_sdk))]
impl CallbackSourceObservation {
    fn reusing(&self)->bool {self.phase.load(std::sync::atomic::Ordering::Acquire)==2}
    fn run(&self,retired:&std::sync::atomic::AtomicBool,
        verify:impl Fn()->Result<()>,observe:impl FnOnce()->Result<()>)->Result<()> {
        use std::sync::atomic::Ordering;
        if retired.load(Ordering::Acquire){return Err("Callback source observation retired".into());}
        if self.phase.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire).is_err() {
            retired.store(true,Ordering::Release);
            return Err("Callback source observation nested or reentered".into());
        }
        struct Guard<'a>{scope:&'a CallbackSourceObservation,retired:&'a std::sync::atomic::AtomicBool,complete:bool}
        impl Drop for Guard<'_>{fn drop(&mut self){
            self.scope.phase.store(0,Ordering::Release);
            if !self.complete {self.retired.store(true,Ordering::Release);}
        }}
        let mut guard=Guard{scope:self,retired,complete:false};
        verify()?;
        self.phase.store(2,Ordering::Release);
        observe()?;
        self.phase.store(1,Ordering::Release);
        verify()?;
        if retired.load(Ordering::Acquire){return Err("Callback source observation retired before completion".into());}
        guard.complete=true;Ok(())
    }
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct AppDelegateOffsetCellsObservation {main_header:usize,cells:[(usize,u64);7],init_selector_cell:(usize,usize)}
#[cfg(all(rcf1_reference_sdk,not(test)))]
impl AppDelegateOffsetCellsObservation {
    pub fn main_header(&self)->usize {self.main_header}
    pub fn cells(&self)->&[(usize,u64);7] {&self.cells}
    pub fn init_selector_cell(&self)->(usize,usize) {self.init_selector_cell}
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct AppStateChildOffsetCellsObservation {main_header:usize,cells:[(usize,u64);2]}
/// Observed only after the genuine state getter returned an initialized object.
/// This connects its actual completed class metadata to the native destructor;
/// it is not field/VWT, Swift-call, final-release or async-drain permission.
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct AppStateDestroyerObservation {
    class_metadata:usize,destroyer:usize,instance_size:u32,alignment_mask:u16,
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[path="workspace_compiler_text.rs"]
mod compiler_text;
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[link(name="Combine",kind="framework")]
unsafe extern "C" {
    #[link_name="$s7Combine14AnyCancellableCMn"] static SUBS_ANY_DESCRIPTOR:u8;
    #[link_name="$s7Combine14AnyCancellableCSHAAMc"] static SUBS_HASHABLE:u8;
    #[link_name="$s7Combine14AnyCancellableCSQAAMc"] static SUBS_EQUATABLE:u8;
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
#[link(name="swiftCore")]
unsafe extern "C" {#[link_name="$ss11_SetStorageCMn"] static SUBS_SET_DESCRIPTOR:u8;}
#[cfg(all(rcf1_reference_sdk,not(test)))]
impl AppStateDestroyerObservation {
    pub fn class_metadata(&self)->usize {self.class_metadata}
    pub fn destroyer(&self)->usize {self.destroyer}
    pub fn instance_size(&self)->usize {self.instance_size as usize}
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
impl AppStateChildOffsetCellsObservation {
    pub fn main_header(&self)->usize {self.main_header}
    pub fn cells(&self)->&[(usize,u64);2] {&self.cells}
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
impl ReferenceAppDelegateContext {
    /// Fixed compiler type graph only. Rust never reads a Set value, its
    /// storage header/elements or Swift reference counts. Cancellation/drain
    /// remain operational requirements, not products of this observation.
    pub(crate) fn verify_subscriptions_compiler_type(&self)->Result<()> {
        self.check_owner()?;
        let result=(|| {
            self.cleanup_metadata_only()?;
            let base=self.observe_app_delegate_offset_cells()?.main_header();
            let at=|offset:usize|base.checked_add(offset).ok_or("Subscriptions type address overflow");
            let combine=b"/System/Library/Frameworks/Combine.framework/Versions/A/Combine".as_slice();
            let core=b"/usr/lib/swift/libswiftCore.dylib".as_slice();
            let descriptors=[
                (0x3c4dc8usize,std::ptr::addr_of!(SUBS_ANY_DESCRIPTOR) as usize,c"$s7Combine14AnyCancellableCMn",combine),
                (0x3c4dd0,std::ptr::addr_of!(SUBS_HASHABLE) as usize,c"$s7Combine14AnyCancellableCSHAAMc",combine),
                (0x3c4dd8,std::ptr::addr_of!(SUBS_EQUATABLE) as usize,c"$s7Combine14AnyCancellableCSQAAMc",combine),
                (0x3c6e18,std::ptr::addr_of!(SUBS_SET_DESCRIPTOR) as usize,c"$ss11_SetStorageCMn",core),
            ];
            let set=b"Shy\x02\x0e\x63\x06\0G\0".as_slice();
            let storage=b"\x02\x41\x83\x06\0y\x02\xeb\x62\x06\0G\0".as_slice();
            let record=b"\x66\x4b\x05\0\x0c\0\0\0\0\0\0\0\0\0\0\0".as_slice();
            let check=||->Result<()> {
                reference_mapping_infrastructure()?;
                for (offset,expected) in [(0x35eab6usize,set),(0x35ead6,storage),(0x309f70,record)] {
                    let address=at(offset)?;reference_mapping(address,expected.len(),None)?;
                    if unsafe{std::slice::from_raw_parts(address as *const u8,expected.len())}!=expected {
                        return Err("Original Set<AnyCancellable> compiler type reference differs".into());
                    }
                }
                for (offset,address,symbol,provider) in descriptors {
                    reference_mapping(at(offset)?,8,None)?;
                    reference_mapping(address,1,None)?;
                    let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
                    if unsafe{libc::dladdr(address as *const libc::c_void,info.as_mut_ptr())}==0{return Err("Subscriptions descriptor provider absent".into());}
                    let info=unsafe{info.assume_init()};
                    if info.dli_fname.is_null()||info.dli_sname.is_null()||info.dli_saddr as usize!=address
                        ||unsafe{CStr::from_ptr(info.dli_fname)}.to_bytes()!=provider
                        ||unsafe{CStr::from_ptr(info.dli_sname)}!=symbol
                        ||unsafe{(at(offset)? as *const usize).read_volatile()}!=address {
                        return Err("Subscriptions compiler descriptor/conformance import differs".into());
                    }
                }
                self.check_owner()
            };
            check()?;self.cleanup_metadata_only()?;check()
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}result
    }
    /// Whole compiler-owned native text provenance for the FIXED original
    /// class/entry contracts. No address in this range becomes callable merely
    /// by passing this check; ABI, imports, metadata and custody stay separate.
    pub(crate) fn verify_original_compiler_text(&self)->Result<()> {
        self.check_owner()?;
        let result=(|| {
            self.cleanup_metadata_only()?;
            let file=self.admission.root.read_optional(&self.admission.endpoint,256*1024*1024)?
                .ok_or("Original compiler executable absent")?;
            if digest(&file)!=self.admission.grant.endpoint_sha256 {
                return Err("Original compiler executable changed".into());
            }
            let disk=compiler_text::parse(&file)?;
            let header=unsafe{_NSGetMachExecuteHeader()} as usize;
            reference_mapping_infrastructure()?;
            reference_mapping(header,32,None)?;
            let command_bytes=unsafe{((header+20) as *const u32).read_unaligned()} as usize;
            if command_bytes>16*1024*1024{return Err("Loaded compiler command bytes exceed bound".into());}
            let header_bytes=32usize.checked_add(command_bytes).ok_or("Loaded compiler header overflow")?;
            reference_mapping(header,header_bytes,None)?;
            let loaded=compiler_text::parse(unsafe{std::slice::from_raw_parts(header as *const u8,header_bytes)})?;
            if loaded!=disk{return Err("Loaded compiler text layout/UUID differs from artifact".into());}
            let offset=usize::try_from(disk.address.checked_sub(disk.text_vm)
                .ok_or("Compiler text virtual offset underflow")?).map_err(|_|"Compiler text offset width")?;
            let address=header.checked_add(offset).ok_or("Loaded compiler text address overflow")?;
            let bytes=usize::try_from(disk.bytes).map_err(|_|"Compiler text byte width")?;
            let begin=usize::try_from(disk.file).map_err(|_|"Compiler text file offset width")?;
            let end=begin.checked_add(bytes).ok_or("Compiler text file range overflow")?;
            let expected=file.get(begin..end).ok_or("Compiler text outside exact executable")?;
            let last=address.checked_add(bytes-1).ok_or("Compiler text last address overflow")?;
            let check=||->Result<()> {
                reference_mapping(address,bytes,Some(true))?;
                for entry in [address,last] {
                    if reference_image_header(entry,&self.admission.endpoint)?!=header
                        ||super::metadata::code_identity(entry)?.uuid()!=disk.uuid {
                        return Err("Compiler text has another loaded image/UUID".into());
                    }
                }
                self.check_owner()
            };
            check()?;
            if unsafe{std::slice::from_raw_parts(address as *const u8,bytes)}!=expected {
                return Err("Loaded original compiler text differs from exact artifact".into());
            }
            check()?;
            // Recheck after dynamic mapping/image observations. Stubs, import
            // cells and relocated data deliberately remain separate contracts.
            if unsafe{std::slice::from_raw_parts(address as *const u8,bytes)}!=expected {
                return Err("Original compiler text changed during binding".into());
            }
            self.cleanup_metadata_only()
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}result
    }
    /// Fixed compiler-owned ControlServer/SocketListener class prefixes only.
    /// A lazy cache is not an initialized object or an aggregate-drain proof.
    pub(crate) fn verify_control_server_compiler_metadata(&self)->Result<()> {
        self.check_owner()?;
        let result=(|| {
            self.cleanup_metadata_only()?;
            let manifest=self.admission.root.read_optional(Path::new(&self.admission.grant.source_manifest.path),2*1024*1024)?
                .ok_or("ControlServer compiler source manifest absent")?;
            if digest(&manifest)!=self.admission.grant.source_manifest.sha256 {
                return Err("ControlServer compiler source manifest changed".into());
            }
            let manifest:HostSourceManifest=serde_json::from_slice(&manifest).map_err(|_|"ControlServer compiler source schema differs")?;
            let path=self.admission.root.path().join("bench/reference/0.8.26/Sources/RuycastApp/ControlServer.swift");
            let path=path.to_str().ok_or("ControlServer source path encoding")?;
            let mut rows=manifest.sources.iter().filter(|row|row.path==path);
            if rows.next().is_none_or(|row|row.sha256!="467f60726fdb85caeeb4cb09ce2e94c935634bbd6997890e349c6bd3bba86661")||rows.next().is_some() {
                return Err("ControlServer is not the exact compiler-bound source member".into());
            }
            let base=self.observe_app_delegate_offset_cells()?.main_header();
            let at=|offset:usize|base.checked_add(offset).ok_or("ControlServer compiler metadata address overflow");
            let check=||->Result<()> {
                reference_mapping_infrastructure()?;
                let task_descriptors=at(0x301d70)?;
                reference_mapping(task_descriptors,16,None)?;
                // Fixed relative async function descriptors for the compiler's
                // MainActor back-deployment path, not caller-supplied closures.
                if unsafe{std::slice::from_raw_parts(task_descriptors as *const u8,16)}
                    !=[0xb8,0x83,0xda,0xff,0x20,0,0,0,0x84,0x84,0xda,0xff,0x20,0,0,0] {
                    return Err("ControlServer compiler back-deployment task descriptors differ".into());
                }
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                for (offset,bytes) in [(0x3ef5b0,88usize),(0x3ef670,88),(0x3fe868,8)] {
                    let start=at(offset)?;let end=start.checked_add(bytes).ok_or("ControlServer metadata range overflow")?;
                    let mut depth=0u32;let mut found=false;
                    for _ in 0..16 {
                        let (mut region,mut len,mut info,mut words)=(start as u64,0u64,[0i32;12],12u32);
                        if unsafe{mach_vm_region_recurse(mach_task_self_,&mut region,&mut len,&mut depth,info.as_mut_ptr(),&mut words)}!=0
                            ||words!=12||region>start as u64||region.checked_add(len).is_none_or(|last|last<end as u64) {
                            return Err("ControlServer complete metadata mapping absent".into());
                        }
                        if info[8]!=0 {depth=depth.checked_add(1).ok_or("ControlServer metadata submap overflow")?;continue;}
                        if info[0]&1==0||info[0]&4!=0{return Err("ControlServer metadata is not readable nonexecuting data".into());}
                        found=true;break;
                    }
                    if !found{return Err("ControlServer metadata submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                    return Err("ControlServer mapping identity changed".into());
                }
                self.check_owner()
            };
            let read=||->Result<[usize;5]> {Ok([
                unsafe{(at(0x3ef5b0)? as *const usize).read_volatile()},
                unsafe{(at(0x3ef600)? as *const usize).read_volatile()},
                unsafe{(at(0x3fe868)? as *const usize).read_volatile()},
                unsafe{(at(0x3ef670)? as *const usize).read_volatile()},
                unsafe{(at(0x3ef6c0)? as *const usize).read_volatile()},
            ])};
            check()?;let before=read()?;self.cleanup_metadata_only()?;check()?;let after=read()?;
            if before!=after||after[0]!=at(0xdfe60)?||after[1]!=at(0x37e278)?
                ||(after[2]!=0&&after[2]!=at(0x3ef5c0)?)
                ||after[3]!=at(0xe01e4)?||after[4]!=at(0x37e2c8)? {
                return Err("ControlServer/SocketListener compiler destruction metadata differs".into());
            }
            self.check_owner()
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}result
    }
    /// The finite original controller cohort and its timer-owning model. This
    /// binds compiler memory destruction, never hide/stop/monitor retirement.
    pub(crate) fn verify_controller_compiler_metadata(&self)->Result<[usize;6]> {
        self.check_owner()?;
        let result=(|| {
            self.cleanup_metadata_only()?;
            let manifest=self.admission.root.read_optional(Path::new(&self.admission.grant.source_manifest.path),2*1024*1024)?
                .ok_or("Controller compiler source manifest absent")?;
            if digest(&manifest)!=self.admission.grant.source_manifest.sha256{return Err("Controller source manifest changed".into());}
            let manifest:HostSourceManifest=serde_json::from_slice(&manifest).map_err(|_|"Controller source manifest schema differs")?;
            for (relative,sha) in [
                ("Palette.swift","b3033c6bd07801c067f8fc524be61e81aa313e28243d9b926e74f042d98dc8b5"),
                ("Management.swift","3057068c7ff081e4f551f46690fb5f29f5e3b1d7f63b55d5667fcdfbebf2d42f"),
                ("SystemMonitor.swift","5a06e9fe1762b1eed1c83ac23026208f8141a2fa12fbac8bbf747f35e7a5a8bb"),
                ("MainRunLoop.swift","4d40b6705099184c26d56d8a03d024a010751d715e0ff34a316a011c87e550ba"),
            ] {
                let path=self.admission.root.path().join("bench/reference/0.8.26/Sources/RuycastApp").join(relative);
                let path=path.to_str().ok_or("Controller source path encoding")?;
                let mut rows=manifest.sources.iter().filter(|row|row.path==path);
                if rows.next().is_none_or(|row|row.sha256!=sha)||rows.next().is_some(){return Err("Controller source is not the exact compiled member".into());}
            }
            let base=self.observe_app_delegate_offset_cells()?.main_header();
            let at=|offset:usize|base.checked_add(offset).ok_or("Controller metadata address overflow");
            // Full compiler-native metadata prefixes, NOT file fixup words.
            // Native fD and nominal descriptor for Palette, Management,
            // SystemMonitorController and SystemMonitorModel respectively.
            let rows=[(0x3f2958usize,0x3f29a8usize,0x14edb4usize,0x37ebc0usize),
                (0x3ea420,0x3ea470,0x10c164,0x37e820),
                (0x3eae00,0x3eae50,0x9b038,0x37f5ec),
                (0x3f4ee8,0x3f4f38,0x1ba9e4,0x37f560)];
            let field_cells=[0x3f0de8usize,0x3f0df0,0x3f0df8,0x3f4e78,0x3f4e80,0x4069e8];
            let invalidate=unsafe{std::mem::transmute::<objc2::runtime::Sel,usize>(objc2::sel!(invalidate))};
            let schedule=unsafe{std::mem::transmute::<objc2::runtime::Sel,usize>(objc2::sel!(scheduledTimerWithTimeInterval:repeats:block:))};
            let timer_class=super::metadata::loaded_class(c"NSTimer")?;
            let check=||->Result<()> {
                reference_mapping_infrastructure()?;
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                let ranges=rows.into_iter().map(|(prefix,_,_,_)|(prefix,88usize))
                    .chain([(0x3f0de8,24),(0x3f4e78,16),(0x4069e8,8),(0x3e87a8,8),(0x3e8d28,8),(0x3e9ad8,8)]);
                for (prefix,bytes) in ranges {
                    let start=at(prefix)?;let end=start.checked_add(bytes).ok_or("Controller metadata range overflow")?;
                    let mut depth=0u32;let mut found=false;
                    for _ in 0..16 {
                        let (mut region,mut len,mut info,mut words)=(start as u64,0u64,[0i32;12],12u32);
                        if unsafe{mach_vm_region_recurse(mach_task_self_,&mut region,&mut len,&mut depth,info.as_mut_ptr(),&mut words)}!=0
                            ||words!=12||region>start as u64||region.checked_add(len).is_none_or(|last|last<end as u64) {
                            return Err("Controller complete metadata mapping absent".into());
                        }
                        if info[8]!=0{depth=depth.checked_add(1).ok_or("Controller metadata submap overflow")?;continue;}
                        if info[0]&1==0||info[0]&4!=0{return Err("Controller metadata is not readable nonexecuting data".into());}
                        found=true;break;
                    }
                    if !found{return Err("Controller metadata submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel{return Err("Controller mapping identity changed".into());}
                if unsafe{(at(0x3e8d28)? as *const usize).read_volatile()}!=schedule
                    ||unsafe{(at(0x3e9ad8)? as *const usize).read_volatile()}!=timer_class.address() {
                    return Err("Original main-run-loop timer factory class/selector differs".into());
                }
                self.check_owner()
            };
            let read=||->Result<([(usize,usize);4],[usize;6],usize)> {
                let mut values=[(0,0);4];
                for (slot,(prefix,descriptor,_,_)) in values.iter_mut().zip(rows) {
                    *slot=unsafe{((at(prefix)? as *const usize).read_volatile(),(at(descriptor)? as *const usize).read_volatile())};
                }
                let mut offsets=[0;6];
                for (slot,cell) in offsets.iter_mut().zip(field_cells){*slot=unsafe{(at(cell)? as *const usize).read_volatile()};}
                Ok((values,offsets,unsafe{(at(0x3e87a8)? as *const usize).read_volatile()}))
            };
            check()?;let before=read()?;self.cleanup_metadata_only()?;check()?;let after=read()?;
            if before!=after{return Err("Controller destruction metadata changed".into());}
            for ((destroyer,descriptor),(_,_,expected_destroyer,expected_descriptor)) in after.0.into_iter().zip(rows) {
                if destroyer!=at(expected_destroyer)?||descriptor!=at(expected_descriptor)?{return Err("Controller compiler destructor/descriptor differs".into());}
            }
            if after.2!=invalidate{return Err("Monitor timer destructor invalidate selector differs".into());}
            self.check_owner()?;Ok(after.1)
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}result
    }
    /// Fixed AppState metadata only; no caller address, offset or object read.
    /// The caller separately retains the genuine initialized state. An absent
    /// lazy cache/size is refused, never treated as a completed nil-state proof.
    pub fn observe_initialized_app_state_destroyer(&self)->Result<AppStateDestroyerObservation> {
        self.observe_app_state_destroyer(true)
    }
    /// Pre-allocation compiler recipe only. A zero lazy cache/size is normal
    /// here; no completed-instance observation escapes this method.
    pub(crate) fn verify_app_state_compiler_metadata(&self)->Result<()> {
        self.observe_app_state_destroyer(false).map(|_|())
    }
    fn observe_app_state_destroyer(&self,completed:bool)->Result<AppStateDestroyerObservation> {
        self.check_owner()?;
        let result=(|| {
            self.verify_input_parent_source_members()?;
            let base=self.observe_app_delegate_offset_cells()?.main_header();
            let at=|offset:usize|base.checked_add(offset).ok_or("AppState fixed metadata address overflow");
            let class_metadata=at(0x3ec968)?;
            let destroyer_cell=at(0x3ec958)?;
            let descriptor_cell=at(0x3ec9a8)?;
            let cache_cell=at(0x3fa940)?;
            let class=if completed {
                let class=super::metadata::loaded_class(c"RuycastF1Reference.AppState")?;
                if class.address()!=class_metadata {return Err("AppState actual metadata address point differs".into());}
                Some(class)
            } else {None};
            let check=||->Result<()> {
                self.cleanup_metadata_only()?;
                // Unlike immutable code/metadata tables, these exact Main
                // cells are runtime-mutable __data/__bss. Keep the global
                // RO/RX reader strict; use the existing live-cell contract.
                reference_mapping_infrastructure()?;
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                for (start,bytes) in [(destroyer_cell,88usize),(cache_cell,8)] {
                    let end=start.checked_add(bytes).ok_or("AppState metadata range overflow")?;
                    let mut depth=0u32;let mut found=false;
                    for _ in 0..16 {
                        let (mut region_start,mut len,mut info,mut words)=(start as u64,0u64,[0i32;12],12u32);
                        if unsafe{mach_vm_region_recurse(mach_task_self_,&mut region_start,&mut len,
                            &mut depth,info.as_mut_ptr(),&mut words)}!=0||words!=12||region_start>start as u64
                            ||region_start.checked_add(len).is_none_or(|last|last<end as u64) {
                            return Err("AppState complete live-metadata mapping absent".into());
                        }
                        if info[8]!=0 {depth=depth.checked_add(1).ok_or("AppState metadata submap overflow")?;continue;}
                        if info[0]&1==0||info[0]&4!=0 {
                            return Err("AppState live metadata is not readable nonexecuting data".into());
                        }
                        found=true;break;
                    }
                    if !found {return Err("AppState complete live-metadata submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                    return Err("AppState metadata kernel mapping identity changed".into());
                }
                if let Some(class)=class {
                    if super::metadata::registered_class(class_metadata)?!=class {
                        return Err("AppState registered class changed".into());
                    }
                }
                self.check_owner()
            };
            let read=|| {
                // These are relocated LIVE words, not encoded Mach-O file
                // fixups. The offsets are also read by the pinned native fD.
                unsafe {(
                    (destroyer_cell as *const usize).read_volatile(),
                    (descriptor_cell as *const usize).read_volatile(),
                    (cache_cell as *const usize).read_volatile(),
                    ((class_metadata+0x30) as *const u32).read_volatile(),
                    ((class_metadata+0x34) as *const u16).read_volatile(),
                )}
            };
            check()?;
            let before=read();
            let instance_size=class.map(|_|unsafe{&*(class_metadata as *const objc2::runtime::AnyClass)}.instance_size());
            check()?;
            let after=read();
            let alignment=usize::from(after.4)+1;
            if before!=after||after.0!=at(0x66478)?||after.1!=at(0x37d988)?
                ||(after.2!=0&&after.2!=class_metadata)
                ||completed&&(after.2!=class_metadata||after.3<16||after.3>1024*1024
                    ||Some(after.3 as usize)!=instance_size||!alignment.is_power_of_two()
                    ||alignment>4096||!(after.3 as usize).is_multiple_of(alignment)) {
                return Err("AppState completed metadata/destructor/layout binding absent or changed".into());
            }
            for (offset,bytes,sha) in [
                (0x66280usize,504usize,"e497cd1915efa0cffa87410d5e325bad677265039b06892b2cd309a14f36cf01"),
                (0x66478,36,"65884dba58d4b893a3844346526450dd988eba58aab34ab2e3e89cd3c4a0338a"),
            ] {self.verify_main_code(at(offset)?,bytes,sha)?;}
            check()?;
            if read()!=after {return Err("AppState metadata changed after destructor code check".into());}
            Ok(AppStateDestroyerObservation{class_metadata,destroyer:after.0,
                instance_size:after.3,alignment_mask:after.4})
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}result
    }
    /// Fixed typed entry/helper membership only. The class-local issuer binds
    /// both actual compiler function pointers and their ABI/source/body closure.
    pub fn verify_app_delegate_sdk_members(&self,entry:usize,checked:usize)->Result<()> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        let result=(|| {
            if entry==0 || checked==0 || entry==checked {return Err("AppDelegate SDK entries absent or duplicate".into());}
            let header=reference_image_header(self.admission.sdk_addresses[0].1,&self.admission.sdk)?;
            if reference_image_header(entry,&self.admission.sdk)?!=header
                || reference_image_header(checked,&self.admission.sdk)?!=header {
                return Err("AppDelegate typed entries are not the same sealed SDK image".into());
            }
            self.cleanup_metadata_only()?;
            self.check_owner()
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// ControlServer's getuid stub uses the already installed private bootstrap
    /// hook. This is an owned replacement observation, never an unpatched one.
    pub(crate) fn observe_control_server_getuid(&self)->Result<super::ImportReplacement> {
        self.check_owner()?;
        let result=(|| {
            self.cleanup_metadata_only()?;
            super::bootstrap::owned_module_context(&self.admission.life(),&self.admission.descriptors())?;
            let original=libc::getuid as *const () as usize;
            let replacement=super::bootstrap::rcf1_getuid as *const () as usize;
            let provider=super::metadata::code_identity(original)?;
            if provider.image()!=b"/usr/lib/system/libsystem_kernel.dylib" {
                return Err("ControlServer getuid original provider differs".into());
            }
            let sdk=reference_image_header(self.admission.sdk_addresses[0].1,&self.admission.sdk)?;
            if reference_image_header(replacement,&self.admission.sdk)?!=sdk {
                return Err("ControlServer getuid replacement belongs to another SDK".into());
            }
            let rows=super::bootstrap::main_imports()?;
            let mut selected=rows.into_iter().filter(|row|row.symbol==c"getuid");
            let row=selected.next().ok_or("ControlServer owned getuid import absent")?;
            if selected.next().is_some()||row.original!=original||row.replacement!=replacement
                ||row.selected_address()!=replacement {
                return Err("ControlServer getuid is not the exact installed bootstrap replacement".into());
            }
            super::bootstrap::owned_module_context(&self.admission.life(),&self.admission.descriptors())?;
            self.cleanup_metadata_only()?;
            if row.selected_address()!=replacement||super::metadata::code_identity(original)?!=provider {
                return Err("ControlServer owned getuid changed during qualification".into());
            }
            self.check_owner()?;Ok(row)
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }

    #[cfg(target_arch = "aarch64")]
    pub(crate) fn observe_control_server_file_import(&self, symbol: &CStr) -> Result<super::ImportReplacement> {
        self.check_owner()?;
        let owner = REFERENCE_LOCK_OWNER.get().ok_or("ControlServer profile import owner absent")?;
        if !Arc::ptr_eq(owner, &self.admission) {
            return Err("ControlServer file import issuer differs".into());
        }
        super::bootstrap::owned_module_context(&owner.life(), &owner.descriptors())?;
        let (spec, offset) = match symbol.to_bytes() {
            b"lstat" => (reference_lstat_spec(), 0x3c4650),
            b"unlink" => (reference_unlink_spec(), 0x3c49a0),
            b"bind" => (reference_bind_spec(), 0x3c4410),
            b"connect" => (reference_connect_spec(), 0x3c4468),
            b"fchmodat" => (reference_chmod_spec(), 0x3c4508),
            b"listen" => (reference_listen_spec(), 0x3c4640),
            b"shutdown" => (reference_shutdown_spec(), 0x3c4908),
            b"accept" => (reference_accept_spec(), 0x3c43d0),
            b"close" => (reference_close_spec(), 0x3c4450),
            b"socket" => (reference_socket_spec(), 0x3c4938),
            _ => return Err("ControlServer file import is outside its roster".into()),
        };
        reference_code(spec.original, Path::new("/usr/lib/system/libsystem_kernel.dylib"))?;
        reference_code(spec.replacement, &self.admission.sdk)?;
        let rows = super::observe_main_imports(&[spec])?;
        let [row] = rows.as_slice() else { return Err("ControlServer file import absent".into()); };
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if base.checked_add(offset) != Some(row.cell_address()) || row.selected_address() != spec.replacement {
            return Err("ControlServer file import is not the installed profile replacement".into());
        }
        self.check_owner()?;
        Ok(row.clone())
    }
    /// The caller holds the AppDelegate's initialized server field on the
    /// creator thread. This observes descriptors without retaining the server.
    #[cfg(target_arch = "aarch64")]
    pub(crate) unsafe fn observe_control_server_descriptors(&self, server: usize) -> Result<(i32, i32)> {
        self.check_owner()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        for (offset, expected) in [(0x301c70, 16), (0x301c78, 24), (0x301ce8, 16)] {
            let cell = base.checked_add(offset).ok_or("Reference control offset overflow")?;
            reference_mapping(cell, 8, Some(true))?;
            if unsafe { (cell as *const u64).read() } != expected {
                return Err("Reference control compiler field offset differs".into());
            }
        }
        if server % 8 != 0 || reference_mapping_protection(server, 32)? & 7 != 3
            || base.checked_add(0x3ef5c0) != Some(unsafe { objc2::ffi::object_getClass(server as *const _) } as usize)
        {
            return Err("Reference ControlServer object differs".into());
        }
        let lock = unsafe { ((server + 16) as *const i32).read_volatile() };
        let listener = unsafe { ((server + 24) as *const usize).read_volatile() };
        if listener % 8 != 0 || reference_mapping_protection(listener, 20)? & 7 != 3
            || base.checked_add(0x3ef680) != Some(unsafe { objc2::ffi::object_getClass(listener as *const _) } as usize)
        {
            return Err("Reference SocketListener object differs".into());
        }
        let descriptor = unsafe { ((listener + 16) as *const i32).read_volatile() };
        self.check_owner()?;
        Ok((lock, descriptor))
    }
    /// Exact raw3 AppDelegate ivar-offset cells, observed as full u64 words.
    /// No caller base/offset is accepted and no receiver/layout permit issued.
    pub fn observe_app_delegate_offset_cells(&self)->Result<AppDelegateOffsetCellsObservation> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        let result=(|| {
            let header=unsafe{_NSGetMachExecuteHeader()} as usize;
            let first=self.admission.grant.main_functions.first().ok_or("AppDelegate MAIN code pin absent")?;
            let code=header.checked_add(first.image_offset).ok_or("AppDelegate MAIN code offset overflow")?;
            if reference_image_header(code,&self.admission.endpoint)?!=header {return Err("AppDelegate offset-cell MAIN differs".into());}
            let offsets=[0x3f60d0usize,0x3f60d8,0x3f60e0,0x3f60e8,0x3f60f0,0x3f60f8,0x3f6100];
            let mut addresses=[0usize;7];
            for (slot,offset) in addresses.iter_mut().zip(offsets) {*slot=header.checked_add(offset).ok_or("AppDelegate offset cell overflow")?;}
            let start=addresses[0];let end=addresses[6].checked_add(8).ok_or("AppDelegate offset range overflow")?;
            let selector=header.checked_add(0x3e8480).ok_or("AppDelegate init selector cell overflow")?;
            let selector_end=selector.checked_add(8).ok_or("AppDelegate selector range overflow")?;
            // objc2 0.6.5 declares Sel repr(transparent) over NonNull<c_void>.
            // Preserve this actual registered selector; no raw selector is constructed.
            let expected_selector=unsafe{std::mem::transmute::<objc2::runtime::Sel,usize>(objc2::sel!(init))};
            let check_region=||->Result<()> {
                reference_mapping_infrastructure()?;
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                for (start,end) in [(start,end),(selector,selector_end)] {
                let mut depth=0u32;let mut found=false;
                for _ in 0..16 {
                    let (mut region_start,mut len,mut region,mut words)=(start as u64,0u64,[0i32;12],12u32);
                    if unsafe{mach_vm_region_recurse(mach_task_self_,&mut region_start,&mut len,&mut depth,region.as_mut_ptr(),&mut words)}!=0
                        || words!=12 || region_start>start as u64 || region_start.checked_add(len).is_none_or(|n|n<end as u64) {
                        return Err("AppDelegate complete offset-cell region absent".into());
                    }
                    if region[8]!=0 {depth=depth.checked_add(1).ok_or("AppDelegate offset submap overflow")?;continue;}
                    if region[0]&1==0 || region[0]&4!=0 {return Err("AppDelegate offset cells are not readable nonexecuting data".into());}
                    found=true;break;
                }
                if !found {return Err("AppDelegate complete cell submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                    return Err("AppDelegate offset-cell mapping infrastructure changed".into());
                }
                self.check_owner()
            };
            check_region()?;
            let read=|| addresses.map(|address|(address,unsafe{(address as *const u64).read_volatile()}));
            let before=read();
            let selector_before=unsafe{(selector as *const usize).read_volatile()};
            check_region()?;
            let after=read();
            let selector_after=unsafe{(selector as *const usize).read_volatile()};
            if before!=after || selector_before!=selector_after || selector_after!=expected_selector
                || after.iter().map(|cell|cell.1).ne([8u64,16,24,32,40,48,56]) {
                return Err("AppDelegate full offset cells differ or changed".into());
            }
            self.check_owner()?;
            Ok(AppDelegateOffsetCellsObservation{main_header:header,cells:after,init_selector_cell:(selector,selector_after)})
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// Fixed compiler-read parent/private-let sources from the same sealed
    /// manifest, not a roster label or a general source-authority constructor.
    pub(crate) fn verify_input_parent_source_members(&self)->Result<()> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        let bytes=self.admission.root.read_optional(Path::new(&self.admission.grant.source_manifest.path),2*1024*1024)?
            .ok_or("Input parent source manifest absent")?;
        if digest(&bytes)!=self.admission.grant.source_manifest.sha256 {return Err("Input parent source manifest changed".into());}
        let manifest:HostSourceManifest=serde_json::from_slice(&bytes).map_err(|_|"Input parent source manifest schema differs")?;
        for (relative,sha) in [
            ("bench/reference/0.8.26/Sources/RuycastApp/Hotkeys.swift","c78041f9d41ba4c97cd2958b3f575b24079d86ce401c3065705835e1d17b8ac0"),
            ("bench/reference/0.8.26/Sources/RuycastApp/HyperKey.swift","a60dff19c9ab210fd8dc3ae564f962a2708b1e32a2f69663ff2c48e6d70b1544"),
            ("bench/reference/0.8.26/Sources/RuycastApp/SnippetExpander.swift","0982fd55b9406f9983d40803ada1f97d415251ca0e5908b888b27da7fd697b96"),
            ("bench/reference/0.8.26/Sources/RuycastApp/MainRunLoop.swift","4d40b6705099184c26d56d8a03d024a010751d715e0ff34a316a011c87e550ba"),
            ("bench/reference/0.8.26/Sources/RuycastApp/PaletteSearchField.swift","9db0c85dee727398ee1960d8eee75bdffd8df0c46e1aabeafa5cd9924ac799e7"),
            ("bench/reference/0.8.26/Sources/RuycastApp/PerformanceTrace.swift","b96fd64ae30c8f550f42c30843d52439b0243d11d7c5df919d05212478ec4dbc"),
            ("bench/reference/0.8.26/Sources/RuycastApp/Appearance.swift","34ed301465269449351ac544fd899902fdfe72c150d577859acc1a90c783bc93"),
            ("bench/reference/0.8.26/Sources/RuycastApp/Management.swift","3057068c7ff081e4f551f46690fb5f29f5e3b1d7f63b55d5667fcdfbebf2d42f"),
            ("bench/reference/0.8.26/Sources/RuycastApp/SystemAccessibility.swift","ceb6320c8132040ab6fe32eb0989a86e11436f580683b7aef9c98aa7243305d2"),
            ("bench/reference/0.8.26/Sources/RuycastApp/WindowManager.swift","01b6d2951b64698664e175694407e9df84faf8a08adb84966fd4a3db7f58fd87"),
            ("bench/reference/0.8.26/Sources/RuycastApp/SystemServices.swift","c2ffbb73cc7723234992b1883bb582e68b175982a6a5991757cf47397e9935ad"),
            ("bench/reference/0.8.26/Sources/RuycastApp/MenuActions.swift","8b0e725641d5e621b456848a7e695a6572d7f5c4ce5e1675e5e1f745dc1b7857"),
            ("bench/reference/0.8.26/Sources/RuycastApp/SnippetChoice.swift","3d43c84d15dc76aaed1a38c5e7d4131232168fb0d0723c79e52deb1e447b9d69"),
            ("bench/reference/0.8.26/Sources/RuycastApp/AppState.swift","ffba512c44134bf74005bd6cc409c1c92d09b4a13ee9709ca60a7b6e895abd14"),
        ] {
            let path=self.admission.root.path().join(relative);
            let expected=path.to_str().ok_or("Input parent source path encoding")?;
            let mut rows=manifest.sources.iter().filter(|source|source.path==expected);
            let row=rows.next().ok_or("Required input parent source is not compiler-bound")?;
            if rows.next().is_some() || row.sha256!=sha {return Err("Input parent source member is duplicated or differs".into());}
        }
        self.check_owner()
    }
    /// Fixed actual MAIN runtime cells only. Their values are observations,
    /// not VM-only 0/8 defaults, field-layout or child/callback permissions.
    pub fn observe_app_state_child_offset_cells(&self)->Result<AppStateChildOffsetCellsObservation> {
        self.check_owner()?;
        let result=(|| {
            self.verify_input_parent_source_members()?;
            let main_header=self.observe_app_delegate_offset_cells()?.main_header();
            let first=main_header.checked_add(0x40c908).ok_or("AppState child offset cell overflow")?;
            let second=main_header.checked_add(0x40c910).ok_or("AppState child offset cell overflow")?;
            let fixed=|offset:usize|main_header.checked_add(offset).ok_or_else(||"AppState field lineage offset overflow".to_owned());
            let state_description=fixed(0x3ec9a8)?;
            let hotkeys_description=fixed(0x3efdd0)?;
            let expander_description=fixed(0x3f4888)?;
            let vector=fixed(0x3ecb10)?;
            let hotkeys_indirect=fixed(0x3c4290)?;
            let expander_indirect=fixed(0x3c41a8)?;
            let static_lineage=||->Result<()> {
                // Fixed raw3 relative-pointer/type bytes; loaded chained-fixup
                // cells are compared separately, never hashed as raw file words.
                for (offset,bytes,sha) in [
                    (0x37d988usize,44usize,"a5703e30e710868f26beccdcf13945f11977b11b31d04878686c97717fe349a9"),
                    (0x382e0c,16,"c14a8c5fc7925d2b5f04e98f3fa727c816d9c7a032e615673b42e3810c0fc876"),
                    (0x383020,24,"084ffad20796ed52ca693717d41385809fb7c7a2827ca15bfcf5e3d08bd8f27b"),
                    (0x332840,8,"70ca13260516cc9bcb6a9c5de537503654140462f8a2ebe0a36beded09b25466"),
                    (0x332848,8,"e648811c7b79a8d2d6fcbc5d6376f40ecc64ee6b41364366907bb6f15fddfbae"),
                    (0x37e4c8,20,"0327ab09d9c9a7e6a552ed9909a402a0bc000310e0607dc4577af29bc5a55130"),
                    (0x37f3a0,20,"7ce8a064f7c7178db54d4ae7fc3f9058f8fcbf773caa62b3132af5ebc1ea8fe4"),
                ] {
                    let address=fixed(offset)?;
                    reference_mapping(address,bytes,None)?;
                    let actual=unsafe{std::slice::from_raw_parts(address as *const u8,bytes)};
                    if digest(actual)!=sha {return Err("AppState fixed field/type lineage bytes differ".into());}
                }
                for (offset,expected) in [(0x380849usize,b"hotkeys\0".as_slice()),(0x380851,b"expander\0".as_slice())] {
                    let address=fixed(offset)?;
                    reference_mapping(address,expected.len(),None)?;
                    if unsafe{std::slice::from_raw_parts(address as *const u8,expected.len())}!=expected {
                        return Err("AppState fixed field names differ".into());
                    }
                }
                self.check_owner()
            };
            let check=||->Result<()> {
                static_lineage()?;
                reference_mapping_infrastructure()?;
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                for (range_start,bytes) in [(first,16usize),(vector,16),(state_description,8),(hotkeys_description,8),(expander_description,8),(hotkeys_indirect,8),(expander_indirect,8)] {
                let range_end=range_start.checked_add(bytes).ok_or("AppState lineage range overflow")?;
                let mut depth=0u32;let mut found=false;
                for _ in 0..16 {
                    let (mut start,mut len,mut info,mut words)=(range_start as u64,0u64,[0i32;12],12u32);
                    if unsafe{mach_vm_region_recurse(mach_task_self_,&mut start,&mut len,&mut depth,info.as_mut_ptr(),&mut words)}!=0
                        || words!=12 || start>range_start as u64 || start.checked_add(len).is_none_or(|n|n<range_end as u64) {
                        return Err("AppState complete child-offset mapping absent".into());
                    }
                    if info[8]!=0 {depth=depth.checked_add(1).ok_or("AppState child-offset submap overflow")?;continue;}
                    if info[0]&1==0 || info[0]&4!=0 {return Err("AppState child-offset cells are not readable nonexecuting data".into());}
                    found=true;break;
                }
                if !found {return Err("AppState complete field-lineage submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                    return Err("AppState child-offset kernel mapping changed".into());
                }
                self.check_owner()
            };
            check()?;
            let read=||[(first,unsafe{(first as *const u64).read_volatile()}),(second,unsafe{(second as *const u64).read_volatile()})];
            let lineage=||[
                unsafe{(state_description as *const u64).read_volatile()},
                unsafe{(hotkeys_description as *const u64).read_volatile()},
                unsafe{(expander_description as *const u64).read_volatile()},
                unsafe{(hotkeys_indirect as *const u64).read_volatile()},
                unsafe{(expander_indirect as *const u64).read_volatile()},
                unsafe{(vector as *const u64).read_volatile()},
                unsafe{((vector+8) as *const u64).read_volatile()},
            ];
            let before=read();let lineage_before=lineage();check()?;
            let after=read();let lineage_after=lineage();
            if before!=after || lineage_before!=lineage_after
                || lineage_after[..5]!=[fixed(0x37d988)? as u64,fixed(0x37e4c8)? as u64,fixed(0x37f3a0)? as u64,fixed(0x37e4c8)? as u64,fixed(0x37f3a0)? as u64]
                || lineage_after[5]!=after[0].1 || lineage_after[6]!=after[1].1 {
                return Err("AppState typed field/vector/offset lineage differs or changed".into());
            }
            self.check_owner()?;
            Ok(AppStateChildOffsetCellsObservation{main_header,cells:after})
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// Fixed raw3 private-let field/type/vector lineage in MAIN. No child
    /// heap read, retain, initializer call or callback permission is issued.
    pub fn observe_hotkeys_keyboard_offset_cell(&self)->Result<(usize,u64)> {
        self.check_owner()?;
        let result=(|| {
            self.verify_input_parent_source_members()?;
            let header=self.observe_app_delegate_offset_cells()?.main_header();
            let fixed=|offset:usize|header.checked_add(offset).ok_or_else(||"Hotkeys keyboard lineage offset overflow".to_owned());
            let address=fixed(0x302b38)?;
            let hotkeys_description=fixed(0x3efdd0)?;
            let hyper_description=fixed(0x3f00b8)?;
            let hyper_indirect=fixed(0x3c42a8)?;
            let vector=fixed(0x3efe08)?;
            let check=||->Result<()> {
                reference_mapping_infrastructure()?;
                let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
                // Actual Hotkeys descriptor: ten fields, vector offset ten,
                // no metadata-init descriptor. Field five is let keyboard and
                // its symbolic nonoptional type is the fixed HyperKey class.
                for (offset,bytes,sha) in [
                    (0x37e4c8usize,44usize,"1d6ed3b4d2bcf26694f3015a544443346dc3c0ae187bfea033d24e3e23b3efff"),
                    (0x383e98,16,"0c267c0dcf87aa68b879f764fe6cbb0dd38b403c6574377943c9a33c82fa47f2"),
                    (0x383ee4,12,"73af1896e4e07289da466cf84cca9d4159178bfa8fe46695d398ac10edb47f28"),
                    (0x33f048,6,"04b6305f14c862ce5caf3efd32b128182071f1c94e3769f53eed4d003ae84995"),
                    (0x37e504,20,"724b4b145cd80e1fd410bc000eddc19c9c616df96c7e9bf1403f29db22c4019a"),
                    (0x381253,9,"435d3fdcedbd68da8e19350cb367d47a90c150e3145dce3b48549115e6418193"),
                ] {
                    let member=fixed(offset)?;
                    reference_mapping(member,bytes,None)?;
                    if digest(unsafe{std::slice::from_raw_parts(member as *const u8,bytes)})!=sha {
                        return Err("Hotkeys fixed keyboard/type lineage bytes differ".into());
                    }
                }
                // The fixed global is immutable __TEXT data. These four
                // loaded metadata/fixup/vector words are READ/nonexec; do not
                // hash file-encoded chained pointers as loaded pointers.
                reference_mapping(address,8,None)?;
                for start in [hotkeys_description,hyper_description,hyper_indirect,vector] {
                    let end=start.checked_add(8).ok_or("Hotkeys keyboard word range overflow")?;
                    let mut depth=0u32;let mut found=false;
                    for _ in 0..16 {
                        let (mut region_start,mut len,mut info,mut words)=(start as u64,0u64,[0i32;12],12u32);
                        if unsafe{mach_vm_region_recurse(mach_task_self_,&mut region_start,&mut len,&mut depth,info.as_mut_ptr(),&mut words)}!=0
                            ||words!=12||region_start>start as u64||region_start.checked_add(len).is_none_or(|n|n<end as u64) {
                            return Err("Hotkeys complete keyboard lineage mapping absent".into());
                        }
                        if info[8]!=0 {depth=depth.checked_add(1).ok_or("Hotkeys keyboard submap overflow")?;continue;}
                        if info[0]&1==0||info[0]&4!=0 {return Err("Hotkeys keyboard lineage is not readable nonexecuting data".into());}
                        found=true;break;
                    }
                    if !found {return Err("Hotkeys complete keyboard lineage submap absent".into());}
                }
                if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                    return Err("Hotkeys keyboard mapping infrastructure changed".into());
                }
                self.check_owner()
            };
            check()?;
            let read=||[
                unsafe{(address as *const u64).read_volatile()},
                unsafe{(hotkeys_description as *const u64).read_volatile()},
                unsafe{(hyper_description as *const u64).read_volatile()},
                unsafe{(hyper_indirect as *const u64).read_volatile()},
                unsafe{(vector as *const u64).read_volatile()},
            ];
            let before=read();check()?;let after=read();
            if before!=after || after!=[56,fixed(0x37e4c8)? as u64,fixed(0x37e504)? as u64,fixed(0x37e504)? as u64,56] {
                return Err("Hotkeys typed keyboard/vector/global lineage differs or changed".into());
            }
            self.check_owner()?;
            Ok((address,after[0]))
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    pub fn before_init(&self)->Result<()> {
        self.check_owner()?;
        self.model.verify_reference_context(&self.admission)?;
        self.cleanup_metadata_only()?;
        self.model.verify_reference_context(&self.admission)?;
        self.check_owner()
    }
    /// Installing the source constructor is assembly; invoking its original
    /// Swift initializer still requires the published, complete SDK boundary.
    pub fn before_original_initialization(&self)->Result<()> {
        self.before_init()?;
        self.admission.before_reference_publication()?;
        self.check_owner()
    }
    fn check_owner(&self)->Result<()> {
        self.verify_published_code_epoch()?;
        if std::thread::current().id()!=self.owner {
            return Err("AppDelegate context on foreign owner thread".into());
        }
        Ok(())
    }
    pub(crate) fn verify_published_code_epoch(&self)->Result<()> {
        if super::process_fork::is_child() {unsafe{libc::_exit(78)}}
        if unsafe{libc::getpid()}!=self.process
            || self.code_retired.load(std::sync::atomic::Ordering::Acquire) {
            return Err("AppDelegate context code retired or foreign process".into());
        }
        Ok(())
    }
    pub(crate) fn retire_code(&self) {
        self.code_retired.store(true,std::sync::atomic::Ordering::Release);
    }
    /// Code/source custody only, intentionally available after business/model
    /// retirement for a separately ledger-qualified known partial allocation.
    /// This does not itself qualify ARC, a destructor or any raw receiver.
    pub fn cleanup_metadata_only(&self)->Result<()> {
        self.check_owner()?;
        let result=(|| {
            image(&self.admission.endpoint)?;
            if !self.callback_sources.reusing() {
                checked_hash(&self.admission.root,&self.admission.endpoint,&self.admission.grant.endpoint_sha256)?;
                checked_hash(&self.admission.root,&self.admission.sdk,&self.admission.grant.sdk_artifact_sha256)?;
                reference_sources(&self.admission.root,&self.admission.grant)?;
            }
            reference_loaded_table(&self.admission.endpoint,&self.admission.sdk,&self.admission.grant,
                self.admission.table,self.admission.sdk_addresses)?;
            let path=self.admission.root.path().join("bench/reference/0.8.26/Sources/RuycastApp/main.swift");
            let bytes=self.admission.root.read_optional(Path::new(&self.admission.grant.source_manifest.path),2*1024*1024)?
                .ok_or("AppDelegate private source manifest absent")?;
            if digest(&bytes)!=self.admission.grant.source_manifest.sha256 {
                return Err("AppDelegate private source manifest changed while reading".into());
            }
            let manifest:HostSourceManifest=serde_json::from_slice(&bytes).map_err(|_|"AppDelegate source manifest schema differs")?;
            let expected=path.to_str().ok_or("AppDelegate source path encoding")?;
            if self.constructor_source.source_sha256!=self.callback_source.source_sha256
                || manifest.sources.iter().filter(|source|source.path==expected
                    && source.sha256==self.constructor_source.source_sha256).count()!=1 {
                return Err("AppDelegate constructor/callback is not the same private compiled source member".into());
            }
            self.check_owner()
        })();
        if result.is_err() {self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }

    #[cfg(target_arch = "aarch64")]
    pub(crate) fn configuration_observer_offset(&self) -> Result<u64> {
        #[link(name = "swiftDarwin")]
        unsafe extern "C" {
            #[link_name = "$s6Darwin4openys5Int32VSPys4Int8VG_ADtF"]
            static OBSERVER_OPEN: u8;
        }
        self.cleanup_metadata_only()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference configuration observer address overflow");
        // The observer calls Swift Darwin's two-argument open, not the C
        // process import. These are code observations only.
        unsafe { self.with_callback_source_observation(|| {
            for (offset, length, hash) in [
                (0x4df08, 136, "3120e115316e188692cd02d5b98bc99c5d68f450fe2ef7ab4c3b61693cb49b90"),
                (0x7f870, 28, "ebb2976c0b35b389e46b634c42d8609168f16e2194fa4917978392d258a0df20"),
                (0x7f988, 24, "aee2487b1b158a8d6dc6b4e21fcf35446ee77dc3d120f941a2eedc8065a009a6"),
                (0x80544, 24, "f2bdb74e6ba12872a923f90c3d5878935c8ddde224bf137f7d311dd05dd1c61d"),
                (0x2f9590, 12, "02cb2b22d63ae09ee3980c4cebc4ca327d5da0214d9ed4fbb0920282a942259f"),
                (0x81248, 200, "500a264672f18ed9be2049132bd5d15b2cfb3a51598f43bc743b3fcac1464a85"),
                (0x81310, 36, "f8ae17bae6d930317ba1015b132253d7efe4d94ddc655ae12f62dd87820458cd"),
                (0x801e8, 48, "2e347d0c2c6ed5aeabf8d12ab852980e9b95ac8000873799859286faa15dd6f6"),
                (0x802b0, 32, "5aea226cd8251f01aeebcc273851d673901ad4c6fe75a76a306fc4c5831ab23b"),
                (0x8089c, 64, "b6b562334ee0e2f5499d4d53099d7552c43a334aeb9cf4d84157d3869800ce56"),
                (0x8096c, 32, "2c60615c28b503857eb4f68805b7a723fb3b70f83ec38f710badac5b7abc933b"),
                (0x818b4, 88, "589e0d6843a239d7ad69c8657445ec5ab379942f5a3dae75f0a6ca5cc9f139d3"),
                (0x81a4c, 296, "10d48b1fc8c515cf39d4fdb0bc4690cf33ee5aed6d2d6405a2b341ff6b6f644f"),
                (0x81bec, 8, "7dca10f6a848c87490f4981e2df04d1f977816d1f3f461fe3e0b91fa0a6bd19f"),
                (0x81c1c, 8, "8a4557fd8756149924aec9568cfe21929c00ed14b5d14c87c2b0b7cabc8946b8"),
            ] {
                self.verify_main_code(at(offset)?, length, hash)?;
            }
            for (descriptor, copy, dispose) in [
                (0x3c9998, 0x81bec, 0x81c1c), // Debounced observer check.
                (0x3c99e8, 0x81bf0, 0x81c20), // Configuration result on main.
            ] {
                let address = at(descriptor)?;
                reference_mapping(address, 40, Some(false))?;
                let words = (address as *const [usize; 5]).read_volatile();
                if words != [0, 48, at(copy)?, at(dispose)?, at(0x3672b0)?] {
                    return Err("Reference configuration observer Dispatch descriptor differs".into());
                }
            }
            Ok(())
        }) }?;
        let spec = super::ImportSpec {
            symbol: super::bootstrap::REFERENCE_PROFILE_READ_IMPORT,
            original: (&raw const OBSERVER_OPEN) as usize,
            replacement: reference_profile_read_open as *const () as usize,
        };
        let rows = super::observe_main_imports(&[spec])?;
        let [open] = rows.as_slice() else {
            return Err("Reference configuration observer open import absent".into());
        };
        if open.cell_address() != at(0x3c7530)? || open.selected_address() != spec.replacement {
            return Err("Reference configuration observer open import cell differs".into());
        }
        reference_code(spec.replacement, &self.admission.sdk)?;
        // The bound shutdown loads this offset and clears the corresponding field.
        let cell = at(0x3fac68)?;
        if reference_mapping_protection(cell, 8)? & 7 != 3 {
            return Err("Reference configuration observer offset cell protection differs".into());
        }
        self.check_owner()?;
        Ok(unsafe { (cell as *const u64).read_volatile() })
    }

    /// The caller keeps the original observer alive through its AppState slot
    /// on the creator thread. The observer retains both sources. Terminal
    /// ingress may cancel them before enqueuing a marker on their queue.
    #[cfg(target_arch = "aarch64")]
    pub(crate) unsafe fn verify_configuration_observer_sources(&self, observer: usize, cancel: bool) -> Result<usize> {
        use std::ffi::c_void;
        unsafe extern "C" {
            fn dispatch_source_get_handle(source: *const c_void) -> usize;
            fn dispatch_source_get_mask(source: *const c_void) -> usize;
            fn dispatch_source_testcancel(source: *const c_void) -> isize;
            fn dispatch_source_cancel(source: *const c_void);
        }
        self.check_owner()?;
        let recorded_fd = REFERENCE_WATCH_FD.with(std::cell::Cell::get);
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let source_offsets = base.checked_add(0x2ff270).ok_or("Reference observer offset overflow")?;
        reference_mapping(source_offsets, 16, Some(true))?;
        let offsets = unsafe { (source_offsets as *const [u64; 2]).read() };
        if offsets != [0x18, 0x20] || observer % 8 != 0
            || reference_mapping_protection(observer, 0x28)? & 7 != 3
        {
            return Err("Reference observer source layout differs".into());
        }
        let class = unsafe { objc2::ffi::object_getClass(observer as *const _) };
        if base.checked_add(0x3ecfc8) != Some(class as usize) {
            return Err("Reference configuration observer runtime class differs".into());
        }
        let queue = unsafe { ((observer + 0x10) as *const usize).read() };
        unsafe { verify_configuration_observer_queue(queue) }?;
        let [vnode, timer] = unsafe { ((observer + 0x18) as *const [usize; 2]).read() };
        let dispatch_class = super::metadata::loaded_class(c"OS_dispatch_source")?;
        if vnode == timer {
            return Err("Reference watcher sources must be distinct".into());
        }
        for source in [vnode, timer] {
            if source % 8 != 0 || reference_mapping_protection(source, 8)? & 7 != 3
                || unsafe { objc2::ffi::object_getClass(source as *const _) } as usize != dispatch_class.address()
            {
                return Err("Reference watcher Dispatch source class differs".into());
            }
        }
        let libdispatch = Path::new("/usr/lib/system/libdispatch.dylib");
        for entry in [
            dispatch_source_get_handle as *const () as usize,
            dispatch_source_get_mask as *const () as usize,
            dispatch_source_testcancel as *const () as usize,
            dispatch_source_cancel as *const () as usize,
        ] {
            reference_code(entry, libdispatch)?;
        }
        // Swift's [.write, .rename, .delete, .attrib, .revoke] vnode mask.
        if unsafe {
            dispatch_source_get_mask(vnode as *const c_void) != 0x6b
                || dispatch_source_testcancel(vnode as *const c_void) != 0
                || dispatch_source_testcancel(timer as *const c_void) != 0
        } {
            return Err("Reference watcher vnode handle, mask or cancellation differs".into());
        }
        let fd = i32::try_from(unsafe { dispatch_source_get_handle(vnode as *const c_void) })
            .map_err(|_| "Reference watcher descriptor width differs")?;
        if fd <= 2 || recorded_fd != Some(fd) {
            return Err("Reference watcher source descriptor differs".into());
        }
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } != libc::FD_CLOEXEC
            || unsafe { libc::fstat(fd, info.as_mut_ptr()) } != 0
        {
            return Err("Reference watcher descriptor is not live and close-on-exec".into());
        }
        let info = unsafe { info.assume_init() };
        if info.st_mode & libc::S_IFMT != libc::S_IFDIR || info.st_uid != self.admission.grant.owner_uid
            || (info.st_dev as u64, info.st_ino) != reference_profile_identity(&self.admission.profile_directory)?
        {
            return Err("Reference watcher descriptor profile differs".into());
        }
        self.check_owner()?;
        if cancel {
            // The original observer still holds both sources and its queue.
            // Cancellation stops future events but permits an active handler
            // to finish. The caller enqueues a drain marker on this same queue.
            unsafe {
                dispatch_source_cancel(vnode as *const c_void);
                dispatch_source_cancel(timer as *const c_void);
            }
            if unsafe { dispatch_source_testcancel(vnode as *const c_void) == 0
                || dispatch_source_testcancel(timer as *const c_void) == 0 }
            {
                return Err("Reference configuration sources did not cancel".into());
            }
        }
        self.check_owner()?;
        Ok(queue)
    }

    /// Observe the borrowed box without performing any ARC operation.
    ///
    /// # Safety
    /// The caller retains the exact borrowed F1Box and both AppDelegate field
    /// owners on the creator thread for the whole observation.
    pub(crate) unsafe fn verify_borrowed_reference_box(&self, pointer: usize, state: usize, palette: usize) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_release();
            fn swift_deallocClassInstance();
        }
        self.cleanup_metadata_only()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference borrowed box offset overflow");
        for (offset, length, hash) in [
            (0x2000cc, 232, "0574453ebb7cf5a6abc6453672c288d081e9829e0f1c5f07a0723aed68ab1f04"),
            (0x200e20, 460, "805d5e2a48e32952e5787a82b9fc9bb3df792595b63d580464edb14ccddf2be9"),
            (0x20013c, 120, "3fef93ad9b9d137d824280587d2d52e9d2be960503ea94d10ec40e8dc08e1306"),
            (0x1ffe58, 44, "ed7b72213c25ba2761eef91e827de509c9a52b104b4f3410989d199a6b6e0e96"),
            (0x202f00, 8, "ea85c3d21a54e096002086d3ee95e7391adc3c5d03c0bce5be26d6268cec4e1b"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
            (0x2fbff0, 12, "0ce9a5a37f21099bbd755b829f71dfc623fda7e466d48b610261375d4dbbe537"),
        ] {
            self.verify_main_code(at(offset)?, length, hash)?;
        }
        if state == 0 || palette == 0 || pointer == 0 || !pointer.is_multiple_of(8)
            || reference_mapping_protection(pointer, 33)? & 7 != 3
        {
            return Err("Reference borrowed box storage or parent identity differs".into());
        }
        // The actual Swift class metadata points at the destructor that
        // releases the two stored references and deallocates the 33-byte box.
        // This observation grants no ARC or release permission.
        let destroy_cell = at(0x3f62e8)?;
        if reference_mapping_protection(destroy_cell, size_of::<usize>())? & 7 != 3
            || unsafe { (destroy_cell as *const usize).read_volatile() } != at(0x1ffe58)?
        {
            return Err("Reference borrowed box destructor selection differs".into());
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"swift_release", swift_release as *const () as usize),
            (c"swift_deallocClassInstance", swift_deallocClassInstance as *const () as usize),
        ])?;
        if imports.len() != 2 {
            return Err("Reference borrowed box ARC imports differ".into());
        }
        for row in imports {
            let offset = if row.symbol == c"swift_release" { 0x3c7438 } else { 0x3c7308 };
            if row.cell_address() != at(offset)? {
                return Err("Reference borrowed box ARC import cell differs".into());
            }
            reference_code(row.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
            row.verify_unpatched()?;
        }
        let class = super::metadata::registered_class(at(0x3f62f8)?)?;
        let metadata = unsafe { (pointer as *const usize).read_volatile() };
        let held_state = unsafe { ((pointer + 16) as *const usize).read_volatile() };
        let held_palette = unsafe { ((pointer + 24) as *const usize).read_volatile() };
        let owns_lifecycle = unsafe { ((pointer + 32) as *const u8).read_volatile() };
        if metadata != class.address() || held_state != state || held_palette != palette || owns_lifecycle != 0 {
            return Err("Reference borrowed box does not hold this AppDelegate state and palette".into());
        }
        self.check_owner()
    }
    /// # Safety
    /// Only fixed observations may enter: callback evidence_body AFTER
    /// dynamic verify_model, AppStateCompilerCleanup's fixed metadata, source
    /// member, code and import checks, and the
    /// separate verify_notification_thunk / verify_control_thunks methods,
    /// ControlServer's fixed stub loop and SubscriptionsCompilerMemory's
    /// source/type/layout/import observation body, and Carbon's fixed
    /// original code and SDK entry-byte/image membership checks, and
    /// AppDelegateConstructionData's fixed original code-pin loop, and the
    /// retained SearchInput conformance/Notification/CTo observations and
    /// configuration observer's fixed Main code-pin loop, and Caps' fixed
    /// parent source membership and loaded compiler text checks.
    /// Other controller/server checks and both dynamic getuid/guardian
    /// observations stay outside these scopes. All bodies are synchronous:
    /// no callback, guardian check, patch, ARC or business effect.
    pub(crate) unsafe fn with_callback_source_observation(&self,observe:impl FnOnce()->Result<()>)->Result<()> {
        self.check_owner()?;
        self.callback_sources.run(&self.code_retired,||self.cleanup_metadata_only(),observe)
    }
    pub(crate) fn callback_source_observation_active(&self) -> Result<bool> {
        self.check_owner()?;
        Ok(self.callback_sources.reusing())
    }
    /// A byte-bound member of the actual sealed MAIN RX image, never a callable
    /// ABI/initializer grant. The Native issuer must also bind type and CFG.
    /// The fixed original representable conformance, including its resilient
    /// make/update/dismantle/coordinator witnesses. This joins actual compiler
    /// data to SwiftUI's coordinator/view lifetime contract; a host pointer or
    /// matching weak delegate alone does not establish that ownership.
    pub(crate) fn verify_search_field_conformance(&self)->Result<()> {
        #[link(name="SwiftUI",kind="framework")]
        unsafe extern "C"{#[link_name="$s7SwiftUI19NSViewRepresentableMp"]static PROTOCOL:u8;}
        self.verify_input_parent_source_members()?;
        self.verify_original_compiler_text()?;
        let base=self.observe_app_delegate_offset_cells()?.main_header();
        let at=|offset|base.checked_add(offset).ok_or_else(||"Search field conformance offset overflow".to_owned());
        let words=[0x000c9639u32,0x00079260,0,0x00030000,14,
            0x000c962d,0x00050bf7,0x000c962d,0x00050bf7,0x000c962d,0x00050bf5,
            0x000c962d,0xffe54840,0x000c962d,0xffe5483c,0x000c962d,0xffe54838,0x000c962d,0xffe54834,
            0x000c962d,0xffd94ad8,0x000c962d,0xffd94ad4,0x000c962d,0xffd94afc,0x000c962d,0xffd94af8,
            0x000c962d,0xffd94af4,0x000c962d,0xffd94af0,0x000c962d,0xffd94aec,0x00010000,0,0x000fc36c];
        let expected=words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        let descriptor=at(0x305b48)?;let protocol=std::ptr::addr_of!(PROTOCOL) as usize;
        let check=||->Result<()> {
            reference_mapping_infrastructure()?;
            reference_mapping(descriptor,expected.len(),None)?;
            reference_mapping(at(0x3cf180)?,8,None)?;
            reference_mapping(protocol,1,None)?;
            if unsafe{std::slice::from_raw_parts(descriptor as *const u8,expected.len())}!=expected
                ||unsafe{(at(0x3cf180)? as *const usize).read_volatile()}!=protocol {
                return Err("Actual SearchField representable conformance differs".into());
            }
            let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
            if unsafe{libc::dladdr(protocol as *const libc::c_void,info.as_mut_ptr())}==0{return Err("SwiftUI protocol provider absent".into());}
            let info=unsafe{info.assume_init()};
            if info.dli_fname.is_null()||info.dli_sname.is_null()||info.dli_saddr as usize!=protocol
                ||unsafe{CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI"
                ||unsafe{CStr::from_ptr(info.dli_sname)}!=c"$s7SwiftUI19NSViewRepresentableMp" {
                return Err("SearchField conformance protocol provider differs".into());
            }
            self.check_owner()
        };
        check()?;self.cleanup_metadata_only()?;check()
    }
    pub fn verify_main_code(&self,address:usize,bytes:usize,expected_sha256:&str)->Result<()> {
        self.cleanup_metadata_only()?;
        let result=(|| {
            if bytes==0 || bytes>65536 {return Err("AppDelegate MAIN code span exceeds bound".into());}
            reference_hash(expected_sha256)?;
            if reference_image_header(address,&self.admission.endpoint)?!=unsafe{_NSGetMachExecuteHeader()} as usize {
                return Err("AppDelegate code is not the actual MAIN image".into());
            }
            reference_mapping(address,bytes,Some(true))?;
            let code=unsafe{std::slice::from_raw_parts(address as *const u8,bytes)};
            if digest(code)!=expected_sha256 {return Err("AppDelegate MAIN code bytes differ".into());}
            self.check_owner()
        })();
        if result.is_err() {self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    pub(crate) fn verify_reference_snippet_panel_constructor(&self) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_retain();
        }
        self.verify_input_parent_source_members()?;
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        let at = |offset| base.checked_add(offset).ok_or("Reference SnippetChoicePanel address overflow");
        // Only immutable code observations share this source-validation scope.
        unsafe { self.with_callback_source_observation(|| {
            for (offset, bytes, sha) in [
                (0x1a2e28, 44, "c185ff2764ba1327047648bcc2d2475383f6cc9ca5dda4e24bfac945ee3f4b82"),
                (0x1a2e54, 200, "9dfb36d8a660fb640af0daaca9069f8d32f4a290195b887be9a7eaeb4dba575c"),
                (0x1a605c, 24, "0effab5cd1088a859b87e14606f5886daadf4f43335adfbf1578b9e47a5470bd"),
                (0x3da58, 8, "edf8c9830e5e4ce45da725117c2d99b70d9192b1a1cc29e5f965b8e21f646853"),
                (0x2fc1c4, 12, "b681305e25fa18b584abf72b26653eb8a3fab78666524b51f2d98df7a2e73e01"),
                (0x1a1a70, 8, "29570d2f1e5bd9561437314032a3e6c809c2925e1eb42abfb224ee4779907c21"),
                (0x2fbb10, 12, "d2bc9ae552d79df7b57b9087552ded25b98e3b8ce50054812ca2edffef979f31"),
                (0x1a60d8, 144, "019e4210e79f3763de74c7890b998f994d4a046bcdb4e55cd03d5090110acfc8"),
                (0x152624, 8, "6b13fbf08d2018a9975cca5d0620c15cd62af47eaa3281e4738830f88c70f73a"),
                (0x1a8eb4, 8, "8df9ac8de9c41f43ee860b96ee227b52d1cbaa0aba04ffa9bf2d13fae2d61a52"),
                // The original global mouse monitor passes a copied Swift
                // block to NSEvent and removes its bridged token in deinit.
                (0x1a6404, 148, "67d4993d5290175ca2e2c07579ab9c50397fd969af7f9a59f2065f9156f7336f"),
                (0x1a587c, 44, "e8b7f14a1f11d61d6e80e1d01b106dec933d518ea3f01450903d645eeff6712e"),
                (0x115a08, 100, "6ee4e8cbc4d0d6733fbb4d382acf2d5162a4e771945caf21e734fa62ee48fc68"),
                (0x1a76f4, 164, "86049619483dff7d0295c34d4661a8e61d3886085a26b120963e07901598e893"),
                (0x1a8cfc, 4, "ca3d3e8eb5db81486bceaa5988dc7f7029f531bc4982f447a9d4fdecdca88ed1"),
                (0x1a8d18, 4, "461b057e639e01da5ec229c6a893c7d2665d7b6887d3775ca742507ee38caff9"),
            ] {
                self.verify_main_code(at(offset)?, bytes, sha)?;
            }
            Ok(())
        }) }?;
        let monitor_selectors = [
            (0x3e8e80, c"addGlobalMonitorForEventsMatchingMask:handler:"),
            (0x3e8e90, c"removeMonitor:"),
        ];
        for &(offset, name) in crate::workspace::objc::REFERENCE_SNIPPET_APPEARANCE_SELECTORS
            .iter().chain(&monitor_selectors)
        {
            let selector = objc2::runtime::Sel::register(name);
            let selector = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) };
            let address = at(offset)?;
            if reference_mapping_protection(address, size_of::<usize>())? & 5 != 1
                || unsafe { (address as *const usize).read_volatile() } != selector
            {
                return Err("Reference SnippetChoicePanel selector differs".into());
            }
        }
        let descriptor = at(0x3d0b40)?;
        reference_mapping(descriptor, 40, Some(false))?;
        let actual = unsafe { (descriptor as *const [usize; 5]).read_volatile() };
        if actual != [0, 48, at(0x1a8cfc)?, at(0x1a8d18)?, at(0x36fe60)?] {
            return Err("Reference SnippetChoice mouse monitor block descriptor differs".into());
        }
        let signature = b"v16@?0@\"NSEvent\"8\0";
        reference_mapping(actual[4], signature.len(), Some(true))?;
        if unsafe { std::slice::from_raw_parts(actual[4] as *const u8, signature.len()) } != signature {
            return Err("Reference SnippetChoice mouse monitor block signature differs".into());
        }
        for (offset, expected) in [(0x307330, 70.0_f64), (0x307338, 130.0),
            (0x307340, 456.0), (0x306308, 440.0)] {
            let address = at(offset)?;
            reference_mapping(address, size_of::<u64>(), None)?;
            if reference_image_header(address, &self.admission.endpoint)? != base
                || unsafe { (address as *const u64).read_volatile() } != expected.to_bits()
            {
                return Err("Reference SnippetChoicePanel geometry constants differ".into());
            }
        }
        let panel = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoicePanel")?;
        let parent = super::metadata::loaded_class(c"NSPanel")?;
        if panel.address() != at(0x3eab70)? {
            return Err("Reference SnippetChoicePanel class differs".into());
        }
        let class = unsafe { &*(panel.address() as *const objc2::runtime::AnyClass) };
        if class.superclass().map(|class| class as *const _ as usize) != Some(parent.address()) {
            return Err("Reference SnippetChoicePanel superclass differs".into());
        }
        let selector_name = c"initWithContentRect:styleMask:backing:defer:";
        let selector = objc2::runtime::Sel::register(selector_name);
        let selector_bits = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) };
        let selector_cell = at(0x3e8440)?;
        if reference_mapping_protection(selector_cell, size_of::<usize>())? & 5 != 1
            || unsafe { (selector_cell as *const usize).read_volatile() } != selector_bits
        {
            return Err("Reference SnippetChoicePanel super-init selector differs".into());
        }
        let inherited = super::metadata::method(parent, selector_name, false)?
            .ok_or("Reference SnippetChoicePanel inherited initializer absent")?;
        use objc2::encode::{Encode, Encoding};
        crate::workspace::classes::verify_typed_abi(&inherited, &[
            Encoding::Object, Encoding::Object, objc2::runtime::Sel::ENCODING,
            objc2_foundation::NSRect::ENCODING, usize::ENCODING, usize::ENCODING,
            objc2::runtime::Bool::ENCODING,
        ])?;
        // The initializer derives its height from the same immutable Array
        // field that the original model getter reads. This observes layout
        // only. No model instance or Array storage is borrowed here.
        let model = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceModel")?;
        if model.address() != at(0x3f43d8)? {
            return Err("Reference SnippetChoiceModel class differs".into());
        }
        let model = unsafe { &*(model.address() as *const objc2::runtime::AnyClass) };
        let model_fields = model.instance_variables();
        let mut template_fields = model_fields.iter().filter(|ivar| ivar.name() == c"fields");
        if template_fields.next().map(|ivar| ivar.offset()) != Some(0xc8)
            || template_fields.next().is_some() || model.instance_size() < 0xd0
        {
            return Err("Reference SnippetChoiceModel template fields layout differs".into());
        }
        self.admission.verify_reference_snippet_panel_cleanup()?;
        let retains = super::imports::observe_unpatched_main_imports(&[
            (c"swift_retain", swift_retain as *const () as usize),
        ])?;
        let [retain] = retains.as_slice() else {
            return Err("Reference SnippetChoicePanel retain import absent".into());
        };
        if retain.cell_address() != at(0x3c7448)? {
            return Err("Reference SnippetChoicePanel retain import cell differs".into());
        }
        reference_code(retain.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
        retain.verify_unpatched()?;
        self.cleanup_metadata_only()
    }

    /// The caller holds the original controller in the expander's strong slot.
    pub(crate) unsafe fn snippet_choice_notification_slots(&self, controller: usize) -> Result<([usize; 3], usize)> {
        self.check_owner()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let class = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceController")?;
        if base.checked_add(0x3f44d0) != Some(class.address()) || !controller.is_multiple_of(8)
            || reference_mapping_protection(controller, 200)? & 7 != 3
            || unsafe { (controller as *const usize).read_volatile() } != class.address()
        {
            return Err("Reference SnippetChoice notification owner differs".into());
        }
        let class = unsafe { &*(class.address() as *const objc2::runtime::AnyClass) };
        let fields = class.instance_variables();
        let mut values = [0; 3];
        if class.instance_size() != 200 {
            return Err("Reference SnippetChoice notification layout differs".into());
        }
        for (value, (name, offset)) in values.iter_mut().zip([(c"panel", 24), (c"activation", 32), (c"resign", 72)]) {
            let mut matches = fields.iter().filter(|field| field.name() == name);
            if matches.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(offset)
                || matches.next().is_some()
            {
                return Err("Reference SnippetChoice notification field differs".into());
            }
            *value = unsafe { ((controller + offset) as *const usize).read_volatile() };
        }
        let mut click = fields.iter().filter(|field| field.name() == c"click");
        if click.next().and_then(|field| usize::try_from(field.offset()).ok()) != Some(40)
            || click.next().is_some()
        {
            return Err("Reference SnippetChoice click field differs".into());
        }
        // The original bridge stores an inline NSObject and its Swift class
        // metadata in Any?. The two remaining payload words are padding.
        let slot = (controller + 40) as *const usize;
        let metadata = unsafe { slot.add(3).read_volatile() };
        let token = if metadata == 0 {
            0
        } else {
            #[link(name = "swiftCore")]
            unsafe extern "C" { fn swift_getObjCClassMetadata(class: usize) -> usize; }
            let address = swift_getObjCClassMetadata as *const () as usize;
            let imports = super::imports::observe_unpatched_main_imports(&[(c"swift_getObjCClassMetadata", address)])?;
            let [import] = imports.as_slice() else { return Err("Reference click metadata import absent".into()); };
            if import.cell_address() != base + 0x3c73a8 {
                return Err("Reference SnippetChoice click metadata binding differs".into());
            }
            reference_code(address, Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
            import.verify_unpatched()?;
            let class = super::metadata::loaded_class(c"NSObject")?;
            if unsafe { swift_getObjCClassMetadata(class.address()) } != metadata
                || unsafe { slot.add(3).read_volatile() } != metadata
            {
                return Err("Reference SnippetChoice click value or metadata changed".into());
            }
            let token = unsafe { slot.read_volatile() };
            if token == 0 {
                return Err("Reference SnippetChoice click token absent".into());
            }
            token
        };
        Ok((values, token))
    }

    pub(crate) fn verify_snippet_choice_deinit(&self) -> Result<()> {
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        for (offset, length, expected) in [
            (0x1a6484, 84, "a3f5a39b058c1c701c108c19d29176f9d7a90591c5f7b3ad4b74c565fd5d72e1"),
            (0x1a5750, 588, "15e927a5c362792034c551f8f6329e06ac34a67e831c400268f383b69a310fb9"),
            (0x1a599c, 44, "ab88cf06089597c059657a7cde32810937713f1b011700ef0915e43b022a2c88"),
            (0x1a59c8, 776, "f144c797bac82634e51231156b92e7a5422feefb9e7e484db24366f8b47475b5"),
        ] {
            self.verify_main_code(base.checked_add(offset).ok_or("SnippetChoice deinit code overflow")?, length, expected)?;
        }
        let class = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceController")?;
        let cell = base.checked_add(0x3f44c0).ok_or("SnippetChoice deinit cell overflow")?;
        if cell.checked_add(16) != Some(class.address())
            || reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3
            || Some(unsafe { (cell as *const usize).read_volatile() }) != base.checked_add(0x1a59c8)
        {
            return Err("Reference SnippetChoice deinit cell differs".into());
        }
        #[link(name = "swift_Concurrency")]
        unsafe extern "C" { fn swift_task_deinitOnExecutor(); }
        let address = swift_task_deinitOnExecutor as *const () as usize;
        let imports = super::imports::observe_unpatched_main_imports(&[(c"swift_task_deinitOnExecutor", address)])?;
        let [import] = imports.as_slice() else { return Err("Reference SnippetChoice deinit executor import absent".into()); };
        if base.checked_add(0x3c7840) != Some(import.cell_address()) {
            return Err("Reference SnippetChoice deinit executor cell differs".into());
        }
        reference_code(address, Path::new("/usr/lib/swift/libswift_Concurrency.dylib"))?;
        import.verify_unpatched()?;
        self.check_owner()
    }

    pub(crate) fn verify_input_deinit_cells(&self) -> Result<()> {
        self.cleanup_metadata_only()?;
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        for (offset, target) in [(0x3efd80, 0xf5df0), (0x3f0068, 0xf8a5c)] {
            let cell = base.checked_add(offset).ok_or("Reference input deinit cell overflow")?;
            let expected = base.checked_add(target).ok_or("Reference input deinit target overflow")?;
            if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3
                || unsafe { (cell as *const usize).read_volatile() } != expected
            {
                return Err("Original Swift input class deallocation entry differs".into());
            }
        }
        Ok(())
    }

    pub(crate) fn verify_snippet_expander_deinit_cell(&self) -> Result<()> {
        self.cleanup_metadata_only()?;
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        let cell = base.checked_add(0x3f4838).ok_or("SnippetExpander deinit cell overflow")?;
        let target = base.checked_add(0x1ae234).ok_or("SnippetExpander deinit target overflow")?;
        if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3
            || unsafe { (cell as *const usize).read_volatile() } != target
        {
            return Err("Original SnippetExpander deallocation entry differs".into());
        }
        let selector = base.checked_add(0x3e87a8).ok_or("SnippetExpander timer selector overflow")?;
        let invalidate = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::sel!(invalidate)) };
        if reference_mapping_protection(selector, size_of::<usize>())? & 5 != 1
            || unsafe { (selector as *const usize).read_volatile() } != invalidate
        {
            return Err("Original SnippetExpander timer selector differs".into());
        }
        Ok(())
    }

    fn verify_snippet_panel_destructor_layout(
        panel: super::metadata::ClassHandle, destructor_address: usize, cell: usize,
    ) -> Result<usize> {
        let class = unsafe { &*(panel.address() as *const objc2::runtime::AnyClass) };
        let destructor = super::metadata::method(panel, c".cxx_destruct", false)?
            .ok_or("Reference SnippetChoicePanel destructor absent")?;
        if destructor.owner() != panel.address() || destructor.implementation() != destructor_address
            || destructor.encoding() != b"v16@0:8"
        {
            return Err("Reference SnippetChoicePanel destructor metadata differs".into());
        }
        if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3 {
            return Err("Reference SnippetChoicePanel model offset mapping differs".into());
        }
        let fields = class.instance_variables();
        let [field] = fields.as_ref() else {
            return Err("Reference SnippetChoicePanel model ivar roster differs".into());
        };
        let offset = usize::try_from(field.offset()).map_err(|_| "Reference SnippetChoicePanel model offset differs")?;
        if field.name() != c"model" || offset < size_of::<usize>() || !offset.is_multiple_of(size_of::<usize>())
            || offset.checked_add(size_of::<usize>()).is_none_or(|end| end > class.instance_size())
            || unsafe { (cell as *const usize).read_volatile() } != offset
        {
            return Err("Reference SnippetChoicePanel model slot differs".into());
        }
        Ok(offset)
    }

    pub(crate) fn monitor_stop_offsets(&self) -> Result<[usize; 2]> {
        self.cleanup_metadata_only()?;
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        let mut values = [0; 2];
        for (value, offset) in values.iter_mut().zip([0x40c980, 0x4069e0]) {
            let cell = base.checked_add(offset).ok_or("Monitor stop offset cell overflow")?;
            if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3 {
                return Err("Monitor stop offset cell mapping differs".into());
            }
            *value = unsafe { (cell as *const usize).read_volatile() };
        }
        Ok(values)
    }
    pub(crate) fn app_state_poll_timer_offset(&self) -> Result<usize> {
        self.cleanup_metadata_only()?;
        let base = self.observe_app_delegate_offset_cells()?.main_header();
        let cell = base.checked_add(0x3fac98).ok_or("AppState timer offset cell overflow")?;
        if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1 {
            return Err("AppState timer offset cell mapping differs".into());
        }
        Ok(unsafe { (cell as *const usize).read_volatile() })
    }
    /// Bound member of the actual retained SDK RX image. The common issuer
    /// still proves the typed wrapper body, ABI, lifecycle and exact callsites.
    pub fn verify_sdk_code(&self,address:usize,bytes:usize,expected_sha256:&str)->Result<()> {
        self.cleanup_metadata_only()?;
        let result=(|| {
            if bytes==0 || bytes>65536 {return Err("AppDelegate SDK code span exceeds bound".into());}
            reference_hash(expected_sha256)?;
            let actual=reference_image_header(self.admission.sdk_addresses[0].1,&self.admission.sdk)?;
            if reference_image_header(address,&self.admission.sdk)?!=actual {
                return Err("AppDelegate wrapper is not the actual sealed SDK image".into());
            }
            reference_mapping(address,bytes,Some(true))?;
            let code=unsafe{std::slice::from_raw_parts(address as *const u8,bytes)};
            if digest(code)!=expected_sha256 {return Err("AppDelegate SDK code bytes differ".into());}
            self.check_owner()
        })();
        if result.is_err() {self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// Observe only public Foundation Notification data. There is no Swift
    /// metadata accessor, witness function, bridge, retain or destroy call here.
    #[cfg(target_arch="aarch64")]
    pub fn observe_foundation_notification_metadata(&self,point:FoundationNotificationAddressPoint)->Result<FoundationNotificationMetadataObservation> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        let result=(|| {
            reference_mapping_infrastructure()?;
            let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
            let symbol_address=std::ptr::addr_of!(FOUNDATION_NOTIFICATION_VN) as usize;
            let descriptor_address=std::ptr::addr_of!(FOUNDATION_NOTIFICATION_MN) as usize;
            let metadata_address=match point {
                FoundationNotificationAddressPoint::Symbol=>symbol_address,
                FoundationNotificationAddressPoint::AfterValueWitness=>symbol_address.checked_add(8).ok_or("Notification address point overflow")?,
            };
            let metadata_start=metadata_address.checked_sub(8).ok_or("Notification metadata prefix underflow")?;
            if symbol_address==0||descriptor_address==0||!metadata_address.is_multiple_of(8) {
                return Err("Notification exported data absent or unaligned".into());
            }
            // This reader is local to the fixed exports and their single VWT
            // pointer. It is not a public arbitrary-address mapping interface.
            let member=|address:usize,bytes:usize|->Result<(usize,[u8;16])> {
                if address==0||bytes==0 {return Err("Notification fixed data absent".into());}
                let end=address.checked_add(bytes).ok_or("Notification data extent overflow")?;
                let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
                if unsafe{libc::dladdr(address as *const libc::c_void,info.as_mut_ptr())}==0 {return Err("Notification Foundation image absent".into());}
                let info=unsafe{info.assume_init()};
                if info.dli_fname.is_null()||info.dli_fbase.is_null()
                    ||unsafe{std::ffi::CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation" {
                    return Err("Notification data has another actual provider".into());
                }
                let header=info.dli_fbase as usize;
                let count=unsafe{_dyld_image_count()};
                if count>4096 {return Err("Notification image count exceeds bound".into());}
                let indices=(0..count).filter(|i|unsafe{_dyld_get_image_header(*i) as usize==header}).collect::<Vec<_>>();
                if indices.len()!=1 {return Err("Notification Foundation image missing or duplicated".into());}
                let slide=unsafe{_dyld_get_image_vmaddr_slide(indices[0])};
                reference_mapping(header,32,None)?;
                let word=|at:usize|unsafe{(at as *const u32).read_unaligned()};
                let wide=|at:usize|unsafe{(at as *const u64).read_unaligned()};
                if word(header)!=0xfeedfacf||word(header+12)!=6 {return Err("Notification provider is not MachO64 dylib".into());}
                let (commands,size)=(word(header+16) as usize,word(header+20) as usize);
                if commands>4096||size>16*1024*1024 {return Err("Notification load-command bounds differ".into());}
                reference_mapping(header,32+size,None)?;
                let (mut cursor,mut uuid,mut members)=(0usize,None,0usize);
                for _ in 0..commands {
                    if cursor.checked_add(8).is_none_or(|n|n>size) {return Err("Notification load command truncated".into());}
                    let at=header+32+cursor;let (command,len)=(word(at),word(at+4) as usize);
                    if len<8||cursor.checked_add(len).is_none_or(|n|n>size) {return Err("Notification load-command length differs".into());}
                    if command==0x1b {
                        if len!=24||uuid.is_some() {return Err("Notification UUID command differs".into());}
                        let mut value=[0u8;16];value.copy_from_slice(unsafe{std::slice::from_raw_parts((at+8) as *const u8,16)});uuid=Some(value);
                    }
                    if command==0x19 {
                        if len<72 {return Err("Notification data segment truncated".into());}
                        let start=wide(at+24).checked_add_signed(slide as i64).ok_or("Notification segment slide overflow")?;
                        let last=start.checked_add(wide(at+32)).ok_or("Notification segment extent overflow")?;
                        if start<=address as u64&&end as u64<=last {
                            let prot=word(at+60);
                            if prot&1==0||prot&4!=0 {return Err("Notification segment is not readable nonexecuting data".into());}
                            members+=1;
                        }
                    }
                    cursor+=len;
                }
                if cursor!=size||members!=1 {return Err("Notification fixed data membership differs".into());}
                let uuid=uuid.filter(|value|*value!=[0;16]).ok_or("Notification actual provider UUID absent")?;
                let mut depth=0u32;let mut found=false;
                for _ in 0..16 {
                    let (mut start,mut len,mut region,mut words)=(address as u64,0u64,[0i32;12],12u32);
                    if unsafe{mach_vm_region_recurse(mach_task_self_,&mut start,&mut len,&mut depth,region.as_mut_ptr(),&mut words)}!=0
                        ||words!=12||start>address as u64||start.checked_add(len).is_none_or(|n|n<end as u64) {
                        return Err("Notification complete fixed data region absent".into());
                    }
                    if region[8]!=0 {depth=depth.checked_add(1).ok_or("Notification region submap overflow")?;continue;}
                    if region[0]&1==0||region[0]&4!=0 {return Err("Notification region is not readable nonexecuting data".into());}
                    found=true;break;
                }
                if !found {return Err("Notification complete data submap absent".into());}
                self.check_owner()?;
                Ok((header,uuid))
            };
            let provider=member(metadata_start,24)?;
            let descriptor_provider=||->Result<()> {
                // Mn is address-only nominal descriptor identity. Its constant
                // storage may be in executable __TEXT; no descriptor byte is
                // read and no executable permission is issued for this label.
                reference_mapping(descriptor_address,1,None)?;
                let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
                if unsafe{libc::dladdr(descriptor_address as *const libc::c_void,info.as_mut_ptr())}==0 {return Err("Notification descriptor image absent".into());}
                let info=unsafe{info.assume_init()};
                if info.dli_fname.is_null()||info.dli_fbase as usize!=provider.0
                    ||unsafe{std::ffi::CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation" {
                    return Err("Notification descriptor has another actual provider".into());
                }
                self.check_owner()
            };
            descriptor_provider()?;
            if member(symbol_address,1)?!=provider {return Err("Notification exported metadata provider differs".into());}
            let metadata=||[
                unsafe{(metadata_start as *const u64).read_volatile()},
                unsafe{(metadata_address as *const u64).read_volatile()},
                unsafe{((metadata_address+8) as *const u64).read_volatile()},
            ];
            let first_metadata=metadata();
            if first_metadata[1]!=0x200||first_metadata[2]!=descriptor_address as u64 {return Err("Notification named struct metadata shape differs".into());}
            let table_address=usize::try_from(first_metadata[0]).map_err(|_|"Notification VWT address width")?;
            if !table_address.is_multiple_of(8)||member(table_address,88)?!=provider {return Err("Notification VWT provider/alignment differs".into());}
            let table=||{
                let mut value=[0u8;88];
                for (i,byte) in value.iter_mut().enumerate(){*byte=unsafe{((table_address+i) as *const u8).read_volatile()};}
                value
            };
            let first_table=table();
            let destroy_address=usize::try_from(u64::from_le_bytes(first_table[8..16].try_into().map_err(|_|"Notification destroy word width")?)).map_err(|_|"Notification destroy address width")?;
            let destroy=super::metadata::code_identity(destroy_address)?;
            reference_mapping(destroy_address,1,Some(true))?;
            // Fresh retained-image/source checks precede the final pure DATA
            // snapshot. No business guard or foreign Swift call follows it.
            self.cleanup_metadata_only()?;
            descriptor_provider()?;
            if member(metadata_start,24)?!=provider||member(symbol_address,1)?!=provider
                ||member(table_address,88)?!=provider {
                return Err("Notification data provider changed".into());
            }
            let final_metadata=metadata();let final_table=table();
            if final_metadata!=first_metadata||final_table!=first_table
                ||super::metadata::code_identity(destroy_address)?!=destroy
                ||super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                return Err("Notification metadata/table/code owner changed".into());
            }
            reference_mapping(destroy_address,1,Some(true))?;
            self.check_owner()?;
            Ok(FoundationNotificationMetadataObservation{symbol_address,metadata_address,descriptor_address,
                image_header:provider.0,uuid:provider.1,metadata_words:final_metadata,table_address,table:final_table,destroy})
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// Fixed consumer ranges only. This verifies readable non-executable
    /// libswiftCore storage; writable storage is not asserted immutable. The
    /// Native issuer separately proves the named global/GOT/source binding.
    pub fn observe_swift_singleton_region(&self,kind:SwiftSingletonDataKind,address:usize)->Result<SwiftSingletonRegionObservation> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        let result=(|| {
            reference_mapping_infrastructure()?;
            let kernel=super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?;
            let bytes=match kind {SwiftSingletonDataKind::EmptySet=>1,SwiftSingletonDataKind::EmptyArray=>24};
            let end=address.checked_add(bytes).ok_or("Swift singleton range overflow")?;
            if address==0 {return Err("Swift singleton address absent".into());}
            let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
            if unsafe{libc::dladdr(address as *const libc::c_void,info.as_mut_ptr())}==0 {
                return Err("Swift singleton image absent".into());
            }
            let info=unsafe{info.assume_init()};
            if info.dli_fname.is_null() || info.dli_fbase.is_null()
                || unsafe{std::ffi::CStr::from_ptr(info.dli_fname)}.to_bytes()!=b"/usr/lib/swift/libswiftCore.dylib" {
                return Err("Swift singleton is not actual libswiftCore storage".into());
            }
            let header=info.dli_fbase as usize;
            let image_offset=address.checked_sub(header).ok_or("Swift singleton precedes image")?;
            let count=unsafe{_dyld_image_count()};
            if count>4096 {return Err("Swift singleton image count exceeds bound".into());}
            let indices=(0..count).filter(|index|unsafe{_dyld_get_image_header(*index) as usize==header}).collect::<Vec<_>>();
            if indices.len()!=1 {return Err("Swift singleton loaded image missing or duplicate".into());}
            let slide=unsafe{_dyld_get_image_vmaddr_slide(indices[0])};
            reference_mapping(header,32,None)?;
            let word=|at:usize|unsafe{(at as *const u32).read_unaligned()};
            let wide=|at:usize|unsafe{(at as *const u64).read_unaligned()};
            if word(header)!=0xfeedfacf || word(header+12)!=6 {return Err("Swift singleton MachO kind differs".into());}
            let (commands,size)=(word(header+16) as usize,word(header+20) as usize);
            if commands>4096 || size>16*1024*1024 {return Err("Swift singleton load command bound".into());}
            reference_mapping(header,32+size,None)?;
            let (mut cursor,mut uuid,mut member)=(0usize,None,0usize);
            for _ in 0..commands {
                if cursor.checked_add(8).is_none_or(|n|n>size) {return Err("Swift singleton command truncated".into());}
                let at=header+32+cursor;
                let (command,len)=(word(at),word(at+4) as usize);
                if len<8 || cursor.checked_add(len).is_none_or(|n|n>size) {return Err("Swift singleton command length differs".into());}
                if command==0x1b {
                    if len!=24 || uuid.is_some() {return Err("Swift singleton UUID command differs".into());}
                    let mut value=[0u8;16];
                    value.copy_from_slice(unsafe{std::slice::from_raw_parts((at+8) as *const u8,16)});uuid=Some(value);
                }
                if command==0x19 {
                    if len<72 {return Err("Swift singleton segment truncated".into());}
                    let start=wide(at+24).checked_add_signed(slide as i64).ok_or("Swift singleton slide overflow")?;
                    let last=start.checked_add(wide(at+32)).ok_or("Swift singleton segment overflow")?;
                    if start<=address as u64 && end as u64<=last {
                        let protection=word(at+60);
                        if protection&1==0 || protection&4!=0 {return Err("Swift singleton segment is not readable data".into());}
                        member=member.checked_add(1).ok_or("Swift singleton segment count overflow")?;
                    }
                }
                cursor+=len;
            }
            if cursor!=size || member!=1 {return Err("Swift singleton data membership differs".into());}
            let uuid=uuid.filter(|value|*value!=[0;16]).ok_or("Swift singleton UUID absent")?;
            let mut depth=0u32;let mut found=false;
            for _ in 0..16 {
                let (mut start,mut len,mut region,mut words)=(address as u64,0u64,[0i32;12],12u32);
                if unsafe{mach_vm_region_recurse(mach_task_self_,&mut start,&mut len,&mut depth,region.as_mut_ptr(),&mut words)}!=0
                    || words!=12 || start>address as u64 || start.checked_add(len).is_none_or(|n|n<end as u64) {
                    return Err("Swift singleton complete region absent".into());
                }
                if region[8]!=0 {depth=depth.checked_add(1).ok_or("Swift singleton submap overflow")?;continue;}
                if region[0]&1==0 || region[0]&4!=0 {return Err("Swift singleton region is not readable nonexecuting data".into());}
                found=true;break;
            }
            if !found || super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?!=kernel {
                return Err("Swift singleton region/infrastructure changed".into());
            }
            self.check_owner()?;
            Ok(SwiftSingletonRegionObservation{uuid,image_offset,address,kind})
        })();
        if result.is_err(){self.code_retired.store(true,std::sync::atomic::Ordering::Release);}
        result
    }
    /// Pure ownership/data check against the existing allocation ledger, not
    /// permission to run ARC, adopt a pointer or mark an initializer finished.
    /// No business/model lease is consulted: a known partial may need cleanup
    /// after retirement. The class-local issuer still qualifies its exact class,
    /// original entry, phase-specific release path and physical construction +1.
    pub fn verify_partial_ledger(&self,identity:&crate::workspace::receiver::SourceConstructionIdentity<'_>)->Result<()> {
        self.check_owner()?;
        let pure=|| {
            self.check_owner()?;
            if !identity.belongs_to(&self.model) || identity.origin_thread()!=self.owner
                || !matches!(identity.phase(),crate::workspace::receiver::ReceiverPhase::Allocated
                    | crate::workspace::receiver::ReceiverPhase::SourceIvarsObserved) {
                return Err("AppDelegate partial allocation model/owner/phase differs".into());
            }
            identity.verify_allocation()?;
            self.check_owner()
        };
        pure()?;
        self.cleanup_metadata_only()?;
        pure()
    }
    pub fn after_dynamic(&self)->Result<()> {self.before_init()}
    /// Exact existing issuer/model identity for the genuine retained root.
    /// This binds custody only; it supplies no Input callback or ARC permission.
    pub fn verify_admission_binding(&self,admission:&Arc<ReferenceAdmission>,model:&Arc<crate::workspace::Workspace>)->Result<()> {
        self.check_owner()?;
        self.cleanup_metadata_only()?;
        if !Arc::ptr_eq(&self.admission,admission) || !Arc::ptr_eq(&self.model,model) {
            return Err("AppDelegate keeper admission/model identity differs".into());
        }
        self.verify_model(model)?;
        self.check_owner()
    }
    pub fn verify_model(&self,model:&Arc<crate::workspace::Workspace>)->Result<()> {
        self.check_owner()?;
        if !Arc::ptr_eq(model,&self.model) {return Err("AppDelegate constructor model identity differs".into());}
        self.before_init()
    }
    /// Locators only; callers must also qualify the whole constructor/lifecycle.
    pub fn constructor_source(&self)->&super::SourceCallsite {&self.constructor_source}
    pub fn callback_source(&self)->&super::SourceCallsite {&self.callback_source}
}
impl ReferenceAdmission {
    pub fn life(&self) -> Arc<dyn AliveLease> { self.admitted.life.clone() }
    pub fn descriptors(&self) -> Arc<DescriptorTable> { self.admitted.descriptors.clone() }
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub fn app_delegate_context(self:&Arc<Self>,model:Arc<crate::workspace::Workspace>)->Result<ReferenceAppDelegateContext> {
        if super::process_fork::is_child() {unsafe{libc::_exit(78)}}
        model.verify_reference_context(self)?;
        let constructor_source=super::bootstrap::source_allocation_callsites().into_iter()
            .find(|source|source.file=="Sources/RuycastApp/main.swift"
                && source.operation=="AppDelegate() at main219: objc_allocWithZone/selected init; same Reference source receiver")
            .ok_or("AppDelegate exact constructor source declaration absent")?;
        let callback_source=crate::workspace::source_roster().into_iter()
            .find(|source|source.file=="reference-0.8.26/Sources/RuycastApp/main.swift"
                && source.operation=="line 158: @MainActor final class AppDelegate:NSObject,NSApplicationDelegate {")
            .ok_or("AppDelegate exact callback class declaration absent")?;
        let context=ReferenceAppDelegateContext{admission:self.clone(),model,constructor_source,callback_source,
            process:unsafe{libc::getpid()},owner:std::thread::current().id(),code_retired:std::sync::atomic::AtomicBool::new(false),
            callback_sources:CallbackSourceObservation::default()};
        context.before_init()?;
        Ok(context)
    }
    /// Only the actual early Registry constructor can bind this retirement
    /// lineage. No callback/source authority is issued by the binding.
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn bind_registry_retirement(&self,retired:&Arc<std::sync::atomic::AtomicBool>)->Result<()> {
        self.before_reference_initialization()?;
        self.registry_retired.set(Arc::downgrade(retired))
            .map_err(|_|"Reference admission already bound its one Registry")?;
        self.registry_current()
    }
    #[cfg(rcf1_reference_sdk)]
    fn registry_current(&self)->Result<()> {
        if let Some(weak)=self.registry_retired.get() {
            let actual=weak.upgrade().ok_or("Reference early Registry was destroyed")?;
            reference_retirement_tail(&actual)?;
        }
        Ok(())
    }
    /// Preparation only: bind an actual installed input module context to the
    /// sealed Reference roots/guardian. No callback, userdata or Native token
    /// authority is created by a successful context identity check.
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_input_context(&self,context:&super::owned::ModuleContext)->Result<()> {
        self.before_reference_initialization()?;
        self.input_context_identity(context)?;
        self.before_reference_initialization()?;
        self.input_context_identity(context)
    }
    /// Preflight the original Edit-command singleton before creating any
    /// receiver. This verifies source/layout only and publishes no allocation.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_native_edit_construction_source(&self) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            // Symbol address only. This verifier never calls Swift ARC.
            fn swift_release();
        }
        self.before_reference_initialization()?;
        let source = include_bytes!("../../../reference/0.8.26/Sources/RuycastApp/NativeEditCommands.swift");
        let path = self.root.path().join("bench/reference/0.8.26/Sources/RuycastApp/NativeEditCommands.swift");
        let bytes = self.root.read_optional(Path::new(&self.grant.source_manifest.path), 2 * 1024 * 1024)?
            .ok_or("Reference Edit-command source manifest absent")?;
        if digest(&bytes) != self.grant.source_manifest.sha256 {
            return Err("Reference Edit-command source manifest differs".into());
        }
        let manifest: HostSourceManifest = serde_json::from_slice(&bytes)
            .map_err(|_| "Reference Edit-command source manifest schema")?;
        let path = path.to_str().ok_or("Reference Edit-command source path encoding")?;
        let mut rows = manifest.sources.iter().filter(|row| row.path == path);
        if rows.next().is_none_or(|row| row.sha256 != digest(source)) || rows.next().is_some() {
            return Err("Reference Edit-command source compiler member differs".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference Edit-command offset overflow");
        for (offset, length, sha256) in [
            (0x14439c, 92, "5056eb60ca21093637ad9cee2213a22f474cfab8dc347e4bc15a19cac984aec3"),
            (0x9b82c, 8, "24d6a167e99eb78d291e641673c2c5fd8661e18285942fe8352ad2238b539048"),
            (0x14593c, 20, "ee13a2093486cf96e77d9ccafae0eeadb3655d4b53c64c01de48fd0b324d40ee"),
            (0x1444d4, 52, "b686b642514fb93b5ec854c639cbc27d8d1a67bfd8e86044c86c37e62c2db9e5"),
            (0x2fbaec, 12, "cfcb0bf56e07e38ed89130ff04f8c5eeed1dd8e1ec7378d52306d6f28a1e18dd"),
            (0x2fbb10, 12, "d2bc9ae552d79df7b57b9087552ded25b98e3b8ce50054812ca2edffef979f31"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
            // Exact editor getter, six action families and menu validator.
            // Observing their entry/body range grants no callback dispatch.
            (0x145010, 0x8a8, "9ec565a41ff1a6ca38e6553c10f3e8650222de7b6c2675ddaee07a208975fde7"),
        ] {
            let address = at(offset)?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference Edit-command code outside Main".into());
            }
            reference_mapping(address, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != sha256 {
                return Err(format!("Reference Edit-command compiler body differs at {offset:#x}"));
            }
        }
        let release = super::imports::observe_unpatched_main_imports(&[
            (c"swift_release", swift_release as *const () as usize),
        ])?;
        let [release] = release.as_slice() else {
            return Err("Reference Edit-command destructor requires one exact Swift release import".into());
        };
        if release.cell_address() != at(0x3c7438)? {
            return Err("Reference Edit-command destructor release import cell differs".into());
        }
        reference_code(release.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
        release.verify_unpatched()?;

        let class = super::metadata::registered_class(at(0x3ea500)?)?;
        for (selector, offset, encoding) in [
            (c"undo:", 0x14509c, b"v24@0:8@16"),
            (c"redo:", 0x1451ac, b"v24@0:8@16"),
            (c"paste:", 0x14529c, b"v24@0:8@16"),
            (c"cut:", 0x1453b0, b"v24@0:8@16"),
            (c"copy:", 0x145498, b"v24@0:8@16"),
            (c"selectAll:", 0x145588, b"v24@0:8@16"),
            (c"validateMenuItem:", 0x14580c, b"B24@0:8@16"),
        ] {
            let method = super::metadata::method(class, selector, false)?
                .ok_or("Reference Edit-command callback absent")?;
            if method.owner() != class.address() || method.implementation() != at(offset)?
                || method.encoding() != encoding
            {
                return Err(format!("Reference Edit-command selected {selector:?} differs"));
            }
        }
        let destructor = super::metadata::method(class, c".cxx_destruct", false)?
            .ok_or("Reference Edit-command destructor absent")?;
        if destructor.owner() != class.address() || destructor.implementation() != at(0x14593c)?
            || destructor.encoding() != b"v16@0:8"
        {
            return Err("Reference Edit-command selected destructor differs".into());
        }
        if super::metadata::method(class, c".cxx_construct", false)?.is_some() {
            return Err("Reference Edit-command allocation would run a CXX constructor".into());
        }
        let class = unsafe { &*(class.address() as *const objc2::runtime::AnyClass) };
        let fields = class.instance_variables();
        let offset = at(0x3f2730)?;
        if reference_mapping_protection(offset, size_of::<usize>())? & 7 != 3
            || unsafe { (offset as *const usize).read_volatile() } != 8
            || class.instance_size() != 24
            || class.superclass().map(|class| class.name()) != Some(c"NSObject")
            || fields.len() != 1 || fields[0].name() != c"window" || fields[0].offset() != 8
        {
            return Err("Reference Edit-command class layout differs".into());
        }
        crate::workspace::factory::RootDeallocator::capture()?.verify_inherited_dealloc(class)?;
        let root = super::metadata::loaded_class(c"NSObject")?;
        let init = super::metadata::method(root, c"init", false)?
            .ok_or("Reference Edit-command superclass initializer absent")?;
        if init.owner() != root.address() || init.encoding() != b"@16@0:8" {
            return Err("Reference Edit-command superclass initializer owner or ABI differs".into());
        }
        reference_code(init.implementation(), Path::new("/usr/lib/libobjc.A.dylib"))?;
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_native_edit_super_import(&self, installed: bool) -> Result<()> {
        let spec = crate::workspace::factory::reference_edit_super_spec();
        let rows = super::observe_main_imports(&[spec])?;
        let [row] = rows.as_slice() else {
            return Err("Reference Edit requires one exact super-init import".into());
        };
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let expected = if installed { spec.replacement } else { spec.original };
        reference_code(spec.original, Path::new("/usr/lib/libobjc.A.dylib"))?;
        reference_code(spec.replacement, &self.sdk)?;
        if base.checked_add(0x3c6948) != Some(row.cell_address()) || row.selected_address() != expected {
            return Err("Reference Edit super-init import differs".into());
        }
        let zone = super::bootstrap::source_allocation_with_zone_import_spec();
        let zone_rows = super::observe_main_imports(&[zone])?;
        let [row] = zone_rows.as_slice() else {
            return Err("Reference Edit requires one exact zone allocation import".into());
        };
        let expected_zone = if installed { zone.replacement } else { zone.original };
        reference_code(zone.original, Path::new("/usr/lib/libobjc.A.dylib"))?;
        reference_code(zone.replacement, &self.sdk)?;
        if base.checked_add(0x3c6930) != Some(row.cell_address())
            || row.selected_address() != expected_zone
        {
            return Err("Reference Edit zone allocation import differs".into());
        }
        crate::workspace::factory::prepare_reference_edit_super(
            base.checked_add(0x1443e0).ok_or("Reference Edit super-init caller overflow")?,
            base.checked_add(0x1443b8).ok_or("Reference Edit allocation caller overflow")?,
            base.checked_add(0x14c7d4).ok_or("Reference PalettePanel super-init caller overflow")?,
            base.checked_add(0x14c628).ok_or("Reference PalettePanel key fallback caller overflow")?,
            base.checked_add(0x1a2ee4).ok_or("Reference SnippetChoicePanel super-init caller overflow")?,
        )
    }

    /// The coalesced workspace clipboard scope requires the actual original
    /// compiler members, not the separate Native caller roster.
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_clipboard_source_members(&self)->Result<()> {
        self.before_reference_initialization()?;
        self.verify_clipboard_manifest_members()?;
        self.before_reference_initialization()
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_clipboard_manifest_members(&self)->Result<()> {
        let bytes=self.root.read_optional(Path::new(&self.grant.source_manifest.path),2*1024*1024)?
            .ok_or("Reference clipboard compiler manifest absent")?;
        if digest(&bytes)!=self.grant.source_manifest.sha256 {return Err("Reference clipboard compiler manifest changed".into());}
        let manifest:HostSourceManifest=serde_json::from_slice(&bytes).map_err(|_|"Reference clipboard manifest schema")?;
        for source in super::clipboard::reference_source_roster() {
            let path=self.root.path().join(source.file);
            let path=path.to_str().ok_or("Reference clipboard source path encoding")?;
            let mut rows=manifest.sources.iter().filter(|row|row.path==path);
            let row=rows.next().ok_or("Reference clipboard caller not compiler-bound")?;
            if rows.next().is_some()||row.sha256!=source.source_sha256 {return Err("Reference clipboard compiler member differs/duplicated".into());}
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_open_dispatch_spec() -> super::ImportSpec {
        #[link(name = "swiftDispatch")]
        unsafe extern "C" {
            #[link_name = "$sSo17OS_dispatch_queueC8DispatchE5async5group3qos5flags7executeySo0a1_b1_F0CSg_AC0D3QoSVAC0D13WorkItemFlagsVyyXBtF"]
            static DISPATCH_ASYNC: u8;
        }
        super::ImportSpec {
            symbol: c"$sSo17OS_dispatch_queueC8DispatchE5async5group3qos5flags7executeySo0a1_b1_F0CSg_AC0D3QoSVAC0D13WorkItemFlagsVyyXBtF",
            original: (&raw const DISPATCH_ASYNC) as usize,
            replacement: crate::workspace::callbacks::reference_dispatch_frontdoor_address(),
        }
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_main_tabs_deadline_spec() -> super::ImportSpec {
        #[link(name = "swiftDispatch")]
        unsafe extern "C" {
            #[link_name = "$sSo17OS_dispatch_queueC8DispatchE10asyncAfter8deadline3qos5flags7executeyAC0D4TimeV_AC0D3QoSVAC0D13WorkItemFlagsVyyXBtF"]
            static MAIN_TABS_DEADLINE: u8;
        }
        super::ImportSpec {
            symbol: c"$sSo17OS_dispatch_queueC8DispatchE10asyncAfter8deadline3qos5flags7executeyAC0D4TimeV_AC0D3QoSVAC0D13WorkItemFlagsVyyXBtF",
            original: (&raw const MAIN_TABS_DEADLINE) as usize,
            replacement: crate::workspace::callbacks::reference_main_tabs_deadline_address(),
        }
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_control_wait_spec() -> super::ImportSpec {
        #[link(name = "swiftDispatch")]
        unsafe extern "C" {
            #[link_name = "$sSo21OS_dispatch_semaphoreC8DispatchE4wait7timeoutAC0D13TimeoutResultOAC0D4TimeV_tF"]
            static CONTROL_REPLY_WAIT: u8;
        }
        super::ImportSpec {
            symbol: c"$sSo21OS_dispatch_semaphoreC8DispatchE4wait7timeoutAC0D13TimeoutResultOAC0D4TimeV_tF",
            original: (&raw const CONTROL_REPLY_WAIT) as usize,
            replacement: crate::workspace::callbacks::reference_control_wait_address(),
        }
    }

    /// Observe the two compiler-generated open completion layouts in the
    /// admitted Main. This does not inspect or retain any live block.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_open_completion_source(&self, installed: bool) -> Result<()> {
        #[link(name = "Foundation", kind = "framework")]
        unsafe extern "C" {
            #[link_name = "$sSa10FoundationE19_bridgeToObjectiveCSo7NSArrayCyF"]
            static ARRAY_BRIDGE: u8;
            #[link_name = "$s10Foundation3URLV19_bridgeToObjectiveCSo5NSURLCyF"]
            static URL_BRIDGE: u8;
        }
        // Address-only binding: this is a Swift method, not a callable C ABI.
        #[link(name = "swiftDispatch")]
        unsafe extern "C" {
            #[link_name = "$sSo17OS_dispatch_queueC8DispatchE4mainABvgZ"]
            static DISPATCH_MAIN: u8;
        }
        #[link(name = "swiftDarwin")]
        unsafe extern "C" {
            #[link_name = "$s6Darwin5fcntlys5Int32VAD_ADtF"]
            static SHORTCUTS_GETFL: u8;
            #[link_name = "$s6Darwin5fcntlys5Int32VAD_A2DtF"]
            static SHORTCUTS_SETFL: u8;
        }
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            #[link_name = "swift_allocObject"]
            static SWIFT_ALLOC_OBJECT: u8;
            // Address-only labels; Swift ownership functions are not called
            // through these declarations.
            fn swift_retain();
            fn swift_release();
            fn swift_errorRetain();
            fn swift_errorRelease(error: usize);
            fn swift_deallocObject();
        }
        #[link(name = "System")]
        unsafe extern "C" {
            #[link_name = "_Block_copy"]
            static BLOCK_COPY: u8;
            #[link_name = "_Block_release"]
            static BLOCK_RELEASE: u8;
        }

        self.before_reference_initialization()?;
        let source = self.root.path().join("bench/reference/0.8.26/Sources/RuycastApp/AppState.swift");
        let source_path = source.to_str().ok_or("Reference open source path encoding")?;
        let manifest_bytes = self.root.read_optional(Path::new(&self.grant.source_manifest.path), 2 * 1024 * 1024)?
            .ok_or("Reference open compiler manifest absent")?;
        if digest(&manifest_bytes) != self.grant.source_manifest.sha256 {
            return Err("Reference open compiler manifest changed".into());
        }
        let manifest: HostSourceManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| "Reference open compiler manifest schema")?;
        let mut members = manifest.sources.iter().filter(|row| row.path == source_path);
        let member = members.next().ok_or("Reference AppState compiler member absent")?;
        if members.next().is_some()
            || member.sha256 != "ffba512c44134bf74005bd6cc409c1c92d09b4a13ee9709ca60a7b6e895abd14"
        {
            return Err("Reference open compiler member differs".into());
        }

        reference_mapping_infrastructure()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference open image address overflow");
        for (offset, length, expected) in [
            (0x40d2c, 268, "f24476c09b91370612671c36736dd2dc583075ad723406af048f46abfeb5ab87"),
            (0x5a20c, 116, "d5f87d5f49903a96091fc21a0e5e044cb5fdc5a355e55847529ed916a34153a9"),
            (0x2fa724, 12, "68fa0d23e74c9af06b535c022ce660f538c6696f79d57c25fd5703d09dd4fd2f"),
            (0x2f8e64, 12, "55140bbf376e2582df6028bc12c615724c3b7e575abc59800dbc40566f53d477"),
            (0x5a29c, 152, "0a21b11a840329be8845e1fc4c8c069ef48b935c89a2a83881e6e36fc61da64a"),
            (0x40fa4, 120, "db82858217f73dfe05c6b1dfbb662d83d8c1c0100b72a4a3161678169906a22d"),
            (0x7009c, 24, "21fa29f34ea4622fd3d82d4a4d180cdfcb949003689188fa8db756be0abe2704"),
            (0x6cd48, 12, "09c3662a25b7a2a51a2e3577f12ed1d15f84df61b70f760582b2b39a143bec7a"),
            // AppState.execute and both specialized forms construct the
            // completion context consumed by the shared Swift body.
            (0x509ec, 92, "17c9518399cd2c99c9ae0c3a7df4b6849fb1644079464cd18ed0d95b40998b14"),
            (0x55894, 88, "3d8a2fff9cc7b90bf563b556d4314569544b1455f5e7fdfee10b9023004fd7ee"),
            (0x59b58, 72, "0c92654dc5b30570d0216108d044d83cd22d36fbcb1530e608ef7aea87659811"),
            (0x6cd54, 12, "c1630e324254ef5b06878e20e6957312abc9c4215962aa7ebdedcbb398d2f158"),
            (0x702e8, 4, "5376dde71b6521d404c932f9e330c7b399bf122f483b863ea0435ad474f3de0f"),
            (0x6ccd8, 4, "b7cd92b19a0eed3364b172de1d7ea1b9ca9967aabfa5aa2ada17fe01938d736f"),
            (0x7033c, 4, "9cb97d5c69e29f7b7e08a47f64c3284d142347e0d46cdec694e6cf5ebbdab7d3"),
            // The original and specialized execute callers allocate the
            // retained 0x90-byte finish context before capturing it.
            (0x59624, 88, "127ed8bedeaf825e16d98ba9fdceec28a03a380e91466a59e1e294621b5f625e"),
            (0x50254, 92, "5ab0ab359412ea801de18cd1f6ff18f2535472c919446e41234cac2bcd1a7500"),
            (0x54fc8, 120, "b022f0746306a21c404b097ce8cafeec516cb0f64ac6ebc9df9a108a2a4eb120"),
            (0x6f3ac, 68, "54398bd9acf44060b18e8db2ae38acc90ad6d7dd0564b292e51659ad1ae0f787"),
            (0x5e464, 452, "892528debeb39939540388cd4a830ef097fa82f59ce1c7f6e30c98672ecba611"),
            (0x40e38, 364, "2423c23cb34997d06856931d736fdee44a304081de0ae585c94ace31a16f8b50"),
            (0x5eabc, 364, "70b3f736b2c00f4b012cd43f1781b95f10c36925d16208157bfe73d209f35d02"),
            // The second blocks run the original MainActor continuations.
            (0x33e24, 44, "b40f91eec25c42bbdb8846a5929e4e7037ae0cb1a205d55056f97ba78982c624"),
            (0x700e8, 152, "48d1660a39b6ca4ce98e65967d920ac485bf659d215abdeaf5aa93c27e256330"),
            (0x6ea4c, 564, "bfdcfdb43b5d91876c5ee6f8aacde1711b47a4d84a3f39b80d8a29d8a8f2c6e9"),
            // Both outer bodies reach these helpers to allocate the 40-byte
            // continuation context, copy the second block and fetch the queue.
            (0x70958, 12, "e8c322f293d59a02b7c601071d2f8f06049b916b8d1c757e200477205560d4c1"),
            (0x70d0c, 12, "1fcbd4200a7be59a80e1cdf87255efdb2bfefdbd5e05726ea7d8049ef9c98a10"),
            (0x70fcc, 8, "eb90a2184d0ef542aed29232fbb10b26c44d60225b5d5696f71b047cfa10b8f2"),
            // Destruction of the second Swift context, including its owned
            // error and captured continuation, after the block drops its +1.
            (0x700a8, 64, "36a2c332eb280224aef60f643687da30f45970670e18e702a233863f49663c1a"),
            (0x6ea34, 24, "c97dbc12323793725e444f09771ede25c633825fb4e9e40d3a1ab192ad69302a"),
            (0x70010, 68, "8c8853dbdc5c90cd2010b94d03d99f4aa9a0abc282be816377608d18ea16a9dc"),
            (0x72020, 12, "2364d65821be5b341813f50654c2de81da6b3b3ee4fa018bdfa031d570097287"),
            (0x70de8, 8, "2fd661a55f7333b31221c25cd5d42ea842a959939c47a4a24845ce2d7409d234"),
            (0x2fbffc, 12, "9a43a65f2d464d3c961ffc9ed50d0c2291c341b94efdfd377866c0c6743f54ea"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
            (0x2fc1c4, 12, "b681305e25fa18b584abf72b26653eb8a3fab78666524b51f2d98df7a2e73e01"),
            (0x2fc074, 12, "372bfc0596a93ef85665b2964d9eac358efa6cfcd4445bfbc52f3aa4e445ad62"),
            (0x2fa88c, 12, "f96db3434b40ec918497d87f80e3d3ec899c0265e56b9ec2eb1fc94384c3acce"),
            (0x2fb4c8, 12, "a0cd8bc83353f7b1ee46e70a379e2aea695c74182f3aaf86c6a3f60328002f3e"),
            (0x2fb4d4, 12, "92f7f8117bbcc2d1b400e9e500eb34c6b35145adadaa014d2f82107494426a46"),
            (0x2fbf54, 12, "7d426483accac52771625b86d33ba8ad37b8ddb88307529c4203dfba4411ac48"),
            (0x71c58, 12, "2e524ff7d2b5aafd59639f5f6ab63083b1cdcb6714746169d19051dbd19a0dfd"),
            (0x2fa8a4, 12, "b00abb9c11b97856b625520940525bdb6f878f8e791ab28f59b187fc8080f0ef"),
            // The same Dispatch cell serves the monitor's worker and main
            // submissions. Bind the compiler bodies that construct each block.
            (0x61de8, 172, "20a08f122f64327a8f85fba4c8b0029c0d142c1113a1b467f73774cb579f795b"),
            (0x6e234, 12, "f3413b8283e33e030ca64373b96170c3cb9b5367407f85809c535bfe87b1d6ad"),
            (0x64e68, 472, "14f4821849187019fa337a27abe8587fee31c870897f1ea4cb49b392b4db3e09"),
            (0x1ba008, 320, "ed46af398ec083569ff8916d1a27aa15d4e13379a39be164f6cffd99c68f9c14"),
            (0x1ba4f8, 12, "1e9e0b1e0e6bc481b7389763d198529577b3131efbcbaf18880de33a57ca3431"),
            (0x1ba254, 564, "b0822b9e85597224c1d9dca903f101e339d1df079a4ce2de54e91a1e68f4ae59"),
            (0x6e220, 20, "13cf2c8c20d09c254d250cd366faf8980024c78189615076318a9e89637c3175"),
            (0x6e4b8, 140, "8e263fb3fc983f3b89363c444cc510289854c237269d569b29a9d616fd8fe5d0"),
            (0x1ba4d0, 40, "668758583562faf299241bd13dbc4872c0d71224f675f512869c5f07d07f8b64"),
            (0x1c4680, 152, "666b89ec9b3440926aa58a58f39bbd6be1f5166df499d0a42ae8269cf2d064eb"),
            (0x70304, 8, "3bb0c69c338ba3aa0d2712dd16668d35b6242612bf5b1f78840fe9bfb5c98d30"),
            (0x1ba8b0, 8, "563dc6d6b17dd52123155d5eaf1d47c35d883f0df1a527d4075f858dd4cdf1fe"),
            (0x1c474c, 4, "d7eedd83d9e24009352d097fd2e199fcc67fcba08b1b32fbec9f3981b601574e"),
            // Both original BrowserTabs cache invalidations retain SystemServices.
            (0x1caa94, 540, "92cd8cfc9aff1c65d3e8f46e08755cc36082ffae6749817430588eb74715c69a"),
            (0x1cf1ec, 88, "1cd6c9ddfcba70e901466d5a032dd5dc4a839c84c91394c8472015318478a6c3"),
            (0x1d3d18, 16, "5791b18295878a852bbbb534365e020efadad9acb8b4620ded6daf8949a4fcec"),
            (0x1d3d54, 8, "f9bc843a4144e17d8b82ac9ec359d3a946df758c06a95043b895535ae3a9e908"),
            // Shared Shortcuts/process invalidation body, both callback thunks
            // and the weak-capture destructor and block ownership helpers.
            (0x1ce5f0, 460, "10d8e8cca246a0e58ba2252395aa744ca41cff67bfd1670452b6cf679d0cf0e2"),
            (0x1ce7bc, 100, "cf56bb374124d2bcb62eb4610fa16901c92270ee86eb0b1a94c3f8bc21bb4896"),
            (0x1d2654, 80, "f5b272c94812980290531dd291a0f48b85a578ba9bc031f6f0769cfcddcade2c"),
            (0x1cf15c, 28, "944e206c021d5492d759ce6ec7d2462984bd1f3fd1f0bda59ba00fb0dc6d86f9"),
            (0x1d3d20, 8, "ed6f1518d673178ceda05c86907f491791c89b62a5130b8ae0c282ca31317dea"),
            (0x1d3d5c, 8, "ebeebeb6ae1bdbd71dd68ad2966832c9b375a06e581fac22c52cb46132814975"),
            // Search submit and requeue both capture the original session
            // and queue, and call the same pinned Dispatch import.
            (0x1c5090, 224, "161259c3ea6365beed692a13c1ff5f13a5f29d48658d5c3f30e8eb1dca7b30b0"),
            (0x1c53b0, 240, "993fb7d42317a768d1f0c2d1252157d22dd905ea048290ed8b326975f096b09b"),
            (0x1c5abc, 4, "c4401b4cf57266dc01c321591a3e1acf8064a1b8f53c9bf3590747cc2789d752"),
            (0x1c5a38, 36, "ef58e8606a0a0f511152d33fb5132a0b868a009e9026c182e1e2de80f1b2da47"),
            (0x1c5250, 4, "e9a2a0e19d73498131d84f6f338cb19e283229f223b74c7163c0a4dfe5c83b89"),
            (0x1c5ab8, 4, "87f896256898b49911315e0aff19a0cb93ccb2ca4f0fd0848edce565c71d15df"),
            (0x1c5a14, 36, "ce73c6bf5bf1af93969265667ee97348f42a582301295e1603e892f4baa2eeb1"),
            (0x1c5500, 8, "737ec30fde87826a039c95390d89b095424a190306e5afaca9f70b0f3f19e710"),
            (0x1c5ac4, 4, "4daeba0af523a088677def4d165fc1b3ceb5be12121ade0ea49825815d91598e"),
            // Original session cancellation clears pending work under its
            // Swift lock and detaches the owned Process before termination.
            (0x1c4e9c, 236, "b4526cc9b74b324b7c719de0f70781905d8f0b2c9eb2799e7c74c298be3da2f7"),
            (0x1c4bd0, 108, "06afa923c404d21da6f8b236d3562845ccf5dacd27c1546467321f45092e72c9"),
            // Swift converts the returned CFArray to NSArray, then bridges
            // its elements using the original AXUIElement foreign metadata.
            (0x1b6844, 128, "08caee742286746b2885ef70957381808a8912cdfed36fbc208ce62eb78c72b3"),
            (0x13e094, 20, "ed23759c0b7718d94a5624147578f7afb6b1f2a35ad881a69dad29a6a0fbda5f"),
            (0x92314, 8, "54eeaff10a57f19417bc02e5d918ffda8430e7bf19ceef1512b47490c9b006cc"),
            (0x1c5aa4, 16, "16977a37e88822aba0464e1b4f9d5a9d5cefc5d2552cbe4338cddbe7e40b6ac2"),
            (0x1156bc, 160, "1affaaf94ad00f1712cd21c0ed94b743de308098a39fdd4d1416a978b4e8a5b6"),
            (0x2fbb04, 12, "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f"),
            (0x2fbb28, 12, "5e49d9900665dfed9d32a8a353b4ab725f78a1018bd53d877b62ed2f8b08463a"),
            // The ordinary SystemServices.search reply retains Ticket and
            // invokes the original MainActor completion only if still current.
            (0x1c8ba4, 356, "183a310a7942be333abd94911e71efed0e870a37c9d358378d141ee952ba3a54"),
            (0x1d3c1c, 180, "c4cfde4704ee0ab3a341e0b462886f37dcbb5d4b14e36c502e2939c82caf8c52"),
            (0x80864, 84, "dce45ebcfb3631bb98aea8cd5c0295283e31e915028601c1424f60337e0b361e"),
            (0x80184, 28, "8045cccb8c42f37b9a12e3786118433d17f349aa7fb7057d1d46047730a35ddb"),
            (0x801d4, 72, "9d9bf04869ff5ccd1f2b47e03e745eff496208a3129249a583f4bea889095e07"),
            (0x802b0, 32, "5aea226cd8251f01aeebcc273851d673901ad4c6fe75a76a306fc4c5831ab23b"),
            (0x818b4, 88, "589e0d6843a239d7ad69c8657445ec5ab379942f5a3dae75f0a6ca5cc9f139d3"),
            (0x80ce4, 112, "a507445d530a0393e553f6af7555b97d2dbdd5895a9ff84ac3fac0fe70ce2b48"),
            (0x80de4, 28, "fd7c952c417802bf494392677f8aeb50cb87e4b73d7485c4702bf9416e551ff8"),
            (0x80e6c, 116, "1da3d4ff6f97a23d22f4af1d348387e00457273bd17cf7ff2fb4c739ae715df6"),
            (0x81be0, 4, "e82c0cab788aa5cf0c3c7730b8f8c7d2011479a233163d8f2415a9d9bdacf4e4"),
            (0x81c10, 4, "34b236fc76ad6b061207baeb109c212c19bfbd0a577255d8ac85bb349cadf774"),
            (0x8089c, 64, "b6b562334ee0e2f5499d4d53099d7552c43a334aeb9cf4d84157d3869800ce56"),
            (0x8096c, 32, "2c60615c28b503857eb4f68805b7a723fb3b70f83ec38f710badac5b7abc933b"),
            (0x81a4c, 296, "10d48b1fc8c515cf39d4fdb0bc4690cf33ee5aed6d2d6405a2b341ff6b6f644f"),
            (0x81bec, 8, "7dca10f6a848c87490f4981e2df04d1f977816d1f3f461fe3e0b91fa0a6bd19f"),
            (0x81c1c, 8, "8a4557fd8756149924aec9568cfe21929c00ed14b5d14c87c2b0b7cabc8946b8"),
            (0x1d3be8, 52, "28d19614a99e43787ca420369b52998b04e4a46e3cea9a9d3e98ad8ec0b861bb"),
            (0x1d3d70, 4, "970714999b63f9f06f81e0fe5b42c01aa40f4cc88f3a9a3ebe8f0ba63c9a619f"),
            (0x1cf218, 8, "75d6e5258fa76b5c6bdd60b4cc9861a8dd718e709da49b9539cfe2494627b75d"),
            (0x1cebd8, 300, "5b879a955ed3ce4d5f49ef60d00f477ea8ecfb032b78708bbf9d59c4b809d420"),
            (0x1d3950, 48, "e3af351cc6d99703034992741d4060f64d19e8c5e124dc8498d2adb4fba580eb"),
            (0x1d3980, 160, "35b76fabeec5d482d036601f2c58f5995ff3847a441eaface8fb194f769d7120"),
            (0x1d3d64, 4, "661fb9c014c60a4baff6a486b77aeccb5bab891755cdcdc44d5a39f3ca07b3e1"),
            (0x1cb784, 268, "2769a03650f3c56dcc3b85a88139dec654b516558f44c5420a2c956d26ab8529"),
            (0x1d3a30, 56, "adccb42f02cee4d54a397a97d6232855e7b59135b396bced0b02ba9f863745cf"),
            (0x1d3a68, 164, "23827dece6760a68dcbe5a94c26c8885aa0997bbd02847c669c4edbf67cd3bea"),
            (0x1d3d68, 4, "439f775fa41072476f127ca772e348f662f9b5c7fb0dbc9e98e59e36c54bd979"),
            (0x1cb408, 264, "ec778279fac8266e7685163622c394e33ea9c34dd2a043f3d1d628235380ef14"),
            (0x1d3b1c, 56, "60e1796354c18d5e39cbab55024e9ab9964579ad3985543f4269bdc9279e567e"),
            (0x1d3b54, 148, "3f4f9fb6518ef3216a3998c2a036d3bf204983a72ef4623b721525a9258c6565"),
            (0x1d3d6c, 4, "fda870b19afbb9256875f5b2724c0df5b4b42cc4e35e1c661144cfbee04b3648"),
            (0x1e3028, 0x128, "c224c6cef8a2ef0769863fe801b2ce9e213a7bad77217b7b9f4c24202d2f5b78"),
            (0x1e3408, 0x114, "dcd5effe821c278d04d642ee56c58b327e1ae2dcde6768428ddc0973fe487865"),
            (0x1eab94, 4, "7fe8b786080be33eae6c69a1ab97739dc71df456f3507531bb1f528fd072eb80"),
            (0x1eab18, 4, "3310dd8bafa709222ad3a0dc80aa737d5133807076185ea8b4ea18110d6a153e"),
            (0xf97b0, 284, "c996fe8b60a956898fd3ec15cb68cfd00a1be63c6b61399851db22edeeca6087"),
            (0xf9d58, 264, "340cc209c93cbb9bfe59591a1035328f9d11aded573cd7fc2416f9a63afa0024"),
            (0xf9d38, 32, "f262b6c1dcb0d6a35a280ade5eee391c7280e0a841efd187b4f2b14fc1250b86"),
            (0xfbfc8, 20, "6c5760e1b346960e09c258ec754b2d5325f87b66674cd4081722978e56405ddd"),
            (0xfad5c, 16, "c0b054b92a7b20174d839b79778d1082ccd533dc91d1748690df1147a5ff561e"),
            (0xfc0f8, 12, "67185cb3d1cb38f0ad74c919f0f0e736fb8a6447a994111ce9842bdfa53d54f1"),
            (0xfa5a0, 20, "1514a182c3e67fcfc77bb8afe3761144404c4d164dfeaca156b793779eef8f0f"),
            (0xfbf4c, 4, "ea259d0539ff7cbc441385651395847efb604ac9e325d73b0e79caa83b76700a"),
            (0x1a6c38, 272, "5a466bab1937ea0b080d43084ccb5fcd97a78e770f000603ea256a0bbc37f7d5"),
            (0x1aee64, 256, "a1a1830b71dd8178ea54315b219624c0643f1607b3cd11b834ce00141691013e"),
            (0x1affd4, 252, "6a78c5dbf78db8feb53772d839f46afedcfb3913bd710e1a83228eda85085ec3"),
            (0x7d974, 36, "3c6f83540a16f0c763efecc6fb5620fef33c0a08c748c762b67798ffde1967aa"),
            (0x7eca4, 8, "afbdb62c8f11b219cb03fa2666578f30b118decbcaacaf799f1f108f56a6e440"),
            (0x1c9194, 0x74, "001b5b3b404ad5e2fc87cfccbd25c7a97b872ac6eb00ea1b8081d7d3649fdd47"),
            (0x19654, 12, "5dcbea50e0fa982f7d08f5fd1cced5386c4e162c68b8c4e3fb3349df685bbf33"),
            (0x7db10, 40, "c8fc571c6a63cbd822c6981f5b204d98afd5728998c9ba3e09c10ab81349bb8d"),
            (0x7df54, 100, "9f0a8aa21f812791db24f0b0d2e461ca74c0bc7ddaf083f24d4e9045a31872d0"),
            (0x7ecec, 12, "072e03488ff0f2a61d33e5f41522d995c5e31997d4825b98402931f9d5b0c5e2"),
            (0x2f95a8, 12, "d66138d95a87757ad35e9a93511de91107d17f3d731c5f7dafcc1457b129e627"),
            (0x2f95b4, 12, "2013a749589ac24872a0770c634edfb6246575d2712f4354ed88a8392148420c"),
            (0x2fbd80, 12, "62977efd86268670d2b0cedf41827b98760c479156f32d02b8ac035be84bf589"),
            // The separate mainTabs timeout uses asyncAfter, not async.
            (0x1cae74, 288, "dbc9a352d657545cbf0e039ac999b39045fffe509a230e80781b26709b6e0dae"),
            (0x2fa838, 12, "c392031137b6a8991d4b4b109cb77896daaad28c9db6806eadf88bddae889891"),
            (0x1cf37c, 272, "3d91245b9d9dd6e58b7442f4e16571f3dc5017c31c5e13d235f97069be83b3a8"),
            (0x1d3d40, 4, "d304a2d27b0d575dcddc78b78ef5b5e217892848bc134b5706fb14169b113d75"),
            (0x1d3d58, 4, "df5a546e8eea874c59abec1047327df699afb0ae1cabab512e4ca59730973759"),
            (0x1cf0ec, 32, "c61dc75b75acbdbd1e56424f2622ec0925f87a691e8243a7297224dc3cb7b533"),
            (0x1cadfc, 32, "25b82baa82abedc53c6eb4c5dcfc424057d4b1db419adca8c4e7c5c1a29646d2"),
            (0x1cafd0, 92, "ef9eafb79220c44275998746f99b09253d33c965b93b0b52a0f7dbcd60469614"),
            (0x1cf48c, 20, "16b8bc46e5edc193544437f06861b7f6844a6fae4590e39b7d2d4bf5b6d28b78"),
            (0x1cf244, 44, "397c6a5d2939bf857531a0a0485aafe3b43d368f7221cbd95221b061d245736f"),
            // ControlServer.receive creates its semaphore before scheduling
            // the handler. Refusing the handler must also settle that wait.
            (0xdf32c, 488, "3d8d2f2cc6394990adcc79d48c4bc48e50a09936fe074cd78097bf336aff87b9"),
            (0x94ed0, 68, "fdc6bb6d5fef121e668bfd40d8854f20cf465d6ca2ffebec6d760d206bdd4740"),
            (0x7ef08, 24, "a931265625a16d2b240d18db750c17e0e5a61849d94c5b1dd22bca02990b614c"),
            (0x94b2c, 652, "a8477cc2fa762a17f46e4d8729bb417a2aa4cc9e31478b2417860a77226ff168"),
            (0x97f90, 144, "200399d03953aa942ced301021acfb00eb4fdd5d567c477f0efa20ba573d17e1"),
            (0x97340, 96, "1c35acd9c2c653994155edca1b07a3e1acc6e52cc94359aed8e903db55a1a414"),
            (0x93814, 288, "c804cd2874256cbc603bb09059be4fd305a1a05db28483c62c34f49707162627"),
            (0x9498c, 340, "63a4fb12a4eec597cbb59b6ba8ddddfa1dda59528eb3452b472a25243036640e"),
            (0x94c50, 272, "b0ac702506f74e5e57571274c4caffa0a08a3a818321e93d81d1e3129f0f5750"),
            (0xdf544, 92, "ba4e9e875f1c49bf2a092cf4a1b1d73b12b4be4e0f73868b77aceca5ae30a6de"),
            (0xe0480, 92, "8df614e588badf50c112dd4533613b4e56873ecba65ac2628ab4fd25d4817028"),
            (0xe04dc, 72, "8daae440a5a9faa313ad853ca7cbda5b00bd6642f94977a8c523a224788c0d65"),
            (0xe0524, 228, "b0a224ef56ad47abf3ef4617e86d5aad5edd9eb5405898680d375235a6d1159d"),
            (0xe093c, 4, "fc603166d4a1590fac1f60c8654d4ab90eb3eb6f364c97b8596bce0613c6f110"),
            (0x2fa994, 12, "8c563955abb7e0e1613ffd2aac14e603c8214a21fc3d78e7fe7ad4b1f977b928"),
            (0x71c34, 8, "d141007d9732b41a89226a11715ddba03b4468a6c7a5d55288dae58cc51634b7"),
            (0x70274, 4, "10dd26436250a0c6d42440ba9575eba69dcc4f49a4956e8aca150584bc45615d"),
            (0x70334, 4, "1c67f465120c1aa0318887806b50ad92518903bbb6710c4fa7feb7a21e1a31d8"),
            (0x70250, 4, "e1890660e7cd68c292df9060156e27d32969c22fec234305fbdbe4ad56308bdb"),
            (0x70310, 4, "ac0fb58aa1d52c4d8ee0c73424eeb8654e63f5d99f4bb3e8d93944c06468ccb7"),
            (0x70270, 4, "6bce3c12b0cfa1736d240b7d5367ed9a25cc12dffe97b2659ca85379788dde85"),
            (0x70330, 4, "2b6b9a3fc2c14f0510927127fd53e3f17147c0c051e00908edfca1f6091fcb83"),
            (0x70238, 4, "b11fe4854ad331d3c03e5b433850e131fa5d8dbf1f27c2166b57a0022eae4011"),
            (0x702f8, 4, "149ab53db19a3af991e1de56380ef8f6e60472ba2a19ce0d5ddedc053e9b9b13"),
            (0x6bd9c, 16, "a6846d5b2c2d5b3fde5ce0d7adc0d722c1fcc582270172c51b631c5d22ff38f2"),
            (0x6bdac, 8, "967eb5708aef6af8565951f8ffc589f3cf33d0fe1f51e54ee709108af239a257"),
        ] {
            let address = at(offset)?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference continuation code outside exact Main image".into());
            }
            reference_mapping(address, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected {
                return Err(format!("Reference continuation compiler body differs at {offset:#x}"));
            }
        }
        // Bind the argument producers and the continuation's queue, allocation
        // and block ownership imports before observing any live handoff.
        let array_symbol = c"$sSa10FoundationE19_bridgeToObjectiveCSo7NSArrayCyF";
        let url_symbol = c"$s10Foundation3URLV19_bridgeToObjectiveCSo5NSURLCyF";
        let main_symbol = c"$sSo17OS_dispatch_queueC8DispatchE4mainABvgZ";
        let imports = super::imports::observe_unpatched_main_imports(&[
            (array_symbol, (&raw const ARRAY_BRIDGE) as usize),
            (url_symbol, (&raw const URL_BRIDGE) as usize),
            (main_symbol, (&raw const DISPATCH_MAIN) as usize),
            (c"swift_allocObject", (&raw const SWIFT_ALLOC_OBJECT) as usize),
            (c"swift_retain", swift_retain as *const () as usize),
            (c"swift_release", swift_release as *const () as usize),
            (c"swift_errorRetain", swift_errorRetain as *const () as usize),
            (c"swift_errorRelease", swift_errorRelease as *const () as usize),
            (c"swift_deallocObject", swift_deallocObject as *const () as usize),
            (c"_Block_copy", (&raw const BLOCK_COPY) as usize),
            (c"_Block_release", (&raw const BLOCK_RELEASE) as usize),
            (c"objc_msgSend", objc2::ffi::objc_msgSend as *const () as usize),
            (c"objc_release", objc2::ffi::objc_release as *const () as usize),
        ])?;
        let foundation = Path::new("/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation");
        let dispatch = Path::new("/usr/lib/swift/libswiftDispatch.dylib");
        let swift = Path::new("/usr/lib/swift/libswiftCore.dylib");
        let blocks = Path::new("/usr/lib/system/libsystem_blocks.dylib");
        let objc = Path::new("/usr/lib/libobjc.A.dylib");
        for import in imports {
            let (offset, provider) = match import.symbol {
                symbol if symbol == array_symbol => (0x3c5768, foundation),
                symbol if symbol == url_symbol => (0x3c5310, foundation),
                symbol if symbol == main_symbol => (0x3c76a8, dispatch),
                symbol if symbol == c"swift_allocObject" => (0x3c72a0, swift),
                symbol if symbol == c"swift_retain" => (0x3c7448, swift),
                symbol if symbol == c"swift_release" => (0x3c7438, swift),
                symbol if symbol == c"swift_errorRetain" => (0x3c7360, swift),
                symbol if symbol == c"swift_errorRelease" => (0x3c7358, swift),
                symbol if symbol == c"swift_deallocObject" => (0x3c7310, swift),
                symbol if symbol == c"_Block_copy" => (0x3c42d0, blocks),
                symbol if symbol == c"_Block_release" => (0x3c42d8, blocks),
                symbol if symbol == c"objc_msgSend" => (0x3c6940, objc),
                symbol if symbol == c"objc_release" => (0x3c6958, objc),
                _ => return Err("Reference unexpected open import".into()),
            };
            if import.cell_address() != at(offset)? {
                return Err("Reference open import cell differs".into());
            }
            reference_mapping(import.cell_address(), size_of::<usize>(), Some(false))?;
            reference_code(import.original_address(), provider)?;
            import.verify_unpatched()?;
        }
        for (offset, selector) in [(0x3e85f0, objc2::sel!(isRunning)), (0x3e8878, objc2::sel!(terminate))] {
            let cell = at(offset)?;
            let expected = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) };
            if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
                || unsafe { (cell as *const usize).read_volatile() } != expected
            {
                return Err("Reference search cancellation selector differs".into());
            }
        }
        // The outer completions and second blocks use the same stack flags.
        // The extended-layout flag is clear, so only the used fields follow.
        let flags = at(0x2fd9b8)?;
        // These literals reside in Main's read-only executable __TEXT
        // segment. Block descriptors below reside in __DATA_CONST.
        reference_mapping(flags, 8, Some(true))?;
        if unsafe { (flags as *const u64).read_volatile() } != 0x4200_0000 {
            return Err("Reference open stack block flags differ".into());
        }
        for (offset, destructor) in [(0x3c93e0, 0x700a8), (0x3c8fa8, 0x6ea34)] {
            let address = at(offset)?;
            reference_mapping(address, 32, Some(false))?;
            // The capture stores the metadata address 16 bytes into this
            // record, after the deallocator and null value-witness pointer.
            let observed = unsafe { (address as *const [usize; 4]).read_volatile() };
            if observed != [at(destructor)?, 0, 0x400, 16] {
                return Err("Reference open continuation context metadata differs".into());
            }
        }
        for (offset, destructor, descriptor) in [
            (0x3c8dc8, 0x6e220, 0x37b240),
            (0x3c8e68, 0x6e4b8, 0x37b27c),
            (0x3d15a0, 0x1ba4d0, 0x37d0c4),
            (0x3d17e8, 0x1c4680, 0x37d190),
            (0x3d1930, 0x1c5250, 0x37d1a8), // initial search worker
            (0x3d1a88, 0x1c5ab8, 0x37d1bc), // next search turn
            (0x3d20a0, 0x1d3be8, 0x37d310), // search main reply
            (0x3d1fb0, 0x1d3950, 0x37d2d0), // process details reply
            (0x3d2000, 0x1d3a30, 0x37d2e8), // tab action reply
            (0x3d2050, 0x1d3b1c, 0x37d2fc), // main tab lookup reply
            (0x3d1bf0, 0x1d3d40, 0x37d23c), // main tab deadline
            (0x3d1bc8, 0x1cf244, 0x37d220), // shared main tab delivery
            (0x3cc390, 0xe0458, 0x37bf48), // ControlServer wire reply capture
            (0x3cc3b8, 0xe04dc, 0x37bf5c), // ControlServer handler capture
            (0x3c9e08, 0x94158, 0x37b7f4), // Safari activation
            (0x3c9ea8, 0x97f64, 0x37b828), // Safari activation eligibility
            (0x3c9e58, 0x9937c, 0x37b810), // Safari consent worker
            (0x3ca300, 0x99318, 0x37b844), // Safari consent reply
            (0x3d2ab8, 0x1ead60, 0x37d43c), // Website cached image
            (0x3d2a68, 0x1eab14, 0x37d414), // Website final result
            (0x3ccb18, 0xfc0cc, 0x37c0f0), // Hyper Quartz action
            (0x3ccac8, 0xfbefc, 0x37c0c8), // Hyper HID action
            (0x3d0c90, 0x1a7bf4, 0x37cda8), // Snippet choice finish
            (0x3d11c0, 0x1b078c, 0x37cf64), // Snippet choice presentation
            (0x3d1210, 0x1b0620, 0x37cf94), // Snippet cursor
        ] {
            let address = at(offset)?;
            reference_mapping(address, 40, Some(false))?;
            if unsafe { (address as *const [usize; 5]).read_volatile() }
                != [at(destructor)?, 0, 0x400, 16, at(descriptor)?]
            {
                return Err("Reference monitor/search capture destructor metadata differs".into());
            }
        }
        for (offset, copy, dispose, signature) in [
            (0x3c93b8, 0x70270, 0x70330, 0x365760),
            (0x3c8c60, 0x70238, 0x702f8, 0x365760),
            (0x3c9408, 0x70274, 0x70334, 0x3672b0),
            (0x3c8fd0, 0x70250, 0x70310, 0x3672b0),
            (0x3c8df0, 0x70244, 0x70304, 0x3672b0), // CLI sampler
            (0x3c8e90, 0x70248, 0x70308, 0x3672b0), // CLI main reply
            (0x3d15c8, 0x1ba8a0, 0x1ba8b0, 0x3672b0), // window sampler
            (0x3d1810, 0x1c4740, 0x1c474c, 0x3672b0), // window main reply
            (0x3d1958, 0x1c54f0, 0x1c5500, 0x3672b0), // initial search worker
            (0x3d1ab0, 0x1c5ab4, 0x1c5ac4, 0x3672b0), // next search turn
            (0x3d20c8, 0x1d3d34, 0x1d3d70, 0x3672b0), // search main reply
            (0x3d1fd8, 0x1d3d28, 0x1d3d64, 0x3672b0), // process details reply
            (0x3d2028, 0x1d3d2c, 0x1d3d68, 0x3672b0), // tab action reply
            (0x3d2078, 0x1d3d30, 0x1d3d6c, 0x3672b0), // main tab lookup reply
            (0x3d1c18, 0x1d3d1c, 0x1d3d58, 0x3672b0), // main tab deadline
            (0x3cc3e0, 0xe0934, 0xe093c, 0x3672b0), // ControlServer handler
            (0x3c9e30, 0x9417c, 0x9418c, 0x3672b0), // Safari activation
            (0x3c9e80, 0x99370, 0x99384, 0x3672b0), // Safari consent worker
            (0x3ca328, 0x99374, 0x99388, 0x3672b0), // Safari consent reply
            (0x3d2ae0, 0x1ead5c, 0x1ead98, 0x3672b0), // Website cached image
            (0x3d2a90, 0x1ead58, 0x1ead94, 0x3672b0), // Website final result
            (0x3ccb40, 0xfc31c, 0xfc32c, 0x3672b0), // Hyper Quartz action
            (0x3ccaf0, 0xfc318, 0xfc328, 0x3672b0), // Hyper HID action
            (0x3d0cb8, 0x1a8d04, 0x1a8d20, 0x3672b0), // Snippet choice finish
            (0x3d11e8, 0x1b055c, 0x1b056c, 0x3672b0), // Snippet choice presentation
            (0x3d1238, 0x1b0780, 0x1b0790, 0x3672b0), // Snippet cursor
            (0x3c99e8, 0x81bf0, 0x81c20, 0x3672b0), // Configuration observer reply
            (0x3c9998, 0x81bec, 0x81c1c, 0x3672b0), // Configuration debounce
            (0x3c9850, 0x81be0, 0x81c10, 0x3672b0), // Configuration acknowledgement
        ] {
            let descriptor = at(offset)?;
            reference_mapping(descriptor, 40, Some(false))?;
            let expected = [0, 48, at(copy)?, at(dispose)?, at(signature)?];
            let observed = unsafe { (descriptor as *const [usize; 5]).read_volatile() };
            if observed != expected {
                return Err(format!("Reference continuation block descriptor differs at {offset:#x}"));
            }
        }
        for (offset, signature) in [
            (0x365760, b"v24@?0@\"NSRunningApplication\"8@\"NSError\"16\0".as_slice()),
            (0x3672b0, b"v8@?0\0".as_slice()),
        ] {
            let address = at(offset)?;
            reference_mapping(address, signature.len(), Some(true))?;
            if unsafe { std::slice::from_raw_parts(address as *const u8, signature.len()) } != signature {
                return Err("Reference open block signature differs".into());
            }
        }
        self.before_reference_initialization()?;
        let adapter = crate::workspace::receiver::app_delegate::constructor::swift_self_adapter as *const () as usize;
        reference_code(adapter, &self.sdk)?;
        reference_mapping(adapter, 100, Some(true))?;
        if digest(unsafe { std::slice::from_raw_parts(adapter as *const u8, 100) })
            != "30291925bf7831296003aa608d880c227737963d1023bcbf739c61de5f69da71"
        {
            return Err("Reference search cancellation Swift adapter differs".into());
        }
        let deadline_spec = Self::reference_main_tabs_deadline_spec();
        let deadline_bindings = super::imports::observe_main_imports(&[deadline_spec])?;
        let [deadline] = deadline_bindings.as_slice() else {
            return Err("Reference mainTabs requires one exact deadline import".into());
        };
        if deadline.cell_address() != at(0x3c7658)? {
            return Err("Reference mainTabs deadline import cell differs".into());
        }
        reference_mapping(deadline.cell_address(), size_of::<usize>(), Some(false))?;
        reference_code(deadline_spec.original, dispatch)?;
        reference_code(deadline_spec.replacement, &self.sdk)?;
        let expected_deadline = if installed { deadline_spec.replacement } else { deadline_spec.original };
        if deadline.selected_address() != expected_deadline {
            return Err("Reference mainTabs deadline selected address differs".into());
        }
        crate::workspace::callbacks::prepare_reference_main_tabs_deadline(
            deadline_spec.original, [at(0x1caf8c)?, at(0x1a6d48)?, at(0x1aef64)?, at(0x1b00d0)?, at(0x802c8)?],
        )?;
        let wait_spec = Self::reference_control_wait_spec();
        let wait_bindings = super::imports::observe_main_imports(&[wait_spec])?;
        let [wait] = wait_bindings.as_slice() else {
            return Err("Reference ControlServer requires one exact wait import".into());
        };
        if wait.cell_address() != at(0x3c7758)? {
            return Err("Reference ControlServer wait import cell differs".into());
        }
        reference_mapping(wait.cell_address(), size_of::<usize>(), Some(false))?;
        reference_code(wait_spec.original, dispatch)?;
        reference_code(wait_spec.replacement, &self.sdk)?;
        let expected_wait = if installed { wait_spec.replacement } else { wait_spec.original };
        if wait.selected_address() != expected_wait {
            return Err("Reference ControlServer wait selected address differs".into());
        }
        crate::workspace::callbacks::prepare_reference_control_wait(wait_spec.original, at(0xdf588)?)?;
        let spec = Self::reference_open_dispatch_spec();
        let bindings = super::imports::observe_main_imports(&[spec])?;
        let [binding] = bindings.as_slice() else {
            return Err("Reference Open requires one exact Dispatch import".into());
        };
        if binding.cell_address() != at(0x3c76b8)? {
            return Err("Reference Open Dispatch cell differs".into());
        }
        reference_mapping(binding.cell_address(), size_of::<usize>(), Some(false))?;
        reference_code(spec.original, dispatch)?;
        reference_code(spec.replacement, &self.sdk)?;
        let expected = if installed { spec.replacement } else { spec.original };
        if binding.selected_address() != expected {
            return Err("Reference Open Dispatch selected address differs".into());
        }
        let shortcuts_imports = [
            (c"$s6Darwin5fcntlys5Int32VAD_ADtF", (&raw const SHORTCUTS_GETFL) as usize),
            (c"$s6Darwin5fcntlys5Int32VAD_A2DtF", (&raw const SHORTCUTS_SETFL) as usize),
            (c"read", libc::read as *const () as usize),
        ];
        let observed = super::imports::observe_unpatched_main_imports(&shortcuts_imports)?;
        for ((symbol, _), offset) in shortcuts_imports.into_iter().zip([0x3c7548, 0x3c7540, 0x3c4850]) {
            let row = observed.iter().find(|row| row.symbol == symbol)
                .ok_or("Reference Shortcuts stream import absent")?;
            if row.cell_address() != at(offset)? {
                return Err("Reference Shortcuts stream import cell differs".into());
            }
            row.verify_unpatched()?;
        }
        let descriptor_selector = unsafe {
            std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::sel!(fileDescriptor))
        };
        let descriptor_cell = at(0x3e8870)?;
        if reference_mapping_protection(descriptor_cell, size_of::<usize>())? & 5 != 1
            || unsafe { (descriptor_cell as *const usize).read_volatile() } != descriptor_selector
        {
            return Err("Reference Shortcuts descriptor selector differs".into());
        }
        crate::workspace::callbacks::prepare_reference_shortcuts_descriptors(
            [at(0x7db14)?, at(0x7db28)?, at(0x7df70)?],
            [at(0x7d984)?, at(0x7d994)?],
        )?;
        crate::workspace::callbacks::prepare_reference_dispatch_frontdoor(
            spec.original, at(0x40f84)?, at(0x5ec08)?,
            [at(0x61e94)?, at(0x65040)?, at(0x1ba148)?, at(0x1ba488)?,
                at(0x1c5168)?, at(0x1c5494)?, at(0x1c8cf8)?, at(0x1cecf4)?, at(0x1cb88c)?,
                at(0x1cb508)?, at(0xdf50c)?, at(0x9392c)?, at(0x94ad0)?, at(0x94d5c)?,
                at(0x1e314c)?, at(0x1e3518)?, at(0xf98c8)?, at(0xf9e60)?, at(0x80984)?, at(0x80dfc)?,
                at(0x1cabe0)?, at(0x1cac84)?, at(0x1ce774)?],
        )
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_worker_dispatch_block(
        &self, route: usize, group: usize, queue: usize, block: usize,
    ) -> Result<usize> {
        #[link(name = "System")]
        unsafe extern "C" {
            static _NSConcreteMallocBlock: u8;
            static _dispatch_main_q: u8;
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference monitor Dispatch address overflow");
        let &(descriptor, thunk, metadata) = [
            (0x3c8df0, 0x6e234, 0x3c8dd8),
            (0x3c8e90, 0x6e578, 0x3c8e78),
            (0x3d15c8, 0x1ba4f8, 0x3d15b0),
            (0x3d1810, 0x1c4718, 0x3d17f8),
            (0x3d1958, 0x1c5abc, 0x3d1940),
            (0x3d1ab0, 0x1c5a38, 0x3d1a98),
            (0x3d20c8, 0x1d3c1c, 0x3d20b0),
            (0x3d1fd8, 0x1d3980, 0x3d1fc0),
            (0x3d2028, 0x1d3a68, 0x3d2010),
            (0x3d2078, 0x1d3b54, 0x3d2060),
            (0x3d1c18, 0x1cf37c, 0x3d1c00),
            (0x3cc3e0, 0xe0524, 0x3cc3c8),
            (0x3c9e30, 0x94160, 0x3c9e18),
            (0x3c9e80, 0x97bb4, 0x3c9e68),
            (0x3ca328, 0x99358, 0x3ca310),
            (0x3d2ae0, 0x1eab94, 0x3d2ac8),
            (0x3d2a90, 0x1eab18, 0x3d2a78),
            (0x3ccb40, 0xfc0f8, 0x3ccb28),
            (0x3ccaf0, 0xfbf4c, 0x3ccad8),
            (0x3d0cb8, 0x1a7c20, 0x3d0ca0),
            (0x3d11e8, 0x1b04b4, 0x3d11d0),
            (0x3d1238, 0x1b0640, 0x3d1220),
            (0x3c99e8, 0x81a4c, 0x3c99d0),
            (0x3c9998, 0x818b4, 0x3c9748),
            (0x3c9850, 0x80e6c, 0x3c9838),
            (0x3d1b78, 0x1cf1ec, 0),
            (0x3d1ba0, 0x1cf220, 0),
            (0x3d1c90, 0x1d2654, 0x3d1b38),
        ].get(route).ok_or("Reference monitor Dispatch route differs")?;
        if group != 0 || queue == 0
            || (queue == (&raw const _dispatch_main_q) as usize) != matches!(route, 1 | 3 | 6..=12 | 14..=22)
        {
            return Err("Reference monitor Dispatch destination differs".into());
        }
        if !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3 {
            return Err("Reference monitor Dispatch block mapping differs".into());
        }
        let words = block as *const usize;
        let word = |index| unsafe { words.add(index).read_volatile() };
        let (descriptor, thunk) = if route == 27 && word(3) == at(0x3d1cb8)? {
            (0x3d1cb8, 0x1d269c)
        } else {
            (descriptor, thunk)
        };
        // The original caller may release its +1 after the callback starts.
        // libclosure updates the low 32-bit flags/refcount atomically.
        let flags = unsafe { std::sync::atomic::AtomicU32::from_ptr(words.add(1).cast_mut().cast()) }
            .load(std::sync::atomic::Ordering::Acquire);
        if word(0) != (&raw const _NSConcreteMallocBlock) as usize
            || flags & !0xfffe != 0x4300_0000 || flags & 0xfffe == 0
            || word(2) != at(0x33e24)? || word(3) != at(descriptor)? || word(4) != at(thunk)?
        {
            return Err("Reference monitor Dispatch block layout differs".into());
        }
        let context = word(5);
        if matches!(route, 25 | 26) {
            // These blocks retain SystemServices directly, not a separate Swift capture.
            let (copy, dispose, field, offset) = if route == 25 {
                let cell = at(0x406ee8)?;
                if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3 {
                    return Err("Reference Safari worker offset cell mapping differs".into());
                }
                (0x1cf208, 0x1cf218, c"worker", unsafe { (cell as *const usize).read_volatile() })
            } else {
                (0x1d3d18, 0x1d3d54, c"mainTabWorker", 0x200)
            };
            let descriptor = at(descriptor)?;
            reference_mapping(descriptor, 40, Some(false))?;
            if unsafe { (descriptor as *const [usize; 5]).read_volatile() }
                != [0, 48, at(copy)?, at(dispose)?, at(0x3672b0)?]
            {
                return Err("Reference Safari cache block descriptor differs".into());
            }
            let class = super::metadata::loaded_class(c"RuycastF1Reference.SystemServices")?;
            if class.address() != at(0x3f55b0)? {
                return Err("Reference Safari cache owner class differs".into());
            }
            let class = unsafe { &*(class.address() as *const objc2::runtime::AnyClass) };
            let ivars = class.instance_variables();
            let mut fields = ivars.iter().filter(|ivar| ivar.name() == field);
            if class.instance_size() != 608 || offset < 16 || !offset.is_multiple_of(8)
                || offset.checked_add(8).is_none_or(|end| end > class.instance_size())
                || fields.next().and_then(|ivar| usize::try_from(ivar.offset()).ok()) != Some(offset)
                || fields.next().is_some() || !context.is_multiple_of(8)
                || reference_mapping_protection(context, class.instance_size())? & 7 != 3
                || unsafe { objc2::ffi::object_getClass(context as *const _) } != class as *const _
                || unsafe { ((context + offset) as *const usize).read_volatile() } != queue
            {
                return Err("Reference Safari cache owner or destination differs".into());
            }
            return Ok(context);
        }
        let capture_size = match route {
            4 | 5 => 32,
            6 => 81,
            7 => 57,
            8 => 41,
            9 => 64,
            10 => 32,
            11 => 72,
            12 => 52,
            13 | 14 => 48,
            15 | 16 => 56,
            18 | 21 => 48,
            20 | 23 | 27 => 24,
            22 => 194,
            _ => 40,
        };
        if !context.is_multiple_of(8) || reference_mapping_protection(context, capture_size)? & 7 != 3
            || unsafe { (context as *const usize).read_volatile() } != at(metadata)?
        {
            return Err("Reference monitor Dispatch capture differs".into());
        }
        if route == 27 {
            let (copy, dispose) = if descriptor == 0x3d1c90 {
                (0x1d3d20, 0x1d3d5c)
            } else {
                (0x1d3d24, 0x1d3d60)
            };
            let descriptor = at(descriptor)?;
            let metadata = at(metadata)?;
            reference_mapping(descriptor, 40, Some(false))?;
            reference_mapping(metadata - 16, 32, Some(false))?;
            if unsafe { (descriptor as *const [usize; 5]).read_volatile() }
                != [0, 48, at(copy)?, at(dispose)?, at(0x3672b0)?]
                || unsafe { ((metadata - 16) as *const usize).read_volatile() } != at(0x1cf15c)?
                || unsafe { (metadata as *const usize).read_volatile() } != 0x400
            {
                return Err("Reference system cache block or weak ownership differs".into());
            }
        }
        if matches!(route, 22 | 24) {
            let weak = unsafe { ((context + 16) as *const usize).read_volatile() };
            if !weak.is_multiple_of(8) || reference_mapping_protection(weak, 24)? & 7 != 3
                || unsafe { (weak as *const usize).read_volatile() } != at(0x3c9748)?
            {
                return Err("Reference configuration reply weak capture differs".into());
            }
        }
        if matches!(route, 23 | 24) {
            unsafe { verify_configuration_observer_queue(queue) }?;
        }
        if matches!(route, 4..=7) {
            let captured = unsafe { ((context + 16) as *const [usize; 2]).read_volatile() };
            if (matches!(route, 4 | 5) && captured[1] != queue) || !captured[0].is_multiple_of(8)
                || reference_mapping_protection(captured[0], 16)? & 7 != 3
                || unsafe { objc2::ffi::object_getClass(captured[0] as *const _) } as usize != at(0x3f5498)?
            {
                return Err("Reference search Dispatch session or queue capture differs".into());
            }
        }
        if matches!(route, 9 | 10) {
            let [function, delivery] = unsafe { ((context + 16) as *const [usize; 2]).read_volatile() };
            if function != at(0x1cf270)? || !delivery.is_multiple_of(8)
                || reference_mapping_protection(delivery, 56)? & 7 != 3
                || unsafe { (delivery as *const usize).read_volatile() } != at(0x3d1bd8)?
            {
                return Err("Reference mainTabs delivery capture differs".into());
            }
        }
        if route == 12 {
            let deadline = unsafe { ((context + 24) as *const f64).read_volatile() };
            let pid = unsafe { ((context + 48) as *const i32).read_volatile() };
            if !deadline.is_finite() || deadline <= 0.
                || pid != super::process_router::REFERENCE_SAFARI_PID
            {
                return Err("Reference Safari activation target or deadline differs".into());
            }
        }
        if matches!(route, 13 | 14) {
            let browser = unsafe { ((context + 16) as *const usize).read_volatile() };
            if !browser.is_multiple_of(8) || reference_mapping_protection(browser, 88)? & 7 != 3
                || unsafe { objc2::ffi::object_getClass(browser as *const _) } as usize != at(0x3ed388)?
                || (route == 13 && unsafe { ((browser + 0x50) as *const usize).read_volatile() } != queue)
            {
                return Err("Reference Safari consent browser or queue differs".into());
            }
        }
        Ok(context)
    }

    // The original block owns the context; the caller independently holds
    // the HyperKey through its live Hotkeys/AppState chain.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_hyper_action_capture(&self, route: usize, context: usize, keyboard: usize) -> Result<()> {
        use std::ffi::c_void;
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_weakLoadStrong(reference: *const c_void) -> *mut c_void;
            fn swift_release();
        }
        if keyboard == 0 || self.owner != std::thread::current().id()
            || unsafe { libc::pthread_main_np() } != 1
        {
            return Err("Reference HyperKey action requires its original owner".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let (size, context_metadata) = match route {
            17 => (40, 0x3ccb28),
            18 => (48, 0x3ccad8),
            _ => return Err("Reference HyperKey action route differs".into()),
        };
        if !context.is_multiple_of(8) || reference_mapping_protection(context, size)? & 7 != 3
            || unsafe { (context as *const usize).read_volatile() } != base + context_metadata
        {
            return Err("Reference HyperKey action context differs".into());
        }
        let weak = unsafe { ((context + 16) as *const usize).read_volatile() };
        let metadata = base + 0x3cc7d8;
        reference_mapping(metadata - 16, 40, Some(false))?;
        if !weak.is_multiple_of(8) || reference_mapping_protection(weak, 24)? & 7 != 3
            || unsafe { (weak as *const usize).read_volatile() } != metadata
            || unsafe { ((metadata - 16) as *const [usize; 5]).read_volatile() }
                != [base + 0xfad5c, 0, 0x400, 16, base + 0x37c040]
        {
            return Err("Reference HyperKey action weak box differs".into());
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"swift_weakLoadStrong", swift_weakLoadStrong as *const () as usize),
            (c"swift_release", swift_release as *const () as usize),
        ])?;
        for (symbol, offset) in [(c"swift_weakLoadStrong", 0x3c74f0), (c"swift_release", 0x3c7438)] {
            let import = imports.iter().find(|row| row.symbol == symbol)
                .ok_or("Reference HyperKey action ARC import absent")?;
            if import.cell_address() != base + offset {
                return Err("Reference HyperKey action ARC cell differs".into());
            }
            reference_code(import.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
            import.verify_unpatched()?;
        }
        let actual = unsafe { swift_weakLoadStrong((weak + 16) as *const c_void) };
        if actual as usize != keyboard {
            return Err("Reference HyperKey action captured a different owner".into());
        }
        let release: unsafe extern "C" fn(*mut c_void) =
            unsafe { std::mem::transmute(swift_release as *const ()) };
        unsafe { release(actual) };
        Ok(())
    }

    // Borrow the original Ticket through the retained activation capture.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn safari_activation_session(context: usize) -> Result<usize> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let [function, eligibility] = unsafe { ((context + 32) as *const [usize; 2]).read_volatile() };
        if base.checked_add(0x97f90) != Some(function)
            || !eligibility.is_multiple_of(8)
            || reference_mapping_protection(eligibility, 72)? & 7 != 3
            || base.checked_add(0x3c9eb8) != Some(unsafe { (eligibility as *const usize).read_volatile() })
        {
            return Err("Reference Safari activation eligibility capture differs".into());
        }
        let session = unsafe { ((eligibility + 16) as *const usize).read_volatile() };
        if !session.is_multiple_of(8) || reference_mapping_protection(session, 16)? & 7 != 3
            || base.checked_add(0x3f5498) != Some(unsafe { objc2::ffi::object_getClass(session as *const _) } as usize)
        {
            return Err("Reference Safari activation Ticket session differs".into());
        }
        Ok(session)
    }

    // The checked consent-worker block keeps this exact BrowserTabs alive.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn cancel_safari_consent(browser: usize) -> Result<()> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let cancel = base.checked_add(0x94ed0).ok_or("Reference Safari cancel address overflow")?;
        unsafe { crate::workspace::receiver::app_delegate::constructor::swift_self_adapter(browser, cancel); }
        Ok(())
    }

    // Only called with the verified ControlServer block's immutable capture.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn control_reply_semaphore(&self, context: usize) -> Result<usize> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference ControlServer address overflow");
        let [wire_function, wire_context, ui_function, ui_context] =
            unsafe { ((context + 40) as *const [usize; 4]).read_volatile() };
        if wire_function != at(0xe0480)? || ui_function != 0 || ui_context != 0
            || !wire_context.is_multiple_of(8) || reference_mapping_protection(wire_context, 32)? & 7 != 3
        {
            return Err("Reference ControlServer wire reply capture differs".into());
        }
        let [metadata, _, _, semaphore] =
            unsafe { (wire_context as *const [usize; 4]).read_volatile() };
        let class = unsafe { objc2::ffi::objc_getClass(c"OS_dispatch_semaphore".as_ptr()) };
        if metadata != at(0x3cc3a0)? || !semaphore.is_multiple_of(8)
            || reference_mapping_protection(semaphore, 8)? & 7 != 3
            || class.is_null() || unsafe { objc2::ffi::object_getClass(semaphore as *const _) } != class
        {
            return Err("Reference ControlServer wire semaphore differs".into());
        }
        Ok(semaphore)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn read_main_tabs_deadline(address: usize) -> Result<u64> {
        super::system_providers::monitor_timeout::verify_dispatch_time_layout()?;
        if !address.is_multiple_of(8) || reference_mapping_protection(address, 8)? & 7 != 3 {
            return Err("Reference mainTabs deadline mapping differs".into());
        }
        let time = unsafe { (address as *const u64).read_volatile() };
        if time == 0 || time > i64::MAX as u64 {
            return Err("Reference mainTabs deadline is not a finite uptime".into());
        }
        Ok(time)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_dispatch_queue_frame(frame: usize, queue: usize, offset: usize) -> Result<()> {
        // Each pinned Swift call saves self.worker in its frame, then
        // reloads it into x20 immediately before calling Dispatch.
        let saved = frame.checked_sub(offset).ok_or("Reference Dispatch queue frame underflow")?;
        if !frame.is_multiple_of(16) || reference_mapping_protection(saved, size_of::<usize>())? & 7 != 3
            || unsafe { (saved as *const usize).read_volatile() } != queue
        {
            return Err("Reference Dispatch queue differs from saved source queue".into());
        }
        Ok(())
    }

    /// The caller holds a verified original search worker block whose
    /// immutable capture owns this exact session for the duration of cancel.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn cancel_search_worker_session(&self, session: usize) -> Result<()> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let cancel = base.checked_add(0x1c4e9c).ok_or("Reference search cancel address overflow")?;
        let class = base.checked_add(0x3f5498).ok_or("Reference search cancel class overflow")?;
        if reference_image_header(cancel, &self.endpoint)? != base || !session.is_multiple_of(8)
            || reference_mapping_protection(session, 24)? & 7 != 3
            || unsafe { objc2::ffi::object_getClass(session as *const _) } as usize != class
        {
            return Err("Reference search cancellation session or image differs".into());
        }
        reference_mapping(cancel, 236, Some(true))?;
        if digest(unsafe { std::slice::from_raw_parts(cancel as *const u8, 236) })
            != "b4526cc9b74b324b7c719de0f70781905d8f0b2c9eb2799e7c74c298be3da2f7"
        {
            return Err("Reference search cancellation code differs".into());
        }
        self.registry_current()?;
        unsafe {
            crate::workspace::receiver::app_delegate::constructor::swift_self_adapter(session, cancel);
        }
        self.registry_current()
    }

    /// The block is borrowed from the exact compiled caller or held by the
    /// owner queue. The caller must keep that ownership through this read.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_open_block(
        &self, return_pc: usize, block: usize, opens_url: bool,
    ) -> Result<(usize, [usize; 4])> {
        self.before_reference_publication()?;
        self.verify_open_completion_source(true)?;
        unsafe { self.verify_open_block_capture(return_pc, block, opens_url) }
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    unsafe fn verify_open_block_capture(
        &self, return_pc: usize, block: usize, opens_url: bool,
    ) -> Result<(usize, [usize; 4])> {
        #[link(name = "System")]
        unsafe extern "C" {
            static _NSConcreteMallocBlock: u8;
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference open image address overflow");
        let (caller, descriptor, thunk, context_metadata) = if opens_url {
            (0x5a32c, 0x3c8c60, 0x6cd48, 0x3c8c48)
        } else {
            (0x40e10, 0x3c93b8, 0x7009c, 0x3c93a0)
        };
        if return_pc != at(caller)? {
            return Err("Reference open caller differs".into());
        }
        if !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3 {
            return Err("Reference open block is not writable non-executable data".into());
        }
        // 64-bit Block_layout plus the two compiler-generated captures.
        // Runtime refcount may change under _Block_copy, but no other bit may.
        let words = unsafe { (block as *const [usize; 6]).read_volatile() };
        if words[0] != (&raw const _NSConcreteMallocBlock) as usize
            || words[1] & !0xfffe != 0x4300_0000
            || words[1] & 0xfffe == 0
            || words[2] != at(0x40fa4)?
            || words[3] != at(descriptor)?
            || words[4] != at(thunk)?
        {
            return Err("Reference open heap block differs".into());
        }
        let context = words[5];
        if !context.is_multiple_of(8) || reference_mapping_protection(context, 32)? & 7 != 3 {
            return Err("Reference open block capture is not writable non-executable data".into());
        }
        let captured = unsafe { (context as *const [usize; 4]).read_volatile() };
        if captured[0] != at(context_metadata)? {
            return Err("Reference open capture metadata differs".into());
        }
        let (finish_context, finish_metadata) = if opens_url {
            if captured[2] != at(0x6ccd8)? {
                return Err("Reference open URL finish thunk differs".into());
            }
            (captured[3], 0x3c8bf8)
        } else {
            let completion = captured[3];
            if !completion.is_multiple_of(8) || reference_mapping_protection(completion, 40)? & 7 != 3 {
                return Err("Reference open completion capture is not writable non-executable data".into());
            }
            let completion = completion as *const usize;
            let metadata = unsafe { completion.read_volatile() };
            let finish = unsafe { completion.add(2).read_volatile() };
            let finish_metadata = match (
                captured[2].checked_sub(base),
                metadata.checked_sub(base),
                finish.checked_sub(base),
            ) {
                (Some(0x6cd54), Some(0x3c8c98), Some(0x6ccd8)) => 0x3c8bf8,
                (Some(0x702e8), Some(0x3c9238), Some(0x7033c)) => 0x3c9198,
                (Some(0x702e8), Some(0x3c8f90), Some(0x7033c)) => 0x3c8ef0,
                _ => return Err("Reference open completion thunk or context metadata differs".into()),
            };
            (unsafe { completion.add(3).read_volatile() }, finish_metadata)
        };
        if !finish_context.is_multiple_of(8)
            || reference_mapping_protection(finish_context, 0x90)? & 7 != 3
        {
            return Err("Reference open finish capture is not writable non-executable data".into());
        }
        if unsafe { (finish_context as *const usize).read_volatile() } != at(finish_metadata)? {
            return Err("Reference open finish capture metadata differs".into());
        }
        self.registry_current()?;
        let app_state = unsafe { ((finish_context + 0x20) as *const usize).read_volatile() };
        if app_state == 0 {
            return Err("Reference open finish AppState absent".into());
        }
        Ok((context, [captured[2], captured[3], finish_context, app_state]))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_snippet_monitor_call(&self, caller: usize, controller: usize, remove: bool) -> Result<()> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if caller.checked_sub(base) != Some(if remove { 0x1a58a0 } else { 0x1a6484 })
            || unsafe { libc::pthread_main_np() } != 1 || super::process_fork::is_child()
            || !controller.is_multiple_of(8) || reference_mapping_protection(controller, 0x30)? & 7 != 3
            || unsafe { (controller as *const usize).read_volatile() } != base + 0x3f44d0
        {
            return Err("Reference SnippetChoice monitor caller or controller differs".into());
        }
        let (start, bytes, expected) = if remove {
            (0x1a587c, 44, "e8b7f14a1f11d61d6e80e1d01b106dec933d518ea3f01450903d645eeff6712e")
        } else {
            (0x1a6404, 148, "67d4993d5290175ca2e2c07579ab9c50397fd969af7f9a59f2065f9156f7336f")
        };
        let address = base + start;
        reference_mapping(address, bytes, Some(true))?;
        if reference_image_header(address, &self.endpoint)? != base
            || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != expected
        {
            return Err("Reference SnippetChoice monitor call code differs".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_snippet_monitor_block(&self, block: usize) -> Result<usize> {
        #[link(name = "System")]
        unsafe extern "C" { static _NSConcreteMallocBlock: u8; }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        self.verify_reference_snippet_model_deallocator()?;
        for (offset, bytes, expected) in [
            (0x115a08, 100, "6ee4e8cbc4d0d6733fbb4d382acf2d5162a4e771945caf21e734fa62ee48fc68"),
            (0x1a76f4, 164, "86049619483dff7d0295c34d4661a8e61d3886085a26b120963e07901598e893"),
            (0x1a8cfc, 4, "ca3d3e8eb5db81486bceaa5988dc7f7029f531bc4982f447a9d4fdecdca88ed1"),
            (0x1a8d18, 4, "461b057e639e01da5ec229c6a893c7d2665d7b6887d3775ca742507ee38caff9"),
        ] {
            let address = base + offset;
            reference_mapping(address, bytes, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != expected
            {
                return Err("Reference SnippetChoice monitor ARC code differs".into());
            }
        }
        let descriptor = base + 0x3d0b40;
        reference_mapping(descriptor, 40, Some(false))?;
        if unsafe { (descriptor as *const [usize; 5]).read_volatile() }
            != [0, 48, base + 0x1a8cfc, base + 0x1a8d18, base + 0x36fe60]
            || !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3
        {
            return Err("Reference SnippetChoice monitor block storage differs".into());
        }
        let words = unsafe { (block as *const [usize; 6]).read_volatile() };
        if words[0] != (&raw const _NSConcreteMallocBlock) as usize || words[1] & !0xfffe != 0x4300_0000
            || words[1] & 0xfffe == 0 || words[2] != base + 0x115a08
            || words[3] != descriptor || words[4] != base + 0x1a76f4
        {
            return Err("Reference SnippetChoice monitor heap block differs".into());
        }
        let capture = words[5];
        let metadata = base + 0x3d0a88;
        reference_mapping(metadata - 16, 32, Some(false))?;
        if !capture.is_multiple_of(8) || reference_mapping_protection(capture, 24)? & 7 != 3
            || unsafe { (capture as *const usize).read_volatile() } != metadata
            || unsafe { ((metadata - 16) as *const usize).read_volatile() } != base + 0x1a7574
            || unsafe { (metadata as *const usize).read_volatile() } != 0x400
        {
            return Err("Reference SnippetChoice monitor weak capture differs".into());
        }
        Ok(capture)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    // The caller holds the original expander and its strong controller slot
    // on the owner thread for the duration of this verification.
    pub(crate) unsafe fn verify_snippet_choice_completion(&self, controller: usize, expander: usize) -> Result<()> {
        use std::ffi::c_void;
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_weakLoadStrong(reference: *const c_void) -> *mut c_void;
            fn swift_release();
        }
        if super::process_fork::is_child() || self.owner != std::thread::current().id()
            || unsafe { libc::pthread_main_np() } != 1 || expander == 0
        {
            return Err("Reference SnippetChoice completion requires its original owner".into());
        }
        self.registry_current()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if !controller.is_multiple_of(8) || reference_mapping_protection(controller, 0x98)? & 7 != 3
            || unsafe { (controller as *const usize).read_volatile() } != base + 0x3f44d0
        {
            return Err("Reference SnippetChoice completion receiver differs".into());
        }
        for (offset, bytes, expected) in [
            (0x1aedac, 100, "6b01864b640d603258b4302956f56723410cf2693713f1384ae4891bd893611e"),
            (0x1a55a8, 424, "715b030c203e2bb530de19829a416c9cabf9b08e517862027c79c775bb3aaf77"),
            (0x1b0444, 112, "f1ab7562a028ee48344e734b99619a503a283fb325de467ceb8308847ec92b47"),
            (0x1adbe8, 36, "631e6dd277d3566aa29a305835ee3fa9f0275a5cf68d3ebb54f8ced72a3cfabc"),
            (0x2fc320, 12, "d525a36d63d4e1413db0b5f70e2c5784a40e07592f377a99d52c53333094ecbe"),
        ] {
            let address = base + offset;
            reference_mapping(address, bytes, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != expected
            {
                return Err("Reference SnippetChoice completion code differs".into());
            }
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"swift_weakLoadStrong", swift_weakLoadStrong as *const () as usize),
            (c"swift_release", swift_release as *const () as usize),
        ])?;
        if imports.len() != 2 {
            return Err("Reference SnippetChoice ARC imports differ".into());
        }
        for import in &imports {
            let offset = if import.symbol == c"swift_weakLoadStrong" { 0x3c74f0 } else { 0x3c7438 };
            if import.cell_address() != base + offset {
                return Err("Reference SnippetChoice ARC cell differs".into());
            }
            reference_code(import.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
            import.verify_unpatched()?;
        }
        let metadata = base + 0x3d10b8;
        reference_mapping(metadata - 16, 32, Some(false))?;
        let [function, capture] = unsafe { ((controller + 0x88) as *const [usize; 2]).read_volatile() };
        if function != base + 0x1b0444
            || !capture.is_multiple_of(8) || reference_mapping_protection(capture, 24)? & 7 != 3
            || unsafe { (capture as *const usize).read_volatile() } != metadata
            || unsafe { ((metadata - 16) as *const usize).read_volatile() } != base + 0x1adbe8
            || unsafe { (metadata as *const usize).read_volatile() } != 0x400
        {
            return Err("Reference SnippetChoice onFinish weak capture differs".into());
        }
        let actual = unsafe { swift_weakLoadStrong((capture + 16) as *const c_void) };
        if actual as usize != expander {
            // The foreign object is unqualified for final release. Keep any
            // retained reference quarantined on this refused path.
            return Err("Reference SnippetChoice onFinish targets another expander".into());
        }
        // The caller independently holds the expander, so this cannot deinit.
        let release: unsafe extern "C" fn(*mut c_void) =
            unsafe { std::mem::transmute(swift_release as *const ()) };
        unsafe { release(actual); }
        self.registry_current()
    }

    /// Check the actual second block against the still-held outer completion.
    /// This is an address observation. It neither copies nor schedules a block.
    ///
    /// # Safety
    /// The caller must hold both blocks for this check. `dispatch_return_pc`,
    /// `queue` and `group` must be taken from the actual Swift dispatch call,
    /// and `error` from the retained argument passed to the outer completion.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub unsafe fn verify_open_continuation(
        &self, outer: (usize, usize), expected_outer: (usize, [usize; 4]), opens_url: bool,
        dispatch_return_pc: usize, group: usize, queue: usize, block: usize, error: usize,
    ) -> Result<usize> {
        self.before_reference_publication()?;
        self.verify_open_completion_source(true)?;
        unsafe {
            self.verify_open_continuation_capture(outer, expected_outer, opens_url, dispatch_return_pc, group, queue, block, error)
        }
    }

    /// Callback-free tail for the held blocks after full source qualification.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_open_continuation_capture(
        &self, outer: (usize, usize), expected_outer: (usize, [usize; 4]), opens_url: bool,
        dispatch_return_pc: usize, group: usize, queue: usize, block: usize, error: usize,
    ) -> Result<usize> {
        #[link(name = "System")]
        unsafe extern "C" {
            static _NSConcreteMallocBlock: u8;
            static _dispatch_main_q: u8;
        }
        let observed = unsafe { self.verify_open_block_capture(outer.0, outer.1, opens_url) }?;
        if observed != expected_outer {
            return Err("Reference outer open capture changed".into());
        }
        let (_, [function, closure, _, _]) = observed;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference open continuation address overflow");
        let (caller, descriptor, thunk, metadata) = if opens_url {
            (0x5ec08, 0x3c8fd0, 0x6ea4c, 0x3c8fb8)
        } else {
            (0x40f84, 0x3c9408, 0x700e8, 0x3c93f0)
        };
        if dispatch_return_pc != at(caller)? || group != 0 || queue != (&raw const _dispatch_main_q) as usize {
            return Err("Reference open continuation caller or destination differs".into());
        }
        if !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3 {
            return Err("Reference open continuation block mapping differs".into());
        }
        let words = unsafe { (block as *const [usize; 6]).read_volatile() };
        if words[0] != (&raw const _NSConcreteMallocBlock) as usize
            || words[1] & !0xfffe != 0x4300_0000
            || words[1] & 0xfffe == 0
            || words[2] != at(0x33e24)?
            || words[3] != at(descriptor)?
            || words[4] != at(thunk)?
        {
            return Err("Reference open continuation block layout differs".into());
        }
        let context = words[5];
        if !context.is_multiple_of(8) || reference_mapping_protection(context, 40)? & 7 != 3 {
            return Err("Reference open continuation capture mapping differs".into());
        }
        let captured = unsafe { (context as *const [usize; 5]).read_volatile() };
        let expected = if opens_url { [error, function, closure] } else { [function, closure, error] };
        if captured[0] != at(metadata)? || captured[2..] != expected {
            return Err("Reference open continuation does not retain the original completion".into());
        }
        self.registry_current()?;
        Ok(context)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_web_session_source(&self) -> Result<()> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference website address overflow");
        for (offset, length, sha) in [
            (0x1e5c88, 24, "f37a463fa917c62a525fdaffbf24172e5276c61dc1c2aa0feceb918163ffffa3"),
            (0x1e67f0, 164, "d01cda91caf57dabe1129cead30593bde2ca551bf0d23b49023f3227f69089c0"),
            (0x1e6894, 28, "241231e4775d3fbdd5279ebcf312889455af4c7faccaa56be7c3591a18be2648"),
            (0x115518, 112, "281c332fa234dbdf112d1af03a895b2734070b2bc60585f73b999928c67b36f6"),
            (0x1e5ca0, 32, "9cf2e660dd6838009a71a573639b6bc73da06729e996c75ce262180fa58e27f2"),
            (0x1e5fbc, 44, "66c74f7752025e70abfede40e079183fe0ecbe69b9962a9bcd4e539ab0581259"),
            // Task creation and the following unconditional resume message.
            (0x1e5dcc, 52, "2a4c4a3f464c9ca4c0d61c4e2dda2538f65907a0a394dbcc1875e58ecdde543f"),
            (0x1e5e64, 20, "55baeca1ef156bed5a3b127fb1d52f1a2512f6869c0f79b01dc9c9d7e08a0f16"),
        ] {
            let address = at(offset)?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference website callsite image differs".into());
            }
            reference_mapping(address, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != sha {
                return Err("Reference website callsite bytes differ".into());
            }
        }
        let cfnetwork = b"/System/Library/Frameworks/CFNetwork.framework/Versions/A/CFNetwork";
        for (role, class_ref, selector, selector_ref, abi, method_offset) in [
            (c"NSURLSessionConfiguration", 0x3e9b58, c"ephemeralSessionConfiguration", 0x3e9198, b"@16@0:8".as_slice(), 0x7cf98),
            (c"NSURLSession", 0x3e9b60, c"sessionWithConfiguration:delegate:delegateQueue:", 0x3e91e8, b"@40@0:8@16@24@32".as_slice(), 0x12878),
        ] {
            let class = super::metadata::loaded_class(role)?;
            let method = super::metadata::method(class, selector, true)?
                .ok_or("Reference website selected class method absent")?;
            let metaclass = unsafe { objc2::ffi::object_getClass(class.address() as *const _) } as usize;
            if metaclass == 0 || method.owner() != metaclass || method.encoding() != abi
                || method.code().image() != cfnetwork || method.code().offset() != method_offset
            {
                return Err("Reference website class method ABI or provider differs".into());
            }
            let selected = unsafe {
                std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::runtime::Sel::register(selector))
            };
            for (offset, expected) in [(class_ref, class.address()), (selector_ref, selected)] {
                let address = at(offset)?;
                if reference_mapping_protection(address, size_of::<usize>())? & 5 != 1
                    || unsafe { (address as *const usize).read_volatile() } != expected
                {
                    return Err("Reference website loaded selector or class differs".into());
                }
            }
        }
        let owner = super::metadata::loaded_class(c"NSURLSession")?;
        let concrete = super::metadata::loaded_class(c"__NSURLSessionLocal")?;
        for (selector, selector_ref, abi, method_offset) in [
            (c"dataTaskWithRequest:", 0x3e91d0, b"@24@0:8@16".as_slice(), 0x52f68),
            (c"invalidateAndCancel", 0x3e91e0, b"v16@0:8".as_slice(), 0x8b298),
        ] {
            let method = super::metadata::method(concrete, selector, false)?
                .ok_or("Reference website session leaf absent")?;
            let selected = unsafe {
                std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::runtime::Sel::register(selector))
            };
            let address = at(selector_ref)?;
            if method.owner() != owner.address() || method.encoding() != abi
                || method.code().image() != cfnetwork || method.code().offset() != method_offset
                || reference_mapping_protection(address, size_of::<usize>())? & 5 != 1
                || unsafe { (address as *const usize).read_volatile() } != selected
            {
                return Err("Reference website session leaf provenance differs".into());
            }
        }
        let resume = at(0x3e91d8)?;
        let selected = unsafe {
            std::mem::transmute::<objc2::runtime::Sel, usize>(
                objc2::runtime::Sel::register(c"resume"),
            )
        };
        if reference_mapping_protection(resume, size_of::<usize>())? & 5 != 1
            || unsafe { (resume as *const usize).read_volatile() } != selected
        {
            return Err("Reference website task resume selector differs".into());
        }
        let init = at(0x3e8480)?;
        let selected = unsafe {
            std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::sel!(init))
        };
        if reference_mapping_protection(init, size_of::<usize>())? & 5 != 1
            || unsafe { (init as *const usize).read_volatile() } != selected
        {
            return Err("Reference website delegate initializer selector differs".into());
        }
        self.registry_current()
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_reference_web_queue_source(&self) -> Result<()> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference website queue address overflow");
        for (offset, length, sha) in [
            // Original singleton initializer and getter, synchronous image
            // lookup (including the enqueue call), fetch, worker and publish.
            (0x1e138c, 0x70, "ed75dbe474879458fc91157ca58f9f69d127e424f9932d83522e925ed8d9e9a9"),
            (0x1e158c, 0x40, "4e071aa6d32c4c8396ecd5f265394a0a436433bda4be1e0aea99f73b2bbc8f91"),
            (0x1e2394, 0x58c, "adcf3b3d74924f4ad1dac74dbc6edbff8313204fd5e716b5ddbcfff662c986ce"),
            (0x1e1894, 0x35c, "e11810029dc6e2550058b036b9c92d1592688d8212663cda56632c4bc468333e"),
            (0x1e2920, 0xe54, "f323ab766c2e4dc5e1dfc6f9d49fb9b07d4429f9820d8988fe5d223f23223317"),
            (0x1e3c10, 0x688, "845fef09ab5ed6a160e3660319be9d0f5c3260e47dd38e0b57419cf0faa7341c"),
            // Worker capture thunk and the Block copy and dispose helpers.
            (0x1e9ee4, 0x8c, "24cc7fafd760da1f20b5af37daf7fa29a8442cdfc4a793c198ea077cbe9ba516"),
            // Explicit private-cache construction and its default loader.
            (0x1e14f4, 0x58, "15e5d51138afa7a04ad0a011608d2fbf006ff024a73ee735616a38ead6ebb3ca"),
            (0x1e1bf0, 0x1cc, "27cc7a4de2293784e287df2a410fa57352f6d46cedd9792e8e105b79ef6554e5"),
            (0x1e9f8c, 0x34, "63c4598f811c51758a1158059669be53f48609499a4091991b703a760c605568"),
            (0x1e1868, 0x2c, "4f29a797d9b4bfb2ec40cf023135c8df1527872f319df20667dc48f476d96103"),
            (0x1e1c88, 28, "e529c86a7fd5ccb3db37f6744414cf1fbccf082f0eaf4c85dca4c85e31c8591a"),
            (0x1e1d10, 44, "7c303bca51fbdd76397d1cc6fdc2ffb07a1e5c493f033aef28e78b54d3202fa4"),
            (0x1e3b04, 32, "e85e9e1282135383929e8fbf196e1d17a4b878879a40158f29b2b8bcb4d1439f"),
            (0x1e411c, 52, "fdb14c2e5d4afab072d2430729be38d9f1c0b2dff4ee097a39a7740a4a7a09f6"),
            (0x09ed4c, 76, "a9d66f3448f3071e910b265f3fa8386981c872fa2724ecc77a155ca42a559296"),
            (0x2fbaec, 12, "cfcb0bf56e07e38ed89130ff04f8c5eeed1dd8e1ec7378d52306d6f28a1e18dd"),
        ] {
            let address = at(offset)?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference website queue source image differs".into());
            }
            reference_mapping(address, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != sha {
                return Err("Reference website queue source bytes differ".into());
            }
        }
        let descriptor = at(0x3d29c0)?;
        reference_mapping(descriptor, 40, Some(false))?;
        if unsafe { (descriptor as *const [usize; 5]).read_volatile() }
            != [0, 48, at(0x1e9f58)?, at(0x1e9f68)?, at(0x3672b0)?]
        {
            return Err("Reference website operation descriptor differs".into());
        }
        let class = super::metadata::loaded_class(c"NSOperationQueue")?;
        let selector = unsafe {
            std::mem::transmute::<objc2::runtime::Sel, usize>(
                objc2::runtime::Sel::register(c"addOperationWithBlock:"),
            )
        };
        for (offset, expected) in [
            (0x3e9a10, class.address()),
            (0x3e9148, selector),
            (0x3e9a80, super::metadata::loaded_class(c"NSCache")?.address()),
            (0x3e99d0, super::metadata::loaded_class(c"NSBitmapImageRep")?.address()),
            (0x3e99c8, super::metadata::loaded_class(c"NSImage")?.address()),
            (0x3e99c0, super::metadata::loaded_class(c"NSLock")?.address()),
        ] {
            let cell = at(offset)?;
            if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
                || unsafe { (cell as *const usize).read_volatile() } != expected
            {
                return Err("Reference website operation loaded class or selector differs".into());
            }
        }
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_palette_panel_allocator(caller: usize) -> bool {
        (unsafe { _NSGetMachExecuteHeader() } as usize).checked_add(0x14c9e0) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// Select only the original model register for this exact frozen caller.
    /// The original constructor owns the borrow until the retain/store pair.
    pub(crate) fn verify_reference_snippet_panel_allocator(
        &self, caller: usize, class: usize, wrapper_model: usize, inline_model: usize,
    ) -> Result<usize> {
        if super::process_fork::is_child() || unsafe { libc::pthread_main_np() } != 1 {
            return Err("Reference SnippetChoicePanel allocator off its owner".into());
        }
        self.before_reference_publication()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference SnippetChoicePanel allocator address overflow");
        if class != at(0x3eab70)? || super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoicePanel")?.address() != class {
            return Err("Reference SnippetChoicePanel allocator class differs".into());
        }
        let (model, spans): (usize, &[(usize, usize, &str)]) = match caller.checked_sub(base) {
            Some(0x1a2e40) => (wrapper_model, &[
                (0x1a2e28, 44, "c185ff2764ba1327047648bcc2d2475383f6cc9ca5dda4e24bfac945ee3f4b82"),
            ]),
            Some(0x1a6064) => (inline_model, &[
                (0x1a5fb8, 72, "99b046691502bea1fe973415008790f07cee47041f2c44584e82869a35e3c272"),
                (0x131b2c, 12, "2c77155716c236d40a91074850c1454818eab9dc18c6d9e2361e4fe1cae0b97f"),
                (0x2fbf54, 12, "7d426483accac52771625b86d33ba8ad37b8ddb88307529c4203dfba4411ac48"),
                (0x1a605c, 24, "0effab5cd1088a859b87e14606f5886daadf4f43335adfbf1578b9e47a5470bd"),
            ]),
            _ => return Err("Reference SnippetChoicePanel allocator caller differs".into()),
        };
        for &(offset, length, expected) in spans {
            let address = at(offset)?;
            reference_mapping(address, length, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference SnippetChoicePanel allocator model producer differs".into());
            }
        }
        unsafe { snippet_panel_model_frame(base, model) }?;
        self.verify_reference_snippet_model_deallocator()?;
        self.verify_reference_snippet_model_cleanup_contexts(model)?;
        Ok(model)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// The super-init ledger must already hold the original allocation and
    /// exact model borrow recorded by its allocator. Validate the real strong
    /// field, then compare the geometry emitted by that same Swift initializer.
    pub(crate) fn verify_reference_snippet_panel_model_slot(
        &self,
        receiver: usize,
        model: usize,
        rect: objc2_foundation::NSRect,
    ) -> Result<()> {
        self.before_reference_publication()?;
        let offset = self.verify_reference_snippet_panel_cleanup()?;
        let field = receiver.checked_add(offset).ok_or("Reference SnippetChoicePanel model address overflow")?;
        if receiver == 0 || !receiver.is_multiple_of(8)
            || reference_mapping_protection(field, size_of::<usize>())? & 7 != 3
            || unsafe { (field as *const usize).read_volatile() } != model
        {
            return Err("Reference SnippetChoicePanel strong model differs from its allocation".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if unsafe { snippet_panel_model_frame(base, model) }? != rect {
            return Err("Reference SnippetChoicePanel frame differs from the original model".into());
        }
        self.verify_reference_snippet_model_deallocator()?;
        self.verify_reference_snippet_model_cleanup_contexts(model)?;
        if unsafe { (field as *const usize).read_volatile() } != model {
            return Err("Reference SnippetChoicePanel model changed during qualification".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// Static anchors for the last release of the model held by the original
    /// panel. A live model and the Combine destroy witness remain separate.
    pub(crate) fn verify_reference_snippet_model_deallocator(&self) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_bridgeObjectRelease();
            fn swift_deallocClassInstance();
            fn swift_release();
            fn swift_allocObject();
            fn swift_weakInit();
            fn swift_weakDestroy();
            fn swift_deallocObject();
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference SnippetChoiceModel deallocator overflow");
        for (offset, length, expected) in [
            (0x1a2d54, 196, "996eadc52028b488e084b38a7163a7ae5c795f7cef1c80bfab56cfb83ecc5e60"),
            (0x2fbfa8, 12, "a2012bd00742bab4ef5498c650651d5cf358d8ed85ec7750844cd614b9853cd0"),
            (0x2fbff0, 12, "0ce9a5a37f21099bbd755b829f71dfc623fda7e466d48b610261375d4dbbe537"),
            // The two captures created by present() share a Swift heap
            // layout with one weak slot at +16 and no strong captures.
            (0x1a5f90, 112, "cae723a6c6d9b2954b75b9535d487f79a7a1b3f6d0e147891306178dc2fe1a16"),
            (0x18fa0, 12, "6eb58e37b95250b604bb5c7914027c90b48d96bad1a059bcaddcafe153100139"),
            (0x71a38, 12, "e010ad6527573c5e4fc4cb66a2ebb522a56a83ee9ee16729e99bfa387599276c"),
            (0x1a7574, 36, "628cc6383cf037f076144182f1220cca2a8726d909caa4ec5b2ecfecd0ae944f"),
            (0x70cac, 12, "e535292a10841e68e8e3a573e10d05c235388aa1451eb913cd4a6352830eac22"),
            (0x2fbf54, 12, "7d426483accac52771625b86d33ba8ad37b8ddb88307529c4203dfba4411ac48"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
            (0x2fc308, 12, "d7ee60ef4229bcde4dd5f91aacd06113a7b10d39824064ee4a13c80d58e4e107"),
            (0x2fc314, 12, "55717894c42b77d2efdb4d65e24a59e6c7a3678ee237d37f82f54b4c26b183c4"),
            (0x2fbffc, 12, "9a43a65f2d464d3c961ffc9ed50d0c2291c341b94efdfd377866c0c6743f54ea"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, length, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference SnippetChoiceModel deallocator code differs".into());
            }
        }
        let class = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceModel")?;
        let deallocation = at(0x3f43c8)?;
        if class.address() != at(0x3f43d8)?
            || reference_mapping_protection(deallocation, size_of::<usize>())? & 7 != 3
            || unsafe { (deallocation as *const usize).read_volatile() } != at(0x1a2df4)?
        {
            return Err("Reference SnippetChoiceModel class deallocation entry differs".into());
        }
        let class = unsafe { &*(class.address() as *const objc2::runtime::AnyClass) };
        let fields = class.instance_variables();
        for (name, cell, bytes) in [(c"_values", 0x4057a8, 9), (c"activeField", 0x40c968, 16),
            (c"onSubmit", 0x4057b8, 16), (c"onCancel", 0x4057c0, 16)] {
            let cell = at(cell)?;
            let mut matching = fields.iter().filter(|field| field.name() == name);
            let field = matching.next().ok_or("Reference SnippetChoiceModel destructor field absent")?;
            let offset = usize::try_from(field.offset()).map_err(|_| "Reference SnippetChoiceModel destructor field offset differs")?;
            if matching.next().is_some() || !offset.is_multiple_of(8) || offset < 0xd0
                || offset.checked_add(bytes).is_none_or(|end| end > class.instance_size())
                || reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3
                || unsafe { (cell as *const usize).read_volatile() } != offset
            {
                return Err("Reference SnippetChoiceModel destructor layout differs".into());
            }
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"swift_bridgeObjectRelease", swift_bridgeObjectRelease as *const () as usize),
            (c"swift_deallocClassInstance", swift_deallocClassInstance as *const () as usize),
            (c"swift_release", swift_release as *const () as usize),
            (c"swift_allocObject", swift_allocObject as *const () as usize),
            (c"swift_weakInit", swift_weakInit as *const () as usize),
            (c"swift_weakDestroy", swift_weakDestroy as *const () as usize),
            (c"swift_deallocObject", swift_deallocObject as *const () as usize),
        ])?;
        for (name, offset) in [
            (c"swift_bridgeObjectRelease", 0x3c72d8),
            (c"swift_deallocClassInstance", 0x3c7308),
            (c"swift_release", 0x3c7438),
            (c"swift_allocObject", 0x3c72a0),
            (c"swift_weakInit", 0x3c74e8),
            (c"swift_weakDestroy", 0x3c74e0),
            (c"swift_deallocObject", 0x3c7310),
        ] {
            let import = imports.iter().find(|row| row.symbol == name)
                .ok_or("Reference SnippetChoiceModel deallocation import absent")?;
            if import.cell_address() != at(offset)? {
                return Err("Reference SnippetChoiceModel deallocation import cell differs".into());
            }
            reference_code(import.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
            import.verify_unpatched()?;
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_snippet_model_cleanup_contexts(&self, model: usize) -> Result<()> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let class = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceModel")?;
        let class = unsafe { &*(class.address() as *const objc2::runtime::AnyClass) };
        if !model.is_multiple_of(8) || reference_mapping_protection(model, class.instance_size())? & 7 != 3
            || unsafe { (model as *const usize).read_volatile() } != class as *const _ as usize
        {
            return Err("Reference SnippetChoiceModel cleanup receiver differs".into());
        }
        let metadata = base.checked_add(0x3d0a88).ok_or("SnippetChoiceModel capture metadata overflow")?;
        let destructor = metadata - 16;
        reference_mapping(destructor, 32, Some(false))?;
        if unsafe { (destructor as *const usize).read_volatile() } != base + 0x1a7574
            || unsafe { (metadata as *const usize).read_volatile() } != 0x400
        {
            return Err("Reference SnippetChoiceModel capture metadata differs".into());
        }
        let mut contexts = [0; 2];
        for (index, (offset_cell, thunk)) in [(0x4057b8, 0x1a7650), (0x4057c0, 0x1a7694)].into_iter().enumerate() {
            let cell = base.checked_add(offset_cell).ok_or("SnippetChoiceModel capture cell overflow")?;
            if reference_mapping_protection(cell, size_of::<usize>())? & 7 != 3 {
                return Err("Reference SnippetChoiceModel capture layout cell differs".into());
            }
            let offset = unsafe { (cell as *const usize).read_volatile() };
            if !offset.is_multiple_of(8) || offset < 0xd0
                || offset.checked_add(16).is_none_or(|end| end > class.instance_size())
            {
                return Err("Reference SnippetChoiceModel closure storage differs".into());
            }
            let closure = model.checked_add(offset).ok_or("SnippetChoiceModel closure address overflow")? as *const usize;
            let (function, context) = unsafe { (closure.read_volatile(), closure.add(1).read_volatile()) };
            // Only inspect the heap header. Swift owns the weak slot and
            // performs the eventual weak destruction through the pinned entry.
            if function != base + thunk || !context.is_multiple_of(8)
                || reference_mapping_protection(context, 24)? & 7 != 3
                || unsafe { (context as *const usize).read_volatile() } != metadata
            {
                return Err("Reference SnippetChoiceModel closure context differs".into());
            }
            contexts[index] = context;
        }
        if contexts[0] == contexts[1] {
            return Err("Reference SnippetChoiceModel closure contexts alias".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_snippet_panel_allocator(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        matches!(caller.checked_sub(base), Some(0x1a2e40 | 0x1a6064))
    }

    /// Physical cleanup metadata only. The caller must separately bind the
    /// live receiver and the model stored in its original strong field.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_snippet_panel_cleanup(&self) -> Result<usize> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_release();
        }
        if super::process_fork::is_child() || unsafe { libc::pthread_main_np() } != 1 {
            return Err("Reference SnippetChoicePanel cleanup off its owner".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference SnippetChoicePanel cleanup address overflow");
        for (offset, bytes, sha) in [
            (0x1a3410, 16, "81b763a92bbd7bd402befeff7becf5a3986d1a156957fb7f40f7bf81d1c41bd3"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, bytes, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != sha
            {
                return Err("Reference SnippetChoicePanel destructor or Swift release stub differs".into());
            }
        }
        let releases = super::imports::observe_unpatched_main_imports(&[
            (c"swift_release", swift_release as *const () as usize),
        ])?;
        let [release] = releases.as_slice() else {
            return Err("Reference SnippetChoicePanel release import absent".into());
        };
        if release.cell_address() != at(0x3c7438)? {
            return Err("Reference SnippetChoicePanel release import cell differs".into());
        }
        reference_code(release.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
        let panel = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoicePanel")?;
        if panel.address() != at(0x3eab70)? {
            return Err("Reference SnippetChoicePanel cleanup class differs".into());
        }
        let offset = ReferenceAppDelegateContext::verify_snippet_panel_destructor_layout(
            panel, at(0x1a3410)?, at(0x3f4328)?,
        )?;
        release.verify_unpatched()?;
        Ok(offset)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// Resolve the exact destruction metadata using the original compiler
    /// helper during construction, before permitting a nonnil model teardown.
    pub(crate) fn observe_reference_snippet_published_destroy(&self) -> Result<(usize, usize)> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_getTypeByMangledNameInContext2();
        }
        #[link(name = "Combine", kind = "framework")]
        unsafe extern "C" {
            #[link_name = "$s7Combine9PublishedVMn"]
            static PUBLISHED_DESCRIPTOR: u8;
        }
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference Published metadata overflow");
        for (offset, length, expected) in [
            (0x448c, 68, "01c7285f4a39a33fd49acc85c77450a298776b6557ebd4ba9f32d9707e634920"),
            (0x2fc128, 12, "1c153c0c5d0fd5001a6e4587f7b3b1edfeb35a526f80ae6df1ccf5d9575cd8c2"),
            (0x35864c, 14, "added75a86d30bebb27bd7b50023217c72bdb4507ab2922830b3ad23c831a6b5"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, length, (offset != 0x35864c).then_some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference Published metadata compiler binding differs".into());
            }
        }
        let descriptor = at(0x3073a0)?;
        reference_mapping(descriptor, 8, None)?;
        if unsafe { (descriptor as *const [u32; 2]).read_volatile() } != [0x512ac, 14] {
            return Err("Reference Published type descriptor differs".into());
        }
        let runtime = Path::new("/usr/lib/swift/libswiftCore.dylib");
        let resolver = swift_getTypeByMangledNameInContext2 as *const () as usize;
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"swift_getTypeByMangledNameInContext2", resolver),
            (c"$s7Combine9PublishedVMn", (&raw const PUBLISHED_DESCRIPTOR) as usize),
        ])?;
        for (symbol, offset) in [(c"swift_getTypeByMangledNameInContext2", 0x3c73d8),
            (c"$s7Combine9PublishedVMn", 0x3c4e48)] {
            let import = imports.iter().find(|row| row.symbol == symbol)
                .ok_or("Reference Published metadata import absent")?;
            if import.cell_address() != at(offset)? {
                return Err("Reference Published metadata import cell differs".into());
            }
        }
        reference_code(resolver, runtime)?;
        let cache = at(0x3f4318)?;
        if reference_mapping_protection(cache, size_of::<usize>())? & 7 != 3 {
            return Err("Reference Published cache mapping differs".into());
        }

        // The exact original helper owns its own metadata cache write.
        for import in &imports { import.verify_unpatched()?; }
        let resolve: unsafe extern "C" fn(*const u8, usize, usize, usize) -> usize = unsafe { std::mem::transmute(resolver) };
        let mangled = b"7Combine9PublishedVySDyS2SGG";
        let expected_type = unsafe { resolve(mangled.as_ptr(), mangled.len(), 0, 0) };
        let cached = unsafe { (cache as *const usize).read_volatile() };
        if expected_type == 0 || (cached != 0 && cached != expected_type) {
            return Err("Reference Published runtime type or cached identity differs".into());
        }
        for import in &imports { import.verify_unpatched()?; }
        let instantiate: unsafe extern "C" fn(usize, usize) -> usize = unsafe { std::mem::transmute(at(0x448c)?) };
        let metadata = unsafe { instantiate(cache, descriptor) };
        if metadata != expected_type {
            return Err("Reference Published original helper resolved a different type".into());
        }
        let table = self.reference_snippet_published_destroy_table(metadata)?;
        Ok((metadata, table))
    }

    /// Recheck the same metadata immediately before the original destructor.
    /// No metadata initialization, business callback or release occurs here.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_snippet_published_destroy_cache(
        &self, (metadata, expected_table): (usize, usize),
    ) -> Result<()> {
        if self.reference_snippet_published_destroy_table(metadata)? != expected_table {
            return Err("Reference Published destroy witness table changed".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn reference_snippet_published_destroy_table(&self, metadata: usize) -> Result<usize> {
        if super::process_fork::is_child() || unsafe { libc::pthread_main_np() } != 1 {
            return Err("Reference Published destroy check off its owner".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let cache = base.checked_add(0x3f4318).ok_or("Reference Published cache overflow")?;
        if reference_mapping_protection(cache, size_of::<usize>())? & 7 != 3
            || unsafe { (cache as *const usize).read_volatile() } != metadata
        {
            return Err("Reference Published destructor metadata cache changed".into());
        }
        if !metadata.is_multiple_of(std::mem::align_of::<usize>()) {
            return Err("Reference Published metadata address is unaligned".into());
        }
        let table_slot = metadata.checked_sub(size_of::<usize>()).ok_or("Reference Published witness pointer absent")?;
        if reference_mapping_protection(table_slot, size_of::<usize>())? & 5 != 1 {
            return Err("Reference Published metadata mapping differs".into());
        }
        let table = unsafe { (table_slot as *const usize).read_volatile() };
        if !table.is_multiple_of(std::mem::align_of::<usize>())
            || reference_mapping_protection(table, 2 * size_of::<usize>())? & 5 != 1
        {
            return Err("Reference Published witness table mapping differs".into());
        }
        let destroy = unsafe { (table as *const usize).add(1).read_volatile() };
        let exported = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"swift_cvw_destroy".as_ptr()) } as usize;
        if destroy == 0 || destroy != exported {
            return Err("Reference Published destroy witness differs".into());
        }
        reference_code(destroy, Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
        Ok(table)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_palette_panel_source(&self) -> Result<()> {
        self.before_reference_initialization()?;
        self.verify_reference_palette_panel_metadata()?;
        self.registry_current()
    }

    /// Source metadata needed by physical deallocation even after the workspace
    /// and guardian have stopped accepting business operations.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_palette_panel_metadata(&self) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_release();
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference PalettePanel address overflow");
        for (offset, bytes, sha) in [
            (0xaea8, 16, "f5b0485bb56a0b5d718b7276427403731063b6fcb6156aad32c20d48b4422089"),
            (0x14c448, 516, "fb429a4a2e2a7e5c47939a9154eff6188ac0ef9f332584f99faf9c65c508438d"),
            (0x14c9d8, 60, "26120ff5a517fc6efe9e22d46a42b91f83bd6a6079dbb8b805edc128f9fbba49"),
            (0x14caf0, 40, "93ade6caf8d50f780847eb255ae02e909595f467c67adb7bfb0772ac89180b6e"),
            (0x14cb30, 68, "0d9fa1e42111b1a268bbb15d93a2aae141ccff7277247d814724e715e0992635"),
            (0x14c754, 156, "76ca2dd98abdef15ab504a66f0a1b77312f594ef8c857754f2c933a0af96b8d2"),
            (0x14c7f0, 212, "4bb812444bf34530be560f47f5d8320dde01411f91449a065cdff58e751e93e5"),
            (0x14c8c4, 64, "8efdf39db9b8707d08ca5a4d5f484021468528849b8f94da1424e7e1f011681e"),
            (0x152660, 16, "733a53bee4693676e302c8e89a9238055e6b9d787037d26828ceb028ed3625c3"),
            (0x152920, 12, "417a2192a0bebcfcb8ee055fd2c9cc672540f3f42e21dbc3bfe492df772626ea"),
            (0x2fc1ac, 12, "c37d5ae8a63c82577eb825cfceae02ec961eff1f1f22532686897fdd08a80797"),
            (0x2fbb10, 12, "d2bc9ae552d79df7b57b9087552ded25b98e3b8ce50054812ca2edffef979f31"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, bytes, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != sha
            {
                return Err("Reference PalettePanel compiler code differs".into());
            }
        }
        // Both compiler-emitted destructor calls use the same optional closure
        // destroy helper, which tail-calls this unmodified Swift ARC import.
        let releases = super::imports::observe_unpatched_main_imports(&[
            (c"swift_release", swift_release as *const () as usize),
        ])?;
        let [release] = releases.as_slice() else {
            return Err("Reference PalettePanel destructor release import absent".into());
        };
        if release.cell_address() != at(0x3c7438)? {
            return Err("Reference PalettePanel destructor release cell differs".into());
        }
        reference_code(release.original_address(), Path::new("/usr/lib/swift/libswiftCore.dylib"))?;
        release.verify_unpatched()?;
        let panel = super::metadata::loaded_class(c"RuycastF1Reference.PalettePanel")?;
        let parent = super::metadata::loaded_class(c"NSPanel")?;
        if panel.address() != at(0x3ea5c0)? {
            return Err("Reference PalettePanel metadata differs".into());
        }
        let class = unsafe { &*(panel.address() as *const objc2::runtime::AnyClass) };
        let superclass = class.superclass().ok_or("Reference PalettePanel superclass absent")?;
        if superclass as *const _ as usize != parent.address() {
            return Err("Reference PalettePanel is not the original NSPanel subclass".into());
        }
        for (selector, implementation, encoding) in [
            (c"initWithContentRect:styleMask:backing:defer:", 0x14c7f0,
                b"@68@0:8{CGRect={CGPoint=dd}{CGSize=dd}}16Q48Q56B64".as_slice()),
            (c".cxx_destruct", 0x14c8c4, b"v16@0:8".as_slice()),
        ] {
            let method = super::metadata::method(panel, selector, false)?
                .ok_or("Reference PalettePanel lifecycle method absent")?;
            if method.owner() != panel.address() || method.implementation() != at(implementation)?
                || method.encoding() != encoding
            {
                return Err("Reference PalettePanel lifecycle method differs".into());
            }
        }
        let fields = class.instance_variables();
        let mut previous_end = superclass.instance_size();
        for (name, offset_cell) in [(c"actions", 0x3f2830), (c"itemBindingKey", 0x3f2838)] {
            let cell = at(offset_cell)?;
            let mut matches = fields.iter().filter(|field| field.name() == name);
            let offset = matches.next().and_then(|field| usize::try_from(field.offset()).ok())
                .ok_or("Reference PalettePanel closure ivar absent")?;
            if matches.next().is_some() || offset < previous_end || !offset.is_multiple_of(8)
                || offset.checked_add(16).is_none_or(|end| end > class.instance_size())
                || reference_mapping_protection(cell, 8)? & 5 != 1
                || unsafe { (cell as *const usize).read_volatile() } != offset
            {
                return Err("Reference PalettePanel closure ivar layout differs".into());
            }
            previous_end = offset + 16;
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_palette_host_allocator(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        matches!(caller.checked_sub(base), Some(0x14cca0 | 0x1a6190))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_snippet_host_allocator(caller: usize) -> bool {
        (unsafe { _NSGetMachExecuteHeader() } as usize).checked_add(0x1a6190) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_palette_host_allocator(
        &self, caller: usize, class: &objc2::runtime::AnyClass,
    ) -> Result<()> {
        #[link(name = "SwiftUI", kind = "framework")]
        unsafe extern "C" {
            #[link_name = "$s7SwiftUI13NSHostingViewC04rootD0ACyxGx_tcfc"]
            static HOST_INIT: u8;
            #[link_name = "$s7SwiftUI13NSHostingViewCMn"]
            static HOST_DESCRIPTOR: u8;
        }
        if !Self::is_palette_host_allocator(caller) {
            return Err("Reference palette host allocation caller differs".into());
        }
        self.before_reference_publication()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference palette host address overflow");
        let snippet = caller == at(0x1a6190)?;
        let (body, length, hash) = if snippet {
            (0x1a6064, 388, "236f74cb1597a71166830ffb1582ae5a4f2f1455ee4224fb49b3608710739d99")
        } else {
            (0x14c958, 928, "114c3c827c6a25cffb16d30aa6d643032540047404ab1f8e828596306b2e6f62")
        };
        for (offset, bytes, sha) in [
            (body, length, hash),
            (0x448c, 64, "13296c97a082b144f6dd77b3e16cd8481b54f3c22192a4e16fb1f484556f66e1"),
            (0x2f9824, 12, "a581a615ccccce8410dcaa44b07fdbf75e63e0043e74c35f41c87377eecd8ce0"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, bytes, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != sha
            {
                return Err("Reference palette host code differs".into());
            }
        }
        // Each accessor stores its concrete specialization in a distinct cache.
        let (cache, record, mangled, record_bits, type_bytes, descriptor) = if snippet {
            (0x3f0278, 0x303158, 0x33f208, 0x0000_000c_0003_c0b0,
                b"\x02\x37\x6a\x08\x00y\x02\x01\x50\x08\x00G", (&raw const HOST_DESCRIPTOR) as usize)
        } else {
            (0x3f2840, 0x305200, 0x352984, 0x0000_000c_0004_d784,
                b"\x01w\xc2\x02\x00y\x01\xe1\xc1\x02\x00G", at(0x37ebfc)?)
        };
        if snippet {
            for (offset, expected) in [(0x3c5c40, descriptor), (0x3c4210, at(0x37f2a4)?)] {
                let cell = at(offset)?;
                reference_mapping(cell, size_of::<usize>(), Some(false))?;
                if unsafe { (cell as *const usize).read_volatile() } != expected {
                    return Err("Reference snippet host descriptor cell differs".into());
                }
            }
        }
        let (cache, record, mangled) = (at(cache)?, at(record)?, at(mangled)?);
        let class_address = class as *const _ as usize;
        reference_mapping(record, 8, Some(true))?;
        reference_mapping(mangled, 12, Some(true))?;
        if reference_mapping_protection(cache, 8)? & 7 != 3
            || reference_mapping_protection(class_address, 72)? & 5 != 1
            || unsafe { (record as *const u64).read_volatile() } != record_bits
            || unsafe { std::slice::from_raw_parts(mangled as *const u8, 12) } != type_bytes
            || unsafe { (cache as *const usize).read_volatile() } != class_address
            || unsafe { (class_address as *const usize).add(8).read_volatile() } != descriptor
        {
            return Err("Reference palette host concrete type differs".into());
        }
        let view = super::metadata::loaded_class(c"NSView")?.address();
        let mut parent = class.superclass();
        if !(0..16).any(|_| {
            let Some(current) = parent else { return false };
            parent = current.superclass();
            current as *const _ as usize == view
        }) {
            return Err("Reference palette host is not an NSView".into());
        }
        let symbol = c"$s7SwiftUI13NSHostingViewC04rootD0ACyxGx_tcfc";
        let entry = (&raw const HOST_INIT) as usize;
        let rows = super::imports::observe_unpatched_main_imports(&[(symbol, entry)])?;
        let [row] = rows.as_slice() else { return Err("Reference palette host initializer import absent".into()) };
        if row.cell_address() != at(0x3c5c30)? || row.original_address() != entry {
            return Err("Reference palette host initializer import differs".into());
        }
        row.verify_unpatched()?;
        reference_code(entry, Path::new("/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI"))?;
        let spec = super::bootstrap::source_allocation_with_zone_import_spec();
        let rows = super::observe_main_imports(&[spec])?;
        let [row] = rows.as_slice() else { return Err("Reference palette host allocator import absent".into()) };
        if row.cell_address() != at(0x3c6930)? || row.selected_address() != spec.replacement {
            return Err("Reference palette host allocator import differs".into());
        }
        reference_code(spec.original, Path::new("/usr/lib/libobjc.A.dylib"))?;
        self.registry_current()
    }

    /// Bind the Swift controller's panel ivar at either the allocating or the
    /// mounting call. The compiler keeps the same Swift self in x19 and has
    /// already stored the initialized panel at offset 0x18.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_palette_host_panel(
        &self, caller: usize, controller: usize, class: &objc2::runtime::AnyClass,
        mounting: bool, snippet_panel: usize,
    ) -> Result<*mut objc2::runtime::AnyObject> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference palette host address overflow");
        let snippet = caller == at(if mounting { 0x1a61e4 } else { 0x1a6190 })?;
        if !snippet && caller != at(if mounting { 0x14ccf8 } else { 0x14cca0 })? {
            return Err("Reference palette host controller caller differs".into());
        }
        self.verify_palette_host_allocator(at(if snippet { 0x1a6190 } else { 0x14cca0 })?, class)?;
        if mounting {
            let cell = at(0x3e8460)?;
            let selector = unsafe {
                std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::sel!(setContentView:))
            };
            if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
                || unsafe { (cell as *const usize).read_volatile() } != selector
            {
                return Err("Reference palette mount selector differs".into());
            }
        }
        if controller == 0 || !controller.is_multiple_of(8)
            || reference_mapping_protection(controller, 0x20)? & 7 != 3
            || unsafe { (controller as *const usize).read_volatile() } != at(if snippet { 0x3f44d0 } else { 0x3f2968 })?
        {
            return Err("Reference palette controller metadata differs".into());
        }
        // SnippetChoiceController stores its panel only after mounting. The
        // original present() keeps this new, initialized panel in x24.
        let panel = if snippet { snippet_panel } else { unsafe { (controller as *const usize).add(3).read_volatile() } };
        if panel == 0 || !panel.is_multiple_of(8) {
            return Err("Reference palette controller panel absent".into());
        }
        Ok(panel as *mut objc2::runtime::AnyObject)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_search_field_allocator(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        matches!(caller.checked_sub(base), Some(0x1593b0 | 0x159488))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_search_field_allocator(&self, caller: usize, class: usize) -> Result<super::metadata::MethodSnapshot> {
        self.before_reference_publication()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let (name, class_offset, parent, code, bytes, sha) = match caller.checked_sub(base) {
            Some(0x1593b0) => (c"_TtCV18RuycastF1Reference18PaletteSearchField11Coordinator", 0x3ea800,
                c"NSObject", 0x1593a4, 52, "4ca10d036d420894facfe5d4a96728f4b718f81b52ce54d0be62753151a8e0e3"),
            Some(0x159488) => (c"RuycastF1Reference.SearchInput", 0x3ea8c0,
                c"NSTextField", 0x159478, 36, "0525a809804717abd23135cbd053916bb8ba15e920747987142e7561ccf8fbc7"),
            _ => return Err("Reference search allocation caller differs".into()),
        };
        let at = |offset| base.checked_add(offset).ok_or("Reference search allocation address overflow");
        let known = super::metadata::loaded_class(name)?;
        if known.address() != class || at(class_offset)? != class
            || unsafe { &*(class as *const objc2::runtime::AnyClass) }.superclass()
                .map(|parent| parent as *const _ as usize) != Some(super::metadata::loaded_class(parent)?.address())
        {
            return Err("Reference search allocation class differs".into());
        }
        let address = at(code)?;
        reference_mapping(address, bytes, Some(true))?;
        if reference_image_header(address, &self.endpoint)? != base
            || digest(unsafe { std::slice::from_raw_parts(address as *const u8, bytes) }) != sha
        {
            return Err("Reference search allocation callsite bytes differ".into());
        }
        let spec = super::bootstrap::source_allocation_with_zone_import_spec();
        let rows = super::observe_main_imports(&[spec])?;
        let [row] = rows.as_slice() else { return Err("Reference search allocation import absent".into()) };
        let init = at(0x3e8480)?;
        let selector = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(objc2::sel!(init)) };
        if row.cell_address() != at(0x3c6930)? || row.selected_address() != spec.replacement
            || reference_mapping_protection(init, size_of::<usize>())? & 5 != 1
            || unsafe { (init as *const usize).read_volatile() } != selector
        {
            return Err("Reference search allocation import or initializer selector differs".into());
        }
        let objc = Path::new("/usr/lib/libobjc.A.dylib");
        reference_code(spec.original, objc)?;
        let (receiver, owner, image) = if code == 0x1593a4 {
            let spec = crate::workspace::factory::reference_edit_super_spec();
            let rows = super::observe_main_imports(&[spec])?;
            let [row] = rows.as_slice() else { return Err("Reference coordinator super import absent".into()) };
            if row.cell_address() != at(0x3c6948)? || row.selected_address() != spec.replacement {
                return Err("Reference coordinator super import differs".into());
            }
            reference_code(spec.original, objc)?;
            reference_code(spec.replacement, &self.sdk)?;
            let parent = super::metadata::loaded_class(c"NSObject")?;
            (parent, parent, objc.as_os_str().as_encoded_bytes())
        } else {
            let parent = super::metadata::loaded_class(c"NSView")?;
            (known, parent, b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".as_slice())
        };
        let initializer = super::metadata::method(receiver, c"init", false)?
            .ok_or("Reference search initializer absent")?;
        if initializer.owner() != owner.address() || initializer.code().image() != image
            || crate::workspace::simple_abi(&initializer)? != b"@@:"
        {
            return Err("Reference search initializer owner, image or ABI differs".into());
        }
        self.registry_current()?;
        Ok(initializer)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_configuration_read_allocator(caller: usize) -> bool {
        (unsafe { _NSGetMachExecuteHeader() } as usize).checked_add(0x80610) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_configuration_read_allocator(
        self: &Arc<Self>, caller: usize, class: usize, fd: i32, selected: Option<&super::metadata::MethodSnapshot>,
    ) -> Result<(usize, super::metadata::MethodSnapshot)> {
        if !Self::is_configuration_read_allocator(caller) || unsafe { libc::pthread_main_np() } == 1
            || !REFERENCE_LOCK_OWNER.get().is_some_and(|owner| Arc::ptr_eq(owner, self))
        {
            return Err("Reference configuration handle allocator owner differs".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        for (offset, length, sha) in [
            (0x80544, 24, "f2bdb74e6ba12872a923f90c3d5878935c8ddde224bf137f7d311dd05dd1c61d"),
            (0x80604, 52, "b7e549ad1366d6fc8e0e706dd8b3876f6349c6f72dff2dbf38d4318cc44eb61b"),
        ] {
            let code = base.checked_add(offset).ok_or("configuration handle code overflow")?;
            reference_mapping(code, length, Some(true))?;
            if reference_image_header(code, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(code as *const u8, length) }) != sha
            {
                return Err("Reference configuration handle code differs".into());
            }
        }
        let reference = base.checked_add(0x3e99b0).ok_or("configuration handle class ref overflow")?;
        if reference_mapping_protection(reference, size_of::<usize>())? & 5 != 1
            || unsafe { (reference as *const usize).read_volatile() } != class
            || super::metadata::loaded_class(c"NSFileHandle")?.address() != class
        {
            return Err("Reference configuration handle class differs".into());
        }
        let concrete = super::metadata::loaded_class(c"NSConcreteFileHandle")?;
        let init = super::metadata::method(concrete, c"initWithFileDescriptor:closeOnDealloc:", false)?
            .ok_or("Reference configuration handle initializer absent")?;
        if init.owner() != concrete.address() || init.encoding() != b"@24@0:8i16B20"
            || selected.is_some_and(|before| before != &init)
        {
            return Err("Reference configuration handle initializer owner/ABI/IMP differs".into());
        }
        reference_code(init.implementation(), Path::new("/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"))?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if fd < 0 || flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY
            || unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0
        {
            return Err("Reference configuration handle descriptor unavailable".into());
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_uid != self.grant.owner_uid || stat.st_mode & libc::S_IFMT != libc::S_IFREG
            || !(0..=32_000_000).contains(&stat.st_size)
        {
            return Err("Reference configuration handle descriptor shape differs".into());
        }
        self.registry_current()?;
        REFERENCE_CONFIG_READ.with(|slot| {
            if slot.get() != Some((fd, stat.st_dev, stat.st_ino)) {
                return Err("Reference configuration handle descriptor issuer differs".into());
            }
            if selected.is_some() {
                slot.set(None);
            }
            Ok((concrete.address(), init))
        })
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn reference_lock_route(caller: usize, frame: usize) -> Option<ReferenceLockRoute> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if matches!(caller.checked_sub(base), Some(0x1c7b08 | 0x7f898 | 0x17da10)) {
            return Some(ReferenceLockRoute::Main);
        }
        if base.checked_add(0x115578) != Some(caller) || !frame.is_multiple_of(16)
            || !reference_mapping_protection(frame, 16).is_ok_and(|protection| protection & 7 == 3)
        {
            return None;
        }
        let [parent, inner_return] = unsafe { (frame as *const [usize; 2]).read_volatile() };
        if base.checked_add(0x115544) != Some(inner_return) || frame.checked_add(32) != Some(parent)
            || !reference_mapping_protection(parent, 16).is_ok_and(|protection| protection & 7 == 3)
        {
            return None;
        }
        let outer_return = unsafe { (parent as *const usize).add(1).read_volatile() };
        match outer_return.checked_sub(base) {
            Some(0x1c5688 | 0x948a0 | 0x948bc | 0x7f3b0 | 0xde7c4) => Some(ReferenceLockRoute::Main),
            Some(0xdf364) => Some(ReferenceLockRoute::ControlClient),
            Some(0x937f8) => Some(ReferenceLockRoute::SafariSearch),
            _ => None,
        }
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn verify_reference_lock_caller(self: &Arc<Self>, caller: usize, frame: usize, class: usize, route: ReferenceLockRoute) -> Result<()> {
        if unsafe { Self::reference_lock_route(caller, frame) } != Some(route)
            || (unsafe { libc::pthread_main_np() } == 1) != (route == ReferenceLockRoute::Main)
        {
            return Err("Reference lock allocator caller/thread differs".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let class_ref = base.checked_add(0x3e99c0).ok_or("SystemServices lock class ref overflow")?;
        let direct = [
            (0x1c7afc, 28, "8b902e337d92369ec5266c7a69fec04d843b40a89884f23d13b821937d5507f1"),
            (0x7f88c, 28, "b54f5c428c3ba51dbe59d9c41b8d944e8bcc43254f071a8a9a62de955ef20a82"),
        ];
        let nested = [
            (0x1c565c, 44, "60925fedd2edc87219618749b120b0cad02e2943e86034453dc62055f2259599"),
            (0x1c5afc, 28, "fab22814b2a01479036d88154280d741250cfb6839e841a5a74597059d134f4d"),
            (0x115518, 112, "281c332fa234dbdf112d1af03a895b2734070b2bc60585f73b999928c67b36f6"),
            // BrowserTabs stores its own target map and consent state in
            // LockedValue. Only these two initializer calls share this route.
            (0x94894, 44, "517081a3e09680d1b2b69a4888e5b773c28c5900fcf71610dc7f3ea82f725b08"),
            (0x994bc, 20, "9203e9e2331f703fd84869047e228943e49a5a08bec4409bc335af9f07a308c7"),
            (0x7f3a0, 24, "25c4caba4a371f9be15b35eb028726b99f0844a3dc6d90044e80570e36a1539d"),
            (0xde7b0, 32, "71a0cd9a36ad8b7c9064561e14779c9792efa23c1c767b25ea1ad102797fdd65"),
            (0xdf350, 32, "bf119290807f8bd2031325bae1d8a14d249e8af3bda26053078a06c85776d6b1"),
            (0xdee88, 48, "c0a027214ed2ce2d8ab71c5610d8a2c9e4aab0d746227c408c3fe070112d41a2"),
            (0xdf0b0, 32, "8a51f544797eed2fd8e63d93d9829a17a049751b4ae571ce24950925fccdcb2b"),
            (0xdf230, 36, "0f662bdd6c1608b9b52ca97b515057e2d1285235a0f3c986b7199fbb094b732c"),
            (0x937e4, 36, "16ba05e61af23a9947829c43795b4c648f07550351f33bd2f4b8859bf8e2b654"),
        ];
        // AppState and SystemMonitorModel both construct the same original
        // ResourceSampler on main, before passing it to their worker queues.
        let sampler = [(0x17d9f8, 0x64, "2d136fe45ddff5d6cee9119c21155fdf4584eba97cb5ccc44bebbbefccabcb33")];
        let sites = match caller.checked_sub(base) {
            Some(0x1c7b08 | 0x7f898) => &direct[..],
            Some(0x17da10) => &sampler[..],
            _ => &nested[..],
        };
        for &(offset, length, sha) in sites {
            let code = base.checked_add(offset).ok_or("SystemServices lock code overflow")?;
            if reference_image_header(code, &self.endpoint)? != base {
                return Err("Reference SystemServices lock allocator image differs".into());
            }
            reference_mapping(code, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(code as *const u8, length) }) != sha {
                return Err("Reference SystemServices lock allocator code differs".into());
            }
        }
        // The dyld-bound class ref lives in writable __DATA, not a GOT page.
        if reference_mapping_protection(class_ref, size_of::<usize>())? & 5 != 1 {
            return Err("Reference SystemServices lock class ref mapping differs".into());
        }
        if unsafe { (class_ref as *const usize).read_volatile() } != class
            || super::metadata::loaded_class(c"NSLock")?.address() != class
        {
            return Err("Reference SystemServices lock allocator code or class differs".into());
        }
        if route == ReferenceLockRoute::SafariSearch
            && !crate::workspace::callbacks::reference_search_dispatch_active(self)
        {
            return Err("Safari activation lock outside the admitted search worker".into());
        }
        if route == ReferenceLockRoute::ControlClient {
            // receive keeps its client in x22 here. LockedValue saves x22
            // at x29-32 before entering the interposed allocator.
            let saved = frame.checked_sub(32).ok_or("ControlServer saved client underflow")?;
            if reference_mapping_protection(saved, size_of::<usize>())? & 7 != 3 {
                return Err("ControlServer saved client mapping differs".into());
            }
            let client = unsafe { (saved as *const usize).read_volatile() } as i32;
            if client < 0 || !control_clients().clients.contains(&client) {
                return Err("ControlServer lock has no accepted client".into());
            }
        }
        self.registry_current()
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) unsafe fn is_reference_website_image_caller(&self, frame: usize) -> Result<bool> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if !frame.is_multiple_of(16) || reference_mapping_protection(frame, 16)? & 7 != 3 {
            return Err("Reference website image frame mapping differs".into());
        }
        // The pinned NSImage thunk stores its caller LR next to the previous
        // frame pointer before entering the interposed zone allocation.
        let saved_caller = unsafe { (frame as *const usize).add(1).read_volatile() };
        Ok(base.checked_add(0x1e414c) == Some(saved_caller))
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) unsafe fn is_reference_website_lock_caller(&self, frame: usize) -> Result<bool> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if !frame.is_multiple_of(16) || reference_mapping_protection(frame, 16)? & 7 != 3 {
            return Err("Reference website lock frame mapping differs".into());
        }
        let [parent, caller] = unsafe { (frame as *const [usize; 2]).read_volatile() };
        if base.checked_add(0x115544) != Some(caller) || frame.checked_add(32) != Some(parent) {
            return Ok(false);
        }
        reference_mapping(parent, 16, Some(false))?;
        let caller = unsafe { (parent as *const usize).add(1).read_volatile() };
        Ok(base.checked_add(0x1e6854) == Some(caller))
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_reference_web_operation(&self, block: usize) -> Result<()> {
        unsafe extern "C" { static _NSConcreteMallocBlock: u8; }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3 {
            return Err("Reference website operation block mapping differs".into());
        }
        let words = unsafe { (block as *const [usize; 6]).read_volatile() };
        if words[0] != (&raw const _NSConcreteMallocBlock) as usize
            || words[1] & !0xfffe != 0x4300_0000
            || words[1] & 0xfffe == 0
            || base.checked_add(0x33e24) != Some(words[2])
            || base.checked_add(0x3d29c0) != Some(words[3])
            || base.checked_add(0x1e9ee4) != Some(words[4])
        {
            return Err("Reference website operation block differs".into());
        }
        // The original allocator at Main +0x1e2818 passes this fixed heap
        // metadata. URL captures have a runtime-dependent layout after the
        // two-word heap header, so inspect only the header here.
        let context = words[5];
        let metadata = base.checked_add(0x3d29a8).ok_or("Reference website capture metadata overflow")?;
        if context == 0 || !context.is_multiple_of(8)
            || reference_mapping_protection(context, 16)? & 7 != 3
            || unsafe { (context as *const usize).read_volatile() } != metadata
        {
            return Err("Reference website operation capture differs".into());
        }
        self.registry_current()
    }

    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_private_security_source_members(&self)->Result<()> {
        self.before_reference_initialization()?;
        let bytes=self.root.read_optional(Path::new(&self.grant.source_manifest.path),2*1024*1024)?
            .ok_or("Reference Security compiler manifest absent")?;
        if digest(&bytes)!=self.grant.source_manifest.sha256 {return Err("Reference Security compiler manifest changed".into());}
        let manifest:HostSourceManifest=serde_json::from_slice(&bytes).map_err(|_|"Reference Security manifest schema")?;
        for source in super::private_network::reference_source_roster() {
            let path=self.root.path().join(source.file);
            let path=path.to_str().ok_or("Reference Security source path encoding")?;
            let mut rows=manifest.sources.iter().filter(|row|row.path==path);
            let row=rows.next().ok_or("Reference Security caller not compiler-bound")?;
            if rows.next().is_some()||row.sha256!=source.source_sha256 {return Err("Reference Security compiler member differs/duplicated".into());}
        }
        self.before_reference_initialization()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_utility_imports(&self, imports: &[super::ImportReplacement]) -> Result<()> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference Utilities import offset overflow");
        for (symbol, cell, caller, code, stub, stub_code) in [
            (c"SecItemCopyMatching", 0x3c5a00, 0x1d4ea4, "39706c53d3adaf3afdadd2f9a52a3218afb1d52d9b4f255a1065d8bafa16d9f7", 0x2fb498, "d75b0ba5c4cadab444d8fe46851a4c185f17fe8db8687d165e1b91945e19a467"),
            (c"SecItemUpdate", 0x3c5a08, 0x1d50b0, "fd732ec7cac7f5c83d772420a2f7c934ca2b546896e398ea3f8cab41e5c1a1b7", 0x2fb4a4, "64a5c35902c38c6c34a85ebf504f995bc45ed896ae814616fbd1694c8740c8ea"),
            (c"SecItemAdd", 0x3c59f8, 0x1d5138, "9fd603e5ebc26a4b440ae2585979258a1a362c54f2f7ae536d0d0f67f1001853", 0x2fb48c, "2488e774f119dc3719f2fe459951ad849f8afef40844514a37884de780ba6221"),
            (c"SecRandomCopyBytes", 0x3c5a10, 0x1d5268, "9e3aa668e917906a4897a69a98973d5d2ee0cdf4263aec05bed4191675815581", 0x2fb4b0, "cde15179f8d8a936689cbe6044567430a7ab22249e11d0b495df2e49f61b39f2"),
        ] {
            for (offset, length, expected) in [(caller, 20, code), (stub, 12, stub_code)] {
                let address = at(offset)?;
                reference_mapping(address, length, Some(true))?;
                if reference_image_header(address, &self.endpoint)? != base
                    || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
                {
                    return Err("Reference Utilities import caller or stub differs".into());
                }
            }
            let import = imports.iter().find(|row| row.symbol == symbol).ok_or("Reference Utilities import absent")?;
            if import.cell_address() != at(cell)? || import.selected_address() != import.replacement {
                return Err("Reference Utilities import cell or selection differs".into());
            }
        }
        self.before_reference_initialization()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_keychain_reader(&self) -> Result<usize> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let entry = base.checked_add(0x1d4dc0).ok_or("Reference Keychain reader offset overflow")?;
        const LENGTH: usize = 0x214;
        reference_mapping(entry, LENGTH, Some(true))?;
        if reference_image_header(entry, &self.endpoint)? != base
            || digest(unsafe { std::slice::from_raw_parts(entry as *const u8, LENGTH) })
                != "391a350258c8c1bf9e72f018b683d44839b450009e1dd0669ac21436baed043b"
        {
            return Err("Reference original Keychain reader differs".into());
        }
        Ok(entry)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_keychain_writer(&self) -> Result<usize> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let entry = base.checked_add(0x1d4fe0).ok_or("Reference Keychain writer offset overflow")?;
        const LENGTH: usize = 0x228;
        reference_mapping(entry, LENGTH, Some(true))?;
        if reference_image_header(entry, &self.endpoint)? != base
            || digest(unsafe { std::slice::from_raw_parts(entry as *const u8, LENGTH) })
                != "9f9162ffce01d86383480d99401c2505a66c24f4b32cfe3767e42c4d78edd37f"
        {
            return Err("Reference original Keychain writer differs".into());
        }
        Ok(entry)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn reference_utility_entropy(&self) -> Result<usize> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let entry = base.checked_add(0x1d5214).ok_or("Reference entropy entry overflow")?;
        const LENGTH: usize = 0xbc;
        reference_mapping(entry, LENGTH, Some(true))?;
        if reference_image_header(entry, &self.endpoint)? != base
            || digest(unsafe { std::slice::from_raw_parts(entry as *const u8, LENGTH) })
                != "5be3c4b5c87f37910194eda26a9517d81755f775ed6eddd17fbc12f3de12b477"
        {
            return Err("Reference original entropy function differs".into());
        }
        Ok(entry)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn reference_monitor_sysctl_entries(&self, import: &super::ImportReplacement) -> Result<[usize; 3]> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference monitor sysctl offset overflow");
        for (offset, length, expected) in [
            (0x17b0d8, 0x268, "71ca27fea454108f81b1b5571b35849d8f2501a474b230a8aae3d25d5dea5710"),
            (0x17b3ac, 0x98, "bd5d1a9876e775af3d6768a1fb301be20c0cd0a2651a3a5a2bda70b80e107604"),
            (0x17b444, 0xd4, "ffebbe5fff3f494dd1cf2a2665024bbe5aebdcc104e57e98f47df6df18692451"),
            (0x180764, 12, "0826364401d62c06c1b23418e99d68c5032144b5f3e89be278ca898b2026f2ff"),
            (0x2fc368, 12, "82a58f491babc89c9df6f8acea9654625de26917a0c3a1bafd2c015f1105f6f3"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, length, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference monitor source or sysctl stub differs".into());
            }
        }
        if import.symbol != c"sysctlbyname" || import.cell_address() != at(0x3c4990)?
            || import.selected_address() != super::system_providers::owned_sysctlbyname as *const () as usize
        {
            return Err("Reference monitor import is not the owned sysctl leaf".into());
        }
        Ok([at(0x17b0d8)?, at(0x17b3ac)?, at(0x17b444)?])
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_original_monitor_process_table(
        &self,
        module: &super::ModuleAttestation,
    ) -> Result<()> {
        #[link(name = "swiftCore")]
        unsafe extern "C" {
            fn swift_errorRelease(error: usize);
            fn swift_bridgeObjectRelease();
        }
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let entry = base.checked_add(0x17c954).ok_or("Reference process table entry overflow")?;
        let parser = entry.checked_add(0x8e8).ok_or("Reference process table parser overflow")?;
        for (address, size, expected) in [
            (entry, 0x8e8, "5cf299d567e58a5ee166787de8331ad81ee78b58be1ff7ccd228febdf84f45e6"),
            (parser, 0x724, "e11754bbeb41df73b0203cb8f0e39a9c53b698bc15fc1d2fe5895be3745cda3e"),
        ] {
            reference_mapping(address, size, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, size) }) != expected
            {
                return Err("Reference original process table or parser differs".into());
            }
        }
        let fixture = super::system_providers::installed_fixture()?;
        let expected = fixture.processes.lock().map_err(|_| "Reference process rows poisoned")?
            .get(&super::system_providers::OWNED_EDITOR_PID)
            .cloned().ok_or("Reference owned process row absent")?;
        module.verify()?;
        let mut error = 0;
        let (array, stage) = super::system_providers::monitor_timeout::observe_assembly_probe(
            || unsafe { call_reference_process_table(entry, &mut error) },
        );
        if error != 0 {
            unsafe { swift_errorRelease(error) };
            return Err(format!("Reference original process table threw after {stage}").into());
        }
        let result = (|| -> Result<()> {
            if array == 0 || array & 7 != 0 || reference_mapping_protection(array, 32)? & 5 != 1 {
                return Err("Reference process table did not return native Swift storage".into());
            }
            // The pinned producer stores count at +16 and appends each
            // 72-byte ProcessResource at +32 in native contiguous storage.
            if unsafe { ((array + 16) as *const usize).read() } != 1 {
                return Err("Reference process table did not return exactly one row".into());
            }
            if reference_mapping_protection(array, 32 + 72)? & 5 != 1 {
                return Err("Reference process row is outside readable native storage".into());
            }
            let row = (array + 32) as *const u8;
            let word = |offset| unsafe { row.add(offset).cast::<u64>().read() };
            let pid_and_uid = word(0);
            if pid_and_uid as u32 as i32 != expected.pid
                || (pid_and_uid >> 32) as u32 != expected.uid
                || word(8) != expected.started_seconds
                || word(16) != expected.started_micros
                || word(56) != expected.cpu.to_bits()
                || word(64) != expected.rss_kib * 1024
            {
                return Err("Reference process row identity, CPU or memory differs".into());
            }
            Ok(())
        })();
        let release: unsafe extern "C" fn(usize) =
            unsafe { std::mem::transmute(swift_bridgeObjectRelease as *const ()) };
        unsafe { release(array) };
        result?;
        module.verify()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn reference_monitor_sample_entries(&self) -> Result<[usize; 6]> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference monitor constructor offset overflow");
        for (offset, size, expected) in [
            (0x17f398, 0x14, "336dc79e74309a7f075ca3b52b7e9a801e4c4c0afd89239d8e81f6fa37601edf"),
            (0x17d9c8, 0x30, "6ff54a22fcc9b3abf448ab896ecdda626b4cb196a60152a4b94c07c8b57e766a"),
            (0x17d9f8, 0x64, "2d136fe45ddff5d6cee9119c21155fdf4584eba97cb5ccc44bebbbefccabcb33"),
            (0x2fbaec, 12, "cfcb0bf56e07e38ed89130ff04f8c5eeed1dd8e1ec7378d52306d6f28a1e18dd"),
            (0x17b60c, 0xc00, "5e9a706448b9d584758d8fc14edc1c0ec53cca2332ff5f3b3616990adb21cf2b"),
            (0x178de4, 0x14, "16d79c664fb1e78fcd15384d671337c7879612878beb1fd363fe6e2d3846fc7b"),
            (0x17ee80, 0x64, "fcb2d20cb371083b74317e4402b375cb9918ec4313996cb19777c3dd145fada0"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, size, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, size) }) != expected
            {
                return Err("Reference original monitor constructor differs".into());
            }
        }
        let allocation_cell = at(0x3c6930)?;
        reference_mapping(allocation_cell, size_of::<usize>(), Some(false))?;
        Ok([
            at(0x17f398)?, at(0x17d9c8)?, allocation_cell, at(0x17b60c)?,
            at(0x178de4)?, at(0x17ee80)?,
        ])
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn is_monitor_constructor_lock(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        base.checked_add(0x17da10) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_monitor_snapshot_layout(&self, metadata: *const usize, destroy: usize) -> Result<usize> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let expected_metadata = base.checked_add(0x3f3898).ok_or("Reference monitor metadata overflow")?;
        let expected_witnesses = base.checked_add(0x3f3830).ok_or("Reference monitor witness overflow")?;
        if metadata as usize != expected_metadata
            || reference_mapping_protection(expected_metadata - 8, 8 + 0x6c)? & 5 != 1
            || reference_mapping_protection(expected_witnesses, 88)? & 5 != 1
        {
            return Err("Reference monitor snapshot metadata address or mapping differs".into());
        }
        let witnesses = unsafe { metadata.sub(1).read() } as *const usize;
        if witnesses as usize != expected_witnesses || unsafe { metadata.read() } != 0x200
            || unsafe { witnesses.add(1).read() } != destroy
        {
            return Err("Reference monitor snapshot destruction witness differs".into());
        }
        let size = unsafe { witnesses.add(8).read() };
        let alignment_mask = unsafe { witnesses.add(10).read() } & 0xff;
        if !(8..=512).contains(&size) || alignment_mask > 15 {
            return Err("Reference monitor snapshot layout exceeds owned storage".into());
        }
        Ok(size)
    }
    /// Match the original process and monitor callers against the sealed
    /// compiler manifest. The caller first verifies the full source set.
    #[cfg(all(not(test), rcf1_reference_sdk))]
    pub(crate) fn verify_process_source_members(&self) -> Result<()> {
        let bytes = self.root.read_optional(Path::new(&self.grant.source_manifest.path), 2 * 1024 * 1024)?
            .ok_or("Reference process compiler manifest absent")?;
        if digest(&bytes) != self.grant.source_manifest.sha256 {
            return Err("Reference process compiler manifest changed".into());
        }
        let manifest: HostSourceManifest = serde_json::from_slice(&bytes)
            .map_err(|_| "Reference process compiler manifest schema")?;
        let mut count = 0;
        for source in super::process_router::source_roster() {
            if !source.file.starts_with("Sources/RuycastApp/") {
                continue;
            }
            let path = self.root.path().join("bench/reference/0.8.26").join(source.file);
            let path = path.to_str().ok_or("Reference process source path encoding")?;
            let mut rows = manifest.sources.iter().filter(|row| row.path == path);
            if !rows.next().is_some_and(|row| row.sha256 == source.source_sha256)
                || rows.next().is_some()
            {
                return Err(format!("Reference process compiler member differs: {}", source.file));
            }
            count += 1;
        }
        if count != 6 {
            return Err("Reference process compiler roster differs".into());
        }
        self.registry_current()
    }
    #[cfg(rcf1_reference_sdk)]
    fn input_context_identity(&self,context:&super::owned::ModuleContext)->Result<()> {
        if self.owner!=std::thread::current().id() || !Arc::ptr_eq(&context.life,&self.admitted.life)
            || super::READY_RETIRED.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Reference input context guardian/owner retired or differs".into());
        }
        for (candidate,expected) in [&context.home,&context.cache,&context.temp,&context.applications].into_iter().zip(&self.roots) {
            if candidate.path()!=expected.path() {return Err("Reference input context root path differs".into());}
            let (actual,pinned)=(candidate.checked(candidate.path())?,expected.checked(expected.path())?);
            let (mut a,mut b):(libc::stat,libc::stat)=unsafe{(std::mem::zeroed(),std::mem::zeroed())};
            if unsafe{libc::fstat(actual.as_raw_fd(),&mut a)}!=0 || unsafe{libc::fstat(pinned.as_raw_fd(),&mut b)}!=0
                || a.st_dev!=b.st_dev || a.st_ino!=b.st_ino || a.st_uid!=self.grant.owner_uid
                || a.st_mode&libc::S_IFMT!=libc::S_IFDIR || a.st_mode&0o077!=0 {
                return Err("Reference input context physical root differs".into());
            }
        }
        Ok(())
    }
    /// The fixed clock adapter obtains return_pc from its actual assembly
    /// entry. This origin check grants no input callback or userdata authority.
    /// Its installed Fixture/clock-table and failure-fence checks must also run
    /// before and after this predicate on the same Process Registry.
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    pub(crate) fn before_monitor_clock_callback(&self,return_pc:usize,registry:&super::process_dispatch::Registry)->Result<()> {
        registry.matches_issuer(&self.admitted.life,&self.admitted.descriptors)?;
        registry.not_retired()?;
        if self.owner==std::thread::current().id() && unsafe{libc::pthread_main_np()}==1 {
            self.before_reference_initialization()?;
        } else {
            #[cfg(all(not(test), target_arch = "aarch64"))]
            let website = super::private_network::qualify_reference_website_clock(self, return_pc)?;
            #[cfg(any(test, not(target_arch = "aarch64")))]
            let website = false;
            if !website {
                self.before_reference_worker_callback()?;
            }
        }
        registry.not_retired()?;
        let caller=return_pc.checked_sub(4).filter(|pc|pc.is_multiple_of(4))
            .ok_or("Reference clock return PC is not an actual aligned call boundary")?;
        if super::metadata::code_identity(caller)?.image()!=self.endpoint.as_os_str().as_encoded_bytes() {
            return Err("Reference clock caller is not its sealed MAIN image".into());
        }
        registry.not_retired()?;
        if super::READY_RETIRED.load(std::sync::atomic::Ordering::Acquire) || !self.admitted.life.is_alive() {
            return Err("Reference clock issuer retired during verification".into());
        }
        if super::READY_RETIRED.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Reference clock retired during the final guardian check".into());
        }
        self.registry_current()?;
        registry.not_retired()
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_monitor_process_info_callers(&self) -> Result<[(usize, usize); 13]> {
        use objc2::runtime::Sel;
        self.before_reference_initialization()?;
        reference_mapping_infrastructure()?;
        let header = unsafe { _NSGetMachExecuteHeader() } as usize;
        // Frozen default and decoding initializers: ADRP/LDR then BL objc_msgSend.
        let selectors = [
            (c"processorCount", 0x3e8ef8, 0xf9477d01u32),
            (c"operatingSystemVersionString", 0x3e8ef0, 0xf9477901),
            (c"hostName", 0x3e8f78, 0xf947bd01),
            (c"systemUptime", 0x3e8858, 0xf9442d01),
        ];
        let mut return_pcs = [(0, 0); 13];
        for (slot, (index, offset, branch)) in return_pcs[..11].iter_mut().zip([
            (0, 0x17a310, 0x940605fb), (1, 0x17a358, 0x940605e9),
            (2, 0x17a398, 0x940605d9), (3, 0x17a3d4, 0x940605ca),
            (0, 0x17a5ac, 0x94060554), (1, 0x17a600, 0x9406053f),
            (2, 0x17a650, 0x9406052b), (3, 0x17a6ac, 0x94060514),
            (0, 0x17ad88, 0x9406035d),
            // The benchmark's environment readback uses the same fixture.
            (1, 0x15d0f4, 0x94067a82), (0, 0x15d1a0, 0x94067a57),
        ]) {
            let (name, cell_offset, load) = selectors[index];
            let page = if matches!(offset, 0x15d0f4 | 0x15d1a0) { 0xf0001448 } else { 0xd0001368 };
            let words = [page, load, branch];
            let code = header.checked_add(offset).ok_or("Reference monitor caller overflow")?;
            let cell = header.checked_add(cell_offset).ok_or("Reference monitor selector cell overflow")?;
            if reference_image_header(code, &self.endpoint)? != header {
                return Err("Reference monitor caller is not in actual Main".into());
            }
            reference_mapping(code, 12, Some(true))?;
            let protection = reference_mapping_protection(cell, std::mem::size_of::<usize>())?;
            if protection & 1 == 0 || protection & 4 != 0 {
                return Err("Reference monitor selector cell is not readable nonexecuting data".into());
            }
            let selector = unsafe { std::mem::transmute::<Sel, usize>(Sel::register(name)) };
            let actual = unsafe { (code as *const [u32; 3]).read_volatile() };
            if actual != words || unsafe { (cell as *const usize).read_volatile() } != selector {
                return Err(format!("Reference monitor caller or selector differs for {name:?}"));
            }
            *slot = (code.checked_add(12).ok_or("Reference monitor return PC overflow")?, index);
        }
        // Both memberwise String defaults tail-branch to the same helper.
        // Their distinct selector-cell addresses remain inputs to its LDR.
        for (offset, words) in [
            (0x178f90, [0x90001380u32, 0x913bc000, 0x14000785]),
            (0x179010, [0xf0001360, 0x913de000, 0x14000765]),
            (0x17ade0, [0xaa0003f4, 0xf9400261, 0x94060347]),
        ] {
            let code = header.checked_add(offset).ok_or("Reference monitor default code overflow")?;
            if reference_image_header(code, &self.endpoint)? != header {
                return Err("Reference monitor default code is not in actual Main".into());
            }
            reference_mapping(code, 12, Some(true))?;
            if unsafe { (code as *const [u32; 3]).read_volatile() } != words {
                return Err("Reference monitor default code differs".into());
            }
        }
        let string_return = header.checked_add(0x17adec).ok_or("Reference monitor default return PC overflow")?;
        return_pcs[11] = (string_return, 1);
        return_pcs[12] = (string_return, 2);
        self.registry_current()?;
        Ok(return_pcs)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_monitor_process_info_main_caller(&self, return_pc: usize) -> Result<bool> {
        let caller = return_pc.checked_sub(4).filter(|address| address.is_multiple_of(4))
            .ok_or("Reference ProcessInfo return address differs")?;
        let code = super::metadata::code_identity(caller)?;
        Ok(code.image() == self.endpoint.as_os_str().as_encoded_bytes())
    }
    /// Qualify the existing process-table timeout source before any live
    /// work item exists. These pins confer no block or capture ownership.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_monitor_timeout_source(&self) -> Result<([usize; 5], [usize; 4], usize)> {
        #[link(name = "swiftDispatch")]
        unsafe extern "C" {
            #[link_name = "$s8Dispatch0A8WorkItemC5flags5blockAcA0abC5FlagsV_yyXBtcfc"]
            static WORK_ITEM_INIT: u8;
            #[link_name = "$sSo17OS_dispatch_queueC8DispatchE10asyncAfter8deadline7executeyAC0D4TimeV_AC0D8WorkItemCtF"]
            static SCHEDULE: u8;
            #[link_name = "$s8Dispatch0A8WorkItemC6cancelyyFTj"]
            static CANCEL: u8;
        }
        let init = c"$s8Dispatch0A8WorkItemC5flags5blockAcA0abC5FlagsV_yyXBtcfc";
        let schedule = c"$sSo17OS_dispatch_queueC8DispatchE10asyncAfter8deadline7executeyAC0D4TimeV_AC0D8WorkItemCtF";
        let cancel = c"$s8Dispatch0A8WorkItemC6cancelyyFTj";
        self.before_reference_initialization()?;
        reference_mapping_infrastructure()?;
        let header = unsafe { _NSGetMachExecuteHeader() } as usize;
        for (offset, length, expected) in [
            (0x17cbfc, 20, "e718b437a8cdffa090240ef361803057a759e758c4216f7de500e395f8ea1510"),
            (0x17cc1c, 88, "ff24516b268349029c1ea9576796ae093562a0299ca32677936bcedf5dde0c3a"),
            (0x17cd1c, 32, "5660d0256795d9a3da7e9521f956e355c716e544394328a6900133f4fb52b23f"),
            (0x17cd48, 128, "cd1b96dc7719307afc01db58de0fe5a184272794ddead905f128f553f5ce4ad4"),
            (0x17cdc8, 12, "e0df05df0cccb35bb08eccb7be173aeaa170ee93111ec3f5fd8196760dca0747"),
            (0x17cde4, 0x54, "961734f7eed2d757c89dc0a4778be40d0e0ac66396d6ca2a799aad4f89b7bed9"),
            (0x17d030, 16, "09205d3911cbdc6c0602f5c656430cb3cde09a2b4421fc551e2da30b08b1d6b8"),
            (0x17d210, 16, "dd6e88ab3965193f9d297da387e55d2259daf1a1f86626dfc58bcbd44028439d"),
            (0x17eaf0, 36, "03b8a37d90317c52e5cd639a6a460ebc2859169d6a6db87c929e8ba4c44c89f0"),
            (0x17eb14, 92, "d2e2e344597c349ab202d31e0c9ddda6beccc8b2b246b60e92e244dfce45fa72"),
            (0x1c9250, 0x14, "c8e87c04c751b8e8d1666ab7f40b40ca0f8caa0b86ae6f828b92cd561b7ed1f3"),
            (0x1c4cec, 0x18, "cf1410e9320a34bfa3c65fb6aefec5642f26dc9e430f71a401789ea75ed28154"),
            (0x1c9324, 0x94, "0e49e8a8c562d099eba0d988911424249a6a2df4a8cc823d7072f8e410038b0a"),
            (0x1c9410, 0xc4, "de16441788e1e4c19063828a801636982e5ba5f8b669f1cc79ba9c8d968aef2e"),
            (0x1cd198, 0xc, "bbd3cca5601775e26d2be872629037429911cf108bb783be8007d3e84df5bc7e"),
            (0x1d3cd0, 0x30, "800f0828b3c693d64ac267a76cbca14a867cb346e3b4763b45ecc08064e787f8"),
            (0x1d3d38, 4, "26d66fbb25c6079ae0f3fcf794210a9bd8d7038a193295e428ffcaa5d6b7570c"),
            (0x1d3d74, 0x10, "2a32272fd4c95e8054dc837598b1d44b38dbba1074253f9eb7bd6fcaff2d854d"),
            (0x1cf208, 0x18, "d2e2a5722f501fcc9a51ecbe8389f19a44d1ed338514d7edbeaf0d966b683cfa"),
        ] {
            let address = header.checked_add(offset).ok_or("Reference monitor timeout code overflow")?;
            if reference_image_header(address, &self.endpoint)? != header {
                return Err("Reference monitor timeout code is not in actual Main".into());
            }
            reference_mapping(address, length, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected {
                return Err(format!("Reference monitor timeout code differs at {offset:#x}"));
            }
        }
        let at = |offset| header.checked_add(offset).ok_or("Reference monitor capture offset overflow");
        let context = at(0x3cfa90)?;
        reference_mapping(context, 40, Some(false))?;
        if unsafe { (context as *const [usize; 5]).read_volatile() }
            != [at(0x17eaf0)?, 0, 0x400, 16, at(0x37cb38)?]
        {
            return Err("Reference monitor Task capture metadata differs".into());
        }
        let descriptor = at(0x3cfab8)?;
        reference_mapping(descriptor, 40, Some(false))?;
        if unsafe { (descriptor as *const [usize; 5]).read_volatile() }
            != [0, 48, at(0x17eb58)?, at(0x17eb68)?, at(0x3672b0)?]
        {
            return Err("Reference monitor timeout block descriptor differs".into());
        }
        let signature = at(0x3672b0)?;
        reference_mapping(signature, 6, None)?;
        if unsafe { (signature as *const [u8; 6]).read_volatile() } != *b"v8@?0\0" {
            return Err("Reference monitor timeout block ABI differs".into());
        }
        let file_context = at(0x3d20f0)?;
        reference_mapping(file_context, 40, Some(false))?;
        if unsafe { (file_context as *const [usize; 5]).read_volatile() }
            != [at(0x1d3cd0)?, 0, 0x400, 16, at(0x37d328)?]
        {
            return Err("Reference file search Task capture metadata differs".into());
        }
        let file_descriptor = at(0x3d2118)?;
        reference_mapping(file_descriptor, 40, Some(false))?;
        if unsafe { (file_descriptor as *const [usize; 5]).read_volatile() }
            != [0, 48, at(0x1d3d38)?, at(0x1d3d74)?, signature]
        {
            return Err("Reference file search timeout block descriptor differs".into());
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (init, (&raw const WORK_ITEM_INIT) as usize),
            (schedule, (&raw const SCHEDULE) as usize),
            (cancel, (&raw const CANCEL) as usize),
        ])?;
        for (symbol, offset) in [(init, 0x3c7618), (schedule, 0x3c7660), (cancel, 0x3c7620)] {
            let address = header.checked_add(offset).ok_or("Reference monitor timeout import overflow")?;
            reference_mapping(address, std::mem::size_of::<usize>(), Some(false))?;
            let mut matches = imports.iter().filter(|row| row.symbol == symbol);
            let row = matches.next().ok_or("Reference monitor timeout import absent")?;
            if matches.next().is_some() || row.cell_address() != address {
                return Err("Reference monitor timeout import cell differs".into());
            }
            reference_code(row.original_address(), Path::new("/usr/lib/swift/libswiftDispatch.dylib"))?;
            row.verify_unpatched()?;
        }
        self.registry_current()?;
        Ok((
            [at(0x17cd3c)?, at(0x17cdd4)?, at(0x17d038)?, at(0x17d218)?, at(0x17cc10)?],
            [at(0x1c9430)?, at(0x1c94d0)?, at(0x1cd1a0)?, at(0x1c4d00)?],
            at(0x17ce10)?,
        ))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) unsafe fn monitor_timeout_task(&self, block: usize, files: bool) -> Result<usize> {
        unsafe extern "C" { static _NSConcreteMallocBlock: u8; }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference monitor timeout address overflow");
        if !block.is_multiple_of(8) || reference_mapping_protection(block, 48)? & 7 != 3 {
            return Err("Reference monitor timeout block mapping differs".into());
        }
        let words = unsafe { (block as *const [usize; 6]).read_volatile() };
        let (descriptor, metadata) = if files { (0x3d2118, 0x3d2100) } else { (0x3cfab8, 0x3cfaa0) };
        if words[0] != (&raw const _NSConcreteMallocBlock) as usize
            || words[1] & !0xfffe != 0x4300_0000
            || words[1] & 0xfffe == 0
            || words[2] != at(0x81bf4)?
            || words[3] != at(descriptor)?
            || words[4] != at(0x17eb14)?
        {
            return Err("Reference monitor timeout heap block differs".into());
        }
        let context = words[5];
        if !context.is_multiple_of(8) || reference_mapping_protection(context, 24)? & 7 != 3 {
            return Err("Reference monitor timeout context mapping differs".into());
        }
        let capture = unsafe { (context as *const [usize; 3]).read_volatile() };
        if capture[0] != at(metadata)? || capture[2] == 0 {
            return Err("Reference monitor timeout Task capture differs".into());
        }
        self.registry_current()?;
        Ok(capture[2])
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// Both pointers must come from the qualified Swift metadata accessors and
    /// remain readable for the process lifetime.
    pub(crate) unsafe fn verify_monitor_timeout_abi(item_metadata: usize, time_metadata: usize) -> Result<()> {
        reference_mapping_infrastructure()?;
        let metadata_readable = |address: usize, bytes| -> Result<()> {
            if !address.is_multiple_of(8) || reference_mapping_protection(address, bytes)? & 5 != 1 {
                return Err("Reference monitor Dispatch metadata mapping differs".into());
            }
            Ok(())
        };
        metadata_readable(item_metadata, 0x38)?;
        let item = item_metadata as *const u8;
        if unsafe { item.add(0x30).cast::<u32>().read_volatile() } != 24
            || unsafe { item.add(0x34).cast::<u16>().read_volatile() } != 7
        {
            return Err("Reference WorkItem metadata instance layout differs".into());
        }
        let witness_cell = time_metadata.checked_sub(8).ok_or("Reference DispatchTime metadata absent")?;
        metadata_readable(witness_cell, 8)?;
        let witness = unsafe { (witness_cell as *const usize).read_volatile() };
        metadata_readable(witness, 88)?;
        let tail = (witness + 64) as *const u64;
        let size = unsafe { tail.read_volatile() };
        let stride = unsafe { tail.add(1).read_volatile() };
        let flags = unsafe { tail.add(2).cast::<u32>().read_volatile() };
        if size != 8 || stride != 8 || flags != 0x0200_0007 {
            return Err("Reference DispatchTime value layout differs".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    /// The caller must hold a live Swift borrow or a retained +1 throughout.
    pub(crate) unsafe fn verify_monitor_work_item(item: usize, metadata: usize) -> Result<()> {
        if !item.is_multiple_of(8) || reference_mapping_protection(item, 24)? & 7 != 3
            || unsafe { (item as *const usize).read_volatile() } != metadata
        {
            return Err("Reference monitor WorkItem instance metadata differs".into());
        }
        Ok(())
    }

    /// Main-only durable output witness after the real Reference owner has
    /// completed its persistence work. No arbitrary path or worker grant.
    pub fn process_ranking_bytes(&self)->Result<Vec<u8>> {
        self.before_reference_publication()?;
        reference_process_ranking_bytes(&self.roots[0],Path::new(&self.grant.runtime.profile),self.grant.owner_uid,
            ||self.before_reference_publication())
    }
    pub fn fixture_epoch_unix_seconds(&self)->Result<u64> {
        self.before_reference_initialization()?;
        let epoch=reference_fixture_epoch(&self.root,&self.grant.runtime.fixture,self.grant.owner_uid)?;
        self.before_reference_initialization()?;
        Ok(epoch)
    }
    pub fn ready_facts(&self) -> (&str,&str,&str) {
        (&self.grant.runtime.fixture.sha256,&self.grant.runtime.authority_sha256,&self.grant.sdk_artifact_sha256)
    }
    pub fn runtime_roots(&self) -> Result<[OwnedRoot;4]> {
        self.before_reference_initialization()?;
        Ok(self.roots.clone())
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn website_assembly_cache(&self, path: &Path) -> Result<Option<&OwnedRoot>> {
        let cache = &self.roots[1];
        if path != cache.path().join("website-assembly")
            || !super::private_network::reference_website_assembly_active()
        {
            return Ok(None);
        }
        super::private_network::qualify_reference_website_cache_directory(self)?;
        Ok(Some(cache))
    }
    pub fn profile(&self) -> Result<&Path> {
        self.before_reference_initialization()?;
        Ok(Path::new(&self.grant.runtime.profile))
    }
    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    pub(crate) fn verify_control_clients_drained(&self) -> Result<()> {
        use std::time::{Duration, Instant};
        if unsafe { libc::pthread_main_np() } != 1
            || !REFERENCE_LOCK_OWNER.get().is_some_and(|owner| std::ptr::eq(owner.as_ref(), self))
        {
            return Err("Reference ControlServer client drain owner differs".into());
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            {
                let state = control_clients();
                if !state.stopped {
                    return Err("Reference ControlServer client ingress was not stopped".into());
                }
                if !state.accepting && state.clients.is_empty() {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err("Reference ControlServer accepted client close unconfirmed".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    pub(crate) fn verify_stopped_startup_descriptors(&self) -> Result<()> {
        if unsafe { libc::pthread_main_np() } != 1
            || !REFERENCE_LOCK_OWNER.get().is_some_and(|owner| std::ptr::eq(owner.as_ref(), self))
        {
            return Err("Reference startup descriptor cleanup owner differs".into());
        }
        verify_reference_closed_descriptors(&[
            ("watcher", REFERENCE_WATCH_FD.with(std::cell::Cell::get)),
            ("control lock", REFERENCE_CONTROL_LOCK.with(std::cell::Cell::get)),
            ("control listener", REFERENCE_CONTROL_LISTENED.with(std::cell::Cell::get)),
        ])?;
        REFERENCE_WATCH_FD.with(|slot| slot.set(None));
        REFERENCE_CONTROL_LOCK.with(|slot| slot.set(None));
        REFERENCE_CONTROL_LISTENED.with(|slot| slot.set(None));
        Ok(())
    }
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_startup_profile(&self) -> Result<()> {
        // ConfigurationLocation must use the pinned profile. ControlServer
        // must create its own lock and socket, without probing a prior server.
        let profile = Path::new(&self.grant.runtime.profile);
        let config = profile.join("config.json");
        let config = config.to_str().ok_or("Reference startup config path encoding")?;
        let bytes = self.root.read_optional(Path::new(&self.grant.source_manifest.path), 2 * 1024 * 1024)?
            .ok_or("Reference startup manifest absent")?;
        if digest(&bytes) != self.grant.source_manifest.sha256 {
            return Err("Reference startup manifest changed".into());
        }
        let manifest: HostSourceManifest = serde_json::from_slice(&bytes)
            .map_err(|_| "Reference startup manifest schema")?;
        let mut members = manifest.sources.iter().filter(|row| row.path == config);
        let config = members.next().ok_or("Reference startup config is not pinned")?;
        if members.next().is_some() {
            return Err("Reference startup config pin is duplicated".into());
        }
        // Swift/Foundation can resolve these roots by name as soon as the
        // original initializer starts. A live retained FD alone is insufficient.
        for root in &self.roots[1..] {
            verify_reference_named_root(root)?;
        }
        self.verify_profile_directory()?;
        reference_private_startup_profile(&self.roots[0], profile, &config.sha256)?;
        self.verify_profile_directory()?;
        #[cfg(all(not(test), target_arch = "aarch64"))]
        self.verify_initial_configuration_read_source()?;
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn verify_initial_configuration_read_source(&self) -> Result<()> {
        #[link(name = "Foundation", kind = "framework")]
        unsafe extern "C" {
            #[link_name = "$s10Foundation4DataV10contentsOf7optionsAcA3URLVh_So20NSDataReadingOptionsVtKcfC"]
            static FOUNDATION_DATA_CONTENTS: u8;
        }
        let main = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| main.checked_add(offset).ok_or("Reference initial configuration code overflow");
        for (offset, length, expected) in [
            (0xdbb18, 268, "554016958b006d761900ff0563b1aff9225f2ff10f4dffdc0b076b40fbca452c"),
            (0xdbebc, 540, "984a5fa52d7caa71ed1eff94bfdc9323a07adaa63a4d772a6345485cec686b7a"),
            (0xdc26c, 740, "968d9a238c12d624ecd432c2b4a000385700d11ff46b7647b782f9a97d267d14"),
            (0x46a5c, 36, "c53c2d7cc5a37671e9f6fc7d0ca8a8d5bfcb139c116b4bee8885723fd63d6c06"),
            (0x46c58, 24, "2fb9912c262c21da85ae23ffe0472c7d656806698bba4eae8df88179ab50a294"),
            (0x46c9c, 48, "2bef346b757278a8a39ec633147c03f8ffb4a3bcdfa9f411e52bea3d7b2e803e"),
            (0x2f8f30, 12, "391cdfb33412505ecc95fe0edb19b2dbb243d0fd359c762934dcbedaa04de587"),
        ] {
            let start = at(offset)?;
            reference_mapping(start, length, Some(true))?;
            if reference_image_header(start, &self.endpoint)? != main
                || digest(unsafe { std::slice::from_raw_parts(start as *const u8, length) }) != expected
            {
                return Err("Reference initial configuration source differs".into());
            }
        }
        let provider = (&raw const FOUNDATION_DATA_CONTENTS) as usize;
        reference_code(provider, Path::new("/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"))?;
        let imports = super::imports::observe_unpatched_main_imports(&[(
            c"$s10Foundation4DataV10contentsOf7optionsAcA3URLVh_So20NSDataReadingOptionsVtKcfC",
            provider,
        )])?;
        let [read] = imports.as_slice() else {
            return Err("Reference initial configuration Foundation import absent".into());
        };
        if read.cell_address() != at(0x3c53a8)? {
            return Err("Reference initial configuration Foundation cell differs".into());
        }
        read.verify_unpatched()
    }
    #[cfg(all(rcf1_reference_sdk, target_arch = "aarch64", not(test)))]
    pub(crate) fn verify_started_control(&self, lock: i32, listener: i32) -> Result<()> {
        self.before_reference_publication()?;
        let profile = self.verify_profile_directory()?;
        verify_reference_control_leaves(&profile, self.grant.owner_uid)?;
        if lock <= 2 || lock == listener
            || unsafe { libc::fcntl(lock, libc::F_GETFD) } != libc::FD_CLOEXEC
            || unsafe { libc::fcntl(listener, libc::F_GETFD) } != libc::FD_CLOEXEC
        {
            return Err("Reference ControlServer lock or listener descriptor differs".into());
        }
        let (mut held, mut named) = (std::mem::MaybeUninit::<libc::stat>::uninit(), std::mem::MaybeUninit::<libc::stat>::uninit());
        if unsafe { libc::fstat(lock, held.as_mut_ptr()) } != 0 || unsafe {
            libc::fstatat(profile.as_raw_fd(), c"instance.lock".as_ptr(), named.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW)
        } != 0 {
            return Err("Reference ControlServer lock metadata unavailable".into());
        }
        let (held, named) = unsafe { (held.assume_init(), named.assume_init()) };
        if held.st_mode & libc::S_IFMT != libc::S_IFREG || held.st_uid != self.grant.owner_uid
            || (held.st_dev, held.st_ino) != (named.st_dev, named.st_ino)
        {
            return Err("Reference ControlServer lock inode differs".into());
        }
        if REFERENCE_CONTROL_LISTENED.with(std::cell::Cell::get) != Some(listener) {
            return Err("Reference ControlServer does not hold the descriptor accepted by listen".into());
        }
        verify_reference_socket_name(listener, &Path::new(&self.grant.runtime.profile).join("control.sock"))
            .map_err(|_| "Reference ControlServer listener address differs")?;
        self.verify_profile_directory()?;
        self.registry_current()?;
        REFERENCE_CONTROL_LOCK.with(|slot| {
            if slot.get().is_some() {
                return Err("Reference ControlServer lock observation replay".into());
            }
            slot.set(Some(lock));
            Ok(())
        })
    }
    fn verify_profile_directory(&self) -> Result<OwnedFd> {
        verify_reference_profile_directory(
            &self.roots[0],
            Path::new(&self.grant.runtime.profile),
            &self.profile_directory,
        )
    }
    pub fn qualified_sdk_addresses(&self) -> Result<[(&'static str,usize);5]> {
        self.before_reference_initialization()?;
        Ok(self.sdk_addresses)
    }
    /// Locator for Native's separate typed SourceCodeReader. The caller must
    /// still use the exact qualified ABI; this integer is not callback authority.
    pub fn qualified_table_address(&self) -> Result<usize> {
        self.before_reference_initialization()?;
        Ok(self.table)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_shortcuts_setter_caller(
        &self, return_pc: usize, arguments_for: Option<super::process_router::ReferenceTaskRoute>,
    ) -> Result<(super::process_router::ReferenceTaskRoute, std::path::PathBuf, impl Fn() -> Result<()> + '_)> {
        use super::process_router::{ReferenceTaskRoute, REFERENCE_SCRIPT_RUNNER};
        let assembly_offset = match arguments_for {
            None => Some(0x17cad4),
            Some(ReferenceTaskRoute::ProcessTable) => Some(0x400f0),
            _ => None,
        };
        let (offset, check) = self.reference_task_caller(return_pc, assembly_offset)?;
        let route = match (arguments_for, offset) {
            (None, Some(0x7d9d0)) | (Some(ReferenceTaskRoute::Discovery), Some(0x17fcc)) => ReferenceTaskRoute::Discovery,
            (None, Some(0x44cf4)) | (Some(ReferenceTaskRoute::Script), Some(0x400f0)) => ReferenceTaskRoute::Script,
            (None, Some(0x17cad4)) | (Some(ReferenceTaskRoute::ProcessTable), Some(0x400f0)) => ReferenceTaskRoute::ProcessTable,
            (None, Some(0x1c91a0)) | (Some(ReferenceTaskRoute::FileSearch), Some(0x400f0)) => ReferenceTaskRoute::FileSearch,
            _ => return Err("Reference Shortcuts Task setter caller differs".into()),
        };
        let path = match route {
            ReferenceTaskRoute::Discovery => std::path::PathBuf::from("/usr/bin/shortcuts"),
            ReferenceTaskRoute::Script => self.root.path().join(REFERENCE_SCRIPT_RUNNER),
            ReferenceTaskRoute::ProcessTable => std::path::PathBuf::from("/bin/ps"),
            ReferenceTaskRoute::FileSearch => std::path::PathBuf::from("/usr/bin/mdfind"),
        };
        Ok((route, path, check))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_script_write(
        &self, return_pc: usize, witness: usize,
    ) -> Result<impl Fn() -> Result<()> + '_> {
        let (offset, check) = self.reference_task_caller(return_pc, None)?;
        if offset != Some(0x44ef4) {
            return Err("Reference script write caller differs".into());
        }
        self.verify_reference_script_write_import(true)?;
        let cache = (unsafe { _NSGetMachExecuteHeader() } as usize)
            .checked_add(0x3ec8c0).ok_or("Reference script witness cache overflow")?;
        let protection = reference_mapping_protection(cache, size_of::<usize>())?;
        if protection & 1 == 0 || protection & 4 != 0 {
            return Err("Reference script witness cache protections differ".into());
        }
        // The pinned Main accessor obtains this through swift_getWitnessTable
        // and publishes it with STLR before passing it as x2.
        let cached = unsafe { &*(cache as *const std::sync::atomic::AtomicUsize) }
            .load(std::sync::atomic::Ordering::Acquire);
        if witness == 0 || cached != witness {
            return Err("Reference script Data witness differs".into());
        }
        check()?;
        Ok(check)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_monitor_environment(
        &self, return_pc: usize,
    ) -> Result<impl Fn() -> Result<()> + '_> {
        #[link(name = "Foundation", kind = "framework")]
        unsafe extern "C" {
            #[link_name = "$sSD10FoundationE19_bridgeToObjectiveCSo12NSDictionaryCyF"]
            static BRIDGE_DICTIONARY: u8;
        }
        let bridge = c"$sSD10FoundationE19_bridgeToObjectiveCSo12NSDictionaryCyF";
        let send = c"objc_msgSend";
        let (offset, current) = self.reference_task_caller(return_pc, Some(0x17cbf4))?;
        if offset != Some(0x17cbf4) {
            return Err("Reference monitor environment caller differs".into());
        }
        let base = return_pc - 0x17cbf4;
        let original_bridge = (&raw const BRIDGE_DICTIONARY) as usize;
        let original_send = objc2::ffi::objc_msgSend as *const () as usize;
        reference_code(original_bridge, Path::new("/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"))?;
        reference_code(original_send, Path::new("/usr/lib/libobjc.A.dylib"))?;
        let imports = super::imports::observe_unpatched_main_imports(&[
            (bridge, original_bridge), (send, original_send),
        ])?;
        for row in &imports {
            let offset = if row.symbol == bridge { 0x3c56f8 } else { 0x3c6940 };
            if base.checked_add(offset) != Some(row.cell_address()) {
                return Err("Reference monitor environment import cell differs".into());
            }
        }
        let cell = base.checked_add(0x3e8608).ok_or("Reference monitor environment selector overflow")?;
        let selector = objc2::runtime::Sel::register(c"setEnvironment:");
        let verify = move || {
            for (offset, length, sha) in [
                (0x17cb90, 104, "b8f95044add6503e1dfc7ea26580f0581f24eb3074f8606e59c5fb0b58dc377b"),
                (0x180918, 12, "eedd8635e63de595a38846c97c616cf19a0c901688bbb84bfacc28901c5d7b71"),
                (0x2fa3dc, 12, "1c66b9221f3af2efd8750b26ff4f059132fe08512e1d7d7d89c94927d84e8e1c"),
                (0x2fbb04, 12, "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f"),
            ] {
                let address = base.checked_add(offset).ok_or("Reference monitor environment code overflow")?;
                reference_mapping(address, length, Some(true))?;
                if digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != sha {
                    return Err("Reference monitor environment code changed".into());
                }
            }
            let protection = reference_mapping_protection(cell, size_of::<usize>())?;
            if protection & 1 == 0 || protection & 4 != 0
                || unsafe { (cell as *const usize).read_volatile() }
                    != unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) }
            {
                return Err("Reference monitor environment selector differs".into());
            }
            for row in &imports {
                row.verify_unpatched()?;
            }
            current()
        };
        verify()?;
        Ok(verify)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_script_write_import(&self, installed: bool) -> Result<()> {
        let spec = crate::workspace::objc::reference_write_spec();
        let rows = super::imports::observe_main_imports(&[spec])?;
        let [row] = rows.as_slice() else {
            return Err("Reference script write requires one exact Main import".into());
        };
        let foundation = Path::new("/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation");
        reference_code(spec.original, foundation)?;
        reference_code(spec.replacement, &self.sdk)?;
        let expected = if installed { spec.replacement } else { spec.original };
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        if base.checked_add(0x3c57a0) != Some(row.cell_address()) || row.selected_address() != expected {
            return Err("Reference script write import cell or selection differs".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn reference_task_caller(&self, return_pc: usize, assembly_offset: Option<usize>)
        -> Result<(Option<usize>, impl Fn() -> Result<()> + '_)> {
        let assembly = super::system_providers::monitor_timeout::assembly_probe_active();
        let snapshot = if assembly {
            self.before_reference_initialization()?;
            None
        } else {
            if !super::SOURCE_ROUTES_QUALIFIED {
                return Err("Reference source routes are not qualified".into());
            }
            self.verify_reference_worker_environment()?;
            let registry = super::REGISTRY.get().ok_or("Reference Task Registry is not published")?
                .try_lock().map_err(|_| "Reference Task Registry busy")?;
            if !Arc::ptr_eq(&registry.life, &self.admitted.life) {
                return Err("Reference Task Registry guardian differs".into());
            }
            Some(registry.ready_snapshot())
        };
        let header = reference_image_header(return_pc, &self.endpoint)?;
        if header != unsafe { _NSGetMachExecuteHeader() } as usize {
            return Err("Reference Task caller differs".into());
        }
        let offset = return_pc.checked_sub(header);
        let check = move || {
            if let Some(snapshot) = &snapshot {
                if !snapshot.current() {
                    return Err("Reference Task Registry retired during caller validation".into());
                }
            } else if assembly_offset.is_none() || offset != assembly_offset
                || super::REGISTRY.get().is_some()
                || !super::system_providers::monitor_timeout::assembly_probe_active()
            {
                return Err("Reference Task caller outside its monitor assembly turn".into());
            }
            self.registry_current()
        };
        check()?;
        Ok((offset, check))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_startup_control_caller(&self, caller: usize, offsets: &[usize]) -> Result<()> {
        self.before_reference_initialization()?;
        let header = reference_image_header(caller, &self.endpoint)?;
        if header != unsafe { _NSGetMachExecuteHeader() } as usize
            || !caller.checked_sub(header).is_some_and(|offset| offsets.contains(&offset))
        {
            return Err("Reference startup control caller differs".into());
        }
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_safari_descriptor_entries(&self) -> Result<[usize; 4]> {
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference Safari descriptor offset overflow");
        for (offset, length, expected) in [
            (0x935d0, 0xbc, "4c22ea6ddb486de966649ef18fd91eff699b8afe33336f6b0b6d1225da5b3780"),
            (0x943a8, 0xf0, "0054e1aeacb42cc177f545d7452305ddbc0a0442196972536955d36e7bc94f82"),
            (0x944d4, 0x1ac, "a08f2c44fe0f62c08a7b494410cf187930fbb4d050551eb12f345f06899bd510"),
            (0x94680, 0x8c, "f4f89b79a393a75a4167c77ea2bed00e537619c782cef614460a72d834e458a3"),
            (0x97980, 0x8c, "00f27ce60abc88aa66c126c0cb3490563f48c3cf0775e9cae8fd8d499d7954a5"),
        ] {
            let address = at(offset)?;
            reference_mapping(address, length, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference original Safari descriptor code differs".into());
            }
        }
        Ok([at(0x943a8)?, at(0x944d4)?, at(0x94680)?, at(0x935d0)?])
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_safari_probe_send(&self, caller: usize) -> Result<()> {
        self.before_reference_initialization()?;
        self.verify_reference_safari_effect(caller, crate::workspace::callbacks::SafariEffect::Send)?;
        let base = caller.checked_sub(0x93610).ok_or("Reference Safari send base differs")?;
        self.verify_reference_safari_dispatch(base, 0x3e8950, objc2::sel!(sendEventWithOptions:timeout:error:))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn verify_reference_safari_dispatch(&self, base: usize, offset: usize, selector: objc2::runtime::Sel) -> Result<()> {
        let stub = base + 0x2fbb04;
        reference_mapping(stub, 12, Some(true))?;
        if digest(unsafe { std::slice::from_raw_parts(stub as *const u8, 12) })
            != "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f"
        {
            return Err("Reference Safari objc_msgSend stub differs".into());
        }
        let send = objc2::ffi::objc_msgSend as *const () as usize;
        let imports = super::imports::observe_unpatched_main_imports(&[(c"objc_msgSend", send)])?;
        let [import] = imports.as_slice() else { return Err("Reference Safari objc_msgSend import absent".into()) };
        if import.cell_address() != base + 0x3c6940 {
            return Err("Reference Safari objc_msgSend import cell differs".into());
        }
        import.verify_unpatched()?;
        let cell = base + offset;
        if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
            || unsafe { (cell as *const usize).read_volatile() }
                != unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) }
        {
            return Err("Reference Safari selector cell differs".into());
        }
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn reference_safari_discovery_probe(&self, class: usize) -> Result<(usize, usize)> {
        unsafe extern "C" { fn objc_opt_self(); }
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let entry = base + 0x93160;
        for (address, size, expected) in [
            (entry, 0x330, "0afa4b8ce779e0aae1482cc6d6f0a86084cbdb06fe68ec388aa4a51cb4870569"),
            (base + 0x2fbb1c, 12, "f484946889f54d188a8a62d95a63cf297d2dda2a33141c46e457965849cecf23"),
        ] {
            reference_mapping(address, size, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, size) }) != expected {
                return Err("Reference Safari discovery probe code differs".into());
            }
        }
        let imports = super::imports::observe_unpatched_main_imports(&[
            (c"objc_opt_self", objc_opt_self as *const () as usize),
        ])?;
        let [import] = imports.as_slice() else { return Err("Reference Safari class import absent".into()) };
        let cell = base + 0x3e9a18;
        if import.cell_address() != base + 0x3c6950
            || reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
            || unsafe { (cell as *const usize).read_volatile() } != class
        {
            return Err("Reference Safari discovery class binding differs".into());
        }
        import.verify_unpatched()?;
        self.verify_reference_safari_dispatch(base, 0x3e8850, objc2::sel!(runningApplicationsWithBundleIdentifier:))?;
        Ok((entry, base + 0x9321c))
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_safari_discovery(&self, caller: usize) -> Result<()> {
        self.verify_reference_safari_callsite(
            caller, 0x9321c, 0x931d4, 92,
            "3154d72a119915e9a042ccdb8cabbde3cd41335acd1eca6497f602fa8cf8080e",
        )
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_reference_safari_discovery(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        base.checked_add(0x9321c) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_safari_effect(
        &self, caller: usize, effect: crate::workspace::callbacks::SafariEffect,
    ) -> Result<()> {
        use crate::workspace::callbacks::SafariEffect;
        let (return_offset, start, length, sha) = match effect {
            SafariEffect::Send => (
                0x93610, 0x935d0, 188,
                "4c22ea6ddb486de966649ef18fd91eff699b8afe33336f6b0b6d1225da5b3780",
            ),
            SafariEffect::Consent => (
                0x93b8c, 0x93b2c, 116,
                "d73e95465dd8f260aeac51869b8e8a6e8e9f0b621a4e5057205dbb2857dde283",
            ),
            SafariEffect::Activate => (
                0x940cc, 0x93f78, 352,
                "046edc3379c2de83328c338568869cd061f8062aeb1a89a7c7bf6b6f459c6d09",
            ),
        };
        self.verify_reference_safari_callsite(caller, return_offset, start, length, sha)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_reference_safari_activation(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        base.checked_add(0x940cc) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_reference_ax_activation(caller: usize) -> bool {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        base.checked_add(0x1c6c24) == Some(caller)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_process_read(&self, caller: usize, symbol: &std::ffi::CStr) -> Result<usize> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let (ret, cell, spans) = match symbol.to_bytes() {
            b"proc_pidinfo" => (0x1c63d0, 0x3c4770, [
                (0x1c6388, 88, "d9bba694b06e450132f3074cbc3449075551dfd614951461ee34151eb77e6078"),
                (0x2fbc30, 12, "664118e042a5b34904d68806b8ceb70b7ea5535f3312afd6e50e94fa44183808"),
            ]),
            b"proc_pidpath" => (0x17d720, 0x3c4778, [
                (0x17d6f4, 56, "4f03e4573660671763f397c1e9f53e90f6e9ce4ce90cc8a6597934a1cc2b4eb3"),
                (0x2fbc3c, 12, "34b2e591346803e9696fe32c98973641f258ba2094d85010c42d46fb1e65c5ad"),
            ]),
            b"proc_listallpids" => (0x17b76c, 0x3c4768, [
                (0x17b75c, 28, "e722dd9f3f2802c20bcda40f08b40c245443be8c5f38512e3426e3df6003bd4f"),
                (0x2fbc24, 12, "6ac4616d1d19e7061c66d530c5b6c3902fc2fd282a571916644670c59759c717"),
            ]),
            _ => return Err("Reference process read symbol differs".into()),
        };
        let at = |offset| base.checked_add(offset).ok_or("Reference process read offset overflow");
        if at(ret)? != caller {
            return Err("Reference process read caller differs".into());
        }
        for (offset, length, hash) in spans {
            let start = at(offset)?;
            reference_mapping(start, length, Some(true))?;
            if reference_image_header(start, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(start as *const u8, length) }) != hash
            {
                return Err("Reference process read code differs".into());
            }
        }
        self.registry_current()?;
        Ok(at(cell)?)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_ax_list_call(&self, caller: usize, copy: bool) -> Result<usize> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference AX list offset overflow");
        let (ret, start, length, hash, stub, stub_hash, cell) = if copy {
            (0x1b6844, 0x1b6820, 36, "3152a8d355e6b2563ff9dba2f6303c59272778b4719b31c9a5fef73dc00b9297",
                0x2fb000, "631d7754767e87ef57077170c0fa486397923aca9be5302f60c18a6f7e3dddeb", 0x3c4ad8)
        } else {
            (0x1b67b8, 0x1b679c, 32, "6b575130fed104283b210659fd49e2dde44ac9197a4f3856f7e455d3a07ec03e",
                0x2fb024, "2070f04bcc910c3c7f66b69efd1445705f240fc5eaaff76e14bd4868bdedaccf", 0x3c4af0)
        };
        if at(ret)? != caller {
            return Err("Reference AX list caller differs".into());
        }
        for (offset, length, expected) in [(start, length, hash), (stub, 12, stub_hash)] {
            let address = at(offset)?;
            reference_mapping(address, length, Some(true))?;
            if reference_image_header(address, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(address as *const u8, length) }) != expected
            {
                return Err("Reference AX list source differs".into());
            }
        }
        self.registry_current()?;
        Ok(at(cell)?)
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_ax_activation(&self, caller: usize, selector: objc2::runtime::Sel) -> Result<()> {
        if !Self::is_reference_ax_activation(caller) || selector != objc2::sel!(activateWithOptions:) {
            return Err("Reference AX activation caller or selector differs".into());
        }
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let at = |offset| base.checked_add(offset).ok_or("Reference AX activation offset overflow");
        for (offset, length, expected) in [
            (0x1c6bd8, 104, "cc266f83bdb4459c34ca511d5cae3e701bf31775386139b98727b1970dbe054d"),
            (0x2fbb04, 12, "10d44d02938bf0656f63270be6591d65149df5611cbc7b3a1c03a274b944d21f"),
        ] {
            let start = at(offset)?;
            reference_mapping(start, length, Some(true))?;
            if reference_image_header(start, &self.endpoint)? != base
                || digest(unsafe { std::slice::from_raw_parts(start as *const u8, length) }) != expected
            {
                return Err("Reference AX activation code differs".into());
            }
        }
        let cell = at(0x3e87b8)?;
        let selected = unsafe { std::mem::transmute::<objc2::runtime::Sel, usize>(selector) };
        if reference_mapping_protection(cell, size_of::<usize>())? & 5 != 1
            || unsafe { (cell as *const usize).read_volatile() } != selected
        {
            return Err("Reference AX activation selector cell differs".into());
        }
        self.registry_current()
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn verify_reference_safari_callsite(
        &self, caller: usize, return_offset: usize, start_offset: usize, length: usize, sha: &str,
    ) -> Result<()> {
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        let start = base.checked_add(start_offset).ok_or("Reference Safari address overflow")?;
        if base.checked_add(return_offset) != Some(caller)
            || reference_image_header(start, &self.endpoint)? != base
        {
            return Err("Reference Safari caller differs".into());
        }
        self.registry_current()?;
        self.admitted.life.before_effect()?;
        reference_mapping(start, length, Some(true))?;
        if digest(unsafe { std::slice::from_raw_parts(start as *const u8, length) }) != sha
        {
            return Err("Reference Safari callsite bytes differ".into());
        }
        self.registry_current()
    }

    fn verify_reference_worker_environment(&self) -> Result<()> {
        #[cfg(rcf1_reference_sdk)]
        self.registry_current()?;
        self.admitted.life.before_effect()?;
        if !self.admitted.life.is_alive() { return Err("Reference worker guardian retired".into()); }
        if self.root.owner_ids()?.0!=self.grant.owner_uid { return Err("Reference worker source root owner differs".into()); }
        stdio(&self.admitted.stdio)?;
        image(&self.endpoint)?;
        checked_hash(&self.root,&self.endpoint,&self.grant.endpoint_sha256)?;
        checked_hash(&self.root,&self.sdk,&self.grant.sdk_artifact_sha256)?;
        reference_sources(&self.root,&self.grant)?;
        reference_environment(&reference_environment_snapshot()?,&reference_environment_values(&self.grant,&self.sdk)?)?;
        for root in &self.roots {
            if root.owner_ids()?.0!=self.grant.owner_uid { return Err("Reference worker runtime root owner differs".into()); }
            let descriptor=root.checked(root.path())?;
            let mut stat:libc::stat=unsafe{std::mem::zeroed()};
            if unsafe{libc::fstat(descriptor.as_raw_fd(),&mut stat)}!=0 || stat.st_mode&0o077!=0 {
                return Err("Reference worker runtime root is not private".into());
            }
        }
        self.verify_profile_directory()?;
        checked_hash(&self.root,Path::new(&self.grant.runtime.fixture.path),&self.grant.runtime.fixture.sha256)?;
        Ok(())
    }
    /// Only the SDK's fixed reply/process-provider worker callbacks may use
    /// this route. It exposes no MAIN table/delegate/CF-perform authority and
    /// does not relax the owner check on any main-only getter.
    pub fn before_reference_worker_callback(&self)->Result<()> {
        if !super::SOURCE_ROUTES_QUALIFIED { return Err("Reference source routes are not qualified".into()); }
        self.verify_reference_worker_environment()?;
        let readiness={
            let registry=super::REGISTRY.get().ok_or("Reference worker Registry is not published")?
                .try_lock().map_err(|_|"Reference worker Registry busy")?;
            if !Arc::ptr_eq(&registry.life,&self.admitted.life) { return Err("Reference worker Registry guardian differs".into()); }
            registry.ready_snapshot()
        };
        let registry_retired=readiness.retired.clone();
        if readiness.ready()!=0xF100002600003FFF {return Err("Reference worker requires actual fourteen-scope readiness".into());}
        reference_mapping_infrastructure()?;
        reference_code_pins(&self.grant.sdk_functions,&REFERENCE_SDK_SYMBOLS)?;
        let sdk_header=reference_image_header(self.sdk_addresses[0].1,&self.sdk)?;
        reference_image_policy(sdk_header,false,0,0)?;
        for i in [2,3] {
            let pin=&self.grant.sdk_functions[i];let (label,address)=self.sdk_addresses[i];
            if label!=REFERENCE_SDK_SYMBOLS[i] || sdk_header.checked_add(pin.image_offset)!=Some(address) {
                return Err("Reference worker fixed callback source differs".into());
            }
            reference_code(address,&self.sdk)?;
        }
        // No caller-supplied/dynamic source callback after these final code
        // comparisons. The concrete admitted Guardian is the same live owner.
        if !self.admitted.life.is_alive() { return Err("Reference worker guardian retired after verification".into()); }
        reference_retirement_tail(&registry_retired)?;
        #[cfg(rcf1_reference_sdk)]
        self.registry_current()?;
        Ok(())
    }

    /// The website assembly provider must first match its own retained queue
    /// and active fixture. This observes environment only and publishes no Registry.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_reference_web_assembly_worker(&self) -> Result<()> {
        if unsafe { libc::pthread_main_np() } != 0 || super::REGISTRY.get().is_some() {
            return Err("Reference website assembly worker outside pre-entry".into());
        }
        self.verify_reference_worker_environment()?;
        self.registry_current()
    }
    pub fn before_reference_initialization(&self) -> Result<()> {
        #[cfg(rcf1_reference_sdk)]
        self.registry_current()?;
        // The constructor-free Reference producer must be able to assemble
        // real imports/factories before complete campaign readiness exists.
        // Publication and the original initializer check that boundary again.
        #[cfg(not(rcf1_reference_sdk))]
        if !super::SOURCE_ROUTES_QUALIFIED { return Err("Reference source routes are not qualified".into()); }
        if self.owner != std::thread::current().id() || unsafe{libc::pthread_main_np()}!=1 {
            return Err("Reference foreign constructor owner".into());
        }
        self.admitted.life.before_effect()?;
        if !self.admitted.life.is_alive() { return Err("Reference guardian retired".into()); }
        stdio(&self.admitted.stdio)?;
        image(&self.endpoint)?;
        checked_hash(&self.root,&self.endpoint,&self.grant.endpoint_sha256)?;
        checked_hash(&self.root,&self.sdk,&self.grant.sdk_artifact_sha256)?;
        reference_sources(&self.root,&self.grant)?;
        reference_environment(&reference_environment_snapshot()?, &reference_environment_values(&self.grant,&self.sdk)?)?;
        for root in &self.roots {
            if root.owner_ids()?.0 != self.grant.owner_uid { return Err("Reference runtime owner differs".into()); }
            let descriptor=root.checked(root.path())?;
            let mut stat:libc::stat=unsafe{std::mem::zeroed()};
            if unsafe{libc::fstat(descriptor.as_raw_fd(),&mut stat)}!=0 || stat.st_mode&0o077!=0 {
                return Err("Reference runtime root is not private".into());
            }
        }
        self.verify_profile_directory()?;
        checked_hash(&self.root,Path::new(&self.grant.runtime.fixture.path),&self.grant.runtime.fixture.sha256)?;
        reference_loaded_table(&self.endpoint,&self.sdk,&self.grant,self.table,self.sdk_addresses)?;
        // No dynamic callback after the final table/code comparison.
        if !self.admitted.life.is_alive() { return Err("Reference guardian retired after verification".into()); }
        #[cfg(rcf1_reference_sdk)]
        self.registry_current()?;
        Ok(())
    }
    #[cfg(all(not(test),rcf1_reference_sdk))]
    pub(crate) fn initialize_bootstrap(
        self: &Arc<Self>,
        mut context: super::bootstrap::Bootstrap,
        prepare_workspace: impl FnOnce() -> Result<Arc<crate::workspace::Workspace>>,
    ) -> Result<(Arc<crate::workspace::Workspace>, super::ModuleAttestation)> {
        self.before_reference_initialization()?;
        if !Arc::ptr_eq(&context.life,&self.admitted.life)
            || [&context.home,&context.cache,&context.temp,&context.applications].iter().zip(&self.roots)
                .any(|(a,b)| a.path()!=b.path()) {
            return Err("Reference bootstrap roots or guardian differ".into());
        }
        // Use the actual retained admitted roots, not caller-reopened aliases.
        context.home=self.roots[0].clone();context.cache=self.roots[1].clone();
        context.temp=self.roots[2].clone();context.applications=self.roots[3].clone();
        unsafe {
            super::bootstrap::initialize_owned_reference(
                context,
                self.life(),
                self.descriptors(),
                || self.before_reference_initialization(),
                prepare_workspace,
                |guard| self.install_profile_lock_import(guard),
            )
        }
    }

    #[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
    fn install_profile_lock_import(self: &Arc<Self>, guard: &dyn AliveLease) -> Result<Vec<super::ImportReplacement>> {
        #[link(name = "swiftDarwin")]
        unsafe extern "C" {
            #[link_name = "$s6Darwin4openys5Int32VSPys4Int8VG_ADs6UInt16VtF"]
            static SWIFT_OPEN_MODE: u8;
            #[link_name = "$s6Darwin4openys5Int32VSPys4Int8VG_ADtF"]
            static SWIFT_OPEN_READ: u8;
        }
        self.before_reference_initialization()?;
        let base = unsafe { _NSGetMachExecuteHeader() } as usize;
        for (offset, size, expected) in [
            (0xdc7e0, 24, "e8cd3b3ce95b4d07260996775a30ebfe59b35b22037c1b44413ce2ab0f181583"),
            (0xde000, 24, "3401af69bd99efbff27ac1f4559a5d635ef4c2a917acfb1f6104e9e1cd5e29a0"),
            (0x2f9584, 24, "a347f9595a0fb5d66ee4864987f7ad5cdf31ffed63d608e4e33cb69a484bcc44"),
            (0x2f320, 24, "469bdd87f1fca81ee0af5f5f35fc22aa19cac788e67eabea260fedd2d037b5a1"),
            (0x7f988, 24, "aee2487b1b158a8d6dc6b4e21fcf35446ee77dc3d120f941a2eedc8065a009a6"),
            (0x80544, 24, "f2bdb74e6ba12872a923f90c3d5878935c8ddde224bf137f7d311dd05dd1c61d"),
            (0xa00d0, 24, "dbb2487f98d6b5d28d4c26bfff5b18463a09cf192839a9d6afd8fde49370ec1d"),
            (0xaf8f8, 24, "be53c34755373f9ae1ecbc0c2c9df0ff4fef75bcb23b7550a357024fd563f875"),
            (0xd25c0, 24, "7bffe590116db7be4d9e4447157248ab9ec3301e3ba5a32fee14d5a34d3fa1f9"),
            (0xd2c0c, 24, "9e781f7534ed94d252517caabf0df6a6448047926b6f6f6995787bcbde0e25eb"),
            (0x207570, 24, "1c6a3689bc0f6c6671c17029524a60c432c12e8f9ee52593631843cc607b65fb"),
            (0x20934c, 24, "ec9615625df1b330e71956717afa18cf02856cef14aba2d71fa4e6fe226877c0"),
            (0x7ff34, 24, "b68aca5993639b3a909fdf0e3b6d7feed6ccf7e2989931231ebe5a0e5473a990"),
            (0xb2138, 24, "d4bc38db7220128b5eb561710acf1efcc65b3f72617f53176ba8dff21414a6c3"),
            (0xdea24, 24, "3da6de1412d3781c617b54a56d72cf40e417be31d746441d51efbeeb33d647a2"),
            (0x277ccc, 24, "0933530469a74f3669ff086eadd5de55e1d6b1c4a3ce5cf5d517aa755d3a6e33"),
            (0x277f18, 24, "c97196598c8b39d2af36b6c42cbe5fbe1cdad46d422e0eeb3e3c5d2d32f1109c"),
            (0x279504, 24, "e5053f6b2a91f93d150b013da8552a426ab624a75457aa4f5eb598b2973f9fb0"),
            (0x28210c, 24, "a5109846fdb4487a291a0b41abe0855a1f52bdffcb07028b0fa8ceb926913d0f"),
            (0x289cc4, 24, "29ff20ee39829a92d66cfd80bc88744e33fc9470cc1a6df7a4862d8dcf4e18a7"),
            (0x289d70, 24, "2725bc6033b60078a937355b768c678f9957393acc5b467db561921c1b4ac244"),
            (0x2fba20, 12, "5cf197002599debdb2f3d987458f841f27b514afb9d3fb9d8ee62577411b7a6e"),
            (0xdf85c, 24, "fb1359f3932db2dfbc343429c9194d5dd2ed047704e85dea01902f6f51d3dc49"),
            (0x277b38, 24, "0eedead53206561afaa4a3e976606637df40c668ca929fdd20fedb8a058ec5a2"),
            (0x277cf0, 24, "04a7a9fd48e89594e93f2028936cc7d23bb24bd62f492cef853a6a342915e958"),
            (0x289c64, 24, "d55dd8e6efd10332f40c35e845459a964a428d6b76e5eed7035d024f5541fa0a"),
            (0x289d94, 24, "7ef144750a5ed00df5ee8f48349de902a016e556080da6d1c0e978bacd552f33"),
            (0x2fc380, 12, "f02afcaaf8d63eced4228fd41767501a2a6b059554d5969d2d014f6f03284a43"),
            (0xde534, 24, "dab8eec307cd0f41496c6a08d9eb923ae54485c9982fdbe03a9b2b7c91e526c9"),
            (0x2733dc, 24, "52dc8655278b5d0c9b835203350fb9fbdb032834a06b32303d52b9db17ca7d14"),
            (0x2737f4, 24, "d0a1bf4a26fc02da6c68c39d27d7ae6276819d0590e7db11a41ed5325fb06738"),
            (0x27b08c, 24, "8ef7b14cd6b1e8b3b19958edabd00f9e5c2b809b9c630da896c1ecb358f47c2f"),
            (0x27b988, 24, "7b19657e11061799d4fc3dcad9bbf8eb2d5bb84230ebfeec4b10cca8a84fbece"),
            (0x2fb6c0, 12, "1457217524c3935a0c787b143742289b3af67fa6a8dee6bf714e70c43d202610"),
            (0xde448, 24, "b498d82a78916a01f36fafe6ab613a20694cb0e5dd31d1934a2b65587f050dcb"),
            (0x272b20, 24, "2fe29340740d54df4f72b49543a969bbbf8b7ad0c0824037974e621044f7e4fb"),
            (0x2731ac, 24, "36be1c377f5173bc840869d1746c13d23361f4f56f45d435269f0f68399509cf"),
            (0x275bdc, 24, "ea15b3dc774b951feea62458678ef4377570866392e78de320ffebc786396739"),
            (0x27b70c, 24, "1f58001f7b6a1e03211e1483e8faa97c2492088961d81344037637e70caf2155"),
            (0x27ba54, 24, "7a4c27f832c0a1ff2bf4fe785598cbcdd12e1bde88520e5703c655eced8dfb1e"),
            (0x2fb744, 12, "e53b0ecf2e74fc392756f800e4c563760bc0de3746a69d568396f8a1fae98e72"),
            (0xde714, 24, "421c2a44926361bf2b4493855af5427a5714caddd492f53fa4ef31df55362d37"),
            (0x2fb834, 12, "346de823713fc1a6f54854546d8d14d8c2a70b5bec3f32d7bfd2de71c7c23f5d"),
            (0xde64c, 16, "54f842aabc99b462c34298d4e95efc1bd1ab123b628decd408b853b95ef70563"),
            (0x27380c, 16, "7aee8152054f5e1f38afa404dfd8e6a39e3aaa536b685c526589ab51875584d7"),
            (0x27b0a4, 16, "eaa3038364ab731de9f1b61a3f2d4655e0937cfa57ca67312079869d793aaa6c"),
            (0x2fba08, 12, "3de5503d10619bbc54c17383949219a112be93ae7ca2e41cb8b067919407ead8"),
            (0xe09a0, 16, "e1c8b2112a30c6da25bb289bf977477ecf74ffd272edbb23cb222b1125be73c4"),
            (0xde118, 24, "fe7accd1e0817c80790b825c8d3174b816a31ccb7196288e95113f78b36bbfe3"),
            (0xde35c, 24, "16c41540722b0021f6efd60a3dbfd7d4b6554c6847ec6bd69561b7eb01d5e570"),
            (0x27b428, 20, "c8a3d0e0a7648fd4688f11272447133de1771de25c4ec7f51114ee986abd1826"),
            (0x2fbedc, 12, "ff5be9c527e5ad253d354c5309f0291dbd54be2a43d67413d00cb9397d8a8901"),
        ] {
            let address = base.checked_add(offset).ok_or("Reference profile lock code overflow")?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference profile lock caller is outside Main".into());
            }
            reference_mapping(address, size, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, size) }) != expected {
                return Err("Reference profile lock caller bytes changed".into());
            }
        }
        let lock = super::ImportSpec {
            symbol: super::bootstrap::REFERENCE_PROFILE_LOCK_IMPORT,
            original: (&raw const SWIFT_OPEN_MODE) as usize,
            replacement: reference_profile_lock_open as *const () as usize,
        };
        let read = super::ImportSpec {
            symbol: super::bootstrap::REFERENCE_PROFILE_READ_IMPORT,
            original: (&raw const SWIFT_OPEN_READ) as usize,
            replacement: reference_profile_read_open as *const () as usize,
        };
        let metadata = reference_lstat_spec();
        let unlink = reference_unlink_spec();
        let bind = reference_bind_spec();
        let connect = reference_connect_spec();
        let chmod = reference_chmod_spec();
        let listen = reference_listen_spec();
        let shutdown = reference_shutdown_spec();
        let accept = reference_accept_spec();
        let close = reference_close_spec();
        let socket = reference_socket_spec();
        for (offset, len, hash) in [
            (0xdebd0, 44, "ad5e929d8343a66ac24a04ad7880103c9d26c37af10a394189cf97a93ea37110"),
            (0xdee00, 20, "d082183bf22bb730e52cc9d32a7c26faf9c9e8de2ae06ef9e8766bf7d9055cd1"),
            (0xdf6f8, 12, "3be61b7b57400bce65c0d6fd2e4346e2e77fe756431b040ed7ec18f84e0fbfaf"),
            (0xdfcc0, 60, "d0c85411eeb84b428e3e7c24cef88b3e1b207d2c13c552c150052818b3040e6e"),
            (0xe01d4, 16, "f808b3d25b18b44764d3de2735c6e20b848662274a132d29cf14756b1f2c9d45"),
            (0x2fbe94, 12, "417456edeeac713c2961205847a753345d65acea17a50c42e52887895cdff6f9"),
        ] {
            let address = base.checked_add(offset).ok_or("Reference control shutdown code overflow")?;
            if reference_image_header(address, &self.endpoint)? != base {
                return Err("Reference control shutdown caller image differs".into());
            }
            reference_mapping(address, len, Some(true))?;
            if digest(unsafe { std::slice::from_raw_parts(address as *const u8, len) }) != hash {
                return Err("Reference control shutdown caller code differs".into());
            }
        }
        let swift_darwin = Path::new("/usr/lib/swift/libswiftDarwin.dylib");
        for (spec, offset, provider) in [
            (lock, 0x3c7528, swift_darwin),
            (read, 0x3c7530, swift_darwin),
            (metadata, 0x3c4650, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (unlink, 0x3c49a0, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (bind, 0x3c4410, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (connect, 0x3c4468, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (chmod, 0x3c4508, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (listen, 0x3c4640, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (shutdown, 0x3c4908, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (accept, 0x3c43d0, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (close, 0x3c4450, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
            (socket, 0x3c4938, Path::new("/usr/lib/system/libsystem_kernel.dylib")),
        ] {
            reference_code(spec.original, provider)?;
            reference_code(spec.replacement, &self.sdk)?;
            let rows = super::observe_main_imports(&[spec])?;
            let [row] = rows.as_slice() else { return Err("Reference profile open import absent".into()); };
            if base.checked_add(offset) != Some(row.cell_address()) || row.selected_address() != spec.original {
                return Err("Reference profile open import cell differs".into());
            }
        }
        REFERENCE_LOCK_OWNER.set(self.clone()).map_err(|_| "Reference profile lock issuer replay")?;
        super::import_patch::install(&[lock, read, metadata, unlink, bind, connect, chmod, listen, shutdown, socket, accept, close], guard)
    }

    #[cfg(all(not(test), rcf1_reference_sdk, not(target_arch = "aarch64")))]
    fn install_profile_lock_import(self: &Arc<Self>, _guard: &dyn AliveLease) -> Result<Vec<super::ImportReplacement>> {
        Err("Reference profile lock route requires pinned arm64 Main".into())
    }
    pub fn before_reference_publication(&self) -> Result<()> {
        self.before_reference_initialization()?;
        if super::ruycast_f1_preentry_ready_v1()!=0xF100002600003FFF {
            return Err("Reference fourteen scopes not actually admitted".into());
        }
        Ok(())
    }
    #[cfg(rcf1_reference_sdk)]
    pub(crate) fn verify_terminal_stdio(&self) -> Result<()> {
        if self.owner != std::thread::current().id() || unsafe { libc::pthread_main_np() } != 1 {
            return Err("Reference terminal stdio on foreign owner".into());
        }
        self.admitted.life.before_effect()?;
        stdio(&self.admitted.stdio)
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceLockRoute {
    Main,
    ControlClient,
    SafariSearch,
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
unsafe fn verify_configuration_observer_queue(queue: usize) -> Result<()> {
    unsafe extern "C" {
        fn dispatch_queue_get_label(queue: *const std::ffi::c_void) -> *const libc::c_char;
    }
    let queue_class = super::metadata::loaded_class(c"OS_dispatch_queue_serial")?;
    if !queue.is_multiple_of(8) || reference_mapping_protection(queue, 8)? & 7 != 3
        || unsafe { objc2::ffi::object_getClass(queue as *const _) } as usize != queue_class.address()
    {
        return Err("Reference configuration observer worker queue class differs".into());
    }
    reference_code(dispatch_queue_get_label as *const () as usize, Path::new("/usr/lib/system/libdispatch.dylib"))?;
    let label = unsafe { dispatch_queue_get_label(queue as *const _) };
    if label.is_null() || unsafe { CStr::from_ptr(label) } != c"ruycast.config-observer" {
        return Err("Reference configuration observer worker queue label differs".into());
    }
    Ok(())
}

#[cfg(all(not(test), rcf1_reference_sdk, target_arch = "aarch64"))]
fn verify_reference_closed_descriptors(descriptors: &[(&str, Option<i32>)]) -> Result<()> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut pending = None;
        for &(name, fd) in descriptors {
            let fd = fd.ok_or_else(|| format!("Reference {name} descriptor was not captured"))?;
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1 {
                pending = Some(name);
            } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF) {
                return Err(format!("Reference {name} descriptor check failed"));
            }
        }
        let Some(pending) = pending else { return Ok(()); };
        if Instant::now() >= deadline {
            return Err(format!("Reference {pending} descriptor close unconfirmed"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn reference_retirement_tail(retired:&std::sync::atomic::AtomicBool)->Result<()> {
    if retired.load(std::sync::atomic::Ordering::Acquire)
        || super::READY_RETIRED.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Reference worker Registry retired during verification".into());
    }
    Ok(())
}
fn reference_sources(root:&OwnedRoot, grant:&ReferenceGrant) -> Result<()> {
    let bytes=root.read_optional(Path::new(&grant.source_manifest.path),2*1024*1024)?.ok_or("Reference source manifest missing")?;
    if digest(&bytes)!=grant.source_manifest.sha256 { return Err("Reference source manifest changed".into()); }
    let manifest:HostSourceManifest=serde_json::from_slice(&bytes).map_err(|_|"Reference source manifest schema")?;
    if manifest.sources.is_empty() || manifest.sources.len()>4096 { return Err("Reference source bounds".into()); }
    let (mut previous,mut hash,mut initializer)=(None,Sha256::new(),false);
    for input in manifest.sources {
        if previous.as_ref().is_some_and(|p:&String|p>=&input.path) { return Err("Reference source order/duplicates".into()); }
        reference_hash(&input.sha256)?;
        checked_hash(root,Path::new(&input.path),&input.sha256)?;
        initializer |= input.path==grant.initializer_source.path && input.sha256==grant.initializer_source.sha256;
        hash.update(input.path.as_bytes());hash.update([0]);hash.update(input.sha256.as_bytes());hash.update(b"\n");
        previous=Some(input.path);
    }
    if !initializer || format!("{:x}",hash.finalize())!=grant.source_set_sha256 {
        return Err("Reference exact initializer/source set differs".into());
    }
    Ok(())
}
#[link(name="System")]
unsafe extern "C" {
    static mach_task_self_: u32;
    fn mach_vm_region_recurse(task:u32,address:*mut u64,size:*mut u64,depth:*mut u32,info:*mut i32,count:*mut u32)->i32;
}
// macOS SDK mach/vm_region.h: vm_region_submap_short_info_64, pack4,
// MACH_VM_REGION_SUBMAP_SHORT_INFO_COUNT_64 == 12. No acquired port to release.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
/// Exact model input to the original SnippetChoicePanel initializer.
/// The allocator or super-init must first prove the compiled caller and keep
/// the original model borrow live. This helper never retains or releases it.
unsafe fn snippet_panel_model_frame(base: usize, model: usize) -> Result<objc2_foundation::NSRect> {
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let expected_class = base.checked_add(0x3f43d8)
        .ok_or("Reference SnippetChoiceModel class overflow")?;
    let class = super::metadata::loaded_class(c"RuycastF1Reference.SnippetChoiceModel")?;
    if class.address() != expected_class || model == 0 || !model.is_multiple_of(8)
        || reference_mapping_protection(model, 0xd0)? & 7 != 3
    {
        return Err("Reference SnippetChoicePanel model mapping or class differs".into());
    }
    // Native Swift layout used by the exact pinned initializer at 0x1a2e78.
    // The caller must reject stale pointers before calling this projection.
    if unsafe { (model as *const usize).read_volatile() } != expected_class {
        return Err("Reference SnippetChoicePanel model metadata differs".into());
    }
    let field = model.checked_add(0xc8).ok_or("Reference SnippetChoiceModel fields overflow")?;
    let storage = unsafe { (field as *const usize).read_volatile() };
    if storage == 0 || !storage.is_multiple_of(8) {
        return Err("Reference SnippetChoiceModel fields storage differs".into());
    }
    if reference_mapping_protection(storage, 0x18)? & 5 != 1 {
        return Err("Reference SnippetChoiceModel fields mapping differs".into());
    }
    let count_field = storage.checked_add(0x10).ok_or("Reference template fields count overflow")?;
    let count = unsafe { (count_field as *const usize).read_volatile() };
    if count > isize::MAX as usize
        || unsafe { (field as *const usize).read_volatile() } != storage
    {
        return Err("Reference SnippetChoiceModel fields changed or have invalid count".into());
    }
    let height = (130.0 + 70.0 * count as f64).min(440.0);
    Ok(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(456.0, height)))
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_reference_process_table(_: usize, _: *mut usize) -> usize {
    core::arch::naked_asm!(
        "stp x19, x21, [sp, #-32]!",
        "str x30, [sp, #16]",
        "mov x19, x1",
        "mov x21, xzr",
        "blr x0",
        "str x21, [x19]",
        "ldr x30, [sp, #16]",
        "ldp x19, x21, [sp], #32",
        "ret",
    );
}

fn reference_mapping(address:usize, bytes:usize, execute:Option<bool>) -> Result<()> {
    let protection = reference_mapping_protection(address, bytes)?;
    if protection & 1 == 0 || protection & 2 != 0 || execute.is_some_and(|x| (protection & 4 != 0) != x) {
        return Err(format!(
            "Reference mapping protections differ at {address:#x}, length {bytes}, observed {protection:#x}, execute {execute:?}"
        ));
    }
    Ok(())
}
fn reference_mapping_protection(address:usize, bytes:usize) -> Result<i32> {
    let end=address.checked_add(bytes).ok_or("Reference mapping overflow")? as u64;
    if address==0 || bytes==0 { return Err("Reference empty mapping".into()); }
    let mut depth=0u32;
    for _ in 0..16 {
        let (mut start,mut size,mut info,mut count)=(address as u64,0u64,[0i32;12],12u32);
        let status=unsafe { mach_vm_region_recurse(mach_task_self_,&mut start,&mut size,&mut depth,info.as_mut_ptr(),&mut count) };
        if status!=0 || count!=12 || start>address as u64 || start.checked_add(size).is_none_or(|n|n<end) {
            return Err("Reference complete mapping absent".into());
        }
        if info[8]!=0 { depth=depth.checked_add(1).ok_or("Reference submap overflow")?;continue; }
        return Ok(info[0]);
    }
    Err("Reference submap bound".into())
}
fn reference_mapping_infrastructure()->Result<()> {
    if super::metadata::code_identity(mach_vm_region_recurse as *const () as usize)?.image()!=b"/usr/lib/system/libsystem_kernel.dylib" {
        return Err("Reference mapping infrastructure differs".into());
    }
    Ok(())
}
fn reference_code(address:usize,image:&Path)->Result<()> {
    if super::metadata::code_identity(address)?.image()!=image.as_os_str().as_encoded_bytes() {
        return Err("Reference typed code image differs".into());
    }
    reference_mapping(address,1,Some(true))
}
fn reference_image_header(address:usize,image:&Path)->Result<usize> {
    reference_code(address,image)?;
    let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
    if unsafe { libc::dladdr(address as *const libc::c_void,info.as_mut_ptr()) }==0 {
        return Err("Reference code image missing".into());
    }
    let header=unsafe { info.assume_init() }.dli_fbase as usize;
    let count=unsafe { _dyld_image_count() };
    if count>4096 || (0..count).filter(|i|unsafe{_dyld_get_image_header(*i) as usize==header}).count()!=1 {
        return Err("Reference exact loaded image missing/duplicate".into());
    }
    Ok(header)
}
// Parse only loader-owned, VM-checked headers. Main permits exactly the one
// pinned offset initializer; Reference SDK permits zero initialization hooks.
fn reference_image_policy(header:usize,main:bool,initializer:usize,table:usize)->Result<()> {
    reference_mapping(header,32,None)?;
    let word=|at:usize| unsafe { (at as *const u32).read_unaligned() };
    let wide=|at:usize| unsafe { (at as *const u64).read_unaligned() };
    if word(header)!=0xfeedfacf || word(header+12)!=if main{2}else{6} {
        return Err("Reference MachO image kind differs".into());
    }
    let (commands,bytes)=(word(header+16) as usize,word(header+20) as usize);
    if commands>4096 || bytes>16*1024*1024 { return Err("Reference load command bounds".into()); }
    reference_mapping(header,32+bytes,None)?;
    let count=unsafe{_dyld_image_count()};
    if count>4096 { return Err("Reference loaded image bound".into()); }
    let index=(0..count).find(|i|unsafe{_dyld_get_image_header(*i) as usize==header}).ok_or("Reference image absent")?;
    let slide=unsafe{_dyld_get_image_vmaddr_slide(index)};
    let (mut at,mut initializers,mut tables)=(0usize,0usize,0usize);
    for _ in 0..commands {
        if at.checked_add(8).is_none_or(|v|v>bytes) { return Err("Reference truncated load command".into()); }
        let p=header+32+at;
        let (kind,size)=(word(p),word(p+4) as usize);
        if size<8 || at.checked_add(size).is_none_or(|v|v>bytes) { return Err("Reference invalid load command".into()); }
        if matches!(kind,0x11|0x1a|0x27) { return Err("Reference routines/loader environment denied".into()); }
        if kind==0x19 {
            if size<72 { return Err("Reference segment truncated".into()); }
            let sections=word(p+64) as usize;
            if sections>4096 || 72+sections*80>size { return Err("Reference section bounds".into()); }
            let (vm,len)=(wide(p+24),wide(p+32));
            for i in 0..sections {
                let sec=p+72+i*80;
                let name=unsafe{std::slice::from_raw_parts(sec as *const u8,16)};
                let (addr,len_sec,flags)=(wide(sec+32),wide(sec+40),word(sec+64)&0xff);
                if len_sec==0 { continue; }
                if nonlazy_objc_initializer(name) || flags==0x16 && !main || matches!(flags,9|0xa|0xd|0x15)
                    || name.split(|b|*b==0).next()==Some(b"__interpose") {
                    return Err("Reference unexpected initialization/termination section".into());
                }
                if flags==0x16 || name.split(|b|*b==0).next()==Some(b"__ruycast_ref") {
                    if addr<vm || addr.checked_add(len_sec).is_none_or(|v|vm.checked_add(len).is_none_or(|end|v>end)) {
                        return Err("Reference section outside its actual segment".into());
                    }
                    let address=usize::try_from(addr).map_err(|_|"Reference section address overflow")?
                        .checked_add_signed(slide).ok_or("Reference section slide overflow")?;
                    if flags==0x16 {
                        if !main || len_sec!=4 { return Err("Reference initializer roster differs".into()); }
                        reference_mapping(address,4,None)?;
                        if header.checked_add(word(address) as usize)!=Some(initializer) { return Err("Reference initializer offset differs".into()); }
                        initializers+=1;
                    }
                    if name.split(|b|*b==0).next()==Some(b"__ruycast_ref") {
                        let segment=unsafe{std::slice::from_raw_parts((sec+16) as *const u8,16)};
                        if !main || address!=table || len_sec!=216 || segment.split(|b|*b==0).next()!=Some(b"__DATA_CONST") {
                            return Err("Reference exact table section differs".into());
                        }
                        tables+=1;
                    }
                }
            }
        }
        at+=size;
    }
    if at!=bytes || initializers!=usize::from(main) || tables!=usize::from(main) { return Err("Reference initialization/table roster differs".into()); }
    Ok(())
}
fn reference_loaded_table(endpoint:&Path,sdk:&Path,grant:&ReferenceGrant,table:usize,sdk_addresses:[(&'static str,usize);5])->Result<()> {
    // The kernel region query itself is a fixed, actual system import.
    reference_mapping_infrastructure()?;
    reference_code_pins(&grant.main_functions,&REFERENCE_MAIN_SYMBOLS)?;
    reference_code_pins(&grant.sdk_functions,&REFERENCE_SDK_SYMBOLS)?;
    let main=unsafe{_NSGetMachExecuteHeader()} as usize;
    if main==0 || !table.is_multiple_of(8) || main.checked_add(grant.main_table_offset)!=Some(table) {
        return Err("Reference incoming table is not the private pinned main address".into());
    }
    if sdk_addresses.iter().zip(REFERENCE_SDK_SYMBOLS).any(|((name,address),expected)|*name!=expected || *address==0) {
        return Err("Reference SDK typed entrypoints differ".into());
    }
    let sdk_header=reference_image_header(sdk_addresses[0].1,sdk)?;
    if sdk_header==main { return Err("Reference SDK cannot be main".into()); }
    let initializer=main.checked_add(grant.main_initializer_offset).ok_or("Reference main initializer overflow")?;
    reference_code(initializer,endpoint)?;
    reference_image_policy(main,true,initializer,table)?;
    reference_image_policy(sdk_header,false,0,0)?;
    for (pin,(_,address)) in grant.sdk_functions.iter().zip(sdk_addresses) {
        if sdk_header.checked_add(pin.image_offset)!=Some(address) { return Err("Reference SDK source offset differs".into()); }
        reference_code(address,sdk)?;
    }
    // This complete read-only range and exact private offset are established
    // BEFORE the first word of the caller-supplied table can be dereferenced.
    reference_mapping(table,216,Some(false))?;
    let magic=unsafe{(table as *const u64).read()};
    let abi=unsafe{((table+8) as *const u32).read()};
    let count=unsafe{((table+12) as *const u32).read()};
    if magic!=0xF10000264D41494E || abi!=1 || count!=25 { return Err("Reference table ABI differs".into()); }
    for (i,pin) in grant.main_functions.iter().enumerate() {
        let expected=main.checked_add(pin.image_offset).ok_or("Reference function offset overflow")?;
        let actual=unsafe{((table+16+i*8) as *const usize).read()};
        if actual!=expected { return Err("Reference table source slot differs".into()); }
        reference_code(actual,endpoint)?;
    }
    Ok(())
}
pub fn enter_reference_with_guard(pins:&ReferencePins<'_>,guard:guardian::Guard,table_address:usize,sdk_functions:[(&'static str,usize);5])->Result<ReferenceAdmission> {
    #[cfg(not(rcf1_reference_sdk))]
    if !super::SOURCE_ROUTES_QUALIFIED { return Err("Reference source routes are not qualified".into()); }
    let life:Arc<dyn AliveLease>=Arc::new(ForkAwareLease::new(Arc::new(GuardianAlive::admitted(guard))));
    host_args(pins.endpoint)?;
    let entries=reference_environment_snapshot()?;
    // Before cap consumption, reject duplicate keys and require exactly one
    // descriptor locator. All fifteen exact keys/values follow private decoding.
    let mut seen=std::collections::BTreeSet::new();
    if entries.len()!=15 || entries.iter().any(|(k,_)|!seen.insert(k.as_str())) { return Err("Reference environment count/duplicates".into()); }
    let grant_fd=entries.iter().find(|(k,_)|k=="F1_CORE_GRANT_FD").ok_or("Reference grant missing")?.1
        .parse::<i32>().map_err(|_|"Reference grant descriptor encoding")?;
    let life_fd=entries.iter().find(|(k,_)|k=="F1_LIFE_FD").ok_or("Reference life missing")?.1
        .parse::<i32>().map_err(|_|"Reference life descriptor encoding")?;
    let admit_fd=entries.iter().find(|(k,_)|k=="F1_ADMIT_FD").ok_or("Reference admit missing")?.1
        .parse::<i32>().map_err(|_|"Reference admit descriptor encoding")?;
    if grant_fd<=2 || life_fd<=2 || admit_fd<=2 || grant_fd==life_fd || grant_fd==admit_fd || life_fd==admit_fd {
        return Err("Reference descriptor alias".into());
    }
    // Reject a duplicated endpoint before consuming the private grantcap.
    // This does not discover whether different endpoints are connected peers.
    stdio(&pins.stdio)?;
    let life_identity=Fingerprint::own(life_fd)?;
    if reference_endpoint_alias(life_identity,pins.grant)
        || pins.stdio.iter().any(|s|reference_endpoint_alias(*s,pins.grant)||reference_endpoint_alias(*s,life_identity)) {
        return Err("Reference grant/life/stdio endpoint alias".into());
    }
    if life_identity.kind!=u32::from(libc::S_IFIFO) || life_identity.access!=libc::O_RDONLY {
        return Err("Reference life descriptor kind/access differs".into());
    }
    let grant:ReferenceGrant=read_private_host_grant(grant_fd,pins.grant,pins.owner_uid,life.as_ref())?;
    if grant.owner_uid!=pins.owner_uid || grant.role!="full-owned-reference-v1" || grant.endpoint_sha256!=pins.endpoint_sha256
        || digest(grant.nonce.as_bytes())!=pins.nonce_sha256 || grant.stdio!=pins.stdio
        || Path::new(&grant.source_manifest.path)!=pins.source_manifest || Path::new(&grant.initializer_source.path)!=pins.initializer_source {
        return Err("Reference private role/source/nonce/stdio differs".into());
    }
    let (checked_life_fd,_,checked_grant_fd)=reference_environment(&entries,&reference_environment_values(&grant,pins.sdk_artifact)?)?;
    // Guard::bootstrap already consumed and closed ADMIT. Its retained decimal
    // locator is checked for aliases above, never reopened or treated as a live
    // capability after that handoff. LIFE remains the actual Guardian resource.
    if checked_grant_fd!=grant_fd || checked_life_fd!=life_fd || Fingerprint::own(life_fd)?!=life_identity {
        return Err("Reference grant/guardian capability changed".into());
    }
    let root=OwnedRoot::open(pins.root)?;
    if root.owner_ids()?.0!=pins.owner_uid { return Err("Reference owner differs".into()); }
    let r=&grant.runtime;
    let roots=[OwnedRoot::open(Path::new(&r.home))?,OwnedRoot::open(Path::new(&r.cache))?,OwnedRoot::open(Path::new(&r.temp))?,OwnedRoot::open(Path::new(&r.applications))?];
    let profile_directory = roots[0].open_watch_directory(Path::new(&r.profile))?;
    reference_profile_identity(&profile_directory)?;
    let admission=ReferenceAdmission {
        admitted:Admitted{life,descriptors:DescriptorTable::new(),imports:vec![],stdio:pins.stdio},
        root,owner:std::thread::current().id(),endpoint:pins.endpoint.into(),sdk:pins.sdk_artifact.into(),
        grant,table:table_address,sdk_addresses:sdk_functions,roots,profile_directory,
        #[cfg(rcf1_reference_sdk)]
        registry_retired:std::sync::OnceLock::new(),
    };
    admission.before_reference_initialization()?;
    Ok(admission)
}
#[cfg(test)]
mod reference_admission_tests {
    use super::*;
    #[test]
    fn callback_source_observation_boundaries_and_retirement() {
        use std::{cell::{Cell,RefCell},sync::atomic::{AtomicBool,Ordering}};
        let scope=CallbackSourceObservation::default();let retired=AtomicBool::new(false);
        let full=Cell::new(0);let live=Cell::new(0);
        for _ in 0..2 {
            scope.run(&retired,||{
                assert!(!scope.reusing());full.set(full.get()+1);Ok(())
            },||{
                for _ in 0..32 {
                    assert!(scope.reusing());live.set(live.get()+1);
                }
                Ok(())
            }).unwrap();
            assert!(!scope.reusing());
        }
        assert_eq!((full.get(),live.get()),(4,64));
        assert!(!retired.load(Ordering::Acquire));

        // Real content hashing at both boundaries refuses a changed synthetic
        // source even though no internal observation publishes a result.
        let scope=CallbackSourceObservation::default();let retired=AtomicBool::new(false);
        let source=RefCell::new(b"owned synthetic source".to_vec());let expected=digest(&source.borrow());
        let full=Cell::new(0);
        assert!(scope.run(&retired,||{
            assert!(!scope.reusing());full.set(full.get()+1);
            if digest(&source.borrow())!=expected {return Err("source changed".into());}Ok(())
        },||{source.borrow_mut()[0]^=1;Ok(())}).is_err());
        assert_eq!(full.get(),2);assert!(!scope.reusing());assert!(retired.load(Ordering::Acquire));

        for failure in ["before","body","nested","retired","panic"] {
            let scope=CallbackSourceObservation::default();let retired=AtomicBool::new(false);
            let entered=Cell::new(false);
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||scope.run(&retired,||{
                assert!(!scope.reusing());
                if failure=="before" {Err("boundary refused".into())} else {Ok(())}
            },||{
                entered.set(true);assert!(scope.reusing());
                match failure {
                    "body"=>return Err("observation refused".into()),
                    "nested"=>{assert!(scope.run(&retired,||Ok(()),||Ok(())).is_err());},
                    "retired"=>retired.store(true,Ordering::Release),
                    "panic"=>panic!("owned observation panic"),
                    _=>{},
                }
                Ok(())
            })));
            assert!(outcome.is_err()||outcome.unwrap().is_err(),"{failure}");
            assert_eq!(entered.get(),failure!="before");
            assert!(!scope.reusing());assert_eq!(scope.phase.load(Ordering::Acquire),0);
            assert!(retired.load(Ordering::Acquire));
            assert!(scope.run(&retired,||Ok(()),||Ok(())).is_err());
        }
        // Reentry is refused during the full boundary check as well.
        let scope=CallbackSourceObservation::default();let retired=AtomicBool::new(false);
        assert!(scope.run(&retired,||scope.run(&retired,||Ok(()),||Ok(())),||Ok(())).is_err());
        assert_eq!(scope.phase.load(Ordering::Acquire),0);assert!(retired.load(Ordering::Acquire));
    }
    fn grant() -> ReferenceGrant {
        let fp=Fingerprint{device:1,inode:2,kind:u32::from(libc::S_IFIFO),access:libc::O_RDONLY};
        let input=||HostInput{path:"/owned/source.rs".into(),sha256:"a".repeat(64)};
        ReferenceGrant {
            owner_uid:501,role:"full-owned-reference-v1".into(),endpoint_sha256:"a".repeat(64),
            sdk_artifact_sha256:"b".repeat(64),source_set_sha256:"c".repeat(64),nonce:"private-unit-only".into(),
            stdio:[fp;3],source_manifest:input(),initializer_source:input(),
            main_initializer_offset:16,main_table_offset:216,
            main_functions:REFERENCE_MAIN_SYMBOLS.iter().enumerate().map(|(i,s)|ReferenceCodePin{symbol:(*s).into(),image_offset:32+i}).collect(),
            sdk_functions:REFERENCE_SDK_SYMBOLS.iter().enumerate().map(|(i,s)|ReferenceCodePin{symbol:(*s).into(),image_offset:64+i}).collect(),
            runtime:ReferenceRuntime{home:"/owned/home".into(),cache:"/owned/cache".into(),temp:"/owned/temp".into(),
                applications:"/owned/applications".into(),profile:"/owned/home/profile".into(),fixture:input(),authority_sha256:"d".repeat(64),arm:"A".into()},
        }
    }
    #[test]
    fn reference_ranking_reads_exact_owned_leaf_and_rejects_postcheck_replacement() {
        let fixture=crate::workspace::tests::Fixture::new();
        let home=OwnedRoot::open(&fixture.root.join("home")).unwrap();
        let profile=fixture.root.join("home/profile");std::fs::create_dir(&profile).unwrap();
        let path=profile.join("search-ranking.json");let uid=unsafe{libc::geteuid()};
        assert!(reference_process_ranking_bytes(&home,&profile,uid,||Ok(())).is_err());
        let bytes=br#"{"schema":1,"entries":{"owned.synthetic":946728000.0}}"#;
        std::fs::write(&path,bytes).unwrap();
        assert_eq!(reference_process_ranking_bytes(&home,&profile,uid,||Ok(())).unwrap(),bytes);
        assert!(reference_process_ranking_bytes(&home,&profile,uid,|| {
            let replacement=profile.join("ranking-replacement");std::fs::write(&replacement,bytes).unwrap();
            std::fs::rename(replacement,&path).unwrap();Ok(())
        }).is_err());
        std::fs::write(&path,vec![0u8;1_000_001]).unwrap();
        assert!(reference_process_ranking_bytes(&home,&profile,uid,||Ok(())).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(profile.join("absent"),&path).unwrap();
        assert!(reference_process_ranking_bytes(&home,&profile,uid,||Ok(())).is_err());
        // Actual owned IO and nested replacement, no fake Source14 admission.
    }
    #[cfg(rcf1_reference_sdk)]
    #[test]
    fn reference_startup_profile_requires_only_pinned_config() {
        let fixture = crate::workspace::tests::Fixture::new();
        let home = OwnedRoot::open(&fixture.root.join("home")).unwrap();
        let profile = home.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let config = profile.join("config.json");
        let pin = digest(b"{}");
        let verify = || reference_private_startup_profile(&home, &profile, &pin);
        assert!(verify().is_err());
        home.create_file(&config, b"{}").unwrap();
        assert!(verify().is_ok());
        for leaf in ["config-location.json", "instance.lock", "control.sock", "clipboard.json"] {
            let path = profile.join(leaf);
            std::fs::write(&path, []).unwrap();
            assert!(verify().is_err());
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(profile.join("absent"), &path).unwrap();
            assert!(verify().is_err());
            std::fs::remove_file(&path).unwrap();
        }
        assert!(verify().is_ok());
        std::fs::write(&config, b"{ }").unwrap();
        assert!(verify().is_err());
        std::fs::remove_file(&config).unwrap();
        let foreign = home.path().join("foreign.json");
        home.create_file(&foreign, b"{}").unwrap();
        std::fs::hard_link(&foreign, &config).unwrap();
        assert!(verify().is_err());
        std::fs::remove_file(&config).unwrap();
        std::os::unix::fs::symlink(&foreign, &config).unwrap();
        assert!(verify().is_err());
    }
    #[test]
    fn reference_control_leaves_require_private_socket_and_lock() {
        use std::os::unix::{fs::{PermissionsExt, symlink}, net::UnixListener};
        let fixture = crate::workspace::tests::Fixture::new();
        let path = fixture.root.join("home");
        let profile: OwnedFd = std::fs::File::open(&path).unwrap().into();
        let uid = unsafe { libc::geteuid() };
        let lock = path.join("instance.lock");
        let socket = path.join("control.sock");
        let verify = || verify_reference_control_leaves(&profile, uid);
        assert!(verify().is_err());
        std::fs::write(&lock, []).unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&lock, &socket).unwrap();
        assert!(verify().is_err());
        std::fs::remove_file(&socket).unwrap();
        let _listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(verify().is_err());
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify().is_ok());
    }
    #[test]
    fn reference_profile_directory_rejects_replacement_and_public_permissions() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let fixture = crate::workspace::tests::Fixture::new();
        let home = OwnedRoot::open(&fixture.root.join("home")).unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = home.path().join("profile");
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&profile).unwrap();
        let held = home.open_watch_directory(&profile).unwrap();
        verify_reference_profile_directory(&home, &profile, &held).unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(verify_reference_profile_directory(&home, &profile, &held).is_err());
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::rename(&profile, home.path().join("previous-profile")).unwrap();
        builder.create(&profile).unwrap();
        assert!(verify_reference_profile_directory(&home, &profile, &held).is_err());
        std::fs::remove_dir(&profile).unwrap();
        std::fs::rename(home.path().join("previous-profile"), &profile).unwrap();
        assert!(verify_reference_profile_directory(&home, &profile, &held).is_ok());
        std::fs::rename(home.path(), fixture.root.join("previous-home")).unwrap();
        builder.create(home.path()).unwrap();
        builder.create(&profile).unwrap();
        assert!(verify_reference_profile_directory(&home, &profile, &held).is_err());
    }
    #[test]
    fn reference_epoch_is_from_pinned_owned_manifest_not_a_clock_or_notice() {
        use std::os::unix::{ffi::OsStrExt,fs::symlink};
        let fixture=crate::workspace::tests::Fixture::new();
        let root=OwnedRoot::open(&fixture.root).unwrap();
        let path=fixture.root.join("fixture.json");
        let good=br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":946728000,"files":{}}"#;
        std::fs::write(&path,good).unwrap();
        let mut input=HostInput{path:path.to_str().unwrap().into(),sha256:digest(good)};
        let uid=unsafe{libc::geteuid()};
        assert_eq!(reference_fixture_epoch(&root,&input,uid).unwrap(),946728000);
        std::fs::write(&path,br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":946728001}"#).unwrap();
        assert!(reference_fixture_epoch(&root,&input,uid).is_err());
        for bad in [br#"{"schema":"wrong","epoch_unix_seconds":1}"#.as_slice(),
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":-1}"#,
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1.5}"#,
            br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1,"epoch_unix_seconds":2}"#] {
            std::fs::write(&path,bad).unwrap();input.sha256=digest(bad);
            assert!(reference_fixture_epoch(&root,&input,uid).is_err());
        }
        std::fs::write(&path,good).unwrap();input.sha256=digest(good);
        let alias=fixture.root.join("alias");symlink(&path,&alias).unwrap();
        input.path=alias.to_str().unwrap().into();
        assert!(reference_fixture_epoch(&root,&input,uid).is_err());
        input.path=path.to_str().unwrap().into();
        std::fs::hard_link(&path,fixture.root.join("hardlink")).unwrap();
        assert!(reference_fixture_epoch(&root,&input,uid).is_err());
        let fifo=fixture.root.join("fifo");
        let name=std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);
        input.path=fifo.to_str().unwrap().into();
        assert!(reference_fixture_epoch(&root,&input,uid).is_err());
        // Pure manifest custody only; no sealed admission/Source14 is issued.
    }
    #[test]
    fn reference_exact_environment_never_admits_native_bridge_or_ambient_fields() {
        let grant=grant();
        let expected=reference_environment_values(&grant,Path::new("/owned/sdk.dylib")).unwrap();
        let mut entries:Vec<_>=expected.iter().map(|(k,v)|((*k).into(),v.clone())).collect();
        entries.extend([("F1_LIFE_FD".into(),"4".into()),("F1_ADMIT_FD".into(),"5".into()),("F1_CORE_GRANT_FD".into(),"6".into())]);
        assert_eq!(reference_environment(&entries,&expected).unwrap(),(4,5,6));
        for (key,value) in [("F1_NATIVE_GRANT_FD","7"),("RUYCAST_F1_REFERENCE_BRIDGE","/unowned"),("CARGO_HOME","/ambient")] {
            let mut wrong=entries.clone();wrong[0]=(key.into(),value.into());
            assert!(reference_environment(&wrong,&expected).is_err());
        }
        let mut wrong=entries.clone();wrong[14].1="4".into();
        assert!(reference_environment(&wrong,&expected).is_err());
        wrong[14].1="not-a-descriptor".into();
        assert_eq!(reference_environment(&wrong,&expected).unwrap_err(),"Reference grant descriptor encoding");
        let mut duplicate=entries.clone();duplicate[0]=duplicate[1].clone();
        assert!(reference_environment(&duplicate,&expected).is_err());
        let mut changed=entries.clone();changed.iter_mut().find(|(k,_)|k=="__CF_USER_TEXT_ENCODING").unwrap().1="0x0:0x0:0x0".into();
        assert!(reference_environment(&changed,&expected).is_err());
        assert!(reference_environment(&entries[..14],&expected).is_err());
        entries.push(("EXTRA".into(),"private-value-never-echoed".into()));
        assert_eq!(reference_environment(&entries,&expected).unwrap_err(),"Reference exact environment count");
    }
    #[test]
    fn reference_source_slots_have_exact_order_and_no_native_schema_fallback() {
        let mut descriptors=[-1;2];
        assert_eq!(unsafe{libc::pipe(descriptors.as_mut_ptr())},0);
        let read=unsafe{OwnedFd::from_raw_fd(descriptors[0])};
        let write=unsafe{OwnedFd::from_raw_fd(descriptors[1])};
        let (ro,wo)=(Fingerprint::own(read.as_raw_fd()).unwrap(),Fingerprint::own(write.as_raw_fd()).unwrap());
        assert_ne!(ro,wo);
        // Darwin exposes distinct fstat endpoint identities for this connected
        // pair. Inequality does not establish channel separation.
        assert!(!reference_endpoint_alias(ro,wo));
        assert_eq!((ro.access,wo.access),(libc::O_RDONLY,libc::O_WRONLY));
        let duplicate=unsafe{libc::fcntl(read.as_raw_fd(),libc::F_DUPFD_CLOEXEC,3)};
        assert!(duplicate>=3);
        let duplicate=unsafe{OwnedFd::from_raw_fd(duplicate)};
        let duplicated=Fingerprint::own(duplicate.as_raw_fd()).unwrap();
        assert_eq!(duplicated,ro);
        assert!(reference_endpoint_alias(ro,duplicated));
        let mut grant=grant();
        reference_code_pins(&grant.main_functions,&REFERENCE_MAIN_SYMBOLS).unwrap();
        reference_code_pins(&grant.sdk_functions,&REFERENCE_SDK_SYMBOLS).unwrap();
        grant.main_functions.swap(0,1);
        assert!(reference_code_pins(&grant.main_functions,&REFERENCE_MAIN_SYMBOLS).is_err());
        grant.sdk_functions[0].symbol="initializer".into();
        assert!(reference_code_pins(&grant.sdk_functions,&REFERENCE_SDK_SYMBOLS).is_err());
        grant.sdk_functions[0].symbol="entry".into();grant.sdk_functions[0].image_offset=0;
        assert!(reference_code_pins(&grant.sdk_functions,&REFERENCE_SDK_SYMBOLS).is_err());
        let mut value=serde_json::to_value(grant).unwrap();
        value.as_object_mut().unwrap().insert("native_grant_fd".into(),serde_json::json!(9));
        assert!(serde_json::from_value::<ReferenceGrant>(value).is_err());
    }
}
/// Actual dedicated initializer installer. finish_host is pinned producer source,
/// not a receipt parser. It must consume the sealed admission through Bootstrap
/// and the qualified Native sink; a missing sink latch makes Native exit86.
#[macro_export]
macro_rules! full_owned_host_initializer {
    ($make_pins:path,$finish_host:path) => {
        #[cfg(rcf1_reference_sdk)]
        compile_error!("Reference SDK role forbids SDK initializers; use its explicit entry");
        extern "C" fn ruycast_full_owned_host_initializer() {
            if !$crate::sdk::SOURCE_ROUTES_QUALIFIED {
                unsafe { libc::_exit(86) }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let guard = $crate::guardian::Guard::bootstrap()
                    .map_err(|_| "Host guardian admission failed".to_string())?;
                let pins = $make_pins();
                let admission = $crate::sdk::core_preentry::enter_host_with_guard(
                    &pins,
                    guard,
                    ruycast_full_owned_host_initializer as *const () as usize,
                )?;
                $finish_host(admission)
            }));
            match result {
                Ok(Ok(())) => (),
                Ok(Err(_)) => unsafe { libc::_exit(86) },
                Err(payload) => {
                    std::mem::forget(payload);
                    unsafe { libc::_exit(86) }
                }
            }
        }
        #[used]
        #[link_section = "__DATA,__mod_init_func"]
        static FULL_OWNED_HOST_INITIALIZER: extern "C" fn() = ruycast_full_owned_host_initializer;
    };
}

#[cfg(test)]
mod host_entry_tests {
    use super::*;
    #[test]
    fn unknown_host_refuses_before_guard_or_unowned_source_queries() {
        const { assert!(!super::super::SOURCE_ROUTES_QUALIFIED) };
        let absent = Path::new("/unowned-host-negative-no-query");
        let fingerprint = Fingerprint {
            device: 0,
            inode: 0,
            kind: 0,
            access: 0,
        };
        let pins=HostPins{owner_uid:0,root:absent,endpoint:absent,endpoint_sha256:"",sdk_artifact:absent,
            nonce_sha256:"",grant:fingerprint,stdio:[fingerprint;3],source_manifest:absent,
            initializer_source:absent,native_grant_fd:-1,native_grant:fingerprint};
        let error = match enter_host(&pins, 0) {
            Ok(_) => panic!("unqualified Host cannot be admitted"),
            Err(error) => error,
        };
        assert_eq!(error, "owned Host source routes are not qualified");
    }
    #[test]
    fn initializer_roster_rejects_routines_and_nonlazy_objc() {
        fn header(command: u32, name: &[u8]) -> Vec<u8> {
            let size = if command == 0x19 { 152 } else { 8 };
            let mut bytes = vec![0u8; 32 + size];
            bytes[0..4].copy_from_slice(&0xfeedfacfu32.to_ne_bytes());
            bytes[12..16].copy_from_slice(&2u32.to_ne_bytes());
            bytes[16..20].copy_from_slice(&1u32.to_ne_bytes());
            bytes[20..24].copy_from_slice(&(size as u32).to_ne_bytes());
            bytes[32..36].copy_from_slice(&command.to_ne_bytes());
            bytes[36..40].copy_from_slice(&(size as u32).to_ne_bytes());
            if command == 0x19 {
                bytes[96..100].copy_from_slice(&1u32.to_ne_bytes());
                bytes[104..104 + name.len()].copy_from_slice(name);
                bytes[144..152].copy_from_slice(&8u64.to_ne_bytes());
            }
            bytes
        }
        for command in [0x11, 0x1a] {
            let bytes = header(command, b"");
            assert!(unsafe { check_main_initializers(bytes.as_ptr()) }.unwrap_err().contains("LC_ROUTINES"));
        }
        for name in [b"__objc_nlclslist".as_slice(), b"__objc_nlcatlist"] {
            let bytes = header(0x19, name);
            assert!(unsafe { check_main_initializers(bytes.as_ptr()) }.is_err());
        }
        let bytes = header(0x19, b"__objc_classlist");
        unsafe { check_main_initializers(bytes.as_ptr()) }.unwrap();
        let bytes = header(0x80000034, b"");
        // Relocation metadata is not a blanket initializer denial.
        unsafe { check_main_initializers(bytes.as_ptr()) }.unwrap();
    }
    #[test]
    fn sdk_grant_uid_refuses_before_read_or_callback_and_preserves_borrow() {
        struct NoCallback;
        impl AliveLease for NoCallback {
            fn is_alive(&self) -> bool { panic!("UID refusal must precede dynamic callbacks") }
            fn before_effect(&self) -> Result<()> { panic!("UID refusal must precede dynamic callbacks") }
        }
        let (read, _write) = guardian::pipe().unwrap();
        let fd = read.as_raw_fd();
        let pin = Fingerprint::own(fd).unwrap();
        let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::fstat(fd, &mut metadata) }, 0);
        let error = match read_host_grant(fd, pin, metadata.st_uid ^ 1, &NoCallback) {
            Ok(_) => panic!("wrong privatecap owner admitted"),
            Err(error) => error,
        };
        assert!(error.contains("owner differs"));
        assert_eq!(Fingerprint::own(fd).unwrap(), pin);
    }

}
