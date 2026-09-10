use anyhow::{Context, Result};
use bollard::query_parameters::{
    ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    RestartContainerOptionsBuilder, StartContainerOptions, StopContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::Stream;

use super::DockerClient;

/// Docker sets this on every container a `docker compose` stack creates.
const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";

#[derive(Debug, Clone)]
pub struct ContainerRow {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    /// Compose project this container belongs to, if any (see
    /// `COMPOSE_PROJECT_LABEL`). Drives the grouped list view.
    pub compose_project: Option<String>,
    /// Deduped, sorted port mappings. Docker reports the same published
    /// port once per bound IP stack (0.0.0.0 and [::]); we collapse those
    /// down to one entry per (container_port, host_port, protocol).
    pub ports: Vec<PortMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PortMapping {
    pub container_port: u16,
    /// `None` when the port is exposed but not published to the host.
    pub host_port: Option<u16>,
    pub protocol: String,
}

/// `host_port->container_port/protocol`, or just `container_port/protocol`
/// when nothing is published — matches `docker ps`'s own notation.
pub fn format_port_mapping(p: &PortMapping) -> String {
    match p.host_port {
        Some(host) => format!("{host}->{}/{}", p.container_port, p.protocol),
        None => format!("{}/{}", p.container_port, p.protocol),
    }
}

/// First mapping plus a "+N" badge for the rest — for a narrow list column.
pub fn format_ports_compact(ports: &[PortMapping]) -> String {
    match ports.split_first() {
        None => String::new(),
        Some((first, [])) => format_port_mapping(first),
        Some((first, rest)) => format!("{} +{}", format_port_mapping(first), rest.len()),
    }
}

/// Every mapping, comma-separated — for a full-width detail line.
pub fn format_ports_full(ports: &[PortMapping]) -> String {
    ports
        .iter()
        .map(format_port_mapping)
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn list(client: &DockerClient) -> Result<Vec<ContainerRow>> {
    let opts = ListContainersOptionsBuilder::default().all(true).build();
    let raw = client
        .raw()
        .list_containers(Some(opts))
        .await
        .context("docker list_containers failed")?;
    let mut rows = Vec::with_capacity(raw.len());
    for c in raw {
        let id = c.id.unwrap_or_default();
        let name = c
            .names
            .as_ref()
            .and_then(|ns| ns.first().cloned())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| short(&id));
        let image = c.image.unwrap_or_default();
        let state = c.state.map(|s| s.to_string()).unwrap_or_default();
        let status = c.status.unwrap_or_default();
        let compose_project = c
            .labels
            .as_ref()
            .and_then(|labels| labels.get(COMPOSE_PROJECT_LABEL))
            .filter(|p| !p.is_empty())
            .cloned();
        let mut ports: Vec<PortMapping> = c
            .ports
            .unwrap_or_default()
            .into_iter()
            .map(|p| PortMapping {
                container_port: p.private_port,
                host_port: p.public_port,
                protocol: p.typ.map(|t| t.to_string()).unwrap_or_default(),
            })
            .collect();
        ports.sort();
        ports.dedup();
        rows.push(ContainerRow {
            id,
            name,
            image,
            state,
            status,
            compose_project,
            ports,
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub async fn start(client: &DockerClient, id: &str) -> Result<()> {
    client
        .raw()
        .start_container(id, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("start_container({})", short(id)))?;
    Ok(())
}

pub async fn stop(client: &DockerClient, id: &str, timeout_secs: i32) -> Result<()> {
    let opts = StopContainerOptionsBuilder::default()
        .t(timeout_secs)
        .build();
    client
        .raw()
        .stop_container(id, Some(opts))
        .await
        .with_context(|| format!("stop_container({})", short(id)))?;
    Ok(())
}

pub async fn restart(client: &DockerClient, id: &str, timeout_secs: i32) -> Result<()> {
    let opts = RestartContainerOptionsBuilder::default()
        .t(timeout_secs)
        .build();
    client
        .raw()
        .restart_container(id, Some(opts))
        .await
        .with_context(|| format!("restart_container({})", short(id)))?;
    Ok(())
}

pub async fn remove(
    client: &DockerClient,
    id: &str,
    force: bool,
    remove_volumes: bool,
) -> Result<()> {
    let opts = RemoveContainerOptionsBuilder::default()
        .force(force)
        .v(remove_volumes)
        .build();
    client
        .raw()
        .remove_container(id, Some(opts))
        .await
        .with_context(|| format!("remove_container({})", short(id)))?;
    Ok(())
}

pub fn logs(
    docker: &Docker,
    id: &str,
    tail_lines: u64,
) -> impl Stream<Item = Result<String, bollard::errors::Error>> + Send + 'static {
    use futures_util::StreamExt;
    let opts = LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .follow(true)
        .timestamps(true)
        .tail(&tail_lines.to_string())
        .build();
    docker
        .logs(id, Some(opts))
        .map(|res| res.map(|chunk| chunk.to_string()))
}

pub fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

pub fn normalize_state(state: &str, status: &str) -> String {
    if !state.is_empty() {
        return state.to_lowercase();
    }
    status
        .split_whitespace()
        .next()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_truncates_to_12_chars() {
        assert_eq!(short("0123456789abcdef0123"), "0123456789ab");
    }

    #[test]
    fn short_id_safe_when_already_short() {
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn normalize_state_uses_state_when_present() {
        assert_eq!(normalize_state("Running", ""), "running");
    }

    #[test]
    fn normalize_state_falls_back_to_status_word() {
        assert_eq!(normalize_state("", "Up 2 hours"), "up");
        assert_eq!(normalize_state("", "Exited (0) 3 minutes ago"), "exited");
    }

    #[test]
    fn normalize_state_unknown_when_both_empty() {
        assert_eq!(normalize_state("", ""), "unknown");
    }

    fn port(container: u16, host: Option<u16>, proto: &str) -> PortMapping {
        PortMapping {
            container_port: container,
            host_port: host,
            protocol: proto.to_string(),
        }
    }

    #[test]
    fn format_published_port_matches_docker_ps_notation() {
        assert_eq!(
            format_port_mapping(&port(8080, Some(8081), "tcp")),
            "8081->8080/tcp"
        );
    }

    #[test]
    fn format_unpublished_port_omits_arrow() {
        assert_eq!(format_port_mapping(&port(8000, None, "tcp")), "8000/tcp");
    }

    #[test]
    fn compact_shows_only_first_plus_count_badge() {
        let ports = vec![
            port(7233, Some(7233), "tcp"),
            port(8000, None, "tcp"),
            port(8080, Some(8080), "tcp"),
        ];
        assert_eq!(format_ports_compact(&ports), "7233->7233/tcp +2");
    }

    #[test]
    fn compact_single_port_has_no_badge() {
        assert_eq!(
            format_ports_compact(&[port(5432, Some(5432), "tcp")]),
            "5432->5432/tcp"
        );
    }

    #[test]
    fn compact_empty_is_empty_string() {
        assert_eq!(format_ports_compact(&[]), "");
    }

    #[test]
    fn full_lists_every_mapping_comma_separated() {
        let ports = vec![port(7233, Some(7233), "tcp"), port(8000, None, "tcp")];
        assert_eq!(format_ports_full(&ports), "7233->7233/tcp, 8000/tcp");
    }

    /// Docker reports the same published port once per bound IP stack —
    /// e.g. brand-boost-temporal-1 in the wild reports 0.0.0.0:7233->7233/tcp
    /// AND [::]:7233->7233/tcp as two separate PortSummary entries. `list`
    /// dedupes these; this test locks in that behavior at the sort+dedup
    /// step directly (ip isn't part of PortMapping, so a naive collect
    /// would otherwise show every port twice).
    #[test]
    fn dedup_collapses_ipv4_ipv6_duplicates() {
        let mut ports = vec![
            port(7233, Some(7233), "tcp"), // 0.0.0.0
            port(7233, Some(7233), "tcp"), // [::]
            port(8000, None, "tcp"),
        ];
        ports.sort();
        ports.dedup();
        assert_eq!(
            ports,
            vec![port(7233, Some(7233), "tcp"), port(8000, None, "tcp")]
        );
    }
}
