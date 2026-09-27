//! Chart pieces for the insights side panel, drawn the way Gonzo draws them:
//! list bars in a `░` track, and the Log Counts stacked severity bars with a
//! legend of the latest interval.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::insights_panel::severity_color;
use crate::insights::severity::Severity;
use crate::insights::timeline::SeverityCounts;

/// Legend width, as in Gonzo.
const LEGEND_WIDTH: usize = 18;
/// Bottom to top in each stacked bar.
const STACK_ORDER: [Severity; 6] = [
    Severity::Trace,
    Severity::Debug,
    Severity::Info,
    Severity::Warn,
    Severity::Error,
    Severity::Fatal,
];

/// Gonzo's list bar split into `(filled, track)`: `█` up to the value's share
/// of `width`, `░` for the rest. A non-zero value always gets one `█`.
pub fn track_bar(value: u64, max: u64, width: usize) -> (String, String) {
    let share = if max == 0 {
        0
    } else {
        (value as f64 / max as f64 * width as f64) as usize
    };
    let filled = share.max(usize::from(value > 0)).min(width);
    ("█".repeat(filled), "░".repeat(width - filled))
}

/// Which severity fills each cell of one stacked bar, bottom first; `None`
/// is empty. Scaled so `max_total` fills `height`. Warnings and worse always
/// get a cell when they occurred, so a rare error never disappears.
pub fn stack_cells(
    counts: &SeverityCounts,
    max_total: u64,
    height: usize,
) -> Vec<Option<Severity>> {
    let mut cells = vec![None; height];
    let total: u64 = counts.iter().sum();
    if total == 0 || max_total == 0 || height == 0 {
        return cells;
    }
    let bar = ((total as f64 / max_total as f64) * height as f64).round() as usize;
    let bar = bar.clamp(1, height);
    // Cumulative rounding keeps the parts summing to the bar's height.
    let mut alloc = [0usize; 6];
    let (mut running, mut placed) = (0u64, 0usize);
    for s in STACK_ORDER {
        running += counts[s.index()];
        let upto = ((running as f64 / total as f64) * bar as f64).round() as usize;
        alloc[s.index()] = upto - placed;
        placed = upto;
    }
    for s in [Severity::Warn, Severity::Error, Severity::Fatal] {
        if counts[s.index()] == 0 || alloc[s.index()] > 0 {
            continue;
        }
        let donor = (0..6).filter(|&i| alloc[i] > 1).max_by_key(|&i| alloc[i]);
        match donor {
            Some(d) => alloc[d] -= 1,
            None if placed < height => placed += 1,
            None => continue,
        }
        alloc[s.index()] += 1;
    }
    let order = STACK_ORDER
        .iter()
        .flat_map(|s| std::iter::repeat_n(Some(*s), alloc[s.index()]));
    for (cell, sev) in cells.iter_mut().zip(order) {
        *cell = sev;
    }
    cells
}

/// Log Counts box body: stacked bars (newest right) plus the legend.
pub fn counts_chart(
    history: &[SeverityCounts],
    area: Rect,
    selected: Option<usize>,
) -> Vec<Line<'static>> {
    let height = area.height as usize;
    let width = area.width as usize;
    if history.is_empty() {
        return vec![Line::styled(
            "No data available",
            Style::default().add_modifier(Modifier::DIM),
        )];
    }
    let chart_w = width.saturating_sub(LEGEND_WIDTH + 2);
    let max_bars = (chart_w / 2).max(1);
    let shown = &history[history.len().saturating_sub(max_bars)..];
    let max_total = shown
        .iter()
        .map(|c| c.iter().sum::<u64>())
        .max()
        .unwrap_or(0);
    let columns: Vec<Vec<Option<Severity>>> = shown
        .iter()
        .map(|c| stack_cells(c, max_total, height))
        .collect();
    let pad = chart_w.saturating_sub(columns.len() * 2);
    let latest = history.last().copied().unwrap_or([0; 6]);
    let legend = legend_lines(&latest, selected);
    (0..height)
        .map(|row| {
            let level = height - 1 - row;
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            for column in &columns {
                spans.push(match column.get(level).copied().flatten() {
                    Some(sev) => Span::styled("█", Style::default().fg(severity_color(sev))),
                    None => Span::raw(" "),
                });
                spans.push(Span::raw(" "));
            }
            spans.push(Span::raw("  "));
            if let Some(entry) = legend.get(row) {
                spans.push(entry.clone());
            }
            Line::from(spans)
        })
        .collect()
}

fn legend_lines(latest: &SeverityCounts, selected: Option<usize>) -> Vec<Span<'static>> {
    let mut lines: Vec<Span<'static>> = Severity::ALL
        .iter()
        .map(|s| {
            let mut style = Style::default().fg(severity_color(*s));
            if selected == Some(s.index()) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Span::styled(format!("{:<6}:{:>6}", s.label(), latest[s.index()]), style)
        })
        .collect();
    lines.push(Span::styled(
        "─".repeat(13),
        Style::default().add_modifier(Modifier::DIM),
    ));
    lines.push(Span::raw(format!(
        "{:<6}:{:>6}",
        "TOTAL",
        latest.iter().sum::<u64>()
    )));
    lines
}

/// `Min: a | Max: b` of interval totals, for the Log Counts title.
pub fn min_max(history: &[SeverityCounts]) -> Option<(u64, u64)> {
    let totals = history.iter().map(|c| c.iter().sum::<u64>());
    Some((totals.clone().min()?, totals.max()?))
}

/// Filled and track parts of a list bar, styled.
pub fn bar_spans(value: u64, max: u64, width: usize, color: Color) -> [Span<'static>; 2] {
    let (filled, track) = track_bar(value, max, width);
    [
        Span::styled(filled, Style::default().fg(color)),
        // Same color: `░` then reads as a lighter shade of the bar (Gonzo).
        Span::styled(track, Style::default().fg(color)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(pairs: &[(Severity, u64)]) -> SeverityCounts {
        let mut c = [0; 6];
        for (s, n) in pairs {
            c[s.index()] = *n;
        }
        c
    }

    #[test]
    fn bar_track_uses_the_bar_color() {
        let [filled, track] = bar_spans(3, 10, 10, Color::Red);
        assert_eq!(filled.style.fg, Some(Color::Red));
        assert_eq!(
            track.style.fg,
            Some(Color::Red),
            "track is a lighter shade of the same color"
        );
    }

    #[test]
    fn track_bar_fills_its_share() {
        assert_eq!(track_bar(5, 10, 10), ("█".repeat(5), "░".repeat(5)));
        assert_eq!(track_bar(0, 10, 4), (String::new(), "░".repeat(4)));
        assert_eq!(track_bar(1, 1000, 4).0, "█");
        assert_eq!(track_bar(10, 10, 4).1, "");
        assert_eq!(track_bar(0, 0, 3), (String::new(), "░".repeat(3)));
    }

    #[test]
    fn stack_orders_severities_bottom_up() {
        let c = counts(&[(Severity::Info, 6), (Severity::Error, 2)]);
        let cells = stack_cells(&c, 8, 8);
        assert_eq!(cells[..6], [Some(Severity::Info); 6]);
        assert_eq!(cells[6..], [Some(Severity::Error); 2]);
    }

    #[test]
    fn stack_scales_to_the_busiest_bar() {
        let c = counts(&[(Severity::Info, 4)]);
        let cells = stack_cells(&c, 16, 8);
        assert_eq!(cells.iter().filter(|c| c.is_some()).count(), 2);
        assert_eq!(cells[0], Some(Severity::Info));
        assert_eq!(cells[7], None);
    }

    #[test]
    fn a_rare_error_still_shows() {
        let c = counts(&[(Severity::Info, 100), (Severity::Error, 1)]);
        let cells = stack_cells(&c, 101, 8);
        assert_eq!(cells.len(), 8);
        assert!(cells.contains(&Some(Severity::Error)));
        assert!(cells.contains(&Some(Severity::Info)));
    }

    #[test]
    fn empty_bar_has_no_cells() {
        assert_eq!(stack_cells(&[0; 6], 10, 3), vec![None; 3]);
        assert_eq!(stack_cells(&[0; 6], 0, 2), vec![None; 2]);
    }

    #[test]
    fn min_max_of_totals() {
        let h = vec![
            counts(&[(Severity::Info, 3)]),
            counts(&[(Severity::Warn, 1), (Severity::Info, 9)]),
        ];
        assert_eq!(min_max(&h), Some((3, 10)));
        assert_eq!(min_max(&[]), None);
    }
}
