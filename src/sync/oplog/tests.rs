//! Op-log tests: fixed adversarial cases and the convergence property test.

use super::*;
use crate::store::{
    uid_hex, LauncherStore, ManagedHost, NewHost, NewHostGroup, FAVORITES_GROUP_UID,
};
use crate::sync::crypto::{DeviceIdentity, DevicePublic, ALG_CURRENT};
use crate::sync::limits::LAMPORT_JUMP_CAP;
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

thread_local! {
    static CUR_SEED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

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
                    if !e.to_string().contains("not pending") {
                        let dump = self
                            .store
                            .with_conn(|c| {
                                let mut out = String::new();
                                for t in ["hosts", "host_groups", "identities"] {
                                    let mut st = c.prepare(&format!("SELECT hex(uid), name FROM {t}"))?;
                                    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                                        let (u, n) = r?;
                                        out.push_str(&format!("{t} {} {n:?}\n", &u[..8]));
                                    }
                                }
                                let mut st = c.prepare("SELECT kind, hex(uid), alive, name, name_lamport, name_device FROM sync_entities")?;
                                for r in st.query_map([], |r| {
                                    Ok(format!("cache {} {} alive={} {:?} {} {}", r.get::<_, String>(0)?, &r.get::<_, String>(1)?[..8], r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?, r.get::<_, String>(5)?))
                                })? {
                                    out.push_str(&r?);
                                    out.push('\n');
                                }
                                Ok(out)
                            })
                            .unwrap();
                        panic!(
                            "seed {}: {e:#}\nfailing op: {:?}\n{dump}",
                            CUR_SEED.with(|c| c.get()),
                            op.body
                        );
                    }
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
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
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
            kind: DeviceKind::Phone,
            verified_by: VerifiedBy::SshHostKey,
        },
        OpBody::DeviceRevoke {
            device_id: "ffeeddccbbaa9988".into(),
        },
        OpBody::ShareRelease {
            host_uid: uid(1),
            request_id: u64::MAX,
            requester: "0011223344556677".into(),
            eph_x25519_pub: [8; 32],
        },
        OpBody::ShareGrant {
            host_uid: uid(1),
            request_id: 7,
            requester: "0011223344556677".into(),
            epoch: 3,
            sealed: vec![5; 128],
        },
        OpBody::ShareReshareSession {
            epoch: 2,
            to: "0011223344556677".into(),
            sealed: vec![1, 2, 3, 4],
        },
        OpBody::ShareStorageKey {
            epoch: 2,
            sealed: vec![],
        },
        OpBody::ShareReshareBatch {
            epoch: 2,
            new_holders: vec!["a".into(), "b".into()],
            items: vec![ReshareItem {
                host_uid: uid(1),
                sub_shares: vec![("a".into(), vec![9; 5]), ("b".into(), vec![])],
                commitments: vec![vec![1, 2], vec![]],
            }],
        },
        OpBody::ShareReshareConfirmBatch {
            epoch: 2,
            items: vec![ConfirmItem {
                host_uid: uid(1),
                sealed_share: vec![255; 7],
            }],
        },
        OpBody::WrapKeyRotate {
            device_id: "0011223344556677".into(),
            wrap_epoch: 1,
            x25519_pub: [3; 32],
            mlkem_pub: vec![0, 1, 2],
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
            third_factor: ThirdFactor::Paper,
            liveness: vec![HeadAttest {
                device_id: "a".into(),
                seq: 3,
                chain_hash: [1; 32],
            }],
            paper_sig: Some([0xa5; 64]),
        },
        OpBody::PhoneReenroll {
            initiated_at: 0,
            signers: vec![],
            via_paper: false,
            third_factor: ThirdFactor::EmailCode,
            liveness: vec![],
            paper_sig: None,
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
        OpBody::RecoveryApprove {
            reenroll_hash: [6; 32],
        },
        OpBody::RedApprove { op_hash: [1; 32] },
        OpBody::RedPhoneSign { op_hash: [2; 32] },
        OpBody::RedFactor {
            op_hash: [3; 32],
            factor: ThirdFactor::EmailCode,
            sig: [0x5a; 64],
        },
        OpBody::FactorRegister {
            factor: ThirdFactor::Paper,
            public: [4; 32],
            sealed: vec![],
            salt: [9; 16],
            sender: None,
        },
        OpBody::FactorRegister {
            factor: ThirdFactor::EmailCode,
            public: [5; 32],
            sealed: vec![7; 128],
            salt: [0xab; 16],
            sender: Some("0011223344556677".into()),
        },
        OpBody::DeviceRoleChange {
            device_id: "0011223344556677".into(),
            to: DeviceRole::Holder,
        },
        OpBody::BulkExport {
            requester: "0011223344556677".into(),
            window_secs: 3600,
        },
        OpBody::ExemptionSet {
            device_id: "0011223344556677".into(),
            slot: None,
        },
        OpBody::ExemptionSet {
            device_id: "0011223344556677".into(),
            slot: Some(DeviceIdentity::generate().public()),
        },
        OpBody::CertExtend {
            cert_id: [8; 32],
            extra_hosts: 50,
        },
        OpBody::CheckpointPropose {
            cut: vec![
                CutEntry {
                    device_id: "a".into(),
                    seq: 9,
                    op_hash: [1; 32],
                    lamport: 12,
                },
                CutEntry {
                    device_id: "b".into(),
                    seq: u64::MAX,
                    op_hash: [2; 32],
                    lamport: u64::MAX,
                },
            ],
            state_hash: [3; 32],
            state_len: u64::MAX,
            prev_checkpoint: [0; 32],
            required: vec!["a".into(), "b".into()],
        },
        OpBody::CheckpointSign {
            proposal_hash: [4; 32],
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
    // A stranger's op cannot be checked: it waits for a human, takes no effect.
    assert!(matches!(
        b.store.sync_ingest(so).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(b.host(&uid(2)).is_none());

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
        kind: DeviceKind::Terminal,
        verified_by: VerifiedBy::Sas,
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
    assert!(matches!(
        b.store.sync_ingest(early.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(b
        .store
        .sync_chain_tail(&newcomer.dev(), 0)
        .unwrap()
        .is_empty());
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
fn an_entity_that_loses_its_name_to_a_void_loses_the_row_the_sync_made() {
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let revoke = c.append(OpBody::DeviceRevoke { device_id: a.dev() });
    let create = a.append(upsert(1, hf("web", "10.0.0.1")));
    assert!(create.lamport >= revoke.lamport);
    b.store.sync_ingest(create.clone()).unwrap();
    b.store.sync_resolve_pending(&create.hash(), true).unwrap();
    assert_eq!(b.host(&uid(1)).unwrap().name, "web");
    // b adds a name-less safe edit, and another host that wants the same name.
    b.append(upsert(
        1,
        HostFields {
            notes: Some(Some("n".into())),
            ..Default::default()
        },
    ));
    b.append(upsert(2, hf("web", "10.0.0.2")));
    assert_eq!(b.host(&uid(2)).unwrap().name, "web (02020202)");
    // The revoke voids a's create: host 1 has no name any more, its row goes
    // and host 2 takes the name (no UNIQUE clash, no stale row).
    b.store.sync_ingest(revoke.clone()).unwrap();
    b.store.sync_resolve_pending(&revoke.hash(), true).unwrap();
    assert!(b.host(&uid(1)).is_none());
    assert_eq!(b.host(&uid(2)).unwrap().name, "web");
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

#[test]
fn base64_and_hex_forms_in_the_canonical_text() {
    let op = wrap(OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![0xfb, 0xff, 0x01],
    });
    let text = String::from_utf8(op.canonical_bytes()).unwrap();
    assert!(text.contains("\"sealed\":\"+/8B\""), "{text}");
    let op = wrap(OpBody::CertRevoke {
        cert_id: [0xab; 32],
    });
    let text = String::from_utf8(op.canonical_bytes()).unwrap();
    assert!(
        text.contains(&format!("\"cert_id\":\"{}\"", "ab".repeat(32))),
        "{text}"
    );
    // Wrong alphabet and missing padding are refused by the strict decoder.
    let op = wrap(OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![0xfb, 0xff],
    });
    let good = String::from_utf8(op.canonical_bytes()).unwrap();
    assert!(good.contains("\"sealed\":\"+/8=\""), "{good}");
    assert!(Op::from_canonical(good.as_bytes(), op.signature).is_ok());
    for bad in [
        good.replace("+/8=", "-_8="),
        good.replace("+/8=", "+/8"),
        good.replace("+/8=", "+/9="),
    ] {
        assert!(
            Op::from_canonical(bad.as_bytes(), op.signature).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn reshare_and_wrap_rotation_ops_are_green_and_bounded() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let item = |n: u8| ConfirmItem {
        host_uid: uid(n),
        sealed_share: vec![n; 4],
    };
    let session = a.append(OpBody::ShareReshareSession {
        epoch: 1,
        to: b.dev(),
        sealed: vec![1; 8],
    });
    let batch = a.append(OpBody::ShareReshareBatch {
        epoch: 1,
        new_holders: vec![b.dev()],
        items: vec![],
    });
    let confirm = a.append(OpBody::ShareReshareConfirmBatch {
        epoch: 1,
        items: vec![item(1)],
    });
    let key = a.append(OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![2; 8],
    });
    for op in [&session, &batch, &confirm, &key] {
        assert_eq!(op.tier(), OpTier::Green);
    }
    for o in pull(b, a) {
        assert!(matches!(o, IngestOutcome::Applied), "{o:?}");
    }
    // Over the item limit: refused by the sender and by a hostile one.
    let too_many: Vec<ConfirmItem> = (0..=crate::sync::limits::RESHARE_ITEMS_PER_OP)
        .map(|i| item(i as u8))
        .collect();
    assert!(a
        .store
        .sync_append_local(
            &a.id,
            OpBody::ShareReshareConfirmBatch {
                epoch: 1,
                items: too_many.clone()
            }
        )
        .is_err());
    let mut forged = Op {
        device_id: a.dev(),
        seq: 5,
        lamport: 50,
        prev_hash: key.hash(),
        timestamp: 0,
        epoch: 0,
        alg: ALG_CURRENT,
        body: OpBody::ShareReshareConfirmBatch {
            epoch: 1,
            items: too_many,
        },
        signature: [0; 64],
    };
    forged.signature = a.id.sign(&forged.canonical_bytes());
    assert!(rejected(&b.store.sync_ingest(forged).unwrap()).contains("item limit"));
}

#[test]
fn wrap_key_rotation_replaces_the_current_keys_and_keeps_the_history() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    // A third device that a and b trust, whose id rotates its wrap keys.
    let mut ident = Peer::new();
    for p in [a, b] {
        p.store
            .sync_register_device(
                &ident.id.public(),
                "rot",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    let old = ident.id.public();
    let new = ident.id.rotate_wrap_keys();
    assert_eq!(
        new.ed25519_pub, old.ed25519_pub,
        "the signing key never rotates"
    );
    assert_ne!(new.x25519_pub, old.x25519_pub);
    let rotate = |epoch: u64, p: &DevicePublic| OpBody::WrapKeyRotate {
        device_id: ident.dev(),
        wrap_epoch: epoch,
        x25519_pub: p.x25519_pub,
        mlkem_pub: p.mlkem_pub.clone(),
    };
    let op = ident.append(rotate(1, &new));
    assert_eq!(op.tier(), OpTier::Green);
    let row = |p: &Peer| {
        p.store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == ident.dev())
            .unwrap()
    };
    assert!(matches!(
        a.store.sync_ingest(op.clone()).unwrap(),
        IngestOutcome::Applied
    ));
    assert_eq!(row(a).public, new);
    assert_eq!(row(a).wrap_epoch, 1);
    let generations: i64 = a
        .store
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM sync_wrap_history WHERE device_id = ?1",
                [ident.dev()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        generations, 2,
        "old and new generation are both addressable"
    );

    // Only the device itself may rotate its keys.
    let mut hostile = Op {
        device_id: a.dev(),
        seq: 1,
        lamport: 1,
        prev_hash: [0; 32],
        timestamp: 0,
        epoch: 0,
        alg: ALG_CURRENT,
        body: rotate(9, &new),
        signature: [0; 64],
    };
    hostile.signature = a.id.sign(&hostile.canonical_bytes());
    assert!(rejected(&b.store.sync_ingest(hostile).unwrap()).contains("own device"));

    // A later op at an equal epoch never rolls the keys back.
    let stale = ident.append(rotate(1, &old));
    a.store.sync_ingest(stale).unwrap();
    assert_eq!(row(a).public, new);

    // b receives the same chain and ends in the same place.
    pull(b, &ident);
    assert_eq!(row(b).public, new);
    assert_eq!(row(b).wrap_epoch, 1);
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
    CUR_SEED.with(|c| c.set(seed));
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

// ------------------------------------------------- checkpoints and pruning

fn file_peer(dir: &std::path::Path) -> Peer {
    Peer {
        store: LauncherStore::open(dir.join("launcher.db")).unwrap(),
        id: DeviceIdentity::generate(),
    }
}

/// `n` attended terminals trusting each other, each with one op so that all
/// are live at any later checkpoint.
fn live_ring(peers: Vec<Peer>) -> Vec<Peer> {
    for p in &peers {
        for q in &peers {
            p.store
                .sync_register_device(
                    &q.id.public(),
                    "peer",
                    DeviceRole::Holder,
                    Attendance::Attended,
                    DeviceKind::Terminal,
                    VerifiedBy::Sas,
                )
                .unwrap();
        }
    }
    peers
}

fn cut_pairs(b: &CheckpointBundle) -> Vec<(DeviceId, u64)> {
    match &b.proposal.body {
        OpBody::CheckpointPropose { cut, .. } => {
            cut.iter().map(|c| (c.device_id.clone(), c.seq)).collect()
        }
        _ => unreachable!(),
    }
}

/// One op per peer, so every device has a head at any cut and counts as live.
fn touch(ring: &[Peer]) {
    for (i, p) in ring.iter().enumerate() {
        p.append(upsert(60 + i as u8, hf(&format!("seed{i}"), "10.1.1.1")));
    }
}

fn settle(ring: &[Peer]) {
    sync_all(ring);
    for p in ring {
        p.approve_all();
    }
    sync_all(ring);
}

/// Propose at `proposer`, have everyone sign, spread the ops and record the
/// bundle everywhere. Returns the bundle and the proposal hash.
fn checkpoint_round(ring: &[Peer], proposer: usize, now: i64) -> (CheckpointBundle, [u8; 32]) {
    let (proposal, state) = ring[proposer]
        .store
        .sync_checkpoint_propose(&ring[proposer].id, None, now)
        .unwrap();
    sync_all(ring);
    let mut signs = Vec::new();
    for p in ring {
        let s = p
            .store
            .sync_checkpoint_sign(&p.id, &proposal, &state)
            .unwrap();
        signs.push(s.expect("an honest verifier signs a matching proposal"));
    }
    sync_all(ring);
    let bundle = CheckpointBundle {
        proposal: proposal.clone(),
        signs,
        heads: vec![],
        state,
    };
    for p in ring {
        let st = p.store.sync_checkpoint_record(&bundle).unwrap();
        assert_eq!(st, CheckpointStatus::Accepted, "{}", p.dev());
    }
    (bundle, proposal.hash())
}

fn random_content(ring: &[Peer], seed: u64, steps: usize) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    for _ in 0..steps {
        let p = (rng.next_u32() as usize) % ring.len();
        ring[p].append(gen_body(&mut rng));
    }
}

#[test]
fn the_same_cut_folds_to_the_same_state_on_every_store() {
    for seed in 1..=seeds(6) {
        let n = 2 + (seed as usize % 2);
        let ring = live_ring((0..n).map(|_| Peer::new()).collect());
        random_content(&ring, seed, 40);
        settle(&ring);
        let cut = ring[0]
            .store
            .sync_checkpoint_build(None, 1_900_000_000)
            .unwrap()
            .0;
        let hashes: Vec<_> = ring
            .iter()
            .map(|p| {
                p.store
                    .sync_checkpoint_build(Some(cut.clone()), 1_900_000_000)
                    .unwrap()
                    .2
            })
            .collect();
        assert!(
            hashes.iter().all(|h| *h == hashes[0]),
            "seed {seed}: state hashes differ"
        );
        // And it is stable: folding twice gives the same bytes.
        let again = ring[0]
            .store
            .sync_checkpoint_build(Some(cut), 1_900_000_000)
            .unwrap()
            .2;
        assert_eq!(again, hashes[0]);
    }
}

#[test]
fn a_lying_proposer_is_disputed_and_suspect_but_never_compromised() {
    let ring = live_ring((0..3).map(|_| Peer::new()).collect());
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    for p in &ring {
        p.append(upsert(
            p.id.public().ed25519_pub[0],
            hf(&format!("h{}", p.dev()), "10.0.0.1"),
        ));
    }
    settle(&ring);
    let now = crate::store::now_ts();
    let (cut, state, _) = a.store.sync_checkpoint_build(None, now).unwrap();
    // The lie: one host's address is something else in the blob.
    let mut v: serde_json::Value = serde_json::from_slice(&state).unwrap();
    v["entities"][0]["fields"]["address"]["value"] = serde_json::json!("evil.example");
    let lie = serde_json::to_vec(&v).unwrap();
    let mut required: Vec<DeviceId> = ring.iter().map(|p| p.dev()).collect();
    required.sort();
    let proposal = a.append(OpBody::CheckpointPropose {
        cut,
        state_hash: crate::sync::crypto::chain_hash(&lie),
        state_len: lie.len() as u64,
        prev_checkpoint: [0; 32],
        required,
    });
    sync_all(&ring);
    // The liar signs its own lie; the honest ones do not.
    assert!(b
        .store
        .sync_checkpoint_sign(&b.id, &proposal, &lie)
        .unwrap()
        .is_none());
    assert!(c
        .store
        .sync_checkpoint_sign(&c.id, &proposal, &lie)
        .unwrap()
        .is_none());
    let verdict = b.store.sync_checkpoint_verify(&proposal, &lie).unwrap();
    match verdict {
        CheckpointVerdict::Differs { section, detail } => {
            assert_eq!(section, "entities", "{detail}");
        }
        other => panic!("{other:?}"),
    }
    let rows = b.store.sync_checkpoints().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, CheckpointStatus::Disputed);
    let status = |p: &Peer| {
        p.store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == a.dev())
            .unwrap()
            .status
    };
    assert_eq!(status(b), DeviceStatus::Suspect, "a rumour, not a proof");
    assert_ne!(status(b), DeviceStatus::Compromised);
    // Even with the liar's signature the lie is never accepted.
    let own_sign = a.append(OpBody::CheckpointSign {
        proposal_hash: proposal.hash(),
    });
    let st = b
        .store
        .sync_checkpoint_record(&CheckpointBundle {
            proposal,
            signs: vec![own_sign],
            heads: vec![],
            state: lie,
        })
        .unwrap();
    assert_eq!(st, CheckpointStatus::Disputed);
}

#[test]
fn a_proposal_with_ops_still_pending_below_the_cut_is_blocked() {
    let ring = live_ring((0..2).map(|_| Peer::new()).collect());
    let (a, b) = (&ring[0], &ring[1]);
    let now = crate::store::now_ts();
    a.append(upsert(1, hf("web", "10.0.0.1")));
    b.append(upsert(2, hf("db", "10.0.0.2")));
    pull(a, b);
    // a has not reviewed b's creation: it refuses to build a checkpoint.
    let err = a.store.sync_checkpoint_build(None, now).unwrap_err();
    assert!(err.to_string().contains("pending"), "{err}");
    assert!(a.store.sync_checkpoint_propose(&a.id, None, now).is_err());
    a.approve_all();
    let (p, state) = a.store.sync_checkpoint_propose(&a.id, None, now).unwrap();
    // b has a's ops but has not reviewed them: blocked, not mismatching.
    pull(b, a);
    let v = b.store.sync_checkpoint_verify(&p, &state).unwrap();
    assert!(matches!(v, CheckpointVerdict::Blocked(_)), "{v:?}");
    b.approve_all();
    assert_eq!(
        b.store.sync_checkpoint_verify(&p, &state).unwrap(),
        CheckpointVerdict::Matches
    );
}

#[test]
fn prune_then_bootstrap_reaches_the_same_state_as_full_history() {
    for k in 0..seeds(1) {
        prune_bootstrap_case(41 + 3 * k);
    }
}

fn prune_bootstrap_case(base: u64) {
    let dir = tempfile::tempdir().unwrap();
    let ring = live_ring(vec![file_peer(dir.path()), Peer::new(), Peer::new()]);
    let t0 = crate::store::now_ts();
    touch(&ring);
    random_content(&ring, base, 40);
    settle(&ring);
    let (_c1, h1) = checkpoint_round(&ring, 0, t0);
    random_content(&ring, base + 1, 40);
    settle(&ring);
    let (_c2, h2) = checkpoint_round(&ring, 1, t0 + 100);
    assert_eq!(ring[0].store.sync_checkpoints().unwrap().len(), 2);
    assert_ne!(h1, h2);
    random_content(&ring, base + 2, 20);
    settle(&ring);

    // Nothing is pruned before the delay has passed.
    assert_eq!(
        ring[0].store.sync_prune(t0 + 200).unwrap(),
        PruneReport::default()
    );
    let report = ring[0].store.sync_prune(t0 + 8 * 86_400 + 1_000).unwrap();
    assert!(report.ops_deleted > 0, "{report:?}");
    // Idempotent.
    assert_eq!(
        ring[0]
            .store
            .sync_prune(t0 + 8 * 86_400 + 2_000)
            .unwrap()
            .ops_deleted,
        0
    );

    // More work after the prune, spread to everyone who keeps a log.
    ring[2].append(upsert(
        1,
        HostFields {
            notes: Some(Some("after the prune".into())),
            ..Default::default()
        },
    ));
    settle(&ring);

    // A fresh device adopts the newest checkpoint (the ring fingerprint is the
    // trust anchor) and syncs the tails.
    let bundle = ring[1]
        .store
        .sync_checkpoint_bundle(&h2)
        .unwrap()
        .expect("the newest checkpoint keeps its blob");
    assert!(!bundle.heads.is_empty());
    let fresh = Peer::new();
    let words = ring_fingerprint(&bundle.state).unwrap();
    assert_eq!(words.split(' ').count(), 8);
    assert!(fresh
        .store
        .sync_bootstrap(
            &bundle,
            &BootstrapTrust::RingFingerprint("wrong words".into())
        )
        .is_err());
    fresh
        .store
        .sync_bootstrap(
            &bundle,
            &BootstrapTrust::RingFingerprint(words.to_uppercase()),
        )
        .unwrap();
    // Right after the bootstrap the store folds to exactly the adopted state.
    let adopted = match &bundle.proposal.body {
        OpBody::CheckpointPropose { cut, .. } => cut.clone(),
        _ => unreachable!(),
    };
    let refold = fresh
        .store
        .sync_checkpoint_build(Some(adopted), bundle.proposal.timestamp)
        .unwrap()
        .1;
    if refold != bundle.state {
        let (a, b): (serde_json::Value, serde_json::Value) = (
            serde_json::from_slice(&refold).unwrap(),
            serde_json::from_slice(&bundle.state).unwrap(),
        );
        panic!(
            "bootstrap does not reproduce the blob: {:?}",
            super::checkpoint::diff_state(&a, &b)
        );
    }
    for p in &ring {
        pull(&fresh, p);
    }
    fresh.approve_all();

    let cut = ring[1]
        .store
        .sync_checkpoint_build(None, t0 + 9 * 86_400)
        .unwrap()
        .0;
    let fold = |p: &Peer| {
        p.store
            .sync_checkpoint_build(Some(cut.clone()), t0 + 9 * 86_400)
            .unwrap()
            .1
    };
    let full = fold(&ring[1]);
    let same = |what: &str, other: Vec<u8>| {
        if other != full {
            let (a, b): (serde_json::Value, serde_json::Value) = (
                serde_json::from_slice(&full).unwrap(),
                serde_json::from_slice(&other).unwrap(),
            );
            panic!("{what}: {:?}", super::checkpoint::diff_state(&a, &b));
        }
    };
    same("pruned store", fold(&ring[0]));
    same("bootstrapped store", fold(&fresh));
    assert_eq!(
        fresh.store.sync_state_hash().unwrap(),
        ring[1].store.sync_state_hash().unwrap()
    );
    assert_eq!(
        fresh.store.sync_projection_hash().unwrap(),
        ring[1].store.sync_projection_hash().unwrap()
    );
    assert_eq!(
        ring[0].store.sync_projection_hash().unwrap(),
        ring[1].store.sync_projection_hash().unwrap()
    );
    // The bootstrapped store holds no history below the cut and says so.
    let dev = ring[1].dev();
    assert!(fresh.store.sync_chain_tail(&dev, 0).is_err());
}

#[test]
fn a_fork_at_the_prune_floor_is_still_proven_and_older_ops_are_covered() {
    let ring = live_ring(vec![Peer::new(), Peer::new(), Peer::new()]);
    let t0 = crate::store::now_ts();
    touch(&ring);
    random_content(&ring, 7, 30);
    settle(&ring);
    let (c1, _) = checkpoint_round(&ring, 0, t0);
    random_content(&ring, 8, 30);
    settle(&ring);
    checkpoint_round(&ring, 1, t0 + 100);
    let a = &ring[0];
    a.store.sync_prune(t0 + 8 * 86_400 + 1_000).unwrap();
    // The floor is the cut of the first checkpoint.
    let victim = &ring[2];
    let entry = cut_pairs(&c1)
        .into_iter()
        .find(|c| c.0 == victim.dev())
        .unwrap();
    let kept = a.store.sync_chain_tail(&victim.dev(), entry.1 - 1).unwrap();
    assert_eq!(kept[0].seq, entry.1, "the head op of the cut is kept");
    // Below the floor: ignored as covered, and no tail can be served.
    let mut below = kept[0].clone();
    below.seq = entry.1 - 1;
    below.signature = victim.id.sign(&below.canonical_bytes());
    assert!(rejected(&a.store.sync_ingest(below).unwrap()).contains("covered"));
    let mut forged = kept[0].clone();
    forged.timestamp += 1;
    forged.signature = victim.id.sign(&forged.canonical_bytes());
    match a.store.sync_ingest(forged.clone()).unwrap() {
        IngestOutcome::ForkProof(x, y) => assert_ne!(x.hash(), y.hash()),
        other => panic!("{other:?}"),
    }
    let row = a
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == victim.dev())
        .unwrap();
    assert_eq!(row.status, DeviceStatus::Compromised);
    let err = a.store.sync_chain_tail(&victim.dev(), 0).unwrap_err();
    assert_eq!(err.downcast_ref::<Pruned>().unwrap().floor_seq, entry.1);
}

#[test]
fn pruning_removes_superseded_wrapped_shares_from_the_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let ring = live_ring(vec![file_peer(dir.path()), Peer::new()]);
    let a = &ring[0];
    let t0 = crate::store::now_ts();
    let mk =
        |tag: &str| -> Vec<u8> { format!("SECRET-MARKER-{tag}-0123456789ABCDEF").into_bytes() };
    let cred = |host: u8, tag: &str| OpBody::CredUpdate {
        host_uid: uid(host),
        ciphertext: mk(&format!("CT{tag}")),
        wrapped_shares: ring
            .iter()
            .map(|p| WrappedShare {
                recipient: p.dev(),
                alg: ALG_CURRENT,
                sealed: mk(tag),
            })
            .collect(),
    };
    a.append(upsert(1, hf("web", "10.0.0.1")));
    a.append(upsert(2, hf("db", "10.0.0.2")));
    a.append(cred(1, "ONE"));
    a.append(cred(1, "TWO")); // supersedes ONE
    a.append(cred(2, "THREE")); // resharded later
    a.append(upsert(
        1,
        HostFields {
            notes: Some(Some("n".into())),
            ..Default::default()
        },
    ));
    ring[1].append(upsert(3, hf("x", "10.0.0.3")));
    settle(&ring);
    checkpoint_round(&ring, 0, t0);
    a.append(OpBody::ShareReshareConfirmBatch {
        epoch: 1,
        items: vec![ConfirmItem {
            host_uid: uid(2),
            sealed_share: mk("FOUR"),
        }],
    });
    a.append(upsert(
        1,
        HostFields {
            notes: Some(Some("m".into())),
            ..Default::default()
        },
    ));
    ring[1].append(upsert(
        3,
        HostFields {
            notes: Some(Some("m".into())),
            ..Default::default()
        },
    ));
    settle(&ring);
    checkpoint_round(&ring, 1, t0 + 100);
    let path = dir.path().join("launcher.db");
    let scan = |tag: &str| -> bool {
        let bytes = std::fs::read(&path).unwrap();
        let raw = mk(tag);
        let b64 = super::canonical::b64_encode(&raw);
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        has(&raw) || has(b64.as_bytes())
    };
    for tag in ["ONE", "TWO", "THREE", "FOUR"] {
        assert!(scan(tag), "{tag} should be in the file before the prune");
    }
    let report = a.store.sync_prune(t0 + 8 * 86_400 + 1_000).unwrap();
    assert!(report.wrapped_share_blobs_dropped >= 3, "{report:?}");
    assert!(!scan("ONE"), "superseded credential op");
    assert!(!scan("THREE"), "shares of a resharded credential");
    assert!(scan("TWO"), "the live wrapped shares stay");
    assert!(scan("FOUR"), "the live reshare envelope stays");
    // Stage 3 can still read both.
    let (ct, shares) = a.store.sync_cred_ciphertext(&uid(1)).unwrap().unwrap();
    assert_eq!(ct, mk("CTTWO"));
    assert_eq!(shares.len(), 2);
    assert_eq!(a.store.sync_share_envelopes(&uid(2)).unwrap().len(), 1);
}

#[test]
fn ops_of_a_device_that_was_stale_at_the_checkpoint_wait_for_review() {
    let ring = live_ring((0..3).map(|_| Peer::new()).collect());
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let now = crate::store::now_ts();
    // c's last op is 40 days old at the checkpoint; a and b are current.
    c.store
        .sync_append_local_at(&c.id, upsert(9, hf("old", "10.0.0.9")), now - 40 * 86_400)
        .unwrap();
    a.append(upsert(1, hf("web", "10.0.0.1")));
    b.append(upsert(2, hf("db", "10.0.0.2")));
    settle(&ring);
    let (proposal, state) = a.store.sync_checkpoint_propose(&a.id, None, now).unwrap();
    sync_all(&ring);
    let mut signs = Vec::new();
    for p in [a, b] {
        signs.push(
            p.store
                .sync_checkpoint_sign(&p.id, &proposal, &state)
                .unwrap()
                .unwrap(),
        );
    }
    sync_all(&ring);
    let bundle = CheckpointBundle {
        proposal,
        signs,
        heads: vec![],
        state,
    };
    for p in &ring {
        assert_eq!(
            p.store.sync_checkpoint_record(&bundle).unwrap(),
            CheckpointStatus::Accepted
        );
    }
    // c comes back and writes a safe field: it waits, whatever the class.
    let late = c.append(upsert(
        9,
        HostFields {
            notes: Some(Some("back".into())),
            ..Default::default()
        },
    ));
    assert_eq!(late.tier(), OpTier::Green);
    assert!(matches!(
        a.store.sync_ingest(late.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert_eq!(
        a.store
            .sync_pending_reason(&late.hash())
            .unwrap()
            .as_deref(),
        Some("stale")
    );
    // The live devices are unaffected.
    let fine = b.append(upsert(
        2,
        HostFields {
            notes: Some(Some("ok".into())),
            ..Default::default()
        },
    ));
    assert!(matches!(
        a.store.sync_ingest(fine).unwrap(),
        IngestOutcome::Applied
    ));
}

#[test]
fn competing_proposals_resolve_to_the_lowest_lamport_and_device() {
    let ring = live_ring((0..2).map(|_| Peer::new()).collect());
    let (a, b) = (&ring[0], &ring[1]);
    a.append(upsert(1, hf("web", "10.0.0.1")));
    b.append(upsert(2, hf("db", "10.0.0.2")));
    settle(&ring);
    let now = crate::store::now_ts();
    let (pa, sa) = a.store.sync_checkpoint_propose(&a.id, None, now).unwrap();
    let (pb, sb) = b.store.sync_checkpoint_propose(&b.id, None, now).unwrap();
    for (p, s) in [(&pa, &sa), (&pb, &sb)] {
        a.store
            .sync_checkpoint_record(&CheckpointBundle {
                proposal: p.clone(),
                signs: vec![],
                heads: vec![],
                state: s.clone(),
            })
            .unwrap();
    }
    let rows = a.store.sync_checkpoints().unwrap();
    let winner = if (pa.lamport, &pa.device_id) <= (pb.lamport, &pb.device_id) {
        pa.hash()
    } else {
        pb.hash()
    };
    for r in rows {
        let want = if r.proposal_hash == winner {
            CheckpointStatus::Verified
        } else {
            CheckpointStatus::Void
        };
        assert_eq!(r.status, want, "{}", r.device_id);
    }
}

#[test]
fn role_change_updates_the_device_and_a_mirror_revoke_is_yellow() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let mirror = Peer::new();
    for p in [a, b] {
        p.store
            .sync_register_device(
                &mirror.id.public(),
                "m",
                DeviceRole::Mirror,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    let role = |p: &Peer| {
        p.store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == mirror.dev())
            .unwrap()
            .role
    };
    // Revoking a mirror is Yellow here, a holder is Red.
    let revoke_mirror = a.append(OpBody::DeviceRevoke {
        device_id: mirror.dev(),
    });
    let revoke_holder = a.append(OpBody::DeviceRevoke { device_id: b.dev() });
    let tier = |p: &Peer, op: &Op| {
        p.store
            .with_conn(|c| super::merge::effective_tier(c, op))
            .unwrap()
    };
    assert_eq!(tier(b, &revoke_mirror), OpTier::Yellow);
    assert_eq!(tier(b, &revoke_holder), OpTier::Red);
    // Promotion is Red, waits, and takes effect through the programmatic resolve.
    let promote = a.append(OpBody::DeviceRoleChange {
        device_id: mirror.dev(),
        to: DeviceRole::Holder,
    });
    assert_eq!(promote.tier(), OpTier::Red);
    assert_eq!(role(a), DeviceRole::Holder, "the author applied it locally");
    pull(b, a);
    assert_eq!(
        b.store
            .sync_pending_reason(&promote.hash())
            .unwrap()
            .as_deref(),
        Some("tier")
    );
    assert_eq!(role(b), DeviceRole::Mirror);
    b.store.sync_resolve_pending(&promote.hash(), true).unwrap();
    assert_eq!(role(b), DeviceRole::Holder);
    let demote = a.append(OpBody::DeviceRoleChange {
        device_id: mirror.dev(),
        to: DeviceRole::Mirror,
    });
    pull(b, a);
    b.store.sync_resolve_pending(&demote.hash(), true).unwrap();
    assert_eq!(role(b), DeviceRole::Mirror);
}

#[test]
fn ceremony_records_are_green_and_bounded() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    for body in [
        OpBody::RedApprove { op_hash: [1; 32] },
        OpBody::RedPhoneSign { op_hash: [1; 32] },
        OpBody::RedFactor {
            op_hash: [1; 32],
            factor: ThirdFactor::Paper,
            sig: [2; 64],
        },
    ] {
        let op = a.append(body);
        assert_eq!(op.tier(), OpTier::Green);
        assert!(matches!(
            b.store.sync_ingest(op).unwrap(),
            IngestOutcome::Applied
        ));
    }
    let reg = |factor, sealed: Vec<u8>| OpBody::FactorRegister {
        factor,
        public: [3; 32],
        sealed,
        salt: [4; 16],
        sender: None,
    };
    assert_eq!(
        a.append(reg(ThirdFactor::Paper, vec![])).tier(),
        OpTier::Red
    );
    assert!(a
        .store
        .sync_append_local(&a.id, reg(ThirdFactor::Paper, vec![1; 1501]))
        .is_err());
    assert!(a
        .store
        .sync_append_local(&a.id, reg(ThirdFactor::EmailCode, vec![0; 129]))
        .is_err());
    let export = |w| OpBody::BulkExport {
        requester: a.dev(),
        window_secs: w,
    };
    assert_eq!(a.append(export(3600)).tier(), OpTier::Red);
    assert!(a.store.sync_append_local(&a.id, export(3601)).is_err());
    assert!(a.store.sync_append_local(&a.id, export(0)).is_err());
}

#[test]
fn exemption_set_projects_the_flag_and_the_newest_op_wins() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let slot = DeviceIdentity::generate().public();
    let exempt = |p: &Peer| {
        p.store
            .sync_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == a.dev())
            .unwrap()
            .exempt
    };
    let on = a.append(OpBody::ExemptionSet {
        device_id: a.dev(),
        slot: Some(slot),
    });
    assert_eq!(on.tier(), OpTier::Red);
    assert!(exempt(a));
    assert!(matches!(
        b.store.sync_ingest(on.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(!exempt(b), "waits for the ceremony");
    b.store.sync_resolve_pending(&on.hash(), true).unwrap();
    assert!(exempt(b));
    let off = a.append(OpBody::ExemptionSet {
        device_id: a.dev(),
        slot: None,
    });
    b.store.sync_ingest(off.clone()).unwrap();
    b.store.sync_resolve_pending(&off.hash(), true).unwrap();
    assert!(!exempt(a) && !exempt(b));
    // An invalid slot key is refused before it is chained.
    let bad = DevicePublic {
        ed25519_pub: [0; 32],
        x25519_pub: [0; 32],
        mlkem_pub: vec![1, 2, 3],
    };
    assert!(a
        .store
        .sync_append_local(
            &a.id,
            OpBody::ExemptionSet {
                device_id: a.dev(),
                slot: Some(bad)
            }
        )
        .is_err());
}

#[test]
fn cert_extend_is_green_inert_and_bounded() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let op = a.append(OpBody::CertExtend {
        cert_id: [1; 32],
        extra_hosts: 10,
    });
    assert_eq!(op.tier(), OpTier::Green);
    assert!(matches!(
        b.store.sync_ingest(op).unwrap(),
        IngestOutcome::Applied
    ));
    for n in [0u32, 51] {
        assert!(a
            .store
            .sync_append_local(
                &a.id,
                OpBody::CertExtend {
                    cert_id: [1; 32],
                    extra_hosts: n
                }
            )
            .is_err());
    }
}

// ------------------------------------------------------------------ join anchor

fn host_n(i: u32) -> OpBody {
    let mut u = [0u8; 16];
    u[..4].copy_from_slice(&i.to_be_bytes());
    OpBody::HostUpsert {
        host_uid: u,
        fields: hf(&format!("h{i}"), "10.0.0.1"),
    }
}

#[test]
fn a_joiner_adopts_the_anchor_reviews_nothing_below_the_cut_and_only_what_comes_after() {
    let ring = live_ring(vec![Peer::new(), Peer::new()]);
    let (a, b) = (&ring[0], &ring[1]);
    for i in 0..500 {
        a.append(host_n(i));
    }
    touch(&ring);
    settle(&ring);
    // A real checkpoint is not an anchor either.
    let (cp, _) = checkpoint_round(&ring, 0, crate::store::now_ts());
    let joiner = Peer::new();
    a.append(add_device_body(&joiner, DeviceRole::Mirror));
    let bundle = a.store.sync_join_anchor_build(&a.id).unwrap();

    // Only as a join anchor, and only with the inviter the user verified.
    for wrong in [
        BootstrapTrust::Enrollment { inviter: b.dev() },
        BootstrapTrust::Anchor(a.dev()),
        BootstrapTrust::RingFingerprint(ring_fingerprint(&bundle.state).unwrap()),
    ] {
        assert!(
            joiner.store.sync_join_adopt(&bundle, &wrong).is_err(),
            "{wrong:?}"
        );
    }
    assert!(joiner
        .store
        .sync_bootstrap(&bundle, &BootstrapTrust::Anchor(a.dev()))
        .unwrap_err()
        .to_string()
        .contains("join anchor"));
    assert!(joiner
        .store
        .sync_join_adopt(&cp, &BootstrapTrust::Enrollment { inviter: a.dev() })
        .unwrap_err()
        .to_string()
        .contains("prune checkpoint"));
    // Nor does a join anchor pass as a checkpoint to anyone else.
    assert!(b.store.sync_checkpoint_record(&bundle).is_err());

    joiner
        .store
        .sync_join_adopt(&bundle, &BootstrapTrust::Enrollment { inviter: a.dev() })
        .unwrap();
    assert_eq!(
        joiner.pending(),
        0,
        "the joiner reviews nothing below the cut"
    );
    assert_eq!(joiner.store.list_hosts().unwrap().len(), 502);
    let devices: Vec<_> = joiner
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    for d in [a.dev(), b.dev(), joiner.dev()] {
        assert!(devices.contains(&d), "{d}");
    }
    assert!(joiner.store.sync_checkpoints().unwrap().is_empty());

    // A dangerous change made after the cut is a foreign change under review.
    let dangerous = HostFields {
        address: Some("6.6.6.6".into()),
        ..Default::default()
    };
    let mut u = [0u8; 16];
    u[..4].copy_from_slice(&3u32.to_be_bytes());
    let op = b.append(OpBody::HostUpsert {
        host_uid: u,
        fields: dangerous,
    });
    pull(&joiner, b);
    let pending = joiner.store.sync_pending_with_reasons().unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].0.hash(), op.hash());
    assert_eq!(pending[0].1, "tier");
}

#[test]
fn an_op_of_an_unknown_author_waits_with_its_reason_and_never_takes_effect() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let newcomer = Peer::new();
    let op = newcomer.append(upsert(5, hf("n", "10.0.0.5")));
    assert!(matches!(
        b.store.sync_ingest(op.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    let pending = b.store.sync_pending_with_reasons().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1, "unknown-author");
    assert_eq!(b.store.sync_pending().unwrap().len(), 1);
    // Nothing of it exists: no host, no chain, nothing to serve, no fork evidence.
    assert!(b.host(&uid(5)).is_none());
    assert!(b
        .store
        .sync_chain_heads()
        .unwrap()
        .iter()
        .all(|(d, _, _)| *d != newcomer.dev()));
    assert!(b
        .store
        .sync_chain_tail(&newcomer.dev(), 0)
        .unwrap()
        .is_empty());
    assert!(rejected(&b.store.sync_ingest(op.clone()).unwrap()).contains("duplicate"));
    // Approving before the author is a device is refused; rejecting forgets the op.
    assert!(b
        .store
        .sync_resolve_pending(&op.hash(), true)
        .unwrap_err()
        .to_string()
        .contains("not a device of the ring yet"));
    b.store.sync_resolve_pending(&op.hash(), false).unwrap();
    assert_eq!(b.pending(), 0);

    // Once the author is admitted, the op goes through the ordinary checks.
    let add = a.append(add_device_body(&newcomer, DeviceRole::Mirror));
    b.store.sync_ingest(add.clone()).unwrap();
    b.store.sync_resolve_pending(&add.hash(), true).unwrap();
    assert!(matches!(
        b.store.sync_ingest(op).unwrap(),
        IngestOutcome::Pending
    ));
    let reasons: Vec<String> = b
        .store
        .sync_pending_with_reasons()
        .unwrap()
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(reasons, vec!["tier".to_string()]);
}

// ------------------------------------------------------- review round 1 regressions

fn validates(body: &OpBody) -> Result<(), String> {
    crate::sync::apply::validate_body(body)
}

#[test]
fn control_characters_never_enter_names_text_or_device_names() {
    let host = |name: &str, notes: Option<&str>| OpBody::HostUpsert {
        host_uid: uid(1),
        fields: HostFields {
            name: Some(name.into()),
            notes: notes.map(|n| Some(n.to_string())),
            ..Default::default()
        },
    };
    assert!(validates(&host("web", Some("two\nlines\tand a tab"))).is_ok());
    // A name that would write ssh_config directives or move a terminal cursor.
    for bad in [
        "hop\nMatch all",
        "hop\r",
        "a\u{1b}[2Kb",
        "a\u{7f}b",
        "a\u{85}b",
        "a\u{9b}31m",
        "a\tb",
    ] {
        assert!(validates(&host(bad, None)).is_err(), "{bad:?}");
    }
    for bad in ["x\u{1b}]52;c;ZXZpbA==\u{7}", "x\ry", "x\u{85}y"] {
        assert!(validates(&host("web", Some(bad))).is_err(), "{bad:?}");
    }
    let add = |name: &str| add_device_body_named(name);
    assert!(validates(&add("laptop")).is_ok());
    assert!(validates(&add("lap\ntop\nBcc: x")).is_err());
}

fn add_device_body_named(name: &str) -> OpBody {
    OpBody::DeviceAdd {
        device: Peer::new().id.public(),
        name: name.into(),
        role: DeviceRole::Mirror,
        attendance: Attendance::Attended,
        kind: DeviceKind::Terminal,
        verified_by: VerifiedBy::Sas,
    }
}

#[test]
fn head_attests_name_a_device_by_sixteen_hex_digits() {
    let attest = |id: &str| OpBody::HeadAttest {
        device_id: id.into(),
        seq: 1,
        chain_hash: [1; 32],
    };
    assert!(validates(&attest("0011223344556677")).is_ok());
    for bad in [
        "zz",
        "0011223344556677 ",
        "0011223344556G77",
        "00112233445566771",
        "AABBCCDDEEFF0011",
    ] {
        assert!(validates(&attest(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn sealed_and_credential_fields_have_byte_caps() {
    let key = |n| OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![0; n],
    };
    assert!(validates(&key(8 * 1024)).is_ok());
    assert!(validates(&key(8 * 1024 + 1)).is_err());
    let session = OpBody::ShareReshareSession {
        epoch: 1,
        to: "0011223344556677".into(),
        sealed: vec![0; 9000],
    };
    assert!(validates(&session).is_err());
    let rot = OpBody::WrapKeyRotate {
        device_id: "0011223344556677".into(),
        wrap_epoch: 1,
        x25519_pub: [0; 32],
        mlkem_pub: vec![0; 3000],
    };
    assert!(validates(&rot).is_err());
    let cred = OpBody::CredUpdate {
        host_uid: uid(1),
        ciphertext: vec![0; 129 * 1024],
        wrapped_shares: vec![],
    };
    assert!(validates(&cred).is_err());
    let confirm = OpBody::ShareReshareConfirmBatch {
        epoch: 1,
        items: (0..200)
            .map(|i| ConfirmItem {
                host_uid: uid(i as u8),
                sealed_share: vec![0; 4000],
            })
            .collect(),
    };
    assert!(validates(&confirm).is_err());
}

#[test]
fn only_a_holder_writes_share_machinery_and_a_promotion_releases_what_waited() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let c = Peer::new();
    for p in [a, b] {
        p.store
            .sync_register_device(
                &c.id.public(),
                "c",
                DeviceRole::Mirror,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    let key = c.append(OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![1; 40],
    });
    assert!(matches!(
        a.store.sync_ingest(key.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert_eq!(
        a.store.sync_pending_reason(&key.hash()).unwrap().as_deref(),
        Some("not-holder")
    );
    // A holder's own op of the same kind is not held.
    let own = b.append(OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![1; 40],
    });
    assert!(matches!(
        a.store.sync_ingest(own).unwrap(),
        IngestOutcome::Applied
    ));
    // Promoting c settles what it wrote before.
    a.append(OpBody::DeviceRoleChange {
        device_id: c.dev(),
        to: DeviceRole::Holder,
    });
    assert_eq!(
        a.pending(),
        0,
        "{:?}",
        a.store.sync_pending_with_reasons().unwrap()
    );
}

#[test]
fn a_delegate_does_not_vouch_for_its_own_certificate() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let issue = a.append(OpBody::CertIssue {
        device_id: b.dev(),
        scope: CertScope::Group(uid(20)),
        expiry: 4_000_000_000,
        max_new_hosts_per_day: 50,
        epoch_lo: 0,
        epoch_hi: 9,
    });
    let opened = b.append(OpBody::CertHostOpened {
        cert_id: issue.hash(),
        host_uid: uid(7),
    });
    pull(a, b);
    assert_eq!(
        a.store
            .sync_pending_reason(&opened.hash())
            .unwrap()
            .as_deref(),
        Some("not-holder")
    );
    assert!(a
        .store
        .sync_cert_hosts_opened(&issue.hash())
        .unwrap()
        .is_empty());
    // Another holder may open hosts under it.
    let other = a.append(OpBody::CertHostOpened {
        cert_id: issue.hash(),
        host_uid: uid(8),
    });
    let _ = other;
    assert_eq!(
        a.store.sync_cert_hosts_opened(&issue.hash()).unwrap(),
        vec![uid(8)]
    );
}

#[test]
fn ops_dated_beyond_the_skew_grace_are_refused_when_red_and_wait_otherwise() {
    use crate::sync::limits::TIMESTAMP_SKEW_GRACE_SECS;
    let ring = ring(3);
    let (a, b, c) = (&ring[0], &ring[1], &ring[2]);
    let now = crate::store::now_ts();
    // Red: a ceremony TTL measured on such a stamp would never end.
    let red = a
        .store
        .sync_append_local_at(
            &a.id,
            OpBody::DeviceRoleChange {
                device_id: c.dev(),
                to: DeviceRole::Mirror,
            },
            now + 400 * 86_400,
        )
        .unwrap();
    assert!(rejected(&b.store.sync_ingest(red).unwrap()).contains("dated"));
    // Green: held with its reason, and applied once the clock is within the grace.
    let green = a
        .store
        .sync_append_local_at(
            &a.id,
            upsert(
                5,
                HostFields {
                    notes: Some(Some("n".into())),
                    ..Default::default()
                },
            ),
            now + 2 * TIMESTAMP_SKEW_GRACE_SECS,
        )
        .unwrap();
    assert!(matches!(
        b.store.sync_ingest(green.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert_eq!(
        b.store
            .sync_pending_reason(&green.hash())
            .unwrap()
            .as_deref(),
        Some("future-dated")
    );
    // Inside the grace nothing changes.
    let fine = a
        .store
        .sync_append_local_at(
            &a.id,
            upsert(6, hf("ok", "10.0.0.6")),
            now + TIMESTAMP_SKEW_GRACE_SECS - 5,
        )
        .unwrap();
    assert_ne!(fine.timestamp, 0);
    assert!(!matches!(
        b.store.sync_ingest(fine).unwrap(),
        IngestOutcome::Rejected(_)
    ));
}

#[test]
fn a_rename_onto_a_name_another_entity_holds_waits_for_review() {
    let ring = ring(2);
    let (a, b) = (&ring[0], &ring[1]);
    let first = a.append(upsert(1, hf("prod", "p.example")));
    let second = a.append(upsert(2, hf("test", "q.example")));
    for o in [&first, &second] {
        b.store.sync_ingest((*o).clone()).unwrap();
    }
    b.approve_all();
    let rename = a.append(upsert(
        2,
        HostFields {
            name: Some("prod".into()),
            ..Default::default()
        },
    ));
    assert_eq!(rename.tier(), OpTier::Green);
    assert!(matches!(
        b.store.sync_ingest(rename.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert_eq!(
        b.store
            .sync_pending_reason(&rename.hash())
            .unwrap()
            .as_deref(),
        Some("tier")
    );
    // A rename to a free name stays automatic.
    let free = a.append(upsert(
        2,
        HostFields {
            name: Some("staging".into()),
            ..Default::default()
        },
    ));
    assert!(matches!(
        b.store.sync_ingest(free).unwrap(),
        IngestOutcome::Applied | IngestOutcome::Pending
    ));
}

#[test]
fn an_unverified_op_never_blocks_a_slot_and_one_claimed_author_holds_only_a_few() {
    let ring = ring(1);
    let b = &ring[0];
    let stranger = Peer::new();
    let ops: Vec<Op> = (0..10)
        .map(|i| stranger.append(upsert(10 + i as u8, hf("x", "10.0.0.9"))))
        .collect();
    // Same slot, different content: the newer claim replaces the older one.
    let mut squat = ops[0].clone();
    squat.body = upsert(99, hf("squat", "10.0.0.66"));
    assert!(matches!(
        b.store.sync_ingest(squat.clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert!(matches!(
        b.store.sync_ingest(ops[0].clone()).unwrap(),
        IngestOutcome::Pending
    ));
    assert_eq!(b.store.sync_pending().unwrap().len(), 1);
    assert_eq!(b.store.sync_pending().unwrap()[0].hash(), ops[0].hash());
    // One claimed author fills at most a few places.
    let mut held = 1;
    for op in &ops[1..] {
        if matches!(
            b.store.sync_ingest(op.clone()).unwrap(),
            IngestOutcome::Pending
        ) {
            held += 1;
        }
    }
    assert_eq!(held, 8);
}

#[test]
fn an_oversized_op_is_refused_even_from_an_unknown_author() {
    use crate::sync::oplog::MAX_OP_BYTES;
    let ring = ring(1);
    let b = &ring[0];
    let stranger = Peer::new();
    let mut op = stranger.append(upsert(1, hf("x", "10.0.0.9")));
    op.body = OpBody::ShareStorageKey {
        epoch: 1,
        sealed: vec![7; MAX_OP_BYTES],
    };
    assert!(rejected(&b.store.sync_ingest(op).unwrap()).contains("larger"));
    const { assert!(MAX_OP_BYTES / 3 * 4 + 8 * 1024 < crate::sync::limits::MAX_FRAME_BYTES) };
}

#[test]
fn a_bundle_over_the_cap_is_not_parsed() {
    let big = vec![b' '; crate::sync::protocol::MAX_CHECKPOINT_BYTES as usize + 1];
    let e = crate::sync::oplog::checkpoint::bundle_from_bytes(&big).unwrap_err();
    assert!(e.to_string().contains("over"), "{e:#}");
}

#[test]
fn an_existing_device_does_not_bootstrap_an_anchor_and_a_fresh_one_does_not_adopt() {
    let ring = live_ring(vec![Peer::new(), Peer::new()]);
    touch(&ring);
    settle(&ring);
    let (cp, _) = checkpoint_round(&ring, 0, crate::store::now_ts());
    let fresh = Peer::new();
    let e = fresh
        .store
        .sync_bootstrap(&cp, &BootstrapTrust::Anchor(ring[0].dev()))
        .unwrap_err();
    assert!(e.to_string().contains("adopt"), "{e:#}");
}
