use std::collections::HashSet;

use anyhow::Result;

use crate::app::HostEntry;
use crate::metadata::MetadataStore;
use crate::ssh::{sync_ssh_config_hosts, HostResolver};
use crate::store::{HostSource, LauncherStore};

/// Load the merged host list (launcher + ssh_config DB rows + legacy aliases).
///
/// Mirrors the host-loading portion of [`crate::app::App::reload_hosts`]:
/// sync ssh_config rows, merge DB hosts, then append unresolved legacy aliases
/// with metadata defaults applied.
pub fn load_merged_hosts(
    resolver: &dyn HostResolver,
    store: &LauncherStore,
    metadata: &dyn MetadataStore,
) -> Result<Vec<HostEntry>> {
    sync_ssh_config_hosts(resolver, store)?;

    let launcher_hosts = store.list_hosts_filtered(Some(HostSource::Launcher))?;
    let ssh_config_hosts = store.list_hosts_filtered(Some(HostSource::SshConfig))?;
    let db_names: HashSet<String> = launcher_hosts
        .iter()
        .chain(ssh_config_hosts.iter())
        .map(|h| h.name.clone())
        .collect();

    let mut hosts: Vec<HostEntry> = launcher_hosts
        .into_iter()
        .chain(ssh_config_hosts)
        .map(HostEntry::from_managed)
        .collect();

    let config_names = resolver.list_hosts()?;
    metadata.ensure_defaults(&config_names)?;

    for name in config_names {
        if db_names.contains(&name) {
            continue;
        }
        let host = match resolver.resolve_host(&name) {
            Ok(host) => host,
            Err(_) => continue,
        };
        let meta = metadata
            .get(&name)?
            .unwrap_or_else(|| crate::metadata::HostMetadata::new(&name));
        hosts.push(HostEntry::Legacy { host, meta });
    }

    Ok(hosts)
}
