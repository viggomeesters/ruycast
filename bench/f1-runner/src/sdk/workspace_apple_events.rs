//! Owned AppleEvent descriptor data and selected methods. No original NSAE
//! initializer, AppleEvent send, process lookup or consent API is invoked.
//! Core alone owns receiver identity, factory grants, generation and threads.
use super::*;
use objc2_foundation::{NSData, NSError};
use std::{
    cell::RefCell,
    ffi::c_void,
    rc::Rc,
    sync::{Arc, Mutex},
};

const fn code(bytes: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*bytes)
}
const NULL: u32 = code(b"null");
const RECORD: u32 = code(b"reco");
const LIST: u32 = code(b"list");
const EVENT: u32 = code(b"aevt");
const PID: u32 = code(b"kpid");
const LONG: u32 = code(b"long");
const TYPE: u32 = code(b"type");
const ENUM: u32 = code(b"enum");
const TEXT: u32 = code(b"utxt");
const MAX_BYTES: usize = 2_097_153; // one-over-limit reply oracle must survive
const MAX_ENCODED_BYTES: usize = MAX_BYTES + 65_536; // bounded AE container overhead
const MAX_ITEMS: usize = 513;
const MAX_DEPTH: usize = 64;

#[repr(C, packed(2))]
pub(super) struct AddressDesc {
    kind: u32,
    storage: *mut c_void,
}
unsafe impl objc2::encode::RefEncode for AddressDesc {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct(
            "AEDesc",
            &[
                objc2::encode::Encoding::UInt,
                objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Pointer(
                    &objc2::encode::Encoding::Struct("OpaqueAEDataStorageType", &[]),
                )),
            ],
        ));
}
const _: () = assert!(std::mem::size_of::<AddressDesc>() == 4 + std::mem::size_of::<usize>());
const _: () = assert!(std::mem::align_of::<AddressDesc>() == 2);
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, length: isize, result: *mut AddressDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AddressDesc) -> i16;
    fn AECreateList(data: *const c_void, length: isize, record: u8, result: *mut AddressDesc) -> i16;
    fn AEPutDesc(list: *mut AddressDesc, index: isize, item: *const AddressDesc) -> i16;
    fn AECreateAppleEvent(
        class: u32,
        id: u32,
        target: *const AddressDesc,
        return_id: i16,
        transaction: i32,
        result: *mut AddressDesc,
    ) -> i16;
    fn AEPutParamDesc(event: *mut AddressDesc, key: u32, item: *const AddressDesc) -> i16;
    fn AEGetDescDataSize(desc: *const AddressDesc) -> isize;
    fn AEGetDescData(desc: *const AddressDesc, data: *mut c_void, maximum: isize) -> i16;
}
struct OwnedDesc(Box<AddressDesc>);
impl OwnedDesc {
    fn from_node(node: &Node) -> Result<Self> {
        node.size(0)?;
        let mut desc = Box::new(AddressDesc {
            kind: 0,
            storage: std::ptr::null_mut(),
        });
        let status = if let Some(event) = &node.event {
            let target = Self::from_node(
                event
                    .target
                    .as_deref()
                    .unwrap_or(&Node::scalar(NULL, vec![])),
            )?;
            unsafe {
                AECreateAppleEvent(
                    event.class,
                    event.id,
                    &*target.0,
                    event.return_id,
                    event.transaction,
                    &mut *desc,
                )
            }
        } else if node.kind == LIST || matches!(node.kind, RECORD | 0x6f626a20) {
            unsafe { AECreateList(std::ptr::null(), 0, u8::from(node.kind != LIST), &mut *desc) }
        } else {
            unsafe {
                AECreateDesc(
                    node.kind,
                    node.bytes.as_ptr().cast(),
                    isize::try_from(node.bytes.len()).map_err(|_| "descriptor length overflow")?,
                    &mut *desc,
                )
            }
        };
        if status != 0 {
            return Err("owned AE descriptor creation failed".into());
        }
        let mut desc = Self(desc);
        for (index, item) in node.items.iter().enumerate() {
            let item = Self::from_node(item)?;
            if unsafe { AEPutDesc(&mut *desc.0, (index + 1) as isize, &*item.0) } != 0 {
                return Err("owned AE list insertion failed".into());
            }
        }
        for (key, item) in &node.fields {
            let item = Self::from_node(item)?;
            // AEPutKeyDesc is a header macro for AEPutParamDesc, not a symbol.
            if unsafe { AEPutParamDesc(&mut *desc.0, *key, &*item.0) } != 0 {
                return Err("owned AE record insertion failed".into());
            }
        }
        for (key, item) in &node.params {
            let item = Self::from_node(item)?;
            if unsafe { AEPutParamDesc(&mut *desc.0, *key, &*item.0) } != 0 {
                return Err("owned AE event parameter failed".into());
            }
        }
        // Object specifiers have record layout; this is the public descriptor
        // type field of our own descriptor, never its opaque storage handle.
        desc.0.kind = node.kind;
        Ok(desc)
    }
    fn data(&self) -> Result<Vec<u8>> {
        let size = unsafe { AEGetDescDataSize(&*self.0) };
        let size = usize::try_from(size).map_err(|_| "negative descriptor data size")?;
        if size > MAX_ENCODED_BYTES {
            return Err("owned encoded descriptor exceeds retention bound".into());
        }
        let mut bytes = vec![0; size];
        if unsafe { AEGetDescData(&*self.0, bytes.as_mut_ptr().cast(), size as isize) } != 0 {
            return Err("owned descriptor data read failed".into());
        }
        Ok(bytes)
    }
}
impl Drop for OwnedDesc {
    fn drop(&mut self) {
        unsafe { AEDisposeDesc(&mut *self.0) };
    }
}
#[derive(Clone)]
struct Event {
    class: u32,
    id: u32,
    target: Option<Rc<Node>>,
    return_id: i16,
    transaction: i32,
}
#[derive(Clone)]
struct Node {
    kind: u32,
    bytes: Vec<u8>,
    fields: BTreeMap<u32, Rc<Node>>,
    params: BTreeMap<u32, Rc<Node>>,
    items: Vec<Rc<Node>>,
    event: Option<Event>,
}
impl Node {
    fn scalar(kind: u32, bytes: Vec<u8>) -> Self {
        Self {
            kind,
            bytes,
            fields: BTreeMap::new(),
            params: BTreeMap::new(),
            items: vec![],
            event: None,
        }
    }
    fn int(value: i32) -> Self {
        Self::scalar(LONG, value.to_ne_bytes().to_vec())
    }
    fn ty(value: u32) -> Self {
        Self::scalar(TYPE, value.to_ne_bytes().to_vec())
    }
    fn text(value: &str) -> Self {
        Self::scalar(
            TEXT,
            value.encode_utf16().flat_map(u16::to_ne_bytes).collect(),
        )
    }
    fn integer(&self) -> Result<i32> {
        if !matches!(self.kind, LONG | PID) {
            return Err("descriptor is not an Int32".into());
        }
        Ok(i32::from_ne_bytes(
            self.bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid Int32 descriptor")?,
        ))
    }
    fn type_code(&self) -> Result<u32> {
        if !matches!(self.kind, TYPE | ENUM) {
            return Err("descriptor is not an OSType".into());
        }
        Ok(u32::from_ne_bytes(
            self.bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid type descriptor")?,
        ))
    }
    fn string(&self) -> Result<String> {
        if self.kind != TEXT || !self.bytes.len().is_multiple_of(2) {
            return Err("descriptor is not Unicode text".into());
        }
        String::from_utf16(
            &self
                .bytes
                .chunks_exact(2)
                .map(|b| u16::from_ne_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "invalid Unicode descriptor".into())
    }
    fn size(&self, depth: usize) -> Result<usize> {
        if depth > MAX_DEPTH
            || self.items.len() > MAX_ITEMS
            || self.fields.len() > 64
            || self.params.len() > 64
        {
            return Err("owned descriptor tree exceeds its bounds".into());
        }
        let mut size = self.bytes.len();
        for child in self
            .fields
            .values()
            .chain(self.params.values())
            .chain(self.items.iter())
            .chain(self.event.iter().flat_map(|e| e.target.iter()))
        {
            size = size
                .checked_add(child.size(depth + 1)?)
                .ok_or("descriptor size overflow")?;
        }
        if size > MAX_BYTES {
            return Err("owned descriptor bytes exceed their bound".into());
        }
        Ok(size)
    }
}
struct Value {
    node: Rc<Node>,
    address: OwnedDesc,
}
thread_local! { static VALUES:RefCell<BTreeMap<Handle,Value>>=const{RefCell::new(BTreeMap::new())}; }
#[cfg(test)]
thread_local! {static PREPARE_FAILURE:std::cell::Cell<u8>=const{std::cell::Cell::new(0)};}
fn prepare(cap: AppleEventHandle, node: Rc<Node>) -> Result<()> {
    node.size(0)?;
    let address = OwnedDesc::from_node(&node)?;
    let key = cap.key()?;
    VALUES.with(|values| -> Result<()> {
        let mut values = values.borrow_mut();
        if values.contains_key(&key) {
            return Err("descriptor handle replay".into());
        }
        values.insert(key, Value { node, address });
        Ok(())
    })?;
    #[cfg(test)]
    match PREPARE_FAILURE.with(std::cell::Cell::get) {
        1 => return Err("owned prepared-data rollback fixture".into()),
        2 => panic!("owned prepared-data panic rollback fixture"),
        _ => (),
    }
    Ok(())
}

#[cfg(test)]
mod shared_wire_tests {
    use super::*;
    #[test]
    fn owned_descriptor_sizes_indices_and_data_follow_native_long_abi() {
        let pid=700001i32;
        let scalar=Node::scalar(PID,pid.to_ne_bytes().to_vec());
        let address=OwnedDesc::from_node(&scalar).unwrap();
        assert_eq!(address.data().unwrap(),pid.to_ne_bytes());
        let text=Node::text("own \0🦀");
        let owned=OwnedDesc::from_node(&text).unwrap();
        assert_eq!(owned.data().unwrap(),text.bytes);
        let mut list=Node::scalar(LIST,vec![]);
        list.items=vec![Rc::new(scalar),Rc::new(text)];
        let owned=OwnedDesc::from_node(&list).unwrap();
        let encoded=owned.data().unwrap();
        assert!(!encoded.is_empty());
        assert_eq!(owned.data().unwrap(),encoded);
        let mut record=Node::scalar(RECORD,vec![]);
        record.fields.insert(code(b"own "),Rc::new(list));
        let owned=OwnedDesc::from_node(&record).unwrap();
        assert!(!owned.data().unwrap().is_empty());
        // Only descriptor allocation/encoding/disposal, never event sending.
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            let old = WIRE.lock().unwrap().take();
            drop(old);
        }
    }
    fn fixture_wire() -> Wire {
        Wire {
            pid: 730001,
            tabs: vec![Tab {
                window: 101,
                index: 2,
                title: "Owned title".into(),
                url: "https://example.test/owned".into(),
            }],
            selected: (101, 2),
            status: 0,
            transport_status: 0,
            sends: 0,
            sets: 0,
            raises: 0,
        }
    }
    // Transport-only: no original class, descriptor, callback or SDK is invoked.
    #[test]
    fn process_replacement_revokes_wire_snapshot_and_final_effect_commit() {
        let fixture=crate::workspace::tests::Fixture::new();let _reset=Reset;
        let monitor=Arc::new(crate::sdk::system_providers::Fixture::new(fixture.model.context.clone()).unwrap());
        monitor.clone().install().unwrap();
        let identity=monitor.processes.lock().unwrap().get(&730001).unwrap().identity();
        configure_for_model(fixture.model.clone(),fixture_wire()).unwrap();
        let generation=fixture.model.backend_generation().unwrap();
        // Plain fixture arrangement for this data/commit test, not a Reference
        // admission. The actual Reference initializer obtains this via router.
        with_wire(&fixture.model,generation,|state|{state.as_mut().unwrap().identity=Some(identity);Ok(())}).unwrap();
        let before=wire_snapshot(&fixture.model,generation).unwrap();
        monitor.processes.lock().unwrap().get_mut(&identity.pid).unwrap().started_seconds+=1;
        assert!(wire_check(&before).is_err());
        assert!(wire_snapshot(&fixture.model,generation).is_err());
        let mut committed=false;
        let result=with_wire(&fixture.model,generation,|state|with_wire_identity(before.identity,|| {
            state.as_mut().unwrap().transport_mut()?.sends+=1;committed=true;Ok(())
        }));
        assert!(result.is_err());assert!(!committed);
        with_wire(&fixture.model,generation,|state|{assert_eq!(state.as_ref().unwrap().transport()?.sends,0);Ok(())}).unwrap();
    }
    #[test]
    fn reference_target_uses_display_metadata_without_granting_real_bundle_access() {
        let fixture=crate::workspace::tests::Fixture::new();
        let physical=fixture.root.join("apps/Safari.app");
        std::fs::create_dir(&physical).unwrap();
        let mut app=fixture.model.applications().unwrap()[0].1.clone();
        app.bundle=Some("com.apple.Safari".into());app.pid=730001;
        app.bundle_path=Some(physical.clone());app.executable_path=None;
        let model=Workspace::new(fixture.model.context.clone(),vec![app.clone()],fixture.model.screens().unwrap(),fixture.model.mouse().unwrap()).unwrap();
        let wire=fixture_wire();
        assert!(validate_reference_wire(&model,Some(&wire),0).is_ok());
        assert_eq!(super::super::reference_bundle_metadata_path(&app).unwrap(),std::path::Path::new("/Applications/Safari.app"));
        assert_eq!(model.applications().unwrap()[0].1.bundle_path.as_ref(),Some(&physical));
        assert!(model.context.applications.info(std::path::Path::new("/Applications/Safari.app")).is_err(),"metadata cannot escape the physical filesystem root");
        let mut wrong=wire.clone();wrong.pid+=1;assert!(validate_reference_wire(&model,Some(&wrong),0).is_err());
        wrong=wire.clone();wrong.selected=(999,1);assert!(validate_reference_wire(&model,Some(&wrong),0).is_err());
        wrong=wire;wrong.sends=1;assert!(validate_reference_wire(&model,Some(&wrong),0).is_err());
        app.bundle=Some("dev.fixture.Unrelated".into());
        assert_eq!(super::super::reference_bundle_metadata_path(&app),Some(physical));
    }
    #[test]
    fn unavailable_backend_preserves_owner_and_revokes_old_transport_snapshots() {
        let fixture=crate::workspace::tests::Fixture::new();
        let foreign=crate::workspace::tests::Fixture::new();
        let _reset=Reset;
        configure_backend_for_model(fixture.model.clone(),None,true).unwrap();
        let generation=fixture.model.with(|state|Ok(state.generation)).unwrap();
        assert!(wire_snapshot(&fixture.model,generation).is_err());
        assert!(configure_backend_for_model(fixture.model.clone(),None,true).is_err());
        assert!(configure_for_model(foreign.model.clone(),fixture_wire()).is_err());
        with_wire(&fixture.model,generation,|state| {
            let owner=state.as_ref().unwrap();
            assert!(Arc::ptr_eq(&owner.model,&fixture.model));
            assert_eq!(owner.revision,1);assert!(owner.wire.is_none());Ok(())
        }).unwrap();
        assert!(validate_reference_wire(&fixture.model,None,-1743).is_ok());
        assert!(validate_reference_wire(&fixture.model,None,123).is_err());
        assert!(validate_reference_wire(&fixture.model,Some(&fixture_wire()),0).is_err(),"unrelated Workspace app cannot stand in for Safari");
        configure_for_model(fixture.model.clone(),fixture_wire()).unwrap();
        let before=wire_snapshot(&fixture.model,generation).unwrap();
        configure_backend_for_model(fixture.model.clone(),None,false).unwrap();
        assert!(wire_check(&before).is_err());
        assert!(wire_snapshot(&fixture.model,generation).is_err());
        fixture.model.retire().unwrap();
        assert!(configure_backend_for_model(fixture.model.clone(),None,false).is_err());
        assert!(configure_for_model(foreign.model.clone(),fixture_wire()).is_err());
    }
    #[test]
    fn original_two_worker_shapes_share_only_plain_wire_data() {
        let fixture = crate::workspace::tests::Fixture::new();
        let _reset = Reset;
        configure_for_model(fixture.model.clone(), fixture_wire()).unwrap();
        let generation = fixture.model.with(|s| Ok(s.generation)).unwrap();
        let ready = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let model = fixture.model.clone();
            let ready = ready.clone();
            workers.push(std::thread::spawn(move || {
                ready.wait();
                model.with(|_| Ok(())).unwrap();
                let snapshot = wire_snapshot(&model, generation).unwrap();
                (
                    snapshot.transport().unwrap().pid,
                    snapshot.transport().unwrap().selected,
                    snapshot.transport().unwrap().tabs[0].title.clone(),
                )
            }));
        }
        ready.wait();
        for worker in workers {
            let observed = worker.join().unwrap();
            assert_eq!(observed, (730001, (101, 2), "Owned title".into()));
        }
    }
    #[test]
    fn shared_wire_reconfiguration_rejects_stale_revision_without_effects() {
        let fixture = crate::workspace::tests::Fixture::new();
        let _reset = Reset;
        configure_for_model(fixture.model.clone(), fixture_wire()).unwrap();
        let generation = fixture.model.with(|s| Ok(s.generation)).unwrap();
        let before = wire_snapshot(&fixture.model, generation).unwrap();
        let model = fixture.model.clone();
        std::thread::spawn(move || configure_for_model(model, fixture_wire()))
            .join()
            .unwrap()
            .unwrap();
        assert!(wire_check(&before).is_err());
        let current = wire_snapshot(&fixture.model, generation).unwrap();
        assert_eq!(current.revision, before.revision + 1);
        assert_eq!(
            (current.transport().unwrap().sends, current.transport().unwrap().sets, current.transport().unwrap().raises),
            (0, 0, 0)
        );
        assert_eq!(current.transport().unwrap().selected, (101, 2));
    }
    #[test]
    fn shared_wire_refuses_foreign_model_generation_and_retirement() {
        let fixture = crate::workspace::tests::Fixture::new();
        let foreign = crate::workspace::tests::Fixture::new();
        let _reset = Reset;
        configure_for_model(fixture.model.clone(), fixture_wire()).unwrap();
        let generation = fixture.model.with(|s| Ok(s.generation)).unwrap();
        assert!(wire_snapshot(&foreign.model, generation).is_err());
        assert!(wire_snapshot(&fixture.model, generation + 1).is_err());
        let before = wire_snapshot(&fixture.model, generation).unwrap();
        assert!(configure_for_model(foreign.model.clone(),fixture_wire()).is_err());
        let after=wire_snapshot(&fixture.model,generation).unwrap();
        assert_eq!(after.revision,before.revision,"foreign replacement must leave the original owner unchanged");
        assert_eq!(after.transport().unwrap().selected,before.transport().unwrap().selected);
        fixture.model.retire().unwrap();
        assert!(wire_check(&before).is_err());
        assert!(wire_snapshot(&fixture.model, generation).is_err());
    }
    #[test]
    fn shared_wire_wrong_target_cannot_change_selection_or_counters() {
        let fixture = crate::workspace::tests::Fixture::new();
        let _reset = Reset;
        configure_for_model(fixture.model.clone(), fixture_wire()).unwrap();
        let generation = fixture.model.with(|s| Ok(s.generation)).unwrap();
        let before = wire_snapshot(&fixture.model, generation).unwrap();
        let mut event = Node::scalar(EVENT, vec![]);
        event.event = Some(Event {
            class: code(b"core"),
            id: code(b"getd"),
            target: Some(Rc::new(Node::scalar(
                PID,
                730002_i32.to_ne_bytes().to_vec(),
            ))),
            return_id: 0,
            transaction: 0,
        });
        let mut candidate = before.transport().unwrap().clone();
        assert!(interpret(&event, &mut candidate).is_err());
        wire_check(&before).unwrap();
        let current = wire_snapshot(&fixture.model, generation).unwrap();
        assert_eq!(current.transport().unwrap().selected, before.transport().unwrap().selected);
        assert_eq!(
            (current.transport().unwrap().sends, current.transport().unwrap().sets, current.transport().unwrap().raises),
            (0, 0, 0)
        );
    }
}
#[derive(Default)]
struct Preparation {
    key: Option<Handle>,
    published: bool,
}
struct PendingReceiver {
    receiver: *mut AnyObject,
    committed: bool,
}
impl Drop for PendingReceiver {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| unsafe {
                release_apple_event_descriptor(self.receiver)
            })) {
                std::mem::forget(payload);
            }
        }
    }
}
impl PendingReceiver {
    fn autorelease(&mut self) -> Result<*mut AnyObject> {
        let receiver = unsafe { autorelease_apple_event_descriptor(self.receiver) }?;
        self.committed = true;
        Ok(receiver)
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        if !self.published {
            if let Some(key) = self.key {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| release(key))) {
                    std::mem::forget(payload);
                }
            }
        }
    }
}
fn make(node: Node) -> Result<*mut AnyObject> {
    node.size(0)?;
    let node = Rc::new(node);
    let mut prepared = Preparation::default();
    let result = new_apple_event_descriptor(|cap| {
        let key = cap.key()?;
        prepared.key = Some(key);
        prepare(cap, node)?;
        Ok(())
    });
    prepared.published = result.is_ok();
    result
}
fn autoreleased(node: Node) -> Result<*mut AnyObject> {
    PendingReceiver {
        receiver: make(node)?,
        committed: false,
    }
    .autorelease()
}
// Constructing a child can run selected factories and dynamic lease callbacks.
// Keep the originating receiver's authority until those callbacks have returned,
// and remove an unreturned child if that origin retired in the meantime.
fn derived(cap: AppleEventHandle, node: Node) -> Result<*mut AnyObject> {
    let mut child = PendingReceiver {
        receiver: make(node)?,
        committed: false,
    };
    cap.check()?;
    child.autorelease()
}
unsafe fn bind(p: *mut AnyObject, node: Node) -> Result<*mut AnyObject> {
    let mut prepared = Preparation::default();
    let result = unsafe {
        bind_apple_event_descriptor(p, |cap| {
            let key = cap.key()?;
            prepared.key = Some(key);
            prepare(cap, Rc::new(node))?;
            Ok(())
        })
    };
    prepared.published = result.is_ok();
    result
}
// An init consumes its incoming allocated +1 even when parsing/admission
// fails. Core binds identity only; it never consumes that incoming reference.
fn initialize(p: *mut AnyObject, node: impl FnOnce() -> Result<Node>) -> Result<*mut AnyObject> {
    check_apple_event_initializer(p)?;
    let mut incoming = PendingReceiver {
        receiver: p,
        committed: false,
    };
    let result = unsafe { bind(p, node()?) }?;
    incoming.committed = true;
    Ok(result)
}
fn snapshot(p: *mut AnyObject) -> Result<(AppleEventHandle, Rc<Node>)> {
    let cap = apple_event_descriptor(p)?;
    let key = cap.key()?;
    let node = VALUES
        .with(|v| v.borrow().get(&key).map(|v| v.node.clone()))
        .ok_or("descriptor data absent")?;
    cap.check()?;
    Ok((cap, node))
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
thread_local! {
    static ASSEMBLY_SEND: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn call_original_safari_send(
    _: usize, _: i32, _: *mut AnyObject, _: usize, _: f64, _: *mut usize,
) -> *mut AnyObject {
    core::arch::naked_asm!(
        "stp x19, x21, [sp, #-32]!",
        "str x30, [sp, #16]",
        "mov x9, x0", "mov x0, x1", "mov x1, x2", "mov x2, x3",
        "mov x19, x4", "mov x21, xzr",
        "blr x9", "str x21, [x19]",
        "ldr x30, [sp, #16]",
        "ldp x19, x21, [sp], #32", "ret",
    );
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn probe_reference_safari_descriptors(
    admission: &crate::sdk::core_preentry::ReferenceAdmission,
) -> Result<()> {
    let [all, object, property, send] = admission.reference_safari_descriptor_entries()?;
    // The original static Swift function takes no arguments and returns the
    // retained result of NSAppleEventDescriptor.init in x0.
    let original: unsafe extern "C" fn() -> *mut AnyObject = unsafe { std::mem::transmute(all) };
    let descriptor = unsafe { Retained::from_raw(original()) }
        .ok_or("Reference original Safari all descriptor absent")?;
    let (_, node) = snapshot(Retained::as_ptr(&descriptor).cast_mut())?;
    if node.kind != code(b"abso") || node.bytes != code(b"all ").to_ne_bytes() {
        return Err("Reference original Safari all descriptor readback differs".into());
    }
    let original_object: unsafe extern "C" fn(usize, usize, &AnyObject, usize, usize, &AnyObject) -> *mut AnyObject =
        unsafe { std::mem::transmute(object) };
    let original_property: unsafe extern "C" fn(usize, usize, &AnyObject) -> *mut AnyObject =
        unsafe { std::mem::transmute(property) };
    // This pinned Main uses inline four-byte Swift strings for these codes.
    let inline = |bytes: &[u8; 4]| u32::from_le_bytes(*bytes) as usize;
    let tag = 0xe400_0000_0000_0000;
    let root = unsafe { Retained::from_raw(make(Node::scalar(NULL, vec![]))?) }
        .ok_or("Reference Safari null descriptor absent")?;
    let id = unsafe { Retained::from_raw(make(Node::int(101))?) }
        .ok_or("Reference Safari window ID descriptor absent")?;
    let window = unsafe { Retained::from_raw(original_object(inline(b"cwin"), tag, &root, inline(b"ID  "), tag, &id)) }
        .ok_or("Reference original Safari window descriptor absent")?;
    let tabs = unsafe { Retained::from_raw(original_object(inline(b"bTab"), tag, &window, inline(b"indx"), tag, &descriptor)) }
        .ok_or("Reference original Safari tab descriptor absent")?;
    let titles = unsafe { Retained::from_raw(original_property(inline(b"pnam"), tag, &tabs)) }
        .ok_or("Reference original Safari title property absent")?;
    let (cap, request) = snapshot(Retained::as_ptr(&titles).cast_mut())?;
    let tabs = field_node(&request, b"from")?;
    let selected = field_node(&tabs, b"seld")?;
    if request.kind != code(b"obj ")
        || field_node(&request, b"want")?.type_code()? != code(b"prop")
        || field_node(&request, b"form")?.type_code()? != code(b"prop")
        || field_node(&request, b"seld")?.type_code()? != code(b"pnam")
        || tabs.kind != code(b"obj ")
        || field_node(&tabs, b"want")?.type_code()? != code(b"bTab")
        || field_node(&tabs, b"form")?.type_code()? != code(b"indx")
        || selected.kind != code(b"abso") || selected.bytes != node.bytes
        || self::window(field_node(&tabs, b"from")?.as_ref())? != 101
    {
        return Err("Reference original Safari title request differs".into());
    }
    // Call the original default send with the actual Swift-built direct
    // object. The assembly permit restricts it to a read-only private reply.
    let before = wire_snapshot(&cap.model, cap.key()?.generation)?;
    let wire = before.transport()?;
    if wire.pid != crate::sdk::process_router::REFERENCE_SAFARI_PID
        || wire.status != 0 || wire.transport_status != 0
    {
        return Err("Reference Safari assembly wire differs".into());
    }
    let mut event = Node::scalar(EVENT, vec![]);
    event.event = Some(Event {
        class: code(b"core"),
        id: code(b"getd"),
        target: Some(Rc::new(Node::scalar(PID, wire.pid.to_ne_bytes().to_vec()))),
        return_id: -1,
        transaction: 0,
    });
    event.params.insert(code(b"----"), request);
    let event = unsafe { Retained::from_raw(make(event)?) }.ok_or("Reference Safari event absent")?;
    let pointer = Retained::as_ptr(&event).cast_mut();
    let caller = send.checked_add(0x40).ok_or("Reference Safari send caller overflow")?;
    snapshot(pointer)?;
    admission.verify_reference_safari_probe_send(caller)?;
    let class = crate::sdk::metadata::registered_class(event.class() as *const AnyClass as usize)?;
    let selected = crate::sdk::metadata::method(class, c"sendEventWithOptions:timeout:error:", false)?
        .ok_or("Reference Safari event send method absent")?;
    if selected.implementation() != self::send as *const () as usize {
        return Err("Reference Safari event send is not the private implementation".into());
    }
    ASSEMBLY_SEND.with(|slot| {
        if slot.get().is_some() {
            return Err("Reference Safari assembly send reentered".to_owned());
        }
        slot.set(Some((pointer as usize, caller)));
        Ok(())
    })?;
    let mut error = 0;
    let reply = unsafe { call_original_safari_send(send, wire.pid, pointer, 0x21013, 1., &mut error) };
    let used = ASSEMBLY_SEND.with(std::cell::Cell::take).is_none();
    if error != 0 {
        #[link(name = "swiftCore")]
        unsafe extern "C" { fn swift_errorRelease(error: usize); }
        unsafe { swift_errorRelease(error) };
        return Err("Reference original Safari send threw".into());
    }
    let reply = unsafe { Retained::from_raw(reply) }.ok_or("Reference original Safari send returned no reply")?;
    if !used {
        return Err("Reference original Safari send did not consume its assembly permit".into());
    }
    let (_, reply) = snapshot(Retained::as_ptr(&reply).cast_mut())?;
    let values = reply.params.get(&code(b"----")).ok_or("Reference Safari title reply absent")?;
    let titles = values.items.iter().map(|item| item.string()).collect::<Result<Vec<_>>>()?;
    if values.kind != LIST || titles != ["Reference Safari fixture", "Initially selected fixture tab"] {
        return Err("Reference Safari title readback differs".into());
    }
    wire_check(&before)?;
    Ok(())
}

fn mutate(p: *mut AnyObject, edit: impl FnOnce(&mut Node) -> Result<()>) -> Result<()> {
    let (cap, node) = snapshot(p)?;
    let mut next = (*node).clone();
    edit(&mut next)?;
    next.size(0)?;
    let address = OwnedDesc::from_node(&next)?;
    let key = cap.key()?;
    let old = VALUES.with(|values| -> Result<OwnedDesc> {
        let mut values = values.borrow_mut();
        let value = values.get_mut(&key).ok_or("descriptor data absent")?;
        if !Rc::ptr_eq(&value.node, &node) {
            return Err("descriptor changed during mutation".into());
        }
        value.node = Rc::new(next);
        Ok(std::mem::replace(&mut value.address, address))
    })?;
    drop(old);
    Ok(())
}
/// Called only by core after resolving the exact retiring descriptor identity.
/// Removes normal data without invoking any unqualified Objective-C destructor.
pub(super) fn release(key: Handle) -> Result<()> {
    let value = VALUES.with(|v| v.borrow_mut().remove(&key));
    drop(value);
    Ok(())
}
fn boundary<T>(fallback: T, body: impl FnOnce() -> Result<T>) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => fallback,
        Err(payload) => {
            std::mem::forget(payload);
            if let Some(installed) = INSTALLED.get() {
                installed.model.fault();
            }
            fallback
        }
    }
}
// Invalid caller values are ordinary nil/scalar failures. They do not retire
// unrelated family state; panics or lost core admission do retire the model.
fn pointer(body: impl FnOnce() -> Result<*mut AnyObject>) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), body)
}
unsafe extern "C" fn null(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    pointer(|| autoreleased(Node::scalar(NULL, vec![])))
}
unsafe extern "C" fn record(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    pointer(|| autoreleased(Node::scalar(RECORD, vec![])))
}
unsafe extern "C" fn list(_: *mut AnyClass, _: Sel) -> *mut AnyObject {
    pointer(|| autoreleased(Node::scalar(LIST, vec![])))
}
unsafe extern "C" fn pid(_: *mut AnyClass, _: Sel, value: i32) -> *mut AnyObject {
    pointer(|| {
        if value <= 0 {
            return Err("invalid owned PID".into());
        }
        autoreleased(Node::scalar(PID, value.to_ne_bytes().to_vec()))
    })
}
unsafe extern "C" fn current_pid(class: *mut AnyClass, selector: Sel) -> *mut AnyObject {
    // Existing pure Rust descriptor oracle references this process only. It
    // performs no foreign application identity lookup or automation operation.
    unsafe { pid(class, selector, std::process::id() as i32) }
}
unsafe extern "C" fn int32(_: *mut AnyClass, _: Sel, value: i32) -> *mut AnyObject {
    pointer(|| autoreleased(Node::int(value)))
}
unsafe extern "C" fn type_code(_: *mut AnyClass, _: Sel, value: u32) -> *mut AnyObject {
    pointer(|| autoreleased(Node::ty(value)))
}
unsafe extern "C" fn enum_code(_: *mut AnyClass, _: Sel, value: u32) -> *mut AnyObject {
    pointer(|| autoreleased(Node::scalar(ENUM, value.to_ne_bytes().to_vec())))
}
unsafe extern "C" fn text(_: *mut AnyClass, _: Sel, value: *mut AnyObject) -> *mut AnyObject {
    pointer(|| {
        model()?.with(|_| Ok(()))?;
        autoreleased(Node::text(&get_string(value)?))
    })
}
fn bytes_node(kind: u32, bytes: *const c_void, length: usize) -> Result<Node> {
    model()?.with(|_| Ok(()))?;
    if length > MAX_BYTES || (length != 0 && bytes.is_null()) {
        return Err("invalid descriptor byte extent".into());
    }
    let bytes = if length == 0 {
        vec![]
    } else {
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }.to_vec()
    };
    Ok(Node::scalar(kind, bytes))
}
unsafe extern "C" fn bytes(
    _: *mut AnyClass,
    _: Sel,
    kind: u32,
    data: *const c_void,
    length: usize,
) -> *mut AnyObject {
    pointer(|| autoreleased(bytes_node(kind, data, length)?))
}
fn data_node(kind: u32, data: *mut AnyObject) -> Result<Node> {
    model()?.with(|_| Ok(()))?;
    if data.is_null() {
        return Ok(Node::scalar(kind, vec![]));
    }
    let data = unsafe { &*data.cast::<NSData>() };
    if !data.isKindOfClass(NSData::class()) || data.length() > MAX_BYTES {
        return Err("invalid owned descriptor NSData".into());
    }
    Ok(Node::scalar(kind, data.to_vec()))
}
unsafe extern "C" fn with_data(
    _: *mut AnyClass,
    _: Sel,
    kind: u32,
    data: *mut AnyObject,
) -> *mut AnyObject {
    pointer(|| autoreleased(data_node(kind, data)?))
}
unsafe extern "C" fn init_data(
    p: *mut AnyObject,
    _: Sel,
    kind: u32,
    data: *mut AnyObject,
) -> *mut AnyObject {
    pointer(|| initialize(p, || data_node(kind, data)))
}
unsafe extern "C" fn init_bytes(
    p: *mut AnyObject,
    _: Sel,
    kind: u32,
    data: *const c_void,
    length: usize,
) -> *mut AnyObject {
    pointer(|| initialize(p, || bytes_node(kind, data, length)))
}
fn event_node(
    class: u32,
    id: u32,
    target: *mut AnyObject,
    return_id: i16,
    transaction: i32,
) -> Result<Node> {
    model()?.with(|_| Ok(()))?;
    let target = if target.is_null() {
        None
    } else {
        Some(snapshot(target)?.1)
    };
    let mut node = Node::scalar(EVENT, vec![]);
    node.event = Some(Event {
        class,
        id,
        target,
        return_id,
        transaction,
    });
    Ok(node)
}
unsafe extern "C" fn event(
    _: *mut AnyClass,
    _: Sel,
    class: u32,
    id: u32,
    target: *mut AnyObject,
    return_id: i16,
    transaction: i32,
) -> *mut AnyObject {
    pointer(|| autoreleased(event_node(class, id, target, return_id, transaction)?))
}
unsafe extern "C" fn init_event(
    p: *mut AnyObject,
    _: Sel,
    class: u32,
    id: u32,
    target: *mut AnyObject,
    return_id: i16,
    transaction: i32,
) -> *mut AnyObject {
    pointer(|| initialize(p, || event_node(class, id, target, return_id, transaction)))
}
unsafe extern "C" fn kind_value(p: *mut AnyObject, _: Sel) -> u32 {
    boundary(0, || Ok(snapshot(p)?.1.kind))
}
unsafe extern "C" fn int_value(p: *mut AnyObject, _: Sel) -> i32 {
    boundary(0, || snapshot(p)?.1.integer())
}
unsafe extern "C" fn type_value(p: *mut AnyObject, _: Sel) -> u32 {
    boundary(0, || snapshot(p)?.1.type_code())
}
unsafe extern "C" fn string_value(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        let out = NSString::from_str(&node.string()?);
        cap.check()?;
        Ok(Retained::autorelease_return(out).cast())
    })
}
unsafe extern "C" fn data_value(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        let bytes = OwnedDesc::from_node(&node)?.data()?;
        let out = NSData::with_bytes(&bytes);
        cap.check()?;
        Ok(Retained::autorelease_return(out).cast())
    })
}
unsafe extern "C" fn address(p: *mut AnyObject, _: Sel) -> *const AddressDesc {
    boundary(std::ptr::null(), || {
        let cap = apple_event_descriptor(p)?;
        let key = cap.key()?;
        cap.check()?;
        VALUES.with(|v| {
            let values = v.borrow();
            let value = values.get(&key).ok_or("descriptor data absent")?;
            Ok(&*value.address.0 as *const AddressDesc)
        })
    })
}
unsafe extern "C" fn count(p: *mut AnyObject, _: Sel) -> isize {
    boundary(0, || {
        let node = snapshot(p)?.1;
        Ok(if node.kind == LIST {
            node.items.len()
        } else {
            node.fields.len()
        } as isize)
    })
}
unsafe extern "C" fn at(p: *mut AnyObject, _: Sel, index: isize) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        let index = usize::try_from(index)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .ok_or("invalid descriptor index")?;
        derived(
            cap,
            (**node.items.get(index).ok_or("missing descriptor index")?).clone(),
        )
    })
}
unsafe extern "C" fn insert(p: *mut AnyObject, _: Sel, item: *mut AnyObject, index: isize) {
    boundary((), || {
        let child = snapshot(item)?.1;
        mutate(p, |node| {
            if node.kind != LIST {
                return Err("not an owned list".into());
            }
            let index = if index == 0 {
                node.items.len()
            } else {
                usize::try_from(index)
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .ok_or("invalid insertion index")?
            };
            if index > node.items.len() {
                return Err("list insertion gap".into());
            }
            node.items.insert(index, child);
            Ok(())
        })
    })
}
unsafe extern "C" fn set_field(p: *mut AnyObject, _: Sel, item: *mut AnyObject, key: u32) {
    boundary((), || {
        let child = snapshot(item)?.1;
        mutate(p, |node| {
            if !matches!(node.kind, RECORD | 0x6f626a20) {
                return Err("not an owned record".into());
            }
            node.fields.insert(key, child);
            Ok(())
        })
    })
}
unsafe extern "C" fn field(p: *mut AnyObject, _: Sel, key: u32) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        match node.fields.get(&key) {
            Some(n) => derived(cap, (**n).clone()),
            None => Ok(std::ptr::null_mut()),
        }
    })
}
unsafe extern "C" fn set_param(p: *mut AnyObject, _: Sel, item: *mut AnyObject, key: u32) {
    boundary((), || {
        let child = snapshot(item)?.1;
        mutate(p, |node| {
            if node.event.is_none() {
                return Err("not an owned event".into());
            }
            node.params.insert(key, child);
            Ok(())
        })
    })
}
unsafe extern "C" fn param(p: *mut AnyObject, _: Sel, key: u32) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        match node.params.get(&key) {
            Some(n) => derived(cap, (**n).clone()),
            None => Ok(std::ptr::null_mut()),
        }
    })
}
unsafe extern "C" fn attribute(p: *mut AnyObject, _: Sel, key: u32) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        let event = node.event.as_ref().ok_or("not an owned event")?;
        match key {
            k if k == code(b"evcl") => derived(cap, Node::ty(event.class)),
            k if k == code(b"evid") => derived(cap, Node::ty(event.id)),
            k if k == code(b"addr") => match &event.target {
                Some(n) => derived(cap, (**n).clone()),
                None => Ok(std::ptr::null_mut()),
            },
            k if k == code(b"rtid") => derived(cap, Node::int(i32::from(event.return_id))),
            k if k == code(b"tran") => derived(cap, Node::int(event.transaction)),
            _ => Ok(std::ptr::null_mut()),
        }
    })
}
unsafe extern "C" fn coerce(p: *mut AnyObject, _: Sel, kind: u32) -> *mut AnyObject {
    pointer(|| {
        let (cap, node) = snapshot(p)?;
        let mut next = (*node).clone();
        if kind != node.kind
            && !(node.kind == RECORD
                && kind == code(b"obj ")
                && [b"want", b"from", b"form", b"seld"]
                    .iter()
                    .all(|k| node.fields.contains_key(&code(k))))
        {
            return Ok(std::ptr::null_mut());
        }
        next.kind = kind;
        derived(cap, next)
    })
}

/// Explicit synthetic wire state, never populated from Safari or the system.
/// No Debug/serialization is implemented, so diagnostics cannot expose content.
#[derive(Clone)]
pub struct Tab {
    pub window: i32,
    pub index: i32,
    pub title: String,
    pub url: String,
}
#[derive(Clone)]
pub struct Wire {
    pub pid: i32,
    pub tabs: Vec<Tab>,
    pub selected: (i32, i32),
    pub status: i32,
    pub transport_status: i32,
    pub sends: u64,
    pub sets: u64,
    pub raises: u64,
}
#[derive(Clone)]
struct WireState {
    model: Arc<Workspace>,
    generation: u64,
    revision: u64,
    wire: Option<Wire>,
    identity:Option<crate::sdk::system_providers::ProcessIdentity>,
}
fn with_wire_identity<T>(identity:Option<crate::sdk::system_providers::ProcessIdentity>,commit:impl FnOnce()->Result<T>)->Result<T> {
    if let Some(identity)=identity {crate::sdk::process_router::with_virtual_identity(identity,commit)} else {commit()}
}
impl WireState {
    fn transport(&self)->Result<&Wire> {self.wire.as_ref().ok_or("owned Safari transport unavailable".into())}
    fn transport_mut(&mut self)->Result<&mut Wire> {self.wire.as_mut().ok_or("owned Safari transport unavailable".into())}
}
// Only plain transport data is shared by the two original serial providers.
// VALUES/Node/Rc and AppleEventHandle retain their creating-thread policy.
static WIRE: Mutex<Option<WireState>> = Mutex::new(None);

// Lock order is always model state -> wire. Every caller supplies only pure
// local reads/writes; dynamic guardians, ARC and descriptor callbacks occur
// before this helper, never while either lock is held.
fn with_wire<T>(
    model: &Arc<Workspace>,
    generation: u64,
    action: impl FnOnce(&mut Option<WireState>) -> Result<T>,
) -> Result<T> {
    let state = model.state.lock().map_err(|_| "workspace state poisoned")?;
    if state.retired || state.generation != generation || model.faulted.load(Ordering::Acquire) {
        return Err("owned wire model/generation retired".into());
    }
    let mut wire = WIRE.lock().map_err(|_| "owned wire poisoned")?;
    action(&mut wire)
}
fn wire_snapshot(model: &Arc<Workspace>, generation: u64) -> Result<WireState> {
    let snapshot=with_wire(model, generation, |state| {
        let current = state.as_ref().ok_or("owned Safari wire fixture absent")?;
        if !Arc::ptr_eq(&current.model, model) || current.generation != generation {
            return Err("owned wire model/generation differs".into());
        }
        current.transport()?;
        Ok(current.clone())
    })?;
    with_wire_identity(snapshot.identity,||Ok(()))?;
    Ok(snapshot)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn activate_reference_safari(
    model: &Arc<Workspace>, handle: Handle, caller: usize, options: usize,
) -> Result<bool> {
    use super::super::callbacks::{SafariEffect, reference_safari_effect};
    if options != 0 {
        return Err("Reference Safari activation options differ".into());
    }
    let stopped = crate::sdk::reference_runtime::verify_safari_effect(model, caller, SafariEffect::Activate)?;
    model.with(|state| {
        let app = state.apps.get(&handle).ok_or("Reference Safari activation target absent")?;
        let wire = WIRE.lock().map_err(|_| "owned wire poisoned")?;
        let wire = wire.as_ref().ok_or("Reference Safari activation wire absent")?;
        let identity = wire.identity.ok_or("Reference Safari activation identity absent")?;
        if !Arc::ptr_eq(model, &wire.model) || wire.generation != state.generation
            || app.terminated || app.bundle.as_deref() != Some("com.apple.Safari")
            || identity.pid != app.pid || wire.transport()?.pid != app.pid
        {
            return Err("Reference Safari activation target differs".into());
        }
        crate::sdk::process_router::with_virtual_identity(identity, || {
            reference_safari_effect(SafariEffect::Activate, stopped, || {
                Workspace::activate_in_state(state, handle)
            })
        })
    })
}

fn same_wire(current: &WireState, before: &WireState) -> bool {
    Arc::ptr_eq(&current.model, &before.model)
        && current.revision == before.revision
        && current.generation == before.generation
        && current.wire.as_ref().map(|wire|wire.pid) == before.wire.as_ref().map(|wire|wire.pid)
        && current.identity==before.identity
}
fn wire_check(before: &WireState) -> Result<()> {
    with_wire(&before.model, before.generation, |state| {
        if !state
            .as_ref()
            .is_some_and(|current| same_wire(current, before))
        {
            return Err("owned wire changed before handoff".into());
        }
        with_wire_identity(before.identity,||Ok(()))
    })
}
pub fn configure(wire: Wire) -> Result<()> {
    let model = INSTALLED
        .get()
        .ok_or("workspace not installed")?
        .model
        .clone();
    configure_for_model(model, wire)
}
fn configure_for_model(model: Arc<Workspace>, wire: Wire) -> Result<()> {
    configure_backend_for_model(model,Some(wire),false)
}
fn configure_backend_for_model(model:Arc<Workspace>,wire:Option<Wire>,initial_only:bool)->Result<()> {
    model.with(|_| Ok(()))?;
    if wire.as_ref().is_some_and(|wire|wire.pid <= 0
        || wire.tabs.len() > MAX_ITEMS
        || wire
            .tabs
            .iter()
            .any(|t| t.window <= 0 || t.index <= 0 || t.title.len() > 8193 || t.url.len() > 8193))
    {
        return Err("invalid owned Safari wire extent".into());
    }
    #[cfg(all(rcf1_reference_sdk,not(test)))]
    let identity=if let Some(wire)=wire.as_ref() {
        validate_reference_wire(&model,Some(wire),0)?;
        Some(crate::sdk::process_router::reference_browser_identity(&model,wire.pid)?)
    } else {None};
    #[cfg(any(not(rcf1_reference_sdk),test))]
    let identity=None;
    let generation = model.with(|s| Ok(s.generation))?;
    let old = with_wire(&model, generation, |state| {
        if initial_only&&state.is_some() {return Err("owned Safari backend already initialized".into());}
        if state.as_ref().is_some_and(|current|
            !Arc::ptr_eq(&current.model,&model)||current.generation!=generation) {
            return Err("owned wire replacement attempted by another model/generation".into());
        }
        let revision = state.as_ref().map_or(Ok(1), |s| {
            s.revision.checked_add(1).ok_or("wire revision exhausted")
        })?;
        with_wire_identity(identity,||Ok(state.replace(WireState {
            model: model.clone(),
            generation,
            revision,
            wire,
            identity,
        })))
    })?;
    drop(old);
    Ok(())
}
#[cfg(any(rcf1_reference_sdk,test))]
fn validate_reference_wire(model:&Arc<Workspace>,wire:Option<&Wire>,disposition:i32)->Result<()> {
    if !matches!(disposition,0|-1743|-1744|-600) {return Err("invalid Reference automation disposition".into());}
    model.with(|_|Ok(()))?;
    let Some(wire)=wire else {return Ok(());};
    if wire.sends!=0||wire.sets!=0||wire.raises!=0
        ||(wire.tabs.is_empty()&&wire.selected!=(0,0))
        ||(!wire.tabs.is_empty()&&wire.tabs.iter().filter(|tab|(tab.window,tab.index)==wire.selected).count()!=1) {
        return Err("Reference Safari fixture has stale counters/selection".into());
    }
    let applications=model.applications()?;
    let matches=applications.iter().filter(|(_,app)|!app.terminated&&app.bundle.as_deref()==Some("com.apple.Safari")).collect::<Vec<_>>();
    if matches.len()!=1||matches[0].1.pid!=wire.pid
        ||!super::super::reference_bundle_metadata_path(&matches[0].1).as_deref().is_some_and(|path|[
            std::path::Path::new("/Applications/Safari.app"),
            std::path::Path::new("/System/Applications/Safari.app"),
            std::path::Path::new("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"),
        ].contains(&path)) {return Err("Reference Safari transport is not the same synthetic Workspace target".into());}
    Ok(())
}
/// Bind plain optional transport/consent only after the real full workspace
/// factory attestation. None preserves an owned unavailable backend, not a PID.
#[cfg(all(rcf1_reference_sdk,not(test)))]
pub(crate) fn initialize_reference_backend(admission:&Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    model:&Arc<Workspace>,factories:&super::super::factory::PublishedWorkspaceFactories,
    wire:Option<Wire>,disposition:i32)->Result<()> {
    struct Failure {model:Arc<Workspace>,security:Option<Arc<crate::sdk::private_network::Context>>,complete:bool}
    impl Drop for Failure {fn drop(&mut self){if !self.complete {
        self.model.fault();
        if let Some(security)=&self.security {security.revoke_backend();}
    }}}
    let mut failure=Failure{model:model.clone(),security:None,complete:false};
    admission.verify_private_security_source_members()?;
    model.verify_reference_context(admission)?;
    validate_reference_wire(model,wire.as_ref(),disposition)?;
    let security=crate::sdk::private_network::automation_workspace_context(model,factories)?;
    failure.security=Some(security.clone());
    security.bind_backend(model)?;
    let target=wire.as_ref().map(|wire|wire.pid);
    if let Some(pid) = target {
        crate::sdk::process_router::register_reference_browser_process(admission, model, pid)?;
    }
    configure_backend_for_model(model.clone(), wire, true)?;
    let identity = if target.is_some() {
        Some(wire_snapshot(model, model.backend_generation()?)?.identity
            .ok_or("Reference Safari wire process identity absent")?)
    } else {
        None
    };
    security.configure_backend_consent(identity, disposition)?;
    admission.verify_private_security_source_members()?;
    model.verify_reference_context(admission)?;
    crate::sdk::private_network::verify_automation_workspace(model,factories)?;
    failure.complete=true;Ok(())
}
fn field_node(node: &Node, key: &[u8; 4]) -> Result<Rc<Node>> {
    node.fields
        .get(&code(key))
        .cloned()
        .ok_or("missing object specifier key".into())
}
fn window(node: &Node) -> Result<i32> {
    if node.kind != code(b"obj ")
        || field_node(node, b"want")?.type_code()? != code(b"cwin")
        || field_node(node, b"form")?.type_code()? != code(b"ID  ")
    {
        return Err("not an explicit owned window".into());
    }
    field_node(node, b"seld")?.integer()
}
fn property_reference(node: &Node) -> Result<(u32, Rc<Node>)> {
    if node.kind != code(b"obj ")
        || field_node(node, b"want")?.type_code()? != code(b"prop")
        || field_node(node, b"form")?.type_code()? != code(b"prop")
    {
        return Err("not an owned property specifier".into());
    }
    Ok((
        field_node(node, b"seld")?.type_code()?,
        field_node(node, b"from")?,
    ))
}
fn list_node(items: Vec<Node>) -> Node {
    let mut value = Node::scalar(LIST, vec![]);
    value.items = items.into_iter().map(Rc::new).collect();
    value
}
fn interpret(node: &Node, wire: &mut Wire) -> Result<Node> {
    let event = node.event.as_ref().ok_or("not an owned AppleEvent")?;
    if event.class != code(b"core")
        || !matches!(event.id, 0x67657464 | 0x73657464)
        || event
            .target
            .as_ref()
            .is_none_or(|target| target.kind != PID)
        || event
            .target
            .as_ref()
            .ok_or("missing owned PID target")?
            .integer()?
            != wire.pid
    {
        return Err("owned event class/id/target mismatch".into());
    }
    let reference = node
        .params
        .get(&code(b"----"))
        .ok_or("owned event lacks direct object")?;
    let (key, parent) = property_reference(reference)?;
    let mut reply = Node::scalar(EVENT, vec![]);
    reply.event = Some(Event {
        class: code(b"aevt"),
        id: code(b"ansr"),
        target: None,
        return_id: 0,
        transaction: 0,
    });
    wire.sends = wire
        .sends
        .checked_add(1)
        .ok_or("owned send count overflow")?;
    if wire.status != 0 {
        reply
            .params
            .insert(code(b"errn"), Rc::new(Node::int(wire.status)));
        return Ok(reply);
    }
    if event.id == code(b"setd") {
        let value = node
            .params
            .get(&code(b"data"))
            .ok_or("owned set lacks data")?;
        let win = window(&parent)?;
        if key == code(b"cTab") {
            if field_node(value, b"want")?.type_code()? != code(b"bTab")
                || field_node(value, b"form")?.type_code()? != code(b"indx")
                || window(&*field_node(value, b"from")?)? != win
            {
                return Err("owned set attempted unsupported navigation".into());
            }
            let index = field_node(value, b"seld")?.integer()?;
            if !wire
                .tabs
                .iter()
                .any(|t| t.window == win && t.index == index)
            {
                return Err("owned selected tab absent".into());
            }
            wire.selected = (win, index);
            wire.sets = wire.sets.checked_add(1).ok_or("owned set count overflow")?;
        } else if key == code(b"pidx") && value.integer()? == 1 {
            if wire.selected.0 != win || !wire.tabs.iter().any(|tab| (tab.window, tab.index) == wire.selected) {
                return Err("owned raise target is not the current Safari window".into());
            }
            wire.raises = wire
                .raises
                .checked_add(1)
                .ok_or("owned raise count overflow")?;
        } else {
            return Err("unsupported owned Safari write".into());
        }
        return Ok(reply);
    }
    let value = if key == code(b"ID  ") {
        if field_node(&parent, b"want")?.type_code()? != code(b"cwin")
            || field_node(&parent, b"form")?.type_code()? != code(b"indx")
            || field_node(&parent, b"seld")?.kind != code(b"abso")
            || field_node(&parent, b"seld")?.bytes != code(b"all ").to_ne_bytes()
        {
            return Err("invalid owned window collection".into());
        }
        let ids: std::collections::BTreeSet<_> = wire.tabs.iter().map(|t| t.window).collect();
        list_node(ids.into_iter().map(Node::int).collect())
    } else if let Ok((selected, win)) = property_reference(&parent) {
        if selected != code(b"cTab") {
            return Err("unsupported owned nested property".into());
        }
        let win = window(&win)?;
        let tab = wire
            .tabs
            .iter()
            .find(|t| (t.window, t.index) == wire.selected && t.window == win)
            .ok_or("owned selected tab missing")?;
        if key == code(b"pidx") {
            Node::int(tab.index)
        } else if key == code(b"pnam") {
            Node::text(&tab.title)
        } else if key == code(b"pURL") {
            Node::text(&tab.url)
        } else {
            return Err("unsupported owned current-tab read".into());
        }
    } else {
        if field_node(&parent, b"want")?.type_code()? != code(b"bTab")
            || field_node(&parent, b"form")?.type_code()? != code(b"indx")
            || field_node(&parent, b"seld")?.kind != code(b"abso")
            || field_node(&parent, b"seld")?.bytes != code(b"all ").to_ne_bytes()
        {
            return Err("invalid owned tab collection".into());
        }
        let win = window(&*field_node(&parent, b"from")?)?;
        let tabs = wire.tabs.iter().filter(|t| t.window == win);
        if key == code(b"pnam") {
            list_node(tabs.map(|t| Node::text(&t.title)).collect())
        } else if key == code(b"pURL") {
            list_node(tabs.map(|t| Node::text(&t.url)).collect())
        } else {
            return Err("unsupported owned tab-list read".into());
        }
    };
    reply.params.insert(code(b"----"), Rc::new(value));
    reply.size(0)?;
    Ok(reply)
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn send(
    p: *mut AnyObject,
    selector: Sel,
    options: usize,
    timeout: f64,
    error: *mut *mut AnyObject,
) -> *mut AnyObject {
    unsafe { send_checked(p, selector, options, timeout, error, 0) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn send(
    _: *mut AnyObject, _: Sel, _: usize, _: f64, _: *mut *mut AnyObject,
) -> *mut AnyObject {
    core::arch::naked_asm!("mov x4, x30", "b {checked}", checked = sym send_checked);
}

unsafe extern "C" fn send_checked(
    p: *mut AnyObject,
    _: Sel,
    options: usize,
    timeout: f64,
    error: *mut *mut AnyObject,
    _caller: usize,
) -> *mut AnyObject {
    pointer(|| {
        if !error.is_aligned() {
            return Err("unaligned owned Safari error output".into());
        }
        let (cap, node) = snapshot(p)?;
        if options != 0x21013 || !timeout.is_finite() || timeout <= 0. || timeout > 8. {
            return Err("unsafe owned send options or timeout".into());
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if ASSEMBLY_SEND.with(std::cell::Cell::get).is_some() {
            if ASSEMBLY_SEND.with(std::cell::Cell::take) != Some((p as usize, _caller))
                || unsafe { libc::pthread_main_np() } != 1
                || node.event.as_ref().is_none_or(|event| event.id != code(b"getd"))
                || timeout != 1.
            {
                return Err("Reference Safari assembly send permit differs".into());
            }
            let before = wire_snapshot(&cap.model, cap.key()?.generation)?;
            let mut wire = before.transport()?.clone();
            if wire.pid != crate::sdk::process_router::REFERENCE_SAFARI_PID
                || wire.status != 0 || wire.transport_status != 0
            {
                return Err("Reference Safari assembly send target differs".into());
            }
            let mut reply = PendingReceiver {
                receiver: make(interpret(&node, &mut wire)?)?,
                committed: false,
            };
            cap.check()?;
            wire_check(&before)?;
            return reply.autorelease();
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let stopped = crate::sdk::reference_runtime::verify_safari_effect(&cap.model, _caller, super::super::callbacks::SafariEffect::Send)?;
        let before = wire_snapshot(&cap.model, cap.key()?.generation)?;
        let status = before.transport()?.transport_status;
        if status != 0 {
            if error.is_null() {
                return Ok(std::ptr::null_mut());
            }
            let failure = unsafe {
                NSError::errorWithDomain_code_userInfo(
                    &NSString::from_str("NSOSStatusErrorDomain"),
                    status as isize,
                    None,
                )
            };
            cap.check()?;
            wire_check(&before)?;
            {
                let failure = Retained::autorelease_return(failure).cast();
                cap.check()?;
                with_wire(&before.model, before.generation, |state| {
                    if !state
                        .as_ref()
                        .is_some_and(|current| same_wire(current, &before))
                    {
                        return Err("owned wire changed during error handoff".into());
                    }
                    with_wire_identity(before.identity, || {
                        let commit = || { unsafe { error.write(failure); } Ok(()) };
                        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                        return crate::workspace::callbacks::reference_safari_effect(super::super::callbacks::SafariEffect::Send, stopped, commit);
                        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
                        commit()
                    })
                })?;
            }
            return Ok(std::ptr::null_mut());
        }
        let mut next = before.clone();
        let reply = interpret(&node, next.transport_mut()?)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("wire revision exhausted")?;
        let mut reply = PendingReceiver {
            receiver: make(reply)?,
            committed: false,
        };
        // No wire mutation, returned reply, or NSError write is committed until
        // the original event's owner survives every intervening callback.
        cap.check()?;
        wire_check(&before)?;
        // This is a pure qualified memory handoff. If the reply itself retired
        // during the final callback, fail before committing any wire effect.
        let returned = reply.autorelease()?;
        let old = with_wire(&before.model, before.generation, |state| {
            let current = state.as_ref().ok_or("owned wire retired")?;
            if !same_wire(current, &before) {
                return Err("owned wire changed during send".into());
            }
            with_wire_identity(before.identity,||{
                let commit = || {
                    let old=state.replace(next);
                    if !error.is_null() { unsafe { error.write(std::ptr::null_mut()); } }
                    Ok(old)
                };
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                return crate::workspace::callbacks::reference_safari_effect(super::super::callbacks::SafariEffect::Send, stopped, commit);
                #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
                commit()
            })
        })?;
        drop(old);
        Ok(returned)
    })
}

macro_rules! method_roster {
    ($consumer:ident)=>{$consumer!{
        (true,c"nullDescriptor",null,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (true,c"recordDescriptor",record,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (true,c"listDescriptor",list,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (true,c"descriptorWithProcessIdentifier:",pid,unsafe extern "C" fn(*mut AnyClass,Sel,i32)->*mut AnyObject),
        (true,c"currentProcessDescriptor",current_pid,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (true,c"descriptorWithInt32:",int32,unsafe extern "C" fn(*mut AnyClass,Sel,i32)->*mut AnyObject),
        (true,c"descriptorWithTypeCode:",type_code,unsafe extern "C" fn(*mut AnyClass,Sel,u32)->*mut AnyObject),
        (true,c"descriptorWithEnumCode:",enum_code,unsafe extern "C" fn(*mut AnyClass,Sel,u32)->*mut AnyObject),
        (true,c"descriptorWithString:",text,unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject)->*mut AnyObject),
        (true,c"descriptorWithDescriptorType:bytes:length:",bytes,unsafe extern "C" fn(*mut AnyClass,Sel,u32,*const c_void,usize)->*mut AnyObject),
        (true,c"descriptorWithDescriptorType:data:",with_data,unsafe extern "C" fn(*mut AnyClass,Sel,u32,*mut AnyObject)->*mut AnyObject),
        (true,c"appleEventWithEventClass:eventID:targetDescriptor:returnID:transactionID:",event,unsafe extern "C" fn(*mut AnyClass,Sel,u32,u32,*mut AnyObject,i16,i32)->*mut AnyObject),
        (false,c"initWithDescriptorType:data:",init_data,unsafe extern "C" fn(*mut AnyObject,Sel,u32,*mut AnyObject)->*mut AnyObject),
        (false,c"initWithDescriptorType:bytes:length:",init_bytes,unsafe extern "C" fn(*mut AnyObject,Sel,u32,*const c_void,usize)->*mut AnyObject),
        (false,c"initWithEventClass:eventID:targetDescriptor:returnID:transactionID:",init_event,unsafe extern "C" fn(*mut AnyObject,Sel,u32,u32,*mut AnyObject,i16,i32)->*mut AnyObject),
        (false,c"descriptorType",kind_value,unsafe extern "C" fn(*mut AnyObject,Sel)->u32),
        (false,c"int32Value",int_value,unsafe extern "C" fn(*mut AnyObject,Sel)->i32),
        (false,c"typeCodeValue",type_value,unsafe extern "C" fn(*mut AnyObject,Sel)->u32),
        (false,c"enumCodeValue",type_value,unsafe extern "C" fn(*mut AnyObject,Sel)->u32),
        (false,c"stringValue",string_value,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"data",data_value,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (false,c"aeDesc",address,unsafe extern "C" fn(*mut AnyObject,Sel)->*const AddressDesc),
        (false,c"numberOfItems",count,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        (false,c"descriptorAtIndex:",at,unsafe extern "C" fn(*mut AnyObject,Sel,isize)->*mut AnyObject),
        (false,c"insertDescriptor:atIndex:",insert,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,isize)),
        (false,c"setDescriptor:forKeyword:",set_field,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,u32)),
        (false,c"descriptorForKeyword:",field,unsafe extern "C" fn(*mut AnyObject,Sel,u32)->*mut AnyObject),
        (false,c"setParamDescriptor:forKeyword:",set_param,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,u32)),
        (false,c"paramDescriptorForKeyword:",param,unsafe extern "C" fn(*mut AnyObject,Sel,u32)->*mut AnyObject),
        (false,c"attributeDescriptorForKeyword:",attribute,unsafe extern "C" fn(*mut AnyObject,Sel,u32)->*mut AnyObject),
        (false,c"coerceToDescriptorType:",coerce,unsafe extern "C" fn(*mut AnyObject,Sel,u32)->*mut AnyObject),
        (false,c"sendEventWithOptions:timeout:error:",send,unsafe extern "C" fn(*mut AnyObject,Sel,usize,f64,*mut *mut AnyObject)->*mut AnyObject)
    }};
}
macro_rules! export_roster {
    ($(($meta:tt,$selector:expr,$function:ident,$ty:ty)),*$(,)?)=>{
        pub(super) fn replacements()->Vec<(&'static CStr,&'static CStr,bool,usize)>{vec![$((c"NSAppleEventDescriptor",$selector,$meta,$function as *const () as usize)),*]}
        pub(super) fn signatures()->Vec<(&'static CStr,bool,Vec<objc2::encode::Encoding>)>{vec![$(($selector,$meta,signature::<$ty>())),*]}
        #[cfg(test)]
        pub(super) fn add_methods(builder:&mut objc2::runtime::ClassBuilder){$(add_method!(builder,$meta,$selector,$function,$ty);)*}
    };
}
fn signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut types = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
    types.extend_from_slice(F::Arguments::ENCODINGS);
    types
}
#[cfg(test)]
macro_rules! add_method {
    ($builder:ident,true,$selector:expr,$function:ident,$ty:ty) => {
        unsafe { $builder.add_class_method(Sel::register($selector), $function as $ty) }
    };
    ($builder:ident,false,$selector:expr,$function:ident,$ty:ty) => {
        unsafe { $builder.add_method(Sel::register($selector), $function as $ty) }
    };
}
method_roster!(export_roster);

#[cfg(test)]
pub(super) fn prove_fixture_descriptors(class: &AnyClass) -> Result<()> {
    use objc2::msg_send;
    prove_normal_arc_handoff(class)?;
    let descriptor: *mut AnyObject =
        unsafe { msg_send![class,descriptorWithProcessIdentifier:4242_i32] };
    let descriptor_type: u32 = unsafe { msg_send![descriptor, descriptorType] };
    if descriptor_type != PID {
        return Err("selected owned PID constructor failed".into());
    }
    let raw: *const AddressDesc = unsafe { msg_send![descriptor, aeDesc] };
    if raw.is_null() || unsafe { AEGetDescDataSize(raw) } != 4 {
        return Err("selected owned PID descriptor is not genuine AE data".into());
    }
    let mut actual_pid = 0_i32;
    if unsafe { AEGetDescData(raw, (&mut actual_pid as *mut i32).cast(), 4) } != 0
        || actual_pid != 4242
    {
        return Err("owned PID descriptor bytes changed".into());
    }
    let list: *mut AnyObject = unsafe { msg_send![class, listDescriptor] };
    let title = NSString::from_str("Owned title");
    let item: *mut AnyObject = unsafe { msg_send![class,descriptorWithString:&*title] };
    unsafe {
        let _: () = msg_send![list,insertDescriptor:item,atIndex:1_isize];
    }
    let count: isize = unsafe { msg_send![list, numberOfItems] };
    if count != 1 {
        return Err("selected owned list insertion failed".into());
    }
    let copy: *mut AnyObject = unsafe { msg_send![list,descriptorAtIndex:1_isize] };
    let text: *mut AnyObject = unsafe { msg_send![copy, stringValue] };
    if get_string(text)? != "Owned title" {
        return Err("selected owned string roundtrip failed".into());
    }
    if snapshot(std::ptr::dangling_mut::<AnyObject>()).is_ok() {
        return Err("unknown descriptor receiver admitted".into());
    }
    let oversized = list_node((0..MAX_ITEMS).map(|_| Node::int(1)).collect());
    oversized.size(0)?;
    if list_node((0..=MAX_ITEMS).map(|_| Node::int(1)).collect())
        .size(0)
        .is_ok()
    {
        return Err("owned descriptor list storage is unbounded".into());
    }
    let before = VALUES.with(|v| v.borrow().len());
    let fake = std::ptr::null_mut::<AnyObject>();
    if unsafe { bind(fake, Node::int(7)) }.is_ok() || VALUES.with(|v| v.borrow().len()) != before {
        return Err("failed initializer retained leaf data".into());
    }
    PREPARE_FAILURE.with(|s| s.set(1));
    let rejected = make(Node::int(7));
    PREPARE_FAILURE.with(|s| s.set(0));
    if rejected.is_ok() || VALUES.with(|v| v.borrow().len()) != before {
        return Err("prepared-data error rollback retained data".into());
    }
    PREPARE_FAILURE.with(|s| s.set(2));
    let panicked = catch_unwind(AssertUnwindSafe(|| make(Node::int(7))));
    PREPARE_FAILURE.with(|s| s.set(0));
    if panicked.is_ok() || VALUES.with(|v| v.borrow().len()) != before {
        return Err("prepared-data panic rollback retained data".into());
    }
    configure(test_wire())?;
    prove_selected_send(class, descriptor, None)?;
    let raw = unsafe { crate::workspace::factory::allocate(class) }?;
    let all = code(b"all ").to_ne_bytes();
    let data = NSData::with_bytes(&all);
    let initialized: *mut AnyObject =
        unsafe { msg_send![raw,initWithDescriptorType:code(b"abso"),data:&*data] };
    if initialized != raw || snapshot(initialized)?.1.bytes != all {
        return Err("selected source data initializer failed".into());
    }
    let raw = unsafe { crate::workspace::factory::allocate(class) }?;
    let initialized_event: *mut AnyObject = unsafe {
        msg_send![raw,initWithEventClass:code(b"core"),eventID:code(b"getd"),targetDescriptor:descriptor,returnID:-1_i16,transactionID:0_i32]
    };
    if initialized_event != raw
        || snapshot(initialized_event)?
            .1
            .event
            .as_ref()
            .is_none_or(|e| e.class != code(b"core") || e.id != code(b"getd"))
    {
        return Err("selected source event initializer failed".into());
    }
    retire_apple_event_descriptor(initialized, release)?;
    retire_apple_event_descriptor(initialized_event, release)?;
    // Init returns an owned +1; explicit identity retirement does not consume it.
    unsafe { release_apple_event_descriptor(initialized) }?;
    unsafe { release_apple_event_descriptor(initialized_event) }?;
    // Exercise the selected IMPs against the same process tuple checked by
    // transport publication/commit, rather than testing only plain Wire data.
    // This ordinary fixture grants no Reference source admission.
    let monitor = Arc::new(crate::sdk::system_providers::Fixture::new(model()?.context.clone())?);
    {
        let mut rows = monitor.processes.lock().map_err(|_| "monitor rows poisoned")?;
        let mut row = rows.remove(&730001).ok_or("monitor fixture row absent")?;
        row.pid = 4242;
        rows.insert(row.pid, row);
    }
    monitor.clone().install()?;
    let class_address = class as *const AnyClass as usize;
    let foreign = descriptor as usize;
    std::thread::spawn(move || {
        objc2::rc::autoreleasepool(|_| -> Result<()> {
            let class = unsafe { &*(class_address as *const AnyClass) };
            if snapshot(foreign as *mut AnyObject).is_ok() {
                return Err("foreign-thread descriptor was admitted".into());
            }
            let descriptor: *mut AnyObject =
                unsafe { msg_send![class,descriptorWithProcessIdentifier:4242_i32] };
            configure(test_wire())?;
            let identity = monitor.processes.lock().map_err(|_| "monitor rows poisoned")?
                .get(&4242).ok_or("monitor fixture row absent")?.identity();
            WIRE.lock().map_err(|_| "owned wire poisoned")?.as_mut()
                .ok_or("owned wire absent")?.identity = Some(identity);
            prove_selected_send(class, descriptor, Some(&monitor))?;
            retire_apple_event_descriptor(descriptor, release)?;
            Ok(())
        })
    })
    .join()
    .map_err(|_| "owned descriptor worker panicked")??;
    let cap = apple_event_descriptor(descriptor)?;
    let key = cap.key()?;
    retire_apple_event_descriptor(descriptor, release)?;
    if cap.check().is_ok()
        || VALUES.with(|v| v.borrow().contains_key(&key))
        || !unsafe { address(descriptor, Sel::register(c"aeDesc")) }.is_null()
    {
        return Err("retired descriptor data remained accessible".into());
    }
    Ok(())
}
#[cfg(test)]
fn prove_normal_arc_handoff(class: &AnyClass) -> Result<()> {
    use objc2::{msg_send, rc::autoreleasepool};
    let before = VALUES.with(|values| values.borrow().len());
    let allocations = crate::workspace::factory::live_owned_allocations(class)?;
    let (pointer, retained) = autoreleasepool(|_| -> Result<_> {
        let pointer: *mut AnyObject = unsafe { msg_send![class,descriptorWithInt32:7_i32] };
        let retained = unsafe { Retained::retain(pointer) }.ok_or("owned class result is nil")?;
        Ok((pointer, retained))
    })?;
    if snapshot(pointer)?.1.integer()? != 7 {
        return Err("retained descriptor did not survive its autorelease pool".into());
    }
    drop(retained);
    if apple_event_descriptor(pointer).is_ok()
        || VALUES.with(|values| values.borrow().len()) != before
        || crate::workspace::factory::live_owned_allocations(class)? != allocations
    {
        return Err("last descriptor owner did not reclaim identity and data".into());
    }
    let initialized = autoreleasepool(|_| -> Result<*mut AnyObject> {
        let allocated = unsafe { crate::workspace::factory::allocate(class) }?;
        let number = 7_i32;
        let initialized: *mut AnyObject = unsafe {
            msg_send![allocated,initWithDescriptorType:LONG,bytes:(&number as *const i32).cast::<c_void>(),length:4_usize]
        };
        if initialized != allocated {
            return Err("owned initializer changed its receiver".into());
        }
        Ok(initialized)
    })?;
    if snapshot(initialized)?.1.integer()? != 7 {
        return Err("owned initializer result was autoreleased".into());
    }
    unsafe { release_apple_event_descriptor(initialized) }?;
    if apple_event_descriptor(initialized).is_ok()
        || VALUES.with(|values| values.borrow().len()) != before
        || crate::workspace::factory::live_owned_allocations(class)? != allocations
    {
        return Err("initializer owned reference was not reclaimed".into());
    }
    let allocated = unsafe { crate::workspace::factory::allocate(class) }?;
    let rejected: *mut AnyObject = unsafe {
        msg_send![allocated,initWithDescriptorType:LONG,bytes:std::ptr::null::<c_void>(),length:MAX_BYTES+1]
    };
    if !rejected.is_null()
        || check_apple_event_initializer(allocated).is_ok()
        || VALUES.with(|values| values.borrow().len()) != before
        || crate::workspace::factory::live_owned_allocations(class)? != allocations
    {
        return Err("failed initializer did not consume its allocated reference".into());
    }
    Ok(())
}
#[cfg(test)]
fn test_wire() -> Wire {
    Wire {
        pid: 4242,
        tabs: vec![
            Tab {
                window: 101,
                index: 1,
                title: "Owned title".into(),
                url: "https://example.test/owned".into(),
            },
            Tab {
                window: 101,
                index: 2,
                title: "Owned second title".into(),
                url: "https://example.test/second".into(),
            },
        ],
        selected: (101, 1),
        status: 0,
        transport_status: 0,
        sends: 0,
        sets: 0,
        raises: 0,
    }
}

#[cfg(test)]
fn prove_selected_send(class: &AnyClass, target: *mut AnyObject, monitor: Option<&Arc<crate::sdk::system_providers::Fixture>>) -> Result<()> {
    use objc2::msg_send;
    fn object(
        class: &AnyClass,
        want: u32,
        from: *mut AnyObject,
        form: u32,
        key: *mut AnyObject,
    ) -> Result<*mut AnyObject> {
        let record: *mut AnyObject = unsafe { msg_send![class, recordDescriptor] };
        let want: *mut AnyObject = unsafe { msg_send![class,descriptorWithTypeCode:want] };
        let form: *mut AnyObject = unsafe { msg_send![class,descriptorWithEnumCode:form] };
        for (key, value) in [
            (code(b"want"), want),
            (code(b"from"), from),
            (code(b"form"), form),
            (code(b"seld"), key),
        ] {
            unsafe {
                let _: () = msg_send![record,setDescriptor:value,forKeyword:key];
            }
        }
        let out: *mut AnyObject = unsafe { msg_send![record,coerceToDescriptorType:code(b"obj ")] };
        if out.is_null() {
            return Err("selected owned object coercion failed".into());
        }
        Ok(out)
    }
    fn perform(
        class: &AnyClass,
        target: *mut AnyObject,
        id: u32,
        reference: *mut AnyObject,
        value: Option<*mut AnyObject>,
    ) -> Result<*mut AnyObject> {
        let event: *mut AnyObject = unsafe {
            msg_send![class,appleEventWithEventClass:code(b"core"),eventID:id,targetDescriptor:target,returnID:-1_i16,transactionID:0_i32]
        };
        unsafe {
            let _: () = msg_send![event,setParamDescriptor:reference,forKeyword:code(b"----")];
        }
        if let Some(value) = value {
            unsafe {
                let _: () = msg_send![event,setParamDescriptor:value,forKeyword:code(b"data")];
            }
        }
        let mut error = std::ptr::null_mut::<AnyObject>();
        let reply: *mut AnyObject = unsafe {
            msg_send![event,sendEventWithOptions:0x21013_usize,timeout:1_f64,error:&mut error]
        };
        if reply.is_null() || !error.is_null() {
            return Err("owned selected request failed".into());
        }
        Ok(reply)
    }
    let null: *mut AnyObject = unsafe { msg_send![class, nullDescriptor] };
    let index: *mut AnyObject = unsafe { msg_send![class,descriptorWithInt32:101_i32] };
    let win = object(class, code(b"cwin"), null, code(b"ID  "), index)?;
    let all = code(b"all ");
    let all: *mut AnyObject = unsafe {
        msg_send![class,descriptorWithDescriptorType:code(b"abso"),bytes:(&all as *const u32).cast::<c_void>(),length:4_usize]
    };
    let tabs = object(class, code(b"bTab"), win, code(b"indx"), all)?;
    let title_key: *mut AnyObject =
        unsafe { msg_send![class,descriptorWithTypeCode:code(b"pnam")] };
    let title_ref = object(class, code(b"prop"), tabs, code(b"prop"), title_key)?;
    let event: *mut AnyObject = unsafe {
        msg_send![class,appleEventWithEventClass:code(b"core"),eventID:code(b"getd"),targetDescriptor:target,returnID:-1_i16,transactionID:0_i32]
    };
    unsafe {
        let _: () = msg_send![event,setParamDescriptor:title_ref,forKeyword:code(b"----")];
    }
    let attr: *mut AnyObject =
        unsafe { msg_send![event,attributeDescriptorForKeyword:code(b"evcl")] };
    let event_class: u32 = unsafe { msg_send![attr, typeCodeValue] };
    if event_class != code(b"core") {
        return Err("selected event attributes changed".into());
    }
    let mut error = std::ptr::dangling_mut::<AnyObject>();
    let reply: *mut AnyObject = unsafe {
        msg_send![event,sendEventWithOptions:0x21013_usize,timeout:1_f64,error:&mut error]
    };
    if reply.is_null() || !error.is_null() {
        return Err("selected owned send failed".into());
    }
    let result: *mut AnyObject =
        unsafe { msg_send![reply,paramDescriptorForKeyword:code(b"----")] };
    let item: *mut AnyObject = unsafe { msg_send![result,descriptorAtIndex:1_isize] };
    let text: *mut AnyObject = unsafe { msg_send![item, stringValue] };
    if get_string(text)? != "Owned title" {
        return Err("selected owned Safari list reply changed".into());
    }
    let current_key: *mut AnyObject =
        unsafe { msg_send![class,descriptorWithTypeCode:code(b"cTab")] };
    let current_ref = object(class, code(b"prop"), win, code(b"prop"), current_key)?;
    let second: *mut AnyObject = unsafe { msg_send![class,descriptorWithInt32:2_i32] };
    let second = object(class, code(b"bTab"), win, code(b"indx"), second)?;
    perform(class, target, code(b"setd"), current_ref, Some(second))?;
    let index_key: *mut AnyObject =
        unsafe { msg_send![class,descriptorWithTypeCode:code(b"pidx")] };
    let selected_index = object(class, code(b"prop"), current_ref, code(b"prop"), index_key)?;
    let readback = perform(class, target, code(b"getd"), selected_index, None)?;
    let value: *mut AnyObject =
        unsafe { msg_send![readback,paramDescriptorForKeyword:code(b"----")] };
    let index: i32 = unsafe { msg_send![value, int32Value] };
    if index != 2 {
        return Err("owned set did not survive selected readback".into());
    }
    for (key, expected) in [
        (code(b"pnam"), "Owned second title"),
        (code(b"pURL"), "https://example.test/second"),
    ] {
        let key: *mut AnyObject = unsafe { msg_send![class,descriptorWithTypeCode:key] };
        let reference = object(class, code(b"prop"), current_ref, code(b"prop"), key)?;
        let reply = perform(class, target, code(b"getd"), reference, None)?;
        let result: *mut AnyObject =
            unsafe { msg_send![reply,paramDescriptorForKeyword:code(b"----")] };
        let text: *mut AnyObject = unsafe { msg_send![result, stringValue] };
        if get_string(text)? != expected {
            return Err("owned selected metadata readback changed".into());
        }
    }
    let front_ref = object(class, code(b"prop"), win, code(b"prop"), index_key)?;
    let one: *mut AnyObject = unsafe { msg_send![class,descriptorWithInt32:1_i32] };
    perform(class, target, code(b"setd"), front_ref, Some(one))?;
    let committed = {
        let state = WIRE.lock().map_err(|_| "owned wire poisoned")?;
        let wire = state.as_ref().ok_or("owned wire absent")?.transport()?;
        wire.selected == (101, 2) && wire.sets == 1 && wire.raises == 1
    };
    if !committed {
        return Err("owned set/raise committed incorrect effects".into());
    }
    let sends = WIRE
        .lock()
        .map_err(|_| "owned wire poisoned")?
        .as_ref()
        .ok_or("owned wire absent")?
        .transport()?
        .sends;
    let absent_id: *mut AnyObject = unsafe { msg_send![class,descriptorWithInt32:999_i32] };
    let absent_window = object(class, code(b"cwin"), null, code(b"ID  "), absent_id)?;
    let absent_front = object(class, code(b"prop"), absent_window, code(b"prop"), index_key)?;
    if perform(class, target, code(b"setd"), absent_front, Some(one)).is_ok() {
        return Err("owned raise accepted an absent Safari window".into());
    }
    for (flags, timeout) in [
        (0_usize, 1.),
        (0x21013, 0.),
        (0x21013, f64::NAN),
        (0x21013, 9.),
    ] {
        error = std::ptr::dangling_mut::<AnyObject>();
        let denied: *mut AnyObject =
            unsafe { msg_send![event,sendEventWithOptions:flags,timeout:timeout,error:&mut error] };
        if !denied.is_null() || error != std::ptr::dangling_mut::<AnyObject>() {
            return Err("unsafe send wrote outputs".into());
        }
    }
    let mut error_storage = [usize::MAX; 2];
    let misaligned = unsafe { error_storage.as_mut_ptr().cast::<u8>().add(1).cast::<*mut AnyObject>() };
    let denied: *mut AnyObject = unsafe {
        msg_send![event,sendEventWithOptions:0x21013_usize,timeout:1_f64,error:misaligned]
    };
    if !denied.is_null() || error_storage != [usize::MAX; 2] {
        return Err("misaligned Safari error slot received output".into());
    }
    if WIRE
        .lock()
        .map_err(|_| "owned wire poisoned")?
        .as_ref()
        .ok_or("owned wire absent")?
        .transport()?
        .sends
        != sends
    {
        return Err("refused Safari send changed wire state".into());
    }
    WIRE.lock()
        .map_err(|_| "owned wire poisoned")?
        .as_mut()
        .ok_or("owned wire absent")?
        .transport_mut()?
        .transport_status = -1743;
    error = std::ptr::null_mut();
    let denied: *mut AnyObject = unsafe {
        msg_send![event,sendEventWithOptions:0x21013_usize,timeout:1_f64,error:&mut error]
    };
    if !denied.is_null()
        || error.is_null()
        || unsafe { (&*error.cast::<NSError>()).code() } != -1743
    {
        return Err("owned NSError permission result changed".into());
    }
    WIRE.lock()
        .map_err(|_| "owned wire poisoned")?
        .as_mut()
        .ok_or("owned wire absent")?
        .transport_mut()?
        .transport_status = 0;
    let Some(monitor) = monitor else { return Ok(()) };
    let effects = {
        let state = WIRE.lock().map_err(|_| "owned wire poisoned")?;
        let wire = state.as_ref().ok_or("owned wire absent")?.transport()?;
        (wire.sends, wire.sets, wire.raises, wire.selected)
    };
    monitor.processes.lock().map_err(|_| "monitor rows poisoned")?
        .get_mut(&4242).ok_or("monitor fixture row absent")?.started_seconds += 1;
    error = std::ptr::dangling_mut();
    let denied: *mut AnyObject = unsafe {
        msg_send![event,sendEventWithOptions:0x21013_usize,timeout:1_f64,error:&mut error]
    };
    if !denied.is_null() || error != std::ptr::dangling_mut() {
        return Err("replaced process escaped selected send or wrote NSError output".into());
    }
    let state = WIRE.lock().map_err(|_| "owned wire poisoned")?;
    let wire = state.as_ref().ok_or("owned wire absent")?.transport()?;
    if (wire.sends, wire.sets, wire.raises, wire.selected) != effects {
        return Err("replaced process committed selected send effects".into());
    }
    Ok(())
}
