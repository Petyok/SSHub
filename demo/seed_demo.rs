//! Populate `demo/home/.local/share/sshub/launcher.db` for VHS recordings.
//! Run via `demo/seed-demo.sh`.
//!
//! Hosts are created directly (not imported from ssh_config) so they can have
//! playful display names. Their addresses are real, pingable public anycast
//! resolvers (Google / Cloudflare / Quad9 / OpenDNS / Level3) so the demo shows
//! hosts as *online* — the actual connect is simulated by `demo/bin/ssh`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use sshub::store::{HostUpdate, LauncherStore, NewHost, NewHostGroup};

fn main() -> Result<()> {
    let data_dir = std::env::var("SSHUB_DATA_DIR").context("SSHUB_DATA_DIR must be set")?;

    // Wipe every host source so a re-seed is deterministic: the launcher DB and
    // the legacy metadata DB that migrates into it on open.
    let dir = PathBuf::from(&data_dir);
    for base in ["launcher.db", "metadata.db"] {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("{base}{suffix}")));
        }
    }

    let store = LauncherStore::open(dir.join("launcher.db"))?;

    let production = mk_group(&store, "Production", 0, None)?;
    let web = mk_group(&store, "Web", 0, Some(production))?;
    let databases = mk_group(&store, "Databases", 1, Some(production))?;
    let staging = mk_group(&store, "Staging", 1, None)?;

    // (display name, address, user, port, group, tags, favorite, proxy_jump)
    let hosts: &[(
        &str,
        &str,
        &str,
        u16,
        Option<i64>,
        &[&str],
        bool,
        Option<&str>,
    )] = &[
        (
            "Real Google DNS (trust me)",
            "8.8.8.8",
            "deploy",
            22,
            Some(web),
            &["prod", "web"],
            true,
            None,
        ),
        (
            "Google DNS: the sequel",
            "8.8.4.4",
            "deploy",
            22,
            Some(web),
            &["prod", "web"],
            false,
            None,
        ),
        (
            "Cloudflare",
            "1.1.1.1",
            "postgres",
            5432,
            Some(databases),
            &["prod", "db"],
            false,
            None,
        ),
        (
            "Cloudflare's server in a mom's garage",
            "1.0.0.1",
            "postgres",
            5432,
            Some(databases),
            &["prod", "db"],
            false,
            None,
        ),
        (
            "Quad9's server in a bunker",
            "9.9.9.9",
            "ubuntu",
            22,
            Some(staging),
            &["staging"],
            false,
            None,
        ),
        (
            "CI runner (OpenDNS)",
            "208.67.222.222",
            "runner",
            22,
            None,
            &["staging", "ci"],
            false,
            Some("Bastion (secretly Level3)"),
        ),
        (
            "Bastion (secretly Level3)",
            "4.2.2.2",
            "jump",
            22,
            None,
            &["prod", "ops"],
            false,
            None,
        ),
    ];

    for (name, addr, user, port, group_id, tags, favorite, proxy) in hosts {
        let host = store.create_host(&NewHost {
            name: (*name).to_string(),
            address: (*addr).to_string(),
            username: Some((*user).to_string()),
            port: *port,
            group_id: *group_id,
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            proxy_jump: proxy.map(|p| p.to_string()),
            ..Default::default()
        })?;
        if *favorite {
            store.update_host(
                host.id,
                &HostUpdate {
                    favorite: Some(true),
                    ..Default::default()
                },
            )?;
        }
    }

    eprintln!("seeded {} hosts", hosts.len());
    Ok(())
}

fn mk_group(
    store: &LauncherStore,
    name: &str,
    sort_order: i32,
    parent_id: Option<i64>,
) -> Result<i64> {
    Ok(store
        .create_group(&NewHostGroup {
            name: name.to_string(),
            sort_order,
            parent_id,
            ..Default::default()
        })?
        .id)
}
