//! Native action and binding controls. Every callback stays on the main thread;
//! weak owners are disconnected before the controller is released.
use crate::native::{Controller, RecordMenuContext};
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString, NSObject, NSObjectProtocol};
use ruycast_native::{
    actions::{EditorView, Item, Mode},
    presentation,
    recorder::{Recorded, Recorder},
};
use std::cell::{Cell, RefCell};

#[derive(Clone, Default)]
pub struct SurfaceData {
    pub launcher_scope: Option<ruycast_native::session::LauncherScope>,
    pub system_scope: Option<ruycast_native::system_search::Scope>,
    pub system_busy: bool,
    pub app_choices: ruycast_native::app_choice::Choices,
    pub app_choices_pending: bool,
    pub visible: bool,
    pub execution_running: bool,
    pub actions_open: bool,
    pub menu_token: u64,
    pub action_selection: usize,
    pub items: Vec<Item>,
    pub title: String,
    pub editor: Option<EditorView>,
    pub separate: bool,
    pub allow_hidden: bool,
}

define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=Cell<bool>]
    pub(crate) struct SurfaceBackground;
    impl SurfaceBackground{
        #[unsafe(method(drawRect:))]
        fn draw(&self,_:NSRect){
            if self.identifier().is_some_and(|s|s.to_string()=="actions-menu"){
                NSColor::windowBackgroundColor().setFill();let bounds=self.bounds();let path=NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(0.5,0.5,bounds.size.width-1.,bounds.size.height-1.),9.,9.);path.fill();NSColor::separatorColor().setStroke();path.setLineWidth(1.);path.stroke();
            }else{crate::native_settings::color(crate::native_settings::palette(self,self.ivars().get()).background).setFill();NSRectFill(self.bounds());}
        }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self)->bool{true}
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent){
            if let Some(window)=self.window(){unsafe{let _:bool=msg_send![&*window,performKeyEquivalent:event];}}
        }
    }
);
define_class!(
    #[unsafe(super=NSClipView)]
    #[thread_kind=MainThreadOnly]
    pub(crate) struct InlineClip;
    impl InlineClip {
        #[unsafe(method_id(hitTest:))]
        fn hit(&self,point:NSPoint)->Option<Retained<NSView>> {
            let hit:Option<Retained<NSView>>=unsafe{msg_send![super(self),hitTest:point]};
            hit.filter(|v|v.downcast_ref::<NSControl>().is_some() || v.downcast_ref::<NSTextView>().is_some())
        }
    }
);
define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    pub(crate) struct InlineDocument;
    impl InlineDocument { #[unsafe(method(isFlipped))] fn flipped(&self)->bool{true} }
);
pub(crate) struct RecordState {
    lease: RefCell<Option<ruycast_native::keyboard_service::RecorderLease>>,
    beginning: Cell<bool>,
    token: Cell<u64>,
    owner: RefCell<Option<Weak<Controller>>>,
    pub recording: Cell<bool>,
    parse: RefCell<Recorder>,
    pub value: RefCell<Option<String>>,
    pub separate: Cell<bool>,
    menu_generation: Cell<u64>,
}
define_class!(
    #[unsafe(super=NSButton)]
    #[thread_kind=MainThreadOnly]
    #[ivars=RecordState]
    pub(crate) struct RecordButton;
    impl RecordButton {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self)->bool{true}
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(&self,_event:&NSEvent)->Option<Retained<NSMenu>>{self.context_menu()}
        #[unsafe(method(resignFirstResponder))]
        fn resign(&self)->bool{self.stop();unsafe{msg_send![super(self),resignFirstResponder]}}
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent){
            if self.preparing(){if let Some(owner)=self.owner(){owner.preparing_record_key(event);}return;}
            if self.recording(){if let Some(owner)=self.owner(){owner.guard_callback(||{let result=self.ivars().parse.borrow_mut().key(event.keyCode(),event.modifierFlags().0 as u64,event.isARepeat(),self.ivars().separate.get());self.deliver(&owner,result);});}}
            else{unsafe{let _:()=msg_send![super(self),keyDown:event];}}
        }
        #[unsafe(method(flagsChanged:))]
        fn flags(&self,event:&NSEvent){
            if self.recording(){if let Some(owner)=self.owner(){owner.guard_callback(||{let result=self.ivars().parse.borrow_mut().flags(event.keyCode(),event.modifierFlags().0 as u64);self.deliver(&owner,result);});}}
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn equivalent(&self,event:&NSEvent)->bool{
            if self.active(){unsafe{let _:()=msg_send![self,keyDown:event];}true}else{unsafe{msg_send![super(self),performKeyEquivalent:event]}}
        }
    }
);
impl RecordButton {
    pub fn settings(main:MainThreadMarker)->Retained<Self>{
        let b=Self::alloc(main).set_ivars(RecordState{owner:RefCell::new(None),lease:RefCell::new(None),beginning:Cell::new(false),token:Cell::new(0),recording:Cell::new(false),parse:RefCell::new(Recorder::default()),value:RefCell::new(None),separate:Cell::new(true),menu_generation:Cell::new(1)});
        unsafe{msg_send![super(b),initWithFrame:rect(0.,0.,190.,30.)]}
    }
    pub fn bind_settings(&self,owner:&Controller){self.bind_owner(owner);}
    pub fn cleanup_settings(&self){self.stop();self.clear_owner();unsafe{self.setTarget(None);self.setAction(None);}}


    fn invalidate_menu(&self) {
        let current = self.ivars().menu_generation.get();
        self.ivars().menu_generation.set(if current == 0 { 0 } else { current.checked_add(1).unwrap_or(0) });
    }
    fn bind_owner(&self, owner: &Controller) {
        if !self.owner().is_some_and(|old| std::ptr::eq::<Controller>(&*old, owner)) {
            self.invalidate_menu();
        }
        *self.ivars().owner.borrow_mut() = Some(Weak::new(owner));
    }
    fn clear_owner(&self) {
        self.invalidate_menu();
        self.ivars().owner.borrow_mut().take();
    }
    fn menu_available(&self) -> bool {
        !self.active() && self.isEnabled() && !self.isHiddenOrHasHiddenAncestor()
            && self.window().is_some() && self.ivars().menu_generation.get() != 0
    }
    pub(crate) fn context_menu(&self) -> Option<Retained<NSMenu>> {
        let owner = self.owner()?;
        owner.guard_callback(|| {
            if !self.menu_available() { return None; }
            let expected = self.value()?;
            either_command(&expected)?;
            let context = owner.recorder_menu_context(self)?;
            let main = self.mtm();
            let target = RecordMenuTarget::alloc(main).set_ivars(RecordMenuAction {
                owner: Weak::new(&owner), record: Weak::new(self), context, expected,
                generation: self.ivars().menu_generation.get(), used: Cell::new(false),
            });
            let target: Retained<RecordMenuTarget> = unsafe { msg_send![super(target), init] };
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(main),
                    ns_string!("Use Either Command"), Some(objc2::sel!(useEitherCommand:)), ns_string!(""))
            };
            // Match the existing Edit menu: target is non-retaining, so the
            // actual item owns its action target through representedObject.
            // Menu copies retain that same one-shot owner, never a raw target.
            unsafe { item.setRepresentedObject(Some(&*target)); item.setTarget(Some(&*target)); }
            let menu = NSMenu::initWithTitle(NSMenu::alloc(main), ns_string!(""));
            menu.addItem(&item);
            Some(menu)
        }).flatten()
    }
    fn owner(&self) -> Option<Retained<Controller>> {
        self.ivars().owner.borrow().as_ref().and_then(Weak::load)
    }
    fn deliver(&self, owner: &Controller, result: Recorded) {
        match result {
            Recorded::Ignore => {}
            Recorded::Cancel => {
                self.stop();
                owner.cancel_recording();
            }
            Recorded::Clear => {
                self.stop();
                owner.save_hotkey(None);
            }
            Recorded::Shortcut(key) => {
                self.stop();
                owner.save_hotkey(Some(key));
            }
        }
    }
    pub fn value(&self) -> Option<String> {
        self.ivars().value.borrow().clone()
    }
    pub fn preparing(&self) -> bool {
        self.ivars().beginning.get()
    }
    pub fn active(&self) -> bool {
        self.recording() || self.preparing()
    }
    pub fn recording(&self) -> bool {
        self.ivars().recording.get()
    }
    pub fn begin(&self) {
        if self.recording() || self.ivars().beginning.get() {
            return;
        }
        let Some(owner) = self.owner() else { return };
        let Some(token) = owner.recording_owner() else {
            return;
        };
        self.invalidate_menu();
        // Acquire before makeFirstResponder: AppKit may reenter an SDK call.
        let lease = match owner.recording_lease() {
            Ok(lease) => lease,
            Err(error) => {
                owner.recording_error(token, &error);
                return;
            }
        };
        self.ivars().token.set(token);
        *self.ivars().lease.borrow_mut() = lease;
        self.ivars().beginning.set(true);
        if !self
            .window()
            .is_some_and(|w| w.makeFirstResponder(Some(self)))
        {
            self.stop();
            return;
        }
        self.setTitle(ns_string!("Preparing…"));
        self.setAccessibilityLabel(Some(ns_string!("Preparing shortcut recording")));
        self.finish_pending();
    }
    pub fn finish_pending(&self) {
        if !self.ivars().beginning.get() {
            return;
        }
        let Some(owner) = self.owner() else {
            self.stop();
            return;
        };
        let token = self.ivars().token.get();
        if owner.recording_owner() != Some(token)
            || !self.window().is_some_and(|w| {
                w.firstResponder().is_some_and(|r| {
                    std::ptr::eq(&*r as *const _ as *const (), self as *const _ as *const ())
                })
            })
        {
            self.stop();
            return;
        }
        let ready = {
            let lease = self.ivars().lease.borrow();
            lease.as_ref().map(|l| l.ready()).unwrap_or(Ok(true))
        };
        match ready {
            Ok(false) => {}
            Err(error) => {
                self.stop();
                owner.recording_error(token, &error);
            }
            Ok(true) => {
                self.ivars().beginning.set(false);
                self.ivars().recording.set(true);
                self.ivars().parse.borrow_mut().reset();
                self.setTitle(ns_string!("Press shortcut…"));
                self.setAccessibilityLabel(Some(ns_string!("Press shortcut")));
                self.setToolTip(Some(ns_string!("Escape to cancel · Delete to clear")));
            }
        }
    }
    pub fn stop(&self) {
        if self.active() || self.ivars().token.get() != 0 { self.invalidate_menu(); }
        self.ivars().beginning.set(false);
        self.ivars().token.set(0);
        self.ivars().recording.set(false);
        self.ivars().lease.borrow_mut().take();
        self.ivars().parse.borrow_mut().reset();
        let value = self.ivars().value.borrow().clone();
        self.configure(value);
    }
    pub fn configure(&self, value: Option<String>) {
        if *self.ivars().value.borrow() != value { self.invalidate_menu(); }
        *self.ivars().value.borrow_mut() = value.clone();
        if !self.recording() && !self.ivars().beginning.get() {
            let (text, label) = value
                .as_deref()
                .map(presentation::shortcut)
                .unwrap_or_else(|| ("Record Shortcut".into(), "Record Shortcut".into()));
            self.setTitle(&NSString::from_str(&text));
            self.setAccessibilityLabel(Some(&NSString::from_str(&label)));
        }
    }
}
/// Only a validated sided modifier is generalized; key/other modifiers retain
/// the existing core parser's canonical spelling and validity rules.
pub(crate) fn either_command(value: &str) -> Option<String> {
    let normalized = rustcast_core::shortcut(value).ok()?;
    let mut parts = normalized.split('+').collect::<Vec<_>>();
    let modifiers = parts.len().checked_sub(1)?;
    let side = parts[..modifiers].iter().position(|part| matches!(*part, "left-cmd" | "right-cmd"))?;
    parts[side] = "cmd";
    rustcast_core::shortcut(&parts.join("+")).ok()
}
struct RecordMenuAction {
    owner: Weak<Controller>,
    record: Weak<RecordButton>,
    context: RecordMenuContext,
    expected: String,
    generation: u64,
    used: Cell<bool>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    #[ivars=RecordMenuAction]
    struct RecordMenuTarget;
    unsafe impl NSObjectProtocol for RecordMenuTarget {}
    unsafe impl NSMenuItemValidation for RecordMenuTarget {
        #[unsafe(method(validateMenuItem:))]
        fn validate(&self,item:&NSMenuItem)->bool { self.valid_menu_item(item) }
    }
    impl RecordMenuTarget {
        #[unsafe(method(useEitherCommand:))]
        fn invoke(&self,_sender:Option<&NSObject>) {
            let Some(owner)=self.ivars().owner.load() else{return;};
            owner.guard_callback(||{
                let Some(record)=self.bound_record(&owner) else{return;};
                if self.ivars().used.replace(true){return;}
                record.invalidate_menu();
                owner.apply_recorder_menu(&record,&self.ivars().context,&self.ivars().expected);
            });
        }
    }
);
impl RecordMenuTarget {
    fn valid_menu_item(&self, item: &NSMenuItem) -> bool {
        if item.action() != Some(objc2::sel!(useEitherCommand:)) { return false; }
        let Some(owner) = self.ivars().owner.load() else { return false; };
        owner.guard_callback(|| self.bound_record(&owner).is_some()).unwrap_or(false)
    }
    fn bound_record(&self, owner: &Controller) -> Option<Retained<RecordButton>> {
        if self.ivars().used.get() { return None; }
        let record = self.ivars().record.load()?;
        if !record.menu_available() || record.ivars().menu_generation.get() != self.ivars().generation
            || !record.owner().is_some_and(|current| std::ptr::eq::<Controller>(&*current, owner))
            || record.value().as_deref() != Some(self.ivars().expected.as_str())
            || owner.recorder_menu_context(&record).as_ref() != Some(&self.ivars().context) {
            return None;
        }
        either_command(&self.ivars().expected)?;
        Some(record)
    }
}
pub struct Surface {
    pub inline_clip: Retained<InlineClip>,
    inline_document: Retained<InlineDocument>,
    pub inline_visible: Cell<bool>,
    pub menu: Retained<SurfaceBackground>,
    pub form: Retained<SurfaceBackground>,
    pub actions_button: Retained<NSButton>,
    pub alias: Retained<NSTextField>,
    pub record: Retained<RecordButton>,
    pub save: Retained<NSButton>,
    pub cancel: Retained<NSButton>,
    pub heading: Retained<NSTextField>,
    pub field_label: Retained<NSTextField>,
    pub menu_heading: Retained<NSTextField>,
    pub buttons: RefCell<Vec<Retained<NSButton>>>,
    menu_generation: Cell<u64>,
    input_buttons: RefCell<Vec<Retained<NSButton>>>,
    input_menu_token: Cell<u64>,
    pub shown_token: Cell<u64>,
    pub pending: Cell<Option<(u64, u64)>>,
    input_busy: Cell<bool>,
    saving_token: Cell<Option<u64>>,
    pub alias_initial: RefCell<String>,
    pub command: crate::native_editor::CommandFields,
    pub template: Retained<crate::native_template::Form>,
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn label(main: MainThreadMarker, frame: NSRect, size: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), main);
    field.setFrame(frame);
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.setMaximumNumberOfLines(1);
    field
}
fn button(main: MainThreadMarker, title: &str, frame: NSRect) -> Retained<NSButton> {
    let b = NSButton::new(main);
    b.setFrame(frame);
    b.setTitle(&NSString::from_str(title));
    b.setAccessibilityLabel(Some(&NSString::from_str(title)));
    b.setFont(Some(&NSFont::systemFontOfSize(13.)));
    b
}
impl Surface {
    pub(crate) fn set_alias_enabled(&self, enabled: bool) {
        // Disabling NSTextField ends its shared field-editor session and loses
        // Undo. Freeze the existing editor in place until this exact save ACK.
        if let Some(editor) = self
            .alias
            .currentEditor()
            .and_then(|e| e.downcast::<NSTextView>().ok())
        {
            editor.setEditable(enabled);
        } else {
            self.alias.setEnabled(enabled);
        }
    }
    pub fn new(main: MainThreadMarker, view: &NSView, width: f64, top_offset: f64) -> Self {
        let menu: Retained<SurfaceBackground> = unsafe {
            let v = SurfaceBackground::alloc(main).set_ivars(Cell::new(false));
            msg_send![super(v),initWithFrame:rect(width-296.,54.,280.,314.)]
        };
        let form: Retained<SurfaceBackground> = unsafe {
            let v = SurfaceBackground::alloc(main).set_ivars(Cell::new(false));
            msg_send![super(v),initWithFrame:rect(24.,84.,width-48.,382.+top_offset)]
        };
        menu.setIdentifier(Some(ns_string!("actions-menu")));
        let menu_heading = label(main, rect(12., 278., 256., 24.), 11.);
        menu.addSubview(&menu_heading);
        let mut buttons = vec![];
        for i in 0..11 {
            let b = button(main, "", rect(12., 244. - i as f64 * 30., 256., 28.));
            b.setBordered(false);
            b.setAlignment(NSTextAlignment::Left);
            b.setTag(i);
            menu.addSubview(&b);
            buttons.push(b);
        }
        let heading = label(main, rect(18., 326. + top_offset, width - 84., 30.), 20.);
        form.addSubview(&heading);
        let field_label = label(main, rect(18., 278. + top_offset, width - 84., 24.), 12.);
        form.addSubview(&field_label);
        let alias = NSTextField::textFieldWithString(ns_string!(""), main);
        alias.setFrame(rect(18., 230. + top_offset, width - 84., 34.));
        alias.setFont(Some(&NSFont::systemFontOfSize(16.)));
        alias.setAccessibilityLabel(Some(ns_string!("Aliases")));
        form.addSubview(&alias);
        let record: Retained<RecordButton> = unsafe {
            let b = RecordButton::alloc(main).set_ivars(RecordState {
                owner: RefCell::new(None),
                lease: RefCell::new(None),
                beginning: Cell::new(false),
                token: Cell::new(0),
                recording: Cell::new(false),
                parse: RefCell::new(Recorder::default()),
                value: RefCell::new(None),
                separate: Cell::new(true),
                menu_generation: Cell::new(1),
            });
            msg_send![super(b),initWithFrame:rect(18.,230.+top_offset,230.,34.)]
        };
        form.addSubview(&record);
        let cancel = button(main, "Cancel", rect(18., 24., 90., 32.));
        form.addSubview(&cancel);
        let save = button(main, "Save", rect(width - 174., 24., 90., 32.));
        form.addSubview(&save);
        let template = crate::native_template::Form::new(main, &form, width, 238. + top_offset);
        let command = crate::native_editor::CommandFields::new(main, &form, width);
        command
            .scroll
            .setFrame(rect(18., 76., width - 84., 238. + top_offset));
        command.scroll.setHidden(true);
        let actions_button = button(main, "Actions ⌘K", rect(width - 148., 12., 124., 26.));
        actions_button.setBordered(false);
        view.addSubview(&menu);
        view.addSubview(&form);
        view.addSubview(&actions_button);
        menu.setHidden(true);
        form.setHidden(true);
        let inline_clip: Retained<InlineClip> = unsafe {
            let v = InlineClip::alloc(main).set_ivars(());
            msg_send![super(v),initWithFrame:rect(0.,0.,1.,1.)]
        };
        inline_clip.setDrawsBackground(false);
        let inline_document: Retained<InlineDocument> = unsafe {
            let v = InlineDocument::alloc(main).set_ivars(());
            msg_send![super(v),initWithFrame:rect(0.,0.,1.,1.)]
        };
        inline_clip.setDocumentView(Some(&inline_document));
        inline_clip.setHidden(true);
        view.addSubview(&inline_clip);
        Self {
            inline_clip,
            inline_document,
            inline_visible: Cell::new(false),
            menu,
            form,
            actions_button,
            alias,
            record,
            save,
            cancel,
            heading,
            field_label,
            menu_heading,
            buttons: RefCell::new(buttons),
            menu_generation: Cell::new(0),
            input_buttons: RefCell::new(vec![]),
            input_menu_token: Cell::new(0),
            shown_token: Cell::new(0),
            pending: Cell::new(None),
            input_busy: Cell::new(false),
            saving_token: Cell::new(None),
            alias_initial: RefCell::new(String::new()),
            command,
            template,
        }
    }
    pub fn theme(&self,graphite:bool){for view in [&*self.form,&*self.menu]{view.ivars().set(graphite);view.setNeedsDisplay(true);}}
    pub fn layout(&self, width: f64, height: f64, management:bool) {
        let form_height = (height - if management{58.}else{0.}).max(260.);
        let form_width = width;
        self.form.setFrame(rect(0., 0., form_width, form_height));
        self.heading.setFrame(rect(22., form_height - 56., form_width - 44., 30.));
        self.field_label.setFrame(rect(22., form_height - 100., form_width - 44., 24.));
        self.save.setFrame(rect(form_width - 112., 22., 90., 32.));
        self.cancel.setFrame(rect(22., 22., 90., 32.));
        self.command.layout(rect(22., 70., form_width - 44., form_height - 138.));
        self.template.layout(rect(22., 70., form_width - 44., form_height - 138.));
        self.actions_button.setFrame(if management {
            rect(width - 148., 12., 124., 26.)
        } else {
            rect(width - 148., 17., 124., 22.)
        });
        self.actions_button.setFont(Some(&NSFont::systemFontOfSize(if management { 13. } else { 11. })));
        self.actions_button.setAlignment(if management { NSTextAlignment::Center } else { NSTextAlignment::Right });
        let mut menu = self.menu.frame();
        menu.origin.x = width - menu.size.width - 16.;
        self.menu.setFrame(menu);
    }
    pub fn position_inline(
        &self,
        editor: Option<&EditorView>,
        table: &NSTableView,
        scroll: &NSScrollView,
        parent: &NSView,
        row: Option<usize>,
    ) {
        let inline_editor = editor.is_some_and(|e| e.inline);
        let inline = inline_editor && row.is_some();
        let changed = self.inline_visible.replace(inline) != inline;
        if changed {
            self.alias
                .setFont(Some(&NSFont::systemFontOfSize(if inline {
                    12.
                } else {
                    16.
                })));
            self.record
                .setFont(Some(&NSFont::systemFontOfSize(if inline {
                    12.
                } else {
                    13.
                })));
        }
        self.inline_clip.setHidden(!inline_editor);
        if inline_editor {
            let frame = if let Some(row) = row {
                let clip = scroll.contentView();
                self.inline_clip
                    .setFrame(clip.convertRect_toView(clip.bounds(), Some(parent)));
                self.inline_document.setFrame(rect(
                    0.,
                    0.,
                    table.frame().size.width,
                    table.frame().size.height,
                ));
                self.inline_clip.scrollToPoint(clip.bounds().origin);
                let column = table.rectOfColumn(if editor.expect("inline").mode == Mode::Alias {
                    1
                } else {
                    2
                });
                let row = table.rectOfRow(row as isize);
                rect(
                    column.origin.x + 2.,
                    row.origin.y + 4.,
                    column.size.width - 4.,
                    30.,
                )
            } else {
                // Recover the same input in the ordinary form area without
                // detaching its native field editor or marked text.
                let frame = self.form.convertRect_toView(
                    rect(
                        18.,
                        self.form.frame().size.height - 152.,
                        self.form.frame().size.width - 36.,
                        34.,
                    ),
                    Some(parent),
                );
                self.inline_clip.setFrame(frame);
                self.inline_document
                    .setFrame(rect(0., 0., frame.size.width, frame.size.height));
                self.inline_clip.scrollToPoint(NSPoint::new(0., 0.));
                rect(0., 0., frame.size.width, frame.size.height)
            };
            if unsafe { self.alias.superview() }
                .is_none_or(|v| !std::ptr::eq::<NSView>(&*v, &**self.inline_document))
            {
                self.inline_document.addSubview(&self.alias);
                self.inline_document.addSubview(&self.record);
            }
            self.alias.setFrame(frame);
            self.record.setFrame(frame);
        } else {
            // CommandFields owns both the recorder's parent and its document
            // position. Generic bindings forms use the form coordinates below.
            if editor.is_some_and(|e| e.mode == Mode::Edit) {
                return;
            }
            if unsafe { self.alias.superview() }
                .is_none_or(|v| !std::ptr::eq::<NSView>(&*v, &**self.form))
            {
                self.form.addSubview(&self.alias);
                self.form.addSubview(&self.record);
            }
            self.alias.setFrame(rect(
                18.,
                self.form.frame().size.height - 152.,
                self.form.frame().size.width - 36.,
                34.,
            ));
            self.record
                .setFrame(rect(18., self.form.frame().size.height - 152., 230., 34.));
        }
    }
    fn clear_input_buttons(&self) {
        for b in self.input_buttons.borrow_mut().drain(..) {
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
            b.removeFromSuperview();
        }
        self.input_menu_token.set(0);
    }
    pub fn menu_buttons(&self) -> Vec<Retained<NSButton>> {
        self.buttons
            .borrow()
            .iter()
            .chain(self.input_buttons.borrow().iter())
            .cloned()
            .collect()
    }
    pub fn input_button_current(&self, sender: &NSButton, token: u64) -> bool {
        self.input_menu_token.get() == token
            && !self.menu.isHidden()
            && self
                .input_buttons
                .borrow()
                .iter()
                .any(|b| std::ptr::eq::<NSButton>(&**b, sender))
    }
    pub fn action_button_current(&self, sender: &NSButton) -> bool {
        !self.menu.isHidden()
            && self.buttons.borrow().iter().any(|b| std::ptr::eq::<NSButton>(&**b, sender))
    }
    pub fn bind(&self, owner: &Controller) {
        self.command.bind(owner);
        self.template.bind(owner);
        self.record.bind_owner(owner);
        for b in self.buttons.borrow().iter() {
            unsafe {
                b.setTarget(Some(owner));
                b.setAction(Some(objc2::sel!(chooseAction:)));
            }
        }
        for (b, selector) in [
            (&*self.actions_button, objc2::sel!(toggleActions:)),
            (&*self.cancel, objc2::sel!(cancelEditor:)),
            (&*self.save, objc2::sel!(saveEditor:)),
            (&*self.record, objc2::sel!(recordShortcut:)),
        ] {
            unsafe {
                b.setTarget(Some(owner));
                b.setAction(Some(selector));
            }
        }
    }
    pub fn cleanup(&self) {
        self.clear_input_buttons();
        self.command.cleanup();
        self.template.cleanup();
        self.stop();
        self.record.clear_owner();
        for b in self.buttons.borrow().iter().map(|b| &**b).chain([
            &*self.actions_button,
            &*self.cancel,
            &*self.save,
            &*self.record,
        ]) {
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
        }
        unsafe {
            self.alias.setDelegate(None);
        }
    }
    pub fn stop(&self) {
        self.record.stop();
    }
    pub fn apply(&self, data: &SurfaceData, panel: &NSWindow) {
        let run_inline = data.editor.as_ref().is_some_and(|e| e.run_inline);
        if data
            .editor
            .as_ref()
            .is_none_or(|e| e.template_fields.is_none() || e.run_inline)
        {
            if self
                .template
                .preserves(data.editor.as_ref().and_then(|e| e.suspended_input_token))
            {
                self.template.hide();
            } else {
                self.template.retire();
            }
        }
        self.actions_button
            .setHidden(data.editor.is_some() && !run_inline);
        self.actions_button
            .setEnabled(!run_inline || (!data.execution_running && self.pending.get().is_none()));
        self.menu.setHidden(
            !data.actions_open || !data.visible || (data.editor.is_some() && !run_inline),
        );
        self.menu_heading
            .setStringValue(&NSString::from_str(&data.title));
        let has_heading=!data.title.is_empty();self.menu_heading.setHidden(!has_heading);
        let height = (if has_heading{74.}else{20.}) + data.items.len() as f64 * 30.;
        self.menu.setFrameSize(NSSize::new(280., height));
        if let Some(parent) = unsafe { self.menu.superview() } {
            let mut frame = self.menu.frame();
            frame.origin.y = 52_f64.min(parent.bounds().size.height - height - 8.).max(8.);
            self.menu.setFrame(frame);
        }
        self.menu_heading
            .setFrame(rect(12., height - 36., 256., 24.));
        if !data.actions_open || !data.visible {
            self.menu_generation.set(0);
        } else if self.menu_generation.get() != data.menu_token || self.buttons.borrow().len() != data.items.len() {
            for old in self.buttons.borrow_mut().drain(..) {
                unsafe {
                    old.setTarget(None);
                    old.setAction(None);
                }
                old.removeFromSuperview();
            }
            for i in 0..data.items.len() {
                let b = button(self.form.mtm(), "", rect(0., 0., 256., 28.));
                b.setBordered(false);
                b.setAlignment(NSTextAlignment::Left);
                b.setTag(i as isize);
                if let Some(owner) = self.record.owner() {
                    unsafe {
                        b.setTarget(Some(&owner));
                        b.setAction(Some(objc2::sel!(chooseAction:)));
                    }
                }
                self.menu.addSubview(&b);
                self.buttons.borrow_mut().push(b);
            }
            self.menu_generation.set(data.menu_token);
        }
        for (i, button) in self.buttons.borrow().iter().enumerate() {
            button.setFrame(rect(12., height - (if has_heading{70.}else{38.}) - i as f64 * 30., 256., 28.));
            let item = data.items.get(i);
            button.setHidden(run_inline || item.is_none());
            if let Some(item) = item {
                let title = if let Some(key) = item.shortcut {
                    format!("{}   {}", item.title, presentation::shortcut(key).0)
                } else {
                    item.title.clone()
                };
                button.setTitle(&NSString::from_str(&title));
                let symbol=match item.id{"clipboard.paste"=>"doc.on.clipboard","clipboard.copy"|"copy-template"=>"doc.on.doc","clipboard.delete"|"remove"=>"trash","clipboard.settings"|"settings"=>"gearshape","edit"=>"pencil","set-alias"=>"text.cursor","set-hotkey"=>"keyboard","disable"=>"pause.circle","open"=>"return",_=>"command"};
                button.setImage(NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol),None).as_deref());button.setImagePosition(NSCellImagePosition::ImageLeft);
                button.setAccessibilityLabel(Some(&NSString::from_str(&item.title)));
                button.setAccessibilityRole(Some(unsafe{NSAccessibilityButtonRole}));button.setAccessibilitySelected(i==data.action_selection);
                button.highlight(i == data.action_selection);
                button.setState(if i == data.action_selection {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
        }
        if run_inline && data.actions_open && data.visible {
            if self.input_menu_token.get() != data.menu_token {
                self.clear_input_buttons();
                if let Some(owner) = self.record.owner() {
                    for (i, item) in data.items.iter().enumerate() {
                        let b = button(
                            self.form.mtm(),
                            &item.title,
                            rect(12., height - 70. - i as f64 * 30., 256., 28.),
                        );
                        b.setBordered(false);
                        b.setAlignment(NSTextAlignment::Left);
                        b.setTag(i as isize);
                        unsafe {
                            b.setTarget(Some(&owner));
                            b.setAction(Some(objc2::sel!(chooseAction:)));
                        }
                        self.menu.addSubview(&b);
                        self.input_buttons.borrow_mut().push(b);
                    }
                    self.input_menu_token.set(data.menu_token);
                }
            }
            for (i, b) in self.input_buttons.borrow().iter().enumerate() {
                b.highlight(i == data.action_selection);
            }
        } else {
            self.clear_input_buttons();
        }
        if let Some(editor) = data.editor.as_ref().filter(|_| data.visible) {
            self.form.setHidden(self.inline_visible.get());
            self.menu
                .setHidden(!editor.run_inline || !data.actions_open || !data.visible);
            let acknowledged = self
                .pending
                .get()
                .is_some_and(|(token, attempt)| token == editor.token && editor.attempt >= attempt);
            if acknowledged
                || self
                    .pending
                    .get()
                    .is_some_and(|(token, _)| token != editor.token)
            {
                self.pending.set(None);
            }
            let pending = self.pending.get().is_some() || editor.saving;
            let save_finished = self
                .saving_token
                .replace(editor.saving.then_some(editor.token))
                == Some(editor.token)
                && !editor.saving;
            if let Some(fields) = &editor.template_fields {
                self.template.show(
                    editor.token,
                    fields.clone(),
                    !(pending || data.execution_running),
                );
            }
            if editor.run_inline {
                self.actions_button
                    .setEnabled(!data.execution_running && !pending);
                self.form.setHidden(true);
                self.command.scroll.setHidden(true);
                self.record.stop();
                return;
            }

            let input_finished = self
                .input_busy
                .replace(editor.mode == Mode::Input && data.execution_running)
                && !data.execution_running;
            self.command
                .set_enabled(!(pending || editor.mode == Mode::Input && data.execution_running));
            self.command.scroll.setHidden(
                editor.template_fields.is_some()
                    || ![Mode::Edit, Mode::Input].contains(&editor.mode),
            );
            self.field_label.setHidden(
                ([Mode::Edit, Mode::Input].contains(&editor.mode)&&editor.command["captureInput"]!=true) || self.inline_visible.get(),
            );
            self.field_label.setFrame(if editor.mode==Mode::Input&&editor.command["captureInput"]==true{rect(18.,58.,self.form.bounds().size.width-36.,18.)}else{rect(22.,self.form.bounds().size.height-100.,self.form.bounds().size.width-44.,24.)});
            self.set_alias_enabled(!pending);
            self.record.setEnabled(!pending);
            self.save.setEnabled(
                !(pending
                    || editor.mode == Mode::Input && (data.execution_running || editor.task_date.as_ref().is_some_and(|d|!d.is_valid()))
                    || self.template.choice_open()
                    || self.command.app_list.is_open()),
            );
            self.heading
                .setStringValue(&NSString::from_str(&if editor.mode == Mode::Remove {
                    "Remove this command?".into()
                } else {
                    editor.title.clone()
                }));
            let field = match editor.mode {
                Mode::Alias => "Alias".to_string(),
                Mode::Hotkey => "Hotkey".to_string(),
                Mode::Remove => editor.title.clone(),
                Mode::Input if editor.command["captureInput"]==true=>"⇧↵ New line".to_string(),
                Mode::Edit | Mode::Input => "".to_string(),
            };
            self.field_label.setStringValue(&NSString::from_str(&field));
            self.alias
                .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "Aliases for {}",
                    editor.title
                ))));
            self.alias.setHidden(editor.mode != Mode::Alias);
            self.record
                .setHidden(![Mode::Hotkey, Mode::Edit].contains(&editor.mode));
            self.save.setHidden(editor.mode == Mode::Hotkey);
            self.save.setTitle(if editor.mode==Mode::Input&&editor.command["captureInput"]==true {
                ns_string!("Save ↵")
            }else if editor.mode == Mode::Input {
                ns_string!("Run ⌘↵")
            } else if editor.mode == Mode::Remove {
                ns_string!("Remove")
            } else {
                ns_string!("Save")
            });
            self.save.setAccessibilityLabel(Some(&self.save.title()));
            if [Mode::Edit,Mode::Input].contains(&editor.mode){
                self.cancel.setTitle(ns_string!("Back"));self.cancel.setAccessibilityLabel(Some(ns_string!("Back")));self.cancel.setFrame(rect(18.,self.form.frame().size.height-56.,70.,30.));
                self.heading.setFrame(rect(100.,self.form.frame().size.height-56.,self.form.frame().size.width-118.,30.));self.heading.setFont(Some(&NSFont::systemFontOfSize_weight(16.,unsafe{NSFontWeightSemibold})));
            }else{self.cancel.setTitle(ns_string!("Cancel"));self.cancel.setAccessibilityLabel(Some(ns_string!("Cancel")));}
            self.record.ivars().separate.set(data.separate);
            if self.shown_token.replace(editor.token) != editor.token {
                if let Some(manager) = panel.undoManager() {
                    manager.removeAllActions();
                }
                self.pending.set(None);
                self.record.stop();
                *self.alias_initial.borrow_mut() = editor.aliases.clone();
                self.alias
                    .setStringValue(&NSString::from_str(&editor.aliases));
                self.record.configure(editor.hotkey.clone());
                if editor.template_fields.is_some() {
                    self.command.clear();
                    self.template.focus(0);
                } else if editor.mode == Mode::Input {
                    self.command.load_input(
                        self.form.mtm(),
                        if editor.task_date.is_some() { "Datum" } else { editor.command["inputPrompt"].as_str().unwrap_or("Input") },
                        editor.command["input"].as_str().unwrap_or(""),
                        editor.command["captureInput"]==true,
                    );
                    panel.makeFirstResponder(Some(&self.command.text));
                } else if editor.mode == Mode::Edit {
                    self.command
                        .load(self.form.mtm(), &editor.command, &self.record);
                    if let Some(field) = self.command.fields.borrow().first() {
                        panel.makeFirstResponder(Some(&field.input));
                    }
                } else if !editor.inline {
                    self.record
                        .setFrame(rect(18., self.form.frame().size.height - 152., 230., 34.));
                    self.form.addSubview(&self.record);
                }
                if editor.mode == Mode::Alias {
                    panel.makeFirstResponder(Some(&self.alias));
                } else if editor.mode == Mode::Hotkey && (panel.isKeyWindow() || data.allow_hidden)
                {
                    self.record.begin();
                } else if editor.mode == Mode::Remove {
                    panel.makeFirstResponder(Some(&self.cancel));
                }
            } else if [Mode::Edit, Mode::Input].contains(&editor.mode)
                && !pending
                && (acknowledged || save_finished || (editor.mode == Mode::Input && input_finished))
                && (panel.isKeyWindow() || data.allow_hidden)
            {
                if editor.template_fields.is_some() {
                    self.template.restore_focus();
                } else if editor.mode == Mode::Input {
                    panel.makeFirstResponder(Some(&self.command.text));
                } else {
                    self.command.restore_focus(panel);
                }
            } else if !self.record.recording() && editor.mode != Mode::Edit {
                self.record.configure(editor.hotkey.clone());
                if !pending
                    && (acknowledged || save_finished)
                    && editor.mode == Mode::Alias
                    && (panel.isKeyWindow() || data.allow_hidden)
                {
                    if self.alias.currentEditor().is_none() {
                        panel.makeFirstResponder(Some(&self.alias));
                    }
                    if let Some(text) = self
                        .alias
                        .currentEditor()
                        .and_then(|e| e.downcast::<NSTextView>().ok())
                        .filter(|_| !editor.error.is_empty())
                    {
                        text.setSelectedRange(objc2_foundation::NSRange::new(
                            0,
                            text.string().length(),
                        ));
                    }
                }
            }
            self.command
                .update_apps(data.app_choices.clone(), data.app_choices_pending);
            self.command.update_date_preview(editor.task_date.as_ref());
        } else {
            self.form.setHidden(true);
            if self.shown_token.get() != 0 {
                self.command.clear();
                self.alias.setStringValue(ns_string!(""));
                self.alias_initial.borrow_mut().clear();
                if let Some(manager) = panel.undoManager() {
                    manager.removeAllActions();
                }
            }
            self.command.scroll.setHidden(true);
            self.shown_token.set(0);
            self.input_busy.set(false);
            self.saving_token.set(None);
            self.pending.set(None);
            self.record.stop();
        }
    }
}

#[cfg(test)]
mod record_menu_tests {
    use super::either_command;
    #[test]
    fn either_command_preserves_key_and_other_modifiers() {
        assert_eq!(either_command("right-cmd+a").as_deref(),Some("cmd+a"));
        assert_eq!(either_command("alt+left-cmd+space").as_deref(),Some("alt+cmd+space"));
        assert_eq!(either_command("SHIFT+RIGHT-CMD+a").as_deref(),Some("cmd+shift+a"));
    }
    #[test]
    fn either_command_rejects_unsided_invalid_or_combined_shortcuts() {
        for value in ["cmd+a","ctrl+right","a","left-cmd+right-cmd+a","cmd+left-cmd+a","right-cmd+right-cmd+a",""] {
            assert!(either_command(value).is_none(),"{value}");
        }
    }
}
