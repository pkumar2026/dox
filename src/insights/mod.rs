//! Native log insights (Gonzo's dashboard, in dox): top words, fields,
//! patterns, per-minute severity counts and processing stats, computed from
//! the log lines dox already streams.

pub mod attributes;
pub mod frequency;
pub mod parse;
pub mod patterns;
pub mod severity;
pub mod stats;
pub mod timeline;
pub mod words;

use std::time::Instant;

use crate::ui::logs::LogLine;
use attributes::{AttributeSummary, AttributeTable};
use frequency::FrequencyTable;
use parse::parse_entry;
use patterns::{PatternInfo, PatternMiner};
use severity::Severity;
use stats::Stats;
use timeline::{SeverityCounts, Timeline};

/// Gonzo's default memory size: distinct words, keys, services, hosts kept.
const MEMORY_SIZE: usize = 10_000;
/// Distinct values kept per field key.
const MAX_VALUES_PER_KEY: usize = 1_000;
/// Rows kept in each snapshot list (the boxes and popups scroll within these).
const SNAPSHOT_ROWS: usize = 100;
/// Patterns and services shown per severity in the Counts analysis popup.
const PER_SEVERITY_TOP: usize = 3;

/// Top patterns and services for one severity (Counts analysis popup).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeverityBreakdown {
    pub patterns: Vec<PatternInfo>,
    pub services: Vec<(String, u64)>,
}

/// Everything the insights UI draws, computed on the refresh timer.
#[derive(Debug, Clone, Default)]
pub struct InsightsSnapshot {
    /// Engine generation the lists were computed at (see `Insights::generation`).
    pub generation: u64,
    pub words: Vec<(String, u64)>,
    /// Distinct words / field keys seen (the lists above are capped).
    pub word_count: usize,
    pub attribute_count: usize,
    pub attributes: Vec<AttributeSummary>,
    pub patterns: Vec<PatternInfo>,
    pub pattern_count: usize,
    pub pattern_lines: u64,
    pub pattern_overflow: u64,
    /// Per-minute counts for the last hour, oldest first.
    pub per_minute: Vec<SeverityCounts>,
    pub totals: SeverityCounts,
    pub by_severity: Vec<SeverityBreakdown>,
    pub services: Vec<(String, u64)>,
    pub hosts: Vec<(String, u64)>,
    pub total_lines: u64,
    pub total_bytes: u64,
    pub current_rate: f64,
    pub peak_rate: u64,
    pub uptime_secs: u64,
}

pub struct Insights {
    started: Instant,
    words: FrequencyTable,
    attributes: AttributeTable,
    patterns: PatternMiner,
    services: FrequencyTable,
    hosts: FrequencyTable,
    timeline: Timeline,
    stats: Stats,
    /// Lines per severity since the last `take_interval_counts`.
    interval: SeverityCounts,
    /// Goes up on every line and every reset, so a snapshot can tell whether
    /// its lists are stale (a line count alone repeats after a reset).
    generation: u64,
}

impl Insights {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            words: FrequencyTable::new(MEMORY_SIZE),
            attributes: AttributeTable::new(MEMORY_SIZE, MAX_VALUES_PER_KEY),
            patterns: PatternMiner::new(),
            services: FrequencyTable::new(MEMORY_SIZE),
            hosts: FrequencyTable::new(MEMORY_SIZE),
            timeline: Timeline::new(),
            stats: Stats::new(),
            interval: [0; 6],
            generation: 0,
        }
    }

    /// Counts per severity since the previous call (one Log Counts bar).
    pub fn take_interval_counts(&mut self) -> SeverityCounts {
        std::mem::take(&mut self.interval)
    }

    /// Forget everything (container switch, stream restart, `r`).
    pub fn reset(&mut self, now: Instant) {
        let generation = self.generation + 1;
        *self = Self::new(now);
        self.generation = generation;
    }

    /// Count one log line that arrived at `now`.
    pub fn observe(&mut self, entry: &LogLine, now: Instant) {
        self.generation += 1;
        let line = parse_entry(&entry.raw, entry.ts_len, entry.level);
        let second = now.saturating_duration_since(self.started).as_secs();
        self.stats.add(second, entry.raw.len());
        words::for_each_word(&line.message, |word| self.words.add(word));
        self.attributes.add(&line.attributes);
        self.patterns.add(&line.message, line.severity);
        if let Some(service) = &line.service {
            self.services.add(service);
        }
        if let Some(host) = &line.host {
            self.hosts.add(host);
        }
        // Gonzo falls back to the host when a line names no service.
        let origin = line.service.as_deref().or(line.host.as_deref());
        self.timeline.add(second / 60, line.severity, origin);
        self.interval[line.severity.index()] += 1;
    }

    /// Values of one field, most common first (field values popup).
    pub fn attribute_values(&self, key: &str, n: usize) -> Vec<(String, u64)> {
        self.attributes.values(key, n)
    }

    /// Update `snap` for `now`. The sorted lists are only rebuilt when lines
    /// arrived (or a reset happened) since `snap` was taken; time-based fields
    /// (per-minute window, rate, uptime) always move on.
    pub fn refresh_into(&self, snap: &mut InsightsSnapshot, now: Instant) {
        let elapsed = now.saturating_duration_since(self.started).as_secs();
        if snap.generation != self.generation {
            snap.generation = self.generation;
            snap.words = self.words.top(SNAPSHOT_ROWS);
            snap.word_count = self.words.len();
            snap.attribute_count = self.attributes.len();
            snap.attributes = self.attributes.top(SNAPSHOT_ROWS);
            snap.patterns = self.patterns.top(SNAPSHOT_ROWS);
            snap.pattern_count = self.patterns.len();
            snap.pattern_lines = self.patterns.total();
            snap.pattern_overflow = self.patterns.overflow();
            snap.by_severity = Severity::ALL
                .iter()
                .map(|s| SeverityBreakdown {
                    patterns: self.patterns.top_for(*s, PER_SEVERITY_TOP),
                    services: self.timeline.top_services(*s, PER_SEVERITY_TOP),
                })
                .collect();
            snap.services = self.services.top(SNAPSHOT_ROWS);
            snap.hosts = self.hosts.top(SNAPSHOT_ROWS);
            snap.totals = self.timeline.totals();
            snap.total_lines = self.stats.total_lines;
            snap.total_bytes = self.stats.total_bytes;
            snap.peak_rate = self.stats.peak_rate();
        }
        snap.per_minute = self.timeline.window(elapsed / 60);
        snap.current_rate = self.stats.current_rate(elapsed);
        snap.uptime_secs = elapsed;
    }

    #[cfg(test)]
    pub fn snapshot(&self, now: Instant) -> InsightsSnapshot {
        let mut snap = InsightsSnapshot {
            generation: u64::MAX,
            ..InsightsSnapshot::default()
        };
        self.refresh_into(&mut snap, now);
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &[&str] = &[
        "2026-09-24T15:12:57.1Z [info     ] request finished [http.access] method=GET status_code=200",
        "2026-09-24T15:12:57.2Z [info     ] request finished [http.access] method=GET status_code=200",
        "2026-09-24T15:12:57.3Z [error    ] request failed [http.access] method=POST status_code=500",
        r#"2026-09-24T15:12:57.4Z {"level":"warn","msg":"slow query took long","service":"api","host":"h1"}"#,
    ];

    fn fed() -> (Insights, Instant) {
        let now = Instant::now();
        let mut i = Insights::new(now);
        for l in LINES {
            i.observe(&LogLine::from_raw(l.to_string()), now);
        }
        (i, now)
    }

    #[test]
    fn snapshot_reflects_observed_lines() {
        let (i, now) = fed();
        let s = i.snapshot(now);
        assert_eq!(s.total_lines, 4);
        assert_eq!(s.totals[Severity::Info.index()], 2);
        assert_eq!(s.totals[Severity::Error.index()], 1);
        assert_eq!(s.totals[Severity::Warn.index()], 1);
        assert!(
            s.words.contains(&("request".to_string(), 3)),
            "{:?}",
            s.words
        );
        assert!(
            !s.words.iter().any(|(w, _)| w == "get"),
            "stop word counted"
        );
        assert!(s
            .attributes
            .iter()
            .any(|a| a.key == "status_code" && a.unique == 2));
        assert!(s.pattern_count >= 2);
        assert_eq!(s.services, vec![("api".to_string(), 1)]);
        assert_eq!(s.hosts, vec![("h1".to_string(), 1)]);
        assert_eq!(s.per_minute.len(), timeline::WINDOW_MINUTES);
        assert_eq!(s.by_severity.len(), 6);
        assert_eq!(s.by_severity[Severity::Error.index()].patterns.len(), 1);
    }

    #[test]
    fn attribute_values_are_available_on_demand() {
        let (i, _) = fed();
        assert_eq!(
            i.attribute_values("status_code", 5),
            vec![("200".to_string(), 2), ("500".to_string(), 1)]
        );
    }

    #[test]
    fn interval_counts_are_taken_once() {
        let (mut i, _) = fed();
        let first = i.take_interval_counts();
        assert_eq!(first[Severity::Info.index()], 2);
        assert_eq!(first[Severity::Error.index()], 1);
        assert_eq!(i.take_interval_counts(), [0; 6]);
    }

    #[test]
    fn refresh_into_updates_time_fields_and_reuses_unchanged_lists() {
        let (i, now) = fed();
        let mut snap = InsightsSnapshot::default();
        i.refresh_into(&mut snap, now);
        let words = snap.words.clone();
        assert!(!words.is_empty());
        // Nothing new: lists stay; time-based fields still move on.
        snap.words.push(("sentinel".into(), 0));
        i.refresh_into(&mut snap, now + std::time::Duration::from_secs(90));
        assert_eq!(snap.words.last().map(|(w, _)| w.as_str()), Some("sentinel"));
        assert_eq!(snap.uptime_secs, 90);
    }

    #[test]
    fn a_switch_with_the_same_line_count_still_rebuilds_the_lists() {
        let now = Instant::now();
        let mut i = Insights::new(now);
        let mut snap = InsightsSnapshot::default();
        i.observe(&LogLine::from_raw("alpha alpha".into()), now);
        i.refresh_into(&mut snap, now);
        assert_eq!(snap.words[0].0, "alpha");
        i.reset(now);
        i.observe(&LogLine::from_raw("omega omega".into()), now);
        i.refresh_into(&mut snap, now);
        assert_eq!(
            snap.words[0].0, "omega",
            "old container's words must not linger"
        );
    }

    #[test]
    fn reset_forgets_everything() {
        let (mut i, now) = fed();
        i.reset(now);
        let s = i.snapshot(now);
        assert_eq!(s.total_lines, 0);
        assert!(s.words.is_empty());
        assert!(s.patterns.is_empty());
        assert_eq!(s.totals, [0; 6]);
    }
}
