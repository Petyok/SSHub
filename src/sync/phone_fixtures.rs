//! Fixtures for the phone page (`assets/companion/test/fixtures/`): challenges the terminal
//! would show, built with the REAL encoders (`ChallengeWire`, `PhoneMaterial`, `fountain_encode`,
//! the hybrid wrap), plus an enrollment vector and a verifier for what the page sent back.
//!
//! * `SSHUB_EMIT_PHONE_FIXTURES=1 cargo test sync::phone_fixtures` rewrites the JSON files.
//! * Without it the committed files are checked for internal consistency with the Rust code
//!   (frames decode, binding hashes recompute, the phone's identity unwraps its material).
//! * `SSHUB_PHONE_E2E_DIR=<dir>` also runs [`e2e_response_opens_with_the_ephemeral_secret`]: it
//!   reads the replies the page produced (`response-full.bin`, `response-short.bin`,
//!   `snapshot.json`, `cancel-frame.bin`) and opens them the way the terminal would.
//!
//! The enrollment vector is built with the types of `enroll.rs` (`Reveal`, `commit_of`,
//! `transcript_hash`, `confirm_message`).
//!
//! Op bytes are written by [`share_release_op`] in the canonical form of
//! `src/sync/oplog/canonical.rs`; the op-vector replay in `test/phone.test.mjs` pins the page's
//! parser to that package's own vectors.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::crypto::{
    aead_seal, bip39_encode, chain_hash, dek_seal, hybrid_unwrap, hybrid_wrap, sas_words,
    shamir_split, verify, DeviceIdentity, DevicePublic, Share,
};
use super::enroll::{commit_of, confirm_message, transcript_hash, Reveal, INITIATOR, RESPONDER};
use super::fountain::fountain_encode;
use super::limits::QR_PARITY_PERCENT;
use super::share_release::{
    combine_and_check, share_to_bytes, unwrap_share_reply, wrapped_share_plaintext,
};
use super::snapshot::{snapshot_open_for, SnapshotBlob};
use super::wire::{
    binding_preimage, ChallengeWire, EnrollFrame, PhoneMaterial, PhoneOpsWire, RecoveryNoticeWire,
    ShareKind, ShareResponseWire,
};

const NOW: i64 = 1_893_452_500;
const FRAME_BYTES: usize = 84; // a full 80x24 terminal: QR version 5, level M
const PHONE_CONTEXT: &str = "sshub/sync/phone/v1";
const HOST: [u8; 16] = [0x0a; 16];
const OTHER_HOST: [u8; 16] = [0x0b; 16];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/companion/test/fixtures")
}

fn hx(b: &[u8]) -> String {
    hex::encode(b)
}

fn pub_json(p: &DevicePublic) -> Value {
    json!({ "ed25519_pub": hx(&p.ed25519_pub), "x25519_pub": hx(&p.x25519_pub), "mlkem_pub": hx(&p.mlkem_pub), "device_id": p.device_id() })
}

#[allow(clippy::too_many_arguments)]
fn share_release_op(
    device_id: &str,
    seq: u64,
    lamport: u64,
    prev: &[u8; 32],
    ts: i64,
    host: &[u8; 16],
    request_id: u64,
    requester: &str,
    eph: &[u8; 32],
) -> Vec<u8> {
    format!(
        r#"{{"device_id":"{device_id}","seq":{seq},"lamport":{lamport},"prev_hash":"{}","timestamp":{ts},"epoch":0,"alg":[1,1],"body":{{"ShareRelease":{{"host_uid":"{}","request_id":{request_id},"requester":"{requester}","eph_x25519_pub":"{}"}}}}}}"#,
        hx(prev),
        hx(host),
        hx(eph)
    )
    .into_bytes()
}

fn head_attest_op(device_id: &str) -> Vec<u8> {
    format!(
        r#"{{"device_id":"{device_id}","seq":6,"lamport":10,"prev_hash":"{}","timestamp":{},"epoch":0,"alg":[1,1],"body":{{"HeadAttest":{{"device_id":"{device_id}","seq":5,"chain_hash":"{}"}}}}}}"#,
        hx(&[7u8; 32]),
        NOW - 10,
        hx(&[8u8; 32])
    )
    .into_bytes()
}

struct Cast {
    phone: DeviceIdentity,
    requester: DeviceIdentity,
    stranger: DeviceIdentity,
    someone_else: DeviceIdentity,
    eph_secret: [u8; 32],
    eph_pub: [u8; 32],
    dek: [u8; 32],
    phone_share: Share,
    requester_share: Share,
    storage_key: [u8; 32],
}

fn cast() -> Cast {
    let dek = [0x5d; 32];
    let mut shares = shamir_split(&dek, 2, 3);
    let phone_share = shares.remove(1);
    let requester_share = shares.remove(0);
    let eph_secret = [0x33u8; 32];
    let eph_pub =
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(eph_secret)).to_bytes();
    Cast {
        phone: DeviceIdentity::from_seed(&[0x42; 32], PHONE_CONTEXT),
        requester: DeviceIdentity::from_seed(&[0x77; 32], "sshub/test/requester"),
        stranger: DeviceIdentity::from_seed(&[0x78; 32], "sshub/test/stranger"),
        someone_else: DeviceIdentity::from_seed(&[0x79; 32], "sshub/test/someone-else"),
        eph_secret,
        eph_pub,
        dek,
        phone_share,
        requester_share,
        storage_key: [0x6c; 32],
    }
}

fn hybrid_material(c: &Cast, to: &DevicePublic, host: &[u8; 16]) -> PhoneMaterial {
    PhoneMaterial {
        kind: ShareKind::Hybrid,
        epoch: 0,
        share_blob: hybrid_wrap(to, &wrapped_share_plaintext(host, &c.phone_share)),
        storage_blob: vec![],
    }
}

fn envelope_material(c: &Cast, to: &DevicePublic, host: &[u8; 16], epoch: u64) -> PhoneMaterial {
    let aad = [
        host.as_slice(),
        &epoch.to_be_bytes(),
        &[c.phone_share.index],
    ]
    .concat();
    PhoneMaterial {
        kind: ShareKind::Envelope,
        epoch,
        share_blob: aead_seal(&c.storage_key, &aad, &share_to_bytes(&c.phone_share)),
        storage_blob: hybrid_wrap(to, &c.storage_key),
    }
}

struct Case {
    name: &'static str,
    expect: &'static str,
    wire: ChallengeWire,
}

fn cases(c: &Cast) -> Vec<Case> {
    let phone = c.phone.public();
    let rid = c.requester.public().device_id();
    let sid = c.stranger.public().device_id();
    let oid = c.someone_else.public().device_id();
    let wire = |op: Vec<u8>,
                signer: &DeviceIdentity,
                request_id: u64,
                expiry: i64,
                material: PhoneMaterial| {
        let sig = signer.sign(&op);
        ChallengeWire {
            op_canonical: op,
            op_signature: sig,
            nonce: [0xA5; 32],
            expiry,
            eph_x25519_pub: c.eph_pub,
            request_id,
            phone_material: material,
        }
    };
    let good_op = |rid_: u64| {
        share_release_op(
            &rid,
            5,
            9,
            &[1; 32],
            NOW - 20,
            &HOST,
            rid_,
            &rid,
            &c.eph_pub,
        )
    };
    let mut v = vec![
        Case {
            name: "good",
            expect: "ok",
            wire: wire(
                good_op(4242),
                &c.requester,
                4242,
                NOW + 120,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "envelope",
            expect: "ok",
            wire: wire(
                good_op(4243),
                &c.requester,
                4243,
                NOW + 120,
                envelope_material(c, &phone, &HOST, 3),
            ),
        },
        Case {
            name: "expired",
            expect: "expired",
            wire: wire(
                good_op(4242),
                &c.requester,
                4242,
                NOW - 5,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "far-expiry",
            expect: "more than",
            wire: wire(
                good_op(4242),
                &c.requester,
                4242,
                NOW + 3600,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "wrong-host",
            expect: "different credential",
            wire: wire(
                good_op(4242),
                &c.requester,
                4242,
                NOW + 120,
                hybrid_material(c, &phone, &OTHER_HOST),
            ),
        },
        Case {
            name: "wrong-host-envelope",
            expect: "does not belong",
            wire: wire(
                good_op(4243),
                &c.requester,
                4243,
                NOW + 120,
                envelope_material(c, &phone, &OTHER_HOST, 3),
            ),
        },
        Case {
            name: "request-id-mismatch",
            expect: "different request id",
            wire: wire(
                good_op(4242),
                &c.requester,
                4244,
                NOW + 120,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "requester-mismatch",
            expect: "names",
            wire: wire(
                share_release_op(
                    &rid,
                    5,
                    9,
                    &[1; 32],
                    NOW - 20,
                    &HOST,
                    4242,
                    &oid,
                    &c.eph_pub,
                ),
                &c.requester,
                4242,
                NOW + 120,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "not-a-release",
            expect: "not a share release",
            wire: wire(
                head_attest_op(&rid),
                &c.requester,
                4242,
                NOW + 120,
                hybrid_material(c, &phone, &HOST),
            ),
        },
        Case {
            name: "wrong-phone",
            expect: "cannot release",
            wire: wire(
                good_op(4242),
                &c.requester,
                4242,
                NOW + 120,
                hybrid_material(c, &c.someone_else.public(), &HOST),
            ),
        },
        Case {
            name: "unknown-requester",
            expect: "ok",
            wire: wire(
                share_release_op(
                    &sid,
                    2,
                    3,
                    &[2; 32],
                    NOW - 20,
                    &HOST,
                    4242,
                    &sid,
                    &c.eph_pub,
                ),
                &c.stranger,
                4242,
                NOW + 120,
                hybrid_material(c, &phone, &HOST),
            ),
        },
    ];
    // Signature checks apply once the phone knows the requester (enrollment).
    let mut bad_sig = wire(
        good_op(4242),
        &c.requester,
        4242,
        NOW + 120,
        hybrid_material(c, &phone, &HOST),
    );
    bad_sig.op_signature[10] ^= 1;
    v.push(Case {
        name: "bad-signature",
        expect: "does not verify",
        wire: bad_sig,
    });
    let mut tampered = wire(
        good_op(4242),
        &c.requester,
        4242,
        NOW + 120,
        hybrid_material(c, &phone, &HOST),
    );
    // The terminal's op is altered after it was signed: the host digit changes, the signature stays.
    let at = tampered
        .op_canonical
        .windows(32)
        .position(|w| w == hx(&HOST).as_bytes())
        .unwrap();
    tampered.op_canonical[at + 31] = b'1';
    v.push(Case {
        name: "tampered-op",
        expect: "does not verify",
        wire: tampered,
    });
    v
}

fn challenge_json(c: &Cast, case: &Case) -> Value {
    let w = &case.wire;
    let frames = w.to_qr_frames(FRAME_BYTES).unwrap();
    let pre = binding_preimage(
        &w.op_canonical,
        &w.nonce,
        w.expiry,
        &w.eph_x25519_pub,
        w.request_id,
        &w.phone_material.hash(),
    );
    json!({
        "name": case.name, "expect": case.expect,
        "frame_hex": hx(&w.encode()), "qr_frames_hex": frames.iter().map(|f| hx(f)).collect::<Vec<_>>(),
        "expiry": w.expiry, "request_id": w.request_id,
        "binding_preimage_hex": hx(&pre), "binding_hash_hex": hx(&w.binding_hash()),
        "binding_words": sas_words(&w.binding_hash()),
        "op_canonical": String::from_utf8_lossy(&w.op_canonical),
        "eph_secret_hex": hx(&c.eph_secret),
    })
}

fn fixtures() -> (Value, Value) {
    let c = cast();
    let credential = dek_seal(&c.dek, b"correct horse battery staple");
    let challenges: Vec<Value> = cases(&c).iter().map(|k| challenge_json(&c, k)).collect();
    let reenroll: Value = {
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir().join("ops.vectors.json")).unwrap())
                .unwrap();
        let rec = v["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "phone-reenroll-email-code")
            .unwrap()
            .clone();
        let op = hex::decode(rec["canonical_hex"].as_str().unwrap()).unwrap();
        let sig: [u8; 64] = hex::decode(rec["signature_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let notice = RecoveryNoticeWire {
            op_canonical: op.clone(),
            op_signature: sig,
        };
        assert_eq!(
            RecoveryNoticeWire::decode(&notice.encode()).unwrap(),
            notice
        );
        let frames = fountain_encode(&notice.encode(), FRAME_BYTES, QR_PARITY_PERCENT).unwrap();
        json!({ "frame_hex": hx(&notice.encode()), "qr_frames_hex": frames.iter().map(|f| hx(f)).collect::<Vec<_>>(),
                "op_hash_hex": hx(&chain_hash(&op)), "op_lamport": 3 })
    };
    let main = json!({
        "meta": { "note": "synthetic keys, not real", "regenerate": "SSHUB_EMIT_PHONE_FIXTURES=1 cargo test sync::phone_fixtures" },
        "now": NOW,
        "phone": { "secret_hex": hx(&[0x42u8; 32]), "context": PHONE_CONTEXT, "words": bip39_encode(&[0x42; 32]), "pub": pub_json(&c.phone.public()) },
        "requester": { "ikm_hex": hx(&[0x77u8; 32]), "context": "sshub/test/requester", "name": "work laptop", "pub": pub_json(&c.requester.public()) },
        "stranger": { "ikm_hex": hx(&[0x78u8; 32]), "context": "sshub/test/stranger", "pub": pub_json(&c.stranger.public()) },
        "host_uid": hx(&HOST), "other_host_uid": hx(&OTHER_HOST),
        "phone_share": { "index": c.phone_share.index, "bytes_hex": hx(&c.phone_share.bytes) },
        "requester_share": { "index": c.requester_share.index, "bytes_hex": hx(&c.requester_share.bytes) },
        "credential_ciphertext_hex": hx(&credential),
        "eph": { "secret_hex": hx(&c.eph_secret), "pub_hex": hx(&c.eph_pub) },
        "challenges": challenges,
        "recovery_notice": reenroll,
    });
    (main, enroll_vector(&c))
}

// ---------------------------------------------------------------- enrollment (uses enroll.rs)

fn reveal(id: DevicePublic, eph: u8, nonce: u8, name: &str, attended: bool) -> Reveal {
    Reveal {
        ed25519_pub: id.ed25519_pub,
        x25519_pub: id.x25519_pub,
        mlkem_pub: id.mlkem_pub,
        eph: [eph; 32],
        nonce: [nonce; 32],
        name: name.into(),
        attended,
    }
}

fn enroll_vector(c: &Cast) -> Value {
    let joining = reveal(c.phone.public(), 0x21, 0x22, "my phone", true);
    let existing = reveal(c.requester.public(), 0x23, 0x24, "work laptop", true);
    let th = transcript_hash(&joining, &existing);
    let sig = c.requester.sign(&confirm_message(RESPONDER, true, &th));
    let frames = [
        EnrollFrame::Commit {
            role: RESPONDER,
            commit: commit_of(RESPONDER, &existing),
        },
        EnrollFrame::Commit {
            role: 0,
            commit: commit_of(INITIATOR, &joining),
        },
        EnrollFrame::Reveal {
            role: 1,
            reveal: existing.bytes(),
        },
        EnrollFrame::Reveal {
            role: 0,
            reveal: joining.bytes(),
        },
        EnrollFrame::Confirm {
            role: 1,
            ok: true,
            sig,
        },
    ];
    for f in &frames {
        assert_eq!(&EnrollFrame::decode(&f.encode()).unwrap(), f);
    }
    let qr = |f: &EnrollFrame| -> Vec<String> {
        fountain_encode(&f.encode(), FRAME_BYTES, QR_PARITY_PERCENT)
            .unwrap()
            .iter()
            .map(|x| hx(x))
            .collect()
    };
    json!({
        "joining": { "reveal_hex": hx(&joining.bytes()), "commit_hex": hx(&commit_of(INITIATOR, &joining)) },
        "existing": { "reveal_hex": hx(&existing.bytes()), "commit_hex": hx(&commit_of(RESPONDER, &existing)) },
        "transcript_hash_hex": hx(&th), "sas": sas_words(&th),
        "confirm_existing_ok_sig_hex": hx(&sig),
        "confirm_message_existing_ok_hex": hx(&confirm_message(RESPONDER, true, &th)),
        "frames_hex": frames.iter().map(|f| hx(&f.encode())).collect::<Vec<_>>(),
        // What the terminal shows before it has seen the phone's reveal (both are static).
        "existing_commit_qr_frames_hex": qr(&frames[0]),
        "existing_reveal_qr_frames_hex": qr(&frames[2]),
    })
}

// ---------------------------------------------------------------- tests

fn read(name: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(dir().join(name))
            .unwrap_or_else(|_| panic!("{name} is committed")),
    )
    .unwrap()
}

fn unhex(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

#[test]
fn committed_fixtures_are_consistent_with_the_rust_code() {
    if std::env::var_os("SSHUB_EMIT_PHONE_FIXTURES").is_some() {
        let (main, enroll) = fixtures();
        std::fs::write(
            dir().join("challenges.json"),
            serde_json::to_string_pretty(&main).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(
            dir().join("enroll.json"),
            serde_json::to_string_pretty(&enroll).unwrap() + "\n",
        )
        .unwrap();
        // The reshare fixture holds random polynomials: only rewritten when asked for by name.
        if std::env::var_os("SSHUB_EMIT_RESHARE_FIXTURE").is_some() {
            std::fs::write(
                dir().join("reshare.json"),
                serde_json::to_string_pretty(&reshare_fixture::emit()).unwrap() + "\n",
            )
            .unwrap();
        }
        return;
    }
    let main = read("challenges.json");
    let c = cast();
    assert_eq!(
        main["phone"]["pub"],
        pub_json(&c.phone.public()),
        "the page derives this identity from the same words"
    );
    assert_eq!(main["phone"]["words"], bip39_encode(&[0x42; 32]));
    for ch in main["challenges"].as_array().unwrap() {
        let name = ch["name"].as_str().unwrap();
        let w = ChallengeWire::decode(&unhex(&ch["frame_hex"])).unwrap();
        assert_eq!(
            hx(&w.binding_hash()),
            ch["binding_hash_hex"].as_str().unwrap(),
            "{name}"
        );
        // The animated frames reassemble to the same bytes.
        let mut dec = super::fountain::FountainDecoder::new();
        for f in ch["qr_frames_hex"].as_array().unwrap() {
            dec.push(&unhex(f)).unwrap();
        }
        assert_eq!(
            dec.finish().unwrap().unwrap(),
            unhex(&ch["frame_hex"]),
            "{name}"
        );
    }
    let enroll = read("enroll.json");
    let fresh = enroll_vector(&c);
    assert_eq!(enroll["sas"], fresh["sas"]);
    assert_eq!(enroll["transcript_hash_hex"], fresh["transcript_hash_hex"]);
    assert!(verify(
        &c.requester.public().ed25519_pub,
        &unhex(&enroll["confirm_message_existing_ok_hex"]),
        &unhex(&enroll["confirm_existing_ok_sig_hex"])
            .try_into()
            .unwrap()
    ));
}

/// The part of the phone that must open on the terminal: material the terminal built unwraps
/// with the phone's identity, and the plain-share path of the Rust helpers gives the DEK.
#[test]
fn phone_material_unwraps_for_the_phone_and_combines_to_the_dek() {
    let c = cast();
    let m = hybrid_material(&c, &c.phone.public(), &HOST);
    let plain = hybrid_unwrap(&c.phone, &m.share_blob).unwrap();
    let share = super::share_release::parse_wrapped_share_plaintext(&HOST, &plain).unwrap();
    let ct = dek_seal(&c.dek, b"x");
    assert_eq!(
        combine_and_check(&c.requester_share, &share, &ct).unwrap(),
        c.dek
    );
}

/// Runs only when the e2e driver left the page's replies in `SSHUB_PHONE_E2E_DIR`.
#[test]
fn e2e_response_opens_with_the_ephemeral_secret() {
    let Some(d) = std::env::var_os("SSHUB_PHONE_E2E_DIR") else {
        return;
    };
    let d = PathBuf::from(d);
    let main = read("challenges.json");
    let c = cast();
    let good = main["challenges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "good")
        .unwrap();
    // The page used the committed shares; `cast()` splits with fresh coefficients every run, so
    // the expectation comes from the fixture, never from a new split.
    let fixture_share = |v: &Value| Share {
        index: v["index"].as_u64().unwrap() as u8,
        bytes: unhex(&v["bytes_hex"]),
    };
    let want_share = fixture_share(&main["phone_share"]);
    let requester_share = fixture_share(&main["requester_share"]);
    let want = &want_share;
    let ct = std::fs::read(d.join("response-full.bin")).unwrap();
    let resp = ShareResponseWire::decode(&ct).expect("the page emitted an exact ShareResponseWire");
    assert_eq!(resp.request_id, 4242);
    let (share, ext) = unwrap_share_reply(&c.eph_secret, &resp.ciphertext).unwrap();
    assert_eq!(&share, want, "the released share is the phone's share");
    let released = share;
    assert_eq!(
        combine_and_check(
            &requester_share,
            &released,
            &unhex(&main["credential_ciphertext_hex"])
        )
        .unwrap(),
        c.dek
    );
    let ext = ext.expect("the QR and clipboard reply carries the courier");
    let phone_pub = c.phone.public().ed25519_pub;
    assert!(
        verify(
            &phone_pub,
            &unhex(&good["binding_preimage_hex"]),
            &ext.approval_sig
        ),
        "approval signature over the binding preimage"
    );
    assert!(!ext.ops.is_empty());
    for (op, sig) in &ext.ops {
        assert!(
            verify(&phone_pub, op, sig),
            "courier op signed by the phone"
        );
        assert!(String::from_utf8_lossy(op).contains(&format!(
            "\"device_id\":\"{}\"",
            c.phone.public().device_id()
        )));
    }
    // The audio reply is the short one: same share, no courier.
    let short =
        ShareResponseWire::decode(&std::fs::read(d.join("response-short.bin")).unwrap()).unwrap();
    let (s2, e2) = unwrap_share_reply(&c.eph_secret, &short.ciphertext).unwrap();
    assert_eq!(&s2, want);
    assert!(e2.is_none());
    // The snapshot blob inside the reply opens with the code the page showed.
    if let Some((host, blob)) = ext.snapshot {
        assert_eq!(host, HOST);
        let code = std::fs::read_to_string(d.join("snapshot-code.txt")).unwrap();
        let parsed = SnapshotBlob::from_bytes(&blob).unwrap();
        assert_eq!(
            parsed.host_uid,
            Some(host),
            "the page issues version 2, bound to the credential"
        );
        let opened = snapshot_open_for(&parsed, &host, code.trim(), NOW + 5).unwrap();
        assert_eq!(&opened, want);
    }
    if let Ok(frame) = std::fs::read(d.join("cancel-frame.bin")) {
        let ops = PhoneOpsWire::decode(&frame).unwrap();
        let last = ops.ops.last().unwrap();
        assert!(verify(&phone_pub, &last.0, &last.1));
        let text = String::from_utf8_lossy(&last.0);
        let notice = &main["recovery_notice"];
        assert!(
            text.contains("PhoneReenrollCancel")
                && text.contains(notice["op_hash_hex"].as_str().unwrap())
        );
    }
}

// ---------------------------------------------------------------- resharing with the phone as a new holder

mod reshare_fixture {
    use super::*;
    use crate::sync::cred_ops::{cred_put, CredPayload};
    use crate::sync::oplog::{CredMode, DeviceKind, Op, OpBody, VerifiedBy};
    use crate::sync::protocol::tests::Dev;
    use crate::sync::quorum::own_shares;
    use crate::sync::reshare::{next_epoch, reissue_plan, reissue_receive};
    use std::sync::Arc;

    fn phone_identity() -> Arc<DeviceIdentity> {
        Arc::new(DeviceIdentity::from_seed(&[0x42; 32], PHONE_CONTEXT))
    }

    fn pub_of(v: &Value) -> DevicePublic {
        DevicePublic {
            ed25519_pub: unhex(&v["ed25519_pub"]).try_into().unwrap(),
            x25519_pub: unhex(&v["x25519_pub"]).try_into().unwrap(),
            mlkem_pub: unhex(&v["mlkem_pub"]),
        }
    }

    fn op_json(op: &Op) -> Value {
        json!({
            "kind": super::super::oplog::canonical::body_name(&op.body),
            "device_id": op.device_id, "seq": op.seq, "lamport": op.lamport,
            "canonical_hex": hx(&op.canonical_bytes()), "sig_hex": hx(&op.signature),
        })
    }

    fn op_of(v: &Value) -> Op {
        Op::from_canonical(
            &unhex(&v["canonical_hex"]),
            unhex(&v["sig_hex"]).try_into().unwrap(),
        )
        .unwrap()
    }

    /// A store that knows `contributors` as terminal holders and the phone as a phone holder.
    fn phone_store(contributors: &[DevicePublic]) -> Dev {
        let phone = Dev::with(phone_identity());
        for p in contributors {
            phone
                .store
                .sync_register_device(
                    p,
                    "terminal",
                    super::super::oplog::DeviceRole::Holder,
                    super::super::oplog::Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
        phone
            .store
            .sync_register_device(
                &phone.id.public(),
                "my phone",
                super::super::oplog::DeviceRole::Holder,
                super::super::oplog::Attendance::Attended,
                DeviceKind::Phone,
                VerifiedBy::Sas,
            )
            .unwrap();
        phone
    }

    /// Builds the fixture: terminals A, B, C share two credentials; the phone becomes a holder and A, B
    /// (the two contributors) reshare to {A, B, phone}.
    pub fn emit() -> Value {
        let (a, b, cc) = (Dev::new(), Dev::new(), Dev::new());
        let phone = Dev::with(phone_identity());
        let all = [&a, &b, &cc];
        for x in all {
            for y in all {
                if x.did != y.did {
                    x.trust(y);
                }
            }
            x.store
                .sync_register_device(
                    &x.id.public(),
                    "self",
                    super::super::oplog::DeviceRole::Holder,
                    super::super::oplog::Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
        let hosts = [HOST, OTHER_HOST];
        a.store
            .sync_append_local(
                &a.id,
                OpBody::ModeChange {
                    to: CredMode::Quorum,
                    migrated_on: a.did.clone(),
                    cosigner: None,
                },
            )
            .unwrap();
        for h in &hosts {
            cred_put(
                &a.store,
                &a.id,
                h,
                &CredPayload {
                    password: Some("pw".into()),
                    private_key: None,
                },
            )
            .unwrap();
        }
        // B and C learn A's chain (credentials arrive as pending Yellow and are approved).
        for x in [&b, &cc] {
            for op in a.store.sync_chain_tail(&a.did, 0).unwrap() {
                x.store.sync_ingest(op).unwrap();
            }
            for op in x.store.sync_pending().unwrap() {
                x.store.sync_resolve_pending(&op.hash(), true).unwrap();
            }
        }
        for x in [&a, &b] {
            x.store
                .sync_register_device(
                    &phone.id.public(),
                    "my phone",
                    super::super::oplog::DeviceRole::Holder,
                    super::super::oplog::Attendance::Attended,
                    DeviceKind::Phone,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
        let holders = vec![a.did.clone(), b.did.clone(), phone.did.clone()];
        let epoch = next_epoch(&a.store).unwrap();
        for x in [&a, &b] {
            for body in reissue_plan(&x.store, &x.id, &holders, epoch)
                .unwrap()
                .bodies
            {
                x.store.sync_append_local(&x.id, body).unwrap();
            }
        }
        // The whole chains of the two contributors (the page needs only the sessions and batches; the
        // Rust receiver ingests them like a terminal would, so the chains must be contiguous).
        let mut ops: Vec<Value> = Vec::new();
        for x in [&a, &b] {
            ops.extend(
                x.store
                    .sync_chain_tail(&x.did, 0)
                    .unwrap()
                    .iter()
                    .map(op_json),
            );
        }
        // What the phone must end up with, computed by the Rust receiver.
        let contributors = [a.id.public(), b.id.public()];
        let rx = phone_store(&contributors);
        for o in &ops {
            rx.store.sync_ingest(op_of(o)).unwrap();
        }
        // The credentials arrive as pending reviews; the receiver knows the old sharing once they applied.
        for op in rx.store.sync_pending().unwrap() {
            rx.store.sync_resolve_pending(&op.hash(), true).unwrap();
        }
        for body in reissue_receive(&rx.store, &rx.id, epoch).unwrap().bodies {
            rx.store.sync_append_local(&rx.id, body).unwrap();
        }
        let mut shares = serde_json::Map::new();
        for h in &hosts {
            let (e, s) = own_shares(&rx.store, &rx.id, h)
                .unwrap_or_else(|e| panic!("phone shares: {e:#}"))
                .remove(0);
            assert_eq!(e, epoch);
            shares.insert(
                hx(h),
                json!({ "epoch": e, "index": s.index, "bytes_hex": hx(&s.bytes) }),
            );
        }
        let mut sorted_holders = holders.clone();
        sorted_holders.sort();
        json!({
            "meta": { "note": "synthetic keys, random polynomials: regenerate only on purpose", "regenerate": "SSHUB_EMIT_PHONE_FIXTURES=1 cargo test sync::phone_fixtures" },
            "epoch": epoch,
            "host_uids": hosts.iter().map(|h| hx(h)).collect::<Vec<_>>(),
            "new_holders": { "sorted_ids": sorted_holders, "phone": phone.did },
            "contributors": [pub_json(&a.id.public()), pub_json(&b.id.public())],
            "phone_pub": pub_json(&phone.id.public()),
            "ops": ops,
            "expected_phone_shares": Value::Object(shares),
        })
    }

    fn receive_from_fixture(v: &Value) -> Dev {
        let contributors: Vec<DevicePublic> = v["contributors"]
            .as_array()
            .unwrap()
            .iter()
            .map(pub_of)
            .collect();
        let rx = phone_store(&contributors);
        for o in v["ops"].as_array().unwrap() {
            rx.store.sync_ingest(op_of(o)).unwrap();
        }
        for op in rx.store.sync_pending().unwrap() {
            rx.store.sync_resolve_pending(&op.hash(), true).unwrap();
        }
        rx
    }

    /// The Rust receiver, run on the committed ops, still produces the committed shares.
    pub fn check_committed() {
        let v = read("reshare.json");
        let epoch = v["epoch"].as_u64().unwrap();
        let rx = receive_from_fixture(&v);
        for body in reissue_receive(&rx.store, &rx.id, epoch).unwrap().bodies {
            rx.store.sync_append_local(&rx.id, body).unwrap();
        }
        for (host_hex, want) in v["expected_phone_shares"].as_object().unwrap() {
            let host: [u8; 16] = hex::decode(host_hex).unwrap().try_into().unwrap();
            let (e, s) = own_shares(&rx.store, &rx.id, &host).unwrap().remove(0);
            assert_eq!((e, s.index), (epoch, want["index"].as_u64().unwrap() as u8));
            assert_eq!(
                hx(&s.bytes),
                want["bytes_hex"].as_str().unwrap(),
                "the new share of {host_hex}"
            );
        }
    }

    /// What the page produced for the same ops (`reshare-out.json`: its `ShareStorageKey` and
    /// `ShareReshareConfirmBatch` ops, canonical and signature in hex): the terminal ingests them and the
    /// phone's envelope opens to the committed share.
    pub fn check_page_output(out: &Value) {
        let v = read("reshare.json");
        let epoch = v["epoch"].as_u64().unwrap();
        let rx = receive_from_fixture(&v);
        for o in out["ops"].as_array().unwrap() {
            let op = op_of(o);
            assert_eq!(op.device_id, rx.did, "the page signs as the phone");
            assert!(
                op.verify(&rx.id.public().ed25519_pub),
                "the page's op signature verifies"
            );
            rx.store.sync_ingest(op).unwrap();
        }
        // The page's clock is the fixtures' fixed time (2030), beyond the skew grace of this terminal's clock, so
        // the ops wait as "future-dated" until a human approves them; the test plays that human.
        for p in rx.store.sync_pending().unwrap() {
            rx.store.sync_resolve_pending(&p.hash(), true).unwrap();
        }
        for (host_hex, want) in v["expected_phone_shares"].as_object().unwrap() {
            let host: [u8; 16] = hex::decode(host_hex).unwrap().try_into().unwrap();
            let (e, s) = own_shares(&rx.store, &rx.id, &host)
                .expect("the page's envelope opens")
                .remove(0);
            assert_eq!(e, epoch, "the page confirmed the new sharing");
            assert_eq!(
                hx(&s.bytes),
                want["bytes_hex"].as_str().unwrap(),
                "the page combined to the same share as Rust"
            );
        }
    }

    /// The trust-session chain the page wrote (`cert-out.json`: CertIssue, two CertExtend, CertRevoke, a second
    /// CertIssue, all signed by the 0x42 phone) on a terminal that enforces certificates: the certificate
    /// validates, the budget refuses a new host, the extension opens it for its own rolling day only, and the
    /// revocation stops the certificate.
    pub fn check_cert_output(out: &Value) {
        use crate::sync::cert::{
            cert_allows, cert_opened_body, extension_budget, validate_cert_issue, DelegationCert,
        };
        use crate::sync::oplog::{Attendance, CertScope, DeviceRole};
        // The delegate is the device of seed 0x71, the same one the page's test derives from the same bytes.
        let delegate = Dev::with(Arc::new(DeviceIdentity::from_seed(
            &[0x71; 32],
            PHONE_CONTEXT,
        )));
        let phone = Dev::with(phone_identity());
        let (t2, t3) = (Dev::new(), Dev::new());
        for d in [&t2, &delegate, &t3] {
            t2.store
                .sync_register_device(
                    &d.id.public(),
                    "terminal",
                    DeviceRole::Holder,
                    Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
        t2.store
            .sync_register_device(
                &phone.id.public(),
                "my phone",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Phone,
                VerifiedBy::Sas,
            )
            .unwrap();
        let ingest = |op: &Op| {
            assert_eq!(op.device_id, phone.did, "the page signs as the phone");
            assert!(
                op.verify(&phone.id.public().ed25519_pub),
                "the page's op signature verifies"
            );
            t2.store.sync_ingest(op.clone()).unwrap();
            for p in t2.store.sync_pending().unwrap() {
                if !crate::sync::ceremony::is_gated(&t2.store, &p).unwrap() {
                    t2.store.sync_resolve_pending(&p.hash(), true).unwrap();
                }
            }
        };
        let ops: Vec<Op> = out["ops"].as_array().unwrap().iter().map(op_of).collect();
        assert_eq!(ops.len(), 5, "issue, extend, extend, revoke, issue");
        let mut rest = ops.iter();

        let issue = rest.next().unwrap();
        assert!(matches!(issue.body, OpBody::CertIssue { .. }));
        ingest(issue);
        validate_cert_issue(&t2.store, issue, &phone.did)
            .expect("the certificate the page wrote validates");
        let cert = DelegationCert::from_op(issue).unwrap();
        assert_eq!(cert.device_id, delegate.did);
        assert_eq!(cert.issuer, phone.did);
        assert_eq!(cert.max_new_hosts_per_day, 2);
        let CertScope::Hosts(hosts) = cert.scope.clone() else {
            panic!("the first certificate lists hosts");
        };
        assert_eq!(hosts.len(), 3);
        let pk = phone.id.public().ed25519_pub;
        // The enforcing holder opens two new hosts: the budget of 2 is used.
        for h in &hosts[..2] {
            t2.store
                .sync_append_local(&t2.id, cert_opened_body(&cert, h))
                .unwrap();
        }
        let ext1 = rest.next().unwrap();
        let OpBody::CertExtend {
            cert_id,
            extra_hosts: e1,
        } = &ext1.body
        else {
            panic!("a CertExtend")
        };
        assert_eq!(*cert_id, cert.id, "the extension names the certificate");
        let t = ext1.timestamp + 10;
        let refused = cert_allows(&cert, &pk, &t2.store, &delegate.did, &hosts[2], t, t)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("budget"), "{refused}");
        ingest(ext1);
        assert_eq!(extension_budget(&t2.store, &cert, t).unwrap(), *e1);
        cert_allows(&cert, &pk, &t2.store, &delegate.did, &hosts[2], t, t)
            .expect("the phone's extension opens the budget");

        let ext2 = rest.next().unwrap();
        let OpBody::CertExtend {
            extra_hosts: e2, ..
        } = &ext2.body
        else {
            panic!("a second CertExtend")
        };
        assert!(ext2.timestamp - ext1.timestamp >= 86_400, "a day apart");
        ingest(ext2);
        assert_eq!(
            extension_budget(&t2.store, &cert, ext2.timestamp + 10).unwrap(),
            *e2
        );
        assert_eq!(
            extension_budget(&t2.store, &cert, t).unwrap(),
            *e1,
            "each extension covers its own day"
        );

        let revoke = rest.next().unwrap();
        let OpBody::CertRevoke { cert_id } = &revoke.body else {
            panic!("a CertRevoke")
        };
        assert_eq!(*cert_id, cert.id);
        ingest(revoke);
        let err = cert_allows(&cert, &pk, &t2.store, &delegate.did, &hosts[0], t, t)
            .unwrap_err()
            .to_string();
        assert!(err.contains("revoked"), "{err}");

        // The second certificate (a group) validates too.
        let second = rest.next().unwrap();
        ingest(second);
        validate_cert_issue(&t2.store, second, &phone.did)
            .expect("the group certificate validates");
        let OpBody::CertIssue { scope, .. } = &second.body else {
            panic!("a CertIssue")
        };
        assert!(matches!(scope, CertScope::Group(g) if *g == [0x06; 16]));
    }
}

#[test]
fn the_rust_receiver_reproduces_the_committed_reshare_shares() {
    if std::env::var_os("SSHUB_EMIT_PHONE_FIXTURES").is_some()
        || !dir().join("reshare.json").exists()
    {
        return;
    }
    reshare_fixture::check_committed();
}

#[test]
fn e2e_the_pages_reshare_ops_open_on_the_terminal() {
    let Some(d) = std::env::var_os("SSHUB_PHONE_E2E_DIR") else {
        return;
    };
    let out = serde_json::from_str::<Value>(
        &std::fs::read_to_string(PathBuf::from(d).join("reshare-out.json")).unwrap(),
    )
    .unwrap();
    reshare_fixture::check_page_output(&out);
}

#[test]
fn e2e_the_pages_trust_session_ops_validate_on_the_terminal() {
    let Some(d) = std::env::var_os("SSHUB_PHONE_E2E_DIR") else {
        return;
    };
    let out = serde_json::from_str::<Value>(
        &std::fs::read_to_string(PathBuf::from(d).join("cert-out.json")).unwrap(),
    )
    .unwrap();
    reshare_fixture::check_cert_output(&out);
}

/// The terminal half of enrolling the REAL page (`phone_enroll::PhoneEnroll`), run as a peer next to the headless
/// page flow of `assets/companion/scripts/e2e.mjs`. Both sides exchange the enrollment frames as files in
/// `SSHUB_PHONE_ENROLL_PEER/<scenario>/` (written by rename, so a reader never sees half a file):
/// `t-*` come from this terminal, `p-*` from the page. Scenarios, in this order: `ok` (same four words, the phone is
/// Verified and admitted with the page's identity), `tampered` (the page's reveal altered on the way: Stopped),
/// `differ` (the page declines: Stopped). Each scenario ends with `done.json`. Not run without the variable.
mod enroll_peer {
    use super::*;
    use crate::sync::oplog::{DeviceKind, DeviceRole, OpBody};
    use crate::sync::phone_enroll::{admit_phone, PhoneEnroll, Step};
    use crate::sync::protocol::tests::Dev;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(240);

    fn put(dir: &Path, name: &str, bytes: &[u8]) {
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, bytes).unwrap();
        std::fs::rename(&tmp, dir.join(name)).unwrap();
    }

    fn wait(dir: &Path, name: &str) -> Vec<u8> {
        let t0 = Instant::now();
        loop {
            if let Ok(b) = std::fs::read(dir.join(name)) {
                return b;
            }
            assert!(
                t0.elapsed() < WAIT,
                "timed out waiting for {name} from the page"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The terminal shows its commitment; scans the page's commitment and shows its reveal; scans the page's reveal.
    fn through_reveal(dir: &Path, t: &Dev, e: &mut PhoneEnroll) -> Step {
        let p_commit = wait(dir, "p-commit.bin");
        let Step::Show(reveal) = e
            .scan(&t.id, &p_commit)
            .expect("the page's commitment frame decodes and is accepted")
        else {
            panic!("after the phone's commitment the terminal shows its reveal")
        };
        put(dir, "t-reveal.bin", &reveal);
        let p_reveal = wait(dir, "p-reveal.bin");
        e.scan(&t.id, &p_reveal)
            .expect("the page's reveal frame decodes")
    }

    fn start(dir: &Path) -> (Dev, PhoneEnroll) {
        let t = Dev::new();
        let (e, first) = PhoneEnroll::begin(&t.id, "work laptop");
        put(dir, "t-id.txt", t.did.as_bytes());
        put(dir, "t-commit.bin", &first);
        (t, e)
    }

    fn compare(dir: &Path, t: &Dev, e: &mut PhoneEnroll) -> String {
        let Step::Compare { words, phone_name } = through_reveal(dir, t, e) else {
            panic!("the words follow the phone's reveal")
        };
        assert_eq!(phone_name, "my phone");
        put(dir, "t-words.txt", words.as_bytes());
        let page_words = String::from_utf8(wait(dir, "p-words.txt")).unwrap();
        assert_eq!(page_words, words, "the page shows the same four words");
        words
    }

    fn ok(dir: &Path) {
        let (t, mut e) = start(dir);
        compare(dir, &t, &mut e);
        assert_eq!(e.answer(&t.id, true).unwrap(), Step::Wait);
        let p_confirm = wait(dir, "p-confirm.bin");
        let Step::Verified(v) = e
            .scan(&t.id, &p_confirm)
            .expect("the page's confirmation decodes")
        else {
            panic!("the page's confirmation verifies the phone")
        };
        put(dir, "t-confirm.bin", &v.frame);
        let phone = DeviceIdentity::from_seed(&[0x42; 32], PHONE_CONTEXT);
        assert_eq!(
            v.phone,
            phone.public(),
            "the admitted DevicePublic is the page's identity"
        );
        assert_eq!(v.name, "my phone");
        let op = admit_phone(&t.store, &t.id, &v).unwrap();
        assert!(matches!(
            op.body,
            OpBody::DeviceAdd {
                role: DeviceRole::Holder,
                kind: DeviceKind::Phone,
                ..
            }
        ));
        assert_eq!(
            crate::sync::quorum::phone_device(&t.store).unwrap(),
            Some(phone.public().device_id()),
            "the ring has the page's phone"
        );
    }

    fn tampered(dir: &Path) {
        let (t, mut e) = start(dir);
        match through_reveal(dir, &t, &mut e) {
            Step::Stopped { why, frame } => {
                assert!(why.contains("do not open the commitment"), "{why}");
                assert!(frame.is_none());
            }
            other => panic!("a tampered reveal must stop the terminal, got {other:?}"),
        }
        assert!(
            e.answer(&t.id, true).is_err(),
            "a stopped enrollment answers nothing"
        );
    }

    fn differ(dir: &Path) {
        let (t, mut e) = start(dir);
        compare(dir, &t, &mut e);
        let p_confirm = wait(dir, "p-confirm.bin");
        assert_eq!(e.scan(&t.id, &p_confirm).unwrap(), Step::Wait);
        let Step::Stopped { why, .. } = e.answer(&t.id, true).unwrap() else {
            panic!("the page declined: the terminal stops")
        };
        // This message is only reached after the decline's signature verified under the page's key.
        assert!(why.contains("the phone says the words differ"), "{why}");
        assert!(crate::sync::quorum::phone_device(&t.store)
            .unwrap()
            .is_none());
    }

    pub fn run(root: &Path) {
        let mut failed = Vec::new();
        type Scenario = (&'static str, fn(&Path));
        let scenarios: [Scenario; 3] = [("ok", ok), ("tampered", tampered), ("differ", differ)];
        for (name, f) in scenarios {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let r = std::panic::catch_unwind(|| f(&dir));
            let why = match &r {
                Ok(()) => String::new(),
                Err(e) => e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into()),
            };
            put(
                &dir,
                "done.json",
                json!({ "pass": r.is_ok(), "why": why })
                    .to_string()
                    .as_bytes(),
            );
            if r.is_err() {
                failed.push(format!("{name}: {why}"));
            }
        }
        assert!(failed.is_empty(), "{failed:?}");
    }
}

#[test]
fn e2e_enrollment_peer_for_the_page() {
    let Some(d) = std::env::var_os("SSHUB_PHONE_ENROLL_PEER") else {
        return;
    };
    enroll_peer::run(Path::new(&d));
}
