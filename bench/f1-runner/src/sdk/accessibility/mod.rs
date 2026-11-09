//! Owned AX data and C-ABI replacements. Real AX services are never invoked.
pub mod abi;
pub mod model;
pub mod opaque;
#[cfg(test)]
mod tests;
