//! Native Utilities storage. Construct on the serialized worker, not AppKit's
//! main thread. SDK test calls use a complete substituted table; no real
//! Keychain lookup is performed merely by loading this module.
use crate::{
    private_storage,
    utilities::{Storage, KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE},
};
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSDate, NSISO8601DateFormatter, NSString};
use std::{
    ffi::{c_void, CString},
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;
// Match the repository's existing opaque CF extern declarations. Pointees are
// immutable unless the SDK's separate API explicitly permits mutation.
type Ptr = *mut c_void;
const MAX_BYTES: usize = 12_000_000;
const NOT_FOUND: i32 = -25300;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecItemCopyMatching(query: Ptr, result: *mut Ptr) -> i32;
    fn SecItemUpdate(query: Ptr, changes: Ptr) -> i32;
    fn SecItemAdd(item: Ptr, result: *mut Ptr) -> i32;
    fn SecRandomCopyBytes(random: Ptr, count: usize, bytes: *mut c_void) -> i32;
    static kSecRandomDefault: Ptr;
    static kSecClass: Ptr;
    static kSecClassGenericPassword: Ptr;
    static kSecAttrService: Ptr;
    static kSecAttrAccount: Ptr;
    static kSecReturnData: Ptr;
    static kSecMatchLimit: Ptr;
    static kSecMatchLimitOne: Ptr;
    static kSecValueData: Ptr;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ptr);
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataCreate(allocator: Ptr, bytes: *const u8, length: isize) -> Ptr;
    fn CFDataGetLength(data: Ptr) -> isize;
    fn CFDataGetBytePtr(data: Ptr) -> *const u8;
    fn CFStringCreateWithCString(allocator: Ptr, text: *const libc::c_char, encoding: u32) -> Ptr;
    fn CFDictionaryCreate(
        allocator: Ptr,
        keys: *const Ptr,
        values: *const Ptr,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> Ptr;
    static kCFBooleanTrue: Ptr;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}
/// An owned Core Foundation create/copy result. Dictionary callbacks retain
/// their entries; the temporary keys/values may be released after construction.
struct Owned(Ptr);
impl Owned {
    fn new(value: Ptr) -> Result<Self> {
        if value.is_null() {
            Err("Could not prepare native Utilities storage.".into())
        } else {
            Ok(Self(value))
        }
    }
    fn text(value: &str) -> Result<Self> {
        let value = CString::new(value).map_err(|_| "Invalid native storage identity.")?;
        Self::new(unsafe {
            CFStringCreateWithCString(std::ptr::null_mut(), value.as_ptr(), 0x08000100)
        })
    }
    fn data(bytes: &[u8]) -> Result<Self> {
        Self::new(unsafe {
            CFDataCreate(std::ptr::null_mut(), bytes.as_ptr(), bytes.len() as isize)
        })
    }
    fn dictionary(entries: &[(Ptr, Ptr)]) -> Result<Self> {
        let keys: Vec<_> = entries.iter().map(|(key, _)| *key).collect();
        let values: Vec<_> = entries.iter().map(|(_, value)| *value).collect();
        Self::new(unsafe {
            CFDictionaryCreate(
                std::ptr::null_mut(),
                keys.as_ptr(),
                values.as_ptr(),
                entries.len() as isize,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
        })
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

#[derive(Clone, Copy)]
struct Security {
    read: unsafe extern "C" fn(Ptr, *mut Ptr) -> i32,
    update: unsafe extern "C" fn(Ptr, Ptr) -> i32,
    add: unsafe extern "C" fn(Ptr, *mut Ptr) -> i32,
    random: unsafe extern "C" fn(Ptr, usize, *mut c_void) -> i32,
}
impl Security {
    fn production() -> Self {
        Self {
            read: SecItemCopyMatching,
            update: SecItemUpdate,
            add: SecItemAdd,
            random: SecRandomCopyBytes,
        }
    }
    fn query(extra: &[(Ptr, Ptr)]) -> Result<Owned> {
        let service = Owned::text(KEYCHAIN_SERVICE)?;
        let account = Owned::text(KEYCHAIN_ACCOUNT)?;
        let mut entries = vec![
            unsafe { (kSecClass, kSecClassGenericPassword) },
            unsafe { (kSecAttrService, service.0) },
            unsafe { (kSecAttrAccount, account.0) },
        ];
        entries.extend_from_slice(extra);
        Owned::dictionary(&entries)
    }
    fn read(&self) -> Result<Option<Vec<u8>>> {
        let query = Self::query(&[unsafe { (kSecReturnData, kCFBooleanTrue) }, unsafe {
            (kSecMatchLimit, kSecMatchLimitOne)
        }])?;
        let mut result = std::ptr::null_mut();
        let status = unsafe { (self.read)(query.0, &mut result) };
        // Even a substituted/error SDK call may supply a create-owned object.
        let result = (!result.is_null()).then(|| Owned(result));
        if status == NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(format!(
                "Could not read password history from Keychain ({status})."
            ));
        }
        let data = result.ok_or("Could not read password history from Keychain (0).")?;
        if unsafe { CFGetTypeID(data.0) != CFDataGetTypeID() } {
            return Err(
                "Password history has an invalid format. The Keychain entry was preserved.".into(),
            );
        }
        let length = unsafe { CFDataGetLength(data.0) };
        if length < 0 || length as usize > MAX_BYTES {
            return Err("Password history is too large to load.".into());
        }
        if length == 0 {
            return Ok(Some(vec![]));
        }
        let pointer = unsafe { CFDataGetBytePtr(data.0) };
        if pointer.is_null() {
            return Err("Could not read password history from Keychain (0).".into());
        }
        // The retained immutable CFData owns these bytes until after this copy.
        Ok(Some(
            unsafe { std::slice::from_raw_parts(pointer, length as usize) }.to_vec(),
        ))
    }
    #[cfg(test)]
    fn write(&self, bytes: &[u8]) -> Result<()> {
        self.write_guarded(bytes, || Ok(()))
    }
    fn write_guarded(&self, bytes: &[u8], mut live: impl FnMut() -> Result<()>) -> Result<()> {
        if bytes.len() > MAX_BYTES {
            return Err("Password history is too large to save.".into());
        }
        let data = Owned::data(bytes)?;
        let query = Self::query(&[])?;
        let changes = Owned::dictionary(&[unsafe { (kSecValueData, data.0) }])?;
        live()?;
        let mut status = unsafe { (self.update)(query.0, changes.0) };
        if status == NOT_FOUND {
            let item = Self::query(&[unsafe { (kSecValueData, data.0) }])?;
            live()?;
            status = unsafe { (self.add)(item.0, std::ptr::null_mut()) };
        }
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "Could not save password history in Keychain ({status})."
            ))
        }
    }
    fn random(&self, count: usize) -> Result<Vec<u8>> {
        if !matches!(count, 4 | 24) {
            return Err("Unsupported Utilities random-data request.".into());
        }
        let mut bytes = vec![0; count];
        let status = unsafe { (self.random)(kSecRandomDefault, count, bytes.as_mut_ptr().cast()) };
        if status != 0 {
            return Err("Could not generate secure random data.".into());
        }
        Ok(bytes)
    }
}

pub struct NativeStorage {
    authority: Option<Authority>,
    _profile_lease: Option<crate::control::ProfileLease>,
    root: PathBuf,
    security: Security,
    previous: Option<Option<Vec<u8>>>,
    publication_warning: Option<String>,
}
impl NativeStorage {
    pub fn fixture(root: PathBuf) -> Result<Self> {
        let mut storage = Self::production(root)?;
        // Complete test-table resolution happens before each call, without a
        // production SDK fallback. Ordinary unrelated host fixtures need no
        // Security replacement merely to start the runtime.
        storage.security = Security {
            read: owned_read,
            update: owned_update,
            add: owned_add,
            random: owned_random,
        };
        Ok(storage)
    }
    pub fn set_authority(&mut self, authority: Option<Authority>) {
        self.authority = authority;
    }
    pub fn retain_profile(&mut self, lease: crate::control::ProfileLease) {
        self._profile_lease = Some(lease);
    }
    fn live(&self) -> Result<()> {
        self.authority.as_ref().map_or(Ok(()), Authority::check)
    }
    fn mutation(&self) -> Result<()> {
        self.authority
            .as_ref()
            .map_or(Ok(()), Authority::before_write)
    }
    /// Merely constructing the store never queries Keychain or random data.
    pub fn production(root: PathBuf) -> Result<Self> {
        private_storage::secure_directory(&root)?;
        Ok(Self {
            authority: None,
            _profile_lease: None,
            root,
            security: Security::production(),
            previous: None,
            publication_warning: None,
        })
    }
    fn profile_bytes(&self) -> Result<Option<Vec<u8>>> {
        read_owned(&self.root.join("utilities.json"))
    }
}

fn owned_security() -> Option<Security> {
    #[cfg(feature = "test-tools")]
    unsafe {
        let names = [
            c"ruycast_utility_fixture_read",
            c"ruycast_utility_fixture_update",
            c"ruycast_utility_fixture_add",
            c"ruycast_utility_fixture_random",
            c"ruycast_utility_fixture_ready",
        ];
        let symbols = names.map(|name| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()));
        if symbols.iter().any(|pointer| pointer.is_null()) {
            return None;
        }
        let ready: unsafe extern "C" fn() -> bool = std::mem::transmute(symbols[4]);
        if !ready() {
            return None;
        }
        Some(Security {
            read: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Ptr, *mut Ptr) -> i32>(
                symbols[0],
            ),
            update: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Ptr, Ptr) -> i32>(
                symbols[1],
            ),
            add: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Ptr, *mut Ptr) -> i32>(
                symbols[2],
            ),
            random: std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(Ptr, usize, *mut c_void) -> i32,
            >(symbols[3]),
        })
    }
    #[cfg(not(feature = "test-tools"))]
    {
        None
    }
}
unsafe extern "C" fn owned_read(query: Ptr, result: *mut Ptr) -> i32 {
    owned_security().map_or(-50000, |sdk| unsafe { (sdk.read)(query, result) })
}
unsafe extern "C" fn owned_update(query: Ptr, changes: Ptr) -> i32 {
    owned_security().map_or(-50000, |sdk| unsafe { (sdk.update)(query, changes) })
}
unsafe extern "C" fn owned_add(item: Ptr, result: *mut Ptr) -> i32 {
    owned_security().map_or(-50000, |sdk| unsafe { (sdk.add)(item, result) })
}
unsafe extern "C" fn owned_random(random: Ptr, count: usize, bytes: *mut c_void) -> i32 {
    owned_security().map_or(-50000, |sdk| unsafe { (sdk.random)(random, count, bytes) })
}
impl Storage for NativeStorage {
    fn read_profile(&mut self) -> Result<Option<Vec<u8>>> {
        self.live()?;
        self.previous = None;
        let bytes = self.profile_bytes()?;
        self.previous = Some(bytes.clone());
        Ok(bytes)
    }
    fn save_profile(&mut self, bytes: &[u8]) -> Result<()> {
        self.mutation()?;
        if bytes.len() > MAX_BYTES {
            return Err("Utilities history is too large to save.".into());
        }
        let expected = self
            .previous
            .take()
            .ok_or("Read Utilities history before saving.")?;
        private_storage::secure_directory(&self.root)?;
        let _lock = Lock::acquire(&self.root)?;
        if self.profile_bytes()? != expected {
            return Err("Utilities history changed. Search again before saving.".into());
        }
        let outcome = private_storage::write_outcome_guarded(
            &self.root.join("utilities.json"),
            bytes,
            expected.is_some(),
            |publish| {
                self.live()?;
                if self.profile_bytes()? != expected {
                    return Err("Utilities history changed. Search again before saving.".into());
                }
                if let Some(authority) = &self.authority {
                    authority.enter()?;
                }
                publish()
            },
        );
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Some(authority) = &self.authority {
                    let _=authority
                        .changed
                    .compare_exchange(1,0,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire);
                }
                return Err(error);
            }
        };
        if let Some(authority) = &self.authority {
            authority
                .changed
                .store(2, std::sync::atomic::Ordering::Release);
        }
        self.publication_warning = match outcome {
            private_storage::WriteOutcome::Durable(_) => None,
            private_storage::WriteOutcome::Published { error, .. } => Some(format!(
                "Utilities history saved, but durability is unconfirmed: {error}"
            )),
        };
        Ok(())
    }
    fn read_passwords(&mut self) -> Result<Option<Vec<u8>>> {
        self.live()?;
        self.security.read()
    }
    fn save_passwords(&mut self, bytes: &[u8]) -> Result<()> {
        self.mutation()?;
        if let Some(authority) = &self.authority {
            authority.enter()?;
        }
        let result = self.security.write_guarded(bytes, || self.live());
        if result.is_ok() {
            if let Some(authority) = &self.authority {
                authority
                    .changed
                    .store(2, std::sync::atomic::Ordering::Release);
            }
        }
        result
    }
    fn random(&mut self, count: usize) -> Result<Vec<u8>> {
        self.live()?;
        self.security.random(count)
    }
    fn created(&mut self) -> String {
        autoreleasepool(|_| {
            NSISO8601DateFormatter::new()
                .stringFromDate(&NSDate::date())
                .to_string()
        })
    }
    fn title_matches(&self, title: &str, query: &str) -> bool {
        autoreleasepool(|_| {
            NSString::from_str(title)
                .localizedCaseInsensitiveContainsString(&NSString::from_str(query))
        })
    }
    fn take_publication_warning(&mut self) -> Option<String> {
        self.publication_warning.take()
    }
}

fn read_owned(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(
                "Utilities history could not be read. Keep the file and restore a valid copy."
                    .into(),
            )
        }
    };
    let metadata = file
        .metadata()
        .map_err(|_| "Could not inspect Utilities history.")?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::getuid() } || metadata.nlink() != 1 {
        return Err("Utilities storage must be an owned regular file.".into());
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err("Utilities history is too large to load.".into());
    }
    let mut bytes = vec![];
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read Utilities history.")?;
    if bytes.len() > MAX_BYTES {
        return Err("Utilities history is too large to load.".into());
    }
    Ok(Some(bytes))
}
struct Lock(File);
impl Lock {
    fn acquire(root: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(root.join("utilities.lock"))
            .map_err(|_| "Could not lock Utilities history.")?;
        let info = file
            .metadata()
            .map_err(|_| "Could not inspect Utilities lock.")?;
        if !info.is_file() || info.uid() != unsafe { libc::getuid() } || info.nlink() != 1 {
            return Err("Invalid Utilities history lock.".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Utilities history is being saved. Try again.".into());
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utilities::{Effect, Utilities};
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicU64, Ordering},
    };
    extern "C" {
        fn CFDictionaryGetValue(dictionary: Ptr, key: Ptr) -> Ptr;
        fn CFEqual(left: Ptr, right: Ptr) -> u8;
    }
    #[derive(Default)]
    struct Fixture {
        data: Option<Vec<u8>>,
        error: i32,
        wrong_type: bool,
        missing_result: bool,
        random_error: bool,
        invalid_query: bool,
        events: Vec<&'static str>,
    }
    thread_local! { static SDK: RefCell<Fixture> = RefCell::new(Fixture::default()); }
    fn query_valid(query: Ptr) -> bool {
        let service = Owned::text(KEYCHAIN_SERVICE).unwrap();
        let account = Owned::text(KEYCHAIN_ACCOUNT).unwrap();
        unsafe {
            CFEqual(
                CFDictionaryGetValue(query, kSecClass),
                kSecClassGenericPassword,
            ) != 0
                && CFEqual(CFDictionaryGetValue(query, kSecAttrService), service.0) != 0
                && CFEqual(CFDictionaryGetValue(query, kSecAttrAccount), account.0) != 0
        }
    }
    unsafe extern "C" fn read(query: Ptr, output: *mut Ptr) -> i32 {
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.events.push("read");
            sdk.invalid_query |= !query_valid(query)
                || unsafe {
                    CFDictionaryGetValue(query, kSecReturnData) != kCFBooleanTrue
                        || CFDictionaryGetValue(query, kSecMatchLimit) != kSecMatchLimitOne
                };
            if sdk.error != 0 {
                return sdk.error;
            }
            if sdk.missing_result {
                return 0;
            }
            let object = if sdk.wrong_type {
                Owned::text("owned wrong result")
            } else {
                let Some(bytes) = &sdk.data else {
                    return NOT_FOUND;
                };
                Owned::data(bytes)
            };
            let Ok(object) = object else {
                return -1;
            };
            unsafe {
                *output = object.0;
            }
            std::mem::forget(object); // The Copy API transfers ownership.
            0
        })
    }
    fn capture_data(dictionary: Ptr) -> Option<Vec<u8>> {
        let object = unsafe { CFDictionaryGetValue(dictionary, kSecValueData) };
        if object.is_null() || unsafe { CFGetTypeID(object) != CFDataGetTypeID() } {
            return None;
        }
        let length = unsafe { CFDataGetLength(object) };
        if length < 0 || length as usize > MAX_BYTES {
            return None;
        }
        if length == 0 {
            return Some(vec![]);
        }
        Some(
            unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(object), length as usize) }
                .to_vec(),
        )
    }
    unsafe extern "C" fn update(query: Ptr, changes: Ptr) -> i32 {
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.events.push("update");
            sdk.invalid_query |= !query_valid(query);
            if sdk.error != 0 {
                return sdk.error;
            }
            if sdk.data.is_none() {
                return NOT_FOUND;
            }
            let Some(bytes) = capture_data(changes) else {
                return -1;
            };
            sdk.data = Some(bytes);
            0
        })
    }
    unsafe extern "C" fn add(query: Ptr, _: *mut Ptr) -> i32 {
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.events.push("add");
            sdk.invalid_query |= !query_valid(query);
            if sdk.error != 0 {
                return sdk.error;
            }
            let Some(bytes) = capture_data(query) else {
                return -1;
            };
            sdk.data = Some(bytes);
            0
        })
    }
    unsafe extern "C" fn random(_: Ptr, count: usize, bytes: *mut c_void) -> i32 {
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.events.push("random");
            if sdk.random_error {
                return -1;
            }
            for (index, byte) in
                unsafe { std::slice::from_raw_parts_mut(bytes.cast::<u8>(), count) }
                    .iter_mut()
                    .enumerate()
            {
                *byte = index as u8;
            }
            0
        })
    }
    fn security() -> Security {
        SDK.with(|sdk| *sdk.borrow_mut() = Fixture::default());
        Security {
            read,
            update,
            add,
            random,
        }
    }
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/utility-storage-fixtures")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            assert!(!root.exists());
            private_storage::secure_directory(&root).unwrap();
            Self(root)
        }
        fn storage(&self) -> NativeStorage {
            NativeStorage {
                authority: None,
                _profile_lease: None,
                root: self.0.clone(),
                security: security(),
                previous: None,
                publication_warning: None,
            }
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn native_security_query_identity_update_add_and_secure_rng_without_real_calls() {
        let security = security();
        assert!(security.read().unwrap().is_none());
        security.write(b"first owned history").unwrap();
        assert_eq!(
            security.read().unwrap().as_deref(),
            Some(b"first owned history".as_slice())
        );
        security.write(b"second owned history").unwrap();
        assert_eq!(security.random(24).unwrap(), (0..24).collect::<Vec<_>>());
        SDK.with(|sdk| {
            let sdk = sdk.borrow();
            assert!(!sdk.invalid_query);
            assert_eq!(
                sdk.events,
                ["read", "update", "add", "read", "update", "random"]
            );
        });
    }
    #[test]
    fn native_security_invalid_results_and_access_failures_preserve_history() {
        let security = security();
        SDK.with(|sdk| sdk.borrow_mut().wrong_type = true);
        assert!(security.read().unwrap_err().contains("preserved"));
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.wrong_type = false;
            sdk.missing_result = true;
        });
        assert!(security.read().is_err());
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            sdk.missing_result = false;
            sdk.data = Some(b"owned unchanged".to_vec());
            sdk.error = -25308;
        });
        assert!(security.read().unwrap_err().contains("-25308"));
        assert!(security
            .write(b"replacement")
            .unwrap_err()
            .contains("-25308"));
        SDK.with(|sdk| {
            let mut sdk = sdk.borrow_mut();
            assert_eq!(sdk.data.as_deref(), Some(b"owned unchanged".as_slice()));
            sdk.random_error = true;
        });
        assert!(security.random(24).is_err());
    }
    #[test]
    fn real_profile_reopen_private_mode_external_edit_and_no_password_on_disk() {
        let directory = Directory::new();
        let mut utilities = Utilities::new(directory.storage());
        utilities
            .execute("utility-set-text", Some("owned saved text"))
            .unwrap();
        let file = directory.0.join("utilities.json");
        assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
        let mut reopened = Utilities::new(directory.storage());
        assert!(
            matches!(reopened.execute("utility-paste-text", None).unwrap(), Effect::Paste(text) if text == "owned saved text")
        );
        let Effect::Copy { text, secret: true } =
            reopened.execute("utility-password", None).unwrap()
        else {
            panic!("missing concealed effect")
        };
        assert!(!String::from_utf8(std::fs::read(&file).unwrap())
            .unwrap()
            .contains(&text));
        let mut storage = directory.storage();
        storage.read_profile().unwrap();
        std::fs::write(&file, b"external owned edit").unwrap();
        assert!(storage
            .save_profile(b"replacement")
            .unwrap_err()
            .contains("changed"));
        assert_eq!(std::fs::read(&file).unwrap(), b"external owned edit");
        assert!(!SDK.with(|sdk| sdk.borrow().invalid_query));
    }
    #[test]
    fn symlink_hardlink_fifo_and_foreign_lock_shapes_fail_without_replacement() {
        let directory = Directory::new();
        let outside = directory.0.join("owned-outside.json");
        std::fs::write(&outside, b"owned sentinel").unwrap();
        let path = directory.0.join("utilities.json");
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(directory.storage().read_profile().is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&outside, &path).unwrap();
        assert!(directory.storage().read_profile().is_err());
        std::fs::remove_file(&path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(directory.storage().read_profile().is_err());
        std::fs::remove_file(&path).unwrap();
        let mut storage = directory.storage();
        storage.read_profile().unwrap();
        std::os::unix::fs::symlink(&outside, directory.0.join("utilities.lock")).unwrap();
        assert!(storage.save_profile(b"replacement").is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read(&outside).unwrap(), b"owned sentinel");
    }
    #[test]
    fn staged_profile_publication_rechecks_cancellation_and_external_bytes() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        for external in [false, true] {
            let directory = Directory::new();
            let file = directory.0.join("utilities.json");
            std::fs::write(&file, b"old owned bytes").unwrap();
            let mut storage = directory.storage();
            storage.read_profile().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let path = file.clone();
            let live = Arc::new(move || {
                let call = count.fetch_add(1, Ordering::AcqRel);
                if call == 2 {
                    if external {
                        std::fs::write(&path, b"external owned edit").unwrap();
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            });
            let authority = Authority::read_write(live, Arc::new(|| Ok(())));
            let changed = authority.changed.clone();
            storage.set_authority(Some(authority));
            assert!(storage.save_profile(b"must not publish").is_err());
            assert_eq!(changed.load(Ordering::Acquire), 0);
            assert!(calls.load(Ordering::Acquire) >= 3);
            assert_eq!(
                std::fs::read(&file).unwrap(),
                if external {
                    b"external owned edit".as_slice()
                } else {
                    b"old owned bytes".as_slice()
                }
            );
            assert!(!std::fs::read_dir(&directory.0).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ruycast-")));
        }
    }
}

#[derive(Clone)]
pub struct Authority {
    live: crate::execution::Live,
    grant: std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>,
    pub changed: std::sync::Arc<std::sync::atomic::AtomicU8>,
}
impl Authority {
    pub fn set_grant(&mut self, grant: std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>) {
        self.grant = grant;
    }
    fn enter(&self) -> Result<()> {
        self.check()?;
        self.changed
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| "Utilities operation was cancelled or its write already started.".into())
    }
    pub fn with_live(mut self, live: crate::execution::Live) -> Self {
        self.live = live;
        self
    }
    pub fn read_write(
        live: crate::execution::Live,
        grant: std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            live,
            grant,
            changed: Default::default(),
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.changed.load(std::sync::atomic::Ordering::Acquire) != 3 && (self.live)() {
            Ok(())
        } else {
            Err("Utilities request expired or was cancelled.".into())
        }
    }
    fn before_write(&self) -> Result<()> {
        self.check()?;
        (self.grant)()?;
        self.check()
    }
}

/// 0 preparing, 1 publication/SDK call may be in flight, 2 known published,
/// 3 retired before publication. Timeout and first mutation share this CAS.
pub(crate) fn retire_publication(phase: &std::sync::atomic::AtomicU8) -> u8 {
    let _ = phase.compare_exchange(
        0,
        3,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    );
    phase.load(std::sync::atomic::Ordering::Acquire)
}
pub(crate) fn decorate_publication(response: &mut serde_json::Value, phase: u8) {
    response["utilitiesStorageOutcome"] = serde_json::json!(match phase {
        0 | 3 => "unchanged",
        2 => "published",
        _ => "unknown",
    });
    response["utilitiesReconciliationRequired"] = serde_json::json!(phase == 1);
    if response["ok"] != true && matches!(phase, 1 | 2) {
        response["actionCompleted"] = serde_json::json!(true);
    }
}
