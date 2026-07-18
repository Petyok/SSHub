use std::path::PathBuf;

use anyhow::Result;

use crate::metadata::HostMetadata;
use crate::ssh::SshHost;
use crate::store::{LauncherStore, NewHost, NewIdentity};

/// Duplicate a legacy ssh_config alias into a launcher-managed host.
///
/// May auto-create a matching identity when user/key fields are present.
pub fn duplicate_legacy_to_launcher(
    store: &LauncherStore,
    host: &SshHost,
    meta: &HostMetadata,
) -> Result<String> {
    let mut name = format!("{}-copy", host.name);
    let mut suffix = 2u32;
    while store.get_host_by_name(&name)?.is_some() {
        name = format!("{}-copy-{}", host.name, suffix);
        suffix += 1;
    }

    let address = host.hostname.clone().unwrap_or_else(|| host.name.clone());
    let port = host.port.unwrap_or(22);

    let mut new_host = NewHost::launcher(name.clone(), address);
    new_host.port = port;
    new_host.tags = meta.tags.clone();
    new_host.notes = meta.description.clone();
    new_host.proxy_jump = host.proxy_jump.clone();
    new_host.forward_agent = host.forward_agent.unwrap_or(false);
    new_host.remote_command = host.remote_command.clone();
    new_host.session_logging = meta.session_logging;
    new_host.transport = meta.transport;
    new_host.identity_id = match_identity_for_ssh_host(store, host)?;
    store.create_host(&new_host)?;
    Ok(name)
}

/// Find or create an identity matching ssh `-G` user/key fields.
pub fn match_identity_for_ssh_host(store: &LauncherStore, host: &SshHost) -> Result<Option<i64>> {
    let user = host.user.as_deref();
    let key = host.identity_file.as_deref();
    if user.is_none() && key.is_none() {
        return Ok(None);
    }

    for identity in store.list_identities()? {
        let id_user = identity.username.as_deref();
        let id_key = identity
            .private_key
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let matches = match (user, key) {
            (Some(u), Some(k)) => id_user == Some(u) && id_key.as_deref() == Some(k),
            (Some(u), None) => id_user == Some(u),
            (None, Some(k)) => id_key.as_deref() == Some(k),
            (None, None) => false,
        };
        if matches {
            return Ok(Some(identity.id));
        }
    }

    let mut identity_name = format!("{}-identity", host.name);
    let mut suffix = 2u32;
    while store.get_identity_by_name(&identity_name)?.is_some() {
        identity_name = format!("{}-identity-{}", host.name, suffix);
        suffix += 1;
    }

    let created = store.create_identity(&NewIdentity {
        name: identity_name,
        username: host.user.clone(),
        private_key: key.map(PathBuf::from),
        ..Default::default()
    })?;
    Ok(Some(created.id))
}
