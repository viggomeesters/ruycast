//! Owned detached capture-holder release and bounded cleanup.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
pub struct Holder {
    pub ready: PathBuf,
    gate: PathBuf,
    pub done: PathBuf,
    released: bool,
}
impl Holder {
    pub fn new(base: &Path, held: &str) -> Self {
        Self {
            ready: base.join(format!("ready-{held}")),
            gate: base.join(format!("release-{held}")),
            done: base.join(format!("done-{held}")),
            released: false,
        }
    }
    pub fn release(&mut self) {
        fs::write(&self.gate, "release").unwrap();
        if self.ready.exists() {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !self.done.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                self.done.exists(),
                "owned detached holder did not self-close"
            );
        }
        self.released = true;
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        if !self.released {
            let _ = fs::write(&self.gate, "release");
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while self.ready.exists() && !self.done.exists() && std::time::Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
