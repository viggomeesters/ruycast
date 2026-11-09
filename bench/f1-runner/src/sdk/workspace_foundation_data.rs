//! Immutable payloads for the single common receiver arena. These are data,
//! never proof that an arbitrary Objective-C pointer belongs to that arena.
use std::{ffi::c_void, ptr::NonNull, sync::Arc};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FoundationValue {
    String(Arc<Text>),
    Data(Arc<Data>),
    Url(Arc<Url>),
    Bool(bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Text {
    utf16: Vec<u16>,
    // Stable storage, including a terminal NUL. Invalid UTF-16 has no UTF-8
    // representation; it remains available losslessly through utf16().
    utf8: Option<Vec<u8>>,
}
impl Text {
    pub fn new(utf16: Vec<u16>) -> Self {
        let utf8 = String::from_utf16(&utf16).ok().map(|text| {
            let mut bytes = text.into_bytes();
            bytes.push(0);
            bytes
        });
        Self { utf16, utf8 }
    }
    pub fn from_utf8(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid UTF-8")?;
        Ok(Self::new(text.encode_utf16().collect()))
    }
    pub fn utf16(&self) -> &[u16] {
        &self.utf16
    }
    pub fn utf8_with_nul(&self) -> Option<&[u8]> {
        self.utf8.as_deref()
    }
    pub fn utf8_len(&self) -> Option<usize> {
        self.utf8.as_ref().map(|bytes| bytes.len() - 1)
    }
    pub fn range(&self, location: usize, length: usize) -> Result<&[u16]> {
        self.utf16
            .get(location..location.checked_add(length).ok_or("text range overflow")?)
            .ok_or_else(|| "text range outside string".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Data(Vec<u8>);
impl Data {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Url {
    string: Text,
    scheme: Option<Text>,
    host: Option<Text>,
    path: Option<Text>,
    last_component: Option<Text>,
    directory: bool,
}
impl Url {
    /// Parses only a string with a null base. No resource access or resolution.
    pub fn parse(text: &Text) -> Result<Self> {
        let source = Cf::string(text.utf16())?;
        // SAFETY: source is our owned CFString; base and allocator are null.
        let url = Cf::new(unsafe {
            CFURLCreateWithString(std::ptr::null(), source.ptr(), std::ptr::null())
        })?;
        Self::extract(&url)
    }
    /// Relative file paths would consult process CWD; this leaf refuses them.
    pub fn file_path(text: &Text, directory: bool) -> Result<Self> {
        if text.utf16().first() != Some(&(b'/' as u16)) || text.utf16().contains(&0) {
            return Err("file URL requires an absolute NUL-free owned path".into());
        }
        let source = Cf::string(text.utf16())?;
        // SAFETY: owned CFString; POSIX style=0; explicit directory bit. This
        // constructor does not inspect the file or determine its kind for us.
        let url = Cf::new(unsafe {
            CFURLCreateWithFileSystemPath(std::ptr::null(), source.ptr(), 0, directory.into())
        })?;
        Self::extract(&url)
    }
    fn extract(url: &Cf) -> Result<Self> {
        // SAFETY: all arguments are our live CFURL. GetString is borrowed only
        // while url lives; every Copy result is released by copy_text().
        unsafe {
            Ok(Self {
                string: Text::new(cf_utf16(CFURLGetString(url.ptr()))?),
                scheme: copy_text(CFURLCopyScheme(url.ptr()))?,
                host: copy_text(CFURLCopyHostName(url.ptr()))?,
                path: copy_text(CFURLCopyFileSystemPath(url.ptr(), 0))?,
                last_component: copy_text(CFURLCopyLastPathComponent(url.ptr()))?,
                directory: CFURLHasDirectoryPath(url.ptr()) != 0,
            })
        }
    }
    pub fn string(&self) -> &Text {
        &self.string
    }
    pub fn is_file(&self) -> bool {
        self.scheme.as_ref().is_some_and(|scheme| {
            scheme.utf16().len() == 4
                && scheme
                    .utf16()
                    .iter()
                    .zip(b"file")
                    .all(|(&a, &b)| u8::try_from(a).is_ok_and(|a| a.eq_ignore_ascii_case(&b)))
        })
    }
    pub fn host(&self) -> Option<&Text> {
        self.host.as_ref()
    }
    pub fn path(&self) -> Option<&Text> {
        self.path.as_ref()
    }
    pub fn last_component(&self) -> Option<&Text> {
        self.last_component.as_ref()
    }
    pub fn is_directory(&self) -> bool {
        self.directory
    }
}

type Ref = *const c_void;
use crate::sdk::accessibility::opaque::Range;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCharacters(allocator: *mut c_void, chars: *const u16, count: isize) -> *mut c_void;
    fn CFStringGetLength(value: *mut c_void) -> isize;
    fn CFStringGetCharacters(value: *mut c_void, range: Range, buffer: *mut u16);
    fn CFURLCreateWithString(allocator: Ref, string: Ref, base: Ref) -> Ref;
    fn CFURLCreateWithFileSystemPath(allocator: Ref, path: Ref, style: isize, directory: u8)
        -> Ref;
    fn CFURLGetString(url: Ref) -> Ref;
    fn CFURLCopyScheme(url: Ref) -> Ref;
    fn CFURLCopyHostName(url: Ref) -> Ref;
    fn CFURLCopyFileSystemPath(url: Ref, style: isize) -> Ref;
    fn CFURLCopyLastPathComponent(url: Ref) -> Ref;
    fn CFURLHasDirectoryPath(url: Ref) -> u8;
    fn CFRelease(value: Ref);
}
struct Cf(NonNull<c_void>);
impl Cf {
    fn new(pointer: Ref) -> Result<Self> {
        NonNull::new(pointer.cast_mut())
            .map(Self)
            .ok_or_else(|| "owned Foundation data construction failed".into())
    }
    fn string(value: &[u16]) -> Result<Self> {
        let count = isize::try_from(value.len()).map_err(|_| "CFString length overflow")?;
        // SAFETY: slice is live for this copying call, count is exact.
        Self::new(unsafe { CFStringCreateWithCharacters(std::ptr::null_mut(), value.as_ptr(), count) })
    }
    fn ptr(&self) -> Ref {
        self.0.as_ptr()
    }
}
impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: each Cf owns exactly one Create/Copy reference, never a Get.
        unsafe { CFRelease(self.ptr()) }
    }
}
unsafe fn cf_utf16(value: Ref) -> Result<Vec<u16>> {
    if value.is_null() {
        return Err("missing owned CFString".into());
    }
    // SAFETY: caller supplies a live string from our CFURL/string functions.
    let length = unsafe { CFStringGetLength(value.cast_mut()) };
    let count = usize::try_from(length).map_err(|_| "negative CFString length")?;
    let mut units = Vec::new();
    units
        .try_reserve_exact(count)
        .map_err(|_| "CFString allocation failed")?;
    units.resize(count, 0);
    // SAFETY: exact initialized output capacity, range covers the live string.
    unsafe {
        CFStringGetCharacters(
            value.cast_mut(),
            Range {
                location: 0,
                length,
            },
            units.as_mut_ptr(),
        )
    };
    Ok(units)
}
unsafe fn copy_text(value: Ref) -> Result<Option<Text>> {
    if value.is_null() {
        return Ok(None);
    }
    let owned = Cf::new(value)?;
    // SAFETY: caller supplies an owned Copy CFString, held until extraction.
    Ok(Some(Text::new(unsafe { cf_utf16(owned.ptr()) }?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: &str) -> Text {
        Text::new(value.encode_utf16().collect())
    }
    #[test]
    fn exact_text_and_binary_storage_has_no_implicit_truncation() {
        let value = Text::new(vec![65, 0, 0xd83d, 0xde80, 66]);
        assert_eq!(value.utf8_with_nul(), Some(&b"A\0\xf0\x9f\x9a\x80B\0"[..]));
        assert_eq!(value.utf8_len(), Some(7));
        assert_eq!(value.range(2, 2).unwrap(), [0xd83d, 0xde80]);
        assert!(value.range(usize::MAX, 1).is_err());
        assert!(value.range(4, 2).is_err());
        let invalid = Text::new(vec![0xd800, 0, 0xdfff]);
        assert_eq!(invalid.utf16(), [0xd800, 0, 0xdfff]);
        assert!(invalid.utf8_with_nul().is_none());
        assert!(Text::from_utf8(&[0xff]).is_err());
        assert_eq!(Data::new(vec![0, 255, 0]).bytes(), [0, 255, 0]);
    }
    #[test]
    fn owned_url_parsing_decodes_components_without_resource_access() {
        let value = Url::parse(&text("file://localhost/own%20fixture/%F0%9F%9A%80.txt")).unwrap();
        assert!(value.is_file());
        assert_eq!(value.host(), Some(&text("localhost")));
        assert_eq!(value.path(), Some(&text("/own fixture/🚀.txt")));
        assert_eq!(value.last_component(), Some(&text("🚀.txt")));
        assert!(!value.is_directory());
        assert_eq!(
            value.string(),
            &text("file://localhost/own%20fixture/%F0%9F%9A%80.txt")
        );
        let remote = Url::parse(&text("https://example.invalid/a%20b?q=own#fixture")).unwrap();
        assert!(!remote.is_file());
        assert_eq!(remote.host(), Some(&text("example.invalid")));
        assert_eq!(remote.path(), Some(&text("/a b")));
        let relative = Url::parse(&text("relative/own.txt")).unwrap();
        assert_eq!(relative.string(), &text("relative/own.txt"));
        assert!(!relative.is_file());
    }
    #[test]
    fn file_constructor_is_explicit_and_never_uses_process_cwd() {
        assert!(Url::file_path(&text("relative/own.txt"), false).is_err());
        assert!(Url::file_path(&text("/own\0fixture"), false).is_err());
        let value = Url::file_path(&text("/own fixture/🚀.txt"), false).unwrap();
        assert!(value.is_file());
        assert_eq!(value.path(), Some(&text("/own fixture/🚀.txt")));
        let directory = Url::file_path(&text("/own fixture"), true).unwrap();
        assert!(directory.is_directory());
        assert_eq!(directory.last_component(), Some(&text("own fixture")));
    }
}
