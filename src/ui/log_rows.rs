//! Log rows when Gonzo-style options are on: extra columns (level, service,
//! host, chosen fields), search highlighting and horizontal scroll. With the
//! defaults, rows keep dox's plain rendering (`logs::style_entry`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::insights_panel::{fit, severity_color};
use super::logs::{message_style, LogLine};
use crate::insights::parse::parse_line;
use crate::insights::severity::Severity;
use crate::insights_ui::picker::Columns;

const SERVICE_WIDTH: usize = 16;
const FIELD_WIDTH: usize = 14;

pub struct RowOptions<'a> {
    pub columns: &'a Columns,
    pub search: Option<&'a str>,
    pub h_scroll: usize,
}

impl RowOptions<'_> {
    /// True when dox's plain rendering can be used unchanged.
    pub fn is_plain(&self) -> bool {
        self.columns.is_default() && self.search.is_none() && self.h_scroll == 0
    }
}

pub fn render_row(entry: &LogLine, opts: &RowOptions) -> Line<'static> {
    let raw = entry.raw.as_str();
    let ts_len = entry.ts_len.min(raw.len());
    let cols = opts.columns;
    let mut spans: Vec<Span<'static>> = Vec::new();
    if cols.time && ts_len > 0 {
        spans.push(Span::styled(
            format!("{} ", short_time(&raw[..ts_len])),
            Style::default().dim(),
        ));
    }
    let severity = Severity::from_level(entry.level);
    if cols.level {
        spans.push(Span::styled(
            format!("{:<5} ", severity.label()),
            Style::default()
                .fg(severity_color(severity))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if cols.service || cols.host || !cols.fields.is_empty() {
        let parsed = parse_line(raw);
        let mut push_col = |value: Option<&str>, width: usize| {
            let text = fit(value.unwrap_or("-"), width);
            spans.push(Span::styled(
                format!("{text:<width$} "),
                Style::default().fg(Color::Blue),
            ));
        };
        if cols.service {
            push_col(parsed.service.as_deref(), SERVICE_WIDTH);
        }
        if cols.host {
            push_col(parsed.host.as_deref(), SERVICE_WIDTH);
        }
        for key in &cols.fields {
            let value = parsed
                .attributes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str());
            push_col(value, FIELD_WIDTH);
        }
    }
    let message: String = raw[ts_len..]
        .trim_start()
        .chars()
        .skip(opts.h_scroll)
        .collect();
    spans.extend(highlight(&message, opts.search, message_style(entry.level)));
    Line::from(spans)
}

/// `HH:MM:SS` from an ISO timestamp, else the timestamp as is.
fn short_time(ts: &str) -> &str {
    ts.find('T')
        .and_then(|t| ts.get(t + 1..t + 9))
        .unwrap_or(ts)
}

/// Split `text` so case-insensitive matches of `term` stand out.
fn highlight(text: &str, term: Option<&str>, base: Style) -> Vec<Span<'static>> {
    let Some(term) = term.filter(|t| !t.is_empty()) else {
        return vec![Span::styled(text.to_string(), base)];
    };
    let lower = text.to_lowercase();
    let needle = term.to_lowercase();
    // Lowercasing can change byte lengths for non-ASCII text; fall back to
    // no highlighting rather than slice at a wrong offset.
    if lower.len() != text.len() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let mut spans = Vec::new();
    let mut last = 0;
    for (at, _) in lower.match_indices(&needle) {
        if at > last {
            spans.push(Span::styled(text[last..at].to_string(), base));
        }
        let end = at + needle.len();
        spans.push(Span::styled(
            text[at..end].to_string(),
            base.add_modifier(Modifier::REVERSED).bold(),
        ));
        last = end;
    }
    if last < text.len() {
        spans.push(Span::styled(text[last..].to_string(), base));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::logs::LogBuffer;

    fn line(raw: &str) -> LogLine {
        let mut b = LogBuffer::new(1);
        b.push(raw.to_string());
        b.entry(0).unwrap().clone()
    }

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn plain_options_are_detected() {
        let cols = Columns::default();
        let opts = RowOptions {
            columns: &cols,
            search: None,
            h_scroll: 0,
        };
        assert!(opts.is_plain());
        let opts = RowOptions {
            columns: &cols,
            search: Some("x"),
            h_scroll: 0,
        };
        assert!(!opts.is_plain());
    }

    #[test]
    fn columns_render_before_the_message() {
        let cols = Columns {
            level: true,
            fields: vec!["status".into()],
            ..Columns::default()
        };
        let opts = RowOptions {
            columns: &cols,
            search: None,
            h_scroll: 0,
        };
        let row = render_row(&line("2026-09-24T15:12:57.1Z ERROR boom status=500"), &opts);
        let t = text(&row);
        assert!(t.starts_with("15:12:57 ERROR 500"), "{t}");
        assert!(t.ends_with("ERROR boom status=500"), "{t}");
    }

    #[test]
    fn search_matches_are_split_out() {
        let spans = highlight("GET /Api ok api", Some("api"), Style::default());
        let hits: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(hits, vec!["Api", "api"]);
    }

    #[test]
    fn horizontal_scroll_skips_message_characters() {
        let cols = Columns {
            time: false,
            ..Columns::default()
        };
        let opts = RowOptions {
            columns: &cols,
            search: None,
            h_scroll: 4,
        };
        let row = render_row(&line("2026-09-24T15:12:57.1Z abcdefgh"), &opts);
        assert_eq!(text(&row), "efgh");
    }
}
