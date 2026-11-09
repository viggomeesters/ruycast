//! Private clipboard state for the common typed receiver arena. No Objective-C
//! pointer registry, class factory, original pasteboard call or admission flag.
//! The common owner must supply qualified factories, +0/+1 ARC and deallocation.
use super::owned::AliveLease;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    thread::ThreadId,
};
pub type Result<T> = std::result::Result<T, String>;
/// Scope ownership is coalesced by the existing workspace factory module.
/// This declaration neither registers a module nor admits its data imports.
pub fn scopes() -> std::collections::BTreeSet<&'static str> {
    ["clipboard"].into_iter().collect()
}
/// Actual Native product callers. Probe-only and frozen-reference fixtures are
/// not promoted to Native callers by this roster. The common factory owner must
/// additionally bind their actual selected methods and imported data cells.
pub fn source_roster() -> std::collections::BTreeSet<super::SourceCallsite> {
    use sha2::{Digest, Sha256};
    let sources = [
        ("rust-native/src/native_services.rs", include_bytes!("../../../../rust-native/src/native_services.rs").as_slice(), "NativeClipboard history polling, private types, file-only reads and quicklink prefill"),
        ("rust-native/src/native_paste.rs", include_bytes!("../../../../rust-native/src/native_paste.rs").as_slice(), "NativePaste board ownership, publication verification, change counts and retirement"),
        ("rust-native/src/snippet_clipboard.rs", include_bytes!("../../../../rust-native/src/snippet_clipboard.rs").as_slice(), "PreparedItem string/data construction, board publication and attached-item readback"),
        ("rust-native/src/creation.rs", include_bytes!("../../../../rust-native/src/creation.rs").as_slice(), "Quicklink prefill reads the selected board string without mutating it"),
        ("rust-native/src/keyword_io.rs", include_bytes!("../../../../rust-native/src/keyword_io.rs").as_slice(), "Keyword transaction clipboard reads, publication, ownership checks and change counts"),
    ]
    .into_iter()
    .map(|(file, bytes, operation)| super::SourceCallsite {
        file,
        source_sha256: format!("{:x}", Sha256::digest(bytes)),
        operation,
    })
    .collect::<std::collections::BTreeSet<_>>();
    #[cfg(rcf1_reference_sdk)]
    let sources = sources.into_iter().chain(reference_source_roster()).collect();
    sources
}
/// Fixed original product callers, independently required in the Reference
/// compiler manifest. Native Rust callers cannot stand in for these members.
#[cfg(any(rcf1_reference_sdk,test))]
pub(crate) fn reference_source_roster() -> std::collections::BTreeSet<super::SourceCallsite> {
    [
        ("bench/reference/0.8.26/Sources/RuycastApp/AppState.swift","ffba512c44134bf74005bd6cc409c1c92d09b4a13ee9709ca60a7b6e895abd14"),
        ("bench/reference/0.8.26/Sources/RuycastApp/ClipboardStore.swift","3b448fe1131b8b9a96825e67a9181132d5719719ec17474ebe74cfa38196ef39"),
        ("bench/reference/0.8.26/Sources/RuycastApp/SnippetClipboard.swift","c433459f32c8eaa4530cb89c10209b5a60bdfc1d8bf336e830bdeb4e6052a916"),
        ("bench/reference/0.8.26/Sources/RuycastApp/SnippetExpander.swift","0982fd55b9406f9983d40803ada1f97d415251ca0e5908b888b27da7fd697b96"),
        ("bench/reference/0.8.26/Sources/RuycastApp/SnippetChoice.swift","3d43c84d15dc76aaed1a38c5e7d4131232168fb0d0723c79e52deb1e447b9d69"),
        ("bench/reference/0.8.26/Sources/RuycastApp/CommandEditor.swift","bedf706738b8c928bccca334e02921483120ec51958d6bb2ea97b270ebd5bf80"),
        ("bench/reference/0.8.26/Sources/RuycastApp/PaletteActions.swift","da7af103f2997d7eb24e6d91985d857ddd6ef3f11e6d8171ef7f53fc4e516213"),
    ].into_iter().map(|(file,hash)|super::SourceCallsite {
        file,source_sha256:hash.into(),operation:"Original private clipboard string/data/items/file URL callers",
    }).collect()
}
pub type Type = Vec<u16>;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Text(Vec<u16>),
    Data(Vec<u8>),
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ItemData(Vec<(Type, Value)>);
impl ItemData {
    pub fn types(&self) -> Vec<Type> {
        self.0.iter().map(|(kind, _)| kind.clone()).collect()
    }
    pub fn value(&self, kind: &[u16]) -> Option<Value> {
        self.0
            .iter()
            .find(|(key, _)| key == kind)
            .map(|(_, value)| value.clone())
    }
    fn set(&mut self, kind: Type, value: Value) -> Result<()> {
        if kind.is_empty() || kind.contains(&0) {
            return Err("invalid clipboard type".into());
        }
        if let Some((_, previous)) = self.0.iter_mut().find(|(key, _)| key == &kind) {
            *previous = value;
        } else {
            self.0.push((kind, value));
        }
        Ok(())
    }
}
struct Guard {
    life: Arc<dyn AliveLease>,
    retired: AtomicBool,
    thread: ThreadId,
}
impl Guard {
    fn pure_check(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) || self.thread != std::thread::current().id() {
            return Err("clipboard context retired or foreign thread".into());
        }
        Ok(())
    }
    fn check(&self) -> Result<()> {
        self.pure_check()?;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.life.before_effect()?;
            if !self.life.is_alive() {
                return Err("clipboard guardian unavailable".into());
            }
            Ok(())
        }));
        match outcome {
            Ok(Ok(())) => self.pure_check(),
            Ok(Err(error)) => {
                self.retired.store(true, Ordering::Release);
                Err(error)
            }
            Err(payload) => {
                std::mem::forget(payload);
                self.retired.store(true, Ordering::Release);
                Err("clipboard guardian callback panicked".into())
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Read,
    Set,
    Write,
    Clear,
}
struct BoardData {
    count: isize,
    items: Vec<Arc<Item>>,
    released: bool,
    fail_next: Option<Operation>,
    partial_write: Option<(usize, usize)>,
}
pub struct Board {
    guard: Arc<Guard>,
    name: Vec<u16>,
    state: Mutex<BoardData>,
    generation: Arc<AtomicU64>,
}
#[derive(Clone)]
struct Attachment {
    generation: Weak<AtomicU64>,
    epoch: u64,
}
pub struct Item {
    guard: Arc<Guard>,
    data: Mutex<ItemData>,
    fail_set: AtomicBool,
    attachment: Mutex<Option<Attachment>>,
}
pub enum WriteObject {
    Item(Arc<Item>),
    FileUrl(Vec<u16>),
}
pub struct Clipboard {
    guard: Arc<Guard>,
    namespace: String,
    serial: AtomicU64,
    general: Arc<Board>,
}
impl Clipboard {
    pub fn new(namespace: &str, life: Arc<dyn AliveLease>) -> Result<Self> {
        if namespace.is_empty()
            || namespace.len() > 128
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("invalid private clipboard namespace".into());
        }
        let guard = Arc::new(Guard {
            life,
            retired: AtomicBool::new(false),
            thread: std::thread::current().id(),
        });
        guard.check()?;
        let namespace = format!("ruycast-f1-private-{namespace}");
        let general = Self::board(&guard, format!("{namespace}-general"));
        Ok(Self {
            guard,
            namespace,
            serial: AtomicU64::new(1),
            general,
        })
    }
    fn board(guard: &Arc<Guard>, name: String) -> Arc<Board> {
        Arc::new(Board {
            guard: guard.clone(),
            name: name.encode_utf16().collect(),
            state: Mutex::new(BoardData {
                count: 0,
                items: vec![],
                released: false,
                fail_next: None,
                partial_write: None,
            }),
            generation: Arc::new(AtomicU64::new(0)),
        })
    }
    pub fn general(&self) -> Result<Arc<Board>> {
        self.guard.check()?;
        Ok(self.general.clone())
    }
    pub fn unique(&self) -> Result<Arc<Board>> {
        self.guard.check()?;
        let serial = self
            .serial
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "clipboard name counter exhausted")?;
        self.guard.pure_check()?;
        Ok(Self::board(
            &self.guard,
            format!("{}-{serial}", self.namespace),
        ))
    }
    /// The sole common factory owns repeated-name identity and released-name
    /// removal. Call only after its qualified name lookup found no live board;
    /// this leaf deliberately has no second receiver/name registry.
    pub fn new_named(&self, name: Vec<u16>) -> Result<Arc<Board>> {
        self.guard.check()?;
        if name.is_empty() || name.len() > 1024 || name.contains(&0) {
            return Err("invalid private clipboard board name".into());
        }
        let name =
            String::from_utf16(&name).map_err(|_| "invalid private clipboard name Unicode")?;
        Ok(Self::board(&self.guard, name))
    }
    pub fn item(&self) -> Result<Arc<Item>> {
        self.item_from_data(ItemData::default())
    }
    /// Pure origin check; this grants no operation or liveness on its own.
    pub fn owns_item(&self, item: &Arc<Item>) -> bool {
        Arc::ptr_eq(&self.guard, &item.guard)
    }
    pub fn item_from_data(&self, data: ItemData) -> Result<Arc<Item>> {
        self.guard.check()?;
        Ok(Item::new(&self.guard, data))
    }
    pub fn retire(&self) {
        self.guard.retired.store(true, Ordering::Release);
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        self.retire();
    }
}
impl Item {
    fn new(guard: &Arc<Guard>, data: ItemData) -> Arc<Self> {
        Arc::new(Self {
            guard: guard.clone(),
            data: Mutex::new(data),
            fail_set: AtomicBool::new(false),
            attachment: Mutex::new(None),
        })
    }
    fn current(&self) -> Result<()> {
        self.guard.pure_check()?;
        if let Some(binding) = self
            .attachment
            .lock()
            .map_err(|_| "clipboard attachment poisoned")?
            .as_ref()
        {
            if binding
                .generation
                .upgrade()
                .is_none_or(|generation| generation.load(Ordering::Acquire) != binding.epoch)
            {
                return Err("stale bound clipboard item".into());
            }
        }
        Ok(())
    }
    fn unbound(&self) -> Result<()> {
        if self
            .attachment
            .lock()
            .map_err(|_| "clipboard attachment poisoned")?
            .is_some()
        {
            return Err("clipboard item already bound".into());
        }
        Ok(())
    }
    fn attach(&self, generation: &Arc<AtomicU64>) -> Result<()> {
        let mut binding = self
            .attachment
            .lock()
            .map_err(|_| "clipboard attachment poisoned")?;
        if binding.is_some() {
            return Err("clipboard item already bound".into());
        }
        *binding = Some(Attachment {
            generation: Arc::downgrade(generation),
            epoch: generation.load(Ordering::Acquire),
        });
        Ok(())
    }
    pub fn fail_next_set(&self) -> Result<()> {
        self.guard.check()?;
        self.fail_set.store(true, Ordering::Release);
        Ok(())
    }
    pub fn set(&self, kind: Type, value: Value) -> Result<()> {
        self.guard.check()?;
        if self.fail_set.swap(false, Ordering::AcqRel) {
            return Err("injected private clipboard item failure".into());
        }
        let mut data = self.data.lock().map_err(|_| "clipboard item poisoned")?;
        self.current()?;
        data.set(kind, value)
    }
    pub fn snapshot(&self) -> Result<ItemData> {
        self.guard.check()?;
        let data = self.data.lock().map_err(|_| "clipboard item poisoned")?;
        self.current()?;
        Ok(data.clone())
    }
}
impl Board {
    fn with<T>(
        &self,
        operation: Operation,
        body: impl FnOnce(&mut BoardData) -> Result<T>,
    ) -> Result<T> {
        self.guard.check()?;
        let mut state = self.state.lock().map_err(|_| "clipboard board poisoned")?;
        self.guard.pure_check()?;
        if state.released {
            return Err("private clipboard board released".into());
        }
        if state.fail_next == Some(operation) {
            state.fail_next = None;
            return Err("injected private clipboard failure".into());
        }
        body(&mut state)
    }
    fn increment(state: &mut BoardData) -> Result<isize> {
        state.count = state
            .count
            .checked_add(1)
            .ok_or("clipboard change count exhausted")?;
        Ok(state.count)
    }
    pub fn name(&self) -> Result<Vec<u16>> {
        self.with(Operation::Read, |_| Ok(self.name.clone()))
    }
    pub fn change_count(&self) -> Result<isize> {
        self.with(Operation::Read, |s| Ok(s.count))
    }
    pub fn clear(&self) -> Result<isize> {
        self.with(Operation::Clear, |s| {
            let next = s
                .count
                .checked_add(1)
                .ok_or("clipboard change count exhausted")?;
            self.generation
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .map_err(|_| "clipboard generation exhausted")?;
            s.count = next;
            s.items.clear();
            Ok(next)
        })
    }
    pub fn release(&self) -> Result<()> {
        self.with(Operation::Clear, |s| {
            self.generation
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .map_err(|_| "clipboard generation exhausted")?;
            s.released = true;
            s.items.clear();
            Ok(())
        })
    }
    pub fn fail_next(&self, operation: Operation) -> Result<()> {
        self.with(Operation::Read, |s| {
            s.fail_next = Some(operation);
            Ok(())
        })
    }
    pub fn fail_next_partial_write(&self, items: usize, types: usize) -> Result<()> {
        self.with(Operation::Read, |s| {
            s.partial_write = Some((items, types));
            Ok(())
        })
    }
    pub fn items(&self) -> Result<Vec<ItemData>> {
        self.bound_items()?
            .iter()
            .map(|item| item.snapshot())
            .collect()
    }
    /// The core wraps these very items, preserving attachment and stale-owner
    /// checks instead of manufacturing unbound snapshots for pasteboardItems.
    pub fn bound_items(&self) -> Result<Vec<Arc<Item>>> {
        self.with(Operation::Read, |s| Ok(s.items.clone()))
    }
    pub fn types(&self) -> Result<Vec<Type>> {
        {
            let mut result = vec![];
            for item in self.items()? {
                for kind in item.types() {
                    if !result.contains(&kind) {
                        result.push(kind);
                    }
                }
            }
            Ok(result)
        }
    }
    pub fn value(&self, kind: &[u16]) -> Result<Option<Value>> {
        let values = self
            .items()?
            .iter()
            .filter_map(|item| item.value(kind))
            .collect::<Vec<_>>();
        if kind == "public.utf8-plain-text".encode_utf16().collect::<Vec<_>>() {
            let mut combined: Option<Vec<u16>> = None;
            for value in values {
                let text = match value {
                    Value::Text(text) => Some(text),
                    Value::Data(bytes) => String::from_utf8(bytes)
                        .ok()
                        .map(|s| s.encode_utf16().collect()),
                };
                if let Some(text) = text {
                    if let Some(out) = combined.as_mut() {
                        out.push(10);
                        out.extend(text);
                    } else {
                        combined = Some(text);
                    }
                }
            }
            return Ok(combined.map(Value::Text));
        }
        Ok(values.into_iter().next())
    }
    pub fn set(&self, kind: Type, value: Value) -> Result<()> {
        self.with(Operation::Set, |s| {
            let item = s.items.first().cloned();
            let mut data = if let Some(item) = &item {
                item.current()?;
                item.data
                    .lock()
                    .map_err(|_| "clipboard item poisoned")?
                    .clone()
            } else {
                ItemData::default()
            };
            data.set(kind, value)?;
            Self::increment(s)?;
            if let Some(item) = item {
                *item.data.lock().map_err(|_| "clipboard item poisoned")? = data;
            } else {
                let item = Item::new(&self.guard, data);
                item.attach(&self.generation)?;
                s.items.push(item);
            }
            Ok(())
        })
    }
    pub fn write(&self, objects: &[WriteObject]) -> Result<()> {
        self.guard.check()?;
        let mut items = Vec::with_capacity(objects.len());
        for object in objects {
            match object {
                WriteObject::Item(item) => {
                    if !Arc::ptr_eq(&self.guard, &item.guard) {
                        return Err("foreign clipboard item context".into());
                    }
                    item.snapshot()?;
                    item.unbound()?;
                    if items.iter().any(|previous| Arc::ptr_eq(previous, item)) {
                        return Err("duplicate clipboard item".into());
                    }
                    items.push(item.clone());
                }
                WriteObject::FileUrl(url) => {
                    let text = String::from_utf16(url).map_err(|_| "invalid file URL Unicode")?;
                    if !text.starts_with("file://") || url.contains(&0) {
                        return Err("non-file clipboard URL".into());
                    }
                    let mut item = ItemData::default();
                    item.set(
                        "public.file-url".encode_utf16().collect(),
                        Value::Text(url.clone()),
                    )?;
                    items.push(Item::new(&self.guard, item));
                }
            }
        }
        self.with(Operation::Write, |s| {
            for item in &items {
                item.current()?;
                item.unbound()?;
            }
            Self::increment(s)?;
            if let Some((count, types)) = s.partial_write.take() {
                items.truncate(count);
                for item in items {
                    let mut data = item
                        .data
                        .lock()
                        .map_err(|_| "clipboard item poisoned")?
                        .clone();
                    data.0.truncate(types);
                    item.attach(&self.generation)?;
                    let partial = Item::new(&self.guard, data);
                    partial.attach(&self.generation)?;
                    s.items.push(partial);
                }
                return Err("injected partial private clipboard write".into());
            }
            for item in &items {
                item.attach(&self.generation)?;
            }
            s.items.extend(items);
            Ok(())
        })
    }
    pub fn file_urls(&self) -> Result<Vec<Vec<u16>>> {
        {
            Ok(self
                .items()?
                .iter()
                .filter_map(|item| {
                    match item.value(&"public.file-url".encode_utf16().collect::<Vec<_>>()) {
                        Some(Value::Text(text))
                            if String::from_utf16(&text)
                                .is_ok_and(|s| s.starts_with("file://") && !s.contains('\0')) =>
                        {
                            Some(text)
                        }
                        Some(Value::Data(bytes)) => String::from_utf8(bytes)
                            .ok()
                            .filter(|s| s.starts_with("file://") && !s.contains('\0'))
                            .map(|s| s.encode_utf16().collect()),
                        _ => None,
                    }
                })
                .collect())
        }
    }
}
