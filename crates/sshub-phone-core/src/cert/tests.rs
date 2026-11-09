//! Ports of the trust-session tests of the page (`phone3.test.mjs`, `review-app-round1.test.mjs` F5b and F7,
//! `review-app-round2.test.mjs` "sound: certificate epoch range").

use std::collections::BTreeMap;

use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
use sshub_sync_proto::crypto::chain_hash;
use sshub_sync_proto::oplog::{canonical::decode_op, CertScope, Op, OpBody};
use sshub_sync_proto::wire::{
    CertExtendRequestWire, CertRenewRequestWire, CertRequestWire, HeadEvidence, PhoneOpsWire,
};

use super::*;
use crate::views::Outgoing;

const NOW: i64 = 1_893_452_400;
const DAY: i64 = 86_400;

struct Fx {
    id: PhoneIdentity,
    state: PhoneState,
    rng: ChaCha8Rng,
}

impl Fx {
    fn new() -> Self {
        Self {
            id: PhoneIdentity::new(&[0x42; 32], 0).unwrap(),
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
}

fn delegate() -> [u8; 8] {
    hex::decode(PhoneIdentity::new(&[0x71; 32], 0).unwrap().device_id())
        .unwrap()
        .try_into()
        .unwrap()
}

fn delegate_hex() -> String {
    hex::encode(delegate())
}

fn req() -> CertRequestWire {
    CertRequestWire {
        device_id: delegate(),
        scope: CertScope::Hosts(vec![[0x0a; 16], [0x0b; 16]]),
        expiry: NOW + 7 * DAY,
        max_new_hosts_per_day: 2,
        epoch_lo: 0,
        epoch_hi: 1,
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

fn issue(fx: &mut Fx, r: &CertRequestWire, now: i64) -> Outgoing {
    approve_cert(
        PendingCert::Request(r.clone()),
        Some(&hex::encode(r.device_id)),
        &mut fx.ctx(now),
    )
    .unwrap()
}

fn cert_view(o: Opened) -> CertRequestView {
    match o {
        Opened::CertRequest { view } => view,
        other => panic!("not a cert request view: {other:?}"),
    }
}

fn extend_view(o: Opened) -> CertExtendView {
    match o {
        Opened::CertExtend { view } => view,
        other => panic!("not a cert extend view: {other:?}"),
    }
}

fn refusal(o: Opened) -> String {
    match o {
        Opened::Refused { view } => view.reason,
        other => panic!("not a refusal: {other:?}"),
    }
}

fn first_cert_id(fx: &Fx) -> String {
    issued_certs(&fx.state, &BTreeMap::new(), NOW)[0]
        .cert_id
        .clone()
}

#[test]
fn the_phone_authors_cert_issue_with_the_request_fields_unchanged_chained_on_its_own_chain() {
    let mut fx = Fx::new();
    let (opened, pending) = open_cert_request(&req().encode(), &fx.ctx(NOW), 7);
    let v = cert_view(opened);
    assert_eq!(v.token, 7);
    assert!(v.problems.is_empty());
    assert!(!v.delegate_known);
    assert_eq!(
        v.type_to_approve.as_deref(),
        Some(delegate_hex().as_str()),
        "a delegate never met needs its id typed"
    );
    let pending = pending.expect("a clean request can be approved");

    // the typed id is checked here, in Rust, not on the screen
    assert!(approve_cert(pending.clone(), None, &mut fx.ctx(NOW)).is_err());
    assert!(approve_cert(pending.clone(), Some("aaaaaaaaaaaaaaaa"), &mut fx.ctx(NOW)).is_err());
    assert!(fx.state.chain.is_empty(), "a refusal writes nothing");
    let typed = format!("  {}  ", delegate_hex().to_uppercase());
    let out = approve_cert(pending, Some(&typed), &mut fx.ctx(NOW)).unwrap();

    assert_eq!(out.kind, 5);
    let ops = ops_of(&out);
    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0].body,
        req().to_body(),
        "the fields are the request's, unchanged"
    );
    assert_eq!(ops[0].device_id, fx.id.device_id());
    assert_eq!(ops[0].seq, 1);
    assert!(ops[0].verify(&fx.id.public().ed25519_pub));
    let canonical = &fx.state.chain[0].canonical;
    assert_eq!(
        first_cert_id(&fx),
        hex::encode(chain_hash(canonical)),
        "the certificate id is the op hash"
    );

    // a second one (group scope) chains on the first and both travel in one list
    let mut group = req();
    group.scope = CertScope::Group([6; 16]);
    let out2 = issue(&mut fx, &group, NOW + 5);
    let ops2 = ops_of(&out2);
    assert_eq!(ops2.len(), 2);
    assert_eq!(ops2[1].seq, 2);
    assert_eq!(ops2[1].prev_hash, fx.state.chain[0].hash32());
    assert!(
        matches!(&ops2[1].body, OpBody::CertIssue { scope: CertScope::Group(g), .. } if *g == [6; 16])
    );
    assert_eq!(issued_certs(&fx.state, &BTreeMap::new(), NOW).len(), 2);
}

#[test]
fn the_request_is_described_from_its_bytes_device_scope_term_budget() {
    let mut fx = Fx::new();
    fx.state
        .aliases
        .insert(delegate_hex(), "work laptop".to_string());
    let (opened, pending) = open_cert_request(&req().encode(), &fx.ctx(NOW), 1);
    let v = cert_view(opened);
    let all: String = v
        .rows
        .iter()
        .map(|r| format!("{}={}\n", r.label, r.value))
        .collect();
    assert!(
        all.contains(&format!("work laptop [{}]", delegate_hex())),
        "{all}"
    );
    assert!(
        all.contains(&"0a".repeat(16)) && all.contains(&"0b".repeat(16)),
        "every listed host is shown"
    );
    assert!(all.contains("2 hosts only"));
    assert!(all.contains("7 days"), "{all}");
    assert!(all.contains("New hosts per day=2"));
    assert!(v.delegate_known);
    assert_eq!(v.type_to_approve, None);
    // a known delegate needs no typed id
    approve_cert(pending.unwrap(), None, &mut fx.ctx(NOW)).unwrap();

    for r in v
        .rows
        .iter()
        .filter(|r| ["Trusted device", "Term", "New hosts per day"].contains(&r.label.as_str()))
    {
        assert!(r.danger, "{}", r.label);
    }
    let mut group = req();
    group.scope = CertScope::Group([6; 16]);
    let v = cert_view(open_cert_request(&group.encode(), &fx.ctx(NOW), 1).0);
    assert!(v
        .rows
        .iter()
        .any(|r| r.value == "06".repeat(16) && r.danger));
    assert!(v.headline.contains("the hosts of a group"));

    // a name from another machine never reaches the screen raw
    fx.state
        .aliases
        .insert(delegate_hex(), "a\u{202e}b".to_string());
    let v = cert_view(open_cert_request(&req().encode(), &fx.ctx(NOW), 1).0);
    assert!(v.headline.contains("a\\u202eb") && !v.headline.contains('\u{202e}'));
}

#[test]
fn every_limit_of_cert_issue_is_enforced_before_signing_and_a_refusal_writes_nothing() {
    let own = PhoneIdentity::new(&[0x42; 32], 0).unwrap();
    let own_bytes: [u8; 8] = hex::decode(own.device_id()).unwrap().try_into().unwrap();
    let with = |f: &dyn Fn(&mut CertRequestWire)| {
        let mut r = req();
        f(&mut r);
        r
    };
    let cases: Vec<(&str, CertRequestWire, &str)> = vec![
        (
            "expiry is now",
            with(&|r| r.expiry = NOW),
            "not in the future",
        ),
        (
            "expiry in the past",
            with(&|r| r.expiry = NOW - 5),
            "not in the future",
        ),
        (
            "90 days and a second",
            with(&|r| r.expiry = NOW + 90 * DAY + 1),
            "longer than 90 days",
        ),
        (
            "budget 0",
            with(&|r| r.max_new_hosts_per_day = 0),
            "between 1 and 50",
        ),
        (
            "budget 51",
            with(&|r| r.max_new_hosts_per_day = 51),
            "between 1 and 50",
        ),
        (
            "epochs inverted",
            with(&|r| (r.epoch_lo, r.epoch_hi) = (6, 5)),
            "inverted",
        ),
        (
            "names the phone itself",
            with(&|r| r.device_id = own_bytes),
            "does not delegate to itself",
        ),
        // hand-built shapes the wire decoder never lets through are refused by the phone too
        (
            "empty list",
            with(&|r| r.scope = CertScope::Hosts(vec![])),
            "1 to 64 hosts",
        ),
        (
            "65 hosts",
            with(&|r| r.scope = CertScope::Hosts(vec![[1; 16]; 65])),
            "1 to 64 hosts",
        ),
    ];
    for (name, r, want) in cases {
        let mut fx = Fx::new();
        let problems = cert_request_problems(&r, NOW, fx.id.device_id(), 0).join(" ");
        assert!(problems.contains(want), "{name}: {problems}");
        let err = approve_cert(
            PendingCert::Request(r.clone()),
            Some(&hex::encode(r.device_id)),
            &mut fx.ctx(NOW),
        )
        .unwrap_err();
        assert!(err.reason().contains(want), "{name}: {}", err.reason());
        assert!(fx.state.chain.is_empty(), "{name}: nothing was signed");
    }
    // through the wire: the view lists the problems and offers no approval
    let mut fx = Fx::new();
    let (opened, pending) = open_cert_request(
        &with(&|r| r.expiry = NOW + 90 * DAY + 1).encode(),
        &fx.ctx(NOW),
        1,
    );
    let v = cert_view(opened);
    assert!(v.problems.iter().any(|p| p.contains("longer than 90 days")));
    assert_eq!(v.type_to_approve, None);
    assert!(pending.is_none());
    // the wire decoder refuses lists of 0 and 65 hosts
    for n in [0usize, 65] {
        let frame = with(&|r| r.scope = CertScope::Hosts(vec![[1; 16]; n])).encode();
        assert!(
            refusal(open_cert_request(&frame, &fx.ctx(NOW), 1).0)
                .starts_with("Not a trust-session request"),
            "{n} hosts"
        );
    }
    assert!(open_cert_request(
        &with(&|r| r.scope = CertScope::Hosts(vec![[1; 16]; 64])).encode(),
        &fx.ctx(NOW),
        1
    )
    .1
    .is_some());

    // the edges themselves are accepted
    let edges: Vec<CertRequestWire> = vec![
        with(&|r| r.expiry = NOW + 1),
        with(&|r| r.expiry = NOW + 90 * DAY),
        with(&|r| r.max_new_hosts_per_day = 1),
        with(&|r| r.max_new_hosts_per_day = 50),
        with(&|r| (r.epoch_lo, r.epoch_hi) = (0, 0)),
    ];
    for r in edges {
        let mut fx = Fx::new();
        issue(&mut fx, &r, NOW);
        assert_eq!(fx.state.chain.len(), 1);
    }
}

#[test]
fn the_limits_are_checked_again_at_signing_time() {
    let mut fx = Fx::new();
    let (_, pending) = open_cert_request(&req().encode(), &fx.ctx(NOW), 1);
    // the person takes a week to decide: the term has ended by then
    let err = approve_cert(
        pending.unwrap(),
        Some(&delegate_hex()),
        &mut fx.ctx(NOW + 8 * DAY),
    )
    .unwrap_err();
    assert!(err.reason().contains("not in the future"));
    assert!(fx.state.chain.is_empty());
}

#[test]
fn f7_the_range_must_start_at_this_phones_epoch_and_be_at_most_one_wider() {
    let own = "dddddddddddddddd";
    let range = |lo: u64, hi: u64| {
        let mut r = req();
        r.epoch_lo = lo;
        r.epoch_hi = hi;
        r
    };
    assert!(cert_request_problems(&range(3, 4), NOW, own, 3).is_empty());
    assert!(cert_request_problems(&range(3, 3), NOW, own, 3).is_empty());
    assert!(cert_request_problems(&range(0, 1), NOW, own, 3)
        .join(" ")
        .contains("this phone is at epoch 3"));
    assert!(cert_request_problems(&range(3, 5), NOW, own, 3)
        .join(" ")
        .contains("wider"));
    assert!(cert_request_problems(&range(2, 3), NOW, own, 3)
        .join(" ")
        .contains("this phone is at epoch 3"));
    assert!(cert_request_problems(&range(0, u64::MAX), NOW, own, 0)
        .join(" ")
        .contains("wider than a terminal asks for"));
    assert!(cert_request_problems(&range(u64::MAX, u64::MAX), NOW, own, u64::MAX).is_empty());
    // the unbounded range is highlighted too
    let (_, rows, _) = describe_cert_request(&range(0, u64::MAX), NOW, &BTreeMap::new());
    assert!(
        rows.iter()
            .find(|r| r.label.starts_with("Phone epochs"))
            .unwrap()
            .danger
    );

    let mut fx = Fx::new();
    fx.state.epoch = 3;
    let err = approve_cert(
        PendingCert::Request(range(0, 1)),
        Some(&delegate_hex()),
        &mut fx.ctx(NOW),
    )
    .unwrap_err();
    assert!(err.reason().contains("epoch"));
    assert!(fx.state.chain.is_empty());
    issue(&mut fx, &range(3, 4), NOW);
    assert_eq!(
        decode_op(
            &fx.state.chain[0].canonical,
            fx.state.chain[0].signature64()
        )
        .unwrap()
        .epoch,
        3
    );
}

#[test]
fn f5_hostile_input_never_panics_and_a_far_future_expiry_is_described() {
    for expiry in [9_000_000_000_000_000_000i64, i64::MAX, i64::MIN, 0, -1] {
        let mut r = req();
        r.expiry = expiry;
        assert!(
            !cert_request_problems(&r, NOW, "dddddddddddddddd", 0).is_empty(),
            "{expiry}"
        );
        let _ = describe_cert_request(&r, NOW, &BTreeMap::new());
        let _ = describe_cert_request(&r, i64::MIN, &BTreeMap::new());
        let _ = describe_cert_request(&r, i64::MAX, &BTreeMap::new());
    }
    let mut fx = Fx::new();
    let ok = req().encode();
    for n in 0..ok.len() {
        assert!(
            refusal(open_cert_request(&ok[..n], &fx.ctx(NOW), 1).0)
                .starts_with("Not a trust-session request"),
            "prefix {n}"
        );
    }
    let mut tag = ok.clone();
    tag[15] = 7;
    assert!(refusal(open_cert_request(&tag, &fx.ctx(NOW), 1).0).contains("scope tag"));
    let ext = CertExtendRequestWire {
        cert_id: [0xab; 32],
        extra_hosts: 7,
    }
    .encode();
    assert!(
        refusal(open_cert_request(&ext, &fx.ctx(NOW), 1).0)
            .starts_with("Not a trust-session request"),
        "a kind 10 frame is not a kind 9 request"
    );
    assert!(refusal(open_cert_extend(&ok, &fx.ctx(NOW), 1).0)
        .starts_with("Not a budget extension request"));
    for n in 0..ext.len() {
        assert!(
            refusal(open_cert_extend(&ext[..n], &fx.ctx(NOW), 1).0)
                .starts_with("Not a budget extension request"),
            "prefix {n}"
        );
    }
    let mut noise = ChaCha8Rng::seed_from_u64(99);
    for len in [0usize, 1, 6, 7, 8, 40, 200] {
        for _ in 0..20 {
            let mut b = vec![0u8; len];
            rand_core::RngCore::fill_bytes(&mut noise, &mut b);
            let _ = open_cert_request(&b, &fx.ctx(NOW), 1);
            let _ = open_cert_extend(&b, &fx.ctx(NOW), 1);
        }
    }
    // a damaged chain entry or an unrelated op does not break reading the issued list
    fx.state.chain.push(crate::state::ChainEntry {
        seq: 1,
        lamport: 1,
        timestamp: 1,
        hash: vec![0; 32],
        canonical: b"nonsense".to_vec(),
        signature: vec![0; 64],
    });
    assert!(issued_certs(&fx.state, &BTreeMap::new(), NOW).is_empty());
}

#[test]
fn a_trust_session_is_revoked_by_a_phone_signed_cert_revoke() {
    let mut fx = Fx::new();
    issue(&mut fx, &req(), NOW);
    let id = first_cert_id(&fx);
    let out = revoke_cert(&id, &mut fx.ctx(NOW + 100)).unwrap();
    let ops = ops_of(&out);
    let last = ops.last().unwrap();
    assert!(matches!(&last.body, OpBody::CertRevoke { cert_id } if hex::encode(cert_id) == id));
    assert_eq!(last.device_id, fx.id.device_id());
    assert!(last.verify(&fx.id.public().ed25519_pub));
    let list = issued_certs(&fx.state, &BTreeMap::new(), NOW);
    assert_eq!(list[0].status, CertStatus::Revoked);
    assert!(!list[0].can_revoke);
    assert!(list[0].summary.ends_with(", REVOKED"));
    assert!(revoke_cert(&id, &mut fx.ctx(NOW + 200))
        .unwrap_err()
        .reason()
        .contains("already revoked"));
    assert!(revoke_cert(&"ee".repeat(32), &mut fx.ctx(NOW))
        .unwrap_err()
        .reason()
        .contains("did not issue"));
    assert!(matches!(
        revoke_cert("nothex", &mut fx.ctx(NOW)),
        Err(CoreError::BadInput { .. })
    ));
    assert_eq!(fx.state.chain.len(), 2);
}

fn extend_frame(id: &str, extra: u32) -> Vec<u8> {
    CertExtendRequestWire {
        cert_id: hex::decode(id).unwrap().try_into().unwrap(),
        extra_hosts: extra,
    }
    .encode()
}

#[test]
fn cert_extend_is_phone_authored_one_per_rolling_24_hours_and_none_after_expiry() {
    let mut fx = Fx::new();
    issue(&mut fx, &req(), NOW);
    let id = first_cert_id(&fx);
    let frame = extend_frame(&id, 5);

    let (opened, pending) = open_cert_extend(&frame, &fx.ctx(NOW + 3600), 3);
    let v = extend_view(opened);
    assert_eq!(v.token, 3);
    assert!(v.problems.is_empty(), "{:?}", v.problems);
    assert!(v
        .rows
        .iter()
        .any(|r| r.label == "Extra new hosts for today" && r.value == "5" && r.danger));
    assert!(v.rows.iter().any(|r| r.label == "Remaining term"));
    let out = approve_cert(pending.unwrap(), None, &mut fx.ctx(NOW + 3600)).unwrap();
    let last = ops_of(&out).pop().unwrap();
    assert!(
        matches!(&last.body, OpBody::CertExtend { cert_id, extra_hosts: 5 } if hex::encode(cert_id) == id)
    );
    assert_eq!(last.device_id, fx.id.device_id());
    assert!(last.verify(&fx.id.public().ed25519_pub));
    assert!(issued_certs(&fx.state, &BTreeMap::new(), NOW)[0]
        .summary
        .contains("extended 1x"));

    // a second extension inside 24 hours is refused, with the time it becomes possible
    let (opened, pending) = open_cert_extend(&frame, &fx.ctx(NOW + 3600 + DAY - 1), 3);
    let v = extend_view(opened);
    assert!(v.problems.join(" ").contains("one per 24 hours"));
    assert!(pending.is_none());
    let req10 = CertExtendRequestWire {
        cert_id: hex::decode(&id).unwrap().try_into().unwrap(),
        extra_hosts: 5,
    };
    let err = approve_cert(
        PendingCert::Extend(req10.clone()),
        None,
        &mut fx.ctx(NOW + 7200),
    )
    .unwrap_err();
    assert!(err.reason().contains("one per 24 hours"));
    assert_eq!(fx.state.chain.len(), 2, "the refusal wrote nothing");
    // 24 hours later it is possible again (rolling window)
    let (opened, pending) = open_cert_extend(&frame, &fx.ctx(NOW + 3600 + DAY), 3);
    assert!(extend_view(opened).problems.is_empty());
    approve_cert(pending.unwrap(), None, &mut fx.ctx(NOW + 3600 + DAY)).unwrap();
    assert_eq!(fx.state.chain.len(), 3);

    // not after expiry, not for an unknown or revoked certificate, not outside 1..=50
    let (p, _) = extend_problems(&fx.state, &req10, NOW + 7 * DAY);
    assert!(p.join(" ").contains("expired"));
    let (opened, pending) = open_cert_extend(&extend_frame(&"ee".repeat(32), 1), &fx.ctx(NOW), 3);
    let v = extend_view(opened);
    assert!(v.problems.join(" ").contains("did not issue"));
    assert!(!v.rows.iter().any(|r| r.label == "Trusted device"));
    assert!(pending.is_none());
    for extra in [51u32, 0] {
        let r = CertExtendRequestWire {
            cert_id: req10.cert_id,
            extra_hosts: extra,
        };
        assert!(
            extend_problems(&fx.state, &r, NOW + 5 * DAY)
                .0
                .join(" ")
                .contains("1 to 50"),
            "{extra}"
        );
    }
    // the wire decoder refuses them before the phone sees a view
    assert!(refusal(
        open_cert_extend(
            &extend_frame(&id, 50)
                .iter()
                .copied()
                .chain([0])
                .collect::<Vec<_>>(),
            &fx.ctx(NOW),
            1
        )
        .0
    )
    .contains("36 bytes"));
    revoke_cert(&id, &mut fx.ctx(NOW + 5 * DAY)).unwrap();
    assert!(extend_problems(&fx.state, &req10, NOW + 6 * DAY)
        .0
        .join(" ")
        .contains("revoked"));
}

#[test]
fn the_issued_list_has_status_and_revoke_only_for_live_certificates() {
    let mut fx = Fx::new();
    fx.state
        .aliases
        .insert(delegate_hex(), "work laptop".to_string());
    issue(&mut fx, &req(), NOW);
    let aliases = fx.ctx(NOW).aliases();
    let live = issued_certs(&fx.state, &aliases, NOW);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].status, CertStatus::Active);
    assert!(live[0].can_revoke);
    assert!(
        live[0].summary.starts_with(&format!(
            "work laptop [{}]: 2 listed hosts, until ",
            delegate_hex()
        )),
        "{}",
        live[0].summary
    );
    assert!(live[0].summary.ends_with("2 new hosts per day"));
    let gone = issued_certs(&fx.state, &aliases, NOW + 7 * DAY);
    assert_eq!(gone[0].status, CertStatus::Expired);
    assert!(!gone[0].can_revoke);
    assert_eq!(describe_term(7 * DAY), "7 days");
    assert_eq!(describe_term(DAY + 3 * 3600), "1 day 3 h");
    assert_eq!(describe_term(2 * 3600 + 300), "2 h 5 min");
    assert_eq!(describe_term(420), "7 min");
    assert_eq!(describe_term(45), "45 s");
}

// ---------------------------------------------------------------- CertRenew (Phase B, contract 4j)

fn renew_req(fx: &Fx, expiry: i64, evidence_ts: &[(u8, i64)]) -> CertRenewRequestWire {
    CertRenewRequestWire {
        cert_id: hex::decode(first_cert_id(fx)).unwrap().try_into().unwrap(),
        expiry,
        evidence: evidence_ts
            .iter()
            .map(|(t, ts)| HeadEvidence {
                terminal: [*t; 8],
                head_seq: 5,
                head_ts: *ts,
            })
            .collect(),
    }
}

fn issued(term_days: i64) -> Fx {
    let mut fx = Fx::new();
    let mut r = req();
    r.expiry = NOW + term_days * DAY;
    issue(&mut fx, &r, NOW);
    fx
}

fn renew(fx: &mut Fx, r: &CertRenewRequestWire, now: i64) -> Result<Vec<u8>> {
    build_cert_renew(&mut fx.state, &fx.id, r, now)
}

fn renew_with(fx: &mut Fx, expiry: i64, ev: &[(u8, i64)], now: i64) -> Result<Vec<u8>> {
    let r = renew_req(fx, expiry, ev);
    renew(fx, &r, now)
}

#[test]
fn a_renewal_with_fresh_evidence_extends_the_end_that_counts() {
    let mut fx = issued(7);
    let now = NOW + 6 * DAY;
    let r = renew_req(&fx, NOW + 14 * DAY, &[(0x33, now - 3_600)]);
    let v = describe_cert_renew(&fx.state, &r, now).unwrap();
    assert_eq!(v.refusal, None);
    assert_eq!(
        (v.term_secs, v.budget, v.renewals_left, v.new_end),
        (7 * DAY, 2, 2, NOW + 14 * DAY)
    );
    assert_eq!(v.evidence_age_secs, Some(3_600));
    assert!(v.delegate.contains(&delegate_hex()) && v.scope == "2 listed hosts");
    let frame = renew(&mut fx, &r, now).unwrap();
    let ops: Vec<Op> = PhoneOpsWire::decode(&frame)
        .unwrap()
        .ops
        .iter()
        .map(|o| decode_op(&o.0, o.1).unwrap())
        .collect();
    assert!(matches!(
        ops.last().unwrap().body,
        OpBody::CertRenew { expiry, .. } if expiry == NOW + 14 * DAY
    ));
    let c = &issued_cert_records(&fx.state)[0];
    assert_eq!(
        (c.effective_expiry(), c.renewals.len()),
        (NOW + 14 * DAY, 1)
    );
    assert!(c.live(NOW + 10 * DAY), "alive after the first end date");
    let again = describe_cert_renew(
        &fx.state,
        &renew_req(&fx, NOW + 21 * DAY, &[(0x33, now)]),
        now,
    )
    .unwrap();
    assert_eq!(again.renewals_left, 1);
}

#[test]
fn a_renewal_is_refused_for_stale_evidence_or_a_list_that_came_only_through_the_delegate() {
    let mut fx = issued(7);
    let now = NOW + 6 * DAY;
    let end = NOW + 14 * DAY;
    let stale = renew_req(&fx, end, &[(0x33, now - 6 * 3_600 - 1)]);
    let e = renew(&mut fx, &stale, now).unwrap_err();
    assert!(e.reason().contains("fresher than 6 h"), "{}", e.reason());
    assert!(describe_cert_renew(&fx.state, &stale, now)
        .unwrap()
        .refusal
        .is_some());
    // at exactly six hours it is still inside
    assert!(renew_with(&mut fx, end, &[(0x33, now - 6 * 3_600)], now).is_ok());
    let mut fx = issued(7);
    let none = renew_req(&fx, end, &[]);
    assert!(renew(&mut fx, &none, now)
        .unwrap_err()
        .reason()
        .contains("only through the trusted device"));
    // the delegate's own id is not another terminal, however fresh
    let me = renew_req(&fx, end, &[(0x00, now)]);
    let mut me = me;
    me.evidence[0].terminal = delegate();
    assert!(renew(&mut fx, &me, now)
        .unwrap_err()
        .reason()
        .contains("only through the trusted device"));
    // an old terminal next to a fresh one: the freshest counts
    let mixed = renew_req(&fx, end, &[(0x33, now - 30 * DAY), (0x34, now - 60)]);
    assert_eq!(
        describe_cert_renew(&fx.state, &mixed, now)
            .unwrap()
            .evidence_age_secs,
        Some(60)
    );
    assert!(fx.state.chain.len() == 1, "refusals sign nothing");
}

#[test]
fn the_caps_hold_two_renewals_the_first_term_and_180_days() {
    let mut fx = issued(7);
    let now = NOW + DAY;
    let ev = [(0x33, now)];
    // longer than the first term, not later than now, in the past
    for (end, why) in [
        (NOW + 14 * DAY + 1, "at most as long as the first term"),
        (NOW + 7 * DAY, "not later than the current one"),
        (NOW + DAY, "not later than the current one"),
    ] {
        let e = renew_with(&mut fx, end, &ev, now).unwrap_err();
        assert!(e.reason().contains(why), "{end}: {}", e.reason());
    }
    renew_with(&mut fx, NOW + 14 * DAY, &ev, now).unwrap();
    renew_with(&mut fx, NOW + 21 * DAY, &ev, now).unwrap();
    let e = renew_with(&mut fx, NOW + 28 * DAY, &ev, now).unwrap_err();
    assert!(
        e.reason().contains("renewed 2 times already"),
        "{}",
        e.reason()
    );
    assert_eq!(
        describe_cert_renew(&fx.state, &renew_req(&fx, NOW + 28 * DAY, &ev), now)
            .unwrap()
            .renewals_left,
        0
    );
    // 180 days from the start, renewals included
    let mut fx = issued(90);
    renew_with(&mut fx, NOW + 180 * DAY, &ev, now).unwrap();
    let e = renew_with(&mut fx, NOW + 180 * DAY + 1, &ev, now).unwrap_err();
    assert!(
        e.reason().contains("180 days") || e.reason().contains("first term"),
        "{}",
        e.reason()
    );
}

#[test]
fn a_renewal_the_host_would_not_count_is_not_counted_by_the_phone_either() {
    // the chain holds a renewal past the cap (written by a modified phone): it does not move the end date
    let mut fx = issued(7);
    let id = hex::decode(first_cert_id(&fx)).unwrap().try_into().unwrap();
    chain::cert_renew(&mut fx.state, &fx.id, id, NOW + 30 * DAY, NOW, None).unwrap();
    let c = &issued_cert_records(&fx.state)[0];
    assert_eq!((c.effective_expiry(), c.renewals.len()), (NOW + 7 * DAY, 0));
}

#[test]
fn a_renewal_is_refused_for_a_revoked_unknown_or_unusable_state() {
    let ev = |now: i64| [(0x33u8, now)];
    let now = NOW + DAY;
    // revoked
    let mut fx = issued(7);
    let r = renew_req(&fx, NOW + 14 * DAY, &ev(now));
    revoke_cert(&first_cert_id(&fx), &mut fx.ctx(now)).unwrap();
    assert!(renew(&mut fx, &r, now)
        .unwrap_err()
        .reason()
        .contains("revoked"));
    // a certificate this phone did not issue
    let mut fx = issued(7);
    let mut r = renew_req(&fx, NOW + 14 * DAY, &ev(now));
    r.cert_id = [9; 32];
    let v = describe_cert_renew(&fx.state, &r, now).unwrap();
    assert!(v.refusal.unwrap().contains("did not issue"));
    assert!(renew(&mut fx, &r, now).is_err());
    // a restored phone without its chain head cannot sign
    let mut fx = issued(7);
    let r = renew_req(&fx, NOW + 14 * DAY, &ev(now));
    fx.state.head_pending = true;
    assert!(renew(&mut fx, &r, now)
        .unwrap_err()
        .reason()
        .contains("chain"));
    // a certificate whose epochs ended before this phone's epoch: shown refused at use (design L5)
    let mut fx = issued(7);
    fx.state.epoch = 5;
    let r = renew_req(&fx, NOW + 14 * DAY, &ev(now));
    let v = describe_cert_renew(&fx.state, &r, now).unwrap();
    assert!(v.refusal.unwrap().contains("refused at use"));
}

#[test]
fn a_kind_11_frame_opens_a_card_and_approval_signs_it() {
    let mut fx = issued(7);
    let now = NOW + DAY;
    let r = renew_req(&fx, NOW + 14 * DAY, &[(0x33, now - 600)]);
    let (o, p) = open_cert_renew(&r.encode(), &fx.ctx(now), 4);
    let Opened::CertRenew { view } = o else {
        panic!()
    };
    assert_eq!(view.token, 4);
    assert!(view.renew.refusal.is_none() && !view.note.is_empty());
    assert!(
        view.plain.starts_with("Keep the trust session of "),
        "{}",
        view.plain
    );
    assert!(view
        .rows
        .iter()
        .any(|r| r.label == "Freshest news from another terminal" && r.value == "10 min old"));
    let out = approve_cert(p.unwrap(), None, &mut fx.ctx(now)).unwrap();
    assert!(matches!(
        ops_of(&out).last().unwrap().body,
        OpBody::CertRenew { .. }
    ));
    // a stale card has no pending request: nothing can be approved
    let stale = renew_req(&fx, NOW + 21 * DAY, &[(0x33, now - 7 * 3_600)]);
    let (o, p) = open_cert_renew(&stale.encode(), &fx.ctx(now), 5);
    assert!(p.is_none());
    let Opened::CertRenew { view } = o else {
        panic!()
    };
    assert!(view.renew.refusal.is_some() && view.note.is_empty());
    assert!(refusal(open_cert_renew(&[1, 2, 3], &fx.ctx(now), 6).0)
        .starts_with("Not a renewal request"));
}
