//! Ports of the release tests of the page: `phone.test.mjs` (the shared `challenges.json` cases, approve, short code) and
//! `review-app-round1.test.mjs` (S1, F2, F6 and the sound checks of the challenge). Randomness is a seeded ChaCha.

use super::*;
use crate::state::{PhoneState, RingDevice};
use crate::text::from_base64;
use crate::views::AudioProfile;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use serde_json::Value;
use sshub_sync_proto::crypto::{bip39_encode, hybrid_wrap, Share};
use sshub_sync_proto::oplog::canonical::decode_op;
use sshub_sync_proto::share_release::{unwrap_share_reply, wrapped_share_plaintext};
use sshub_sync_proto::snapshot::{snapshot_open_for, SnapshotBlob};

struct World {
    id: PhoneIdentity,
    state: PhoneState,
    rng: ChaCha8Rng,
}

impl World {
    fn new(id: PhoneIdentity) -> Self {
        Self {
            id,
            state: PhoneState::default(),
            rng: ChaCha8Rng::seed_from_u64(7),
        }
    }
    fn ctx(&mut self, now: i64) -> Ctx<'_> {
        Ctx {
            id: &mut self.id,
            state: &mut self.state,
            rng: &mut self.rng,
            now,
        }
    }
    fn enrol(&mut self, id: &str, name: &str, ed25519_pub: &[u8; 32]) {
        self.state.ring.insert(
            id.to_string(),
            RingDevice {
                name: name.to_string(),
                ed25519_pub: hex::encode(ed25519_pub),
                enrolled_at: 0,
            },
        );
    }
    fn open(
        &mut self,
        frame: &[u8],
        now: i64,
    ) -> std::result::Result<(ShareRequestView, PendingRelease), RefusalView> {
        open_challenge(frame, &mut self.ctx(now), 1)
    }
}

fn opts(typed: Option<&str>, code: bool) -> ReleaseOptions {
    ReleaseOptions {
        typed_id: typed.map(String::from),
        qr: false,
        clipboard: true,
        sound: true,
        audio_profile: AudioProfile::Fast,
        issue_short_code: code,
        by_hand: true,
    }
}

// ------------------------------------------------------------ the shared fixture

fn fx() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/companion/test/fixtures/challenges.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn hx(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn fx_world(f: &Value, with_requester: bool) -> World {
    let secret: [u8; 32] = hx(&f["phone"]["secret_hex"]).try_into().unwrap();
    let mut w = World::new(PhoneIdentity::new(&secret, 0).unwrap());
    if with_requester {
        let pubkey: [u8; 32] = hx(&f["requester"]["pub"]["ed25519_pub"])
            .try_into()
            .unwrap();
        w.enrol(
            f["requester"]["pub"]["device_id"].as_str().unwrap(),
            f["requester"]["name"].as_str().unwrap(),
            &pubkey,
        );
    }
    w
}

fn case<'a>(f: &'a Value, name: &str) -> &'a Value {
    f["challenges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()
}

#[test]
fn every_challenges_json_case_is_judged_like_the_page_and_the_rust_terminal() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let rid = f["requester"]["pub"]["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let share_hex = f["phone_share"]["bytes_hex"].as_str().unwrap().to_string();
    for c in f["challenges"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut w = fx_world(&f, name != "unknown-requester");
        let r = w.open(&hx(&c["frame_hex"]), now);
        if c["expect"] == "ok" {
            let (view, p) = r.unwrap_or_else(|e| panic!("{name}: {} {:?}", e.reason, e.checks));
            assert_eq!(
                view.binding_hash,
                c["binding_hash_hex"].as_str().unwrap(),
                "{name}: binding hash equals the Rust one"
            );
            assert_eq!(
                view.binding_words,
                c["binding_words"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(p.binding_preimage, hx(&c["binding_preimage_hex"]), "{name}");
            assert_eq!(hex::encode(&p.share[1..]), share_hex, "{name}");
            assert_eq!(
                u64::from(p.share[0]),
                f["phone_share"]["index"].as_u64().unwrap(),
                "{name}"
            );
            assert_eq!(
                view.checks.iter().filter(|c| c.warn).count(),
                usize::from(name == "unknown-requester"),
                "{name}"
            );
            assert_eq!(
                view.type_to_approve.is_some(),
                name == "unknown-requester",
                "{name}"
            );
            assert_eq!(view.expires_at, c["expiry"].as_i64().unwrap());
            assert_eq!(view.host_uid, f["host_uid"].as_str().unwrap());
            let who = if name == "unknown-requester" {
                f["stranger"]["pub"]["device_id"].as_str().unwrap()
            } else {
                rid.as_str()
            };
            assert!(
                view.op.headline.contains(who),
                "{name}: {}",
                view.op.headline
            );
            let req = view
                .op
                .rows
                .iter()
                .find(|r| r.label == "Requesting device")
                .unwrap();
            assert!(req.danger);
            assert!(
                name == "unknown-requester"
                    || req.value.contains(f["requester"]["name"].as_str().unwrap()),
                "the requesting device is named"
            );
            assert_eq!(
                view.op
                    .rows
                    .iter()
                    .find(|r| r.label == "Host credential")
                    .unwrap()
                    .value,
                f["host_uid"].as_str().unwrap()
            );
            assert_eq!(view.short_code, ShortCodeOffer::Available);
            // and the approval of every ok case (hybrid and envelope) opens with the fixture ephemeral secret
            let out = approve(
                p,
                &opts(view.type_to_approve.as_deref(), false),
                &mut w.ctx(now + 1),
            )
            .unwrap();
            let eph: [u8; 32] = hx(&c["eph_secret_hex"]).try_into().unwrap();
            let resp = ShareResponseWire::decode(&out.frame).unwrap();
            assert_eq!(resp.request_id, c["request_id"].as_u64().unwrap());
            let (share, ext) = unwrap_share_reply(&eph, &resp.ciphertext).unwrap();
            assert_eq!(hex::encode(&share.bytes), share_hex, "{name}");
            assert!(
                ext.unwrap()
                    .approved_by(&w.id.public().ed25519_pub, &hx(&c["binding_preimage_hex"])),
                "{name}"
            );
        } else {
            let e = r
                .err()
                .unwrap_or_else(|| panic!("{name}: should be refused"));
            let want = c["expect"].as_str().unwrap();
            assert!(
                e.reason.contains(want),
                "{name}: \"{}\" should mention \"{want}\"",
                e.reason
            );
        }
    }
}

#[test]
fn a_known_requester_with_a_bad_signature_is_refused_an_unknown_one_only_warns() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let bad = hx(&case(&f, "bad-signature")["frame_hex"]);
    assert!(fx_world(&f, true)
        .open(&bad, now)
        .err()
        .unwrap()
        .reason
        .contains("does not verify"));
    let (view, _) = fx_world(&f, false).open(&bad, now).ok().unwrap();
    assert_eq!(view.checks.iter().filter(|c| c.warn).count(), 1);
}

#[test]
fn approve_reply_opens_with_the_ephemeral_key_carries_the_courier_and_the_chain_continues() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let good = case(&f, "good");
    let frame = hx(&good["frame_hex"]);
    let eph: [u8; 32] = hx(&good["eph_secret_hex"]).try_into().unwrap();
    let mut w = fx_world(&f, true);
    let phone_pub = w.id.public().ed25519_pub;

    let (_, p) = w.open(&frame, now).ok().unwrap();
    let out = approve(p, &opts(None, false), &mut w.ctx(now + 3)).unwrap();
    assert_eq!(out.kind, 2);
    assert_eq!(&out.frame[..7], &[0x53, 0x57, 0x52, 0x31, 0, 1, 2]);
    assert_eq!(from_base64(&out.text).unwrap(), out.frame);
    let resp = ShareResponseWire::decode(&out.frame).unwrap();
    assert_eq!(resp.request_id, 4242);
    let (share, ext) = unwrap_share_reply(&eph, &resp.ciphertext).unwrap();
    assert_eq!(
        hex::encode(&share.bytes),
        f["phone_share"]["bytes_hex"].as_str().unwrap()
    );
    let ext = ext.expect("the full reply carries the extension");
    assert!(
        ext.approved_by(&phone_pub, &hx(&good["binding_preimage_hex"])),
        "approval signature over the binding preimage"
    );
    assert!(ext.snapshot.is_none());
    assert_eq!(ext.ops.len(), 1);
    let op = decode_op(&ext.ops[0].0, ext.ops[0].1).unwrap();
    assert!(op.verify(&phone_pub));
    assert_eq!(op.device_id, w.id.device_id());
    assert!(matches!(op.body, OpBody::HeadAttest { seq: 0, .. }));
    // the audio reply is share-only
    let short = ShareResponseWire::decode(&out.audio_frame.clone().unwrap()).unwrap();
    let (s2, e2) = unwrap_share_reply(&eph, &short.ciphertext).unwrap();
    assert_eq!(s2.bytes, share.bytes);
    assert!(e2.is_none());
    // the manual entry is the index and the 24 words of the 32 share bytes
    let m = out.manual.clone().unwrap();
    assert_eq!(
        u64::from(m.index),
        f["phone_share"]["index"].as_u64().unwrap()
    );
    assert_eq!(
        m.words,
        bip39_encode(<&[u8; 32]>::try_from(&share.bytes[..]).unwrap())
    );
    assert_eq!(m.words.split(' ').count(), 24);
    assert!(out.short_code.is_none());
    assert!(out.qr.is_empty());

    // second approval: seq 2 continues from the first op
    let (_, p) = w.open(&frame, now + 5).ok().unwrap();
    let out2 = approve(
        p,
        &ReleaseOptions {
            qr: true,
            sound: false,
            ..opts(None, false)
        },
        &mut w.ctx(now + 6),
    )
    .unwrap();
    assert!(!out2.qr.is_empty(), "the qr option is honoured");
    assert!(out2.audio_frame.is_none(), "the sound option is honoured");
    let seqs: Vec<u64> = w.state.chain.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2]);
    let second = decode_op(&w.state.chain[1].canonical, w.state.chain[1].signature64()).unwrap();
    assert_eq!(second.prev_hash, w.state.chain[0].hash32());
    assert!(second.lamport > w.state.chain[0].lamport);
    // the verified requester op (lamport 9) moved the counters: 9 + 1
    assert_eq!(w.state.chain[0].lamport, 10);
}

#[test]
fn the_decision_checks_expiry_and_the_typed_id_again() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let expiry = case(&f, "unknown-requester")["expiry"].as_i64().unwrap();
    let frame = hx(&case(&f, "unknown-requester")["frame_hex"]);
    let rid = f["stranger"]["pub"]["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut w = fx_world(&f, false);
    let (view, _) = w.open(&frame, now).ok().unwrap();
    assert_eq!(
        view.type_to_approve.as_deref(),
        Some(rid.as_str()),
        "S1: an unknown requester needs the typed id"
    );
    for typed in [None, Some(""), Some("0000000000000000")] {
        let (_, p) = w.open(&frame, now).ok().unwrap();
        let e = approve(p, &opts(typed, false), &mut w.ctx(now + 1)).unwrap_err();
        assert!(matches!(e, CoreError::BadInput { .. }), "{typed:?}");
    }
    assert!(
        w.state.chain.is_empty(),
        "a refused approval writes nothing"
    );
    let (_, p) = w.open(&frame, now).ok().unwrap();
    let e = approve(p, &opts(Some(&rid), false), &mut w.ctx(expiry)).unwrap_err();
    assert!(matches!(e, CoreError::Refused { .. }) && e.reason().contains("expired"));
    assert!(w.state.chain.is_empty());
    let (_, p) = w.open(&frame, now).ok().unwrap();
    let typed = format!("  {}  ", rid.to_uppercase());
    approve(p, &opts(Some(&typed), false), &mut w.ctx(now + 1)).unwrap();
    // the requester was never verified: its counters moved nothing (lamport 0 + 1)
    assert_eq!(w.state.chain[0].lamport, 1);
}

#[test]
fn a_short_code_is_issued_once_per_credential_opens_with_its_code_and_expires_after_seven_days() {
    const CHEAP: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let frame = hx(&case(&f, "good")["frame_hex"]);
    let eph: [u8; 32] = hx(&case(&f, "good")["eph_secret_hex"]).try_into().unwrap();
    let host: [u8; 16] = hex::decode(f["host_uid"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let mut w = fx_world(&f, true);

    let (_, p) = w.open(&frame, now).ok().unwrap();
    let out = approve_with(p, &opts(None, true), &mut w.ctx(now + 2), &[CHEAP]).unwrap();
    let code = out.short_code.clone().expect("a short code was asked for");
    assert_eq!(code.code.len(), 19);
    assert!(
        code.code.split('-').count() == 4
            && code.code.split('-').all(|g| g.len() == 4
                && g.bytes()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()))
    );
    assert_eq!(
        code.valid_until,
        now + 2 + sshub_sync_proto::limits::SNAPSHOT_TTL_SECS
    );
    assert!(!code.weak);
    let resp = ShareResponseWire::decode(&out.frame).unwrap();
    let (share, ext) = unwrap_share_reply(&eph, &resp.ciphertext).unwrap();
    let (blob_host, blob) = ext.unwrap().snapshot.expect("the blob rides in the reply");
    assert_eq!(blob_host, host);
    let blob = SnapshotBlob::from_bytes(&blob).unwrap();
    assert_eq!(
        blob.host_uid,
        Some(host),
        "version 2: the credential is in the header"
    );
    let opened = snapshot_open_for(&blob, &host, &code.code, now + 5).unwrap();
    assert_eq!(opened.bytes, share.bytes);
    assert_eq!(opened.index, share.index);
    let rec = w
        .state
        .live_snapshot(&hex::encode(host), now + 60)
        .expect("the issue is recorded");
    assert_eq!(rec.expires_at, code.valid_until);

    // a second one is not issued while one is live, and the screen is told so
    let (view, p) = w.open(&frame, now + 60).ok().unwrap();
    assert_eq!(
        view.short_code,
        ShortCodeOffer::LiveUntil {
            expires_at: code.valid_until
        }
    );
    let out = approve_with(p, &opts(None, true), &mut w.ctx(now + 61), &[CHEAP]).unwrap();
    assert!(out.short_code.is_none());
    let resp = ShareResponseWire::decode(&out.frame).unwrap();
    assert!(unwrap_share_reply(&eph, &resp.ciphertext)
        .unwrap()
        .1
        .unwrap()
        .snapshot
        .is_none());
    assert_eq!(w.state.snapshots.len(), 1);

    // expiry frees the slot; so does the user declaring it dead
    let later = code.valid_until + 1;
    assert!(w.state.live_snapshot(&hex::encode(host), later).is_none());
    let (view, _) = w.open(&frame, now + 100).ok().unwrap();
    assert!(matches!(view.short_code, ShortCodeOffer::LiveUntil { .. }));
    w.state.invalidate_snapshot(&hex::encode(host));
    let (view, _) = w.open(&frame, now + 100).ok().unwrap();
    assert_eq!(view.short_code, ShortCodeOffer::Available);

    // a profile that cannot run falls back to the next one and is recorded as weak
    let bad = Argon2Profile {
        mem_kib: 1,
        iters: 1,
    };
    let (_, p) = w.open(&frame, now + 100).ok().unwrap();
    let out = approve_with(p, &opts(None, true), &mut w.ctx(now + 101), &[bad, CHEAP]).unwrap();
    assert!(out.short_code.unwrap().weak);
    assert!(w.state.snapshots.values().all(|s| s.weak));
    // and when none runs the whole approval fails before anything is written
    let len = w.state.chain.len();
    w.state.invalidate_snapshot(&hex::encode(host));
    let (_, p) = w.open(&frame, now + 100).ok().unwrap();
    assert!(approve_with(p, &opts(None, true), &mut w.ctx(now + 102), &[bad]).is_err());
    assert_eq!(w.state.chain.len(), len);
}

// ------------------------------------------------------------ the page's review repros (constructed challenges)

const NOW: i64 = 1_900_000_000;
const HOST: [u8; 16] = [0xab; 16];

fn requester() -> DeviceIdentity {
    DeviceIdentity::from_seed(&[9; 32], "sshub/test/requester")
}

fn phone_id() -> PhoneIdentity {
    PhoneIdentity::new(&[7; 32], 0).unwrap()
}

/// `shareReleaseOp` of the page tests: the raw canonical text of a ShareRelease, with every field overridable.
struct Spec {
    device: String,
    lamport: String,
    epoch: String,
    ts: i64,
    req_id: String,
    req: String,
    eph: Option<String>,
    host: String,
}

impl Spec {
    fn new() -> Self {
        let rid = requester().public().device_id();
        Self {
            device: rid.clone(),
            lamport: "5".into(),
            epoch: "3".into(),
            ts: NOW - 10,
            req_id: "7".into(),
            req: rid,
            eph: Some("02".repeat(32)),
            host: hex::encode(HOST),
        }
    }
    fn text(&self) -> String {
        format!(
            "{{\"device_id\":\"{}\",\"seq\":1,\"lamport\":{},\"prev_hash\":\"{}\",\"timestamp\":{},\"epoch\":{},\"alg\":[1,1],\"body\":{{\"ShareRelease\":{{\"host_uid\":\"{}\",\"request_id\":{},\"requester\":\"{}\"{}}}}}}}",
            self.device,
            self.lamport,
            "0".repeat(64),
            self.ts,
            self.epoch,
            self.host,
            self.req_id,
            self.req,
            self.eph.as_ref().map_or(String::new(), |e| format!(",\"eph_x25519_pub\":\"{e}\"")),
        )
    }
    /// A frame the way wire.rs lays it out, the op signed by the requester key.
    fn frame(&self, expiry: i64) -> Vec<u8> {
        let text = self.text();
        let sig = requester().sign(text.as_bytes());
        challenge(&text, sig, expiry)
    }
}

fn challenge(op: &str, sig: [u8; 64], expiry: i64) -> Vec<u8> {
    let share = Share {
        index: 1,
        bytes: vec![5; 32],
    };
    let blob = hybrid_wrap(
        &phone_id().public(),
        &wrapped_share_plaintext(&HOST, &share),
    );
    ChallengeWire {
        op_canonical: op.as_bytes().to_vec(),
        op_signature: sig,
        nonce: [1; 32],
        expiry,
        eph_x25519_pub: [2; 32],
        request_id: 7,
        phone_material: PhoneMaterial {
            kind: ShareKind::Hybrid,
            epoch: 3,
            share_blob: blob,
            storage_blob: Vec::new(),
        },
    }
    .encode()
}

fn known_world() -> World {
    let mut w = World::new(phone_id());
    w.enrol(
        &requester().public().device_id(),
        "laptop",
        &requester().public().ed25519_pub,
    );
    w
}

#[test]
fn f6_a_year_old_signed_release_inside_a_fresh_challenge_is_refused() {
    let s = Spec {
        ts: NOW - 365 * 86_400,
        ..Spec::new()
    };
    let e = known_world().open(&s.frame(NOW + 100), NOW).err().unwrap();
    assert!(e.reason.contains("minutes old"), "{}", e.reason);
}

#[test]
fn f6_the_expiry_must_sit_0_to_720_s_after_the_signed_op_a_future_op_is_refused_a_stale_one_is_flagged(
) {
    let run =
        |ts: i64, expiry: i64| known_world().open(&Spec { ts, ..Spec::new() }.frame(expiry), NOW);
    assert!(run(NOW + 50, NOW + 100).is_ok());
    assert!(run(NOW + 101, NOW + 100).is_err(), "expiry before the op");
    assert!(run(NOW + 601, NOW + 700).is_err(), "dated in the future");
    let Ok((stale, _)) = run(NOW - 300, NOW + 100) else {
        panic!("inside the skew window");
    };
    assert!(
        stale
            .op
            .rows
            .iter()
            .find(|r| r.label == "Op time")
            .unwrap()
            .danger,
        "but flagged"
    );
    let (fresh, _) = run(NOW - 10, NOW + 100).ok().unwrap();
    assert!(
        !fresh
            .op
            .rows
            .iter()
            .find(|r| r.label == "Op time")
            .unwrap()
            .danger
    );
    assert!(
        run(NOW - 700, NOW + 20).is_ok(),
        "exactly 720 s from op to expiry"
    );
    assert!(
        run(NOW - 701, NOW + 20).is_err(),
        "more than 120 s plus the skew from op to expiry"
    );
    assert!(
        run(NOW - 721, NOW + 1).is_err(),
        "more than 120 s plus the skew old"
    );
}

#[test]
fn s1_an_unknown_requester_is_flagged_for_the_typed_confirmation_a_verified_one_is_not() {
    let s = Spec::new();
    let mut w = World::new(phone_id());
    let (view, _) = w.open(&s.frame(NOW + 100), NOW).ok().unwrap();
    assert_eq!(view.type_to_approve.as_deref(), Some(s.req.as_str()));
    assert_eq!(view.requester, s.req);
    let (view, _) = known_world().open(&s.frame(NOW + 100), NOW).ok().unwrap();
    assert!(view.type_to_approve.is_none());
}

#[test]
fn f2_counters_of_an_unverified_op_are_not_adopted_and_a_verified_one_at_u64_max_cannot_push_the_chain_over(
) {
    let max = u64::MAX.to_string();
    let s = Spec {
        lamport: max.clone(),
        epoch: max.clone(),
        ..Spec::new()
    };
    let mut w = World::new(phone_id());
    let (_, p) = w.open(&s.frame(NOW + 100), NOW).ok().unwrap();
    assert!(!p.signature_verified);
    approve(p, &opts(Some(&s.req), false), &mut w.ctx(NOW)).unwrap();
    let mine = decode_op(&w.state.chain[0].canonical, w.state.chain[0].signature64()).unwrap();
    assert_eq!(
        (mine.lamport, mine.epoch),
        (1, 0),
        "the unverified op moved nothing"
    );

    let mut w = known_world();
    let (_, p) = w.open(&s.frame(NOW + 100), NOW).ok().unwrap();
    assert!(p.signature_verified);
    let e = approve(p, &opts(None, false), &mut w.ctx(NOW)).unwrap_err();
    assert!(matches!(e, CoreError::Refused { .. }), "{e}");
    assert!(w.state.chain.is_empty(), "nothing was signed");
}

#[test]
fn the_signed_op_must_agree_with_the_challenge_and_have_exactly_the_four_field_form() {
    let mut w = World::new(phone_id());
    assert!(w.open(&Spec::new().frame(NOW + 100), NOW).is_ok());
    let swapped = w
        .open(
            &Spec {
                eph: Some("03".repeat(32)),
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(
        swapped.reason.contains("another ephemeral key"),
        "{}",
        swapped.reason
    );
    let old = w
        .open(
            &Spec {
                eph: None,
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(
        old.reason.contains("malformed"),
        "the three field body is refused: {}",
        old.reason
    );
    for eph in ["02".repeat(31), "zz".repeat(32), "AB".repeat(32)] {
        assert!(
            w.open(
                &Spec {
                    eph: Some(eph.clone()),
                    ..Spec::new()
                }
                .frame(NOW + 100),
                NOW
            )
            .is_err(),
            "{eph}"
        );
    }
    // F5a: a non-numeric or string request id
    for rid in ["\"abc\"", "\"\"", "-1", "1.5", "18446744073709551616"] {
        assert!(
            w.open(
                &Spec {
                    req_id: rid.into(),
                    ..Spec::new()
                }
                .frame(NOW + 100),
                NOW
            )
            .is_err(),
            "{rid}"
        );
    }
    // another request id than the frame
    let other = w
        .open(
            &Spec {
                req_id: "8".into(),
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(other.reason.contains("different request id"));
    // requester mismatch
    let mism = w
        .open(
            &Spec {
                req: "ee".repeat(8),
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(mism.reason.contains("names"), "{}", mism.reason);
    // a bidi or short requester id is a malformed field, never shown
    let bad = w
        .open(
            &Spec {
                req: "ab\u{202e}ab".into(),
                device: "ab\u{202e}ab".into(),
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(bad.reason.contains("malformed"), "{}", bad.reason);
    // material wrapped for another host
    let swapped_host = w
        .open(
            &Spec {
                host: "cd".repeat(16),
                ..Spec::new()
            }
            .frame(NOW + 100),
            NOW,
        )
        .err()
        .unwrap();
    assert!(
        swapped_host.reason.contains("different credential"),
        "{}",
        swapped_host.reason
    );
    // expiry window of the challenge itself
    assert!(w.open(&Spec::new().frame(NOW - 1), NOW).is_err());
    assert!(w.open(&Spec::new().frame(NOW + 721), NOW).is_err());
    assert!(w
        .open(
            &Spec {
                ts: NOW,
                ..Spec::new()
            }
            .frame(NOW + 720),
            NOW
        )
        .is_ok());
    // hostile bytes in place of the frame
    assert!(w.open(&[], NOW).err().unwrap().reason.contains("malformed"));
    assert!(w.open(&Spec::new().frame(NOW + 100)[..40], NOW).is_err());
}

#[test]
fn an_op_that_is_not_canonical_is_refused_before_anything_else_is_looked_at() {
    let mut w = known_world();
    let good = Spec::new().text();
    for bad in [
        good.replace("\"seq\":1,", "\"seq\": 1,"),
        good.replace("ShareRelease", "Mystery"),
        good.replace("\"epoch\":3,", ""),
    ] {
        let e = w
            .open(
                &challenge(&bad, requester().sign(bad.as_bytes()), NOW + 100),
                NOW,
            )
            .err()
            .unwrap();
        assert!(e.reason.contains("operation is malformed"), "{}", e.reason);
    }
}

#[test]
fn the_share_words_leave_the_core_only_when_asked_for_by_hand() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let frame = hx(&case(&f, "good")["frame_hex"]);
    for (by_hand, want) in [(false, false), (true, true)] {
        let mut w = fx_world(&f, true);
        let (_, p) = w.open(&frame, now).ok().unwrap();
        let o = ReleaseOptions {
            by_hand,
            ..opts(None, false)
        };
        let out = approve(p, &o, &mut w.ctx(now + 3)).unwrap();
        assert_eq!(out.manual.is_some(), want, "by_hand {by_hand}");
        // the sealed reply and the audio frame do not depend on it
        assert_eq!(out.kind, 2);
        assert!(out.audio_frame.is_some());
    }
}

#[test]
fn a_restored_phone_is_refused_before_the_short_code_or_anything_else_runs() {
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let frame = hx(&case(&f, "good")["frame_hex"]);
    let mut w = fx_world(&f, true);
    w.state.head_pending = true;
    let (view, p) = w.open(&frame, now).ok().unwrap();
    assert!(view.head_pending, "the view says so up front");
    let rng_before = w.rng.clone();
    let e = approve(p, &opts(None, true), &mut w.ctx(now + 1)).unwrap_err();
    assert!(matches!(e, CoreError::Refused { .. }), "{e}");
    assert_eq!(e.reason(), crate::state::HEAD_PENDING_MESSAGE);
    assert_eq!(
        w.rng, rng_before,
        "no randomness was drawn, so no Argon2 salt was made"
    );
    assert!(w.state.snapshots.is_empty() && w.state.chain.is_empty());
    let (view, _) = fx_world(&f, true).open(&frame, now).ok().unwrap();
    assert!(!view.head_pending);
}

#[test]
fn a_failure_after_the_ops_were_appended_leaves_the_chain_the_lamport_and_the_short_codes_untouched(
) {
    const CHEAP: Argon2Profile = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };
    let f = fx();
    let now = f["now"].as_i64().unwrap();
    let frame = hx(&case(&f, "good")["frame_hex"]);
    let mut w = fx_world(&f, true);
    let (_, p) = w.open(&frame, now).ok().unwrap();
    let before = w.state.clone();
    crate::outgoing::fault::FAIL_NEXT_RENDER.with(|f| f.set(true));
    // the short code is recorded and the HeadAttest appended before the reply is rendered: then the render fails
    let r = w
        .ctx(now + 2)
        .atomically(|c| approve_with(p, &opts(None, true), c, &[CHEAP]));
    assert!(r.is_err());
    assert_eq!(w.state, before, "the state is exactly as it was");
    assert!(w.state.chain.is_empty() && w.state.snapshots.is_empty());
    assert_eq!(w.state.lamport, before.lamport);
    // without the transaction the same failure would have left them behind (this is what the courier tail would carry)
    let (_, p) = w.open(&frame, now).ok().unwrap();
    crate::outgoing::fault::FAIL_NEXT_RENDER.with(|f| f.set(true));
    assert!(approve_with(p, &opts(None, true), &mut w.ctx(now + 2), &[CHEAP]).is_err());
    assert_eq!(w.state.chain.len(), 1);
    assert_eq!(w.state.snapshots.len(), 1);
}

#[test]
fn an_allocation_failure_at_the_floor_takes_the_weak_profile_and_records_it() {
    use sshub_sync_proto::crypto::set_alloc_limit_kib;
    let floor = Argon2Profile {
        mem_kib: 256 * 1024,
        iters: 4,
    };
    let weak = Argon2Profile {
        mem_kib: 64,
        iters: 1,
    };
    let share = [7u8; 33];
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    // a device that cannot allocate the floor's 256 MiB: no abort, the weak profile seals and is recorded as weak
    set_alloc_limit_kib(Some(1024));
    let i = snapshot_issue::issue(&share, &[1; 16], 1_000, &mut rng, &[floor, weak]).unwrap();
    assert!(i.record.weak);
    assert_eq!((i.record.mem_kib, i.record.iters), (64, 1));
    // when the floor is allowed, the floor wins and is not weak (a cheap stand-in for the floor here)
    set_alloc_limit_kib(None);
    let cheap_floor = Argon2Profile {
        mem_kib: 128,
        iters: 1,
    };
    let i = snapshot_issue::issue(&share, &[1; 16], 1_000, &mut rng, &[cheap_floor, weak]).unwrap();
    assert!(!i.record.weak);
    // when not even the weak profile fits: a clean refusal, never an abort
    set_alloc_limit_kib(Some(8));
    match snapshot_issue::issue(&share, &[1; 16], 1_000, &mut rng, &[floor, weak]) {
        Err(e) => assert!(matches!(e, CoreError::Refused { .. }), "{e}"),
        Ok(_) => panic!("nothing fits, yet a code was issued"),
    }
    set_alloc_limit_kib(None);
}
