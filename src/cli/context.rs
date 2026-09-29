//! CLI bootstrap context — opens store/metadata without starting the TUI.

use std::cell::OnceCell;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::app::{resolve_pending_secret_for_managed, HostEntry};
use crate::config::AppConfig;
use crate::credentials::{
    check_keyring_available, migrate_fallback_to_keyring, FilePasswordStore,
    NamespacedPasswordStore, OsKeyring, PasswordStore,
};
use crate::hosts::load_merged_hosts;
use crate::metadata::MetadataDb;
use crate::profile::ProfilePaths;
use crate::ssh::SshConfigResolver;
use crate::store::{HostGroup, Identity, LauncherStore, ManagedHost, Tunnel};

pub struct CliContext {
    pub config: AppConfig,
    pub store: Arc<LauncherStore>,
    pub metadata: Arc<MetadataDb>,
    pub resolver: SshConfigResolver,
    /// Opened on first use, see [`CliContext::password_store`].
    pub(crate) password_store: OnceCell<Box<dyn PasswordStore>>,
    pub hosts: Vec<HostEntry>,
    /// Resolved profile workspace for this invocation.
    pub profile: ProfilePaths,
}

impl CliContext {
    /// Bootstrap against an explicitly resolved profile.
    pub fn bootstrap_with(paths: ProfilePaths) -> Result<Self> {
        let config = crate::config::load_config_at(&paths.config_file)?;
        std::fs::create_dir_all(&paths.root)?;

        let metadata = Arc::new(MetadataDb::open(paths.metadata_db())?);
        let store = Arc::new(LauncherStore::open(paths.launcher_db())?);
        let resolver = SshConfigResolver::with_config_path(paths.ssh_config.clone());

        let mut ctx = Self {
            config,
            store,
            metadata,
            resolver,
            password_store: OnceCell::new(),
            hosts: Vec::new(),
            profile: paths,
        };
        ctx.reload_hosts()?;
        Ok(ctx)
    }

    /// The credential store (OS keyring, or the `credentials.json` fallback),
    /// opened the first time a command reads or writes a secret.
    ///
    /// Opening it runs a write probe against the keyring, and on a locked
    /// Secret Service collection that probe waits for the unlock prompt. When
    /// the prompt cannot be shown it waits forever, so commands that never
    /// touch a secret (`host list`, `completions`, ...) must not open it.
    pub fn password_store(&self) -> &dyn PasswordStore {
        self.password_store
            .get_or_init(|| open_password_store(&self.profile))
            .as_ref()
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
        resolve_pending_secret_for_managed(host, self.password_store())
    }
}

fn open_password_store(paths: &ProfilePaths) -> Box<dyn PasswordStore> {
    let prefix = paths.credential_prefix();
    if check_keyring_available() {
        let _ = migrate_fallback_to_keyring(&paths.credentials_file());
        Box::new(NamespacedPasswordStore::new(Box::new(OsKeyring), prefix))
    } else {
        Box::new(NamespacedPasswordStore::new(
            Box::new(FilePasswordStore::new(paths.credentials_file())),
            prefix,
        ))
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A context bootstrapped the way `main` does it, over a throwaway
    /// profile directory whose launcher database holds `hosts`.
    pub(crate) fn bootstrapped(dir: &std::path::Path, hosts: &[&str]) -> CliContext {
        let root = dir.join("profile");
        std::fs::create_dir_all(&root).unwrap();
        let paths = ProfilePaths {
            data_root: dir.to_path_buf(),
            id: "test".into(),
            name: "test".into(),
            config_file: root.join("config.toml"),
            ssh_config: dir.join("ssh_config"),
            root,
            compat: false,
        };
        let store = LauncherStore::open(paths.launcher_db()).unwrap();
        for name in hosts {
            store
                .create_host(&crate::store::NewHost::launcher(*name, "10.0.0.1"))
                .unwrap();
        }
        drop(store);
        CliContext::bootstrap_with(paths).unwrap()
    }

    /// #135: opening the store probes the OS keyring, which blocks forever on
    /// a locked collection whose unlock prompt cannot open. `host list` and
    /// `completions` (sourced from the shell rc) read no secret, so they must
    /// finish without ever opening it.
    #[test]
    fn commands_without_secrets_never_open_the_credential_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = bootstrapped(dir.path(), &["web"]);
        assert!(
            ctx.password_store.get().is_none(),
            "bootstrap opened the credential store"
        );

        let list = ["list", "--format", "json"].map(String::from);
        assert_eq!(
            crate::cli::run_subcommand(&mut ctx, "host", &list).unwrap(),
            0
        );
        assert!(
            ctx.password_store.get().is_none(),
            "host list opened the credential store"
        );

        let zsh = ["zsh".to_string()];
        assert_eq!(
            crate::cli::run_subcommand(&mut ctx, "completions", &zsh).unwrap(),
            0
        );
        assert!(
            ctx.password_store.get().is_none(),
            "completions opened the credential store"
        );
    }
}
