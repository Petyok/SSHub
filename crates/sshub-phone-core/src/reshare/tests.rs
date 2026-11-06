//! Ports of the reshare tests of the page: `phone2.test.mjs` (receiver, lying contributor, bad signature, foreign
//! holder set, delivery cap), `phone3.test.mjs` (the Rust fixture `reshare.json`), `review-app-round1` (F5c, S2),
//! `review-app-round2` (PF2, PF5) and `review-app-round3` (RP1). The scenario is built with the proto primitives.

use std::collections::BTreeMap;

use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
use sshub_sync_proto::crypto::{
    aead_open, aead_seal, derive_key, hybrid_wrap, reshare_contribute, shamir_combine,
    shamir_split_at, DeviceIdentity, Share, ALG_CURRENT,
};
use sshub_sync_proto::wire::PhoneOpsWire;

use super::*;
use crate::state::RingDevice;
use crate::views::Outgoing;

const NOW: i64 = 1_893_452_500;
const EPOCH: u64 = 1;
const HOST: [u8; 16] = [0x0a; 16];

struct Fx {
    id: PhoneIdentity,
    state: PhoneState,
    rng: ChaCha8Rng,
}

impl Fx {
    fn new(secret: u8, ring: &BTreeMap<String, RingDevice>) -> Self {
        let state = PhoneState {
            ring: ring.clone(),
            ..PhoneState::default()
        };
        Self {
            id: PhoneIdentity::new(&[secret; 32], 0).unwrap(),
            state,
            rng: ChaCha8Rng::seed_from_u64(3),
        }
    }
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx {
            id: &mut self.id,
            state: &mut self.state,
            rng: &mut self.rng,
            now: NOW,
        }
    }
}

fn dev(n: u8) -> DeviceIdentity {
    DeviceIdentity::from_seed(&[n; 32], &format!("sshub/test/contrib/{n}"))
}

fn op(author: &DeviceIdentity, lamport: u64, body: OpBody) -> SignedOpBytes {
    let mut o = Op {
        device_id: author.public().device_id(),
        seq: lamport,
        lamport,
        prev_hash: [0; 32],
        timestamp: 1_893_452_400,
        epoch: 0,
        alg: ALG_CURRENT,
        body,
        signature: [0; 64],
    };
    let canonical = o.canonical_bytes();
    o.signature = author.sign(&canonical);
    (canonical, o.signature)
}

fn commit(from: u8, index: u8, sub: &[u8]) -> Vec<u8> {
    derive_key(
        "sshub/sync/reshare/commit/v1",
        &[[from, index].as_slice(), sub].concat(),
    )
    .to_vec()
}

fn ring_of(devs: &[(&str, &DeviceIdentity)]) -> BTreeMap<String, RingDevice> {
    devs.iter()
        .map(|(name, d)| {
            (
                d.public().device_id(),
                RingDevice {
                    name: (*name).to_string(),
                    ed25519_pub: hex::encode(d.public().ed25519_pub),
                    enrolled_at: 0,
                },
            )
        })
        .collect()
}

/// What one contributor writes for the phone: its session op (unless it re-uses another's) and its batch.
struct Writer<'a> {
    phone: &'a PhoneIdentity,
    epoch: u64,
    holders: Vec<String>,
    indices: Vec<u8>,
    lamport: u64,
    signed: Vec<SignedOpBytes>,
    rng: ChaCha8Rng,
}

impl Writer<'_> {
    fn key(&mut self) -> [u8; 32] {
        let mut k = [0u8; 32];
        self.rng.fill_bytes(&mut k);
        k
    }

    /// `session`: `Some(key)` re-uses a key (no session op), `None` draws one and writes a session op for the phone.
    /// `author_dev` writes and signs the ops. `lie`: the commitment for the phone is wrong.
    fn contribute(
        &mut self,
        author_dev: &DeviceIdentity,
        old: &Share,
        session: Option<[u8; 32]>,
        lie: bool,
    ) -> (ReshareContribution, [u8; 32]) {
        let author = author_dev.public().device_id();
        let c = reshare_contribute(old, &self.indices, 2);
        let phone_id = self.phone.device_id().to_string();
        let session_key = match session {
            Some(k) => k,
            None => {
                let k = self.key();
                let sealed = hybrid_wrap(&self.phone.public(), &k);
                self.signed.push(op(
                    author_dev,
                    self.lamport,
                    OpBody::ShareReshareSession {
                        epoch: self.epoch,
                        to: phone_id.clone(),
                        sealed,
                    },
                ));
                self.lamport += 1;
                k
            }
        };
        let mut sub_shares = Vec::new();
        let mut commitments = Vec::new();
        for (j, to) in self.holders.clone().iter().enumerate() {
            let (idx, sub) = &c.sub_shares[j];
            let plain = [
                0u64.to_be_bytes().as_slice(),
                [c.from].as_slice(),
                sub.as_slice(),
            ]
            .concat();
            let key = if *to == phone_id {
                session_key
            } else {
                self.key()
            };
            sub_shares.push((
                to.clone(),
                aead_seal(&key, &sub_aad(&HOST, self.epoch, &author, to), &plain),
            ));
            let committed = if lie && *to == phone_id {
                vec![0u8; 32]
            } else {
                sub.clone()
            };
            commitments.push(commit(c.from, *idx, &committed));
        }
        let body = OpBody::ShareReshareBatch {
            epoch: self.epoch,
            new_holders: self.holders.clone(),
            items: vec![ReshareItem {
                host_uid: HOST,
                sub_shares,
                commitments,
            }],
        };
        self.signed.push(op(author_dev, self.lamport, body));
        self.lamport += 1;
        (c, session_key)
    }
}

struct Sc {
    signed: Vec<SignedOpBytes>,
    holders: Vec<String>,
    ring: BTreeMap<String, RingDevice>,
    secret: [u8; 32],
    contribs: Vec<ReshareContribution>,
}

/// Two contributors (devices A and B, old shares 1 and 2 of a 3 way sharing) reshare one credential to the new holders
/// [A, B, phone, last]. `one_author`: B's batch is written and signed by A with another `from` byte, re-using A's key.
fn scenario(phone: &PhoneIdentity, epoch: u64, lie: bool, one_author: bool, last: &str) -> Sc {
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let mut secret = [0u8; 32];
    rng.fill_bytes(&mut secret);
    let old = shamir_split_at(&secret, 2, &[1, 2, 3]);
    let (a, b) = (dev(1), dev(2));
    let mut holders = vec![
        a.public().device_id(),
        b.public().device_id(),
        phone.device_id().to_string(),
        last.to_string(),
    ];
    holders.sort();
    let indices: Vec<u8> = (1..=holders.len() as u8).collect();
    let mut w = Writer {
        phone,
        epoch,
        holders: holders.clone(),
        indices,
        lamport: 10,
        signed: Vec::new(),
        rng,
    };
    let (ca, key_a) = w.contribute(&a, &old[0], None, false);
    let (cb, _) = if one_author {
        w.contribute(&a, &old[1], Some(key_a), false)
    } else {
        w.contribute(&b, &old[1], None, lie)
    };
    Sc {
        signed: w.signed,
        holders,
        ring: ring_of(&[("terminal A", &a), ("terminal B", &b)]),
        secret,
        contribs: vec![ca, cb],
    }
}

fn phone_of(n: u8) -> PhoneIdentity {
    PhoneIdentity::new(&[n; 32], 0).unwrap()
}

/// `plan` on a list, the way `open_reshare` calls it.
fn run(
    id: &mut PhoneIdentity,
    state: &PhoneState,
    ops: &[SignedOpBytes],
    epoch: u64,
) -> std::result::Result<Plan, Fail> {
    let parsed = parse_ops(ops).map_err(Fail::from)?;
    plan(
        id,
        state,
        &mut ChaCha8Rng::seed_from_u64(1),
        ops,
        &parsed,
        epoch,
    )
}

fn reshare_view(o: Opened) -> ReshareView {
    match o {
        Opened::Reshare { view } => view,
        other => panic!("not a reshare view: {other:?}"),
    }
}

fn refusal(o: Opened) -> RefusalView {
    match o {
        Opened::Refused { view } => view,
        other => panic!("not a refusal: {other:?}"),
    }
}

fn ops_of(out: &Outgoing) -> Vec<Op> {
    PhoneOpsWire::decode(&out.frame)
        .unwrap()
        .ops
        .iter()
        .map(|o| decode_op(&o.0, o.1).unwrap())
        .collect()
}

#[test]
fn the_phone_adopts_a_reshare_its_new_share_combines_with_another_new_holder_to_the_secret() {
    let id = phone_of(0x42);
    let sc = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let mut fx = Fx::new(0x42, &sc.ring);
    let (opened, pending) = open_reshare(sc.signed.clone(), &mut fx.ctx(), 11);
    let v = reshare_view(opened);
    let my_index = (sc
        .holders
        .iter()
        .position(|h| h == fx.id.device_id())
        .unwrap()
        + 1) as u8;
    assert_eq!(
        (v.token, v.epoch, v.holders, v.my_index),
        (11, EPOCH, 4, u32::from(my_index))
    );
    assert_eq!((v.credentials_ready, v.waiting), (1, 0));
    assert!(
        v.storage_key_new,
        "a first adoption publishes a storage key"
    );
    assert_eq!(v.contributors.len(), 2);
    assert!(v
        .contributors
        .iter()
        .all(|c| c.verified && c.has_session && c.known));
    assert!(v.contributors.iter().any(|c| c.name == "terminal A"));
    assert_eq!(
        v.holders_after.iter().filter(|h| h.is_this_phone).count(),
        1
    );
    assert!(v
        .holders_after
        .iter()
        .any(|h| h.is_this_phone && h.name == "this phone"));

    let out = publish_reshare(pending.unwrap(), &mut fx.ctx()).unwrap();
    let ops = ops_of(&out);
    assert!(matches!(ops[0].body, OpBody::ShareStorageKey { .. }));
    assert!(matches!(
        ops[1].body,
        OpBody::ShareReshareConfirmBatch { .. }
    ));
    for o in &ops {
        assert!(o.verify(&fx.id.public().ed25519_pub));
        assert_eq!(o.device_id, fx.id.device_id());
    }
    // the sealed share opens with the storage key the chain holds, under the envelope aad
    let key = own_storage_key(&mut fx.id, &fx.state.chain, EPOCH)
        .expect("the storage key is on the chain");
    let OpBody::ShareReshareConfirmBatch { epoch, items } = &ops[1].body else {
        unreachable!()
    };
    assert_eq!((*epoch, items.len(), items[0].host_uid), (EPOCH, 1, HOST));
    let mine = aead_open(
        &key,
        &envelope_aad(&HOST, EPOCH, my_index),
        &items[0].sealed_share,
    )
    .unwrap();
    assert_eq!(mine[0], my_index);
    // another new holder's share from the same contributions (what its own receiver would compute)
    let d_index = (sc
        .holders
        .iter()
        .position(|h| h == "dddddddddddddddd")
        .unwrap()
        + 1) as u8;
    let for_d: Vec<ReshareContribution> = sc
        .contribs
        .iter()
        .map(|c| ReshareContribution {
            from: c.from,
            sub_shares: vec![c.sub_shares[usize::from(d_index) - 1].clone()],
            commitments: vec![c.commitments[usize::from(d_index) - 1].clone()],
        })
        .collect();
    let other = reshare_combine(d_index, &for_d).unwrap();
    let mine = Share {
        index: mine[0],
        bytes: mine[1..].to_vec(),
    };
    assert_eq!(shamir_combine(&[mine, other]).unwrap(), sc.secret);

    // a second pass reuses the published storage key
    let (opened, pending) = open_reshare(sc.signed.clone(), &mut fx.ctx(), 12);
    assert!(!reshare_view(opened).storage_key_new);
    assert!(pending.unwrap().storage_sealed.is_none());
}

#[test]
fn the_phone_sealing_is_reproducible_with_the_same_randomness_and_reads_the_proto_layout() {
    let id = phone_of(0x42);
    let sc = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let mut a = Fx::new(0x42, &sc.ring);
    let mut b = Fx::new(0x42, &sc.ring);
    let (_, pa) = open_reshare(sc.signed.clone(), &mut a.ctx(), 1);
    let (_, pb) = open_reshare(sc.signed.clone(), &mut b.ctx(), 1);
    // the nonce of every sealed share comes from the injected RNG (the storage key wrap alone draws from the OS)
    let (pa, pb) = (pa.unwrap(), pb.unwrap());
    assert_eq!(pa.items, pb.items);
    assert_ne!(pa.storage_sealed, pb.storage_sealed);
    // `nonce(24) || ciphertext+tag`: 24 + (1 + 32) + 16 bytes
    assert_eq!(pa.items[0].sealed_share.len(), 24 + 33 + 16);
}

#[test]
fn a_lying_contributor_a_bad_signature_and_a_foreign_holder_set_are_not_adopted() {
    let id = phone_of(0x42);
    let lie = scenario(&id, EPOCH, true, false, "dddddddddddddddd");
    let mut fx = Fx::new(0x42, &lie.ring);
    let (opened, pending) = open_reshare(lie.signed.clone(), &mut fx.ctx(), 1);
    assert!(refusal(opened)
        .reason
        .contains("two readable contributions"));
    assert!(pending.is_none());
    assert!(fx.state.chain.is_empty(), "a refusal authors nothing");

    let good = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let mut forged = good.signed.clone();
    forged[1].1[3] ^= 1;
    let f = run(&mut fx.id, &Fx::new(0x42, &good.ring).state, &forged, EPOCH).unwrap_err();
    assert!(f.error.contains("does not verify"), "{}", f.error);

    let mut other = phone_of(0x43);
    let o = run(
        &mut other,
        &Fx::new(0x43, &good.ring).state,
        &good.signed,
        EPOCH,
    )
    .unwrap_err();
    assert!(o.error.contains("not a holder"), "{}", o.error);
    let state = Fx::new(0x42, &good.ring).state;
    assert!(run(&mut fx.id, &state, &good.signed, 9)
        .unwrap_err()
        .error
        .contains("no reshare batches"));
}

#[test]
fn s2_two_batches_written_by_one_device_are_not_two_contributors_and_su2_unmet_contributors_prove_nothing(
) {
    let id = phone_of(0x42);
    let sc = scenario(&id, EPOCH, false, true, "dddddddddddddddd");
    let mut fx = Fx::new(0x42, &sc.ring);
    let f = run(&mut fx.id, &fx.state, &sc.signed, EPOCH).unwrap_err();
    assert!(
        f.error.contains("fewer than 2 contributors"),
        "one author with two from-bytes must not combine a share: {}",
        f.error
    );
    let honest = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    assert!(run(&mut fx.id, &fx.state, &honest.signed, EPOCH).is_ok());

    // SU2: the phone has met nobody
    let mut stranger = Fx::new(0x42, &BTreeMap::new());
    let (opened, pending) = open_reshare(honest.signed.clone(), &mut stranger.ctx(), 1);
    let r = refusal(opened);
    assert!(r.reason.contains("fewer than 2 contributors"));
    assert!(pending.is_none());
    assert_eq!(r.checks.len(), 2);
    assert!(
        r.checks.iter().all(|c| c.warn
            && c.text.contains("not known to this phone")
            && c.text.contains("signature not checked")),
        "{:?}",
        r.checks
    );
}

/// RP1: real ring terminals A and B (known to the phone, genuine signatures) contribute a batch each but NO session to the
/// phone, so their sub-shares cannot be opened. Invented authors X and Y (unknown to the phone) carry the sessions and
/// sub-shares that would be combined: the plan must not combine them.
#[test]
fn rp1_k_verified_contributions_are_combined_not_k_verified_signatures_next_to_unverified_shares() {
    let id = phone_of(31);
    let mut rng = ChaCha8Rng::seed_from_u64(8);
    let mut secret = [0u8; 32];
    rng.fill_bytes(&mut secret);
    let old = shamir_split_at(&secret, 2, &[1, 2, 3]);
    let (a, b, x, y) = (dev(1), dev(2), dev(3), dev(4));
    let mut holders = vec![
        a.public().device_id(),
        b.public().device_id(),
        id.device_id().to_string(),
        "dddddddddddddddd".to_string(),
    ];
    holders.sort();
    let mut w = Writer {
        phone: &id,
        epoch: EPOCH,
        holders,
        indices: vec![1, 2, 3, 4],
        lamport: 10,
        signed: Vec::new(),
        rng,
    };
    // A and B write batches whose phone sub-share is sealed under a key the phone never gets (no session op)
    let blind_a = w.key();
    let blind_b = w.key();
    w.contribute(&a, &old[0], Some(blind_a), false);
    w.contribute(&b, &old[1], Some(blind_b), false);
    w.contribute(&x, &old[0], None, false);
    w.contribute(&y, &old[1], None, false);
    let ring = ring_of(&[("A", &a), ("B", &b)]);
    let mut fx = Fx::new(31, &ring);
    let (opened, pending) = open_reshare(w.signed.clone(), &mut fx.ctx(), 1);
    assert!(
        pending.is_none(),
        "the two shares that open came from authors this phone never met"
    );
    let r = refusal(opened);
    assert!(
        r.reason.contains("two readable contributions"),
        "{}",
        r.reason
    );
    assert_eq!(r.checks.len(), 4);
    // the page's contributor summary shows who had a session and who was verified
    assert!(r
        .checks
        .iter()
        .any(|c| c.text.contains("signature verified")
            && c.text.contains("no session key for this phone")));
    assert!(fx.state.chain.is_empty());
}

#[test]
fn pf5_holder_ids_must_be_16_lower_case_hex_characters() {
    let id = phone_of(15);
    for evil in [
        "ab\u{202e}dddddddddddd",
        "DDDDDDDDDDDDDDDD",
        "dddd",
        "dddddddddddddddd0",
    ] {
        let sc = scenario(&id, EPOCH, false, false, evil);
        let mut fx = Fx::new(15, &sc.ring);
        let (opened, pending) = open_reshare(sc.signed.clone(), &mut fx.ctx(), 1);
        assert!(pending.is_none(), "{evil}");
        assert!(refusal(opened).reason.contains("holder id"), "{evil}");
    }
    // a holder named twice would shift every index: refused
    let sc = scenario(&id, EPOCH, false, false, id.device_id());
    let mut fx = Fx::new(15, &sc.ring);
    assert!(open_reshare(sc.signed, &mut fx.ctx(), 1).1.is_none());
}

#[test]
fn pf4_the_epoch_is_a_u64_all_the_way_to_the_op_the_phone_signs() {
    let id = phone_of(14);
    for epoch in [0, u64::MAX] {
        let sc = scenario(&id, epoch, false, false, "dddddddddddddddd");
        let mut fx = Fx::new(14, &sc.ring);
        let (opened, pending) = open_reshare(sc.signed.clone(), &mut fx.ctx(), 1);
        assert_eq!(reshare_view(opened).epoch, epoch);
        let out = publish_reshare(pending.unwrap(), &mut fx.ctx()).unwrap();
        assert!(ops_of(&out).iter().all(|o| match &o.body {
            OpBody::ShareStorageKey { epoch: e, .. }
            | OpBody::ShareReshareConfirmBatch { epoch: e, .. } => *e == epoch,
            _ => false,
        }));
    }
}

#[test]
fn f5c_malformed_ops_end_on_a_refusal_never_a_panic() {
    let id = phone_of(0x42);
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let sc = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let cases: Vec<(&str, Vec<SignedOpBytes>)> = vec![
        ("empty list", vec![]),
        ("not json", vec![(b"{\"a\":\"".to_vec(), [0; 64])]),
        ("not an op", vec![(b"{}".to_vec(), [0; 64])]),
        (
            "noncanonical copy",
            vec![(
                format!("{} ", String::from_utf8(sc.signed[1].0.clone()).unwrap()).into_bytes(),
                sc.signed[1].1,
            )],
        ),
        ("bytes", vec![(vec![0xff, 0xfe], [0; 64])]),
        ("only a session", vec![sc.signed[0].clone()]),
    ];
    for (name, ops) in cases {
        let (opened, pending) = open_reshare(ops, &mut fx.ctx(), 1);
        assert!(pending.is_none(), "{name}");
        assert!(!refusal(opened).reason.is_empty(), "{name}");
    }
    // a head attest and other kinds next to the batches are ignored, not a reason to refuse
    let a = dev(1);
    let mut mixed = sc.signed.clone();
    mixed.push(op(
        &a,
        99,
        OpBody::HeadAttest {
            device_id: a.public().device_id(),
            seq: 1,
            chain_hash: [0; 32],
        },
    ));
    let mut fx = Fx::new(0x42, &sc.ring);
    assert!(open_reshare(mixed, &mut fx.ctx(), 1).1.is_some());
    // random noise never panics
    let mut noise = ChaCha8Rng::seed_from_u64(4);
    for len in [0usize, 3, 50, 700] {
        let mut b = vec![0u8; len];
        noise.fill_bytes(&mut b);
        let (_, p) = open_reshare(vec![(b, [7; 64])], &mut fx.ctx(), 1);
        assert!(p.is_none());
    }
}

#[test]
fn counters_are_adopted_only_from_ops_whose_signature_verified() {
    let id = phone_of(0x42);
    let sc = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let x = dev(9);
    let mut signed = sc.signed.clone();
    // a stranger's op with a huge lamport and epoch must not push this phone's chain out of range
    signed.push(op(
        &x,
        9_000_000_000,
        OpBody::ShareReshareSession {
            epoch: EPOCH,
            to: "eeeeeeeeeeeeeeee".into(),
            sealed: vec![1],
        },
    ));
    let mut fx = Fx::new(0x42, &sc.ring);
    let (_, pending) = open_reshare(signed, &mut fx.ctx(), 1);
    let p = pending.unwrap();
    assert_eq!(
        p.seen.lamport, 13,
        "the four verified ops carry lamports 10 to 13"
    );
    publish_reshare(p, &mut fx.ctx()).unwrap();
    assert!(fx.state.lamport < 100, "{}", fx.state.lamport);
}

#[test]
fn a_delivery_that_needs_more_than_8_ops_is_refused_with_the_reason_and_writes_nothing() {
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let items: Vec<ConfirmItem> = (0..1500)
        .map(|i| ConfirmItem {
            host_uid: [i as u8; 16],
            sealed_share: vec![1; 73],
        })
        .collect();
    let p = PendingReshare {
        epoch: EPOCH,
        items,
        storage_sealed: Some(vec![1; 10]),
        seen: Seen::default(),
    };
    let e = publish_reshare(p, &mut fx.ctx()).unwrap_err();
    assert!(e.reason().contains("carries 8"), "{}", e.reason());
    assert!(fx.state.chain.is_empty());
}

#[test]
fn credentials_are_confirmed_in_batches_of_200() {
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let items: Vec<ConfirmItem> = (0..401)
        .map(|i| ConfirmItem {
            host_uid: [(i % 251) as u8; 16],
            sealed_share: vec![1; 73],
        })
        .collect();
    let p = PendingReshare {
        epoch: EPOCH,
        items,
        storage_sealed: Some(vec![1; 10]),
        seen: Seen {
            lamport: 40,
            epoch: 2,
        },
    };
    let out = publish_reshare(p, &mut fx.ctx()).unwrap();
    let ops = ops_of(&out);
    assert_eq!(ops.len(), 4);
    let sizes: Vec<usize> = ops
        .iter()
        .filter_map(|o| match &o.body {
            OpBody::ShareReshareConfirmBatch { items, .. } => Some(items.len()),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [200, 200, 1]);
    assert_eq!(ops[0].lamport, 41, "adopted from the verified counters");
    assert_eq!(fx.state.epoch, 2);
    // exactly 8 ops is still one delivery (1 key + 7 batches = 1400 items)
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let items: Vec<ConfirmItem> = (0..1400)
        .map(|i| ConfirmItem {
            host_uid: [(i % 251) as u8; 16],
            sealed_share: vec![1; 73],
        })
        .collect();
    let p = PendingReshare {
        epoch: EPOCH,
        items,
        storage_sealed: Some(vec![1; 10]),
        seen: Seen::default(),
    };
    assert_eq!(ops_of(&publish_reshare(p, &mut fx.ctx()).unwrap()).len(), 8);
}

#[test]
fn a_restored_phone_does_not_sign_a_reshare_before_its_head_is_known() {
    let id = phone_of(0x42);
    let sc = scenario(&id, EPOCH, false, false, "dddddddddddddddd");
    let mut fx = Fx::new(0x42, &sc.ring);
    fx.state.head_pending = true;
    let (_, pending) = open_reshare(sc.signed, &mut fx.ctx(), 1);
    let e = publish_reshare(pending.unwrap(), &mut fx.ctx()).unwrap_err();
    assert!(e.reason().contains("must not sign yet"));
    assert!(fx.state.chain.is_empty());
}

// ------------------------------------------------------------ the Rust-made fixture

fn fixture() -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/companion/test/fixtures/reshare.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

fn fixture_ops(v: &serde_json::Value) -> Vec<SignedOpBytes> {
    v["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let canonical = hex::decode(o["canonical_hex"].as_str().unwrap()).unwrap();
            let sig: [u8; 64] = hex::decode(o["sig_hex"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            (canonical, sig)
        })
        .collect()
}

#[test]
fn the_committed_rust_reshare_fixture_is_adopted_and_reproduces_the_committed_phone_shares() {
    let v = fixture();
    let ops = fixture_ops(&v);
    let epoch = v["epoch"].as_u64().unwrap();
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    assert_eq!(
        fx.id.device_id(),
        v["phone_pub"]["device_id"].as_str().unwrap(),
        "the fixture phone is the 0x42 phone"
    );
    assert_eq!(
        hex::encode(fx.id.public().mlkem_pub),
        v["phone_pub"]["mlkem_pub"].as_str().unwrap()
    );
    // the ring is built from the contributors (the terminals the phone enrolled with)
    for c in v["contributors"].as_array().unwrap() {
        let id = c["device_id"].as_str().unwrap();
        fx.state.ring.insert(
            id.to_string(),
            RingDevice {
                name: format!("terminal {}", &id[..4]),
                ed25519_pub: c["ed25519_pub"].as_str().unwrap().to_string(),
                enrolled_at: 0,
            },
        );
    }
    let sorted: Vec<&str> = v["new_holders"]["sorted_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    let my_index = (sorted.iter().position(|h| *h == fx.id.device_id()).unwrap() + 1) as u8;
    let expected = v["expected_phone_shares"].as_object().unwrap();

    let (opened, pending) = open_reshare(ops, &mut fx.ctx(), 5);
    let view = reshare_view(opened);
    assert_eq!(view.epoch, epoch);
    assert_eq!(
        view.holders_after
            .iter()
            .map(|h| h.device_id.as_str())
            .collect::<Vec<_>>(),
        sorted
    );
    assert_eq!(view.my_index, u32::from(my_index));
    assert_eq!(view.credentials_ready as usize, expected.len());
    assert_eq!(view.waiting, 0);
    assert_eq!(view.contributors.len(), 2);
    assert!(view
        .contributors
        .iter()
        .all(|c| c.has_session && c.verified));

    let out = publish_reshare(pending.unwrap(), &mut fx.ctx()).unwrap();
    let key = own_storage_key(&mut fx.id, &fx.state.chain, epoch).unwrap();
    let confirms: Vec<ConfirmItem> = ops_of(&out)
        .into_iter()
        .filter_map(|o| match o.body {
            OpBody::ShareReshareConfirmBatch { items, .. } => Some(items),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(confirms.len(), expected.len());
    for item in confirms {
        let want = &expected[&hex::encode(item.host_uid)];
        let plain = aead_open(
            &key,
            &envelope_aad(&item.host_uid, epoch, my_index),
            &item.sealed_share,
        )
        .unwrap();
        assert_eq!(u64::from(plain[0]), want["index"].as_u64().unwrap());
        assert_eq!(
            hex::encode(&plain[1..]),
            want["bytes_hex"].as_str().unwrap(),
            "the new share of {}",
            hex::encode(item.host_uid)
        );
    }
}

#[test]
fn the_fixture_is_not_adopted_when_the_phone_has_met_nobody() {
    let v = fixture();
    let ops = fixture_ops(&v);
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let (opened, pending) = open_reshare(ops, &mut fx.ctx(), 5);
    assert!(refusal(opened).reason.contains("fewer than 2 contributors"));
    assert!(pending.is_none());
}

// ------------------------------------------------------------ wrap key rotation

#[test]
fn rotating_announces_the_derived_keys_and_the_identity_moves_to_the_new_generation() {
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    let old_pub = fx.id.public();
    let out = rotate_wrap_keys(&mut fx.ctx()).unwrap();
    let ops = ops_of(&out);
    assert_eq!(ops.len(), 1);
    assert!(
        ops[0].verify(&old_pub.ed25519_pub),
        "signed by the signing key, which never rotates"
    );
    let now = fx.id.public();
    assert_eq!(fx.id.wrap_epoch(), 1);
    assert_eq!(fx.state.wrap_epoch, 1);
    match &ops[0].body {
        OpBody::WrapKeyRotate {
            device_id,
            wrap_epoch,
            x25519_pub,
            mlkem_pub,
        } => {
            assert_eq!(device_id, fx.id.device_id());
            assert_eq!(*wrap_epoch, 1);
            assert_eq!(*x25519_pub, now.x25519_pub);
            assert_eq!(*mlkem_pub, now.mlkem_pub);
            assert_ne!(*x25519_pub, old_pub.x25519_pub);
        }
        other => panic!("not a rotation: {other:?}"),
    }
    assert_eq!(now.ed25519_pub, old_pub.ed25519_pub);
    assert_eq!(*fx.id.secret(), [0x42; 32]);
    // the identity afterwards is the one at generation 1, and it still opens what was wrapped to generation 0
    assert_eq!(
        fx.id.public(),
        PhoneIdentity::new(&[0x42; 32], 1).unwrap().public()
    );
    assert_eq!(
        fx.id.unwrap(&hybrid_wrap(&old_pub, b"old")).unwrap(),
        b"old"
    );
    assert_eq!(fx.id.unwrap(&hybrid_wrap(&now, b"new")).unwrap(), b"new");
    // again: generation 2
    rotate_wrap_keys(&mut fx.ctx()).unwrap();
    assert_eq!((fx.id.wrap_epoch(), fx.state.wrap_epoch), (2, 2));
}

#[test]
fn pf2_a_rotation_never_starts_below_a_generation_the_ring_already_uses() {
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    // a blob wrapped to generation 2 opens on this generation 0 identity and remembers it
    let g2 = PhoneIdentity::new(&[0x42; 32], 2).unwrap();
    assert_eq!(
        fx.id.unwrap(&hybrid_wrap(&g2.public(), b"fresh")).unwrap(),
        b"fresh"
    );
    assert_eq!(fx.id.seen_wrap_epoch(), 2);
    let out = rotate_wrap_keys(&mut fx.ctx()).unwrap();
    assert!(matches!(
        &ops_of(&out)[0].body,
        OpBody::WrapKeyRotate { wrap_epoch: 3, .. }
    ));
    assert_eq!((fx.id.wrap_epoch(), fx.state.wrap_epoch), (3, 3));
    assert!(fx.id.seen_wrap_epoch() >= 2);
    // the state's own counter (from an adopted chain) counts too
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    fx.state.wrap_epoch = 5;
    rotate_wrap_keys(&mut fx.ctx()).unwrap();
    assert_eq!(fx.id.wrap_epoch(), 6);
}

#[test]
fn rotation_stops_at_the_bound_and_a_restored_phone_does_not_rotate() {
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    fx.state.wrap_epoch = WRAP_GEN_MAX;
    assert!(rotate_wrap_keys(&mut fx.ctx()).is_err());
    assert!(fx.state.chain.is_empty());
    let mut fx = Fx::new(0x42, &BTreeMap::new());
    fx.state.head_pending = true;
    assert!(rotate_wrap_keys(&mut fx.ctx())
        .unwrap_err()
        .reason()
        .contains("must not sign yet"));
    assert_eq!(
        fx.id.wrap_epoch(),
        0,
        "the identity stays at its generation when nothing was announced"
    );
}
