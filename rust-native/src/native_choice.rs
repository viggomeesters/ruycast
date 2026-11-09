//! A virtual, contained choice list. Never creates an AppKit popup window.
use crate::{native::Controller, native_template::Form};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::*;
use objc2_foundation::{
    ns_string, NSIndexSet, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use ruycast_native::template_form::Fields;
use std::cell::{Cell, RefCell};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

define_class!(
    #[unsafe(super=NSTableRowView)] #[thread_kind=MainThreadOnly]
    struct OptionRow;
    impl OptionRow {
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self,_:NSRect){NSColor::unemphasizedSelectedContentBackgroundColor().setFill();NSRectFill(self.bounds());}
        #[unsafe(method(interiorBackgroundStyle))]
        fn background_style(&self)->NSBackgroundStyle {NSBackgroundStyle::Normal}
    }
);
define_class!(
    #[unsafe(super=NSTableView)] #[thread_kind=MainThreadOnly] #[ivars=Cell<u64>]
    pub(crate) struct List;
    impl List {
        #[unsafe(method(mouseDown:))]
        fn mouse(&self,event:&NSEvent){
            let _lease=self.retain();
            if let Some(owner)=self.target().and_then(|o|o.downcast::<Widget>().ok()){
                owner.guard(||{if owner.valid(self){unsafe{let _:()=msg_send![super(self),mouseDown:event];}}});
            }
        }
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent) {
            let _lease=self.retain();
            if let Some(owner)=self.target().and_then(|o|o.downcast::<Widget>().ok()) {
                owner.guard(|| {if owner.valid(self) && !owner.key(event) {unsafe{let _:()=msg_send![super(self),keyDown:event];}}});
            }
        }
    }
);
define_class!(
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=RefCell<Option<Weak<Widget>>>]
    struct Cover;
    impl Cover {
        #[unsafe(method(drawRect:))]
        fn draw(&self,_:NSRect) {NSColor::windowBackgroundColor().setFill();NSRectFill(self.bounds());}
        #[unsafe(method(mouseDown:))]
        fn mouse(&self,_:&NSEvent) {
            let owner=self.ivars().borrow().as_ref().and_then(Weak::load);
            if let Some(owner)=owner {owner.guard(||owner.dismiss(true));}
        }
    }
);
#[derive(Clone)]
enum Owner {
    Template(Weak<Form>),
    Editor(Weak<Controller>),
}
enum Options {
    Template(Fields, usize),
    Apps {
        choices: ruycast_native::app_choice::Choices,
        missing: Option<(String, String)>,
    },
}
impl Options {
    fn len(&self) -> usize {
        match self {
            Self::Template(f, i) => f.0[*i].options.len(),
            Self::Apps { choices, missing } => 1 + usize::from(missing.is_some()) + choices.0.len(),
        }
    }
    fn label(&self, i: usize) -> Option<&str> {
        match self {
            Self::Template(f, field) => f.0[*field].options.get(i).map(String::as_str),
            Self::Apps { choices, missing } => {
                if i == 0 {
                    return Some("Default app");
                }
                if let Some((_, label)) = missing {
                    if i == 1 {
                        return Some(label);
                    }
                }
                choices
                    .0
                    .get(i - 1 - usize::from(missing.is_some()))
                    .map(|e| e.label.as_str())
            }
        }
    }
    fn value(&self, i: usize) -> Option<&str> {
        match self {
            Self::Template(..) => self.label(i),
            Self::Apps { choices, missing } => {
                if i == 0 {
                    return Some("");
                }
                if let Some((id, _)) = missing {
                    if i == 1 {
                        return Some(id);
                    }
                }
                choices
                    .0
                    .get(i - 1 - usize::from(missing.is_some()))
                    .map(|e| e.id.as_str())
            }
        }
    }
    fn heading(&self) -> &str {
        match self {
            Self::Template(f, i) => &f.0[*i].label,
            Self::Apps { .. } => "Open with",
        }
    }
}
struct Open {
    token: u64,
    field: usize,
    generation: u64,
    options: Options,
}
pub(crate) struct State {
    cover: Retained<Cover>,
    heading: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    table: RefCell<Option<Retained<List>>>,
    owner: RefCell<Option<Owner>>,
    open: RefCell<Option<Open>>,
    generation: Cell<u64>,
    #[cfg(feature = "test-tools")]
    reads: Cell<u64>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<List>>>,
}
define_class!(
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[ivars=State]
    pub(crate) struct Widget;
    unsafe impl NSObjectProtocol for Widget {}
    unsafe impl NSControlTextEditingDelegate for Widget {}
    unsafe impl NSTableViewDataSource for Widget {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self,table:&NSTableView)->isize {
            self.guard(||if self.valid(table){self.ivars().open.borrow().as_ref().map(|o|o.options.len() as isize).unwrap_or(0)}else{0}).unwrap_or(0)
        }
    }
    unsafe impl NSTableViewDelegate for Widget {
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn background(&self,table:&NSTableView,_:isize)->Option<Retained<NSTableRowView>> {
            self.guard(||{
                if !self.valid(table){return None;}
                let row=OptionRow::alloc(self.mtm()).set_ivars(());
                let row:Retained<OptionRow>=unsafe{msg_send![super(row),initWithFrame:rect(0.,0.,100.,28.)]};
                Some(row.into_super())
            }).flatten()
        }
        #[unsafe(method_id(tableView:typeSelectStringForTableColumn:row:))]
        fn type_text(&self,table:&NSTableView,_:Option<&NSTableColumn>,index:isize)->Option<Retained<NSString>>{
            self.guard(||{
                if !self.valid(table){return None;}
                let open=self.ivars().open.borrow();let o=open.as_ref()?;
                o.options.label(usize::try_from(index).ok()?).map(NSString::from_str)
            }).flatten()
        }
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn row(&self,table:&NSTableView,_:Option<&NSTableColumn>,index:isize)->Option<Retained<NSView>> {
            self.guard(|| {
                if !self.valid(table){return None;}
                let text={let open=self.ivars().open.borrow();let o=open.as_ref()?;o.options.label(usize::try_from(index).ok()?)?.to_string()};
                #[cfg(feature="test-tools")] self.ivars().reads.set(self.ivars().reads.get()+1);
                let label=unsafe{table.makeViewWithIdentifier_owner(ns_string!("TemplateOption"),Some(self))}.and_then(|v|v.downcast::<NSTextField>().ok()).unwrap_or_else(||{
                    let label=NSTextField::labelWithString(ns_string!(""),self.mtm());label.setIdentifier(Some(ns_string!("TemplateOption")));label
                });
                label.setStringValue(&NSString::from_str(&text));label.setAccessibilityLabel(Some(&NSString::from_str(&text)));
                label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                label.setTextColor(Some(&NSColor::labelColor()));
                label.setFrame(rect(8.,0.,table.tableColumns().objectAtIndex(0).width()-16.,28.));
                Some(label.into_super().into_super())
            }).flatten()
        }
    }
    impl Widget {
        #[unsafe(method(choose:))]
        fn clicked(&self,sender:&AnyObject) {self.guard(||{
            let lease=self.ivars().table.borrow().clone();
            if let (Some(table),Some(sender))=(lease,sender.downcast_ref::<List>()) {
                if std::ptr::eq(&*table,sender){let row=table.clickedRow();if row>=0{self.commit_for(&table,row as usize);}}
            }
        });}
    }
);
impl Widget {
    pub fn set_frame(&self, frame: NSRect) {
        self.ivars().cover.setFrame(frame);
        self.ivars().heading.setFrame(rect(
            8.,
            frame.size.height - 30.,
            frame.size.width - 16.,
            22.,
        ));
        self.ivars().scroll.setFrame(rect(
            8.,
            8.,
            frame.size.width - 16.,
            frame.size.height - 46.,
        ));
    }
    pub fn new(main: MainThreadMarker, parent: &NSView, frame: NSRect) -> Retained<Self> {
        let cover = Cover::alloc(main).set_ivars(RefCell::new(None));
        let cover: Retained<Cover> = unsafe { msg_send![super(cover),initWithFrame:frame] };
        cover.setHidden(true);
        let heading = NSTextField::labelWithString(ns_string!(""), main);
        heading.setFrame(rect(
            8.,
            frame.size.height - 30.,
            frame.size.width - 16.,
            22.,
        ));
        heading.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        cover.addSubview(&heading);
        let scroll = NSScrollView::new(main);
        scroll.setFrame(rect(
            8.,
            8.,
            frame.size.width - 16.,
            frame.size.height - 46.,
        ));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setBorderType(NSBorderType::BezelBorder);
        cover.addSubview(&scroll);
        parent.addSubview(&cover);
        let value = Self::alloc(main).set_ivars(State {
            cover,
            heading,
            scroll,
            table: RefCell::new(None),
            owner: RefCell::new(None),
            open: RefCell::new(None),
            generation: Cell::new(0),
            #[cfg(feature = "test-tools")]
            reads: Cell::new(0),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
        });
        let value: Retained<Self> = unsafe { msg_send![super(value), init] };
        *value.ivars().cover.ivars().borrow_mut() = Some(Weak::new(&*value));
        value
    }
    pub fn bind(&self, owner: &Form) {
        *self.ivars().owner.borrow_mut() = Some(Owner::Template(Weak::new(owner)));
    }
    pub fn bind_editor(&self, owner: &Controller) {
        *self.ivars().owner.borrow_mut() = Some(Owner::Editor(Weak::new(owner)));
    }
    pub fn guard<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let owner = self.ivars().owner.borrow().clone();
        match owner {
            Some(Owner::Template(w)) => w.load().and_then(|o| o.guard(f)),
            Some(Owner::Editor(w)) => w.load().and_then(|o| o.guard_callback(f)),
            None => None,
        }
    }
    fn owner_valid(&self, token: u64, field: usize) -> bool {
        let owner = self.ivars().owner.borrow().clone();
        match owner {
            Some(Owner::Template(w)) => w.load().is_some_and(|o| o.choice_valid(token, field)),
            Some(Owner::Editor(w)) => w.load().is_some_and(|o| o.app_choice_valid(token)),
            None => false,
        }
    }
    fn closed(&self, token: u64, field: usize, restore: bool) {
        let owner = self.ivars().owner.borrow().clone();
        match owner {
            Some(Owner::Template(w)) => {
                if let Some(o) = w.load() {
                    o.choice_closed(token, field, restore);
                }
            }
            Some(Owner::Editor(w)) => {
                if let Some(o) = w.load() {
                    o.app_choice_closed(token, restore);
                }
            }
            None => {}
        }
    }
    fn committed(&self, token: u64, field: usize, value: String) {
        let owner = self.ivars().owner.borrow().clone();
        match owner {
            Some(Owner::Template(w)) => {
                if let Some(o) = w.load() {
                    o.choice_committed(token, field, value);
                }
            }
            Some(Owner::Editor(w)) => {
                if let Some(o) = w.load() {
                    o.app_choice_committed(token, value);
                }
            }
            None => {}
        }
    }
    fn tab(&self, field: usize, back: bool) {
        let owner = self.ivars().owner.borrow().clone();
        match owner {
            Some(Owner::Template(w)) => {
                if let Some(o) = w.load() {
                    o.choice_tab(field, back);
                }
            }
            Some(Owner::Editor(w)) => {
                if let Some(o) = w.load() {
                    o.app_choice_tab(back);
                }
            }
            None => {}
        }
    }
    pub fn is_open(&self) -> bool {
        !self.ivars().cover.isHidden() && self.ivars().open.borrow().is_some()
    }
    fn valid(&self, table: &NSTableView) -> bool {
        let Some(table) = table.downcast_ref::<List>() else {
            return false;
        };
        if !self.is_open() {
            return false;
        }
        let open = self.ivars().open.borrow();
        let Some(o) = open.as_ref() else {
            return false;
        };
        table.ivars().get() == o.generation
            && self
                .ivars()
                .table
                .borrow()
                .as_ref()
                .is_some_and(|t| std::ptr::eq(&**t, table))
            && self.owner_valid(o.token, o.field)
    }
    pub fn open(&self, token: u64, field: usize, fields: Fields, selected: Option<usize>) -> bool {
        self.open_options(token, field, Options::Template(fields, field), selected)
    }
    pub fn open_apps(
        &self,
        token: u64,
        choices: ruycast_native::app_choice::Choices,
        saved: &str,
    ) -> bool {
        let known = choices.index(saved);
        let missing = if saved.is_empty() || known.is_some() {
            None
        } else {
            Some((saved.to_string(), format!("Saved app — {saved}")))
        };
        let selected = if saved.is_empty() {
            0
        } else {
            known.map(|i| i + 1).unwrap_or(1)
        };
        self.open_options(token, 0, Options::Apps { choices, missing }, Some(selected))
    }
    fn open_options(
        &self,
        token: u64,
        field: usize,
        options: Options,
        selected: Option<usize>,
    ) -> bool {
        self.dismiss(false);
        let Some(generation) = self.ivars().generation.get().checked_add(1) else {
            return false;
        };
        if !self.owner_valid(token, field) {
            return false;
        }
        self.ivars().generation.set(generation);
        let label = options.heading().to_string();
        self.ivars()
            .heading
            .setStringValue(&NSString::from_str(&label));
        self.ivars()
            .heading
            .setAccessibilityLabel(Some(&NSString::from_str(&label)));
        *self.ivars().open.borrow_mut() = Some(Open {
            token,
            field,
            generation,
            options,
        });
        let table = List::alloc(self.mtm()).set_ivars(Cell::new(generation));
        let frame = self.ivars().scroll.contentView().bounds();
        let table: Retained<List> = unsafe { msg_send![super(table),initWithFrame:frame] };
        table.setHeaderView(None);
        table.setRowHeight(28.);
        table.setAllowsEmptySelection(true);
        table.setAllowsMultipleSelection(false);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAccessibilityLabel(Some(&NSString::from_str(&label)));
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(self.mtm()),
            ns_string!("option"),
        );
        column.setWidth(frame.size.width - 20.);
        table.addTableColumn(&column);
        unsafe {
            table.setTarget(Some(self));
            table.setAction(Some(sel!(choose:)));
            table.setDelegate(Some(ProtocolObject::from_ref(self)));
            table.setDataSource(Some(ProtocolObject::from_ref(self)));
        }
        *self.ivars().table.borrow_mut() = Some(table.clone());
        self.ivars().scroll.setDocumentView(Some(&table));
        self.ivars().cover.setHidden(false);
        table.reloadData();
        if let Some(index) = selected {
            table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
            table.scrollRowToVisible(index as isize);
        } else {
            unsafe {
                table.deselectAll(None);
            }
        }
        if let Some(window) = table.window() {
            window.makeFirstResponder(Some(&table));
        }
        true
    }
    pub fn dismiss(&self, restore: bool) {
        let old = self.ivars().open.borrow_mut().take();
        self.ivars().cover.setHidden(true);
        self.ivars().heading.setStringValue(ns_string!(""));
        self.ivars().heading.setAccessibilityLabel(None);
        if let Some(table) = self.ivars().table.borrow_mut().take() {
            let rows = table.rowsInRect(table.visibleRect());
            for index in rows.location..rows.location + rows.length {
                if let Some(label) = table
                    .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
                    .and_then(|v| v.downcast::<NSTextField>().ok())
                {
                    label.setStringValue(ns_string!(""));
                    label.setAccessibilityLabel(None);
                }
            }
            unsafe {
                table.setTarget(None);
                table.setAction(None);
                table.setDelegate(None);
                table.setDataSource(None);
            }
            table.ivars().set(0);
        }
        self.ivars().scroll.setDocumentView(None);
        if let Some(o) = old {
            self.closed(o.token, o.field, restore);
        }
    }
    pub fn cleanup(&self) {
        self.dismiss(false);
        *self.ivars().owner.borrow_mut() = None;
        *self.ivars().cover.ivars().borrow_mut() = None;
    }
    pub fn commit(&self, index: usize) {
        let value = {
            let open = self.ivars().open.borrow();
            let Some(o) = open.as_ref() else {
                return;
            };
            if !self.owner_valid(o.token, o.field) {
                return;
            }
            o.options
                .value(index)
                .map(str::to_string)
                .map(|v| (o.token, o.field, v))
        };
        if let Some((token, field, value)) = value {
            self.dismiss(false);
            self.committed(token, field, value);
        }
    }
    fn commit_for(&self, table: &List, index: usize) {
        if self.valid(table) {
            self.commit(index);
        }
    }
    pub fn key(&self, event: &NSEvent) -> bool {
        if !self.is_open() {
            return false;
        }
        let table = self.ivars().table.borrow().clone();
        let Some(table) = table else {
            return true;
        };
        if !self.valid(&table) {
            self.dismiss(false);
            return true;
        }
        let flags = event
            .modifierFlags()
            .intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
        if flags.contains(NSEventModifierFlags::Command) {
            return true;
        }
        match event.keyCode() {
            53 => self.dismiss(true),
            36 | 76 | 49 => {
                let row = table.selectedRow();
                if row >= 0 {
                    self.commit(row as usize);
                }
            }
            48 => {
                let field = self.ivars().open.borrow().as_ref().map(|o| o.field);
                self.dismiss(false);
                if let Some(field) = field {
                    self.tab(field, flags.contains(NSEventModifierFlags::Shift));
                }
            }
            125 | 126 | 115 | 119 => {
                let count = table.numberOfRows();
                if count > 0 {
                    let row = match event.keyCode() {
                        115 => 0,
                        119 => count - 1,
                        125 => (table.selectedRow() + 1).clamp(0, count - 1),
                        _ => (table.selectedRow() - 1).clamp(0, count - 1),
                    };
                    table.selectRowIndexes_byExtendingSelection(
                        &NSIndexSet::indexSetWithIndex(row as usize),
                        false,
                    );
                    table.scrollRowToVisible(row);
                }
            }
            _ => {
                return false;
            }
        }
        true
    }
    pub fn visible_text(&self) -> Vec<String> {
        if !self.is_open() {
            return vec![];
        }
        let table = self.ivars().table.borrow().clone();
        let Some(table) = table else {
            return vec![];
        };
        let rows = table.rowsInRect(table.visibleRect());
        let mut result = vec![self.ivars().heading.stringValue().to_string()];
        for index in rows.location..rows.location + rows.length {
            if let Some(v) = table
                .viewAtColumn_row_makeIfNecessary(0, index as isize, false)
                .and_then(|v| v.downcast::<NSTextField>().ok())
            {
                result.push(v.stringValue().to_string());
            }
        }
        result
    }
    pub fn accessibility_text(&self) -> Vec<String> {
        if !self.is_open() {
            return vec![];
        }
        let table = self.ivars().table.borrow().clone();
        let Some(table) = table else {
            return vec![];
        };
        let mut result = vec![];
        for view in [&*self.ivars().heading as &NSView, &*table as &NSView] {
            if let Some(label) = view.accessibilityLabel() {
                result.push(label.to_string());
            }
        }
        let rows = table.rowsInRect(table.visibleRect());
        for index in rows.location..rows.location + rows.length {
            if let Some(view) = table.viewAtColumn_row_makeIfNecessary(0, index as isize, false) {
                if let Some(label) = view.accessibilityLabel() {
                    result.push(label.to_string());
                }
            }
        }
        result
    }
    #[cfg(feature = "test-tools")]
    pub fn inspect(&self) -> serde_json::Value {
        let table = self.ivars().table.borrow();
        serde_json::json!({"open":self.is_open(),"generation":self.ivars().generation.get(),"field":self.ivars().open.borrow().as_ref().map(|o|o.field),"selected":table.as_ref().map(|t|t.selectedRow()),"rows":table.as_ref().map(|t|t.numberOfRows()),"visibleRows":table.as_ref().map(|t|t.rowsInRect(t.visibleRect()).length),"reads":self.ivars().reads.get()})
    }
    #[cfg(feature = "test-tools")]
    pub fn reads(&self) -> u64 {
        self.ivars().reads.get()
    }
    #[cfg(feature = "test-tools")]
    pub fn retain(&self) {
        *self.ivars().stale.borrow_mut() = self.ivars().table.borrow().clone();
    }
    #[cfg(feature = "test-tools")]
    pub fn stale(&self, index: usize) {
        if let Some(table) = self.ivars().stale.borrow().clone() {
            self.commit_for(&table, index);
        }
    }
    #[cfg(feature = "test-tools")]
    pub fn select(&self, index: usize) -> bool {
        let table = self.ivars().table.borrow().clone();
        let Some(table) = table else {
            return false;
        };
        if index >= table.numberOfRows() as usize {
            return false;
        }
        table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(index), false);
        table.scrollRowToVisible(index as isize);
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> Result<(), String> {
        let table = self
            .ivars()
            .table
            .borrow()
            .clone()
            .ok_or("Choice unavailable")?;
        if !self.valid(&table) || index >= table.numberOfRows() as usize {
            return Err("Choice unavailable".into());
        }
        table.scrollRowToVisible(index as isize);
        table.layoutSubtreeIfNeeded();
        let r = table.rectOfRow(index as isize);
        let point = table.convertPoint_toView(
            NSPoint::new(r.origin.x + 8., r.origin.y + r.size.height / 2.),
            None,
        );
        let window = table.window().ok_or("Choice window unavailable")?;
        let event = |kind, pressure| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind,point,NSEventModifierFlags::empty(),0.,window.windowNumber(),None,1,1,pressure).ok_or("Choice event unavailable")
        };
        let up = event(NSEventType::LeftMouseUp, 0.)?;
        let down = event(NSEventType::LeftMouseDown, 1.)?;
        let receiver = &*table as *const List;
        // The widget owns the current table at dispatch. Do not let a fixture
        // lease mask the product's whole-event lifetime guard during retirement.
        drop(table);
        NSApplication::sharedApplication(self.mtm()).postEvent_atStart(&up, true);
        unsafe {
            let _: () = msg_send![receiver,mouseDown:&*down];
        }
        Ok(())
    }
}
