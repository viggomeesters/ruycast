//! Direct AppKit / Accessibility / AppleEvents transport. No script helper.
use super::*;
use objc2::rc::{autoreleasepool, Retained};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace,
};
use objc2_foundation::{NSAppleEventDescriptor as D, NSAppleEventSendOptions, NSString};
use std::{
    collections::{HashMap, VecDeque},
    ffi::c_void,
    sync::Mutex,
};

type Ptr = *mut c_void;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> Ptr;
    fn AXUIElementGetPid(node: Ptr, pid: *mut i32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementSetMessagingTimeout(node: Ptr, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(node: Ptr, key: Ptr, value: *mut Ptr) -> i32;
    fn AXUIElementGetAttributeValueCount(node: Ptr, key: Ptr, count: *mut isize) -> i32;
    fn AXUIElementCopyAttributeValues(
        node: Ptr,
        key: Ptr,
        index: isize,
        count: isize,
        value: *mut Ptr,
    ) -> i32;
    fn AXUIElementSetAttributeValue(node: Ptr, key: Ptr, value: Ptr) -> i32;
    fn AXUIElementPerformAction(node: Ptr, action: Ptr) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetType(value: Ptr) -> u32;
    fn AXValueGetValue(value: Ptr, kind: u32, output: *mut c_void) -> u8;
    fn AXValueCreate(kind: u32, value: *const c_void) -> Ptr;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFGetTypeID(value: Ptr) -> usize;
    fn CFRetain(value: Ptr) -> Ptr;
    fn CFRelease(value: Ptr);
    fn CFEqual(a: Ptr, b: Ptr) -> u8;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: Ptr) -> isize;
    #[allow(clashing_extern_declarations)]
    fn CFStringGetCharacters(value: Ptr, range: Range, buffer: *mut u16);
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ptr) -> u8;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: Ptr, kind: isize, out: Ptr) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(value: Ptr) -> isize;
    fn CFArrayGetValueAtIndex(value: Ptr, index: isize) -> Ptr;
    static kCFBooleanFalse: Ptr;
}
#[repr(C)]
struct Range {
    location: isize,
    length: isize,
}
struct CF(Ptr);
// AXUIElements and immutable CF values are retained, not AppKit UI objects.
unsafe impl Send for CF {}
unsafe impl Sync for CF {}
impl Drop for CF {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
fn cf_boolean(value: Option<CF>) -> Option<bool> {
    value
        .filter(|v| unsafe { CFGetTypeID(v.0) == CFBooleanGetTypeID() })
        .map(|v| unsafe { CFBooleanGetValue(v.0) != 0 })
}
fn cf_integer(value: Option<CF>) -> Option<i64> {
    let v = value?;
    let mut n = 0i64;
    if unsafe {
        CFGetTypeID(v.0) == CFNumberGetTypeID()
            && CFNumberGetValue(v.0, 4, (&mut n as *mut i64).cast()) != 0
    } {
        Some(n)
    } else {
        None
    }
}
#[derive(Default)]
struct Handles {
    next: u64,
    nodes: HashMap<u64, Arc<CF>>,
    order: VecDeque<u64>,
}
#[derive(Default)]
pub struct Native {
    handles: Mutex<Handles>,
}
impl Native {
    pub fn new() -> Self {
        Self::default()
    }
    fn retain(&self, value: Ptr) -> Result<Element, String> {
        if value.is_null() || unsafe { CFGetTypeID(value) != AXUIElementGetTypeID() } {
            return Err(EXPIRED.into());
        }
        let mut h = self.handles.lock().unwrap_or_else(|p| p.into_inner());
        h.next = h.next.checked_add(1).ok_or(EXPIRED)?;
        let id = h.next;
        if h.order.len() >= 8192 {
            if let Some(old) = h.order.pop_front() {
                h.nodes.remove(&old);
            }
        }
        h.nodes.insert(id, Arc::new(CF(unsafe { CFRetain(value) })));
        h.order.push_back(id);
        Ok(Element(id))
    }
    fn get(&self, node: Element) -> Result<Arc<CF>, String> {
        self.handles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .nodes
            .get(&node.0)
            .cloned()
            .ok_or(EXPIRED.into())
    }
    fn timeout(&self, node: &CF, t: &Ticket) -> Result<(), String> {
        let remaining = t.remaining()?;
        if unsafe { AXUIElementSetMessagingTimeout(node.0, remaining.min(0.12) as f32) } != 0 {
            return Err(EXPIRED.into());
        }
        Ok(())
    }
    fn value(&self, node: Element, key: &str, t: &Ticket) -> Result<Option<CF>, String> {
        let node = self.get(node)?;
        self.timeout(&node, t)?;
        let key = NSString::from_str(key);
        let mut value = std::ptr::null_mut();
        let status = unsafe {
            AXUIElementCopyAttributeValue(
                node.0,
                Retained::as_ptr(&key).cast_mut().cast(),
                &mut value,
            )
        };
        // All Copy outputs are released even for a late/cancelled reply.
        let value = (!value.is_null()).then(|| CF(value));
        t.check()?;
        if status == 0 {
            Ok(value)
        } else if [-25205, -25212].contains(&status) {
            Ok(None)
        } else {
            Err("The app did not respond. Refresh to retry.".into())
        }
    }
    fn placement_value(&self, node: Element, key: &str, t: &Ticket) -> Result<CF, String> {
        let node = self.get(node)?;
        let name = NSString::from_str(key);
        for attempt in 0..4 {
            self.timeout(&node, t)?;
            let mut raw = std::ptr::null_mut();
            let status = unsafe {
                AXUIElementCopyAttributeValue(
                    node.0,
                    Retained::as_ptr(&name).cast_mut().cast(),
                    &mut raw,
                )
            };
            let value = (!raw.is_null()).then(|| CF(raw));
            t.check()?;
            if status == 0 {
                return value.ok_or_else(|| format!("Cannot read {key} from this window (0)."));
            }
            if status != -25204 {
                return Err(format!("Cannot read {key} from this window ({status})."));
            }
            if attempt == 3 {
                return Err(format!(
                    "The app did not respond while reading {key}. Try again."
                ));
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(
                [0.02f64, 0.04, 0.08][attempt].min(t.remaining()?),
            ));
        }
        unreachable!()
    }
    fn placement_write(
        &self,
        node: Element,
        key: &str,
        kind: u32,
        value: *const c_void,
        t: &Ticket,
    ) -> Result<(), String> {
        t.check()?;
        let node = self.get(node)?;
        self.timeout(&node, t)?;
        let key = NSString::from_str(key);
        let value = unsafe { AXValueCreate(kind, value) };
        if value.is_null() {
            return Err("Could not prepare the window frame.".into());
        }
        let value = CF(value);
        let status = unsafe {
            AXUIElementSetAttributeValue(node.0, Retained::as_ptr(&key).cast_mut().cast(), value.0)
        };
        t.check()?;
        if status == 0 || status == -25204 {
            Ok(())
        } else {
            Err(format!(
                "This window does not allow changing {key} ({status})."
            ))
        }
    }
}
pub fn identity(pid: i32) -> Option<Identity> {
    if pid <= 1 {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        #[cfg(feature = "test-tools")]
        {
            let fixture = libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_system_fixture_proc".as_ptr());
            if !fixture.is_null() {
                let read: unsafe extern "C" fn(i32, i32, u64, *mut c_void, i32) -> i32 =
                    std::mem::transmute(fixture);
                read(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size as i32,
                )
            } else {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size as i32,
                )
            }
        }
        #[cfg(not(feature = "test-tools"))]
        {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size as i32,
            )
        }
    };
    if read != size as i32 {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(Identity {
        pid,
        uid: info.pbi_uid,
        seconds: info.pbi_start_tvsec,
        micros: info.pbi_start_tvusec,
    })
}

/// Only main calls these. The original foreground identity is captured before
/// ordering the palette, then carried as data; no later PID-only substitution.
#[derive(Default)]
pub struct Foreground(Mutex<Option<Application>>);
impl Foreground {
    pub fn remember(&self, fixture: bool) {
        if fixture {
            return;
        }
        if let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() {
            let pid = app.processIdentifier();
            if pid == unsafe { libc::getpid() } {
                return;
            }
            let next = identity(pid)
                .filter(|id| id.uid == unsafe { libc::getuid() })
                .map(|id| Application {
                    identity: id,
                    name: app
                        .localizedName()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "App".into()),
                    bundle_identifier: app.bundleIdentifier().map(|s| s.to_string()),
                });
            *self.0.lock().unwrap_or_else(|p| p.into_inner()) = next;
        } else {
            *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }
    pub fn snapshot(&self) -> Option<Application> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    #[cfg(any(test, feature = "test-tools"))]
    pub fn fixture(&self, value: Option<Application>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = value;
    }
}
pub fn activate(id: Identity, t: &Ticket) -> Result<(), String> {
    t.check()?;
    if identity(id.pid) != Some(id) || id.uid != unsafe { libc::getuid() } {
        return Err(EXPIRED.into());
    }
    let app =
        NSRunningApplication::runningApplicationWithProcessIdentifier(id.pid).ok_or(EXPIRED)?;
    if app.isTerminated() || identity(id.pid) != Some(id) {
        return Err(EXPIRED.into());
    }
    if !app.activateWithOptions(NSApplicationActivationOptions::empty()) {
        return Err("Could not activate the original app.".into());
    }
    t.check()
}
impl Platform for Native {
    fn files(&self,query:&str,ticket:&Ticket)->Result<Vec<Command>,String>{crate::file_search::search(query,ticket)}
    fn processes(&self,ticket:&Ticket)->Result<Vec<crate::processes::Resource>,String>{crate::processes::table(&crate::processes::Native,ticket)}
    fn terminate(&self,identity:Identity,ticket:&Ticket)->Result<(),String>{crate::processes::terminate(&crate::processes::Native,identity,ticket)}
    fn frame(&self, node: Element, t: &Ticket) -> Result<rustcast_core::Frame, String> {
        let point = self.placement_value(node, "AXPosition", t)?;
        let size = self.placement_value(node, "AXSize", t)?;
        let mut p = objc2_foundation::NSPoint::new(0., 0.);
        let mut s = objc2_foundation::NSSize::new(0., 0.);
        if unsafe {
            CFGetTypeID(point.0) != AXValueGetTypeID()
                || CFGetTypeID(size.0) != AXValueGetTypeID()
                || AXValueGetType(point.0) != 1
                || AXValueGetType(size.0) != 2
                || AXValueGetValue(
                    point.0,
                    1,
                    (&mut p as *mut objc2_foundation::NSPoint).cast(),
                ) == 0
                || AXValueGetValue(size.0, 2, (&mut s as *mut objc2_foundation::NSSize).cast()) == 0
        } {
            return Err("Cannot read the window frame.".into());
        }
        t.check()?;
        let frame = rustcast_core::Frame {
            x: p.x,
            y: p.y,
            width: s.width,
            height: s.height,
        };
        if crate::window_placement::valid(frame) {
            Ok(frame)
        } else {
            Err("Cannot read the window frame.".into())
        }
    }
    fn resize(&self, node: Element, frame: rustcast_core::Frame, t: &Ticket) -> Result<(), String> {
        let size = objc2_foundation::NSSize::new(frame.width, frame.height);
        self.placement_write(
            node,
            "AXSize",
            2,
            (&size as *const objc2_foundation::NSSize).cast(),
            t,
        )
    }
    fn position(
        &self,
        node: Element,
        frame: rustcast_core::Frame,
        t: &Ticket,
    ) -> Result<(), String> {
        let point = objc2_foundation::NSPoint::new(frame.x, frame.y);
        self.placement_write(
            node,
            "AXPosition",
            1,
            (&point as *const objc2_foundation::NSPoint).cast(),
            t,
        )
    }
    fn identity(&self, pid: i32) -> Option<Identity> {
        identity(pid)
    }
    fn trusted(&self) -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }
    fn applications(&self, t: &Ticket) -> Result<Vec<Application>, String> {
        t.check()?;
        autoreleasepool(|_| {
            let mut result = vec![];
            for app in NSWorkspace::sharedWorkspace()
                .runningApplications()
                .iter()
                .take(200)
            {
                t.check()?;
                if app.activationPolicy() != NSApplicationActivationPolicy::Regular
                    || app.isTerminated()
                {
                    continue;
                }
                let Some(id) = identity(app.processIdentifier()) else {
                    continue;
                };
                result.push(Application {
                    identity: id,
                    name: app
                        .localizedName()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "App".into()),
                    bundle_identifier: app.bundleIdentifier().map(|s| s.to_string()),
                });
            }
            Ok(result)
        })
    }
    fn application(&self, pid: i32, t: &Ticket) -> Result<Element, String> {
        t.check()?;
        let value = CF(unsafe { AXUIElementCreateApplication(pid) });
        self.retain(value.0)
    }
    fn pid(&self, node: Element) -> Option<i32> {
        let value = self.get(node).ok()?;
        let mut pid = 0;
        if unsafe { AXUIElementGetPid(value.0, &mut pid) } == 0 {
            Some(pid)
        } else {
            None
        }
    }
    fn string(&self, node: Element, key: &str, t: &Ticket) -> Result<Option<String>, String> {
        let Some(value) = self.value(node, key, t)? else {
            return Ok(None);
        };
        if unsafe { CFGetTypeID(value.0) != CFStringGetTypeID() } {
            return Ok(None);
        }
        let n = unsafe { CFStringGetLength(value.0) };
        if !(0..=8192).contains(&n) {
            return Err("App metadata is too large to load.".into());
        }
        let mut units = vec![0u16; n as usize];
        unsafe {
            CFStringGetCharacters(
                value.0,
                Range {
                    location: 0,
                    length: n,
                },
                units.as_mut_ptr(),
            );
        }
        let text = String::from_utf16(&units).map_err(|_| "Could not read app metadata.")?;
        if text.len() > 8192 {
            return Err("App metadata is too large to load.".into());
        }
        Ok(Some(text))
    }
    fn boolean(&self, node: Element, key: &str, t: &Ticket) -> Result<Option<bool>, String> {
        Ok(cf_boolean(self.value(node, key, t)?))
    }
    fn integer(&self, node: Element, key: &str, t: &Ticket) -> Result<Option<i64>, String> {
        Ok(cf_integer(self.value(node, key, t)?))
    }
    fn elements(
        &self,
        node: Element,
        key: &str,
        limit: usize,
        unsupported_empty: bool,
        t: &Ticket,
    ) -> Result<Vec<Element>, String> {
        t.check()?;
        if limit == 0 {
            return Ok(vec![]);
        }
        let node = self.get(node)?;
        self.timeout(&node, t)?;
        let key = NSString::from_str(key);
        let key = Retained::as_ptr(&key).cast_mut().cast();
        let mut count = 0isize;
        let status = unsafe { AXUIElementGetAttributeValueCount(node.0, key, &mut count) };
        t.check()?;
        if unsupported_empty && status == -25205 {
            return Ok(vec![]);
        }
        if status != 0 || count < 0 {
            return Err("Could not read app items. Refresh to retry.".into());
        }
        if count == 0 {
            return Ok(vec![]);
        }
        self.timeout(&node, t)?;
        let mut raw = std::ptr::null_mut();
        let status = unsafe {
            AXUIElementCopyAttributeValues(node.0, key, 0, count.min(limit as isize), &mut raw)
        };
        let raw = (!raw.is_null()).then(|| CF(raw));
        t.check()?;
        let raw = raw.ok_or(EXPIRED)?;
        if status != 0 || unsafe { CFGetTypeID(raw.0) != CFArrayGetTypeID() } {
            return Err(EXPIRED.into());
        }
        let count = unsafe { CFArrayGetCount(raw.0) };
        if count < 0 || count as usize > limit {
            return Err("App item list is too large to load.".into());
        }
        let mut out = vec![];
        for i in 0..count {
            t.check()?;
            out.push(self.retain(unsafe { CFArrayGetValueAtIndex(raw.0, i) })?);
        }
        Ok(out)
    }
    fn element(&self, node: Element, key: &str, t: &Ticket) -> Result<Option<Element>, String> {
        self.value(node, key, t)?
            .map(|v| self.retain(v.0))
            .transpose()
    }
    fn same(&self, a: Element, b: Element) -> bool {
        match (self.get(a), self.get(b)) {
            (Ok(a), Ok(b)) => unsafe { CFEqual(a.0, b.0) != 0 },
            _ => false,
        }
    }
    fn restore(&self, node: Element, t: &Ticket) -> Result<(), String> {
        let node = self.get(node)?;
        self.timeout(&node, t)?;
        let key = NSString::from_str("AXMinimized");
        t.check()?;
        if unsafe {
            AXUIElementSetAttributeValue(
                node.0,
                Retained::as_ptr(&key).cast_mut().cast(),
                kCFBooleanFalse,
            )
        } != 0
        {
            return Err("Could not restore this window.".into());
        }
        t.check()
    }
    fn action(&self, node: Element, action: &str, t: &Ticket) -> Result<(), String> {
        let node = self.get(node)?;
        self.timeout(&node, t)?;
        let action = NSString::from_str(action);
        t.check()?;
        if unsafe { AXUIElementPerformAction(node.0, Retained::as_ptr(&action).cast_mut().cast()) }
            != 0
        {
            return Err("This result is no longer available.".into());
        }
        t.check()
    }
    fn safari(&self, t: &Ticket) -> Result<Identity, String> {
        t.check()?;
        autoreleasepool(|_| {
            for app in NSRunningApplication::runningApplicationsWithBundleIdentifier(
                &NSString::from_str("com.apple.Safari"),
            )
            .iter()
            {
                if app.isTerminated() {
                    continue;
                }
                let Some(url) = app.bundleURL() else { continue };
                let Some(path) = url.path() else { continue };
                if ![
                    "/Applications/Safari.app",
                    "/System/Applications/Safari.app",
                    "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app",
                ]
                .contains(&path.to_string().as_str())
                {
                    continue;
                }
                if let Some(id) = identity(app.processIdentifier())
                    .filter(|id| id.uid == unsafe { libc::getuid() })
                {
                    return Ok(id);
                }
            }
            Err("Open Safari to search its tabs.".into())
        })
    }
    fn safari_windows(&self, b: Identity, t: &Ticket) -> Result<Vec<i32>, String> {
        autoreleasepool(|_| {
            let ids = send(
                "getd",
                b,
                t,
                &property(
                    "ID  ",
                    &*object("cwin", &D::nullDescriptor(), "indx", &all())?,
                ),
                None,
            )?;
            if ids.descriptorType() != code("list") || ids.numberOfItems() > 64 {
                return Err("Too many Safari windows to load. The limit is 64.".into());
            }
            (1..=ids.numberOfItems())
                .map(|i| {
                    t.check()?;
                    let v = ids.descriptorAtIndex(i).ok_or(EXPIRED)?;
                    if v.descriptorType() != code("long") {
                        return Err(EXPIRED.into());
                    }
                    Ok(v.int32Value())
                })
                .collect()
        })
    }
    fn safari_tabs(&self, b: Identity, window: i32, t: &Ticket) -> Result<Vec<Tab>, String> {
        autoreleasepool(|_| {
            let reference = object("bTab", &*win(window)?, "indx", &all())?;
            let read =
                |key: &str| strings(&*send("getd", b, t, &property(key, &reference), None)?, t);
            let titles = read("pnam")?;
            let urls = read("pURL")?;
            if titles.len() != urls.len() || read("pnam")? != titles || read("pURL")? != urls {
                return Err(EXPIRED.into());
            }
            Ok(titles
                .into_iter()
                .zip(urls)
                .enumerate()
                .map(|(i, (title, url))| Tab {
                    window,
                    index: (i + 1) as i32,
                    title,
                    url,
                })
                .collect())
        })
    }
    fn safari_select(&self, b: Identity, tab: &Tab, t: &Ticket) -> Result<(), String> {
        autoreleasepool(|_| {
            send(
                "setd",
                b,
                t,
                &property("cTab", &*win(tab.window)?),
                Some(&*object(
                    "bTab",
                    &*win(tab.window)?,
                    "indx",
                    &D::descriptorWithInt32(tab.index),
                )?),
            )?;
            Ok(())
        })
    }
    fn safari_selected(&self, b: Identity, tab: &Tab, t: &Ticket) -> Result<bool, String> {
        autoreleasepool(|_| {
            let reference = property("cTab", &*win(tab.window)?);
            let index = send("getd", b, t, &property("pidx", &reference), None)?;
            let title = send("getd", b, t, &property("pnam", &reference), None)?;
            let url = send("getd", b, t, &property("pURL", &reference), None)?;
            Ok(index.descriptorType() == code("long")
                && index.int32Value() == tab.index
                && title.stringValue().map(|s| s.to_string()).as_ref() == Some(&tab.title)
                && url.stringValue().map(|s| s.to_string()).as_ref() == Some(&tab.url))
        })
    }
    fn safari_front_window(&self, b: Identity, window: i32, t: &Ticket) -> Result<(), String> {
        autoreleasepool(|_| {
            send(
                "setd",
                b,
                t,
                &property("pidx", &*win(window)?),
                Some(&D::descriptorWithInt32(1)),
            )?;
            Ok(())
        })
    }
    fn shortcuts(&self, t: &Ticket) -> Result<Vec<Command>, String> {
        apple_shortcuts::list(t)
    }
    fn safari_consent(&self, t: &Ticket) -> Result<(), String> {
        let browser = self.safari(t)?;
        t.check()?;
        let address = D::descriptorWithProcessIdentifier(browser.pid);
        #[link(name = "CoreServices", kind = "framework")]
        extern "C" {
            fn AEDeterminePermissionToAutomateTarget(
                target: *const c_void,
                class: u32,
                id: u32,
                ask: u8,
            ) -> i32;
        }
        complete_safari_consent(t, || unsafe {
            AEDeterminePermissionToAutomateTarget(
                address.aeDesc().cast(),
                code("****"),
                code("****"),
                1,
            )
        })
    }
}
/// Completion of the explicit permission request is separate from whether the
/// subsequent non-prompting tab read is authorized.
pub(crate) fn complete_safari_consent(t: &Ticket, request: impl FnOnce() -> i32) -> Result<(), String> {
    let _status = request();
    t.check()?;
    Ok(())
}
fn code(s: &str) -> u32 {
    s.bytes().fold(0, |v, b| (v << 8) | b as u32)
}
fn all() -> Retained<D> {
    let value = code("all ");
    unsafe {
        D::descriptorWithDescriptorType_bytes_length(code("abso"), (&value as *const u32).cast(), 4)
    }
    .expect("absolute descriptor")
}
fn object(class: &str, container: &D, form: &str, key: &D) -> Result<Retained<D>, String> {
    let record = D::recordDescriptor();
    record.setDescriptor_forKeyword(&D::descriptorWithTypeCode(code(class)), code("want"));
    record.setDescriptor_forKeyword(container, code("from"));
    record.setDescriptor_forKeyword(&D::descriptorWithEnumCode(code(form)), code("form"));
    record.setDescriptor_forKeyword(key, code("seld"));
    record
        .coerceToDescriptorType(code("obj "))
        .ok_or(EXPIRED.into())
}
fn property(name: &str, from: &D) -> Retained<D> {
    object("prop", from, "prop", &D::descriptorWithTypeCode(code(name)))
        .expect("fixed property descriptor")
}
fn win(id: i32) -> Result<Retained<D>, String> {
    object(
        "cwin",
        &D::nullDescriptor(),
        "ID  ",
        &D::descriptorWithInt32(id),
    )
}
fn send(
    event: &str,
    b: Identity,
    t: &Ticket,
    reference: &D,
    value: Option<&D>,
) -> Result<Retained<D>, String> {
    t.check()?;
    if identity(b.pid) != Some(b) {
        return Err(EXPIRED.into());
    }
    let event = D::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
        code("core"),
        code(event),
        Some(&D::descriptorWithProcessIdentifier(b.pid)),
        -1,
        0,
    );
    event.setParamDescriptor_forKeyword(reference, code("----"));
    if let Some(value) = value {
        event.setParamDescriptor_forKeyword(value, code("data"));
    }
    // kAEDoNotPromptForUserConsent is intentionally not a default Foundation
    // option. Passive main/dedicated search must never display Automation UI.
    let reply = event
        .sendEventWithOptions_timeout_error(
            NSAppleEventSendOptions(3 | 16 | 4096 | 0x20000),
            t.remaining()?,
        )
        .map_err(|e| {
            if [-1743, -1744].contains(&e.code()) {
                SAFARI_PERMISSION.to_owned()
            } else {
                "Could not read Safari tabs. Refresh to retry.".into()
            }
        })?;
    t.check()?;
    if identity(b.pid) != Some(b) {
        return Err(EXPIRED.into());
    }
    if reply.data().len() > 2_097_152 {
        return Err("Safari tab list is too large to load.".into());
    }
    if let Some(error) = reply.paramDescriptorForKeyword(code("errn")) {
        if error.int32Value() != 0 {
            return Err(if [-1743, -1744].contains(&error.int32Value()) {
                SAFARI_PERMISSION.into()
            } else {
                EXPIRED.into()
            });
        }
    }
    Ok(reply
        .paramDescriptorForKeyword(code("----"))
        .unwrap_or_else(D::nullDescriptor))
}
fn strings(value: &D, t: &Ticket) -> Result<Vec<String>, String> {
    if value.descriptorType() != code("list") || value.numberOfItems() > 512 {
        return Err("Too many Safari tabs to load. The limit is 512.".into());
    }
    let mut result = vec![];
    for index in 1..=value.numberOfItems() {
        t.check()?;
        let item = value.descriptorAtIndex(index).ok_or(EXPIRED)?;
        let text = item.stringValue().ok_or(EXPIRED)?.to_string();
        if text.len() > 8192 {
            return Err("Could not read Safari tab metadata. Refresh to retry.".into());
        }
        result.push(text);
    }
    Ok(result)
}

/// No AE send/permission query or external app lookup occurs in this fixture.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_cf_menu_attribute_consumers_refuse_wrong_types_and_preserve_values() {
        autoreleasepool(|_| {
            fn retained(value: Ptr) -> CF {
                assert!(!value.is_null());
                // Each input below is an actual owned Foundation/CF object.
                CF(unsafe { CFRetain(value) })
            }
            let unknown_object: Retained<objc2::runtime::AnyObject> =
                unsafe { objc2::msg_send![objc2::class!(NSNull), null] };
            let unknown = Retained::as_ptr(&unknown_object).cast_mut().cast();
            let number_object = objc2_foundation::NSNumber::numberWithLongLong(-42);
            let number = Retained::as_ptr(&number_object).cast_mut().cast();
            let false_value = unsafe { kCFBooleanFalse };
            assert_eq!(cf_boolean(None), None);
            assert_eq!(cf_integer(None), None);
            assert_eq!(cf_integer(Some(retained(false_value))), None);
            assert_eq!(cf_boolean(Some(retained(unknown))), None);
            assert_eq!(cf_boolean(Some(retained(number))), None);
            assert_eq!(cf_integer(Some(retained(unknown))), None);
            assert_eq!(cf_boolean(Some(retained(false_value))), Some(false));
            let true_object = objc2_foundation::NSNumber::numberWithBool(true);
            let true_value = Retained::as_ptr(&true_object).cast_mut().cast();
            assert_eq!(cf_boolean(Some(retained(true_value))), Some(true));
            assert_eq!(cf_integer(Some(retained(number))), Some(-42));
        });
    }

    #[test]
    fn explicit_consent_completion_ignores_status_but_preserves_cancellation() {
        let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let ticket = Ticket {
            deadline: Instant::now() + std::time::Duration::from_secs(1),
            live: {let live=live.clone();Arc::new(move ||live.load(std::sync::atomic::Ordering::Acquire))},
        };
        for status in [0, -1743, -1744, -600] {
            let calls=std::cell::Cell::new(0);
            assert_eq!(complete_safari_consent(&ticket,||{calls.set(calls.get()+1);status}),Ok(()));
            assert_eq!(calls.get(),1);
        }
        assert_eq!(complete_safari_consent(&ticket,||{
            live.store(false,std::sync::atomic::Ordering::Release);
            -1743
        }),Err(CANCELLED.into()));
    }
    #[test]
    fn owned_descriptor_roundtrip_uses_real_foundation_sdk() {
        autoreleasepool(|_| {
            let w = win(42).unwrap();
            assert_eq!(w.descriptorType(), code("obj "));
            let property = property("pnam", &w);
            assert_eq!(property.descriptorType(), code("obj "));
            let event = D::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
                code("core"),
                code("getd"),
                Some(&D::currentProcessDescriptor()),
                -1,
                0,
            );
            event.setParamDescriptor_forKeyword(&property, code("----"));
            assert_eq!(
                event
                    .paramDescriptorForKeyword(code("----"))
                    .unwrap()
                    .descriptorType(),
                code("obj ")
            );
            let list = D::listDescriptor();
            list.insertDescriptor_atIndex(
                &D::descriptorWithString(&NSString::from_str("Owned fixture")),
                1,
            );
            let ticket = Ticket {
                deadline: Instant::now() + std::time::Duration::from_secs(1),
                live: Arc::new(|| true),
            };
            assert_eq!(strings(&list, &ticket).unwrap(), ["Owned fixture"]);
        })
    }
}
