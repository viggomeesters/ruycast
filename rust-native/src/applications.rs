//! Worker-only application metadata discovery. Never loads bundle executable
//! code or publishes a personal inventory into operational diagnostics.
use crate::control::HostEvent;
use objc2::rc::autoreleasepool;
use objc2_foundation::{
    NSArray, NSBundle, NSComparisonResult, NSDirectoryEnumerationOptions, NSFileManager, NSString,
    NSURL,
};
use rustcast_core::Command;
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    thread::JoinHandle,
};

pub struct Roots {
    paths: Vec<PathBuf>,
    confinement: Option<PathBuf>,
}
pub fn system_roots(home: &Path) -> Roots {
    let mut paths = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if home.is_absolute() {
        paths.push(home.join("Applications"));
    }
    Roots {
        paths,
        confinement: None,
    }
}
#[cfg(feature = "test-tools")]
pub fn fixture_roots(paths: &crate::profile::ProfilePaths) -> Result<Roots, String> {
    let home = paths
        .home
        .canonicalize()
        .map_err(|_| "Invalid discovery fixture home.")?;
    if home.parent().is_none()
        || !paths
            .root
            .canonicalize()
            .is_ok_and(|p| p.starts_with(&home))
        || std::fs::read(home.join(".ruycast-test-fixture"))
            .ok()
            .as_deref()
            != Some(b"own-disposable-fixture-v1")
    {
        return Err("App discovery requires an explicit disposable fixture home.".into());
    }
    let raw = std::env::var("RUYCAST_NATIVE_TEST_APP_ROOTS")
        .map_err(|_| "Missing app discovery fixture roots.")?;
    if raw.len() > 65536 {
        return Err("Invalid app discovery fixture roots.".into());
    }
    let roots: Vec<PathBuf> =
        serde_json::from_str(&raw).map_err(|_| "Invalid app discovery fixture roots.")?;
    if roots.len() > 16 {
        return Err("Invalid app discovery fixture roots.".into());
    }
    let paths = roots
        .into_iter()
        .map(|p| {
            let p = p
                .canonicalize()
                .map_err(|_| "Missing app discovery fixture root.")?;
            if !p.starts_with(&home) {
                return Err("App discovery fixture root escapes its home.".into());
            }
            Ok(p)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Roots {
        paths,
        confinement: Some(home),
    })
}
fn canonical(value: &str) -> String {
    NSString::from_str(value)
        .precomposedStringWithCanonicalMapping()
        .to_string()
}
pub fn available(configured: &[Command], app: &Command) -> bool {
    autoreleasepool(|_| {
        let id = canonical(&app.id);
        let target = canonical(&app.target);
        !configured
            .iter()
            .any(|c| canonical(&c.id) == id || (c.kind == "app" && canonical(&c.target) == target))
    })
}
pub fn disabled_target(configured: &[Command], target: &str) -> bool {
    autoreleasepool(|_| {
        let target = canonical(target);
        configured
            .iter()
            .any(|c| c.enabled == Some(false) && c.kind == "app" && canonical(&c.target) == target)
    })
}
/// Configured app targets (including disabled ones) suppress discovery. A
/// configured ID also wins over an accidental discovered-ID collision, matching
/// the resolver's saved-command priority without publishing duplicate row IDs.
pub fn merge(configured: &[Command], applications: &[Command]) -> Vec<Command> {
    autoreleasepool(|_| {
        let targets: HashSet<_> = configured
            .iter()
            .filter(|c| c.kind == "app")
            .map(|c| canonical(&c.target))
            .collect();
        let ids: HashSet<_> = configured.iter().map(|c| canonical(&c.id)).collect();
        configured
            .iter()
            .filter(|c| c.enabled != Some(false))
            .cloned()
            .chain(
                applications
                    .iter()
                    .filter(|c| {
                        !targets.contains(&canonical(&c.target)) && !ids.contains(&canonical(&c.id))
                    })
                    .cloned(),
            )
            .collect()
    })
}
fn metadata(url: &NSURL) -> Option<Command> {
    let bundle = NSBundle::bundleWithURL(url)?;
    // Reject malformed identifiers before the typed Cocoa string accessor.
    bundle
        .infoDictionary()?
        .objectForKey(&NSString::from_str("CFBundleIdentifier"))?
        .downcast::<NSString>()
        .ok()?;
    let id = bundle.bundleIdentifier()?.to_string();
    if id.trim().is_empty() {
        return None;
    }
    let title = ["CFBundleDisplayName", "CFBundleName"]
        .into_iter()
        .find_map(|key| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))?
                .downcast::<NSString>()
                .ok()
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| {
            url.URLByDeletingPathExtension()
                .and_then(|u| u.lastPathComponent())
                .map(|s| s.to_string())
                .unwrap_or_default()
        });
    Some(Command {
        id: format!("discovered.{id}"),
        title,
        kind: "app".into(),
        target: id,
        aliases: vec![],
        hotkey: None,
        input_prompt: None,
        keyword: None,
        arguments: vec![],
        enabled: None,
        timeout: None,
        application_id: None,
        snippet_output: None,
    })
}
// Only the explicit synthetic test mode uses this stricter tree policy.
// Bundle reads localized resources as well as Info.plist. Reject all symlinks,
// multiply-linked files and special nodes before invoking NSBundle, not just
// a symlink on the app directory. Cooperating fixture trees remain private;
// this is not an atomic defense against mutation by a malicious same-user peer.
fn fixture_bundle_safe(url: &NSURL, home: &Path, stop: &AtomicBool) -> bool {
    let bytes = unsafe { CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }.to_bytes();
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
    if !path.canonicalize().is_ok_and(|p| p.starts_with(home)) {
        return false;
    }
    let mut pending = vec![path];
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 10000 || stop.load(Ordering::Acquire) {
            return false;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if metadata.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return false;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    return false;
                };
                pending.push(entry.path());
                if pending.len() + visited > 10000 {
                    return false;
                }
            }
        } else if !metadata.is_file() || metadata.nlink() != 1 {
            return false;
        }
    }
    true
}
pub fn scan(roots: &Roots, stop: &AtomicBool) -> Result<Vec<Command>, String> {
    autoreleasepool(|_| {
        let manager = NSFileManager::new();
        let mut apps = HashMap::new();
        for root in &roots.paths {
            if stop.load(Ordering::Acquire) {
                return Err("App discovery cancelled.".into());
            }
            let path = CString::new(root.as_os_str().as_bytes())
                .map_err(|_| "Invalid app discovery root.")?;
            // Foundation consumes the borrowed NUL-terminated path within this call.
            let url = unsafe {
                NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                    NonNull::new(path.as_ptr().cast_mut()).unwrap(),
                    true,
                    None,
                )
            };
            // Explicit empty prefetch keys in synthetic mode avoid implicit
            // metadata prefetch before the complete bundle-tree guard runs.
            let keys = roots.confinement.as_ref().map(|_| NSArray::new());
            let Some(items) = manager
                .enumeratorAtURL_includingPropertiesForKeys_options_errorHandler(
                    &url,
                    keys.as_deref(),
                    NSDirectoryEnumerationOptions::SkipsHiddenFiles,
                    None,
                )
            else {
                continue;
            };
            loop {
                if stop.load(Ordering::Acquire) {
                    return Err("App discovery cancelled.".into());
                }
                // No autoreleased per-entry URLs/metadata accumulate over a scan.
                let Some(command) = autoreleasepool(|_| {
                    let url = items.nextObject()?;
                    let command = if url.pathExtension().is_some_and(|e| e.to_string() == "app") {
                        items.skipDescendants();
                        let within_fixture = roots
                            .confinement
                            .as_ref()
                            .is_none_or(|home| fixture_bundle_safe(&url, home, stop));
                        within_fixture.then(|| metadata(&url)).flatten()
                    } else {
                        if items.level() > 3 {
                            items.skipDescendants();
                        }
                        None
                    };
                    Some(command)
                }) else {
                    break;
                };
                if let Some(command) = command {
                    apps.insert(canonical(&command.target), command);
                }
            }
        }
        let mut commands: Vec<Command> = apps.into_values().collect();
        commands.sort_by(|a, b| {
            autoreleasepool(|_| {
                match NSString::from_str(&a.title)
                    .localizedStandardCompare(&NSString::from_str(&b.title))
                {
                    NSComparisonResult::Ascending => std::cmp::Ordering::Less,
                    NSComparisonResult::Descending => std::cmp::Ordering::Greater,
                    _ => a.id.cmp(&b.id),
                }
            })
        });
        if stop.load(Ordering::Acquire) {
            return Err("App discovery cancelled.".into());
        }
        Ok(commands)
    })
}
type Ready = (Vec<Command>, crate::app_choice::Choices);
type Snapshot = Arc<Mutex<Option<Result<Ready, String>>>>;
fn publish(
    stop: &AtomicBool,
    output: &Snapshot,
    sender: &SyncSender<HostEvent>,
    value: Result<Vec<Command>, String>,
) {
    let value = value.map(|apps| {
        let choices = crate::app_choice::Choices::new(&apps);
        (apps, choices)
    });
    let mut slot = output.lock().unwrap_or_else(|p| p.into_inner());
    if stop.load(Ordering::Acquire) {
        return;
    }
    *slot = Some(value);
    drop(slot);
    let _ = sender.try_send(HostEvent::UiWake);
}
pub struct Discovery {
    stop: Arc<AtomicBool>,
    result: Snapshot,
    thread: Option<JoinHandle<()>>,
}
impl Discovery {
    pub fn start(roots: Roots, sender: SyncSender<HostEvent>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        let thread = std::thread::Builder::new()
            .name("ruycast-app-discovery".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if cancelled.load(Ordering::Acquire) {
                            return Err("App discovery cancelled.".into());
                        }
                        #[cfg(feature = "test-tools")]
                        if let Some(home) = roots.confinement.as_ref().filter(|_| {
                            std::env::var_os("RUYCAST_NATIVE_TEST_APP_BARRIER").is_some()
                        }) {
                            std::fs::write(home.join("app-discovery-ready"), b"ready")
                                .map_err(|_| "Could not publish discovery fixture readiness.")?;
                            while !home.join("app-discovery-release").exists() {
                                if cancelled.load(Ordering::Acquire) {
                                    return Err("App discovery cancelled.".into());
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                        }
                        scan(&roots, &cancelled)
                    }))
                    .unwrap_or_else(|_| Err("App discovery failed unexpectedly.".into()));
                publish(&cancelled, &output, &sender, result);
            })
            .map_err(|_| "Could not start app discovery.".to_string())?;
        Ok(Self {
            stop,
            result,
            thread: Some(thread),
        })
    }
    pub fn take(&mut self) -> Option<Result<Vec<Command>, String>> {
        self.take_with_choices().map(|r| r.map(|(apps, _)| apps))
    }
    pub fn take_with_choices(&mut self) -> Option<Result<Ready, String>> {
        self.result.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.result.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                let _ = thread.join();
            }
            // An OS metadata read may not be interruptible. Retire its slot and
            // detach this read-only worker instead of blocking host shutdown.
            // It owns no profile writer, UI handle or effect authority. The stop
            // check prevents any result publication if that read later returns.
        }
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let id = autoreleasepool(|_| objc2_foundation::NSUUID::new().UUIDString().to_string());
            let p = std::env::temp_dir().join(format!("ruycast-apps-{id}"));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn bundle(&self, path: &str, id: &str, info: &str) -> PathBuf {
            let root = self.0.join(path);
            fs::create_dir_all(root.join("Contents")).unwrap();
            fs::write(root.join("Contents/Info.plist"),format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundlePackageType</key><string>APPL</string>{info}</dict></plist>")).unwrap();
            root
        }
        fn roots(&self) -> Roots {
            Roots {
                paths: vec![self.0.clone()],
                confinement: None,
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scan_fixture(t: &Temp) -> Vec<Command> {
        scan(&t.roots(), &AtomicBool::new(false)).unwrap()
    }
    #[test]
    fn bundle_metadata_name_fallbacks_hidden_and_nested_apps() {
        let t = Temp::new();
        t.bundle("Shown.app","dev.fixture.shown","<key>CFBundleDisplayName</key><string>Shown Display</string><key>CFBundleName</key><string>Not used</string>");
        t.bundle(
            "Name.app",
            "dev.fixture.name",
            "<key>CFBundleName</key><string>Bundle Name</string>",
        );
        t.bundle("Fallback Ω.app", "dev.fixture.fallback", "");
        t.bundle("Blank.app","dev.fixture.blank","<key>CFBundleDisplayName</key><string>  </string><key>CFBundleName</key><string>Blank fallback</string>");
        t.bundle(".Invisible.app", "dev.fixture.hidden", "");
        t.bundle(".hidden/Hidden.app", "dev.fixture.hidden-dir", "");
        t.bundle("Shown.app/Contents/Helper.app", "dev.fixture.helper", "");
        fs::create_dir(t.0.join("Invalid.app")).unwrap();
        let apps = scan_fixture(&t);
        let names: Vec<_> = apps.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Blank fallback",
                "Bundle Name",
                "Fallback Ω",
                "Shown Display"
            ]
        );
        assert!(apps.iter().all(|c| c.kind == "app"
            && c.id == format!("discovered.{}", c.target)
            && c.aliases.is_empty()
            && c.hotkey.is_none()));
    }
    #[test]
    fn foundation_depth_boundary_is_four_for_apps_and_three_for_descent() {
        let t = Temp::new();
        t.bundle("a/b/c/Allowed.app", "dev.fixture.allowed", "");
        t.bundle("a/b/c/d/TooDeep.app", "dev.fixture.deep", "");
        let apps = scan_fixture(&t);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].target, "dev.fixture.allowed");
    }
    #[test]
    fn later_roots_override_same_bundle_id_and_natural_names_sort() {
        let t = Temp::new();
        t.bundle(
            "system/First.app",
            "dev.fixture.shared",
            "<key>CFBundleName</key><string>Old</string>",
        );
        t.bundle(
            "user/Last.app",
            "dev.fixture.shared",
            "<key>CFBundleName</key><string>Item 2</string>",
        );
        t.bundle(
            "user/Ten.app",
            "dev.fixture.ten",
            "<key>CFBundleName</key><string>Item 10</string>",
        );
        let roots = Roots {
            paths: vec![t.0.join("system"), t.0.join("user")],
            confinement: None,
        };
        let apps = scan(&roots, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            apps.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["Item 2", "Item 10"]
        );
    }
    #[test]
    fn missing_roots_and_pre_cancelled_scan_are_distinct() {
        let t = Temp::new();
        let roots = Roots {
            paths: vec![t.0.join("missing")],
            confinement: None,
        };
        assert!(scan(&roots, &AtomicBool::new(false)).unwrap().is_empty());
        assert!(scan(&roots, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled"));
    }
    #[test]
    fn fixture_cannot_follow_app_symlink_outside_its_home() {
        let t = Temp::new();
        let external = Temp::new();
        let app = external.bundle("External.app", "dev.fixture.external", "");
        std::os::unix::fs::symlink(app, t.0.join("Linked.app")).unwrap();
        let roots = Roots {
            paths: vec![t.0.clone()],
            confinement: Some(t.0.canonicalize().unwrap()),
        };
        assert!(scan(&roots, &AtomicBool::new(false)).unwrap().is_empty());
    }
    #[test]
    fn configured_disabled_target_and_saved_id_win_without_duplicate_rows() {
        let t = Temp::new();
        t.bundle("One.app", "dev.fixture.one", "");
        t.bundle("Two.app", "dev.fixture.two", "");
        let apps = scan_fixture(&t);
        let mut saved = apps[0].clone();
        saved.id = "saved.one".into();
        saved.enabled = Some(false);
        let mut id_collision = apps[1].clone();
        id_collision.kind = "script".into();
        id_collision.title = "Configured wins".into();
        let catalog = merge(&[saved, id_collision.clone()], &apps);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].title, id_collision.title);
    }
    #[test]
    fn canonical_unicode_targets_and_ids_compare_like_swift_strings() {
        let mut c:Command=serde_json::from_value(serde_json::json!({"id":"saved","title":"Saved","kind":"app","target":"dev.Cafe\u{301}"})).unwrap();
        let mut a = c.clone();
        a.id = "discovered.dev.Café".into();
        a.target = "dev.Café".into();
        c.enabled = Some(false);
        assert!(merge(&[c], &[a]).is_empty());
    }
    #[test]
    fn missing_home_never_adds_a_relative_current_directory_scan() {
        assert_eq!(system_roots(Path::new("")).paths.len(), 2);
        assert_eq!(system_roots(Path::new("relative")).paths.len(), 2);
        assert_eq!(
            system_roots(Path::new("/own/home")).paths[2],
            Path::new("/own/home/Applications")
        );
    }
    #[test]
    fn stop_retires_readonly_worker_without_waiting_for_a_blocked_read() {
        use std::time::Duration;
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
        let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let result: Snapshot = Arc::new(Mutex::new(None));
        let output = result.clone();
        let thread = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            publish(&cancelled, &output, &wake_tx, Ok(vec![]));
            done_tx.send(()).unwrap();
        });
        ready_rx.recv().unwrap();
        let mut discovery = Discovery {
            stop,
            result: result.clone(),
            thread: Some(thread),
        };
        let (stopped_tx, stopped_rx) = std::sync::mpsc::sync_channel(1);
        let controller = std::thread::spawn(move || {
            discovery.stop();
            stopped_tx.send(()).unwrap();
        });
        let returned = stopped_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        // Release even on regression, so a failing test cannot strand its worker.
        release_tx.send(()).unwrap();
        controller.join().unwrap();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(returned, "stop waited for a blocked read-only worker");
        assert!(result.lock().unwrap().is_none());
        assert!(wake_rx.try_recv().is_err());
    }
}
