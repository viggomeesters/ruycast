//! Own status/menu/button model with actual typed Objective-C dispatch. No menu
//! registration, status item, app activation, pop-up or desktop event API.
use super::super::event_loop::Event;
use super::*;
#[derive(Clone)]
struct Fields {
    title: String,
    key: String,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    identifier: Option<String>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    key_modifier_mask: usize,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    separator: bool,
    tooltip: Option<String>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    accessibility_label: Option<String>,
    items: Vec<Handle>,
    menu: Option<Handle>,
    parent_menu: Option<Handle>,
    button: Option<Handle>,
    target: usize,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    target_identity: Option<super::super::receiver::ReceiverIdentity>,
    action: Option<Sel>,
    tag: isize,
    state: isize,
    enabled: bool,
    live: bool,
    length: f64,
    image: usize,
}
struct Controls {
    model: Arc<Workspace>,
    nodes: Mutex<BTreeMap<Handle, Fields>>,
    owner: std::thread::ThreadId,
}
static CONTROLS: OnceLock<Controls> = OnceLock::new();
pub(super) fn initialize(model: Arc<Workspace>) -> Result<()> {
    CONTROLS
        .set(Controls {
            model,
            nodes: Mutex::new(BTreeMap::new()),
            owner: std::thread::current().id(),
        })
        .map_err(|_| "controls already installed".into())
}
fn state() -> Result<&'static Controls> {
    model()?.with(|_| Ok(()))?;
    let s = CONTROLS.get().ok_or("controls absent")?;
    if std::thread::current().id() != s.owner {
        return Err("control on foreign owner thread".into());
    }
    Ok(s)
}
fn allocate(title: String) -> Result<Handle> {
    let s = state()?;
    let h = s.model.with(|s| {
        let h = Handle {
            id: s.next,
            generation: s.generation,
        };
        s.next = s.next.checked_add(1).ok_or("control IDs exhausted")?;
        Ok(h)
    })?;
    s.nodes
        .lock()
        .map_err(|_| "control state poisoned")?
        .insert(
            h,
            Fields {
                title,
                key: String::new(),
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                identifier: None,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                key_modifier_mask: 1 << 20,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                separator: false,
                tooltip: None,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                accessibility_label: None,
                items: vec![],
                menu: None,
                parent_menu: None,
                button: None,
                target: 0,
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                target_identity: None,
                action: None,
                tag: 0,
                state: 0,
                enabled: true,
                live: true,
                length: 0.0,
                image: 0,
            },
        );
    Ok(h)
}
fn handle(p: *mut AnyObject) -> Result<Handle> {
    match kind(p)? {
        Kind::Menu(h) | Kind::MenuItem(h) | Kind::StatusItem(h) | Kind::Button(h) => Ok(h),
        _ => Err("unknown control receiver".into()),
    }
}
fn with<T>(p: *mut AnyObject, f: impl FnOnce(&mut Fields) -> Result<T>) -> Result<T> {
    let h = handle(p)?;
    let mut nodes = state()?
        .nodes
        .lock()
        .map_err(|_| "control state poisoned")?;
    let node = nodes.get_mut(&h).ok_or("unknown control record")?;
    if !node.live {
        return Err("control retired".into());
    }
    f(node)
}
fn fatal() -> ! {
    unsafe { libc::_exit(78) }
}
pub fn menu(title: &str) -> Result<*mut AnyObject> {
    object(Kind::Menu(allocate(title.into())?))
}
pub fn item(title: &str, action: Option<Sel>, key: &str) -> Result<*mut AnyObject> {
    let h = allocate(title.into())?;
    {
        let mut nodes = state()?
            .nodes
            .lock()
            .map_err(|_| "control state poisoned")?;
        let n = nodes.get_mut(&h).unwrap();
        n.action = action;
        n.key = key.into();
    }
    object(Kind::MenuItem(h))
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn init_menu(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> *mut AnyObject {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym init_menu_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn init_menu(p: *mut AnyObject, s: Sel, title: *mut AnyObject) -> *mut AnyObject {
    unsafe { init_menu_checked(p, s, title, 0) }
}

unsafe extern "C" fn init_menu_checked(
    p: *mut AnyObject, _: Sel, title: *mut AnyObject, caller: usize,
) -> *mut AnyObject {
    initialize_menu_control(p, Some(title), None, caller)
}
unsafe extern "C" fn init_menu_default(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    initialize_menu_control(p, None, None, 0)
}
// Own only the new unpublished pure-data node; removal never executes a
// source/ARC callback while the controls mutex is held.
struct PendingMenuNode {controls:&'static Controls,handle:Handle,committed:bool}
impl Drop for PendingMenuNode {
    fn drop(&mut self) {
        if !self.committed {
            let removed=match self.controls.nodes.lock() {
                Ok(mut nodes)=>nodes.remove(&self.handle),
                Err(_)=>{self.controls.model.fault();None},
            };
            drop(removed);
        }
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn init_item(
    _: *mut AnyObject, _: Sel, _: *mut AnyObject, _: Option<Sel>, _: *mut AnyObject,
) -> *mut AnyObject {
    core::arch::naked_asm!("mov x5, x30", "b {checked}", checked = sym init_item_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn init_item(
    p: *mut AnyObject, s: Sel, title: *mut AnyObject, action: Option<Sel>, key: *mut AnyObject,
) -> *mut AnyObject {
    unsafe { init_item_checked(p, s, title, action, key, 0) }
}

unsafe extern "C" fn init_item_checked(
    p: *mut AnyObject, _: Sel, title: *mut AnyObject, action: Option<Sel>, key: *mut AnyObject,
    caller: usize,
) -> *mut AnyObject {
    initialize_menu_control(p, Some(title), Some((action, key)), caller)
}
fn initialize_menu_control(
    p: *mut AnyObject,
    title: Option<*mut AnyObject>,
    item: Option<(Option<Sel>, *mut AnyObject)>,
    caller: usize,
) -> *mut AnyObject {
    (||->Result<_>{
        let controls=CONTROLS.get().ok_or("controls absent")?;
        let installed=INSTALLED.get().ok_or("workspace absent")?;
        if controls.owner!=std::thread::current().id()||!Arc::ptr_eq(&controls.model,&installed.model) {
            return Err("menu initializer owner/model differs".into());
        }
        let generation=controls.model.state.lock().map_err(|_|"workspace state poisoned")?.generation;
        #[cfg(test)]
        let actual=!installed.fixture_classes;
        #[cfg(not(test))]
        let actual=true;
        // Claim the exact real new +1 before argument/source callbacks. Fixture
        // classes remain an explicit test-only path, never production authority.
        let claim=if actual {
            let role = if item.is_some() { c"NSMenuItem" } else { c"NSMenu" };
            if super::super::factory::receiver_role(p,&controls.model)? != role {
                return Err("menu initializer receiver role differs".into());
            }
            let serial=super::super::factory::owned_reference_serial(p)?;
            let class=unsafe {&*p}.class();
            let grant=super::super::factory::grant_for(class)?;
            if !Arc::ptr_eq(grant.workspace(),&controls.model)||!grant.owns_allocation(p as usize)? {
                return Err("menu initializer physical allocation differs".into());
            }
            grant.claim_owned_initializer(p as usize)?;
            Some((grant,serial,unsafe {super::super::factory::OwnedReceiver::from_owned(p)}?))
        }else {None};
        let owned_fields = || -> Result<_> {
            let title = title.map(get_string).transpose()?.unwrap_or_default();
            let item = item.map(|(action, key)| get_string(key).map(|key| (action, key))).transpose()?;
            Ok((title, item))
        };
        let (title, item_fields) = match owned_fields() {
            Ok(fields) => fields,
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Err(_) => {
                let title = title.ok_or("Reference startup menu title absent")?;
                let offsets: &[usize] = if item.is_some() { &[0x1459b8] } else { &[0x144b9c] };
                let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
                    &installed.model, caller, offsets,
                );
                check()?;
                let title = unsafe { super::task_objects::reference_string(title)? };
                let item = match item {
                    Some((action, key)) => {
                        let key = unsafe { super::task_objects::reference_string(key)? };
                        let valid = match (title.as_str(), key.as_str(), action.map(|selector| selector.name().to_bytes())) {
                            ("Edit" | "Ruycast", "", None) => true,
                            ("Undo", "z", Some(b"undo:"))
                            | ("Redo", "Z", Some(b"redo:"))
                            | ("Cut", "x", Some(b"cut:"))
                            | ("Copy", "c", Some(b"copy:"))
                            | ("Paste", "v", Some(b"paste:"))
                            | ("Select All", "a", Some(b"selectAll:"))
                            | ("Open Launcher", "", Some(b"openLauncher"))
                            | ("Settings…", "", Some(b"openSettings"))
                            | ("Quit", "", Some(b"quit")) => true,
                            _ => false,
                        };
                        if !valid {
                            return Err("Reference startup menu item differs".into());
                        }
                        Some((action, key))
                    }
                    None if matches!(title.as_str(), "Edit" | "Ruycast") => None,
                    None => return Err("Reference startup menu title differs".into()),
                };
                check()?;
                (title, item)
            }
            #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
            Err(error) => return Err(error),
        };
        let _ = caller;
        let h=allocate(title)?;
        let mut node=PendingMenuNode{controls,handle:h,committed:false};
        // Check the dynamic model guard before the pure node updates. Receiver
        // registration below runs its own verification outside these locks.
        if !std::ptr::eq(state()?,controls) {return Err("menu control context changed".into());}
        if let Some((grant,serial,_))=&claim {
            grant.verify_owned_initializer_claim(p as usize)?;
            if grant.owned_reference_serial(p as usize)?!=*serial||unsafe {&*p}.class()!=grant.class() {
                return Err("menu initializer changed during argument decoding".into());
            }
        }
        {
            let model_state=controls.model.state.lock().map_err(|_|"workspace state poisoned")?;
            if model_state.retired||model_state.generation!=generation||h.generation!=generation
                ||controls.model.faulted.load(Ordering::Acquire) {
                return Err("menu initializer model epoch changed".into());
            }
            let mut nodes=controls.nodes.lock().map_err(|_|"control state poisoned")?;
            let node=nodes.get_mut(&h).ok_or("new menu item node absent")?;
            if let Some((action, key)) = item_fields {
                node.action = action;
                node.key = key;
            }
        }
        let kind = if item.is_some() { Kind::MenuItem(h) } else { Kind::Menu(h) };
        unsafe {register_receiver(p,kind)}?;
        if let Some((_,_,pending))=claim {pending.into_raw();}
        node.committed=true;
        Ok(p)
    })().unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn status_bar(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    object(Kind::StatusBar).unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn status_item(
    p: *mut AnyObject,
    _: Sel,
    length: f64,
) -> *mut AnyObject {
    (|| -> Result<_> {
        if kind(p)? != Kind::StatusBar || !length.is_finite() || length < -2.0 {
            return Err("invalid status item request".into());
        }
        let button = allocate(String::new())?;
        let item = allocate(String::new())?;
        {
            let mut nodes = state()?
                .nodes
                .lock()
                .map_err(|_| "control state poisoned")?;
            let n = nodes.get_mut(&item).unwrap();
            n.button = Some(button);
            n.length = length;
        }
        object(Kind::StatusItem(item))
    })()
    .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn remove_status(p: *mut AnyObject, _: Sel, item: *mut AnyObject) {
    let result = (|| -> Result<()> {
        if kind(p)? != Kind::StatusBar {
            return Err("unknown statusbar".into());
        }
        let h = match kind(item)? {
            Kind::StatusItem(h) => h,
            _ => return Err("unowned status item".into()),
        };
        let button = with(item, |n| {
            n.live = false;
            Ok(n.button)
        })?;
        let mut nodes = state()?
            .nodes
            .lock()
            .map_err(|_| "control state poisoned")?;
        if let Some(button) = button {
            nodes.get_mut(&button).ok_or("status button missing")?.live = false;
        }
        drop(nodes);
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(button) = button {
            let previous = STATUS_IMAGES.with(|slot| {
                slot.try_borrow_mut().map(|mut slot| slot.remove(&button))
                    .map_err(|_| "Reference status image owner busy")
            })?;
            drop(previous);
        }
        let _ = application::engine()?.unregister_target(h);
        Ok(())
    })();
    if result.is_err() {
        fatal();
    }
}
pub(super) unsafe extern "C" fn button(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.button))
        .and_then(|h| object(Kind::Button(h.ok_or("control has no button")?)))
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn title(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.title.clone()))
        .map(string)
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn set_title(p: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    if get_string(value)
        .and_then(|s| {
            with(p, |n| {
                n.title = s;
                Ok(())
            })
        })
        .is_err()
    {
        fatal();
    }
}
pub(super) unsafe extern "C" fn key(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.key.clone()))
        .map(string)
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn tag(p: *mut AnyObject, _: Sel) -> isize {
    with(p, |n| Ok(n.tag)).unwrap_or(0)
}
pub(super) unsafe extern "C" fn set_tag(p: *mut AnyObject, _: Sel, value: isize) {
    if with(p, |n| {
        n.tag = value;
        Ok(())
    })
    .is_err()
    {
        fatal();
    }
}
pub(super) unsafe extern "C" fn enabled(p: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(with(p, |n| Ok(n.enabled)).unwrap_or(false))
}
pub(super) unsafe extern "C" fn set_enabled(p: *mut AnyObject, _: Sel, value: Bool) {
    if with(p, |n| {
        n.enabled = value.as_bool();
        Ok(())
    })
    .is_err()
    {
        fatal();
    }
}
pub(super) unsafe extern "C" fn target(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    {
        let Ok((pointer, identity)) = with(p, |n| Ok((n.target, n.target_identity.clone()))) else {
            return std::ptr::null_mut();
        };
        if identity.is_some_and(|identity| identity.verify_pure().is_err()) {
            return std::ptr::null_mut();
        }
        pointer as *mut AnyObject
    }
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    with(p, |n| Ok(n.target as *mut AnyObject)).unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn set_target(p: *mut AnyObject, _: Sel, target: *mut AnyObject) {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let result = (|| -> Result<()> {
        let identity = if target.is_null() {
            None
        } else {
            super::super::factory::observed_receiver_identity(target)?
        };
        let handle = handle(p)?;
        let controls = state()?;
        if let Some(identity) = &identity {
            if !identity.belongs_to(&controls.model) {
                return Err("Reference control target belongs to another workspace".into());
            }
            identity.verify_pure()?;
        }
        let mut nodes = controls.nodes.lock().map_err(|_| "control state poisoned")?;
        let node = nodes.get_mut(&handle).filter(|node| node.live).ok_or("control retired")?;
        // Unobserved AppKit targets keep their existing property behavior.
        // Only a target observed at assignment can enter SDK dispatch.
        node.target = target as usize;
        node.target_identity = identity;
        Ok(())
    })();
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    let result = with(p, |n| { n.target = target as usize; Ok(()) });
    if result.is_err() {
        fatal();
    }
}
pub(super) unsafe extern "C" fn action(p: *mut AnyObject, _: Sel) -> Option<Sel> {
    with(p, |n| Ok(n.action)).unwrap_or(None)
}
pub(super) unsafe extern "C" fn set_action(p: *mut AnyObject, _: Sel, action: Option<Sel>) {
    if with(p, |n| {
        n.action = action;
        Ok(())
    })
    .is_err()
    {
        fatal();
    }
}
pub(super) unsafe extern "C" fn menu_get(p:*mut AnyObject,_:Sel)->*mut AnyObject {
    (||->Result<_>{
        let parent=actor_parent_reference(p)?;
        // Classify the held actual parent before the nodes mutex. kind() can
        // invoke model/source guards and must never run inside with's closure.
        let is_item=matches!(kind(p)?,Kind::MenuItem(_));
        let menu=with(p,|node|Ok(if is_item {node.parent_menu}else{node.menu}))?;
        if let Some(parent)=&parent {
            if parent.as_ptr()?!=p {return Err("menu getter parent changed".into());}
        }
        let result=menu.map(|handle|object(Kind::Menu(handle))).transpose()?
            .unwrap_or(std::ptr::null_mut());
        if let Some(parent)=&parent {
            if parent.as_ptr()?!=p {return Err("menu getter parent changed during return".into());}
        }
        Ok(result)
    })().unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn submenu_get(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.menu))
        .and_then(|h| {
            h.map(|h| object(Kind::Menu(h)))
                .transpose()
                .map(|p| p.unwrap_or(std::ptr::null_mut()))
        })
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn menu_set(p: *mut AnyObject, _: Sel, menu: *mut AnyObject) {
    let result = (|| -> Result<()> {
        let h = if menu.is_null() {
            None
        } else {
            match kind(menu)? {
                Kind::Menu(h) => Some(h),
                _ => return Err("unowned submenu".into()),
            }
        };
        with(p, |n| {
            n.menu = h;
            Ok(())
        })
    })();
    if result.is_err() {
        fatal();
    }
}
pub(super) unsafe extern "C" fn add_item(p: *mut AnyObject, _: Sel, item: *mut AnyObject) {
    if attach_menu_item(p, item).is_err() {
        fatal();
    }
}

fn attach_menu_item(menu: *mut AnyObject, item: *mut AnyObject) -> Result<()> {
    let Kind::Menu(parent) = kind(menu)? else {
        return Err("not a menu".into());
    };
    let Kind::MenuItem(child) = kind(item)? else {
        return Err("unowned menu item".into());
    };
    let controls = state()?;
    let model = controls.model.state.lock().map_err(|_| "workspace state poisoned")?;
    if model.retired || model.generation != parent.generation || model.generation != child.generation
        || controls.model.faulted.load(Ordering::Acquire)
    {
        return Err("menu attachment owner changed".into());
    }
    let mut nodes = controls.nodes.lock().map_err(|_| "control state poisoned")?;
    if !nodes.get(&parent).is_some_and(|node| node.live && !node.items.contains(&child))
        || !nodes.get(&child).is_some_and(|node| node.live && node.parent_menu.is_none())
    {
        return Err("invalid or duplicate menu item".into());
    }
    nodes.get_mut(&parent).unwrap().items.push(child);
    nodes.get_mut(&child).unwrap().parent_menu = Some(parent);
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn add_item_with_title(
    _: *mut AnyObject, _: Sel, _: *mut AnyObject, _: Option<Sel>, _: *mut AnyObject,
) -> *mut AnyObject {
    core::arch::naked_asm!("mov x5, x30", "b {checked}", checked = sym add_item_with_title_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn add_item_with_title_checked(
    menu: *mut AnyObject, _: Sel, title: *mut AnyObject, action: Option<Sel>,
    key: *mut AnyObject, caller: usize,
) -> *mut AnyObject {
    (|| -> Result<_> {
        if !matches!(kind(menu)?, Kind::Menu(_)) {
            return Err("Reference startup item receiver differs".into());
        }
        let controls = state()?;
        let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
            &controls.model, caller, &[0x144eb4],
        );
        check()?;
        if action.map(|selector| selector.name()) != Some(c"terminate:")
            || unsafe { super::task_objects::reference_string(title)? } != "Quit Ruycast"
            || unsafe { super::task_objects::reference_string(key)? } != "q"
        {
            return Err("Reference startup quit item differs".into());
        }
        check()?;
        let item = item("Quit Ruycast", action, "q")?;
        attach_menu_item(menu, item)?;
        Ok(item)
    // Swift discards this selector's return. Refuse a broken startup as
    // explicitly as addItem:, rather than returning an unobserved nil.
    })().unwrap_or_else(|_| fatal())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn menu_identifier(menu: *mut AnyObject, _: Sel) -> *mut AnyObject {
    if !matches!(kind(menu), Ok(Kind::Menu(_))) {
        fatal();
    }
    match with(menu, |node| Ok(node.identifier.clone())) {
        Ok(Some(identifier)) => {
            let value = string(identifier);
            if value.is_null() { fatal(); }
            value
        }
        Ok(None) => std::ptr::null_mut(),
        Err(_) => fatal(),
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_menu_identifier(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_menu_identifier_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_menu_identifier_checked(
    menu: *mut AnyObject, _: Sel, value: *mut AnyObject, caller: usize,
) {
    let result = (|| -> Result<()> {
        if !matches!(kind(menu)?, Kind::Menu(_)) {
            return Err("Reference menu identifier receiver differs".into());
        }
        let controls = state()?;
        let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
            &controls.model, caller, &[0x1448b0],
        );
        check()?;
        let identifier = unsafe { super::task_objects::reference_string(value)? };
        if identifier != "native-edit" {
            return Err("Reference menu identifier differs".into());
        }
        check()?;
        with(menu, |node| {
            if node.title != "Edit" {
                return Err("Reference identifier requires the Edit menu".into());
            }
            node.identifier = Some(identifier);
            Ok(())
        })
    })();
    if result.is_err() {
        fatal();
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn key_modifier_mask(item: *mut AnyObject, _: Sel) -> usize {
    with(item, |node| Ok(node.key_modifier_mask)).unwrap_or_else(|_| fatal())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_key_modifier_mask(item: *mut AnyObject, _: Sel, mask: usize) {
    const COMMAND: usize = 1 << 20;
    const SHIFT: usize = 1 << 17;
    let result = (|| -> Result<()> {
        if !matches!(kind(item)?, Kind::MenuItem(_)) {
            return Err("Reference modifier mask receiver differs".into());
        }
        with(item, |node| {
            let expected = if node.action.is_some_and(|action| action.name() == c"redo:") {
                COMMAND | SHIFT
            } else {
                COMMAND
            };
            if mask != expected {
                return Err("Reference menu modifier mask differs".into());
            }
            node.key_modifier_mask = mask;
            Ok(())
        })
    })();
    if result.is_err() {
        fatal();
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn separator_item(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym separator_item_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn separator_item_checked(
    class: *mut AnyClass, _: Sel, caller: usize,
) -> *mut AnyObject {
    (|| -> Result<_> {
        let controls = state()?;
        if !super::super::factory::class_role_matches(class, c"NSMenuItem", &controls.model)? {
            return Err("Reference separator class differs".into());
        }
        crate::sdk::reference_runtime::verify_startup_control_caller(
            &controls.model, caller, &[0x144a54],
        )?;
        let handle = allocate(String::new())?;
        let mut pending = PendingMenuNode { controls, handle, committed: false };
        {
            let mut nodes = controls.nodes.lock().map_err(|_| "control state poisoned")?;
            let node = nodes.get_mut(&handle).ok_or("Reference separator node absent")?;
            node.separator = true;
            node.enabled = false;
        }
        let result = object(Kind::MenuItem(handle))?;
        pending.committed = true;
        Ok(result)
    })().unwrap_or_else(|_| fatal())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn is_separator_item(item: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(with(item, |node| Ok(node.separator)).unwrap_or_else(|_| fatal()))
}

pub(super) unsafe extern "C" fn items(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.items.clone()))
        .and_then(|hs| array(hs.into_iter().map(Kind::MenuItem)))
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn count(p: *mut AnyObject, _: Sel) -> isize {
    with(p, |n| Ok(n.items.len() as isize)).unwrap_or(0)
}
pub(super) unsafe extern "C" fn item_at(p: *mut AnyObject, _: Sel, index: isize) -> *mut AnyObject {
    if index < 0 {
        return std::ptr::null_mut();
    }
    with(p, |n| Ok(n.items.get(index as usize).copied()))
        .and_then(|h| {
            h.map(|h| object(Kind::MenuItem(h)))
                .transpose()
                .map(|p| p.unwrap_or(std::ptr::null_mut()))
        })
        .unwrap_or(std::ptr::null_mut())
}
pub(super) unsafe extern "C" fn tooltip(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    with(p, |n| Ok(n.tooltip.clone()))
        .ok()
        .flatten()
        .map(string)
        .unwrap_or(std::ptr::null_mut())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe fn reference_status_text(
    button: *mut AnyObject, value: *mut AnyObject, caller: usize, offset: usize, allowed: &[&str],
) -> Result<String> {
    if !matches!(kind(button)?, Kind::Button(_)) {
        return Err("Reference status text receiver differs".into());
    }
    let controls = state()?;
    let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
        &controls.model, caller, &[offset],
    );
    check()?;
    let text = unsafe { super::task_objects::reference_string(value)? };
    if !allowed.contains(&text.as_str()) {
        return Err("Reference status text differs".into());
    }
    check()?;
    Ok(text)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn set_tooltip(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_tooltip_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
pub(super) unsafe extern "C" fn set_tooltip(p: *mut AnyObject, s: Sel, value: *mut AnyObject) {
    unsafe { set_tooltip_checked(p, s, value, 0) }
}

unsafe extern "C" fn set_tooltip_checked(
    p: *mut AnyObject, _: Sel, value: *mut AnyObject, caller: usize,
) {
    let _ = caller;
    let result = if value.is_null() {
        Ok(None)
    } else {
        match get_string(value) {
            Ok(text) => Ok(Some(text)),
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Err(_) => unsafe {
                reference_status_text(p, value, caller, 0x1fca18,
                    &["Ruycast · Running", "Ruycast · Needs attention"]).map(Some)
            },
            #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
            Err(error) => Err(error),
        }
    };
    if result.and_then(|text| with(p, |node| {
        node.tooltip = text;
        Ok(())
    })).is_err() {
        fatal();
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_status_accessibility_label(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_status_accessibility_label_checked);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn set_status_accessibility_label_checked(
    button: *mut AnyObject, _: Sel, value: *mut AnyObject, caller: usize,
) {
    let result = unsafe {
        reference_status_text(button, value, caller, 0x1fca90,
            &["Ruycast, running", "Ruycast, needs attention"])
    }.and_then(|text| with(button, |node| {
        node.accessibility_label = Some(text);
        Ok(())
    }));
    if result.is_err() {
        fatal();
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn status_accessibility_label(button: *mut AnyObject, _: Sel) -> *mut AnyObject {
    match with(button, |node| Ok(node.accessibility_label.clone())) {
        Ok(Some(label)) => {
            let value = string(label);
            if value.is_null() { fatal(); }
            value
        }
        Ok(None) => std::ptr::null_mut(),
        Err(_) => fatal(),
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
thread_local! {
    // The AppKit drawing image is never sent to another thread or held in the
    // shared model mutex. Drops occur after releasing the RefCell borrow.
    static STATUS_IMAGES: std::cell::RefCell<BTreeMap<Handle, Retained<AnyObject>>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn retire_reference_status_images(model: &Arc<Workspace>) -> Result<()> {
    let controls = CONTROLS.get().ok_or("Reference status controls absent")?;
    if !Arc::ptr_eq(&controls.model, model) || controls.owner != std::thread::current().id() {
        return Err("Reference status image teardown owner differs".into());
    }
    let images = STATUS_IMAGES.with(|slot| {
        slot.try_borrow_mut().map(|mut slot| std::mem::take(&mut *slot))
            .map_err(|_| "Reference status image teardown busy")
    })?;
    drop(images);
    Ok(())
}

pub(super) unsafe extern "C" fn image(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if let Ok(Kind::Button(handle)) = kind(p) {
        if with(p, |_| Ok(())).is_err() { return std::ptr::null_mut(); }
        if let Some(pointer) = STATUS_IMAGES.with(|slot| {
            slot.try_borrow().ok()?.get(&handle).map(Retained::as_ptr)
        }) {
            return pointer.cast_mut();
        }
    }
    with(p, |n| Ok(n.image as *mut AnyObject)).unwrap_or(std::ptr::null_mut())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
pub(super) unsafe extern "C" fn set_image(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_image_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
pub(super) unsafe extern "C" fn set_image(p: *mut AnyObject, s: Sel, image: *mut AnyObject) {
    unsafe { set_image_checked(p, s, image, 0) }
}

unsafe extern "C" fn set_image_checked(
    p: *mut AnyObject, _: Sel, image: *mut AnyObject, caller: usize,
) {
    let _ = caller;
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    if let Ok(Kind::Button(handle)) = kind(p) {
        let result = (|| -> Result<()> {
            let controls = state()?;
            let check = || crate::sdk::reference_runtime::verify_startup_control_caller(
                &controls.model, caller, &[0x1fc9a0],
            );
            check()?;
            let object = unsafe { image.as_ref() }.ok_or("Reference status image absent")?;
            let class = crate::sdk::metadata::loaded_class(c"NSImage")?;
            if object.class() as *const AnyClass as usize != class.address() {
                return Err("Reference status image class differs".into());
            }
            let held = unsafe { Retained::retain(image) }.ok_or("Reference status image retain failed")?;
            check()?;
            if !matches!(kind(p)?, Kind::Button(current) if current == handle) {
                return Err("Reference status button changed during image retain".into());
            }
            with(p, |_| Ok(()))?;
            let previous = STATUS_IMAGES.with(|slot| {
                slot.try_borrow_mut().map(|mut slot| slot.insert(handle, held))
                    .map_err(|_| "Reference status image owner busy")
            })?;
            drop(previous);
            Ok(())
        })();
        if result.is_err() { fatal(); }
        return;
    }
    if with(p, |n| {
        n.image = image as usize;
        Ok(())
    }).is_err() {
        fatal();
    }
}
pub(super) unsafe extern "C" fn state_get(p: *mut AnyObject, _: Sel) -> isize {
    with(p, |n| Ok(n.state)).unwrap_or(0)
}
pub(super) unsafe extern "C" fn state_set(p: *mut AnyObject, _: Sel, value: isize) {
    if with(p, |n| {
        n.state = value;
        Ok(())
    })
    .is_err()
    {
        fatal();
    }
}
pub(crate) fn verify_action_control(handle: Handle) -> Result<()> {
    let controls = CONTROLS.get().ok_or("controls absent")?;
    if std::thread::current().id() != controls.owner {
        return Err("queued action on foreign control owner".into());
    }
    if controls.model.state.lock().map_err(|_| "workspace state poisoned")?.generation != handle.generation {
        return Err("queued action control generation changed".into());
    }
    let nodes = controls.nodes.lock().map_err(|_| "control state poisoned")?;
    if !nodes.get(&handle).is_some_and(|node| node.live) {
        return Err("queued action control retired".into());
    }
    Ok(())
}
/// # Safety
/// p is a live owned initialized receiver whose allocation/destruction grant
/// remains valid through sender retention and callback admission.
pub unsafe fn trigger(p: *mut AnyObject) -> Result<()> {
    let h = handle(p)?;
    let (target, selector) = with(p, |n| {
        if !n.enabled {
            return Err("control disabled".into());
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let target = n.target_identity.clone();
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let target = n.target;
        Ok((target, n.action))
    })?;
    let selector = selector.ok_or("control action absent")?;
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    if target == 0 {
        return Err("control target absent".into());
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let target = target.ok_or("Reference control target has no observed allocation")?;
    let app = unsafe { application::shared(std::ptr::null_mut(), objc2::sel!(sharedApplication)) };
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let callback = application::selected_control_callback(&target, app)?;
    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
    let callback = application::callback(target as *mut AnyObject, app)?;
    let engine = application::engine()?;
    // Sender-bearing dispatch is completed only with the receiver factory's
    // genuine initialized sender lease. No arbitrary pointer/nil is fabricated.
    let grant = super::super::factory::grant_for(unsafe { &*p }.class())?;
    let sender = unsafe { grant.sender(p) }?;
    let controls = state()?;
    engine.enqueue_checked(Event::ActionWithSender {
        target: h,
        callback,
        selector: selector.name().to_string_lossy().into_owned(),
        sender,
    }, || {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        target.verify_pure()?;
        if controls.model.state.lock().map_err(|_| "workspace state poisoned")?.generation != h.generation {
            return Err("control generation changed during action qualification".into());
        }
        let nodes = controls.nodes.lock().map_err(|_| "control state poisoned")?;
        let node = nodes.get(&h).filter(|node| node.live && node.enabled)
            .ok_or("control retired or disabled during action qualification")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let same = node.target_identity.as_ref().is_some_and(|current| {
            current.receiver() == target.receiver()
                && current.allocation_serial() == target.allocation_serial()
        });
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let same = node.target == target;
        if !same || node.action != Some(selector) {
            return Err("control target or action changed during qualification".into());
        }
        Ok(())
    })
}
pub(super) unsafe extern "C" fn click(p: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if unsafe { trigger(p) }.is_err() {
        fatal();
    }
}

#[test]
#[ignore = "isolated AppKit lifecycle transaction with process-wide class mutations"]
fn queued_menu_actions_keep_their_original_target_and_sender() {
    use super::super::event_loop::{Callback, Lifecycle};
    use objc2::rc::Retained;
    use objc2_foundation::NSObject;

    struct Record {
        calls: Arc<Mutex<Vec<(usize, String, usize)>>>,
        id: usize,
    }
    impl Callback for Record {
        fn lifecycle(&self, _: Lifecycle) -> Result<()> { Ok(()) }
        fn action(&self, _: &str) -> Result<()> { Err("fixture action requires a sender".into()) }
        fn action_with_sender(
            &self, control: Handle, selector: &str, sender: &super::super::receiver::SenderLease,
        ) -> Result<()> {
            verify_action_control(control)?;
            self.calls.lock().unwrap().push((self.id, selector.into(), sender.pointer()? as usize));
            Ok(())
        }
    }

    let fixture = super::super::tests::Fixture::new();
    let model = fixture.model.clone();
    unsafe { super::prepare_application_window_control_factories_checked(model.clone(), || Ok(())) }
        .unwrap();
    super::install(model).unwrap();
    let engine = application::engine().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(Record { calls: calls.clone(), id: 1 });
    let second = Arc::new(Record { calls: calls.clone(), id: 2 });
    engine.set_delegate(first.clone()).unwrap();
    engine.launch().unwrap();

    let first_target = NSObject::new();
    let second_target = NSObject::new();
    let first_pointer = Retained::as_ptr(&first_target).cast_mut().cast::<AnyObject>();
    let second_pointer = Retained::as_ptr(&second_target).cast_mut().cast::<AnyObject>();
    application::own_fixture_delegate(first_pointer, first).unwrap();
    application::own_fixture_delegate(second_pointer, second).unwrap();

    let first_action = Sel::register(c"fixtureFirst:");
    let second_action = Sel::register(c"fixtureSecond:");
    let menu_item = item("Fixture", Some(first_action), "").unwrap();
    unsafe {
        set_target(menu_item, Sel::register(c"setTarget:"), first_pointer);
        trigger(menu_item).unwrap();
        set_target(menu_item, Sel::register(c"setTarget:"), second_pointer);
        set_action(menu_item, Sel::register(c"setAction:"), Some(second_action));
        trigger(menu_item).unwrap();
    }
    assert!(engine.pump().unwrap());
    assert!(engine.pump().unwrap());
    assert_eq!(*calls.lock().unwrap(), [
        (1, "fixtureFirst:".into(), menu_item as usize),
        (2, "fixtureSecond:".into(), menu_item as usize),
    ]);
    engine.enqueue(Event::Terminate).unwrap();
    assert!(engine.pump().unwrap());
    drop(unsafe { Retained::from_raw(menu_item) });
}

#[cfg(test)]
pub(super) fn add_methods(b: &mut objc2::runtime::ClassBuilder) {
    unsafe {
        b.add_method(
            objc2::sel!(button),
            button as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(title),
            title as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setTitle:),
            set_title as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(keyEquivalent),
            key as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(tag),
            tag as unsafe extern "C" fn(*mut AnyObject, Sel) -> isize,
        );
        b.add_method(
            objc2::sel!(setTag:),
            set_tag as unsafe extern "C" fn(*mut AnyObject, Sel, isize),
        );
        b.add_method(
            objc2::sel!(isEnabled),
            enabled as unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
        );
        b.add_method(
            objc2::sel!(setEnabled:),
            set_enabled as unsafe extern "C" fn(*mut AnyObject, Sel, Bool),
        );
        b.add_method(
            objc2::sel!(target),
            target as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setTarget:),
            set_target as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(action),
            action as unsafe extern "C" fn(*mut AnyObject, Sel) -> Option<Sel>,
        );
        b.add_method(
            objc2::sel!(setAction:),
            set_action as unsafe extern "C" fn(*mut AnyObject, Sel, Option<Sel>),
        );
        b.add_method(
            objc2::sel!(menu),
            menu_get as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setMenu:),
            menu_set as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(submenu),
            submenu_get as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setSubmenu:),
            menu_set as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(addItem:),
            add_item as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(itemArray),
            items as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(numberOfItems),
            count as unsafe extern "C" fn(*mut AnyObject, Sel) -> isize,
        );
        b.add_method(
            objc2::sel!(itemAtIndex:),
            item_at as unsafe extern "C" fn(*mut AnyObject, Sel, isize) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(toolTip),
            tooltip as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setToolTip:),
            set_tooltip as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(image),
            image as unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject,
        );
        b.add_method(
            objc2::sel!(setImage:),
            set_image as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
        b.add_method(
            objc2::sel!(state),
            state_get as unsafe extern "C" fn(*mut AnyObject, Sel) -> isize,
        );
        b.add_method(
            objc2::sel!(setState:),
            state_set as unsafe extern "C" fn(*mut AnyObject, Sel, isize),
        );
        b.add_method(
            objc2::sel!(performClick:),
            click as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }
}
pub fn replacements() -> Vec<(&'static CStr, &'static CStr, bool, usize)> {
    let mut rows = vec![
        (
            c"NSStatusBar",
            c"systemStatusBar",
            true,
            status_bar as *const () as usize,
        ),
        (
            c"NSStatusBar",
            c"statusItemWithLength:",
            false,
            status_item as *const () as usize,
        ),
        (
            c"NSStatusBar",
            c"removeStatusItem:",
            false,
            remove_status as *const () as usize,
        ),
        (
            c"NSStatusItem",
            c"button",
            false,
            button as *const () as usize,
        ),
    ];
    for class in [
        c"NSMenu",
        c"NSMenuItem",
        c"NSStatusItem",
        c"NSStatusBarButton",
    ] {
        for (selector, pointer) in [
            (c"title", title as *const () as usize),
            (c"setTitle:", set_title as *const () as usize),
        ] {
            rows.push((class, selector, false, pointer));
        }
    }
    for class in [c"NSMenuItem", c"NSStatusBarButton"] {
        for (selector, pointer) in [
            (c"target", target as *const () as usize),
            (c"setTarget:", set_target as *const () as usize),
            (c"action", action as *const () as usize),
            (c"setAction:", set_action as *const () as usize),
            (c"tag", tag as *const () as usize),
            (c"setTag:", set_tag as *const () as usize),
            (c"isEnabled", enabled as *const () as usize),
            (c"setEnabled:", set_enabled as *const () as usize),
            (c"image", image as *const () as usize),
            (c"setImage:", set_image as *const () as usize),
            (c"state", state_get as *const () as usize),
            (c"setState:", state_set as *const () as usize),
            (c"toolTip", tooltip as *const () as usize),
            (c"setToolTip:", set_tooltip as *const () as usize),
        ] {
            rows.push((class, selector, false, pointer));
        }
    }
    rows.extend([
        (
            c"NSMenu",
            c"init",
            false,
            init_menu_default as *const () as usize,
        ),
        (
            c"NSMenu",
            c"initWithTitle:",
            false,
            init_menu as *const () as usize,
        ),
        (
            c"NSMenuItem",
            c"initWithTitle:action:keyEquivalent:",
            false,
            init_item as *const () as usize,
        ),
        (
            c"NSMenuItem",
            c"menu",
            false,
            menu_get as *const () as usize,
        ),
        (
            c"NSStatusItem",
            c"menu",
            false,
            menu_get as *const () as usize,
        ),
        (
            c"NSStatusItem",
            c"setMenu:",
            false,
            menu_set as *const () as usize,
        ),
        (
            c"NSStatusBarButton",
            c"menu",
            false,
            menu_get as *const () as usize,
        ),
        (
            c"NSStatusBarButton",
            c"setMenu:",
            false,
            menu_set as *const () as usize,
        ),
        (
            c"NSMenu",
            c"addItem:",
            false,
            add_item as *const () as usize,
        ),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenu", c"addItemWithTitle:action:keyEquivalent:", false, add_item_with_title as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenu", c"identifier", false, menu_identifier as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenu", c"setIdentifier:", false, set_menu_identifier as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenuItem", c"keyEquivalentModifierMask", false, key_modifier_mask as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenuItem", c"setKeyEquivalentModifierMask:", false, set_key_modifier_mask as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenuItem", c"separatorItem", true, separator_item as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSMenuItem", c"isSeparatorItem", false, is_separator_item as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSStatusBarButton", c"accessibilityLabel", false, status_accessibility_label as *const () as usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        (c"NSStatusBarButton", c"setAccessibilityLabel:", false, set_status_accessibility_label as *const () as usize),
        (c"NSMenu", c"itemArray", false, items as *const () as usize),
        (
            c"NSMenu",
            c"numberOfItems",
            false,
            count as *const () as usize,
        ),
        (
            c"NSMenu",
            c"itemAtIndex:",
            false,
            item_at as *const () as usize,
        ),
        (
            c"NSMenuItem",
            c"keyEquivalent",
            false,
            key as *const () as usize,
        ),
        (
            c"NSMenuItem",
            c"submenu",
            false,
            submenu_get as *const () as usize,
        ),
        (
            c"NSMenuItem",
            c"setSubmenu:",
            false,
            menu_set as *const () as usize,
        ),
        (
            c"NSStatusBarButton",
            c"performClick:",
            false,
            click as *const () as usize,
        ),
    ]);
    rows
}

#[cfg(any(test, rcf1_reference_sdk))]
fn control_signature(selector: &CStr) -> Result<Vec<objc2::encode::Encoding>> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    use objc2::runtime::MethodImplementation;
    fn signature<F: MethodImplementation>(_: F) -> Vec<Encoding> {
        let mut result = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
        result.extend_from_slice(F::Arguments::ENCODINGS);
        result
    }
    type Receiver = *mut AnyObject;
    type GetObject = unsafe extern "C" fn(Receiver, Sel) -> Receiver;
    type SetObject = unsafe extern "C" fn(Receiver, Sel, Receiver);
    type GetInteger = unsafe extern "C" fn(Receiver, Sel) -> isize;
    type SetInteger = unsafe extern "C" fn(Receiver, Sel, isize);
    Ok(match selector.to_bytes() {
        b"systemStatusBar" => signature(status_bar as GetObject),
        b"statusItemWithLength:" => signature(status_item as unsafe extern "C" fn(Receiver, Sel, f64) -> Receiver),
        b"removeStatusItem:" => signature(remove_status as SetObject),
        b"button" => signature(button as GetObject),
        b"title" => signature(title as GetObject),
        b"setTitle:" => signature(set_title as SetObject),
        b"target" => signature(target as GetObject),
        b"setTarget:" => signature(set_target as SetObject),
        b"action" => signature(action as unsafe extern "C" fn(Receiver, Sel) -> Option<Sel>),
        b"setAction:" => signature(set_action as unsafe extern "C" fn(Receiver, Sel, Option<Sel>)),
        b"tag" => signature(tag as GetInteger),
        b"setTag:" => signature(set_tag as SetInteger),
        b"isEnabled" => signature(enabled as unsafe extern "C" fn(Receiver, Sel) -> Bool),
        b"setEnabled:" => signature(set_enabled as unsafe extern "C" fn(Receiver, Sel, Bool)),
        b"image" => signature(image as GetObject),
        b"setImage:" => signature(set_image as SetObject),
        b"state" => signature(state_get as GetInteger),
        b"setState:" => signature(state_set as SetInteger),
        b"toolTip" => signature(tooltip as GetObject),
        b"setToolTip:" => signature(set_tooltip as SetObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"accessibilityLabel" => signature(status_accessibility_label as GetObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"setAccessibilityLabel:" => signature(set_status_accessibility_label as SetObject),
        b"init" => signature(init_menu_default as GetObject),
        b"initWithTitle:" => signature(init_menu as unsafe extern "C" fn(Receiver, Sel, Receiver) -> Receiver),
        b"initWithTitle:action:keyEquivalent:" =>
            signature(init_item as unsafe extern "C" fn(Receiver, Sel, Receiver, Option<Sel>, Receiver) -> Receiver),
        b"menu" => signature(menu_get as GetObject),
        b"setMenu:" | b"setSubmenu:" => signature(menu_set as SetObject),
        b"addItem:" => signature(add_item as SetObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"addItemWithTitle:action:keyEquivalent:" =>
            signature(add_item_with_title as unsafe extern "C" fn(Receiver, Sel, Receiver, Option<Sel>, Receiver) -> Receiver),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"identifier" => signature(menu_identifier as GetObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"setIdentifier:" => signature(set_menu_identifier as SetObject),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"keyEquivalentModifierMask" => signature(key_modifier_mask as unsafe extern "C" fn(Receiver, Sel) -> usize),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"setKeyEquivalentModifierMask:" => signature(set_key_modifier_mask as unsafe extern "C" fn(Receiver, Sel, usize)),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"separatorItem" => signature(separator_item as unsafe extern "C" fn(*mut AnyClass, Sel) -> Receiver),
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        b"isSeparatorItem" => signature(is_separator_item as unsafe extern "C" fn(Receiver, Sel) -> Bool),
        b"itemArray" => signature(items as GetObject),
        b"numberOfItems" => signature(count as GetInteger),
        b"itemAtIndex:" => signature(item_at as unsafe extern "C" fn(Receiver, Sel, isize) -> Receiver),
        b"keyEquivalent" => signature(key as GetObject),
        b"submenu" => signature(submenu_get as GetObject),
        b"performClick:" => signature(click as SetObject),
        _ => return Err(format!("untyped control selector {selector:?}")),
    })
}

#[cfg(any(test, rcf1_reference_sdk))]
pub(super) fn preflight_control_methods() -> Result<()> {
    let mut failures = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (role, selector, meta, _) in replacements() {
        if !seen.insert((role, selector, meta)) {
            return Err(format!("duplicate control method {role:?}/{selector:?}/{meta}"));
        }
        let result = (|| -> Result<()> {
            let class = crate::sdk::metadata::loaded_class(role)?;
            let method = crate::sdk::metadata::method(class, selector, meta)?
                .ok_or("source control method absent")?;
            super::super::classes::verify_typed_abi(&method, &control_signature(selector)?)
        })();
        if let Err(error) = result {
            failures.push(format!("{role:?}/{selector:?}/{meta}: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[cfg(any(test, rcf1_reference_sdk))]
pub(super) const FACTORY_ROLES: [(&CStr, &CStr); 5] = [
    (c"NSStatusBar", c"RuycastF1OwnedStatusBarSource"),
    (c"NSStatusItem", c"RuycastF1OwnedStatusItemSource"),
    (c"NSStatusBarButton", c"RuycastF1OwnedStatusButtonSource"),
    (c"NSMenu", c"RuycastF1OwnedMenuSource"),
    (c"NSMenuItem", c"RuycastF1OwnedMenuItemSource"),
];

#[cfg(any(test, rcf1_reference_sdk))]
pub(super) unsafe fn prepare_factories(
    model: &Arc<Workspace>,
    parents: [&'static AnyClass; 5],
    guard: &super::super::classes::ClassGuard,
) -> Result<()> {
    let methods = replacements();
    for ((role, name), parent) in FACTORY_ROLES.into_iter().zip(parents) {
        if parent.name() != role {
            return Err("control factory parent differs".into());
        }
        let proxy = unsafe { guard.subclass_qualified(name, parent) }?;
        let mut bindings = guard.lifecycle_bindings_for(parent)?;
        if role == c"NSStatusBarButton" {
            bindings.extend(unsafe { guard.bind_preserved_appkit_proxy_lifecycle(proxy, deallocate) }?);
        }
        for &(_, selector, meta, implementation) in methods.iter().filter(|row| row.0 == role) {
            let leaf = unsafe {
                guard.overlay_bound(
                    if meta { parent } else { proxy },
                    selector,
                    meta,
                    implementation,
                    &control_signature(selector)?,
                )
            }?;
            bindings.push(super::super::receiver::LifecycleBinding::leaf(leaf));
        }
        let grant = unsafe {
            super::super::receiver::WorkspaceGrant::for_owned_sdk(model.clone(), proxy, bindings)
        }?;
        super::super::factory::publish_proxy(role, Arc::new(grant))?;
    }
    Ok(())
}

#[cfg(any(test, rcf1_reference_sdk))]
pub(super) unsafe extern "C" fn deallocate(pointer: *mut AnyObject, _: Sel) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        let installed = INSTALLED.get().ok_or("workspace absent during control cleanup")?;
        let role = super::super::factory::receiver_role(pointer, &installed.model)?;
        if !FACTORY_ROLES.iter().any(|(class, _)| *class == role) {
            return Err("foreign control cleanup role".into());
        }
        let serial = super::super::factory::owned_reference_serial(pointer)?;
        let expected = {
            let objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
            let record = objects.get(&(pointer as usize));
            let valid = match record {
                None => true,
                Some(Kind::StatusBar) => role == c"NSStatusBar",
                Some(Kind::StatusItem(_)) => role == c"NSStatusItem",
                Some(Kind::Button(_)) => role == c"NSStatusBarButton",
                Some(Kind::Menu(_)) => role == c"NSMenu",
                Some(Kind::MenuItem(_)) => role == c"NSMenuItem",
                _ => false,
            };
            if !valid {
                return Err("control cleanup receiver kind differs".into());
            }
            record.cloned()
        };
        if super::super::factory::owned_reference_serial(pointer)? != serial {
            return Err("control allocation changed before cleanup".into());
        }
        unsafe {
            super::super::factory::deallocate_owned(pointer, || {
                let mut objects = installed.objects.lock().map_err(|_| "object arena poisoned")?;
                if objects.get(&(pointer as usize)) != expected.as_ref() {
                    return Err("control arena identity changed during cleanup".into());
                }
                // Handles belong to the workspace model and can still be
                // referenced by another control after this receiver dies.
                objects.remove(&(pointer as usize));
                Ok(())
            })
        }
    }));
    let failed = match result {
        Ok(result) => result.is_err(),
        Err(payload) => {
            std::mem::forget(payload);
            true
        }
    };
    if failed {
        if let Some(installed) = INSTALLED.get() {
            installed.model.fault();
        }
    }
}

#[cfg(test)]
#[test]
fn control_methods_match_loaded_appkit_without_initializing_controls() {
    preflight_control_methods().unwrap();
    assert!(CONTROLS.get().is_none());
}
