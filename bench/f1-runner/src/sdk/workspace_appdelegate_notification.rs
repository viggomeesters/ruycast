//! Fixed Notification provider and complete compiler-owned local-value route.
//! Rust never invokes Swift, loads an opaque value or calls a VWT destroy.
use super::*;
use crate::sdk::core_preentry::{FoundationNotificationAddressPoint,FoundationNotificationMetadataObservation};

pub(super) struct NotificationValueWitnessData {
    context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>,
    accessor:metadata::FoundationNotificationAccessorReturn,
    point:FoundationNotificationAddressPoint,
    observed:FoundationNotificationMetadataObservation,
    metadata_import:crate::sdk::imports::UnpatchedImportObservation,
    bridge_import:crate::sdk::imports::UnpatchedImportObservation,
    bridge_code:metadata::CodeIdentity,
    retired:AtomicBool,
}
impl NotificationValueWitnessData {
    /// Consumed only by the complete callback authority, which separately
    /// qualifies runtime imports, nonfinal self ARC and the actual sender.
    /// Its selected caller holds the owned payload +1 through the last pure
    /// payload check and synchronous C-unwind dispatch.
    pub(super) fn verify_compiler_value_lifetime(&self)->Result<()> {
        let result=(|| {
            self.verify()?;
            let pins=super::pins::PINS;
            let shared=pins.iter().find(|pin|pin.role=="notification_shared_CTo")
                .ok_or("Complete Notification compiler thunk absent")?;
            if !super::pins::notification_value_lifetime(shared) {
                return Err("Notification compiler-owned bridge/body/destroy sequence differs".into());
            }
            let base=self.context.observe_app_delegate_offset_cells()?.main_header();
            let rows=super::pins::ROWS.iter().filter(|row|row.notification).collect::<Vec<_>>();
            if rows.len()!=2 {return Err("Notification original entry pair differs".into());}
            for row in rows {
                let entry=&pins[row.entry];let body=&pins[row.body];
                if !super::pins::notification_entry_body(entry,body) {
                    return Err("Notification selected original entry/body differs".into());
                }
                for pin in [entry,body] {
                    self.context.verify_main_code(base.checked_add(pin.offset)
                        .ok_or("Notification entry/body address overflow")?,pin.bytes,pin.sha)?;
                }
            }
            // Whole caller, both ownership adapters and final register restore.
            for role in ["notification_shared_CTo","notification_retain_self_tail",
                "notification_postbody_objc_release_tail","notification_restore_registers"] {
                let pin=pins.iter().find(|pin|pin.role==role).ok_or("Notification compiler lifetime member absent")?;
                self.context.verify_main_code(base.checked_add(pin.offset)
                    .ok_or("Notification compiler lifetime address overflow")?,pin.bytes,pin.sha)?;
            }
            // The bridge and selected VWT destroy belong to this SAME genuine
            // Foundation metadata/type/provider. The fixed original callbacks
            // do not use/escape their Notification argument. Parameter payload
            // and its children remain retained by the existing SDK record.
            self.verify()
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    /// Qualify just the constant MetadataResponse call inside the pinned CTo.
    /// Rust does not call Swift or receive an opaque Notification value here.
    pub(super) fn verify_metadata_transition(&self,pc:usize,target:usize)->Result<()> {
        if pc!=0x1fd564||target!=0x2f8c24 {
            self.retired.store(true,Ordering::Release);
            return Err("Notification metadata transition is not the pinned CTo call".into());
        }
        let result=(|| {
            self.verify()?;
            let base=self.context.observe_app_delegate_offset_cells()?.main_header();
            // mov x0,#0; bl actual Ma stub; mov x19,x0. The surrounding
            // complete CTo is separately pinned by the callback authority.
            self.context.verify_main_code(base.checked_add(0x1fd560)
                .ok_or("Notification metadata caller address overflow")?,12,
                "5038097f60b4cc83666419f5a197552a244730ac680c0c69f9044ab32d794d5f")?;
            self.verify()
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
    pub(super) fn observe(context:Arc<crate::sdk::core_preentry::ReferenceAppDelegateContext>)->Result<Self> {
        context.cleanup_metadata_only()?;
        // Derive the sole point from actual four-instruction Ma code FIRST.
        // Never try an offset, swallow its refusal, then retry another point.
        let accessor=metadata::FoundationNotificationAccessorReturn::observe()?;
        let point=match accessor.point() {
            metadata::NotificationMetadataAddressPoint::Symbol=>FoundationNotificationAddressPoint::Symbol,
            metadata::NotificationMetadataAddressPoint::AfterValueWitness=>FoundationNotificationAddressPoint::AfterValueWitness,
        };
        let observed=context.observe_foundation_notification_metadata(point)?;
        #[link(name="Foundation",kind="framework")]
        unsafe extern "C" {
            #[link_name="$s10Foundation12NotificationV36_unconditionallyBridgeFromObjectiveCyACSo14NSNotificationCSgFZ"]
            fn NOTIFICATION_BRIDGE_LABEL();
        }
        let bridge=NOTIFICATION_BRIDGE_LABEL as *const () as usize;
        let rows=crate::sdk::imports::observe_unpatched_main_imports(&[
            (c"$s10Foundation12NotificationVMa",accessor.address()),
            (c"$s10Foundation12NotificationV36_unconditionallyBridgeFromObjectiveCyACSo14NSNotificationCSgFZ",bridge),
        ])?;
        let metadata_import=rows.iter().find(|row|row.symbol==c"$s10Foundation12NotificationVMa")
            .ok_or("Actual Main Notification metadata import absent")?.clone();
        let bridge_import=rows.into_iter().find(|row|row.symbol==c"$s10Foundation12NotificationV36_unconditionallyBridgeFromObjectiveCyACSo14NSNotificationCSgFZ")
            .ok_or("Actual Main Notification bridge import absent")?;
        let bridge_code=metadata::code_identity(bridge)?;
        let value=Self{context,accessor,point,observed,metadata_import,bridge_import,bridge_code,retired:AtomicBool::new(false)};
        value.verify()?;Ok(value)
    }
    pub(super) fn verify(&self)->Result<()> {
        self.context.cleanup_metadata_only()?;
        if self.retired.load(Ordering::Acquire){return Err("Notification witness observation retired".into());}
        let result=(|| {
            self.accessor.verify()?;
            let current=self.context.observe_foundation_notification_metadata(self.point)?;
            if current!=self.observed||current.symbol_address()!=self.accessor.vn_address()
                ||current.metadata_address()!=self.accessor.returned_address()
                ||self.accessor.address().checked_sub(self.accessor.code().offset())!=Some(current.image_header())
                ||self.accessor.code().uuid()!=current.uuid() {
                return Err("Actual Notification Ma/metadata/provider chain differs".into());
            }
            let base=self.context.observe_app_delegate_offset_cells()?.main_header();
            for (offset,cell,sha,row) in [
                (0x2f8c24usize,0x3c5168usize,"6fa665be86e82c801b5b3a2e18e07092712347729a2d4ebe10e7de4ff0afd38d",&self.metadata_import),
                (0x2f8bf4usize,0x3c5148usize,"63028a6ef42fc80effef505715c6759de721331dc83b3aab38d5c94d771d1423",&self.bridge_import),
            ] {
                self.context.verify_main_code(base.checked_add(offset).ok_or("Notification stub address overflow")?,12,sha)?;
                if base.checked_add(cell)!=Some(row.cell_address()) {return Err("Notification actual stub-to-import-cell differs".into());}
                row.verify_unpatched()?;
            }
            if self.metadata_import.original_address()!=self.accessor.address()
                ||self.bridge_code.image()!=self.accessor.code().image()
                ||self.bridge_import.original_address().checked_sub(self.bridge_code.offset())!=Some(current.image_header())
                ||self.bridge_code.uuid()!=current.uuid()
                ||metadata::code_identity(self.bridge_import.original_address())?!=self.bridge_code {
                return Err("Notification original Ma/bridge typed imports belong to another provider".into());
            }
            // Required Swift VWT ABI: 8 function pointers, size/stride,
            // flags:u32/extra:u32. These are observed words, not a guessed
            // Notification representation or a declaration-only DWARF size.
            let flags=current.flags();
            let alignment=(flags&0xff) as usize+1;
            let size=usize::try_from(current.size()).map_err(|_|"Notification value size exceeds address width")?;
            let stride=usize::try_from(current.stride()).map_err(|_|"Notification stride exceeds address width")?;
            if flags&0x00400000!=0||!alignment.is_power_of_two()||alignment>16
                ||size==0||size>65536||stride<size
                ||!stride.is_multiple_of(alignment)||current.extra_inhabitants()>0x7fffffff {
                return Err("Actual Notification VWT is incomplete or outside the supported value bound".into());
            }
            let words=current.metadata_words();
            if words[0]!=current.table_address() as u64||words[1]!=0x200
                ||words[2]!=current.descriptor_address() as u64 {
                return Err("Notification full metadata/table/nominal type chain differs".into());
            }
            let destroy=usize::try_from(u64::from_le_bytes(current.table()[8..16].try_into()
                .map_err(|_|"Notification destroy word width")?)).map_err(|_|"Notification destroy address width")?;
            if destroy!=current.destroy_address(){return Err("Notification selected destroy word changed".into());}
            let local=current.destroy_identity().image()==b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
                &&destroy.checked_sub(current.destroy_identity().offset())==Some(current.image_header())
                &&current.destroy_identity().uuid()==current.uuid();
            if !local {
                // The platform may select its named common value witness.
                // This is not permission for an arbitrary SwiftCore address:
                // exact export/code identity plus this same genuine complete
                // Foundation metadata/VWT and pinned x0/x1 caller are required.
                metadata::verify_notification_runtime_destroy(destroy,current.destroy_identity())?;
            }
            // Final fresh data/code observations, no model/Source callback.
            self.accessor.verify()?;
            if self.context.observe_foundation_notification_metadata(self.point)?!=current {
                return Err("Notification witness changed before its opaque caller transition".into());
            }
            self.accessor.verify()?;
            self.context.cleanup_metadata_only()?;
            self.metadata_import.verify_unpatched()?;
            self.bridge_import.verify_unpatched()?;
            self.accessor.verify()?;
            if self.retired.load(Ordering::Acquire){return Err("Notification witness retired during verification".into());}
            Ok(())
        })();
        if result.is_err(){self.retired.store(true,Ordering::Release);}result
    }
}
