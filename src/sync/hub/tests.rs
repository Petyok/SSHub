//! Hub rules: an unattended holder releases only under a certificate, recovery events are announced
//! once and without codes, and courier ops are ingested strictly.

use std::cell::RefCell;

use super::*;
use crate::sync::cert::DelegationCert;
use crate::sync::cred_ops::cred_put;
use crate::sync::oplog::{Attendance, CertScope, DeviceKind, DeviceRole, ThirdFactor, VerifiedBy};
use crate::sync::protocol::tests::{ring, Dev};
use crate::sync::quorum::tests::{all_to_all, append, deliver, payload, quorum_mode};
use crate::sync::quorum::{own_shares, release_share_for, Grant};

fn eph() -> [u8; 32] {
    x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([9u8; 32])).to_bytes()
}

#[test]
fn an_unattended_holder_releases_only_under_a_valid_certificate() {
    // 0 = phone, 1 = terminal that wants a share, 2 = unattended holder (the VPS), 3 = attended holder
    let d = ring(4);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [61u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("x")).unwrap();
    all_to_all(&d);
    // The VPS knows itself as unattended.
    d[2].store
        .sync_register_device(
            &d[2].id.public(),
            "vps",
            DeviceRole::Holder,
            Attendance::Unattended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();

    let now = crate::store::now_ts();
    let op = append(
        &d[0],
        OpBody::CertIssue {
            device_id: d[1].did.clone(),
            scope: CertScope::Hosts(vec![host]),
            expiry: now + 7 * 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 9,
        },
    );
    all_to_all(&d);
    let cert = DelegationCert::from_op(&op).unwrap();
    let issuer = d[0].id.public().ed25519_pub;
    let at = op.timestamp + 5;

    // A prompt on an unattended machine is no consent.
    let denied = release_share_for(
        &d[2].store,
        &d[2].id,
        &host,
        0,
        &eph(),
        &d[1].did,
        Grant::Approved,
    );
    assert!(denied.unwrap_err().to_string().contains("unattended"));
    // An attended holder releases on approval.
    assert!(release_share_for(
        &d[3].store,
        &d[3].id,
        &host,
        0,
        &eph(),
        &d[1].did,
        Grant::Approved
    )
    .is_ok());

    // Under the certificate the unattended holder releases, and the first release for a host is recorded.
    let grant = || Grant::Cert {
        cert: &cert,
        issuer_ed25519: issuer,
        phone_head_ts: at,
        now: at,
    };
    let (reply, opened) =
        release_share_for(&d[2].store, &d[2].id, &host, 0, &eph(), &d[1].did, grant()).unwrap();
    assert!(!reply.is_empty());
    let opened = opened.expect("a new host is recorded");
    append(&d[2], opened);
    all_to_all(&d);
    let (_, again) =
        release_share_for(&d[2].store, &d[2].id, &host, 0, &eph(), &d[1].did, grant()).unwrap();
    assert!(again.is_none(), "an opened host is free and recorded once");
    // The certificate is for one requester and one scope.
    assert!(
        release_share_for(&d[2].store, &d[2].id, &host, 0, &eph(), &d[3].did, grant()).is_err()
    );
    let other_host = [62u8; 16];
    assert!(release_share_for(
        &d[2].store,
        &d[2].id,
        &other_host,
        0,
        &eph(),
        &d[1].did,
        grant()
    )
    .is_err());
    let _ = own_shares;
}

fn reenroll_body(via_paper: bool) -> OpBody {
    OpBody::PhoneReenroll {
        initiated_at: crate::store::now_ts(),
        signers: vec![],
        via_paper,
        third_factor: if via_paper {
            ThirdFactor::EmailCode
        } else {
            ThirdFactor::Paper
        },
        liveness: vec![],
        paper_sig: None,
    }
}

#[test]
fn recovery_ops_from_others_are_announced_once_without_codes_and_failures_retry() {
    let d = ring(3);
    let started = append(&d[0], reenroll_body(false));
    deliver(&d[0], &d[1]);
    // The author announces nothing about its own op.
    assert!(pending_announcements(&d[0].store).unwrap().is_empty());

    let sent: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
    let mut failing = |_: &str, _: &str| -> Result<()> { bail!("mail server down") };
    let r = announce_recovery_ops(&d[1].store, &mut failing).unwrap();
    assert_eq!((r.sent, r.failed), (0, 1), "a failed send is not marked");
    let mut ok = |s: &str, b: &str| -> Result<()> {
        sent.borrow_mut().push((s.into(), b.into()));
        Ok(())
    };
    assert_eq!(announce_recovery_ops(&d[1].store, &mut ok).unwrap().sent, 1);
    assert_eq!(
        announce_recovery_ops(&d[1].store, &mut ok).unwrap().sent,
        0,
        "at most once per op"
    );
    let (subject, body) = sent.borrow()[0].clone();
    assert!(subject.contains("phone replacement was started"));
    assert!(body.contains(&hex::encode(&started.hash()[..4])) && body.contains(&d[0].did));
    assert!(
        body.contains("UTC") && !body.to_lowercase().contains("code:"),
        "{body}"
    );

    // A cancellation is announced as its own op.
    append(
        &d[0],
        OpBody::PhoneReenrollCancel {
            reenroll_hash: started.hash(),
            by_phone: true,
            third_factor: None,
        },
    );
    deliver(&d[0], &d[1]);
    assert_eq!(announce_recovery_ops(&d[1].store, &mut ok).unwrap().sent, 1);
    assert!(sent.borrow()[1].0.contains("cancelled"));
}

#[test]
fn courier_ops_are_ingested_strictly_and_the_notice_is_the_stored_op() {
    let d = ring(2);
    let phone_op = append(
        &d[0],
        OpBody::HeadAttest {
            device_id: d[0].did.clone(),
            seq: 1,
            chain_hash: [3; 32],
        },
    );
    let wire = vec![(phone_op.canonical_bytes(), phone_op.signature)];
    let sum = courier_ingest(&d[1].store, &wire).unwrap();
    assert_eq!((sum.received, sum.applied), (1, 1));
    assert_eq!(courier_ingest(&d[1].store, &wire).unwrap().duplicates, 1);
    assert!(
        courier_ingest(&d[1].store, &[(b"{}".to_vec(), [0; 64])]).is_err(),
        "malformed op"
    );
    assert!(
        courier_ingest(&d[1].store, &vec![wire[0].clone(); MAX_COURIER_OPS + 1]).is_err(),
        "too many ops"
    );

    let started = append(&d[0], reenroll_body(false));
    deliver(&d[0], &d[1]);
    let notice = recovery_notice(&d[1].store, &started.hash()).unwrap();
    assert_eq!(notice.op_canonical, started.canonical_bytes());
    assert_eq!(notice.op_signature, started.signature);
    assert!(
        recovery_notice(&d[1].store, &phone_op.hash()).is_err(),
        "only a PhoneReenroll"
    );
    let _ = Dev::new;
}

#[test]
fn utc_formats_known_instants() {
    assert_eq!(utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(utc(1_709_210_096), "2024-02-29 12:34 UTC");
    assert_eq!(utc(-86_400), "1969-12-31 00:00 UTC");
}
