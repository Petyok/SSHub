use anyhow::Result;
use std::collections::HashMap;
use std::fs;

const SERVICE: &str = "sshub";

pub trait PasswordStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, password: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    /// Fetch a host password named by a per-host `Attribute=value` spec
    /// (#142). A method so tests can answer without a session bus.
    fn lookup(&self, spec: &str) -> Result<String> {
        secret_service_lookup(spec)
    }
    /// Where this store lives, so another process (the askpass helper that
    /// ssh runs) can open the same one. `None` for a store that only exists
    /// inside this process.
    fn locator(&self) -> Option<StoreLocator> {
        None
    }
    /// The unlock gate when this store guards synced credentials; `None` for a
    /// store that holds only local secrets.
    fn gate(&self) -> Option<&SecretGate> {
        None
    }
    /// The same gate as a shared handle, for the unlocker that fills it.
    fn shared_gate(&self) -> Option<std::sync::Arc<SecretGate>> {
        None
    }
}

/// Enough to reopen a [`PasswordStore`] in another process: never a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreLocator {
    Keyring {
        prefix: String,
    },
    File {
        path: std::path::PathBuf,
        prefix: String,
    },
}

impl StoreLocator {
    pub fn open(&self) -> Box<dyn PasswordStore> {
        match self {
            Self::Keyring { prefix } => {
                Box::new(NamespacedPasswordStore::new(Box::new(OsKeyring), prefix))
            }
            Self::File { path, prefix } => Box::new(NamespacedPasswordStore::new(
                Box::new(FilePasswordStore::new(path.clone())),
                prefix,
            )),
        }
    }

    fn prefixed(self, outer: &str) -> Self {
        match self {
            Self::Keyring { prefix } => Self::Keyring {
                prefix: format!("{outer}{prefix}"),
            },
            Self::File { path, prefix } => Self::File {
                path,
                prefix: format!("{outer}{prefix}"),
            },
        }
    }
}

/// The credential store of one profile: the OS keyring when usable, else the
/// profile's `credentials.json`, always under the profile's key prefix. Synced
/// credentials are answered by the gate, never read from the inner store.
pub fn open_for_profile(paths: &crate::profile::ProfilePaths) -> Box<dyn PasswordStore> {
    Box::new(QuorumGatedStore::new(open_ungated_for_profile(paths)))
}

/// [`open_for_profile`] without the gate: for the one job that must read the
/// local copies of secrets about to be synced (migration), and for keys the
/// sync stage keeps itself (`sync-ring-secret`).
pub fn open_ungated_for_profile(paths: &crate::profile::ProfilePaths) -> Box<dyn PasswordStore> {
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

/// Which credential-store entry holds a secret: a stored key (`host:3`,
/// `identity:7`), a host's Secret Service lookup spec (#142), or the opened
/// plaintext of a synced credential handed over directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretRef {
    Key(String),
    Lookup(String),
    /// A synced credential already open in this process, held by whoever
    /// stages it for a child that cannot open it itself (the askpass helper).
    Staged(StagedValue),
    /// What the child sees of a [`SecretRef::Staged`]: a handle to ask the
    /// staging process for, never the value.
    Handle(String),
}

/// Plaintext of an opened synced credential. Zeroized on drop; `Debug` never
/// prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct StagedValue(zeroize::Zeroizing<String>);

impl StagedValue {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for StagedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StagedValue(..)")
    }
}

impl SecretRef {
    /// The secret now, `None` when the entry is missing or unreadable.
    pub fn read(&self, store: &dyn PasswordStore) -> Option<String> {
        match self {
            Self::Key(key) => store.get(key).ok().flatten(),
            Self::Lookup(spec) => store.lookup(spec).ok(),
            Self::Staged(value) => Some(value.value().to_string()),
            // Only the staging process holds the value; the helper asks for it.
            Self::Handle(_) => None,
        }
    }
}

/// Split a secret lookup spec into `(attribute, value)` at the first `=`.
// ponytail: one pair is the ceiling; `;`-separated pairs are the upgrade
// path if a single attribute ever stops being unique enough.
pub fn parse_secret_lookup(spec: &str) -> Result<(&str, &str)> {
    let Some((attr, value)) = spec.split_once('=') else {
        anyhow::bail!("secret lookup must be Attribute=value");
    };
    let (attr, value) = (attr.trim(), value.trim());
    if attr.is_empty() || value.is_empty() {
        anyhow::bail!("secret lookup must be Attribute=value");
    }
    Ok((attr, value))
}

/// Read the one Secret Service item whose attribute matches `spec` exactly
/// (KeePassXC exposes `Title`, `UserName`, `URL`, `Notes`, `Path`, `Uuid`).
#[cfg(target_os = "linux")]
pub fn secret_service_lookup(spec: &str) -> Result<String> {
    use dbus_secret_service::{EncryptionType, SecretService};
    let (attr, value) = parse_secret_lookup(spec)?;
    let ss = SecretService::connect(EncryptionType::Dh)?;
    let found = ss.search_items(HashMap::from([(attr, value)]))?;
    let mut items: Vec<_> = found.unlocked.into_iter().chain(found.locked).collect();
    match items.len() {
        0 => anyhow::bail!("no Secret Service entry has {attr}={value}"),
        1 => {}
        n => anyhow::bail!(
            "{n} Secret Service entries have {attr}={value}; use a unique attribute such as Uuid"
        ),
    }
    let item = items.remove(0);
    // The provider shows its own unlock/confirm prompt here.
    item.ensure_unlocked()?;
    Ok(String::from_utf8(item.get_secret()?)?)
}

#[cfg(not(target_os = "linux"))]
pub fn secret_service_lookup(_spec: &str) -> Result<String> {
    anyhow::bail!("secret lookup needs a Secret Service (Linux)")
}

/// Password store that prefixes every key with a namespace. Profiles use
/// `profile:<id>:` so identically named hosts in separate profiles never
/// share a keyring entry; compat mode uses an empty prefix (current keys).
pub struct NamespacedPasswordStore {
    inner: Box<dyn PasswordStore>,
    prefix: String,
}

impl NamespacedPasswordStore {
    pub fn new(inner: Box<dyn PasswordStore>, prefix: impl Into<String>) -> Self {
        Self {
            inner,
            prefix: prefix.into(),
        }
    }

    fn namespaced(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }
}

impl PasswordStore for NamespacedPasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        self.inner.get(&self.namespaced(key))
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        self.inner.set(&self.namespaced(key), password)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(&self.namespaced(key))
    }
    // The spec names a provider entry, not an SSHub key: no prefix.
    fn lookup(&self, spec: &str) -> Result<String> {
        self.inner.lookup(spec)
    }
    fn locator(&self) -> Option<StoreLocator> {
        self.inner.locator().map(|l| l.prefixed(&self.prefix))
    }
}

pub fn check_keyring_available() -> bool {
    store_available(&OsKeyring)
}

/// Whether `store` can actually hold a secret: write a unique probe entry,
/// read it back, and remove it. A read-only probe cannot see the failure that
/// matters here — with no session bus (`env -i`, no `DBUS_SESSION_BUS_ADDRESS`)
/// `get_password` fails with a dbus-autolaunch error no needle list matches,
/// so the old check reported `true`, the app picked the OS keyring, and every
/// remember-me write failed closed. Asking the store to do the real operation
/// sees it. The account is unique per call so concurrent startups never share it.
fn store_available(store: &dyn PasswordStore) -> bool {
    let account = format!(
        "sshub-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    if store.set(&account, "probe").is_err() {
        return false;
    }
    let ok = matches!(store.get(&account), Ok(Some(value)) if value == "probe");
    let _ = store.delete(&account);
    ok
}

pub fn migrate_fallback_to_keyring(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let map: HashMap<String, String> = serde_json::from_str(&content)?;

    let keyring_store = OsKeyring;
    let mut failed = false;
    for (key, password) in &map {
        if let Err(e) = keyring_store.set(key, password) {
            eprintln!("Failed to migrate key {key} to keyring: {e}");
            failed = true;
        }
    }

    if !failed {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

pub struct OsKeyring;

impl PasswordStore for OsKeyring {
    fn locator(&self) -> Option<StoreLocator> {
        Some(StoreLocator::Keyring {
            prefix: String::new(),
        })
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.get_password() {
            Ok(pw) => Ok(Some(pw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keyring: {e}")),
        }
    }
    fn set(&self, key: &str, password: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        entry.set_password(password)?;
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keyring: {e}")),
        }
    }
}

pub struct FilePasswordStore {
    path: std::path::PathBuf,
}

impl FilePasswordStore {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    fn read_map(&self) -> Result<HashMap<String, String>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let content = fs::read_to_string(&self.path)
            .map_err(|e| anyhow::anyhow!("Failed to read credentials file: {e}"))?;
        let map = serde_json::from_str::<HashMap<String, String>>(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse credentials file: {e}"))?;
        Ok(map)
    }

    fn write_map(&self, map: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            crate::secure_fs::restrict_dir(parent);
        }
        let content = serde_json::to_string_pretty(map)?;
        let tmp = self.path.with_extension("json.tmp");
        if tmp.exists() {
            anyhow::ensure!(
                !std::fs::symlink_metadata(&tmp)?.file_type().is_symlink(),
                "refusing symlink credentials temporary file: {}",
                tmp.display()
            );
            fs::remove_file(&tmp)?;
        }

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to create secure temp credentials file: {e}")
                })?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            crate::secure_fs::restrict_file(&tmp);
        }

        fs::rename(&tmp, &self.path)?;
        // Make the rename itself durable: without it a crash can leave the old
        // map after the caller was told the secret was saved.
        #[cfg(unix)]
        {
            if let Some(parent) = self.path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        Ok(())
    }

    /// Exclusive advisory lock around a read-modify-write. It sits on a sidecar
    /// file: `write_map` replaces the data file by rename, so a lock on the
    /// data file would guard an inode that is already gone. Two sshub
    /// processes (or an import next to the TUI) otherwise both read the old
    /// map and the later write drops the earlier one's secret. Released when
    /// the returned file is dropped.
    fn lock(&self) -> Result<fs::File> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            crate::secure_fs::restrict_dir(parent);
        }
        let lock_path = self.path.with_extension("json.lock");
        let mut opts = fs::OpenOptions::new();
        opts.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts
            .open(&lock_path)
            .map_err(|e| anyhow::anyhow!("Failed to open credentials lock: {e}"))?;
        file.lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock credentials file: {e}"))?;
        Ok(file)
    }
}

impl PasswordStore for FilePasswordStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        // Under the same lock as writers; an unlockable directory (read-only)
        // still reads, since the data file is replaced atomically by rename.
        let _lock = self.lock().ok();
        let map = self.read_map()?;
        Ok(map.get(key).cloned())
    }

    fn locator(&self) -> Option<StoreLocator> {
        Some(StoreLocator::File {
            path: self.path.clone(),
            prefix: String::new(),
        })
    }

    fn set(&self, key: &str, password: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut map = self.read_map()?;
        map.insert(key.to_string(), password.to_string());
        self.write_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut map = self.read_map()?;
        if map.remove(key).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }
}

pub struct NoopPasswordStore;

impl PasswordStore for NoopPasswordStore {
    fn get(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn set(&self, _key: &str, _password: &str) -> Result<()> {
        Ok(())
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}

pub fn identity_key(id: i64) -> String {
    format!("identity:{id}")
}

pub fn host_key(id: i64) -> String {
    format!("host:{id}")
}

// ── Gate for synced credentials ──────────────────────────

/// A synced credential that is not open in this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    pub secret: SecretRef,
}

impl std::fmt::Display for Locked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.secret {
            SecretRef::Key(key) => write!(
                f,
                "{key} is a synced credential and is locked: it needs a second share"
            ),
            SecretRef::Lookup(spec) => write!(f, "secret lookup {spec} is locked"),
            SecretRef::Staged(_) | SecretRef::Handle(_) => {
                write!(f, "a staged credential is locked")
            }
        }
    }
}

impl std::error::Error for Locked {}

type SyncedCheck = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Which store keys are synced credentials, and the plaintext of those opened
/// in this process. The plaintext lives in memory only, in [`Zeroizing`]
/// strings; nothing here touches a disk.
#[derive(Default)]
pub struct SecretGate {
    synced: std::sync::RwLock<Option<SyncedCheck>>,
    unlocked: std::sync::Mutex<HashMap<String, zeroize::Zeroizing<String>>>,
    replicator: std::sync::OnceLock<Box<dyn Replicator>>,
}

/// Turns a secret save into a replicated credential when the profile is in a
/// ring: every write that goes through the gated store calls it first.
pub trait Replicator: Send + Sync {
    /// Save `password` (or clear it, `None`) of store key `key` as a synced
    /// credential. `Ok(false)`: not a ring credential, store it locally.
    fn replicate(&self, key: &str, password: Option<&str>) -> Result<bool>;
    /// The key bytes of identity `identity_id` changed: replicate them too.
    fn replicate_identity_key(&self, identity_id: i64) -> Result<bool>;
}

impl SecretGate {
    /// Install the replicator once; a second call is ignored.
    pub fn set_replicator(&self, replicator: Box<dyn Replicator>) {
        let _ = self.replicator.set(replicator);
    }

    pub fn replicator(&self) -> Option<&dyn Replicator> {
        self.replicator.get().map(|r| r.as_ref())
    }

    /// Say which keys are synced credentials. Until this is called nothing is:
    /// every secret is local and passes through.
    pub fn set_synced_check(&self, check: impl Fn(&str) -> bool + Send + Sync + 'static) {
        *self.synced.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(check));
    }

    /// Whether any key can be synced at all. False until the sync stage says
    /// which keys are; lets hot paths skip the gate entirely.
    pub fn guards_any(&self) -> bool {
        self.synced
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn is_synced(&self, key: &str) -> bool {
        self.synced
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|check| check(key))
    }

    /// Open `key` for this process.
    pub fn insert(&self, key: impl Into<String>, value: zeroize::Zeroizing<String>) {
        let hash = value_hash(&value);
        let old = self
            .unlocked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.into(), value);
        open_values_add(hash);
        if let Some(old) = old {
            open_values_remove(value_hash(&old));
        }
    }

    /// The opened plaintext of `key`, if it is open.
    pub fn get(&self, key: &str) -> Option<zeroize::Zeroizing<String>> {
        self.unlocked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// Forget every opened secret (zeroized as they drop).
    pub fn lock_all(&self) {
        let closed: Vec<_> = self
            .unlocked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for (_, value) in closed {
            open_values_remove(value_hash(&value));
        }
    }

    /// The synced credentials among `refs` that are not open yet.
    pub fn locked(&self, refs: &[SecretRef]) -> Vec<SecretRef> {
        refs.iter()
            .filter(|r| match r {
                SecretRef::Key(key) => self.is_synced(key) && self.get(key).is_none(),
                SecretRef::Lookup(_) | SecretRef::Staged(_) | SecretRef::Handle(_) => false,
            })
            .cloned()
            .collect()
    }
}

impl Drop for SecretGate {
    fn drop(&mut self) {
        self.lock_all();
    }
}

// A process-wide count of the opened synced plaintexts, by hash, so a path that
// only holds a secret's value (the askpass staging) can tell it is synced and
// keep it off the disk. Hashes only: no second copy of a plaintext lives here.
static OPEN_VALUES: std::sync::Mutex<Option<HashMap<[u8; 32], usize>>> =
    std::sync::Mutex::new(None);

fn value_hash(value: &str) -> [u8; 32] {
    *blake3::hash(value.as_bytes()).as_bytes()
}

fn open_values_add(hash: [u8; 32]) {
    let mut map = OPEN_VALUES.lock().unwrap_or_else(|e| e.into_inner());
    *map.get_or_insert_with(HashMap::new)
        .entry(hash)
        .or_insert(0) += 1;
}

fn open_values_remove(hash: [u8; 32]) {
    let mut map = OPEN_VALUES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = map.as_mut() {
        if let Some(n) = map.get_mut(&hash) {
            *n -= 1;
            if *n == 0 {
                map.remove(&hash);
            }
        }
    }
}

/// Whether `value` is the plaintext of a synced credential that is open in
/// this process. A local secret equal to one is treated the same: it takes the
/// safer route.
pub fn is_open_synced_value(value: &str) -> bool {
    OPEN_VALUES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|map| map.contains_key(&value_hash(value)))
}

/// Decorator around a profile's store: a synced credential is answered from
/// the gate (`Err(Locked)` until it is open in this process) and never read
/// from or written to the inner store; every other key passes through.
pub struct QuorumGatedStore {
    inner: Box<dyn PasswordStore>,
    gate: std::sync::Arc<SecretGate>,
}

impl QuorumGatedStore {
    /// A gate that guards nothing until [`SecretGate::set_synced_check`].
    pub fn new(inner: Box<dyn PasswordStore>) -> Self {
        Self::with_gate(inner, std::sync::Arc::new(SecretGate::default()))
    }

    pub fn with_gate(inner: Box<dyn PasswordStore>, gate: std::sync::Arc<SecretGate>) -> Self {
        Self { inner, gate }
    }

    /// A handle on the gate, for whoever opens credentials.
    pub fn gate_handle(&self) -> std::sync::Arc<SecretGate> {
        self.gate.clone()
    }
}

impl PasswordStore for QuorumGatedStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        if !self.gate.is_synced(key) {
            return self.inner.get(key);
        }
        match self.gate.get(key) {
            Some(value) => Ok(Some(value.to_string())),
            None => Err(Locked {
                secret: SecretRef::Key(key.to_string()),
            }
            .into()),
        }
    }

    fn set(&self, key: &str, password: &str) -> Result<()> {
        if let Some(r) = self.gate.replicator() {
            if r.replicate(key, Some(password))? {
                // The user just typed it: it is open in this process.
                self.gate
                    .insert(key, zeroize::Zeroizing::new(password.to_string()));
                return Ok(());
            }
        }
        if self.gate.is_synced(key) {
            anyhow::bail!(
                "{key} is a synced credential: change it through sync, not the local store"
            );
        }
        self.inner.set(key, password)
    }

    fn delete(&self, key: &str) -> Result<()> {
        if let Some(r) = self.gate.replicator() {
            if r.replicate(key, None)? {
                self.gate
                    .insert(key, zeroize::Zeroizing::new(String::new()));
                return Ok(());
            }
        }
        if self.gate.is_synced(key) {
            anyhow::bail!(
                "{key} is a synced credential: remove it through sync, not the local store"
            );
        }
        self.inner.delete(key)
    }

    fn lookup(&self, spec: &str) -> Result<String> {
        self.inner.lookup(spec)
    }

    fn locator(&self) -> Option<StoreLocator> {
        self.inner.locator()
    }

    fn gate(&self) -> Option<&SecretGate> {
        Some(&self.gate)
    }

    fn shared_gate(&self) -> Option<std::sync::Arc<SecretGate>> {
        Some(self.gate.clone())
    }
}

/// Why a credential could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockError {
    /// More shares are needed; says where to get them.
    NeedsShares(String),
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for UnlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnlockError::NeedsShares(how) => {
                write!(f, "this credential needs a second share: {how}")
            }
            UnlockError::Cancelled => write!(f, "unlock cancelled"),
            UnlockError::Failed(why) => write!(f, "unlock failed: {why}"),
        }
    }
}

impl std::error::Error for UnlockError {}

/// Opens synced credentials: assembles the shares (or opens the vault) and
/// puts the plaintext into its [`SecretGate`]. Implemented by the sync stage.
pub trait Unlocker: Send + Sync {
    /// The gate of the store the credentials are read from.
    fn gate(&self) -> &SecretGate;
    /// Open `locked`; may block (a CLI waits for shares).
    fn unlock(&self, locked: &[SecretRef]) -> std::result::Result<(), UnlockError>;
}

/// Open every credential in `refs` that is synced and still locked. A pass for
/// nothing locked; an error if the unlocker fails or leaves one closed.
pub fn ensure_unlocked(
    refs: &[SecretRef],
    unlocker: &dyn Unlocker,
) -> std::result::Result<(), UnlockError> {
    let locked = unlocker.gate().locked(refs);
    if locked.is_empty() {
        return Ok(());
    }
    unlocker.unlock(&locked)?;
    let still = unlocker.gate().locked(refs);
    if still.is_empty() {
        Ok(())
    } else {
        Err(UnlockError::Failed(format!(
            "{} credential(s) are still locked",
            still.len()
        )))
    }
}

/// The synced credentials among `refs` that `store` has not opened.
pub fn locked_refs(store: &dyn PasswordStore, refs: &[SecretRef]) -> Vec<SecretRef> {
    store.gate().map(|g| g.locked(refs)).unwrap_or_default()
}

/// What a caller with no way to open credentials says when one is locked.
pub fn needs_share_message() -> UnlockError {
    UnlockError::NeedsShares(
        "open it from the Sync tab (7) of the TUI, with a sync worker running".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Oracle: observed on a host with no session bus (`env -i`, no
    /// `DBUS_SESSION_BUS_ADDRESS`, probed 2026-09-18). There `OsKeyring::set`
    /// fails with "Platform secure storage failure: DBus error: Using X11 for
    /// dbus-daemon autolaunch was disabled at compile time, set your
    /// DBUS_SESSION_BUS_ADDRESS instead: ...", and `get` fails the same way —
    /// yet `check_keyring_available` still reported `true`, so the app picked
    /// the OS keyring and every remember-me write failed closed.
    struct UnreachableKeyring;

    impl PasswordStore for UnreachableKeyring {
        fn get(&self, _key: &str) -> Result<Option<String>> {
            Err(anyhow::anyhow!(
                "keyring: Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn set(&self, _key: &str, _password: &str) -> Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
        fn delete(&self, _key: &str) -> Result<()> {
            Err(anyhow::anyhow!(
                "Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time, set your DBUS_SESSION_BUS_ADDRESS instead"
            ))
        }
    }

    #[derive(Default)]
    struct WorkingStore {
        map: std::sync::Mutex<HashMap<String, String>>,
    }

    impl PasswordStore for WorkingStore {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, password: &str) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_string(), password.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn probe_reports_unwritable_store_as_unavailable() {
        assert!(
            !store_available(&UnreachableKeyring),
            "env -i oracle: writes fail, so the store must not be selected"
        );
    }

    #[test]
    fn probe_reports_working_store_as_available() {
        assert!(store_available(&WorkingStore::default()));
    }

    #[test]
    fn test_file_password_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let store = FilePasswordStore::new(path);

        let test_key = "test:host:123";
        let test_pw = "fallback_secret_pass";

        assert_eq!(store.get(test_key).unwrap(), None);

        store.set(test_key, test_pw).unwrap();

        assert_eq!(store.get(test_key).unwrap(), Some(test_pw.to_string()));

        store.delete(test_key).unwrap();

        assert_eq!(store.get(test_key).unwrap(), None);
    }

    /// Concurrent writers (two sshub processes, or the importer next to the TUI)
    /// must not lose each other's secret: each does read-modify-write of the
    /// whole map. Without the lock the later rename drops the earlier key.
    #[test]
    fn concurrent_sets_keep_every_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    // One store per thread: separate lock-file descriptors, as
                    // separate processes would have (flock is per open file).
                    let store = FilePasswordStore::new(path);
                    for i in 0..10 {
                        store.set(&format!("k{t}-{i}"), "pw").unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let store = FilePasswordStore::new(path);
        for t in 0..8 {
            for i in 0..10 {
                assert_eq!(
                    store.get(&format!("k{t}-{i}")).unwrap().as_deref(),
                    Some("pw"),
                    "k{t}-{i} lost"
                );
            }
        }
    }

    #[test]
    fn secret_lookup_spec_splits_at_the_first_equals() {
        assert_eq!(
            parse_secret_lookup("URL=ssh://a@b:22").unwrap(),
            ("URL", "ssh://a@b:22")
        );
        assert_eq!(
            parse_secret_lookup(" Notes = a=b ").unwrap(),
            ("Notes", "a=b")
        );
        for bad in ["URL", "=x", "URL=", " = "] {
            assert!(
                parse_secret_lookup(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_store_reopens_from_its_locator_with_the_same_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let store = NamespacedPasswordStore::new(
            Box::new(FilePasswordStore::new(dir.path().join("c.json"))),
            "profile:4:",
        );
        store.set("host:1", "pw").unwrap();
        let locator = store.locator().unwrap();
        assert_eq!(
            locator,
            StoreLocator::File {
                path: dir.path().join("c.json"),
                prefix: "profile:4:".into()
            }
        );
        let reopened = locator.open();
        assert_eq!(
            SecretRef::Key("host:1".into()).read(reopened.as_ref()),
            Some("pw".into())
        );
        assert_eq!(
            SecretRef::Key("host:2".into()).read(reopened.as_ref()),
            None
        );
        assert!(NoopPasswordStore.locator().is_none());
    }

    // ── The gate ─────────────────────────────────────────

    fn gated(dir: &std::path::Path) -> QuorumGatedStore {
        let store = QuorumGatedStore::new(Box::new(FilePasswordStore::new(dir.join("c.json"))));
        store
            .gate_handle()
            .set_synced_check(|k| k.starts_with("host:9"));
        store
    }

    #[test]
    fn a_local_key_passes_through_and_a_synced_one_is_locked_until_opened() {
        let dir = tempfile::tempdir().unwrap();
        let store = gated(dir.path());
        store.set("host:1", "local-pw").unwrap();
        assert_eq!(store.get("host:1").unwrap(), Some("local-pw".into()));

        let err = store.get("host:9").unwrap_err();
        let locked = err.downcast_ref::<Locked>().expect("a Locked error");
        assert_eq!(locked.secret, SecretRef::Key("host:9".into()));
        assert!(locked.to_string().contains("second share"));
        // `read` swallows errors: a locked credential reads as nothing.
        assert_eq!(SecretRef::Key("host:9".into()).read(&store), None);

        store
            .gate_handle()
            .insert("host:9", zeroize::Zeroizing::new("synced-pw".into()));
        assert_eq!(store.get("host:9").unwrap(), Some("synced-pw".into()));
        store.gate_handle().lock_all();
        assert!(store.get("host:9").is_err(), "locking again closes it");
    }

    #[test]
    fn a_synced_credential_is_never_written_to_the_local_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = gated(dir.path());
        assert!(store.set("host:9", "TOPSECRET").is_err());
        assert!(store.delete("host:9").is_err());
        store
            .gate_handle()
            .insert("host:9", zeroize::Zeroizing::new("TOPSECRET".into()));
        let _ = store.get("host:9");
        store.set("host:1", "other").unwrap();
        // Nothing under the store's directory carries the plaintext.
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let text = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !String::from_utf8_lossy(&text).contains("TOPSECRET"),
                "plaintext reached the disk"
            );
        }
    }

    #[test]
    fn a_store_that_guards_nothing_costs_nothing_and_passes_everything() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            QuorumGatedStore::new(Box::new(FilePasswordStore::new(dir.path().join("c.json"))));
        assert!(!store.gate().unwrap().guards_any());
        store.set("host:9", "pw").unwrap();
        assert_eq!(store.get("host:9").unwrap(), Some("pw".into()));
        assert!(locked_refs(&store, &[SecretRef::Key("host:9".into())]).is_empty());
        assert!(NoopPasswordStore.gate().is_none());
    }

    #[test]
    fn a_staged_value_reads_without_a_store_and_never_prints() {
        let r = SecretRef::Staged(StagedValue::new("hunter2".into()));
        assert_eq!(r.read(&NoopPasswordStore), Some("hunter2".into()));
        assert!(!format!("{r:?}").contains("hunter2"));
    }

    struct Opener {
        gate: std::sync::Arc<SecretGate>,
        calls: std::sync::atomic::AtomicUsize,
        open: bool,
    }

    impl Unlocker for Opener {
        fn gate(&self) -> &SecretGate {
            &self.gate
        }
        fn unlock(&self, locked: &[SecretRef]) -> std::result::Result<(), UnlockError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if !self.open {
                return Err(UnlockError::Cancelled);
            }
            for r in locked {
                if let SecretRef::Key(k) = r {
                    self.gate
                        .insert(k.clone(), zeroize::Zeroizing::new("pw".into()));
                }
            }
            Ok(())
        }
    }

    fn opener(open: bool) -> Opener {
        let gate = std::sync::Arc::new(SecretGate::default());
        gate.set_synced_check(|k| k.starts_with("host:"));
        Opener {
            gate,
            calls: Default::default(),
            open,
        }
    }

    #[test]
    fn ensure_unlocked_opens_only_what_is_locked_and_checks_the_result() {
        use std::sync::atomic::Ordering::SeqCst;
        let refs = [
            SecretRef::Key("host:1".into()),
            SecretRef::Key("identity:2".into()), // local: never asked about
        ];
        let u = opener(true);
        ensure_unlocked(&refs, &u).unwrap();
        assert_eq!(u.calls.load(SeqCst), 1);
        assert!(u.gate.get("host:1").is_some() && u.gate.get("identity:2").is_none());
        ensure_unlocked(&refs, &u).unwrap();
        assert_eq!(
            u.calls.load(SeqCst),
            1,
            "already open: the unlocker is not asked"
        );

        let refused = opener(false);
        assert_eq!(
            ensure_unlocked(&refs, &refused),
            Err(UnlockError::Cancelled)
        );

        struct Liar(std::sync::Arc<SecretGate>);
        impl Unlocker for Liar {
            fn gate(&self) -> &SecretGate {
                &self.0
            }
            fn unlock(&self, _: &[SecretRef]) -> std::result::Result<(), UnlockError> {
                Ok(()) // claims success, opens nothing
            }
        }
        let liar = Liar(refused.gate.clone());
        assert!(matches!(
            ensure_unlocked(&refs, &liar),
            Err(UnlockError::Failed(_))
        ));
        assert!(needs_share_message().to_string().contains("second share"));
    }
}
