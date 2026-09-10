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
        rows.push(ContainerRow {
            id,
            name,
            image,
            state,
            status,
            compose_project,
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
}
