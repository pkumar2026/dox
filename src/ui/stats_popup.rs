//! The `o` popup: one container's live usage, like ctop's single-container
//! view. Four boxes (CPU, memory, network, disk), each with the newest value
//! on top and a graph of the last two minutes below.

use std::collections::VecDeque;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use super::help::centered_rect;
use super::stats_columns::{format_bytes, format_percent, gauge};
use crate::app::App;
use crate::docker::images::format_size;
use crate::docker::stats::ContainerStats;
use crate::stats::HISTORY_LEN;

const MAX_WIDTH: u16 = 140;
const MAX_HEIGHT: u16 = 34;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let Some((id, name)) = app.stats_target.as_ref() else {
        return;
    };
    render(frame, area, name, app.stats.history(id));
}

fn render(frame: &mut Frame, area: Rect, name: &str, history: Option<&VecDeque<ContainerStats>>) {
    let popup = centered_rect(
        area.width.saturating_sub(4).min(MAX_WIDTH),
        area.height.saturating_sub(2).min(MAX_HEIGHT),
        area,
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {name} "),
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(" o / Esc: close ").right_aligned())
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let Some(samples) = history.filter(|h| !h.is_empty()) else {
        let note = Paragraph::new("\n  No stats: the container isn't running.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(note, inner);
        return;
    };
    let latest = &samples[samples.len() - 1];

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(7),
            Constraint::Min(7),
            Constraint::Length(1),
        ])
        .split(inner);
    let (top, bottom) = (halves(rows[0]), halves(rows[1]));
    let (cpu, mem) = (cpu_graph(samples), mem_graph(samples));
    let (net, io) = (net_graph(samples), io_graph(samples));
    draw_box(frame, top[0], &cpu, cpu_gauge(latest, top[0]));
    draw_box(frame, top[1], &mem, mem_gauge(latest, top[1]));
    draw_box(frame, bottom[0], &net, rates(&net));
    draw_box(frame, bottom[1], &io, rates(&io));
    frame.render_widget(totals(latest), rows[2]);
}

fn halves(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area)
}

/// Width of a box's inside, where its gauge goes.
fn inner_width(area: Rect) -> usize {
    usize::from(area.width.saturating_sub(2))
}

fn cpu_gauge(latest: &ContainerStats, area: Rect) -> Line<'static> {
    match latest.cpu_percent {
        Some(p) => gauge(
            p / 100.0,
            &format!("{} of one core", format_percent(p)),
            inner_width(area),
        ),
        None => gauge(0.0, "-", inner_width(area)),
    }
}

/// Sizes as `155.3 MB / 11.65 GB`: the popup has room for the precision
/// `docker stats` shows.
fn mem_gauge(latest: &ContainerStats, area: Rect) -> Line<'static> {
    let used = format_size(latest.mem_used as i64);
    match latest.mem_limit {
        0 => gauge(0.0, &used, inner_width(area)),
        limit => {
            let share = latest.mem_used as f64 / limit as f64;
            let label = format!(
                "{used} / {}  ({})",
                format_size(limit as i64),
                format_percent(share * 100.0)
            );
            gauge(share, &label, inner_width(area))
        }
    }
}

/// `━ rx 12K/s  ━ tx 3.0K/s`: each line's color key and its newest rate.
fn rates(graph: &Graph) -> Line<'static> {
    let spans: Vec<Span> = graph
        .series
        .iter()
        .flat_map(|s| {
            let rate = s.points.last().map_or_else(
                || "-".into(),
                |&(_, y)| format!("{}/s", format_bytes(y as u64)),
            );
            [
                Span::styled(
                    format!(" ━ {} ", s.name),
                    Style::default().fg(s.color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{rate} ")),
            ]
        })
        .collect();
    Line::from(spans)
}

/// One line of a graph: its legend name, color and points (x = seconds ago).
struct Series {
    name: &'static str,
    color: Color,
    points: Vec<(f64, f64)>,
}

struct Graph {
    title: &'static str,
    series: Vec<Series>,
    /// Filled area under a single series; lines for a pair.
    filled: bool,
    label: fn(f64) -> String,
    /// Smallest top of the y axis, so a near-idle graph isn't all noise.
    floor: f64,
}

fn cpu_graph(samples: &VecDeque<ContainerStats>) -> Graph {
    let cpu: Vec<f64> = samples
        .iter()
        .map(|s| s.cpu_percent.unwrap_or(0.0))
        .collect();
    Graph {
        title: " CPU ",
        series: vec![series("", Color::Cyan, &cpu)],
        filled: true,
        label: |v| format!("{v:.0}%"),
        floor: 1.0,
    }
}

fn mem_graph(samples: &VecDeque<ContainerStats>) -> Graph {
    let mem: Vec<f64> = samples.iter().map(|s| s.mem_used as f64).collect();
    Graph {
        title: " Memory ",
        series: vec![series("", Color::Magenta, &mem)],
        filled: true,
        label: |v| format_bytes(v as u64),
        floor: 1024.0 * 1024.0,
    }
}

fn net_graph(samples: &VecDeque<ContainerStats>) -> Graph {
    Graph {
        title: " Network ",
        series: vec![
            series("rx", Color::Green, &deltas(samples, |s| s.net_rx)),
            series("tx", Color::Yellow, &deltas(samples, |s| s.net_tx)),
        ],
        filled: false,
        label: |v| format_bytes(v as u64),
        floor: 1024.0,
    }
}

fn io_graph(samples: &VecDeque<ContainerStats>) -> Graph {
    Graph {
        title: " Block I/O ",
        series: vec![
            series("read", Color::LightBlue, &deltas(samples, |s| s.io_read)),
            series(
                "write",
                Color::LightMagenta,
                &deltas(samples, |s| s.io_write),
            ),
        ],
        filled: false,
        label: |v| format_bytes(v as u64),
        floor: 1024.0,
    }
}

fn series(name: &'static str, color: Color, values: &[f64]) -> Series {
    let newest = values.len() as f64 - 1.0;
    Series {
        name,
        color,
        points: values
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as f64 - newest, v))
            .collect(),
    }
}

/// How much a counter grew from one sample to the next (about one second).
/// A drop means the container restarted, and counts as zero.
fn deltas(samples: &VecDeque<ContainerStats>, counter: fn(&ContainerStats) -> u64) -> Vec<f64> {
    let values: Vec<u64> = samples.iter().map(counter).collect();
    values
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]) as f64)
        .collect()
}

/// A titled box: the newest value on its first line, the graph below.
fn draw_box(frame: &mut Frame, area: Rect, graph: &Graph, header: Line<'static>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(title(graph));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(header, parts[0]);
    frame.render_widget(chart(graph), parts[1]);
}

fn chart(graph: &Graph) -> Chart<'_> {
    let top = graph
        .series
        .iter()
        .flat_map(|s| s.points.iter().map(|&(_, y)| y))
        .fold(graph.floor, f64::max)
        * 1.2;
    let datasets = graph
        .series
        .iter()
        .map(|s| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(if graph.filled {
                    GraphType::Area
                } else {
                    GraphType::Line
                })
                .style(Style::default().fg(s.color))
                .data(&s.points)
        })
        .collect();
    let dim = Style::default().fg(Color::DarkGray);
    Chart::new(datasets)
        .x_axis(
            Axis::default()
                .bounds([-(HISTORY_LEN as f64 - 1.0), 0.0])
                .labels(["-2m", "-1m", "now"])
                .style(dim),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, top])
                .labels([
                    (graph.label)(0.0),
                    (graph.label)(top / 2.0),
                    (graph.label)(top),
                ])
                .style(dim),
        )
}

/// The box's name, in its line's color when it has one line. A pair's
/// colors are keyed on the box's first line instead (see `rates`).
fn title(graph: &Graph) -> Span<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    match graph.series.as_slice() {
        [only] => Span::styled(graph.title, bold.fg(only.color)),
        _ => Span::styled(graph.title, bold),
    }
}

fn totals(latest: &ContainerStats) -> Line<'static> {
    let key = Style::default().add_modifier(Modifier::BOLD);
    let pids = latest.pids.map_or_else(|| "-".into(), |n| n.to_string());
    Line::from(vec![
        Span::styled(" NET ", key),
        Span::raw(format!(
            "{} rx / {} tx",
            format_bytes(latest.net_rx),
            format_bytes(latest.net_tx)
        )),
        Span::styled("   BLOCK I/O ", key),
        Span::raw(format!(
            "{} read / {} write",
            format_bytes(latest.io_read),
            format_bytes(latest.io_write)
        )),
        Span::styled("   PIDS ", key),
        Span::raw(pids),
        Span::styled(
            "   (totals since the container started)",
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample(n: u64) -> ContainerStats {
        ContainerStats {
            cpu_percent: Some(n as f64),
            mem_used: 100 * 1024 * 1024 + n * 1024 * 1024,
            mem_limit: 1024 * 1024 * 1024,
            net_rx: n * 2048,
            net_tx: n * 1024,
            io_read: n * 4096,
            io_write: 0,
            pids: Some(30),
        }
    }

    fn screen(width: u16, height: u16, history: Option<&VecDeque<ContainerStats>>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|f| render(f, f.area(), "temporal-1", history))
            .expect("draw");
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn counters_turn_into_per_sample_rates() {
        let samples: VecDeque<ContainerStats> = [10, 15, 15, 12, 20]
            .into_iter()
            .map(|rx| ContainerStats {
                net_rx: rx,
                ..ContainerStats::default()
            })
            .collect();
        assert_eq!(
            deltas(&samples, |s| s.net_rx),
            vec![5.0, 0.0, 0.0, 8.0],
            "a drop (restart) counts as zero"
        );
    }

    #[test]
    fn newest_point_sits_at_now() {
        let s = series("", Color::Cyan, &[1.0, 2.0, 3.0]);
        assert_eq!(s.points, vec![(-2.0, 1.0), (-1.0, 2.0), (0.0, 3.0)]);
    }

    #[test]
    fn shows_gauges_graphs_and_totals() {
        let history: VecDeque<ContainerStats> = (0..30).map(sample).collect();
        let out = screen(120, 34, Some(&history));
        for text in [
            "temporal-1",
            "o / Esc: close",
            " CPU ",
            "29% of one core",
            " Memory ",
            "129.0 MB / 1.00 GB  (13%)",
            " Network ",
            "━ rx 2.0K/s",
            "━ tx 1.0K/s",
            " Block I/O ",
            "━ read 4.0K/s",
            "━ write 0B/s",
            "-2m",
            "now",
            "NET 58K rx / 29K tx",
            "PIDS 30",
        ] {
            assert!(out.contains(text), "missing {text:?} in:\n{out}");
        }
    }

    #[test]
    fn fits_a_half_width_tmux_pane() {
        let history: VecDeque<ContainerStats> = (0..30).map(sample).collect();
        let out = screen(80, 24, Some(&history));
        for text in [
            " CPU ",
            "29% of one core",
            " Memory ",
            " Network ",
            "PIDS 30",
        ] {
            assert!(out.contains(text), "missing {text:?} in:\n{out}");
        }
    }

    #[test]
    fn a_tiny_terminal_does_not_crash() {
        let history: VecDeque<ContainerStats> = (0..30).map(sample).collect();
        for (w, h) in [(30, 8), (10, 3), (1, 1)] {
            screen(w, h, Some(&history));
        }
    }

    #[test]
    fn a_stopped_container_says_so() {
        let out = screen(120, 34, None);
        assert!(out.contains("isn't running"), "{out}");
        assert!(screen(120, 34, Some(&VecDeque::new())).contains("isn't running"));
    }
}
