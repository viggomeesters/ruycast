//! Fixed AppDelegate construction DATA. This is no ARC or initializer permit.
use crate::sdk::core_preentry::{ReferenceAppDelegateContext,SwiftSingletonDataKind,SwiftSingletonRegionObservation};
use crate::sdk::imports::{observe_unpatched_main_imports,UnpatchedImportObservation};
use crate::sdk::owned::Result;
use sha2::{Digest,Sha256};
use std::{ffi::CStr,io::Read,os::unix::fs::{MetadataExt,OpenOptionsExt},sync::{Arc,atomic::{AtomicBool,Ordering}}};

// These are the actual C globals, not pointer variables. The declaration
// matches the captured target headers; only Array.body.count is ever read.
#[repr(C)]
struct HeapHeader {opaque:[usize;2]}
#[repr(C)]
struct EmptyArrayStorage {header:HeapHeader,count:isize,capacity_flags:usize}
#[repr(C)]
struct SetBody {count:isize,capacity:isize,scale:i8,reserved_scale:i8,extra:i16,age:i32,seed:isize,elements:*mut libc::c_void}
#[repr(C)]
struct EmptySetStorage {header:HeapHeader,body:SetBody,metadata:usize}
const _:()={assert!(std::mem::size_of::<EmptyArrayStorage>()==32);assert!(std::mem::align_of::<EmptyArrayStorage>()==8);
    assert!(std::mem::size_of::<EmptySetStorage>()==64);assert!(std::mem::align_of::<EmptySetStorage>()==8);
    assert!(std::mem::offset_of!(EmptyArrayStorage,count)==16);};
#[link(name="swiftCore")]
unsafe extern "C" {
    static mut _swiftEmptyArrayStorage:EmptyArrayStorage;
    static mut _swiftEmptySetSingleton:EmptySetStorage;
}
#[path="workspace_appdelegate_declarations.rs"]
mod declarations;
use declarations::DECLARATIONS;
fn verify_declarations()->Result<()> {
    for row in &DECLARATIONS {
        if format!("{:x}",Sha256::digest(row.bytes))!=row.sha {return Err("AppDelegate compiled Swift declaration differs".into());}
        let mut file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK).open(row.path).map_err(|e|e.to_string())?;
        let before=file.metadata().map_err(|e|e.to_string())?;
        if !before.is_file()||before.uid()!=0||before.mode()&0o022!=0||before.nlink()!=1
            ||before.len()!=row.bytes.len() as u64||row.bytes.len()>16*1024*1024 {return Err("AppDelegate Swift declaration file differs".into());}
        let mut bytes=Vec::with_capacity(row.bytes.len()+1);
        (&mut file).take(row.bytes.len() as u64+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        let after=file.metadata().map_err(|e|e.to_string())?;
        let path=std::fs::symlink_metadata(row.path).map_err(|e|e.to_string())?;
        let identity=|m:&std::fs::Metadata|(m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
        if identity(&before)!=identity(&after)||identity(&before)!=identity(&path)||bytes!=row.bytes {
            return Err("AppDelegate Swift declaration changed during read".into());
        }
    }
    Ok(())
}
/// Read-only fixed globals and their exact original Main import cells. Public
/// observation yields no class, receiver, constructor, root or ARC capability.
pub struct AppDelegateSingletonData {
    context:Arc<ReferenceAppDelegateContext>,
    set:SwiftSingletonRegionObservation,
    array:SwiftSingletonRegionObservation,
    set_import:UnpatchedImportObservation,
    array_import:UnpatchedImportObservation,
    retired:AtomicBool,
}
impl AppDelegateSingletonData {
    pub fn observe(context:Arc<ReferenceAppDelegateContext>)->Result<Self> {
        context.cleanup_metadata_only()?;
        verify_declarations()?;
        let array_address=std::ptr::addr_of!(_swiftEmptyArrayStorage) as usize;
        let set_address=std::ptr::addr_of!(_swiftEmptySetSingleton) as usize;
        let array=context.observe_swift_singleton_region(SwiftSingletonDataKind::EmptyArray,array_address)?;
        let set=context.observe_swift_singleton_region(SwiftSingletonDataKind::EmptySet,set_address)?;
        let rows=observe_unpatched_main_imports(&[(c"_swiftEmptyArrayStorage",array_address),(c"_swiftEmptySetSingleton",set_address)])?;
        let mut array_import=None;let mut set_import=None;
        for row in rows {
            match row.symbol.to_bytes() {
                b"_swiftEmptyArrayStorage" if array_import.is_none()=>array_import=Some(row),
                b"_swiftEmptySetSingleton" if set_import.is_none()=>set_import=Some(row),
                _=>return Err("AppDelegate singleton import roster differs".into()),
            }
        }
        let value=Self{context,set,array,set_import:set_import.ok_or("EmptySet Main import absent")?,array_import:array_import.ok_or("EmptyArray Main import absent")?,retired:AtomicBool::new(false)};
        value.verify()?;Ok(value)
    }
    fn verify_region(&self,kind:SwiftSingletonDataKind,expected:&SwiftSingletonRegionObservation)->Result<()> {
        let current=self.context.observe_swift_singleton_region(kind,expected.address())?;
        if &current!=expected {return Err("AppDelegate singleton data image changed".into());}
        Ok(())
    }
    fn verify_import(row:&UnpatchedImportObservation,symbol:&CStr,address:usize)->Result<()> {
        if row.symbol!=symbol||row.original_address()!=address {return Err("AppDelegate singleton symbol identity changed".into());}
        row.verify_unpatched()
    }
    pub fn verify(&self)->Result<()> {
        self.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire) {return Err("AppDelegate singleton observation retired".into());}
        let result=(|| {
            verify_declarations()?;
            self.verify_region(SwiftSingletonDataKind::EmptyArray,&self.array)?;
            self.verify_region(SwiftSingletonDataKind::EmptySet,&self.set)?;
            Self::verify_import(&self.array_import,c"_swiftEmptyArrayStorage",self.array.address())?;
            Self::verify_import(&self.set_import,c"_swiftEmptySetSingleton",self.set.address())?;
            // Actual specialized native getCount source: reject the high-tag
            // branch and require the masked base to remain this named global.
            let address=self.array.address();
            if address>>62!=0||(address&0x0ffffffffffffff8)!=address||!address.is_multiple_of(8) {return Err("AppDelegate EmptyArray native branch differs".into());}
            let count=unsafe{((address+16) as *const isize).read_volatile()};
            if count!=0 {return Err("AppDelegate original Array is not empty".into());}
            self.verify_region(SwiftSingletonDataKind::EmptyArray,&self.array)?;
            self.verify_region(SwiftSingletonDataKind::EmptySet,&self.set)?;
            Self::verify_import(&self.array_import,c"_swiftEmptyArrayStorage",address)?;
            Self::verify_import(&self.set_import,c"_swiftEmptySetSingleton",self.set.address())?;
            let after=unsafe{((address+16) as *const isize).read_volatile()};
            if after!=count||self.retired.load(Ordering::Acquire) {return Err("AppDelegate empty branch changed during verification".into());}
            Ok(())
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    /// Equality for the original initialized field word. The caller still
    /// proves known +1, exact receiver/class/serial, seven-field source layout
    /// and both before/after field snapshots. No Set ARC permission is issued.
    pub fn verify_initialized_word(&self,word:usize)->Result<()> {
        self.verify()?;
        if word!=self.set.address(){self.retired.store(true,Ordering::Release);return Err("AppDelegate subscriptions is not qualified empty Set".into());}
        Ok(())
    }
}

#[cfg(target_arch="aarch64")]
#[path="workspace_appdelegate_constructor.rs"]
pub(crate) mod constructor;
