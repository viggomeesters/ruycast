//! Fully owned provider substitute. No native discovery, AX query, permission
//! check, activation, shortcut CLI or foreground-app lookup is permitted here.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
#[derive(Clone, Default)]
pub struct Node {
    pub unreadable_lists: bool,
    pub frame: Option<rustcast_core::Frame>,
    pub pid: i32,
    pub strings: std::collections::HashMap<String, String>,
    pub bools: std::collections::HashMap<String, bool>,
    pub ints: std::collections::HashMap<String, i64>,
    pub lists: std::collections::HashMap<String, Vec<Element>>,
    pub elements: std::collections::HashMap<String, Element>,
}
pub type SafariEffectHook = Box<dyn FnOnce(&Fixture) + Send>;
pub struct Fixture {
    pub file_paths: Mutex<Vec<String>>,
    pub process_fixture:crate::processes::fixture::Fixture,
    pub nodes: Mutex<std::collections::HashMap<Element, Node>>,
    pub apps: Mutex<Vec<Application>>,
    pub application_roots: Mutex<std::collections::HashMap<i32, Element>>,
    pub application_reads: Mutex<Vec<i32>>,
    pub list_requests: Mutex<Vec<(Element, String, usize)>>,
    pub trace_read_requests: AtomicBool,
    pub tabs: Mutex<Vec<Tab>>,
    pub shortcuts: Mutex<Vec<Command>>,
    pub shortcuts_failure: AtomicBool,
    pub trusted: AtomicBool,
    pub safari_denied: AtomicBool,
    pub reads: AtomicUsize,
    pub effects: Mutex<Vec<String>>,
    pub delay_ms: AtomicUsize,
    pub before_identity: Mutex<Option<SafariEffectHook>>,
    pub selected: Mutex<Option<Tab>>,
    pub after_safari_select: Mutex<Option<SafariEffectHook>>,
    pub after_safari_front: Mutex<Option<SafariEffectHook>>,
    pub restore_failure: AtomicBool,
    pub raise_failure: AtomicBool,
    pub after_window_restore: Mutex<Option<SafariEffectHook>>,
}
impl Default for Fixture {
    fn default() -> Self {
        let id = Identity {
            pid: 730001,
            uid: unsafe { libc::getuid() },
            seconds: 11,
            micros: 22,
        };
        let app = Application {
            identity: id,
            name: "Owned Editor".into(),
            bundle_identifier: Some("invalid.ruycast.owned".into()),
        };
        let mut application = Node {
            pid: id.pid,
            ..Default::default()
        };
        application
            .lists
            .insert("AXWindows".into(), vec![Element(2), Element(3)]);
        application.elements.insert("AXMenuBar".into(), Element(4));
        application
            .elements
            .insert("AXFocusedWindow".into(), Element(2));
        let node = |title: &str, role: &str| {
            let mut node = Node {
                pid: id.pid,
                ..Default::default()
            };
            node.strings.insert("AXTitle".into(), title.into());
            node.strings.insert("AXRole".into(), role.into());
            node.bools.insert("AXEnabled".into(), true);
            node
        };
        let mut first = node("Owned Project", "AXWindow");
        first.frame = Some(rustcast_core::Frame {
            x: 40.,
            y: 50.,
            width: 600.,
            height: 400.,
        });
        first.bools.insert("AXMinimized".into(), false);
        let mut second = node("Owned Draft", "AXWindow");
        second.bools.insert("AXMinimized".into(), true);
        let mut bar = node("", "AXMenuBar");
        bar.lists.insert("AXChildren".into(), vec![Element(5)]);
        let mut file = node("File", "AXMenuBarItem");
        file.lists.insert("AXChildren".into(), vec![Element(6)]);
        let mut menu = node("", "AXMenu");
        menu.lists
            .insert("AXChildren".into(), vec![Element(7), Element(8)]);
        let mut open = node("Open Owned…", "AXMenuItem");
        open.strings.insert("AXMenuItemCmdChar".into(), "o".into());
        open.ints.insert("AXMenuItemCmdModifiers".into(), 0);
        let mut unavailable = node("Unavailable Owned", "AXMenuItem");
        unavailable.bools.insert("AXEnabled".into(), false);
        Self{file_paths:Mutex::new(vec!["/owned/Fixture Document.txt".into(),"/owned/Fixture Folder".into()]),process_fixture:Default::default(),nodes:Mutex::new([(Element(1),application),(Element(2),first),(Element(3),second),(Element(4),bar),(Element(5),file),(Element(6),menu),(Element(7),open),(Element(8),unavailable)].into()),apps:Mutex::new(vec![app]),application_roots:Mutex::new([(id.pid,Element(1))].into()),application_reads:Default::default(),list_requests:Default::default(),trace_read_requests:AtomicBool::new(false),tabs:Mutex::new(vec![Tab{window:51,index:1,title:"Owned Tab".into(),url:"https://example.invalid/owned".into()}]),shortcuts:Mutex::new(apple_shortcuts::parse(b"Owned Shortcut (aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)\nOwned Shortcut (11111111-2222-3333-4444-555555555555)\n").unwrap()),shortcuts_failure:AtomicBool::new(false),trusted:AtomicBool::new(true),safari_denied:AtomicBool::new(false),reads:AtomicUsize::new(0),effects:Default::default(),delay_ms:AtomicUsize::new(0),before_identity:Default::default(),selected:Default::default(),after_safari_select:Default::default(),after_safari_front:Default::default(),restore_failure:AtomicBool::new(false),raise_failure:AtomicBool::new(false),after_window_restore:Default::default()}
    }
}
impl Fixture {
    fn read(&self, t: &Ticket) -> Result<(), String> {
        t.check()?;
        self.reads.fetch_add(1, Ordering::AcqRel);
        let delay = self.delay_ms.load(Ordering::Acquire);
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay as u64));
        }
        t.check()
    }
    pub fn previous(&self) -> Option<Application> {
        self.apps.lock().unwrap().first().cloned()
    }
    pub fn activate(self: &Arc<Self>) -> Activate {
        let f = self.clone();
        Arc::new(move |id, t| {
            t.check()?;
            if f.identity(id.pid) != Some(id) {
                return Err(EXPIRED.into());
            }
            f.effects.lock().unwrap().push("activate".into());
            Ok(())
        })
    }
    pub fn edit(&self, node: Element, edit: impl FnOnce(&mut Node)) {
        if let Some(n) = self.nodes.lock().unwrap().get_mut(&node) {
            edit(n);
        }
    }
}
impl Platform for Fixture {
    fn files(&self,query:&str,ticket:&Ticket)->Result<Vec<Command>,String>{self.read(ticket)?;let query=crate::file_search::query(query)?;if query.is_empty(){return Ok(vec![]);}let mut parser=crate::file_search::Parser::default();for path in self.file_paths.lock().unwrap().iter(){parser.feed(path.as_bytes());parser.feed(&[0]);}Ok(parser.commands().into_iter().filter(|c|super::matches(&c.title,&query)).collect())}
    fn processes(&self,ticket:&Ticket)->Result<Vec<crate::processes::Resource>,String>{self.read(ticket)?;crate::processes::table(&self.process_fixture,ticket)}
    fn terminate(&self,identity:Identity,ticket:&Ticket)->Result<(),String>{self.read(ticket)?;crate::processes::terminate(&self.process_fixture,identity,ticket)?;self.effects.lock().unwrap().push("terminate-owned".into());Ok(())}
    fn frame(&self, node: Element, t: &Ticket) -> Result<rustcast_core::Frame, String> {
        self.read(t)?;
        self.nodes
            .lock()
            .unwrap()
            .get(&node)
            .and_then(|n| n.frame)
            .ok_or(EXPIRED.into())
    }
    fn resize(&self, node: Element, frame: rustcast_core::Frame, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        let mut nodes = self.nodes.lock().unwrap();
        let value = nodes
            .get_mut(&node)
            .and_then(|n| n.frame.as_mut())
            .ok_or(EXPIRED)?;
        value.width = frame.width;
        value.height = frame.height;
        self.effects.lock().unwrap().push("resize".into());
        Ok(())
    }
    fn position(
        &self,
        node: Element,
        frame: rustcast_core::Frame,
        t: &Ticket,
    ) -> Result<(), String> {
        self.read(t)?;
        let mut nodes = self.nodes.lock().unwrap();
        let value = nodes
            .get_mut(&node)
            .and_then(|n| n.frame.as_mut())
            .ok_or(EXPIRED)?;
        value.x = frame.x;
        value.y = frame.y;
        self.effects.lock().unwrap().push("position".into());
        Ok(())
    }
    fn identity(&self, pid: i32) -> Option<Identity> {
        let hook = self.before_identity.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        self.apps
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.identity.pid == pid)
            .map(|a| a.identity)
    }
    fn trusted(&self) -> bool {
        self.trusted.load(Ordering::Acquire)
    }
    fn applications(&self, t: &Ticket) -> Result<Vec<Application>, String> {
        self.read(t)?;
        Ok(self.apps.lock().unwrap().clone())
    }
    fn application(&self, pid: i32, t: &Ticket) -> Result<Element, String> {
        self.read(t)?;
        if self.trace_read_requests.load(Ordering::Acquire) {
            self.application_reads.lock().unwrap().push(pid);
        }
        self.application_roots
            .lock()
            .unwrap()
            .get(&pid)
            .copied()
            .ok_or_else(|| "Owned AX application root is unavailable".into())
    }
    fn pid(&self, e: Element) -> Option<i32> {
        self.nodes.lock().unwrap().get(&e).map(|n| n.pid)
    }
    fn string(&self, e: Element, k: &str, t: &Ticket) -> Result<Option<String>, String> {
        self.read(t)?;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .and_then(|n| n.strings.get(k))
            .cloned())
    }
    fn boolean(&self, e: Element, k: &str, t: &Ticket) -> Result<Option<bool>, String> {
        self.read(t)?;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .and_then(|n| n.bools.get(k))
            .copied())
    }
    fn integer(&self, e: Element, k: &str, t: &Ticket) -> Result<Option<i64>, String> {
        self.read(t)?;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .and_then(|n| n.ints.get(k))
            .copied())
    }
    fn elements(
        &self,
        e: Element,
        k: &str,
        l: usize,
        _: bool,
        t: &Ticket,
    ) -> Result<Vec<Element>, String> {
        self.read(t)?;
        if self.trace_read_requests.load(Ordering::Acquire) {
            self.list_requests.lock().unwrap().push((e, k.into(), l));
        }
        if self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .is_some_and(|node| node.unreadable_lists)
        {
            return Err("Owned AX fixture list is unreadable".into());
        }
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .and_then(|n| n.lists.get(k))
            .map(|v| v.iter().take(l).copied().collect())
            .unwrap_or_default())
    }
    fn element(&self, e: Element, k: &str, t: &Ticket) -> Result<Option<Element>, String> {
        self.read(t)?;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&e)
            .and_then(|n| n.elements.get(k))
            .copied())
    }
    fn same(&self, a: Element, b: Element) -> bool {
        a == b
    }
    fn restore(&self, e: Element, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        if self.restore_failure.load(Ordering::Acquire) {
            self.effects.lock().unwrap().push("restore".into());
            return Err("Owned AX fixture restore failed".into());
        }
        self.edit(e, |n| {
            n.bools.insert("AXMinimized".into(), false);
        });
        self.effects.lock().unwrap().push("restore".into());
        let hook = self.after_window_restore.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        Ok(())
    }
    fn action(&self, _: Element, a: &str, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        self.effects.lock().unwrap().push(a.into());
        if a == "AXRaise" && self.raise_failure.load(Ordering::Acquire) {
            return Err("Owned AX fixture raise failed".into());
        }
        Ok(())
    }
    fn safari(&self, t: &Ticket) -> Result<Identity, String> {
        self.read(t)?;
        if self.safari_denied.load(Ordering::Acquire) {
            return Err(SAFARI_PERMISSION.into());
        }
        self.previous()
            .map(|a| a.identity)
            .ok_or("Open Safari to search its tabs.".into())
    }
    fn safari_windows(&self, _: Identity, t: &Ticket) -> Result<Vec<i32>, String> {
        self.read(t)?;
        let mut ids: Vec<_> = self.tabs.lock().unwrap().iter().map(|t| t.window).collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }
    fn safari_tabs(&self, _: Identity, w: i32, t: &Ticket) -> Result<Vec<Tab>, String> {
        self.read(t)?;
        Ok(self
            .tabs
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.window == w)
            .cloned()
            .collect())
    }
    fn safari_select(&self, _: Identity, tab: &Tab, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        *self.selected.lock().unwrap() = Some(tab.clone());
        self.effects.lock().unwrap().push("select-tab".into());
        let hook = self.after_safari_select.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        Ok(())
    }
    fn safari_selected(&self, _: Identity, tab: &Tab, t: &Ticket) -> Result<bool, String> {
        self.read(t)?;
        Ok(self.selected.lock().unwrap().as_ref() == Some(tab))
    }
    fn safari_front_window(&self, _: Identity, _: i32, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        self.effects.lock().unwrap().push("front-window".into());
        let hook = self.after_safari_front.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        Ok(())
    }
    fn shortcuts(&self, t: &Ticket) -> Result<Vec<Command>, String> {
        self.read(t)?;
        if self.shortcuts_failure.load(Ordering::Acquire) {
            return Err("Could not load shortcuts. Check Shortcuts, then refresh.".into());
        }
        Ok(self.shortcuts.lock().unwrap().clone())
    }
    fn run_shortcut(&self, id: &str, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        let id = apple_shortcuts::identifier(id).ok_or(apple_shortcuts::FAILURE)?;
        if !self
            .shortcuts
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.arguments[1] == id)
        {
            return Err(apple_shortcuts::FAILURE.into());
        }
        self.effects.lock().unwrap().push(format!("shortcut:{id}"));
        Ok(())
    }
    fn safari_consent(&self, t: &Ticket) -> Result<(), String> {
        self.read(t)?;
        self.effects.lock().unwrap().push("consent".into());
        Ok(())
    }
}
