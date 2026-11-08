//! The phone's own op chain: authoring the ops the phone signs (header, signature, chaining) and head adoption after a
//! restore. Ports `state.mjs` (`nextHeader`, `appendCanonical`, `adoptOwnChain`, `acceptHead`, `confirmNoChain`).
//!
//! Received ops are parsed by `Op::from_canonical` (strict: the bytes must be exactly what the encoder would write, so
//! the hash and the signature always cover the bytes that were received). Authored ops are built from the typed body,
//! encoded by the same canonical encoder the terminal uses, and signed over those bytes.

use sshub_sync_proto::crypto::{chain_hash, verify, ALG_CURRENT};
use sshub_sync_proto::oplog::{canonical::decode_op, CertScope, ConfirmItem, DeviceId, Op, OpBody};
use sshub_sync_proto::wire::{SignedOpBytes, TerminalsBlock};

use crate::error::{CoreError, Result};
use crate::identity::{PhoneIdentity, WRAP_GEN_MAX};
use crate::state::{
    ChainEntry, HeadCandidate, HeadProposal, PhoneState, COURIER_TAIL, HEAD_PENDING_MESSAGE,
};

/// Largest canonical op the phone writes or accepts (`wire.rs MAX_COURIER_OP_BYTES`).
pub const MAX_OP_BYTES: usize = sshub_sync_proto::wire::MAX_COURIER_OP_BYTES;

/// `lamport` and `epoch` of an op whose signature the caller VERIFIED (never of an unverified one: a forged huge
/// lamport would push the chain out of range).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub lamport: u64,
    pub epoch: u64,
}

/// A freshly authored op, signed and appended to the chain.
#[derive(Clone, Debug)]
pub struct Authored {
    pub op: Op,
    pub canonical: Vec<u8>,
    pub signature: [u8; 64],
    pub hash: [u8; 32],
}

impl Authored {
    pub fn signed(&self) -> SignedOpBytes {
        (self.canonical.clone(), self.signature)
    }
}

/// Appends one op to the chain: the header continues the chain (seq + 1, lamport above everything seen, the highest
/// epoch seen), the body is whatever the caller built. Refuses while the head is pending, and past u64.
pub fn append(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    now: i64,
    seen: Option<Seen>,
    body: OpBody,
) -> Result<Authored> {
    if state.head_pending {
        return Err(CoreError::refused(HEAD_PENDING_MESSAGE));
    }
    let last = state.chain.last();
    let seen = seen.unwrap_or_default();
    let lamport = [last.map_or(0, |e| e.lamport), state.lamport, seen.lamport]
        .into_iter()
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let seq = last.map_or(0, |e| e.seq).checked_add(1);
    let (Some(lamport), Some(seq)) = (lamport, seq) else {
        return Err(CoreError::refused(
            "a counter of this phone chain would leave the u64 range: refusing to sign",
        ));
    };
    let epoch = state.epoch.max(seen.epoch);
    let mut op = Op {
        device_id: id.device_id().to_string(),
        seq,
        lamport,
        prev_hash: last.map_or([0; 32], |e| e.hash32()),
        timestamp: now,
        epoch,
        alg: ALG_CURRENT,
        body,
        signature: [0; 64],
    };
    let canonical = op.canonical_bytes();
    if canonical.len() > MAX_OP_BYTES {
        return Err(CoreError::refused(format!(
            "op is larger than {MAX_OP_BYTES} bytes"
        )));
    }
    op.signature = id.sign(&canonical);
    let hash = chain_hash(&canonical);
    state.chain.push(ChainEntry {
        seq,
        lamport,
        timestamp: now,
        hash: hash.to_vec(),
        canonical: canonical.clone(),
        signature: op.signature.to_vec(),
    });
    state.lamport = lamport;
    state.epoch = epoch;
    Ok(Authored {
        signature: op.signature,
        op,
        canonical,
        hash,
    })
}

/// Self head attestation: "my head before this op was (seq, hash)" with a fresh timestamp. The phone's proof of recent
/// contact (`PHONE_FRESH_SECS`) that rides every response.
pub fn head_attest(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    let (seq, chain_hash) = state
        .chain
        .last()
        .map_or((0, [0; 32]), |e| (e.seq, e.hash32()));
    append(
        state,
        id,
        now,
        seen,
        OpBody::HeadAttest {
            device_id: id.device_id().to_string(),
            seq,
            chain_hash,
        },
    )
}

/// `PhoneReenrollCancel { reenroll_hash, by_phone: true, third_factor: null }`.
pub fn reenroll_cancel(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    reenroll_hash: [u8; 32],
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(
        state,
        id,
        now,
        seen,
        OpBody::PhoneReenrollCancel {
            reenroll_hash,
            by_phone: true,
            third_factor: None,
        },
    )
}

/// `RedPhoneSign { op_hash }`: the phone's proof that it saw the Red op with this hash.
pub fn red_phone_sign(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    op_hash: [u8; 32],
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(state, id, now, seen, OpBody::RedPhoneSign { op_hash })
}

/// Announces the next wrap key generation (C14 step 1). `new_gen` is the identity at the new generation; the caller
/// then keeps using it. Blobs wrapped to older generations stay readable.
pub fn wrap_key_rotate(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    new_gen: &PhoneIdentity,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    let wrap_epoch = state.wrap_epoch + 1;
    if wrap_epoch > WRAP_GEN_MAX {
        return Err(CoreError::refused(format!(
            "this phone cannot open wrap key generations above {WRAP_GEN_MAX}"
        )));
    }
    let public = new_gen.public();
    let out = append(
        state,
        id,
        now,
        seen,
        OpBody::WrapKeyRotate {
            device_id: id.device_id().to_string(),
            wrap_epoch: u64::from(wrap_epoch),
            x25519_pub: public.x25519_pub,
            mlkem_pub: public.mlkem_pub,
        },
    )?;
    state.wrap_epoch = wrap_epoch;
    Ok(out)
}

pub fn share_storage_key(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    epoch: u64,
    sealed: Vec<u8>,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(
        state,
        id,
        now,
        seen,
        OpBody::ShareStorageKey { epoch, sealed },
    )
}

pub fn confirm_batch(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    epoch: u64,
    items: Vec<ConfirmItem>,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(
        state,
        id,
        now,
        seen,
        OpBody::ShareReshareConfirmBatch { epoch, items },
    )
}

/// The fields of a kind 9 request UNCHANGED (`CertRequestWire::to_body`).
#[allow(clippy::too_many_arguments)]
pub fn cert_issue(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    delegate: DeviceId,
    scope: CertScope,
    expiry: i64,
    max_new_hosts_per_day: u32,
    epoch_lo: u64,
    epoch_hi: u64,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(
        state,
        id,
        now,
        seen,
        OpBody::CertIssue {
            device_id: delegate,
            scope,
            expiry,
            max_new_hosts_per_day,
            epoch_lo,
            epoch_hi,
        },
    )
}

pub fn cert_revoke(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    cert_id: [u8; 32],
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(state, id, now, seen, OpBody::CertRevoke { cert_id })
}

pub fn cert_extend(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    cert_id: [u8; 32],
    extra_hosts: u32,
    now: i64,
    seen: Option<Seen>,
) -> Result<Authored> {
    append(
        state,
        id,
        now,
        seen,
        OpBody::CertExtend {
            cert_id,
            extra_hosts,
        },
    )
}

/// Newest ops of the chain as signed bytes for the courier lists.
pub fn courier_ops(state: &PhoneState, count: usize) -> Vec<SignedOpBytes> {
    let n = count.min(COURIER_TAIL);
    let from = state.chain.len().saturating_sub(n);
    state.chain[from..]
        .iter()
        .map(|e| (e.canonical.clone(), e.signature64()))
        .collect()
}

// ------------------------------------------------------------ head adoption

/// What a list of the phone's own ops says.
struct OwnChain {
    chain: Vec<ChainEntry>,
    epoch: u64,
    wrap_epoch: u32,
}

/// Reads a list of the phone's OWN ops as a terminal holds them. Every op must be authored by this key, verify under it
/// and continue the previous one; the newest is the head. Errors are sentences for the screen.
fn read_own_chain(id: &PhoneIdentity, ops: &[SignedOpBytes]) -> Result<OwnChain> {
    if ops.is_empty() {
        return Err(CoreError::bad(
            "The list holds no op of this phone. If this key never signed anything, say so with the other button.",
        ));
    }
    let mut parsed: Vec<(Op, &SignedOpBytes)> = Vec::with_capacity(ops.len());
    for o in ops {
        let op = decode_op(&o.0, o.1)
            .map_err(|e| CoreError::bad(format!("op is not canonical: {e:#}")))?;
        parsed.push((op, o));
    }
    parsed.sort_by_key(|(op, _)| op.seq);
    let own_pub = id.public().ed25519_pub;
    let mut chain: Vec<ChainEntry> = Vec::with_capacity(parsed.len());
    let mut wrap_epoch = 0u32;
    let mut epoch = 0u64;
    for (op, raw) in &parsed {
        if op.device_id != id.device_id() {
            return Err(CoreError::bad(format!(
                "An op in the list is not by this phone ({}).",
                op.device_id
            )));
        }
        if !verify(&own_pub, &raw.0, &raw.1) {
            return Err(CoreError::bad(format!(
                "The signature of seq {} does not verify under this phone's key.",
                op.seq
            )));
        }
        let hash = chain_hash(&raw.0);
        match chain.last() {
            Some(prev) => {
                if prev.seq.checked_add(1) != Some(op.seq)
                    || op.prev_hash.as_slice() != prev.hash.as_slice()
                {
                    return Err(CoreError::bad("The ops in the list do not form one chain."));
                }
            }
            None if op.seq == 1 && op.prev_hash != [0; 32] => {
                return Err(CoreError::bad(
                    "The first op of a chain starts from nothing.",
                ));
            }
            None => {}
        }
        // The wrap generation is read from the signed rotations (the phone cannot keep it any other way after a restore).
        if let OpBody::WrapKeyRotate { wrap_epoch: w, .. } = &op.body {
            wrap_epoch = wrap_epoch.max((*w).min(u64::from(WRAP_GEN_MAX)) as u32);
        }
        epoch = epoch.max(op.epoch);
        chain.push(ChainEntry {
            seq: op.seq,
            lamport: op.lamport,
            timestamp: op.timestamp,
            hash: hash.to_vec(),
            canonical: raw.0.clone(),
            signature: raw.1.to_vec(),
        });
    }
    Ok(OwnChain {
        chain,
        epoch,
        wrap_epoch,
    })
}

/// What `take_chain` tells the screen about the list it just read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptInfo {
    pub head_seq: u64,
    /// Hash of the newest op of the list (what `accept_head` is bound to).
    pub head_hash: Vec<u8>,
    pub newest_at: i64,
    pub ops: u32,
    /// The terminal id this list named (hex), if it named one.
    pub sender: Option<String>,
    /// Two lists from two different terminals named the same newest op: the head waits for the user's acceptance
    /// (`accept_head`).
    pub agreed: bool,
    /// The seq the earlier, disagreeing list ended at.
    pub previous_seq: Option<u64>,
    /// `agreed` came from ONE list whose sender reported its ring's terminal count: `accept_head` needs the tick.
    pub single_source: bool,
    /// What the sender said about its ring (unsigned, informational): shown on the card, never trusted.
    pub terminals: Option<TerminalsBlock>,
}

/// A single list cannot show that it is the newest (a courier or a terminal behind on sync hands over an old tail, and
/// signing on top of it forks the chain). So a head is adopted in two steps: two lists must name the same newest op
/// (`agreed`), the lists must come from two DIFFERENT terminals (the kind 5 sender id, unsigned: it tells two lists
/// apart, it does not prove who made them), and the user then confirms what they see (`accept_head`). A list without a
/// sender id can be the first candidate but never agrees. A terminal-signed head would be stronger.
pub const SAME_TERMINAL_MESSAGE: &str =
    "This list comes from the same terminal as the first one. Scan the list from another terminal.";

pub const SINGLE_SOURCE_TICK_MESSAGE: &str = "This head comes from one list. Tick that you know the risk (the terminal may be behind, and the next approval would then fork your chain), or take a second list from another terminal.";

/// The card text for a head from one list: the count the terminal reported, the risk, never "trusted".
pub fn single_source_note(head_seq: u64, terminals: Option<TerminalsBlock>) -> String {
    let ring = match terminals.map(|t| t.count) {
        Some(1) => "Your ring has one terminal".to_string(),
        Some(n) => format!("Your ring has {n} terminals and I can reach only this one"),
        None => "One list".to_string(),
    };
    format!("{ring}. This phone will continue after seq {head_seq}. Risk: if that terminal is behind, or this phone signed something it never received, the next approval forks your chain and every device raises an alarm; you would then re-enroll this phone. The count is that terminal's own statement.")
}

pub fn adopt_own_chain(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    ops: &[SignedOpBytes],
    sender: Option<[u8; 8]>,
    terminals: Option<TerminalsBlock>,
) -> Result<AdoptInfo> {
    let sender = sender.map(hex::encode);
    let r = read_own_chain(id, ops)?;
    let head = r
        .chain
        .last()
        .expect("a chain read has at least one op")
        .clone();
    let mut info = AdoptInfo {
        head_seq: head.seq,
        head_hash: head.hash.clone(),
        newest_at: head.timestamp,
        ops: r.chain.len() as u32,
        agreed: false,
        previous_seq: None,
        sender: sender.clone(),
        single_source: false,
        terminals,
    };
    let mut keep_sender = sender.clone();
    if let Some(c) = &state.head_candidate {
        if c.seq == head.seq && c.hash == head.hash {
            match (&c.sender, &sender) {
                (Some(a), Some(b)) if a == b => {
                    return Err(CoreError::refused(SAME_TERMINAL_MESSAGE));
                }
                (Some(_), Some(_)) => {
                    state.head_proposal = Some(HeadProposal {
                        chain: r.chain,
                        epoch: r.epoch,
                        wrap_epoch: r.wrap_epoch,
                        single_source: false,
                    });
                    state.head_candidate = None;
                    info.agreed = true;
                    return Ok(info);
                }
                // a list that names no terminal never agrees, and never costs the first one its id
                _ => keep_sender = sender.or_else(|| c.sender.clone()),
            }
        } else {
            info.previous_seq = Some(c.seq);
        }
    }
    state.head_proposal = None;
    // One list is enough when its sender reports a ring (count >= 1: itself): the user ticks the risk (`accept_head`).
    // The first list stays a candidate, so a second agreeing list from another terminal upgrades it to the two-list
    // default that needs no tick. A dead or away terminal never blocks the restore.
    if terminals.is_some_and(|t| t.count >= 1) {
        state.head_proposal = Some(HeadProposal {
            chain: r.chain,
            epoch: r.epoch,
            wrap_epoch: r.wrap_epoch,
            single_source: true,
        });
        info.agreed = true;
        info.single_source = true;
    }
    state.head_candidate = Some(HeadCandidate {
        seq: head.seq,
        hash: head.hash,
        sender: keep_sender,
    });
    Ok(info)
}

/// The user looked at the agreed head (seq, newest op time) and accepted it. Returns (head seq, ops read).
/// `shown` is the (seq, hash) of the head the user was looking at: a proposal that has changed since (another list was
/// taken) is refused, so what is accepted is what was approved.
/// A proposal from one list needs `acknowledged_single_source` (the user's tick); two agreeing lists need none.
pub fn accept_head(
    state: &mut PhoneState,
    shown: (u64, &[u8]),
    acknowledged_single_source: bool,
) -> Result<(u64, u32)> {
    let Some(p) = state.head_proposal.as_ref() else {
        return Err(CoreError::refused(
            "No chain head was agreed by two lists yet.",
        ));
    };
    if p.chain.last().map(|e| (e.seq, e.hash.as_slice())) != Some(shown) {
        return Err(CoreError::refused(
            "The chain head changed since it was shown: take the chain again and look at it before accepting.",
        ));
    }
    if p.single_source && !acknowledged_single_source {
        return Err(CoreError::bad(SINGLE_SOURCE_TICK_MESSAGE));
    }
    let p = state.head_proposal.take().expect("checked above");
    let head_seq = p.chain.last().map_or(0, |e| e.seq);
    let n = p.chain.len() as u32;
    state.lamport = state
        .lamport
        .max(p.chain.iter().map(|e| e.lamport).max().unwrap_or(0));
    state.epoch = state.epoch.max(p.epoch);
    state.wrap_epoch = state.wrap_epoch.max(p.wrap_epoch);
    state.chain = p.chain;
    state.head_pending = false;
    state.head_candidate = None;
    Ok((head_seq, n))
}

/// "This key never signed anything" is a claim nothing can check: the user types the device id to make it deliberate.
pub fn confirm_no_chain(state: &mut PhoneState, id: &PhoneIdentity, typed: &str) -> Result<()> {
    if typed.trim().to_lowercase() != id.device_id() {
        return Err(CoreError::bad(format!(
            "Type this phone's device id ({}) to confirm that this key never signed anything.",
            id.device_id()
        )));
    }
    state.head_pending = false;
    state.head_candidate = None;
    state.head_proposal = None;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use sshub_sync_proto::oplog::canonical::decode_op;

    const NOW: i64 = 1_900_000_000;

    fn id(fill: u8) -> PhoneIdentity {
        PhoneIdentity::new(&[fill; 32], 0).unwrap()
    }

    fn live(id: &PhoneIdentity, n: usize) -> PhoneState {
        let mut s = PhoneState::default();
        for i in 0..n {
            head_attest(&mut s, id, NOW + i as i64, None).unwrap();
        }
        s
    }

    #[test]
    fn authored_ops_chain_verify_and_parse_strictly() {
        let me = id(11);
        let s = live(&me, 3);
        assert_eq!(s.chain.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(!s.chain[0].canonical.is_empty());
        let mut prev = [0u8; 32];
        for e in &s.chain {
            let op = decode_op(&e.canonical, e.signature64()).unwrap();
            assert_eq!(op.prev_hash, prev);
            assert!(op.verify(&me.public().ed25519_pub));
            prev = e.hash32();
        }
        // the HeadAttest names the previous head (seq 0 and zeros before the first op)
        let op1 = decode_op(&s.chain[0].canonical, s.chain[0].signature64()).unwrap();
        assert!(
            matches!(op1.body, OpBody::HeadAttest { seq: 0, chain_hash, .. } if chain_hash == [0; 32])
        );
        let op3 = decode_op(&s.chain[2].canonical, s.chain[2].signature64()).unwrap();
        assert!(
            matches!(op3.body, OpBody::HeadAttest { seq: 2, chain_hash, .. } if chain_hash == s.chain[1].hash32())
        );
        assert_eq!(courier_ops(&s, 8).len(), 3);
        assert_eq!(courier_ops(&s, 2).len(), 2);
    }

    #[test]
    fn counters_never_leave_u64_and_lamport_adopts_only_what_the_caller_verified() {
        let me = id(12);
        let mut s = PhoneState::default();
        s.lamport = u64::MAX;
        assert!(head_attest(&mut s, &me, NOW, None).is_err());
        let mut s = PhoneState::default();
        let a = head_attest(
            &mut s,
            &me,
            NOW,
            Some(Seen {
                lamport: 500,
                epoch: 7,
            }),
        )
        .unwrap();
        assert_eq!((a.op.lamport, a.op.epoch), (501, 7));
        let b = head_attest(&mut s, &me, NOW + 1, None).unwrap();
        assert_eq!((b.op.lamport, b.op.epoch), (502, 7));
    }

    #[test]
    fn a_restored_key_does_not_sign_until_its_head_is_known() {
        let me = id(13);
        let mut s = PhoneState::restored();
        let e = head_attest(&mut s, &me, NOW, None).unwrap_err();
        assert!(e.reason().contains("must not sign yet"));
        assert!(s.chain.is_empty());
    }

    #[test]
    fn one_list_is_not_enough_two_lists_must_name_the_same_newest_op() {
        let me = id(11);
        let live = live(&me, 3);
        let stale: Vec<_> = courier_ops(&live, 8).into_iter().take(2).collect();
        let mut fresh = PhoneState::restored();
        let first = adopt_own_chain(&mut fresh, &me, &stale, Some([1; 8]), None).unwrap();
        assert!(!first.agreed);
        assert!(
            head_attest(&mut fresh, &me, NOW + 100, None).is_err(),
            "one list adopts nothing"
        );
        let newer =
            adopt_own_chain(&mut fresh, &me, &courier_ops(&live, 8), Some([2; 8]), None).unwrap();
        assert!(!newer.agreed);
        assert_eq!(newer.previous_seq, Some(2));
        assert!(accept_head(&mut fresh, (0, &[]), false).is_err());
        assert!(
            adopt_own_chain(&mut fresh, &me, &courier_ops(&live, 8), Some([1; 8]), None)
                .unwrap()
                .agreed
        );
        assert_eq!(
            accept_head(&mut fresh, (3, &live.chain[2].hash), false).unwrap(),
            (3, 3)
        );
        let next = head_attest(&mut fresh, &me, NOW + 100, None).unwrap();
        assert_eq!(next.op.seq, 4);
        assert_eq!(next.op.prev_hash, live.chain[2].hash32());
    }

    #[test]
    fn head_adoption_refuses_foreign_forged_and_broken_lists() {
        let me = id(11);
        let other = id(12);
        let ops = courier_ops(&live(&me, 2), 8);
        let mut s = PhoneState::restored();
        assert!(adopt_own_chain(&mut s, &me, &[], None, None)
            .unwrap_err()
            .reason()
            .contains("holds no op"));
        assert!(adopt_own_chain(&mut s, &other, &ops, None, None)
            .unwrap_err()
            .reason()
            .contains("not by this phone"));
        let forged: Vec<_> = ops.iter().map(|(c, _)| (c.clone(), [0u8; 64])).collect();
        assert!(adopt_own_chain(&mut s, &me, &forged, None, None)
            .unwrap_err()
            .reason()
            .contains("does not verify"));
        let dup = vec![ops[1].clone(), ops[1].clone()];
        assert!(adopt_own_chain(&mut s, &me, &dup, None, None)
            .unwrap_err()
            .reason()
            .contains("one chain"));
        assert!(
            s.head_candidate.is_none(),
            "a refused list leaves no candidate"
        );
    }

    #[test]
    fn never_signed_needs_the_device_id_typed() {
        let me = id(13);
        let mut s = PhoneState::restored();
        assert!(confirm_no_chain(&mut s, &me, "").is_err());
        assert!(confirm_no_chain(&mut s, &me, "aaaaaaaaaaaaaaaa").is_err());
        assert!(s.head_pending);
        let typed = format!(" {} ", me.device_id().to_uppercase());
        confirm_no_chain(&mut s, &me, &typed).unwrap();
        assert!(!s.head_pending);
        assert_eq!(head_attest(&mut s, &me, NOW, None).unwrap().op.seq, 1);
    }

    #[test]
    fn the_same_terminal_twice_never_agrees_and_neither_does_a_list_without_a_sender() {
        let me = id(14);
        let live = live(&me, 3);
        let ops = courier_ops(&live, 8);
        let a = Some([1; 8]);
        // the same frame twice: refused with the sentence, the first candidate stays
        let mut s = PhoneState::restored();
        assert!(!adopt_own_chain(&mut s, &me, &ops, a, None).unwrap().agreed);
        let e = adopt_own_chain(&mut s, &me, &ops, a, None).unwrap_err();
        assert_eq!(e.reason(), SAME_TERMINAL_MESSAGE);
        assert!(s.head_proposal.is_none() && s.head_candidate.is_some());
        // two terminals naming the same head agree
        assert!(
            adopt_own_chain(&mut s, &me, &ops, Some([2; 8]), None)
                .unwrap()
                .agreed
        );
        // a list without a sender can be the first candidate but never agrees, from either side
        let mut s = PhoneState::restored();
        assert!(
            !adopt_own_chain(&mut s, &me, &ops, None, None)
                .unwrap()
                .agreed
        );
        assert!(
            !adopt_own_chain(&mut s, &me, &ops, None, None)
                .unwrap()
                .agreed
        );
        assert!(
            !adopt_own_chain(&mut s, &me, &ops, a, None).unwrap().agreed,
            "the first one named nobody"
        );
        assert!(s.head_proposal.is_none());
        assert_eq!(
            s.head_candidate.as_ref().unwrap().sender.as_deref(),
            Some("0101010101010101")
        );
        assert!(
            !adopt_own_chain(&mut s, &me, &ops, None, None)
                .unwrap()
                .agreed,
            "an anonymous list never completes a pair"
        );
        assert_eq!(
            s.head_candidate.as_ref().unwrap().sender.as_deref(),
            Some("0101010101010101"),
            "and keeps the id"
        );
        assert!(
            adopt_own_chain(&mut s, &me, &ops, Some([2; 8]), None)
                .unwrap()
                .agreed
        );
        // the same terminal with a newer head is a new candidate, not a refusal
        let mut s = PhoneState::restored();
        let old: Vec<_> = ops.iter().take(2).cloned().collect();
        adopt_own_chain(&mut s, &me, &old, a, None).unwrap();
        let newer = adopt_own_chain(&mut s, &me, &ops, a, None).unwrap();
        assert!(!newer.agreed);
        assert_eq!(newer.previous_seq, Some(2));
    }

    #[test]
    fn accepting_a_head_is_bound_to_the_head_that_was_shown() {
        let me = id(13);
        let live = live(&me, 3);
        let mut fresh = PhoneState::restored();
        let ops = courier_ops(&live, 8);
        adopt_own_chain(&mut fresh, &me, &ops, Some([1; 8]), None).unwrap();
        let agreed = adopt_own_chain(&mut fresh, &me, &ops, Some([2; 8]), None).unwrap();
        assert!(agreed.agreed);
        assert_eq!(agreed.head_hash, live.chain[2].hash);
        // the screen showed another head (an older one): refused, and the agreed proposal stays
        let e = accept_head(&mut fresh, (2, &live.chain[1].hash), false).unwrap_err();
        assert!(
            e.reason().contains("changed since it was shown"),
            "{}",
            e.reason()
        );
        assert!(fresh.head_pending && fresh.head_proposal.is_some());
        accept_head(&mut fresh, (3, &agreed.head_hash), false).unwrap();
        assert!(!fresh.head_pending);
    }

    #[test]
    fn wrap_generation_comes_back_from_the_adopted_rotations() {
        let g0 = id(12);
        let mut live = PhoneState::default();
        let g1 = g0.at_generation(1).unwrap();
        wrap_key_rotate(&mut live, &g0, &g1, NOW, None).unwrap();
        let g2 = g0.at_generation(2).unwrap();
        wrap_key_rotate(&mut live, &g1, &g2, NOW + 1, None).unwrap();
        assert_eq!(live.wrap_epoch, 2);
        let mut fresh = PhoneState::restored();
        let me = id(12);
        adopt_own_chain(&mut fresh, &me, &courier_ops(&live, 8), Some([1; 8]), None).unwrap();
        adopt_own_chain(&mut fresh, &me, &courier_ops(&live, 8), Some([2; 8]), None).unwrap();
        let last = live.chain.last().unwrap();
        accept_head(&mut fresh, (last.seq, &last.hash), false).unwrap();
        assert_eq!(fresh.wrap_epoch, 2);
        // a rotation op announces the keys of that generation
        let op = decode_op(&live.chain[1].canonical, live.chain[1].signature64()).unwrap();
        match op.body {
            OpBody::WrapKeyRotate {
                wrap_epoch,
                x25519_pub,
                mlkem_pub,
                ..
            } => {
                assert_eq!(wrap_epoch, 2);
                assert_eq!(x25519_pub, g2.public().x25519_pub);
                assert_eq!(mlkem_pub, g2.public().mlkem_pub);
            }
            _ => panic!("not a rotation"),
        }
    }

    #[test]
    fn rotation_stops_at_the_bound() {
        let g = id(14);
        let mut s = PhoneState::default();
        s.wrap_epoch = WRAP_GEN_MAX;
        let next = g.at_generation(WRAP_GEN_MAX).unwrap();
        assert!(wrap_key_rotate(&mut s, &g, &next, NOW, None).is_err());
    }

    #[test]
    fn a_first_op_that_claims_seq_one_must_start_from_nothing() {
        let me = id(15);
        let live = live(&me, 2);
        // an op of seq 1 whose prev_hash is not zero: re-sign a tampered copy with the real key
        let mut op = decode_op(&live.chain[0].canonical, live.chain[0].signature64()).unwrap();
        op.prev_hash = [9; 32];
        let canon = op.canonical_bytes();
        let sig = me.sign(&canon);
        let mut s = PhoneState::restored();
        let err = adopt_own_chain(&mut s, &me, &[(canon, sig)], None, None).unwrap_err();
        assert!(err.reason().contains("starts from nothing"));
    }

    fn tb(count: u8) -> Option<TerminalsBlock> {
        Some(TerminalsBlock {
            count,
            newest_op_ts: NOW,
        })
    }

    fn restored_with_list() -> (PhoneIdentity, PhoneState, Vec<SignedOpBytes>, Vec<u8>) {
        let me = id(21);
        let live = live(&me, 3);
        let fresh = PhoneState {
            head_pending: true,
            ..PhoneState::default()
        };
        (me, fresh, courier_ops(&live, 8), live.chain[2].hash.clone())
    }

    #[test]
    fn n2_with_one_terminal_unreachable_accepts_one_list_with_the_tick() {
        let (me, mut s, ops, hash) = restored_with_list();
        let r = adopt_own_chain(&mut s, &me, &ops, Some([1; 8]), tb(2)).unwrap();
        assert!(r.agreed && r.single_source);
        assert_eq!(r.terminals, tb(2));
        let e = accept_head(&mut s, (3, &hash), false).unwrap_err();
        assert_eq!(e.reason(), SINGLE_SOURCE_TICK_MESSAGE);
        assert!(
            s.head_pending && s.head_proposal.is_some(),
            "a refusal keeps the proposal"
        );
        assert_eq!(accept_head(&mut s, (3, &hash), true).unwrap(), (3, 3));
        assert!(!s.head_pending);
    }

    #[test]
    fn a_hub_plus_one_laptop_agree_without_a_tick() {
        let (me, mut s, ops, hash) = restored_with_list();
        let hub = adopt_own_chain(&mut s, &me, &ops, Some([1; 8]), tb(2)).unwrap();
        assert!(hub.single_source);
        let laptop = adopt_own_chain(&mut s, &me, &ops, Some([2; 8]), tb(2)).unwrap();
        assert!(laptop.agreed && !laptop.single_source);
        assert_eq!(accept_head(&mut s, (3, &hash), false).unwrap(), (3, 3));
    }

    #[test]
    fn a_list_without_a_terminals_block_still_needs_two_lists() {
        let (me, mut s, ops, _) = restored_with_list();
        let r = adopt_own_chain(&mut s, &me, &ops, Some([1; 8]), None).unwrap();
        assert!(!r.agreed && !r.single_source && s.head_proposal.is_none());
        // a count of zero cannot come from an honest terminal (it counts itself): treated as no block
        let z = adopt_own_chain(&mut s, &me, &ops, Some([3; 8]), tb(0)).unwrap();
        assert!(
            z.agreed && !z.single_source,
            "a second sender agrees the usual way"
        );
    }

    #[test]
    fn the_same_terminal_twice_never_upgrades_a_single_list_proposal() {
        let (me, mut s, ops, hash) = restored_with_list();
        adopt_own_chain(&mut s, &me, &ops, Some([1; 8]), tb(1)).unwrap();
        let e = adopt_own_chain(&mut s, &me, &ops, Some([1; 8]), tb(1)).unwrap_err();
        assert_eq!(e.reason(), SAME_TERMINAL_MESSAGE);
        assert!(s.head_proposal.as_ref().unwrap().single_source);
        assert!(accept_head(&mut s, (3, &hash), false).is_err());
    }
}
