use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const ENTRIES: &[(&str, &str)] = &[
    ("Tab / Shift-Tab", "cycle panels"),
    ("← / →", "previous / next panel"),
    ("1 2 3 4", "jump straight to a panel"),
    ("↑ ↓", "move selection"),
    ("g / G", "jump top / bottom"),
    ("Enter", "focus detail (logs)"),
    ("Esc", "back one level (logs / selection / filter)"),
    ("x / s / r", "stop / start / restart container"),
    ("d", "delete selected"),
    ("D", "prune dangling (images) / unused (volumes, networks)"),
    ("l", "show logs for container"),
    ("f", "toggle live follow"),
    ("v", "enter visual selection in logs"),
    ("y", "yank selection (or whole buffer) to clipboard"),
    ("/", "filter rows"),
    ("m", "toggle mouse capture (wheel scroll ↔ native drag-select)"),
    ("click", "header tab or list row to focus and select"),
    ("wheel", "scrolls whatever is under the pointer"),
    ("drag", "panel/log border resizes · in logs selects lines"),
    ("R", "restart the log stream for selected container"),
    ("+ / - / =", "grow / shrink / reset top panels area"),
    ("?", "this help"),
    ("q / Ctrl-C", "quit"),
];

pub fn draw(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(72, (ENTRIES.len() as u16) + 6, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" dox — keys ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = Vec::with_capacity(ENTRIES.len() + 2);
    lines.push(Line::from(""));
    for (k, v) in ENTRIES {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<18}", k),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(*v),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        Style::default().dim(),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(popup_layout[1])[1]
}
