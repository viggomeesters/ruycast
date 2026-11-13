//! Pure Auto Quit clock policy. No OS observations or application effects.
//!
//! Each observation is a complete current snapshot. A background interval starts
//! on its first observation; foreground, unknown/missing state, sleep/wake,
//! process replacement and duration changes reset it. Startup/restart begins a
//! fresh interval. Expiry produces candidates, never permission to terminate.
pub mod safety;
pub mod runtime;
pub mod native;
use crate::system_search::Identity;
use rustcast_core::AutoQuitSettings;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Foreground,
    Background,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub application: String,
    pub process: Identity,
    pub activity: Activity,
}

/// A due candidate pins the original bundle and complete process lifetime.
/// Consumers must reobserve and recheck policy/safeguards before any effect.
#[derive(Clone, Debug)]
pub struct Due {
    pub application: String,
    pub process: Identity,
    generation: u64,
    owner: Arc<()>,
}
impl PartialEq for Due {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.generation == other.generation
            && self.application == other.application
            && self.process == other.process
    }
}
impl Eq for Due {}

struct Interval {
    application: String,
    minutes: u32,
    allow_unverified_activity: bool,
    deadline: Duration,
    generation: u64,
}

#[derive(Default)]
pub struct Tracker {
    owner: Arc<()>,
    intervals: HashMap<Identity, Interval>,
    generation: u64,
    previous: Option<Duration>,
    sleeping: bool,
}

impl Tracker {
    pub fn reset_application(&mut self, application: &str) {
        self.intervals.retain(|_, interval| !interval.application.eq_ignore_ascii_case(application));
    }

    /// Both sleep and wake retire every old candidate. The first observation
    /// after wake starts fresh; time spent asleep never expires an interval.
    pub fn set_sleeping(&mut self, sleeping: bool) {
        if self.sleeping != sleeping {
            self.intervals.clear();
            self.previous = None;
            self.sleeping = sleeping;
        }
    }

    /// `now` is monotonic elapsed time supplied by the caller (simulated in
    /// tests). Preferences come from validated configuration. A backward clock
    /// or ambiguous process snapshot resets state.
    /// Due candidates repeat while overdue; downstream admission/attempt policy
    /// must prevent repeated effects. This layer does not initiate a quit.
    pub fn observe(
        &mut self,
        now: Duration,
        applications: &[Observation],
        preferences: &BTreeMap<String, AutoQuitSettings>,
    ) -> Vec<Due> {
        if self.sleeping {
            return vec![];
        }
        if self.previous.is_some_and(|previous| now < previous) {
            self.intervals.clear();
        }
        self.previous = Some(now);
        let mut pids = HashSet::new();
        // One snapshot cannot safely describe two lifetimes for the same PID.
        // Reject the whole snapshot before retaining any overdue old interval.
        if applications.iter().any(|app| !pids.insert(app.process.pid)) {
            self.intervals.clear();
            return vec![];
        }
        let mut next = HashMap::new();
        let mut due = vec![];
        for application in applications {
            let preference = preferences
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(&application.application))
                .map(|(_, preference)| preference);
            let Some(preference) = preference.filter(|p| p.enabled && p.minutes > 0) else {
                continue;
            };
            if application.activity != Activity::Background
                || application.process.pid <= 1
                || application.process.seconds == 0
                || application.process.micros >= 1_000_000
            {
                continue;
            }
            let interval = self
                .intervals
                .remove(&application.process)
                .filter(|interval| {
                    interval
                        .application
                        .eq_ignore_ascii_case(&application.application)
                        && interval.minutes == preference.minutes
                        && interval.allow_unverified_activity == preference.allow_unverified_activity
                });
            let interval = match interval {
                Some(interval) => interval,
                None => {
                    let Some(deadline) =
                        now.checked_add(Duration::from_secs(u64::from(preference.minutes) * 60))
                    else {
                        continue;
                    };
                    let Some(generation) = self.generation.checked_add(1) else {
                        // Exhausted identity space fails closed, never reusing a ticket.
                        continue;
                    };
                    self.generation = generation;
                    Interval {
                        application: application.application.clone(),
                        minutes: preference.minutes,
                        allow_unverified_activity: preference.allow_unverified_activity,
                        deadline,
                        generation,
                    }
                }
            };
            if now >= interval.deadline {
                due.push(Due {
                    application: interval.application.clone(),
                    process: application.process,
                    generation: interval.generation,
                    owner: Arc::clone(&self.owner),
                });
            }
            next.insert(application.process, interval);
        }
        self.intervals = next;
        due
    }

    /// Tests the latest snapshot only. It cannot replace an immediate OS check
    /// or prove foreground/media/recording safety between observations.
    pub fn is_current(&self, due: &Due) -> bool {
        !self.sleeping
            && Arc::ptr_eq(&self.owner, &due.owner)
            && self.intervals.get(&due.process).is_some_and(|interval| {
                interval.generation == due.generation && interval.application == due.application
            })
    }
}

#[cfg(test)]
#[path = "../tests/support/auto_quit_owned.rs"]
mod owned_native_tests;


#[cfg(test)]
mod tests {
    use super::*;
    fn clock(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }
    fn app() -> Observation {
        Observation {
            application: "test.synthetic.Editor".into(),
            process: Identity {
                pid: 42,
                uid: 501,
                seconds: 100,
                micros: 25,
            },
            activity: Activity::Background,
        }
    }
    fn settings(minutes: u32) -> BTreeMap<String, AutoQuitSettings> {
        BTreeMap::from([(
            "test.synthetic.Editor".into(),
            AutoQuitSettings {
            allow_unverified_activity: false,
                enabled: true,
                minutes,
            },
        )])
    }
    #[test]
    fn expiry_pins_the_original_application_and_process_without_clock_drift() {
        let mut tracker = Tracker::default();
        let application = app();
        let settings = settings(2);
        assert!(tracker
            .observe(clock(5), &[application.clone()], &settings)
            .is_empty());
        assert!(tracker
            .observe(clock(124), &[application.clone()], &settings)
            .is_empty());
        let due = tracker
            .observe(clock(125), &[application.clone()], &settings)
            .pop()
            .unwrap();
        assert_eq!(due.application, application.application);
        assert_eq!(due.process, application.process);
        assert!(tracker.is_current(&due));
        assert_eq!(
            tracker.observe(clock(500), &[application], &settings),
            vec![due]
        );
    }
    #[test]
    fn foreground_and_unknown_reset_even_an_already_expired_candidate() {
        for activity in [Activity::Foreground, Activity::Unknown] {
            let mut tracker = Tracker::default();
            let mut application = app();
            let settings = settings(1);
            tracker.observe(clock(0), &[application.clone()], &settings);
            let due = tracker
                .observe(clock(60), &[application.clone()], &settings)
                .pop()
                .unwrap();
            application.activity = activity;
            assert!(tracker
                .observe(clock(61), &[application.clone()], &settings)
                .is_empty());
            assert!(!tracker.is_current(&due));
            application.activity = Activity::Background;
            assert!(tracker
                .observe(clock(61), &[application.clone()], &settings)
                .is_empty());
            assert!(tracker
                .observe(clock(120), &[application.clone()], &settings)
                .is_empty());
            assert_eq!(
                tracker.observe(clock(121), &[application], &settings).len(),
                1
            );
        }
    }
    #[test]
    fn disabled_removed_and_changed_preferences_start_a_new_interval() {
        let mut tracker = Tracker::default();
        let application = app();
        let mut settings = settings(1);
        tracker.observe(clock(0), &[application.clone()], &settings);
        let due = tracker
            .observe(clock(60), &[application.clone()], &settings)
            .pop()
            .unwrap();
        settings.values_mut().next().unwrap().enabled = false;
        assert!(tracker
            .observe(clock(61), &[application.clone()], &settings)
            .is_empty());
        assert!(!tracker.is_current(&due));
        settings.values_mut().next().unwrap().enabled = true;
        assert!(tracker
            .observe(clock(100), &[application.clone()], &settings)
            .is_empty());
        settings.values_mut().next().unwrap().minutes = 2;
        assert!(tracker
            .observe(clock(150), &[application.clone()], &settings)
            .is_empty());
        assert!(tracker
            .observe(clock(269), &[application.clone()], &settings)
            .is_empty());
        let due = tracker
            .observe(clock(270), &[application.clone()], &settings)
            .pop()
            .unwrap();
        settings.clear();
        assert!(tracker
            .observe(clock(271), &[application], &settings)
            .is_empty());
        assert!(!tracker.is_current(&due));
    }
    #[test]
    fn sleep_and_wake_discard_elapsed_time_and_old_candidates() {
        let mut tracker = Tracker::default();
        let application = app();
        let settings = settings(1);
        tracker.observe(clock(0), &[application.clone()], &settings);
        let due = tracker
            .observe(clock(60), &[application.clone()], &settings)
            .pop()
            .unwrap();
        tracker.set_sleeping(true);
        assert!(!tracker.is_current(&due));
        assert!(tracker
            .observe(clock(5000), &[application.clone()], &settings)
            .is_empty());
        tracker.set_sleeping(false);
        assert!(tracker
            .observe(clock(5001), &[application.clone()], &settings)
            .is_empty());
        assert!(tracker
            .observe(clock(5060), &[application.clone()], &settings)
            .is_empty());
        assert_eq!(
            tracker
                .observe(clock(5061), &[application], &settings)
                .len(),
            1
        );
        assert!(!tracker.is_current(&due));
    }
    #[test]
    fn process_restart_and_pid_reuse_never_reuse_an_expired_candidate() {
        for replace_pid in [false, true] {
            let mut tracker = Tracker::default();
            let mut application = app();
            let settings = settings(1);
            tracker.observe(clock(0), &[application.clone()], &settings);
            let due = tracker
                .observe(clock(60), &[application.clone()], &settings)
                .pop()
                .unwrap();
            if replace_pid {
                application.process.pid += 1;
            }
            application.process.seconds += 1;
            assert!(tracker
                .observe(clock(61), &[application.clone()], &settings)
                .is_empty());
            assert!(!tracker.is_current(&due));
            let replacement = tracker
                .observe(clock(121), &[application], &settings)
                .pop()
                .unwrap();
            assert_ne!(replacement.process, due.process);
            assert!(tracker.is_current(&replacement));
        }
    }
    #[test]
    fn missing_ambiguous_and_backward_observations_retire_old_candidates() {
        let application = app();
        let settings = settings(1);
        for snapshot in [vec![], vec![application.clone(), application.clone()]] {
            let mut tracker = Tracker::default();
            tracker.observe(clock(0), &[application.clone()], &settings);
            let due = tracker
                .observe(clock(60), &[application.clone()], &settings)
                .pop()
                .unwrap();
            assert!(tracker.observe(clock(61), &snapshot, &settings).is_empty());
            assert!(!tracker.is_current(&due));
        }
        let mut tracker = Tracker::default();
        tracker.observe(clock(10), &[application.clone()], &settings);
        let due = tracker
            .observe(clock(70), &[application.clone()], &settings)
            .pop()
            .unwrap();
        assert!(tracker
            .observe(clock(0), &[application.clone()], &settings)
            .is_empty());
        assert!(!tracker.is_current(&due));
        assert_eq!(
            tracker.observe(clock(60), &[application], &settings).len(),
            1
        );
    }
    #[test]
    fn conflicting_lifetimes_for_one_pid_retire_overdue_candidates_in_either_order() {
        for reverse in [false, true] {
            let old = app();
            let mut replacement = old.clone();
            replacement.process.seconds += 1;
            replacement.activity = Activity::Foreground;
            let settings = settings(1);
            let mut tracker = Tracker::default();
            tracker.observe(clock(0), &[old.clone()], &settings);
            let due = tracker
                .observe(clock(60), &[old.clone()], &settings)
                .pop()
                .unwrap();
            let mut snapshot = vec![old, replacement.clone()];
            if reverse {
                snapshot.reverse();
            }
            assert!(tracker.observe(clock(61), &snapshot, &settings).is_empty());
            assert!(!tracker.is_current(&due));
            replacement.activity = Activity::Background;
            assert!(tracker
                .observe(clock(62), &[replacement.clone()], &settings)
                .is_empty());
            assert!(tracker
                .observe(clock(121), &[replacement.clone()], &settings)
                .is_empty());
            let current = tracker
                .observe(clock(122), &[replacement.clone()], &settings)
                .pop()
                .unwrap();
            assert_eq!(current.process, replacement.process);
            assert!(tracker.is_current(&current));
            assert!(!tracker.is_current(&due));
        }
    }
    #[test]
    fn bundle_matching_is_case_insensitive_and_deadline_overflow_fails_closed() {
        let mut tracker = Tracker::default();
        let mut application = app();
        application.application.make_ascii_uppercase();
        let settings = settings(u32::MAX);
        assert!(tracker
            .observe(Duration::MAX, &[application.clone()], &settings)
            .is_empty());
        assert!(tracker
            .observe(clock(0), &[application.clone()], &settings)
            .is_empty());
        let due = tracker
            .observe(clock(u64::from(u32::MAX) * 60), &[application], &settings)
            .pop()
            .unwrap();
        assert!(tracker.is_current(&due));
    }
    #[test]
    fn replacement_tracker_never_accepts_the_previous_owners_candidate() {
        let application = app();
        let settings = settings(1);
        let mut previous = Tracker::default();
        previous.observe(clock(0), &[application.clone()], &settings);
        let old = previous
            .observe(clock(60), &[application.clone()], &settings)
            .pop()
            .unwrap();
        drop(previous);
        let mut replacement = Tracker::default();
        replacement.observe(clock(0), &[application.clone()], &settings);
        assert!(!replacement.is_current(&old));
        let new = replacement
            .observe(clock(60), &[application], &settings)
            .pop()
            .unwrap();
        assert_ne!(old, new);
        assert!(!replacement.is_current(&old));
        assert!(replacement.is_current(&new));
    }
}
