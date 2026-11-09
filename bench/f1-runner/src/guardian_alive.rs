//! Rust-only adapter to the integrator-owned v1 lease interface.
use crate::{guardian::Guard, sdk::owned::AliveLease};

pub struct GuardianAlive {
    guard: Guard,
}
impl GuardianAlive {
    pub fn admitted(guard: Guard) -> Self {
        Self { guard }
    }
}
impl AliveLease for GuardianAlive {
    fn is_alive(&self) -> bool {
        self.guard.is_alive()
    }
    fn before_effect(&self) -> Result<(), String> {
        self.guard
            .before_effect()
            .map_err(|error| error.to_string())
    }
}
