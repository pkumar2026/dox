//! The `docker stats` columns on the container list. Each one takes room from
//! NAME, so only as many as fit are shown, in priority order.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::docker::stats::ContainerStats;

/// NAME keeps at least this many columns before a stats column is added, so
/// compose group headers like `▾ project (3/4)` still read in full.
const NAME_MIN_WIDTH: u16 = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatColumn {
    Cpu,
    Mem,
    Net,
    Io,
    Pids,
}

impl StatColumn {
    const PRIORITY: [StatColumn; 5] = [Self::Cpu, Self::Mem, Self::Net, Self::Io, Self::Pids];

    pub fn header(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Mem => "MEM",
            Self::Net => "NET I/O",
            Self::Io => "BLOCK I/O",
            Self::Pids => "PIDS",
        }
    }

    /// Wide enough for the header and for values up to `800%`, `1023M`,
    /// `1023G/1023G` and 9999 processes. CPU and MEM are gauges, so wider.
    pub fn width(self) -> u16 {
        match self {
            Self::Cpu | Self::Mem => 10,
            Self::Net | Self::Io => 11,
            Self::Pids => 4,
        }
    }

    pub fn cell(self, stats: &ContainerStats) -> Line<'static> {
        match self {
            Self::Cpu => stats.cpu_percent.map_or_else(
                || Line::from("-"),
                |p| gauge(p / 100.0, &format_percent(p), self.width().into()),
            ),
            Self::Mem => match stats.mem_limit {
                0 => Line::from(format_bytes(stats.mem_used)),
                limit => gauge(
                    stats.mem_used as f64 / limit as f64,
                    &format_bytes(stats.mem_used),
                    self.width().into(),
                ),
            },
            Self::Net => Line::from(format!(
                "{}/{}",
                format_bytes(stats.net_rx),
                format_bytes(stats.net_tx)
            )),
            Self::Io => Line::from(format!(
                "{}/{}",
                format_bytes(stats.io_read),
                format_bytes(stats.io_write)
            )),
            Self::Pids => Line::from(stats.pids.map_or_else(|| "-".into(), |n| n.to_string())),
        }
    }
}

/// The stats columns that fit when NAME and they share `room` columns.
pub fn fitting(room: u16) -> Vec<StatColumn> {
    StatColumn::PRIORITY
        .into_iter()
        .scan(NAME_MIN_WIDTH, |used, col| {
            *used += col.width() + 1;
            (*used <= room).then_some(col)
        })
        .collect()
}

/// ctop's gauge: `label` centered in `width` cells, with the filled share
/// (`fraction`, clamped to 0..=1) on a background of the load color. No
/// track, so it reads the same on light and dark themes. A load too small to
/// fill a cell gets a thin edge instead, so the gauge stays visible.
pub fn gauge(fraction: f64, label: &str, width: usize) -> Line<'static> {
    let fraction = fraction.clamp(0.0, 1.0);
    let color = level_color(fraction);
    let text: Vec<char> = format!("{label:^width$}").chars().collect();
    let filled = ((fraction * width as f64).round() as usize).min(text.len());
    if filled == 0 {
        return match text.split_first() {
            Some((' ', rest)) if fraction > 0.0 => Line::from(vec![
                Span::styled("▏", Style::default().fg(color)),
                Span::raw(rest.iter().collect::<String>()),
            ]),
            _ => Line::from(text.iter().collect::<String>()),
        };
    }
    let (bar, rest) = text.split_at(filled);
    Line::from(vec![
        Span::styled(
            bar.iter().collect::<String>(),
            Style::default().bg(color).fg(Color::Black),
        ),
        Span::raw(rest.iter().collect::<String>()),
    ])
}

/// Green below 50%, yellow below 80%, red from there.
pub fn level_color(fraction: f64) -> Color {
    match fraction {
        f if f < 0.5 => Color::Green,
        f if f < 0.8 => Color::Yellow,
        _ => Color::Red,
    }
}

/// `3.2%` under ten, whole numbers above, so a list cell needs four characters.
pub fn format_percent(percent: f64) -> String {
    if percent < 9.95 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// Bytes in at most five characters: `512B`, `9.5K`, `34M`, `1.2G`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "K", "M", "G", "T", "P", "E"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes}B"),
        _ if value < 10.0 => format!("{value:.1}{}", UNITS[unit]),
        _ => format!("{value:.0}{}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    fn text(line: Line) -> String {
        line.to_string()
    }

    #[test]
    fn columns_appear_in_priority_order_as_room_grows() {
        use StatColumn::*;
        assert_eq!(fitting(28), vec![]);
        assert_eq!(fitting(38), vec![], "the CPU gauge needs 10 + a gap");
        assert_eq!(fitting(39), vec![Cpu]);
        assert_eq!(fitting(50), vec![Cpu, Mem]);
        assert_eq!(fitting(62), vec![Cpu, Mem, Net]);
        assert_eq!(fitting(74), vec![Cpu, Mem, Net, Io]);
        assert_eq!(fitting(79), vec![Cpu, Mem, Net, Io, Pids]);
        assert_eq!(fitting(500), vec![Cpu, Mem, Net, Io, Pids]);
    }

    #[test]
    fn a_narrow_column_never_skips_ahead_of_a_wider_one() {
        // 7 spare columns after NET would hold PIDS, but IO comes first.
        assert_eq!(fitting(70).last(), Some(&StatColumn::Net));
    }

    #[test]
    fn headers_fit_their_columns() {
        for col in StatColumn::PRIORITY {
            assert!(col.header().len() <= col.width() as usize, "{col:?}");
        }
    }

    #[test]
    fn bytes_stay_within_five_characters() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(1023), "1023B");
        assert_eq!(format_bytes(1536), "1.5K");
        assert_eq!(format_bytes(34 * MB), "34M");
        assert_eq!(format_bytes(1023 * MB), "1023M");
        assert_eq!(format_bytes(GB + GB / 5), "1.2G");
        assert_eq!(format_bytes(5000 * GB), "4.9T");
        for b in [9 * KB + 1000, 1023 * KB + 1023, u64::MAX] {
            assert!(format_bytes(b).len() <= 5, "{b} -> {}", format_bytes(b));
        }
    }

    fn green_fill() -> Style {
        Style::default().bg(Color::Green).fg(Color::Black)
    }

    #[test]
    fn gauge_fills_its_share_behind_a_centered_label() {
        let line = gauge(0.3, "30%", 10);
        assert_eq!(text(line.clone()), "   30%    ");
        assert_eq!(line.spans[0].content, "   ");
        assert_eq!(line.spans[0].style, green_fill());
        assert_eq!(line.spans[1].content, "30%    ");
        assert_eq!(line.spans[1].style, Style::default());
    }

    #[test]
    fn a_tiny_load_keeps_a_thin_edge_so_the_gauge_stays_visible() {
        let line = gauge(0.02, "2%", 10);
        assert_eq!(text(line.clone()), "▏   2%    ");
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
        assert_eq!(text(gauge(0.0, "0%", 10)), "    0%    ", "nothing for zero");
    }

    #[test]
    fn over_100_percent_fills_the_whole_gauge() {
        let line = gauge(3.0, "300%", 10);
        assert_eq!(line.spans[0].content, "   300%   ");
        assert_eq!(line.spans[0].style.bg, Some(Color::Red));
    }

    #[test]
    fn gauges_go_green_yellow_red_with_load() {
        assert_eq!(level_color(0.1), Color::Green);
        assert_eq!(level_color(0.5), Color::Yellow);
        assert_eq!(level_color(0.8), Color::Red);
        assert_eq!(level_color(2.0), Color::Red);
    }

    fn stats() -> ContainerStats {
        ContainerStats {
            cpu_percent: Some(12.345),
            mem_used: 512 * MB,
            mem_limit: 1024 * MB,
            net_rx: GB + GB / 5,
            net_tx: 34 * MB,
            io_read: 0,
            io_write: 12 * MB,
            pids: Some(23),
        }
    }

    #[test]
    fn cpu_and_mem_render_as_gauges() {
        let s = stats();
        assert_eq!(text(StatColumn::Cpu.cell(&s)), "   12%    ");
        assert_eq!(text(StatColumn::Mem.cell(&s)), "   512M   ");
        let busy = ContainerStats {
            cpu_percent: Some(3.24),
            ..s
        };
        assert_eq!(
            text(StatColumn::Cpu.cell(&busy)),
            "▏  3.2%   ",
            "3% is a thin edge"
        );
    }

    #[test]
    fn gauge_fill_takes_the_load_color() {
        let hot = ContainerStats {
            cpu_percent: Some(95.0),
            ..stats()
        };
        let line = StatColumn::Cpu.cell(&hot);
        assert_eq!(line.spans[0].style.bg, Some(Color::Red));
        assert_eq!(line.spans[0].style.fg, Some(Color::Black));
    }

    #[test]
    fn counters_render_like_docker_stats_compactly() {
        let s = stats();
        assert_eq!(text(StatColumn::Net.cell(&s)), "1.2G/34M");
        assert_eq!(text(StatColumn::Io.cell(&s)), "0B/12M");
        assert_eq!(text(StatColumn::Pids.cell(&s)), "23");
    }

    #[test]
    fn unknown_values_show_a_dash_and_no_limit_shows_no_bar() {
        let s = ContainerStats::default();
        assert_eq!(text(StatColumn::Cpu.cell(&s)), "-");
        assert_eq!(text(StatColumn::Pids.cell(&s)), "-");
        assert_eq!(text(StatColumn::Mem.cell(&s)), "0B");
    }

    #[test]
    fn cells_fit_their_columns_at_realistic_peaks() {
        let busy = ContainerStats {
            cpu_percent: Some(799.9),
            mem_used: 1023 * MB,
            mem_limit: 1024 * MB,
            net_rx: 1023 * GB,
            net_tx: 1023 * GB,
            io_read: 1023 * GB,
            io_write: 1023 * GB,
            pids: Some(9999),
        };
        for col in StatColumn::PRIORITY {
            assert!(col.cell(&busy).width() <= col.width() as usize, "{col:?}");
        }
    }
}
