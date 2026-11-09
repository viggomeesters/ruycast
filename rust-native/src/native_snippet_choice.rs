//! Dedicated main-owned snippet form. This window grants no target/effect
//! authority: a separate request owner handles submitted values and return.
use crate::{
    native_text,
    snippet_choice::{Action, Key, Model, Prepared},
    templates,
};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::*;
use objc2_foundation::{
    ns_string, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSPointInRect, NSRect, NSSize,
    NSString,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
pub enum Event {
    Submit(Vec<(String, String)>),
    Cancel { restore: bool },
}
pub type SubmitPermit = Rc<dyn Fn() -> bool>;
pub type SubmitPermitFactory = Rc<dyn Fn() -> SubmitPermit>;
pub struct Callbacks {
    pub submit_permit: SubmitPermitFactory,
    pub failure: Rc<dyn Fn()>,
    pub clipboard: Rc<dyn Fn() -> Result<Option<String>, String>>,
    pub event: Rc<dyn Fn(Event)>,
}
struct PanelState {
    owner: RefCell<Option<Rc<Weak<SnippetFormOwner>>>>,
    ordering: bool,
    permit: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    attempted: Cell<bool>,
    #[cfg(feature = "test-tools")]
    fixture_key: Cell<bool>,
}
struct PopupState {
    ordering: bool,
    blocked: Cell<u64>,
}
define_class!(
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=()]
    struct SnippetFormDocument;
    impl SnippetFormDocument { #[unsafe(method(isFlipped))] fn flipped(&self)->bool{true} }
);
define_class!(
    #[unsafe(super=NSPopUpButton)] #[thread_kind=MainThreadOnly] #[ivars=PopupState]
    struct SnippetFormPopup;
    impl SnippetFormPopup {
        #[unsafe(method(mouseDown:))] fn mouse(&self,event:&NSEvent){let _lease=self.retain();if self.may_track(){unsafe{let _:()=msg_send![super(self),mouseDown:event];}}else{self.ivars().blocked.set(self.ivars().blocked.get().saturating_add(1));}}
        #[unsafe(method(keyDown:))] fn key(&self,event:&NSEvent){let _lease=self.retain();if self.may_track(){unsafe{let _:()=msg_send![super(self),keyDown:event];}}else{self.ivars().blocked.set(self.ivars().blocked.get().saturating_add(1));}}
        #[unsafe(method(performClick:))] fn click(&self,sender:Option<&AnyObject>){let _lease=self.retain();if self.may_track(){unsafe{let _:()=msg_send![super(self),performClick:sender];}}else{self.ivars().blocked.set(self.ivars().blocked.get().saturating_add(1));}}
        #[unsafe(method(performKeyEquivalent:))] fn equivalent(&self,event:&NSEvent)->bool{let _lease=self.retain();if self.may_track(){unsafe{msg_send![super(self),performKeyEquivalent:event]}}else{self.ivars().blocked.set(self.ivars().blocked.get().saturating_add(1));false}}
    }
);
impl SnippetFormPopup {
    fn may_track(&self) -> bool {
        self.ivars().ordering
            && self
                .window()
                .and_then(|w| w.downcast::<SnippetFormPanel>().ok())
                .is_some_and(|p| p.may_order())
    }
}

define_class!(
    #[unsafe(super=NSPanel)] #[thread_kind=MainThreadOnly] #[ivars=PanelState]
    struct SnippetFormPanel;
    impl SnippetFormPanel {
        #[unsafe(method(canBecomeKeyWindow))] fn key_allowed(&self)->bool {let _lease=self.retain();self.may_order()}
        #[unsafe(method(canBecomeMainWindow))] fn main_allowed(&self)->bool {false}
        #[unsafe(method(makeKeyAndOrderFront:))] fn key_front(&self,sender:Option<&AnyObject>){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),makeKeyAndOrderFront:sender];}}}
        #[unsafe(method(orderFront:))] fn front(&self,sender:Option<&AnyObject>){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),orderFront:sender];}}}
        #[unsafe(method(orderFrontRegardless))] fn front_regardless(&self){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),orderFrontRegardless];}}}
        #[unsafe(method(orderBack:))] fn back(&self,sender:Option<&AnyObject>){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),orderBack:sender];}}}
        #[unsafe(method(orderWindow:relativeTo:))] fn order(&self,mode:NSWindowOrderingMode,relative:isize){let _lease=self.retain();if mode==NSWindowOrderingMode::Out || self.may_order() {unsafe{let _:()=msg_send![super(self),orderWindow:mode,relativeTo:relative];}}}
        #[unsafe(method(makeKeyWindow))] fn make_key(&self){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),makeKeyWindow];}}}
        #[unsafe(method(becomeKeyWindow))] fn become_key(&self){let _lease=self.retain();if self.may_order() {unsafe{let _:()=msg_send![super(self),becomeKeyWindow];}}}
        #[unsafe(method(sendEvent:))] fn send(&self,event:&NSEvent){let _lease=self.retain();if !self.route(event){unsafe{let _:()=msg_send![super(self),sendEvent:event];}}}
        #[unsafe(method(performKeyEquivalent:))] fn equivalent(&self,event:&NSEvent)->bool{let _lease=self.retain();self.owner().is_none_or(|owner|owner.guard(||self.equivalent_route(event)).unwrap_or(true))}
        #[unsafe(method(keyDown:))] fn down(&self,event:&NSEvent){let _lease=self.retain();if !self.route(event){unsafe{let _:()=msg_send![super(self),keyDown:event];}}}
        #[unsafe(method(close))] fn close_choice(&self){let _lease=self.retain();let owner=self.owner();if let Some(o)=owner{o.guard(||o.cancel(false));}
            unsafe{let _:()=msg_send![super(self),close];}}
        #[unsafe(method(orderOut:))] fn out(&self,sender:Option<&NSObject>){let _lease=self.retain();let owner=self.owner();if let Some(o)=owner{o.guard(||o.cancel(false));}
            unsafe{let _:()=msg_send![super(self),orderOut:sender];}}
        #[unsafe(method(resignKeyWindow))] fn resign(&self){let _lease=self.retain();let owner=self.owner();if let Some(o)=owner{o.guard(||o.cancel(false));}
            unsafe{let _:()=msg_send![super(self),resignKeyWindow];}}
    }
);
impl SnippetFormPanel {
    fn may_order(&self) -> bool {
        if !self.ivars().ordering {
            return false;
        }
        let Some(owner) = self.owner() else {
            return false;
        };
        if !owner.ivars().live.get() || owner.ivars().failed.get() {
            return false;
        }
        let permit = self.ivars().permit.borrow().clone();
        permit.is_some_and(|p| owner.guard(|| p()).unwrap_or(false))
            && owner.ivars().live.get()
            && !owner.ivars().failed.get()
    }
    fn equivalent_route(&self, event: &NSEvent) -> bool {
        if self.route(event) {
            return true;
        }
        if self
            .firstResponder()
            .and_then(|r| r.downcast::<NSTextView>().ok())
            .is_some_and(|e| unsafe {
                let marked: bool = msg_send![&*e, hasMarkedText];
                marked
            })
        {
            return false;
        }
        if crate::responder_edit::perform(event, self.firstResponder().as_deref()) {
            if let Some(o) = self.owner() {
                o.guard(|| {
                    o.flush();
                    o.refresh();
                });
            }
            return true;
        }
        unsafe { msg_send![super(self),performKeyEquivalent:event] }
    }
    fn owner(&self) -> Option<Retained<SnippetFormOwner>> {
        {
            let weak = self.ivars().owner.borrow().clone();
            weak.and_then(|w| w.load())
        }
    }
    fn route(&self, event: &NSEvent) -> bool {
        self.owner()
            .is_none_or(|o| o.guard(|| o.route(event)).unwrap_or(true))
    }
}
enum Control {
    Text(Retained<NSTextField>),
    Choice(Retained<NSPopUpButton>),
}
struct State {
    panel: Retained<SnippetFormPanel>,
    model: RefCell<Option<Model>>,
    controls: Vec<Control>,
    insert: Retained<NSButton>,
    cancel: Retained<NSButton>,
    live: Cell<bool>,
    failed: Cell<bool>,
    epoch: Cell<u64>,
    callbacks: Callbacks,
    observers: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>>,
    undo_managers: RefCell<Vec<Retained<objc2_foundation::NSUndoManager>>>,
    undo_ids: RefCell<std::collections::HashSet<usize>>,
}
define_class!(
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[ivars=State]
    struct SnippetFormOwner;
    unsafe impl NSObjectProtocol for SnippetFormOwner {}
    unsafe impl NSTextFieldDelegate for SnippetFormOwner {}
    unsafe impl NSControlTextEditingDelegate for SnippetFormOwner {
        #[unsafe(method(controlTextDidEndEditing:))] fn ended(&self,note:&NSNotification){self.guard(||{if let Some(text)=note.object().and_then(|o|o.downcast::<NSTextField>().ok()){if usize::try_from(text.tag()).ok().is_some_and(|i|self.control_live(i,&text)){self.flush();self.refresh();}}});}
        #[unsafe(method(controlTextDidChange:))] fn changed(&self,note:&NSNotification){self.guard(||{
            let Some(text)=note.object().and_then(|v|v.downcast::<NSTextField>().ok()) else{return;};
            let index=usize::try_from(text.tag()).ok();let Some(index)=index else{return;};
            if !self.control_live(index,&text){return;}
            let value=native_text::repaired(&text.stringValue());self.with_model(|m|m.set(index,value));self.refresh();
        });}
        #[unsafe(method(controlTextDidBeginEditing:))] fn began(&self,note:&NSNotification){self.guard(||{if let Some(text)=note.object().and_then(|v|v.downcast::<NSTextField>().ok()){if let Ok(i)=usize::try_from(text.tag()){if self.control_live(i,&text){self.with_model(|m|m.focus(i));if self.ivars().live.get(){text.scrollRectToVisible(text.bounds());}
            if let Some(editor)=text.currentEditor().and_then(|e|e.downcast::<NSTextView>().ok()){editor.setAllowsUndo(true);if let Some(manager)=editor.undoManager(){self.watch_undo(manager);}}}}}});}
    }
    impl SnippetFormOwner {
        #[unsafe(method(insertChoice:))] fn insert_action(&self,_:&NSButton){self.guard(||self.submit());}
        #[unsafe(method(cancelChoice:))] fn cancel_action(&self,_:&NSButton){self.guard(||self.cancel(true));}
        #[unsafe(method(selectChoice:))] fn option_action(&self,sender:&NSPopUpButton){self.guard(||{
            let Ok(i)=usize::try_from(sender.tag()) else{return;};
            if !self.ivars().live.get() || !matches!(self.ivars().controls.get(i),Some(Control::Choice(c)) if std::ptr::eq(&**c,sender)){return;}
            let value=sender.titleOfSelectedItem().map(|s|native_text::repaired(&s)).unwrap_or_default();self.with_model(|m|m.set(i,value));self.refresh();
        });}
    }
);
impl SnippetFormOwner {
    fn watch_undo(&self, manager: Retained<objc2_foundation::NSUndoManager>) {
        if !self.ivars().live.get() {
            return;
        }
        let epoch = self.ivars().epoch.get();
        let id = &*manager as *const _ as usize;
        let inserted = self.ivars().undo_ids.borrow_mut().insert(id);
        if !inserted {
            return;
        }
        let mut tokens = vec![];
        for name in unsafe {
            [
                objc2_foundation::NSUndoManagerDidUndoChangeNotification,
                objc2_foundation::NSUndoManagerDidRedoChangeNotification,
            ]
        } {
            let weak = Rc::new(Weak::new(self));
            let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
                if MainThreadMarker::new().is_none() {
                    return;
                }
                if let Some(owner) = weak.load() {
                    owner.guard(|| {
                        owner.flush();
                        owner.refresh();
                    });
                }
            });
            let token = unsafe {
                objc2_foundation::NSNotificationCenter::defaultCenter()
                    .addObserverForName_object_queue_usingBlock(
                        Some(name),
                        Some(std::convert::AsRef::<AnyObject>::as_ref(&*manager)),
                        Some(&objc2_foundation::NSOperationQueue::mainQueue()),
                        &block,
                    )
            };
            tokens.push(token);
        }
        if self.ivars().live.get() && self.ivars().epoch.get() == epoch {
            self.ivars().observers.borrow_mut().extend(tokens);
            self.ivars().undo_managers.borrow_mut().push(manager);
        } else {
            for token in tokens {
                unsafe {
                    objc2_foundation::NSNotificationCenter::defaultCenter()
                        .removeObserver(std::convert::AsRef::<AnyObject>::as_ref(&*token));
                }
            }
        }
    }
    fn guard<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let _lease = self.retain();
        if self.ivars().failed.get() {
            return None;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(v) => Some(v),
            Err(_) => {
                self.ivars().failed.set(true);
                self.revoke();
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (self.ivars().callbacks.failure)()
                }));
                self.ivars().panel.orderOut(None);
                None
            }
        }
    }
    // A taken model lives on the stack. No RefCell borrow survives Foundation,
    // control SDK calls, render validation, callbacks or native destruction.
    fn with_model<T>(&self, f: impl FnOnce(&mut Model) -> T) -> Option<T> {
        if !self.ivars().live.get() {
            return None;
        }
        let epoch = self.ivars().epoch.get();
        let mut model = self.ivars().model.borrow_mut().take()?;
        let value = f(&mut model);
        if self.ivars().live.get() && self.ivars().epoch.get() == epoch {
            *self.ivars().model.borrow_mut() = Some(model);
            Some(value)
        } else {
            None
        }
    }
    fn revoke(&self) {
        self.ivars().live.set(false);
        #[cfg(feature = "test-tools")]
        self.ivars().panel.ivars().fixture_key.set(false);
        self.ivars()
            .epoch
            .set(self.ivars().epoch.get().saturating_add(1));
        let model = self.ivars().model.borrow_mut().take();
        drop(model);
    }
    fn control_live(&self, i: usize, text: &NSTextField) -> bool {
        self.ivars().live.get()
            && matches!(self.ivars().controls.get(i),Some(Control::Text(t)) if std::ptr::eq(&**t,text))
    }
    fn refresh(&self) {
        let Some((valid, indices)) = self.with_model(|m| {
            (
                m.can_submit(),
                (0..m.values().len())
                    .map(|i| m.selected(i))
                    .collect::<Vec<_>>(),
            )
        }) else {
            return;
        };
        if !self.ivars().live.get() {
            return;
        }
        let available = (self.ivars().callbacks.submit_permit)();
        self.ivars().insert.setEnabled(valid && available());
        for (i, index) in indices.into_iter().enumerate() {
            if !self.ivars().live.get() {
                return;
            }
            if let (Some(Control::Choice(c)), Some(index)) = (self.ivars().controls.get(i), index) {
                c.selectItemAtIndex(index as isize);
            }
        }
    }
    fn submit(&self) {
        if !self.flush() {
            return;
        }
        self.refresh();
        let Some((eligible, needs)) = self.with_model(|m| {
            (
                !m.finished() && m.can_submit(),
                m.prepared().needs_clipboard(),
            )
        }) else {
            return;
        };
        let permit = (self.ivars().callbacks.submit_permit)();
        if !eligible || !permit() {
            return;
        }
        let clipboard = if needs {
            match (self.ivars().callbacks.clipboard)() {
                Ok(v) => v,
                Err(_) => return,
            }
        } else {
            None
        };
        let Some(Ok(Some(values))) = self.with_model(|m| {
            m.submit_if(&templates::Context::current(clipboard.as_deref()), &|| {
                permit()
            })
        }) else {
            return;
        };
        self.revoke();
        // Data callback establishes Returning on the separate request owner
        // before our orderOut can synchronously notify a resign observer.
        (self.ivars().callbacks.event)(Event::Submit(values));
        self.ivars().panel.orderOut(None);
    }
    fn cancel(&self, restore: bool) {
        if !self.ivars().live.replace(false) {
            return;
        }
        self.revoke();
        (self.ivars().callbacks.event)(Event::Cancel { restore });
        self.ivars().panel.orderOut(None);
    }
    fn route(&self, event: &NSEvent) -> bool {
        if event.r#type() != NSEventType::KeyDown {
            return false;
        }
        if !self.ivars().live.get() {
            return true;
        }
        let first = self.ivars().panel.firstResponder();
        let editor = first.as_ref().and_then(|v| v.downcast_ref::<NSTextView>());
        if let Some(popup) = first
            .as_ref()
            .and_then(|v| v.downcast_ref::<NSPopUpButton>())
        {
            if let Ok(i) = usize::try_from(popup.tag()) {
                self.with_model(|m| m.focus(i));
            }
        }
        let composing = editor.is_some_and(|e| unsafe {
            let value: bool = msg_send![e, hasMarkedText];
            value
        });
        let action = self.with_model(|m| {
            m.handle(Key {
                code: event.keyCode(),
                modifiers: event.modifierFlags().0 as u64,
                repeat: event.isARepeat(),
                editing_text: editor.is_some(),
                composing,
            })
        });
        match action {
            Some(Action::Unhandled) => false,
            Some(Action::Submit) => {
                self.submit();
                true
            }
            Some(Action::Cancel) => {
                self.cancel(true);
                true
            }
            _ => {
                self.refresh();
                true
            }
        }
    }
    fn retire(&self) {
        self.revoke();
        let observers = std::mem::take(&mut *self.ivars().observers.borrow_mut());
        for observer in observers {
            unsafe {
                objc2_foundation::NSNotificationCenter::defaultCenter()
                    .removeObserver(std::convert::AsRef::<AnyObject>::as_ref(&*observer));
            }
        }
        for c in &self.ivars().controls {
            match c {
                Control::Text(t) => unsafe {
                    t.setDelegate(None);
                },
                Control::Choice(c) => unsafe {
                    c.setTarget(None);
                    c.setAction(None);
                },
            }
        }
        unsafe {
            self.ivars().insert.setTarget(None);
            self.ivars().insert.setAction(None);
            self.ivars().cancel.setTarget(None);
            self.ivars().cancel.setAction(None);
        }
        let weak = self.ivars().panel.ivars().owner.borrow_mut().take();
        drop(weak);
        let permit = self.ivars().panel.ivars().permit.borrow_mut().take();
        drop(permit);
        self.ivars().panel.orderOut(None);
        self.ivars().panel.close();
        let managers = std::mem::take(&mut *self.ivars().undo_managers.borrow_mut());
        drop(managers);
        self.ivars().undo_ids.borrow_mut().clear();
    }
    fn flush(&self) -> bool {
        if !self.ivars().live.get() {
            return false;
        }
        let epoch = self.ivars().epoch.get();
        let first = self.ivars().panel.firstResponder();
        let mut values = vec![];
        for (i, control) in self.ivars().controls.iter().enumerate() {
            if !self.ivars().live.get() || self.ivars().epoch.get() != epoch {
                return false;
            }
            if let Control::Text(text) = control {
                let editor = text
                    .currentEditor()
                    .and_then(|v| v.downcast::<NSTextView>().ok());
                let value = if let Some(editor) = editor.filter(|e| {
                    first.as_ref().is_some_and(|f| {
                        std::ptr::eq(
                            &**e as *const _ as *const AnyObject,
                            &**f as *const _ as *const AnyObject,
                        )
                    }) && e.window().is_some_and(|w| {
                        std::ptr::eq(
                            &*w as *const _ as *const AnyObject,
                            &*self.ivars().panel as *const _ as *const AnyObject,
                        )
                    })
                }) {
                    native_text::repaired(&editor.string())
                } else {
                    native_text::repaired(&text.stringValue())
                };
                values.push((i, value));
            }
        }
        if !self.ivars().live.get() || self.ivars().epoch.get() != epoch {
            return false;
        }
        self.with_model(|m| {
            for (i, value) in values {
                m.sync(i, value);
            }
        })
        .is_some()
    }
}
pub struct Window {
    owner: Retained<SnippetFormOwner>,
}
struct Construction {
    panel: Retained<SnippetFormPanel>,
    owner: Option<Retained<SnippetFormOwner>>,
    armed: bool,
}
impl Drop for Construction {
    fn drop(&mut self) {
        if self.armed {
            if let Some(owner) = self.owner.take() {
                owner.retire();
            } else {
                self.panel.orderOut(None);
                self.panel.close();
            }
        }
    }
}
impl Window {
    pub fn new(
        main: MainThreadMarker,
        prepared: Arc<Prepared>,
        ordering: bool,
        callbacks: Callbacks,
    ) -> Self {
        let height = (130.0f64 + prepared.fields().len() as f64 * 70.0).min(440.0);
        let panel: Retained<SnippetFormPanel> = unsafe {
            let p = SnippetFormPanel::alloc(main).set_ivars(PanelState {
                owner: RefCell::new(None),
                ordering,
                permit: RefCell::new(None),
                attempted: Cell::new(false),
                #[cfg(feature = "test-tools")]
                fixture_key: Cell::new(false),
            });
            msg_send![super(p),initWithContentRect:rect(-10000.,-10000.,456.,height),styleMask:NSWindowStyleMask::Borderless|NSWindowStyleMask::NonactivatingPanel,backing:NSBackingStoreType::Buffered,defer:false]
        };
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        let mut construction = Construction {
            panel: panel.clone(),
            owner: None,
            armed: true,
        };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let root = NSVisualEffectView::initWithFrame(
            NSVisualEffectView::alloc(main),
            rect(0., 0., 456., height),
        );
        root.setMaterial(NSVisualEffectMaterial::Popover);
        root.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        root.setState(NSVisualEffectState::Active);
        let draw = block2::RcBlock::new(|bounds: NSRect| -> objc2::runtime::Bool {
            std::panic::catch_unwind(|| {
                if MainThreadMarker::new().is_none() {
                    return false;
                }
                NSColor::whiteColor().setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 14., 14.).fill();
                true
            })
            .unwrap_or(false)
            .into()
        });
        let mask =
            NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(456., height), false, &draw);
        root.setMaskImage(Some(&mask));
        panel.setContentView(Some(&root));
        let title = NSTextField::wrappingLabelWithString(
            &NSString::from_str(&prepared.command().title),
            main,
        );
        title.setFont(Some(&NSFont::systemFontOfSize_weight(17., unsafe {
            NSFontWeightSemibold
        })));
        title.setMaximumNumberOfLines(2);
        title.setPreferredMaxLayoutWidth(412.);
        let title_height = title.intrinsicContentSize().height.clamp(21., 42.);
        title.setFrame(rect(22., height - 22. - title_height, 412., title_height));
        title.setAccessibilityLabel(Some(&NSString::from_str(&prepared.command().title)));
        root.addSubview(&title);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(main),
            rect(22., 65., 412., (height - 105. - title_height).max(0.)),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        let rows = prepared
            .fields()
            .iter()
            .map(|field| {
                let label =
                    NSTextField::wrappingLabelWithString(&NSString::from_str(&field.label), main);
                label.setFont(Some(&NSFont::systemFontOfSize_weight(12., unsafe {
                    NSFontWeightMedium
                })));
                label.setTextColor(Some(&NSColor::labelColor()));
                label.setPreferredMaxLayoutWidth(395.);
                let label_height = label.intrinsicContentSize().height.max(19.);
                let row_height = (label_height + 48.).max(70.);
                (label, label_height, row_height)
            })
            .collect::<Vec<_>>();
        let doc_height = rows.iter().map(|r| r.2).sum::<f64>();
        let document: Retained<SnippetFormDocument> = unsafe {
            {
                let view = SnippetFormDocument::alloc(main).set_ivars(());
                msg_send![super(view),initWithFrame:rect(0.,0.,395.,doc_height)]
            }
        };
        scroll.setDocumentView(Some(&document));
        root.addSubview(&scroll);
        let model = Model::new(prepared.clone());
        let mut controls = vec![];
        let mut y = 0.;
        for (i, (field, (label, label_height, row_height))) in
            prepared.fields().iter().zip(rows).enumerate()
        {
            label.setFrame(rect(0., y, 395., label_height));
            document.addSubview(&label);
            if field.options.is_empty() {
                let text =
                    NSTextField::textFieldWithString(&NSString::from_str(&model.values()[i]), main);
                text.setFrame(rect(0., y + label_height + 6., 395., 28.));
                text.setTag(i as isize);
                text.setAccessibilityLabel(Some(&NSString::from_str(&field.label)));
                document.addSubview(&text);
                controls.push(Control::Text(text));
            } else {
                let choice: Retained<SnippetFormPopup> = unsafe {
                    let p = SnippetFormPopup::alloc(main).set_ivars(PopupState {
                        ordering,
                        blocked: Cell::new(0),
                    });
                    msg_send![super(p),initWithFrame:rect(0.,y+label_height+6.,395.,28.),pullsDown:false]
                };
                let choice: Retained<NSPopUpButton> = choice.into_super();
                for value in &field.options {
                    choice.addItemWithTitle(&NSString::from_str(value));
                }
                choice.setTag(i as isize);
                choice.setAccessibilityLabel(Some(&NSString::from_str(&field.label)));
                if let Some(index) = model.selected(i) {
                    choice.selectItemAtIndex(index as isize);
                }
                document.addSubview(&choice);
                controls.push(Control::Choice(choice));
            }
            y += row_height;
        }
        let cancel = NSButton::initWithFrame(NSButton::alloc(main), rect(22., 20., 84., 30.));
        cancel.setTitle(ns_string!("Cancel"));
        cancel.setBezelStyle(NSBezelStyle::Push);
        cancel.setAccessibilityLabel(Some(ns_string!("Cancel")));
        root.addSubview(&cancel);
        let insert = NSButton::initWithFrame(NSButton::alloc(main), rect(350., 20., 84., 30.));
        insert.setTitle(ns_string!("Insert"));
        insert.setKeyEquivalent(ns_string!("\r"));
        insert.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        insert.setBezelStyle(NSBezelStyle::Push);
        insert.setAccessibilityLabel(Some(ns_string!("Insert")));
        insert.setEnabled(model.can_submit());
        root.addSubview(&insert);
        let owner: Retained<SnippetFormOwner> = unsafe {
            let o = SnippetFormOwner::alloc(main).set_ivars(State {
                panel: panel.clone(),
                model: RefCell::new(Some(model)),
                controls,
                insert,
                cancel,
                live: Cell::new(false),
                failed: Cell::new(false),
                epoch: Cell::new(1),
                callbacks,
                observers: RefCell::new(vec![]),
                undo_managers: RefCell::new(vec![]),
                undo_ids: RefCell::new(std::collections::HashSet::new()),
            });
            msg_send![super(o), init]
        };
        construction.owner = Some(owner.clone());
        let weak = Rc::new(Weak::new(&*owner));
        let old = panel.ivars().owner.replace(Some(weak));
        drop(old);
        for c in &owner.ivars().controls {
            match c {
                Control::Text(t) => unsafe {
                    t.setDelegate(Some(ProtocolObject::from_ref(&*owner)));
                },
                Control::Choice(c) => unsafe {
                    c.setTarget(Some(&owner));
                    c.setAction(Some(sel!(selectChoice:)));
                },
            }
        }
        unsafe {
            owner.ivars().cancel.setTarget(Some(&owner));
            owner.ivars().cancel.setAction(Some(sel!(cancelChoice:)));
            owner.ivars().insert.setTarget(Some(&owner));
            owner.ivars().insert.setAction(Some(sel!(insertChoice:)));
        }
        owner.ivars().live.set(true);
        let window = Self { owner };
        construction.armed = false;
        window
    }
    /// One presentation attempt. Permit is data-only, weakly bound to the
    /// original request; this method never supplies target/effect authority.
    pub fn show(&self, permit: Rc<dyn Fn() -> bool>) -> bool {
        if !self.owner.ivars().live.get() || !self.owner.ivars().panel.ivars().ordering {
            return false;
        }
        let panel = &self.owner.ivars().panel;
        if panel.ivars().attempted.replace(true) {
            return false;
        }
        let old = panel.ivars().permit.replace(Some(permit));
        drop(old);
        if !panel.may_order() {
            return false;
        }
        let mouse = NSEvent::mouseLocation();
        if !panel.may_order() {
            return false;
        }
        let Some(screen) = NSScreen::screens(self.owner.mtm())
            .iter()
            .find(|s| NSPointInRect(mouse, s.frame()))
            .or_else(|| NSScreen::mainScreen(self.owner.mtm()))
        else {
            return false;
        };
        if panel.may_order() {
            let frame = screen.visibleFrame();
            let size = panel.frame().size;
            if !panel.may_order() {
                return false;
            }
            panel.setFrameOrigin(NSPoint::new(
                frame.origin.x + (frame.size.width - size.width) / 2.,
                frame.origin.y + frame.size.height - size.height - 100.,
            ));
        }
        if !panel.may_order() {
            return false;
        }
        panel.makeKeyAndOrderFront(None);
        panel.may_order()
    }
    pub fn cancel(&self, restore: bool) {
        self.owner.guard(|| self.owner.cancel(restore));
    }
    pub fn failed(&self) -> bool {
        self.owner.ivars().failed.get()
    }
    pub fn is_key(&self) -> bool {
        #[cfg(feature = "test-tools")]
        if !self.owner.ivars().panel.ivars().ordering {
            return self.owner.ivars().panel.ivars().fixture_key.get()
                && self.owner.ivars().live.get();
        }
        self.owner.ivars().panel.isKeyWindow()
    }
    pub fn refresh_availability(&self) {
        self.owner.guard(|| self.owner.refresh());
    }
    pub fn set_theme(&self, theme: Option<&str>) {
        self.owner.guard(|| {
            if !self.owner.ivars().live.get() {
                return;
            }
            let appearance = match theme {
                Some("light") => NSAppearance::appearanceNamed(ns_string!("NSAppearanceNameAqua")),
                Some("dark") => {
                    NSAppearance::appearanceNamed(ns_string!("NSAppearanceNameDarkAqua"))
                }
                _ => None,
            };
            if self.owner.ivars().live.get() {
                self.owner
                    .ivars()
                    .panel
                    .setAppearance(appearance.as_deref());
            }
        });
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_present(&self, permit: Rc<dyn Fn() -> bool>) -> bool {
        if self.owner.ivars().panel.ivars().ordering
            || !self.owner.ivars().live.get()
            || self.failed()
            || !permit()
        {
            return false;
        }
        let old = self
            .owner
            .ivars()
            .panel
            .ivars()
            .permit
            .replace(Some(permit));
        drop(old);
        self.owner.ivars().panel.ivars().fixture_key.set(true);
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_control(&self, case: &serde_json::Value) -> Result<serde_json::Value, String> {
        if self.owner.ivars().panel.ivars().ordering {
            return Err("Hidden own form required".into());
        }
        match case["choiceAction"].as_str() {
            Some("set") => {
                let index = case["field"].as_u64().ok_or("Own field required")? as usize;
                let value = case["value"].as_str().ok_or("Own field value required")?;
                let Some(Control::Text(text)) = self.owner.ivars().controls.get(index) else {
                    return Err("Own text field required".into());
                };
                text.setStringValue(&NSString::from_str(value));
                self.owner.guard(|| {
                    self.owner.flush();
                    self.owner.refresh();
                });
            }
            Some("insert") => unsafe { self.owner.ivars().insert.performClick(None) },
            Some("submit-key") => {
                self.owner.guard(|| self.owner.submit());
            }
            Some("cancel") => unsafe { self.owner.ivars().cancel.performClick(None) },
            Some("read") => {}
            Some("render") => {
                let directory = std::env::var_os("RUYCAST_KEYWORD_FIXTURE_HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or("Own fixture path required")?;
                if std::fs::read(directory.join(".ruycast-keyboard-fixture"))
                    .ok()
                    .as_deref()
                    != Some(b"own-keyboard-fixture-v1")
                {
                    return Err("Own fixture stamp required".into());
                }
                let root = self
                    .owner
                    .ivars()
                    .panel
                    .contentView()
                    .ok_or("Own form view required")?;
                root.layoutSubtreeIfNeeded();
                let bitmap = root
                    .bitmapImageRepForCachingDisplayInRect(root.bounds())
                    .ok_or("Own bitmap failed")?;
                root.cacheDisplayInRect_toBitmapImageRep(root.bounds(), &bitmap);
                let data = unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &objc2_foundation::NSDictionary::new(),
                    )
                }
                .ok_or("Own PNG failed")?;
                std::fs::write(directory.join("choice.png"), unsafe {
                    data.as_bytes_unchecked()
                })
                .map_err(|e| e.to_string())?;
            }
            _ => return Err("Unknown own form action".into()),
        }
        let values = self.owner.with_model(|m| m.values().to_vec());
        fn text(view: &NSView, depth: usize, output: &mut Vec<String>) -> Result<(), String> {
            if depth > 64 {
                return Err("Own view nesting exceeded".into());
            }
            if let Some(field) = view.downcast_ref::<NSTextField>() {
                let value = native_text::repaired(&field.stringValue());
                if !value.is_empty() {
                    output.push(value);
                }
            } else if let Some(button) = view.downcast_ref::<NSButton>() {
                let value = native_text::repaired(&button.title());
                if !value.is_empty() {
                    output.push(value);
                }
            }
            for child in view.subviews().iter() {
                text(&child, depth + 1, output)?;
            }
            Ok(())
        }
        let mut screen_text = vec![];
        if let Some(root) = self.owner.ivars().panel.contentView() {
            text(&root, 0, &mut screen_text)?;
        }
        let labels = self
            .owner
            .ivars()
            .controls
            .iter()
            .map(|control| match control {
                Control::Text(t) => t.accessibilityLabel().map(|s| native_text::repaired(&s)),
                Control::Choice(c) => {
                    NSAccessibility::accessibilityLabel(&**c).map(|s| native_text::repaired(&s))
                }
            })
            .collect::<Vec<_>>();
        Ok(
            serde_json::json!({"live":self.owner.ivars().live.get(),"failed":self.failed(),"values":values,"fixtureKey":self.is_key(),"nativeIsKey":self.owner.ivars().panel.isKeyWindow(),"nativeVisible":self.owner.ivars().panel.isVisible(),"insertEnabled":self.owner.ivars().insert.isEnabled(),"screenText":screen_text,"labels":labels,"appearance":native_text::repaired(&self.owner.ivars().panel.effectiveAppearance().name()),"explicitAppearance":self.owner.ivars().panel.appearance().map(|a|native_text::repaired(&a.name()))}),
        )
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        self.owner.retire();
    }
}

#[cfg(feature = "test-tools")]
pub fn fixture(
    directory: &std::path::Path,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if value["dropDuring"].is_string() {
        return drop_fixture(value);
    }
    let (mut result, weak) = objc2::rc::autoreleasepool(|_| {
        let main = MainThreadMarker::new().ok_or("Main fixture required")?;
        let guard =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_fixture_guard".as_ptr()) };
        if guard.is_null() {
            return Err("Complete owned clipboard substitute required".into());
        }
        let guard: unsafe extern "C" fn() -> bool = unsafe { std::mem::transmute(guard) };
        if !unsafe { guard() } {
            return Err("Owned clipboard substitute unavailable".into());
        }
        let app = NSApplication::sharedApplication(main);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        let command =
            serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())?;
        let prepared = Arc::new(Prepared::new(command)?);
        let events = Rc::new(RefCell::new(vec![]));
        let event_count = events.clone();
        let weak_slot: Rc<RefCell<Option<Rc<Weak<SnippetFormOwner>>>>> =
            Rc::new(RefCell::new(None));
        let weak = weak_slot.clone();
        let reenter = value["reenter"].as_bool().unwrap_or(false);
        let panic = value["panic"].as_bool().unwrap_or(false);
        let reads = Rc::new(Cell::new(0));
        let read_count = reads.clone();
        let clipboard = value["clipboard"].as_str().map(str::to_owned);
        let window = Window::new(
            main,
            prepared,
            false,
            Callbacks {
                submit_permit: Rc::new(|| Rc::new(|| true)),
                failure: Rc::new(|| {}),
                clipboard: Rc::new(move || {
                    read_count.set(read_count.get() + 1);
                    Ok(clipboard.clone())
                }),
                event: Rc::new(move |event| {
                    match event {
                        Event::Submit(values) => event_count
                            .borrow_mut()
                            .push(serde_json::json!({"submit":values})),
                        Event::Cancel { restore } => event_count
                            .borrow_mut()
                            .push(serde_json::json!({"cancel":true,"restore":restore})),
                    }
                    if reenter {
                        let w = weak.borrow().clone();
                        let owner = w.and_then(|w| w.load());
                        if let Some(o) = owner {
                            o.submit();
                            o.cancel(true);
                        }
                    }
                    assert!(!panic, "own choice callback panic");
                }),
            },
        );
        let fixture_weak = Rc::new(Weak::new(&*window.owner));
        *weak_slot.borrow_mut() = Some(fixture_weak);
        let appearance = if value["theme"].as_str() == Some("dark") {
            unsafe { NSAppearanceNameDarkAqua }
        } else {
            unsafe { NSAppearanceNameAqua }
        };
        window
            .owner
            .ivars()
            .panel
            .setAppearance(NSAppearance::appearanceNamed(appearance).as_deref());
        let mut steps = vec![];
        for op in value["steps"].as_array().into_iter().flatten() {
            match op["op"].as_str() {
                Some("set") => {
                    let i = op["field"].as_u64().unwrap_or(0) as usize;
                    if let Some(Control::Text(text)) = window.owner.ivars().controls.get(i) {
                        text.setStringValue(&NSString::from_str(
                            op["value"].as_str().unwrap_or(""),
                        ));
                        let note = unsafe {
                            NSNotification::notificationWithName_object(
                                NSControlTextDidChangeNotification,
                                Some(text),
                            )
                        };
                        unsafe {
                            let _: () = msg_send![&*window.owner,controlTextDidChange:&*note];
                        }
                    }
                }
                Some("choose") => {
                    if let Some(Control::Choice(c)) = window
                        .owner
                        .ivars()
                        .controls
                        .get(op["field"].as_u64().unwrap_or(0) as usize)
                    {
                        c.selectItemAtIndex(op["index"].as_i64().unwrap_or(0) as isize);
                        unsafe {
                            let _: () = msg_send![&*window.owner,selectChoice:&**c];
                        }
                    }
                }
                Some("focus") => {
                    if let Some(c) = window
                        .owner
                        .ivars()
                        .controls
                        .get(op["field"].as_u64().unwrap_or(0) as usize)
                    {
                        let responder: &NSResponder = match c {
                            Control::Text(t) => t,
                            Control::Choice(c) => c,
                        };
                        let accepted = window
                            .owner
                            .ivars()
                            .panel
                            .makeFirstResponder(Some(responder));
                        steps.push(serde_json::json!({"focus":accepted}));
                    }
                }
                Some("marked") => {
                    let responder = window.owner.ivars().panel.firstResponder();
                    let editor = responder
                        .as_ref()
                        .and_then(|r| r.downcast_ref::<NSTextView>())
                        .ok_or("Own field editor absent")?;
                    let text = NSString::from_str("仮");
                    unsafe {
                        let _: () = msg_send![editor,setMarkedText:&*text,selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                    }
                }
                Some("type") => {
                    let responder = window.owner.ivars().panel.firstResponder();
                    let editor = responder
                        .as_ref()
                        .and_then(|r| r.downcast_ref::<NSTextView>())
                        .ok_or("Own field editor absent")?;
                    editor.setAllowsUndo(true);
                    editor.setSelectedRange(objc2_foundation::NSRange::new(
                        editor.string().length(),
                        0,
                    ));
                    let undo = editor.undoManager().ok_or("Own Undo manager absent")?;
                    undo.beginUndoGrouping();
                    let text = NSString::from_str(op["value"].as_str().unwrap_or(""));
                    unsafe {
                        let _: () = msg_send![editor,insertText:&*text,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                    }
                    undo.endUndoGrouping();
                    steps.push(serde_json::json!({"typed":native_text::repaired(&editor.string()),"canUndo":undo.canUndo()}));
                }
                Some("undo") | Some("redo") => {
                    let first = window.owner.ivars().panel.firstResponder();
                    let editor = first
                        .as_ref()
                        .and_then(|r| r.downcast_ref::<NSTextView>())
                        .ok_or("Own active editor missing")?;
                    let undo = editor.undoManager().ok_or("Own active Undo missing")?;
                    if op["op"].as_str() == Some("undo") {
                        if undo.canUndo() {
                            undo.undo();
                        }
                    } else if undo.canRedo() {
                        undo.redo();
                    }
                }
                Some("edit-key") => {
                    let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),NSEventModifierFlags::Command|if op["redo"].as_bool().unwrap_or(false){NSEventModifierFlags::Shift}else{NSEventModifierFlags::empty()},0.,0,None,ns_string!("z"),ns_string!("z"),false,6).ok_or("Own edit event missing")?;
                    let handled = window.owner.ivars().panel.performKeyEquivalent(&event);
                    steps.push(serde_json::json!({"editHandled":handled}));
                }
                Some("unmark") => {
                    if let Some(e) = window
                        .owner
                        .ivars()
                        .panel
                        .firstResponder()
                        .and_then(|r| r.downcast::<NSTextView>().ok())
                    {
                        unsafe {
                            let _: () = msg_send![&*e, unmarkText];
                        }
                    }
                }
                Some("key") | Some("route") | Some("equivalent") => {
                    let code = op["code"].as_u64().unwrap_or(36) as u16;
                    let chars = match code {
                        36 | 76 => "\r",
                        48 => "\t",
                        49 => " ",
                        _ => "",
                    };
                    let chars = NSString::from_str(chars);
                    let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),NSEventModifierFlags(op["flags"].as_u64().unwrap_or(0) as usize),0.,0,None,&chars,&chars,op["repeat"].as_bool().unwrap_or(false),code).ok_or("Own event missing")?;
                    if op["op"].as_str() == Some("route") {
                        let handled = window.owner.guard(|| window.owner.route(&event));
                        steps.push(serde_json::json!({"handled":handled}));
                    } else if op["op"].as_str() == Some("equivalent") {
                        let handled = window.owner.ivars().panel.performKeyEquivalent(&event);
                        steps.push(serde_json::json!({"handled":handled}));
                    } else {
                        window.owner.ivars().panel.sendEvent(&event);
                    }
                }
                Some("insert") => unsafe { window.owner.ivars().insert.performClick(None) },
                Some("cancel") => unsafe { window.owner.ivars().cancel.performClick(None) },
                Some("resign") => unsafe {
                    let _: () = msg_send![&*window.owner.ivars().panel, resignKeyWindow];
                },
                Some("hide") => window.owner.ivars().panel.orderOut(None),
                Some("popup-track") => {
                    if let Some(Control::Choice(p)) = window.owner.ivars().controls.first() {
                        unsafe {
                            p.performClick(None);
                        }
                    }
                }
                Some("show") => {
                    steps.push(serde_json::json!({"show":window.show(Rc::new(||true))}))
                }
                Some("render") => {
                    let root = window
                        .owner
                        .ivars()
                        .panel
                        .contentView()
                        .ok_or("Own view missing")?;
                    root.layoutSubtreeIfNeeded();
                    let bitmap = root
                        .bitmapImageRepForCachingDisplayInRect(root.bounds())
                        .ok_or("Own bitmap allocation failed")?;
                    root.cacheDisplayInRect_toBitmapImageRep(root.bounds(), &bitmap);
                    let data = unsafe {
                        bitmap.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &objc2_foundation::NSDictionary::new(),
                        )
                    }
                    .ok_or("Own PNG failed")?;
                    std::fs::write(directory.join("choice.png"), unsafe {
                        data.as_bytes_unchecked()
                    })
                    .map_err(|e| e.to_string())?;
                }
                _ => return Err("Unknown own native choice operation".into()),
            }
        }
        let panel = &window.owner.ivars().panel;
        let controls=window.owner.ivars().controls.iter().map(|c|match c{Control::Text(t)=>serde_json::json!({"type":"text","value":native_text::repaired(&t.stringValue()),"label":t.accessibilityLabel().map(|s|native_text::repaired(&s)),"enabled":t.isEnabled()}),Control::Choice(c)=>serde_json::json!({"type":"choice","value":c.titleOfSelectedItem().map(|s|native_text::repaired(&s)),"index":c.indexOfSelectedItem(),"label":NSAccessibility::accessibilityLabel(&**c).map(|s|native_text::repaired(&s)),"blockedTracking":c.downcast_ref::<SnippetFormPopup>().map(|p|p.ivars().blocked.get())})}).collect::<Vec<_>>();
        fn screen_text(
            view: &NSView,
            depth: usize,
            output: &mut Vec<String>,
        ) -> Result<(), String> {
            if depth > 64 {
                return Err("Own native view nesting exceeded".into());
            }
            if let Some(text) = view.downcast_ref::<NSTextField>() {
                let text = native_text::repaired(&text.stringValue());
                if !text.is_empty() {
                    output.push(text);
                }
            } else if let Some(button) = view.downcast_ref::<NSButton>() {
                let text = native_text::repaired(&button.title());
                if !text.is_empty() {
                    output.push(text);
                }
            }
            let children = view.subviews();
            for child in children.iter() {
                screen_text(&child, depth + 1, output)?;
            }
            Ok(())
        }
        let mut screen = vec![];
        if let Some(root) = panel.contentView() {
            screen_text(&root, 0, &mut screen)?;
        }
        let events_snapshot = events.borrow().clone();
        let result = serde_json::json!({"ok":true,"steps":steps,"events":events_snapshot,"screenText":screen,"controls":controls,"clipboardReads":reads.get(),"live":window.owner.ivars().live.get(),"failed":window.failed(),"insertEnabled":window.owner.ivars().insert.isEnabled(),"visible":panel.isVisible(),"width":panel.frame().size.width,"height":panel.frame().size.height,"canBecomeKey":panel.canBecomeKeyWindow(),"canBecomeMain":panel.canBecomeMainWindow(),"nonactivating":panel.styleMask().contains(NSWindowStyleMask::NonactivatingPanel),"prohibited":app.activationPolicy()==NSApplicationActivationPolicy::Prohibited,"generalBoardSubstitute":true});
        let stale = window.owner.ivars().insert.clone();
        let weak = Weak::new(&*window.owner);
        drop(window);
        unsafe {
            stale.performClick(None);
        }
        Ok::<_, String>((result, weak))
    })?;
    result["ownerRetired"] = weak.load().is_none().into();
    Ok(result)
}

#[cfg(feature = "test-tools")]
fn drop_fixture(value: serde_json::Value) -> Result<serde_json::Value, String> {
    let main = MainThreadMarker::new().ok_or("Own main required")?;
    let guard =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_fixture_guard".as_ptr()) };
    if guard.is_null() {
        return Err("Complete owned clipboard substitute required".into());
    }
    let guard: unsafe extern "C" fn() -> bool = unsafe { std::mem::transmute(guard) };
    if !unsafe { guard() } {
        return Err("Own guard missing".into());
    }
    NSApplication::sharedApplication(main)
        .setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let slot: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let callback_slot = slot.clone();
    let events = Rc::new(Cell::new(0));
    let recorded = events.clone();
    let (window, owner_weak, panel_weak, raw) = objc2::rc::autoreleasepool(|_| {
        let command =
            serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())?;
        let window = Window::new(
            main,
            Arc::new(Prepared::new(command)?),
            false,
            Callbacks {
                submit_permit: Rc::new(|| Rc::new(|| true)),
                failure: Rc::new(|| {}),
                clipboard: Rc::new(|| Ok(None)),
                event: Rc::new(move |event| {
                    assert!(matches!(event, Event::Cancel { restore: false }));
                    recorded.set(recorded.get() + 1);
                    let window = callback_slot.borrow_mut().take();
                    drop(window);
                }),
            },
        );
        let owner_weak = Weak::new(&*window.owner);
        let panel_weak = Weak::new(&*window.owner.ivars().panel);
        let raw = &*window.owner.ivars().panel as *const SnippetFormPanel;
        Ok::<_, String>((window, owner_weak, panel_weak, raw))
    })?;
    *slot.borrow_mut() = Some(window);
    // Deliberately no caller retain for the panel: the only Window is in slot,
    // and its real cancel callback drops that Window during this native call.
    // Receiver entry leases must protect the ensuing superclass operation.
    objc2::rc::autoreleasepool(|_| unsafe {
        match value["dropDuring"].as_str() {
            Some("close") => {
                let _: () = msg_send![raw, close];
            }
            Some("hide") => {
                let _: () = msg_send![raw,orderOut:None::<&NSObject>];
            }
            Some("resign") => {
                let _: () = msg_send![raw, resignKeyWindow];
            }
            _ => return Err("Unknown own drop phase".to_owned()),
        }
        Ok(())
    })?;
    let leftover = slot.borrow_mut().take();
    let dropped_by_callback = leftover.is_none();
    drop(leftover);
    Ok(
        serde_json::json!({"ok":true,"rootDropped":dropped_by_callback,"events":events.get(),"ownerRetired":owner_weak.load().is_none(),"panelRetired":panel_weak.load().is_none(),"generalBoardSubstitute":true,"prohibited":true}),
    )
}
