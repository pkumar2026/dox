use anyhow::{Context, Result};
use bollard::query_parameters::{
    ListVolumesOptionsBuilder, PruneVolumesOptionsBuilder, RemoveVolumeOptionsBuilder,
};

use super::DockerClient;

#[derive(Debug, Clone)]
pub struct VolumeRow {
    pub name: String,
    pub driver: String,
}

pub async fn list(client: &DockerClient) -> Result<Vec<VolumeRow>> {
    let opts = ListVolumesOptionsBuilder::default().build();
    let res = client
        .raw()
        .list_volumes(Some(opts))
        .await
        .context("docker list_volumes failed")?;
    let mut rows = Vec::new();
    if let Some(list) = res.volumes {
        for v in list {
            rows.push(VolumeRow {
                name: v.name,
                driver: v.driver,
            });
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub async fn remove(client: &DockerClient, name: &str, force: bool) -> Result<()> {
    let opts = RemoveVolumeOptionsBuilder::default().force(force).build();
    client
        .raw()
        .remove_volume(name, Some(opts))
        .await
        .with_context(|| format!("remove_volume({})", name))?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct PruneResult {
    pub deleted: usize,
    pub space_reclaimed: i64,
}

pub async fn prune_unused(client: &DockerClient) -> Result<PruneResult> {
    let opts = PruneVolumesOptionsBuilder::default().build();
    let res = client
        .raw()
        .prune_volumes(Some(opts))
        .await
        .context("docker prune_volumes failed")?;
    Ok(PruneResult {
        deleted: res.volumes_deleted.map(|v| v.len()).unwrap_or(0),
        space_reclaimed: res.space_reclaimed.unwrap_or(0),
    })
}
