//! Fail-closed admission for real Tracker candidates. No sensor/effect fiction.
//!
//! Recording/helper coverage remains incomplete. Enabled alone never admits;
//! an independently stored per-app acknowledgement may accept unknown coverage.
//! Positive activity and failed/unavailable observations still block.
use super::{Activity, Due, Observation, Tracker};
use rustcast_core::AutoQuitSettings;
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Active,
    Inactive,
    Unknown,
    Unavailable,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helpers {
    Complete,
    Unknown,
    Unavailable,
    Failed,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub application: Observation,
    pub sampled_at: Duration,
    pub running: Signal,
    pub input: Signal,
    pub output: Signal,
    pub recording: Signal,
    pub helpers: Helpers,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    StaleTimer,
    Disabled,
    PreferenceChanged,
    MissingSafety,
    IdentityChanged,
    NotBackground,
    StaleSafety,
    ActiveAudio,
    ActiveRecording,
    UnknownSafety,
    UnavailableSafety,
    FailedSafety,
    UnqualifiedCoverage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Blocked(Blocked),
    /// Explicit per-app acknowledgement; never inferred from enabled=true.
    AllowedUnverified,
    /// Synthetic qualification remains unit-only and does not provide product
    /// acknowledgement or a fixture allowlist in test-tools binaries.
    #[cfg(test)]
    OwnedFixture,
}
pub struct Assessment {
    pub candidate: Due,
    pub decision: Decision,
}
#[derive(Default)]
pub struct Guard {
    #[cfg(test)]
    fixture: Option<(String, crate::system_search::Identity)>,
}
impl Guard {
    /// Checks current timer and configuration again. At most one second of
    /// monotonic sample age is admitted to inspection, never to an OS effect.
    /// #141 must acquire fresh live state and recheck immediately before quit.
    pub fn assess(
        &self,
        tracker: &Tracker,
        due: &Due,
        now: Duration,
        preferences: &BTreeMap<String, AutoQuitSettings>,
        snapshot: Option<&Snapshot>,
    ) -> Decision {
        use Blocked::*;
        let block = Decision::Blocked;
        if !tracker.is_current(due) || tracker.previous.is_none_or(|previous| now < previous) {
            return block(StaleTimer);
        }
        let preference = preferences
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&due.application))
            .map(|(_, value)| value);
        let Some(preference) = preference.filter(|p| p.enabled && p.minutes > 0) else {
            return block(Disabled);
        };
        if tracker.intervals[&due.process].minutes != preference.minutes
            || tracker.intervals[&due.process].allow_unverified_activity
                != preference.allow_unverified_activity
        {
            return block(PreferenceChanged);
        }
        let Some(snapshot) = snapshot else {
            return block(MissingSafety);
        };
        if snapshot.application.process != due.process
            || !snapshot
                .application
                .application
                .eq_ignore_ascii_case(&due.application)
        {
            return block(IdentityChanged);
        }
        if snapshot.application.activity != Activity::Background {
            return block(NotBackground);
        }
        if now
            .checked_sub(snapshot.sampled_at)
            .is_none_or(|age| age > Duration::from_secs(1))
        {
            return block(StaleSafety);
        }
        if snapshot.running == Signal::Active
            || snapshot.input == Signal::Active
            || snapshot.output == Signal::Active
        {
            return block(ActiveAudio);
        }
        if snapshot.recording == Signal::Active {
            return block(ActiveRecording);
        }
        let signals = [
            snapshot.running,
            snapshot.input,
            snapshot.output,
            snapshot.recording,
        ];
        if signals.contains(&Signal::Failed) || snapshot.helpers == Helpers::Failed {
            return block(FailedSafety);
        }
        if signals.contains(&Signal::Unavailable) || snapshot.helpers == Helpers::Unavailable {
            return block(UnavailableSafety);
        }
        if preference.allow_unverified_activity {
            return Decision::AllowedUnverified;
        }
        if signals.contains(&Signal::Unknown) || snapshot.helpers == Helpers::Unknown {
            return block(UnknownSafety);
        }
        #[cfg(test)]
        if self.fixture.as_ref().is_some_and(|(app, process)| {
            app.eq_ignore_ascii_case(&due.application) && *process == due.process
        }) {
            return Decision::OwnedFixture;
        }
        block(UnqualifiedCoverage)
    }
}
impl Tracker {
    /// The clock-to-policy path. Candidates never become termination actions;
    /// product effects still require final live state and explicit acknowledgement.
    pub fn observe_guarded(
        &mut self,
        guard: &Guard,
        now: Duration,
        applications: &[Observation],
        preferences: &BTreeMap<String, AutoQuitSettings>,
        safety: &[Snapshot],
    ) -> Vec<Assessment> {
        self.observe(now, applications, preferences)
            .into_iter()
            .map(|candidate| {
                let mut matches = safety
                    .iter()
                    .filter(|s| s.application.process.pid == candidate.process.pid);
                let first = matches.next();
                // Multiple records for one PID are ambiguous, including PID reuse.
                let snapshot = if matches.next().is_none() {
                    first
                } else {
                    None
                };
                let decision = guard.assess(self, &candidate, now, preferences, snapshot);
                Assessment {
                    candidate,
                    decision,
                }
            })
            .collect()
    }
}

#[cfg(test)]
pub(super) fn owned_fixture_guard(application: &Observation) -> Guard {
    Guard {
        fixture: Some((application.application.clone(), application.process)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_search::Identity;
    fn app() -> Observation {
        Observation {
            application: "test.synthetic.NoCapture".into(),
            process: Identity {
                pid: 42,
                uid: 501,
                seconds: 100,
                micros: 25,
            },
            activity: Activity::Background,
        }
    }
    fn preferences() -> BTreeMap<String, AutoQuitSettings> {
        BTreeMap::from([(
            "test.synthetic.NoCapture".into(),
            AutoQuitSettings {
                allow_unverified_activity: false,
                enabled: true,
                minutes: 1,
            },
        )])
    }
    fn snapshot(app: Observation) -> Snapshot {
        Snapshot {
            application: app,
            sampled_at: Duration::from_secs(60),
            running: Signal::Inactive,
            input: Signal::Inactive,
            output: Signal::Inactive,
            recording: Signal::Inactive,
            helpers: Helpers::Complete,
        }
    }
    #[test]
    fn real_clock_candidates_flow_through_production_default_deny() {
        let mut tracker = Tracker::default();
        let app = app();
        let prefs = preferences();
        let guard = Guard::default();
        assert!(tracker
            .observe_guarded(&guard, Duration::ZERO, &[app.clone()], &prefs, &[])
            .is_empty());
        let assessment = tracker.observe_guarded(
            &guard,
            Duration::from_secs(60),
            &[app.clone()],
            &prefs,
            &[snapshot(app.clone())],
        );
        assert_eq!(assessment.len(), 1);
        assert_eq!(assessment[0].candidate.process, app.process);
        assert_eq!(
            assessment[0].decision,
            Decision::Blocked(Blocked::UnqualifiedCoverage)
        );
    }
    #[test]
    fn owned_fixture_never_overrides_audio_recording_unknown_or_failed_information() {
        let app = app();
        let prefs = preferences();
        let mut tracker = Tracker::default();
        tracker.observe(Duration::ZERO, &[app.clone()], &prefs);
        let due = tracker
            .observe(Duration::from_secs(60), &[app.clone()], &prefs)
            .pop()
            .unwrap();
        let guard = Guard {
            fixture: Some((app.application.clone(), app.process)),
        };
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&snapshot(app.clone()))
            ),
            Decision::OwnedFixture
        );
        for (signal, reason) in [
            (Signal::Active, Blocked::ActiveAudio),
            (Signal::Unknown, Blocked::UnknownSafety),
            (Signal::Unavailable, Blocked::UnavailableSafety),
            (Signal::Failed, Blocked::FailedSafety),
        ] {
            for input in [false, true] {
                let mut state = snapshot(app.clone());
                if input {
                    state.input = signal;
                } else {
                    state.output = signal;
                }
                assert_eq!(
                    guard.assess(
                        &tracker,
                        &due,
                        Duration::from_secs(60),
                        &prefs,
                        Some(&state)
                    ),
                    Decision::Blocked(reason)
                );
            }
        }
        let mut state = snapshot(app.clone());
        state.recording = Signal::Active;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::ActiveRecording)
        );
        let mut state = snapshot(app.clone());
        state.recording = Signal::Unknown;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::UnknownSafety)
        );
        for (helpers, reason) in [
            (Helpers::Unknown, Blocked::UnknownSafety),
            (Helpers::Unavailable, Blocked::UnavailableSafety),
            (Helpers::Failed, Blocked::FailedSafety),
        ] {
            let mut state = snapshot(app.clone());
            state.helpers = helpers;
            assert_eq!(
                guard.assess(
                    &tracker,
                    &due,
                    Duration::from_secs(60),
                    &prefs,
                    Some(&state)
                ),
                Decision::Blocked(reason)
            );
        }
        assert_eq!(
            guard.assess(&tracker, &due, Duration::from_secs(60), &prefs, None),
            Decision::Blocked(Blocked::MissingSafety)
        );
    }
    #[test]
    fn stale_future_replaced_foreground_and_disabled_state_never_qualify() {
        let app = app();
        let mut prefs = preferences();
        let mut tracker = Tracker::default();
        tracker.observe(Duration::ZERO, &[app.clone()], &prefs);
        let due = tracker
            .observe(Duration::from_secs(60), &[app.clone()], &prefs)
            .pop()
            .unwrap();
        let guard = Guard {
            fixture: Some((app.application.clone(), app.process)),
        };
        let state = snapshot(app.clone());
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(62),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::StaleSafety)
        );
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(59),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::StaleTimer)
        );
        let mut future = state.clone();
        future.sampled_at = Duration::from_secs(61);
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&future)
            ),
            Decision::Blocked(Blocked::StaleSafety)
        );
        let mut changed = state.clone();
        changed.application.process.seconds += 1;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&changed)
            ),
            Decision::Blocked(Blocked::IdentityChanged)
        );
        changed = state.clone();
        changed.application.activity = Activity::Foreground;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&changed)
            ),
            Decision::Blocked(Blocked::NotBackground)
        );
        prefs.values_mut().next().unwrap().enabled = false;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::Disabled)
        );
        prefs.values_mut().next().unwrap().enabled = true;
        prefs.values_mut().next().unwrap().minutes = 2;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::PreferenceChanged)
        );
        tracker.set_sleeping(true);
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(60),
                &prefs,
                Some(&state)
            ),
            Decision::Blocked(Blocked::StaleTimer)
        );
    }
    #[test]
    fn other_fixture_and_ambiguous_safety_snapshots_remain_blocked() {
        let app = app();
        let prefs = preferences();
        let mut tracker = Tracker::default();
        let mut other = app.process;
        other.seconds += 1;
        let guard = Guard {
            fixture: Some((app.application.clone(), other)),
        };
        tracker.observe(Duration::ZERO, &[app.clone()], &prefs);
        let mut replacement = snapshot(app.clone());
        replacement.application.process.seconds += 1;
        let outcomes = tracker.observe_guarded(
            &guard,
            Duration::from_secs(60),
            &[app.clone()],
            &prefs,
            &[snapshot(app.clone())],
        );
        assert_eq!(
            outcomes[0].decision,
            Decision::Blocked(Blocked::UnqualifiedCoverage)
        );
        let outcomes = tracker.observe_guarded(
            &guard,
            Duration::from_secs(60),
            &[app.clone()],
            &prefs,
            &[snapshot(app), replacement],
        );
        assert_eq!(
            outcomes[0].decision,
            Decision::Blocked(Blocked::MissingSafety)
        );
    }
}
