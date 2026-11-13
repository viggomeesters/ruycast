//! Integration compilation uses the product library without cfg(test), including
//! when test-tools is enabled. Its Decision type must have no admitting variant.
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
fn product_library_never_exposes_unit_fixture_admission() {
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
    let preferences = BTreeMap::from([(
        application.application.clone(),
        AutoQuitSettings {
            enabled: true,
            minutes: 1,
        },
    )]);
    let safety = Snapshot {
        application: application.clone(),
        sampled_at: Duration::from_secs(60),
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
        &[application],
        &preferences,
        &[safety],
    );
    assert_eq!(assessment.len(), 1);
    // Intentionally exhaustive: adding a product admission variant requires
    // changing this safety contract and independently reviewing that change.
    match assessment[0].decision {
        Decision::Blocked(reason) => assert_eq!(reason, Blocked::UnqualifiedCoverage),
    }
}
