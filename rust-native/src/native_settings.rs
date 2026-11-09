//! General and Advanced share the existing management window and recorder lease.
use crate::{native::Controller, native_surface::RecordButton};
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::settings_surface::Surface;
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
pub(crate) fn color(rgb: [f64; 3]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(rgb[0], rgb[1], rgb[2], 1.)
}
pub(crate) fn palette(view: &NSView, graphite: bool) -> ruycast_native::settings_surface::Palette {
    ruycast_native::settings_surface::Palette::new(
        view.effectiveAppearance()
            .name()
            .to_string()
            .to_lowercase()
            .contains("dark"),
        graphite,
    )
}
struct Binding {
    owner: Weak<Controller>,
    revision: u64,
    epoch: u64,
    dismissal: u64,
    key: String,
    value: serde_json::Value,
}
define_class!(
    #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=RefCell<Option<Binding>>]
    struct SettingButton;
    impl SettingButton {
        #[unsafe(method(acceptsFirstResponder))]fn accepts(&self)->bool{true}
        #[unsafe(method(invoke:))] fn invoke(&self,_:Option<&NSObject>){
            let b=self.ivars().borrow().as_ref().map(|b|(b.owner.clone(),b.revision,b.epoch,b.dismissal,b.key.clone(),b.value.clone()));
            if let Some((owner,revision,epoch,dismissal,key,value))=b {if let Some(owner)=owner.load(){owner.settings_control_bound(revision,epoch,dismissal,&key,value);}}
        }
    }
);
struct PopupBinding {
    owner: Weak<Controller>,
    revision: u64,
    epoch: u64,
    dismissal: u64,
}
define_class!(
    #[unsafe(super=NSPopUpButton)] #[thread_kind=MainThreadOnly] #[ivars=RefCell<Option<PopupBinding>>]
    struct ThemePopup;
    impl ThemePopup {
        #[unsafe(method(invoke:))] fn invoke(&self,_:Option<&NSObject>){
            let b=self.ivars().borrow().as_ref().map(|b|(b.owner.clone(),b.revision,b.epoch,b.dismissal));
            if let Some((owner,revision,epoch,dismissal))=b {if let Some(owner)=owner.load(){let index=self.indexOfSelectedItem() as usize;if let Some(value)=["system","light","dark","graphite"].get(index){owner.settings_control_bound(revision,epoch,dismissal,"theme",serde_json::json!(value));}}}
        }
    }
);
define_class!(
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=Cell<bool>]
    struct Document;
    impl Document {
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool{true}
        #[unsafe(method(drawRect:))] fn draw(&self,dirty:NSRect){color(palette(self,self.ivars().get()).background).setFill();NSRectFill(dirty);}
    }
);
pub struct Settings {
    pub scroll: Retained<NSScrollView>,
    document: Retained<Document>,
    current: RefCell<Option<Arc<Surface>>>,
    buttons: RefCell<Vec<Retained<SettingButton>>>,
    popup: RefCell<Option<Retained<ThemePopup>>>,
    fields: RefCell<Vec<Retained<NSTextField>>>,
    pub records: [Retained<RecordButton>; 2],
    selected: Cell<Option<usize>>,
    serial: Cell<u64>,
    dialog: RefCell<Option<Retained<NSOpenPanel>>>,
    dialog_serial: Cell<u64>,
    dialog_token: Cell<u64>,
    fixture: Cell<bool>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<SettingButton>>>,
    observed_permissions: Cell<(bool, bool)>,
    opened: RefCell<Vec<String>>,
    width: Cell<f64>,
    palette: Cell<ruycast_native::settings_surface::Palette>,
    feedback: RefCell<Option<Retained<NSTextField>>>,
}
impl Settings {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(main),
            rect(0., 0., width, height - 58.),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(true);
        scroll.setBackgroundColor(&NSColor::windowBackgroundColor());
        scroll.setHidden(true);
        let document: Retained<Document> = unsafe {
            let v = Document::alloc(main).set_ivars(Cell::new(false));
            msg_send![super(v),initWithFrame:rect(0.,0.,width,700.)]
        };
        scroll.setDocumentView(Some(&document));
        parent.addSubview(&scroll);
        let records = [RecordButton::settings(main), RecordButton::settings(main)];
        for r in &records {
            r.setHidden(true);
            document.addSubview(r);
        }
        Self {
            scroll,
            document,
            current: RefCell::new(None),
            buttons: RefCell::new(vec![]),
            popup: RefCell::new(None),
            fields: RefCell::new(vec![]),
            records,
            selected: Cell::new(None),
            serial: Cell::new(1),
            dialog: RefCell::new(None),
            dialog_serial: Cell::new(0),
            dialog_token: Cell::new(0),
            fixture: Cell::new(false),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
            observed_permissions: Cell::new((false, false)),
            opened: RefCell::new(vec![]),
            width: Cell::new(width),
            palette: Cell::new(palette(parent, false)),
            feedback: RefCell::new(None),
        }
    }
    fn clear(&self) {
        self.stop_recording();
        self.retire_dialog();
        for b in self.buttons.take() {
            b.ivars().borrow_mut().take();
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
            b.removeFromSuperview();
        }
        if let Some(p) = self.popup.take() {
            p.ivars().borrow_mut().take();
            unsafe {
                p.setTarget(None);
                p.setAction(None);
            }
            p.removeFromSuperview();
        }
        self.feedback.borrow_mut().take();
        for f in self.fields.take() {
            f.removeFromSuperview();
        }
        for r in &self.records {
            r.setHidden(true);
        }
    }
    pub fn cleanup(&self) {
        self.clear();
        self.current.borrow_mut().take();
        for r in &self.records {
            r.cleanup_settings();
        }
    }
    #[allow(clippy::too_many_arguments)] // Native label content and explicit frame.
    fn text(&self, main: MainThreadMarker, value: &str, x: f64, y: f64, w: f64, h: f64, size: f64) {
        let f = NSTextField::wrappingLabelWithString(&NSString::from_str(value), main);
        f.setFrame(rect(x, y, w, h));
        f.setFont(Some(&NSFont::systemFontOfSize(size)));
        f.setTextColor(Some(&color(if size < 13. {
            self.palette.get().muted
        } else {
            self.palette.get().ink
        })));
        f.setAccessibilityLabel(Some(&NSString::from_str(value)));
        self.document.addSubview(&f);
        self.fields.borrow_mut().push(f);
    }
    #[allow(clippy::too_many_arguments)] // Binding, accessibility label and explicit frame.
    fn button(
        &self,
        owner: &Controller,
        s: &Surface,
        key: &str,
        title: &str,
        value: serde_json::Value,
        x: f64,
        y: f64,
        w: f64,
        toggle: Option<bool>,
    ) {
        let b = SettingButton::alloc(owner.mtm()).set_ivars(RefCell::new(Some(Binding {
            owner: Weak::new(owner),
            revision: s.revision,
            epoch: s.epoch,
            dismissal: s.dismissal,
            key: key.into(),
            value,
        })));
        let b: Retained<SettingButton> =
            unsafe { msg_send![super(b),initWithFrame:rect(x,y,w,30.)] };
        b.setAccessibilityElement(true);
        b.setAccessibilityRole(Some(unsafe {
            if toggle.is_some() {
                NSAccessibilityCheckBoxRole
            } else {
                NSAccessibilityButtonRole
            }
        }));
        b.setTitle(&NSString::from_str(if toggle.is_some() {
            ""
        } else {
            title
        }));
        b.setAccessibilityLabel(Some(&NSString::from_str(title)));
        b.setEnabled(!s.busy);
        if let Some(value) = toggle {
            b.setButtonType(NSButtonType::Switch);
            unsafe {
                b.setAccessibilityValue(Some(&objc2_foundation::NSNumber::numberWithBool(value)));
            }
            b.setState(if value {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        unsafe {
            b.setTarget(Some(&*b));
            b.setAction(Some(sel!(invoke:)));
        }
        self.document.addSubview(&b);
        self.buttons.borrow_mut().push(b);
    }
    pub fn apply(&self, owner: &Controller, s: Option<Arc<Surface>>) {
        let size = unsafe { self.scroll.superview() }
            .map(|v| v.bounds().size)
            .unwrap_or(NSSize::new(760., 490.));
        self.scroll
            .setFrame(rect(0., 0., size.width, size.height - 58.));
        self.scroll.setHidden(s.is_none());
        let Some(s) = s else {
            if self.current.borrow().is_some() {
                self.clear();
                self.current.borrow_mut().take();
            }
            return;
        };
        let graphite = owner.management_graphite();
        self.document.ivars().set(graphite);
        let colors = palette(&self.document, graphite);
        let colors_changed = self.palette.replace(colors) != colors;
        self.scroll.setBackgroundColor(&color(colors.background));
        self.document.setNeedsDisplay(true);
        let permission = permissions(owner.settings_hidden());
        let permissions_changed = self.observed_permissions.replace(permission) != permission;
        if !permissions_changed
            && !colors_changed
            && self.width.get() == size.width
            && self
                .current
                .borrow()
                .as_ref()
                .is_some_and(|old| **old == *s)
        {
            return;
        }
        let old = self.current.borrow().clone();
        // Do not retire a preparing or active recorder for passive writer/error updates.
        let same = !permissions_changed
            && !colors_changed
            && old.as_ref().is_some_and(|v| {
                v.tab == s.tab
                    && v.revision == s.revision
                    && v.epoch == s.epoch
                    && v.dismissal == s.dismissal
            })
            && self.width.get() == size.width;
        if same {
            for b in self.buttons.borrow().iter() {
                b.setEnabled(!s.busy);
            }
            for r in &self.records {
                r.setEnabled(!s.busy);
            }
            if let Some(p) = &*self.popup.borrow() {
                p.setEnabled(!s.busy);
            }
            if let Some(f) = &*self.feedback.borrow() {
                f.setStringValue(&NSString::from_str(&s.message));
                f.setAccessibilityLabel(Some(&NSString::from_str(&s.message)));
                f.setHidden(s.message.is_empty());
            }
            self.document.setFrame(rect(
                0.,
                0.,
                size.width,
                if s.message.is_empty() { 550. } else { 670. },
            ));
            *self.current.borrow_mut() = Some(s);
            return;
        }
        let position = self.scroll.contentView().bounds().origin;
        self.clear();
        self.width.set(size.width);
        self.serial
            .set(self.serial.get().checked_add(1).unwrap_or(0));
        let w = (size.width - 60.).min(650.);
        let x = (size.width - w) / 2.;
        let right = x + w - 190.;
        let main = owner.mtm();
        self.text(
            main,
            if s.tab == "general" {
                "General"
            } else {
                "Advanced"
            },
            x,
            24.,
            w,
            30.,
            21.,
        );
        if s.tab == "general" {
            for (i, (label, key)) in [
                ("Open Launcher", "launcherHotkey"),
                ("Clipboard History", "clipboardHotkey"),
            ]
            .iter()
            .enumerate()
            {
                let y = 78. + i as f64 * 55.;
                self.text(main, label, x, y + 4., w - 210., 28., 13.);
                let r = &self.records[i];
                r.bind_settings(owner);
                r.setFrame(rect(right, y, 190., 30.));
                r.setHidden(false);
                r.setEnabled(!s.busy);
                r.configure(s.configuration[key].as_str().map(str::to_owned));
                r.ivars().separate.set(
                    s.configuration["separateCommandKeys"]
                        .as_bool()
                        .unwrap_or(true),
                );
                unsafe {
                    r.setTag(i as isize);
                    r.setTarget(Some(owner));
                    r.setAction(Some(sel!(recordSetting:)));
                }
            }
            for (label, key, y, default) in [
                ("Caps Lock as Hyper", "hyperKeyEnabled", 188., false),
                (
                    "Separate Left and Right Command",
                    "separateCommandKeys",
                    243.,
                    true,
                ),
                ("Menu Bar Icon", "menuBarVisible", 423., true),
            ] {
                let value = s.configuration[key].as_bool().unwrap_or(default);
                self.text(main, label, x, y + 4., w - 100., 30., 13.);
                self.button(
                    owner,
                    &s,
                    key,
                    label,
                    serde_json::json!(!value),
                    x + w - 44.,
                    y,
                    44.,
                    Some(value),
                );
            }
            let separate = s.configuration["separateCommandKeys"]
                .as_bool()
                .unwrap_or(true);
            self.text(
                main,
                if separate {
                    "Record L⌘ or R⌘ shortcuts to launch apps independently."
                } else {
                    "L⌘ and R⌘ shortcuts are paused. New shortcuts use either Command key."
                },
                x,
                286.,
                w,
                44.,
                12.,
            );
            self.text(main, "Theme", x, 360., w - 210., 28., 13.);
            let p = ThemePopup::alloc(main).set_ivars(RefCell::new(Some(PopupBinding {
                owner: Weak::new(owner),
                revision: s.revision,
                epoch: s.epoch,
                dismissal: s.dismissal,
            })));
            let p: Retained<ThemePopup> = unsafe {
                msg_send![super(p),initWithFrame:rect(right,354.,190.,30.), pullsDown:false]
            };
            for title in ["System", "Light", "Dark", "Graphite"] {
                p.addItemWithTitle(&NSString::from_str(title));
            }
            p.selectItemAtIndex(
                ["system", "light", "dark", "graphite"]
                    .iter()
                    .position(|v| Some(*v) == s.configuration["theme"].as_str())
                    .unwrap_or(0) as isize,
            );
            p.setAccessibilityElement(true);
            p.setAccessibilityRole(Some(unsafe { NSAccessibilityPopUpButtonRole }));
            p.setAccessibilityLabel(Some(&NSString::from_str("Theme")));
            p.setEnabled(!s.busy);
            unsafe {
                p.setTarget(Some(&*p));
                p.setAction(Some(sel!(invoke:)));
            }
            self.document.addSubview(&p);
            *self.popup.borrow_mut() = Some(p);
            // These explicit actions are safe even when permission is already granted.
            let hidden = owner.settings_hidden();
            let (trusted, monitoring) = permissions(hidden);
            if !trusted {
                self.button(
                    owner,
                    &s,
                    "accessibility",
                    "Allow Accessibility…",
                    serde_json::Value::Null,
                    x,
                    490.,
                    210.,
                    None,
                );
            }
            if !monitoring
                && (s.configuration["hyperKeyEnabled"] == true
                    || s.configuration["snippetExpansionEnabled"] == true
                    || separate)
            {
                self.button(
                    owner,
                    &s,
                    "input-monitoring",
                    "Allow Input Monitoring…",
                    serde_json::Value::Null,
                    x + 230.,
                    490.,
                    210.,
                    None,
                );
            }
        } else {
            self.text(main, "Configuration", x, 82., w - 210., 28., 13.);
            self.button(
                owner,
                &s,
                "open-config",
                "Open Configuration…",
                serde_json::Value::Null,
                right,
                76.,
                190.,
                None,
            );
            self.text(main, "Configuration Folder", x, 137., w - 210., 28., 13.);
            self.text(main, &s.directory, right - 40., 135., 230., 56., 12.);
            self.button(
                owner,
                &s,
                "choose-folder",
                "Choose Folder…",
                serde_json::Value::Null,
                right,
                195.,
                190.,
                None,
            );
            self.text(main, "Reload", x, 260., w - 210., 28., 13.);
            self.button(
                owner,
                &s,
                "reload",
                "Reload Configuration",
                serde_json::Value::Null,
                right,
                254.,
                190.,
                None,
            );
            self.text(main, "Permissions", x, 315., w - 210., 28., 13.);
            self.button(
                owner,
                &s,
                "accessibility",
                "Accessibility…",
                serde_json::Value::Null,
                right,
                309.,
                190.,
                None,
            );
            self.text(main, "Version", x, 370., w - 210., 28., 13.);
            self.text(main, &format!("Ruycast {}", rustcast_core::product_version::CURRENT), right, 370., 190., 28., 13.);
        }
        self.text(main, &s.message, x, 545., w, 100., 12.);
        *self.feedback.borrow_mut() = self.fields.borrow().last().cloned();
        if let Some(f) = &*self.feedback.borrow() {
            f.setHidden(s.message.is_empty());
        }
        self.document.setFrame(rect(
            0.,
            0.,
            size.width,
            if s.message.is_empty() { 550. } else { 670. },
        ));
        let same_tab = old.as_ref().is_some_and(|v| v.tab == s.tab);
        *self.current.borrow_mut() = Some(s);
        if same_tab {
            self.scroll.contentView().scrollToPoint(position);
        } else {
            self.scroll
                .contentView()
                .scrollToPoint(NSPoint::new(0., 0.));
        }
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
    }
    pub fn visible(&self) -> bool {
        !self.scroll.isHidden()
    }
    pub fn surface(&self) -> Option<Arc<Surface>> {
        self.current.borrow().clone()
    }
    pub fn stop_recording(&self) {
        self.selected.set(None);
        for r in &self.records {
            r.stop();
        }
    }
    pub fn record(&self, owner: &Controller, index: usize) {
        if !self.live(owner)
            || self
                .current
                .borrow()
                .as_ref()
                .is_none_or(|s| s.tab != "general" || s.busy)
            || index > 1
        {
            return;
        }
        let active = self.records[index].active();
        self.stop_recording();
        if !active {
            self.selected.set(Some(index));
            self.records[index].begin();
        }
    }
    pub fn recorder(&self) -> Option<Retained<RecordButton>> {
        self.selected.get().map(|i| self.records[i].clone())
    }
    pub fn recording_token(&self) -> Option<u64> {
        self.selected
            .get()
            .filter(|_| self.visible())
            .map(|_| self.serial.get())
            .filter(|s| *s != 0)
    }
    pub fn recorded(&self, value: Option<String>) -> Option<(u64, String, serde_json::Value)> {
        let index = self.selected.take()?;
        let s = self.surface()?;
        self.records[index].stop();
        value.map(|v| {
            (
                s.revision,
                if index == 0 {
                    "launcherHotkey"
                } else {
                    "clipboardHotkey"
                }
                .into(),
                serde_json::json!(v),
            )
        })
    }
    pub fn open(&self, owner: &Controller, key: &str, s: &Surface, hidden: bool) -> String {
        if !owner.settings_live(s) {
            return String::new();
        }
        let target = match key {
            "open-config" => s.path.clone(),
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
                    .into()
            }
            "input-monitoring" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent".into()
            }
            _ => return String::new(),
        };
        self.opened.borrow_mut().push(target.clone());
        if hidden {
            return String::new();
        }
        let url = if key == "open-config" {
            Some(objc2_foundation::NSURL::fileURLWithPath(
                &NSString::from_str(&target),
            ))
        } else {
            objc2_foundation::NSURL::URLWithString(&NSString::from_str(&target))
        };
        if url.is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url)) {
            String::new()
        } else {
            "macOS could not open the selected settings item.".into()
        }
    }
    pub fn live(&self, owner: &Controller) -> bool {
        self.surface().is_some_and(|s| owner.settings_live(&s))
    }
    pub fn dialog_open(&self) -> bool {
        self.dialog_token.get() != 0
    }
    pub fn retire_dialog(&self) {
        self.dialog_token.set(0);
        self.fixture.set(false);
        let old = self.dialog.take();
        if let Some(p) = old {
            unsafe {
                p.cancel(None);
            }
        }
    }
    pub fn choose(&self, owner: &Controller, hidden: bool, window: &NSWindow) {
        if !self.live(owner) || self.dialog_open() {
            return;
        }
        let Some(serial) = self.dialog_serial.get().checked_add(1) else {
            return;
        };
        self.dialog_serial.set(serial);
        self.dialog_token.set(serial);
        if hidden {
            self.fixture.set(true);
            return;
        }
        if !self.live(owner) {
            self.retire_dialog();
            return;
        }
        let panel = NSOpenPanel::openPanel(owner.mtm());
        panel.setCanChooseDirectories(true);
        panel.setCanChooseFiles(false);
        panel.setAllowsMultipleSelection(false);
        panel.setCanCreateDirectories(true);
        panel.setPrompt(Some(&NSString::from_str("Use Folder")));
        if !self.live(owner) || self.dialog_token.get() != serial {
            self.retire_dialog();
            return;
        }
        *self.dialog.borrow_mut() = Some(panel.clone());
        let weak = Weak::new(owner);
        let receive = panel.clone();
        let block = block2::RcBlock::new(move |response: NSModalResponse| {
            if let Some(owner) = weak.load() {
                owner.settings_folder_completed(
                    serial,
                    response == NSModalResponseOK,
                    Some(&receive),
                );
            }
        });
        panel.beginSheetModalForWindow_completionHandler(window, &block);
    }
    pub fn finish_dialog(&self, serial: u64) -> bool {
        if serial == 0 || self.dialog_token.get() != serial {
            return false;
        }
        self.dialog_token.set(0);
        self.fixture.set(false);
        self.dialog.take();
        true
    }
    pub fn text_values(&self) -> Vec<String> {
        let mut text = self
            .fields
            .borrow()
            .iter()
            .map(|f| f.stringValue().to_string())
            .collect::<Vec<_>>();
        text.extend(self.buttons.borrow().iter().map(|b| {
            NSAccessibility::accessibilityLabel(&***b)
                .map(|s| s.to_string())
                .unwrap_or_default()
        }));
        if let Some(p) = &*self.popup.borrow() {
            text.push("Theme".into());
            if let Some(t) = p.titleOfSelectedItem() {
                text.push(t.to_string());
            }
        }
        for r in &self.records {
            if !r.isHidden() {
                text.push(r.title().to_string());
            }
        }
        text
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        serde_json::json!({"surface":self.surface().as_deref(),"text":self.text_values(),"visible":self.visible(),"opened":*self.opened.borrow(),"dialogOpen":self.dialog_open(),"dialogSerial":self.dialog_token.get(),"fixtureDialog":self.fixture.get(),"recording":self.records.iter().map(|r|r.recording()).collect::<Vec<_>>(),"preparing":self.records.iter().map(|r|r.preparing()).collect::<Vec<_>>(),"buttons":self.buttons.borrow().iter().map(|b|serde_json::json!({"label":NSAccessibility::accessibilityLabel(&***b).map(|s|s.to_string()),"role":NSAccessibility::accessibilityRole(&***b).map(|s|s.to_string()),"enabled":b.isEnabled(),"value":b.state()})).collect::<Vec<_>>()})
    }
    #[cfg(feature = "test-tools")]
    pub fn retain(&self, index: usize) {
        *self.stale.borrow_mut() = self.buttons.borrow().get(index).cloned();
    }
    #[cfg(feature = "test-tools")]
    pub fn stale(&self) {
        if let Some(b) = self.stale.borrow().clone() {
            unsafe {
                let _: () = msg_send![&*b,invoke:Option::<&NSObject>::None];
            }
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> bool {
        if self.dialog_open() {
            return false;
        }
        let b = self.buttons.borrow().get(index).cloned();
        if let Some(b) = b.filter(|b| b.isEnabled()) {
            unsafe {
                b.performClick(None);
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn theme(&self, index: isize) -> bool {
        let p = self.popup.borrow().clone();
        if let Some(p) = p.filter(|p| p.isEnabled()) {
            p.selectItemAtIndex(index);
            unsafe {
                let _: () = msg_send![&*p,invoke:Option::<&NSObject>::None];
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn snapshot(&self, path: &std::path::Path) -> Result<(), String> {
        let bitmap = self
            .document
            .bitmapImageRepForCachingDisplayInRect(self.document.bounds())
            .ok_or("Settings render unavailable")?;
        self.document
            .cacheDisplayInRect_toBitmapImageRep(self.document.bounds(), &bitmap);
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .ok_or("Settings PNG unavailable")?;
        std::fs::write(path, unsafe { data.as_bytes_unchecked() }).map_err(|e| e.to_string())
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGPreflightListenEventAccess() -> bool;
}
fn permissions(hidden: bool) -> (bool, bool) {
    if hidden {
        (false, false)
    } else {
        unsafe { (AXIsProcessTrusted() != 0, CGPreflightListenEventAccess()) }
    }
}
