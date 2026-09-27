//! What the log pane shows: the whole buffer, or only the lines that pass the
//! severity filter and the `/` regex. Scrolling, selection and copying all
//! work in positions of this view.

use regex::Regex;

use crate::insights::severity::Severity;
use crate::ui::logs::{LogBuffer, LogLine};

#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Severities switched off in the severity filter popup.
    pub hidden: [bool; 6],
    pub regex: Option<Regex>,
}

impl LogFilter {
    pub fn is_active(&self) -> bool {
        self.regex.is_some() || self.hidden.iter().any(|h| *h)
    }

    pub fn matches(&self, line: &LogLine) -> bool {
        let severity = Severity::from_level(line.level);
        if self.hidden[severity.index()] {
            return false;
        }
        self.regex.as_ref().is_none_or(|re| re.is_match(&line.raw))
    }
}

/// Absolute ids (see `LogBuffer::first_id`) of lines passing a filter.
#[derive(Debug, Clone, Default)]
pub struct FilteredIds {
    ids: Vec<u64>,
}

impl FilteredIds {
    pub fn rebuild(buf: &LogBuffer, filter: &LogFilter) -> Self {
        let first = buf.first_id();
        let ids = (0..buf.len())
            .filter(|&i| buf.entry(i).is_some_and(|l| filter.matches(l)))
            .map(|i| first + i as u64)
            .collect();
        Self { ids }
    }

    /// Account for `added` new lines at the end of `buf` and for any old lines
    /// the buffer dropped.
    pub fn update(&mut self, buf: &LogBuffer, filter: &LogFilter, added: usize) {
        let first = buf.first_id();
        let gone = self.ids.partition_point(|id| *id < first);
        self.ids.drain(..gone);
        let start = buf.len().saturating_sub(added);
        for i in start..buf.len() {
            if buf.entry(i).is_some_and(|l| filter.matches(l)) {
                self.ids.push(first + i as u64);
            }
        }
    }
}

pub enum LogView<'a> {
    All(&'a LogBuffer),
    Filtered { buf: &'a LogBuffer, ids: &'a [u64] },
}

impl<'a> LogView<'a> {
    pub fn new(buf: &'a LogBuffer, filtered: Option<&'a FilteredIds>) -> Self {
        match filtered {
            Some(f) => LogView::Filtered { buf, ids: &f.ids },
            None => LogView::All(buf),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            LogView::All(buf) => buf.len(),
            LogView::Filtered { ids, .. } => ids.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn entry(&self, pos: usize) -> Option<&'a LogLine> {
        match self {
            LogView::All(buf) => buf.entry(pos),
            LogView::Filtered { buf, ids } => ids.get(pos).and_then(|id| buf.entry_by_id(*id)),
        }
    }

    /// Raw text of positions `start..=end`, one line each.
    pub fn collect_range(&self, start: usize, end: usize) -> String {
        if let LogView::All(buf) = self {
            return buf.collect_range(start, end);
        }
        let end = end.min(self.len().saturating_sub(1));
        if self.is_empty() || start > end {
            return String::new();
        }
        (start..=end)
            .filter_map(|pos| self.entry(pos))
            .map(|l| format!("{}\n", l.raw))
            .collect()
    }

    pub fn collect_all(&self) -> String {
        match self {
            LogView::All(buf) => buf.collect_all(),
            LogView::Filtered { .. } => self.collect_range(0, self.len().saturating_sub(1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(cap: usize, lines: &[&str]) -> LogBuffer {
        let mut b = LogBuffer::new(cap);
        for l in lines {
            b.push(l.to_string());
        }
        b
    }

    fn errors_only() -> LogFilter {
        let mut f = LogFilter::default();
        for s in Severity::ALL {
            f.hidden[s.index()] = s != Severity::Error;
        }
        f
    }

    #[test]
    fn inactive_filter_shows_everything() {
        let b = buffer(10, &["INFO a", "ERROR b"]);
        let f = LogFilter::default();
        assert!(!f.is_active());
        let v = LogView::new(&b, None);
        assert_eq!(v.len(), 2);
        assert_eq!(v.entry(1).map(|l| l.raw.as_str()), Some("ERROR b"));
    }

    #[test]
    fn severity_filter_hides_levels() {
        let b = buffer(10, &["INFO a", "ERROR b", "plain c", "ERROR d"]);
        let ids = FilteredIds::rebuild(&b, &errors_only());
        let v = LogView::new(&b, Some(&ids));
        assert_eq!(v.len(), 2);
        assert_eq!(v.collect_all(), "ERROR b\nERROR d\n");
    }

    #[test]
    fn lines_without_a_level_count_as_info() {
        let mut f = LogFilter::default();
        f.hidden[Severity::Info.index()] = true;
        assert!(!f.matches(&buffer(1, &["plain text"]).entry(0).unwrap().clone()));
    }

    #[test]
    fn regex_filter() {
        let b = buffer(10, &["GET /a 200", "POST /b 500", "GET /c 500"]);
        let f = LogFilter {
            regex: Some(Regex::new(r"GET .* 500").unwrap()),
            ..LogFilter::default()
        };
        let ids = FilteredIds::rebuild(&b, &f);
        assert_eq!(LogView::new(&b, Some(&ids)).collect_all(), "GET /c 500\n");
    }

    #[test]
    fn appended_lines_join_the_view() {
        let mut b = buffer(10, &["ERROR a"]);
        let f = errors_only();
        let mut ids = FilteredIds::rebuild(&b, &f);
        let added = b.extend_chunk("INFO x\nERROR y\n");
        ids.update(&b, &f, added);
        assert_eq!(
            LogView::new(&b, Some(&ids)).collect_all(),
            "ERROR a\nERROR y\n"
        );
    }

    #[test]
    fn dropped_lines_leave_the_view_when_the_buffer_is_full() {
        let mut b = buffer(3, &["ERROR 1", "INFO 2", "ERROR 3"]);
        let f = errors_only();
        let mut ids = FilteredIds::rebuild(&b, &f);
        // Buffer holds 3: pushing 2 more drops "ERROR 1" and "INFO 2".
        let added = b.extend_chunk("ERROR 4\nINFO 5\n");
        ids.update(&b, &f, added);
        let v = LogView::new(&b, Some(&ids));
        assert_eq!(v.collect_all(), "ERROR 3\nERROR 4\n");
        assert_eq!(v.entry(0).map(|l| l.raw.as_str()), Some("ERROR 3"));
        assert!(v.entry(2).is_none());
    }

    #[test]
    fn collect_range_clamps_and_handles_empty() {
        let b = buffer(10, &["a", "b", "c"]);
        let v = LogView::new(&b, None);
        assert_eq!(v.collect_range(1, 99), "b\nc\n");
        assert_eq!(v.collect_range(2, 1), "");
        let empty = buffer(10, &[]);
        assert_eq!(LogView::new(&empty, None).collect_all(), "");
    }
}
