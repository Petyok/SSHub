//! The Red ceremony over a real log: the three proofs, who may give them, expiry, blocking rules, the
//! single-machine ring, the emailed code.

use super::*;
use crate::sync::crypto::{bip39_encode, paper_derive, PaperDomain};
use crate::sync::oplog::VerifiedBy;
use crate::sync::protocol::tests::Dev;
use crate::sync::quorum::tests::append;

const R: usize = 0; // proposer terminal
const A: usize = 1; // attended terminal
const B: usize = 2; // attended terminal
const H: usize = 3; // unattended hub
const P: usize = 4; // phone
const OBS: usize = 5; // observer that judges

fn world(terminals: &[usize]) -> Vec<Dev> {
    let d: Vec<Dev> = (0..6).map(|_| Dev::new()).collect();
    for (i, x) in d.iter().enumerate() {
        for (j, y) in d.iter().enumerate() {
            if i == j || !(terminals.contains(&j) || j == H || j == P) {
                continue;
            }
            let kind = if j == P {
                DeviceKind::Phone
            } else {
                DeviceKind::Terminal
            };
            let attendance = if j == H {
                Attendance::Unattended
            } else {
                Attendance::Attended
            };
            x.trust_with(y, attendance, kind);
        }
    }
    d[H].store
        .sync_register_device(
            &d[H].id.public(),
            "hub",
            DeviceRole::Holder,
            Attendance::Unattended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    d
}

/// Ingest everything `from` has into `to` WITHOUT anyone clicking approve.
fn pull(from: &Dev, to: &Dev) {
    for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
        let have = to
            .store
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(x, _, _)| *x == dev)
            .map_or(0, |(_, s, _)| s);
        for op in from.store.sync_chain_tail(&dev, have).unwrap() {
            to.store.sync_ingest(op).unwrap();
        }
    }
}

fn sync(d: &[Dev]) {
    for x in d {
        for y in d {
            if x.did != y.did {
                pull(x, y);
            }
        }
    }
}

/// A proposal reaches everyone before anyone votes on it: proofs must come after the op they name.
fn propose(d: &[Dev], from: usize, body: OpBody) -> Op {
    let op = append(&d[from], body);
    sync(d);
    op
}

fn status(d: &[Dev], op: &Op, now: i64) -> RedStatus {
    red_status(&d[OBS].store, op, now).unwrap()
}

#[test]
fn a_gated_op_waits_for_all_three_proofs_from_the_right_participants() {
    let d = world(&[R, A, B]);
    let phrase = bip39_encode(&[0x31; 32]);
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();

    // Bootstrap: registering the first factor needs the phone and a second device, nothing else.
    let reg = propose(&d, R, factor_register_paper(&recover));
    append(&d[P], red_phone_sign_body(&reg));
    append(&d[A], red_approve_body(&reg));
    sync(&d);
    let now = reg.timestamp + 60;
    assert!(
        status(&d, &reg, now).complete,
        "{:?}",
        status(&d, &reg, now)
    );
    assert_eq!(
        settle_red_ops(&d[OBS].store, now).unwrap().applied,
        vec![reg.hash()]
    );

    // A mode change: nothing yet.
    let op = propose(
        &d,
        R,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[R].did.clone(),
            cosigner: None,
        },
    );
    sync(&d);
    let st = status(&d, &op, now);
    assert_eq!(
        st.missing,
        vec![
            RedProof::Phone,
            RedProof::SecondDevice,
            RedProof::Factor(None)
        ]
    );
    assert!(!settle_red_ops(&d[OBS].store, now)
        .unwrap()
        .applied
        .contains(&op.hash()));

    // Proofs from the wrong participants do not count: the proposer, the unattended hub, a stranger's
    // phone signature, and a factor signed for another op.
    append(&d[R], red_approve_body(&op));
    append(&d[H], red_approve_body(&op));
    append(&d[A], red_phone_sign_body(&op));
    let other = propose(
        &d,
        R,
        OpBody::ModeChange {
            to: CredMode::Vault,
            migrated_on: d[R].did.clone(),
            cosigner: Some(d[A].did.clone()),
        },
    );
    append(&d[B], red_factor_paper(&recover, &other));
    sync(&d);
    let st = status(&d, &op, now);
    assert!(st.have.is_empty(), "{st:?}");

    // The right ones.
    append(&d[P], red_phone_sign_body(&op));
    append(&d[A], red_approve_body(&op));
    sync(&d);
    assert_eq!(status(&d, &op, now).missing, vec![RedProof::Factor(None)]);
    append(&d[B], red_factor_paper(&recover, &op)); // any device may carry the factor
    sync(&d);
    assert!(status(&d, &op, now).complete);
    // The UI cannot approve early, and settling applies it.
    assert!(settle_red_ops(&d[OBS].store, now)
        .unwrap()
        .applied
        .contains(&op.hash()));
    assert_eq!(d[OBS].store.sync_cred_mode().unwrap(), CredMode::Quorum);
}

#[test]
fn a_ui_cannot_approve_a_red_op_early_and_a_stale_ceremony_is_dropped() {
    let d = world(&[R, A, B]);
    let op = propose(
        &d,
        R,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[R].did.clone(),
            cosigner: None,
        },
    );
    sync(&d);
    let now = op.timestamp + 10;
    let err = resolve(&d[OBS].store, &op.hash(), true, now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Red change"), "{err}");
    assert_eq!(
        d[OBS].store.sync_pending().unwrap().len(),
        1,
        "still pending"
    );
    // Rejecting is always allowed; so is a stale one: after the TTL nothing can complete it.
    let late = op.timestamp + RED_CEREMONY_TTL_SECS + 1;
    let report = settle_red_ops(&d[OBS].store, late).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(d[OBS].store.sync_pending().unwrap().is_empty());
}

#[test]
fn rules_that_make_an_op_impossible_reject_it_instead_of_waiting() {
    // Revoking a holder with only one other terminal left cannot reshare.
    let small = world(&[R, A]);
    let revoke = propose(
        &small,
        R,
        OpBody::DeviceRevoke {
            device_id: small[A].did.clone(),
        },
    );
    sync(&small);
    let st = red_status(&small[OBS].store, &revoke, revoke.timestamp).unwrap();
    assert!(
        st.blocked
            .as_deref()
            .is_some_and(|b| b.contains("two remaining")),
        "{st:?}"
    );
}

#[test]
fn a_ring_of_one_terminal_waives_the_second_device_and_requires_the_paper() {
    let d = world(&[R]);
    let phrase = bip39_encode(&[0x32; 32]);
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
    let reg = propose(&d, R, factor_register_paper(&recover));
    append(&d[P], red_phone_sign_body(&reg));
    sync(&d);
    // First factor ever: bootstrap, phone only.
    let st = red_status(&d[OBS].store, &reg, reg.timestamp).unwrap();
    assert!(st.single_machine_ring && st.complete, "{st:?}");
    settle_red_ops(&d[OBS].store, reg.timestamp).unwrap();

    let op = propose(
        &d,
        R,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[R].did.clone(),
            cosigner: None,
        },
    );
    append(&d[P], red_phone_sign_body(&op));
    sync(&d);
    let st = red_status(&d[OBS].store, &op, op.timestamp).unwrap();
    assert!(
        !st.complete && st.missing.contains(&RedProof::Factor(None)),
        "no second device to vote, so the paper is what is missing: {st:?}"
    );
    append(&d[R], red_factor_paper(&recover, &op));
    sync(&d);
    assert!(
        red_status(&d[OBS].store, &op, op.timestamp)
            .unwrap()
            .complete
    );
}

#[test]
fn a_cert_issue_is_proposed_by_the_phone_and_needs_a_second_device_that_is_not_the_delegate() {
    let d = world(&[R, A, B]);
    let phrase = bip39_encode(&[0x33; 32]);
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
    let reg = propose(&d, R, factor_register_paper(&recover));
    append(&d[P], red_phone_sign_body(&reg));
    append(&d[A], red_approve_body(&reg));
    sync(&d);
    settle_red_ops(&d[OBS].store, reg.timestamp + 5).unwrap();

    let now = crate::store::now_ts();
    let cert = propose(
        &d,
        P,
        OpBody::CertIssue {
            device_id: d[A].did.clone(),
            scope: crate::sync::oplog::CertScope::Hosts(vec![[1; 16]]),
            expiry: now + 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 9,
        },
    );
    append(&d[A], red_approve_body(&cert)); // the delegate itself: does not count
    append(&d[B], red_factor_paper(&recover, &cert));
    sync(&d);
    let st = red_status(&d[OBS].store, &cert, now).unwrap();
    assert!(
        st.have.contains(&RedProof::Phone),
        "the phone is the author"
    );
    assert_eq!(st.missing, vec![RedProof::SecondDevice]);
    append(&d[B], red_approve_body(&cert));
    sync(&d);
    assert!(red_status(&d[OBS].store, &cert, now).unwrap().complete);
}

#[test]
fn the_emailed_code_is_a_factor_and_its_sender_mails_only_when_the_other_proofs_are_in() {
    let d = world(&[R, A, B]);
    let phrase = bip39_encode(&[0x34; 32]);
    let recover = paper_derive(&phrase, PaperDomain::Recover).unwrap();
    let reg = propose(&d, R, factor_register_paper(&recover));
    append(&d[P], red_phone_sign_body(&reg));
    append(&d[A], red_approve_body(&reg));
    sync(&d);
    for x in &d {
        settle_red_ops(&x.store, reg.timestamp + 5).unwrap();
    }

    // Register the email factor with B as the sender; the paper authorizes the registration.
    let (body, code) = factor_register_email(&d[B].did).unwrap();
    let ereg = propose(&d, R, body);
    append(&d[P], red_phone_sign_body(&ereg));
    append(&d[A], red_approve_body(&ereg));
    append(&d[R], red_factor_paper(&recover, &ereg));
    sync(&d);
    for x in &d {
        settle_red_ops(&x.store, ereg.timestamp + 5).unwrap();
    }
    remember_email_code(&d[B].store, &code).unwrap();
    sync(&d);

    let op = propose(
        &d,
        R,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[R].did.clone(),
            cosigner: None,
        },
    );
    sync(&d);
    let now = op.timestamp + 30;
    assert!(
        owed_code_mails(&d[B].store, &d[B].did, now)
            .unwrap()
            .is_empty(),
        "the phone and the second device have not voted yet"
    );
    append(&d[P], red_phone_sign_body(&op));
    append(&d[A], red_approve_body(&op));
    sync(&d);
    assert!(
        owed_code_mails(&d[A].store, &d[A].did, now)
            .unwrap()
            .is_empty(),
        "only the sender mails"
    );
    let mails = owed_code_mails(&d[B].store, &d[B].did, now).unwrap();
    assert_eq!(mails.len(), 1);
    assert!(mails[0].2.contains(&code) && mails[0].2.contains("opens nothing"));
    mark_code_sent(&d[B].store, &mails[0].0).unwrap();
    assert!(
        owed_code_mails(&d[B].store, &d[B].did, now)
            .unwrap()
            .is_empty(),
        "once per op"
    );

    // The user pastes the code into the proposer: wrong code fails, right code signs.
    assert!(red_factor_email(&d[R].store, "ABCDE-FGHJK-MNPQR-STVWX", &op).is_err());
    append(&d[R], red_factor_email(&d[R].store, &code, &op).unwrap());
    sync(&d);
    assert!(red_status(&d[OBS].store, &op, now).unwrap().complete);
}
