//! The `docker stats` columns on the container list. Each one takes room from
//! NAME, so only as many as fit are shown, in priority order.

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

    /// Wide enough for the header and for values up to `799.9%`, `1023M`,
    /// `1023G/1023G` and 9999 processes.
    pub fn width(self) -> u16 {
        match self {
            Self::Cpu => 6,
            Self::Mem => 5,
            Self::Net | Self::Io => 11,
            Self::Pids => 4,
        }
    }

    pub fn cell(self, stats: &ContainerStats) -> String {
        match self {
            Self::Cpu => stats
                .cpu_percent
                .map_or_else(|| "-".into(), |p| format!("{p:.1}%")),
            Self::Mem => format_bytes(stats.mem_used),
            Self::Net => format!(
                "{}/{}",
                format_bytes(stats.net_rx),
                format_bytes(stats.net_tx)
            ),
            Self::Io => format!(
                "{}/{}",
                format_bytes(stats.io_read),
                format_bytes(stats.io_write)
            ),
            Self::Pids => stats.pids.map_or_else(|| "-".into(), |n| n.to_string()),
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

/// Bytes in at most five characters: `512B`, `9.5K`, `34M`, `1.2G`.
fn format_bytes(bytes: u64) -> String {
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

    #[test]
    fn columns_appear_in_priority_order_as_room_grows() {
        use StatColumn::*;
        assert_eq!(fitting(28), vec![]);
        assert_eq!(fitting(34), vec![], "CPU needs 6 + a gap");
        assert_eq!(fitting(35), vec![Cpu]);
        assert_eq!(fitting(41), vec![Cpu, Mem]);
        assert_eq!(fitting(53), vec![Cpu, Mem, Net]);
        assert_eq!(fitting(65), vec![Cpu, Mem, Net, Io]);
        assert_eq!(fitting(70), vec![Cpu, Mem, Net, Io, Pids]);
        assert_eq!(fitting(500), vec![Cpu, Mem, Net, Io, Pids]);
    }

    #[test]
    fn a_narrow_column_never_skips_ahead_of_a_wider_one() {
        // 7 spare columns after NET would hold PIDS, but IO comes first.
        assert_eq!(fitting(60).last(), Some(&StatColumn::Net));
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

    fn stats() -> ContainerStats {
        ContainerStats {
            cpu_percent: Some(12.345),
            mem_used: 512 * MB,
            net_rx: GB + GB / 5,
            net_tx: 34 * MB,
            io_read: 0,
            io_write: 12 * MB,
            pids: Some(23),
        }
    }

    #[test]
    fn cells_render_like_docker_stats_compactly() {
        let s = stats();
        assert_eq!(StatColumn::Cpu.cell(&s), "12.3%");
        assert_eq!(StatColumn::Mem.cell(&s), "512M");
        assert_eq!(StatColumn::Net.cell(&s), "1.2G/34M");
        assert_eq!(StatColumn::Io.cell(&s), "0B/12M");
        assert_eq!(StatColumn::Pids.cell(&s), "23");
    }

    #[test]
    fn unknown_values_show_a_dash() {
        let s = ContainerStats::default();
        assert_eq!(StatColumn::Cpu.cell(&s), "-");
        assert_eq!(StatColumn::Pids.cell(&s), "-");
    }

    #[test]
    fn cells_fit_their_columns_at_realistic_peaks() {
        let busy = ContainerStats {
            cpu_percent: Some(799.9),
            mem_used: 1023 * MB,
            net_rx: 1023 * GB,
            net_tx: 1023 * GB,
            io_read: 1023 * GB,
            io_write: 1023 * GB,
            pids: Some(9999),
        };
        for col in StatColumn::PRIORITY {
            assert!(col.cell(&busy).len() <= col.width() as usize, "{col:?}");
        }
    }
}
