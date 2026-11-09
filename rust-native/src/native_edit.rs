//! Native editing fixture; production routing is shared with all panels.
use objc2_app_kit::{NSEvent, NSEventModifierFlags as Flags, NSEventType, NSTextView};
pub use ruycast_native::responder_edit::perform;

// The native probe overrides clipboard selectors so routing can be exercised
// through the real panel without reading or writing the general pasteboard.
use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSMenu, NSPanel, NSTextField,
    NSUserInterfaceItemIdentification, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, NSObject, NSPoint, NSRange, NSRect, NSSize, NSString};
use std::cell::Cell;

define_class!(
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<u32>]
    struct ProbeEditor;
    impl ProbeEditor {
        #[unsafe(method(paste:))]
        fn paste_probe(&self, _: Option<&NSObject>) { self.ivars().set(self.ivars().get() | 1); }
        #[unsafe(method(copy:))]
        fn copy_probe(&self, _: Option<&NSObject>) { self.ivars().set(self.ivars().get() | 2); }
        #[unsafe(method(cut:))]
        fn cut_probe(&self, _: Option<&NSObject>) { self.ivars().set(self.ivars().get() | 4); }
        #[unsafe(method(selectAll:))]
        fn all_probe(&self, sender: Option<&NSObject>) {
            self.ivars().set(self.ivars().get() | 8);
            unsafe { let _: () = msg_send![super(self), selectAll: sender]; }
        }
    }
);
fn key(panel: &NSWindow, value: &str, shift: bool) -> Result<Retained<NSEvent>, String> {
    let characters = if shift {
        value.to_uppercase()
    } else {
        value.to_owned()
    };
    let string = NSString::from_str(&characters);
    let code = match value {
        "v" => 9,
        "a" => 0,
        "z" => 6,
        "c" => 8,
        "x" => 7,
        _ => 0,
    };
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::new(0.,0.), Flags::Command | if shift {Flags::Shift} else {Flags::empty()},
        0., panel.windowNumber(), None, &string, &string, false, code).ok_or("local key event allocation failed".into())
}
pub fn check(panel: &NSPanel, view: &NSView, mtm: MainThreadMarker) -> Result<(), String> {
    let editor: Retained<ProbeEditor> = unsafe {
        let editor = ProbeEditor::alloc(mtm).set_ivars(Cell::new(0));
        msg_send![super(editor), initWithFrame: NSRect::new(NSPoint::new(0.,0.), NSSize::new(100.,40.))]
    };
    view.addSubview(&editor);
    struct Remove<'a>(&'a NSView);
    impl Drop for Remove<'_> {
        fn drop(&mut self) {
            self.0.removeFromSuperview();
        }
    }
    let _remove = Remove(&editor);
    editor.setString(ns_string!("abcdef"));
    editor.setAllowsUndo(true);
    if !panel.makeFirstResponder(Some(&editor)) {
        return Err("native editing responder failed".into());
    }
    for value in ["v", "c", "x", "a"] {
        let event = key(panel, value, false)?;
        let handled: bool = unsafe { msg_send![panel, performKeyEquivalent: &*event] };
        if !handled {
            return Err(format!("panel did not route native editing {value}"));
        }
    }
    if editor.ivars().get() != 15 || editor.selectedRange().length != 6 {
        return Err("native editing selectors/Select All failed".into());
    }
    editor.setEditable(false);
    for value in ["v", "x", "z"] {
        if perform(&*key(panel, value, false)?, Some(&editor)) {
            return Err("read-only editing was accepted".into());
        }
    }
    editor.setEditable(true);
    editor.setString(ns_string!("a"));
    let manager = editor.undoManager().ok_or("missing native Undo manager")?;
    manager.removeAllActions();
    manager.beginUndoGrouping();
    unsafe {
        let _: () =
            msg_send![&*editor, insertText: ns_string!("b"), replacementRange: NSRange::new(1,0)];
    }
    manager.endUndoGrouping();
    if editor.string().to_string() != "ab" || !manager.canUndo() {
        return Err("native text insertion did not register Undo".into());
    }
    let event = key(panel, "z", false)?;
    let handled: bool = unsafe { msg_send![panel, performKeyEquivalent: &*event] };
    if !handled || editor.string().to_string() != "a" || !manager.canRedo() {
        return Err("native Undo failed".into());
    }
    let event = key(panel, "z", true)?;
    let handled: bool = unsafe { msg_send![panel, performKeyEquivalent: &*event] };
    if !handled || editor.string().to_string() != "ab" {
        return Err("native Redo failed".into());
    }
    check_snippet_replacement(&editor)?;
    panel.makeFirstResponder(None);
    check_menu(mtm, &editor)?;
    Ok(())
}

/// Original InteractionChecks local-data conjunctions. The supplied editor is
/// already owned by the hidden native check; no event is posted or sourced.
fn check_snippet_replacement(editor: &NSTextView) -> Result<(), String> {
    use ruycast_native::{keyword_matcher::{self, Catalog, ChoiceRequest, Matcher, Replacement}, templates::{Context, Limits, Plan}};
    let apply = |replacement: &Replacement| {
        replacement.apply(
            |code| unsafe {
                match code {
                    51 => { let _: () = msg_send![editor, deleteBackward: None::<&NSObject>]; }
                    123 => { let _: () = msg_send![editor, moveLeft: None::<&NSObject>]; }
                    _ => panic!("unexpected local replacement key"),
                }
            },
            |text| unsafe {
                let _: () = msg_send![editor, insertText: &*NSString::from_str(text), replacementRange: editor.selectedRange()];
            },
        );
    };
    editor.setString(ns_string!("prefix ;hell"));
    editor.setSelectedRange(NSRange::new(12, 0));
    apply(&Replacement::new(";hello", "o", "Hi Ω!".into(), Some(3)));
    if editor.string().to_string() != "prefix Hi Ω!" || editor.selectedRange() != NSRange::new(10, 0) {
        return Err("local snippet replacement/cursor differs".into());
    }
    let mut command = ruycast_native::session::fixture(1).remove(0);
    command.id = "snippet.now".into(); command.kind = "snippet".into();
    command.target = "{datetime}".into(); command.keyword = Some(";;".into());
    let catalog = Catalog::new(&[command.clone()])?;
    let mut matcher = Matcher::default();
    if !keyword_matcher::allows_expansion(false, None)
        || matcher.type_text(";", &catalog).is_some()
        || matcher.type_text(";", &catalog).map(|c|c.id.as_str()) != Some("snippet.now")
        || keyword_matcher::allows_expansion(true, None)
        || keyword_matcher::allows_expansion(false, Some("AXSecureTextField"))
    { return Err("local datetime matcher/focus policy differs".into()); }
    let context = Context { clipboard: None, now: 1_761_420_900., locale: Some("en_GB"), time_zone: Some("GMT") };
    let (text, cursor) = catalog.plan(&command.id).ok_or("missing local datetime plan")?.render(&[], &context)?;
    editor.setString(ns_string!("prefix ;")); editor.setSelectedRange(NSRange::new(8, 0));
    apply(&Replacement::new(";;", ";", text, cursor));
    if editor.string().to_string() != "prefix 25 October 2025 19:35" {
        return Err("local datetime expansion differs".into());
    }

    // Genuine local CGEvent objects, never the keyboard fixture table. Official
    // CGEvent.h uses bool/u16 and UniCharCount = unsigned long on this platform.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(source: *const std::ffi::c_void, key: u16, down: bool) -> *mut std::ffi::c_void;
        fn CGEventKeyboardSetUnicodeString(event: *mut std::ffi::c_void, length: usize, text: *const u16);
        fn CGEventKeyboardGetUnicodeString(event: *mut std::ffi::c_void, capacity: usize, length: *mut usize, text: *mut u16);
    }
    use objc2_core_foundation::{CFRetained, CFType};
    let long = "Line Ω 👨‍👩‍👧‍👦\n".repeat(40);
    let mut roundtrip = String::new();
    let mut valid = true;
    Replacement::new(";test", "t", long.clone(), None).apply(|_| {}, |chunk| {
        let units: Vec<_> = chunk.encode_utf16().collect();
        if units.is_empty() || units.len() > 16 { valid = false; return; }
        let Some(pointer) = std::ptr::NonNull::new(unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), 0, true) }) else { valid = false; return; };
        // Adopt exactly the creator's +1; the existing CF owner releases it on
        // every return, including a failed Unicode check.
        let event = unsafe { CFRetained::<CFType>::from_raw(pointer.cast()) };
        let pointer = CFRetained::as_ptr(&event).as_ptr().cast();
        let mut output = [0u16; 32]; let mut length = 0usize;
        unsafe {
            CGEventKeyboardSetUnicodeString(pointer, units.len(), units.as_ptr());
            CGEventKeyboardGetUnicodeString(pointer, output.len(), &mut length, output.as_mut_ptr());
        }
        if units.len() > 16 || length > 16 || length > output.len() || output[..length] != units {
            valid = false; return;
        }
        match String::from_utf16(&output[..length]) {
            Ok(text) if text == chunk => roundtrip.push_str(&text),
            _ => valid = false,
        }
    });
    if !valid || roundtrip != long { return Err("local CGEvent Unicode roundtrip differs".into()); }

    command.id = "snippet.choice".into(); command.keyword = Some(";contact".into());
    command.target = "{argument name=\"Contact\" options=\"Example A, Example B\"}{cursor} Ω".into();
    let fields = Plan::parse(&command, Limits::application())?;
    if fields.fields().first().map(|f|f.options.as_slice()) != Some(&["Example A".to_owned(), "Example B".to_owned()][..]) {
        return Err("local choice options differ".into());
    }
    editor.setString(ns_string!("prefix ;contact")); editor.setSelectedRange(NSRange::new(15, 0));
    let mut cancelled = ChoiceRequest::new(command.clone(), 42); cancelled.cancel();
    if cancelled.active() || editor.string().to_string() != "prefix ;contact" {
        return Err("local choice cancellation changed text".into());
    }
    let mut chosen = ChoiceRequest::new(command, 42);
    apply(&chosen.replacement(&[("Contact".into(), "Example B".into())], 42, true, true, &context)?);
    if editor.string().to_string() != "prefix Example B Ω" || editor.selectedRange() != NSRange::new(16, 0) {
        return Err("local choice replacement/cursor differs".into());
    }
    Ok(())
}

fn check_menu(main: MainThreadMarker, editor: &ProbeEditor) -> Result<(), String> {
    let app = NSApplication::sharedApplication(main);
    struct RestoreMenu(Retained<NSApplication>, Option<Retained<NSMenu>>);
    impl Drop for RestoreMenu {
        fn drop(&mut self) {
            self.0.setMainMenu(self.1.as_deref());
        }
    }
    let _restore = RestoreMenu(app.clone(), app.mainMenu());
    let first = crate::native_edit_menu::install(main);
    let expected = ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"];
    if first.labels() != expected {
        return Err("standard Edit menu labels differ".into());
    }
    drop(first);
    let second = crate::native_edit_menu::install(main);
    let count = app
        .mainMenu()
        .ok_or("missing main menu")?
        .itemArray()
        .iter()
        .filter_map(|item| item.submenu())
        .filter(|menu| {
            menu.identifier()
                .is_some_and(|id| id.to_string() == "native-edit")
        })
        .count();
    if count != 1 || second.labels() != expected {
        return Err("Edit menu was duplicated or lost its owner".into());
    }
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(main),
            NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(400., 240.)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    struct CloseWindow<'a>(&'a NSWindow);
    impl Drop for CloseWindow<'_> {
        fn drop(&mut self) {
            self.0.makeFirstResponder(None);
            self.0.close();
        }
    }
    let _close = CloseWindow(&window);
    window.setContentView(Some(editor));
    editor.setEditable(true);
    editor.setSelectable(true);
    editor.setString(ns_string!("abcdef"));
    editor.ivars().set(0);
    if !window.makeFirstResponder(Some(editor)) {
        return Err("ordinary window text responder missing".into());
    }
    let menu = crate::native_edit_menu::for_window(main, &window);
    for value in ["v", "a", "c", "x"] {
        if !menu.perform(&*key(&window, value, false)?) {
            return Err(format!("ordinary window Edit menu did not route {value}"));
        }
    }
    if editor.ivars().get() != 15 || editor.selectedRange().length != 6 {
        return Err("ordinary window Edit selectors/selection differ".into());
    }
    editor.setString(ns_string!("a"));
    let undo = editor
        .undoManager()
        .ok_or("ordinary window Undo manager missing")?;
    undo.removeAllActions();
    editor.breakUndoCoalescing();
    undo.setGroupsByEvent(false);
    undo.beginUndoGrouping();
    unsafe {
        let _: () =
            msg_send![editor, insertText: ns_string!("b"), replacementRange: NSRange::new(1,0)];
    }
    undo.endUndoGrouping();
    if !menu.perform(&*key(&window, "z", false)?) || editor.string().to_string() != "a" {
        return Err("ordinary window menu Undo failed".into());
    }
    let redo_enabled = menu.is_enabled(objc2::sel!(redo:));
    let before_redo = undo.canRedo();
    let redo_handled = menu.perform(&*key(&window, "z", true)?);
    if !redo_enabled || !redo_handled || editor.string().to_string() != "ab" {
        return Err(format!("ordinary window menu Redo failed: enabled={redo_enabled}, handled={redo_handled}, text={}, beforeCanRedo={before_redo}, afterCanRedo={}, afterCanUndo={}", editor.string(), undo.canRedo(), undo.canUndo()));
    }
    editor.setSelectedRange(NSRange::new(0, 1));
    editor.setEditable(false);
    if menu.is_enabled(objc2::sel!(paste:))
        || menu.is_enabled(objc2::sel!(cut:))
        || menu.is_enabled(objc2::sel!(undo:))
        || menu.is_enabled(objc2::sel!(redo:))
        || !menu.is_enabled(objc2::sel!(copy:))
        || !menu.is_enabled(objc2::sel!(selectAll:))
    {
        return Err("read-only Edit menu validation differs".into());
    }
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(main),
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(300., 30.)),
    );
    field.setStringValue(ns_string!("Form field"));
    window.setContentView(Some(&field));
    if !window.makeFirstResponder(Some(&field)) || !menu.perform(&*key(&window, "a", false)?) {
        return Err("ordinary native field editor Select All failed".into());
    }
    let field_editor = window
        .firstResponder()
        .and_then(|r| r.downcast::<NSTextView>().ok())
        .ok_or("ordinary field editor absent")?;
    if field_editor.selectedRange().length != 10 {
        return Err("ordinary field editor selection differs".into());
    }
    window.makeFirstResponder(None);
    for action in [
        objc2::sel!(undo:),
        objc2::sel!(redo:),
        objc2::sel!(cut:),
        objc2::sel!(copy:),
        objc2::sel!(paste:),
        objc2::sel!(selectAll:),
    ] {
        if menu.is_enabled(action) {
            return Err("Edit action enabled without a text responder".into());
        }
    }
    if window.isVisible() || window.isKeyWindow() || window.isMainWindow() {
        return Err("ordinary Edit check ordered or focused a window".into());
    }
    Ok(())
}
