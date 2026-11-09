//! Genuine CF allocations with a private, registered test-only type. This uses
//! the Apple CFRuntimeClass version0 ABI, never CFRuntimeBase field layout.
//! Register serially during SDK setup, before endpoint initializers/workers.
//! Missing symbols/identity/allocation refuse; there is no real AX fallback.
use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    ptr::NonNull,
    sync::{Mutex, OnceLock},
};
pub type Ptr = *mut c_void;
pub type Result<T> = std::result::Result<T, String>;
#[repr(C)]
struct RuntimeClass {
    version: isize,
    name: *const c_char,
    init: usize,
    copy: usize,
    finalize: Option<unsafe extern "C" fn(Ptr)>,
    equal: usize,
    hash: usize,
    formatting: usize,
    debug: usize,
    reclaim: usize,
    refcount: usize,
    alignment: usize,
}
// Descriptors are immutable static data; CF remembers their address for the
// process lifetime. No thread mutates them, and no class is unregistered.
unsafe impl Sync for RuntimeClass {}
const fn class(name: *const c_char, finalize: Option<unsafe extern "C" fn(Ptr)>) -> RuntimeClass {
    RuntimeClass {
        version: 0,
        name,
        init: 0,
        copy: 0,
        finalize,
        equal: 0,
        hash: 0,
        formatting: 0,
        debug: 0,
        reclaim: 0,
        refcount: 0,
        alignment: 0,
    }
}
static ELEMENT: RuntimeClass = class(c"RuycastF1OwnedAXElement".as_ptr(), None);
static VALUE: RuntimeClass = class(c"RuycastF1OwnedAXValue".as_ptr(), Some(finalize_value));
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn _CFRuntimeRegisterClass(class: *const RuntimeClass) -> usize;
    fn _CFRuntimeGetClassWithTypeID(id: usize) -> *const RuntimeClass;
    fn _CFRuntimeCreateInstance(
        allocator: Ptr,
        id: usize,
        extra: isize,
        category: *const u8,
    ) -> Ptr;
    pub fn CFGetTypeID(value: *const c_void) -> usize;
    pub fn CFRetain(value: *const c_void) -> *const c_void;
    pub fn CFRelease(value: *const c_void);
    fn CFStringCreateWithCharacters(allocator: Ptr, text: *const u16, count: isize) -> Ptr;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: Ptr) -> isize;
    fn CFStringGetCharacters(value: Ptr, range: Range, output: *mut u16);
    fn CFArrayCreate(
        allocator: Ptr,
        values: *const Ptr,
        count: isize,
        callbacks: *const usize,
    ) -> Ptr;
    static kCFTypeArrayCallBacks: [usize; 5];
    fn CFNumberCreate(allocator: Ptr, kind: isize, number: *const i64) -> Ptr;
    static kCFBooleanTrue: Ptr;
    static kCFBooleanFalse: Ptr;
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Range {
    pub location: isize,
    pub length: isize,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Payload {
    Point([f64; 2]),
    Size([f64; 2]),
    Range(Range),
}
impl Payload {
    pub fn kind(self) -> u32 {
        match self {
            Self::Point(_) => 1,
            Self::Size(_) => 2,
            Self::Range(_) => 4,
        }
    }
}
static TYPES: OnceLock<Result<(usize, usize)>> = OnceLock::new();
static VALUES: OnceLock<Mutex<HashMap<usize, Payload>>> = OnceLock::new();
fn values() -> &'static Mutex<HashMap<usize, Payload>> {
    VALUES.get_or_init(Mutex::default)
}
unsafe extern "C" fn finalize_value(value: Ptr) {
    // Callback cannot unwind. No allocator, effect, CF release or recursive
    // registry acquisition occurs while this lock is held.
    if let Ok(mut entries) = values().lock() {
        entries.remove(&(value as usize));
    }
}
pub fn types() -> Result<(usize, usize)> {
    TYPES
        .get_or_init(|| {
            let element = unsafe { _CFRuntimeRegisterClass(&ELEMENT) };
            let value = unsafe { _CFRuntimeRegisterClass(&VALUE) };
            if element == 0
                || value == 0
                || element == value
                || unsafe { _CFRuntimeGetClassWithTypeID(element) } != &raw const ELEMENT
                || unsafe { _CFRuntimeGetClassWithTypeID(value) } != &raw const VALUE
            {
                return Err("owned CF class registration identity failed".into());
            }
            Ok((element, value))
        })
        .clone()
}
/// No initialization, allocation, CF call or mutex acquisition.
pub fn cached_types() -> Option<(usize, usize)> {
    TYPES.get().and_then(|result| result.as_ref().ok().copied())
}
/// Owns exactly one CF retain. Only immutable owned CF data cross threads.
pub struct Token(NonNull<c_void>);
unsafe impl Send for Token {}
unsafe impl Sync for Token {}
impl Clone for Token {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.ptr());
        }
        Self(self.0)
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr());
        }
    }
}
impl Token {
    fn take(pointer: Ptr) -> Result<Self> {
        NonNull::new(pointer)
            .map(Self)
            .ok_or("owned CF allocation failed".into())
    }
    pub fn ptr(&self) -> Ptr {
        self.0.as_ptr()
    }
    pub fn retained(&self) -> Ptr {
        unsafe { CFRetain(self.ptr()).cast_mut() }
    }
    pub fn element() -> Result<Self> {
        let id = types()?.0;
        let token = Self::take(unsafe {
            _CFRuntimeCreateInstance(std::ptr::null_mut(), id, 0, std::ptr::null())
        })?;
        if unsafe { CFGetTypeID(token.ptr()) } != id {
            return Err("owned CF element type mismatch".into());
        }
        Ok(token)
    }
    pub fn value(payload: Payload) -> Result<Self> {
        let id = types()?.1;
        let token = Self::take(unsafe {
            _CFRuntimeCreateInstance(std::ptr::null_mut(), id, 0, std::ptr::null())
        })?;
        if unsafe { CFGetTypeID(token.ptr()) } != id {
            return Err("owned CF value type mismatch".into());
        }
        values()
            .lock()
            .map_err(|_| "owned CF value registry poisoned")?
            .insert(token.ptr() as usize, payload);
        Ok(token)
    }
    pub fn string(text: &str) -> Result<Self> {
        if text.encode_utf16().count() > 65_536 {
            return Err("owned CF string bound exceeded".into());
        }
        let utf16 = text.encode_utf16().collect::<Vec<_>>();
        Self::take(unsafe {
            CFStringCreateWithCharacters(std::ptr::null_mut(), utf16.as_ptr(), utf16.len() as isize)
        })
    }
    pub fn integer(value: i64) -> Result<Self> {
        Self::take(unsafe { CFNumberCreate(std::ptr::null_mut(), 4, &value) })
    }
    pub fn boolean(value: bool) -> Self {
        let p = unsafe {
            if value {
                kCFBooleanTrue
            } else {
                kCFBooleanFalse
            }
        };
        Self::take(unsafe { CFRetain(p).cast_mut() }).expect("static CF booleans are nonnull")
    }
    pub fn array(tokens: &[&Self]) -> Result<Self> {
        if tokens.len() > 8192 {
            return Err("owned CF array bound exceeded".into());
        }
        let pointers = tokens.iter().map(|t| t.ptr()).collect::<Vec<_>>();
        Self::take(unsafe {
            CFArrayCreate(
                std::ptr::null_mut(),
                pointers.as_ptr(),
                pointers.len() as isize,
                (&raw const kCFTypeArrayCallBacks).cast(),
            )
        })
    }
}
/// # Safety
/// `value` is null or a live CoreFoundation object supplied by the caller.
pub unsafe fn string(value: Ptr) -> Result<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return Err("expected CF string".into());
    }
    let n = unsafe { CFStringGetLength(value) };
    if !(0..=65_536).contains(&n) {
        return Err("CF string bound exceeded".into());
    }
    let mut text = vec![0_u16; n as usize];
    unsafe {
        CFStringGetCharacters(
            value,
            Range {
                location: 0,
                length: n,
            },
            text.as_mut_ptr(),
        );
    }
    String::from_utf16(&text).map_err(|_| "invalid UTF16 in CF string".into())
}
pub fn payload(value: Ptr) -> Result<Payload> {
    values()
        .lock()
        .map_err(|_| "owned CF value registry poisoned")?
        .get(&(value as usize))
        .copied()
        .ok_or("unknown or released owned CF value".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn released_value_metadata_does_not_accumulate_over_ten_thousand_cycles() {
        let before = values().lock().unwrap().len();
        for i in 0..10_000 {
            let v = Token::value(Payload::Range(Range {
                location: i,
                length: 0,
            }))
            .unwrap();
            assert_eq!(payload(v.ptr()).unwrap().kind(), 4);
            drop(v);
        }
        assert_eq!(values().lock().unwrap().len(), before);
    }
    #[test]
    fn genuine_cf_objects_have_own_identity_and_ordinary_retain_lifetime() {
        let (element, value) = types().unwrap();
        let token = Token::element().unwrap();
        assert_eq!(unsafe { CFGetTypeID(token.ptr()) }, element);
        let array = Token::array(&[&token]).unwrap();
        let clone = token.clone();
        drop(token);
        assert_eq!(unsafe { CFGetTypeID(clone.ptr()) }, element);
        assert_ne!(unsafe { CFGetTypeID(array.ptr()) }, element);
        let own = Token::value(Payload::Point([12.5, 3.0])).unwrap();
        let pointer = own.ptr();
        assert_eq!(unsafe { CFGetTypeID(pointer) }, value);
        let clone = own.clone();
        drop(own);
        assert_eq!(payload(pointer).unwrap(), Payload::Point([12.5, 3.0]));
        drop(clone);
        assert!(payload(pointer).is_err());
        let text = Token::string("e\u{301}\0🦀").unwrap();
        assert_eq!(unsafe { string(text.ptr()) }.unwrap(), "e\u{301}\0🦀");
    }
}
