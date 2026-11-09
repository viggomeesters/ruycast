//! Independent native Monitor window. AppKit objects remain on the main thread.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    NSDateFormatter, NSDateFormatterStyle, NSISO8601DateFormatter, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use ruycast_native::monitor::{Operation, Surface, Tab};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
#[derive(Clone, Copy)]
struct Theme {
    background: [f64; 3],
    ink: [f64; 3],
    muted: [f64; 3],
    accent: [f64; 3],
}
impl Theme {
    fn new(dark: bool, graphite: bool) -> Self {
        Self {
            background: if graphite {
                [0.15; 3]
            } else if dark {
                [0.10, 0.12, 0.17]
            } else {
                [0.98, 0.97, 0.94]
            },
            ink: if dark {
                [0.96, 0.95, 0.91]
            } else {
                [0.18, 0.20, 0.25]
            },
            muted: if dark {
                [0.68, 0.71, 0.77]
            } else {
                [0.42, 0.44, 0.48]
            },
            accent: if dark {
                [1., 0.66, 0.35]
            } else {
                [0.64, 0.30, 0.08]
            },
        }
    }
}
fn color(rgb: [f64; 3]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(rgb[0], rgb[1], rgb[2], 1.)
}
define_class!(#[unsafe(super=NSView)]#[thread_kind=MainThreadOnly]#[ivars=Cell<Theme>]struct Background;
    impl Background{#[unsafe(method(isFlipped))]fn flipped(&self)->bool{true}#[unsafe(method(drawRect:))]fn draw(&self,_:NSRect){color(self.ivars().get().background).setFill();NSRectFill(self.bounds());}}
);
struct GraphData {
    points: Vec<(f64, f64)>,
    accent: [f64; 3],
    disk: Option<f64>,
}
define_class!(#[unsafe(super=NSView)]#[thread_kind=MainThreadOnly]#[ivars=RefCell<GraphData>]struct Graph;
    impl Graph{#[unsafe(method(drawRect:))]fn draw(&self,_:NSRect){let data=self.ivars().borrow();let bounds=self.bounds();let path=NSBezierPath::bezierPath();color(data.accent).setStroke();path.setLineWidth(2.);
        if let Some(value)=data.disk{color(data.accent).setFill();NSRectFill(rect(0.,bounds.size.height/2.-3.,bounds.size.width*value.clamp(0.,1.),6.));return;}
        let Some(first)=data.points.first()else{return;};let min_x=first.0;let max_x=data.points.last().map(|p|p.0).unwrap_or(min_x);let max_y=data.points.iter().map(|p|p.1).fold(1f64,f64::max);
        for (index,(time,value)) in data.points.iter().enumerate(){let x=if max_x>min_x{(time-min_x)/(max_x-min_x)*(bounds.size.width-4.)+2.}else{2.};let y=2.+value.max(0.)/max_y*(bounds.size.height-4.);if index==0{path.moveToPoint(NSPoint::new(x,y));}else{path.lineToPoint(NSPoint::new(x,y));}}path.stroke();}}
);
struct WindowState {
    allowed: bool,
    blocked: Cell<u64>,
}
define_class!(#[unsafe(super=NSWindow)]#[thread_kind=MainThreadOnly]#[ivars=WindowState]struct Window;
    impl Window{
        #[unsafe(method(makeKeyAndOrderFront:))]fn key_front(&self,sender:Option<&NSObject>){if !self.ivars().allowed{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),makeKeyAndOrderFront:sender];}}
        #[unsafe(method(orderFront:))]fn front(&self,sender:Option<&NSObject>){if !self.ivars().allowed{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderFront:sender];}}
        #[unsafe(method(orderBack:))]fn back(&self,sender:Option<&NSObject>){if !self.ivars().allowed{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderBack:sender];}}
        #[unsafe(method(orderWindow:relativeTo:))]fn order(&self,place:NSWindowOrderingMode,relative:isize){if !self.ivars().allowed&&place!=NSWindowOrderingMode::Out{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderWindow:place,relativeTo:relative];}}
        #[unsafe(method(orderFrontRegardless))]fn regardless(&self){if !self.ivars().allowed{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderFrontRegardless];}}
        #[unsafe(method(makeKeyWindow))]fn key(&self){if !self.ivars().allowed{self.ivars().blocked.set(self.ivars().blocked.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),makeKeyWindow];}}
    }
);
struct Binding {
    owner: Weak<Controller>,
    token: u64,
    operation: Operation,
    selected: bool,
    theme: Theme,
}
define_class!(#[unsafe(super=NSButton)]#[thread_kind=MainThreadOnly]#[ivars=RefCell<Option<Binding>>]struct Button;
    impl Button{#[unsafe(method(invoke:))]fn invoke(&self,_:Option<&NSObject>){let binding=self.ivars().borrow().as_ref().map(|b|(b.owner.clone(),b.token,b.operation));if let Some((owner,token,operation))=binding{if let Some(owner)=owner.load(){owner.monitor_control(token,operation);}}}
        #[unsafe(method(drawRect:))]fn draw(&self,dirty:NSRect){if let Some(binding)=self.ivars().borrow().as_ref().filter(|b|b.selected){let c=binding.theme.ink;NSColor::colorWithSRGBRed_green_blue_alpha(c[0],c[1],c[2],0.08).setFill();NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(),8.,8.).fill();}
            unsafe{let _:()=msg_send![super(self),drawRect:dirty];}}
    }
);
struct DelegateState {
    owner: Weak<Controller>,
    token: Cell<u64>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    #[ivars=DelegateState]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn closing(&self, _: &NSNotification) {
            if let Some(owner) = self.ivars().owner.load() {
                owner.monitor_control(self.ivars().token.get(), Operation::Close);
            }
        }
        #[unsafe(method(windowDidMiniaturize:))]
        fn minimized(&self, _: &NSNotification) {
            if let Some(owner) = self.ivars().owner.load() {
                owner.monitor_control(self.ivars().token.get(), Operation::Pause);
            }
        }
        #[unsafe(method(windowDidDeminiaturize:))]
        fn restored(&self, _: &NSNotification) {
            if let Some(owner) = self.ivars().owner.load() {
                owner.monitor_control(self.ivars().token.get(), Operation::Resume);
            }
        }
        #[unsafe(method(windowDidResize:))]
        fn resized(&self, _: &NSNotification) {
            if let Some(owner) = self.ivars().owner.load() {
                owner.monitor_layout();
            }
        }
    }
);
pub struct Monitor {
    main: MainThreadMarker,
    window: Retained<Window>,
    root: Retained<Background>,
    document: Retained<Background>,
    scroll: Retained<NSScrollView>,
    delegate: Retained<Delegate>,
    heading: Retained<NSTextField>,
    time: Retained<NSTextField>,
    footer: Retained<NSTextField>,
    buttons: Vec<Retained<Button>>,
    badges: Vec<Retained<NSTextField>>,
    labels: RefCell<Vec<Retained<NSTextField>>>,
    graph: Retained<Graph>,
    surface: RefCell<Option<Arc<Surface>>>,
    serial: Cell<u64>,
    graphite: Cell<bool>,
    tab: Cell<Tab>,
    hidden: bool,
}
fn label(main: MainThreadMarker, text: &str, size: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), main);
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setSelectable(true);
    field.setAccessibilityLabel(Some(&NSString::from_str(text)));
    field
}
fn background(main: MainThreadMarker, frame: NSRect, theme: Theme) -> Retained<Background> {
    let view = Background::alloc(main).set_ivars(Cell::new(theme));
    unsafe { msg_send![super(view),initWithFrame:frame] }
}
impl Monitor {
    pub fn new(owner: &Controller, main: MainThreadMarker, hidden: bool) -> Self {
        let theme = Theme::new(false, false);
        let window = Window::alloc(main).set_ivars(WindowState {
            allowed: !hidden,
            blocked: Cell::new(0),
        });
        let window: Retained<Window> = unsafe {
            msg_send![super(window),initWithContentRect:rect(0.,0.,900.,640.),styleMask:NSWindowStyleMask::Titled|NSWindowStyleMask::Closable|NSWindowStyleMask::Miniaturizable|NSWindowStyleMask::Resizable,backing:NSBackingStoreType::Buffered,defer:false]
        };
        window.setTitle(&NSString::from_str("System Monitor"));
        window.setMinSize(NSSize::new(740., 500.));
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        let root = background(main, rect(0., 0., 900., 640.), theme);
        window.setContentView(Some(&root));
        let delegate = Delegate::alloc(main).set_ivars(DelegateState {
            owner: Weak::new(owner),
            token: Cell::new(0),
        });
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let heading = label(main, "System Monitor", 17.);
        root.addSubview(&heading);
        let time = label(main, "", 11.);
        root.addSubview(&time);
        let footer = label(main, "Ruycast", 11.);
        root.addSubview(&footer);
        let scroll = NSScrollView::new(main);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        let document = background(main, rect(0., 0., 600., 700.), theme);
        scroll.setDocumentView(Some(&document));
        root.addSubview(&scroll);
        let mut buttons = vec![];
        for (title, operation) in Tab::ALL
            .into_iter()
            .map(|tab| (tab.title(), Operation::Tab(tab)))
            .chain(std::iter::once((
                "Open Activity Monitor",
                Operation::ActivityMonitor,
            )))
        {
            let button = Button::alloc(main).set_ivars(RefCell::new(None));
            let button: Retained<Button> =
                unsafe { msg_send![super(button),initWithFrame:rect(0.,0.,200.,30.)] };
            button.setTitle(&NSString::from_str(title));
            button.setBezelStyle(NSBezelStyle::Push);
            button.setAccessibilityLabel(Some(&NSString::from_str(title)));
            if let Operation::Tab(tab) = operation {
                let icon = match tab {
                    Tab::System => "info.circle",
                    Tab::Cpu => "cpu",
                    Tab::Memory => "memorychip",
                    Tab::Disk => "internaldrive",
                    Tab::Power => "battery.100",
                    Tab::Network => "network",
                };
                button.setImage(
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str(icon),
                        None,
                    )
                    .as_deref(),
                );
                button.setImagePosition(NSCellImagePosition::ImageLeft);
            }
            unsafe {
                button.setTarget(Some(&*button));
                button.setAction(Some(sel!(invoke:)));
            }
            *button.ivars().borrow_mut() = Some(Binding {
                owner: Weak::new(owner),
                token: 0,
                operation,
                selected: false,
                theme,
            });
            root.addSubview(&button);
            buttons.push(button);
        }
        let badges = (0..6)
            .map(|_| {
                let field = label(main, "", 11.);
                field.setSelectable(false);
                field.setAlignment(NSTextAlignment::Right);
                root.addSubview(&field);
                field
            })
            .collect();
        let graph = Graph::alloc(main).set_ivars(RefCell::new(GraphData {
            points: vec![],
            accent: theme.accent,
            disk: None,
        }));
        let graph: Retained<Graph> =
            unsafe { msg_send![super(graph),initWithFrame:rect(0.,0.,500.,150.)] };
        graph.setAccessibilityElement(true);
        document.addSubview(&graph);
        graph.setHidden(true);
        Self {
            main,
            window,
            root,
            document,
            scroll,
            delegate,
            heading,
            time,
            footer,
            buttons,
            badges,
            labels: Default::default(),
            graph,
            surface: Default::default(),
            serial: Cell::new(0),
            graphite: Cell::new(false),
            tab: Cell::new(Tab::System),
            hidden,
        }
    }
    pub fn apply(&self, owner: &Controller, surface: Arc<Surface>, theme: Option<&str>) {
        self.graphite.set(theme == Some("graphite"));
        let appearance = match theme {
            Some("dark" | "graphite") => {
                NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameDarkAqua"))
            }
            Some("light") => {
                NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameAqua"))
            }
            _ => None,
        };
        self.window.setAppearance(appearance.as_deref());
        self.delegate.ivars().token.set(surface.token);
        for button in &self.buttons {
            if let Some(binding) = button.ivars().borrow_mut().as_mut() {
                binding.owner = Weak::new(owner);
                binding.token = surface.token;
            }
        }
        let shown = surface.shown;
        let serial = surface.show_serial;
        *self.surface.borrow_mut() = Some(surface);
        self.layout();
        if !shown {
            self.window.orderOut(None);
        } else if self.serial.replace(serial) != serial && !self.hidden {
            if !self.window.isVisible() {
                self.window.center();
            }
            self.window.deminiaturize(None);
            self.window.makeKeyAndOrderFront(None);
            NSApplication::sharedApplication(self.main).activate();
        }
    }
    pub fn layout(&self) {
        let Some(surface) = self.surface.borrow().clone() else {
            return;
        };
        let bounds = self.root.bounds();
        let (width, height) = (bounds.size.width, bounds.size.height);
        let dark = self
            .window
            .effectiveAppearance()
            .name()
            .to_string()
            .to_lowercase()
            .contains("dark");
        let theme = Theme::new(dark, self.graphite.get());
        self.root.ivars().set(theme);
        self.document.ivars().set(theme);
        self.root.setNeedsDisplay(true);
        self.document.setNeedsDisplay(true);
        self.heading.setFrame(rect(20., 17., 300., 28.));
        self.heading.setTextColor(Some(&color(theme.ink)));
        self.time.setFrame(rect(width - 300., 23., 276., 18.));
        self.time.setAlignment(NSTextAlignment::Right);
        self.time.setTextColor(Some(&color(theme.muted)));
        let time = surface
            .snapshot
            .as_ref()
            .and_then(|s| {
                NSISO8601DateFormatter::new().dateFromString(&NSString::from_str(&s.sampled_at))
            })
            .map(|date| {
                let formatter = NSDateFormatter::new();
                formatter.setDateStyle(NSDateFormatterStyle::NoStyle);
                formatter.setTimeStyle(NSDateFormatterStyle::MediumStyle);
                formatter.stringFromDate(&date).to_string()
            })
            .unwrap_or_default();
        self.time.setStringValue(&NSString::from_str(&time));
        self.footer.setFrame(rect(20., height - 35., 120., 18.));
        self.footer.setTextColor(Some(&color(theme.muted)));
        for (index, button) in self.buttons.iter().enumerate() {
            button.setFrame(if index == 6 {
                rect(width - 215., height - 44., 195., 30.)
            } else {
                rect(14., 81. + index as f64 * 44., 212., 35.)
            });
            if index < 6 {
                button.setBordered(false);
                button.setAlignment(NSTextAlignment::Left);
                button.setFont(Some(&NSFont::systemFontOfSize(13.)));
                button.setContentTintColor(Some(&color(theme.ink)));
                if let Some(binding) = button.ivars().borrow_mut().as_mut() {
                    binding.selected = Tab::ALL[index] == surface.tab;
                    binding.theme = theme;
                }
                NSView::setNeedsDisplay(button, true);
                button.setState(if Tab::ALL[index] == surface.tab {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                let value = surface
                    .snapshot
                    .as_ref()
                    .map(|s| match Tab::ALL[index] {
                        Tab::Cpu => ruycast_native::monitor::percent(s.cpu_percent),
                        Tab::Memory => {
                            if s.memory_total > 0 {
                                format!(
                                    "{}%",
                                    (s.memory_used as f64 * 100. / s.memory_total as f64) as u64
                                )
                            } else {
                                "—".into()
                            }
                        }
                        Tab::Disk => {
                            if s.disk_total > 0 {
                                format!(
                                    "{}%",
                                    (s.disk_total.saturating_sub(s.disk_available) as f64 * 100.
                                        / s.disk_total as f64)
                                        as u64
                                )
                            } else {
                                "—".into()
                            }
                        }
                        Tab::Power => s
                            .battery_percent
                            .map(|n| format!("{n}%"))
                            .unwrap_or_else(|| "—".into()),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                let badge = &self.badges[index];
                badge.setStringValue(&NSString::from_str(&value));
                badge.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "{} {}",
                    Tab::ALL[index].title(),
                    value
                ))));
                badge.setTextColor(Some(&color(theme.muted)));
                badge.setFrame(rect(173., 90. + index as f64 * 44., 43., 18.));
                badge.setHidden(value.is_empty());
            }
        }
        self.scroll
            .setFrame(rect(244., 70., width - 260., height - 132.));
        let document_width = self.scroll.contentView().bounds().size.width;
        let keep_scroll = self.tab.replace(surface.tab) == surface.tab;
        let scroll_y = if keep_scroll {
            self.scroll.contentView().bounds().origin.y
        } else {
            0.
        };
        let mut fields = vec![(surface.tab.title().to_string(), 23., false)];
        let mut metric = None;
        let mut rows = vec![];
        if !surface.error.is_empty() {
            fields.push((surface.error.clone(), 13., false));
        } else if let Some(snapshot) = &surface.snapshot {
            metric = ruycast_native::monitor::metric(snapshot, surface.tab);
            rows = ruycast_native::monitor::rows(snapshot, surface.tab);
        } else {
            fields.push(("Loading…".into(), 13., true));
        }
        if let Some((value, name)) = &metric {
            fields.push((value.clone(), 32., false));
            fields.push((name.to_string(), 11., true));
        }
        let mut labels = self.labels.borrow_mut();
        while labels.len() < fields.len() + rows.len() * 2 {
            let field = label(self.main, "", 13.);
            self.document.addSubview(&field);
            labels.push(field);
        }
        for field in labels.iter() {
            field.setHidden(true);
        }
        let mut y = 14.;
        let mut index = 0;
        for (text, size, muted) in fields {
            let field = &labels[index];
            field.setHidden(false);
            field.setStringValue(&NSString::from_str(&text));
            field.setAccessibilityLabel(Some(&NSString::from_str(&text)));
            field.setFont(Some(&NSFont::systemFontOfSize(size)));
            field.setAlignment(NSTextAlignment::Left);
            field.setTextColor(Some(&color(if muted { theme.muted } else { theme.ink })));
            field.setFrame(rect(
                10.,
                y,
                document_width - 20.,
                if size >= 23. { 44. } else { 24. },
            ));
            y += if size >= 23. { 48. } else { 28. };
            index += 1;
        }
        let graph_visible = surface.error.is_empty()
            && surface.snapshot.is_some()
            && matches!(
                surface.tab,
                Tab::Cpu | Tab::Memory | Tab::Disk | Tab::Network
            );
        self.graph.setHidden(!graph_visible);
        if graph_visible {
            let points = surface
                .history
                .iter()
                .filter_map(|s| {
                    ruycast_native::monitor::graph(s, surface.tab).map(|v| (s.timestamp, v))
                })
                .collect::<Vec<_>>();
            let disk = surface
                .snapshot
                .as_ref()
                .filter(|_| surface.tab == Tab::Disk)
                .map(|s| {
                    s.disk_total.saturating_sub(s.disk_available) as f64
                        / s.disk_total.max(1) as f64
                });
            *self.graph.ivars().borrow_mut() = GraphData {
                points,
                accent: theme.accent,
                disk,
            };
            self.graph.setFrame(rect(
                10.,
                y,
                document_width - 20.,
                if surface.tab == Tab::Disk { 22. } else { 150. },
            ));
            self.graph
                .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "Recent {} usage",
                    surface.tab.title()
                ))));
            self.graph.setNeedsDisplay(true);
            y += if surface.tab == Tab::Disk { 42. } else { 172. };
        }
        for (key, value) in rows {
            let value_width = (document_width - 182.).max(100.);
            let row_height = (value.chars().count() as f64 / (value_width / 7.).max(1.))
                .ceil()
                .max(1.)
                * 19.
                + 18.;
            for (text, x, w, muted) in [(key, 10., 150., true), (value, 172., value_width, false)] {
                let field = &labels[index];
                field.setHidden(false);
                field.setStringValue(&NSString::from_str(&text));
                field.setAccessibilityLabel(Some(&NSString::from_str(&text)));
                field.setTextColor(Some(&color(if muted { theme.muted } else { theme.ink })));
                field.setFont(Some(&NSFont::systemFontOfSize(13.)));
                field.setUsesSingleLineMode(false);
                field.setMaximumNumberOfLines(0);
                field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                if let Some(cell) = field.cell() {
                    cell.setWraps(true);
                    cell.setScrollable(false);
                }
                field.setFrame(rect(x, y, w, row_height));
                field.setAlignment(if muted {
                    NSTextAlignment::Left
                } else {
                    NSTextAlignment::Right
                });
                index += 1;
            }
            y += row_height + 4.;
        }
        self.document.setFrame(rect(
            0.,
            0.,
            document_width,
            y.max(self.scroll.contentView().bounds().size.height),
        ));
        self.scroll.contentView().scrollToPoint(NSPoint::new(
            0.,
            scroll_y.min((y - self.scroll.contentView().bounds().size.height).max(0.)),
        ));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
    }
    pub fn cleanup(&self) {
        for button in &self.buttons {
            *button.ivars().borrow_mut() = None;
            unsafe {
                button.setTarget(None);
                button.setAction(None);
            }
        }
        self.window.setDelegate(None);
        self.window.orderOut(None);
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> bool {
        let Some(button) = self.buttons.get(index) else {
            return false;
        };
        unsafe {
            let _: () = msg_send![&**button,invoke:Option::<&NSObject>::None];
        }
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn lifecycle(&self, event: &str) -> bool {
        let notification = unsafe {
            NSNotification::notificationWithName_object(
                &NSString::from_str("Owned Monitor Lifecycle"),
                None,
            )
        };
        match event {
            "pause" => unsafe {
                let _: () = msg_send![&*self.delegate,windowDidMiniaturize:&*notification];
            },
            "resume" => unsafe {
                let _: () = msg_send![&*self.delegate,windowDidDeminiaturize:&*notification];
            },
            "close" => unsafe {
                let _: () = msg_send![&*self.delegate,windowWillClose:&*notification];
            },
            _ => return false,
        }
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn resize(&self, width: f64, height: f64) {
        self.window.setContentSize(NSSize::new(width, height));
        self.layout();
    }
    #[cfg(feature = "test-tools")]
    pub fn render_state(
        &self,
        state: &str,
        path: &std::path::Path,
    ) -> Result<serde_json::Value, String> {
        let original = self.surface.borrow().clone().ok_or("Monitor unavailable")?;
        let mut owned = (*original).clone();
        match state {
            "loading" => {
                owned.snapshot = None;
                owned.history.clear();
                owned.error.clear();
            }
            "error" => owned.error = "Could not sample system resources. Try again.".into(),
            "long" => {
                owned.tab = Tab::System;
                if let Some(s) = owned.snapshot.as_mut() {
                    s.hostname = "owned-host-with-a-long-readable-name.invalid".repeat(3);
                    s.model = "Owned model ".repeat(15);
                }
            }
            "no-battery" => {
                owned.tab = Tab::Power;
                if let Some(s) = owned.snapshot.as_mut() {
                    s.battery_percent = None;
                    s.charging = None;
                    s.power_source = None;
                }
            }
            _ => return Err("Unknown owned render state".into()),
        }
        *self.surface.borrow_mut() = Some(Arc::new(owned));
        self.layout();
        let result = self.snapshot(path).map(|_| self.probe());
        *self.surface.borrow_mut() = Some(original);
        self.layout();
        result
    }
    #[cfg(feature = "test-tools")]
    pub fn snapshot(&self, path: &std::path::Path) -> Result<(), String> {
        self.root.layoutSubtreeIfNeeded();
        let bitmap = self
            .root
            .bitmapImageRepForCachingDisplayInRect(self.root.bounds())
            .ok_or("Monitor render allocation failed")?;
        self.root
            .cacheDisplayInRect_toBitmapImageRep(self.root.bounds(), &bitmap);
        let bytes = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .ok_or("Monitor PNG encoding failed")?;
        std::fs::write(path, unsafe { bytes.as_bytes_unchecked() }).map_err(|e| e.to_string())
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        let mut text = self
            .labels
            .borrow()
            .iter()
            .filter(|f| !f.isHidden())
            .map(|f| f.stringValue().to_string())
            .collect::<Vec<_>>();
        text.extend(
            [&self.heading, &self.time, &self.footer]
                .into_iter()
                .map(|f| f.stringValue().to_string()),
        );
        text.extend(self.buttons.iter().map(|b| b.title().to_string()));
        text.extend(
            self.badges
                .iter()
                .filter(|f| !f.isHidden())
                .map(|f| f.stringValue().to_string()),
        );
        let mut ax = self
            .labels
            .borrow()
            .iter()
            .filter(|f| !f.isHidden())
            .filter_map(|f| f.accessibilityLabel().map(|s| s.to_string()))
            .collect::<Vec<_>>();
        ax.extend(
            self.buttons
                .iter()
                .filter_map(|b| NSAccessibility::accessibilityLabel(&***b).map(|s| s.to_string())),
        );
        ax.extend(
            self.badges
                .iter()
                .filter(|f| !f.isHidden())
                .filter_map(|f| f.accessibilityLabel().map(|s| s.to_string())),
        );
        serde_json::json!({"windowShown":self.window.isVisible(),"windowKey":self.window.isKeyWindow(),"windowMain":self.window.isMainWindow(),"orderingAllowed":self.window.ivars().allowed,"blockedOrders":self.window.ivars().blocked.get(),"width":self.root.bounds().size.width,"height":self.root.bounds().size.height,"appearance":self.window.effectiveAppearance().name().to_string(),"tab":self.surface.borrow().as_ref().map(|s|s.tab.title()),"token":self.surface.borrow().as_ref().map(|s|s.token),"shown":self.surface.borrow().as_ref().map(|s|s.shown),"active":self.surface.borrow().as_ref().map(|s|s.active),"history":self.surface.borrow().as_ref().map(|s|s.history.len()),"screenText":text,"accessibilityText":ax,"graphAccessibility":(!self.graph.isHidden()).then(||self.graph.accessibilityLabel().map(|s|s.to_string()))})
    }
}
