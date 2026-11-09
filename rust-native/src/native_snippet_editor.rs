//! Selection-bound link editing and logical line numbers in the existing form.
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    ns_string, NSDictionary, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};
use ruycast_native::snippet_output::{line_starts_utf16, LinkDraft};
use std::cell::{Cell, RefCell};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn guard<T>(callback: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => Some(value),
        Err(payload) => {
            std::mem::forget(payload);
            None
        }
    }
}
struct RulerState {
    text: Weak<NSTextView>,
}
define_class!(
    #[unsafe(super=NSRulerView)] #[thread_kind=MainThreadOnly] #[ivars=RulerState]
    struct LineRuler;
    impl LineRuler {
        #[unsafe(method(drawHashMarksAndLabelsInRect:))]
        fn draw_labels(&self, area:NSRect) { guard(||self.draw_lines(area)); }
    }
);
impl LineRuler {
    fn markers(&self, area: NSRect) -> Vec<(usize, f64)> {
        let Some(text) = self.ivars().text.load() else {
            return vec![];
        };
        // The live NSTextView owns its layout manager; all access stays on
        // this retained main-thread view during the ruler callback.
        let Some(layout) = (unsafe { text.layoutManager() }) else {
            return vec![];
        };
        let Some(container) = (unsafe { text.textContainer() }) else {
            return vec![];
        };
        layout.ensureLayoutForTextContainer(&container);
        let source = text.string();
        let starts = line_starts_utf16(&ruycast_native::native_text::repaired(&source));
        starts
            .into_iter()
            .enumerate()
            .filter_map(|(i, offset)| {
                let fragment = if offset == source.length() {
                    let fragment = layout.extraLineFragmentRect();
                    if fragment.size.height == 0. {
                        rect(0., 0., 0., 16.)
                    } else {
                        fragment
                    }
                } else {
                    unsafe {
                        layout.lineFragmentRectForGlyphAtIndex_effectiveRange(
                            layout.glyphIndexForCharacterAtIndex(offset),
                            std::ptr::null_mut(),
                        )
                    }
                };
                let y = text
                    .convertPoint_toView(
                        NSPoint::new(0., fragment.origin.y + text.textContainerOrigin().y),
                        Some(self),
                    )
                    .y;
                (y + fragment.size.height >= area.origin.y && y <= area.origin.y + area.size.height)
                    .then_some((i + 1, y))
            })
            .collect()
    }
    fn draw_lines(&self, area: NSRect) {
        NSGraphicsContext::saveGraphicsState_class();
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                NSGraphicsContext::restoreGraphicsState_class();
            }
        }
        let _restore = Restore;
        NSBezierPath::bezierPathWithRect(self.bounds()).addClip();
        let Some(text) = self.ivars().text.load() else {
            return;
        };
        let count = line_starts_utf16(&ruycast_native::native_text::repaired(&text.string())).len();
        let width = (count.to_string().len() as f64 * 8. + 20.).max(44.);
        if self.ruleThickness() != width {
            self.setRuleThickness(width);
        }
        NSColor::controlBackgroundColor().setFill();
        NSBezierPath::bezierPathWithRect(self.bounds()).fill();
        NSColor::separatorColor().setFill();
        let bounds = self.bounds();
        NSBezierPath::bezierPathWithRect(rect(
            bounds.size.width - 1.,
            bounds.origin.y,
            1.,
            bounds.size.height,
        ))
        .fill();
        let font =
            NSFont::monospacedDigitSystemFontOfSize_weight(11., unsafe { NSFontWeightRegular });
        let color = NSColor::secondaryLabelColor();
        let attributes = NSDictionary::from_slices(
            &[unsafe { NSFontAttributeName }, unsafe {
                NSForegroundColorAttributeName
            }],
            &[&*font as &AnyObject, &*color as &AnyObject],
        );
        for (number, y) in self.markers(area) {
            let label = NSString::from_str(&number.to_string());
            unsafe {
                let size = label.sizeWithAttributes(Some(&attributes));
                label.drawAtPoint_withAttributes(
                    NSPoint::new(width - size.width - 10., y + 2.),
                    Some(&attributes),
                );
            }
        }
    }
}

struct LinkState {
    changed: std::rc::Rc<dyn Fn()>,
    text: Weak<NSTextView>,
    scroll: Weak<NSScrollView>,
    root: Retained<NSView>,
    label: Retained<NSTextField>,
    url: Retained<NSTextField>,
    error: Retained<NSTextField>,
    open: Retained<NSButton>,
    save: Retained<NSButton>,
    cancel: Retained<NSButton>,
    ruler: Retained<LineRuler>,
    draft: RefCell<Option<LinkDraft>>,
    selection: Cell<NSRange>,
    active: Cell<bool>,
    enabled: Cell<bool>,
    heading: Weak<NSTextField>,
    previous_font: Option<Retained<NSFont>>,
    previous_inset: NSSize,
    previous_substitutions: [bool; 3],
    previous_color: Option<Retained<NSColor>>,
    previous_background: bool,
    previous_mask: NSAutoresizingMaskOptions,
    previous_scroll_clip: bool,
}
define_class!(
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[ivars=LinkState]
    struct LinkController;
    impl LinkController {
        #[unsafe(method(openLink:))]
        fn open_link(&self,_:Option<&NSObject>){guard(||self.open());}
        #[unsafe(method(saveLink:))]
        fn save_link(&self,_:Option<&NSObject>){guard(||self.save());}
        #[unsafe(method(cancelLink:))]
        fn cancel_link(&self,_:Option<&NSObject>){guard(||self.cancel());}
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self,_:&objc2_foundation::NSNotification){guard(||self.validate());}
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self,_:&NSControl,text:&NSTextView,selector:Sel)->bool{
            guard(||{
                if !self.ivars().active.get()||self.ivars().draft.borrow().is_none(){return false}
                let marked:bool=unsafe{msg_send![text,hasMarkedText]};
                if marked{return false}
                if selector==sel!(cancelOperation:){self.cancel();return true}
                if selector==sel!(insertNewline:){self.save();return true}
                false
            }).unwrap_or(false)
        }
    }
    unsafe impl NSObjectProtocol for LinkController{}
    unsafe impl NSControlTextEditingDelegate for LinkController{}
    unsafe impl NSTextFieldDelegate for LinkController{}
);
impl LinkController {
    fn open(&self) {
        let s = self.ivars();
        if !s.active.get() || !s.enabled.get() || s.draft.borrow().is_some() {
            return;
        }
        let Some(text) = s.text.load().filter(|t| t.isEditable()) else {
            return;
        };
        let selection = text.selectedRange();
        let draft = LinkDraft::new(
            &ruycast_native::native_text::repaired(&text.string()),
            selection.into(),
        );
        s.selection.set(selection);
        s.label.setStringValue(&NSString::from_str(&draft.label));
        s.url.setStringValue(&NSString::from_str(&draft.url));
        *s.draft.borrow_mut() = Some(draft);
        s.error.setStringValue(ns_string!(""));
        s.root.setHidden(false);
        s.open.setHidden(true);
        if let Some(heading) = s.heading.load() {
            heading.setStringValue(ns_string!("Link"));
        }
        if let Some(scroll) = s.scroll.load() {
            scroll.setHidden(true);
        }
        self.validate();
        (s.changed)();
        if let Some(window) = s.root.window() {
            window.makeFirstResponder(Some(if s.url.stringValue().is_empty() {
                &*s.url
            } else {
                &*s.label
            }));
        }
    }
    fn updated(&self) -> Option<LinkDraft> {
        let s = self.ivars();
        let mut draft = s.draft.borrow().clone()?;
        draft.label = ruycast_native::native_text::repaired(&s.label.stringValue());
        draft.url = ruycast_native::native_text::repaired(&s.url.stringValue());
        Some(draft)
    }
    fn validate(&self) {
        let s = self.ivars();
        let valid = s.active.get()
            && s.enabled.get()
            && self.updated().is_some_and(|d| {
                s.text.load().is_some_and(|t| {
                    d.validation_error(&ruycast_native::native_text::repaired(&t.string()))
                        .is_none()
                })
            });
        s.save.setEnabled(valid);
        if valid {
            s.error.setStringValue(ns_string!(""));
        }
    }
    fn save(&self) {
        let s = self.ivars();
        if !s.active.get() || !s.enabled.get() {
            return;
        }
        let Some(text) = s.text.load().filter(|t| t.isEditable()) else {
            return;
        };
        let Some(draft) = self.updated() else { return };
        let current = ruycast_native::native_text::repaired(&text.string());
        let Some((_, selection)) = draft.apply(&current) else {
            s.error.setStringValue(&NSString::from_str(
                draft
                    .validation_error(&current)
                    .unwrap_or("The snippet could not be updated."),
            ));
            self.validate();
            return;
        };
        let range = draft.range();
        let range = NSRange::new(range.location, range.length);
        let replacement = NSString::from_str(&ruycast_native::snippet_output::link_source(
            &draft.label,
            &draft.url,
        ));
        if !text.shouldChangeTextInRange_replacementString(range, Some(&replacement)) {
            s.error
                .setStringValue(ns_string!("The snippet could not be updated."));
            return;
        }
        // Native edit notifications register undo and preserve the real field's
        // identity; no setString/reset of the user's undo stack.
        text.replaceCharactersInRange_withString(range, &replacement);
        text.didChangeText();
        text.setSelectedRange(NSRange::new(selection.location, selection.length));
        self.finish();
    }
    fn finish(&self) {
        let s = self.ivars();
        s.draft.borrow_mut().take();
        s.root.setHidden(true);
        s.open.setHidden(false);
        if let Some(heading) = s.heading.load() {
            heading.setStringValue(ns_string!("Text"));
        }
        if let Some(scroll) = s.scroll.load() {
            scroll.setHidden(false);
        }
        s.ruler.setNeedsDisplay(true);
        (s.changed)();
        if let (Some(text), Some(window)) = (s.text.load(), s.root.window()) {
            window.makeFirstResponder(Some(&*text));
        }
    }
    fn cancel(&self) {
        let s = self.ivars();
        if !s.active.get() || s.draft.borrow().is_none() {
            return;
        }
        if let Some(text) = s.text.load() {
            let range = s.selection.get();
            let length = text.string().length();
            let location = range.location.min(length);
            text.setSelectedRange(NSRange::new(location, range.length.min(length - location)));
        }
        self.finish();
    }
}
fn button(main: MainThreadMarker, title: &str) -> Retained<NSButton> {
    let button = NSButton::new(main);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setAccessibilityLabel(Some(&NSString::from_str(title)));
    button
}
fn label(main: MainThreadMarker, title: &str) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(title), main);
    field.setFont(Some(&NSFont::systemFontOfSize(12.)));
    field
}
pub struct SnippetEditor {
    controller: Retained<LinkController>,
}
impl SnippetEditor {
    pub fn is_open(&self) -> bool {
        self.controller.ivars().active.get() && self.controller.ivars().draft.borrow().is_some()
    }
    pub fn finish_link(&self, save: bool) -> bool {
        if !self.is_open() {
            return false;
        }
        if save {
            self.controller.save();
        } else {
            self.controller.cancel();
        }
        true
    }
    pub fn new(
        main: MainThreadMarker,
        parent: &NSView,
        text: &NSTextView,
        scroll: &NSScrollView,
        heading: &NSTextField,
        changed: std::rc::Rc<dyn Fn()>,
    ) -> Self {
        let previous_font = text.font();
        let previous_color = text.textColor();
        let previous_background = text.drawsBackground();
        let previous_mask = text.autoresizingMask();
        let previous_scroll_clip = scroll.clipsToBounds();
        let previous_inset = text.textContainerInset();
        let previous_substitutions = [
            text.isAutomaticQuoteSubstitutionEnabled(),
            text.isAutomaticDashSubstitutionEnabled(),
            text.isAutomaticTextReplacementEnabled(),
        ];
        text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
            13.,
            unsafe { NSFontWeightRegular },
        )));
        text.setTextColor(Some(&NSColor::labelColor()));
        text.setDrawsBackground(false);
        text.setAutomaticQuoteSubstitutionEnabled(false);
        text.setAutomaticDashSubstitutionEnabled(false);
        text.setAutomaticTextReplacementEnabled(false);
        text.setTextContainerInset(NSSize::new(8., 10.));
        text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        scroll.setClipsToBounds(true);
        text.setAccessibilityLabel(Some(ns_string!("Snippet text")));
        let ruler = LineRuler::alloc(main).set_ivars(RulerState {
            text: Weak::new(text),
        });
        let ruler: Retained<LineRuler> = unsafe {
            msg_send![super(ruler),initWithScrollView:scroll,orientation:NSRulerOrientation::VerticalRuler]
        };
        ruler.setClientView(Some(text));
        ruler.setClipsToBounds(true);
        ruler.setRuleThickness(44.);
        ruler.setAccessibilityLabel(Some(ns_string!("Line numbers")));
        scroll.setHasVerticalRuler(true);
        scroll.setVerticalRulerView(Some(&ruler));
        scroll.setRulersVisible(true);
        let root = NSView::new(main);
        root.setHidden(true);
        parent.addSubview(&root);
        let name = NSTextField::new(main);
        name.setAccessibilityLabel(Some(ns_string!("Link text")));
        let url = NSTextField::new(main);
        url.setAccessibilityLabel(Some(ns_string!("Link URL")));
        url.setPlaceholderString(Some(ns_string!("https://")));
        let error = NSTextField::wrappingLabelWithString(ns_string!(""), main);
        error.setTextColor(Some(&NSColor::systemRedColor()));
        error.setFont(Some(&NSFont::systemFontOfSize(11.)));
        let open = button(main, "Link…");
        open.setAccessibilityLabel(Some(ns_string!("Add or edit link")));
        parent.addSubview(&open);
        let save = button(main, "Save link");
        let cancel = button(main, "Cancel");
        let text_label = label(main, "Text");
        text_label.setFrame(rect(0., 146., 50., 20.));
        let url_label = label(main, "URL");
        url_label.setFrame(rect(0., 84., 50., 20.));
        for view in [
            &*text_label as &NSView,
            &*url_label as &NSView,
            &*name as &NSView,
            &*url as &NSView,
            &*error as &NSView,
            &*save as &NSView,
            &*cancel as &NSView,
        ] {
            root.addSubview(view);
        }
        let controller = LinkController::alloc(main).set_ivars(LinkState {
            changed,
            text: Weak::new(text),
            scroll: Weak::new(scroll),
            root,
            label: name,
            url,
            error,
            open,
            save,
            cancel,
            ruler,
            draft: RefCell::new(None),
            selection: Cell::new(NSRange::new(0, 0)),
            active: Cell::new(true),
            enabled: Cell::new(true),
            heading: Weak::new(heading),
            previous_font,
            previous_inset,
            previous_substitutions,
            previous_color,
            previous_background,
            previous_mask,
            previous_scroll_clip,
        });
        let controller: Retained<LinkController> = unsafe { msg_send![super(controller), init] };
        let s = controller.ivars();
        unsafe {
            for (button, action) in [
                (&s.open, sel!(openLink:)),
                (&s.save, sel!(saveLink:)),
                (&s.cancel, sel!(cancelLink:)),
            ] {
                button.setTarget(Some(&*controller));
                button.setAction(Some(action));
            }
            for field in [&s.label, &s.url] {
                field.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
            }
        }
        let editor = Self { controller };
        editor.layout();
        editor
    }
    pub fn ruler(&self) -> Weak<NSRulerView> {
        Weak::new(&*self.controller.ivars().ruler)
    }
    pub fn layout(&self) {
        let s = self.controller.ivars();
        let Some(scroll) = s.scroll.load() else {
            return;
        };
        let frame = scroll.frame();
        let w = frame.size.width;
        s.open
            .setFrame(rect((w - 88.).max(0.), frame.origin.y - 26., 88., 24.));
        s.root.setFrame(frame);
        s.label.setFrame(rect(0., 116., w, 26.));
        s.url.setFrame(rect(0., 54., w, 26.));
        s.error.setFrame(rect(0., 28., (w - 160.).max(60.), 26.));
        s.cancel.setFrame(rect((w - 160.).max(0.), 0., 76., 28.));
        s.save.setFrame(rect((w - 80.).max(0.), 0., 80., 28.));
    }
    pub fn enabled(&self, value: bool) {
        let s = self.controller.ivars();
        s.enabled.set(value);
        s.open.setEnabled(value);
        s.label.setEnabled(value);
        s.url.setEnabled(value);
        s.cancel.setEnabled(value);
        self.controller.validate();
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_buttons(&self) -> Vec<Retained<NSButton>> {
        let s = self.controller.ivars();
        vec![s.open.clone(), s.save.clone(), s.cancel.clone()]
    }
    pub fn retire(&self) {
        let s = self.controller.ivars();
        if !s.active.replace(false) {
            return;
        }
        s.draft.borrow_mut().take();
        s.root.setHidden(true);
        s.root.removeFromSuperview();
        s.open.removeFromSuperview();
        unsafe {
            for field in [&s.label, &s.url] {
                field.setDelegate(None);
            }
            for button in [&s.open, &s.save, &s.cancel] {
                button.setTarget(None);
                button.setAction(None);
            }
        }
        if let Some(scroll) = s.scroll.load() {
            scroll.setHidden(false);
            scroll.setRulersVisible(false);
            scroll.setVerticalRulerView(None);
            scroll.setClipsToBounds(s.previous_scroll_clip);
        }
        if let Some(text) = s.text.load() {
            text.setFont(s.previous_font.as_deref());
            text.setTextColor(s.previous_color.as_deref());
            text.setDrawsBackground(s.previous_background);
            text.setAutoresizingMask(s.previous_mask);
            text.setTextContainerInset(s.previous_inset);
            text.setAutomaticQuoteSubstitutionEnabled(s.previous_substitutions[0]);
            text.setAutomaticDashSubstitutionEnabled(s.previous_substitutions[1]);
            text.setAutomaticTextReplacementEnabled(s.previous_substitutions[2]);
        }
    }
    pub fn accessibility(&self) -> Vec<String> {
        let s = self.controller.ivars();
        if s.draft.borrow().is_none() {
            return vec!["Add or edit link".into(), "Line numbers".into()];
        }
        vec![
            "Link text".into(),
            s.label.stringValue().to_string(),
            "Link URL".into(),
            s.url.stringValue().to_string(),
            s.error.stringValue().to_string(),
            "Cancel".into(),
            "Save link".into(),
        ]
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self, value: &serde_json::Value) -> Result<serde_json::Value, String> {
        let s = self.controller.ivars();
        let text = s.text.load().ok_or("Snippet editor retired")?;
        match value["command"].as_str().unwrap_or("read") {
            "open" => unsafe {
                s.open.performClick(None);
            },
            "fill" | "save" | "invoke-save" => {
                if let Some(label) = value["label"].as_str() {
                    s.label.setStringValue(&NSString::from_str(label));
                }
                if let Some(url) = value["url"].as_str() {
                    s.url.setStringValue(&NSString::from_str(url));
                }
                self.controller.validate();
                if value["command"] == "save" {
                    unsafe {
                        s.save.performClick(None);
                    }
                }
                if value["command"] == "invoke-save" {
                    unsafe {
                        let _: () = msg_send![&*self.controller,saveLink:None::<&NSObject>];
                    }
                }
            }
            "cancel" => unsafe {
                s.cancel.performClick(None);
            },
            "selection" => {
                let n = text.string().length();
                let at = value["location"].as_u64().unwrap_or(0).min(n as u64) as usize;
                let len = value["length"].as_u64().unwrap_or(0).min((n - at) as u64) as usize;
                text.setSelectedRange(NSRange::new(at, len));
            }
            "scroll" => {
                let at = value["location"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(text.string().length() as u64) as usize;
                text.scrollRangeToVisible(NSRange::new(at, 0));
                s.ruler.setNeedsDisplay(true);
            }
            "undo" => {
                if let Some(undo) = text.undoManager() {
                    undo.undo();
                }
            }
            "redo" => {
                if let Some(undo) = text.undoManager() {
                    undo.redo();
                }
            }
            "read" => {}
            _ => return Err("Unknown snippet link fixture operation".into()),
        }
        let selection = text.selectedRange();
        let markers = s.ruler.markers(s.ruler.bounds());
        let f = s.ruler.frame();
        let b = s.ruler.bounds();
        Ok(
            serde_json::json!({"open":s.draft.borrow().is_some(),"source":text.string().to_string(),"selection":[selection.location,selection.length],"label":s.label.stringValue().to_string(),"url":s.url.stringValue().to_string(),"saveEnabled":s.save.isEnabled(),"error":s.error.stringValue().to_string(),"lineStarts":line_starts_utf16(&text.string().to_string()),"visibleMarkers":markers,"rulerVisible":s.scroll.load().is_some_and(|v|v.rulersVisible()),"sourceVisible":s.scroll.load().is_some_and(|v|!v.isHidden()),"rulerFrame":[f.origin.x,f.origin.y,f.size.width,f.size.height],"rulerBounds":[b.origin.x,b.origin.y,b.size.width,b.size.height],"canUndo":text.undoManager().is_some_and(|u|u.canUndo()),"active":s.active.get()}),
        )
    }
}
impl Drop for SnippetEditor {
    fn drop(&mut self) {
        self.retire();
    }
}
