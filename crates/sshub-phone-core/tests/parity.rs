//! Parity with the page: the scenario of `assets/companion/scripts/parity-page.mjs` run through the phone core. The page's
//! results are committed at `assets/companion/test/fixtures/parity-page.json` (the gate regenerates them with node and
//! fails on a difference); every equality below is BYTE for byte, or sentence for sentence for the refusal texts, unless a
//! comment says otherwise.

use std::collections::BTreeMap;

use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
use serde_json::Value;
use sshub_phone_core::chain;
use sshub_phone_core::channels::audio_burst;
use sshub_phone_core::ctx::Ctx;
use sshub_phone_core::describe::describe_op;
use sshub_phone_core::identity::PhoneIdentity;
use sshub_phone_core::state::{PhoneState, RingDevice};
use sshub_phone_core::views::{AudioProfile, Opened, ReleaseOptions};
use sshub_phone_core::{cert, release, reshare};
use sshub_sync_proto::crypto::{chain_hash, open_sealed_with_secret, sas_words};
use sshub_sync_proto::enroll_core::{commit_of, confirm_message, transcript_hash, Reveal};
use sshub_sync_proto::fountain::fountain_encode;
use sshub_sync_proto::oplog::{CertScope, ConfirmItem, Op};
use sshub_sync_proto::share_release::open_envelope_share;
use sshub_sync_proto::uid::uid_from_hex;
use sshub_sync_proto::wire::{CertRequestWire, EnrollFrame, SignedOpBytes};

const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/companion/test/fixtures/");

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(format!("{FIX}{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn page() -> Value {
    load("parity-page.json")
}

fn hx(s: &str) -> Vec<u8> {
    hex::decode(s).unwrap()
}

fn h32(s: &str) -> [u8; 32] {
    hx(s).try_into().unwrap()
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    (0..n as u32).map(|i| (i * 31 + seed) as u8).collect()
}

fn b3hex(b: &[u8]) -> String {
    hex::encode(chain_hash(b))
}

fn rng() -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(7)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn rows_of(rows: &[sshub_phone_core::views::Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| serde_json::json!({ "label": r.label, "value": r.value, "danger": r.danger }))
            .collect(),
    )
}

#[test]
fn identity_and_wrap_generations() {
    let p = page();
    for e in p["identity"].as_array().unwrap() {
        let secret = vec![u8::from_str_radix(s(&e["secret"]), 16).unwrap(); 32];
        let id = PhoneIdentity::from_slice(&secret, e["wrap_epoch"].as_u64().unwrap() as u32).unwrap();
        let public = id.public();
        assert_eq!(id.device_id(), s(&e["device_id"]));
        assert_eq!(hex::encode(public.ed25519_pub), s(&e["ed_pub"]));
        assert_eq!(hex::encode(public.x25519_pub), s(&e["x_pub"]));
        assert_eq!(b3hex(&public.mlkem_pub), s(&e["mlkem_pub_hash"]), "ML-KEM public key of generation {}", e["wrap_epoch"]);
    }
}

#[test]
fn every_op_the_phone_authors_is_byte_identical() {
    let p = page();
    let now0 = p["meta"]["now0"].as_i64().unwrap();
    let g0 = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
    let g1 = g0.at_generation(1).unwrap();
    let mut st = PhoneState::default();
    let mut got: Vec<chain::Authored> = Vec::new();
    got.push(chain::head_attest(&mut st, &g0, now0, None).unwrap());
    got.push(chain::head_attest(&mut st, &g0, now0 + 1, Some(chain::Seen { lamport: 500, epoch: 3 })).unwrap());
    got.push(chain::red_phone_sign(&mut st, &g0, h32(&"11".repeat(32)), now0 + 2, None).unwrap());
    got.push(chain::wrap_key_rotate(&mut st, &g0, &g1, now0 + 3, None).unwrap());
    let a = uid_hex(&"0a".repeat(16));
    let b = uid_hex(&"0b".repeat(16));
    let issue = chain::cert_issue(&mut st, &g1, "cccccccccccccccc".into(), CertScope::Hosts(vec![a, b]), now0 + 86400, 5, 3, 4, now0 + 4, None).unwrap();
    let cert_id = issue.hash;
    got.push(issue);
    got.push(chain::cert_issue(&mut st, &g1, "dddddddddddddddd".into(), CertScope::Group(uid_hex(&"06".repeat(16))), now0 + 30 * 86400, 50, 3, 3, now0 + 5, None).unwrap());
    got.push(chain::cert_extend(&mut st, &g1, cert_id, 7, now0 + 6, None).unwrap());
    got.push(chain::cert_revoke(&mut st, &g1, cert_id, now0 + 7, None).unwrap());
    got.push(chain::reenroll_cancel(&mut st, &g1, h32(&"22".repeat(32)), now0 + 8, Some(chain::Seen { lamport: 9000, epoch: 9 })).unwrap());
    got.push(chain::share_storage_key(&mut st, &g1, 4, pattern(60, 1), now0 + 9, None).unwrap());
    got.push(
        chain::confirm_batch(
            &mut st,
            &g1,
            4,
            vec![
                ConfirmItem { host_uid: a, sealed_share: pattern(70, 2) },
                ConfirmItem { host_uid: b, sealed_share: pattern(70, 3) },
            ],
            now0 + 10,
            None,
        )
        .unwrap(),
    );
    let want = p["chain"]["ops"].as_array().unwrap();
    assert_eq!(got.len(), want.len());
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(hex::encode(&g.canonical), s(&w["canonical"]), "op {} canonical", i + 1);
        assert_eq!(hex::encode(g.signature), s(&w["signature"]), "op {} signature", i + 1);
    }
    assert_eq!(hex::encode(st.chain.last().unwrap().hash.clone()), s(&p["chain"]["head_hash"]));
    assert_eq!(hex::encode(cert_id), s(&p["chain"]["cert_id"]));
    let tail = chain::courier_ops(&st, 8);
    let want_tail = p["courier_tail"].as_array().unwrap();
    assert_eq!(tail.len(), want_tail.len());
    for ((c, sig), w) in tail.iter().zip(want_tail) {
        assert_eq!(hex::encode(c), s(&w["canonical"]));
        assert_eq!(hex::encode(sig), s(&w["signature"]));
    }
}

fn uid_hex(s: &str) -> [u8; 16] {
    uid_from_hex(s).unwrap()
}

#[test]
fn what_an_approval_screen_shows_is_the_same_text() {
    let p = page();
    let v = load("ops.vectors.json");
    let aliases: BTreeMap<String, String> = [("723ed85f0566df0c".to_string(), "terminal A".to_string())].into();
    let want = p["describe"].as_array().unwrap();
    assert_eq!(want.len(), v["vectors"].as_array().unwrap().len());
    for (w, x) in want.iter().zip(v["vectors"].as_array().unwrap()) {
        assert_eq!(s(&w["id"]), s(&x["id"]));
        let canonical = hx(s(&x["canonical_hex"]));
        let sig: [u8; 64] = hx(s(&x["signature_hex"])).try_into().unwrap();
        let op = Op::from_canonical(&canonical, sig).unwrap_or_else(|e| panic!("{}: {e:#}", s(&x["id"])));
        let d = describe_op(&op, &canonical, &aliases, None, 120);
        assert_eq!(d.headline, s(&w["headline"]), "{} headline", s(&w["id"]));
        assert_eq!(d.requester.as_deref(), w["requester"].as_str(), "{} requester", s(&w["id"]));
        assert_eq!(rows_of(&d.rows), w["rows"], "{} rows", s(&w["id"]));
        assert_eq!(u64::from(d.danger_count), w["danger_count"].as_u64().unwrap());
    }
}

#[test]
fn share_release_every_challenge_same_checks_refusals_and_reply() {
    let p = page();
    let fx = load("challenges.json");
    let now = fx["now"].as_i64().unwrap();
    let secret = hx(s(&fx["phone"]["secret_hex"]));
    let by_name: BTreeMap<String, &Value> = fx["challenges"].as_array().unwrap().iter().map(|c| (s(&c["name"]).to_string(), c)).collect();
    for rec in p["release"].as_array().unwrap() {
        let name = s(&rec["name"]);
        let known = rec["ring_known"].as_bool().unwrap();
        let c = by_name[name];
        let mut id = PhoneIdentity::from_slice(&secret, 0).unwrap();
        let mut state = PhoneState::default();
        if known {
            state.ring.insert(
                s(&fx["requester"]["pub"]["device_id"]).to_string(),
                RingDevice { name: "work laptop".into(), ed25519_pub: s(&fx["requester"]["pub"]["ed25519_pub"]).to_string(), enrolled_at: 0 },
            );
        }
        let mut r = rng();
        let mut ctx = Ctx { id: &mut id, state: &mut state, rng: &mut r, now };
        let opened = release::open_challenge(&hx(s(&c["frame_hex"])), &mut ctx, 1);
        let label = format!("{name} (requester known: {known})");
        if !rec["ok"].as_bool().unwrap() {
            let view = opened.err().unwrap_or_else(|| panic!("{label}: the page refused, the core did not"));
            if name == "wrong-phone" {
                // the sentence is the same; the tail is each crypto library's own words for "the tag did not verify"
                let prefix = "this phone cannot release for this request: ";
                assert!(view.reason.starts_with(prefix) && s(&rec["refusal"]).starts_with(prefix), "{label}: {}", view.reason);
            } else {
                assert_eq!(view.reason, s(&rec["refusal"]), "{label} refusal");
            }
            let checks: Vec<(String, bool, bool)> = view.checks.iter().map(|k| (k.text.clone(), k.warn, k.ok)).collect();
            let want: Vec<(String, bool, bool)> = rec["checks"].as_array().unwrap().iter().map(|k| (s(&k["text"]).to_string(), k["warn"].as_bool().unwrap(), k["ok"].as_bool().unwrap())).collect();
            if name == "wrong-phone" {
                let strip = |v: Vec<(String, bool, bool)>| v.into_iter().map(|(t, w, o)| (if o { t } else { t.chars().take(44).collect() }, w, o)).collect::<Vec<_>>();
                assert_eq!(strip(checks), strip(want), "{label} checks");
            } else {
                assert_eq!(checks, want, "{label} checks");
            }
            continue;
        }
        let (view, pending) = opened.unwrap_or_else(|v| panic!("{label}: the core refused: {}", v.reason));
        assert_eq!(view.binding_words, s(&rec["binding_words"]), "{label}");
        assert_eq!(view.binding_hash, s(&rec["binding_hash"]), "{label}");
        assert_eq!(view.type_to_approve.is_some(), rec["unknown_requester"].as_bool().unwrap(), "{label}");
        assert_eq!(view.requester, s(&rec["requester"]));
        assert_eq!(view.host_uid, s(&rec["host"]));
        assert_eq!(view.op.headline, s(&rec["description"]["headline"]), "{label}");
        assert_eq!(rows_of(&view.op.rows), rec["description"]["rows"], "{label} rows");
        let checks: Vec<(String, bool, bool)> = view.checks.iter().map(|k| (k.text.clone(), k.warn, k.ok)).collect();
        let want: Vec<(String, bool, bool)> = rec["checks"].as_array().unwrap().iter().map(|k| (s(&k["text"]).to_string(), k["warn"].as_bool().unwrap(), k["ok"].as_bool().unwrap())).collect();
        assert_eq!(checks, want, "{label} checks");

        let opts = ReleaseOptions { typed_id: view.type_to_approve.clone(), qr: false, clipboard: false, sound: true, audio_profile: AudioProfile::Fast, issue_short_code: false };
        let out = release::approve(pending, &opts, &mut ctx).unwrap_or_else(|e| panic!("{label}: approve: {e}"));
        let eph: [u8; 32] = hx(s(&c["eph_secret_hex"])).try_into().unwrap();
        assert_eq!(hex::encode(open_sealed_with_secret(&eph, &out.frame[23..]).unwrap()), s(&rec["reply_full"]), "{label} full reply plaintext");
        let short = out.audio_frame.clone().expect("the audio frame is the share only reply");
        assert_eq!(hex::encode(open_sealed_with_secret(&eph, &short[23..]).unwrap()), s(&rec["reply_short"]), "{label} short reply plaintext");
        let head = &state.chain[0];
        assert_eq!(hex::encode(&head.canonical), s(&rec["head_attest"]["canonical"]), "{label} head attest");
        assert_eq!(hex::encode(&head.signature), s(&rec["head_attest"]["signature"]), "{label} head attest signature");
        let manual = out.manual.expect("the manual entry");
        assert_eq!(u64::from(manual.index), rec["manual"]["index"].as_u64().unwrap());
        assert_eq!(manual.words, s(&rec["manual"]["words"]));
    }
}

#[test]
fn trust_session_requests_same_problems_and_rows() {
    let p = page();
    const NOW: i64 = 1_893_452_400;
    const DAY: i64 = 86_400;
    let phone = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
    let delegate = |s: &str| -> [u8; 8] { hx(s).try_into().unwrap() };
    let hosts = |n: usize| (0..n).map(|_| uid_hex(&"0a".repeat(16))).collect::<Vec<_>>();
    let base = || CertRequestWire { device_id: delegate("cccccccccccccccc"), scope: CertScope::Hosts(hosts(1)), expiry: NOW + 7 * DAY, max_new_hosts_per_day: 2, epoch_lo: 3, epoch_hi: 4 };
    let mut cases: Vec<(&str, CertRequestWire)> = Vec::new();
    cases.push(("ok", base()));
    cases.push(("ended", CertRequestWire { expiry: NOW - 1, ..base() }));
    cases.push(("too_long", CertRequestWire { expiry: NOW + 91 * DAY, ..base() }));
    cases.push(("budget_zero", CertRequestWire { max_new_hosts_per_day: 0, ..base() }));
    cases.push(("budget_over", CertRequestWire { max_new_hosts_per_day: 51, ..base() }));
    cases.push(("no_hosts", CertRequestWire { scope: CertScope::Hosts(vec![]), ..base() }));
    cases.push(("group", CertRequestWire { scope: CertScope::Group(uid_hex(&"06".repeat(16))), ..base() }));
    cases.push(("inverted", CertRequestWire { epoch_lo: 5, epoch_hi: 4, ..base() }));
    cases.push(("wide", CertRequestWire { epoch_lo: 3, epoch_hi: 9, ..base() }));
    cases.push(("wrong_start", CertRequestWire { epoch_lo: 2, epoch_hi: 3, ..base() }));
    cases.push(("self", CertRequestWire { device_id: delegate(phone.device_id()), ..base() }));
    let want = p["cert_requests"].as_array().unwrap();
    assert_eq!(want.len(), cases.len());
    for (w, (name, req)) in want.iter().zip(cases) {
        assert_eq!(s(&w["name"]), name);
        let mut id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
        let mut state = PhoneState::default();
        state.epoch = 3;
        state.aliases.insert("cccccccccccccccc".into(), "build box".into());
        let mut r = rng();
        let ctx = Ctx { id: &mut id, state: &mut state, rng: &mut r, now: NOW };
        let (opened, _) = cert::open_cert_request(&req.encode(), &ctx, 1);
        match opened {
            Opened::CertRequest { view } => {
                let problems: Vec<String> = view.problems.clone();
                let want_problems: Vec<String> = w["problems"].as_array().unwrap().iter().map(|x| s(x).to_string()).collect();
                assert_eq!(problems, want_problems, "{name} problems");
                assert_eq!(view.headline, s(&w["headline"]), "{name} headline");
                assert_eq!(rows_of(&view.rows), w["rows"], "{name} rows");
                assert_eq!(view.delegate_known, w["delegate_known"].as_bool().unwrap(), "{name}");
            }
            // an empty host list never decodes (the wire rule `1..=64`); the page's problem sentence for it is a second
            // line of defence behind its own decoder
            Opened::Refused { view } if name == "no_hosts" => assert!(view.reason.contains("1..=64"), "{}", view.reason),
            other => panic!("{name}: not a certificate view: {other:?}"),
        }
    }
}

#[test]
fn reshare_fixture_the_core_combines_to_the_same_shares() {
    let p = page();
    let fx = load("reshare.json");
    let epoch = fx["epoch"].as_u64().unwrap();
    let mut id = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
    let phone_id = id.device_id().to_string();
    let mut state = PhoneState::default();
    for c in fx["contributors"].as_array().unwrap() {
        let did = s(&c["device_id"]).to_string();
        state.ring.insert(did.clone(), RingDevice { name: format!("terminal {}", &did[..4]), ed25519_pub: s(&c["ed25519_pub"]).to_string(), enrolled_at: 0 });
    }
    let list: Vec<SignedOpBytes> = fx["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (hx(s(&o["canonical_hex"])), hx(s(&o["sig_hex"])).try_into().unwrap()))
        .filter(|(c, _)| {
            let v: Value = serde_json::from_slice(c).unwrap();
            let body = v["body"].as_object().unwrap();
            body.contains_key("ShareReshareBatch") || (body.get("ShareReshareSession").map_or(false, |f| f["to"] == phone_id.as_str()))
        })
        .collect();
    let now0 = p["meta"]["now0"].as_i64().unwrap();
    let mut r = rng();
    let mut ctx = Ctx { id: &mut id, state: &mut state, rng: &mut r, now: now0 };
    let (opened, pending) = reshare::open_reshare(list, &mut ctx, 1);
    let view = match opened {
        Opened::Reshare { view } => view,
        other => panic!("not a reshare view: {other:?}"),
    };
    let want = &p["reshare"];
    assert_eq!(want["ok"], true);
    assert_eq!(view.epoch, epoch);
    assert_eq!(view.my_index as u64, want["my_index"].as_u64().unwrap());
    assert_eq!(view.holders_after.iter().map(|h| h.device_id.clone()).collect::<Vec<_>>(), want["holders"].as_array().unwrap().iter().map(|x| s(x).to_string()).collect::<Vec<_>>());
    assert_eq!(u64::from(view.credentials_ready), want["credentials"].as_u64().unwrap());
    let mut have: Vec<(String, bool, bool, bool)> = view.contributors.iter().map(|c| (c.device_id.clone(), c.verified, c.known, c.has_session)).collect();
    let mut exp: Vec<(String, bool, bool, bool)> = want["contributors"].as_array().unwrap().iter().map(|c| (s(&c["id"]).to_string(), c["verified"].as_bool().unwrap(), c["known"].as_bool().unwrap(), c["session"].as_bool().unwrap())).collect();
    have.sort();
    exp.sort();
    assert_eq!(have, exp);
    let out = reshare::publish_reshare(pending.expect("a plan"), &mut ctx).unwrap();
    assert!(out.kind == 5);
    // the storage key sits sealed in the published ShareStorageKey op; the confirm items open under it
    let mut key = None;
    for e in &ctx.state.chain {
        let v: Value = serde_json::from_slice(&e.canonical).unwrap();
        if let Some(f) = v["body"].get("ShareStorageKey") {
            let sealed = sshub_sync_proto::oplog::canonical::b64_decode(s(&f["sealed"])).unwrap();
            let k = ctx.id.unwrap(&sealed).unwrap();
            key = Some(<[u8; 32]>::try_from(k.as_slice()).unwrap());
        }
    }
    let key = key.expect("a storage key was published");
    let my_index = view.my_index as u8;
    let mut shares = BTreeMap::new();
    for e in &ctx.state.chain {
        let v: Value = serde_json::from_slice(&e.canonical).unwrap();
        if let Some(f) = v["body"].get("ShareReshareConfirmBatch") {
            for it in f["items"].as_array().unwrap() {
                let host = s(&it["host_uid"]);
                let sealed = sshub_sync_proto::oplog::canonical::b64_decode(s(&it["sealed_share"])).unwrap();
                let mut aad = hx(host);
                aad.extend_from_slice(&epoch.to_be_bytes());
                aad.push(my_index);
                let share = open_envelope_share(&key, &aad, &sealed).unwrap();
                let mut plain = vec![share.index];
                plain.extend_from_slice(&share.bytes);
                shares.insert(host.to_string(), hex::encode(plain));
            }
        }
    }
    let want_shares: BTreeMap<String, String> = want["shares"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), s(v).to_string())).collect();
    assert_eq!(shares, want_shares);
}

#[test]
fn enrollment_commitments_reveals_sas_and_confirmations() {
    let p = page();
    let reveal = |v: u8, name: &str| Reveal { ed25519_pub: [v; 32], x25519_pub: [v + 1; 32], mlkem_pub: vec![v + 2; 1184], eph: [v + 3; 32], nonce: [v + 4; 32], name: name.into(), attended: true };
    let me = reveal(1, "my phone");
    let them = reveal(0x21, "work laptop");
    let th = transcript_hash(&me, &them);
    let w = &p["enroll"];
    assert_eq!(hex::encode(commit_of(0, &me)), s(&w["phone_commit"]));
    assert_eq!(hex::encode(commit_of(1, &them)), s(&w["terminal_commit"]));
    assert_eq!(hex::encode(EnrollFrame::Commit { role: 0, commit: commit_of(0, &me) }.encode()), s(&w["commit_frame"]));
    assert_eq!(hex::encode(EnrollFrame::Reveal { role: 0, reveal: me.bytes() }.encode()), s(&w["reveal_frame"]));
    assert_eq!(hex::encode(th), s(&w["transcript_hash"]));
    assert_eq!(sas_words(&th), s(&w["sas_words"]));
    assert_eq!(hex::encode(confirm_message(0, true, &th)), s(&w["confirm_message_ok"]));
    assert_eq!(hex::encode(confirm_message(0, false, &th)), s(&w["confirm_message_no"]));
    assert_eq!(hex::encode(EnrollFrame::Confirm { role: 0, ok: true, sig: [9; 64] }.encode()), s(&w["confirm_frame"]));
}

#[test]
fn fountain_frames_and_the_fsk_burst() {
    let p = page();
    let payload = pattern(1000, 5);
    let frames = fountain_encode(&payload, 106, 25).unwrap();
    assert_eq!(frames.len() as u64, p["fountain"]["frames"].as_u64().unwrap());
    assert_eq!(b3hex(&frames.concat()), s(&p["fountain"]["frames_hash"]));
    for (profile, name) in [(AudioProfile::Fast, "fast"), (AudioProfile::Slow, "slow")] {
        let pcm = audio_burst(&pattern(128, 7), profile).unwrap();
        let want = &p["fsk"][name];
        assert_eq!(pcm.len() as u64, want["samples"].as_u64().unwrap(), "{name} sample count");
        let step = want["step"].as_u64().unwrap() as usize;
        let mut worst = 0i32;
        for (k, v) in want["probe"].as_array().unwrap().iter().enumerate() {
            let w = v.as_i64().unwrap() as i32;
            let g = i32::from(pcm[k * step]);
            worst = worst.max((g - w).abs());
        }
        // The page computes sin and cos in double precision (V8) and stores float32; libm may differ by an ulp,
        // which moves a quantised sample by at most one step.
        assert!(worst <= 1, "{name}: the PCM differs by {worst} least significant bits");
    }
}
