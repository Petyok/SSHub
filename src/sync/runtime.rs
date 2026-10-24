//! What a sync run needs from a profile, shared by the headless commands and
//! the TUI worker: the store, the device identity, which ssh host reaches
//! which device, and the two client-side runs (a round, an enrollment).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::profile::ProfilePaths;
use crate::ssh::SshHost;
use crate::store::LauncherStore;
use crate::sync::client::{
    Round, RoundCtx, RoundFn, SshTransport, SyncCmd, SyncEvent, TransportError,
};
use crate::sync::crypto::DeviceIdentity;
use crate::sync::enroll::{run_sync_join, EnrollOutcome, SasConfirm};
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::DeviceId;
use crate::sync::protocol::{run_round, RoundSummary};
use crate::sync::transport::Limits;

/// What runs on the far side of the ssh connection.
pub const SERVE_COMMAND: &str = "sshub sync-serve";
pub const ENROLL_COMMAND: &str = "sshub sync-enroll";

/// Enrollment waits for a human on both ends: idle and round limits in minutes,
/// not the 30 s of an unattended round.
pub fn enroll_limits() -> Limits {
    Limits {
        idle: Duration::from_secs(300),
        round: Duration::from_secs(900),
        ..Limits::default()
    }
}

/// Which ssh host reaches which device: `device id -> host name or
/// `[user@]host[:port]``. Written when an enrollment succeeds; a device nobody
/// enrolled from here is reached through the ring, not directly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Peers(BTreeMap<DeviceId, String>);

impl Peers {
    fn file(paths: &ProfilePaths) -> PathBuf {
        paths.root.join("sync").join("peers.json")
    }

    pub fn load(paths: &ProfilePaths) -> Result<Self> {
        match std::fs::read(Self::file(paths)) {
            Ok(bytes) => Ok(Self(
                serde_json::from_slice(&bytes).context("read sync/peers.json")?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("read sync/peers.json"),
        }
    }

    pub fn host_of(&self, device: &DeviceId) -> Option<&str> {
        self.0.get(device).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DeviceId, &str)> {
        self.0.iter().map(|(d, h)| (d, h.as_str()))
    }

    /// Remember how to reach `device` and save the file (owner-only).
    pub fn record(paths: &ProfilePaths, device: &DeviceId, host: &str) -> Result<()> {
        let mut peers = Self::load(paths)?;
        peers.0.insert(device.clone(), host.to_string());
        let file = Self::file(paths);
        let dir = file.parent().context("peers path has no parent")?;
        std::fs::create_dir_all(dir)?;
        crate::secure_fs::restrict_dir(dir);
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&peers.0)?)?;
        crate::secure_fs::restrict_file(&tmp);
        std::fs::rename(&tmp, &file)?;
        Ok(())
    }
}

/// A host from the launcher list by name, else `[user@]host[:port]` as typed
/// (a blank machine has no list yet).
pub fn resolve_host(store: &LauncherStore, spec: &str) -> Result<SshHost> {
    if let Some(m) = store.get_host_by_name(spec)? {
        return Ok(crate::app::util::managed_to_ssh_host(&m));
    }
    let (user, rest) = match spec.split_once('@') {
        Some((u, r)) => (Some(u.to_string()), r),
        None => (None, spec),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !h.contains(':') => (
            h,
            Some(
                p.parse::<u16>()
                    .with_context(|| format!("port in '{spec}'"))?,
            ),
        ),
        _ => (rest, None),
    };
    anyhow::ensure!(!host.is_empty(), "no host in '{spec}'");
    let mut h = SshHost::new(spec);
    h.hostname = Some(host.to_string());
    h.user = user;
    h.port = port;
    Ok(h)
}

/// This machine's name as peers will see it.
pub fn device_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().chars().take(60).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sshub-device".into())
}

/// One profile's sync state, cheap to clone into a worker thread.
#[derive(Clone)]
pub struct SyncRuntime {
    pub paths: ProfilePaths,
    pub store: Arc<LauncherStore>,
    pub identity: Arc<DeviceIdentity>,
    /// A program to run where the argv names `ssh` (tests).
    pub ssh_program: Option<String>,
}

impl SyncRuntime {
    pub fn new(paths: ProfilePaths, store: Arc<LauncherStore>, identity: DeviceIdentity) -> Self {
        Self {
            paths,
            store,
            identity: Arc::new(identity),
            ssh_program: None,
        }
    }

    fn transport(&self, host: SshHost, command: &str) -> SshTransport {
        let t = SshTransport::new(host, command);
        match &self.ssh_program {
            Some(p) => t.with_program(p.clone()),
            None => t,
        }
    }

    /// One sync round with `device`, over the host recorded for it.
    pub fn sync_with(
        &self,
        device: &DeviceId,
        cancel: &Arc<AtomicBool>,
    ) -> Result<RoundSummary, TransportError> {
        let spec = Peers::load(&self.paths)
            .map_err(|e| TransportError::Failed(format!("{e:#}")))?
            .host_of(device)
            .map(str::to_string)
            .ok_or_else(|| {
                TransportError::Offline(
                    "no host is known for this device: enroll it from the hosts tab (t) or `sshub sync-join <host>`"
                        .into(),
                )
            })?;
        let host = resolve_host(&self.store, &spec)
            .map_err(|e| TransportError::Offline(format!("{e:#}")))?;
        self.transport(host, SERVE_COMMAND)
            .run(cancel, Limits::default(), |conn| {
                run_round(conn, &self.store, &self.identity, device)
            })
    }

    /// Join the ring of the participant at `host_spec`: run `sshub sync-enroll`
    /// there, compare the words through `ui`, and remember the host.
    pub fn join(
        &self,
        host_spec: &str,
        ui: &mut dyn SasConfirm,
        cancel: &Arc<AtomicBool>,
    ) -> Result<EnrollOutcome, TransportError> {
        let host = resolve_host(&self.store, host_spec)
            .map_err(|e| TransportError::Offline(format!("{e:#}")))?;
        let name = device_name();
        let outcome =
            self.transport(host, ENROLL_COMMAND)
                .run(cancel, enroll_limits(), |conn| {
                    run_sync_join(conn, &self.store, &self.identity, &name, true, ui)
                })?;
        Peers::record(&self.paths, &outcome.peer.device_id(), host_spec)
            .map_err(|e| TransportError::Failed(format!("enrolled, but {e:#}")))?;
        Ok(outcome)
    }
}

/// The enrollment's human decision, asked of the TUI: show the words (a
/// `SasPrompt` event), wait for the answer the SAS modal sends back.
pub(crate) struct UiSas<'a, 'b>(pub &'a RoundCtx<'b>);

impl SasConfirm for UiSas<'_, '_> {
    fn confirm(&mut self, sas: &str, _peer_name: &str) -> bool {
        self.0.emit(SyncEvent::SasPrompt(sas.to_string()));
        loop {
            match self.0.recv_cmd(Duration::from_secs(600)) {
                Some(SyncCmd::EnrollConfirmSas(ok)) => return ok,
                Some(_) => {}
                None => return false,
            }
        }
    }
}

/// The TUI worker's round function over ssh. The device identity is loaded
/// (or created) on the first round, not at start-up.
pub fn round_fn(paths: ProfilePaths, store: Arc<LauncherStore>) -> RoundFn {
    let mut runtime: Option<SyncRuntime> = None;
    Box::new(move |kind, device, ctx| {
        if runtime.is_none() {
            let secrets = crate::credentials::open_for_profile(&paths);
            let identity = load_or_init_identity(&paths, secrets.as_ref())?;
            runtime = Some(SyncRuntime::new(paths.clone(), store.clone(), identity));
        }
        let rt = runtime.as_ref().expect("set above");
        let cancel = ctx.cancel_flag().clone();
        match kind {
            Round::Sync => Ok(rt.sync_with(device, &cancel)?),
            // For an enrollment `device` is the host to join through.
            Round::Enroll => {
                let outcome = rt.join(device, &mut UiSas(ctx), &cancel)?;
                Ok(RoundSummary {
                    peer: outcome.peer.device_id(),
                    first_contact: true,
                    ..Default::default()
                })
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> LauncherStore {
        LauncherStore::open_in_memory().unwrap()
    }

    #[test]
    fn an_unlisted_host_is_parsed_as_user_host_port() {
        let h = resolve_host(&store(), "me@box.example:2222").unwrap();
        assert_eq!(h.hostname.as_deref(), Some("box.example"));
        assert_eq!(h.user.as_deref(), Some("me"));
        assert_eq!(h.port, Some(2222));
        let h = resolve_host(&store(), "box").unwrap();
        assert_eq!(
            (h.hostname.as_deref(), h.user, h.port),
            (Some("box"), None, None)
        );
        assert!(resolve_host(&store(), "box:notaport").is_err());
        assert!(resolve_host(&store(), "me@").is_err());
    }

    #[test]
    fn peers_round_trip_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProfilePaths {
            data_root: dir.path().to_path_buf(),
            id: "t".into(),
            name: "t".into(),
            root: dir.path().to_path_buf(),
            config_file: dir.path().join("config.toml"),
            ssh_config: dir.path().join("ssh_config"),
            compat: false,
        };
        assert!(Peers::load(&paths).unwrap().iter().next().is_none());
        Peers::record(&paths, &"d1".to_string(), "hub").unwrap();
        Peers::record(&paths, &"d2".to_string(), "me@x:22").unwrap();
        let p = Peers::load(&paths).unwrap();
        assert_eq!(p.host_of(&"d1".to_string()), Some("hub"));
        assert_eq!(p.host_of(&"d2".to_string()), Some("me@x:22"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(Peers::file(&paths))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
