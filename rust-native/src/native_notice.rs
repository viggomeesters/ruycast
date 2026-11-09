//! Bounded, nonactivating completion notice. No application activation or
//! keyboard responder changes. Hidden fixtures only render its own content.
use crate::main_loop::Timer;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::connected::PassiveNotice;

define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=()]
    struct NoticeContent;
    impl NoticeContent {
        #[unsafe(method(drawRect:))]
        fn draw(&self,_dirty:NSRect){
            NSColor::windowBackgroundColor().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(),12.,12.).fill();
        }
    }
);
pub struct Service {
    main: MainThreadMarker,
    seen: Option<u64>,
    panel: Option<Retained<NSPanel>>,
    label: Option<Retained<NSTextField>>,
    timer: Option<Timer>,
    success: bool,
}
impl Service {
    pub fn new(main: MainThreadMarker) -> Self {
        Self {
            main,
            seen: None,
            panel: None,
            label: None,
            timer: None,
            success: false,
        }
    }
    pub fn show(
        &mut self,
        notice: &PassiveNotice,
        hidden: bool,
        appearance: &NSAppearance,
        live: impl Fn() -> bool,
    ) -> Result<(), String> {
        if !live() || self.seen == Some(notice.serial) {
            return Ok(());
        }
        self.timer.take();
        if let Some(panel) = self.panel.take() {
            panel.orderOut(None);
        }
        let bounds = NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(380., 88.));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(self.main),
            bounds,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        panel.setTitle(ns_string!("Ruycast"));
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setLevel(NSStatusWindowLevel);
        panel.setIgnoresMouseEvents(true);
        panel.setHasShadow(true);
        panel.setAppearance(Some(appearance));
        let content = unsafe {
            let content = NoticeContent::alloc(self.main).set_ivars(());
            msg_send![super(content),initWithFrame:NSRect::new(NSPoint::new(0.,0.),bounds.size)]
        };
        let content: Retained<NoticeContent> = content;
        let icon = NSTextField::labelWithString(
            if notice.success {
                ns_string!("✓")
            } else {
                ns_string!("!")
            },
            self.main,
        );
        icon.setFrame(NSRect::new(NSPoint::new(16., 32.), NSSize::new(20., 24.)));
        icon.setFont(Some(&NSFont::systemFontOfSize(18.)));
        let color = if notice.success {
            NSColor::systemGreenColor()
        } else {
            NSColor::systemOrangeColor()
        };
        icon.setTextColor(Some(&color));
        content.addSubview(&icon);
        let label =
            NSTextField::wrappingLabelWithString(&NSString::from_str(&notice.text), self.main);
        label.setFrame(NSRect::new(NSPoint::new(46., 14.), NSSize::new(318., 60.)));
        label.setFont(Some(&NSFont::systemFontOfSize(13.)));
        label.setTextColor(Some(&NSColor::labelColor()));
        label.setMaximumNumberOfLines(3);
        let text_height = label
            .sizeThatFits(NSSize::new(318., 60.))
            .height
            .clamp(18., 60.);
        let height = (text_height + 28.).max(65.);
        panel.setContentSize(NSSize::new(380., height));
        content.setFrameSize(NSSize::new(380., height));
        icon.setFrameOrigin(NSPoint::new(16., (height - 24.) / 2.));
        label.setFrame(NSRect::new(
            NSPoint::new(46., (height - text_height) / 2.),
            NSSize::new(318., text_height),
        ));
        content.addSubview(&label);
        panel.setContentView(Some(&content));
        let later = panel.clone();
        let timer = Timer::new_default(self.main, 5., move || later.orderOut(None))?;
        if !live() {
            return Ok(());
        }
        if !hidden {
            let mouse = NSEvent::mouseLocation();
            let screens = NSScreen::screens(self.main);
            let screen = screens
                .iter()
                .find(|screen| {
                    let f = screen.frame();
                    mouse.x >= f.origin.x
                        && mouse.x < f.origin.x + f.size.width
                        && mouse.y >= f.origin.y
                        && mouse.y < f.origin.y + f.size.height
                })
                .or_else(|| screens.firstObject());
            if let Some(screen) = screen {
                let f = screen.visibleFrame();
                panel.setFrameOrigin(NSPoint::new(
                    f.origin.x + f.size.width - 400.,
                    f.origin.y + f.size.height - height - 20.,
                ));
            }
            panel.orderFrontRegardless();
        } else {
            panel.orderOut(None);
        }
        self.success = notice.success;
        self.seen = Some(notice.serial);
        self.label = Some(label);
        self.panel = Some(panel);
        self.timer = Some(timer);
        Ok(())
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self, path: Option<&std::path::Path>) -> Result<serde_json::Value, String> {
        let Some(panel) = &self.panel else {
            return Ok(serde_json::json!({"present":false}));
        };
        if let Some(path) = path {
            let view = panel.contentView().ok_or("Notice content unavailable")?;
            view.layoutSubtreeIfNeeded();
            let bitmap = view
                .bitmapImageRepForCachingDisplayInRect(view.bounds())
                .ok_or("Notice render allocation failed")?;
            view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
            let data = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &objc2_foundation::NSDictionary::new(),
                )
            }
            .ok_or("Notice PNG encoding failed")?;
            std::fs::write(path, unsafe { data.as_bytes_unchecked() })
                .map_err(|e| e.to_string())?;
        }
        Ok(
            serde_json::json!({"present":true,"success":self.success,"text":self.label.as_ref().map(|l|l.stringValue().to_string()),"serial":self.seen,"windowShown":panel.isVisible(),"appearance":panel.effectiveAppearance().name().to_string(),"canBecomeKey":panel.canBecomeKeyWindow(),"nonactivating":panel.styleMask().contains(NSWindowStyleMask::NonactivatingPanel),"ignoresMouse":panel.ignoresMouseEvents(),"height":panel.contentView().map(|v|v.bounds().size.height),"level":panel.level(),"timer":self.timer.as_ref().is_some_and(Timer::valid)}),
        )
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.timer.take();
        if let Some(panel) = &self.panel {
            panel.orderOut(None);
        }
    }
}
