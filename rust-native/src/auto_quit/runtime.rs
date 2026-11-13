//! Serial clock-to-effect adapter. Unknown coverage requires separate explicit
//! per-application acknowledgement; positive audio and failed reads still deny.
use super::{
    safety::{Decision, Guard, Snapshot},
    Due, Observation, Tracker,
};
use rustcast_core::AutoQuitSettings;
use std::{collections::BTreeMap, time::Duration};
pub type Preferences = BTreeMap<String, AutoQuitSettings>;
pub trait Adapter {
    fn now(&self) -> Duration;
    fn preferences(&mut self) -> Result<Preferences, String>;
    fn applications(&mut self, preferences: &Preferences) -> Vec<Observation>;
    fn snapshot(&mut self, candidate: &Due) -> Option<Snapshot>;
    /// Must recheck exact live identity, foreground, preference and cancellation
    /// before sending a normal quit; never force or confirm save prompts.
    fn request(&mut self, candidate: &Due, expected: &AutoQuitSettings) -> bool;
}
#[derive(Default)]
pub struct Runtime {
    tracker: Tracker,
    attempted: Vec<Due>,
}
impl Runtime {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn reset_application(&mut self, application: &str) {
        self.tracker.reset_application(application);
    }
    pub fn tick(&mut self, adapter: &mut impl Adapter) -> Result<usize, String> {
        let preferences = match adapter.preferences() {
            Ok(value) => value,
            Err(error) => {
                self.reset();
                return Err(error);
            }
        };
        let applications = adapter.applications(&preferences);
        let due = self
            .tracker
            .observe(adapter.now(), &applications, &preferences);
        self.attempted
            .retain(|candidate| self.tracker.is_current(candidate));
        let mut requests = 0;
        for candidate in due {
            if self.attempted.contains(&candidate) {
                continue;
            }
            let current = match adapter.preferences() {
                Ok(value) => value,
                Err(error) => {
                    self.reset();
                    return Err(error);
                }
            };
            let snapshot = adapter.snapshot(&candidate);
            if Guard::default().assess(
                &self.tracker,
                &candidate,
                adapter.now(),
                &current,
                snapshot.as_ref(),
            ) != Decision::AllowedUnverified
            {
                continue;
            }
            let Some(preference) = current
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(&candidate.application))
                .map(|(_, value)| value)
            else {
                continue;
            };
            // Consume before the OS call. Refusal, deferred save decisions and
            // send failures never produce a repeated automatic prompt storm.
            self.attempted.push(candidate.clone());
            if adapter.request(&candidate, preference) {
                requests += 1;
            }
        }
        Ok(requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auto_quit::{
            safety::{Helpers, Signal},
            Activity,
        },
        system_search::Identity,
    };
    struct Fake {
        time: Duration,
        prefs: Preferences,
        app: Observation,
        running: Signal,
        input: Signal,
        output: Signal,
        requests: usize,
        reread_disable: bool,
        reads: usize,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                time: Duration::ZERO,
                prefs: Preferences::from([(
                    "test.owned.Editor".into(),
                    AutoQuitSettings {
                        enabled: true,
                        minutes: 1,
                        allow_unverified_activity: true,
                    },
                )]),
                app: Observation {
                    application: "test.owned.Editor".into(),
                    process: Identity {
                        pid: 42,
                        uid: 501,
                        seconds: 100,
                        micros: 1,
                    },
                    activity: Activity::Background,
                },
                running: Signal::Inactive,
                input: Signal::Inactive,
                output: Signal::Inactive,
                requests: 0,
                reread_disable: false,
                reads: 0,
            }
        }
    }
    impl Adapter for Fake {
        fn now(&self) -> Duration {
            self.time
        }
        fn preferences(&mut self) -> Result<Preferences, String> {
            self.reads += 1;
            let mut p = self.prefs.clone();
            if self.reread_disable && self.reads > 1 {
                p.values_mut().next().unwrap().enabled = false;
            }
            Ok(p)
        }
        fn applications(&mut self, _: &Preferences) -> Vec<Observation> {
            vec![self.app.clone()]
        }
        fn snapshot(&mut self, _: &Due) -> Option<Snapshot> {
            Some(Snapshot {
                application: self.app.clone(),
                sampled_at: self.time,
                running: self.running,
                input: self.input,
                output: self.output,
                recording: Signal::Unknown,
                helpers: Helpers::Unknown,
            })
        }
        fn request(&mut self, _: &Due, _: &AutoQuitSettings) -> bool {
            self.requests += 1;
            true
        }
    }
    #[test]
    fn acknowledgement_is_required_and_positive_audio_or_read_failures_never_admit() {
        for signal in [Signal::Active, Signal::Failed, Signal::Unavailable] {
            let mut fake = Fake::new();
            fake.running = signal;
            let mut runtime = Runtime::default();
            runtime.tick(&mut fake).unwrap();
            fake.time = Duration::from_secs(60);
            assert_eq!(
                runtime.tick(&mut fake).unwrap(),
                0,
                "generic IO running/error must block even when input/output are zero"
            );

            for input in [true, false] {
                let mut fake = Fake::new();
                if input {
                    fake.input = signal;
                } else {
                    fake.output = signal;
                }
                let mut runtime = Runtime::default();
                runtime.tick(&mut fake).unwrap();
                fake.time = Duration::from_secs(60);
                assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
                assert_eq!(fake.requests, 0);
            }
        }
        let mut fake = Fake::new();
        fake.prefs
            .values_mut()
            .next()
            .unwrap()
            .allow_unverified_activity = false;
        let mut runtime = Runtime::default();
        runtime.tick(&mut fake).unwrap();
        fake.time = Duration::from_secs(60);
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
        fake.prefs
            .values_mut()
            .next()
            .unwrap()
            .allow_unverified_activity = true;
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
        fake.time = Duration::from_secs(120);
        assert_eq!(runtime.tick(&mut fake).unwrap(), 1);
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
    }
    #[test]
    fn final_preference_read_and_foreground_notifications_revoke_old_candidates() {
        let mut fake = Fake::new();
        let mut runtime = Runtime::default();
        runtime.tick(&mut fake).unwrap();
        fake.time = Duration::from_secs(60);
        fake.reread_disable = true;
        fake.reads = 0;
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
        assert_eq!(fake.requests, 0);
        fake.reread_disable = false;
        runtime.reset_application("test.owned.Editor");
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
        fake.time = Duration::from_secs(120);
        assert_eq!(runtime.tick(&mut fake).unwrap(), 1);
        runtime.reset();
        assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
    }
    #[test]
    fn app_restart_foreground_and_duration_change_require_a_new_full_interval() {
        for mutation in ["restart", "foreground", "duration"] {
            let mut fake = Fake::new();
            let mut runtime = Runtime::default();
            runtime.tick(&mut fake).unwrap();
            fake.time = Duration::from_secs(60);
            match mutation {
                "restart" => fake.app.process.seconds += 1,
                "foreground" => fake.app.activity = Activity::Foreground,
                _ => fake.prefs.values_mut().next().unwrap().minutes = 2,
            }
            assert_eq!(runtime.tick(&mut fake).unwrap(), 0);
        }
    }
}
