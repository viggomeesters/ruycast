//! A missing workspace wall deadline retains cancellation, never a busy spin.
use std::time::{Duration, Instant};
pub(crate) const POLL: Duration = Duration::from_millis(20);
pub(crate) fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}
pub(crate) fn poll(deadline: Option<Instant>) -> Duration {
    deadline.map_or(POLL, |d| {
        POLL.min(d.saturating_duration_since(Instant::now()))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_wall_deadline_still_has_a_bounded_cancellation_poll() {
        assert!(!expired(None));
        assert_eq!(poll(None), POLL);
        assert_eq!(poll(Some(Instant::now())), Duration::ZERO);
    }
}
