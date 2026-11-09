//! Native workspace primitive. Call from a bounded execution worker only.
//! The runtime adapter must pin/revalidate its command/file/origin and supply
//! before_commit (including acknowledged palette dismissal for app activation).
//! Runtime command admission supplies that authority; wall deadlines belong
//! to the caller, while cancellation and native result ownership always apply.
use block2::RcBlock;
use objc2::{
    rc::{autoreleasepool, Retained},
    AnyThread,
};
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSError, NSFileManager, NSRange, NSString, NSURL};
use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(test)]
use std::time::Duration;
use std::{
    cell::Cell,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Instant,
};

pub const CALLBACK_LIMIT: usize = 16;
static CALLBACKS: AtomicUsize = AtomicUsize::new(0);
pub fn pending_callbacks() -> usize {
    CALLBACKS.load(Ordering::Acquire)
}
struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        CALLBACKS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < CALLBACK_LIMIT).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| {
                "macOS open requests are busy. Wait for an earlier request to finish.".into()
            })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    App,
    Url,
    Path,
}
/// Worker-private prepared input. URL templates must already be rendered by
/// the shared template owner; no argv or shell interpretation occurs here.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenRequest {
    pub kind: Kind,
    pub command_id: String,
    pub title: String,
    pub target: String,
    #[serde(default)]
    pub application_id: Option<String>,
}
pub struct Outcome {
    pub response: Value,
    pub committed: bool,
}
/// Linearizes retirement against possible SDK entry. A precommit callback's
/// permission is not this boundary. Retirement winning here prevents all later
/// native open selectors, even after a blocked lookup or callback returns.
#[derive(Default)]
pub struct Dispatch {
    phase: AtomicU8, // 0 preparing, 1 may enter SDK, 2 retired before entry
}
impl Dispatch {
    pub(crate) fn is_preparing(&self) -> bool {
        self.phase.load(Ordering::Acquire) == 0
    }
    fn enter(&self) -> bool {
        self.phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(crate) fn retire(&self) -> Outcome {
        if self
            .phase
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_err_and(|n| n == 1)
        {
            unknown()
        } else {
            cancelled()
        }
    }
}
fn failure(message: impl Into<String>, committed: bool, unknown: bool) -> Outcome {
    let mut response = json!({"ok":false,"error":message.into()});
    if unknown {
        response["actionCompleted"] = json!(true);
        response["code"] = json!("open-outcome-unknown");
    }
    Outcome {
        response,
        committed,
    }
}
fn unknown() -> Outcome {
    failure(
        "The open request may have completed. Check its outcome before trying again.",
        true,
        true,
    )
}
fn expired(live: &dyn Fn() -> bool, deadline: Option<Instant>) -> bool {
    !live() || crate::deadline::expired(deadline)
}
fn cancelled() -> Outcome {
    failure("Request expired or was cancelled.", false, false)
}
fn budget(request: &OpenRequest) -> Result<(), String> {
    let mut size = 128usize;
    for s in [&request.command_id, &request.title, &request.target]
        .into_iter()
        .chain(request.application_id.as_ref())
    {
        if s.contains('\0') {
            return Err("Open request contains an invalid NUL character.".into());
        }
        size = size
            .checked_add(s.len())
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or("Open request exceeds the supported size.")?;
    }
    Ok(())
}
fn text(error: &NSError) -> String {
    let value = error.localizedDescription();
    let len = value.length();
    let n = len.min(16384);
    let mut units = vec![0u16; n];
    if n > 0 {
        unsafe {
            value.getCharacters_range(
                NonNull::new(units.as_mut_ptr()).unwrap(),
                NSRange::new(0, n),
            );
        }
    }
    let mut text = String::from_utf16_lossy(&units);
    if n < len {
        text.push('…');
    }
    text
}
enum CallbackResult {
    Success,
    Failure(String),
    Unknown,
}
enum State {
    Pending,
    Ready(CallbackResult),
    Retired,
}
struct Slot {
    state: Mutex<State>,
    wake: Condvar,
}
impl Slot {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::Pending),
            wake: Condvar::new(),
        }
    }
    fn pending(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(|p| p.into_inner()),
            State::Pending
        )
    }
    fn publish(&self, value: CallbackResult) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(*state, State::Pending) {
            *state = State::Ready(value);
            self.wake.notify_one();
        }
    }
    fn wait(
        &self,
        live: &dyn Fn() -> bool,
        deadline: impl Into<Option<Instant>>,
    ) -> CallbackResult {
        let deadline = deadline.into();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if matches!(*state, State::Ready(_)) {
                let old = std::mem::replace(&mut *state, State::Retired);
                if let State::Ready(result) = old {
                    return result;
                }
                unreachable!();
            }
            if expired(live, deadline) {
                *state = State::Retired;
                return CallbackResult::Unknown;
            }
            let wait = crate::deadline::poll(deadline);
            state = self
                .wake
                .wait_timeout(state, wait)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
struct Completion {
    slot: Arc<Slot>,
    _permit: Permit,
}
struct RetireOnDrop(Arc<Slot>);
impl Drop for RetireOnDrop {
    fn drop(&mut self) {
        *self.0.state.lock().unwrap_or_else(|p| p.into_inner()) = State::Retired;
    }
}
impl Completion {
    fn deliver(&self, app: *mut NSRunningApplication, error: *mut NSError) {
        if !self.slot.pending() {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            autoreleasepool(|_| {
                if let Some(error) = unsafe { error.as_ref() } {
                    CallbackResult::Failure(text(error))
                } else if app.is_null() {
                    CallbackResult::Unknown
                } else {
                    CallbackResult::Success
                }
            })
        }))
        .unwrap_or(CallbackResult::Unknown);
        self.slot.publish(result);
    }
}
/// Never logs URLs, paths, titles or native error contents. Outcome delivery,
/// job limits and profile attribution belong to the serialized runtime adapter.
pub fn execute(
    request: &OpenRequest,
    live: &dyn Fn() -> bool,
    deadline: Instant,
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Outcome {
    execute_authorized(request, live, deadline, before_commit, &Dispatch::default())
}
pub fn execute_authorized(
    request: &OpenRequest,
    live: &dyn Fn() -> bool,
    deadline: impl Into<Option<Instant>>,
    before_commit: impl FnOnce() -> Result<(), String>,
    dispatch: &Dispatch,
) -> Outcome {
    protected(
        request,
        live,
        deadline.into(),
        before_commit,
        None,
        dispatch,
    )
}
#[cfg(feature = "test-tools")]
pub fn execute_fixture(
    request: &OpenRequest,
    home: &Path,
    live: &dyn Fn() -> bool,
    deadline: Instant,
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Outcome {
    execute_fixture_authorized(
        request,
        home,
        live,
        deadline,
        before_commit,
        &Dispatch::default(),
    )
}
#[cfg(feature = "test-tools")]
pub fn execute_fixture_authorized(
    request: &OpenRequest,
    home: &Path,
    live: &dyn Fn() -> bool,
    deadline: impl Into<Option<Instant>>,
    before_commit: impl FnOnce() -> Result<(), String>,
    dispatch: &Dispatch,
) -> Outcome {
    if !fixture_installed(home) {
        return failure(
            "Native workspace fixture boundary is not installed.",
            false,
            false,
        );
    }
    protected(
        request,
        live,
        deadline.into(),
        before_commit,
        Some(home),
        dispatch,
    )
}
fn protected(
    request: &OpenRequest,
    live: &dyn Fn() -> bool,
    deadline: Option<Instant>,
    before_commit: impl FnOnce() -> Result<(), String>,
    fixture: Option<&Path>,
    dispatch: &Dispatch,
) -> Outcome {
    let committed = Cell::new(false);
    catch_unwind(AssertUnwindSafe(|| {
        execute_inner(
            request,
            live,
            deadline,
            before_commit,
            fixture,
            &committed,
            dispatch,
        )
    }))
    .unwrap_or_else(|_| {
        if committed.get() {
            unknown()
        } else {
            failure("The open request could not be prepared.", false, false)
        }
    })
}
#[cfg(feature = "test-tools")]
pub fn fixture_installed(home: &Path) -> bool {
    if std::fs::read(home.join(".ruycast-test-fixture"))
        .ok()
        .as_deref()
        != Some(b"own-disposable-fixture-v1")
    {
        return false;
    }
    let symbol = unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            c"ruycast_workspace_fixture_ready".as_ptr(),
        )
    };
    if symbol.is_null() {
        return false;
    }
    let ready: unsafe extern "C" fn() -> u64 = unsafe { std::mem::transmute(symbol) };
    unsafe { ready() == 0x52555943 }
}
fn execute_inner(
    request: &OpenRequest,
    live: &dyn Fn() -> bool,
    deadline: Option<Instant>,
    before_commit: impl FnOnce() -> Result<(), String>,
    fixture: Option<&Path>,
    committed: &Cell<bool>,
    dispatch: &Dispatch,
) -> Outcome {
    if let Err(error) = budget(request) {
        return failure(error, false, false);
    }
    if expired(live, deadline) {
        return cancelled();
    }
    if fixture.is_some() && request.kind == Kind::App && !request.target.starts_with("dev.fixture.")
    {
        return failure("Only a synthetic fixture app is allowed.", false, false);
    }
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let url: Retained<NSURL> = match request.kind {
            Kind::App => {
                let url = workspace
                    .URLForApplicationWithBundleIdentifier(&NSString::from_str(&request.target));
                if expired(live, deadline) {
                    return cancelled();
                }
                let Some(url) = url else {
                    return failure(
                        format!("App is not installed: {}", request.target),
                        false,
                        false,
                    );
                };
                if !url.isFileURL() {
                    return failure(
                        "macOS returned an invalid application location.",
                        false,
                        false,
                    );
                }
                url
            }
            Kind::Url => {
                let Some(url) = NSURL::initWithString_encodingInvalidCharacters(
                    NSURL::alloc(),
                    &NSString::from_str(&request.target),
                    true,
                ) else {
                    return failure("Invalid URL.", false, false);
                };
                url
            }
            Kind::Path => {
                let path = NSString::from_str(&request.target).stringByExpandingTildeInPath();
                if let Some(home) = fixture {
                    let p = PathBuf::from(path.to_string());
                    if !p.is_absolute()
                        || !p.starts_with(home)
                        || p.components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                    {
                        return failure("Fixture path escapes its prepared home.", false, false);
                    }
                    if !fixture_path_allowed(&p, home) {
                        return failure("Fixture path escapes its prepared home.", false, false);
                    }
                }
                if !NSFileManager::new().fileExistsAtPath(&path) {
                    return failure(format!("Path does not exist: {path}"), false, false);
                }
                NSURL::fileURLWithPath(&path)
            }
        };
        // The installed fixture shim never calls a real opening implementation.
        // Restrict even shim inputs before any open selector for defense in depth.
        if let Some(home) = fixture {
            if !fixture_url_allowed(&url, home) {
                return failure("Only a synthetic fixture URL is allowed.", false, false);
            }
        }
        let application = if request.kind != Kind::App {
            if let Some(id) = &request.application_id {
                if fixture.is_some() && !id.starts_with("dev.fixture.") {
                    return failure("Only a synthetic fixture app is allowed.", false, false);
                }
                let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(id));
                if expired(live, deadline) {
                    return cancelled();
                }
                let Some(url) = url else {
                    return failure(
                        "The selected app is not installed. Choose another app in Settings.",
                        false,
                        false,
                    );
                };
                if !url.isFileURL() {
                    return failure(
                        "macOS returned an invalid application location.",
                        false,
                        false,
                    );
                }
                if fixture.is_some_and(|home| !fixture_url_allowed(&url, home)) {
                    return failure("Only a synthetic fixture URL is allowed.", false, false);
                }
                Some(url)
            } else {
                None
            }
        } else {
            None
        };
        let asynchronous = request.kind == Kind::App || application.is_some();
        let permit = if asynchronous {
            match Permit::acquire() {
                Ok(p) => Some(p),
                Err(e) => return failure(e, false, false),
            }
        } else {
            None
        };
        if expired(live, deadline) {
            return cancelled();
        }
        if let Err(error) = before_commit() {
            return failure(error, false, false);
        }
        if expired(live, deadline) {
            return cancelled();
        }
        // From this point the OS effect may be committed; cancellation cannot
        // undo it. A synchronous OS call is not a hard real-time wait bound.
        if !asynchronous {
            if !dispatch.enter() {
                return cancelled();
            }
            committed.set(true);
            if workspace.openURL(&url) {
                return success(request, false);
            }
            return failure(
                format!("macOS could not open {}.", request.title),
                true,
                false,
            );
        }
        let slot = Arc::new(Slot::new());
        let _retire = RetireOnDrop(slot.clone());
        let completion = Completion {
            slot: slot.clone(),
            _permit: permit.unwrap(),
        };
        let block = RcBlock::new(move |app: *mut NSRunningApplication, error: *mut NSError| {
            completion.deliver(app, error)
        });
        let config = NSWorkspaceOpenConfiguration::new();
        if expired(live, deadline) {
            return cancelled();
        }
        if request.kind == Kind::App {
            if !dispatch.enter() {
                return cancelled();
            }
            committed.set(true);
            workspace.openApplicationAtURL_configuration_completionHandler(
                &url,
                &config,
                Some(&block),
            );
        } else {
            let urls = NSArray::from_slice(&[&*url]);
            if expired(live, deadline) {
                return cancelled();
            }
            if !dispatch.enter() {
                return cancelled();
            }
            committed.set(true);
            workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
                &urls,
                application.as_ref().unwrap(),
                &config,
                Some(&block),
            );
        }
        match slot.wait(live, deadline) {
            CallbackResult::Success => success(request, application.is_some()),
            CallbackResult::Failure(error) => failure(error, true, false),
            CallbackResult::Unknown => unknown(),
        }
    })
}
fn success(request: &OpenRequest, with_app: bool) -> Outcome {
    Outcome {
        response: if with_app {
            json!({"ok":true,"result":{"state":"opened"}})
        } else {
            json!({"ok":true,"result":{"command":request.command_id,"state":"opened"}})
        },
        committed: true,
    }
}
fn fixture_url_allowed(url: &NSURL, home: &Path) -> bool {
    if url.isFileURL() {
        return url.path().is_some_and(|s| {
            let path = PathBuf::from(s.to_string());
            path.is_absolute() && fixture_path_allowed(&path, home)
        });
    }
    let scheme = url.scheme().map(|s| s.to_string()).unwrap_or_default();
    // A capture completion uses an encoded Obsidian URL. Test mode still
    // requires the installed complete SDK shim before reaching this function;
    // admit only one decoded path confined to the owned disposable home.
    if scheme == "obsidian" {
        return url
            .absoluteString()
            .and_then(|text| objc2_foundation::NSURLComponents::componentsWithString(&text))
            .is_some_and(|parts| {
                if parts.host().is_none_or(|s| s.to_string() != "open")
                    || parts.fragment().is_some()
                    || parts.user().is_some()
                    || parts.password().is_some()
                    || parts.port().is_some()
                {
                    return false;
                }
                parts.queryItems().is_some_and(|items| {
                    items.len() == 1 && {
                        let item = items.objectAtIndex(0);
                        item.name().to_string() == "path"
                            && item.value().is_some_and(|value| {
                                let path = PathBuf::from(value.to_string());
                                path.is_absolute() && fixture_path_allowed(&path, home)
                            })
                    }
                })
            });
    }
    scheme == "ruycast-fixture"
        || (["http", "https"].contains(&scheme.as_str())
            && url
                .host()
                .is_some_and(|s| s.to_string().ends_with(".invalid")))
}

fn fixture_path_allowed(path: &Path, home: &Path) -> bool {
    let Ok(tail) = path.strip_prefix(home) else {
        return false;
    };
    // Check the complete lexical tail before an absent prefix can end the walk.
    // A nonexistent Fake-x must not hide later parent components in app URLs.
    if tail.components().any(|c| {
        !matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return false;
    }
    let mut current = home.to_path_buf();
    for component in tail.components() {
        match component {
            std::path::Component::Normal(name) => current.push(name),
            std::path::Component::CurDir => {}
            _ => return false,
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return false,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_capture_fixture_urls_reject_external_or_ambiguous_paths() {
        autoreleasepool(|_| {
            let home = Path::new("/owned/disposable");
            for (target, expected) in [
                (
                    "obsidian://open?path=/owned/disposable/Own%20%26%3F%23.md",
                    true,
                ),
                ("obsidian://open?path=/external/Note.md", false),
                (
                    "obsidian://open?path=/owned/disposable/new/../escape.md",
                    false,
                ),
                (
                    "obsidian://open?path=/owned/disposable/a.md&path=/owned/disposable/b.md",
                    false,
                ),
                (
                    "obsidian://open?path=/owned/disposable/a.md#fragment",
                    false,
                ),
                ("obsidian://user@open?path=/owned/disposable/a.md", false),
                ("obsidian://open:4?path=/owned/disposable/a.md", false),
                ("obsidian://other?path=/owned/disposable/a.md", false),
            ] {
                let url = NSURL::initWithString_encodingInvalidCharacters(
                    NSURL::alloc(),
                    &NSString::from_str(target),
                    true,
                )
                .unwrap();
                assert_eq!(fixture_url_allowed(&url, home), expected, "{target}");
            }
        });
    }
    #[test]
    fn retirement_before_entry_permanently_denies_dispatch() {
        let dispatch = Dispatch::default();
        assert!(!dispatch.retire().committed);
        assert!(!dispatch.enter());
        assert!(!dispatch.retire().committed);
    }
    #[test]
    fn permitted_entry_is_single_use_and_retirement_is_conservatively_unknown() {
        let dispatch = Dispatch::default();
        assert!(dispatch.enter());
        assert!(!dispatch.enter());
        let result = dispatch.retire();
        assert!(result.committed);
        assert_eq!(result.response["code"], "open-outcome-unknown");
        assert!(dispatch.retire().committed);
    }
    #[test]
    fn concurrent_entry_and_retirement_have_one_linearized_winner() {
        for _ in 0..64 {
            let dispatch = Arc::new(Dispatch::default());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let worker_dispatch = dispatch.clone();
            let worker_barrier = barrier.clone();
            let worker = std::thread::spawn(move || {
                worker_barrier.wait();
                worker_dispatch.enter()
            });
            barrier.wait();
            let retired = dispatch.retire();
            assert_eq!(worker.join().unwrap(), retired.committed);
            assert!(!dispatch.enter());
        }
    }
    #[test]
    fn first_callback_wins_even_if_delivered_before_wait() {
        let slot = Slot::new();
        slot.publish(CallbackResult::Success);
        slot.publish(CallbackResult::Failure("second".into()));
        assert!(matches!(
            slot.wait(&|| true, Instant::now() + Duration::from_secs(1)),
            CallbackResult::Success
        ));
    }
    #[test]
    fn retired_callback_cannot_replace_unknown_outcome() {
        let slot = Slot::new();
        assert!(matches!(
            slot.wait(&|| true, Instant::now()),
            CallbackResult::Unknown
        ));
        slot.publish(CallbackResult::Success);
        assert!(!slot.pending());
        assert!(matches!(*slot.state.lock().unwrap(), State::Retired));
    }
    #[test]
    fn known_ready_completion_is_not_lost_at_deadline_or_cancel() {
        let slot = Slot::new();
        slot.publish(CallbackResult::Success);
        assert!(matches!(
            slot.wait(&|| false, Instant::now()),
            CallbackResult::Success
        ));
    }
    #[test]
    fn concurrent_completion_wakes_waiter_with_exact_failure() {
        let slot = Arc::new(Slot::new());
        let output = slot.clone();
        let worker = std::thread::spawn(move || {
            output.publish(CallbackResult::Failure("exact error".into()))
        });
        assert!(
            matches!(slot.wait(&||true,Instant::now()+Duration::from_secs(1)),CallbackResult::Failure(s) if s=="exact error")
        );
        worker.join().unwrap();
    }
    #[test]
    fn callback_lifetime_reservations_are_bounded_and_released() {
        assert_eq!(pending_callbacks(), 0);
        let mut permits = vec![];
        for _ in 0..CALLBACK_LIMIT {
            permits.push(Permit::acquire().unwrap());
        }
        assert!(Permit::acquire().is_err());
        assert_eq!(pending_callbacks(), CALLBACK_LIMIT);
        permits.pop();
        assert_eq!(pending_callbacks(), CALLBACK_LIMIT - 1);
        drop(permits);
        assert_eq!(pending_callbacks(), 0);
    }
    #[test]
    fn nul_is_rejected_but_application_data_has_no_socket_packet_quota() {
        let mut request = OpenRequest {
            kind: Kind::Path,
            command_id: "test".into(),
            title: "Test".into(),
            target: "a\0b".into(),
            application_id: None,
        };
        assert!(budget(&request).is_err());
        request.target = "x".repeat(1_048_576);
        assert!(budget(&request).is_ok());
        request.target = "valid".into();
        assert!(budget(&request).is_ok());
    }
    #[test]
    fn nonexistent_component_cannot_hide_later_fixture_escape() {
        let home =
            std::env::temp_dir().join(format!("ruycast-workspace-lexical-{}", std::process::id()));
        assert!(!fixture_path_allowed(
            &home.join("missing/../../outside"),
            &home
        ));
    }
}
