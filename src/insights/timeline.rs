//! Per-minute counts by severity over the last hour (by arrival time, like
//! Gonzo), lifetime totals, and which services produce each severity.

use std::collections::VecDeque;

use super::frequency::FrequencyTable;
use super::severity::Severity;

/// Minutes kept for the chart and the heatmap.
pub const WINDOW_MINUTES: usize = 60;
/// Distinct services remembered per severity.
const MAX_SERVICES: usize = 1_000;

pub type SeverityCounts = [u64; 6];

pub struct Timeline {
    /// `(minute number, counts)`, oldest first, at most `WINDOW_MINUTES`.
    minutes: VecDeque<(u64, SeverityCounts)>,
    totals: SeverityCounts,
    services: Vec<FrequencyTable>,
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            minutes: VecDeque::with_capacity(WINDOW_MINUTES),
            totals: [0; 6],
            services: Severity::ALL
                .iter()
                .map(|_| FrequencyTable::new(MAX_SERVICES))
                .collect(),
        }
    }

    /// Count one line that arrived during `minute` (minutes since start).
    pub fn add(&mut self, minute: u64, severity: Severity, service: Option<&str>) {
        let idx = severity.index();
        match self.minutes.back_mut() {
            Some((m, counts)) if *m >= minute => counts[idx] += 1,
            _ => {
                let mut counts = [0; 6];
                counts[idx] = 1;
                self.minutes.push_back((minute, counts));
            }
        }
        while self
            .minutes
            .front()
            .is_some_and(|(m, _)| m + WINDOW_MINUTES as u64 <= minute)
        {
            self.minutes.pop_front();
        }
        self.totals[idx] += 1;
        if let Some(service) = service {
            self.services[idx].add(service);
        }
    }

    pub fn totals(&self) -> SeverityCounts {
        self.totals
    }

    /// Counts for the `WINDOW_MINUTES` minutes ending at `now_minute`,
    /// oldest first, with empty minutes as zeros.
    pub fn window(&self, now_minute: u64) -> Vec<SeverityCounts> {
        (0..WINDOW_MINUTES as u64)
            .map(|i| {
                let back = WINDOW_MINUTES as u64 - 1 - i;
                now_minute
                    .checked_sub(back)
                    .and_then(|m| self.minutes.iter().find(|(mm, _)| *mm == m))
                    .map(|(_, counts)| *counts)
                    .unwrap_or([0; 6])
            })
            .collect()
    }

    pub fn top_services(&self, severity: Severity, n: usize) -> Vec<(String, u64)> {
        self.services[severity.index()].top(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_land_in_their_minute_and_totals() {
        let mut t = Timeline::new();
        t.add(0, Severity::Info, None);
        t.add(0, Severity::Error, None);
        t.add(2, Severity::Error, None);
        let w = t.window(2);
        assert_eq!(w.len(), WINDOW_MINUTES);
        let last = w[WINDOW_MINUTES - 1];
        let first_used = w[WINDOW_MINUTES - 3];
        assert_eq!(last[Severity::Error.index()], 1);
        assert_eq!(first_used[Severity::Error.index()], 1);
        assert_eq!(first_used[Severity::Info.index()], 1);
        assert_eq!(w[WINDOW_MINUTES - 2], [0; 6]);
        assert_eq!(t.totals()[Severity::Error.index()], 2);
    }

    #[test]
    fn minutes_older_than_the_window_fall_off() {
        let mut t = Timeline::new();
        t.add(0, Severity::Warn, None);
        t.add(100, Severity::Warn, None);
        let w = t.window(100);
        assert_eq!(w.iter().map(|c| c[Severity::Warn.index()]).sum::<u64>(), 1);
        assert_eq!(t.totals()[Severity::Warn.index()], 2);
    }

    #[test]
    fn services_per_severity() {
        let mut t = Timeline::new();
        t.add(0, Severity::Error, Some("api"));
        t.add(0, Severity::Error, Some("api"));
        t.add(0, Severity::Error, Some("worker"));
        t.add(0, Severity::Info, Some("web"));
        assert_eq!(
            t.top_services(Severity::Error, 3),
            vec![("api".to_string(), 2), ("worker".to_string(), 1)]
        );
        assert_eq!(t.top_services(Severity::Info, 3).len(), 1);
    }
}
