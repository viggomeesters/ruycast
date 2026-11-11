//! Incomplete provider genuinely omits GetEventParameter's fixture export.
const KEYBOARD_READY_GATE: Option<fn() -> bool> = None;
mod keyboard_sdk;
