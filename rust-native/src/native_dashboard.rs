//! Main-thread dashboard. Controls keep a weak owner and a route-specific token;
//! ordinary data refreshes preserve control identities, focus and scroll.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::analytics_dashboard::{Operation, Point, Surface};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
struct Binding {
    owner: Weak<Controller>,
    token: u64,
    operation: Option<Operation>,
}
struct ButtonState {
    binding: RefCell<Option<Binding>>,
}
define_class!(
    #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=ButtonState]
    struct DashboardButton;
    impl DashboardButton {
        #[unsafe(method(acceptsFirstResponder))]fn accepts(&self)->bool{true}
        #[unsafe(method(invoke:))]
        fn invoke(&self,_:Option<&objc2_foundation::NSObject>) {
            let bound=self.ivars().binding.borrow().as_ref().map(|b|(b.owner.clone(),b.token,b.operation.clone()));
            if let Some((owner,token,operation))=bound {
                if let Some(owner)=owner.load() {
                    if let Some(mut operation)=operation {
                        if let Operation::Enabled{value,..}=&mut operation { *value=self.state()==NSControlStateValueOn; }
                        owner.dashboard_control(token,operation);
                    } else { owner.dashboard_export_dialog(token); }
                }
            }
        }
    }
);
struct PopupState {
    owner: RefCell<Option<Weak<Controller>>>,
    token: Cell<u64>,
    expected: Cell<i64>,
}
define_class!(
    #[unsafe(super=NSPopUpButton)] #[thread_kind=MainThreadOnly] #[ivars=PopupState]
    struct RetentionPopup;
    impl RetentionPopup {
        #[unsafe(method(acceptsFirstResponder))]fn accepts(&self)->bool{true}
        #[unsafe(method(invoke:))]
        fn invoke(&self,_:Option<&objc2_foundation::NSObject>) {
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
            if let (Some(owner),Some(item))=(owner,self.selectedItem()) {
                owner.dashboard_control(self.ivars().token.get(),Operation::Retention{expected:self.ivars().expected.get(),value:item.tag() as i64});
            }
        }
    }
);
define_class!(
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=Cell<bool>]
    struct Document;
    impl Document {
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool {true}
        #[unsafe(method(drawRect:))]
        fn draw(&self,dirty:NSRect){crate::native_settings::color(crate::native_settings::palette(self,self.ivars().get()).background).setFill();NSRectFill(dirty);}
    }
);
struct BarState {
    uses: Option<i64>,
    max: i64,
    accent: [f64; 3],
}
define_class!(
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=BarState]
    struct Bar;
    impl Bar {
        #[unsafe(method(drawRect:))]
        fn draw(&self,_:NSRect) {
            if let Some(n)=self.ivars().uses {
                crate::native_settings::color(self.ivars().accent).setFill();
                let mut r=self.bounds();r.size.height=if n==0 {1.} else {r.size.height*n as f64/self.ivars().max as f64};
                NSRectFill(r);
            } else {
                NSColor::tertiaryLabelColor().setFill();
                let mut r=self.bounds();r.size.height=1.;NSRectFill(r);
            }
        }
    }
);
#[derive(Clone)]
struct Controls {
    buttons: Vec<Retained<DashboardButton>>,
    retention: Retained<RetentionPopup>,
    token: u64,
    confirmation: Option<u64>,
}
impl Controls {
    fn cleanup(&self) {
        for b in &self.buttons {
            b.ivars().binding.borrow_mut().take();
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
            b.removeFromSuperview();
        }
        self.retention.ivars().owner.borrow_mut().take();
        unsafe {
            self.retention.setTarget(None);
            self.retention.setAction(None);
        }
        self.retention.removeFromSuperview();
    }
}
pub struct Dashboard {
    pub scroll: Retained<NSScrollView>,
    document: Retained<Document>,
    controls: RefCell<Option<Controls>>,
    content: RefCell<Vec<Retained<NSView>>>,
    fields: RefCell<Vec<Retained<NSTextField>>>,
    bars: RefCell<Vec<Retained<Bar>>>,
    current: RefCell<Option<Arc<Surface>>>,
    restore_focus: RefCell<Option<Retained<NSView>>>,
    dialog: RefCell<Option<Retained<NSSavePanel>>>,
    dialog_serial: Cell<u64>,
    dialog_token: Cell<u64>,
    fixture_dialog: Cell<bool>,
    width: Cell<f64>,
    palette: Cell<ruycast_native::settings_surface::Palette>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<DashboardButton>>>,
}
impl Dashboard {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let scroll = NSScrollView::new(main);
        scroll.setFrame(rect(0., 0., width, height - 58.));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.setHidden(true);
        let document: Retained<Document> = unsafe {
            let d = Document::alloc(main).set_ivars(Cell::new(false));
            msg_send![super(d),initWithFrame:rect(0.,0.,width,height)]
        };
        scroll.setDocumentView(Some(&document));
        parent.addSubview(&scroll);
        Self {
            scroll,
            document,
            controls: RefCell::new(None),
            content: RefCell::new(vec![]),
            fields: RefCell::new(vec![]),
            bars: RefCell::new(vec![]),
            current: RefCell::new(None),
            restore_focus: RefCell::new(None),
            dialog: RefCell::new(None),
            dialog_serial: Cell::new(0),
            dialog_token: Cell::new(0),
            fixture_dialog: Cell::new(false),
            width: Cell::new(width),
            palette: Cell::new(crate::native_settings::palette(parent, false)),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
        }
    }
    fn button(
        &self,
        owner: &Controller,
        token: u64,
        title: &str,
        operation: Option<Operation>,
    ) -> Retained<DashboardButton> {
        let b: Retained<DashboardButton> = unsafe {
            let b = DashboardButton::alloc(owner.mtm()).set_ivars(ButtonState {
                binding: RefCell::new(Some(Binding {
                    owner: Weak::new(owner),
                    token,
                    operation,
                })),
            });
            msg_send![super(b),initWithFrame:rect(0.,0.,150.,28.)]
        };
        b.setAccessibilityElement(true);
        b.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        b.setTitle(&NSString::from_str(title));
        b.setAccessibilityLabel(Some(&NSString::from_str(title)));
        unsafe {
            b.setTarget(Some(&b));
            b.setAction(Some(sel!(invoke:)));
        }
        self.document.addSubview(&b);
        b
    }
    fn controls(&self, owner: &Controller, s: &Surface) -> Controls {
        if let Some(c) = self
            .controls
            .borrow()
            .as_ref()
            .filter(|c| c.token == s.token && c.confirmation == s.confirmation_token)
            .cloned()
        {
            return c;
        }
        let old = self.controls.borrow_mut().take();
        if let Some(c) = old {
            c.cleanup();
        }
        let buttons = vec![
            self.button(owner, s.token, "Refresh", Some(Operation::Refresh)),
            self.button(
                owner,
                s.token,
                "Local analytics",
                Some(Operation::Enabled {
                    expected: s.enabled,
                    value: !s.enabled,
                }),
            ),
            self.button(owner, s.token, "Export Diagnostics…", None),
            self.button(
                owner,
                s.token,
                "Reset Analytics…",
                Some(Operation::ConfirmReset),
            ),
            self.button(
                owner,
                s.token,
                "Reset Analytics",
                Some(Operation::Reset {
                    confirmation: s.confirmation_token.unwrap_or(0),
                }),
            ),
            self.button(
                owner,
                s.token,
                "Cancel",
                Some(Operation::CancelReset {
                    confirmation: s.confirmation_token.unwrap_or(0),
                }),
            ),
        ];
        buttons[1].setButtonType(NSButtonType::Switch);
        buttons[1].setAccessibilityRole(Some(unsafe { NSAccessibilityCheckBoxRole }));
        let retention: Retained<RetentionPopup> = unsafe {
            let p = RetentionPopup::alloc(owner.mtm()).set_ivars(PopupState {
                owner: RefCell::new(Some(Weak::new(owner))),
                token: Cell::new(s.token),
                expected: Cell::new(s.retention_days),
            });
            msg_send![super(p),initWithFrame:rect(0.,0.,140.,28.),pullsDown:false]
        };
        retention.setAccessibilityElement(true);
        retention.setAccessibilityRole(Some(unsafe { NSAccessibilityPopUpButtonRole }));
        retention.setAccessibilityLabel(Some(&NSString::from_str("Retention")));
        unsafe {
            retention.setTarget(Some(&retention));
            retention.setAction(Some(sel!(invoke:)));
        }
        self.document.addSubview(&retention);
        let c = Controls {
            buttons,
            retention,
            token: s.token,
            confirmation: s.confirmation_token,
        };
        *self.controls.borrow_mut() = Some(c.clone());
        c
    }
    fn text(
        &self,
        main: MainThreadMarker,
        text: &str,
        x: f64,
        y: f64,
        w: f64,
        size: f64,
    ) -> Retained<NSTextField> {
        let label = NSTextField::labelWithString(&NSString::from_str(text), main);
        label.setFont(Some(&NSFont::systemFontOfSize(size)));
        label.setTextColor(Some(&crate::native_settings::color(if size < 12. {
            self.palette.get().muted
        } else {
            self.palette.get().ink
        })));
        label.setFrame(rect(x, y, w, if size >= 20. { 36. } else { 22. }));
        label.setAccessibilityLabel(Some(&NSString::from_str(text)));
        self.document.addSubview(&label);
        self.fields.borrow_mut().push(label.clone());
        self.content
            .borrow_mut()
            .push(Retained::into_super(Retained::into_super(label.clone())));
        label
    }
    fn paragraph(&self, main: MainThreadMarker, text: &str, y: &mut f64, width: f64, size: f64) {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), main);
        label.setFont(Some(&NSFont::systemFontOfSize(size)));
        label.setTextColor(Some(&crate::native_settings::color(
            self.palette.get().muted,
        )));
        label.setAccessibilityLabel(Some(&NSString::from_str(text)));
        let height = ((text.chars().count() as f64 * size * 0.57 / width)
            .ceil()
            .max(1.)
            * (size + 7.))
            .max(24.);
        label.setFrame(rect(26., *y, width, height));
        self.document.addSubview(&label);
        self.fields.borrow_mut().push(label.clone());
        self.content
            .borrow_mut()
            .push(Retained::into_super(Retained::into_super(label)));
        *y += height + 10.;
    }
    fn chart(
        &self,
        owner: &Controller,
        points: &[Point],
        title: &str,
        y: &mut f64,
        width: f64,
        height: f64,
    ) {
        let main = owner.mtm();
        self.text(main, title, 26., *y, width - 52., 16.);
        *y += 30.;
        let max = points
            .iter()
            .filter_map(|p| p.uses)
            .max()
            .unwrap_or(0)
            .max(1);
        self.text(main, &max.to_string(), 26., *y, 40., 11.);
        let plot_width = width - 88.;
        let step = plot_width / points.len().max(1) as f64;
        let stride = (points.len() / 5).max(1);
        for (i, p) in points.iter().enumerate() {
            let bar: Retained<Bar> = unsafe {
                let b = Bar::alloc(main).set_ivars(BarState {
                    uses: p.uses,
                    max,
                    accent: self.palette.get().accent,
                });
                msg_send![super(b),initWithFrame:rect(64.+i as f64*step,*y,step*0.72,height)]
            };
            bar.setAccessibilityElement(true);
            bar.setAccessibilityRole(Some(unsafe { NSAccessibilityValueIndicatorRole }));
            bar.setAccessibilityLabel(Some(&NSString::from_str(&p.accessibility)));
            let value = p
                .uses
                .map(|n| format!("{n} uses"))
                .unwrap_or_else(|| "Not recorded".into());
            unsafe {
                bar.setAccessibilityValue(Some(&NSString::from_str(&value)));
            }
            self.document.addSubview(&bar);
            self.bars.borrow_mut().push(bar.clone());
            self.content.borrow_mut().push(Retained::into_super(bar));
            if i % stride == 0 {
                self.text(
                    main,
                    &p.label,
                    58. + i as f64 * step,
                    *y + height + 5.,
                    (step * stride as f64).max(50.),
                    11.,
                );
            }
        }
        self.text(main, "0", 26., *y + height - 14., 30., 11.);
        *y += height + 38.;
    }
    pub fn apply(&self, owner: &Controller, surface: Option<Arc<Surface>>) {
        let bounds = unsafe { self.scroll.superview() }
            .map(|v| v.bounds())
            .unwrap_or(self.scroll.frame());
        let changed_size = self.width.replace(bounds.size.width) != bounds.size.width
            || self.scroll.frame().size.height != bounds.size.height - 58.;
        self.scroll
            .setFrame(rect(0., 0., bounds.size.width, bounds.size.height - 58.));
        self.scroll.setHidden(surface.is_none());
        let Some(s) = surface else {
            self.retire_dialog();
            self.restore_focus.borrow_mut().take();
            self.current.borrow_mut().take();
            let old = self.controls.borrow_mut().take();
            if let Some(c) = old {
                c.cleanup();
            }
            return;
        };
        let graphite = owner.management_graphite();
        self.document.ivars().set(graphite);
        let colors = crate::native_settings::palette(&self.document, graphite);
        let colors_changed = self.palette.replace(colors) != colors;
        self.document.setNeedsDisplay(true);
        let old = self.current.borrow().clone();
        if old.as_ref().is_some_and(|v| v.token != s.token) {
            self.retire_dialog();
            self.restore_focus.borrow_mut().take();
        }
        if !changed_size && !colors_changed && old.as_ref().is_some_and(|v| **v == *s) {
            return;
        }
        let controls = self.controls(owner, &s);
        if s.busy && old.as_ref().is_some_and(|v| !v.busy) {
            *self.restore_focus.borrow_mut() = self
                .scroll
                .window()
                .and_then(|w| w.firstResponder())
                .and_then(|r| r.downcast::<NSView>().ok());
        }

        let old_content = self.content.take();
        self.fields.borrow_mut().clear();
        self.bars.borrow_mut().clear();
        for v in old_content {
            v.removeFromSuperview();
        }
        let main = owner.mtm();
        let width = self.width.get();
        let content_width = width - 52.;
        let scroll = self.scroll.contentView().bounds().origin;
        let mut y = 24.;
        self.text(main, "Dashboard", 26., y, width - 190., 23.);
        controls.buttons[0].setFrame(rect(width - 132., y, 106., 28.));
        y += 55.;
        let card_width = (content_width - 24.) / 3.;
        for (i, (label, value)) in [
            ("Uses", s.uses),
            ("Successful", s.successes),
            ("Failed", s.failures),
        ]
        .iter()
        .enumerate()
        {
            let x = 26. + i as f64 * (card_width + 12.);
            self.text(main, &value.to_string(), x, y, card_width, 27.);
            self.text(main, label, x, y + 35., card_width, 12.);
        }
        y += 82.;
        self.chart(
            owner,
            &s.hours,
            &format!("Last 24 hours · {} uses", s.hourly_uses),
            &mut y,
            width,
            150.,
        );
        let mut info = "Per hour · current hour included".to_string();
        if let Some(date) = &s.recording_since {
            info.push_str(&format!(" · Recording since {date}"));
        }
        self.text(main, &info, 26., y, content_width, 11.);
        y += 42.;
        if !s.days.is_empty() {
            self.chart(
                owner,
                &s.days,
                "Recorded daily activity",
                &mut y,
                width,
                110.,
            );
            y += 16.;
        }
        self.text(main, "Commands", 26., y, content_width, 16.);
        y += 30.;
        let name_width = content_width - 240.;
        for (text, x, w) in [
            ("Name", 26., name_width),
            ("Uses", width - 248., 65.),
            ("Failed", width - 176., 65.),
            ("p95", width - 102., 76.),
        ] {
            self.text(main, text, x, y, w, 11.);
        }
        y += 26.;
        if s.commands.is_empty() {
            self.text(
                main,
                "Run a command to see its activity.",
                26.,
                y,
                content_width,
                12.,
            );
            y += 28.;
        }
        for row in &s.commands {
            self.text(main, &row.name, 26., y, name_width, 12.);
            self.text(main, &row.uses.to_string(), width - 248., y, 65., 12.);
            self.text(main, &row.failures.to_string(), width - 176., y, 65., 12.);
            self.text(main, &row.p95, width - 102., y, 76., 12.);
            y += 28.;
        }
        if s.omitted_commands > 0 {
            self.text(
                main,
                &format!("Additional commands: {} uses", s.omitted_commands),
                26.,
                y,
                content_width,
                12.,
            );
            y += 28.;
        }
        y += 20.;
        self.text(main, "Recent timings", 26., y, content_width, 16.);
        y += 30.;
        if s.timings.is_empty() {
            self.text(main, "No timings recorded", 26., y, content_width, 12.);
            y += 28.;
        }
        for (name, value) in &s.timings {
            self.text(main, name, 26., y, name_width, 12.);
            self.text(main, &format!("{value} p95"), width - 180., y, 154., 12.);
            y += 28.;
        }
        self.text(
            main,
            "p95: latest 32 samples per command, 256 per measurement.",
            26.,
            y,
            content_width,
            11.,
        );
        y += 48.;
        self.text(main, "Recent errors", 26., y, content_width, 16.);
        y += 30.;
        if s.errors.is_empty() {
            self.text(main, "No errors recorded", 26., y, content_width, 12.);
            y += 28.;
        }
        for row in &s.errors {
            self.text(main, &row.name, 26., y, content_width - 190., 12.);
            self.text(main, &row.date, width - 210., y, 184., 11.);
            y += 22.;
            self.text(main, &row.reason, 26., y, content_width, 12.);
            y += 34.;
        }
        y += 22.;
        controls.buttons[1].setFrame(rect(26., y, 220., 28.));
        y += 44.;
        self.text(
            main,
            "Recent activity and errors",
            26.,
            y,
            content_width - 170.,
            12.,
        );
        controls
            .retention
            .setFrame(rect(width - 166., y - 3., 140., 28.));
        y += 44.;
        controls.buttons[2].setFrame(rect(26., y, 180., 28.));
        controls.buttons[3].setFrame(rect(width - 192., y, 166., 28.));
        y += 44.;
        for i in [4, 5] {
            controls.buttons[i].setHidden(!s.confirming_reset);
        }
        if s.confirming_reset {
            self.text(main, "Reset Analytics?", 26., y, content_width, 16.);
            y += 30.;
            self.text(
                main,
                "Deletes analytics only. Commands and clipboard history are kept.",
                26.,
                y,
                content_width,
                12.,
            );
            y += 32.;
            controls.buttons[4].setFrame(rect(26., y, 160., 28.));
            controls.buttons[5].setFrame(rect(200., y, 100., 28.));
            y += 44.;
        }
        if s.prior_successes > 0 {
            self.text(main,&format!("Includes {} earlier successful uses. Earlier failures and timings are unknown.",s.prior_successes),26.,y,content_width,11.);
            y += 28.;
        }
        self.text(
            main,
            "Stored on this Mac. No input or clipboard contents are logged.",
            26.,
            y,
            content_width,
            11.,
        );
        y += 28.;
        if let Some(error) = &s.storage_error {
            self.paragraph(main, error, &mut y, content_width, 12.);
        }
        if !s.message.is_empty() {
            self.paragraph(main, &s.message, &mut y, content_width, 12.);
        }
        self.document.setFrame(rect(0., 0., width, y + 26.));
        self.scroll.contentView().scrollToPoint(NSPoint::new(
            scroll.x,
            scroll
                .y
                .min((y + 26. - self.scroll.contentView().bounds().size.height).max(0.)),
        ));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
        for (i, b) in controls.buttons.iter().enumerate() {
            b.setEnabled((!s.busy && (!s.confirming_reset || i == 4 || i == 5)) || i == 5);
        }
        unsafe {
            controls.buttons[1].setAccessibilityValue(Some(
                &objc2_foundation::NSNumber::numberWithBool(s.enabled),
            ));
        }
        controls.buttons[1].setState(if s.enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        if let Some(binding) = controls.buttons[1].ivars().binding.borrow_mut().as_mut() {
            binding.operation = Some(Operation::Enabled {
                expected: s.enabled,
                value: !s.enabled,
            });
        }
        let mut values = vec![7, 30, 90];
        if !values.contains(&s.retention_days) {
            values.insert(0, s.retention_days);
        }
        let old_values: Vec<_> = controls
            .retention
            .itemArray()
            .iter()
            .map(|i| i.tag() as i64)
            .collect();
        if old_values != values {
            controls.retention.removeAllItems();
            for value in &values {
                controls
                    .retention
                    .addItemWithTitle(&NSString::from_str(&format!("{value} days")));
                if let Some(item) = controls.retention.lastItem() {
                    item.setTag(*value as isize);
                }
            }
        }
        controls.retention.selectItemAtIndex(
            values
                .iter()
                .position(|n| *n == s.retention_days)
                .unwrap_or(0) as isize,
        );
        controls.retention.ivars().expected.set(s.retention_days);
        controls
            .retention
            .setEnabled(!s.busy && !s.confirming_reset);
        let mut order: Vec<&NSView> = vec![
            &controls.buttons[0],
            &controls.buttons[1],
            &controls.retention,
            &controls.buttons[2],
            &controls.buttons[3],
        ];
        if s.confirming_reset {
            order = vec![&controls.buttons[4], &controls.buttons[5]];
        }
        for (index, view) in order.iter().enumerate() {
            unsafe {
                view.setNextKeyView(Some(order[(index + 1) % order.len()]));
            }
        }

        if !s.busy && old.as_ref().is_some_and(|v| v.busy && v.token == s.token) {
            let restore = self.restore_focus.borrow_mut().take();
            if let Some(view) = restore {
                if let Some(w) = self.scroll.window() {
                    w.makeFirstResponder(Some(&*view));
                }
            }
        }
        if s.confirming_reset
            && old
                .as_ref()
                .is_none_or(|v| v.confirmation_token != s.confirmation_token)
        {
            if let Some(w) = self.scroll.window() {
                w.makeFirstResponder(Some(&controls.buttons[5]));
            }
        }
        *self.current.borrow_mut() = Some(s);
    }
    pub fn text_values(&self) -> Vec<String> {
        let fields = self.fields.borrow().clone();
        let controls = self.controls.borrow().clone();
        let mut values: Vec<_> = fields.iter().map(|f| f.stringValue().to_string()).collect();
        if let Some(c) = controls {
            values.extend(
                c.buttons
                    .iter()
                    .filter(|b| !b.isHidden())
                    .map(|b| b.title().to_string()),
            );
            values.extend(
                c.retention
                    .itemArray()
                    .iter()
                    .map(|i| i.title().to_string()),
            );
        }
        values
    }
    pub fn accessibility_values(&self) -> Vec<String> {
        let fields = self.fields.borrow().clone();
        let bars = self.bars.borrow().clone();
        let controls = self.controls.borrow().clone();
        let mut values: Vec<_> = fields
            .iter()
            .filter_map(|f| f.accessibilityLabel().map(|s| s.to_string()))
            .collect();
        for b in bars {
            if let Some(s) = NSAccessibility::accessibilityLabel(&**b) {
                values.push(s.to_string());
            }
            if let Some(s) = b
                .accessibilityValue()
                .and_then(|s| s.downcast::<NSString>().ok())
            {
                values.push(s.to_string());
            }
        }
        if let Some(c) = controls {
            for b in &c.buttons {
                if !b.isHidden() {
                    if let Some(s) = NSAccessibility::accessibilityLabel(&***b) {
                        values.push(s.to_string());
                    }
                }
            }
            if let Some(s) = NSAccessibility::accessibilityLabel(&**c.retention) {
                values.push(s.to_string());
            }
        }
        values
    }
    pub fn token(&self) -> Option<u64> {
        self.current.borrow().as_ref().map(|s| s.token)
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        let controls = self.controls.borrow().clone();
        let current = self.current.borrow().clone();
        let fields = self.fields.borrow().clone();
        serde_json::json!({"surface":current.as_deref(),"visible":!self.scroll.isHidden(),"scrollY":self.scroll.contentView().bounds().origin.y,"documentHeight":self.document.frame().size.height,"width":self.width.get(),"focused":self.focused(),"dialogOpen":self.dialog_open(),"fixtureDialog":self.fixture_dialog.get(),"dialogSerial":self.dialog_token.get(),"buttons":controls.as_ref().map(|c|c.buttons.iter().map(|b|serde_json::json!({"title":b.title().to_string(),"enabled":b.isEnabled(),"hidden":b.isHidden(),"role":NSAccessibility::accessibilityRole(&***b).map(|r|r.to_string()),"axValue":NSAccessibility::accessibilityValue(&***b).map(|v|{let s:Retained<NSString>=unsafe{msg_send![&*v,description]};s.to_string()}),"value":b.state()})).collect::<Vec<_>>()),"fields":fields.iter().map(|f|serde_json::json!({"text":f.stringValue().to_string(),"x":f.frame().origin.x,"y":f.frame().origin.y,"width":f.frame().size.width,"height":f.frame().size.height})).collect::<Vec<_>>(),"bars":self.bars.borrow().iter().map(|b|serde_json::json!({"role":NSAccessibility::accessibilityRole(&***b).map(|r|r.to_string()),"label":NSAccessibility::accessibilityLabel(&***b).map(|r|r.to_string()),"value":NSAccessibility::accessibilityValue(&***b).map(|r|{let s:Retained<NSString>=unsafe{msg_send![&*r,description]};s.to_string()})})).collect::<Vec<_>>(),"text":self.text_values(),"accessibility":self.accessibility_values()})
    }
    #[cfg(feature = "test-tools")]
    pub fn focus(&self, index: usize) -> bool {
        let b = self
            .controls
            .borrow()
            .as_ref()
            .and_then(|c| c.buttons.get(index))
            .cloned();
        if let Some(b) = b {
            self.scroll
                .window()
                .is_some_and(|w| w.makeFirstResponder(Some(&*b)))
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn focused(&self) -> Option<String> {
        self.scroll
            .window()
            .and_then(|w| w.firstResponder())
            .and_then(|r| r.downcast::<NSButton>().ok())
            .map(|b| b.title().to_string())
    }
    pub fn advance_focus(&self, reverse: bool) {
        if let Some(w) = self.scroll.window() {
            let current = w.firstResponder().and_then(|r| r.downcast::<NSView>().ok());
            let next = current.and_then(|v| unsafe {
                if reverse {
                    v.previousKeyView()
                } else {
                    v.nextKeyView()
                }
            });
            if let Some(next) = next {
                w.makeFirstResponder(Some(&*next));
            } else {
                let first = self
                    .controls
                    .borrow()
                    .as_ref()
                    .and_then(|c| c.buttons.get(if self.confirming() { 5 } else { 0 }))
                    .cloned();
                if let Some(first) = first {
                    w.makeFirstResponder(Some(&*first));
                }
            }
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> bool {
        if self.dialog_open() {
            return false;
        }
        let button = self
            .controls
            .borrow()
            .as_ref()
            .and_then(|c| c.buttons.get(index))
            .cloned();
        if let Some(b) = button.filter(|b| b.isEnabled() && !b.isHidden()) {
            unsafe {
                b.performClick(None);
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retention(&self, value: i64) -> bool {
        if self.dialog_open() {
            return false;
        }
        let control = self.controls.borrow().as_ref().map(|c| c.retention.clone());
        if let Some(p) = control.filter(|p| p.isEnabled()) {
            if !p.selectItemWithTag(value as isize) {
                return false;
            }
            unsafe {
                let _: () = msg_send![&*p,invoke:Option::<&objc2_foundation::NSObject>::None];
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_control(&self, index: usize) {
        let b = self
            .controls
            .borrow()
            .as_ref()
            .and_then(|c| c.buttons.get(index))
            .cloned();
        *self.stale.borrow_mut() = b;
    }
    #[cfg(feature = "test-tools")]
    pub fn click_stale(&self) -> bool {
        let b = self.stale.borrow().clone();
        if let Some(b) = b {
            unsafe {
                let _: () = msg_send![&*b,invoke:Option::<&objc2_foundation::NSObject>::None];
            }
            true
        } else {
            false
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn complete_fixture(
        &self,
        owner: &Controller,
        serial: u64,
        token: u64,
        accepted: bool,
        path: Option<std::path::PathBuf>,
    ) -> bool {
        if !self.fixture_dialog.get() || !self.finish_dialog(serial) {
            return false;
        }
        if accepted {
            owner.dashboard_control(
                token,
                path.map(Operation::Export)
                    .unwrap_or(Operation::ExportPathUnavailable),
            );
        }
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn scroll_to(&self, y: f64) {
        self.scroll.contentView().scrollToPoint(NSPoint::new(0., y));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
    }
    #[cfg(feature = "test-tools")]
    pub fn snapshot(&self, path: &std::path::Path) -> Result<(), String> {
        self.document.layoutSubtreeIfNeeded();
        let bitmap = self
            .document
            .bitmapImageRepForCachingDisplayInRect(self.document.bounds())
            .ok_or("Dashboard render allocation failed")?;
        self.document
            .cacheDisplayInRect_toBitmapImageRep(self.document.bounds(), &bitmap);
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .ok_or("Dashboard PNG unavailable")?;
        std::fs::write(path, unsafe { data.as_bytes_unchecked() }).map_err(|e| e.to_string())
    }
    pub fn confirmation_token(&self) -> Option<u64> {
        self.current
            .borrow()
            .as_ref()
            .and_then(|s| s.confirmation_token)
    }
    pub fn confirming(&self) -> bool {
        self.current
            .borrow()
            .as_ref()
            .is_some_and(|s| s.confirming_reset)
    }
    pub fn dialog_open(&self) -> bool {
        self.dialog_token.get() != 0
    }
    pub fn retire_dialog(&self) {
        self.dialog_token.set(0);
        self.fixture_dialog.set(false);
        let old = self.dialog.borrow_mut().take();
        if let Some(panel) = old {
            unsafe {
                panel.cancel(None);
            }
        }
    }
    pub fn cleanup(&self) {
        self.retire_dialog();
        let old = self.controls.borrow_mut().take();
        if let Some(c) = old {
            c.cleanup();
        }
        self.current.borrow_mut().take();
    }
    pub fn begin_export(&self, owner: &Controller, token: u64, window: &NSWindow, hidden: bool) {
        if !owner.dashboard_live(token)
            || self.token() != Some(token)
            || self.dialog_open()
            || self.current.borrow().as_ref().is_some_and(|s| s.busy)
        {
            return;
        }
        let Some(serial) = self.dialog_serial.get().checked_add(1) else {
            return;
        };
        self.dialog_serial.set(serial);
        self.dialog_token.set(serial);
        if hidden {
            self.fixture_dialog.set(true);
            return;
        }
        // No NSSavePanel construction at all in hidden fixture mode: native save
        // panels may display remote/XPC UI even when their parent is offscreen.
        if !owner.dashboard_live(token) {
            self.retire_dialog();
            return;
        }
        let panel = NSSavePanel::savePanel(owner.mtm());
        panel.setNameFieldStringValue(&NSString::from_str("ruycast-diagnostics.json"));
        if let Some(json) = objc2_uniform_type_identifiers::UTType::typeWithIdentifier(
            &NSString::from_str("public.json"),
        ) {
            panel.setAllowedContentTypes(&NSArray::from_slice(&[&*json]));
        }
        if !owner.dashboard_live(token)
            || self.dialog_token.get() != serial
            || self.token() != Some(token)
        {
            self.retire_dialog();
            return;
        }
        *self.dialog.borrow_mut() = Some(panel.clone());
        let weak = Weak::new(owner);
        let receive = panel.clone();
        let block = block2::RcBlock::new(move |response: NSModalResponse| {
            if let Some(owner) = weak.load() {
                owner.dashboard_export_completed(
                    serial,
                    token,
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
        self.fixture_dialog.set(false);
        let old = self.dialog.borrow_mut().take();
        drop(old);
        true
    }
}
