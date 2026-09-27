//! The `i` Stats popup, laid out like Gonzo's Log Statistics.

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use super::insights_modals::heading;
use super::insights_panel::severity_color;
use crate::app::App;
use crate::insights::severity::Severity;

const TOP_ROWS: usize = 5;

pub fn lines(app: &App) -> Vec<Line<'static>> {
    let s = &app.insights.snapshot;
    let view_len = app.log_view().len();
    let mut out = vec![
        heading("General Statistics"),
        row("Total Logs Processed", s.total_lines.to_string()),
        row("Total Bytes Processed", human_bytes(s.total_bytes)),
        row("Logs in Buffer", app.logs.len().to_string()),
        row("Filtered Logs Displayed", view_len.to_string()),
        row("Uptime", human_secs(s.uptime_secs)),
        row(
            "Current Processing Rate",
            format!("{:.1} logs/sec", s.current_rate),
        ),
        row("Peak Logs per Second", s.peak_rate.to_string()),
        Line::raw(""),
        heading("Severity Distribution"),
    ];
    let total: u64 = s.totals.iter().sum();
    for sev in Severity::ALL {
        let count = s.totals[sev.index()];
        let pct = count as f64 * 100.0 / total.max(1) as f64;
        out.push(Line::from(vec![
            Span::styled(
                format!("  {:<28}", sev.label()),
                Style::default().fg(severity_color(sev)).bold(),
            ),
            Span::raw(format!("{count} ({pct:.1}%)")),
        ]));
    }
    let ratio = if s.pattern_count == 0 {
        0.0
    } else {
        s.pattern_lines as f64 / s.pattern_count as f64
    };
    out.extend([
        Line::raw(""),
        heading("Pattern Analysis"),
        row("Unique Patterns Detected", s.pattern_count.to_string()),
        row("Logs Analyzed for Patterns", s.pattern_lines.to_string()),
        row("Pattern Compression Ratio", format!("{ratio:.1}:1")),
    ]);
    out.extend(top_list("Top Services", &s.services));
    out.extend(top_list("Top Hosts", &s.hosts));
    let attrs: Vec<(String, u64)> = s
        .attributes
        .iter()
        .map(|a| (format!("{} ({} unique)", a.key, a.unique), a.total))
        .collect();
    out.extend(top_list("Top Attributes", &attrs));
    out
}

fn row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {label:<28}")),
        Span::raw(value).bold(),
    ])
}

fn top_list(title: &str, items: &[(String, u64)]) -> Vec<Line<'static>> {
    let mut out = vec![Line::raw(""), heading(title)];
    if items.is_empty() {
        out.push(Line::styled("  none seen", Style::default().dim()));
    }
    out.extend(
        items
            .iter()
            .take(TOP_ROWS)
            .map(|(name, count)| row(name, count.to_string())),
    );
    out
}

fn human_bytes(b: u64) -> String {
    const UNITS: [(&str, f64); 3] = [("GB", 1e9), ("MB", 1e6), ("KB", 1e3)];
    UNITS
        .iter()
        .find(|(_, size)| b as f64 >= *size)
        .map(|(unit, size)| format!("{:.1} {unit}", b as f64 / size))
        .unwrap_or_else(|| format!("{b} B"))
}

fn human_secs(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{:.1}m", s as f64 / 60.0),
        s => format!("{:.1}h", s as f64 / 3600.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2_500), "2.5 KB");
        assert_eq!(human_bytes(3_000_000), "3.0 MB");
        assert_eq!(human_secs(42), "42s");
        assert_eq!(human_secs(90), "1.5m");
        assert_eq!(human_secs(7200), "2.0h");
    }
}
