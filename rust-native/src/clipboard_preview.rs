//! One decoder and one replaceable latest request. A cancelled same-ID reopen
//! gets a new generation; neither stale bytes nor stale metadata can publish.
use crate::clipboard_image::Preview;
use rustcast_core::Clip;
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
};
struct Request {
    generation: u64,
    root: PathBuf,
    clip: Clip,
}
type DecodeResult = Option<(u64, Result<Option<Arc<Preview>>, String>)>;
struct State {
    pending: Mutex<Option<Request>>,
    changed: Condvar,
    stopped: AtomicBool,
    generation: AtomicU64,
    result: Mutex<DecodeResult>,
}
pub struct Worker {
    state: Arc<State>,
    worker: Option<JoinHandle<()>>,
    pub generation: u64,
    pub id: Option<String>,
    pub loading: bool,
    pub image: Option<Arc<Preview>>,
    pub error: Option<String>,
}
impl Worker {
    pub fn new(fixture: bool) -> Result<Self, String> {
        #[cfg(not(feature = "test-tools"))]
        let _ = fixture;
        let state = Arc::new(State {
            pending: Mutex::default(),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            result: Mutex::default(),
        });
        let shared = state.clone();
        let worker = std::thread::Builder::new()
            .name("ruycast-clipboard-preview".into())
            .spawn(move || loop {
                let mut lock = shared.pending.lock().unwrap_or_else(|p| p.into_inner());
                while lock.is_none() && !shared.stopped.load(Ordering::Acquire) {
                    lock = shared.changed.wait(lock).unwrap_or_else(|p| p.into_inner());
                }
                if shared.stopped.load(Ordering::Acquire) {
                    break;
                }
                let request = lock.take().unwrap();
                drop(lock);
                #[cfg(feature = "test-tools")]
                if fixture
                    && request.root.parent().is_some_and(|p| {
                        std::fs::read(p.join(".ruycast-test-fixture"))
                            .ok()
                            .as_deref()
                            == Some(b"own-disposable-fixture-v1")
                    })
                    && request.root.join("clipboard-preview-hold").exists()
                {
                    let _ = std::fs::write(
                        request.root.join("clipboard-preview-entered"),
                        b"owned decode boundary",
                    );
                    while request.root.join("clipboard-preview-hold").exists()
                        && !shared.stopped.load(Ordering::Acquire)
                        && shared.generation.load(Ordering::Acquire) == request.generation
                    {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
                if shared.stopped.load(Ordering::Acquire) {
                    break;
                }
                if shared.generation.load(Ordering::Acquire) != request.generation {
                    continue;
                }
                let result = objc2::rc::autoreleasepool(|_| {
                    preview(&request.root, &request.clip).map(|v| v.map(Arc::new))
                });
                if shared.generation.load(Ordering::Acquire) == request.generation {
                    *shared.result.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some((request.generation, result));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            state,
            worker: Some(worker),
            generation: 0,
            id: None,
            loading: false,
            image: None,
            error: None,
        })
    }
    pub fn load(&mut self, root: &Path, clip: Option<&Clip>) {
        self.generation = self.state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.id = clip.map(|c| c.id.clone());
        self.image = None;
        self.error = None;
        self.loading = false;
        *self.state.result.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let mut pending = self.state.pending.lock().unwrap_or_else(|p| p.into_inner());
        *pending = None;
        if let Some(clip) =
            clip.filter(|c| matches!(c.content_type.as_deref(), Some("image" | "files")))
        {
            self.loading = true;
            *pending = Some(Request {
                generation: self.generation,
                root: root.into(),
                clip: clip.clone(),
            });
            self.state.changed.notify_one();
        }
    }
    pub fn poll(&mut self) -> bool {
        let ready = self
            .state
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some((g, result)) = ready {
            if g == self.generation {
                self.loading = false;
                match result {
                    Ok(image) => self.image = image,
                    Err(error) => self.error = Some(error),
                }
                return true;
            }
        }
        false
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        self.state.changed.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn preview(root: &Path, clip: &Clip) -> Result<Option<Preview>, String> {
    let bytes = if clip.content_type.as_deref() == Some("image") {
        // Reuse the exact asset ownership/path checks; do not prune or load metadata.
        crate::clipboard_history::preview_asset(
            root,
            clip.data_file
                .as_deref()
                .ok_or("Clipboard image is missing.")?,
        )?
    } else if clip.content_type.as_deref() == Some("files") {
        let Some(raw) = clip.file_urls.as_ref().and_then(|v| v.first()) else {
            return Ok(None);
        };
        crate::clipboard_history::validate_files(std::slice::from_ref(raw))?;
        let url =
            objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(raw))
                .ok_or("Invalid clipboard file reference.")?;
        let Some(ext) = url.pathExtension() else {
            return Ok(None);
        };
        if ![
            "png", "jpg", "jpeg", "gif", "tiff", "tif", "heic", "webp", "bmp",
        ]
        .contains(&ext.to_string().to_lowercase().as_str())
        {
            return Ok(None);
        }
        let path = url.path().ok_or("Image file is no longer available.")?;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path.to_string())
            .map_err(|_| "Image file is no longer available.")?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        let mut volume: libc::statfs = unsafe { std::mem::zeroed() };
        if !m.is_file()
            || m.len() == 0
            || m.len() > 30_000_000
            || unsafe { libc::fstatfs(file.as_raw_fd(), &mut volume) } != 0
            || volume.f_flags & 0x1000 == 0
        {
            return Err("Image file cannot be previewed.".into());
        }
        let mut bytes = vec![];
        (&mut file)
            .take(30_000_001)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 30_000_000 {
            return Err("Image file cannot be previewed.".into());
        }
        bytes
    } else {
        return Ok(None);
    };
    crate::clipboard_image::preview(&bytes).map(Some)
}

#[cfg(all(test, feature = "test-tools"))]
mod tests {
    use super::*;
    #[test]
    fn original_two_color_image_and_local_file_previews_preserve_pixels_and_refuse_bad_assets() {
        use objc2::AnyThread;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace};
        use objc2_foundation::{NSData, NSDictionary, NSURL};
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let root = parent.join(format!("rich-preview-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), 240, 140, 8, 4, true, false,
                NSDeviceRGBColorSpace, 960, 32,
            )
        }.unwrap();
        let pointer = bitmap.bitmapData();
        assert!(!pointer.is_null());
        // Bitmap owns exactly 140 rows of 960 initialized RGBA8 bytes.
        let pixels = unsafe { std::slice::from_raw_parts_mut(pointer, 140 * 960) };
        for y in 0..140 {
            for x in 0..240 {
                pixels[y * 960 + x * 4..y * 960 + x * 4 + 4].copy_from_slice(if x < 120 {
                    &[255, 128, 0, 255]
                } else {
                    &[0, 102, 255, 255]
                });
            }
        }
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .unwrap()
        .to_vec();
        let asset = "clipboard-assets/11111111-1111-4111-8111-111111111111.png";
        crate::private_storage::secure_directory(&root.join("clipboard-assets")).unwrap();
        crate::private_storage::write(&root.join(asset), &png, false).unwrap();
        let mut clip = Clip {
            id: "owned-two-color".into(),
            text: "Image".into(),
            date: crate::clipboard_history::now(),
            source: "fixture".into(),
            content_type: Some("image".into()),
            data_file: Some(asset.into()),
            file_urls: None,
        };
        let image = preview(&root, &clip).unwrap().unwrap();
        let file = root.join("owned-preview.png");
        crate::private_storage::write(&file, &png, false).unwrap();
        clip.content_type = Some("files".into());
        clip.data_file = None;
        clip.file_urls = Some(vec![NSURL::fileURLWithPath(
            &objc2_foundation::NSString::from_str(file.to_str().unwrap()),
        )
        .absoluteString()
        .unwrap()
        .to_string()]);
        let from_file = preview(&root, &clip).unwrap().unwrap();
        for decoded in [&image, &from_file] {
            assert_eq!((decoded.width, decoded.height), (240, 140));
            let decoded =
                NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&decoded.png)).unwrap();
            assert_eq!((decoded.pixelsWide(), decoded.pixelsHigh()), (240, 140));
            let left = decoded.colorAtX_y(60, 70).unwrap();
            let right = decoded.colorAtX_y(180, 70).unwrap();
            assert!(left.alphaComponent() > 0.9 && right.alphaComponent() > 0.9);
            assert!(left.redComponent() > 0.8 && left.blueComponent() < 0.2);
            assert!(right.blueComponent() > 0.8 && right.redComponent() < 0.2);
        }
        clip.content_type = Some("image".into());
        clip.data_file = Some("../outside.png".into());
        assert_eq!(
            preview(&root, &clip).unwrap_err(),
            "Unsafe clipboard asset path."
        );
        clip.data_file = Some(asset.into());
        std::fs::write(root.join(asset), b"invalid own image").unwrap();
        assert!(preview(&root, &clip).is_err());
        std::fs::remove_file(root.join(asset)).unwrap();
        assert!(preview(&root, &clip).is_err());
        clip.content_type = Some("files".into());
        std::fs::write(&file, b"invalid own image").unwrap();
        assert!(preview(&root, &clip).is_err());
        std::fs::remove_file(file).unwrap();
        assert_eq!(
            preview(&root, &clip).unwrap_err(),
            "Image file is no longer available."
        );
    }
    #[test]
    fn cancelled_same_id_reopen_cannot_publish_old_error_or_asset() {
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("clipboard-preview-{}", std::process::id()));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(parent.clone());
        let root = parent.join("profile");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            parent.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        std::fs::write(
            root.join("clipboard-preview-hold"),
            b"own bounded decode barrier",
        )
        .unwrap();
        let mut worker = Worker::new(true).unwrap();
        let mut clip = Clip {
            id: "same-id".into(),
            text: "Image".into(),
            date: crate::clipboard_history::now(),
            source: "fixture".into(),
            content_type: Some("image".into()),
            data_file: Some("clipboard-assets/11111111-1111-4111-8111-111111111111.png".into()),
            file_urls: None,
        };
        worker.load(&root, Some(&clip));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !root.join("clipboard-preview-entered").exists() && std::time::Instant::now() < until
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(root.join("clipboard-preview-entered").exists());
        worker.load(&root, None);
        clip.data_file = None;
        worker.load(&root, Some(&clip));
        std::fs::remove_file(root.join("clipboard-preview-hold")).unwrap();
        while worker.loading && std::time::Instant::now() < until {
            worker.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!worker.loading);
        assert!(worker.image.is_none());
        assert_eq!(worker.id.as_deref(), Some("same-id"));
        assert_eq!(worker.error.as_deref(), Some("Clipboard image is missing."));
        worker.load(&root, None);
        assert!(worker.error.is_none());
        assert!(worker.image.is_none());
        assert!(worker.id.is_none());
    }
}
