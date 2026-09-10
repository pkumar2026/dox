use anyhow::{Context, Result};
use bollard::query_parameters::ListNetworksOptionsBuilder;

use super::DockerClient;

#[derive(Debug, Clone)]
pub struct NetworkRow {
    pub id: String,
    pub name: String,
    pub driver: String,
}

const PROTECTED: &[&str] = &["bridge", "host", "none"];

pub async fn list(client: &DockerClient) -> Result<Vec<NetworkRow>> {
    let opts = ListNetworksOptionsBuilder::default().build();
    let raw = client
        .raw()
        .list_networks(Some(opts))
        .await
        .context("docker list_networks failed")?;
    let mut rows = Vec::with_capacity(raw.len());
    for n in raw {
        rows.push(NetworkRow {
            id: n.id.unwrap_or_default(),
            name: n.name.unwrap_or_default(),
            driver: n.driver.unwrap_or_default(),
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub async fn remove(client: &DockerClient, id_or_name: &str) -> Result<()> {
    client
        .raw()
        .remove_network(id_or_name)
        .await
        .with_context(|| format!("remove_network({})", id_or_name))?;
    Ok(())
}

pub fn is_protected(name: &str) -> bool {
    PROTECTED.contains(&name)
}

#[derive(Debug, Clone, Default)]
pub struct PruneResult {
    pub deleted: usize,
}

pub async fn prune_unused(client: &DockerClient) -> Result<PruneResult> {
    use bollard::query_parameters::PruneNetworksOptionsBuilder;
    let opts = PruneNetworksOptionsBuilder::default().build();
    let res = client
        .raw()
        .prune_networks(Some(opts))
        .await
        .context("docker prune_networks failed")?;
    Ok(PruneResult {
        deleted: res.networks_deleted.map(|v| v.len()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_networks_recognised() {
        assert!(is_protected("bridge"));
        assert!(is_protected("host"));
        assert!(is_protected("none"));
        assert!(!is_protected("my-network"));
    }
}
