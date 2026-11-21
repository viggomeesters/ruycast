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

    fn one_pixel_image_clip(root: &Path, id: &str) -> Clip {
        use objc2::AnyThread;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace};
        use objc2_foundation::NSDictionary;

        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), 1, 1, 8, 4, true, false,
                NSDeviceRGBColorSpace, 4, 32,
            )
        }
        .unwrap();
        let pointer = bitmap.bitmapData();
        assert!(!pointer.is_null());
        unsafe { std::slice::from_raw_parts_mut(pointer, 4).copy_from_slice(&[120, 60, 30, 255]) };
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
        Clip {
            id: id.into(),
            text: "Image".into(),
            date: crate::clipboard_history::now(),
            source: "fixture".into(),
            content_type: Some("image".into()),
            data_file: Some(asset.into()),
            file_urls: None,
        }
    }

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

    #[test]
    fn equal_metadata_different_id_replaces_preview_owner_and_releases_old_image() {
        use std::{
            os::unix::fs::DirBuilderExt,
            sync::Arc,
            time::{Duration, Instant},
        };

        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let root = parent.join(format!("clipboard-preview-owner-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());

        let first = one_pixel_image_clip(&root, "preview-first-owner");
        let mut second = first.clone();
        second.id = "preview-second-owner".into();
        assert_ne!(first.id, second.id);
        assert_eq!(first.content_type, second.content_type);
        assert_eq!(first.data_file, second.data_file);
        assert_eq!(first.text, second.text);

        fn settle(worker: &mut Worker) {
            let until = Instant::now() + Duration::from_secs(3);
            while worker.loading && Instant::now() < until {
                worker.poll();
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                !worker.loading,
                "Preview decode did not finish before its deadline"
            );
            assert!(worker.error.is_none());
            assert!(worker.image.is_some());
        }

        let mut worker = Worker::new(false).unwrap();
        worker.load(&root, Some(&first));
        settle(&mut worker);
        assert_eq!(worker.id.as_deref(), Some(first.id.as_str()));
        let previous = worker.image.as_ref().unwrap().clone();
        let previous_weak = Arc::downgrade(&previous);
        assert_eq!((previous.width, previous.height), (1, 1));

        worker.load(&root, Some(&second));
        assert_eq!(worker.id.as_deref(), Some(second.id.as_str()));
        assert!(worker.loading);
        assert!(worker.image.is_none());
        settle(&mut worker);
        let current = worker.image.as_ref().unwrap().clone();
        assert_eq!(worker.id.as_deref(), Some(second.id.as_str()));
        assert_eq!(
            (current.width, current.height),
            (previous.width, previous.height)
        );
        assert_eq!(current.bytes, previous.bytes);
        assert_eq!(current.png, previous.png);
        assert!(!Arc::ptr_eq(&current, &previous));
        drop(previous);
        assert!(previous_weak.upgrade().is_none());
    }

    #[test]
    fn failed_reopen_releases_published_preview_and_clears_on_deselect() {
        use std::{
            os::unix::fs::DirBuilderExt,
            sync::Arc,
            time::{Duration, Instant},
        };

        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let root = parent.join(format!(
            "clipboard-preview-failed-reopen-{}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());

        let clip = one_pixel_image_clip(&root, "failed-reopen-owner");
        let asset = root.join(clip.data_file.as_deref().unwrap());
        let mut worker = Worker::new(false).unwrap();
        worker.load(&root, Some(&clip));
        let until = Instant::now() + Duration::from_secs(3);
        while worker.loading && Instant::now() < until {
            worker.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            !worker.loading,
            "Initial preview decode exceeded its deadline"
        );
        assert!(worker.error.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        let previous = worker.image.as_ref().expect("Initial preview must publish");
        assert_eq!((previous.width, previous.height), (1, 1));
        let previous_weak = Arc::downgrade(previous);

        std::fs::write(&asset, b"invalid owned image").unwrap();
        worker.load(&root, Some(&clip));
        assert!(worker.loading);
        assert!(worker.image.is_none());
        assert!(worker.error.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        assert!(previous_weak.upgrade().is_none());
        let until = Instant::now() + Duration::from_secs(3);
        while worker.loading && Instant::now() < until {
            worker.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            !worker.loading,
            "Failed preview decode exceeded its deadline"
        );
        assert!(worker.image.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        assert!(worker.error.is_some(), "Decode failure must be surfaced");

        worker.load(&root, None);
        assert!(!worker.loading);
        assert!(worker.image.is_none());
        assert!(worker.id.is_none());
        assert!(worker.error.is_none());
    }

    #[test]
    fn stale_success_cannot_replace_same_id_reopened_preview() {
        use std::{
            os::unix::fs::DirBuilderExt,
            sync::Arc,
            time::{Duration, Instant},
        };

        let fixture_parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let fixture = fixture_parent.join(format!(
            "clipboard-preview-stale-success-{}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&fixture)
            .unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(fixture.clone());
        std::fs::write(
            fixture.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        let root = fixture.join("profile");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let clip = one_pixel_image_clip(&root, "same-id-reopen");
        let mut worker = Worker::new(true).unwrap();

        worker.load(&root, Some(&clip));
        let until = Instant::now() + Duration::from_secs(3);
        let old_result = loop {
            if let Some(result) = worker
                .state
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                break result;
            }
            assert!(
                Instant::now() < until,
                "Initial preview decode did not finish before its deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        let (old_generation, old_image) = match old_result {
            (generation, Ok(Some(image))) => (generation, image),
            _ => panic!("Expected the initial successful decoded preview"),
        };
        assert_eq!(old_generation, worker.generation);

        worker.load(&root, None);
        let hold = root.join("clipboard-preview-hold");
        std::fs::write(&hold, b"hold reopened decode at the test boundary").unwrap();
        worker.load(&root, Some(&clip));
        let reopened_generation = worker.generation;
        assert!(reopened_generation > old_generation);
        let until = Instant::now() + Duration::from_secs(3);
        while !root.join("clipboard-preview-entered").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            root.join("clipboard-preview-entered").exists(),
            "Reopened decode did not reach its held boundary"
        );
        assert!(worker.loading);
        assert!(worker.image.is_none());
        assert!(worker.error.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));

        *worker
            .state
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((old_generation, Ok(Some(old_image.clone()))));
        assert!(!worker.poll(), "Stale successful generation was accepted");
        assert_eq!(worker.generation, reopened_generation);
        assert!(worker.loading);
        assert!(worker.image.is_none());
        assert!(worker.error.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));

        std::fs::remove_file(hold).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while worker.loading && Instant::now() < until {
            worker.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!worker.loading, "Reopened preview exceeded its deadline");
        assert!(worker.error.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        let current = worker
            .image
            .as_ref()
            .expect("Reopened successful preview must publish")
            .clone();
        assert!(!Arc::ptr_eq(&current, &old_image));

        *worker
            .state
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((old_generation, Ok(Some(old_image))));
        assert!(
            !worker.poll(),
            "Late stale success replaced the current preview"
        );
        assert_eq!(worker.generation, reopened_generation);
        assert!(!worker.loading);
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        assert!(worker.error.is_none());
        assert!(Arc::ptr_eq(worker.image.as_ref().unwrap(), &current));

        worker.load(&root, None);
        assert!(worker.id.is_none());
        assert!(worker.image.is_none());
        assert!(worker.error.is_none());
    }

    #[test]
    fn decoded_preview_cancelled_before_poll_is_not_published_and_released() {
        use std::{
            os::unix::fs::DirBuilderExt,
            sync::Arc,
            time::{Duration, Instant},
        };

        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let root = parent.join(format!(
            "clipboard-preview-cancel-ready-{}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());

        let clip = one_pixel_image_clip(&root, "ready-before-publication");
        let mut worker = Worker::new(false).unwrap();
        worker.load(&root, Some(&clip));
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let ready = worker
                .state
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some();
            if ready {
                break;
            }
            assert!(
                Instant::now() < until,
                "Preview decode did not finish before its deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(worker.loading);
        assert!(worker.image.is_none());
        assert_eq!(worker.id.as_deref(), Some(clip.id.as_str()));
        let generation = worker.generation;
        let unpublished = {
            let result = worker
                .state
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some((result_generation, Ok(Some(image)))) = result.as_ref() else {
                panic!("Expected a completed successful preview awaiting poll");
            };
            assert_eq!(*result_generation, generation);
            assert_eq!((image.width, image.height), (1, 1));
            Arc::downgrade(image)
        };

        worker.load(&root, None);
        assert!(worker.generation > generation);
        assert!(!worker.poll());
        assert!(!worker.loading);
        assert!(worker.image.is_none());
        assert!(worker.id.is_none());
        assert!(worker.error.is_none());
        assert!(unpublished.upgrade().is_none());
    }
}
