//! Native status item. Hidden fixtures substitute registration before NSStatusBar.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};
struct Binding {
    owner: Weak<Controller>,
    serial: u64,
}
define_class!(
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[ivars=Binding]
    struct Target;
    impl Target{
        #[unsafe(method(invoke:))]fn invoke(&self,sender:&NSMenuItem){if let Some(owner)=self.ivars().owner.load(){owner.menu_bar_action(self.ivars().serial,sender.tag());}}
    }
);
pub struct Service {
    item: RefCell<Option<Retained<NSStatusItem>>>,
    menu: RefCell<Option<Retained<NSMenu>>>,
    target: RefCell<Option<Retained<Target>>>,
    serial: Cell<u64>,
    state: RefCell<Option<ruycast_native::menu_bar::State>>,
    hidden: Cell<bool>,
    registrations: Cell<u64>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<NSMenuItem>>>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            item: RefCell::new(None),
            menu: RefCell::new(None),
            target: RefCell::new(None),
            serial: Cell::new(1),
            state: RefCell::new(None),
            hidden: Cell::new(true),
            registrations: Cell::new(0),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
        }
    }
}
impl Service {
    pub fn apply(
        &self,
        owner: &Controller,
        state: Option<ruycast_native::menu_bar::State>,
        hidden: bool,
    ) {
        let mut state = state;
        if !hidden {
            if let Some(s) = &mut state {
                if s.requires_input {
                    s.healthy &=
                        unsafe { AXIsProcessTrusted() != 0 && CGPreflightListenEventAccess() }
                            && NSRunningApplication::runningApplicationsWithBundleIdentifier(
                                &NSString::from_str("com.raycast.macos"),
                            )
                            .is_empty();
                }
            }
        }
        if *self.state.borrow() == state {
            return;
        }
        self.hidden.set(hidden);
        let visible = state.as_ref().is_some_and(|s| s.visible);
        if !visible {
            self.retire();
            *self.state.borrow_mut() = state;
            return;
        }
        if self.menu.borrow().is_none() {
            let main = owner.mtm();
            let target = Target::alloc(main).set_ivars(Binding {
                owner: Weak::new(owner),
                serial: self.serial.get(),
            });
            let target: Retained<Target> = unsafe { msg_send![super(target), init] };
            let menu = NSMenu::new(main);
            for (index, title) in ["Open Launcher", "Settings…", "Quit"].iter().enumerate() {
                let row = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(main),
                        &NSString::from_str(title),
                        Some(sel!(invoke:)),
                        &NSString::from_str(""),
                    )
                };
                row.setTag(index as isize);
                unsafe {
                    row.setTarget(Some(&*target));
                }
                menu.addItem(&row);
            }
            *self.menu.borrow_mut() = Some(menu.clone());
            *self.target.borrow_mut() = Some(target);
            if !hidden {
                // Never call systemStatusBar/statusItemWithLength in fixtures.
                let item =
                    NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
                item.setMenu(Some(&menu));
                *self.item.borrow_mut() = Some(item);
                self.registrations.set(self.registrations.get() + 1);
            }
        }
        if let Some(item) = &*self.item.borrow() {
            if let Some(button) = item.button(owner.mtm()) {
                let healthy = state.as_ref().is_some_and(|s| s.healthy);
                button.setImage(Some(&icon(healthy)));
                button.setToolTip(Some(&NSString::from_str(if healthy {
                    "Ruycast · Running"
                } else {
                    "Ruycast · Needs attention"
                })));
                button.setAccessibilityLabel(Some(&NSString::from_str(if healthy {
                    "Ruycast, running"
                } else {
                    "Ruycast, needs attention"
                })));
            }
        }
        *self.state.borrow_mut() = state;
    }
    pub fn refresh(&self, owner: &Controller) {
        let state = owner.menu_bar_state();
        self.apply(owner, state, self.hidden.get());
    }
    pub fn live(&self, serial: u64) -> bool {
        serial != 0
            && self.serial.get() == serial
            && self.state.borrow().as_ref().is_some_and(|s| s.visible)
    }
    pub fn retire(&self) {
        self.serial
            .set(self.serial.get().checked_add(1).unwrap_or(0));
        if let Some(menu) = self.menu.take() {
            for row in menu.itemArray().iter() {
                unsafe {
                    row.setTarget(None);
                    row.setAction(None);
                }
            }
        }
        self.target.take();
        let item = self.item.take();
        if let Some(item) = item {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
        }
    }
    pub fn cleanup(&self) {
        self.state.borrow_mut().take();
        self.retire();
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        serde_json::json!({"state":*self.state.borrow(),"serial":self.serial.get(),"hiddenSubstitution":self.hidden.get(),"nativeRegistrations":self.registrations.get(),"items":self.menu.borrow().as_ref().map(|m|m.itemArray().iter().map(|r|r.title().to_string()).collect::<Vec<_>>())})
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> bool {
        let row = self
            .menu
            .borrow()
            .as_ref()
            .and_then(|m| m.itemAtIndex(index as isize));
        if let Some(row) = row {
            if let Some(target) = self.target.borrow().clone() {
                unsafe {
                    let _: () = msg_send![&*target,invoke:&*row];
                }
                return true;
            }
        }
        false
    }
    #[cfg(feature = "test-tools")]
    pub fn retain(&self) {
        *self.stale.borrow_mut() = self.menu.borrow().as_ref().and_then(|m| m.itemAtIndex(0));
    }
    #[cfg(feature = "test-tools")]
    pub fn stale(&self) {
        if let Some(row) = &*self.stale.borrow() {
            if let Some(target) = row.target() {
                unsafe {
                    let _: () = msg_send![&*target,invoke:&**row];
                }
            }
        }
    }
}
fn icon(healthy: bool) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
        let color = if healthy {
            NSColor::blackColor()
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.78, 0.37, 0.10, 1.)
        };
        color.setFill();
        let p = NSBezierPath::bezierPath();
        let point = |x: f64, y: f64| NSPoint::new(1. + x * 0.16, 1. + y * 0.16);
        p.moveToPoint(point(17., 15.));
        for (x, y) in [
            (17., 85.),
            (58., 85.),
            (83., 61.),
            (59., 38.),
            (34., 38.),
            (34., 15.),
        ] {
            p.lineToPoint(point(x, y));
        }
        p.closePath();
        p.moveToPoint(point(34., 69.));
        for (x, y) in [(53., 69.), (62., 61.), (53., 53.), (34., 53.)] {
            p.lineToPoint(point(x, y));
        }
        p.closePath();
        p.setWindingRule(NSWindingRule::EvenOdd);
        p.fill();
        let ray = NSBezierPath::bezierPath();
        ray.moveToPoint(point(39., 43.));
        for (x, y) in [(59., 43.), (85., 15.), (62., 15.)] {
            ray.lineToPoint(point(x, y));
        }
        ray.closePath();
        ray.fill();
        true.into()
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(18., 18.), false, &draw);
    image.setTemplate(healthy);
    image.setAccessibilityDescription(Some(&NSString::from_str(if healthy {
        "Ruycast"
    } else {
        "Ruycast needs attention"
    })));
    image
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGPreflightListenEventAccess() -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_health_images_preserve_template_state_and_accessible_description() {
        objc2::rc::autoreleasepool(|_| {
            let healthy = icon(true);
            let unhealthy = icon(false);
            assert!(healthy.isTemplate());
            assert!(!unhealthy.isTemplate());
            assert_eq!(healthy.accessibilityDescription().unwrap().to_string(), "Ruycast");
            assert_eq!(unhealthy.accessibilityDescription().unwrap().to_string(), "Ruycast needs attention");
        });
    }
}
