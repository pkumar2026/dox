use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::query_parameters::{
    ListImagesOptionsBuilder, PruneImagesOptionsBuilder, RemoveImageOptionsBuilder,
};

use super::DockerClient;

#[derive(Debug, Clone)]
pub struct ImageRow {
    pub id: String,
    pub repo_tag: String,
    pub size_bytes: i64,
}

pub async fn list(client: &DockerClient) -> Result<Vec<ImageRow>> {
    let opts = ListImagesOptionsBuilder::default().all(false).build();
    let raw = client
        .raw()
        .list_images(Some(opts))
        .await
        .context("docker list_images failed")?;
    let mut rows = Vec::with_capacity(raw.len());
    for img in raw {
        let repo_tag = img
            .repo_tags
            .first()
            .cloned()
            .unwrap_or_else(|| "<none>:<none>".into());
        rows.push(ImageRow {
            id: img.id,
            repo_tag,
            size_bytes: img.size,
        });
    }
    rows.sort_by(|a, b| a.repo_tag.cmp(&b.repo_tag));
    Ok(rows)
}

pub async fn remove(client: &DockerClient, id: &str, force: bool) -> Result<()> {
    let opts = RemoveImageOptionsBuilder::default()
        .force(force)
        .noprune(false)
        .build();
    client
        .raw()
        .remove_image(id, Some(opts), None)
        .await
        .with_context(|| format!("remove_image({})", id))?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct PruneResult {
    pub deleted: usize,
    pub space_reclaimed: i64,
}

pub async fn prune_dangling(client: &DockerClient) -> Result<PruneResult> {
    let mut filters = HashMap::new();
    filters.insert("dangling", vec!["true"]);
    let opts = PruneImagesOptionsBuilder::default()
        .filters(&filters)
        .build();
    let res = client
        .raw()
        .prune_images(Some(opts))
        .await
        .context("docker prune_images failed")?;
    Ok(PruneResult {
        deleted: res.images_deleted.map(|v| v.len()).unwrap_or(0),
        space_reclaimed: res.space_reclaimed.unwrap_or(0),
    })
}

pub fn format_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(512), "512 B");
    }
    #[test]
    fn format_size_kb() {
        assert_eq!(format_size(2048), "2.0 KB");
    }
    #[test]
    fn format_size_mb() {
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }
    #[test]
    fn format_size_gb() {
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.00 GB");
    }
}
