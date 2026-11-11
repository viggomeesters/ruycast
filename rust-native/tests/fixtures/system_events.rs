//! Build local reply descriptors only. Never send an AppleEvent or request consent.
use super::system_nodes::State;
use objc2::rc::Retained;
use objc2_foundation::{NSAppleEventDescriptor as D, NSAppleEventSendOptions, NSString};
pub(super) const fn code(value: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*value)
}
fn field(value: Option<&D>, key: &[u8; 4]) -> Option<Retained<D>> {
    value.and_then(|value| value.descriptorForKeyword(code(key)))
}
fn type_code(value: Option<&D>) -> u32 {
    value.map(D::typeCodeValue).unwrap_or(0)
}
fn list(items: &[Retained<D>]) -> Retained<D> {
    let list = D::listDescriptor();
    for value in items {
        list.insertDescriptor_atIndex(value, list.numberOfItems() + 1);
    }
    list
}
pub(super) fn event(
    state: &State,
    event: &D,
    flags: NSAppleEventSendOptions,
    seconds: f64,
) -> Retained<D> {
    // Preserve the exact original flags and C comparison (including NaN behavior).
    assert!(flags.0 == (3 | 16 | 4096 | 0x20000) && !(seconds <= 0. || seconds > 8.));
    let mode = {
        let mut data = state.data.lock().unwrap();
        data.reads += 1;
        data.mode.clone()
    };
    let reply = D::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
        code(b"aevt"),
        code(b"ansr"),
        None,
        0,
        0,
    );
    if mode == "automation-denied" {
        reply.setParamDescriptor_forKeyword(&D::descriptorWithInt32(-1743), code(b"errn"));
        return reply;
    }
    assert_eq!(event.eventClass(), code(b"core"));
    assert_eq!(
        event
            .attributeDescriptorForKeyword(code(b"addr"))
            .map(|value| value.descriptorType())
            .unwrap_or(0),
        code(b"kpid")
    );
    let reference = event.paramDescriptorForKeyword(code(b"----"));
    let key = type_code(field(reference.as_deref(), b"seld").as_deref());
    let from = field(reference.as_deref(), b"from");
    let current = type_code(field(from.as_deref(), b"seld").as_deref()) == code(b"cTab");
    if event.eventID() == code(b"setd") {
        let value = event.paramDescriptorForKeyword(code(b"data"));
        if key == code(b"cTab") {
            assert_eq!(
                type_code(field(value.as_deref(), b"want").as_deref()),
                code(b"bTab")
            );
            let selected = field(value.as_deref(), b"seld")
                .map(|value| value.int32Value())
                .unwrap_or(0);
            let mut data = state.data.lock().unwrap();
            data.selected = selected;
            data.effects += 1;
            if data.mode == "mutate-selection" {
                data.mode = "tab-changed".into();
            }
        } else {
            assert_eq!(key, code(b"pidx"));
            state.data.lock().unwrap().effects += 1;
        }
    } else {
        assert_eq!(event.eventID(), code(b"getd"));
        let title = if state.data.lock().unwrap().mode == "tab-changed" {
            "Changed"
        } else {
            "Owned Native Tab"
        };
        let value = if key == code(b"ID  ") {
            list(&[D::descriptorWithInt32(51)])
        } else if current {
            if key == code(b"pidx") {
                let selected = state.data.lock().unwrap().selected;
                D::descriptorWithInt32(selected)
            } else if key == code(b"pnam") {
                D::descriptorWithString(&NSString::from_str(title))
            } else {
                assert_eq!(key, code(b"pURL"));
                D::descriptorWithString(&NSString::from_str("https://example.invalid/native"))
            }
        } else if key == code(b"pnam") {
            list(&[D::descriptorWithString(&NSString::from_str(title))])
        } else {
            assert_eq!(key, code(b"pURL"));
            list(&[D::descriptorWithString(&NSString::from_str(
                "https://example.invalid/native",
            ))])
        };
        reply.setParamDescriptor_forKeyword(&value, code(b"----"));
    }
    reply
}
pub(super) fn consent(
    state: &State,
    target: *const std::ffi::c_void,
    class: u32,
    id: u32,
    ask: u8,
) -> i32 {
    assert!(!target.is_null() && class == code(b"****") && id == code(b"****") && ask == 1);
    state.data.lock().unwrap().consents += 1;
    0
}
