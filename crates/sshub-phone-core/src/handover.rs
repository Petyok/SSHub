//! PhoneHandover on the phone (design 5, contract 4j): the OLD phone signs the op that names its successor, and the NEW
//! phone signs its own admission (`RedPhoneSign`). The eight compare words come from the proto crate
//! (`handover_words`), the same function the terminal's screen uses, so the two screens cannot disagree.
//!
//! The ring decides whether the handover counts (a second attended terminal and the paper phrase must also approve it);
//! the phone only authors and signs its part, and refuses what it knows is wrong.

use sshub_sync_proto::crypto::handover_words;
use sshub_sync_proto::oplog::{canonical::decode_op, OpBody};
use sshub_sync_proto::wire::{PhoneHandoverRequestWire, PhoneOpsWire};

use crate::cert::issued_cert_records;
use crate::chain;
use crate::enroll::check_name;
use crate::error::{CoreError, Result};
use crate::identity::PhoneIdentity;
use crate::state::{PhoneState, COURIER_TAIL};
use crate::text::safe_text;
use crate::views::HandoverView;

/// This phone's OWN device id as the 8 bytes the compare words bind (it is the OLD phone: the one being handed over).
/// `state.enrolled` only says the phone is in a ring: it names the terminal, not this phone.
fn own_id8(state: &PhoneState, id: &PhoneIdentity) -> Result<[u8; 8]> {
    if state.enrolled.is_none() {
        return Err(CoreError::refused(
            "This phone is not part of a ring yet: there is nothing to hand over.",
        ));
    }
    hex::decode(id.device_id())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| CoreError::internal("this phone's device id is not 8 bytes"))
}

/// Everything about a request that makes the phone refuse before it shows or signs anything.
fn check_request(
    state: &PhoneState,
    id: &PhoneIdentity,
    req: &PhoneHandoverRequestWire,
) -> Result<()> {
    if check_name(&req.name)? != req.name {
        return Err(CoreError::refused(
            "The new phone's name has spaces around it: ask the terminal to show it again.",
        ));
    }
    req.new_phone
        .validate()
        .map_err(|e| CoreError::refused(format!("The new phone's keys are not valid: {e:#}")))?;
    if state.enrolled.is_some() && id.device_id() == req.new_phone.device_id() {
        return Err(CoreError::refused(
            "The request names this phone as its own successor.",
        ));
    }
    Ok(())
}

/// What the old phone shows: the new phone's name, the eight words to compare with the terminal's screen, and the trust
/// sessions that stay valid for the successor (design 5.4).
pub fn describe_phone_handover(
    state: &PhoneState,
    id: &PhoneIdentity,
    req: &PhoneHandoverRequestWire,
) -> Result<HandoverView> {
    if req.op.is_some() {
        return Err(CoreError::refused(
            "This is the admission frame for the new phone, not a request to the old one.",
        ));
    }
    let old = own_id8(state, id)?;
    check_request(state, id, req)?;
    let certs_carried = issued_cert_records(state)
        .iter()
        .filter(|c| !c.revoked)
        .map(|c| {
            format!(
                "{} [{}]",
                safe_text(&crate::cert::describe_cert(c, &state.aliases)),
                hex::encode(c.id)
            )
        })
        .collect();
    Ok(HandoverView {
        name: safe_text(&req.name),
        words: handover_words(&req.new_phone, &req.name, &old),
        certs_carried,
        op_hash: None,
        op_lamport: None,
    })
}

/// The admission frame the NEW phone scans (contract 4l b): the same request plus the hash and lamport of the ingested
/// `PhoneHandover` op. It is refused unless it names THIS phone's own keys, so a frame meant for another phone never
/// gets this phone's signature.
fn check_admission(id: &PhoneIdentity, req: &PhoneHandoverRequestWire) -> Result<([u8; 32], u64)> {
    let Some((hash, lamport)) = req.op else {
        return Err(CoreError::refused(
            "This request carries no handover operation: it is for the old phone.",
        ));
    };
    if req.new_phone != id.public() {
        return Err(CoreError::refused(
            "This handover names another phone, not this one. Do not sign it.",
        ));
    }
    check_name(&req.name).map_err(|e| CoreError::refused(e.reason().to_string()))?;
    if lamport > MAX_ADMISSION_OP_LAMPORT {
        return Err(CoreError::refused(
            "The handover operation names an impossible counter: ask the terminal to show it again.",
        ));
    }
    Ok((hash, lamport))
}

/// What the new phone shows before it signs its admission: the name and the operation it will sign over.
pub fn describe_handover_admission(
    id: &PhoneIdentity,
    req: &PhoneHandoverRequestWire,
) -> Result<HandoverView> {
    let (hash, lamport) = check_admission(id, req)?;
    Ok(HandoverView {
        name: safe_text(&req.name),
        words: Vec::new(),
        certs_carried: Vec::new(),
        op_hash: Some(hash.to_vec()),
        op_lamport: Some(lamport),
    })
}

/// The lamport the terminal reports is unsigned: a forged huge one would push this phone's counters out of range, so
/// anything a ring could not have reached is refused (2^32 ops).
pub const MAX_ADMISSION_OP_LAMPORT: u64 = u32::MAX as u64;

fn handover_in_chain(state: &PhoneState) -> bool {
    state.chain.iter().any(|e| {
        decode_op(&e.canonical, e.signature64())
            .is_ok_and(|op| matches!(op.body, OpBody::PhoneHandover { .. }))
    })
}

fn ops_frame(state: &PhoneState) -> Vec<u8> {
    PhoneOpsWire {
        sender: None,
        terminals: None,
        ops: chain::courier_ops(state, COURIER_TAIL),
    }
    .encode()
}

/// Signs the `PhoneHandover` op naming the new phone (the author proof) and returns the kind 5 list to carry to a
/// terminal. One handover per phone: a second one in the same chain would be a rule violation, so it is refused here.
pub fn build_phone_handover(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    req: &PhoneHandoverRequestWire,
    now: i64,
) -> Result<Vec<u8>> {
    if req.op.is_some() {
        return Err(CoreError::refused(
            "This is the admission frame for the new phone: only the new phone signs it.",
        ));
    }
    own_id8(state, id)?;
    check_request(state, id, req)?;
    if handover_in_chain(state) {
        return Err(CoreError::refused(
            "This phone already signed a handover. Only one counts; ask the terminal to finish it or cancel it.",
        ));
    }
    chain::phone_handover(
        state,
        id,
        req.new_phone.clone(),
        req.name.clone(),
        now,
        None,
    )?;
    Ok(ops_frame(state))
}

/// The NEW phone's own `RedPhoneSign` over the handover op (one of the proofs the ring counts), from the admission frame
/// (contract 4l b). Its lamport is `max(own next, op_lamport + 1)`: a proof written before the op it names would not
/// count on the host. Returns the kind 5 list to carry to a terminal.
pub fn sign_handover_admission(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    req: &PhoneHandoverRequestWire,
    now: i64,
) -> Result<Vec<u8>> {
    let (hash, lamport) = check_admission(id, req)?;
    chain::red_phone_sign(
        state,
        id,
        hash,
        now,
        Some(chain::Seen { lamport, epoch: 0 }),
    )?;
    Ok(ops_frame(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Enrolled;
    use sshub_sync_proto::crypto::DevicePublic;
    use sshub_sync_proto::oplog::CertScope;

    const NOW: i64 = 1_893_452_400;

    fn old() -> (PhoneIdentity, PhoneState) {
        let id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
        let s = PhoneState {
            enrolled: Some(Enrolled {
                terminal_id: "aaaaaaaaaaaaaaaa".into(),
                name: "old phone".into(),
                at: NOW,
            }),
            ..PhoneState::default()
        };
        (id, s)
    }

    fn new_phone() -> DevicePublic {
        PhoneIdentity::new(&[0x55; 32], 0).unwrap().public()
    }

    fn req(name: &str) -> PhoneHandoverRequestWire {
        PhoneHandoverRequestWire {
            new_phone: new_phone(),
            name: name.into(),
            terminal: [7; 8],
            op: None,
        }
    }

    fn reason<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().reason().to_string()
    }

    #[test]
    fn the_view_shows_the_proto_words_the_name_and_the_certs_that_stay_valid() {
        let (id, mut s) = old();
        chain::cert_issue(
            &mut s,
            &id,
            "cccccccccccccccc".into(),
            CertScope::Hosts(vec![[1; 16]]),
            NOW + 86_400,
            2,
            0,
            1,
            NOW,
            None,
        )
        .unwrap();
        let v = describe_phone_handover(&s, &id, &req("new phone")).unwrap();
        let want = handover_words(
            &new_phone(),
            "new phone",
            &hex::decode(id.device_id()).unwrap().try_into().unwrap(),
        );
        assert_eq!(v.words, want);
        assert_eq!(v.words.len(), sshub_sync_proto::limits::HANDOVER_WORDS);
        assert_eq!(v.name, "new phone");
        assert_eq!(v.certs_carried.len(), 1);
        assert!(v.certs_carried[0].contains("cccccccccccccccc"));
        // the words bind the old phone: another old phone shows other words
        let other_id = PhoneIdentity::new(&[0x43; 32], 0).unwrap();
        assert_ne!(
            describe_phone_handover(&s, &other_id, &req("new phone"))
                .unwrap()
                .words,
            v.words
        );
        // and the name
        assert_ne!(
            describe_phone_handover(&s, &id, &req("new phonf"))
                .unwrap()
                .words,
            v.words
        );
    }

    #[test]
    fn a_request_that_is_not_right_is_refused_before_anything_is_shown_or_signed() {
        let (id, mut s) = old();
        for bad in ["", "  ", " padded ", "tab\there", &"x".repeat(65)] {
            assert!(
                describe_phone_handover(&s, &id, &req(bad)).is_err(),
                "{bad:?}"
            );
            assert!(
                build_phone_handover(&mut s, &id, &req(bad), NOW).is_err(),
                "{bad:?}"
            );
        }
        let mut broken = req("new phone");
        broken.new_phone.mlkem_pub.pop();
        assert!(reason(describe_phone_handover(&s, &id, &broken)).contains("keys are not valid"));
        let mut me = req("new phone");
        me.new_phone = id.public();
        assert!(reason(build_phone_handover(&mut s, &id, &me, NOW)).contains("its own successor"));
        // a phone that is in no ring has nothing to hand over
        let mut loose = PhoneState::default();
        assert!(reason(build_phone_handover(
            &mut loose,
            &id,
            &req("new phone"),
            NOW
        ))
        .contains("not part of a ring"));
        // a restored phone without its chain head cannot sign
        s.head_pending = true;
        assert!(build_phone_handover(&mut s, &id, &req("new phone"), NOW).is_err());
        s.head_pending = false;
        assert!(s.chain.is_empty(), "refusals sign nothing");
    }

    #[test]
    fn the_old_phone_signs_one_handover_naming_the_new_phone_and_no_second() {
        let (id, mut s) = old();
        let frame = build_phone_handover(&mut s, &id, &req("new phone"), NOW).unwrap();
        let ops = PhoneOpsWire::decode(&frame).unwrap().ops;
        let op = decode_op(&ops.last().unwrap().0, ops.last().unwrap().1).unwrap();
        assert!(op.verify(&id.public().ed25519_pub));
        match op.body {
            OpBody::PhoneHandover { new_phone: n, name } => {
                assert_eq!((n, name.as_str()), (new_phone(), "new phone"));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            reason(build_phone_handover(&mut s, &id, &req("another"), NOW + 1))
                .contains("already signed a handover")
        );
        assert_eq!(s.chain.len(), 1);
    }

    fn admission(op: Option<([u8; 32], u64)>) -> PhoneHandoverRequestWire {
        PhoneHandoverRequestWire {
            op,
            ..req("new phone")
        }
    }

    #[test]
    fn the_new_phone_signs_its_own_admission_after_the_op_it_names() {
        let nid = PhoneIdentity::new(&[0x55; 32], 0).unwrap();
        let mut s = PhoneState::default();
        let r = admission(Some(([9; 32], 700)));
        let v = describe_handover_admission(&nid, &r).unwrap();
        assert_eq!(
            (v.op_hash.clone(), v.op_lamport),
            (Some(vec![9; 32]), Some(700))
        );
        assert!(v.words.is_empty() && v.certs_carried.is_empty());
        let frame = sign_handover_admission(&mut s, &nid, &r, NOW).unwrap();
        let ops = PhoneOpsWire::decode(&frame).unwrap().ops;
        let op = decode_op(&ops.last().unwrap().0, ops.last().unwrap().1).unwrap();
        assert!(op.verify(&nid.public().ed25519_pub));
        assert!(matches!(op.body, OpBody::RedPhoneSign { op_hash } if op_hash == [9; 32]));
        assert_eq!(op.lamport, 701, "after the op it names");
        // a phone that is already far ahead keeps its own next lamport
        let r2 = admission(Some(([8; 32], 3)));
        let frame = sign_handover_admission(&mut s, &nid, &r2, NOW + 1).unwrap();
        let ops = PhoneOpsWire::decode(&frame).unwrap().ops;
        let op = decode_op(&ops.last().unwrap().0, ops.last().unwrap().1).unwrap();
        assert_eq!(op.lamport, 702);
    }

    #[test]
    fn an_admission_for_another_phone_or_without_an_op_is_refused() {
        let nid = PhoneIdentity::new(&[0x55; 32], 0).unwrap();
        let mut s = PhoneState::default();
        // names another phone's keys
        let other = PhoneIdentity::new(&[0x66; 32], 0).unwrap();
        let mut r = admission(Some(([9; 32], 1)));
        r.new_phone = other.public();
        assert!(reason(describe_handover_admission(&nid, &r)).contains("another phone"));
        assert!(reason(sign_handover_admission(&mut s, &nid, &r, NOW)).contains("another phone"));
        // one key of three differs
        let mut r = admission(Some(([9; 32], 1)));
        r.new_phone.x25519_pub[0] ^= 1;
        assert!(sign_handover_admission(&mut s, &nid, &r, NOW).is_err());
        // the old phone's request carries no op
        assert!(
            reason(sign_handover_admission(&mut s, &nid, &admission(None), NOW))
                .contains("no handover operation")
        );
        // the old phone refuses the admission frame, the new phone's checks do not apply to it
        let (_, old_state) = old();
        assert!(reason(describe_phone_handover(
            &old_state,
            &PhoneIdentity::new(&[0x42; 32], 0).unwrap(),
            &admission(Some(([9; 32], 1)))
        ))
        .contains("admission frame"));
        // a forged huge lamport
        assert!(
            sign_handover_admission(&mut s, &nid, &admission(Some(([9; 32], u64::MAX))), NOW)
                .is_err()
        );
        assert!(sign_handover_admission(&mut s, &nid, &admission(Some(([9; 32], 1))), NOW).is_ok());
        s.head_pending = true;
        assert!(
            sign_handover_admission(&mut s, &nid, &admission(Some(([9; 32], 1))), NOW + 1).is_err()
        );
    }
}
