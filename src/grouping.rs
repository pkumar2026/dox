use std::collections::HashSet;

use crate::docker::containers::{normalize_state, ContainerRow};

/// One section of the grouped container list: either a compose project with
/// its member containers, or the bucket of containers with no compose label
/// (`project: None`, rendered flat with no header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub project: Option<String>,
    /// Indices into the source `containers` slice, sorted by name.
    pub members: Vec<usize>,
    pub running: usize,
}

impl Group {
    pub fn total(&self) -> usize {
        self.members.len()
    }
}

/// Group `matched` indices into compose projects: sorted by project name,
/// ungrouped containers last, members sorted by name within each group.
/// Pure and independent of collapse state — collapsing only changes which
/// members `flatten_selectable` keeps navigable.
pub fn build_groups(containers: &[ContainerRow], matched: &[usize]) -> Vec<Group> {
    let mut by_project: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for &idx in matched {
        let Some(row) = containers.get(idx) else {
            continue;
        };
        let key = row.compose_project.clone();
        match by_project.iter_mut().find(|(p, _)| *p == key) {
            Some((_, members)) => members.push(idx),
            None => by_project.push((key, vec![idx])),
        }
    }

    for (_, members) in &mut by_project {
        members.sort_by(|&a, &b| containers[a].name.cmp(&containers[b].name));
    }

    by_project.sort_by(|(a, _), (b, _)| match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    });

    by_project
        .into_iter()
        .map(|(project, members)| {
            let running = members
                .iter()
                .filter(|&&i| {
                    normalize_state(&containers[i].state, &containers[i].status) == "running"
                })
                .count();
            Group {
                project,
                members,
                running,
            }
        })
        .collect()
}

/// Flatten groups into the selectable index list. Members of a collapsed
/// project are dropped entirely — not just visually hidden, but unreachable
/// by navigation until the group reopens.
pub fn flatten_selectable(groups: &[Group], collapsed: &HashSet<String>) -> Vec<usize> {
    groups
        .iter()
        .filter(|g| g.project.as_deref().is_none_or(|p| !collapsed.contains(p)))
        .flat_map(|g| g.members.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, project: Option<&str>, state: &str) -> ContainerRow {
        ContainerRow {
            id: name.to_string(),
            name: name.to_string(),
            image: "img".to_string(),
            state: state.to_string(),
            status: String::new(),
            compose_project: project.map(str::to_string),
        }
    }

    #[test]
    fn groups_by_project_and_sorts_alphabetically() {
        let rows = vec![
            row("web-1", Some("beta"), "running"),
            row("db-1", Some("alpha"), "running"),
            row("web-2", Some("alpha"), "exited"),
        ];
        let matched = vec![0, 1, 2];
        let groups = build_groups(&rows, &matched);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].project.as_deref(), Some("alpha"));
        // members sorted by name within the group, not insertion order
        assert_eq!(groups[0].members, vec![1, 2]);
        assert_eq!(groups[1].project.as_deref(), Some("beta"));
        assert_eq!(groups[1].members, vec![0]);
    }

    #[test]
    fn ungrouped_bucket_sorts_after_named_groups() {
        let rows = vec![
            row("standalone", None, "running"),
            row("web-1", Some("alpha"), "running"),
        ];
        let matched = vec![0, 1];
        let groups = build_groups(&rows, &matched);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].project.as_deref(), Some("alpha"));
        assert_eq!(groups[1].project, None);
        assert_eq!(groups[1].members, vec![0]);
    }

    #[test]
    fn running_count_reflects_normalized_state() {
        let rows = vec![
            row("a", Some("proj"), "running"),
            row("b", Some("proj"), "exited"),
            row("c", Some("proj"), "Running"),
        ];
        let matched = vec![0, 1, 2];
        let groups = build_groups(&rows, &matched);

        assert_eq!(groups[0].running, 2);
        assert_eq!(groups[0].total(), 3);
    }

    #[test]
    fn matched_respects_filter_not_full_list() {
        let rows = vec![
            row("web-1", Some("alpha"), "running"),
            row("db-1", Some("alpha"), "running"),
        ];
        // simulate a text filter that only matched index 0
        let matched = vec![0];
        let groups = build_groups(&rows, &matched);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec![0]);
        assert_eq!(groups[0].total(), 1);
    }

    #[test]
    fn flatten_keeps_members_of_open_groups_in_group_order() {
        let rows = vec![
            row("web-1", Some("alpha"), "running"),
            row("web-2", Some("beta"), "running"),
        ];
        let matched = vec![0, 1];
        let groups = build_groups(&rows, &matched);
        let selectable = flatten_selectable(&groups, &HashSet::new());

        assert_eq!(selectable, vec![0, 1]);
    }

    #[test]
    fn flatten_drops_members_of_collapsed_groups() {
        let rows = vec![
            row("web-1", Some("alpha"), "running"),
            row("db-1", Some("alpha"), "running"),
            row("web-2", Some("beta"), "running"),
        ];
        let matched = vec![0, 1, 2];
        let groups = build_groups(&rows, &matched);
        let mut collapsed = HashSet::new();
        collapsed.insert("alpha".to_string());
        let selectable = flatten_selectable(&groups, &collapsed);

        // alpha's members (0, 1) are gone; beta's member (2) remains.
        assert_eq!(selectable, vec![2]);
    }

    #[test]
    fn flatten_never_collapses_the_ungrouped_bucket() {
        let rows = vec![row("standalone", None, "running")];
        let matched = vec![0];
        let groups = build_groups(&rows, &matched);
        let mut collapsed = HashSet::new();
        // an empty-string key can't exist for `None`, but guard against a bug
        // where collapsing "" would wrongly hide the ungrouped bucket.
        collapsed.insert(String::new());
        let selectable = flatten_selectable(&groups, &collapsed);

        assert_eq!(selectable, vec![0]);
    }
}
