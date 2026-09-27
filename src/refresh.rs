//! When dox asks Docker for fresh lists. Docker events drive refreshes while
//! the event stream is up (plus a slow safety poll, since uptime text only
//! comes from the list call); without it dox polls every tick as before.
//! One refresh runs at a time, and requests made meanwhile are queued, not
//! dropped.

use std::time::{Duration, Instant};

/// A refresh that never replied is abandoned after this long.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// Ticks between safety polls while events flow (10s at the default 1s tick).
pub const SAFETY_POLL_TICKS: u64 = 10;
/// Without events, every Nth tick also refreshes images, volumes, networks.
pub const FULL_REFRESH_EVERY: u64 = 5;
/// Events arriving together (e.g. `compose up`) are folded into one refresh.
pub const EVENT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Serializes refreshes. `full` means images, volumes and networks too.
#[derive(Debug, Default)]
pub struct RefreshGate {
    in_flight: Option<Instant>,
    queued: Option<bool>,
}

impl RefreshGate {
    /// Ask for a refresh. `Some(full)` means start one now; `None` means it
    /// was queued behind the running one (a queued `full` is kept).
    pub fn request(&mut self, full: bool, now: Instant) -> Option<bool> {
        let running = self
            .in_flight
            .is_some_and(|started| now.saturating_duration_since(started) < STALE_AFTER);
        if running {
            self.queued = Some(self.queued.unwrap_or(false) || full);
            return None;
        }
        self.in_flight = Some(now);
        Some(full)
    }

    /// The running refresh delivered its lists. `Some(full)` means start the
    /// queued one now.
    pub fn finished(&mut self, now: Instant) -> Option<bool> {
        self.in_flight = None;
        let full = self.queued.take()?;
        self.in_flight = Some(now);
        Some(full)
    }
}

/// What the periodic tick should refresh: `Some(full)` or nothing.
pub fn on_tick(tick: u64, events_live: bool) -> Option<bool> {
    if events_live {
        tick.is_multiple_of(SAFETY_POLL_TICKS).then_some(false)
    } else {
        Some(tick.is_multiple_of(FULL_REFRESH_EVERY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_refresh_at_a_time_and_nothing_lost() {
        let t0 = Instant::now();
        let mut g = RefreshGate::default();
        assert_eq!(g.request(false, t0), Some(false));
        assert_eq!(g.request(false, t0), None, "queued behind the running one");
        assert_eq!(g.request(true, t0), None, "still queued, now full");
        assert_eq!(
            g.finished(t0),
            Some(true),
            "queued request starts, full kept"
        );
        assert_eq!(g.finished(t0), None, "nothing left");
        assert_eq!(g.request(false, t0), Some(false));
    }

    #[test]
    fn a_refresh_that_never_replies_is_abandoned() {
        let t0 = Instant::now();
        let mut g = RefreshGate::default();
        assert_eq!(g.request(false, t0), Some(false));
        assert_eq!(g.request(true, t0 + Duration::from_secs(11)), Some(true));
    }

    #[test]
    fn events_live_means_a_slow_containers_only_poll() {
        let polls: Vec<u64> = (1..=30).filter(|t| on_tick(*t, true).is_some()).collect();
        assert_eq!(polls, vec![10, 20, 30]);
        assert_eq!(on_tick(10, true), Some(false));
    }

    #[test]
    fn without_events_poll_every_tick_as_before() {
        assert_eq!(on_tick(1, false), Some(false));
        assert_eq!(on_tick(5, false), Some(true));
    }
}
