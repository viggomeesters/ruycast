use objc2::MainThreadMarker;
use std::{io, thread, thread::JoinHandle};

/// Keeps an AppKit main-thread capability attached to state owned by that thread.
pub struct MainThreadState {
    marker: MainThreadMarker,
}

impl MainThreadState {
    pub fn new(marker: MainThreadMarker) -> Self {
        Self { marker }
    }

    pub fn marker(&self) -> MainThreadMarker {
        self.marker
    }
}

/// Starts the one named runtime worker used by the native host.
pub fn spawn_runtime<F>(worker: F) -> io::Result<JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name("ruycast-runtime".into())
        .spawn(worker)
}
