//! Configuration-derived registration plan and transactional native ownership.
//! Only canonical bindings/action IDs cross this boundary, never command text.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub action: String,
    pub code: u32,
    pub modifiers: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub carbon: BTreeMap<String, Binding>,
    pub sided: BTreeMap<String, String>,
    pub modifier_taps: BTreeMap<u32, String>,
    pub hyper: bool,
}
impl Plan {
    pub fn from_config(config: &rustcast_core::Config) -> Result<Self, String> {
        let mut desired = BTreeMap::new();
        desired.insert(
            rustcast_core::shortcut(&config.launcher_hotkey)?,
            "@launcher".into(),
        );
        desired.insert(
            rustcast_core::shortcut(&config.clipboard_hotkey)?,
            "@clipboard".into(),
        );
        for command in config.commands.iter().filter(|c| c.enabled != Some(false)) {
            if let Some(key) = &command.hotkey {
                desired.insert(rustcast_core::shortcut(key)?, command.id.clone());
            }
        }
        let mut plan = Self {
            hyper: config.hyper_key_enabled,
            ..Self::default()
        };
        for (canonical, action) in desired {
            let mut parts = canonical.split('+').collect::<Vec<_>>();
            let key = parts.pop().expect("validated shortcut has a key");
            if parts.iter().any(|m| ["left-cmd", "right-cmd"].contains(m)) {
                if config.separate_command_keys.unwrap_or(true) {
                    plan.sided.insert(canonical, action);
                }
                continue;
            }
            if ["left-control", "left-option"].contains(&key) {
                let expected = if key == "left-control" { "ctrl" } else { "alt" };
                if parts != [expected] {
                    return Err(
                        "Use Ctrl+Left-Control or Alt+Left-Option for modifier taps.".into(),
                    );
                }
                plan.modifier_taps
                    .insert(if key == "left-control" { 0xe0 } else { 0xe2 }, action);
                continue;
            }
            let code = (0..=127)
                .find(|code| {
                    crate::recorder::key_name(*code)
                        == Some(if key == "backspace" { "delete" } else { key })
                })
                .ok_or_else(|| format!("Unsupported hotkey key: {key}"))?;
            let modifiers = parts.iter().fold(0, |flags, m| {
                flags
                    | match *m {
                        "cmd" => 1 << 8,
                        "shift" => 1 << 9,
                        "alt" => 1 << 11,
                        "ctrl" => 1 << 12,
                        _ => 0,
                    }
            });
            plan.carbon.insert(
                canonical,
                Binding {
                    action,
                    code: u32::from(code),
                    modifiers,
                },
            );
        }
        Ok(plan)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Press {
    pub generation: u64,
    pub canonical: String,
    pub action: String,
}
/// One bounded callback message. No downstream owner/native calls occur in
/// the Carbon callback; these clocks reject queued events across recording/UI.
#[derive(Clone, Debug)]
pub struct RoutedPress {
    pub hook_epoch: Option<u64>,
    pub press: Press,
    pub received: std::time::Instant,
    pub recording_epoch: u64,
    pub ui_epoch: u64,
    pub ui_intent: u64,
    pub ui_dismissal: u64,
}
pub trait Platform {
    type Handle;
    fn register(&mut self, code: u32, modifiers: u32, id: u32) -> Result<Self::Handle, i32>;
    fn unregister(&mut self, handle: Self::Handle) -> Result<(), (Self::Handle, i32)>;
    /// Preserve ownership if the final teardown retry cannot retire a reference.
    fn quarantine(&mut self, handle: Self::Handle);
    /// Called before releasing obsolete native references. Native callbacks
    /// must snapshot these routes without borrowing the registry across SDK calls.
    fn routes(&mut self, routes: BTreeMap<u32, Press>);
    fn cleanup(&mut self) -> bool {
        true
    }
}
struct Registration<H> {
    id: u32,
    handle: H,
}
struct Staged<H> {
    generation: u64,
    plan: Plan,
    added: BTreeMap<String, Registration<H>>,
}
pub struct Registry<P: Platform> {
    platform: P,
    active: BTreeMap<String, Registration<P::Handle>>,
    pending: Option<Staged<P::Handle>>,
    retired: Vec<P::Handle>,
    next_id: u32,
    next_generation: u64,
    current: Plan,
    current_generation: u64,
    suspended: bool,
    warnings: Vec<String>,
}
impl<P: Platform> Registry<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            active: BTreeMap::new(),
            pending: None,
            retired: vec![],
            next_id: 1,
            next_generation: 1,
            current: Plan::default(),
            current_generation: 0,
            suspended: false,
            warnings: vec![],
        }
    }
    pub fn generation(&self) -> u64 {
        self.current_generation
    }
    pub fn current(&self) -> &Plan {
        &self.current
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn suspension_clean(&mut self) -> bool {
        self.suspended
            && self.active.is_empty()
            && self.retired.is_empty()
            && self.pending.as_ref().is_none_or(|p| p.added.is_empty())
            && self.platform.cleanup()
    }
    fn release(&mut self, handle: P::Handle) {
        if let Err((handle, status)) = self.platform.unregister(handle) {
            self.retired.push(handle);
            self.warnings.push(format!(
                "Could not release a hotkey ({status}). Reload configuration to retry."
            ));
        }
    }
    fn retry_retired(&mut self) {
        let handles = std::mem::take(&mut self.retired);
        for handle in handles {
            self.release(handle);
        }
    }
    pub fn stage(&mut self, plan: Plan) -> Result<u64, String> {
        self.stage_if(plan, || true)
    }
    pub fn stage_if(&mut self, plan: Plan, live: impl Fn() -> bool) -> Result<u64, String> {
        if self.pending.is_some() {
            return Err("A keyboard configuration is already pending.".into());
        }
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .ok_or("Keyboard generation exhausted.")?;
        self.warnings.clear();
        self.retry_retired();
        if !self.warnings.is_empty() {
            return Err(self.warnings.join(" "));
        }
        let mut added: BTreeMap<String, Registration<P::Handle>> = BTreeMap::new();
        if !self.suspended {
            for (key, binding) in &plan.carbon {
                if !live() {
                    for reg in added.into_values() {
                        self.release(reg.handle);
                    }
                    return Err("Keyboard configuration was cancelled.".into());
                }
                if self.active.contains_key(key) {
                    continue;
                }
                let Some(next) = self.next_id.checked_add(1) else {
                    for reg in added.into_values() {
                        let Registration { handle, .. } = reg;
                        self.release(handle);
                    }
                    return Err("Hotkey registration sequence exhausted.".into());
                };
                let id = self.next_id;
                self.next_id = next;
                match self.platform.register(binding.code, binding.modifiers, id) {
                    Ok(handle) => {
                        added.insert(key.clone(), Registration { id, handle });
                    }
                    Err(status) => self.warnings.push(format!(
                        "Hotkey {key} is unavailable ({status}); choose another binding."
                    )),
                }
            }
        }
        if !live() {
            for reg in added.into_values() {
                self.release(reg.handle);
            }
            return Err("Keyboard configuration was cancelled.".into());
        }
        if !self.warnings.is_empty() {
            for reg in added.into_values() {
                self.release(reg.handle);
            }
            return Err(self.warnings.join(" "));
        }
        self.pending = Some(Staged {
            generation,
            plan,
            added,
        });
        Ok(generation)
    }
    pub fn rollback(&mut self) {
        if let Some(staged) = self.pending.take() {
            for reg in staged.added.into_values() {
                self.release(reg.handle);
            }
        }
    }
    pub fn commit(&mut self) {
        let Some(staged) = self.pending.take() else {
            return;
        };
        self.active.extend(staged.added);
        let routes = if self.suspended {
            BTreeMap::new()
        } else {
            staged
                .plan
                .carbon
                .iter()
                .filter_map(|(key, b)| {
                    self.active.get(key).map(|reg| {
                        (
                            reg.id,
                            Press {
                                generation: staged.generation,
                                canonical: key.clone(),
                                action: b.action.clone(),
                            },
                        )
                    })
                })
                .collect()
        };
        self.platform.routes(routes);
        let old = std::mem::take(&mut self.active);
        for (key, reg) in old {
            if !self.suspended && staged.plan.carbon.contains_key(&key) {
                self.active.insert(key, reg);
            } else {
                self.release(reg.handle);
            }
        }
        self.current_generation = staged.generation;
        self.current = staged.plan;
    }
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.platform.routes(BTreeMap::new());
        self.retry_retired();
        for reg in std::mem::take(&mut self.active).into_values() {
            self.release(reg.handle);
        }
        if let Some(pending) = &mut self.pending {
            let added = std::mem::take(&mut pending.added);
            for reg in added.into_values() {
                self.release(reg.handle);
            }
        }
    }
    pub fn resume(&mut self) -> Result<(), String> {
        self.resume_if(|| true)
    }
    pub fn resume_if(&mut self, live: impl Fn() -> bool) -> Result<(), String> {
        if !self.suspended {
            return Ok(());
        }
        if self.pending.is_some() {
            return Err("Keyboard configuration is pending.".into());
        }
        self.suspended = false;
        match self.stage_if(self.current.clone(), live) {
            Ok(_) => {
                if let Some(stage) = self.pending.as_mut() {
                    stage.generation = self.current_generation;
                }
                self.commit();
                Ok(())
            }
            Err(error) => {
                self.suspended = true;
                Err(error)
            }
        }
    }
}
impl<P: Platform> Drop for Registry<P> {
    fn drop(&mut self) {
        self.suspend();
        self.rollback();
        self.retry_retired();
        for handle in std::mem::take(&mut self.retired) {
            self.platform.quarantine(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[derive(Default)]
    struct State {
        trace: Vec<String>,
        live: BTreeMap<usize, (u32, u32, u32)>,
        routes: BTreeMap<u32, Press>,
        fail_code: Option<u32>,
        fail_release: bool,
    }
    struct Fake(Rc<RefCell<State>>);
    impl Platform for Fake {
        type Handle = usize;
        fn register(&mut self, code: u32, mods: u32, id: u32) -> Result<usize, i32> {
            let mut s = self.0.borrow_mut();
            s.trace.push(format!("register:{id}"));
            assert!(!s.routes.contains_key(&id), "staged IDs cannot dispatch");
            if s.fail_code == Some(code) || s.live.values().any(|&(c, m, _)| c == code && m == mods)
            {
                return Err(-9878);
            }
            s.live.insert(id as usize, (code, mods, id));
            Ok(id as usize)
        }
        fn unregister(&mut self, handle: usize) -> Result<(), (usize, i32)> {
            let mut s = self.0.borrow_mut();
            s.trace.push(format!("unregister:{handle}"));
            if s.fail_release {
                return Err((handle, -50));
            }
            assert!(s.live.remove(&handle).is_some());
            Ok(())
        }
        fn routes(&mut self, routes: BTreeMap<u32, Press>) {
            let mut s = self.0.borrow_mut();
            s.trace.push("routes".into());
            s.routes = routes;
        }
        fn quarantine(&mut self, handle: usize) {
            self.0
                .borrow_mut()
                .trace
                .push(format!("quarantine:{handle}"));
        }
    }
    fn plan(keys: &[(&str, u32, &str)]) -> Plan {
        Plan {
            carbon: keys
                .iter()
                .map(|(k, c, a)| {
                    (
                        k.to_string(),
                        Binding {
                            action: a.to_string(),
                            code: *c,
                            modifiers: 256,
                        },
                    )
                })
                .collect(),
            ..Plan::default()
        }
    }
    #[test]
    fn staged_additions_preserve_old_routes_until_commit_and_failures_rollback() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut r = Registry::new(Fake(state.clone()));
        let a = plan(&[("cmd+a", 0, "old")]);
        r.stage(a.clone()).unwrap();
        r.commit();
        let old = state.borrow().routes.clone();
        let b = plan(&[("cmd+a", 0, "new"), ("cmd+s", 1, "other")]);
        r.stage(b.clone()).unwrap();
        assert_eq!(state.borrow().routes, old);
        r.rollback();
        assert_eq!(state.borrow().routes, old);
        assert_eq!(state.borrow().live.len(), 1);
        state.borrow_mut().fail_code = Some(2);
        let c = plan(&[("cmd+s", 1, "stage"), ("cmd+d", 2, "conflict")]);
        assert!(r.stage(c).is_err());
        assert_eq!(r.current(), &a);
        assert_eq!(state.borrow().live.len(), 1);
        assert_eq!(state.borrow().routes, old);
        state.borrow_mut().fail_code = None;
        r.stage(b).unwrap();
        r.commit();
        let snapshot = state.borrow();
        assert_eq!(
            snapshot
                .routes
                .values()
                .find(|p| p.canonical == "cmd+a")
                .unwrap()
                .action,
            "new"
        );
        assert_eq!(snapshot.live.len(), 2);
        drop(snapshot);
        r.stage(plan(&[("cmd+s", 1, "remaining")])).unwrap();
        state.borrow_mut().trace.clear();
        r.commit();
        assert_eq!(state.borrow().trace[0], "routes");
        assert!(state.borrow().trace[1].starts_with("unregister:"));
        drop(r);
        assert!(state.borrow().live.is_empty());
        assert!(state.borrow().routes.is_empty());
    }
    #[test]
    fn suspend_cancels_physical_keys_and_resume_uses_latest_plan_without_new_config_generation() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut r = Registry::new(Fake(state.clone()));
        let gen = r.stage(plan(&[("cmd+a", 0, "first")])).unwrap();
        r.commit();
        r.suspend();
        assert!(state.borrow().live.is_empty());
        assert!(state.borrow().routes.is_empty());
        let next = plan(&[("cmd+s", 1, "latest")]);
        let gen2 = r.stage(next).unwrap();
        r.commit();
        assert!(gen2 > gen);
        assert!(state.borrow().live.is_empty());
        r.resume().unwrap();
        assert_eq!(state.borrow().live.len(), 1);
        let press = state.borrow().routes.values().next().unwrap().clone();
        assert_eq!(press.generation, gen2);
        assert_eq!(press.action, "latest");
        r.suspend();
        state.borrow_mut().fail_code = Some(1);
        assert!(r.resume().is_err());
        assert!(state.borrow().routes.is_empty());
        state.borrow_mut().fail_code = None;
        r.resume().unwrap();
        assert_eq!(
            state.borrow().routes.values().next().unwrap().generation,
            gen2
        );
    }
    #[test]
    fn suspension_between_stage_and_commit_releases_every_reference() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut r = Registry::new(Fake(state.clone()));
        r.stage(plan(&[("cmd+a", 0, "old")])).unwrap();
        r.commit();
        r.stage(plan(&[("cmd+s", 1, "new")])).unwrap();
        r.suspend();
        r.commit();
        assert!(state.borrow().live.is_empty());
        r.resume().unwrap();
        assert_eq!(state.borrow().live.len(), 1);
        assert_eq!(state.borrow().routes.values().next().unwrap().action, "new");
    }
    #[test]
    fn ids_never_wrap_and_failed_unregister_stays_owned_until_retry() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut r = Registry::new(Fake(state.clone()));
        r.stage(plan(&[("cmd+a", 0, "old")])).unwrap();
        r.commit();
        r.next_id = u32::MAX;
        assert!(r.stage(plan(&[("cmd+s", 1, "new")])).is_err());
        assert_eq!(state.borrow().live.len(), 1);
        r.next_id = 20;
        state.borrow_mut().fail_release = true;
        r.suspend();
        assert_eq!(r.retired.len(), 1);
        assert_eq!(state.borrow().live.len(), 1);
        state.borrow_mut().fail_release = false;
        r.resume().unwrap();
        assert!(r.retired.is_empty());
        assert_eq!(state.borrow().live.len(), 1);
    }
    #[test]
    fn unresolved_retired_references_block_new_additions_without_growing_ownership() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut r = Registry::new(Fake(state.clone()));
        r.stage(plan(&[("cmd+a", 0, "old")])).unwrap();
        r.commit();
        state.borrow_mut().fail_release = true;
        r.suspend();
        assert_eq!(r.retired.len(), 1);
        state.borrow_mut().trace.clear();
        for (key, code) in [("cmd+s", 1), ("cmd+d", 2), ("cmd+f", 3)] {
            assert!(r.stage(plan(&[(key, code, "new")])).is_err());
            assert_eq!(r.retired.len(), 1);
        }
        assert!(!state
            .borrow()
            .trace
            .iter()
            .any(|s| s.starts_with("register:")));
        assert_eq!(state.borrow().live.len(), 1);
        state.borrow_mut().fail_release = false;
        r.resume().unwrap();
        assert!(r.retired.is_empty());
        assert_eq!(state.borrow().live.len(), 1);
    }
    #[test]
    fn plan_preserves_carbon_flags_sided_policy_modifier_taps_and_disabled_items() {
        let mut value = rustcast_core::default_config();
        value["separateCommandKeys"] = serde_json::json!(true);
        value["commands"] = serde_json::json!([
            {"id":"a","title":"A","kind":"snippet","target":"not put in plan","hotkey":"RIGHT-CMD+a"},
            {"id":"b","title":"B","kind":"snippet","target":"private","hotkey":"ctrl+left-control"},
            {"id":"c","title":"C","kind":"snippet","target":"private","hotkey":"cmd+backspace"},
            {"id":"d","title":"D","kind":"snippet","target":"private","hotkey":"ctrl+alt+shift+cmd+f12","enabled":false}
        ]);
        let c = rustcast_core::validate(&value).unwrap();
        let p = Plan::from_config(&c).unwrap();
        assert_eq!(p.sided["right-cmd+a"], "a");
        assert_eq!(p.modifier_taps[&0xe0], "b");
        assert_eq!(p.carbon["cmd+backspace"].code, 51);
        assert_eq!(p.carbon["cmd+backspace"].modifiers, 256);
        assert!(!format!("{p:?}").contains("private"));
        assert!(!p.carbon.contains_key("alt+cmd+ctrl+shift+f12"));
        let mut c = c;
        c.separate_command_keys = Some(false);
        assert!(Plan::from_config(&c).unwrap().sided.is_empty());
        c.commands[1].hotkey = Some("cmd+left-control".into());
        assert!(Plan::from_config(&c).is_err());
    }
}
