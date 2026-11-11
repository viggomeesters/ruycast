//! Low-level owned AX reads only: no interposes, global constructor or ready token.
#![allow(
    dead_code,
    reason = "Shared state and ABI are reused by the later separately accepted System SDK components"
)]
use objc2::{rc::Retained, runtime::AnyObject, Message};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSSize, NSString};
use std::{ffi::c_void, sync::Mutex};
pub(super) type Ptr = *mut c_void;
pub(super) const UNSUPPORTED: i32 = -25205;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> Ptr;
    pub(super) fn AXUIElementGetTypeID() -> usize;
    fn AXValueCreate(kind: u32, value: *const c_void) -> Ptr;
    pub(super) fn AXValueGetTypeID() -> usize;
    pub(super) fn AXValueGetType(value: Ptr) -> u32;
    pub(super) fn AXValueGetValue(value: Ptr, kind: u32, out: Ptr) -> u8;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub(super) fn CFGetTypeID(value: Ptr) -> usize;
    pub(super) fn CFEqual(left: Ptr, right: Ptr) -> u8;
    fn CFRetain(value: Ptr) -> Ptr;
    fn CFRelease(value: Ptr);
    pub(super) fn CFArrayGetTypeID() -> usize;
    pub(super) fn CFArrayGetCount(value: Ptr) -> isize;
    pub(super) fn CFArrayGetValueAtIndex(value: Ptr, index: isize) -> Ptr;
    pub(super) fn CFBooleanGetTypeID() -> usize;
    pub(super) fn CFBooleanGetValue(value: Ptr) -> u8;
}
pub(super) struct CF(pub Ptr);
// Retained opaque AX handles and immutable CF values, never AppKit UI objects.
unsafe impl Send for CF {}
unsafe impl Sync for CF {}
impl Drop for CF {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
impl CF {
    unsafe fn retained(value: Ptr) -> Self {
        Self(CFRetain(value))
    }
    fn object<T: Message>(value: Retained<T>) -> Self {
        Self(Retained::into_raw(value).cast())
    }
    fn into_raw(self) -> Ptr {
        let pointer = self.0;
        std::mem::forget(self);
        pointer
    }
}
pub(super) struct Data {
    pub mode: String,
    pub selected: i32,
    pub effects: i32,
    pub reads: i32,
    pub consents: i32,
    pub placement: NSPoint,
    pub dimensions: NSSize,
}
pub(super) struct State {
    nodes: [CF; 8],
    pub data: Mutex<Data>,
}
impl State {
    pub fn new() -> Self {
        // These allocate local opaque handles for nonexistent fixture PIDs.
        // No AX message, app lookup or permission request is sent.
        let nodes = std::array::from_fn(|index| {
            let pointer = unsafe { AXUIElementCreateApplication(730002 + index as i32) };
            assert!(!pointer.is_null());
            CF(pointer)
        });
        Self {
            nodes,
            data: Mutex::new(Data {
                mode: String::new(),
                selected: 1,
                effects: 0,
                reads: 0,
                consents: 0,
                placement: NSPoint::new(40., 50.),
                dimensions: NSSize::new(600., 400.),
            }),
        }
    }
    pub fn node(&self, index: usize) -> Ptr {
        self.nodes[index - 1].0
    }
    pub fn set_mode(&self, mode: &str) {
        let mut data = self.data.lock().unwrap();
        data.mode = mode.into();
        data.selected = 1;
        data.effects = 0;
    }
    fn index(&self, node: Ptr) -> usize {
        assert!(!node.is_null());
        self.nodes
            .iter()
            .position(|candidate| unsafe { CFEqual(node, candidate.0) != 0 })
            .map(|i| i + 1)
            .expect("Only owned AX handles may be read")
    }
    fn array(&self, indices: &[usize]) -> CF {
        let values = indices
            .iter()
            .map(|index| unsafe {
                Retained::retain(self.node(*index).cast::<AnyObject>()).unwrap()
            })
            .collect::<Vec<_>>();
        CF::object(NSArray::from_retained_slice(&values))
    }
    unsafe fn value(&self, node: Ptr, key: &NSString) -> Option<CF> {
        let index = self.index(node);
        let mode = {
            let mut data = self.data.lock().unwrap();
            data.reads += 1;
            data.mode.clone()
        };
        let title = [
            "",
            "Owned App",
            "Owned Window",
            "Owned Minimized",
            "",
            "File",
            "",
            "Open Owned…",
            "Disabled Owned",
        ][index];
        let title = if index == 2 && mode == "renamed" {
            "Changed"
        } else {
            title
        };
        let role = [
            "",
            "AXApplication",
            "AXWindow",
            "AXWindow",
            "AXMenuBar",
            "AXMenuBarItem",
            "AXMenu",
            "AXMenuItem",
            "AXMenuItem",
        ][index];
        let is = |name: &str| key.isEqualToString(&NSString::from_str(name));
        if is("AXRole") {
            return Some(CF::object(NSString::from_str(role)));
        }
        if is("AXTitle") {
            return Some(CF::object(NSString::from_str(title)));
        }
        if is("AXEnabled") {
            return Some(CF::object(NSNumber::numberWithBool(index != 8)));
        }
        if is("AXMinimized") && (index == 2 || index == 3) {
            return Some(CF::object(NSNumber::numberWithBool(
                index == 3 && mode != "restored",
            )));
        }
        if is("AXWindows") && index == 1 {
            return Some(self.array(&[2, 3]));
        }
        if is("AXMenuBar") && index == 1 {
            return Some(CF::retained(self.node(4)));
        }
        if is("AXFocusedWindow") && index == 1 {
            return Some(CF::retained(self.node(2)));
        }
        if is("AXChildren") {
            return Some(self.array(match index {
                4 => &[5],
                5 => &[6],
                6 => &[7, 8],
                _ => &[],
            }));
        }
        if is("AXMenuItemCmdChar") && index == 7 {
            return Some(CF::object(NSString::from_str("o")));
        }
        if is("AXMenuItemCmdModifiers") && index == 7 {
            return Some(CF::object(NSNumber::numberWithInt(0)));
        }
        None
    }
    pub fn trust(&self) -> u8 {
        u8::from(self.data.lock().unwrap().mode != "denied")
    }
    pub unsafe fn application(&self, pid: i32) -> Ptr {
        assert_eq!(pid, 730001);
        CFRetain(self.node(1))
    }
    pub unsafe fn pid(&self, node: Ptr, out: *mut i32) -> i32 {
        assert!(!node.is_null());
        *out = 730001;
        0
    }
    pub fn timeout(&self, node: Ptr, seconds: f32) -> i32 {
        // The original C comparison promotes its float argument to double.
        let seconds = f64::from(seconds);
        assert!(!node.is_null() && seconds > 0. && seconds <= 0.121);
        0
    }
    pub unsafe fn attr(&self, node: Ptr, key: Ptr, out: *mut Ptr) -> i32 {
        let key = &*key.cast::<NSString>();
        let position = key.isEqualToString(&NSString::from_str("AXPosition"));
        let size = key.isEqualToString(&NSString::from_str("AXSize"));
        if CFEqual(node, self.node(2)) != 0 && (position || size) {
            let (denied, point, dimensions) = {
                let mut data = self.data.lock().unwrap();
                data.reads += 1;
                (data.mode == "frame-denied", data.placement, data.dimensions)
            };
            if denied {
                *out = std::ptr::null_mut();
                return UNSUPPORTED;
            }
            *out = if position {
                AXValueCreate(1, (&point as *const NSPoint).cast())
            } else {
                AXValueCreate(2, (&dimensions as *const NSSize).cast())
            };
            return 0;
        }
        match self.value(node, key) {
            Some(value) => {
                *out = value.into_raw();
                0
            }
            None => {
                *out = std::ptr::null_mut();
                UNSUPPORTED
            }
        }
    }
    pub unsafe fn count(&self, node: Ptr, key: Ptr, out: *mut isize) -> i32 {
        let Some(value) = self.value(node, &*key.cast::<NSString>()) else {
            return UNSUPPORTED;
        };
        if CFGetTypeID(value.0) != CFArrayGetTypeID() {
            return UNSUPPORTED;
        }
        *out = CFArrayGetCount(value.0);
        0
    }
    pub unsafe fn values(
        &self,
        node: Ptr,
        key: Ptr,
        start: isize,
        max: isize,
        out: *mut Ptr,
    ) -> i32 {
        let Some(value) = self.value(node, &*key.cast::<NSString>()) else {
            return UNSUPPORTED;
        };
        if CFGetTypeID(value.0) != CFArrayGetTypeID() {
            return UNSUPPORTED;
        }
        let length = CFArrayGetCount(value.0) as usize;
        let start = start as usize;
        assert!(start <= length, "Owned array range must fit");
        let count = (max as usize).min(length - start);
        let values = (start..start + count)
            .map(|index| {
                Retained::retain(
                    CFArrayGetValueAtIndex(value.0, index as isize).cast::<AnyObject>(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        *out = CF::object(NSArray::from_retained_slice(&values)).into_raw();
        0
    }
}
