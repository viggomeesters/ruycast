//! Selected notification-center and main-queue leaves. Receivers and tokens
//! live in the existing workspace arena; every real proxy still requires the
//! common typed factory and original callbacks require the common catalog.
use super::*;
use crate::workspace::notifications::{Hub, NotificationFilter, NotificationReply};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    thread::{self, ThreadId},
};

pub(super) struct Centers {
    owner: ThreadId,
    next: Mutex<usize>,
    hubs: Mutex<BTreeMap<usize, Arc<Hub>>>,
    #[cfg(test)]
    targets: Mutex<BTreeMap<usize, Arc<delegate::ObjectiveC>>>,
}
impl Centers {
    pub(super) fn new(model: Arc<Workspace>) -> Result<Self> {
        Ok(Self {
            owner: thread::current().id(),
            next: Mutex::new(2),
            hubs: Mutex::new(
                [
                    (0, Arc::new(Hub::new(model.clone())?)),
                    (1, Arc::new(Hub::new(model)?)),
                ]
                .into(),
            ),
            #[cfg(test)]
            targets: Mutex::new(BTreeMap::new()),
        })
    }
    fn owner(&self) -> Result<()> {
        if self.owner != thread::current().id() {
            return Err("notification center on foreign owner thread".into());
        }
        model()?.with(|_| Ok(()))
    }
    fn hub(&self, id: usize) -> Result<Arc<Hub>> {
        self.owner()?;
        self.hubs
            .lock()
            .map_err(|_| "center state poisoned")?
            .get(&id)
            .cloned()
            .ok_or("unknown center".into())
    }
    fn private(&self) -> Result<usize> {
        self.owner()?;
        let id = {
            let mut next = self.next.lock().map_err(|_| "center ids poisoned")?;
            let id = *next;
            *next = next.checked_add(1).ok_or("center ids exhausted")?;
            id
        };
        let hub = Arc::new(Hub::new(
            INSTALLED.get().ok_or("workspace absent")?.model.clone(),
        )?);
        self.hubs
            .lock()
            .map_err(|_| "center state poisoned")?
            .insert(id, hub);
        Ok(id)
    }
}
fn centers() -> Result<&'static Centers> {
    let centers = &INSTALLED.get().ok_or("workspace absent")?.notifications;
    centers.owner()?;
    Ok(centers)
}
fn center(p: *mut AnyObject) -> Result<(usize, Arc<Hub>)> {
    let Kind::NotificationCenter(id) = kind(p)? else {
        return Err("unowned notification center".into());
    };
    Ok((id, centers()?.hub(id)?))
}
fn object_filter(p: *mut AnyObject) -> Result<Option<usize>> {
    if p.is_null() {
        return Ok(None);
    }
    kind(p)?;
    Ok(Some(p as usize))
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_snippet_choice_observers(model: &Arc<Workspace>, [panel, activation, resign]: [usize; 3]) -> Result<impl FnOnce() -> Result<()> + '_> {
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if !Arc::ptr_eq(model, &installed.model) || (panel == 0) != (resign == 0) {
        return Err("Reference SnippetChoice observer model or panel differs".into());
    }
    let verify = |token, center, name, filter| -> Result<(Arc<Hub>, u64)> {
        let Kind::NotificationToken(owner, id) = kind(token as *mut AnyObject)? else {
            return Err("Reference SnippetChoice observer token is not owned".into());
        };
        if owner != center || !model.notification_recipients(name)?.contains(&id) {
            return Err("Reference SnippetChoice observer center or notification differs".into());
        }
        let hub = centers()?.hub(center)?;
        hub.verify_owned_block(id, filter)?;
        Ok((hub, id))
    };
    let activation_name = "NSWorkspaceDidActivateApplicationNotification";
    let resign_name = "NSWindowDidResignKeyNotification";
    let activation = verify(activation, 1, activation_name, None)?;
    let resign = if panel != 0 {
        let window = window_identity(panel as *mut AnyObject)?;
        if window.origin != WindowOrigin::ReferenceSnippetChoice || !Arc::ptr_eq(model, &window.model) {
            return Err("Reference SnippetChoice observer panel is foreign".into());
        }
        Some(verify(resign, 0, resign_name,
            Some(super::super::notifications::NotificationFilter::Window { pointer: panel, handle: window.handle }))?)
    } else {
        None
    };
    Ok(move || {
        let verify_removed = |(hub, id): (Arc<Hub>, u64), name| -> Result<()> {
            if model.notification_recipients(name)?.contains(&id) {
                return Err("Reference SnippetChoice observer subscription remains live".into());
            }
            hub.verify_block_removed(id)
        };
        verify_removed(activation, activation_name)?;
        if let Some(resign) = resign {
            verify_removed(resign, resign_name)?;
        }
        Ok(())
    })
}

pub(super) fn window_resigned_key(model: &Workspace, handle: Handle) -> Result<()> {
    let name = "NSWindowDidResignKeyNotification";
    let recipients = model.notification_recipients(name)?;
    if recipients.is_empty() {
        return Ok(());
    }
    let installed = INSTALLED.get().ok_or("workspace absent")?;
    if !std::ptr::eq(model, installed.model.as_ref()) {
        return Err("key-window notification model differs".into());
    }
    let hub = centers()?.hub(0)?;
    let pointer = {
        let objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
        objects.iter().find_map(|(&pointer, kind)| {
            match kind {
                Kind::Window(window) => *window == handle,
                Kind::WindowPayload(window) => window.0.handle == handle,
                _ => false,
            }.then_some(pointer as *mut AnyObject)
        }).ok_or("resigned key window has no owned receiver")?
    };
    if !hub.has_recipient_for_window(&recipients, pointer as usize, handle)? {
        return Ok(());
    }
    if kind(pointer)? != Kind::Window(handle) {
        return Err("resigned key window receiver changed".into());
    }
    #[cfg(test)]
    if installed.fixture_classes {
        let window = unsafe { Retained::retain(pointer) }.ok_or("resigned key window retain failed")?;
        unsafe { hub.post(name, Some(&window)) }?;
        return Ok(());
    }
    let window = WindowReference::acquire(pointer, c"NSWindow")?;
    unsafe { hub.post(name, window.pointer()?.as_ref()) }?;
    Ok(())
}

#[cfg(test)]
#[test]
fn key_transfer_and_order_out_emit_owned_resignation_once() {
    use block2::RcBlock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    objc2::rc::autoreleasepool(|_| {
        let fixture = super::super::tests::Fixture::new();
        super::install_fixture(fixture.model.clone()).unwrap();
        let make_window = || {
            let handle = fixture.model.new_window(Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Point { x: 300.0, y: 200.0 },
            }).unwrap();
            object(Kind::Window(handle)).unwrap()
        };
        let first = make_window();
        let second = make_window();
        let name = NSString::from_str("NSWindowDidResignKeyNotification");
        let hits = Arc::new(AtomicUsize::new(0));
        let received = hits.clone();
        let first_address = first as usize;
        let block = RcBlock::new(move |_: *mut AnyObject| {
            received.fetch_add(1, Ordering::SeqCst);
            unsafe { super::order(first_address as *mut AnyObject, objc2::sel!(orderOut:), std::ptr::null_mut()); }
        });
        let center = object(Kind::NotificationCenter(0)).unwrap();
        let token = subscribe_block(
            center, Retained::as_ptr(&name).cast_mut().cast(), first, std::ptr::null_mut(),
            (&*block as *const NotificationReply).cast_mut(),
        ).unwrap();
        let (_, hub) = self::center(center).unwrap();
        let recipients = fixture.model.notification_recipients("NSWindowDidResignKeyNotification").unwrap();
        let first_handle = super::resignation_window_handle(first as usize).unwrap().unwrap();
        let second_handle = super::resignation_window_handle(second as usize).unwrap().unwrap();
        assert!(hub.has_recipient_for_window(&recipients, first as usize, first_handle).unwrap());
        assert!(!hub.has_recipient_for_window(&recipients, first as usize, second_handle).unwrap());
        let order = |window, selector| unsafe { super::order(window, selector, std::ptr::null_mut()) };
        let key = objc2::sel!(makeKeyAndOrderFront:);
        let out = objc2::sel!(orderOut:);
        order(first, key);
        order(first, key);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        order(second, key);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        order(first, key);
        order(first, out);
        order(first, out);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        remove(center, token).unwrap();
        order(first, key);
        order(first, out);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(!fixture.model.faulted.load(Ordering::Acquire));
    });
}
fn boundary<T>(fallback: T, f: impl FnOnce() -> Result<T>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => return value,
        // Even dropping an unknown panic payload can run Rust code. Keep it
        // for process lifetime rather than allowing a second foreign unwind.
        Err(payload) => std::mem::forget(payload),
        Ok(Err(_)) => (),
    }
    if let Some(installed) = INSTALLED.get() {
        installed.model.fault();
    }
    fallback
}
unsafe extern "C" fn default_center(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        centers()?;
        object(Kind::NotificationCenter(0))
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_CENTER: OnceLock<(usize, crate::sdk::metadata::MethodSnapshot)> = OnceLock::new();

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn bind_platform_center(class: &AnyClass, original: crate::sdk::metadata::MethodSnapshot) -> Result<()> {
    if original.owner() != class.metaclass() as *const AnyClass as usize
        || original.encoding() != b"@16@0:8"
        || original.code().image() != b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
    {
        return Err("Reference platform notification center source differs".into());
    }
    PLATFORM_CENTER.set((class as *const AnyClass as usize, original))
        .map_err(|_| "Reference platform notification center already bound".into())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn selected_default_center(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym default_center_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn default_center_checked(class: *mut AnyClass, selector: Sel, caller: usize) -> *mut AnyObject {
    if crate::sdk::private_network::reference_website_assembly_active() {
        if let Some((owner, original)) = PLATFORM_CENTER.get() {
            let caller = caller.checked_sub(4).and_then(|pc| crate::sdk::metadata::code_identity(pc).ok());
            if caller.is_some_and(|site| matches!(site.offset(), 0x24634 | 0x53b48)
                && site.image() == original.code().image() && site.uuid() == original.code().uuid())
            {
                if class as usize != *owner || selector != objc2::sel!(defaultCenter)
                    || !crate::sdk::metadata::code_identity(original.implementation()).is_ok_and(|code| code == *original.code())
                {
                    unsafe { libc::_exit(89) };
                }
                let platform: unsafe extern "C" fn(*mut AnyClass, Sel) -> *mut AnyObject =
                    unsafe { std::mem::transmute(original.implementation()) };
                return unsafe { platform(class, selector) };
            }
        }
    }
    unsafe { default_center(class, selector) }
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
use default_center as selected_default_center;
unsafe extern "C" fn workspace_center(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        if kind(p)? != Kind::Workspace {
            return Err("notificationCenter requires owned workspace".into());
        }
        object(Kind::NotificationCenter(1))
    })
}
unsafe extern "C" fn new_center(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let p = object(Kind::NotificationCenter(centers()?.private()?))?;
        #[cfg(test)]
        if INSTALLED
            .get()
            .is_some_and(|installed| installed.fixture_classes)
        {
            return Ok(Retained::into_raw(
                unsafe { Retained::retain(p) }.ok_or("fixture center retain failed")?,
            ));
        }
        // `new` transfers one +1 beyond the arena's reference, through the
        // actual factory's owner-thread/allocation/qualified ARC proof.
        unsafe { super::factory::retain_owned(p) }
    })
}
unsafe extern "C" fn main_queue(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        centers()?;
        object(Kind::MainQueue)
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static PLATFORM_MAIN_QUEUE: OnceLock<(usize, crate::sdk::metadata::MethodSnapshot)> = OnceLock::new();

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn bind_platform_main_queue(class: &AnyClass, original: crate::sdk::metadata::MethodSnapshot) -> Result<()> {
    if original.owner() != class.metaclass() as *const AnyClass as usize
        || original.encoding() != b"@16@0:8"
        || original.code().image() != b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
    {
        return Err("Reference platform main queue source differs".into());
    }
    PLATFORM_MAIN_QUEUE.set((class as *const AnyClass as usize, original))
        .map_err(|_| "Reference platform main queue already bound".into())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn selected_main_queue(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym main_queue_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn main_queue_checked(class: *mut AnyClass, selector: Sel, caller: usize) -> *mut AnyObject {
    if crate::sdk::private_network::reference_website_uses_platform_main_queue(caller) {
        let Some((owner, original)) = PLATFORM_MAIN_QUEUE.get() else { unsafe { libc::_exit(89) } };
        if class as usize != *owner || selector != objc2::sel!(mainQueue)
            || !crate::sdk::metadata::code_identity(original.implementation()).is_ok_and(|code| code == *original.code())
        {
            unsafe { libc::_exit(89) };
        }
        let platform: unsafe extern "C" fn(*mut AnyClass, Sel) -> *mut AnyObject =
            unsafe { std::mem::transmute(original.implementation()) };
        return unsafe { platform(class, selector) };
    }
    unsafe { main_queue(class, selector) }
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
use main_queue as selected_main_queue;
fn subscribe_block(
    p: *mut AnyObject,
    name: *mut AnyObject,
    filter: *mut AnyObject,
    queue: *mut AnyObject,
    block: *mut NotificationReply,
) -> Result<*mut AnyObject> {
    let (center_id, hub) = center(p)?;
    if !queue.is_null() && kind(queue)? != Kind::MainQueue {
        return Err("only the exact owned main queue is supported".into());
    }
    let name = get_string(name)?;
    let filter = if name == "NSWindowDidResignKeyNotification" {
        let window = super::resignation_window_handle(filter as usize)?;
        if center_id != 0 || window.is_none() {
            return Err("key-window resignation requires the default center and an owned window".into());
        }
        #[cfg(test)]
        let fixture = INSTALLED.get().is_some_and(|installed| installed.fixture_classes);
        #[cfg(not(test))]
        let fixture = false;
        if !fixture {
            WindowReference::acquire(filter, c"NSWindow")?;
        }
        window.map(|handle| NotificationFilter::Window { pointer: filter as usize, handle })
    } else {
        object_filter(filter)?.map(NotificationFilter::Object)
    };
    unsafe {
        hub.subscribe_with_token(&name, block, filter, |id| object(Kind::NotificationToken(center_id, id)))
    }
}
unsafe extern "C" fn add_block(
    p: *mut AnyObject,
    _: Sel,
    name: *mut AnyObject,
    filter: *mut AnyObject,
    queue: *mut AnyObject,
    block: *mut NotificationReply,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        subscribe_block(p, name, filter, queue, block)
    })
}
fn subscribe_selector(
    p: *mut AnyObject,
    target: *mut AnyObject,
    selector: Sel,
    name: *mut AnyObject,
    filter: *mut AnyObject,
) -> Result<()> {
    let (_, hub) = center(p)?;
    let name = get_string(name)?;
    if name == "NSWindowDidResignKeyNotification" {
        return Err("key-window resignation requires its source block observer".into());
    }
    let filter = object_filter(filter)?;
    #[cfg(test)]
    let fixture = centers()?
        .targets
        .lock()
        .map_err(|_| "fixture target state poisoned")?
        .get(&(target as usize))
        .cloned();
    #[cfg(not(test))]
    let fixture: Option<Arc<delegate::ObjectiveC>> = None;
    let selected = match fixture {
        Some(target) => target,
        None => application::selected_callback(target, p)?,
    };
    unsafe {
        hub.subscribe_selector(
            &name,
            selected,
            selector
                .name()
                .to_str()
                .map_err(|_| "nonUTF8 notification selector")?,
            filter,
        )
    }?;
    Ok(())
}
unsafe extern "C" fn add_selector(
    p: *mut AnyObject,
    _: Sel,
    target: *mut AnyObject,
    selector: Sel,
    name: *mut AnyObject,
    filter: *mut AnyObject,
) {
    boundary((), || subscribe_selector(p, target, selector, name, filter))
}
fn remove(p: *mut AnyObject, observer: *mut AnyObject) -> Result<()> {
    let (center_id, hub) = center(p)?;
    let known = INSTALLED
        .get()
        .ok_or("workspace absent")?
        .objects
        .lock()
        .map_err(|_| "object arena poisoned")?
        .get(&(observer as usize))
        .cloned();
    match known {
        Some(Kind::NotificationToken(owner, id)) if owner == center_id => {
            hub.unsubscribe_if_present(id)
        }
        Some(Kind::NotificationToken(_, _)) => Ok(()),
        _ => hub.unsubscribe_selector_target(observer as usize),
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
struct HyperObserverDrain {
    caller: usize,
    keyboard: usize,
    removed: Vec<u64>,
    removing: bool,
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
thread_local! {
    static HYPER_OBSERVER_DRAIN: std::cell::RefCell<Option<HyperObserverDrain>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn drain_hyper_observers(
    caller: usize, keyboard: usize, release: impl FnOnce() -> Result<()>,
) -> Result<()> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            HYPER_OBSERVER_DRAIN.with(|slot| { slot.borrow_mut().take(); });
        }
    }
    HYPER_OBSERVER_DRAIN.with(|slot| -> Result<()> {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err("Reference HyperKey observer drain reentered".into());
        }
        *slot = Some(HyperObserverDrain { caller, keyboard, removed: Vec::with_capacity(6), removing: false });
        Ok(())
    })?;
    let _reset = Reset;
    release()?;
    HYPER_OBSERVER_DRAIN.with(|slot| {
        let slot = slot.borrow();
        if slot.as_ref().is_some_and(|drain| !drain.removing && drain.removed.len() == 6) {
            Ok(())
        } else {
            Err("Original HyperKey deinit did not drain its six observers".into())
        }
    })
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn remove_observer(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!(
        "mov x3, x30", "mov x4, x24", "mov x5, sp", "b {checked}", checked = sym remove_observer_checked,
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn remove_observer_checked(
    p: *mut AnyObject, _: Sel, observer: *mut AnyObject, caller: usize, index: usize,
    caller_stack: usize,
) {
    boundary((), || {
        let selected = HYPER_OBSERVER_DRAIN.with(|slot| -> Result<bool> {
            let mut slot = slot.borrow_mut();
            let Some(drain) = slot.as_mut().filter(|drain| drain.caller == caller) else {
                return Ok(false);
            };
            if drain.removing || index != drain.removed.len() + 1 || index > 6 {
                return Err("Original HyperKey observer iteration differs".into());
            }
            // The pinned destructor saves its x19 self at sp+16 before the loop.
            let owner = caller_stack.checked_add(16).ok_or("HyperKey observer stack overflow")?;
            if !caller_stack.is_multiple_of(16) || unsafe { (owner as *const usize).read() } != drain.keyboard {
                return Err("Original HyperKey observer owner differs".into());
            }
            drain.removing = true;
            Ok(true)
        })?;
        if !selected {
            return remove(p, observer);
        }
        let (center_id, hub) = center(p)?;
        let Kind::NotificationToken(owner, id) = kind(observer)? else {
            return Err("Original HyperKey observer token is not owned".into());
        };
        if center_id != 1 || owner != center_id {
            return Err("Original HyperKey observer belongs to another center".into());
        }
        HYPER_OBSERVER_DRAIN.with(|slot| -> Result<()> {
            let slot = slot.borrow();
            let drain = slot.as_ref().ok_or("Reference HyperKey observer drain disappeared")?;
            if drain.removed.contains(&id) {
                return Err("Original HyperKey observer token repeated".into());
            }
            Ok(())
        })?;
        hub.remove_reference_block(id)?;
        HYPER_OBSERVER_DRAIN.with(|slot| {
            let mut slot = slot.borrow_mut();
            let drain = slot.as_mut().ok_or("Reference HyperKey observer drain disappeared")?;
            drain.removed.push(id);
            drain.removing = false;
            Ok(())
        })
    })
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn remove_observer(p: *mut AnyObject, _: Sel, observer: *mut AnyObject) {
    boundary((), || remove(p, observer))
}
unsafe extern "C" fn post_name(
    p: *mut AnyObject,
    _: Sel,
    name: *mut AnyObject,
    object: *mut AnyObject,
) {
    boundary((), || {
        let (_, hub) = center(p)?;
        object_filter(object)?;
        unsafe { hub.post(&get_string(name)?, object.as_ref()) }?;
        Ok(())
    })
}
unsafe extern "C" fn post_with_info(
    p: *mut AnyObject,
    _: Sel,
    name: *mut AnyObject,
    object: *mut AnyObject,
    info: *mut AnyObject,
) {
    boundary((), || {
        let (_, hub) = center(p)?;
        object_filter(object)?;
        let info = if info.is_null() {
            None
        } else {
            // Resolve the existing typed dictionary before any cast or message
            // to the caller's raw pointer. A class-name probe is no ownership.
            check_user_info(info)?;
            let value = unsafe { &*info.cast::<objc2_foundation::NSDictionary>() };
            Some(value)
        };
        unsafe { hub.post_with_user_info(&get_string(name)?, object.as_ref(), info) }?;
        Ok(())
    })
}
fn check_user_info(info: *mut AnyObject) -> Result<()> {
    #[cfg(test)]
    if INSTALLED
        .get()
        .is_some_and(|installed| installed.fixture_classes)
    {
        // The existing isolated class fixture supplies its own real data.
        let value = unsafe { &*info.cast::<objc2_foundation::NSDictionary>() };
        return if value
            .isKindOfClass(objc2_foundation::NSDictionary::<AnyObject, AnyObject>::class())
        {
            Ok(())
        } else {
            Err("notification fixture userInfo must be a dictionary".into())
        };
    }
    foundation_dictionary_entries(info)?;
    Ok(())
}
macro_rules! notification_methods {
    ($consumer:ident) => {$consumer! {
        (c"NSNotificationCenter",c"defaultCenter",true,selected_default_center,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (c"NSNotificationCenter",c"new",true,new_center,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (c"NSWorkspace",c"notificationCenter",false,workspace_center,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSOperationQueue",c"mainQueue",true,selected_main_queue,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (c"NSNotificationCenter",c"addObserverForName:object:queue:usingBlock:",false,add_block,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject,*mut AnyObject,*mut NotificationReply)->*mut AnyObject),
        (c"NSNotificationCenter",c"addObserver:selector:name:object:",false,add_selector,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)),
        (c"NSNotificationCenter",c"removeObserver:",false,remove_observer,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
        (c"NSNotificationCenter",c"postNotificationName:object:",false,post_name,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)),
        (c"NSNotificationCenter",c"postNotificationName:object:userInfo:",false,post_with_info,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject,*mut AnyObject))
    }};
}
macro_rules! export_notification_methods {
    ($(($class:expr,$selector:expr,$meta:expr,$handler:ident,$ty:ty)),*$(,)?) => {
        pub(super) fn replacements()->Vec<(&'static CStr,&'static CStr,bool,usize)> {
            vec![$(($class,$selector,$meta,($handler as $ty) as *const () as usize)),*]
        }
        pub(super) fn signatures()->Vec<(&'static CStr,&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
            vec![$(($class,$selector,$meta,signature::<$ty>())),*]
        }
    };
}
fn signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut types = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
    types.extend_from_slice(F::Arguments::ENCODINGS);
    types
}
notification_methods!(export_notification_methods);

/// Exact loaded method observations only; no constructor or callback authority.
pub(super) struct NotificationMethodPreflight {
    pub class: &'static CStr,
    pub selector: &'static CStr,
    pub class_method: bool,
    pub source: crate::sdk::metadata::MethodSnapshot,
    pub replacement: crate::sdk::metadata::CodeIdentity,
}
/// Complete fixed nine-method ABI preflight, before any class/factory mutation.
pub(super) fn preflight() -> Result<Vec<NotificationMethodPreflight>> {
    let signatures = signatures();
    let replacements = replacements();
    if signatures.len() != 9 || replacements.len() != 9 {
        return Err("notification fixed method roster differs".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut observed = Vec::with_capacity(9);
    for (class, selector, class_method, implementation) in replacements {
        if !seen.insert((class, selector, class_method)) {
            return Err("duplicate notification method".into());
        }
        let mut matching = signatures
            .iter()
            .filter(|(c, s, m, _)| *c == class && *s == selector && *m == class_method);
        let signature = matching
            .next()
            .ok_or("notification method signature missing")?;
        if matching.next().is_some() {
            return Err("duplicate notification signature".into());
        }
        let loaded = crate::sdk::metadata::loaded_class(class)?;
        let source = crate::sdk::metadata::method(loaded, selector, class_method)?
            .ok_or("source notification method missing")?;
        super::classes::verify_typed_abi(&source, &signature.3)?;
        observed.push(NotificationMethodPreflight {
            class,
            selector,
            class_method,
            source,
            replacement: crate::sdk::metadata::code_identity(implementation)?,
        });
    }
    Ok(observed)
}
#[cfg(test)]
pub(super) fn add_methods(builder: &mut objc2::runtime::ClassBuilder) {
    unsafe {
        builder.add_class_method(
            objc2::sel!(defaultCenter),
            default_center as unsafe extern "C" fn(*mut AnyClass, Sel) -> *mut AnyObject,
        );
        builder.add_class_method(
            objc2::sel!(mainQueue),
            main_queue as unsafe extern "C" fn(*mut AnyClass, Sel) -> *mut AnyObject,
        );
        builder.add_method(
            objc2::sel!(notificationCenter),
            workspace_center as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        builder.add_method(
            objc2::sel!(addObserverForName:object:queue:usingBlock:),
            add_block
                as unsafe extern "C" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                    *mut AnyObject,
                    *mut NotificationReply,
                ) -> *mut AnyObject,
        );
        builder.add_method(
            objc2::sel!(addObserver:selector:name:object:),
            add_selector
                as unsafe extern "C" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                ),
        );
        builder.add_method(
            objc2::sel!(removeObserver:),
            remove_observer as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        builder.add_method(
            objc2::sel!(postNotificationName:object:),
            post_name as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
        );
        builder.add_method(
            objc2::sel!(postNotificationName:object:userInfo:),
            post_with_info
                as unsafe extern "C" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                    *mut AnyObject,
                ),
        );
    }
}

#[cfg(test)]
pub(super) fn prove_fixture_centers(
    model: Arc<Workspace>,
    workspace: *mut AnyObject,
    window: *mut AnyObject,
) {
    use block2::RcBlock;
    use objc2::{msg_send, runtime::ClassBuilder};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let class = own_class();
    let default: *mut AnyObject = unsafe { msg_send![class, defaultCenter] };
    let queue: *mut AnyObject = unsafe { msg_send![class, mainQueue] };
    let workspace_center: *mut AnyObject = unsafe { msg_send![&*workspace, notificationCenter] };
    // Call the selected +new leaf against the owned class only: it is not
    // installed as +new on the shared data class (that would recurse into the
    // fixture allocator). No real center factory or family admission is claimed.
    let private =
        unsafe { new_center(class as *const AnyClass as *mut AnyClass, objc2::sel!(new)) };
    let private_again =
        unsafe { new_center(class as *const AnyClass as *mut AnyClass, objc2::sel!(new)) };
    assert_eq!(
        [
            default as usize,
            workspace_center as usize,
            private as usize,
            private_again as usize
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len(),
        4
    );
    assert!(kind(queue).unwrap() == Kind::MainQueue);
    let name = NSString::from_str("NSWorkspaceWillSleepNotification");
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let third = Arc::new(AtomicUsize::new(0));
    let mut tokens = vec![];
    let mut blocks = vec![];
    for (center, hits) in [
        (default, first.clone()),
        (workspace_center, second.clone()),
        (private, third.clone()),
    ] {
        let block = RcBlock::new(move |notice: *mut AnyObject| {
            let notice = unsafe { &*notice.cast::<objc2_foundation::NSNotification>() };
            if notice.name().to_string() == "NSWorkspaceWillSleepNotification" {
                hits.fetch_add(1, Ordering::SeqCst);
            }
        });
        let token: *mut AnyObject = unsafe {
            msg_send![&*center,addObserverForName:&*name,object:std::ptr::null::<AnyObject>(),queue:queue,usingBlock:&*block]
        };
        assert!(!token.is_null());
        tokens.push(token);
        blocks.push(block);
    }
    unsafe {
        let _: () = msg_send![&*workspace_center,postNotificationName:&*name,object:std::ptr::null::<AnyObject>()];
    }
    assert_eq!(
        (
            first.load(Ordering::SeqCst),
            second.load(Ordering::SeqCst),
            third.load(Ordering::SeqCst)
        ),
        (0, 1, 0)
    );
    unsafe {
        let _: () = msg_send![&*default,removeObserver:tokens[1]];
    }
    unsafe {
        let _: () = msg_send![&*workspace_center,postNotificationName:&*name,object:std::ptr::null::<AnyObject>()];
    }
    assert_eq!(second.load(Ordering::SeqCst), 2);
    unsafe {
        let _: () = msg_send![&*workspace_center,removeObserver:tokens[1]];
    }
    unsafe {
        let _: () = msg_send![&*workspace_center,removeObserver:tokens[1]];
    }
    unsafe {
        let _: () = msg_send![&*workspace_center,postNotificationName:&*name,object:std::ptr::null::<AnyObject>()];
    }
    assert_eq!(second.load(Ordering::SeqCst), 2);
    unsafe {
        let _: () =
            msg_send![&*default,postNotificationName:&*name,object:std::ptr::null::<AnyObject>()];
    }
    unsafe {
        let _: () =
            msg_send![&*private,postNotificationName:&*name,object:std::ptr::null::<AnyObject>()];
    }
    assert_eq!(
        (first.load(Ordering::SeqCst), third.load(Ordering::SeqCst)),
        (1, 1)
    );
    // Wrong queue is refused by exact receiver identity before block retention.
    let name_ptr = Retained::as_ptr(&name).cast_mut().cast();
    assert!(subscribe_block(
        default,
        name_ptr,
        std::ptr::null_mut(),
        workspace,
        (&*blocks[0] as *const NotificationReply).cast_mut()
    )
    .is_err());

    static SELECTED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn resized(_: *mut AnyObject, _: Sel, notice: *mut AnyObject) {
        let notice = unsafe { &*notice.cast::<objc2_foundation::NSNotification>() };
        if notice.name().to_string() == "NSWindowDidResizeNotification" && notice.object().is_some()
        {
            SELECTED.fetch_add(1, Ordering::SeqCst);
        }
    }
    let mut target_class =
        ClassBuilder::new(c"RuycastF1OwnedCenterBridgeTarget", NSObject::class()).unwrap();
    unsafe {
        target_class.add_method(
            objc2::sel!(resized:),
            resized as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }
    let target_class = target_class.register();
    let target: Retained<AnyObject> = unsafe { msg_send![target_class, new] };
    let target_ptr = Retained::as_ptr(&target).cast_mut();
    let selected = Arc::new(
        unsafe {
            delegate::ObjectiveC::owned_fixture(
                model,
                target_ptr,
                &[(c"resized:", resized as *const () as usize)],
            )
        }
        .unwrap(),
    );
    centers()
        .unwrap()
        .targets
        .lock()
        .unwrap()
        .insert(target_ptr as usize, selected);
    let resized_name = NSString::from_str("NSWindowDidResizeNotification");
    unsafe {
        let _: () = msg_send![&*default,addObserver:target_ptr,selector:objc2::sel!(resized:),name:&*resized_name,object:window];
    }
    unsafe {
        let _: () = msg_send![&*default,postNotificationName:&*resized_name,object:workspace];
    }
    assert_eq!(SELECTED.load(Ordering::SeqCst), 0);
    unsafe {
        let _: () = msg_send![&*default,postNotificationName:&*resized_name,object:window];
    }
    assert_eq!(SELECTED.load(Ordering::SeqCst), 1);
    unsafe {
        let _: () = msg_send![&*default,removeObserver:target_ptr];
    }
    unsafe {
        let _: () = msg_send![&*default,postNotificationName:&*resized_name,object:window];
    }
    assert_eq!(SELECTED.load(Ordering::SeqCst), 1);
    let resigned = NSString::from_str("NSWindowDidResignKeyNotification");
    let resign_hits = Arc::new(AtomicUsize::new(0));
    let hits = resign_hits.clone();
    let resign_block = RcBlock::new(move |_: *mut AnyObject| {
        hits.fetch_add(1, Ordering::SeqCst);
    });
    let resigned_ptr = Retained::as_ptr(&resigned).cast_mut().cast();
    let block_ptr = (&*resign_block as *const NotificationReply).cast_mut();
    assert!(subscribe_block(workspace_center, resigned_ptr, window, queue, block_ptr).is_err());
    assert!(subscribe_block(default, resigned_ptr, workspace, queue, block_ptr).is_err());
    let resign_token = subscribe_block(default, resigned_ptr, window, queue, block_ptr).unwrap();
    unsafe {
        let _: () = msg_send![&*default,postNotificationName:&*resigned,object:workspace];
    }
    assert_eq!(resign_hits.load(Ordering::SeqCst), 0);
    unsafe {
        let _: () = msg_send![&*default,postNotificationName:&*resigned,object:window];
    }
    assert_eq!(resign_hits.load(Ordering::SeqCst), 1);
    unsafe {
        let _: () = msg_send![&*default,removeObserver:resign_token];
        let _: () = msg_send![&*default,postNotificationName:&*resigned,object:window];
    }
    assert_eq!(resign_hits.load(Ordering::SeqCst), 1);
    for (center, token) in [(default, tokens[0]), (private, tokens[2])] {
        unsafe {
            let _: () = msg_send![&*center,removeObserver:token];
        }
    }
    let payload_name = NSString::from_str("NSWorkspaceDidActivateApplicationNotification");
    let key = NSString::from_str("OwnedPayload");
    let payload = objc2_foundation::NSDictionary::from_slices(&[&*key], &[unsafe { &*window }]);
    let payload_hits = Arc::new(AtomicUsize::new(0));
    let received = payload_hits.clone();
    let expected = window as usize;
    let block = RcBlock::new(move |notice: *mut AnyObject| {
        let notice = unsafe { &*notice.cast::<objc2_foundation::NSNotification>() };
        if notice
            .userInfo()
            .and_then(|info| info.objectForKey(&key))
            .is_some_and(|value| Retained::as_ptr(&value) as usize == expected)
        {
            received.fetch_add(1, Ordering::SeqCst);
        }
    });
    let token: *mut AnyObject = unsafe {
        msg_send![&*workspace_center,addObserverForName:&*payload_name,object:std::ptr::null::<AnyObject>(),queue:queue,usingBlock:&*block]
    };
    unsafe {
        let _: () = msg_send![&*workspace_center,postNotificationName:&*payload_name,object:std::ptr::null::<AnyObject>(),userInfo:&*payload];
    }
    assert_eq!(payload_hits.load(Ordering::SeqCst), 1);
    unsafe {
        let _: () = msg_send![&*workspace_center,removeObserver:token];
    }
    // Balance the explicit +new caller references; arena pins remain alive.
    drop(unsafe { Retained::from_raw(private) }.unwrap());
    drop(unsafe { Retained::from_raw(private_again) }.unwrap());
}
#[cfg(test)]
pub(super) fn prove_retired_boundary() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Payload;
    impl Drop for Payload {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
            panic!("own payload destructor");
        }
    }
    unsafe extern "C" fn own_boundary() -> usize {
        boundary(0, || std::panic::panic_any(Payload))
    }
    assert_eq!(unsafe { own_boundary() }, 0);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
}
