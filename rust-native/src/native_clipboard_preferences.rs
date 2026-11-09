//! Clipboard preferences. Host supplies revision-guarded full-settings save and
//! pause callbacks; edited values remain local until Save.
use objc2::{
    define_class, msg_send, rc::Retained, sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
type Save = Rc<dyn Fn(u64, Value)>;
define_class!(
 #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly]
 struct Toggle;
 impl Toggle {
  #[unsafe(method_id(accessibilityValue))]
  fn value(&self)->Retained<objc2_foundation::NSNumber>{objc2_foundation::NSNumber::numberWithBool(self.state()==NSControlStateValueOn)}
 }
);
define_class!(
 #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=Cell<bool>]
 struct ClipboardPreferencesDocument;
 impl ClipboardPreferencesDocument {
  #[unsafe(method(isFlipped))] fn flipped(&self)->bool {true}
  #[unsafe(method(drawRect:))] fn draw(&self,dirty:NSRect){
   crate::native_settings::color(crate::native_settings::palette(self,self.ivars().get()).background).setFill();NSRectFill(dirty);
  }
 }
);
struct Binding {
    revision: u64,
    value: Value,
    save: Save,
    limit: Retained<NSTextField>,
    days: Retained<NSTextField>,
    excluded: Retained<NSTextView>,
    enabled: Retained<Toggle>,
}
define_class!(
 #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=RefCell<Option<Binding>>]
 struct SaveButton;
 impl SaveButton {
  #[unsafe(method(invoke:))]
  fn invoke(&self,_:Option<&NSObject>){if let Some(b)=self.ivars().borrow().as_ref(){let mut value=b.value.clone();value["enabled"]=json!(b.enabled.state()==NSControlStateValueOn);value["limit"]=json!(b.limit.integerValue());value["retentionDays"]=json!(b.days.integerValue());value["excludedApps"]=json!(b.excluded.string().to_string().lines().map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>());(b.save)(b.revision,value);}}
 }
);
define_class!(
 #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=RefCell<Option<Rc<dyn Fn()>>>]
 struct ResumeButton;
 impl ResumeButton {
  #[unsafe(method(invoke:))]
  fn invoke(&self,_:Option<&NSObject>){let callback=self.ivars().borrow().clone();if let Some(callback)=callback{callback();}}
 }
);
pub struct Preferences {
    saved: Retained<NSTextField>,
    resume: RefCell<Retained<ResumeButton>>,
    pub root: Retained<NSScrollView>,
    document: Retained<ClipboardPreferencesDocument>,
    excluded_scroll: Retained<NSScrollView>,
    feedback: Retained<NSTextField>,
    enabled: Retained<Toggle>,
    limit: Retained<NSTextField>,
    days: Retained<NSTextField>,
    excluded: Retained<NSTextView>,
    save: RefCell<Retained<SaveButton>>,
    labels: Vec<Retained<NSTextField>>,
    revision: RefCell<Option<u64>>,
    authority: Cell<[u64; 3]>,
    #[cfg(feature = "test-tools")]
    retained: RefCell<Option<(Retained<SaveButton>, Retained<ResumeButton>)>>,
}
impl Preferences {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let page = NSScrollView::initWithFrame(
            NSScrollView::alloc(main),
            rect(0., 0., width, height - 58.),
        );
        page.setHasVerticalScroller(true);
        page.setAutohidesScrollers(true);
        page.setHidden(true);
        let document = ClipboardPreferencesDocument::alloc(main).set_ivars(Cell::new(false));
        let root: Retained<ClipboardPreferencesDocument> =
            unsafe { msg_send![super(document),initWithFrame:rect(0.,0.,width,550.)] };
        page.setDocumentView(Some(&root));
        parent.addSubview(&page);
        let enabled = Toggle::alloc(main).set_ivars(());
        let enabled: Retained<Toggle> =
            unsafe { msg_send![super(enabled),initWithFrame:rect(0.,0.,300.,30.)] };
        enabled.setButtonType(NSButtonType::Switch);
        enabled.setTitle(&NSString::from_str("Clipboard history"));
        enabled.setFrame(rect(30., height - 150., 300., 30.));
        enabled.setAccessibilityLabel(Some(&NSString::from_str("Clipboard history")));
        root.addSubview(&enabled);
        let mut labels = vec![];
        for (title, y) in [
            ("Limit", height - 210.),
            ("Retention days", height - 260.),
            ("Excluded app bundle IDs", height - 310.),
        ] {
            let f = NSTextField::labelWithString(&NSString::from_str(title), main);
            f.setFrame(rect(30., y, 250., 25.));
            root.addSubview(&f);
            labels.push(f);
        }
        let limit = NSTextField::new(main);
        limit.setFrame(rect(310., height - 210., 130., 28.));
        limit.setAccessibilityLabel(Some(&NSString::from_str("Limit")));
        root.addSubview(&limit);
        let days = NSTextField::new(main);
        days.setFrame(rect(310., height - 260., 130., 28.));
        days.setAccessibilityLabel(Some(&NSString::from_str("Retention days")));
        root.addSubview(&days);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(main),
            rect(30., height - 460., width.min(650.) - 60., 145.),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setBorderType(NSBorderType::BezelBorder);
        let excluded = NSTextView::new(main);
        excluded.setRichText(false);
        excluded.setFont(Some(&NSFont::systemFontOfSize(13.)));
        excluded.setAccessibilityLabel(Some(&NSString::from_str("Excluded app bundle IDs")));
        scroll.setDocumentView(Some(&excluded));
        root.addSubview(&scroll);
        let b = SaveButton::alloc(main).set_ivars(RefCell::new(None));
        let save: Retained<SaveButton> =
            unsafe { msg_send![super(b),initWithFrame:rect(30.,height-510.,100.,30.)] };
        save.setTitle(&NSString::from_str("Save"));
        save.setAccessibilityLabel(Some(&NSString::from_str("Save clipboard settings")));
        save.setBezelStyle(NSBezelStyle::Push);
        unsafe {
            save.setTarget(Some(&*save));
            save.setAction(Some(sel!(invoke:)));
        }
        root.addSubview(&save);
        let saved = NSTextField::labelWithString(&NSString::from_str(""), main);
        saved.setFrame(rect(30., height - 480., 250., 24.));
        root.addSubview(&saved);
        let b = ResumeButton::alloc(main).set_ivars(RefCell::new(None));
        let resume: Retained<ResumeButton> =
            unsafe { msg_send![super(b),initWithFrame:rect(150.,height-510.,180.,30.)] };
        resume.setTitle(&NSString::from_str("Resume recording"));
        resume.setAccessibilityLabel(Some(&NSString::from_str("Resume recording")));
        resume.setBezelStyle(NSBezelStyle::Push);
        unsafe {
            resume.setTarget(Some(&*resume));
            resume.setAction(Some(sel!(invoke:)));
        }
        resume.setHidden(true);
        root.addSubview(&resume);
        let feedback = NSTextField::wrappingLabelWithString(&NSString::new(), main);
        feedback.setAccessibilityLabel(Some(&NSString::from_str("Clipboard status")));
        root.addSubview(&feedback);
        Self {
            saved,
            resume: RefCell::new(resume),
            root: page,
            document: root,
            excluded_scroll: scroll,
            feedback,
            enabled,
            limit,
            days,
            excluded,
            save: RefCell::new(save),
            labels,
            revision: RefCell::new(None),
            authority: Cell::new([0; 3]),
            #[cfg(feature = "test-tools")]
            retained: RefCell::new(None),
        }
    }
    pub fn authority(&self, authority: [u64; 3]) {
        if self.authority.replace(authority) == authority {
            return;
        }
        let old_save = self.save.borrow().clone();
        let old_resume = self.resume.borrow().clone();
        old_save.ivars().borrow_mut().take();
        old_resume.ivars().borrow_mut().take();
        unsafe {
            old_save.setTarget(None);
            old_save.setAction(None);
            old_resume.setTarget(None);
            old_resume.setAction(None);
        }
        old_save.setEnabled(false);
        old_resume.setEnabled(false);
        old_save.removeFromSuperview();
        old_resume.removeFromSuperview();
        let save = SaveButton::alloc(self.root.mtm()).set_ivars(RefCell::new(None));
        let save: Retained<SaveButton> =
            unsafe { msg_send![super(save),initWithFrame:old_save.frame()] };
        save.setTitle(&NSString::from_str("Save"));
        save.setAccessibilityLabel(Some(&NSString::from_str("Save clipboard settings")));
        save.setBezelStyle(NSBezelStyle::Push);
        let resume = ResumeButton::alloc(self.root.mtm()).set_ivars(RefCell::new(None));
        let resume: Retained<ResumeButton> =
            unsafe { msg_send![super(resume),initWithFrame:old_resume.frame()] };
        resume.setTitle(&NSString::from_str("Resume recording"));
        resume.setAccessibilityLabel(Some(&NSString::from_str("Resume recording")));
        resume.setBezelStyle(NSBezelStyle::Push);
        resume.setHidden(true);
        unsafe {
            save.setTarget(Some(&*save));
            save.setAction(Some(sel!(invoke:)));
            resume.setTarget(Some(&*resume));
            resume.setAction(Some(sel!(invoke:)));
        }
        self.document.addSubview(&save);
        self.document.addSubview(&resume);
        *self.save.borrow_mut() = save;
        *self.resume.borrow_mut() = resume;
        *self.revision.borrow_mut() = None;
        self.root.contentView().scrollToPoint(NSPoint::new(0., 0.));
    }
    pub fn apply(&self, value: Option<&Value>, revision: u64, busy: bool, save: Save) {
        self.root.setHidden(value.is_none());
        let Some(value) = value else {
            self.save.borrow().ivars().borrow_mut().take();
            self.resume.borrow().ivars().borrow_mut().take();
            *self.revision.borrow_mut() = None;
            return;
        };
        self.enabled
            .setAccessibilityRole(Some(unsafe { NSAccessibilityCheckBoxRole }));
        self.limit
            .setAccessibilityRole(Some(unsafe { NSAccessibilityTextFieldRole }));
        self.days
            .setAccessibilityRole(Some(unsafe { NSAccessibilityTextFieldRole }));
        self.excluded
            .setAccessibilityRole(Some(unsafe { NSAccessibilityTextAreaRole }));
        self.save
            .borrow()
            .setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        self.resume
            .borrow()
            .setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        if *self.revision.borrow() != Some(revision) {
            self.enabled
                .setState(if value["enabled"].as_bool().unwrap_or(false) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            self.limit
                .setIntegerValue(value["limit"].as_i64().unwrap_or(200) as isize);
            self.days
                .setIntegerValue(value["retentionDays"].as_i64().unwrap_or(30) as isize);
            self.excluded.setString(&NSString::from_str(
                &value["excludedApps"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default(),
            ));
            *self.revision.borrow_mut() = Some(revision);
        }
        self.enabled.setEnabled(!busy);
        self.limit.setEnabled(!busy);
        self.days.setEnabled(!busy);
        self.excluded.setEditable(!busy);
        self.save.borrow().setEnabled(!busy);
        *self.save.borrow().ivars().borrow_mut() = Some(Binding {
            revision,
            value: value.clone(),
            save,
            limit: self.limit.clone(),
            days: self.days.clone(),
            excluded: self.excluded.clone(),
            enabled: self.enabled.clone(),
        });
    }
    pub fn recording(&self, count: usize, paused: bool, resume: Option<Rc<dyn Fn()>>) {
        self.saved
            .setStringValue(&NSString::from_str(&format!("{count} saved items")));
        self.resume.borrow().setHidden(!paused);
        *self.resume.borrow().ivars().borrow_mut() = resume;
    }
    pub fn layout(&self, graphite: bool, message: &str) {
        let size = unsafe { self.root.superview() }
            .map(|v| v.bounds().size)
            .unwrap_or(NSSize::new(680., 480.));
        self.root
            .setFrame(rect(0., 0., size.width, (size.height - 58.).max(1.)));
        let width = self.root.contentSize().width;
        self.document.ivars().set(graphite);
        let colors = crate::native_settings::palette(&self.document, graphite);
        self.root
            .setBackgroundColor(&crate::native_settings::color(colors.background));
        self.document.setNeedsDisplay(true);
        let y = if message.is_empty() { 68. } else { 116. };
        self.document.setFrame(rect(0., 0., width, y + 410.));
        self.saved.setFrame(rect(30., 24., 230., 24.));
        self.resume.borrow().setFrame(rect(280., 20., 180., 30.));
        self.feedback
            .setFrame(rect(30., 58., (width - 60.).max(1.), 48.));
        self.feedback.setStringValue(&NSString::from_str(message));
        self.feedback.setHidden(message.is_empty());
        self.feedback.setTextColor(Some(&NSColor::systemRedColor()));
        self.enabled.setFrame(rect(30., y, 300., 30.));
        for (label, offset) in self.labels.iter().zip([55., 110., 165.]) {
            label.setFrame(rect(30., y + offset, 250., 25.));
            label.setTextColor(Some(&crate::native_settings::color(colors.ink)));
        }
        self.saved
            .setTextColor(Some(&crate::native_settings::color(colors.muted)));
        self.limit.setFrame(rect(310., y + 55., 130., 28.));
        self.days.setFrame(rect(310., y + 110., 130., 28.));
        let input_width = (width.min(710.) - 60.).max(1.);
        self.excluded_scroll
            .setFrame(rect(30., y + 195., input_width, 145.));
        self.excluded.setFrameSize(NSSize::new(
            input_width,
            (self.excluded.string().to_string().lines().count() as f64 * 18. + 20.).max(145.),
        ));
        self.save.borrow().setFrame(rect(30., y + 355., 100., 30.));
    }
    pub fn cleanup(&self) {
        self.save.borrow().ivars().borrow_mut().take();
        self.resume.borrow().ivars().borrow_mut().take();
        self.root.setHidden(true);
        self.revision.borrow_mut().take();
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self, action: &str, y: f64) -> Result<Value, String> {
        match action {
            "resume" => unsafe {
                self.resume.borrow().performClick(None);
            },
            "retain" => {
                *self.retained.borrow_mut() =
                    Some((self.save.borrow().clone(), self.resume.borrow().clone()))
            }
            "stale" => {
                let retained = self.retained.borrow();
                let (save, resume) = retained.as_ref().ok_or("No retained clipboard controls")?;
                unsafe {
                    let _: () = msg_send![&**save,invoke:None::<&NSObject>];
                    let _: () = msg_send![&**resume,invoke:None::<&NSObject>];
                }
            }
            "scroll" => {
                let clip = self.root.contentView();
                let mut r = clip.bounds();
                r.origin.y = y;
                clip.scrollToPoint(clip.constrainBoundsRect(r).origin);
                self.root.reflectScrolledClipView(&clip);
            }
            _ => (),
        }
        let frame = |view: &NSView| {
            let f = view.frame();
            json!([f.origin.x, f.origin.y, f.size.width, f.size.height])
        };
        let save = self.save.borrow();
        let resume = self.resume.borrow();
        let controls=[&*self.enabled as &NSView,&*self.limit as &NSView,&*self.days as &NSView,&*self.excluded as &NSView,&**save as &NSView,&**resume as &NSView].iter().map(|v|json!({"role":NSAccessibility::accessibilityRole(*v).map(|s|s.to_string()),"value":NSAccessibility::accessibilityValue(*v).map(|value|{let text:Retained<NSString>=unsafe{msg_send![&*value,description]};text.to_string()}),"label":v.accessibilityLabel().map(|s|s.to_string()),"hidden":v.isHidden(),"frame":frame(v)})).collect::<Vec<_>>();
        let visible = save.visibleRect();
        Ok(
            json!({"text":self.text(),"accessibility":self.accessibility(),"authority":self.authority.get(),"scrollY":self.root.contentView().bounds().origin.y,"document":frame(&self.document),"viewport":frame(&self.root),"saveFrame":frame(&save),"saveVisibleRect":[visible.origin.x,visible.origin.y,visible.size.width,visible.size.height],"saveEnabled":save.isEnabled(),"controls":controls}),
        )
    }
    #[cfg(feature = "test-tools")]
    pub fn snapshot(&self, path: &std::path::Path) -> Result<(), String> {
        let bitmap = self
            .document
            .bitmapImageRepForCachingDisplayInRect(self.document.bounds())
            .ok_or("Clipboard preferences render unavailable")?;
        self.document
            .cacheDisplayInRect_toBitmapImageRep(self.document.bounds(), &bitmap);
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .ok_or("Clipboard preferences PNG unavailable")?;
        std::fs::write(path, unsafe { data.as_bytes_unchecked() }).map_err(|e| e.to_string())
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_save(&self, value: &Value) -> bool {
        if self.root.isHidden() {
            return false;
        }
        if let Some(n) = value["limit"].as_i64() {
            self.limit.setIntegerValue(n as isize)
        }
        if let Some(n) = value["retentionDays"].as_i64() {
            self.days.setIntegerValue(n as isize)
        }
        if let Some(enabled) = value["enabled"].as_bool() {
            self.enabled.setState(if enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            })
        }
        if let Some(excluded) = value["excludedApps"].as_array() {
            self.excluded.setString(&NSString::from_str(
                &excluded
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        unsafe {
            self.save.borrow().performClick(None);
        }
        true
    }
    pub fn accessibility(&self) -> Vec<String> {
        if self.root.isHidden() {
            return vec![];
        }
        let mut values = vec![
            self.limit.stringValue().to_string(),
            self.days.stringValue().to_string(),
            self.excluded.string().to_string(),
            self.saved.stringValue().to_string(),
        ];
        if !self.feedback.isHidden() {
            values.push(self.feedback.stringValue().to_string());
        }
        for view in [
            &*self.enabled as &NSView,
            &*self.limit as &NSView,
            &*self.days as &NSView,
            &*self.excluded as &NSView,
            &*self.save.borrow() as &NSView,
            &*self.resume.borrow() as &NSView,
        ] {
            if !view.isHidden() {
                if let Some(label) = view.accessibilityLabel() {
                    values.push(label.to_string());
                }
            }
        }
        values
    }
    pub fn text(&self) -> Vec<String> {
        if self.root.isHidden() {
            vec![]
        } else {
            let mut v = vec![self.saved.stringValue().to_string()];
            if !self.feedback.isHidden() {
                v.push(self.feedback.stringValue().to_string());
            }
            if !self.resume.borrow().isHidden() {
                v.push(self.resume.borrow().title().to_string());
            }
            v.extend(
                self.labels
                    .iter()
                    .map(|l| l.stringValue().to_string())
                    .collect::<Vec<_>>(),
            );
            v.extend([
                self.enabled.title().to_string(),
                self.limit.stringValue().to_string(),
                self.days.stringValue().to_string(),
                self.excluded.string().to_string(),
                self.save.borrow().title().to_string(),
            ]);
            v
        }
    }
}
