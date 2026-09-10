use std::collections::VecDeque;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, FocusArea};

/// A single log line plus its pre-computed level + timestamp boundary so
/// rendering is allocation-light and detection runs ONCE per line, not per frame.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub raw: String,
    pub level: LogLevel,
    pub ts_len: usize,
}

#[derive(Debug, Clone)]
pub struct LogBuffer {
    lines: VecDeque<LogLine>,
    cap: usize,
    /// Total lines dropped from the front (for indicators).
    pub dropped: u64,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(cap.min(1024)),
            cap,
            dropped: 0,
        }
    }

    pub fn push(&mut self, raw: String) {
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
            self.dropped += 1;
        }
        let level = detect_level(&raw);
        let ts_len = timestamp_prefix_len(&raw);
        self.lines.push_back(LogLine {
            raw,
            level,
            ts_len,
        });
    }

    pub fn extend_chunk(&mut self, chunk: &str) {
        for line in chunk.split_inclusive('\n') {
            let trimmed = line.trim_end_matches('\n').to_string();
            self.push(trimmed);
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.dropped = 0;
    }

    #[cfg(test)]
    pub fn line(&self, idx: usize) -> Option<&str> {
        self.lines.get(idx).map(|l| l.raw.as_str())
    }

    pub fn entry(&self, idx: usize) -> Option<&LogLine> {
        self.lines.get(idx)
    }

    pub fn collect_range(&self, start: usize, end_inclusive: usize) -> String {
        let end = end_inclusive.min(self.lines.len().saturating_sub(1));
        if self.lines.is_empty() || start > end {
            return String::new();
        }
        let mut out = String::new();
        for i in start..=end {
            if let Some(l) = self.lines.get(i) {
                out.push_str(&l.raw);
                out.push('\n');
            }
        }
        out
    }

    pub fn collect_all(&self) -> String {
        let mut out =
            String::with_capacity(self.lines.iter().map(|l| l.raw.len() + 1).sum());
        for l in &self.lines {
            out.push_str(&l.raw);
            out.push('\n');
        }
        out
    }
}

/// Visual-mode selection state. Inclusive range over buffer indices.
#[derive(Debug, Clone, Copy, Default)]
pub struct Selection {
    pub anchor: usize,
    pub cursor: usize,
}

impl Selection {
    pub fn range(&self) -> (usize, usize) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Fatal,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Other,
}

/// Heuristic level detection. Scans the first ~120 chars for whole-word matches
/// against common level tokens, in priority order. Handles `[ERROR]`, ` ERR `,
/// `level=warn`, `klog`-style letters, etc. — but it's a heuristic, not a parser.
pub fn detect_level(line: &str) -> LogLevel {
    let scan_len = line.len().min(120);
    let scan: String = line[..scan_len].to_ascii_uppercase();
    if has_token(&scan, "FATAL") || has_token(&scan, "PANIC") || has_token(&scan, "CRITICAL") {
        return LogLevel::Fatal;
    }
    if has_token(&scan, "ERROR") || has_token(&scan, "ERR") {
        return LogLevel::Error;
    }
    if has_token(&scan, "WARNING") || has_token(&scan, "WARN") {
        return LogLevel::Warn;
    }
    if has_token(&scan, "INFO") || has_token(&scan, "NOTICE") {
        return LogLevel::Info;
    }
    if has_token(&scan, "DEBUG") || has_token(&scan, "DBG") {
        return LogLevel::Debug;
    }
    if has_token(&scan, "TRACE") {
        return LogLevel::Trace;
    }
    LogLevel::Other
}

fn has_token(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let wb = word.as_bytes();
    let n = bytes.len();
    let w = wb.len();
    if w == 0 || w > n {
        return false;
    }
    let mut i = 0;
    while i + w <= n {
        if &bytes[i..i + w] == wb {
            let pre_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let post_ok = i + w == n || !is_word_char(bytes[i + w]);
            if pre_ok && post_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Return the length of an ISO-8601-ish timestamp prefix, or 0 if none.
/// Matches `YYYY-MM-DD[T ]HH:MM:SS...` up to the first whitespace AFTER the time.
pub fn timestamp_prefix_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    if bytes.len() < 11 {
        return 0;
    }
    let date_shape = bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4] == b'-'
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && bytes[7] == b'-'
        && bytes[8].is_ascii_digit()
        && bytes[9].is_ascii_digit();
    if !date_shape {
        return 0;
    }
    let sep = bytes[10];
    if sep != b'T' && sep != b' ' {
        return 0;
    }
    // Skip past the date+separator and find the next whitespace, which ends
    // the time portion (after HH:MM:SS[.fff][Z|+00:00]).
    for (i, c) in s.char_indices().skip(11) {
        if c.is_whitespace() {
            return i;
        }
    }
    0
}

fn level_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Fatal => Color::Magenta,
        LogLevel::Error => Color::Red,
        LogLevel::Warn => Color::Yellow,
        LogLevel::Info => Color::Cyan,
        LogLevel::Debug => Color::DarkGray,
        LogLevel::Trace => Color::DarkGray,
        LogLevel::Other => Color::Reset,
    }
}

fn message_style(level: LogLevel) -> Style {
    let mut s = Style::default();
    match level {
        LogLevel::Fatal => {
            s = s
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        }
        LogLevel::Error => s = s.fg(Color::Red),
        LogLevel::Warn => s = s.fg(Color::Yellow),
        LogLevel::Info => s = s.fg(Color::Cyan),
        LogLevel::Debug => s = s.fg(Color::DarkGray),
        LogLevel::Trace => s = s.fg(Color::DarkGray).add_modifier(Modifier::DIM),
        LogLevel::Other => {}
    }
    s
}

fn style_entry(entry: &LogLine, selected: bool) -> Line<'static> {
    let raw = entry.raw.as_str();
    if selected {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let level = entry.level;
    let ts_len = entry.ts_len;
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(3);

    if ts_len > 0 {
        spans.push(Span::styled(
            raw[..ts_len].to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let rest = &raw[ts_len..];
    if rest.is_empty() {
        return Line::from(spans);
    }

    if let Some((before, word, after)) = locate_level_word(rest, level) {
        if !before.is_empty() {
            spans.push(Span::styled(before.to_string(), message_style(level)));
        }
        spans.push(Span::styled(
            word.to_string(),
            Style::default()
                .fg(level_color(level))
                .add_modifier(Modifier::BOLD),
        ));
        if !after.is_empty() {
            spans.push(Span::styled(after.to_string(), message_style(level)));
        }
    } else {
        spans.push(Span::styled(rest.to_string(), message_style(level)));
    }
    Line::from(spans)
}

fn locate_level_word(haystack: &str, level: LogLevel) -> Option<(&str, &str, &str)> {
    let tokens: &[&str] = match level {
        LogLevel::Fatal => &["FATAL", "PANIC", "CRITICAL"],
        LogLevel::Error => &["ERROR", "ERR"],
        LogLevel::Warn => &["WARNING", "WARN"],
        LogLevel::Info => &["INFO", "NOTICE"],
        LogLevel::Debug => &["DEBUG", "DBG"],
        LogLevel::Trace => &["TRACE"],
        LogLevel::Other => return None,
    };
    let scan_len = haystack.len().min(120);
    let upper: String = haystack[..scan_len].to_ascii_uppercase();
    let upper_bytes = upper.as_bytes();
    for &tok in tokens {
        let tb = tok.as_bytes();
        if tb.len() > upper_bytes.len() {
            continue;
        }
        let mut i = 0;
        while i + tb.len() <= upper_bytes.len() {
            if &upper_bytes[i..i + tb.len()] == tb {
                let pre_ok = i == 0 || !is_word_char(upper_bytes[i - 1]);
                let post_ok =
                    i + tb.len() == upper_bytes.len() || !is_word_char(upper_bytes[i + tb.len()]);
                if pre_ok && post_ok {
                    let (a, b) = haystack.split_at(i);
                    let (b, c) = b.split_at(tb.len());
                    return Some((a, b, c));
                }
            }
            i += 1;
        }
    }
    None
}

/// Compute the half-open `[start, end)` line range to render.
///
/// When `follow` is true, the window sticks to the end of the buffer.
/// Otherwise we honour `scroll` clamped to the valid range.
pub fn visible_range(total: usize, height: usize, scroll: usize, follow: bool) -> (usize, usize) {
    if total == 0 || height == 0 {
        return (0, 0);
    }
    let start = if follow {
        total.saturating_sub(height)
    } else {
        scroll.min(total.saturating_sub(1))
    };
    let end = (start + height).min(total);
    (start, end)
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let height = area.height as usize;
    app.last_log_height = height.max(1);
    app.last_log_inner = area;
    let buf = &app.logs;
    if buf.is_empty() {
        let hint = match &app.log_stream {
            crate::app::LogStreamState::Idle => {
                if app.selected_container_id().is_none() {
                    "(no container selected — Enter on a row to view its logs)".to_string()
                } else {
                    "(stream not started — press R to restart)".to_string()
                }
            }
            crate::app::LogStreamState::Starting { container_name } => format!(
                "(starting log stream for {} … if this stays put, the container may have no recent output — try `docker logs {}`)",
                container_name, container_name
            ),
            crate::app::LogStreamState::Streaming {
                container_name,
                chunks,
            } => format!(
                "(streaming from {} · {} chunk(s) received, all empty so far)",
                container_name, chunks
            ),
            crate::app::LogStreamState::Errored {
                container_name,
                reason,
            } => format!(
                "(stream from {} errored: {} — press R to retry)",
                container_name, reason
            ),
            crate::app::LogStreamState::Ended { container_name } => format!(
                "(stream for {} ended — container may have stopped. Press R to restart)",
                container_name
            ),
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, Style::default().dim()))),
            area,
        );
        return;
    }

    let total = buf.len();
    let (scroll, end) = visible_range(total, height, app.logs_scroll, app.logs_follow);
    if app.logs_follow {
        app.logs_scroll = scroll;
    }
    let sel = if matches!(app.focus, FocusArea::Detail) && app.visual_select.is_some() {
        app.visual_select
    } else {
        None
    };

    let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
    for idx in scroll..end {
        let entry = match buf.entry(idx) {
            Some(e) => e,
            None => continue,
        };
        let selected = sel
            .map(|s| {
                let (lo, hi) = s.range();
                idx >= lo && idx <= hi
            })
            .unwrap_or(false);
        lines.push(style_entry(entry, selected));
    }

    let follow_marker = if app.logs_follow {
        ""
    } else {
        " [PAUSED — f to resume]"
    };
    let header = Line::from(vec![
        Span::styled(
            format!("{} lines", total),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(follow_marker, Style::default().fg(Color::Yellow)),
    ]);
    if scroll == 0 {
        lines.insert(0, header);
    }

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_caps_buffer_and_counts_drops() {
        let mut b = LogBuffer::new(3);
        b.push("a".into());
        b.push("b".into());
        b.push("c".into());
        b.push("d".into());
        b.push("e".into());
        assert_eq!(b.len(), 3);
        assert_eq!(b.dropped, 2);
        assert_eq!(b.line(0), Some("c"));
        assert_eq!(b.line(2), Some("e"));
    }

    #[test]
    fn extend_chunk_splits_lines() {
        let mut b = LogBuffer::new(10);
        b.extend_chunk("hello\nworld\npartial");
        assert_eq!(b.len(), 3);
        assert_eq!(b.line(0), Some("hello"));
        assert_eq!(b.line(1), Some("world"));
        assert_eq!(b.line(2), Some("partial"));
    }

    #[test]
    fn collect_range_returns_inclusive_range() {
        let mut b = LogBuffer::new(10);
        for s in ["a", "b", "c", "d", "e"] {
            b.push(s.into());
        }
        assert_eq!(b.collect_range(1, 3), "b\nc\nd\n");
    }

    #[test]
    fn collect_range_clamps_end() {
        let mut b = LogBuffer::new(10);
        for s in ["a", "b", "c"] {
            b.push(s.into());
        }
        assert_eq!(b.collect_range(0, 100), "a\nb\nc\n");
    }

    #[test]
    fn collect_all_yields_full_buffer() {
        let mut b = LogBuffer::new(10);
        b.push("x".into());
        b.push("y".into());
        assert_eq!(b.collect_all(), "x\ny\n");
    }

    #[test]
    fn selection_range_normalises_anchor_after_cursor() {
        let s = Selection {
            anchor: 7,
            cursor: 3,
        };
        assert_eq!(s.range(), (3, 7));
    }

    #[test]
    fn selection_range_normalises_anchor_before_cursor() {
        let s = Selection {
            anchor: 1,
            cursor: 4,
        };
        assert_eq!(s.range(), (1, 4));
    }

    #[test]
    fn visible_range_follow_sticks_to_bottom() {
        assert_eq!(visible_range(100, 30, 0, true), (70, 100));
        assert_eq!(visible_range(100, 30, 999, true), (70, 100));
        assert_eq!(visible_range(10, 30, 0, true), (0, 10));
    }

    #[test]
    fn visible_range_manual_scroll_respected() {
        assert_eq!(visible_range(100, 30, 50, false), (50, 80));
        assert_eq!(visible_range(100, 30, 0, false), (0, 30));
        // Scroll past end clamps to total-1.
        assert_eq!(visible_range(100, 30, 999, false), (99, 100));
    }

    #[test]
    fn visible_range_zero_buffer_or_height() {
        assert_eq!(visible_range(0, 30, 0, true), (0, 0));
        assert_eq!(visible_range(100, 0, 50, false), (0, 0));
    }

    #[test]
    fn detects_common_levels() {
        assert_eq!(detect_level("2024-01-01 ERROR something"), LogLevel::Error);
        assert_eq!(detect_level("INFO  starting"), LogLevel::Info);
        assert_eq!(detect_level("[WARN] disk full"), LogLevel::Warn);
        assert_eq!(detect_level("DEBUG x=1"), LogLevel::Debug);
        assert_eq!(detect_level("FATAL crash"), LogLevel::Fatal);
        assert_eq!(detect_level("trace x"), LogLevel::Trace);
        assert_eq!(detect_level("plain message"), LogLevel::Other);
    }

    #[test]
    fn does_not_match_inside_other_words() {
        // "INFORMATION" should NOT match INFO.
        assert_eq!(detect_level("INFORMATION about state"), LogLevel::Other);
        // "ERRORS" should NOT match ERROR (word boundary).
        assert_eq!(detect_level("the ERRORS in the file"), LogLevel::Other);
    }

    #[test]
    fn timestamp_prefix_iso_8601() {
        assert_eq!(
            timestamp_prefix_len("2026-06-06T19:02:11Z listening"),
            20
        );
        assert_eq!(
            timestamp_prefix_len("2026-06-06 19:02:11.123 hello"),
            23
        );
        assert_eq!(timestamp_prefix_len("INFO no ts"), 0);
        assert_eq!(timestamp_prefix_len("short"), 0);
    }

    #[test]
    fn visible_range_paged_up_from_follow() {
        // Simulate the path: follow=true → switch to manual at logs_scroll=70 → PageUp 20.
        let (start_after_seed, _) = visible_range(100, 30, 0, true);
        assert_eq!(start_after_seed, 70);
        // After PageUp, scroll=50 (70-20), follow=false.
        let (start, end) = visible_range(100, 30, 50, false);
        assert_eq!((start, end), (50, 80));
    }
}
