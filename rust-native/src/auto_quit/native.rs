//! Public native observations scoped to configured bundle identifiers only.
use super::{
    runtime::{Adapter, Preferences},
    safety::{Helpers, Signal, Snapshot},
    Activity, Due, Observation,
};
use crate::{
    profile::{ConfigStore, ProfilePaths},
    system_search::{native::identity, Identity},
};
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;
use std::{
    ffi::c_void,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};
#[repr(C)]
struct Serial {
    high: u32,
    low: u32,
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(process: *mut Serial) -> i16;
    fn GetProcessPID(process: *const Serial, pid: *mut i32) -> i32;
}
fn foreground() -> Option<i32> {
    let mut serial = Serial { high: 0, low: 0 };
    let mut pid = 0;
    if unsafe { GetFrontProcess(&mut serial) } != 0
        || unsafe { GetProcessPID(&serial, &mut pid) } != 0
        || pid <= 1
    {
        None
    } else {
        Some(pid)
    }
}
#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}
#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}
fn property(object: u32, selector: &[u8; 4], pid: Option<&i32>) -> Result<u32, ()> {
    let address = Address {
        selector: u32::from_be_bytes(*selector),
        scope: u32::from_be_bytes(*b"glob"),
        element: 0,
    };
    let mut size = 4u32;
    let mut value = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &address,
            if pid.is_some() { 4 } else { 0 },
            pid.map_or(ptr::null(), |value| (value as *const i32).cast()),
            &mut size,
            (&mut value as *mut u32).cast(),
        )
    };
    if status != 0 || size != 4 {
        Err(())
    } else {
        Ok(value)
    }
}
fn audio(process: Identity) -> (Signal, Signal, Signal) {
    let read = || -> Result<(Signal, Signal, Signal), ()> {
        let object = property(1, b"id2p", Some(&process.pid))?;
        // No process object is unknown, never fabricated proof of inactivity.
        if object == 0 {
            return Ok((Signal::Unknown, Signal::Unknown, Signal::Unknown));
        }
        if property(object, b"ppid", None)? != process.pid as u32 {
            return Err(());
        }
        let running = property(object, b"pir?", None)?;
        let input = property(object, b"piri", None)?;
        let output = property(object, b"piro", None)?;
        if running > 1
            || input > 1
            || output > 1
            || property(object, b"ppid", None)? != process.pid as u32
        {
            return Err(());
        }
        Ok((
            if running == 1 {
                Signal::Active
            } else {
                Signal::Inactive
            },
            if input == 1 {
                Signal::Active
            } else {
                Signal::Inactive
            },
            if output == 1 {
                Signal::Active
            } else {
                Signal::Inactive
            },
        ))
    };
    read().unwrap_or((Signal::Failed, Signal::Failed, Signal::Failed))
}
pub struct Native {
    paths: ProfilePaths,
    started: Instant,
    live: Rc<dyn Fn() -> bool>,
    _main: Option<MainThreadMarker>,
    #[cfg(test)]
    owned: Option<(Identity, std::path::PathBuf)>,
    #[cfg(test)]
    before_effect: Option<Box<dyn FnMut()>>,
    #[cfg(test)]
    after_config_read: Option<Box<dyn FnMut(&mut Self)>>,
    #[cfg(test)]
    foreground_override: Option<i32>,
}
impl Native {
    pub fn new(main: MainThreadMarker, paths: ProfilePaths, live: Rc<dyn Fn() -> bool>) -> Self {
        Self {
            paths,
            started: Instant::now(),
            live,
            _main: Some(main),
            #[cfg(test)]
            owned: None,
            #[cfg(test)]
            before_effect: None,
            #[cfg(test)]
            after_config_read: None,
            #[cfg(test)]
            foreground_override: None,
        }
    }
    #[cfg(test)]
    pub(super) fn owned_test(
        paths: ProfilePaths,
        process: Identity,
        executable: std::path::PathBuf,
    ) -> Self {
        Self {
            paths,
            started: Instant::now(),
            live: Rc::new(|| true),
            _main: None,
            owned: Some((process, executable)),
            before_effect: None,
            #[cfg(test)]
            after_config_read: None,
            #[cfg(test)]
            foreground_override: None,
        }
    }
    #[cfg(test)]
    pub(super) fn before_effect(&mut self, hook: impl FnMut() + 'static) {
        self.before_effect = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn after_config_read(&mut self, hook: impl FnMut(&mut Self) + 'static) {
        self.after_config_read = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn owned_foreground(&mut self, pid: i32) {
        self.foreground_override = Some(pid);
    }
    fn foreground(&self) -> Option<i32> {
        #[cfg(test)]
        if let Some(pid) = self.foreground_override {
            return Some(pid);
        }
        foreground()
    }
    fn app(&self, application: &str, process: Identity) -> Option<Retained<NSRunningApplication>> {
        if !(self.live)()
            || process.uid != unsafe { libc::getuid() }
            || process.pid == unsafe { libc::getpid() }
            || identity(process.pid) != Some(process)
        {
            return None;
        }
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(process.pid)?;
        #[cfg(test)]
        if let Some((expected, executable)) = &self.owned {
            if *expected != process
                || app.executableURL()?.path()?.to_string() != executable.to_str()?
            {
                return None;
            }
        }

        if !app
            .bundleIdentifier()?
            .to_string()
            .eq_ignore_ascii_case(application)
            || identity(process.pid) != Some(process)
        {
            return None;
        }
        Some(app)
    }
    fn observation(&self, application: &str, process: Identity) -> Option<Observation> {
        self.app(application, process)?;
        let activity = match self.foreground() {
            Some(pid) if pid == process.pid => Activity::Foreground,
            Some(_) => Activity::Background,
            None => Activity::Unknown,
        };
        if identity(process.pid) != Some(process) {
            return None;
        }
        Some(Observation {
            application: application.into(),
            process,
            activity,
        })
    }
}
impl Adapter for Native {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }
    fn preferences(&mut self) -> Result<Preferences, String> {
        if !(self.live)() {
            return Err("Auto Quit paused.".into());
        }
        Ok(ConfigStore::open_existing(self.paths.clone())?
            .current()
            .auto_quit
            .clone())
    }
    fn applications(&mut self, preferences: &Preferences) -> Vec<Observation> {
        let mut applications = vec![];
        for (application, preference) in preferences.iter().filter(|(_, value)| value.enabled) {
            let _ = preference;
            for app in NSRunningApplication::runningApplicationsWithBundleIdentifier(
                &NSString::from_str(application),
            ) {
                let Some(process) = identity(app.processIdentifier()) else {
                    continue;
                };
                if let Some(observation) = self.observation(application, process) {
                    applications.push(observation);
                }
            }
        }
        applications
    }
    fn snapshot(&mut self, candidate: &Due) -> Option<Snapshot> {
        let application = self.observation(&candidate.application, candidate.process)?;
        let (running, input, output) = audio(candidate.process);
        if identity(candidate.process.pid) != Some(candidate.process) {
            return None;
        }
        Some(Snapshot {
            application,
            sampled_at: self.now(),
            running,
            input,
            output,
            recording: Signal::Unknown,
            helpers: Helpers::Unknown,
        })
    }
    fn request(&mut self, candidate: &Due, expected: &rustcast_core::AutoQuitSettings) -> bool {
        let Some(current) = self.preferences().ok().and_then(|p| {
            p.into_iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(&candidate.application))
                .map(|(_, value)| value)
        }) else {
            return false;
        };
        if !current.enabled
            || !current.allow_unverified_activity
            || current.minutes != expected.minutes
        {
            return false;
        }
        let Some(snapshot) = self.snapshot(candidate) else {
            return false;
        };
        if snapshot.application.activity != Activity::Background
            || [snapshot.running, snapshot.input, snapshot.output]
                .iter()
                .any(|value| matches!(value, Signal::Active | Signal::Failed | Signal::Unavailable))
        {
            return false;
        }
        if !(self.live)()
            || self
                .foreground()
                .is_none_or(|pid| pid == candidate.process.pid)
            || identity(candidate.process.pid) != Some(candidate.process)
        {
            return false;
        }
        #[cfg(test)]
        if let Some(mut hook) = self.before_effect.take() {
            hook();
        }
        // The existing config.lock is also held by native/CLI save and
        // relocation. Disable published during sensor queries wins; a busy
        // writer blocks this effect. Keep the final read and normal request in
        // one bounded nonblocking admission, without any new writer authority.
        let paths = self.paths.clone();
        ConfigStore::with_locked_current(&paths, |config| {
            #[cfg(test)]
            if let Some(mut hook) = self.after_config_read.take() {
                hook(self);
            }
            let allowed = config
                .auto_quit
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(&candidate.application))
                .is_some_and(|(_, value)| {
                    value.enabled
                        && value.allow_unverified_activity
                        && value.minutes == expected.minutes
                });
            if !allowed {
                return Ok(false);
            }
            // Config reading can take time. Refresh mutable safety after it,
            // while the same writer lock prevents a concurrent disable.
            let Some(snapshot) = self.snapshot(candidate) else {
                return Ok(false);
            };
            if snapshot.application.activity != Activity::Background
                || [snapshot.running, snapshot.input, snapshot.output]
                    .iter()
                    .any(|value| {
                        matches!(value, Signal::Active | Signal::Failed | Signal::Unavailable)
                    })
            {
                return Ok(false);
            }
            // Revalidate identity, bundle and foreground directly before effect.
            let Some(app) = self.app(&candidate.application, candidate.process) else {
                return Ok(false);
            };
            if self
                .foreground()
                .is_none_or(|pid| pid == candidate.process.pid)
                || identity(candidate.process.pid) != Some(candidate.process)
                || !(self.live)()
            {
                return Ok(false);
            }
            Ok(app.terminate())
        })
        .unwrap_or(false)
    }
}
