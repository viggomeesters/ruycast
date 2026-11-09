//! Pure owned PNG data -> retained CGImage + complete premultiplied RGBA.
//! No file URL, NSWorkspace, NSImage, screen/display or default-context API.
//! ABI read from the installed pinned macOS SDK ImageIO/CoreGraphics headers;
//! no new Rust package/version is introduced.
use super::*;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{ffi::c_void, ptr};
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, len: isize) -> *const c_void;
    fn CFRelease(value: *const c_void);
}
#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: *const c_void, options: *const c_void) -> *const c_void;
    fn CGImageSourceGetCount(source: *const c_void) -> usize;
    fn CGImageSourceCreateImageAtIndex(
        source: *const c_void,
        index: usize,
        options: *const c_void,
    ) -> *const c_void;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: *const c_void) -> usize;
    fn CGImageGetHeight(image: *const c_void) -> usize;
    fn CGImageRelease(image: *const c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGColorSpaceRelease(space: *const c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits: usize,
        stride: usize,
        space: *const c_void,
        info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *const c_void);
    fn CGContextRelease(context: *mut c_void);
}
struct CF(*const c_void);
impl Drop for CF {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
struct Space(*const c_void);
impl Drop for Space {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CGColorSpaceRelease(self.0) }
        }
    }
}
struct Context(*mut c_void);
impl Drop for Context {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CGContextRelease(self.0) }
        }
    }
}
pub struct Image {
    raw: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Arc<[u8]>,
    lease: OwnedHandleLease,
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.raw as *const c_void) }
    }
}
impl Image {
    pub fn with_raw<T>(&self, f: impl FnOnce(*const c_void) -> T) -> Result<T> {
        self.lease.before_effect()?;
        Ok(f(self.raw as *const c_void))
    }

    pub(crate) fn icon_pixels(&self) -> Arc<IconPixels> {
        Arc::new(IconPixels {
            width: self.width,
            height: self.height,
            rgba: self.rgba.clone(),
            lease: self.lease.clone(),
        })
    }
}

/// Eagerly decoded pixels retain no ImageIO or CoreFoundation object.
pub(crate) struct IconPixels {
    pub(crate) width: usize,
    pub(crate) height: usize,
    rgba: Arc<[u8]>,
    lease: OwnedHandleLease,
}
impl IconPixels {
    pub(crate) fn tiff(&self) -> Result<Vec<u8>> {
        self.lease.before_effect()?;
        let width = u32::try_from(self.width).map_err(|_| "TIFF width out of range")?;
        let height = u32::try_from(self.height).map_err(|_| "TIFF height out of range")?;
        let bytes = width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4))
            .ok_or("TIFF byte count overflow")?;
        if bytes as usize != self.rgba.len() || bytes == 0 {
            return Err("TIFF pixel count differs".into());
        }
        // Little-endian, uncompressed, one strip, chunky RGBA with associated
        // alpha. All sizes come from the same bounded decoded image.
        const COUNT: u16 = 11;
        const BITS: u32 = 8 + 2 + COUNT as u32 * 12 + 4;
        const STRIP: u32 = BITS + 8;
        let mut output = Vec::with_capacity(STRIP as usize + self.rgba.len());
        output.extend_from_slice(b"II\x2a\0\x08\0\0\0");
        output.extend_from_slice(&COUNT.to_le_bytes());
        for (tag, kind, count, value) in [
            (256u16, 4u16, 1u32, width),
            (257, 4, 1, height),
            (258, 3, 4, BITS),
            (259, 3, 1, 1),
            (262, 3, 1, 2),
            (273, 4, 1, STRIP),
            (277, 3, 1, 4),
            (278, 4, 1, height),
            (279, 4, 1, bytes),
            (284, 3, 1, 1),
            (338, 3, 1, 1),
        ] {
            output.extend_from_slice(&tag.to_le_bytes());
            output.extend_from_slice(&kind.to_le_bytes());
            output.extend_from_slice(&count.to_le_bytes());
            output.extend_from_slice(&value.to_le_bytes());
        }
        output.extend_from_slice(&0u32.to_le_bytes());
        output.extend_from_slice(&[8, 0, 8, 0, 8, 0, 8, 0]);
        output.extend_from_slice(&self.rgba);
        self.lease.before_effect()?;
        Ok(output)
    }
}
pub fn decode_png(bytes: &[u8], lease: OwnedHandleLease) -> Result<Arc<Image>> {
    lease.before_effect()?;
    if bytes.len() < 33
        || bytes.len() > 8 * 1024 * 1024
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != 13u32.to_be_bytes()
        || &bytes[12..16] != b"IHDR"
    {
        return Err("invalid/bounded PNG fixture".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let length = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("PNG dimension overflow")?;
    if width == 0 || height == 0 || width > 2048 || height > 2048 || length > 16 * 1024 * 1024 {
        return Err("PNG dimensions exceed fixture budget".into());
    }
    let data = CF(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize) });
    if data.0.is_null() {
        return Err("owned PNG CFData allocation failed".into());
    }
    let source = CF(unsafe { CGImageSourceCreateWithData(data.0, ptr::null()) });
    if source.0.is_null() {
        return Err("PNG source decode failed".into());
    }
    if unsafe { CGImageSourceGetCount(source.0) } != 1 {
        return Err("fixture PNG must have exactly one complete image".into());
    }
    let raw = unsafe { CGImageSourceCreateImageAtIndex(source.0, 0, ptr::null()) };
    if raw.is_null() {
        return Err("PNG pixels decode failed".into());
    }
    let mut image = Image {
        raw: raw as usize,
        width,
        height,
        rgba: Arc::from([]),
        lease,
    };
    if unsafe { CGImageGetWidth(raw) } != width || unsafe { CGImageGetHeight(raw) } != height {
        return Err("decoded PNG dimensions differ".into());
    }
    let space = Space(unsafe { CGColorSpaceCreateDeviceRGB() });
    if space.0.is_null() {
        return Err("owned bitmap RGB allocation failed".into());
    }
    let mut pixels = vec![0u8; length];
    // Header constants: premultipliedLast=1, byteOrder32Big=4<<12 -> RGBA.
    let context = Context(unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space.0,
            1 | (4 << 12),
        )
    });
    if context.0.is_null() {
        return Err("owned bitmap context allocation failed".into());
    }
    image.lease.before_effect()?;
    unsafe {
        CGContextDrawImage(
            context.0,
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width as f64, height as f64),
            ),
            raw,
        )
    };
    drop(context);
    image.rgba = pixels.into();
    image.lease.before_effect()?;
    Ok(Arc::new(image))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn own_png_decode_keeps_full_rgba_and_denies_bad_input_or_dead_guard() {
        let f = super::super::tests::Fixture::new();
        let bytes = include_bytes!("../../../../assets/app-icon.png");
        let image = decode_png(bytes, f.model.lease()).unwrap();
        assert_eq!(image.rgba.len(), image.width * image.height * 4);
        assert!(image.rgba.chunks_exact(4).any(|p| p[3] != 0));
        let second = decode_png(bytes, f.model.lease()).unwrap();
        assert_eq!(&*image.rgba, &*second.rgba);
        let tiff = image.icon_pixels().tiff().unwrap();
        let data = CF(unsafe { CFDataCreate(ptr::null(), tiff.as_ptr(), tiff.len() as isize) });
        assert!(!data.0.is_null());
        let source = CF(unsafe { CGImageSourceCreateWithData(data.0, ptr::null()) });
        assert!(!source.0.is_null());
        assert_eq!(unsafe { CGImageSourceGetCount(source.0) }, 1);
        let decoded = unsafe { CGImageSourceCreateImageAtIndex(source.0, 0, ptr::null()) };
        assert!(!decoded.is_null());
        assert_eq!(unsafe { CGImageGetWidth(decoded) }, image.width);
        assert_eq!(unsafe { CGImageGetHeight(decoded) }, image.height);
        unsafe { CGImageRelease(decoded) };
        if let Some(evidence) = std::env::var_os("RUYCAST_F1_WORKSPACE_IMAGE_EVIDENCE") {
            let path = PathBuf::from(evidence);
            assert!(path.is_absolute());
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("decoded.rgba"), &image.rgba).unwrap();
            use sha2::{Digest, Sha256};
            let record = serde_json::json!({"scope":"pure own PNG data only; no original host or complete SDK acceptance","width":image.width,"height":image.height,"pngSHA256":format!("{:x}",Sha256::digest(bytes)),"rgbaSHA256":format!("{:x}",Sha256::digest(&image.rgba))});
            std::fs::write(
                path.join("decode.json"),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
        }
        assert!(decode_png(b"not PNG", f.model.lease()).is_err());
        let mut oversized = bytes.to_vec();
        oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_png(&oversized, f.model.lease()).is_err());
        f.life.0.store(false, std::sync::atomic::Ordering::Release);
        assert!(image.with_raw(|_| ()).is_err());
        assert!(decode_png(bytes, f.model.lease()).is_err());
    }
}
