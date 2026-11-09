//! Offline F1 protocol, ordered parity and raw statistics. No desktop API or normal app launch.
mod f1_support;
pub mod guardian;
pub mod host_io;
fn main() {
    if let Err(error) = f1_support::entry(std::env::args().skip(1).collect()) {
        eprintln!("f1-runner: {error}");
        std::process::exit(2);
    }
}
