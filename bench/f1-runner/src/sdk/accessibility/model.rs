//! Complete owned fixture tree. Addresses never authorize OS objects; only
//! retained tokens in this generation's explicit tree are recognized.
use super::opaque::{Payload, Ptr, Token};
use crate::sdk::owned::AliveLease;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    panic::{catch_unwind,AssertUnwindSafe},
};
pub const INVALID: i32 = -25202;
pub const ILLEGAL: i32 = -25201;
pub const UNSUPPORTED: i32 = -25205;
pub const ACTION_UNSUPPORTED: i32 = -25206;
pub const DISABLED: i32 = -25211;
pub const NO_VALUE: i32 = -25212;
pub type Result<T> = std::result::Result<T, i32>;
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    Text(String),
    Boolean(bool),
    Integer(i64),
    Value(Payload),
    Element(u64),
    Elements(Vec<u64>),
}
#[derive(Clone, Debug)]
pub struct NodeSpec {
    pub id: u64,
    pub pid: i32,
    pub attributes: BTreeMap<String, Attribute>,
    pub writable: BTreeSet<String>,
    pub actions: BTreeSet<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Faults {
    pub reads: VecDeque<i32>,
    pub writes: VecDeque<(i32, bool)>,
    pub actions: VecDeque<i32>,
    pub maximum_size: Option<[f64; 2]>,
}
struct Node {
    token: Token,
    specification: NodeSpec,
    faults: Faults,
    valid: bool,
    timeout: f32,
    action_count: BTreeMap<String, usize>,
}
struct Tree {
    trusted: bool,
    nodes: BTreeMap<u64, Node>,
    applications: BTreeMap<i32, u64>,
    pointers: BTreeMap<usize, u64>,
    system: u64,
}
pub struct Context {
    life: Arc<dyn AliveLease>,
    retired: AtomicBool,
    tree: Mutex<Tree>,
    #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
    pub(super) reference:Option<Arc<super::abi::ReferenceBinding>>,
}
impl Context {
    pub(super) fn live(&self) -> bool {
        self.check().is_ok()
    }
    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub fn new(
        life: Arc<dyn AliveLease>,
        system: u64,
        applications: BTreeMap<i32, u64>,
        specifications: Vec<NodeSpec>,
    ) -> std::result::Result<Arc<Self>, String> {
        life.before_effect()?;
        if !life.is_alive() || specifications.is_empty() || specifications.len() > 8192 {
            return Err("invalid owned AX lease/tree size".into());
        }
        let mut nodes = BTreeMap::new();
        let mut pointers = BTreeMap::new();
        for s in specifications {
            if s.id == 0
                || s.pid < 0
                || nodes.contains_key(&s.id)
                || s.attributes.len() > 64
                || s.attributes
                    .keys()
                    .chain(s.writable.iter())
                    .chain(s.actions.iter())
                    .any(|s| s.is_empty() || s.len() > 256)
            {
                return Err("invalid owned AX node".into());
            }
            for attribute in s.attributes.values() {
                match attribute {
                    Attribute::Text(text) if text.encode_utf16().count() > 65_536 => {
                        return Err("owned AX text bound".into())
                    }
                    Attribute::Value(value) if !valid_payload(*value) => {
                        return Err("invalid owned AX geometry/range".into())
                    }
                    Attribute::Elements(ids) if ids.len() > 8192 => {
                        return Err("owned AX list bound".into())
                    }
                    _ => (),
                }
            }
            let token = Token::element()?;
            pointers.insert(token.ptr() as usize, s.id);
            nodes.insert(
                s.id,
                Node {
                    token,
                    specification: s,
                    faults: Faults::default(),
                    valid: true,
                    timeout: 0.0,
                    action_count: BTreeMap::new(),
                },
            );
        }
        if !nodes.contains_key(&system) {
            return Err("missing own system element".into());
        }
        for (pid, id) in &applications {
            if *pid <= 0 || nodes.get(id).is_none_or(|n| n.specification.pid != *pid) {
                return Err("unmapped owned AX app identity".into());
            }
        }
        for node in nodes.values() {
            for attribute in node.specification.attributes.values() {
                match attribute {
                    Attribute::Element(id) if !nodes.contains_key(id) => {
                        return Err("unknown own AX reference".into())
                    }
                    Attribute::Elements(ids) if ids.iter().any(|id| !nodes.contains_key(id)) => {
                        return Err("unknown own AX list reference".into())
                    }
                    _ => (),
                }
            }
        }
        Ok(Arc::new(Self {
            life,
            retired: AtomicBool::new(false),
            tree: Mutex::new(Tree {
                trusted: true,
                nodes,
                applications,
                pointers,
                system,
            }),
            #[cfg(all(rcf1_reference_sdk,not(test),target_arch="aarch64"))]
            reference:None,
        }))
    }
    fn check(&self) -> Result<()> {
        self.check_pure()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if let Some(reference) = self.reference.as_ref().filter(|reference| !reference.is_owner_thread()) {
            // A stopped or refused worker never owns retirement of the
            // main-thread AX context. Main still needs it for teardown.
            catch_unwind(AssertUnwindSafe(|| reference.verify_search_worker()))
                .ok().and_then(std::result::Result::ok).ok_or(DISABLED)?;
            return self.check_pure();
        }
        let live=catch_unwind(AssertUnwindSafe(||self.life.is_alive()&&self.life.before_effect().is_ok())).unwrap_or(false);
        if !live {
            self.retired.store(true, Ordering::Release);
            return Err(DISABLED);
        }
        self.check_pure()
    }
    pub(super) fn check_pure(&self)->Result<()>{
        if self.retired.load(Ordering::Acquire){Err(DISABLED)}else{Ok(())}
    }
    fn with<T>(&self, operation: impl FnOnce(&mut Tree) -> Result<T>) -> Result<T> {
        self.check()?;
        let mut tree = self.tree.lock().map_err(|_| DISABLED)?;
        self.check_pure()?;
        let result = operation(&mut tree)?;
        // A guardian can reenter this Context. Never run it under Tree's lock.
        drop(tree);
        self.check()?;
        Ok(result)
    }
    pub fn trusted(&self) -> bool {
        self.with(|t| Ok(t.trusted)).unwrap_or(false)
    }
    pub fn application(&self, pid: i32) -> Result<Token> {
        self.with(|t| {
            t.applications
                .get(&pid)
                .and_then(|id| t.nodes.get(id))
                .filter(|n| n.valid)
                .map(|n| n.token.clone())
                .ok_or(INVALID)
        })
    }
    pub fn system(&self) -> Result<Token> {
        self.with(|t| {
            t.nodes
                .get(&t.system)
                .filter(|n| n.valid)
                .map(|n| n.token.clone())
                .ok_or(INVALID)
        })
    }
    pub fn pid(&self, pointer: Ptr) -> Result<i32> {
        self.with(|t| Ok(node(t, pointer, false)?.specification.pid))
    }
    pub fn timeout(&self, pointer: Ptr, seconds: f32) -> Result<()> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(ILLEGAL);
        }
        self.with(|t| {
            node(t, pointer, false)?.timeout = seconds;
            Ok(())
        })
    }
    pub fn attribute(&self, pointer: Ptr, key: &str) -> Result<Token> {
        self.with(|t| {
            let selected = {
                let node = node(t, pointer, true)?;
                read_fault(node)?;
                node.specification
                    .attributes
                    .get(key)
                    .cloned()
                    .ok_or(UNSUPPORTED)?
            };
            encode(t, &selected)
        })
    }
    pub fn count(&self, pointer: Ptr, key: &str) -> Result<isize> {
        self.with(|t| {
            let node = node(t, pointer, true)?;
            read_fault(node)?;
            match node.specification.attributes.get(key) {
                Some(Attribute::Elements(ids)) => Ok(ids.len() as isize),
                Some(_) => Err(ILLEGAL),
                None => Err(UNSUPPORTED),
            }
        })
    }
    pub fn elements(&self, pointer: Ptr, key: &str, index: isize, count: isize) -> Result<Token> {
        if index < 0 || count < 0 {
            return Err(ILLEGAL);
        }
        self.with(|t| {
            let selected = {
                let node = node(t, pointer, true)?;
                read_fault(node)?;
                let ids = match node.specification.attributes.get(key) {
                    Some(Attribute::Elements(ids)) => ids,
                    Some(_) => return Err(ILLEGAL),
                    None => return Err(UNSUPPORTED),
                };
                let start = index as usize;
                if start > ids.len() {
                    return Err(ILLEGAL);
                }
                let end = start
                    .checked_add(count as usize)
                    .ok_or(ILLEGAL)?
                    .min(ids.len());
                Attribute::Elements(ids[start..end].to_vec())
            };
            encode(t, &selected)
        })
    }
    pub fn write(&self, pointer: Ptr, key: &str, value: Attribute) -> Result<()> {
        self.write_inner(pointer, key, value, false)
    }
    pub(super) fn write_inner(&self, pointer: Ptr, key: &str, mut value: Attribute, restore: bool) -> Result<()> {
        if matches!(&value,Attribute::Text(t) if t.encode_utf16().count()>65_536) {
            return Err(ILLEGAL);
        }
        let commit = |t: &mut Tree| {
            let n = node(t, pointer, true)?;
            if !n.specification.writable.contains(key) {
                return Err(UNSUPPORTED);
            }
            let before = n.specification.attributes.get(key).ok_or(UNSUPPORTED)?;
            if restore && (before != &Attribute::Boolean(true)
                || !matches!(n.specification.attributes.get("AXRole"), Some(Attribute::Text(role)) if role == "AXWindow"))
            {
                return Err(UNSUPPORTED);
            }
            if !same_type(before, &value) {
                return Err(ILLEGAL);
            }
            if let Attribute::Value(payload) = value {
                if !valid_payload(payload) {
                    return Err(ILLEGAL);
                }
            }
            let (code, apply) = n.faults.writes.pop_front().unwrap_or((0, true));
            if apply {
                if let (Some(limit), Attribute::Value(Payload::Size(size))) =
                    (n.faults.maximum_size, &value)
                {
                    value = Attribute::Value(Payload::Size([
                        size[0].min(limit[0]),
                        size[1].min(limit[1]),
                    ]));
                }
                n.specification.attributes.insert(key.into(), value);
            }
            if code == 0 {
                Ok(())
            } else {
                Err(code)
            }
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if restore {
            return self.with_reference_activation(pointer, false, commit);
        }
        self.with(commit)
    }
    pub fn action(&self, pointer: Ptr, key: &str) -> Result<()> {
        let commit = |t: &mut Tree| {
            let n = node(t, pointer, true)?;
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if self.reference.is_some() && key == "AXPress"
                && (!matches!(n.specification.attributes.get("AXRole"), Some(Attribute::Text(role)) if role == "AXMenuItem")
                    || n.specification.attributes.get("AXEnabled") != Some(&Attribute::Boolean(true))
                    || !matches!(n.specification.attributes.get("AXChildren"), Some(Attribute::Elements(children)) if children.is_empty()))
            {
                return Err(ACTION_UNSUPPORTED);
            }
            if !n.specification.actions.contains(key) {
                return Err(ACTION_UNSUPPORTED);
            }
            let code = n.faults.actions.pop_front().unwrap_or(0);
            if code != 0 {
                return Err(code);
            }
            let count = n.action_count.entry(key.into()).or_default();
            *count = count.checked_add(1).ok_or(ILLEGAL)?;
            Ok(())
        };
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if self.reference.is_some() {
            return self.with_reference_activation(pointer, true, commit);
        }
        self.with(commit)
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn with_reference_activation(
        &self, pointer: Ptr, consume_activation: bool, commit: impl FnOnce(&mut Tree) -> Result<()>,
    ) -> Result<()> {
        let reference = self.reference.as_ref().ok_or(DISABLED)?;
        if !reference.is_owner_thread() {
            return Err(DISABLED);
        }
        let pid = self.pid(pointer)?;
        reference.model.with_reference_frontmost(pid, consume_activation, |workspace_faulted, stopped| {
            let mut tree = self.tree.lock().map_err(|_| DISABLED)?;
            self.check_pure()?;
            if workspace_faulted.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
                return Err(DISABLED);
            }
            commit(&mut tree)
        }).map_err(|_| DISABLED)?
    }
    // Typed fixture-owner controls, never product SDK callbacks.
    pub fn permission(&self, granted: bool) -> Result<()> {
        self.with(|t| {
            t.trusted = granted;
            Ok(())
        })
    }
    pub fn invalidate(&self, id: u64) -> Result<()> {
        self.with(|t| {
            t.nodes.get_mut(&id).ok_or(INVALID)?.valid = false;
            Ok(())
        })
    }
    pub fn faults(&self, id: u64, faults: Faults) -> Result<()> {
        if faults
            .maximum_size
            .is_some_and(|v| !v.into_iter().all(|n| n.is_finite() && n >= 0.0))
        {
            return Err(ILLEGAL);
        }
        self.with(|t| {
            t.nodes.get_mut(&id).ok_or(INVALID)?.faults = faults;
            Ok(())
        })
    }
    pub fn action_count(&self, id: u64, key: &str) -> Result<usize> {
        self.with(|t| {
            Ok(*t
                .nodes
                .get(&id)
                .ok_or(INVALID)?
                .action_count
                .get(key)
                .unwrap_or(&0))
        })
    }
}
fn node(tree: &mut Tree, pointer: Ptr, permission: bool) -> Result<&mut Node> {
    if permission && !tree.trusted {
        return Err(DISABLED);
    }
    let id = tree.pointers.get(&(pointer as usize)).ok_or(INVALID)?;
    tree.nodes.get_mut(id).filter(|n| n.valid).ok_or(INVALID)
}
fn read_fault(n: &mut Node) -> Result<()> {
    let code = n.faults.reads.pop_front().unwrap_or(0);
    if code == 0 {
        Ok(())
    } else {
        Err(code)
    }
}
pub fn valid_payload(payload: Payload) -> bool {
    match payload {
        Payload::Point(v) => v.into_iter().all(f64::is_finite),
        Payload::Size(v) => v.into_iter().all(|n| n.is_finite() && n >= 0.0),
        Payload::Range(r) => {
            r.location >= 0 && r.length >= 0 && r.location.checked_add(r.length).is_some()
        }
    }
}
fn same_type(a: &Attribute, b: &Attribute) -> bool {
    match (a, b) {
        (Attribute::Text(_), Attribute::Text(_))
        | (Attribute::Boolean(_), Attribute::Boolean(_))
        | (Attribute::Integer(_), Attribute::Integer(_)) => true,
        (Attribute::Value(a), Attribute::Value(b)) => a.kind() == b.kind(),
        _ => false,
    }
}
fn encode(tree: &Tree, value: &Attribute) -> Result<Token> {
    let error = |_: String| ILLEGAL;
    match value {
        Attribute::Text(s) => Token::string(s).map_err(error),
        Attribute::Boolean(b) => Ok(Token::boolean(*b)),
        Attribute::Integer(n) => Token::integer(*n).map_err(error),
        Attribute::Value(v) => Token::value(*v).map_err(error),
        Attribute::Element(id) => tree
            .nodes
            .get(id)
            .filter(|n| n.valid)
            .map(|n| n.token.clone())
            .ok_or(NO_VALUE),
        Attribute::Elements(ids) => {
            let tokens = ids
                .iter()
                .map(|id| {
                    tree.nodes
                        .get(id)
                        .filter(|n| n.valid)
                        .map(|n| &n.token)
                        .ok_or(INVALID)
                })
                .collect::<Result<Vec<_>>>()?;
            Token::array(&tokens).map_err(error)
        }
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    struct Audit(Mutex<std::sync::Weak<Context>>);
    impl AliveLease for Audit {
        fn is_alive(&self) -> bool { true }
        fn before_effect(&self) -> std::result::Result<(), String> {
            if let Some(context) = self.0.lock().unwrap().upgrade() {
                if context.tree.try_lock().is_err() {
                    return Err("dynamic guardian ran under AX tree lock".into());
                }
            }
            Ok(())
        }
    }
    #[test]
    fn dynamic_guard_runs_outside_tree_lock() {
        let audit = Arc::new(Audit(Mutex::new(std::sync::Weak::new())));
        let context = Context::new(audit.clone(), 1, BTreeMap::new(), vec![NodeSpec {
            id: 1, pid: 0, attributes: BTreeMap::new(), writable: BTreeSet::new(), actions: BTreeSet::new(),
        }]).unwrap();
        *audit.0.lock().unwrap() = Arc::downgrade(&context);
        assert!(context.trusted());
        assert!(context.system().is_ok());
        context.permission(false).unwrap();
        assert!(!context.trusted());
        // Permission denial must not be confused with guardian retirement.
        context.permission(true).unwrap();
        assert!(context.trusted());
    }
}
