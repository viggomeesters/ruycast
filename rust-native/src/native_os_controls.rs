//! Process sort control. Callbacks are bound to the visible query/dismissal.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAccessibility, NSPopUpButton, NSView};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::processes::Sort;
use std::cell::{Cell, RefCell};
struct Binding {
    owner: Weak<Controller>,
    epoch: u64,
    dismissal: u64,
}
define_class!(#[unsafe(super=NSPopUpButton)]#[thread_kind=MainThreadOnly]#[ivars=RefCell<Option<Binding>>]struct Popup;
    impl Popup{#[unsafe(method(invoke:))]fn invoke(&self,_:Option<&NSObject>){let binding=self.ivars().borrow().as_ref().map(|b|(b.owner.clone(),b.epoch,b.dismissal));if let Some((owner,epoch,dismissal))=binding{if let Some(owner)=owner.load(){let sort=match self.indexOfSelectedItem(){1=>Sort::Memory,2=>Sort::Name,_=>Sort::Cpu};owner.process_sort_bound(sort,epoch,dismissal);}}}}
);
pub struct Controls {
    popup: Retained<Popup>,
    selected: Cell<Sort>,
}
impl Controls {
    pub fn new(main: MainThreadMarker, parent: &NSView) -> Self {
        let popup = Popup::alloc(main).set_ivars(RefCell::new(None));
        let popup: Retained<Popup> = unsafe {
            msg_send![super(popup),initWithFrame:NSRect::new(NSPoint::new(0.,0.),NSSize::new(145.,26.)),pullsDown:false]
        };
        for title in ["CPU Usage", "Memory", "Name"] {
            popup.addItemWithTitle(&NSString::from_str(title));
        }
        popup.setAccessibilityLabel(Some(&NSString::from_str("Sort processes")));
        unsafe {
            popup.setTarget(Some(&*popup));
            popup.setAction(Some(sel!(invoke:)));
        }
        popup.setHidden(true);
        parent.addSubview(&popup);
        Self {
            popup,
            selected: Cell::new(Sort::Cpu),
        }
    }
    pub fn apply(
        &self,
        owner: &Controller,
        state: Option<(Sort, u64, u64)>,
        width: f64,
        height: f64,
    ) {
        self.popup.setHidden(state.is_none());
        self.popup.setFrame(NSRect::new(
            NSPoint::new(width - 175., height - 109.),
            NSSize::new(148., 26.),
        ));
        if let Some((sort, epoch, dismissal)) = state {
            *self.popup.ivars().borrow_mut() = Some(Binding {
                owner: Weak::new(owner),
                epoch,
                dismissal,
            });
            if self.selected.replace(sort) != sort {
                self.popup.selectItemAtIndex(match sort {
                    Sort::Cpu => 0,
                    Sort::Memory => 1,
                    Sort::Name => 2,
                });
            }
        } else {
            *self.popup.ivars().borrow_mut() = None;
        }
    }
    pub fn cleanup(&self) {
        *self.popup.ivars().borrow_mut() = None;
        unsafe {
            self.popup.setTarget(None);
            self.popup.setAction(None);
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn choose(&self, index: isize) {
        self.popup.selectItemAtIndex(index);
        unsafe {
            let _: () = msg_send![&*self.popup,invoke:Option::<&NSObject>::None];
        }
    }
}
