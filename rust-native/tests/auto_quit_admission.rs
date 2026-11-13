//! Product compilation: enabled alone never accepts unverified activity.
//! The exhaustive match has no unit-only OwnedFixture variant.
use rustcast_core::AutoQuitSettings;
use ruycast_native::{
    auto_quit::{
        safety::{Blocked, Decision, Guard, Helpers, Signal, Snapshot},
        Activity, Observation, Tracker,
    },
    system_search::Identity,
};
use std::{collections::BTreeMap, time::Duration};

#[test]
fn product_library_requires_separate_unverified_activity_acknowledgement() {
    let application = Observation {
        application: "test.synthetic.NoCapture".into(),
        process: Identity {
            pid: 42,
            uid: 501,
            seconds: 100,
            micros: 25,
        },
        activity: Activity::Background,
    };
    let mut preferences = BTreeMap::from([(
        application.application.clone(),
        AutoQuitSettings {
            allow_unverified_activity: false,
            enabled: true,
            minutes: 1,
        },
    )]);
    let mut safety = Snapshot {
        application: application.clone(),
        sampled_at: Duration::from_secs(60),
        running: Signal::Inactive,
        input: Signal::Inactive,
        output: Signal::Inactive,
        recording: Signal::Inactive,
        helpers: Helpers::Complete,
    };
    let guard = Guard::default();
    let mut tracker = Tracker::default();
    tracker.observe_guarded(
        &guard,
        Duration::ZERO,
        std::slice::from_ref(&application),
        &preferences,
        &[],
    );
    let assessment = tracker.observe_guarded(
        &guard,
        Duration::from_secs(60),
        std::slice::from_ref(&application),
        &preferences,
        std::slice::from_ref(&safety),
    );
    assert_eq!(assessment.len(), 1);
    // Intentionally exhaustive: product admission needs explicit consent; the
    // unit-only synthetic fixture variant is absent in either product build.
    match assessment[0].decision {
        Decision::Blocked(reason) => assert_eq!(reason, Blocked::UnqualifiedCoverage),
        Decision::AllowedUnverified => {
            panic!("enabled alone must never acknowledge unverified activity")
        }
    }

    preferences
        .values_mut()
        .next()
        .unwrap()
        .allow_unverified_activity = true;
    assert!(tracker
        .observe(
            Duration::from_secs(60),
            std::slice::from_ref(&application),
            &preferences
        )
        .is_empty());
    let due = tracker
        .observe(
            Duration::from_secs(120),
            std::slice::from_ref(&application),
            &preferences,
        )
        .pop()
        .unwrap();
    safety.sampled_at = Duration::from_secs(120);
    safety.running = Signal::Unknown;
    safety.input = Signal::Unknown;
    safety.output = Signal::Unknown;
    safety.recording = Signal::Unknown;
    safety.helpers = Helpers::Unknown;
    assert_eq!(
        guard.assess(
            &tracker,
            &due,
            Duration::from_secs(120),
            &preferences,
            Some(&safety)
        ),
        Decision::AllowedUnverified
    );
    for (signal, reason) in [
        (Signal::Active, Blocked::ActiveAudio),
        (Signal::Failed, Blocked::FailedSafety),
        (Signal::Unavailable, Blocked::UnavailableSafety),
    ] {
        safety.running = signal;
        assert_eq!(
            guard.assess(
                &tracker,
                &due,
                Duration::from_secs(120),
                &preferences,
                Some(&safety)
            ),
            Decision::Blocked(reason)
        );
    }
}
