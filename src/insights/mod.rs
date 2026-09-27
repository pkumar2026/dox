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
    patterns_by_severity: Vec<PatternMiner>,
    services: FrequencyTable,
    hosts: FrequencyTable,
    timeline: Timeline,
    stats: Stats,
    /// Lines per severity since the last `take_interval_counts`.
    interval: SeverityCounts,
}

impl Insights {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            words: FrequencyTable::new(MEMORY_SIZE),
            attributes: AttributeTable::new(MEMORY_SIZE, MAX_VALUES_PER_KEY),
            patterns: PatternMiner::new(),
            patterns_by_severity: Severity::ALL.iter().map(|_| PatternMiner::new()).collect(),
            services: FrequencyTable::new(MEMORY_SIZE),
            hosts: FrequencyTable::new(MEMORY_SIZE),
            timeline: Timeline::new(),
            stats: Stats::new(),
            interval: [0; 6],
        }
    }

    /// Counts per severity since the previous call (one Log Counts bar).
    pub fn take_interval_counts(&mut self) -> SeverityCounts {
        std::mem::take(&mut self.interval)
    }

    /// Forget everything (container switch, stream restart, `r`).
    pub fn reset(&mut self, now: Instant) {
        *self = Self::new(now);
    }

    /// Count one log line that arrived at `now`.
    pub fn observe(&mut self, entry: &LogLine, now: Instant) {
        let line = parse_entry(&entry.raw, entry.ts_len, entry.level);
        let second = now.saturating_duration_since(self.started).as_secs();
        self.stats.add(second, entry.raw.len());
        for word in words::extract_words(&line.message) {
            self.words.add(&word);
        }
        self.attributes.add(&line.attributes);
        self.patterns.add(&line.message);
        self.patterns_by_severity[line.severity.index()].add(&line.message);
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

    pub fn snapshot(&self, now: Instant) -> InsightsSnapshot {
        let elapsed = now.saturating_duration_since(self.started).as_secs();
        InsightsSnapshot {
            words: self.words.top(SNAPSHOT_ROWS),
            word_count: self.words.len(),
            attribute_count: self.attributes.len(),
            attributes: self.attributes.top(SNAPSHOT_ROWS),
            patterns: self.patterns.top(SNAPSHOT_ROWS),
            pattern_count: self.patterns.len(),
            pattern_lines: self.patterns.total(),
            pattern_overflow: self.patterns.overflow(),
            per_minute: self.timeline.window(elapsed / 60),
            totals: self.timeline.totals(),
            by_severity: Severity::ALL
                .iter()
                .map(|s| SeverityBreakdown {
                    patterns: self.patterns_by_severity[s.index()].top(PER_SEVERITY_TOP),
                    services: self.timeline.top_services(*s, PER_SEVERITY_TOP),
                })
                .collect(),
            services: self.services.top(SNAPSHOT_ROWS),
            hosts: self.hosts.top(SNAPSHOT_ROWS),
            total_lines: self.stats.total_lines,
            total_bytes: self.stats.total_bytes,
            current_rate: self.stats.current_rate(elapsed),
            peak_rate: self.stats.peak_rate(),
            uptime_secs: elapsed,
        }
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
