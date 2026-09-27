//! Totals and processing rate for the Stats popup.

use std::collections::VecDeque;

/// Seconds averaged for "current" rate.
const RATE_WINDOW_SECS: u64 = 5;

pub struct Stats {
    pub total_lines: u64,
    pub total_bytes: u64,
    /// `(second number, lines)`, recent seconds only.
    per_second: VecDeque<(u64, u64)>,
    peak_per_second: u64,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            total_lines: 0,
            total_bytes: 0,
            per_second: VecDeque::new(),
            peak_per_second: 0,
        }
    }

    /// Count one line of `bytes` that arrived during `second` (since start).
    pub fn add(&mut self, second: u64, bytes: usize) {
        self.total_lines += 1;
        self.total_bytes += bytes as u64;
        match self.per_second.back_mut() {
            Some((s, count)) if *s >= second => *count += 1,
            _ => self.per_second.push_back((second, 1)),
        }
        if let Some((_, count)) = self.per_second.back() {
            self.peak_per_second = self.peak_per_second.max(*count);
        }
        while self
            .per_second
            .front()
            .is_some_and(|(s, _)| s + RATE_WINDOW_SECS <= second)
        {
            self.per_second.pop_front();
        }
    }

    /// Average lines per second over the last few seconds up to `now_second`.
    pub fn current_rate(&self, now_second: u64) -> f64 {
        let from = (now_second + 1).saturating_sub(RATE_WINDOW_SECS);
        let lines: u64 = self
            .per_second
            .iter()
            .filter(|(s, _)| *s >= from && *s <= now_second)
            .map(|(_, count)| count)
            .sum();
        lines as f64 / RATE_WINDOW_SECS as f64
    }

    pub fn peak_rate(&self) -> u64 {
        self.peak_per_second
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_and_rates() {
        let mut s = Stats::new();
        for _ in 0..10 {
            s.add(3, 100);
        }
        for _ in 0..5 {
            s.add(4, 10);
        }
        assert_eq!(s.total_lines, 15);
        assert_eq!(s.total_bytes, 1050);
        assert_eq!(s.peak_rate(), 10);
        // seconds 0..=4 averaged over the 5-second window
        assert!((s.current_rate(4) - 3.0).abs() < 0.01);
        // long after the burst, the current rate drops to zero
        assert_eq!(s.current_rate(60), 0.0);
    }
}
