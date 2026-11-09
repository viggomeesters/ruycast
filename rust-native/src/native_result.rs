//! Native result controls and selectable readonly detail. Every retained button
//! pins its session token; old controls are disconnected before replacement.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::*;
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
    NSUndoManager, NSUndoManagerDidRedoChangeNotification, NSUndoManagerDidUndoChangeNotification,
};
use ruycast_native::script_session::Surface;
use std::cell::{Cell, RefCell};
struct ButtonState {
    owner: RefCell<Option<Weak<Controller>>>,
    token: u64,
    name: &'static str,
}
define_class!(
    #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=ButtonState]
    struct ResultButton;
    impl ResultButton {
        #[unsafe(method(invoke:))]
        fn invoke(&self,_:Option<&objc2_foundation::NSObject>){
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if let Some(owner)=owner{owner.result_control(self.ivars().token,self.ivars().name);}
        }
    }
);
struct TextState {
    token: u64,
    owner: RefCell<Option<Weak<Controller>>>,
}
define_class!(
    #[unsafe(super=NSTextView)] #[thread_kind=MainThreadOnly] #[ivars=TextState]
    struct ResultText;
    impl ResultText {
        #[unsafe(method(doCommandBySelector:))]
        fn command(&self,selector:objc2::runtime::Sel){
            let marked:bool=unsafe{msg_send![self,hasMarkedText]};
            if selector==sel!(cancelOperation:) && !marked {
                let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if let Some(owner)=owner{owner.result_text_back(self.ivars().token);}return;
            }
            unsafe{let _:()=msg_send![super(self),doCommandBySelector:selector];}
        }
    }
);
struct QueryState {
    owner: RefCell<Option<Weak<Controller>>>,
    token: u64,
    suppress: Cell<bool>,
    undo: Retained<NSUndoManager>,
}
define_class!(
    #[unsafe(super=NSTextView)] #[thread_kind=MainThreadOnly] #[ivars=QueryState]
    struct ResultQuery;
    unsafe impl NSObjectProtocol for ResultQuery {}
    unsafe impl NSTextDelegate for ResultQuery {
        #[unsafe(method(textDidChange:))]
        fn notified(&self,_:&NSNotification){self.publish();}
    }
    unsafe impl NSTextViewDelegate for ResultQuery {}
    impl ResultQuery {
        #[unsafe(method(undoChanged:))]
        fn undo_changed(&self,_:&NSNotification){self.publish();}
        #[unsafe(method_id(undoManager))]
        fn undo(&self)->Option<Retained<NSUndoManager>>{self.ivars().owner.borrow().is_some().then(||self.ivars().undo.clone())}
        #[unsafe(method(doCommandBySelector:))]
        fn command(&self,selector:objc2::runtime::Sel){
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
            let marked:bool=unsafe{msg_send![self,hasMarkedText]};
            if let Some(owner)=owner {
                self.publish();
                if owner.result_query_key(self.ivars().token,&selector.name().to_string_lossy(),marked){return;}
            }else{return;}
            unsafe{let _:()=msg_send![super(self),doCommandBySelector:selector];}
        }
    }
);
impl ResultQuery {
    fn publish(&self) {
        if self.ivars().suppress.get() {
            return;
        }
        let owner = self.ivars().owner.borrow().as_ref().and_then(Weak::load);
        if let Some(owner) = owner {
            let marked: bool = unsafe { msg_send![self, hasMarkedText] };
            owner.result_query_changed(self.ivars().token, &self.string().to_string(), marked);
        }
    }
}
fn new_query(main: MainThreadMarker, frame: NSRect, token: u64) -> Retained<ResultQuery> {
    let text: Retained<ResultQuery> = unsafe {
        let t = ResultQuery::alloc(main).set_ivars(QueryState {
            owner: RefCell::new(None),
            token,
            suppress: Cell::new(false),
            undo: NSUndoManager::new(main),
        });
        msg_send![super(t),initWithFrame:frame]
    };
    text.setDelegate(Some(ProtocolObject::from_ref(&*text)));
    unsafe {
        for name in [
            NSUndoManagerDidUndoChangeNotification,
            NSUndoManagerDidRedoChangeNotification,
        ] {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &text,
                sel!(undoChanged:),
                Some(name),
                Some(&text.ivars().undo),
            );
        }
    }
    text.ivars().undo.setLevelsOfUndo(32);
    text.setRichText(false);
    text.setAllowsUndo(true);
    text.setDrawsBackground(false);
    text.setFont(Some(&NSFont::systemFontOfSize(22.)));
    text.setVerticallyResizable(false);
    text.setHorizontallyResizable(false);
    text.setTextContainerInset(NSSize::new(0., 2.));
    if let Some(container) = unsafe { text.textContainer() } {
        container.setWidthTracksTextView(true);
    }
    text
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn new_text(main: MainThreadMarker, frame: NSRect, token: u64) -> Retained<ResultText> {
    let text: Retained<ResultText> = unsafe {
        let t = ResultText::alloc(main).set_ivars(TextState {
            owner: RefCell::new(None),
            token,
        });
        msg_send![super(t),initWithFrame:frame]
    };
    text.setRichText(false);
    text.setEditable(false);
    text.setSelectable(true);
    text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(13., 0.)));
    text.setHorizontallyResizable(false);
    text.setVerticallyResizable(true);
    if let Some(container) = unsafe { text.textContainer() } {
        container.setWidthTracksTextView(true);
    }
    text.setAccessibilityLabel(Some(&NSString::from_str("Result details")));
    text
}
pub struct ResultPanel {
    pub scroll: Retained<NSScrollView>,
    text: RefCell<Retained<ResultText>>,
    query: RefCell<Retained<ResultQuery>>,
    query_scroll: Retained<NSScrollView>,
    placeholder: Retained<NSTextField>,
    parent: Retained<NSView>,
    buttons: RefCell<Vec<Retained<ResultButton>>>,
    stamp: Cell<(u64, bool, bool)>,
    width: f64,
    #[cfg(feature = "test-tools")]
    undo_role_before: Cell<isize>,
    #[cfg(feature = "test-tools")]
    undo_role_after: Cell<isize>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<ResultButton>>>,
    #[cfg(feature = "test-tools")]
    stale_text: RefCell<Option<Retained<ResultText>>>,
    #[cfg(feature = "test-tools")]
    stale_query: RefCell<Option<Retained<ResultQuery>>>,
}
impl ResultPanel {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let scroll = NSScrollView::new(main);
        scroll.setFrame(rect(18., 76., width - 36., height - 190.));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        let text = new_text(main, rect(0., 0., width - 40., height - 194.), 0);
        scroll.setDocumentView(Some(&text));
        scroll.setHidden(true);
        parent.addSubview(&scroll);
        let placeholder = NSTextField::labelWithString(&NSString::from_str(""), main);
        placeholder.setFrame(rect(28., height - 61., width - 56., 34.));
        placeholder.setFont(Some(&NSFont::systemFontOfSize(22.)));
        placeholder.setTextColor(Some(&NSColor::secondaryLabelColor()));
        placeholder.setHidden(true);
        parent.addSubview(&placeholder);
        let query_scroll = NSScrollView::new(main);
        query_scroll.setFrame(rect(24., height - 64., width - 48., 38.));
        query_scroll.setDrawsBackground(false);
        query_scroll.contentView().setDrawsBackground(false);
        query_scroll.setHidden(true);
        let query = new_query(main, rect(0., 0., width - 48., 38.), 0);
        query_scroll.setDocumentView(Some(&query));
        parent.addSubview(&query_scroll);
        Self {
            scroll,
            text: RefCell::new(text),
            query: RefCell::new(query),
            query_scroll,
            placeholder,
            parent: parent.retain(),
            buttons: RefCell::new(vec![]),
            stamp: Cell::new((0, false, false)),
            width,
            #[cfg(feature = "test-tools")]
            undo_role_before: Cell::new(0),
            #[cfg(feature = "test-tools")]
            undo_role_after: Cell::new(0),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            stale_text: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            stale_query: RefCell::new(None),
        }
    }
    fn clear_buttons(&self) {
        for b in self.buttons.borrow_mut().drain(..) {
            *b.ivars().owner.borrow_mut() = None;
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
            b.removeFromSuperview();
        }
    }
    pub fn apply(&self, owner: &Controller, result: Option<&Surface>, visible: bool) {
        let key = result.map_or((0, false, false), |r| {
            (r.token, r.detail_visible, r.detail.is_some())
        });
        let old = self.stamp.replace(key);
        if old.0 != key.0 {
            *self.text.borrow().ivars().owner.borrow_mut() = None;
            *self.query.borrow().ivars().owner.borrow_mut() = None;
            unsafe {
                NSNotificationCenter::defaultCenter().removeObserver(&self.query.borrow());
            }
            self.query.borrow().setEditable(false);
            self.query.borrow().setSelectable(false);
            self.query.borrow().ivars().undo.removeAllActions();
            if key.0 != 0 {
                let frame = self.text.borrow().frame();
                let fresh = new_text(owner.mtm(), frame, key.0);
                self.scroll.setDocumentView(Some(&fresh));
                *self.text.borrow_mut() = fresh;
                let frame = self.query.borrow().frame();
                let fresh = new_query(owner.mtm(), frame, key.0);
                self.query_scroll.setDocumentView(Some(&fresh));
                *self.query.borrow_mut() = fresh;
            }
        }
        if old != key {
            self.clear_buttons();
            if let Some(r) = result {
                let mut specs = vec![(
                    "Back",
                    if r.detail_visible && r.has_choices {
                        "choices"
                    } else {
                        "close"
                    },
                    18.,
                    90.,
                )];
                if !r.detail_visible {
                    specs.push(("Choose ↵", "choose", 118., 110.));
                    if r.detail.is_some() {
                        specs.push(("Details ⌘D", "details", 238., 90.));
                    }
                    specs.push(("Reload ⌘R", "reload", self.width - 118., 100.));
                }
                for (title, name, x, w) in specs {
                    let b: Retained<ResultButton> = unsafe {
                        let b = ResultButton::alloc(owner.mtm()).set_ivars(ButtonState {
                            owner: RefCell::new(Some(Weak::from_retained(&owner.retain()))),
                            token: r.token,
                            name,
                        });
                        msg_send![super(b),initWithFrame:rect(x,10.,w,28.)]
                    };
                    b.setTitle(&NSString::from_str(title));
                    b.setAccessibilityLabel(Some(&NSString::from_str(title)));
                    unsafe {
                        b.setTarget(Some(&b));
                        b.setAction(Some(sel!(invoke:)));
                    }
                    self.parent.addSubview(&b);
                    self.buttons.borrow_mut().push(b);
                }
            }
        }
        for b in self.buttons.borrow().iter() {
            b.setHidden(!visible);
            b.setEnabled(result.is_some_and(|r| {
                b.ivars().name == "close"
                    || (!r.busy
                        && (b.ivars().name != "choose" || r.query_valid && r.selection.is_some()))
            }));
        }
        let shown = visible && result.is_some_and(|r| r.detail_visible);
        let searching = visible && result.is_some_and(|r| !r.detail_visible);
        self.query_scroll.setHidden(!searching);
        let query = self.query.borrow().clone();
        if old.1 != key.1 {
            #[cfg(feature = "test-tools")]
            self.undo_role_before
                .set(query.ivars().undo.groupingLevel());
            query.breakUndoCoalescing();
            #[cfg(feature = "test-tools")]
            self.undo_role_after.set(query.ivars().undo.groupingLevel());
        }
        *query.ivars().owner.borrow_mut() = searching.then(|| Weak::from_retained(&owner.retain()));
        if let Some(r) = result {
            query.setEditable(searching && !r.busy);
            query.setSelectable(searching && !r.busy);
            query.setAccessibilityLabel(Some(&NSString::from_str(
                if r.placeholder.trim().is_empty() {
                    "Search choices"
                } else {
                    &r.placeholder
                },
            )));
            let marked: bool = unsafe { msg_send![&*query, hasMarkedText] };
            if !marked {
                if let Some(value) = &r.query {
                    if query.string().to_string() != *value {
                        query.ivars().suppress.set(true);
                        query.setString(&NSString::from_str(value));
                        query.ivars().suppress.set(false);
                    }
                }
            }
            self.placeholder
                .setStringValue(&NSString::from_str(&r.placeholder));
            self.placeholder
                .setHidden(!searching || !query.string().is_empty());
            if searching && !r.busy && (old.0 != key.0 || old.1) {
                self.focus_query();
            }
        } else {
            self.placeholder.setHidden(true);
        }
        self.scroll.setHidden(!shown);
        if !shown {
            *self.text.borrow().ivars().owner.borrow_mut() = None;
        }
        if let Some(r) = result.filter(|r| r.detail_visible && shown) {
            *self.text.borrow().ivars().owner.borrow_mut() =
                Some(Weak::from_retained(&owner.retain()));
            let detail = r.detail.as_deref().unwrap_or("");
            if self.text.borrow().string().to_string() != detail {
                self.text.borrow().setString(&NSString::from_str(detail));
            }
        }
    }
    pub fn focus(&self) {
        if !self.scroll.isHidden() {
            if let Some(w) = self.scroll.window() {
                w.makeFirstResponder(Some(&*self.text.borrow()));
            }
        }
    }
    pub fn focus_query(&self) {
        if !self.query_scroll.isHidden() {
            if let Some(w) = self.query_scroll.window() {
                w.makeFirstResponder(Some(&*self.query.borrow()));
                self.query.borrow().breakUndoCoalescing();
            }
        }
    }
    pub fn has_marked(&self) -> bool {
        unsafe { msg_send![&**self.query.borrow(), hasMarkedText] }
    }
    pub fn screen_text(&self) -> Vec<String> {
        let mut out = self
            .buttons
            .borrow()
            .iter()
            .filter(|b| !b.isHidden())
            .map(|b| b.title().to_string())
            .collect::<Vec<_>>();
        if !self.scroll.isHidden() {
            out.push(self.text.borrow().string().to_string());
        }
        if !self.query_scroll.isHidden() {
            out.push(self.query.borrow().string().to_string());
            if !self.placeholder.isHidden() {
                out.push(self.placeholder.stringValue().to_string());
            }
        }
        out
    }
    pub fn accessibility_text(&self) -> Vec<String> {
        let mut out = self
            .buttons
            .borrow()
            .iter()
            .filter(|b| !b.isHidden())
            .filter_map(|b| {
                (&**b as &NSView)
                    .accessibilityLabel()
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>();
        if !self.scroll.isHidden() {
            if let Some(s) = self.text.borrow().accessibilityLabel() {
                out.push(s.to_string());
            }
        }
        if !self.query_scroll.isHidden() {
            if let Some(s) = self.query.borrow().accessibilityLabel() {
                out.push(s.to_string());
            }
        }
        out
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        serde_json::json!({"queryGroupBeforeRole":self.undo_role_before.get(),"queryGroupAfterRole":self.undo_role_after.get(),"detailVisible":!self.scroll.isHidden(),"editable":self.text.borrow().isEditable(),"selectable":self.text.borrow().isSelectable(),"detail":self.text.borrow().string().to_string(),"buttons":self.screen_text(),"scrollMaxY":self.text.borrow().frame().size.height-self.scroll.contentView().bounds().size.height})
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, name: &str) -> bool {
        let b = self
            .buttons
            .borrow()
            .iter()
            .find(|b| b.ivars().name == name && !b.isHidden() && b.isEnabled())
            .cloned();
        if let Some(b) = b {
            unsafe {
                b.performClick(None);
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_button(&self, name: &str) -> bool {
        let b = self
            .buttons
            .borrow()
            .iter()
            .find(|b| b.ivars().name == name)
            .cloned();
        let ok = b.is_some();
        *self.stale.borrow_mut() = b;
        ok
    }
    #[cfg(feature = "test-tools")]
    pub fn click_stale(&self) {
        if let Some(b) = self.stale.borrow().as_ref() {
            unsafe {
                b.performClick(None);
            }
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_text(&self) {
        *self.stale_text.borrow_mut() = Some(self.text.borrow().clone());
    }
    #[cfg(feature = "test-tools")]
    pub fn stale_text_escape(&self) {
        let text = self.stale_text.borrow().clone();
        if let Some(text) = text {
            unsafe {
                let _: () = msg_send![&*text,doCommandBySelector:sel!(cancelOperation:)];
            }
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_query(&self) {
        *self.stale_query.borrow_mut() = Some(self.query.borrow().clone());
    }
    #[cfg(feature = "test-tools")]
    pub fn exercise_stale_query(&self) -> serde_json::Value {
        let old = self.stale_query.borrow().clone();
        if let Some(q) = old {
            let cleared = !q.ivars().undo.canUndo();
            let detached = q.ivars().owner.borrow().is_none();
            if q.ivars().undo.canUndo() {
                q.ivars().undo.undo();
            }
            if q.ivars().undo.canRedo() {
                q.ivars().undo.redo();
            }
            unsafe {
                let _: () = msg_send![&*q,insertText:&*NSString::from_str("OLD QUERY"),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                let _: () = msg_send![&*q,doCommandBySelector:sel!(cancelOperation:)];
            }
            serde_json::json!({"cleared":cleared,"detached":detached,"editable":q.isEditable(),"canUndoAfter":q.ivars().undo.canUndo()})
        } else {
            serde_json::json!({"missing":true})
        }
    }
    pub fn cleanup(&self) {
        self.clear_buttons();
        *self.text.borrow().ivars().owner.borrow_mut() = None;
        *self.query.borrow().ivars().owner.borrow_mut() = None;
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&self.query.borrow());
        }
        self.query.borrow().ivars().undo.removeAllActions();
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_query(&self, owner: &Controller, value: &str) {
        let q = self.query.borrow().clone();
        q.ivars().suppress.set(true);
        q.setString(&NSString::from_str(value));
        q.ivars().suppress.set(false);
        owner.result_query_changed(q.ivars().token, value, false);
    }
    #[cfg(feature = "test-tools")]
    pub fn query_probe(&self, action: &str, value: Option<&str>) -> serde_json::Value {
        let q = self.query.borrow().clone();
        match action {
            "rich-query-type" => {
                let s = NSString::from_str(value.unwrap_or(""));
                unsafe {
                    let _: () = msg_send![&*q,insertText:&*s,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            "rich-query-mark" => {
                let s = NSString::from_str(value.unwrap_or("CA"));
                unsafe {
                    let _: () = msg_send![&*q,setMarkedText:&*s,selectedRange:objc2_foundation::NSRange::new(s.length(),0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            "rich-query-unmark" => unsafe {
                let _: () = msg_send![&*q, unmarkText];
            },
            _ => {}
        }
        let marked: bool = unsafe { msg_send![&*q, hasMarkedText] };
        serde_json::json!({"text":q.string().to_string(),"marked":marked,"canUndo":q.ivars().undo.canUndo(),"groupingLevel":q.ivars().undo.groupingLevel()})
    }
}
