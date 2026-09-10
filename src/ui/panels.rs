use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Row, Table};
use ratatui::Frame;

use crate::app::{App, Panel};
use crate::docker::containers::normalize_state;
use crate::docker::images::format_size;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, panel: Panel) {
    let focused = panel == app.panel;
    match panel {
        Panel::Containers => draw_containers(frame, area, app, focused),
        Panel::Images => draw_images(frame, area, app, focused),
        Panel::Volumes => draw_volumes(frame, area, app, focused),
        Panel::Networks => draw_networks(frame, area, app, focused),
    }
}

fn dimmed(_: &str) -> Style {
    Style::default().fg(Color::DarkGray)
}

fn container_state_color(state: &str) -> Color {
    match state {
        "running" | "up" => Color::Green,
        "exited" | "dead" | "stopped" => Color::Red,
        "paused" | "restarting" => Color::Yellow,
        "created" => Color::Cyan,
        _ => Color::Reset,
    }
}

fn image_size_color(size_bytes: i64) -> Color {
    const MB: i64 = 1024 * 1024;
    const GB: i64 = MB * 1024;
    match size_bytes {
        s if s >= GB => Color::Red,        // huge
        s if s >= 500 * MB => Color::Yellow, // chunky
        _ => Color::Green,                 // small / fine
    }
}

fn driver_color(driver: &str) -> Color {
    match driver {
        "local" => Color::Cyan,
        "bridge" => Color::Magenta,
        "host" => Color::Yellow,
        "overlay" => Color::Blue,
        _ => Color::Reset,
    }
}

fn header_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
    } else {
        // Subtle highlight on unfocused panels so the user can still see
        // selection state without the colour competing for attention.
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

fn draw_containers(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let rows: Vec<Row> = app
        .visible_containers()
        .iter()
        .filter_map(|&i| app.containers.get(i))
        .map(|c| {
            let state = normalize_state(&c.state, &c.status);
            let style = if focused {
                Style::default().fg(container_state_color(&state))
            } else {
                dimmed(&state)
            };
            Row::new(vec![c.name.clone(), state]).style(style)
        })
        .collect();

    let table = Table::new(rows, [Constraint::Min(10), Constraint::Length(8)])
        .header(Row::new(vec!["NAME", "STATE"]).style(header_style(focused)))
        .row_highlight_style(highlight_style(focused))
        .highlight_symbol(if focused { "▌" } else { " " });
    frame.render_stateful_widget(table, area, &mut app.containers_state);
}

fn draw_images(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let rows: Vec<Row> = app
        .visible_images()
        .iter()
        .filter_map(|&n| app.images.get(n))
        .map(|i| {
            let style = if focused {
                Style::default().fg(image_size_color(i.size_bytes))
            } else {
                dimmed("")
            };
            Row::new(vec![truncate(&i.repo_tag, 28), format_size(i.size_bytes)]).style(style)
        })
        .collect();

    let table = Table::new(rows, [Constraint::Min(10), Constraint::Length(10)])
        .header(Row::new(vec!["REPO:TAG", "SIZE"]).style(header_style(focused)))
        .row_highlight_style(highlight_style(focused))
        .highlight_symbol(if focused { "▌" } else { " " });
    frame.render_stateful_widget(table, area, &mut app.images_state);
}

fn draw_volumes(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let rows: Vec<Row> = app
        .visible_volumes()
        .iter()
        .filter_map(|&i| app.volumes.get(i))
        .map(|v| {
            let style = if focused {
                Style::default().fg(driver_color(&v.driver))
            } else {
                dimmed("")
            };
            Row::new(vec![truncate(&v.name, 22), v.driver.clone()]).style(style)
        })
        .collect();
    let table = Table::new(rows, [Constraint::Min(10), Constraint::Length(8)])
        .header(Row::new(vec!["NAME", "DRIVER"]).style(header_style(focused)))
        .row_highlight_style(highlight_style(focused))
        .highlight_symbol(if focused { "▌" } else { " " });
    frame.render_stateful_widget(table, area, &mut app.volumes_state);
}

fn draw_networks(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let rows: Vec<Row> = app
        .visible_networks()
        .iter()
        .filter_map(|&i| app.networks.get(i))
        .map(|n| {
            let style = if focused {
                Style::default().fg(driver_color(&n.driver))
            } else {
                dimmed("")
            };
            Row::new(vec![truncate(&n.name, 22), n.driver.clone()]).style(style)
        })
        .collect();
    let table = Table::new(rows, [Constraint::Min(10), Constraint::Length(8)])
        .header(Row::new(vec!["NAME", "DRIVER"]).style(header_style(focused)))
        .row_highlight_style(highlight_style(focused))
        .highlight_symbol(if focused { "▌" } else { " " });
    frame.render_stateful_widget(table, area, &mut app.networks_state);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_colors_by_state() {
        assert_eq!(container_state_color("running"), Color::Green);
        assert_eq!(container_state_color("exited"), Color::Red);
        assert_eq!(container_state_color("paused"), Color::Yellow);
        assert_eq!(container_state_color("created"), Color::Cyan);
        assert_eq!(container_state_color("weird"), Color::Reset);
    }

    #[test]
    fn image_colors_by_size() {
        assert_eq!(image_size_color(1), Color::Green);
        assert_eq!(image_size_color(600 * 1024 * 1024), Color::Yellow);
        assert_eq!(image_size_color(2_i64 * 1024 * 1024 * 1024), Color::Red);
    }

    #[test]
    fn driver_colors_known() {
        assert_eq!(driver_color("local"), Color::Cyan);
        assert_eq!(driver_color("bridge"), Color::Magenta);
        assert_eq!(driver_color("host"), Color::Yellow);
        assert_eq!(driver_color("custom"), Color::Reset);
    }
}
