//! The terminal side against a phone written the way the page does it (JOINING = initiator).

use super::*;
use crate::sync::protocol::tests::{ring, Dev};

/// The phone half, in Rust: what `app.mjs` does step by step.
struct Phone {
    id: DeviceIdentity,
    party: Party,
}

impl Phone {
    fn new() -> Self {
        let id = DeviceIdentity::generate();
        let party = Party::new(&id, "my phone", true);
        Self { id, party }
    }

    fn commit(&self) -> Vec<u8> {
        EnrollFrame::Commit {
            role: INITIATOR,
            commit: self.party.commit(INITIATOR),
        }
        .encode()
    }

    fn reveal(&self) -> Vec<u8> {
        EnrollFrame::Reveal {
            role: INITIATOR,
            reveal: self.party.reveal.bytes(),
        }
        .encode()
    }

    fn confirm(&self, th: &[u8; 32], ok: bool) -> Vec<u8> {
        EnrollFrame::Confirm {
            role: INITIATOR,
            ok,
            sig: self.id.sign(&confirm_message(INITIATOR, ok, th)),
        }
        .encode()
    }
}

fn shown(step: Step) -> Vec<u8> {
    match step {
        Step::Show(f) => f,
        other => panic!("expected a frame to show, got {other:?}"),
    }
}

/// Run the exchange up to the words; returns (terminal state machine, phone, transcript hash, words).
fn up_to_the_words(t: &Dev) -> (PhoneEnroll, Phone, [u8; 32], String) {
    let phone = Phone::new();
    let (mut enroll, first) = PhoneEnroll::begin(&t.id, "terminal");
    // The phone checks the terminal commit, shows its own; the terminal scans it and shows its reveal.
    let EnrollFrame::Commit { role, commit } = EnrollFrame::decode(&first).unwrap() else {
        panic!("first frame is a commitment")
    };
    assert_eq!(role, RESPONDER);
    let reveal = shown(enroll.scan(&t.id, &phone.commit()).unwrap());
    let EnrollFrame::Reveal { reveal: tr, .. } = EnrollFrame::decode(&reveal).unwrap() else {
        panic!("second frame is a reveal")
    };
    let term_reveal = Reveal::from_bytes(&tr).unwrap();
    check_reveal(RESPONDER, &commit, &term_reveal).unwrap();
    let th = transcript_hash(&phone.party.reveal, &term_reveal);
    let Step::Compare { words, phone_name } = enroll.scan(&t.id, &phone.reveal()).unwrap() else {
        panic!("the words come after the phone's reveal")
    };
    assert_eq!(phone_name, "my phone");
    assert_eq!(words, sas_words(&th), "both sides derive the same words");
    (enroll, phone, th, words)
}

#[test]
fn the_phone_is_verified_registered_and_proposed_as_a_red_holder() {
    let d = ring(1);
    let t = &d[0];
    let (mut enroll, phone, th, _) = up_to_the_words(t);
    assert_eq!(enroll.answer(&t.id, true).unwrap(), Step::Wait);
    let Step::Verified(v) = enroll.scan(&t.id, &phone.confirm(&th, true)).unwrap() else {
        panic!("the phone's confirmation finishes it")
    };
    // The terminal's own confirmation verifies on the phone's side.
    let EnrollFrame::Confirm { role, ok, sig } = EnrollFrame::decode(&v.frame).unwrap() else {
        panic!("confirm frame")
    };
    assert!(ok && role == RESPONDER);
    assert!(verify(
        &t.id.public().ed25519_pub,
        &confirm_message(RESPONDER, true, &th),
        &sig
    ));
    let op = admit_phone(&t.store, &t.id, &v).unwrap();
    assert!(matches!(
        &op.body,
        OpBody::DeviceAdd {
            kind: DeviceKind::Phone,
            role: DeviceRole::Holder,
            ..
        }
    ));
    assert_eq!(
        crate::sync::quorum::phone_device(&t.store).unwrap(),
        Some(phone.id.public().device_id())
    );
    // A second phone is a recovery, not an enrollment.
    assert!(admit_phone(&t.store, &t.id, &v).is_err());
}

#[test]
fn the_phone_may_confirm_before_the_user_answers() {
    let d = ring(1);
    let t = &d[0];
    let (mut enroll, phone, th, _) = up_to_the_words(t);
    assert_eq!(
        enroll.scan(&t.id, &phone.confirm(&th, true)).unwrap(),
        Step::Wait
    );
    assert!(matches!(
        enroll.answer(&t.id, true).unwrap(),
        Step::Verified(_)
    ));
}

#[test]
fn different_words_stop_both_sides_and_register_nothing() {
    let d = ring(1);
    let t = &d[0];
    let (mut enroll, _phone, th, _) = up_to_the_words(t);
    let Step::Stopped { frame: Some(f), .. } = enroll.answer(&t.id, false).unwrap() else {
        panic!("a decline shows the phone a declining confirmation")
    };
    let EnrollFrame::Confirm { ok, sig, .. } = EnrollFrame::decode(&f).unwrap() else {
        panic!("confirm frame")
    };
    assert!(!ok);
    assert!(verify(
        &t.id.public().ed25519_pub,
        &confirm_message(RESPONDER, false, &th),
        &sig
    ));
    assert!(enroll.answer(&t.id, true).is_err(), "stopped for good");
    assert!(crate::sync::quorum::phone_device(&t.store)
        .unwrap()
        .is_none());
}

#[test]
fn a_phone_that_declines_or_forges_its_confirmation_does_not_get_in() {
    let d = ring(1);
    let t = &d[0];
    let (mut enroll, phone, th, _) = up_to_the_words(t);
    enroll.answer(&t.id, true).unwrap();
    let Step::Stopped { why, .. } = enroll.scan(&t.id, &phone.confirm(&th, false)).unwrap() else {
        panic!("a declining phone stops it")
    };
    assert!(why.contains("differ"), "{why}");

    let (mut enroll, _phone, th, _) = up_to_the_words(t);
    enroll.answer(&t.id, true).unwrap();
    let stranger = Phone::new();
    let Step::Stopped { why, .. } = enroll.scan(&t.id, &stranger.confirm(&th, true)).unwrap()
    else {
        panic!("a confirmation by another key stops it")
    };
    assert!(why.contains("not signed"), "{why}");
}

#[test]
fn a_reveal_that_does_not_open_its_commitment_is_refused_and_frames_out_of_order_are_errors() {
    let d = ring(1);
    let t = &d[0];
    let phone = Phone::new();
    let (mut enroll, _) = PhoneEnroll::begin(&t.id, "terminal");
    assert!(
        enroll.scan(&t.id, &phone.reveal()).is_err(),
        "a reveal before any commitment"
    );
    enroll.scan(&t.id, &phone.commit()).unwrap();
    // Another phone's reveal under the first phone's commitment: someone is in the middle.
    let other = Phone::new();
    let Step::Stopped { why, .. } = enroll.scan(&t.id, &other.reveal()).unwrap() else {
        panic!("a reveal that does not open the commitment stops it")
    };
    assert!(why.contains("commitment"), "{why}");
    assert!(enroll.scan(&t.id, &phone.commit()).is_err());
    assert!(Reveal::from_bytes(&phone.party.reveal.bytes()[..40]).is_err());
}

#[test]
fn pasted_phone_ops_reach_the_log_once() {
    use crate::sync::wire::PhoneOpsWire;
    let d = ring(2);
    d[1].append_host();
    let (canonical, sig) = {
        let tail = d[1].store.sync_chain_tail(&d[1].did, 0).unwrap();
        let op = tail.last().unwrap();
        (op.canonical_bytes(), op.signature)
    };
    let frame = PhoneOpsWire {
        ops: vec![(canonical, sig)],
    }
    .encode();
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let text = B64.encode(&frame);
    let sum = crate::sync::unlock_paths::phone_ops_from_text(&d[0].store, &text).unwrap();
    assert_eq!(sum.received, 1);
    let again = crate::sync::unlock_paths::phone_ops_from_text(&d[0].store, &text).unwrap();
    assert_eq!(again.duplicates, 1);
    assert!(crate::sync::unlock_paths::phone_ops_from_text(&d[0].store, "not base64!").is_err());
}
