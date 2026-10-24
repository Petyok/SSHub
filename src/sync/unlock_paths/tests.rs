use super::*;
use crate::sync::cred_ops::cred_put;
use crate::sync::crypto::{bip39_encode, DeviceIdentity};
use crate::sync::protocol::tests::ring;
use crate::sync::quorum::tests::{all_to_all, payload, quorum_mode};
use crate::sync::quorum::{assemble_dek, make_challenge, own_shares};
use crate::sync::share_release::share_to_bytes;
use crate::sync::snapshot::snapshot_seal;
use crate::sync::wire::ShareResponseWire;

struct Fixture {
    d: Vec<crate::sync::protocol::tests::Dev>,
    host: HostUid,
    challenge: Challenge,
    eph: [u8; 32],
    phone_share: Share,
}

fn fixture() -> Fixture {
    let d = ring(3); // 0 = requester, 1 = phone, 2 = another holder
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [71u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("phone-secret")).unwrap();
    all_to_all(&d);
    let (challenge, eph) =
        make_challenge(&d[0].store, &d[0].id, &host, &d[1].did, 1_000, 120).unwrap();
    let phone_share = own_shares(&d[1].store, &d[1].id, &host)
        .unwrap()
        .remove(0)
        .1;
    Fixture {
        d,
        host,
        challenge,
        eph: *eph,
        phone_share,
    }
}

/// What the phone page builds: share, then the extension, sealed to the challenge's ephemeral key.
fn reply(f: &Fixture, ext: Option<ReplyExt>, request_id: u64) -> Vec<u8> {
    let mut plain = share_to_bytes(&f.phone_share);
    if let Some(e) = ext {
        plain.extend_from_slice(&e.encode());
    }
    let ciphertext = DeviceIdentity::seal_to(&f.challenge.eph_x25519_pub, &plain);
    ShareResponseWire {
        request_id,
        ciphertext,
    }
    .encode()
}

fn signed_ext(f: &Fixture, snapshot: Option<([u8; 16], Vec<u8>)>) -> ReplyExt {
    ReplyExt {
        approval_sig: f.d[1].id.sign(&f.challenge.binding_preimage()),
        ops: vec![],
        snapshot,
    }
}

#[test]
fn the_clipboard_text_is_the_base64_of_the_whole_response_and_opens_the_credential() {
    let f = fixture();
    let phone_pub = f.d[1].id.public().ed25519_pub;
    let bytes = reply(&f, Some(signed_ext(&f, None)), f.challenge.request_id);
    let text = B64.encode(&bytes);
    // Pasted with line breaks, as a terminal may wrap it.
    let wrapped: String = text
        .as_bytes()
        .chunks(60)
        .map(|c| format!("{}\n", std::str::from_utf8(c).unwrap()))
        .collect();
    let got = open_phone_reply_clipboard(&wrapped, &f.challenge, &f.eph, &phone_pub).unwrap();
    assert_eq!(got.share, f.phone_share);
    assert!(got.ext.is_some());
    let mine = own_shares(&f.d[0].store, &f.d[0].id, &f.host)
        .unwrap()
        .remove(0)
        .1;
    let dek = assemble_dek(&f.d[0].store, &f.host, mine, got.share).unwrap();
    assert_eq!(dek.len(), 32);
}

#[test]
fn a_reply_is_refused_when_it_is_for_another_request_unsigned_or_damaged() {
    let f = fixture();
    let phone_pub = f.d[1].id.public().ed25519_pub;
    let open = |b: &[u8]| open_phone_reply(b, &f.challenge, &f.eph, &phone_pub);
    // The audio reply: the share only, no extension, accepted.
    assert!(open(&reply(&f, None, f.challenge.request_id))
        .unwrap()
        .ext
        .is_none());
    assert!(
        open(&reply(&f, None, f.challenge.request_id + 1)).is_err(),
        "another request id"
    );
    // An extension signed by someone else, or for another challenge, takes the whole reply with it.
    let mut forged = signed_ext(&f, None);
    forged.approval_sig = f.d[2].id.sign(&f.challenge.binding_preimage());
    assert!(open(&reply(&f, Some(forged), f.challenge.request_id)).is_err());
    let (other, _) =
        make_challenge(&f.d[0].store, &f.d[0].id, &f.host, &f.d[1].did, 1_000, 120).unwrap();
    let mut replay = signed_ext(&f, None);
    replay.approval_sig = f.d[1].id.sign(&other.binding_preimage());
    assert!(
        open(&reply(&f, Some(replay), f.challenge.request_id)).is_err(),
        "approval for another challenge"
    );
    // Damage.
    let mut bytes = reply(&f, None, f.challenge.request_id);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(open(&bytes).is_err());
    assert!(open_phone_reply_clipboard("not base64 !!", &f.challenge, &f.eph, &phone_pub).is_err());
    assert!(
        open_phone_reply(
            &reply(&f, None, f.challenge.request_id),
            &f.challenge,
            &[1u8; 32],
            &phone_pub
        )
        .is_err(),
        "another ephemeral key"
    );
}

#[test]
fn a_snapshot_from_a_reply_opens_later_with_the_sixteen_character_code() {
    let f = fixture();
    let code = crate::sync::codes::gen_snapshot_code();
    let now = crate::store::now_ts();
    let blob = snapshot_seal(&f.phone_share, &code, now).to_bytes();
    let ext = signed_ext(&f, Some((f.host, blob)));
    let sum = apply_reply_ext(&f.d[0].store, &ext).unwrap();
    assert_eq!(sum.received, 0);
    // Typed in lower case with spaces and the look-alike letters, the way people do.
    let typed = code.to_lowercase().replace('-', " ");
    assert_eq!(
        share_from_code(&f.d[0].store, &f.host, &typed, now + 10).unwrap(),
        f.phone_share
    );
    assert!(
        share_from_code(&f.d[0].store, &f.host, "0000-0000-0000-0000", now + 10).is_err(),
        "wrong code"
    );
    assert!(
        share_from_code(&f.d[0].store, &f.host, &code, now + 8 * 86_400).is_err(),
        "expired after 7 days"
    );
    assert!(
        share_from_code(&f.d[0].store, &[1; 16], &code, now).is_err(),
        "no snapshot for another credential"
    );
    assert!(remember_snapshot(&f.d[0].store, &f.host, b"junk").is_err());
}

#[test]
fn twenty_four_words_are_the_share_bytes_and_the_index_comes_from_the_sharing() {
    let f = fixture();
    let words = bip39_encode(f.phone_share.bytes.as_slice().try_into().unwrap());
    let got = full_entry_share(&f.d[0].store, &f.host, 0, &f.d[1].did, &words).unwrap();
    assert_eq!(
        got, f.phone_share,
        "bytes from the words, index from the holder's position"
    );
    let mine = own_shares(&f.d[0].store, &f.d[0].id, &f.host)
        .unwrap()
        .remove(0)
        .1;
    assert!(assemble_dek(&f.d[0].store, &f.host, mine, got).is_ok());
    assert!(
        full_entry_share(
            &f.d[0].store,
            &f.host,
            0,
            &"ffffffffffffffff".to_string(),
            &words
        )
        .is_err(),
        "not a holder"
    );
    assert!(full_entry_share(
        &f.d[0].store,
        &f.host,
        0,
        &f.d[1].did,
        "abandon abandon abandon"
    )
    .is_err());
    let mut wrong = words.split(' ').map(String::from).collect::<Vec<_>>();
    wrong.swap(0, 1);
    assert!(
        full_entry_share(&f.d[0].store, &f.host, 0, &f.d[1].did, &wrong.join(" ")).is_err() || {
            let s =
                full_entry_share(&f.d[0].store, &f.host, 0, &f.d[1].did, &wrong.join(" ")).unwrap();
            let mine = own_shares(&f.d[0].store, &f.d[0].id, &f.host)
                .unwrap()
                .remove(0)
                .1;
            assemble_dek(&f.d[0].store, &f.host, mine, s).is_err()
        }
    );
}

#[test]
fn typed_paper_words_are_the_second_share_for_a_lone_terminal_and_wrong_words_open_nothing() {
    use crate::sync::quorum::paper_registration;
    let d = ring(3);
    let phrase = bip39_encode(&[0x61; 32]);
    // The paper is published through the log: only d[0] ever typed it.
    crate::sync::quorum::tests::append(&d[0], paper_registration(&phrase).unwrap());
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [72u8; 16];
    // d[1] never saw the phrase and still wraps the credential to the paper.
    cred_put(&d[1].store, &d[1].id, &host, &payload("lone")).unwrap();
    all_to_all(&d);
    let (epoch, mine) = own_shares(&d[2].store, &d[2].id, &host).unwrap().remove(0);
    let second = paper_second_share(&d[2].store, &host, epoch, &phrase).unwrap();
    let dek = assemble_dek(&d[2].store, &host, mine, second).unwrap();
    let (ct, _) = d[2].store.sync_cred_ciphertext(&host).unwrap().unwrap();
    assert!(crate::sync::crypto::dek_open(&dek, &ct).is_ok());
    // Another phrase is no key to any share.
    let other = bip39_encode(&[0x62; 32]);
    assert!(paper_second_share(&d[2].store, &host, epoch, &other).is_err());
}

#[test]
fn a_ring_of_exactly_two_holders_needs_the_paper_before_credentials_or_a_mode_change() {
    use crate::sync::quorum::paper_registration;
    let d = ring(2);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [73u8; 16];
    let err = cred_put(&d[0].store, &d[0].id, &host, &payload("x")).unwrap_err();
    assert!(format!("{err:#}").contains("paper"), "{err:#}");
    crate::sync::quorum::tests::append(
        &d[0],
        paper_registration(&bip39_encode(&[0x63; 32])).unwrap(),
    );
    cred_put(&d[0].store, &d[0].id, &host, &payload("x")).unwrap();
}
