//! Populate `demo/home/.local/share/sshub/launcher.db` for VHS recordings.
//! Run via `demo/seed-demo.sh`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use sshub::metadata::MetadataDb;
use sshub::ssh::{import_ssh_config, SshConfigResolver};
use sshub::store::{HostUpdate, LauncherStore, NewHostGroup};

fn main() -> Result<()> {
    let data_dir = std::env::var("SSHUB_DATA_DIR").context("SSHUB_DATA_DIR must be set")?;
    let ssh_config = std::env::var("SSHUB_SSH_CONFIG").context("SSHUB_SSH_CONFIG must be set")?;

    let db_path = PathBuf::from(&data_dir).join("launcher.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
        let _ = std::fs::remove_file(sidecar);
    }

    let store = LauncherStore::open(&db_path)?;
    let resolver = SshConfigResolver::with_config_path(&ssh_config);
    let metadata = MetadataDb::default();
    import_ssh_config(&resolver, &store, &metadata)?;

    let production = store.create_group(&NewHostGroup {
        name: "Production".into(),
        sort_order: 0,
        ..Default::default()
    })?;
    let web = store.create_group(&NewHostGroup {
        name: "Web".into(),
        sort_order: 0,
        parent_id: Some(production.id),
        ..Default::default()
    })?;
    let databases = store.create_group(&NewHostGroup {
        name: "Databases".into(),
        sort_order: 1,
        parent_id: Some(production.id),
        ..Default::default()
    })?;
    let staging = store.create_group(&NewHostGroup {
        name: "Staging".into(),
        sort_order: 1,
        ..Default::default()
    })?;

    tag_group(
        &store,
        "web-prod-01",
        &["prod", "web"],
        Some(web.id),
        true,
    )?;
    tag_group(&store, "web-prod-02", &["prod", "web"], Some(web.id), false)?;
    tag_group(
        &store,
        "db-primary",
        &["prod", "db"],
        Some(databases.id),
        false,
    )?;
    tag_group(
        &store,
        "db-replica",
        &["prod", "db"],
        Some(databases.id),
        false,
    )?;
    tag_group(
        &store,
        "staging-app",
        &["staging"],
        Some(staging.id),
        false,
    )?;
    tag_group(&store, "bastion", &["prod", "ops"], None, false)?;
    tag_group(&store, "ci-runner", &["staging", "ci"], None, false)?;

    eprintln!("seeded {}", db_path.display());
    Ok(())
}

fn tag_group(
    store: &LauncherStore,
    name: &str,
    tags: &[&str],
    group_id: Option<i64>,
    favorite: bool,
) -> Result<()> {
    let host = store
        .get_host_by_name(name)?
        .with_context(|| format!("host {name} missing after import"))?;
    store.update_host(
        host.id,
        &HostUpdate {
            tags: Some(tags.iter().map(|t| (*t).to_string()).collect()),
            group_id: Some(group_id),
            favorite: Some(favorite),
            ..Default::default()
        },
    )?;
    Ok(())
}
