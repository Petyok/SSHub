//! Reissuing shares to a new holder set without ever reconstructing a secret (C14 epoch envelopes).
//!
//! Contributor side, [`reissue_plan`]: for every credential it holds a share of, the contributor
//! re-splits its own share under a fresh polynomial for the new holder indices and seals each
//! sub-share under a per-(contributor, recipient) session key that it delivers wrapped to the
//! recipient. Receiver side, [`reissue_receive`]: each new holder sums the sub-shares of two
//! contributors with Lagrange coefficients (`reshare_combine`), keeps its new share sealed under
//! its epoch storage key and publishes that envelope so every peer replicates it.
//!
//! Sub-share plaintext is `old_epoch u64 BE || from index || 32 bytes`; its AEAD context is
//! `host_uid || epoch u64 BE || contributor id || recipient id`. The paper recipient cannot take
//! part (it is offline): it keeps only the sharing of the epoch it was written for.

use anyhow::{ensure, Context, Result};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use super::crypto::{
    aead_open, aead_seal, hybrid_unwrap, hybrid_wrap, reshare_combine, reshare_contribute,
    DeviceIdentity, DevicePublic, ReshareContribution, Share,
};
use super::limits::RESHARE_ITEMS_PER_OP;
use super::oplog::{ConfirmItem, DeviceId, DeviceRole, DeviceStatus, HostUid, OpBody, ReshareItem};
use super::quorum::{
    envelope_aad, holder_index, meta_get, meta_set, ops_of, own_shares, paper_pub, SHARE_K,
};
use super::share_release::share_to_bytes;
use crate::store::{uid_hex, LauncherStore};

/// Ops a device has to append for one step of a reissue.
#[derive(Debug, Default)]
pub struct ReissuePlan {
    pub epoch: u64,
    pub bodies: Vec<OpBody>,
}

/// The epoch a new reissue starts at: one above every epoch seen in the log.
pub fn next_epoch(store: &LauncherStore) -> Result<u64> {
    Ok(ops_of(store, "ShareReshareBatch", None)?
        .iter()
        .filter_map(|o| match &o.body {
            OpBody::ShareReshareBatch { epoch, .. } => Some(*epoch),
            _ => None,
        })
        .max()
        .map_or(1, |e| e + 1))
}

fn sub_aad(host: &HostUid, epoch: u64, from: &DeviceId, to: &DeviceId) -> Vec<u8> {
    [
        host.as_slice(),
        &epoch.to_be_bytes(),
        from.as_bytes(),
        to.as_bytes(),
    ]
    .concat()
}

fn quorum_credentials(store: &LauncherStore) -> Result<Vec<HostUid>> {
    store.with_conn(|c| {
        let mut st = c.prepare("SELECT host_uid FROM sync_cred_state ORDER BY host_uid")?;
        let rows = st
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|u| HostUid::try_from(u).ok())
            .collect())
    })
}

fn holder_public(store: &LauncherStore, me: &DevicePublic, id: &DeviceId) -> Result<DevicePublic> {
    if *id == me.device_id() {
        return Ok(me.clone());
    }
    let row = store
        .sync_devices()?
        .into_iter()
        .find(|d| d.id == *id)
        .with_context(|| format!("{id} is not a device of this ring"))?;
    ensure!(row.role == DeviceRole::Holder, "{id} is not a holder");
    ensure!(
        !matches!(
            row.status,
            DeviceStatus::Revoked | DeviceStatus::Compromised
        ),
        "{id} is revoked or compromised"
    );
    Ok(row.public)
}

/// Contributor side of reissuing to `new_holders` at `epoch` (use [`next_epoch`] to start one, or
/// the epoch of the batch you were asked to join). Never reconstructs a secret.
pub fn reissue_plan(
    store: &LauncherStore,
    id: &DeviceIdentity,
    new_holders: &[DeviceId],
    epoch: u64,
) -> Result<ReissuePlan> {
    let me = id.public().device_id();
    let mut holders = new_holders.to_vec();
    // The paper is a permanent holder: it gets a share index too, its sub-shares sealed to its
    // permanent recipient key instead of a session key (it is offline and adopts nothing).
    let paper = paper_pub(store)?;
    if let Some(p) = &paper {
        holders.push(p.device_id());
    }
    holders.sort();
    holders.dedup();
    ensure!(
        holders.len() >= SHARE_K as usize,
        "a sharing needs at least two holders"
    );
    ensure!(holders.len() <= 255, "too many holders");
    let paper_id = paper.as_ref().map(|p| p.device_id());
    let publics = holders
        .iter()
        .map(|h| match &paper {
            Some(p) if Some(h) == paper_id.as_ref() => Ok(p.clone()),
            _ => holder_public(store, &id.public(), h),
        })
        .collect::<Result<Vec<_>>>()?;
    let indices: Vec<u8> = (1..=holders.len() as u8).collect();

    let mut plan = ReissuePlan {
        epoch,
        bodies: Vec::new(),
    };
    let mut sessions: Vec<Option<Zeroizing<[u8; 32]>>> = Vec::new();
    for (h, public) in holders.iter().zip(&publics) {
        if Some(h) == paper_id.as_ref() {
            sessions.push(None);
            continue;
        }
        let mut key = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *key);
        plan.bodies.push(OpBody::ShareReshareSession {
            epoch,
            to: h.clone(),
            sealed: hybrid_wrap(public, &*key),
        });
        sessions.push(Some(key));
    }

    let mut items = Vec::new();
    for host in quorum_credentials(store)? {
        let Ok(mut mine) = own_shares(store, id, &host) else {
            continue;
        };
        let (old_epoch, share) = mine.remove(0);
        let c = reshare_contribute(&share, &indices, SHARE_K);
        let mut sub_shares = Vec::new();
        for (j, h) in holders.iter().enumerate() {
            let (to_index, sub) = &c.sub_shares[j];
            ensure!(*to_index == indices[j], "contribution order");
            let plain = Zeroizing::new(
                [
                    old_epoch.to_be_bytes().as_slice(),
                    &[c.from],
                    sub.as_slice(),
                ]
                .concat(),
            );
            let sealed = match &sessions[j] {
                Some(key) => aead_seal(key, &sub_aad(&host, epoch, &me, h), &plain),
                None => hybrid_wrap(
                    &publics[j],
                    &[host.as_slice(), &epoch.to_be_bytes(), plain.as_slice()].concat(),
                ),
            };
            sub_shares.push((h.clone(), sealed));
        }
        items.push(ReshareItem {
            host_uid: host,
            sub_shares,
            commitments: c.commitments,
        });
    }
    for chunk in items.chunks(RESHARE_ITEMS_PER_OP) {
        plan.bodies.push(OpBody::ShareReshareBatch {
            epoch,
            new_holders: holders.clone(),
            items: chunk.to_vec(),
        });
    }
    Ok(plan)
}

fn storage_key_for(
    store: &LauncherStore,
    id: &DeviceIdentity,
    epoch: u64,
    plan: &mut ReissuePlan,
) -> Result<Zeroizing<[u8; 32]>> {
    let me = id.public().device_id();
    for op in ops_of(store, "ShareStorageKey", Some(&me))? {
        if let OpBody::ShareStorageKey { epoch: e, sealed } = &op.body {
            if *e == epoch {
                let k = Zeroizing::new(hybrid_unwrap(id, sealed)?);
                return Ok(Zeroizing::new(
                    k.as_slice().try_into().context("storage key length")?,
                ));
            }
        }
    }
    let mut key = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *key);
    plan.bodies.push(OpBody::ShareStorageKey {
        epoch,
        sealed: hybrid_wrap(&id.public(), &*key),
    });
    Ok(key)
}

/// Receiver side: turn the sub-shares addressed to this device at `epoch` into its new shares, held
/// sealed under its epoch storage key. Credentials that lack two contributors are skipped (retry
/// after the next sync). Never reconstructs a secret.
pub fn reissue_receive(
    store: &LauncherStore,
    id: &DeviceIdentity,
    epoch: u64,
) -> Result<ReissuePlan> {
    let me = id.public().device_id();
    let batches: Vec<_> = ops_of(store, "ShareReshareBatch", None)?
        .into_iter()
        .filter_map(|op| match op.body {
            OpBody::ShareReshareBatch {
                epoch: e,
                new_holders,
                items,
            } if e == epoch => Some((op.device_id, new_holders, items)),
            _ => None,
        })
        .collect();
    ensure!(!batches.is_empty(), "no reshare batches for epoch {epoch}");
    let mut holders = batches[0].1.clone();
    holders.sort();
    ensure!(
        batches.iter().all(|b| {
            let mut h = b.1.clone();
            h.sort();
            h == holders
        }),
        "contributors disagree on the new holder set"
    );
    let my_index =
        holder_index(&holders, &me).context("this device is not a holder at that epoch")?;

    let mut sessions: Vec<(DeviceId, Zeroizing<[u8; 32]>)> = Vec::new();
    for op in ops_of(store, "ShareReshareSession", None)? {
        if let OpBody::ShareReshareSession {
            epoch: e,
            to,
            sealed,
        } = &op.body
        {
            if *e == epoch && *to == me {
                if let Ok(k) = hybrid_unwrap(id, sealed) {
                    if let Ok(k) = <[u8; 32]>::try_from(k.as_slice()) {
                        sessions.push((op.device_id.clone(), Zeroizing::new(k)));
                    }
                }
            }
        }
    }

    let mut plan = ReissuePlan {
        epoch,
        bodies: Vec::new(),
    };
    let mut confirms = Vec::new();
    let mut storage: Option<Zeroizing<[u8; 32]>> = None;
    let mut hosts: Vec<HostUid> = batches
        .iter()
        .flat_map(|b| b.2.iter().map(|i| i.host_uid))
        .collect();
    hosts.sort();
    hosts.dedup();
    for host in hosts {
        // old_epoch -> contributions of distinct contributors
        let mut groups: Vec<(u64, Vec<ReshareContribution>)> = Vec::new();
        for (author, _, items) in &batches {
            let Some(item) = items.iter().find(|i| i.host_uid == host) else {
                continue;
            };
            let Some((_, key)) = sessions.iter().find(|(a, _)| a == author) else {
                continue;
            };
            let Some(pos) = item.sub_shares.iter().position(|(to, _)| *to == me) else {
                continue;
            };
            let Some(commit) = item.commitments.get(pos) else {
                continue;
            };
            let Ok(plain) = aead_open(
                key,
                &sub_aad(&host, epoch, author, &me),
                &item.sub_shares[pos].1,
            ) else {
                continue;
            };
            let plain = Zeroizing::new(plain);
            if plain.len() != 8 + 1 + 32 {
                continue;
            }
            let old_epoch = u64::from_be_bytes(plain[..8].try_into().unwrap());
            let c = ReshareContribution {
                from: plain[8],
                sub_shares: vec![(my_index, plain[9..].to_vec())],
                commitments: vec![commit.clone()],
            };
            match groups.iter_mut().find(|(e, _)| *e == old_epoch) {
                Some((_, v)) if !v.iter().any(|x| x.from == c.from) => v.push(c),
                Some(_) => {}
                None => groups.push((old_epoch, vec![c])),
            }
        }
        let Some((_, group)) = groups
            .into_iter()
            .filter(|(_, g)| g.len() >= SHARE_K as usize)
            .max_by_key(|(e, _)| *e)
        else {
            continue;
        };
        let Ok(share) = reshare_combine(my_index, &group[..SHARE_K as usize]) else {
            continue;
        };
        let key = match &storage {
            Some(k) => k.clone(),
            None => {
                let k = storage_key_for(store, id, epoch, &mut plan)?;
                storage = Some(k.clone());
                k
            }
        };
        confirms.push(ConfirmItem {
            host_uid: host,
            sealed_share: aead_seal(
                &key,
                &envelope_aad(&host, epoch, my_index),
                &share_to_bytes(&share),
            ),
        });
    }
    for chunk in confirms.chunks(RESHARE_ITEMS_PER_OP) {
        plan.bodies.push(OpBody::ShareReshareConfirmBatch {
            epoch,
            items: chunk.to_vec(),
        });
    }
    Ok(plan)
}

/// The paper's share of `host` at a reshare `epoch`, rebuilt in memory by the device that holds the
/// typed phrase: the contributors sealed sub-shares to the paper's permanent key, nobody adopted on
/// its behalf. Lagrange-combines the sub-shares of two contributors (`reshare_combine`); the DEK is
/// never reconstructed.
pub fn paper_share_at(
    store: &LauncherStore,
    paper: &DeviceIdentity,
    host: &HostUid,
    epoch: u64,
) -> Result<Share> {
    let me = paper.public().device_id();
    let mut holders: Option<Vec<DeviceId>> = None;
    let mut groups: Vec<(u64, Vec<ReshareContribution>)> = Vec::new();
    for op in ops_of(store, "ShareReshareBatch", None)? {
        let OpBody::ShareReshareBatch {
            epoch: e,
            new_holders,
            items,
        } = &op.body
        else {
            continue;
        };
        if *e != epoch {
            continue;
        }
        let Some(item) = items.iter().find(|i| i.host_uid == *host) else {
            continue;
        };
        let mut sorted = new_holders.clone();
        sorted.sort();
        let Some(my_index) = holder_index(&sorted, &me) else {
            continue;
        };
        holders = Some(sorted);
        let Some(pos) = item.sub_shares.iter().position(|(to, _)| *to == me) else {
            continue;
        };
        let Some(commit) = item.commitments.get(pos) else {
            continue;
        };
        let Ok(plain) = hybrid_unwrap(paper, &item.sub_shares[pos].1) else {
            continue;
        };
        let plain = Zeroizing::new(plain);
        if plain.len() != 16 + 8 + 8 + 1 + 32
            || plain[..16] != host[..]
            || plain[16..24] != epoch.to_be_bytes()
        {
            continue;
        }
        let old_epoch = u64::from_be_bytes(plain[24..32].try_into().unwrap());
        let c = ReshareContribution {
            from: plain[32],
            sub_shares: vec![(my_index, plain[33..].to_vec())],
            commitments: vec![commit.clone()],
        };
        match groups.iter_mut().find(|(e, _)| *e == old_epoch) {
            Some((_, v)) if !v.iter().any(|x| x.from == c.from) => v.push(c),
            Some(_) => {}
            None => groups.push((old_epoch, vec![c])),
        }
    }
    let holders = holders.context("the paper is not a holder at that epoch")?;
    let my_index =
        holder_index(&holders, &me).context("the paper is not a holder at that epoch")?;
    let (_, group) = groups
        .into_iter()
        .filter(|(_, g)| g.len() >= SHARE_K as usize)
        .max_by_key(|(e, _)| *e)
        .context(
            "fewer than two contributors sealed a sub-share to the paper for this credential",
        )?;
    reshare_combine(my_index, &group[..SHARE_K as usize])
}

/// Epochs at which any reshare batch exists, newest first.
pub fn reshare_epochs(store: &LauncherStore) -> Result<Vec<u64>> {
    let mut e: Vec<u64> = ops_of(store, "ShareReshareBatch", None)?
        .into_iter()
        .filter_map(|o| match o.body {
            OpBody::ShareReshareBatch { epoch, .. } => Some(epoch),
            _ => None,
        })
        .collect();
    e.sort_by_key(|x| std::cmp::Reverse(*x));
    e.dedup();
    Ok(e)
}

fn adopted_key(host: &HostUid) -> String {
    format!("adopted:{}", uid_hex(host))
}

/// A real unlock succeeded under the sharing of `epoch`: from now on the old sharing may go.
pub fn mark_adopted(store: &LauncherStore, host: &HostUid, epoch: u64) -> Result<()> {
    if adopted_epoch(store, host)? < epoch {
        meta_set(store, &adopted_key(host), &epoch.to_be_bytes())?;
    }
    Ok(())
}

pub fn adopted_epoch(store: &LauncherStore, host: &HostUid) -> Result<u64> {
    Ok(meta_get(store, &adopted_key(host))?
        .and_then(|b| b.try_into().ok())
        .map_or(0, u64::from_be_bytes))
}

/// Credentials reshared at `epoch` that have not been confirmed by an unlock yet. Empty means the
/// previous wrap keys may be retired (`retire_previous_wrap_keys`).
pub fn unadopted(store: &LauncherStore, epoch: u64) -> Result<Vec<HostUid>> {
    let mut hosts: Vec<HostUid> = ops_of(store, "ShareReshareBatch", None)?
        .into_iter()
        .filter_map(|o| match o.body {
            OpBody::ShareReshareBatch {
                epoch: e, items, ..
            } if e == epoch => Some(items.into_iter().map(|i| i.host_uid).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect();
    hosts.sort();
    hosts.dedup();
    let mut out = Vec::new();
    for h in hosts {
        if adopted_epoch(store, &h)? < epoch {
            out.push(h);
        }
    }
    Ok(out)
}
