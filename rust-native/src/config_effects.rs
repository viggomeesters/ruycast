//! Platform staging is supplied by the host. A dropped stage rolls back;
//! commit follows successful publication/adoption, including durability warnings.
use std::sync::Arc;
#[derive(Clone, Debug, Default)]
pub struct Receipt {
    pub warning: Option<String>,
    pub generation: Option<u64>,
}
pub trait Stage: Send {
    fn ready(&self) -> Result<(), String> {
        Ok(())
    }
    /// Fallible data publication after validation, before activating bindings.
    /// SAVE invokes this only after its configuration publication succeeded.
    fn before_adopt(&mut self, _publication: Option<Arc<crate::configuration_jobs::Publication>>) -> Result<(), String> {
        Ok(())
    }
    fn commit(&mut self) -> Result<Receipt, String>;
}
pub type Factory =
    Arc<dyn Fn(&rustcast_core::Config) -> Result<Box<dyn Stage>, String> + Send + Sync>;
