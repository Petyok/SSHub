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

/// This phone's device id as the 8 bytes the compare words bind (it is the OLD phone: the one being handed over).
fn own_id8(state: &PhoneState) -> Result<[u8; 8]> {
    let e = state.enrolled.as_ref().ok_or_else(|| {
        CoreError::refused("This phone is not part of a ring yet: there is nothing to hand over.")
    })?;
    hex::decode(&e.device_id)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| CoreError::internal("the stored device id is not 8 bytes"))
}

/// Everything about a request that makes the phone refuse before it shows or signs anything.
fn check_request(state: &PhoneState, req: &PhoneHandoverRequestWire) -> Result<()> {
    if check_name(&req.name)? != req.name {
        return Err(CoreError::refused(
            "The new phone's name has spaces around it: ask the terminal to show it again.",
        ));
    }
    req.new_phone
        .validate()
        .map_err(|e| CoreError::refused(format!("The new phone's keys are not valid: {e:#}")))?;
    if state
        .enrolled
        .as_ref()
        .is_some_and(|e| e.device_id == req.new_phone.device_id())
    {
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
    req: &PhoneHandoverRequestWire,
) -> Result<HandoverView> {
    let old = own_id8(state)?;
    check_request(state, req)?;
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
    })
}

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
    own_id8(state)?;
    check_request(state, req)?;
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

/// The NEW phone's own `RedPhoneSign` over the handover op (one of the proofs the ring counts). Returns the kind 5 list
/// to carry to a terminal.
pub fn sign_handover_admission(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    handover_op_hash: [u8; 32],
    now: i64,
) -> Result<Vec<u8>> {
    chain::red_phone_sign(state, id, handover_op_hash, now, None)?;
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
        let mut s = PhoneState::default();
        s.enrolled = Some(Enrolled {
            device_id: id.device_id().to_string(),
            name: "old phone".into(),
            at: NOW,
        });
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
        let v = describe_phone_handover(&s, &req("new phone")).unwrap();
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
        let (_, mut other) = old();
        other.enrolled.as_mut().unwrap().device_id = "0102030405060708".into();
        assert_ne!(
            describe_phone_handover(&other, &req("new phone"))
                .unwrap()
                .words,
            v.words
        );
        // and the name
        assert_ne!(
            describe_phone_handover(&s, &req("new phonf"))
                .unwrap()
                .words,
            v.words
        );
    }

    #[test]
    fn a_request_that_is_not_right_is_refused_before_anything_is_shown_or_signed() {
        let (id, mut s) = old();
        for bad in ["", "  ", " padded ", "tab\there", &"x".repeat(65)] {
            assert!(describe_phone_handover(&s, &req(bad)).is_err(), "{bad:?}");
            assert!(
                build_phone_handover(&mut s, &id, &req(bad), NOW).is_err(),
                "{bad:?}"
            );
        }
        let mut broken = req("new phone");
        broken.new_phone.mlkem_pub.pop();
        assert!(reason(describe_phone_handover(&s, &broken)).contains("keys are not valid"));
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

    #[test]
    fn the_new_phone_signs_its_own_admission_over_the_op_hash() {
        let nid = PhoneIdentity::new(&[0x55; 32], 0).unwrap();
        let mut s = PhoneState::default();
        let frame = sign_handover_admission(&mut s, &nid, [9; 32], NOW).unwrap();
        let ops = PhoneOpsWire::decode(&frame).unwrap().ops;
        let op = decode_op(&ops.last().unwrap().0, ops.last().unwrap().1).unwrap();
        assert!(op.verify(&nid.public().ed25519_pub));
        assert!(matches!(op.body, OpBody::RedPhoneSign { op_hash } if op_hash == [9; 32]));
        s.head_pending = true;
        assert!(sign_handover_admission(&mut s, &nid, [9; 32], NOW + 1).is_err());
    }
}
