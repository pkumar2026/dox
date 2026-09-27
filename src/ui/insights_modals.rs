//! Insights popups: log details, a field's values, all patterns, the severity
//! filter and column picker, and the full-screen log viewer. The counts
//! analysis and stats popups live in their own files.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::insights_panel::{bar, fit, severity_color};
use super::{insights_counts, insights_stats, logs};
use crate::app::App;
use crate::insights_ui::detail::detail;
use crate::insights_ui::picker::Picker;
use crate::insights_ui::Modal;

const VALUES_SHOWN: usize = 200;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(modal) = app.insights.modal.clone() else {
        return;
    };
    if let Modal::Fullscreen = modal {
        draw_fullscreen(frame, area, app);
        return;
    }
    let popup = popup_area(area, 80, 80);
    frame.render_widget(Clear, popup);
    let (title, lines, hint) = match &modal {
        Modal::Detail { raw } => (
            " Log Details ".to_string(),
            detail_lines(raw),
            "y copy message · Ctrl+y copy all · ↑↓ scroll · Esc close",
        ),
        Modal::AttributeValues { key } => values_popup(app, key),
        Modal::Patterns => patterns_popup(app),
        Modal::Counts => (
            " Log Counts Analysis ".to_string(),
            insights_counts::lines(app, popup.width),
            "↑↓ scroll · Esc close",
        ),
        Modal::Stats => (
            " Log Statistics ".to_string(),
            insights_stats::lines(app),
            "↑↓ scroll · Esc close",
        ),
        Modal::SeverityFilter(p) => picker_popup(
            "Severity Filter",
            p,
            "Space toggle · Enter apply · Esc cancel",
        ),
        Modal::Columns(p) => picker_popup("Columns", p, "Space toggle · Enter apply · Esc cancel"),
        Modal::Fullscreen => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(format!(" {hint} ")).right_aligned())
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.insights.modal_scroll = app.insights.modal_scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.insights.modal_scroll as u16, 0)),
        inner,
    );
}

/// A `width_pct` x `height_pct` rectangle centered in `area`.
pub fn popup_area(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(row);
    cell
}

pub fn heading(text: &str) -> Line<'static> {
    Line::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::default().bold()),
        Span::raw(value),
    ])
}

fn detail_lines(raw: &str) -> Vec<Line<'static>> {
    let d = detail(raw);
    let mut lines = vec![
        field("Time", d.time.clone()),
        Line::from(vec![
            Span::styled(format!("{:<10}", "Severity"), Style::default().bold()),
            Span::styled(
                d.severity.label(),
                Style::default().fg(severity_color(d.severity)).bold(),
            ),
        ]),
        field("Service", d.service.clone().unwrap_or_else(|| "-".into())),
        field("Host", d.host.clone().unwrap_or_else(|| "-".into())),
        field("Message", d.message.clone()),
        Line::raw(""),
        heading(&format!("Attributes ({})", d.attributes.len())),
    ];
    lines.extend(d.attributes.iter().map(|(k, v)| {
        Line::from(vec![
            Span::styled(format!("  {k}"), Style::default().fg(Color::Blue)),
            Span::raw(format!(" = {v}")),
        ])
    }));
    lines.push(Line::raw(""));
    match &d.pretty_json {
        Some(json) => {
            lines.push(heading("JSON"));
            lines.extend(json.lines().map(|l| Line::raw(l.to_string())));
        }
        None => {
            lines.push(heading("Raw"));
            lines.push(Line::raw(d.raw.clone()));
        }
    }
    lines
}

fn values_popup(app: &App, key: &str) -> (String, Vec<Line<'static>>, &'static str) {
    let values = app.insights.engine.attribute_values(key, VALUES_SHOWN);
    let total: u64 = values.iter().map(|(_, c)| c).sum();
    let max = values.first().map(|(_, c)| *c).unwrap_or(0);
    let lines = values
        .iter()
        .map(|(value, count)| {
            let pct = *count as f64 * 100.0 / total.max(1) as f64;
            Line::from(vec![
                Span::raw(format!("{:>8} {:>5.1}% ", count, pct)),
                Span::styled(
                    format!("{:<20} ", bar(*count, max, 20)),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(fit(value, 200)),
            ])
        })
        .collect();
    let distinct = app
        .insights
        .snapshot
        .attributes
        .iter()
        .find(|a| a.key == key)
        .map_or(values.len(), |a| a.unique);
    let title = format!(
        " {key}: {distinct} distinct values, top {} shown ",
        values.len()
    );
    (title, lines, "↑↓ scroll · Esc close")
}

fn patterns_popup(app: &App) -> (String, Vec<Line<'static>>, &'static str) {
    let snap = &app.insights.snapshot;
    let lines = snap
        .patterns
        .iter()
        .map(|p| {
            Line::from(vec![
                Span::raw(format!("{:>8} ", p.count)),
                Span::styled(
                    format!("{:>5.1}% ", p.percent),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(p.template.clone()),
            ])
        })
        .collect();
    let title = format!(
        " Log Patterns: {} patterns from {} logs ",
        snap.pattern_count, snap.pattern_lines
    );
    (title, lines, "↑↓ scroll · Esc close")
}

fn picker_popup(
    name: &str,
    p: &Picker,
    hint: &'static str,
) -> (String, Vec<Line<'static>>, &'static str) {
    let options = p.items.len() - p.actions;
    let title = format!(" {name} ({}/{options} active) ", p.active());
    let lines = p
        .items
        .iter()
        .enumerate()
        .map(|(i, (label, on))| {
            let text = if i < p.actions {
                format!("  {label}")
            } else {
                format!("  [{}] {label}", if *on { "x" } else { " " })
            };
            let style = if i == p.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Line::styled(text, style)
        })
        .collect();
    (title, lines, hint)
}

/// Gonzo's `f` viewer: the log list over the whole screen.
fn draw_fullscreen(frame: &mut Frame, area: Rect, app: &mut App) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Logs (full screen) · ↑↓ move · Enter details · End live · Esc/f close ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    logs::draw(frame, inner, app);
}
