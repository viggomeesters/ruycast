//! Compile-only SDK composition. Bootstrap interposition and the authoritative
//! readiness export are present; source qualification remains closed. Do not
//! load this library or a host before independent complete source admission.
pub mod sdk;
#[path = "sdk/workspace.rs"]
pub mod workspace;
pub mod guardian;
pub mod guardian_alive;
