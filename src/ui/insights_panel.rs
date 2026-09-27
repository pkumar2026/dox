//! The insights row above the logs: Top Words, Top Attributes, Log Patterns
//! and Log Counts, each in its own box (Gonzo's dashboard, as one row).
//! Colors are named terminal colors so light and dark themes both work.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::insights_charts::{bar_spans, counts_chart, min_max};
use crate::app::{App, FocusArea};
use crate::insights::severity::Severity;
use crate::insights_ui::Section;

const BOX_TITLES: [&str; 4] = ["Top Words", "Top Attributes", "Log Patterns", "Log Counts"];

pub fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Fatal => Color::Magenta,
        Severity::Error => Color::Red,
        Severity::Warn => Color::Yellow,
        Severity::Info => Color::Cyan,
        Severity::Debug | Severity::Trace => Color::DarkGray,
    }
}

/// `█` bar of `value` against `max`, `width` cells wide.
pub fn bar(value: u64, max: u64, width: usize) -> String {
    let filled = if max == 0 {
        0
    } else {
        ((value as f64 / max as f64) * width as f64).ceil() as usize
    };
    "█".repeat(filled.min(width))
}

/// Cut `s` to `width` characters, ending with `…` when cut.
pub fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Row height: 8 chart/list rows plus borders, as in Gonzo.
pub const ROW_HEIGHT: u16 = 10;
/// Box widths in the row: words, attributes, patterns, counts.
const BOX_WIDTHS: [u16; 4] = [22, 22, 30, 26];
/// Gonzo's bar widths: 15 cells, 8 in narrow boxes.
const LIST_BAR: usize = 15;
const LIST_BAR_NARROW: usize = 8;
/// Pattern rows and bar width, as in Gonzo.
const PATTERN_ROWS: usize = 8;
const PATTERN_BAR: usize = 12;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::horizontal(BOX_WIDTHS.map(Constraint::Percentage)).split(area);
    let focused = matches!(app.focus, FocusArea::Detail);
    for (i, rect) in rows.iter().enumerate() {
        app.insights.box_areas[i] = *rect;
        let section = Section::from_box(i).unwrap_or_default();
        let active = focused && app.insights.section == section;
        let mut block = Block::default()
            .borders(Borders::ALL)
            .title(box_title(app, i))
            .border_style(if active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().dim()
            });
        if section == Section::Counts {
            if let Some((min, max)) = min_max(app.insights.counts_history.make_contiguous()) {
                // Like Gonzo, drop the stats when the title would not fit.
                let stats = format!(" Min: {min} | Max: {max} ");
                let needed = BOX_TITLES[3].len() + stats.len() + 6;
                if rect.width as usize >= needed {
                    block = block.title(Line::from(stats).right_aligned());
                }
            }
        }
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        let lines = match section {
            Section::Words => words_lines(app, inner, active),
            Section::Attributes => attribute_lines(app, inner, active),
            Section::Patterns => pattern_lines(app, inner, active),
            _ => {
                let selected = active.then_some(app.insights.selected[3]);
                counts_chart(
                    app.insights.counts_history.make_contiguous(),
                    inner,
                    selected,
                )
            }
        };
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn box_title(app: &App, i: usize) -> String {
    let snap = &app.insights.snapshot;
    let paused = if app.insights.paused { " ⏸" } else { "" };
    match i {
        0 => format!(" {} ({}){paused} ", BOX_TITLES[0], snap.word_count),
        1 => format!(" {} ({}) ", BOX_TITLES[1], snap.attribute_count),
        2 => {
            let over = if snap.pattern_overflow > 0 {
                format!(", {} over limit", snap.pattern_overflow)
            } else {
                String::new()
            };
            format!(
                " {} ({} patterns from {} logs{over}) ",
                BOX_TITLES[2], snap.pattern_count, snap.pattern_lines
            )
        }
        _ => format!(" {} ", BOX_TITLES[3]),
    }
}

/// Rows `offset..offset+height` of a list, keeping `selected` on screen.
fn window(selected: usize, len: usize, height: usize) -> std::ops::Range<usize> {
    let start = selected.saturating_sub(height.saturating_sub(1));
    start..(start + height).min(len)
}

fn highlight(line: Line<'static>, on: bool) -> Line<'static> {
    if on {
        line.patch_style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

/// Gonzo's ranked list row: `NN. label  count |█████░░░░|`.
fn ranked_row(
    rank: usize,
    label: &str,
    value: u64,
    max: u64,
    width: usize,
    count_w: usize,
) -> Line<'static> {
    // "NN. " + label + " " + count + " |" + bar + "|"
    let room = width.saturating_sub(count_w + 8);
    let bar_w = if room >= LIST_BAR + 12 {
        LIST_BAR
    } else if room >= LIST_BAR_NARROW + 8 {
        LIST_BAR_NARROW
    } else {
        room / 2
    };
    let label_w = room - bar_w;
    let [filled, track] = bar_spans(value, max, bar_w, Color::Cyan);
    Line::from(vec![
        Span::raw(format!(
            "{rank:>2}. {:<label_w$} {value:>count_w$} |",
            fit(label, label_w)
        )),
        filled,
        track,
        Span::raw("|"),
    ])
}

fn count_width(max: u64) -> usize {
    max.to_string().len().max(3)
}

fn words_lines(app: &App, area: Rect, active: bool) -> Vec<Line<'static>> {
    let words = &app.insights.snapshot.words;
    if words.is_empty() {
        return vec![empty_hint()];
    }
    let max = words.first().map(|(_, c)| *c).unwrap_or(0);
    let count_w = count_width(max);
    let selected = app.insights.selected[0];
    let searched = app.insights.search.as_deref();
    window(selected, words.len(), area.height as usize)
        .map(|i| {
            let (word, count) = &words[i];
            let label = if searched == Some(word.as_str()) {
                format!("{word} *")
            } else {
                word.clone()
            };
            let row = ranked_row(i + 1, &label, *count, max, area.width as usize, count_w);
            highlight(row, active && i == selected)
        })
        .collect()
}

fn attribute_lines(app: &App, area: Rect, active: bool) -> Vec<Line<'static>> {
    let attrs = &app.insights.snapshot.attributes;
    if attrs.is_empty() {
        return vec![empty_hint()];
    }
    let max = attrs.iter().map(|a| a.unique as u64).max().unwrap_or(0);
    let count_w = count_width(max);
    let selected = app.insights.selected[1];
    window(selected, attrs.len(), area.height as usize)
        .map(|i| {
            let a = &attrs[i];
            let row = ranked_row(
                i + 1,
                &a.key,
                a.unique as u64,
                max,
                area.width as usize,
                count_w,
            );
            highlight(row, active && i == selected)
        })
        .collect()
}

/// Gonzo colors pattern bars by rank: top 3 red, next 3 yellow, rest blue.
fn pattern_color(rank: usize) -> Color {
    match rank {
        0..=2 => Color::Red,
        3..=5 => Color::Yellow,
        _ => Color::Blue,
    }
}

fn pattern_lines(app: &App, area: Rect, active: bool) -> Vec<Line<'static>> {
    let patterns = &app.insights.snapshot.patterns;
    let selected = app.insights.selected[2];
    let rows = (area.height as usize).max(PATTERN_ROWS.min(area.height as usize));
    let range = window(selected, patterns.len().max(PATTERN_ROWS), rows);
    let max = patterns.first().map(|p| p.count).unwrap_or(0);
    let text_w = (area.width as usize).saturating_sub(PATTERN_BAR + 11);
    range
        .map(|i| match patterns.get(i) {
            Some(p) => {
                let [filled, track] = bar_spans(p.count, max, PATTERN_BAR, pattern_color(i));
                let row = Line::from(vec![
                    filled,
                    track,
                    Span::styled(format!(" {:>5.1}% │ ", p.percent), Style::default().dim()),
                    Span::raw(fit(&p.template, text_w)),
                ]);
                highlight(row, active && i == selected)
            }
            None => Line::styled(
                format!("{}        │ (no pattern)", "░".repeat(PATTERN_BAR)),
                Style::default().fg(Color::DarkGray),
            ),
        })
        .collect()
}

fn empty_hint() -> Line<'static> {
    Line::styled("No data available", Style::default().dim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_scales_to_width() {
        assert_eq!(bar(5, 10, 10).chars().count(), 5);
        assert_eq!(bar(10, 10, 10).chars().count(), 10);
        assert_eq!(bar(0, 0, 10), "");
    }

    #[test]
    fn fit_marks_cut_text() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("abc", 4), "abc");
    }

    #[test]
    fn ranked_rows_fit_their_box() {
        for width in [20usize, 24, 30, 38, 45, 60, 90] {
            let row = ranked_row(10, "a_very_long_attribute_name", 426, 484, width, 3);
            let len: usize = row.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(len <= width, "width {width}: row is {len}");
            let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.ends_with('|'), "width {width}: {text}");
        }
    }

    #[test]
    fn window_keeps_selection_visible() {
        assert_eq!(window(0, 10, 3), 0..3);
        assert_eq!(window(5, 10, 3), 3..6);
        assert_eq!(window(9, 10, 3), 7..10);
    }
}
