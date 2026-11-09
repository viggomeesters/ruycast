use super::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
enum Target {
    Kill { identity:Identity },
    Window {
        identity: Identity,
        node: Element,
        info: WindowInfo,
    },
    Menu {
        identity: Identity,
        node: Element,
        application: Element,
        chain: Vec<Element>,
        ancestors: Vec<MenuNode>,
        info: MenuInfo,
    },
    Tab {
        browser: Identity,
        tab: Tab,
        captured: Instant,
    },
}
#[derive(Default)]
struct Store {
    targets: HashMap<String, Target>,
    order: VecDeque<String>,
}
pub struct Provider {
    process_cache:Mutex<Option<(Instant,Vec<crate::processes::Resource>)>>,
    pub platform: Arc<dyn Platform>,
    namespace: &'static str,
    targets: Mutex<Store>,
    serial: AtomicU64,
    shortcut_cache: Mutex<Option<(Instant, Vec<Command>)>>,
    tab_cache: Mutex<Option<(Identity, Instant, Results)>>,
}
impl Provider {
    pub fn new(platform: Arc<dyn Platform>, root: bool) -> Self {
        Self {
            process_cache:Default::default(),
            platform,
            namespace: if root { "root." } else { "" },
            targets: Default::default(),
            serial: AtomicU64::new(0),
            shortcut_cache: Default::default(),
            tab_cache: Default::default(),
        }
    }
    pub fn refresh(&self) {
        *self.process_cache.lock().unwrap_or_else(|p|p.into_inner())=None;
        *self
            .shortcut_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        *self.tab_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    fn remember(&self, kind: &str, target: Target) -> Result<String, String> {
        let n = self
            .serial
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| EXPIRED)?;
        // Unpredictable process nonce + non-reusable monotonic instance token.
        static NONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let nonce = NONCE.get_or_init(|| {
            objc2_foundation::NSUUID::UUID()
                .UUIDString()
                .to_string()
                .to_lowercase()
        });
        let token = format!("system.{kind}:{}{nonce}.{n}", self.namespace);
        let mut store = self.targets.lock().unwrap_or_else(|p| p.into_inner());
        if store.order.len() >= 2048 {
            if let Some(old) = store.order.pop_front() {
                store.targets.remove(&old);
            }
        }
        store.order.push_back(token.clone());
        store.targets.insert(token.clone(), target);
        Ok(token)
    }
    pub fn recognizes(&self, token: &str) -> bool {
        self.targets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .targets
            .contains_key(token)
    }
    pub fn search(
        &self,
        scope: Scope,
        query: &str,
        previous: Option<Application>,
        ticket: &Ticket,
    ) -> Result<Results, String> {
        ticket.check()?;
        if query.len() > 8192 || query.contains('\0') {
            return Err("Search text is too long or invalid.".into());
        }
        let result = match scope {
            Scope::Files=>Ok(Results{commands:self.platform.files(query,ticket)?,..Default::default()}),
            Scope::Processes=>self.processes(query,ticket),
            Scope::MainTabs | Scope::Tabs => self.tabs(query, ticket),
            Scope::Windows => self.windows(query, ticket),
            Scope::Menus => self.menus(
                query,
                previous.ok_or("Open an app before searching its menu.")?,
                ticket,
            ),
            Scope::Shortcuts => {
                let cached = self
                    .shortcut_cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                let commands = if let Some((time, commands)) =
                    cached.filter(|(time, _)| time.elapsed() < Duration::from_secs(30))
                {
                    let _ = time;
                    commands
                } else {
                    let commands = self.platform.shortcuts(ticket)?;
                    ticket.check()?;
                    *self
                        .shortcut_cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) =
                        Some((Instant::now(), commands.clone()));
                    commands
                };
                Ok(Results {
                    commands,
                    ..Default::default()
                })
            }
        }?;
        ticket.check()?;
        Ok(result)
    }
    fn check_identity(&self, id: Identity, t: &Ticket) -> Result<(), String> {
        t.check()?;
        if self.platform.identity(id.pid) == Some(id) {
            Ok(())
        } else {
            Err(EXPIRED.into())
        }
    }
    fn processes(&self,query:&str,ticket:&Ticket)->Result<Results,String>{
        let cached=self.process_cache.lock().unwrap_or_else(|p|p.into_inner()).clone();
        let samples=if let Some((_,rows))=cached.filter(|(at,_)|at.elapsed()<Duration::from_millis(1500)){rows}else{let rows=self.platform.processes(ticket)?;ticket.check()?;*self.process_cache.lock().unwrap_or_else(|p|p.into_inner())=Some((Instant::now(),rows.clone()));rows};
        let mut samples=samples.into_iter().filter(|r|crate::processes::matches(r,query)).collect::<Vec<_>>();crate::processes::sort(&mut samples,crate::processes::Sort::Cpu);samples.truncate(1000);
        let mut results=Results::default();
        for process in samples{
            ticket.check()?;
            let id=process.identity;
            let existing=self.targets.lock().unwrap_or_else(|p|p.into_inner()).targets.iter().find_map(|(token,target)|matches!(target,Target::Kill{identity} if *identity==id).then(||token.clone()));
            let token=if let Some(token)=existing{token}else{self.remember("kill",Target::Kill{identity:id})?};
            results.commands.push(command(token.clone(),process.name.clone(),None));results.metadata.insert(token,Metadata::Process{process});
        }
        Ok(results)
    }
    fn trusted(&self) -> Result<(), String> {
        if self.platform.trusted() {
            Ok(())
        } else {
            Err("Allow Ruycast in System Settings → Privacy & Security → Accessibility, then refresh.".into())
        }
    }
    fn windows(&self, q: &str, t: &Ticket) -> Result<Results, String> {
        self.trusted()?;
        let p = &self.platform;
        let deadline = Instant::now() + Duration::from_secs(2);
        let available = || -> Result<bool, String> {
            t.check()?;
            Ok(Instant::now() < deadline)
        };
        let mut out = Results::default();
        for app in p.applications(t)?.into_iter().take(200) {
            if !available()? || out.commands.len() >= 100 {
                break;
            }
            let id = app.identity;
            if id.pid <= 1
                || id.pid == unsafe { libc::getpid() }
                || id.uid != unsafe { libc::getuid() }
                || p.identity(id.pid) != Some(id)
            {
                continue;
            }
            let application = p.application(id.pid, t)?;
            if !available()? {
                break;
            }
            if p.pid(application) != Some(id.pid) {
                continue;
            }
            let windows = match p.elements(application, "AXWindows", 50, false, t) {
                Ok(v) => v,
                Err(_) => {
                    t.check()?;
                    continue;
                }
            };
            for node in windows {
                if !available()? {
                    break;
                }
                if out.commands.len() >= 100 {
                    return Ok(out);
                }
                if p.pid(node) != Some(id.pid)
                    || p.string(node, "AXRole", t).ok().flatten().as_deref() != Some("AXWindow")
                {
                    t.check()?;
                    continue;
                }
                if !available()? {
                    break;
                }
                let Some(title) = p.string(node, "AXTitle", t).ok().flatten() else {
                    t.check()?;
                    continue;
                };
                if !available()? {
                    break;
                }
                if !matches(&(app.name.clone() + " " + &title), q) {
                    continue;
                }
                let minimized = p.boolean(node, "AXMinimized", t)?;
                if !available()? {
                    break;
                }
                self.check_identity(id, t)?;
                let info = WindowInfo {
                    application: app.name.clone(),
                    title: title.clone(),
                    minimized,
                };
                let token = self.remember(
                    "window",
                    Target::Window {
                        identity: id,
                        node,
                        info: info.clone(),
                    },
                )?;
                out.metadata
                    .insert(token.clone(), Metadata::Window { window: info });
                out.commands.push(command(
                    token,
                    if title.is_empty() {
                        "Untitled Window".into()
                    } else {
                        title
                    },
                    None,
                ));
            }
        }
        Ok(out)
    }
    fn read_menu(&self, node: Element, t: &Ticket) -> Result<MenuNode, String> {
        let p = &self.platform;
        Ok(MenuNode {
            element: node,
            role: p.string(node, "AXRole", t)?.unwrap_or_default(),
            title: p.string(node, "AXTitle", t)?.unwrap_or_default(),
            enabled: p.boolean(node, "AXEnabled", t)?,
            character: p.string(node, "AXMenuItemCmdChar", t)?,
            glyph: p.integer(node, "AXMenuItemCmdGlyph", t)?,
            virtual_key: p.integer(node, "AXMenuItemCmdVirtualKey", t)?,
            modifiers: p.integer(node, "AXMenuItemCmdModifiers", t)?,
        })
    }
    fn menus(&self, q: &str, app: Application, t: &Ticket) -> Result<Results, String> {
        self.trusted()?;
        self.check_identity(app.identity, t)?;
        if app.identity.pid == unsafe { libc::getpid() } {
            return Err("Open an app before searching its menu.".into());
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let available = || -> Result<bool, String> {
            t.check()?;
            Ok(Instant::now() < deadline)
        };
        let p = &self.platform;
        let application = p.application(app.identity.pid, t)?;
        let menu = p
            .element(application, "AXMenuBar", t)?
            .ok_or("This app does not expose a menu bar.")?;
        let mut out = Results::default();
        let mut stack = vec![(menu, vec![], vec![], true, 0usize)];
        let mut visited = 0;
        while let Some((element, ancestors, mut chain, parent_enabled, depth)) = stack.pop() {
            if !available()? {
                break;
            }
            if depth >= 12 || visited >= 1200 || out.commands.len() >= 100 {
                continue;
            }
            visited += 1;
            // Exhausting the child-list budget cannot establish an empty list.
            // In particular it must not turn the last submenu into a leaf.
            if visited >= 1200 {
                break;
            }
            if p.pid(element) != Some(app.identity.pid) {
                continue;
            }
            chain.push(element);
            let node = match (|| -> Result<Option<MenuNode>, String> {
                let role = p.string(element, "AXRole", t)?.unwrap_or_default();
                if !available()? {
                    return Ok(None);
                }
                let title = p.string(element, "AXTitle", t)?.unwrap_or_default();
                if !available()? {
                    return Ok(None);
                }
                let enabled = p.boolean(element, "AXEnabled", t)?;
                if !available()? {
                    return Ok(None);
                }
                Ok(Some(MenuNode {
                    element,
                    role,
                    title,
                    enabled,
                    character: None,
                    glyph: None,
                    virtual_key: None,
                    modifiers: None,
                }))
            })() {
                Ok(Some(v)) => v,
                Ok(None) => break,
                Err(_) => {
                    t.check()?;
                    continue;
                }
            };
            let named = !node.title.is_empty()
                && ["AXMenuItem", "AXMenuBarItem"].contains(&node.role.as_str());
            let enabled = parent_enabled
                && if node.role == "AXMenuItem" {
                    node.enabled == Some(true)
                } else {
                    node.enabled != Some(false)
                };
            let mut path: Vec<MenuNode> = ancestors;
            if named {
                path.push(node.clone());
            }
            let kids = match p.elements(element, "AXChildren", 1200 - visited, true, t) {
                Ok(v) => v,
                Err(_) => {
                    t.check()?;
                    continue;
                }
            };
            if !available()? {
                break;
            }
            if named && kids.is_empty() && node.role == "AXMenuItem" {
                let breadcrumb = path
                    .iter()
                    .take(path.len() - 1)
                    .map(|n| n.title.as_str())
                    .collect::<Vec<_>>()
                    .join(" › ");
                if matches(&format!("{} {} {}", app.name, breadcrumb, node.title), q) {
                    // Only a matching, verified leaf needs keyboard metadata.
                    // Nonmatching/container nodes consume three attributes.
                    let leaf = match (|| -> Result<Option<MenuNode>, String> {
                        let mut leaf = node.clone();
                        leaf.character = p.string(element, "AXMenuItemCmdChar", t)?;
                        if !available()? {
                            return Ok(None);
                        }
                        leaf.glyph = p.integer(element, "AXMenuItemCmdGlyph", t)?;
                        if !available()? {
                            return Ok(None);
                        }
                        leaf.virtual_key = p.integer(element, "AXMenuItemCmdVirtualKey", t)?;
                        if !available()? {
                            return Ok(None);
                        }
                        leaf.modifiers = p.integer(element, "AXMenuItemCmdModifiers", t)?;
                        if !available()? {
                            return Ok(None);
                        }
                        Ok(Some(leaf))
                    })() {
                        Ok(Some(leaf)) => leaf,
                        Ok(None) => break,
                        Err(_) => {
                            t.check()?;
                            continue;
                        }
                    };
                    *path.last_mut().ok_or("menu leaf path missing")? = leaf.clone();
                    let info = MenuInfo {
                        application: app.name.clone(),
                        path: breadcrumb,
                        title: node.title.clone(),
                        shortcut: shortcut(&leaf),
                        enabled,
                    };
                    let token = self.remember(
                        "menu",
                        Target::Menu {
                            identity: app.identity,
                            node: element,
                            application,
                            chain: chain.clone(),
                            ancestors: path.clone(),
                            info: info.clone(),
                        },
                    )?;
                    out.metadata
                        .insert(token.clone(), Metadata::Menu { menu: info });
                    out.commands.push(command(token, node.title, Some(enabled)));
                }
            }
            for child in kids.into_iter().rev() {
                stack.push((child, path.clone(), chain.clone(), enabled, depth + 1));
            }
        }
        self.check_identity(app.identity, t)?;
        out.foreground = Some(app);
        Ok(out)
    }
    fn tabs(&self, q: &str, t: &Ticket) -> Result<Results, String> {
        let p = &self.platform;
        let browser = p.safari(t)?;
        // Match the shipped clock: discovery time consumes the cache/target
        // lifetime instead of granting a fresh lease after a slow response.
        let captured = Instant::now();
        if let Some((_, _, cached)) = self
            .tab_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|(id, time, _)| *id == browser && time.elapsed() < Duration::from_secs(1))
        {
            return filter_tabs(cached, q, t);
        }
        let windows = p.safari_windows(browser, t)?;
        if windows.len() > 64 {
            return Err("Too many Safari windows to load. The limit is 64.".into());
        }
        let mut seen = HashSet::new();
        let mut rows = vec![];
        let mut bytes = 0;
        for window in windows {
            self.check_identity(browser, t)?;
            if !seen.insert(window) {
                return Err(EXPIRED.into());
            }
            let tabs = p.safari_tabs(browser, window, t)?;
            if tabs
                .iter()
                .any(|tab| tab.title.len() > 8192 || tab.url.len() > 8192)
            {
                return Err("Could not read Safari tab metadata. Refresh to retry.".into());
            }
            bytes += tabs
                .iter()
                .map(|tab| tab.title.len() + tab.url.len())
                .sum::<usize>();
            rows.extend(tabs);
            if rows.len() > 512 || bytes > 2_097_152 {
                return Err("Too many Safari tabs to load. The limit is 512.".into());
            }
        }
        let mut out = Results::default();
        for tab in rows {
            t.check()?;
            let title = if tab.title.is_empty() {
                if tab.url.is_empty() {
                    "Untitled Tab".into()
                } else {
                    tab.url.clone()
                }
            } else {
                tab.title.clone()
            };
            let metadata = Metadata::Tab {
                tab: TabInfo {
                    url: tab.url.clone(),
                },
            };
            let token = self.remember(
                "tab",
                Target::Tab {
                    browser,
                    tab,
                    captured,
                },
            )?;
            out.metadata.insert(token.clone(), metadata);
            out.commands.push(command(token, title, None));
        }
        self.check_identity(browser, t)?;
        *self.tab_cache.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((browser, captured, out.clone()));
        filter_tabs(out, q, t)
    }
    pub fn execute(
        &self,
        token: &str,
        previous: Option<Identity>,
        t: &Ticket,
        activate: &Activate,
    ) -> Result<(), String> {
        t.check()?;
        let target = self
            .targets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .targets
            .get(token)
            .cloned()
            .ok_or(EXPIRED)?;
        let p = &self.platform;
        match target {
            Target::Kill{identity}=>{t.check()?;p.terminate(identity,t)?;*self.process_cache.lock().unwrap_or_else(|p|p.into_inner())=None;return Ok(());},
            Target::Window {
                identity,
                node,
                info,
            } => {
                let application = p.application(identity.pid, t)?;
                let validate = || -> Result<Option<bool>, String> {
                    self.trusted()?;
                    self.check_identity(identity, t)?;
                    if p.pid(node) != Some(identity.pid)
                        || p.pid(application) != Some(identity.pid)
                        || p.string(node, "AXRole", t)?.as_deref() != Some("AXWindow")
                        || p.string(node, "AXTitle", t)?.as_deref() != Some(&info.title)
                        || !p
                            .elements(application, "AXWindows", 50, false, t)?
                            .into_iter()
                            .any(|n| p.same(n, node))
                    {
                        return Err("This window changed or closed. Search again.".into());
                    }
                    self.check_identity(identity, t)?;
                    p.boolean(node, "AXMinimized", t)
                };
                validate()?;
                activate(identity, t)?;
                if validate()? == Some(true) {
                    p.restore(node, t)?;
                    if validate()? != Some(false) {
                        return Err("This window could not be restored. Search again.".into());
                    }
                }
                validate()?;
                p.action(node, "AXRaise", t)?;
            }
            Target::Menu {
                identity,
                node,
                application,
                chain,
                ancestors,
                info,
            } => {
                if previous != Some(identity) {
                    return Err("The original app changed. Search its menu again.".into());
                }
                if !info.enabled || ancestors.is_empty() {
                    return Err("This menu item is unavailable. Search its menu again.".into());
                }
                let validate = || -> Result<(), String> {
                    self.trusted()?;
                    self.check_identity(identity, t)?;
                    let root = p.element(application, "AXMenuBar", t)?.ok_or(EXPIRED)?;
                    if chain.first().is_none_or(|&saved| !p.same(saved, root)) {
                        return Err(EXPIRED.into());
                    }
                    for edge in chain.windows(2) {
                        if p.pid(edge[0]) != Some(identity.pid)
                            || p.pid(edge[1]) != Some(identity.pid)
                            || !p
                                .elements(edge[0], "AXChildren", 1200, true, t)?
                                .into_iter()
                                .any(|n| p.same(n, edge[1]))
                        {
                            return Err("This menu path changed. Search its menu again.".into());
                        }
                    }
                    for saved in &ancestors {
                        if p.pid(saved.element) != Some(identity.pid) {
                            return Err(EXPIRED.into());
                        }
                        let current = self.read_menu(saved.element, t)?;
                        let enabled = if current.role == "AXMenuBarItem" {
                            current.enabled != Some(false)
                        } else {
                            current.enabled == Some(true)
                        };
                        // The leaf shortcut can change this action's meaning;
                        // ancestors establish role/title/enabled/path identity.
                        if current.role != saved.role
                            || current.title != saved.title
                            || !enabled
                            || (p.same(saved.element, node)
                                && shortcut(&current) != shortcut(saved))
                        {
                            return Err(
                                "This menu item changed or is unavailable. Search its menu again."
                                    .into(),
                            );
                        }
                    }
                    if !p.elements(node, "AXChildren", 1, true, t)?.is_empty() {
                        return Err(EXPIRED.into());
                    }
                    self.check_identity(identity, t)
                };
                validate()?;
                activate(identity, t)?;
                validate()?;
                p.action(node, "AXPress", t)?;
            }
            Target::Tab {
                browser,
                tab,
                captured,
            } => {
                if captured.elapsed() >= Duration::from_secs(30) {
                    return Err("This tab changed or closed. Search Safari tabs again.".into());
                }
                let validate = || -> Result<(), String> {
                    self.check_identity(browser, t)?;
                    if p.safari(t)? != browser {
                        return Err(EXPIRED.into());
                    }
                    let current = p.safari_tabs(browser, tab.window, t)?;
                    if !current.contains(&tab) {
                        return Err("This tab changed or closed. Search Safari tabs again.".into());
                    }
                    if current
                        .iter()
                        .filter(|x| x.title == tab.title && x.url == tab.url)
                        .count()
                        != 1
                    {
                        return Err(
                            "These tabs have the same title and address. Select one in Safari."
                                .into(),
                        );
                    }
                    Ok(())
                };
                let selected = || -> Result<(), String> {
                    if !p.safari_selected(browser, &tab, t)? {
                        return Err(EXPIRED.into());
                    }
                    validate()
                };
                validate()?;
                p.safari_select(browser, &tab, t)?;
                selected()?;
                p.safari_front_window(browser, tab.window, t)?;
                selected()?;
                activate(browser, t)?;
                selected()?;
            }
        }
        t.check()
    }
}

pub fn shortcut(node: &MenuNode) -> Option<Shortcut> {
    let modifiers = node.modifiers.filter(|m| (0..=15).contains(m))?;
    let glyph = match node.glyph {
        Some(2) => Some("⇥".into()),
        Some(3) => Some("⇤".into()),
        Some(4) => Some("⌤".into()),
        Some(9) => Some("Space".into()),
        Some(10) => Some("⌦".into()),
        Some(11..=13) => Some("↵".into()),
        Some(23) => Some("⌫".into()),
        Some(27) => Some("⎋".into()),
        Some(28) => Some("Clear".into()),
        Some(98) => Some("⇞".into()),
        Some(100) => Some("←".into()),
        Some(101) => Some("→".into()),
        Some(102) => Some("↖".into()),
        Some(103) => Some("Help".into()),
        Some(104) => Some("↑".into()),
        Some(105) => Some("↘".into()),
        Some(106) => Some("↓".into()),
        Some(107) => Some("⇟".into()),
        Some(n @ 111..=122) => Some(format!("F{}", n - 110)),
        Some(n @ 135..=137) => Some(format!("F{}", n - 122)),
        _ => None,
    };
    let special = || {
        match node.virtual_key {
            Some(36) => Some("↵"),
            Some(48) => Some("⇥"),
            Some(49) => Some("Space"),
            Some(51) => Some("⌫"),
            Some(53) => Some("⎋"),
            Some(76) => Some("⌤"),
            Some(115) => Some("↖"),
            Some(116) => Some("⇞"),
            Some(117) => Some("⌦"),
            Some(119) => Some("↘"),
            Some(121) => Some("⇟"),
            Some(123) => Some("←"),
            Some(124) => Some("→"),
            Some(125) => Some("↓"),
            Some(126) => Some("↑"),
            _ => None,
        }
        .map(str::to_owned)
        .or_else(|| {
            [
                122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79,
                80, 90,
            ]
            .iter()
            .position(|&x| Some(x) == node.virtual_key)
            .map(|i| format!("F{}", i + 1))
        })
    };
    let key = glyph
        .or_else(|| {
            node.character
                .as_ref()
                .filter(|s| {
                    unicode_segmentation::UnicodeSegmentation::graphemes(s.as_str(), true).count()
                        == 1
                })
                .and_then(|s| {
                    Some(match s.as_str() {
                        "\r" | "\n" => "↵".into(),
                        "\t" => "⇥".into(),
                        "\u{8}" | "\u{7f}" => "⌫".into(),
                        "\u{1b}" => "⎋".into(),
                        " " => "Space".into(),
                        "\u{f700}" => "↑".into(),
                        "\u{f701}" => "↓".into(),
                        "\u{f702}" => "←".into(),
                        "\u{f703}" => "→".into(),
                        _ => {
                            let scalar = s.chars().next()? as u32;
                            if (0xf704..=0xf726).contains(&scalar) {
                                format!("F{}", scalar - 0xf703)
                            } else if s
                                .chars()
                                .any(|c| c.is_control() || (0xe000..=0xf8ff).contains(&(c as u32)))
                            {
                                return special();
                            } else {
                                let upper = s.to_uppercase();
                                if unicode_segmentation::UnicodeSegmentation::graphemes(
                                    upper.as_str(),
                                    true,
                                )
                                .count()
                                    == 1
                                {
                                    upper
                                } else {
                                    s.clone()
                                }
                            }
                        }
                    })
                })
        })
        .or_else(special)?;
    let mut keys = vec![];
    let mut names = vec![];
    for (bit, symbol, name) in [(4, "⌃", "Control"), (2, "⌥", "Option"), (1, "⇧", "Shift")] {
        if modifiers & bit != 0 {
            keys.push(symbol.into());
            names.push(name);
        }
    }
    if modifiers & 8 == 0 {
        keys.push("⌘".into());
        names.push("Command");
    }
    let spoken = match key.as_str() {
        "⇥" => "Tab",
        "⇤" => "Back Tab",
        "⌤" => "Enter",
        "⌦" => "Forward Delete",
        "↵" => "Return",
        "⌫" => "Delete",
        "⎋" => "Escape",
        "⇞" => "Page Up",
        "←" => "Left Arrow",
        "→" => "Right Arrow",
        "↖" => "Home",
        "↑" => "Up Arrow",
        "↘" => "End",
        "↓" => "Down Arrow",
        "⇟" => "Page Down",
        _ => &key,
    };
    let label = names
        .into_iter()
        .chain(std::iter::once(spoken))
        .collect::<Vec<_>>()
        .join(" + ");
    keys.push(key);
    Some(Shortcut { keys, label })
}

fn filter_tabs(mut result: Results, query: &str, t: &Ticket) -> Result<Results, String> {
    let mut commands = vec![];
    for c in result.commands {
        t.check()?;
        let url = match result.metadata.get(&c.id) {
            Some(Metadata::Tab { tab }) => tab.url.as_str(),
            _ => "",
        };
        if matches_words(&format!("{} {}", c.title, url), query) {
            commands.push(c);
        }
    }
    result.commands = commands;
    t.check()?;
    Ok(result)
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    fn ticket() -> Ticket {
        Ticket {
            deadline: Instant::now() + Duration::from_secs(2),
            live: Arc::new(|| true),
        }
    }
    #[test]
    fn safari_thirty_second_target_expiry_refuses_before_any_native_read_or_effect() {
        for scope in [Scope::Tabs, Scope::MainTabs] {
            let fixture = Arc::new(crate::system_search::fixture::Fixture::default());
            let provider = Provider::new(fixture.clone(), scope == Scope::MainTabs);
            let rows = provider.search(scope, "", None, &ticket()).unwrap();
            {
                let mut store = provider.targets.lock().unwrap();
                let Target::Tab { captured, .. } =
                    store.targets.get_mut(&rows.commands[0].id).unwrap()
                else {
                    panic!("expected tab target")
                };
                *captured = Instant::now() - Duration::from_secs(30);
            }
            let reads = fixture.reads.load(Ordering::Acquire);
            assert!(provider
                .execute(&rows.commands[0].id, None, &ticket(), &fixture.activate())
                .unwrap_err()
                .contains("changed or closed"));
            assert_eq!(fixture.reads.load(Ordering::Acquire), reads);
            assert!(fixture.effects.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn shortcut_discovery_cache_hit_thirty_one_second_expiry_and_explicit_refresh() {
        let fixture=Arc::new(crate::system_search::fixture::Fixture::default());
        let provider=Provider::new(fixture.clone(),false);
        let first=provider.search(Scope::Shortcuts,"",None,&ticket()).unwrap();
        assert_eq!(first.commands.len(),2);
        let reads=fixture.reads.load(Ordering::Acquire);
        let replacement=crate::system_search::apple_shortcuts::parse(b"Owned replacement (33333333-3333-4333-8333-333333333333)\n").unwrap();
        *fixture.shortcuts.lock().unwrap()=replacement.clone();
        let cached=provider.search(Scope::Shortcuts,"",None,&ticket()).unwrap();
        assert_eq!(serde_json::to_value(&cached.commands).unwrap(),serde_json::to_value(&first.commands).unwrap());
        assert_eq!(fixture.reads.load(Ordering::Acquire),reads,"cache hit performed discovery");
        provider.shortcut_cache.lock().unwrap().as_mut().unwrap().0=Instant::now()-Duration::from_secs(31);
        assert_eq!(serde_json::to_value(provider.search(Scope::Shortcuts,"",None,&ticket()).unwrap().commands).unwrap(),serde_json::to_value(&replacement).unwrap());
        assert_eq!(fixture.reads.load(Ordering::Acquire),reads+1);
        *fixture.shortcuts.lock().unwrap()=first.commands.clone();
        provider.refresh();
        assert_eq!(serde_json::to_value(provider.search(Scope::Shortcuts,"",None,&ticket()).unwrap().commands).unwrap(),serde_json::to_value(&first.commands).unwrap());
        assert_eq!(fixture.reads.load(Ordering::Acquire),reads+2);
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
    fn processes() -> Arc<crate::system_search::fixture::Fixture> {
        let fixture = Arc::new(crate::system_search::fixture::Fixture::default());
        *fixture.process_fixture.rows.lock().unwrap() = (0..2100)
            .map(|i| crate::processes::Resource {
                identity: Identity {
                    pid: 200000 + i,
                    uid: unsafe { libc::getuid() },
                    seconds: i as u64,
                    micros: 0,
                },
                name: format!("Owned row {i:04}"),
                path: format!("/owned/Owned row {i:04}"),
                cpu_percent: i as f64,
                memory_bytes: 1,
            })
            .collect();
        fixture
    }
    #[test]
    fn process_result_cap_bounds_opaque_target_registration() {
        let provider = Provider::new(processes(), false);
        let rows = provider
            .search(Scope::Processes, "", None, &ticket())
            .unwrap();
        assert_eq!(rows.commands.len(), 1000);
        assert_eq!(rows.metadata.len(), 1000);
        assert!(
            matches!(rows.metadata.get(&rows.commands[0].id),Some(Metadata::Process{process}) if process.cpu_percent==2099.)
        );
        let store = provider.targets.lock().unwrap();
        assert_eq!(store.targets.len(), 1000);
        assert_eq!(store.order.len(), 1000);
    }
    #[test]
    fn cross_kind_and_missing_opaque_tokens_refuse_before_native_reads_or_effects() {
        let fixture = Arc::new(crate::system_search::fixture::Fixture::default());
        let provider = Provider::new(fixture.clone(), false);
        let rows = provider.search(Scope::Processes, "", None, &ticket()).unwrap();
        let kill = &rows.commands[0].id;
        let suffix = kill.strip_prefix("system.kill:").expect("captured process target");
        assert!(provider.recognizes(kill));
        let reads = fixture.reads.load(Ordering::Acquire);
        for rejected in [format!("system.menu:{suffix}"), "system.window:missing".into()] {
            assert!(!provider.recognizes(&rejected));
            assert_eq!(provider.execute(&rejected, None, &ticket(), &fixture.activate()).unwrap_err(), EXPIRED);
            assert_eq!(fixture.reads.load(Ordering::Acquire), reads);
            assert!(fixture.effects.lock().unwrap().is_empty());
            assert!(fixture.process_fixture.kills.lock().unwrap().is_empty());
            assert!(provider.recognizes(kill));
        }
    }
    #[test]
    fn concurrent_process_queries_keep_each_result_and_metadata_snapshot() {
        let provider = Arc::new(Provider::new(processes(), false));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            for query in ["Owned row 0001", "Owned row 2099"] {
                let provider = provider.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let rows = provider
                        .search(Scope::Processes, query, None, &ticket())
                        .unwrap();
                    assert_eq!(rows.commands.len(), 1);
                    assert_eq!(rows.metadata.len(), 1);
                    let Metadata::Process { process } =
                        rows.metadata.get(&rows.commands[0].id).unwrap()
                    else {
                        panic!("expected process metadata")
                    };
                    assert_eq!(process.name, query);
                    assert_eq!(rows.commands[0].title, query);
                    assert_eq!(
                        process.identity.pid,
                        if query.ends_with("0001") {
                            200001
                        } else {
                            202099
                        }
                    );
                });
            }
        });
    }
}
