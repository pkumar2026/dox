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

    /// Append a chunk's lines; returns how many lines were added.
    pub fn extend_chunk(&mut self, chunk: &str) -> usize {
        let mut added = 0;
        for line in chunk.split_inclusive('\n') {
            let trimmed = line.trim_end_matches('\n').to_string();
            self.push(trimmed);
            added += 1;
        }
        added
    }

    /// Absolute id of the oldest line still held (ids never repeat, even
    /// after old lines are dropped).
    pub fn first_id(&self) -> u64 {
        self.dropped
    }

    pub fn entry_by_id(&self, id: u64) -> Option<&LogLine> {
        let idx = id.checked_sub(self.dropped)?;
        self.lines.get(usize::try_from(idx).ok()?)
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

/// Bytes at the start of each line searched for a level word.
const LEVEL_SCAN_BYTES: usize = 120;

/// Level words, grouped by level in the order `locate_level_word` prefers.
const LEVEL_WORDS: [(&str, LogLevel); 12] = [
    ("FATAL", LogLevel::Fatal),
    ("PANIC", LogLevel::Fatal),
    ("CRITICAL", LogLevel::Fatal),
    ("ERROR", LogLevel::Error),
    ("ERR", LogLevel::Error),
    ("WARNING", LogLevel::Warn),
    ("WARN", LogLevel::Warn),
    ("INFO", LogLevel::Info),
    ("NOTICE", LogLevel::Info),
    ("DEBUG", LogLevel::Debug),
    ("DBG", LogLevel::Debug),
    ("TRACE", LogLevel::Trace),
];

/// Heuristic level detection. Scans the first ~120 bytes for whole-word matches
/// against common level tokens; the most severe level found wins. Handles
/// `[ERROR]`, ` ERR `, `level=warn`, etc. — but it's a heuristic, not a parser.
///
/// Works on bytes in one pass, so a multi-byte character at the edge of the
/// window can't split a string slice.
pub fn detect_level(line: &str) -> LogLevel {
    let bytes = line.as_bytes();
    level_words(line)
        .filter_map(|r| word_level(&bytes[r]))
        .min_by_key(|level| *level as u8)
        .unwrap_or(LogLevel::Other)
}

/// Byte ranges of the words (`[A-Za-z0-9_]+`) in the scanned window. A word
/// cut by the window edge ends there.
fn level_words(line: &str) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    let bytes = &line.as_bytes()[..line.len().min(LEVEL_SCAN_BYTES)];
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() && !is_word_char(bytes[i]) {
            i += 1;
        }
        if i == bytes.len() {
            return None;
        }
        let start = i;
        while i < bytes.len() && is_word_char(bytes[i]) {
            i += 1;
        }
        Some(start..i)
    })
}

/// The level one word names, ignoring ASCII case.
fn word_level(word: &[u8]) -> Option<LogLevel> {
    LEVEL_WORDS
        .iter()
        .find(|(w, _)| w.as_bytes().eq_ignore_ascii_case(word))
        .map(|(_, level)| *level)
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

pub(super) fn message_style(level: LogLevel) -> Style {
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

/// Split `haystack` around the word that gave it `level`, for highlighting.
/// Split points are word edges (ASCII bytes), so they are always valid.
fn locate_level_word(haystack: &str, level: LogLevel) -> Option<(&str, &str, &str)> {
    let bytes = haystack.as_bytes();
    let range = LEVEL_WORDS
        .iter()
        .filter(|(_, l)| *l == level)
        .find_map(|(word, _)| {
            level_words(haystack).find(|r| bytes[r.clone()].eq_ignore_ascii_case(word.as_bytes()))
        })?;
    Some((
        &haystack[..range.start],
        &haystack[range.clone()],
        &haystack[range.end..],
    ))
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

/// Scrolled-away marker. While the logs are focused `f` opens the full-screen
/// viewer, so End (not `f`) returns to live.
pub(super) fn paused_marker(app: &App) -> &'static str {
    if matches!(app.focus, FocusArea::Detail) && matches!(app.mode, crate::events::Mode::Normal) {
        " [PAUSED — End to resume]"
    } else {
        " [PAUSED — f to resume]"
    }
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let height = area.height as usize;
    app.last_log_height = height.max(1);
    app.last_log_inner = area;
    let view = crate::logview::LogView::new(&app.logs, app.insights.filtered.as_ref());
    if view.is_empty() && !app.logs.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "(no lines match the filter — Ctrl+f levels, / regex, Esc while typing clears)",
                Style::default().dim(),
            ))),
            area,
        );
        return;
    }
    if view.is_empty() {
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

    let total = view.len();
    let (scroll, end) = visible_range(total, height, app.logs_scroll, app.logs_follow);
    if app.logs_follow {
        app.logs_scroll = scroll;
    }
    let sel = if matches!(app.focus, FocusArea::Detail) && app.visual_select.is_some() {
        app.visual_select
    } else {
        None
    };

    let insights = &app.insights;
    let cursor = insights.log_cursor.filter(|_| {
        matches!(app.focus, FocusArea::Detail)
            && (insights.section == crate::insights_ui::Section::Logs
                || matches!(insights.modal, Some(crate::insights_ui::Modal::Fullscreen)))
    });
    let opts = super::log_rows::RowOptions {
        columns: &insights.columns,
        search: insights.search.as_deref(),
        h_scroll: insights.h_scroll,
    };
    let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
    for idx in scroll..end {
        let entry = match view.entry(idx) {
            Some(e) => e,
            None => continue,
        };
        let selected = sel
            .map(|s| {
                let (lo, hi) = s.range();
                idx >= lo && idx <= hi
            })
            .unwrap_or(false);
        let line = if selected || opts.is_plain() {
            style_entry(entry, selected)
        } else {
            super::log_rows::render_row(entry, &opts)
        };
        let line = if cursor == Some(idx) {
            line.patch_style(Style::default().add_modifier(Modifier::REVERSED))
        } else {
            line
        };
        lines.push(line);
    }

    let follow_marker = if app.logs_follow {
        ""
    } else {
        paused_marker(app)
    };
    let count = if total == app.logs.len() {
        format!("{} lines", total)
    } else {
        format!("{} of {} lines", total, app.logs.len())
    };
    let header = Line::from(vec![
        Span::styled(count, Style::default().fg(Color::DarkGray)),
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

    /// The pre-0.3.1 detector, kept to prove the rewrite gives the same
    /// answers. Only used on ASCII input: it sliced `&str` at byte 120, which
    /// panicked when a multi-byte character straddled that byte.
    fn detect_level_reference(line: &str) -> LogLevel {
        let scan_len = line.len().min(120);
        let scan: String = line[..scan_len].to_ascii_uppercase();
        let has = |w: &str| reference_find(&scan, w).is_some();
        if has("FATAL") || has("PANIC") || has("CRITICAL") {
            return LogLevel::Fatal;
        }
        if has("ERROR") || has("ERR") {
            return LogLevel::Error;
        }
        if has("WARNING") || has("WARN") {
            return LogLevel::Warn;
        }
        if has("INFO") || has("NOTICE") {
            return LogLevel::Info;
        }
        if has("DEBUG") || has("DBG") {
            return LogLevel::Debug;
        }
        if has("TRACE") {
            return LogLevel::Trace;
        }
        LogLevel::Other
    }

    fn reference_find(upper: &str, word: &str) -> Option<usize> {
        let (h, w) = (upper.as_bytes(), word.as_bytes());
        (0..=h.len().saturating_sub(w.len())).find(|&i| {
            h.len() >= w.len()
                && &h[i..i + w.len()] == w
                && (i == 0 || !is_word_char(h[i - 1]))
                && (i + w.len() == h.len() || !is_word_char(h[i + w.len()]))
        })
    }

    fn locate_reference(haystack: &str, level: LogLevel) -> Option<(usize, usize)> {
        let tokens: &[&str] = match level {
            LogLevel::Fatal => &["FATAL", "PANIC", "CRITICAL"],
            LogLevel::Error => &["ERROR", "ERR"],
            LogLevel::Warn => &["WARNING", "WARN"],
            LogLevel::Info => &["INFO", "NOTICE"],
            LogLevel::Debug => &["DEBUG", "DBG"],
            LogLevel::Trace => &["TRACE"],
            LogLevel::Other => return None,
        };
        let upper = haystack[..haystack.len().min(120)].to_ascii_uppercase();
        tokens
            .iter()
            .find_map(|t| reference_find(&upper, t).map(|i| (i, i + t.len())))
    }

    /// Deterministic pseudo-random lines with level words near the 120-byte edge.
    fn generated_lines(n: usize) -> Vec<String> {
        const WORDS: &str = "ERROR error ERR Err WARN warning INFO info NOTICE DEBUG dbg TRACE \
            FATAL panic CRITICAL errors INFO_x xINFO [ERROR] level=warn req 42 user_id a bb \
            INFORMATION WARNINGS";
        let words: Vec<&str> = WORDS.split_whitespace().collect();
        const SEPS: &[&str] = &[" ", "  ", "=", ":", "[", "]", "_", "-", ",", "|"];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |m: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % m
        };
        (0..n)
            .map(|_| {
                let target = 100 + next(40);
                let mut line = String::new();
                while line.len() < target {
                    line.push_str(words[next(words.len())]);
                    line.push_str(SEPS[next(SEPS.len())]);
                }
                line
            })
            .collect()
    }

    #[test]
    fn rewrite_matches_the_old_detector() {
        for line in generated_lines(20_000) {
            let level = detect_level(&line);
            assert_eq!(level, detect_level_reference(&line), "detect: {line:?}");
            let found =
                locate_level_word(&line, level).map(|(a, b, _)| (a.len(), a.len() + b.len()));
            assert_eq!(found, locate_reference(&line, level), "locate: {line:?}");
        }
    }

    #[test]
    fn multibyte_characters_at_any_offset_never_panic() {
        let columns = crate::insights_ui::picker::Columns {
            level: true,
            service: true,
            fields: vec!["k".into()],
            ..Default::default()
        };
        let opts = crate::ui::log_rows::RowOptions {
            columns: &columns,
            search: Some("error"),
            h_scroll: 3,
        };
        for ch in ["é", "€", "😀"] {
            for offset in 80..=160 {
                for prefix in ["", "2026-09-26T10:00:00.000000000Z "] {
                    for body in [
                        format!("{}{ch} ERROR tail k=v", "a".repeat(offset)),
                        format!("INFO {}{ch} tail", "a".repeat(offset)),
                        format!(
                            "{{\"level\":\"warn\",\"msg\":\"{}{ch}\"}}",
                            "a".repeat(offset)
                        ),
                    ] {
                        let line = format!("{prefix}{body}");
                        let mut b = LogBuffer::new(1);
                        b.push(line.clone());
                        let entry = b.entry(0).expect("pushed");
                        let _ = style_entry(entry, false);
                        let _ = crate::ui::log_rows::render_row(entry, &opts);
                        let _ = crate::insights::parse::parse_line(&line);
                        let _ = crate::insights_ui::detail::detail(&line);
                    }
                }
            }
        }
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
