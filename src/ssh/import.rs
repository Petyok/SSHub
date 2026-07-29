use anyhow::Result;

use crate::metadata::MetadataStore;
use crate::session_transport::SessionTransport;
use crate::ssh::{HostResolver, SshHost};
use crate::store::{LauncherStore, SshConfigHostImport, UpsertSshConfigOutcome};

/// Summary of an ssh config import run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportReport {
    pub inserted: usize,
    pub updated: usize,
    pub skipped_launcher: usize,
    pub failed: usize,
}

/// Import hosts from ssh config via resolver into the launcher store.
///
/// Existing launcher rows are never overwritten. Imported rows use `source=ssh_config`
/// and merge metadata from the legacy metadata store when present.
/// Re-sync `source=ssh_config` rows when resolver output changes (hash mismatch).
///
/// Preserves user metadata on the row (tags, notes, favorite) via [`LauncherStore::upsert_ssh_config_host`].
pub fn sync_ssh_config_hosts(resolver: &dyn HostResolver, store: &LauncherStore) -> Result<usize> {
    let mut updated = 0usize;
    let rows = store.list_hosts_filtered(Some(crate::store::HostSource::SshConfig))?;

    for existing in rows {
        let resolved = match resolver.resolve_host(&existing.name) {
            Ok(host) => host,
            Err(_) => {
                // Host no longer resolvable; leave the existing row untouched.
                // Runs during a TUI reload — no stderr (would corrupt the UI).
                continue;
            }
        };

        let new_hash = compute_ssh_config_hash(&resolved);
        if existing.ssh_config_hash.as_deref() == Some(new_hash.as_str()) {
            continue;
        }

        let import = build_sync_import(&existing, &resolved);
        if store.upsert_ssh_config_host(&import)? == UpsertSshConfigOutcome::Updated {
            updated += 1;
        }
    }

    Ok(updated)
}

pub fn import_ssh_config(
    resolver: &dyn HostResolver,
    store: &LauncherStore,
    metadata: &dyn MetadataStore,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let names = resolver.list_hosts()?;
    metadata.ensure_defaults(&names)?;

    for name in names {
        let resolved = match resolver.resolve_host(&name) {
            Ok(host) => host,
            Err(_) => {
                // Counted in report.failed (surfaced to the user by the import
                // handler). No stderr — import runs under raw mode.
                report.failed += 1;
                continue;
            }
        };

        let import = build_import_row(&name, &resolved, metadata)?;
        match store.upsert_ssh_config_host(&import)? {
            UpsertSshConfigOutcome::Inserted => report.inserted += 1,
            UpsertSshConfigOutcome::Updated => report.updated += 1,
            UpsertSshConfigOutcome::SkippedLauncher => report.skipped_launcher += 1,
        }
    }

    Ok(report)
}

fn build_sync_import(
    existing: &crate::store::ManagedHost,
    resolved: &SshHost,
) -> SshConfigHostImport {
    let address = resolved
        .hostname
        .clone()
        .unwrap_or_else(|| existing.name.clone());
    SshConfigHostImport {
        name: existing.name.clone(),
        address,
        port: resolved.port.unwrap_or(22),
        proxy_jump: resolved.proxy_jump.clone(),
        forward_agent: resolved.forward_agent.unwrap_or(false),
        remote_command: resolved.remote_command.clone(),
        ssh_config_hash: compute_ssh_config_hash(resolved),
        tags: existing.tags.clone(),
        notes: existing.notes.clone(),
        environment: existing.environment.clone(),
        favorite: existing.favorite,
        last_connected: existing.last_connected,
        session_logging: existing.session_logging,
        transport: existing.transport,
    }
}

/// Materialize a single live ssh_config alias into launcher.db as a
/// `source=ssh_config` row, so it gains a metadata overlay (group, identity,
/// tags, notes) that a live-resolver-only alias cannot store.
///
/// Returns `Ok(true)` if the host now exists as an ssh_config row, `Ok(false)`
/// if it could not be resolved or a launcher row already owns the name.
pub fn materialize_ssh_config_host(
    resolver: &dyn HostResolver,
    store: &LauncherStore,
    metadata: &dyn MetadataStore,
    name: &str,
) -> Result<bool> {
    let resolved = match resolver.resolve_host(name) {
        Ok(host) => host,
        Err(_) => return Ok(false),
    };
    let import = build_import_row(name, &resolved, metadata)?;
    Ok(store.upsert_ssh_config_host(&import)? != UpsertSshConfigOutcome::SkippedLauncher)
}

fn build_import_row(
    name: &str,
    resolved: &SshHost,
    metadata: &dyn MetadataStore,
) -> Result<SshConfigHostImport> {
    let address = resolved
        .hostname
        .clone()
        .unwrap_or_else(|| name.to_string());
    let port = resolved.port.unwrap_or(22);

    let meta = metadata.get(name)?;
    let (tags, notes, environment, favorite, last_connected, session_logging, transport) =
        match meta {
            Some(m) => (
                m.tags,
                m.description,
                m.environment,
                m.favorite,
                m.last_connected,
                m.session_logging,
                m.transport,
            ),
            None => (
                Vec::new(),
                None,
                None,
                false,
                None,
                crate::session_log::SessionLoggingOverride::Inherit,
                SessionTransport::Ssh,
            ),
        };

    Ok(SshConfigHostImport {
        name: name.to_string(),
        address,
        port,
        proxy_jump: resolved.proxy_jump.clone(),
        forward_agent: resolved.forward_agent.unwrap_or(false),
        remote_command: resolved.remote_command.clone(),
        ssh_config_hash: compute_ssh_config_hash(resolved),
        tags,
        notes,
        environment,
        favorite,
        last_connected,
        session_logging,
        transport,
    })
}

/// Deterministic fingerprint of resolved ssh config fields for change detection.
pub fn compute_ssh_config_hash(host: &SshHost) -> String {
    format!(
        "hostname={}|user={}|port={}|proxy={}|identity={}|fwd={}|cmd={}",
        host.hostname.as_deref().unwrap_or(""),
        host.user.as_deref().unwrap_or(""),
        host.port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "22".into()),
        host.proxy_jump.as_deref().unwrap_or(""),
        host.identity_file.as_deref().unwrap_or(""),
        match host.forward_agent {
            Some(true) => "yes",
            Some(false) => "no",
            None => "",
        },
        host.remote_command.as_deref().unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::MetadataDb;
    use crate::ssh::SshHost;
    use crate::store::{HostSource, LauncherStore, NewHost};
    use std::collections::HashMap;
    use std::sync::Arc;

    struct MapResolver {
        hosts: HashMap<String, SshHost>,
        order: Vec<String>,
    }

    impl HostResolver for MapResolver {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(self.order.clone())
        }

        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            self.hosts
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown {name}"))
        }
    }

    fn host(name: &str, hostname: &str) -> SshHost {
        let mut h = SshHost::new(name);
        h.hostname = Some(hostname.to_string());
        h.port = Some(22);
        h
    }

    #[test]
    fn compute_ssh_config_hash_changes_with_forward_agent_and_remote_command() {
        let base = host("web", "1.2.3.4");

        let mut with_agent = host("web", "1.2.3.4");
        with_agent.forward_agent = Some(true);
        assert_ne!(
            compute_ssh_config_hash(&base),
            compute_ssh_config_hash(&with_agent)
        );

        let mut with_agent_false = host("web", "1.2.3.4");
        with_agent_false.forward_agent = Some(false);
        assert_ne!(
            compute_ssh_config_hash(&with_agent),
            compute_ssh_config_hash(&with_agent_false)
        );

        let mut with_cmd = host("web", "1.2.3.4");
        with_cmd.remote_command = Some("tmux attach".into());
        assert_ne!(
            compute_ssh_config_hash(&base),
            compute_ssh_config_hash(&with_cmd)
        );

        let mut with_cmd2 = host("web", "1.2.3.4");
        with_cmd2.remote_command = Some("screen".into());
        assert_ne!(
            compute_ssh_config_hash(&with_cmd),
            compute_ssh_config_hash(&with_cmd2)
        );
    }

    #[test]
    fn import_inserts_ssh_config_hosts() {
        let store = LauncherStore::open_in_memory().unwrap();
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let resolver = MapResolver {
            hosts: HashMap::from([
                ("alpha".into(), host("alpha", "10.0.0.1")),
                ("beta".into(), host("beta", "10.0.0.2")),
            ]),
            order: vec!["alpha".into(), "beta".into()],
        };

        let report = import_ssh_config(&resolver, &store, metadata.as_ref()).unwrap();
        assert_eq!(report.inserted, 2);
        assert_eq!(report.skipped_launcher, 0);

        let listed = store
            .list_hosts_filtered(Some(HostSource::SshConfig))
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].address, "10.0.0.1");
    }

    #[test]
    fn import_skips_launcher_name_collision() {
        let store = LauncherStore::open_in_memory().unwrap();
        let default_id = store.get_identity_by_name("Default").unwrap().unwrap().id;
        store
            .create_host(&NewHost {
                name: "alpha".into(),
                label: None,
                address: "192.168.0.1".into(),
                port: 22,
                group_id: None,
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();

        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let resolver = MapResolver {
            hosts: HashMap::from([("alpha".into(), host("alpha", "10.0.0.1"))]),
            order: vec!["alpha".into()],
        };

        let report = import_ssh_config(&resolver, &store, metadata.as_ref()).unwrap();
        assert_eq!(report.skipped_launcher, 1);
        assert_eq!(report.inserted, 0);

        let launcher = store.get_host_by_name("alpha").unwrap().unwrap();
        assert_eq!(launcher.address, "192.168.0.1");
        assert_eq!(launcher.source, HostSource::Launcher);
    }

    #[test]
    fn sync_updates_connection_fields_preserves_db_metadata() {
        let store = LauncherStore::open_in_memory().unwrap();
        store
            .upsert_ssh_config_host(&SshConfigHostImport {
                name: "web".into(),
                address: "1.2.3.4".into(),
                port: 22,
                ssh_config_hash: "hash-v1".into(),
                ..Default::default()
            })
            .unwrap();
        let id = store.get_host_by_name("web").unwrap().unwrap().id;
        store
            .update_host(
                id,
                &crate::store::HostUpdate {
                    tags: Some(vec!["keep".into()]),
                    notes: Some(Some("note".into())),
                    favorite: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();

        let resolver = MapResolver {
            hosts: HashMap::from([("web".into(), {
                let mut h = host("web", "9.9.9.9");
                h.port = Some(2222);
                h
            })]),
            order: vec!["web".into()],
        };

        let updated = sync_ssh_config_hosts(&resolver, &store).unwrap();
        assert_eq!(updated, 1);

        let host = store.get_host_by_name("web").unwrap().unwrap();
        assert_eq!(host.address, "9.9.9.9");
        assert_eq!(host.port, 2222);
        assert_eq!(host.tags, vec!["keep"]);
        assert_eq!(host.notes.as_deref(), Some("note"));
        assert!(host.favorite);
        assert_ne!(host.ssh_config_hash.as_deref(), Some("hash-v1"));
    }

    #[test]
    fn sync_skips_when_hash_unchanged() {
        let store = LauncherStore::open_in_memory().unwrap();
        let resolver = MapResolver {
            hosts: HashMap::from([("web".into(), host("web", "1.2.3.4"))]),
            order: vec!["web".into()],
        };
        import_ssh_config(&resolver, &store, &MetadataDb::default()).unwrap();
        let before = store.get_host_by_name("web").unwrap().unwrap().updated_at;

        assert_eq!(sync_ssh_config_hosts(&resolver, &store).unwrap(), 0);
        assert_eq!(
            store.get_host_by_name("web").unwrap().unwrap().updated_at,
            before
        );
    }

    #[test]
    fn import_merges_metadata_overlay() {
        let store = LauncherStore::open_in_memory().unwrap();
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        metadata
            .upsert(&crate::metadata::HostMetadata {
                host_name: "alpha".into(),
                tags: vec!["prod".into()],
                description: Some("Primary".into()),
                environment: None,
                favorite: true,
                last_connected: Some(42),
                session_logging: crate::session_log::SessionLoggingOverride::On,
                transport: SessionTransport::Ssh,
            })
            .unwrap();

        let resolver = MapResolver {
            hosts: HashMap::from([("alpha".into(), host("alpha", "10.0.0.1"))]),
            order: vec!["alpha".into()],
        };

        import_ssh_config(&resolver, &store, metadata.as_ref()).unwrap();
        let imported = store.get_host_by_name("alpha").unwrap().unwrap();
        assert_eq!(imported.tags, vec!["prod"]);
        assert_eq!(imported.notes.as_deref(), Some("Primary"));
        assert!(imported.favorite);
        assert_eq!(imported.last_connected, Some(42));
        assert_eq!(
            imported.session_logging,
            crate::session_log::SessionLoggingOverride::On
        );
    }
}
