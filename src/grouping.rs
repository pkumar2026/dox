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

/// One line of the on-screen container list: either a group's header or one
/// of its (or the ungrouped bucket's) member rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderRow<'a> {
    Header(&'a Group),
    /// `view_idx` is the row's position in the selectable list
    /// (`flatten_selectable`'s output) — what `TableState::select` expects.
    /// `container_idx` is the index into the source `containers` slice.
    Container {
        view_idx: usize,
        container_idx: usize,
    },
}

/// Expand groups into the exact sequence of lines the list draws, in order.
/// This is the single source of truth for "what is on screen at row N" —
/// both the draw call and mouse hit-testing walk this same list, so a click
/// always lands on what's actually visible.
pub fn render_rows<'a>(groups: &'a [Group], collapsed: &HashSet<String>) -> Vec<RenderRow<'a>> {
    let mut out = Vec::new();
    let mut view_idx = 0;
    for group in groups {
        let is_collapsed = group
            .project
            .as_deref()
            .is_some_and(|p| collapsed.contains(p));
        if group.project.is_some() {
            out.push(RenderRow::Header(group));
        }
        if is_collapsed {
            continue;
        }
        for &container_idx in &group.members {
            out.push(RenderRow::Container {
                view_idx,
                container_idx,
            });
            view_idx += 1;
        }
    }
    out
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
            ports: Vec::new(),
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

    #[test]
    fn render_rows_interleaves_headers_with_members_in_view_order() {
        let rows = vec![
            row("web-1", Some("alpha"), "running"),
            row("db-1", Some("alpha"), "running"),
            row("standalone", None, "running"),
        ];
        let matched = vec![0, 1, 2];
        let groups = build_groups(&rows, &matched);
        let rendered = render_rows(&groups, &HashSet::new());

        assert_eq!(
            rendered,
            vec![
                RenderRow::Header(&groups[0]),
                RenderRow::Container {
                    view_idx: 0,
                    container_idx: 1
                }, // db-1 sorts before web-1
                RenderRow::Container {
                    view_idx: 1,
                    container_idx: 0
                },
                RenderRow::Container {
                    view_idx: 2,
                    container_idx: 2
                }, // ungrouped bucket: no header
            ]
        );
    }

    #[test]
    fn render_rows_skips_members_of_a_collapsed_group_but_keeps_its_header() {
        let rows = vec![
            row("web-1", Some("alpha"), "running"),
            row("web-2", Some("beta"), "running"),
        ];
        let matched = vec![0, 1];
        let groups = build_groups(&rows, &matched);
        let mut collapsed = HashSet::new();
        collapsed.insert("alpha".to_string());
        let rendered = render_rows(&groups, &collapsed);

        // alpha's header still shows (collapsed indicator lives in draw code)
        // but its member is gone; beta is untouched and starts at view_idx 0
        // since alpha contributed no selectable rows.
        assert_eq!(
            rendered,
            vec![
                RenderRow::Header(&groups[0]),
                RenderRow::Header(&groups[1]),
                RenderRow::Container {
                    view_idx: 0,
                    container_idx: 1
                },
            ]
        );
    }
}
