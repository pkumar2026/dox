//! Live resource usage per running container, the numbers `docker stats`
//! (and ctop) show: one stats stream per container, each sample reduced to
//! the few values the container list renders.

use std::collections::HashMap;

use bollard::models::{
    ContainerBlkioStats, ContainerCpuStats, ContainerMemoryStats, ContainerNetworkStats,
    ContainerStatsResponse,
};
use bollard::query_parameters::StatsOptionsBuilder;
use bollard::Docker;
use futures_util::StreamExt;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerStats {
    /// `None` until the daemon has a previous sample to diff against.
    pub cpu_percent: Option<f64>,
    /// Memory in use minus reclaimable page cache, as `docker stats` counts it.
    pub mem_used: u64,
    /// Bytes received / sent since the container started, all networks summed.
    pub net_rx: u64,
    pub net_tx: u64,
    /// Bytes read / written to block devices since the container started.
    pub io_read: u64,
    pub io_write: u64,
    pub pids: Option<u64>,
}

pub fn from_response(r: &ContainerStatsResponse) -> ContainerStats {
    let (net_rx, net_tx) = r.networks.as_ref().map(net_totals).unwrap_or_default();
    let (io_read, io_write) = r.blkio_stats.as_ref().map(io_totals).unwrap_or_default();
    ContainerStats {
        cpu_percent: r
            .cpu_stats
            .as_ref()
            .zip(r.precpu_stats.as_ref())
            .and_then(|(cur, prev)| cpu_percent(cur, prev)),
        mem_used: r.memory_stats.as_ref().map(mem_used).unwrap_or_default(),
        net_rx,
        net_tx,
        io_read,
        io_write,
        pids: r.pids_stats.as_ref().and_then(|p| p.current),
    }
}

/// The Docker CLI's formula: the container's share of host CPU time between
/// the two samples, scaled so one fully busy core reads 100%.
fn cpu_percent(cur: &ContainerCpuStats, prev: &ContainerCpuStats) -> Option<f64> {
    let total = |s: &ContainerCpuStats| s.cpu_usage.as_ref().and_then(|u| u.total_usage);
    let prev_system = prev.system_cpu_usage.filter(|&v| v > 0)?;
    let system_delta = cur
        .system_cpu_usage?
        .checked_sub(prev_system)
        .filter(|&d| d > 0)?;
    let cpu_delta = total(cur)?.saturating_sub(total(prev).unwrap_or(0));
    let cpus = cur
        .online_cpus
        .map(u64::from)
        .filter(|&n| n > 0)
        .or_else(|| {
            let per_cpu = cur.cpu_usage.as_ref()?.percpu_usage.as_ref()?;
            Some(per_cpu.len() as u64)
        })
        .filter(|&n| n > 0)?;
    Some(cpu_delta as f64 * cpus as f64 * 100.0 / system_delta as f64)
}

/// Usage minus inactive page cache, which the kernel can reclaim at will.
/// cgroup v1 reports it as `total_inactive_file`, v2 as `inactive_file`.
fn mem_used(m: &ContainerMemoryStats) -> u64 {
    let usage = m.usage.unwrap_or(0);
    let Some(stats) = m.stats.as_ref() else {
        return usage;
    };
    ["total_inactive_file", "inactive_file"]
        .iter()
        .filter_map(|key| stats.get(*key))
        .find(|&&cache| cache < usage)
        .map_or(usage, |cache| usage - cache)
}

fn net_totals(networks: &HashMap<String, ContainerNetworkStats>) -> (u64, u64) {
    networks.values().fold((0, 0), |(rx, tx), n| {
        (rx + n.rx_bytes.unwrap_or(0), tx + n.tx_bytes.unwrap_or(0))
    })
}

/// Ops are `read`/`write` on cgroup v2 and `Read`/`Write` on v1 (alongside
/// `Sync`, `Total` and friends, which would double count).
fn io_totals(blkio: &ContainerBlkioStats) -> (u64, u64) {
    blkio
        .io_service_bytes_recursive
        .iter()
        .flatten()
        .fold((0, 0), |(read, write), entry| {
            let value = entry.value.unwrap_or(0);
            match entry.op.as_deref().and_then(|op| op.chars().next()) {
                Some('r' | 'R') => (read + value, write),
                Some('w' | 'W') => (read, write + value),
                _ => (read, write),
            }
        })
}

/// Follow one container's stats stream until it ends (the container stopped)
/// or the returned task is aborted, handing each sample to `notify`.
pub fn watch(
    docker: Docker,
    id: String,
    notify: impl Fn(ContainerStats) + Send + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let options = StatsOptionsBuilder::default().stream(true).build();
        let mut stream = Box::pin(docker.stats(&id, Some(options)));
        while let Some(sample) = stream.next().await {
            match sample {
                Ok(r) => notify(from_response(&r)),
                Err(e) => {
                    tracing::debug!(container = %id, error = %e, "stats stream ended");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(v: serde_json::Value) -> ContainerStatsResponse {
        serde_json::from_value(v).expect("fixture parses as a stats response")
    }

    /// A cgroup v2 sample as colima's daemon sends it.
    fn v2_sample() -> ContainerStatsResponse {
        response(json!({
            "cpu_stats": {
                "cpu_usage": { "total_usage": 3_000_000_000_u64 },
                "system_cpu_usage": 40_000_000_000_u64,
                "online_cpus": 4
            },
            "precpu_stats": {
                "cpu_usage": { "total_usage": 2_000_000_000_u64 },
                "system_cpu_usage": 30_000_000_000_u64,
                "online_cpus": 4
            },
            "memory_stats": {
                "usage": 300_000_000_u64,
                "limit": 8_000_000_000_u64,
                "stats": { "inactive_file": 100_000_000_u64 }
            },
            "networks": {
                "eth0": { "rx_bytes": 1000, "tx_bytes": 200 },
                "eth1": { "rx_bytes": 500, "tx_bytes": 50 }
            },
            "blkio_stats": {
                "io_service_bytes_recursive": [
                    { "major": 254, "minor": 0, "op": "read", "value": 4096 },
                    { "major": 254, "minor": 0, "op": "write", "value": 8192 },
                    { "major": 254, "minor": 16, "op": "read", "value": 1024 }
                ]
            },
            "pids_stats": { "current": 23 }
        }))
    }

    #[test]
    fn cpu_percent_matches_docker_stats_formula() {
        // 1s of CPU over 10s of system time on 4 CPUs = 40%.
        let r = v2_sample();
        let got = cpu_percent(
            r.cpu_stats.as_ref().unwrap(),
            r.precpu_stats.as_ref().unwrap(),
        );
        assert_eq!(got, Some(40.0));
    }

    #[test]
    fn cpu_percent_unknown_without_a_previous_sample() {
        // The first message on a stream carries an empty precpu_stats.
        let r = response(json!({
            "cpu_stats": {
                "cpu_usage": { "total_usage": 3_000_000_000_u64 },
                "system_cpu_usage": 40_000_000_000_u64,
                "online_cpus": 4
            },
            "precpu_stats": { "cpu_usage": { "total_usage": 0 } }
        }));
        assert_eq!(from_response(&r).cpu_percent, None);
    }

    #[test]
    fn cpu_percent_is_zero_for_an_idle_container() {
        let r = response(json!({
            "cpu_stats": {
                "cpu_usage": { "total_usage": 2_000_000_000_u64 },
                "system_cpu_usage": 40_000_000_000_u64,
                "online_cpus": 4
            },
            "precpu_stats": {
                "cpu_usage": { "total_usage": 2_000_000_000_u64 },
                "system_cpu_usage": 30_000_000_000_u64
            }
        }));
        assert_eq!(from_response(&r).cpu_percent, Some(0.0));
    }

    #[test]
    fn cpu_count_falls_back_to_percpu_entries() {
        let r = response(json!({
            "cpu_stats": {
                "cpu_usage": { "total_usage": 3_000_000_000_u64, "percpu_usage": [1, 1] },
                "system_cpu_usage": 40_000_000_000_u64
            },
            "precpu_stats": {
                "cpu_usage": { "total_usage": 2_000_000_000_u64 },
                "system_cpu_usage": 30_000_000_000_u64
            }
        }));
        assert_eq!(from_response(&r).cpu_percent, Some(20.0));
    }

    #[test]
    fn mem_used_drops_inactive_page_cache_on_cgroup_v2() {
        let r = v2_sample();
        assert_eq!(mem_used(r.memory_stats.as_ref().unwrap()), 200_000_000);
    }

    #[test]
    fn mem_used_drops_total_inactive_file_on_cgroup_v1() {
        let r = response(json!({
            "memory_stats": {
                "usage": 300_000_000_u64,
                "stats": { "total_inactive_file": 50_000_000_u64, "inactive_file": 1 }
            }
        }));
        assert_eq!(from_response(&r).mem_used, 250_000_000);
    }

    #[test]
    fn mem_used_is_raw_usage_when_cache_is_missing_or_larger() {
        let no_cache = response(json!({ "memory_stats": { "usage": 300 } }));
        assert_eq!(from_response(&no_cache).mem_used, 300);
        let bogus = response(json!({
            "memory_stats": { "usage": 300, "stats": { "inactive_file": 999 } }
        }));
        assert_eq!(from_response(&bogus).mem_used, 300);
    }

    #[test]
    fn network_totals_sum_every_interface() {
        let r = v2_sample();
        assert_eq!(net_totals(r.networks.as_ref().unwrap()), (1500, 250));
    }

    #[test]
    fn block_io_sums_reads_and_writes_across_devices() {
        let r = v2_sample();
        assert_eq!(io_totals(r.blkio_stats.as_ref().unwrap()), (5120, 8192));
    }

    #[test]
    fn block_io_reads_cgroup_v1_capitalized_ops() {
        let r = response(json!({
            "blkio_stats": {
                "io_service_bytes_recursive": [
                    { "op": "Read", "value": 10 },
                    { "op": "Write", "value": 20 },
                    { "op": "Sync", "value": 30 },
                    { "op": "Total", "value": 30 }
                ]
            }
        }));
        let s = from_response(&r);
        assert_eq!((s.io_read, s.io_write), (10, 20));
    }

    #[test]
    fn missing_sections_read_as_zero() {
        // cgroup v2 hosts can send a null blkio list; a host-network container
        // has no networks map.
        let r = response(json!({
            "blkio_stats": { "io_service_bytes_recursive": null }
        }));
        assert_eq!(
            from_response(&r),
            ContainerStats {
                cpu_percent: None,
                ..ContainerStats::default()
            }
        );
    }

    #[test]
    fn full_sample_reduces_to_list_values() {
        assert_eq!(
            from_response(&v2_sample()),
            ContainerStats {
                cpu_percent: Some(40.0),
                mem_used: 200_000_000,
                net_rx: 1500,
                net_tx: 250,
                io_read: 5120,
                io_write: 8192,
                pids: Some(23),
            }
        );
    }
}
