//! The ways a second share reaches a terminal that do not go through the op-log: the phone's
//! reply (QR, clipboard, audio), a short-code snapshot, and a share typed as 24 words.
//!
//! * Phone reply: bytes of a `ShareResponseWire`; the clipboard path is the base64 of exactly those
//!   bytes, as the page copies them. A reply that carries the courier extension must be signed by
//!   the phone over the challenge's binding preimage, otherwise the whole reply is refused.
//! * Short code: the 16 Crockford characters open the snapshot blob this terminal stored from an
//!   earlier reply extension (`sync_meta` `snapshot:<host uid hex>`, local only, never replicated).
//! * Full entry: the 32 share bytes as 24 BIP39 words; the share index is not typed, it is the
//!   holder's position in the sharing of that epoch.

use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::codes::normalize_code;
use super::crypto::{bip39_decode, Share};
use super::oplog::{DeviceId, HostUid};
use super::protocol::RoundSummary;
use super::quorum::{holder_index, meta_get, meta_set, sharing_holders, Challenge};
use super::share_release::{unwrap_share_reply, ReplyExt};
use super::limits::{SNAPSHOT_TTL_SECS, TIMESTAMP_SKEW_GRACE_SECS};
use super::snapshot::{snapshot_open_for, SnapshotBlob};
use super::wire::ShareResponseWire;
use crate::store::{uid_hex, LauncherStore};

/// A phone reply that passed its checks.
#[derive(Debug, PartialEq, Eq)]
pub struct PhoneReply {
    pub share: Share,
    pub ext: Option<ReplyExt>,
}

/// Opens the bytes of a `ShareResponseWire` for `challenge`. `phone_ed25519` is the phone's signing
/// key as the ring knows it. A reply with an extension must carry the phone's approval signature.
pub fn open_phone_reply(
    bytes: &[u8],
    challenge: &Challenge,
    eph_secret: &[u8; 32],
    phone_ed25519: &[u8; 32],
) -> Result<PhoneReply> {
    let wire = ShareResponseWire::decode(bytes)?;
    ensure!(
        wire.request_id == challenge.request_id,
        "this reply answers another request"
    );
    let (share, ext) = unwrap_share_reply(eph_secret, &wire.ciphertext)?;
    if let Some(ext) = &ext {
        ensure!(
            ext.approved_by(phone_ed25519, &challenge.binding_preimage()),
            "the reply carries an extension the phone did not sign for this challenge"
        );
    }
    Ok(PhoneReply { share, ext })
}

/// The clipboard path: the user pastes the base64 text the page copied (whitespace is ignored).
pub fn open_phone_reply_clipboard(
    text: &str,
    challenge: &Challenge,
    eph_secret: &[u8; 32],
    phone_ed25519: &[u8; 32],
) -> Result<PhoneReply> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = B64
        .decode(compact)
        .context("the pasted text is not base64")?;
    open_phone_reply(&bytes, challenge, eph_secret, phone_ed25519)
}

fn snapshot_key(host: &HostUid) -> String {
    format!("snapshot:{}", uid_hex(host))
}

/// Keeps the snapshot blob of an earlier reply for the short-code path. A version 2 blob must name this
/// credential, and a second blob is refused while the first is live: the phone issues one at a time, so
/// two live codes for one credential mean a restored or hostile phone (use [`forget_snapshot`] after a
/// reissue of the credential, or let the first expire).
pub fn remember_snapshot(store: &LauncherStore, host: &HostUid, blob: &[u8]) -> Result<()> {
    remember_snapshot_at(store, host, blob, crate::store::now_ts())
}

pub(crate) fn remember_snapshot_at(
    store: &LauncherStore,
    host: &HostUid,
    blob: &[u8],
    now: i64,
) -> Result<()> {
    let new = SnapshotBlob::from_bytes(blob)?;
    ensure!(
        new.host_uid.is_none_or(|h| h == *host),
        "the snapshot is for another credential"
    );
    if let Some(old) = meta_get(store, &snapshot_key(host))? {
        if old != blob {
            if let Ok(old) = SnapshotBlob::from_bytes(&old) {
                ensure!(
                    now - old.issued_at > SNAPSHOT_TTL_SECS || old.issued_at > now + TIMESTAMP_SKEW_GRACE_SECS,
                    "a live short-code snapshot already exists for this credential: the phone issues one at a time"
                );
            }
        }
    }
    meta_set(store, &snapshot_key(host), blob)
}

/// Drops the stored snapshot of `host` (its credential was reissued, or the code was used).
pub fn forget_snapshot(store: &LauncherStore, host: &HostUid) -> Result<()> {
    meta_set(store, &snapshot_key(host), b"")
}

/// What to do with a verified reply extension: chain the courier ops and keep the snapshot.
pub fn apply_reply_ext(store: &LauncherStore, ext: &ReplyExt) -> Result<RoundSummary> {
    let summary = super::hub::courier_ingest(store, &ext.ops)?;
    if let Some((host, blob)) = &ext.snapshot {
        remember_snapshot(store, host, blob)?;
    }
    Ok(summary)
}

/// The short-code path: opens the stored snapshot of `host` with the 16 characters the user typed.
pub fn share_from_code(
    store: &LauncherStore,
    host: &HostUid,
    code: &str,
    now: i64,
) -> Result<Share> {
    let blob = meta_get(store, &snapshot_key(host))?
        .filter(|b| !b.is_empty())
        .context("no snapshot is stored for this credential: unlock with the phone once first")?;
    snapshot_open_for(
        &SnapshotBlob::from_bytes(&blob)?,
        host,
        &normalize_code(code)?,
        now,
    )
}

/// The full-entry path: 24 words are the 32 share bytes; `holder` is the device whose share this is
/// (the phone), whose position in the sharing of `epoch` is the index.
pub fn full_entry_share(
    store: &LauncherStore,
    host: &HostUid,
    epoch: u64,
    holder: &DeviceId,
    words: &str,
) -> Result<Share> {
    let bytes = bip39_decode(words)?;
    let holders = sharing_holders(store, host, epoch)?;
    let Some(index) = holder_index(&holders, holder) else {
        bail!("{holder} holds no share of this credential at epoch {epoch}");
    };
    Ok(Share {
        index,
        bytes: bytes.to_vec(),
    })
}

/// The kind 5 paste path (no camera): the phone's ops list as the page copies it (base64 of the whole
/// frame) goes through the courier ingest like a reply extension, so the ops reach every peer with the
/// next round. Malformed text or frames are errors and change nothing.
pub fn phone_ops_from_text(store: &LauncherStore, text: &str) -> Result<RoundSummary> {
    let bytes = B64
        .decode(text.split_whitespace().collect::<String>())
        .context("the pasted text is not base64")?;
    let wire = super::wire::PhoneOpsWire::decode(&bytes)?;
    super::hub::courier_ingest(store, &wire.ops)
}

/// The paper path: the 24 typed paper words are the paper's key; its share of `host` at `epoch` (the
/// credential op or a reshare) is the second share next to this device's own, so a lone terminal with
/// the paper opens a credential without any other holder. The paper is a registered recipient
/// (`FactorRegister { Paper }` replicates its public key), so the share is in the log.
pub fn paper_second_share(
    store: &LauncherStore,
    host: &HostUid,
    epoch: u64,
    phrase: &str,
) -> Result<Share> {
    let paper = super::crypto::paper_derive(phrase, super::crypto::PaperDomain::Unwrap)?;
    super::quorum::own_shares(store, &paper, host)?
        .into_iter()
        .find(|(e, _)| *e == epoch)
        .map(|(_, s)| s)
        .context("the paper holds no share of this credential at that epoch: wrong words, or the paper was not registered when it was made")
}

#[cfg(test)]
mod tests;
