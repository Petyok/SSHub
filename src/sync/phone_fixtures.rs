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
use super::snapshot::{snapshot_open, SnapshotBlob};
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
) -> Vec<u8> {
    format!(
        r#"{{"device_id":"{device_id}","seq":{seq},"lamport":{lamport},"prev_hash":"{}","timestamp":{ts},"epoch":0,"alg":[1,1],"body":{{"ShareRelease":{{"host_uid":"{}","request_id":{request_id},"requester":"{requester}"}}}}}}"#,
        hx(prev),
        hx(host)
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
    let good_op = |rid_: u64| share_release_op(&rid, 5, 9, &[1; 32], NOW - 20, &HOST, rid_, &rid);
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
                share_release_op(&rid, 5, 9, &[1; 32], NOW - 20, &HOST, 4242, &oid),
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
                share_release_op(&sid, 2, 3, &[2; 32], NOW - 20, &HOST, 4242, &sid),
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
        let opened = snapshot_open(
            &SnapshotBlob::from_bytes(&blob).unwrap(),
            code.trim(),
            NOW + 5,
        )
        .unwrap();
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
