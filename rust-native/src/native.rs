//! AppKit UI written in Rust. Current entry points are isolated fixtures only.
use crate::native_model::Model;
use objc2::rc::{autoreleasepool, Retained, Weak};
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    ns_string, NSIndexSet, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString,
};
use ruycast_native::{
    presentation,
    session::{fixture, Session},
};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};

define_class!(
    // SAFETY: NSView drawing remains on AppKit's main thread.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars=Cell<bool>]
    struct PaletteContent;
    impl PaletteContent {
        #[unsafe(method(drawRect:))]
        fn draw(&self, dirty: NSRect) {
            let _timing = ruycast_native::performance_trace::shared().scope(ruycast_native::performance_trace::Stage::HostDraw);
            crate::native_settings::color(crate::native_settings::palette(self,self.ivars().get()).background).setFill();
            NSRectFill(dirty);
        }
    }
);

struct PanelState {
    failed: Cell<bool>,
    owner: RefCell<Option<Weak<Controller>>>,
    management: bool,
    ordering_allowed: bool,
    blocked_orders: Cell<u64>,
    #[cfg(feature = "test-tools")]
    hidden_repairs: Cell<u64>,
    #[cfg(feature = "test-tools")]
    last_hidden_repair: RefCell<Option<serde_json::Value>>,
}

// Main-thread-owned subscriptions. Taking this value out of the controller
// precedes removal, so a callback during removal cannot observe a live lease.
struct OutsideMonitors {
    generation: u64,
    binding: (u64, u64),
    global: Option<Retained<objc2::runtime::AnyObject>>,
    local: Option<Retained<objc2::runtime::AnyObject>>,
}
impl Drop for OutsideMonitors {
    fn drop(&mut self) {
        if let Some(token) = self.global.take() { unsafe { NSEvent::removeMonitor(&token); } }
        if let Some(token) = self.local.take() { unsafe { NSEvent::removeMonitor(&token); } }
    }
}
define_class!(
    // SAFETY: NSPanel subclass with no additional ivars or Drop implementation.
    #[unsafe(super = NSPanel)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PanelState]
    struct PalettePanel;
    impl PalettePanel {
        #[unsafe(method(orderBack:))]
        fn order_back(&self,sender:Option<&objc2::runtime::AnyObject>){
            if !self.ivars().ordering_allowed {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderBack:sender];}
        }
        #[unsafe(method(orderFront:))]
        fn order_front(&self,sender:Option<&objc2::runtime::AnyObject>){
            if !self.ivars().ordering_allowed {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderFront:sender];}
        }
        #[unsafe(method(makeKeyAndOrderFront:))]
        fn key_front(&self,sender:Option<&objc2::runtime::AnyObject>){
            if !self.ivars().ordering_allowed {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),makeKeyAndOrderFront:sender];}
        }
        #[unsafe(method(orderFrontRegardless))]
        fn front_regardless(&self){
            if !self.ivars().ordering_allowed {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderFrontRegardless];}
        }
        #[unsafe(method(makeKeyWindow))]
        fn make_key(&self){
            if !self.ivars().ordering_allowed {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),makeKeyWindow];}
        }
        #[unsafe(method(orderWindow:relativeTo:))]
        fn order(&self,place:NSWindowOrderingMode,relative:objc2_foundation::NSInteger){
            if !self.ivars().ordering_allowed && place!=NSWindowOrderingMode::Out {self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);return;}
            unsafe{let _:()=msg_send![super(self),orderWindow:place,relativeTo:relative];}
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn edit_key(&self, event: &NSEvent) -> bool {
            if self.ivars().failed.get() { false } else { match catch_unwind(AssertUnwindSafe(|| {
                let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
                if owner.is_some_and(|owner|owner.route_key(event)){return true;}
                if crate::native_edit::perform(event, self.firstResponder().as_deref()) { true }
                else { unsafe { msg_send![super(self), performKeyEquivalent: event] } }
            })) {
                Ok(handled) => handled,
                Err(_) => { self.ivars().failed.set(true); self.orderOut(None); false }
            }}
        }
        #[unsafe(method(keyDown:))]
        fn route_down(&self,event:&NSEvent){let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if !owner.is_some_and(|owner|owner.route_key(event)){unsafe{let _:()=msg_send![super(self),keyDown:event];}}}
        #[unsafe(method(orderOut:))]
        fn hide(&self,sender:Option<&NSObject>){
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
            if let Some(owner) = owner.as_ref() { owner.stop_outside_monitors(); }
            if let Some(owner) = owner.as_ref() { owner.cancel_performance_input(); }
            if let Some(owner)=owner{owner.ivars().dashboard.retire_dialog();owner.ivars().settings.retire_dialog();owner.ivars().settings.stop_recording();owner.ivars().surface.stop();}
            unsafe{let _:()=msg_send![super(self),orderOut:sender];}
        }
        #[unsafe(method(resignKeyWindow))]
        fn resigned(&self){
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
            if let Some(owner)=owner{owner.ivars().inline_autosave_allowed.set(false);owner.ivars().settings.stop_recording();owner.ivars().surface.stop();}
            unsafe{let _:()=msg_send![super(self),resignKeyWindow];}
        }
        #[unsafe(method(becomeKeyWindow))]
        fn became_key(&self){if self.ivars().ordering_allowed{unsafe{let _:()=msg_send![super(self),becomeKeyWindow];}}else{self.ivars().blocked_orders.set(self.ivars().blocked_orders.get()+1);}let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);if let Some(owner)=owner{owner.ivars().inline_autosave_allowed.set(true);}}
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool { self.ivars().ordering_allowed }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {self.ivars().management && self.ivars().ordering_allowed}
        #[unsafe(method(close))]
        fn close_window(&self){
            let owner=self.ivars().owner.borrow().as_ref().and_then(Weak::load);
            if let Some(owner)=owner{owner.guard_callback(||{owner.ivars().session.borrow_mut().hide_palette();owner.ivars().surface.stop();owner.reconcile();});}
            unsafe{let _:()=msg_send![super(self),close];}
        }
    }
);

define_class!(
    // SAFETY: View callbacks are confined to AppKit's main thread.
    #[unsafe(super = NSTableRowView)]
    #[thread_kind = MainThreadOnly]
    struct ResultRow;
    impl ResultRow {
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self, _: NSRect) {
            let dark=self.effectiveAppearance().name().to_string().to_lowercase().contains("dark");
            NSColor::labelColor().colorWithAlphaComponent(if dark{0.085}else{0.065}).setFill();
            let b=self.bounds();NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(b.origin.x+2.,b.origin.y+1.,(b.size.width-4.).max(1.),(b.size.height-2.).max(1.)),8.,8.).fill();
        }
    }
);

impl PalettePanel {
    // Fixture isolation, not a visibility getter: preserve every real transition.
    // The offscreen panel remains unable to become key/main. No editor stop,
    // reconciliation or callbacks are initiated by this policy boundary.
    #[cfg(feature = "test-tools")]
    fn enforce_hidden_fixture(&self) {
        if self.ivars().ordering_allowed || !self.isVisible() {
            return;
        }
        let frame = self.frame();
        self.ivars()
            .hidden_repairs
            .set(self.ivars().hidden_repairs.get() + 1);
        *self.ivars().last_hidden_repair.borrow_mut() = Some(serde_json::json!({
            "phase":"apply-view", "visibleBefore":true, "keyBefore":self.isKeyWindow(),
            "mainBefore":self.isMainWindow(),"x":frame.origin.x,"y":frame.origin.y,
            "width":frame.size.width,"height":frame.size.height
        }));
        unsafe {
            let _: () = msg_send![super(self),orderOut:Option::<&NSObject>::None];
        }
    }
    #[cfg(feature = "test-tools")]
    fn fixture_force_visible(&self) -> Result<(), String> {
        let frame = self.frame();
        if self.ivars().ordering_allowed || frame.origin.x > -10000. || frame.origin.y > -10000. {
            return Err("Only an own offscreen hidden fixture may exercise ordering.".into());
        }
        unsafe {
            let _: () =
                msg_send![super(self),orderWindow:NSWindowOrderingMode::Above,relativeTo:0isize];
        }
        Ok(())
    }
}

define_class!(
    // SAFETY: Main-thread table; local mouse events resolve the current row
    // before sending the target action. Empty space has no action.
    #[unsafe(super = NSTableView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<bool>]
    struct ResultTable;
    impl ResultTable {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool { false }
        #[unsafe(method(mouseDown:))]
        fn clicked(&self, event: &NSEvent) {
            if self.ivars().get() { return; }
            let result = catch_unwind(AssertUnwindSafe(|| {
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                let row = self.rowAtPoint(point);
                if row < 0 { return; }
                let owner=self.target().and_then(|owner|owner.downcast::<Controller>().ok());
                if let Some(owner)=owner{if owner.ivars().session.borrow().script_result().is_some() && owner.ivars().panel.firstResponder().and_then(|r|r.downcast_ref::<NSTextView>().map(|t|unsafe{msg_send![t,hasMarkedText]})).unwrap_or(false){return;}let group=owner.ivars().session.borrow().row(row as usize).is_some_and(|r|r.is_group);if group{let name=owner.ivars().session.borrow().row(row as usize).map(|r|r.id.to_string());if let Some(name)=name{owner.library_navigate("group",&name);}return;}}
                self.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false);
                if let Some(action) = self.action() {
                    unsafe { self.sendAction_to(Some(action), self.target().as_deref()); }
                }
            }));
            if result.is_err() { self.ivars().set(true); if let Some(window) = self.window() { window.orderOut(None); } }
        }
    }
);

#[cfg(feature="test-tools")]
type ClipboardFixture=std::rc::Rc<dyn Fn(&serde_json::Value)->Result<serde_json::Value,String>>;
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RecordMenuContext {
    Settings {
        index: usize,
        revision: u64,
        epoch: u64,
        dismissal: u64,
        navigation_revision: u64,
    },
    Inline {
        token: u64,
        id: String,
        mode: ruycast_native::actions::Mode,
        catalog_revision: u64,
        config_revision: u64,
        epoch: u64,
        dismissal: u64,
    },
}
#[derive(Clone,PartialEq)]
struct SnippetHotkeyBinding {
    target:ruycast_native::actions::Target,
    dismissal:u64,
    configuration:u64,
    query:String,
    launcher:Option<ruycast_native::session::LauncherScope>,
    system:Option<ruycast_native::system_search::Scope>,
}
struct SnippetHotkeyControl {
    button:Retained<NSButton>,
    binding:SnippetHotkeyBinding,
}
pub(crate) struct State {
    outside: RefCell<Option<OutsideMonitors>>,
    outside_generation: Cell<u64>,
    outside_stopped: Cell<bool>,
    monitor:RefCell<Option<crate::native_monitor::Monitor>>,
    process_controls:crate::native_os_controls::Controls,
    failed: Cell<bool>,
    toast: RefCell<Option<crate::native_notice::Service>>,
    peer: RefCell<Option<Weak<Controller>>>,
    toolbar: Option<crate::native_library::Toolbar>,
    clipboard_preferences:crate::native_clipboard_preferences::Preferences,
    clipboard_panel: crate::native_clipboard::Panel,
    dashboard: crate::native_dashboard::Dashboard,
    settings: crate::native_settings::Settings,
    menu_bar:crate::native_menu_bar::Service,
    handled_dashboard_request: Cell<u64>,
    table_height: f64,
    surface: crate::native_surface::Surface,
    result_panel: crate::native_result::ResultPanel,
    session: RefCell<Model>,
    panel: Retained<PalettePanel>,
    input: Retained<NSTextField>,
    table: Retained<ResultTable>,
    heading: Retained<NSTextField>,
    empty: Retained<NSTextField>,
    footer: Retained<NSTextField>,
    snippet_hotkey:RefCell<Option<SnippetHotkeyControl>>,
    #[cfg(feature="test-tools")]
    stale_snippet_hotkey:RefCell<Option<Retained<NSButton>>>,
    #[cfg(feature = "test-tools")]
    held_record_menu: RefCell<Option<Retained<NSMenu>>>,
    launcher_back: Retained<NSButton>,
    notice: Retained<NSTextField>,
    #[cfg(feature = "test-tools")]
    stale_text: RefCell<Option<Retained<crate::native_editor::CommandText>>>,
    #[cfg(feature = "test-tools")]
    stale_enabled: RefCell<Option<Retained<crate::native_library::EnabledButton>>>,
    #[cfg(feature = "test-tools")]
    stale_input_menu: RefCell<Option<Retained<NSButton>>>,
    timers: RefCell<Vec<crate::main_loop::Timer>>,
    #[cfg(feature = "test-tools")]
    clipboard_fixture:RefCell<Option<ClipboardFixture>>,
    #[cfg(feature = "test-tools")]
    source_failure: Cell<bool>,
    #[cfg(feature = "test-tools")]
    shutdown_witness: RefCell<Option<std::rc::Rc<ShutdownWitness>>>,
    probe_output: RefCell<Option<std::path::PathBuf>>,
    scroll: Retained<NSScrollView>,
    reconciling: Cell<bool>,
    restore_result_focus: Cell<bool>,
    keyboard: RefCell<Option<ruycast_native::keyboard_service::Client>>,
    run_inline_token: Cell<u64>,
    run_inline_disabled: Cell<bool>,
    run_inline_selection: Cell<Option<objc2_foundation::NSRange>>,
    pending_input_menu: Cell<Option<u64>>,
    inline_autosave_allowed: Cell<bool>,
    inline_autosave: RefCell<Option<crate::main_loop::Timer>>,
    rendered_revision: Cell<u64>,
    rendered_bindings: Cell<Option<(u64, u64, u64)>>,
    rendered_title_width: Cell<Option<f64>>,
    #[cfg(feature = "test-tools")]
    defer_input_child: Cell<bool>,
    #[cfg(feature = "test-tools")]
    recorder_acquire_starts: Cell<u64>,
    #[cfg(feature = "test-tools")]
    key_event_handled: Cell<Option<bool>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; all AppKit ivars are
    // main-thread confined. The owner outlives the weak NSTextField delegate.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    pub(crate) struct Controller;
    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSControlTextEditingDelegate for Controller {
        #[unsafe(method(control:textShouldBeginEditing:))]
        fn should_begin_alias(&self, control: &NSControl, _: &NSText) -> bool {
            self.guard_callback(|| {
                let state = self.ivars();
                !std::ptr::eq(control, &*state.surface.alias as &NSControl)
                    || (state.surface.pending.get().is_none()
                        && !state.session.borrow().editor_saving())
            }).unwrap_or(false)
        }

        #[unsafe(method(controlTextDidBeginEditing:))]
        fn began(&self, _: &NSNotification) {
            self.guard_callback(|| {
                for field in [&*self.ivars().input,&*self.ivars().surface.alias].into_iter().chain(self.ivars().surface.command.fields.borrow().iter().map(|f|&*f.input)){
                if let Some(editor) = field.currentEditor() {
                    if let Some(text) = editor.downcast_ref::<NSTextView>() { text.setAllowsUndo(true); }
                }}
            });
        }
        #[unsafe(method(control:textShouldEndEditing:))]
        fn should_end_inline(&self,control:&NSControl,text:&NSText)->bool{
            self.guard_callback(||{
                let state=self.ivars();let inline=state.session.borrow().surface().editor.is_some_and(|e|e.inline);
                if !state.reconciling.get() && inline && state.session.borrow().visible() && std::ptr::eq(control,&*state.surface.alias as &NSControl){
                    return !text.downcast_ref::<NSTextView>().is_some_and(|t|unsafe{msg_send![t,hasMarkedText]});
                }
                true
            }).unwrap_or(false)
        }
        #[unsafe(method(controlTextDidEndEditing:))]
        fn ended(&self,note:&NSNotification){self.guard_callback(||{
            let same=note.object().is_some_and(|o|std::ptr::eq::<objc2::runtime::AnyObject>(&*o,&*self.ivars().surface.alias as *const _ as *const _));
            if same{self.inline_focus_ended();}
        });}
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, note: &NSNotification) {
            self.guard_callback(|| {
                if self.ivars().reconciling.get(){return;}
                if self.ivars().session.borrow().run_inline(){
                    let same = note.object().is_some_and(|o|std::ptr::eq::<objc2::runtime::AnyObject>(&*o,&*self.ivars().input as *const _ as *const _));
                    if same {
                        let field = self.ivars().input.currentEditor();
                        let text = field.as_ref().and_then(|e|e.downcast_ref::<NSTextView>());
                        if !text.is_some_and(|t|unsafe{msg_send![t,hasMarkedText]}) {
                            let value = text.map(|t|t.string()).unwrap_or_else(||self.ivars().input.stringValue());
                            self.ivars().session.borrow().preview_input(value.to_string());
                        }
                    }
                    return;
                }
                let editor=self.ivars().session.borrow().surface().editor;
                if let Some(editor)=editor{
                    if editor.run_inline { return; }
                    let query=note.object().is_some_and(|o|std::ptr::eq::<objc2::runtime::AnyObject>(&*o,&*self.ivars().input as *const _ as *const _));
                    if !editor.inline || !query{return;}
                    if editor.mode==ruycast_native::actions::Mode::Alias{self.save_editor();}else{self.cancel_editor();}
                }
                let field_editor=self.ivars().input.currentEditor();
                let text=field_editor.as_ref().and_then(|e|e.downcast_ref::<NSTextView>());
                let marked=text.is_some_and(|t|unsafe{msg_send![t,hasMarkedText]});
                let value=text.map(|t|t.string()).unwrap_or_else(||self.ivars().input.stringValue());
                let own_field = note.object().is_some_and(|o|std::ptr::eq::<objc2::runtime::AnyObject>(&*o,&*self.ivars().input as *const _ as *const _));
                if own_field { ruycast_native::performance_trace::shared().input_changed(self.performance_field()); }
                {
                    let _timing = own_field.then(||ruycast_native::performance_trace::shared().scope(ruycast_native::performance_trace::Stage::InputModel));
                    self.ivars().session.borrow_mut().set_query_composing(&value.to_string(),marked);
                }
                self.reconcile();
            });
        }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn command(&self, control: &NSControl, text: &NSTextView, selector: Sel) -> bool {
            self.guard_callback(|| {
                // Query the native input client rather than guessing IME state.
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                if self.ivars().session.borrow().script_result().is_some(){
                    if !marked && !self.ivars().session.borrow().script_result().is_some_and(|r|r.detail_visible) && self.ivars().session.borrow().query()!=self.ivars().input.stringValue().to_string(){self.ivars().session.borrow_mut().set_query(&self.ivars().input.stringValue().to_string());}
                    let handled=self.ivars().session.borrow_mut().key(&selector.name().to_string_lossy(),marked);if handled{self.reconcile();}return handled;
                }
                if std::ptr::eq(control,&*self.ivars().surface.alias as &NSControl){
                    if marked{return false;}
                    if selector==sel!(insertNewline:){self.save_editor();return true;}
                    if selector==sel!(cancelOperation:){self.cancel_editor();return true;}
                    return false;
                }
                if self.ivars().session.borrow().run_inline() {
                    if marked{return false;}
                    if self.ivars().session.borrow().input_menu_open() { let handled=self.ivars().session.borrow_mut().key(&selector.name().to_string_lossy(),false); return handled; }
                    if selector==sel!(insertNewline:){self.inline_input_control(ruycast_native::connected::InputOperation::Run);return true;}
                    if selector==sel!(insertLineBreak:) || selector==sel!(insertNewlineIgnoringFieldEditor:){self.inline_input_control(ruycast_native::connected::InputOperation::Expand);return true;}

                    if selector==sel!(cancelOperation:){if !self.cancel_pending_input_menu(){self.cancel_editor();}return true;}
                    if selector==sel!(moveDown:) || selector==sel!(moveUp:) || selector==sel!(insertTab:){return true;}
                    return false;
                }
                if self.ivars().session.borrow().surface().editor.is_some(){if !marked && selector==sel!(cancelOperation:){self.cancel_editor();return true;}return false;}
                if !marked && self.ivars().session.borrow().query()!=self.ivars().input.stringValue().to_string() {
                    self.ivars().session.borrow_mut().set_query(&self.ivars().input.stringValue().to_string());
                }
                let handled = self
                    .ivars()
                    .session
                    .borrow_mut()
                    .key(&selector.name().to_string_lossy(), marked);
                if handled {
                    self.reconcile();
                    if !self.ivars().session.borrow().visible() {
                        self.ivars().panel.orderOut(None);
                    }
                }
                handled
            })
            .unwrap_or(false)
        }
    }
    unsafe impl NSTextFieldDelegate for Controller {}
    unsafe impl NSApplicationDelegate for Controller{
        #[unsafe(method(applicationDidBecomeActive:))]
        fn app_became_active(&self,_:&NSNotification){self.guard_callback(||{self.ivars().menu_bar.refresh(self);self.reconcile();});}
        #[unsafe(method(applicationDidResignActive:))]
        fn app_resigned(&self,_:&NSNotification){self.guard_callback(||{
            self.ivars().inline_autosave_allowed.set(false);self.ivars().settings.stop_recording();self.ivars().surface.stop();
            let peer=self.ivars().peer.borrow().as_ref().and_then(Weak::load);
            if let Some(peer)=peer{peer.ivars().inline_autosave_allowed.set(false);peer.ivars().settings.stop_recording();peer.ivars().surface.stop();}
        });}
    }

    unsafe impl NSTableViewDataSource for Controller {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn row_count(&self, _: &NSTableView) -> isize {
            self.guard_callback(|| self.ivars().session.borrow().row_count() as isize).unwrap_or(0)
        }
    }
    unsafe impl NSTableViewDelegate for Controller {
        #[unsafe(method(tableView:heightOfRow:))]
        fn row_height(&self,_:&NSTableView,row:isize)->f64 {self.guard_callback(||{let session=self.ivars().session.borrow();session.row(row as usize).map_or(39.,|r|crate::native_rows::height(&r,session.management()))}).unwrap_or(39.)}
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_background(&self, _: &NSTableView, _: isize) -> Option<Retained<NSTableRowView>> {
            self.guard_callback(|| {
                let row = ResultRow::alloc(self.mtm()).set_ivars(());
                let row: Retained<ResultRow> = unsafe { msg_send![super(row), initWithFrame: rect(0.,0.,100.,39.)] };
                row.into_super()
            })
        }
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn row_view(&self, table: &NSTableView, column: Option<&NSTableColumn>, row: isize) -> Option<Retained<NSView>> {
            self.guard_callback(|| {
                let column = column?;
                let session = self.ivars().session.borrow();
                let command = session.row(usize::try_from(row).ok()?)?;
                if command.is_group && column.identifier().to_string()!="title"{return None;}
                if column.identifier().to_string()=="title" && !command.is_group {
                    return Some(crate::native_rows::title(self.mtm(),&command,column.width(),!self.ivars().panel.ivars().ordering_allowed,session.management()));
                }
                if !session.management() && column.identifier().to_string()=="shortcut" {
                    if let Some(value)=command.hotkey { return Some(crate::native_rows::shortcut(self.mtm(),value,column.width())); }
                }
                if column.identifier().to_string()=="enabled" {
                    if let Model::Live{view,..}=&*session {
                        let target=ruycast_native::actions::Target{id:command.id.to_string(),epoch:view.epoch,catalog_revision:view.catalog_revision};
                        let button=crate::native_library::EnabledButton::new(self,target,view.dismissal,command.enabled,command.title,column.width());
                        return Some(button.into_super().into_super().into_super());
                    }
                }
                if session.management() && ["aliases","shortcut"].contains(&column.identifier().to_string().as_str()) {
                    if let Model::Live{view,..}=&*session {
                        let mode=if column.identifier().to_string()=="aliases"{ruycast_native::actions::Mode::Alias}else{ruycast_native::actions::Mode::Hotkey};
                        let text=if mode==ruycast_native::actions::Mode::Alias {if command.aliases.is_empty(){"Add alias".into()}else{command.aliases.join(", ")}}else{command.hotkey.map(|k|presentation::shortcut(k).0).unwrap_or_else(||"Record Shortcut".into())};
                        let target=ruycast_native::actions::Target{id:command.id.to_string(),epoch:view.epoch,catalog_revision:view.catalog_revision};
                        let button=crate::native_library::BindingButton::new(self,target,view.dismissal,mode,&text,command.title,column.width());
                        button.setHidden(view.editor.as_ref().is_some_and(|e|e.inline && e.id==command.id && e.mode==mode));
                        return Some(button.into_super().into_super().into_super());
                    }
                }
                if command.system && column.identifier().to_string()=="title" {
                    let width=column.width();let view=NSView::initWithFrame(NSView::alloc(self.mtm()),rect(0.,0.,width,39.));
                    let title=label(self.mtm(),command.title,rect(0.,18.,width,19.),13.);
                    let subtitle=label(self.mtm(),command.subtitle.unwrap_or(""),rect(0.,1.,width,16.),11.);
                    title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);subtitle.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    if !command.enabled{title.setTextColor(Some(&NSColor::secondaryLabelColor()));}
                    subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    title.setAccessibilityLabel(Some(&NSString::from_str(command.title)));subtitle.setAccessibilityLabel(Some(&NSString::from_str(command.subtitle.unwrap_or(""))));
                    view.addSubview(&title);view.addSubview(&subtitle);return Some(view);
                }
                let text = match column.identifier().to_string().as_str() {
                    "aliases"=>command.aliases.join(", "),
                    "enabled"=>if command.enabled{"Enabled".into()}else{"Disabled".into()},
                    "shortcut" => command.app_shortcut.map(|s|s.keys.join(" ")).or_else(||command.hotkey.map(presentation::shortcut).map(|v|v.0)).unwrap_or_default(),
                    "kind" => if command.system {if command.id.starts_with("system.tab:"){"Safari Tab"}else if command.id.starts_with("system.window:"){"Window"}else if command.id.starts_with("system.menu:"){"Menu Action"}else if command.id.starts_with("system.kill:"){"Process"}else{"Shortcut"}}else{command.subtitle.unwrap_or_else(||presentation::kind_label(command.kind))}.into(),
                    _ => command.title.to_string(),
                };
                let identifier = column.identifier();
                if !session.management() && identifier.to_string() == "kind" {
                    let height = crate::native_rows::height(&command, false);
                    let view = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., column.width(), height));
                    let field = label(self.mtm(), &text, rect(0., 0., column.width(), 18.), 12.);
                    let text_height = field.intrinsicContentSize().height.ceil();
                    field.setFrame(rect(0., (height - text_height) / 2., column.width(), text_height));
                    field.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    field.setAlignment(NSTextAlignment::Right);
                    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                    view.addSubview(&field);
                    return Some(view);
                }
                let view = unsafe { table.makeViewWithIdentifier_owner(&identifier, None) };
                let field = if let Some(view) = view {
                    view.downcast::<NSTextField>().ok()?
                } else {
                    let field = label(self.mtm(), "", rect(0., 0., column.width(), 24.), 13.);
                    field.setIdentifier(Some(&identifier));
                    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    if identifier.to_string() != "title" {
                        field.setTextColor(Some(&NSColor::secondaryLabelColor()));
                        field.setFont(Some(&NSFont::systemFontOfSize(11.)));
                    }
                    field
                };
                if identifier.to_string()=="title"{let font=if command.is_group{NSFont::boldSystemFontOfSize(11.)}else{NSFont::systemFontOfSize(13.)};let color=if command.is_group{NSColor::secondaryLabelColor()}else{NSColor::labelColor()};field.setFont(Some(&font));field.setTextColor(Some(&color));}
                field.setStringValue(&NSString::from_str(&text));
                field.setFrame(rect(0.,9.,column.width(),22.));
                let accessible = if identifier.to_string()=="shortcut" {
                    command.app_shortcut.map(|s|format!("App shortcut: {}",s.label)).or_else(||command.hotkey.map(presentation::shortcut).map(|v|v.1)).unwrap_or_default()
                } else { text };
                field.setAccessibilityLabel(Some(&NSString::from_str(&accessible)));
                Some(field.into_super().into_super())
            }).flatten()
        }
        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select(&self,_:&NSTableView,row:isize)->bool{self.guard_callback(||self.ivars().session.borrow().row(row as usize).is_some_and(|r|!r.is_group)).unwrap_or(false)}
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, _: &NSNotification) {
            self.guard_callback(|| {
                if self.ivars().reconciling.get() { return; }
                let index = usize::try_from(self.ivars().table.selectedRow()).ok();
                self.ivars().session.borrow_mut().select_row(index);
                self.update_footer();
                self.reconcile_snippet_hotkey();
            });
        }
    }
    impl Controller {
        #[unsafe(method(inlineScrolled:))]
        fn inline_scrolled(&self,_:&NSNotification){self.guard_callback(||{
            let state=self.ivars();if state.reconciling.get(){return;}
            let data=state.session.borrow().surface();
            if !data.editor.as_ref().is_some_and(|e|e.inline){return;}
            let row=data.editor.as_ref().and_then(|e|{let model=state.session.borrow();(0..model.row_count()).find(|&i|model.row(i).is_some_and(|r|!r.is_group && r.id==e.id))});
            if let Some(parent)=state.panel.contentView(){state.surface.position_inline(data.editor.as_ref(),&state.table,&state.scroll,&parent,row);}
        });}
        #[unsafe(method(selectLibraryTab:))]
        fn select_tab(&self,sender:&NSButton){self.guard_callback(||{if let Some(tab)=crate::native_library::TABS.get(sender.tag() as usize){self.ivars().surface.stop();self.library_navigate("tab",tab);}});}
        #[unsafe(method(launcherBack:))]
        fn launcher_back(&self, _: &NSButton) { self.guard_callback(|| self.ivars().session.borrow().launcher_back()); }
        #[unsafe(method(filterLibrary:))]
        fn filter_library(&self,sender:&NSPopUpButton){self.guard_callback(||{if let Some(filter)=crate::native_library::FILTERS.get(sender.indexOfSelectedItem() as usize){self.library_navigate("filter",filter);}});}
        #[unsafe(method(addLibraryCommand:))]
        fn add_command(&self,sender:&NSPopUpButton){self.guard_callback(||{let name=match sender.indexOfSelectedItem(){1=>"new-snippet",2=>"new-quicklink",3=>"new-script",_=>return};sender.selectItemAtIndex(0);if self.ivars().session.borrow().surface().editor.is_some_and(|e|e.inline){self.library_create(name);}else{self.ivars().session.borrow_mut().named(name,false,None);}});}
        #[unsafe(method(toggleActions:))]
        fn toggle_action_button(&self,_:Option<&NSObject>){self.guard_callback(||{if self.ivars().session.borrow().surface().editor.is_some_and(|e|e.run_inline){self.toggle_input_menu();}else{self.sync_query();self.ivars().session.borrow_mut().toggle_actions();self.reconcile();}});}
        #[unsafe(method(chooseAction:))]
        fn choose_action(&self,sender:&NSButton){self.guard_callback(||{let data=self.ivars().session.borrow().surface();if let Some(item)=data.items.get(sender.tag() as usize){if data.editor.as_ref().is_some_and(|e|e.run_inline){if self.ivars().surface.input_button_current(sender,data.menu_token){self.ivars().session.borrow().input_menu(Some(item.id.into()),None);}}else if self.ivars().surface.action_button_current(sender){self.ivars().session.borrow_mut().named(item.id,true,None);}}});}
        #[unsafe(method(openAppChoice:))]
        fn open_app_action(&self,button:&crate::native_editor::AppButton){self.guard_callback(||{
            let token=button.ivars().token.get();
            if self.app_choice_valid(token) && self.ivars().surface.command.application.borrow().as_ref().is_some_and(|b|std::ptr::eq(&**b,button)){
                self.ivars().surface.command.open_app(token);self.app_save_state(token);
            }
        });}
        #[unsafe(method(cancelEditor:))]
        fn cancel_action(&self,_:Option<&NSObject>){self.cancel_editor();}
        #[unsafe(method(saveEditor:))]
        fn save_action(&self,_:Option<&NSObject>){self.save_editor();}
        #[unsafe(method(refreshMenuHealth:))]
        fn refresh_menu_health(&self,_:&NSNotification){self.guard_callback(||{self.ivars().menu_bar.refresh(self);});}
        #[unsafe(method(managementResized:))]
        fn management_resized(&self,_:&NSNotification){self.guard_callback(||{if !self.ivars().reconciling.get(){self.reconcile();}});}
        #[unsafe(method(recordSetting:))]
        fn record_setting(&self,sender:&NSButton){self.guard_callback(||{self.ivars().settings.record(self,sender.tag() as usize);});}
        #[unsafe(method(snippetHotkey:))]
        fn snippet_hotkey_action(&self,sender:&NSButton){self.guard_callback(||{
            let binding={let control=self.ivars().snippet_hotkey.borrow();control.as_ref()
                .filter(|c|std::ptr::eq::<NSButton>(&*c.button,sender)&&sender.isEnabled()&&!sender.isHidden())
                .map(|c|c.binding.clone())};
            let Some(binding)=binding else{return;};
            if self.ivars().input.currentEditor().and_then(|editor|editor.downcast_ref::<NSTextView>().map(|text|unsafe{msg_send![text,hasMarkedText]})).unwrap_or(false){return;}
            self.sync_query();
            let Some((current,true))=self.snippet_hotkey_binding() else{return;};
            if binding!=current || self.inline_has_marked_text(){return;}
            let mut model=self.ivars().session.borrow_mut();
            let index=model.selection();
            model.named("set-hotkey",false,index);
        });}
        #[unsafe(method(recordShortcut:))]
        fn record_action(&self,_:Option<&NSObject>){self.guard_callback(||{
            let data=self.ivars().session.borrow().surface();
            if !data.visible || !data.editor.is_some_and(|e|[ruycast_native::actions::Mode::Hotkey,ruycast_native::actions::Mode::Edit].contains(&e.mode)) || (self.ivars().surface.form.isHidden() && !self.ivars().surface.inline_visible.get()) || self.ivars().surface.pending.get().is_some() || (!self.ivars().panel.isKeyWindow() && !data.allow_hidden){return;}
            if self.ivars().surface.record.active(){self.ivars().surface.record.stop();}else{self.ivars().surface.record.begin();}
        });}
        #[unsafe(method(addArgument:))]
        fn add_argument(&self,_:Option<&NSObject>){self.change_argument(None);}
        #[unsafe(method(removeArgument:))]
        fn remove_argument(&self,sender:&NSButton){self.change_argument(usize::try_from(sender.tag()).ok());}
        #[unsafe(method(activateRow:))]
        fn activate_row(&self, _: Option<&NSObject>) {
            self.guard_callback(|| {
                if self.ivars().session.borrow().script_result().is_some() && self.ivars().panel.firstResponder().and_then(|r|r.downcast_ref::<NSTextView>().map(|t|unsafe{msg_send![t,hasMarkedText]})).unwrap_or(false){return;}
                let row=usize::try_from(self.ivars().table.selectedRow()).ok();
                let inline=self.ivars().session.borrow().surface().editor.is_some_and(|e|e.inline);
                if inline {
                    let binding={let model=self.ivars().session.borrow();if let Model::Live{view,..}=&*model{row.and_then(|i|view.rows.get(i)).filter(|r|!r.is_group).map(|r|(ruycast_native::actions::Target{id:r.id.clone(),catalog_revision:view.catalog_revision,epoch:view.epoch},view.dismissal))}else{None}};
                    if let Some((target,dismissal))=binding{self.library_inline(target,dismissal,ruycast_native::actions::Mode::Edit);}
                    return;
                }
                self.ivars().session.borrow_mut().activate_row(usize::try_from(self.ivars().table.selectedRow()).ok());
                self.reconcile();
                if !self.ivars().session.borrow().visible() { self.ivars().panel.orderOut(None); }
            });
        }
    }
);

impl Controller {
    pub(crate) fn snippet_link_changed(&self){self.guard_callback(||self.reconcile());}
    fn performance_field(&self) -> u64 {
        (&*self.ivars().input as *const NSTextField as usize) as u64
    }
    fn cancel_performance_input(&self) {
        ruycast_native::performance_trace::shared().cancel_input(Some(self.performance_field()));
    }
    fn reconcile_performance_input(&self) {
        let state = self.ivars();
        let hidden = state.input.isHidden();
        let (cancel, reconciled) = {
            let model = state.session.borrow();
            let current = match &*model {
                Model::Live { view, connection } => view.epoch == connection.latest_epoch()
                    && view.dismissal == connection.latest_dismissal(),
                _ => true,
            };
            // The retained field changes role for run input; that retires its
            // search reconciliation just as dismantling the old field does.
            (!model.visible() || hidden || model.run_inline() || state.failed.get(), current)
        };
        let trace = ruycast_native::performance_trace::shared();
        if cancel { trace.cancel_input(Some(self.performance_field())); }
        else if reconciled { trace.did_reconcile_field(self.performance_field()); }
    }
    pub(crate) fn date_input_changed(&self, sender: &crate::native_editor::CommandText) {
        self.guard_callback(|| {
            if !self.ivars().reconciling.get()
                && std::ptr::eq(sender, &*self.ivars().surface.command.text)
                && !self.ivars().session.borrow().run_inline()
                && !unsafe{msg_send![sender,hasMarkedText]}
            {
                self.ivars().session.borrow().preview_input(sender.string().to_string());
            }
        });
    }
    pub(crate) fn monitor_control(&self,token:u64,operation:ruycast_native::monitor::Operation){self.guard_callback(||{if let Some(connection)=self.ivars().session.borrow().os_connection(){connection.monitor_action(token,operation);}});}
    pub(crate) fn monitor_layout(&self){self.guard_callback(||{if let Some(monitor)=self.ivars().monitor.borrow().as_ref(){monitor.layout();}});}
    pub(crate) fn process_sort_bound(&self,sort:ruycast_native::processes::Sort,epoch:u64,dismissal:u64){self.guard_callback(||{if self.ivars().session.borrow().process_state().is_some_and(|(_,e,d)|e==epoch&&d==dismissal){if let Some(connection)=self.ivars().session.borrow().os_connection(){if connection.latest_epoch()==epoch&&connection.latest_dismissal()==dismissal{connection.action(ruycast_native::connected::Action::ProcessSort(sort));}}}});}
    fn process_dashboard_request(&self) {
        let request=self.ivars().session.borrow().dashboard_request();
        let Some((serial,intent,dismissal))=request else{return;};
        if self.ivars().handled_dashboard_request.get()==serial{return;}
        let connection=self.ivars().session.borrow().dashboard_connection();
        let Some(connection)=connection else{return;};
        if connection.latest_intent()!=intent || connection.latest_dismissal()!=dismissal {
            self.ivars().handled_dashboard_request.set(serial);return;
        }
        if self.ivars().surface.pending.get().is_some() || self.inline_has_marked_text() || self.ivars().session.borrow().editor_saving(){return;}
        self.ivars().handled_dashboard_request.set(serial);
        if self.ivars().session.borrow().dashboard().is_some() {return;}
        self.library_navigate("tab","dashboard");
    }
    pub(crate) fn clipboard_control(&self,operation:&str,id:Option<String>,revision:u64,epoch:u64,dismissal:u64){self.guard_callback(||self.ivars().session.borrow().clipboard_action(operation,id,revision,epoch,dismissal));}
    pub(crate) fn dashboard_control(&self,token:u64,operation:ruycast_native::analytics_dashboard::Operation) {
        self.guard_callback(|| {
            if self.ivars().dashboard.dialog_open() {return;}
            let connection=self.ivars().session.borrow().dashboard_connection();
            if let Some(connection)=connection {connection.dashboard_action(token,operation);}
        });
    }
    pub(crate) fn dashboard_live(&self,token:u64)->bool {
        let model=self.ivars().session.borrow();
        match &*model {
            Model::Live {view,connection} => view.visible && view.dashboard.as_ref().is_some_and(|s|s.token==token)
                && !connection.stop_requested() && !connection.stopped()
                && connection.latest_dismissal()==view.dismissal && connection.latest_epoch()==view.epoch,
            _=>false,
        }
    }
    pub(crate) fn dashboard_export_dialog(&self,token:u64) {
        self.guard_callback(|| {
            let connection=self.ivars().session.borrow().dashboard_connection();
            if let Some(connection)=connection.filter(|_|self.dashboard_live(token)) {
                self.ivars().dashboard.begin_export(self,token,&self.ivars().panel,connection.hidden_tests);
            }
        });
    }
    pub(crate) fn dashboard_export_completed(&self,serial:u64,token:u64,accepted:bool,panel:Option<&NSSavePanel>) {
        self.guard_callback(|| {
            if !self.ivars().dashboard.finish_dialog(serial) || !accepted {return;}
            let path=panel.and_then(|p|p.URL()).filter(|u|u.isFileURL()).and_then(|url| {
                let mut bytes=vec![0i8;libc::PATH_MAX as usize];
                if !unsafe {url.getFileSystemRepresentation_maxLength(std::ptr::NonNull::new(bytes.as_mut_ptr()).expect("nonempty path buffer"),bytes.len())} {return None;}
                use std::os::unix::ffi::OsStrExt;
                let bytes=unsafe{std::ffi::CStr::from_ptr(bytes.as_ptr())};
                Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes.to_bytes())))
            });
            let op=path.map(ruycast_native::analytics_dashboard::Operation::Export).unwrap_or(ruycast_native::analytics_dashboard::Operation::ExportPathUnavailable);
            self.dashboard_control(token,op);
        });
    }
    pub(crate) fn guard_callback<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        if self.ivars().failed.get()
            || self.ivars().panel.ivars().failed.get()
            || self.ivars().table.ivars().get()
        {
            return None;
        }
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(value) => Some(value),
            Err(_) => {
                // No panic unwinds across an Objective-C callback. Hide the
                // isolated panel; never execute a partially selected intent.
                self.ivars().failed.set(true);
                self.ivars().panel.orderOut(None);
                None
            }
        }
    }
    fn new(
        mtm: MainThreadMarker,
        width: f64,
        dark: bool,
        session: impl Into<Model>,
    ) -> Retained<Self> {
        let session: Model = session.into();
        let management = session.management();
        let height = if management { 650. } else { 490. };
        let search_offset = if management { 108. } else { 0. };
        let style = if management {
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
        } else {
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel
        };
        let panel = unsafe {
            let panel = PalettePanel::alloc(mtm).set_ivars(PanelState {
                failed: Cell::new(false),
                owner: RefCell::new(None),
                management,
                ordering_allowed: matches!(&session,Model::Live{connection,..} if !connection.hidden_tests),
                blocked_orders: Cell::new(0),
                #[cfg(feature = "test-tools")]
                hidden_repairs: Cell::new(0),
                #[cfg(feature = "test-tools")]
                last_hidden_repair: RefCell::new(None),
            });
            let panel: Retained<PalettePanel> = msg_send![super(panel),
                initWithContentRect: NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(width, height)),
                styleMask: style,
                backing: NSBackingStoreType::Buffered, defer: false];
            panel.setReleasedWhenClosed(false);
            panel
        };
        panel.setTitle(if management {
            ns_string!("Ruycast Settings")
        } else {
            ns_string!("Ruycast")
        });
        panel.setHidesOnDeactivate(false);
        if !management {
            panel.setLevel(NSFloatingWindowLevel);
            panel.setMovableByWindowBackground(true);
            panel.setOpaque(false);
            panel.setHasShadow(true);
            panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary);
        }
        if management { panel.setContentMinSize(NSSize::new(680.,480.)); }
        let background = if management {
            NSColor::windowBackgroundColor()
        } else { NSColor::clearColor() };
        panel.setBackgroundColor(Some(&background));
        panel.setAppearance(
            NSAppearance::appearanceNamed(if dark {
                ns_string!("NSAppearanceNameDarkAqua")
            } else {
                ns_string!("NSAppearanceNameAqua")
            })
            .as_deref(),
        );
        let view: Retained<PaletteContent> = unsafe {
            let view = PaletteContent::alloc(mtm).set_ivars(Cell::new(false));
            msg_send![super(view), initWithFrame: rect(0., 0., width, height)]
        };
        panel.setContentView(Some(&view));
        let input = NSTextField::textFieldWithString(ns_string!(""), mtm);
        input.setFrame(rect(24., if management {height-145.} else {420. + search_offset}, width - 48., 44.));
        input.setFont(Some(&NSFont::systemFontOfSize(if management { 23. } else { 17. })));
        let prompt = if management {
            ns_string!("Search commands…")
        } else {
            ns_string!("Search apps, tabs or commands…")
        };
        input.setPlaceholderString(Some(prompt));
        unsafe {
            let _: () = msg_send![&*input, setAccessibilityLabel: prompt];
        }
        input.setBezeled(false);
        input.setDrawsBackground(false);
        view.addSubview(&input);
        let heading = label(
            mtm,
            "Results",
            rect(24., if management {height-180.} else {384. + search_offset}, width - 48., 20.),
            11.,
        );
        heading.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&heading);
        let scroll = NSScrollView::new(mtm);
        scroll.setFrame(rect(16., 50., width - 32., 322. + search_offset));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        let table: Retained<ResultTable> = unsafe {
            let table = ResultTable::alloc(mtm).set_ivars(Cell::new(false));
            msg_send![super(table), initWithFrame: rect(0.,0.,width-32.,322.+search_offset)]
        };
        table.setFrame(rect(0., 0., width - 32., 322. + search_offset));
        if management {
            let header = NSTableHeaderView::initWithFrame(
                NSTableHeaderView::alloc(mtm),
                rect(0., 0., width - 32., 24.),
            );
            table.setHeaderView(Some(&header));
        } else {
            table.setHeaderView(None);
        }
        table.setRowHeight(39.);
        table.setIntercellSpacing(NSSize::new(10., 0.));
        table.setAllowsMultipleSelection(false);
        table.setBackgroundColor(&NSColor::clearColor());
        table.setAllowsEmptySelection(true);
        table.setColumnAutoresizingStyle(
            NSTableViewColumnAutoresizingStyle::FirstColumnOnlyAutoresizingStyle,
        );
        let columns = if management {
            vec![
                ("title", width - 514.),
                ("aliases", 130.),
                ("shortcut", 112.),
                ("kind", 108.),
                ("enabled", 72.),
            ]
        } else {
            vec![("title", width - 292.), ("shortcut", 112.), ("kind", 108.)]
        };
        for (name, column_width) in columns {
            let column = NSTableColumn::initWithIdentifier(
                NSTableColumn::alloc(mtm),
                &NSString::from_str(name),
            );
            if management {
                column.setTitle(&NSString::from_str(match name {
                    "title" => "Name",
                    "aliases" => "Aliases",
                    "shortcut" => "Hotkey",
                    "kind" => "Type",
                    _ => "Enabled",
                }));
            }
            column.setWidth(column_width);
            column.setMinWidth(if name == "title" { 120. } else { column_width });
            table.addTableColumn(&column);
        }
        unsafe {
            let _: () = msg_send![&*table, setAccessibilityLabel: ns_string!("Results")];
        }
        scroll.setDocumentView(Some(&table));
        view.addSubview(&scroll);
        let empty = label(
            mtm,
            "No commands found",
            rect(24., 330., width - 48., 24.),
            13.,
        );
        empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&empty);
        let footer = label(mtm, "", rect(24., 14., width - 48., 22.), 11.);
        footer.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&footer);
        let launcher_back = NSButton::new(mtm);
        launcher_back.setTitle(ns_string!("Back"));
        launcher_back.setAccessibilityLabel(Some(ns_string!("Back")));
        launcher_back.setBordered(false);
        launcher_back.setFrame(rect(18., 10., 68., 28.));
        launcher_back.setHidden(true);
        view.addSubview(&launcher_back);
        let notice = label(mtm, "", rect(24., 42., width - 48., 24.), 11.);
        notice.setTextColor(Some(&NSColor::systemRedColor()));
        view.addSubview(&notice);
        let surface = crate::native_surface::Surface::new(mtm, &view, width, height - 490.);
        let result_panel = crate::native_result::ResultPanel::new(mtm, &view, width, height);
        let toolbar =
            management.then(|| crate::native_library::Toolbar::new(mtm, &view, width, height));
        let this = Self::alloc(mtm).set_ivars(State {
            outside: RefCell::new(None),
            outside_generation: Cell::new(0),
            outside_stopped: Cell::new(false),
            monitor:RefCell::new(None),
            process_controls:crate::native_os_controls::Controls::new(mtm,&view),
            failed: Cell::new(false),
            peer: RefCell::new(None),
            toolbar,
            clipboard_preferences:crate::native_clipboard_preferences::Preferences::new(mtm,&view,width,height),
            clipboard_panel: crate::native_clipboard::Panel::new(mtm,&view,width,height),
            dashboard: crate::native_dashboard::Dashboard::new(mtm,&view,width,height),
            settings:crate::native_settings::Settings::new(mtm,&view,width,height),
            menu_bar:Default::default(),
            handled_dashboard_request:Cell::new(0),
            table_height: if management {height-250.} else {322. + search_offset},
            surface,
            result_panel,
            session: RefCell::new(session),
            panel,
            input,
            table,
            heading,
            empty,
            footer,
            snippet_hotkey:RefCell::new(None),
            #[cfg(feature="test-tools")]
            stale_snippet_hotkey:RefCell::new(None),
            #[cfg(feature = "test-tools")]
            held_record_menu: RefCell::new(None),
            launcher_back,
            notice,
            #[cfg(feature = "test-tools")]
            stale_text: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            stale_enabled: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            stale_input_menu: RefCell::new(None),
            toast: RefCell::new(Some(crate::native_notice::Service::new(mtm))),
            timers: RefCell::new(vec![]),
            #[cfg(feature = "test-tools")]
            clipboard_fixture:RefCell::new(None),
            #[cfg(feature = "test-tools")]
            source_failure: Cell::new(false),
            #[cfg(feature = "test-tools")]
            shutdown_witness: RefCell::new(None),
            probe_output: RefCell::new(None),
            scroll,
            reconciling: Cell::new(false),
            restore_result_focus: Cell::new(false),
            keyboard: RefCell::new(None),
            run_inline_token: Cell::new(0),
            #[cfg(feature = "test-tools")]
            defer_input_child: Cell::new(false),
            #[cfg(feature = "test-tools")]
            recorder_acquire_starts: Cell::new(0),
            #[cfg(feature = "test-tools")]
            key_event_handled: Cell::new(None),
            run_inline_disabled: Cell::new(false),
            run_inline_selection: Cell::new(None),
            pending_input_menu: Cell::new(None),
            inline_autosave_allowed: Cell::new(true),
            inline_autosave: RefCell::new(None),
            rendered_revision: Cell::new(u64::MAX),
            rendered_bindings: Cell::new(None),
            rendered_title_width: Cell::new(None),
        });
        let controller: Retained<Self> = unsafe { msg_send![super(this), init] };
        unsafe {
            controller
                .ivars()
                .input
                .setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        }
        unsafe {
            controller
                .ivars()
                .table
                .setDataSource(Some(ProtocolObject::from_ref(&*controller)));
            controller
                .ivars()
                .table
                .setDelegate(Some(ProtocolObject::from_ref(&*controller)));
            controller.ivars().table.setTarget(Some(&controller));
            controller.ivars().table.setAction(Some(sel!(activateRow:)));
            controller.ivars().launcher_back.setTarget(Some(&controller));
            controller.ivars().launcher_back.setAction(Some(sel!(launcherBack:)));
        }
        if let Some(toolbar) = &controller.ivars().toolbar {
            toolbar.bind(&controller);
        }
        *controller.ivars().panel.ivars().owner.borrow_mut() = Some(Weak::new(&*controller));
        controller.ivars().surface.bind(&controller);
        if management {
            unsafe { NSNotificationCenter::defaultCenter().addObserver_selector_name_object(&controller,sel!(managementResized:),Some(ns_string!("NSWindowDidResizeNotification")),Some(&*controller.ivars().panel)); }
        }
        if !management && controller.ivars().panel.ivars().ordering_allowed {
            for name in ["NSWorkspaceDidLaunchApplicationNotification","NSWorkspaceDidTerminateApplicationNotification"] {
                unsafe{NSWorkspace::sharedWorkspace().notificationCenter().addObserver_selector_name_object(&controller,sel!(refreshMenuHealth:),Some(&NSString::from_str(name)),None);}
            }
        }

        controller
            .ivars()
            .scroll
            .contentView()
            .setPostsBoundsChangedNotifications(true);
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &controller,
                sel!(inlineScrolled:),
                Some(ns_string!("NSViewBoundsDidChangeNotification")),
                Some(&controller.ivars().scroll.contentView()),
            );
        }

        unsafe {
            controller
                .ivars()
                .surface
                .alias
                .setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        }
        controller.reconcile();
        controller
    }
    fn update_footer(&self) {
        let session = self.ivars().session.borrow();
        let value = session
            .selected()
            .map(|c| format!("{} ↵", c.action))
            .unwrap_or_default();
        self.ivars()
            .footer
            .setStringValue(&NSString::from_str(&value));
    }
    fn snippet_hotkey_binding(&self)->Option<(SnippetHotkeyBinding,bool)> {
        let state=self.ivars();let model=state.session.borrow();
        let Model::Live{view,connection}=&*model else{return None;};
        if model.management() || !view.visible || view.clipboard.is_some() || view.editor.is_some()
            || view.script_result.is_some() || view.dashboard.is_some() || view.settings.is_some()
            || view.utility_scope.is_some() || connection.stop_requested()
            || connection.latest_epoch()!=view.epoch || connection.latest_dismissal()!=view.dismissal {return None;}
        let row=view.selection.and_then(|index|view.rows.get(index))?;
        if row.is_group || row.kind!="snippet" || !view.action_items.iter().any(|item|item.id=="set-hotkey") {return None;}
        Some((SnippetHotkeyBinding{
            target:ruycast_native::actions::Target{id:row.id.clone(),catalog_revision:view.catalog_revision,epoch:view.epoch},
            dismissal:view.dismissal,configuration:view.config_revision,query:view.query.clone(),launcher:view.launcher_scope,system:view.system_scope,
        },!view.execution_running && !view.utility_busy && state.surface.pending.get().is_none() && !state.surface.record.active()))
    }
    fn retire_snippet_hotkey(&self) {
        let old=self.ivars().snippet_hotkey.borrow_mut().take();
        if let Some(old)=old {unsafe{old.button.setTarget(None);old.button.setAction(None);}old.button.removeFromSuperview();}
    }
    fn reconcile_snippet_hotkey(&self) {
        let next=self.snippet_hotkey_binding();
        let changed=self.ivars().snippet_hotkey.borrow().as_ref().map(|c|&c.binding)!=next.as_ref().map(|(binding,_)|binding);
        if changed {
            self.retire_snippet_hotkey();
            if let Some((binding,_))=&next {
                let button=NSButton::new(self.mtm());button.setTitle(ns_string!("Set Hotkey…"));
                unsafe{button.setTarget(Some(self));button.setAction(Some(objc2::sel!(snippetHotkey:)));}
                button.setBordered(false);button.setFont(Some(&NSFont::systemFontOfSize(11.)));
                button.setAlignment(NSTextAlignment::Left);
                button.setAccessibilityLabel(Some(ns_string!("Set Hotkey…")));
                button.setAccessibilityRole(Some(unsafe{NSAccessibilityButtonRole}));
                if let Some(parent)=self.ivars().panel.contentView(){parent.addSubview(&button);}
                *self.ivars().snippet_hotkey.borrow_mut()=Some(SnippetHotkeyControl{button,binding:binding.clone()});
            }
        }
        let state=self.ivars();
        let ordinary={let model=state.session.borrow();!model.management() && model.clipboard().is_none()};
        if ordinary {
            let width=state.panel.contentView().map_or(760.,|v|v.bounds().size.width);
            let scoped=!state.launcher_back.isHidden();
            let x=if scoped{100.}else{24.};
            let base_width=if scoped{width-270.}else{width-48.};
            let button=state.snippet_hotkey.borrow().as_ref().map(|c|c.button.clone());
            if let Some(button)=button {
                button.setFrame(rect(x,10.,88.,26.));button.setEnabled(next.as_ref().is_some_and(|(_,enabled)|*enabled));
                state.footer.setFrame(rect(x+100.,14.,(base_width-100.).max(1.),22.));
            }else{state.footer.setFrame(rect(x,14.,base_width.max(1.),22.));}
        }
    }
    /// Shared Show/mode-switch geometry. Hidden fixtures branch before screen
    /// inventory and cursor reads; injected geometry is never window ordering.
    pub(crate) fn layout_for_mode(&self,clipboard:bool,injected:Option<NSRect>) {
        let state=self.ivars();let hidden=!state.panel.ivars().ordering_allowed;
        let visible=if let Some(frame)=injected{Some(frame)}else if hidden{
            let width=std::env::var("RUYCAST_NATIVE_TEST_WIDTH").ok().and_then(|s|s.parse::<f64>().ok()).filter(|w|(600.0..=1200.0).contains(w));
            Some(rect(-20000.,-20000.,width.map_or(1440.,|w|w+48.),900.))
        }else{
            let mouse=NSEvent::mouseLocation();NSScreen::screens(self.mtm()).iter().find(|s|objc2_foundation::NSPointInRect(mouse,s.frame())).or_else(||NSScreen::mainScreen(self.mtm())).map(|s|s.visibleFrame())
        };
        if let Some(frame)=visible {
            if state.session.borrow().management(){
                if !hidden{let size=state.panel.frame().size;state.panel.setFrameOrigin(NSPoint::new(frame.origin.x+(frame.size.width-size.width)/2.,frame.origin.y+(frame.size.height-size.height)/2.));}
            }else{
                let [x,y,w,h]=presentation::palette_frame([frame.origin.x,frame.origin.y,frame.size.width,frame.size.height],clipboard);
                state.panel.setFrame_display(rect(x,y,w,h),false);
                if let Some(content)=state.panel.contentView(){let size=content.bounds().size;state.input.setFrame(rect(24.,size.height-70.,size.width-48.,44.));state.heading.setFrame(rect(24.,size.height-110.,size.width-48.,20.));state.scroll.setFrame(rect(16.,50.,size.width-32.,(size.height-168.).max(1.)));state.footer.setFrame(rect(24.,14.,size.width-48.,22.));state.notice.setFrame(rect(24.,42.,size.width-48.,24.));}
            }
        }
    }
    fn reconcile(&self) {
        let state = self.ivars();
        state.reconciling.set(true);
        if let Some(parent)=state.panel.contentView(){let size=parent.bounds().size;state.surface.layout(size.width,size.height,state.session.borrow().management());let graphite=self.management_graphite();if let Some(content)=parent.downcast_ref::<PaletteContent>(){content.ivars().set(graphite);content.setNeedsDisplay(true);}state.surface.theme(graphite);}
        if state.session.borrow().management() {
            if let Some(parent)=state.panel.contentView(){
                let size=parent.bounds().size;
                state.input.setFrame(rect(24.,size.height-145.,size.width-48.,44.));
                state.heading.setFrame(rect(24.,size.height-180.,size.width-48.,20.));
                state.footer.setFrame(rect(24.,14.,size.width-48.,22.));
                state.notice.setFrame(rect(24.,42.,size.width-48.,24.));
                state.empty.setFrame(rect(24.,size.height-250.,size.width-48.,24.));
                state.scroll.setFrameSize(NSSize::new(size.width-32.,(size.height-250.).max(80.)));
            }
        }
        if let Some(surface)=state.session.borrow().monitor(){
            if surface.shown&&state.monitor.borrow().is_none(){*state.monitor.borrow_mut()=Some(crate::native_monitor::Monitor::new(self,self.mtm(),state.session.borrow().os_connection().is_some_and(|c|c.hidden_tests)));}
            if let Some(monitor)=state.monitor.borrow().as_ref(){monitor.apply(self,surface,state.session.borrow().theme());}
        }
        let revision = state.session.borrow().results_revision();
        let bindings = state.session.borrow().library_binding_stamp();
        if state.rendered_revision.get() != revision || state.rendered_bindings.get() != bindings {
            state.table.reloadData();
            state.rendered_revision.set(revision);
            state.rendered_bindings.set(bindings);
        }
        let (count, selection) = {
            let session = state.session.borrow();
            (session.row_count(), session.selection())
        };
        state.heading.setStringValue(&NSString::from_str(&state.session.borrow().results_heading()));
        let mut message = state.session.borrow().message().to_string();
        let date_preview = state.session.borrow().surface().editor
            .filter(|e|e.run_inline)
            .and_then(|e|e.task_date);
        let showing_date = message.is_empty() && date_preview.is_some();
        if showing_date {message = date_preview.as_ref().expect("date preview").label.clone();}
        let notice_color = if (message == "Running…" && state.session.borrow().execution_running())
            || message == "Loading…"
        {
            NSColor::secondaryLabelColor()
        } else if showing_date || state.session.borrow().utility_scope().is_some() {
            NSColor::labelColor()
        } else {
            NSColor::systemRedColor()
        };
        state.notice.setTextColor(Some(&notice_color));
        state.notice.setStringValue(&NSString::from_str(&message));
        state.notice.setHidden(message.is_empty());
        let width = state.scroll.frame().size.width;
        let table_height=state.panel.contentView().map(|v|(v.bounds().size.height-if state.session.borrow().management(){250.}else{168.}).max(80.)).unwrap_or(state.table_height);
        state.scroll.setFrame(if message.is_empty() {
            rect(16., 50., width, table_height)
        } else {
            rect(16., 76., width, table_height - 26.)
        });
        state.empty.setHidden(count != 0);
        if let Some(index) = selection {
            state.table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
            state.table.scrollRowToVisible(index as isize);
        } else {
            unsafe {
                state.table.deselectAll(None);
            }
        }
        self.update_footer();
        let data = state.session.borrow().surface();
        let was_inline = state.surface.inline_visible.get();
        let inline_row = data.editor.as_ref().filter(|e| e.inline).and_then(|e| {
            let model = state.session.borrow();
            (0..model.row_count())
                .find(|&i| model.row(i).is_some_and(|r| !r.is_group && r.id == e.id))
        });
        if let Some(parent) = state.panel.contentView() {
            state.surface.position_inline(
                data.editor.as_ref(),
                &state.table,
                &state.scroll,
                &parent,
                inline_row,
            );
        }
        let run_inline = data.editor.as_ref().filter(|e| e.run_inline);
        let entered_inline = run_inline.is_some_and(|e| state.run_inline_token.get() != e.token);
        if let Some(editor) = run_inline {
            if entered_inline && editor.resumed_input_from != Some(state.run_inline_token.get()) {
                state.input.setStringValue(&NSString::from_str(
                    editor.command["input"].as_str().unwrap_or(""),
                ));
                if let Some(text) = state
                    .input
                    .currentEditor()
                    .and_then(|e| e.downcast::<NSTextView>().ok())
                {
                    if let Some(undo) = text.undoManager() {
                        undo.removeAllActions();
                    }
                }
                state.run_inline_selection.set(None);
            }
            if entered_inline {
                state.run_inline_token.set(editor.token);
                state.pending_input_menu.set(None);
            }
            let prompt = NSString::from_str(if editor.task_date.is_some() {
                "Datum, bijv. morgen of in 3 days"
            } else {editor.command["inputPrompt"].as_str().unwrap_or("Input")});
            state.input.setPlaceholderString(Some(&prompt));
            state.input.setAccessibilityLabel(Some(&prompt));
            state
                .heading
                .setStringValue(&NSString::from_str(&editor.title));
            state.footer.setStringValue(if data.actions_open {
                ns_string!("Choose ↵ · Close Esc")
            } else if editor.input_disabled {
                ns_string!("Disabled · Expand ⌘↵")
            } else if editor.command["captureInput"] == true {
                ns_string!("Save ↵ · Expand ⌘↵")
            } else {
                ns_string!("Run ↵ · Expand ⌘↵")
            });
        } else if state.run_inline_token.replace(0) != 0 {
            state.run_inline_selection.set(None);
            state.pending_input_menu.set(None);
            if let Some(text) = state
                .input
                .currentEditor()
                .and_then(|e| e.downcast::<NSTextView>().ok())
            {
                unsafe {
                    let _: () = msg_send![&*text, unmarkText];
                }
                if let Some(undo) = text.undoManager() {
                    undo.removeAllActions();
                }
            }
            state
                .input
                .setStringValue(&NSString::from_str(state.session.borrow().query()));
            state
                .input
                .setPlaceholderString(Some(ns_string!("Search apps, tabs or commands…")));
            state
                .input
                .setAccessibilityLabel(Some(ns_string!("Search apps, tabs or commands…")));
        }
        let editing = data.editor.is_some()
            && run_inline.is_none()
            && data.visible
            && !state.surface.inline_visible.get();
        let restore =
            (was_inline || !state.surface.form.isHidden() || !state.surface.menu.isHidden())
                && data.editor.is_none()
                && !data.actions_open;
        state.input.setHidden(editing);
        state.heading.setHidden(editing);
        state.scroll.setHidden(editing);
        state.footer.setHidden(editing);
        if editing {
            state.empty.setHidden(true);
        }
        if state.session.borrow().management() {
            let shown = state.table.rowsInRect(state.table.visibleRect());
            for row in shown.location..(shown.location + shown.length).min(count) {
                let id = state.session.borrow().row(row).map(|r| r.id.to_string());
                for (column, mode) in [
                    (1, ruycast_native::actions::Mode::Alias),
                    (2, ruycast_native::actions::Mode::Hotkey),
                ] {
                    if let Some(view) =
                        state
                            .table
                            .viewAtColumn_row_makeIfNecessary(column, row as isize, true)
                    {
                        view.setHidden(data.editor.as_ref().is_some_and(|e| {
                            e.inline && Some(e.id.as_str()) == id.as_deref() && e.mode == mode
                        }));
                    }
                }
            }
        }
        state.surface.apply(&data, &state.panel);
        let link_open=state.surface.command.link_open();
        state.surface.cancel.setEnabled(!link_open);
        if link_open {state.surface.save.setHidden(true);state.surface.save.setEnabled(false);state.surface.record.stop();state.surface.record.setEnabled(false);}
        if let Some(toolbar)=state.toolbar.as_ref(){for button in &toolbar.tabs{button.setEnabled(!link_open);}}



        if state.surface.pending.get().is_none() {
            state.pending_input_menu.set(None);
        }

        let input_disabled = run_inline.is_some()
            && (state.surface.pending.get().is_some()
                || run_inline.is_some_and(|editor| editor.saving)
                || data.execution_running
                || data.actions_open);
        if state.input.isEnabled() == input_disabled {
            state.input.setEnabled(!input_disabled);
        }
        if state.run_inline_disabled.replace(input_disabled)
            && !input_disabled
            && run_inline.is_some()
            && data.visible
        {
            state.panel.makeFirstResponder(Some(&state.input));
            if let Some(text) = state
                .input
                .currentEditor()
                .and_then(|e| e.downcast::<NSTextView>().ok())
            {
                if let Some(range) = state.run_inline_selection.take() {
                    let len = text.string().length();
                    let location = range.location.min(len);
                    text.setSelectedRange(objc2_foundation::NSRange::new(
                        location,
                        range.length.min(len - location),
                    ));
                }
            }
        }

        if entered_inline && !input_disabled && data.visible {
            state.panel.makeFirstResponder(Some(&state.input));
            if let Some(text) = state
                .input
                .currentEditor()
                .and_then(|e| e.downcast::<NSTextView>().ok())
            {
                text.setSelectedRange(objc2_foundation::NSRange::new(text.string().length(), 0));
            }
        }
        state
            .heading
            .setAccessibilityLabel(Some(&state.heading.stringValue()));
        state
            .footer
            .setAccessibilityLabel(Some(&state.footer.stringValue()));
        if let Some(toolbar) = &state.toolbar {
            if let Some(parent)=state.panel.contentView(){let size=parent.bounds().size;toolbar.layout(size.width,size.height);}
            toolbar.apply(state.session.borrow().library_info(), editing);
        }
        if data.actions_open && !editing {
            state.panel.makeFirstResponder(Some(&state.surface.menu));
        } else if restore && data.visible {
            state.panel.makeFirstResponder(Some(&state.input));
        }
        let result = state.session.borrow().script_result().cloned();
        state
            .result_panel
            .apply(self, result.as_ref(), data.visible);
        if let Some(r) = result {
            state.surface.form.setHidden(true);
            state.surface.menu.setHidden(true);
            state.surface.inline_clip.setHidden(true);
            state.surface.actions_button.setHidden(true);
            state.footer.setHidden(true);
            state.input.setHidden(true);
            state.input.setEnabled(!r.busy);
            state.heading.setHidden(false);
            state.heading.setStringValue(&NSString::from_str(&r.title));
            state
                .heading
                .setAccessibilityLabel(Some(&state.heading.stringValue()));
            state.scroll.setHidden(r.detail_visible);
            state
                .empty
                .setHidden(r.detail_visible || !r.rows.is_empty());
            state
                .empty
                .setStringValue(&NSString::from_str("No results"));
            let prompt = NSString::from_str(&r.placeholder);
            state.input.setPlaceholderString(Some(&prompt));
            state.input.setAccessibilityLabel(Some(&prompt));
            if r.detail_visible {
                state.result_panel.focus();
            }
        } else {
            state
                .empty
                .setStringValue(&NSString::from_str("No commands found"));
            if !data.editor.as_ref().is_some_and(|e| e.run_inline) {
                let prompt=NSString::from_str(data.launcher_scope.map(|s|s.placeholder()).or_else(|| data.system_scope.map(|s|s.placeholder())).unwrap_or("Search apps, tabs or commands…"));
                state.input.setPlaceholderString(Some(&prompt));state.input.setAccessibilityLabel(Some(&prompt));
                state.empty.setStringValue(&NSString::from_str(if data.system_busy{"Loading…"}else if data.launcher_scope.is_none() && matches!(data.system_scope, None | Some(ruycast_native::system_search::Scope::MainTabs)){"No commands found"}else{"No results"}));
            }
        }
        state.launcher_back.setHidden(data.launcher_scope.is_none() || data.editor.is_some() || data.actions_open || state.session.borrow().script_result().is_some());
        if let Some(scope) = data.launcher_scope {
            state.heading.setStringValue(&NSString::from_str(scope.title()));
            state.footer.setFrame(rect(100., 14., state.panel.contentView().map_or(760., |v|v.bounds().size.width)-270., 22.));
        }
        if let Some(mode)=state.session.borrow().utility_scope(){
            if data.editor.is_none(){let prompt=NSString::from_str(match mode.as_str(){"utility-password-history"=>"Search password history…","utility-username-history"=>"Search username history…",_=>"Search text history…"});state.input.setPlaceholderString(Some(&prompt));state.input.setAccessibilityLabel(Some(&prompt));}
            state.empty.setStringValue(ns_string!("No history found"));
            if data.editor.is_none(){state.empty.setHidden(state.session.borrow().utility_busy()||!message.is_empty()||count>0);}
        }
        let dashboard=state.session.borrow().dashboard();
        state.process_controls.apply(self,state.session.borrow().process_state(),state.panel.contentView().map(|v|v.bounds().size.width).unwrap_or(760.),state.panel.contentView().map(|v|v.bounds().size.height).unwrap_or(490.));
        state.dashboard.apply(self,dashboard.clone());
        let settings=match &*state.session.borrow(){Model::Live{view,..}=>view.settings.clone(),_=>None};
        state.settings.apply(self,settings.clone().filter(|s|["general","advanced"].contains(&s.tab.as_str())));
        if !state.session.borrow().management(){let menu=match &*state.session.borrow(){Model::Live{view,..}=>view.menu_bar.clone(),_=>None};state.menu_bar.apply(self,menu,!state.panel.ivars().ordering_allowed);}
        if dashboard.is_some() || settings.is_some() {
            state.input.setHidden(true);state.heading.setHidden(true);state.scroll.setHidden(true);
            state.empty.setHidden(true);state.footer.setHidden(true);state.notice.setHidden(true);
            state.surface.form.setHidden(true);state.surface.menu.setHidden(true);
            state.surface.inline_clip.setHidden(true);state.surface.actions_button.setHidden(true);
        }
        {let model=state.session.borrow();state.table.setRowHeight(if model.clipboard().is_some(){58.}else{39.});state.clipboard_panel.apply(self,model.clipboard(),model.epoch(),model.dismissal(),&state.scroll,!state.panel.ivars().ordering_allowed);if model.clipboard().is_none()&&!model.management(){for column in state.table.tableColumns(){column.setHidden(false);if column.identifier().to_string()=="title"{column.setWidth(state.scroll.frame().size.width-260.);}}}
        if model.clipboard().is_some(){state.heading.setStringValue(&NSString::from_str("Clipboard"));state.input.setPlaceholderString(Some(&NSString::from_str("Search clipboard")));state.input.setAccessibilityLabel(Some(&NSString::from_str("Search clipboard")));state.footer.setStringValue(&NSString::from_str("Copy ⌘↵ · Paste ↵"));state.footer.setFrame(rect(100.,14.,state.panel.contentView().map_or(760.,|v|v.bounds().size.width)-270.,22.));state.surface.actions_button.setHidden(false);state.empty.setStringValue(&NSString::from_str(if model.query().is_empty(){"Clipboard is empty"}else{"No clipboard items found"}));state.empty.setFrame(rect(26.,230.,state.scroll.frame().size.width-20.,24.));state.empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
            for column in state.table.tableColumns(){let title=column.identifier().to_string()=="title";column.setHidden(!title);if title{column.setWidth(state.scroll.frame().size.width-16.);}}
            state.input.setAccessibilityLabel(Some(&NSString::from_str("Search clipboard")));}}
        if state.session.borrow().clipboard().is_some(){state.table.setFrameSize(NSSize::new(state.scroll.contentSize().width,state.table.frame().size.height));}
        let title_width = state.table.tableColumns().iter()
            .find(|column| column.identifier().to_string() == "title")
            .map(|column| column.width());
        // AppKit can resize the first column before reconcile. Rebuild its
        // compound cells from the final width instead of shrinking alias badges.
        if state.rendered_title_width.replace(title_width) != title_width {
            let selection = state.session.borrow().selection();
            state.table.reloadData();
            if let Some(index) = selection {
                state.table.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(index), false,
                );
            } else {
                unsafe { state.table.deselectAll(None); }
            }
        }
        if data.actions_open{if let Some(parent)=state.panel.contentView(){parent.addSubview_positioned_relativeTo(&state.surface.menu,NSWindowOrderingMode::Above,None);}}
        {
            let preferences=settings.as_ref().filter(|s|s.tab=="clipboard");
            let epoch=preferences.map_or(0,|s|s.epoch);let dismissal=preferences.map_or(0,|s|s.dismissal);
            let navigation=preferences.map_or(0,|s|s.navigation_revision);
            let owner=Weak::new(self);
            state.clipboard_preferences.authority([epoch,dismissal,navigation]);
            state.clipboard_preferences.apply(preferences.map(|s|&s.configuration["clipboard"]),preferences.map_or(0,|s|s.revision),preferences.is_some_and(|s|s.busy),std::rc::Rc::new(move|revision,value|{
                if let Some(owner)=owner.load(){owner.clipboard_settings_control(revision,epoch,dismissal,navigation,"clipboard",value);}
            }));
            if let Some(p)=preferences {
                let owner=Weak::new(self);let revision=p.revision;
                let status=p.clipboard.as_ref();
                state.clipboard_preferences.recording(status.map_or(0,|s|s.count),status.is_some_and(|s|s.paused),Some(std::rc::Rc::new(move||{
                    if let Some(owner)=owner.load(){owner.clipboard_settings_control(revision,epoch,dismissal,navigation,"clipboard-paused",serde_json::json!(false));}
                })));
                let message=status.and_then(|s|s.error()).unwrap_or(&p.message);
                state.clipboard_preferences.layout(self.management_graphite(),message);
                state.scroll.setHidden(true);state.input.setHidden(true);state.heading.setHidden(true);state.empty.setHidden(true);state.footer.setHidden(true);
            }
        }
        self.reconcile_snippet_hotkey();
        state.reconciling.set(false);
        self.process_dashboard_request();
        self.reconcile_performance_input();
        self.reconcile_outside_monitors();
    }
}

impl Controller {
    fn outside_binding(&self) -> Option<(u64, u64)> {
        let state = self.ivars();
        if state.failed.get() || state.outside_stopped.get() || state.panel.ivars().failed.get()
            || state.panel.ivars().management { return None; }
        let model = state.session.borrow();
        if !model.visible() { return None; }
        if let Model::Live { view, connection } = &*model {
            if connection.stop_requested() || connection.stopped()
                || connection.latest_dismissal() != view.dismissal { return None; }
        }
        // Query epochs govern data/actions, not ownership of the still-visible
        // presentation. An outside click must also cancel its pending query.
        Some((model.show_serial(), model.dismissal()))
    }

    fn stop_outside_monitors(&self) {
        let old = self.ivars().outside.borrow_mut().take();
        // No RefCell or model borrow spans AppKit's removal call.
        drop(old);
    }

    fn reconcile_outside_monitors(&self) {
        let binding = self.outside_binding();
        if self.ivars().outside.borrow().as_ref().map(|m| m.binding) == binding { return; }
        self.stop_outside_monitors();
        let Some(binding) = binding else { return; };
        let Some(generation) = self.ivars().outside_generation.get().checked_add(1) else {
            self.ivars().outside_stopped.set(true);
            return;
        };
        self.ivars().outside_generation.set(generation);
        *self.ivars().outside.borrow_mut() = Some(OutsideMonitors {
            generation, binding, global: None, local: None,
        });
        // Offscreen fixtures exercise the same owned callback route, never OS
        // event subscriptions or somebody else's input/window inventory.
        if !self.ivars().panel.ivars().ordering_allowed { return; }
        let owner = Weak::new(self);
        let global = block2::RcBlock::new(move |_: std::ptr::NonNull<NSEvent>| {
            if MainThreadMarker::new().is_none() { return; }
            if let Some(owner) = owner.load() {
                owner.guard_callback(|| owner.outside_click(generation, None, true));
            }
        });
        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global);
        let owner = Weak::new(self);
        let local = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            if MainThreadMarker::new().is_some() {
                if let Some(owner) = owner.load() {
                    owner.guard_callback(|| {
                        // AppKit owns this event for the callback's duration.
                        let event = unsafe { event.as_ref() };
                        owner.outside_local_event(generation, event);
                    });
                }
            }
            event.as_ptr()
        });
        let local = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) };
        // Registration can reenter. Publish handles only to their same live
        // generation; otherwise remove the newly obtained handles immediately.
        let mut pending = OutsideMonitors { generation, binding, global, local };
        if self.outside_binding() == Some(binding) {
            let mut slot = self.ivars().outside.borrow_mut();
            if let Some(current) = slot.as_mut().filter(|m| m.generation == generation) {
                current.global = pending.global.take();
                current.local = pending.local.take();
            }
        }
        drop(pending);
    }

    fn outside_click(&self, generation: u64, window: Option<&NSWindow>, global: bool) {
        let captured = self.ivars().outside.borrow().as_ref()
            .filter(|m| m.generation == generation).map(|m| m.binding);
        if captured.is_none() || captured != self.outside_binding() { return; }
        if !global {
            let own: &NSWindow = &self.ivars().panel;
            if window.is_some_and(|window| std::ptr::eq(window, own)) { return; }
            // The Rust action-key view is this exact owned SurfaceBackground,
            // normally inside our panel. No foreign view identifier/class name
            // can claim the action-menu exemption.
            if window.and_then(NSWindow::contentView)
                .is_some_and(|view| self.contains_action_menu(&view)) { return; }
        }
        self.stop_outside_monitors();
        self.ivars().inline_autosave_allowed.set(false);
        self.ivars().session.borrow_mut().hide_palette();
        self.ivars().surface.stop();
        self.ivars().panel.orderOut(None);
        self.reconcile();
    }

    fn contains_action_menu(&self, view: &NSView) -> bool {
        std::ptr::eq(view, &*self.ivars().surface.menu as &NSView)
            || view.subviews().iter().any(|child| self.contains_action_menu(&child))
    }

    fn outside_local_event(&self, generation: u64, event: &NSEvent) -> *mut NSEvent {
        if matches!(event.r#type(), NSEventType::LeftMouseDown | NSEventType::RightMouseDown) {
            self.outside_click(generation, event.window(self.mtm()).as_deref(), false);
        }
        std::ptr::from_ref(event).cast_mut()
    }
}

/// This guard outlives every native operation and runs on all early returns,
/// before the retained delegate owner is dropped. It cannot move off-main.
struct Cleanup<'a>(&'a Controller);
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        self.0.ivars().outside_stopped.set(true);
        self.0.stop_outside_monitors();
        self.0.cancel_performance_input();
        if let Model::Live { connection, .. } = &*self.0.ivars().session.borrow() { connection.cancel_open_trace(); }
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(self.0);
            if self.0.ivars().panel.ivars().ordering_allowed{NSWorkspace::sharedWorkspace().notificationCenter().removeObserver(self.0);}
        }
        self.0.ivars().inline_autosave.borrow_mut().take();
        self.0.ivars().dashboard.cleanup();
        self.0.ivars().settings.cleanup();
        self.0.ivars().menu_bar.cleanup();
        self.0.ivars().clipboard_preferences.cleanup();
        if let Some(monitor)=self.0.ivars().monitor.borrow().as_ref(){monitor.cleanup();}
        self.0.ivars().process_controls.cleanup();
        self.0.ivars().clipboard_panel.cleanup();
        self.0.ivars().result_panel.cleanup();
        self.0.retire_snippet_hotkey();
        self.0.ivars().surface.cleanup();
        if let Some(toolbar) = &self.0.ivars().toolbar {
            toolbar.cleanup();
        }
        *self.0.ivars().panel.ivars().owner.borrow_mut() = None;
        unsafe {
            self.0.ivars().input.setDelegate(None);
            self.0.ivars().table.setDataSource(None);
            self.0.ivars().table.setDelegate(None);
            self.0.ivars().table.setTarget(None);
            self.0.ivars().table.setAction(None);
            self.0.ivars().launcher_back.setTarget(None);
            self.0.ivars().launcher_back.setAction(None);
        }
        self.0.ivars().panel.close();
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn label(mtm: MainThreadMarker, text: &str, frame: NSRect, size: f64) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label
}

pub fn check(
    output: Option<&std::path::Path>,
    width: f64,
    dark: bool,
) -> Result<serde_json::Value, String> {
    autoreleasepool(|_| {
        let mtm = MainThreadMarker::new().ok_or("AppKit requires main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        #[cfg(feature = "test-tools")]
        crate::native_rows::check_cache(mtm)?;
        let controller = Controller::new(mtm, width, dark, Session::new(fixture(500)));
        let _cleanup = Cleanup(&controller);
        let state = controller.ivars();
        crate::native_edit::check(
            &state.panel,
            &*state.panel.contentView().ok_or("missing native content")?,
            mtm,
        )?;
        // Attach the native field editor to our offscreen, never-shown panel.
        assert!(!state.panel.isVisible());
        if !state.panel.makeFirstResponder(Some(&state.input)) {
            return Err("native first responder failed".into());
        }
        state.input.setStringValue(ns_string!("alias42"));
        let note = unsafe {
            NSNotification::notificationWithName_object(
                ns_string!("NSControlTextDidChangeNotification"),
                None,
            )
        };
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        if state.session.borrow().selected().map(|c| c.id) != Some("fixture.42") {
            return Err("native input did not reconcile".into());
        }
        let editor = state.input.currentEditor().ok_or("no field editor")?;
        let text = editor
            .downcast_ref::<NSTextView>()
            .ok_or("unexpected field editor")?;
        if !text.allowsUndo() {
            return Err("search field editor has no Undo support".into());
        }
        unsafe {
            use objc2_foundation::NSRange;
            let marked = ns_string!("に");
            let _: () = msg_send![text, setMarkedText: marked, selectedRange: NSRange::new(1,0), replacementRange: NSRange::new(usize::MAX,0)];
            let has_marked: bool = msg_send![text, hasMarkedText];
            if !has_marked {
                return Err("native IME composition was not established".into());
            }
            let handled: bool = msg_send![&*controller, control: &*state.input, textView: text, doCommandBySelector: sel!(insertNewline:)];
            if handled
                || state.session.borrow().activated_id().is_some()
                || !state.session.borrow().visible()
            {
                return Err("native IME Return incorrectly activated".into());
            }
            let _: () = msg_send![text, unmarkText];
        }
        state.input.setStringValue(ns_string!("alias42"));
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        unsafe {
            let _: bool = msg_send![&*controller, control: &*state.input, textView: text, doCommandBySelector: sel!(insertNewline:)];
        }
        if state.session.borrow().activated_id() != Some("fixture.42") {
            return Err("Return did not select exact item".into());
        }
        let search_label = ns_string!("Search apps, tabs or commands…");
        unsafe {
            let actual: Option<Retained<NSString>> = msg_send![&*state.input, accessibilityLabel];
            if actual.as_deref() != Some(search_label) {
                return Err("search accessibility name mismatch".into());
            }
        }
        // Verify real native rows/scrolling and selection notifications, not
        // just the Rust model. No mouse/global key event or desktop focus.
        state.input.setStringValue(ns_string!(""));
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        if state.table.numberOfRows() != 100 {
            return Err("native result cap mismatch".into());
        }
        for _ in 0..24 {
            unsafe {
                let _: bool = msg_send![&*controller, control: &*state.input, textView: text, doCommandBySelector: sel!(moveDown:)];
            }
        }
        let visible = state.table.rowsInRect(state.table.visibleRect());
        if state.table.selectedRow() != 24
            || visible.location > 24
            || visible.location + visible.length <= 24
        {
            return Err("keyboard selection did not scroll into view".into());
        }
        state
            .table
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(42), false);
        if state.session.borrow().selection() != Some(42) {
            return Err("native selection notification did not reach model".into());
        }
        let revision = state.rendered_revision.get();
        state.session.borrow_mut().move_selection(1);
        controller.reconcile();
        if state.rendered_revision.get() != revision {
            return Err("navigation reloaded native rows".into());
        }
        // Exact selected identity survives updates, but a removed selected row
        // does not activate a replacement at the same offset.
        let selected = state
            .session
            .borrow()
            .selected()
            .ok_or("missing selection")?
            .id
            .to_string();
        let mut commands = fixture(500);
        commands.rotate_left(10);
        state
            .session
            .borrow_mut()
            .replace_commands(commands.clone());
        controller.reconcile();
        if state.session.borrow().selected().map(|c| c.id) != Some(selected.as_str()) {
            return Err("native selection identity changed on reload".into());
        }
        commands.retain(|c| c.id != selected);
        state.session.borrow_mut().replace_commands(commands);
        controller.reconcile();
        if state.table.selectedRow() != -1 || state.session.borrow().selected().is_some() {
            return Err("removed native selection was substituted".into());
        }
        state.session.borrow_mut().key("insertNewline:", false);
        if state.session.borrow().activated_id().is_some() {
            return Err("removed native selection activated".into());
        }
        state
            .input
            .setStringValue(ns_string!("no matching command Ω"));
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        if state.table.numberOfRows() != 0
            || state.empty.isHidden()
            || !state.footer.stringValue().is_empty()
        {
            return Err("native empty state has stale results/action".into());
        }
        state
            .session
            .borrow_mut()
            .replace_commands(vec![fixture(43).remove(42)]);
        state.input.setStringValue(ns_string!("alias42"));
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        if state.table.numberOfRows() != 1 {
            return Err("mouse fixture must contain one native row".into());
        }
        state.session.borrow_mut().set_visible(true);
        let click = |point: NSPoint| -> Result<(), String> {
            let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDown,state.table.convertPoint_toView(point,None),NSEventModifierFlags::empty(),0.,state.panel.windowNumber(),None,1,1,1.).ok_or("local mouse event allocation failed")?;
            unsafe {
                let _: () = msg_send![&*state.table,mouseDown:&*event];
            }
            Ok(())
        };
        click(NSPoint::new(10., 120.))?;
        if state.session.borrow().activated_id().is_some() || !state.session.borrow().visible() {
            return Err("empty-space click activated selected command".into());
        }
        click(NSPoint::new(10., 15.))?;
        if state.session.borrow().activated_id() != Some("fixture.42") {
            return Err("local mouse click did not activate exact selected row".into());
        }
        if state
            .panel
            .firstResponder()
            .as_deref()
            .is_none_or(|r| !std::ptr::eq(r, &**text as &NSResponder))
        {
            return Err("row click stole text editor focus".into());
        }
        // Prepare a representative scrolling list for visual review.
        state.session.borrow_mut().replace_commands(fixture(500));
        state.input.setStringValue(ns_string!(""));
        unsafe {
            let _: () = msg_send![&*controller, controlTextDidChange: &*note];
        }
        state.session.borrow_mut().move_selection(24);
        controller.reconcile();
        snapshot(&controller, output)?;
        let screen_text = screen_text(&controller);
        if state.failed.get() {
            return Err("native callback failed".into());
        }
        // Separate fault owner: exercise recovery without making the normal
        // controller's partially changed state usable after a panic.
        let failed = Controller::new(mtm, width, dark, Session::new(fixture(500)));
        let _failed_cleanup = Cleanup(&failed);
        let caught = failed.guard_callback(|| panic!("isolated callback failure fixture"));
        if caught.is_some() || !failed.ivars().failed.get() {
            return Err("callback panic not recorded".into());
        }
        let mut ran_again = false;
        failed.guard_callback(|| {
            ran_again = true;
        });
        if ran_again {
            return Err("failed callback owner was reused".into());
        }
        Ok(
            serde_json::json!({"ok":true,"rustNativeAppKit":true,"nativeTextDelegate":true,
            "returnIntent":"fixture.42","externalExecution":false,"windowShown":false,"nativeIMEGuard":true,
            "width":width,"dark":dark,"searchAccessibilityLabel":"Search apps, tabs or commands…",
            "nativeMouseIntentAndEmptyClick":true,"nativeEditingAndUndoRedo":true,"nativeScrollableRows":true,"nativeSelectionNotifications":true,"reloadKeepsIdentity":true,"emptyActionHidden":true,
            "callbackFailureQuarantined":true,"cleanupOnEveryReturn":true,
            "screenText":screen_text,
            "scope":"isolated first native slice; complete product parity not implemented"}),
        )
    })
}

#[cfg(feature = "test-tools")]
fn inline_tracking_trace(controller: &Controller, phase: &str) {
    use std::io::Write;
    let state = controller.ivars();
    let path = state
        .probe_output
        .borrow()
        .as_ref()
        .map(|p| p.with_file_name("inline-tracking-trace.jsonl"));
    let Some(path) = path.filter(|p| p.exists()) else {
        return;
    };
    let mode = objc2_core_foundation::CFRunLoop::main()
        .and_then(|r| r.current_mode())
        .map(|m| m.to_string());
    let data = state.session.borrow().surface();
    let event = serde_json::json!({"phase":phase,"mode":mode,"pending":state.surface.pending.get(),"editorToken":data.editor.as_ref().map(|e|e.token),"editorSaving":data.editor.as_ref().is_some_and(|e|e.saving),"inline":state.surface.inline_visible.get()});
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
}
fn snapshot(controller: &Controller, output: Option<&std::path::Path>) -> Result<(), String> {
    let state = controller.ivars();
    if let Some(path) = output {
        let view = state.panel.contentView().ok_or("missing content")?;
        view.layoutSubtreeIfNeeded();
        let bitmap = view
            .bitmapImageRepForCachingDisplayInRect(view.bounds())
            .ok_or("render allocation failed")?;
        view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &objc2_foundation::NSDictionary::new(),
            )
        }
        .ok_or("PNG encode failed")?;
        std::fs::write(path, unsafe { data.as_bytes_unchecked() }).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn screen_text(controller: &Controller) -> Vec<String> {
    let state = controller.ivars();
    if state.settings.visible(){let mut text=state.toolbar.as_ref().map(|t|t.text()).unwrap_or_default();text.extend(state.settings.text_values());return text;}
    if !state.clipboard_preferences.root.isHidden(){let mut text=state.toolbar.as_ref().map(|t|t.text()).unwrap_or_default();text.extend(state.clipboard_preferences.text());return text;}
    if state.session.borrow().clipboard().is_some(){let model=state.session.borrow();let mut text=state.clipboard_panel.text();text.extend((0..model.row_count()).filter_map(|i|model.row(i).map(|r|r.title.to_owned())));text.extend([state.heading.stringValue().to_string(),state.input.placeholderString().map(|s|s.to_string()).unwrap_or_default(),state.footer.stringValue().to_string()]);if !state.empty.isHidden(){text.push(state.empty.stringValue().to_string());}
        if !state.notice.isHidden(){text.push(state.notice.stringValue().to_string());}return text;}
    if !state.dashboard.scroll.isHidden() {let mut text=state.toolbar.as_ref().map(|t|t.text()).unwrap_or_default();text.extend(state.dashboard.text_values());return text;}
    if !state.surface.form.isHidden() {
        let mut text = state.toolbar.as_ref().map(|t| t.text()).unwrap_or_default();
        text.extend([
            state.surface.heading.stringValue().to_string(),
            state.surface.field_label.stringValue().to_string(),
        ]);
        if !state.surface.command.scroll.isHidden() {
            text.extend(state.surface.command.screen_text());
        }
        text.extend(state.surface.template.visible_text());
        if !state.surface.alias.isHidden() {
            text.push(state.surface.alias.stringValue().to_string());
        }
        if !state.surface.record.isHidden() && state.surface.command.scroll.isHidden() {
            text.push(state.surface.record.title().to_string());
        }
        text.push(state.surface.cancel.title().to_string());
        if !state.surface.save.isHidden() {
            text.push(state.surface.save.title().to_string());
        }
        if !state.notice.isHidden() {
            text.push(state.notice.stringValue().to_string());
        }
        return text;
    }
    let mut screen_text = state.toolbar.as_ref().map(|t| t.text()).unwrap_or_default();
    if !state.launcher_back.isHidden() { screen_text.push(state.launcher_back.title().to_string()); }
    if state.table.headerView().is_some() {
        screen_text.extend(
            state
                .table
                .tableColumns()
                .iter()
                .map(|column| column.title().to_string()),
        );
    }
    if state.surface.inline_visible.get() {
        if !state.surface.alias.isHidden() && {
            let r = objc2_foundation::NSIntersectionRect(
                state.surface.alias.visibleRect(),
                state.surface.alias.bounds(),
            );
            r.size.width > 0. && r.size.height > 0.
        } {
            screen_text.push(state.surface.alias.stringValue().to_string());
        }
        if !state.surface.record.isHidden() && {
            let r = objc2_foundation::NSIntersectionRect(
                state.surface.record.visibleRect(),
                state.surface.record.bounds(),
            );
            r.size.width > 0. && r.size.height > 0.
        } {
            screen_text.push(state.surface.record.title().to_string());
        }
    }
    screen_text.extend([
        if state.input.isHidden() {
            String::new()
        } else {
            state.input.stringValue().to_string()
        },
        state.heading.stringValue().to_string(),
    ]);
    let shown = state.table.rowsInRect(state.table.visibleRect());
    for row in
        shown.location..(shown.location + shown.length).min(state.table.numberOfRows() as usize)
    {
        for column in 0..state.table.numberOfColumns() {
            if let Some(view) =
                state
                    .table
                    .viewAtColumn_row_makeIfNecessary(column, row as isize, true)
            {
                if view.isHidden() {
                    continue;
                }
                if let Some(button) = view.downcast_ref::<NSButton>() {
                    let title = button.title().to_string();
                    if !title.is_empty() {
                        screen_text.push(title);
                    }
                }
                if view.downcast_ref::<NSTextField>().is_none() && view.downcast_ref::<NSButton>().is_none() {
                    for child in view.subviews().iter(){if !child.isHidden(){if let Some(field)=child.downcast_ref::<NSTextField>(){let value=field.stringValue().to_string();if !value.is_empty(){screen_text.push(value);}}}}
                }
                if let Some(field) = view.downcast_ref::<NSTextField>() {
                    let value = field.stringValue().to_string();
                    if !value.is_empty() {
                        screen_text.push(value);
                    }
                } else if view.downcast_ref::<NSButton>().is_none() {
                    screen_text.extend(crate::native_rows::texts(&view));
                }
            }
        }
    }
    if !state.notice.isHidden() {
        screen_text.push(state.notice.stringValue().to_string());
    }
    if !state.empty.isHidden() {
        screen_text.push(state.empty.stringValue().to_string());
    }
    screen_text.extend(state.surface.template.visible_text());
    screen_text.extend(state.result_panel.screen_text());
    if !state.footer.isHidden() && !state.footer.stringValue().is_empty() {
        screen_text.push(state.footer.stringValue().to_string());
    }
    let snippet_button=state.snippet_hotkey.borrow().as_ref().map(|c|c.button.clone());
    if let Some(button)=snippet_button.filter(|b|!b.isHidden()){screen_text.push(button.title().to_string());}
    if !state.surface.actions_button.isHidden() {
        screen_text.push(state.surface.actions_button.title().to_string());
    }
    if !state.surface.menu.isHidden() {
        screen_text.push(state.surface.menu_heading.stringValue().to_string());
        for button in state.surface.menu_buttons() {
            if !button.isHidden() {
                screen_text.push(button.title().to_string());
            }
        }
    }
    screen_text
}

pub fn check_profile(
    paths: ruycast_native::profile::ProfilePaths,
    output: &std::path::Path,
    width: f64,
    dark: bool,
    query: &str,
) -> Result<serde_json::Value, String> {
    // Secure instance ownership before configuration initialization, exactly as
    // the headless owner does. A running profile is never borrowed by a probe.
    let (tx, _rx) = std::sync::mpsc::sync_channel(32);
    let _lease =
        ruycast_native::control::ControlServer::start(&paths.root, tx, Default::default())?;
    let host = ruycast_native::state::HostState::open(paths)?;
    let commands = host.configuration.current().commands.len();
    autoreleasepool(|_| {
        let mtm = MainThreadMarker::new().ok_or("AppKit requires main thread")?;
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        #[cfg(feature = "test-tools")]
        crate::native_rows::check_cache(mtm)?;
        let controller = Controller::new(mtm, width, dark, host.session);
        let _cleanup = Cleanup(&controller);
        let state = controller.ivars();
        state.input.setStringValue(&NSString::from_str(query));
        state.session.borrow_mut().set_query(query);
        controller.reconcile();
        snapshot(&controller, Some(output))?;
        if state.failed.get() || state.panel.isVisible() {
            return Err("profile native probe failed".into());
        }
        let session = state.session.borrow();
        Ok(
            serde_json::json!({"ok":true,"productionReady":false,"windowShown":false,"externalExecution":false,
            "configuredCommands":commands,"nativeRows":state.table.numberOfRows(),"selectedId":session.selected().map(|c|c.id),
            "screenText":screen_text(&controller),"accessibilityText":accessibility_text(&controller),
            "width":width,"dark":dark,"scope":"configured Rust launcher list; production actions/runtime remain incomplete"}),
        )
    })
}
fn accessibility_text(controller: &Controller) -> Vec<String> {
    let state = controller.ivars();
    if state.settings.visible(){let mut text=state.toolbar.as_ref().map(|t|t.accessibility_text()).unwrap_or_default();text.extend(state.settings.text_values());return text;}
    if !state.clipboard_preferences.root.isHidden(){let mut text=state.toolbar.as_ref().map(|t|t.accessibility_text()).unwrap_or_default();text.extend(state.clipboard_preferences.accessibility());return text;}
    if state.session.borrow().clipboard().is_some(){let mut text=state.clipboard_panel.accessibility();for view in [&*state.heading as &NSView,&*state.input as &NSView,&*state.footer as &NSView,&*state.empty as &NSView]{if !view.isHidden(){if let Some(label)=view.accessibilityLabel(){text.push(label.to_string());}}}return text;}
    if !state.dashboard.scroll.isHidden() {let mut text=state.toolbar.as_ref().map(|t|t.accessibility_text()).unwrap_or_default();text.extend(state.dashboard.accessibility_values());return text;}
    if !state.surface.form.isHidden() {
        let mut result = state
            .toolbar
            .as_ref()
            .map(|t| t.accessibility_text())
            .unwrap_or_default();
        result.extend([
            state.surface.heading.stringValue().to_string(),
            state.surface.field_label.stringValue().to_string(),
        ]);
        if !state.surface.command.scroll.isHidden() {
            result.extend(state.surface.command.accessibility_text());
        }
        result.extend(state.surface.template.accessibility_text());
        for view in [
            &*state.surface.alias as &NSView,
            &*state.surface.record as &NSView,
            &*state.surface.save as &NSView,
            &*state.surface.cancel as &NSView,
        ] {
            let hidden_by_picker = state.surface.command.app_list.is_open()
                && std::ptr::eq(view, &*state.surface.record as &NSView);
            if !view.isHidden() && !hidden_by_picker {
                if let Some(label) = view.accessibilityLabel() {
                    result.push(label.to_string());
                }
            }
        }
        if !state.notice.isHidden() {
            result.push(state.notice.stringValue().to_string());
        }
        return result;
    }
    let mut result = state
        .toolbar
        .as_ref()
        .map(|t| t.accessibility_text())
        .unwrap_or_default();
    if state.table.headerView().is_some() {
        result.extend(
            state
                .table
                .tableColumns()
                .iter()
                .map(|column| column.title().to_string()),
        );
    }
    if state.surface.inline_visible.get() {
        for view in [
            &*state.surface.alias as &NSView,
            &*state.surface.record as &NSView,
        ] {
            if !view.isHidden() {
                if let Some(label) = view.accessibilityLabel() {
                    result.push(label.to_string());
                }
            }
        }
    }
    for view in [&*state.input as &NSView, &*state.table as &NSView] {
        if view.isHidden() {
            continue;
        }
        if let Some(label) = view.accessibilityLabel() {
            result.push(label.to_string());
        }
    }
    for view in [
        &*state.heading as &NSView,
        &*state.footer as &NSView,
        &*state.surface.actions_button as &NSView,
        &*state.launcher_back as &NSView,
    ] {
        if !view.isHidden() {
            if let Some(label) = view.accessibilityLabel() {
                if !label.is_empty() {
                    result.push(label.to_string());
                }
            }
        }
    }
    if !state.surface.menu.isHidden() {
        for button in state.surface.menu_buttons() {
            if !button.isHidden() {
                if let Some(label) = (&*button as &NSView).accessibilityLabel() {
                    result.push(label.to_string());
                }
            }
        }
    }
    let rows = state.table.rowsInRect(state.table.visibleRect());
    for row in rows.location..(rows.location + rows.length).min(state.table.numberOfRows() as usize)
    {
        for column in 0..state.table.numberOfColumns() {
            if let Some(view) =
                state
                    .table
                    .viewAtColumn_row_makeIfNecessary(column, row as isize, true)
            {
                if view.isHidden() {
                    continue;
                }
                if let Some(label) = view.accessibilityLabel() {
                    if !label.is_empty() {
                        result.push(label.to_string());
                    }
                }
            }
        }
    }
    result.extend(state.surface.template.visible_text());
    result.extend(state.result_panel.accessibility_text());
    let snippet_button=state.snippet_hotkey.borrow().as_ref().map(|c|c.button.clone());
    if let Some(button)=snippet_button.filter(|b|!b.isHidden()){
        if let Some(label)=NSAccessibility::accessibilityLabel(&*button){result.push(label.to_string());}
    }
    result
}

/// Native development runtime: one worker HostState/session, immutable main
/// view snapshots and a source-driven AppKit loop. Startup never opens a panel.
#[cfg(feature = "test-tools")]
pub fn run_owned(
    host: ruycast_native::owned_host_entry::QualifiedOwnedHost,
) -> Result<serde_json::Value, String> {
    // Admission comes from the independently pinned constructor. Keep the
    // production graph: hidden mode changes services and permission handling.
    let paths = host.paths().clone();
    let fixture_epoch = host.fixture_epoch_unix_seconds()?;
    let (fixture, authority, sdk) = host.ready_facts();
    let bindings = crate::native_f1::ReadyBindings {
        fixture_sha256: fixture.into(), authority_sha256: authority.into(),
        sdk_guard_sha256: sdk.into(), clock_resolution_ns: owned_f1_clock_resolution()?,
    };
    run_graph(paths, false, Some(OwnedMode { host, bindings, fixture_epoch }))
}

pub fn run(
    paths: ruycast_native::profile::ProfilePaths,
    hidden: bool,
) -> Result<serde_json::Value, String> {
    run_graph(paths, hidden, None)
}

#[cfg(feature = "test-tools")]
struct OwnedMode {
    host: ruycast_native::owned_host_entry::QualifiedOwnedHost,
    bindings: crate::native_f1::ReadyBindings,
    fixture_epoch: u64,
}
#[cfg(not(feature = "test-tools"))]
type OwnedMode = ();

fn run_graph(
    paths: ruycast_native::profile::ProfilePaths,
    hidden: bool,
    owned: Option<OwnedMode>,
) -> Result<serde_json::Value, String> {
    #[cfg(not(feature = "test-tools"))]
    let _ = owned;
    if hidden && !cfg!(feature = "test-tools") {
        return Err("Hidden native runtime requires the test-tools feature.".into());
    }
    autoreleasepool(|_| {
        use ruycast_native::connected::Connection;
        use std::sync::mpsc;
        let mtm = MainThreadMarker::new().ok_or("AppKit requires main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(if hidden {
            NSApplicationActivationPolicy::Prohibited
        } else {
            NSApplicationActivationPolicy::Accessory
        });
        let _edit_menu = crate::native_edit_menu::install(mtm);
        let (tx, rx) = mpsc::sync_channel(32);
        let clipboard = std::rc::Rc::new(crate::native_services::Clipboard::new(mtm, &paths, hidden)?);
        let _clipboard_watch = clipboard.watch();
        let (services_client, services) = ruycast_native::main_services::channel();
        services.system.fixture(hidden);
        #[cfg(feature = "test-tools")]
        services.system.require_fixture(hidden);
        let clipboard_monitor=std::rc::Rc::new(RefCell::new(crate::native_services::ClipboardMonitor::new(mtm,clipboard.clone(),services_client.clipboard_capture.clone(),tx.clone())));
        let paste_client = services_client.paste();
        let mut paste = crate::native_paste::Service::new(
            mtm,
            &paths,
            false,
            hidden,
            std::sync::Arc::new(move || paste_client.notify()),
            clipboard.own_writes(),
        );
        let hide_trace = hidden.then(|| paths.root.join("native-hide.jsonl"));
        let connection = Connection::new(tx.clone(), hidden);
        #[cfg(feature = "test-tools")]
        let owned = owned.map(|mode| {
            let (runtime, inspection) = ruycast_native::runtime::owned_channel(mode.host, tx.clone());
            (runtime, inspection, mode.bindings, mode.fixture_epoch)
        });
        #[cfg(feature = "test-tools")]
        let (f1_requests, f1_receiver) = mpsc::sync_channel(1);
        #[cfg(feature = "test-tools")]
        let f1_owner = std::rc::Rc::new(RefCell::new(OwnedCoordinator { requests: f1_receiver, pending: None }));
        #[cfg(feature = "test-tools")]
        let source_f1_owner = f1_owner.clone();
        #[cfg(feature = "test-tools")]
        let endpoint_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(feature = "test-tools")]
        let source_endpoint_cancel = endpoint_cancel.clone();
        #[cfg(feature = "test-tools")]
        let failed_endpoint_cancel = endpoint_cancel.clone();
        let management_connection = Connection::new_management(tx.clone(), hidden);
        Connection::link_windows(&connection, &management_connection);
        #[cfg(not(feature = "test-tools"))]
        let wants_fixture = false;
        #[cfg(feature = "test-tools")]
        let wants_fixture =
            hidden && std::env::var("RUYCAST_NATIVE_TEST_KEYBOARD").as_deref() == Ok("1");
        #[cfg(feature = "test-tools")]
        if wants_fixture
            && (!ruycast_native::workspace::fixture_installed(&paths.home)
                || std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME")
                    .and_then(|p| std::fs::canonicalize(p).ok())
                    != std::fs::canonicalize(&paths.home).ok())
        {
            return Err("Owned keyboard fixture home is required.".into());
        }
        let keyboard = if !hidden || wants_fixture {
            Some(crate::native_keyboard::Service::new(
                mtm,
                hidden,
                tx.clone(),
                connection.clone(),
                clipboard.own_writes(),
            )?)
        } else {
            None
        };
        if let Some(keyboard) = &keyboard {
            services_client.attach_keyboard(keyboard.client());
        }

        let management =
            Controller::new(mtm, 900., false, Model::live(management_connection.clone()));
        let _management_cleanup = Cleanup(&management);


        let width = 760.;
        #[cfg(feature = "test-tools")]
        let width = if hidden {
            std::env::var("RUYCAST_NATIVE_TEST_WIDTH")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|w| [600., 760.].contains(w))
                .unwrap_or(width)
        } else {
            width
        };
        let controller = Controller::new(mtm, width, false, Model::live(connection.clone()));
        let _cleanup = Cleanup(&controller);
        #[cfg(feature="test-tools")]
        if hidden {let board=clipboard.clone();let port=services_client.clipboard_capture.clone();let events=tx.clone();*controller.ivars().clipboard_fixture.borrow_mut()=Some(std::rc::Rc::new(move|value|board.fixture(value,&port,&events)));}
        if let Some(keyboard) = &keyboard {
            *controller.ivars().keyboard.borrow_mut() = Some(keyboard.client());
            *management.ivars().keyboard.borrow_mut() = Some(keyboard.client());
        }

        *controller.ivars().peer.borrow_mut() = Some(Weak::new(&*management));
        *management.ivars().peer.borrow_mut() = Some(Weak::new(&*controller));
        app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        struct DelegateCleanup<'a>(&'a NSApplication);
        impl Drop for DelegateCleanup<'_> {
            fn drop(&mut self) {
                self.0.setDelegate(None);
            }
        }
        let _delegate_cleanup = DelegateCleanup(&app);
        controller.ivars().panel.setAppearance(None);
        management.ivars().panel.setAppearance(None);
        if hidden {
            *controller.ivars().probe_output.borrow_mut() =
                Some(paths.root.join("native-probe.png"));
            *management.ivars().probe_output.borrow_mut() =
                Some(paths.root.join("native-settings-probe.png"));
            controller
                .ivars()
                .panel
                .makeFirstResponder(Some(&controller.ivars().input));
        }
        let windows = vec![
            (controller.clone(), connection.clone()),
            (management.clone(), management_connection.clone()),
        ];
        let application = app.clone();
        let fail_connection = connection.clone();
        let fail_app = app.clone();
        let fail_windows = [controller.clone(), management.clone()];
        let keyboard_wake = keyboard.as_ref().map(|k| k.client());
        let keyboard = std::rc::Rc::new(RefCell::new(keyboard));
        let recovery_keyboard = keyboard.clone();

        let source_clipboard_monitor=clipboard_monitor.clone();
        #[cfg(feature = "test-tools")]
        let _alias_probe_gap_cleanup = AliasProbeGapCleanup;
        let source = crate::main_loop::Source::new(
            mtm,
            move || {
                #[cfg(feature = "test-tools")]
                if windows[0].0.ivars().source_failure.replace(false) {
                    panic!("owned source callback failure");
                }
                // Normal writer quit may revoke Client before Connection::finish.
                let (keyboard_failed, keyboard_stopped) = keyboard
                    .borrow()
                    .as_ref()
                    .map_or((false, false), |k| (k.failed(), !k.running()));
                if windows[0].1.stop_requested()
                    || windows[0].1.stopped()
                    || keyboard_failed
                    || keyboard_stopped
                {
                    #[cfg(feature = "test-tools")]
                    source_endpoint_cancel.store(true, std::sync::atomic::Ordering::Release);
                    if keyboard_failed && !windows[0].1.stop_requested() {
                        windows[0].0.ivars().failed.set(true);
                    }
                    revoke_and_hide(&[&windows[0].0, &windows[1].0]);
                    let keyboard_retired = keyboard
                        .borrow_mut()
                        .as_mut()
                        .is_none_or(|keyboard| keyboard.stop());
                    source_clipboard_monitor.borrow_mut().stop();
                    paste.stop();
                    for ticket in services.take_pastes() {
                        let outcome = ticket.authority.dispatch.retire();
                        ticket.finish(outcome);
                    }
                    if windows[0].1.stopped() && keyboard_retired {
                        stop_application(&application);
                    }
                    return;
                }
                let mut keyboard_owner = keyboard.borrow_mut();
                if let Some(keyboard) = keyboard_owner.as_mut() {
                    keyboard.pump();
                    if keyboard.failed() {
                        windows[0].0.ivars().failed.set(true);
                        windows[0].1.request_stop();
                        return;
                    }
                }
                if let Err(error)=source_clipboard_monitor.borrow_mut().pump(){windows[0].1.request_stop();panic!("{error}");}
                let icons_changed=crate::native_rows::poll();
                #[cfg(feature = "test-tools")]
                source_f1_owner.borrow_mut().pump(&windows[0].0, &windows[0].1, false,services.owned_pending());
                for (current, connected) in &windows {
                    if icons_changed&&current.ivars().session.borrow().visible(){current.ivars().table.reloadData();current.reconcile();}
                    let view = connected.view();
                    if view.visible
                        && !current.ivars().session.borrow().visible()
                        && !connected.open_hide_masks(view.epoch)
                    {
                        paste.remember_target();
                        services.system.remember();
                    }
                }
                services.drain_with_prefill(|| clipboard.read(), |token| {
                    let ui = token.ui.upgrade().ok_or("Window owner stopped.")?;
                    let (current, _) = windows.iter().find(|(_, connected)| std::sync::Arc::ptr_eq(connected, &ui)).ok_or("Window owner changed.")?;
                    let before = current.ivars().panel.isVisible();
                    let result = ui.acknowledge_open_hide(&token.key, || {
                        current.ivars().session.borrow_mut().set_visible(false);
                        current.ivars().panel.orderOut(None);
                        !current.ivars().panel.isVisible()
                    });
                    if result.is_ok() {
                        if let Some(path) = &hide_trace {
                            use std::io::Write;
                            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e|e.to_string())?;
                            writeln!(file,"{}",serde_json::json!({"trace":token.key.trace,"serial":token.key.serial,"epoch":token.key.epoch,"intent":token.key.intent,"dismissal":token.key.dismissal,"mainThread":MainThreadMarker::new().is_some(),"beforeVisible":before,"afterVisible":current.ivars().panel.isVisible()})).map_err(|e|e.to_string())?;
                        }
                    }
                    result
                }, ||clipboard.prefill());
                let result = paste.pump(services.take_pastes(), |token| {
                    let ui = token.ui.upgrade().ok_or("Window owner stopped.")?;
                    let (current, _) = windows
                        .iter()
                        .find(|(_, connected)| std::sync::Arc::ptr_eq(connected, &ui))
                        .ok_or("Window owner changed.")?;
                    let before=current.ivars().panel.isVisible();
                    let result=ui.acknowledge_open_hide(&token.key, || {
                        current.ivars().session.borrow_mut().set_visible(false);
                        current.ivars().panel.orderOut(None);
                        !current.ivars().panel.isVisible()
                    });
                    if result.is_ok() {
                        if let Some(path)=&hide_trace {
                            use std::io::Write;
                            let mut file=std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e|e.to_string())?;
                            writeln!(file,"{}",serde_json::json!({"kind":"snippet","trace":token.key.trace,"serial":token.key.serial,"epoch":token.key.epoch,"intent":token.key.intent,"dismissal":token.key.dismissal,"mainThread":MainThreadMarker::new().is_some(),"beforeVisible":before,"afterVisible":current.ivars().panel.isVisible()})).map_err(|e|e.to_string())?;
                        }
                    }
                    result.map(|_|())
                });
                if result.is_err() {
                    paste.stop();
                    windows[0].0.ivars().failed.set(true);
                    windows[0].1.request_stop();
                    return;
                }
                if windows[0].1.stop_requested() || windows[0].1.stopped() {
                    drop(keyboard_owner);
                    revoke_and_hide(&[&windows[0].0, &windows[1].0]);
                    let retired = keyboard
                        .borrow_mut()
                        .as_mut()
                        .is_none_or(|keyboard| keyboard.stop());
                    if windows[0].1.stopped() && retired {
                        stop_application(&application);
                    }
                    return;
                }
                for (current, connected) in &windows {
                    current
                        .ivars()
                        .timers
                        .borrow_mut()
                        .retain(|timer| timer.valid());
                    if connected.latest_dismissal() > current.ivars().session.borrow().dismissal() {
                        if current.ivars().session.borrow().visible() {
                            connected.note_local_hide();
                        }
                        current.ivars().session.borrow_mut().set_visible(false);
                        current.ivars().panel.orderOut(None);
                    }
                    let next = connected.view();
                    #[cfg(feature = "test-tools")]
                    let mut gap_applied = false;
                    if next.revision > connected.applied_revision() && apply_connected_view(current, connected, next, hidden, &application) {
                        #[cfg(feature = "test-tools")]
                        { gap_applied = true; }
                    }
                    #[cfg(feature = "test-tools")]
                    let gap = match alias_gap_pause(current, connected, hidden, gap_applied) {
                        Ok(gap) => gap,
                        Err(_) => { connected.request_stop(); return; }
                    };
                    let requests = connected.take_probes();
                    #[cfg(feature = "test-tools")]
                    if let Some(gap) = &gap {
                        if alias_gap_taken(gap, current, connected, &requests).is_err() { connected.request_stop(); return; }
                    }
                    for request in requests {
                        if !request.context.is_live() {
                            continue;
                        }
                        #[cfg(feature = "test-tools")]
                        {
                            // Only the one proved, explicitly stale Alias fixture
                            // keeps the old dispatch as the paired negative.
                            let captured_stale = gap.as_ref().is_some_and(|g| matches!(g.dispatch, AliasGapDispatch::CapturedStale));
                            let deferred_input = gap.is_none() && deferred_input_probe(current, connected, hidden, &request.value);
                            if !captured_stale && !deferred_input {
                                if let Err(error) = acknowledge_probe_view(current, connected, hidden, &application) {
                                    let _ = request.respond(serde_json::json!({"ok":false,"error":error}));
                                    continue;
                                }
                            }
                            if !request.context.is_live() { continue; }
                            if let Some(gap) = &gap {
                                let witness = alias_gap_scalars(current, connected).and_then(|scalars|alias_gap_write(&gap.directory,c"alias-main-gap-dispatch.json",&serde_json::json!({"acknowledged":!captured_stale,"beforeInput":scalars})));
                                if witness.is_err() { connected.request_stop(); return; }
                            }
                        }
                        let value = current
                        .guard_callback(|| native_probe(current, connected, &request.value))
                        .unwrap_or_else(
                            || serde_json::json!({"ok":false,"error":"Native callback failed."}),
                        );
                        let _ = request.respond(value);
                    }
                    if current.ivars().failed.get()
                        || current.ivars().panel.ivars().failed.get()
                        || current.ivars().table.ivars().get()
                    {
                        connected.request_stop();
                    }
                }
                if let Some(keyboard) = keyboard_owner.as_mut() {
                    keyboard.pump();
                }
                #[cfg(feature = "test-tools")]
                source_f1_owner.borrow_mut().pump(&windows[0].0, &windows[0].1, true,services.owned_pending());
                for (current, _) in &windows {
                    current.ivars().surface.record.finish_pending();
                    for r in &current.ivars().settings.records {r.finish_pending();}
                }
            },
            move || {
                #[cfg(feature = "test-tools")]
                failed_endpoint_cancel.store(true, std::sync::atomic::Ordering::Release);
                revoke_and_hide(&[&fail_windows[0], &fail_windows[1]]);
                let retired = recovery_keyboard
                    .borrow_mut()
                    .as_mut()
                    .is_none_or(|keyboard| keyboard.stop());
                if fail_connection.stopped() && retired {
                    stop_application(&fail_app);
                }
            },
        )?;
        let signal = source.signal();
        #[cfg(feature = "test-tools")]
        let f1_signal = signal.clone();
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> =
            std::sync::Arc::new(move || signal.wake());
        connection.attach_wake(wake.clone());
        management_connection.attach_wake(wake.clone());
        if let Some(keyboard) = keyboard_wake {
            keyboard.attach_wake(wake.clone());
        }
        services_client.attach_wake(wake.clone());
        crate::native_rows::start(wake.clone(),hidden);
        let _icon_cleanup=crate::native_rows::Cleanup;
        clipboard_monitor.borrow_mut().start(wake)?;
        let worker_connection = connection.clone();
        #[cfg(feature = "test-tools")]
        let (owned_runtime, f1_endpoint) = match owned {
            Some((runtime, inspection, bindings, fixture_epoch)) => (Some(runtime), Some(NativeEndpoint {
                requests: f1_requests, signal: f1_signal, connection: connection.clone(),
                inspection, bindings, fixture_epoch, root: paths.root.clone(),
                cancel: endpoint_cancel.clone(),
                admitted: std::collections::BTreeSet::new(),
                reload: None,
                process_prepared: None,
            })),
            None => (None, None),
        };
        let worker = std::thread::Builder::new()
            .name("ruycast-runtime".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    #[cfg(feature = "test-tools")]
                    if let Some(runtime) = owned_runtime {
                        return runtime.run(tx, rx, worker_connection.clone(), services_client);
                    }
                    ruycast_native::runtime::run_with_services(
                        paths,
                        tx,
                        rx,
                        Some(worker_connection.clone()),
                        hidden,
                        Some(services_client),
                    )
                }))
                .unwrap_or_else(|_| Err("Rust runtime worker failed.".into()));
                worker_connection.finish(result);
            })
            .map_err(|e| e.to_string())?;
        #[cfg(feature = "test-tools")]
        let endpoint_worker = f1_endpoint.map(|mut endpoint| {
            std::thread::Builder::new().name("ruycast-f1-endpoint".into()).spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    use std::os::fd::AsFd;
                    let input=std::io::stdin(); let output=std::io::stdout();
                    let reader=crate::native_f1::borrowed_stdio::Reader::new(input.as_fd(),endpoint.cancel.clone()).map_err(|e|e.to_string())?;
                    let writer=crate::native_f1::borrowed_stdio::Writer::new(output.as_fd(),endpoint.cancel.clone()).map_err(|e|e.to_string())?;
                    let mut frames = crate::native_f1::Frames::new(reader,writer);
                    crate::native_f1::serve(&mut endpoint, &mut frames)
                })).unwrap_or_else(|_| Err("Owned endpoint panicked.".into()));
                endpoint.connection.request_stop();
                result
            }).map_err(|e|e.to_string())
        }).transpose()?;
        app.run();
        #[cfg(feature = "test-tools")]
        endpoint_cancel.store(true, std::sync::atomic::Ordering::Release);
        crate::native_rows::stop();
        // Never wait for a wire acknowledgement or join workers in a callback.
        // The normal exit wakes this loop only after runtime teardown finishes.
        if !connection.stopped() {
            connection.request_stop();
        }
        worker
            .join()
            .map_err(|_| "Rust runtime worker failed.".to_string())?;
        #[cfg(feature = "test-tools")]
        if let Some(endpoint) = endpoint_worker {
            endpoint.join().map_err(|_| "Owned endpoint worker failed.".to_string())??;
        }
        if source.failed()
            || [&*controller, &*management].iter().any(|c| {
                c.ivars().failed.get()
                    || c.ivars().panel.ivars().failed.get()
                    || c.ivars().table.ivars().get()
            })
        {
            return Err("Native run-loop callback failed.".into());
        }
        connection
            .take_outcome()
            .unwrap_or_else(|| Err("Missing runtime outcome.".into()))
    })
}
#[cfg(feature = "test-tools")]
fn owned_f1_clock_resolution() -> Result<u64, String> {
    let mut resolution = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: one initialized timespec output; no platform object or data read.
    if unsafe { libc::clock_getres(libc::CLOCK_MONOTONIC, &mut resolution) } != 0 {
        return Err("Could not observe the endpoint clock resolution.".into());
    }
    u64::try_from(resolution.tv_sec).ok().and_then(|s|s.checked_mul(1_000_000_000))
        .and_then(|s|u64::try_from(resolution.tv_nsec).ok().and_then(|n|s.checked_add(n)))
        .filter(|n|*n>0).ok_or_else(||"Invalid endpoint clock resolution.".into())
}

#[cfg(feature = "test-tools")]
enum OwnedOperation {
    Query(crate::native_f1::QueryStage, String),
    Action(crate::native_f1::UiAction),
    Hide,
    Snapshot,
}
#[cfg(feature = "test-tools")]
struct OwnedRequest {
    operation: OwnedOperation,
    deadline: std::time::Instant,
    reply: std::sync::mpsc::SyncSender<Result<serde_json::Value,String>>,
}
#[cfg(feature = "test-tools")]
struct OwnedQuery {
    request: OwnedRequest,
    epoch: u64,
    catalog_revision: u64,
    selected_id: Option<String>,
}
#[cfg(feature = "test-tools")]
struct OwnedCoordinator {
    requests: std::sync::mpsc::Receiver<OwnedRequest>,
    pending: Option<OwnedQuery>,
}
#[cfg(feature = "test-tools")]
impl OwnedCoordinator {
    /// Called before and after the ordinary apply_view pass. Search observes
    /// publication; reconciliation/input additionally require the actual ack.
    /// This callback never waits for the runtime or performs a wire request.
    fn pump(&mut self, owner: &Controller, connection: &ruycast_native::connected::Connection, applied: bool, main_services_pending:usize) {
        if self.pending.as_ref().is_some_and(|p|std::time::Instant::now()>=p.request.deadline) {
            let pending=self.pending.take().expect("Expired UI request exists.");
            let _=pending.request.reply.try_send(Err("Owned UI request expired.".into()));
        }
        if self.pending.is_none() {
            if let Ok(request) = self.requests.try_recv() {
                if std::time::Instant::now() >= request.deadline || connection.stop_requested() {
                    let _=request.reply.try_send(Err("Owned UI request expired or stopped.".into()));
                } else {
                    match &request.operation {
                        OwnedOperation::Query(stage, query) => {
                            if *stage == crate::native_f1::QueryStage::Input {
                                let Some(editor)=owner.ivars().input.currentEditor() else {
                                    let _=request.reply.try_send(Err("The production search field has no active input client.".into()));
                                    return;
                                };
                                owner.ivars().input.setStringValue(&NSString::from_str(query));
                                editor.setString(&NSString::from_str(query));
                                let note = unsafe { NSNotification::notificationWithName_object(
                                    ns_string!("NSControlTextDidChangeNotification"), Some(&*owner.ivars().input)) };
                                unsafe { let _:()=msg_send![owner,controlTextDidChange:&*note]; }
                            } else { connection.query(query); }
                            self.pending=Some(OwnedQuery{epoch:connection.latest_epoch(),catalog_revision:0,selected_id:None,request});
                        }
                        OwnedOperation::Action(action) => {
                            let view=connection.view();
                            if !view.visible||view.editor.is_some()||view.script_result.is_some(){
                                let _=request.reply.try_send(Err("The production palette cannot accept this action.".into()));
                                return;
                            }
                            let selected_id=match action {
                                crate::native_f1::UiAction::Select{index}=>{
                                    let Some(row)=view.rows.get(*index).filter(|r|!r.is_group) else {
                                        let _=request.reply.try_send(Err("The production selection is unavailable.".into()));return;
                                    };
                                    owner.ivars().session.borrow_mut().select_row(Some(*index));
                                    Some(row.id.clone())
                                }
                                crate::native_f1::UiAction::Actions{open}=>{
                                    if view.actions_open!=*open {owner.ivars().session.borrow_mut().toggle_actions();}
                                    None
                                }
                            };
                            self.pending=Some(OwnedQuery{epoch:view.epoch,catalog_revision:view.catalog_revision,selected_id,request});
                        }
                        OwnedOperation::Hide => {
                            owner.stop_outside_monitors();
                            owner.ivars().inline_autosave_allowed.set(false);
                            owner.ivars().session.borrow_mut().hide_palette();
                            owner.ivars().surface.stop();
                            owner.ivars().panel.orderOut(None);
                            owner.reconcile();
                            self.pending=Some(OwnedQuery{epoch:connection.latest_epoch(),catalog_revision:0,selected_id:None,request});
                        }
                        OwnedOperation::Snapshot => {
                            let view=connection.view();
                            let rows=view.rows.iter().map(|r|serde_json::json!({
                                "id":r.id,"title":r.title,"kind":r.kind,"keyword":r.keyword,
                                "aliases":r.aliases,"hotkey":r.hotkey,"enabled":r.enabled,
                                "action":r.action,"is_group":r.is_group,"detail":r.detail
                            })).collect::<Vec<_>>();
                            let actions=view.action_items.iter().map(|a|serde_json::json!({
                                "id":a.id,"title":a.title,"shortcut":a.shortcut
                            })).collect::<Vec<_>>();
                            let _=request.reply.try_send(Ok(serde_json::json!({
                                "display_rows":rows,"selection":view.selection,
                                "actions":actions,"actionsOpen":view.actions_open,
                                "query":view.query,"message":view.message,
                                "paletteVisible":owner.ivars().panel.isVisible(),
                                "modelVisible":view.visible,"nativeInput":owner.ivars().input.stringValue().to_string(),
                                "revision":view.revision,"appliedRevision":connection.applied_revision(),
                                "epoch":view.epoch,"configRevision":view.config_revision,
                                "catalogRevision":view.catalog_revision,"executionBusy":view.execution_running,
                                "clipboard":view.clipboard.as_ref().map(|c|serde_json::json!({
                                    "revision":c.revision,"selected_id":c.selected_id,"text":c.text,
                                    "source":c.source,"date":c.date,"kind":c.kind,"files":c.files,
                                    "paused":c.paused,"loading":c.loading,"error":c.error
                                })),"inputOwners":connection.pending_owned_inputs(),"mainServicesPending":main_services_pending
                            })));
                        }
                    }
                }
            }
        }
        if let Some(pending)=self.pending.as_ref() {
            let expired=std::time::Instant::now()>=pending.request.deadline;
            let stopped=connection.stop_requested()||owner.ivars().failed.get();
            let view=connection.view();
            let reconciled=applied&&connection.applied_revision()==view.revision;
            let ready=view.epoch==pending.epoch&&match &pending.request.operation {
                OwnedOperation::Hide=>!view.visible&&!owner.ivars().panel.isVisible()&&reconciled,
                OwnedOperation::Query(stage,query)=>view.visible&&view.query==*query&&(*stage==crate::native_f1::QueryStage::Search||reconciled),
                OwnedOperation::Action(action)=>view.visible&&reconciled&&view.catalog_revision==pending.catalog_revision&&match action {
                    crate::native_f1::UiAction::Select{index}=>view.selection==Some(*index)&&view.rows.get(*index).map(|r|r.id.as_str())==pending.selected_id.as_deref(),
                    crate::native_f1::UiAction::Actions{open}=>view.actions_open==*open,
                },
                OwnedOperation::Snapshot=>unreachable!(),
            };
            if expired||stopped||ready {
                let pending=self.pending.take().expect("Pending UI request exists.");
                let result=if expired||stopped {Err("Owned UI request expired or stopped.".into())}
                    else {Ok(serde_json::json!({"revision":view.revision,"epoch":view.epoch}))};
                let _=pending.request.reply.try_send(result);
            }
        }
    }
}

// The fixed reload lane measures the ordinary serialized request. Preparation
// and full semantic/privacy observations are deliberately outside its timer.
#[cfg(feature = "test-tools")]
struct ReloadWitness {
    configuration: serde_json::Value,
    revision: u64,
    usage: serde_json::Value,
    recency: serde_json::Value,
}
#[cfg(feature = "test-tools")]
impl ReloadWitness {
    fn capture(state: &serde_json::Value) -> Result<Self, String> {
        for key in ["configuration_writer", "configuration_reply", "configuration_dirty"] {
            if state["owners"][key].as_u64() != Some(0) {
                return Err("Reload configuration ownership is not settled.".into());
            }
        }
        if state["keyboardActivationPending"].as_bool() != Some(false)
            || state.get("configurationWarning").is_none_or(|v| !v.is_null())
        {
            return Err("Reload configuration activation remains unresolved.".into());
        }
        let configuration = state.get("configuration")
            .filter(|v| v.is_object() && v["commands"].is_array())
            .ok_or("Reload preparation has no complete configuration.")?.clone();
        let revision = state["configuration_revision"].as_u64()
            .ok_or("Reload preparation has no configuration revision.")?;
        let usage = state.get("usage").filter(|v| v.is_object())
            .ok_or("Reload preparation has no usage observation.")?.clone();
        let recency = state.get("recency").filter(|v| v.is_object())
            .ok_or("Reload preparation has no ranking observation.")?.clone();
        if state["palette"]["config"] != configuration {
            return Err("Reload configuration has not reached the palette owner.".into());
        }
        Ok(Self { configuration, revision, usage, recency })
    }
    fn acknowledge(&self, response: &serde_json::Value) -> Result<serde_json::Value, String> {
        // AppState.handle("reload") returns exactly ok + result(commands,
        // hotkeyWarnings). Native's additional publication dispositions must
        // all be checked before this closed, source-bound schema projection.
        let keys = ["ok", "result", "configurationWrite", "keyboardActivationPending",
            "configurationWarning", "configurationReconciliationRequired",
            "clipboardWarning", "clipboardCleanupError"];
        if !response.as_object().is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
            || response["ok"].as_bool() != Some(true)
            || response["configurationWrite"].as_object().is_none_or(|o| o.len() != 2)
            || response["configurationWrite"]["published"].as_bool() != Some(false)
            || response["configurationWrite"]["outcomeUncertain"].as_bool() != Some(false)
            || response["keyboardActivationPending"].as_bool() != Some(false)
            || response["configurationReconciliationRequired"].as_bool() != Some(false)
            || ["configurationWarning", "clipboardWarning", "clipboardCleanupError"].iter().any(|k| !response[*k].is_null())
        {
            return Err("Reload did not return a complete, settled production acknowledgement.".into());
        }
        let result = &response["result"];
        let count = self.configuration["commands"].as_array().expect("Validated configuration commands").len();
        if !result.as_object().is_some_and(|o| o.len() == 2 && o.contains_key("commands") && o.contains_key("hotkeyWarnings"))
            || result["commands"].as_u64() != u64::try_from(count).ok()
            || !result["hotkeyWarnings"].as_array().is_some_and(|a| a.is_empty())
        {
            return Err("Reload acknowledgement differs from the prepared configuration or has shortcut warnings.".into());
        }
        Ok(serde_json::json!({"ok":true,"result":result}))
    }
    fn complete(&self, state: &serde_json::Value, native: &serde_json::Value) -> Result<(), String> {
        let after = Self::capture(state)?;
        if after.configuration != self.configuration || after.revision != self.revision
            || after.usage != self.usage || after.recency != self.recency
        {
            return Err("Reload changed the fixed configuration or usage/ranking privacy state.".into());
        }
        let revision = native["revision"].as_u64().ok_or("Reload has no native publication revision.")?;
        if native["configRevision"].as_u64() != Some(after.revision)
            || native["appliedRevision"].as_u64() != Some(revision)
        {
            return Err("Reload has not reached the actual native owner acknowledgement.".into());
        }
        Ok(())
    }
}
#[cfg(feature = "test-tools")]
enum ReloadProgress {
    Prepared(ReloadWitness),
    Acknowledged(ReloadWitness),
}

#[cfg(feature = "test-tools")]
struct NativeEndpoint {
    requests: std::sync::mpsc::SyncSender<OwnedRequest>,
    signal: std::sync::Arc<crate::main_loop::Signal>,
    connection: std::sync::Arc<ruycast_native::connected::Connection>,
    inspection: ruycast_native::runtime::OwnedClient,
    root: std::path::PathBuf,
    bindings: crate::native_f1::ReadyBindings,
    // Read only from the independently pinned fixture while Host admission
    // still owns it; this value never comes from an endpoint request or env.
    fixture_epoch: u64,
    // Populating implementation methods is not independent route admission.
    admitted: std::collections::BTreeSet<String>,
    reload: Option<ReloadProgress>,
    process_prepared: Option<(u64,u64)>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
#[cfg(feature = "test-tools")]
impl NativeEndpoint {
    fn owner_call(&self, operation: OwnedOperation, deadline: std::time::Instant)->Result<serde_json::Value,String> {
        let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Owned request expired.")?;
        let (reply,response)=std::sync::mpsc::sync_channel(1);
        self.requests.try_send(OwnedRequest{operation,deadline,reply}).map_err(|_|"Owned coordinator is busy or stopped.")?;
        self.signal.wake();
        match response.recv_timeout(remaining) {
            Ok(result)=>result,
            Err(_)=>{self.signal.wake();Err("Owned coordinator expired or stopped.".into())}
        }
    }
    fn unqualified<T>(&self, route: &str)->Result<T,String> {
        Err(format!("Native production route is not yet source-qualified: {route}; no completion."))
    }
    fn process_preparation_witness(&self,state:&serde_json::Value)->Result<(u64,u64),String>{
        let revision=state["configuration_revision"].as_u64().ok_or("Process fixture configuration revision absent.")?;
        let ranking=state["ranking_revision"].as_u64().ok_or("Process fixture ranking revision absent.")?;
        let entries=serde_json::from_value::<std::collections::HashMap<String,f64>>(state["recency"].clone()).map_err(|e|e.to_string())?;
        let epoch=i64::try_from(self.fixture_epoch).map_err(|_|"Process fixture epoch overflow.")?;
        let expected=serde_json::from_value::<std::collections::HashMap<String,f64>>(
            crate::native_f1::fixture::recency(epoch)["entries"].clone()).map_err(|e|e.to_string())?;
        if state["configuration"]["commands"]!=serde_json::json!(crate::native_f1::fixture::catalog(10000)?)
            || entries!=expected||state["palette"]["mode"]!="processes"||state["palette"]["paletteVisible"]!=false {
            return Err("The prepared process fixture changed; no automatic reset.".into());
        }
        Ok((revision,ranking))
    }
}
#[cfg(feature = "test-tools")]
impl crate::native_f1::Host for NativeEndpoint {
    fn verify_live(&mut self)->Result<(),String> {
        if self.cancel.load(std::sync::atomic::Ordering::Acquire)||self.connection.stop_requested()||self.connection.stopped(){Err("Owned host stopped.".into())}else{Ok(())}
    }
    fn bindings(&self)->&crate::native_f1::ReadyBindings{&self.bindings}
    fn admitted_routes(&self)->&std::collections::BTreeSet<String>{&self.admitted}
    fn stage_catalog(&mut self,size:usize,deadline:std::time::Instant)->Result<(),String>{
        // An explicit new catalog starts a different workload preparation.
        self.process_prepared=None;
        let committed=loop {
            let response=self.inspection.perform(ruycast_native::runtime::OwnedOperation::StageCatalog(size),deadline)?;
            if response["busy"]!=true {break response;}
            let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Catalog staging remained busy until its deadline.")?;
            std::thread::park_timeout(remaining.min(std::time::Duration::from_millis(2)));
        };
        if committed["ok"]!=true{return Err("The production owner did not confirm durable catalog staging.".into());}
        let observed=loop {
            let observed=self.inspection.inspect(deadline)?;
            let pending=observed.get("keyboardActivationPending").and_then(serde_json::Value::as_bool)
                .ok_or("The production owner did not report catalog activation state.")?;
            let warning=observed.get("configurationWarning")
                .ok_or("The production owner did not report catalog warning state.")?;
            if pending||!warning.is_null(){return Err("Catalog activation or configuration remains unresolved; no catalog measurement.".into());}
            if ["configuration_writer","configuration_reply","configuration_dirty"].iter().all(|key|observed["owners"][*key]==0){break observed;}
            let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Catalog adoption remained pending until its deadline.")?;
            std::thread::park_timeout(remaining.min(std::time::Duration::from_millis(2)));
        };
        if observed["configuration"]["commands"]!=serde_json::json!(crate::native_f1::fixture::catalog(size)?){
            return Err("The production configuration owner has a different complete catalog fixture.".into());
        }
        let response=self.wire_call(serde_json::json!({"op":"show","query":""}),deadline)?;
        if response["ok"]!=true{return Err("The production owner refused to open the catalog.".into());}
        self.query(crate::native_f1::QueryStage::Reconciliation,"",deadline)
    }
    fn query(&mut self,stage:crate::native_f1::QueryStage,query:&str,deadline:std::time::Instant)->Result<(),String>{
        self.owner_call(OwnedOperation::Query(stage,query.into()),deadline).map(|_|())
    }
    fn native_action(&mut self,action:crate::native_f1::UiAction,deadline:std::time::Instant)->Result<serde_json::Value,String>{
        self.owner_call(OwnedOperation::Action(action),deadline)
    }
    fn snapshot(&mut self,deadline:std::time::Instant)->Result<serde_json::Value,String>{
        let state=self.inspection.inspect(deadline)?;
        let display=self.owner_call(OwnedOperation::Snapshot,deadline)?;
        if state["configuration_revision"]!=display["configRevision"]||state["palette_epoch"]!=display["epoch"] {
            return Err("Configuration changed across the production snapshot.".into());
        }
        if let Some(progress) = self.reload.take() {
            match progress {
                ReloadProgress::Acknowledged(witness) => witness.complete(&state, &display)?,
                ReloadProgress::Prepared(_) => return Err("Reload has no production acknowledgement.".into()),
            }
        }
        // Preserve all owner facts while the full cross-host semantic schema
        // (including non-configuration rows) remains separately unqualified.
        Ok(state["palette"].clone())
    }
    fn wire_call(&mut self,request:serde_json::Value,deadline:std::time::Instant)->Result<serde_json::Value,String>{
        use std::io::{Read,Write};
        use std::os::unix::net::UnixStream;
        let reload = if request["op"] == "reload" {
            if request != serde_json::json!({"op":"reload"}) {
                return Err("The fixed reload lane accepts only its source-bound request.".into());
            }
            match self.reload.take() {
                Some(ReloadProgress::Prepared(witness)) => Some(witness),
                _ => return Err("Reload was not prepared on this production endpoint.".into()),
            }
        } else { None };
        let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Owned wire request expired.")?;
        let mut stream=UnixStream::connect(self.root.join("control.sock")).map_err(|e|e.to_string())?;
        stream.set_read_timeout(Some(remaining)).map_err(|e|e.to_string())?;
        stream.set_write_timeout(Some(remaining)).map_err(|e|e.to_string())?;
        let mut bytes=serde_json::to_vec(&request).map_err(|e|e.to_string())?;
        if bytes.len()>1_048_576{return Err("Owned wire request exceeds the production limit.".into());}
        bytes.push(b'\n'); stream.write_all(&bytes).map_err(|e|e.to_string())?;
        let mut response=Vec::new(); let mut chunk=[0;16384];
        loop{
            let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Owned wire response expired; outcome may be unknown.")?;
            stream.set_read_timeout(Some(remaining)).map_err(|e|e.to_string())?;
            let n=match stream.read(&mut chunk){Ok(n)=>n,Err(e) if e.kind()==std::io::ErrorKind::Interrupted=>continue,Err(e)=>return Err(e.to_string())};
            if n==0{return Err("Production wire response closed.".into());}
            response.extend_from_slice(&chunk[..n]);
            if response.len()>crate::native_f1::MAX_FRAME_BYTES{return Err("Owned wire response exceeds64MiB.".into());}
            if let Some(end)=response.iter().position(|b|*b==b'\n') {
                let parsed: serde_json::Value = serde_json::from_slice(&response[..end]).map_err(|e|e.to_string())?;
                if let Some(witness) = reload {
                    self.verify_live()?;
                    let result = witness.acknowledge(&parsed)?;
                    self.reload = Some(ReloadProgress::Acknowledged(witness));
                    return Ok(result);
                }
                return Ok(parsed);
            }
        }
    }
    fn prepare_functional(&mut self,lane:crate::native_f1::Functional,deadline:std::time::Instant)->Result<(),String>{
        self.reload = None;
        match lane {
            crate::native_f1::Functional::CachedSearch=>{
                // The original four catalogs each advance 10+1000 times;
                // query[4040 % 10] is m. Preparation stays outside the timer.
                self.stage_catalog(10000,deadline)?;
                self.query(crate::native_f1::QueryStage::Search,"m",deadline)
            }
            crate::native_f1::Functional::CachedProcesses1000|crate::native_f1::Functional::ProcessSort1000=>{
                self.verify_live()?;
                if let Some(witness)=self.process_prepared {
                    let current=self.inspection.inspect(deadline)?;
                    if self.process_preparation_witness(&current)?!=witness {
                        return Err("Process preparation ownership changed; no automatic reset.".into());
                    }
                    // The real owner validates all resources/order/ID96 and
                    // preserves its current CPU/Memory sort and selection.
                    let checked=self.inspection.perform(ruycast_native::runtime::OwnedOperation::ValidateProcesses1000,deadline)?;
                    if self.process_preparation_witness(&checked)?!=witness {
                        return Err("Process preparation changed during validation.".into());
                    }
                    return Ok(());
                }
                // PerformanceChecks reaches processes after the final 10k
                // catalog and query m. Keep actual catalog publication,
                // owner query and native hide acknowledgement outside timing.
                self.stage_catalog(10000,deadline)?;
                self.query(crate::native_f1::QueryStage::Search,"m",deadline)?;
                self.owner_call(OwnedOperation::Hide,deadline)?;
                self.inspection.perform(ruycast_native::runtime::OwnedOperation::StageProcessRecency(self.fixture_epoch),deadline)?;
                let prepared=self.inspection.perform(ruycast_native::runtime::OwnedOperation::StageProcesses1000,deadline)?;
                self.process_prepared=Some(self.process_preparation_witness(&prepared)?);
                Ok(())
            }
            crate::native_f1::Functional::ConfigReload=>{
                self.verify_live()?;
                let observed = self.inspection.inspect(deadline)?;
                self.reload = Some(ReloadProgress::Prepared(ReloadWitness::capture(&observed)?));
                Ok(())
            }
            crate::native_f1::Functional::AnalyticsTiming|crate::native_f1::Functional::ConfigRead=>{
                self.inspection.inspect(deadline).map(|_|())
            }
            _=>self.unqualified("functional fixture preparation"),
        }
    }
    fn functional(&mut self,lane:crate::native_f1::Functional,deadline:std::time::Instant)->Result<crate::native_f1::Completed,String>{
        let operation=match lane {
            crate::native_f1::Functional::CachedSearch=>ruycast_native::runtime::OwnedOperation::CachedResultRead,
            crate::native_f1::Functional::CachedProcesses1000=>ruycast_native::runtime::OwnedOperation::CachedProcesses1000,
            crate::native_f1::Functional::ProcessSort1000=>ruycast_native::runtime::OwnedOperation::ProcessSort1000,
            crate::native_f1::Functional::AnalyticsTiming=>ruycast_native::runtime::OwnedOperation::AnalyticsTiming,
            crate::native_f1::Functional::ConfigRead=>ruycast_native::runtime::OwnedOperation::ConfigRawRead,
            _=>return self.unqualified("functional completion"),
        };
        let measured=self.inspection.perform(operation,deadline)?;
        let elapsed_ns=measured["measured"]["elapsed_ns"].as_u64().ok_or("The production owner did not return a measured operation.")?;
        let state=self.snapshot(deadline)?;
        let diagnostics=self.drain(deadline)?;
        Ok(crate::native_f1::Completed{elapsed_ns,output:serde_json::json!({"result":measured["measured"]["result"],"state":state}),diagnostics})
    }
    fn startup(&mut self,_:crate::native_f1::Startup,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("startup")}
    fn lifecycle_10000(&mut self,_:crate::native_f1::Phase,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("10000 lifecycle transitions")}
    fn idle_60seconds(&mut self,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("60 second idle ownership")}
    fn copies_allocations_input(&mut self,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("input allocation counters")}
    fn artifact(&mut self,_:crate::native_f1::Artifact,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("artifact provenance")}
    fn build(&mut self,_:crate::native_f1::Build,_:std::time::Instant)->Result<crate::native_f1::Completed,String>{self.unqualified("build completion")}
    fn drain(&mut self,deadline:std::time::Instant)->Result<serde_json::Value,String>{
        let observed=self.inspection.inspect(deadline)?;
        let native=self.owner_call(OwnedOperation::Snapshot,deadline)?;
        match native["mainServicesPending"].as_u64() {
            Some(0)=>{},
            Some(count)=>return Err(format!("The real main-services owner still retains {count} requests.")),
            None=>return Err("The main-services owner did not return a pending-request observation.".into()),
        }
        if observed["owner_coverage_complete"]!=true{return Err(format!("Production drain coverage is incomplete: {}",observed["unobserved_owners"]));}
        self.unqualified("SDK effect and fixture coverage")
    }
    fn shutdown(&mut self,_:std::time::Instant)->Result<(),String>{
        // A pre-stop observation cannot prove post-teardown drain. In
        // particular, normal retirement must not cancel the ACK writer first.
        self.unqualified("post-teardown owner drain and shutdown acknowledgement")
    }
}

#[cfg(all(test, feature = "test-tools"))]
mod reload_endpoint_tests {
    use super::ReloadWitness;
    use serde_json::{json, Value};
    fn state() -> Value {
        let configuration = json!({"version":1,"commands":[{"id":"fixture.reload","title":"Reload fixture"}]});
        json!({"configuration":configuration,"configuration_revision":7,
            "palette":{"config":configuration},"usage":{"fixture.persisted":7},"recency":{"fixture.persisted":42.0},
            "owners":{"configuration_writer":0,"configuration_reply":0,"configuration_dirty":0},
            "keyboardActivationPending":false,"configurationWarning":null})
    }
    fn response() -> Value {
        json!({"ok":true,"result":{"commands":1,"hotkeyWarnings":[]},
            "configurationWrite":{"published":false,"outcomeUncertain":false},
            "keyboardActivationPending":false,"configurationWarning":null,
            "configurationReconciliationRequired":false,"clipboardWarning":null,"clipboardCleanupError":null})
    }
    #[test]
    fn reload_preparation_and_complete_publication_preserve_config_and_privacy() {
        let before = state();
        let witness = ReloadWitness::capture(&before).unwrap();
        assert_eq!(witness.acknowledge(&response()).unwrap(),json!({"ok":true,"result":{"commands":1,"hotkeyWarnings":[]}}));
        let native = json!({"configRevision":7,"revision":13,"appliedRevision":13});
        witness.complete(&before,&native).unwrap();
        for path in ["configuration_writer","configuration_reply","configuration_dirty"] {
            let mut busy = before.clone(); busy["owners"][path] = json!(1);
            assert!(ReloadWitness::capture(&busy).is_err());
            busy["owners"].as_object_mut().unwrap().remove(path);
            assert!(ReloadWitness::capture(&busy).is_err());
        }
        for key in ["configuration","configuration_revision","usage","recency","keyboardActivationPending","configurationWarning"] {
            let mut missing = before.clone(); missing.as_object_mut().unwrap().remove(key);
            assert!(ReloadWitness::capture(&missing).is_err(),"missing {key}");
        }
        let mut unadopted=before.clone();unadopted["palette"]["config"]=json!({});
        assert!(ReloadWitness::capture(&unadopted).is_err());
    }
    #[test]
    fn reload_ack_refuses_partial_warning_uncertain_and_wrong_count_responses() {
        let witness=ReloadWitness::capture(&state()).unwrap();
        for key in response().as_object().unwrap().keys() {
            let mut missing=response(); missing.as_object_mut().unwrap().remove(key);
            assert!(witness.acknowledge(&missing).is_err(),"missing {key}");
        }
        for (path,value) in [
            ("/ok",json!(false)),("/configurationWrite/published",json!(true)),
            ("/configurationWrite/outcomeUncertain",json!(true)),
            ("/keyboardActivationPending",json!(true)),("/configurationReconciliationRequired",json!(true)),
            ("/configurationWarning",json!("own warning")),("/clipboardWarning",json!("own warning")),
            ("/clipboardCleanupError",json!("own cleanup error")),("/result/commands",json!(2)),
            ("/result/hotkeyWarnings",json!(["own shortcut warning"])),
        ] {
            let mut wrong=response();*wrong.pointer_mut(path).unwrap()=value;
            assert!(witness.acknowledge(&wrong).is_err(),"{path}");
        }
        let mut extra=response();extra["unexpected"]=json!("unqualified disposition");
        assert!(witness.acknowledge(&extra).is_err());
        let mut extra_result=response();extra_result["result"]["detail"]=json!("not a reload result");
        assert!(witness.acknowledge(&extra_result).is_err());
    }
    #[test]
    fn reload_completion_refuses_changed_config_usage_ranking_or_stale_native_ack() {
        let before=state();let witness=ReloadWitness::capture(&before).unwrap();
        let native=json!({"configRevision":7,"revision":13,"appliedRevision":13});
        for (path,value) in [
            ("/configuration_revision",json!(8)),("/usage/fixture.persisted",json!(8)),
            ("/recency/fixture.persisted",json!(43.0)),
        ] {
            let mut changed=before.clone();*changed.pointer_mut(path).unwrap()=value;
            assert!(witness.complete(&changed,&native).is_err(),"{path}");
        }
        let mut changed=before.clone();changed["configuration"]["commands"][0]["title"]=json!("changed");
        changed["palette"]["config"]=changed["configuration"].clone();
        assert!(witness.complete(&changed,&native).is_err());
        for (key,value) in [("configRevision",json!(6)),("appliedRevision",json!(12)),("revision",Value::Null)] {
            let mut stale=native.clone();stale[key]=value;
            assert!(witness.complete(&before,&stale).is_err(),"{key}");
        }
    }
}

// Preserve the production apply/notice ordering; the owned probe frontdoor
// also uses this exact guarded adoption before it injects a native input.
fn apply_connected_view(current: &Controller, connected: &ruycast_native::connected::Connection, next: ruycast_native::connected::View, hidden: bool, application: &NSApplication) -> bool {
    let notice = next.passive_notice.clone();
    if !apply_view(current, connected, next, hidden, application) { return false; }
    if let Some(notice) = notice.filter(|n| n.live(connected)) {
        let service = current.ivars().toast.borrow_mut().take();
        if let Some(mut service) = service {
            let result = service.show(
                &notice,
                hidden,
                &current.ivars().panel.effectiveAppearance(),
                || notice.live(connected),
            );
            *current.ivars().toast.borrow_mut() = Some(service);
            if result.is_err() {
                current.ivars().failed.set(true);
                connected.request_stop();
            }
        }
    }
    true
}
#[cfg(feature = "test-tools")]
fn deferred_input_probe(current: &Controller, connected: &std::sync::Arc<ruycast_native::connected::Connection>, hidden: bool, request: &serde_json::Value) -> bool {
    let state = current.ivars();
    if !hidden || !state.defer_input_child.get() || connected.stop_requested() || connected.stopped() {
        return false;
    }
    let allowed = match request["action"].as_str() {
        Some("defer-input-child") => request["defer"].as_bool().is_some(),
        Some("run-inline-read" | "run-inline-type" | "run-inline-mark") => true,
        Some("key-event") => request["code"] == 53 && request["flags"] == 0 && request["chars"] == "",
        _ => false,
    };
    if !allowed { return false; }
    let model = state.session.borrow();
    let Model::Live { view, connection } = &*model else { return false; };
    let Some(parent) = view.editor.as_ref().filter(|e|
        e.run_inline && e.mode == ruycast_native::actions::Mode::Input
            && state.run_inline_token.get() == e.token) else { return false; };
    let next = connected.view();
    let Some(published) = next.editor.as_ref() else { return false; };
    // This fixture deliberately holds publication before presentation. Only
    // observe/cancel/edit its actual still-presented parent; never dispatch an
    // Alias action, Return or another input through an unacknowledged view.
    // The owner may publish another child revision or the fresh resumed Input
    // while this same local field editor remains deliberately unpresented.
    // Bind that published lineage to the held physical parent instead of
    // requiring two independently read publication revisions to stay equal.
    let same_parent = published.id == parent.id
        && ((published.mode == ruycast_native::actions::Mode::Alias
            && !published.run_inline && published.suspended_input_token == Some(parent.token))
            || (published.mode == ruycast_native::actions::Mode::Input
                && published.run_inline && published.token > parent.token
                && published.suspended_input_token.is_none()
                && published.resumed_input_from == Some(parent.token)));
    std::sync::Arc::ptr_eq(connection, connected)
        && view.revision == connected.applied_revision()
        && next.revision > view.revision
        && view.epoch == next.epoch && view.epoch == connected.latest_epoch()
        && view.dismissal == next.dismissal && view.dismissal == connected.latest_dismissal()
        && same_parent
}
#[cfg(feature = "test-tools")]
fn acknowledge_probe_view(current: &Controller, connected: &std::sync::Arc<ruycast_native::connected::Connection>, hidden: bool, application: &NSApplication) -> Result<(), String> {
    if connected.stop_requested() || connected.stopped() { return Err("Native owner stopped.".into()); }
    let next = connected.view();
    if next.revision > connected.applied_revision() && !apply_connected_view(current, connected, next, hidden, application) {
        return Err("Native view could not be adopted.".into());
    }
    let latest = connected.view();
    let model = current.ivars().session.borrow();
    let same_model = matches!(&*model, Model::Live { view, connection } if std::sync::Arc::ptr_eq(connection,connected) && view.revision==latest.revision && view.epoch==latest.epoch && view.dismissal==latest.dismissal);
    if !same_model || latest.revision != connected.applied_revision() || latest.epoch < connected.latest_epoch() || latest.dismissal < connected.latest_dismissal() || connected.stop_requested() || connected.stopped() {
        return Err("Native view changed before probe dispatch.".into());
    }
    Ok(())
}

fn apply_view(
    controller: &Controller,
    connection: &ruycast_native::connected::Connection,
    mut next: ruycast_native::connected::View,
    hidden: bool,
    app: &NSApplication,
) -> bool {
    #[cfg(feature = "test-tools")]
    if controller.ivars().defer_input_child.get()
        && next
            .editor
            .as_ref()
            .is_some_and(|e| e.suspended_input_token.is_some())
    {
        return false;
    }
    if next
        .script_result
        .as_ref()
        .is_some_and(|r| connection.result_cancelled(r.token))
    {
        return false;
    }
    if next
        .completion_intent
        .is_some_and(|intent| intent != connection.latest_intent())
    {
        return false;
    }
    if next.epoch < connection.latest_epoch() {
        return false;
    }
    if connection.open_hide_masks(next.epoch) {
        next.visible = false;
    }
    if next.dismissal < connection.latest_dismissal() {
        next.visible = false;
    }
    if next.editor.as_ref().is_some_and(|e| {
        connection.editor_cancelled(e.token)
            || e.origin_inline
                .is_some_and(|t| connection.editor_cancelled(t))
            || e.origin_menu
                .is_some_and(|token| connection.menu_cancelled(token))
    }) {
        if next.editor.as_ref().is_some_and(|e| {
            e.suspended_input_token == Some(controller.ivars().run_inline_token.get())
        }) {
            // Child cancelled before presentation. Keep the live local Input
            // editor until the owner's fresh resume arrives (including IME/Undo).
            return false;
        }
        next.editor = None;
    }
    if connection.menu_cancelled(next.menu_token) {
        next.actions_open = false;
    }
    if next
        .editor
        .as_ref()
        .and_then(|e| e.input_menu_attempt)
        .is_some_and(|a| connection.input_control_cancelled(a))
    {
        next.actions_open = false;
    }

    let state = controller.ivars();
    if let Some(editor) = next.editor.as_ref().filter(|e| {
        e.run_inline
            && state.run_inline_token.get() != e.token
            && e.resumed_input_from != Some(state.run_inline_token.get())
    }) {
        if state
            .input
            .currentEditor()
            .and_then(|e| {
                e.downcast_ref::<NSTextView>()
                    .map(|t| unsafe { msg_send![t, hasMarkedText] })
            })
            .unwrap_or(false)
        {
            connection.action(ruycast_native::connected::Action::CancelEditor {
                token: editor.token,
            });
            return false;
        }
    }
    let leaving_detail = state.restore_result_focus.get()
        || state
            .session
            .borrow()
            .script_result()
            .is_some_and(|r| r.detail_visible)
            && !next
                .script_result
                .as_ref()
                .is_some_and(|r| r.detail_visible);
    let visible = state.session.borrow().visible();
    let repeated_show = next.show_serial > state.session.borrow().show_serial();
    let opening = next.visible && (!visible || repeated_show);
    let mode_changed = next.clipboard.is_some() != state.session.borrow().clipboard().is_some();
    let theme_changed = state.session.borrow().theme() != next.theme.as_deref();
    controller
        .guard_callback(|| {
            let mut open_ready = false;
            if next
                .script_result
                .as_ref()
                .is_some_and(|r| r.detail_visible)
            {
                state.surface.command.text.breakUndoCoalescing();
            }
            state.session.borrow_mut().apply(next.clone());
            if next.visible && (mode_changed || !visible || repeated_show) {
                controller.layout_for_mode(next.clipboard.is_some(),None);
            }
            if theme_changed {
                let appearance = match next.theme.as_deref() {
                    Some("dark" | "graphite") => {
                        NSAppearance::appearanceNamed(ns_string!("NSAppearanceNameDarkAqua"))
                    }
                    Some("light") => {
                        NSAppearance::appearanceNamed(ns_string!("NSAppearanceNameAqua"))
                    }
                    _ => None,
                };
                state.panel.setAppearance(appearance.as_deref());
            }
            let marked = state
                .input
                .currentEditor()
                .and_then(|editor| {
                    editor
                        .downcast_ref::<NSTextView>()
                        .map(|text| unsafe { msg_send![text, hasMarkedText] })
                })
                .unwrap_or(false);
            if next.script_result.is_none()
                && !marked
                && next
                    .script_result
                    .as_ref()
                    .is_none_or(|r| r.query.is_some())
            {
                connection.adopt_query(&next.query, next.epoch);
            }
            if next.script_result.is_none()
                && !marked
                && next
                    .script_result
                    .as_ref()
                    .is_none_or(|r| r.query.is_some())
                && !next.editor.as_ref().is_some_and(|e| e.run_inline)
                && state.run_inline_token.get() == 0
                && state.input.stringValue().to_string() != next.query
            {
                state.input.setStringValue(&NSString::from_str(&next.query));
            }
            controller.reconcile();
            if leaving_detail
                && next.visible
                && !next
                    .script_result
                    .as_ref()
                    .is_some_and(|r| r.detail_visible)
            {
                state.restore_result_focus.set(false);
                if next.script_result.is_some() {
                    state.result_panel.focus_query();
                } else if next.editor.as_ref().is_some_and(|e| {
                    e.mode == ruycast_native::actions::Mode::Input && !e.run_inline
                }) {
                    state
                        .panel
                        .makeFirstResponder(Some(&state.surface.command.text));
                    state.surface.command.text.breakUndoCoalescing();
                } else {
                    state.panel.makeFirstResponder(Some(&state.input));
                }
            }
            if next.visible && (!visible || repeated_show) {
                state.inline_autosave_allowed.set(true);
                if !hidden {
                    state.panel.makeKeyAndOrderFront(None);
                    app.activate();
                }
                if next
                    .script_result
                    .as_ref()
                    .is_some_and(|r| r.detail_visible)
                {
                    state.result_panel.focus();
                } else if next.script_result.is_some() {
                    state.result_panel.focus_query();
                } else if next.dashboard.is_some() {
                    state.panel.makeFirstResponder(Some(&state.dashboard.scroll));
                } else if next.editor.is_none() {
                    open_ready = state.panel.makeFirstResponder(Some(&state.input));
                }
            } else if !next.visible {
                connection.cancel_open_trace();
                state.panel.orderOut(None);
            }
            #[cfg(feature = "test-tools")]
            if hidden {
                // A real AppKit focus/reconciliation operation may mark an
                // offscreen fixture visible without a public ordering call.
                // Enforce its policy here; reads and screenshots stay passive.
                state.panel.enforce_hidden_fixture();
            }
            connection.applied(next.revision);
            if opening { connection.finish_open_trace(next.show_serial, open_ready); }
        })
        .is_some()
}

/// Revoke all data-only authority before any AppKit/native cleanup can reenter
/// or block. This also runs after a failed primary source callback.
fn revoke_and_hide(owners: &[&Controller]) {
    for owner in owners {
        let client = owner.ivars().keyboard.borrow().as_ref().cloned();
        let connection = match &*owner.ivars().session.borrow() {
            Model::Live { connection, .. } => Some(connection.clone()),
            _ => None,
        };
        if let Some(client) = client.filter(|c| c.running()) {
            client.stop();
        }
        if let Some(connection) = connection.filter(|c| !c.stop_requested()) {
            connection.request_stop();
        }
    }
    for owner in owners {
        let state = owner.ivars();
        state.inline_autosave_allowed.set(false);
        let autosave = state.inline_autosave.borrow_mut().take();
        let timers = std::mem::take(&mut *state.timers.borrow_mut());
        drop(autosave);
        drop(timers);
        state.session.borrow_mut().set_visible(false);
        state.panel.orderOut(None); // Native override also releases recorder lease.
        state.surface.stop();
    }
}

#[cfg(feature = "test-tools")]
struct ShutdownWitness {
    owners: Vec<Weak<Controller>>,
    client: ruycast_native::keyboard_service::Client,
    output: std::path::PathBuf,
}
#[cfg(feature = "test-tools")]
unsafe extern "C" fn observe_shutdown_sdk(context: *mut std::ffi::c_void) {
    // Complete owned SDK invokes synchronously on main, while the controller
    // retains this Rc. No native keyboard/registry borrow is touched here.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let witness = unsafe { &*context.cast::<ShutdownWitness>() };
        let panels: Vec<_> = witness
            .owners
            .iter()
            .map(|weak| {
                let owner = weak.load().expect("Own shutdown controller");
                let state = owner.ivars();
                let connection_stopped = match &*state.session.borrow() {
                    Model::Live { connection, .. } => connection.stop_requested(),
                    _ => false,
                };
                serde_json::json!({"visible":state.panel.isVisible(),
                "modelVisible":state.session.borrow().visible(),
                "recording":state.surface.record.recording(),
                "preparing":state.surface.record.preparing(),
                "connectionStopped":connection_stopped})
            })
            .collect();
        let proof = serde_json::json!({"mainThread":MainThreadMarker::new().is_some(),
            "running":witness.client.running(), "recording":witness.client.recording(),
            "panels":panels,"pid":std::process::id()});
        std::fs::write(&witness.output, proof.to_string()).expect("Own shutdown witness");
    }));
}
#[cfg(feature = "test-tools")]
fn arm_shutdown_witness(
    owner: &Controller,
    force_visible: bool,
) -> Result<serde_json::Value, String> {
    crate::native_keyboard::fixture_hid(&serde_json::json!({"control":"state"}))?;
    let peer = owner
        .ivars()
        .peer
        .borrow()
        .as_ref()
        .and_then(Weak::load)
        .ok_or("Own peer required")?;
    let client = owner
        .ivars()
        .keyboard
        .borrow()
        .as_ref()
        .cloned()
        .ok_or("Own keyboard required")?;
    let home = std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME").ok_or("Own home required")?;
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_hid_fixture_hook".as_ptr()) };
    if symbol.is_null() {
        return Err("Own SDK hook missing".into());
    }
    // Titled AppKit windows may constrain their frame during editor setup.
    // Validate both before any ordering or hook installation.
    for panel in [&*owner.ivars().panel, &*peer.ivars().panel] {
        if panel.ivars().ordering_allowed
            || panel.isVisible()
            || panel.isKeyWindow()
            || panel.isMainWindow()
        {
            return Err("Only hidden prohibited fixture panels may be positioned".into());
        }
    }
    for panel in [&*owner.ivars().panel, &*peer.ivars().panel] {
        panel.setFrameOrigin(NSPoint::new(-20000., -20000.));
        if force_visible {
            panel.fixture_force_visible()?;
        }
    }
    let witness = std::rc::Rc::new(ShutdownWitness {
        owners: vec![Weak::new(owner), Weak::new(&*peer)],
        client,
        output: std::path::PathBuf::from(home).join("shutdown-sdk-witness.json"),
    });
    let hook: unsafe extern "C" fn(
        *const std::ffi::c_char,
        unsafe extern "C" fn(*mut std::ffi::c_void),
        *mut std::ffi::c_void,
    ) = unsafe { std::mem::transmute(symbol) };
    *owner.ivars().shutdown_witness.borrow_mut() = Some(witness.clone());
    unsafe {
        hook(
            c"manager_close".as_ptr(),
            observe_shutdown_sdk,
            std::rc::Rc::as_ptr(&witness).cast_mut().cast(),
        )
    };
    Ok(
        serde_json::json!({"panelsVisible":[owner.ivars().panel.isVisible(),peer.ivars().panel.isVisible()],
        "recordersActive":[owner.ivars().surface.record.active(),peer.ivars().surface.record.active()],
        "mainOrKey":[owner.ivars().panel.isKeyWindow() || owner.ivars().panel.isMainWindow(),peer.ivars().panel.isKeyWindow() || peer.ivars().panel.isMainWindow()]}),
    )
}

fn stop_application(app: &NSApplication) {
    app.stop(None);
    if let Some(event)=NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,NSPoint::new(0.,0.),NSEventModifierFlags::empty(),0.,0,None,0,0,0){app.postEvent_atStart(&event,true);}
}
// One explicit owned-fixture interleave, never a production dispatch repair.
#[cfg(feature = "test-tools")]
#[derive(Clone, Copy)]
enum AliasGapDispatch { CapturedStale, Acknowledged }
#[cfg(feature = "test-tools")]
struct AliasProbeGap {
    dispatch: AliasGapDispatch,
    owner: Weak<Controller>,
    connection: std::sync::Weak<ruycast_native::connected::Connection>,
    directory: std::fs::File,
    after_revision: u64,
    entered: serde_json::Value,
}
#[cfg(feature = "test-tools")]
thread_local! {
    static ALIAS_PROBE_GAP: RefCell<Option<AliasProbeGap>> = const { RefCell::new(None) };
    static ALIAS_PROBE_GAP_USED: Cell<bool> = const { Cell::new(false) };
}
#[cfg(feature = "test-tools")]
struct AliasProbeGapCleanup;
#[cfg(feature = "test-tools")]
impl Drop for AliasProbeGapCleanup {
    fn drop(&mut self) { ALIAS_PROBE_GAP.with(|gap| { gap.borrow_mut().take(); }); }
}
#[cfg(feature = "test-tools")]
fn alias_gap_write(directory: &std::fs::File, name: &std::ffi::CStr, value: &serde_json::Value) -> Result<(), String> {
    use std::{io::Write, os::fd::{AsRawFd, FromRawFd}};
    let bytes = serde_json::to_vec(value).map_err(|_| "Owned gap scalar encoding failed.")?;
    if bytes.len() > 4096 { return Err("Owned gap scalar bound exceeded.".into()); }
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW, 0o600) };
    if fd < 0 { return Err("Owned gap witness could not be created.".into()); }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|_| "Owned gap witness could not be committed.".into())
}
#[cfg(feature = "test-tools")]
fn alias_gap_release(directory: &std::fs::File) -> Result<bool, String> {
    use std::{io::Read, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}};
    let fd = unsafe { libc::openat(directory.as_raw_fd(), c"alias-main-gap-release".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK) };
    if fd < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound { Ok(false) } else { Err("Owned gap release could not be opened.".into()) };
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|_| "Owned gap release metadata unavailable.")?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 || metadata.len() > 16 {
        return Err("Owned gap release is not a private regular marker.".into());
    }
    let mut bytes = Vec::new();
    file.take(17).read_to_end(&mut bytes).map_err(|_| "Owned gap release could not be read.")?;
    if bytes != b"release\n" { return Err("Owned gap release marker differs.".into()); }
    Ok(true)
}
#[cfg(feature = "test-tools")]
fn alias_gap_scalars(controller: &Controller, connection: &std::sync::Arc<ruycast_native::connected::Connection>) -> Result<serde_json::Value, String> {
    let model = controller.ivars().session.borrow();
    let Model::Live { view, connection: owner } = &*model else { return Err("Owned gap requires a connected model.".into()); };
    if !std::sync::Arc::ptr_eq(owner, connection) { return Err("Owned gap connection changed.".into()); }
    let published = connection.view();
    let row = controller.ivars().table.selectedRow();
    let selected = usize::try_from(row).ok().and_then(|i| view.rows.get(i));
    let current = published.selection.and_then(|i| published.rows.get(i));
    Ok(serde_json::json!({"modelRevision":view.revision,"appliedRevision":connection.applied_revision(),"modelEpoch":view.epoch,"modelCatalog":view.catalog_revision,"modelConfig":view.config_revision,"publishedRevision":published.revision,"publishedEpoch":published.epoch,"publishedCatalog":published.catalog_revision,"publishedConfig":published.config_revision,"selectedRow":row,"sameSelectedId":selected.zip(current).is_some_and(|(a,b)|a.id==b.id),"sameQuery":view.query==published.query,"windowShown":controller.ivars().panel.isVisible(),"windowKey":controller.ivars().panel.isKeyWindow(),"windowMain":controller.ivars().panel.isMainWindow()}))
}
#[cfg(feature = "test-tools")]
fn alias_gap_arm(controller: &Controller, connection: &std::sync::Arc<ruycast_native::connected::Connection>, dispatch: AliasGapDispatch) -> Result<serde_json::Value, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if !connection.hidden_tests || connection.is_management || connection.stop_requested() || connection.stopped()
        || !controller.ivars().session.borrow().visible() || controller.ivars().session.borrow().surface().editor.is_some()
        || controller.ivars().panel.isVisible() || controller.ivars().panel.isKeyWindow() || controller.ivars().panel.isMainWindow() {
        return Err("An idle hidden owned launcher is required for this gap.".into());
    }
    let home = std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME").map(std::path::PathBuf::from).ok_or("Owned gap fixture home absent.")?;
    let uid = unsafe { libc::geteuid() };
    let private_dir = |path: &std::path::Path| -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o077 == 0)
            && std::fs::canonicalize(path).is_ok_and(|p|p==path)
    };
    let regular = |path: &std::path::Path| -> bool { std::fs::symlink_metadata(path).is_ok_and(|m|m.is_file() && m.uid()==uid && m.nlink()==1 && m.len()<=64) };
    let parent = home.parent().ok_or("Owned gap fixture parent absent.")?;
    if !home.is_absolute() || !private_dir(&home) || !private_dir(parent)
        || !regular(&parent.join(".ruycast-ui-fixture-parent")) || std::fs::read(parent.join(".ruycast-ui-fixture-parent")).ok().as_deref()!=Some(b"owned-ui-fixture-parent-v1") || !regular(&home.join(".ruycast-test-fixture"))
        || !ruycast_native::workspace::fixture_installed(&home)
        || controller.ivars().probe_output.borrow().as_ref().and_then(|p|p.parent()).is_none_or(|p|p!=home.join("p")) {
        return Err("Owned gap requires the existing private marked fixture boundary.".into());
    }
    if ALIAS_PROBE_GAP_USED.with(Cell::get) { return Err("Owned gap is one-shot.".into()); }
    let directory = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW).open(&home).map_err(|_| "Owned gap directory unavailable.")?;
    let scalars = alias_gap_scalars(controller, connection)?;
    alias_gap_write(&directory, c"alias-main-gap-arm.json", &scalars)?;
    ALIAS_PROBE_GAP_USED.with(|used|used.set(true));
    ALIAS_PROBE_GAP.with(|gap| *gap.borrow_mut() = Some(AliasProbeGap { dispatch,owner:Weak::new(controller),connection:std::sync::Arc::downgrade(connection),directory,after_revision:connection.applied_revision(),entered:serde_json::Value::Null }));
    Ok(scalars)
}
#[cfg(feature = "test-tools")]
fn alias_gap_pause(controller: &Controller, connection: &std::sync::Arc<ruycast_native::connected::Connection>, hidden: bool, applied: bool) -> Result<Option<AliasProbeGap>, String> {
    if !applied { return Ok(None); }
    let gap = ALIAS_PROBE_GAP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let matches = slot.as_ref().is_some_and(|gap| gap.owner.load().is_some_and(|owner|std::ptr::eq(&*owner,controller)) && gap.connection.upgrade().is_some_and(|owner|std::sync::Arc::ptr_eq(&owner,connection)) && connection.applied_revision()>gap.after_revision);
        if matches {slot.take()} else {None}
    });
    let Some(mut gap) = gap else {return Ok(None)};
    if !hidden || !connection.hidden_tests || connection.stop_requested() || connection.stopped() { return Err("Owned gap owner retired.".into()); }
    gap.entered = alias_gap_scalars(controller, connection)?;
    alias_gap_write(&gap.directory, c"alias-main-gap-entered.json", &gap.entered)?;
    let deadline = std::time::Instant::now()+std::time::Duration::from_secs(4);
    let mut advanced = false;
    let mut queued = false;
    let result = (|| {
        loop {
            if connection.stop_requested() || connection.stopped() {return Err("Owned gap stopped.".to_owned());}
            if std::time::Instant::now()>=deadline {return Err("Owned gap exceeded four seconds.".to_owned());}
            let current = alias_gap_scalars(controller, connection)?;
            if !advanced && current["publishedConfig"].as_u64()>gap.entered["modelConfig"].as_u64() && current["publishedCatalog"].as_u64()>gap.entered["modelCatalog"].as_u64() {
                alias_gap_write(&gap.directory,c"alias-main-gap-advanced.json",&current)?;advanced=true;
            }
            if advanced && !queued && connection.pending_owned_inputs()["native_probes"]==1 {
                alias_gap_write(&gap.directory,c"alias-main-gap-queued.json",&current)?;queued=true;
            }
            if alias_gap_release(&gap.directory)? {
                if !advanced || !queued {return Err("Owned gap released before catalog and queued-probe witnesses.".to_owned());}
                alias_gap_write(&gap.directory,c"alias-main-gap-ended.json",&serde_json::json!({"released":true,"advanced":advanced,"queued":queued}))?;
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    })();
    if let Err(error) = &result {
        // The closed errors below contain no request values, paths or OS data.
        let class=match error.as_str() {
            "Owned gap stopped."=>"stopped",
            "Owned gap exceeded four seconds."=>"deadline",
            "Owned gap release could not be opened."=>"release-open",
            "Owned gap release metadata unavailable."=>"release-metadata",
            "Owned gap release is not a private regular marker."=>"release-shape",
            "Owned gap release could not be read."=>"release-read",
            "Owned gap release marker differs."=>"release-bytes",
            "Owned gap released before catalog and queued-probe witnesses."=>"release-premature",
            _=>"witness-or-owner",
        };
        let _=alias_gap_write(&gap.directory,c"alias-main-gap-ended.json",&serde_json::json!({"released":false,"advanced":advanced,"queued":queued,"stopped":connection.stop_requested(),"errorClass":class}));
    }
    result?;
    Ok(Some(gap))
}
#[cfg(feature = "test-tools")]
fn alias_gap_taken(gap: &AliasProbeGap, controller: &Controller, connection: &std::sync::Arc<ruycast_native::connected::Connection>, requests: &[ruycast_native::control::Envelope]) -> Result<(), String> {
    let alias = requests.iter().filter(|r|r.context.is_live() && r.value["op"]=="native.test" && r.value["action"]=="key-event" && r.value["code"]==43 && r.value["flags"]==((1_u64<<20)|(1_u64<<19)) && r.value["chars"]==",").count();
    let scalars = alias_gap_scalars(controller, connection)?;
    alias_gap_write(&gap.directory,c"alias-main-gap-taken.json",&serde_json::json!({"entered":gap.entered,"beforeDispatch":scalars,"requests":requests.len(),"aliasRequests":alias}))?;
    if requests.len()!=1 || alias!=1 {return Err("Owned gap did not take exactly one live Alias probe.".into());}
    Ok(())
}
#[cfg(not(feature = "test-tools"))]
fn native_probe(
    _controller: &Controller,
    _connection: &std::sync::Arc<ruycast_native::connected::Connection>,
    _request: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({"ok":false,"error":"Native test tools are disabled."})
}
#[cfg(feature = "test-tools")]
fn schedule_keyboard_fixture(
    controller: &Controller,
    steps: std::sync::Arc<Vec<serde_json::Value>>,
    home: std::path::PathBuf,
    profile: std::path::PathBuf,
    deadline: std::time::Instant,
) -> Result<(), String> {
    let owner = Weak::new(controller);
    let timer = crate::main_loop::Timer::new(controller.mtm(), 0.01, move || {
        if let Some(owner) = owner.load() {
            owner.guard_callback(||{
            if !profile.join("owner-delay-started").exists(){
                if std::time::Instant::now()<deadline{schedule_keyboard_fixture(&owner,steps,home,profile,deadline).expect("Own keyboard barrier retry");}
                return;
            }
            for (index,step) in steps.iter().enumerate(){
                let weak=Weak::new(&*owner);let step=step.clone();let home=home.clone();let profile=profile.clone();
                let timer=crate::main_loop::Timer::new(owner.mtm(),step["after"].as_f64().unwrap_or(0.02),move||{
                    if let Some(owner)=weak.load(){owner.guard_callback(||{
                        let client=owner.ivars().keyboard.borrow().as_ref().cloned().expect("Own keyboard client");
                        let release=step["control"]=="release-owner";
                        let result=if release{Ok(serde_json::Value::Null)}else if step["control"]=="carbon"{crate::native_keyboard::fixture_emit(step["key"].as_u64().unwrap_or(11) as u32,step["modifiers"].as_u64().unwrap_or(256) as u32,None).map(|status|serde_json::json!({"status":status}))}else{crate::native_keyboard::fixture_hid(&step)};
                        let proof=serde_json::json!({"ownerEntered":profile.join("owner-delay-started").exists(),"ownerStillHeld":!profile.join("owner-delay-finished").exists(),"forwarded":client.hook_forwarded(),"hookEpoch":client.hook_epoch(),"health":client.hook_health(),"result":result});
                        std::fs::write(home.join(format!("hid-schedule-{index}.json")),proof.to_string()).expect("Own HID schedule witness");
                        if release{std::fs::write(profile.join("owner-delay-release"),b"release").expect("Own owner release");}
                    });}
                }).expect("Own keyboard step timer");
                owner.ivars().timers.borrow_mut().push(timer);
            }
        });
        }
    })?;
    controller.ivars().timers.borrow_mut().push(timer);
    Ok(())
}
#[cfg(feature = "test-tools")]
fn native_probe(
    controller: &Controller,
    connection: &std::sync::Arc<ruycast_native::connected::Connection>,
    request: &serde_json::Value,
) -> serde_json::Value {
    // Called only by the feature-gated hidden runtime. No global events,
    // external services, personal/general clipboard access or arbitrary output
    // path is offered. Fixed snippet controls affect only its owned named
    // board and synthetic foreground PID behind the complete SDK stamp.
    let result = (|| -> Result<serde_json::Value, String> {
        let state = controller.ivars();
        match request["action"].as_str() {
            Some("alias-main-gap-arm") => {
                return alias_gap_arm(controller,connection,AliasGapDispatch::CapturedStale).map(|result|serde_json::json!({"ok":true,"result":result}));
            }
            Some("alias-main-gap-ack-arm") => {
                return alias_gap_arm(controller,connection,AliasGapDispatch::Acknowledged).map(|result|serde_json::json!({"ok":true,"result":result}));
            }
            Some("snippet-hotkey")=>{
                let button=state.snippet_hotkey.borrow().as_ref().map(|c|c.button.clone());
                match request["operation"].as_str().unwrap_or("read") {
                    "retain"=>{*state.stale_snippet_hotkey.borrow_mut()=button.clone();},
                    "stale"=>{let old=state.stale_snippet_hotkey.borrow().clone();if let Some(old)=old{unsafe{old.performClick(None);}}},
                    "click"=>{if let Some(button)=&button{unsafe{button.performClick(None);}}},
                    "read"=>{},_=>return Err("Unknown own snippet hotkey probe.".into()),
                }
                let result=if let Some(button)=button {
                    let center=button.convertPoint_toView(NSPoint::new(button.bounds().size.width/2.,button.bounds().size.height/2.),state.panel.contentView().as_deref());
                    let hit=state.panel.contentView().and_then(|parent|parent.hitTest(center)).is_some_and(|hit|std::ptr::eq::<NSView>(&*hit,&*button as *const NSButton as *const NSView));
                    serde_json::json!({"present":true,"hidden":button.isHidden(),"enabled":button.isEnabled(),"title":button.title().to_string(),"keyEquivalent":button.keyEquivalent().to_string(),"label":NSAccessibility::accessibilityLabel(&*button).map(|s|s.to_string()),"role":button.accessibilityRole().map(|s|s.to_string()),"identity":format!("{:p}",&*button),"frame":[button.frame().origin.x,button.frame().origin.y,button.frame().size.width,button.frame().size.height],"hit":hit})
                }else{serde_json::json!({"present":false})};
                return Ok(serde_json::json!({"ok":true,"result":result}));
            }
            Some("snippet-link")=>{
                let mut result=state.surface.command.snippet_probe(request)?;
                result["parentSaveEnabled"]=serde_json::json!(state.surface.save.isEnabled());
                result["parentSaveHidden"]=serde_json::json!(state.surface.save.isHidden());
                result["parentCancelEnabled"]=serde_json::json!(state.surface.cancel.isEnabled());
                result["parentRecordEnabled"]=serde_json::json!(state.surface.record.isEnabled());
                result["parentTabsEnabled"]=serde_json::json!(state.toolbar.as_ref().is_none_or(|bar|bar.tabs.iter().all(|b|b.isEnabled())));
                result["parentFieldsEnabled"]=serde_json::json!(state.surface.command.fields.borrow().iter().all(|f|f.input.isEnabled())&&state.surface.command.output.isEnabled()&&state.surface.command.enabled.isEnabled());
                result["parentFieldsLocked"]=serde_json::json!(state.surface.command.fields.borrow().iter().all(|f|!f.input.isEnabled())&&!state.surface.command.output.isEnabled()&&!state.surface.command.enabled.isEnabled());
                result["parentTabsLocked"]=serde_json::json!(state.toolbar.as_ref().is_some_and(|bar|bar.tabs.iter().all(|b|!b.isEnabled())));
                return Ok(serde_json::json!({"ok":true,"result":result}));
            },
            Some("keyboard-schedule") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Owned keyboard required.".into());
                }
                let steps = request["steps"]
                    .as_array()
                    .filter(|s| s.len() <= 8)
                    .ok_or("Invalid own keyboard steps.")?
                    .clone();
                if steps.iter().any(|s| {
                    s["after"]
                        .as_f64()
                        .is_none_or(|v| !(0.0..=2.0).contains(&v))
                }) {
                    return Err("Invalid keyboard step time.".into());
                }
                let home = std::path::PathBuf::from(
                    std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME")
                        .ok_or("Owned keyboard home required.")?,
                );
                let profile = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                    .ok_or("Owned profile unavailable.")?;
                schedule_keyboard_fixture(
                    controller,
                    std::sync::Arc::new(steps),
                    home,
                    profile,
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                )?;
                return Ok(serde_json::json!({"ok":true,"result":{}}));
            }
            Some("keyboard-source-failure") => {
                let client = state
                    .keyboard
                    .borrow()
                    .as_ref()
                    .cloned()
                    .ok_or("Owned keyboard required.")?;
                state.source_failure.set(true);
                client.wake_main();
                return Ok(serde_json::json!({"ok":true,"result":{}}));
            }
            Some("keyboard-shutdown-witness") => {
                return Ok(
                    serde_json::json!({"ok":true,"result":arm_shutdown_witness(controller, request["forceVisible"].as_bool().unwrap_or(true))?}),
                );
            }
            Some("keyboard-keyword") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Native keyboard fixture is not enabled.".into());
                }
                return Ok(
                    serde_json::json!({"ok":true,"result":ruycast_native::keyword_capture::fixture(request)?}),
                );
            }
            Some("keyboard-hid") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Native keyboard fixture is not enabled.".into());
                }
                return Ok(
                    serde_json::json!({"ok":true,"result":crate::native_keyboard::fixture_hid(request)?}),
                );
            }
            Some("keyboard-press") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Native keyboard fixture is not enabled.".into());
                }
                let count = request["count"].as_u64().unwrap_or(1);
                if count > 64 {
                    return Err("Own callback burst is limited to64.".into());
                }
                let code = u32::try_from(request["code"].as_u64().unwrap_or(49))
                    .map_err(|_| "Invalid own code")?;
                let modifiers = u32::try_from(request["modifiers"].as_u64().unwrap_or(256))
                    .map_err(|_| "Invalid own modifiers")?;
                let id = request["id"]
                    .as_u64()
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|_| "Invalid own ID")?;
                let mut results = vec![];
                for _ in 0..count {
                    results.push(crate::native_keyboard::fixture_emit(code, modifiers, id)?);
                }
                return Ok(serde_json::json!({"ok":true,"result":{"statuses":results}}));
            }
            Some("notice-read") | Some("notice-screenshot") => {
                let output = if request["action"] == "notice-screenshot" {
                    Some(
                        state
                            .probe_output
                            .borrow()
                            .as_ref()
                            .ok_or("Own notice output unavailable")?
                            .with_file_name("native-notice-probe.png"),
                    )
                } else {
                    None
                };
                let value = state
                    .toast
                    .borrow()
                    .as_ref()
                    .ok_or("Notice busy")?
                    .probe(output.as_deref())?;
                return Ok(serde_json::json!({"ok":true,"result":value}));
            }

            Some("utility-board") => {
                use sha2::Digest;
                let board=crate::native_services::shared_fixture_board().ok_or("Complete owned Utility board required.")?;
                let text=board.stringForType(unsafe{NSPasteboardTypeString}).map(|s|s.to_string()).unwrap_or_default();
                let concealed=board.dataForType(&NSString::from_str("org.nspasteboard.ConcealedType")).is_some();
                return Ok(serde_json::json!({"ok":true,"result":{"privateNamedBoard":true,"concealed":concealed,"textLength":text.chars().count(),"textSHA256":format!("{:x}",sha2::Sha256::digest(text.as_bytes())),"changeCount":board.changeCount()}}));
            }
            Some("snippet-fixture-control") => {
                let home = std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or("Snippet fixture missing")?;
                if !ruycast_native::workspace::fixture_installed(&home) {
                    return Err("Snippet fixture missing".into());
                }
                let ready = unsafe {
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_ready".as_ptr(),
                    )
                };
                let control = unsafe {
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_control".as_ptr(),
                    )
                };
                if ready.is_null() || control.is_null() {
                    return Err("Complete snippet fixture unavailable".into());
                }
                let ready: unsafe extern "C" fn() -> u64 = unsafe { std::mem::transmute(ready) };
                if unsafe { ready() } != 0x5255504153544531 {
                    return Err("Complete snippet fixture unavailable".into());
                }
                let operation = request["operation"]
                    .as_u64()
                    .filter(|n| [1, 2, 3].contains(n))
                    .ok_or("Unknown own snippet control")?;
                let control: unsafe extern "C" fn(u32) = unsafe { std::mem::transmute(control) };
                unsafe { control(operation as u32) };
            }
            Some("workspace-release-callbacks") => {
                let home = std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or("Workspace fixture missing")?;
                if !ruycast_native::workspace::fixture_installed(&home) {
                    return Err("Workspace fixture boundary unavailable".into());
                }
                let symbol = unsafe {
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_workspace_fixture_release".as_ptr(),
                    )
                };
                if symbol.is_null() {
                    return Err("Workspace fixture release unavailable".into());
                }
                let release: unsafe extern "C" fn() = unsafe { std::mem::transmute(symbol) };
                unsafe {
                    release();
                }
            }
            Some("query") | Some("burst") | Some("inline-focus-and-query") => {
                if state.session.borrow().script_result().is_some() {
                    let value = request["query"].as_str().ok_or("Missing query")?;
                    state.result_panel.fixture_query(controller, value);
                    return Ok(serde_json::json!({"ok":true,"result":{"query":value}}));
                }
                if request["action"] == "inline-focus-and-query" {
                    state.panel.makeFirstResponder(Some(&state.input));
                }
                let query = request["query"].as_str().ok_or("Missing query")?;
                state.input.setStringValue(&NSString::from_str(query));
                let note = unsafe {
                    NSNotification::notificationWithName_object(
                        ns_string!("NSControlTextDidChangeNotification"),
                        Some(&state.input),
                    )
                };
                unsafe {
                    let _: () = msg_send![controller,controlTextDidChange:&*note];
                }
                if request["action"] == "burst" {
                    let editor = state
                        .input
                        .currentEditor()
                        .or_else(|| unsafe { state.panel.fieldEditor_forObject(false,Some(&state.input)) })
                        .ok_or("Missing native field editor")?;
                    let text = editor
                        .downcast_ref::<NSTextView>()
                        .ok_or("Wrong field editor")?;
                    for _ in 0..request["count"].as_u64().unwrap_or(5).min(32) {
                        unsafe {
                            let _: bool = msg_send![controller,control:&*state.input,textView:text,doCommandBySelector:sel!(moveDown:)];
                        }
                    }
                }
            }
            Some("key-event") | Some("flags-event") => {
                state.key_event_handled.set(None);
                let flags = NSEventModifierFlags::from_bits_retain(
                    request["flags"].as_u64().unwrap_or(0) as usize,
                );
                let code = u16::try_from(request["code"].as_u64().ok_or("Missing key code")?)
                    .map_err(|_| "Invalid key code")?;
                let chars = NSString::from_str(request["chars"].as_str().unwrap_or(""));
                let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(if request["action"]=="flags-event"{NSEventType::FlagsChanged}else{NSEventType::KeyDown},NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,&chars,&chars,request["repeat"]==true,code).ok_or("Local key allocation failed")?;
                if request["action"] == "flags-event" {
                    unsafe {
                        let _: () = msg_send![&*state.surface.record,flagsChanged:&*event];
                    }
                } else {
                    let handled: bool =
                        unsafe { msg_send![&*state.panel,performKeyEquivalent:&*event] };
                    state.key_event_handled.set(Some(handled));
                    if !handled {
                        if let Some(responder) = state.panel.firstResponder() {
                            unsafe {
                                let _: () = msg_send![&*responder,keyDown:&*event];
                            }
                        }
                    }
                }
            }
            Some("select-and-alias") | Some("unseen-selection-and-alias") => {
                let index = request["row"].as_u64().ok_or("Missing row")? as usize;
                if index >= state.table.numberOfRows() as usize {
                    return Err("No such row".into());
                }
                if request["action"] == "select-and-alias" {
                    state.table.selectRowIndexes_byExtendingSelection(
                        &NSIndexSet::indexSetWithIndex(index),
                        false,
                    );
                } else {
                    state.session.borrow_mut().select_row(Some(index));
                }
                let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),NSEventModifierFlags::Command|NSEventModifierFlags::Option,0.,state.panel.windowNumber(),None,ns_string!(","),ns_string!(","),false,43).ok_or("Local binding key allocation")?;
                let _: bool = unsafe { msg_send![&*state.panel,performKeyEquivalent:&*event] };
            }
            Some("alias-ime") | Some("command-ime") => {
                let field = if request["action"] == "command-ime" {
                    state
                        .surface
                        .command
                        .fields
                        .borrow()
                        .first()
                        .ok_or("Command editor is not open")?
                        .input
                        .clone()
                } else {
                    state.surface.alias.clone()
                };
                state.panel.makeFirstResponder(Some(&*field));
                let editor = field.currentEditor().ok_or("Missing alias field editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong alias editor")?;
                unsafe {
                    let _: () = msg_send![text,setMarkedText:ns_string!("に"),selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                if !marked {
                    return Err("Alias composition was not established".into());
                }
                for (code, flags, selector) in [
                    (36, NSEventModifierFlags::Command, sel!(insertNewline:)),
                    (53, NSEventModifierFlags::empty(), sel!(cancelOperation:)),
                ] {
                    let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,ns_string!(""),ns_string!(""),false,code).ok_or("Local key allocation")?;
                    if controller.route_key(&event) {
                        return Err("Alias shortcut bypassed native composition".into());
                    }
                    let handled: bool = unsafe {
                        msg_send![controller,control:&*field,textView:text,doCommandBySelector:selector]
                    };
                    if handled {
                        return Err("Alias delegate bypassed composition".into());
                    }
                }
                unsafe {
                    let _: () = msg_send![text, unmarkText];
                }
                if state.surface.pending.get().is_some() {
                    return Err("Alias composition submitted a save".into());
                }
            }
            Some("alias-type") => {
                if state.surface.alias.isHidden() || !state.surface.alias.isEnabled() {
                    return Err("Alias editor is not editable".into());
                }
                let editor = state
                    .surface
                    .alias
                    .currentEditor()
                    .ok_or("Alias field lost focus")?;
                let responder = state
                    .panel
                    .firstResponder()
                    .ok_or("Missing alias responder")?;
                if !std::ptr::eq::<objc2::runtime::AnyObject>(
                    &*editor as *const _ as *const _,
                    &*responder as *const _ as *const _,
                ) {
                    return Err("Alias field is not the current responder".into());
                }
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong alias field editor")?;
                let value =
                    NSString::from_str(request["text"].as_str().ok_or("Missing alias text")?);
                unsafe {
                    let _: () = msg_send![text,insertText:&*value,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            Some("retain-argument") => {
                *state.stale_text.borrow_mut() = Some(
                    state
                        .surface
                        .command
                        .arguments
                        .borrow()
                        .first()
                        .ok_or("No argument to retain")?
                        .clone(),
                );
            }
            Some("old-text-command") => {
                let old = state
                    .stale_text
                    .borrow_mut()
                    .take()
                    .ok_or("No retained text view")?;
                unsafe {
                    let _: () = msg_send![&*old,doCommandBySelector:sel!(cancelOperation:)];
                }
            }
            Some("editor-text-command") => {
                let text = if let Some(index) = request["argument"].as_u64() {
                    state
                        .surface
                        .command
                        .arguments
                        .borrow()
                        .get(index as usize)
                        .ok_or("No such argument")?
                        .clone()
                } else {
                    state.surface.command.text.clone()
                };
                state.panel.makeFirstResponder(Some(&*text));
                if request["marked"] == true {
                    unsafe {
                        let _: () = msg_send![&*text,setMarkedText:ns_string!("に"),selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                    }
                    let marked: bool = unsafe { msg_send![&*text, hasMarkedText] };
                    if !marked {
                        return Err("Text composition not established".into());
                    }
                }
                unsafe {
                    let _: () = msg_send![&*text,doCommandBySelector:sel!(cancelOperation:)];
                }
                if request["marked"] == true {
                    unsafe {
                        let _: () = msg_send![&*text, unmarkText];
                    }
                }
            }
            #[cfg(feature = "test-tools")]
            Some("schedule-input-submit") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let expire = request["expire"] == true;
                let marker = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .ok_or("Missing fixture path")?
                    .with_file_name("input-submit-fired");
                let ui = connection.clone();
                let owner = Weak::new(controller);
                state.timers.borrow_mut().push(crate::main_loop::Timer::new(
                    controller.mtm(),
                    0.1,
                    move || {
                        if let Some(owner) = owner.load() {
                            owner.save_editor();
                            let witnessed = if expire {
                                ui.expire_input_for_test()
                            } else {
                                owner.cancel_editor();
                                true
                            };
                            if witnessed {
                                let _ = std::fs::write(&marker, b"witnessed");
                            }
                        }
                    },
                )?);
            }
            Some("run-input-new-query") => {
                connection.query(request["query"].as_str().ok_or("Missing query")?);
            }
            Some("run-input-ime") => {
                let text = &state.surface.command.text;
                unsafe {
                    let _: () = msg_send![&**text,setMarkedText:ns_string!("に"),selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
                let marked: bool = unsafe { msg_send![&**text, hasMarkedText] };
                let before = connection.latest_intent();
                let mut handled = false;
                for (code, flags) in [
                    (36, NSEventModifierFlags::Command),
                    (53, NSEventModifierFlags::empty()),
                ] {
                    let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,ns_string!(""),ns_string!(""),false,code).ok_or("Missing fixture event")?;
                    handled |= controller.route_key(&event);
                }
                controller.save_editor();
                unsafe {
                    let _: () = msg_send![&**text, unmarkText];
                }
                if !marked || handled || before != connection.latest_intent() {
                    return Err("Run input interrupted native composition".into());
                }
                return Ok(serde_json::json!({"ok":true,"result":{"runAndEscapeIgnored":true}}));
            }
            Some("retain-input-button") => {
                if !state.session.borrow().input_menu_open() {
                    return Err("No input menu".into());
                }
                *state.stale_input_menu.borrow_mut() = state
                    .surface
                    .menu_buttons()
                    .into_iter()
                    .filter(|b| !b.isHidden())
                    .nth(request["index"].as_u64().unwrap_or(1) as usize);
            }
            Some("old-input-button") => {
                if let Some(button) = state.stale_input_menu.borrow().as_ref() {
                    unsafe {
                        button.performClick(None);
                    }
                }
            }
            Some("run-inline-selector") => {
                let editor = state.input.currentEditor().ok_or("No field editor")?;
                let text = editor.downcast_ref::<NSTextView>().ok_or("Wrong editor")?;
                let selector = match request["selector"].as_str() {
                    Some("line-break") => sel!(insertLineBreak:),
                    Some("newline-ignore") => sel!(insertNewlineIgnoringFieldEditor:),
                    _ => return Err("Unknown input selector".into()),
                };
                let handled: bool = unsafe {
                    msg_send![controller,control:&*state.input,textView:text,doCommandBySelector:selector]
                };
                if !handled {
                    return Err("Expand selector was not handled".into());
                }
            }
            #[cfg(feature = "test-tools")]
            Some("defer-input-child") => {
                state.defer_input_child.set(request["defer"] == true);
                return Ok(
                    serde_json::json!({"ok":true,"result":{"pendingChild":connection.view().editor.as_ref().is_some_and(|e| e.suspended_input_token.is_some())}}),
                );
            }
            #[cfg(feature = "test-tools")]
            Some("schedule-input-action-cancel") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let owner = Weak::new(controller);
                let marker = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .ok_or("Missing fixture path")?
                    .with_file_name("input-action-cancel-fired");
                state.timers.borrow_mut().push(crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = owner.load() { owner.guard_callback(|| {
                        owner.ivars().session.borrow().input_menu(Some("set-alias".into()), None);
                        owner.ivars().session.borrow_mut().key("cancelOperation:", false);
                        owner.reconcile();
                        if owner.ivars().input.isEnabled() {
                            if let Some(text) = owner.ivars().input.currentEditor().and_then(|e| e.downcast::<NSTextView>().ok()) {
                                unsafe { let _: () = msg_send![&*text,insertText:ns_string!(" continued"),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)]; }
                                let _ = std::fs::write(&marker, b"typed before owner acknowledgement");
                            }
                        }
                    }); }
                })?);
            }
            #[cfg(feature = "test-tools")]
            Some("schedule-inline-control") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let operation = match request["operation"].as_str() {
                    Some("expand") => ruycast_native::connected::InputOperation::Expand,
                    Some("menu") => ruycast_native::connected::InputOperation::Menu,
                    _ => return Err("Unknown input operation".into()),
                };
                let expire = request["expire"] == true;
                let cancel = request["cancel"] == true;
                let owner = Weak::new(controller);
                let ui = connection.clone();
                let marker = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .ok_or("Missing fixture path")?
                    .with_file_name("inline-control-fired");
                state.timers.borrow_mut().push(crate::main_loop::Timer::new(controller.mtm(),0.1,move||{if let Some(owner)=owner.load(){owner.guard_callback(||{
                    owner.inline_input_control(operation);
                    let witnessed=if expire {ui.expire_input_for_test()} else if cancel {
                        let cancelled=owner.cancel_pending_input_menu();
                        let text=owner.ivars().input.currentEditor().and_then(|e|e.downcast::<NSTextView>().ok());
                        if let Some(text)=text {unsafe{let _:()=msg_send![&*text,insertText:ns_string!(" continued"),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];}}
                        cancelled && owner.ivars().input.isEnabled()
                    } else {false};
                    if witnessed {let _=std::fs::write(&marker,b"witnessed");}
                });}})?);
            }
            Some("assert-hidden-order-block") => {
                let before = state.panel.ivars().blocked_orders.get();
                state.panel.orderFront(None);
                state.panel.orderBack(None);
                state.panel.makeKeyAndOrderFront(None);
                state.panel.orderFrontRegardless();
                state.panel.makeKeyWindow();
                state
                    .panel
                    .orderWindow_relativeTo(NSWindowOrderingMode::Above, 0);
                if state.panel.isVisible()
                    || state.panel.isKeyWindow()
                    || state.panel.isMainWindow()
                    || state.panel.canBecomeKeyWindow()
                    || state.panel.canBecomeMainWindow()
                    || state.panel.ivars().blocked_orders.get() < before + 6
                {
                    return Err("Hidden order prohibition failed".into());
                }
            }
            Some("run-inline-mark") | Some("query-composing") => {
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("No native field editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong field editor")?;
                let value = NSString::from_str(request["text"].as_str().unwrap_or("に"));
                unsafe {
                    let _: () = msg_send![text,setMarkedText:&*value,selectedRange:objc2_foundation::NSRange::new(value.length(),0),replacementRange:objc2_foundation::NSRange::new(0,text.string().length())];
                }
                if request["action"] == "query-composing" {
                    let note = unsafe {
                        NSNotification::notificationWithName_object(
                            ns_string!("NSControlTextDidChangeNotification"),
                            Some(&state.input),
                        )
                    };
                    unsafe {
                        let _: () = msg_send![controller,controlTextDidChange:&*note];
                    }
                }
            }
            Some("run-inline-unmark") | Some("query-commit") => {
                if let Some(text) = state
                    .input
                    .currentEditor()
                    .and_then(|e| e.downcast::<NSTextView>().ok())
                {
                    unsafe {
                        let _: () = msg_send![&*text, unmarkText];
                    }
                }
                if request["action"] == "query-commit" {
                    let note = unsafe {
                        NSNotification::notificationWithName_object(
                            ns_string!("NSControlTextDidChangeNotification"),
                            Some(&state.input),
                        )
                    };
                    unsafe {
                        let _: () = msg_send![controller,controlTextDidChange:&*note];
                    }
                }
            }
            Some("run-inline-ime-guards") => {
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("No inline field editor")?;
                let text = editor.downcast_ref::<NSTextView>().ok_or("Wrong editor")?;
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                if !marked {
                    return Err("Fixture has no marked text".into());
                }
                let intent = connection.latest_intent();
                for operation in [
                    ruycast_native::connected::InputOperation::Run,
                    ruycast_native::connected::InputOperation::Expand,
                    ruycast_native::connected::InputOperation::Menu,
                ] {
                    controller.inline_input_control(operation);
                }
                for (code, flags) in [
                    (36, NSEventModifierFlags::Command),
                    (53, NSEventModifierFlags::empty()),
                ] {
                    let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,ns_string!(""),ns_string!(""),false,code).ok_or("No fixture key")?;
                    if controller.route_key(&event) {
                        return Err("Inline composition key was intercepted".into());
                    }
                }
                if intent != connection.latest_intent() || state.surface.pending.get().is_some() {
                    return Err("Inline composition submitted input".into());
                }
            }
            Some("run-inline-read") => {
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("No inline field editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong inline field editor")?;
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                return Ok(
                    serde_json::json!({"ok":true,"result":{"text":text.string().to_string(),"marked":marked,"canUndo":text.undoManager().is_some_and(|u|u.canUndo()),"editorAddress":text as *const _ as usize,"selectedLocation":text.selectedRange().location,"selectedLength":text.selectedRange().length,"pending":state.surface.pending.get().is_some(),"enabled":state.input.isEnabled(),"query":state.session.borrow().query(),"token":state.run_inline_token.get()}}),
                );
            }
            Some("run-inline-type") => {
                if !state.input.isEnabled() || state.run_inline_token.get() == 0 {
                    return Err("Inline input is not editable".into());
                }
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("Inline input lost native focus")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong inline field editor")?;
                let value =
                    NSString::from_str(request["text"].as_str().ok_or("Missing inline text")?);
                unsafe {
                    let _: () = msg_send![text,insertText:&*value,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            Some("launcher-back") => {
                if state.launcher_back.isHidden() { return Err("Launcher Back is unavailable.".into()); }
                // SAFETY: This owned button targets this live main-thread Controller
                // with the matching launcherBack: signature established at construction.
                unsafe { state.launcher_back.performClick(None); }
            }
            Some("template-mark") => {
                state.surface.template.mark_field(
                    request["index"].as_u64().unwrap_or(0) as usize,
                    request["text"].as_str().ok_or("Missing text")?,
                )?;
            }
            Some("template-unmark") => {
                state.surface.template.unmark();
            }
            Some("template-scroll") => {
                state
                    .surface
                    .template
                    .scroll_to(request["y"].as_f64().unwrap_or(0.));
            }
            Some("template-retain") => {
                state
                    .surface
                    .template
                    .retain_field(request["index"].as_u64().unwrap_or(0) as usize)?;
            }
            Some("template-stale") => {
                return Ok(
                    serde_json::json!({"ok":true,"result":state.surface.template.stale_callbacks()?}),
                )
            }
            Some("template-read") => {
                return Ok(serde_json::json!({"ok":true,"result":state.surface.template.inspect()}))
            }
            Some("app-picker") => {
                if request["command"].as_str() == Some("read") {
                    return Ok(
                        serde_json::json!({"ok":true,"result":state.surface.command.app_read()}),
                    );
                }
                state.surface.command.app_probe(
                    request["command"]
                        .as_str()
                        .ok_or("Missing app choice action")?,
                    request["index"].as_u64().unwrap_or(0) as usize,
                )?;
            }
            Some("template-choice") => state.surface.template.choice_probe(
                request["command"].as_str().ok_or("Missing choice action")?,
                request["index"].as_u64().unwrap_or(0) as usize,
            )?,
            Some("template-type") => {
                state.surface.template.type_field(
                    request["index"].as_u64().ok_or("Missing field index")? as usize,
                    request["text"].as_str().ok_or("Missing text")?,
                    request["append"].as_bool().unwrap_or(false),
                )?;
            }
            Some("template-choose") => {
                state.surface.template.choose_field(
                    request["index"].as_u64().ok_or("Missing field index")? as usize,
                    request["selected"].as_u64().ok_or("Missing choice")? as usize,
                )?;
            }
            Some("run-input-read") => {
                if !state
                    .session
                    .borrow()
                    .surface()
                    .editor
                    .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Input)
                {
                    return Err("No run input is open".into());
                }
                let text = &state.surface.command.text;
                let focused = state.panel.firstResponder().is_some_and(|r| {
                    std::ptr::eq::<objc2::runtime::AnyObject>(
                        &*r as *const _ as *const _,
                        &**text as *const _ as *const _,
                    )
                });
                return Ok(
                    serde_json::json!({"ok":true,"result":{"text":text.string().to_string(),"editable":text.isEditable(),"focused":focused,"canUndo":text.undoManager().is_some_and(|m|m.canUndo()),"pending":state.surface.pending.get().is_some()}}),
                );
            }
            Some("run-input-type") => {
                let text = &state.surface.command.text;
                if !text.isEditable()
                    || !state.panel.firstResponder().is_some_and(|r| {
                        std::ptr::eq::<objc2::runtime::AnyObject>(
                            &*r as *const _ as *const _,
                            &**text as *const _ as *const _,
                        )
                    })
                {
                    return Err("Run input has no native typing focus".into());
                }
                let value =
                    NSString::from_str(request["text"].as_str().ok_or("Missing run input")?);
                unsafe {
                    let _: () = msg_send![&**text,insertText:&*value,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            Some("snippet-type") => {
                if state.surface.command.scroll.isHidden() {
                    return Err("Command editor is not open".into());
                }
                let text = &state.surface.command.text;
                state.panel.makeFirstResponder(Some(&**text));
                let value =
                    NSString::from_str(request["text"].as_str().ok_or("Missing snippet text")?);
                unsafe {
                    let _: () = msg_send![&**text,insertText:&*value,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            Some("snippet-undo-state") => {
                return Ok(
                    serde_json::json!({"ok":true,"result":{"canUndo":state.surface.command.text.undoManager().is_some_and(|m|m.canUndo())}}),
                );
            }
            Some("command-read") => {
                if state.surface.command.scroll.isHidden() {
                    return Err("Command editor is not open".into());
                }
                return Ok(
                    serde_json::json!({"ok":true,"result":{"value":state.surface.command.value(state.surface.record.value())?,"recording":state.surface.record.recording()}}),
                );
            }
            Some("editor-record") => {
                let record = &state.surface.record;
                let clip = state.surface.command.scroll.contentView();
                let frame = record.convertRect_toView(record.bounds(), Some(&clip));
                let bounds = clip.bounds();
                let point = NSPoint::new(frame.origin.x + frame.size.width / 2., frame.origin.y + frame.size.height / 2.);
                let parent = unsafe { record.superview() }.ok_or("Recorder detached")?;
                let content = state.panel.contentView().ok_or("Missing content")?;
                let local = record.convertPoint_toView(NSPoint::new(record.bounds().size.width/2.,record.bounds().size.height/2.), Some(&content));
                let hit = content.hitTest(local).is_some_and(|v| std::ptr::eq::<NSView>(&*v, &**record as *const _ as *const NSView));
                if request["click"].as_bool() == Some(true) { unsafe { record.performClick(None); } }
                return Ok(serde_json::json!({"ok":true,"result":{"frame":[record.frame().origin.x,record.frame().origin.y,record.frame().size.width,record.frame().size.height],"identity":format!("{:p}",&**record),"inDocument":state.surface.command.scroll.documentView().is_some_and(|doc|std::ptr::eq::<NSView>(&*parent,&*doc)),"hit":hit,"visibleCenter":point.x>=bounds.origin.x&&point.x<bounds.origin.x+bounds.size.width&&point.y>=bounds.origin.y&&point.y<bounds.origin.y+bounds.size.height,"recording":record.recording(),"preparing":record.preparing()}}));
            }
            Some("editor-layout") => { return Ok(serde_json::json!({"ok":true,"result":state.surface.command.geometry(request)})); }
            Some("editor-mark") | Some("editor-unmark") => {
                let text=&state.surface.command.text;state.panel.makeFirstResponder(Some(&**text));
                unsafe{if request["action"]=="editor-mark"{let _:()=msg_send![&**text,setMarkedText:ns_string!("日本"),selectedRange:objc2_foundation::NSRange::new(2,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];}else{let _:()=msg_send![&**text,unmarkText];}}
                return Ok(serde_json::json!({"ok":true,"result":state.surface.command.geometry(request)}));
            }
            Some("command-input") => {
                if state.surface.command.scroll.isHidden() {
                    return Err("Command editor is not open".into());
                }
                let key = request["key"].as_str().ok_or("Missing command field")?;
                match key {
                    "target"
                        if state
                            .session
                            .borrow()
                            .surface()
                            .editor
                            .is_some_and(|e| e.command["kind"] == "snippet") =>
                    {
                        state.surface.command.text.setString(&NSString::from_str(
                            request["text"].as_str().ok_or("Missing text")?,
                        ))
                    }
                    "snippetOutput" => state.surface.command.output.selectItemAtIndex(
                        match request["text"].as_str() {
                            Some("teams") => 1,
                            Some("html") => 2,
                            _ => 0,
                        },
                    ),
                    "enabled" => {
                        state
                            .surface
                            .command
                            .enabled
                            .setState(if request["value"] == false {
                                NSControlStateValueOff
                            } else {
                                NSControlStateValueOn
                            })
                    }
                    _ => {
                        let fields = state.surface.command.fields.borrow();
                        let field = fields
                            .iter()
                            .find(|f| f.key == key)
                            .ok_or("No such command field")?;
                        field.input.setStringValue(&NSString::from_str(
                            request["text"].as_str().ok_or("Missing text")?,
                        ));
                    }
                }
            }
            Some("command-argument") => {
                let args = state.surface.command.arguments.borrow();
                let arg = args
                    .get(request["index"].as_u64().ok_or("Missing argument")? as usize)
                    .ok_or("No such argument")?;
                arg.setString(&NSString::from_str(
                    request["text"].as_str().ok_or("Missing argument text")?,
                ));
            }
            Some("add-argument") => unsafe {
                state.surface.command.add_argument.performClick(None);
            },
            Some("remove-argument") => {
                let args = state.surface.command.remove_arguments.borrow();
                let button = args
                    .get(request["index"].as_u64().ok_or("Missing argument")? as usize)
                    .ok_or("No such argument")?
                    .clone();
                drop(args);
                unsafe {
                    button.performClick(None);
                }
            }
            Some("command-type") => {
                let field = state.surface.command.fields.borrow();
                let field = field.first().ok_or("Command editor is not open")?;
                let editor = field
                    .input
                    .currentEditor()
                    .ok_or("Command name lost focus")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong name editor")?;
                let value = NSString::from_str(request["text"].as_str().ok_or("Missing text")?);
                unsafe {
                    let _: () = msg_send![text,insertText:&*value,replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
            }
            Some("command-scroll") => {
                let clip = state.surface.command.scroll.contentView();
                let mut bounds = clip.bounds();
                bounds.origin = NSPoint::new(0., request["y"].as_f64().unwrap_or(0.));
                clip.scrollToPoint(clip.constrainBoundsRect(bounds).origin);
                state.surface.command.scroll.reflectScrolledClipView(&clip);
            }
            Some("alias-input") => {
                if state.surface.alias.isHidden() {
                    return Err("Alias editor is not open".into());
                }
                state.surface.alias.setStringValue(&NSString::from_str(
                    request["text"].as_str().ok_or("Missing alias")?,
                ));
            }
            Some("save-editor") => unsafe {
                state.surface.save.performClick(None);
            },
            Some("cancel-editor") => unsafe {
                state.surface.cancel.performClick(None);
            },
            Some("record") => unsafe {
                state.surface.record.performClick(None);
            },
            Some("recorder-menu") => {
                let record = match request["record"].as_str() {
                    Some("launcher") => state.settings.records[0].clone(),
                    Some("clipboard") => state.settings.records[1].clone(),
                    Some("inline") => state.surface.record.clone(),
                    _ => return Err("Unknown owned recorder".into()),
                };
                let menu = if request["held"].as_bool().unwrap_or(false) {
                    state.held_record_menu.borrow().clone()
                } else {
                    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        NSEventType::RightMouseDown, NSPoint::new(0., 0.),
                        NSEventModifierFlags::empty(), 0., state.panel.windowNumber(),
                        None, 1, 1, 1.,
                    ).ok_or("Owned menu event allocation failed")?;
                    let menu: Option<Retained<NSMenu>> = unsafe {
                        msg_send![&*record, menuForEvent: &*event]
                    };
                    *state.held_record_menu.borrow_mut() = menu.clone();
                    menu
                };
                let mut items = Vec::new();
                if let Some(menu) = menu {
                    menu.update();
                    items = menu.itemArray().iter().map(|item| serde_json::json!({
                        "title": item.title().to_string(), "enabled": item.isEnabled(),
                    })).collect();
                    if request["force"].as_bool().unwrap_or(false) {
                        let item = menu.itemAtIndex(0).ok_or("Owned menu item absent")?;
                        let target = item.target().ok_or("Owned menu target absent")?;
                        unsafe { let _: () = msg_send![&*target, useEitherCommand: &*item]; }
                    } else if request["invoke"].as_bool().unwrap_or(false) && menu.numberOfItems() > 0 {
                        menu.performActionForItemAtIndex(0);
                    }
                }
                return Ok(serde_json::json!({"ok":true,"result":{"menuItems": items, "value": record.value()}}));
            }
            Some("queued-recorder-hide") => {
                if state.panel.ivars().ordering_allowed || state.panel.ivars().management {
                    return Err("Queued recorder probes require an own hidden launcher".into());
                }
                let own: &NSWindow = &state.panel;
                let attached = || state.surface.record.window()
                    .is_some_and(|window| std::ptr::eq::<NSWindow>(&*window, own));
                let attached_before = attached();
                let (index, target_id, target_epoch, target_revision) = {
                    let model = state.session.borrow();
                    if !model.visible() || model.surface().editor.is_some()
                        || model.script_result().is_some() || model.dashboard().is_some()
                        || model.clipboard().is_some()
                        || !model.surface().items.iter().any(|item| item.id == "set-hotkey")
                        || state.surface.record.active() || !attached_before {
                        return Err("An attached idle recorder and eligible current target are required".into());
                    }
                    let index = model.selection().ok_or("Missing owned selected row")?;
                    let row = model.row(index).filter(|row| !row.is_group)
                        .ok_or("Missing owned selected target")?;
                    let Model::Live { view, .. } = &*model else {
                        return Err("Queued recorder probe requires the real live owner".into());
                    };
                    (index, row.id.to_string(), view.epoch, view.catalog_revision)
                };
                let pending_before = connection.pending_owned_inputs();
                if ["query", "actions", "close", "surface_cancel", "save", "preview", "transition", "native_probes"]
                    .iter().any(|key| pending_before[*key] != 0) || pending_before["overflow"] != false {
                    return Err("Queued recorder probe requires drained owned inputs".into());
                }
                let owner_before = connection.view();
                if !owner_before.visible || owner_before.editor.is_some()
                    || owner_before.epoch != target_epoch || owner_before.catalog_revision != target_revision {
                    return Err("Queued recorder probe requires an applied current owner snapshot".into());
                }
                let owner_revision_before = owner_before.revision;
                let intent_before = connection.latest_intent();
                let routing_before = connection.routing_revision();
                let dismissal_before = connection.latest_dismissal();
                let starts_before = state.recorder_acquire_starts.get();
                state.session.borrow_mut().named("set-hotkey", false, Some(index));
                // The model worker may prepare the draft concurrently. This
                // same main-thread turn cannot apply it or start its recorder.
                let intent_queued = connection.latest_intent();
                let routing_queued = connection.routing_revision();
                let dismissal_queued = connection.latest_dismissal();
                let pending_queued = connection.pending_owned_inputs();
                let attached_queued = attached();
                let native_editor_queued = state.session.borrow().surface().editor.is_some();
                let starts_queued = state.recorder_acquire_starts.get();
                state.session.borrow_mut().hide_palette();
                state.surface.stop();
                state.panel.orderOut(None);
                controller.reconcile();
                return Ok(serde_json::json!({"ok":true,"result":{
                    "phase":"pre-native-main-apply", "targetId":target_id,
                    "targetEpoch":target_epoch,"targetCatalogRevision":target_revision,
                    "ownerRevisionBefore":owner_revision_before,
                    "intentBefore":intent_before,"intentQueued":intent_queued,
                    "routingBefore":routing_before,"routingQueued":routing_queued,
                    "dismissalBefore":dismissal_before,"dismissalQueued":dismissal_queued,
                    "dismissalAfterHide":connection.latest_dismissal(),
                    "attachedBefore":attached_before,"attachedAfterQueue":attached_queued,
                    "nativeEditorAfterQueue":native_editor_queued,
                    "startsBefore":starts_before,"startsAfterQueue":starts_queued,
                    "startsAfterHide":state.recorder_acquire_starts.get(),
                    "pendingBefore":pending_before,"pendingQueued":pending_queued
                }}));
            }
            Some("outside-click") => {
                if state.panel.ivars().ordering_allowed || state.panel.ivars().management {
                    return Err("Outside-click probes require an own hidden launcher fixture".into());
                }
                let route = request["route"].as_str().ok_or("Missing owned outside-click route")?;
                if !matches!(route, "global-left" | "global-right" | "local-own" | "local-other" | "local-action-menu" | "retired" | "pending-query") {
                    return Err("Unknown owned outside-click route".into());
                }
                let before = state.outside.borrow().as_ref().map(|m| m.generation)
                    .ok_or("Owned outside-click lease is not armed")?;
                let query_epoch_before = connection.latest_epoch();
                let view_epoch_before = state.session.borrow().epoch();
                if route == "pending-query" {
                    connection.query("own pending outside-click query");
                }
                let query_epoch_after = connection.latest_epoch();
                let view_epoch_after = state.session.borrow().epoch();
                let returned = if route.starts_with("global-") || route == "pending-query" {
                    controller.outside_click(before, None, true);
                    None
                } else if route == "retired" {
                    controller.stop_outside_monitors();
                    controller.reconcile_outside_monitors();
                    controller.outside_click(before, None, true);
                    None
                } else {
                    let other = if route != "local-own" {
                        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                            NSPanel::alloc(controller.mtm()), rect(-20000., -20000., 20., 20.),
                            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                            NSBackingStoreType::Buffered, false);
                        unsafe { panel.setReleasedWhenClosed(false); }
                        Some(panel)
                    } else { None };
                    let window: &NSWindow = other.as_deref().map(|p| &**p).unwrap_or(&state.panel);
                    let parent = unsafe { state.surface.menu.superview() };
                    let frame = state.surface.menu.frame();
                    // Restore even if a callback unwinds. No window is ordered.
                    struct RestoreMenu<'a> {
                        menu: &'a NSView, parent: Option<Retained<NSView>>,
                        panel: Option<&'a NSPanel>, frame: NSRect,
                    }
                    impl Drop for RestoreMenu<'_> {
                        fn drop(&mut self) {
                            if let Some(panel) = self.panel {
                                panel.setContentView(None);
                                if let Some(parent) = &self.parent { parent.addSubview(self.menu); }
                                self.menu.setFrame(self.frame);
                            }
                        }
                    }
                    let _restore = RestoreMenu { menu: &state.surface.menu, parent,
                        panel: (route == "local-action-menu").then_some(other.as_deref()).flatten(), frame };
                    if route == "local-action-menu" { window.setContentView(Some(&state.surface.menu)); }
                    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        NSEventType::LeftMouseDown, NSPoint::new(0., 0.), NSEventModifierFlags::empty(),
                        0., window.windowNumber(), None, 1, 1, 1.,
                    ).ok_or("Owned outside-click event allocation failed")?;
                    if !event.window(controller.mtm()).as_ref().is_some_and(|actual| std::ptr::eq::<NSWindow>(&**actual, window)) {
                        return Err("Owned outside-click event lost its own window".into());
                    }
                    Some(controller.outside_local_event(before, &event) == Retained::as_ptr(&event).cast_mut())
                };
                return Ok(serde_json::json!({"ok":true,"result":{
                    "callbackReturnedSameEvent":returned,"beforeGeneration":before,
                    "afterGeneration":state.outside_generation.get(),
                    "queryEpochBefore":query_epoch_before,"queryEpochAfter":query_epoch_after,
                    "viewEpochBefore":view_epoch_before,"viewEpochAfter":view_epoch_after,
                    "visibleAfter":state.session.borrow().visible(),
                    "dismissalAfter":connection.latest_dismissal()
                }}));
            }
            Some("hide-palette") => {
                state.session.borrow_mut().hide_palette();
                state.surface.stop();
                state.panel.orderOut(None);
                controller.reconcile();
            }
            Some("resign-window") => unsafe {
                let _: () = msg_send![&*state.panel, resignKeyWindow];
            },
            Some("resign-app") => {
                let note = unsafe {
                    NSNotification::notificationWithName_object(
                        ns_string!("NSApplicationDidResignActiveNotification"),
                        None,
                    )
                };
                unsafe {
                    let _: () = msg_send![controller,applicationDidResignActive:&*note];
                }
            }
            Some("unfocus") => {
                state.panel.makeFirstResponder(None);
            }
            Some("action-button") => {
                let index = request["index"].as_u64().ok_or("Missing action")? as usize;
                let buttons = state.surface.menu_buttons();
                let button = buttons
                    .iter()
                    .filter(|b| !b.isHidden() && !state.surface.menu.isHidden())
                    .nth(index)
                    .ok_or("No such visible action")?;
                unsafe {
                    button.performClick(None);
                }
            }
            Some("retain-action-button") => {
                *state.stale_input_menu.borrow_mut() = state.surface.menu_buttons().into_iter()
                    .filter(|b| !b.isHidden() && !state.surface.menu.isHidden())
                    .nth(request["index"].as_u64().unwrap_or(0) as usize);
                if state.stale_input_menu.borrow().is_none() { return Err("No action to retain".into()); }
            }
            Some("key") => {
                let selector = match request["key"].as_str() {
                    Some("down") => sel!(moveDown:),
                    Some("up") => sel!(moveUp:),
                    Some("return") => sel!(insertNewline:),
                    Some("escape") => sel!(cancelOperation:),
                    _ => return Err("Unknown local key".into()),
                };
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("Missing native field editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong field editor")?;
                unsafe {
                    let _: bool = msg_send![controller,control:&*state.input,textView:text,doCommandBySelector:selector];
                }
            }
            Some("click") => {
                let index = request["row"].as_i64().ok_or("Missing row")? as isize;
                if index < 0 || index >= state.table.numberOfRows() {
                    return Err("No such native row".into());
                }
                state.table.scrollRowToVisible(index);
                let row = state.table.rectOfRow(index);
                let point = state.table.convertPoint_toView(
                    NSPoint::new(row.origin.x + 10., row.origin.y + 15.),
                    None,
                );
                let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(NSEventType::LeftMouseDown,point,NSEventModifierFlags::empty(),0.,state.panel.windowNumber(),None,1,1,1.).ok_or("Local click allocation failed")?;
                unsafe {
                    let _: () = msg_send![&*state.table,mouseDown:&*event];
                }
            }
            Some("execution-ime-escape") => {
                let editor = state.input.currentEditor().ok_or("Missing native editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong native editor")?;
                unsafe {
                    let _: () = msg_send![text,setMarkedText:ns_string!("に"),selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                let before = connection.latest_dismissal();
                let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),NSEventModifierFlags::empty(),0.,state.panel.windowNumber(),None,ns_string!(""),ns_string!(""),false,53).ok_or("Missing local Escape")?;
                let handled = controller.route_key(&event);
                unsafe {
                    let _: () = msg_send![text, unmarkText];
                }
                if !marked || handled || before != connection.latest_dismissal() {
                    return Err("Running Escape interrupted native composition".into());
                }
                return Ok(serde_json::json!({"ok":true,"result":{"imeEscapeIgnored":true}}));
            }
            Some("schedule-execution-intent") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let mode = request["mode"].as_str().ok_or("Missing mode")?.to_string();
                if !["query", "menu", "editor", "selection", "stop"].contains(&mode.as_str()) {
                    return Err("Unknown execution intent".into());
                }
                let marker = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .ok_or("Missing fixture path")?
                    .with_file_name("execution-intent-fired");
                let owner = Weak::new(controller);
                let connected = connection.clone();
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = owner.load() {
                        owner.guard_callback(|| {
                            match mode.as_str() {
                                "query" => {
                                    owner.ivars().input.setStringValue(ns_string!("finder"));
                                    let note = unsafe {
                                        NSNotification::notificationWithName_object(
                                            ns_string!("NSControlTextDidChangeNotification"),
                                            Some(&owner.ivars().input),
                                        )
                                    };
                                    unsafe {
                                        let _: () = msg_send![&*owner,controlTextDidChange:&*note];
                                    }
                                }
                                "menu" => owner.ivars().session.borrow_mut().toggle_actions(),
                                "editor" => {
                                    let row =
                                        usize::try_from(owner.ivars().table.selectedRow()).ok();
                                    owner.ivars().session.borrow_mut().named(
                                        "set-alias",
                                        false,
                                        row,
                                    );
                                }
                                "selection" => {
                                    owner.ivars().session.borrow_mut().select_row(Some(1))
                                }
                                "stop" => connected.request_stop(),
                                _ => unreachable!(),
                            }
                            owner.reconcile();
                            std::fs::write(&marker, b"fired")
                                .expect("own execution intent witness");
                        });
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("reject-stale-execution-view") => {
                let mut stale = connection.view();
                stale.completion_intent = Some(connection.latest_intent());
                stale.visible = false;
                state.session.borrow_mut().toggle_actions();
                let applied = apply_view(
                    controller,
                    connection,
                    stale,
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
                if applied || !state.session.borrow().visible() {
                    return Err("Stale execution view changed native state".into());
                }
            }
            Some("schedule-begin-recorder") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Owned keyboard service required".into());
                }
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let home = std::path::PathBuf::from(
                    std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME")
                        .ok_or("Missing owned keyboard fixture home")?,
                );
                let owner = Weak::new(controller);
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = owner.load() {
                        owner.guard_callback(|| {
                            owner.ivars().surface.record.begin();
                            let status = crate::native_keyboard::fixture_emit(49,6144,None);
                            let proof = serde_json::json!({"preparing":owner.ivars().surface.record.preparing(),"recording":owner.ivars().surface.record.recording(),"blockedCallback":status,"physicalHold":home.join("keyboard-register-entered").exists()});
                            std::fs::write(home.join("keyboard-nested-begin.json"),proof.to_string()).expect("Own nested recorder witness");
                            std::fs::write(home.join("keyboard-register-release"),b"release").expect("Own keyboard release");
                        });
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("schedule-pending-recorder") => {
                if state.keyboard.borrow().is_none() {
                    return Err("Owned keyboard service required".into());
                }
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let home = std::path::PathBuf::from(
                    std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME")
                        .ok_or("Missing owned keyboard fixture home")?,
                );
                let close = request["close"] == true;
                let owner = Weak::new(controller);
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = owner.load() {
                        owner.guard_callback(|| {
                            let state = owner.ivars();
                            let preparing = state.surface.record.preparing();
                            let recording = state.surface.record.recording();
                            snapshot(&owner, Some(&home.join("keyboard-pending.png")))
                                .expect("Own pending recorder render");
                            let text = screen_text(&owner);
                            let accessibility = accessibility_text(&owner);
                            let token = owner.recording_owner();
                            let mut consumed = vec![];
                            for (code, flags, chars) in [
                                (36, NSEventModifierFlags::Command, "\r"),
                                (36, NSEventModifierFlags::empty(), "\r"),
                            ] {
                                let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,&NSString::from_str(chars),&NSString::from_str(chars),false,code).expect("Own pending recorder event");
                                consumed.push(owner.route_key(&event));
                            }
                            let unchanged = token == owner.recording_owner()
                                && state.surface.pending.get().is_none()
                                && state.surface.record.preparing();
                            let (code, flags, chars) = if close {
                                (13, NSEventModifierFlags::Command, "w")
                            } else {
                                (53, NSEventModifierFlags::empty(), "")
                            };
                            let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.,0.),flags,0.,state.panel.windowNumber(),None,&NSString::from_str(chars),&NSString::from_str(chars),false,code).expect("Own pending recorder cancellation");
                            let cancelled = owner.route_key(&event)
                                && !state.surface.record.active();
                            let proof = serde_json::json!({"preparing":preparing,"recording":recording,"consumed":consumed,"unchanged":unchanged,"cancelled":cancelled,"physicalHold":home.join("keyboard-unregister-entered").exists(),"screenText":text,"accessibilityText":accessibility});
                            std::fs::write(home.join("keyboard-pending.json"),proof.to_string()).expect("Own pending recorder witness");
                            std::fs::write(home.join("keyboard-unregister-release"),b"release").expect("Own keyboard release");
                        });
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("schedule-save-cancel")
            | Some("schedule-save-hide")
            | Some("schedule-menu-cancel") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let owner = Weak::new(controller);
                let mode = request["action"].as_str().unwrap().to_string();
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.2, move || {
                    if let Some(owner) = owner.load() {
                        owner.guard_callback(|| {
                            if mode == "schedule-menu-cancel" {
                                owner
                                    .ivars()
                                    .session
                                    .borrow_mut()
                                    .key("insertNewline:", false);
                                owner
                                    .ivars()
                                    .session
                                    .borrow_mut()
                                    .key("cancelOperation:", false);
                                owner.reconcile();
                            } else {
                                owner.save_editor();
                                owner.cancel_editor();
                                if mode == "schedule-save-hide" {
                                    owner.ivars().session.borrow_mut().hide_palette();
                                    owner.ivars().panel.orderOut(None);
                                    owner.reconcile();
                                }
                            }
                        });
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("arm-slow-prefill") => {
                if !connection.arm_test_prefill() {
                    return Err("Hidden prefill fixture unavailable".into());
                }
            }
            Some("cancel-creation-origin") => {
                let token = state
                    .session
                    .borrow()
                    .surface()
                    .editor
                    .and_then(|e| e.origin_menu)
                    .ok_or("Editor has no originating menu")?;
                connection.action(ruycast_native::connected::Action::CloseActions { token });
                apply_view(
                    controller,
                    connection,
                    connection.view(),
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
            }
            Some("schedule-menu-escape") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let owner = Weak::new(controller);
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.2, move || {
                    if let Some(owner) = owner.load() {
                        owner.guard_callback(|| {
                            owner
                                .ivars()
                                .session
                                .borrow_mut()
                                .key("cancelOperation:", false);
                            owner.reconcile();
                        });
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("schedule-escape") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let connection = connection.clone();
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.2, move || {
                    connection.action(ruycast_native::connected::Action::Key(
                        "cancelOperation:".into(),
                    ))
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("ime") => {
                let editor = state
                    .input
                    .currentEditor()
                    .ok_or("Missing native field editor")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong field editor")?;
                unsafe {
                    let _: () = msg_send![text,setMarkedText:ns_string!("に"),selectedRange:objc2_foundation::NSRange::new(1,0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                }
                let before = state.input.stringValue().to_string();
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                if !marked {
                    return Err("Native marked text was not established".into());
                }
                let mut pending = connection.view();
                pending.epoch = connection.latest_epoch();
                pending.query = "different pending snapshot".into();
                apply_view(
                    controller,
                    connection,
                    pending,
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
                let handled: bool = unsafe {
                    msg_send![controller,control:&*state.input,textView:text,doCommandBySelector:sel!(insertNewline:)]
                };
                let preserved = state.input.stringValue().to_string() == before;
                unsafe {
                    let _: () = msg_send![text, unmarkText];
                }
                let note = unsafe {
                    NSNotification::notificationWithName_object(
                        ns_string!("NSControlTextDidChangeNotification"),
                        None,
                    )
                };
                unsafe {
                    let _: () = msg_send![controller,controlTextDidChange:&*note];
                }
                apply_view(
                    controller,
                    connection,
                    connection.view(),
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
                if handled || !preserved {
                    return Err("Pending snapshot/Return changed native composition".into());
                }
            }
            Some("stale-query") => {
                let old = connection.view();
                let query = request["query"].as_str().ok_or("Missing query")?;
                state.input.setStringValue(&NSString::from_str(query));
                let note = unsafe {
                    NSNotification::notificationWithName_object(
                        ns_string!("NSControlTextDidChangeNotification"),
                        None,
                    )
                };
                unsafe {
                    let _: () = msg_send![controller,controlTextDidChange:&*note];
                }
                let applied = apply_view(
                    controller,
                    connection,
                    old,
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
                if applied || state.input.stringValue().to_string() != query {
                    return Err("Older snapshot replaced newer native query".into());
                }
            }
            Some("select") => {
                let index = request["row"].as_u64().ok_or("Missing row")? as usize;
                if index >= state.table.numberOfRows() as usize {
                    return Err("No such native row".into());
                }
                state.table.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(index),
                    false,
                );
            }
            Some("screenshot") => {
                let output = state.probe_output.borrow();
                snapshot(
                    controller,
                    Some(output.as_deref().ok_or("No hidden probe output")?),
                )?;
            }
            Some("alias-undo-state") => {
                let editor = state
                    .surface
                    .alias
                    .currentEditor()
                    .ok_or("Alias lost focus")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong alias editor")?;
                return Ok(
                    serde_json::json!({"ok":true,"result":{"canUndo":text.undoManager().is_some_and(|m|m.canUndo())}}),
                );
            }
            Some("inline-control-read") => {
                let data = state.session.borrow().surface();
                let editor = data
                    .editor
                    .filter(|e| e.inline)
                    .ok_or("Missing inline editor")?;
                let field: &NSView = if editor.mode == ruycast_native::actions::Mode::Alias {
                    &state.surface.alias
                } else {
                    &state.surface.record
                };
                let visible =
                    objc2_foundation::NSIntersectionRect(field.visibleRect(), field.bounds());
                let shown = visible.size.width > 0. && visible.size.height > 0.;
                let content = state.panel.contentView().ok_or("Missing content")?;
                let mut hit_editor = false;
                if shown {
                    let point = content.convertPoint_fromView(
                        NSPoint::new(
                            visible.origin.x + visible.size.width / 2.,
                            visible.origin.y + visible.size.height / 2.,
                        ),
                        Some(field),
                    );
                    let hit = content.hitTest(point).ok_or("Inline editor was not hit")?;
                    hit_editor = std::ptr::eq::<objc2::runtime::AnyObject>(
                        &*hit as *const _ as *const _,
                        field as *const _ as *const _,
                    ) || state.surface.alias.currentEditor().is_some_and(|e| {
                        std::ptr::eq::<objc2::runtime::AnyObject>(
                            &*hit as *const _ as *const _,
                            &*e as *const _ as *const _,
                        )
                    });
                    if !hit_editor {
                        return Err(format!(
                            "Inline input intercepted by {:?}",
                            hit.class().name()
                        ));
                    }
                }
                let anchor = {
                    let model = state.session.borrow();
                    (0..model.row_count()).find(|&i| {
                        model
                            .row(i)
                            .is_some_and(|r| !r.is_group && r.id == editor.id)
                    })
                };
                let underlying_hidden = anchor
                    .and_then(|row| {
                        state.table.viewAtColumn_row_makeIfNecessary(
                            if editor.mode == ruycast_native::actions::Mode::Alias {
                                1
                            } else {
                                2
                            },
                            row as isize,
                            true,
                        )
                    })
                    .map(|view| view.isHidden());
                return Ok(
                    serde_json::json!({"ok":true,"result":{"visible":shown,"hitEditor":hit_editor,"underlyingHidden":underlying_hidden,"autosaveSlots":usize::from(state.inline_autosave.borrow().is_some())}}),
                );
            }
            Some("inline-repeat-focus") => {
                for _ in 0..50 {
                    state.panel.makeFirstResponder(Some(&state.input));
                    state.panel.makeFirstResponder(Some(&state.surface.alias));
                }
            }
            Some("inline-tracking-switch") => {
                let trace = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .ok_or("Own trace output missing")?
                    .with_file_name("inline-tracking-trace.jsonl");
                std::fs::write(trace, b"").map_err(|e| e.to_string())?;
                inline_tracking_trace(controller, "tracking-probe-start");
                use objc2_core_foundation::{CFRunLoop, CFString};
                let mode = CFString::from_static_str("NSEventTrackingRunLoopMode");
                let run_loop = CFRunLoop::main().ok_or("Missing main loop")?;
                run_loop.add_common_mode(Some(&mode));
                let tick = std::rc::Rc::new(Cell::new(false));
                let fired = tick.clone();
                let _timer =
                    crate::main_loop::Timer::new(controller.mtm(), 0.005, move || fired.set(true))?;
                state.panel.makeFirstResponder(Some(&state.input));
                CFRunLoop::run_in_mode(Some(&mode), 0.02, false);
                if !tick.get() || state.surface.pending.get().is_some() {
                    return Err(
                        "Autosave ran during native tracking mode or tracking was not exercised"
                            .into(),
                    );
                }
                inline_tracking_trace(controller, "tracking-probe-end");
                return Ok(native_probe(
                    controller,
                    connection,
                    &serde_json::json!({"action":"inline-alias","id":request["id"]}),
                ));
            }
            Some("schedule-inline-cancel") => {
                state.timers.borrow_mut().retain(|timer| timer.valid());
                if state.timers.borrow().len() >= 16 {
                    return Err("Native scheduler is busy".into());
                }
                let owner = Weak::new(controller);
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = owner.load() {
                        owner.cancel_editor();
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("cancel-inline-origin") => {
                let origin = state
                    .session
                    .borrow()
                    .surface()
                    .editor
                    .and_then(|e| e.origin_inline)
                    .ok_or("Missing inline origin")?;
                connection
                    .action(ruycast_native::connected::Action::CancelEditor { token: origin });
                apply_view(
                    controller,
                    connection,
                    connection.view(),
                    true,
                    &NSApplication::sharedApplication(controller.mtm()),
                );
            }
            Some("inline-stale-switch") => {
                let data = state.session.borrow().surface();
                let editor = data.editor.filter(|e| e.inline).ok_or("No inline draft")?;
                let target = ruycast_native::actions::Target {
                    id: "app.safari".into(),
                    epoch: connection.latest_epoch(),
                    catalog_revision: 0,
                };
                let request = connection
                    .begin_inline(
                        target,
                        ruycast_native::actions::Mode::Alias,
                        Some((
                            editor.token,
                            Some(state.surface.alias.stringValue().to_string()),
                            None,
                        )),
                    )
                    .ok_or("Missing transition acknowledgement")?;
                state.surface.pending.set(Some(request));
                state.surface.set_alias_enabled(false);
            }
            Some("inline-mark") | Some("inline-unmark") | Some("inline-marked-read") => {
                let editor = state
                    .surface
                    .alias
                    .currentEditor()
                    .ok_or("Inline alias lost focus")?;
                let text = editor
                    .downcast_ref::<NSTextView>()
                    .ok_or("Wrong inline editor")?;
                if request["action"] == "inline-mark" {
                    let value = NSString::from_str(request["text"].as_str().unwrap_or("に"));
                    unsafe {
                        let _: () = msg_send![text,setMarkedText:&*value,selectedRange:objc2_foundation::NSRange::new(value.length(),0),replacementRange:objc2_foundation::NSRange::new(usize::MAX,0)];
                    }
                }
                if request["action"] == "inline-unmark" {
                    unsafe {
                        let _: () = msg_send![text, unmarkText];
                    }
                }
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                return Ok(
                    serde_json::json!({"ok":true,"result":{"marked":marked,"text":text.string().to_string(),"inlineVisible":state.surface.inline_visible.get(),"formVisible":!state.surface.form.isHidden(),"saveEnabled":state.surface.save.isEnabled(),"runInputInline":state.session.borrow().surface().editor.is_some_and(|e|e.run_inline),"editorToken":state.session.borrow().surface().editor.map(|e|e.token)}}),
                );
            }
            Some("inline-focus-search") => {
                state.panel.makeFirstResponder(Some(&state.input));
            }
            Some("inline-scroll") => {
                let clip = state.scroll.contentView();
                let mut bounds = clip.bounds();
                bounds.origin.y = request["y"].as_f64().unwrap_or(0.);
                clip.scrollToPoint(clip.constrainBoundsRect(bounds).origin);
                state.scroll.reflectScrolledClipView(&clip);
            }
            Some("inline-alias") | Some("inline-hotkey") => {
                let id = request["id"].as_str().ok_or("Missing command ID")?;
                let index = {
                    let model = state.session.borrow();
                    (0..model.row_count())
                        .find(|&i| model.row(i).is_some_and(|r| !r.is_group && r.id == id))
                        .ok_or("Missing library command")?
                };
                state.table.scrollRowToVisible(index as isize);
                let column = if request["action"] == "inline-alias" {
                    1
                } else {
                    2
                };
                let view = state
                    .table
                    .viewAtColumn_row_makeIfNecessary(column, index as isize, true)
                    .ok_or("Missing binding control")?;
                let button = view
                    .downcast::<crate::native_library::BindingButton>()
                    .map_err(|_| "Wrong binding control")?;
                let content = state.panel.contentView().ok_or("Missing content")?;
                let bounds = button.bounds();
                let point = content.convertPoint_fromView(
                    NSPoint::new(
                        bounds.origin.x + bounds.size.width / 2.,
                        bounds.origin.y + bounds.size.height / 2.,
                    ),
                    Some(&button),
                );
                let hit = content.hitTest(point).ok_or("Binding was not hit")?;
                if !std::ptr::eq::<objc2::runtime::AnyObject>(
                    &*hit as *const _ as *const _,
                    &*button as *const _ as *const _,
                ) {
                    return Err(format!("Binding intercepted by {:?}", hit.class().name()));
                }
                inline_tracking_trace(controller, "before-performClick");
                unsafe {
                    button.performClick(None);
                }
                inline_tracking_trace(controller, "after-performClick");
            }
            Some("old-library-enable") => {
                let button = state
                    .stale_enabled
                    .borrow()
                    .clone()
                    .ok_or("No retained enabled control")?;
                unsafe {
                    button.performClick(None);
                }
            }
            Some("library-enable")
            | Some("retain-library-enabled")
            | Some("library-enabled-read") => {
                let id = request["id"].as_str().ok_or("Missing command ID")?;
                let index = {
                    let model = state.session.borrow();
                    (0..model.row_count())
                        .find(|&i| model.row(i).is_some_and(|r| !r.is_group && r.id == id))
                        .ok_or("Missing library command")?
                };
                state.table.scrollRowToVisible(index as isize);
                let view = state
                    .table
                    .viewAtColumn_row_makeIfNecessary(4, index as isize, true)
                    .ok_or("Missing enabled control")?;
                let button = view
                    .downcast::<crate::native_library::EnabledButton>()
                    .map_err(|_| "Wrong enabled control")?;
                let content = state.panel.contentView().ok_or("Missing native content")?;
                let bounds = button.bounds();
                let center = NSPoint::new(
                    bounds.origin.x + bounds.size.width / 2.,
                    bounds.origin.y + bounds.size.height / 2.,
                );
                let point = content.convertPoint_fromView(center, Some(&button));
                let hit = content.hitTest(point).ok_or("Checkbox was not hit")?;
                if !std::ptr::eq::<objc2::runtime::AnyObject>(
                    &*hit as *const _ as *const _,
                    &*button as *const _ as *const _,
                ) {
                    return Err(format!("Checkbox intercepted by {:?}", hit.class().name()));
                }
                if request["action"] == "library-enabled-read" {
                    return Ok(
                        serde_json::json!({"ok":true,"result":{"checked":button.state()==NSControlStateValueOn,"label":NSAccessibility::accessibilityLabel(&**button).map(|s|s.to_string()),"hitCheckbox":true}}),
                    );
                }
                if request["action"] == "retain-library-enabled" {
                    *state.stale_enabled.borrow_mut() = Some(button);
                } else {
                    unsafe {
                        button.performClick(None);
                    }
                }
            }
            Some("library-read") => {
                let model = state.session.borrow();
                let Model::Live { view, .. } = &*model else {
                    return Err("Live library is unavailable".into());
                };
                if !model.management() {
                    return Err("Not a Settings window".into());
                }
                let rows=view.rows.iter().map(|r|serde_json::json!({"id":r.id,"title":r.title,"group":r.is_group,"enabled":r.enabled,"aliases":r.aliases,"hotkey":r.hotkey})).collect::<Vec<_>>();
                return Ok(
                    serde_json::json!({"ok":true,"result":{"rows":rows,"tab":view.library.as_ref().map(|n|&n.tab),"filter":view.library.as_ref().map(|n|&n.filter),"query":view.query,"visible":view.visible,"editorToken":view.editor.as_ref().map(|e|e.token),"selectedRow":state.table.selectedRow()}}),
                );
            }
            Some("library-tab") => {
                let bar = state.toolbar.as_ref().ok_or("Not a Settings window")?;
                let index = crate::native_library::TABS
                    .iter()
                    .position(|tab| Some(*tab) == request["tab"].as_str())
                    .ok_or("Unknown library tab")?;
                unsafe {
                    bar.tabs[index].performClick(None);
                }
            }
            Some("library-filter") => {
                let bar = state.toolbar.as_ref().ok_or("Not a Settings window")?;
                let index = crate::native_library::FILTERS
                    .iter()
                    .position(|f| Some(*f) == request["filter"].as_str())
                    .ok_or("Unknown filter")?;
                bar.filter.selectItemAtIndex(index as isize);
                unsafe {
                    let _: () = msg_send![controller,filterLibrary:&*bar.filter];
                }
            }
            Some("library-add") => {
                let bar = state.toolbar.as_ref().ok_or("Not a Settings window")?;
                let index = match request["kind"].as_str() {
                    Some("snippet") => 1,
                    Some("url") => 2,
                    Some("script") => 3,
                    _ => return Err("Unknown creation kind".into()),
                };
                bar.add.selectItemAtIndex(index);
                unsafe {
                    let _: () = msg_send![controller,addLibraryCommand:&*bar.add];
                }
            }
            Some("fixture-force-visible") => state.panel.fixture_force_visible()?,
            Some("window-close") => state.panel.close(),
            Some("source-reentry") => {
                #[cfg(feature = "test-tools")]
                {
                    let deferrals = crate::main_loop::check_reentry(controller.mtm())?;
                    return Ok(
                        serde_json::json!({"ok":true,"result":{"deferredDeliveries":deferrals}}),
                    );
                }
                #[cfg(not(feature = "test-tools"))]
                return Err("Test source unavailable".into());
            }
            Some("rich-query-type")
            | Some("rich-query-read")
            | Some("rich-query-mark")
            | Some("rich-query-unmark") => {
                if !state
                    .session
                    .borrow()
                    .script_result()
                    .is_some_and(|r| !r.detail_visible && !r.busy)
                {
                    return Err("Result search unavailable".into());
                }
                return Ok(
                    serde_json::json!({"ok":true,"result":state.result_panel.query_probe(request["action"].as_str().unwrap_or(""),request["text"].as_str())}),
                );
            }
            Some("rich-query-native-event") => {
                let event=NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(NSEventType::ApplicationDefined,NSPoint::new(0.,0.),NSEventModifierFlags::empty(),0.,0,None,0,0,0).ok_or("Own application event unavailable")?;
                NSApplication::sharedApplication(controller.mtm()).postEvent_atStart(&event, false);
            }
            Some("retain-result-query") => state.result_panel.retain_query(),
            Some("stale-result-query") => {
                return Ok(
                    serde_json::json!({"ok":true,"result":state.result_panel.exercise_stale_query()}),
                )
            }
            Some("retain-result-text") => state.result_panel.retain_text(),
            Some("stale-result-text") => state.result_panel.stale_text_escape(),
            Some("schedule-result-close") => {
                let query_before = request["queryBeforeClose"] == true;
                let snapshot = connection.view();
                let token = snapshot
                    .script_result
                    .as_ref()
                    .ok_or("Missing result")?
                    .token;
                let path = state
                    .probe_output
                    .borrow()
                    .as_ref()
                    .and_then(|p| p.parent())
                    .ok_or("Missing fixture profile")?
                    .join("result-close-barrier.json");
                let weak = Weak::new(controller);
                let connection = connection.clone();
                let timer = crate::main_loop::Timer::new(controller.mtm(), 0.1, move || {
                    if let Some(owner) = weak.load() {
                        let before = connection.latest_epoch();
                        if query_before {
                            let _ = owner
                                .ivars()
                                .result_panel
                                .query_probe("rich-query-type", Some("queued input"));
                        }
                        let query_advanced = connection.latest_epoch() > before;
                        owner.result_control(token, "close");
                        let rejected = !apply_view(
                            &owner,
                            &connection,
                            snapshot.clone(),
                            true,
                            &NSApplication::sharedApplication(owner.mtm()),
                        );
                        let _=std::fs::write(&path,serde_json::json!({"rejected":rejected,"windowShown":owner.ivars().panel.isVisible(),"queryBeforeClose":query_before,"queryEpochAdvanced":query_advanced}).to_string());
                    }
                })?;
                state.timers.borrow_mut().push(timer);
            }
            Some("result-control") => {
                if !state
                    .result_panel
                    .click(request["name"].as_str().unwrap_or(""))
                {
                    return Err("Result control unavailable".into());
                }
            }
            Some("retain-result-button") => {
                if !state
                    .result_panel
                    .retain_button(request["name"].as_str().unwrap_or(""))
                {
                    return Err("Result control unavailable".into());
                }
            }
            Some("stale-result-button") => state.result_panel.click_stale(),
            Some("result-query-event") => connection.query_result(
                request["token"].as_u64().unwrap_or(0),
                request["query"].as_str().unwrap_or(""),
                false,
            ),
            Some("result-event") => connection.action(ruycast_native::connected::Action::Result {
                token: request["token"].as_u64().unwrap_or(0),
                epoch: request["epoch"].as_u64().unwrap_or(0),
                name: request["name"].as_str().unwrap_or("").into(),
                id: request["id"].as_str().map(str::to_owned),
            }),
            Some("menu-bar-read")=>return Ok(serde_json::json!({"ok":true,"result":state.menu_bar.probe()})),
            Some("menu-bar-click")=>{if !state.menu_bar.click(request["index"].as_u64().unwrap_or(0) as usize){return Err("Menu item unavailable".into());}},
            Some("menu-bar-retain")=>state.menu_bar.retain(),
            Some("menu-bar-stale")=>state.menu_bar.stale(),
            Some("settings-read") => return Ok(serde_json::json!({"ok":true,"result":state.settings.probe()})),
            Some("settings-retain")=>state.settings.retain(request["index"].as_u64().unwrap_or(0) as usize),
            Some("settings-stale")=>state.settings.stale(),
            Some("settings-control")=>{if !state.settings.click(request["index"].as_u64().unwrap_or(0) as usize){return Err("Setting unavailable".into());}},
            Some("settings-theme")=>{if !state.settings.theme(request["index"].as_i64().unwrap_or(0) as isize){return Err("Theme unavailable".into());}},
            Some("settings-record")=>state.settings.record(controller,request["index"].as_u64().unwrap_or(0) as usize),
            Some("settings-folder-complete")=>{
                let accepted=state.settings.finish_dialog(request["serial"].as_u64().unwrap_or(0));
                if accepted&&request["accepted"]==true{if let Some(path)=request["path"].as_str(){controller.settings_directory(path.into());}}
                return Ok(serde_json::json!({"ok":true,"result":{"accepted":accepted}}));
            },
            Some("settings-screenshot")=>{let path=state.probe_output.borrow().clone().ok_or("Missing owned output")?.with_file_name("native-settings-preferences-probe.png");state.settings.snapshot(&path)?;return Ok(serde_json::json!({"ok":true,"result":state.settings.probe()}));},
            Some("clipboard-fixture")=>{let callback=state.clipboard_fixture.borrow().clone().ok_or("Owned clipboard fixture unavailable.")?;return Ok(serde_json::json!({"ok":true,"result":callback(request)?}));},
            Some("clipboard-preferences-save")=>{if !state.clipboard_preferences.fixture_save(request){return Err("Clipboard preferences unavailable.".into());}},
            Some("clipboard-preferences-probe")=>{return Ok(serde_json::json!({"ok":true,"result":state.clipboard_preferences.probe(request["operation"].as_str().unwrap_or("read"),request["y"].as_f64().unwrap_or(0.))?}));},
            Some("clipboard-preferences-screenshot")=>{let path=state.probe_output.borrow().clone().ok_or("Missing owned probe output")?.with_file_name("native-clipboard-preferences-probe.png");state.clipboard_preferences.snapshot(&path)?;return Ok(serde_json::json!({"ok":true,"result":state.clipboard_preferences.probe("read",0.)?}));},
            Some("clipboard-preferences")=>{controller.library_navigate("tab","clipboard");},
            Some("clipboard-control")=>{let model=state.session.borrow();let s=model.clipboard().ok_or("Clipboard unavailable.")?;model.clipboard_action(request["operation"].as_str().unwrap_or(""),s.selected_id.clone(),s.revision,model.epoch(),model.dismissal());},
            Some("clipboard-retained-control")=>{state.clipboard_panel.probe(request["operation"].as_str().unwrap_or("read"))?;},
            Some("palette-geometry")=>{let a=request["visible"].as_array().filter(|a|a.len()==4).ok_or("Invalid fixture geometry")?;let mut f=[0.;4];for(i,v)in a.iter().enumerate(){f[i]=v.as_f64().filter(|v|v.is_finite()).ok_or("Invalid fixture geometry")?;}controller.layout_for_mode(request["clipboard"]==true,Some(rect(f[0],f[1],f[2],f[3])));controller.reconcile();let f=state.panel.frame();return Ok(serde_json::json!({"ok":true,"result":{"frame":[f.origin.x,f.origin.y,f.size.width,f.size.height]}}));},
            Some("dashboard-read") => return Ok(serde_json::json!({"ok":true,"result":state.dashboard.probe()})),
            Some("process-sort")=>state.process_controls.choose(request["index"].as_i64().unwrap_or(0) as isize),
            Some("monitor-read")=>return state.monitor.borrow().as_ref().map(|m|serde_json::json!({"ok":true,"result":m.probe()})).ok_or("Monitor window unavailable".into()),
            Some("monitor-control")=>{if !state.monitor.borrow().as_ref().is_some_and(|m|m.click(request["index"].as_u64().unwrap_or(0) as usize)){return Err("Monitor control unavailable".into());}},
            Some("monitor-lifecycle")=>{if !state.monitor.borrow().as_ref().is_some_and(|m|m.lifecycle(request["event"].as_str().unwrap_or(""))){return Err("Monitor lifecycle unavailable".into());}},
            Some("monitor-render-state")=>{let path=state.probe_output.borrow().clone().ok_or("Missing owned output")?.with_file_name("native-monitor-probe.png");return state.monitor.borrow().as_ref().ok_or("Monitor unavailable")?.render_state(request["state"].as_str().unwrap_or(""),&path).map(|result|serde_json::json!({"ok":true,"result":result}));},
            Some("monitor-size")=>{let width=request["width"].as_f64().filter(|v|(740.0..=1200.0).contains(v)).ok_or("Invalid monitor width")?;let height=request["height"].as_f64().filter(|v|(500.0..=900.0).contains(v)).ok_or("Invalid monitor height")?;state.monitor.borrow().as_ref().ok_or("Monitor unavailable")?.resize(width,height);},
            Some("monitor-screenshot")=>{let path=state.probe_output.borrow().clone().ok_or("Missing owned output")?.with_file_name("native-monitor-probe.png");let monitor=state.monitor.borrow();let monitor=monitor.as_ref().ok_or("Monitor unavailable")?;monitor.snapshot(&path)?;return Ok(serde_json::json!({"ok":true,"result":monitor.probe()}));},
            Some("dashboard-control") => {
                if !state.dashboard.click(request["index"].as_u64().unwrap_or(0) as usize){return Err("Dashboard control unavailable".into());}
            }
            Some("dashboard-focus")=>{state.dashboard.focus(request["index"].as_u64().unwrap_or(0) as usize);},
            Some("dashboard-next-focus")=>state.dashboard.advance_focus(false),
            Some("dashboard-retention") => {if !state.dashboard.retention(request["days"].as_i64().unwrap_or(0)){return Err("Retention control unavailable".into());}}
            Some("dashboard-retain") => state.dashboard.retain_control(request["index"].as_u64().unwrap_or(1) as usize),
            Some("dashboard-stale") => {state.dashboard.click_stale();}
            Some("dashboard-dialog-complete") => {
                let path=request["path"].as_str().map(std::path::PathBuf::from);
                let accepted=state.dashboard.complete_fixture(controller,request["serial"].as_u64().unwrap_or(0),request["token"].as_u64().unwrap_or(0),request["accepted"]==true,path);
                return Ok(serde_json::json!({"ok":true,"result":{"accepted":accepted}}));
            }
            Some("dashboard-scroll") => state.dashboard.scroll_to(request["y"].as_f64().unwrap_or(0.)),
            Some("dashboard-size") => {
                let width=request["width"].as_f64().filter(|v|(600.0..=1200.0).contains(v)).ok_or("Invalid owned dashboard width")?;
                let height=request["height"].as_f64().filter(|v|(480.0..=900.0).contains(v)).unwrap_or(650.);
                state.panel.setContentSize(NSSize::new(width,height));controller.reconcile();
            }
            Some("dashboard-screenshot") => {
                let path=state.probe_output.borrow().clone().ok_or("Missing owned probe output")?.with_file_name("native-dashboard-probe.png");
                state.dashboard.snapshot(&path)?;
                return Ok(serde_json::json!({"ok":true,"result":state.dashboard.probe()}));
            }
            Some("read") => {}
            _ => return Err("Unknown native probe action".into()),
        }
        let menu_controls=state.surface.menu_buttons().iter().filter(|b|!b.isHidden() && !state.surface.menu.isHidden()).map(|button|{
            let frame=button.frame();
            let content=state.panel.contentView();
            let center=content.as_ref().map(|v|v.convertPoint_fromView(NSPoint::new(button.bounds().size.width/2.,button.bounds().size.height/2.),Some(button)));
            let hit=content.as_ref().zip(center).and_then(|(v,p)|v.hitTest(p)).is_some_and(|h|std::ptr::eq::<objc2::runtime::AnyObject>(&*h as *const _ as *const _,&**button as *const _ as *const _));
            let inside=content.as_ref().zip(center).is_some_and(|(v,p)|{let b=v.bounds();p.x>=b.origin.x&&p.y>=b.origin.y&&p.x<b.origin.x+b.size.width&&p.y<b.origin.y+b.size.height});
            serde_json::json!({"title":button.title().to_string(),"frame":[frame.origin.x,frame.origin.y,frame.size.width,frame.size.height],"hit":hit,"inside":inside})
        }).collect::<Vec<_>>();
        let mut response = serde_json::json!({"ok":true,"result":{"rows":state.table.numberOfRows(),"selectedRow":state.table.selectedRow(),"selectedId":state.session.borrow().selected().map(|row|row.id.to_string()),"actionSelection":state.session.borrow().surface().action_selection,"menuControls":menu_controls,"responderMenu":state.panel.firstResponder().is_some_and(|r|std::ptr::eq::<objc2::runtime::AnyObject>(&*r as *const _ as *const _,&*state.surface.menu as *const _ as *const _)),"responderSearch":state.panel.firstResponder().is_some_and(|r|std::ptr::eq::<objc2::runtime::AnyObject>(&*r as *const _ as *const _,&*state.input as *const _ as *const _) || r.downcast_ref::<NSTextView>().and_then(|t|t.delegate()).is_some_and(|d|std::ptr::eq::<objc2::runtime::AnyObject>(&*d as *const _ as *const _,&*state.input as *const _ as *const _))),"input":state.input.stringValue().to_string(),"modelQuery":state.session.borrow().query(),"scriptResult":state.session.borrow().script_result(),"resultPanel":state.result_panel.probe(),"appearance":state.panel.effectiveAppearance().name().to_string(),"explicitAppearance":state.panel.appearance().map(|a|a.name().to_string()),"windowShown":state.panel.isVisible(),"blockedOrders":state.panel.ivars().blocked_orders.get(),"orderingAllowed":state.panel.ivars().ordering_allowed,"hiddenVisibilityRepairs":state.panel.ivars().hidden_repairs.get(),"lastHiddenVisibilityRepair":state.panel.ivars().last_hidden_repair.borrow().clone(),"windowKey":state.panel.isKeyWindow(),"windowMain":state.panel.isMainWindow(),"prefillTestRuns":connection.test_prefill_runs(),"actionsVisible":!state.surface.menu.isHidden(),"actionLabels":state.surface.menu_buttons().iter().filter(|b|!b.isHidden() && !state.surface.menu.isHidden()).map(|b|b.title().to_string()).collect::<Vec<_>>(),"formVisible":!state.surface.form.isHidden(),"saveEnabled":state.surface.save.isEnabled(),"runInputInline":state.session.borrow().surface().editor.is_some_and(|e|e.run_inline),"inlineVisible":state.surface.inline_visible.get(),"aliasValue":state.surface.alias.stringValue().to_string(),"recording":state.surface.record.recording(),"recordingPreparing":state.surface.record.preparing(),"editorSaving":state.session.borrow().editor_saving(),"editorRequestPending":state.surface.pending.get().is_some(),"commandScrollY":state.surface.command.scroll.contentView().bounds().origin.y,"commandScrollMaxY":state.surface.command.scroll.documentView().map(|v|(v.frame().size.height-state.surface.command.scroll.contentView().bounds().size.height).max(0.)),"editorToken":state.session.borrow().surface().editor.map(|e|e.token),"editorId":state.session.borrow().surface().editor.map(|e|e.id),"screenText":screen_text(controller),"accessibilityText":accessibility_text(controller)}});
        let recorder = serde_json::json!({"recorderResponder":state.panel.firstResponder().is_some_and(|r|std::ptr::eq::<objc2::runtime::AnyObject>(&*r as *const _ as *const _,&*state.surface.record as *const _ as *const _)),"recorderAccessibilityLabel":NSAccessibilityButton::accessibilityLabel(&**state.surface.record).map(|label|label.to_string()),"recorderAcquireStarts":state.recorder_acquire_starts.get(),"recorderPendingInputs":connection.pending_owned_inputs(),"keyEventHandled":state.key_event_handled.get()});
        response["result"].as_object_mut().expect("Native read object").extend(recorder.as_object().expect("Recorder read object").clone());
        let lifecycle = serde_json::json!({"releasedWhenClosed":state.panel.isReleasedWhenClosed(),"panelOpaque":state.panel.isOpaque(),"panelHasShadow":state.panel.hasShadow(),"panelBackgroundAlpha":state.panel.backgroundColor().alphaComponent(),"panelLevel":state.panel.level(),"movableByWindowBackground":state.panel.isMovableByWindowBackground(),"collectionBehavior":state.panel.collectionBehavior().0,"outsideGeneration":state.outside_generation.get(),"outsideArmed":state.outside.borrow().is_some(),"outsideGlobalInstalled":state.outside.borrow().as_ref().is_some_and(|m|m.global.is_some()),"outsideLocalInstalled":state.outside.borrow().as_ref().is_some_and(|m|m.local.is_some())});
        response["result"].as_object_mut().expect("Native read object").extend(lifecycle.as_object().expect("Lifecycle read object").clone());
        Ok(response)
    })();
    result.unwrap_or_else(|error| serde_json::json!({"ok":false,"error":error}))
}

impl Controller {
    fn inline_alias_value(&self) -> Option<String> {
        self.ivars()
            .session
            .borrow()
            .surface()
            .editor
            .filter(|e| e.inline && e.mode == ruycast_native::actions::Mode::Alias)
            .map(|_| self.ivars().surface.alias.stringValue().to_string())
    }
    fn inline_has_marked_text(&self) -> bool {
        self.ivars()
            .surface
            .alias
            .currentEditor()
            .and_then(|e| {
                e.downcast_ref::<NSTextView>()
                    .map(|t| unsafe { msg_send![t, hasMarkedText] })
            })
            .unwrap_or(false)
    }
    fn mark_inline_pending(&self, request: Option<(u64, u64)>) {
        if let Some(request) = request {
            let surface = &self.ivars().surface;
            surface.pending.set(Some(request));
            surface.set_alias_enabled(false);
            surface.record.setEnabled(false);
        }
    }
    fn library_navigate(&self, kind: &str, value: &str) {
        self.guard_callback(|| {
            if self.ivars().surface.pending.get().is_some() || self.inline_has_marked_text() {
                return;
            }
            self.ivars().dashboard.retire_dialog();
            self.ivars().settings.retire_dialog();self.ivars().settings.stop_recording();
            let aliases = self.inline_alias_value();
            let request = self
                .ivars()
                .session
                .borrow_mut()
                .library_action(kind, value, aliases);
            self.mark_inline_pending(request);
            self.reconcile();
        });
    }
    fn library_create(&self, name: &str) {
        self.guard_callback(|| {
            if self.ivars().surface.pending.get().is_some() || self.inline_has_marked_text() {
                return;
            }
            let request = self
                .ivars()
                .session
                .borrow()
                .inline_global(name, self.inline_alias_value());
            self.mark_inline_pending(request);
        });
    }
    fn inline_focus_ended(&self) {
        let state = self.ivars();
        if state.reconciling.get()
            || !state.inline_autosave_allowed.get()
            || state.surface.pending.get().is_some()
        {
            return;
        }
        let data = state.session.borrow().surface();
        let Some(editor) = data
            .editor
            .filter(|e| e.inline && !e.saving && e.mode == ruycast_native::actions::Mode::Alias)
        else {
            return;
        };
        if !data.visible || (!state.panel.isKeyWindow() && !data.allow_hidden) {
            return;
        }
        let token = editor.token;
        let owner = Weak::new(self);
        match crate::main_loop::Timer::new_default(self.mtm(), 0., move || {
            if let Some(owner) = owner.load() {
                owner.guard_callback(|| {
                    #[cfg(feature = "test-tools")]
                    inline_tracking_trace(&owner, "autosave-timer-fired");
                    let state = owner.ivars();
                    let data = state.session.borrow().surface();
                    if state.reconciling.get()
                        || !state.inline_autosave_allowed.get()
                        || state.surface.pending.get().is_some()
                        || !data.visible
                        || (!state.panel.isKeyWindow() && !data.allow_hidden)
                        || state.surface.alias.currentEditor().is_some()
                    {
                        return;
                    }
                    let Some(_editor) = data.editor.filter(|e| {
                        e.token == token
                            && e.inline
                            && !e.saving
                            && e.mode == ruycast_native::actions::Mode::Alias
                    }) else {
                        return;
                    };
                    if state.surface.alias.stringValue().to_string()
                        == *state.surface.alias_initial.borrow()
                    {
                        owner.cancel_editor();
                    } else {
                        #[cfg(feature = "test-tools")]
                        inline_tracking_trace(&owner, "autosave-dispatch");
                        owner.save_editor();
                    }
                });
            }
        }) {
            Ok(timer) => *state.inline_autosave.borrow_mut() = Some(timer),
            Err(_) => {
                state.notice.setStringValue(ns_string!(
                    "Could not save alias. Press Enter to try again."
                ));
                state.notice.setHidden(false);
                state.panel.makeFirstResponder(Some(&state.surface.alias));
            }
        }
    }
    pub(crate) fn library_inline(
        &self,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        mode: ruycast_native::actions::Mode,
    ) {
        self.guard_callback(|| {
            #[cfg(feature = "test-tools")]
            inline_tracking_trace(self, "binding-action-entry");
            let state = self.ivars();
            if state.surface.pending.get().is_some() {
                return;
            }
            let marked = state
                .surface
                .alias
                .currentEditor()
                .and_then(|e| {
                    e.downcast_ref::<NSTextView>()
                        .map(|t| unsafe { msg_send![t, hasMarkedText] })
                })
                .unwrap_or(false);
            if marked {
                return;
            }
            let aliases = state
                .session
                .borrow()
                .surface()
                .editor
                .filter(|e| e.inline && e.mode == ruycast_native::actions::Mode::Alias)
                .map(|_| state.surface.alias.stringValue().to_string());
            if let Some(request) = state
                .session
                .borrow()
                .begin_inline(target, dismissal, mode, aliases)
            {
                state.surface.pending.set(Some(request));
                state.surface.set_alias_enabled(false);
                state.surface.record.setEnabled(false);
            }
        });
    }
    pub(crate) fn library_enabled(
        &self,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        expected: bool,
        enabled: bool,
    ) {
        self.guard_callback(|| {
            let state = self.ivars();
            if state.surface.pending.get().is_some() {
                return;
            }
            let marked = state
                .surface
                .alias
                .currentEditor()
                .and_then(|e| {
                    e.downcast_ref::<NSTextView>()
                        .map(|t| unsafe { msg_send![t, hasMarkedText] })
                })
                .unwrap_or(false);
            if marked {
                return;
            }
            let aliases = state
                .session
                .borrow()
                .surface()
                .editor
                .filter(|e| e.inline && e.mode == ruycast_native::actions::Mode::Alias)
                .map(|_| state.surface.alias.stringValue().to_string());
            if let Some(request) = state
                .session
                .borrow()
                .library_enabled(target, dismissal, expected, enabled, aliases)
            {
                state.surface.pending.set(Some(request));
                state.surface.set_alias_enabled(false);
                state.surface.record.setEnabled(false);
            }
        });
    }
    fn sync_query(&self) {
        if self.ivars().session.borrow().surface().editor.is_none() {
            let text = self.ivars().input.stringValue().to_string();
            if self.ivars().session.borrow().query() != text {
                self.ivars().session.borrow_mut().set_query(&text);
            }
        }
    }
    pub(crate) fn cancel_editor(&self) {
        self.guard_callback(|| {
            if self.ivars().surface.command.finish_link(false){return;}
            self.ivars().surface.stop();
            self.ivars().session.borrow_mut().cancel_editor();
            self.reconcile();
        });
    }
    fn change_argument(&self, index: Option<usize>) {
        self.guard_callback(|| {
            let surface = &self.ivars().surface;
            if surface.pending.get().is_some()
                || !self
                    .ivars()
                    .session
                    .borrow()
                    .surface()
                    .editor
                    .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Edit)
            {
                return;
            }
            if let Err(error) = surface
                .command
                .change_argument(self.mtm(), index, &surface.record)
            {
                self.ivars()
                    .notice
                    .setStringValue(&NSString::from_str(&error));
                self.ivars().notice.setHidden(false);
            }
        });
    }
    pub(crate) fn cancel_recording(&self) {
        if self.ivars().settings.recording_token().is_some(){self.ivars().settings.stop_recording();return;}
        if self
            .ivars()
            .session
            .borrow()
            .surface()
            .editor
            .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Edit)
        {
            self.ivars().surface.stop();
        } else {
            self.cancel_editor();
        }
    }
    fn cancel_pending_input_menu(&self) -> bool {
        let Some(attempt) = self.ivars().pending_input_menu.take() else {
            return false;
        };
        self.ivars().session.borrow().cancel_input_control(attempt);
        if self
            .ivars()
            .surface
            .pending
            .get()
            .is_some_and(|(_, a)| a == attempt)
        {
            self.ivars().surface.pending.set(None);
        }
        self.reconcile();
        true
    }
    fn toggle_input_menu(&self) {
        if self.cancel_pending_input_menu() {
            return;
        }
        if self.ivars().session.borrow().surface().actions_open {
            self.ivars()
                .session
                .borrow_mut()
                .key("cancelOperation:", false);
            self.reconcile();
        } else {
            self.inline_input_control(ruycast_native::connected::InputOperation::Menu);
        }
    }
    fn inline_input_control(&self, operation: ruycast_native::connected::InputOperation) {
        self.guard_callback(|| {
            let state = self.ivars();
            let data = state.session.borrow().surface();
            if !data.visible
                || data.execution_running
                || state.surface.pending.get().is_some()
                || !data.editor.as_ref().is_some_and(|e| e.run_inline)
            {
                return;
            }
            let text = state
                .input
                .currentEditor()
                .and_then(|e| e.downcast::<NSTextView>().ok());
            if text
                .as_ref()
                .is_some_and(|t| unsafe { msg_send![&**t, hasMarkedText] })
            {
                return;
            }
            let string = text
                .as_ref()
                .map(|t| t.string())
                .unwrap_or_else(|| state.input.stringValue());
            let value = string.to_string();
            let request = state.session.borrow().input_control(value, operation);
            if let Some(request) = request {
                state
                    .run_inline_selection
                    .set(text.as_ref().map(|t| t.selectedRange()));
                if operation == ruycast_native::connected::InputOperation::Menu {
                    state.pending_input_menu.set(Some(request.1));
                }
                state.surface.pending.set(Some(request));
                state.run_inline_disabled.set(true);
                state.input.setEnabled(false);
            }
        });
    }
    pub(crate) fn current_template_token(&self) -> Option<u64> {
        let state = self.ivars();
        if state.surface.form.isHidden() {
            return None;
        }
        state
            .session
            .borrow()
            .template_token()
            .filter(|token| *token == state.surface.shown_token.get())
    }
    pub(crate) fn current_app_editor_token(&self) -> Option<u64> {
        let s = self.ivars();
        if s.surface.form.isHidden() || s.surface.command.scroll.isHidden() {
            return None;
        }
        s.session
            .borrow()
            .edit_token()
            .filter(|t| *t == s.surface.shown_token.get())
    }
    pub(crate) fn app_choice_valid(&self, token: u64) -> bool {
        self.current_app_editor_token() == Some(token)
            && self.ivars().surface.pending.get().is_none()
            && self.ivars().surface.command.app_valid(token)
    }
    fn app_save_state(&self, token: u64) {
        if self.current_app_editor_token() == Some(token) {
            let s = self.ivars();
            s.surface.save.setEnabled(
                s.surface.pending.get().is_none() && !s.surface.command.app_list.is_open(),
            );
        }
    }
    pub(crate) fn app_choice_closed(&self, token: u64, restore: bool) {
        self.ivars()
            .surface
            .command
            .close_app(restore && self.app_choice_valid(token));
        self.app_save_state(token);
    }
    pub(crate) fn app_choice_committed(&self, token: u64, value: String) {
        if self.app_choice_valid(token) {
            self.ivars().surface.command.commit_app(value);
        }
    }
    pub(crate) fn app_choice_tab(&self, back: bool) {
        self.ivars().surface.command.tab_app(back);
    }
    pub(crate) fn app_button_key(&self, button: &crate::native_editor::AppButton, event: &NSEvent) {
        self.guard_callback(|| {
            if !self.app_choice_valid(button.ivars().token.get()) || !button.isEnabled() {
                return;
            }
            match event.keyCode() {
                48 => {
                    self.app_choice_tab(event.modifierFlags().contains(NSEventModifierFlags::Shift))
                }
                49 | 36 | 76 | 125 | 126 => {
                    self.ivars()
                        .surface
                        .command
                        .open_app(button.ivars().token.get());
                    self.app_save_state(button.ivars().token.get());
                }
                53 => self.cancel_editor(),
                _ => {}
            }
        });
    }
    pub(crate) fn save_editor(&self) {
        self.guard_callback(|| {
            if self.ivars().surface.command.finish_link(true){return;}
            if self.ivars().surface.template.choice_open()
                || self.ivars().surface.command.app_list.is_open()
            {
                return;
            }
            if self.ivars().surface.pending.get().is_some() {
                return;
            }
            let data = self.ivars().session.borrow().surface();
            if data.editor.as_ref().is_some_and(|e| e.saving) {
                return;
            }
            if data
                .editor
                .as_ref()
                .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Input)
            {
                if data.execution_running
                    || (data
                        .editor
                        .as_ref()
                        .is_none_or(|e| e.template_fields.is_none())
                        && unsafe { msg_send![&*self.ivars().surface.command.text, hasMarkedText] })
                {
                    return;
                }
                if let Some(editor) = data.editor.as_ref().filter(|e| e.template_fields.is_some()) {
                    if self.ivars().surface.template.marked() {
                        return;
                    }
                    match self.ivars().surface.template.capture(editor.token) {
                        Ok(values) => {
                            if let Some(request) =
                                self.ivars().session.borrow().submit_template(values)
                            {
                                self.ivars().surface.pending.set(Some(request));
                                self.ivars().surface.template.set_enabled(false);
                                self.ivars().surface.save.setEnabled(false);
                            }
                        }
                        Err(error) => {
                            self.ivars()
                                .notice
                                .setStringValue(&NSString::from_str(&error));
                            self.ivars().notice.setHidden(false);
                        }
                    }
                    return;
                }
                let string = self.ivars().surface.command.text.string();
                let value = string.to_string();
                if let Some(request) = self.ivars().session.borrow().submit_input(value) {
                    self.ivars().surface.pending.set(Some(request));
                    self.ivars().surface.command.set_enabled(false);
                    self.ivars().surface.save.setEnabled(false);
                }
                return;
            }
            if data
                .editor
                .as_ref()
                .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Edit)
            {
                match self
                    .ivars()
                    .surface
                    .command
                    .value(self.ivars().surface.record.value())
                {
                    Ok(value) => {
                        if let Some(request) = self.ivars().session.borrow().save_command(value) {
                            self.ivars().surface.pending.set(Some(request));
                            self.ivars()
                                .surface
                                .command
                                .remember_focus(&self.ivars().panel);
                            self.ivars().surface.command.set_enabled(false);
                            self.ivars().surface.record.setEnabled(false);
                            self.ivars().surface.save.setEnabled(false);
                        }
                    }
                    Err(error) => {
                        self.ivars()
                            .notice
                            .setStringValue(&NSString::from_str(&error));
                        self.ivars().notice.setHidden(false);
                    }
                }
                return;
            }
            let aliases = data
                .editor
                .filter(|e| e.mode == ruycast_native::actions::Mode::Alias)
                .map(|_| self.ivars().surface.alias.stringValue().to_string());
            if let Some(request) = self.ivars().session.borrow().save_editor(aliases, None) {
                self.ivars().surface.pending.set(Some(request));
                self.ivars().surface.set_alias_enabled(false);
                self.ivars().surface.save.setEnabled(false);
            }
        });
    }
    pub(crate) fn template_choice_changed(&self, token: u64, open: bool) {
        if self.current_template_token() != Some(token) {
            return;
        }
        self.ivars().surface.save.setEnabled(
            !open
                && self.ivars().surface.pending.get().is_none()
                && !self.ivars().session.borrow().surface().execution_running,
        );
    }
    pub(crate) fn preparing_record_key(&self, event: &NSEvent) -> bool {
        self.guard_callback(|| {
            let record=self.ivars().settings.recorder().unwrap_or_else(||self.ivars().surface.record.clone());
            if !record.preparing() {
                return false;
            }
            if event.r#type() != NSEventType::KeyDown {
                return true;
            }
            let flags = event
                .modifierFlags()
                .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
            // Preparing is not capture: CmdW remains a window close, whereas
            // active recording intentionally captures it as a possible shortcut.
            if self.ivars().session.borrow().management()
                && flags == NSEventModifierFlags::Command
                && event
                    .charactersIgnoringModifiers()
                    .is_some_and(|s| s.to_string().eq_ignore_ascii_case("w"))
            {
                if !event.isARepeat() {
                    self.ivars().panel.close();
                }
            } else if event.keyCode() == 53 && (flags & !NSEventModifierFlags::Function).is_empty()
            {
                record.stop();
                self.cancel_recording();
            }
            true
        })
        .unwrap_or(true)
    }
    pub(crate) fn recording_lease(
        &self,
    ) -> Result<Option<ruycast_native::keyboard_service::RecorderLease>, String> {
        #[cfg(feature = "test-tools")]
        self.ivars().recorder_acquire_starts.set(
            self.ivars().recorder_acquire_starts.get().saturating_add(1),
        );
        self.ivars()
            .keyboard
            .borrow()
            .as_ref()
            .map(|c| c.recorder())
            .transpose()
    }
    pub(crate) fn recording_owner(&self) -> Option<u64> {
        if let Some(token)=self.ivars().settings.recording_token(){return self.ivars().settings.live(self).then_some(token);}
        if self.ivars().surface.pending.get().is_some() {
            return None;
        }
        self.ivars().session.borrow().recording_owner()
    }
    pub(crate) fn recording_error(&self, token: u64, message: &str) {
        if self.recording_owner() == Some(token) {
            self.ivars()
                .notice
                .setStringValue(&NSString::from_str(message));
            self.ivars().notice.setHidden(false);
        }
    }
    pub(crate) fn recorder_menu_context(
        &self,
        record: &crate::native_surface::RecordButton,
    ) -> Option<RecordMenuContext> {
        let state = self.ivars();
        if state.failed.get() || !record.isEnabled() || record.active()
            || record.isHiddenOrHasHiddenAncestor() || state.surface.pending.get().is_some()
            || !record.window().is_some_and(|window| {
                let parent: &NSWindow = &state.panel;
                std::ptr::eq(&*window, parent)
            })
        {
            return None;
        }
        let model = state.session.borrow();
        let Model::Live { view, connection } = &*model else { return None; };
        if !(state.panel.isVisible() || cfg!(feature = "test-tools") && connection.hidden_tests) {
            return None;
        }
        let latest = connection.view();
        if !view.visible || !latest.visible || connection.stop_requested() || connection.stopped()
            || connection.latest_epoch() != view.epoch
            || connection.latest_dismissal() != view.dismissal
            || latest.config_revision != view.config_revision
        {
            return None;
        }
        if let Some(index) = state.settings.records.iter().position(|button| std::ptr::eq(&**button, record)) {
            let surface = state.settings.surface()?;
            let current = latest.settings.as_ref()?;
            if !state.settings.visible() || state.settings.dialog_open()
                || state.settings.records.iter().any(|button| button.active())
                || surface.tab != "general" || surface.busy || current.busy
                || current.tab != surface.tab || current.revision != surface.revision
                || current.epoch != surface.epoch || current.dismissal != surface.dismissal
                || current.navigation_revision != surface.navigation_revision
                || !self.settings_live(&surface)
            {
                return None;
            }
            return Some(RecordMenuContext::Settings {
                index, revision: surface.revision, epoch: surface.epoch,
                dismissal: surface.dismissal, navigation_revision: surface.navigation_revision,
            });
        }
        if !std::ptr::eq(&*state.surface.record, record) || self.inline_has_marked_text() {
            return None;
        }
        let editor = view.editor.as_ref()?;
        let current = latest.editor.as_ref()?;
        if editor.saving || current.saving || editor.input_disabled || current.input_disabled
            || !matches!(editor.mode, ruycast_native::actions::Mode::Edit | ruycast_native::actions::Mode::Hotkey)
            || current.token != editor.token || current.id != editor.id || current.mode != editor.mode
            || latest.catalog_revision != view.catalog_revision
        {
            return None;
        }
        Some(RecordMenuContext::Inline {
            token: editor.token, id: editor.id.clone(), mode: editor.mode,
            catalog_revision: view.catalog_revision, config_revision: view.config_revision,
            epoch: view.epoch, dismissal: view.dismissal,
        })
    }
    pub(crate) fn apply_recorder_menu(
        &self,
        record: &crate::native_surface::RecordButton,
        context: &RecordMenuContext,
        expected: &str,
    ) {
        self.guard_callback(|| {
            if record.value().as_deref() != Some(expected)
                || self.recorder_menu_context(record).as_ref() != Some(context)
            {
                return;
            }
            let Some(value) = crate::native_surface::either_command(expected) else { return; };
            match context {
                RecordMenuContext::Settings { index, revision, epoch, dismissal, .. } => {
                    let Some(key) = ["launcherHotkey", "clipboardHotkey"].get(*index) else { return; };
                    self.settings_control_bound(*revision, *epoch, *dismissal, key, serde_json::json!(value));
                }
                RecordMenuContext::Inline { .. } => self.save_hotkey(Some(value)),
            }
        });
    }
    pub(crate) fn save_hotkey(&self, value: Option<String>) {
        self.guard_callback(|| {
            if self.ivars().settings.recording_token().is_some(){
                if let Some((revision,key,value))=self.ivars().settings.recorded(value){self.settings_control(revision,&key,value);}
                return;
            }
            if self
                .ivars()
                .session
                .borrow()
                .surface()
                .editor
                .is_some_and(|e| e.mode == ruycast_native::actions::Mode::Edit)
            {
                self.ivars().surface.record.configure(value);
                return;
            }
            if self.ivars().surface.pending.get().is_none() {
                if let Some(request) = self.ivars().session.borrow().save_editor(None, Some(value))
                {
                    self.ivars().surface.pending.set(Some(request));
                    self.ivars().surface.record.setEnabled(false);
                }
            }
        });
    }
    pub(crate) fn result_query_changed(&self, token: u64, text: &str, marked: bool) {
        self.guard_callback(|| {
            if let Model::Live { view, connection } = &*self.ivars().session.borrow() {
                if view.visible
                    && view
                        .script_result
                        .as_ref()
                        .is_some_and(|r| r.token == token && !r.detail_visible && !r.busy)
                {
                    connection.query_result(token, text, marked);
                }
            }
        });
    }
    pub(crate) fn result_query_key(&self, token: u64, key: &str, marked: bool) -> bool {
        self.guard_callback(|| {
            let valid = self
                .ivars()
                .session
                .borrow()
                .script_result()
                .is_some_and(|r| r.token == token && !r.detail_visible);
            if !valid {
                return false;
            }
            if key == "cancelOperation:" && !marked {
                self.result_back();
                return true;
            }
            let handled = self.ivars().session.borrow_mut().key(key, marked);
            if handled {
                self.reconcile();
            }
            handled
        })
        .unwrap_or(false)
    }
    pub(crate) fn result_back(&self) {
        self.guard_callback(|| {
            let detail = self
                .ivars()
                .session
                .borrow()
                .script_result()
                .is_some_and(|r| r.detail_visible);
            if detail {
                self.ivars().restore_result_focus.set(true);
            }
            self.ivars().session.borrow_mut().result_back();
            self.reconcile();
        });
    }
    pub(crate) fn result_text_back(&self, token: u64) {
        let valid = self
            .ivars()
            .session
            .borrow()
            .script_result()
            .is_some_and(|r| r.token == token && r.detail_visible);
        if valid {
            self.result_back();
        }
    }
    pub(crate) fn result_control(&self, token: u64, name: &str) {
        self.guard_callback(|| {
            let state = self.ivars();
            if state.result_panel.has_marked()
                || state
                    .input
                    .currentEditor()
                    .and_then(|e| e.downcast::<NSTextView>().ok())
                    .is_some_and(|t| unsafe { msg_send![&*t, hasMarkedText] })
            {
                return;
            }
            let mut model = state.session.borrow_mut();
            if model.visible() && model.script_result().is_some_and(|r| r.token == token) {
                if model.script_result().is_some_and(|r| r.detail_visible)
                    && ["close", "choices"].contains(&name)
                {
                    state.restore_result_focus.set(true);
                }
                model.result_action(token, name, None);
                if name == "close" {
                    model.mark_result_closing(token);
                }
            }
            drop(model);
            self.reconcile();
        });
    }
    fn route_key(&self, event: &NSEvent) -> bool {
        self.guard_callback(|| {
            if self.ivars().settings.visible(){
                if self.ivars().settings.dialog_open(){return false;}
                if let Some(record)=self.ivars().settings.recorder(){
                    if record.preparing(){return self.preparing_record_key(event);}
                    if record.recording(){return record.performKeyEquivalent(event);}
                }
                if event.r#type()==NSEventType::KeyDown {
                    let flags=event.modifierFlags().intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
                    if flags==NSEventModifierFlags::Command && event.charactersIgnoringModifiers().is_some_and(|s|s.to_string().eq_ignore_ascii_case("w")){
                        if !event.isARepeat(){self.ivars().panel.close();}return true;
                    }
                    if event.keyCode()==53 {self.ivars().session.borrow_mut().hide_palette();self.reconcile();self.ivars().panel.orderOut(None);return true;}
                    if [36,76].contains(&event.keyCode()) || flags.contains(NSEventModifierFlags::Command)&&event.charactersIgnoringModifiers().is_some_and(|s|s.to_string().eq_ignore_ascii_case("k")){return true;}
                }
                return false;
            }
            if self.ivars().session.borrow().dashboard().is_some() {
                if self.ivars().dashboard.dialog_open() { return false; }
                if event.r#type()==NSEventType::KeyDown && event.keyCode()==48 {
                    self.ivars().dashboard.advance_focus(event.modifierFlags().contains(NSEventModifierFlags::Shift));return true;
                }
                if event.r#type()==NSEventType::KeyDown && event.keyCode()==53 && self.ivars().dashboard.confirming() {
                    if let Some(token)=self.ivars().dashboard.token() {self.dashboard_control(token,ruycast_native::analytics_dashboard::Operation::CancelReset{confirmation:self.ivars().dashboard.confirmation_token().unwrap_or(0)});}
                    return true;
                }
                return false;
            }
            if self.ivars().surface.command.app_list.key(event)
                || self.ivars().surface.template.choice_key(event)
            {
                return true;
            }
            let result = self.ivars().session.borrow().script_result().cloned();
            if let Some(r) = result {
                if event.r#type() == NSEventType::KeyDown
                    && event
                        .modifierFlags()
                        .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask)
                        == NSEventModifierFlags::Command
                    && event
                        .charactersIgnoringModifiers()
                        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("r"))
                {
                    if !event.isARepeat() && !r.busy && !r.detail_visible {
                        self.result_control(r.token, "reload");
                    }
                    return true;
                }
                if event.r#type() == NSEventType::KeyDown
                    && event
                        .modifierFlags()
                        .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask)
                        == NSEventModifierFlags::Command
                    && event
                        .charactersIgnoringModifiers()
                        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("d"))
                {
                    if !event.isARepeat() && !r.busy && !r.detail_visible && r.detail.is_some() {
                        self.result_control(r.token, "details");
                    }
                    return true;
                }
                let flags = event
                    .modifierFlags()
                    .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
                if event.keyCode() == 53 && (flags & !NSEventModifierFlags::Function).is_empty() {
                    let marked = self
                        .ivars()
                        .panel
                        .firstResponder()
                        .and_then(|r| {
                            r.downcast_ref::<NSTextView>()
                                .map(|t| unsafe { msg_send![t, hasMarkedText] })
                        })
                        .unwrap_or(false);
                    if marked {
                        return false;
                    }
                    if !event.isARepeat() {
                        self.result_back();
                    }
                    return true;
                }
                if event
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Command)
                    && ([36, 76].contains(&event.keyCode())
                        || event
                            .charactersIgnoringModifiers()
                            .is_some_and(|s| s.to_string().eq_ignore_ascii_case("k")))
                {
                    return true;
                }
                return false;
            }
            if self.ivars().surface.record.preparing() {
                return self.preparing_record_key(event);
            }
            if self.ivars().surface.record.recording() {
                return self.ivars().surface.record.performKeyEquivalent(event);
            }
            if self.ivars().session.borrow().management()
                && event.r#type() == NSEventType::KeyDown
                && event
                    .modifierFlags()
                    .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask)
                    == NSEventModifierFlags::Command
                && event
                    .charactersIgnoringModifiers()
                    .is_some_and(|s| s.to_string().eq_ignore_ascii_case("w"))
            {
                if !event.isARepeat() {
                    self.ivars().panel.close();
                }
                return true;
            }
            if event.r#type() != NSEventType::KeyDown {
                return false;
            }
            let state = self.ivars();
            if event.r#type() == NSEventType::KeyDown
                && event.keyCode() != 53
                && state.surface.alias.currentEditor().is_some()
                && (state.surface.pending.get().is_some() || state.session.borrow().editor_saving())
            {
                return true;
            }

            let flags = event.modifierFlags()
                & (NSEventModifierFlags::Command
                    | NSEventModifierFlags::Option
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Shift
                    | NSEventModifierFlags::Function);
            if self.ivars().session.borrow().run_inline() {
                let marked = self
                    .ivars()
                    .input
                    .currentEditor()
                    .and_then(|e| {
                        e.downcast_ref::<NSTextView>()
                            .map(|t| unsafe { msg_send![t, hasMarkedText] })
                    })
                    .unwrap_or(false);
                if marked {
                    return false;
                }
                if self.ivars().session.borrow().input_menu_open()
                    && (flags & !NSEventModifierFlags::Function).is_empty()
                {
                    let key = match event.keyCode() {
                        125 => Some("moveDown:"),
                        126 => Some("moveUp:"),
                        36 | 76 => Some("insertNewline:"),
                        48 => Some("insertTab:"),
                        _ => None,
                    };
                    if let Some(key) = key {
                        if !event.isARepeat() {
                            self.ivars().session.borrow_mut().key(key, false);
                        }
                        return true;
                    }
                }
                if flags == NSEventModifierFlags::Command
                    && event
                        .charactersIgnoringModifiers()
                        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("k"))
                {
                    if !event.isARepeat() {
                        self.toggle_input_menu();
                    }
                    return true;
                }
                if [36, 76].contains(&event.keyCode()) && flags == NSEventModifierFlags::Command {
                    if !event.isARepeat() {
                        self.inline_input_control(
                            ruycast_native::connected::InputOperation::Expand,
                        );
                    }
                    return true;
                }
                if event.keyCode() == 53 {
                    if !event.isARepeat() {
                        if self.cancel_pending_input_menu() {
                            return true;
                        }
                        if self.ivars().session.borrow().input_menu_open() {
                            self.ivars()
                                .session
                                .borrow_mut()
                                .key("cancelOperation:", false);
                            self.reconcile();
                        } else {
                            self.cancel_editor();
                        }
                    }
                    return true;
                }
                return false;
            }
            let data = self.ivars().session.borrow().surface();
            if let Some(editor) = &data.editor {
                if [
                    ruycast_native::actions::Mode::Alias,
                    ruycast_native::actions::Mode::Edit,
                    ruycast_native::actions::Mode::Input,
                ]
                .contains(&editor.mode)
                {
                    let marked = self
                        .ivars()
                        .panel
                        .firstResponder()
                        .and_then(|e| {
                            e.downcast_ref::<NSTextView>()
                                .map(|text| unsafe { msg_send![text, hasMarkedText] })
                        })
                        .unwrap_or(false);
                    if marked && [36, 76, 53].contains(&event.keyCode()) {
                        return false;
                    }
                }
                if event.keyCode() == 53 {
                    if !event.isARepeat() {
                        self.cancel_editor();
                    }
                    return true;
                }
                if (event.keyCode() == 36 || event.keyCode() == 76)
                    && (flags == NSEventModifierFlags::Command
                        || (editor.mode == ruycast_native::actions::Mode::Remove
                            && flags.is_empty()))
                {
                    if !event.isARepeat() {
                        self.save_editor();
                    }
                    return true;
                }
                return false;
            }
            if self.ivars().session.borrow().clipboard().is_some()&&flags==NSEventModifierFlags::Command&&[36,76].contains(&event.keyCode()) {
                if !event.isARepeat(){let model=self.ivars().session.borrow();if let Some(s)=model.clipboard(){model.clipboard_action("copy",s.selected_id.clone(),s.revision,model.epoch(),model.dismissal());}}
                return true;
            }
            if data.execution_running
                && !data.actions_open
                && event.keyCode() == 53
                && (flags & !NSEventModifierFlags::Function).is_empty()
            {
                let marked = self
                    .ivars()
                    .panel
                    .firstResponder()
                    .and_then(|r| {
                        r.downcast_ref::<NSTextView>()
                            .map(|text| unsafe { msg_send![text, hasMarkedText] })
                    })
                    .unwrap_or(false);
                if marked {
                    return false;
                }
                if !event.isARepeat() {
                    self.ivars()
                        .session
                        .borrow_mut()
                        .key("cancelOperation:", false);
                    self.reconcile();
                }
                return true;
            }
            if flags == NSEventModifierFlags::Command
                && event
                    .charactersIgnoringModifiers()
                    .is_some_and(|v| v.to_string().eq_ignore_ascii_case("k"))
            {
                if !event.isARepeat() {
                    self.sync_query();
                    if data.actions_open {
                        self.ivars()
                            .session
                            .borrow_mut()
                            .key("cancelOperation:", false);
                    } else {
                        self.ivars().session.borrow_mut().toggle_actions();
                    }
                    self.reconcile();
                }
                return true;
            }
            let binding = if event.keyCode() == 43
                && flags == (NSEventModifierFlags::Command | NSEventModifierFlags::Option)
            {
                Some("set-alias")
            } else if event.keyCode() == 43
                && flags == (NSEventModifierFlags::Command | NSEventModifierFlags::Shift)
            {
                Some("set-hotkey")
            } else {
                None
            };
            if let Some(binding) = binding {
                if !event.isARepeat() {
                    self.sync_query();
                    self.ivars().session.borrow_mut().named(
                        binding,
                        false,
                        usize::try_from(self.ivars().table.selectedRow()).ok(),
                    );
                }
                return true;
            }
            if data.actions_open
                && (flags
                    & (NSEventModifierFlags::Command
                        | NSEventModifierFlags::Control
                        | NSEventModifierFlags::Option))
                    .is_empty()
            {
                let key = match event.keyCode() {
                    125 => "moveDown:",
                    126 => "moveUp:",
                    36 | 76 => "insertNewline:",
                    53 => "cancelOperation:",
                    48 => {
                        if flags.contains(NSEventModifierFlags::Shift) {
                            "insertBacktab:"
                        } else {
                            "insertTab:"
                        }
                    }
                    _ => return false,
                };
                self.ivars().session.borrow_mut().key(key, false);
                self.reconcile();
                return true;
            }
            false
        })
        .unwrap_or(false)
    }
}

impl Controller {
    pub(crate) fn management_graphite(&self)->bool{self.ivars().session.borrow().theme()==Some("graphite")}
    pub(crate) fn clipboard_settings_control(&self,revision:u64,epoch:u64,dismissal:u64,navigation:u64,key:&str,value:serde_json::Value){
        let valid=match &*self.ivars().session.borrow(){Model::Live{view,..}=>view.settings.as_ref().is_some_and(|s|s.tab=="clipboard"&&s.navigation_revision==navigation),_=>false};
        if valid{self.settings_control_bound(revision,epoch,dismissal,key,value);}
    }
    pub(crate) fn settings_live(&self,s:&ruycast_native::settings_surface::Surface)->bool {
        match &*self.ivars().session.borrow(){Model::Live{view,connection}=>view.visible && view.settings.as_ref().is_some_and(|v|v.revision==s.revision&&v.tab==s.tab)
            && !connection.stop_requested()&&!connection.stopped()&&connection.latest_dismissal()==s.dismissal&&connection.latest_epoch()==s.epoch,_=>false}
    }
    pub(crate) fn settings_control_bound(&self,revision:u64,epoch:u64,dismissal:u64,key:&str,value:serde_json::Value){
        let valid=match &*self.ivars().session.borrow(){Model::Live{view,connection}=>view.visible&&view.epoch==epoch&&view.dismissal==dismissal&&connection.latest_epoch()==epoch&&connection.latest_dismissal()==dismissal,_=>false};
        if valid{self.settings_control(revision,key,value);}
    }
    pub(crate) fn settings_hidden(&self)->bool {match &*self.ivars().session.borrow(){Model::Live{connection,..}=>connection.hidden_tests,_=>true}}
    pub(crate) fn settings_control(&self,revision:u64,key:&str,value:serde_json::Value){
        self.guard_callback(||{
            let surface=match &*self.ivars().session.borrow(){Model::Live{view,..}=>view.settings.clone(),_=>None};
            let Some(s)=surface.filter(|s|s.revision==revision&&!s.busy&&self.settings_live(s))else{return;};
            if self.ivars().settings.dialog_open(){return;}
            let model=self.ivars().session.borrow();let Model::Live{connection,..}=&*model else{return;};let connection=connection.clone();drop(model);
            let operation=match key{
                "clipboard-paused"=>{let Some(paused)=value.as_bool().filter(|_|s.tab=="clipboard")else{return;};ruycast_native::settings_surface::Operation::ClipboardPause(paused)},
                "choose-folder"=>{self.ivars().settings.choose(self,connection.hidden_tests,&self.ivars().panel);return;},
                "reload"=>ruycast_native::settings_surface::Operation::Reload,
                "open-config"|"accessibility"|"input-monitoring"=>ruycast_native::settings_surface::Operation::Feedback(self.ivars().settings.open(self,key,&s,connection.hidden_tests)),
                _=>ruycast_native::settings_surface::Operation::Set{key:key.into(),value},
            };
            connection.action(ruycast_native::connected::Action::Setting{revision:s.revision,navigation_revision:s.navigation_revision,operation});
        });
    }
    pub(crate) fn settings_folder_completed(&self,serial:u64,accepted:bool,panel:Option<&NSOpenPanel>){
        self.guard_callback(||{
            if !self.ivars().settings.finish_dialog(serial)||!accepted{return;}
            let path=panel.and_then(|p|p.URL()).filter(|u|u.isFileURL()).and_then(|url|{
                let mut bytes=vec![0i8;libc::PATH_MAX as usize];
                if !unsafe{url.getFileSystemRepresentation_maxLength(std::ptr::NonNull::new(bytes.as_mut_ptr()).expect("path buffer"),bytes.len())}{return None;}
                unsafe{std::ffi::CStr::from_ptr(bytes.as_ptr())}.to_str().ok().map(str::to_owned)
            });
            if let Some(path)=path{self.settings_directory(path);}
        });
    }
    fn settings_directory(&self,path:String){
        if let Some(s)=self.ivars().settings.surface().filter(|s|self.settings_live(s)){
            if let Model::Live{connection,..}=&*self.ivars().session.borrow(){connection.action(ruycast_native::connected::Action::Setting{revision:s.revision,navigation_revision:s.navigation_revision,operation:ruycast_native::settings_surface::Operation::Directory(path)});}
        }
    }
}

impl Controller {
    pub(crate) fn menu_bar_state(&self)->Option<ruycast_native::menu_bar::State>{match &*self.ivars().session.borrow(){Model::Live{view,..}=>view.menu_bar.clone(),_=>None}}
    pub(crate) fn menu_bar_action(&self,serial:u64,index:isize){self.guard_callback(||{
        if !self.ivars().menu_bar.live(serial){return;}
        let connection=match &*self.ivars().session.borrow(){Model::Live{connection,..} if !connection.stop_requested()&&!connection.stopped()=>connection.clone(),_=>return};
        match index{0=>connection.action(ruycast_native::connected::Action::Show),1=>{if let Some(peer)=connection.peer(){peer.action(ruycast_native::connected::Action::Show);}},2=>connection.request_stop(),_=>{}}
    });}
}
