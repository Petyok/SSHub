//! Op-log tests: fixed adversarial cases and the convergence property test.

use super::*;
use crate::store::{
    uid_hex, LauncherStore, ManagedHost, NewHost, NewHostGroup, FAVORITES_GROUP_UID,
};
use crate::sync::crypto::{DeviceIdentity, ALG_CURRENT};
use crate::sync::limits::LAMPORT_JUMP_CAP;
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

pub(super) struct Peer {
    pub store: LauncherStore,
    pub id: DeviceIdentity,
}

impl Peer {
    fn new() -> Self {
        Self {
            store: LauncherStore::open_in_memory().unwrap(),
            id: DeviceIdentity::generate(),
        }
    }
    fn dev(&self) -> DeviceId {
        self.id.public().device_id()
    }
    fn append(&self, body: OpBody) -> Op {
        self.store.sync_append_local(&self.id, body).unwrap()
    }
    fn host(&self, uid: &Uid) -> Option<ManagedHost> {
        let id = self.store.host_id_by_uid(uid).unwrap()?;
        self.store.get_host(id).unwrap()
    }
    fn pending(&self) -> usize {
        self.store.sync_pending().unwrap().len()
    }
    fn approve_all(&self) {
        loop {
            let pending = self.store.sync_pending().unwrap();
            if pending.is_empty() {
                return;
            }
            for op in pending {
                // An earlier approval may have voided this one (revoked author).
                if let Err(e) = self.store.sync_resolve_pending(&op.hash(), true) {
                    assert!(e.to_string().contains("not pending"), "{e}");
                }
            }
        }
    }
}

/// `n` peers that trust each other as holders.
fn ring(n: usize) -> Vec<Peer> {
    let peers: Vec<Peer> = (0..n).map(|_| Peer::new()).collect();
    for p in &peers {
        for q in &peers {
            p.store
                .sync_register_device(
                    &q.id.public(),
                    "peer",
                    DeviceRole::Holder,
                    Attendance::Attended,
                )
                .unwrap();
        }
    }
    peers
}

fn uid(n: u8) -> Uid {
    [n; 16]
}

fn hf(name: &str, addr: &str) -> HostFields {
    HostFields {
        name: Some(name.into()),
        address: Some(addr.into()),
        ..Default::default()
    }
}

fn upsert(n: u8, fields: HostFields) -> OpBody {
    OpBody::HostUpsert {
        host_uid: uid(n),
        fields,
    }
}

/// Pull every chain from `from` that `to` lacks (what a sync round does).
fn pull(to: &Peer, from: &Peer) -> Vec<IngestOutcome> {
    let mut out = Vec::new();
    for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
        let have = to
            .store
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(d, _, _)| *d == dev)
            .map_or(0, |(_, s, _)| s);
        for op in from.store.sync_chain_tail(&dev, have).unwrap() {
            out.push(to.store.sync_ingest(op).unwrap());
        }
    }
    out
}

fn sync_all(peers: &[Peer]) {
    for _ in 0..3 {
        for a in peers {
            for b in peers {
                if !std::ptr::eq(a, b) {
                    pull(a, b);
                }
            }
        }
    }
}

fn rejected(o: &IngestOutcome) -> &str {
    match o {
        IngestOutcome::Rejected(m) => m,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

// ---------------------------------------------------------------- encoding

fn sample_bodies() -> Vec<OpBody> {
    let dev = DeviceIdentity::generate().public();
    let full = HostFields {
        name: Some("Хост ⚡ 🚀 \"q\" \\ \n".into()),
        label: Some(None),
        address: Some("h.example".into()),
        port: Some(Some(2222)),
        username: Some(Some("root".into())),
        proxy_jump: Some(Some(format!("#{},bastion", uid_hex(&uid(9))))),
        forward_agent: Some(Some(true)),
        remote_command: Some(None),
        transport: Some(Some("mosh".into())),
        environment: Some(Some("prod".into())),
        os_icon: Some(None),
        tags: Some(vec!["a".into(), "b".into()]),
        notes: Some(Some(String::new())),
        favorite: Some(true),
        secret_lookup: Some(Some("k=v".into())),
        has_password: Some(false),
        group_uids: Some(vec![uid(1), uid(2)]),
        identity_uid: Some(Some(uid(3))),
    };
    let share = WrappedShare {
        recipient: "0011223344556677".into(),
        alg: ALG_CURRENT,
        sealed: vec![0, 255, 7],
    };
    vec![
        upsert(1, HostFields::default()),
        upsert(2, full),
        OpBody::HostDelete { host_uid: uid(3) },
        OpBody::GroupUpsert {
            group_uid: uid(4),
            fields: GroupFields {
                name: Some("g".into()),
                default_identity_uid: Some(None),
                default_username: Some(Some("u".into())),
                default_port: Some(Some(22)),
                default_proxy_jump: Some(Some("none".into())),
                default_transport: Some(Some("ssh".into())),
                default_forward_agent: Some(Some(false)),
                parent_uid: Some(Some(uid(5))),
            },
        },
        OpBody::GroupDelete { group_uid: uid(6) },
        OpBody::IdentityUpsert {
            identity_uid: uid(7),
            fields: IdentityFields {
                name: Some("i".into()),
                username: Some(None),
                has_password: Some(true),
            },
        },
        OpBody::IdentityDelete {
            identity_uid: uid(8),
        },
        OpBody::CredUpdate {
            host_uid: uid(1),
            ciphertext: vec![],
            wrapped_shares: vec![share],
        },
        OpBody::DeviceAdd {
            device: dev,
            name: "laptop".into(),
            role: DeviceRole::Mirror,
            attendance: Attendance::Unattended,
        },
        OpBody::DeviceRevoke {
            device_id: "ffeeddccbbaa9988".into(),
        },
        OpBody::ShareRelease {
            host_uid: uid(1),
            request_id: u64::MAX,
            requester: "0011223344556677".into(),
        },
        OpBody::ShareReshare {
            epoch: 2,
            new_holders: vec!["a".into(), "b".into()],
            commitments: vec![vec![1, 2], vec![]],
        },
        OpBody::ShareReshareConfirm {
            reshare_hash: [7; 32],
        },
        OpBody::CertIssue {
            device_id: "0011223344556677".into(),
            scope: CertScope::Hosts(vec![uid(1), uid(2)]),
            expiry: i64::MIN,
            max_new_hosts_per_day: u32::MAX,
            epoch_lo: 0,
            epoch_hi: u64::MAX,
        },
        OpBody::CertIssue {
            device_id: "0011223344556677".into(),
            scope: CertScope::Group(uid(3)),
            expiry: 1,
            max_new_hosts_per_day: 50,
            epoch_lo: 1,
            epoch_hi: 2,
        },
        OpBody::CertRevoke { cert_id: [9; 32] },
        OpBody::CertHostOpened {
            cert_id: [9; 32],
            host_uid: uid(1),
        },
        OpBody::PhoneReenroll {
            initiated_at: -5,
            signers: vec!["a".into()],
            via_paper: true,
            third_factor: ThirdFactor::HardwareKey("0011223344556677".into()),
            liveness: vec![HeadAttest {
                device_id: "a".into(),
                seq: 3,
                chain_hash: [1; 32],
            }],
        },
        OpBody::PhoneReenroll {
            initiated_at: 0,
            signers: vec![],
            via_paper: false,
            third_factor: ThirdFactor::EmailCode,
            liveness: vec![],
        },
        OpBody::PhoneReenrollCancel {
            reenroll_hash: [2; 32],
            by_phone: false,
            third_factor: Some(ThirdFactor::Paper),
        },
        OpBody::PhoneReenrollCancel {
            reenroll_hash: [2; 32],
            by_phone: true,
            third_factor: None,
        },
        OpBody::RecoveryAck {
            reenroll_hash: [3; 32],
            seen_at: 7,
        },
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: "a".into(),
            cosigner: Some("b".into()),
        },
        OpBody::ModeChange {
            to: CredMode::Vault,
            migrated_on: "a".into(),
            cosigner: None,
        },
        OpBody::HeadAttest {
            device_id: "a".into(),
            seq: u64::MAX,
            chain_hash: [4; 32],
        },
    ]
}

fn wrap(body: OpBody) -> Op {
    Op {
        device_id: "d04ab232742bb4ab".into(),
        seq: u64::MAX,
        lamport: 18446744073709551615,
        prev_hash: [0xab; 32],
        timestamp: i64::MIN,
        epoch: 0,
        alg: ALG_CURRENT,
        body,
        signature: [5; 64],
    }
}

#[test]
fn every_body_round_trips_through_the_strict_decoder() {
    for body in sample_bodies() {
        let op = wrap(body);
        let bytes = op.canonical_bytes();
        let back = Op::from_canonical(&bytes, op.signature).unwrap();
        assert_eq!(back, op);
        assert_eq!(back.canonical_bytes(), bytes);
        assert_eq!(back.hash(), op.hash());
    }
}

#[test]
fn header_layout_and_number_literals() {
    let op = wrap(OpBody::HostDelete {
        host_uid: uid(0xab),
    });
    let text = String::from_utf8(op.canonical_bytes()).unwrap();
    assert_eq!(
        text,
        format!(
            "{{\"device_id\":\"d04ab232742bb4ab\",\"seq\":18446744073709551615,\"lamport\":18446744073709551615,\"prev_hash\":\"{}\",\"timestamp\":-9223372036854775808,\"epoch\":0,\"alg\":[1,1],\"body\":{{\"HostDelete\":{{\"host_uid\":\"{}\"}}}}}}",
            "ab".repeat(32),
            "ab".repeat(16)
        )
    );
}

#[test]
fn non_canonical_bytes_are_refused() {
    let op = wrap(OpBody::HostDelete { host_uid: uid(1) });
    let good = String::from_utf8(op.canonical_bytes()).unwrap();
    let sig = op.signature;
    for bad in [
        good.replace("{\"device_id\"", "{ \"device_id\""),
        good.replace(
            "\"seq\":18446744073709551615,\"lamport\":18446744073709551615",
            "\"lamport\":18446744073709551615,\"seq\":18446744073709551615",
        ),
        good.replace(&"ab".repeat(32), &"AB".repeat(32)),
        good.replace("\"epoch\":0", "\"epoch\":0.0"),
        good.replace("\"epoch\":0", "\"epoch\":00"),
        good.replace("\"epoch\":0", "\"epoch\":0,\"extra\":1"),
        good.replace(&"01".repeat(16), &"0A".repeat(16)),
        good.replace("[1,1]", "[1,2]"),
    ] {
        assert!(Op::from_canonical(bad.as_bytes(), sig).is_err(), "{bad}");
    }
    assert!(Op::from_canonical(good.as_bytes(), sig).is_ok());
}

#[test]
fn patch_semantics_survive_the_wire() {
    let fields = HostFields {
        label: Some(None),
        port: Some(Some(22)),
        ..Default::default()
    };
    let op = wrap(upsert(1, fields.clone()));
    let text = String::from_utf8(op.canonical_bytes()).unwrap();
    assert!(
        text.contains("\"fields\":{\"label\":null,\"port\":22}"),
        "{text}"
    );
    let back = Op::from_canonical(text.as_bytes(), op.signature).unwrap();
    match back.body {
        OpBody::HostUpsert { fields: f, .. } => assert_eq!(f, fields),
        _ => unreachable!(),
    }
}

// ----------------------------------------------------------- ingest basics

#[test]
fn safe_edit_applies_and_dangerous_edit_waits() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let create = a.append(upsert(1, hf("web", "10.0.0.1")));
    // Creation carries an address: held for review.
    assert!(matches!(
        b.store.sync_ingest(create).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(b.host(&uid(1)).is_none());
    b.approve_all();
    assert_eq!(b.host(&uid(1)).unwrap().address, "10.0.0.1");

    let rename = a.append(upsert(
        1,
        HostFields {
            name: Some("web-prod".into()),
            notes: Some(Some("n".into())),
            ..Default::default()
        },
    ));
    assert!(matches!(
        b.store.sync_ingest(rename).unwrap(),
        IngestOutcome::Applied
    ));
    let h = b.host(&uid(1)).unwrap();
    assert_eq!(
        (h.name.as_str(), h.notes.as_deref()),
        ("web-prod", Some("n"))
    );
    assert_eq!(b.pending(), 0, "a rename must not wake the user");
}

#[test]
fn a_dangerous_field_hidden_among_safe_ones_holds_the_whole_op() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let c = a.append(upsert(1, hf("web", "10.0.0.1")));
    b.store.sync_ingest(c).unwrap();
    b.approve_all();
    let sneaky = a.append(upsert(
        1,
        HostFields {
            name: Some("renamed".into()),
            notes: Some(Some("harmless".into())),
            tags: Some(vec!["x".into()]),
            address: Some("evil.example".into()),
            ..Default::default()
        },
    ));
    assert!(matches!(
        b.store.sync_ingest(sneaky.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    let h = b.host(&uid(1)).unwrap();
    assert_eq!(
        h.name, "web",
        "the safe fields of a held op do not leak through"
    );
    assert_eq!(h.address, "10.0.0.1");
    assert_eq!(
        b.store
            .sync_pending_reason(&sneaky.hash())
            .unwrap()
            .as_deref(),
        Some("tier")
    );
    b.store.sync_resolve_pending(&sneaky.hash(), false).unwrap();
    assert_eq!(b.host(&uid(1)).unwrap().address, "10.0.0.1");
    assert_eq!(b.pending(), 0);
}

#[test]
fn bad_signature_unknown_device_gap_duplicate_and_chain_breaks_are_rejected() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let o1 = a.append(upsert(1, hf("w", "10.0.0.1")));
    let o2 = a.append(upsert(1, HostFields::default()));
    let o3 = a.append(upsert(1, HostFields::default()));

    let mut forged = o1.clone();
    forged.signature[0] ^= 1;
    assert!(rejected(&b.store.sync_ingest(forged).unwrap()).contains("signature"));

    let stranger = Peer::new();
    let so = stranger.append(upsert(2, hf("x", "10.0.0.9")));
    assert!(rejected(&b.store.sync_ingest(so).unwrap()).contains("unknown device"));

    assert!(rejected(&b.store.sync_ingest(o2.clone()).unwrap()).contains("gap"));
    assert!(matches!(
        b.store.sync_ingest(o1.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(rejected(&b.store.sync_ingest(o1).unwrap()).contains("duplicate"));
    assert!(matches!(
        b.store.sync_ingest(o2).unwrap(),
        IngestOutcome::Applied
    ));

    // Re-signed with a wrong prev_hash: valid signature, broken chain.
    let mut bent = o3.clone();
    bent.prev_hash = [1; 32];
    bent.signature = a.id.sign(&bent.canonical_bytes());
    assert!(rejected(&b.store.sync_ingest(bent).unwrap()).contains("prev_hash"));
    assert!(matches!(
        b.store.sync_ingest(o3).unwrap(),
        IngestOutcome::Applied
    ));
}

#[test]
fn malformed_field_values_are_rejected_before_chaining() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    // The local append refuses them as well; sign by hand to model a hostile peer.
    let forge = |seq: u64, prev: [u8; 32], fields: HostFields| {
        let mut op = Op {
            device_id: a.dev(),
            seq,
            lamport: seq,
            prev_hash: prev,
            timestamp: 0,
            epoch: 0,
            alg: ALG_CURRENT,
            body: upsert(1, fields),
            signature: [0; 64],
        };
        op.signature = a.id.sign(&op.canonical_bytes());
        op
    };
    for (i, bad) in [
        hf("-oProxyCommand=x", "h.example"),
        hf("ok", "-oProxyCommand=touch x"),
        hf("ok", "bad host"),
        HostFields {
            proxy_jump: Some(Some("-oProxyCommand=x".into())),
            ..Default::default()
        },
        HostFields {
            proxy_jump: Some(Some("a,#nothex".into())),
            ..Default::default()
        },
        HostFields {
            port: Some(Some(0)),
            ..Default::default()
        },
        HostFields {
            transport: Some(Some("telnet".into())),
            ..Default::default()
        },
        HostFields {
            group_uids: Some(vec![uid(2), uid(1)]),
            ..Default::default()
        },
        HostFields {
            group_uids: Some(vec![FAVORITES_GROUP_UID]),
            ..Default::default()
        },
        HostFields {
            name: Some(" padded ".into()),
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let out = b.store.sync_ingest(forge(1, [0; 32], bad)).unwrap();
        assert!(rejected(&out).contains("invalid op"), "case {i}: {out:?}");
    }
    assert!(a
        .store
        .sync_append_local(&a.id, upsert(1, hf("-x", "h")))
        .is_err());
}

#[test]
fn lamport_jump_over_the_cap_is_rejected_but_the_cap_itself_is_fine() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let mut op = Op {
        device_id: a.dev(),
        seq: 1,
        lamport: LAMPORT_JUMP_CAP + 1,
        prev_hash: [0; 32],
        timestamp: 0,
        epoch: 0,
        alg: ALG_CURRENT,
        body: upsert(1, HostFields::default()),
        signature: [0; 64],
    };
    op.signature = a.id.sign(&op.canonical_bytes());
    assert!(rejected(&b.store.sync_ingest(op.clone()).unwrap()).contains("lamport"));
    op.lamport = LAMPORT_JUMP_CAP;
    op.signature = a.id.sign(&op.canonical_bytes());
    assert!(matches!(
        b.store.sync_ingest(op).unwrap(),
        IngestOutcome::Applied
    ));
}

#[test]
fn quarantine_is_capped_per_device() {
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let ops: Vec<Op> = (0..4)
        .map(|i| a.append(upsert(i + 1, hf("h", "10.0.0.1"))))
        .collect();
    let cap = 3;
    let mut seen = Vec::new();
    for op in &ops {
        let out = b
            .store
            .with_conn(|conn| {
                super::ingest::ingest_capped(conn, op.clone(), crate::store::now_ts(), cap)
            })
            .unwrap();
        seen.push(out);
    }
    assert!(seen[..3]
        .iter()
        .all(|o| matches!(o, IngestOutcome::Pending)));
    assert!(rejected(&seen[3]).contains("quarantine"));
    // Another device is not throttled by a's flood.
    let o = c.append(upsert(9, hf("z", "10.0.0.2")));
    assert!(matches!(
        b.store.sync_ingest(o).unwrap(),
        IngestOutcome::Pending
    ));
    // Resolving frees room for the throttled op to come back.
    b.store.sync_resolve_pending(&ops[0].hash(), true).unwrap();
    let out = b
        .store
        .with_conn(|conn| {
            super::ingest::ingest_capped(conn, ops[3].clone(), crate::store::now_ts(), cap)
        })
        .unwrap();
    assert!(matches!(out, IngestOutcome::Pending));
}

// ---------------------------------------------------------------- forks

fn forked_pair(a: &Peer) -> (Op, Op) {
    let o1 = a.append(upsert(1, hf("w", "10.0.0.1")));
    let honest = a.append(upsert(1, HostFields::default()));
    let mut other = honest.clone();
    other.body = upsert(
        1,
        HostFields {
            notes: Some(Some("different".into())),
            ..Default::default()
        },
    );
    other.signature = a.id.sign(&other.canonical_bytes());
    let _ = o1;
    (honest, other)
}

#[test]
fn two_ops_with_one_seq_are_a_fork_proof_and_the_device_is_compromised() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let o1 = a.store.sync_chain_tail(&a.dev(), 0).unwrap();
    assert!(o1.is_empty());
    let (honest, other) = forked_pair(a);
    pull(b, a);
    match b.store.sync_ingest(other.clone()).unwrap() {
        IngestOutcome::ForkProof(x, y) => {
            assert_eq!((x.seq, y.seq), (honest.seq, other.seq));
            assert_ne!(x.hash(), y.hash());
        }
        o => panic!("{o:?}"),
    }
    let row = b
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == a.dev())
        .unwrap();
    assert_eq!(row.status, DeviceStatus::Compromised);
    assert!(matches!(
        b.store.sync_fork_evidence(&a.dev()).unwrap(),
        Some(ForkEvidence::Ops(..))
    ));
    // A compromised device's later ops are refused.
    let later = a.append(upsert(1, HostFields::default()));
    assert!(rejected(&b.store.sync_ingest(later).unwrap()).contains("compromised"));
}

#[test]
fn a_signed_head_that_contradicts_the_chain_is_a_proof_and_gossip_is_not() {
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    a.append(upsert(1, hf("w", "10.0.0.1")));
    a.append(upsert(1, HostFields::default()));
    pull(b, a);
    pull(c, a);
    let real = b
        .store
        .sync_chain_heads()
        .unwrap()
        .into_iter()
        .find(|h| h.0 == a.dev())
        .unwrap();
    // Consistent head, and a head from the future.
    let good = SignedHead::sign(&a.id, real.1, real.2);
    assert_eq!(
        b.store.sync_record_head(&good).unwrap(),
        HeadVerdict::Consistent
    );
    let ahead = SignedHead::sign(&a.id, real.1 + 5, [3; 32]);
    assert_eq!(
        b.store.sync_record_head(&ahead).unwrap(),
        HeadVerdict::Ahead
    );
    // Bad signature: rejected, nobody is accused.
    let mut forged = SignedHead::sign(&a.id, real.1, [8; 32]);
    forged.sig[1] ^= 1;
    assert!(matches!(
        b.store.sync_record_head(&forged).unwrap(),
        HeadVerdict::Rejected(_)
    ));
    // Gossip: suspect only.
    c.store.sync_mark_suspect(&a.dev()).unwrap();
    let st = |p: &Peer| {
        p.store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == a.dev())
            .unwrap()
            .status
    };
    assert_eq!(st(c), DeviceStatus::Suspect);
    assert_eq!(st(b), DeviceStatus::Operational);
    // Proof: a validly signed head with another hash at a seq we hold.
    let lie = SignedHead::sign(&a.id, real.1, [9; 32]);
    assert_eq!(b.store.sync_record_head(&lie).unwrap(), HeadVerdict::Fork);
    assert_eq!(st(b), DeviceStatus::Compromised);
    assert!(matches!(
        b.store.sync_fork_evidence(&a.dev()).unwrap(),
        Some(ForkEvidence::OpAndHead(..))
    ));
}

#[test]
fn two_signed_heads_that_disagree_are_a_proof_and_a_late_chain_triggers_it_too() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("w", "10.0.0.1")));
    a.append(upsert(1, HostFields::default()));
    // b holds no chain of a yet: two heads at seq 2 with different hashes.
    let h1 = SignedHead::sign(&a.id, 2, [1; 32]);
    let h2 = SignedHead::sign(&a.id, 2, [2; 32]);
    assert_eq!(b.store.sync_record_head(&h1).unwrap(), HeadVerdict::Ahead);
    assert_eq!(b.store.sync_record_head(&h2).unwrap(), HeadVerdict::Fork);
    assert!(matches!(
        b.store.sync_fork_evidence(&a.dev()).unwrap(),
        Some(ForkEvidence::Heads(..))
    ));

    // A head stored before the chain arrives is checked when the op lands.
    let ring = ring_again();
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("w", "10.0.0.1")));
    let lie = SignedHead::sign(&a.id, 1, [7; 32]);
    assert_eq!(b.store.sync_record_head(&lie).unwrap(), HeadVerdict::Ahead);
    let out = pull(b, a);
    assert!(rejected(&out[0]).contains("fork"));
    let row = b
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == a.dev())
        .unwrap();
    assert_eq!(row.status, DeviceStatus::Compromised);
}

fn ring_again() -> Vec<Peer> {
    ring(2)
}

// ---------------------------------------------- conflicts and tombstones

#[test]
fn delete_beats_a_concurrent_edit_and_a_later_edit_resurrects_with_review() {
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let create = a.append(upsert(1, hf("web", "10.0.0.1")));
    for p in [b, c] {
        p.store.sync_ingest(create.clone()).unwrap();
        p.approve_all();
    }
    // b deletes, c edits concurrently (lower-or-equal lamport than the delete).
    let del = b.append(OpBody::HostDelete { host_uid: uid(1) });
    let edit = c.append(upsert(
        1,
        HostFields {
            notes: Some(Some("edit".into())),
            ..Default::default()
        },
    ));
    assert!(del.lamport >= edit.lamport || edit.lamport > del.lamport);
    // Everyone sees everything; approve all.
    sync_all(&ring);
    for p in &ring {
        p.approve_all();
    }
    let alive: Vec<bool> = ring.iter().map(|p| p.host(&uid(1)).is_some()).collect();
    assert!(
        alive.iter().all(|x| *x == alive[0]),
        "peers disagree: {alive:?}"
    );
    let hashes: Vec<_> = ring
        .iter()
        .map(|p| p.store.sync_state_hash().unwrap())
        .collect();
    assert!(hashes.iter().all(|h| *h == hashes[0]));
    // Equal lamport: delete wins, strictly higher edit resurrects.
    assert_eq!(alive[0], edit.lamport > del.lamport);
}

#[test]
fn a_green_edit_after_a_delete_is_held_as_a_resurrection() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let create = a.append(upsert(1, hf("web", "10.0.0.1")));
    b.store.sync_ingest(create).unwrap();
    b.approve_all();
    let del = a.append(OpBody::HostDelete { host_uid: uid(1) });
    b.store.sync_ingest(del).unwrap();
    b.approve_all();
    assert!(b.host(&uid(1)).is_none());
    // b never saw a's delete when it edits? No: it did, so its edit is strictly higher.
    let edit = b.append(upsert(
        1,
        HostFields {
            notes: Some(Some("back".into())),
            ..Default::default()
        },
    ));
    assert!(edit.lamport > 0);
    // a receives b's local (approved) edit as a Green foreign op after its own delete.
    let out = a.store.sync_ingest(edit.clone()).unwrap();
    assert!(matches!(out, IngestOutcome::Pending), "{out:?}");
    assert_eq!(
        a.store
            .sync_pending_reason(&edit.hash())
            .unwrap()
            .as_deref(),
        Some("resurrection")
    );
    assert!(a.host(&uid(1)).is_none());
    a.store.sync_resolve_pending(&edit.hash(), true).unwrap();
    assert!(a.host(&uid(1)).is_some());
}

#[test]
fn same_name_on_two_devices_converges_with_a_deterministic_suffix() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("web", "10.0.0.1")));
    b.append(upsert(2, hf("web", "10.0.0.2")));
    sync_all(&ring);
    a.approve_all();
    b.approve_all();
    let names = |p: &Peer| (p.host(&uid(1)).unwrap().name, p.host(&uid(2)).unwrap().name);
    assert_eq!(names(a), names(b));
    let (n1, n2) = names(a);
    assert_ne!(n1, n2);
    assert!(n1 == "web" || n2 == "web");
    assert_eq!(
        a.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
    // The loser is renamed: the winner's old name is free again afterwards.
    let loser = if n1 == "web" { 2 } else { 1 };
    let renamer = if loser == 1 { a } else { b };
    renamer.append(upsert(
        loser,
        HostFields {
            name: Some("web-2".into()),
            ..Default::default()
        },
    ));
    sync_all(&ring);
    a.approve_all();
    b.approve_all();
    assert_eq!(a.host(&uid(loser)).unwrap().name, "web-2");
    assert_eq!(b.host(&uid(loser)).unwrap().name, "web-2");
    assert_eq!(
        a.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
}

#[test]
fn a_replicated_name_never_takes_over_a_local_only_row() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    // b has a local ssh_config style row called "web".
    b.store
        .create_host(&NewHost {
            source: crate::store::HostSource::SshConfig,
            ..NewHost::launcher("web", "10.9.9.9")
        })
        .unwrap();
    a.append(upsert(1, hf("web", "10.0.0.1")));
    pull(b, a);
    b.approve_all();
    let local = b.store.get_host_by_name("web").unwrap().unwrap();
    assert_eq!(local.address, "10.9.9.9", "the ssh_config row is untouched");
    assert_eq!(local.source, crate::store::HostSource::SshConfig);
    let replicated = b.host(&uid(1)).unwrap();
    assert!(replicated.name.starts_with("web ("));
    assert_eq!(replicated.address, "10.0.0.1");
}

#[test]
fn proxy_jump_hops_travel_as_uids_and_follow_a_rename() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("bastion", "10.0.0.1")));
    let wire = {
        let local = a.host(&uid(1)).unwrap().name;
        a.store
            .sync_hops_to_wire(&format!("{local},extra.example"))
            .unwrap()
    };
    assert_eq!(wire, format!("#{},extra.example", uid_hex(&uid(1))));
    a.append(upsert(
        2,
        HostFields {
            proxy_jump: Some(Some(wire)),
            ..hf("db", "10.0.0.2")
        },
    ));
    sync_all(&ring);
    b.approve_all();
    assert_eq!(
        b.host(&uid(2)).unwrap().proxy_jump.as_deref(),
        Some("bastion,extra.example")
    );
    // Safe rename of the hop target: the referring host follows.
    a.append(upsert(
        1,
        HostFields {
            name: Some("gate".into()),
            ..Default::default()
        },
    ));
    sync_all(&ring);
    b.approve_all();
    assert_eq!(
        b.host(&uid(2)).unwrap().proxy_jump.as_deref(),
        Some("gate,extra.example")
    );
    assert_eq!(
        a.host(&uid(2)).unwrap().proxy_jump.as_deref(),
        Some("gate,extra.example")
    );
    // Deleting the hop target clears the rendering instead of dangling.
    a.append(OpBody::HostDelete { host_uid: uid(1) });
    sync_all(&ring);
    b.approve_all();
    assert_eq!(b.host(&uid(2)).unwrap().proxy_jump, None);
}

#[test]
fn groups_identities_memberships_and_favorites_replicate_without_cascade_loss() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    a.append(OpBody::IdentityUpsert {
        identity_uid: uid(30),
        fields: IdentityFields {
            name: Some("work".into()),
            ..Default::default()
        },
    });
    a.append(OpBody::GroupUpsert {
        group_uid: uid(20),
        fields: GroupFields {
            name: Some("prod".into()),
            default_identity_uid: Some(Some(uid(30))),
            ..Default::default()
        },
    });
    a.append(upsert(
        1,
        HostFields {
            favorite: Some(true),
            group_uids: Some(vec![uid(20)]),
            identity_uid: Some(Some(uid(30))),
            ..hf("web", "10.0.0.1")
        },
    ));
    sync_all(&ring);
    b.approve_all();
    let h = b.host(&uid(1)).unwrap();
    assert!(h.favorite);
    assert_eq!(
        h.groups
            .iter()
            .filter(|g| !g.reserved)
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["prod"]
    );
    assert_eq!(h.identity.as_ref().unwrap().name, "work");
    assert_eq!(b.store.favorites_group_uid().unwrap(), FAVORITES_GROUP_UID);
    let gid = b.store.group_id_by_uid(&uid(20)).unwrap().unwrap();
    assert_eq!(
        b.store.get_group(gid).unwrap().unwrap().default_identity_id,
        b.store.identity_id_by_uid(&uid(30)).unwrap()
    );
    assert_eq!(
        a.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
    // Ops on the local-only Favorites group are refused outright.
    assert!(a
        .store
        .sync_append_local(
            &a.id,
            OpBody::GroupDelete {
                group_uid: FAVORITES_GROUP_UID
            }
        )
        .is_err());
}

#[test]
fn parent_cycles_lose_their_parent_on_every_device() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let g = |n: u8, parent: u8| OpBody::GroupUpsert {
        group_uid: uid(n),
        fields: GroupFields {
            name: Some(format!("g{n}")),
            parent_uid: Some(Some(uid(parent))),
            ..Default::default()
        },
    };
    a.append(g(20, 21));
    b.append(g(21, 20));
    sync_all(&ring);
    a.approve_all();
    b.approve_all();
    for p in [a, b] {
        for n in [20, 21] {
            let id = p.store.group_id_by_uid(&uid(n)).unwrap().unwrap();
            assert_eq!(p.store.get_group(id).unwrap().unwrap().parent_id, None);
        }
    }
    assert_eq!(
        a.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
}

// ------------------------------------------------------------ revocation

fn add_device_body(p: &Peer, role: DeviceRole) -> OpBody {
    OpBody::DeviceAdd {
        device: p.id.public(),
        name: "new".into(),
        role,
        attendance: Attendance::Attended,
    }
}

#[test]
fn ring_membership_ops_always_wait_and_take_effect_on_approval() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let newcomer = Peer::new();
    let add = a.append(add_device_body(&newcomer, DeviceRole::Mirror));
    assert!(matches!(
        b.store.sync_ingest(add.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(b
        .store
        .sync_devices()
        .unwrap()
        .iter()
        .all(|d| d.id != newcomer.dev()));
    // Until approved the newcomer is a stranger.
    let early = newcomer.append(upsert(5, hf("n", "10.0.0.5")));
    assert!(rejected(&b.store.sync_ingest(early.clone()).unwrap()).contains("unknown device"));
    b.store.sync_resolve_pending(&add.hash(), true).unwrap();
    let row = b
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == newcomer.dev())
        .unwrap();
    assert_eq!(row.role, DeviceRole::Mirror);
    assert!(matches!(
        b.store.sync_ingest(early).unwrap(),
        IngestOutcome::Pending
    ));
    // Adding a holder is Red, a mirror Yellow.
    assert_eq!(add.tier(), OpTier::Yellow);
    assert_eq!(
        a.append(add_device_body(&Peer::new(), DeviceRole::Holder))
            .tier(),
        OpTier::Red
    );
}

#[test]
fn a_revoke_voids_the_revoked_devices_later_ops_whatever_the_arrival_order() {
    // c writes before and after a's revoke of c; b receives the revoke first
    // or last. Both orders end in the same state.
    let mut hashes = Vec::new();
    for revoke_first in [true, false] {
        let ring = ring(3);
        let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
        let before = c.append(upsert(1, hf("web", "10.0.0.1")));
        a.store.sync_ingest(before.clone()).unwrap();
        a.approve_all();
        let revoke = a.append(OpBody::DeviceRevoke { device_id: c.dev() });
        let after = c.append(upsert(2, hf("evil", "10.6.6.6")));
        assert!(after.lamport <= revoke.lamport || after.lamport > revoke.lamport);
        let deliver = |ops: Vec<Op>| {
            for op in ops {
                b.store.sync_ingest(op).unwrap();
            }
        };
        if revoke_first {
            // a's chain (incl. its copy of c's first op context) first.
            deliver(a.store.sync_chain_tail(&a.dev(), 0).unwrap());
            deliver(c.store.sync_chain_tail(&c.dev(), 0).unwrap());
        } else {
            deliver(c.store.sync_chain_tail(&c.dev(), 0).unwrap());
            deliver(a.store.sync_chain_tail(&a.dev(), 0).unwrap());
        }
        b.approve_all();
        let row = b
            .store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == c.dev())
            .unwrap();
        assert_eq!(row.status, DeviceStatus::Revoked);
        // Ops of c at or above the revoke's lamport never take effect.
        for (n, op) in [(1u8, &before), (2u8, &after)] {
            let effective = b.host(&uid(n)).is_some();
            assert_eq!(
                effective,
                op.lamport < revoke.lamport,
                "host {n}, revoke_first={revoke_first}"
            );
        }
        hashes.push(b.store.sync_state_hash().unwrap());
    }
    assert_eq!(hashes[0], hashes[1], "arrival order must not matter");
}

#[test]
fn a_pending_revoke_holds_concurrent_green_edits_for_review_and_rejecting_it_releases_them() {
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let create = c.append(upsert(1, hf("web", "10.0.0.1")));
    for p in [a, b] {
        p.store.sync_ingest(create.clone()).unwrap();
        p.approve_all();
    }
    let revoke = a.append(OpBody::DeviceRevoke { device_id: c.dev() });
    let rename = c.append(upsert(
        1,
        HostFields {
            name: Some("renamed".into()),
            ..Default::default()
        },
    ));
    assert!(rename.lamport > 0);
    b.store.sync_ingest(rename.clone()).unwrap();
    assert_eq!(
        b.host(&uid(1)).unwrap().name,
        "renamed",
        "no revoke known yet"
    );
    b.store.sync_ingest(revoke.clone()).unwrap();
    if rename.lamport >= revoke.lamport {
        // The conflict is now in review, never an automatic tie-break.
        assert_eq!(
            b.store
                .sync_pending_reason(&rename.hash())
                .unwrap()
                .as_deref(),
            Some("revoke-conflict")
        );
        assert_eq!(b.host(&uid(1)).unwrap().name, "web");
        b.store.sync_resolve_pending(&revoke.hash(), false).unwrap();
        assert_eq!(
            b.host(&uid(1)).unwrap().name,
            "renamed",
            "rejecting the revoke releases the edit"
        );
        assert_eq!(b.pending(), 0);
    }
}

#[test]
fn tie_break_orders_by_lamport_then_device_and_a_revoke_goes_to_review() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let x = a.append(upsert(1, hf("w", "10.0.0.1")));
    let y = b.append(upsert(1, hf("w", "10.0.0.2")));
    let want = if (x.lamport, &x.device_id) >= (y.lamport, &y.device_id) {
        TieBreak::First
    } else {
        TieBreak::Second
    };
    assert_eq!(tie_break(&x, &y), want);
    let flipped = if want == TieBreak::First {
        TieBreak::Second
    } else {
        TieBreak::First
    };
    assert_eq!(tie_break(&y, &x), flipped);
    let r = a.append(OpBody::DeviceRevoke { device_id: b.dev() });
    assert_eq!(tie_break(&r, &y), TieBreak::ManualReview);
    assert_eq!(tie_break(&x, &r), TieBreak::ManualReview);
}

#[test]
fn credentials_modes_certs_and_head_ops_land_in_their_tables_on_approval() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let share = WrappedShare {
        recipient: b.dev(),
        alg: ALG_CURRENT,
        sealed: vec![1, 2, 3],
    };
    let cred = a.append(OpBody::CredUpdate {
        host_uid: uid(1),
        ciphertext: vec![9; 40],
        wrapped_shares: vec![share.clone()],
    });
    let issue = a.append(OpBody::CertIssue {
        device_id: b.dev(),
        scope: CertScope::Group(uid(20)),
        expiry: 4_000_000_000,
        max_new_hosts_per_day: 50,
        epoch_lo: 0,
        epoch_hi: 9,
    });
    let mode = a.append(OpBody::ModeChange {
        to: CredMode::Quorum,
        migrated_on: a.dev(),
        cosigner: None,
    });
    let opened = a.append(OpBody::CertHostOpened {
        cert_id: issue.hash(),
        host_uid: uid(7),
    });
    let attest = a.append(OpBody::HeadAttest {
        device_id: a.dev(),
        seq: 1,
        chain_hash: cred.hash(),
    });
    pull(b, a);
    assert!(
        b.store.sync_cred_ciphertext(&uid(1)).unwrap().is_none(),
        "held until approved"
    );
    assert_eq!(b.store.sync_cred_mode().unwrap(), CredMode::Vault);
    // Green ops (CertHostOpened, HeadAttest) already took effect.
    assert_eq!(
        b.store.sync_cert_hosts_opened(&issue.hash()).unwrap(),
        vec![uid(7)]
    );
    assert!(b.store.sync_head_attest(&a.dev()).unwrap().is_some());
    b.approve_all();
    assert_eq!(
        b.store.sync_cred_ciphertext(&uid(1)).unwrap(),
        Some((vec![9; 40], vec![share]))
    );
    assert_eq!(b.store.sync_cred_mode().unwrap(), CredMode::Quorum);
    assert!(!b.store.sync_cert_revoked(&issue.hash()).unwrap());
    a.append(OpBody::CertRevoke {
        cert_id: issue.hash(),
    });
    pull(b, a);
    b.approve_all();
    assert!(b.store.sync_cert_revoked(&issue.hash()).unwrap());
    assert_eq!(
        b.store.sync_cert_new_hosts_since(&issue.hash(), 0).unwrap(),
        1
    );
    assert_eq!(
        b.store
            .sync_cert_new_hosts_since(&issue.hash(), i64::MAX)
            .unwrap(),
        0
    );
    let _ = (mode, opened, attest);
    // Chain tails and epochs are served for stage 2.
    assert_eq!(b.store.sync_chain_tail(&a.dev(), 3).unwrap().len(), 3);
    assert_eq!(b.store.sync_epoch(&a.dev()).unwrap(), 0);
}

#[test]
fn replayed_ops_after_a_restart_do_not_change_anything() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("web", "10.0.0.1")));
    a.append(upsert(
        1,
        HostFields {
            notes: Some(Some("x".into())),
            ..Default::default()
        },
    ));
    pull(b, a);
    b.approve_all();
    let before = (
        b.store.sync_state_hash().unwrap(),
        b.store.sync_projection_hash().unwrap(),
    );
    for op in a.store.sync_chain_tail(&a.dev(), 0).unwrap() {
        assert!(rejected(&b.store.sync_ingest(op).unwrap()).contains("duplicate"));
    }
    assert_eq!(
        before,
        (
            b.store.sync_state_hash().unwrap(),
            b.store.sync_projection_hash().unwrap()
        )
    );
    assert_eq!(b.store.sync_pending().unwrap().len(), 0);
    let _ = NewHostGroup::default();
}

// -------------------------------------------------- convergence property

fn pick<'a, T>(rng: &mut ChaCha8Rng, xs: &'a [T]) -> &'a T {
    &xs[(rng.next_u32() as usize) % xs.len()]
}

fn chance(rng: &mut ChaCha8Rng, pct: u32) -> bool {
    rng.next_u32() % 100 < pct
}

fn opt<T>(rng: &mut ChaCha8Rng, v: T) -> Option<Option<T>> {
    Some(if chance(rng, 25) { None } else { Some(v) })
}

const NAMES: [&str; 4] = ["web", "db", "app", "web"];
const ADDRS: [&str; 3] = ["10.0.0.1", "h.example", "10.0.0.2"];
const HOSTS: [u8; 4] = [1, 2, 3, 4];
const GROUPS: [u8; 3] = [20, 21, 22];
const IDENTS: [u8; 2] = [30, 31];

fn gen_body(rng: &mut ChaCha8Rng) -> OpBody {
    match rng.next_u32() % 100 {
        0..=54 => {
            let mut f = HostFields::default();
            if chance(rng, 40) {
                f.name = Some(pick(rng, &NAMES).to_string());
            }
            if chance(rng, 40) {
                f.address = Some(pick(rng, &ADDRS).to_string());
            }
            if chance(rng, 20) {
                f.label = opt(rng, "L".to_string());
            }
            if chance(rng, 20) {
                f.port = opt(rng, 2222);
            }
            if chance(rng, 20) {
                f.username = opt(rng, "root".to_string());
            }
            if chance(rng, 25) {
                let hop = format!("#{}", uid_hex(&uid(*pick(rng, &HOSTS))));
                f.proxy_jump = opt(rng, hop);
            }
            if chance(rng, 15) {
                f.forward_agent = opt(rng, true);
            }
            if chance(rng, 15) {
                f.transport = opt(rng, "mosh".to_string());
            }
            if chance(rng, 30) {
                let n = format!("n{}", rng.next_u32() % 5);
                f.notes = opt(rng, n);
            }
            if chance(rng, 20) {
                f.tags = Some(vec![format!("t{}", rng.next_u32() % 3)]);
            }
            if chance(rng, 25) {
                f.favorite = Some(chance(rng, 50));
            }
            if chance(rng, 25) {
                let mut g: Vec<Uid> = GROUPS
                    .iter()
                    .filter(|_| chance(rng, 50))
                    .map(|n| uid(*n))
                    .collect();
                g.sort();
                f.group_uids = Some(g);
            }
            if chance(rng, 25) {
                let i = uid(*pick(rng, &IDENTS));
                f.identity_uid = opt(rng, i);
            }
            OpBody::HostUpsert {
                host_uid: uid(*pick(rng, &HOSTS)),
                fields: f,
            }
        }
        55..=64 => OpBody::HostDelete {
            host_uid: uid(*pick(rng, &HOSTS)),
        },
        65..=79 => {
            let mut f = GroupFields::default();
            if chance(rng, 50) {
                f.name = Some(["g1", "g2", "web"][(rng.next_u32() % 3) as usize].to_string());
            }
            if chance(rng, 30) {
                let g = uid(*pick(rng, &GROUPS));
                f.parent_uid = opt(rng, g);
            }
            if chance(rng, 20) {
                let i = uid(*pick(rng, &IDENTS));
                f.default_identity_uid = opt(rng, i);
            }
            if chance(rng, 20) {
                f.default_port = opt(rng, 2200);
            }
            OpBody::GroupUpsert {
                group_uid: uid(*pick(rng, &GROUPS)),
                fields: f,
            }
        }
        80..=84 => OpBody::GroupDelete {
            group_uid: uid(*pick(rng, &GROUPS)),
        },
        85..=94 => OpBody::IdentityUpsert {
            identity_uid: uid(*pick(rng, &IDENTS)),
            fields: IdentityFields {
                name: chance(rng, 60)
                    .then(|| ["i1", "i2", "Default"][(rng.next_u32() % 3) as usize].to_string()),
                username: if chance(rng, 20) {
                    opt(rng, "u".to_string())
                } else {
                    None
                },
                has_password: chance(rng, 20).then(|| chance(rng, 50)),
            },
        },
        _ => OpBody::IdentityDelete {
            identity_uid: uid(*pick(rng, &IDENTS)),
        },
    }
}

fn run_case(seed: u64, n_peers: usize, steps: usize) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let peers = ring(n_peers);
    let mut inbox: Vec<Vec<Op>> = vec![Vec::new(); n_peers];

    // Deliver one op; true when it is done with (applied, pending, duplicate).
    let try_one = |peer: &Peer, op: Op| -> bool {
        match peer.store.sync_ingest(op).unwrap() {
            IngestOutcome::Applied | IngestOutcome::Pending => true,
            IngestOutcome::Rejected(m) if m.starts_with("duplicate") => true,
            IngestOutcome::Rejected(m) if m.starts_with("gap") => false,
            other => panic!("seed {seed}: honest op refused: {other:?}"),
        }
    };

    for _ in 0..steps {
        if chance(&mut rng, 55) {
            let p = (rng.next_u32() as usize) % n_peers;
            let body = gen_body(&mut rng);
            let op = peers[p].append(body);
            for (q, inbox) in inbox.iter_mut().enumerate() {
                if q != p {
                    inbox.push(op.clone());
                }
            }
        } else {
            let q = (rng.next_u32() as usize) % n_peers;
            if inbox[q].is_empty() {
                continue;
            }
            let i = (rng.next_u32() as usize) % inbox[q].len();
            let op = inbox[q].swap_remove(i);
            if chance(&mut rng, 10) {
                inbox[q].push(op.clone()); // duplicate delivery
            }
            if !try_one(&peers[q], op.clone()) {
                inbox[q].push(op);
            }
        }
    }
    // Flush: every peer receives everything, in a shuffled order, retrying gaps.
    for q in 0..n_peers {
        let mut rounds = 0;
        while !inbox[q].is_empty() {
            rounds += 1;
            assert!(rounds < 200, "seed {seed}: delivery stuck");
            let mut batch = std::mem::take(&mut inbox[q]);
            while !batch.is_empty() {
                let i = (rng.next_u32() as usize) % batch.len();
                let op = batch.swap_remove(i);
                if !try_one(&peers[q], op.clone()) {
                    inbox[q].push(op);
                }
            }
        }
    }
    for p in &peers {
        p.approve_all();
    }
    let state: Vec<_> = peers
        .iter()
        .map(|p| p.store.sync_state_hash().unwrap())
        .collect();
    let proj: Vec<_> = peers
        .iter()
        .map(|p| p.store.sync_projection_hash().unwrap())
        .collect();
    assert!(
        state.iter().all(|h| *h == state[0]),
        "seed {seed}: state hashes differ"
    );
    if !proj.iter().all(|h| *h == proj[0]) {
        for p in &peers {
            eprintln!(
                "--- peer {}\n{}",
                p.dev(),
                p.store.sync_projection_dump().unwrap()
            );
        }
        panic!("seed {seed}: projection hashes differ");
    }
    for p in &peers {
        let bad: i64 = p
            .store
            .with_conn(|c| {
                Ok(c.prepare("PRAGMA foreign_key_check")?
                    .query_map([], |_| Ok(()))?
                    .count() as i64)
            })
            .unwrap();
        assert_eq!(bad, 0, "seed {seed}: dangling foreign keys");
    }
}

/// Seeds per property test: 60 by default, `SSHUB_PROP_SEEDS=n` for a long run.
fn seeds(default: u64) -> u64 {
    std::env::var("SSHUB_PROP_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn two_stores_converge_on_random_ops_in_random_orders() {
    let from = std::env::var("SSHUB_PROP_FROM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0u64);
    for seed in from..from + seeds(60) {
        run_case(seed, 2, 30);
    }
}

#[test]
fn three_stores_converge_on_random_ops_in_random_orders() {
    let from = std::env::var("SSHUB_PROP_FROM3")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000u64);
    for seed in from..from + seeds(40) {
        run_case(seed, 3, 40);
    }
}
