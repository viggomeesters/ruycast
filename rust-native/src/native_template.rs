//! Main-owned virtual native fields. Only a complete Run snapshot crosses to
//! the actor; ordinary typing never writes configuration or echoes all fields.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::*;
use objc2_foundation::{
    ns_string, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use ruycast_native::template_form::Fields;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

define_class!(
    #[unsafe(super=NSTextField)] #[thread_kind=MainThreadOnly] #[ivars=Cell<u64>]
    pub(crate) struct Text;
);
define_class!(
    #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=Cell<u64>]
    pub(crate) struct Choice;
    impl Choice {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self)->bool {self.isEnabled()}
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent) {
            let _lease=self.retain();
            if let Some(owner)=self.target().and_then(|o|o.downcast::<Form>().ok()) {
                owner.guard(||owner.choice_button_key(self,event));
            }
        }
    }
);
struct RowState {
    label: Retained<NSTextField>,
    text: Retained<Text>,
    choice: Retained<Choice>,
}
define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=RowState]
    struct Row;
);
#[derive(Default)]
struct Draft {
    token: u64,
    fields: Option<Fields>,
    values: Vec<String>,
    bytes: usize,
    invalid: HashSet<usize>,
}
pub(crate) struct State {
    pub scroll: Retained<NSScrollView>,
    table: RefCell<Retained<NSTableView>>,
    owner: RefCell<Option<Weak<Controller>>>,
    draft: RefCell<Draft>,
    loading: Cell<bool>,
    enabled: Cell<bool>,
    rows: RefCell<Vec<Weak<Row>>>,
    focus: Cell<usize>,
    choice_widget: Retained<crate::native_choice::Widget>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<(Retained<Text>, Retained<NSTextView>)>>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    #[ivars=State]
    pub(crate) struct Form;
    unsafe impl NSObjectProtocol for Form {}
    unsafe impl NSTableViewDataSource for Form {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> isize {
            self.guard(|| self.ivars().draft.borrow().values.len() as isize)
                .unwrap_or(0)
        }
    }
    unsafe impl NSTableViewDelegate for Form {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn row(
            &self,
            table: &NSTableView,
            _: Option<&NSTableColumn>,
            index: isize,
        ) -> Option<Retained<NSView>> {
            self.guard(|| self.make_row(table, usize::try_from(index).ok()?))
                .flatten()
        }
        #[unsafe(method(tableView:shouldSelectRow:))]
        fn select(&self, _: &NSTableView, _: isize) -> bool {
            false
        }
    }
    unsafe impl NSTextFieldDelegate for Form {}
    unsafe impl NSControlTextEditingDelegate for Form {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, note: &NSNotification) {
            self.guard(|| {
                if self.ivars().loading.get() {
                    return;
                }
                if let Some(object) = note.object() {
                    if let Some(text) = object.downcast_ref::<Text>() {
                        self.record(text);
                    }
                }
            });
        }
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn began(&self, note: &NSNotification) {
            self.guard(|| {
                if let Some(object) = note.object().and_then(|o| o.downcast::<Text>().ok()) {
                    if !self.control_live(&object) {
                        return;
                    }
                    self.ivars().focus.set(object.tag() as usize);
                    if let Some(editor) = object
                        .currentEditor()
                        .and_then(|e| e.downcast::<NSTextView>().ok())
                    {
                        editor.setAllowsUndo(true);
                    }
                }
            });
        }
        #[unsafe(method(controlTextDidEndEditing:))]
        fn ended(&self, note: &NSNotification) {
            self.guard(|| {
                if let Some(text) = note.object().and_then(|v| v.downcast::<Text>().ok()) {
                    self.record(&text);
                }
            });
        }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self, control: &NSControl, text: &NSTextView, selector: Sel) -> bool {
            self.guard(|| {
                if !self.control_live(control)
                    || control.currentEditor().is_none_or(|editor| {
                        !std::ptr::eq(
                            &*editor as *const _ as *const AnyObject,
                            text as *const _ as *const AnyObject,
                        )
                    })
                {
                    return true;
                }
                let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                if marked {
                    return false;
                }
                let owner = self.owner();
                if selector == sel!(insertNewline:) {
                    if let Some(owner) = owner {
                        owner.save_editor();
                    }
                    return true;
                }
                if selector == sel!(cancelOperation:) {
                    if let Some(owner) = owner {
                        owner.cancel_editor();
                    }
                    return true;
                }
                if selector == sel!(insertTab:) || selector == sel!(insertBacktab:) {
                    let next = control.tag() + if selector == sel!(insertTab:) { 1 } else { -1 };
                    if next >= 0 && (next as usize) < self.ivars().draft.borrow().values.len() {
                        self.focus(next as usize);
                        return true;
                    }
                }
                false
            })
            .unwrap_or(true)
        }
    }
    impl Form {
        #[unsafe(method(showChoice:))]
        fn choice_action(&self,sender:&Choice) {self.guard(||self.open_choice(sender));}
    }
);
impl Form {
    pub(crate) fn choice_valid(&self, token: u64, field: usize) -> bool {
        self.live_token() == Some(token)
            && self
                .ivars()
                .draft
                .borrow()
                .fields
                .as_ref()
                .is_some_and(|f| f.0.get(field).is_some_and(|f| !f.options.is_empty()))
    }
    fn open_choice(&self, button: &Choice) {
        if !self.control_live(button) || self.marked() {
            return;
        }
        self.flush_active();
        let (token, field, fields, selected) = {
            let d = self.ivars().draft.borrow();
            let field = button.tag() as usize;
            let Some(fields) = d.fields.clone() else {
                return;
            };
            let Some(value) = d.values.get(field) else {
                return;
            };
            let key = NSString::from_str(value)
                .precomposedStringWithCanonicalMapping()
                .to_string();
            let selected = fields.option_index(field, &key);
            (d.token, field, fields, selected)
        };
        self.ivars().focus.set(field);
        if self
            .ivars()
            .choice_widget
            .open(token, field, fields, selected)
        {
            button.setAccessibilityExpanded(true);
            self.ivars().scroll.setAccessibilityHidden(true);
            self.fields_enabled(false);
            if let Some(owner) = self.owner() {
                owner.template_choice_changed(token, true);
            }
        }
    }
    pub(crate) fn choice_closed(&self, token: u64, field: usize, restore: bool) {
        self.ivars().scroll.setAccessibilityHidden(false);
        self.fields_enabled(self.ivars().enabled.get());
        if !self.choice_valid(token, field) {
            return;
        }
        if let Some(owner) = self.owner() {
            owner.template_choice_changed(token, false);
        }
        if let Some(row) = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, field as isize, false)
            .and_then(|v| v.downcast::<Row>().ok())
        {
            row.ivars().choice.setAccessibilityExpanded(false);
        }
        if restore {
            self.focus(field);
        }
    }
    pub(crate) fn choice_committed(&self, token: u64, field: usize, value: String) {
        if !self.choice_valid(token, field) {
            return;
        }
        self.store(field, value.clone());
        if let Some(row) = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, field as isize, false)
            .and_then(|v| v.downcast::<Row>().ok())
        {
            row.ivars().choice.setTitle(&NSString::from_str(&value));
            unsafe {
                row.ivars()
                    .choice
                    .setAccessibilityValue(Some(&NSString::from_str(&value)));
            }
        }
        self.focus(field);
    }
    pub(crate) fn choice_tab(&self, field: usize, back: bool) {
        let next = field as isize + if back { -1 } else { 1 };
        if next >= 0 && (next as usize) < self.ivars().draft.borrow().values.len() {
            self.focus(next as usize);
        } else if let Some(window) = self.ivars().scroll.window() {
            if back {
                window.selectPreviousKeyView(None);
            } else {
                window.selectNextKeyView(None);
            }
        }
    }
    fn choice_button_key(&self, button: &Choice, event: &NSEvent) {
        if !self.control_live(button) {
            return;
        }
        let flags = event
            .modifierFlags()
            .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
        match event.keyCode() {
            48 => self.choice_tab(
                button.tag() as usize,
                flags.contains(NSEventModifierFlags::Shift),
            ),
            49 | 36 | 76 | 125 | 126 => self.open_choice(button),
            53 => {
                if let Some(owner) = self.owner() {
                    owner.cancel_editor();
                }
            }
            _ => {}
        }
    }
    pub fn choice_key(&self, event: &NSEvent) -> bool {
        self.ivars().choice_widget.key(event)
    }
    pub fn choice_open(&self) -> bool {
        self.ivars().choice_widget.is_open()
    }
    #[cfg(feature = "test-tools")]
    pub fn choice_probe(&self, action: &str, index: usize) -> Result<(), String> {
        match action {
            "open" => {
                if !self.marked() {
                    self.focus(index);
                }
                let row = self
                    .table()
                    .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
                    .and_then(|v| v.downcast::<Row>().ok())
                    .ok_or("Field unavailable")?;
                unsafe {
                    row.ivars().choice.performClick(None);
                }
            }
            "select" => {
                if !self.ivars().choice_widget.select(index) {
                    return Err("Choice unavailable".into());
                }
            }
            "click" => self.ivars().choice_widget.click(index)?,
            "retain" => self.ivars().choice_widget.retain(),
            "stale" => self.ivars().choice_widget.stale(index),
            _ => return Err("Unknown choice action".into()),
        }
        Ok(())
    }
    fn table(&self) -> Retained<NSTableView> {
        self.ivars().table.borrow().clone()
    }
    fn fresh_table(&self) {
        let old = self.table();
        unsafe {
            old.setDelegate(None);
            old.setDataSource(None);
        }
        let table = NSTableView::new(self.mtm());
        table.setFrame(old.frame());
        table.setHeaderView(None);
        table.setRowHeight(64.);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAllowsEmptySelection(true);
        table.setBackgroundColor(&NSColor::clearColor());
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(self.mtm()),
            ns_string!("field"),
        );
        column.setWidth(old.tableColumns().objectAtIndex(0).width());
        table.addTableColumn(&column);
        unsafe {
            table.setDelegate(Some(ProtocolObject::from_ref(self)));
            table.setDataSource(Some(ProtocolObject::from_ref(self)));
        }
        self.ivars().scroll.setDocumentView(Some(&table));
        *self.ivars().table.borrow_mut() = table;
    }
    fn owner(&self) -> Option<Retained<Controller>> {
        self.ivars().owner.borrow().as_ref().and_then(Weak::load)
    }
    pub(crate) fn guard<T>(&self, body: impl FnOnce() -> T) -> Option<T> {
        self.owner().and_then(|owner| owner.guard_callback(body))
    }
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Retained<Self> {
        let scroll = NSScrollView::new(main);
        scroll.setFrame(rect(18., 76., width - 84., height));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setHidden(true);
        let table = NSTableView::new(main);
        table.setFrame(rect(0., 0., width - 104., height));
        table.setHeaderView(None);
        table.setRowHeight(64.);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAllowsEmptySelection(true);
        table.setBackgroundColor(&NSColor::clearColor());
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(main), ns_string!("field"));
        column.setWidth(width - 104.);
        table.addTableColumn(&column);
        scroll.setDocumentView(Some(&table));
        parent.addSubview(&scroll);
        let value = Self::alloc(main).set_ivars(State {
            scroll,
            table: RefCell::new(table),
            owner: RefCell::new(None),
            draft: RefCell::new(Draft::default()),
            loading: Cell::new(false),
            enabled: Cell::new(true),
            rows: RefCell::new(vec![]),
            focus: Cell::new(0),
            choice_widget: crate::native_choice::Widget::new(
                main,
                parent,
                rect(18., 76., width - 84., height),
            ),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
        });
        let value: Retained<Self> = unsafe { msg_send![super(value), init] };
        // SAFETY: Form owns the table and disconnects its nonretained delegates in cleanup.
        unsafe {
            value
                .table()
                .setDelegate(Some(ProtocolObject::from_ref(&*value)));
            value
                .table()
                .setDataSource(Some(ProtocolObject::from_ref(&*value)));
        }
        value.ivars().choice_widget.bind(&value);
        value
    }
    pub fn layout(&self, frame: NSRect) {
        self.ivars().scroll.setFrame(frame);
        self.ivars().choice_widget.set_frame(frame);
        let width = self.ivars().scroll.contentSize().width;
        let table = self.table();
        table.tableColumns().objectAtIndex(0).setWidth(width);
        for row in self.ivars().rows.borrow().iter().filter_map(Weak::load) {
            let r = row.ivars();
            for view in [
                &*r.label as &NSView,
                &*r.text as &NSView,
                &*r.choice as &NSView,
            ] {
                let mut f = view.frame();
                f.size.width = (width - 8.).max(1.);
                view.setFrame(f);
            }
        }
    }
    pub fn bind(&self, owner: &Controller) {
        *self.ivars().owner.borrow_mut() = Some(Weak::new(owner));
    }
    pub fn cleanup(&self) {
        self.retire();
        self.ivars().choice_widget.cleanup();
        // Disconnect every still-live recycled control before their weak owner disappears.
        unsafe {
            self.table().setDataSource(None);
            self.table().setDelegate(None);
        }
        for row in self.ivars().rows.borrow().iter().filter_map(Weak::load) {
            unsafe {
                row.ivars().choice.setTarget(None);
            }
            unsafe {
                row.ivars().choice.setAction(None);
                row.ivars().text.setDelegate(None);
            }
        }

        *self.ivars().owner.borrow_mut() = None;
    }
    pub fn show(&self, token: u64, fields: Fields, enabled: bool) -> bool {
        let fresh = self.ivars().draft.borrow().token != token;
        if fresh {
            self.retire();
            self.fresh_table();
            let values: Vec<_> = fields.0.iter().map(|f| f.default_value.clone()).collect();
            let bytes = values.iter().map(String::len).sum();
            *self.ivars().draft.borrow_mut() = Draft {
                token,
                fields: Some(fields),
                values,
                bytes,
                invalid: HashSet::new(),
            };
            self.table().reloadData();
            self.table().scrollRowToVisible(0);
        }
        self.ivars().scroll.setHidden(false);
        self.set_enabled(enabled);
        fresh
    }
    pub fn hide(&self) {
        self.ivars().choice_widget.dismiss(false);
        self.ivars().scroll.setHidden(true);
    }
    pub fn set_enabled(&self, enabled: bool) {
        if !enabled {
            self.ivars().choice_widget.dismiss(false);
        }
        self.ivars().enabled.set(enabled);
        self.fields_enabled(enabled && !self.choice_open());
    }
    fn fields_enabled(&self, enabled: bool) {
        let rows = self.table().rowsInRect(self.table().visibleRect());
        for index in rows.location..rows.location + rows.length {
            if let Some(view) = self
                .table()
                .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
                .and_then(|v| v.downcast::<Row>().ok())
            {
                view.ivars().text.setEnabled(enabled);
                view.ivars().choice.setEnabled(enabled);
            }
        }
    }
    fn live_token(&self) -> Option<u64> {
        if self.ivars().scroll.isHidden()
            || !self.ivars().enabled.get()
            || self.ivars().loading.get()
        {
            return None;
        }
        let token = self.ivars().draft.borrow().token;
        self.owner()
            .and_then(|owner| owner.current_template_token())
            .filter(|current| *current == token)
    }
    fn control_live(&self, control: &NSControl) -> bool {
        if self.ivars().choice_widget.is_open() {
            return false;
        }
        let token = if let Some(text) = control.downcast_ref::<Text>() {
            text.ivars().get()
        } else if let Some(choice) = control.downcast_ref::<Choice>() {
            choice.ivars().get()
        } else {
            return false;
        };
        self.live_token() == Some(token) && control.isEnabled()
    }
    fn flush_active(&self) {
        let rows: Vec<_> = self
            .ivars()
            .rows
            .borrow()
            .iter()
            .filter_map(Weak::load)
            .collect();
        for row in rows {
            if row.ivars().text.currentEditor().is_some() && self.control_live(&row.ivars().text) {
                self.record(&row.ivars().text);
            }
        }
    }
    pub fn retire(&self) {
        self.ivars().choice_widget.dismiss(false);
        if self.ivars().draft.borrow().token == 0 {
            self.hide();
            return;
        }
        self.ivars().scroll.setHidden(true);
        *self.ivars().draft.borrow_mut() = Draft::default();
        self.ivars().focus.set(0);
        self.ivars().loading.set(true);
        for row in self.ivars().rows.borrow().iter().filter_map(Weak::load) {
            row.ivars().text.ivars().set(0);
            row.ivars().choice.ivars().set(0);
            row.ivars().choice.setAccessibilityExpanded(false);
            if let Some(editor) = row
                .ivars()
                .text
                .currentEditor()
                .and_then(|v| v.downcast::<NSTextView>().ok())
            {
                editor.setString(ns_string!(""));
                if let Some(undo) = editor.undoManager() {
                    undo.removeAllActions();
                }
            }
            row.ivars().text.setStringValue(ns_string!(""));
            row.ivars().choice.setTitle(ns_string!(""));
            row.ivars().label.setStringValue(ns_string!(""));
            unsafe {
                row.ivars().text.setDelegate(None);
                row.ivars().choice.setAction(None);
                row.ivars().choice.setTarget(None);
            }
        }
        self.ivars().loading.set(false);
    }
    pub fn preserves(&self, token: Option<u64>) -> bool {
        token.is_some_and(|token| token == self.ivars().draft.borrow().token)
    }
    fn make_row(&self, table: &NSTableView, index: usize) -> Option<Retained<NSView>> {
        let (token, fields, value) = {
            let d = self.ivars().draft.borrow();
            (d.token, d.fields.clone()?, d.values.get(index)?.clone())
        };
        let field = fields.0.get(index)?;
        let row = unsafe {
            table.makeViewWithIdentifier_owner(ns_string!("TemplateRow"), Some(self))
        }
        .and_then(|r| r.downcast::<Row>().ok())
        .unwrap_or_else(|| {
            let main = self.mtm();
            let label = NSTextField::labelWithString(ns_string!(""), main);
            let text = Text::alloc(main).set_ivars(Cell::new(0));
            let text: Retained<Text> =
                unsafe { msg_send![super(text),initWithFrame:rect(0.,0.,1.,1.)] };
            let choice = Choice::alloc(main).set_ivars(Cell::new(0));
            let choice: Retained<Choice> =
                unsafe { msg_send![super(choice),initWithFrame:rect(0.,0.,1.,1.)] };
            choice.setBezelStyle(NSBezelStyle::Push);
            choice.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            choice.setImage(
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!("chevron.down"),
                    None,
                )
                .as_deref(),
            );
            choice.setImagePosition(NSCellImagePosition::ImageRight);
            unsafe {
                choice.setTarget(Some(self));
                choice.setAction(Some(sel!(showChoice:)));
                text.setDelegate(Some(ProtocolObject::from_ref(self)));
            }
            let row = Row::alloc(main).set_ivars(RowState {
                label,
                text,
                choice,
            });
            let row: Retained<Row> = unsafe {
                msg_send![super(row),initWithFrame:rect(0.,0.,table.bounds().size.width,64.)]
            };
            row.setIdentifier(Some(ns_string!("TemplateRow")));
            row.addSubview(&row.ivars().label);
            row.addSubview(&row.ivars().text);
            row.addSubview(&row.ivars().choice);
            let mut rows = self.ivars().rows.borrow_mut();
            rows.retain(|row| row.load().is_some());
            rows.push(Weak::new(&*row));
            row
        });
        self.ivars().loading.set(true);
        let r = row.ivars();
        unsafe {
            r.text.setDelegate(Some(ProtocolObject::from_ref(self)));
            r.choice.setTarget(Some(self));
            r.choice.setAction(Some(sel!(showChoice:)));
        }
        let width = table.tableColumns().objectAtIndex(0).width() - 8.;
        r.label.setFrame(rect(4., 40., width, 20.));
        r.label.setFont(Some(&NSFont::systemFontOfSize(12.)));
        r.label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        r.label.setStringValue(&NSString::from_str(&field.label));
        r.label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        r.text.ivars().set(token);
        r.choice.ivars().set(token);
        r.text.setTag(index as isize);
        r.choice.setTag(index as isize);
        r.text.setFrame(rect(4., 8., width, 28.));
        r.choice.setFrame(rect(4., 8., width, 28.));
        r.text
            .setAccessibilityLabel(Some(&NSString::from_str(&field.label)));
        r.choice
            .setAccessibilityLabel(Some(&NSString::from_str(&field.label)));
        r.text.setHidden(!field.options.is_empty());
        r.choice.setHidden(field.options.is_empty());
        r.text.setStringValue(&NSString::from_str(&value));
        r.choice.setTitle(&NSString::from_str(&value));
        r.choice.setAccessibilityExpanded(false);
        unsafe {
            r.choice
                .setAccessibilityValue(Some(&NSString::from_str(&value)));
        }
        r.text
            .setEnabled(self.ivars().enabled.get() && !self.choice_open());
        r.choice
            .setEnabled(self.ivars().enabled.get() && !self.choice_open());
        self.ivars().loading.set(false);
        Some(row.into_super())
    }
    fn store(&self, index: usize, value: String) {
        let mut d = self.ivars().draft.borrow_mut();
        let Some(old) = d.values.get(index) else {
            return;
        };
        let Some(total) = d
            .bytes
            .checked_sub(old.len())
            .and_then(|n| n.checked_add(value.len()))
            .filter(|n| *n <= isize::MAX as usize)
        else {
            d.invalid.insert(index);
            return;
        };
        d.bytes = total;
        d.values[index] = value;
        d.invalid.remove(&index);
    }
    fn record(&self, text: &Text) {
        if !self.control_live(text) {
            return;
        }
        if text.ivars().get() != self.ivars().draft.borrow().token {
            return;
        }
        let string = text
            .currentEditor()
            .and_then(|e| e.downcast::<NSTextView>().ok())
            .map(|e| e.string())
            .unwrap_or_else(|| text.stringValue());
        self.store(text.tag() as usize, string.to_string());
    }
    pub fn marked(&self) -> bool {
        self.ivars()
            .scroll
            .window()
            .and_then(|w| w.firstResponder())
            .and_then(|r| r.downcast::<NSTextView>().ok())
            .is_some_and(|t| unsafe { msg_send![&*t, hasMarkedText] })
    }
    pub fn capture(&self, token: u64) -> Result<Vec<(String, String)>, String> {
        if self.ivars().choice_widget.is_open() {
            return Err("Finish the current choice first.".into());
        }
        if self.marked() {
            return Err("Finish the current text composition first.".into());
        }
        self.flush_active();
        let d = self.ivars().draft.borrow();
        if d.token != token {
            return Err("This input is no longer current.".into());
        }
        if !d.invalid.is_empty() {
            return Err("Input exceeds the supported size.".into());
        }
        let fields = d
            .fields
            .as_ref()
            .ok_or("Template fields are unavailable.")?;
        Ok(fields
            .0
            .iter()
            .zip(&d.values)
            .map(|(field, value)| (field.id.clone(), value.clone()))
            .collect())
    }
    pub fn focus(&self, index: usize) {
        self.flush_active();
        self.ivars().focus.set(index);
        self.table().scrollRowToVisible(index as isize);
        if let Some(row) = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
            .and_then(|v| v.downcast::<Row>().ok())
        {
            if let Some(window) = row.window() {
                if row.ivars().text.isHidden() {
                    window.makeFirstResponder(Some(&row.ivars().choice));
                } else {
                    window.makeFirstResponder(Some(&row.ivars().text));
                }
            }
        }
    }
    pub fn restore_focus(&self) {
        self.focus(self.ivars().focus.get());
    }
    pub fn accessibility_text(&self) -> Vec<String> {
        if self.ivars().choice_widget.is_open() {
            return self.ivars().choice_widget.accessibility_text();
        }
        if self.ivars().scroll.isHidden() {
            return vec![];
        }
        let mut result = vec![];
        let table = self.table();
        let rows = table.rowsInRect(table.visibleRect());
        for index in rows.location..rows.location + rows.length {
            if let Some(row) = table
                .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
                .and_then(|v| v.downcast::<Row>().ok())
            {
                let control: &NSView = if row.ivars().text.isHidden() {
                    &row.ivars().choice
                } else {
                    &row.ivars().text
                };
                if let Some(label) = control.accessibilityLabel() {
                    result.push(label.to_string());
                }
            }
        }
        result
    }
    pub fn visible_text(&self) -> Vec<String> {
        if self.ivars().choice_widget.is_open() {
            return self.ivars().choice_widget.visible_text();
        }
        if self.ivars().scroll.isHidden() {
            return vec![];
        }
        let mut text = vec![];
        let rows = self.table().rowsInRect(self.table().visibleRect());
        for index in rows.location..rows.location + rows.length {
            if let Some(row) = self
                .table()
                .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
                .and_then(|v| v.downcast::<Row>().ok())
            {
                text.push(row.ivars().label.stringValue().to_string());
                text.push(if row.ivars().text.isHidden() {
                    row.ivars().choice.title().to_string()
                } else {
                    row.ivars().text.stringValue().to_string()
                });
            }
        }
        text
    }
    #[cfg(feature = "test-tools")]
    pub fn scroll_to(&self, y: f64) {
        let clip = self.ivars().scroll.contentView();
        let mut bounds = clip.bounds();
        bounds.origin.y = y;
        clip.scrollToPoint(clip.constrainBoundsRect(bounds).origin);
        self.ivars().scroll.reflectScrolledClipView(&clip);
    }
    #[cfg(feature = "test-tools")]
    pub fn mark_field(&self, index: usize, value: &str) -> Result<(), String> {
        self.focus(index);
        let row = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
            .and_then(|v| v.downcast::<Row>().ok())
            .ok_or("Field unavailable")?;
        let editor = row
            .ivars()
            .text
            .currentEditor()
            .and_then(|v| v.downcast::<NSTextView>().ok())
            .ok_or("Field is not editable")?;
        let value = NSString::from_str(value);
        unsafe {
            let _: () = msg_send![&*editor,setMarkedText:&*value,selectedRange:objc2_foundation::NSRange::new(value.length(),0),replacementRange:objc2_foundation::NSRange::new(0,editor.string().length())];
        }
        Ok(())
    }
    #[cfg(feature = "test-tools")]
    pub fn unmark(&self) {
        if let Some(editor) = self
            .ivars()
            .scroll
            .window()
            .and_then(|w| w.firstResponder())
            .and_then(|v| v.downcast::<NSTextView>().ok())
        {
            unsafe {
                let _: () = msg_send![&*editor, unmarkText];
            }
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn retain_field(&self, index: usize) -> Result<(), String> {
        self.focus(index);
        let row = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
            .and_then(|v| v.downcast::<Row>().ok())
            .ok_or("Field unavailable")?;
        let editor = row
            .ivars()
            .text
            .currentEditor()
            .and_then(|v| v.downcast::<NSTextView>().ok())
            .ok_or("Field is not editable")?;
        *self.ivars().stale.borrow_mut() = Some((row.ivars().text.clone(), editor));
        Ok(())
    }
    #[cfg(feature = "test-tools")]
    pub fn stale_callbacks(&self) -> Result<serde_json::Value, String> {
        let controls = self
            .ivars()
            .stale
            .borrow()
            .clone()
            .ok_or("No retained field")?;
        let note = unsafe {
            NSNotification::notificationWithName_object(
                ns_string!("NSControlTextDidBeginEditingNotification"),
                Some(&controls.0),
            )
        };
        unsafe {
            let _: () = msg_send![self,controlTextDidBeginEditing:&*note];
            let _: () = msg_send![self,controlTextDidChange:&*note];
            let _: bool = msg_send![self,control:&*controls.0,textView:&*controls.1,doCommandBySelector:sel!(insertNewline:)];
            let _: bool = msg_send![self,control:&*controls.0,textView:&*controls.1,doCommandBySelector:sel!(cancelOperation:)];
        }
        Ok(
            serde_json::json!({"oldValue":controls.0.stringValue().to_string(),"oldToken":controls.0.ivars().get()}),
        )
    }
    #[cfg(feature = "test-tools")]
    pub fn inspect(&self) -> serde_json::Value {
        let d = self.ivars().draft.borrow();
        serde_json::json!({"token":d.token,"fields":d.fields.as_ref().map(|f|f.0.len()),"values":d.values,"enabled":self.ivars().enabled.get(),"scrollY":self.table().visibleRect().origin.y,"liveRows":self.ivars().rows.borrow().iter().filter(|r|r.load().is_some()).count(),"optionReads":self.ivars().choice_widget.reads(),"marked":self.marked(),"choice":self.ivars().choice_widget.inspect(),"fieldsAXHidden":self.ivars().scroll.isAccessibilityHidden(),"fieldControlsEnabled":self.ivars().rows.borrow().iter().filter_map(Weak::load).filter(|r|r.ivars().text.ivars().get()==d.token).map(|r|if r.ivars().text.isHidden(){r.ivars().choice.isEnabled()}else{r.ivars().text.isEnabled()}).collect::<Vec<_>>(),"focusedField":self.ivars().focus.get()})
    }
    #[cfg(feature = "test-tools")]
    pub fn type_field(&self, index: usize, value: &str, append: bool) -> Result<(), String> {
        self.focus(index);
        let row = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
            .and_then(|v| v.downcast::<Row>().ok())
            .ok_or("Field is not visible")?;
        if row.ivars().text.isHidden() {
            return Err("Field is a choice".into());
        }
        let editor = row
            .ivars()
            .text
            .currentEditor()
            .and_then(|e| e.downcast::<NSTextView>().ok())
            .ok_or("Field did not become editable")?;
        let value = NSString::from_str(value);
        let range = if append {
            objc2_foundation::NSRange::new(editor.string().length(), 0)
        } else {
            objc2_foundation::NSRange::new(0, editor.string().length())
        };
        unsafe {
            let _: () = msg_send![&*editor, insertText:&*value, replacementRange:range];
        }
        self.record(&row.ivars().text);
        Ok(())
    }
    #[cfg(feature = "test-tools")]
    pub fn choose_field(&self, index: usize, selected: usize) -> Result<(), String> {
        self.focus(index);
        let row = self
            .table()
            .viewAtColumn_row_makeIfNecessary(0, index as isize, true)
            .and_then(|v| v.downcast::<Row>().ok())
            .ok_or("Field is not visible")?;
        self.open_choice(&row.ivars().choice);
        if !self.ivars().choice_widget.select(selected) {
            return Err("Choice unavailable".into());
        }
        self.ivars().choice_widget.commit(selected);
        Ok(())
    }
}
