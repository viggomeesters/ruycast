//! Mutate only the shared synthetic AX state. Never send an AX action/message.
use super::system_nodes::{AXValueGetType, AXValueGetValue, CFEqual, Ptr, State};
use objc2_foundation::{NSNumber, NSPoint, NSSize, NSString};
pub(super) const CANNOT_COMPLETE: i32 = -25204;
pub(super) unsafe fn set(state: &State, node: Ptr, key: Ptr, value: Ptr) -> i32 {
    let key = &*key.cast::<NSString>();
    if CFEqual(node, state.node(2)) != 0 {
        if key.isEqualToString(&NSString::from_str("AXPosition")) {
            assert_eq!(AXValueGetType(value), 1);
            let mut point = NSPoint::new(0., 0.);
            assert_ne!(
                AXValueGetValue(value, 1, (&mut point as *mut NSPoint).cast()),
                0
            );
            let mut data = state.data.lock().unwrap();
            data.placement = point;
            data.effects += 1;
            return if data.mode == "frame-cannot-complete" {
                CANNOT_COMPLETE
            } else {
                0
            };
        }
        if key.isEqualToString(&NSString::from_str("AXSize")) {
            assert_eq!(AXValueGetType(value), 2);
            let mut dimensions = NSSize::new(0., 0.);
            assert_ne!(
                AXValueGetValue(value, 2, (&mut dimensions as *mut NSSize).cast()),
                0
            );
            let mut data = state.data.lock().unwrap();
            data.dimensions = dimensions;
            if data.mode == "frame-constrained" {
                data.dimensions.width = 100.;
            }
            data.effects += 1;
            return if data.mode == "frame-cannot-complete" {
                CANNOT_COMPLETE
            } else {
                0
            };
        }
    }
    let false_value = NSNumber::numberWithBool(false);
    assert!(
        CFEqual(node, state.node(3)) != 0
            && key.isEqualToString(&NSString::from_str("AXMinimized"))
            && CFEqual(
                value,
                (&*false_value as *const NSNumber as *mut NSNumber).cast()
            ) != 0
    );
    let mut data = state.data.lock().unwrap();
    data.effects += 1;
    data.mode = "restored".into();
    0
}
pub(super) unsafe fn action(state: &State, node: Ptr, key: Ptr) -> i32 {
    let key = &*key.cast::<NSString>();
    assert!(
        key.isEqualToString(&NSString::from_str("AXRaise"))
            || key.isEqualToString(&NSString::from_str("AXPress"))
    );
    assert!(!node.is_null());
    state.data.lock().unwrap().effects += 1;
    0
}
