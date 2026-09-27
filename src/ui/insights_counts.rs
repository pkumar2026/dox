//! The Counts analysis popup (Enter on Log Counts), laid out like Gonzo's:
//! a 60-minute severity activity heatmap, then the top patterns and services
//! per severity.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::insights_modals::heading;
use super::insights_panel::{fit, severity_color};
use crate::app::App;
use crate::insights::severity::Severity;
use crate::insights::timeline::{SeverityCounts, WINDOW_MINUTES};

/// Left column of the heatmap; the row labels are padded to the same width.
const HEADER: &str = "Time (mins ago):";
const LEGEND: &str = "Legend: █ High Activity  ▓ Medium Activity  ▒ Low Activity  . No Activity";

pub fn lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let s = &app.insights.snapshot;
    let mut out = vec![
        heading("Severity Activity Heatmap (Last 60 Minutes)"),
        Line::raw(format!("{HEADER}{}", axis())),
        Line::styled(
            "─".repeat(HEADER.chars().count() + WINDOW_MINUTES),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ];
    for sev in Severity::ALL {
        out.push(heat_row(sev, &s.per_minute));
    }
    out.push(Line::raw(""));
    out.push(Line::raw(LEGEND));
    out.push(Line::raw(""));
    let text_w = (width as usize).saturating_sub(24);
    for sev in Severity::ALL {
        let breakdown = &s.by_severity[sev.index()];
        out.push(Line::styled(
            format!("{} ({})", sev.label(), s.totals[sev.index()]),
            Style::default()
                .fg(severity_color(sev))
                .add_modifier(Modifier::BOLD),
        ));
        if breakdown.patterns.is_empty() {
            out.push(Line::styled(
                "   no logs",
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        for p in &breakdown.patterns {
            out.push(Line::raw(format!(
                "   {:>6} {:>5.1}%  {}",
                p.count,
                p.percent,
                fit(&p.template, text_w)
            )));
        }
        if !breakdown.services.is_empty() {
            let services: Vec<String> = breakdown
                .services
                .iter()
                .map(|(name, count)| format!("{name} ({count})"))
                .collect();
            out.push(Line::styled(
                format!("   services: {}", services.join(", ")),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
    }
    out
}

/// Heatmap colors, as Gonzo's: red for FATAL and ERROR, orange (yellow here)
/// for WARN, blue for INFO, gray for DEBUG and TRACE.
fn heat_color(sev: Severity) -> Color {
    match sev {
        Severity::Fatal | Severity::Error => Color::Red,
        Severity::Warn => Color::Yellow,
        Severity::Info => Color::Blue,
        Severity::Debug | Severity::Trace => Color::DarkGray,
    }
}

/// Minutes-ago labels at 60, 50, ... 10 and 0, one cell per minute.
fn axis() -> String {
    let mut cells = vec![' '; WINDOW_MINUTES];
    for ago in (0..WINDOW_MINUTES + 1).step_by(10) {
        let label = ago.to_string();
        let at = (WINDOW_MINUTES - 1)
            .saturating_sub(ago)
            .min(WINDOW_MINUTES - label.len());
        for (i, ch) in label.chars().enumerate() {
            cells[at + i] = ch;
        }
    }
    cells.into_iter().collect()
}

/// `SEVERITY (total)` padded to the header width.
fn row_label(sev: Severity, total: u64) -> String {
    let width = HEADER.chars().count();
    format!("{:<width$}", format!("{} ({total})", sev.label()))
}

/// One severity's minutes, shaded against that severity's busiest minute.
fn heat_row(sev: Severity, minutes: &[SeverityCounts]) -> Line<'static> {
    let counts: Vec<u64> = minutes.iter().map(|m| m[sev.index()]).collect();
    let total: u64 = counts.iter().sum();
    let max = counts.iter().copied().max().unwrap_or(0).max(1);
    let color = heat_color(sev);
    let mut spans = vec![Span::styled(
        row_label(sev, total),
        Style::default()
            .fg(severity_color(sev))
            .add_modifier(Modifier::BOLD),
    )];
    spans.extend(counts.iter().map(|c| match shade(*c, max) {
        "." => Span::raw("."),
        symbol => Span::styled(symbol, Style::default().fg(color)),
    }));
    Line::from(spans)
}

/// Gonzo's intensity thresholds: above 70% `█`, 40% `▓`, 10% `▒`, else `░`.
fn shade(count: u64, max: u64) -> &'static str {
    if count == 0 || max == 0 {
        return ".";
    }
    let intensity = count as f64 / max as f64;
    if intensity > 0.7 {
        "█"
    } else if intensity > 0.4 {
        "▓"
    } else if intensity > 0.1 {
        "▒"
    } else {
        "░"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shade_follows_gonzo_thresholds() {
        assert_eq!(shade(0, 10), ".");
        assert_eq!(shade(1, 10), "░");
        assert_eq!(shade(3, 10), "▒");
        assert_eq!(shade(5, 10), "▓");
        assert_eq!(shade(8, 10), "█");
        assert_eq!(shade(10, 10), "█");
    }

    #[test]
    fn row_label_is_severity_and_total() {
        assert_eq!(row_label(Severity::Error, 42), "ERROR (42)      ");
        assert_eq!(
            row_label(Severity::Error, 42).chars().count(),
            HEADER.chars().count()
        );
    }

    #[test]
    fn axis_labels_line_up() {
        let a = axis();
        assert_eq!(a.chars().count(), WINDOW_MINUTES);
        assert!(a.starts_with("60"), "{a}");
        assert!(a.ends_with('0'), "{a}");
    }
}
