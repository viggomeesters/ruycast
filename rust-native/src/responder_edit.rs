//! Local responder editing. No process-wide key monitor or clipboard reads.
use objc2::{runtime::Sel, sel};
use objc2_app_kit::{NSEvent, NSEventModifierFlags as Flags, NSEventType, NSResponder, NSTextView};

pub fn perform(event: &NSEvent, responder: Option<&NSResponder>) -> bool {
    if event.r#type() != NSEventType::KeyDown {
        return false;
    }
    let Some(editor) = responder.and_then(|r| r.downcast_ref::<NSTextView>()) else {
        return false;
    };
    let flags =
        event.modifierFlags() & (Flags::Command | Flags::Shift | Flags::Control | Flags::Option);
    let key = event
        .charactersIgnoringModifiers()
        .map(|v| crate::native_text::repaired(&v).to_lowercase());
    if key.as_deref() == Some("z")
        && (flags == Flags::Command || flags == (Flags::Command | Flags::Shift))
    {
        if !editor.isEditable() {
            return false;
        }
        if let Some(manager) = editor.undoManager() {
            if flags.contains(Flags::Shift) {
                if manager.canRedo() {
                    manager.redo();
                }
            } else if manager.canUndo() {
                manager.undo();
            }
        }
        return true;
    }
    if flags != Flags::Command {
        return false;
    }
    let action: Sel = match key.as_deref() {
        Some("v") if editor.isEditable() => sel!(paste:),
        Some("x") if editor.isEditable() => sel!(cut:),
        Some("c") => sel!(copy:),
        Some("a") => sel!(selectAll:),
        _ => return false,
    };
    unsafe { editor.tryToPerform_with(action, None) }
}
