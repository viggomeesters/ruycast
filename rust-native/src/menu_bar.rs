//! Plain menu state, refreshed with configuration and analytics health.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct State {
    pub visible: bool,
    pub healthy: bool,
    pub requires_input: bool,
}
