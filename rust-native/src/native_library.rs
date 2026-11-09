//! Main-thread Settings navigation. All durable state and filtering live on the worker.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::{define_class, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
pub struct Toolbar {
    pub tabs: Vec<Retained<NSButton>>,
    pub filter: Retained<NSPopUpButton>,
    pub add: Retained<NSPopUpButton>,
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
pub const TABS: [&str; 7] = ["dashboard", "general", "commands", "snippets", "quicklinks", "clipboard", "advanced"];
pub const FILTERS: [&str; 6] = ["all", "app", "window", "script", "builtin", "disabled"];
impl Toolbar {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let mut tabs = vec![];
        for (i, title) in ["Dashboard", "General", "Commands", "Snippets", "Quicklinks", "Clipboard", "Advanced"].iter().enumerate() {
            let button = NSButton::new(main);
            button.setButtonType(NSButtonType::PushOnPushOff);
            button.setTitle(&NSString::from_str(title));
            button.setAccessibilityLabel(Some(&NSString::from_str(title)));
            button.setTag(i as isize);
            button.setFrame(rect(12. + i as f64 * ((width-24.)/7.), height - 48., (width-24.)/7.-4., 30.));
            parent.addSubview(&button);
            tabs.push(button);
        }
        let filter = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(main),
            rect(width - 360., height - 88., 190., 30.),
            false,
        );
        for title in [
            "All Commands",
            "Applications",
            "Window Management",
            "Scripts",
            "Ruycast",
            "Disabled",
        ] {
            filter.addItemWithTitle(&NSString::from_str(title));
        }
        filter.setAccessibilityLabel(Some(&NSString::from_str("Filter commands")));
        parent.addSubview(&filter);
        let add = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(main),
            rect(width - 146., height - 88., 122., 30.),
            true,
        );
        for title in ["Add", "Snippet", "Quicklink", "Shell Command"] {
            add.addItemWithTitle(&NSString::from_str(title));
        }
        add.setAccessibilityLabel(Some(&NSString::from_str("Add command")));
        parent.addSubview(&add);
        Self { tabs, filter, add }
    }
    pub fn layout(&self,width:f64,height:f64){
        for (i,b) in self.tabs.iter().enumerate(){b.setFrame(rect(12.+i as f64*((width-24.)/7.),height-48.,(width-24.)/7.-4.,30.));}
        self.filter.setFrame(rect(width-360.,height-88.,190.,30.));
        self.add.setFrame(rect(width-146.,height-88.,122.,30.));
    }
    pub fn bind(&self, owner: &Controller) {
        for button in &self.tabs {
            unsafe {
                button.setTarget(Some(owner));
                button.setAction(Some(sel!(selectLibraryTab:)));
            }
        }
        unsafe {
            self.filter.setTarget(Some(owner));
            self.filter.setAction(Some(sel!(filterLibrary:)));
            self.add.setTarget(Some(owner));
            self.add.setAction(Some(sel!(addLibraryCommand:)));
        }
    }
    pub fn cleanup(&self) {
        for b in &self.tabs {
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
        }
        unsafe {
            self.filter.setTarget(None);
            self.filter.setAction(None);
            self.add.setTarget(None);
            self.add.setAction(None);
        }
    }
    pub fn apply(&self, info: Option<&ruycast_native::library::Info>, editing: bool) {
        for (i, button) in self.tabs.iter().enumerate() {
            button.setState(if info.is_some_and(|n| n.tab == TABS[i]) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(info) = info {
            self.filter.selectItemAtIndex(
                FILTERS.iter().position(|f| *f == info.filter).unwrap_or(0) as isize,
            );
            self.filter.setHidden(info.tab != "commands" || editing);
        }
        self.add.setHidden(editing||info.is_some_and(|n|!["commands","snippets","quicklinks"].contains(&n.tab.as_str())));
        self.add.setEnabled(!editing);
    }
    pub fn accessibility_text(&self) -> Vec<String> {
        let mut labels = self
            .tabs
            .iter()
            .filter_map(|b| NSAccessibility::accessibilityLabel(&**b))
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        if !self.filter.isHidden() {
            if let Some(label) = NSAccessibility::accessibilityLabel(&*self.filter) {
                labels.push(label.to_string());
            }
        }
        if let Some(label) = NSAccessibility::accessibilityLabel(&*self.add) {
            labels.push(label.to_string());
        }
        labels
    }
    pub fn text(&self) -> Vec<String> {
        let mut text = self
            .tabs
            .iter()
            .map(|b| b.title().to_string())
            .collect::<Vec<_>>();
        if !self.filter.isHidden() {
            if let Some(title) = self.filter.titleOfSelectedItem() {
                text.push(title.to_string());
            }
        }
        text.push("Add".into());
        text
    }
}

pub struct EnabledState {
    owner: Weak<Controller>,
    target: ruycast_native::actions::Target,
    dismissal: u64,
    enabled: bool,
}
define_class!(
    #[unsafe(super=NSButton)]
    #[thread_kind=MainThreadOnly]
    #[ivars=EnabledState]
    pub(crate) struct EnabledButton;
    impl EnabledButton {
        #[unsafe(method(changeEnabled:))]
        fn changed(&self, _: Option<&objc2_foundation::NSObject>) {
            let state=self.ivars();
            let enabled=self.state()==NSControlStateValueOn;
            // Keep the published state until the owner accepts the private write.
            self.setState(if state.enabled {NSControlStateValueOn}else{NSControlStateValueOff});
            if let Some(owner)=state.owner.load() { owner.library_enabled(state.target.clone(),state.dismissal,state.enabled,enabled); }
        }
    }
);
impl EnabledButton {
    pub fn new(
        owner: &Controller,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        enabled: bool,
        title: &str,
        width: f64,
    ) -> Retained<Self> {
        let button = Self::alloc(owner.mtm()).set_ivars(EnabledState {
            owner: Weak::new(owner),
            target,
            dismissal,
            enabled,
        });
        let button: Retained<Self> =
            unsafe { objc2::msg_send![super(button),initWithFrame:rect(0.,0.,width,30.)] };
        button.setButtonType(NSButtonType::Switch);
        button.setTitle(&NSString::from_str(""));
        button.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        button.setAccessibilityLabel(Some(&NSString::from_str(&format!("Enable {title}"))));
        unsafe {
            button.setTarget(Some(&*button));
            button.setAction(Some(sel!(changeEnabled:)));
        }
        button
    }
}

pub struct BindingState {
    owner: Weak<Controller>,
    target: ruycast_native::actions::Target,
    dismissal: u64,
    mode: ruycast_native::actions::Mode,
}
define_class!(
    #[unsafe(super=NSButton)]
    #[thread_kind=MainThreadOnly]
    #[ivars=BindingState]
    pub(crate) struct BindingButton;
    impl BindingButton {
        #[unsafe(method(editBinding:))]
        fn edit(&self,_:Option<&objc2_foundation::NSObject>){if let Some(owner)=self.ivars().owner.load(){owner.library_inline(self.ivars().target.clone(),self.ivars().dismissal,self.ivars().mode);}}
    }
);
impl BindingButton {
    pub fn new(
        owner: &Controller,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        mode: ruycast_native::actions::Mode,
        value: &str,
        title: &str,
        width: f64,
    ) -> Retained<Self> {
        let button = Self::alloc(owner.mtm()).set_ivars(BindingState {
            owner: Weak::new(owner),
            target,
            dismissal,
            mode,
        });
        let button: Retained<Self> =
            unsafe { objc2::msg_send![super(button),initWithFrame:rect(0.,0.,width,30.)] };
        button.setTitle(&NSString::from_str(value));
        button.setFont(Some(&NSFont::systemFontOfSize(12.)));
        button.setAccessibilityLabel(Some(&NSString::from_str(&format!(
            "{} for {}",
            if mode == ruycast_native::actions::Mode::Alias {
                "Aliases"
            } else {
                "Shortcut"
            },
            title
        ))));
        unsafe {
            button.setTarget(Some(&*button));
            button.setAction(Some(sel!(editBinding:)));
        }
        button
    }
}
