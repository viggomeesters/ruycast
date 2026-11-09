//! A wholly owned notification center; no call to defaultCenter, mainQueue or
//! NSWorkspace. Copies held by a delivery retain callbacks across unregister.
use super::*;
use block2::{Block, RcBlock};
#[cfg(test)]
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
#[cfg(test)]
use objc2_foundation::NSString;
use objc2_foundation::{NSDictionary, NSNotification};
use std::thread::{self, ThreadId};
pub type NotificationReply = Block<dyn Fn(*mut AnyObject)>;
/// Exact current source declarations, not image/callback authority or admission.
pub fn source_roster() -> std::collections::BTreeSet<crate::sdk::SourceCallsite> {
    [
        ("frozen-current/rust-native/src/keyboard_environment.rs", "913e4dc6c2ceb8a2e991ea040956c9ccaf517b93c1198035780d64ba26c4ce9e", "lines 64-114: owned/private or NSWorkspace notificationCenter; addObserverForName:object:queue:usingBlock: with NSOperationQueue.mainQueue"),
        ("frozen-current/rust-native/src/keyboard_environment.rs", "913e4dc6c2ceb8a2e991ea040956c9ccaf517b93c1198035780d64ba26c4ce9e", "lines 203-211: Environment::drop removeObserver:"),
        ("frozen-current/rust-native/src/native.rs", "b3a7e47cadfe8816cca351c6c456048b7eb4a3e9cdf77d679a998002a58b4387", "lines 869-875: defaultCenter managementResized: object=panel; workspace refreshMenuHealth: launch/terminate object=nil"),
        ("frozen-current/rust-native/src/native.rs", "b3a7e47cadfe8816cca351c6c456048b7eb4a3e9cdf77d679a998002a58b4387", "lines 883-890: defaultCenter inlineScrolled: object=scroll.contentView"),
        ("frozen-current/rust-native/src/workspace_activation.rs", "618d66bde54c6723efaa747e6284995269133d9b9796beaf5fb8ed4e8436c768", "lines 61-101: addObserverForName:object:queue:usingBlock:; original notification userInfo application payload"),
        ("frozen-current/rust-native/src/workspace_activation.rs", "618d66bde54c6723efaa747e6284995269133d9b9796beaf5fb8ed4e8436c768", "lines 106-115: Watch::drop removeObserver:"),
    ].into_iter().map(|(file, hash, operation)| crate::sdk::SourceCallsite { file, source_sha256: hash.into(), operation }).collect()
}
enum Delivery {
    Block(usize),
    Selector(Arc<delegate::ObjectiveC>, String),
}

struct Observer {
    delivery: Delivery,
    object: Option<NotificationFilter>,
    copy_borrows: usize,
    retired: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NotificationFilter {
    Object(usize),
    Window { pointer: usize, handle: Handle },
}

impl NotificationFilter {
    fn matches(self, pointer: Option<usize>, window: Option<Handle>) -> bool {
        match self {
            Self::Object(expected) => pointer == Some(expected),
            Self::Window { pointer: expected, handle } => pointer == Some(expected) && window == Some(handle),
        }
    }
}
const MAX_COPY_BORROWS: usize = 1024;

/// Holds the map's actual owned block reference through a foreign copy. The
/// original cannot be released by reentrant unsubscribe until this borrow ends.
/// It never moves off the creating owner thread, including during unwind.
struct CopyBorrow<'a> {
    hub: &'a Hub,
    id: u64,
    block: usize,
    active: bool,
    _main: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl<'a> CopyBorrow<'a> {
    fn reserve(hub: &'a Hub, id: u64, observer: &mut Observer) -> Result<Self> {
        let Delivery::Block(block) = observer.delivery else {
            return Err("notification copy requires a block owner".into());
        };
        if observer.retired || observer.copy_borrows >= MAX_COPY_BORROWS {
            return Err("notification copy custody limit/retirement".into());
        }
        observer.copy_borrows += 1;
        Ok(Self {
            hub,
            id,
            block,
            active: true,
            _main: std::marker::PhantomData,
        })
    }
    fn finish(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.hub.check_owner()?;
        let removed = {
            let mut observers = self
                .hub
                .observers
                .lock()
                .map_err(|_| "notification hub poisoned")?;
            let observer = observers
                .get_mut(&self.id)
                .ok_or("notification copy owner absent")?;
            if !matches!(observer.delivery,Delivery::Block(block) if block==self.block)
                || observer.copy_borrows == 0
            {
                return Err("notification copy custody changed".into());
            }
            observer.copy_borrows -= 1;
            let remove = observer.retired && observer.copy_borrows == 0;
            self.active = false;
            if remove {
                observers.remove(&self.id)
            } else {
                None
            }
        };
        // Final block disposal may invoke capture cleanup; no map lock remains.
        release_observer(removed)
    }
}
impl Drop for CopyBorrow<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.finish()));
        match outcome {
            Ok(Ok(())) => (),
            outcome => {
                self.hub
                    .model
                    .faulted
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Err(payload) = outcome {
                    std::mem::forget(payload);
                }
            }
        }
    }
}
fn release_observer(observer: Option<Observer>) -> Result<()> {
    if let Some(Observer {
        delivery: Delivery::Block(block),
        ..
    }) = observer
    {
        let held = unsafe { RcBlock::from_raw(block as *mut NotificationReply) }
            .ok_or("lost owned notification block")?;
        drop(held);
    }
    Ok(())
}
enum PendingCopy<'a> {
    Block(CopyBorrow<'a>),
    Selector(Arc<delegate::ObjectiveC>, String),
}
pub struct Hub {
    model: Arc<Workspace>,
    owner: ThreadId,
    #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
    deliveries: std::sync::atomic::AtomicUsize,
    observers: Mutex<BTreeMap<u64, Observer>>,
}
impl Hub {
    fn check_owner(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err("notification operation on foreign owner thread".into());
        }
        Ok(())
    }
    pub fn new(model: Arc<Workspace>) -> Result<Self> {
        model.with(|_| Ok(()))?;
        Ok(Self {
            model,
            owner: thread::current().id(),
            #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
            deliveries: std::sync::atomic::AtomicUsize::new(0),
            observers: Mutex::new(BTreeMap::new()),
        })
    }
    /// # Safety
    /// A valid live notification block with one object argument. Only the owned
    /// endpoint thread pumps it; no SDK or real center retains this callback.
    pub unsafe fn subscribe(&self, name: &str, block: *mut NotificationReply) -> Result<u64> {
        unsafe { self.subscribe_filtered(name, block, None) }
    }
    /// # Safety
    /// Same as subscribe. A non-null filter is an owned object whose identity
    /// remains live until unsubscribe; the hub never messages that object.
    pub unsafe fn subscribe_filtered(
        &self,
        name: &str,
        block: *mut NotificationReply,
        object: Option<usize>,
    ) -> Result<u64> {
        unsafe { self.subscribe_with_token(name, block, object.map(NotificationFilter::Object), Ok) }
    }

    /// The block remains local and cannot be delivered until token creation succeeds.
    /// The caller must satisfy the same block and filter requirements as subscribe_filtered.
    pub(super) unsafe fn subscribe_with_token<T>(
        &self,
        name: &str,
        block: *mut NotificationReply,
        object: Option<NotificationFilter>,
        token: impl FnOnce(u64) -> Result<T>,
    ) -> Result<T> {
        self.check_owner()?;
        self.model.with(|_| Ok(()))?;
        if matches!(object, Some(NotificationFilter::Object(0) | NotificationFilter::Window { pointer: 0, .. })) {
            return Err("null notification object filter".into());
        }
        if block.is_null() {
            return Err("null notification callback".into());
        }
        let mut held = Some(unsafe { RcBlock::copy(block) }.ok_or("notification retain failed")?);
        let id = self.model.subscribe(name)?;
        struct Pending<'a> {
            model: &'a Workspace,
            id: Option<u64>,
        }
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                if let Some(id) = self.id {
                    self.model.discard_pending_notification(id);
                }
            }
        }
        let mut pending = Pending { model: &self.model, id: Some(id) };
        let token = token(id)?;
        self.model.with(|state| {
            if !state.notifications.iter().any(|(candidate, registered, generation)| {
                *candidate == id && registered == name && *generation == state.generation
            }) {
                return Err("pending notification changed during token creation".into());
            }
            let mut observers = self.observers.lock().map_err(|_| "notification hub poisoned")?;
            if self.model.faulted.load(std::sync::atomic::Ordering::Acquire) {
                return Err("workspace authority retired during notification commit".into());
            }
            let held = held.take().ok_or("pending notification lost its block")?;
            observers.insert(
                id,
                Observer {
                    delivery: Delivery::Block(RcBlock::into_raw(held) as usize),
                    object,
                    copy_borrows: 0,
                    retired: false,
                },
            );
            Ok(())
        })?;
        pending.id = None;
        Ok(token)
    }
    /// Selected target carries the existing callback code/source authority;
    /// there is no second selector resolver or permissive method fallback.
    /// # Safety
    /// Any object filter obeys the same owned lifetime as subscribe_filtered.
    pub unsafe fn subscribe_selector(
        &self,
        name: &str,
        target: Arc<delegate::ObjectiveC>,
        selector: &str,
        object: Option<usize>,
    ) -> Result<u64> {
        self.check_owner()?;
        if object == Some(0) {
            return Err("null notification object filter".into());
        }
        target.accepts_notification(&self.model, selector)?;
        let id = self.model.subscribe(name)?;
        self.observers
            .lock()
            .map_err(|_| "notification hub poisoned")?
            .insert(
                id,
                Observer {
                    delivery: Delivery::Selector(target, selector.into()),
                    object: object.map(NotificationFilter::Object),
                    copy_borrows: 0,
                    retired: false,
                },
            );
        Ok(id)
    }
    pub fn unsubscribe(&self, id: u64) -> Result<()> {
        self.check_owner()?;
        self.model.with(|_| Ok(()))?;
        if self
            .observers
            .lock()
            .map_err(|_| "notification hub poisoned")?
            .get(&id)
            .is_none_or(|observer| observer.retired)
        {
            return Err("observer belongs to another hub or is unknown".into());
        }
        self.model.unsubscribe(id)?;
        let removed = {
            let mut observers = self
                .observers
                .lock()
                .map_err(|_| "notification hub poisoned")?;
            let observer = observers.get_mut(&id).ok_or("unknown notification token")?;
            observer.retired = true;
            if observer.copy_borrows == 0 {
                observers.remove(&id)
            } else {
                None
            }
        };
        // A copy still in progress owns a map borrow. It will remove the
        // tombstone and release this original reference after the copy returns.
        release_observer(removed)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn remove_reference_block(&self, id: u64) -> Result<()> {
        self.check_owner()?;
        if !self.observers.lock().map_err(|_| "notification hub poisoned")?
            .get(&id).is_some_and(|observer| {
                !observer.retired && observer.copy_borrows == 0
                    && matches!(observer.delivery, Delivery::Block(_))
            })
        {
            return Err("Original HyperKey observer is absent, borrowed or not a block".into());
        }
        self.unsubscribe(id)?;
        if self.observers.lock().map_err(|_| "notification hub poisoned")?.contains_key(&id) {
            return Err("Original HyperKey observer block is still held".into());
        }
        Ok(())
    }
    pub(super) fn unsubscribe_if_present(&self, id: u64) -> Result<()> {
        self.check_owner()?;
        self.model.with(|_| Ok(()))?;
        let present = self
            .observers
            .lock()
            .map_err(|_| "notification hub poisoned")?
            .get(&id)
            .is_some_and(|observer| !observer.retired);
        if present {
            self.unsubscribe(id)?;
        }
        Ok(())
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_owned_block(&self, id: u64, filter: Option<NotificationFilter>) -> Result<()> {
        self.check_owner()?;
        if self.deliveries.load(Ordering::Relaxed) != 0 {
            return Err("Reference notification delivery still in flight".into());
        }
        let observers = self.observers.lock().map_err(|_| "notification hub poisoned")?;
        if !observers.get(&id).is_some_and(|observer| {
            !observer.retired && observer.copy_borrows == 0 && observer.object == filter
                && matches!(observer.delivery, Delivery::Block(_))
        }) {
            return Err("Reference SnippetChoice observer differs or is borrowed".into());
        }
        Ok(())
    }

    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn verify_block_removed(&self, id: u64) -> Result<()> {
        self.check_owner()?;
        if self.deliveries.load(Ordering::Relaxed) != 0
            || self.observers.lock().map_err(|_| "notification hub poisoned")?.contains_key(&id)
        {
            return Err("Reference SnippetChoice observer did not drain".into());
        }
        Ok(())
    }

    pub(super) fn has_recipient_for_window(&self, ids: &[u64], object: usize, window: Handle) -> Result<bool> {
        self.check_owner()?;
        let observers = self.observers.lock().map_err(|_| "notification hub poisoned")?;
        Ok(ids.iter().any(|id| observers.get(id).is_some_and(|observer| {
            !observer.retired && observer.object.is_none_or(|filter| filter.matches(Some(object), Some(window)))
        })))
    }
    pub(super) fn unsubscribe_selector_target(&self, target: usize) -> Result<()> {
        self.check_owner()?;
        self.model.with(|_| Ok(()))?;
        let ids: Vec<_> = self
            .observers
            .lock()
            .map_err(|_| "notification hub poisoned")?
            .iter()
            .filter_map(|(id, observer)| match &observer.delivery {
                Delivery::Selector(selected, _)
                    if !observer.retired && selected.target_identity() == target =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        for id in ids {
            self.unsubscribe(id)?;
        }
        Ok(())
    }
    /// # Safety
    /// notice is a live owned data-only notification object retained by caller
    /// through this synchronous pump. Never a borrowed real desktop notification.
    pub unsafe fn emit(&self, name: &str, notice: *mut AnyObject) -> Result<usize> {
        unsafe { self.deliver(name, notice, None, false) }
    }
    /// # Safety
    /// object and any data reachable from it are owned synthetic data, retained
    /// by the caller through this synchronous owner-thread dispatch.
    pub unsafe fn post(&self, name: &str, object: Option<&AnyObject>) -> Result<usize> {
        unsafe { self.post_with_user_info(name, object, None) }
    }
    /// # Safety
    /// Same as post, including every object reachable from user_info.
    pub unsafe fn post_with_user_info(
        &self,
        name: &str,
        object: Option<&AnyObject>,
        user_info: Option<&NSDictionary>,
    ) -> Result<usize> {
        self.check_owner()?;
        self.model.with(|_| Ok(()))?;
        #[cfg(test)]
        if super::objc::notification_fixture_payload(&self.model) {
            return unsafe { self.post_fixture_notice(name, object, user_info) };
        }
        let notice = super::objc::new_owned_notification(
            &self.model,
            name,
            object.map(|value| (value as *const AnyObject).cast_mut()),
            user_info.map(|value| (value as *const NSDictionary).cast_mut().cast()),
        )?;
        // This actual factory +1 remains local through every nested callback
        // and error path. Its final release follows synchronous delivery.
        unsafe {
            self.deliver(
                name,
                notice.as_ptr(),
                object.map(|value| value as *const AnyObject as usize),
                true,
            )
        }
    }
    #[cfg(test)]
    unsafe fn post_fixture_notice(
        &self,
        name: &str,
        object: Option<&AnyObject>,
        user_info: Option<&NSDictionary>,
    ) -> Result<usize> {
        let notice = unsafe {
            NSNotification::notificationWithName_object_userInfo(
                &NSString::from_str(name),
                object,
                user_info,
            )
        };
        unsafe {
            self.deliver(
                name,
                Retained::as_ptr(&notice).cast_mut().cast(),
                object.map(|p| p as *const AnyObject as usize),
                true,
            )
        }
    }
    unsafe fn deliver(
        &self,
        name: &str,
        notice: *mut AnyObject,
        object: Option<usize>,
        typed: bool,
    ) -> Result<usize> {
        self.check_owner()?;
        #[cfg(any(test, all(rcf1_reference_sdk, target_arch = "aarch64")))]
        let _delivery = {
            use std::sync::atomic::{AtomicUsize, Ordering};
            struct DeliveryGuard<'a>(&'a AtomicUsize);
            impl Drop for DeliveryGuard<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::Relaxed);
                }
            }
            self.deliveries.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| depth.checked_add(1))
                .map_err(|_| "notification delivery depth overflow")?;
            // This guard outlives every copied callback, including disposal.
            DeliveryGuard(&self.deliveries)
        };
        self.model.with(|_| Ok(()))?;
        if notice.is_null() {
            return Err("null notification object".into());
        }
        let ids = self.model.notification_recipients(name)?;
        let window = if name == "NSWindowDidResignKeyNotification" {
            object.map(super::objc::resignation_window_handle).transpose()?.flatten()
        } else {
            None
        };
        let mut borrowed = vec![];
        {
            let mut observers = self
                .observers
                .lock()
                .map_err(|_| "notification hub poisoned")?;
            for id in ids {
                // Each center has its own Hub; model IDs are shared but a
                // notification may never cross into a different center.
                let Some(observer) = observers.get_mut(&id) else {
                    continue;
                };
                if observer.retired || observer.object.is_some_and(|filter| !filter.matches(object, window)) {
                    continue;
                }
                let delivery = match &observer.delivery {
                    Delivery::Block(_) => {
                        PendingCopy::Block(CopyBorrow::reserve(self, id, observer)?)
                    }
                    Delivery::Selector(target, selector) => {
                        if !typed {
                            return Err(
                                "selector delivery requires a genuine owned notification".into()
                            );
                        }
                        PendingCopy::Selector(target.clone(), selector.clone())
                    }
                };
                borrowed.push((id, delivery));
            }
        }
        let mut copies = Vec::with_capacity(borrowed.len());
        for (id, delivery) in borrowed {
            let delivery = match delivery {
                PendingCopy::Block(mut custody) => {
                    let copy = unsafe { RcBlock::copy(custody.block as *mut NotificationReply) }
                        .ok_or("notification copy failed")?;
                    custody.finish()?;
                    (Some(copy), None)
                }
                PendingCopy::Selector(target, selector) => (None, Some((target, selector))),
            };
            copies.push((id, delivery));
        }
        let mut done = 0;
        for (id, (block, selector)) in copies {
            self.model.with(|_| Ok(()))?;
            if !self.model.notification_recipients(name)?.contains(&id) {
                continue;
            }
            if let Some(block) = block {
                block.call((notice,));
            } else if let Some((target, selector)) = selector {
                unsafe { target.notification(&selector, &*notice.cast::<NSNotification>()) }?;
            }
            done += 1;
        }
        Ok(done)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{ClassBuilder, Sel},
        ClassType,
    };
    use objc2_foundation::NSObject;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CopyPayload {
        hub: std::sync::Weak<Hub>,
        dropped: Arc<AtomicUsize>,
    }
    impl Drop for CopyPayload {
        fn drop(&mut self) {
            let hub = self.hub.upgrade().unwrap();
            assert!(
                hub.observers.try_lock().is_ok(),
                "capture cleanup ran under observer lock"
            );
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn failed_token_creation_discards_notification_and_releases_block() {
        let fixture = super::super::tests::Fixture::new();
        let hub = Arc::new(Hub::new(fixture.model.clone()).unwrap());
        let dropped = Arc::new(AtomicUsize::new(0));
        let payload = CopyPayload { hub: Arc::downgrade(&hub), dropped: dropped.clone() };
        let block = RcBlock::new(move |_: *mut AnyObject| {
            let _held = &payload;
        });
        let result: Result<()> = unsafe {
            hub.subscribe_with_token(
                "NSWindowDidResignKeyNotification",
                (&*block as *const NotificationReply).cast_mut(),
                None,
                |id| {
                    assert!(!hub.observers.lock().unwrap().contains_key(&id));
                    fixture.model.fault();
                    Err("fixture token failure".into())
                },
            )
        };
        assert_eq!(result.unwrap_err(), "fixture token failure");
        assert!(fixture.model.state.lock().unwrap().notifications.is_empty());
        assert!(hub.observers.lock().unwrap().is_empty());
        drop(block);
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn retirement_during_token_creation_refuses_observer_commit() {
        let fixture = super::super::tests::Fixture::new();
        let hub = Hub::new(fixture.model.clone()).unwrap();
        let block = RcBlock::new(|_: *mut AnyObject| {});
        let result = unsafe {
            hub.subscribe_with_token(
                "NSWindowDidResignKeyNotification",
                (&*block as *const NotificationReply).cast_mut(),
                None,
                |id| {
                    fixture.model.retire()?;
                    Ok(id)
                },
            )
        };
        assert_eq!(result.unwrap_err(), "workspace generation retired");
        assert!(fixture.model.state.lock().unwrap().notifications.is_empty());
        assert!(hub.observers.lock().unwrap().is_empty());
    }
    #[test]
    fn copy_borrow_preserves_block_through_reentrant_unsubscribe_and_outside_lock_copy() {
        let fixture = super::super::tests::Fixture::new();
        let hub = Arc::new(Hub::new(fixture.model.clone()).unwrap());
        let dropped = Arc::new(AtomicUsize::new(0));
        let payload = CopyPayload {
            hub: Arc::downgrade(&hub),
            dropped: dropped.clone(),
        };
        let block = RcBlock::new(move |_: *mut AnyObject| {
            let _held = &payload;
        });
        let id = unsafe {
            hub.subscribe(
                "NSWindowDidResizeNotification",
                (&*block as *const NotificationReply).cast_mut(),
            )
        }
        .unwrap();
        drop(block);
        let mut custody = {
            let mut observers = hub.observers.lock().unwrap();
            CopyBorrow::reserve(&hub, id, observers.get_mut(&id).unwrap()).unwrap()
        };
        // Exact dangerous window: original owner removed before foreign copy.
        hub.unsubscribe(id).unwrap();
        hub.unsubscribe_if_present(id).unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        {
            let observers = hub.observers.lock().unwrap();
            let observer = observers.get(&id).unwrap();
            assert!(observer.retired);
            assert_eq!(observer.copy_borrows, 1);
        }
        assert!(hub.observers.try_lock().is_ok());
        let copied = unsafe { RcBlock::copy(custody.block as *mut NotificationReply) }.unwrap();
        custody.finish().unwrap();
        assert!(!hub.observers.lock().unwrap().contains_key(&id));
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        drop(copied);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn copy_borrow_unwind_releases_retired_owner_and_refuses_count_overflow() {
        let fixture = super::super::tests::Fixture::new();
        let hub = Arc::new(Hub::new(fixture.model.clone()).unwrap());
        let dropped = Arc::new(AtomicUsize::new(0));
        let payload = CopyPayload {
            hub: Arc::downgrade(&hub),
            dropped: dropped.clone(),
        };
        let block = RcBlock::new(move |_: *mut AnyObject| {
            let _held = &payload;
        });
        let id = unsafe {
            hub.subscribe(
                "NSWindowDidResizeNotification",
                (&*block as *const NotificationReply).cast_mut(),
            )
        }
        .unwrap();
        drop(block);
        {
            let mut observers = hub.observers.lock().unwrap();
            let observer = observers.get_mut(&id).unwrap();
            observer.copy_borrows = MAX_COPY_BORROWS;
            assert!(CopyBorrow::reserve(&hub, id, observer).is_err());
            assert_eq!(observer.copy_borrows, MAX_COPY_BORROWS);
            observer.copy_borrows = 0;
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _custody = {
                let mut observers = hub.observers.lock().unwrap();
                CopyBorrow::reserve(&hub, id, observers.get_mut(&id).unwrap()).unwrap()
            };
            hub.unsubscribe(id).unwrap();
            panic!("owned copy setup failure");
        }));
        assert!(outcome.is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(!hub.observers.lock().unwrap().contains_key(&id));
    }
    static RECEIVED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn received(_: *mut AnyObject, _: Sel, notice: *mut AnyObject) {
        let notice = unsafe { &*notice.cast::<NSNotification>() };
        if notice.name().to_string() == "NSWindowDidResizeNotification" && notice.object().is_some()
        {
            RECEIVED.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn genuine_owned_notifications_share_block_and_selected_dispatch_with_object_filter() {
        let fixture = super::super::tests::Fixture::new();
        let mut class =
            ClassBuilder::new(c"RuycastF1OwnedNotificationReceiver", NSObject::class()).unwrap();
        unsafe {
            class.add_method(
                Sel::register(c"resized:"),
                received as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        let class = class.register();
        let target: Retained<AnyObject> = unsafe { msg_send![class, new] };
        let selected = Arc::new(
            unsafe {
                delegate::ObjectiveC::owned_fixture(
                    fixture.model.clone(),
                    Retained::as_ptr(&target).cast_mut(),
                    &[(c"resized:", received as *const () as usize)],
                )
            }
            .unwrap(),
        );
        let other = super::super::tests::Fixture::new();
        let foreign = Hub::new(other.model.clone()).unwrap();
        assert!(unsafe {
            foreign.subscribe_selector(
                "NSWindowDidResizeNotification",
                selected.clone(),
                "resized:",
                None,
            )
        }
        .is_err());
        let hub = Arc::new(Hub::new(fixture.model.clone()).unwrap());
        let object = NSObject::new();
        let object_id = Retained::as_ptr(&object) as usize;
        let id = unsafe {
            hub.subscribe_selector(
                "NSWindowDidResizeNotification",
                selected.clone(),
                "resized:",
                Some(object_id),
            )
        }
        .unwrap();
        let sibling = Hub::new(fixture.model.clone()).unwrap();
        assert!(sibling.unsubscribe(id).is_err());
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let delivery_hub = Arc::downgrade(&hub);
        let block = RcBlock::new(move |notice: *mut AnyObject| {
            assert_ne!(delivery_hub.upgrade().unwrap().deliveries.load(Ordering::Relaxed), 0);
            let notice = unsafe { &*notice.cast::<NSNotification>() };
            if notice.name().to_string() == "NSWindowDidResizeNotification" {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert_eq!(hub.deliveries.load(Ordering::Relaxed), 0);
        let block_id = unsafe {
            hub.subscribe(
                "NSWindowDidResizeNotification",
                (&*block as *const NotificationReply).cast_mut(),
            )
        }
        .unwrap();
        assert_eq!(
            unsafe { hub.post("NSWindowDidResizeNotification", None) }.unwrap(),
            1
        );
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 0);
        assert_eq!(
            unsafe { hub.post("NSWindowDidResizeNotification", Some(&object)) }.unwrap(),
            2
        );
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // The legacy raw block-only helper cannot manufacture a typed notice
        // for a selected observer; rejection precedes any callback.
        let raw_id = unsafe {
            hub.subscribe_selector("NSWindowDidResizeNotification", selected, "resized:", None)
        }
        .unwrap();
        assert!(unsafe {
            hub.emit(
                "NSWindowDidResizeNotification",
                Retained::as_ptr(&object).cast_mut().cast(),
            )
        }
        .is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(hub.deliveries.load(Ordering::Relaxed), 0);
        hub.unsubscribe(raw_id).unwrap();
        hub.unsubscribe(id).unwrap();
        hub.unsubscribe(block_id).unwrap();
        assert_eq!(
            unsafe { hub.post("NSWindowDidResizeNotification", Some(&object)) }.unwrap(),
            0
        );
    }
}
