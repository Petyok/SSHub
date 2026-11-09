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
