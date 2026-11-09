//! Source-owned arm64 Swift Date.init leaf. The Swift value has no Rust layout.
//! Only the existing numeric Swift constructor may initialize caller x8.
use super::{Fixture, PowerSourcePin, Result, FAULTED, FIXTURE};
use crate::sdk::{core_preentry::ReferenceAdmission, metadata, owned::AliveLease, ImportReplacement, ImportSpec};
use std::{ffi::{c_void,CStr}, sync::{Arc,OnceLock,Weak,atomic::Ordering}};

const FOUNDATION: &[u8] = b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation";
const NOW: &CStr = c"$s10Foundation4DateVACycfC";
const EPOCH: &CStr = c"$s10Foundation4DateV21timeIntervalSince1970ACSd_tcfC";

// These labels provide addresses only. Calling either through the C ABI is
// forbidden: their proven Swift ABI uses opaque indirect result register x8.
unsafe extern "C" {
    #[link_name = "$s10Foundation4DateVACycfC"]
    static DATE_NOW_LABEL: u8;
    #[link_name = "$s10Foundation4DateV21timeIntervalSince1970ACSd_tcfC"]
    static DATE_EPOCH_LABEL: u8;
    fn rcf1_monitor_date_init();
}

/// Private capability, minted only from the same actual Reference admission,
/// installed Fixture/Registry, compiler-bound source and actual loader cells.
pub(super) struct ClockSource {
    state: Weak<Fixture>,
    admission: Arc<ReferenceAdmission>,
    registry: Arc<crate::sdk::process_dispatch::Registry>,
    pins: Vec<PowerSourcePin>,
    now: usize,
    epoch: usize,
    now_code: metadata::CodeIdentity,
    epoch_code: metadata::CodeIdentity,
    replacement_code: metadata::CodeIdentity,
    imports: OnceLock<Vec<ImportReplacement>>,
    epoch_imports: OnceLock<Vec<ImportReplacement>>,
}

#[cfg(rcf1_monitor_swift_clock)]
fn compiled_sources() -> [(&'static str,&'static [u8]);5] {
    [
        (env!("RUYCAST_F1_MONITOR_SWIFT_INTERFACE"),include_bytes!(env!("RUYCAST_F1_MONITOR_SWIFT_INTERFACE"))),
        (env!("RUYCAST_F1_MONITOR_SWIFT_DATE_HEADER"),include_bytes!(env!("RUYCAST_F1_MONITOR_SWIFT_DATE_HEADER"))),
        (concat!(env!("CARGO_MANIFEST_DIR"),"/../reference/0.8.26/Sources/RuycastApp/ResourceSampler.swift"),include_bytes!("../../../../reference/0.8.26/Sources/RuycastApp/ResourceSampler.swift")),
        (concat!(env!("CARGO_MANIFEST_DIR"),"/../reference/0.8.26/Sources/RuycastApp/AnalyticsStore.swift"),include_bytes!("../../../../reference/0.8.26/Sources/RuycastApp/AnalyticsStore.swift")),
        (concat!(env!("CARGO_MANIFEST_DIR"),"/src/sdk/system_providers/monitor_clock.rs"),include_bytes!("monitor_clock.rs")),
    ]
}

fn source_pins(owner:u32)->Result<Vec<PowerSourcePin>> {
    #[cfg(not(rcf1_monitor_swift_clock))]
    {let _=owner;Err("Swift clock actual compiler source inputs absent".into())}
    #[cfg(rcf1_monitor_swift_clock)]
    {
        use sha2::{Digest,Sha256};
        let text=option_env!("RUYCAST_F1_MONITOR_SWIFT_DECLARATION_PINS")
            .ok_or("Swift clock compiler pins absent")?;
        if text.len()>32768 {return Err("Swift clock compiler pin bound".into());}
        let rows:[(std::path::PathBuf,String,u32);5]=serde_json::from_str(text).map_err(|_|"Swift clock compiler pin schema")?;
        let mut pins=Vec::new();
        for (index,((path,hash,uid),(compiled_path,bytes))) in rows.into_iter().zip(compiled_sources()).enumerate() {
            let expected=std::fs::canonicalize(compiled_path).map_err(|_|"Swift clock compiled path absent")?;
            if !path.is_absolute() || path!=expected || uid!=if index<2 {0}else{owner}
                || hash.len()!=64 || !hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
                || hash!=format!("{:x}",Sha256::digest(bytes)) {
                return Err("Swift clock pins differ from actual compiler bytes/issuer".into());
            }
            let mut digest=[0;32];
            for (at,byte) in digest.iter_mut().enumerate() {
                *byte=u8::from_str_radix(&hash[at*2..at*2+2],16).map_err(|_|"Swift clock pin digest")?;
            }
            pins.push(PowerSourcePin{path,sha256:digest,owner:uid});
        }
        let cf=super::PowerSource::compiled_inputs()?;
        let frameworks=std::path::Path::new(cf[0].0).parent().and_then(|p|p.parent())
            .and_then(|p|p.parent()).and_then(|p|p.parent()).and_then(|p|p.parent())
            .ok_or("Swift clock selected SDK framework root absent")?;
        let expected_interface=std::fs::canonicalize(frameworks.join("Foundation.framework/Modules/Foundation.swiftmodule/arm64e-apple-macos.swiftinterface"))
            .map_err(|_|"Swift clock selected SDK interface absent")?;
        let expected_date_header=frameworks.join("Foundation.framework/Versions/C/Headers/NSDate.h");
        if !expected_interface.starts_with(frameworks) || pins[0].path!=expected_interface || pins[1].path!=expected_date_header {
            return Err("Swift clock interface is not in the same selected compiler SDK".into());
        }
        let interface=super::read_power_source(&pins[0])?;
        let compact=interface.iter().copied().filter(|b|!b.is_ascii_whitespace()).collect::<Vec<_>>();
        for required in [b"publicstructDate:Swift.Comparable,Swift.Hashable,Swift.Equatable,Swift.Sendable{".as_slice(),
            b"publicinit()",b"publicinit(timeIntervalSince1970:Foundation.TimeInterval)"] {
            if !compact.windows(required.len()).any(|w|w==required) {
                return Err("Swift Date typed declaration differs".into());
            }
        }
        // Primitive TimeInterval declaration only; this grants no NSDate
        // receiver/constructor/clock and does not assert Date's value layout.
        let date_header=super::read_power_source(&pins[1])?;
        let compact_header=date_header.iter().copied().filter(|b|!b.is_ascii_whitespace()).collect::<Vec<_>>();
        if !compact_header.windows(b"typedefdoubleNSTimeInterval;".len())
            .any(|w|w==b"typedefdoubleNSTimeInterval;") {
            return Err("Swift clock primitive interval declaration differs".into());
        }
        Ok(pins)
    }
}

fn named_code(address:usize,name:&CStr)->Result<metadata::CodeIdentity> {
    let code=metadata::code_identity(address)?;
    let mut info=std::mem::MaybeUninit::<libc::Dl_info>::uninit();
    if code.image()!=FOUNDATION || unsafe {libc::dladdr(address as *const c_void,info.as_mut_ptr())}==0 {
        return Err("Swift clock callee is not original Foundation code".into());
    }
    let info=unsafe {info.assume_init()};
    if info.dli_sname.is_null() || info.dli_saddr as usize!=address
        || unsafe {CStr::from_ptr(info.dli_sname)}!=name {
        return Err("Swift clock exact callee symbol differs".into());
    }
    Ok(code)
}

impl ClockSource {
    fn specs(&self)->[ImportSpec;2] {
        // Only NOW is installed. EPOCH's replacement field is an observer
        // marker; its actual cell must remain the original numeric constructor.
        let replacement=rcf1_monitor_date_init as *const () as usize;
        [ImportSpec{symbol:NOW,original:self.now,replacement},ImportSpec{symbol:EPOCH,original:self.epoch,replacement}]
    }
    pub(super) fn verify_pure(&self)->Result<()> {
        if FAULTED.load(Ordering::Acquire) {return Err("Swift clock faulted".into());}
        let state=self.state.upgrade().ok_or("Swift clock fixture dropped")?;
        if !FIXTURE.get().is_some_and(|current|Arc::ptr_eq(current,&state))
            || !state.module.get().is_some_and(|module|Arc::ptr_eq(&module.registry,&self.registry)) {
            return Err("Swift clock installed fixture/Registry differs".into());
        }
        for pin in &self.pins {super::read_power_source(pin)?;}
        if named_code(self.now,NOW)?!=self.now_code || named_code(self.epoch,EPOCH)?!=self.epoch_code
            || metadata::code_identity(rcf1_monitor_date_init as *const () as usize)?!=self.replacement_code {
            return Err("Swift clock code changed".into());
        }
        let observed=crate::sdk::observe_main_imports(&self.specs())?;
        let retained=self.imports.get().ok_or("Swift clock imports unpublished")?;
        let epoch_retained=self.epoch_imports.get().ok_or("Swift clock epoch cells absent")?;
        if observed.len()!=2 || retained.is_empty()
            || observed.iter().any(|row|if row.symbol==NOW {
                row.selected_address()!=row.replacement || !retained.iter().any(|old|old.symbol==NOW
                    && old.cell_address()==row.cell_address() && old.original==row.original && old.replacement==row.replacement)
            } else {row.symbol!=EPOCH || row.selected_address()!=self.epoch
                || !epoch_retained.iter().any(|old|old.symbol==EPOCH && old.cell_address()==row.cell_address()
                    && old.original==row.original && old.replacement==row.replacement)})
            || observed.iter().filter(|row|row.symbol==NOW).count()!=retained.len()
            || observed.iter().filter(|row|row.symbol==EPOCH).count()!=epoch_retained.len() {
            return Err("Swift clock selected imports differ".into());
        }
        self.registry.not_retired()
    }
}

pub(super) fn install(state:&Arc<Fixture>,admission:Arc<ReferenceAdmission>,guard:&dyn AliveLease)->Result<()> {
    super::verify_same_fixture(state)?;
    admission.before_reference_initialization()?;
    if !state.matches_epoch(admission.fixture_epoch_unix_seconds()?) {
        return Err("Swift clock fixture epoch differs from installed MonitorFacts".into());
    }
    let registry=state.module.get().ok_or("Swift clock process module absent")?.registry.clone();
    registry.matches_issuer(&admission.life(),&admission.descriptors())?;
    let now=std::ptr::addr_of!(DATE_NOW_LABEL) as usize;
    let epoch=std::ptr::addr_of!(DATE_EPOCH_LABEL) as usize;
    let source=Arc::new(ClockSource{state:Arc::downgrade(state),admission,registry:registry.clone(),
        pins:source_pins(state.context.home.owner_ids()?.0)?,now,epoch,
        now_code:named_code(now,NOW)?,epoch_code:named_code(epoch,EPOCH)?,
        replacement_code:metadata::code_identity(rcf1_monitor_date_init as *const () as usize)?,
        imports:OnceLock::new(),epoch_imports:OnceLock::new()});
    let specs=source.specs();
    let before=crate::sdk::observe_main_imports(&specs)?;
    if before.len()!=2 || before.iter().any(|row|row.selected_address()!=row.original)
        || !before.iter().any(|row|row.symbol==NOW) || !before.iter().any(|row|row.symbol==EPOCH) {
        return Err("Swift clock actual two original import cells absent".into());
    }
    source.epoch_imports.set(before.into_iter().filter(|row|row.symbol==EPOCH).collect())
        .map_err(|_|"Swift clock epoch cells already retained")?;
    super::verify_same_fixture(state)?;
    if !state.matches_epoch(source.admission.fixture_epoch_unix_seconds()?) {
        return Err("Swift clock fixture epoch changed before publication".into());
    }
    state.clock.set(source.clone()).map_err(|_|"Swift clock already installed")?;
    let result=(|| {
        let imports=super::super::import_patch::install(&specs[..1],guard)?;
        source.imports.set(imports).map_err(|_|"Swift clock imports already retained")?;
        source.verify_pure()?;
        super::verify_same_fixture(state)?;
        if !state.matches_epoch(source.admission.fixture_epoch_unix_seconds()?) {
            return Err("Swift clock fixture epoch differs after installation".into());
        }
        source.verify_pure()
    })();
    if result.is_err(){registry.retire();}
    result
}

#[repr(C)]
struct Prepared {constructor:usize,seconds:f64}
const _:()=assert!(std::mem::size_of::<Prepared>()==16
    && std::mem::align_of::<Prepared>()==8 && std::mem::offset_of!(Prepared,seconds)==8);

// The only writer to Prepared is this Rust helper, with a pointer into our own
// assembly stack frame. It never reads/writes the opaque Swift result buffer.
#[no_mangle]
unsafe extern "C" fn rcf1_monitor_clock_prepare(output:usize,return_pc:usize,prepared:*mut Prepared) {
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<Prepared>{
        if output==0 || prepared.is_null() {return Err("Swift clock nil ABI storage".into());}
        let (state,facts)=super::monitor_facts()?;
        let source=state.clock.get().ok_or("Swift clock source absent")?;
        super::verify_same_fixture(&state)?;
        source.registry.not_retired()?;
        source.admission.before_monitor_clock_callback(return_pc,&source.registry)?;
        // No dynamic guard, metadata accessor or user callback after this.
        source.verify_pure()?;
        if !facts.wall_unix.is_finite() {return Err("Swift clock epoch is not finite".into());}
        source.registry.not_retired()?;
        Ok(Prepared{constructor:source.epoch,seconds:facts.wall_unix})
    }));
    match result {
        Ok(Ok(value))=>unsafe{prepared.write(value)},
        failure=>{
            FAULTED.store(true,Ordering::Release);
            #[cfg(not(test))]
            match failure {
                Ok(Err(error)) => crate::sdk::private_network::report_reference_website_clock_refusal(return_pc, &error),
                Err(payload) => {
                    std::mem::forget(payload);
                    crate::sdk::private_network::report_reference_website_clock_refusal(return_pc, "clock callback panicked");
                }
                Ok(Ok(_)) => unreachable!(),
            }
            #[cfg(test)]
            let _ = failure;
            // Date.init is non-throwing: returning without an initialized value
            // is forbidden. No fallback clock or partially initialized return.
            unsafe{libc::_exit(94)}
        }
    }
}

// Proven arm64 Swift ABI: result storage arrives in x8. Rust C code uses only
// the private stack record; then tail-enter the qualified epoch constructor
// with seconds in d0 and original x8/LR. x19-x28 and d8-d15 remain ABI-preserved.
std::arch::global_asm!(r#"
.text
.p2align 2
.globl _rcf1_monitor_date_init
_rcf1_monitor_date_init:
 sub sp, sp, #48
 stp x29, x30, [sp]
 str x8, [sp, #16]
 mov x0, x8
 mov x1, x30
 add x2, sp, #24
 bl _rcf1_monitor_clock_prepare
 ldr x16, [sp, #24]
 ldr d0, [sp, #32]
 ldr x8, [sp, #16]
 ldp x29, x30, [sp]
 add sp, sp, #48
 br x16
"#);
