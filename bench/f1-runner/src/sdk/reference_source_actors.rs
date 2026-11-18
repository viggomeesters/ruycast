//! Reference actor cleanup owner retained by the SDK runtime.
//! It does not construct source actors or widen callback/ARC permission.
use super::{core_preentry::ReferenceAdmission, owned::Result};
use crate::workspace::delegate::CallbackCodeAuthority;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub(super) struct SetupFailure {
    retired: Arc<AtomicBool>,
    processes: Option<Arc<super::process_dispatch::Registry>>,
    security: Option<Arc<super::private_network::Context>>,
    _callbacks: Option<Arc<dyn CallbackCodeAuthority>>,
    carbon:Option<Arc<super::input_carbon::Context>>,
    hid:Option<Arc<super::input_hid::Context>>,
    events:Option<std::rc::Rc<super::input_events::Context>>,
    input_prerequisites:Option<Arc<super::input_permissions_caps::Context>>,
    accessibility:Option<Arc<super::accessibility::model::Context>>,
    complete: bool,
}
/// The exact process and Security owners created during Reference assembly.
pub(super) struct RuntimeProviders {
    processes: Arc<super::process_dispatch::Registry>,
    security: Arc<super::private_network::Context>,
}
impl RuntimeProviders {
    #[cfg(target_arch = "aarch64")]
    pub(super) fn cancel_jobs(&self) -> Result<()> {
        super::process_abi::cancel_reference_jobs(&self.processes)
    }

    #[cfg(target_arch = "aarch64")]
    pub(super) fn verify_jobs_drained(&self) -> Result<()> {
        super::process_abi::verify_reference_jobs_drained(&self.processes)
    }

    pub(super) fn retire(&self) -> Result<()> {
        self.processes.retire();
        let website = super::private_network::drain_reference_website(&self.security);
        let security = super::private_network::retire(&self.security);
        website.and(security)
    }
}
impl SetupFailure {
    fn retire(&self) {
        // Pure monotone flags first, before any fallible cleanup or actor drop.
        self.retired.store(true, Ordering::Release);
        if let Some(processes) = &self.processes { processes.retire(); }
        if let Some(security) = &self.security {
            let _ = super::private_network::retire(security);
            let _ = super::private_network::rollback_reference_website(security);
        }
        if let Some(carbon)=&self.carbon {let _=super::input_carbon::retire(carbon);}
        if let Some(hid)=&self.hid {let _=super::input_hid::retire(hid);}
        if let Some(events)=&self.events{let _=super::input_events::retire(events);}
        if let Some(input)=&self.input_prerequisites{let _=super::input_permissions_caps::retire(input);}
        if let Some(ax)=&self.accessibility{let _=super::accessibility::abi::retire(ax);}
    }
    fn checked<T>(&self, result: Result<T>) -> Result<T> {
        result.inspect_err(|_| self.retire())
    }

    pub(super) fn runtime_providers(&self) -> Result<RuntimeProviders> {
        Ok(RuntimeProviders {
            processes: self.processes.clone().ok_or("Reference process owner absent")?,
            security: self.security.clone().ok_or("Reference Security owner absent")?,
        })
    }

    pub(super) fn install_website(&self, admission: Arc<ReferenceAdmission>) -> Result<super::ModuleAttestation> {
        let security = self.security.as_ref().ok_or("Reference Security rollback owner absent")?;
        super::private_network::install_reference_website(admission, security)
    }

    pub(super) fn probe_website(&self) -> Result<()> {
        let security = self.security.as_ref().ok_or("Reference Security probe owner absent")?;
        super::private_network::probe_reference_website(security)
    }

    pub(super) fn verify_keychain_fixture(
        &self,
        admission: &ReferenceAdmission,
        module: &super::ModuleAttestation,
        expected: &[u8],
    ) -> Result<()> {
        self.checked((|| {
            let security = self.security.as_ref().ok_or("Reference Security rollback owner absent")?;
            module.verify()?;
            admission.verify_reference_utility_imports(&module.imports)?;
            let before = security.counts()?;
            let entry = admission.reference_keychain_reader()?;
            let copy = module.imports.iter().find(|row| row.symbol == c"SecItemCopyMatching")
                .ok_or("Reference Keychain copy import absent")?;
            // Both the complete original Swift body and the selected private
            // Security import have been verified in this pre-entry owner.
            unsafe { super::private_network::verify_original_read(entry, copy, security, None) }?;
            let writer = admission.reference_keychain_writer()?;
            let alternate = b"owned synthetic Keychain update";
            if expected == alternate {
                return Err("Reference Keychain update fixture must differ".into());
            }
            for bytes in [expected, alternate.as_slice(), expected] {
                admission.verify_reference_utility_imports(&module.imports)?;
                unsafe {
                    super::private_network::verify_original_write(writer, &module.imports, security, bytes)?;
                    super::private_network::verify_original_read(entry, copy, security, Some(bytes))?;
                }
            }
            admission.verify_reference_utility_imports(&module.imports)?;
            let random = module.imports.iter().find(|row| row.symbol == c"SecRandomCopyBytes")
                .ok_or("Reference entropy import absent")?;
            let entropy = admission.reference_utility_entropy()?;
            unsafe { super::private_network::verify_original_entropy(entropy, random, security) }?;
            let after = security.counts()?;
            if before[0].checked_add(4) != Some(after[0])
                || before[1].checked_add(3) != Some(after[1])
                || before[2].checked_add(1) != Some(after[2])
                || before[3].checked_add(4) != Some(after[3])
            {
                return Err("Reference original Keychain or entropy effects differ".into());
            }
            module.verify()?;
            admission.verify_reference_utility_imports(&module.imports)
        })())
    }

    pub(super) fn finish(&mut self) {
        self.complete = true;
    }
}
impl Drop for SetupFailure {
    fn drop(&mut self) { if !self.complete { self.retire(); } }
}
