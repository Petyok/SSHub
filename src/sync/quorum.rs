//! Access to synced credentials: the challenge a terminal shows a phone, the shares a credential
//! is split into, the vault recipient, and the `Unlocker` that feeds `credentials::SecretGate`.
//!
//! Share indices are not stored anywhere: a sharing is a sorted list of holder ids and a holder's
//! index is its 1-based position in that list. The list of the original sharing is the recipient
//! list of the `CredUpdate`; the list of a reshare epoch is `new_holders` of its batch op. So a
//! receiver of a sub-share can compute the contributor's index and its own without trusting
//! anything it cannot check.

use std::sync::Arc;

use anyhow::{anyhow, bail, ensure, Context, Result};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};
use zeroize::Zeroizing;

use super::crypto::{
    dek_open, derive_key, hybrid_unwrap, hybrid_wrap, paper_derive, vault_recipient_with,
    Argon2Profile, DeviceIdentity, DevicePublic, PaperDomain, Share, ALG_CURRENT, ARGON2_MASTER,
};
use super::oplog::{
    CredMode, DeviceId, DeviceRole, DeviceStatus, HostUid, Op, OpBody, WrappedShare,
};
use super::share_release::{combine_and_check, open_envelope_share, parse_wrapped_share_plaintext};
use super::wire::{ChallengeWire, PhoneMaterial, ShareKind};
use crate::credentials::{SecretGate, SecretRef, UnlockError, Unlocker};
use crate::store::LauncherStore;

/// The product minimum: two shares open a credential.
pub const SHARE_K: u8 = 2;

pub enum UnlockPath {
    Peer(DeviceId),
    PhoneAudio,
    PhoneReverseQr,
    PhoneClipboard,
    ShortCode,
    FullEntry,
}

/// What a terminal shows a phone: the whole `ShareRelease` op and the material the phone needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub op: Op,
    pub nonce: [u8; 32],
    pub expiry: i64,
    pub eph_x25519_pub: [u8; 32],
    pub request_id: u64,
    pub phone_material: PhoneMaterial,
}

impl Challenge {
    pub fn to_wire(&self) -> ChallengeWire {
        ChallengeWire {
            op_canonical: self.op.canonical_bytes(),
            op_signature: self.op.signature,
            nonce: self.nonce,
            expiry: self.expiry,
            eph_x25519_pub: self.eph_x25519_pub,
            request_id: self.request_id,
            phone_material: self.phone_material.clone(),
        }
    }

    /// Fountain frames of the wire form, `frame_bytes` each (see `fountain::frame_bytes_for_area`).
    pub fn to_qr_payload(&self, frame_bytes: usize) -> Result<Vec<Vec<u8>>> {
        self.to_wire().to_qr_frames(frame_bytes)
    }

    pub fn binding_hash(&self) -> [u8; 32] {
        self.to_wire().binding_hash()
    }

    /// The bytes the phone signs.
    pub fn binding_preimage(&self) -> Vec<u8> {
        let w = self.to_wire();
        super::wire::binding_preimage(
            &w.op_canonical,
            &w.nonce,
            w.expiry,
            &w.eph_x25519_pub,
            w.request_id,
            &w.phone_material.hash(),
        )
    }
}

// ------------------------------------------------------------------ store helpers

pub(crate) fn meta_get(store: &LauncherStore, key: &str) -> Result<Option<Vec<u8>>> {
    store.with_conn(|c| {
        Ok(c.query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?)
    })
}

pub(crate) fn meta_set(store: &LauncherStore, key: &str, value: &[u8]) -> Result<()> {
    store.with_conn(|c| {
        c.execute(
            "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    })
}

/// Applied or approved ops of one kind, oldest first, optionally by one author.
pub(crate) fn ops_of(
    store: &LauncherStore,
    kind: &str,
    author: Option<&DeviceId>,
) -> Result<Vec<Op>> {
    store.with_conn(|c| {
        let mut st = c.prepare(
            "SELECT canonical, signature FROM sync_ops
             WHERE kind = ?1 AND (?2 IS NULL OR device_id = ?2) AND state IN ('applied','approved')
             ORDER BY lamport, device_id, seq",
        )?;
        let rows = st
            .query_map(params![kind, author], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(c, s)| {
                Op::from_canonical(&c, s.try_into().map_err(|_| anyhow!("stored signature"))?)
            })
            .collect()
    })
}

fn pub_to_bytes(p: &DevicePublic) -> Vec<u8> {
    [p.ed25519_pub.as_slice(), &p.x25519_pub, &p.mlkem_pub].concat()
}

fn pub_from_bytes(b: &[u8]) -> Result<DevicePublic> {
    ensure!(b.len() > 64, "stored public key is too short");
    let p = DevicePublic {
        ed25519_pub: b[..32].try_into()?,
        x25519_pub: b[32..64].try_into()?,
        mlkem_pub: b[64..].to_vec(),
    };
    p.validate()?;
    Ok(p)
}

// ------------------------------------------------------------------ vault

/// The ring's Argon2 salt: derived from the ring secret, so a new machine needs nothing but the kit.
pub fn vault_salt(ring_secret: &[u8; 16]) -> [u8; 16] {
    derive_key("sshub/sync/vault/salt/v1", ring_secret)[..16]
        .try_into()
        .unwrap()
}

/// The text on the printed ring-secret sheet (its QR and the hex groups encode the same thing):
/// `sshub:ring:1:` and 32 lowercase hex digits.
pub fn ring_secret_text(ring_secret: &[u8; 16]) -> String {
    format!("sshub:ring:1:{}", hex::encode(ring_secret))
}

/// Parses the ring-secret sheet payload, as a QR scan or typed hex groups (spaces and dashes in
/// the hex part are ignored, the prefix is optional when typing). Anything else is refused.
pub fn parse_ring_secret(text: &str) -> Result<[u8; 16]> {
    let t = text.trim();
    let hex_part = t.strip_prefix("sshub:ring:1:").unwrap_or(t);
    ensure!(
        !hex_part.starts_with("sshub:"),
        "this is not a version 1 ring secret"
    );
    let digits: String = hex_part
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    ensure!(digits.len() == 32, "a ring secret is 32 hex digits");
    ensure!(
        digits.chars().all(|c| c.is_ascii_hexdigit()),
        "a ring secret is hexadecimal"
    );
    let bytes = hex::decode(digits.to_ascii_lowercase())?;
    Ok(bytes.try_into().expect("16 bytes"))
}

/// The vault recipient's public key as this device knows it. LOCAL only (`sync_meta`, never
/// replicated): a replicated vault key would let one device redirect every new credential to itself.
pub fn vault_pub(store: &LauncherStore) -> Result<Option<DevicePublic>> {
    meta_get(store, "vault_pub")?
        .map(|b| pub_from_bytes(&b))
        .transpose()
}

/// Ring setup (or joining a ring with the kit): derive the vault recipient from the master phrase
/// and the ring secret and keep its public half locally. Consumes `SyncIntent::OnboardingDone`.
pub fn vault_setup(
    store: &LauncherStore,
    master_phrase: &str,
    ring_secret: &[u8; 16],
) -> Result<DevicePublic> {
    super::crypto::eff_decode(master_phrase)
        .context("the master phrase is not 6 words of the EFF list")?;
    vault_setup_with(store, master_phrase, ring_secret, ARGON2_MASTER)
}

pub(crate) fn vault_setup_with(
    store: &LauncherStore,
    phrase: &str,
    ring_secret: &[u8; 16],
    profile: Argon2Profile,
) -> Result<DevicePublic> {
    let (public, _) = vault_recipient_with(phrase, ring_secret, &vault_salt(ring_secret), profile);
    meta_set(store, "vault_pub", &pub_to_bytes(&public))?;
    Ok(public)
}

/// Opens the DEK of a vault-mode credential with the master phrase and the ring secret.
pub fn vault_open_dek(
    store: &LauncherStore,
    host: &HostUid,
    master: &str,
    ring_secret: &[u8; 16],
) -> Result<[u8; 32]> {
    vault_open_dek_with(store, host, master, ring_secret, ARGON2_MASTER)
}

pub(crate) fn vault_open_dek_with(
    store: &LauncherStore,
    host: &HostUid,
    master: &str,
    ring_secret: &[u8; 16],
    profile: Argon2Profile,
) -> Result<[u8; 32]> {
    let (ciphertext, wraps) = store
        .sync_cred_ciphertext(host)?
        .context("no such credential")?;
    let (public, identity) =
        vault_recipient_with(master, ring_secret, &vault_salt(ring_secret), profile);
    let id = public.device_id();
    let wrap = wraps.iter().find(|w| w.recipient == id).context("this credential is not wrapped to this vault (wrong phrase, wrong ring secret, or a quorum credential)")?;
    let plain = Zeroizing::new(hybrid_unwrap(&identity, &wrap.sealed)?);
    ensure!(
        plain.len() == 48 && plain[..16] == host[..],
        "the vault blob names another credential"
    );
    let dek: [u8; 32] = plain[16..].try_into().unwrap();
    dek_open(&dek, &ciphertext)
        .map_err(|_| anyhow!("the vault key does not open this credential"))?;
    Ok(dek)
}

/// The vault wrap of one credential's DEK.
pub(crate) fn vault_wrap(
    vault: &DevicePublic,
    host: &HostUid,
    dek: &[u8; 32],
) -> Result<WrappedShare> {
    let sealed = hybrid_wrap(vault, &[host.as_slice(), dek.as_slice()].concat());
    Ok(WrappedShare {
        recipient: vault.device_id(),
        alg: ALG_CURRENT,
        sealed,
    })
}

// ------------------------------------------------------------------ paper

/// Register the paper phrase's unwrap key as a permanent share recipient (local record; the paper
/// is a recipient of every credential made afterwards).
pub fn paper_setup(store: &LauncherStore, phrase: &str) -> Result<DevicePublic> {
    let public = paper_derive(phrase, PaperDomain::Unwrap)?.public();
    meta_set(store, "paper_pub", &pub_to_bytes(&public))?;
    Ok(public)
}

pub fn paper_pub(store: &LauncherStore) -> Result<Option<DevicePublic>> {
    meta_get(store, "paper_pub")?
        .map(|b| pub_from_bytes(&b))
        .transpose()
}

// ------------------------------------------------------------------ sharings

/// Holders that receive a share of a new credential: devices with the holder role that are neither
/// revoked nor compromised, plus the paper recipient, sorted by id.
pub fn share_recipients(
    store: &LauncherStore,
    me: &DevicePublic,
) -> Result<Vec<(DeviceId, DevicePublic)>> {
    let mut out: Vec<(DeviceId, DevicePublic)> = store
        .sync_devices()?
        .into_iter()
        .filter(|d| {
            d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .map(|d| (d.id, d.public))
        .collect();
    // A device that never wrote anything is not in its own table yet; it is a holder of its ring.
    if !out.iter().any(|(id, _)| *id == me.device_id())
        && !store.sync_devices()?.iter().any(|d| d.id == me.device_id())
    {
        out.push((me.device_id(), me.clone()));
    }
    if let Some(p) = paper_pub(store)? {
        if !out.iter().any(|(id, _)| *id == p.device_id()) {
            out.push((p.device_id(), p));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// 1-based position of `who` in a sorted holder list.
pub fn holder_index(holders: &[DeviceId], who: &DeviceId) -> Option<u8> {
    holders
        .iter()
        .position(|h| h == who)
        .and_then(|i| u8::try_from(i + 1).ok())
}

/// The sorted holder list of a credential's sharing at `epoch` (0: the credential op itself).
pub fn sharing_holders(store: &LauncherStore, host: &HostUid, epoch: u64) -> Result<Vec<DeviceId>> {
    let mut ids: Vec<DeviceId> = if epoch == 0 {
        let (_, wraps) = store
            .sync_cred_ciphertext(host)?
            .context("no such credential")?;
        let vault = vault_pub(store)?.map(|v| v.device_id());
        wraps
            .into_iter()
            .map(|w| w.recipient)
            .filter(|r| Some(r) != vault.as_ref())
            .collect()
    } else {
        ops_of(store, "ShareReshareBatch", None)?
            .into_iter()
            .find_map(|op| match op.body {
                OpBody::ShareReshareBatch {
                    epoch: e,
                    new_holders,
                    items,
                } if e == epoch && items.iter().any(|i| i.host_uid == *host) => Some(new_holders),
                _ => None,
            })
            .with_context(|| format!("no reshare of this credential at epoch {epoch}"))?
    };
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn envelope_aad(host: &HostUid, epoch: u64, index: u8) -> Vec<u8> {
    [host.as_slice(), &epoch.to_be_bytes(), &[index]].concat()
}

/// The storage key of `epoch` that `id` published (`ShareStorageKey`), opened with its wrap keys.
fn storage_key(store: &LauncherStore, id: &DeviceIdentity, epoch: u64) -> Result<[u8; 32]> {
    let me = id.public().device_id();
    for op in ops_of(store, "ShareStorageKey", Some(&me))? {
        if let OpBody::ShareStorageKey { epoch: e, sealed } = &op.body {
            if *e == epoch {
                let k = Zeroizing::new(hybrid_unwrap(id, sealed)?);
                return k
                    .as_slice()
                    .try_into()
                    .map_err(|_| anyhow!("storage key has the wrong length"));
            }
        }
    }
    bail!("no storage key for epoch {epoch}")
}

/// This holder's shares of `host`, newest sharing first, epoch 0 (the credential op) last. A
/// reshare is adopted only after a real unlock succeeded under it, so a caller tries them in
/// order and falls back.
pub fn own_shares(
    store: &LauncherStore,
    id: &DeviceIdentity,
    host: &HostUid,
) -> Result<Vec<(u64, Share)>> {
    let me = id.public().device_id();
    let mut out = Vec::new();
    for op in ops_of(store, "ShareReshareConfirmBatch", Some(&me))?
        .into_iter()
        .rev()
    {
        let OpBody::ShareReshareConfirmBatch { epoch, items } = &op.body else {
            continue;
        };
        let Some(item) = items.iter().find(|i| i.host_uid == *host) else {
            continue;
        };
        let Ok(holders) = sharing_holders(store, host, *epoch) else {
            continue;
        };
        let Some(index) = holder_index(&holders, &me) else {
            continue;
        };
        let Ok(key) = storage_key(store, id, *epoch) else {
            continue;
        };
        if let Ok(share) =
            open_envelope_share(&key, &envelope_aad(host, *epoch, index), &item.sealed_share)
        {
            if share.index == index {
                out.push((*epoch, share));
            }
        }
    }
    // The paper adopts nothing: at the epochs it is a holder of, rebuild its share from the
    // sub-shares the contributors sealed to its permanent key.
    for e in super::reshare::reshare_epochs(store)? {
        if !out.iter().any(|(x, _)| *x == e) {
            if let Ok(share) = super::reshare::paper_share_at(store, id, host, e) {
                out.push((e, share));
            }
        }
    }
    out.sort_by_key(|(e, _)| std::cmp::Reverse(*e));
    out.dedup_by_key(|(e, _)| *e);
    if let Some((_, wraps)) = store.sync_cred_ciphertext(host)? {
        if let Some(w) = wraps.iter().find(|w| w.recipient == me) {
            let plain = Zeroizing::new(hybrid_unwrap(id, &w.sealed)?);
            out.push((0, parse_wrapped_share_plaintext(host, &plain)?));
        }
    }
    ensure!(
        !out.is_empty(),
        "this device holds no share of that credential"
    );
    Ok(out)
}

/// The newest share only.
pub fn own_share(store: &LauncherStore, id: &DeviceIdentity, host: &HostUid) -> Result<Share> {
    Ok(own_shares(store, id, host)?.remove(0).1)
}

/// Two shares of the same sharing, combined in memory and checked by opening the credential.
pub fn assemble_dek(
    store: &LauncherStore,
    host: &HostUid,
    my_share: Share,
    second: Share,
) -> Result<[u8; 32]> {
    let (ciphertext, _) = store
        .sync_cred_ciphertext(host)?
        .context("no such credential")?;
    combine_and_check(&my_share, &second, &ciphertext).map_err(|_| {
        anyhow!(
            "the two shares do not open this credential (wrong, stale or from different sharings)"
        )
    })
}

/// What a phone needs to recover its own share of `host` at the newest sharing it can serve.
pub fn phone_material(
    store: &LauncherStore,
    host: &HostUid,
    phone: &DeviceId,
) -> Result<PhoneMaterial> {
    for op in ops_of(store, "ShareReshareConfirmBatch", Some(phone))?
        .into_iter()
        .rev()
    {
        let OpBody::ShareReshareConfirmBatch { epoch, items } = &op.body else {
            continue;
        };
        let Some(item) = items.iter().find(|i| i.host_uid == *host) else {
            continue;
        };
        let storage = ops_of(store, "ShareStorageKey", Some(phone))?
            .into_iter()
            .find_map(|o| match o.body {
                OpBody::ShareStorageKey { epoch: e, sealed } if e == *epoch => Some(sealed),
                _ => None,
            });
        if let Some(storage_blob) = storage {
            return Ok(PhoneMaterial {
                kind: ShareKind::Envelope,
                epoch: *epoch,
                share_blob: item.sealed_share.clone(),
                storage_blob,
            });
        }
    }
    let (_, wraps) = store
        .sync_cred_ciphertext(host)?
        .context("no such credential")?;
    let w = wraps
        .iter()
        .find(|w| w.recipient == *phone)
        .context("the phone holds no share of this credential")?;
    Ok(PhoneMaterial {
        kind: ShareKind::Hybrid,
        epoch: 0,
        share_blob: w.sealed.clone(),
        storage_blob: Vec::new(),
    })
}

/// A terminal asks for a release: appends its `ShareRelease` op and returns the challenge for the
/// phone together with the ephemeral secret that opens the answer.
pub fn make_challenge(
    store: &LauncherStore,
    id: &DeviceIdentity,
    host: &HostUid,
    phone: &DeviceId,
    now: i64,
    ttl_secs: i64,
) -> Result<(Challenge, Zeroizing<[u8; 32]>)> {
    let material = phone_material(store, host, phone)?;
    let me = id.public().device_id();
    let mut rid = [0u8; 8];
    OsRng.fill_bytes(&mut rid);
    let request_id = u64::from_be_bytes(rid) >> 11; // within the integer range every language keeps exactly
    let op = store.sync_append_local(
        id,
        OpBody::ShareRelease {
            host_uid: *host,
            request_id,
            requester: me,
        },
    )?;
    let mut secret = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *secret);
    let eph = XPublic::from(&XSecret::from(*secret)).to_bytes();
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    Ok((
        Challenge {
            op,
            nonce,
            expiry: now + ttl_secs,
            eph_x25519_pub: eph,
            request_id,
            phone_material: material,
        },
        secret,
    ))
}

// ------------------------------------------------------------------ unlocking

/// Where the second share (or the master phrase) comes from. Implemented by whoever owns the
/// interaction: the TUI worker (peer round, phone QR, short code) or a CLI prompt.
pub trait ShareSource: Send + Sync {
    /// The master phrase for a vault-mode credential, typed by the user.
    fn master_phrase(&self) -> Option<Zeroizing<String>>;
    /// A share of `host` from another holder at `epoch`'s sharing (0 = the credential op).
    fn second_share(
        &self,
        host: &HostUid,
        epoch: u64,
        mine: &Share,
    ) -> std::result::Result<Share, UnlockError>;
}

/// `credentials::Unlocker` for a profile with sync: opens synced credentials into the gate.
pub struct QuorumUnlocker {
    store: Arc<LauncherStore>,
    id: Arc<DeviceIdentity>,
    gate: Arc<SecretGate>,
    source: Box<dyn ShareSource>,
    ring_secret: Option<Zeroizing<[u8; 16]>>,
    argon: Argon2Profile,
}

impl QuorumUnlocker {
    pub fn new(
        store: Arc<LauncherStore>,
        id: Arc<DeviceIdentity>,
        gate: Arc<SecretGate>,
        source: Box<dyn ShareSource>,
        ring_secret: Option<[u8; 16]>,
    ) -> Self {
        Self::with_profile(store, id, gate, source, ring_secret, ARGON2_MASTER)
    }

    pub(crate) fn with_profile(
        store: Arc<LauncherStore>,
        id: Arc<DeviceIdentity>,
        gate: Arc<SecretGate>,
        source: Box<dyn ShareSource>,
        ring_secret: Option<[u8; 16]>,
        argon: Argon2Profile,
    ) -> Self {
        let synced = store.clone();
        gate.set_synced_check(move |key| {
            secret_uid(&synced, key)
                .is_some_and(|u| synced.sync_cred_ciphertext(&u).ok().flatten().is_some())
        });
        Self {
            store,
            id,
            gate,
            source,
            ring_secret: ring_secret.map(Zeroizing::new),
            argon,
        }
    }

    fn open_one(&self, host: &HostUid) -> std::result::Result<Zeroizing<Vec<u8>>, UnlockError> {
        let fail = |e: anyhow::Error| UnlockError::Failed(format!("{e:#}"));
        let (ciphertext, wraps) = self
            .store
            .sync_cred_ciphertext(host)
            .map_err(fail)?
            .ok_or_else(|| UnlockError::Failed("no such credential".into()))?;
        let vault = vault_pub(&self.store).map_err(fail)?.map(|v| v.device_id());
        let is_vault = self.store.sync_cred_mode().map_err(fail)? == CredMode::Vault
            || vault
                .as_ref()
                .is_some_and(|v| wraps.iter().any(|w| w.recipient == *v));
        let dek = if is_vault {
            let ring = self.ring_secret.as_ref().ok_or_else(|| {
                UnlockError::NeedsShares("this device has no ring secret: import the kit".into())
            })?;
            let phrase = self.source.master_phrase().ok_or(UnlockError::Cancelled)?;
            vault_open_dek_with(&self.store, host, &phrase, ring, self.argon).map_err(fail)?
        } else {
            let mine = own_shares(&self.store, &self.id, host).map_err(fail)?;
            let mut last = UnlockError::NeedsShares("another holder must release a share".into());
            let mut found = None;
            for (epoch, share) in mine {
                match self.source.second_share(host, epoch, &share) {
                    Ok(second) => match assemble_dek(&self.store, host, share, second) {
                        Ok(dek) => {
                            found = Some((epoch, dek));
                            break;
                        }
                        Err(e) => last = fail(e),
                    },
                    Err(e) => last = e,
                }
            }
            let (epoch, dek) = found.ok_or(last)?;
            if epoch > 0 {
                super::reshare::mark_adopted(&self.store, host, epoch).map_err(fail)?;
            }
            dek
        };
        dek_open(&dek, &ciphertext)
            .map(Zeroizing::new)
            .map_err(|e| UnlockError::Failed(e.to_string()))
    }
}

/// Which synced credential a store key names: `host:<row id>` or `identity:<row id>`.
pub fn secret_uid(store: &LauncherStore, key: &str) -> Option<HostUid> {
    if let Some(id) = key.strip_prefix("host:") {
        return store.host_uid(id.parse().ok()?).ok().flatten();
    }
    if let Some(id) = key.strip_prefix("identity:") {
        return store.identity_uid(id.parse().ok()?).ok().flatten();
    }
    None
}

impl Unlocker for QuorumUnlocker {
    fn gate(&self) -> &SecretGate {
        &self.gate
    }

    fn unlock(&self, locked: &[SecretRef]) -> std::result::Result<(), UnlockError> {
        for r in locked {
            let SecretRef::Key(key) = r else { continue };
            let uid = secret_uid(&self.store, key)
                .ok_or_else(|| UnlockError::Failed(format!("{key} is not a synced credential")))?;
            let plain = self.open_one(&uid)?;
            let payload = super::cred_ops::CredPayload::from_bytes(&plain)
                .map_err(|e| UnlockError::Failed(e.to_string()))?;
            // The gate holds the password the TUI asks the store for; an identity key travels
            // separately through `cred_ops::materialise`.
            self.gate.insert(
                key.clone(),
                Zeroizing::new(payload.password.clone().unwrap_or_default()),
            );
        }
        Ok(())
    }
}

/// Releases `share` to a requester's ephemeral key after the holder's user approved.
pub fn release_share(
    store: &LauncherStore,
    id: &DeviceIdentity,
    host: &HostUid,
    epoch: u64,
    eph_x25519_pub: &[u8; 32],
) -> Result<Vec<u8>> {
    let shares = own_shares(store, id, host)?;
    let (_, share) = shares
        .into_iter()
        .find(|(e, _)| *e == epoch)
        .context("this device holds no share at that epoch")?;
    Ok(super::share_release::seal_share_response(
        eph_x25519_pub,
        &share,
    ))
}

#[cfg(test)]
mod tests;
