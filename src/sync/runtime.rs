//! What a sync run needs from a profile, shared by the headless commands and
//! the TUI worker: the store, the device identity, which ssh host reaches
//! which device, and the two client-side runs (a round, an enrollment).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use zeroize::Zeroizing;

use crate::app::sync::OnboardingSecrets;
use crate::credentials::{PasswordStore, SecretGate, SecretRef, UnlockError, Unlocker};
use crate::profile::ProfilePaths;
use crate::ssh::SshHost;
use crate::store::LauncherStore;
use crate::sync::client::{
    Round, RoundCtx, RoundFn, SasPromptInfo, SshTransport, SyncCmd, SyncEvent, TransportError,
};
use crate::sync::cred_ops::{cred_put, CredPayload};
use crate::sync::crypto::{Argon2Profile, DeviceIdentity, ARGON2_MASTER};
use crate::sync::enroll::{run_sync_join, EnrollOutcome, EnrollProgress, SasConfirm, SasPrompt};
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::{Attendance, DeviceId, DeviceKind, DeviceRole, VerifiedBy};
use crate::sync::protocol::{run_round, run_share_request, RoundSummary, ShareAsk, ShareReply};
use crate::sync::quorum::{parse_ring_secret, ring_secret_text, QuorumUnlocker};
use crate::sync::transport::Limits;
use crate::sync::ui_shares::UiShares;

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
        // Local hosts, groups and identities the ring has not heard of go first.
        crate::sync::entity_capture::publish_pending(&self.store, &self.identity)
            .map_err(|e| TransportError::Failed(format!("{e:#}")))?;
        let summary =
            self.transport(host, SERVE_COMMAND)
                .run(cancel, Limits::default(), |conn| {
                    run_round(conn, &self.store, &self.identity, device)
                })?;
        self.mail_owed_codes();
        Ok(summary)
    }

    /// After a completed round: when this device is the registered sender of an
    /// emailed third-factor code that a Red change now waits for, mail it through
    /// `[sync.notify]`. A missing or broken config, or a failing mail command,
    /// sends nothing and marks nothing (the next round tries again); nothing is
    /// printed because this also runs under the TUI.
    fn mail_owed_codes(&self) {
        let Ok(config) = crate::config::load_config_at(&self.paths.config_file) else {
            return;
        };
        let notify = &config.sync.notify;
        if notify.command.is_none() {
            return;
        }
        let _ = crate::sync::ceremony::send_owed_codes(
            &self.store,
            &self.identity,
            crate::store::now_ts(),
            &mut |subject, body| crate::sync::notify::send_notification(notify, subject, body),
        );
    }

    /// Ask `holder` for a share over its recorded host, inside an authenticated
    /// session (the trust-session path).
    pub(crate) fn share_request(
        &self,
        holder: &DeviceId,
        ask: &ShareAsk,
    ) -> Result<ShareReply, TransportError> {
        let spec = Peers::load(&self.paths)
            .map_err(|e| TransportError::Failed(format!("{e:#}")))?
            .host_of(holder)
            .map(str::to_string)
            .ok_or_else(|| TransportError::Offline("no host is known for this device".into()))?;
        let host = resolve_host(&self.store, &spec)
            .map_err(|e| TransportError::Offline(format!("{e:#}")))?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.transport(host, SERVE_COMMAND)
            .run(&cancel, Limits::default(), |conn| {
                run_share_request(conn, &self.store, &self.identity, holder, ask)
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
                    run_sync_join(
                        conn,
                        &self.store,
                        &self.identity,
                        &name,
                        true,
                        Some(&mut *ui),
                    )
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
    fn confirm(&mut self, prompt: &SasPrompt<'_>) -> bool {
        self.0.emit(SyncEvent::SasPrompt(SasPromptInfo {
            words: prompt.sas.to_string(),
            peer_name: prompt.peer_name.to_string(),
            peer_has_screen: prompt.peer_has_screen,
        }));
        loop {
            match self.0.recv_cmd(Duration::from_secs(600)) {
                Some(SyncCmd::EnrollConfirmSas(ok)) => return ok,
                Some(_) => {}
                None => return false,
            }
        }
    }

    fn progress(&mut self, p: EnrollProgress) {
        self.0.emit(SyncEvent::Progress(progress_line(p)));
    }
}

/// The worker's status line for a join anchor step.
pub(crate) fn progress_line(p: EnrollProgress) -> String {
    match p {
        EnrollProgress::Receiving { chunk, total } => {
            format!("receiving the ring ({chunk}/{total})")
        }
        EnrollProgress::Adopting { hosts } => format!("adopting the ring ({hosts} hosts)"),
        EnrollProgress::Adopted { hosts } => format!("adopted the ring ({hosts} hosts)"),
    }
}

/// Saves of secrets become replicated credentials while the profile is in a
/// ring (installed on the gate, so every `PasswordStore::set` and `delete`
/// goes through it, TUI and CLI alike).
pub struct StoreReplicator {
    store: Arc<LauncherStore>,
    paths: ProfilePaths,
    open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
    gate: std::sync::Weak<SecretGate>,
    identity: Mutex<Option<Arc<DeviceIdentity>>>,
}

impl StoreReplicator {
    pub fn install(
        gate: &Arc<SecretGate>,
        paths: ProfilePaths,
        store: Arc<LauncherStore>,
        open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
    ) {
        gate.set_replicator(Box::new(Self {
            store,
            paths,
            open_secrets,
            gate: Arc::downgrade(gate),
            identity: Mutex::new(None),
        }));
    }

    fn identity(&self) -> Result<Arc<DeviceIdentity>> {
        let mut slot = self.identity.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let secrets = (self.open_secrets)(&self.paths);
            *slot = Some(Arc::new(load_or_init_identity(
                &self.paths,
                secrets.as_ref(),
            )?));
        }
        Ok(slot.clone().expect("set above"))
    }

    /// The key file text of identity `id`, when it has one that is a key.
    fn key_text(&self, id: i64) -> Option<String> {
        let path = self.store.get_identity(id).ok().flatten()?.private_key?;
        let text =
            std::fs::read_to_string(crate::ssh::expand_tilde(&path.to_string_lossy())).ok()?;
        crate::ssh::looks_like_private_key(&text).then_some(text)
    }

    fn in_ring(&self) -> Result<bool> {
        Ok(!self.store.sync_devices()?.is_empty())
    }
}

impl crate::credentials::Replicator for StoreReplicator {
    fn replicate(&self, key: &str, password: Option<&str>) -> Result<bool> {
        let Some(uid) = crate::sync::quorum::secret_uid(&self.store, key) else {
            return Ok(false);
        };
        if !self.in_ring()? {
            return Ok(false);
        }
        let synced = self.store.sync_cred_ciphertext(&uid)?.is_some();
        if password.is_none() && !synced {
            return Ok(false);
        }
        let private_key = key
            .strip_prefix("identity:")
            .and_then(|i| i.parse::<i64>().ok())
            .and_then(|id| self.key_text(id));
        let payload = CredPayload {
            password: password.map(str::to_string),
            private_key,
        };
        let identity = self.identity()?;
        cred_put(&self.store, &identity, &uid, &payload)?;
        Ok(true)
    }

    fn replicate_identity_key(&self, identity_id: i64) -> Result<bool> {
        let key = crate::credentials::identity_key(identity_id);
        let Some(uid) = crate::sync::quorum::secret_uid(&self.store, &key) else {
            return Ok(false);
        };
        let Some(private_key) = self.key_text(identity_id) else {
            return Ok(false);
        };
        if !self.in_ring()? {
            return Ok(false);
        }
        let synced = self.store.sync_cred_ciphertext(&uid)?.is_some();
        let open = self.gate.upgrade().and_then(|g| g.get(&key));
        let password = match (open, synced) {
            (Some(p), _) => (!p.is_empty()).then(|| p.to_string()),
            (None, false) => (self.open_secrets)(&self.paths).get(&key).ok().flatten(),
            (None, true) => anyhow::bail!(
                "this identity's credential is locked: unlock it, then save again to replicate the key"
            ),
        };
        let payload = CredPayload {
            password,
            private_key: Some(private_key),
        };
        let identity = self.identity()?;
        cred_put(&self.store, &identity, &uid, &payload)?;
        Ok(true)
    }
}

/// What an admin job gets on the worker: the store, this device's identity and
/// the profile's secrets.
pub struct JobEnv {
    pub store: Arc<LauncherStore>,
    pub identity: Arc<DeviceIdentity>,
    pub paths: ProfilePaths,
    pub gate: Arc<SecretGate>,
    pub argon: Argon2Profile,
    pub open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
}

/// A job: runs once on the worker, returns the line to tell the user.
pub type JobFn = Box<dyn FnOnce(&JobEnv) -> Result<String> + Send>;

/// A [`JobFn`] that can ride in a command and an intent: cloneable, compared by
/// identity, taken once.
#[derive(Clone)]
pub struct JobBox(Arc<Mutex<Option<JobFn>>>);

impl JobBox {
    pub fn new(job: impl FnOnce(&JobEnv) -> Result<String> + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(job)))))
    }

    pub fn take(&self) -> Option<JobFn> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl std::fmt::Debug for JobBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JobBox(..)")
    }
}

impl PartialEq for JobBox {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for JobBox {}

/// Credential-store entry holding the ring secret of this profile's Vault.
pub const RING_SECRET_KEY: &str = "sync-ring-secret";

/// What the TUI worker's rounds need besides the store.
#[derive(Clone)]
pub struct RoundDeps {
    pub paths: ProfilePaths,
    pub store: Arc<LauncherStore>,
    /// The gate of the app's credential store: unlocking fills it.
    pub gate: Arc<SecretGate>,
    /// Master-phrase stretching (tests use a cheap profile).
    pub argon: Argon2Profile,
    /// Opens the profile's UNGATED credential store: the device key
    /// passphrase and the ring secret live there.
    pub open_secrets: fn(&ProfilePaths) -> Box<dyn PasswordStore>,
    /// A program to run where the argv names `ssh` (tests).
    pub ssh_program: Option<String>,
}

impl RoundDeps {
    pub fn for_profile(
        paths: ProfilePaths,
        store: Arc<LauncherStore>,
        gate: Arc<SecretGate>,
    ) -> Self {
        Self {
            paths,
            store,
            gate,
            argon: ARGON2_MASTER,
            open_secrets: crate::credentials::open_ungated_for_profile,
            ssh_program: None,
        }
    }
}

/// Make the gate guard every credential the ring holds: from now on those keys
/// answer `Locked` until opened, and never fall through to the local store.
pub fn guard_synced_credentials(gate: &SecretGate, store: Arc<LauncherStore>) {
    gate.set_synced_check(move |key| {
        crate::sync::quorum::secret_uid(&store, key)
            .is_some_and(|u| store.sync_cred_ciphertext(&u).ok().flatten().is_some())
    });
}

fn read_ring_secret(deps: &RoundDeps) -> Option<[u8; 16]> {
    let text = (deps.open_secrets)(&deps.paths)
        .get(RING_SECRET_KEY)
        .ok()??;
    parse_ring_secret(&text).ok()
}

/// Run the unlock on its own thread and pump the UI's answers to it until it
/// finishes: the share source blocks on the answers, the worker keeps reading
/// commands.
fn unlock_round(deps: &RoundDeps, rt: &SyncRuntime, keys: &str, ctx: &RoundCtx<'_>) -> Result<()> {
    let refs: Vec<SecretRef> = keys
        .lines()
        .map(|k| SecretRef::Key(k.to_string()))
        .collect();
    let (answer_tx, answer_rx) = mpsc::channel::<String>();
    let source = UiShares {
        events: ctx.event_sender(),
        answers: Mutex::new(answer_rx),
        runtime: rt.clone(),
        request: Mutex::new(None),
    };
    let unlocker = QuorumUnlocker::with_profile(
        deps.store.clone(),
        rt.identity.clone(),
        deps.gate.clone(),
        Box::new(source),
        read_ring_secret(deps),
        deps.argon,
    );
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(unlocker.unlock(&refs));
    });
    let mut answer_tx = Some(answer_tx);
    let result = loop {
        match done_rx.try_recv() {
            Ok(result) => break result,
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err(UnlockError::Failed(
                    "the unlock stopped unexpectedly".into(),
                ))
            }
        }
        match ctx.recv_cmd(Duration::from_millis(100)) {
            Some(SyncCmd::UnlockCode(code)) => {
                if let Some(tx) = &answer_tx {
                    let _ = tx.send(code);
                }
            }
            Some(SyncCmd::UnlockCancel) => answer_tx = None,
            _ if ctx.cancel_flag().load(std::sync::atomic::Ordering::SeqCst) => answer_tx = None,
            _ => {}
        }
    };
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

/// Set up the Vault ring: the recipient key from phrase and ring secret, the
/// ring secret in the credential store, this device as the ring's first entry.
fn onboard_round(deps: &RoundDeps, identity: &DeviceIdentity, s: &OnboardingSecrets) -> Result<()> {
    crate::sync::crypto::eff_decode(&s.phrase)
        .context("the master phrase is not 6 words of the EFF list")?;
    crate::sync::quorum::vault_setup_with(&deps.store, &s.phrase, &s.ring_secret, deps.argon)?;
    let text = Zeroizing::new(ring_secret_text(&s.ring_secret));
    (deps.open_secrets)(&deps.paths)
        .set(RING_SECRET_KEY, &text)
        .context("keep the ring secret in the credential store")?;
    if deps.store.sync_devices()?.is_empty() {
        let me = identity.public();
        deps.store.sync_register_device(
            &me,
            &device_name(),
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )?;
        // The first device founds the ring: enrollment roles follow `ring_id`.
        crate::sync::quorum::meta_set(&deps.store, "ring_id", me.device_id().as_bytes())?;
    }
    Ok(())
}

/// Answer a pending `ShareRelease`: resolve it in the store and, when approved,
/// append one grant per share epoch this holder has (sealed to the request's
/// ephemeral key). `spec` is `<0|1>:<op hash hex>`.
fn release_round(deps: &RoundDeps, identity: &DeviceIdentity, spec: &str) -> Result<()> {
    let (flag, hash_hex) = spec.split_once(':').context("release request")?;
    let approve = flag == "1";
    let hash: [u8; 32] = hex::decode(hash_hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("release request hash"))?;
    let op = deps
        .store
        .sync_pending()?
        .into_iter()
        .find(|o| o.hash() == hash)
        .context("that release request is no longer waiting")?;
    crate::sync::ceremony::resolve(&deps.store, &hash, approve, crate::store::now_ts())?;
    if approve {
        for body in crate::sync::quorum::grant_bodies(&deps.store, identity, &op)? {
            deps.store.sync_append_local(identity, body)?;
        }
    }
    Ok(())
}

/// The TUI worker's round function. The device identity is loaded (or
/// created) on the first round, not at start-up.
pub fn round_fn(deps: RoundDeps) -> RoundFn {
    let mut runtime: Option<SyncRuntime> = None;
    Box::new(move |kind, device, ctx| {
        if runtime.is_none() {
            let secrets = (deps.open_secrets)(&deps.paths);
            let identity = load_or_init_identity(&deps.paths, secrets.as_ref())?;
            let mut rt = SyncRuntime::new(deps.paths.clone(), deps.store.clone(), identity);
            rt.ssh_program = deps.ssh_program.clone();
            runtime = Some(rt);
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
            Round::Unlock => {
                unlock_round(&deps, rt, device, ctx)?;
                Ok(RoundSummary::default())
            }
            Round::Job => {
                let job = ctx
                    .take_job()
                    .and_then(|j| j.take())
                    .context("the job was already taken")?;
                let env = JobEnv {
                    store: deps.store.clone(),
                    identity: rt.identity.clone(),
                    paths: deps.paths.clone(),
                    gate: deps.gate.clone(),
                    argon: deps.argon,
                    open_secrets: deps.open_secrets,
                };
                let outcome = job(&env);
                // A job may have rotated the device keys: reload on the next round.
                drop(env);
                runtime = None;
                ctx.emit(SyncEvent::Notice(outcome?));
                Ok(RoundSummary::default())
            }
            Round::Identify => {
                ctx.emit(SyncEvent::ThisDevice(rt.identity.public().device_id()));
                Ok(RoundSummary::default())
            }
            Round::Release => {
                release_round(&deps, &rt.identity, device)?;
                Ok(RoundSummary::default())
            }
            Round::Onboard => {
                let secrets = ctx.take_secrets().context("no onboarding secrets")?;
                onboard_round(&deps, &rt.identity, &secrets)?;
                Ok(RoundSummary::default())
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
        let dir_mode = std::fs::metadata(Peers::file(&paths).parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o077, 0, "the sync directory is owner-only too");
        assert!(
            Peers::file(&paths).starts_with(&paths.root),
            "under the profile root"
        );
    }
}

#[cfg(test)]
mod mail_tests {
    use super::*;
    use crate::sync::ceremony::{
        factor_register_email, factor_register_paper, owed_code_mails, red_approve_body,
        red_factor_paper, red_phone_sign_body, remember_email_code, settle_red_ops,
    };
    use crate::sync::crypto::{bip39_encode, paper_derive, PaperDomain};
    use crate::sync::oplog::{CredMode, OpBody};
    use crate::sync::protocol::tests::Dev;
    use crate::sync::quorum::tests::append;

    /// Everything of `from`'s log that `to` lacks, ingested without anyone approving.
    fn sync(d: &[Dev]) {
        for from in d {
            for to in d {
                if from.did == to.did {
                    continue;
                }
                for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
                    let have = to
                        .store
                        .sync_chain_heads()
                        .unwrap()
                        .into_iter()
                        .find(|(x, _, _)| *x == dev)
                        .map_or(0, |(_, s, _)| s);
                    for op in from.store.sync_chain_tail(&dev, have).unwrap() {
                        to.store.sync_ingest(op).unwrap();
                    }
                }
            }
        }
    }

    /// A fake sendmail: appends what it is given to `capture`, then exits with `code`.
    fn fake_sendmail(dir: &std::path::Path, capture: &std::path::Path, code: i32) -> String {
        let script = dir.join(format!("sendmail{code}.sh"));
        std::fs::write(
            &script,
            format!("#!/bin/sh\ncat >> '{}'\nexit {code}\n", capture.display()),
        )
        .unwrap();
        format!("/bin/sh {}", script.display())
    }

    #[test]
    fn a_completed_round_mails_each_owed_code_once_and_marks_it_only_when_the_mail_went() {
        // 0 proposer, 1 and 2 attended terminals, 3 unattended hub, 4 phone.
        let d: Vec<Dev> = (0..5).map(|_| Dev::new()).collect();
        for x in &d {
            crate::sync::ceremony::test_force(&x.store, false);
        }
        for (i, x) in d.iter().enumerate() {
            for (j, y) in d.iter().enumerate() {
                if i == j || j == 0 {
                    continue;
                }
                let (attendance, kind) = match j {
                    3 => (Attendance::Unattended, DeviceKind::Terminal),
                    4 => (Attendance::Attended, DeviceKind::Phone),
                    _ => (Attendance::Attended, DeviceKind::Terminal),
                };
                x.trust_with(y, attendance, kind);
            }
            // The proposer is known to everybody as an attended terminal.
        }
        for x in d.iter().skip(1) {
            x.trust_with(&d[0], Attendance::Attended, DeviceKind::Terminal);
        }
        d[3].store
            .sync_register_device(
                &d[3].id.public(),
                "hub",
                DeviceRole::Holder,
                Attendance::Unattended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        let propose = |from: usize, body: OpBody| {
            let op = append(&d[from], body);
            sync(&d);
            op
        };
        let phrase = bip39_encode(&[0x35; 32]);
        let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
        let reg = propose(0, factor_register_paper(&phrase).unwrap());
        append(&d[4], red_phone_sign_body(&reg));
        append(&d[1], red_approve_body(&reg));
        sync(&d);
        for x in &d {
            settle_red_ops(&x.store, reg.timestamp + 5).unwrap();
        }
        // Device 2 is the registered sender of the emailed code.
        let (body, code) = factor_register_email(&d[2].did).unwrap();
        let ereg = propose(0, body);
        append(&d[4], red_phone_sign_body(&ereg));
        append(&d[1], red_approve_body(&ereg));
        append(&d[0], red_factor_paper(&recover, &ereg));
        sync(&d);
        for x in &d {
            settle_red_ops(&x.store, ereg.timestamp + 5).unwrap();
        }
        remember_email_code(&d[2].store, &d[2].id, &code).unwrap();
        sync(&d);
        let op = propose(
            0,
            OpBody::ModeChange {
                to: CredMode::Quorum,
                migrated_on: d[0].did.clone(),
                cosigner: None,
            },
        );
        append(&d[4], red_phone_sign_body(&op));
        append(&d[1], red_approve_body(&op));
        sync(&d);
        let now = crate::store::now_ts();
        assert_eq!(
            owed_code_mails(&d[2].store, &d[2].id, now).unwrap().len(),
            1,
            "the sender owes the code once the other proofs are in"
        );

        // This device (2) runs a completed client round with [sync.notify] configured.
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
        let capture = dir.path().join("mail.txt");
        let rt = SyncRuntime {
            paths: paths.clone(),
            store: d[2].store.clone(),
            identity: d[2].id.clone(),
            ssh_program: None,
        };
        let configure = |command: &str| {
            std::fs::write(
                &paths.config_file,
                format!(
                    "[sync.notify]\ncommand = \"{command}\"\nto = [\"me@example.com\"]\nfrom = \"sshub@example.com\"\n"
                ),
            )
            .unwrap();
        };
        let owed = || owed_code_mails(&d[2].store, &d[2].id, now).unwrap().len();

        // No config: nothing is sent, nothing is marked.
        rt.mail_owed_codes();
        assert_eq!(owed(), 1);

        // A failing mail command marks nothing (the next round tries again).
        configure(&fake_sendmail(dir.path(), &capture, 1));
        rt.mail_owed_codes();
        assert_eq!(owed(), 1, "a failed mail is still owed");

        // A working one sends the code once and marks it.
        capture
            .exists()
            .then(|| std::fs::remove_file(&capture).unwrap());
        configure(&fake_sendmail(dir.path(), &capture, 0));
        rt.mail_owed_codes();
        assert_eq!(owed(), 0, "marked after the send");
        let mail = std::fs::read_to_string(&capture).unwrap();
        assert_eq!(mail.matches(&code).count(), 1, "{mail}");
        assert!(mail.contains("To: me@example.com"), "{mail}");

        // The next round sends nothing more.
        rt.mail_owed_codes();
        assert_eq!(std::fs::read_to_string(&capture).unwrap(), mail);
    }
}
