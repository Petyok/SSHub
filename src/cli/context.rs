//! CLI bootstrap context — opens store/metadata without starting the TUI.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::app::{resolve_pending_secret_for_managed, HostEntry};
use crate::config::{self, AppConfig};
use crate::credentials::{OsKeyring, PasswordStore};
use crate::hosts::load_merged_hosts;
use crate::metadata::MetadataDb;
use crate::ssh::SshConfigResolver;
use crate::store::{HostGroup, Identity, LauncherStore, ManagedHost, Tunnel};

pub struct CliContext {
    pub config: AppConfig,
    pub store: Arc<LauncherStore>,
    pub metadata: Arc<MetadataDb>,
    pub resolver: SshConfigResolver,
    pub password_store: Box<dyn PasswordStore>,
    pub hosts: Vec<HostEntry>,
}

impl CliContext {
    pub fn bootstrap() -> Result<Self> {
        let config = config::load_config()?;
        let data_dir = config::data_dir()?;
        std::fs::create_dir_all(&data_dir)?;

        let metadata = Arc::new(MetadataDb::open(data_dir.join("metadata.db"))?);
        let store = Arc::new(LauncherStore::open(data_dir.join("launcher.db"))?);
        let resolver = SshConfigResolver::default();
        let password_store: Box<dyn PasswordStore> = Box::new(OsKeyring);

        let mut ctx = Self {
            config,
            store,
            metadata,
            resolver,
            password_store,
            hosts: Vec::new(),
        };
        ctx.reload_hosts()?;
        Ok(ctx)
    }

    pub fn reload_hosts(&mut self) -> Result<()> {
        self.hosts = load_merged_hosts(&self.resolver, &self.store, self.metadata.as_ref())?;
        Ok(())
    }

    pub fn host_by_name(&self, name: &str) -> Result<&HostEntry> {
        self.hosts
            .iter()
            .find(|h| h.name() == name)
            .with_context(|| format!("host '{name}' not found"))
    }

    pub fn managed_host_by_name(&self, name: &str) -> Result<ManagedHost> {
        match self.host_by_name(name)? {
            HostEntry::Managed(m) => Ok(m.clone()),
            HostEntry::Legacy { .. } => {
                anyhow::bail!("host '{name}' is not a managed launcher host")
            }
        }
    }

    pub fn group_by_name(&self, name: &str) -> Result<HostGroup> {
        self.store
            .list_groups()?
            .into_iter()
            .find(|g| g.name == name)
            .with_context(|| format!("group '{name}' not found"))
    }

    pub fn identity_by_name(&self, name: &str) -> Result<Identity> {
        self.store
            .list_identities()?
            .into_iter()
            .find(|i| i.name == name)
            .with_context(|| format!("identity '{name}' not found"))
    }

    /// Resolve a tunnel by numeric id, label, or local port. Ambiguity is an error.
    pub fn resolve_tunnel(&self, token: &str) -> Result<Tunnel> {
        if let Ok(id) = token.parse::<i64>() {
            return self
                .store
                .get_tunnel(id)?
                .with_context(|| format!("tunnel {id} not found"));
        }
        if let Ok(port) = token.parse::<u16>() {
            let matches = self.store.find_tunnels_by_local_port(port)?;
            return pick_tunnel(matches, "local-port");
        }
        let matches = self.store.find_tunnels_by_label(token)?;
        pick_tunnel(matches, "label")
    }

    pub fn resolve_tunnel_host(&self, tunnel: &Tunnel) -> Result<ManagedHost> {
        let host_id = tunnel
            .host_id
            .with_context(|| format!("tunnel {} has no host", tunnel.id))?;
        self.store
            .get_host(host_id)?
            .with_context(|| format!("host {host_id} not found for tunnel {}", tunnel.id))
    }

    pub fn resolve_tunnel_secret(
        &self,
        host: &ManagedHost,
    ) -> (Option<crate::session::PendingSecret>, String) {
        resolve_pending_secret_for_managed(host, self.password_store.as_ref())
    }
}

fn pick_tunnel(mut matches: Vec<Tunnel>, kind: &str) -> Result<Tunnel> {
    match matches.len() {
        0 => anyhow::bail!("no tunnel found for {kind}"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("ambiguous {kind}: {n} tunnels match"),
    }
}

/// Resolve a group name to its database id (used by `group add/edit --parent`).
pub fn resolve_parent_id(ctx: &CliContext, name: &str) -> Result<i64> {
    ctx.group_by_name(name).map(|g| g.id)
}

/// Resolve an identity name to its database id (used by `group … --default-identity`).
pub fn resolve_identity_id(ctx: &CliContext, name: &str) -> Result<i64> {
    ctx.identity_by_name(name).map(|i| i.id)
}

/// Normalize a `--*-key`/`--certificate` path flag value: trim surrounding
/// whitespace, expand a leading `~`, and return `None` for an empty value so
/// callers can distinguish "not provided" from "set to blank".
pub fn optional_path_flag(raw: &str) -> Result<Option<PathBuf>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::ssh::expand_tilde(trimmed)))
}

/// Read a secret from stdin (for `--password-stdin`), stripping the trailing
/// newline so a piped `echo secret` does not store the newline.
pub fn read_password_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("read password from stdin")?;
    Ok(buf.trim_end_matches(['\r', '\n']).to_string())
}
