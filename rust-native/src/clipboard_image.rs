//! ImageIO decoding with bounds inspected before allocating decoded pixels.
use std::ffi::c_void;
type Ptr = *mut c_void;
#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageSourceCreateWithData(data: Ptr, options: Ptr) -> Ptr;
    fn CGImageSourceCopyPropertiesAtIndex(source: Ptr, index: usize, options: Ptr) -> Ptr;
    fn CGImageSourceCreateImageAtIndex(source: Ptr, index: usize, options: Ptr) -> Ptr;
    fn CGImageSourceCreateThumbnailAtIndex(source: Ptr, index: usize, options: Ptr) -> Ptr;
    fn CGImageDestinationCreateWithData(data: Ptr, kind: Ptr, count: usize, options: Ptr) -> Ptr;
    fn CGImageDestinationAddImage(dest: Ptr, image: Ptr, properties: Ptr);
    fn CGImageDestinationFinalize(dest: Ptr) -> bool;
    static kCGImagePropertyPixelWidth: Ptr;
    static kCGImagePropertyPixelHeight: Ptr;
    static kCGImageSourceShouldCache: Ptr;
    static kCGImageSourceCreateThumbnailFromImageAlways: Ptr;
    static kCGImageSourceThumbnailMaxPixelSize: Ptr;
    static kCGImageSourceCreateThumbnailWithTransform: Ptr;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: Ptr);
    fn CFDataCreate(alloc: Ptr, bytes: *const u8, length: isize) -> Ptr;
    fn CFDataCreateMutable(alloc: Ptr, capacity: isize) -> Ptr;
    fn CFDataGetLength(data: Ptr) -> isize;
    fn CFDataGetBytePtr(data: Ptr) -> *const u8;
    fn CFStringCreateWithBytes(
        alloc: Ptr,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ptr;
    fn CFDictionaryGetValue(dict: Ptr, key: Ptr) -> Ptr;
    fn CFNumberGetValue(number: Ptr, kind: isize, value: Ptr) -> u8;
    fn CFNumberCreate(alloc: Ptr, kind: isize, value: *const c_void) -> Ptr;
    fn CFDictionaryCreate(
        alloc: Ptr,
        keys: *const Ptr,
        values: *const Ptr,
        count: isize,
        k: *const u8,
        v: *const u8,
    ) -> Ptr;
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFNumberGetTypeID() -> usize;
    static kCFBooleanFalse: Ptr;
    static kCFBooleanTrue: Ptr;
}
struct Owned(Ptr);
impl Owned {
    fn new(p: Ptr) -> Result<Self, String> {
        if p.is_null() {
            Err("Clipboard image is invalid or unavailable.".into())
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
fn dictionary(keys: &[Ptr], values: &[Ptr]) -> Result<Owned, String> {
    unsafe {
        Owned::new(CFDictionaryCreate(
            std::ptr::null_mut(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as isize,
            std::ptr::null(),
            std::ptr::null_mut(),
        ))
    }
}
fn dimensions(source: Ptr) -> Result<(usize, usize), String> {
    unsafe {
        let props = Owned::new(CGImageSourceCopyPropertiesAtIndex(
            source,
            0,
            std::ptr::null_mut(),
        ))?;
        let mut out = [0i64; 2];
        for (i, key) in [kCGImagePropertyPixelWidth, kCGImagePropertyPixelHeight]
            .into_iter()
            .enumerate()
        {
            let v = CFDictionaryGetValue(props.0, key);
            if v.is_null()
                || CFGetTypeID(v) != CFNumberGetTypeID()
                || CFNumberGetValue(v, 4, (&mut out[i] as *mut i64).cast()) == 0
                || out[i] <= 0
            {
                return Err("Clipboard image dimensions are invalid.".into());
            }
        }
        Ok((out[0] as usize, out[1] as usize))
    }
}
// Keep genuine CF input storage alive through every ImageIO source use.
fn source(input: &[u8]) -> Result<(Owned, Owned), String> {
    let length = isize::try_from(input.len())
        .map_err(|_| "Clipboard image exceeds its storage limit.".to_owned())?;
    let bytes = unsafe { Owned::new(CFDataCreate(std::ptr::null_mut(), input.as_ptr(), length))? };
    let options = unsafe { dictionary(&[kCGImageSourceShouldCache], &[kCFBooleanFalse])? };
    let source = unsafe { Owned::new(CGImageSourceCreateWithData(bytes.0, options.0))? };
    Ok((source, bytes))
}
fn png_destination(image: Ptr, max: Option<usize>) -> Result<Vec<u8>, String> {
    // ImageIO requires genuine CFMutableData and CFString storage, rather than
    // assuming a selected Objective-C subclass has a toll-free CF layout.
    let data = unsafe { Owned::new(CFDataCreateMutable(std::ptr::null_mut(), 0))? };
    let name = b"public.png";
    let kind = unsafe {
        Owned::new(CFStringCreateWithBytes(
            std::ptr::null_mut(),
            name.as_ptr(),
            name.len() as isize,
            0x0800_0100, // kCFStringEncodingUTF8, from the installed CFString.h.
            0,
        ))?
    };
    let destination = unsafe {
        Owned::new(CGImageDestinationCreateWithData(
            data.0,
            kind.0,
            1,
            std::ptr::null_mut(),
        ))?
    };
    unsafe { CGImageDestinationAddImage(destination.0, image, std::ptr::null_mut()) };
    if !unsafe { CGImageDestinationFinalize(destination.0) } {
        return Err(if max.is_some() {
            "Clipboard image exceeds its storage limit."
        } else {
            "Image preview is unavailable."
        }
        .into());
    }
    let length = usize::try_from(unsafe { CFDataGetLength(data.0) })
        .map_err(|_| "Clipboard image is invalid or unavailable.".to_owned())?;
    if max.is_some_and(|max| length > max) {
        return Err("Clipboard image exceeds its storage limit.".into());
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    let bytes = unsafe { CFDataGetBytePtr(data.0) };
    if bytes.is_null() || length > isize::MAX as usize {
        return Err("Clipboard image is invalid or unavailable.".into());
    }
    // The genuine data owner remains live and unmodified through this copy.
    Ok(unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec())
}
pub fn png(input: &[u8], max: usize) -> Result<Vec<u8>, String> {
    if input.is_empty() || input.len() > max {
        return Err("Clipboard image exceeds its storage limit.".into());
    }
    let (source, _bytes) = source(input)?;
    let (w, h) = dimensions(source.0)?;
    if w > 8192 || h > 8192 || w > 16_000_000 / h {
        return Err("Clipboard image exceeds the supported size.".into());
    }
    unsafe {
        let image = Owned::new(CGImageSourceCreateImageAtIndex(
            source.0,
            0,
            std::ptr::null_mut(),
        ))?;
        png_destination(image.0, Some(max))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub png: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub bytes: usize,
}
pub fn preview(input: &[u8]) -> Result<Preview, String> {
    thumbnail(input, 1600, 32768, 64_000_000)
}
pub fn website_icon(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() > 1_048_576 {
        return Err("Website icon is too large.".into());
    }
    let image = thumbnail(input, 64, 8192, 8192 * 8192)?;
    if image.png.len() > 131072 {
        return Err("Website icon is too large.".into());
    }
    Ok(image.png)
}
fn thumbnail(
    input: &[u8],
    size: i64,
    max_dimension: usize,
    max_pixels: usize,
) -> Result<Preview, String> {
    let (source, _bytes) = source(input)?;
    let (width, height) = dimensions(source.0)?;
    if width > max_dimension || height > max_dimension || width > max_pixels / height {
        return Err("Image preview exceeds the supported size.".into());
    }
    unsafe {
        let number = Owned::new(CFNumberCreate(
            std::ptr::null_mut(),
            4,
            (&size as *const i64).cast(),
        ))?;
        let options = dictionary(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceThumbnailMaxPixelSize,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceShouldCache,
            ],
            &[kCFBooleanTrue, number.0, kCFBooleanTrue, kCFBooleanFalse],
        )?;
        let image = Owned::new(CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.0))?;
        Ok(Preview {
            png: png_destination(image.0, None)?,
            width,
            height,
            bytes: input.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genuine_cf_image_owners_preserve_png_preview_icon_and_output_bounds() {
        let input = include_bytes!("../../assets/app-icon.png");
        let normalized = png(input, 8 * 1024 * 1024).unwrap();
        assert!(normalized.starts_with(b"\x89PNG\r\n\x1a\n"));
        let full = preview(&normalized).unwrap();
        let (original_source, _original_bytes) = source(input).unwrap();
        assert_eq!(
            (full.width, full.height),
            dimensions(original_source.0).unwrap()
        );
        assert_eq!(full.bytes, normalized.len());
        let icon = website_icon(&normalized).unwrap();
        let (icon_source, _icon_bytes) = source(&icon).unwrap();
        let (width, height) = dimensions(icon_source.0).unwrap();
        assert!(width > 0 && height > 0 && width <= 64 && height <= 64);
        assert!(icon.len() <= 131072);
        let image = unsafe {
            Owned::new(CGImageSourceCreateImageAtIndex(
                original_source.0,
                0,
                std::ptr::null_mut(),
            ))
            .unwrap()
        };
        assert_eq!(
            png_destination(image.0, Some(1)).unwrap_err(),
            "Clipboard image exceeds its storage limit."
        );
        assert_eq!(
            png(&[], 1024).unwrap_err(),
            "Clipboard image exceeds its storage limit."
        );
        assert!(png(input, 1).is_err());
        assert!(preview(b"invalid owned image").is_err());
        assert!(website_icon(&vec![0; 1_048_577]).is_err());
    }
}
