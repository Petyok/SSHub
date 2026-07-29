//! Host list filtering and sorting for CLI.

use crate::app::{sort_host_indices, HostEntry, SortMode};
use crate::store::{HostGroup, LauncherStore};

/// Tag AND filter (same semantics as TUI [`rebuild_filter`](crate::app::hostlist::App::rebuild_filter)).
pub fn filter_by_tags(hosts: &[HostEntry], tags: &[String]) -> Vec<usize> {
    if tags.is_empty() {
        return (0..hosts.len()).collect();
    }
    hosts
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            let host_tags = entry.tags();
            tags.iter().all(|f| host_tags.iter().any(|t| t == f))
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// Exact group membership by group name (not subtree).
pub fn filter_by_group_name(
    hosts: &[HostEntry],
    groups: &[HostGroup],
    group_name: &str,
) -> Vec<usize> {
    let Some(group) = groups.iter().find(|g| g.name == group_name) else {
        return Vec::new();
    };
    hosts
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.group_ids().contains(&group.id))
        .map(|(idx, _)| idx)
        .collect()
}

pub fn apply_filters(
    hosts: &[HostEntry],
    store: &LauncherStore,
    tags: &[String],
    group: Option<&str>,
    sort: SortMode,
) -> anyhow::Result<Vec<usize>> {
    let mut indices = filter_by_tags(hosts, tags);
    if let Some(gname) = group {
        let groups = store.list_groups()?;
        let group_indices = filter_by_group_name(hosts, &groups, gname);
        indices.retain(|i| group_indices.contains(i));
    }
    sort_host_indices(hosts, &mut indices, sort);
    Ok(indices)
}

pub fn all_group_names(
    store: &LauncherStore,
    include_reserved: bool,
) -> anyhow::Result<Vec<String>> {
    Ok(store
        .list_groups()?
        .into_iter()
        .filter(|g| include_reserved || !g.reserved)
        .map(|g| g.name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::HostMetadata;
    use crate::ssh::SshHost;
    use crate::store::{HostUpdate, LauncherStore, NewHost, NewHostGroup};

    /// Build a `Legacy` host entry with the given name and tags.
    fn legacy(name: &str, tags: &[&str]) -> HostEntry {
        HostEntry::Legacy {
            host: SshHost {
                name: name.into(),
                ..Default::default()
            },
            meta: HostMetadata {
                host_name: name.into(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn tag_and_filter() {
        let hosts = vec![
            HostEntry::Legacy {
                host: SshHost {
                    name: "a".into(),
                    ..Default::default()
                },
                meta: HostMetadata {
                    host_name: "a".into(),
                    tags: vec!["web".into(), "prod".into()],
                    ..Default::default()
                },
            },
            HostEntry::Legacy {
                host: SshHost {
                    name: "b".into(),
                    ..Default::default()
                },
                meta: HostMetadata {
                    host_name: "b".into(),
                    tags: vec!["web".into()],
                    ..Default::default()
                },
            },
        ];
        let idx = filter_by_tags(&hosts, &["web".into(), "prod".into()]);
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn group_name_filter() {
        let store = LauncherStore::open_in_memory().unwrap();
        let g = store
            .create_group(&crate::store::NewHostGroup {
                name: "prod".into(),
                ..Default::default()
            })
            .unwrap();
        let host = store
            .create_host(&crate::store::NewHost::launcher("h1", "1.2.3.4"))
            .unwrap();
        store.set_host_groups(host.id, &[g.id]).unwrap();
        let managed = store.get_host(host.id).unwrap().unwrap();
        let hosts = vec![HostEntry::from_managed(managed)];
        let groups = store.list_groups().unwrap();
        let idx = filter_by_group_name(&hosts, &groups, "prod");
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn tag_filter_empty_returns_all() {
        let hosts = vec![legacy("a", &["web"]), legacy("b", &[])];
        // No requested tags: every host passes, order preserved.
        assert_eq!(filter_by_tags(&hosts, &[]), vec![0, 1]);
    }

    #[test]
    fn tag_filter_single_matches_every_host_with_that_tag() {
        let hosts = vec![
            legacy("a", &["web", "prod"]),
            legacy("b", &["db"]),
            legacy("c", &["web"]),
        ];
        // A single requested tag returns all hosts carrying it.
        assert_eq!(filter_by_tags(&hosts, &["web".into()]), vec![0, 2]);
    }

    #[test]
    fn tag_filter_requires_all_tags_not_any() {
        let hosts = vec![
            // Has one of the two requested tags: must be excluded (AND, not OR).
            legacy("a", &["web"]),
            // Has both requested tags plus an extra: included.
            legacy("b", &["web", "prod", "eu"]),
            // Has neither: excluded.
            legacy("c", &["db"]),
        ];
        let idx = filter_by_tags(&hosts, &["web".into(), "prod".into()]);
        assert_eq!(idx, vec![1]);
    }

    #[test]
    fn group_filter_is_exact_membership_not_subtree() {
        let store = LauncherStore::open_in_memory().unwrap();
        let parent = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();
        let child = store
            .create_group(&NewHostGroup {
                name: "eu".into(),
                sort_order: 1,
                parent_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        // Host lives only in the child group.
        let host = store
            .create_host(&NewHost {
                name: "e1".into(),
                address: "10.0.0.1".into(),
                group_id: Some(child.id),
                ..Default::default()
            })
            .unwrap();
        store.set_host_groups(host.id, &[child.id]).unwrap();
        let hosts = vec![HostEntry::from_managed(
            store.get_host(host.id).unwrap().unwrap(),
        )];
        let groups = store.list_groups().unwrap();

        // Filtering by the child group name returns the host.
        assert_eq!(filter_by_group_name(&hosts, &groups, "eu"), vec![0]);
        // Filtering by the parent group name does NOT (no subtree matching).
        assert!(filter_by_group_name(&hosts, &groups, "prod").is_empty());
    }

    #[test]
    fn group_filter_unknown_name_returns_empty() {
        let store = LauncherStore::open_in_memory().unwrap();
        let hosts: Vec<HostEntry> = Vec::new();
        let groups = store.list_groups().unwrap();
        assert!(filter_by_group_name(&hosts, &groups, "nope").is_empty());
    }

    #[test]
    fn sort_label_is_case_insensitive_alphabetical() {
        let hosts = vec![
            legacy("banana", &[]),
            legacy("Apple", &[]),
            legacy("cherry", &[]),
        ];
        let mut idx: Vec<usize> = (0..hosts.len()).collect();
        sort_host_indices(&hosts, &mut idx, SortMode::Label);
        // Apple, banana, cherry (case-folded).
        assert_eq!(idx, vec![1, 0, 2]);
    }

    #[test]
    fn sort_manual_puts_legacy_hosts_at_tail() {
        let store = LauncherStore::open_in_memory().unwrap();
        let host = store
            .create_host(&NewHost::launcher("managed", "1.2.3.4"))
            .unwrap();
        // Give the managed host a small, finite sort_order.
        store
            .update_host(
                host.id,
                &HostUpdate {
                    sort_order: Some(5),
                    ..Default::default()
                },
            )
            .unwrap();
        let managed = HostEntry::from_managed(store.get_host(host.id).unwrap().unwrap());
        assert_eq!(managed.sort_order(), 5);

        // Index 0 is legacy (sort_order == i32::MAX), index 1 is managed.
        let hosts = vec![legacy("zzz-legacy", &[]), managed];
        let mut idx: Vec<usize> = (0..hosts.len()).collect();
        sort_host_indices(&hosts, &mut idx, SortMode::Manual);
        // Managed (finite order) sorts ahead of the legacy host at i32::MAX.
        assert_eq!(idx, vec![1, 0]);
    }

    #[test]
    fn sort_group_uses_primary_group_order_not_label() {
        let store = LauncherStore::open_in_memory().unwrap();
        // "zeta" has a lower group sort_order than "alpha", so hosts in "zeta"
        // must sort first even though "alpha" < "zeta" alphabetically.
        let zeta = store
            .create_group(&NewHostGroup {
                name: "zeta".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();
        let alpha = store
            .create_group(&NewHostGroup {
                name: "alpha".into(),
                sort_order: 1,
                ..Default::default()
            })
            .unwrap();
        let in_alpha = store
            .create_host(&NewHost {
                name: "a-host".into(),
                address: "10.0.0.1".into(),
                group_id: Some(alpha.id),
                ..Default::default()
            })
            .unwrap();
        let in_zeta = store
            .create_host(&NewHost {
                name: "z-host".into(),
                address: "10.0.0.2".into(),
                group_id: Some(zeta.id),
                ..Default::default()
            })
            .unwrap();

        // Index 0 is the alpha-group host, index 1 is the zeta-group host.
        let hosts = vec![
            HostEntry::from_managed(store.get_host(in_alpha.id).unwrap().unwrap()),
            HostEntry::from_managed(store.get_host(in_zeta.id).unwrap().unwrap()),
        ];
        let mut idx: Vec<usize> = (0..hosts.len()).collect();
        sort_host_indices(&hosts, &mut idx, SortMode::GroupThenLabel);
        // zeta-group host (group sort_order 0) comes before alpha-group host.
        assert_eq!(idx, vec![1, 0]);
    }
}
