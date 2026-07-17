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
    use crate::store::LauncherStore;

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
}
