//! The existing standard Edit menu follows the active native text responder.
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSMenu, NSMenuItem, NSMenuItemValidation,
    NSTextView, NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{ns_string, NSObject, NSObjectProtocol, NSString};

fn boundary<T>(operation: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => Some(value),
        Err(payload) => {
            std::mem::forget(payload);
            None
        }
    }
}

struct Source {
    // None uses the actual key window. Tests supply a retained own hidden window.
    window: Option<Weak<NSWindow>>,
}
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Source]
    struct EditTarget;
    unsafe impl NSObjectProtocol for EditTarget {}
    unsafe impl NSMenuItemValidation for EditTarget {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            boundary(|| self.enabled(item.action())).unwrap_or(false)
        }
    }
    impl EditTarget {
        #[unsafe(method(undo:))]
        fn undo(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(undo:), sender)); }
        #[unsafe(method(redo:))]
        fn redo(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(redo:), sender)); }
        #[unsafe(method(cut:))]
        fn cut(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(cut:), sender)); }
        #[unsafe(method(copy:))]
        fn copy(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(copy:), sender)); }
        #[unsafe(method(paste:))]
        fn paste(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(paste:), sender)); }
        #[unsafe(method(selectAll:))]
        fn select_all(&self, sender: Option<&AnyObject>) { boundary(|| self.dispatch(sel!(selectAll:), sender)); }
    }
);
impl EditTarget {
    fn editor(&self) -> Option<Retained<NSTextView>> {
        let window = match &self.ivars().window {
            Some(window) => window.load(),
            None => NSApplication::sharedApplication(self.mtm()).keyWindow(),
        }?;
        window.firstResponder()?.downcast::<NSTextView>().ok()
    }
    fn enabled(&self, action: Option<Sel>) -> bool {
        let Some(editor) = self.editor() else {
            return false;
        };
        match action {
            Some(action) if action == sel!(undo:) => {
                editor.isEditable() && editor.undoManager().is_some_and(|m| m.canUndo())
            }
            Some(action) if action == sel!(redo:) => {
                editor.isEditable() && editor.undoManager().is_some_and(|m| m.canRedo())
            }
            Some(action) if action == sel!(cut:) => {
                editor.isEditable() && editor.selectedRange().length > 0
            }
            Some(action) if action == sel!(copy:) => editor.selectedRange().length > 0,
            Some(action) if action == sel!(paste:) => editor.isEditable(),
            Some(action) if action == sel!(selectAll:) => editor.isSelectable(),
            _ => false,
        }
    }
    fn dispatch(&self, action: Sel, sender: Option<&AnyObject>) {
        let Some(editor) = self.editor() else { return };
        if action == sel!(undo:) || action == sel!(redo:) {
            if !editor.isEditable() {
                return;
            }
            if let Some(manager) = editor.undoManager() {
                if action == sel!(undo:) && manager.canUndo() {
                    manager.undo();
                }
                if action == sel!(redo:) && manager.canRedo() {
                    manager.redo();
                }
            }
        } else if (action != sel!(cut:) && action != sel!(paste:)) || editor.isEditable() {
            unsafe {
                editor.tryToPerform_with(action, sender);
            }
        }
    }
}

pub struct Installed {
    menu: Retained<NSMenu>,
    // NSMenuItem.target is not an owner. Retain the actual existing owners too.
    _targets: Vec<Retained<AnyObject>>,
}
impl Installed {
    fn retain(menu: Retained<NSMenu>) -> Self {
        let targets = menu
            .itemArray()
            .iter()
            .filter_map(|item| item.target())
            .collect();
        Self {
            menu,
            _targets: targets,
        }
    }
    pub fn labels(&self) -> Vec<String> {
        self.menu
            .itemArray()
            .iter()
            .filter(|item| !item.isSeparatorItem())
            .map(|item| item.title().to_string())
            .collect()
    }
    pub fn is_enabled(&self, action: Sel) -> bool {
        self.menu.update();
        self.menu
            .itemArray()
            .iter()
            .find(|item| item.action() == Some(action))
            .is_some_and(|item| item.isEnabled())
    }
    pub fn perform(&self, event: &NSEvent) -> bool {
        unsafe { msg_send![&*self.menu, performKeyEquivalent: event] }
    }
}
fn menu(main: MainThreadMarker, window: Option<&NSWindow>) -> Installed {
    let target = EditTarget::alloc(main).set_ivars(Source {
        window: window.map(Weak::new),
    });
    let target: Retained<EditTarget> = unsafe { msg_send![super(target), init] };
    let menu = NSMenu::initWithTitle(NSMenu::alloc(main), ns_string!("Edit"));
    menu.setIdentifier(Some(ns_string!("native-edit")));
    for (title, action, key, shift) in [
        ("Undo", sel!(undo:), "z", false),
        ("Redo", sel!(redo:), "Z", true),
        ("Cut", sel!(cut:), "x", false),
        ("Copy", sel!(copy:), "c", false),
        ("Paste", sel!(paste:), "v", false),
        ("Select All", sel!(selectAll:), "a", false),
    ] {
        if title == "Cut" {
            menu.addItem(&NSMenuItem::separatorItem(main));
        }
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(main),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        item.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command
                | if shift {
                    NSEventModifierFlags::Shift
                } else {
                    NSEventModifierFlags::empty()
                },
        );
        // The actual menu retains its action owner independently of a caller guard.
        unsafe {
            item.setRepresentedObject(Some(&*target));
            item.setTarget(Some(&*target));
        }
        menu.addItem(&item);
    }
    Installed::retain(menu)
}
pub fn for_window(main: MainThreadMarker, window: &NSWindow) -> Installed {
    menu(main, Some(window))
}
pub fn install(main: MainThreadMarker) -> Installed {
    let app = NSApplication::sharedApplication(main);
    let main_menu = app.mainMenu().unwrap_or_else(|| NSMenu::new(main));
    if let Some(menu) = main_menu
        .itemArray()
        .iter()
        .filter_map(|item| item.submenu())
        .find(|menu| {
            menu.identifier()
                .is_some_and(|id| id.to_string() == "native-edit")
        })
    {
        return Installed::retain(menu);
    }
    if main_menu.numberOfItems() == 0 {
        let app_menu = NSMenu::initWithTitle(NSMenu::alloc(main), ns_string!("Ruycast"));
        let quit = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(main),
                ns_string!("Quit Ruycast"),
                Some(sel!(terminate:)),
                ns_string!("q"),
            )
        };
        app_menu.addItem(&quit);
        let row = NSMenuItem::new(main);
        row.setTitle(ns_string!("Ruycast"));
        row.setSubmenu(Some(&app_menu));
        main_menu.addItem(&row);
    }
    let installed = menu(main, None);
    let row = NSMenuItem::new(main);
    row.setTitle(ns_string!("Edit"));
    row.setSubmenu(Some(&installed.menu));
    main_menu.addItem(&row);
    app.setMainMenu(Some(&main_menu));
    installed
}
