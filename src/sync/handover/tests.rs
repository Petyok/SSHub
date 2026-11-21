use super::*;
use crate::sync::protocol::tests::Dev;

#[test]
fn the_words_are_the_proto_crates_and_a_malformed_old_phone_id_gives_none() {
    let new = Dev::new();
    let old = Dev::new();
    let words = handover_words(&new.id.public(), "my new phone", &old.did);
    assert_eq!(words.len(), crate::sync::limits::HANDOVER_WORDS);
    let raw: [u8; 8] = hex::decode(&old.did).unwrap().try_into().unwrap();
    assert_eq!(
        words,
        crate::sync::crypto::handover_words(&new.id.public(), "my new phone", &raw)
    );
    assert_ne!(
        words,
        handover_words(&new.id.public(), "another name", &old.did),
        "the name is bound"
    );
    assert!(handover_words(&new.id.public(), "x", &"not hex".to_string()).is_empty());
}

#[test]
fn the_request_names_the_terminal_and_refuses_a_bad_name() {
    let new = Dev::new();
    let term = Dev::new();
    let req = handover_request(new.id.public(), "phone", &term.did).unwrap();
    assert_eq!(hex::encode(req.terminal), term.did);
    assert_eq!(req.name, "phone");
    assert!(handover_request(new.id.public(), "", &term.did).is_err());
    assert!(handover_request(new.id.public(), "two\nlines", &term.did).is_err());
    assert!(handover_request(new.id.public(), &"x".repeat(65), &term.did).is_err());
    assert!(handover_request(new.id.public(), "ok", &"zz".to_string()).is_err());
    // It survives the wire.
    let back = PhoneHandoverRequestWire::decode(&req.encode()).unwrap();
    assert_eq!(back, req);
}

#[test]
fn the_admission_frame_carries_the_op_hash_and_lamport() {
    let new = Dev::new();
    let phone = Dev::new();
    let term = Dev::new();
    let op = phone
        .store
        .sync_append_local(
            &phone.id,
            OpBody::PhoneHandover {
                new_phone: new.id.public(),
                name: "next".into(),
            },
        )
        .unwrap();
    let req = handover_admission_request(&op, &term.did).unwrap();
    assert_eq!(req.op, Some((op.hash(), op.lamport)));
    assert_eq!(req.new_phone, new.id.public());
    // Stage 1 has no trailer; both survive the wire.
    let stage1 = handover_request(new.id.public(), "next", &term.did).unwrap();
    assert_eq!(stage1.op, None);
    for r in [&req, &stage1] {
        assert_eq!(&PhoneHandoverRequestWire::decode(&r.encode()).unwrap(), r);
    }
    // Any other op is refused.
    let other = phone
        .store
        .sync_append_local(&phone.id, OpBody::RedApprove { op_hash: [1; 32] })
        .unwrap();
    assert!(handover_admission_request(&other, &term.did).is_err());
}
