//! State of the Gonzo-style insights UI: the side panel's sections, popups,
//! filter/search input, log columns and the refresh timer.

pub mod cursor;
pub mod detail;
pub mod keys;
pub mod picker;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::insights::timeline::SeverityCounts;
use crate::insights::{Insights, InsightsSnapshot};
use crate::logview::{FilteredIds, LogFilter};
use crate::ui::logs::LogBuffer;
use picker::{Columns, Picker};

/// Gonzo's refresh intervals (`u` slower, `U` faster).
/// Log Counts bars kept (one per refresh), as in Gonzo.
pub const COUNTS_HISTORY: usize = 50;

pub const INTERVALS: [Duration; 7] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];
const DEFAULT_INTERVAL: usize = 1;

/// Focusable parts of the log/insights area, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Logs,
    Words,
    Attributes,
    Patterns,
    Counts,
}

impl Section {
    const ORDER: [Section; 5] = [
        Section::Logs,
        Section::Words,
        Section::Attributes,
        Section::Patterns,
        Section::Counts,
    ];

    fn position(self) -> usize {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ORDER[(self.position() + 1) % Self::ORDER.len()]
    }

    pub fn prev(self) -> Self {
        Self::ORDER[(self.position() + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// Index among the four side-panel boxes (`None` for the log list).
    pub fn box_index(self) -> Option<usize> {
        self.position().checked_sub(1)
    }

    /// The section drawn in side-panel box `index`.
    pub fn from_box(index: usize) -> Option<Self> {
        Self::ORDER.get(index + 1).copied()
    }
}

/// Which popup is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Detail,
    AttributeValues,
    Patterns,
    Counts,
    Stats,
    SeverityFilter,
    Columns,
    Fullscreen,
}

#[derive(Debug, Clone)]
pub enum Modal {
    /// The raw line is copied in so the popup stays put as logs scroll by.
    Detail {
        raw: String,
    },
    AttributeValues {
        key: String,
    },
    Patterns,
    Counts,
    Stats,
    SeverityFilter(Picker),
    Columns(Picker),
    Fullscreen,
}

impl Modal {
    pub fn kind(&self) -> ModalKind {
        match self {
            Modal::Detail { .. } => ModalKind::Detail,
            Modal::AttributeValues { .. } => ModalKind::AttributeValues,
            Modal::Patterns => ModalKind::Patterns,
            Modal::Counts => ModalKind::Counts,
            Modal::Stats => ModalKind::Stats,
            Modal::SeverityFilter(_) => ModalKind::SeverityFilter,
            Modal::Columns(_) => ModalKind::Columns,
            Modal::Fullscreen => ModalKind::Fullscreen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Filter,
    Search,
}

#[derive(Debug, Clone)]
pub struct Input {
    pub kind: InputKind,
    pub text: String,
}

pub struct InsightsState {
    pub engine: Insights,
    /// What the panel draws; refreshed on the timer unless paused.
    pub snapshot: InsightsSnapshot,
    /// Lines per severity in each refresh interval, oldest first.
    pub counts_history: VecDeque<SeverityCounts>,
    pub paused: bool,
    /// Follow state to restore when pause ends.
    pub follow_before_pause: bool,
    last_refresh: Instant,
    interval: usize,
    pub section: Section,
    /// Highlighted row in each side-panel box.
    pub selected: [usize; 4],
    pub modal: Option<Modal>,
    pub modal_scroll: usize,
    pub input: Option<Input>,
    pub search: Option<String>,
    pub filter: LogFilter,
    /// The committed `/` text, shown in the log title.
    pub filter_text: String,
    /// Positions passing `filter`; `None` when nothing is filtered.
    pub filtered: Option<FilteredIds>,
    pub columns: Columns,
    /// Characters of each log row scrolled off to the left.
    pub h_scroll: usize,
    /// Highlighted log row (view position) for Enter / detail.
    pub log_cursor: Option<usize>,
    /// Box rectangles as last drawn, for mouse clicks.
    pub box_areas: [Rect; 4],
}

impl InsightsState {
    pub fn new(now: Instant) -> Self {
        Self {
            engine: Insights::new(now),
            snapshot: InsightsSnapshot::default(),
            counts_history: VecDeque::new(),
            paused: false,
            follow_before_pause: true,
            last_refresh: now,
            interval: DEFAULT_INTERVAL,
            section: Section::default(),
            selected: [0; 4],
            modal: None,
            modal_scroll: 0,
            input: None,
            search: None,
            filter: LogFilter::default(),
            filter_text: String::new(),
            filtered: None,
            columns: Columns::default(),
            h_scroll: 0,
            log_cursor: None,
            box_areas: [Rect::default(); 4],
        }
    }

    pub fn interval(&self) -> Duration {
        INTERVALS[self.interval]
    }

    /// `u`: next longer interval (stops at the longest).
    pub fn slower(&mut self) {
        self.interval = (self.interval + 1).min(INTERVALS.len() - 1);
    }

    /// `U`: next shorter interval (stops at the shortest).
    pub fn faster(&mut self) {
        self.interval = self.interval.saturating_sub(1);
    }

    /// Time until the next snapshot is due.
    pub fn time_until_refresh(&self, now: Instant) -> Duration {
        self.interval()
            .saturating_sub(now.saturating_duration_since(self.last_refresh))
    }

    /// Take a new snapshot if the interval has passed and not paused.
    /// Returns true when the panel changed.
    pub fn refresh_if_due(&mut self, now: Instant) -> bool {
        if self.paused || !self.time_until_refresh(now).is_zero() {
            return false;
        }
        let bar = self.engine.take_interval_counts();
        self.counts_history.push_back(bar);
        while self.counts_history.len() > COUNTS_HISTORY {
            self.counts_history.pop_front();
        }
        self.refresh_now(now);
        true
    }

    /// Count the lines `buf` just gained (`added`) and keep the filtered view
    /// current. Reads the buffer's entries so each line's level and timestamp
    /// are detected once, not again here.
    pub fn on_new_lines(&mut self, buf: &LogBuffer, added: usize, now: Instant) {
        let len = buf.len();
        for i in len - added.min(len)..len {
            if let Some(entry) = buf.entry(i) {
                self.engine.observe(entry, now);
            }
        }
        if let Some(ids) = self.filtered.as_mut() {
            ids.update(buf, &self.filter, added);
        }
    }

    /// A new stream (container switch or `R`): start the counts over. The
    /// filter, search and columns stay, as in Gonzo.
    pub fn on_stream_restart(&mut self, buf: &LogBuffer, now: Instant) {
        self.engine.reset(now);
        self.paused = false;
        self.counts_history.clear();
        self.refresh_now(now);
        self.log_cursor = None;
        self.selected = [0; 4];
        self.refilter(buf);
    }

    /// Rebuild the filtered positions after the filter changed.
    pub fn refilter(&mut self, buf: &LogBuffer) {
        self.filtered = self
            .filter
            .is_active()
            .then(|| FilteredIds::rebuild(buf, &self.filter));
    }

    /// Snapshot right away (after a reset or a container switch).
    pub fn refresh_now(&mut self, now: Instant) {
        self.snapshot = self.engine.snapshot(now);
        self.last_refresh = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::logs::LogLine;

    #[test]
    fn sections_cycle_both_ways() {
        assert_eq!(Section::Logs.next(), Section::Words);
        assert_eq!(Section::Counts.next(), Section::Logs);
        assert_eq!(Section::Logs.prev(), Section::Counts);
        assert_eq!(Section::Logs.box_index(), None);
        assert_eq!(Section::Patterns.box_index(), Some(2));
    }

    #[test]
    fn intervals_step_and_stop_at_the_ends() {
        let mut s = InsightsState::new(Instant::now());
        assert_eq!(s.interval(), Duration::from_secs(1));
        s.faster();
        s.faster();
        assert_eq!(s.interval(), Duration::from_millis(500));
        for _ in 0..20 {
            s.slower();
        }
        assert_eq!(s.interval(), Duration::from_secs(60));
    }

    #[test]
    fn refresh_waits_for_the_interval_and_respects_pause() {
        let t0 = Instant::now();
        let mut s = InsightsState::new(t0);
        s.engine
            .observe(&LogLine::from_raw("ERROR boom".into()), t0);
        assert!(!s.refresh_if_due(t0 + Duration::from_millis(200)));
        assert_eq!(s.snapshot.total_lines, 0);
        assert!(s.refresh_if_due(t0 + Duration::from_secs(1)));
        assert_eq!(s.snapshot.total_lines, 1);

        s.paused = true;
        s.engine
            .observe(&LogLine::from_raw("ERROR again".into()), t0);
        assert!(!s.refresh_if_due(t0 + Duration::from_secs(5)));
        assert_eq!(s.snapshot.total_lines, 1, "paused view stays frozen");
        s.paused = false;
        assert!(s.refresh_if_due(t0 + Duration::from_secs(5)));
        assert_eq!(s.snapshot.total_lines, 2, "data kept coming while paused");
    }

    #[test]
    fn chunks_feed_counts_and_the_filtered_view() {
        let t0 = Instant::now();
        let mut s = InsightsState::new(t0);
        let mut buf = LogBuffer::new(100);
        s.filter.hidden[crate::insights::severity::Severity::Info.index()] = true;
        s.refilter(&buf);
        let added = buf.extend_chunk("INFO a\nERROR b\n");
        s.on_new_lines(&buf, added, t0);
        s.refresh_now(t0);
        assert_eq!(s.snapshot.total_lines, 2);
        let view = crate::logview::LogView::new(&buf, s.filtered.as_ref());
        assert_eq!(view.collect_all(), "ERROR b\n");

        s.paused = true;
        s.on_stream_restart(&LogBuffer::new(100), t0);
        assert_eq!(s.snapshot.total_lines, 0);
        assert!(!s.paused, "a new container starts live");
        assert!(
            s.filter.hidden[crate::insights::severity::Severity::Info.index()],
            "filter kept"
        );
    }

    #[test]
    fn each_refresh_adds_one_counts_bar() {
        let t0 = Instant::now();
        let mut s = InsightsState::new(t0);
        s.engine.observe(&LogLine::from_raw("ERROR a".into()), t0);
        s.engine.observe(&LogLine::from_raw("INFO b".into()), t0);
        assert!(s.refresh_if_due(t0 + Duration::from_secs(1)));
        assert!(s.refresh_if_due(t0 + Duration::from_secs(2)));
        assert_eq!(s.counts_history.len(), 2);
        let first = s.counts_history[0];
        assert_eq!(first[crate::insights::severity::Severity::Error.index()], 1);
        assert_eq!(s.counts_history[1], [0; 6]);

        for n in 3..100 {
            s.refresh_if_due(t0 + Duration::from_secs(n));
        }
        assert_eq!(s.counts_history.len(), COUNTS_HISTORY);

        s.on_stream_restart(&LogBuffer::new(10), t0);
        assert!(s.counts_history.is_empty());
    }

    #[test]
    fn time_until_refresh_counts_down() {
        let t0 = Instant::now();
        let s = InsightsState::new(t0);
        assert_eq!(
            s.time_until_refresh(t0 + Duration::from_millis(400)),
            Duration::from_millis(600)
        );
        assert_eq!(
            s.time_until_refresh(t0 + Duration::from_secs(3)),
            Duration::ZERO
        );
    }
}
