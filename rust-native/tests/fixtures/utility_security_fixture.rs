//! Standalone std-only test dylib. No real Security API is declared or called.
//! Complete named SDK table for NativeStorage::fixture; owned HOME/marker only.
use std::{
    ffi::c_void,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
    time::{Duration, Instant},
};
type Ptr = *mut c_void;
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: Ptr, key: Ptr) -> Ptr;
    fn CFDictionaryGetCount(dictionary: Ptr) -> isize;
    fn CFStringCreateWithCString(allocator: Ptr, value: *const i8, encoding: u32) -> Ptr;
    fn CFEqual(left: Ptr, right: Ptr) -> u8;
    fn CFRelease(value: Ptr);
    fn CFDataCreate(allocator: Ptr, bytes: *const u8, length: isize) -> Ptr;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(data: Ptr) -> isize;
    fn CFDataGetBytePtr(data: Ptr) -> *const u8;
    static kCFBooleanTrue: Ptr;
}
#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecClass: Ptr;
    static kSecClassGenericPassword: Ptr;
    static kSecAttrService: Ptr;
    static kSecAttrAccount: Ptr;
    static kSecReturnData: Ptr;
    static kSecMatchLimit: Ptr;
    static kSecMatchLimitOne: Ptr;
    static kSecValueData: Ptr;
}
extern "C" {
    fn getuid() -> u32;
}
fn home() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("RUYCAST_UTILITY_FIXTURE_HOME")?);
    if !root.is_absolute() || std::env::var_os("HOME").map(PathBuf::from) != Some(root.clone()) {
        return None;
    }
    let metadata = fs::symlink_metadata(&root).ok()?;
    if !metadata.is_dir() || metadata.uid() != unsafe { getuid() } || metadata.mode() & 0o077 != 0 {
        return None;
    }
    if fs::read(root.join(".ruycast-test-fixture"))
        .ok()?
        .as_slice()
        != b"own-disposable-fixture-v1"
    {
        return None;
    }
    Some(root)
}
fn record(root: &std::path::Path, phase: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(root.join("utility-security-events"))
    {
        let _ = writeln!(file, "{phase}");
    }
}
fn query(query: Ptr, extra: isize) -> bool {
    if query.is_null()
        || unsafe {
            CFGetTypeID(query) != CFDictionaryGetTypeID()
                || CFDictionaryGetCount(query) != 3 + extra
        }
    {
        return false;
    }
    unsafe {
        let service = CFStringCreateWithCString(
            std::ptr::null_mut(),
            c"viggo-raycast-utilities".as_ptr(),
            0x08000100,
        );
        let account = CFStringCreateWithCString(
            std::ptr::null_mut(),
            c"password-history".as_ptr(),
            0x08000100,
        );
        if service.is_null() || account.is_null() {
            return false;
        }
        let valid = CFEqual(
            CFDictionaryGetValue(query, kSecClass),
            kSecClassGenericPassword,
        ) != 0
            && CFEqual(CFDictionaryGetValue(query, kSecAttrService), service) != 0
            && CFEqual(CFDictionaryGetValue(query, kSecAttrAccount), account) != 0;
        CFRelease(service);
        CFRelease(account);
        valid
    }
}
fn denied(root: &std::path::Path) -> bool {
    root.join("utility-denied").exists()
}
fn held(root: &std::path::Path, phase: &str) {
    if !root.join(format!("utility-hold-{phase}")).exists() {
        return;
    }
    let _ = fs::write(root.join(format!("utility-entered-{phase}")), b"entered");
    let deadline = Instant::now() + Duration::from_secs(8);
    while !root.join("utility-release").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn bytes(dictionary: Ptr) -> Option<Vec<u8>> {
    unsafe {
        let data = CFDictionaryGetValue(dictionary, kSecValueData);
        if data.is_null() || CFGetTypeID(data) != CFDataGetTypeID() {
            return None;
        }
        let length = CFDataGetLength(data);
        if !(0..=12_000_000).contains(&length) {
            return None;
        }
        if length == 0 {
            return Some(vec![]);
        }
        Some(std::slice::from_raw_parts(CFDataGetBytePtr(data), length as usize).to_vec())
    }
}
fn save(root: &std::path::Path, bytes: &[u8]) -> i32 {
    let stage = root.join(".utility-keychain-stage");
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
    {
        Ok(file) => file,
        Err(_) => return -1,
    };
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(stage);
        return -1;
    }
    if fs::rename(&stage, root.join("utility-keychain.bin")).is_err() {
        let _ = fs::remove_file(stage);
        return -1;
    }
    0
}
#[no_mangle]
pub extern "C" fn ruycast_utility_fixture_ready() -> bool {
    home().is_some()
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_utility_fixture_read(request: Ptr, result: *mut Ptr) -> i32 {
    let Some(root) = home() else {
        return -50000;
    };
    record(&root, "read");
    if !query(request, 2)
        || unsafe {
            CFDictionaryGetValue(request, kSecReturnData) != kCFBooleanTrue
                || CFDictionaryGetValue(request, kSecMatchLimit) != kSecMatchLimitOne
        }
    {
        record(&root, "invalid-query");
        return -50;
    }
    held(&root, "read");
    if denied(&root) {
        return -25308;
    }
    let data = match fs::read(root.join("utility-keychain.bin")) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return -25300,
        Err(_) => return -1,
    };
    if result.is_null() {
        return -50;
    }
    let data = unsafe { CFDataCreate(std::ptr::null_mut(), data.as_ptr(), data.len() as isize) };
    if data.is_null() {
        return -1;
    }
    unsafe {
        *result = data;
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_utility_fixture_update(request: Ptr, changes: Ptr) -> i32 {
    let Some(root) = home() else {
        return -50000;
    };
    record(&root, "update");
    if !query(request, 0) {
        record(&root, "invalid-query");
        return -50;
    }
    held(&root, "write");
    if denied(&root) {
        return -25308;
    }
    if !root.join("utility-keychain.bin").exists() {
        return -25300;
    }
    let Some(data) = bytes(changes) else {
        return -50;
    };
    save(&root, &data)
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_utility_fixture_add(request: Ptr, _: *mut Ptr) -> i32 {
    let Some(root) = home() else {
        return -50000;
    };
    record(&root, "add");
    if !query(request, 1) {
        record(&root, "invalid-query");
        return -50;
    }
    if denied(&root) {
        return -25308;
    }
    if root.join("utility-keychain.bin").exists() {
        return -25299;
    }
    let Some(data) = bytes(request) else {
        return -50;
    };
    save(&root, &data)
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_utility_fixture_random(
    _: Ptr,
    count: usize,
    bytes: *mut c_void,
) -> i32 {
    let Some(root) = home() else {
        return -50000;
    };
    record(&root, "random");
    if root.join("utility-random-denied").exists() {
        return -1;
    }
    if !matches!(count, 4 | 24) || bytes.is_null() {
        return -50;
    }
    for (index, value) in unsafe { std::slice::from_raw_parts_mut(bytes.cast::<u8>(), count) }
        .iter_mut()
        .enumerate()
    {
        *value = index as u8;
    }
    0
}
