pub mod confirm;
pub mod help;
pub mod logs;
pub mod panels;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
#[allow(unused_imports)]
use ratatui::style::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;

use crate::app::{App, FocusArea, Panel};

const PANEL_TITLES: [&str; 4] = ["Containers", "Images", "Volumes", "Networks"];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_body(frame, chunks[1], app);
    draw_footer(frame, chunks[2], app);

    if app.mode_is_confirm() {
        confirm::draw(frame, area, app);
    }
    if app.mode_is_help() {
        help::draw(frame, area);
    }
    if let Some(toast) = app.toast_text() {
        draw_toast(frame, area, toast);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &mut App) {
    let mouse_tag = if app.mouse_on {
        " mouse:on "
    } else {
        " mouse:off "
    };
    let mouse_style = if app.mouse_on {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().dim()
    };
    let socket_text = format!(
        " {} {} ",
        app.client_socket_short(),
        app.client_version_short()
    );
    let right_width = (mouse_tag.len() + socket_text.len()) as u16;
    let inner = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_width)])
        .split(area);

    // Tabs are laid out here and their column ranges recorded so a click on the
    // header can be mapped back to a panel without re-deriving the widths.
    app.last_header_y = area.y;
    let mut spans: Vec<Span> = Vec::with_capacity(PANEL_TITLES.len() * 2);
    let mut x = inner[0].x;
    for (i, title) in PANEL_TITLES.iter().enumerate() {
        let active = i == app.panel.index();
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().dim()
        };
        let label = format!(" {} ", title);
        let w = label.chars().count() as u16;
        let end = (x + w).min(inner[0].right());
        app.last_tab_spans[i] = (x.min(end), end);
        x = x.saturating_add(w + 1); // +1 for the separator space below
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Line::from(spans), inner[0]);
    let right_line = Line::from(vec![
        Span::styled(mouse_tag.to_string(), mouse_style),
        Span::styled(socket_text, Style::default().dim()),
    ])
    .right_aligned();
    frame.render_widget(right_line, inner[1]);
}

const NARROW_THRESHOLD: u16 = 140;

fn draw_body(frame: &mut Frame, area: Rect, app: &mut App) {
    app.last_body_y = area.y;
    app.last_body_height = area.height;
    if area.width < NARROW_THRESHOLD {
        draw_body_2x2(frame, area, app);
    } else {
        draw_body_1x4(frame, area, app);
    }
}

fn draw_body_1x4(frame: &mut Frame, area: Rect, app: &mut App) {
    let top = app.top_split_percent;
    let bottom = 100u16.saturating_sub(top);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(top), Constraint::Percentage(bottom)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[0]);

    draw_panel(frame, cols[0], app, Panel::Containers);
    draw_panel(frame, cols[1], app, Panel::Images);
    draw_panel(frame, cols[2], app, Panel::Volumes);
    draw_panel(frame, cols[3], app, Panel::Networks);

    draw_logs(frame, rows[1], app);
}

fn draw_body_2x2(frame: &mut Frame, area: Rect, app: &mut App) {
    let top = app.top_split_percent;
    let bottom = 100u16.saturating_sub(top);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(top), Constraint::Percentage(bottom)])
        .split(area);

    // Top half = 2x2 grid of panels.
    let top_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    let top1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top_rows[0]);
    let top2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top_rows[1]);

    draw_panel(frame, top1[0], app, Panel::Containers);
    draw_panel(frame, top1[1], app, Panel::Images);
    draw_panel(frame, top2[0], app, Panel::Volumes);
    draw_panel(frame, top2[1], app, Panel::Networks);

    draw_logs(frame, rows[1], app);
}

fn draw_panel(frame: &mut Frame, area: Rect, app: &mut App, panel: Panel) {
    let focused =
        panel == app.panel && matches!(app.focus, FocusArea::List | FocusArea::Detail);
    let title = panel_title_for(app, panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().dim()
        });
    let inner = block.inner(area);
    app.last_panel_inner[panel.index()] = inner;
    frame.render_widget(block, area);
    panels::draw(frame, inner, app, panel);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = matches!(app.focus, FocusArea::Detail);
    let label = app.active_log_label();
    let total = app.logs.len();
    let inner_h = area.height.saturating_sub(2) as usize;
    let first_visible = if app.logs_follow {
        total.saturating_sub(inner_h)
    } else {
        app.logs_scroll.min(total.saturating_sub(1))
    };
    let last_visible = (first_visible + inner_h).min(total);
    let follow_marker = if app.logs_follow {
        " [follow]"
    } else {
        " [PAUSED — f to resume]"
    };
    let ports = app.active_log_ports();
    let ports_suffix = if ports.is_empty() {
        String::new()
    } else {
        format!("  ports: {ports}")
    };
    let title = format!(
        " Logs: {}{}  {}-{}/{}{} ",
        label,
        ports_suffix,
        first_visible.saturating_add(if total == 0 { 0 } else { 1 }),
        last_visible,
        total,
        follow_marker,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().dim()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    logs::draw(frame, inner, app);
}

fn panel_title_for(app: &App, panel: Panel) -> String {
    let label = PANEL_TITLES[panel.index()];
    let count = app.visible_len(panel);
    let filter = app.filter_query();
    let active = panel == app.panel;
    let marker = if active { "▸ " } else { "  " };
    if filter.is_empty() || !active {
        format!(" {}{} ({}) ", marker, label, count)
    } else {
        format!(" {}{} ({}, /{}) ", marker, label, count, filter)
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let txt = if app.mode_is_filtering() {
        format!(" /{}_", app.filter_query())
    } else {
        " Tab/←→/1-4: panel  ↑↓: nav  Enter: view logs  Esc: back  x/s/r: stop/start/restart  d: delete  D: prune  f: follow  v: select  y: copy  /: filter  ?: help  q: quit ".into()
    };
    frame.render_widget(Line::from(Span::styled(txt, Style::default().dim())), area);
}

fn draw_toast(frame: &mut Frame, area: Rect, text: &str) {
    let w = text.chars().count() as u16 + 4;
    let x = area.x + area.width.saturating_sub(w + 2);
    let y = area.y + 1;
    let rect = Rect {
        x,
        y,
        width: w.min(area.width),
        height: 1,
    };
    let line = Line::from(Span::styled(
        format!(" {} ", text),
        Style::default().fg(Color::Black).bg(Color::Yellow),
    ));
    frame.render_widget(line, rect);
}
