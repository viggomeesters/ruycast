//! Native selected-command form. Executable/snippet payload is copied only for
//! the explicitly opened editor; the search rows remain metadata-only.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::*;
use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
#[derive(Clone, Copy)]
enum Focus {
    Application,
    AutoQuit,
    AutoQuitRisk,
    Field(usize),
    Text,
    Argument(usize),
}
pub(crate) struct AppButtonState {
    pub owner: RefCell<Option<Weak<Controller>>>,
    pub token: Cell<u64>,
}
define_class!(
    #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=AppButtonState]
    pub(crate) struct AppButton;
    impl AppButton {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self)->bool{self.isEnabled()}
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent){let _lease=self.retain();let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if let Some(owner)=owner{owner.app_button_key(self,event);}}
    }
);

define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    struct Document;
    impl Document {
        #[unsafe(method(isFlipped))]
        fn flipped(&self)->bool{true}
    }
);
struct AdvancedState {
    active: Cell<bool>,
    expanded: Cell<bool>,
    controls: RefCell<Vec<Weak<NSView>>>,
    document: Weak<Document>,
    scroll: Weak<NSScrollView>,
    closed_height: f64,
    open_height: Cell<f64>,
}
define_class!(
    #[unsafe(super=NSButton)]
    #[thread_kind=MainThreadOnly]
    #[ivars=AdvancedState]
    struct AdvancedButton;
    impl AdvancedButton {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self) -> bool { self.isEnabled() }
        #[unsafe(method(toggleAdvanced:))]
        fn toggle(&self, _: Option<&objc2_foundation::NSObject>) {
            let _retained = self.retain();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if self.ivars().active.get() && self.isEnabled() {
                    self.expand(!self.ivars().expanded.get());
                }
            }));
            if let Err(payload) = result { std::mem::forget(payload); }
        }
    }
);
impl AdvancedButton {
    fn expand(&self, expanded: bool) {
        let state = self.ivars();
        let was_expanded = state.expanded.replace(expanded);
        if was_expanded && !expanded {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
        }
        self.setState(if expanded {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        for view in state.controls.borrow().iter().filter_map(Weak::load) {
            view.setHidden(!expanded);
        }
        if let (Some(document), Some(scroll)) = (state.document.load(), state.scroll.load()) {
            document.setFrameSize(NSSize::new(
                document.frame().size.width,
                if expanded {
                    state.open_height.get()
                } else {
                    state.closed_height
                },
            ));
            let clip = scroll.contentView();
            let max_y = (document.frame().size.height - clip.bounds().size.height).max(0.);
            clip.scrollToPoint(NSPoint::new(
                clip.bounds().origin.x,
                clip.bounds().origin.y.min(max_y),
            ));
            scroll.reflectScrolledClipView(&clip);
        }
    }
    fn retire(&self) {
        self.ivars().active.set(false);
        unsafe {
            self.setTarget(None);
            self.setAction(None);
        }
    }
}
pub(crate) struct TextState {
    owner: RefCell<Option<Weak<Controller>>>,
    capture: Cell<bool>,
    ruler: RefCell<Option<Weak<NSRulerView>>>,
}
define_class!(
    #[unsafe(super=NSTextView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=TextState]
    pub(crate) struct CommandText;
    impl CommandText{
        #[unsafe(method(didChangeText))]
        fn changed(&self){
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                unsafe{let _:()=msg_send![super(self),didChangeText];}
                if let Some(ruler)=self.ivars().ruler.borrow().as_ref().and_then(Weak::load){ruler.setNeedsDisplay(true);}
                let owner = self.ivars().owner.borrow().as_ref().and_then(Weak::load);
                if let Some(owner) = owner { owner.date_input_changed(self); }
            }));
            if let Err(payload)=outcome{std::mem::forget(payload);}
        }
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent){
            if self.ivars().capture.get()&&[36,76].contains(&event.keyCode()) {
                let marked:bool=unsafe{msg_send![self,hasMarkedText]};
                let modifiers=event.modifierFlags()&(NSEventModifierFlags::Command|NSEventModifierFlags::Control|NSEventModifierFlags::Option|NSEventModifierFlags::Shift);
                if !marked {
                    if modifiers==NSEventModifierFlags::Shift {unsafe{let _:()=msg_send![super(self),insertNewline:None::<&objc2_foundation::NSObject>];}return;}
                    if modifiers.is_empty()||modifiers==NSEventModifierFlags::Command {let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if let Some(owner)=owner{owner.save_editor();return;}}
                }
            }
            unsafe{let _:()=msg_send![super(self),keyDown:event];}
        }
        #[unsafe(method(doCommandBySelector:))]
        fn command(&self,selector:Sel){
            let marked:bool=unsafe{msg_send![self,hasMarkedText]};
            if selector==objc2::sel!(cancelOperation:) && !marked {
                let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
                if let Some(owner)=owner{owner.cancel_editor();return;}
            }
            unsafe{let _:()=msg_send![super(self),doCommandBySelector:selector];}
        }
    }
);
pub struct Field {
    pub key: &'static str,
    pub input: Retained<NSTextField>,
}
pub struct CommandFields {
    advanced: RefCell<Option<Retained<AdvancedButton>>>,
    #[cfg(feature = "test-tools")]
    retained_advanced: RefCell<Option<Retained<AdvancedButton>>>,
    #[cfg(feature = "test-tools")]
    retained_snippet: RefCell<Vec<Retained<NSButton>>>,
    snippet: RefCell<Option<crate::native_snippet_editor::SnippetEditor>>,
    #[cfg(feature = "test-tools")]
    retained_app: RefCell<Option<Retained<AppButton>>>,
    pub application: RefCell<Option<Retained<AppButton>>>,
    pub app_list: Retained<crate::native_choice::Widget>,
    app_value: RefCell<String>,
    app_changed: Cell<bool>,
    app_choices: RefCell<ruycast_native::app_choice::Choices>,
    app_pending: Cell<bool>,
    edit_enabled: Cell<bool>,
    pub scroll: Retained<NSScrollView>,
    document: Retained<Document>,
    pub fields: RefCell<Vec<Field>>,
    pub text: Retained<CommandText>,
    text_scroll: Retained<NSScrollView>,
    pub output: Retained<NSPopUpButton>,
    pub enabled: Retained<NSButton>,
    pub auto_quit: Retained<NSButton>,
    pub auto_quit_risk: Retained<NSButton>,
    pub arguments: RefCell<Vec<Retained<CommandText>>>,
    pub add_argument: Retained<NSButton>,
    pub remove_arguments: RefCell<Vec<Retained<NSButton>>>,
    base: RefCell<Value>,
    date_preview: RefCell<Option<Retained<NSTextField>>>,
    width: Cell<f64>,
    owner: RefCell<Option<Weak<Controller>>>,
    focus: Cell<Option<Focus>>,
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn label(main: MainThreadMarker, text: &str, y: f64, width: f64) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), main);
    label.setFrame(rect(0., y, width, 20.));
    label.setFont(Some(&NSFont::systemFontOfSize(12.)));
    label
}
fn text_area(
    main: MainThreadMarker,
    width: f64,
    height: f64,
) -> (Retained<NSScrollView>, Retained<CommandText>) {
    let scroll = NSScrollView::new(main);
    scroll.setFrame(rect(0., 0., width, height));
    scroll.setHasVerticalScroller(true);
    scroll.setBorderType(NSBorderType::BezelBorder);
    let text: Retained<CommandText> = unsafe {
        let text = CommandText::alloc(main).set_ivars(TextState {
            owner: RefCell::new(None),
            capture: Cell::new(false),
            ruler: RefCell::new(None),
        });
        msg_send![super(text),initWithFrame:rect(0.,0.,width-4.,height-4.)]
    };
    text.setRichText(false);
    text.setAllowsUndo(true);
    text.setFont(Some(&NSFont::systemFontOfSize(14.)));
    text.setHorizontallyResizable(false);
    text.setVerticallyResizable(true);
    if let Some(container) = unsafe { text.textContainer() } {
        container.setWidthTracksTextView(true);
    }
    scroll.setDocumentView(Some(&text));
    (scroll, text)
}
impl CommandFields {
    pub fn link_open(&self) -> bool {
        self.snippet.borrow().as_ref().is_some_and(|s| s.is_open())
    }
    pub fn finish_link(&self, save: bool) -> bool {
        self.snippet
            .borrow()
            .as_ref()
            .is_some_and(|snippet| snippet.finish_link(save))
    }
    #[cfg(feature = "test-tools")]
    pub fn snippet_probe(&self, value: &Value) -> Result<Value, String> {
        if value["command"] == "stale" {
            for button in self.retained_snippet.borrow().iter() {
                unsafe {
                    button.performClick(None);
                }
            }
            return Ok(
                json!({"targetsRetired":self.retained_snippet.borrow().iter().all(|button|button.target().is_none()),"source":self.text.string().to_string()}),
            );
        }
        if value["command"] == "retain" {
            *self.retained_snippet.borrow_mut() = self
                .snippet
                .borrow()
                .as_ref()
                .ok_or("Snippet editor unavailable")?
                .retain_buttons();
            return Ok(json!({"count":self.retained_snippet.borrow().len()}));
        }
        self.snippet
            .borrow()
            .as_ref()
            .ok_or("Snippet editor unavailable")?
            .probe(value)
    }
    #[cfg(feature = "test-tools")]
    pub fn geometry(&self, request: &Value) -> Value {
        match request["advanced"].as_str() {
            Some("toggle") => {
                if let Some(button) = self.advanced.borrow().as_ref() {
                    unsafe {
                        button.performClick(None);
                    }
                }
            }
            Some("retain") => {
                *self.retained_advanced.borrow_mut() = self.advanced.borrow().clone();
            }
            Some("stale") => {
                if let Some(button) = self.retained_advanced.borrow().as_ref() {
                    unsafe {
                        button.performClick(None);
                    }
                }
            }
            _ => {}
        }
        let frame = |view: &NSView| {
            let f = view.frame();
            json!([f.origin.x, f.origin.y, f.size.width, f.size.height])
        };
        let arguments=self.arguments.borrow().iter().map(|t|json!({"identity":format!("{:p}",&**t),"undo":t.undoManager().is_some_and(|u|u.canUndo()),"selection":[t.selectedRange().location,t.selectedRange().length]})).collect::<Vec<_>>();
        json!({"arguments":arguments,"advanced":self.advanced.borrow().as_ref().map(|b| b.ivars().expanded.get()),"staleAdvancedRetired":self.retained_advanced.borrow().as_ref().is_some_and(|b|b.target().is_none()),"scroll":frame(&self.scroll),"document":frame(&self.document),"fields":self.document.subviews().iter().map(|v|json!({"frame":frame(&v),"hidden":v.isHidden(),"identity":format!("{:p}",&*v),"label":v.accessibilityLabel().map(|s|s.to_string()),"text":v.downcast_ref::<NSTextField>().map(|t|t.stringValue().to_string())})).collect::<Vec<_>>(),"textIdentity":format!("{:p}",&*self.text),"marked":self.has_marked_text(),"undo":self.text.undoManager().is_some_and(|u|u.canUndo())})
    }
    /// Reflow attached controls only: resizing must not replace an active field
    /// editor, marked-text composition, undo manager, or callback identity.
    pub fn layout(&self, frame: NSRect) {
        self.scroll.setFrame(frame);
        self.app_list.set_frame(frame);
        let width = (self.scroll.contentSize().width - 4.).max(1.);
        let old = self.width.replace(width);
        if (old - width).abs() < 0.5 {
            return;
        }
        for child in self.document.subviews() {
            let mut f = child.frame();
            if f.origin.x > 0. && (f.origin.x - (old - 44.)).abs() < 1. {
                f.origin.x = width - 44.;
            } else if (f.size.width - old).abs() < 1. {
                f.size.width = width;
            } else if (f.size.width - (old - 52.)).abs() < 1. {
                f.size.width = (width - 52.).max(1.);
            }
            child.setFrame(f);
            if let Some(scroll) = child.downcast_ref::<NSScrollView>() {
                if let Some(text) = scroll.documentView() {
                    text.setFrameSize(NSSize::new(
                        (scroll.contentSize().width - 4.).max(1.),
                        text.frame().size.height,
                    ));
                }
            }
        }
        self.document
            .setFrameSize(NSSize::new(width, self.document.frame().size.height));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
        if let Some(snippet) = self.snippet.borrow().as_ref() {
            snippet.layout();
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn app_read(&self) -> Value {
        let button = self.application.borrow();
        json!({"value":&*self.app_value.borrow(),"token":button.as_ref().map(|b|b.ivars().token.get()),"title":button.as_ref().map(|b|b.title().to_string()),"enabled":button.as_ref().is_some_and(|b|b.isEnabled()),"expanded":button.as_ref().is_some_and(|b|b.isAccessibilityExpanded()),"axValue":button.as_ref().and_then(|b|b.accessibilityValue()).and_then(|v|v.downcast::<NSString>().ok()).map(|v|v.to_string()),"fieldAXHidden":self.scroll.isAccessibilityHidden(),"pending":self.app_pending.get(),"available":self.app_choices.borrow().0.len(),"list":self.app_list.inspect()})
    }
    #[cfg(feature = "test-tools")]
    pub fn app_probe(&self, action: &str, index: usize) -> Result<(), String> {
        match action {
            "open" => {
                let b = self
                    .application
                    .borrow()
                    .clone()
                    .ok_or("App picker unavailable")?;
                unsafe {
                    b.performClick(None);
                }
            }
            "focus" => self.focus_app(),
            "select" => {
                if !self.app_list.select(index) {
                    return Err("App choice unavailable".into());
                }
            }
            "click" => self.app_list.click(index)?,
            "retain" => {
                self.app_list.retain();
                *self.retained_app.borrow_mut() = self.application.borrow().clone();
            }
            "stale" => self.app_list.stale(index),
            "stale-button" => {
                let b = self
                    .retained_app
                    .borrow()
                    .clone()
                    .ok_or("No old app button")?;
                let owner = self
                    .owner
                    .borrow()
                    .as_ref()
                    .and_then(Weak::load)
                    .ok_or("No app owner")?;
                unsafe {
                    let _: () = msg_send![&*owner,openAppChoice:&*b];
                }
            }
            _ => return Err("Unknown app picker action".into()),
        }
        Ok(())
    }
    fn has_marked_text(&self) -> bool {
        self.scroll
            .window()
            .and_then(|w| w.firstResponder())
            .and_then(|r| r.downcast::<NSTextView>().ok())
            .is_some_and(|t| unsafe { msg_send![&*t, hasMarkedText] })
    }
    fn retire_app(&self) {
        self.app_list.dismiss(false);
        let old = self.application.borrow_mut().take();
        if let Some(b) = old {
            b.ivars().token.set(0);
            *b.ivars().owner.borrow_mut() = None;
            b.setEnabled(false);
            b.setTitle(ns_string!(""));
            unsafe {
                b.setTarget(None);
                b.setAction(None);
                b.setAccessibilityValue(None);
            }
        }
        self.app_value.borrow_mut().clear();
        self.app_changed.set(false);
    }
    fn display_app(&self) {
        let button = self.application.borrow().clone();
        let Some(button) = button else {
            return;
        };
        let id = self.app_value.borrow().clone();
        let title = if self.app_pending.get() {
            "Loading apps…".to_string()
        } else if id.is_empty() {
            "Default app".to_string()
        } else {
            let c = self.app_choices.borrow();
            c.index(&id)
                .and_then(|i| c.0.get(i))
                .map(|e| e.label.clone())
                .unwrap_or_else(|| id.clone())
        };
        button.setTitle(&NSString::from_str(&title));
        unsafe {
            button.setAccessibilityValue(Some(&NSString::from_str(&id)));
        }
        button.setEnabled(
            self.edit_enabled.get() && !self.app_pending.get() && !self.app_list.is_open(),
        );
    }
    pub fn update_apps(&self, choices: ruycast_native::app_choice::Choices, pending: bool) {
        if *self.app_choices.borrow() != choices || self.app_pending.get() != pending {
            *self.app_choices.borrow_mut() = choices;
            self.app_pending.set(pending);
            self.display_app();
        }
    }
    pub fn app_valid(&self, token: u64) -> bool {
        self.edit_enabled.get()
            && self
                .application
                .borrow()
                .as_ref()
                .is_some_and(|b| b.ivars().token.get() == token)
    }
    pub fn open_app(&self, token: u64) {
        if !self.app_valid(token) || self.app_pending.get() || self.has_marked_text() {
            return;
        }
        self.app_list.set_frame(self.scroll.frame());
        let choices = self.app_choices.borrow().clone();
        let id = self.app_value.borrow().clone();
        if self.app_list.open_apps(token, choices, &id) {
            self.scroll.setAccessibilityHidden(true);
            self.controls_enabled(false);
            if let Some(b) = self.application.borrow().as_ref() {
                b.setAccessibilityExpanded(true);
            }
        }
    }
    pub fn close_app(&self, restore: bool) {
        self.scroll.setAccessibilityHidden(false);
        self.controls_enabled(self.edit_enabled.get());
        if let Some(b) = self.application.borrow().as_ref() {
            b.setAccessibilityExpanded(false);
        }
        if restore {
            self.focus_app();
        }
    }
    pub fn commit_app(&self, value: String) {
        self.app_changed.set(true);
        *self.app_value.borrow_mut() = value;
        self.display_app();
        self.focus_app();
    }
    pub fn focus_app(&self) {
        let button = self.application.borrow().clone();
        if let Some(b) = button {
            self.focus.set(Some(Focus::Application));
            self.document.scrollRectToVisible(b.frame());
            if let Some(w) = b.window() {
                w.makeFirstResponder(Some(&b));
            }
        }
    }
    pub fn tab_app(&self, back: bool) {
        self.focus_app();
        if let Some(w) = self.scroll.window() {
            if back {
                w.selectPreviousKeyView(None);
            } else {
                w.selectNextKeyView(None);
            }
        }
    }
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64) -> Self {
        let scroll = NSScrollView::new(main);
        scroll.setFrame(rect(18., 76., width - 84., 238.));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        let document: Retained<Document> = unsafe {
            let d = Document::alloc(main).set_ivars(());
            msg_send![super(d),initWithFrame:rect(0.,0.,width-108.,900.)]
        };
        scroll.setDocumentView(Some(&document));
        parent.addSubview(&scroll);
        let (text_scroll, text) = text_area(main, width - 108., 180.);
        let output = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(main),
            rect(0., 0., 230., 28.),
            false,
        );
        for title in ["Plain text", "Teams", "Mail (HTML)"] {
            output.addItemWithTitle(&NSString::from_str(title));
        }
        let enabled = NSButton::new(main);
        enabled.setButtonType(NSButtonType::Switch);
        enabled.setTitle(ns_string!("Enabled"));
        enabled.setAccessibilityLabel(Some(ns_string!("Enabled")));
        let auto_quit = NSButton::new(main);
        auto_quit.setButtonType(NSButtonType::Switch);
        auto_quit.setTitle(ns_string!("Auto Quit"));
        auto_quit.setAccessibilityLabel(Some(ns_string!("Auto Quit")));
        let auto_quit_risk = NSButton::new(main);
        auto_quit_risk.setButtonType(NSButtonType::Switch);
        auto_quit_risk.setTitle(ns_string!("Allow quitting when activity is unknown"));
        auto_quit_risk.setAccessibilityLabel(Some(ns_string!("Allow quitting when activity is unknown")));
        let add_argument = NSButton::new(main);
        add_argument.setTitle(ns_string!("Add Argument"));
        add_argument.setAccessibilityLabel(Some(ns_string!("Add Argument")));
        Self {
            advanced: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            retained_advanced: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            retained_snippet: RefCell::new(vec![]),
            snippet: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            retained_app: RefCell::new(None),
            application: RefCell::new(None),
            app_list: crate::native_choice::Widget::new(
                main,
                parent,
                rect(18., 76., width - 84., 238.),
            ),
            app_value: RefCell::new(String::new()),
            app_changed: Cell::new(false),
            app_choices: RefCell::new(Default::default()),
            app_pending: Cell::new(true),
            edit_enabled: Cell::new(true),
            scroll,
            document,
            fields: RefCell::new(vec![]),
            text,
            text_scroll,
            output,
            enabled,
            auto_quit,
            auto_quit_risk,
            arguments: RefCell::new(vec![]),
            add_argument,
            remove_arguments: RefCell::new(vec![]),
            base: RefCell::new(Value::Null),
            date_preview: RefCell::new(None),
            width: Cell::new(width - 108.),
            owner: RefCell::new(None),
            focus: Cell::new(None),
        }
    }
    pub fn remember_focus(&self, panel: &NSWindow) {
        let Some(responder) = panel.firstResponder() else {
            return;
        };
        let same = |object: &objc2::runtime::AnyObject| {
            std::ptr::eq(object, &*responder as &objc2::runtime::AnyObject)
        };
        if self.application.borrow().as_ref().is_some_and(|b| same(b)) {
            self.focus.set(Some(Focus::Application));
            return;
        }
        if same(&self.auto_quit) {
            self.focus.set(Some(Focus::AutoQuit));
            return;
        }
        if same(&self.auto_quit_risk) { self.focus.set(Some(Focus::AutoQuitRisk)); return; }
        for (i, f) in self.fields.borrow().iter().enumerate() {
            if f.input.currentEditor().is_some_and(|e| same(&e)) {
                self.focus.set(Some(Focus::Field(i)));
                return;
            }
        }
        if same(&self.text) {
            self.focus.set(Some(Focus::Text));
            return;
        }
        for (i, arg) in self.arguments.borrow().iter().enumerate() {
            if same(arg) {
                self.focus.set(Some(Focus::Argument(i)));
                return;
            }
        }
    }
    pub fn restore_focus(&self, panel: &NSWindow) {
        match self.focus.get() {
            Some(Focus::Application) => self.focus_app(),
            Some(Focus::AutoQuit) => {
                panel.makeFirstResponder(Some(&self.auto_quit));
            }
            Some(Focus::AutoQuitRisk) => { panel.makeFirstResponder(Some(&self.auto_quit_risk)); }
            Some(Focus::Field(i)) => {
                if let Some(f) = self.fields.borrow().get(i) {
                    panel.makeFirstResponder(Some(&f.input));
                }
            }
            Some(Focus::Text) => {
                panel.makeFirstResponder(Some(&self.text));
            }
            Some(Focus::Argument(i)) => {
                if let Some(t) = self.arguments.borrow().get(i) {
                    panel.makeFirstResponder(Some(&**t));
                }
            }
            None => {}
        }
    }
    fn clear_undo(&self) {
        if let Some(manager) = self.scroll.window().and_then(|w| w.undoManager()) {
            manager.removeAllActions();
        }
        if let Some(manager) = self.text.undoManager() {
            manager.removeAllActions();
        }
    }
    pub fn clear(&self) {
        if let Some(label) = self.date_preview.borrow_mut().take() { label.removeFromSuperview(); }
        self.snippet.borrow_mut().take();
        self.text.ivars().ruler.borrow_mut().take();
        self.retire_app();
        self.clear_undo();
        self.detach_fields();
        for child in self.document.subviews().to_vec() {
            child.removeFromSuperview();
        }
        self.fields.borrow_mut().clear();
        self.arguments.borrow_mut().clear();
        self.remove_arguments.borrow_mut().clear();
        self.text.setString(ns_string!(""));
        *self.base.borrow_mut() = Value::Null;
        self.focus.set(None);
    }
    pub fn bind(&self, owner: &Controller) {
        self.app_list.bind_editor(owner);
        *self.owner.borrow_mut() = Some(Weak::new(owner));
        *self.text.ivars().owner.borrow_mut() = Some(Weak::new(owner));
        self.bind_buttons();
    }
    fn bind_buttons(&self) {
        if let Some(owner) = self.owner.borrow().as_ref().and_then(Weak::load) {
            unsafe {
                self.add_argument.setTarget(Some(&owner));
                self.add_argument.setAction(Some(objc2::sel!(addArgument:)));
            }
            for b in self.remove_arguments.borrow().iter() {
                unsafe {
                    b.setTarget(Some(&owner));
                    b.setAction(Some(objc2::sel!(removeArgument:)));
                }
            }
        }
    }
    fn detach_fields(&self) {
        if let Some(button) = self.advanced.borrow_mut().take() {
            button.retire();
        }
        self.snippet.borrow_mut().take();
        self.text.ivars().ruler.borrow_mut().take();
        *self.text.ivars().owner.borrow_mut() = None;
        for text in self.arguments.borrow().iter() {
            *text.ivars().owner.borrow_mut() = None;
        }
        for f in self.fields.borrow().iter() {
            unsafe {
                f.input.setDelegate(None);
            }
        }
        for b in self.remove_arguments.borrow().iter() {
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
        }
    }
    pub fn cleanup(&self) {
        self.retire_app();
        self.app_list.cleanup();
        *self.text.ivars().owner.borrow_mut() = None;
        for t in self.arguments.borrow().iter() {
            *t.ivars().owner.borrow_mut() = None;
        }
        self.detach_fields();
        unsafe {
            self.add_argument.setTarget(None);
            self.add_argument.setAction(None);
        }
        *self.owner.borrow_mut() = None;
    }
    pub fn change_argument(
        &self,
        main: MainThreadMarker,
        remove: Option<usize>,
        record: &crate::native_surface::RecordButton,
    ) -> Result<(), String> {
        let mut value = self.value(record.value())?;
        let args = value["arguments"]
            .as_array_mut()
            .ok_or("Arguments are unavailable")?;
        if let Some(index) = remove {
            if index >= args.len() {
                return Err("Argument changed. Try again.".into());
            }
            args.remove(index);
        } else {
            args.push(json!(""));
        }
        self.load(main, &value, record);
        Ok(())
    }
    fn field(
        &self,
        main: MainThreadMarker,
        key: &'static str,
        title: &str,
        value: &str,
        y: &mut f64,
    ) {
        self.document
            .addSubview(&label(main, title, *y, self.width.get()));
        *y += 24.;
        let input = NSTextField::textFieldWithString(&NSString::from_str(value), main);
        input.setFrame(rect(0., *y, self.width.get(), 28.));
        input.setFont(Some(&NSFont::systemFontOfSize(14.)));
        input.setAccessibilityLabel(Some(&NSString::from_str(title)));
        self.document.addSubview(&input);
        if let Some(owner) = self.owner.borrow().as_ref().and_then(Weak::load) {
            unsafe {
                input.setDelegate(Some(ProtocolObject::from_ref(&*owner)));
            }
        }
        self.fields.borrow_mut().push(Field { key, input });
        *y += 44.;
    }
    pub fn load_input(&self, main: MainThreadMarker, label_text: &str, value: &str, capture: bool) {
        self.clear();
        *self.text.ivars().owner.borrow_mut() = self.owner.borrow().as_ref().cloned();
        *self.base.borrow_mut() = json!({"kind":"input"});
        self.text.ivars().capture.set(capture);
        self.document
            .addSubview(&label(main, label_text, 0., self.width.get()));
        self.text.setString(&NSString::from_str(value));
        self.text
            .setAccessibilityLabel(Some(&NSString::from_str(label_text)));
        self.text_scroll
            .setFrame(rect(0., 26., self.width.get(), 180.));
        self.document.addSubview(&self.text_scroll);
        self.document.setFrame(rect(0., 0., self.width.get(), 212.));
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0., 0.));
    }
    pub fn update_date_preview(&self, preview: Option<&ruycast_native::task_date::TaskDate>) {
        if let Some(preview) = preview {
            let mut current = self.date_preview.borrow_mut();
            let field = current.get_or_insert_with(|| {
                let field = label(self.text.mtm(), "", 214., self.width.get());
                self.document.addSubview(&field);
                field
            });
            field.setFrame(rect(0., 214., self.width.get(), 20.));
            field.setStringValue(&NSString::from_str(&preview.label));
            self.document.setFrame(rect(0., 0., self.width.get(), 240.));
        } else if let Some(field) = self.date_preview.borrow_mut().take() { field.removeFromSuperview(); }
    }
    pub fn load(&self, main: MainThreadMarker, command: &Value, record: &NSButton) {
        let advanced_expanded = self.base.borrow()["id"] == command["id"]
            && self
                .advanced
                .borrow()
                .as_ref()
                .is_some_and(|b| b.ivars().expanded.get());
        self.text.ivars().capture.set(false);
        self.retire_app();
        self.clear_undo();
        self.detach_fields();
        for child in self.document.subviews().to_vec() {
            child.removeFromSuperview();
        }
        self.fields.borrow_mut().clear();
        self.arguments.borrow_mut().clear();
        self.remove_arguments.borrow_mut().clear();
        *self.base.borrow_mut() = command.clone();
        *self.text.ivars().owner.borrow_mut() = self.owner.borrow().as_ref().cloned();
        let kind = command["kind"].as_str().unwrap_or("");
        let mut y = 0.;
        self.field(
            main,
            "title",
            "Name",
            command["title"].as_str().unwrap_or(""),
            &mut y,
        );
        if kind == "snippet" {
            let heading = label(main, "Text", y, self.width.get());
            self.document.addSubview(&heading);
            y += 24.;
            self.text_scroll
                .setFrame(rect(0., y, self.width.get(), 180.));
            self.text.setString(&NSString::from_str(
                command["target"].as_str().unwrap_or(""),
            ));
            self.text.setAccessibilityLabel(Some(ns_string!("Text")));
            self.document.addSubview(&self.text_scroll);
            let owner = self.owner.borrow().as_ref().cloned();
            let snippet = crate::native_snippet_editor::SnippetEditor::new(
                main,
                &self.document,
                &self.text,
                &self.text_scroll,
                &heading,
                std::rc::Rc::new(move || {
                    if let Some(owner) = owner.as_ref().and_then(Weak::load) {
                        owner.snippet_link_changed();
                    }
                }),
            );
            *self.text.ivars().ruler.borrow_mut() = Some(snippet.ruler());
            *self.snippet.borrow_mut() = Some(snippet);
            y += 196.;
            self.document
                .addSubview(&label(main, "Output", y, self.width.get()));
            y += 24.;
            self.output.setFrame(rect(0., y, 230., 28.));
            self.output
                .setAccessibilityLabel(Some(ns_string!("Output")));
            self.output
                .selectItemAtIndex(match command["snippetOutput"].as_str() {
                    Some("teams") => 1,
                    Some("html") => 2,
                    _ => 0,
                });
            self.document.addSubview(&self.output);
            y += 44.;
            self.field(
                main,
                "keyword",
                "Keyword",
                command["keyword"].as_str().unwrap_or(""),
                &mut y,
            );
        } else if kind != "script" {
            let title = match kind {
                "url" => "Website",
                "path" => "Folder",
                "app" => "Application",
                "script" => "Target",
                _ => "Action",
            };
            self.field(
                main,
                "target",
                title,
                command["target"].as_str().unwrap_or(""),
                &mut y,
            );
        }
        if kind == "app" {
            self.auto_quit.setFrame(rect(0., y, self.width.get(), 28.));
            self.auto_quit.setState(if command["_autoQuit"]["enabled"] == true {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            self.document.addSubview(&self.auto_quit);
            y += 36.;
            self.document.addSubview(&label(
                main,
                "May interrupt recordings or helper audio.",
                y,
                self.width.get(),
            ));
            y += 28.;
            self.auto_quit_risk.setFrame(rect(0., y, self.width.get(), 28.));
            self.auto_quit_risk.setState(if command["_autoQuit"]["allowUnverifiedActivity"] == true {
                NSControlStateValueOn
            } else { NSControlStateValueOff });
            self.document.addSubview(&self.auto_quit_risk);
            y += 36.;
            self.field(
                main,
                "autoQuitMinutes",
                "Quit after (minutes)",
                &command["_autoQuit"]["minutes"]
                    .as_u64()
                    .unwrap_or(30)
                    .to_string(),
                &mut y,
            );
        }
        self.field(
            main,
            "aliases",
            "Aliases",
            &command["aliases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            &mut y,
        );
        self.document
            .addSubview(&label(main, "Shortcut", y, self.width.get()));
        y += 24.;
        record.setFrame(rect(0., y, 230., 30.));
        self.document.addSubview(record);
        y += 46.;
        if ["url", "path"].contains(&kind) {
            self.document
                .addSubview(&label(main, "Open with", y, self.width.get()));
            y += 24.;
            *self.app_value.borrow_mut() =
                command["applicationID"].as_str().unwrap_or("").to_string();
            let owner = self.owner.borrow().as_ref().and_then(Weak::load);
            let token = owner
                .as_ref()
                .and_then(|o| o.current_app_editor_token())
                .unwrap_or(0);
            let b = AppButton::alloc(main).set_ivars(AppButtonState {
                owner: RefCell::new(owner.as_ref().map(|o| Weak::new(&**o))),
                token: Cell::new(token),
            });
            let b: Retained<AppButton> =
                unsafe { msg_send![super(b),initWithFrame:rect(0.,y,self.width.get(),28.)] };
            b.setBezelStyle(NSBezelStyle::Push);
            b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            b.setAccessibilityLabel(Some(ns_string!("Open with")));
            b.setImage(
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!("chevron.down"),
                    None,
                )
                .as_deref(),
            );
            b.setImagePosition(NSCellImagePosition::ImageRight);
            unsafe {
                b.setTarget(owner.as_deref().map(|o| &***o));
                b.setAction(Some(objc2::sel!(openAppChoice:)));
            }
            self.document.addSubview(&b);
            *self.application.borrow_mut() = Some(b);
            self.display_app();
            y += 44.;
        }
        self.field(
            main,
            "inputPrompt",
            "Input prompt",
            command["inputPrompt"].as_str().unwrap_or(""),
            &mut y,
        );
        self.enabled.setState(if command["enabled"] == false {
            NSControlStateValueOff
        } else {
            NSControlStateValueOn
        });
        if kind == "script" {
            let closed_height = y + 36.;
            let button = AdvancedButton::alloc(main).set_ivars(AdvancedState {
                active: Cell::new(true),
                expanded: Cell::new(false),
                controls: RefCell::new(vec![]),
                document: Weak::new(&self.document),
                scroll: Weak::new(&self.scroll),
                closed_height,
                open_height: Cell::new(closed_height),
            });
            let button: Retained<AdvancedButton> =
                unsafe { msg_send![super(button), initWithFrame: rect(0., y, 18., 24.)] };
            button.setButtonType(NSButtonType::OnOff);
            button.setBezelStyle(NSBezelStyle::Disclosure);
            button.setTitle(ns_string!(""));
            button.setAccessibilityLabel(Some(ns_string!("Advanced")));
            unsafe {
                button.setTarget(Some(&button));
                button.setAction(Some(objc2::sel!(toggleAdvanced:)));
            }
            self.document.addSubview(&button);
            let title = label(main, "Advanced", y + 2., 120.);
            title.setFrameOrigin(NSPoint::new(24., y + 2.));
            self.document.addSubview(&title);
            y = closed_height;
            let start = self.document.subviews().len();
            self.field(
                main,
                "target",
                "Target",
                command["target"].as_str().unwrap_or(""),
                &mut y,
            );
            self.document
                .addSubview(&label(main, "Arguments", y, self.width.get()));
            y += 24.;
            for (i, arg) in command["arguments"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let (scroll, text) = text_area(main, self.width.get() - 52., 60.);
                scroll.setFrame(rect(0., y, self.width.get() - 52., 60.));
                text.setString(&NSString::from_str(arg.as_str().unwrap_or("")));
                text.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "Argument {}",
                    i + 1
                ))));
                self.document.addSubview(&scroll);
                *text.ivars().owner.borrow_mut() = self.owner.borrow().as_ref().cloned();
                self.arguments.borrow_mut().push(text);
                let remove = NSButton::new(main);
                remove.setTitle(ns_string!("−"));
                remove.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "Remove argument {}",
                    i + 1
                ))));
                remove.setTag(i as isize);
                remove.setFrame(rect(self.width.get() - 44., y, 40., 28.));
                self.document.addSubview(&remove);
                self.remove_arguments.borrow_mut().push(remove);
                y += 76.;
            }
            self.add_argument.setFrame(rect(0., y, 150., 28.));
            self.document.addSubview(&self.add_argument);
            y += 44.;
            self.field(
                main,
                "timeout",
                "Timeout in seconds",
                &command["timeout"]
                    .as_f64()
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                &mut y,
            );
            *button.ivars().controls.borrow_mut() = self
                .document
                .subviews()
                .iter()
                .skip(start)
                .map(|v| Weak::new(&*v))
                .collect();
            button.ivars().open_height.set(y);
            button.expand(advanced_expanded);
            if !advanced_expanded {
                y = closed_height;
            }
            *self.advanced.borrow_mut() = Some(button);
        }
        self.bind_buttons();
        self.document.setFrameSize(NSSize::new(self.width.get(), y));
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0., 0.));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
    }
    pub fn value(&self, hotkey: Option<String>) -> Result<Value, String> {
        if self
            .snippet
            .borrow()
            .as_ref()
            .is_some_and(|snippet| snippet.is_open())
        {
            return Err("Finish or cancel link editing before saving the snippet.".into());
        }
        let mut value = self.base.borrow().clone();
        if self.application.borrow().is_some() && self.app_changed.get() {
            let id = self.app_value.borrow();
            if id.is_empty() {
                value
                    .as_object_mut()
                    .ok_or("Editor unavailable")?
                    .remove("applicationID");
            } else {
                value["applicationID"] = json!(&*id);
            }
        }
        for field in self.fields.borrow().iter() {
            let text = field.input.stringValue().to_string();
            match field.key {
                "autoQuitMinutes" => {
                    let minutes = text.trim().parse::<u32>()
                        .ok()
                        .filter(|minutes| *minutes > 0)
                        .ok_or("Auto Quit minutes must be a positive whole number (1–4294967295).")?;
                    value["_autoQuit"]["minutes"] = json!(minutes);
                    value["_autoQuit"]["enabled"] = json!(self.auto_quit.state() == NSControlStateValueOn);
                    if self.auto_quit_risk.state() == NSControlStateValueOn {
                        value["_autoQuit"]["allowUnverifiedActivity"] = json!(true);
                    } else if let Some(preference) = value["_autoQuit"].as_object_mut() {
                        preference.remove("allowUnverifiedActivity");
                    }
                }
                "aliases" => {
                    let original = value["aliases"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    if text == original {
                        continue;
                    }
                    value["aliases"] = json!(text
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>())
                }
                "timeout" => {
                    if text.trim().is_empty() {
                        value.as_object_mut().unwrap().remove("timeout");
                    } else {
                        let n = text
                            .trim()
                            .parse::<f64>()
                            .map_err(|_| "Timeout must be a number of seconds.")?;
                        if !n.is_finite() {
                            return Err("Timeout must be a finite number of seconds.".into());
                        }
                        value["timeout"] = json!(n);
                    }
                }
                "keyword" | "applicationID" | "inputPrompt" => {
                    if value[field.key].as_str() == Some(text.as_str()) {
                        continue;
                    }
                    if text.is_empty() {
                        value.as_object_mut().unwrap().remove(field.key);
                    } else {
                        value[field.key] = json!(text);
                    }
                }
                _ => value[field.key] = json!(text),
            }
        }
        if value["kind"] == "snippet" {
            value["target"] = json!(self.text.string().to_string());
            let index = self.output.indexOfSelectedItem();
            if index == 0 {
                if value["snippetOutput"].as_str() != Some("plain") {
                    value.as_object_mut().unwrap().remove("snippetOutput");
                }
            } else {
                value["snippetOutput"] = json!(if index == 1 { "teams" } else { "html" });
            }
        }
        if value["kind"] == "script" {
            value["arguments"] = json!(self
                .arguments
                .borrow()
                .iter()
                .map(|t| t.string().to_string())
                .collect::<Vec<_>>());
        }
        if let Some(hotkey) = hotkey {
            value["hotkey"] = json!(hotkey);
        } else {
            value.as_object_mut().unwrap().remove("hotkey");
        }
        // Preserve absent default Enabled unless the checkbox actually changes.
        let enabled = self.enabled.state() == NSControlStateValueOn;
        if enabled != (value["enabled"] != false) {
            value["enabled"] = json!(enabled);
        }
        Ok(value)
    }
    pub fn set_enabled(&self, enabled: bool) {
        if !enabled {
            self.app_list.dismiss(false);
        }
        self.edit_enabled.set(enabled);
        self.controls_enabled(enabled && !self.app_list.is_open());
    }
    fn controls_enabled(&self, enabled: bool) {
        let owner_enabled = enabled;
        if let Some(snippet) = self.snippet.borrow().as_ref() {
            snippet.enabled(enabled);
        }
        let enabled = enabled && !self.link_open();
        if let Some(b) = self.application.borrow().as_ref() {
            b.setEnabled(enabled && !self.app_pending.get());
        }
        for f in self.fields.borrow().iter() {
            f.input.setEnabled(enabled);
        }
        // The hidden source view remains the native undo/edit engine for the
        // link form. Visible parent inputs are locked independently below.
        self.text.setEditable(owner_enabled);
        self.output.setEnabled(enabled);
        self.enabled.setEnabled(enabled);
        self.auto_quit.setEnabled(enabled);
        self.auto_quit_risk.setEnabled(enabled);
        self.add_argument.setEnabled(enabled);
        if let Some(button) = self.advanced.borrow().as_ref() {
            button.setEnabled(enabled);
        }
        for t in self.arguments.borrow().iter() {
            t.setEditable(enabled);
        }
        for b in self.remove_arguments.borrow().iter() {
            b.setEnabled(enabled);
        }
    }
    pub fn accessibility_text(&self) -> Vec<String> {
        if self.app_list.is_open() {
            return self.app_list.accessibility_text();
        }
        let mut result = vec![];
        if let Some(b) = self.application.borrow().as_ref() {
            if let Some(name) = (&**b as &NSView).accessibilityLabel() {
                result.push(name.to_string());
            }
        }
        for field in self.fields.borrow().iter() {
            if field.input.isHidden() {
                continue;
            }
            if let Some(name) = field.input.accessibilityLabel() {
                result.push(name.to_string());
            }
        }
        if self.base.borrow()["kind"] == "input" {
            if let Some(name) = self.text.accessibilityLabel() {
                result.push(name.to_string());
            }
            if let Some(preview) = self.date_preview.borrow().as_ref() {
                result.push(preview.stringValue().to_string());
            }
        }
        if self.base.borrow()["kind"] == "snippet" {
            if let Some(snippet) = self.snippet.borrow().as_ref() {
                result.extend(snippet.accessibility());
            }
            for view in [&*self.text as &NSView, &*self.output as &NSView] {
                if let Some(name) = view.accessibilityLabel() {
                    result.push(name.to_string());
                }
            }
        }
        for arg in self.arguments.borrow().iter() {
            if arg.enclosingScrollView().is_some_and(|s| s.isHidden()) {
                continue;
            }
            if let Some(name) = arg.accessibilityLabel() {
                result.push(name.to_string());
            }
        }
        for button in self
            .remove_arguments
            .borrow()
            .iter()
            .map(|b| &**b)
            .chain([&*self.enabled, &*self.auto_quit, &*self.auto_quit_risk, &*self.add_argument])
        {
            if unsafe { button.superview() }.is_some() && !button.isHidden() {
                if let Some(name) = (button as &NSView).accessibilityLabel() {
                    result.push(name.to_string());
                }
            }
        }
        if self.advanced.borrow().is_some() {
            result.push("Advanced".into());
        }
        result
    }
    pub fn screen_text(&self) -> Vec<String> {
        if self.app_list.is_open() {
            return self.app_list.visible_text();
        }
        let mut result = vec![];
        let clip = self.scroll.contentView();
        let viewport = self
            .document
            .convertRect_fromView(clip.bounds(), Some(&clip));
        for child in self.document.subviews().to_vec() {
            let frame = child.frame();
            if child.isHidden()
                || frame.origin.x + frame.size.width <= viewport.origin.x
                || frame.origin.y + frame.size.height <= viewport.origin.y
                || frame.origin.x >= viewport.origin.x + viewport.size.width
                || frame.origin.y >= viewport.origin.y + viewport.size.height
            {
                continue;
            }
            if let Some(field) = child.downcast_ref::<NSTextField>() {
                result.push(field.stringValue().to_string());
            } else if let Some(popup) = child.downcast_ref::<NSPopUpButton>() {
                if let Some(title) = popup.titleOfSelectedItem() {
                    result.push(title.to_string());
                }
            } else if let Some(button) = child.downcast_ref::<NSButton>() {
                result.push(button.title().to_string());
            } else if let Some(scroll) = child.downcast_ref::<NSScrollView>() {
                if let Some(text) = scroll
                    .documentView()
                    .and_then(|v| v.downcast::<NSTextView>().ok())
                {
                    result.push(text.string().to_string());
                }
            }
        }
        result
    }
}
