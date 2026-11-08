//! The phone as a RECEIVER of a reshare (C14), mirroring `reissue_receive` of `src/sync/reshare.rs`. Port of
//! `assets/companion/src/reshare.mjs` plus the reshare and rotate screens of `app.mjs`.
//!
//! The ops of the epoch arrive as a kind 5 list (the terminal shows or pastes them). The phone opens the session keys
//! sealed to it, opens its sub-share of every credential from at least two contributors, checks each against its
//! commitment, combines them into its NEW share and seals that under an epoch storage key. It never reconstructs a
//! secret and holds no share afterwards: the sealed result is what the ring replicates.
//!
//! Sub-share plaintext is `old_epoch u64 BE || from index || 32 bytes`, sealed with XChaCha20-Poly1305 under the session
//! key with associated data `host_uid || epoch u64 BE || contributor id || phone id`. A confirm item is
//! `aead_seal(storage key, host_uid || epoch u64 BE || own index, index || share)` (`nonce(24) || ct`, the layout of
//! `sshub_sync_proto::crypto::aead_seal`, with the nonce drawn from the injected RNG).
//!
//! Rules carried over from the review rounds:
//! - only contributions of authors whose signature this phone VERIFIED (a key it enrolled with) are combined, and at
//!   least `SHARE_K` of them per credential (RP1: the k check counts the very entries that are combined);
//! - contributors are counted by the DEVICE that authored the batch, never by the byte inside the sealed text (S2);
//! - holder ids are 16 lower-case hex characters (PF5); the epoch is a u64 by type (PF4);
//! - session keys open through `PhoneIdentity::unwrap`, which tries wrap generations;
//! - counters (`lamport`, `epoch`) are adopted only from ops whose signature verified.

use std::collections::{BTreeMap, BTreeSet};

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305,
};
use rand_core::RngCore;
use sshub_sync_proto::crypto::{
    aead_open, hybrid_wrap, reshare_combine, reshare_verify, verify, ReshareContribution,
};
use sshub_sync_proto::oplog::{canonical::decode_op, ConfirmItem, Op, OpBody, ReshareItem};
use sshub_sync_proto::wire::SignedOpBytes;
use zeroize::Zeroizing;

use crate::chain::{self, Seen};
use crate::ctx::Ctx;
use crate::error::{CoreError, Result};
use crate::identity::{PhoneIdentity, WRAP_GEN_MAX};
use crate::outgoing::ops_out;
use crate::state::{ChainEntry, PhoneState, COURIER_TAIL};
use crate::text::safe_text;
use crate::views::{Check, ContributorView, HolderView, Opened, RefusalView, ReshareView};

pub use sshub_sync_proto::limits::RESHARE_ITEMS_PER_OP;

/// A credential needs this many readable contributions (the sharing threshold of the product).
pub const SHARE_K: usize = 2;

#[cfg(test)]
mod tests;

/// What the phone will write if the person approves: the new shares, already sealed, and the counters to adopt.
#[derive(Debug)]
pub struct PendingReshare {
    epoch: u64,
    items: Vec<ConfirmItem>,
    /// `hybrid_wrap(own current pub, storage key)`; `None` when a storage key of this epoch is already published.
    storage_sealed: Option<Vec<u8>>,
    seen: Seen,
}

#[derive(Debug)]
struct Contributor {
    id: String,
    session: bool,
    verified: bool,
    known: bool,
}

#[derive(Debug)]
struct Plan {
    pending: PendingReshare,
    holders: Vec<String>,
    my_index: u8,
    contributors: Vec<Contributor>,
    skipped: usize,
}

/// A refusal sentence for the screen, with the contributors that were read when that is what the refusal is about.
#[derive(Debug)]
struct Fail {
    error: String,
    contributors: Vec<Contributor>,
}

impl From<String> for Fail {
    fn from(error: String) -> Self {
        Self {
            error,
            contributors: Vec::new(),
        }
    }
}

impl From<&str> for Fail {
    fn from(error: &str) -> Self {
        error.to_string().into()
    }
}

fn u64be(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}

/// Associated data of a contributor's sub-share for this phone.
fn sub_aad(host: &[u8; 16], epoch: u64, author: &str, to: &str) -> Vec<u8> {
    [
        host.as_slice(),
        u64be(epoch).as_slice(),
        author.as_bytes(),
        to.as_bytes(),
    ]
    .concat()
}

/// Associated data of a confirm item (`envelopeAad` of the page).
pub fn envelope_aad(host: &[u8; 16], epoch: u64, index: u8) -> Vec<u8> {
    [host.as_slice(), u64be(epoch).as_slice(), [index].as_slice()].concat()
}

/// `aead_seal` of the proto with the nonce taken from the injected RNG.
fn seal(rng: &mut dyn RngCore, key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut nonce);
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt((&nonce).into(), Payload { msg: plain, aad })
        .expect("XChaCha20-Poly1305 encrypt");
    [nonce.as_slice(), ct.as_slice()].concat()
}

fn is_device_id(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn name_of(aliases: &BTreeMap<String, String>, id: &str) -> String {
    match aliases.get(id) {
        Some(n) if !n.is_empty() => safe_text(n),
        _ => "(no name known)".to_string(),
    }
}

/// `Some(true)` the signature verifies under a key this phone enrolled with, `Some(false)` it does not (or the stored key
/// is damaged), `None` the author is not in the ring (nothing is proven either way).
fn ring_verify(state: &PhoneState, op: &Op, raw: &SignedOpBytes) -> Option<bool> {
    let known = state.ring.get(&op.device_id)?;
    let key = hex::decode(&known.ed25519_pub)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    Some(key.is_some_and(|k| verify(&k, &raw.0, &raw.1)))
}

/// The storage key this phone already published for `epoch`, opened from its own chain, or `None`.
pub fn own_storage_key(
    id: &mut PhoneIdentity,
    chain: &[ChainEntry],
    epoch: u64,
) -> Option<Zeroizing<[u8; 32]>> {
    for e in chain {
        let Ok(op) = decode_op(&e.canonical, e.signature64()) else {
            continue;
        };
        if let OpBody::ShareStorageKey { epoch: ep, sealed } = &op.body {
            if *ep != epoch {
                continue;
            }
            // an older generation's key that cannot be opened: keep looking
            if let Ok(k) = id.unwrap(sealed) {
                if let Ok(k) = <[u8; 32]>::try_from(k.as_slice()) {
                    return Some(Zeroizing::new(k));
                }
            }
        }
    }
    None
}

fn parse_ops(ops: &[SignedOpBytes]) -> std::result::Result<Vec<Op>, String> {
    ops.iter()
        .map(|o| decode_op(&o.0, o.1).map_err(|e| format!("Not a list of reshare ops: {e:#}")))
        .collect()
}

/// `planReshare`: everything the phone decides about a delivery, from the bytes. `ops` and `parsed` are the same list.
fn plan(
    id: &mut PhoneIdentity,
    state: &PhoneState,
    rng: &mut dyn RngCore,
    ops: &[SignedOpBytes],
    parsed: &[Op],
    epoch: u64,
) -> std::result::Result<Plan, Fail> {
    let me = id.device_id().to_string();
    let sigs: Vec<Option<bool>> = parsed
        .iter()
        .zip(ops)
        .map(|(op, raw)| ring_verify(state, op, raw))
        .collect();

    // (index in the list, new_holders, items)
    let mut batches: Vec<(usize, &[String], &[ReshareItem])> = Vec::new();
    let mut sessions: Vec<(usize, &[u8])> = Vec::new();
    for (i, op) in parsed.iter().enumerate() {
        match &op.body {
            OpBody::ShareReshareBatch {
                epoch: e,
                new_holders,
                items,
            } if *e == epoch => batches.push((i, new_holders.as_slice(), items.as_slice())),
            OpBody::ShareReshareSession {
                epoch: e,
                to,
                sealed,
            } if *e == epoch && *to == me => sessions.push((i, sealed.as_slice())),
            _ => {}
        }
    }
    if batches.is_empty() {
        return Err(format!("no reshare batches for epoch {epoch} in these ops").into());
    }
    for i in batches
        .iter()
        .map(|b| b.0)
        .chain(sessions.iter().map(|s| s.0))
    {
        if sigs[i] == Some(false) {
            return Err(format!(
                "the signature of {} does not verify over its reshare op",
                parsed[i].device_id
            )
            .into());
        }
    }
    // A holder id is the 16 lower-case hex characters of a device id; anything else never reaches a screen (PF5).
    if !batches.iter().all(|b| b.1.iter().all(|h| is_device_id(h))) {
        return Err("a holder id in the reshare is not a device id".into());
    }
    let mut holders: Vec<String> = batches[0].1.to_vec();
    holders.sort();
    for b in &batches {
        let mut other = b.1.to_vec();
        other.sort();
        if other != holders {
            return Err("the contributors disagree on the new holder set".into());
        }
    }
    // An index is a u8 and a holder is named once: a repeated id would shift every index after it.
    if holders.len() > 255 || holders.windows(2).any(|w| w[0] == w[1]) {
        return Err(
            "the new holder set is not a list of distinct holders this phone can index".into(),
        );
    }
    let Some(at) = holders.iter().position(|h| *h == me) else {
        return Err("this phone is not a holder at that epoch".into());
    };
    let my_index = (at + 1) as u8;

    // Session keys: `unwrap` tries the wrap generations; a blob sealed to another key or damaged skips that contributor.
    let mut keys: BTreeMap<String, Zeroizing<[u8; 32]>> = BTreeMap::new();
    for (i, sealed) in &sessions {
        if let Ok(k) = id.unwrap(sealed) {
            if let Ok(k) = <[u8; 32]>::try_from(k.as_slice()) {
                keys.insert(parsed[*i].device_id.clone(), Zeroizing::new(k));
            }
        }
    }
    let mut authors: Vec<&str> = Vec::new();
    for b in &batches {
        let a = parsed[b.0].device_id.as_str();
        if !authors.contains(&a) {
            authors.push(a);
        }
    }
    let contributors: Vec<Contributor> = authors
        .iter()
        .map(|a| {
            let first = batches
                .iter()
                .find(|b| parsed[b.0].device_id == *a)
                .expect("an author wrote a batch")
                .0;
            Contributor {
                id: (*a).to_string(),
                session: keys.contains_key(*a),
                verified: sigs[first] == Some(true),
                known: state.ring.contains_key(*a),
            }
        })
        .collect();
    // Contributors this phone has not met prove nothing: at least SHARE_K of them must be devices it enrolled with.
    if contributors.iter().filter(|c| c.verified).count() < SHARE_K {
        return Err(Fail {
            error: format!(
                "fewer than {SHARE_K} contributors have a signature this phone could verify (it only knows the terminals it enrolled with)"
            ),
            contributors,
        });
    }

    let hosts: BTreeSet<[u8; 16]> = batches
        .iter()
        .flat_map(|b| b.2.iter().map(|i| i.host_uid))
        .collect();
    let mut skipped = 0usize;
    let mut planned: Vec<([u8; 16], Zeroizing<Vec<u8>>)> = Vec::new();
    for host in &hosts {
        // old epoch -> contributions of distinct contributors: (from, author, contribution)
        let mut groups: BTreeMap<u64, Vec<(u8, &str, ReshareContribution)>> = BTreeMap::new();
        for (bi, _, items) in &batches {
            let author = parsed[*bi].device_id.as_str();
            let Some(key) = keys.get(author) else {
                continue;
            };
            let Some(item) = items.iter().find(|i| i.host_uid == *host) else {
                continue;
            };
            // Only a contribution of an author this phone verified is combined: the k check counts these very entries.
            if sigs[*bi] != Some(true) {
                continue;
            }
            let Some(pos) = item.sub_shares.iter().position(|(to, _)| *to == me) else {
                continue;
            };
            let Some(commitment) = item.commitments.get(pos) else {
                continue;
            };
            let Ok(plain) = aead_open(
                key,
                &sub_aad(host, epoch, author, &me),
                &item.sub_shares[pos].1,
            ) else {
                continue;
            };
            let plain = Zeroizing::new(plain);
            if plain.len() != 41 {
                continue;
            }
            let old_epoch = u64::from_be_bytes(plain[..8].try_into().expect("8 bytes"));
            let from = plain[8];
            let c = ReshareContribution {
                from,
                sub_shares: vec![(my_index, plain[9..].to_vec())],
                commitments: vec![commitment.clone()],
            };
            if reshare_verify(&c, my_index).is_err() {
                continue; // a lying contributor
            }
            let g = groups.entry(old_epoch).or_default();
            // Counted by the DEVICE that authored the batch: one author cannot pose as two by writing two `from` values.
            if !g.iter().any(|x| x.0 == from || x.1 == author) {
                g.push((from, author, c));
            }
        }
        let Some((_, g)) = groups.iter().rev().find(|(_, g)| g.len() >= SHARE_K) else {
            skipped += 1;
            continue;
        };
        let picked: Vec<ReshareContribution> = g[..SHARE_K].iter().map(|x| x.2.clone()).collect();
        match reshare_combine(my_index, &picked) {
            Ok(share) => planned.push((*host, Zeroizing::new(share.bytes.clone()))),
            Err(_) => skipped += 1,
        }
    }
    if planned.is_empty() {
        return Err(Fail {
            error: "no credential has two readable contributions yet".into(),
            contributors,
        });
    }

    let (storage, storage_sealed) = match own_storage_key(id, &state.chain, epoch) {
        Some(k) => (k, None),
        None => {
            let mut k = Zeroizing::new([0u8; 32]);
            rng.fill_bytes(&mut k[..]);
            let sealed = hybrid_wrap(&id.public(), &k[..]);
            (k, Some(sealed))
        }
    };
    let items: Vec<ConfirmItem> = planned
        .iter()
        .map(|(host, share)| ConfirmItem {
            host_uid: *host,
            sealed_share: seal(
                rng,
                &storage,
                &envelope_aad(host, epoch, my_index),
                &[[my_index].as_slice(), share.as_slice()].concat(),
            ),
        })
        .collect();

    // Counters are adopted only from ops whose signature verified under a key this phone knows.
    let mut seen = Seen::default();
    for (op, s) in parsed.iter().zip(&sigs) {
        if *s == Some(true) {
            seen.lamport = seen.lamport.max(op.lamport);
            seen.epoch = seen.epoch.max(op.epoch);
        }
    }
    Ok(Plan {
        pending: PendingReshare {
            epoch,
            items,
            storage_sealed,
            seen,
        },
        holders,
        my_index,
        contributors,
        skipped,
    })
}

/// One line per contributor of a refused delivery (the page's contributor summary).
fn contributor_line(aliases: &BTreeMap<String, String>, c: &Contributor) -> String {
    format!(
        "{} [{}] ({}{}{})",
        name_of(aliases, &c.id),
        c.id,
        if c.verified {
            "signature verified"
        } else {
            "signature not checked"
        },
        if c.known {
            ""
        } else {
            ", not known to this phone"
        },
        if c.session {
            ""
        } else {
            ", no session key for this phone"
        }
    )
}

fn refuse(reason: impl Into<String>) -> (Opened, Option<PendingReshare>) {
    (
        Opened::Refused {
            view: RefusalView {
                kind: crate::views::RefusalKind::Other,
                compared: Vec::new(),
                reason: reason.into(),
                checks: Vec::new(),
            },
        },
        None,
    )
}

/// A kind 5 list that carries `ShareReshareBatch` ops: the summary the person approves, or a refusal. Remote ops of any
/// shape end on a refusal (`reshareLoad`).
pub fn open_reshare(
    ops: Vec<SignedOpBytes>,
    ctx: &mut Ctx,
    token: u64,
) -> (Opened, Option<PendingReshare>) {
    let parsed = match parse_ops(&ops) {
        Ok(p) => p,
        Err(e) => return refuse(e),
    };
    let epoch = parsed
        .iter()
        .filter_map(|o| match &o.body {
            OpBody::ShareReshareBatch { epoch, .. } => Some(*epoch),
            _ => None,
        })
        .max();
    let Some(epoch) = epoch else {
        return refuse("These ops carry no reshare batch.");
    };
    let p = match plan(
        &mut *ctx.id,
        &*ctx.state,
        &mut *ctx.rng,
        &ops,
        &parsed,
        epoch,
    ) {
        Ok(p) => p,
        Err(f) => {
            let aliases = ctx.aliases();
            let checks = f
                .contributors
                .iter()
                .map(|c| Check {
                    text: contributor_line(&aliases, c),
                    warn: !c.verified,
                    ok: true,
                })
                .collect();
            return (
                Opened::Refused {
                    view: RefusalView {
                        kind: crate::views::RefusalKind::Other,
                        compared: Vec::new(),
                        reason: f.error,
                        checks,
                    },
                },
                None,
            );
        }
    };
    let aliases = ctx.aliases();
    let me = ctx.id.device_id().to_string();
    let view = ReshareView {
        token,
        epoch: p.pending.epoch,
        holders: p.holders.len() as u32,
        my_index: u32::from(p.my_index),
        credentials_ready: p.pending.items.len() as u32,
        waiting: p.skipped as u32,
        contributors: p
            .contributors
            .iter()
            .map(|c| ContributorView {
                device_id: c.id.clone(),
                name: name_of(&aliases, &c.id),
                verified: c.verified,
                known: c.known,
                has_session: c.session,
            })
            .collect(),
        holders_after: p
            .holders
            .iter()
            .map(|h| HolderView {
                device_id: h.clone(),
                name: if *h == me {
                    "this phone".to_string()
                } else {
                    name_of(&aliases, h)
                },
                is_this_phone: *h == me,
            })
            .collect(),
        storage_key_new: p.pending.storage_sealed.is_some(),
    };
    (Opened::Reshare { view }, Some(p.pending))
}

/// Appends the ops of a plan to the phone chain (`ShareStorageKey` when new, then confirm batches of at most
/// `RESHARE_ITEMS_PER_OP` items) and returns the list a terminal must take. A delivery carries at most `COURIER_TAIL`
/// ops: a bigger plan is refused before anything is signed.
pub fn publish_reshare(p: PendingReshare, ctx: &mut Ctx) -> Result<crate::views::Outgoing> {
    let count =
        p.items.chunks(RESHARE_ITEMS_PER_OP).count() + usize::from(p.storage_sealed.is_some());
    if count > COURIER_TAIL {
        return Err(CoreError::refused(format!(
            "{} credentials need {count} ops; a delivery carries {COURIER_TAIL}. Ask the terminal to send them in parts.",
            p.items.len()
        )));
    }
    let seen = Some(p.seen);
    if let Some(sealed) = p.storage_sealed {
        chain::share_storage_key(ctx.state, ctx.id, p.epoch, sealed, ctx.now, seen)?;
    }
    for chunk in p.items.chunks(RESHARE_ITEMS_PER_OP) {
        chain::confirm_batch(ctx.state, ctx.id, p.epoch, chunk.to_vec(), ctx.now, seen)?;
    }
    ops_out(
        &chain::courier_ops(ctx.state, count),
        "Deliver these ops to a terminal. The ring marks a credential adopted only after a real unlock with the new sharing.",
    )
}

/// Announces the next wrap key generation and switches this phone to it. Never rotates below a generation the ring
/// already uses: the adopted chain and any unwrap that needed a higher one say which (PF2).
pub fn rotate_wrap_keys(ctx: &mut Ctx) -> Result<crate::views::Outgoing> {
    let base = ctx.state.wrap_epoch.max(ctx.id.seen_wrap_epoch());
    let next_epoch = base + 1;
    if next_epoch > WRAP_GEN_MAX {
        return Err(CoreError::refused(format!(
            "this phone cannot open wrap key generations above {WRAP_GEN_MAX}"
        )));
    }
    ctx.state.wrap_epoch = base;
    let next = ctx.id.at_generation(next_epoch)?;
    chain::wrap_key_rotate(ctx.state, ctx.id, &next, ctx.now, None)?;
    *ctx.id = next;
    ops_out(
        &chain::courier_ops(ctx.state, 1),
        "Deliver this to a terminal before the reshare is started: contributors wrap to the new keys. Older generations stay readable on this phone (they derive from the same 24 words).",
    )
}
